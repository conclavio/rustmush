//! S9.3: the pulse tick's fire intents are gated on the window's line of fire.
//! `Fire` needs the standing line; `Launch` needs the line or the arc; two
//! units the window does not both hold are not gated; the refusal names
//! nothing about what blocks it.

use std::sync::Arc;

use mush_core::{GOD, ObjKind};
use mush_sim::tick::{IntentKind, NO_LINE, line_of_fire_gate};
use mush_sim::{MetricGeometry, SimulationModule};
use mush_testkit::TestWorld;

fn lot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(SimulationModule::new())])
}

/// A geometry that occludes the standing line between `a` and `b` only, and an
/// arc that clears it.
fn occluded_pair(tw: &mut TestWorld, a: mush_core::ObjId, b: mush_core::ObjId, arc: bool) {
    mush_sim::register_window_geometry(
        tw.world_mut(),
        Arc::new(move |_, near, far| {
            let pair = (near == a && far == b) || (near == b && far == a);
            pair.then_some(MetricGeometry {
                range_cells: 12,
                occluded: true,
            })
        }),
    )
    .expect("the geometry registers once");
    mush_sim::register_window_arc(
        tw.world_mut(),
        Arc::new(move |_, near, far| {
            let pair = (near == a && far == b) || (near == b && far == a);
            pair.then_some(arc)
        }),
    )
    .expect("the arc registers once");
}

#[test]
fn a_blocked_line_refuses_fire_and_a_clear_arc_allows_launch() {
    let mut tw = lot();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Ridge", GOD, None)
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Mech", GOD, Some(room))
        .unwrap();
    let foe = tw
        .world_mut()
        .create(ObjKind::Thing, "Foe", GOD, Some(room))
        .unwrap();
    occluded_pair(&mut tw, mech, foe, true);
    let refusal = line_of_fire_gate(tw.world(), IntentKind::Fire, mech, foe).unwrap_err();
    assert_eq!(refusal, NO_LINE);
    assert!(
        !refusal.contains("ridge") && !refusal.contains("Ridge"),
        "{refusal}"
    );
    assert!(line_of_fire_gate(tw.world(), IntentKind::Launch, mech, foe).is_ok());
    // A continuous kind is never gated.
    assert!(line_of_fire_gate(tw.world(), IntentKind::Heading, mech, foe).is_ok());
}

#[test]
fn a_blocked_line_and_a_blocked_arc_refuse_launch_too() {
    let mut tw = lot();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Ridge", GOD, None)
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Mech", GOD, Some(room))
        .unwrap();
    let foe = tw
        .world_mut()
        .create(ObjKind::Thing, "Foe", GOD, Some(room))
        .unwrap();
    occluded_pair(&mut tw, mech, foe, false);
    assert_eq!(
        line_of_fire_gate(tw.world(), IntentKind::Launch, mech, foe).unwrap_err(),
        NO_LINE
    );
}

#[test]
fn two_units_on_no_shared_window_are_not_gated() {
    let mut tw = lot();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Ridge", GOD, None)
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Mech", GOD, Some(room))
        .unwrap();
    let foe = tw
        .world_mut()
        .create(ObjKind::Thing, "Foe", GOD, Some(room))
        .unwrap();
    let other = tw
        .world_mut()
        .create(ObjKind::Thing, "Other", GOD, Some(room))
        .unwrap();
    occluded_pair(&mut tw, mech, foe, false);
    // The geometry answers for one pair only; every other pair is not on a window.
    assert!(line_of_fire_gate(tw.world(), IntentKind::Fire, mech, other).is_ok());
    // And a world with no geometry registered at all gates nothing.
    let bare = lot();
    assert!(line_of_fire_gate(bare.world(), IntentKind::Fire, mech, foe).is_ok());
}
