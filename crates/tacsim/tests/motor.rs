//! **The motor's audit** (17b cut M1a.1; doc 17 §4 "The motor", D-UM-3: *the
//! motor's reads need an audit in M1a*).
//!
//! `crates/tacsim/src/arena/motor.rs` is what drives a hull to a goal, and from
//! 17b cut M1a.4 on it drives a pilot's `DRIVE` and a mind's `goto` as well as
//! the reflex controller's goals. Doc 17 §1's observer rule then bites on it
//! twice over: a kernel that read another unit's live pose, or the spatial
//! index, would hand a pilot or a mind knowledge no sensor gave it. So each
//! kernel the motor exposes is held here, on the map the owner plays on
//! ([`support::shipped_arena`]), to what it may read — the ground, the hull's
//! own pose and row, the plot it is handed and the cells it was refused — and
//! the two pins of the cut are asserted: the shipped duel hashes as it did at
//! `32a1cc23`, and the doctrine the motor reads is the arena's.
//!
//! The kernels are `pub` on [`Arena`] since this cut for exactly this file (and
//! for 17b cut M1a.6, which lifts them into free functions).

mod support;

use mush_core::ObjId;
use support::{CELL_CM, Muster, booted_world, cell_of, run_until, shipped_arena, tank};
use tacsim::arena::CostRow;
use tacsim::arena::motion::BlockedCause;
use tacsim::arena::motor::{MotorRow, Refusal};
use tacsim::arena::reflex::ReflexCounters;
use tacsim::arena::state::{Arena, HoldState, Power, SeenPose, Team};

/// The final state hash of [`duel`] stepped at 1,000 ms for 60 s, read at
/// `32a1cc23` — before `arena/motor.rs` existed — off this very run.
const DUEL_HASH_AT_HEAD: u64 = 0xd3e7_ee71_474b_6738;

/// Two machines a side, every one on the reflex controller, at their
/// authored musters on the proving ground.
fn duel(world: &mush_testkit::TestWorld) -> Arena {
    shipped_arena(
        world,
        &[
            &[Muster::reflex(Team::Red, 1), Muster::reflex(Team::Red, 2)],
            &[Muster::reflex(Team::Blue, 1), Muster::reflex(Team::Blue, 2)],
        ],
    )
}

/// The centre of `cell`, centimetres.
fn centre(cell: (u16, u16)) -> (i64, i64) {
    let side = i64::from(CELL_CM);
    (
        i64::from(cell.0) * side + side / 2,
        i64::from(cell.1) * side + side / 2,
    )
}

/// Put `unit` at `cell`'s centre and rebuild the index, as a staff place would
/// leave it (minus the ledger line, which nothing here reads).
fn put(arena: &mut Arena, unit: ObjId, cell: (u16, u16)) {
    let (x, y) = centre(cell);
    if let Some(state) = arena.unit_mut(unit) {
        state.pose.x_cm = x;
        state.pose.y_cm = y;
    }
    arena.index.rebuild(&arena.definition.terrain, &arena.units);
}

/// **17b cut M1a.1's second pin: the shipped duel hashes as it did** (the M1a
/// plan's head, "the production hash"). Four stock machines on the proving
/// ground for a minute at a one-second cadence — patrols, searches, routes and
/// the steer, every one of them now the motor's — end in the state hash read at
/// `32a1cc23` before the motor was split out. 17b cuts M1a.4, M1a.5 and M1a.6
/// re-assert it; a move is a defect in the move, never a re-bless.
#[test]
fn m1a1_the_shipped_duel_hashes_as_before() {
    let world = booted_world();
    let mut arena = duel(&world);
    let mut searched = 0_u32;
    let _ = run_until(&mut arena, 60_000, 1_000, |_, out| {
        searched = searched.saturating_add(out.reflex.path_queries);
    });
    assert_eq!(arena.now_ms, 60_000);
    assert!(searched > 0, "the motor searched: the pin covers a drive");
    assert!(
        arena.motors.rows().any(|(_, motor)| motor.goal.is_some()),
        "the machines are driving somewhere"
    );
    assert_eq!(
        tacsim::arena::hash::state_hash(&arena),
        DUEL_HASH_AT_HEAD,
        "the shipped duel's hash moved"
    );
}

