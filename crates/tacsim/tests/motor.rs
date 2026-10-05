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
//! `32a1cc23`, and the doctrine the motor reads is the arena's. Since 17b cut
//! M1b.0 the duel's every ledger line is pinned with its hash as well
//! ([`m1b0_the_production_line_hashes_do_not_move`]).
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

/// **The shipped duel's ledger, every line whole with its hash, recorded at
/// `e23694b0`** (17b cut M1b.0; the M1b plan's §1(a) and (c)) — before the cut
/// touched the due loop, off [`duel`] stepped at 1,000 ms for 60 s, each line
/// as `serde_json` writes a [`LedgerEvent`](tacsim::arena::ledger::LedgerEvent).
/// The minute holds no shot (the four machines start, see, classify and
/// drive), so its only action-run lines are the four reflex `startup`s; every
/// other line carries its substep's state hash, which no action hash reaches
/// (16b cut M.3: `Arena::event_hash` alone reads it). What it pins is the
/// shipped duel's bytes; the reflex shots' lines are `tests/reflex.rs`' half
/// of the pin, and that no trigger opens `ActionHash::Pending` for a hull with
/// no slot is asserted where a trigger is built (17b cut M1b.1), not here.
const M1B0_DUEL_LINES: &[&str] = &[
    r#"{"seq":1,"t_ms":0,"kind":"match_started","hash":17218947927064157514}"#,
    r#"{"seq":2,"t_ms":258,"kind":"startup","unit":8,"hash":6531919843051298410}"#,
    r#"{"seq":3,"t_ms":421,"kind":"startup","unit":3,"hash":6531919843051298410}"#,
    r#"{"seq":4,"t_ms":482,"kind":"startup","unit":4,"hash":6531919843051298410}"#,
    r#"{"seq":5,"t_ms":258,"kind":"detected","viewer":8,"contact":8314046734819043908,"target":7,"hash":3550940230815811454}"#,
    r#"{"seq":6,"t_ms":421,"kind":"detected","viewer":3,"contact":5844251635052541762,"target":4,"hash":3550940230815811454}"#,
    r#"{"seq":7,"t_ms":482,"kind":"detected","viewer":4,"contact":5992554990774319474,"target":3,"hash":3550940230815811454}"#,
    r#"{"seq":8,"t_ms":594,"kind":"startup","unit":7,"hash":14780345450209965067}"#,
    r#"{"seq":9,"t_ms":594,"kind":"detected","viewer":7,"contact":3822974778006219272,"target":8,"hash":12083510933067632373}"#,
    r#"{"seq":10,"t_ms":1000,"kind":"tempo","from":"quiet","to":"alert","hash":8975984572782758866}"#,
    r#"{"seq":11,"t_ms":3258,"kind":"classified","viewer":8,"contact":8314046734819043908,"target":7,"team":"blue","draws":[["classify:8:7:4",3884165266672269218]],"hash":7341467584742566535}"#,
    r#"{"seq":12,"t_ms":3421,"kind":"classified","viewer":3,"contact":5844251635052541762,"target":4,"team":"red","draws":[["classify:3:4:4",16846213216163885984]],"hash":7341467584742566535}"#,
    r#"{"seq":13,"t_ms":3482,"kind":"classified","viewer":4,"contact":5992554990774319474,"target":3,"team":"red","draws":[["classify:4:3:4",2094150884743799771]],"hash":7341467584742566535}"#,
    r#"{"seq":14,"t_ms":3594,"kind":"classified","viewer":7,"contact":3822974778006219272,"target":8,"team":"blue","draws":[["classify:7:8:4",989486057018502202]],"hash":14243656697491612673}"#,
    r#"{"seq":15,"t_ms":5000,"kind":"position_summary","units":[[3,6500,97500,100,90000],[4,6500,95500,100,90000],[7,249500,97500,-100,270000],[8,249500,95500,-100,270000]],"hash":10563322734833990605}"#,
    r#"{"seq":16,"t_ms":10000,"kind":"position_summary","units":[[3,6500,97500,100,90000],[4,6500,95500,100,90000],[7,249500,97500,-100,270000],[8,249500,95500,-100,270000]],"hash":6143504604192454938}"#,
    r#"{"seq":17,"t_ms":10258,"kind":"power_online","unit":8,"hash":10227290508120997443}"#,
    r#"{"seq":18,"t_ms":10421,"kind":"power_online","unit":3,"hash":10227290508120997443}"#,
    r#"{"seq":19,"t_ms":10482,"kind":"power_online","unit":4,"hash":10227290508120997443}"#,
    r#"{"seq":20,"t_ms":10594,"kind":"power_online","unit":7,"hash":1478522656763317366}"#,
    r#"{"seq":21,"t_ms":15000,"kind":"position_summary","units":[[3,7439,97532,100,75000],[4,7284,96213,100,58740],[7,249411,98355,-100,11593],[8,248715,96213,-100,301304]],"hash":8866010401143912890}"#,
    r#"{"seq":22,"t_ms":20000,"kind":"position_summary","units":[[3,7007,99968,100,316411],[4,10817,96500,100,90000],[7,249500,101783,-100,0],[8,245182,96500,-200,270000]],"hash":9094660931383026748}"#,
    r#"{"seq":23,"t_ms":25000,"kind":"position_summary","units":[[3,6510,103848,100,359237],[4,16842,96500,200,90000],[7,249685,104621,-200,37500],[8,239120,96500,-200,270000]],"hash":1605124276104618781}"#,
    r#"{"seq":24,"t_ms":30000,"kind":"position_summary","units":[[3,4350,104385,100,224237],[4,24231,96500,200,90000],[7,251770,103083,-100,187500],[8,231639,96500,-200,270000]],"hash":13396401788286150218}"#,
    r#"{"seq":25,"t_ms":35000,"kind":"position_summary","units":[[3,6412,100440,100,138097],[4,30111,96500,200,90000],[7,249058,99460,-100,210129],[8,226376,96500,-200,270000]],"hash":12151586208571199750}"#,
    r#"{"seq":26,"t_ms":40000,"kind":"position_summary","units":[[3,8764,98371,100,80898],[4,32604,96500,200,90000],[7,247671,97719,-100,229535],[8,223701,96500,-300,270000]],"hash":1092346026098237883}"#,
    r#"{"seq":27,"t_ms":45000,"kind":"position_summary","units":[[3,13915,98500,100,89953],[4,36922,96500,200,90000],[7,243327,97500,-200,270000],[8,218739,96500,-300,270000]],"hash":7272711587993577274}"#,
    r#"{"seq":28,"t_ms":50000,"kind":"position_summary","units":[[3,20872,97517,200,95437],[4,43676,96500,200,90000],[7,236514,97500,-200,270000],[8,211570,96500,-300,270000]],"hash":3042155669244909283}"#,
    r#"{"seq":29,"t_ms":55000,"kind":"position_summary","units":[[3,28022,97500,200,90000],[4,50991,96626,300,75000],[7,229273,97500,-200,270000],[8,204573,96500,-300,270000]],"hash":16781236920069850708}"#,
    r#"{"seq":30,"t_ms":60000,"kind":"position_summary","units":[[3,31325,97500,200,90000],[4,54990,97523,200,92559],[7,225761,97500,-300,270000],[8,201099,96500,-300,270000]],"hash":2150276939325885971}"#,
];

