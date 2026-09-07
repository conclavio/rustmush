//! E7 cut 10 B4–B8 — the seat, crew and muster command surface (10b §3.10).
//!
//! ## Registration groups (K8.5: classify BEFORE the first `reg.command`)
//!
//! | verb | class | why |
//! |---|---|---|
//! | `+seat` | `Authoritative` | `/activate` and `/leave` reach K6's lease through cut 5's ingress (`@station`'s precedent) |
//! | `@seat` | `Authoritative` | `/revoke` reaches the same lease |
//! | `@crew` · `+crewcall` · `+muster` | `LegacyPostImage` | they write only the exempt `campaign.crew` / `campaign.muster` workflow documents |
//!
//! `LegacyPostImage` is restored last, so the module still ends in the class its
//! own documents ride.
//!
//! ## What activation is, mechanically
//!
//! `+seat/activate` is the K6 gate's **two policy refusals** followed by ONE call
//! into `mush_sim::request_seat_activation`, which performs the billet write, the
//! narrowed grant, the focus claim and the `control_handoff_request`. Cut 10
//! opens **no second lease-creation path**, persists **no lease-like record**,
//! and reaches **no eviction path**.
//!
//! `Active { epoch }` is never written here: the grant stays `Dormant` until the
//! campaign tick's seat arm **pulls** the lease and sees the seat actually held
//! (the cut-6 kernel fact — a reaction to durable state is a Coarse pull, never
//! an observer, and there is no world-side seam on a scheduled commit).

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, ChannelId, CmdError, CommandCompatibility, CommandSpec, Ctx, ObjId, match_object,
};
use mush_org::OrgScopeId;
use mush_sim::{SeatError, SeatRequest};
use smol_str::SmolStr;

use crate::ids::OperationId;
use crate::record::UnitRef;
use crate::render::{self, Lens, NO_SUCH_OPERATION, is_staff};
use crate::seat::{
    CrewComplement, DriverToken, EligibilitySpec, FallbackSpec, GrantState, MAX_CREW_NAMED,
    MAX_MUSTER_SLOTS, MAX_SEAT_OFFERS_PER_OPERATION, MusterReport, MusterSlot, RevocationTerms,
    SEAT_OFFER_TTL_SECS, SeatGrant, SeatOffer, SeatOfferId, SeatScope, SlotFill, seat_consent_hash,
};
use crate::store::{
    CampaignStore, persist_meta, put_crew, put_muster, put_seat_grant, put_seat_offer,
};

/// The one refusal for a seat a viewer may not see, may not take, or that does
/// not exist. **Byte-identical across all three**, at every verbosity (§4 rule 4;
/// cut 5 B1's `NO_SUCH_STATION` discipline).
pub const NO_SUCH_SEAT: &str = "No such seat.";

/// The one refusal for a crew surface a viewer holds no authority over.
pub const NO_SUCH_CREW: &str = "No such crew.";

fn deny() -> CmdError {
    CmdError::msg("Permission denied.")
}

fn no_seat() -> CmdError {
    CmdError::msg(NO_SUCH_SEAT)
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "campaign", msg.into());
}

// ---- authority ------------------------------------------------------------

/// **Who may offer or assign a seat** (OPEN 5, as decided): staff, the unit's own
/// controller, or `force.assign` on the formation node that owns the unit —
/// cut 9's vocabulary, resolved through `mush_org::authorize_in_world` against
/// the node's own `OrgScopeId`. A crew is **not** an org scope (R2) and the org
/// tree never mirrors the order of battle; the compartment is the formation's.
fn may_seat(ctx: &Ctx<'_>, unit: UnitRef) -> bool {
    if is_staff(ctx.w, ctx.actor) || mush_core::perm::controls(ctx.w, ctx.actor, unit.0) {
        return true;
    }
    let scopes: Vec<OrgScopeId> = ctx
        .w
        .module_state::<CampaignStore>()
        .map(|store| {
            store
                .formations()
                .take(crate::store::MAX_FORMATION_NODES)
                .filter(|node| node.members.contains(&unit))
                .map(|node| node.scope)
                .collect()
        })
        .unwrap_or_default();
    scopes
        .into_iter()
        .any(|scope| crate::commands::authorized_for(ctx, "force.assign", scope))
}