/// **The doctrine the motor reads is the arena's** (17b cut M1a.1): `follow`
/// reads `self.arena_cfg.reflex` where it read the reflex memory's copy, and the
/// two are one value — `Arena::new` builds the memory from that block and
/// nothing writes either afterwards. `crates/tacsim/tests/reflex.rs` asserts the
/// same on its own rig, under a non-default doctrine.
#[test]
fn m1a1_the_reflex_config_is_the_arenas() {
    let world = booted_world();
    let arena = duel(&world);
    assert_eq!(arena.reflex.config(), &arena.arena_cfg.reflex);
}

/// **`stand_off_cell` reads the plot and not the target** (doc 17 §4's audit;
/// 16b cut F.3's rule). Red Tank 1 holds Blue Tank 1 as a classified contact
/// whose plot is where Blue stands; Blue is then moved two cells by writing its
/// live pose, and the stand-off cell off the held plot is unchanged — while a
/// plot read off Blue's new pose would have given another cell, so the
/// assertion could see a leak.
#[test]
fn m1a1_stand_off_cell_reads_the_plot_and_not_the_target() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = tank(&world, Team::Red, 1);
    let blue = tank(&world, Team::Blue, 1);
    let seen = arena.unit(blue).map(SeenPose::of).expect("blue");
    let id = tacsim::arena::rng::draw_contact_id(support::SEED, red, blue);
    arena.contacts.insert(
        red,
        id,
        tacsim::arena::state::ShadowContact::sighted(
            blue,
            tacsim::arena::state::Classification::Classified { team: Team::Blue },
            0,
            seen,
        ),
    );
    let plot = |arena: &Arena| arena.contacts.resolve(red, id).map(|held| held.seen_at);
    let want = 39_000_u64;
    let before = plot(&arena).and_then(|at| arena.stand_off_cell(red, &at, want));
    assert!(before.is_some(), "a stand-off cell on the shipped ground");

    let moved_to = cell_of(&arena, blue).map(|(x, y)| (x, y.saturating_add(2)));
    put(&mut arena, blue, moved_to.expect("blue is on the window"));
    assert_eq!(cell_of(&arena, blue), moved_to, "blue moved");
    assert_eq!(plot(&arena), Some(seen), "and its plot did not");
    assert_eq!(
        plot(&arena).and_then(|at| arena.stand_off_cell(red, &at, want)),
        before,
        "the stand-off cell followed the target's live pose"
    );
    let live = arena.unit(blue).map(SeenPose::of).expect("blue");
    assert_ne!(
        arena.stand_off_cell(red, &live, want),
        before,
        "the probe could not have seen a leak: the live pose gives the same cell"
    );
}

/// The four pickers' answers: the entry cell, the far entry cell, the row the
/// waypoint walk leaves, and whether the hull has arrived at the Hold spot.
type Picks = (Option<(u16, u16)>, Option<(u16, u16)>, MotorRow, bool);

/// What the four navigation pickers answer for `unit`, standing where it stands.
fn picks(arena: &Arena, unit: ObjId) -> Picks {
    let here = cell_of(arena, unit);
    let mut row = MotorRow::default();
    arena.patrol_step(unit, &mut row, here);
    (
        arena.entry_cell(unit),
        arena.far_entry_cell(unit, &[]),
        row,
        arena.hold_arrived(unit, here),
    )
}

