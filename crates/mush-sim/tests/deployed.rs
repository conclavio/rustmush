//! **T0: a deployed unit is never a participant of a world-side pulse**
//! (16b cut T.3; doc 16 §3.1 "Custody", §3.6 "World-side exclusion").
//!
//! While an arena holds a unit, *it* is the single writer for that unit's pose
//! (D-16-1). The unit's world placement stays where it was and is deliberately
//! left frozen — the pilot's channel grants require a placed unit and would be
//! revoked by an unplace — so the placement graph goes on holding a row for it,
//! and every world-side reader of that row would go on reading a pose the arena
//! has moved on from.
//!
//! [`mush_sim::participants`] is that reader: it is the roster the domain pulse,
//! the atlas `+map` frame and every world unit listing take. So the exclusion
//! lives **in** it, and this file is what says so. The frozen placement does not
//! imply the exclusion on its own — a frozen placement is still a placement —
//! which is exactly why it had to be written.
//!
//! [`mush_sim::raw_participants`] is the unfiltered read, and it is here to prove
//! the difference is the marker and nothing else: the unit is still placed, still
//! in the domain, still at its frozen pose.

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_proto::OutputEvent;
use mush_sim::deployed::{DeployedMark, DeployedUnits};
use mush_sim::{DomainId, SimulationModule, SpatialParent, SpatialState};
use mush_testkit::{TestWorld, text_of};

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(SimulationModule::new())])
}

