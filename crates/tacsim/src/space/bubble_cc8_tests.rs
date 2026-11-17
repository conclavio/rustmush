#![cfg(test)]
//! **Doc 49 unit CC.8: the space palette, the cycle per mode and the screen balance**
//! (Q-49-7, Q-49-10, Q-49-19, Q-49-24, Q-49-25, Q-49-26).
//!
//! The measurement fires one shooter's full magazine at each stock hull, as her fit lays her
//! screen, and counts what lands. The cut note keeps the table it gave; the tests below hold
//! the outcomes the palette was set for.

use mush_core::ObjId;

use super::*;
use crate::space::custody;

/// The hull `key` names (a unit id, or the interdictor's overlay key), as check-out builds her
/// and as the picket's cast lays an overlay over her.
pub(super) fn stock_hull(
    key: &str,
    id: &str,
    party: PartyId,
    pos_m: [i64; 3],
    vel_mmps: [i64; 3],
) -> SpaceShipState {
    let unit_key = match key {
        "tacsim.unit.corvette_interdictor" => "tacsim.unit.corvette",
        other => other,
    };
    let (unit, _) =
        custody::instantiate_stock_space_unit(ObjId(77), unit_key).expect("a stock space unit");
    let mut ship = custody::space_ship_from_tactical_unit(&unit, id, id, party, pos_m, vel_mmps)
        .expect("her ship");
    if unit_key != key {
        custody::apply_space_unit_overlay(&mut ship, &custody::space_unit_overlay_for_id(key));
    }
    ship
}

/// How a screen stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Posture {
    /// Her lasers in the modes her fit starts them in; automatic point defence on.
    AsFitted,
    /// Every laser her fit allows in defence, a whole defence cycle before the first salvo.
    Braced,
}

/// What a lone shooter's full magazine did to one screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Outcome {
    pub fired: u32,
    pub landed: u32,
    pub by_laser: u32,
    pub by_rotary: u32,
    pub lost: u32,
    pub rotary_rounds: u32,
    /// How many had landed when she was left disabled; `None` if she held.
    pub disabled_at: Option<u32>,
}

/// Overrides for a sweep: each `None` keeps the shipped figure.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Knobs {
    pub defence_cycle_ms: Option<u32>,
    pub envelope_m: Option<i64>,
    pub damage_per_burst: Option<u16>,
}

/// A bubble with `shooter` (party 0) and `screen` (party 1), hostile both ways.
fn duel(shooter: SpaceShipState, screen: SpaceShipState) -> LocalSpaceBubble {
    let mut bubble = LocalSpaceBubble::new("bubble.cc8", "site.test", "CC.8", 1_000);
    bubble.set_party_relation(admitted_party(0), admitted_party(1), Some(Mark::Hostile));
    bubble.insert_ship(shooter);
    bubble.insert_ship(screen);
    bubble
}

