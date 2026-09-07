//! E7 cut 10 — the seat lane's world-state seams: the server-side audit holder,
//! the account reads, the muster pull and the `Dormant → Active` observation.
//!
//! **What this module reaches into `mush-sim` for, and why that is not a third
//! seam.** Cut 10's *mechanism* crosses the new edge through exactly two
//! exported `pub fn`s (`request_seat_activation`, `channel_lease_holder`). What
//! this module additionally reads — `mush_sim::AccountOracle`,
//! `mush_sim::FocusState` and `mush_sim::SpatialState` — are **pre-existing
//! public world-state types cut 5 and cut 2 already exported**, read through
//! `World::module_state`, with no mutation and no authority. They are how the
//! muster answers "who is actually in this seat", which is the question a muster
//! *is*.

use std::collections::BTreeMap;

use mush_core::{AccountId, ObjId, SeatAudit};
use smol_str::SmolStr;

use crate::record::{OperationDoc, UnitRef};
use crate::seat::{GrantState, MAX_MUSTER_SLOTS, MusterReport, MusterSlot, SlotFill};
use crate::store::CampaignStore;

/// The composition-installed seat-attribution log, held in world state on
/// `mush_sim::AccountOracle`'s exact shape.
///
/// `None` = unwired, and **every consumer fails closed**: a seat cannot be taken
/// (there would be no attribution) and a grant with no row cannot be activated.
#[derive(Clone, Default)]
pub struct SeatAuditHolder(Option<std::sync::Arc<dyn SeatAudit>>);

impl std::fmt::Debug for SeatAuditHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeatAuditHolder")
            .field("wired", &self.0.is_some())
            .finish()
    }
}

impl SeatAuditHolder {
    /// Wrap a composition-installed log.
    #[must_use]
    pub fn new(audit: std::sync::Arc<dyn SeatAudit>) -> SeatAuditHolder {
        SeatAuditHolder(Some(audit))
    }

    /// The unwired holder every consumer fails closed against.
    #[must_use]
    pub fn unwired() -> SeatAuditHolder {
        SeatAuditHolder(None)
    }

    /// Whether a log is wired at all.
    #[must_use]
    pub fn is_wired(&self) -> bool {
        self.0.is_some()
    }
}

/// Record `token -> account` outside the world. `false` = refuse the grant.
pub(crate) fn audit_record(ctx: &mut mush_core::Ctx<'_>, token: &str, account: AccountId) -> bool {
    ctx.w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.clone())
        .is_some_and(|audit| audit.record(token, account))
}

/// Whether an attribution row exists — a boolean, never an identity.
pub(crate) fn audit_knows(ctx: &mush_core::Ctx<'_>, token: &str) -> bool {
    ctx.w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.as_ref())
        .is_some_and(|audit| audit.knows(token))
}

/// Forget a terminal grant's row.
pub(crate) fn audit_forget(ctx: &mut mush_core::Ctx<'_>, token: &str) {
    if let Some(audit) = ctx
        .w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.clone())
    {
        audit.forget(token);
    }
}

/// The account a character belongs to, through cut 5's oracle. `None` = unknown
/// = **the caller refuses**, never a default.
pub(crate) fn account_of(ctx: &mush_core::Ctx<'_>, who: ObjId) -> Option<AccountId> {
    ctx.w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(who))
}

/// The `(unit, station)` an account is currently driving **through a proxy
/// seat**, if any. An ordinary `+focus` is not one.
pub(crate) fn proxy_focus_of(ctx: &mush_core::Ctx<'_>, who: ObjId) -> Option<(ObjId, String)> {
    let account = account_of(ctx, who)?;
    let focus = ctx
        .w
        .module_state::<mush_sim::FocusState>()?
        .focus_of_account(account)?;
    focus
        .proxy
        .then(|| (focus.unit, focus.station.as_str().to_owned()))
}

