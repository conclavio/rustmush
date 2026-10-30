//! E7 cut 5 B5/B6 — the `@station` control-lease ingress and the declared
//! fallback controller (10b §3.5).
//!
//! **B5 — the K6 station-control policy ingress.** `@station/handoff` and
//! `/cancel` ride cut 2's [`Ctx::submit_authoritative`](mush_core::Ctx::submit_authoritative)
//! seam onto the migrated `control_handoff_request`/`control_handoff_cancel`
//! actions (`crate::handoff`), whose `project_offer` declares
//! **`required_channels = []`** and pins the requester's control-generation as a
//! FIXED parameter (candidate (iii); §3.5 as-built). Cut 5 is the **first
//! production `ControlLeaseState` creator**: a focused billet-holder establishes
//! their vacant station's lease. The authority split lives in the action's guard
//! (`crate::handoff::HandoffProvider::project_station_offer`), and the epoch
//! binding is re-checked at apply by `HandoffScheduleAdapter::plan` — so a focus
//! change between request and apply invalidates the in-flight request. The lease
//! state machine is K6's `ControlLeaseState::checked_*`; this module never
//! re-implements it. `/show` is read-only.
//!
//! **B6 — `@station/fallback` and the immediate deterministic controller.** The
//! command declares one bounded whole-document `sim.station_fallback` record (a
//! legacy-post-image exempt write) and mirrors it into a RAM slot the controller
//! reads. On a clean focus switch the released station's declared fallback fires
//! **immediately** on the H-series K7.0 template (`crate::handoff` request via
//! `submit_authoritative`, a deterministic key, **no `PendingDecision`**). Cut 5
//! registers no decision source and no LLM path, so the "a fallback competing
//! with an existing LLM decision reuses that decision's record/idempotency key"
//! clause is a `#[ignore]` K7 pin (§4.0c C).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    ChannelId, CmdError, Ctx, Flags, IdempotencyKeyText, ModuleOp, ModuleWrite, ObjId,
    PerceivedRef, match_object,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use crate::NS;
use crate::commands::{focused_station, holds_billet, info, is_admin, rhs_required, st};
use crate::data::{
    HostStations, MAX_GRANTED_CHANNELS, MAX_SEATS_PER_UNIT, SpatialState, StationId, StationSpec,
};

// ---------------------------------------------------------------------------
// 16b cut K.2: the station-topology seam
// ---------------------------------------------------------------------------

/// Why [`declare_station`] refused. Rendered by the caller, so a refusal a
/// viewer may not distinguish stays byte-identical there.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StationTopologyError {
    /// No such live host.
    #[error("no such unit")]
    NoSuchUnit,
    /// The set is empty, or over [`MAX_GRANTED_CHANNELS`].
    #[error("a station grants 1 to {MAX_GRANTED_CHANNELS} channels")]
    Channels,
    /// The host already carries [`MAX_SEATS_PER_UNIT`] stations.
    #[error("that unit already carries as many stations as it may")]
    Full,
    /// The role label is empty.
    #[error("a station needs a role")]
    NoRole,
    /// The host's control-generation counter is exhausted (E5a S2: arithmetic
    /// overflow **pages**, never stalls).
    #[error("control generation exhausted")]
    CounterExhausted,
    /// The simulation is not composed.
    #[error("the simulation is not available")]
    Unavailable,
}