/// **Fire `shooter_key`'s whole tube magazine at `screen_key`**, `range_m` off, the shooter
/// closing at `closing_mmps`, a salvo at each ready moment, and step until every torpedo is
/// done.
pub(super) fn measure(
    shooter_key: &str,
    screen_key: &str,
    posture: Posture,
    range_m: i64,
    closing_mmps: i64,
    knobs: Knobs,
) -> Outcome {
    let shooter = stock_hull(
        shooter_key,
        "shooter",
        admitted_party(0),
        [0, 0, 0],
        [closing_mmps, 0, 0],
    );
    let mut screen = stock_hull(
        screen_key,
        "screen",
        admitted_party(1),
        [range_m, 0, 0],
        [0, 0, 0],
    );
    screen.pdc_auto = true;
    for laser in &mut screen.lasers {
        if let Some(cycle) = knobs.defence_cycle_ms {
            laser.defence_cooldown_ms = Some(cycle);
        }
        if posture == Posture::Braced && laser.allowed_modes.defence {
            laser.mode = LaserMode::Defense;
        }
    }
    for pd in &mut screen.pdcs {
        if let Some(envelope) = knobs.envelope_m {
            pd.envelope_m = envelope;
        }
        if let Some(damage) = knobs.damage_per_burst {
            pd.damage_per_burst = damage;
        }
    }
    let rounds_before: u32 = screen.pdcs.iter().map(|pd| pd.ammo_remaining).sum();
    let mut bubble = duel(shooter, screen);

    let mut outcome = Outcome::default();
    let mut ic = 1_000;
    for _ in 0..1_200 {
        let ready = bubble.ships.get("shooter").is_some_and(|ship| {
            ship.torpedo_tubes
                .first()
                .is_some_and(|tube| tube.cooldown_remaining_ms == 0 && tube.magazine_remaining > 0)
        });
        let screen_up = bubble
            .ships
            .get("screen")
            .is_some_and(|ship| ship.status == HullStatus::Operational);
        if ready && screen_up {
            for event in bubble.fire_torpedo_salvo("shooter", "screen", 2) {
                if matches!(event, SpaceCombatEvent::TorpedoLaunched { .. }) {
                    outcome.fired += 1;
                }
            }
        }
        let magazine_left = bubble
            .ships
            .get("shooter")
            .and_then(|ship| ship.torpedo_tubes.first())
            .map_or(0, |tube| tube.magazine_remaining);
        if (magazine_left == 0 || !screen_up) && bubble.torpedoes.is_empty() {
            break;
        }
        ic += 1;
        let report = bubble.step_wall_ms_at(MAX_SUBSTEP_MS, ic);
        for event in report.events {
            match event {
                SpaceCombatEvent::TorpedoHit { .. } => outcome.landed += 1,
                SpaceCombatEvent::TorpedoInterceptedByLaser { .. } => outcome.by_laser += 1,
                SpaceCombatEvent::TorpedoInterceptedByPdc { .. } => outcome.by_rotary += 1,
                SpaceCombatEvent::ShipDisabledDerelict { ship_id, .. }
                    if ship_id == "screen" && outcome.disabled_at.is_none() =>
                {
                    outcome.disabled_at = Some(outcome.landed);
                }
                _ => {}
            }
        }
    }
    outcome.lost = outcome
        .fired
        .saturating_sub(outcome.landed + outcome.by_laser + outcome.by_rotary);
    let rounds_after: u32 = bubble
        .ships
        .get("screen")
        .map_or(0, |ship| ship.pdcs.iter().map(|pd| pd.ammo_remaining).sum());
    outcome.rotary_rounds = rounds_before.saturating_sub(rounds_after);
    outcome
}

/// What a corvette's spinal magazine did to one Dray.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SpinalOutcome {
    pub fired: u32,
    pub struck: u32,
    pub dodged: u32,
    /// The change of speed her dodging cost her, m/s.
    pub dv_spent_mps: i64,
}

/// **A corvette's whole spinal magazine at a Dray `range_m` off**, a round at each ready
/// moment. With `dodge` she burns across the line of fire while any round is in flight at
/// her, from the moment it leaves, and otherwise coasts.
pub(super) fn measure_spinal(range_m: i64, dodge: bool) -> SpinalOutcome {
    let shooter = stock_hull(
        "tacsim.unit.corvette",
        "shooter",
        admitted_party(0),
        [0, 0, 0],
        [0, 0, 0],
    );
    let screen = stock_hull(
        "tacsim.unit.dray",
        "screen",
        admitted_party(1),
        [range_m, 0, 0],
        [0, 0, 0],
    );
    let dv_before = screen.delta_v_mps;
    let mut bubble = duel(shooter, screen);
    let mut outcome = SpinalOutcome::default();
    let mut ic = 1_000;
    for _ in 0..2_400 {
        let ready = bubble.ships.get("shooter").is_some_and(|ship| {
            ship.railguns
                .first()
                .is_some_and(|gun| gun.cooldown_remaining_ms == 0 && gun.rounds_remaining > 0)
        });
        if ready {
            for event in bubble.fire_railgun("shooter", "screen") {
                if matches!(event, SpaceCombatEvent::RailgunFired { .. }) {
                    outcome.fired += 1;
                }
            }
        }
        let in_flight = bubble
            .railgun_slugs
            .iter()
            .any(|slug| slug.target_id == "screen");
        if dodge {
            let across = bubble
                .ships
                .get("screen")
                .map_or(0, |ship| ship.max_accel_mmps2);
            let thrust = if in_flight { [0, across, 0] } else { [0, 0, 0] };
            let _ = bubble.command_burn_mmps2("screen", thrust, true);
        }
        let rounds_left = bubble
            .ships
            .get("shooter")
            .and_then(|ship| ship.railguns.first())
            .map_or(0, |gun| gun.rounds_remaining);
        if rounds_left == 0 && !in_flight {
            break;
        }
        ic += 1;
        for event in bubble.step_wall_ms_at(MAX_SUBSTEP_MS, ic).events {
            match event {
                SpaceCombatEvent::RailgunHit { .. } => outcome.struck += 1,
                SpaceCombatEvent::RailgunDodgedByJink { .. } => outcome.dodged += 1,
                _ => {}
            }
        }
    }
    outcome.dv_spent_mps = dv_before
        - bubble
            .ships
            .get("screen")
            .map_or(dv_before, |ship| ship.delta_v_mps);
    outcome
}

