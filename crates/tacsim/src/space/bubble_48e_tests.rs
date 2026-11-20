#![cfg(test)]
//! **Doc 48e review fixes, slice `kernel`**: G6 (a spinal round strikes the facet it approached),
//! G7 (a torpedo strikes the facet it approached), G17 (the facet test is integer) and G18 (the
//! state hash sees the mounts and the armour).

use super::cc8_tests::stock_hull;
use super::*;

/// A bubble with a stock corvette `shooter` (party 0) at `shooter_at` and a stock Dray
/// `target` (party 1) at the origin, at rest, nose along +x; hostile both ways.
fn range(shooter_at: [i64; 3], attitude_deg: (i32, i32, i32)) -> LocalSpaceBubble {
    let shooter = stock_hull(
        "tacsim.unit.corvette",
        "shooter",
        admitted_party(0),
        shooter_at,
        [0, 0, 0],
    );
    let mut target = stock_hull(
        "tacsim.unit.dray",
        "target",
        admitted_party(1),
        [0, 0, 0],
        [0, 0, 0],
    );
    target.attitude_deg = attitude_deg;
    target.pdc_auto = false;
    for laser in &mut target.lasers {
        laser.mode = LaserMode::Offense;
    }
    let mut bubble = LocalSpaceBubble::new("bubble.48e", "site.test", "48e", 1_000);
    bubble.set_party_relation(admitted_party(0), admitted_party(1), Some(Mark::Hostile));
    bubble.insert_ship(shooter);
    bubble.insert_ship(target);
    bubble
}

/// Step until the first strike on the target; its facet.
fn first_strike(bubble: &mut LocalSpaceBubble) -> Option<FacetBand> {
    let mut ic = 1_000;
    for _ in 0..1_200 {
        ic += 1;
        for event in bubble.step_wall_ms_at(MAX_SUBSTEP_MS, ic).events {
            match event {
                SpaceCombatEvent::RailgunHit { facet, .. }
                | SpaceCombatEvent::TorpedoHit { facet, .. } => return Some(facet),
                _ => {}
            }
        }
    }
    None
}

/// The facet a spinal round from `shooter_at` strikes on the Dray.
fn spinal_strike(shooter_at: [i64; 3], attitude_deg: (i32, i32, i32)) -> Option<FacetBand> {
    let mut bubble = range(shooter_at, attitude_deg);
    let fired = bubble.fire_railgun("shooter", "target");
    assert!(
        fired
            .iter()
            .any(|e| matches!(e, SpaceCombatEvent::RailgunFired { .. })),
        "the round leaves: {fired:?}"
    );
    first_strike(&mut bubble)
}

/// The facet a torpedo from `shooter_at` strikes on the Dray.
fn torpedo_strike(shooter_at: [i64; 3]) -> Option<FacetBand> {
    let mut bubble = range(shooter_at, (0, 0, 0));
    let fired = bubble.fire_torpedo_salvo("shooter", "target", 1);
    assert!(!fired.is_empty(), "a torpedo leaves");
    first_strike(&mut bubble)
}

/// **G6: a spinal round strikes the facet facing its shooter.** A round from dead ahead strikes
/// her bow, from astern her stern, from her right her right side; turned about, the same round
/// strikes her stern. Before, a round from any bearing at a hull at rest landed on its lead
/// point exactly and read `Right`.
#[test]
fn g6_a_spinal_round_strikes_the_facet_facing_its_shooter() {
    assert_eq!(
        spinal_strike([60_000, 0, 0], (0, 0, 0)),
        Some(FacetBand::Front)
    );
    assert_eq!(
        spinal_strike([-60_000, 0, 0], (0, 0, 0)),
        Some(FacetBand::Rear)
    );
    // Yaw 0: her right lies along +y.
    assert_eq!(
        spinal_strike([0, 60_000, 0], (0, 0, 0)),
        Some(FacetBand::Right)
    );
    assert_eq!(
        spinal_strike([0, -60_000, 0], (0, 0, 0)),
        Some(FacetBand::Left)
    );
    assert_eq!(
        spinal_strike([60_000, 0, 0], (180, 0, 0)),
        Some(FacetBand::Rear)
    );
}

