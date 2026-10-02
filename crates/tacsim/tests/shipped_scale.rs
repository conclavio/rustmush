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

use support::{Muster, Timeline, booted_world, ledger_events, run_until, shipped_arena};
use tacsim::arena::clock::Tempo;
use tacsim::arena::reflex::ReflexState;
use tacsim::arena::state::{Arena, EndReason, Team};

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
///   is the defect this cut exists to close. `Approach → Engage` and
///   `engage_orders` are one tick and `resolve_fire` has no speed gate, so the
///   shot leaves while the tank is still rolling, which is what a crew with a
///   line does. **16b cut E.3 tightened the three claims about that roll**: with
///   `B-42`'s braking profile the shot is fired *below the ceiling* (measured
///   950 cm/s of 1,500), the machine reads zero inside seven arena seconds of it
///   (measured 5,750 ms, which is 950 cm/s at the stock 200 cm/s² of brake), and
///   from that zero until it leaves `Engage` it never reads above zero again.
///   (E.3 measured 1,097 cm/s and 6,750 ms; **16b cut E.4's hunt** sends the two
///   machines at each other's entry zones at 90 s of arena time instead of
///   leaving them on their waypoint loops, so they meet on different ground and
///   the tick the edge is crossed on is a different one. The bounds are
///   unchanged and both readings still sit well inside them.) **16b cut AI.1**
///   (16c `B-84`) took the orbit out of the approach, so the duel now closes
///   head-on at 3,000 cm/s and the shot leaves at a measured 1,208 cm/s; by the
///   programme lead's ruling the first claim is asserted at its intent — below
///   the machine's own ceiling and slower than at its previous pulse — and not
///   at E.3's `1,200` literal, which the orbit's slow closure had made room for.
///   The seven-second horizon stands and is now opened at the pulse that
///   carried the shot, where every other reading is made: 1,208 cm/s at the
///   stock brake is six seconds, and the first zero is read 7,000 ms after that
///   pulse (7,406 ms after the shot's own millisecond, 406 ms inside it).
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
    // Each machine's own forward ceiling, off its decoded profile, read before
    // the run so a hull wrecked in it is still answered: (b)'s "below its
    // ceiling" names the profile's figure and not a literal.
    let ceilings: Vec<(mush_core::ObjId, i32)> = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                i32::try_from(unit.mobility.max_speed_cms).unwrap_or(i32::MAX),
            )
        })
        .collect();
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
    // **16b cut E.3 tightened the two claims E.1 left here; 16b cut AI.1 (16c
    // `B-84`) re-states the first at its intent, by the programme lead's
    // ruling of 2026-09-28.**
    //
    // The first shot is fired *braking*, not at full speed and not stopped. The
    // machine fires on the tick it crosses `engage_cm`, and that tick is a couple
    // of cells short of its stand-off cell, so `B-42`'s profile has had a tick or
    // two to bite and no more — the stock tank sheds two metres a second squared.
    // E.3 measured 1,097 cm/s of a 1,500 cm/s tank at the pulse that carried the
    // shot and pinned it as `<= 1_200`, "below the ceiling with room to spare".
    // **That room was the B-84 orbit's.** Before AI.1 the duel met on ground the
    // machines were circling route cells over, so the closure was slow and the
    // brake had four `Approach` ticks to work in. With the orbit gone both
    // machines drive the ford road straight at 1,500 cm/s each, a 3,000 cm/s
    // closure; they detect each other at about 500 m, and the `Engage` edge at
    // 420 m arrives about a second before the brake toward the stand-off cell
    // would have finished: **measured 1,208 cm/s**, ordered 0 and falling
    // (1,500, 1,458, 1,408 at the three `Approach` ticks). The steer orders
    // exactly what E.3's follower ordered on a straight line — the aim dead
    // ahead, no turn-speed cap in play, `braking_speed_cms` to the goal — so what
    // moved is the fight's geometry, not the brake.
    //
    // So "braking into its band" is asserted as what it means, at the pulse that
    // carried the shot: ordered to 0 (above), **below the machine's own ceiling**
    // (its decoded profile's `max_speed_cms`), and **decelerating** (slower than
    // at its previous pulse). A machine that fired at its ceiling, or that was
    // still gaining or holding speed, did not brake for its band at all.
    let ceiling = ceilings
        .iter()
        .find(|(unit, _)| *unit == shooter)
        .map(|(_, ceiling)| *ceiling)
        .unwrap_or_else(|| panic!("#{} was on the field at the opening", shooter.0));
    assert!(
        at_the_shot.speed_cms < ceiling,
        "the first shot was fired at {} cm/s, the machine's whole {ceiling} cm/s ceiling, which \
         is not a machine braking into its band: {at_the_shot:?}",
        at_the_shot.speed_cms
    );
    let before_the_shot = seen
        .iter()
        .filter(|row| row.unit == shooter && row.at_ms < at_the_shot.at_ms)
        .max_by_key(|row| row.at_ms)
        .unwrap_or_else(|| panic!("the pulse before the one that carried the shot"));
    println!(
        "e1_shipped_duel: the shooter rolled {} cm/s at {} ms and {} cm/s at the shot's pulse, \
         against its {ceiling} cm/s ceiling",
        before_the_shot.speed_cms, before_the_shot.at_ms, at_the_shot.speed_cms
    );
    assert!(
        at_the_shot.speed_cms < before_the_shot.speed_cms,
        "the first shot was fired at {} cm/s, no slower than the {} cm/s of the pulse before, \
         which is not a machine braking into its band: {at_the_shot:?}",
        at_the_shot.speed_cms,
        before_the_shot.speed_cms
    );
    // And it really does come to a stop, and then stays stopped for as long as it
    // holds the band. **The window is the stock tank's own physics and not a
    // round number**: E.3 measured 1,097 cm/s, which at 200 cm/s² is five and a
    // half seconds of braking, and the `speed 0` order was itself given at the
    // tick before the shot, so the measured shot-to-stop figure was 6,750 ms and
    // the horizon is seven seconds. The window is scoped to the shot rather than
    // to the match, so a stop the machine made half a minute earlier is not what
    // satisfies it.
    //
    // **16b cut AI.1 (16c `B-84`) anchors those seven seconds at the pulse that
    // carried the shot**, the reading every claim above is made at, and not at
    // the shot's own millisecond inside it: speeds are read at pulse ends, so a
    // window opened mid-pulse and closed on a pulse end is up to a pulse short
    // of the physics it names. With the orbit gone the shot's pulse reads the
    // accepted 1,208 cm/s (above), which is 6.04 s at 200 cm/s² — the tank
    // physically stops about 6,450 ms after the shot at 141,594 ms, inside
    // seven seconds, but 40 ms after the pulse end at 148,000 ms, so the first
    // zero is read at 149,000 ms: 7,406 ms after the shot and exactly 7,000 ms
    // after its pulse. The horizon itself does not move.
    let until_ms = at_the_shot.at_ms + 7_000;
    let after: Vec<&Seen> = seen
        .iter()
        .filter(|row| row.unit == shooter && row.at_ms > first.t_ms)
        .collect();
    let still_engaged = after
        .iter()
        .filter(|row| row.at_ms >= until_ms)
        .min_by_key(|row| row.at_ms)
        .is_some_and(|row| row.state == ReflexState::Engage);
    let first_zero = after
        .iter()
        .find(|row| row.speed_cms == 0)
        .map(|row| row.at_ms);
    println!(
        "e1_shipped_duel: the shooter first read zero {} ms after its first shot",
        first_zero.map_or(-1, |at| at - first.t_ms)
    );
    if still_engaged {
        let stopped: Vec<i64> = after
            .iter()
            .filter(|row| row.at_ms <= until_ms && row.speed_cms == 0)
            .map(|row| row.at_ms)
            .collect();
        assert!(
            !stopped.is_empty(),
            "seven seconds after the pulse that carried its first shot the machine is still \
             engaged and has never read zero: it is fighting rolling"
        );
        // From the first zero until it leaves `Engage` it never reads above zero:
        // a machine that holds its band holds it standing still.
        let from = stopped.first().copied().unwrap_or(until_ms);
        let rolling: Vec<(i64, i32)> = after
            .iter()
            .take_while(|row| row.state == ReflexState::Engage)
            .filter(|row| row.at_ms >= from && row.speed_cms > 0)
            .map(|row| (row.at_ms, row.speed_cms))
            .collect();
        assert!(
            rolling.is_empty(),
            "the machine started rolling again while it was still holding its band: {rolling:?}"
        );
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

/// **Two machines of one side cross the shipped ford in column and stop each
/// other nowhere** (16b cut E.3; 16c `B-46`).
///
/// The proving ground has one ford on the east-west road, which is where every
/// solo match crosses and the one place a column is forced nose to tail. Before
/// this cut the follower was stopped dead at the cells its leader had not
/// vacated, journalled a `blocked` line for each, and learned to avoid the cells
/// its own leader was standing in — on the one piece of ground the whole match
/// has to use.
///
/// Red slots 1 and 2 at their authored musters, both on the machine, no hostiles
/// at all, four minutes of arena time: whatever the two of them do, neither is
/// ever stopped by the other, and neither remembers a cell the other was in.
///
/// **16b cut E.4 made this bite harder without touching an assertion.** With
/// nothing hostile ever held, both machines now cross to Blue's entry zone at
/// `hunt_after_ms` — together, within a second of each other, which is the
/// deliberate un-staggered hunt — so the run really is a two-machine column on
/// the one piece of ground the match has to use, and the east-most reading moved
/// from 130,288 cm to 238,528 cm. `far_entry_cell`'s clamp keeps each of them on
/// its own row; the two claims below are what says the pair of rules works.
///
/// **16b cut AI.1 (16c `B-84`) moved what this test proves, again without
/// touching an assertion.** Before AI.1 the pass was partly the orbit's: the two
/// machines wandered off the road row as they circled their route cells. With
/// the orbit gone both drive road row 97 to the far zone and the clamp hands
/// them **one** cell, so the first there turns back into the second — four
/// `Occupied` stops between 231,750 and 234,750 ms. By the programme lead's
/// ruling the property stands and doctrine was corrected: `far_entry_cell`'s
/// fan keeps a hunter's cell `HUNT_FAN_ROWS` clear of every hunt cell its side
/// already holds, so the second machine is fanned off the first's row. This
/// test is that fan's integration proof on the shipped ground; the clamp alone
/// no longer passes it.
#[test]
fn e3_two_machines_of_one_side_cross_the_ford_without_stopping_each_other() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[&[Muster::reflex(Team::Red, 1), Muster::reflex(Team::Red, 2)]],
    );
    let one = support::tank(&world, Team::Red, 1);
    let two = support::tank(&world, Team::Red, 2);
    let opened = arena
        .units
        .iter()
        .map(|unit| unit.pose.x_cm)
        .collect::<Vec<i64>>();
    // Every cell either machine ever remembered being stopped at, and the extent
    // of the ground they covered. The remembered cells age out after
    // `REFLEX_AVOID_TTL_MS`, so this is the union over the run.
    let mut learned: Vec<(mush_core::ObjId, (u16, u16))> = Vec::new();
    let mut east_most = 0_i64;
    let mut bytes = run_until(&mut arena, 240_000, 1_000, |arena, _out| {
        for id in [one, two] {
            let Some(row) = arena.reflex.unit(id) else {
                continue;
            };
            for (cell, _) in &row.avoid {
                if !learned.contains(&(id, *cell)) {
                    learned.push((id, *cell));
                }
            }
        }
        east_most = east_most.max(
            arena
                .units
                .iter()
                .map(|unit| unit.pose.x_cm)
                .max()
                .unwrap_or(0),
        );
    });
    let log = ledger_events(&mut arena, &mut bytes);
    let stops: Vec<(i64, Option<mush_core::ObjId>)> = log
        .iter()
        .filter(|event| event.kind.as_str() == "blocked")
        .map(|event| (event.t_ms, support::unit_of_event(event)))
        .collect();
    println!(
        "e3_shipped_column: opened at {opened:?} cm, east-most {east_most} cm, \
         {} stops, remembered {learned:?}",
        stops.len()
    );
    assert!(
        stops.is_empty(),
        "the two machines stopped each other on the shipped ground: {stops:?}"
    );
    assert!(
        learned.is_empty(),
        "a machine learned to avoid a cell on the one road the match has to use: {learned:?}"
    );
    // And they really did drive: a run in which nothing moved would pass the two
    // assertions above and prove nothing.
    assert!(
        arena.units.iter().any(|unit| {
            opened
                .iter()
                .all(|from| (unit.pose.x_cm - *from).abs() > 10_000)
        }),
        "neither machine went anywhere, so the ford was never crossed"
    );
}