/// **Declare one control station on a unit** (16b cut K.2, K2; doc 16 §3.3
/// "Stations are on the tank").
///
/// One of the three mechanical seams 16b cut K.2 added to this crate
/// ([`vacate_station`] and [`crate::place_interior`] are the others; the seat
/// seam's own "exactly two `pub fn`s" fence is `seat.rs`'s and is untouched),
/// and the *why* is doc 16's own sentence: a
/// simulator pod's station record lives on the **tank**, so the `helm`,
/// `sensors` and `fire` grants have the tank as their subject and the pod is
/// only where the body sits. The game crate owns that pairing — which pod flies
/// which tank is the centre's content, not the simulation's — and everything
/// mechanical about the station is this crate's: the `stations:<host>` document,
/// the control-generation bump that makes a redefinition visible to every queued
/// intent, and the crew grant re-derived from it. `@spatial/station` is the same
/// write behind a staff verb with an argument parser; this is that write without
/// the parser, so no caller outside this crate has to spell a `HostStations`
/// record or find the persistence.
///
/// **Idempotent**: a station that already carries exactly this role, channel set,
/// priority and console space is left alone and answers `Ok(false)` — no
/// document, no epoch bump, so a pilot boarding the same pod twice does not
/// invalidate their own grant.
///
/// Every fallible check runs before the first mutation.
///
/// # Errors
///
/// [`StationTopologyError`], naming the subject and nothing about who may see
/// it.
pub fn declare_station(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    role: &str,
    channels: &BTreeSet<ChannelId>,
    space: Option<ObjId>,
    priority: u16,
) -> Result<bool, StationTopologyError> {
    if role.trim().is_empty() {
        return Err(StationTopologyError::NoRole);
    }
    if channels.is_empty() || channels.len() > MAX_GRANTED_CHANNELS {
        return Err(StationTopologyError::Channels);
    }
    if ctx
        .w
        .get(unit)
        .is_none_or(|object| object.flags.contains(Flags::GOING))
    {
        return Err(StationTopologyError::NoSuchUnit);
    }
    let policy = crate::data::passive_rules_value(serde_json::json!({}))
        .map_err(|_| StationTopologyError::Unavailable)?;
    let spec = StationSpec {
        role: SmolStr::new(role),
        channels: channels.clone(),
        priority,
        space,
        policy,
    };
    {
        let held = st(ctx).stations.get(&unit);
        if held.and_then(|hs| hs.stations.get(station)) == Some(&spec) {
            return Ok(false);
        }
        if held.is_some_and(|hs| {
            !hs.stations.contains_key(station) && hs.stations.len() >= MAX_SEATS_PER_UNIT
        }) {
            return Err(StationTopologyError::Full);
        }
    }
    let generation = st(ctx)
        .next_control_generation(unit)
        .map_err(|_| StationTopologyError::CounterExhausted)?;
    // ---- apply: no fallible work past here ----
    let record = {
        let Some(state) = ctx.w.module_state_mut::<SpatialState>() else {
            return Err(StationTopologyError::Unavailable);
        };
        let host = state.stations.entry(unit).or_insert_with(|| HostStations {
            host: unit,
            stations: BTreeMap::new(),
        });
        host.stations.insert(station.clone(), spec);
        let record = host.clone();
        state.apply_control_generation(unit, generation);
        record
    };
    crate::data::persist_stations(ctx.out, &record);
    crate::data::persist_control_gen(ctx.out, unit, generation);
    // A redefined station changes what its holder may do, so whoever crews it
    // gets their grant re-derived — the same republish `@spatial/station` does.
    crate::links::refresh_unit_actors(ctx.w, unit);
    Ok(true)
}

/// **Give one station's billet back** (16b cut K.2, K2; doc 16 §3.8: *seats,
/// leases, focus and pods released atomically*).
///
/// `request_seat_activation(Release | Revoke)` stands a **driver** down and
/// deliberately leaves the billet where it is: in cut 10's story the seat is an
/// NPC's own station and standing down is what returns it to AI control. A
/// simulator cockpit is the other story — the pod is lent for one match, the
/// billet is the loan, and a billet left behind is the next pilot's
/// [`crate::SeatError::StationHeld`]. So this is the give-back, and it is
/// `@spatial/unassign`'s exact write without the staff verb around it: the seat
/// removed, the `assignment:<unit>` document rewritten, the control generation
/// bumped (so every intent queued under the old grant fails
/// `World::channel_grant_is_current`) and the former holder's links re-derived.
///
/// Answers whether anything was held. Idempotent, so a second `EXIT` or a
/// check-in after one costs nothing.
///
/// # Errors
///
/// [`StationTopologyError::CounterExhausted`] or
/// [`StationTopologyError::Unavailable`]; a station nobody holds is `Ok(false)`,
/// not an error.
pub fn vacate_station(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    actor: ObjId,
) -> Result<bool, StationTopologyError> {
    let held = st(ctx)
        .assignments
        .get(&unit)
        .and_then(|host| host.seats.get(station))
        .is_some_and(|seat| seat.actor == actor);
    if !held {
        return Ok(false);
    }
    let generation = st(ctx)
        .next_control_generation(unit)
        .map_err(|_| StationTopologyError::CounterExhausted)?;
    // ---- apply: no fallible work past here ----
    let record = {
        let Some(state) = ctx.w.module_state_mut::<SpatialState>() else {
            return Err(StationTopologyError::Unavailable);
        };
        let Some(host) = state.assignments.get_mut(&unit) else {
            return Err(StationTopologyError::Unavailable);
        };
        host.seats.remove(station);
        let record = host.clone();
        if record.seats.is_empty() {
            state.assignments.remove(&unit);
        }
        state.derived = crate::graph::rebuild(state);
        state.apply_control_generation(unit, generation);
        record
    };
    crate::data::persist_assignments(ctx.out, &record);
    crate::data::persist_control_gen(ctx.out, unit, generation);
    crate::links::refresh_actor_links(ctx.w, actor);
    Ok(true)
}

