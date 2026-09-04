//! Housing boot: load the documents, then reconcile them against the live world
//! (E5b, 08d §4's first bullet).
//!
//! ## The document discipline
//!
//! | doc | class | a corrupt one |
//! |---|---|---|
//! | `property:{id}` | container | **fails the boot** |
//! | `tenancy:{id}` | container | **fails the boot** |
//! | `assess:{id}` | leaf | skipped loudly; re-seeded at the next fire |
//! | `notice:{obj}` | leaf | skipped loudly; costs one queued line |
//! | `meta` | allocators | tolerated field by field, floored by the keys |
//!
//! A property and a tenancy are containers for the same reason an obligation is
//! (D-E4-5): their state **is** the record. A tenancy that fails to parse is a home
//! whose roster, guest list and status nobody can read — the access hook would answer
//! "nobody lives here" for rooms somebody lives in, the destroy gate would let those
//! rooms be deleted, and the ladder would start again from `Pending`. Failing the
//! boot is loud and recoverable; booting without it is silent and is not.
//!
//! Both types deserialize through E5a's `TryFrom` wires (§3.5 A2/A3), so "parses" is
//! the strong sense: every §9 cap, the anchor-in-rooms rule, tenant ∉ guests, and
//! "a plan is filed under its own resident" are all re-checked here, on the way in.
//!
//! ## Reconciliation, and what it deliberately does not do
//!
//! Nothing here ends a tenancy. **D-E5-1 runs at boot too**: a residence whose rooms
//! vanished, or whose rent obligation is missing, is a staff question — a level-2 page
//! carrying the command that answers it — and never an automatic ending. The three
//! passes are exactly 08d §4's:
//!
//! 1. **A property whose anchor or rooms are gone or `GOING` → `Withdrawn` + a page.**
//!    Its tenancies stay exactly as they are.
//! 2. **A tenant object that no longer exists → dropped through
//!    [`crate::Tenancy::remove_tenant`]** (which takes their plan and writes the
//!    history entry) and never by editing `tenants`/`homed`/`funding` directly (§3.5
//!    S11) — `homed` belongs to the ending sweep, and a roster edited around the
//!    constructor is exactly the shape `assess_at`'s roster check exists to catch.
//!    The **last** tenant cannot be removed that way (it would end a tenancy nobody
//!    resolved), so that case is paged instead.
//! 3. **An exact tenancy whose obligation is missing → paged**, and left in whatever
//!    status it was in. Never `EvictionPending`: "the record we mirror is gone" is not
//!    evidence that anybody missed a payment.

use mush_core::module::{ModError, ModuleBootData};
use mush_core::{Ctx, ObjId};
use mush_economy::{EscalationKind, EscalationSubject, ObligationStatus};

use crate::ids::{PropertyId, TenancyId};
use crate::module::{
    AssessState, HousingStore, MAX_PENDING_NOTICES, META_KEY, PendingNotice, alive, is_live_room,
    put_property, put_tenancy,
};
use crate::record::{Funding, Property, PropertyStatus, Tenancy};

/// The most level-2 records one boot may open (the economy's `MAX_BOOT_ESCALATIONS`
/// rule): a world that lost a whole region of rooms must not mint a thousand alerts.
/// Past this the property is still withdrawn and the reason is still logged.
pub const MAX_BOOT_PAGES: usize = 16;