// ---- parsing --------------------------------------------------------------

fn parse_operation(text: &str) -> Result<OperationId, CmdError> {
    OperationId::parse(text).ok_or_else(|| CmdError::msg(NO_SUCH_OPERATION))
}

/// `<op>/<offer>` — the coordinates every seat verb after `/offer` takes.
fn parse_seat_ref(text: &str) -> Result<(OperationId, SeatOfferId), CmdError> {
    let (op, id) = text.trim().split_once('/').ok_or_else(no_seat)?;
    let operation = OperationId::parse(op).ok_or_else(no_seat)?;
    let offer = SeatOfferId::parse(id).ok_or_else(no_seat)?;
    Ok((operation, offer))
}

/// A seat offer the viewer may see at all. **Existence-hiding**: an offer on an
/// operation the viewer is not in the audience of is byte-identical to one that
/// never existed, and so is a declined offer (which leaves no document at all).
fn visible_offer(ctx: &Ctx<'_>, operation: OperationId, id: SeatOfferId) -> Option<SeatOffer> {
    let (doc, lens) = render::visible_operation(ctx.w, operation, ctx.actor)?;
    let _ = &doc;
    if lens == Lens::Hidden {
        return None;
    }
    ctx.w
        .module_state::<CampaignStore>()?
        .seat_offer(operation, id)
        .cloned()
}

// ---- +seat ----------------------------------------------------------------

/// The `+seat` primary verb.
pub(crate) fn cmd_seat(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("offer") {
        seat_offer(ctx)
    } else if ctx.switches.has("accept") {
        seat_accept(ctx)
    } else if ctx.switches.has("decline") {
        seat_decline(ctx)
    } else if ctx.switches.has("activate") {
        seat_activate(ctx)
    } else if ctx.switches.has("leave") {
        seat_leave(ctx)
    } else if ctx.switches.has("list") || ctx.args.raw().trim().is_empty() {
        seat_list(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: +seat/offer <op>/<unit>/<station>=<npc>[,<channel>…][,area:<where>] · \
             +seat/list · \
             +seat/accept <op>/<id> · +seat/decline <op>/<id> · +seat/activate <op>/<id> · \
             +seat/leave",
        ))
    }
}

