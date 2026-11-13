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
//!
//! **Whose unit it is** (doc 45 unit AP.6; Q-45-8). A unit on `GUARD` may answer fire on
//! another unit of the same player, and the arena knows no players, so the word carries one
//! ([`player_of`]): the player seated in the unit; for a unit nobody sits in, its carrier's
//! commander if it is a carrier, else the operator who launched it. A unit a hired crew sits
//! in has none: a crew is people in the world and not the player's unit (Q-45-13). It is told
//! and told again exactly as the posture is.
//!
//! **A carrier is home** (doc 45 unit AP.7; Q-45-11). `RETURN` means go home, and a carrier
//! on its pad is there: for it the word means stay. So the arena of a carrier whose record
//! says `RETURN` is told `HOLD` ([`flown_by`]): its mind stands where it is and answers only a
//! unit that has fired on it, by the orders `HOLD` already lays, and the kernel is never asked
//! to drive an immobile hull anywhere. The record, and everything a player reads of it, still
//! says `RETURN`. No verb sets a carrier's posture today, so this is every carrier.

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
    /// The posture and the player last accepted by the unit's task, and the task.
    rows: BTreeMap<ObjId, (MatchId, Posture, Option<ObjId>)>,
    /// When every unit off the default was last told again.
    refreshed_wall_s: i64,
}

/// **Bring every running field's arena up to its units' records** (this module's head).
pub(crate) fn sync(ctx: &mut Ctx<'_>) {
    let Some(book) = ctx.w.module_state::<FieldBook>() else {
        return;
    };
    let units: Vec<(MatchId, ObjId, Option<ObjId>)> = book
        .live
        .values()
        .flat_map(|hosted| {
            hosted.live.units.iter().map(|row| {
                (
                    hosted.task,
                    row.unit,
                    row.pilot.as_ref().map(|(pilot, _)| *pilot),
                )
            })
        })
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
    for (task, unit, pilot) in units {
        let want = (flown_by(ctx.w, unit), player_of(ctx, unit, pilot));
        let have = told
            .get(&unit)
            .filter(|(told_task, _, _)| *told_task == task)
            .map(|(_, posture, player)| (*posture, *player));
        // A unit this host has not told this task of is told whatever it holds: the host
        // does not know what the arena has for it (a join, a resume).
        let stale = have != Some(want);
        if !(stale || (refresh && !want.0.is_return())) {
            if let Some((posture, player)) = have {
                rows.insert(unit, (task, posture, player));
            }
            continue;
        }
        if tasks
            .offer(
                task,
                ArenaMsg::Posture {
                    unit,
                    posture: want.0,
                    player: want.1,
                },
            )
            .is_ok()
        {
            rows.insert(unit, (task, want.0, want.1));
        } else if let Some((posture, player)) = have {
            // The task would not take the word now; the next pass offers it again.
            rows.insert(unit, (task, posture, player));
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

/// **The posture `unit`'s arena is told** (this module's head, "a carrier is home"): the
/// unit's own record, except that a carrier on `RETURN` is told `HOLD`.
#[must_use]
pub(crate) fn flown_by(w: &mush_core::World, unit: ObjId) -> Posture {
    match crate::posture::record::of(w, unit) {
        Posture::Return if super::carrier::is_carrier(w, unit) => Posture::Hold,
        posture => posture,
    }
}

/// **The player whose unit `unit` is** (this module's head, "whose unit it is"), `pilot`
/// being who the field's row says sits in it.
#[must_use]
pub(crate) fn player_of(ctx: &Ctx<'_>, unit: ObjId, pilot: Option<ObjId>) -> Option<ObjId> {
    let player = |who: ObjId| (!super::orders::is_npc(ctx.w, who)).then_some(who);
    if let Some(pilot) = pilot {
        return player(pilot);
    }
    if super::carrier::is_carrier(ctx.w, unit) {
        return crate::port::coop::helm_holder(ctx.w, unit).and_then(player);
    }
    super::parties::rig_record(ctx, unit)
        .and_then(|record| record.excursion.map(|out| out.operator))
        .and_then(player)
}

/// **A resumed arena reads each unit's record** (this module's head, "a restart"): the
/// snapshot's word on a posture may be older than the record's, which is written at once.
/// Whose each unit is (doc 45 unit AP.6) is the snapshot's until the first pass tells it
/// again, within a second.
pub(crate) fn at_resume(
    w: &mush_core::World,
    arena: &mut crate::arena::state::Arena,
    units: impl Iterator<Item = ObjId>,
) {
    for unit in units {
        arena.set_posture(unit, flown_by(w, unit));
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

    /// **Doc 45 unit AP.7: a unit that is no carrier is told its own word** (Q-45-11): only
    /// a carrier's `RETURN` is told as `HOLD`, and a Thing with no hull row is no carrier.
    #[test]
    fn ap7_only_a_carriers_return_is_told_as_hold() {
        let mut w = World::new_minimal();
        let rig = w
            .create(ObjKind::Thing, "Rig", GOD, None)
            .expect("a thing is made");
        assert!(!crate::field::carrier::is_carrier(&w, rig));
        assert_eq!(super::flown_by(&w, rig), Posture::Return);
        w.set_attr(
            rig,
            POSTURE_ATTR,
            Attr::json(serde_json::json!({ "posture": "guard" }), GOD),
        )
        .expect("the record is written");
        assert_eq!(super::flown_by(&w, rig), Posture::Guard);
    }
}
