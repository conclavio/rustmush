//! **Boot pass 0 and the fail-closed quarantine** (E1–E5 look-back fold, D-LBK-1 +
//! D-LBK-4a; 08h F1/F4).
//!
//! ## The three container invariants
//!
//! A `property:{id}` and a `tenancy:{id}` document each re-check their *own* shape on
//! the way in (E5a's `TryFrom` wires), and the write paths refuse every cross-record
//! shape below — but nothing checked those shapes **between** records at boot, so a
//! restore from two half-matching snapshots, or one hand-edited document, booted a
//! world in which:
//!
//! - (a) **one room belonged to two properties**, and `property_of_room` answered the
//!   lowest id — whose property might have no live tenancy, so the door of somebody's
//!   home fell **open**;
//! - (b) **one property carried two live tenancies**, and `live_tenancy_of` picked
//!   the lowest id and logged;
//! - (c) **a live tenancy named a property that was not loaded**, and the tick
//!   assessed it at tier 0 on a number nobody wrote.
//!
//! ## The rule: fail closed, never fatal, release only by re-validation
//!
//! Every record involved in a violation goes into an in-memory **quarantine** —
//! recomputed from the records on every boot and after every write, never
//! persisted. While a record is quarantined: every room of every property involved
//! refuses everyone but staff at the door ([`crate::may_reach`]); the destroy gate
//! refuses those rooms; the assessment tick skips the tenancies entirely (no
//! assessment, no window advance, no notice); members and officers read "(under
//! review)" and everyone else the hidden text; the exact-mode **rent is suspended
//! through the economy's own seam** ([`mush_economy::suspend_obligation`]) under the
//! conflict's key, synchronously, in the boot batch — so nobody is charged for a home
//! they were refused, and that is guaranteed rather than paged; and the economy's
//! ladder carries **one page per conflict** with the repair written out in the
//! shipped syntax.
//!
//! The world does **not** halt: the money side keeps its own strict boot gate (E2)
//! because a wrong answer there moves money; a housing document takes the world
//! down for nobody (the staff-scarce principle — one corrupt document must not park
//! every player on "wait for staff").
//!
//! **There is no release verb.** The mutators that can repair a conflict
//! (`@property/rooms <id>=-<room>`, `@tenancy/resolve <id>=end`,
//! `@property/withdraw <id>`, the flow-level tenancy end) all go through the module's
//! single write path, which re-runs this check and lifts a quarantine — resuming the
//! rent — only when the invariant holds again. `@housing/quarantine show` lists the
//! set; `@housing/quarantine recheck <key>` re-runs the check after a repair made
//! outside those verbs. Nothing releases a conflict that still exists.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, ObjId};
use mush_economy::{EscalationKind, EscalationSubject, SuspensionKey};

use crate::ids::{PropertyId, TenancyId};
use crate::module::{HousingStore, LiveEntry, RoomEntry};
use crate::record::{Funding, PropertyStatus};

/// The most pages one revalidation opens — the economy's `MAX_BOOT_ESCALATIONS`
/// rule, applied to the fold's pass 0: a restore that crossed a whole region must not
/// mint a thousand alerts. Past the cap the records are still quarantined and the
/// reason is still logged.
pub const MAX_QUARANTINE_PAGES: usize = 16;

/// The identity of one conflict — what the escalation is about and what the rent
/// suspension is keyed on.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QuarantineKey {
    /// A room held by more than one property: `housing.corrupt:room:<room>`.
    Room(ObjId),
    /// A property carrying more than one live tenancy:
    /// `housing.corrupt:tenancy:<property>`.
    Tenancy(PropertyId),
    /// A live tenancy whose property is not loaded: `housing.corrupt:orphan:<tenancy>`.
    Orphan(TenancyId),
}