/// The stock screens, in the order the cut note reports them.
pub(super) const SCREENS: [&str; 6] = [
    "tacsim.unit.dray",
    "tacsim.unit.dray_p",
    "tacsim.unit.corvette",
    "tacsim.unit.corvette_interdictor",
    "tacsim.unit.raider_heavy",
    "tacsim.unit.halcyon",
];

/// The haulers among [`SCREENS`]; the rest are warships.
const HAULERS: [&str; 2] = ["tacsim.unit.dray", "tacsim.unit.dray_p"];

/// A laser of the shipped row `key`, both modes allowed, in offence.
fn laser_of(key: &str) -> crate::space::weapons::LaserMount {
    let row = custody::space_mount_row(key).expect("a shipped row");
    let crate::definitions::space_mount::SpaceMountFamily::Laser(def) = &row.family else {
        panic!("{key} is a laser");
    };
    crate::space::weapons::LaserMount::from_def(
        "laser_1",
        def,
        crate::space::weapons::LaserModes::BOTH,
        LaserMode::Offense,
    )
}

/// **The palette** (doc 49 §5; Q-49-19, Q-49-10): on every shipped fit, torpedo tubes reload
/// in 8 s, a spinal gun cycles in 6 s, an offence laser at the 5 s floor, and a point-defence
/// rotary every 0.5 s on its own; a laser in defence keeps its own automatic cycle. The
/// corvette family's spinal guns stow 24 rounds.
#[test]
fn cc8_every_shipped_fit_flies_the_ruled_palette() {
    for key in SCREENS {
        let ship = stock_hull(key, "her", admitted_party(0), [0; 3], [0; 3]);
        for tube in &ship.torpedo_tubes {
            assert_eq!(tube.reload_cooldown_ms, 8_000, "{key} {}", tube.id);
        }
        for gun in &ship.railguns {
            assert_eq!(gun.cooldown_ms, 6_000, "{key} {}", gun.id);
            let stowed = if key == "tacsim.unit.halcyon" { 32 } else { 24 };
            assert_eq!(gun.magazine_max(), stowed, "{key} {}", gun.id);
        }
        for laser in &ship.lasers {
            assert_eq!(
                laser.cycle_ms(LaserMode::Offense),
                5_000,
                "{key} {}",
                laser.id
            );
            assert_eq!(
                laser.cycle_ms(LaserMode::Defense),
                4_000,
                "{key} {}",
                laser.id
            );
            assert!(
                laser.allowed_modes.is_both(),
                "{key} {}: dual-mode",
                laser.id
            );
        }
        for pd in &ship.pdcs {
            assert_eq!(pd.cooldown_ms, 500, "{key} {}", pd.id);
        }
    }
    // Q-49-25: the haulers start their lasers in offence, to answer fire; a warship of the
    // corvette's hull starts one in defence, her screen.
    for key in HAULERS {
        let ship = stock_hull(key, "her", admitted_party(0), [0; 3], [0; 3]);
        assert!(
            ship.lasers
                .iter()
                .all(|laser| laser.mode == LaserMode::Offense),
            "{key}"
        );
    }
    for key in [
        "tacsim.unit.corvette",
        "tacsim.unit.corvette_interdictor",
        "tacsim.unit.raider_heavy",
    ] {
        let ship = stock_hull(key, "her", admitted_party(0), [0; 3], [0; 3]);
        let modes: Vec<LaserMode> = ship.lasers.iter().map(|laser| laser.mode).collect();
        assert_eq!(modes, [LaserMode::Offense, LaserMode::Defense], "{key}");
    }
}

/// **An offence laser is short and weak against a hull** (Q-49-7): it reaches less far than
/// the spinal gun, does under half its close-range damage past 25 km and a point at the
/// raider's ambush range, and nothing past its reach; a Dray's bow takes many pulses where
/// one torpedo or spinal round does as much.
#[test]
fn cc8_an_offence_laser_is_short_and_weak_against_a_hull() {
    let spinal = custody::space_mount_row("tacsim.space_mount.spinal_gun").expect("the row");
    let crate::definitions::space_mount::SpaceMountFamily::Spinal(spinal) = &spinal.family else {
        panic!("the spinal gun");
    };
    for key in [
        "tacsim.space_mount.turret_laser",
        "tacsim.space_mount.turret_laser_short",
    ] {
        let laser = laser_of(key);
        assert!(laser.max_range_m < spinal.max_range_m, "{key}");
        let close = laser.damage_at_range(0);
        assert!(laser.damage_at_range(26_000) * 2 < close, "{key}");
        assert_eq!(laser.damage_at_range(laser.max_range_m + 1), 0, "{key}");
        assert!(
            close * 3 < spinal.round_damage,
            "{key}: weak beside a spinal round"
        );
    }
    let turret = laser_of("tacsim.space_mount.turret_laser");
    assert!(turret.damage_at_range(crate::space::picket::RAIDER_AMBUSH_RANGE_M) <= 2);
    assert!(turret.damage_at_range(crate::space::picket::RAIDER_AMBUSH_RANGE_M) >= 1);
}