// ---------------------------------------------------------------------------
// The endgame (16b cut E.4; 16c `B-04`)
// ---------------------------------------------------------------------------

/// **The bound every endgame case is held to**, arena milliseconds.
///
/// Fifteen minutes. The live smoke of 2026-09-20 is what fixes it: with the
/// human dead, a 1 v 2 machine endgame went quiet for five minutes and had not
/// ended at 533 s of arena time, and a 1 v 1 took 7 m 42 s. The owner plays
/// solo, so when they die the match has to end **among the machines** and in
/// minutes; the only exits before this cut were `SIM ABORT`, which records no
/// result, and the cap (an hour then; fifteen minutes since 16b cut AP.2).
const ENDGAME_MS: i64 = 900_000;

/// Play a field out until it ends or the bound runs out, at a one-second
/// cadence, taking a [`Timeline`] reading every ten arena seconds.
fn play_to_the_end(
    arena: &mut Arena,
    mut watch: impl FnMut(&Arena),
) -> (Option<EndReason>, i64, Timeline, std::time::Duration) {
    let opened = std::time::Instant::now();
    let mut timeline = Timeline::new();
    run_until(arena, ENDGAME_MS, 1_000, |arena, out| {
        timeline.sample(arena, out.reflex.path_failures);
        watch(arena);
    });
    let wall = opened.elapsed();
    let reason = arena.ended.map(|end| end.reason);
    (reason, arena.now_ms, timeline, wall)
}

