//! **The staff observer's picture of a match** (16b cut I.4; 16c N10, the
//! referee's third lever; doc 16 §1, §3.1 "Pulse phases" step 5, §5).
//!
//! Staff can start, drive, slow, place, abort and reset a match; this is what
//! lets them *see* one while it runs. [`Observer`] is the whole field — every
//! unit's callsign, side, cell, heading, speed, power, wreck, held flag and
//! controller, and how many rounds are in flight — built by
//! [`crate::arena::state::Arena::observer`] and handed out by phase 5 of
//! [`crate::arena::state::Arena::step`] on
//! [`crate::arena::step::StepOutput::observer`] **at most once per pulse and
//! at most once an arena second**, and only while
//! [`crate::arena::state::Arena::watched`] is set.
//!
//! # A type of its own, and why
//!
//! Doc 16 §1's observer rule is that nothing tells a pilot about a unit their
//! platform has not detected, and a pilot's [`crate::arena::step::Frame`] is
//! built per platform by `Arena::frames` for exactly that reason. This picture
//! is the opposite: it is objective and names everything, by object number. So
//! it is a **type distinct from `Frame`**: nothing converts one into the other,
//! it leaves the task in its own field of `ArenaOut::Frames` and the world
//! publishes it on its own feed (`arena.observer`) to at most eight staff
//! watchers. `crates/mush-contract-tests/tests/arena_library.rs`'s
//! `the_observer_has_no_path_into_a_pilot_feed` refuses its names in the cockpit
//! payload module and in the text of `Arena::frames`, so a path from here into a
//! pilot's feed is a reviewed diff and not an accident.
//!
//! # A view, and rate independence
//!
//! The picture reads state and writes none; the two fields that gate it,
//! `Arena::watched` and `Arena::last_observer_s`, are views beside
//! `last_frames` that no hash folds (`crate::arena::hash`'s head says so), so a
//! watched match is the same match — one event list and one hash sequence at
//! every tempo (`crates/tacsim/tests/arena.rs`,
//! `i4_a_watched_match_is_the_same_match`). How many pictures a run builds
//! depends on the cadence, and that is the point of the gate: it is the number
//! of distinct arena seconds the pulses landed in, never more.
//!
//! # Its size
//!
//! One compact row per unit: every field is a small integer, an enum or a
//! designator, so the row is `Copy` and a 300-unit picture is a few kilobytes on
//! the wire (`crate::match_::observer::observer_frame` pins the figure under
//! 16,384 bytes).

use mush_core::ObjId;

use crate::arena::clock::Tempo;
use crate::arena::state::{Arena, Controller, Designator, Power, Team};

/// **The field as staff see it** (16b cut I.4): the arena clock, the tempo,
/// how many rounds are in flight and one row per unit in unit-id order.
///
/// Built by [`Arena::observer`]; never converted into or out of a pilot's
/// [`crate::arena::step::Frame`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observer {
    /// Arena time, milliseconds since match start, when the picture was taken.
    pub now_ms: i64,
    /// The tempo the arena is in.
    pub tempo: Tempo,
    /// Rounds in flight: `Arena::shots`' length (one row per accepted shot
    /// whose impact has not been applied).
    pub in_flight: u32,
    /// Every unit, in unit-id order (the order [`Arena::units`] is kept in).
    pub units: Vec<ObserverUnit>,
}

/// **One unit's row of the staff picture** (16b cut I.4). Every field is read
/// off [`crate::arena::state::UnitState`] as it stands; nothing is derived
/// from what any platform has detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverUnit {
    /// The unit's object number.
    pub unit: ObjId,
    /// Its callsign (`RA`, `BC`), the same for every viewer.
    pub callsign: Designator,
    /// Its side.
    pub team: Team,
    /// The window cell it stands in, or `None` off the window.
    pub cell: Option<(u16, u16)>,
    /// Its heading, millidegrees clockwise from north.
    pub heading_mdeg: u32,
    /// Its signed speed, cm/s.
    pub speed_cms: i32,
    /// Its power state.
    pub power: Power,
    /// Whether it is a wreck (`MotionLimits::wreck`).
    pub wreck: bool,
    /// Whether it is held against a cell it could not enter
    /// (`UnitState::held`).
    pub held: bool,
    /// Who drives it: a pilot or the reflex controller.
    pub controller: Controller,
}