impl QuarantineKey {
    /// Parse the rendered form back (`@housing/quarantine recheck <key>`).
    #[must_use]
    pub fn parse(s: &str) -> Option<QuarantineKey> {
        let rest = s.trim().strip_prefix("housing.corrupt:")?;
        let (kind, id) = rest.split_once(':')?;
        let id = id.trim().trim_start_matches('#');
        match kind {
            "room" => id
                .parse::<u32>()
                .ok()
                .map(|n| QuarantineKey::Room(ObjId(n))),
            "tenancy" => id
                .parse::<u64>()
                .ok()
                .map(|n| QuarantineKey::Tenancy(PropertyId(n))),
            "orphan" => id
                .parse::<u64>()
                .ok()
                .map(|n| QuarantineKey::Orphan(TenancyId(n))),
            _ => None,
        }
    }
    /// The economy-side key the rent suspension is held under. The rendered form is
    /// at most 44 bytes, so it always fits [`mush_economy::MAX_SUSPENSION_KEY_BYTES`];
    /// `None` is unreachable and treated as "cannot suspend" by the caller.
    #[must_use]
    pub fn suspension_key(self) -> Option<SuspensionKey> {
        SuspensionKey::new(&self.to_string())
    }
    /// The ladder's subject — the dedupe/reopen identity of the one page per conflict.
    #[must_use]
    pub fn subject(self) -> EscalationSubject {
        match self {
            QuarantineKey::Room(room) => EscalationSubject::HousingRoom(room.0),
            QuarantineKey::Tenancy(property) => EscalationSubject::HousingProperty(property.0),
            QuarantineKey::Orphan(tenancy) => EscalationSubject::HousingTenancy(tenancy.0),
        }
    }
}

impl std::fmt::Display for QuarantineKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuarantineKey::Room(room) => write!(f, "housing.corrupt:room:{}", room.0),
            QuarantineKey::Tenancy(p) => write!(f, "housing.corrupt:tenancy:{p}"),
            QuarantineKey::Orphan(t) => write!(f, "housing.corrupt:orphan:{t}"),
        }
    }
}

/// One quarantined conflict: which records it involves, what a staffer types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Identity.
    pub key: QuarantineKey,
    /// Every property whose rooms are refused while this stands.
    pub properties: BTreeSet<PropertyId>,
    /// Every live tenancy the tick skips and the rent of which is suspended.
    pub tenancies: BTreeSet<TenancyId>,
    /// The diagnosis, one line.
    pub summary: String,
    /// The repair, in the shipped syntax, arguments filled in.
    pub repair: Vec<String>,
}