/// The one assertion all three cases make, with the diagnosis in the message
/// rather than the number alone.
fn ended_by_elimination(
    case: &str,
    reason: Option<EndReason>,
    at_ms: i64,
    timeline: &Timeline,
    wall: std::time::Duration,
) {
    println!("{case}: ended {reason:?} at {at_ms} ms, wall {wall:?} in the debug profile");
    assert!(
        matches!(reason, Some(EndReason::Elimination { winner: Some(_) })),
        "{case}: the match ended {reason:?} at {at_ms} ms of arena time, not by an elimination \
         with a winner inside {ENDGAME_MS} ms. What the machines were doing:\n{}",
        timeline.render()
    );
}

/// **A hurt machine against two healthy ones ends by elimination** (16b cut
/// E.4; 16c `B-04`) — the case the sub-cut exists for, and the live smoke's own
/// shape.
///
/// Red Tank 1 opens hurt to just under `withdraw_permille` (see
/// `support::hurt_to_withdraw`: every facet's armour at its floor plus one, no
/// internals touched, so every section is still `intact`), against Blue Tanks 1
/// and 2 at full health, all three at their authored musters, all three on the
/// machine, shipped doctrine, no pilots at all.
///
/// Two things are asserted before the bound, because a case whose hurt machine
/// never withdrew has not run: the health it opens at is under
/// `withdraw_permille` and at or above `stand_min_permille`, and the machine is
/// in `Withdraw` within five decide ticks of its power coming online.
#[test]
fn e4_a_hurt_machine_against_two_ends_by_elimination() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1).hurt()],
            &[Muster::reflex(Team::Blue, 1), Muster::reflex(Team::Blue, 2)],
        ],
    );
    let hurt = support::tank(&world, Team::Red, 1);

    let doctrine = arena.arena_cfg.reflex.clone();
    let health = arena.reflex_health_permille(hurt);
    println!(
        "e4_hurt_one_v_two: #{} opens at {health} per mille (withdraw under {}, stand at or above \
         {})",
        hurt.0, doctrine.withdraw_permille, doctrine.stand_min_permille
    );
    assert!(
        health < doctrine.withdraw_permille,
        "the hurt machine opens at {health} per mille, which is not under `withdraw_permille` of \
         {}: it would never withdraw and the case observes nothing",
        doctrine.withdraw_permille
    );
    assert!(
        health >= doctrine.stand_min_permille,
        "the hurt machine opens at {health} per mille, under `stand_min_permille` of {}: it could \
         never stand on its way home",
        doctrine.stand_min_permille
    );

    let mut online_ms: Option<i64> = None;
    let mut withdrew_ms: Option<i64> = None;
    let (reason, at_ms, timeline, wall) = play_to_the_end(&mut arena, |arena| {
        if online_ms.is_none()
            && arena
                .unit(hurt)
                .is_some_and(|unit| unit.power == tacsim::arena::state::Power::Online)
        {
            online_ms = Some(arena.now_ms);
        }
        if withdrew_ms.is_none()
            && arena
                .reflex
                .unit(hurt)
                .is_some_and(|row| row.state == ReflexState::Withdraw)
        {
            withdrew_ms = Some(arena.now_ms);
        }
    });
    let online = online_ms.unwrap_or_else(|| {
        panic!(
            "the hurt machine never brought its power up:\n{}",
            timeline.render()
        )
    });
    let withdrew = withdrew_ms.unwrap_or_else(|| {
        panic!(
            "the hurt machine never withdrew, so this case did not run:\n{}",
            timeline.render()
        )
    });
    let grace = 5 * i64::from(doctrine.decide_ms);
    println!("e4_hurt_one_v_two: online at {online} ms, withdrew at {withdrew} ms");
    assert!(
        withdrew <= online.saturating_add(grace),
        "the hurt machine came online at {online} ms and did not withdraw until {withdrew} ms, \
         which is more than five decide ticks:\n{}",
        timeline.render()
    );
    ended_by_elimination("e4_hurt_one_v_two", reason, at_ms, &timeline, wall);
}