/// `+seat/offer <op>/<unit>/<station>=<npc>[,<channel>…]` — publish one seat.
fn seat_offer(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_owned();
    let mut parts = lhs.splitn(3, '/');
    let operation = parse_operation(parts.next().unwrap_or_default())?;
    let unit_ref = parts.next().unwrap_or_default().trim().to_owned();
    let station = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    if unit_ref.is_empty() || station.is_empty() || rhs.is_empty() {
        return Err(CmdError::msg(
            "Usage: +seat/offer <op>/<unit>/<station>=<npc>[,<channel>…][,area:<where>]",
        ));
    }
    let mut fields = rhs.split(',').map(str::trim);
    let npc_ref = fields.next().unwrap_or_default().to_owned();
    // `area:<obj>` is the ONE writer of `SeatScope.permitted_area`. It is inside
    // the hashed consent surface, so it must have a writer AND a reader before
    // it may exist at all (cut 9's fold B13: a hashed field with no writer is a
    // hash-space change waiting for its first one). Its reader is the
    // permitted-area check in `seat_activate`.
    let mut area_ref: Option<String> = None;
    let mut channels: BTreeSet<SmolStr> = BTreeSet::new();
    for field in fields.filter(|c| !c.is_empty()) {
        match field.strip_prefix("area:") {
            Some(area) => area_ref = Some(area.trim().to_owned()),
            None => {
                channels.insert(SmolStr::new(field.to_ascii_lowercase()));
            }
        }
    }
    // The operation must exist AND be visible: an invisible one is refused
    // byte-identically to one that never existed.
    let (doc, _) = render::visible_operation(ctx.w, operation, ctx.actor)
        .ok_or_else(|| CmdError::msg(NO_SUCH_OPERATION))?;
    let unit = UnitRef(match_object(ctx.w, ctx.actor, &unit_ref).map_err(|_| no_seat())?);
    let npc = UnitRef(match_object(ctx.w, ctx.actor, &npc_ref).map_err(|_| no_seat())?);
    // **Existence-hidden authority** (hazard axis 2): a caller without
    // `force.assign` over the unit's own compartment is refused with the SAME
    // line whether the unit and station exist or not.
    if !may_seat(ctx, unit) {
        return Err(no_seat());
    }
    if channels.is_empty() {
        return Err(CmdError::msg(
            "Name at least one control channel the seat may use.",
        ));
    }
    let now = ctx.w.world_ic_now();
    let mut expires = now.clone();
    expires.at = now
        .at
        .checked_add(SEAT_OFFER_TTL_SECS)
        .ok_or_else(|| CmdError::msg("That seat cannot be timed on this clock."))?;
    let id = ctx
        .w
        .module_state_mut::<CampaignStore>()
        .and_then(|store| store.mint_seat_offer(operation))
        .ok_or_else(|| {
            CmdError::msg(format!(
                "That operation already carries {MAX_SEAT_OFFERS_PER_OPERATION} seat offers."
            ))
        })?;
    let permitted_area = match area_ref {
        Some(area) => Some(UnitRef(
            match_object(ctx.w, ctx.actor, &area).map_err(|_| no_seat())?,
        )),
        None => None,
    };
    let scope = SeatScope {
        unit,
        station: SmolStr::new(&station),
        channels,
        mission_order: SmolStr::new(doc.terms.objective.summary.as_str()),
        roe: SmolStr::new_static("operation"),
        permitted_area,
    };
    let mut offer = SeatOffer {
        id,
        operation,
        // Never wider than the operation's own: the seat is visible to exactly
        // the operation's eligible defenders/audience, and to nobody else.
        audience: doc.terms.eligible_defenders.clone(),
        eligibility: EligibilitySpec::default(),
        expires,
        scope,
        revocation: RevocationTerms {
            by_commander: true,
            on_operation_end: true,
        },
        fallback: FallbackSpec { holder: npc },
        consent_hash: [0; 32],
    };
    offer.check().map_err(|e| CmdError::msg(e.to_string()))?;
    offer.consent_hash = seat_consent_hash(&offer);
    if !put_seat_offer(ctx, offer) {
        return Err(CmdError::msg("That seat could not be published."));
    }
    persist_meta(ctx);
    info(
        ctx,
        format!("Published seat {id} on operation {operation}."),
    );
    Ok(())
}

/// `+seat/list` — the viewer's own seats and the offers they may see. Shows
/// **nothing** about an operation the viewer is not in the audience of.
fn seat_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut lines: Vec<String> = Vec::new();
    let visible: Vec<(OperationId, SeatOfferId, SmolStr, bool)> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            info(ctx, "No seats are offered.");
            return Ok(());
        };
        store
            .seat_offers()
            .take(MAX_SEAT_OFFERS_PER_OPERATION * 8)
            .map(|offer| {
                (
                    offer.operation,
                    offer.id,
                    offer.scope.station.clone(),
                    store.seat_grant(offer.operation, offer.id).is_some(),
                )
            })
            .collect()
    };
    for (operation, id, station, taken) in visible {
        // The lens is asked per operation, so a covert operation's seat is
        // invisible rather than merely unnamed.
        if render::visible_operation(ctx.w, operation, ctx.actor).is_none() {
            continue;
        }
        lines.push(format!(
            "  {id} — operation {operation}, station `{station}`{}",
            if taken { " (taken)" } else { "" }
        ));
    }
    if lines.is_empty() {
        info(ctx, "No seats are offered to you.");
    } else {
        info(ctx, format!("Seats offered to you:\n{}", lines.join("\n")));
    }
    Ok(())
}