/// **Compute the quarantine from the projections** — the pure half of pass 0.
///
/// The `Conflict` entries of the room and live-tenancy projections are the source
/// of truth for (a) and (b); (c) is every open live tenancy whose property is not
/// loaded. Pure: it reads the store and writes nothing.
#[must_use]
pub(crate) fn compute(store: &HousingStore) -> BTreeMap<QuarantineKey, Conflict> {
    let mut out: BTreeMap<QuarantineKey, Conflict> = BTreeMap::new();
    // (a) a room held by more than one property.
    for (room, entry) in &store.room_index {
        let RoomEntry::Conflict(props) = entry else {
            continue;
        };
        let mut tenancies = BTreeSet::new();
        let mut repair = Vec::new();
        for p in props {
            match store.live_index.get(p) {
                Some(LiveEntry::Live(t)) => {
                    tenancies.insert(*t);
                }
                Some(LiveEntry::Conflict(ts)) => tenancies.extend(ts.iter().copied()),
                Some(LiveEntry::Vacant) | None => {}
            }
            let is_let = store
                .property(*p)
                .is_some_and(|x| x.status == PropertyStatus::Let);
            let is_anchor = store.property(*p).is_some_and(|x| x.anchor == *room);
            if is_anchor {
                // The anchor can never be removed; the property itself has to go.
                repair.push(format!("@property/withdraw {p}"));
            } else {
                if is_let {
                    repair.push(format!("@property/withdraw {p}"));
                }
                repair.push(format!("@property/rooms {p}=-#{}", room.0));
            }
        }
        let names: Vec<String> = props.iter().map(|p| format!("#{p}")).collect();
        out.insert(QuarantineKey::Room(*room), Conflict {
            key: QuarantineKey::Room(*room),
            properties: props.clone(),
            tenancies,
            summary: format!(
                "room #{} is a room of {} properties at once ({}); every room of every one of them \
                 refuses everyone but staff until exactly one holds it",
                room.0,
                props.len(),
                names.join(", ")
            ),
            repair: repair.into_iter().take(mush_economy::MAX_RECOMMENDED_COMMANDS).collect(),
        });
    }
    // (b) a property carrying more than one live tenancy.
    for (property, entry) in &store.live_index {
        let LiveEntry::Conflict(ts) = entry else {
            continue;
        };
        let names: Vec<String> = ts.iter().map(|t| format!("#{t}")).collect();
        let repair: Vec<String> = ts
            .iter()
            .map(|t| format!("@tenancy/resolve {t}=end"))
            .take(mush_economy::MAX_RECOMMENDED_COMMANDS)
            .collect();
        out.insert(QuarantineKey::Tenancy(*property), Conflict {
            key: QuarantineKey::Tenancy(*property),
            properties: BTreeSet::from([*property]),
            tenancies: ts.clone(),
            summary: format!(
                "property #{property} carries {} live tenancies at once ({}); a residence answers \
                 for exactly one, so every one of them is under review until the others are ended",
                ts.len(),
                names.join(", ")
            ),
            repair,
        });
    }
    // (c) a live tenancy on a property that is not loaded.
    for id in &store.open {
        let Some(t) = store.tenancies.get(id) else {
            continue;
        };
        if !t.status.is_live() || store.properties.contains_key(&t.property) {
            continue;
        }
        out.insert(
            QuarantineKey::Orphan(*id),
            Conflict {
                key: QuarantineKey::Orphan(*id),
                properties: BTreeSet::new(),
                tenancies: BTreeSet::from([*id]),
                summary: format!(
                    "tenancy #{id} is live on property #{}, which does not exist; it is neither \
                 assessed nor charged until it is ended or the property is restored",
                    t.property
                ),
                repair: vec![
                    format!("@tenancy/show {id}"),
                    format!("@tenancy/resolve {id}=end"),
                ],
            },
        );
    }
    out
}

/// **Re-run pass 0 and apply the difference** — the write path's and the boot's one
/// entry (D-LBK-1's "release only by re-validation").
///
/// The store's quarantine is replaced **before** the economy is asked anything, so
/// the observer re-entry a rent resumption causes (`ObligationChanged { Active }` →
/// housing mirrors it → the single write path → this function again) finds no
/// difference and stops. Then, per conflict that appeared: the rent of every
/// involved exact tenancy is suspended under the conflict's key and one page opens
/// (at most [`MAX_QUARANTINE_PAGES`] per call); per conflict that lifted: every rent
/// held under its key is released — the economy returns the obligation to the due
/// pool only when no other holder remains, which is what makes two overlapping
/// conflicts on one tenancy safe. Returns whether the set changed.
pub(crate) fn revalidate(ctx: &mut Ctx<'_>) -> bool {
    let (added, removed) = {
        let Some(store) = ctx.w.module_state_mut::<HousingStore>() else {
            return false;
        };
        let next = compute(store);
        if next == store.quarantine {
            return false;
        }
        let mut added: Vec<Conflict> = Vec::new();
        let mut removed: Vec<Conflict> = Vec::new();
        for (key, conflict) in &next {
            match store.quarantine.get(key) {
                None => added.push(conflict.clone()),
                Some(prev) if prev.tenancies != conflict.tenancies => {
                    // The same conflict, a different set of tenancies: the delta is
                    // exactly what the rent suspension has to follow.
                    let mut gained = conflict.clone();
                    gained.tenancies = conflict
                        .tenancies
                        .difference(&prev.tenancies)
                        .copied()
                        .collect();
                    let mut lost = prev.clone();
                    lost.tenancies = prev
                        .tenancies
                        .difference(&conflict.tenancies)
                        .copied()
                        .collect();
                    if !gained.tenancies.is_empty() {
                        added.push(gained);
                    }
                    if !lost.tenancies.is_empty() {
                        removed.push(lost);
                    }
                }
                Some(_) => {}
            }
        }
        for (key, prev) in &store.quarantine {
            if !next.contains_key(key) {
                removed.push(prev.clone());
            }
        }
        store.quarantine = next;
        store.population_version = store.population_version.saturating_add(1);
        (added, removed)
    };
    for conflict in &removed {
        resume_rent(ctx, conflict);
        tracing::warn!(key = %conflict.key, "housing: a quarantine lifted (the invariant holds again)");
    }
    let mut pages = 0usize;
    for conflict in &added {
        suspend_rent(ctx, conflict);
        tracing::error!(key = %conflict.key, summary = %conflict.summary, "housing: quarantined");
        if pages < MAX_QUARANTINE_PAGES {
            pages += 1;
            let mut repair = conflict.repair.clone();
            if repair.is_empty() {
                repair.push("@housing/quarantine show".to_string());
            }
            mush_economy::open_escalation(
                ctx,
                EscalationKind::BootReconciliation,
                conflict.key.subject(),
                format!(
                    "{} [{}]. Fail-closed: staff only at the door, the tick skips it, rent is \
                     suspended. It lifts by itself the moment the repair holds \
                     (@housing/quarantine show)",
                    conflict.summary, conflict.key
                ),
                repair,
            );
        }
    }
    true
}

