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

use std::collections::{BTreeMap, BTreeSet};
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

/// **The one line a character the account oracle does not know is refused
/// with** (16b cut F.4).
///
/// It used to say "your account is not mapped yet; reconnect and try again",
/// from the days when the map was built once at boot and a character created
/// after it was unmapped until the next restart. The map learns of a character
/// in the world-task turn that creates it now, so reaching this means the
/// character genuinely belongs to no account — a seeded `#1`, or an account
/// document that does not name it — and reconnecting re-reads nothing. The line
/// says what is true and promises no repair that exists.
pub const NO_ACCOUNT_ON_RECORD: &str =
    "This character has no account on record, so it cannot take a seat.";

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

/// **Who may TAKE and DRIVE one seat** (E7 cut 10 independent verify fold, A1 /
/// decision E1(a)).
///
/// Cut 10 had no such predicate at all. [`visible_offer`] asked
/// `render::visible_operation(..) != Lens::Hidden` and nothing else, and
/// `render::lens_for` hands `Lens::Notice` to **every actor in the world** for a
/// `Public` operation — so a stranger with no side, no audience membership, no
/// unit and no capability listed the commander's seat, accepted it and activated
/// it, consuming the account-scoped proxy focus on the commander's own NPC and
/// narrowing the commander's own station. `SeatOffer.audience` and
/// `SeatOffer.eligibility` were read only by the cap check and by
/// `seat_consent_hash`: a hashed field with a writer and no reader (cut 9's fold
/// B13), and the writer was **wrong as well** — `+seat/offer` wrote
/// `doc.terms.eligible_defenders`, i.e. the ENEMY list.
///
/// `render::lens_for` stays the ONE projection predicate; this is a **command**
/// predicate beside it (cut 9's fold E1(a) shape), and it reuses the very
/// `acts_for` the lens uses, so no second disclosure rule appears. It is asked by
/// `+seat/accept`, by `+crewcall/apply`, by `+seat/list` and **again** by
/// `+seat/activate`, so a grant taken before an audience narrowing cannot be
/// driven; every refusal is the byte-identical [`NO_SUCH_SEAT`].
fn may_take_seat(ctx: &Ctx<'_>, offer: &SeatOffer) -> bool {
    offer
        .audience
        .members
        .iter()
        .any(|p| render::acts_for(ctx.w, ctx.actor, p))
        || may_seat(ctx, offer.scope.unit)
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
            "Usage: +seat/offer <op>/<unit>/<station>=<npc>[,<channel>…][,to:<who>][,area:<where>] · \
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
            "Usage: +seat/offer <op>/<unit>/<station>=<npc>[,<channel>…][,to:<who>][,area:<where>]",
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
    // A1: the explicit narrower call. `to:<who>` names one principal the seat is
    // offered to, beside the operation's own attacking side.
    let mut to_refs: Vec<String> = Vec::new();
    let mut channels: BTreeSet<SmolStr> = BTreeSet::new();
    for field in fields.filter(|c| !c.is_empty()) {
        if let Some(area) = field.strip_prefix("area:") {
            area_ref = Some(area.trim().to_owned());
        } else if let Some(who) = field.strip_prefix("to:") {
            to_refs.push(who.trim().to_owned());
        } else {
            channels.insert(SmolStr::new(field.to_ascii_lowercase()));
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
    // **A15: a dead seat is never published.** `SeatScope::check` bounds lengths
    // and counts and nothing else, so cut 10 published, consent-hashed and let a
    // driver accept a seat naming a station the unit does not carry or channels
    // it does not publish — discovered only at `+seat/activate`, where the
    // refusal collapses to the existence-hiding line, so the driver could not
    // tell a mis-typed offer from a revoked one. The same clamp
    // `request_seat_activation` applies is run HERE, at publish time, so the
    // consent surface never freezes an unreachable scope.
    let station_channels: BTreeSet<SmolStr> = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.stations.get(&unit.0))
        .and_then(|host| mush_sim::StationId::new(&station).and_then(|id| host.stations.get(&id)))
        .map(|spec| {
            spec.channels
                .iter()
                .map(|c| SmolStr::new(c.as_str()))
                .collect()
        })
        .ok_or_else(no_seat)?;
    if channels.intersection(&station_channels).next().is_none() {
        return Err(no_seat());
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
    // **A1's corrected writer.** Cut 10 wrote `doc.terms.eligible_defenders`
    // under the comment "visible to exactly the operation's eligible
    // defenders/audience" — the field's declared value was the ENEMY list, and
    // because it was inert the wrongness was invisible. A seat is offered by the
    // operation's own side: its attacker principal, plus whoever `to:<who>`
    // names.
    let mut members: Vec<mush_org::PrincipalId> = vec![doc.terms.attacker.clone()];
    for who in &to_refs {
        let id = match_object(ctx.w, ctx.actor, who).map_err(|_| no_seat())?;
        let principal = mush_org::PrincipalId::Object(id);
        if !members.contains(&principal) {
            members.push(principal);
        }
    }
    if members.len() > mush_economy::MAX_AUDIENCE {
        return Err(CmdError::msg(format!(
            "A seat is offered to at most {} principals.",
            mush_economy::MAX_AUDIENCE
        )));
    }
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
        // The seat's own audience: the operation's attacking side and whoever
        // `to:<who>` names, and NOBODY else. `may_take_seat` is its reader.
        audience: crate::record::AudienceSpec { members },
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
    // **K8: filter BEFORE bounding.** Cut 10 took a global 256-row prefix in key
    // order and only then asked the lens, so invisible (covert) offers consumed a
    // visible viewer's budget and, past 256 offers in the world, a viewer's own
    // operation's offers fell outside the prefix entirely and `+seat/list`
    // answered "No seats are offered to you." for a seat they held. The lens and
    // the audience are asked per row, memoized per operation so the scan stays
    // bounded on records **visited**, and the budget then bounds what is shown.
    let offers: Vec<SeatOffer> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            info(ctx, "No seats are offered.");
            return Ok(());
        };
        store.seat_offers().cloned().collect()
    };
    let mut lens_of: BTreeMap<u64, bool> = BTreeMap::new();
    let mut shown = 0usize;
    for offer in &offers {
        if shown >= MAX_SEAT_OFFERS_PER_OPERATION * 8 {
            break;
        }
        // The lens is asked per operation, so a covert operation's seat is
        // invisible rather than merely unnamed.
        let visible = *lens_of.entry(offer.operation.0).or_insert_with(|| {
            render::visible_operation(ctx.w, offer.operation, ctx.actor).is_some()
        });
        if !visible || !may_take_seat(ctx, offer) {
            continue;
        }
        shown += 1;
        // `taken` counts only a LIVE grant: cut 10 asked `is_some()`, so a
        // revoked or expired seat read as taken for ever and a revocation was
        // invisible to the crew call it frees.
        let taken = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.seat_grant(offer.operation, offer.id))
            .is_some_and(|grant| !grant.state.is_terminal());
        lines.push(format!(
            "  {} — operation {}, station `{}`{}",
            offer.id,
            offer.operation,
            offer.scope.station,
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
    // **A1.** The audience is a predicate now, and the refusal is byte-identical
    // to an id that never existed.
    if !may_take_seat(ctx, &offer) {
        return Err(no_seat());
    }
    let now = ctx.w.world_ic_now();
    if offer.is_expired(&now) {
        return Err(no_seat());
    }
    if !offer.consent_intact() {
        return Err(CmdError::msg(
            "That seat's terms no longer match what was offered; it cannot be accepted.",
        ));
    }
    // **A9(2): a locked roster is a gate now.** `MusterReport.locked` had no
    // reader anywhere, so C23A/C24A's "an account may materially assist only one
    // side" was enforced nowhere and nothing re-checked after `+muster/lock`.
    if roster_locked(ctx, operation) {
        return Err(CmdError::msg(
            "That operation's roster is locked; no further seat can be taken on it.",
        ));
    }
    // **B7: the audit-loss runbook's own instruction now works.** A `Dormant`
    // grant whose token has no audit row — the state EVERY restart produces,
    // since the log is process-local by design — is re-accepted: the token is
    // re-minted and the row re-recorded over it. Cut 10 refused whenever a grant
    // existed at all, whatever its state, so the page's first instruction
    // (`+seat/accept {op}/{id}`) could never succeed, and it failed with the
    // existence-hiding line so the driver could not tell why.
    let existing = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_grant(operation, id))
        .cloned();
    match &existing {
        None => {}
        Some(grant)
            if matches!(grant.state, GrantState::Dormant)
                && !crate::seatstate::audit_knows(ctx, grant.driver.0.as_str()) => {}
        Some(_) => return Err(no_seat()),
    }
    // The account whose ONE foreground focus this seat will consume. `None` =
    // unknown = refuse, never a default (fail closed).
    let account = crate::seatstate::account_of(ctx, ctx.actor)
        .ok_or_else(|| CmdError::msg(NO_ACCOUNT_ON_RECORD))?;
    let actor = offer.fallback.holder;
    // K5: the SEED is the seat's own coordinates; the token itself is minted
    // server-side with a process nonce folded in, so it cannot be inverted from
    // four public coordinates and cannot be forged for a seat one does not hold.
    // A hash failure REFUSES rather than falling back to the coordinates in
    // plaintext.
    let seed = DriverToken::seed(operation, id, actor, &now).ok_or_else(|| {
        CmdError::msg("That seat's driver token could not be minted; the seat was not taken.")
    })?;
    // R3/R7: the row lives OUTSIDE the world. If it cannot be stored, the grant
    // is not created — a seat is never taken un-attributed.
    let Some(token) = crate::seatstate::audit_record(ctx, seed.as_str(), account) else {
        return Err(CmdError::msg(
            "Seat attribution is unavailable on this server; the seat was not taken.",
        ));
    };
    let driver = DriverToken(token);
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

/// Whether the operation's latest muster locked its roster (fold A9(2):
/// `MusterReport.locked` had no reader anywhere in production).
fn roster_locked(ctx: &Ctx<'_>, operation: OperationId) -> bool {
    ctx.w
        .module_state::<CampaignStore>()
        .and_then(|store| store.latest_muster(operation))
        .is_some_and(|report| report.locked)
}

/// `+seat/activate <op>/<id>` — the K6 gate, then `mush-sim`.
fn seat_activate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (operation, id) = parse_seat_ref(ctx.args.raw())?;
    let offer = visible_offer(ctx, operation, id).ok_or_else(no_seat)?;
    // **A1, asked a SECOND time**: a grant taken before an audience narrowing
    // cannot be driven after it.
    if !may_take_seat(ctx, &offer) {
        return Err(no_seat());
    }
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
        // **B11 gives `epoch` its reader.** §3.10 B7 justified the payload as
        // "the exact value `World::channel_grant_is_current` compares, so a
        // revocation's bump makes it stale by construction" — and no comparison
        // existed anywhere. It exists here: a grant recorded live under an epoch
        // the unit has since bumped past is STALE, not drivable.
        GrantState::Active { epoch } => {
            let current = ctx
                .w
                .module_state::<mush_sim::SpatialState>()
                .map_or(0, |state| state.control_generation(offer.scope.unit.0));
            return if current == epoch {
                Err(CmdError::msg("You are already driving that seat."))
            } else {
                Err(no_seat())
            };
        }
    }
    // **B14: self-idempotent on WORLD state, not on the grant document.**
    // `Active` is written only by the 15 s campaign tick, so for up to one whole
    // period a second `+seat/activate` passed every check and re-ran the focus
    // bump — and `cgen` is per UNIT, so each re-run staled every OTHER crew
    // member's captured intents as well as the driver's own.
    if crate::seatstate::proxy_focus_of(ctx, ctx.actor).is_some_and(|(unit, station)| {
        unit == offer.scope.unit.0 && station == offer.scope.station.as_str()
    }) {
        info(ctx, "You are already driving that seat.");
        return Ok(());
    }
    // **A12: "frozen at acceptance" is enforced.** `seat_accept` copies the
    // offer's hash into the grant and cut 10 then checked only
    // `offer.consent_intact()` — the offer's self-consistency, which any re-hash
    // satisfies. The point of the frozen hash is to catch a future edit verb, a
    // held-document repair or a hand-edited `seat_offer:` key.
    if grant.consent_hash != offer.consent_hash {
        return Err(no_seat());
    }
    let now = ctx.w.world_ic_now();
    if offer.is_expired(&now) || !offer.consent_intact() {
        return Err(no_seat());
    }
    if roster_locked(ctx, operation) {
        return Err(CmdError::msg(
            "That operation's roster is locked; no further seat can be taken on it.",
        ));
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
    // **A2: attribution is not a boolean.** Cut 10's only check was
    // `audit_knows(token)` — "does ANY row exist" — so account B drove a grant
    // whose one `token -> account` row named account A, and R7/T9's dual
    // actor/account attribution resolved to somebody who was not driving. The
    // row must name the DRIVING account, and a mismatch refuses byte-identically
    // to a seat that never existed.
    let account = crate::seatstate::account_of(ctx, ctx.actor)
        .ok_or_else(|| CmdError::msg(NO_ACCOUNT_ON_RECORD))?;
    if !crate::seatstate::audit_is(ctx, grant.driver.0.as_str(), account) {
        return Err(no_seat());
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
    //
    // **E5/B5: an unknown answer is not a vacancy.** The mirror the seam reads is
    // refreshed only from `mush-sim`'s own Coarse fire and the `Disconnected`
    // hook, so between boot and the first fire it is empty AND self-declared
    // incomplete — while `control_leases` is durable K state that survived the
    // reboot. Cut 10 never consulted `is_complete()` (which had zero production
    // callers tree-wide), so leg (2) read ignorance as "nobody holds this".
    // (A world-state READ of a published type, exactly as `FocusState` and
    // `SpatialState` are read — not a third seam: `e7c10_p5` pins that
    // `mush-campaign` CALLS only `request_seat_activation` and
    // `channel_lease_holder`.)
    if !ctx
        .w
        .module_state::<mush_sim::ControlLeaseView>()
        .is_some_and(mush_sim::ControlLeaseView::is_complete)
    {
        return Err(CmdError::msg(
            "That unit's control state is still being read; try again in a moment.",
        ));
    }
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
    //
    // **K7: it is not an oracle.** Cut 10 answered with a DISTINCT refusal
    // ("That seat is not authorized where its unit currently is."), and a
    // grant-holder may retry freely with nothing rate-limiting them — so polling
    // turned the refusal into a boolean "is the unit at `<area>` yet" that
    // flipped at the instant of arrival: a fogged-position bit about a unit the
    // driver may not otherwise perceive. The refusal is now byte-identical to
    // [`NO_SUCH_SEAT`], so a poller learns the same thing from an area mismatch,
    // a revoked seat and an id that never existed: nothing.
    if let Some(area) = offer.scope.permitted_area
        && ctx.w.get(offer.scope.unit.0).and_then(|o| o.location) != Some(area.0)
    {
        return Err(no_seat());
    }
    let station = mush_sim::StationId::new(offer.scope.station.as_str()).ok_or_else(no_seat)?;
    // ---- E7c11 independent verify fold, D5 (T11's "assumption of another
    // station", on the activation path) ----
    // The B14 short-circuit above fires only for the SAME `(unit, station)`; a
    // proxy focus on a **different** seat fell straight through, and
    // `FocusState::claim` displaced the incumbent. The mechanical half was sound
    // — `seat::activate` clears the displaced seat's narrowing — but the
    // campaign half was never told: grant 1 stayed non-terminal with a live
    // `SeatAudit` row, `active_grant_of` resolved only grant 2, and on a
    // single-channel unit the tick promoted grant 1 to `Active { epoch }` with
    // nobody driving it, after which the epoch comparison answered `no_seat()`
    // for ever and staff `@seat/revoke` was the only exit.
    //
    // The incumbent is stood down through the ONE seam (`stand_down` ->
    // `request_seat_activation(Revoke)`), which revokes grant 1, forgets its
    // audit row and clears its narrowing, before the new activation. It is the
    // last fallible step before the activation itself, so a refusal above leaves
    // the incumbent untouched; the two submissions it can stage are keyed on two
    // different NPCs, so cut 2's one-submit-per-actor rule (C4) still holds.
    // If the stand-down itself refuses, grant 2 is left untaken and grant 1 is
    // NAMED — a seat is never displaced silently.
    if let Some(previous) = active_grant_of(ctx, ctx.actor)
        && !(previous.operation == operation && previous.offer == id)
    {
        let previous_offer = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.seat_offer(previous.operation, previous.offer))
            .cloned()
            .ok_or_else(no_seat)?;
        stand_down(ctx, &previous, &previous_offer).map_err(|_| {
            CmdError::msg(format!(
                "You are already driving seat {}/{}; +seat/leave it before taking another.",
                previous.operation.0, previous.offer.0
            ))
        })?;
    }
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
///
/// **A5.** Cut 10 resolved the caller's proxy focus to `(unit, station)` and then
/// took the FIRST non-terminal grant on that station in `(operation, offer)` key
/// order, never comparing the driver token to the caller — so `+seat/leave`
/// revoked another player's grant and forgot *their* audit row, while the
/// leaver's own grant stayed re-activatable. The grant is resolved through the
/// audit seam now: it must be the caller's OWN.
fn active_grant_of(ctx: &Ctx<'_>, who: ObjId) -> Option<SeatGrant> {
    let (unit, station) = crate::seatstate::proxy_focus_of(ctx, who)?;
    let account = crate::seatstate::account_of(ctx, who)?;
    let candidates: Vec<SeatGrant> = {
        let store = ctx.w.module_state::<CampaignStore>()?;
        store
            .seat_grants()
            .filter(|grant| {
                !grant.state.is_terminal()
                    && store
                        .seat_offer(grant.operation, grant.offer)
                        .is_some_and(|offer| {
                            offer.scope.unit.0 == unit && offer.scope.station.as_str() == station
                        })
            })
            .cloned()
            .collect()
    };
    candidates
        .into_iter()
        .find(|grant| crate::seatstate::audit_is(ctx, grant.driver.0.as_str(), account))
}

/// The shared stand-down: `mush-sim` clears the narrowing, bumps the epoch and
/// fires the declared fallback at a safe boundary; the grant becomes terminal
/// and its audit row is forgotten.
fn stand_down(ctx: &mut Ctx<'_>, grant: &SeatGrant, offer: &SeatOffer) -> Result<(), CmdError> {
    // **16b cut T.3, admission site 5 of 6** (doc 16 §3.1 "Custody"). A seat on a
    // unit an arena holds is a *cockpit*: the arena authorises every order
    // against a `(unit, channel)` epoch the world publishes, and a third party
    // revoking that seat reaches no arena at all — there is no hook for it, and
    // an arena that never hears about it keeps taking the evicted pilot's queued
    // orders. So the eviction is refused at the door instead, which is the one
    // defence the seam's shape allows.
    mush_sim::deployed::refuse_if_custodied(ctx.w, offer.scope.unit.0)
        .map_err(|error| CmdError::msg(error.to_string()))?;
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
        SeatError::UnmappedAccount => CmdError::msg(NO_ACCOUNT_ON_RECORD),
        SeatError::OutOfReach => {
            CmdError::msg("That seat's console cannot be reached from where its crew stands.")
        }
        SeatError::CounterExhausted => CmdError::msg(
            "That unit's control generation is exhausted; staff must repair it before it can be \
             crewed.",
        ),
        // B5/E5(a): distinct, but neutral — it names no holder and no station.
        SeatError::ControlUnknown => {
            CmdError::msg("That unit's control state is still being read; try again in a moment.")
        }
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
            "Usage: @seat/revoke <op>/<id> · @seat/audit <op>/<id>[=<character>] (staff)",
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

/// `@seat/audit <op>/<id>[=<character>]` (staff, read-only) — cut 5's
/// `@identity/audit` shape, and **no token is ever typed by hand or printed**.
///
/// **K6.** Cut 10's verb took a raw token and answered `audit_knows`, a boolean
/// — while no render, page, log or journal anywhere printed a token, so a
/// staffer investigating a proxy seat had neither an input to the verb nor an
/// answer from it, and `SeatAudit::account_for` (documented "staff only, and
/// never rendered ... exactly one staff verb resolves it") had **no production
/// caller at all**. The verb now names the seat by its own public coordinates
/// and answers SAME / DIFFERENT / UNKNOWN about a character, through
/// `account_for`, printing no id on any arm.
fn seat_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(deny());
    }
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_owned();
    if lhs.is_empty() {
        return Err(CmdError::msg(
            "Usage: @seat/audit <op>/<id>[=<character>] (staff)",
        ));
    }
    let (operation, id) = parse_seat_ref(&lhs)?;
    let grant = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.seat_grant(operation, id))
        .cloned()
        .ok_or_else(no_seat)?;
    if rhs.is_empty() {
        let known = crate::seatstate::audit_knows(ctx, grant.driver.0.as_str());
        info(
            ctx,
            if known {
                "That seat is attributed to a known account."
            } else {
                "That seat has no attribution row."
            },
        );
        return Ok(());
    }
    let who = match_object(ctx.w, ctx.actor, &rhs).map_err(|_| no_seat())?;
    let verdict = crate::seatstate::audit_verdict(ctx, grant.driver.0.as_str(), who);
    let name = mush_core::output::display_name(ctx.w, who);
    info(
        ctx,
        match verdict {
            crate::seatstate::AuditVerdict::Same => {
                format!("SAME: that seat is driven from {name}'s account.")
            }
            crate::seatstate::AuditVerdict::Different => {
                format!("DIFFERENT: that seat is not driven from {name}'s account.")
            }
            crate::seatstate::AuditVerdict::Unknown => {
                "UNKNOWN: that seat has no attribution row on this server.".to_owned()
            }
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
    // **K2 / decision E2(a): the units this caller may actually command.**
    // `lens_for` grants `Lens::Plan` to staff, to the attacker, to ANY named side
    // principal — i.e. the declared defender — and to any committed unit's
    // controller, because those are the parties entitled to *see* the plan. Cut
    // 10 used that lens as the entire gate on a render that walks BOTH sides'
    // participants, calls `stations_of`/`seat_holder` on each and classifies each
    // slot from `ActiveFocus.proxy` — so `+muster` handed a named defender the
    // attacker's station-by-station manning (which ships are on a skeleton crew,
    // which stations are empty) at the same lens where `render::render`
    // deliberately publishes only `Participants: N of M` and names no unit.
    let commandable: BTreeSet<u32> = doc
        .runtime
        .participants
        .iter()
        .filter(|p| may_seat(ctx, p.unit))
        .map(|p| p.unit.0.0)
        .collect();
    // **The write gate** (cut 9's fold C1, one cut on): `+muster/lock` mutates —
    // it freezes a roster and stores a distinctness verdict — so it asks a
    // command predicate, not the disclosure lens.
    if lock && commandable.is_empty() && !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH_OPERATION));
    }
    let report = crate::seatstate::take_muster(ctx, &doc, lock);
    // The RECORD is the whole roster (only `latest_muster` reads it, and it names
    // no account); the RENDER is only what this caller commands, so a slot the
    // caller may not command never carries the `HumanDirectedNpc` bit — the
    // proxy flag being the skeleton-crew tell.
    let lines: Vec<String> = report
        .slots
        .iter()
        .filter(|slot| commandable.contains(&slot.unit.0.0))
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
            help: "+seat/offer <op>/<unit>/<station>=<npc>[,<channel>…][,to:<who>] · +seat/list · \
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
            help: "@seat/revoke <op>/<id> · @seat/audit <op>/<id>[=<character>] (staff) — end a \
                   proxy seat at a safe boundary, or ask whether a seat is driven from a \
                   character's own account. The audit verb prints no account and no token."
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