/// `+seat/accept <op>/<id>` — **accepting seizes nothing.** The grant is
/// `Dormant`, the driver token is minted, and the audit row is recorded outside
/// the world. A grant whose audit row cannot be stored is refused rather than
/// created un-attributed.
fn seat_accept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (operation, id) = parse_seat_ref(ctx.args.raw())?;
    let offer = visible_offer(ctx, operation, id).ok_or_else(no_seat)?;
    let now = ctx.w.world_ic_now();
    if offer.is_expired(&now) {
        return Err(no_seat());
    }
    if !offer.consent_intact() {
        return Err(CmdError::msg(
            "That seat's terms no longer match what was offered; it cannot be accepted.",
        ));
    }
    if ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_grant(operation, id))
        .is_some()
    {
        return Err(no_seat());
    }
    // The account whose ONE foreground focus this seat will consume. `None` =
    // unknown = refuse, never a default (fail closed).
    let account = crate::seatstate::account_of(ctx, ctx.actor)
        .ok_or_else(|| CmdError::msg("Your account is not mapped yet; reconnect and try again."))?;
    let actor = offer.fallback.holder;
    let driver = DriverToken::mint(operation, id, actor, &now);
    // R3/R7: the row lives OUTSIDE the world. If it cannot be stored, the grant
    // is not created — a seat is never taken un-attributed.
    if !crate::seatstate::audit_record(ctx, driver.0.as_str(), account) {
        return Err(CmdError::msg(
            "Seat attribution is unavailable on this server; the seat was not taken.",
        ));
    }
    let grant = SeatGrant {
        operation,
        offer: id,
        actor,
        driver: driver.clone(),
        accepted_at: now,
        consent_hash: offer.consent_hash,
        state: GrantState::Dormant,
    };
    if !put_seat_grant(ctx, grant) {
        return Err(CmdError::msg("That seat could not be taken."));
    }
    info(
        ctx,
        format!(
            "You accept seat {id} on operation {operation}. It is dormant: \
             `+seat/activate {operation}/{id}` when you are ready to drive it."
        ),
    );
    Ok(())
}

/// `+seat/decline <op>/<id>` — **leaves no trace a third party can read.** No
/// document is written at all, so there is nothing for a projection to leak.
fn seat_decline(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (operation, id) = parse_seat_ref(ctx.args.raw())?;
    let _ = visible_offer(ctx, operation, id).ok_or_else(no_seat)?;
    info(ctx, format!("You decline seat {id}."));
    Ok(())
}

/// `+seat/activate <op>/<id>` — the K6 gate, then `mush-sim`.
fn seat_activate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (operation, id) = parse_seat_ref(ctx.args.raw())?;
    let offer = visible_offer(ctx, operation, id).ok_or_else(no_seat)?;
    let grant = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_grant(operation, id))
        .cloned()
        .ok_or_else(no_seat)?;
    match grant.state {
        GrantState::Dormant => {}
        // A revoked grant can never be re-activated, and an expired one is gone.
        GrantState::Revoked | GrantState::Expired => return Err(no_seat()),
        GrantState::Active { .. } => {
            return Err(CmdError::msg("You are already driving that seat."));
        }
    }
    let now = ctx.w.world_ic_now();
    if offer.is_expired(&now) || !offer.consent_intact() {
        return Err(no_seat());
    }
    // **A grant whose audit row is missing refuses activation and pages** — it
    // never activates un-attributed (R3/R7).
    if !crate::seatstate::audit_knows(ctx, grant.driver.0.as_str()) {
        crate::settle::page(
            ctx,
            SmolStr::new(format!("seat:{}:{}", operation.0, id.0)),
            "a seat grant has no attribution row and cannot be activated",
            &format!(
                "seat {id} on operation {operation} carries a driver token with no audit row \
                 (the row is server-side and does not survive a restart in this build). The \
                 driver must re-take the seat with `+seat/accept {operation}/{id}`; revoke it \
                 with `@seat/revoke {operation}/{id}` if they cannot"
            ),
        );
        return Err(CmdError::msg(
            "That seat has no attribution on this server and cannot be activated.",
        ));
    }
    let channels: BTreeSet<ChannelId> = offer
        .scope
        .channels
        .iter()
        .filter_map(|c| ChannelId::new(c.as_str()))
        .collect();
    if channels.is_empty() {
        return Err(no_seat());
    }
    // **K6 gate leg (2), cut 10's OWN policy check** (08j S1): no channel in the
    // seat's set is under a live lease held by anyone but the grant's own
    // fallback. The kernel's `checked_request_handoff` never asks — requesting
    // over a live lease is its ordinary path — so this refusal is ours, and it
    // names the station and nothing else about the holder.
    let permitted = [
        mush_core::PrincipalId::Object {
            id: offer.fallback.holder.0,
        },
        mush_core::PrincipalId::Object { id: grant.actor.0 },
    ];
    for channel in &channels {
        if let Some(holder) = mush_sim::channel_lease_holder(ctx.w, offer.scope.unit.0, channel)
            && !permitted.contains(&holder)
        {
            return Err(CmdError::msg(format!(
                "`{}` on that unit is under another controller's lease.",
                offer.scope.station
            )));
        }
    }
    // **T11's area arm.** A seat declared for one area does not activate outside
    // it: the check is against the unit's own PROJECTED location — what a
    // bystander would see it in — never a record the seat could carry with it.
    if let Some(area) = offer.scope.permitted_area
        && ctx.w.get(offer.scope.unit.0).and_then(|o| o.location) != Some(area.0)
    {
        return Err(CmdError::msg(
            "That seat is not authorized where its unit currently is.",
        ));
    }
    let station = mush_sim::StationId::new(offer.scope.station.as_str()).ok_or_else(no_seat)?;
    let ticket = mush_sim::request_seat_activation(
        ctx,
        &SeatRequest::Activate {
            unit: offer.scope.unit.0,
            station,
            npc: grant.actor.0,
            granted: channels,
            fallback: Some(offer.fallback.holder.0),
        },
    )
    .map_err(render_seat_error)?;
    info(
        ctx,
        format!(
            "You take seat {id}: you now direct {} at `{}`. Your own character stays where \
             it is and cannot act. ({} channel(s) granted.)",
            mush_core::output::display_name(ctx.w, grant.actor.0),
            offer.scope.station,
            ticket.channels.len()
        ),
    );
    Ok(())
}

