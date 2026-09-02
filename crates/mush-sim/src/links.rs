//! Control command-subjects, DERIVED from station assignments (cut 2). An
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

use mush_core::{CommandSubjectLink, Flags, ObjId, World};
use smol_str::SmolStr;

use crate::data::{NS, SpatialState, role_priority};

/// The (unit, role, priority) links `actor` should currently hold: one per
/// unit the actor crews, for a **placed, live** unit, at the actor's
/// highest-priority role on that unit.
fn desired_links(st: &SpatialState, w: &World, actor: ObjId) -> Vec<(ObjId, SmolStr, u16)> {
    let Some(units) = st.derived.assignments_by_actor.get(&actor) else {
        return Vec::new();
    };
    units
        .iter()
        .copied()
        .filter(|&unit| st.is_placed(unit) && w.get(unit).is_some_and(|o| !o.flags.contains(Flags::GOING)))
        .filter_map(|unit| {
            let seats = &st.assignments.get(&unit)?.seats;
            let hs = st.stations.get(&unit)?;
            let mut best: Option<(SmolStr, u16)> = None;
            for (sid, asg) in seats {
                if asg.actor == actor
                    && let Some(spec) = hs.stations.get(sid)
                {
                    let pr = role_priority(&spec.role);
                    if best.as_ref().is_none_or(|(_, b)| pr > *b) {
                        best = Some((spec.role.clone(), pr));
                    }
                }
            }
            best.map(|(role, pr)| (unit, role, pr))
        })
        .collect()
}

/// Recompute and republish `actor`'s `sim` control links from the current
/// assignments. Idempotent.
pub(crate) fn refresh_actor_links(w: &mut World, actor: ObjId) {
    let desired = {
        let st = w.module_state::<SpatialState>().expect("sim state slot installed at registration");
        desired_links(st, w, actor)
    };
    w.retract_command_subjects(NS, Some(actor));
    for (unit, relation, priority) in desired {
        w.publish_command_subject(CommandSubjectLink {
            actor,
            subject: unit,
            relation,
            owner: SmolStr::new_static(NS),
            priority,
        });
    }
}

/// Refresh the links of every actor crewing `unit` (used when the unit's
/// placement changes so its crew's links follow — e.g. it is unplaced or
/// quarantined and must stop granting control).
pub(crate) fn refresh_unit_actors(w: &mut World, unit: ObjId) {
    let actors: Vec<ObjId> = {
        let st = w.module_state::<SpatialState>().expect("sim state slot installed at registration");
        st.assignments
            .get(&unit)
            .map(|a| a.seats.values().map(|s| s.actor).collect())
            .unwrap_or_default()
    };
    for a in actors {
        refresh_actor_links(w, a);
    }
}
