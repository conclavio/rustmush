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

/// **Mint the opaque driver token and record `token -> account` outside the
/// world.** `None` = refuse the grant; the token itself is minted server-side
/// from `seed` plus a process-scoped nonce, so the world cannot recompute it
/// (fold E7(a)/K5).
pub(crate) fn audit_record(
    ctx: &mut mush_core::Ctx<'_>,
    seed: &str,
    account: AccountId,
) -> Option<SmolStr> {
    ctx.w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.clone())
        .and_then(|audit| audit.record(seed, account))
        .map(|token| SmolStr::new(&token))
}

/// Whether an attribution row exists — a boolean, never an identity.
pub(crate) fn audit_knows(ctx: &mush_core::Ctx<'_>, token: &str) -> bool {
    ctx.w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.as_ref())
        .is_some_and(|audit| audit.knows(token))
}

/// **Whether the row for `token` names `account`** (fold A2/A5).
///
/// Cut 10's only attribution question was [`audit_knows`] — "does ANY row
/// exist" — so a second account activated a grant somebody else accepted and the
/// one attribution row named a person who was not driving, and `+seat/leave`
/// stood down whichever grant sorted first on the station rather than the
/// caller's own. This is the identity question, asked **inside** the seam: the
/// account id never leaves this function and is never rendered.
pub(crate) fn audit_is(ctx: &mush_core::Ctx<'_>, token: &str, account: AccountId) -> bool {
    ctx.w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.as_ref())
        .and_then(|audit| audit.account_for(token))
        == Some(account)
}

/// **The staff attribution answer** (fold K6): whether `token`'s row names the
/// same account as `who`'s character, without either id leaving this function.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuditVerdict {
    /// The row names that character's account.
    Same,
    /// The row names a different account.
    Different,
    /// There is no row, or the character's account cannot be resolved.
    Unknown,
}

