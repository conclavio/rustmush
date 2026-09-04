//! Per-station channel grants, DERIVED from station assignments (cut 2). An
//! assignment to a control station grants the actor the unit as a command
//! subject at the `Operated` reach tier, so the unit's contextual behavior
//! commands (ruleset-tacsim STARTUP/HEADING, later) reach only its crew.
//!
//! The `command_subject_links` on `World` are treated as a pure projection of
//! [`HostAssignments`]: any change recomputes and republishes an actor's links
//! from scratch. This sidesteps the coarse `retract_command_subjects(owner,
//! actor)` API (which drops ALL of an actor's `sim` links) — we drop them all
//! and republish the survivors. `World::destroy` already scrubs links for a
//! dead actor/subject, so lifecycle cleanup only drops the durable records.

use mush_core::{ChannelGrant, Flags, ObjId, World};
use smol_str::SmolStr;

use crate::data::{NS, SpatialState};

/// Exact grants `actor` should currently hold: one per occupied station on a
/// **placed, live** unit. Channels, priority, and epoch stay paired on that
/// station all the way through command resolution and stale-intent checks.
fn desired_links(st: &SpatialState, w: &World, actor: ObjId) -> Vec<ChannelGrant> {
    let Some(units) = st.derived.assignments_by_actor.get(&actor) else {
        return Vec::new();
    };
    let mut grants = Vec::new();
    for unit in units.iter().copied() {
        if !st.is_placed(unit)
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
                channels: spec.channels.clone(),
                epoch,
            });
        }
    }
    grants
}

/// Recompute and republish `actor`'s `sim` control links from the current
/// assignments. Idempotent.
pub(crate) fn refresh_actor_links(w: &mut World, actor: ObjId) {
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

/// Refresh the links of every actor crewing `unit` (used when the unit's
/// placement changes so its crew's links follow — e.g. it is unplaced or
/// quarantined and must stop granting control).
pub(crate) fn refresh_unit_actors(w: &mut World, unit: ObjId) {
    let actors: Vec<ObjId> = {
        let st = w
            .module_state::<SpatialState>()
            .expect("sim state slot installed at registration");
        st.assignments
            .get(&unit)
            .map(|a| a.seats.values().map(|s| s.actor).collect())
            .unwrap_or_default()
    };
    for a in actors {
        refresh_actor_links(w, a);
    }
}