/// **17b cut M1b.0: the production line-hash pin, the reflex half.** The
/// shipped duel — four stock machines on the reflex controller on the proving
/// ground — journals exactly the lines it journalled at `e23694b0`, each with
/// its hash; [`m1a1_the_shipped_duel_hashes_as_before`] pins its final state
/// hash, this pins every line's. Every later sub-cut of M1b re-asserts it
/// (`tests/weapons.rs` holds the seat half); a move is a defect, never a
/// re-bless. `tests/reflex.rs` holds the reflex shots' half.
#[test]
fn m1b0_the_production_line_hashes_do_not_move() {
    let world = booted_world();
    let mut arena = duel(&world);
    let mut bytes = run_until(&mut arena, 60_000, 1_000, |_, _| {});
    assert_eq!(arena.now_ms, 60_000);
    let lines: Vec<String> = support::ledger_events(&mut arena, &mut bytes)
        .iter()
        .map(|event| serde_json::to_string(event).unwrap_or_default())
        .collect();
    assert_eq!(
        lines,
        M1B0_DUEL_LINES
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>(),
        "the shipped duel's ledger lines moved"
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

// ---------------------------------------------------------------------------
// 17b cut M1a.4 — a pilot's `DRIVE` (doc 17 §4 D-UM-3, D-UM-10)
// ---------------------------------------------------------------------------

/// The pilot a [`drive_run`] seats in Red Tank 1, so the arena builds frames.
const M1A4_PILOT: ObjId = ObjId(9_001);

/// What one [`drive_run`] produced.
struct DriveRun {
    /// Every ledger line: its time, its kind as JSON, its substep hash.
    events: Vec<(i64, String, u64)>,
    /// The state hash of every substep, in order.
    hashes: Vec<u64>,
    /// How many pulses it took — three cadences really are three numbers.
    pulses: u32,
    /// Every `drive_*` notice the pilot's frames carried, with the arena
    /// window `[start, end]` of the pulse that built the frame.
    told: Vec<(&'static str, i64, i64)>,
    /// After each pulse: the arena time it started at, and the motor's goal.
    goals: Vec<(i64, Option<(u16, u16)>)>,
    /// After each pulse: the arena time it started at, and the speed the tank
    /// was ordered.
    speeds: Vec<(i64, i32)>,
    /// Where the tank ended, and what it was last ordered.
    cell: Option<(u16, u16)>,
    desired_heading_mdeg: u32,
    desired_speed_cms: i32,
    /// Motor ticks left on the heap at the end.
    ticks: usize,
}

/// **Red Tank 1, a pilot's, on the proving ground**, driven at `cadence_ms`
/// to `until_ms` through `beats` — each `(t_ms, kind)` delivered as a seat's
/// order, at the unit's current epoch, in the first pulse whose arena time has
/// reached `t_ms` (every `t_ms` a multiple of 5,000, so one script means one
/// thing at all three cadences: `tests/arena.rs`'s `Beat` rule).
fn drive_run(
    world: &mush_testkit::TestWorld,
    cadence_ms: i64,
    until_ms: i64,
    beats: &[(i64, tacsim::arena::intents::IntentKind)],
) -> DriveRun {
    use tacsim::arena::intents::Intent;
    use tacsim::arena::perception::FrameNotice;
    let mut arena = shipped_arena(world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(world, Team::Red, 1);
    let _ = arena.seat_pilot(
        M1A4_PILOT,
        red,
        arena
            .epochs
            .current(red, tacsim::arena::state::Channel::Helm),
    );
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut run = DriveRun {
        events: Vec::new(),
        hashes: Vec::new(),
        pulses: 0,
        told: Vec::new(),
        goals: Vec::new(),
        speeds: Vec::new(),
        cell: None,
        desired_heading_mdeg: 0,
        desired_speed_cms: 0,
        ticks: 0,
    };
    let mut next = 0_usize;
    let mut seq = 0_u64;
    let mut wall = 0_i64;
    loop {
        let start = arena.now_ms;
        let mut intents: Vec<Intent> = Vec::new();
        while let Some((at, kind)) = beats.get(next) {
            if *at > arena.now_ms {
                break;
            }
            seq += 1;
            let channel = kind.channel_required();
            intents.push(Intent {
                unit: red,
                channel,
                epoch: arena.epochs.current(red, channel),
                kind: kind.clone(),
                seq,
            });
            next += 1;
        }
        let out = arena.step(wall, &[], &intents);
        run.pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        run.hashes.extend(arena.substep_hashes.iter().copied());
        for (_, frame) in &out.frames {
            for notice in &frame.notices {
                if let FrameNotice::Drive { .. } = notice {
                    run.told.push((notice.kind().as_str(), start, arena.now_ms));
                }
            }
        }
        run.goals
            .push((start, arena.motors.row(red).and_then(|motor| motor.goal)));
        run.speeds.push((
            start,
            arena.unit(red).map_or(0, |unit| unit.desired_speed_cms),
        ));
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    run.events = support::ledger_events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    run.cell = cell_of(&arena, red);
    if let Some(unit) = arena.unit(red) {
        run.desired_heading_mdeg = unit.desired_heading_mdeg;
        run.desired_speed_cms = unit.desired_speed_cms;
    }
    run.ticks = arena
        .due
        .iter()
        .filter(|event| event.kind == tacsim::arena::state::DueKind::MotorTick { unit: red })
        .count();
    run
}

/// The three cadences of one script, held to one event list and one hash
/// sequence, answered for the case's own assertions.
fn drive_at_three_cadences(
    world: &mush_testkit::TestWorld,
    until_ms: i64,
    beats: &[(i64, tacsim::arena::intents::IntentKind)],
    case: &str,
) -> Vec<DriveRun> {
    let runs: Vec<DriveRun> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| drive_run(world, cadence, until_ms, beats))
        .collect();
    let [slow, mid, fast] = runs.as_slice() else {
        panic!("three runs");
    };
    assert!(
        slow.pulses < mid.pulses && mid.pulses < fast.pulses,
        "{case}: three cadences, three pulse counts"
    );
    for (cadence, run) in [(1_000, mid), (500, fast)] {
        assert_eq!(
            run.events, slow.events,
            "{case}: the event list at {cadence} ms"
        );
        assert_eq!(
            run.hashes, slow.hashes,
            "{case}: the hash sequence at {cadence} ms"
        );
        assert_eq!(
            run.cell, slow.cell,
            "{case}: the final cell at {cadence} ms"
        );
        let kinds = |run: &DriveRun| run.told.iter().map(|(kind, ..)| *kind).collect::<Vec<_>>();
        assert_eq!(
            kinds(run),
            kinds(slow),
            "{case}: the notices at {cadence} ms"
        );
    }
    // Each notice was staged at one arena millisecond: the windows of the
    // pulses that carried it, one per cadence, have a millisecond in common.
    for index in 0..slow.told.len() {
        let windows: Vec<(i64, i64)> = runs
            .iter()
            .filter_map(|run| run.told.get(index).map(|(_, start, end)| (*start, *end)))
            .collect();
        let from = windows.iter().map(|(start, _)| *start).max().unwrap_or(0);
        let to = windows.iter().map(|(_, end)| *end).min().unwrap_or(0);
        assert!(from <= to, "{case}: notice {index} in windows {windows:?}");
    }
    runs
}

/// **A pilot's `DRIVE` drives the tank to the cell, at three cadences** (17b
/// cut M1a.4; doc 17 §4 D-UM-3 and D-UM-10; doc 16 §3.1's D-16-2 — 16b §11's
/// mandatory cross-tempo fixture for a cut that changes arena behaviour).
///
/// Red Tank 1 on the proving ground, a pilot's, powered up by a scripted
/// `STARTUP` at nought; four scripts, each run at 5,000, 1,000 and 500 ms and
/// held to one event list, one hash sequence and one final cell:
///
/// 1. a `Goal { Cell }` at 15,000 to a cell twenty cells along the road: the
///    tank ends within `ARRIVE_CELLS` of it, the pilot is told `drive_arrived`
///    exactly once (the pulses that carried it share a millisecond at every
///    cadence), the goal is gone and no motor tick is left on the heap;
/// 2. a tank already driving (a `SPEED 300` at 10,000) given a goal on ground
///    its row cannot stand on: `drive_no_route` once, at the landing, the goal
///    dropped, the ordered speed nought — the "holding" of the sentence — no
///    tick left;
/// 3. the first script with a `HEADING 180` at 20,000 — a heading the road's
///    route never gives — the goal is still held after the pulse that ends at
///    20,000 and gone after the one that starts there, at every cadence, and
///    the tank is ordered the typed heading — no notice;
/// 4. the first script with a `DRIVE HOLD` at 20,000: no goal, speed nought,
///    no notice.
#[test]
fn m1a4_a_piloted_drive_drives_to_the_cell_at_three_cadences() {
    use tacsim::arena::intents::{IntentKind, MotorGoal};
    use tacsim::arena::motor::ARRIVE_CELLS;

    let world = booted_world();
    let probe = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let from = cell_of(&probe, red).expect("red is on the window");
    let terrain = &probe.definition.terrain;
    let row = probe.unit(red).map(|unit| unit.cost_row).expect("red");
    let to = (from.0 + 20, from.1);
    assert!(terrain.passable_for(row, to.0, to.1), "the road at {to:?}");
    // The nearest cell five or more cells off that the tank's row cannot
    // stand on: no route reaches it.
    let sealed = (0..terrain.height())
        .flat_map(|y| (0..terrain.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| !terrain.passable_for(row, x, y))
        .filter(|&(x, y)| x.abs_diff(from.0).max(y.abs_diff(from.1)) >= 5)
        .min_by_key(|&(x, y)| (x.abs_diff(from.0).max(y.abs_diff(from.1)), x, y))
        .expect("the proving ground has ground a tank cannot stand on");
    let cell = |at: (u16, u16)| IntentKind::Goal(MotorGoal::Cell { x: at.0, y: at.1 });

    // 1. Arrival.
    let runs = drive_at_three_cadences(
        &world,
        90_000,
        &[(0, IntentKind::Startup), (15_000, cell(to))],
        "arrival",
    );
    for run in &runs {
        let at = run.cell.expect("on the window");
        assert!(
            at.0.abs_diff(to.0).max(at.1.abs_diff(to.1)) <= ARRIVE_CELLS,
            "arrived at {at:?} for {to:?}"
        );
        let told: Vec<&str> = run.told.iter().map(|(kind, ..)| *kind).collect();
        assert_eq!(told, ["drive_arrived"], "said once");
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert_eq!(run.ticks, 0, "no motor tick after arrival");
        assert!(
            run.speeds.iter().any(|(_, speed)| *speed > 0),
            "the motor drove it there"
        );
        assert_eq!(run.desired_speed_cms, 0, "and stopped it on arrival");
    }

    // 2. No route.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (10_000, IntentKind::Speed { cms: 300 }),
            (15_000, cell(sealed)),
        ],
        "no route",
    );
    for run in &runs {
        let moving = run
            .speeds
            .iter()
            .rev()
            .find(|(start, _)| *start < 15_000)
            .map(|(_, speed)| *speed);
        assert_eq!(moving, Some(300), "driving when the goal lands");
        let told: Vec<&str> = run.told.iter().map(|(kind, ..)| *kind).collect();
        assert_eq!(told, ["drive_no_route"], "said once");
        let (_, start, _) = run.told.first().copied().unwrap_or(("", 0, 0));
        assert_eq!(start, 15_000, "said at the landing");
        assert_eq!(run.desired_speed_cms, 0);
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert_eq!(run.ticks, 0);
    }

    // 3. A `HEADING` mid-route takes the helm back at its own millisecond.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (15_000, cell(to)),
            (20_000, IntentKind::Heading { mdeg: 180_000 }),
        ],
        "heading",
    );
    for run in &runs {
        let before = run
            .goals
            .iter()
            .rev()
            .find(|(start, _)| *start < 20_000)
            .and_then(|(_, goal)| *goal);
        let after = run
            .goals
            .iter()
            .find(|(start, _)| *start == 20_000)
            .map(|(_, goal)| *goal);
        assert_eq!(before, Some(to), "still driving until the HEADING");
        assert_eq!(after, Some(None), "the HEADING's pulse dropped the goal");
        assert!(run.told.is_empty(), "nothing said: {:?}", run.told);
        assert_eq!(run.desired_heading_mdeg, 180_000, "steering what was typed");
        assert_eq!(run.ticks, 0, "the cadence ended with the goal");
    }

    // 4. `DRIVE HOLD`: a stop, no goal, nothing said.
    let runs = drive_at_three_cadences(
        &world,
        30_000,
        &[
            (0, IntentKind::Startup),
            (15_000, cell(to)),
            (20_000, IntentKind::Goal(MotorGoal::Hold)),
        ],
        "hold",
    );
    for run in &runs {
        assert_eq!(run.desired_speed_cms, 0);
        assert_eq!(run.goals.last().and_then(|(_, goal)| *goal), None);
        assert!(run.told.is_empty(), "a hold says nothing: {:?}", run.told);
    }
}

/// **The fan keeps two same-side goals apart in a far zone** (17b cut M1a.4;
/// the M1a plan's departure (d); 16b cut AI.1's `HUNT_FAN_ROWS`). Two pilots of
/// red `DRIVE` the same passable cell of blue's entry zone in one pulse: the
/// first is given the cell, the second a passable cell `HUNT_FAN_ROWS` or more
/// rows from it (Chebyshev), so their arrival squares do not overlap. The same
/// two orders to a cell outside every far zone are never fanned. And a reflex
/// unit's own hunt fan — `far_entry_cell`, which reads the side's hunt cells it
/// is handed and never a motor row — answers as it did before anyone drove.
#[test]
fn m1a4_the_fan_keeps_two_same_side_goals_apart_in_a_far_zone() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::motor::HUNT_FAN_ROWS;
    use tacsim::arena::state::Channel;

    let world = booted_world();
    let field = || {
        let mut arena = shipped_arena(
            &world,
            &[
                &[Muster::piloted(Team::Red, 1), Muster::piloted(Team::Red, 2)],
                &[Muster::reflex(Team::Blue, 1)],
            ],
        );
        for unit in [tank(&world, Team::Red, 1), tank(&world, Team::Red, 2)] {
            if let Some(state) = arena.unit_mut(unit) {
                state.power = Power::Online;
            }
        }
        arena
    };
    let (one, two) = (tank(&world, Team::Red, 1), tank(&world, Team::Red, 2));
    let blue = tank(&world, Team::Blue, 1);
    let probe = field();
    let row = probe.unit(one).map(|unit| unit.cost_row).expect("red");
    let zone = probe
        .definition
        .entry_zones
        .iter()
        .find(|zone| zone.team == Team::Blue)
        .copied()
        .expect("blue's entry zone");
    let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
    let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
    let mid_y = y0 + (y1 - y0) / 2;
    let target = (x0..=x1)
        .find(|&x| probe.definition.terrain.passable_for(row, x, mid_y))
        .map(|x| (x, mid_y))
        .expect("a passable cell on the zone's middle row");
    let hunt_before = probe.far_entry_cell(blue, &[]);

    let drive_both = |arena: &mut Arena, at: (u16, u16)| {
        let intents: Vec<Intent> = [one, two]
            .into_iter()
            .enumerate()
            .map(|(seq, unit)| {
                let kind = IntentKind::Goal(MotorGoal::Cell { x: at.0, y: at.1 });
                Intent {
                    unit,
                    channel: Channel::Helm,
                    epoch: arena.epochs.current(unit, Channel::Helm),
                    kind,
                    seq: u64::try_from(seq).unwrap_or(0) + 1,
                }
            })
            .collect();
        let _ = arena.step(0, &[], &intents);
        (
            arena.motors.row(one).and_then(|motor| motor.goal),
            arena.motors.row(two).and_then(|motor| motor.goal),
        )
    };

    let mut arena = field();
    let (first, second) = drive_both(&mut arena, target);
    assert_eq!(first, Some(target), "the first is given the cell");
    let second = second.expect("the second has a goal");
    assert!(
        second.0.abs_diff(target.0).max(second.1.abs_diff(target.1)) >= HUNT_FAN_ROWS,
        "{second:?} overlaps {target:?}"
    );
    assert!(
        (x0..=x1).contains(&second.0) && (y0..=y1).contains(&second.1),
        "fanned inside the zone: {second:?}"
    );
    assert!(
        arena
            .definition
            .terrain
            .passable_for(row, second.0, second.1),
        "fanned onto passable ground: {second:?}"
    );
    assert_eq!(
        arena.fan_goal(blue, target),
        target,
        "blue's own zone fans nothing"
    );
    assert_eq!(
        arena.far_entry_cell(blue, &[]),
        hunt_before,
        "the machine's hunt reads no motor row"
    );

    // Outside every far zone: never fanned.
    let mut arena = field();
    let open = cell_of(&arena, one)
        .map(|(x, y)| (x + 3, y))
        .expect("red is on the window");
    assert!(
        !arena
            .definition
            .entry_zones
            .iter()
            .any(|zone| zone.team != Team::Red
                && (zone.x0.min(zone.x1)..=zone.x0.max(zone.x1)).contains(&open.0)
                && (zone.y0.min(zone.y1)..=zone.y0.max(zone.y1)).contains(&open.1)),
        "the premise: {open:?} is in no far zone"
    );
    assert_eq!(drive_both(&mut arena, open), (Some(open), Some(open)));
}