/// **Two healthy machines, one against one, end by elimination** (16b cut E.4;
/// 16c `B-04`) — the live smoke's 7 m 42 s case, with the bound on it.
#[test]
fn e4_a_healthy_one_against_one_ends_by_elimination() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let (reason, at_ms, timeline, wall) = play_to_the_end(&mut arena, |_| {});
    ended_by_elimination("e4_healthy_one_v_one", reason, at_ms, &timeline, wall);
}

/// **A dry loser is found and finished** (16b cut E.4; 16c `B-04`).
///
/// Red Tank 1 opens with every magazine empty, so it is `is_dry` on its first
/// decide tick: it withdraws at once, it never rallies (a unit with nothing to
/// fire has no fight to come back to), and it parks in its own entry zone. Blue
/// Tank 1 has to go and find it — which before this cut nothing made it do,
/// because no patrol loop passes through an entry zone.
#[test]
fn e4_a_dry_loser_is_found_and_finished() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1).dry()],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let dry = support::tank(&world, Team::Red, 1);
    assert!(
        arena.unit(dry).is_some_and(|unit| unit.armed_at_checkout()
            && unit.weapons.iter().all(|w| unit.weapon_ammo(&w.id) == 0)),
        "the dry machine checked out with rounds, so this case does not run"
    );
    let (reason, at_ms, timeline, wall) = play_to_the_end(&mut arena, |_| {});
    ended_by_elimination("e4_dry_loser", reason, at_ms, &timeline, wall);
}

