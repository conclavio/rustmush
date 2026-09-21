//! **The machines on the map the owner plays on** (16b cut E.1; doc 16 §6,
//! §3.1, §3.5; 16c `B-05`, `T-32`).
//!
//! Every other reflex test in this crate runs on a sixteen-cell window with a
//! two-cell carbine, which is the right shape for an edge and says nothing about
//! the fight. This file runs the shipped arena — 256 cells of 10 m authored
//! ground, a 600 m sight, a 3 km cannon with a 1.5 km inner band — with the
//! production roster's own stock tanks, composed by `tests/support`'s rig out of
//! `match_::custody_unit` and posed by `match_::opening_pose`, so what it
//! measures is what a pilot meets.
//!
//! 16b cut E.3 and E.4 add to it.

mod support;

use support::{Muster, booted_world, ledger_events, run_until, shipped_arena};
use tacsim::arena::clock::Tempo;
use tacsim::arena::reflex::ReflexState;
use tacsim::arena::state::{Arena, Team};

/// One cell of the shipped window, centimetres.
const CELL_CM: i64 = 1_000;

/// **The stock tank's `Ranges`, in centimetres**, written here because a
/// shipped-scale assertion has to name the number it is asserting.
///
/// `crates/tacsim/tests/content.rs`'s `e1_the_ranges_the_shipped_machines_fight_at`
/// is what keeps these the content's: the sight is 60 cells of 1,000 cm and the
/// cannon's inner band is 1.5 km, so the platform's fighting reach is 60,000 cm
/// and doctrine's 700 per mille of it is 42,000.
const ENGAGE_CM: i64 = 42_000;
/// The firing ceiling, and where `Engage` gives the band up.
const RELEASE_CM: i64 = 60_000;

/// **One pulse's reading of one machine** — the shape claim (b) is asserted on.
///
/// A shipped-scale claim about *the first shot* is a claim about one arena
/// millisecond, and the values it needs (the state, the ordered speed, the range
/// to the plot) are per tick and gone by the end of the run. They are collected
/// as the run goes and read back at the shot's own pulse rather than being
/// reconstructed from the field's last frame.
#[derive(Clone, Copy, Debug)]
struct Seen {
    unit: mush_core::ObjId,
    at_ms: i64,
    state: ReflexState,
    /// What the hull is doing.
    speed_cms: i32,
    /// What the controller told it to do, which is the one the first shot is
    /// asserted on: the `Approach → Engage` edge and `engage_orders` are one
    /// tick, so the shot leaves while the tank is still rolling to a stop.
    desired_speed_cms: i32,
    /// The range to its target's plot, or `None` when it holds none.
    plot_cm: Option<i64>,
}

/// The range between two points on the window, centimetres.
fn range_cm(a: (i64, i64), b: (i64, i64)) -> i64 {
    ((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)).isqrt()
}

/// A cell's centre, centimetres.
fn centre_of(cell: (u16, u16)) -> (i64, i64) {
    (
        i64::from(cell.0) * CELL_CM + CELL_CM / 2,
        i64::from(cell.1) * CELL_CM + CELL_CM / 2,
    )
}