/// **G7: a torpedo strikes the facet facing where it came from**, ahead or astern.
#[test]
fn g7_a_torpedo_strikes_the_facet_facing_where_it_came_from() {
    assert_eq!(torpedo_strike([40_000, 0, 0]), Some(FacetBand::Front));
    assert_eq!(torpedo_strike([-40_000, 0, 0]), Some(FacetBand::Rear));
}

/// **G6, G7: the approach point** is ten seconds back along the relative path.
#[test]
fn g6_the_approach_point_is_ten_seconds_back_along_the_relative_path() {
    // A round at +2 m, flying -x at 3 km/s against a target drifting +x at 10 m/s.
    assert_eq!(
        approach_point([2, 0, 0], [-3_000_000, 0, 0], [10_000, 0, 0]),
        [30_102, 0, 0]
    );
    assert_eq!(approach_point([5, 6, 7], [1, 2, 3], [1, 2, 3]), [5, 6, 7]);
}

/// **G17: the facet test's integer trigonometry** reads the table's quadrants, and the bands
/// stand where the float test put them for every whole-degree attitude on the axes.
#[test]
fn g17_the_facet_test_is_integer_and_keeps_the_bands() {
    assert_eq!(sin_q16(0), 0);
    assert_eq!(sin_q16(30), 32_768);
    assert_eq!(sin_q16(90), 65_536);
    assert_eq!(sin_q16(150), 32_768);
    assert_eq!(sin_q16(210), -32_768);
    assert_eq!(sin_q16(-30), -32_768);
    assert_eq!(cos_q16(60), 32_768);
    assert_eq!(cos_q16(180), -65_536);
    assert_eq!(cos_q16(-90), 0);

    let mut ship = stock_hull(
        "tacsim.unit.dray",
        "her",
        admitted_party(1),
        [0, 0, 0],
        [0, 0, 0],
    );
    // Yaw 0: bow +x, right +y, top +z. The float test's 0.65 edge sits at 49.46° off the bow.
    assert_eq!(
        ship.select_facet_for_attack([1_000, 0, 0]),
        FacetBand::Front
    );
    assert_eq!(
        ship.select_facet_for_attack([-1_000, 0, 0]),
        FacetBand::Rear
    );
    assert_eq!(
        ship.select_facet_for_attack([0, 1_000, 0]),
        FacetBand::Right
    );
    assert_eq!(
        ship.select_facet_for_attack([0, -1_000, 0]),
        FacetBand::Left
    );
    assert_eq!(ship.select_facet_for_attack([0, 0, 1_000]), FacetBand::Top);
    assert_eq!(
        ship.select_facet_for_attack([0, 0, -1_000]),
        FacetBand::Right
    );
    // 49° off the bow is in the bow band; 50° is out of it.
    assert_eq!(
        ship.select_facet_for_attack([656, 755, 0]),
        FacetBand::Front
    );
    assert_eq!(
        ship.select_facet_for_attack([643, 766, 0]),
        FacetBand::Right
    );
    assert_eq!(ship.select_facet_for_attack([0, 0, 0]), FacetBand::Right);
    // Yawed 90°: her bow is +y; pitched 90°: her bow is +z; rolled 180°: her top is -z.
    ship.attitude_deg = (90, 0, 0);
    assert_eq!(
        ship.select_facet_for_attack([0, 1_000, 0]),
        FacetBand::Front
    );
    assert_eq!(
        ship.select_facet_for_attack([-1_000, 0, 0]),
        FacetBand::Right
    );
    ship.attitude_deg = (0, 90, 0);
    assert_eq!(
        ship.select_facet_for_attack([0, 0, 1_000]),
        FacetBand::Front
    );
    ship.attitude_deg = (0, 0, 180);
    assert_eq!(ship.select_facet_for_attack([0, 0, -1_000]), FacetBand::Top);
    assert_eq!(
        ship.select_facet_for_attack([0, -1_000, 0]),
        FacetBand::Right
    );
}

