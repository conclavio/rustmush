//! **The field tells its arena each unit's posture** (doc 45 unit AP.3; Q-45-4, Q-45-23).
//!
//! The posture is the unit's own, kept on its object (`crate::posture::record`), and it was
//! there before the field opened and will be after it demotes. A field that flies the unit is
//! only a reader: this module copies the word to the field's arena, where the unit's frame and
//! its mind's view carry it (`crate::arena::state::UnitState::posture`).
//!
//! **When.** [`sync`] runs once a world second from the host's pass (`super::home::tick`) and
//! at once when the record is written (`crate::posture::record::set`). It offers the arena a
//! word for every unit this boot's host has not yet told the unit's task of, and for every
//! unit whose record differs from what it last told. So every unit is told once when a task
//! first holds it, whatever its posture: a unit that was on `GUARD` before the field opened
//! is on `GUARD` in it within a second of its admission, a unit that joins a running field
//! likewise (the task keeps the word until the join lands, `ArenaMsg::Posture`), and a posture
//! set mid-field is read at the mind's next think. A unit on a posture other than the default
//! is told again every [`REFRESH_WALL_S`] seconds, which costs one message and covers a unit
//! that left at a ramp and joined again while its row stood in the live record, whose fresh
//! arena row starts on the default.
//!
//! **A restart.** The resumed arena is set from the record before its task starts
//! ([`at_resume`]), since the snapshot's word may be older than the record's. What the host
//! had told is RAM and starts empty, so the first pass tells every unit again; the arena's
//! setter is idempotent.
//!
//! **A demotion** changes nothing: the arena's copy goes with the arena, and the record
//! stands.

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};

use super::FieldBook;
use crate::arena_task::{ArenaMsg, MatchId};
use crate::posture::Posture;

/// How often a unit off the default posture is told to its arena again, world wall seconds.
pub(crate) const REFRESH_WALL_S: i64 = 10;

/// **What this boot's host has told each running field's arena** (RAM, module state).
#[derive(Clone, Debug, Default)]
pub(crate) struct Told {
    /// The posture last accepted by the unit's task, and the task.
    rows: BTreeMap<ObjId, (MatchId, Posture)>,
    /// When every unit off the default was last told again.
    refreshed_wall_s: i64,
}

/// **Bring every running field's arena up to its units' records** (this module's head).
pub(crate) fn sync(ctx: &mut Ctx<'_>) {
    let Some(book) = ctx.w.module_state::<FieldBook>() else {
        return;
    };
    let units: Vec<(MatchId, ObjId)> = book
        .live
        .values()
        .flat_map(|hosted| hosted.live.units.iter().map(|row| (hosted.task, row.unit)))
        .collect();
    let now = ctx.w.now();
    let (told, refresh) = ctx
        .w
        .module_state::<Told>()
        .map(|told| {
            (
                told.rows.clone(),
                now.saturating_sub(told.refreshed_wall_s) >= REFRESH_WALL_S,
            )
        })
        .unwrap_or_default();
    let Some(tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return;
    };
    let mut rows = BTreeMap::new();
    for (task, unit) in units {
        let want = crate::posture::record::of(ctx.w, unit);
        let have = told
            .get(&unit)
            .filter(|(told_task, _)| *told_task == task)
            .map(|(_, posture)| *posture);
        // A unit this host has not told this task of is told whatever it holds: the host
        // does not know what the arena has for it (a join, a resume).
        let stale = have != Some(want);
        if !(stale || (refresh && !want.is_return())) {
            if let Some(posture) = have {
                rows.insert(unit, (task, posture));
            }
            continue;
        }
        if tasks
            .offer(
                task,
                ArenaMsg::Posture {
                    unit,
                    posture: want,
                },
            )
            .is_ok()
        {
            rows.insert(unit, (task, want));
        } else if let Some(posture) = have {
            // The task would not take the word now; the next pass offers it again.
            rows.insert(unit, (task, posture));
        }
    }
    if let Some(told) = ctx.w.module_state_mut::<Told>() {
        // Rebuilt from the live records: a unit no field holds any more is forgotten.
        told.rows = rows;
        if refresh {
            told.refreshed_wall_s = now;
        }
    }
}

/// **A resumed arena reads each unit's record** (this module's head, "a restart"): the
/// snapshot's word on a posture may be older than the record's, which is written at once.
pub(crate) fn at_resume(
    w: &mush_core::World,
    arena: &mut crate::arena::state::Arena,
    units: impl Iterator<Item = ObjId>,
) {
    for unit in units {
        arena.set_posture(unit, crate::posture::record::of(w, unit));
    }
}

#[cfg(test)]
mod tests {
    use mush_core::{Attr, GOD, ObjId, ObjKind, World};

    use crate::arena::PartyId;
    use crate::arena::test_support::{arena_of, pose, tank};
    use crate::posture::Posture;
    use crate::posture::record::{POSTURE_ATTR, of};

    /// **At a resume the record decides, both ways**: a unit whose snapshot says `HOLD` and
    /// whose record was set back to `RETURN` after it resumes on `RETURN`, and one whose
    /// record was set after the snapshot resumes on what the record says.
    #[test]
    fn ap3_a_resumed_arena_takes_each_units_posture_from_its_record() {
        let mut w = World::new_minimal();
        let mut make = |name: &str| {
            w.create(ObjKind::Thing, name, GOD, None)
                .expect("a thing is made")
        };
        let (older, newer) = (make("Older"), make("Newer"));
        // As the snapshot left them.
        let mut arena = arena_of(vec![
            tank(older, PartyId::RED, pose(1_500, 1_500)),
            tank(newer, PartyId::BLUE, pose(6_500, 1_500)),
        ]);
        assert!(arena.set_posture(older, Posture::Hold));
        // As the records stand now.
        w.set_attr(
            newer,
            POSTURE_ATTR,
            Attr::json(serde_json::json!({ "posture": "guard" }), GOD),
        )
        .expect("the record is written");
        assert_eq!(
            (of(&w, older), of(&w, newer)),
            (Posture::Return, Posture::Guard)
        );
        super::at_resume(&w, &mut arena, [older, newer, ObjId(9_999)].into_iter());
        assert_eq!(arena.posture(older), Posture::Return);
        assert_eq!(arena.posture(newer), Posture::Guard);
    }
}