/// **Two stock machines on the proving ground close to their real reach and
/// fight there** (16b cut E.1; 16c `B-05`, `T-47`, `T-32`).
///
/// Red Tank 1 and Blue Tank 1, at their authored musters on the ford road,
/// both on the reflex controller, shipped doctrine, ten minutes of arena time at
/// a one-second cadence. Four claims, three of them sampled at every pulse:
///
/// * **(a) `B-05`.** Whenever a machine is in `Approach` with a goal and holds
///   its target's plot, the goal is **not further from the plot than the machine
///   already is**. The form asserted is
///   `range(goal centre, plot) ≤ max(range(own pose, plot), engage_cm)`, and the
///   slack is what makes it true of a value a test can read: the goal was
///   computed at the machine's decide tick and is read at the pulse's end, up to
///   a second later, by when the machine has closed up to 15 m and a sensor pass
///   may have moved the plot as far. At HEAD the goal was 700 per mille of the
///   platform's 3 km gun projected back from the plot — up to 2.1 km, on a map
///   where nothing is ever held past 900 m — so it failed this by two orders of
///   magnitude.
/// * **(b)** the first shot is fired from `Engage`, inside 600 m, with the
///   machine **ordered** to a stop — three readings of the pulse that carries it,
///   and they are the cut's headline measure: a machine that fires from
///   `Approach`, or from a kilometre out, or while it is still ordered to drive,
///   is the defect this cut exists to close. Its *actual* speed is deliberately
///   not asserted there: `Approach → Engage` and `engage_orders` are one tick,
///   `resolve_fire` has no speed gate, and the stock tank sheds 15 m/s at 2 m/s²
///   over seven and a half seconds — so at E.1 the first shot is fired at close
///   to full speed, which is what a crew with a line does. What is asserted
///   instead is that the machine really does come to a stop: if it is still in
///   `Engage` ten arena seconds later, it has read zero on at least one pulse in
///   between. 16b cut E.3 adds the braking profile and tightens this.
/// * **(c)** no machine is ever in `Engage` at a plot range past `release_cm`
///   plus one tick of a target opening the range, plus a cell.
/// * **(d) `T-32`**: the match spends time in more than one tempo, which it
///   could not before the `Engaged` trigger was re-based on the band a platform
///   fights in.
#[test]
fn e1_two_stock_machines_close_to_their_real_reach_and_fight() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let opened = std::time::Instant::now();

    // The pulse-by-pulse lens. `skip_next` is review E-03's exclusion: `follow`'s
    // `Err` arm legitimately replaces the goal with a waypoint node when a search
    // fails, and the `Approach` arm re-asserts its own goal on the next tick — so
    // a pulse on which a search failed, and the pulse after it, are counted and
    // skipped rather than absorbed into slack.
    let mut approach_pulses = 0_u32;
    let mut skipped = 0_u32;
    let mut skip_next = false;
    let mut worst_engage_cm = 0_i64;
    let mut engage_pulses = 0_u32;
    // Every pulse's reading of every machine, which is what (b) is asserted on:
    // the first shot is one arena millisecond and the claim about it is about the
    // pulse that carried it, not about anything that happened later in the match.
    let mut seen: Vec<Seen> = Vec::new();

    let mut bytes = run_until(&mut arena, 600_000, 1_000, |arena, out| {
        let failed = out.reflex.path_failures > 0;
        let skipping = skip_next || failed;
        skip_next = failed;
        for (unit, row) in arena.reflex.all() {
            let Some(state) = arena.unit(unit) else {
                continue;
            };
            let here = (state.pose.x_cm, state.pose.y_cm);
            let plot = row
                .target
                .and_then(|(contact, _)| arena.contacts.resolve(unit, contact))
                .map(|held| (held.seen_at.x_cm, held.seen_at.y_cm));
            seen.push(Seen {
                unit,
                at_ms: arena.now_ms,
                state: row.state,
                speed_cms: state.speed_cms,
                desired_speed_cms: state.desired_speed_cms,
                plot_cm: plot.map(|plot| range_cm(here, plot)),
            });
            match row.state {
                ReflexState::Approach => {
                    let (Some(goal), Some(plot)) = (row.goal, plot) else {
                        continue;
                    };
                    approach_pulses = approach_pulses.saturating_add(1);
                    if skipping {
                        skipped = skipped.saturating_add(1);
                        continue;
                    }
                    let own = range_cm(here, plot);
                    let want = range_cm(centre_of(goal), plot);
                    assert!(
                        want <= own.max(ENGAGE_CM),
                        "`B-05`: at {} ms #{} is {own} cm from its plot and its goal is {want} cm \
                         from it, so the goal is behind the machine",
                        arena.now_ms,
                        unit.0
                    );
                }
                ReflexState::Engage => {
                    engage_pulses = engage_pulses.saturating_add(1);
                    if let Some(plot) = plot {
                        worst_engage_cm = worst_engage_cm.max(range_cm(here, plot));
                    }
                }
                _ => {}
            }
        }
    });
    let wall = opened.elapsed();

    // (a)'s bookkeeping: the exclusion is an exclusion and not a loophole.
    println!(
        "e1_shipped_duel: ended {:?} at {} ms; {approach_pulses} approach pulses \
         ({skipped} skipped for a failed search), {engage_pulses} engage pulses",
        arena.ended.as_ref().map(|end| end.reason),
        arena.now_ms
    );
    assert!(
        approach_pulses > 0,
        "nothing ever approached, so `B-05` was never asked"
    );
    assert!(
        u64::from(skipped).saturating_mul(100) < u64::from(approach_pulses),
        "{skipped} of {approach_pulses} approach pulses were skipped for a failed search, which \
         is more than one in a hundred: the path budget, and not the goal, is what this run \
         measured"
    );

    // (b) The first shot: from `Engage`, inside 600 m, ordered to a stop.
    let log = ledger_events(&mut arena, &mut bytes);
    let first = log
        .iter()
        .find(|event| event.kind.as_str() == "fired")
        .expect("two machines on the proving ground shot at each other");
    let shooter = support::unit_of_event(first).expect("a `fired` line names its shooter");
    // The pulse that carried it: the first reading of that machine at or after
    // the shot's own arena millisecond, because a pulse's hook runs when the
    // pulse ends.
    let at_the_shot = seen
        .iter()
        .filter(|row| row.unit == shooter && row.at_ms >= first.t_ms)
        .min_by_key(|row| row.at_ms)
        .unwrap_or_else(|| panic!("the pulse that carried the shot at {} ms", first.t_ms));
    println!(
        "e1_shipped_duel: first shot at {} ms by #{}, read at {} ms as {:?} at {:?} cm, ordered \
         {} cm/s, rolling {} cm/s",
        first.t_ms,
        shooter.0,
        at_the_shot.at_ms,
        at_the_shot.state,
        at_the_shot.plot_cm,
        at_the_shot.desired_speed_cms,
        at_the_shot.speed_cms
    );
    assert_eq!(
        at_the_shot.state,
        ReflexState::Engage,
        "the first shot was not fired from `Engage`: {at_the_shot:?}"
    );
    assert!(
        at_the_shot.plot_cm.is_some_and(|range| range <= RELEASE_CM),
        "the first shot was fired from outside the platform's own firing ceiling of \
         {RELEASE_CM} cm: {at_the_shot:?}"
    );
    assert_eq!(
        at_the_shot.desired_speed_cms, 0,
        "the machine was still ordered to drive when it fired: {at_the_shot:?}"
    );
    // And it really does come to a stop: if it is still engaged ten arena seconds
    // later, it read zero somewhere in between. The window is scoped to the shot
    // rather than to the match, so a stop the machine made half a minute earlier
    // is not what satisfies it.
    let until_ms = first.t_ms + 10_000;
    let still_engaged = seen
        .iter()
        .filter(|row| row.unit == shooter && row.at_ms >= until_ms)
        .min_by_key(|row| row.at_ms)
        .is_some_and(|row| row.state == ReflexState::Engage);
    if still_engaged {
        let stopped: Vec<i64> = seen
            .iter()
            .filter(|row| {
                row.unit == shooter
                    && (first.t_ms..=until_ms).contains(&row.at_ms)
                    && row.speed_cms == 0
            })
            .map(|row| row.at_ms)
            .collect();
        assert!(
            !stopped.is_empty(),
            "ten seconds after its first shot the machine is still engaged and has never read \
             zero: it is fighting rolling"
        );
        println!("e1_shipped_duel: the shooter read zero at {stopped:?}");
    }

    // (c) `Engage` is never held past the firing ceiling.
    println!("e1_shipped_duel: widest `Engage` plot range {worst_engage_cm} cm");
    assert!(
        worst_engage_cm > 0,
        "nothing ever engaged, so the ceiling was never asked"
    );
    assert!(
        worst_engage_cm <= RELEASE_CM + 3 * CELL_CM,
        "a machine held the band at {worst_engage_cm} cm, past its own {RELEASE_CM} cm ceiling \
         plus a tick of the target opening the range and a cell"
    );

    // (d) `T-32`: the time in each tempo, off the `tempo` ledger lines.
    let (quiet, alert, engaged) = time_in_tempo(&log, arena.now_ms);
    println!(
        "e1_shipped_duel: quiet {quiet} ms, alert {alert} ms, engaged {engaged} ms, wall \
         {:?} in the debug profile",
        wall
    );
    assert!(
        quiet > 0,
        "the match was never quiet: {quiet}/{alert}/{engaged}"
    );
    assert!(
        engaged > 0,
        "the match never reached `Engaged`: {quiet}/{alert}/{engaged}"
    );
}