/// Parse the persisted documents into a fresh store.
///
/// # Errors
///
/// [`ModError`] for a bad key or an unparsable **container** document — the boot
/// fails rather than starting a world in which a home does not exist.
pub(crate) fn load(data: &ModuleBootData) -> Result<HousingStore, ModError> {
    let mut store = HousingStore::default();
    let mut meta: Option<&serde_json::Value> = None;
    let mut max_property: Option<u64> = None;
    let mut max_tenancy: Option<u64> = None;

    for (key, value) in &data.kv {
        if key == META_KEY {
            meta = Some(value);
        } else if let Some(id) = key.strip_prefix("property:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("housing: bad property key {key:?}")))?;
            // The id floors the allocator whether or not the document parses: an id
            // that was minted was minted, and re-minting it would attach a live title
            // record and a live tenancy to a different residence.
            max_property = Some(max_property.map_or(id, |m: u64| m.max(id)));
            let p = serde_json::from_value::<Property>(value.clone())
                .map_err(|e| ModError::msg(format!("housing: bad property doc {key:?}: {e}")))?;
            if p.id != PropertyId(id) {
                return Err(ModError::msg(format!(
                    "housing: property doc {key:?} claims id {}",
                    p.id
                )));
            }
            store.properties.insert(p.id, p);
        } else if let Some(id) = key.strip_prefix("tenancy:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("housing: bad tenancy key {key:?}")))?;
            max_tenancy = Some(max_tenancy.map_or(id, |m: u64| m.max(id)));
            let t = serde_json::from_value::<Tenancy>(value.clone())
                .map_err(|e| ModError::msg(format!("housing: bad tenancy doc {key:?}: {e}")))?;
            if t.id != TenancyId(id) {
                return Err(ModError::msg(format!(
                    "housing: tenancy doc {key:?} claims id {}",
                    t.id
                )));
            }
            store.tenancies.insert(t.id, t);
        } else if let Some(id) = key.strip_prefix("assess:") {
            let Ok(id) = id.parse::<u64>() else {
                tracing::warn!(%key, "housing: bad assessment-cursor key; ignoring");
                continue;
            };
            match serde_json::from_value::<AssessState>(value.clone()) {
                Ok(s) => {
                    store.assess.insert(TenancyId(id), s);
                }
                Err(error) => tracing::warn!(
                    %key, %error,
                    "housing: corrupt assessment cursor; re-seeding from the tenancy's clock"
                ),
            }
        } else if let Some(id) = key.strip_prefix("notice:") {
            let Ok(id) = id.parse::<u32>() else {
                tracing::warn!(%key, "housing: bad notice key; ignoring");
                continue;
            };
            match serde_json::from_value::<Vec<PendingNotice>>(value.clone()) {
                Ok(mut list) => {
                    // The cap is enforced on load as well as on write: a hand-written
                    // list must not become an unbounded render at reconnect.
                    if list.len() > MAX_PENDING_NOTICES {
                        let drop = list.len() - MAX_PENDING_NOTICES;
                        list.drain(0..drop);
                    }
                    if !list.is_empty() {
                        store.notices.insert(ObjId(id), list);
                    }
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "housing: corrupt pending-notice doc; skipping");
                }
            }
        } else {
            tracing::warn!(%key, "housing: unknown persisted document; ignoring");
        }
    }

    // Allocators: the stored high-water mark, floored past every parsable key, and
    // **saturating** (E4b1's B1) — a single `property:18446744073709551615` parks the
    // allocator at the ceiling, where minting refuses, instead of wrapping to zero
    // and re-minting live ids.
    store.next_property = meta_u64(meta, "next_property_id").max(next_after(max_property));
    store.next_tenancy = meta_u64(meta, "next_tenancy_id").max(next_after(max_tenancy));
    store.cursor = meta_u64(meta, "assess_cursor");
    Ok(store)
}

fn meta_u64(meta: Option<&serde_json::Value>, field: &str) -> u64 {
    meta.and_then(|m| m.get(field)).and_then(serde_json::Value::as_u64).unwrap_or(0)
}
/// One past the largest id seen, saturating (never `m + 1`).
fn next_after(max: Option<u64>) -> u64 {
    max.map_or(0, |m| m.saturating_add(1))
}