// ---------------------------------------------------------------------------
// The immobile machine (16b cut AA.1; 16c `B-76`)
// ---------------------------------------------------------------------------

/// **Blue's cell for the engine-out duel**: on the ford road, 50 cells due east
/// of Red Tank 1's authored muster at `(6, 97)`, which is a road cell too.
const ENGINE_OUT_BLUE: (u16, u16) = (56, 97);

/// **Two machines with their engines gone still fight out to their reach** (16b
/// cut AA.1; 16c `B-76`).
///
/// Red Tank 1 at its authored muster `(6, 97)` and Blue Tank 1 at
/// [`ENGINE_OUT_BLUE`] `(56, 97)` — both **road** cells, 50,000 cm apart on one
/// row with a clear line: past `ENGAGE_CM` and inside `RELEASE_CM`, the window a
/// dead engine used to make permanent. Both check out with the engine killed
/// through `hull.rear` by their M32 records (`support::engine_struck`: the two
/// cells to their minimum and `SetSectionCondition { engine, destroyed }`, which
/// is what `plan_hit` writes on the transition), so `limits.motion_disabled`
/// comes from the arena's own `entry_consequences` and not from a test's flag.
///
/// **The ground is chosen for the sensor, not for the look of it.** Both
/// machines are stopped for the whole run, and `visual_mk1` sees a stopped
/// machine at its own cell's class — road 1,000 per mille (60 cells), ground 900
/// (54), grass 800 (48) — so a road-to-grass pairing at 50 cells would be
/// one-sided. Road to road, each sees the other at the full 60 cells.
///
/// At HEAD each re-planned a stand-off cell it could not drive to on every tick,
/// never fired, and — armed — held its side up for good. Now each journals a shot
/// from between the two edges inside two arena minutes, without moving. An
/// elimination is not required (two 105 mm guns at 500 m against front plate may
/// take longer than the bound); if the match ends it is printed.
#[test]
fn aa1_two_machines_with_their_engines_gone_still_fight_out_to_their_reach() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1).engine_out()],
            &[Muster::reflex(Team::Blue, 1)
                .engine_out()
                .at(ENGINE_OUT_BLUE)],
        ],
    );
    let red = support::tank(&world, Team::Red, 1);
    let blue = support::tank(&world, Team::Blue, 1);
    let red_cell = support::cell_of(&arena, red).expect("red opened on the window");
    let blue_cell = support::cell_of(&arena, blue).expect("blue opened on the window");
    assert_eq!(red_cell, (6, 97), "red's authored muster");
    assert_eq!(
        blue_cell, ENGINE_OUT_BLUE,
        "blue kept the cell it asked for"
    );
    for cell in [red_cell, blue_cell] {
        assert_eq!(
            arena
                .definition
                .terrain
                .get(cell.0, cell.1)
                .map(|terrain| terrain.class),
            Some(mush_atlas::TerrainClass::Road),
            "{cell:?} is not a road cell, so a stopped machine there is not seen at 60 cells"
        );
    }
    let apart = range_cm(centre_of(red_cell), centre_of(blue_cell));
    assert!(
        apart > ENGAGE_CM && apart <= RELEASE_CM,
        "the two are {apart} cm apart, which is not between the two edges"
    );
    for id in [red, blue] {
        let Some(unit) = arena.unit(id) else {
            panic!("#{} is in the arena", id.0)
        };
        assert!(
            unit.limits.motion_disabled,
            "#{}'s destroyed engine did not disable its motion at the check-out",
            id.0
        );
        assert!(unit.armed_at_checkout(), "#{} carries its gun", id.0);
        assert_eq!(
            arena.reflex_health_permille(id),
            909,
            "#{}: (2,990 - 150 - 120) of 2,990, conditions not counted",
            id.0
        );
        assert!(!arena.out_of_action(id), "#{} is not out of action", id.0);
    }
    let opened: Vec<(mush_core::ObjId, i64, i64, u32)> = [red, blue]
        .into_iter()
        .filter_map(|id| {
            arena
                .unit(id)
                .map(|unit| (id, unit.pose.x_cm, unit.pose.y_cm, unit.pose.heading_mdeg))
        })
        .collect();

    let mut timeline = Timeline::new();
    let mut seen: Vec<(mush_core::ObjId, i64, Option<i64>)> = Vec::new();
    let mut bytes = run_until(&mut arena, 120_000, 1_000, |arena, out| {
        timeline.sample(arena, out.reflex.path_failures);
        for (unit, row) in arena.reflex.all() {
            let Some(state) = arena.unit(unit) else {
                continue;
            };
            let plot = row
                .target
                .and_then(|(contact, _)| arena.contacts.resolve(unit, contact))
                .map(|held| {
                    range_cm(
                        (state.pose.x_cm, state.pose.y_cm),
                        (held.seen_at.x_cm, held.seen_at.y_cm),
                    )
                });
            seen.push((unit, arena.now_ms, plot));
        }
    });
    println!(
        "aa1_engine_out_duel: ended {:?} at {} ms",
        arena.ended.as_ref().map(|end| end.reason),
        arena.now_ms
    );
    let log = ledger_events(&mut arena, &mut bytes);
    for id in [red, blue] {
        let first = log
            .iter()
            .filter(|event| event.kind.as_str() == "fired")
            .find(|event| support::unit_of_event(event) == Some(id))
            .unwrap_or_else(|| {
                panic!(
                    "#{} never fired ({} `locked` lines in the log). What the machines were \
                     doing:\n{}",
                    id.0,
                    log.iter()
                        .filter(|event| event.kind.as_str() == "locked")
                        .count(),
                    timeline.render()
                )
            });
        let range = seen
            .iter()
            .filter(|(unit, at_ms, _)| *unit == id && *at_ms >= first.t_ms)
            .min_by_key(|(_, at_ms, _)| *at_ms)
            .and_then(|(_, _, plot)| *plot)
            .unwrap_or_else(|| panic!("#{}'s plot at its first shot", id.0));
        println!(
            "aa1_engine_out_duel: #{} first fired at {} ms from {range} cm",
            id.0, first.t_ms
        );
        assert!(
            range > ENGAGE_CM && range <= RELEASE_CM,
            "#{}'s first shot was taken at {range} cm, not between the two edges",
            id.0
        );
    }
    for (id, x_cm, y_cm, heading_mdeg) in opened {
        assert_eq!(
            arena
                .unit(id)
                .map(|unit| (unit.pose.x_cm, unit.pose.y_cm, unit.pose.heading_mdeg)),
            Some((x_cm, y_cm, heading_mdeg)),
            "#{} moved with its engine gone",
            id.0
        );
    }
}