/// `+seat/leave` — relinquish the driven seat. **Restores AI control and does
/// not move the character.**
fn seat_leave(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(grant) = active_grant_of(ctx, ctx.actor) else {
        return Err(CmdError::msg("You are driving no seat."));
    };
    let offer = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_offer(grant.operation, grant.offer))
        .cloned()
        .ok_or_else(no_seat)?;
    stand_down(ctx, &grant, &offer)?;
    info(
        ctx,
        "You leave the seat; its declared controller takes it back.",
    );
    Ok(())
}

/// The grant whose NPC this account is currently driving, if any.
fn active_grant_of(ctx: &Ctx<'_>, who: ObjId) -> Option<SeatGrant> {
    let (unit, station) = crate::seatstate::proxy_focus_of(ctx, who)?;
    let store = ctx.w.module_state::<CampaignStore>()?;
    store
        .seat_grants()
        .find(|grant| {
            !grant.state.is_terminal()
                && store
                    .seat_offer(grant.operation, grant.offer)
                    .is_some_and(|offer| {
                        offer.scope.unit.0 == unit && offer.scope.station.as_str() == station
                    })
        })
        .cloned()
}

/// The shared stand-down: `mush-sim` clears the narrowing, bumps the epoch and
/// fires the declared fallback at a safe boundary; the grant becomes terminal
/// and its audit row is forgotten.
fn stand_down(ctx: &mut Ctx<'_>, grant: &SeatGrant, offer: &SeatOffer) -> Result<(), CmdError> {
    let station = mush_sim::StationId::new(offer.scope.station.as_str()).ok_or_else(no_seat)?;
    mush_sim::request_seat_activation(
        ctx,
        &SeatRequest::Revoke {
            unit: offer.scope.unit.0,
            station,
            npc: grant.actor.0,
        },
    )
    .map_err(render_seat_error)?;
    let mut done = grant.clone();
    done.state = GrantState::Revoked;
    crate::seatstate::audit_forget(ctx, done.driver.0.as_str());
    put_seat_grant(ctx, done);
    Ok(())
}

