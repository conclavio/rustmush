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
//! `inventory.book` are written together in one bundle, so one half lost beneath
//! the kernel leaves a state no single family can see. Three shapes are checked,
//! and all three quarantine the endpoint — every `debit`, `transfer`, `reserve`
//! and `crystallize` against it refuses until it is reconciled, and
//! **availability never rises**:
//!
//! * a pool without its book, or a book without its pool (presence);
//! * a pair whose two halves are both present but whose
//!   `ReservationBook::resource_revision` disagrees with the stored scalar's
//!   `revision` — the *other* crash shape of two `Put`s in one bundle, which
//!   before the E7c2 fold booted clean, was never quarantined and was never
//!   paged even though every later read and write against it refused (fold R2);
//! * a pool whose vault the economy did not hydrate, which is invisible to every
//!   viewer including staff unless something names it (fold R7).
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

/// How many pages the applied-marker walk visits. Deliberately wider than
/// [`MAX_APPLIED_MARKERS`] / [`INV_PAGE`] so the mirror can *see* past its own
/// capacity and keep the **newest** markers rather than the lexicographically
/// first ones (fold R3).
pub const MAX_MARKER_PAGES: usize = 256;

/// Why one endpoint is quarantined, and the reconciliation a staffer runs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Quarantine {
    /// A book survived without its pool scalar (or the reverse).
    HalfPair,
    /// Both halves are present and their revisions disagree.
    RevisionMismatch,
    /// The economy has no vault record for the endpoint the pool names.
    VaultMissing,
}

impl Quarantine {
    /// The one line a staff surface prints, with the order filled in (§4.0's
    /// staff ladder: never park state on "wait for staff").
    #[must_use]
    pub const fn runbook(self) -> &'static str {
        match self {
            Self::HalfPair => {
                "one half of the (inventory.pool, inventory.book) pair was lost beneath the \
                 kernel; restore the missing half from the last complete bundle, then \
                 `@inv/show <endpoint>`"
            }
            Self::RevisionMismatch => {
                "both halves are present and their revisions disagree, so every read and \
                 write against this endpoint refuses; restore the half carrying the \
                 pre-image, then `@inv/show <endpoint>`"
            }
            Self::VaultMissing => {
                "the economy hydrated no vault for this endpoint, so its quantity is \
                 unreachable; restore the vault record (`@vault/show <id>`), then \
                 `@inv/show <endpoint>`"
            }
        }
    }
}

/// The module's RAM projection: what boot derived and what a `&World`-only
/// reader (the fact provider) is allowed to see.
#[derive(Clone, Debug)]
pub struct InventoryStore {
    /// Endpoints whose pool/book pair disagreed at boot, and why.
    quarantined: BTreeMap<EndpointRef, Quarantine>,
    /// Pool keys whose scalar survived without a book. The endpoint is not
    /// recoverable from a scalar alone (only the book carries the
    /// `ResourceRef`), so the key itself is the marker.
    quarantined_keys: BTreeSet<SmolStr>,
    /// The applied-marker mirror the `inventory.*` fact provider answers from.
    applied: BTreeMap<SmolStr, AppliedMarker>,
    /// Whether the last reconciliation completed inside its page budget. A
    /// truncated walk leaves the store **fail-closed**: it does not claim the
    /// mirror is complete, and `facts::resolve` answers nothing at all.
    complete: bool,
    /// How many reconciliations have run. E7c2 fold B4 dropped the
    /// per-command rebuild; this counter is what pins that it stays dropped.
    reconciliations: u64,
    /// IC second the last mutating `@inv` submission was stamped with, and its
    /// ordinal inside that second (E7c2 fold R1(b)). Two deliberately retyped
    /// intents are two requests; one submission driven twice is still one.
    attempt_at: i64,
    attempt_seq: u32,
}

impl Default for InventoryStore {
    fn default() -> Self {
        Self {
            quarantined: BTreeMap::new(),
            quarantined_keys: BTreeSet::new(),
            applied: BTreeMap::new(),
            complete: false,
            reconciliations: 0,
            // A sentinel no IC second equals, so the first submission of any
            // second is ordinal zero.
            attempt_at: i64::MIN,
            attempt_seq: 0,
        }
    }
}