/// One change to a ship.
type Edit = dyn Fn(&mut SpaceShipState);

/// **G18: the state hash sees a magazine, a cooldown, a laser's mode and a facet's armour.**
#[test]
fn g18_the_state_hash_sees_the_mounts_and_the_armour() {
    let base = range([60_000, 0, 0], (0, 0, 0));
    let quiet = base.state_hash();
    let changed = |edit: &dyn Fn(&mut SpaceShipState)| {
        let mut bubble = base.clone();
        edit(bubble.ships.get_mut("shooter").expect("the shooter"));
        bubble.state_hash()
    };
    let edits: [(&str, &Edit); 6] = [
        ("a tube's magazine", &|s| {
            s.torpedo_tubes[0].magazine_remaining -= 1
        }),
        ("a spinal round", &|s| s.railguns[0].rounds_remaining -= 1),
        ("a rotary's rounds", &|s| s.pdcs[0].ammo_remaining -= 1),
        ("a laser's mode", &|s| s.lasers[0].mode = LaserMode::Defense),
        ("a laser's cooldown", &|s| {
            s.lasers[0].cooldown_remaining_ms += 1
        }),
        ("a facet's armour", &|s| {
            let section = s.sections.values_mut().next().expect("a section");
            let armour = section.facets.values_mut().next().expect("a facet");
            *armour -= 1;
        }),
    ];
    for (what, edit) in edits {
        assert_ne!(changed(edit), quiet, "the hash sees {what}");
    }
}

/// **G14: a collector drone is soft**: unarmoured, and one offence laser shot at close range
/// disables it, where before it carried a Dray's hull section.
#[test]
fn g14_one_offence_laser_shot_disables_a_collector_drone() {
    let mut bubble = range([2_000, 0, 0], (0, 0, 0));
    bubble.ships.remove("target");
    bubble.insert_ship(SpaceShipState::new_collector_drone(
        "target",
        "Drone",
        admitted_party(1),
        "home",
        "collector",
        500,
        [0, 0, 0],
    ));
    let drone = &bubble.ships["target"];
    let hull = &drone.sections[&ShipSectionId::Hull];
    assert_eq!(hull.internals, DRONE_HULL_INTERNALS);
    assert!(hull.facets.values().all(|armour| *armour == 0));
    let shooter = bubble.ships.get_mut("shooter").expect("the shooter");
    for laser in &mut shooter.lasers {
        laser.mode = LaserMode::Offense;
    }
    let first = shooter.lasers[0].id.clone();
    let events = bubble.fire_offensive_lasers_from("shooter", "target", Some(&first));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SpaceCombatEvent::ShipDisabledDerelict { .. })),
        "{events:?}"
    );
}

/// **G16: the holotank's point-defence ring is her rotaries' envelope**, and none for a hull
/// that carries no rotary; never a fixed 15 km.
#[test]
fn g16_the_point_defence_ring_is_her_rotaries_envelope() {
    let bubble = range([60_000, 0, 0], (0, 0, 0));
    let corvette = &bubble.ships["shooter"];
    let widest = corvette
        .pdcs
        .iter()
        .map(|pd| pd.envelope_m)
        .max()
        .expect("rotaries");
    let ring = |feed: &serde_json::Value| feed["pdc_range_km"].as_f64().expect("a ring");
    assert!((ring(&bubble.holotank_feed("shooter")) - widest as f64 / 1_000.0).abs() < 1e-9);
    assert!(
        (widest as f64 / 1_000.0 - 15.0).abs() > 1e-9,
        "not the old fixed ring"
    );
    let mut bare = bubble.clone();
    bare.ships.get_mut("shooter").expect("her").pdcs.clear();
    assert!(ring(&bare.holotank_feed("shooter")).abs() < 1e-9, "no ring");
}