/// Reconcile the loaded records against the live world (08d §4).
pub(crate) fn reconcile(ctx: &mut Ctx<'_>) {
    let mut pages = 0usize;
    reconcile_properties(ctx, &mut pages);
    reconcile_tenants(ctx, &mut pages);
    reconcile_exact(ctx, &mut pages);
    reconcile_let_status(ctx);
    crate::module::persist_meta(ctx);
}

/// Pass 1 — a property whose anchor or rooms went away.
fn reconcile_properties(ctx: &mut Ctx<'_>, pages: &mut usize) {
    let broken: Vec<(Property, Vec<ObjId>)> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        store
            .properties()
            .filter(|p| p.status != PropertyStatus::Withdrawn)
            .filter_map(|p| {
                let gone: Vec<ObjId> =
                    p.rooms.iter().copied().filter(|r| !is_live_room(ctx.w, *r)).collect();
                (!gone.is_empty()).then(|| (p.clone(), gone))
            })
            .collect()
    };
    for (mut p, gone) in broken {
        let id = p.id;
        let missing = gone.iter().map(|r| format!("#{}", r.0)).collect::<Vec<_>>().join(", ");
        let anchor_gone = gone.contains(&p.anchor);
        p.status = PropertyStatus::Withdrawn;
        p.revision = p.revision.saturating_add(1);
        put_property(ctx, p);
        tracing::error!(
            property = id.0,
            %missing,
            anchor_gone,
            "housing: a property's rooms are missing or GOING; withdrawn, tenancies untouched"
        );
        if *pages < MAX_BOOT_PAGES {
            *pages += 1;
            mush_economy::open_escalation(
                ctx,
                EscalationKind::BootReconciliation,
                EscalationSubject::HousingProperty(id.0),
                format!(
                    "property #{id}'s room(s) {missing} are gone or GOING{}; it is withdrawn and \
                     every tenancy on it is untouched. A home does not end because its rooms \
                     vanished — rebuild the rooms and @property/rooms them back, or end the \
                     tenancy deliberately",
                    if anchor_gone { " (the anchor among them)" } else { "" }
                ),
                vec![
                    format!("@property/show {id}"),
                    format!("@tenancy/list property={id}"),
                ],
            );
        }
    }
}

/// Pass 2 — a tenant object that no longer exists.
fn reconcile_tenants(ctx: &mut Ctx<'_>, pages: &mut usize) {
    let candidates: Vec<(TenancyId, Vec<ObjId>)> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        store
            .tenancies()
            .filter(|t| !t.status.is_ended())
            .filter_map(|t| {
                let gone: Vec<ObjId> =
                    t.tenants.iter().copied().filter(|w| !alive(ctx.w, *w)).collect();
                (!gone.is_empty()).then_some((t.id, gone))
            })
            .collect()
    };
    for (id, gone) in candidates {
        // The cadence position is the `now` every housing arrow takes; a tenancy
        // whose clock is unresolvable is stamped at 0 rather than at wall time, which
        // would be a position on a clock this record has never been on.
        let now = tenancy_now(ctx, id).unwrap_or(0);
        for who in gone {
            let Some(t) = ctx.w.module_state::<HousingStore>().and_then(|s| s.tenancy(id).cloned())
            else {
                continue;
            };
            // The library refuses the LAST tenant deliberately: removing them would
            // end a tenancy nobody resolved (D-E5-1). Page instead.
            if t.tenants.len() <= 1 {
                tracing::error!(
                    tenancy = id.0,
                    who = who.0,
                    "housing: every tenant of a live tenancy is gone; paging rather than ending it"
                );
                if *pages < MAX_BOOT_PAGES {
                    *pages += 1;
                    mush_economy::open_escalation(
                        ctx,
                        EscalationKind::BootReconciliation,
                        EscalationSubject::HousingTenancy(id.0),
                        format!(
                            "tenancy #{id}'s last tenant (#{}) no longer exists. Nothing ends a \
                             home automatically, so it is untouched: decide it",
                            who.0
                        ),
                        vec![
                            format!("@tenancy/show {id}"),
                            format!("@tenancy/resolve {id}=end"),
                        ],
                    );
                }
                break;
            }
            match t.remove_tenant(who, now) {
                Ok(next) => {
                    tracing::warn!(
                        tenancy = id.0,
                        who = who.0,
                        "housing: a tenant object is gone; dropped from the roster"
                    );
                    put_tenancy(ctx, next);
                }
                Err(error) => tracing::error!(
                    tenancy = id.0,
                    who = who.0,
                    %error,
                    "housing: could not drop a destroyed tenant; leaving the roster alone"
                ),
            }
        }
    }
}