/// **16b cut M.4 (16c `B-56`): on the ground the owner plays on, A sees B
/// exactly when B sees A.**
///
/// The proving ground's authored heightfield, twenty thousand fixed-seed pairs of
/// cells at most 60 cells apart on either axis (the shipped sight's reach), each
/// traced both ways through the arena's own line walk
/// (`mush_atlas::trace::trace_cells` over the arena's `ArenaTerrain`, which is
/// what the perception pass calls) with both eyes at the stock tank's 150 cm.
/// The answer — clear, attenuated by how much, or blocked — is the same from
/// either end; which obstacle a blocked walk meets first is not compared, since
/// that is a fact about the direction of the walk.
#[test]
fn m4_on_the_proving_ground_a_sees_b_exactly_when_b_sees_a() {
    use mush_atlas::trace::{Eye, Sight, trace_cells};

    /// splitmix64, so the pairs are the same on every run and every platform.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn seen(sight: &Sight) -> (u8, u8) {
        match sight {
            Sight::Clear => (0, 0),
            Sight::Partial(pct) => (1, *pct),
            Sight::BlockedBy(_) => (2, 0),
        }
    }

    let world = booted_world();
    let definition = tacsim::match_::definition_for(
        world.world(),
        &smol_str::SmolStr::new_static(tacsim::match_::PROVING_GROUND),
    )
    .expect("the proving ground's ground builds")
    .expect("the proving ground is shipped");
    let terrain = &definition.terrain;
    let (width, height) = (i64::from(terrain.width()), i64::from(terrain.height()));
    let mut state = 0x4d56_u64;
    let (mut blocked, mut open, mut pairs) = (0_u32, 0_u32, 0_u32);
    while pairs < 20_000 {
        let pick = |state: &mut u64, span: i64| {
            i64::try_from(next(state) % u64::try_from(span).unwrap_or(1)).unwrap_or(0)
        };
        let a = (pick(&mut state, width), pick(&mut state, height));
        let b = (
            a.0 + pick(&mut state, 121) - 60,
            a.1 + pick(&mut state, 121) - 60,
        );
        let (Ok(ax), Ok(ay), Ok(bx), Ok(by)) = (
            u16::try_from(a.0),
            u16::try_from(a.1),
            u16::try_from(b.0),
            u16::try_from(b.1),
        ) else {
            continue;
        };
        if b.0 >= width || b.1 >= height {
            continue;
        }
        pairs += 1;
        let from = Eye {
            cell: (ax, ay),
            height_cm: 150,
        };
        let to = Eye {
            cell: (bx, by),
            height_cm: 150,
        };
        let there = trace_cells(terrain, from, to);
        let back = trace_cells(terrain, to, from);
        assert_eq!(
            seen(&there.sight),
            seen(&back.sight),
            "({ax}, {ay}) and ({bx}, {by}) disagree: {:?} against {:?}",
            there.sight,
            back.sight
        );
        assert_eq!(there.attenuation, back.attenuation);
        if seen(&there.sight).0 == 2 {
            blocked += 1;
        } else {
            open += 1;
        }
    }
    // The ground is worth the property: both answers occur.
    assert!(blocked > 0 && open > 0, "{blocked} blocked, {open} not");
}