/// Byte-identical for an unknown/unviewable unit AND for a station the actor
/// does not hold a billet on (B1): a refusal must reveal nothing about which
/// units or stations exist to an actor with no authority over them.
/// `resolve_visible_unit` collapses the unit level; `station_handoff`'s
/// `holds_billet` gate collapses the station level (an existing-but-not-held
/// station and a non-existent one both refuse with this line to a non-holder).
const NO_SUCH_STATION: &str = "No such station.";

/// One declared station fallback — a bounded whole-document workflow record
/// (family `sim.station_fallback`, a legacy-post-image exempt write). The
/// declared `holder` is the controller the released station hands to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StationFallback {
    /// The controlled unit.
    pub unit: ObjId,
    /// The unit's station.
    pub station: StationId,
    /// The declared fallback controller the released station hands control to.
    pub holder: ObjId,
}

/// Census bound on the declared-fallback family (B2, folded into D2): a K-style
/// count cap on the RAM mirror and, by construction, the durable
/// `sim.station_fallback` doc set the boot hydration reads back. The count is
/// otherwise only naturally bounded by station topology and the `is_admin` gate;
/// this makes the ceiling explicit so a declared-policy family cannot grow
/// unbounded. Station-name length is bounded by the existing
/// station-must-exist-on-the-unit gate on the declare path.
pub const MAX_STATION_FALLBACKS: usize = 4096;

/// The world-owned RAM mirror of the declared fallbacks (keyed by unit+station),
/// the data the immediate controller reads on a clean switch. Persisted as
/// `sim.station_fallback` docs and re-hydrated at boot from a `station_fallback:`
/// branch in `load_boot` (D2), bounded by [`MAX_STATION_FALLBACKS`].
#[derive(Clone, Debug, Default)]
pub struct StationFallbacks(BTreeMap<(ObjId, StationId), ObjId>);

impl StationFallbacks {
    /// The declared fallback controller for one station, if any.
    #[must_use]
    pub fn holder(&self, unit: ObjId, station: &StationId) -> Option<ObjId> {
        self.0.get(&(unit, station.clone())).copied()
    }

    /// Declare (or replace) one station's fallback controller. Returns `false`
    /// without inserting when a NEW key would exceed [`MAX_STATION_FALLBACKS`]
    /// (re-declaring an existing station never grows the map, so it always
    /// succeeds).
    #[must_use]
    pub fn declare(&mut self, unit: ObjId, station: StationId, holder: ObjId) -> bool {
        let key = (unit, station);
        if !self.0.contains_key(&key) && self.0.len() >= MAX_STATION_FALLBACKS {
            return false;
        }
        self.0.insert(key, holder);
        true
    }

