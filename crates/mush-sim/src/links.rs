//! Per-station channel grants, DERIVED from station assignments (cut 2). An
//! assignment to a control station grants the actor the unit as a command
//! subject at the `Operated` reach tier, so the unit's contextual behavior
//! commands (tacsim STARTUP/HEADING, later) reach only its crew.
//!
//! The `command_subject_links` on `World` are treated as a pure projection of
//! [`crate::HostAssignments`]: any change recomputes and republishes an actor's links
//! from scratch. This sidesteps the coarse `retract_command_subjects(owner,
//! actor)` API (which drops ALL of an actor's `sim` links) — we drop them all
//! and republish the survivors. `World::destroy` already scrubs links for a
//! dead actor/subject, so lifecycle cleanup only drops the durable records.

use std::collections::BTreeSet;

use mush_core::{ChannelGrant, ControlLeaseKey, Ctx, Flags, ObjId, ObjKind, World};
use smol_str::SmolStr;

use crate::data::{NS, SpatialCounterError, SpatialState, persist_control_gen};
use crate::focus::FocusState;
use crate::journey::JourneyBook;
use crate::seat::ControlLeaseView;

/// Whether `unit`'s control surface is live — the gate a crew grant hangs on.
///
/// **E7 cut 6 B3 (10b §3.6 contradiction (v), the T lens).** This used to be a
/// bare `st.is_placed(unit)`. A carrier root under way is deliberately *not*
/// directly placed (only its subtree is), so the bare placement test silently
/// dropped every crew grant on a ship the moment it left — the pilot lost the
/// helm mid-passage and got it back on arrival, with no refusal and no line of
/// output anywhere. Controls are live when the unit is placed **or** when it is
/// a carrier root that is currently under way; `journey::is_under_way` is the
/// one predicate the seal, the move/carry gate and the tick all share, so the
/// four can never disagree about what "moving" means.
fn controls_are_live(st: &SpatialState, w: &World, unit: ObjId) -> bool {
    st.is_placed(unit)
        || w.module_state::<JourneyBook>()
            .is_some_and(|book| book.under_way(unit))
}

/// Exact grants `actor` should currently hold: one per occupied station on a
/// **live-controlled, live** unit. Channels, priority, and epoch stay paired on
/// that station all the way through command resolution and stale-intent checks.
fn desired_links(st: &SpatialState, w: &World, actor: ObjId) -> Vec<ChannelGrant> {
    let Some(units) = st.derived.assignments_by_actor.get(&actor) else {
        return Vec::new();
    };
    let mut grants = Vec::new();
    for unit in units.iter().copied() {
        if !controls_are_live(st, w, unit)
            || w.get(unit)
                .is_none_or(|object| object.flags.contains(Flags::GOING))
        {
            continue;
        }
        let (Some(assignments), Some(stations)) =
            (st.assignments.get(&unit), st.stations.get(&unit))
        else {
            continue;
        };
        let epoch = st.control_generation(unit);
        for (station, assignment) in &assignments.seats {
            if assignment.actor != actor {
                continue;
            }
            let Some(spec) = stations.stations.get(station) else {
                continue;
            };
            grants.push(ChannelGrant {
                actor,
                subject: unit,
                station: SmolStr::new(station.as_str()),
                relation: spec.role.clone(),
                owner: SmolStr::new_static(NS),
                priority: spec.priority,
                // E7c10 B1 (10b §3.10 OPEN 2, T11's mechanical half): ONE extra
                // intersection term inside the ONE production projection. A
                // seat with no narrowing (`granted == None` — every ordinary
                // billet and every pre-cut-10 document) takes the station's own
                // set unchanged; a proxy seat takes the intersection, so an
                // action whose `required_channels` fall outside it projects NO
                // offer at all. No tenth `ChannelGrant` consumer, no second
                // projection, no second focus: the 08g §10 C12 census stands at
                // nine and `desired_links` is still the only production writer.
                channels: assignment.granted_channels(spec),
                epoch,
            });
        }
    }
    grants
}

/// Recompute and republish `actor`'s `sim` control links from the current
/// assignments. Idempotent.
pub fn refresh_actor_links(w: &mut World, actor: ObjId) {
    let desired = {
        let st = w
            .module_state::<SpatialState>()
            .expect("sim state slot installed at registration");
        desired_links(st, w, actor)
    };
    w.retract_command_subjects(NS, Some(actor));
    for grant in desired {
        w.publish_command_subject(grant);
    }
}

/// Refresh the links of every actor crewing `unit` or any unit in its spatial
/// subtree (used when placement changes so every affected crew's links follow —
/// e.g. an unplaced or quarantined unit stops granting control, and a newly
/// placed or re-parented unit republishes grants at its fresh control epoch;
/// `P2-MOD-63`).
pub(crate) fn refresh_unit_actors(w: &mut World, unit: ObjId) {
    let actors: BTreeSet<ObjId> = {
        let st = w
            .module_state::<SpatialState>()
            .expect("sim state slot installed at registration");
        let mut out = BTreeSet::new();
        for node in st.subtree(unit) {
            if let Some(a) = st.assignments.get(&node) {
                out.extend(a.seats.values().map(|s| s.actor));
            }
        }
        out
    };
    for a in actors {
        refresh_actor_links(w, a);
    }
}