/// **A pilot's motor row ages its stops on the motor's own tick** (17b cut
/// M1a.4; the M1a plan's §4 D2): a row no reflex decision runs is swept by
/// `motor_step`, so a remembered stop thirty seconds old and the refusal it
/// carried are gone at the next `MotorTick`, as a machine's are at its tick.
#[test]
fn m1a4_a_pilots_motor_row_ages_its_stops_on_the_tick() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    let tick = arena
        .due
        .iter()
        .find(|event| event.kind == DueKind::MotorTick { unit: red })
        .map(|event| event.t_ms)
        .expect("the goal put the hull on the motor's cadence");
    let ttl = tacsim::arena::motor::REFLEX_AVOID_TTL_MS;
    {
        let Some(motor) = arena.motors.row_mut(red) else {
            panic!("the pilot's row");
        };
        motor.avoid = vec![((1, 1), tick - ttl)];
        motor.refused = Some(Refusal {
            at: (1, 1),
            cell: (1, 2),
            cause: BlockedCause::Impassable,
            goal: to,
            until_ms: tick,
        });
    }
    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &arena.arena_cfg.tempo,
        arena.cell_cm(),
        arena.max_speed_cms(),
    ));
    let mut wall = 0_i64;
    while arena.now_ms < tick {
        wall += substep;
        let _ = arena.step(wall, &[], &[]);
    }
    let motor = arena.motors.row(red).cloned().expect("the pilot's row");
    assert!(
        motor.avoid.is_empty(),
        "the old stop aged: {:?}",
        motor.avoid
    );
    assert_eq!(motor.refused, None, "and took its refusal with it");
    assert_eq!(motor.goal, Some(to), "the drive goes on");
}