fn render_seat_error(error: SeatError) -> CmdError {
    match error {
        // Leg (1): a live human operator is never evicted, and the refusal names
        // the station and nothing else about the holder.
        SeatError::StationHeld => CmdError::msg(
            "That station is held by another controller; seats are never taken by force.",
        ),
        SeatError::ChannelLeased { channel } => CmdError::msg(format!(
            "`{channel}` on that unit is under another controller's lease."
        )),
        SeatError::UnmappedAccount => {
            CmdError::msg("Your account is not mapped yet; reconnect and try again.")
        }
        SeatError::OutOfReach => {
            CmdError::msg("That seat's console cannot be reached from where its crew stands.")
        }
        SeatError::CounterExhausted => CmdError::msg(
            "That unit's control generation is exhausted; staff must repair it before it can be \
             crewed.",
        ),
        // Everything else collapses to the one existence-hiding line.
        SeatError::NoSuchStation
        | SeatError::NoChannel
        | SeatError::TooManyChannels
        | SeatError::NotSeated
        | SeatError::Unavailable => CmdError::msg(NO_SUCH_SEAT),
    }
}

// ---- @seat ----------------------------------------------------------------

/// The `@seat` primary verb (staff and the offering commander).
pub(crate) fn cmd_seat_staff(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("revoke") {
        seat_revoke(ctx)
    } else if ctx.switches.has("audit") {
        seat_audit(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @seat/revoke <op>/<id> · @seat/audit <token> (staff)",
        ))
    }
}

/// `@seat/revoke <op>/<id>` — bump the epoch **immediately** so every queued
/// intent under the old grant fails stale, clear the narrowing, and return the
/// seat to its declared fallback **at a safe boundary**.
fn seat_revoke(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (operation, id) = parse_seat_ref(ctx.args.raw())?;
    let offer = visible_offer(ctx, operation, id).ok_or_else(no_seat)?;
    if !may_seat(ctx, offer.scope.unit) {
        return Err(no_seat());
    }
    let grant = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_grant(operation, id))
        .cloned()
        .ok_or_else(no_seat)?;
    if grant.state.is_terminal() {
        return Err(no_seat());
    }
    stand_down(ctx, &grant, &offer)?;
    info(ctx, format!("Seat {id} is revoked."));
    Ok(())
}

/// `@seat/audit <token>` (staff, read-only). Answers whether the token is known
/// and **prints no account id** (R7; cut 5 B7's `@identity/audit` shape).
fn seat_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(deny());
    }
    let token = ctx.args.raw().trim().to_owned();
    if token.is_empty() {
        return Err(CmdError::msg("Usage: @seat/audit <token>"));
    }
    let known = crate::seatstate::audit_knows(ctx, &token);
    info(
        ctx,
        if known {
            "That driver token is attributed to a known account."
        } else {
            "That driver token has no attribution row."
        },
    );
    Ok(())
}

// ---- +muster --------------------------------------------------------------

/// The `+muster` primary verb.
pub(crate) fn cmd_muster(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lock = ctx.switches.has("lock");
    let operation = parse_operation(ctx.args.raw())?;
    let (doc, lens) = render::visible_operation(ctx.w, operation, ctx.actor)
        .ok_or_else(|| CmdError::msg(NO_SUCH_OPERATION))?;
    if lens != Lens::Plan {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    }
    let report = crate::seatstate::take_muster(ctx, &doc, lock);
    let lines: Vec<String> = report
        .slots
        .iter()
        .map(|slot| {
            format!(
                "  {} `{}` — {}",
                mush_core::output::display_name(ctx.w, slot.unit.0),
                slot.station,
                match slot.fill {
                    SlotFill::Player => "a player character",
                    SlotFill::HumanDirectedNpc => "a human-directed NPC",
                    SlotFill::AiNpc => "an AI-controlled NPC",
                    SlotFill::Automation => "automation",
                    SlotFill::Vacant => "VACANT",
                }
            )
        })
        .collect();
    let vacancies = report.vacancies();
    if lock {
        // **Roster lock stores only the VERDICT** (Q11A/R3): the account id never
        // enters this or any other record, and an oracle answering `None` fails
        // closed — the lock is refused, the operation is untouched.
        match report.accounts_distinct {
            Some(true) => {}
            Some(false) => {
                return Err(CmdError::msg(
                    "Two of this roster's human-directed slots resolve to one account; a single \
                     account may materially assist only one side.",
                ));
            }
            None => {
                return Err(CmdError::msg(
                    "The account oracle cannot answer for this roster; the lock is refused.",
                ));
            }
        }
    }
    if !put_muster(ctx, report) {
        return Err(CmdError::msg("That muster could not be recorded."));
    }
    persist_meta(ctx);
    info(
        ctx,
        format!(
            "Muster for operation {operation}{}:\n{}\n{vacancies} vacancy(ies) — the operation \
             stands.",
            if lock { " (roster locked)" } else { "" },
            if lines.is_empty() {
                "  (no committed units)".to_owned()
            } else {
                lines.join("\n")
            }
        ),
    );
    Ok(())
}