/// Whether `unit` currently has an active or pending K6 control lease.
///
/// Consults the world-owned [`ControlLeaseView`] mirror (which tracks both
/// active holders and pending handoffs across foreign-module calls such as
/// `tacsim`), and also probes `ctx.typed_state` directly for every channel
/// declared on `unit` when `ctx` carries `"sim"` read authority.
fn has_unit_control_lease(ctx: &Ctx<'_>, st: &SpatialState, unit: ObjId) -> bool {
    let Some(view) = ctx.w.module_state::<ControlLeaseView>() else {
        return false;
    };
    if view.has_unit_lease(unit) {
        return true;
    }
    let Some(handle) = view.handle() else {
        return false;
    };
    let Some(host_stations) = st.stations.get(&unit) else {
        return false;
    };
    for spec in host_stations.stations.values() {
        for channel in &spec.channels {
            let key = ControlLeaseKey::new(unit, channel.clone());
            if let Ok(Some(lease)) = ctx.typed_state.get(handle, &key)
                && (lease.holder().is_some() || lease.pending().is_some())
            {
                return true;
            }
        }
    }
    false
}

/// Whether any live `ObjKind::Npc` currently stands in a room belonging to a
/// space anchored on `unit` (`SpaceAnchor::MovingObject { host: unit }`),
/// covering `NpcStationValidator` in `handoff.rs`.
fn has_live_npc_in_anchored_space(st: &SpatialState, w: &World, unit: ObjId) -> bool {
    let has_live_npc = |room: ObjId| {
        w.contents_of(room).iter().any(|&obj| {
            w.get(obj)
                .is_some_and(|o| o.kind == ObjKind::Npc && !o.flags.contains(Flags::GOING))
        })
    };
    if let Some(spaces) = st.derived.anchored_spaces.get(&unit) {
        for &space in spaces {
            if has_live_npc(space) {
                return true;
            }
            if let Some(rec) = st.spaces.get(&space)
                && rec.rooms.iter().any(|&room| has_live_npc(room))
            {
                return true;
            }
        }
    }
    false
}

/// Whether `unit` currently has an active control holder or live revision
/// witness depending on `control_generation(unit)`:
/// 1. non-empty station assignments (`HostAssignments::seats`),
/// 2. active foreground `FocusState` targeting `unit`,
/// 3. active or pending K6 control lease on `unit`, or
/// 4. a live `ObjKind::Npc` inside a space anchored to `unit`.
fn unit_has_live_control_witness(ctx: &Ctx<'_>, st: &SpatialState, unit: ObjId) -> bool {
    if st
        .assignments
        .get(&unit)
        .is_some_and(|a| !a.seats.is_empty())
    {
        return true;
    }
    if ctx
        .w
        .module_state::<FocusState>()
        .is_some_and(|focus| focus.has_focus_on(unit))
    {
        return true;
    }
    if has_unit_control_lease(ctx, st, unit) {
        return true;
    }
    has_live_npc_in_anchored_space(st, ctx.w, unit)
}

/// Preflight control-generation bumps for every unit in `root`'s spatial
/// subtree that has an active control holder or live revision witness depending
/// on `control_generation(unit)` before a placement transition, so counter
/// exhaustion refuses atomically with zero mutation while unmanned, unleased,
/// unfocused units emit no redundant `cgen:` KV write (`P2-MOD-63`).
pub(crate) fn prepare_subtree_control_bumps(
    ctx: &Ctx<'_>,
    st: &SpatialState,
    root: ObjId,
) -> Result<Vec<(ObjId, u64)>, SpatialCounterError> {
    prepare_subtree_control_bumps_inner(ctx, st, root, false)
}

/// Staff `@spatial/place` / `@spatial/unplace` variant of
/// [`prepare_subtree_control_bumps`] that also bumps `root` itself whenever
/// `root` declares stations (preserving the F3 contract for direct staff
/// placement transitions on a station host).
pub(crate) fn prepare_subtree_control_bumps_for_staff_place(
    ctx: &Ctx<'_>,
    st: &SpatialState,
    root: ObjId,
) -> Result<Vec<(ObjId, u64)>, SpatialCounterError> {
    prepare_subtree_control_bumps_inner(ctx, st, root, true)
}

fn prepare_subtree_control_bumps_inner(
    ctx: &Ctx<'_>,
    st: &SpatialState,
    root: ObjId,
    include_root_station_host: bool,
) -> Result<Vec<(ObjId, u64)>, SpatialCounterError> {
    let mut bumps = Vec::new();
    for unit in st.subtree(root) {
        let is_root_host =
            include_root_station_host && unit == root && st.stations.contains_key(&unit);
        if is_root_host || unit_has_live_control_witness(ctx, st, unit) {
            let next = st.next_control_generation(unit)?;
            bumps.push((unit, next));
        }
    }
    Ok(bumps)
}

/// Commit previously preflighted control-generation bumps FIRST, persist each
/// `cgen:<unit>`, and THEN republish `ChannelGrant` links for every actor
/// crewing `root` or any descendant in its subtree (bump-then-republish
/// invariant; `P2-MOD-63`).
pub(crate) fn commit_control_bumps_and_refresh(
    ctx: &mut Ctx<'_>,
    root: ObjId,
    bumps: &[(ObjId, u64)],
) {
    if !bumps.is_empty() {
        if let Some(st) = ctx.w.module_state_mut::<SpatialState>() {
            for &(unit, next) in bumps {
                st.apply_control_generation(unit, next);
            }
        }
        for &(unit, next) in bumps {
            persist_control_gen(ctx.out, unit, next);
        }
    }
    refresh_unit_actors(ctx.w, root);
}