/// **The navigation pickers read the ground and the own pose only** (doc 17
/// §4's audit). `entry_cell` (a withdrawal's goal), `far_entry_cell` (a hunt's),
/// `patrol_step` (the waypoint walk) and `hold_arrived` (a Hold match's arrival,
/// with a Hold objective written onto the arena whose spot is the unit's own
/// cell) answer for Red Tank 1 exactly what they answered with every other unit
/// removed from the arena, and with every other unit standing on each cell
/// they picked.
#[test]
fn m1a1_the_nav_pickers_read_the_ground_and_the_own_pose_only() {
    let world = booted_world();
    let red = tank(&world, Team::Red, 1);
    let field = || {
        let mut arena = duel(&world);
        let here = cell_of(&arena, red).expect("red is on the window");
        arena.hold = Some(HoldState {
            spot: here,
            letter: None,
            radius_cells: 3,
            hold_ms: 60_000,
            held_by: None,
        });
        arena
    };
    let arena = field();
    let base = picks(&arena, red);
    assert!(base.0.is_some() && base.1.is_some(), "{base:?}");
    assert!(base.2.goal.is_some(), "the walk has a node: {base:?}");
    assert!(base.3, "standing on the spot is arrived");

    // Alone.
    let mut alone = field();
    alone.units.retain(|unit| unit.id == red);
    alone.index.rebuild(&alone.definition.terrain, &alone.units);
    assert_eq!(alone.units.len(), 1);
    assert_eq!(picks(&alone, red), base, "the pickers read another unit");

    // Everybody else on each picked cell in turn.
    let others: Vec<ObjId> = arena
        .units
        .iter()
        .map(|unit| unit.id)
        .filter(|id| *id != red)
        .collect();
    assert_eq!(others.len(), 3);
    let picked = [base.0, base.1, base.2.goal];
    for cell in picked.into_iter().flatten() {
        let mut crowded = field();
        for other in &others {
            put(&mut crowded, *other, cell);
        }
        assert_eq!(
            picks(&crowded, red),
            base,
            "the pickers read the units standing on {cell:?}"
        );
    }
}

/// The four open-line kernels' answers for `unit` toward `next`.
fn walks(arena: &Arena, unit: ObjId, next: (u16, u16), avoid: &[(u16, u16)]) -> [bool; 5] {
    let state = arena.unit(unit).cloned().expect("the unit");
    let here = cell_of(arena, unit).expect("on the window");
    let heading = state.pose.heading_mdeg;
    let (to_x, to_y) = centre(next);
    let (dx, dy) = (to_x - state.pose.x_cm, to_y - state.pose.y_cm);
    let reach = (dx * dx + dy * dy).isqrt();
    let mut counters = ReflexCounters::default();
    [
        arena.step_is_drivable(&state, here, next, avoid),
        arena.first_step_is_open(&state, here, heading, avoid, &mut counters),
        arena.line_is_open(&state, here, heading, 5_000, avoid, false),
        arena.line_is_open(&state, here, heading, 5_000, avoid, true),
        arena.walk_is_open(&state, here, (dx, dy, reach), reach, avoid, true),
    ]
}