// ---- +crewcall ------------------------------------------------------------

/// `+crewcall/list` · `+crewcall/apply <op>/<id>` — the player-facing half of the
/// seat market. `apply` is `+seat/accept` under the name the crew call uses.
pub(crate) fn cmd_crewcall(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("apply") {
        seat_accept(ctx)
    } else {
        seat_list(ctx)
    }
}

// ---- @crew ----------------------------------------------------------------

/// The `@crew` primary verb.
pub(crate) fn cmd_crew(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("assign") {
        crew_assign(ctx)
    } else if ctx.switches.has("readiness") {
        crew_readiness(ctx)
    } else if ctx.switches.has("complement") || ctx.args.raw().trim().is_empty() {
        crew_complement(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @crew/assign <unit>/<station>=<who> · @crew/complement <unit> · \
             @crew/readiness <unit>[=<0-100>]",
        ))
    }
}

fn resolve_unit(ctx: &Ctx<'_>, text: &str) -> Result<UnitRef, CmdError> {
    match_object(ctx.w, ctx.actor, text.trim())
        .map(UnitRef)
        .map_err(|_| CmdError::msg(NO_SUCH_CREW))
}

/// `@crew/assign <unit>/<station>=<who>` — record a named crew position on the
/// aggregate complement. **The named half IS cut 5's billets**; this is the
/// aggregate's own index and takes no part in authority.
fn crew_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_owned();
    let (unit_ref, station) = lhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @crew/assign <unit>/<station>=<who>"))?;
    let unit = resolve_unit(ctx, unit_ref)?;
    if !may_seat(ctx, unit) {
        return Err(CmdError::msg(NO_SUCH_CREW));
    }
    let who = resolve_unit(ctx, &rhs)?;
    let station = SmolStr::new(station.trim().to_ascii_lowercase());
    if station.is_empty() {
        return Err(CmdError::msg(NO_SUCH_CREW));
    }
    let mut crew = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.crew(unit))
        .cloned()
        .unwrap_or_else(|| CrewComplement::new(unit));
    if !crew.named.contains_key(&station) && crew.named.len() >= MAX_CREW_NAMED {
        return Err(CmdError::msg(format!(
            "That crew already names {MAX_CREW_NAMED} positions."
        )));
    }
    crew.named.insert(station.clone(), who);
    if !put_crew(ctx, crew) {
        return Err(CmdError::msg(NO_SUCH_CREW));
    }
    info(
        ctx,
        format!(
            "{} is now the named crew for `{station}`.",
            mush_core::output::display_name(ctx.w, who.0)
        ),
    );
    Ok(())
}

/// `@crew/complement <unit>` — the aggregate. Names no unit location and no
/// account.
fn crew_complement(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = resolve_unit(ctx, ctx.args.lhs())?;
    if !may_seat(ctx, unit) {
        return Err(CmdError::msg(NO_SUCH_CREW));
    }
    let crew = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.crew(unit))
        .cloned()
        .unwrap_or_else(|| CrewComplement::new(unit));
    let named: Vec<String> = crew
        .named
        .iter()
        .map(|(station, who)| {
            format!(
                "  `{station}` — {}",
                mush_core::output::display_name(ctx.w, who.0)
            )
        })
        .collect();
    info(
        ctx,
        format!(
            "Complement of {}: readiness {}, capacity {}, morale {}{}{}",
            mush_core::output::display_name(ctx.w, unit.0),
            crew.readiness,
            crew.capacity,
            crew.morale,
            if crew.obligation.is_some() {
                " (readiness obligation live)"
            } else {
                ""
            },
            if named.is_empty() {
                String::new()
            } else {
                format!("\n{}", named.join("\n"))
            }
        ),
    );
    Ok(())
}