/// **16b cut AF.1: the shipped centre, snapshotted** (16c N16) — twelve stock
/// tanks, the production roster's own six a side on the machine, on the
/// proving ground, stepped at a one-second cadence past a minute and on to the
/// first pulse boundary with a round in flight (the missile rack's 250 m/s
/// round is airborne for seconds). The encoded state is under half a mebibyte
/// and the figure is printed as `af1_shipped_bytes=` — the first data point for
/// the bound 16b cut AF.2 measures on the harness and sets there; it holds the
/// shot and no float anywhere (the stock units' own rules blobs are in it —
/// the shipped content's float guard); and it decodes on the shipped ground,
/// under the shipped locomotion table, to the arena that was encoded.
///
/// **And the round lands the same**: at 1,000 ms and again at 500 ms, the
/// decoded arena resumed on the last wall and the uninterrupted arena (a clone
/// of the one encoded) are stepped thirty seconds further side by side — one
/// tape byte for byte, one per-substep hash sequence, one final state hash and
/// one pair of ledger digests. The only arena-level proof that a `Shot`'s
/// unhashed fields and its impact's due time ride a resume: the state hash
/// folds only how many shots and due events there are.
#[test]
fn af1_the_shipped_centre_encodes_under_the_expectation() {
    const CADENCE_MS: i64 = 1_000;
    let world = booted_world();
    let pods = tacsim::match_::PODS_PER_SIDE;
    let red: Vec<Muster> = (1..=pods)
        .map(|slot| Muster::reflex(Team::Red, slot))
        .collect();
    let blue: Vec<Muster> = (1..=pods)
        .map(|slot| Muster::reflex(Team::Blue, slot))
        .collect();
    let mut arena = shipped_arena(&world, &[&red, &blue]);
    assert_eq!(arena.units.len(), 12, "twelve stock tanks");
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        assert!(
            out.ended.is_none(),
            "the match ended at {} ms with no round in flight at a pulse boundary",
            arena.now_ms
        );
        if arena.now_ms >= 60_000 && !arena.shots.is_empty() {
            break;
        }
        assert!(
            arena.now_ms < 300_000,
            "no round was in flight at a pulse boundary in five minutes"
        );
        wall += CADENCE_MS;
    }
    let snapshot =
        tacsim::arena::Snapshot::encode(&arena).expect("the shipped centre encodes between pulses");
    let bytes = snapshot.bytes().len();
    println!("af1_shipped_bytes={bytes} at_ms={}", arena.now_ms);
    assert!(
        bytes < 512 * 1024,
        "the shipped centre's snapshot is {bytes} bytes, past the 512 KiB expectation"
    );
    let state: serde_json::Value =
        serde_json::from_slice(snapshot.bytes()).expect("the state string is JSON");
    assert!(
        state
            .get("shots")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|shots| !shots.is_empty()),
        "the snapshot carries the round in flight"
    );
    let carried: Vec<u64> = arena.shots.keys().copied().collect();
    let numbers = no_float(&state, "state");
    assert!(numbers > 1_000, "{numbers} numbers is not a shipped state");
    let decoded = tacsim::arena::Snapshot::decode(&snapshot.document, arena.definition.clone())
        .expect("it decodes on the shipped ground");
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        snapshot.state_hash
    );
    for cadence in [CADENCE_MS, 500] {
        let mut uninterrupted = arena.clone();
        let mut resumed =
            tacsim::arena::Snapshot::decode(&snapshot.document, arena.definition.clone())
                .expect("it decodes again");
        let _ = resumed.resume_at(wall);
        let until = arena.now_ms + 30_000;
        let reference = run_on(&mut uninterrupted, wall, cadence, until);
        let continued = run_on(&mut resumed, wall, cadence, until);
        // A round the snapshot carried in flight lands in the continuation.
        let tape = String::from_utf8_lossy(&reference.0);
        assert!(
            carried.iter().any(|seq| {
                let shot = format!("\"shot_seq\":{seq},");
                tape.lines()
                    .any(|line| line.contains("\"kind\":\"impact\"") && line.contains(&shot))
            }),
            "at {cadence} ms: no round the snapshot carried ({carried:?}) landed"
        );
        assert!(
            continued == reference,
            "at {cadence} ms the resumed centre is not the uninterrupted one \
             (tape {} against {} bytes, {} against {} substeps)",
            continued.0.len(),
            reference.0.len(),
            continued.1.len(),
            reference.1.len()
        );
    }
}