/// **The open-line walks read terrain and the avoid list, and never the
/// index** (doc 17 §4's audit; doc 16 §3.6: a hull knows it was refused a
/// cell, never what refused it). Red Tank 1 drives a route on the proving
/// ground; Blue Tank 1 is put on the route's next cell, the index rebuilt, and
/// `step_is_drivable`, `first_step_is_open`, `line_is_open` (both walks) and
/// `walk_is_open` answer what they answered before. The hull then drives on,
/// is stopped by the occupant, and `note_blocked` writes the cell onto its
/// motor row's avoid list — the one way traffic reaches the motor.
#[test]
fn m1a1_the_open_line_walks_read_terrain_and_the_avoid_list_and_never_the_index() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = tank(&world, Team::Red, 1);
    let blue = tank(&world, Team::Blue, 1);
    // Under way on a straight run of its route, pointed at the next cell: the
    // route's first three cells one step apart in one direction, that step's
    // compass heading within ten degrees of the hull's, the hull in the cell one
    // step short of the first and still with cells to go past it — so the next
    // cell is one motion must enter, and the hull is not about to arrive and
    // stop short of it on its own. And every one of the five walks open there
    // before anything stands on the cell: an occupant can only close a line, so
    // a walk already closed by the ground could not show an index read.
    let mut wall = 0_i64;
    let next = loop {
        let _ = arena.step(wall, &[], &[]);
        wall += 250;
        assert!(
            arena.now_ms < 300_000,
            "red never ran straight at a route cell"
        );
        let Some(state) = arena.unit(red) else {
            continue;
        };
        let (Some(here), Some(motor)) = (cell_of(&arena, red), arena.motors.row(red)) else {
            continue;
        };
        let [a, b, c, _, ..] = motor.route.as_slice() else {
            continue;
        };
        let step = |from: (u16, u16), to: (u16, u16)| {
            (
                i32::from(to.0) - i32::from(from.0),
                i32::from(to.1) - i32::from(from.1),
            )
        };
        let way = step(here, *a);
        let compass = match way {
            (0, 1) => 0_u32,
            (1, 1) => 45_000,
            (1, 0) => 90_000,
            (1, -1) => 135_000,
            (0, -1) => 180_000,
            (-1, -1) => 225_000,
            (-1, 0) => 270_000,
            (-1, 1) => 315_000,
            _ => continue,
        };
        let off = state.pose.heading_mdeg.abs_diff(compass);
        let off = off.min(360_000_u32.saturating_sub(off));
        let remembered: Vec<(u16, u16)> = motor.avoid.iter().map(|(cell, _)| *cell).collect();
        if state.speed_cms > 300
            && off <= 10_000
            && step(*a, *b) == way
            && step(*b, *c) == way
            && walks(&arena, red, *a, &remembered) == [true; 5]
        {
            break *a;
        }
    };
    let avoid: Vec<(u16, u16)> = arena
        .motors
        .row(red)
        .map(|motor| motor.avoid.iter().map(|(cell, _)| *cell).collect())
        .unwrap_or_default();
    assert!(
        !avoid.contains(&next),
        "the premise: nothing remembered there"
    );
    let before = walks(&arena, red, next, &avoid);
    assert_eq!(
        before, [true; 5],
        "the premise: every walk is open before the occupant, so an index read could only close one"
    );

    put(&mut arena, blue, next);
    assert_eq!(arena.index.cell_of_unit(blue), Some(next), "blue is on it");
    assert_eq!(
        walks(&arena, red, next, &avoid),
        before,
        "an open-line walk read the index"
    );

    // And the hull, which cannot see the occupant in any kernel, bumps it. Its
    // sight is taken away first, so the hostile in the road is a stone and not
    // a target: a machine that classified it would stop to fight it, and what
    // is under test here is the drive.
    if let Some(state) = arena.unit_mut(red) {
        state.sensor = None;
    }
    let until = arena.now_ms + 30_000;
    while arena.now_ms < until {
        let _ = arena.step(wall, &[], &[]);
        wall += 250;
        if arena
            .motors
            .row(red)
            .is_some_and(|motor| motor.avoid.iter().any(|(cell, _)| *cell == next))
        {
            break;
        }
    }
    assert!(
        arena
            .motors
            .row(red)
            .is_some_and(|motor| motor.avoid.iter().any(|(cell, _)| *cell == next)),
        "the stop at {next:?} was not learned: {:?}",
        arena.motors.row(red)
    );
}