fn texts(events: &[(mush_core::Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let out = texts(&events);
    assert!(
        !out.contains("Huh?") && !out.to_lowercase().contains("usage:"),
        "`{line}` refused: {out}"
    );
    out
}

/// A domain with two things placed in it.
fn fixture(tw: &mut TestWorld) -> (DomainId, ObjId, ObjId) {
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .expect("a room");
    let left = tw
        .world_mut()
        .create(ObjKind::Thing, "Left Tank", GOD, Some(LIMBO))
        .expect("a thing");
    let right = tw
        .world_mut()
        .create(ObjKind::Thing, "Right Tank", GOD, Some(LIMBO))
        .expect("a thing");
    for obj in [left, right] {
        tw.world_mut()
            .attach_behavior(
                obj,
                mush_core::BehaviorId::new("sim.participant").expect("a canonical behavior"),
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )
            .expect("a thing may be a participant");
    }
    ok(tw, &format!("@behavior/add #{}=sim.domain", field.0));
    let created = ok(tw, &format!("@domain/create #{}=sim.passive", field.0));
    let domain = tw
        .world()
        .module_state::<SpatialState>()
        .and_then(|state| state.domains.keys().next().copied())
        .unwrap_or_else(|| panic!("the domain exists: {created}"));
    ok(
        tw,
        &format!("@spatial/place/domain #{}={}", left.0, domain.0),
    );
    ok(
        tw,
        &format!("@spatial/place/domain #{}={}", right.0, domain.0),
    );
    (domain, left, right)
}

#[test]
fn a_deployed_unit_leaves_the_world_side_roster_and_keeps_its_frozen_placement() {
    let mut tw = boot();
    let (domain, left, right) = fixture(&mut tw);

    let before: Vec<ObjId> = mush_sim::participants(tw.world(), domain)
        .into_iter()
        .map(|(obj, _)| obj)
        .collect();
    assert_eq!(
        before,
        vec![left, right],
        "both tanks are on the roster before either is deployed"
    );
    let frozen = mush_sim::pose_of(tw.world(), left).expect("the left tank is placed");

    mush_sim::deployed::mark(
        tw.world_mut(),
        left,
        DeployedMark::new(7, "tacsim.arena.proving_ground"),
    );

    // (1) The roster every world-side pulse, map frame and listing reads.
    let during: Vec<ObjId> = mush_sim::participants(tw.world(), domain)
        .into_iter()
        .map(|(obj, _)| obj)
        .collect();
    assert_eq!(
        during,
        vec![right],
        "a deployed unit is never a participant of a world-side pulse (doc 16 §3.1, §3.6)"
    );

    // (2) …and the difference is the marker, not the placement: the unit is
    // still placed, still in the domain, still exactly where it was.
    let raw: Vec<ObjId> = mush_sim::raw_participants(tw.world(), domain)
        .into_iter()
        .map(|(obj, _)| obj)
        .collect();
    assert_eq!(raw, vec![left, right], "the placement graph still holds it");
    let held = mush_sim::pose_of(tw.world(), left).expect("its placement is frozen, not removed");
    assert_eq!(held.0, frozen.0);
    assert_eq!(
        (held.1.x_cm, held.1.y_cm, held.1.heading_mdeg),
        (frozen.1.x_cm, frozen.1.y_cm, frozen.1.heading_mdeg),
        "the world never moved it"
    );
    assert!(matches!(
        tw.world()
            .module_state::<SpatialState>()
            .map(|state| state.parent_of(left)),
        Some(SpatialParent::Direct { .. })
    ));

    // (3) And clearing the marker puts it back, with nothing else changed.
    mush_sim::deployed::clear(tw.world_mut(), left);
    let after: Vec<ObjId> = mush_sim::participants(tw.world(), domain)
        .into_iter()
        .map(|(obj, _)| obj)
        .collect();
    assert_eq!(after, before, "check-in restores the roster exactly");
}

#[test]
fn the_spatial_moves_refuse_a_deployed_unit_by_name() {
    let mut tw = boot();
    let (domain, left, _right) = fixture(&mut tw);
    mush_sim::deployed::mark(
        tw.world_mut(),
        left,
        DeployedMark::new(9, "tacsim.arena.proving_ground"),
    );

    for line in [
        format!("@spatial/place/domain #{}={}", left.0, domain.0),
        format!("@spatial/unplace #{}", left.0),
    ] {
        let out = texts(&tw.run(GOD, &line));
        assert!(
            out.contains("deployed in arena match 9"),
            "`{line}` must refuse with the one named error: {out}"
        );
    }

    // Closing the window over it is refused for the same reason: retiring the
    // domain under a running match would leave the arena integrating a unit
    // whose world placement no longer exists.
    let out = texts(&tw.run(GOD, &format!("@domain/retire {}", domain.0)));
    assert!(
        !out.contains("Huh?"),
        "the retire verb exists on this composition: {out}"
    );

    mush_sim::deployed::clear(tw.world_mut(), left);
    let out = texts(&tw.run(GOD, &format!("@spatial/unplace #{}", left.0)));
    assert!(
        !out.contains("deployed in arena match"),
        "and a checked-in unit moves again: {out}"
    );
}

#[test]
fn the_index_is_composed_even_with_no_game_layer_above_it() {
    let tw = boot();
    let index = tw
        .world()
        .module_state::<DeployedUnits>()
        .expect("the sim composes the custody index unconditionally");
    assert!(
        index.is_empty(),
        "an admission site that reads a missing slot is an admission site that admits"
    );
}

/// **16b cut S.2: the `look` surface** (doc 16 §3.6 "World-side exclusion": *for
/// every viewer including staff*).
///
/// The room render is `mush-core`'s and cannot ask `mush-sim` anything, so the
/// exclusion arrives on the one seam built for a per-observer projection —
/// `HookPoint::Perceive`, which the contents loop consults and which is not waived
/// for staff. This is that hook, end to end through `look`.
#[test]
fn a_deployed_unit_is_not_in_the_room_a_looker_is_shown() {
    let mut tw = boot();
    let tank = tw
        .world_mut()
        .create(ObjKind::Thing, "Red Tank 1", GOD, Some(LIMBO))
        .expect("a tank");
    let seen = texts(&tw.run(GOD, "look"));
    assert!(
        seen.contains("Red Tank 1"),
        "the control: an ordinary thing is in the room:\n{seen}"
    );

    mush_sim::deployed::mark(
        tw.world_mut(),
        tank,
        DeployedMark::new(1, "tacsim.arena.proving_ground"),
    );
    let seen = texts(&tw.run(GOD, "look"));
    assert!(
        !seen.contains("Red Tank 1"),
        "a unit an arena holds is in nobody's room, staff included:\n{seen}"
    );

    mush_sim::deployed::clear(tw.world_mut(), tank);
    let seen = texts(&tw.run(GOD, "look"));
    assert!(
        seen.contains("Red Tank 1"),
        "and the check-in gives it back:\n{seen}"
    );
}