impl InventoryStore {
    /// Whether that endpoint is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, endpoint: EndpointRef) -> bool {
        self.quarantined.contains_key(&endpoint)
    }

    /// Why that endpoint is quarantined, if it is.
    #[must_use]
    pub fn quarantine_reason(&self, endpoint: EndpointRef) -> Option<Quarantine> {
        self.quarantined.get(&endpoint).copied()
    }

    /// Whether that exact pool key is quarantined.
    #[must_use]
    pub fn is_key_quarantined(&self, key: &str) -> bool {
        self.quarantined_keys.contains(key)
    }

    /// Every quarantined endpoint and its reason, in canonical order.
    pub fn quarantined(&self) -> impl Iterator<Item = (EndpointRef, Quarantine)> + '_ {
        self.quarantined.iter().map(|(id, why)| (*id, *why))
    }

    /// Every quarantined pool key whose endpoint is not recoverable.
    pub fn orphaned_keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.quarantined_keys.iter().map(SmolStr::as_str)
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

    /// How many reconciliations have run in this process.
    #[must_use]
    pub const fn reconciliations(&self) -> u64 {
        self.reconciliations
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

    /// The ordinal the next mutating submission of this IC second carries.
    ///
    /// E7c2 fold R1(b): the handler's `key` used to be a *constant* per-verb
    /// discriminator, so `@inv/move 1/metals:5=2` had one request identity for
    /// that actor for ever and a legitimate "move five more" silently collapsed
    /// into a preflight HIT. Stamping the IC second and an ordinal inside it
    /// makes a retyped intent a new request, as the player means it, while
    /// driving the same `AuthoritativeSubmit` value twice is still one commit.
    /// It is **not** a random: nothing here draws.
    pub fn next_attempt(&mut self, at: i64) -> u32 {
        if at == self.attempt_at {
            self.attempt_seq = self.attempt_seq.saturating_add(1);
        } else {
            self.attempt_at = at;
            self.attempt_seq = 0;
        }
        self.attempt_seq
    }
}