/// The exact-mode rent obligations of a conflict's tenancies, with the key to hold
/// them under. A draft (`PENDING_OBLIGATION`) has no rent record yet and is skipped.
fn rents(ctx: &Ctx<'_>, conflict: &Conflict) -> Vec<(TenancyId, mush_economy::ObligationId)> {
    let Some(store) = ctx.w.module_state::<HousingStore>() else {
        return Vec::new();
    };
    conflict
        .tenancies
        .iter()
        .filter_map(|id| {
            let t = store.tenancy(*id)?;
            match &t.funding {
                Funding::Exact { obligation, .. } if !crate::econ::is_pending_lease(&t.funding) => {
                    Some((*id, *obligation))
                }
                Funding::Exact { .. } | Funding::Abstract { .. } => None,
            }
        })
        .collect()
}

fn suspend_rent(ctx: &mut Ctx<'_>, conflict: &Conflict) {
    let Some(key) = conflict.key.suspension_key() else {
        return;
    };
    for (tenancy, obligation) in rents(ctx, conflict) {
        match mush_economy::suspend_obligation(ctx, obligation, key.clone()) {
            Ok(()) => tracing::warn!(
                tenancy = tenancy.0,
                obligation = obligation.0,
                key = %key,
                "housing: rent suspended for a quarantined tenancy"
            ),
            // A missing or finished obligation cannot charge anybody; its absence is
            // pass 3's evidence and pages there.
            Err(error) => tracing::error!(
                tenancy = tenancy.0,
                obligation = obligation.0,
                %error,
                "housing: could not suspend a quarantined tenancy's rent (it cannot charge either)"
            ),
        }
    }
}

fn resume_rent(ctx: &mut Ctx<'_>, conflict: &Conflict) {
    let Some(key) = conflict.key.suspension_key() else {
        return;
    };
    for (tenancy, obligation) in rents(ctx, conflict) {
        match mush_economy::resume_obligation(ctx, obligation, &key) {
            Ok(returned) => tracing::info!(
                tenancy = tenancy.0,
                obligation = obligation.0,
                key = %key,
                returned,
                "housing: a quarantine's hold on the rent released"
            ),
            Err(error) => tracing::warn!(
                tenancy = tenancy.0,
                obligation = obligation.0,
                %error,
                "housing: could not release a quarantine's hold on the rent"
            ),
        }
    }
}

/// The "(under review)" line a member or an officer reads for a quarantined record.
#[must_use]
pub(crate) fn under_review(name: &str) -> String {
    format!("{name}: (under review)")
}