impl Arena {
    /// **The staff picture of this match, now** (16b cut I.4): one
    /// [`ObserverUnit`] per unit, in unit-id order, and `in_flight` the number
    /// of shots in flight. A pure read.
    #[must_use]
    pub fn observer(&self) -> Observer {
        Observer {
            now_ms: self.now_ms,
            tempo: self.clock.tempo(),
            in_flight: u32::try_from(self.shots.len()).unwrap_or(u32::MAX),
            units: self
                .units
                .iter()
                .map(|unit| ObserverUnit {
                    unit: unit.id,
                    callsign: unit.callsign,
                    team: unit.team,
                    cell: self.definition.terrain.cell_of(&unit.pose),
                    heading_mdeg: unit.pose.heading_mdeg,
                    speed_cms: unit.speed_cms,
                    power: unit.power,
                    wreck: unit.limits.wreck,
                    held: unit.held.is_some(),
                    controller: unit.controller,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;
    use smol_str::SmolStr;

    use super::*;
    use crate::arena::motion::{BlockedCause, Held};
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, pose, tank};

    const RED: ObjId = ObjId(301);
    const BLUE: ObjId = ObjId(302);
    const THIRD: ObjId = ObjId(303);

    /// A shot in flight from `shooter` to `target`, as `resolve_fire` leaves
    /// one — only the count is read here.
    fn shot(seq: u64, shooter: ObjId, target: ObjId) -> crate::arena::weapons::Shot {
        crate::arena::weapons::Shot {
            seq,
            shooter,
            weapon: SmolStr::new("cannon"),
            contact: crate::arena::state::ContactId(1),
            target,
            rounds: 1,
            fired_at_ms: 0,
            impact_at_ms: 1_000,
            range_cm: 1_000,
            aim_x_cm: 0,
            aim_y_cm: 0,
            aim_cell: None,
            damage: 1,
            resolution: crate::arena::resolution::Resolution::blind(
                "front",
                1_000,
                mush_atlas::Cover::None,
            ),
        }
    }

    /// **16b cut I.4**: the rows are in unit-id order whatever order the
    /// check-out named them in; a wreck reads `wreck`, a held unit `held`, a
    /// unit off the window has no cell; `in_flight` counts the shots.
    #[test]
    fn i4_the_picture_is_every_unit_in_id_order_as_it_stands() {
        let mut arena = arena_of(vec![
            tank(THIRD, Team::Blue, pose(6_500, 1_500)),
            tank(RED, Team::Red, pose(1_500, 2_500)),
            tank(BLUE, Team::Blue, pose(6_500, 5_500)),
        ]);
        if let Some(unit) = arena.unit_mut(BLUE) {
            unit.limits.wreck = true;
        }
        if let Some(unit) = arena.unit_mut(RED) {
            unit.held = Some(Held {
                x: 2,
                y: 2,
                cause: BlockedCause::Impassable,
            });
            unit.pose.heading_mdeg = 90_000;
            unit.speed_cms = 300;
        }
        if let Some(unit) = arena.unit_mut(THIRD) {
            // Off the window's west edge: no cell.
            unit.pose.x_cm = -1;
        }
        arena.shots.insert(1, shot(1, RED, BLUE));
        arena.shots.insert(2, shot(2, THIRD, RED));

        let picture = arena.observer();
        assert_eq!(picture.now_ms, 0);
        assert_eq!(picture.tempo, arena.clock.tempo());
        assert_eq!(picture.in_flight, 2);
        let ids: Vec<ObjId> = picture.units.iter().map(|row| row.unit).collect();
        assert_eq!(ids, vec![RED, BLUE, THIRD], "unit-id order");

        let red = picture.units.first().copied().expect("the red row");
        assert_eq!(red.team, Team::Red);
        assert_eq!(red.cell, Some((1, 2)));
        assert_eq!(red.heading_mdeg, 90_000);
        assert_eq!(red.speed_cms, 300);
        assert!(red.held && !red.wreck);
        assert_eq!(red.power, Power::Off);
        assert_eq!(red.controller, Controller::Pilot);
        assert_eq!(
            red.callsign,
            arena.unit(RED).map(|unit| unit.callsign).expect("red")
        );

        let blue = picture.units.get(1).copied().expect("the blue row");
        assert!(blue.wreck && !blue.held);
        assert_eq!(blue.cell, Some((6, 5)));

        let third = picture.units.get(2).copied().expect("the third row");
        assert_eq!(third.cell, None, "a unit off the window has no cell");
    }

    /// Two tanks, one a side, far enough apart to leave each other alone.
    fn duel() -> Arena {
        arena_of(vec![
            tank(RED, Team::Red, pose(1_500, 1_500)),
            tank(BLUE, Team::Blue, pose(6_500, 6_500)),
        ])
    }

    /// **16b cut I.4**, the cadence: at most once per pulse and at most once an
    /// arena second. The opening pulse builds one for second 0; a second pulse
    /// inside that second builds none; the next second builds another; one long
    /// pulse builds one, whatever it spans.
    #[test]
    fn i4_the_picture_is_built_at_most_once_an_arena_second() {
        let mut arena = duel();
        arena.watched = true;
        let opening = arena.step(0, &[], &[]);
        let first = opening.observer.expect("the opening pulse builds one");
        assert_eq!(first.now_ms, 0);
        assert_eq!(arena.last_observer_s, Some(0));

        let inside = arena.step(400, &[], &[]);
        assert!(arena.now_ms > 0 && arena.now_ms < 1_000, "{}", arena.now_ms);
        assert!(
            inside.observer.is_none(),
            "second 0 already has its picture"
        );

        let next = arena.step(1_000, &[], &[]);
        assert_eq!(arena.now_ms, 1_000);
        let second = next.observer.expect("second 1 builds one");
        assert_eq!(second.now_ms, 1_000);
        assert_eq!(second, arena.observer(), "built from the pulse's state");
        assert!(arena.step(1_250, &[], &[]).observer.is_none());

        // One long pulse: however many seconds it spans, one picture.
        let long = arena.step(6_250, &[], &[]);
        assert!(arena.now_ms >= 5_000, "{}", arena.now_ms);
        assert_eq!(
            long.observer.map(|picture| picture.now_ms),
            Some(arena.now_ms),
            "one picture for the long pulse"
        );
        assert_eq!(arena.last_observer_s, Some(arena.now_ms.div_euclid(1_000)));
    }

    /// **16b cut I.4**: nobody watching, nothing built — and the gate is left
    /// where it was.
    #[test]
    fn i4_an_unwatched_match_builds_no_picture() {
        let mut arena = duel();
        for wall in [0, 500, 1_000, 2_000, 7_000] {
            assert!(arena.step(wall, &[], &[]).observer.is_none(), "{wall}");
        }
        assert_eq!(arena.last_observer_s, None);
    }

    /// **16b cut I.4**: the pulse the match ends on builds a picture even
    /// inside a second that already has one, so a watcher sees the end.
    #[test]
    fn i4_the_ending_pulse_builds_a_picture() {
        let mut arena = duel();
        arena.watched = true;
        assert!(arena.step(0, &[], &[]).observer.is_some());
        assert!(arena.step(1_000, &[], &[]).observer.is_some());
        // The duration bound, as phase 5's backstop reads it, is passed on
        // the next pulse — inside second 1, which already has its picture.
        arena.match_cfg.max_duration_ms = 1_200;
        let out = arena.step(1_500, &[], &[]);
        assert!(out.ended.is_some(), "the match ended on this pulse");
        assert_eq!(arena.now_ms.div_euclid(1_000), 1);
        assert!(
            out.observer.is_some(),
            "the ending pulse carries the picture"
        );
        // And an ended match's early return carries none.
        assert!(arena.step(3_000, &[], &[]).observer.is_none());
    }
}