/// `@crew/readiness <unit>[=<0-100>]` — read or set readiness, and start the
/// periodic readiness obligation from the **command** (a provider has no world
/// and cannot post — cut 4's deviation (a)).
fn crew_readiness(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = resolve_unit(ctx, ctx.args.lhs())?;
    if !may_seat(ctx, unit) {
        return Err(CmdError::msg(NO_SUCH_CREW));
    }
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_owned();
    let mut crew = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.crew(unit))
        .cloned()
        .unwrap_or_else(|| CrewComplement::new(unit));
    if !rhs.is_empty() {
        let value: u8 = rhs
            .parse()
            .ok()
            .filter(|v| *v <= 100)
            .ok_or_else(|| CmdError::msg("Readiness is 0-100."))?;
        crew.readiness = value;
        if !put_crew(ctx, crew.clone()) {
            return Err(CmdError::msg(NO_SUCH_CREW));
        }
    }
    info(
        ctx,
        format!(
            "{}: readiness {}.",
            mush_core::output::display_name(ctx.w, unit.0),
            crew.readiness
        ),
    );
    Ok(())
}

// ---- registration ---------------------------------------------------------

/// Register cut 10's five primary verbs in two classified groups. **K8.5:
/// `classify_commands` before the first `reg.command` of each group, and
/// `LegacyPostImage` restored last.**
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // Group A — the two verbs that reach K6's lease (cut 5's `@station`
    // precedent: an ingress onto a migrated authoritative action publishes
    // `Authoritative`, whoever stages it).
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "+seat".into(),
            switches: vec![
                "offer".into(),
                "list".into(),
                "accept".into(),
                "decline".into(),
                "activate".into(),
                "leave".into(),
            ],
            shape: ArgShape::EqPair,
            help: "+seat/offer <op>/<unit>/<station>=<npc>[,<channel>…] · +seat/list · \
                   +seat/accept <op>/<id> · +seat/decline <op>/<id> · +seat/activate <op>/<id> · \
                   +seat/leave — proxy seats. Accepting seizes nothing; activating consumes your \
                   one foreground focus and leaves your own character present, vulnerable and \
                   unable to act."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_seat),
    )?;
    reg.command(
        CommandSpec {
            name: "@seat".into(),
            switches: vec!["revoke".into(), "audit".into()],
            shape: ArgShape::EqPair,
            help: "@seat/revoke <op>/<id> · @seat/audit <token> (staff) — end a proxy seat at a \
                   safe boundary, or check that a driver token is attributed. The audit verb \
                   prints no account."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_seat_staff),
    )?;
    // Group B — the exempt workflow documents. Restored last, so the module ends
    // in the class its own records ride.
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "+crewcall".into(),
            switches: vec!["list".into(), "apply".into()],
            shape: ArgShape::EqPair,
            help: "+crewcall/list · +crewcall/apply <op>/<id> — the open seats and how to take \
                   one."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_crewcall),
    )?;
    reg.command(
        CommandSpec {
            name: "+muster".into(),
            switches: vec!["lock".into()],
            shape: ArgShape::EqPair,
            help: "+muster <op> · +muster/lock <op> — who is actually crewing an operation. A \
                   vacancy reduces discretion; it never cancels a valid operation."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_muster),
    )?;
    reg.command(
        CommandSpec {
            name: "@crew".into(),
            switches: vec!["assign".into(), "complement".into(), "readiness".into()],
            shape: ArgShape::EqPair,
            help: "@crew/assign <unit>/<station>=<who> · @crew/complement <unit> · \
                   @crew/readiness <unit>[=<0-100>] — the aggregate complement beside the \
                   billets. The named half IS the billets; this is readiness, capacity and \
                   morale."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_crew),
    )?;
    let _ = MAX_MUSTER_SLOTS;
    let _: Option<MusterReport> = None;
    let _: Option<MusterSlot> = None;
    Ok(())
}