/// Resolve one attribution question for the staff verb. Prints nothing, returns
/// no id.
pub(crate) fn audit_verdict(ctx: &mush_core::Ctx<'_>, token: &str, who: ObjId) -> AuditVerdict {
    let Some(row) = ctx
        .w
        .module_state::<SeatAuditHolder>()
        .and_then(|holder| holder.0.as_ref())
        .and_then(|audit| audit.account_for(token))
    else {
        return AuditVerdict::Unknown;
    };
    match account_of(ctx, who) {
        Some(account) if account == row => AuditVerdict::Same,
        Some(_) => AuditVerdict::Different,
        None => AuditVerdict::Unknown,
    }
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
    // `take_muster` is called **only** from `+muster`; the tick's two arms are
    // `observe_activations` and `expire_grants` (fold correction (4)).
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
    // A9: the player-held slots DO name characters, so the distinctness oracle
    // itself answers over them — `AccountOracle::all_accounts_distinct` had no
    // production caller at all before this fold. A proxy slot names no character
    // (the focus is keyed by account), so its half still compares accounts, and
    // neither id ever leaves this function.
    let mut characters: Vec<ObjId> = Vec::new();
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
                        characters.push(actor);
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
    let players_distinct = ctx
        .w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.all_accounts_distinct(&characters));
    let accounts_distinct = match players_distinct {
        None => None,
        Some(false) => Some(false),
        Some(true) if accounts.iter().any(Option::is_none) => None,
        Some(true) => {
            let mut seen = std::collections::BTreeSet::new();
            Some(accounts.iter().flatten().all(|id| seen.insert(*id)))
        }
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
/// revocation's bump makes it stale by construction. **B11 (fold):** that was
/// the justification and there was no comparison anywhere — the only match on
/// the variant discarded the payload. `seatcmd::seat_activate` compares it now,
/// and a grant recorded under a generation the unit has bumped past is stale.
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
/// rather than surviving to a later one.** Bounded on grants **worked**.
///
/// ## What the fold changed
///
/// **B2 — the budget bounds work, not the scan.** Cut 10 read
/// `.filter(!is_terminal).take(budget).filter(stale)`, so the staleness
/// predicate ran *after* the take and the arm kept no cursor: with 32 healthy
/// non-terminal grants ahead of it, a grant on an aborted operation never
/// expired and the reaper re-visited the same prefix for ever. The predicate now
/// runs inside the same `filter`, and `.take(budget)` is last.
///
/// **A4 — a LIVE seat is stood down before it is expired.** Cut 10 wrote
/// `Expired` and called `audit_forget` while the billet, the narrowing, the
/// proxy focus and the K6 lease all survived — after which `+seat/leave`
/// answered "You are driving no seat" and `@seat/revoke` refused on
/// `is_terminal()`, so control persisted un-attributed with no exit but a
/// disconnect. The sweep now stands the seat down **through the one seam**
/// (`request_seat_activation(&SeatRequest::Revoke { .. })`) before writing
/// `Expired` and before forgetting the row, at most one staged submit per actor
/// per fire (cut 2's fold C4), and **when the revoke itself refuses the grant is
/// left non-terminal and staff are paged** rather than expired anyway.
pub(crate) fn expire_grants(ctx: &mut mush_core::Ctx<'_>, budget: usize) -> usize {
    let now = ctx.w.world_ic_now();
    let stale: Vec<(crate::seat::SeatGrant, Option<(ObjId, SmolStr)>)> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        store
            .seat_grants()
            .filter(|grant| {
                if grant.state.is_terminal() {
                    return false;
                }
                let operation_over = store
                    .operation(grant.operation)
                    .is_none_or(|doc| doc.runtime.state.is_terminal());
                let offer_lapsed = store
                    .seat_offer(grant.operation, grant.offer)
                    .is_none_or(|offer| offer.is_expired(&now));
                operation_over || (matches!(grant.state, GrantState::Dormant) && offer_lapsed)
            })
            .take(budget)
            .map(|grant| {
                let target = store
                    .seat_offer(grant.operation, grant.offer)
                    .map(|offer| (offer.scope.unit.0, offer.scope.station.clone()));
                (grant.clone(), target)
            })
            .collect()
    };
    let mut expired = 0usize;
    let mut stood_down: std::collections::BTreeSet<ObjId> = std::collections::BTreeSet::new();
    for (grant, target) in stale {
        if seat_is_live(ctx, &grant) {
            // One staged submit per actor per drain: a second live seat on the
            // same NPC waits for the next fire rather than voiding this one.
            if !stood_down.insert(grant.actor.0) {
                continue;
            }
            if !release_expiring_seat(ctx, &grant, target) {
                continue;
            }
        }
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

/// Whether a grant is somebody's live seat right now: `Active`, or its NPC
/// carries a proxy focus (an `Active` grant across a restart has the billet and
/// the narrowing but no focus, and must still be stood down).
fn seat_is_live(ctx: &mush_core::Ctx<'_>, grant: &crate::seat::SeatGrant) -> bool {
    if matches!(grant.state, GrantState::Active { .. }) {
        return true;
    }
    ctx.w
        .module_state::<mush_sim::FocusState>()
        .and_then(|state| state.focus_of_actor(grant.actor.0))
        .is_some_and(|focus| focus.proxy)
}

/// Stand one expiring seat down through the ONE seam. `false` = the revoke
/// refused, staff were paged, and the caller must leave the grant non-terminal.
fn release_expiring_seat(
    ctx: &mut mush_core::Ctx<'_>,
    grant: &crate::seat::SeatGrant,
    target: Option<(ObjId, SmolStr)>,
) -> bool {
    let target = target.or_else(|| {
        ctx.w
            .module_state::<mush_sim::FocusState>()
            .and_then(|state| state.focus_of_actor(grant.actor.0))
            .filter(|focus| focus.proxy)
            .map(|focus| (focus.unit, SmolStr::new(focus.station.as_str())))
    });
    let refused = match target {
        // **16b cut T.3, admission site 6 of 6** (doc 16 §3.1 "Custody"): the
        // campaign handoff's own stand-down, refused for the same reason
        // `@seat/revoke` is — it reaches no arena, and the page below is exactly
        // the right outcome, because staff ending the match is what frees it.
        Some((unit, _)) if mush_sim::deployed::is_custodied(ctx.w, unit) => true,
        Some((unit, station)) => match mush_sim::StationId::new(station.as_str()) {
            Some(station) => mush_sim::request_seat_activation(
                ctx,
                &mush_sim::SeatRequest::Revoke {
                    unit,
                    station,
                    npc: grant.actor.0,
                },
            )
            .is_err(),
            None => true,
        },
        None => true,
    };
    if refused {
        crate::settle::page(
            ctx,
            SmolStr::new(format!("seat:{}:{}", grant.operation.0, grant.offer.0)),
            "an operation ended while a seat was still being driven and the seat could not be \
             stood down",
            &format!(
                "seat {} on operation {} is still live after its operation ended, and the \
                 automatic stand-down refused. The grant is deliberately left NON-TERMINAL so it \
                 still has an exit: end it with `@seat/revoke {}/{}`",
                grant.offer, grant.operation, grant.operation.0, grant.offer.0
            ),
        );
        return false;
    }
    true
}

/// The unit a seat drives, for the muster's own bounded scan.
#[must_use]
pub fn unit_of(unit: UnitRef) -> ObjId {
    unit.0
}