/// **The rest of a shipped run from `wall`** (16b cut AF.1): stepped at
/// `cadence_ms` until arena time reaches `until_ms`, then flushed — the tape it
/// handed out, byte for byte, the per-substep hashes, the final state hash and
/// the ledger's two running digests.
fn run_on(
    arena: &mut Arena,
    wall: i64,
    cadence_ms: i64,
    until_ms: i64,
) -> (Vec<u8>, Vec<u64>, u64, [u8; 32], [u8; 32]) {
    let mut tape: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = wall;
    while arena.now_ms < until_ms {
        wall += cadence_ms;
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            tape.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        if out.ended.is_some() {
            break;
        }
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        tape.extend_from_slice(&chunk.bytes);
    }
    (
        tape,
        hashes,
        tacsim::arena::hash::state_hash(arena),
        arena.ledger.digest(),
        arena.ledger.shipped_digest(),
    )
}

/// **No float anywhere in a JSON value** (16b cut AF.1): answers how many
/// numbers it walked.
fn no_float(value: &serde_json::Value, path: &str) -> usize {
    match value {
        serde_json::Value::Number(number) => {
            assert!(!number.is_f64(), "{path} is a float: {number}");
            1
        }
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| no_float(item, &format!("{path}[]")))
            .sum(),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, item)| no_float(item, &format!("{path}.{key}")))
            .sum(),
        _ => 0,
    }
}