/// The arena milliseconds spent in each tempo, from the `tempo` ledger lines and
/// the match's own end.
fn time_in_tempo(log: &[tacsim::arena::ledger::LedgerEvent], end_ms: i64) -> (i64, i64, i64) {
    let mut spans: Vec<(i64, Tempo)> = vec![(0, Tempo::Quiet)];
    for event in log {
        if event.kind.as_str() != "tempo" {
            continue;
        }
        let Ok(value) = serde_json::to_value(&event.kind) else {
            continue;
        };
        let word = value.get("to").and_then(serde_json::Value::as_str);
        let tempo = match word {
            Some("quiet") => Tempo::Quiet,
            Some("alert") => Tempo::Alert,
            Some("engaged") => Tempo::Engaged,
            _ => continue,
        };
        spans.push((event.t_ms, tempo));
    }
    let (mut quiet, mut alert, mut engaged) = (0_i64, 0_i64, 0_i64);
    for (at, span) in spans.iter().enumerate() {
        let until = spans
            .get(at + 1)
            .map_or(end_ms, |(next, _)| *next)
            .max(span.0);
        let held = until.saturating_sub(span.0);
        match span.1 {
            Tempo::Quiet => quiet += held,
            Tempo::Alert => alert += held,
            Tempo::Engaged => engaged += held,
        }
    }
    (quiet, alert, engaged)
}

