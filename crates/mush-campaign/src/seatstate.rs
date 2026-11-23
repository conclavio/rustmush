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
    // P2-MOD-59: plain read-only `+muster` (`lock == false`) never consults the
    // OOC account oracle at all, since it discards `accounts_distinct`.
    let accounts_distinct = if lock {
        let players_distinct = ctx
            .w
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.all_accounts_distinct(&characters));
        match players_distinct {
            None => None,
            Some(false) => Some(false),
            Some(true) if accounts.iter().any(Option::is_none) => None,
            Some(true) => {
                let mut seen = std::collections::BTreeSet::new();
                Some(accounts.iter().flatten().all(|id| seen.insert(*id)))
            }
        }
    } else {
        None
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

/// Advance a `(operation, offer)` rotation cursor strictly past `key`, wrapping
/// cleanly to `(0, 0)` at `(u64::MAX, u64::MAX)` (P2-MOD-58).
const fn next_seat_cursor((op, offer): (u64, u64)) -> (u64, u64) {
    match offer.checked_add(1) {
        Some(next_offer) => (op, next_offer),
        None => (op.wrapping_add(1), 0),
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
/// Bounded on grants **promoted** with a rotating `(operation, offer)` cursor
/// (P2-MOD-58); unactivated dormant grants are skipped before consuming budget
/// so they can never starve promotion of active seats. A re-run past the applied
/// point is a no-op, so a reboot re-running the sweep changes nothing.
pub(crate) fn observe_activations(ctx: &mut mush_core::Ctx<'_>, budget: usize) -> usize {
    observe_activations_with(ctx, budget, mush_sim::channel_lease_holder)
}

fn observe_activations_with(
    ctx: &mut mush_core::Ctx<'_>,
    budget: usize,
    holder_of: impl Fn(
        &mush_core::World,
        ObjId,
        &mush_core::ChannelId,
    ) -> Option<mush_core::PrincipalId>,
) -> usize {
    if budget == 0 {
        return 0;
    }
    let (candidates, next_cursor) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        let cursor = store.seat_observe_cursor;
        let eligible: Vec<(crate::seat::SeatGrant, ObjId)> = store
            .seat_grants()
            .filter(|grant| matches!(grant.state, GrantState::Dormant))
            .filter_map(|grant| {
                let offer = store.seat_offer(grant.operation, grant.offer)?;
                let channel_name = offer.scope.channels.iter().next()?;
                let channel = mush_core::ChannelId::new(channel_name.as_str())?;
                let unit = offer.scope.unit.0;
                let held_by_seat = holder_of(ctx.w, unit, &channel)
                    == Some(mush_core::PrincipalId::Object { id: grant.actor.0 });
                held_by_seat.then(|| (grant.clone(), unit))
            })
            .collect();
        let chosen: Vec<(crate::seat::SeatGrant, ObjId)> = eligible
            .iter()
            .filter(|(g, _)| (g.operation.0, g.offer.0) >= cursor)
            .chain(
                eligible
                    .iter()
                    .filter(|(g, _)| (g.operation.0, g.offer.0) < cursor),
            )
            .take(budget)
            .cloned()
            .collect();
        let next = chosen
            .last()
            .map(|(g, _)| next_seat_cursor((g.operation.0, g.offer.0)));
        (chosen, next)
    };
    if let (Some(next), Some(store)) = (next_cursor, ctx.w.module_state_mut::<CampaignStore>()) {
        store.seat_observe_cursor = next;
    }
    let mut promoted = 0usize;
    for (grant, unit) in candidates {
        // The seat really is live: record the epoch the grant is now current
        // under, read from the unit's own control generation.
        let epoch = ctx
            .w
            .module_state::<mush_sim::SpatialState>()
            .map_or(0, |state| state.control_generation(unit));
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
/// rather than surviving to a later one.** Bounded on grants **worked** with a
/// rotating `(operation, offer)` cursor (P2-MOD-58).
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
    if budget == 0 {
        return 0;
    }
    let now = ctx.w.world_ic_now();
    // P2-MOD-58: rotate across stale grants via `seat_expire_cursor` and skip
    // duplicate same-actor live stand-downs and already-paged custodied seats
    // before consuming budget so blocked/custodied live seats cannot starve
    // expiry of later stale grants.
    let (stale, next_cursor) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        let cursor = store.seat_expire_cursor;
        let all_stale: Vec<(crate::seat::SeatGrant, Option<(ObjId, SmolStr)>)> = store
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
            .map(|grant| {
                let target = store
                    .seat_offer(grant.operation, grant.offer)
                    .map(|offer| (offer.scope.unit.0, offer.scope.station.clone()));
                (grant.clone(), target)
            })
            .collect();
        let rotated = all_stale
            .iter()
            .filter(|(g, _)| (g.operation.0, g.offer.0) >= cursor)
            .chain(
                all_stale
                    .iter()
                    .filter(|(g, _)| (g.operation.0, g.offer.0) < cursor),
            );
        let mut seen_live_actors: std::collections::BTreeSet<ObjId> =
            std::collections::BTreeSet::new();
        let mut chosen = Vec::new();
        let mut last_visited: Option<(u64, u64)> = None;
        for (grant, target) in rotated {
            if chosen.len() >= budget {
                break;
            }
            if seat_is_live(ctx, grant) {
                let resolved_unit = target.as_ref().map(|(u, _)| *u).or_else(|| {
                    ctx.w
                        .module_state::<mush_sim::FocusState>()
                        .and_then(|state| state.focus_of_actor(grant.actor.0))
                        .filter(|focus| focus.proxy)
                        .map(|focus| focus.unit)
                });
                if let Some(unit) = resolved_unit
                    && mush_sim::deployed::is_custodied(ctx.w, unit)
                {
                    let subject =
                        SmolStr::new(format!("seat:{}:{}", grant.operation.0, grant.offer.0));
                    if store.paged.contains(&subject) {
                        continue;
                    }
                }
                if !seen_live_actors.insert(grant.actor.0) {
                    continue;
                }
            }
            last_visited = Some((grant.operation.0, grant.offer.0));
            chosen.push((grant.clone(), target.clone()));
        }
        let next = last_visited.map(next_seat_cursor);
        (chosen, next)
    };
    if let (Some(next), Some(store)) = (next_cursor, ctx.w.module_state_mut::<CampaignStore>()) {
        store.seat_expire_cursor = next;
    }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use mush_core::{ChannelId, ClockRef, ClockStamp, Ctx, GOD, ObjId, Outbox, PrincipalId};
    use mush_testkit::TestWorld;
    use smol_str::SmolStr;

    use super::*;
    use crate::ids::OperationId;
    use crate::record::AudienceSpec;
    use crate::seat::{
        DriverToken, EligibilitySpec, FallbackSpec, RevocationTerms, SeatOffer, SeatOfferId,
        SeatScope,
    };

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    fn make_offer(op: OperationId, id: SeatOfferId, unit: ObjId) -> SeatOffer {
        let mut offer = SeatOffer {
            id,
            operation: op,
            audience: AudienceSpec {
                members: vec![mush_org::PrincipalId::Object(GOD)],
            },
            eligibility: EligibilitySpec::default(),
            expires: stamp(10_000),
            scope: SeatScope {
                unit: UnitRef(unit),
                station: SmolStr::new_static("pilot"),
                channels: BTreeSet::from([SmolStr::new_static("movement")]),
                mission_order: SmolStr::new_static("hold"),
                roe: SmolStr::new_static("operation"),
                permitted_area: None,
            },
            fallback: FallbackSpec {
                holder: UnitRef(ObjId(unit.0 + 1_000)),
            },
            revocation: RevocationTerms {
                by_commander: true,
                on_operation_end: true,
            },
            consent_hash: [0; 32],
        };
        offer.consent_hash = crate::seat::seat_consent_hash(&offer);
        offer
    }

    fn make_grant(op: OperationId, id: SeatOfferId, npc: ObjId) -> crate::seat::SeatGrant {
        crate::seat::SeatGrant {
            operation: op,
            offer: id,
            actor: UnitRef(npc),
            driver: DriverToken(format!("{:016x}{:016x}", op.0, id.0).into()),
            accepted_at: stamp(1),
            consent_hash: [1; 32],
            state: GrantState::Dormant,
        }
    }

    /// **P2-MOD-58a.** `observe_activations` skips unactivated `Dormant` grants
    /// before consuming its 32-slot cap budget AND rotates via
    /// `seat_observe_cursor` when more than `budget` grants are live at once.
    #[test]
    fn p2_mod_58_observe_activations_skips_unactivated_and_rotates_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tw = TestWorld::with_modules(vec![Box::new(crate::CampaignModule::new())]);
        let movement = ChannelId::new("movement").ok_or("channel id")?;
        let mut leases: BTreeMap<(ObjId, ChannelId), PrincipalId> = BTreeMap::new();

        // 1. Seed 32 unactivated dormant grants on op 1 (no lease held), followed
        //    by 1 live dormant grant on op 2 whose lease IS held.
        let mut out = Outbox::new();
        {
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::store::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            for i in 1..=32_u64 {
                let unit = ObjId(100 + u32::try_from(i)?);
                let npc = ObjId(200 + u32::try_from(i)?);
                let _ = crate::store::put_seat_offer(
                    &mut ctx,
                    make_offer(OperationId(1), SeatOfferId(i), unit),
                );
                let _ = crate::store::put_seat_grant(
                    &mut ctx,
                    make_grant(OperationId(1), SeatOfferId(i), npc),
                );
            }
            let live_unit = ObjId(500);
            let live_npc = ObjId(600);
            let _ = crate::store::put_seat_offer(
                &mut ctx,
                make_offer(OperationId(2), SeatOfferId(1), live_unit),
            );
            let _ = crate::store::put_seat_grant(
                &mut ctx,
                make_grant(OperationId(2), SeatOfferId(1), live_npc),
            );
            leases.insert(
                (live_unit, movement.clone()),
                PrincipalId::Object { id: live_npc },
            );
        }

        let promoted = {
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::store::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            observe_activations_with(&mut ctx, crate::MAX_SEATS_PER_FIRE, |_, unit, chan| {
                leases.get(&(unit, chan.clone())).cloned()
            })
        };
        if promoted != 1 {
            return Err(format!("expected 1 promoted seat, got {promoted}").into());
        }
        let promoted_state = tw
            .world()
            .module_state::<CampaignStore>()
            .and_then(|s| s.seat_grant(OperationId(2), SeatOfferId(1)))
            .map(|g| g.state.clone());
        if !matches!(promoted_state, Some(GrantState::Active { .. })) {
            return Err(
                "live seat behind 32 unactivated dormant grants must promote in one tick".into(),
            );
        }

        // 2. Seed 4 live grants across ops 3..=6 and run with budget=2 so
        //    `seat_observe_cursor` rotates cleanly across fires.
        {
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::store::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            for op_n in 3..=6_u64 {
                let unit = ObjId(700 + u32::try_from(op_n)?);
                let npc = ObjId(800 + u32::try_from(op_n)?);
                let _ = crate::store::put_seat_offer(
                    &mut ctx,
                    make_offer(OperationId(op_n), SeatOfferId(1), unit),
                );
                let _ = crate::store::put_seat_grant(
                    &mut ctx,
                    make_grant(OperationId(op_n), SeatOfferId(1), npc),
                );
                leases.insert((unit, movement.clone()), PrincipalId::Object { id: npc });
            }
        }
        let (first_batch, second_batch) = {
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::store::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let a = observe_activations_with(&mut ctx, 2, |_, unit, chan| {
                leases.get(&(unit, chan.clone())).cloned()
            });
            let b = observe_activations_with(&mut ctx, 2, |_, unit, chan| {
                leases.get(&(unit, chan.clone())).cloned()
            });
            (a, b)
        };
        if first_batch != 2 || second_batch != 2 {
            return Err(
                format!("expected batches (2, 2), got ({first_batch}, {second_batch})").into(),
            );
        }
        let store = tw
            .world()
            .module_state::<CampaignStore>()
            .ok_or("campaign store")?;
        for op_n in 3..=6_u64 {
            if !matches!(
                store
                    .seat_grant(OperationId(op_n), SeatOfferId(1))
                    .map(|g| &g.state),
                Some(GrantState::Active { .. })
            ) {
                return Err(format!("op {op_n} grant must be promoted after rotation").into());
            }
        }
        Ok(())
    }
}