    /// How many fallbacks are declared.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The `sim.station_fallback` document key.
pub(crate) fn fallback_key(unit: ObjId, station: &StationId) -> SmolStr {
    SmolStr::new(format!("station_fallback:{}:{}", unit.0, station.as_str()))
}

fn fallbacks<'a>(ctx: &'a Ctx<'_>) -> Option<&'a StationFallbacks> {
    ctx.w.module_state::<StationFallbacks>()
}

/// Resolve a `<unit>` reference with existence hiding: an unknown OR an
/// unviewable unit is refused with the SAME line, so the refusal reveals nothing.
fn resolve_visible_unit(ctx: &Ctx<'_>, unit_ref: &str) -> Result<ObjId, CmdError> {
    match match_object(ctx.w, ctx.actor, unit_ref) {
        Ok(unit) if ctx.can_perceive(ctx.actor, unit) => Ok(unit),
        _ => Err(CmdError::msg(NO_SUCH_STATION)),
    }
}

/// Parse `<unit>/<station>` from the lhs.
fn parse_unit_station(ctx: &Ctx<'_>, lhs: &str) -> Result<(ObjId, StationId), CmdError> {
    let (unit_ref, station_ref) = lhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @station/<verb> <unit>/<station>[=<holder>]"))?;
    let unit = resolve_visible_unit(ctx, unit_ref.trim())?;
    let station = StationId::new(station_ref.trim())
        .ok_or_else(|| CmdError::msg("Usage: @station/<verb> <unit>/<station>[=<holder>]"))?;
    Ok((unit, station))
}

/// The requester's current control-generation on `unit`'s station — the exact
/// grant epoch the subject query and the apply-time witness compare.
fn requester_grant_epoch(ctx: &Ctx<'_>, unit: ObjId, station: &str) -> Option<u64> {
    actor_grant_epoch(ctx, ctx.actor, unit, station)
}

/// The same read for an arbitrary actor (E7c10 B4: the relief is staged as the
/// departing actor, who is not always `ctx.actor`).
pub(crate) fn actor_grant_epoch(
    ctx: &Ctx<'_>,
    actor: ObjId,
    unit: ObjId,
    station: &str,
) -> Option<u64> {
    ctx.w
        .command_subjects_of(actor)
        .iter()
        .find(|grant| grant.subject == unit && grant.station.as_str() == station)
        .map(|grant| grant.epoch)
}

/// The primary `@station` verbs. `handoff`/`cancel` are the authoritative
/// ingress; `show` is read-only; `fallback` writes the exempt record.
pub(crate) fn cmd_station(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("handoff") {
        station_handoff(ctx, false)
    } else if ctx.switches.has("cancel") {
        station_handoff(ctx, true)
    } else if ctx.switches.has("fallback") {
        station_fallback(ctx)
    } else if ctx.switches.has("show") || ctx.args.raw().trim().is_empty() {
        station_show(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @station/handoff <unit>/<station>   @station/cancel <unit>/<station>\n\
             @station/fallback <unit>/<station>=<who>   @station/show [<unit>/<station>]",
        ))
    }
}

/// `@station/handoff <unit>/<station>` (and `/cancel`) — select the migrated K6
/// control-handoff action for the actor's focused station and submit it on cut
/// 2's authoritative seam. A thin selector: no world/module mutation and no
/// success line — the mechanic and the confirmation are the provider's and the
/// durable receipt's. Authority and every coordinate are the offer's
/// (`project_station_offer`); this handler only proves reach/focus for an
/// immediate refusal and passes the exact subject revision. The existence-hiding
/// guarantee holds at BOTH the unit level (`resolve_visible_unit`) and the
/// station level (the `holds_billet` gate below — B1): a co-located non-holder
/// cannot tell an existing station from a missing one.
fn station_handoff(ctx: &mut Ctx<'_>, cancel: bool) -> Result<(), CmdError> {
    let (unit, station) = parse_unit_station(ctx, ctx.args.lhs())?;
    // Existence hiding (B1): a station is refused with the SAME `NO_SUCH_STATION`
    // line to any actor who does not hold that billet — whether the station is
    // absent OR merely not this actor's seat. `holds_billet` is false in both
    // cases, so a co-located non-holder can never distinguish an existing station
    // from a missing one, and cannot enumerate a unit's station complement. Only a
    // billet-holder (who already knows the seat exists) sees the "+focus" hint.
    if !holds_billet(ctx, ctx.actor, unit, &station) {
        return Err(CmdError::msg(NO_SUCH_STATION));
    }
    // Focus-gated: the offer is projected only for the actor's focused station
    // (the billet that proves authority and fixes the one channel). Refuse early
    // when the actor holds the billet but is not driving that station.
    if focused_station(ctx, ctx.actor) != Some((unit, station.clone()))
        && crate::handoff::physical_npc_station(ctx.w, ctx.actor, unit) != Some(station.clone())
    {
        return Err(CmdError::msg(format!(
            "You are not driving `{station}` there; +focus it first."
        )));
    }
    let action = if cancel {
        crate::handoff::cancel_action_ref(ctx.reg)
    } else {
        crate::handoff::request_action_ref(ctx.reg)
    }
    .ok_or_else(|| CmdError::msg("Station control is not available on this server."))?;
    let revision = requester_grant_epoch(ctx, unit, station.as_str())
        .ok_or_else(|| CmdError::msg(NO_SUCH_STATION))?;
    submit_station_action(ctx, action, unit, revision, cancel);
    Ok(())
}

/// Submit the request/cancel action through the one authoritative ingress. The
/// offer pins every parameter (`required_channels = []`), so the command
/// supplies none.
fn submit_station_action(
    ctx: &mut Ctx<'_>,
    action: mush_core::DefinitionRef,
    unit: ObjId,
    revision: u64,
    cancel: bool,
) {
    let at = ctx.w.world_ic_now().at;
    let verb = if cancel { "cancel" } else { "handoff" };
    let discriminator = format!("station.{verb}.{}.{at:x}", unit.0);
    let Ok(key) = IdempotencyKeyText::parse_canonical(&discriminator) else {
        info(ctx, "Station control is not available on this server.");
        return;
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::KnownObject { id: unit, revision },
        Vec::new(),
        json!({}),
        key,
    );
}