/// **A goal is driven at its landing millisecond** (17b cut M1a.4; doc 17 §4
/// D-UM-10: *when a `Goal` lands, the motor runs one `follow` at the landing
/// millisecond, so a new goal takes effect at N+1 and does not wait for the
/// next motor tick*). A pilot's powered tank is given a goal twenty cells along
/// the road in the opening pulse, which ends at the millisecond it began: the
/// hull's first `MotorTick` is still on the heap, inside the first `decide_ms`
/// (so it has not run), and yet the motor has already searched at nought, laid
/// a route and ordered the tank a speed — which only the landing's own step
/// can have done.
#[test]
fn m1a4_a_goal_is_driven_at_its_landing_millisecond() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    assert_eq!(arena.now_ms, 0, "the opening pulse ends where it began");
    let ticks: Vec<i64> = arena
        .due
        .iter()
        .filter(|event| event.kind == DueKind::MotorTick { unit: red })
        .map(|event| event.t_ms)
        .collect();
    let decide_ms = i64::from(arena.arena_cfg.reflex.decide_ms);
    assert!(
        matches!(ticks.as_slice(), [tick] if *tick < decide_ms),
        "the first motor tick has not run: {ticks:?}"
    );
    let motor = arena.motors.row(red).cloned().expect("the pilot's row");
    assert_eq!(motor.goal, Some(to));
    assert_eq!(motor.searched_ms, Some(0), "searched at the landing");
    assert!(!motor.route.is_empty(), "and a route laid");
    assert!(
        arena
            .unit(red)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "and the tank ordered on"
    );
}

