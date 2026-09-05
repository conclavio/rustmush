//! E7c2 B6 — boot hydration, the endpoint quarantine, and the fact mirror.
//!
//! # The document table, and the direction of every loss
//!
//! Cut 2 owns **no conventional document**. Every record it writes is an
//! authoritative typed family, so the kernel's own hydration rules apply and they
//! are *stricter* than §4 rule 2's "skipped loudly": `ActionStateStore::hydrate`
//! is atomic and a record that fails path, envelope, schema, bounds, typed
//! validation or canonical decode→serialize equality **refuses the boot**.
//!
//! | family | class | corrupt → | direction of the loss |
//! |---|---|---|---|
//! | `inventory.pool` (`BoundedScalar`) | authoritative | **boot refuses** | a pool that cannot be decoded is a balance nobody can bound; booting without it would make every later `debit` a fabrication |
//! | `inventory.book` (`ReservationBook`) | authoritative | **boot refuses** | losing the book loses every live hold, so availability would *rise* — the one fail-open direction the design forbids |
//! | `inventory.lot` (`Lot`) | authoritative | **boot refuses** | E5a lens A: the missing lot is exactly the one the hold was against |
//! | `inventory.tranche` (`PoolTranche`) | authoritative | **boot refuses** | the tranche is the immutable lineage root; losing it would let a re-mint claim clean provenance |
//! | `inventory.applied` (`AppliedMarker`) | authoritative | **boot refuses** | a marker is what a settlement points at; an unreadable one must never read as "did not happen" *or* as "happened" |
//!
//! # What this module adds on top
//!
//! The kernel decodes each family independently and cannot see across them. The
//! **pair** invariant can only be checked here: `inventory.pool` and
//! `inventory.book` are written together in one bundle, so a pool without its
//! book — or a book without its pool — means one half was lost beneath the
//! kernel. That endpoint is **quarantined**: every `debit`, `transfer`,
//! `reserve` and `crystallize` against it refuses until it is reconciled, and
//! **availability never rises**, because the fail-open shape here is *"the
//! missing book was the one the hold was in"*.
//!
//! The quarantine marker is **not a document**. It is derived at boot from the
//! typed decode outcomes themselves, so it cannot go stale and needs no key.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, IdempotencyKeyText, SubjectKey};
use smol_str::SmolStr;

use crate::pool::InventoryFamilies;
use crate::record::{
    AppliedMarker, EndpointRef, INV_PAGE, MAX_APPLIED_MARKERS, MAX_POOLS_PER_ENDPOINT,
};

/// How many family pages one reconciliation walks. Bounded on records
/// **visited**, never on records matched (D-LBK-5).
pub const MAX_BOOT_PAGES: usize = 128;

/// The module's RAM projection: what boot derived and what a `&World`-only
/// reader (the fact provider) is allowed to see.
#[derive(Clone, Debug, Default)]
pub struct InventoryStore {
    /// Endpoints whose pool/book pair disagreed at boot.
    quarantined: BTreeSet<EndpointRef>,
    /// Pool keys whose scalar survived without a book. The endpoint is not
    /// recoverable from a scalar alone (only the book carries the
    /// `ResourceRef`), so the key itself is the marker.
    quarantined_keys: BTreeSet<SmolStr>,
    /// The applied-marker mirror the `inventory.*` fact provider answers from.
    applied: BTreeMap<SmolStr, AppliedMarker>,
    /// Whether the last reconciliation completed inside its page budget. A
    /// truncated walk leaves the store **fail-closed**: it does not claim the
    /// mirror is complete.
    complete: bool,
}

impl InventoryStore {
    /// Whether that endpoint is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, endpoint: EndpointRef) -> bool {
        self.quarantined.contains(&endpoint)
    }

    /// Whether that exact pool key is quarantined.
    #[must_use]
    pub fn is_key_quarantined(&self, key: &str) -> bool {
        self.quarantined_keys.contains(key)
    }

    /// Every quarantined endpoint, in canonical order.
    pub fn quarantined(&self) -> impl Iterator<Item = EndpointRef> + '_ {
        self.quarantined.iter().copied()
    }

    /// How many pool keys are quarantined without a nameable endpoint.
    #[must_use]
    pub fn orphaned_pool_keys(&self) -> usize {
        self.quarantined_keys.len()
    }

    /// Whether the last reconciliation saw the whole of both families.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// One applied marker, by its fact id.
    #[must_use]
    pub fn marker(&self, id: &str) -> Option<&AppliedMarker> {
        self.applied.get(id)
    }

    /// How many markers the mirror holds.
    #[must_use]
    pub fn markers(&self) -> usize {
        self.applied.len()
    }
}