/// Rebuild the RAM projection from the typed families.
///
/// Called at boot and from the coarse sweep tick — and, since E7c2 fold B4, **not
/// from the command path**: rebuilding three whole families on the world
/// single-writer for every read switch grew with the tree's pool count rather
/// than with the caller's, and 08i answer 15 grants typed reads in ticks and boot
/// reconciliation, which is exactly what those two are. The mirror is therefore
/// **behind** the authoritative truth by at most one sweep, and never ahead —
/// which is the direction that matters, because `FactProvider` documents *unknown
/// is unsatisfied, everywhere* and a contract that waits one tick longer is safe
/// while one that settles early is not.
///
/// (Why a mirror at all: `FactProvider::resolve` receives `&World`, and typed
/// state lives on the `Engine`. Widening that seam is recorded as an ask in
/// 10b §3.2's "As built" rather than taken in cut 2.)
pub fn reconcile(ctx: &mut Ctx<'_>, families: &InventoryFamilies) {
    let mut quarantined: BTreeMap<EndpointRef, Quarantine> = BTreeMap::new();
    let mut quarantined_keys = BTreeSet::new();
    let mut applied: BTreeMap<SmolStr, AppliedMarker> = BTreeMap::new();
    let mut complete = true;

    // Book keys → (endpoint, the book's own view of the scalar revision). Only
    // the book carries the `ResourceRef`, and only the book carries
    // `resource_revision` — which is the half of the pair check that was
    // missing (fold R2).
    let mut books: BTreeMap<SmolStr, (EndpointRef, u64)> = BTreeMap::new();
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    loop {
        let Ok(page) = ctx.scan_action_state(&families.books, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            if let Some(endpoint) = endpoint_of(entry.value().resource().subject.clone()) {
                books.insert(
                    SmolStr::new(entry.encoded_key()),
                    (endpoint, entry.value().resource_revision()),
                );
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

    // Pool keys → the stored scalar's revision. Anything present in exactly one
    // of the two families is a half-pool; anything present in both whose two
    // revisions disagree is the *other* half-applied shape of one bundle's two
    // `Put`s, and it refuses every later read and write just as hard.
    let mut pools: BTreeMap<SmolStr, u64> = BTreeMap::new();
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    loop {
        let Ok(page) = ctx.scan_action_state(&families.pools, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            pools.insert(SmolStr::new(entry.encoded_key()), entry.value().revision());
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

    for (key, (endpoint, resource_revision)) in &books {
        match pools.get(key) {
            None => {
                quarantined.insert(*endpoint, Quarantine::HalfPair);
            }
            Some(revision) if revision != resource_revision => {
                quarantined
                    .entry(*endpoint)
                    .or_insert(Quarantine::RevisionMismatch);
            }
            Some(_) => {}
        }
    }
    for key in pools.keys() {
        if !books.contains_key(key) {
            quarantined_keys.insert(key.clone());
        }
    }

    // The applied-marker mirror. Marker ids are `{at}{request}` (fold R3), so
    // canonical key order IS time order and keeping the LAST
    // `MAX_APPLIED_MARKERS` keeps the newest rather than whichever request
    // digests happened to sort first.
    let mut after: Option<SmolStr> = None;
    let mut pages = 0_usize;
    let mut truncated_markers = false;
    'markers: loop {
        let Ok(page) = ctx.scan_action_state(&families.applied, after.as_deref(), INV_PAGE) else {
            complete = false;
            break;
        };
        for entry in page.entries() {
            applied.insert(SmolStr::new(entry.encoded_key()), entry.value().clone());
            if applied.len() > MAX_APPLIED_MARKERS {
                applied.pop_first();
                truncated_markers = true;
            }
        }
        after = page.next_after().cloned();
        pages += 1;
        if after.is_none() {
            break;
        }
        if pages >= MAX_MARKER_PAGES {
            // A labelled break: the walk used to keep paging and discarding
            // after the mirror was full, which is wasted work on the world
            // single-writer (fold B14).
            complete = false;
            break 'markers;
        }
    }

    // Fold R7: inventory is composed immediately after the economy *because* its
    // boot reconciliation asks the economy about vault status — and it never
    // did. A pool whose vault did not hydrate is unreachable to every viewer,
    // staff included, and nothing named it.
    for (endpoint, _) in books.values() {
        if mush_economy::endpoint_state(ctx.w, endpoint.vault())
            == mush_economy::EndpointState::Missing
        {
            quarantined
                .entry(*endpoint)
                .or_insert(Quarantine::VaultMissing);
        }
    }

    if !quarantined.is_empty() || !quarantined_keys.is_empty() {
        // Level 2: stop only the subject. The world boots; the named endpoints
        // refuse every mutation until a staffer reconciles them, and nothing
        // else in the world is affected (08c §4.6's ladder).
        for (endpoint, why) in &quarantined {
            tracing::error!(
                endpoint = %endpoint,
                "inventory: {} — {}",
                match why {
                    Quarantine::HalfPair => "pool and book disagree by presence",
                    Quarantine::RevisionMismatch => "pool and book disagree by revision",
                    Quarantine::VaultMissing => "no vault hydrated for this endpoint",
                },
                why.runbook()
            );
        }
        if !quarantined_keys.is_empty() {
            tracing::error!(
                orphaned_pool_keys = quarantined_keys.len(),
                "inventory: pool scalars survived without their books; no endpoint is \
                 recoverable from a scalar alone. Run `@inv/quarantine` for the keys and \
                 the reconciliation order."
            );
        }
    }
    if truncated_markers || !complete {
        // Fold R3: a truncated mirror is not "behind by one sweep", it is a
        // mirror that answers nothing — so it is an operator event, not a note.
        tracing::error!(
            markers = applied.len(),
            complete,
            "inventory: the applied-marker mirror is truncated; every inventory fact \
             answers unsatisfied until the family is pruned"
        );
        complete = false;
    }

    // Never let an ordinary pool/marker sweep erase an exact-input refusal.
    // Reconstruct its permanent K/lot/head/census joins on every reconciliation.
    if !crate::core_input::audit::reconcile(ctx, families) {
        complete = false;
        tracing::error!(
            "inventory: exact core-input history disagrees; quantity readiness remains unavailable"
        );
    }
    let (reconciliations, attempt_at, attempt_seq) = ctx
        .w
        .module_state::<InventoryStore>()
        .map_or((0, i64::MIN, 0), |store| {
            (store.reconciliations, store.attempt_at, store.attempt_seq)
        });
    if let Some(store) = ctx.w.module_state_mut::<InventoryStore>() {
        *store = InventoryStore {
            quarantined,
            quarantined_keys,
            applied,
            complete,
            reconciliations: reconciliations.saturating_add(1),
            attempt_at,
            attempt_seq,
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