/// The rig itself, checked once: the poses production would have given these two
/// tanks are the poses the rig gives them (16b §0.4).
#[test]
fn e1_the_rig_poses_its_units_the_way_the_check_out_does() {
    let world = booted_world();
    let arena: Arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1), Muster::reflex(Team::Red, 2)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    assert_eq!(arena.units.len(), 3);
    // Every unit opened on its own authored muster cell, which is what
    // `opening_pose` keeps when the world pose is legal and free.
    let mut cells: Vec<(u16, u16)> = arena
        .units
        .iter()
        .filter_map(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .collect();
    cells.sort_unstable();
    cells.dedup();
    assert_eq!(cells.len(), 3, "no two units share a cell");

    // And an override that lands on another unit's cell is relocated into the
    // side's entry zone rather than accepted, exactly as production relocates
    // one.
    //
    // **The cell has to be one an EARLIER unit of the same run has claimed**, or
    // the test asks nothing: `opening_pose` only refuses a world pose whose cell
    // is already in `taken`, and the rig fills `taken` side by side and unit by
    // unit. Red 1 is posed first, so its cell is the contended one — and the
    // assertion that has teeth is that Red 2 is *not* standing on it, not that
    // three units ended up on three cells (which they would if the override were
    // honoured and Red 1 were the one relocated).
    let red_one = support::tank(&world, Team::Red, 1);
    let red_two = support::tank(&world, Team::Red, 2);
    let taken = support::cell_of(&arena, red_one).expect("red 1 opened on the window");
    let crowded = shipped_arena(
        &world,
        &[
            &[
                Muster::reflex(Team::Red, 1),
                Muster::reflex(Team::Red, 2).at(taken),
            ],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let mut cells: Vec<(u16, u16)> = crowded
        .units
        .iter()
        .filter_map(|unit| crowded.definition.terrain.cell_of(&unit.pose))
        .collect();
    cells.sort_unstable();
    cells.dedup();
    assert_eq!(cells.len(), 3, "no two units share a cell");
    assert_eq!(
        support::cell_of(&crowded, red_one),
        Some(taken),
        "red 1 was posed first and keeps the cell it asked for"
    );
    let moved = support::cell_of(&crowded, red_two).expect("red 2 opened on the window");
    assert_ne!(
        moved, taken,
        "the override was honoured onto a cell another unit already held"
    );
    // And it was relocated the way production relocates one: into its own side's
    // entry zone, not to some third place of the rig's own.
    let zone = crowded
        .definition
        .entry_zones
        .iter()
        .find(|zone| zone.team == Team::Red)
        .expect("red has an entry zone");
    assert!(
        (zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&moved.0)
            && (zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&moved.1),
        "relocated to {moved:?}, which is not in red's entry zone"
    );

    // Every machine carries the sight and the guns the pack authors, which is the
    // whole reason the rig goes through `custody_unit`.
    for unit in &arena.units {
        assert_eq!(unit.sight_cm(1_000), 60_000, "#{}'s scope", unit.id.0);
        assert_eq!(unit.fight_inner_cm(1_000), 60_000);
        assert_eq!(unit.fight_limit_cm(1_000), 60_000);
    }
}