/// A Dray alone in a bubble, her first laser as her fit lays it.
fn dray_bubble() -> LocalSpaceBubble {
    let mut bubble = LocalSpaceBubble::new("bubble.cc8.modes", "site.test", "CC.8", 1_000);
    bubble.insert_ship(stock_hull(
        "tacsim.unit.dray",
        "her",
        admitted_party(0),
        [0; 3],
        [0; 3],
    ));
    bubble
}

fn first_laser(bubble: &LocalSpaceBubble) -> &crate::space::weapons::LaserMount {
    bubble
        .ships
        .get("her")
        .and_then(|ship| ship.lasers.first())
        .expect("her first laser")
}

/// **Q-49-24: the cycle belongs to the mode, and a switch never fires early.** A laser
/// switched to defence waits a whole defence cycle from the switch before it stops a torpedo;
/// a laser switched to offence with no offence cycle running waits a whole offence cycle.
#[test]
fn cc8_a_switch_starts_the_new_modes_cycle_and_never_fires_early() {
    let mut bubble = dray_bubble();
    assert_eq!(
        first_laser(&bubble).cooldown_remaining_ms,
        0,
        "ready in offence"
    );
    assert!(bubble.set_laser_mode("her", "laser_dorsal", LaserMode::Defense));
    assert_eq!(
        first_laser(&bubble).cooldown_remaining_ms,
        4_000,
        "the defence cycle starts"
    );
    let _ = bubble.step_wall_ms_at(3_750, 1_001);
    assert_eq!(first_laser(&bubble).cooldown_remaining_ms, 250);
    let _ = bubble.step_wall_ms_at(250, 1_002);
    assert_eq!(
        first_laser(&bubble).cooldown_remaining_ms,
        0,
        "ready in defence"
    );
    // Back to offence, long after her last offence pulse: a whole offence cycle.
    assert!(bubble.set_laser_mode("her", "laser_dorsal", LaserMode::Offense));
    assert_eq!(first_laser(&bubble).cooldown_remaining_ms, 5_000);
    // A switch to the mode she is in changes nothing.
    assert!(bubble.set_laser_mode("her", "laser_dorsal", LaserMode::Offense));
    assert_eq!(first_laser(&bubble).cooldown_remaining_ms, 5_000);
}

/// **Q-49-24: switching back within one offence cycle keeps the offence cooldown**, so a
/// switch is no way round the 5 s floor: the offence cycle runs on behind the switch to
/// defence, and the switch back keeps what is left of it, neither more nor less.
#[test]
fn cc8_switching_back_within_one_offence_cycle_keeps_the_offence_cooldown() {
    let mut bubble = dray_bubble();
    let mut target = stock_hull(
        "tacsim.unit.corvette",
        "target",
        admitted_party(1),
        [10_000, 0, 0],
        [0; 3],
    );
    target.pdc_auto = false;
    bubble.insert_ship(target);
    bubble.set_party_relation(admitted_party(0), admitted_party(1), Some(Mark::Hostile));
    let hits = |events: &[SpaceCombatEvent]| {
        events
            .iter()
            .filter(|event| matches!(event, SpaceCombatEvent::LaserHit { .. }))
            .count()
    };
    assert_eq!(hits(&bubble.fire_offensive_lasers("her", "target")), 2);
    assert_eq!(
        first_laser(&bubble).cooldown_remaining_ms,
        5_000,
        "the 5 s floor"
    );
    let _ = bubble.step_wall_ms_at(1_000, 1_001);
    assert!(bubble.set_laser_mode("her", "laser_dorsal", LaserMode::Defense));
    assert_eq!(first_laser(&bubble).offence_left_ms, 4_000);
    let _ = bubble.step_wall_ms_at(1_500, 1_002);
    assert!(bubble.set_laser_mode("her", "laser_dorsal", LaserMode::Offense));
    assert_eq!(
        first_laser(&bubble).cooldown_remaining_ms,
        2_500,
        "what was left of her offence cycle, not a fresh one and not none"
    );
    // The other laser never left offence: it is on the same footing.
    let _ = bubble.step_wall_ms_at(2_250, 1_003);
    assert_eq!(
        hits(&bubble.fire_offensive_lasers("her", "target")),
        0,
        "not early"
    );
    let _ = bubble.step_wall_ms_at(250, 1_004);
    assert_eq!(
        hits(&bubble.fire_offensive_lasers("her", "target")),
        2,
        "both again at 5 s"
    );
}