/// Pass 3 — an exact tenancy whose obligation is gone.
fn reconcile_exact(ctx: &mut Ctx<'_>, pages: &mut usize) {
    let orphans: Vec<(TenancyId, u64)> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        let econ = ctx.w.module_state::<mush_economy::EconomyStore>();
        store
            .tenancies()
            .filter(|t| t.status.is_live())
            .filter_map(|t| {
                let Funding::Exact { obligation, .. } = &t.funding else { return None };
                let known = econ.is_some_and(|e| {
                    e.snapshot()
                        .obligation(*obligation)
                        .is_some_and(|o| !matches!(o.status, ObligationStatus::Cancelled))
                });
                (!known).then_some((t.id, obligation.0))
            })
            .collect()
    };
    for (id, obligation) in orphans {
        tracing::error!(
            tenancy = id.0,
            obligation,
            "housing: an exact tenancy's rent obligation is missing or cancelled; paging"
        );
        if *pages < MAX_BOOT_PAGES {
            *pages += 1;
            mush_economy::open_escalation(
                ctx,
                EscalationKind::BootReconciliation,
                EscalationSubject::HousingTenancy(id.0),
                format!(
                    "tenancy #{id} is funded by obligation #{obligation}, which is missing or \
                     cancelled. Its status is untouched — a record we mirror going away is not \
                     evidence anybody missed a payment, so it is NOT eviction-pending"
                ),
                vec![
                    format!("@tenancy/show {id}"),
                    format!("@obligation/show {obligation}"),
                ],
            );
        }
    }
}

/// Pass 4 — the `Let`/`Vacant` latch follows the tenancies, not the other way round.
///
/// It is a projection, so boot re-derives it rather than trusting it: a property with
/// a live tenancy is `Let` (the access hook and the destroy gate both key off that),
/// and one without is `Vacant`. `Withdrawn` is never re-derived away — it is a staff
/// decision or pass 1's, and only `@property/rooms` un-withdraws.
fn reconcile_let_status(ctx: &mut Ctx<'_>) {
    let fixes: Vec<Property> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        store
            .properties()
            .filter(|p| p.status != PropertyStatus::Withdrawn)
            .filter_map(|p| {
                let want = if store.has_live_tenancy(p.id) {
                    PropertyStatus::Let
                } else {
                    PropertyStatus::Vacant
                };
                (p.status != want).then(|| {
                    let mut next = p.clone();
                    next.status = want;
                    next.revision = next.revision.saturating_add(1);
                    next
                })
            })
            .collect()
    };
    for p in fixes {
        tracing::warn!(property = p.id.0, status = ?p.status, "housing: re-derived the let latch at boot");
        put_property(ctx, p);
    }
}

/// "Now" on one tenancy's own cadence clock.
pub(crate) fn tenancy_now(ctx: &Ctx<'_>, id: TenancyId) -> Option<i64> {
    let store = ctx.w.module_state::<HousingStore>()?;
    let t = store.tenancy(id)?;
    crate::module::cadence_now(ctx.w, t.cadence)
}

/// Every tenancy id, in id order (the rotation the tick walks).
#[must_use]
pub(crate) fn tenancy_ids(store: &HousingStore) -> Vec<TenancyId> {
    store.tenancies.keys().copied().collect()
}