/// Request the existing K6 handoff as a physically seated NPC. The campaign
/// caller admits employment and unit permissions before arranging this request.
/// The offer and scheduled apply independently recheck the real console/billet.
pub fn request_npc_control(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    npc: ObjId,
) -> Result<bool, CmdError> {
    if crate::handoff::physical_npc_station(ctx.w, npc, unit).as_ref() != Some(station) {
        return Err(CmdError::msg(
            "NPC is not at its assigned physical console.",
        ));
    }
    let channel = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|state| crate::handoff::station_channel(state, unit, station))
        .ok_or_else(|| CmdError::msg(NO_SUCH_STATION))?;
    // Advisory duplicate suppression only. The campaign context cannot read
    // Sim's typed family. Actual admission and launch use the real provider.
    if crate::seat::channel_lease_holder(ctx.w, unit, &channel)
        == Some(mush_core::PrincipalId::Object { id: npc })
    {
        return Ok(false);
    }
    if ctx.authoritative_pending(npc)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err(CmdError::msg("A control request is already pending."));
    }
    let revision = actor_grant_epoch(ctx, npc, unit, station.as_str())
        .ok_or_else(|| CmdError::msg(NO_SUCH_STATION))?;
    let action = crate::handoff::request_action_ref(ctx.reg)
        .ok_or_else(|| CmdError::msg("Station control unavailable."))?;
    let key = IdempotencyKeyText::parse_canonical(format!(
        "npc.station.{}.{}.{}.{}",
        unit.0,
        npc.0,
        revision,
        ctx.w.world_ic_now().at
    ))
    .map_err(|_| CmdError::msg("Station request unavailable."))?;
    ctx.out
        .defer(mush_core::Deferred::AuthoritativeSubmit(Box::new(
            mush_core::AuthoritativeSubmit {
                actor: npc,
                action,
                subject: PerceivedRef::KnownObject { id: unit, revision },
                targets: Vec::new(),
                parameters: json!({}),
                key,
            },
        )));
    Ok(true)
}

/// `@station/show [<unit>/<station>]` — the actor's foreground station and its
/// lease disposition (read-only). Names no unit location.
fn station_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // ---- E7c11 independent verify fold, D8 ----
    // Resolve through the ingress. `focused_station` is `FocusState`'s
    // ACTOR-keyed read and a proxy focus is keyed on the NPC, so cut 11 answered
    // "You are driving no station." to somebody who was in fact steering — the
    // first surface a confused driver checks, and the one that made D1's
    // stranding hard to diagnose. The represented NPC is named (T12).
    let drive = crate::focus::proxy_drive_of(ctx.w, ctx.actor);
    let who = drive.as_ref().map_or(ctx.actor, |drive| drive.npc);
    let Some((unit, station)) = focused_station(ctx, who) else {
        info(ctx, "You are driving no station.");
        return Ok(());
    };
    let line = match &drive {
        Some(drive) => format!(
            "You are directing {} at `{station}` on {}.",
            mush_core::output::display_name(ctx.w, drive.npc),
            mush_core::output::display_name(ctx.w, unit)
        ),
        None => format!(
            "You are driving `{station}` on {}.",
            mush_core::output::display_name(ctx.w, unit)
        ),
    };
    info(ctx, line);
    Ok(())
}