/// **The routeless pre-pass holds any steered hull, whatever its controller**
/// (17b cut M1a.1: `hold_routeless` reads the motor rows, and a row exists for
/// a hull something is steering). A **pilot's** tank — no machine, no reflex
/// row — is given a motor row with a goal and no route, the shape 17b cut
/// M1a.4's `DRIVE` lays down, stands facing a cell its row cannot enter, and is
/// ordered on; one substep later the pre-pass has ordered it to stop
/// (`StepOutput::reflex`'s `intents` counts the order, and the hull's desired
/// speed is nought). What this cannot show is the order's source: a continuous
/// kind returns from `apply_intent` before the seat's rate is counted whatever
/// its source, so `orders_in_window` staying nought proves only that the stop
/// cost the seat nothing, not that it went in as a machine order.
#[test]
fn m1a1_hold_routeless_holds_a_routeless_row_whatever_the_controller() {
    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let terrain = arena.definition.terrain.clone();
    let row = arena.unit(red).map(|unit| unit.cost_row).expect("red");
    assert_eq!(row, CostRow::Tracked);
    // A cell its row may stand in with one it may not enter due north of it.
    let facing = (0..terrain.height().saturating_sub(1))
        .flat_map(|y| (0..terrain.width()).map(move |x| (x, y)))
        .find(|&(x, y)| terrain.passable_for(row, x, y) && !terrain.passable_for(row, x, y + 1));
    let Some(cell) = facing else {
        panic!("the proving ground has no impassable cell north of a passable one");
    };
    put(&mut arena, red, cell);
    if let Some(state) = arena.unit_mut(red) {
        state.pose.heading_mdeg = 0;
        state.power = Power::Online;
        state.desired_speed_cms = 800;
    }
    assert_eq!(
        arena
            .unit(red)
            .map(|unit| (unit.controller, unit.orders_in_window)),
        Some((tacsim::arena::state::Controller::Pilot, 0)),
        "a pilot's tank, with no seat order counted"
    );
    assert!(arena.reflex.unit(red).is_none(), "and no machine");
    arena.motors.ensure(red).goal = Some((cell.0, cell.1.saturating_add(10)));

    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &arena.arena_cfg.tempo,
        arena.cell_cm(),
        arena.max_speed_cms(),
    ));
    let _ = arena.step(0, &[], &[]);
    let out = arena.step(substep, &[], &[]);
    assert!(arena.now_ms > 0, "a substep ran");
    assert!(out.reflex.intents >= 1, "the pre-pass ordered nothing");
    assert_eq!(
        arena
            .unit(red)
            .map(|unit| (unit.desired_speed_cms, unit.orders_in_window)),
        Some((0, 0)),
        "held, and the stop counted against no seat rate"
    );
}

/// **`age_avoid` ages a pilot's row** (17b cut M1a.1): the sweep that stood at
/// the top of the reflex decision, lifted onto [`MotorRow`] so a row no
/// decision runs — a pilot's `DRIVE`, from 17b cut M1a.4 — ages its stops. Two
/// stops and a remembered refusal: before the TTL nothing goes and the answer
/// is `false`; at the first stop's TTL that stop goes, the refusal with it, and
/// the answer is `true`; a sweep that ages nothing answers `false` again.
#[test]
fn m1a1_age_avoid_ages_a_pilot_rows_stops_and_drops_its_refusal() {
    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    assert!(arena.reflex.unit(red).is_none(), "a pilot's tank");
    let ttl = tacsim::arena::reflex::REFLEX_AVOID_TTL_MS;
    let refusal = Refusal {
        at: (3, 3),
        cell: (3, 4),
        cause: BlockedCause::Impassable,
        goal: (3, 9),
        until_ms: ttl,
    };
    {
        let motor = arena.motors.ensure(red);
        motor.avoid = vec![((1, 1), 0), ((2, 2), 20_000)];
        motor.refused = Some(refusal);
    }
    let Some(motor) = arena.motors.row_mut(red) else {
        panic!("the pilot's row");
    };
    assert!(
        !motor.age_avoid(ttl - 1),
        "nothing aged a millisecond early"
    );
    assert_eq!(motor.avoid, vec![((1, 1), 0), ((2, 2), 20_000)]);
    assert_eq!(motor.refused, Some(refusal));

    assert!(motor.age_avoid(ttl), "the first stop aged at its TTL");
    assert_eq!(motor.avoid, vec![((2, 2), 20_000)]);
    assert_eq!(motor.refused, None, "and took the refusal with it");

    assert!(!motor.age_avoid(ttl + 1), "nothing more to age");
    assert!(motor.age_avoid(20_000 + ttl), "the second one's turn");
    assert!(motor.avoid.is_empty());
}