/// **Q-49-24 and Q-49-10: each mode fires on its own cycle.** An offence pulse starts the
/// 5 s offence cycle; a torpedo stopped in defence starts the defence mode's own automatic
/// cycle.
#[test]
fn cc8_each_mode_fires_on_its_own_cycle() {
    let shooter = stock_hull(
        "tacsim.unit.corvette",
        "shooter",
        admitted_party(0),
        [0; 3],
        [0; 3],
    );
    let mut screen = stock_hull(
        "tacsim.unit.corvette",
        "screen",
        admitted_party(1),
        [60_000, 0, 0],
        [0; 3],
    );
    screen.pdc_auto = false;
    let mut bubble = duel(shooter, screen);
    assert_eq!(bubble.fire_torpedo_salvo("shooter", "screen", 1).len(), 1);
    let mut stopped = false;
    for ic in 0..400 {
        let report = bubble.step_wall_ms_at(MAX_SUBSTEP_MS, 1_001 + ic);
        if report
            .events
            .iter()
            .any(|event| matches!(event, SpaceCombatEvent::TorpedoInterceptedByLaser { .. }))
        {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "her laser in defence stops the torpedo");
    let screen = bubble.ships.get("screen").expect("the screen");
    let defence = screen
        .lasers
        .iter()
        .find(|laser| laser.mode == LaserMode::Defense)
        .expect("her laser in defence");
    assert_eq!(defence.cooldown_remaining_ms, 4_000, "the defence cycle");
    let offence = screen
        .lasers
        .iter()
        .find(|laser| laser.mode == LaserMode::Offense)
        .expect("her laser in offence");
    assert_eq!(offence.cooldown_remaining_ms, 0, "untouched");
}

/// **The screen balance** (Q-49-26): a lone corvette's full magazine of torpedoes, a salvo of
/// two at each reload, is stopped whole by every warship's screen as her fit lays it, and lets
/// one round in two through a hauler's at range. A hauler that puts both lasers in defence
/// stops it too, and has then nothing to answer with.
#[test]
fn cc8_a_warships_screen_stops_a_lone_salvo_and_a_haulers_leaks() {
    for (geometry, range_m, closing) in [
        ("150 km, still", 150_000, 0),
        ("300 km, still", 300_000, 0),
        ("200 km, closing", 200_000, 6_000_000),
    ] {
        for key in SCREENS {
            let fitted = measure(
                "tacsim.unit.corvette",
                key,
                Posture::AsFitted,
                range_m,
                closing,
                Knobs::default(),
            );
            assert_eq!(fitted.fired, 8, "{geometry} {key}: {fitted:?}");
            if HAULERS.contains(&key) {
                assert_eq!(fitted.landed, 4, "{geometry} {key}: one in two: {fitted:?}");
                let braced = measure(
                    "tacsim.unit.corvette",
                    key,
                    Posture::Braced,
                    range_m,
                    closing,
                    Knobs::default(),
                );
                assert_eq!(braced.landed, 0, "{geometry} {key} braced: {braced:?}");
            } else {
                assert_eq!(fitted.landed, 0, "{geometry} {key}: {fitted:?}");
            }
        }
    }
}

/// **The spinal gun's dodge stays, priced in her change of speed** (Q-49-26): far enough out
/// that her drive can move her off the line in a round's flight, a Dray that burns across it
/// whenever a round is coming is never struck, and it costs her change of speed; a Dray that
/// coasts takes every round.
#[test]
fn cc8_the_spinal_guns_dodge_stays_and_costs_her_change_of_speed() {
    let coasting = measure_spinal(150_000, false);
    assert_eq!(coasting.fired, 24, "{coasting:?}");
    assert_eq!(coasting.struck, coasting.fired, "{coasting:?}");
    assert_eq!(coasting.dv_spent_mps, 0, "{coasting:?}");
    let dodging = measure_spinal(150_000, true);
    assert_eq!(dodging.struck, 0, "{dodging:?}");
    assert_eq!(dodging.dodged, dodging.fired, "{dodging:?}");
    assert!(dodging.dv_spent_mps > 0, "{dodging:?}");
}
