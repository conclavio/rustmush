#![cfg(test)]
//! **Doc 49 unit CC.8: the space palette, the cycle per mode and the screen balance**
//! (Q-49-7, Q-49-10, Q-49-19, Q-49-24, Q-49-25, Q-49-26).
//!
//! The measurement fires one shooter's full magazine at each stock hull, as her fit lays her
//! screen, and counts what lands. `cc8_measure_the_screens` prints the table the cut note
//! reports (`cargo test -p tacsim --lib cc8_measure -- --ignored --nocapture`).

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
    /// Every laser her fit allows in defence, set before the first salvo leaves.
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
    /// The torpedo, counted from one, that left her disabled; `None` if she held.
    pub disabled_at: Option<u32>,
}

/// Overrides for a sweep: each `None` keeps the shipped figure.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Knobs {
    pub defence_cycle_ms: Option<u32>,
    pub envelope_m: Option<i64>,
    pub damage_per_burst: Option<u16>,
}

/// **Fire `shooter_key`'s whole tube magazine at `screen_key`**, `range_m` off, the shooter
/// closing at `closing_mmps`, salvos at each ready moment, and step until every torpedo is
/// done.
pub(super) fn measure(
    shooter_key: &str,
    screen_key: &str,
    posture: Posture,
    range_m: i64,
    closing_mmps: i64,
    knobs: Knobs,
) -> Outcome {
    let mut bubble = LocalSpaceBubble::new("bubble.cc8", "site.test", "CC.8", 1_000);
    bubble.set_party_relation(admitted_party(0), admitted_party(1), Some(Mark::Hostile));
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
    bubble.insert_ship(shooter);
    bubble.insert_ship(screen);

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

/// The stock screens, in the order the cut note reports them.
pub(super) const SCREENS: [&str; 6] = [
    "tacsim.unit.dray",
    "tacsim.unit.dray_p",
    "tacsim.unit.corvette",
    "tacsim.unit.corvette_interdictor",
    "tacsim.unit.raider_heavy",
    "tacsim.unit.halcyon",
];

/// The geometries: standing off at a range, or closing.
pub(super) const GEOMETRIES: [(&str, i64, i64); 5] = [
    ("60 km, still", 60_000, 0),
    ("150 km, still", 150_000, 0),
    ("300 km, still", 300_000, 0),
    ("200 km, closing 6 km/s", 200_000, 6_000_000),
    ("120 km, closing 20 km/s", 120_000, 20_000_000),
];

fn row(label: &str, outcome: &Outcome) -> String {
    format!(
        "{label:<56} fired {:>2} landed {:>2} laser {:>2} rotary {:>2} lost {:>2} rounds {:>5} disabled {}",
        outcome.fired,
        outcome.landed,
        outcome.by_laser,
        outcome.by_rotary,
        outcome.lost,
        outcome.rotary_rounds,
        outcome
            .disabled_at
            .map_or_else(|| "-".to_owned(), |n| format!("at {n}")),
    )
}

/// **The measurement** (Q-49-26): a lone corvette's full magazine against each stock screen,
/// as fitted and braced, at each geometry. With `CC8_SWEEP` set it also sweeps the defence
/// cycle and the rotary's envelope and burst against the shipped figures.
#[test]
#[ignore = "a measurement: prints the table the cut note reports"]
fn cc8_measure_the_screens() {
    let sweep = std::env::var_os("CC8_SWEEP").is_some();
    let mut knob_sets = vec![("shipped", Knobs::default())];
    if sweep {
        for cycle in [1_000, 2_000, 3_000, 4_000, 6_000, 8_000] {
            for envelope in [15_000, 10_000, 6_000, 4_000] {
                for damage in [50, 34, 25] {
                    knob_sets.push((
                        "sweep",
                        Knobs {
                            defence_cycle_ms: Some(cycle),
                            envelope_m: Some(envelope),
                            damage_per_burst: Some(damage),
                        },
                    ));
                }
            }
        }
    }
    for (name, knobs) in knob_sets {
        println!("== {name} {knobs:?}");
        for (geometry, range_m, closing) in GEOMETRIES {
            for screen in SCREENS {
                for posture in [Posture::AsFitted, Posture::Braced] {
                    let outcome =
                        measure("tacsim.unit.corvette", screen, posture, range_m, closing, knobs);
                    let label = format!(
                        "{geometry} | {} {posture:?}",
                        screen.trim_start_matches("tacsim.unit.")
                    );
                    println!("{}", row(&label, &outcome));
                }
            }
        }
    }
}