/// Rebuild the RAM projection from the typed families.
///
/// Called at boot, from the coarse sweep tick, and at the top of every `@inv`
/// command. The mirror is therefore **behind** the authoritative truth by at most
/// one of those, and never ahead — which is the direction that matters, because
/// `FactProvider` documents *unknown is unsatisfied, everywhere* and a contract
/// that waits one tick longer is safe while one that settles early is not.
///
/// (Why a mirror at all: `FactProvider::resolve` receives `&World`, and typed
/// state lives on the `Engine`. Widening that seam is recorded as an ask in
/// 10b §3.2's "As built" rather than taken in cut 2.)
pub fn reconcile(ctx: &mut Ctx<'_>, families: &InventoryFamilies) {
    let mut quarantined = BTreeSet::new();
    let mut quarantined_keys = BTreeSet::new();
    let mut applied = BTreeMap::new();
    let mut complete = true;

    // Book keys → endpoint. Only the book carries the `ResourceRef`.
    let mut books: BTreeMap<SmolStr, EndpointRef> = BTreeMap::new();
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    loop {
        let Ok(page) = ctx.scan_action_state(&families.books, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            if let Some(endpoint) = endpoint_of(entry.value().resource().subject.clone()) {
                books.insert(SmolStr::new(entry.encoded_key()), endpoint);
            }
        }
        after = page.next_after().cloned();
        pages += 1;
        if after.is_none() {
            break;
        }
        if pages >= MAX_BOOT_PAGES {
            complete = false;
            break;
        }
    }

    // Pool keys. Anything present in exactly one of the two families is a
    // half-pool and quarantines its endpoint.
    let mut pools: BTreeSet<SmolStr> = BTreeSet::new();
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    loop {
        let Ok(page) = ctx.scan_action_state(&families.pools, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            pools.insert(SmolStr::new(entry.encoded_key()));
        }
        after = page.next_after().cloned();
        pages += 1;
        if after.is_none() {
            break;
        }
        if pages >= MAX_BOOT_PAGES {
            complete = false;
            break;
        }
    }

    for (key, endpoint) in &books {
        if !pools.contains(key) {
            quarantined.insert(*endpoint);
        }
    }
    for key in &pools {
        if !books.contains_key(key) {
            quarantined_keys.insert(key.clone());
        }
    }

    // The applied-marker mirror.
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    loop {
        let Ok(page) = ctx.scan_action_state(&families.applied, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            if applied.len() >= MAX_APPLIED_MARKERS {
                complete = false;
                break;
            }
            applied.insert(SmolStr::new(entry.encoded_key()), entry.value().clone());
        }
        after = page.next_after().cloned();
        pages += 1;
        if after.is_none() {
            break;
        }
        if pages >= MAX_BOOT_PAGES {
            complete = false;
            break;
        }
    }

    if !quarantined.is_empty() || !quarantined_keys.is_empty() {
        // Level 2: stop only the subject. The world boots; the named endpoints
        // refuse every mutation until a staffer reconciles them, and nothing
        // else in the world is affected (08c §4.6's ladder).
        tracing::error!(
            endpoints = quarantined.len(),
            orphaned_pool_keys = quarantined_keys.len(),
            "inventory: pool/book pairs disagree; those endpoints are quarantined. \
             Run `@inv/show <endpoint>` on each and reconcile before any debit."
        );
    }
    if !complete {
        tracing::warn!(
            "inventory: reconciliation did not complete inside its page budget; \
             the fact mirror is treated as incomplete (unknown is unsatisfied)"
        );
    }

    if let Some(store) = ctx.w.module_state_mut::<InventoryStore>() {
        *store = InventoryStore {
            quarantined,
            quarantined_keys,
            applied,
            complete,
        };
    }
}

/// Recover the endpoint a pool's `ResourceRef` names.
///
/// Total and fail-closed: an unexpected subject shape answers `None` rather than
/// guessing an endpoint that would then be trusted for a debit.
#[must_use]
pub fn endpoint_of(subject: SubjectKey) -> Option<EndpointRef> {
    let SubjectKey::Scoped { owner, class, id } = subject else {
        return None;
    };
    if owner != crate::record::owner() || class.as_str() != crate::record::ENDPOINT_CLASS {
        return None;
    }
    parse_endpoint_text(&id)
}

fn parse_endpoint_text(id: &IdempotencyKeyText) -> Option<EndpointRef> {
    id.as_str()
        .strip_prefix('v')
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(|vault| EndpointRef(mush_economy::VaultId(vault)))
}

/// How many distinct commodities one endpoint may pool (§7's
/// [`MAX_POOLS_PER_ENDPOINT`]) — re-exported here because the check belongs
/// wherever an endpoint's pools are counted.
#[must_use]
pub const fn max_pools_per_endpoint() -> usize {
    MAX_POOLS_PER_ENDPOINT
}