/// Who, if anybody, holds `unit`'s `station` right now — cut 5's billet, read
/// from the published spatial state.
fn seat_holder(ctx: &mush_core::Ctx<'_>, unit: ObjId, station: &str) -> Option<ObjId> {
    let state = ctx.w.module_state::<mush_sim::SpatialState>()?;
    let id = mush_sim::StationId::new(station)?;
    state
        .assignments
        .get(&unit)
        .and_then(|seats| seats.seats.get(&id))
        .map(|seat| seat.actor)
}

/// Every station a unit defines, in id order (bounded on records **visited**).
fn stations_of(ctx: &mush_core::Ctx<'_>, unit: ObjId, budget: usize) -> Vec<String> {
    ctx.w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.stations.get(&unit))
        .map(|host| {
            host.stations
                .keys()
                .take(budget)
                .map(|id| id.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// **Take one muster** (T14): resolve every committed unit's stations to a
/// player character, a human-directed NPC, an AI NPC, automation, or **vacancy**
/// — and vacancy is a resolution, not a cancellation.
///
/// Bounded on records **visited** ([`MAX_MUSTER_SLOTS`], D-LBK-5). It reads the
/// billets, the focus map and this crate's own grants; it never reads a
/// `ContactState`, never a unit's position, and never an account id into the
/// record — only the distinctness **verdict**.
pub(crate) fn take_muster(
    ctx: &mut mush_core::Ctx<'_>,
    doc: &OperationDoc,
    lock: bool,
) -> MusterReport {
    let cycle = ctx
        .w
        .module_state::<CampaignStore>()
        .map_or(1, |store| store.muster_cycle.saturating_add(1));
    let taken_at = ctx.w.world_ic_now();
    // The grants this operation carries, so a human-directed NPC is recognised
    // as one rather than as an AI.
    let driven: BTreeMap<(u32, String), ()> = ctx
        .w
        .module_state::<CampaignStore>()
        .map(|store| {
            store
                .seat_grants()
                .filter(|grant| grant.operation == doc.id() && !grant.state.is_terminal())
                .filter_map(|grant| {
                    store
                        .seat_offer(grant.operation, grant.offer)
                        .map(|offer| ((offer.scope.unit.0.0, offer.scope.station.to_string()), ()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut slots: Vec<MusterSlot> = Vec::new();
    let mut accounts: Vec<Option<AccountId>> = Vec::new();
    let mut visited = 0usize;
    for participant in &doc.runtime.participants {
        if visited >= MAX_MUSTER_SLOTS {
            break;
        }
        let unit = participant.unit;
        for station in stations_of(ctx, unit.0, MAX_MUSTER_SLOTS - visited) {
            if visited >= MAX_MUSTER_SLOTS {
                break;
            }
            visited += 1;
            let holder = seat_holder(ctx, unit.0, &station);
            let is_driven = driven.contains_key(&(unit.0.0, station.clone()));
            let fill = match holder {
                None => SlotFill::Vacant,
                Some(actor) if is_driven => {
                    // A human directs this NPC: its account is the FOCUS's, and
                    // the focus is keyed by account, so no id ever leaves RAM.
                    accounts.push(
                        ctx.w
                            .module_state::<mush_sim::FocusState>()
                            .and_then(|state| state.focus_of_actor(actor))
                            .filter(|focus| focus.proxy)
                            .map(|focus| focus.account),
                    );
                    SlotFill::HumanDirectedNpc
                }
                Some(actor) => match ctx.w.get(actor).map(|object| object.kind) {
                    Some(mush_core::ObjKind::Player) => {
                        accounts.push(account_of(ctx, actor));
                        SlotFill::Player
                    }
                    Some(mush_core::ObjKind::Npc) => SlotFill::AiNpc,
                    _ => SlotFill::Automation,
                },
            };
            slots.push(MusterSlot {
                unit,
                station: SmolStr::new(&station),
                fill,
            });
        }
    }
    // **Only the verdict is stored** (Q11A/R3/C23A/C24A). An unknown account
    // makes the verdict `None`, which **fails closed** at roster lock.
    let accounts_distinct = if accounts.iter().any(Option::is_none) {
        None
    } else {
        let mut seen = std::collections::BTreeSet::new();
        Some(accounts.iter().flatten().all(|id| seen.insert(*id)))
    };
    MusterReport {
        operation: doc.id(),
        cycle,
        taken_at,
        slots,
        accounts_distinct,
        locked: lock && accounts_distinct == Some(true),
    }
}

/// **The `Dormant → Active` observation** — a PULL over durable lease state, run
/// as an arm on cut 9's ONE campaign `Coarse` tick.
///
/// `Active { epoch }` is recorded **only** from what the lease actually shows:
/// the command that requested the handoff never writes it, because
/// `checked_apply_handoff` has not run when the command returns and there is no
/// world-side seam on a scheduled authoritative commit (the cut-6 kernel fact).
/// The epoch recorded is the unit's control generation the seat was observed
/// live under — the exact value `World::channel_grant_is_current` compares, so a
/// revocation's bump makes it stale by construction.
///
/// Bounded on grants **visited**; a re-run past the applied point is a no-op, so
/// a reboot re-running the sweep changes nothing.
pub(crate) fn observe_activations(ctx: &mut mush_core::Ctx<'_>, budget: usize) -> usize {
    let candidates: Vec<(crate::seat::SeatGrant, ObjId, String)> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        store
            .seat_grants()
            .filter(|grant| matches!(grant.state, GrantState::Dormant))
            .take(budget)
            .filter_map(|grant| {
                store.seat_offer(grant.operation, grant.offer).map(|offer| {
                    (
                        grant.clone(),
                        offer.scope.unit.0,
                        offer.scope.station.to_string(),
                    )
                })
            })
            .collect()
    };
    let mut promoted = 0usize;
    for (grant, unit, station) in candidates {
        let Some(channel) = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.seat_offer(grant.operation, grant.offer))
            .and_then(|offer| offer.scope.channels.iter().next().cloned())
        else {
            continue;
        };
        let Some(channel) = mush_core::ChannelId::new(channel.as_str()) else {
            continue;
        };
        let held_by_seat = mush_sim::channel_lease_holder(ctx.w, unit, &channel)
            == Some(mush_core::PrincipalId::Object { id: grant.actor.0 });
        if !held_by_seat {
            continue;
        }
        // The seat really is live: record the epoch the grant is now current
        // under, read from the unit's own control generation.
        let epoch = ctx
            .w
            .module_state::<mush_sim::SpatialState>()
            .map_or(0, |state| state.control_generation(unit));
        let _ = &station;
        let mut live = grant;
        live.state = GrantState::Active { epoch };
        if crate::store::put_seat_grant(ctx, live) {
            promoted += 1;
        }
    }
    promoted
}

/// Expire every dormant grant whose offer has lapsed, and every grant on a
/// terminal operation. **A `Dormant` grant on an aborted operation expires
/// rather than surviving to a later one.** Bounded on grants visited.
pub(crate) fn expire_grants(ctx: &mut mush_core::Ctx<'_>, budget: usize) -> usize {
    let now = ctx.w.world_ic_now();
    let stale: Vec<crate::seat::SeatGrant> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        store
            .seat_grants()
            .filter(|grant| !grant.state.is_terminal())
            .take(budget)
            .filter(|grant| {
                let operation_over = store
                    .operation(grant.operation)
                    .is_none_or(|doc| doc.runtime.state.is_terminal());
                let offer_lapsed = store
                    .seat_offer(grant.operation, grant.offer)
                    .is_none_or(|offer| offer.is_expired(&now));
                operation_over || (matches!(grant.state, GrantState::Dormant) && offer_lapsed)
            })
            .cloned()
            .collect()
    };
    let mut expired = 0usize;
    for grant in stale {
        let token = grant.driver.0.to_string();
        let mut done = grant;
        done.state = GrantState::Expired;
        if crate::store::put_seat_grant(ctx, done) {
            audit_forget(ctx, &token);
            expired += 1;
        }
    }
    expired
}

/// The unit a seat drives, for the muster's own bounded scan.
#[must_use]
pub fn unit_of(unit: UnitRef) -> ObjId {
    unit.0
}