/// **An engine killed under a `DRIVE` lets go of the goal and says so** (17b
/// cut M1a.4). A motion kill leaves no wreck, the pilot cannot let go of the
/// goal (the cockpit's and the arena's motion gates refuse `DRIVE OFF` on a
/// tank that cannot move), and the motor could never reach it — so the next
/// `MotorTick` drops it, the pilot is told `drive_no_route` once, and the
/// cadence ends with it rather than driving a dead hull every `decide_ms` for
/// the rest of the match.
#[test]
fn m1a4_an_engine_killed_under_a_drive_lets_go_and_says_so() {
    use tacsim::arena::intents::{Intent, IntentKind, MotorGoal};
    use tacsim::arena::perception::FrameNotice;
    use tacsim::arena::state::{Channel, DueKind};

    let world = booted_world();
    let mut arena = shipped_arena(&world, &[&[Muster::piloted(Team::Red, 1)]]);
    let red = tank(&world, Team::Red, 1);
    let _ = arena.seat_pilot(M1A4_PILOT, red, arena.epochs.current(red, Channel::Helm));
    if let Some(state) = arena.unit_mut(red) {
        state.power = Power::Online;
    }
    let from = cell_of(&arena, red).expect("on the window");
    let to = (from.0 + 20, from.1);
    let _ = arena.step(
        0,
        &[],
        &[Intent {
            unit: red,
            channel: Channel::Helm,
            epoch: arena.epochs.current(red, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell { x: to.0, y: to.1 }),
            seq: 1,
        }],
    );
    let motor_ticks = |arena: &Arena| {
        arena
            .due
            .iter()
            .filter(|event| event.kind == DueKind::MotorTick { unit: red })
            .count()
    };
    assert_eq!(motor_ticks(&arena), 1, "driving");
    if let Some(state) = arena.unit_mut(red) {
        state.limits.motion_disabled = true;
    }
    let mut told: Vec<&'static str> = Vec::new();
    let mut wall = 0_i64;
    while arena.now_ms < 10_000 {
        wall += 500;
        let out = arena.step(wall, &[], &[]);
        for (_, frame) in &out.frames {
            for notice in &frame.notices {
                if let FrameNotice::Drive { .. } = notice {
                    told.push(notice.kind().as_str());
                }
            }
        }
    }
    assert_eq!(told, ["drive_no_route"], "said once");
    assert_eq!(arena.motors.row(red).and_then(|motor| motor.goal), None);
    assert_eq!(motor_ticks(&arena), 0, "the cadence ended with the goal");
}