/// `@station/fallback <unit>/<station>=<who>` (staff) — declare the deterministic
/// fallback controller for a station: one bounded `sim.station_fallback` record
/// plus its RAM mirror. The declared data is what the immediate controller reads
/// on a clean switch.
fn station_fallback(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // ---- E7c11: B0 (T11's guardrail, call site 4 of 4) ----
    // **A fallback change.** The staff gate below is an account power, and a
    // staff account can hold a proxy seat like anyone else; T11 forbids the act
    // *from the seat*, not from the person. Re-declaring the fallback while
    // driving would let the driver choose who the station returns to when they
    // stand down — the give-back T11 exists to keep deterministic.
    //
    // It runs **before** the staff gate deliberately: the refusal is about the
    // actor's own seat, which they already know they hold, so it discloses
    // nothing, and it names the blocker a driver can actually act on
    // (`+seat/leave`) instead of the one they cannot ("Permission denied.").
    crate::focus::seat_restriction(ctx.w, ctx.actor, crate::focus::ProxyAct::FallbackChange)
        .map_err(CmdError::msg)?;
    if !is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let (unit, station) = parse_unit_station(ctx, ctx.args.lhs())?;
    let holder = match_object(
        ctx.w,
        ctx.actor,
        &rhs_required(ctx, "@station/fallback <unit>/<station>=<who>")?,
    )?;
    // The station must exist on the unit.
    if st(ctx)
        .stations
        .get(&unit)
        .is_none_or(|hs| !hs.stations.contains_key(&station))
    {
        return Err(CmdError::msg(NO_SUCH_STATION));
    }
    let record = StationFallback {
        unit,
        station: station.clone(),
        holder,
    };
    // The RAM mirror the immediate controller reads. Bounded by the census
    // (B2/D2): refuse past the cap BEFORE writing the durable doc, so the RAM
    // mirror and the persisted set never disagree.
    if let Some(slot) = ctx.w.module_state_mut::<StationFallbacks>()
        && !slot.declare(unit, station.clone(), holder)
    {
        return Err(CmdError::msg(
            "Too many station fallbacks are declared; remove one first.",
        ));
    }
    // ...and the durable exempt record.
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: fallback_key(unit, &station),
            value: serde_json::to_value(&record).expect("station fallback serializes"),
        },
    });
    info(
        ctx,
        format!(
            "Declared {} as the fallback controller for `{station}` on {}.",
            mush_core::output::display_name(ctx.w, holder),
            mush_core::output::display_name(ctx.w, unit)
        ),
    );
    Ok(())
}

/// The immediate deterministic controller (B6 / invariant 6; K7.0 template). On
/// a clean release OR switch away from `station` on `unit`, if a fallback is
/// declared, hand the released station to it **immediately** — a direct
/// `submit_authoritative` with a deterministic key and **no `PendingDecision`**,
/// exactly the `mush-npc` `UseAbility` immediate-controller shape. Called from
/// BOTH `focus_release` and `focus_acquire`'s switch path (C2), **after** the
/// unit's control-generation bump and republish (C1), so the deferred submit
/// pins the NEW grant epoch — the offer re-sources the relief from the surviving
/// billet grant (`HandoffProvider::departed_fallback_station`) rather than the
/// focus the release just tore down, and so the seat actually fills.
pub(crate) fn fire_station_fallback(ctx: &mut Ctx<'_>, unit: ObjId, station: &StationId) {
    let actor = ctx.actor;
    let _ = fire_station_fallback_as(ctx, actor, unit, station);
}

/// The same fire, staged **as `actor`** rather than as `ctx.actor`.
///
/// **E7c10 B4/B6.** A proxy seat's stand-down, and the displaced own-station
/// handoff a seat activation performs, are both relieved by an actor who is not
/// the one typing the command: the NPC gives its station back to the declared
/// fallback, and the absent player's character gives back the station the seat
/// took its focus from. `Ctx::submit_authoritative` is hard-wired to
/// `ctx.actor`, and the engine keys its one-submit-per-drain slot by ACTOR, so
/// staging under the departing actor is both necessary (the offer is projected
/// for *that* actor's surviving billet grant) and safe (two actors, two slots,
/// neither fans the other out). Returns whether a submit was staged.
pub(crate) fn fire_station_fallback_as(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    unit: ObjId,
    station: &StationId,
) -> bool {
    let Some(holder) = fallbacks(ctx).and_then(|f| f.holder(unit, station)) else {
        return false;
    };
    let Some(action) = crate::handoff::request_action_ref(ctx.reg) else {
        return false;
    };
    let Some(revision) = actor_grant_epoch(ctx, actor, unit, station.as_str()) else {
        return false;
    };
    // Deterministic over (unit, station, holder): the model's role would only be
    // to select; the mapping here is fixed, so it enters as this actor's own
    // authoritative submission with no captured decision.
    let discriminator = format!(
        "station.fallback.{}.{}.{}",
        unit.0,
        station.as_str(),
        holder.0
    );
    let Ok(key) = IdempotencyKeyText::parse_canonical(&discriminator) else {
        return false;
    };
    ctx.out
        .defer(mush_core::Deferred::AuthoritativeSubmit(Box::new(
            mush_core::AuthoritativeSubmit {
                actor,
                action,
                subject: PerceivedRef::KnownObject { id: unit, revision },
                targets: Vec::new(),
                parameters: json!({}),
                key,
            },
        )));
    true
}
