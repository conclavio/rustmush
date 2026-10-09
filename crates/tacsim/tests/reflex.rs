//! **The reflex controller, through `Arena::step`** (16b cut N.1, N1/N2; doc 16
//! §6 "AI", §3.4, §3.5, §3.8 "Disconnects"; 16b §8's cut N row).
//!
//! Eleven tests, driving the library directly — no host, no store, no task, the
//! same shape `arena.rs`, `perception.rs`, `weapons.rs` and `resolution.rs` take.
//! One per edge of doc 16 §6's machine, plus the two properties the cut is
//! measured by:
//!
//! - **(a)** `Startup → Patrol`: a cold AI tank starts itself up, comes online and
//!   walks the two-node waypoint graph, node to node;
//! - **(b)** `Patrol → Approach`: a classified hostile out of weapons reach is
//!   closed on, and the goal is the stand-off cell and not the target's;
//! - **(c)** `Approach → Engage`: in the band with a line, it locks and fires, and
//!   both lines are in the black box;
//! - **(d)** `→ Withdraw`: armour and internals under
//!   `arena.reflex.withdraw_permille` and it breaks off toward its **own** entry
//!   zone, holding its fire;
//! - **(e)** `→ Disabled`: a wreck orders nothing at all;
//! - **(f)** the `blocked` edge: a tank held against an impassable band keeps or
//!   re-plans its route and never gives up (16b cut AI.1: a stop beside the route
//!   keeps it — the band's gap is now crossed without a stop, and the stop
//!   itself, with the route kept, is `M44C_BESIDE`'s in `tests/motor.rs`'s
//!   `m44c_ai1_the_orbit_the_corner_and_a_stop_beside_the_route_are_one_log_at_three_cadences`);
//! - **(g)** the `lost` edge: a contact that expires drops it back to `Patrol`;
//! - **(h)** the `unreachable` edge: a goal behind a sealed wall falls back to the
//!   nearest waypoint, and the search is counted as a failure;
//! - **(i)** the **path budget**: `arena.reflex.max_path_queries` per
//!   `decide_ms` window of arena time is a bound the arena applies, and a unit
//!   refused by it keeps the path it has;
//! - **(j)** the **cross-tempo property with two reflex units**: one event list and
//!   one per-substep hash sequence at 5,000, 1,000 and 500 ms;
//! - **(k)** doc 16 §3.8's released seat: the tank goes to the controller in
//!   `Withdraw` and drives itself home.
//!
//! **17b cut M4.4: the machine is the unit mind.** Every edge above is now
//! asserted on a `Controller::Mind` flown by Standard Doctrine (the `t1_*`
//! tests, each doc saying what moved and why); the fixtures that were about the
//! drive and not the machine — (f), the ford, the column, the brake, AI.1 —
//! moved to `tests/motor.rs` under a scripted mind's goals; the edges a stock
//! mind cannot reach yet stay here on the reflex, untouched, as STOPs (A to G,
//! named in the doc of the `t1_*` port beside them or in the cut's record),
//! until the cut that decides them.
//!
//! Since 17b cut M1b.0 the (j) field's every ledger line is pinned with its
//! hash as well, as it was at `e23694b0` (`m1b0_the_production_line_hashes_do_not_move`):
//! the reflex shots' half of the cut's production line-hash pin.
//!
//! # Where a test reaches into the arena's own state
//!
//! The arena's fields are public (see `arena/state.rs`), and three of these use
//! that deliberately: **(d)** applies one `mush_tactics::UnitMutationPlan` to take
//! a tank's armour down, because the claim under test is *what the controller does
//! at a health figure* and not how it got there; **(g)** and **(h)** write the
//! shadow contact store, because the claim is about the machine's edges and not
//! about how the sensor pass minted a contact — that is `perception.rs`'s test.

mod support;

use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit, UnitMutationOp,
    UnitMutationPlan, UnitSpec,
};
use smol_str::SmolStr;
use support::rig::*;
use support::scripted::{Kept, Stock, think_at};
use tacsim::arena::clock::Tempo;
use tacsim::arena::doctrine::MachineState;
use tacsim::arena::intents::{Control, ControlState};
use tacsim::arena::rng::draw_contact_id;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, Controller, CustodyOut,
    CustodyUnit, EntryZone, Pilot, Power, ShadowContact, Team, TerrainCell, TrackedRules, Waypoint,
    Waypoints, WeaponSummary,
};
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, ReflexConfig, SensorsConfig};

/// **Step 500 ms at a time with every think settled** (17b cut M4.4): the
/// rig's `run` over [`Arena::step_settled`], so a think the pulse's last
/// substep collected is written before anything is read between two pulses
/// (M4.1's gate) — what the startup, approach, engage, health, wreck, expiry
/// and pocket fixtures below step with.
fn run_settled(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> tacsim::arena::motor::MotorCounters {
    let mut wall = arena.now_ms;
    let mut total = tacsim::arena::motor::MotorCounters::default();
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        total.absorb(&out.motor);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    total
}

/// **Step 500 ms at a time until the machine flying `unit` reads `state`**
/// (17b cut M4.4: `e4_run_until_state` keyed on `Arena::machine_state` — a
/// mind's cached doctrine state, written at its landing), answering whether
/// it got there inside `limit_ms` of arena time. The pulses are read as they
/// go, for the reason `e4_run_until_state` gives.
fn t1_run_until_machine_state(
    arena: &mut Arena,
    unit: ObjId,
    state: MachineState,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> bool {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        if machine_state_of(arena, unit) == Some(state) {
            return true;
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    false
}

/// **The stock memory a kept think answered** (17b cut M4.4): the recorder's
/// `ThinkResult::memory` decoded as Standard Doctrine's keys — what the
/// landing wrote on the row, read off the answer itself.
fn kept_memory(kept: &Kept) -> tacsim::arena::stock_mind::StockMemory {
    tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
        &kept.3.memory,
    ))
}

/// **17b cut M4.4 — (a) `Startup → Patrol`, on the unit mind.** A cold stock
/// mind starts itself up, comes online and walks the two-node graph node to
/// node; every claim of the reflex fixture is kept. What moved is the clock:
/// a mind thinks every `STOCK_THINK_MS` at `think_at(RED, n)` — 482, 1,482,
/// … — and acts at the landing one substep (250 ms) on, where the reflex
/// ticked every 500 ms and acted at the tick. So think 482 lands `Startup`
/// at 732, power is `Online` at 1,232 (`match.startup_ms` 500), think 1,482
/// finds it online and lands `Goal(Cell (1, 8))` at 1,732, and the first
/// read is at 1,750 — the reflex's own 18 ms margin, kept: the goal's first
/// `follow` ran at the landing off a pose on the row, and the next MotorTick
/// (on the 500 ms grid, at 1,982) has not yet re-plotted off a pose north of
/// it, which is what would move the heading off due west. The state, the
/// walk's nodes and the goal are read off the memory (`stock_memory_of`);
/// the route, the speed and the heading off the motor row and the hull, as
/// before. The 40 s read is the reflex's claim unchanged — `west` reached,
/// bound for `east` — the mind reading its arrival off `drive_arrived` at
/// most one think late, which the window has room for.
#[test]
fn t1_a_cold_stock_mind_starts_itself_up_and_then_walks_the_waypoint_graph() {
    // One mind's tank, nobody else on the window: nothing to detect, nothing to
    // approach, so what is left is doc 16 §6's first two states.
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Startup));
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Off));
    assert!(
        arena.minds.contains_key(&RED),
        "a mind's row from the check-out"
    );

    let mut bytes: Vec<u8> = Vec::new();
    // The timings in the doc above are this stagger's and this substep's.
    assert_eq!(think_at(RED, 0), 482, "the timings above are this offset's");
    assert_eq!(
        arena.substep_len_ms(),
        250,
        "and a landing stands one of these off its think"
    );
    run_settled(&mut arena, 1_500, &mut bytes);
    let out = arena.step_settled(1_750, &[], &[]);
    absorb(&out, &mut bytes);
    assert_eq!(arena.now_ms, 1_750, "the read is 18 ms after the landing");
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Online));
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "startup"),
        1,
        "the mind started it up once, not once a think"
    );
    assert_eq!(
        log.iter()
            .find(|event| event.kind.as_str() == "startup")
            .map(|event| event.t_ms),
        Some(732),
        "at the first think's landing"
    );

    // It is making for the nearest node — `west`, at (1, 8), four cells away — and
    // it has a route and an order to drive.
    assert!(arena.minds.contains_key(&RED), "a mind");
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Patrol);
    assert_eq!(memory.goal, Some((1, 8)));
    assert_eq!(memory.waypoint, Some(0));
    let drive = motor(&arena, RED);
    assert_eq!(drive.goal, Some((1, 8)));
    assert!(!drive.route.is_empty(), "a patrol with a goal has a route");
    assert_eq!(
        arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(1_000),
        "it was ordered to make way"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(270_000),
        "west is 270 degrees"
    );

    // Back onto the 500 ms pulse grid, then given the time to get there: it
    // arrives and takes the graph's other edge.
    let out = arena.step_settled(2_000, &[], &[]);
    absorb(&out, &mut bytes);
    run_settled(&mut arena, 40_000, &mut bytes);
    assert!(arena.minds.contains_key(&RED), "a mind");
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        (memory.waypoint, memory.from_waypoint),
        (Some(1), Some(0)),
        "it reached `west` and set off for `east`: {memory:?}"
    );
    assert_eq!(memory.goal, Some((14, 8)), "{memory:?}");
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
}

// ---------------------------------------------------------------------------
// (b) `Patrol -> Approach`
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (b) `Patrol → Approach`, on the unit mind.** A classified
/// hostile out of weapons reach is closed on, and the goal is the stand-off
/// cell and not the target's — every claim kept, the hull's machine now the
/// stock mind on an UNPINNED field (this file's `arena_of`, whose tempo is
/// live; `tests/minds.rs`' field pins Quiet, so no mind fixture pins `Alert`
/// off a mind's `Approach`). What moved: the read is at 6,000 rather than
/// 5,000 — the mind's think after the classification lands one think period
/// plus a substep later than the reflex's tick did — and the state, the
/// target and the goal are the memory's (the target a contact id, the blue
/// tank's own through `arena.contacts.id_of`), beside the motor row's goal.
/// The `Alert` trigger reads the flip's accessor: `Arena::machine_state` is
/// what the tempo's `ai_approaching` reads for a mind (17b cut M4.3), off
/// the state its last think wrote.
#[test]
fn t1_a_classified_hostile_out_of_reach_is_closed_on_to_the_stand_off_cell() {
    // The red tank carries a two-cell carbine and the blue one is seven cells
    // away, so the hostile is held, classified and **out of band**: doc 16 §6's
    // `Approach`.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![carbine()],
                    Controller::Mind,
                ),
                14,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(13, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    // Online, detected, and classified: `sensors.classify_after_ms` is 3,000 —
    // and the think after it, landed.
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Approach));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Approach);
    assert!(memory.target.is_some(), "a target: {memory:?}");
    assert_eq!(
        memory.target,
        arena.contacts.id_of(RED, BLUE),
        "the target is the blue tank's own contact"
    );
    let drive = motor(&arena, RED);
    // The goal is the stand-off cell, measured from the **plot** back along the
    // line — not the target's own cell.
    //
    // **16b cut E.1** re-priced it and this fixture is its degenerate row, which
    // is why the cell is pinned exactly: the carbine's one band is 2,000 cm and
    // the sight is fourteen cells, so `fight_inner_cm` is 2,000 and `engage_cm`
    // is 1,400 — less than `STAND_OFF_MARGIN_CELLS`' three cells, so
    // `stand_off_want` floors at one cell and the goal is the cell one thousand
    // centimetres short of the plot. A gun shorter than the margin cannot have a
    // stand-off band, and the floor is what keeps the goal in front of the
    // machine rather than behind it (16c `B-05`; `stand_off_want`'s own table
    // pins the arithmetic).
    assert_ne!(
        memory.goal,
        Some((13, 8)),
        "it does not drive onto the target"
    );
    assert_eq!(
        memory.goal,
        Some((12, 8)),
        "the stand-off cell is one cell short of the plot's own"
    );
    assert_eq!(drive.goal, Some((12, 8)), "and the motor row holds it");
    // And doc 16 §3.1's `Alert` trigger *a controller has entered `Approach`* is
    // live at last — read for a mind off its cached doctrine state.
    assert_eq!(
        arena.machine_state(RED),
        Some(MachineState::Approach),
        "the tempo's `ai_approaching` reads a mind through `Arena::machine_state`"
    );
    assert!(arena.clock.tempo() >= Tempo::Alert);
}

// ---------------------------------------------------------------------------
// (c) `Approach -> Engage`
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (c) `Approach → Engage`, on the unit mind.** In the band
/// with a line it locks and fires, and both lines are in the black box —
/// every claim kept: the premise (the platform's fighting reach is its scope,
/// not the gun's 2.5 km), the tanks inside `engage_cm` at 6,000, `Engage`,
/// shots, exactly one `locked` line (the stock mind's `fresh_lock`, not one a
/// think), at least one `fired`, the hull holding the band at speed 0, at
/// most two shots in six seconds of a four-second cycle (a mind's slot fires
/// at `WeaponReady`, so a `Cycling` refusal is never staged), and doc 16
/// §3.1's `Engaged` tempo off the arena's `hostile_in_weapons_range`, raised
/// here by a MIND's shot through `Arena::step` (every other mind field pins
/// `Quiet`). The memory reads `Engage` with the blue tank's contact as its
/// target. **Two fixture constants re-derived against the view (the
/// `range_round` divergence):** the sight is ten cells, not the rig's five —
/// a contact's range reaches a think rounded to 50 m and floored at one step,
/// so a five-cell sight's 3,500 cm `engage_cm` is under anything the view
/// can say and `Engage` is never entered (measured: every think reads
/// `range_m: 50`, `Approach` to the end) — and the blue tank presents every
/// band ([`presenting`]): a mind's slot fires only at odds, and a default
/// profile presents none. The premise reads 10,000 for the scope's ten cells,
/// so the band is [`ENGAGE_CM`], 7,000 cm.
#[test]
fn t1_in_band_with_a_line_it_locks_and_fires_and_both_lines_are_in_the_black_box() {
    let mut arena = arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    // The premise: the platform's fighting reach is its scope, and the two tanks
    // are inside seven tenths of it.
    let reach = arena.unit(RED).map(|unit| unit.fight_inner_cm(CELL_CM));
    assert_eq!(
        reach,
        Some(REACH_CM.unsigned_abs()),
        "ten cells of scope, not the gun's 2.5 km"
    );
    let counters = run_settled(&mut arena, 6_000, &mut bytes);
    assert!(
        plot_range(&arena, RED, BLUE) <= ENGAGE_CM,
        "the tanks are inside `engage_cm`, which is what makes this the in-band case: {} cm",
        plot_range(&arena, RED, BLUE)
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Engage);
    assert_eq!(
        memory.target,
        arena.contacts.id_of(RED, BLUE),
        "the target is the blue tank's own contact"
    );
    assert!(counters.shots > 0, "{counters:?}");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "locked"), 1, "one lock, not one a think");
    assert!(count(&log, "fired") > 0);
    // It holds the band rather than driving on to it.
    assert_eq!(arena.unit(RED).map(|unit| unit.desired_speed_cms), Some(0));
    // The cycle is four seconds, so six seconds allow at most two.
    let fired = count(&log, "fired");
    assert!(
        fired <= 2,
        "six seconds of cycle allows at most two: {fired}"
    );
    // Doc 16 §3.1's `Engaged` tempo, on a shot and on the third trigger both.
    assert!(arena.hostile_in_weapons_range());
    assert_eq!(arena.clock.tempo(), Tempo::Engaged);
}

// ---------------------------------------------------------------------------
// (d) `-> Withdraw`
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (d) `→ Withdraw`, on the unit mind.** Armour and
/// internals under `withdraw_permille` and it breaks off toward its OWN entry
/// zone, holding its fire — every claim kept: `Engage` by 6,000; the health
/// edge (`set_health(RED, 1, 2)`: three of eighteen points, 166 per mille,
/// under the stock 350 and under `stand_min_permille` 175); the contact gone
/// by 12,000; and at 24,000 `Withdraw` with the target given up, the goal in
/// the red zone (`entry_cell` of its own team, never the blue one it was
/// closing on), no new `fired` line (`withdraw_stand` admits nothing at 166
/// and `arming_for` disarms) and the hull west of where it fought. What
/// moved: the health is read as the mind reads it —
/// `stock_mind::health_permille` over the VIEW of a kept think (the `Stock`
/// recorder) — 1,000 on the last think before the wound and 166 on the first
/// think after it, which is up to one think period late and inside every
/// window here; the state, the target and the goal are the memory's (the
/// motor row forgets a goal it reached; the memory keeps it as the reflex
/// row did). 24,000 is confirmed on this run to be inside the rally (the
/// quiet clock starts at the first think home with nothing held, `rally_ms`
/// is 20,000) and past the arrival. **One fixture constant re-derived: the
/// sight is ten cells (`seeing(.., 10)`, as 17b cut M4.3's `j_field` fields
/// its minds), not the rig's five.** The view publishes a contact's range
/// rounded to `CONTACT_RANGE_STEP_M` and floored at one step
/// (`view::range_m_rounded`, 50 m), so on a five-cell sight the stock mind's
/// `engage_cm` (3,500 cm) lies under anything its view can say and the
/// `Engage` this test starts from is unreachable (the `range_round`
/// divergence; measured: every think reads `range_m: 50`, `Approach` to the
/// end). Nothing this test claims pins the sight; ten cells puts `engage_cm`
/// at 7,000 and the blue tank's ridge at (15, 15) is still outside it.
#[test]
fn t1_under_the_health_threshold_it_breaks_off_toward_its_own_entry_zone() {
    let mut arena = arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                10,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    let kept = stock.kept();
    let last = kept.last().expect("a think before the wound");
    assert_eq!(
        tacsim::arena::stock_mind::health_permille(&last.2),
        1_000,
        "the view the mind last read: whole"
    );
    let thought = kept.len();

    // Eighteen points of armour and internals; three left is 166 per mille, under
    // the stock 350. See this file's head for why the plan is applied here.
    set_health(&mut arena, RED, 1, 2);

    // **16b cut Q.7**: the hostile has to be *gone* for this test's claim to be
    // about the health threshold at all. A withdrawing tank that still holds a
    // classified hostile inside its reach with a line stands and shoots back —
    // that is the cut's own `stand` edge, and it has its own test below. So the
    // blue tank drives over the ridge first (the ten-cell sight has an outside
    // on a sixteen-cell window: (15, 15) is past it from wherever the machine
    // stands to fight) and the contact expires before the damage lands.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    // Where it stood when the damage landed, so "it drove west" is measured from
    // the engagement and not from wherever the run home had already reached.
    let west_of = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    run_settled(&mut arena, 12_000, &mut bytes);
    assert_eq!(
        arena.contacts.of(RED).count(),
        0,
        "the contact expired, so what is left is the health edge"
    );
    // The first think after the wound read it off its view.
    let kept = stock.kept();
    assert_eq!(
        kept.get(thought)
            .map(|think| tacsim::arena::stock_mind::health_permille(&think.2)),
        Some(166),
        "the next think's view carries the wound"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");
    // **16b cut E.1** shortened the horizon from 32,000 ms, and the reason is the
    // behaviour under test working: at 166 per mille this tank is under
    // `stand_min_permille`, so it no longer stops for three seconds to shoot at a
    // contact it is about to lose — it turns and drives, arrives home sooner, and
    // its `rally_ms` of quiet therefore runs out sooner. Twenty-four seconds is
    // comfortably inside the rally and comfortably past the arrival, which is the
    // window this test's claims are about.
    run_settled(&mut arena, 24_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    // It is making for its **own** entry zone — the red one, columns 0 and 1 —
    // and not the blue one it was closing on.
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Withdraw);
    assert!(
        memory.goal.is_some_and(|(x, _)| x <= 1),
        "the goal is in the red entry zone: {:?}",
        memory.goal
    );
    assert_eq!(memory.target, None, "it gave the target up");
    assert_eq!(
        count(&events(&mut arena, &mut bytes), "fired"),
        before,
        "a withdrawing tank holds its fire"
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < west_of),
        "it drove west"
    );
}

// ---------------------------------------------------------------------------
// (e) `-> Disabled`
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (e) `→ Disabled`, on the unit mind.** A wreck orders
/// nothing at all, and an inert hull reads `Disabled`. **(1) The wreck**:
/// after it patrols, the hull is wrecked by hand as before, and from then on
/// nothing of the mind's or the motor's is on the heap (no `MindThink`, no
/// `MindLand`, no `MotorTick`), it thinks nothing (`thinks` 0, the recorder
/// never called again), lands nothing (`landings` 0), stages no intent and
/// fires no shot; `Arena::machine_state` reads `Disabled` off the hull flag
/// (tests/minds.rs' `m1a5_a_wreck_cancels_its_minds_thinks_and_landing`
/// edge, re-read on the stock mind through `Arena::step`). What moved: a
/// mind never WRITES `Disabled` for a wreck (`collect_think` and `mind_land`
/// gate on `!limits.wreck`; `Arena::machine_state` derives it off the hull
/// flag since 17b cut M4.3), so
/// the state is the arena's reading and the target is the memory's; the
/// motor row keeps what the last tick left (nothing drives it: the tick is
/// not run for a wreck and not rescheduled) — **STOP G**: the original's
/// `goal == None` and empty route were the reflex's own reset on its
/// `Disabled` edge, which nothing does for a mind, so they stay on the
/// reflex in `a_wreck_and_a_disabled_tank_order_nothing_at_all`, untouched,
/// until 17b cut M4.6 decides where a wreck's motor row is forgotten. **(2)
/// The reflex's hand-set
/// `disabled = true` half is unreachable as written** — `UnitState::disabled`
/// is not on the view; the stock mind's `Disabled` edge reads
/// `wreck || (immobile && weapons.is_empty())` — so it is reached through the
/// production route instead: a hull fielded UNARMED (the cannon must go)
/// whose motion is killed (`limits.motion_disabled`, what
/// `Consequence::DisableMotion` writes; `Arena::settle_disabled` then marks
/// it disabled). The next think's answer is read off the recorder: the
/// memory `state: Disabled` with every other key defaulted, and no intent.
#[test]
fn t1_a_wreck_thinks_nothing_and_an_inert_hull_reads_disabled() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    assert!(
        motor(&arena, RED).goal.is_some(),
        "the precondition: the walk's goal is on the motor row"
    );

    if let Some(state) = arena.unit_mut(RED) {
        state.limits.wreck = true;
        state.desired_speed_cms = 0;
        state.speed_cms = 0;
    }
    let asked = stock.kept().len();
    let counters = run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Disabled));
    assert_eq!(counters.thinks, 0, "a wreck thinks nothing: {counters:?}");
    assert_eq!(counters.landings, 0, "and lands nothing: {counters:?}");
    assert_eq!(stock.kept().len(), asked, "the runner was not called");
    assert_eq!(
        counters.intents, 0,
        "a wreck is not the subject of an order"
    );
    assert_eq!(counters.shots, 0);
    let kinds: Vec<String> = arena
        .due
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                tacsim::arena::state::DueKind::MindThink { unit, .. }
                    | tacsim::arena::state::DueKind::MindLand { unit, .. }
                    | tacsim::arena::state::DueKind::MotorTick { unit }
                    if unit == RED
            )
        })
        .map(|event| format!("{:?}", event.kind))
        .collect();
    assert!(kinds.is_empty(), "the wreck's due kinds: {kinds:?}");
    assert_eq!(stock_memory_of(&arena, RED).target, None);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "nothing ordered it to make way"
    );

    // The same for a hull that is merely disabled: fielded unarmed, its motion
    // killed the way a consequence kills it, and then `settle_disabled`'s own
    // reading — no weapon and no motion — marks it. The think after reads the
    // view's `immobile` with no weapon rows and writes `Disabled`.
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    if let Some(state) = arena.unit_mut(RED) {
        state.limits.motion_disabled = true;
    }
    let now = arena.now_ms;
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.settle_disabled(RED, now, &mut journal);
    assert_eq!(journal.len(), 1, "said once: {journal:?}");
    assert_eq!(arena.unit(RED).map(|unit| unit.disabled), Some(true));
    let asked = stock.kept().len();
    let counters = run_settled(&mut arena, 4_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Disabled));
    assert_eq!(counters.intents, 0);
    let kept = stock.kept();
    let think = kept.get(asked).expect("the think after the motion died");
    assert!(think.2.immobile, "the view says it cannot move");
    assert!(think.2.weapons.is_empty(), "and carries no weapon row");
    assert!(
        think.3.intents.is_empty(),
        "an inert hull orders nothing: {:?}",
        think.3.intents
    );
    assert_eq!(
        kept_memory(think),
        tacsim::arena::stock_mind::StockMemory {
            state: MachineState::Disabled,
            ..tacsim::arena::stock_mind::StockMemory::default()
        },
        "the `Disabled` edge resets every key"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Disabled);
}

#[test]
fn a_wreck_and_a_disabled_tank_order_nothing_at_all() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Reflex,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(MachineState::Patrol));

    if let Some(state) = arena.unit_mut(RED) {
        state.limits.wreck = true;
        state.desired_speed_cms = 0;
        state.speed_cms = 0;
    }
    let counters = run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(MachineState::Disabled));
    assert_eq!(
        counters.intents, 0,
        "a wreck is not the subject of an order"
    );
    assert_eq!(counters.shots, 0);
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    let drive = motor(&arena, RED);
    assert_eq!((drive.goal, row.target), (None, None));
    assert!(drive.route.is_empty());

    // The same for a tank that is merely disabled.
    if let Some(state) = arena.unit_mut(RED) {
        state.limits.wreck = false;
        state.disabled = true;
    }
    let counters = run(&mut arena, 9_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(MachineState::Disabled));
    assert_eq!(counters.intents, 0);
}

// ---------------------------------------------------------------------------
// (g) the `lost` edge
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (g) the `lost` edge, on the unit mind.** A contact that
/// expires drops the mind back to `Patrol`, and `Patrol`'s first goal is the
/// cell it last saw the tank in (16b cut E.4's search) — every claim kept:
/// `Engage` by 6,000, `Patrol` inside 20 s of the blue tank driving over the
/// ridge (perception's TTL, the arena's), the contact gone, the target given
/// up, the goal the last plot's own cell (9, 8) and not a node, and by
/// 40,000 the plot forgotten on arrival, a node of the graph the goal again
/// and a `lost` line in the black box. What moved: the state is read per
/// pulse off `Arena::machine_state` (`t1_run_until_machine_state`, the
/// mind's cached doctrine state, written at its landing) in place of the
/// reflex-keyed `e4_run_until_state`; the target, the goal and the last plot
/// are the memory's (the motor row forgets a goal it reached; the memory
/// keeps it). The `lost` edge and `search_goal` run off the view alone
/// (stock_mind.rs); no T1 fixture reached a search before this one. **Two
/// fixture constants re-derived.** (1) The sight is eight cells
/// (`seeing(.., 8)`), not the rig's five: the view publishes a contact's
/// range rounded to `CONTACT_RANGE_STEP_M` and floored at one step
/// (`view::range_m_rounded`, 50 m), so on a five-cell sight the stock mind's
/// `engage_cm` (3,500 cm) lies under anything its view can say and the
/// `Engage` this test starts from is unreachable (the `range_round`
/// divergence; measured: every think reads `range_m: 50`, `Approach` to the
/// end); eight cells puts `engage_cm` at 5,600, over the floor, and keeps
/// the ridge at (15, 15) outside the sight from the search's cell (9, 8)
/// (9.2 cells) and from `west` — ten cells, 17b cut M4.3's `j_field` figure,
/// would re-detect it from the search's own cell and the search would end in
/// a fight, not a walk. (2) The "found nothing" read is at 24,000, not
/// 40,000: the walk's east leg brings (14, 8) within eight cells of the
/// ridge (7.2), so a 40 s read would find the blue tank re-detected and the
/// mind re-engaged — a different fact from the one this read exists for. At
/// 24,000 it has been to (9, 8), found nothing and walked on, which is
/// "given the time to get there" (measured: the think at 13,482 reads the
/// arrival, clears the plot and hands the walk `west`; `east` by 20,482;
/// re-detected at 30,482).
#[test]
fn t1_a_contact_that_expires_drops_the_mind_back_to_patrol_and_it_searches_the_last_plot() {
    let mut arena = arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                8,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);

    // The blue tank drives over the ridge: the eight-cell sight has an outside
    // on a sixteen-cell window, so the contact stops being refreshed and the
    // time-to-live takes it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the mind back to `Patrol`"
    );
    assert!(
        arena.contacts.of(RED).count() == 0,
        "the contact should have expired"
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.target, None, "{memory:?}");
    // **16b cut E.4 (16c `B-04`)**: the state is still `Patrol` and always was,
    // but the **goal** is no longer a node of the waypoint graph. It is the cell
    // the machine last saw the tank in, for `reflex.search_ms` of arena time or
    // until it gets there — `Patrol`'s own second answer. Before this cut the
    // walk took over at this tick, and its fixed cycle per unit id could lead
    // the machine directly away from the tank that had just driven over a ridge.
    assert_eq!(
        memory.goal,
        Some((9, 8)),
        "the goal after the `lost` edge is the last plot's own cell: {memory:?}"
    );
    assert_eq!(memory.last_plot.map(|(cell, _)| cell), Some((9, 8)));
    assert_eq!(
        motor(&arena, RED).goal,
        Some((9, 8)),
        "and the motor row was handed it"
    );

    // And when it has been there and found nothing, the walk has it back —
    // read on the walk's west leg (see the doc above for why not at 40 s).
    run_settled(&mut arena, 24_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        memory.last_plot, None,
        "arrival ends the search: {memory:?}"
    );
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the waypoint walk resumes: {goal:?}"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(count(&log, "lost") > 0);
    assert!(
        stock
            .kept()
            .iter()
            .any(|think| kept_memory(think).goal == Some((9, 8))),
        "a think handed the search its goal"
    );
}

// ---------------------------------------------------------------------------
// (h) the `unreachable` edge
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — (h) the `unreachable` edge, on the unit mind.** A goal
/// behind a sealed wall is told no route, and the mind falls back in memory
/// to the nearest waypoint — the claims kept: the search is counted as a
/// failure (`path_failures > 0`, under `Fallback::Notice`), the fallback is a
/// node of the graph and the machine says so (the memory's `waypoint` and
/// `from_waypoint` are exactly `fallback_waypoint`'s answer off the think's
/// own cell and the failed goal). What moved, and the one divergence: the
/// reflex's motor fell back INSIDE the tick, so its row's goal read as the
/// node; a mind's goal is dropped with a `drive_no_route` notice and the
/// NEXT think reads it (stock_mind.rs' departure (i)), performs the fallback
/// in memory, and then — still in `Approach` with the contact held — re-asks
/// the stand-off cell, so the goal it emits is `Approach`'s own and never the
/// plot's cell; `MotorRow.waypoint` is never written for a mind (the
/// `fallback` divergence, doc 17 §17). The motor row's goal is therefore not
/// the fallback node on T1 and is not read as the edge; the memory's walk is.
/// Halves mirrored in isolation (stock_mind.rs' no-route notice test,
/// motor.rs' fallback policies); this is the end-to-end pocket.
#[test]
fn t1_a_goal_behind_a_sealed_wall_is_told_no_route_and_falls_back_in_memory_to_the_nearest_waypoint()
 {
    // A three-by-three pocket, sealed: the blue tank is inside it and the red one
    // is outside, holding it classified (see this file's head).
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 8..=12_u16 {
        paint.push((x, 6, wall()));
        paint.push((x, 10, wall()));
    }
    for y in 6..=10_u16 {
        paint.push((8, y, wall()));
        paint.push((12, y, wall()));
    }
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(4, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(10, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &paint,
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 1_500, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let counters = run_settled(&mut arena, 6_000, &mut bytes);

    assert!(
        counters.path_failures > 0,
        "the sealed pocket is unreachable: {counters:?}"
    );
    assert!(arena.minds.contains_key(&RED), "a mind");
    let nodes: Vec<(u16, u16)> = arena
        .definition
        .waypoints
        .nodes()
        .iter()
        .map(|node| (node.x, node.y))
        .collect();
    let kept = stock.kept();
    // The `Approach`'s goal: the stand-off cell short of the plot, which the
    // pocket seals in — the fixture's premise.
    let (asked_at, asked) = kept
        .iter()
        .find_map(|think| {
            let memory = kept_memory(think);
            (memory.state == MachineState::Approach).then_some((think.1, memory.goal))
        })
        .expect("an `Approach` think");
    let asked = asked.expect("the `Approach` handed the motor a goal");
    assert_ne!(asked, (10, 8), "the stand-off cell is not the plot's own");
    assert!(
        (9..=11).contains(&asked.0) && (7..=9).contains(&asked.1),
        "the stand-off cell is inside the sealed pocket: {asked:?}"
    );
    // The think told no route: the view carries the motor's word, the memory
    // took the fallback's walk — the nearest node, as the reflex's motor took
    // it in the tick — and the state body re-asked `Approach`'s own goal.
    let told = kept
        .iter()
        .position(|think| {
            think.1 > asked_at
                && think.2.notices.iter().any(|notice| {
                    matches!(
                        notice,
                        tacsim::arena::perception::FrameNotice::Drive {
                            outcome: tacsim::arena::perception::DriveOutcome::NoRoute,
                        }
                    )
                })
        })
        .expect("a think was told no route");
    let before = kept_memory(&kept[told - 1]);
    let after = kept_memory(&kept[told]);
    let here = kept[told].2.cell.expect("on the window");
    let (waypoint, from_waypoint, fallback) = tacsim::arena::motor::fallback_waypoint(
        &arena.definition,
        here,
        before.goal,
        before.waypoint,
        before.from_waypoint,
        RED.0,
    );
    assert_eq!(before.goal, Some(asked), "the goal that failed: {before:?}");
    assert_eq!(
        waypoint,
        arena.definition.waypoints.nearest(here.0, here.1),
        "the fallback is the nearest node"
    );
    assert!(
        fallback.is_some_and(|cell| nodes.contains(&cell)),
        "the fallback is a node of the graph: {fallback:?} of {nodes:?}"
    );
    assert_eq!(
        (after.waypoint, after.from_waypoint),
        (waypoint, from_waypoint),
        "the memory took the fallback's walk: {after:?}"
    );
    assert!(after.waypoint.is_some());
    assert_eq!(after.state, MachineState::Approach, "{after:?}");
    let goals: Vec<(u16, u16)> = kept[told]
        .3
        .intents
        .iter()
        .filter_map(|intent| match intent {
            tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell {
                x,
                y,
            }) => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        goals,
        vec![asked],
        "the state body re-asks the stand-off cell and never the plot's: {:?}",
        kept[told].3.intents
    );
    assert_eq!(after.goal, Some(asked));
}

/// **The escalation terminates, and it keeps what it learned** (16b cut Q.7).
///
/// Two claims the test above cannot make, because it runs at [`reflex_cfg`]'s
/// `repath_ms` of zero — where a route-empty tick searches whatever happened
/// before it.
///
/// 1. **A sealed goal is not a failing search a tick.** At the shipped damper
///    the `unreachable` edge fires, the goal becomes a waypoint, and the next
///    failure is one `repath_ms` away and not one `decide_ms` away. Forty ticks
///    of arena time buy a handful of failures, not forty.
/// 2. **`avoid` survives the edge.** The tank is stopped by a parked teammate on
///    the way, learns the cell, and then fails its search: if the fallback threw
///    the list away, the escalation would cycle — stop, learn, fall back, forget,
///    stop at the same cell again — which is the "cycling wreck, fallback,
///    wreck" the cut brief names.
///
/// **17b cut M4.4 — ported onto the unit mind.** Both claims kept with the
/// same three hulls (RED2 stays a pilot's parked tank), the same doctrine and
/// the same sixteen-second re-plant loop: the escalation terminates (the
/// failures a handful, not one a decide tick) and the avoid list survives
/// the `unreachable` edge. The bound holds for a mind by the same throttle:
/// `land_goal` keeps `searched_ms` for a mind's goal, so a stand-off cell
/// re-asked every think is deferred by `repath_ms` rather than searched
/// again, and `forget_goal` keeps the row's avoid list. What moved: the
/// stops are read off the motor row as before (`note_blocked`'s list, the
/// rig's `avoided`); the fallback's node is the MEMORY's `waypoint` — a
/// mind's fallback is performed one think late in its memory and
/// `MotorRow.waypoint` is never written for a mind (the `fallback`
/// divergence) — in place of the motor row's.
#[test]
fn t1_a_sealed_goal_escalates_without_cycling_and_keeps_its_avoided_cells() {
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 8..=12_u16 {
        paint.push((x, 6, wall()));
        paint.push((x, 10, wall()));
    }
    for y in 6..=10_u16 {
        paint.push((8, y, wall()));
        paint.push((12, y, wall()));
    }
    let cfg = arena_cfg(ReflexConfig {
        decide_ms: 500,
        repath_ms: 4_000,
        ..ReflexConfig::default()
    });
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(4, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(10, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            // Parked on the way to the only node the fallback can reach: the
            // stop the machine has to remember while the escalation runs.
            tank(RED2, Team::Red, centre(3, 8), Vec::new(), Controller::Pilot),
        ],
        &paint,
        &cfg,
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    // First the stop: nothing is held yet, so the mind patrols west into the
    // parked tank and learns the cell.
    run_settled(&mut arena, 6_000, &mut bytes);
    let learned = avoided(&arena, RED);
    assert!(
        !learned.is_empty(),
        "the parked tank never stopped it, so there is nothing to keep"
    );

    // Then the escalation. The contact is re-planted every pulse: a five-cell
    // sight cannot hold a tank six cells away inside a sealed pocket, and what
    // this test is about is what an `Approach` does with a goal it cannot reach —
    // which needs the `Approach` to last (see this file's head for why a test
    // may plant one).
    let until = arena.now_ms + 16_000;
    let mut wall_ms = arena.now_ms;
    let mut counters = tacsim::arena::MotorCounters::default();
    while arena.now_ms < until {
        let now = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
        wall_ms += 500;
    }

    assert!(
        counters.path_failures > 0,
        "the sealed pocket is unreachable: {counters:?}"
    );
    // Sixteen thinks in sixteen seconds, each re-asking the goal; the damper
    // is four.
    assert!(
        counters.path_failures < 10,
        "the escalation ran the same failing search over and over: {counters:?}"
    );
    assert_eq!(
        avoided(&arena, RED),
        learned,
        "the `unreachable` edge threw away what the stop taught it"
    );
    // The goal read at the end is `Approach`'s own and not the fallback's: the
    // state re-asks `stand_off_cell` off the contact's plot on every think,
    // which is the cut brief's own rule and what makes the bound above the
    // interesting number. What the fallback leaves behind is the node it
    // stepped to — in the mind's memory.
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Approach, "{memory:?}");
    assert!(
        memory.waypoint.is_some(),
        "the fallback never took: {memory:?}"
    );
}

// ---------------------------------------------------------------------------
// (j) the cross-tempo property, with two reflex units
// ---------------------------------------------------------------------------

/// One unit as a replay ends: where it is, how fast, and what its machine was
/// doing — the state the hash folds plus the two fields it deliberately does not
/// (`speed_cms` is in the hash; the goal is what the machine *decided*).
type Final = (ObjId, i64, i64, u32, i32, MachineState, Option<(u16, u16)>);

/// What one replay produced (the shape `arena.rs` uses).
struct Replay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<Final>,
    /// How many pulses it took — **16b cut E.4**'s guard, the shape
    /// `perception.rs`'s replay carries: three cadences really are three
    /// different numbers of pulses, so an equality of the three is a property
    /// and not three copies of one run.
    pulses: u32,
}

/// **The (j) field**: two machines, `RED` armed with the cannon and `RED2`
/// with nothing, and the seated `BLUE` they detect, classify, lock and shoot
/// — the tempo pinned at `Quiet` (D-16-2: the tempo decides only how often
/// the arena steps, so the replays of it differ in **cadence** and in nothing
/// else). **17b cut M4.3**: the machines are minds sealed as the production
/// fill is ([`as_fill`]) with their runner installed as production installs
/// it ([`install_standard`]) — the production shape, which is what the pins
/// over this field hold — and, as `tests/minds.rs`' two-T1 (j) field has
/// been since 17b cut M1b.2, every hull sees ten cells and the seated tank
/// presents every band ([`presenting`]): a mind's slot fires at odds, and a
/// default profile presents none, so the reflex-era field (five cells, no
/// bands) held a mind in `Approach` with no shot where the reflex machine
/// fired seven times. Declared in the cut's record with the one re-bless.
fn j_field() -> Arena {
    let mut arena = arena_of(
        vec![
            as_fill(seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                10,
            )),
            as_fill(seeing(
                tank(RED2, Team::Red, centre(6, 10), Vec::new(), Controller::Mind),
                10,
            )),
            as_fill(presenting(seeing(
                tank(
                    BLUE,
                    Team::Blue,
                    centre(9, 8),
                    Vec::new(),
                    Controller::Pilot,
                ),
                10,
            ))),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let _ = install_standard(&mut arena);
    // D-16-2: the tempo decides only how often the arena steps. It is pinned so
    // that the three replays differ in **cadence** and in nothing else.
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    arena
}

/// **The (j) field replayed at a fixed `cadence_ms` of wall time** (`wall +=
/// cadence` after every pulse) until `until_ms`: the replay
/// [`m1a1_the_moved_motor_folds_the_same_bytes`] pins its substep hashes off,
/// stepped as it was when they were blessed. **17b cut M4.4**: 16b cut U.1's
/// task driver (`Drive::Task`, waking on the 250 ms lattice at
/// `next_due_ms`), its `woke_at` and substep returns went with
/// `u1_two_reflex_units_on_the_task_s_own_wakes_are_the_fixed_cadence_replay`,
/// whose mirror on the mind is `tests/minds.rs`'
/// `m1a6_two_stock_minds_at_three_cadences_are_one_event_list_and_one_hash_sequence`
/// (the (j) field on `Mind` at `Every` and `Task` 5,000/1,000/500 under
/// `step_settled`, one event list, one hash sequence, one think sequence).
fn replay(cadence_ms: i64, until_ms: i64) -> Replay {
    let mut arena = j_field();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                // 17b cut M4.3: either machine's state (the mind's cached).
                machine_state_of(&arena, unit.id).unwrap_or(MachineState::Startup),
                motor(&arena, unit.id).goal,
            )
        })
        .collect();
    Replay {
        events,
        hashes,
        finals,
        pulses,
    }
}

/// **17b cut M1a.1's first pin: the motor moved and the hash did not** (the
/// M1a plan's head, "the production hash"; doc 17 §4, D-UM-3).
///
/// The (j) field at a fixed 1,000 ms cadence for 30 s — two reflex machines,
/// one armed, a seated tank they find and shoot — whose machines' goals,
/// routes, searches and stops all moved from the reflex rows to
/// `Arena::motors` in this cut, and whose fold reads them back in the order it
/// always did (`hash.rs`, item 3). The three numbers were read at `32a1cc23`,
/// before `arena/motor.rs` existed, off this very replay: the first and the
/// last per-substep hash and how many there were. 17b cuts M1a.4, M1a.5 and
/// M1a.6 re-assert them; a pin that moves is a defect in the move, never a
/// re-bless.
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). The first and
/// last substep hashes re-blessed here, the count unmoved:
/// `0x9da6_3882_e319_aec9` became `0x18df_dd94_73aa_5a36` and
/// `0xdb4f_9096_c79f_b530` became `0x5689_35a8_5830_609d` (the field's three
/// `mind_sealed` lines shift the ledger sequence every substep folds; its
/// tape's diff is `M1B0_J_LINES`').
///
/// **17b cut M4.3 — the flip's one production re-bless** (declared with
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, the same
/// constants): the field's two machines are minds sealed as the production
/// fill is, on the two-T1 (j) shape ([`j_field`]), so the controller byte,
/// the mind rows, the production engine's seal digests and the script's
/// digest fold where the reflex rows did: `0x18df_dd94_73aa_5a36` became
/// `0x692d_5b0e_3c04_93a6` and `0x5689_35a8_5830_609d` became
/// `0xd243_f6a1_30c0_00c7`.
///
/// **The fill decision — re-pinned, hashes only** (2026-10-09; declared with
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, the same
/// constants): the production fill is sealed `native`
/// (`PRODUCTION_FILL_ENGINE`, the owner's three ALIEN18 runs all `held=no`),
/// so the field's minds carry native seals, whose digests (the engine folded
/// in) the mind rows fold; nothing else moved — the substep count is 120 as
/// before and the field's tape ([`M1B0_J_LINES`]) is byte-equal but for its
/// `hash` values. `0x692d_5b0e_3c04_93a6` became `0xc734_ca3c_2cce_2a0c` and
/// `0xd243_f6a1_30c0_00c7` became `0xbabf_b4c3_71e8_786d`.
#[test]
fn m1a1_the_moved_motor_folds_the_same_bytes() {
    let run = replay(1_000, 30_000);
    assert_eq!(run.hashes.len(), 120, "the substeps of 30 s at 250 ms");
    assert_eq!(
        run.hashes.first().copied(),
        Some(0xc734_ca3c_2cce_2a0c),
        "the first substep's hash moved"
    );
    assert_eq!(
        run.hashes.last().copied(),
        Some(0xbabf_b4c3_71e8_786d),
        "the last substep's hash moved"
    );
}

/// **The (j) field's ledger, every line whole with its hash, recorded at
/// `e23694b0`** (17b cut M1b.0, the fixer's addition to its production
/// line-hash pin; the M1b plan's §1(a) and (c)) — off the unchanged source,
/// before the cut's due-loop edit was applied, from [`j_field`] stepped at
/// 1,000 ms for 30 s, each line as `serde_json` writes a [`LedgerEvent`]. The
/// reflex shots are here: `RED`'s `locked` and seven `fired` lines, each with
/// its `impact` and `miss`, the reflex tick's lines carrying the tick's one
/// lazy hash (`ActionHash`, 16b cut M.3), several in a substep that also ran
/// a sensor pass (the `detected` at 25,482 beside the `fired` there).
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). Re-recorded
/// here from 58 lines to 61: three `mind_sealed` lines after `match_started`
/// (the field runs to 30 s and does not end, so no close), every line's `seq`
/// and `hash` re-blessed with them and every other key byte-equal (the diff,
/// ignoring `seq` and `hash`, is the three inserted lines alone).
///
/// **17b cut M4.3 — re-recorded with the flip's one production re-bless**
/// (61 lines to 50): the field's machines are minds on the production
/// fill's seal, on the two-T1 (j) shape — every `fired` line now carries the
/// mind's `by` and slot (the first a fresh-lock citation at a landing, the
/// rest the slot's own at the cannon's `WeaponReady`), the one `locked` is a
/// mind's, `impact` lines land as `hit` on the presenting tank, and every
/// `mind_sealed` line's `source` is the script's new digest; every `hash` is
/// re-blessed under the one reason.
///
/// **The fill decision — re-recorded, hashes only** (2026-10-09; 50 lines to
/// 50): the field's minds carry the production fill's seal, now `native`, so
/// the seal digests folded into every substep's hash moved, and nothing else
/// did: compared with every `hash` taken out, the 50 lines are byte-equal to
/// the M4.3 record — the same `locked`, the same seven `fired` with their
/// `by` and slot, the same seven `impact` and `hit` lines and the one
/// `section_destroyed` at the same milliseconds, the same `mind_sealed`
/// fields (the line names no engine) — and all 50
/// `hash` values are re-blessed under that one reason.
const M1B0_J_LINES: &[&str] = &[
    r#"{"seq":1,"t_ms":0,"kind":"match_started","hash":3401558477649851151}"#,
    r#"{"seq":2,"t_ms":0,"kind":"mind_sealed","unit":4,"sortie":4,"mind":"Standard Doctrine","source":"102cc7d27eb411c4","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":3401558477649851151}"#,
    r#"{"seq":3,"t_ms":0,"kind":"mind_sealed","unit":6,"sortie":6,"mind":"Standard Doctrine","source":"102cc7d27eb411c4","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":3401558477649851151}"#,
    r#"{"seq":4,"t_ms":0,"kind":"mind_sealed","unit":9,"sortie":9,"mind":"Standard Doctrine","source":"102cc7d27eb411c4","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine","hash":3401558477649851151}"#,
    r#"{"seq":5,"t_ms":4,"kind":"detected","viewer":6,"contact":7330038967278110817,"target":4,"hash":14354320272135170572}"#,
    r#"{"seq":6,"t_ms":4,"kind":"detected","viewer":6,"contact":10007312734772483259,"target":9,"hash":14354320272135170572}"#,
    r#"{"seq":7,"t_ms":189,"kind":"detected","viewer":9,"contact":15474813515948476951,"target":4,"hash":14354320272135170572}"#,
    r#"{"seq":8,"t_ms":189,"kind":"detected","viewer":9,"contact":14127509583146488135,"target":6,"hash":14354320272135170572}"#,
    r#"{"seq":9,"t_ms":254,"kind":"startup","unit":6,"hash":13194113808842539994}"#,
    r#"{"seq":10,"t_ms":482,"kind":"detected","viewer":4,"contact":527481421542167805,"target":6,"hash":10059987601909732965}"#,
    r#"{"seq":11,"t_ms":482,"kind":"detected","viewer":4,"contact":5852833454602713067,"target":9,"hash":10059987601909732965}"#,
    r#"{"seq":12,"t_ms":732,"kind":"startup","unit":4,"hash":6231986064508145913}"#,
    r#"{"seq":13,"t_ms":754,"kind":"power_online","unit":6,"hash":15974000723241144905}"#,
    r#"{"seq":14,"t_ms":1232,"kind":"power_online","unit":4,"hash":17620887440979305236}"#,
    r#"{"seq":15,"t_ms":3004,"kind":"classified","viewer":6,"contact":7330038967278110817,"target":4,"team":"red","draws":[["classify:6:4:4",3994458990081984151]],"hash":14471382252753135496}"#,
    r#"{"seq":16,"t_ms":3004,"kind":"classified","viewer":6,"contact":10007312734772483259,"target":9,"team":"blue","draws":[["classify:6:9:4",16446071241866099752]],"hash":14471382252753135496}"#,
    r#"{"seq":17,"t_ms":3189,"kind":"classified","viewer":9,"contact":15474813515948476951,"target":4,"team":"red","draws":[["classify:9:4:4",3682179631169270960]],"hash":14471382252753135496}"#,
    r#"{"seq":18,"t_ms":3189,"kind":"classified","viewer":9,"contact":14127509583146488135,"target":6,"team":"red","draws":[["classify:9:6:4",28171549963284800]],"hash":14471382252753135496}"#,
    r#"{"seq":19,"t_ms":3482,"kind":"classified","viewer":4,"contact":527481421542167805,"target":6,"team":"red","draws":[["classify:4:6:4",18412658312983768788]],"hash":745037494538193099}"#,
    r#"{"seq":20,"t_ms":3482,"kind":"classified","viewer":4,"contact":5852833454602713067,"target":9,"team":"blue","draws":[["classify:4:9:4",2838647583109313249]],"hash":745037494538193099}"#,
    r#"{"seq":21,"t_ms":3732,"kind":"locked","unit":4,"contact":5852833454602713067,"target":9,"hash":15913006082884754962}"#,
    r#"{"seq":22,"t_ms":3732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":1,"target":9,"range_cm":4116,"rounds":1,"by":"mind","slot":1,"hash":15913006082884754962}"#,
    r#"{"seq":23,"t_ms":3760,"kind":"impact","shot_seq":1,"unit":4,"target":9,"hash":17553969049737172352}"#,
    r#"{"seq":24,"t_ms":3760,"kind":"hit","shot_seq":1,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:1","draws":[["hit:1",12072725193963834385],["facet:1",2237253966278422799]],"hash":17553969049737172352}"#,
    r#"{"seq":25,"t_ms":3760,"kind":"section_destroyed","unit":9,"section":"hull","by":4,"hash":17553969049737172352}"#,
    r#"{"seq":26,"t_ms":5000,"kind":"position_summary","units":[[4,4630,8434,0,224848],[6,5000,9000,0,225000],[9,9500,8500,0,0]],"hash":10616055270688422001}"#,
    r#"{"seq":27,"t_ms":6750,"kind":"blocked","unit":6,"x":4,"y":8,"cause":"occupied","hash":17996252260018578192}"#,
    r#"{"seq":28,"t_ms":7732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":2,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":15302021202950651515}"#,
    r#"{"seq":29,"t_ms":7765,"kind":"impact","shot_seq":2,"unit":4,"target":9,"hash":10530880657644539099}"#,
    r#"{"seq":30,"t_ms":7765,"kind":"hit","shot_seq":2,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:2","draws":[["hit:2",11307540289106727687],["facet:2",4135577070013810089]],"hash":10530880657644539099}"#,
    r#"{"seq":31,"t_ms":10000,"kind":"position_summary","units":[[4,4653,8350,0,88000],[6,2956,8673,0,249288],[9,9500,8500,0,0]],"hash":14250318599224705375}"#,
    r#"{"seq":32,"t_ms":11732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":3,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":9231970805638117777}"#,
    r#"{"seq":33,"t_ms":11765,"kind":"impact","shot_seq":3,"unit":4,"target":9,"hash":3741900735261067093}"#,
    r#"{"seq":34,"t_ms":11765,"kind":"hit","shot_seq":3,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:3","draws":[["hit:3",17241375169118599602],["facet:3",10008162893314153691]],"hash":3741900735261067093}"#,
    r#"{"seq":35,"t_ms":15000,"kind":"position_summary","units":[[4,4653,8350,0,88000],[6,5627,7497,0,89847],[9,9500,8500,0,0]],"hash":12964669429928505210}"#,
    r#"{"seq":36,"t_ms":15732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":4,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":10644091025303897755}"#,
    r#"{"seq":37,"t_ms":15765,"kind":"impact","shot_seq":4,"unit":4,"target":9,"hash":17322859602011605373}"#,
    r#"{"seq":38,"t_ms":15765,"kind":"hit","shot_seq":4,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:4","draws":[["hit:4",9783297951381390390],["facet:4",8754771002850247076]],"hash":17322859602011605373}"#,
    r#"{"seq":39,"t_ms":19732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":5,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":7291660234278408159}"#,
    r#"{"seq":40,"t_ms":19765,"kind":"impact","shot_seq":5,"unit":4,"target":9,"hash":1771317520095032327}"#,
    r#"{"seq":41,"t_ms":19765,"kind":"hit","shot_seq":5,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:5","draws":[["hit:5",3759784274594068254],["facet:5",4443187530762531632]],"hash":1771317520095032327}"#,
    r#"{"seq":42,"t_ms":20000,"kind":"position_summary","units":[[4,4653,8350,0,88000],[6,10627,7500,0,90000],[9,9500,8500,0,0]],"hash":1771317520095032327}"#,
    r#"{"seq":43,"t_ms":23732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":6,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":4847157839294600788}"#,
    r#"{"seq":44,"t_ms":23765,"kind":"impact","shot_seq":6,"unit":4,"target":9,"hash":11047615940332324292}"#,
    r#"{"seq":45,"t_ms":23765,"kind":"hit","shot_seq":6,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:6","draws":[["hit:6",12808839036685962400],["facet:6",15270049831497957593]],"hash":11047615940332324292}"#,
    r#"{"seq":46,"t_ms":25000,"kind":"position_summary","units":[[4,4653,8350,0,88000],[6,12843,7339,0,295055],[9,9500,8500,0,0]],"hash":18219047020498874199}"#,
    r#"{"seq":47,"t_ms":27732,"kind":"fired","unit":4,"weapon":"cannon","shot_seq":7,"target":9,"range_cm":4849,"rounds":1,"by":"cond","slot":1,"hash":10896915975377394483}"#,
    r#"{"seq":48,"t_ms":27765,"kind":"impact","shot_seq":7,"unit":4,"target":9,"hash":16583577261773445350}"#,
    r#"{"seq":49,"t_ms":27765,"kind":"hit","shot_seq":7,"unit":4,"target":9,"section":"hull","facet":"front","armor_section":"hull","rounds":1,"damage":120,"armor_left":0,"internals_left":0,"cause":"shot:7","draws":[["hit:7",9819418404979189944],["facet:7",11750702725261985454]],"hash":16583577261773445350}"#,
    r#"{"seq":50,"t_ms":30000,"kind":"position_summary","units":[[4,4653,8350,0,88000],[6,7868,7500,0,270000],[9,9500,8500,0,0]],"hash":13456672963129014381}"#,
];

/// **17b cut M1b.0: the production line-hash pin, the reflex shots.** The (j)
/// field — two reflex machines, one armed, and the seated tank they find and
/// shoot — journals exactly the lines it journalled at `e23694b0`, each with
/// its hash; [`m1a1_the_moved_motor_folds_the_same_bytes`] pins its substep
/// hashes, this pins every line's. `tests/motor.rs` holds the shipped duel's
/// half (whose minute holds no shot) and `tests/weapons.rs` the seat's.
///
/// What a line pin can and cannot see: the action hash is not folded into
/// the state hash, and a line journalled outside an action run carries the
/// substep's hash, so a trigger that merely set `ActionHash::Pending` and
/// journalled nothing moves no byte here. A trigger that TOOK the lazy hash
/// (an `event_hash` call) for a hull with no slot moves the hash of the next
/// reflex line of its substep wherever the state changed between the two —
/// these `fired` lines are where it shows. That no trigger opens the action
/// hash at all is a claim about the trigger, asserted where a trigger is
/// built (17b cut M1b.1), not by this pin; this pin proves the production
/// bytes.
#[test]
fn m1b0_the_production_line_hashes_do_not_move() {
    let lines = j_field_lines(1_000, 30_000);
    // The pin is worth holding: the reflex machine locked, fired and hit.
    for want in [
        r#""kind":"locked""#,
        r#""kind":"fired""#,
        r#""kind":"impact""#,
    ] {
        assert!(
            lines.iter().any(|line| line.contains(want)),
            "the (j) field never journalled {want}"
        );
    }
    assert_eq!(
        lines,
        M1B0_J_LINES
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>(),
        "the (j) field's ledger lines moved"
    );
}

/// The (j) field stepped at `cadence_ms` until `until_ms`, as [`replay`] steps
/// it, answering every ledger line whole — `seq` and `hash` included — as
/// `serde_json` writes it.
fn j_field_lines(cadence_ms: i64, until_ms: i64) -> Vec<String> {
    let mut arena = j_field();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    events(&mut arena, &mut bytes)
        .iter()
        .map(|event| serde_json::to_string(event).unwrap_or_default())
        .collect()
}

/// **17b cut M3.6, cross-tempo (16b §11, mandatory) on the (j) field**: the
/// field stepped at 5,000, 1,000 and 500 ms to 30,000, then stopped
/// (`StopReason::Abort`, honoured at the next pulse's phase 1), journals one
/// event list and one hash sequence at every cadence — the opening's three
/// `mind_sealed` lines after `match_started`, the fight, then the three
/// `sortie_closed { match_end }` lines immediately before `aborted`, which is
/// the tape's last line.
#[test]
fn m3_the_j_fields_sortie_lines_are_one_list_at_three_cadences() {
    let run = |cadence_ms: i64| {
        let mut arena = j_field();
        let mut bytes: Vec<u8> = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        let mut wall = 0_i64;
        loop {
            let out = arena.step(wall, &[], &[]);
            absorb(&out, &mut bytes);
            hashes.extend(arena.substep_hashes.iter().copied());
            if arena.now_ms >= 30_000 {
                break;
            }
            wall += cadence_ms;
        }
        arena.stop(tacsim::arena::state::StopReason::Abort);
        absorb(&arena.step(wall + cadence_ms, &[], &[]), &mut bytes);
        let lines: Vec<(i64, String, u64)> = events(&mut arena, &mut bytes)
            .iter()
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.hash,
                )
            })
            .collect();
        (lines, hashes)
    };
    let (first, first_hashes) = run(5_000);
    let kinds: Vec<&str> = first
        .iter()
        .map(|(_, json, _)| json.split('"').nth(3).unwrap_or(""))
        .collect();
    assert_eq!(kinds.first().copied(), Some("match_started"));
    assert_eq!(kinds.get(1..4), Some(&["mind_sealed"; 3][..]));
    let last = kinds.len() - 1;
    assert_eq!(kinds.get(last).copied(), Some("aborted"), "the end is last");
    assert_eq!(
        kinds.get(last - 3..last),
        Some(&["sortie_closed"; 3][..]),
        "the closes immediately before it"
    );
    for (cadence, (lines, hashes)) in [1_000_i64, 500].into_iter().map(|c| (c, run(c))) {
        assert_eq!(lines, first, "the event list at {cadence} ms");
        assert_eq!(hashes, first_hashes, "the hash sequence at {cadence} ms");
    }
}

// ---------------------------------------------------------------------------
// (k) doc 16 section 3.8's released seat
// ---------------------------------------------------------------------------

/// **17b cut M4.3 — ported onto the mind** (the plan's first of three
/// released-tank fixtures): a `Released` hands the hull to its mind seeded
/// `Withdraw` (`seed_withdraw`), which drives it home as the reflex machine
/// did and rallies on Q.7's clock; a reclaim keeps the row dormant (17b cut
/// M3.5) and takes the motor row back.
///
/// **17b cut M4.5a** (Q24): the three reflex-table probes, vacuous on a
/// pilot's or a mind's hull since 17b cut M4.3, read the machine instead — a
/// pilot's tank has no machine state, the released hull has a mind row, and
/// the reclaimed one's `None` machine state is asserted where it was — and the
/// reclaim's heap check names only the mind's due kinds (`ReflexTick` goes
/// with the controller).
#[test]
fn t1_a_released_seat_hands_the_tank_to_its_mind_in_withdraw() {
    // A pilot's tank, running, out in the middle of the window. Its seat is
    // released past the linkdead grace — doc 16 §3.8's `Control { Released }` —
    // and its mind, seeded `Withdraw`, takes it home.
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Pilot,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[RED],
    );
    let mut bytes: Vec<u8> = Vec::new();
    // The pilot brings it online and drives it east, the ordinary way.
    let out = arena.step(
        0,
        &[],
        &[tacsim::arena::intents::Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch: 1,
            kind: tacsim::arena::intents::IntentKind::Startup,
            seq: 1,
        }],
    );
    absorb(&out, &mut bytes);
    run(&mut arena, 1_500, &mut bytes);
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Online));
    assert_eq!(
        arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        None,
        "a pilot's tank has no machine"
    );

    // The seat goes. The world raises the epoch as it always does.
    let now = arena.now_ms;
    let out = arena.step(
        now,
        &[Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: None,
        }],
        &[],
    );
    absorb(&out, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind)
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert!(
        arena.minds.contains_key(&RED),
        "a mind row: the machine is a mind"
    );
    assert!(
        !stock_memory_of(&arena, RED).ordered,
        "a release's Withdraw is nobody's order"
    );

    // And it drives itself home, on the epoch the world published — not on one of
    // its own.
    let start = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    run(&mut arena, 16_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert!(arena.minds.contains_key(&RED), "a mind");
    // The mind's goal is in its memory (the motor row forgets a goal it has
    // reached; the reflex row kept it).
    let goal = stock_memory_of(&arena, RED).goal;
    assert!(
        goal.is_some_and(|(x, _)| x <= 1),
        "the goal is the red entry zone: {goal:?}"
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < start),
        "it drove west"
    );
    // **16b cut Q.7**: and then it comes back. A seat released past the linkdead
    // grace used to leave a tank parked in the entry zone for the rest of the
    // match; after `arena.reflex.rally_ms` of quiet it is an AI teammate again.
    run(&mut arena, 60_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    assert!(stock_memory_of(&arena, RED).rallied, "it rallied once");

    // A reclaimed seat takes it back; the mind's row is kept dormant (17b cut
    // M3.5) and its motor row goes with the seat (17b cut M1a.1: the row is
    // what the motor's readers gate on, so a stale one would hold, or
    // remember stops for, the pilot's tank).
    assert!(
        arena.motors.row(RED).is_some(),
        "the mind drove from a motor row"
    );
    let now = arena.now_ms;
    let out = arena.step(
        now,
        &[Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 3,
            state: ControlState::Live,
            pilot: None,
        }],
        &[],
    );
    absorb(&out, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
    assert!(
        arena.minds.contains_key(&RED),
        "the mind's row is kept dormant (17b cut M3.5)"
    );
    assert_eq!(machine_state_of(&arena, RED), None, "a pilot's hull");
    assert!(
        arena.motors.row(RED).is_none(),
        "the reclaimed seat took the motor row back"
    );
    assert!(
        !arena.due.iter().any(|event| matches!(
            event.kind,
            tacsim::arena::state::DueKind::MindThink { .. }
                | tacsim::arena::state::DueKind::MindLand { .. }
        )),
        "the reclaimed seat took its thinks off the heap"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — a dry hull withdraws (the blocked edge and the follower's
// refused edge are `tests/motor.rs`'s since 17b cut M4.4)
// ---------------------------------------------------------------------------

/// **A dry hull withdraws to its entry zone** — the controller half of 16b
/// cut Q.7's `the_follower_holds_rather_than_drive_at_a_bluff`, ported onto
/// the unit mind by 17b cut M4.4 (its motor half, the follower's one-cell
/// test at the bluff, is `tests/motor.rs`'
/// `m44a_the_follower_holds_rather_than_drive_at_a_bluff`, under a scripted
/// goal). A mind's hull that came in with a cannon and no rounds is DRY
/// (`stock_mind::is_dry` over the view's weapon rows), and Standard
/// Doctrine's first online think takes the `Withdraw` edge on it as the
/// reflex machine's tick did: the memory reads `Withdraw`, its goal is a cell
/// of the hull's OWN entry zone (`entry_cell`: column 0 or 1), the motor
/// drives it there — the hull ends within `ARRIVE_CELLS` of the zone — and
/// it is still withdrawing at 30 s, having fired nothing: a dry hull never
/// rallies, its quiet clock never starts. What moved, and why: the fixture
/// split. On the graphless bluff window the whole fixture cannot be ported —
/// a mind's `Withdraw` goal fails its search at every landing there,
/// `Fallback::Notice` drops it and `fallback_waypoint` answers `None`, so
/// the hull never leaves (6, 8), where the reflex's `Waypoint` fallback kept
/// the goal and the routeless steer walked it to (5, 8) — so the dry edge is
/// proved here on the graphed window, where the entry zone is reachable, and
/// the bluff's edge on the motor under a scripted goal. The 30 s window is
/// the reflex fixture's own; the mind's first online think lands at 1,732.
#[test]
fn t1_a_dry_hull_withdraws_to_its_entry_zone() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 30_000, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Withdraw),
        "a hull with nothing to fire withdraws"
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Withdraw, "{memory:?}");
    assert!(
        memory.goal.is_some_and(|(x, _)| x <= 1),
        "the goal is a cell of its own entry zone: {memory:?}"
    );
    assert!(!memory.rallied, "a dry hull never rallies: {memory:?}");
    let cell = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose));
    assert!(
        cell.is_some_and(|(x, _)| x <= 1 + tacsim::arena::motor::ARRIVE_CELLS),
        "it drove home: {cell:?}"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0, "nothing to fire");
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — Withdraw is not terminal
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — the cornered withdrawal, on the unit mind.** The flicker:
/// the hostile crosses in and out of the band the withdrawal will stand for,
/// once a THINK for ten seconds. A mind that answered it with a **state**
/// change would throw its route away and buy a path query a second; this one
/// stands where it is (the stand's `Goal(Hold)` and its dwell of
/// `STAND_TICKS × decide_ms` = 1,500 ms, which outlasts a far think,
/// so the far think stops too and the motor is never handed a cell to search
/// for), shoots when the line is there, and never leaves `Withdraw`. Every
/// claim is the original's: `Withdraw` after every pulse of the flicker, at
/// most three searches, more `fired` lines during the withdrawal than before
/// it, and 277 per mille read by the mind itself
/// (`stock_mind::health_permille` over the last kept think's view).
///
/// What moved, and why. The flicker toggles once per think (1,000 ms) and not
/// once per pulse: the sensor pass refreshes a plot every `pass_ms` (1,000 ms)
/// and a think reads the latest pass, so a toggle every 500 ms would have
/// shown every pass the same cell. **Three fixture constants are re-derived
/// against the view** (the `range_round` divergence: a contact's range is
/// published rounded to `CONTACT_RANGE_STEP_M` (50 m) and floored at one
/// step, held within five eighths of a step of the figure last published): a
/// third of a fourteen-cell sight is 4,666 cm, under the 5,000 cm a view can
/// say at all, so no en-route stand could ever open — the sight is
/// **sixteen cells** (stand band 5,333 cm: a near cell reads 50 m and is a
/// stand); the far cell must read 100 m, more than 8,125 cm from where the
/// machine stands (the hysteresis off a held 50), so the machine opens at
/// **(6, 8)** (it patrols west, away from the hostile, and stands wherever it
/// classified it, well short of column 7) and the far cell is **(15, 8)**;
/// the near cell is (7, 8). The seated tank presents every band
/// ([`presenting`]): a mind's armed slot fires at odds and a default profile
/// presents none, and the shots during the withdrawal are the slot's (the
/// lock was taken in `Engage` before the wound, so no think of the stand
/// plans a `Fire` of its own).
#[test]
fn t1_a_cornered_withdrawal_stands_and_fights_without_leaving_the_state() {
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                16,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(7, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 5_000, &mut bytes);
    // Hurt it under the threshold, so the next think is a `Withdraw`.
    hurt_but_fighting(&mut arena, RED);
    run_settled(&mut arena, 8_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Withdraw);
    let last = stock.kept().last().map(|kept| kept.2.clone());
    assert_eq!(
        last.as_ref()
            .map(tacsim::arena::stock_mind::health_permille),
        Some(277),
        "over `stand_min_permille`, so the stand is open to it"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");

    let mut wall_ms = arena.now_ms;
    let mut queries = 0_u32;
    let mut near = true;
    for pulse in 0..20 {
        if pulse % 2 == 0 {
            if let Some(state) = arena.unit_mut(BLUE) {
                state.pose = centre(if near { 7 } else { 15 }, 8);
            }
            near = !near;
        }
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        queries = queries.saturating_add(out.motor.path_queries);
        assert_eq!(
            machine_state_of(&arena, RED),
            Some(MachineState::Withdraw),
            "the flicker moved the machine out of `Withdraw` at {} ms",
            arena.now_ms
        );
        wall_ms += 500;
    }
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Withdraw);
    assert!(
        queries <= 3,
        "a state flip a second would have bought a search a second: {queries}"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(
        count(&log, "fired") > before,
        "cornered, it shot back — during the withdrawal and not before it"
    );
}

/// **17b cut M4.4 — the rally, on the unit mind.** A hurt mind breaks off,
/// drives home, and `rally_ms` of quiet later turns round once: `Withdraw`
/// and not yet `rallied` at 12,000; `Patrol` and `rallied` at 60,000 with the
/// hull still under `withdraw_permille` (read as the mind reads it,
/// `stock_mind::health_permille` over the last kept think's view); and still
/// `Patrol` at 120,000, because the health edge gates on `!rallied` and is
/// spent. What moved: the state is the doctrine state and `rallied` is the
/// stock memory's key; the horizons are the original's (the rally think is a
/// think period plus a substep late at most, inside every window here).
#[test]
fn t1_a_withdrawn_tank_rallies_once_and_then_fights_to_the_end() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    hurt(&mut arena, RED);
    run_settled(&mut arena, 12_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert!(
        !stock_memory_of(&arena, RED).rallied,
        "it has not come back yet"
    );

    // Home, and quiet. The doctrine's `rally_ms` later it turns round.
    run_settled(&mut arena, 60_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Patrol, "{memory:?}");
    assert!(memory.rallied, "{memory:?}");
    let last = stock.kept().last().map(|kept| kept.2.clone());
    assert!(
        last.as_ref()
            .map(tacsim::arena::stock_mind::health_permille)
            .is_some_and(|health| health < ReflexConfig::default().withdraw_permille),
        "it is still hurt, which is what makes the next assertion mean something"
    );

    // And it does not withdraw again: the health edge is spent.
    run_settled(&mut arena, 120_000, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Patrol),
        "a rallied tank fights to the end"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Patrol);
}

/// **17b cut M4.4 — a dry mind withdraws and never rallies.** With nothing to
/// fire, `is_dry` takes the `Withdraw` edge at the first online think, and a
/// dry withdrawal holds `quiet_since_ms` at `None` every think, so the rally
/// clock never starts: `Withdraw` at 5,000 and still `Withdraw` and not
/// `rallied` at 90,000 — four times the rally lull, at full health and with
/// nothing hostile to see. What moved: the state is the doctrine state and
/// `rallied` the stock memory's key; nothing else.
#[test]
fn t1_a_tank_with_nothing_to_fire_withdraws_and_never_rallies() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Mind,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 5_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    // Four times the rally lull, at full health and with nothing hostile to see:
    // every other reason to stay out is gone, and it stays out.
    run_settled(&mut arena, 90_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Withdraw, "{memory:?}");
    assert!(!memory.rallied, "{memory:?}");
}

/// A hurt mind one cell from a classified hostile, with `rounds` in the
/// magazine: how far west it got over twenty seconds of `Withdraw`, and how many
/// rounds it fired.
///
/// **17b cut M4.4**: the machine is a mind under `step_settled`, its state the
/// doctrine state, its health the figure its own next think read
/// (`stock_mind::health_permille` over the kept view). **Two fixture
/// constants re-derived against the view** (the `range_round` divergence: a
/// contact's range reaches a think rounded to 50 m and floored at one step):
/// the sight is sixteen cells, not fourteen, because a third of 14,000 cm is
/// 4,666 and under the 5,000 cm a view can say, so no en-route stand could
/// open for the armed control — at sixteen the band is 5,333 cm and the
/// hostile one cell off reads 50 m; and the hostile presents every band
/// ([`presenting`]), because the stand's shots are the armed slot's (the lock
/// was taken in `Engage` before the wound) and a slot fires at odds.
fn withdrawing_beside_a_hostile(rounds: u32) -> (i64, usize) {
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(9, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                16,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(10, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), rounds);
    }
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 4_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    // **16b cut E.1**: the milder recipe, because this fixture's whole subject is
    // what an ARMED withdrawal does when a hostile is on top of it — and at 166
    // per mille the answer would be "keeps driving", for the health and not for
    // the magazine, which is the control's own claim stolen from it.
    hurt_but_fighting(&mut arena, RED);
    let thought = stock.kept().len();
    let from = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 20_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        stock
            .kept()
            .get(thought)
            .map(|kept| tacsim::arena::stock_mind::health_permille(&kept.2)),
        Some(277),
        "under `withdraw_permille` and over `stand_min_permille`: the first think after the wound"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Withdraw),
        "the fixture is about what a withdrawal does, so it has to be one"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Withdraw);
    let west = from - arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    (
        west,
        count(&events(&mut arena, &mut bytes), "fired") - before,
    )
}

/// **A mind with nothing to fire is never cornered** (16b cut Q.7; 17b cut
/// M4.4 on the unit mind).
///
/// The stand stops a withdrawing tank in the open and turns it onto the plot,
/// which is worth doing because it shoots back. A dry tank would stop, turn,
/// stage nothing and re-arm the dwell every think for as long as it held the
/// contact — a parked target in front of a gun, which is the debt this cut was
/// paid to remove and not a stand. The armed half is the control: same ground,
/// same hostile, same damage, and the only difference is the magazine
/// (`withdraw_stand` answers `None` for a dry hull). The figures are the
/// original's claims, re-measured on T1 and kept: the armed control stands
/// and shoots, the dry one makes more than 4,000 cm and 3,000 cm more than
/// the control.
#[test]
fn t1_a_dry_withdrawal_drives_on_where_an_armed_one_stands_and_fights() {
    // Twenty rounds, because a magazine that runs out mid-test would turn the
    // control into the case it is a control for.
    let (armed_west, armed_fired) = withdrawing_beside_a_hostile(20);
    assert!(
        armed_fired > 0,
        "the control is a tank that stands and shoots back"
    );
    assert!(
        armed_west < 2_000,
        "and one that is standing: it made {armed_west} cm of ground"
    );

    let (dry_west, dry_fired) = withdrawing_beside_a_hostile(0);
    assert_eq!(dry_fired, 0, "it had nothing to fire");
    // Cell 9 to the red entry zone at columns 0 and 1 is seven cells of ground,
    // and twenty seconds at ten metres a second is more than enough of it.
    //
    // **16b cut E.3 lowered the floor from 5,000 cm** (the run made 4,990). The
    // machine now brakes for its entry cell instead of driving flat out at it
    // (16c `B-42`), so a fixed twenty seconds buys it a little less ground —
    // deliberately, because the old behaviour was a tank that shot past the goal
    // and restarted its rally clock. What this test is about is the *difference*
    // between the two magazines, and that is untouched: 4,990 cm against nothing.
    //
    // **16b cut M.2 lowered it again, to 4,000** (16c `T-02`; the run makes
    // 4,490). The machine's own `STARTUP` resolves at its tick now, so it came
    // online a decide tick sooner and had patrolled five metres further west by
    // the four seconds the contact is planted at; it withdraws to the same
    // cell as before (measured: 2,644 cm either way) from a start five metres
    // nearer it. The difference between the magazines is still the claim.
    assert!(
        dry_west > 4_000 && dry_west > armed_west + 3_000,
        "a dry tank drives home instead of stopping in front of the gun: \
         {dry_west} cm against the armed tank's {armed_west} cm"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — heat
// ---------------------------------------------------------------------------

/// **The heat fixture's hull, with its Class I core kept** (17b cut M4.4; the
/// reflex fixture `heated` it replaces is retired with its tests): the same
/// heat cell on the same eighteen-point hull, assembled as [`tactical_unit`]
/// assembles it — with `tacsim::definitions::core::core_component` in the
/// hull — because the mind's seal records that core and a sealed core that is
/// gone drops the hull to the ROM (`Arena::core_lost`, 17b cut M3.4), which
/// is not what a heat fixture is about.
fn t1_heated(arena: &mut Arena, unit: ObjId, max: u32, now: u32, per_shot: u32) {
    let (core, capability) = tacsim::definitions::core::core_component(
        "hull",
        tacsim::arena::core::CoreClass::I,
        dref("component", "tacsim.unit.line_tank.core"),
        dref("schema", tacsim::definitions::unit::CORE_RULES_SCHEMA),
    )
    .expect("a core");
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: vec![FacetSpec {
                id: FacetId::new("front").expect("a canonical facet"),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("armor").expect("a cell"),
                    min: 0,
                    max: 10,
                    init: 10,
                }],
            }],
            slots: Vec::new(),
            cells: vec![
                CellSpec {
                    id: mush_tactics::CellId::new("internals").expect("a cell"),
                    min: 0,
                    max: 8,
                    init: 8,
                },
                CellSpec {
                    id: mush_tactics::CellId::new(tacsim::definitions::unit::HEAT_CELL)
                        .expect("a cell"),
                    min: 0,
                    max: i64::from(max),
                    init: i64::from(now),
                },
            ],
        }],
        unit_cells: Vec::new(),
        components: vec![core],
        capabilities: vec![capability],
        mobility: None,
        rules: rules(),
    };
    let mut per_shot_by_weapon = std::collections::BTreeMap::new();
    per_shot_by_weapon.insert(SmolStr::new("cannon"), per_shot);
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = TacticalUnit::from_spec(&spec, unit).expect("a unit that carries heat");
        state.heat = Some(tacsim::arena::state::HeatState {
            section: SmolStr::new("hull"),
            max,
            per_s: 0,
            per_shot: per_shot_by_weapon,
            carry_milli: 0,
        });
    }
}

/// The two tanks the heat test shares: the mind in reach of a parked target.
///
/// **17b cut M4.4**: the machine is a mind seeing ten cells (as 17b cut
/// M4.3's `j_field` fields its minds, not the rig's five: a contact's range
/// reaches a think rounded to 50 m and floored at one step — the
/// `range_round` divergence — so a five-cell sight's 3,500 cm engage band
/// lies under anything the view can say and the mind would never engage; at
/// ten the band is 7,000 cm and the target three cells off reads 50 m), and
/// the target presents every band ([`presenting`]), so the armed slot's
/// later shots are decided at odds and the resolver's heat refusal is the
/// only thing holding them.
fn heat_field() -> Arena {
    arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                10,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    )
}

/// **17b cut M4.4 — the overheated mind holds its fire, on the unit mind.**
/// The three cases are the original's, each read through the `fired` and
/// `locked` lines, the doctrine state and `heat_now`: (a) at the ceiling the
/// mind locks and never fires — `would_overheat` holds the think's own
/// `Fire` back, and the armed slot's shots are refused `Overheated` by the
/// resolver, silently; (b) one shot short of the ceiling the one legal shot
/// is taken and lands the hull on it; (c) a shot the engine clamps is taken
/// and the next is refused. What moved: `counters.shots` (the reflex row's
/// accepted `FIRE`s) is dropped — the `fired` line is the count that
/// survives the controller — and the field is [`heat_field`]'s mind on
/// [`t1_heated`], the heat cell on a hull that keeps its core.
#[test]
fn t1_an_overheated_machine_holds_its_fire_rather_than_feeding_a_refusal() {
    // (a) **at the ceiling**: the platform is carrying its whole ten, which is
    // the one reading `apply_intent`'s `Overheated` refusal takes. The mind
    // reads it too and plans no shot at all.
    let mut arena = heat_field();
    t1_heated(&mut arena, RED, 10, 10, 1);
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 8_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0, "the hot gun stayed quiet");
    assert!(count(&log, "locked") > 0, "it still closed and locked");

    // (b) **one shot short of it**: a cold platform whose shot costs its whole
    // ceiling. The engine's rule is that a shot which would carry a unit past
    // the maximum leaves it exactly at it and the *next* one is refused
    // (`UnitState::overheated`), so this is a gun that works once — and the
    // machine must take that shot, or a pilot and a machine in the same tank
    // would disagree about whether the gun works at all.
    let mut arena = heat_field();
    t1_heated(&mut arena, RED, 10, 0, 10);
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 8_000, &mut bytes);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 1, "the engine's last legal shot");
    assert_eq!(
        arena.unit(RED).and_then(|unit| unit.heat_now()),
        Some(10),
        "and it is at the ceiling now, which is what stopped the second"
    );

    // (c) **a shot that would go past it** — **16b cut E.2 (16c `T-55`)
    // reversed this case.** Twenty on a platform carrying fifty of sixty is a
    // shot the engine accepts and clamps (`Arena::raise_heat` writes
    // `min(now + per_shot, max)`), so the machine takes it too: the two halves
    // of one platform may not disagree about whether the gun works, which is
    // the sentence `would_overheat`'s doc used to make the other argument with.
    // It lands ON the ceiling, and the next think is (a).
    let mut arena = heat_field();
    t1_heated(&mut arena, RED, 60, 50, 20);
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 8_000, &mut bytes);
    assert_eq!(
        arena.unit(RED).and_then(|unit| unit.heat_now()),
        Some(60),
        "it landed on the ceiling rather than past it"
    );
    // And it held its fire afterwards, which is the other half of the same
    // line: eight seconds is two cannon cycles.
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "fired"),
        1,
        "the engine would have taken this shot and clamped, and the second was refused"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — `EndReason::Spent`
// ---------------------------------------------------------------------------

/// **Two armed tanks facing each other, one the mind's and one a pilot's, for
/// the two `Spent` fixtures below** (16b cut Q.7's `dry_pair`, on the unit
/// mind by 17b cut M4.4): the machine's tank a mind seeing ten cells (as 17b cut M4.3's
/// `j_field` fields its minds — a contact's range reaches a think rounded to
/// 50 m and floored at one step, the `range_round` divergence, so at the
/// rig's five cells the 3,500 cm engage band is under anything the view can
/// say and the mind would never lock or fire; at ten the band is 7,000 cm and
/// the pilot's tank three cells off reads 50 m), the pilot's tank presenting
/// every band ([`presenting`]), so a round the mind decides on is a round the
/// arena fires — and the mind's cannon doing no damage, because a presented
/// eighteen-point hull dies to one 120-damage hit and the match would end by
/// elimination before the draw: what these fixtures pin is the draw over the
/// magazines, never what a round does to armour.
fn t1_q7_spent_pair(red_rounds: u32, blue_rounds: u32) -> Arena {
    let harmless = WeaponSummary {
        damage: 0,
        ..cannon()
    };
    let mut arena = arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![harmless],
                    Controller::Mind,
                ),
                10,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                vec![cannon()],
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    for (unit, rounds) in [(RED, red_rounds), (BLUE, blue_rounds)] {
        if let Some(state) = arena.unit_mut(unit) {
            state.ammo.insert(SmolStr::new("cannon"), rounds);
        }
    }
    arena
}

/// **17b cut M4.4 — the `Spent` draw, with the machine a mind.** The edge is
/// the arena's (`Arena::spent`): both magazines empty and both tanks whole
/// ends the match `Spent`, a draw the black box names; one dry side plays on.
/// What moved: the machine's tank is [`t1_q7_spent_pair`]'s mind, stepped with
/// `step_settled`; every claim is the original's.
#[test]
fn t1_two_dry_sides_end_in_a_draw_and_one_dry_side_plays_on() {
    // Both magazines empty and both tanks whole: doc 16 §3.8's elimination is
    // untouched — neither is a wreck and neither is disabled — so what ends it is
    // that nobody can shoot.
    let mut arena = t1_q7_spent_pair(0, 0);
    let mut bytes: Vec<u8> = Vec::new();
    let mut ended = None;
    let mut wall_ms = 0_i64;
    while arena.now_ms < 20_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        if let Some(end) = out.ended {
            ended = Some(end);
            break;
        }
        wall_ms += 500;
    }
    let end = ended.unwrap_or_else(|| panic!("nobody could shoot, so it should have ended"));
    assert_eq!(end.reason, tacsim::arena::state::EndReason::Spent);
    assert_eq!(
        tacsim::match_::MatchOutcome::from(end.reason),
        tacsim::match_::MatchOutcome::Draw,
        "it names no winner"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(
        log.iter().any(|event| matches!(
            event.kind,
            tacsim::arena::ledger::EventKind::MatchEnded {
                reason: tacsim::arena::state::EndReason::Spent
            }
        )),
        "the black box says why"
    );

    // One side dry is not a result. A solo player at full armour does not lose
    // the instant their last round leaves the breech.
    let mut arena = t1_q7_spent_pair(0, 40);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    while arena.now_ms < 15_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        assert!(out.ended.is_none(), "a dry side plays on: {:?}", out.ended);
        wall_ms += 500;
    }
}

/// **17b cut M4.4 — a round in flight holds the draw off, the round a mind's.**
/// The mind's one round is the `Fire` its think plans beside the fresh lock
/// (resolved at the landing, a substep on), and at 10 m/s it is three seconds
/// in the air across pulse boundaries: `Arena::spent` is false while
/// `arena.shots` holds it, and the match ends once it lands. What moved: the
/// machine's tank is [`t1_q7_spent_pair`]'s mind under `step_settled`; the
/// claims are the original's, and the end is pinned `Spent` (the harmless
/// gun keeps elimination out of it).
#[test]
fn t1_a_round_still_in_flight_holds_the_draw_off() {
    // The last round has left the breech and has not landed: it can still decide
    // the match, so calling a draw over it would be calling a draw over a kill.
    let mut arena = t1_q7_spent_pair(1, 0);
    // A slow round, so "in flight" outlives the pulse that fired it: the stock
    // cannon's 1,450 m/s crosses three cells in twenty milliseconds and the
    // impact is resolved inside the same substep.
    if let Some(weapon) = arena
        .unit_mut(RED)
        .and_then(|state| state.weapons.first_mut())
    {
        weapon.muzzle_speed_cms = 1_000;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut in_flight = false;
    while arena.now_ms < 20_000 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        if !arena.shots.is_empty() {
            in_flight = true;
            assert!(
                !arena.spent(),
                "a round is still in the air at {} ms",
                arena.now_ms
            );
        }
        if out.ended.is_some() {
            break;
        }
        wall_ms += 500;
    }
    assert!(in_flight, "the machine never fired its one round");
    assert!(
        arena.ended.is_some(),
        "and once it landed, nobody could shoot"
    );
    assert_eq!(
        arena.ended.map(|end| end.reason),
        Some(tacsim::arena::state::EndReason::Spent),
        "the draw, and not an elimination"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — the cross-tempo property (16b §11: mandatory)
// ---------------------------------------------------------------------------

/// **The one arena millisecond a scripted replay does something to the world.**
///
/// A multiple of all three cadences, so the pulse that reaches it ends at
/// exactly this arena millisecond at every one of them and the script is the
/// same fact of arena time in all three logs.
const SCRIPT_AT_MS: i64 = 20_000;

/// One scripted replay of `build`'s arena, at one cadence. `script` runs at
/// [`SCRIPT_AT_MS`] and nowhere else.
fn replay_of(
    cadence_ms: i64,
    until_ms: i64,
    build: fn() -> Arena,
    script: fn(&mut Arena),
) -> Replay {
    replay_with(
        cadence_ms,
        until_ms,
        build(),
        &|_, _| Vec::new(),
        &mut |arena: &mut Arena| {
            if arena.now_ms == SCRIPT_AT_MS {
                script(arena);
            }
        },
    )
}

/// **[`replay_of`], with the world's hand on the arena at more than one beat**
/// (16b cut Z.2).
///
/// `beat` is asked before every pulse, with the pulse's index, and what it
/// answers is that pulse's intent slice — applied in phase 2 at the pulse's
/// **opening** arena millisecond, which is how the world delivers an `ORDER` or
/// a `RELEASE`. A beat is cadence-safe when it keys on an `arena.now_ms` that is
/// a multiple of all three cadences, or on pulse `0` for arena millisecond `0`
/// (the first two pulses both open at `0`: the first integrates nothing).
/// `after` runs after every pulse and keys on the pulse's **closing** arena
/// millisecond the same way; it may also watch the arena, which is how a test
/// samples a unit's cell pulse by pulse.
fn replay_with(
    cadence_ms: i64,
    until_ms: i64,
    arena: Arena,
    beat: &dyn Fn(&Arena, u32) -> Vec<tacsim::arena::intents::Intent>,
    after: &mut dyn FnMut(&mut Arena),
) -> Replay {
    replay_counted(cadence_ms, until_ms, arena, beat, after).0
}

/// [`replay_with`], with every pulse's reflex counters summed (17b cut M4.4):
/// the one loop both read — the moat port reads the motor's one failed search
/// off `MotorCounters::path_failures` (M4.5's `MotorCounters`).
fn replay_counted(
    cadence_ms: i64,
    until_ms: i64,
    mut arena: Arena,
    beat: &dyn Fn(&Arena, u32) -> Vec<tacsim::arena::intents::Intent>,
    after: &mut dyn FnMut(&mut Arena),
) -> (Replay, tacsim::arena::motor::MotorCounters) {
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut total = tacsim::arena::motor::MotorCounters::default();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let intents = beat(&arena, pulses);
        // 17b cut M4.4: settled between pulses, as the task's pool settles
        // (M4.1's gate for a field with minds; the same bytes on one without).
        let out = arena.step_settled(wall, &[], &intents);
        pulses = pulses.saturating_add(1);
        absorb(&out, &mut bytes);
        total.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        after(&mut arena);
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                machine_state_of(&arena, unit.id).unwrap_or(MachineState::Startup),
                motor(&arena, unit.id).goal,
            )
        })
        .collect();
    (
        Replay {
            events,
            hashes,
            finals,
            pulses,
        },
        total,
    )
}

/// Nothing at all — the wreck detour writes its own story.
fn no_script(_arena: &mut Arena) {}

/// **`withdraw_and_rally` on the unit mind** (17b cut M4.4): the same hurt
/// machine beside the same classified hostile, the machine a mind. **Two
/// fixture constants re-derived against the view** (the `range_round`
/// divergence: a contact's range reaches a think rounded to 50 m and floored
/// at one step, held within five eighths of a step of the figure last
/// published): the sight is **fifteen cells**, not five — a third of a
/// five-cell reach is 1,666 cm, under the 5,000 cm a view can say, so the
/// stand this script exists to replay could never open; at fifteen the band
/// is exactly 5,000 cm, the hostile one cell off reads 50 m and the stand
/// opens at the first online think; the far corner **(15, 15) is grass**
/// (the rig's sight shows a stopped tank on grass at eight tenths, 12,000
/// cm): at fifteen cells the home stand's ceiling (`release_cm`) is the whole
/// reach and the whole window is inside it, so a machine home on column 2
/// (where `ARRIVE_CELLS` stops it) would stand and shoot at a corner 14,400
/// cm off that it never loses and never rally; on grass the corner is off
/// the view from home and still on it from the stand (9,900 cm), so the
/// contact is `lost` and the quiet clock can start — and the hostile presents
/// every band ([`presenting`]), so the stand's shots are fired at odds
/// (`fired` is one of the kinds the log must carry).
fn t1_q7_withdraw_and_rally() -> Arena {
    let grass = TerrainCell {
        class: mush_atlas::TerrainClass::Grass,
        elev_cm: 0,
        move_cost: 100,
    };
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                15,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[(15, 15, grass)],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    // **16b cut E.1**: the milder recipe, so the stand this script exists to
    // replay is one the machine's health still allows.
    hurt_but_fighting(&mut arena, RED);
    // The contact is planted classified at nought: a script whose subject is
    // the stand starts with the classification rather than waits for it.
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    arena
}

/// The blue tank pulls out to the far corner, out of the red one's sight.
fn blue_pulls_out(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
}

/// **17b cut M4.4 — the whole of `Withdraw`'s life on the unit mind, one
/// event list and one hash sequence at three cadences** (16b cut Q.7; 16b
/// §11). The arc is T1's own: the health edge at the first online think, the
/// stand inside `stand_cm` with its dwell re-armed every think the hostile is
/// close, the hostile gone at [`SCRIPT_AT_MS`], the drive home, the contact
/// `lost` off the sight, the quiet clock and the rally `rally_ms` later — all
/// of it in `startup`, `detected`, `classified`, `fired` and `lost` lines,
/// and in a final state that is not `Withdraw`, which only the rally can
/// give. What moved: the field is [`t1_q7_withdraw_and_rally`]'s mind, replayed
/// through [`replay_of`] under `step_settled`, the finals' state the doctrine
/// state; the 70,000 horizon is the original's and holds the rally with room
/// (measured: the rally lands inside it at every cadence).
#[test]
fn t1_withdraw_stand_and_rally_are_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_of(cadence, 70_000, t1_q7_withdraw_and_rally, blue_pulls_out))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "detected", "classified", "fired", "lost"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}: {kinds:?}"
        );
    }
    // The arc really happened: the hurt tank is out of `Withdraw` again, which
    // only the rally can do.
    assert!(
        first
            .finals
            .iter()
            .any(|(unit, _, _, _, _, state, _)| *unit == RED
                && !matches!(state, MachineState::Withdraw)),
        "the machine never rallied, so the log proves only half of it: {:?}",
        first.finals
    );
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (l) 16b cut D.3 (`B-34`) — a released COLD tank starts itself up
// ---------------------------------------------------------------------------

/// A pilot's tank that is **cold** — it was never started up — out in the middle
/// of the window, with nobody else in the match.
fn cold_pilot_tank() -> Arena {
    arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Pilot,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[RED],
    )
}

/// The release, delivered in the pulse that begins at [`SCRIPT_AT_MS`].
///
/// [`replay_of`]'s own `script` hook takes `&mut Arena` and cannot step one, and
/// a `Control` is a message the world sends *into* `step` — so this is
/// `replay_of` with the control put on the wire instead. [`SCRIPT_AT_MS`] is a
/// multiple of all three cadences, so the release is the same fact of arena time
/// in every log.
fn replay_released_cold(cadence_ms: i64, until_ms: i64) -> Replay {
    let mut arena = cold_pilot_tank();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut sent = false;
    let mut pulses = 0_u32;
    loop {
        let release = if !sent && arena.now_ms >= SCRIPT_AT_MS {
            sent = true;
            vec![Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
                pilot: None,
            }]
        } else {
            Vec::new()
        };
        let out = arena.step(wall, &release, &[]);
        pulses = pulses.saturating_add(1);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let finals = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                // 17b cut M4.3: either machine's state (the mind's cached).
                machine_state_of(&arena, unit.id).unwrap_or(MachineState::Startup),
                motor(&arena, unit.id).goal,
            )
        })
        .collect();
    Replay {
        events,
        hashes,
        finals,
        pulses,
    }
}

/// **A released cold tank brings its own power up and drives home** (16b cut
/// D.3, `B-34`).
///
/// It replaces the claim `arena.rs`'s
/// `startup_then_released_in_one_burst_never_starts` used to make. A pilot who
/// `EXIT`s before `STARTUP` used to leave a dark tank standing on its entry cell
/// for the rest of the match — and because doc 16 §3.8's elimination is *a wreck,
/// or disabled with no weapons*, that tank kept its side standing until somebody
/// found it and killed it, which is a match nobody could end.
///
/// Tick by tick: the release puts the machine in `Withdraw` with the power still
/// `Off`; its **first** tick sets the plan's `startup`, so the arena accepts a
/// `Startup` and the power goes `Starting`; `match.startup_ms` later the
/// scheduled completion carries it to `Online`; and it stays in `Withdraw` all
/// the way through — the `Online` arm re-enters `Patrol` from `Startup` and
/// `Disabled` alone — so it drives to its own entry zone and does not go looking
/// for a fight it has already left.
///
/// **17b cut M4.3 — ported onto the mind** (the plan's second released-tank
/// fixture): the release hands the cold tank to its mind seeded `Withdraw`;
/// the mind's FIRST think (on the stagger grid, landing a substep on) orders
/// the startup, and the doctrine's `Online` arm re-enters `Patrol` from
/// `Startup` and `Disabled` alone, so it stays in `Withdraw` all the way.
#[test]
fn t1_a_released_cold_tank_starts_itself_up_and_stays_in_withdraw() {
    let mut arena = cold_pilot_tank();
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 1_000, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.power),
        Some(Power::Off),
        "the pilot never started it"
    );

    // The seat goes, cold.
    let now = arena.now_ms;
    let out = arena.step(
        now,
        &[Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: None,
        }],
        &[],
    );
    absorb(&out, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind)
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert_eq!(
        arena.unit(RED).map(|unit| unit.power),
        Some(Power::Off),
        "the release itself starts nothing"
    );

    // Its first think orders the startup, `match.startup_ms` carries it to
    // `Online`, and it is in `Withdraw` the whole way — a machine that had gone
    // through `Startup` would be in `Patrol` by the end of this block, because
    // that is the only state the `Online` arm re-enters `Patrol` from. The
    // first think is at most a think period away on the grid and lands a
    // substep on (17b cut M4.3).
    let released_at = arena.now_ms;
    let start = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    let until = released_at
        + tacsim::arena::mind::STOCK_THINK_MS
        + arena.substep_len_ms()
        + i64::from(match_cfg().startup_ms)
        + 1_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.power),
        Some(Power::Online),
        "the mind never ordered a startup"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));

    // And it drives home rather than standing where its pilot left it. The
    // horizon stops short of `arena.reflex.rally_ms`, which is a later edge of
    // its own (16b cut Q.7) and not this row's.
    let until = arena.now_ms + 8_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < start),
        "it came up and stood still"
    );
    assert!(arena.minds.contains_key(&RED), "a mind");
    let goal = stock_memory_of(&arena, RED).goal;
    assert!(
        goal.is_some_and(|(x, _)| x <= 1),
        "the goal is the red entry zone: {goal:?}"
    );
    // The black box, tick by tick: the startup is the machine's own — after the
    // release, not the pilot's — and the power comes online exactly
    // `match.startup_ms` after it.
    let log = events(&mut arena, &mut bytes);
    let at = |word: &str| -> Option<i64> {
        log.iter()
            .find(|event| event.kind.as_str() == word)
            .map(|event| event.t_ms)
    };
    let startup = at("startup").unwrap_or_else(|| panic!("no startup was journalled"));
    let online = at("power_online").unwrap_or_else(|| panic!("the power never came online"));
    assert!(
        startup >= released_at,
        "the startup predates the release: {startup} against {released_at}"
    );
    assert_eq!(
        online - startup,
        i64::from(match_cfg().startup_ms),
        "the completion did not land at `match.startup_ms`"
    );
}

/// **Cross-tempo (16b §11, mandatory): the released cold tank's whole arc is one
/// event list and one hash sequence at 5,000 / 1,000 / 500 ms** (16b cut D.3,
/// `B-34`).
///
/// **17b cut M4.3 — ported onto the mind** (the plan's third released-tank
/// fixture): the same arc with the released tank a mind's, its state read
/// through `Arena::machine_state`.
#[test]
fn t1_a_released_cold_tank_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_released_cold(cadence, 35_000))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "power_online"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    assert!(
        first
            .finals
            .iter()
            .any(|(unit, _, _, _, _, state, _)| *unit == RED && *state == MachineState::Withdraw),
        "the machine left `Withdraw` inside the horizon (the rally is a later edge          of its own): {:?}",
        first.finals
    );
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut E.1 — the machine fights at the range it can actually see
// ---------------------------------------------------------------------------

/// **The fixture's own numbers, at a ten-cell sight.**
///
/// The fixture cannon is one band at 2,500 m and the window is sixteen cells of
/// 1,000 cm, so the sight is what bounds everything: with ten cells the
/// platform's fighting reach is 10,000 cm, doctrine's `engage_band_permille` of
/// 700 puts `Approach → Engage` at 7,000 cm, `Engage → Approach` at 10,000 cm,
/// and an en-route `Withdraw` stands inside the `close` band of 3,333 cm — a
/// band no figure the view publishes is ever inside, on the unit mind (the
/// `range_round` divergence: 50 m is the view's floor). Every test below says
/// which of those four it is about.
const SIGHT_CELLS: u16 = 10;
/// `UnitState::fight_inner_cm` and `fight_limit_cm` at [`SIGHT_CELLS`].
const REACH_CM: i64 = 10_000;
/// `Ranges::engage_cm` at [`SIGHT_CELLS`]: seven tenths of [`REACH_CM`].
const ENGAGE_CM: i64 = 7_000;
/// `Ranges::release_cm` at [`SIGHT_CELLS`], which is [`REACH_CM`] itself.
const RELEASE_CM: i64 = REACH_CM;

/// **A five-cell gun**: shorter than [`SIGHT_CELLS`], so a platform that falls
/// back to it has to close another five cells, and its firing ceiling is inside
/// what it can see. Everything else is the cannon's shape.
fn coaxial() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("coaxial"),
        max_range_cm: 5_000,
        bands: vec![WeaponBand {
            max_range_cm: 5_000,
            dispersion_mrad: 3,
        }],
        ..cannon()
    }
}

/// A machine with the cannon and a ten-cell sight, and a target tank with
/// nothing, on `paint`ed ground.
fn duel(red: (u16, u16), blue: (u16, u16), paint: &[(u16, u16, TerrainCell)]) -> Arena {
    duel_with(vec![cannon()], red, blue, paint)
}

/// The same with a named armament.
fn duel_with(
    weapons: Vec<WeaponSummary>,
    red: (u16, u16),
    blue: (u16, u16),
    paint: &[(u16, u16, TerrainCell)],
) -> Arena {
    arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(red.0, red.1),
                    weapons,
                    Controller::Reflex,
                ),
                SIGHT_CELLS,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(blue.0, blue.1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        paint,
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    )
}

/// The range from `unit`'s pose to `target`'s plot, centimetres.
fn plot_range(arena: &Arena, unit: ObjId, target: ObjId) -> i64 {
    let Some(state) = arena.unit(unit) else {
        return i64::MAX;
    };
    i64::try_from(plot_of(arena, target).range_from(&state.pose)).unwrap_or(i64::MAX)
}

/// **[`duel_with`] on the unit mind** (17b cut M4.4, for cut E.1's ported
/// fixtures): the machine's tank a mind seeing `sight` cells, the target tank
/// presenting every band ([`presenting`]) — a mind's armed slot fires at
/// odds, and a default profile presents none — on `paint`ed ground.
fn t1_e1_duel_with(
    weapons: Vec<WeaponSummary>,
    red: (u16, u16),
    blue: (u16, u16),
    paint: &[(u16, u16, TerrainCell)],
    sight: u16,
) -> Arena {
    arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(red.0, red.1),
                    weapons,
                    Controller::Mind,
                ),
                sight,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(blue.0, blue.1),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        paint,
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    )
}

/// [`duel`] on the unit mind: the cannon at [`SIGHT_CELLS`].
fn t1_e1_duel(red: (u16, u16), blue: (u16, u16), paint: &[(u16, u16, TerrainCell)]) -> Arena {
    t1_e1_duel_with(vec![cannon()], red, blue, paint, SIGHT_CELLS)
}

/// **An eight-cell gun** (17b cut M4.4): [`coaxial`]'s role on the unit mind
/// — a gun shorter than [`SIGHT_CELLS`], whose firing ceiling is inside what
/// the platform can see, so a target past the ceiling is still a target the
/// pass refreshes. **Re-derived from five cells against the view** (the
/// `range_round` divergence: a contact's range reaches a think rounded to 50
/// m and floored at one step, so no band under 5,000 cm is ever read inside,
/// and a five-cell gun's 3,500 cm engage band can never be entered on T1):
/// eight cells put `engage_cm` at 5,600 (a target reading 50 m is inside it)
/// and `release_cm` at 8,000 (a target reading 100 m is past it), both
/// inside the ten-cell sight, with two cells of the sight left for a target
/// to be past the ceiling and still refreshed. Everything else is the
/// cannon's shape.
fn t1_e1_coaxial() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("coaxial"),
        max_range_cm: T1_COAXIAL_RELEASE_CM_U32,
        bands: vec![WeaponBand {
            max_range_cm: T1_COAXIAL_RELEASE_CM_U32,
            dispersion_mrad: 3,
        }],
        ..cannon()
    }
}

/// [`t1_e1_coaxial`]'s one band, centimetres.
const T1_COAXIAL_RELEASE_CM_U32: u32 = 8_000;
/// `Ranges::engage_cm` for a mind holding [`t1_e1_coaxial`]: seven tenths of
/// its 8,000 cm band, which is inside [`SIGHT_CELLS`].
const T1_COAXIAL_ENGAGE_CM: i64 = 5_600;
/// `Ranges::release_cm` for the same mind — the band itself, inside what its
/// scope can hold.
const T1_COAXIAL_RELEASE_CM: i64 = 8_000;

/// **`Approach` with range and no line closes on the plot's own cell** (16b cut
/// E.1; 16c's Q9), and takes the trace again on the next think — **17b cut
/// M4.4, on the unit mind.**
///
/// Before cut E.1 a machine inside its (3 km) reach with no line held where
/// it stood: the `Engage` arm needed a line, and the `else` arm projected a
/// stand-off cell up to 2.1 km behind it. A crew that has lost the line
/// drives at where it last saw the tank, and that is what the goal is: the
/// stock mind's `Approach` arm, inside `engage_cm` with no `line_between`,
/// writes the row's own cell as the goal. Every claim is the original's:
/// `Approach` with the wall between them, inside the band, the goal the
/// plot's own cell (read off the motor row, as before), driving at it; then,
/// with a line at the same sort of range, `Engage`. What moved: the field is
/// [`t1_e1_duel`]'s mind, the state the doctrine state, and the window after
/// the move is 2,500 ms for the think and its landing where the reflex's
/// 1,500 covered three ticks; the four cells read 50 m on the view, inside
/// the 7,000 cm band as 4,000 is.
#[test]
fn t1_with_range_and_no_line_it_closes_on_the_plot() {
    // One wall cell squarely between them, and four cells of separation — well
    // inside `ENGAGE_CM`, so the only thing stopping the engagement is the line.
    let mut arena = t1_e1_duel((6, 8), (10, 8), &[(8, 8, wall())]);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let start = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    run_settled(&mut arena, 4_000, &mut bytes);

    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Approach),
        "the wall is between them, so there is no engagement to be had"
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range <= ENGAGE_CM,
        "and it is inside the band, which is what makes this the no-line case: {range} cm"
    );
    assert_eq!(
        motor(&arena, RED).goal,
        Some((10, 8)),
        "the goal is the plot's own cell"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).goal,
        Some((10, 8)),
        "and the mind remembers handing it over"
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm > start),
        "and it is driving at it"
    );

    // Now give it a line at the same sort of range: the trace is taken again on
    // the next think and the mind engages.
    let here = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .expect("the machine is on the window");
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(here.0, here.1.saturating_sub(3));
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 2_500;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "with a line inside the band it locks and fires"
    );
}

/// **The hysteresis** (16b cut E.1; 16c `T-47`): `Approach → Engage` at
/// `engage_cm`, `Engage → Approach` past `release_cm`, and three tenths of the
/// reach between them where neither edge fires — **17b cut M4.4, on the unit
/// mind.**
///
/// At HEAD of cut E.1 both edges read one number — the platform's whole
/// weapon reach — so `Engage` could never be left and `engage_band_permille`
/// governed nothing. The stock mind's `Approach` arm enters `Engage` at
/// `view_ranges`' `engage_cm` and its `Engage` arm leaves past `release_cm`
/// (`stock_mind.rs`), the same two numbers off the loaded gun and the sight:
/// [`ENGAGE_CM`] 7,000 and [`RELEASE_CM`] 10,000 at [`SIGHT_CELLS`]. Every
/// claim is the original's: `Engage` inside the band; between the edges the
/// state held and shots fired; past `release_cm` the band given up,
/// `Approach`, and a goal nearer the plot than the machine (`B-05`).
///
/// What moved, and why (the `range_round` divergence): a contact's range
/// reaches a think rounded to 50 m and held within five eighths of a step of
/// the figure last published (`held_range_m`, 3,125 cm), so each cell is
/// placed for **the figure the mind reads**, and the test asserts that figure
/// too. (a) is **nine cells east of the cell the machine engaged from** (it
/// patrols west for the three seconds the classification takes, so a fixed
/// column would not be the place): 9,000 cm is more than 3,125 cm from the
/// 50 m it was holding, so it re-rounds to 100 m — past `engage_cm`, not past
/// `release_cm`, where a mind with no release edge of its own would drop back
/// to `Approach`. (b) is the window's far corner `(15, 0)`: more than
/// 13,125 cm from the machine, so the 100 m it was holding re-rounds to
/// 150 m, past `release_cm`. The window after (b)'s move is 2,000 ms for the
/// think and its landing.
#[test]
fn t1_it_engages_at_the_band_releases_past_the_reach_and_holds_between_them() {
    let mut arena = t1_e1_duel((6, 8), (9, 8), &[]);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 5_000, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "the opening range is inside the 7,000 cm band"
    );
    let here = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .expect("the machine is on the window");
    // The figure each think since `from_ms` read for the target, centimetres.
    let read_since = |from_ms: i64| -> Vec<i64> {
        stock
            .kept()
            .iter()
            .filter(|(unit, t_ms, _, _)| *unit == RED && *t_ms > from_ms)
            .filter_map(|(_, _, view, _)| {
                view.contacts
                    .iter()
                    .find(|row| !row.friendly)
                    .map(|row| i64::try_from(row.range_m).unwrap_or(i64::MAX) * 100)
            })
            .collect()
    };

    // (a) Between the two edges — nine cells, 9,000 cm true and 100 m on the
    //     view: above `ENGAGE_CM`, not above `RELEASE_CM`. An engaged machine
    //     stays engaged there, and fires there.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(here.0 + 9, 8);
    }
    let moved_at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, moved_at);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        (ENGAGE_CM..=RELEASE_CM).contains(&range),
        "the fixture put the target at {range} cm, which is not between the edges"
    );
    let read = read_since(moved_at);
    assert!(
        !read.is_empty() && read.iter().all(|cm| *cm > ENGAGE_CM && *cm <= RELEASE_CM),
        "every think after the move read the target between the edges, not {read:?}"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "between the edges the machine holds the state it had"
    );
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and it may fire across the whole of its `Engage` region"
    );

    // (b) Past `RELEASE_CM`: it gives the band up and closes again, and its goal
    //     is nearer the plot than it is.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 0);
    }
    let moved_at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, moved_at);
    let opened = plot_range(&arena, RED, BLUE);
    assert!(
        opened > RELEASE_CM + 3_125,
        "the premise: the corner is past the bottom of a held 100 m figure from where the \
         machine stands: {opened} cm"
    );
    let until = arena.now_ms + 2_000;
    run_settled(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(range > RELEASE_CM, "the target is at {range} cm");
    let read = read_since(moved_at);
    assert!(
        read.first().is_some_and(|cm| *cm > RELEASE_CM),
        "the first think after the move read the target past the release edge, not {read:?}"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Approach));
    let goal = motor(&arena, RED)
        .goal
        .expect("a closing machine has a goal");
    let plot = plot_of(&arena, BLUE);
    let cell = i64::from(CELL_CM);
    let to_plot = ((i64::from(goal.0) * cell + cell / 2 - plot.x_cm).pow(2)
        + (i64::from(goal.1) * cell + cell / 2 - plot.y_cm).pow(2))
    .isqrt();
    assert!(
        to_plot < range,
        "the stand-off goal is {to_plot} cm from the plot and the machine is {range} cm from \
         it: the goal is behind the machine (`B-05`)"
    );
}

/// **The margin under `STAND_OFF_MARGIN_CELLS`** (16b cut E.1): a machine that
/// arrives at its stand-off cell is *inside* `engage_cm`, so it engages and stays
/// engaged instead of standing a few metres outside the band for ever — **17b
/// cut M4.4, on the unit mind.** The stock mind's `Approach` drives for
/// `stand_off_of(.., stand_off_want(engage_cm, ..))`, three cells inside the
/// band, and its `Engage` holds with the goal `here` and a stop. Every claim
/// is the original's: `Engage` inside the band after the thirteen-cell
/// approach, `Engage` after each of forty more pulses (a per-landing read of
/// the doctrine state) and a hull holding, not closing. What moved: the field
/// is [`t1_e1_duel`]'s mind under `step_settled`; the 30,000 horizon is the
/// original's and has the think latency's slack.
#[test]
fn t1_an_approach_that_arrives_is_inside_the_band_and_stays_there() {
    // Thirteen cells apart: the machine has a real approach to make.
    let mut arena = t1_e1_duel((2, 8), (15, 8), &[]);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 30_000, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "it closed the thirteen cells and engaged"
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range <= ENGAGE_CM,
        "and it is inside the band it aimed for: {range} cm"
    );

    // Twenty more seconds against a stationary target: no flapping. Without the
    // margin the goal would sit AT `engage_cm`, and an arrived machine — the
    // arrive window is one cell Chebyshev and the goal is a cell centre — would
    // stand a few metres outside it, in `Approach`, never firing.
    let mut wall_ms = arena.now_ms;
    for _ in 0..40 {
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        assert_eq!(
            machine_state_of(&arena, RED),
            Some(MachineState::Engage),
            "it fell out of the band at {} ms, {} cm out",
            arena.now_ms,
            plot_range(&arena, RED, BLUE)
        );
        assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
        wall_ms += 500;
    }
    assert_eq!(
        arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "and it is holding, not still closing"
    );
}

/// **`T-46`: a tank whose long gun is dry closes to the band of the one it can
/// actually fire** (16b cut E.1).
///
/// Every distance is priced off `UnitState::loaded_weapon`, so a machine with an
/// empty cannon and a loaded carbine stops holding 7,000 cm and comes in to the
/// carbine's own band. At HEAD the reach was the longest gun the platform
/// carried, loaded or not, and `engage_orders`' `max_range_cm >= range` filter
/// then refused every weapon it had — a machine that sat at cannon range and
/// never fired.
///
/// **17b cut M4.4 — on the unit mind.** `stock_mind::loaded_weapon(view)`
/// (the lifted `loaded_weapon_of`) falls through the empty cannon to the
/// coaxial, so `view_ranges` prices every distance off the coaxial's band and
/// `is_dry` is false. Every claim is the original's: it closes to inside the
/// coaxial's engage band, engages, fires the gun it has, and the cannon's
/// magazine reads unchanged at nought. What moved, and why: the coaxial is
/// [`t1_e1_coaxial`]'s eight cells and not five (the `range_round`
/// divergence: no band under 5,000 cm can be read inside on the view, so a
/// five-cell gun's 3,500 cm band could never be entered), and because the
/// view's 50 m step makes the Engage edge itself fall where the true range
/// drops under 6,875 cm for an eight-cell gun and a ten-cell cannon alike,
/// the claim that the distance is the LOADED gun's is pinned on the
/// approach's goal as well: the mind hands the motor a cell `stand_off_want`
/// of the coaxial's `engage_cm` short of the plot — 2,600 cm, where the
/// cannon's would be 4,000 — so the target opens the match nine cells off
/// (inside the sight, reading 100 m) with an approach to price.
#[test]
fn t1_t46_a_dry_cannon_closes_to_the_coaxial_and_fires_it() {
    let mut arena = t1_e1_duel_with(
        vec![cannon(), t1_e1_coaxial()],
        (2, 8),
        (11, 8),
        &[],
        SIGHT_CELLS,
    );
    // The cannon is empty; the eight-cell coaxial is not.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
        state.ammo.insert(SmolStr::new("coaxial"), 40);
    }
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 30_000, &mut bytes);

    // The coaxial's band is 8,000 cm, inside the sight, so the mind's whole
    // fighting reach is 8,000 cm and its engagement band is 5,600 — where with
    // a loaded cannon it would have been 7,000.
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range <= T1_COAXIAL_ENGAGE_CM,
        "it should have closed to the coaxial's own band, and it is {range} cm out"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    // The approach was priced off the coaxial: the cell the mind handed the
    // motor is 2,600 cm short of the plot (`stand_off_want(5_600, ..)`) —
    // 8,900 cm along the row, column 8; the cannon's 4,000 would have put it
    // at 7,500, column 7.
    let goals: Vec<(u16, u16)> = stock
        .kept()
        .iter()
        .flat_map(|kept| kept.3.intents.iter())
        .filter_map(|intent| match intent {
            tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell {
                x,
                y,
            }) => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert!(
        goals.contains(&(8, 8)),
        "the approach's goal is priced off the coaxial's band: {goals:?}"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(count(&log, "fired") > 0, "and it fired the gun it has");
    // The cannon stayed empty, so every one of those rounds was the coaxial's.
    assert_eq!(
        arena.unit(RED).map(|unit| unit.weapon_ammo("cannon")),
        Some(0)
    );
    assert!(
        arena
            .unit(RED)
            .is_some_and(|unit| unit.weapon_ammo("coaxial") < 40)
    );
}

// ---------------------------------------------------------------------------
// 16b cut E.1 — the two `Withdraw` stands (`B-39`, 16c section 5's Q8)
// ---------------------------------------------------------------------------

/// A hurt withdrawing machine with a classified hostile at `blue`: how many
/// rounds it fired, how much ground west it made, and **whether it ever
/// stood** — the memory's `stand_until_ms`, which is the stand itself rather than
/// its consequence. The two are read separately because `engage_orders` has a
/// firing ceiling of its own: a case that asserted only "nothing was fired"
/// would pass for a machine that stopped dead in front of a gun it could not
/// answer, which is the parked target 16b cut Q.7 was paid to remove.
///
/// `fighting` chooses the recipe — [`hurt_but_fighting`] at 277 per mille or
/// [`hurt`] at 166, either side of `stand_min_permille`'s 175 — and `home` opens
/// the machine in its own entry zone, which is the *arrived* arm of the stand.
///
/// **Both tanks are put where the case wants them after the power-up run**, and
/// the reason is the fixture rather than the cut: a machine spends its first
/// three seconds patrolling toward a waypoint, and on this window the nearest
/// node to column 9 is the eastern one — so a case written as "nine cells out,
/// hostile five cells east" would in fact be measured at whatever range the
/// patrol had drifted to. A stand is a range test now, so the range has to be
/// the one the case names.
///
/// **17b cut M4.4 — on the unit mind.** The machine is a mind seeing
/// [`T1_STAND_SIGHT_CELLS`] with the hostile at `blue`, a cell of the window
/// and not a column offset; `stood` is the stock memory's `stand_until_ms`
/// (the stand itself, as the reflex row's was) at the end of the run, the
/// health the figure the mind's first think after the wound read
/// (`stock_mind::health_permille` over the kept view), and the state the
/// doctrine state. The pose reset after the power-up run is kept, for the
/// reason above.
fn t1_a_withdrawal_beside(
    gun: WeaponSummary,
    blue: (u16, u16),
    fighting: bool,
    home: bool,
    run_ms: i64,
) -> (usize, i64, bool) {
    let at = if home { 1 } else { 9 };
    let mut arena = t1_e1_duel_with(vec![gun], (at, 8), blue, &[], T1_STAND_SIGHT_CELLS);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(at, 8);
        state.speed_cms = 0;
    }
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(blue.0, blue.1);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    if fighting {
        hurt_but_fighting(&mut arena, RED);
    } else {
        hurt(&mut arena, RED);
    }
    let thought = stock.kept().len();
    let from = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + run_ms;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        stock
            .kept()
            .get(thought)
            .map(|kept| tacsim::arena::stock_mind::health_permille(&kept.2)),
        Some(if fighting { 277 } else { 166 }),
        "the recipe this case asked for, read by the first think after the wound"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Withdraw),
        "every case here is about what a withdrawal does, so it has to be one"
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Withdraw, "{memory:?}");
    (
        count(&events(&mut arena, &mut bytes), "fired") - before,
        from - arena.unit(RED).map_or(0, |unit| unit.pose.x_cm),
        memory.stand_until_ms.is_some(),
    )
}

/// **The stand fixture's sight on the unit mind** (17b cut M4.4): sixteen
/// cells, so the en-route band (`stand_cm`, a third of the reach) is 5,333
/// cm and a view can read something inside it — a contact's range reaches a
/// think rounded to 50 m and floored at one step (the `range_round`
/// divergence), so the 3,333 cm band of a ten-cell sight admits nothing at
/// all.
const T1_STAND_SIGHT_CELLS: u16 = 16;
/// `fight_inner_cm` at [`T1_STAND_SIGHT_CELLS`].
const T1_STAND_REACH_CM: i64 = 16_000;
/// `Ranges::engage_cm` at [`T1_STAND_SIGHT_CELLS`].
const T1_STAND_ENGAGE_CM: i64 = 11_200;
/// `Ranges::stand_cm` at [`T1_STAND_SIGHT_CELLS`]: a third of the reach.
const T1_STAND_CM: i64 = 5_333;
/// **The least range a view says** (`CONTACT_RANGE_STEP_M` = 50 m), and the
/// range a plot must be held past for a figure held at 50 m to be re-read:
/// five eighths of a step beyond it (`held_range_m`).
const T1_RANGE_FLOOR_CM: i64 = 5_000;
const T1_RANGE_PAST_FLOOR_CM: i64 = 8_125;

/// **The stands, and the ones that are not** (16b cut E.1; 16c `B-39`, 16c
/// section 5's Q8) — **17b cut M4.4, on the unit mind.**
///
/// En route it turns only for something inside the `close` band and only while
/// it has plate left; once home it shoots back at anything inside its firing
/// ceiling, at any health (`withdraw_stand`: inside `stand_cm` at
/// `stand_min_permille` or better en route, inside `release_cm` at any health
/// arrived). At HEAD of cut E.1 the ceiling was the platform's whole 3 km
/// reach and there was no health floor, so every one of the five cases below
/// stood and fought. The five cases and every threshold are the original's,
/// none lowered.
///
/// What moved, and why: the sight is [`T1_STAND_SIGHT_CELLS`] (sixteen), so
/// the en-route band is 5,333 cm and the view's 50 m floor sits inside it;
/// and the cases' geometry is re-derived against that floor and its
/// hysteresis — a hostile anywhere under 7,500 cm reads 50 m (a stand), and
/// one the view has held at 50 m is re-read only past 8,125 cm — so (b)'s
/// "outside the band, inside the engage reach" hostile is the far corner
/// (15, 15), 9,220 cm from column 9 (reads 100 m: past 5,333 and inside
/// 11,200), where five cells on the row would read 50 m and stand. (e)'s gun
/// is the five-cell [`coaxial`] still: eight cells read 100 m, past a 5,000
/// cm ceiling exactly as before.
#[test]
fn t1_a_withdrawal_stands_for_what_is_close_on_the_way_out_and_for_anything_at_home() {
    // The geometry the cases are built on, said once: two cells reads the
    // floor, inside the en-route band; the far corner from column 9 is past
    // the figure a held 50 m is re-read at, reads 100 m, outside the band and
    // inside the reach the same machine would ENGAGE at.
    const { assert!(T1_RANGE_FLOOR_CM <= T1_STAND_CM) };
    assert!(2 * i64::from(CELL_CM) <= T1_RANGE_FLOOR_CM);
    let corner = ((15_i64 - 9).pow(2) + (15_i64 - 8).pow(2)).isqrt() * i64::from(CELL_CM);
    assert!(corner > T1_RANGE_PAST_FLOOR_CM);
    const { assert!(2 * T1_RANGE_FLOOR_CM > T1_STAND_CM) };
    const { assert!(2 * T1_RANGE_FLOOR_CM <= T1_STAND_ENGAGE_CM) };
    assert!(corner <= T1_STAND_REACH_CM);

    // (a) En route, above the floor, a hostile two cells out — 2,000 cm, which
    //     reads 50 m, inside the band's 5,333: it stands and shoots.
    let (fired, west, stood) = t1_a_withdrawal_beside(cannon(), (11, 8), true, false, 8_000);
    assert!(fired > 0, "inside the close band it turns and fights");
    assert!(west < 2_000, "and it is standing: it made {west} cm");
    assert!(stood, "and the stand is a stand, not a lucky shot");

    // (b) En route, above the floor, a hostile at the far corner — 9,220 cm,
    //     which reads 100 m, outside the band: it drives on. That is still
    //     well inside the reach it would ENGAGE at (11,200 cm), which is the
    //     whole point of a narrower band for a machine that is running away.
    let (fired, west, stood) = t1_a_withdrawal_beside(cannon(), (15, 15), true, false, 6_000);
    assert_eq!(fired, 0, "past the close band it does not stop for it");
    assert!(west > 3_000, "it drove west instead: {west} cm");
    assert!(!stood, "and it never stopped for it either");

    // (c) En route, below the floor, point blank: it keeps driving. A machine
    //     down to its last plate does not turn round for anything. The horizon is
    //     short enough that it has not reached its entry zone, so this is the
    //     en-route arm and not the arrived one.
    let (fired, west, stood) = t1_a_withdrawal_beside(cannon(), (10, 8), false, false, 6_000);
    assert_eq!(fired, 0, "under `stand_min_permille` it will not turn");
    assert!(west > 3_000, "it drove west instead: {west} cm");
    assert!(!stood, "and it never stopped for it either");

    // (d) Home, below the floor, a hostile four cells out — outside the close
    //     band's reading and inside the reach: it stands and shoots back. It
    //     has nowhere further to run, and a machine shot at and never replying
    //     is a parked target.
    let (fired, _, stood) = t1_a_withdrawal_beside(cannon(), (5, 8), false, true, 8_000);
    assert!(fired > 0, "arrived home, it shoots back at any health");
    assert!(stood, "and it is the arrived stand that did it");

    // (e) Home, below the floor, a hostile past its firing ceiling: it does
    //     not. The gun is the five-cell coaxial so that the case is a real one
    //     — the hostile is eight cells out, which its sight holds comfortably
    //     and its firing ceiling does not reach.
    //
    //     Both halves are asserted, and the second is the one with teeth: the
    //     firing ceiling alone would keep `fired` at nought even if the arrived
    //     arm range-gated at nothing at all, and a machine that stopped and
    //     turned here would be standing in front of a gun it cannot answer.
    let (fired, _, stood) = t1_a_withdrawal_beside(coaxial(), (9, 8), false, true, 8_000);
    assert_eq!(
        fired, 0,
        "past its firing ceiling there is nothing to shoot"
    );
    assert!(
        !stood,
        "and it did not stop for it: a stand it could stage no shot from is a parked target"
    );
}

// ---------------------------------------------------------------------------
// 16b cut E.1 — `T-32`: the alert range and the `Engaged` trigger
// ---------------------------------------------------------------------------

/// **The derived `tempo.alert_range_cm` is capped at the window's diagonal, and
/// the `Engaged` trigger reads the band a platform fights in** (16b cut E.1; 16c
/// `T-32`).
///
/// **17b cut M4.5a** (Q27; was `e1_t32_…`, a keep row of 17b cut M4.4): the
/// machine is a unit mind, fielded by the test's own builder below — [`duel`]
/// itself stays on the reflex for STOP B's
/// `aa1_a_hull_authored_with_no_speed_engages_the_same_way`. Nothing is stepped,
/// so no think runs and the controller is no input to either claim: the alert
/// range is derived in `Arena::new`, and `hostile_in_weapons_range` walks every
/// unit by its `fight_inner_cm` and its held contacts. The field is
/// [`duel`]'s exactly (the cannon at [`SIGHT_CELLS`], a target tank with the
/// default profile), every constant and claim unchanged.
#[test]
fn t1_t32_the_alert_range_is_the_window_and_the_engaged_trigger_is_the_band() {
    // The sixteen-cell window of 1,000 cm cells: `isqrt(16² + 16²)` is 22, so
    // 22,000 cm — against the cannon's own 2 × 250,000 = 500,000 cm.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(2, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(14, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    assert_eq!(arena.alert_range_cm, 22_000, "the window's own diagonal");

    // A configured key is used exactly as written, derived or not.
    let configured = ArenaConfig {
        tempo: tacsim::config::TempoConfig {
            alert_range_cm: 1_234_567,
            ..tacsim::config::TempoConfig::default()
        },
        ..arena_cfg(reflex_cfg())
    };
    let named = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(2, 8),
            vec![cannon()],
            Controller::Mind,
        )],
        &[],
        &configured,
        &[],
    );
    assert_eq!(named.alert_range_cm, 1_234_567);

    // The `Engaged` trigger: 12,000 cm is past `REACH_CM` and far inside the
    // cannon's own 250,000 cm, so the machine can hold it and cannot fight it.
    // Nothing is stepped first, so the two poses are the ones the fixture wrote.
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    assert_eq!(plot_range(&arena, RED, BLUE), 12_000);
    assert!(
        !arena.hostile_in_weapons_range(),
        "a contact it cannot shoot at is not `hostile_in_weapons_range`"
    );
    // And inside the band it is.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(7, 8);
    }
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    assert_eq!(plot_range(&arena, RED, BLUE), 5_000);
    assert!(arena.hostile_in_weapons_range());
}

// ---------------------------------------------------------------------------
// 16b cut E.1 — cross-tempo (16b section 11, mandatory)
// ---------------------------------------------------------------------------

/// **Cut E.1's `Approach`/`Engage` arc, as one scripted arena**: a machine
/// closes on a pilot's tank five cells away, engages at `COAXIAL_ENGAGE_CM`, and
/// at [`SCRIPT_AT_MS`] the tank is put **past `COAXIAL_RELEASE_CM` and in the
/// open**, so the machine gives the band up, closes again and re-engages.
///
/// The gun is the five-cell [`coaxial`] and not the fixture cannon, and that is
/// the whole design of this fixture: with the cannon the firing ceiling *is* the
/// sight, so a target past `release_cm` is a target the sensor pass cannot see —
/// the plot stops being refreshed, the range the `Engage` arm reads is the stale
/// one, and the release edge can never fire. A gun whose inner band is inside
/// the scope is the shape in which "it opened the range on me" is a thing a
/// machine can observe at all.
///
/// The one thing left out is the no-line arm (the goal becoming the plot's own
/// cell), for the same reason: a plot only moves when a pass refreshes it, so
/// *past the release edge* and *behind a wall* cannot be the same scripted
/// moment. `e1_with_range_and_no_line_it_closes_on_the_plot` pins that arm, and
/// it is a goal assignment inside one decide tick rather than anything scheduled
/// — there is no cadence for it to be sensitive to.
///
/// **17b cut M4.4 — on the unit mind.** The gun is [`t1_e1_coaxial`]'s eight
/// cells (its doc: no five-cell band can be entered on the view) and the
/// mind opens at **(6, 8)**, a cell from the tank, instead of (2, 8), five
/// off: the view's figure for the tank, held at 50 m through the engagement,
/// is re-read only when the plot is past 8,125 cm (`held_range_m`), and the
/// pass refreshes a plot only inside the ten-cell sight — so the cell the
/// tank breaks to, (13, 8), must be past the 8,000 cm ceiling and past
/// 8,125, and at most 10,000 cm, from where the mind engaged. It patrols
/// west for the seconds the classification takes and engages where it is
/// then: from column 2 that is column 1 or 2, eleven cells off the break,
/// which the pass never sees; from column 6 it is column 4, 8,850 cm off,
/// with seven hundred centimetres to spare on the near side and twelve on
/// the far.
fn t1_close_engage_and_release() -> Arena {
    t1_e1_duel_with(vec![t1_e1_coaxial()], (6, 8), (7, 8), &[], SIGHT_CELLS)
}

/// `Ranges::engage_cm` for a machine holding the [`coaxial`]: seven tenths of its
/// 5,000 cm band, which is inside [`SIGHT_CELLS`].
const COAXIAL_ENGAGE_CM: i64 = 3_500;
/// `Ranges::release_cm` for the same machine — the band itself, and half of what
/// its scope can hold.
const COAXIAL_RELEASE_CM: i64 = 5_000;

/// The blue tank opens the range well past the release edge, in the open, where
/// the machine's own pass will see it go.
fn blue_breaks_contact(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(13, 8);
    }
}

/// **The arc the three replays exist to compare, walked once at one cadence.**
///
/// Without it the cadences could agree perfectly on a log in which the machine
/// approached, engaged and then did nothing at all for twenty seconds: the
/// event kinds a replay journals (`detected`, `classified`, `locked`, `fired`)
/// are the ones a plain approach produces, and none of them is evidence that the
/// release edge fired. Each pulse's reading of the machine: the time, its state
/// and its range to the tank.
///
/// **17b cut M4.4**: the mind's arc, each pulse's reading of the doctrine
/// state under `step_settled`.
fn t1_engage_release_arc(cadence_ms: i64, until_ms: i64) -> Vec<(i64, MachineState, i64)> {
    let mut arena = t1_close_engage_and_release();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut seen: Vec<(i64, MachineState, i64)> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if arena.now_ms == SCRIPT_AT_MS {
            blue_breaks_contact(&mut arena);
        }
        if let Some(state) = machine_state_of(&arena, RED) {
            seen.push((arena.now_ms, state, plot_range(&arena, RED, BLUE)));
        }
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    seen
}

/// **17b cut M4.4 — cut E.1's band taken and given up, on the unit mind, one
/// event list and one hash sequence at three cadences.** The arc is T1's:
/// the `Approach` arm enters `Engage` inside `engage_cm`; at
/// [`SCRIPT_AT_MS`] the tank opens the range past `release_cm` in the open,
/// its plot refreshed because the gun's ceiling is inside the sight, and the
/// `Engage` arm releases on `range > release_cm` off the view's row; the
/// approach closes again and re-engages. Every claim is the original's: the
/// three moments of the arc at one cadence, the five kinds the log must
/// carry, at least a hundred substep hashes, and the three-way equality of
/// events, hashes and finals (the finals' state the doctrine state).
/// What moved: the gun and the opening cell ([`t1_close_engage_and_release`]'s
/// doc) and the band's two figures with them ([`T1_COAXIAL_ENGAGE_CM`],
/// [`T1_COAXIAL_RELEASE_CM`]); 20,000 is a multiple of all three cadences.
#[test]
fn t1_the_band_taken_and_given_up_is_one_event_list_at_three_cadences() {
    // First: the arc really happens, at one cadence, before anything is compared.
    let arc = t1_engage_release_arc(1_000, 40_000);
    assert!(
        arc.iter().any(|(at, state, range)| *at < SCRIPT_AT_MS
            && *state == MachineState::Engage
            && *range <= T1_COAXIAL_ENGAGE_CM),
        "it never engaged inside the band before the release: {arc:?}"
    );
    assert!(
        arc.iter().any(|(at, state, range)| *at > SCRIPT_AT_MS
            && *state == MachineState::Approach
            && *range > T1_COAXIAL_RELEASE_CM),
        "it never gave the band up when the tank opened the range past its firing ceiling: {arc:?}"
    );
    assert!(
        arc.iter().rev().any(|(at, state, range)| *at > SCRIPT_AT_MS
            && *state == MachineState::Engage
            && *range <= T1_COAXIAL_ENGAGE_CM),
        "it released the band and never closed again: {arc:?}"
    );

    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                40_000,
                t1_close_engage_and_release,
                blue_breaks_contact,
            )
        })
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "detected", "classified", "locked", "fired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    assert!(first.hashes.len() >= 100, "{} substeps", first.hashes.len());
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut E.2 — a dead target is knowledge (16c `B-45`; doc 16 §1, §5's Q11)
//
// Five live reads of the TARGET's own `limits.wreck` decided what a machine did
// with a contact, which is hidden truth steering a unit: a machine dropped a
// target the pulse it died, however stale its track, whoever killed it, through
// a wall and across the map, where a pilot in the same seat keeps the row on
// `CONTACTS` until it expires. They read the hold now, through
// `Arena::contact_reads_dead`.
// ---------------------------------------------------------------------------

/// **A profile whose hull is the whole of the unit**: every band a round can
/// arrive at reads `hull`, and `hull`'s consequence is `destroy_unit`. A round
/// that lands therefore kills, which is what makes the kill test below a bounded
/// number of pulses rather than a dice game.
fn body_profile() -> tacsim::arena::resolution::UnitProfile {
    let band = |name: &str| tacsim::arena::resolution::BandProfile {
        band: SmolStr::new(name),
        width_cm: 700,
        height_cm: 350,
        sections: vec![(SmolStr::new("hull"), 100)],
    };
    tacsim::arena::resolution::UnitProfile {
        bands: ["front", "left", "right", "rear", "top"]
            .into_iter()
            .map(band)
            .collect(),
        sections: vec![tacsim::arena::resolution::SectionProfile {
            id: SmolStr::new("hull"),
            facets: ["front", "left", "right", "rear", "top"]
                .into_iter()
                .map(SmolStr::new)
                .collect(),
            consequences: vec![tacsim::arena::resolution::Consequence::DestroyUnit],
            struck_through: None,
        }],
    }
}

/// The unit that goes with it: [`tactical_unit`]'s one hull widened to five
/// facets of ten, so a hit on any band finds armour to pass.
fn body_unit(id: ObjId) -> TacticalUnit {
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: ["front", "left", "right", "rear", "top"]
                .into_iter()
                .map(|facet| FacetSpec {
                    id: FacetId::new(facet).expect("a canonical facet"),
                    cells: vec![CellSpec {
                        id: mush_tactics::CellId::new("armor").expect("a cell"),
                        min: 0,
                        max: 10,
                        init: 10,
                    }],
                })
                .collect(),
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit a round can kill")
}

/// A tank a round can kill.
fn killable(mut unit: CustodyUnit) -> CustodyUnit {
    unit.unit = body_unit(unit.id);
    unit.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    unit.profile = body_profile();
    unit
}

/// **A two-section unit**, `hull` and `turret`. `TacticalUnit::sections` answers
/// in section-id order, so `hull` is index 0 and `turret` index 1 — the indices a
/// `Condition`'s bands are packed at and the bits `UnitState::fatal_sections`
/// numbers.
fn hull_and_turret(id: ObjId) -> TacticalUnit {
    let section = |name: &str| SectionSpec {
        id: SectionId::new(name).expect("a canonical section"),
        condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
        facets: vec![FacetSpec {
            id: FacetId::new("front").expect("a canonical facet"),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("armor").expect("a cell"),
                min: 0,
                max: 10,
                init: 10,
            }],
        }],
        slots: Vec::new(),
        cells: vec![CellSpec {
            id: mush_tactics::CellId::new("internals").expect("a cell"),
            min: 0,
            max: 8,
            init: 8,
        }],
    };
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![section("hull"), section("turret")],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a two-section unit")
}

/// The profile that makes `hull` the body and the turret merely a gun mount:
/// `UnitState::fatal_sections` is bit 0 alone.
fn hull_is_the_body() -> tacsim::arena::resolution::UnitProfile {
    let section = |name: &str, consequences: Vec<tacsim::arena::resolution::Consequence>| {
        tacsim::arena::resolution::SectionProfile {
            id: SmolStr::new(name),
            facets: vec![SmolStr::new("front")],
            consequences,
            struck_through: None,
        }
    };
    tacsim::arena::resolution::UnitProfile {
        bands: Vec::new(),
        sections: vec![
            section(
                "hull",
                vec![tacsim::arena::resolution::Consequence::DestroyUnit],
            ),
            section(
                "turret",
                vec![tacsim::arena::resolution::Consequence::RemoveWeapons {
                    section: SmolStr::new("turret"),
                }],
            ),
        ],
    }
}

/// A tank whose hull kills it and whose turret does not.
fn two_sectioned(mut unit: CustodyUnit) -> CustodyUnit {
    unit.unit = hull_and_turret(unit.id);
    unit.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    unit.profile = hull_is_the_body();
    unit
}

/// **A five-metre pistol**: a weapon a tank carries so that it has a fighting
/// reach and therefore a target, and never a shot — every distance in these
/// fixtures is further than its one band.
fn pistol() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("pistol"),
        max_range_cm: 500,
        bands: vec![WeaponBand {
            max_range_cm: 500,
            dispersion_mrad: 3,
        }],
        ..cannon()
    }
}

/// A wall down column 9, every row: nothing west of it ever has a line to
/// anything east of it.
fn curtain() -> Vec<(u16, u16, TerrainCell)> {
    (0..SIDE).map(|y| (9_u16, y, wall())).collect()
}

/// **A second blue tank, parked out of everybody's sight.**
///
/// Doc 16 §3.8's elimination ends a match the moment one side has nothing left,
/// and an ended arena steps no further — so a fixture whose whole subject is
/// what happens in the seconds *after* a kill needs the other side to still
/// exist. It carries no weapon, it is never detected and nothing ever targets
/// it.
const BLUE2: ObjId = ObjId(11);

/// That tank, at `cell`.
fn bystander(cell: (u16, u16)) -> CustodyUnit {
    tank(
        BLUE2,
        Team::Blue,
        centre(cell.0, cell.1),
        Vec::new(),
        Controller::Pilot,
    )
}

/// **What the mind settled on** (17b cut M4.4): the memory's `target` — a
/// contact id of the hull's own row — resolved through the hull's own
/// contact store to the object behind it, where the reflex row carried the
/// object itself. A memory naming a row its store no longer holds is a read
/// between an expiry and the think that reads it, which no fixture below
/// reads at: it panics rather than answering `None`, so a dropped target is
/// always the mind's own doing and never the store's.
fn target_of(arena: &Arena, unit: ObjId) -> Option<ObjId> {
    stock_memory_of(arena, unit).target.map(|id| {
        arena
            .contacts
            .get(unit, id)
            .map(|held| held.target)
            .unwrap_or_else(|| {
                panic!("the memory of {unit:?} names a contact its store no longer holds")
            })
    })
}

/// What `viewer` holds on `target`, if anything.
fn hold_on(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<ShadowContact> {
    let id = arena.contacts.id_of(viewer, target)?;
    arena.contacts.get(viewer, id).copied()
}

/// Write a reading onto `viewer`'s hold on `target` with one section destroyed —
/// the shape a close look, or a section the gunner watched come apart, leaves
/// behind.
fn plant_reading(arena: &mut Arena, viewer: ObjId, target: ObjId, destroyed: usize, at_ms: i64) {
    let id = arena
        .contacts
        .id_of(viewer, target)
        .expect("the viewer holds the target");
    if let Some(held) = arena.contacts.get_mut(viewer, id) {
        held.seen_condition = Some(tacsim::arena::condition::Reading {
            at_ms,
            condition: tacsim::arena::condition::Condition {
                bands: 0,
                sections: 2,
            }
            .with_band(
                destroyed,
                tacsim::arena::condition::ConditionBand::Destroyed,
            ),
        });
    }
}

/// Mark `viewer`'s hold on `target` the way the kill line's own arm does.
fn learn_the_kill(arena: &mut Arena, viewer: ObjId, target: ObjId) {
    let id = arena.contacts.id_of(viewer, target).expect("the hold");
    if let Some(held) = arena.contacts.get_mut(viewer, id) {
        held.known_dead = true;
    }
}

/// **The first landing after `from_ms` of each of `units`** (17b cut M4.4):
/// one substep (250 ms) at a time, settled, until every one of `units` has a
/// think kept at a millisecond past `from_ms` whose landing (one substep on)
/// has passed — the mind's "next tick" after a fact written by `from_ms`. A
/// think inside the fact's own pulse may or may not have read it; one past
/// the pulse's close has. Two seconds is the bound: a think every 1,000 ms,
/// landed 250 ms on.
fn t1_landed_after(
    arena: &mut Arena,
    stock: &Stock,
    units: &[ObjId],
    from_ms: i64,
    bytes: &mut Vec<u8>,
) {
    let mut wall = arena.now_ms;
    loop {
        let kept = stock.kept();
        let landed = units.iter().all(|unit| {
            kept.iter().any(|think| {
                think.0 == *unit
                    && think.1 > from_ms
                    && arena.now_ms >= think.1 + arena.substep_len_ms()
            })
        });
        if landed {
            return;
        }
        assert!(
            arena.now_ms - from_ms <= 2_000,
            "no think of {units:?} landed inside two seconds of {from_ms} ms"
        );
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        assert!(out.ended.is_none(), "the match ended under the fixture");
        wall += 250;
    }
}

/// **17b cut M4.4 — (a) a target that dies out of sight is held until the
/// contact expires, on the unit mind** (16b cut E.2; 16c `B-45`). Every claim
/// of the reflex fixture is kept: the mind holds a planted classified contact
/// on a tank behind the curtain and closes on it; the tank destroyed by
/// nobody it could have watched (the rig's own `destroy_unit` with no
/// shooter: no `Kill` line, no `known_dead`) is still its target and it is
/// still in `Approach`, the hold neither `known_dead` nor reading dead; and
/// it lets go when the track does, three passes after the last sighting,
/// back in `Patrol`. What moved: the hull is a `Controller::Mind`; the state
/// is the mind's cached doctrine state (`machine_state_of`) and the target
/// the memory's ([`target_of`], the memory's contact id resolved through the
/// hull's own store), where the reflex row's were read. The mind reads the
/// hold off its view's `reads_dead` (step.rs' `frame_scope`: the wreck is
/// still a unit, so the row stays on the frame, and an armed hull with no
/// `known_dead` and no reading builds `Some(false)`; the `lost` edge drops a
/// row only on `Some(true)`), and acts at a landing: a think every 1,000 ms
/// at `think_at(RED, n)` (482, 1,482, …), landed 250 ms on. Each of the
/// reflex's `+1,000` windows already holds the landing after its plant (a
/// plant at a pulse's close `x,000` is read by the think at `x,482` and
/// landed at `x,732`), and the expiry's `+6,000` holds the landing after the
/// loss, so no window moved; the pulses are `step_settled` (`run_settled`),
/// as every mind fixture's are.
#[test]
fn t1_a_target_that_dies_out_of_sight_is_held_until_the_contact_expires() {
    // (a) The mind holds a planted classified contact on a tank behind a
    // curtain of wall: no line, no sight, so nothing will refresh or correct it.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            killable(tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                Vec::new(),
                Controller::Pilot,
            )),
            bystander((15, 15)),
        ],
        &curtain(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run_settled(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Approach));
    assert_eq!(target_of(&arena, RED), Some(BLUE));

    // Killed by nobody this mind could have watched: the rig's own destroy,
    // with no shooter, so no `Kill` line and no `known_dead`.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, at, None, &mut journal);
    run_settled(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(
        target_of(&arena, RED),
        Some(BLUE),
        "the mind dropped a target on a fact no platform on the field had"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Approach));
    let held = hold_on(&arena, RED, BLUE).expect("the hold is still there");
    assert!(!held.known_dead, "nobody told it");
    assert!(!arena.contact_reads_dead(&held));

    // And it lets go when the track does, three passes after the last sighting
    // and not a millisecond sooner: the think after the loss reads no row.
    run_settled(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(arena.contacts.of(RED).count(), 0, "the track expired");
    assert_eq!(stock_memory_of(&arena, RED).target, None);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
}

/// **17b cut M4.4 — (d) a target that leaves the match is dropped at once,
/// on the unit mind** (16b cut E.2; 16c `B-45`). The claim is kept: *gone*
/// is not *dead* — there is no unit for the contact to be a fact about, so
/// the `lost` edge takes it at the mind's next landing whatever the hold
/// says (the hold itself stays on the store until it expires). What moved:
/// the hull is a `Controller::Mind`, the target and the state the memory's
/// ([`target_of`], `machine_state_of`). The mind never sees the row at all
/// — step.rs' `frame_scope` skips a contact whose unit has left the match —
/// so the `lost` edge's `contact(view, target).is_none_or(..)` drops it at
/// the think after the retain (`x,482` for a retain at `x,000`), landed at
/// `x,732`, inside the reflex's own `+1,000` window; stepped `step_settled`.
#[test]
fn t1_a_target_that_leaves_the_match_is_dropped_at_once() {
    // (d) *Gone* is not *dead*: there is no unit for the contact to be a fact
    // about, so the `lost` edge takes it at the next landing whatever the hold
    // says.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &curtain(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run_settled(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(target_of(&arena, RED), Some(BLUE));

    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    arena.units.retain(|unit| unit.id != BLUE);
    run_settled(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(stock_memory_of(&arena, RED).target, None);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
}

/// **17b cut M4.4 — (c) a reading that shows the body gone drops the target
/// and a turret does not, on the unit mind** (16b cut E.2; 16c `B-45`; 16b
/// cut Q.2's reading). Every claim is kept: the mask is the profile's
/// (`fatal_sections == 0b01`, `hull` the body and `turret` a gun mount), the
/// blue tank is alive in both halves, the hold is not `known_dead`, and
/// `Arena::contact_reads_dead` reads the planted reading the way the drop
/// goes — the body destroyed drops the target, the turret destroyed keeps
/// it. What moved: the hull is a `Controller::Mind` and the target is the
/// memory's ([`target_of`]) at the landing after the reading was planted
/// (think `x,482` for a plant at `x,000`, landed `x,732`, inside the
/// reflex's own `+1,000`); the mind reads the view's `reads_dead`, which
/// step.rs' `frame_scope` builds off the held reading's destroyed mask
/// against the target chassis's fatal mask — value-identical to
/// `contact_reads_dead`, pinned arm by arm by step.rs' own m1a3 frame-scope
/// test — so the two arena-side pins stay beside the memory's drop. The
/// `contact_reads_dead` pins mirror the PREDICATE; this fixture is the drop
/// edge, end to end.
#[test]
fn t1_a_reading_that_shows_the_body_gone_drops_the_target_and_a_turret_does_not() {
    // (c) The knowledge a close look earns. The blue tank is NOT a wreck in
    // either half: what the mind acts on is its own reading, masked with the
    // target's `fatal_sections` — `hull` is the body, `turret` is a gun mount.
    let build = || {
        arena_of(
            vec![
                with_sight(
                    tank(
                        RED,
                        Team::Red,
                        centre(6, 8),
                        vec![cannon()],
                        Controller::Mind,
                    ),
                    SIGHT_CELLS,
                ),
                two_sectioned(tank(
                    BLUE,
                    Team::Blue,
                    centre(12, 8),
                    Vec::new(),
                    Controller::Pilot,
                )),
            ],
            &curtain(),
            &arena_cfg(reflex_cfg()),
            &[BLUE],
        )
    };
    // The mask is the profile's, laid down at check-out and never moved.
    assert_eq!(
        build().unit(BLUE).map(|unit| unit.fatal_sections),
        Some(0b01),
        "`hull` is section 0 and the only one that kills"
    );

    for (destroyed, still_held) in [(0_usize, false), (1, true)] {
        let mut arena = build();
        let mut bytes: Vec<u8> = Vec::new();
        run_settled(&mut arena, 2_000, &mut bytes);
        let now = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
        run_settled(&mut arena, now + 1_000, &mut bytes);
        assert_eq!(target_of(&arena, RED), Some(BLUE));

        let at = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
        plant_reading(&mut arena, RED, BLUE, destroyed, at);
        let held = hold_on(&arena, RED, BLUE).expect("the hold");
        assert!(!held.known_dead, "this half is about the reading alone");
        assert_eq!(arena.contact_reads_dead(&held), !still_held);
        run_settled(&mut arena, at + 1_000, &mut bytes);
        assert_eq!(
            target_of(&arena, RED).is_some(),
            still_held,
            "section {destroyed} destroyed: the mind read it wrong"
        );
        assert!(
            arena.unit(BLUE).is_some_and(|unit| !unit.limits.wreck),
            "the target is alive in both halves, which is what makes this about the reading"
        );
    }
}

#[test]
fn e2_b45_the_two_tempo_triggers_read_the_hold_and_not_the_wreck() {
    // Doc 16 §3.1's `Alert` and third `Engaged` triggers, which are the other two
    // readers of the same sentence. How often a pilot is sent a frame is
    // something a pilot can count, so a tempo that fell at the arena millisecond
    // a hull burned would be hidden truth on a surface a pilot reads.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Pilot,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(7, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[RED, BLUE],
    );
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    assert!(arena.hostile_in_weapons_range());
    assert!(arena.contact_within_alert_range());

    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, 0, None, &mut journal);
    assert!(
        arena.hostile_in_weapons_range(),
        "the hull burned and the holder's track said so before the holder did"
    );
    assert!(arena.contact_within_alert_range());

    learn_the_kill(&mut arena, RED, BLUE);
    assert!(!arena.hostile_in_weapons_range());
    assert!(!arena.contact_within_alert_range());
}

/// **17b cut M4.4 — (e) a kill starts the rally clock and a death out of
/// sight does not, on the unit mind** (16b cut E.2; 16c `B-45`; 16b cut
/// Q.7's clock). Every claim is kept: a hurt mind standing in its own entry
/// zone is in `Withdraw` and has arrived, so the only thing between it and
/// its quiet clock is whether it still holds a live hostile; holding the
/// planted tank the clock is `None`; the tank destroyed by nobody it could
/// have watched leaves the clock `None`, because nothing has told it; the
/// kill written onto the hold starts the clock at once. What moved: the
/// hull is a `Controller::Mind`, the clock is the memory's `quiet_since_ms`
/// (`stock_memory_of`) and the state the cached doctrine state, read at the
/// pulse after the landing of the think after each fact (a plant, a
/// destroy, a mark at `x,000` is read by the think at `x,482`, landed
/// `x,732` — inside each of the reflex's `+1,000` windows, so none moved).
/// The mind's `Withdraw` arm mirrors the reflex's field for field
/// (stock_mind.rs: `holds_a_hostile` is a classified row of the other side
/// whose `reads_dead != Some(true)`; the view's `reads_dead` is step.rs'
/// `frame_scope`'s, `Some(false)` after the shooterless destroy and
/// `Some(true)` once `known_dead` is on the hold); stepped `step_settled`.
#[test]
fn t1_a_kill_starts_the_rally_clock_and_a_death_out_of_sight_does_not() {
    // (e) `holds_a_hostile` through the rally. A hurt mind standing in its own
    // entry zone has arrived, so the only thing between it and its quiet clock
    // is whether it still holds a live hostile — and *live* is what it knows.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            killable(tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                Vec::new(),
                Controller::Pilot,
            )),
            bystander((15, 15)),
        ],
        &curtain(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    hurt(&mut arena, RED);
    let quiet = |arena: &Arena| stock_memory_of(arena, RED).quiet_since_ms;

    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run_settled(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    assert_eq!(quiet(&arena), None, "it holds a live hostile");

    // Killed by somebody else, out of this mind's sight: the board is not
    // quiet yet, because nothing has told it.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, at, None, &mut journal);
    run_settled(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(
        quiet(&arena),
        None,
        "the clock started on a fact the mind had not been given"
    );

    // The same board with the kill written onto the hold, which is what the
    // `Kill` line the shooter's pilot reads says: the clock starts at the next
    // think.
    learn_the_kill(&mut arena, RED, BLUE);
    let from = arena.now_ms;
    run_settled(&mut arena, from + 1_000, &mut bytes);
    assert!(
        quiet(&arena).is_some(),
        "a mind that watched its own round finish the tank is still waiting"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
}

/// **17b cut M4.4 — (b) the shooter knows its own kill and a teammate waits
/// for the track, on the unit mind** (16b cut E.2; 16c `B-45`). Every claim
/// is kept, through the production path: the shooter's own `FIRE` — a mind's
/// cited shot on the slot it armed and the lock it took, resolved by
/// `Arena::resolve_fire`, whose `unit_destroyed` line carries `by` — writes
/// `known_dead` on the shooter's hold and no other (step.rs' `UnitDestroyed
/// { by }` arm, whoever the controller); the teammate's own-sight hold reads
/// alive; at the next landing the shooter is off the target and the
/// teammate is not; the teammate lets go when the track expires; one
/// `unit_destroyed` line. What moved: both reds are `Controller::Mind`
/// under the stock recorder; the targets are the two memories', read at
/// the first landing of each mind after the pulse the kill closed in
/// ([`t1_landed_after`]: the reflex's "one tick later", on the lattice —
/// a think inside the kill's own pulse may or may not have read it, the
/// first past its close has); the expiry's `+6,000` is the original's. The
/// shooter's view carries `reads_dead = Some(true)` off its `known_dead`,
/// the teammate's `Some(false)` (its pistol keeps the row built); the
/// kill is bounded by the hull that dies to any round, as before; stepped
/// `step_settled`.
#[test]
fn t1_the_shooter_knows_its_own_kill_and_a_teammate_waits_for_the_track() {
    // (b) The whole of it through the production path: a real `FIRE`, resolved by
    // `Arena::resolve_fire`, whose `destroyed` line carries `by`. The writer is
    // the notice scan's own arm, beside the `Kill` line the shooter's pilot gets.
    //
    // The second red mind here holds the tank through its **own** sight —
    // the mesh is off in this fixture, as it is in every other one in this file
    // ([`sight`]'s `share_range_cells` is nought), so what is proved is that the
    // kill is written on one hold and no other. The mesh's own arm is the next
    // test.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            with_sight(
                tank(
                    RED2,
                    Team::Red,
                    centre(2, 8),
                    vec![pistol()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            killable(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
            // Out of both minds' ten-cell scopes, so nothing ever holds it
            // and the only thing it does is keep the match from ending.
            bystander((15, 0)),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut ended = None;
    while arena.now_ms < 90_000 {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        // A match that has ended does nothing at all in `Arena::step` — its
        // clock stops — so a loop that only watched the hull would spin for
        // ever if the bystander ever stopped keeping blue on the board. It is
        // asserted rather than merely broken on, because an early end would
        // otherwise shorten this window in silence.
        if out.ended.is_some() {
            ended = out.ended;
            break;
        }
        if arena.unit(BLUE).is_some_and(|unit| unit.limits.wreck) {
            break;
        }
        wall += 500;
    }
    assert!(
        ended.is_none(),
        "the match ended under the fixture: {ended:?}"
    );
    assert!(
        arena.unit(BLUE).is_some_and(|unit| unit.limits.wreck),
        "the mind never killed it, so nothing was proved"
    );

    // The shooter's hold carries the kill, and the shooter's alone: the second
    // red mind watched nothing and holds a track that still says the tank is
    // there.
    let shooter = hold_on(&arena, RED, BLUE).expect("the shooter's hold");
    assert!(shooter.known_dead, "the gunner who killed it does not know");
    assert!(arena.contact_reads_dead(&shooter));
    let mate = hold_on(&arena, RED2, BLUE).expect("the teammate's hold");
    assert!(
        !mate.known_dead,
        "a kill travelled to a platform that did not see it"
    );
    assert!(
        !arena.contact_reads_dead(&mate),
        "the teammate was handed a fact its own sensors never took"
    );

    // At the landing after the kill the shooter is off it and the teammate is
    // not.
    let at = arena.now_ms;
    t1_landed_after(&mut arena, &stock, &[RED, RED2], at, &mut bytes);
    assert_eq!(
        stock_memory_of(&arena, RED).target,
        None,
        "the shooter is still on it"
    );
    assert_eq!(
        target_of(&arena, RED2),
        Some(BLUE),
        "the teammate let go of a track nothing had refreshed or contradicted"
    );

    // And the teammate lets go when the track expires: `sees_wrecks` is false, so
    // nothing refreshes it once the hull is a wreck.
    run_settled(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(stock_memory_of(&arena, RED2).target, None);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "unit_destroyed"),
        1,
        "the kill went through the engine's own line, once"
    );
}

/// **17b cut M4.4 — (b)'s other half: a share carries the track and never
/// the kill, on the unit mind** (16b cut E.2; 16c `B-45`; doc 16 §2 W6).
/// Every claim is kept, the mesh's arena-side: the teammate holds nothing
/// through the curtain; the sharer's planted hold with the kill written on
/// it is copied by `Arena::share_pass`'s insert arm as a sighting — no
/// `own_seen`, no reading, no `known_dead`, not reading dead — and the
/// update arm on the same row keeps the kill out of it; the sharer keeps
/// knowing its kill; the copy expires with the sharer's own stamp and not a
/// millisecond sooner. The machine's side: the teammate closes on the copy
/// (`Approach`, the target the copy's object), keeps closing on a tank that
/// is not there, and lets go in `Patrol` when the track does. What moved:
/// the teammate is a `Controller::Mind` under the stock recorder (the
/// sharer stays the piloted hull it was); its target and state are the
/// memory's, read at the original's two marks inside the copy's life and at
/// its end. RED2 thinks at `think_at(RED2, n)`, 4 + n·1,000, landed 250 ms
/// on; the copy is on its store from the sharer's pass at `+482` (the share
/// is scheduled at the pass's own millisecond), so the think at `+1,004`
/// reads it after the insert arm and lands at `+1,254`, before the `+1,500`
/// mark, and the think at `+2,004` reads it after the update arm and lands
/// at `+2,254`, before the `+2,500` mark — two landings, one per arm. No
/// landing after `+2,500` can read the copy: the think at `+3,004` falls on
/// the very millisecond of RED2's own sensor pass, which expires the
/// stamp-`at` copy first (the pass and the think are both staggered at
/// `stagger_ms(SEED, RED2, 1,000)`). The `+6,000` mark reads the think after
/// the loss, landed. The mind reads the
/// copy's `reads_dead = Some(false)` off its view; stepped `step_settled`.
#[test]
fn t1_a_share_carries_the_track_and_never_the_kill() {
    // (b)'s other half, and the one the mesh's own arm is about. In the test
    // above the second red mind has its **own** sighting of the tank; here it
    // has no line to it at any range — the curtain down column 9 sees to that —
    // so the only hold it has is the one `Arena::share_pass` built, and what
    // that arm writes into `known_dead` is the whole of what is under test.
    //
    // A share carries a **sighting**: the sharer's own stamp, its own plot and
    // its own reading (doc 16 §2 W6). A kill its gunner watched is not one of
    // those, so it does not travel, and the teammate keeps its track until the
    // track expires — which is doc 16 §1's observer rule applied to the
    // machines, and the door this cut's own field doc says is shut.
    let mut arena = arena_of(
        vec![
            with_mesh(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Pilot,
                ),
                SIGHT_CELLS,
                5,
            ),
            with_mesh(
                tank(
                    RED2,
                    Team::Red,
                    centre(4, 8),
                    vec![pistol()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
                5,
            ),
            killable(tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                Vec::new(),
                Controller::Pilot,
            )),
            // The side has to still exist after the kill or the match ends and
            // nothing after it happens (doc 16 §3.8).
            bystander((15, 15)),
        ],
        &curtain(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    assert!(
        hold_on(&arena, RED2, BLUE).is_none(),
        "the teammate saw through a wall, so the copy below is not a mesh copy"
    );

    // The sharer holds the tank and watched it die. The hold is planted and the
    // kill written where the kill line's own arm writes it — which is the
    // production path the test above drives end to end; what this one is about
    // is what the mesh does with it afterwards.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, at, None, &mut journal);
    learn_the_kill(&mut arena, RED, BLUE);

    // One pass of arena time: the sharer's own sensor pass finds that it holds
    // something the mesh may carry and schedules the share at that same
    // timestamp, and the teammate's row is created by `share_pass`'s insert arm.
    run_settled(&mut arena, at + 1_500, &mut bytes);
    let mate = hold_on(&arena, RED2, BLUE).expect("the mesh handed the track over");
    assert!(
        mate.own_seen.is_none(),
        "the teammate's own sensors took this, so it is not the arm under test"
    );
    assert!(
        !mate.known_dead,
        "a mesh copy opened already knowing a kill its platform never watched"
    );
    assert!(
        mate.seen_condition.is_none(),
        "the sharer had no reading to give, so nothing but the track travelled"
    );
    assert!(
        !arena.contact_reads_dead(&mate),
        "the teammate was handed a fact its own sensors never took"
    );
    assert!(
        hold_on(&arena, RED, BLUE).is_some_and(|held| held.known_dead),
        "the sharer stopped knowing its own kill"
    );
    // The mind's own reading of the copy, landed by the mark.
    assert_eq!(target_of(&arena, RED2), Some(BLUE));
    assert_eq!(machine_state_of(&arena, RED2), Some(MachineState::Approach));

    // And the passes after it take `share_pass`'s **update** arm on the very
    // same row, which is where a later tidy-up would most easily let the kill
    // across. It stays out of it, and the mind keeps closing on a tank that
    // is not there — the cost this cut's predicate states.
    run_settled(&mut arena, at + 2_500, &mut bytes);
    let mate = hold_on(&arena, RED2, BLUE).expect("the track is still inside its time to live");
    assert!(mate.own_seen.is_none(), "still a mesh copy");
    assert!(
        !mate.known_dead,
        "a share updated the copy and carried the kill with it"
    );
    assert_eq!(target_of(&arena, RED2), Some(BLUE));
    assert_eq!(machine_state_of(&arena, RED2), Some(MachineState::Approach));

    // It lets go when the track expires, and not one millisecond sooner: the
    // sharer's own sighting is what the copy's stamp is, and a wreck is never
    // sighted again (`sees_wrecks: false`).
    run_settled(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(hold_on(&arena, RED2, BLUE), None, "the track expired");
    assert_eq!(stock_memory_of(&arena, RED2).target, None);
    assert_eq!(machine_state_of(&arena, RED2), Some(MachineState::Patrol));
}

// ---------------------------------------------------------------------------
// 16b cut E.2 — the cross-tempo property (16b §11: mandatory)
// ---------------------------------------------------------------------------

/// **Everything this cut moved, in one scripted log**: a red machine kills the
/// blue tank with its own round while a second red machine holds a track it
/// cannot refresh, the whole under a `withdraw` order on red's squad that runs
/// out half way through.
///
/// What that exercises: the one new hashed field
/// (`ShadowContact::known_dead`, written by the kill line's own arm and folded
/// beside `numbered`), the two readers that decide what each machine does with
/// its hold after the kill, the rally the standing order holds back (16c
/// `B-40`), and the expiry that gives the squad back to doctrine. None of it may
/// depend on how often the arena is stepped (doc 16 §3.1's D-16-2).
///
/// The pair is **meshed**, so the share pass runs over a hold that carries a
/// kill for the whole of the replay: what a share does with `known_dead` is a
/// fact of arena time like everything else here, and the one place a later tidy
/// of `Arena::share_pass` could make it a fact of the pulse rate instead.
/// `t1_a_share_carries_the_track_and_never_the_kill` is the test of what the
/// share carries; this is the test that it carries it at every cadence.
///
/// **17b cut M4.4**: both reds are `Controller::Mind`s. The order is no
/// longer written on the rows at build (`ReflexMemory::set_order` has no
/// mind arm): it is the world's beat at pulse 0 ([`t1_withdraw_order_beat`]),
/// as `tests/minds.rs`' `lance_run` delivers one.
fn a_kill_under_a_withdraw_order() -> Arena {
    arena_of(
        vec![
            with_mesh(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
                SIGHT_CELLS,
            ),
            with_mesh(
                tank(
                    RED2,
                    Team::Red,
                    centre(2, 8),
                    vec![pistol()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
                SIGHT_CELLS,
            ),
            killable(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
            // Out of both scopes: the side has to still exist after the kill, or
            // the match ends and the seconds this replay is about never happen.
            bystander((15, 0)),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    )
}

/// **The squad's order, as the world delivers one at a beat** (17b cut
/// M4.4): one `IntentKind::Order` per unit of `units` on each unit's own
/// epoch for the order's channel, answered at pulse 0 and at no other — the
/// opening pulse, whose phase 2 runs at arena millisecond 0 at every
/// cadence, so the `ordered` lines and the deadline (`issued_ms + ttl_ms`,
/// `StandingOrder::accepted` at 0) are the same at 5,000, 1,000 and 500 ms.
/// A mind holds it re-keyed to its own rows (`Arena::rekey_order`) and the
/// arena ends it at the landing that reaches the deadline, with one
/// `order_expired` line per hull (mind.rs' `expire_mind_order`).
fn t1_order_beat(
    arena: &Arena,
    pulse: u32,
    units: &[ObjId],
    objective: tacsim::deliberation::Objective,
    ttl_ms: u32,
) -> Vec<tacsim::arena::intents::Intent> {
    if pulse != 0 {
        return Vec::new();
    }
    let lead = units.first().copied().expect("at least one hull to order");
    let squad = arena
        .squad_of(lead)
        .map(|roster| roster.id)
        .expect("the ordered hulls are a squad");
    let told = tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        // The originals' horizons: five seconds past the TTL (the withdraw
        // order's 30,000), never past the cap (the advance order's 600,000).
        horizon_ms: ttl_ms
            .saturating_add(5_000)
            .min(tacsim::deliberation::order::MAX_HORIZON_MS),
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms,
        issued_ms: 0,
        issuer: None,
        by: author(squad, None),
        spot: None,
    };
    units
        .iter()
        .zip(1_u64..)
        .map(|(unit, seq)| {
            let kind = tacsim::arena::intents::IntentKind::Order(Box::new(told.clone()));
            let channel = kind.channel_required();
            tacsim::arena::intents::Intent {
                unit: *unit,
                channel,
                epoch: arena.epochs.current(*unit, channel),
                kind,
                seq,
            }
        })
        .collect()
}

/// [`t1_order_beat`] for the kill replay: red's pair told to `withdraw` for
/// 25,000 ms, as the reflex rows were written at build.
fn t1_withdraw_order_beat(arena: &Arena, pulse: u32) -> Vec<tacsim::arena::intents::Intent> {
    t1_order_beat(
        arena,
        pulse,
        &[RED, RED2],
        tacsim::deliberation::Objective::Withdraw,
        25_000,
    )
}

/// **17b cut M4.4 — 16b cut E.2's cross-tempo property on the unit mind**
/// (16b §11: mandatory). Every claim is kept: the story really happens at
/// one cadence (`startup`, `detected`, `classified`, `fired`,
/// `unit_destroyed`, `order_expired`, `lost` all journalled), at least a
/// hundred substeps, and the event list, the hash sequence and the field's
/// end are one at 5,000, 1,000 and 500 ms. What moved: both reds are minds
/// ([`a_kill_under_a_withdraw_order`]), the `withdraw` order lands as the
/// world's beat at pulse 0 ([`t1_withdraw_order_beat`]) instead of on the
/// reflex rows at build, so the log opens with two `ordered` lines at
/// millisecond 0 (cadence-safe for the reason `apply_intent`'s own doc
/// gives); the arena ends a mind's order at the landing that reaches
/// 25,000 with one `order_expired` per hull (mind.rs' `expire_mind_order`),
/// and the stock mind's `ordered_home` holds the rally back under it as the
/// reflex's did; the finals read each mind's cached doctrine state and the
/// motor goal (`replay_with`), the pulses `step_settled`.
#[test]
fn t1_a_kill_under_an_expiring_withdraw_order_is_one_log_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_with(
                cadence,
                40_000,
                a_kill_under_a_withdraw_order(),
                &t1_withdraw_order_beat,
                &mut |_| {},
            )
        })
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    // The story really happened, at one cadence, before anything is compared.
    for want in [
        "startup",
        "detected",
        "classified",
        "fired",
        "unit_destroyed",
        "order_expired",
        "lost",
    ] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    assert!(first.hashes.len() >= 100, "{} substeps", first.hashes.len());

    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut Y.1 — a pilot's order, the hint on the issuer's scope (16b §11:
// mandatory)
// ---------------------------------------------------------------------------

/// The seated red tank whose pilot gives the order: in no squad, because a
/// pilot's tank takes none.
const Y1_PILOT: ObjId = ObjId(5);

/// **The cell in the corner pocket**, sealed by [`a_pocket`] and reached by no
/// route at all.
const POCKET: (u16, u16) = (13, 13);

/// The ring of wall that seals it.
fn a_pocket() -> Vec<(u16, u16, TerrainCell)> {
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 12..=14_u16 {
        paint.push((x, 12, wall()));
        paint.push((x, 14, wall()));
    }
    for y in 12..=14_u16 {
        paint.push((12, y, wall()));
        paint.push((14, y, wall()));
    }
    paint
}

/// **The other two things this cut moved in the arena, in one scripted log**: an
/// `advance` to a cell no route reaches, and a hot gun one shot short of its
/// ceiling.
///
/// The machine opens under a ten-minute `Objective::Advance` on [`POCKET`]. Its
/// first search for that cell fails, which under 16c `B-41` ends the order **for
/// it** at that tick — one `order_expired` line at one arena millisecond, and
/// doctrine from the tick after. Doctrine then classifies the tank in front of
/// it and shoots: at fifty of a ceiling of sixty with a cannon that makes
/// twenty, 16c `T-55` says the machine takes the shot the engine would take and
/// clamps on the ceiling, and holds its fire afterwards at the same number the
/// engine refuses at. Neither the tick the order ends at nor the shot may depend
/// on how often the arena is stepped (doc 16 §3.1's D-16-2; 16b §11 makes a
/// replay mandatory for what a cut adds to arena behaviour, and the kill above
/// covers only `B-45` and `B-40`).
///
/// **17b cut M4.4**: the machine is a `Controller::Mind` on [`t1_heated`]
/// (the same heat cell on a hull that keeps the core its seal recorded);
/// the order lands as the world's beat at pulse 0 ([`t1_advance_order_beat`])
/// instead of on the reflex row at build. **Two fixture constants re-derived
/// against the view**, as [`heat_field`] re-derives them for the same
/// (6, 8)/(9, 8) geometry (the `range_round` divergence: a contact's range
/// reaches a think rounded to 50 m and floored at one step): the sight is
/// [`SIGHT_CELLS`], not the rig's five — at five the 3,500 cm engage band is
/// under anything the view can say and the mind would never lock or fire, so
/// `T-55`'s one shot could not be taken; at ten the band is 7,000 cm and the
/// tank three cells off reads 50 m — and the tank presents every band
/// ([`presenting`]), so the round the mind decides on is a round the arena
/// fires. Neither is a claim of the replay; the heat cell, the pocket and the
/// order are unchanged.
fn an_unreachable_order_and_a_hot_gun() -> Arena {
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &a_pocket(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    t1_heated(&mut arena, RED, 60, 50, 20);
    arena
}

/// [`t1_order_beat`] for the pocket replay: the mind told to `advance` on
/// [`POCKET`] for ten minutes.
fn t1_advance_order_beat(arena: &Arena, pulse: u32) -> Vec<tacsim::arena::intents::Intent> {
    t1_order_beat(
        arena,
        pulse,
        &[RED],
        tacsim::deliberation::Objective::Advance { to: POCKET },
        600_000,
    )
}

/// One replay of [`an_unreachable_order_and_a_hot_gun`] at `cadence_ms`
/// under the stock recorder: the replay and every think it kept.
fn t1_pocket_replay(cadence_ms: i64) -> (Replay, Vec<Kept>) {
    let mut arena = an_unreachable_order_and_a_hot_gun();
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let replay = replay_with(
        cadence_ms,
        20_000,
        arena,
        &t1_advance_order_beat,
        &mut |_| {},
    );
    (replay, stock.kept())
}

/// **17b cut M4.4 — the unreachable order and the hot gun on the unit mind,
/// one log at three cadences** (16b cut E.2; 16c `B-41`, `T-55`; 16b §11).
/// Kept: the story happens at one cadence (`startup`, `detected`,
/// `classified`, `fired`); `T-55`'s shot — exactly one `fired` in twenty
/// seconds, the 50→60 shot the engine accepts and clamps, every later ready
/// held `Overheated` (weapons.rs refuses only at `overheated()`; the mind's
/// `would_overheat` holds its own fire at the same number); at least forty
/// substeps; and the event list, the hash sequence and the field's end one
/// at 5,000, 1,000 and 500 ms — joined by the kept think sequence, every
/// view and answer. What moved, and the one divergence (`order_end`, doc
/// 17 §17): a mind cannot end an order, so `B-41`'s `order_expired` line at
/// the tick whose search failed is reachable by no mind by design; the
/// arena journals `order_expired` only at `expires_ms`, which is **not**
/// inside this window (asserted: no such line), and T1's own edge stands in
/// its place — the first think after the motor's `drive_no_route` for the
/// pocket writes the ended order's deadline to `ignore_until_ms`
/// (`Some(600_000)`, stock_mind.rs' departure (i)), with no think before it
/// having done so, the hunt and the last plot clear, and that think's own
/// view still carrying the `advance` the arena holds; its landing is inside
/// the reflex's two seconds grown by one think period and a substep (the
/// order's goal lands at 1,732, the search fails there, the think at 2,482
/// reads the notice and lands at 2,732). The kernel half is stock_mind.rs'
/// `m1a6_a_drive_no_route_notice_performs_the_give_ups_and_the_fallback`;
/// this is the pocket end to end under `step_settled`.
#[test]
fn t1_an_unreachable_order_and_a_hot_gun_are_one_log_at_three_cadences() {
    let cadences = [5_000_i64, 1_000, 500];
    let runs: Vec<(Replay, Vec<Kept>)> = cadences.into_iter().map(t1_pocket_replay).collect();
    let (first, kept) = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "detected", "classified", "fired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    // The `order_end` divergence: the order was written for ten minutes and the
    // arena ends a mind's order only at its deadline, so no line is journalled
    // inside this window — the end is the mind's, in its memory.
    assert!(
        !kinds.iter().any(|kind| kind.contains("order_expired")),
        "a mind's early end journalled an expiry: {kinds:?}"
    );
    let gave_up = kept
        .iter()
        .position(|think| kept_memory(think).ignore_until_ms == Some(600_000))
        .expect("a think ended the order for this hull");
    let think = kept.get(gave_up).expect("the give-up think");
    assert!(
        kept.iter()
            .take(gave_up)
            .all(|think| kept_memory(think).ignore_until_ms.is_none()),
        "the order was ended before the search failed"
    );
    // The reflex's `< 2,000` grown by one think period and a substep (the
    // landing is one 250 ms substep after the think).
    assert!(
        think.1 + 250 < 3_250,
        "the give-up waited for the time to live: think at {} ms",
        think.1
    );
    assert!(
        think.2.order.as_ref().is_some_and(|order| {
            order.objective == tacsim::arena::perception::MindObjective::Advance { to: POCKET }
                && order.expires_ms == 600_000
        }),
        "the arena still holds the order the mind ended: {:?}",
        think.2.order
    );
    // It is the think that read the motor's `drive_no_route` for the pocket,
    // and what it answered is the fallback: the hunt and the last plot clear,
    // the walk's node written, the next goal that node's cell and handed to
    // the motor (`Goal(Cell)`), the machine in `Patrol` on doctrine.
    assert!(
        think.2.notices.iter().any(|notice| matches!(
            notice,
            tacsim::arena::perception::FrameNotice::Drive {
                outcome: tacsim::arena::perception::DriveOutcome::NoRoute
            }
        )),
        "the give-up was not the no-route notice's: {:?}",
        think.2.notices
    );
    let memory = kept_memory(think);
    assert_eq!(memory.state, MachineState::Patrol, "{memory:?}");
    assert_eq!(memory.hunt, None, "{memory:?}");
    assert_eq!(memory.last_plot, None, "{memory:?}");
    let fallback = memory.goal.expect("the fallback waypoint is the next goal");
    assert!(
        waypoint_cells().contains(&fallback) && memory.waypoint.is_some(),
        "the next goal is not a node of the graph: {memory:?}"
    );
    assert!(
        think.3.intents.iter().any(|intent| {
            *intent
                == tacsim::arena::intents::IntentKind::Goal(
                    tacsim::arena::intents::MotorGoal::Cell {
                        x: fallback.0,
                        y: fallback.1,
                    },
                )
        }),
        "the fallback was not handed to the motor: {:?}",
        think.3.intents
    );
    // `T-55`'s shot: the mind takes the one the engine would take, and the
    // ceiling it lands on is what keeps the rest of a twenty-second window
    // quiet.
    assert_eq!(
        first
            .events
            .iter()
            .filter(|(_, kind, _)| kind.contains("fired"))
            .count(),
        1,
        "the hot gun fired something other than the engine's one legal shot"
    );
    assert!(first.hashes.len() >= 40, "{} substeps", first.hashes.len());

    for ((run, run_kept), cadence) in runs.iter().zip(cadences).skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
        assert_eq!(
            run_kept, kept,
            "the think sequence diverged at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut E.3 — driving (16c `B-42`, `B-43`, `B-44`, `B-46`; §5's Q10)
// ---------------------------------------------------------------------------

/// A long east-west road, `len` cells by eight, with one patrol node at `node`.
fn road(len: u16, node: (u16, u16)) -> ArenaDefinition {
    let cells = vec![ground(); usize::from(len) * 8];
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex.road"),
        terrain: ArenaTerrain::new(len, 8, CELL_CM, cells).expect("the road window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 7,
            },
            EntryZone {
                team: Team::Blue,
                x0: len - 2,
                y0: 0,
                x1: len - 1,
                y1: 7,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::new(
            vec![Waypoint {
                id: SmolStr::new("goal"),
                x: node.0,
                y: node.1,
            }],
            &[],
        ),
        modes: Vec::new(),
    }
}

/// [`road`] with the **Red entry zone in the middle of it**, so that a machine
/// running home can drive through the zone and out the far side.
///
/// Every other window in this file puts Red's zone against the west wall, where
/// an overshoot is impossible by construction: the window's own edge stops the
/// tank inside the arrive window and there is nothing for the rally clock to
/// notice. The zone here is eight cells in from it.
fn road_home(len: u16, zone_x: u16, node: (u16, u16)) -> ArenaDefinition {
    ArenaDefinition {
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: zone_x,
                y0: 0,
                x1: zone_x + 1,
                y1: 7,
            },
            EntryZone {
                team: Team::Blue,
                x0: len - 2,
                y0: 0,
                x1: len - 1,
                y1: 7,
            },
        ],
        ..road(len, node)
    }
}

/// **17b cut M4.4 — `B-42`'s other half on the unit mind: a withdrawn
/// mind's rally clock is not restarted by an overshoot** (16b cut E.3).
///
/// Every claim of the reflex fixture is kept: the heavy breaks off, arrives
/// home, the quiet clock starts once and is never restarted, and the
/// west-most cell it reaches is 8 — the west edge of *home* for a goal at
/// the clamped corner (9, 4) and a one-cell arrive window. What moved: the
/// hull is a `Controller::Mind` and the clock is the memory's
/// `quiet_since_ms` (`StockMemory`, read after every settled pulse through
/// `stock_memory_of` — the one place the clock lives, which is why this is
/// a port and not a motor move), the state `Arena::machine_state`. T1's
/// `Withdraw` goal is `entry_cell` — the same clamped corner — and
/// `quiet_since_ms` is `get_or_insert`ed only on a think that reads the hull
/// arrived and holding nothing hostile, and cleared on one that does not
/// (stock_mind.rs' `Withdraw` arm), so a drive out of the zone and back
/// would read as a restart here exactly as it did on the reflex row; the
/// restart window is think-granular (a second, not half of one). The
/// fixture's reason for the road window and the 1,500 cm/s against
/// 200 cm/s² heavy stands as the reflex test wrote it: on the rig's own tank
/// and window an overshoot is impossible by construction, and a test built
/// on it would prove nothing. The heavy is re-sealed over its mobility.
#[test]
fn t1_a_withdrawn_minds_rally_clock_is_not_restarted_by_an_overshoot() {
    let heavy = CustodyUnit {
        mobility: TrackedRules {
            max_speed_cms: 1_500,
            accel_cms2: 100,
            brake_cms2: 200,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        ..tank(
            RED,
            Team::Red,
            centre(40, 4),
            vec![cannon()],
            Controller::Mind,
        )
    }
    .sealed();
    let mut arena = arena_on(
        road_home(48, 8, (44, 4)),
        vec![heavy],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 4_000, &mut bytes);
    hurt(&mut arena, RED);
    let mut wall = arena.now_ms;
    let mut started: Option<i64> = None;
    let mut restarts: Vec<(i64, i64)> = Vec::new();
    let mut withdrew = false;
    let mut west_most = u16::MAX;
    while arena.now_ms < 90_000 {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let quiet = stock_memory_of(&arena, RED).quiet_since_ms;
        match (started, quiet) {
            (None, Some(at)) => started = Some(at),
            (Some(was), Some(at)) if at != was => restarts.push((was, at)),
            _ => {}
        }
        if let Some((x, _)) = arena
            .unit(RED)
            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        {
            west_most = west_most.min(x);
        }
        let running = machine_state_of(&arena, RED) == Some(MachineState::Withdraw);
        withdrew |= running;
        // The rally is what ends the run home, and this test is about the clock
        // that leads to it: stop once the mind has come back.
        if withdrew && !running {
            break;
        }
        wall += 500;
    }
    println!("t1_e3_rally: west-most cell {west_most}, clock started at {started:?}");
    assert!(
        withdrew,
        "the mind never broke off, so nothing was measured"
    );
    assert!(
        started.is_some(),
        "the mind never arrived home, so the clock never started"
    );
    assert!(
        restarts.is_empty(),
        "the rally clock was restarted after the mind had arrived: {restarts:?}"
    );
    // And the reason it was not: the hull braked into its zone instead of
    // driving through it. The goal is the clamped corner `(9, 4)` and the
    // arrive window is a cell wide, so cell 8 is the west edge of *home*.
    assert!(
        west_most >= 8,
        "it drove out of its own entry zone to cell {west_most}, which is what restarts the clock"
    );
}

// ---------------------------------------------------------------------------
// 16b cut E.4 — the endgame (16c `B-04`, `T-42`)
// ---------------------------------------------------------------------------

/// **A seven-cell sight**, for the search's own fixtures.
///
/// [`SIGHT_CELLS`]' ten leave no corner of a sixteen-cell window a tank can be
/// lost in — every cell of row 8 is within ten of (15, 15) — and [`sight`]'s
/// five are too short to hold a target far enough away that the drive to its
/// plot is visible for more than a pulse. Seven is the one that gives both: a
/// fighting reach of 7,000 cm, `engage_cm` of 4,900, and two corners a pilot
/// tank can sit in and not be seen from anywhere on the search's route.
const E4_SIGHT_CELLS: u16 = 7;

/// The doctrine a cut E.4 fixture runs on: [`reflex_cfg`] with the two endgame
/// clocks named, so every test below says which of them it is about and turns
/// the other one off.
fn e4_cfg(search_ms: u32, hunt_after_ms: u32) -> ArenaConfig {
    arena_cfg(ReflexConfig {
        search_ms,
        hunt_after_ms,
        ..reflex_cfg()
    })
}

/// **A fixture on one tempo** (16b cut E.4).
///
/// What makes a pulse of these tests one decide tick is **not** this pin: a
/// pulse's `dt` is the wall interval (`Clock::advance`, clamped only by
/// `max_catch_up_steps` periods of the current tempo, which every tempo's
/// period clears), and [`run`], [`e4_run_until_state`] and
/// `e4_run_until_hunt` (retired by the T1 ports) all step the wall 500 ms at a time — which is
/// [`reflex_cfg`]'s own `decide_ms` — so one pulse is one decision at any
/// tempo. The pin only keeps the tempo's own transitions (and their `tempo`
/// ledger lines) out of the fixture, so the log a test reads is the machine's
/// and nothing else. It changes no outcome (doc 16 §3.1's D-16-2 is the whole
/// point); [`replay_of`] pins `Quiet` over it for its own reasons.
fn e4_pinned(mut arena: Arena) -> Arena {
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    arena
}

/// The same two units, so a test that needs its own [`ArenaDefinition`] fields
/// them through [`arena_on`].
fn e4_units(red: (u16, u16), blue: (u16, u16), weapons: Vec<WeaponSummary>) -> Vec<CustodyUnit> {
    vec![
        with_sight(
            tank(
                RED,
                Team::Red,
                centre(red.0, red.1),
                weapons,
                Controller::Reflex,
            ),
            E4_SIGHT_CELLS,
        ),
        tank(
            BLUE,
            Team::Blue,
            centre(blue.0, blue.1),
            Vec::new(),
            Controller::Pilot,
        ),
    ]
}

/// The fixture window with a named set of entry zones.
///
/// The hunt's whole input is the arena definition's zones (doc 16 §1: map
/// knowledge, and no unit's pose), so a test of it has to be able to say what
/// they are.
fn e4_definition(paint: &[(u16, u16, TerrainCell)], zones: Vec<EntryZone>) -> ArenaDefinition {
    ArenaDefinition {
        entry_zones: zones,
        ..definition(paint)
    }
}

/// Red's own zone, the two west columns — every fixture's, unchanged.
fn red_zone() -> EntryZone {
    EntryZone {
        team: Team::Red,
        x0: 0,
        y0: 0,
        x1: 1,
        y1: SIDE - 1,
    }
}

/// A blue zone of this shape, which is what a hunt answers out of.
fn blue_zone(x0: u16, x1: u16, y0: u16, y1: u16) -> EntryZone {
    EntryZone {
        team: Team::Blue,
        x0,
        y0,
        x1,
        y1,
    }
}

/// Where the machine is going.
fn goal_of(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena.motors.row(unit).and_then(|row| row.goal)
}

/// The far entry cell it is crossing to — **17b cut M4.4**: the mind's, off
/// its memory (`StockMemory::hunt`; read by the drone port alone).
fn hunt_of(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    stock_memory_of(arena, unit).hunt
}

/// The cell a unit stands in.
fn cell_of_unit(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena
        .unit(unit)
        .and_then(|state| arena.definition.terrain.cell_of(&state.pose))
}

/// Every cell the waypoint graph offers as a goal — what "it went back to its
/// patrol" means.
fn waypoint_cells() -> Vec<(u16, u16)> {
    vec![(1, 8), (14, 8)]
}

/// **Step 500 ms at a time until `unit`'s machine is in `state`**, answering
/// whether it got there inside `limit_ms` of arena time.
///
/// A claim about *the tick after an edge* cannot be made on the state a
/// ten-second run ends in: the machine searched, arrived and went back to its
/// walk while the run was still going, and the run's last pulse says only the
/// last of those. So the pulses are read as they go, exactly as
/// `tests/shipped_scale.rs` reads its own.
fn e4_run_until_state(
    arena: &mut Arena,
    unit: ObjId,
    state: MachineState,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> bool {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        if state_of(arena, unit) == Some(state) {
            return true;
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    false
}

// ---------------------------------------------------------------------------
// 17b cut M4.4 — the endgame on the unit mind (the e4 family's T1 rig)
// ---------------------------------------------------------------------------

/// **An eight-cell sight for the endgame's T1 fixtures** (17b cut M4.4), in
/// place of `E4_SIGHT_CELLS`' seven. The view publishes a contact's range
/// in whole metres rounded to `CONTACT_RANGE_STEP_M` and floored at one step
/// (`view::range_m_rounded`: 50 m), so the nearest range a mind can read is
/// 5,000 cm — over the seven-cell `engage_cm` of 4,900 (the divergence
/// `range_round`, stock_mind.rs' head) — and the `Engage` the search's
/// fixtures start from is unreachable on seven cells. Eight cells put
/// `engage_cm` at 5,600 and keep what seven bought: (15, 15) and (15, 0) are
/// more than eight cells from every cell of row 8 between x = 1 and x = 6
/// and from the search's own cell (6, 8) (9.2 and 12 cells), and (15, 15)
/// is ten and more from the hunt's crossing on rows 3 to 5. The clamped
/// corners the hunt answers are the map's, never the sight's. One fixture
/// did not survive the extra cell unchanged — the withdrawal's tank, which
/// [`t1_withdraw_forgets_the_search_and_a_stand_writes_one`] re-sites and
/// says why.
const T1_E4_SIGHT_CELLS: u16 = 8;

/// `Ranges::engage_cm` at [`T1_E4_SIGHT_CELLS`]: seven tenths of 8,000 cm.
const T1_E4_ENGAGE_CM: i64 = 5_600;

/// **Where the search's three fixtures start the mind** (17b cut M4.4; the
/// row's one re-derived fixture constant). 16b cut M.2 put the reflex machine
/// at (1, 8) so that its 500 ms tick halted it in `Engage` at (3, 8), three
/// cells short of the plot at (6, 8) — the margin the `> ARRIVE_CELLS` guards
/// below are written with. A mind reads the classified contact at its next
/// think (up to a second later) and lands the stop a substep on, so where
/// it halts for the same start had to be measured, not assumed: the halt is
/// printed by the search fixture, and it is (3, 8) — the reflex's own cell,
/// the classified contact reaching the think at 3,482 and the stop landing
/// at 3,732 where the reflex's tick stopped it at 3,482. The start cell
/// stands, and the guards stay live by the margin they were written with.
const T1_SEARCH_START: (u16, u16) = (1, 8);

/// `e4_units` on the unit mind: the red hull a `Controller::Mind` seeing
/// [`T1_E4_SIGHT_CELLS`] (`seeing` re-seals it), the blue one the parked
/// pilot tank it was.
fn t1_e4_units(red: (u16, u16), blue: (u16, u16), weapons: Vec<WeaponSummary>) -> Vec<CustodyUnit> {
    vec![
        seeing(
            tank(
                RED,
                Team::Red,
                centre(red.0, red.1),
                weapons,
                Controller::Mind,
            ),
            T1_E4_SIGHT_CELLS,
        ),
        tank(
            BLUE,
            Team::Blue,
            centre(blue.0, blue.1),
            Vec::new(),
            Controller::Pilot,
        ),
    ]
}

/// `e4_pinned`, with the stock recorder over the native mind the field
/// flies (`Stock::on`), so a test reads every think's view and answer.
fn t1_e4_on(arena: Arena) -> (Arena, std::sync::Arc<Stock>) {
    let mut arena = e4_pinned(arena);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    (arena, stock)
}

/// `e4_field` on the unit mind, recorded.
fn t1_e4_field(
    red: (u16, u16),
    blue: (u16, u16),
    paint: &[(u16, u16, TerrainCell)],
    cfg: &ArenaConfig,
) -> (Arena, std::sync::Arc<Stock>) {
    t1_e4_on(arena_of(
        t1_e4_units(red, blue, vec![cannon()]),
        paint,
        cfg,
        &[BLUE],
    ))
}

/// **Step one substep (250 ms) at a time, settled, to `until_ms`** — for a
/// read inside one think period and a substep of a plant: a think at `x482`
/// lands at `x732`, which a 500 ms pulse opened at `x500` reads only at
/// `x + 1,500`.
fn run_substeps(arena: &mut Arena, until_ms: i64, bytes: &mut Vec<u8>) {
    let mut wall = arena.now_ms;
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        if out.ended.is_some() {
            break;
        }
        wall += 250;
    }
}

/// **The next think, landed** (17b cut M4.4): one substep at a time until the
/// recorder keeps a new think and its landing (one substep on) has passed,
/// answering the think — what a plant is read against, where the reflex
/// fixtures read the next 500 ms tick.
fn t1_next_landing(arena: &mut Arena, stock: &Stock, bytes: &mut Vec<u8>) -> Kept {
    let before = stock.kept().len();
    let from = arena.now_ms;
    let mut wall = arena.now_ms;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        wall += 250;
        if let Some(think) = stock.kept().get(before)
            && arena.now_ms >= think.1 + arena.substep_len_ms()
        {
            return think.clone();
        }
        assert!(
            arena.now_ms - from <= 2_000 && out.ended.is_none(),
            "no think landed inside two seconds of {from} ms"
        );
    }
}

/// **What one think stood in and chose**: the pulse the landed hunt was read
/// on, the think's OWN `view.cell`, the cell it chose and the think's
/// millisecond.
type Hunted = (i64, (u16, u16), (u16, u16), i64);

/// **Step 500 ms at a time until a think sets a hunt and it lands**
/// (`e4_run_until_hunt` on the mind). The cell answered is the setting
/// think's own `view.cell`, not the hull's at the pulse's end: the hull
/// drives on between the think and the landing's pulse, and the clamped
/// corner is on the row the think read.
fn t1_run_until_hunt(
    arena: &mut Arena,
    stock: &Stock,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> Option<Hunted> {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        if let Some(cell) = stock_memory_of(arena, RED).hunt {
            let think = stock
                .kept()
                .into_iter()
                .find(|think| kept_memory(think).hunt == Some(cell))
                .expect("the think that set the hunt was kept");
            return think.2.cell.map(|here| (arena.now_ms, here, cell, think.1));
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    None
}

/// **Step 500 ms at a time until a landed think read no order** (17b cut
/// M4.4; `e4_run_until_unordered` on the mind): the first kept think after
/// the ones kept so far whose `view.order` is `None`, once its landing has
/// passed. The arena ends a mind's order at the landing that reaches its
/// `expires_ms` (`order_expired`), so this is the think after that landing.
fn t1_run_until_unordered(
    arena: &mut Arena,
    stock: &Stock,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> Option<Kept> {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    let from = stock.kept().len();
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        if let Some(think) = stock
            .kept()
            .into_iter()
            .skip(from)
            .find(|think| think.2.order.is_none())
            && arena.now_ms >= think.1 + arena.substep_len_ms()
        {
            return Some(think);
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    None
}

/// **A squad's order delivered through the arena's step** (17b cut M4.4).
/// `e4_order` wrote the reflex row (`ReflexMemory::set_order`, which has no
/// mind arm); a mind's order is an `IntentKind::Order` through
/// `apply_intent`'s Mind arm — re-keyed to the hull's own rows at the write
/// (D-UM-15) and ended by the arena at the landing that reaches its
/// `expires_ms`, with `order_expired` journalled — as `tests/minds.rs`'
/// `unarmed_run` delivers one. A zero-length settled pulse at `now_ms` lands
/// it at this arena millisecond (phase 2), the way a world beat lands in the
/// pulse that reaches it.
fn t1_order(
    arena: &mut Arena,
    unit: ObjId,
    objective: tacsim::deliberation::Objective,
    ttl_ms: u32,
    bytes: &mut Vec<u8>,
) {
    let squad = arena
        .squad_of(unit)
        .map(|roster| roster.id)
        .expect("one mind is a squad of one");
    let told = tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: ttl_ms,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms,
        issued_ms: arena.now_ms,
        issuer: None,
        by: author(squad, None),
        spot: None,
    };
    let kind = tacsim::arena::intents::IntentKind::Order(Box::new(told));
    let channel = kind.channel_required();
    let intent = tacsim::arena::intents::Intent {
        unit,
        channel,
        epoch: arena.epochs.current(unit, channel),
        kind,
        seq: 1,
    };
    let now = arena.now_ms;
    let out = arena.step_settled(now, &[], &[intent]);
    absorb(&out, bytes);
}

// ---------------------------------------------------------------------------
// Fix A — the search
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — a mind that loses its contact drives to where it last
/// saw it** (16b cut E.4; 16c `B-04`), and not to its waypoint.
///
/// The three moments of the reflex fixture's arc are kept and asserted at
/// the pulse they happen: the goal after the `lost` edge is the memory's
/// `last_plot` cell and not a node of the graph; the target moved again
/// *after* its contact expired does not move the goal (the observer rule
/// from the mind's side — `search_goal` runs off the memory and the view's
/// own cell, never a reading of where anything now is); on arrival
/// `last_plot` is cleared and the next goal is a waypoint, read at the pulse
/// the clear happens with the mind's cell and the plot's age, so an arrival
/// cannot be mistaken for an age-out. What moved: the hull is a
/// `Controller::Mind` on [`t1_e4_field`]; the state is read per settled
/// pulse off `Arena::machine_state` (`t1_run_until_machine_state`), the
/// plot off the memory (`stock_memory_of`), the goal off the motor row as
/// before. Two fixture constants are re-derived and say why at their
/// definitions: the sight ([`T1_E4_SIGHT_CELLS`]: the view's 50 m floor
/// against a seven-cell `engage_cm`) and the start cell
/// ([`T1_SEARCH_START`]: the mind's think-and-landing halt margin). The
/// fourth moment of the arc — a hostile found on the way — is
/// [`t1_a_hostile_found_while_searching_pre_empts_it`]. `ARRIVE_CELLS` is
/// `arena::motor`'s.
#[test]
fn t1_a_lost_contact_is_searched_for_where_it_was_last_seen() {
    // Twenty seconds of search, no hunt: this test is about the first of the two
    // clocks and nothing else.
    let (mut arena, _stock) = t1_e4_field(T1_SEARCH_START, (6, 8), &[], &e4_cfg(20_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "the plot reads 5,000 cm on the view, inside the {T1_E4_ENGAGE_CM} cm band, so the mind \
         holds it"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).last_plot.map(|(cell, _)| cell),
        Some((6, 8)),
        "and an `Engage` think writes the plot it is holding"
    );
    println!(
        "t1_e4_search: halted in `Engage` at {:?}",
        cell_of_unit(&arena, RED)
    );

    // Over the hill: (15, 15) is more than eight cells from anywhere on row 8
    // between x = 1 and x = 6, so nothing refreshes the contact and the
    // time-to-live takes it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the mind back to `Patrol`"
    );
    assert_eq!(arena.contacts.of(RED).count(), 0, "the contact expired");
    assert_eq!(
        goal_of(&arena, RED),
        Some((6, 8)),
        "the goal is the cell it last saw the tank in, not a waypoint"
    );
    assert!(
        !waypoint_cells().contains(&(6, 8)),
        "the fixture would say nothing if the plot's cell were also a node"
    );

    // The observer rule: the tank moves again, and the goal does not follow it.
    // (15, 0) is also out of sight from every cell of the route.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 0);
    }
    let until = arena.now_ms + 1_000;
    run_settled(&mut arena, until, &mut bytes);
    let here = cell_of_unit(&arena, RED).expect("the mind is on the window");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::motor::ARRIVE_CELLS,
        "it has already arrived at the plot, so the claim below is about nothing: {here:?}"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some((6, 8)),
        "the search is the mind's own last plot, never a reading of where the target now is"
    );

    // It arrives, finds nothing, and goes back to its patrol. Read at the pulse
    // the search is given up, so what is asserted is *why* it was given up.
    let stamped = stock_memory_of(&arena, RED)
        .last_plot
        .map(|(_, at)| at)
        .expect("the search is live");
    let until = arena.now_ms + 25_000;
    let mut wall = arena.now_ms;
    let mut cleared: Option<(i64, (u16, u16))> = None;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if stock_memory_of(&arena, RED).last_plot.is_none() {
            cleared = cell_of_unit(&arena, RED).map(|here| (arena.now_ms, here));
            break;
        }
        wall += 500;
    }
    let (at_ms, here) = cleared.expect("the search never ended");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) <= tacsim::arena::motor::ARRIVE_CELLS,
        "the search was given up at {here:?}, which is not an arrival at (6, 8)"
    );
    assert!(
        at_ms - stamped < 20_000,
        "and it was given up {} ms after the plot was stamped, which is an age-out and not an \
         arrival",
        at_ms - stamped
    );
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "after the search the waypoint walk resumes: {goal:?}"
    );
}

/// **17b cut M4.4 — a hostile found on the way to the last plot pre-empts
/// the search, on the unit mind** (16b cut E.4; the brief's search item).
///
/// The reflex fixture's claims are kept: while the search is live
/// (`last_plot` set, the hull short of it) a classified contact planted in
/// the store turns the mind on it — `Approach` or `Engage` — with the new
/// target and the new plot, not the search it gave up. The hostile edges run
/// above the state's own body and score the view's classified rows, which
/// the planted store row feeds. What moved: a `Controller::Mind` on
/// [`t1_e4_field`]; the read after the plant is one think and a substep on
/// (≤ 1,250 ms, stepped a substep at a time by [`run_substeps`]) where the
/// reflex read the next 500 ms tick; the target is the memory's `ContactId`
/// — the row's own key, `draw_contact_id(SEED, RED, BLUE)` — and the plot the
/// memory's. The second tank is handed to the store classified (this file's
/// head says why a test may) so the pre-emption lands at the next think
/// rather than after `classify_after_ms`, inside which a mind four cells
/// from its plot would have arrived. No kernel mirror: stock_mind.rs' own
/// tests are kernel tests.
#[test]
fn t1_a_hostile_found_while_searching_pre_empts_it() {
    let (mut arena, _stock) = t1_e4_field(T1_SEARCH_START, (6, 8), &[], &e4_cfg(20_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the mind back to `Patrol`"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some((6, 8)),
        "the search has the goal"
    );
    let here = cell_of_unit(&arena, RED).expect("the mind is on the window");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::motor::ARRIVE_CELLS,
        "it has already arrived at the plot, so the search is not live: {here:?}"
    );

    // Over the ridge to the south of its route, four cells off: in sight, and
    // not on the plot's cell, so the new target cannot be mistaken for the old
    // search.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(4, 12);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run_substeps(&mut arena, now + 1_250, &mut bytes);
    assert!(
        matches!(
            machine_state_of(&arena, RED),
            Some(MachineState::Approach | MachineState::Engage)
        ),
        "a hostile found on the way outranks the search: {:?}",
        machine_state_of(&arena, RED)
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        memory.target,
        Some(draw_contact_id(SEED, RED, BLUE)),
        "and the mind has turned on it: {memory:?}"
    );
    assert_eq!(
        memory.last_plot.map(|(cell, _)| cell),
        Some((4, 12)),
        "and the plot it now remembers is the new one, not the search it gave up"
    );
}

/// **17b cut M4.4 — a plot older than `search_ms`, and `search_ms = 0`, are
/// both the old `lost` edge, on the unit mind** (16b cut E.4) — the pre-E.4
/// edge reproduced two ways, each read one think at a time.
///
/// The claim is the reflex fixture's: the search is a clock and nothing
/// else, and a mind whose clock has run out (or was never wound) drops the
/// plot short of it — never at an arrival — and walks the graph. What each
/// case counts is how many `Patrol` thinks drove at the plot, and where the
/// hull stood when the plot was dropped. What moved, the table's constants:
/// the floor row is `search_ms = STOCK_THINK_MS` (1,000), the mind's own
/// period, where the reflex's floor was its 500 ms decide tick — the plot is
/// stamped by the last think that held the target and the `lost` edge is
/// read one think later, so the first `Patrol` think finds it exactly
/// `search_ms` old; the inclusive compare drives at it (the one think the
/// floor promises) and the next drops it. `drove` is counted per THINK off
/// the recorder's answers (a `Patrol` think whose memory goal is the plot),
/// not per pulse: the two pulses of one think period would read the one
/// drive twice. The edge (`search_goal`'s inclusive compare and its `== 0`
/// drop) is unchanged. No mirror.
#[test]
fn t1_an_aged_plot_and_a_search_switched_off_are_both_the_old_lost_edge() {
    for (search_ms, driven, why) in [
        // The validated floor, one think period (`STOCK_THINK_MS`).
        (1_000_u32, 1_usize, "a plot aged past `search_ms`"),
        // Off: the first `Patrol` think drops it unread.
        (0, 0, "`search_ms` switched off"),
    ] {
        let (mut arena, stock) = t1_e4_field(T1_SEARCH_START, (6, 8), &[], &e4_cfg(search_ms, 0));
        let mut bytes: Vec<u8> = Vec::new();
        run_settled(&mut arena, 6_000, &mut bytes);
        assert_eq!(
            machine_state_of(&arena, RED),
            Some(MachineState::Engage),
            "{why}"
        );
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose = centre(15, 15);
        }
        let from = stock.kept().len();
        let until = arena.now_ms + 5_000;
        let mut wall = arena.now_ms;
        let mut dropped_at: Option<(u16, u16)> = None;
        while arena.now_ms < until {
            let out = arena.step_settled(wall, &[], &[]);
            absorb(&out, &mut bytes);
            if dropped_at.is_none()
                && machine_state_of(&arena, RED) == Some(MachineState::Patrol)
                && stock_memory_of(&arena, RED).last_plot.is_none()
            {
                dropped_at = cell_of_unit(&arena, RED);
            }
            wall += 500;
        }
        assert_eq!(
            machine_state_of(&arena, RED),
            Some(MachineState::Patrol),
            "{why}"
        );
        let drove = stock
            .kept()
            .iter()
            .skip(from)
            .filter(|think| {
                let memory = kept_memory(think);
                memory.state == MachineState::Patrol && memory.goal == Some((6, 8))
            })
            .count();
        assert_eq!(
            drove, driven,
            "{why}: the mind drove at the plot for {drove} thinks"
        );
        let here = dropped_at.unwrap_or_else(|| panic!("{why}: the plot was never dropped"));
        assert!(
            here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::motor::ARRIVE_CELLS,
            "{why}: the plot was dropped at {here:?}, which is an arrival and not an age-out"
        );
        let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
        assert!(
            waypoint_cells().contains(&goal),
            "{why}: the goal should be a waypoint, and it is {goal:?}"
        );
    }
}

/// **17b cut M4.4 — a last plot no route reaches costs exactly one failed
/// search, on the unit mind** (16b cut E.4; 16c `B-41`'s lesson applied to
/// the search).
///
/// The reflex fixture's claims are kept: the blue tank sits in
/// [`a_pocket`]'s sealed corner, the mind is handed a classified contact on
/// it and closes (`Approach`, the plot [`POCKET`] remembered, no failed
/// search for the stand-off cell, which is ordinary ground); the contact
/// expires, the search takes the goal, fails exactly once over twenty
/// seconds, the plot is forgotten and never re-asserted, and the walk has
/// the goal. What moved: the mind reads the motor's `drive_no_route` notice
/// at its next think and clears the plot there (stock_mind.rs' departure
/// (i): `last_plot` and the hunt given up on the notice, then the fallback
/// waypoint) where the reflex's `follow` cleared it inside the tick; the
/// state and the plot are the memory's, the goal at the end the memory's
/// too (the motor row forgets a node it reached, the memory keeps it); the
/// failed searches are summed off the step counters. The memory half is
/// in-source (stock_mind.rs' no-route notice test); the field count is this
/// port's.
#[test]
fn t1_a_last_plot_behind_a_wall_costs_one_failed_search_and_is_not_re_asserted() {
    let (mut arena, _stock) = t1_e4_field((6, 8), POCKET, &a_pocket(), &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 1_000;
    let closing = run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Approach),
        "the pocket reads 10,000 cm off on the view, outside the {T1_E4_ENGAGE_CM} cm band"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).last_plot.map(|(cell, _)| cell),
        Some(POCKET),
        "an `Approach` think writes the plot it is closing on"
    );
    assert_eq!(
        closing.path_failures, 0,
        "the stand-off cell is ordinary ground and the route to it is found"
    );

    // Now let the contact go. The search takes the goal, fails once, and never
    // asks again.
    let until = arena.now_ms + 20_000;
    let searching = run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        searching.path_failures, 1,
        "a sealed last plot should cost exactly one failed search, and it cost {}",
        searching.path_failures
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        memory.last_plot, None,
        "the `unreachable` edge gives the search up, or the `Patrol` arm re-asserts it every think"
    );
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the waypoint walk has it: {goal:?}"
    );
}

// ---------------------------------------------------------------------------
// Fix B — the hunt
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — a mind that has seen nobody crosses to the other side's
/// entry zone** (16b cut E.4; 16c `B-04`), and comes back to its patrol when
/// it finds nothing there.
///
/// The reflex fixture's claims are kept: the clock is read at its due and
/// not merely by the end of a run (a clock seeded at check-out, or one that
/// fired early, would pass a test that only waited for the crossing); the
/// cell is the clamped corner of the OTHER side's zone on the mind's own
/// row — (14, r), deliberately not a node — and `Patrol`'s goal while it is
/// set; on arrival the hunt is cleared, the clock restarted at the arrival
/// think, and the walk resumes at once from the node nearest the far edge,
/// (14, 8). What moved: a `Controller::Mind` on [`t1_e4_field`] with the
/// stock recorder; `unheld_since_ms` and `hunt` are the memory's, read after
/// every settled pulse; the set window becomes the first think at or after
/// `started + hunt_after_ms`, landed a substep later (the pulse the landing
/// fell in is the pulse the hunt reads on the row); the cell the mind stood
/// in when it chose is the setting think's own `view.cell` (the hull drives
/// on between the think and the landing's pulse); the restart is pinned to
/// the clearing think's own millisecond, exactly. `hunt_goal` is the
/// reflex's clock and cell (`far_entry_cell` with no `taken` fan at the
/// mind); the arrival half is in-source (stock_mind.rs' arrived-notice
/// test), the clock-at-due and the field crossing were not.
#[test]
fn t1_a_mind_that_has_seen_nobody_crosses_to_look() {
    // Three seconds of nothing held, no search: this test is about the second
    // clock alone. The blue tank sits in the far corner, out of the eight-cell
    // sight from every cell of the crossing, so what ends the hunt is arrival
    // and not a contact.
    let (mut arena, stock) = t1_e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    let substep = arena.substep_len_ms();
    let mut wall = arena.now_ms;
    let mut started: Option<i64> = None;
    let mut chosen: Option<Hunted> = None;
    while arena.now_ms < 20_000 {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if started.is_none() {
            started = stock_memory_of(&arena, RED).unheld_since_ms;
        }
        if let Some(cell) = stock_memory_of(&arena, RED).hunt {
            let think = stock
                .kept()
                .into_iter()
                .find(|think| kept_memory(think).hunt == Some(cell))
                .expect("the think that set the hunt was kept");
            chosen = think.2.cell.map(|here| (arena.now_ms, here, cell, think.1));
            break;
        }
        wall += 500;
    }
    let (set_ms, here, cell, think_ms) =
        chosen.expect("a mind that has seen nobody for three seconds should cross");
    let started = started.expect("the clock started at the first `Patrol` think");
    let due = started + 3_000;
    assert!(
        think_ms >= due && think_ms - tacsim::arena::mind::STOCK_THINK_MS < due,
        "the clock started at {started} ms, so the hunt is due at {due} ms, and the think that \
         set it was at {think_ms} ms"
    );
    assert!(
        set_ms - 500 < think_ms + substep && think_ms + substep <= set_ms,
        "the think at {think_ms} ms lands a substep on, in the pulse ending {set_ms} ms"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    assert_eq!(
        cell,
        (SIDE - 2, here.1),
        "the clamped corner of the OTHER side's zone, on the mind's own row: it was standing in \
         {here:?}"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some(cell),
        "and while it is set it is `Patrol`'s goal"
    );
    assert!(
        !waypoint_cells().contains(&cell),
        "the fixture would say nothing if the hunt cell were also a node: {cell:?}"
    );

    // It crosses, finds nothing, and the walk resumes from the node nearest the
    // far edge. **Read at the arrival and not at the end of a long run**:
    // `hunt_after_ms` is three seconds here, so a mind left to itself for
    // twenty of them has arrived, walked on and set out again, and the last
    // pulse would say only the last of that. Successive hunts are the design
    // (the walk carries the hull to a different row each time, so the
    // crossings sweep the zone); what this asserts is the clear that ends
    // one of them.
    let mut arrived: Option<i64> = None;
    let until = arena.now_ms + 25_000;
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if stock_memory_of(&arena, RED).hunt.is_none() {
            arrived = Some(arena.now_ms);
            break;
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    let at_ms = arrived.expect("the mind should have crossed and arrived");
    let here = cell_of_unit(&arena, RED).expect("the mind is on the window");
    assert!(
        here.0.abs_diff(cell.0).max(here.1.abs_diff(cell.1)) <= tacsim::arena::motor::ARRIVE_CELLS,
        "the hunt was given up at {here:?}, which is not an arrival at {cell:?}"
    );
    let since = stock_memory_of(&arena, RED)
        .unheld_since_ms
        .expect("the clock is restarted at the arrival think");
    let cleared = stock
        .kept()
        .into_iter()
        .find(|think| think.1 > think_ms && kept_memory(think).hunt.is_none())
        .expect("the think that read the arrival was kept");
    assert_eq!(
        since, cleared.1,
        "the clock should be restarted at the think that read the arrival ({} ms) and it reads \
         {since} ms",
        cleared.1
    );
    assert!(
        at_ms - 500 < cleared.1 + substep && cleared.1 + substep <= at_ms,
        "the arrival think at {} ms landed in the pulse ending {at_ms} ms",
        cleared.1
    );
    // The walk resumes at once, and from the node **nearest the far edge**,
    // (14, 8) — not the one it was walking to when the clock ran out. It set
    // out from (6, 3), whose nearest node is (1, 8) on its own side: a mind
    // that kept that node would turn round at the far edge and drive the whole
    // window back, and the next hunt would start from where it began.
    assert_eq!(
        goal_of(&arena, RED),
        Some((14, 8)),
        "after the crossing the walk resumes from the node nearest the far edge"
    );
    assert_eq!(
        cell_of_unit(&arena, RED)
            .and_then(|here| arena.definition.waypoints.nearest(here.0, here.1))
            .and_then(|node| arena.definition.waypoints.node(node))
            .map(|node| (node.x, node.y)),
        Some((14, 8)),
        "the fixture would say nothing if (14, 8) were not the node nearest the arrival"
    );
}

/// **17b cut M4.4 — the hunt reads the map and no unit's pose, on the unit
/// mind** (16b cut E.4; doc 16 §1, the observer rule from the mind's side).
///
/// The reflex fixture's claim is kept: two fields differing in one thing
/// only — whether the enemy stands in the zone the mind crosses to — and the
/// mind holds no contact on it either way (both cells are well outside its
/// sight), and the cell it picks is the same, because `far_entry_cell` reads
/// the arena definition's entry zones and nothing else. What moved: a
/// `Controller::Mind` under the stock recorder; the hunt is read off the
/// first kept think whose memory `hunt` is `Some`, *here* is that think's
/// own `view.cell`, and the view it chose on held no contact at all
/// (`view.contacts` empty) — the whole input of the think is the view, so
/// the emptiness is asserted on the view and not only on the store. The
/// kernel's observer half is in tests/motor.rs
/// (`m1a1_the_nav_pickers_read_the_ground_and_the_own_pose_only`); no T1
/// fixture reached a hunt at all.
#[test]
fn t1_the_hunt_reads_the_arena_definition_and_never_a_unit() {
    let crossing = |blue: (u16, u16)| {
        let (mut arena, stock) = t1_e4_field((6, 3), blue, &[], &e4_cfg(0, 3_000));
        let mut bytes: Vec<u8> = Vec::new();
        let (_, here, cell, think_ms) =
            t1_run_until_hunt(&mut arena, &stock, 20_000, &mut bytes).expect("the mind crosses");
        assert_eq!(
            arena.contacts.of(RED).count(),
            0,
            "the mind must hold nothing when it chooses, or the fields differ in two things"
        );
        let think = stock
            .kept()
            .into_iter()
            .find(|think| think.1 == think_ms)
            .expect("the setting think was kept");
        assert!(
            think.2.contacts.is_empty(),
            "the view the hunt was chosen on held a contact: {:?}",
            think.2.contacts
        );
        (here, cell)
    };
    // (15, 0) is inside Blue's own entry zone; (6, 15) is nowhere near it. Both
    // are more than eight cells from every cell the mind stands in.
    let standing_there = crossing((15, 0));
    let standing_elsewhere = crossing((6, 15));
    assert_eq!(
        standing_there, standing_elsewhere,
        "the hunt followed the enemy's pose instead of the arena's own entry zones"
    );
}

/// **17b cut M4.4 — what the hunt's clock reads is `holds_a_hostile`, on the
/// unit mind** (16b cut E.4; 16c `B-04`, on 16b cut E.2's reading of
/// *hostile*).
///
/// Three fields, each read every pulse, as the reflex fixture read them: (a)
/// an unarmed mind that sees the whole window (sixteen cells) and holds a
/// classified hostile never crosses, and once the classification lands its
/// clock is held at `None` — the one shape in which a hull both holds a
/// hostile and reaches the hunt's own arm, because an unarmed hull has no
/// fight limit and so no target (`best_hostile_ranked`, D-UM-48); (c) the
/// key at nought never hunts. What moved in (b): the view's `reads_dead` is
/// built only for a hull with a weapons station (step.rs' `frame_scope`,
/// D-UM-48), so the dead hold cannot be read on the unarmed hull — (b) runs
/// on the ARMED mind with the contact planted classified and marked
/// `known_dead` at nought, before the first think: `best_hostile_ranked`
/// skips a dead-reading row and `holds_a_hostile` is false, so the hull
/// stays in `Patrol` and the clock runs, and the crossing is asserted with
/// the hold still reading dead and the tank still in plain sight. Marked at
/// nought and not after five seconds, because an armed mind seeing a live
/// classified tank three cells off would be in `Engage` by then; the same
/// edge, not a weakened pin. `hunt`, `unheld_since_ms` and the goal are the
/// memory's.
#[test]
fn t1_a_hostile_held_stops_the_clock_and_a_dead_one_does_not() {
    let scoped = |cfg: &ArenaConfig, blue: (u16, u16), weapons: Vec<WeaponSummary>| {
        t1_e4_on(arena_of(
            vec![
                seeing(
                    tank(RED, Team::Red, centre(6, 3), weapons, Controller::Mind),
                    SIDE,
                ),
                tank(
                    BLUE,
                    Team::Blue,
                    centre(blue.0, blue.1),
                    Vec::new(),
                    Controller::Pilot,
                ),
            ],
            &[],
            cfg,
            &[BLUE],
        ))
    };
    // Every settled pulse of `run_ms`, with `check` asked of each.
    let watch = |arena: &mut Arena, run_ms: i64, check: &dyn Fn(&Arena)| {
        let mut bytes: Vec<u8> = Vec::new();
        let until = arena.now_ms + run_ms;
        let mut wall = arena.now_ms;
        while arena.now_ms < until {
            let out = arena.step_settled(wall, &[], &[]);
            absorb(&out, &mut bytes);
            check(arena);
            wall += 500;
        }
    };

    // (a) A hostile in sight, held and classified. Six seconds of clock, which
    // is longer than the three `classify_after_ms` takes after the first pass
    // detects it, so the classification lands before the clock could run out
    // and the claim is about the hold and not about a race.
    let (mut arena, _stock) = scoped(&e4_cfg(0, 6_000), (9, 3), Vec::new());
    watch(&mut arena, 20_000, &|arena| {
        assert_eq!(
            stock_memory_of(arena, RED).hunt,
            None,
            "a mind that can see the enemy does not go looking for it (at {} ms)",
            arena.now_ms
        );
    });
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Patrol),
        "an unarmed mind has no target and patrols whatever it sees"
    );
    assert!(
        arena.contacts.of(RED).any(|(_, held)| matches!(
            held.class,
            Classification::Classified { team } if team == Team::Blue
        )),
        "and it really is holding the tank: the fixture says nothing otherwise"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).unheld_since_ms,
        None,
        "and its clock is not merely ignored, it is held at `None`"
    );

    // (b) The same hold, marked the way the kill line marks it, on the armed
    // hull whose view carries `reads_dead`. It is not a hostile, so the clock
    // runs and the mind crosses — with the tank still in its sight, so it is
    // the mark and not a lost contact that let it go.
    let (mut arena, stock) = scoped(&e4_cfg(0, 6_000), (9, 3), vec![cannon()]);
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    learn_the_kill(&mut arena, RED, BLUE);
    let mut bytes: Vec<u8> = Vec::new();
    let (_, here, cell, _) = t1_run_until_hunt(&mut arena, &stock, 20_000, &mut bytes)
        .expect("a contact that reads dead is not a hostile, so the clock runs");
    assert!(
        arena
            .contacts
            .of(RED)
            .any(|(_, held)| held.target == BLUE && arena.contact_reads_dead(held)),
        "the hold it crossed under is the one it knows is dead, still held"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Patrol),
        "a dead-reading row is never a target"
    );
    assert_eq!(
        cell,
        (SIDE - 2, here.1),
        "and the crossing is the far zone's clamped corner on its own row"
    );

    // (c) The key at nought: never, on any pulse.
    let (mut arena, _stock) = scoped(&e4_cfg(0, 0), (15, 15), Vec::new());
    watch(&mut arena, 30_000, &|arena| {
        assert_eq!(
            stock_memory_of(arena, RED).hunt,
            None,
            "`hunt_after_ms = 0` never hunts (at {} ms)",
            arena.now_ms
        );
    });
    let goal = stock_memory_of(&arena, RED)
        .goal
        .expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and it is on the waypoint walk: {goal:?}"
    );
}

/// **17b cut M4.4 — what `far_entry_cell` answers, and what it refuses to
/// answer, on the unit mind** (16b cut E.4; 16c `B-04`).
///
/// The reflex fixture's three windows and claims are kept: (a) the clamped
/// corner impassable — the answer is the nearest passable cell of that zone
/// on the mind's own row, (15, r), and never the wall; (b) nothing in the
/// zone passable — `None`, no failed search paid for it, the walk kept; (c)
/// no other side's zone at all — the same. What moved: a `Controller::Mind`
/// on [`t1_e4_units`] under the stock recorder; the hunt is the kept think's
/// memory and *here* that think's own `view.cell`
/// ([`t1_run_until_hunt`]); the goal is the memory's; `path_failures` is
/// summed off the step counters, as before. `hunt_goal`'s `None` arm writes
/// `unheld_since_ms = Some(t_ms)` and walks (stock_mind.rs); the kernel half
/// of (a) is in-source (motor.rs' `ae1_the_goal_pickers_read_the_units_own_row`).
#[test]
fn t1_the_hunt_never_answers_a_cell_the_ground_refused() {
    // (a) The whole of the zone's near column is wall, so the clamped corner is
    // impassable whatever row the mind has walked to by the time the clock
    // runs out. The scan then reads the mind's own row first, finds (14, r)
    // refused and (15, r) open at a Manhattan distance nothing on the rows
    // either side can beat, and answers that. `entry_cell`'s own
    // `.or(Some(clamped))` would have answered the wall.
    // The wall stops at row 12, so the far column is **reachable** round the
    // bottom of it. A column walled end to end would seal the zone off, and the
    // `unreachable` edge would take the hunt back at the next think: the
    // fixture would then be the island case below wearing this one's name.
    let walled_column: Vec<(u16, u16, TerrainCell)> =
        (0..=12_u16).map(|y| (SIDE - 2, y, wall())).collect();
    let walled_corner = e4_definition(
        &walled_column,
        vec![red_zone(), blue_zone(SIDE - 2, SIDE - 1, 0, SIDE - 1)],
    );
    let (mut arena, stock) = t1_e4_on(arena_on(
        walled_corner,
        t1_e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let (_, here, cell, _) = t1_run_until_hunt(&mut arena, &stock, 20_000, &mut bytes)
        .expect("the zone has passable cells, so the hunt has a goal");
    assert!(
        here.1 <= 12,
        "the mind walked past the wall's end, so its clamped corner is passable and the fixture \
         asks nothing: {here:?}"
    );
    assert_ne!(
        cell,
        (SIDE - 2, here.1),
        "the hunt answered the impassable corner the passability test had just refused"
    );
    assert!(
        arena
            .definition
            .terrain
            .get(cell.0, cell.1)
            .is_some_and(TerrainCell::passable),
        "and whatever it answered is ground a tank can stand on: {cell:?}"
    );
    assert_eq!(
        cell,
        (SIDE - 1, here.1),
        "the nearest passable cell of the mind's own row, {here:?} being where it stood"
    );

    // (b) The whole zone is wall. `None`, and the walk goes on.
    let sealed: Vec<(u16, u16, TerrainCell)> = (0..=5_u16).map(|y| (11_u16, y, wall())).collect();
    let no_way_in = e4_definition(&sealed, vec![red_zone(), blue_zone(11, 11, 0, 5)]);
    let (mut arena, _stock) = t1_e4_on(arena_on(
        no_way_in,
        t1_e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let counted = run_settled(&mut arena, 15_000, &mut bytes);
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        memory.hunt, None,
        "a zone with no passable cell is honestly nothing, not a wall to drive at"
    );
    // Both nodes are reachable on this window, so a failed search can only
    // be a hunt at a cell the ground refused — which the `unreachable` edge
    // would clear at the next think, leaving the two assertions above green.
    // The count is what tells `None` from *the wall, given up*.
    assert_eq!(
        counted.path_failures, 0,
        "the hunt answered a refused cell and paid a failed search for it"
    );
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the mind keeps its waypoint walk: {goal:?}"
    );

    // (c) No other side has a zone. Same answer, for a different reason.
    let one_zone = e4_definition(&[], vec![red_zone()]);
    let (mut arena, _stock) = t1_e4_on(arena_on(
        one_zone,
        t1_e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let counted = run_settled(&mut arena, 15_000, &mut bytes);
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.hunt, None);
    assert_eq!(counted.path_failures, 0, "and no search was paid for it");
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert!(waypoint_cells().contains(&goal), "{goal:?}");
}

/// **17b cut M4.4 — a hunt cell no route reaches costs one failed search per
/// `hunt_after_ms` and no more, on the unit mind** (16b cut E.4) — the
/// island the adversarial review named.
///
/// The reflex fixture's claim and ceiling are kept: the other side's zone is
/// the one cell of [`POCKET`], which [`a_pocket`] seals — passable ground, so
/// `far_entry_cell` answers it honestly, and no route at all — and over
/// forty seconds the failed searches number at most `elapsed /
/// hunt_after_ms + 1` and at least one. What moved: a `Controller::Mind` on
/// [`t1_e4_units`]; under `Fallback::Notice` the failed search drops the
/// goal (no re-search on the next tick, whatever `repath_ms` says) and the
/// mind's next think performs the hunt's give-up off the `drive_no_route`
/// notice and restarts the clock (stock_mind.rs' departure (i)) — one
/// failure per `hunt_after_ms` plus at most one think, inside the ceiling.
/// The counters are the step's, as before. The unit-level give-up is
/// in-source; the rate bound is this port's.
#[test]
fn t1_an_island_hunt_cell_costs_one_failed_search_per_clock() {
    let hunt_after_ms = 5_000_i64;
    let island = e4_definition(
        &a_pocket(),
        vec![
            red_zone(),
            blue_zone(POCKET.0, POCKET.0, POCKET.1, POCKET.1),
        ],
    );
    let (mut arena, _stock) = t1_e4_on(arena_on(
        island,
        t1_e4_units((6, 8), (15, 0), vec![cannon()]),
        &e4_cfg(0, 5_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    // Past the first crossing, so the count below is of steady-state failures.
    run_settled(&mut arena, 6_000, &mut bytes);
    let from = arena.now_ms;
    let until = from + 40_000;
    let counted = run_settled(&mut arena, until, &mut bytes);
    let elapsed = arena.now_ms - from;
    let ceiling = u32::try_from(elapsed / hunt_after_ms + 1).unwrap_or(u32::MAX);
    println!(
        "t1_e4_island_hunt: {} failed searches in {elapsed} ms of arena time, ceiling {ceiling}",
        counted.path_failures
    );
    assert!(
        counted.path_failures > 0,
        "the hunt never fired, so nothing was measured"
    );
    assert!(
        counted.path_failures <= ceiling,
        "an island hunt cell cost {} failed searches in {elapsed} ms, which is more than one per \
         `hunt_after_ms`: the `unreachable` edge is not giving the hunt up",
        counted.path_failures
    );
}

/// Write an `Advance { to }` for `unit`'s squad onto its machine, live for
/// `ttl_ms` from now.
fn e4_advance(arena: &mut Arena, unit: ObjId, to: (u16, u16), ttl_ms: u32) {
    e4_order(
        arena,
        unit,
        tacsim::deliberation::Objective::Advance { to },
        ttl_ms,
    );
}

/// Write `objective` for `unit`'s squad onto its machine, live for `ttl_ms`
/// from now.
fn e4_order(
    arena: &mut Arena,
    unit: ObjId,
    objective: tacsim::deliberation::Objective,
    ttl_ms: u32,
) {
    let squad = arena
        .squad_of(unit)
        .map(|roster| roster.id)
        .expect("one reflex machine is a squad of one");
    let told = tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: ttl_ms,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms,
        issued_ms: arena.now_ms,
        issuer: None,
        by: author(squad, None),
        spot: None,
    };
    arena.reflex.set_order(
        unit,
        tacsim::deliberation::StandingOrder::accepted(told, arena.now_ms),
    );
}

/// **17b cut M4.4 — a movement order outranks both clocks, and neither
/// clock's goal survives it, on the unit mind** (16b cut E.4, on 16b cut
/// N.2's order steering).
///
/// The reflex fixture's two arcs and claims are kept, each run through a
/// short order to the think after it ends: (a) the order lands with a live
/// `last_plot` (well inside the minute of `search_ms`) — the first ordered
/// think gives the search up, the hunt is never set under the order, the
/// goal is the commander's cell, and at the end the walk resumes with a
/// node that is not the plot; (b) the order lands with the mind already
/// crossing — the first ordered think gives the hunt up, the goal is the
/// commander's cell, and at the end the walk resumes with a node that is
/// not the crossing; in both the hunt's wait counts from the order's end.
/// What moved: the order is an `IntentKind::Order(Advance { to })` through
/// `arena.step` ([`t1_order`]; re-keyed by D-UM-15, cleared by the arena at
/// the landing that reaches its deadline with `order_expired`); the first
/// ordered think is read as the next landed think
/// ([`t1_next_landing`], ≤ 1,250 ms) and "unordered" as the first kept
/// think whose `view.order` is `None` ([`t1_run_until_unordered`]);
/// `last_plot`, `hunt` and `unheld_since_ms` are those thinks' memory; the
/// commander's cell is read off the memory's goal (the mind's belief — the
/// motor row forgets a goal it reached, and ten seconds is enough for six
/// cells); the clock tolerance is one think period — the `Patrol` arm under
/// a movement objective writes `unheld_since_ms = Some(t_ms)` on every
/// ordered think, so at the first unordered think it reads the last ordered
/// think's millisecond, exactly one period back. The arm clears `last_plot`
/// and `hunt` and sets `goal = to` (stock_mind.rs); (b)'s hunt in flight is
/// what [`t1_a_mind_that_has_seen_nobody_crosses_to_look`] shows T1 reaches.
#[test]
fn t1_a_movement_order_outranks_the_search_and_the_hunt() {
    let period = tacsim::arena::mind::STOCK_THINK_MS;
    // (a) The search.
    let (mut arena, stock) = t1_e4_field((2, 8), (6, 8), &[], &e4_cfg(60_000, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    let (plot, _) = stock_memory_of(&arena, RED)
        .last_plot
        .expect("the mind has something to search for");

    // The tank goes over the hill and a commander names a cell in the same
    // breath. The order steers: `Advance` drops the target, the mind goes to
    // `Patrol`, and the goal is the commander's cell — not the plot, and not the
    // far entry zone once the hunt's three seconds have gone by.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    t1_order(
        &mut arena,
        RED,
        tacsim::deliberation::Objective::Advance { to: (2, 2) },
        20_000,
        &mut bytes,
    );
    let ordered = t1_next_landing(&mut arena, &stock, &mut bytes);
    assert!(ordered.2.order.is_some(), "the order reached the view");
    let memory = kept_memory(&ordered);
    assert_eq!(memory.state, MachineState::Patrol, "{memory:?}");
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    assert_eq!(
        memory.last_plot, None,
        "the order's first think gives the search up"
    );
    let until = arena.now_ms + 10_000;
    run_settled(&mut arena, until, &mut bytes);
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(
        memory.hunt, None,
        "the hunt is never set under a movement order"
    );
    let goal = memory
        .goal
        .expect("an ordered mind has the commander's cell");
    assert!(
        goal == (2, 2) || (goal.0.abs_diff(2).max(goal.1.abs_diff(2)) <= 1),
        "the goal should be the ordered cell and it is {goal:?}"
    );
    let unordered =
        t1_run_until_unordered(&mut arena, &stock, 20_000, &mut bytes).expect("the order runs out");
    let memory = kept_memory(&unordered);
    assert_eq!(
        memory.last_plot, None,
        "the search does not come back when the order runs out"
    );
    assert_eq!(memory.hunt, None);
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert_ne!(
        goal, plot,
        "the mind turned round for a plot it saw before the order"
    );
    assert!(
        waypoint_cells().contains(&goal),
        "at expiry the walk resumes: {goal:?}"
    );
    let since = memory.unheld_since_ms.expect("the hunt's clock is running");
    assert!(
        since >= unordered.1 - period,
        "the hunt's wait should count from the order's end (the think before {} ms), and it \
         reads {since} ms",
        unordered.1
    );
    assert!(
        count(&events(&mut arena, &mut bytes), "order_expired") > 0,
        "the arena ended the order at its deadline"
    );

    // (b) The hunt: a mind already crossing when the order lands.
    let (mut arena, stock) = t1_e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    let (_, _, crossing, _) =
        t1_run_until_hunt(&mut arena, &stock, 20_000, &mut bytes).expect("the mind crosses");
    t1_order(
        &mut arena,
        RED,
        tacsim::deliberation::Objective::Advance { to: (2, 2) },
        5_000,
        &mut bytes,
    );
    let ordered = t1_next_landing(&mut arena, &stock, &mut bytes);
    assert!(ordered.2.order.is_some(), "the order reached the view");
    let memory = kept_memory(&ordered);
    assert_eq!(
        memory.hunt, None,
        "the order's first think gives the hunt up"
    );
    assert_eq!(memory.goal, Some((2, 2)));
    assert_eq!(
        goal_of(&arena, RED),
        Some((2, 2)),
        "and the motor row was handed it"
    );
    let unordered =
        t1_run_until_unordered(&mut arena, &stock, 10_000, &mut bytes).expect("the order runs out");
    let memory = kept_memory(&unordered);
    assert_eq!(
        memory.hunt, None,
        "the crossing does not resume when the order runs out"
    );
    let goal = memory.goal.expect("a patrol has a node to walk to");
    assert_ne!(
        goal, crossing,
        "the mind went back to the crossing it was ordered off"
    );
    assert!(
        waypoint_cells().contains(&goal),
        "at expiry the walk resumes: {goal:?}"
    );
    let since = memory.unheld_since_ms.expect("the hunt's clock is running");
    assert!(
        since >= unordered.1 - period,
        "the hunt's wait should count from the order's end (the think before {} ms), and it \
         reads {since} ms",
        unordered.1
    );
}

/// **17b cut M4.4 — `Withdraw` forgets the search, and a stand writes one,
/// on the unit mind** (16b cut E.4).
///
/// The reflex fixture's three arms are kept: (a) the health edge clears
/// `last_plot` on entry and the rallied walk is the graph's, not the plot
/// it was hurt at; (b) a commander's `Withdraw` clears it on entry; (c) a
/// `Withdraw` that stands and fights writes it from the plot it turned
/// onto, with no target — the one writer that cannot read the target,
/// which `Withdraw` clears. What moved: a `Controller::Mind` on
/// [`t1_e4_field`]; the state is `Arena::machine_state`
/// (`t1_run_until_machine_state`), the plot and the target the memory's;
/// the order is an `IntentKind::Order(Withdraw)` through `arena.step`
/// ([`t1_order`]); the rally think emits `Goal(Hold)` and the NEXT think
/// walks the graph, so (a)'s "first `Patrol` tick" is the think after the
/// rally, one think period on; (c) is the Q.7 family's
/// [`t1_q7_withdraw_and_rally`], the Mind copy of `withdraw_and_rally`'s
/// shape (fifteen cells, so the running stand's ceiling meets the 50 m the
/// view reads for a tank one cell off — why, at its definition). One
/// fixture constant re-derived in (a) and (b): the tank
/// stands at (11, 8), five cells off, not (10, 8), four — the view reads
/// 50 m either way (inside the band, outside the running stand's
/// 2,666 cm), and what the reflex's four bought was a tank out of its
/// seven-cell sight from home; the mind halts at (2, 8) (the arrive
/// window, one cell short of the corner), which is exactly eight cells
/// from (10, 8), inside [`T1_E4_SIGHT_CELLS`] — measured: the contact
/// refreshed every pass, a held hostile, `quiet_since_ms` pinned at `None`
/// with the home stand open, no rally in sixty seconds. Nine cells restore
/// the reflex's fact. T1 clears `last_plot` on both `Withdraw` entries and writes
/// it from `withdraw_stand`'s row with the target `None` (stock_mind.rs);
/// stock_mind.rs' healthy-withdraw test covers a different edge.
#[test]
fn t1_withdraw_forgets_the_search_and_a_stand_writes_one() {
    // (a) The health edge. Five cells apart (the reflex's four; see the doc
    // above), 5,000 cm on the view, inside the 5,600 cm band: the mind holds
    // it and remembers the plot.
    let (mut arena, _stock) = t1_e4_field((6, 8), (11, 8), &[], &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(
        stock_memory_of(&arena, RED).last_plot.map(|(cell, _)| cell),
        Some((11, 8))
    );
    // 166 per mille: under `stand_min_permille`, so nothing on the way home
    // writes a plot back.
    hurt(&mut arena, RED);
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Withdraw, 2_000, &mut bytes),
        "a mind hurt this far breaks off"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).last_plot,
        None,
        "the health edge's first think gives the search up"
    );
    // Home — (2, 8), the arrive window's one cell short of the corner — out of
    // sight of the tank (nine cells off), and quiet: it rallies, and the walk
    // it rallies to is the graph's and not the plot it was hurt at.
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Patrol, 60_000, &mut bytes),
        "the mind should have rallied"
    );
    // The rally is decided inside `Withdraw`'s own arm and lets the hull hold;
    // the first `Patrol` think is the one after it, a think period on.
    let until = arena.now_ms + tacsim::arena::mind::STOCK_THINK_MS;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(stock_memory_of(&arena, RED).last_plot, None);
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "a rallied mind walks the graph, and it is driving at {goal:?}"
    );

    // (b) A commander's `Withdraw`, on a healthy mind: the tank reads 5,000 cm
    // off on the view, outside the running stand's 2,666 cm, so the first
    // think in `Withdraw` writes nothing back either.
    let (mut arena, _stock) = t1_e4_field((6, 8), (11, 8), &[], &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 6_000, &mut bytes);
    assert!(stock_memory_of(&arena, RED).last_plot.is_some());
    t1_order(
        &mut arena,
        RED,
        tacsim::deliberation::Objective::Withdraw,
        30_000,
        &mut bytes,
    );
    assert!(
        t1_run_until_machine_state(&mut arena, RED, MachineState::Withdraw, 2_000, &mut bytes),
        "the order puts the mind in `Withdraw`"
    );
    assert_eq!(
        stock_memory_of(&arena, RED).last_plot,
        None,
        "the order's first think gives the search up"
    );

    // (c) The stand. `t1_q7_withdraw_and_rally`'s mind breaks off at 277 per
    // mille beside a classified tank one cell away, and stands on its first
    // `Withdraw` think. It never held a target before it broke off (the health
    // edge takes it straight out of `Patrol`), so a plot it remembers can only
    // be the stand's.
    let (mut arena, _stock) = t1_e4_on(t1_q7_withdraw_and_rally());
    let mut bytes: Vec<u8> = Vec::new();
    let until = arena.now_ms + 5_000;
    let mut wall = arena.now_ms;
    let mut stood: Option<((u16, u16), Option<tacsim::arena::state::ContactId>)> = None;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let memory = stock_memory_of(&arena, RED);
        if machine_state_of(&arena, RED) == Some(MachineState::Withdraw)
            && let Some((cell, _)) = memory.last_plot
        {
            stood = Some((cell, memory.target));
            break;
        }
        wall += 500;
    }
    let (cell, target) = stood.expect("a mind that stood and fought wrote no plot");
    assert_eq!(
        cell,
        (9, 8),
        "the plot it turned onto is the tank's own cell"
    );
    assert_eq!(
        target, None,
        "and `Withdraw` holds no target, so the plot came from the stand"
    );
}

/// **17b cut M4.4 — a fight gives the hunt up, and the wait starts again
/// from it, on the unit mind** (16b cut E.4) — `hunt` belongs to `Patrol`
/// and to nothing else.
///
/// The reflex fixture's claims are kept: a classified contact planted three
/// cells ahead of a crossing mind turns it (`Approach` or `Engage`), the
/// crossing is given up, and the clock is restarted, not stopped. What
/// moved: a `Controller::Mind` on [`t1_e4_field`] under the stock recorder;
/// after the first kept think with `hunt` set the plant is read against the
/// next landed think ([`t1_next_landing`]), and its memory says state,
/// `hunt` and `unheld_since_ms` — the last pinned EXACTLY to that think's
/// own millisecond, tighter than the reflex's pulse window: `best_hostile`
/// moves `Patrol` to `Approach` and the end-of-think rule restarts the
/// clock at the think's own `t_ms` whenever the state is not `Patrol`
/// (stock_mind.rs).
#[test]
fn t1_a_fight_gives_the_hunt_up() {
    let (mut arena, stock) = t1_e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    let (_, here, _, _) =
        t1_run_until_hunt(&mut arena, &stock, 20_000, &mut bytes).expect("the mind crosses");
    // Something turns up three cells ahead of it on the crossing, classified
    // (this file's head says why a test may hand it over).
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(here.0 + 3, here.1);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let think = t1_next_landing(&mut arena, &stock, &mut bytes);
    let memory = kept_memory(&think);
    assert!(
        matches!(memory.state, MachineState::Approach | MachineState::Engage),
        "the mind turned on what it found: {memory:?}"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(memory.state));
    assert_eq!(memory.hunt, None, "leaving `Patrol` gives the crossing up");
    assert_eq!(
        memory.unheld_since_ms,
        Some(think.1),
        "the wait restarts at the think that left `Patrol` ({} ms), and it reads {:?}",
        think.1,
        memory.unheld_since_ms
    );
}

// ---------------------------------------------------------------------------
// The cross-tempo property (16b §11)
// ---------------------------------------------------------------------------

/// The whole arc in one fixture: contact, lost, search, arrival, quiet, hunt,
/// arrival in the far zone, contact — on the unit mind, recorded (17b cut
/// M4.4: `a_search_and_a_hunt`'s field on [`t1_e4_field`]).
///
/// The clocks are short enough that all of it fits inside 45 s of arena time,
/// and the ground is the plain fixture window, so nothing in the replay depends
/// on a search finding a way round anything.
fn t1_a_search_and_a_hunt() -> (Arena, std::sync::Arc<Stock>) {
    t1_e4_field((2, 8), (6, 8), &[], &e4_cfg(20_000, 10_000))
}

/// The one thing the script does: the blue tank pulls out to the far corner of
/// its own entry zone, where the hunt will eventually find it.
fn blue_pulls_into_its_zone(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 14);
    }
}

/// What the kept thinks saw of the middle of the arc.
#[derive(Debug, Default)]
struct Arc {
    /// A `Patrol` think whose goal was the live `last_plot`'s cell.
    searched: bool,
    /// A think on which `hunt` was set.
    hunted: bool,
    /// A think, once the hunt had been set, on which the mind stood within
    /// `ARRIVE_CELLS` of the far zone's near column.
    crossed: bool,
}

/// **The arc off the recorder** (17b cut M4.4): the kept thinks of one
/// replay read for the search's goal and the hunt's — each think's answered
/// memory (`kept_memory`) and the view's own cell.
fn t1_the_arc(kept: &[Kept]) -> Arc {
    let mut arc = Arc::default();
    for think in kept {
        let memory = kept_memory(think);
        arc.searched |= memory.state == MachineState::Patrol
            && memory
                .last_plot
                .is_some_and(|(cell, _)| memory.goal == Some(cell));
        arc.hunted |= memory.hunt.is_some();
        arc.crossed |= arc.hunted && think.2.cell.is_some_and(|here| here.0 >= SIDE - 3);
    }
    arc
}

/// **17b cut M4.4 — the search and the hunt at three cadences are one event
/// list and one hash sequence, on the unit mind** (16b §11; doc 16 §3.1's
/// D-16-2).
///
/// The reflex fixture's claims are kept: the arc really happened (a `lost`
/// and a `detected` after it in the log), the replay is more than one
/// pulse, the three cadences are three different pulse counts, the middle
/// of the arc — the search's goal, the hunt set, the far zone reached — is
/// read off the finest record, and the events, the hashes and the finals
/// are one match at 5,000, 1,000 and 500 ms. What moved: a `Controller::Mind`
/// on [`t1_a_search_and_a_hunt`] under the stock recorder, stepped by
/// `replay_with` (M4.1's `step_settled`); the middle of the arc is read off
/// the KEPT THINKS of each replay ([`t1_the_arc`]) instead of a fourth
/// pulse-by-pulse run at 500 ms, and the kept thinks themselves — every
/// view and every answer, at their lattice milliseconds — are asserted
/// equal across the three cadences beside the events and the hashes: both
/// clocks are read at an arena millisecond off a view built at an arena
/// millisecond, inside a think that lands at the same arena millisecond at
/// every tempo. Not mirrored: minds.rs' two three-cadence fixtures end in
/// `Engage` and `Withdraw` with `hunt` null, and no T1 fixture asserted a
/// hunt at all. The `finals`' state column is the cached doctrine state.
#[test]
fn t1_the_search_and_the_hunt_are_one_log_at_three_cadences() {
    let runs: Vec<(Replay, Vec<Kept>)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            let (arena, stock) = t1_a_search_and_a_hunt();
            let replay = replay_with(
                cadence,
                45_000,
                arena,
                &|_, _| Vec::new(),
                &mut |arena: &mut Arena| {
                    if arena.now_ms == SCRIPT_AT_MS {
                        blue_pulls_into_its_zone(arena);
                    }
                },
            );
            (replay, stock.kept())
        })
        .collect();
    let (first, thought) = runs.first().expect("three replays");
    // The arc really did happen: a mind that never lost a contact, or never
    // crossed, would compare equal three ways and prove nothing. The log says
    // the two ends of it — a `lost`, and a `detected` after it.
    let kinds: Vec<(i64, &str)> = first
        .events
        .iter()
        .filter_map(|(t_ms, kind, _)| kind.split('"').nth(3).map(|kind| (*t_ms, kind)))
        .collect();
    println!("t1_e4_search_and_hunt: {} events, {kinds:?}", kinds.len());
    let lost_at = kinds
        .iter()
        .find(|(_, kind)| *kind == "lost")
        .map(|(t_ms, _)| *t_ms)
        .unwrap_or_else(|| {
            panic!("the contact never expired, so there was nothing to search for: {kinds:?}")
        });
    assert!(
        kinds
            .iter()
            .any(|(t_ms, kind)| *kind == "detected" && *t_ms > lost_at),
        "nothing was found again after the `lost` at {lost_at} ms, so the arc never closed: \
         {kinds:?}"
    );
    assert!(
        first.hashes.len() > 3,
        "a replay of one pulse is not a cross-tempo property"
    );
    // The guard: three cadences really are three different numbers of pulses.
    let counts: Vec<u32> = runs.iter().map(|(run, _)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair[0] != pair[1]),
        "the three replays ran the same number of pulses: {counts:?}"
    );
    // And the middle of the arc, which the log does not name: the kept
    // thinks, read for the search's goal and the hunt's. A replay whose hunt
    // never fired would still agree with itself three ways.
    let arc = t1_the_arc(thought);
    println!("t1_e4_search_and_hunt: {arc:?}, {} thinks", thought.len());
    assert!(
        arc.searched,
        "the mind never drove at its last plot: {arc:?}"
    );
    assert!(
        arc.hunted,
        "the hunt never fired inside the replay: {arc:?}"
    );
    assert!(
        arc.crossed,
        "the mind never reached the far zone on its hunt: {arc:?}"
    );
    for (at, (run, kept)) in runs.iter().enumerate().skip(1) {
        assert_eq!(
            run.events, first.events,
            "cadence {at}: the event list differs"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "cadence {at}: the per-substep hash sequence differs"
        );
        assert_eq!(run.finals, first.finals, "cadence {at}: the field differs");
        assert_eq!(
            kept, thought,
            "cadence {at}: the thinks differ (their views or their answers)"
        );
    }
}

// ---------------------------------------------------------------------------
// `T-42` — three properties the code claimed and no test held
// ---------------------------------------------------------------------------

/// **The curtain of `curtain`, with the machine's own row left open.**
///
/// [`curtain`] walls every row of column 9, which seals the window in two: a
/// machine west of it could never drive at a plot east of it, and the far
/// waypoint would be a node no route reaches. Row 0 is left open, which is a
/// way round and **not** a way through: the line from row 8 to row 8 still
/// crosses (9, 8), so the wall stops the sight and nothing else, which is what
/// the edge is about.
fn curtain_with_a_gap() -> Vec<(u16, u16, TerrainCell)> {
    curtain().into_iter().filter(|(_, y, _)| *y != 0).collect()
}

/// **The `no line` edge** (doc 16 §6; 16c `T-42`): a round of the machine's own
/// stopped by the ground re-plans it.
///
/// No test in the tree reached [`Arena::note_no_line`] before this one. The
/// machine holds the band on a pilot tank; the pilot drives behind a wall and
/// the contact lives another three seconds, so the machine still believes in
/// the plot and fires at it. The round is accepted, traced, and stopped: the
/// black box records a miss whose reason is `blocked`, `damage`'s re-validation
/// calls `note_no_line`, and the next tick throws the goal away, falls out of
/// `Engage` into `Approach` and — 16b cut E.1's rule — drives at the plot's own
/// cell.
///
/// **Held on the reflex — STOP J** (17b cut M4.4's row 76, a STOP there with
/// no letter; lettered at 17b cut M4.5a by the orchestrator's Q14, beside
/// STOP I): **the `line` divergence**. The stock mind's `Approach` arm tests
/// the line to the PLOT's cell (`line_between(here, row.cell)`), where the
/// reflex traced it to the target's live cell. M4.4's probe on a mind (the
/// same field, an eight-cell sight, a presenting target so a slot fires)
/// had the slot's rounds resolve `Miss { blocked }` behind the wall, the next
/// think read the notices and set `no_line`, went `Approach` — and, the line
/// from its cell to the plot's cell being clear, went straight back to
/// `Engage` in the same think: no goal, no intent; the `(8, 8)` goal that
/// came later was the search's, after the contact expired. The edge needs the
/// plot's cell behind the wall, and a mind never fires at a plot whose cell
/// has no line. Like STOP I (a mind is taught `no_line` by any blocked miss
/// of a held target), it is "`no line` on a mind", for 17b cut M4.6 to hold
/// or decide; this test is untouched until then.
#[test]
fn e4_t42_a_round_stopped_by_the_ground_re_plans_the_machine() {
    // The curtain is a wall down column 9, painted from the start (the terrain
    // is immutable for the life of a match). Both tanks open **west** of it, so
    // the line is clear and the band is held; the pilot then drives east through
    // the one gap the fixture leaves and the wall is between them.
    // A five-hundred-millisecond breech, because the contact lives only three
    // sensor passes after the tank goes behind the wall and the stock cannon's
    // four-second cycle would spend all of them.
    let quick = WeaponSummary {
        cycle_ms: 500,
        ..cannon()
    };
    let mut arena = e4_pinned(arena_on(
        definition(&curtain_with_a_gap()),
        e4_units((6, 8), (8, 8), vec![quick]),
        &e4_cfg(60_000, 0),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(MachineState::Engage),
        "it holds the band on a tank it can see"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");

    // Behind the wall. The contact is not refreshed but it lives another three
    // passes, and the machine keeps firing at where it last saw it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(11, 8);
    }
    assert!(
        e4_run_until_state(&mut arena, RED, MachineState::Approach, 3_000, &mut bytes),
        "the `no line` edge should have taken the machine out of `Engage` within three seconds"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(
        count(&log, "fired") > before,
        "the machine never fired at the plot, so no round was ever stopped"
    );
    let blocked = log
        .iter()
        .filter(|event| {
            serde_json::to_value(&event.kind)
                .ok()
                .and_then(|value| {
                    value
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some(tacsim::arena::damage::BLOCKED)
        })
        .count();
    assert!(
        blocked > 0,
        "no round resolved as a `blocked` miss, so the `no line` edge was never fed"
    );
    assert!(
        arena.reflex.unit(RED).is_some_and(|row| !row.no_line),
        "and the tick that re-planned on it cleared the flag"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some((8, 8)),
        "and 16b cut E.1's rule puts the goal on the plot's own cell"
    );
}

/// `dry_pair`'s shape for the three-cadence `Spent` scripts (17b cut M4.4; the
/// one-cadence twin keeps its own [`t1_spent_pair`]): the red hull a
/// `Controller::Mind` and the blue one `blue`'s, both seeing ten cells and
/// presenting every band (`seeing`, `presenting` — 17b cut M4.3's `j_field`
/// figures, for its reasons: on the rig's five-cell sight the mind's
/// `engage_cm` is 3,500 cm, under the 5,000 cm floor of what its view can
/// say of a tank three cells off, so it never reaches `Engage` and never
/// fires), with the cannon's damage nought. A mind's slot fires only
/// at odds above its card's floor, and a hull presenting no band is nought
/// to every shot (the divergence `floor_held`, 17b cut M1b.2) — so the
/// reflex-era pair, whose every shot missed a silhouette-less target, would
/// never spend a mind's magazine — and a presented eighteen-point hull dies
/// to one 120-damage hit, which ends the match by elimination before the
/// draw. A harmless gun keeps the shots and the draw: what the fixture pins
/// is the arena millisecond the magazines run out at, never what a round
/// does to armour. The pair's rounds are the fixture's constants, unchanged.
fn t1_e4_spent_pair(red_rounds: u32, blue_rounds: u32, blue: Controller) -> Arena {
    let harmless = WeaponSummary {
        damage: 0,
        ..cannon()
    };
    let seated: &[ObjId] = if blue == Controller::Pilot {
        &[BLUE]
    } else {
        &[]
    };
    let mut arena = arena_of(
        vec![
            presenting(seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![harmless.clone()],
                    Controller::Mind,
                ),
                10,
            )),
            presenting(seeing(
                tank(BLUE, Team::Blue, centre(9, 8), vec![harmless], blue),
                10,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        seated,
    );
    for (unit, rounds) in [(RED, red_rounds), (BLUE, blue_rounds)] {
        if let Some(state) = arena.unit_mut(unit) {
            state.ammo.insert(SmolStr::new("cannon"), rounds);
        }
    }
    arena
}

/// The `Spent` draw, as a function nothing but the cadence can vary.
fn t1_both_sides_dry() -> Arena {
    t1_e4_spent_pair(0, 0, Controller::Pilot)
}

/// The same with the last round in the air — `a_round_still_in_flight_holds_the
/// _draw_off`'s own fixture, so the replay is of the case the draw is held off
/// for and not only of the easy one.
fn t1_one_round_in_flight() -> Arena {
    let mut arena = t1_e4_spent_pair(1, 0, Controller::Pilot);
    if let Some(weapon) = arena
        .unit_mut(RED)
        .and_then(|state| state.weapons.first_mut())
    {
        weapon.muzzle_speed_cms = 1_000;
    }
    arena
}

/// **Both sides fire their last round in the middle of the match**: two minds
/// facing each other with one round each, so the draw is called by the arena
/// after both rounds have left the breech and landed — several pulses in, at
/// every cadence, and not on the opening substep as [`t1_both_sides_dry`]'s is.
fn t1_both_fire_their_last() -> Arena {
    t1_e4_spent_pair(1, 1, Controller::Mind)
}

/// **17b cut M4.4 — `EndReason::Spent` at three cadences, on the unit mind**
/// (16c `T-42`; doc 16 §3.8; 16b §11).
///
/// The reflex fixture's claims are kept, script for script: the plain draw
/// (both magazines empty at check-out, called on the opening substep), the
/// one whose last round is still in the air when the magazines empty, and
/// the one where both sides fire their last round mid-match; one
/// `match_ended … spent` line, last, with M3.6's `sortie_closed
/// { match_end }` block right before it at the end's millisecond and hash;
/// exactly 0, 1 and 2 `fired` lines, every one at or before the end; the
/// two that fire replayed over several pulses at three different counts;
/// and the events, the hashes and the finals one match at 5,000, 1,000 and
/// 500 ms. What moved: the machines are minds ([`t1_e4_spent_pair`], which says
/// why the targets present and the gun is harmless); `replay_of` steps with
/// `step_settled` and reads the finals' state off the cached doctrine state.
/// T1 cites its one `Fire` at the fresh-lock landing, so a one-round
/// magazine yields exactly one `fired` line per shooter (verified on the
/// run). The edge is the arena's (`Arena::spent`; M3.6's close-before-end);
/// the one-cadence half is `t1_two_dry_sides_end_in_a_draw_and_one_dry_side_plays_on`'s.
#[test]
fn t1_the_spent_draw_is_one_log_at_three_cadences() {
    for (build, what, rounds) in [
        (
            t1_both_sides_dry as fn() -> Arena,
            "both sides dry",
            0_usize,
        ),
        (
            t1_one_round_in_flight as fn() -> Arena,
            "a round in flight",
            1,
        ),
        (
            t1_both_fire_their_last as fn() -> Arena,
            "both fire their last",
            2,
        ),
    ] {
        let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
            .into_iter()
            .map(|cadence| replay_of(cadence, 30_000, build, no_script))
            .collect();
        let first = runs.first().expect("three replays");
        let ended: Vec<&(i64, String, u64)> = first
            .events
            .iter()
            .filter(|(_, kind, _)| kind.contains("match_ended"))
            .collect();
        println!("t1_e4_spent ({what}): ended {ended:?}");
        assert_eq!(
            ended.len(),
            1,
            "{what}: the match should end exactly once: {ended:?}"
        );
        let (end_ms, end_kind, _) = ended.first().copied().expect("one end");
        assert!(
            end_kind.contains("spent"),
            "{what}: and it should end `Spent`: {ended:?}"
        );
        // **17b cut M3.6** (doc 17 §13): `Spent`, settled inside a substep,
        // closes every sortie still open first — one `sortie_closed
        // { match_end }` per sealed hull not already closed at its
        // destruction, right before the end at its millisecond and hash —
        // and the end stays the tape's last line.
        let count = |kind: &str| {
            first
                .events
                .iter()
                .filter(|(_, json, _)| json.contains(kind))
                .count()
        };
        let open = count(r#""kind":"mind_sealed""#) - count(r#""word":"destroyed""#);
        assert!(open > 0, "{what}: a sortie is open at the end");
        let last = first.events.len() - 1;
        assert!(
            first.events[last].1.contains("match_ended"),
            "{what}: the end is the last line"
        );
        for line in &first.events[last - open..last] {
            assert!(
                line.1.contains(r#""kind":"sortie_closed""#)
                    && line.1.contains(r#""word":"match_end""#),
                "{what}: {line:?}"
            );
            assert_eq!(
                (line.0, line.2),
                (first.events[last].0, first.events[last].2),
                "{what}: at the end's millisecond and hash"
            );
        }
        assert_eq!(
            count(r#""word":"match_end""#),
            open,
            "{what}: one close per open sortie"
        );
        let fired: Vec<i64> = first
            .events
            .iter()
            .filter(|(_, kind, _)| kind.split('"').nth(3) == Some("fired"))
            .map(|(t_ms, _, _)| *t_ms)
            .collect();
        assert_eq!(
            fired.len(),
            rounds,
            "{what}: {} rounds left the breech, not {rounds}",
            fired.len()
        );
        if rounds > 0 {
            // At or before: the stock cannon's round crosses three cells inside
            // the substep that fired it, so both of `t1_both_fire_their_last`'s
            // land at the millisecond they left and the draw is called there —
            // seconds in, which is what makes it mid-match.
            assert!(
                *end_ms > 0 && fired.iter().all(|t_ms| t_ms <= end_ms),
                "{what}: every round was fired before the draw at {end_ms} ms: {fired:?}"
            );
            assert!(
                first.hashes.len() > 3,
                "{what}: a replay of one pulse is not a cross-tempo property"
            );
            let counts: Vec<u32> = runs.iter().map(|run| run.pulses).collect();
            assert!(
                counts.windows(2).all(|pair| pair[0] != pair[1]),
                "{what}: the three replays ran the same number of pulses: {counts:?}"
            );
        }
        for (at, run) in runs.iter().enumerate().skip(1) {
            assert_eq!(
                run.events, first.events,
                "{what}, cadence {at}: the event list differs"
            );
            assert_eq!(
                run.hashes, first.hashes,
                "{what}, cadence {at}: the hash sequence differs"
            );
            assert_eq!(
                run.finals, first.finals,
                "{what}, cadence {at}: the field differs"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 16b cut AA.1 — the immobile machine (16c `B-76`)
//
// A machine whose engine was killed while it held a hostile between its two
// edges — past `engage_cm`, inside `release_cm` — never fired again: `Approach`
// re-planned a stand-off cell it could not drive to on every tick, its drive was
// refused `MotionDisabled`, and `engage_orders` was never reached although its
// own ceiling was `release_cm`. Carrying a gun it was never `disabled`, so it held
// its side up for the rest of the match. `UnitState::cannot_close` is the reading
// the two edges it needed are now chosen off.
//
// 17b cut M4.4: the family runs on the unit mind (`t1_*` below), whose view
// reads `immobile` off `limits.motion_disabled` alone; the authored-zero-speed
// fixture stays on the reflex as M4's recorded residual (STOP B of the
// M4.4 inventory, and `stock_mind.rs`'s head).
// ---------------------------------------------------------------------------

/// **Kill a unit's engine the way `damage.rs`'s `disable_motion` does**: motion
/// disabled, the speed and the ordered speed to nought — and nothing else. No
/// wreck, no power change, so the machine still sees, locks and shoots.
fn disable_motion(arena: &mut Arena, unit: ObjId) {
    if let Some(state) = arena.unit_mut(unit) {
        state.limits.motion_disabled = true;
        state.speed_cms = 0;
        state.desired_speed_cms = 0;
    }
}

/// **A true range as a think first reads it** (17b cut M4.4): rounded to the
/// nearest `CONTACT_RANGE_STEP_M` (50 m) and never under it, the view's own
/// `range_m_rounded` — what a fixture's cell has to be checked against
/// before a claim about a mind's edge is made on it (the `range_round`
/// divergence). A row the view has held is re-read only outside
/// `held_range_m`'s tolerance; every cell below is placed so the plain
/// rounding and the held figure agree.
fn t1_view_range_cm(range_cm: i64) -> i64 {
    i64::try_from(
        tacsim::arena::view::range_m_rounded(
            u64::try_from(range_cm).unwrap_or(0),
            tacsim::arena::perception::CONTACT_RANGE_STEP_M,
        )
        .saturating_mul(100),
    )
    .unwrap_or(i64::MAX)
}

/// **The figure `unit`'s last kept think read for the hostile** (17b cut
/// M4.4), centimetres: the held `range_m` on the view, which is what a claim
/// about a mind's edge discriminates on (and what [`t1_view_range_cm`]'s
/// plain rounding is checked against).
fn t1_last_read_cm(stock: &Stock, unit: ObjId) -> Option<i64> {
    stock
        .kept()
        .iter()
        .rev()
        .find(|think| think.0 == unit)
        .and_then(|think| think.2.contacts.iter().find(|row| !row.friendly))
        .map(|row| i64::try_from(row.range_m).unwrap_or(i64::MAX) * 100)
}

/// **A gun that marks and does not kill** (17b cut M4.4): `weapon` at no
/// damage, for the reason [`t1_q7_spent_pair`]'s doc gives — a presented
/// eighteen-point hull dies to one 120-damage hit and the match ends by
/// elimination at the first shot, where every round of the reflex-era
/// fixture missed a silhouette-less target and the fight went on. What this
/// family pins is the edge a mind enters `Engage` at and the shot it takes
/// there, never what a round does to armour.
fn t1_aa1_harmless(weapon: WeaponSummary) -> WeaponSummary {
    WeaponSummary {
        damage: 0,
        ..weapon
    }
}

/// **The pose every case below starts from, on the unit mind** (17b cut M4.4;
/// 16b cut AA.1's `aa1_posed`): RED a `Controller::Mind` seeing `sight` cells
/// at `(4, 8)`, outside its entry zone, and BLUE at `blue` presenting every
/// band ([`t1_e1_duel_with`]: a mind's shot is warranted at odds, and a
/// default profile presents none), both put there **after** the power-up run
/// for the reason [`t1_a_withdrawal_beside`]'s doc gives — a machine drives
/// toward its nearest waypoint through its first seconds, so a case written
/// as "RED at (4, 8)" would otherwise be measured wherever the patrol had
/// drifted to. The contact is planted classified and the next think is
/// landed ([`t1_next_landing`]: a substep at a time, at most a think period
/// and a substep where the reflex stepped one 500 ms tick) so the hostile
/// edge has put the mind in `Approach` — and no further: the landing's goal
/// has not yet moved the hull when the helper returns (motion runs before
/// the substep's due events), so a case that kills the engine or wounds the
/// hull does it at the cell the case names. The recorder is handed back so
/// a case can read a think's view (the health the mind read).
fn t1_aa1_posed(
    weapons: Vec<WeaponSummary>,
    blue: (u16, u16),
    sight: u16,
) -> (Arena, std::sync::Arc<Stock>, Vec<u8>) {
    let mut arena = t1_e1_duel_with(weapons, (4, 8), blue, &[], sight);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(4, 8);
        state.speed_cms = 0;
    }
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(blue.0, blue.1);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let _ = t1_next_landing(&mut arena, &stock, &mut bytes);
    assert!(
        arena.now_ms - now <= 1_250,
        "the first landing after the plant: {} ms on",
        arena.now_ms - now
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Approach),
        "the recipe: a classified hostile past `engage_cm` puts the mind in `Approach`"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Approach);
    (arena, stock, bytes)
}

/// Step `run_ms` more of arena time, a settled pulse at a time, and answer
/// RED's plot range at the first pulse that carried a new `fired` line — the
/// range the first shot was taken at, read at the pulse that carried it.
fn t1_aa1_first_shot_range(arena: &mut Arena, bytes: &mut Vec<u8>, run_ms: i64) -> Option<i64> {
    let before = count(&events(arena, bytes), "fired");
    let until = arena.now_ms + run_ms;
    let mut wall = arena.now_ms;
    let mut first: Option<i64> = None;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        if first.is_none() && count(&events(arena, bytes), "fired") > before {
            first = Some(plot_range(arena, RED, BLUE));
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    first
}

/// **Write `objective` for RED's squad under `constraints`, live for a
/// minute, as the world hands a mind one** (17b cut M4.4): an
/// `IntentKind::Order` at the hull's helm epoch through a zero-length
/// settled pulse at `now_ms` — `apply_intent`'s Mind arm re-keys it
/// (`Arena::rekey_order`) onto the mind row's held `FrameOrder`, whose
/// `hold_fire` and `max_range_band` are what the next think's `view_ranges`
/// and `engage_orders` read. `aa1_order` wrote the reflex row
/// (`ReflexMemory::set_order`, which has no mind arm).
fn t1_aa1_order(
    arena: &mut Arena,
    objective: tacsim::deliberation::Objective,
    constraints: tacsim::deliberation::Constraints,
    bytes: &mut Vec<u8>,
) {
    let squad = arena
        .squad_of(RED)
        .map(|roster| roster.id)
        .expect("one mind is a squad of one");
    let told = tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: 60_000,
        constraints,
        ttl_ms: 60_000,
        issued_ms: arena.now_ms,
        issuer: None,
        by: author(squad, None),
        spot: None,
    };
    let kind = tacsim::arena::intents::IntentKind::Order(Box::new(told));
    let channel = kind.channel_required();
    let intent = tacsim::arena::intents::Intent {
        unit: RED,
        channel,
        epoch: arena.epochs.current(RED, channel),
        kind,
        seq: 1,
    };
    let now = arena.now_ms;
    let out = arena.step_settled(now, &[], &[intent]);
    absorb(&out, bytes);
    assert!(
        arena
            .minds
            .get(&RED)
            .is_some_and(|row| row.order.as_ref().is_some_and(|order| {
                order.hold_fire == constraints.hold_fire
                    && order.max_range_band == constraints.max_range_band
            })),
        "the order landed on the mind row with its constraints"
    );
}

/// **17b cut M4.4 — a mind that cannot close engages from where it stands**
/// (16b cut AA.1; 16c `B-76`), and its mobile twin still closes to
/// `engage_cm` first. Every claim of the reflex fixture is kept.
///
/// The recipe leaves RED in `Approach` 8,000 cm from BLUE — past `ENGAGE_CM`,
/// inside `RELEASE_CM`. With its engine killed there, the mind's edge into
/// `Engage` is `release_cm` (the Approach arm's `close_edge` when the view's
/// `immobile` — `limits.motion_disabled`, the one thing `disable_motion`
/// sets — is up): it locks, cites its shot on the fresh lock and never
/// moves, the helm half of the emission being withheld for an immobile
/// hull. The reflex machine re-planned a stand-off cell every tick, was
/// refused the drive, and never shot again.
///
/// The twin is the contrast that makes it a proof: the same recipe with the
/// engine running drives in and takes its first shot inside `ENGAGE_CM` —
/// firing on the move while closing is not built (16b row E), and AA.1 does
/// not change it. What moved: the hull is a `Controller::Mind` on
/// [`t1_aa1_posed`] (the state off the doctrine state and the memory), the
/// `Approach` is read at the first landing after the plant rather than one
/// 500 ms tick on, and the two 6,000 and 30,000 ms windows are the
/// original's — each holds a think period and a substep of latency.
#[test]
fn t1_a_mind_that_cannot_close_engages_from_where_it_stands() {
    let (mut arena, _stock, mut bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(cannon())], (12, 8), SIGHT_CELLS);
    disable_motion(&mut arena, RED);
    let pose = arena.unit(RED).map(|unit| unit.pose.clone());
    let first = t1_aa1_first_shot_range(&mut arena, &mut bytes, 6_000);

    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
    let log = events(&mut arena, &mut bytes);
    assert!(count(&log, "locked") > 0, "it locked the plot it holds");
    assert!(
        count(&log, "fired") > 0,
        "and it fired from where it stands"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.pose.clone()),
        pose,
        "a dead engine does not move, or turn"
    );
    let first = first.expect("a first shot");
    assert!(
        first > ENGAGE_CM && first <= RELEASE_CM,
        "the first shot was taken at {first} cm, which is not between the two edges"
    );

    // The mobile twin: the same recipe, the engine running.
    let (mut twin, _stock, mut twin_bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(cannon())], (12, 8), SIGHT_CELLS);
    let first = t1_aa1_first_shot_range(&mut twin, &mut twin_bytes, 30_000)
        .expect("the mobile twin closed and fired");
    assert!(
        first <= ENGAGE_CM,
        "a mind that can close still holds its fire to `engage_cm`: its first shot was at \
         {first} cm"
    );
}

/// **A hull authored with no speed engages the same way** (16b cut AA.1) —
/// N13's fixed-target shape, `mobility.max_speed_cms` at nought and its motion
/// not disabled.
///
/// **Position only.** A zero-speed hull that is not `motion_disabled` still
/// turns in place (`intents.rs` refuses a heading only under `motion_disabled`),
/// so it turns onto the plot; what it cannot do is close, which is the whole of
/// `UnitState::cannot_close`. No re-pose is needed: it never moves, and BLUE at
/// eight cells is inside the ten-cell sight, so the sensor pass classifies it
/// without a plant.
#[test]
fn aa1_a_hull_authored_with_no_speed_engages_the_same_way() {
    let mut arena = duel((4, 8), (12, 8), &[]);
    if let Some(state) = arena.unit_mut(RED) {
        state.mobility.max_speed_cms = 0;
    }
    let opened = arena.unit(RED).map(|unit| (unit.pose.x_cm, unit.pose.y_cm));
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    while arena.now_ms < 15_000
        && !(state_of(&arena, RED) == Some(MachineState::Engage)
            && count(&events(&mut arena, &mut bytes), "fired") > 0)
    {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        wall += 500;
    }
    assert_eq!(
        state_of(&arena, RED),
        Some(MachineState::Engage),
        "a zero-speed hull 8,000 cm out engages"
    );
    assert!(count(&events(&mut arena, &mut bytes), "fired") > 0);
    assert_eq!(
        arena.unit(RED).map(|unit| (unit.pose.x_cm, unit.pose.y_cm)),
        opened,
        "and it never left its opening position"
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > ENGAGE_CM && range <= RELEASE_CM,
        "from between the two edges: {range} cm"
    );
}

/// **17b cut M4.4 — the ceiling holds for an immobile mind** (16b cut AA.1):
/// its edge is `release_cm` and no further, on a gun whose reach is shorter
/// than the sight. Every claim of the reflex fixture is kept.
///
/// The gun is the five-cell [`coaxial`], for the reason
/// [`t1_close_engage_and_release`]'s doc gives: with the cannon the release edge
/// IS the sight, so a planted contact past it dies after `contact_ttl_ms` and
/// the `lost` edge drops the mind to `Patrol` for a reason that has nothing
/// to do with the ceiling. BLUE is past `COAXIAL_RELEASE_CM` and inside the
/// sight, so RED's own sensor keeps the contact live and nothing expires. On
/// T1 the stand-off goal past the ceiling is computed and never emitted for
/// an immobile hull, so the mind stays `Approach` with no lock (the lock is
/// `engage_orders`', reached only in `Engage` or a stand); re-posed inside
/// the annulus and re-planted, BLUE reads 50 m — the view's floor, inside the
/// 5,000 cm ceiling — and the mind engages and fires from where it stands.
/// What moved: the hull is a `Controller::Mind` on [`t1_aa1_posed`], the
/// state the doctrine state; the two 6,000 ms windows are the original's,
/// each with a think's slack; and the first half's target stands eight cells
/// out, not seven — a contact's range reaches a think rounded to the nearest
/// 50 m (the `range_round` divergence), so 7,000 cm reads 50 m, the ceiling
/// itself, and 8,000 reads 100 m, past it: the cell is the fixture's,
/// re-derived against the view, the claim the original's.
#[test]
fn t1_the_ceiling_past_release_holds_for_an_immobile_mind() {
    let (mut arena, stock, mut bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(coaxial())], (12, 8), SIGHT_CELLS);
    disable_motion(&mut arena, RED);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > COAXIAL_RELEASE_CM && t1_view_range_cm(range) > COAXIAL_RELEASE_CM,
        "the fixture put the target at {range} cm, which is not past the release edge on the view"
    );
    assert_eq!(
        t1_last_read_cm(&stock, RED),
        Some(10_000),
        "the mind read 100 m: past the coaxial's ceiling"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Approach),
        "past its firing ceiling an immobile mind stays in `Approach`"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Approach);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "locked"), 0, "and locks nothing");
    assert_eq!(count(&log, "fired"), before, "and fires nothing");

    // Inside the release edge and outside `COAXIAL_ENGAGE_CM`: it engages there.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(8, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > COAXIAL_ENGAGE_CM && range <= COAXIAL_RELEASE_CM,
        "the fixture put the target at {range} cm, which is not between the coaxial's edges"
    );
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        t1_last_read_cm(&stock, RED),
        Some(5_000),
        "the mind read 50 m: inside the ceiling, past its seven tenths"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "inside the release edge it shoots from where it stands"
    );
}

/// **17b cut M4.4 — an ordered `close` caps the immobile edge too** (16b cut
/// AA.1; 16b cut N.2): the order's band caps `release_cm` inside
/// `view_ranges` as it did inside `ranges()`, so the edge a mind that cannot
/// close enters `Engage` at is the ORDERED ceiling. Every claim is kept: past
/// the ordered ceiling and inside doctrine's it never reads `Engage` (read
/// per settled pulse, which is every landing), locks nothing and fires
/// nothing; inside the ordered ceiling and past its seven tenths it engages
/// and shoots.
///
/// What moved, and why: the order is an `IntentKind::Order` through the step
/// ([`t1_aa1_order`]); and the sight is [`T1_STAND_SIGHT_CELLS`] (sixteen),
/// with the first half's target at the far corner `(15, 15)` and the second
/// half's at `(8, 8)`, because the view reads a contact's range to the
/// nearest 50 m and never under it (the `range_round` divergence): at the
/// ten-cell sight the `close` ceiling is 3,333 cm, under the 5,000 cm floor,
/// so no row could ever read inside it and the second half would be
/// unreachable. At sixteen cells the ceiling is 5,333 cm (a third of the
/// 16,000 cm reach, doctrine's `release_cm` here), its seven tenths 3,733,
/// and doctrine's own `engage_cm` 11,200 — so the recipe's hostile, which
/// must be past THAT for the recipe to leave the mind in `Approach`, has to
/// read 150 m, and the far corner (13,038 cm from `(4, 8)`) is the safest
/// cell of this window that does, clear of the 12,500 cm rounding edge — it
/// reads 150 m because the row has no figure, or 150 m, before it (a held
/// 100 m would hold to 13,125 cm, which the read pin below guards): past the
/// ordered ceiling, inside doctrine's. Four cells out
/// reads 50 m, inside the ordered ceiling and past its seven tenths, which is
/// where the true range 4,000 also stands. The constants are the fixture's,
/// re-derived; the claim is the original's.
#[test]
fn t1_a_close_band_order_caps_the_immobile_edge_too() {
    use tacsim::deliberation::{Constraints, Objective, RangeBand};

    // `view_ranges`' rule for this fixture: the order's band against the
    // cannon's edges and the sixteen-cell sight, and seven tenths of it for
    // `engage_cm`.
    let sight = u32::from(T1_STAND_SIGHT_CELLS) * u32::from(CELL_CM);
    let close_release =
        i64::try_from(RangeBand::Close.ceiling_cm(&cannon().bands_cm(), sight)).unwrap_or(0);
    let close_engage = close_release * 700 / 1_000;
    assert_eq!(close_release, 5_333);
    assert!(
        close_release >= T1_RANGE_FLOOR_CM,
        "the ordered ceiling must hold the view's floor, or nothing reads inside it"
    );

    let (mut arena, stock, mut bytes) = t1_aa1_posed(
        vec![t1_aa1_harmless(cannon())],
        (15, 15),
        T1_STAND_SIGHT_CELLS,
    );
    disable_motion(&mut arena, RED);
    t1_aa1_order(
        &mut arena,
        Objective::Engage { target_hint: None },
        Constraints {
            hold_fire: false,
            max_range_band: Some(RangeBand::Close),
        },
        &mut bytes,
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > close_release && range <= T1_STAND_REACH_CM,
        "the target is at {range} cm: inside doctrine's ceiling and past the order's"
    );
    assert!(
        t1_view_range_cm(range) > T1_STAND_ENGAGE_CM,
        "and past doctrine's `engage_cm` as the view reads it (the nearest 50 m), so the \
         recipe's `Approach` is doctrine's own: {range} cm"
    );
    let log = events(&mut arena, &mut bytes);
    let before = count(&log, "fired");
    let locked_before = count(&log, "locked");
    // Sampled every settled pulse — every landing's state — and the `locked`
    // lines counted: `engage_orders` stages its lock before its own
    // `release_cm` return, so an edge wider than the ordered ceiling would
    // show as a mind flicking into `Engage` for a think, locking, and being
    // thrown back — with nothing fired and the end state `Approach`, which
    // the last two assertions alone cannot tell from a held edge.
    let until = arena.now_ms + 6_000;
    while arena.now_ms < until {
        let next = arena.now_ms + 500;
        run_settled(&mut arena, next, &mut bytes);
        assert_ne!(
            machine_state_of(&arena, RED),
            Some(MachineState::Engage),
            "past the ordered ceiling it never reads `Engage` (at {} ms)",
            arena.now_ms
        );
    }
    assert_eq!(
        t1_last_read_cm(&stock, RED),
        Some(15_000),
        "the mind read 150 m: past the ordered ceiling, inside doctrine's"
    );
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Approach),
        "past the ordered ceiling it does not engage"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Approach);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "locked"), locked_before, "and locks nothing");
    assert_eq!(count(&log, "fired"), before, "and fires nothing");

    // Inside the ordered ceiling, past its seven tenths: the immobile edge.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(8, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > close_engage && range <= close_release,
        "the target is at {range} cm, which is not between the ordered edges"
    );
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        t1_last_read_cm(&stock, RED),
        Some(5_000),
        "the mind read 50 m: inside the ordered ceiling, past its seven tenths"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "inside the ordered ceiling it shoots"
    );
}

/// **17b cut M4.4 — `hold_fire` still holds an immobile mind** (16b cut AA.1;
/// 16b cut N.2): the edge brings it into `Engage` and `engage_orders` takes
/// the lock and keeps the trigger — it returns under `hold_fire` before the
/// slots arm, so no slot owns a shot either (stock_mind.rs's
/// `m1b2_t1_arms_on_the_admitting_ceiling…` pins "lock taken, slots
/// DISARMED" at the unit level; `tests/conditions.rs`'
/// `m1b2_hold_fire_stops_a_mind_and_its_slot_and_never_a_pilot` the same on
/// a mobile hull). What moved: the order is an `IntentKind::Order` through
/// the step ([`t1_aa1_order`]), the hull a `Controller::Mind`, the state the
/// doctrine state.
#[test]
fn t1_hold_fire_still_holds_an_immobile_mind() {
    use tacsim::deliberation::{Constraints, Objective};

    let (mut arena, _stock, mut bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(cannon())], (12, 8), SIGHT_CELLS);
    disable_motion(&mut arena, RED);
    t1_aa1_order(
        &mut arena,
        Objective::Engage { target_hint: None },
        Constraints {
            hold_fire: true,
            max_range_band: None,
        },
        &mut bytes,
    );
    let log = events(&mut arena, &mut bytes);
    let before = count(&log, "fired");
    let locked_before = count(&log, "locked");
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Engage),
        "the edge still brings it into `Engage`"
    );
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
    assert!(
        count(&events(&mut arena, &mut bytes), "locked") > locked_before,
        "the lock is taken under the order"
    );
    assert_eq!(
        count(&events(&mut arena, &mut bytes), "fired"),
        before,
        "`hold_fire` stages no FIRE, whether or not the mind can move"
    );
}

/// **17b cut M4.4 — a hurt immobile mind stands and shoots back at any
/// health** (16b cut AA.1; the E row's Q8 departure, extended). Every claim
/// of the reflex fixture is kept.
///
/// `hurt` is 166 per mille — under `withdraw_permille` AND under
/// `stand_min_permille`, so the en-route stand is closed to it. With its
/// engine killed it is home wherever it is (the Withdraw arm reads `arrived`
/// off the view's `immobile`), so its ceiling is `release_cm`
/// (`withdraw_stand` admits the row inside it at any health): it stands —
/// `stand_until_ms` on the stock memory — and fires at BLUE 8,000 cm out.
///
/// The mobile hurt twin is E.1's rule re-asserted, and the proof that
/// `(4, 8)` is en route and not home: it withdraws, never stands and fires
/// nothing while it drives. That is what makes the immobile half a new rule
/// rather than the `arrived` arm E.1 already had. The twin is read over
/// three seconds — the original's two and one think period — before it can
/// have covered the two cells that would put it inside `ARRIVE_CELLS` of its
/// entry zone: the wound is read a think late at most, and the Withdraw
/// landing (982 ms after the wound on this lattice) turns round a hull the
/// Approach landing's goal had already set moving east, so at two seconds
/// it is still east of where the wound found it; at three it is west of it
/// in its own cell, two cells from its entry zone, which it reaches at four
/// and a half (measured on this rig; the reflex read it 18 ms into a 500 ms
/// tick and had no drive to turn). What moved: the hull is a
/// `Controller::Mind` on [`t1_aa1_posed`]; the health is the figure the
/// mind's first think after the wound read (`stock_mind::health_permille`
/// over the kept view, where the reflex read `reflex_health_permille`); the
/// state is the doctrine state, the stand the memory's `stand_until_ms`,
/// `ARRIVE_CELLS` is `arena::motor`'s.
#[test]
fn t1_a_hurt_immobile_mind_stands_and_shoots_back_at_any_health() {
    let (mut arena, stock, mut bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(cannon())], (12, 8), SIGHT_CELLS);
    hurt(&mut arena, RED);
    disable_motion(&mut arena, RED);
    let thought = stock.kept().len();
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        stock
            .kept()
            .get(thought)
            .map(|kept| tacsim::arena::stock_mind::health_permille(&kept.2)),
        Some(166),
        "the recipe, read by the first think after the wound"
    );
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Withdraw));
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Withdraw, "{memory:?}");
    assert!(memory.stand_until_ms.is_some(), "it stood: {memory:?}");
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and shot back from where it stands, at 166 per mille"
    );

    // The mobile hurt twin.
    let (mut twin, _stock, mut twin_bytes) =
        t1_aa1_posed(vec![t1_aa1_harmless(cannon())], (12, 8), SIGHT_CELLS);
    hurt(&mut twin, RED);
    let from = twin.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut twin, &mut twin_bytes), "fired");
    let until = twin.now_ms + 3_000;
    run_settled(&mut twin, until, &mut twin_bytes);
    assert_eq!(machine_state_of(&twin, RED), Some(MachineState::Withdraw));
    let memory = stock_memory_of(&twin, RED);
    assert_eq!(memory.state, MachineState::Withdraw, "{memory:?}");
    let here = twin
        .unit(RED)
        .and_then(|unit| twin.definition.terrain.cell_of(&unit.pose))
        .expect("the twin is on the window");
    let goal = twin
        .motors
        .row(RED)
        .and_then(|row| row.goal)
        .expect("a withdrawing mind has somewhere to be");
    assert!(
        here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) > tacsim::arena::motor::ARRIVE_CELLS,
        "the twin is still en route: at {here:?}, home {goal:?}"
    );
    assert!(
        twin.unit(RED).is_some_and(|unit| unit.pose.x_cm < from),
        "and driving home: from {from}, at {:?}",
        twin.unit(RED).map(|unit| unit.pose.x_cm)
    );
    assert_eq!(
        memory.stand_until_ms, None,
        "en route and under `stand_min_permille` it never stands"
    );
    assert_eq!(
        count(&events(&mut twin, &mut twin_bytes), "fired"),
        before,
        "and fires nothing while it drives"
    );
}

/// **17b cut M4.4 — an immobile withdrawn mind rallies once, like one that
/// is home** (16b cut AA.1), and then fights through the immobile edge: the
/// whole path a hurt mind with a dead engine takes. Every claim is kept.
///
/// BLUE opens eleven cells out, past the ten-cell sight, so nothing is held
/// while the rally clock runs: immobile is `arrived`, so the Withdraw arm's
/// quiet clock runs from the first hurt think and `rally_ms` later the mind
/// is `Patrol` with `rallied` on the memory. The planted contact then takes
/// it to `Approach` at the first landing — read inside a think period and a
/// substep (1,250 ms), where the reflex read one 500 ms tick on: the one
/// window that moved — and to `Engage` at `release_cm`, from where it
/// stands. What moved: the hull is a `Controller::Mind` ([`t1_e1_duel_with`]
/// with a harmless cannon, [`t1_aa1_harmless`]), the state the doctrine
/// state and `rallied` the memory's; the 2,000 and `rally + 3,000` windows
/// are the original's, each with a think's slack.
#[test]
fn t1_an_immobile_withdrawn_mind_rallies_once_like_one_that_is_home() {
    let mut arena = t1_e1_duel_with(
        vec![t1_aa1_harmless(cannon())],
        (4, 8),
        (15, 8),
        &[],
        SIGHT_CELLS,
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();
    run_settled(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(4, 8);
        state.speed_cms = 0;
    }
    hurt(&mut arena, RED);
    disable_motion(&mut arena, RED);
    let until = arena.now_ms + 2_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Withdraw),
        "hurt, it breaks off"
    );
    assert!(!stock_memory_of(&arena, RED).rallied);
    let rally = i64::from(reflex_cfg().rally_ms);
    let until = arena.now_ms + rally + 3_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(
        machine_state_of(&arena, RED),
        Some(MachineState::Patrol),
        "twenty quiet seconds later it has rallied where it stands"
    );
    let memory = stock_memory_of(&arena, RED);
    assert_eq!(memory.state, MachineState::Patrol, "{memory:?}");
    assert!(memory.rallied, "{memory:?}");

    // Now something comes: it approaches and engages from where it stands.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(12, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let _ = t1_next_landing(&mut arena, &stock, &mut bytes);
    assert!(
        arena.now_ms - now <= 1_250,
        "the first landing after the plant is inside a think period and a substep"
    );
    assert!(
        matches!(
            machine_state_of(&arena, RED),
            Some(MachineState::Approach | MachineState::Engage)
        ),
        "a rallied mind takes the hostile: {:?}",
        machine_state_of(&arena, RED)
    );
    let until = arena.now_ms + 6_000;
    run_settled(&mut arena, until, &mut bytes);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));
    assert_eq!(stock_memory_of(&arena, RED).state, MachineState::Engage);
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and fights to the end"
    );
}

/// **An immobile mind in the coaxial's annulus, as one scripted arena** (16b
/// cut AA.1; 16b §11's mandatory cross-tempo test; **17b cut M4.4** on the
/// unit mind): RED a `Controller::Mind` holding the [`coaxial`] — harmless
/// ([`t1_aa1_harmless`]), so the `lost` edge below is reached on a live
/// target — with its engine killed before the first step — so all three
/// cadences share the fact from arena time nought; the power-up is not
/// motion and still runs — 4,000 cm from BLUE, inside `COAXIAL_RELEASE_CM`
/// and outside `COAXIAL_ENGAGE_CM`. It engages from where it stands (the
/// view reads the row at its 50 m floor, inside the immobile edge's 5,000 cm
/// ceiling); at [`SCRIPT_AT_MS`] BLUE is put past the release edge and the
/// sight ([`blue_breaks_contact`]), the contact expires, and the `lost` edge
/// drops RED to `Patrol`.
fn t1_immobile_in_the_annulus() -> Arena {
    let mut arena = t1_e1_duel_with(
        vec![t1_aa1_harmless(coaxial())],
        (2, 8),
        (6, 8),
        &[],
        SIGHT_CELLS,
    );
    disable_motion(&mut arena, RED);
    arena
}

/// **17b cut M4.4** — every claim of the reflex fixture is kept: the arc
/// walked once at one cadence (the doctrine state per settled pulse,
/// `Engage` inside the annulus before the script and `Patrol` after the
/// contact is gone), then the three replays under `step_settled` with the
/// five event kinds present, at least a hundred substeps, and one event
/// list, one hash sequence and one field (its state column the doctrine
/// state).
#[test]
fn t1_an_immobile_mind_in_the_annulus_is_one_log_at_three_cadences() {
    // First: the arc really happens, at one cadence, before anything is compared.
    let mut arena = t1_immobile_in_the_annulus();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut arc: Vec<(i64, MachineState, i64)> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if arena.now_ms == SCRIPT_AT_MS {
            blue_breaks_contact(&mut arena);
        }
        if let Some(state) = machine_state_of(&arena, RED) {
            arc.push((arena.now_ms, state, plot_range(&arena, RED, BLUE)));
        }
        if arena.now_ms >= 40_000 || out.ended.is_some() {
            break;
        }
        wall += 1_000;
    }
    assert!(
        arc.iter().any(|(at, state, range)| *at < SCRIPT_AT_MS
            && *state == MachineState::Engage
            && *range > COAXIAL_ENGAGE_CM
            && *range <= COAXIAL_RELEASE_CM),
        "it never engaged from inside the annulus before the script: {arc:?}"
    );
    assert!(
        arc.iter()
            .any(|(at, state, _)| *at > SCRIPT_AT_MS && *state == MachineState::Patrol),
        "it never dropped to `Patrol` once the contact was gone: {arc:?}"
    );

    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                40_000,
                t1_immobile_in_the_annulus,
                blue_breaks_contact,
            )
        })
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "detected", "classified", "locked", "fired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    assert!(first.hashes.len() >= 100, "{} substeps", first.hashes.len());
    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = [5_000_i64, 1_000, 500].get(at).copied().unwrap_or(0);
        assert_eq!(
            run.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the field ended differently at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut M.2 (16c `T-02`) — the one-clock rule, as the unit mind has it
// (17b cut M4.4: the reflex's "stamped at its tick" fixture is deleted — the
// mind's form of T-02's stamp is `tests/minds.rs`'
// `m1a5_a_think_sees_the_substeps_end_and_lands_one_substep_later`,
// `m3_a_resumed_landing_lands_the_carried_answer`,
// `m1a6_two_stock_minds_at_three_cadences_are_one_event_list_and_one_hash_sequence`
// and `m1b0_the_beam_before_the_think_holds_at_three_cadences`; and the clocks
// counted from a mind's landing — `power_online` at the landing plus
// `startup_ms`, a ballistic impact at the shot plus its flight, the next shot
// at the shot plus the cycle — are `M1B5_TWO_T1_EVENTS`' rows, asserted by
// `m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`)
// ---------------------------------------------------------------------------

/// **17b cut M4.4 — a weapon ready between the view and the landing is cited
/// at the landing** (16b cut M.2, 16c `T-02`; review F10 — the one-clock
/// rule as the mind has it: doc 17 §17's `landing` divergence and
/// `engage_orders`' doc).
///
/// The reflex planner and the resolver both read the tick's millisecond, so
/// a cannon that came ready between the tick and the end of its substep was
/// not fired on that tick and waited for the next one (the shot list the
/// reflex fixture pinned: the coaxial at the tick, the cannon at the tick
/// after). A mind has two clocks a substep apart by design: it reads the
/// keyframe at the end of the substep its think fell in (`view.t_ms`), and
/// its answer is resolved at the landing one substep later
/// (`ThinkJob::lands_ms`); so `engage_orders` cites the first armed weapon
/// whose `ready_in_ms` as of the view is at most the milliseconds still to
/// run to the landing — never a shot the resolver would refuse `Cycling`,
/// never one held a whole think for a gun that is ready when the shot is
/// resolved. The reflex's shot list is not readable on T1: after the
/// citation the slot owns the shot (17b cut M1b.2), so the second gun's
/// shot is its slot's at the gun's own `WeaponReady`, `by: "cond"`. The port
/// asserts the rule as declared, not a weaker one, in the reflex fixture's
/// own shape (16b's M2-R2: the gun FIRST in component order is the one
/// that is not ready, so a planner reading a later clock than the landing's
/// would cite it and be refused `Cycling`, and one reading the view's clock
/// alone would cite nothing): RED's think 2 is at 2,482 (`think_at`), reads
/// the view at 2,500 and lands at 2,732; the cannon, S1, hand-set ready at
/// 2,733 — one millisecond after the landing — is not cited and is fired by
/// its armed slot at its own `WeaponReady` at 2,733, `by: "cond"`; the
/// coaxial, S2, hand-set ready at 2,732 — after the view, AT the landing
/// (the inclusive edge) — IS the cited shot at 2,732, `by: "mind"` citing
/// S2, with no `Cycling` refusal on the mind row: never early, never
/// refused, and a clock even one millisecond past the landing's fails. The two
/// `WeaponReady` due events are scheduled by hand beside the hand-set
/// readiness, as the resolver schedules them at a shot. The hull sees ten
/// cells (as `tests/minds.rs`' (j) field does: a contact's range reaches a
/// think at a 50 m floor, inside a ten-cell band and outside a five-cell
/// one), the target presents every band and both guns are harmless
/// ([`t1_aa1_harmless`]): the fixture's constants, so the fight the rule is
/// read on exists and goes on.
#[test]
fn t1_a_weapon_ready_between_the_view_and_the_landing_is_cited_at_the_landing() {
    use tacsim::arena::intents::IntentKind;
    use tacsim::arena::perception::FrameNotice;
    use tacsim::arena::state::DueKind;

    let mut arena = arena_of(
        vec![
            seeing(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![t1_aa1_harmless(cannon()), t1_aa1_harmless(coaxial())],
                    Controller::Mind,
                ),
                SIGHT_CELLS,
            ),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes: Vec<u8> = Vec::new();

    // The fixture's premise, from the lattice: think 0 lands `Startup`, the
    // hull is online by think 1, and think 2 is the one read here — inside
    // its substep (never on the 250 ms grid), so the view's clock is strictly
    // after it and the landing strictly after the view.
    let substep = arena.substep_len_ms();
    let think = think_at(RED, 2);
    assert_ne!(
        think.rem_euclid(substep),
        0,
        "the fixture needs a think that is not a substep boundary"
    );
    let view_ms = think - think.rem_euclid(substep) + substep;
    let landing = think + substep;
    // The edge exactly: the coaxial ready AT the landing (the inclusive
    // edge, as the reflex fixture's coaxial was ready at its tick), the
    // cannon one millisecond after it — so any clock later than the
    // landing's, even by one millisecond, cites the cannon and is refused.
    let ready_before = landing;
    let ready_after = landing + 1;
    assert!(
        think < view_ms
            && view_ms < ready_before
            && ready_before <= landing
            && landing < ready_after,
        "the premise: think {think}, view {view_ms}, ready {ready_before}, landing {landing}, \
         ready {ready_after}"
    );

    // A pulse boundary two substeps before the view: online, patrolling,
    // holding nothing.
    let at = view_ms - 2 * substep;
    run_settled(&mut arena, at, &mut bytes);
    assert_eq!(arena.now_ms, at);
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Online));
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Patrol));
    assert_eq!(arena.unit(RED).and_then(|unit| unit.lock), None);
    // The hostile, classified, three cells east; the cannon (first in
    // component order) comes ready after the landing, the coaxial between
    // the view and the landing, each with the `WeaponReady` the resolver
    // would have scheduled for it.
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    if let Some(unit) = arena.unit_mut(RED) {
        unit.ready_at_ms.insert(SmolStr::new("cannon"), ready_after);
        unit.ready_at_ms
            .insert(SmolStr::new("coaxial"), ready_before);
    }
    let _ = arena.due.schedule(
        ready_after,
        DueKind::WeaponReady {
            unit: RED,
            weapon: SmolStr::new("cannon"),
        },
    );
    let _ = arena.due.schedule(
        ready_before,
        DueKind::WeaponReady {
            unit: RED,
            weapon: SmolStr::new("coaxial"),
        },
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let thought = stock.kept().len();
    run_settled(&mut arena, view_ms + 2 * substep, &mut bytes);

    // The think: read at the view's clock, it saw the coaxial ready inside
    // the milliseconds to its landing and the cannon not, and cited the
    // coaxial on S2 and nothing else — passing over the cannon first.
    let kept = stock.kept();
    let kept = kept.get(thought).expect("the think at 2,482 was kept");
    assert_eq!(kept.1, think, "the think's own millisecond");
    assert_eq!(
        kept.2.t_ms, view_ms,
        "the view's clock is the substep's end"
    );
    let ready_in = |id: &str| {
        kept.2
            .weapons
            .iter()
            .find(|weapon| weapon.id == id)
            .map(|weapon| weapon.ready_in_ms)
    };
    assert_eq!(ready_in("cannon"), Some(ready_after - view_ms));
    assert_eq!(ready_in("coaxial"), Some(ready_before - view_ms));
    let cited: Vec<(SmolStr, Option<u8>)> = kept
        .3
        .intents
        .iter()
        .filter_map(|kind| match kind {
            IntentKind::Fire { weapon, slot } => Some((weapon.clone(), *slot)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cited,
        vec![(SmolStr::new("coaxial"), Some(2))],
        "the think cited the gun ready at its landing, and not the one ready after it: \
         {:?}",
        kept.3.intents
    );
    assert_eq!(kept_memory(kept).state, MachineState::Engage);
    assert_eq!(machine_state_of(&arena, RED), Some(MachineState::Engage));

    // The shots: the cited coaxial at the landing, by the mind; the cannon at
    // its own `WeaponReady`, by its slot. Nothing early, nothing refused.
    let shots: Vec<(i64, SmolStr, Option<SmolStr>, Option<u8>)> = events(&mut arena, &mut bytes)
        .into_iter()
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::Fired {
                unit,
                weapon,
                by,
                slot,
                ..
            } if unit == RED => Some((event.t_ms, weapon, by, slot)),
            _ => None,
        })
        .skip(before)
        .collect();
    assert_eq!(
        shots,
        vec![
            (
                landing,
                SmolStr::new("coaxial"),
                Some(SmolStr::new("mind")),
                Some(2)
            ),
            (
                ready_after,
                SmolStr::new("cannon"),
                Some(SmolStr::new("cond")),
                Some(1)
            ),
        ],
        "the coaxial came ready at {ready_before}, between the view at {view_ms} and the \
         landing at {landing}: the landing fires it; the cannon came ready at {ready_after}, \
         after the landing: its slot fires it then"
    );
    let refused: Vec<FrameNotice> = arena
        .minds
        .get(&RED)
        .map(|row| {
            row.notices
                .iter()
                .filter(|notice| matches!(notice, FrameNotice::Refused { .. }))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    assert!(
        refused.is_empty(),
        "no intent of the landing was refused: {refused:?}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Y.2 — W8's read: the squads under a live order, per issuer
// ---------------------------------------------------------------------------

/// A ten-minute-or-`ttl_ms` `HOLD` for `squad`, as `issuer` gave it.
fn y2_hold(
    squad: tacsim::deliberation::SquadId,
    issuer: Option<ObjId>,
    ttl_ms: u32,
) -> tacsim::deliberation::Order {
    tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Hold,
        target: None,
        horizon_ms: ttl_ms,
        constraints: tacsim::deliberation::Constraints {
            hold_fire: false,
            max_range_band: None,
        },
        ttl_ms,
        issued_ms: 0,
        issuer,
        by: author(squad, issuer),
        spot: None,
    }
}

/// **16b cut Y.2** (doc 16 §2 W8): [`Arena::live_orders_by_issuer`] files each
/// live order's squad under the order's issuer, files an order with no issuer —
/// the LLM seam's — under the squad commander of the unit it steers, drops an
/// expired one, and answers empty for an arena with no orders at all.
///
/// Four red machines are two squads of pairs — pinned to `squad_size: 2`
/// through this test's own arena config (16b cut Z.1): at the shipped lance of
/// four they would be one squad, and what this test proves needs one issuer over
/// two squads beside a commander over one of them. The seated red tank is the
/// pilot who orders both, and is in no squad.
#[test]
fn y2_live_orders_by_issuer_groups_per_issuer_and_the_machines_under_their_commander() {
    let reds = [ObjId(20), ObjId(21), ObjId(22), ObjId(23)];
    let mut units: Vec<CustodyUnit> = reds
        .iter()
        .zip([1_u16, 3, 5, 7])
        .map(|(id, x)| {
            tank(
                *id,
                Team::Red,
                centre(x, 1),
                vec![cannon()],
                Controller::Reflex,
            )
        })
        .collect();
    units.push(tank(
        Y1_PILOT,
        Team::Red,
        centre(1, 4),
        Vec::new(),
        Controller::Pilot,
    ));
    units.push(tank(
        BLUE,
        Team::Blue,
        centre(14, 14),
        Vec::new(),
        Controller::Pilot,
    ));
    let pairs = ArenaConfig {
        squad_size: 2,
        ..arena_cfg(reflex_cfg())
    };
    let mut arena = arena_of(units, &[], &pairs, &[Y1_PILOT, BLUE]);
    assert!(
        arena.live_orders_by_issuer(0).is_empty(),
        "no order, no commander"
    );

    let squad_of = |arena: &Arena, unit: ObjId| {
        arena
            .squad_of(unit)
            .map(|roster| (roster.id, roster.commander))
            .expect("every red machine is in a squad")
    };
    let (first, _) = squad_of(&arena, reds[0]);
    let (second, second_commander) = squad_of(&arena, reds[2]);
    assert_ne!(first, second, "four machines are two squads");
    assert_eq!(squad_of(&arena, reds[1]).0, first);
    assert_eq!(squad_of(&arena, reds[3]).0, second);
    assert!(
        arena.squad_of(Y1_PILOT).is_none(),
        "a pilot's tank is in no squad"
    );

    let standing = |order, at| tacsim::deliberation::StandingOrder::accepted(order, at);
    // The pilot orders both squads: one squad counts once however many of its
    // units carry the order.
    arena.reflex.set_order(
        reds[0],
        standing(y2_hold(first, Some(Y1_PILOT), 600_000), 0),
    );
    arena.reflex.set_order(
        reds[2],
        standing(y2_hold(second, Some(Y1_PILOT), 600_000), 0),
    );
    // The machines' own order, on the second squad's wingman: filed under that
    // squad's commander.
    arena
        .reflex
        .set_order(reds[3], standing(y2_hold(second, None, 600_000), 0));
    // A third issuer's order that ran out at 30 s.
    arena.reflex.set_order(
        reds[1],
        standing(y2_hold(first, Some(ObjId(77)), 30_000), 0),
    );

    let live = arena.live_orders_by_issuer(100_000);
    let want: std::collections::BTreeMap<
        Option<ObjId>,
        std::collections::BTreeSet<tacsim::deliberation::SquadId>,
    > = [
        (Some(Y1_PILOT), [first, second].into_iter().collect()),
        (Some(second_commander), [second].into_iter().collect()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        live, want,
        "two squads under the pilot, one under the commander"
    );
    assert_eq!(
        second_commander, reds[2],
        "the commander is the lowest id of its pair"
    );
    // Before the third issuer's order ran out it was there too.
    assert_eq!(
        arena
            .live_orders_by_issuer(29_999)
            .get(&Some(ObjId(77)))
            .map(std::collections::BTreeSet::len),
        Some(1)
    );
    // And at the ten minutes every order has run out.
    assert!(arena.live_orders_by_issuer(600_000).is_empty());
}

// ---------------------------------------------------------------------------
// 16b cut Z.2 — the screen, the support and the release (16c N6, Q44)
// ---------------------------------------------------------------------------

/// Every line of `kind` in a replay's event list, decoded.
fn z2_lines(run: &Replay, kind: &str) -> Vec<(i64, tacsim::arena::ledger::EventKind)> {
    run.events
        .iter()
        .filter_map(|(t_ms, json, _)| {
            serde_json::from_str::<tacsim::arena::ledger::EventKind>(json)
                .ok()
                .map(|decoded| (*t_ms, decoded))
        })
        .filter(|(_, decoded)| decoded.as_str() == kind)
        .collect()
}

/// **The cross-tempo property over three replays** (16b §11: mandatory): one
/// event list, one hash sequence and one final field, from three different
/// numbers of pulses.
fn z2_one_log(runs: &[Replay], what: &str) {
    let first = runs.first().expect("three replays");
    assert!(
        first.hashes.len() >= 10,
        "{what}: {} substeps",
        first.hashes.len()
    );
    assert!(
        runs.iter()
            .map(|run| run.pulses)
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "{what}: three cadences, three different numbers of pulses"
    );
    for (run, cadence) in runs.iter().zip([5_000_i64, 1_000, 500]).skip(1) {
        assert_eq!(
            run.events, first.events,
            "{what}: the event list diverged at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "{what}: the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "{what}: the field ended differently at {cadence} ms"
        );
    }
}

/// One order for `squad`, ten minutes long, as the world writes one.
fn z2_order(
    squad: tacsim::deliberation::SquadId,
    objective: tacsim::deliberation::Objective,
    hold_fire: bool,
    issuer: Option<ObjId>,
) -> tacsim::deliberation::Order {
    tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints {
            hold_fire,
            max_range_band: None,
        },
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer,
        by: author(squad, issuer),
        spot: None,
    }
}

/// `kind` for each of `units`, on each unit's own `helm` epoch — the shape the
/// world's `send_order` gives an `ORDER` or a `RELEASE`.
fn z2_fan(
    arena: &Arena,
    units: &[ObjId],
    kind: &tacsim::arena::intents::IntentKind,
) -> Vec<tacsim::arena::intents::Intent> {
    units
        .iter()
        .zip(1_u64..)
        .map(|(unit, seq)| tacsim::arena::intents::Intent {
            unit: *unit,
            channel: Channel::Helm,
            epoch: arena.epochs.current(*unit, Channel::Helm),
            kind: kind.clone(),
            seq,
        })
        .collect()
}

/// The squad `unit` is in.
fn z2_squad(arena: &Arena, unit: ObjId) -> tacsim::deliberation::SquadId {
    arena
        .squad_of(unit)
        .map(|roster| roster.id)
        .expect("the machine is in a squad")
}

/// A unit's cell after every pulse of a replay.
type Z2Cells = Vec<Option<(u16, u16)>>;

/// A unit's cell after every pulse of a replay, with the pulse's closing arena
/// millisecond.
type Z2Samples = Vec<(i64, Option<(u16, u16)>)>;

/// The cell `unit` stands in.
fn z2_cell(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena
        .unit(unit)
        .and_then(|state| arena.definition.terrain.cell_of(&state.pose))
}

// ---- (a) the screen --------------------------------------------------------

/// Where the crossing tank is put, and when: from 18° (outside the 30° to 150°
/// arc of a screen facing east) through 45°, 90° and 135° (inside it) to 170°
/// (outside again), about six cells out, one move every five seconds of arena
/// time.
const Z2_CROSSING: [(i64, (u16, u16)); 5] = [
    (5_000, (10, 14)),
    (10_000, (12, 12)),
    (15_000, (14, 8)),
    (20_000, (12, 4)),
    (25_000, (9, 2)),
];

/// **A screen facing east.** The red machine at the middle of the window with a
/// ten-cell sight (so `release_cm` is ten cells); a blue tank six cells due
/// north of it, 30° outside the arc, that the script drives across the front;
/// and a second blue tank parked five cells due west — at 270°, inside
/// `release_cm` and nearer than the first — that the screen must never turn
/// for. Both blue tanks are unarmed and nobody flies them. **17b cut M4.4**:
/// the red machine is a `Controller::Mind` on Standard Doctrine.
fn z2_screen_field() -> Arena {
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Mind,
                ),
                10,
            ),
            // 17b cut M4.4: both blue tanks present every band ([`presenting`]):
            // a mind's armed slot fires at odds and a `UnitProfile::default()`
            // target presents none, where the reflex machine fired regardless —
            // so the one the screen never turns for is one it COULD have shot.
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(8, 14),
                Vec::new(),
                Controller::Pilot,
            )),
            presenting(tank(
                BLUE2,
                Team::Blue,
                centre(3, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    plant_contact(&mut arena, RED, BLUE2, Team::Blue, 0);
    arena
}

/// One replay of the screen: `SCREEN 90` at beat 0, the crossing driven by the
/// script, the red machine's cell sampled after every pulse — and the arena's
/// substep length, for the mind's "stops once outside" window (17b cut M4.4).
fn z2_screen(cadence_ms: i64) -> (Replay, Z2Cells, i64) {
    let arena = z2_screen_field();
    let substep_ms = arena.substep_len_ms();
    let squad = z2_squad(&arena, RED);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Screen {
            facing_mdeg: 90_000,
        },
        false,
        None,
    )));
    let mut cells: Z2Cells = Vec::new();
    let run = replay_with(
        cadence_ms,
        40_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &[RED], &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            if let Some((_, cell)) = Z2_CROSSING.iter().find(|(at, _)| *at == arena.now_ms) {
                if let Some(state) = arena.unit_mut(BLUE) {
                    state.pose = centre(cell.0, cell.1);
                }
                let at_ms = arena.now_ms;
                plant_contact(arena, RED, BLUE, Team::Blue, at_ms);
            }
            cells.push(z2_cell(arena, RED));
        },
    );
    (run, cells, substep_ms)
}

/// **16b cut Z.2 (a), 16c N6 (16b §11: mandatory), on the unit mind** (17b
/// cut M4.4): a screen fires on what enters its arc and never moves.
///
/// Every claim of the reflex fixture is kept. At 5,000, 1,000 and 500 ms one
/// event list, one hash sequence (which folds the facing — the `FrameOrder`'s,
/// hash item 3b) and one final field, stepped with `step_settled`. The
/// machine's cell is the same after every pulse of every replay. It locks and
/// fires on the crossing tank only once the script has put it inside the arc
/// (the move at 10 s, to 45°), and on nothing before — the first lock after
/// the move lands at most a think period and a substep later, inside the
/// (10,000, 15,000) window; and the tank parked at 270° — inside `release_cm`,
/// held and classified the whole time, and nearer than the crossing tank
/// (though both read the view's 50 m floor, so the lure is the arc's alone
/// on T1) — is never locked at all. What moved: the red machine is a `Controller::Mind`
/// on Standard Doctrine, whose `SCREEN 90` reaches it through
/// `IntentKind::Order` (`rekey_order` → `MindObjective::Screen { facing_mdeg
/// }`), and whose Screen arm parks it (`Goal(Hold)`), faces it onto the
/// order's bearing and stands over `(facing, SCREEN_HALF_ARC_MDEG)` with
/// `engage_orders` — so only a row inside the arc is ever locked or fired on.
/// The one re-derived constant is the "stops once outside" window below: a
/// mind thinks every `STOCK_THINK_MS` and its answer lands a substep later,
/// where the reflex ticked every `decide_ms` and acted at its tick — so the
/// window ends at the landing of the first think after the move.
#[test]
fn t1_a_screen_fires_on_what_enters_its_arc_and_never_moves() {
    let replays: Vec<(Replay, Z2Cells, i64)> =
        [5_000_i64, 1_000, 500].into_iter().map(z2_screen).collect();
    for (_, cells, _) in &replays {
        assert!(!cells.is_empty());
        assert!(
            cells.iter().all(|cell| *cell == Some((8, 8))),
            "the screen never moves: {cells:?}"
        );
    }
    let substep_ms = replays[0].2;
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _)| run).collect();
    z2_one_log(&runs, "the screen");
    let first = &runs[0];
    let ordered = z2_lines(first, "ordered");
    assert_eq!(ordered.len(), 1, "one order: {ordered:?}");
    assert!(
        ordered.iter().all(|(t_ms, line)| *t_ms == 0
            && matches!(
                line,
                tacsim::arena::ledger::EventKind::Ordered { objective, .. } if objective == "screen"
            )),
        "one `screen` order at beat 0: {ordered:?}"
    );
    let locked = z2_lines(first, "locked");
    assert!(!locked.is_empty(), "the screen locked something");
    for (t_ms, line) in &locked {
        let tacsim::arena::ledger::EventKind::Locked { unit, target, .. } = line else {
            continue;
        };
        assert_eq!(*unit, RED);
        assert_eq!(*target, BLUE, "only the crossing tank is ever locked");
        assert!(*t_ms > 10_000, "not before it was inside the arc: {t_ms}");
    }
    let fired: Vec<i64> = z2_lines(first, "fired")
        .into_iter()
        .filter_map(|(t_ms, line)| match line {
            tacsim::arena::ledger::EventKind::Fired { unit, target, .. } => {
                assert_eq!(unit, RED);
                assert_eq!(target, BLUE, "the screen fires on the crossing tank alone");
                Some(t_ms)
            }
            _ => None,
        })
        .collect();
    assert!(
        fired
            .first()
            .is_some_and(|t_ms| *t_ms > 10_000 && *t_ms < 15_000),
        "the screen fires once the tank is inside its arc: {fired:?}"
    );
    // **16b cut Z.2's review**: and it stops once the tank has left the arc. The
    // move at 25 s puts it at 170°; the machine's first think after that reads
    // the new plot, so no round is fired from a think after it. It does not
    // keep the last target it had, and it does not turn for the tank at 270°.
    // **17b cut M4.4**: the reflex's `25,000 + decide_ms` on the mind's
    // lattice — the first think after the move (`think_at(RED, 25)`, at
    // 25,482) reads the tank outside the arc and its answer, disarming the
    // slot, lands a substep on; no round is fired from then on.
    let outside_by = think_at(RED, 25) + substep_ms;
    assert!(
        outside_by > 25_000
            && outside_by <= 25_000 + tacsim::arena::mind::STOCK_THINK_MS + substep_ms,
        "the first think after the move lands inside a think period and a substep: {outside_by}"
    );
    assert!(
        fired.iter().all(|t_ms| *t_ms < outside_by),
        "the screen fired on a tank outside its arc: {fired:?}"
    );
    // **The facing the acknowledgement promises** ("faces 090°"): with nothing
    // inside the arc from 25 s to the end, the machine is turned back onto the
    // order's bearing and holds it.
    assert_eq!(
        first
            .finals
            .iter()
            .find(|row| row.0 == RED)
            .map(|row| row.3),
        Some(90_000),
        "the screen faces its bearing when it has nothing to shoot"
    );
}

// ---- (b) the support -------------------------------------------------------

/// The supporter.
const Z2_SUPPORTER: ObjId = RED2;
/// Its charge.
const Z2_CHARGE: ObjId = RED;
/// The hostile the charge designates: farther from the supporter, and showing
/// it its front.
const Z2_DESIGNATED: ObjId = BLUE;
/// The hostile the supporter's own score prefers: nearer, and showing its rear.
const Z2_BETTER: ObjId = BLUE2;

/// **The designation field.** Row 8: the supporter at column 6 with an
/// eight-cell sight, its charge two cells east at column 8, and the designated
/// tank at column 12 facing west — six cells from the supporter, front on. The
/// better target sits at (3, 10) facing west, 3.6 cells from the supporter and
/// rear on, and further than the charge's five-cell sight from the charge, so
/// the charge never holds it. The supporter holds all three, classified; the
/// charge holds the designated tank.
///
/// `machine` puts the charge under the controller (its row then targets the one
/// hostile it holds); otherwise it is a pilot's tank nobody is flying,
/// **locked** on the designated tank.
fn z2_designation_field(machine: bool) -> Arena {
    let mut designated = tank(
        Z2_DESIGNATED,
        Team::Blue,
        centre(12, 8),
        Vec::new(),
        Controller::Pilot,
    );
    designated.pose.heading_mdeg = 270_000;
    let mut better = tank(
        Z2_BETTER,
        Team::Blue,
        centre(3, 10),
        Vec::new(),
        Controller::Pilot,
    );
    better.pose.heading_mdeg = 270_000;
    let mut arena = arena_of(
        vec![
            tank(
                Z2_CHARGE,
                Team::Red,
                centre(8, 8),
                vec![cannon()],
                if machine {
                    Controller::Reflex
                } else {
                    Controller::Pilot
                },
            ),
            with_sight(
                tank(
                    Z2_SUPPORTER,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                8,
            ),
            designated,
            better,
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_CHARGE, Team::Red, 0);
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_DESIGNATED, Team::Blue, 0);
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_BETTER, Team::Blue, 0);
    plant_contact(&mut arena, Z2_CHARGE, Z2_DESIGNATED, Team::Blue, 0);
    if !machine {
        let lock = arena.contacts.id_of(Z2_CHARGE, Z2_DESIGNATED);
        if let Some(state) = arena.unit_mut(Z2_CHARGE) {
            state.lock = lock;
        }
    }
    arena
}

/// What one designation replay read at 5 s: the supporter's target, the
/// charge's own target and state (a machine charge's), and the charge's plot
/// range to the designated tank.
type Z2Read = (Option<ObjId>, Option<ObjId>, Option<MachineState>, i64);

/// **A reflex row's target** (17b cut M4.4): what `target_of` read before the
/// ports re-pointed it onto a mind's memory. The designation half stays on
/// the reflex controller (a STOP) and keeps reading its own rows.
fn z2_reflex_target_of(arena: &Arena, unit: ObjId) -> Option<ObjId> {
    arena
        .reflex
        .unit(unit)
        .and_then(|row| row.target)
        .map(|(_, target)| target)
}

/// One replay of the designation field under `objective`, fanned to the
/// supporter's squad at beat 0 with the fire held, read at 5 s.
fn z2_designation(
    cadence_ms: i64,
    machine: bool,
    objective: tacsim::deliberation::Objective,
) -> (Replay, Z2Read) {
    let arena = z2_designation_field(machine);
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let units: Vec<ObjId> = arena
        .squad_of(Z2_SUPPORTER)
        .map(|roster| roster.units.clone())
        .unwrap_or_default();
    let issuer = if machine { None } else { Some(Z2_CHARGE) };
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad, objective, true, issuer,
    )));
    let mut read: Option<Z2Read> = None;
    let run = replay_with(
        cadence_ms,
        10_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &units, &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            if arena.now_ms == 5_000 {
                read = Some((
                    z2_reflex_target_of(arena, Z2_SUPPORTER),
                    z2_reflex_target_of(arena, Z2_CHARGE),
                    state_of(arena, Z2_CHARGE),
                    plot_range(arena, Z2_CHARGE, Z2_DESIGNATED),
                ));
            }
        },
    );
    (run, read.expect("a pulse ended at 5 s"))
}

/// **The station field**, on a window forty-eight cells wide: the supporter at
/// (10, 8) with a twenty-one-cell sight, its charge — a pilot's tank nobody is
/// flying — two cells west of it, and one unarmed blue tank parked in the far
/// corner, out of the supporter's sight wherever the replay takes it. The
/// supporter on the reflex controller — what the immobile-support fixture
/// (`z2_a_supporter_that_cannot_close_fights_where_it_stands_past_its_leash`,
/// a STOP of 17b cut M4.4) still fields.
fn z2_station_field() -> Arena {
    z2_station_field_on(&[], Controller::Reflex)
}

/// **The hostile a machine under `controller` can shoot at** (17b cut M4.4):
/// presenting every band ([`presenting`]) for a mind — its armed slot fires
/// at odds, and a `UnitProfile::default()` target presents none, where the
/// reflex machine fired regardless — and as authored for the reflex, so the
/// fixtures left on it (this cut's STOPs) run as they did.
fn z2_hostile_for(controller: Controller, unit: CustodyUnit) -> CustodyUnit {
    if controller == Controller::Mind {
        presenting(unit)
    } else {
        unit
    }
}

/// [`z2_station_field`] with `paint`ed cells over its uniform ground (16b cut
/// Z.2's review: the unreachable station) and the supporter under
/// `controller` — **17b cut M4.4**: a `Controller::Mind` on Standard Doctrine
/// for the station, the blind stand and the moat.
fn z2_station_field_on(paint: &[(u16, u16, TerrainCell)], controller: Controller) -> Arena {
    const WIDE: u16 = 48;
    let mut cells = vec![ground(); usize::from(WIDE) * usize::from(SIDE)];
    for (x, y, cell) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(WIDE) + usize::from(*x)) {
            *slot = *cell;
        }
    }
    let definition = ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex.wide"),
        terrain: ArenaTerrain::new(WIDE, SIDE, CELL_CM, cells).expect("the wide window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: WIDE - 2,
                y0: 0,
                x1: WIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: graph(),
        modes: Vec::new(),
    };
    let mut arena = arena_on(
        definition,
        vec![
            tank(
                Z2_CHARGE,
                Team::Red,
                centre(8, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            with_sight(
                tank(
                    Z2_SUPPORTER,
                    Team::Red,
                    centre(10, 8),
                    vec![cannon()],
                    controller,
                ),
                21,
            ),
            z2_hostile_for(
                controller,
                tank(
                    BLUE,
                    Team::Blue,
                    centre(47, 15),
                    Vec::new(),
                    Controller::Pilot,
                ),
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_CHARGE, Team::Red, 0);
    arena
}

/// What the station replay read at 30 s: the supporter's cell, its range to
/// its own plot of the charge, and the speed it was ordered.
type Z2Station = (Option<(u16, u16)>, Option<u64>, Option<i32>);

/// **What became of the order on the mind** (17b cut M4.4): whether any pulse
/// ended with the supporter's memory carrying `ignore_until_ms` (the mind's
/// one way of giving an order up), and whether the arena still holds the
/// order at the end.
type Z2Order = (bool, bool);

/// One replay of the station field: `SUPPORT` the charge at beat 0; at 5 s the
/// charge is moved twenty cells east; at 30 s the station is read and the
/// charge is moved out of the supporter's sight, and nothing refreshes the
/// supporter's hold on it, which expires; at 35 s the blue tank is put four
/// cells north of wherever the supporter stands. **17b cut M4.4**: the
/// supporter is a mind, its thinks kept, and what became of its order is
/// read after every pulse ([`Z2Order`]).
fn z2_station(cadence_ms: i64) -> (Replay, Z2Station, Z2Samples, Z2Order, Vec<Kept>) {
    let mut arena = z2_station_field_on(&[], Controller::Mind);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        Some(Z2_CHARGE),
    )));
    let mut read: Option<Z2Station> = None;
    let mut cells: Z2Samples = Vec::new();
    let mut gave_up = false;
    let mut held_at_end = false;
    let run = replay_with(
        cadence_ms,
        45_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &[Z2_SUPPORTER], &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            match arena.now_ms {
                5_000 => {
                    if let Some(state) = arena.unit_mut(Z2_CHARGE) {
                        state.pose = centre(28, 8);
                    }
                }
                30_000 => {
                    let plot = hold_on(arena, Z2_SUPPORTER, Z2_CHARGE).map(|held| held.seen_at);
                    read = Some((
                        z2_cell(arena, Z2_SUPPORTER),
                        plot.zip(arena.unit(Z2_SUPPORTER))
                            .map(|(plot, state)| plot.range_from(&state.pose)),
                        arena
                            .unit(Z2_SUPPORTER)
                            .map(|state| state.desired_speed_cms),
                    ));
                    if let Some(state) = arena.unit_mut(Z2_CHARGE) {
                        state.pose = centre(0, 8);
                    }
                }
                35_000 => {
                    if let Some((x, y)) = z2_cell(arena, Z2_SUPPORTER) {
                        if let Some(state) = arena.unit_mut(BLUE) {
                            state.pose = centre(x, y + 4);
                        }
                        let at_ms = arena.now_ms;
                        plant_contact(arena, Z2_SUPPORTER, BLUE, Team::Blue, at_ms);
                    }
                }
                _ => {}
            }
            cells.push((arena.now_ms, z2_cell(arena, Z2_SUPPORTER)));
            gave_up |= stock_memory_of(arena, Z2_SUPPORTER)
                .ignore_until_ms
                .is_some();
            held_at_end = arena
                .minds
                .get(&Z2_SUPPORTER)
                .is_some_and(|row| row.order.is_some());
        },
    );
    (
        run,
        read.expect("a pulse ended at 30 s"),
        cells,
        (gave_up, held_at_end),
        stock.kept(),
    )
}

/// **16b cut Z.2 (b) part 1, 16c N6 (16b §11: mandatory)**: a support prefers
/// what its charge is fighting; a unit whose charge is itself fights on
/// doctrine.
///
/// Every replay below is one event list, one hash sequence (which now folds the
/// charge) and one final field at 5,000, 1,000 and 500 ms.
///
/// **The designation, for a pilot charge and for a machine charge.** The
/// supporter holds two classified hostiles, and its own score prefers the
/// nearer one showing its rear (the control replay under `ENGAGE` proves
/// it). Under `SUPPORT` its row targets the farther, front-on tank its charge
/// is fighting: the tank a pilot's lock names, read on the pilot's own store,
/// and the tank a machine's row targets, read off the memory the tick holds.
/// The machine charge — whose `of` is itself — fights on doctrine: it takes
/// that tank and closes on it as `ENGAGE` would.
///
/// **17b cut M4.4 — a STOP, left on the reflex controller** (the `support`
/// divergence): a charge's designation is not on a mind's view — `hint()`
/// answers `None` for a `Support`, so a T1 supporter takes its own score's
/// rear-on tank, the control's answer — and `supporter == Some(Z2_DESIGNATED)`
/// is unreachable by design for the pilot charge and the machine charge
/// alike. Parts 2 and 3 of the fixture (the station and the blind stand) are
/// ported as [`t1_a_support_keeps_station_and_stands_blind`]. The owner's
/// call: delete this half against the twin's `support` counter
/// (`tests/twin.rs` `line_divergences`) and `stock_mind.rs`'
/// `m1a6_a_supporter_stands_blind_rejoins_off_the_leash_and_keeps_station`
/// — the machine charge's `of == self` half against `tests/minds.rs`'
/// `l2_a_self_charged_support_fights_on_doctrine` (held as `Engage`, it
/// closes into the band and fires) — or put the charge's target on the view
/// (a keyframe field: M5).
#[test]
fn z2_a_support_keeps_station_and_prefers_its_charges_target() {
    use tacsim::deliberation::Objective;
    let support = Objective::Support { of: Z2_CHARGE };

    // 1. The control: on its own score the supporter takes the rear-on tank.
    let (_, (own_choice, _, _, _)) =
        z2_designation(1_000, false, Objective::Engage { target_hint: None });
    assert_eq!(
        own_choice,
        Some(Z2_BETTER),
        "the supporter's own score prefers the nearer, rear-on tank"
    );
    for machine in [false, true] {
        let what = if machine {
            "a machine charge"
        } else {
            "a pilot charge"
        };
        let replays: Vec<(Replay, Z2Read)> = [5_000_i64, 1_000, 500]
            .into_iter()
            .map(|cadence| z2_designation(cadence, machine, support))
            .collect();
        let (supporter, charge, charge_state, charge_range) = replays[0].1;
        assert!(
            replays.iter().all(|(_, read)| *read == replays[0].1),
            "{what}: the reading diverged"
        );
        let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
        z2_one_log(&runs, what);
        assert_eq!(
            supporter,
            Some(Z2_DESIGNATED),
            "{what}: the supporter fights what its charge is fighting"
        );
        if machine {
            // The charge's own `of` is itself: doctrine, `ENGAGE`'s semantics.
            assert_eq!(charge, Some(Z2_DESIGNATED), "the charge fights on doctrine");
            assert!(
                matches!(
                    charge_state,
                    Some(MachineState::Approach | MachineState::Engage)
                ),
                "the charge is in the fight: {charge_state:?}"
            );
            assert!(
                charge_range < 4 * i64::from(CELL_CM),
                "the charge closed on its target as ENGAGE would: {charge_range} cm"
            );
        }
    }
}

/// **16b cut Z.2 (b) parts 2 and 3, 16c N6 (16b §11: mandatory), on the unit
/// mind** (17b cut M4.4): a support keeps station on its charge and fights
/// where it stands when it cannot see it.
///
/// Every claim of the reflex fixture's station and blind stand is kept; what
/// moved is the supporter, a `Controller::Mind` on Standard Doctrine, whose
/// `SUPPORT` reaches it through `IntentKind::Order` (`rekey_order` →
/// `MindObjective::Support { of }`, the charge's own classified row), and the
/// hostile, which presents every band ([`z2_hostile_for`]). Every replay is
/// one event list, one hash sequence (which folds the charge) and one final
/// field at 5,000, 1,000 and 500 ms under `step_settled`.
///
/// 2. **The station.** The charge is moved twenty cells: the supporter, past
///    its leash (`SUPPORT_LEASH_CELLS` to its own plot of the charge — the
///    leash's arm (b), `stand_off_of(plot, SUPPORT_STATION_CELLS)`), re-plans
///    toward it and stops inside `SUPPORT_STATION_CELLS` of its own plot of
///    the charge (the station park: `Goal(Hold)`), ordered to a standstill.
///    **One constant re-derived against the view** (the `range_round`
///    divergence): a mind measures the station on its view, where a plot's
///    range is published rounded to `CONTACT_RANGE_STEP_M` (50 m — exactly
///    the station's five cells) and held at the figure last published while
///    the plot stays within five eighths of a step of it (`held_range_m`),
///    so the reflex's "inside 5,000 cm of the plot" is asserted where the
///    mind reads it: the think that parks (`Goal(Hold)`) reads the charge
///    inside the station, and the think before it read it past the station
///    (the 100 m figure) and was still closing (a cell goal held in its
///    memory, not a park). The true range
///    it parks at is bounded by what the view can publish on the way in: a
///    hull closing at most 1,000 cm a think on a charge that stands cannot
///    skip the 100 m figure, which is held down to 6,875 cm (`held_range_m`:
///    five eighths of a step under 100 m), so the park is inside the station
///    plus three eighths of a step. The true 5,000 cm is the `range_round`
///    divergence, unreachable at any fixture scale: the station's five cells
///    ARE the view's floor.
/// 3. **The blind stand.** The charge is moved out of the supporter's sight
///    and its hold expires: the leash reads a charge it does not hold
///    (`Some(None)`) and the supporter stands — its cell never changes again,
///    it stays in `Patrol`, and when a hostile is put inside `release_cm` it
///    fires on it from where it stands. The order does not end: a charge it
///    cannot see is not a charge it has lost — no `order_expired` line, the
///    arena still holding the order, and the mind's one way of giving an
///    order up (`ignore_until_ms`) never written.
///
/// Part 1 of the reflex fixture (the designation) stays on the reflex as
/// [`z2_a_support_keeps_station_and_prefers_its_charges_target`], a STOP.
#[test]
fn t1_a_support_keeps_station_and_stands_blind() {
    use tacsim::arena::doctrine::{SUPPORT_LEASH_CELLS, SUPPORT_STATION_CELLS};
    let replays: Vec<(Replay, Z2Station, Z2Samples, Z2Order, Vec<Kept>)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_station)
        .collect();
    let (cell, range, speed) = replays[0].1;
    assert!(
        replays
            .iter()
            .all(|(_, read, _, _, _)| *read == replays[0].1),
        "the station reading diverged"
    );
    let kept = replays[0].4.clone();
    for (_, _, cells, (gave_up, held_at_end), thinks) in &replays {
        assert_eq!(*thinks, kept, "the think sequence diverged");
        let stood: Vec<Option<(u16, u16)>> = cells
            .iter()
            .filter(|(at, _)| *at >= 30_000)
            .map(|(_, cell)| *cell)
            .collect();
        assert!(!stood.is_empty());
        assert!(
            stood.iter().all(|at| *at == cell),
            "from the station on, the supporter never moves: {stood:?}"
        );
        assert!(!*gave_up, "the mind never gave the order up");
        assert!(*held_at_end, "and the arena still holds it");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _, _, _)| run).collect();
    z2_one_log(&runs, "the station");
    assert!(
        cell.is_some_and(|(x, _)| x >= 20),
        "the supporter re-planned toward its charge, twenty cells east: {cell:?}"
    );
    const {
        assert!(
            SUPPORT_LEASH_CELLS > SUPPORT_STATION_CELLS,
            "the station is inside the leash"
        )
    };
    // The station, where the mind measures it (`row_range_cm`, the station
    // arm's own ruler): the think that parks reads the charge inside
    // `SUPPORT_STATION_CELLS` on its view, and the think before it read the
    // charge past the station and was still closing on it.
    let station_cm = u64::from(SUPPORT_STATION_CELLS) * u64::from(CELL_CM);
    let charge = draw_contact_id(SEED, Z2_SUPPORTER, Z2_CHARGE);
    let read_of = |think: &Kept| {
        think
            .2
            .contacts
            .iter()
            .find(|row| row.id == charge)
            .map(tacsim::arena::stock_mind::row_range_cm)
    };
    let hold = tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Hold);
    let supporter: Vec<&Kept> = kept
        .iter()
        .filter(|think| think.0 == Z2_SUPPORTER && think.1 > 5_000 && think.1 < 30_000)
        .collect();
    let park_at = supporter
        .iter()
        .position(|think| think.3.intents.contains(&hold))
        .expect("the supporter parked before the station read");
    let parked = supporter[park_at];
    assert!(
        read_of(parked).is_some_and(|cm| cm <= station_cm),
        "its view holds the charge inside the station: {:?} at {} ms",
        read_of(parked),
        parked.1
    );
    let closing = park_at
        .checked_sub(1)
        .and_then(|at| supporter.get(at))
        .expect("a think closing on the charge before the park");
    assert!(
        read_of(closing).is_some_and(|cm| cm > station_cm),
        "the think before the park read the charge past the station: {:?} at {} ms",
        read_of(closing),
        closing.1
    );
    assert!(
        kept_memory(closing).goal.is_some(),
        "and was still closing on a cell: goal {:?}",
        kept_memory(closing).goal
    );
    // And where the plot truly is: no further out than the bottom of the 100 m
    // figure the view held on the way in (five eighths of a step under 100 m:
    // the station plus three eighths).
    let tolerance_cm = tacsim::arena::perception::CONTACT_RANGE_STEP_M * 100 * 3 / 8;
    assert!(
        range.is_some_and(|cm| cm <= station_cm + tolerance_cm),
        "it stopped inside the station of its plot of the charge, as a view can say \
         it: {range:?}"
    );
    assert_eq!(speed, Some(0), "ordered to a standstill at the station");
    let first = &runs[0];
    let fired: Vec<i64> = z2_lines(first, "fired")
        .into_iter()
        .filter_map(|(t_ms, line)| match line {
            tacsim::arena::ledger::EventKind::Fired { unit, target, .. } => {
                assert_eq!(unit, Z2_SUPPORTER);
                assert_eq!(target, BLUE);
                Some(t_ms)
            }
            _ => None,
        })
        .collect();
    assert!(
        fired.first().is_some_and(|t_ms| *t_ms > 35_000),
        "the blind supporter fired from its stand, and only then: {fired:?}"
    );
    assert_eq!(
        first
            .finals
            .iter()
            .find(|row| row.0 == Z2_SUPPORTER)
            .map(|row| row.5),
        Some(MachineState::Patrol),
        "it fired from a stand, never leaving `Patrol`"
    );
    assert!(
        z2_lines(first, "order_expired").is_empty(),
        "a charge the supporter cannot see is not a charge it has lost"
    );
}

/// One replay of the station field with a supporter that **cannot close** — a
/// hull authored with no speed, which still turns in place (16b cut AA.1's
/// shape): `SUPPORT` the charge at beat 0; at 5 s the charge is moved twenty
/// cells east, past the leash and still inside the supporter's sight; at 10 s
/// the blue tank is put four cells north of the supporter, with a contact.
fn z2_immobile_support(cadence_ms: i64) -> (Replay, Z2Samples, bool) {
    let mut arena = z2_station_field();
    if let Some(state) = arena.unit_mut(Z2_SUPPORTER) {
        state.mobility.max_speed_cms = 0;
    }
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        Some(Z2_CHARGE),
    )));
    let mut cells: Z2Samples = Vec::new();
    let mut ordered_at_end = false;
    let run = replay_with(
        cadence_ms,
        30_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &[Z2_SUPPORTER], &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            match arena.now_ms {
                5_000 => {
                    if let Some(state) = arena.unit_mut(Z2_CHARGE) {
                        state.pose = centre(30, 8);
                    }
                }
                10_000 => {
                    if let Some(state) = arena.unit_mut(BLUE) {
                        state.pose = centre(10, 12);
                    }
                    let at_ms = arena.now_ms;
                    plant_contact(arena, Z2_SUPPORTER, BLUE, Team::Blue, at_ms);
                }
                _ => {}
            }
            cells.push((arena.now_ms, z2_cell(arena, Z2_SUPPORTER)));
            ordered_at_end = arena.reflex.order(Z2_SUPPORTER).is_some();
        },
    );
    (run, cells, ordered_at_end)
}

/// **16b cut Z.2's review (16c N6; the `B-76` class, 16b §11: mandatory)**: a
/// supporter past its leash that **cannot close** stands and fights where it
/// is.
///
/// Its charge is moved twenty cells off, where the supporter still sees it: a
/// mobile supporter would rejoin (and fire nothing while it drove), but this
/// one never can, so the rejoining leg would have held it silent for the whole
/// order while a hostile shot it. It takes the blind supporter's stand instead:
/// its cell never changes, it stays in `Patrol`, it fires on the tank put four
/// cells from it, and the order stands — at 5,000, 1,000 and 500 ms, one event
/// list, one hash sequence and one final field.
#[test]
fn z2_a_supporter_that_cannot_close_fights_where_it_stands_past_its_leash() {
    let replays: Vec<(Replay, Z2Samples, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_immobile_support)
        .collect();
    for (_, cells, ordered) in &replays {
        assert!(!cells.is_empty());
        assert!(
            cells.iter().all(|(_, cell)| *cell == Some((10, 8))),
            "a hull with no speed never moves: {cells:?}"
        );
        assert!(*ordered, "the order stands: a stand is not an end");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _)| run).collect();
    z2_one_log(&runs, "the immobile supporter");
    let first = &runs[0];
    let fired: Vec<i64> = z2_lines(first, "fired")
        .into_iter()
        .filter_map(|(t_ms, line)| match line {
            tacsim::arena::ledger::EventKind::Fired { unit, target, .. } => {
                assert_eq!(unit, Z2_SUPPORTER);
                assert_eq!(target, BLUE);
                Some(t_ms)
            }
            _ => None,
        })
        .collect();
    assert!(
        fired.first().is_some_and(|t_ms| *t_ms > 10_000),
        "past its leash and unable to close, the supporter fired from where it \
         stands: {fired:?}"
    );
    assert_eq!(
        first
            .finals
            .iter()
            .find(|row| row.0 == Z2_SUPPORTER)
            .map(|row| row.5),
        Some(MachineState::Patrol),
        "it fired from a stand, never leaving `Patrol`"
    );
    assert!(
        z2_lines(first, "order_expired").is_empty(),
        "a supporter that stands has not lost its charge"
    );
}

/// Open water, which no hull crosses and every sensor sees over.
fn z2_water() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Water,
        elev_cm: 0,
        move_cost: 0,
    }
}

/// The cell a think's answer drove for, if it named one (17b cut M4.4).
fn z2_goal_cell(intents: &[tacsim::arena::intents::IntentKind]) -> Option<(u16, u16)> {
    intents.iter().find_map(|intent| match intent {
        tacsim::arena::intents::IntentKind::Goal(tacsim::arena::intents::MotorGoal::Cell {
            x,
            y,
        }) => Some((*x, *y)),
        _ => None,
    })
}

/// One replay of the station field with a moat of open water three cells wide
/// from edge to edge at columns 19 to 21: `SUPPORT` the charge at beat 0; at
/// 5 s the charge is moved twenty cells east, across the moat, where the
/// supporter still sees it and no route reaches its station. **17b cut
/// M4.4**: the supporter is a mind, its thinks kept. Answers the replay, the
/// kept thinks, the motor's failed searches and whether the arena still holds
/// the supporter's order at the end.
fn z2_moat_support(cadence_ms: i64) -> (Replay, Vec<Kept>, u32, bool) {
    let moat: Vec<(u16, u16, TerrainCell)> = (19..=21_u16)
        .flat_map(|x| (0..SIDE).map(move |y| (x, y, z2_water())))
        .collect();
    let mut arena = z2_station_field_on(&moat, Controller::Mind);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        Some(Z2_CHARGE),
    )));
    let mut ordered_at_end = false;
    let (run, counters) = replay_counted(
        cadence_ms,
        20_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &[Z2_SUPPORTER], &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            if arena.now_ms == 5_000
                && let Some(state) = arena.unit_mut(Z2_CHARGE)
            {
                state.pose = centre(30, 8);
            }
            ordered_at_end = arena
                .minds
                .get(&Z2_SUPPORTER)
                .is_some_and(|row| row.order.is_some());
        },
    );
    (run, stock.kept(), counters.path_failures, ordered_at_end)
}

/// **16b cut Z.2's review (16c N6; `B-41`'s lesson, 16b §11: mandatory), on
/// the unit mind** (17b cut M4.4): a supporter whose station no route reaches
/// stops obeying its order.
///
/// Past its leash a supporter rejoins, and fires nothing while it does; a
/// station across a moat it cannot cross would hold it silent for the order's
/// whole life. Every claim of the reflex fixture is kept but the line, which
/// is re-pinned to the T1 shape — the `order_end` divergence (doc 17 §17;
/// row 54's ruling): a mind cannot end an order, only stop obeying it. The
/// supporter's first think after the charge crossed (at most a think period
/// after 5 s) asks the motor for the station across the moat; the search
/// fails at that answer's landing (a substep on — `Fallback::Notice`: the
/// goal dropped, ONE `path_failures`), and the next think reads the motor's
/// `drive_no_route` and gives the order up for this hull — a `Support` in
/// `Patrol` under `no_route` writes `ignore_until_ms` at the order's deadline
/// — the same lattice millisecond at every cadence. From then on `obeyed`
/// answers nothing: the station is never asked for again, the key is kept
/// while the order stands on the view, and the supporter walks the graph on
/// doctrine. What the reflex journalled, the arena never does for a mind: NO
/// `order_expired` line (a mind's order expires only at its `expires_ms`),
/// and `arena.minds[&supporter].order` still standing at the end. Three
/// cadences under `step_settled`: one event list, one hash sequence, one
/// final field, one think sequence.
#[test]
fn t1_a_supporter_whose_station_no_route_reaches_stops_obeying_its_order() {
    use tacsim::arena::mind::STOCK_THINK_MS;
    use tacsim::arena::perception::{DriveOutcome, FrameNotice};
    let replays: Vec<(Replay, Vec<Kept>, u32, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_moat_support)
        .collect();
    let kept = replays[0].1.clone();
    for (_, thinks, path_failures, ordered) in &replays {
        assert!(
            *ordered,
            "the arena still holds the order: a mind cannot end one"
        );
        assert_eq!(*path_failures, 1, "one failed search, for the station");
        assert_eq!(*thinks, kept, "the think sequence diverged");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _, _)| run).collect();
    z2_one_log(&runs, "the unreachable station");
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "a mind cannot end an order: the arena journals its expiry at `expires_ms` alone"
    );
    // The station think: the first after the charge crossed, asking for a
    // cell across the moat.
    let (at, station) = kept
        .iter()
        .find(|think| think.1 > 5_000)
        .map(|think| (think.1, z2_goal_cell(&think.3.intents)))
        .expect("a think after the charge crossed");
    assert!(
        at <= 5_000 + STOCK_THINK_MS,
        "the supporter's first think after its charge crossed: {at}"
    );
    let station = station.expect("it asked the motor for the station");
    assert!(station.0 > 21, "a station across the moat: {station:?}");
    // The give-up: the think after the failed search, off the motor's word.
    let gave_up = kept
        .iter()
        .find(|think| kept_memory(think).ignore_until_ms.is_some())
        .expect("the order given up");
    assert_eq!(
        gave_up.1,
        at + STOCK_THINK_MS,
        "at the think after the failed search"
    );
    assert!(
        gave_up.2.notices.iter().any(|notice| matches!(
            notice,
            FrameNotice::Drive {
                outcome: DriveOutcome::NoRoute
            }
        )),
        "read off the motor's `drive_no_route`: {:?}",
        gave_up.2.notices
    );
    let memory = kept_memory(gave_up);
    assert_eq!(
        memory.ignore_until_ms,
        Some(600_000),
        "the order's own deadline: {memory:?}"
    );
    assert_eq!(memory.state, MachineState::Patrol, "{memory:?}");
    // On doctrine from then on.
    let after: Vec<&Kept> = kept.iter().filter(|think| think.1 >= gave_up.1).collect();
    assert!(after.len() > 1, "thinks after the give-up");
    assert!(
        after
            .iter()
            .all(|think| kept_memory(think).ignore_until_ms == Some(600_000)),
        "the key is kept while the order stands on the view"
    );
    assert!(
        after
            .iter()
            .all(|think| z2_goal_cell(&think.3.intents) != Some(station)),
        "the station is never asked for again"
    );
    assert!(
        after.iter().any(|think| z2_goal_cell(&think.3.intents)
            .is_some_and(|cell| waypoint_cells().contains(&cell))),
        "and the supporter walks the graph"
    );
}

// ---- (c) the release -------------------------------------------------------

/// One red machine in the middle of the window and an unarmed blue tank in the
/// far corner that nobody flies. **17b cut M4.4**: the machine is a
/// `Controller::Mind` on Standard Doctrine.
fn z2_release_field() -> Arena {
    arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(8, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 15),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    )
}

/// The releasing pilot's tank.
const Z2_RELEASER: ObjId = ObjId(77);

/// One replay of the release: a ten-minute `WITHDRAW` at beat 0; red-1's
/// `RELEASE` at 10 s; the `WITHDRAW` again at 15 s; a `RELEASE` for red-2 at
/// 20 s. Read at 15 s, before the second order lands.
fn z2_release(cadence_ms: i64) -> (Replay, Option<MachineState>) {
    let arena = z2_release_field();
    let squad = z2_squad(&arena, RED);
    let other = tacsim::deliberation::SquadId {
        team: Team::Red,
        index: squad.index + 1,
    };
    let withdraw = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Withdraw,
        false,
        Some(Z2_RELEASER),
    )));
    let release = |squad| tacsim::arena::intents::IntentKind::ReleaseOrder {
        squad,
        issuer: Some(Z2_RELEASER),
    };
    let mut read: Option<Option<MachineState>> = None;
    let run = replay_with(
        cadence_ms,
        30_000,
        arena,
        &|arena, pulse| match (pulse, arena.now_ms) {
            (0, _) | (_, 15_000) => z2_fan(arena, &[RED], &withdraw),
            (_, 10_000) => z2_fan(arena, &[RED], &release(squad)),
            (_, 20_000) => z2_fan(arena, &[RED], &release(other)),
            _ => Vec::new(),
        },
        &mut |arena: &mut Arena| {
            if arena.now_ms == 15_000 {
                // 17b cut M4.4: the mind's cached doctrine state.
                read = Some(machine_state_of(arena, RED));
            }
        },
    );
    (run, read.expect("a pulse ended at 15 s"))
}

/// **16b cut Z.2 (c), 16c Q44 (16b §11: mandatory), on the unit mind** (17b
/// cut M4.4): a release ends an order at one millisecond.
///
/// Every claim of the reflex fixture is kept. A healthy machine under a
/// ten-minute `WITHDRAW` is released at a pulse-aligned beat: one
/// `order_released` line at that arena millisecond at every cadence,
/// carrying the squad and the issuer, and the machine reads `Patrol` on the
/// landings after it, so it does not keep driving home. A release for
/// another squad's word, at a later beat and on a machine under red-1's
/// order again, journals nothing and leaves the order standing; and a
/// release is not an expiry. What moved: the red machine is a
/// `Controller::Mind` on Standard Doctrine. The `WITHDRAW` reaches it
/// through `IntentKind::Order` (`rekey_order` → `MindObjective::Withdraw`;
/// the Withdraw arm writes the state and `ordered`); the `RELEASE` at 10 s
/// is `release_machine_order`'s Mind arm (`holds_order_for` by the squad's
/// rendered word, the mind row's order cleared, `journal_released` at the
/// beat — the line's issuer is the intent's, and red-2's word matches
/// nothing and journals nothing); and the restoration is the memory's (an
/// order's `Withdraw` with no order on the view, healthy and not dry →
/// `Patrol`), read off the cached doctrine state (`machine_state_of`) at
/// 15 s — the first landing after the beat already holds it — as the
/// finals' state after the second `WITHDRAW` is. The beats are pulse-aligned
/// at every cadence; `step_settled`.
#[test]
fn t1_a_release_ends_an_order_at_one_millisecond() {
    let replays: Vec<(Replay, Option<MachineState>)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_release)
        .collect();
    for (_, read) in &replays {
        assert_eq!(
            *read,
            Some(MachineState::Patrol),
            "released, the machine is on doctrine"
        );
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
    z2_one_log(&runs, "the release");
    let released = z2_lines(&runs[0], "order_released");
    assert_eq!(
        released,
        vec![(
            10_000,
            tacsim::arena::ledger::EventKind::OrderReleased {
                unit: RED,
                squad: SmolStr::new("red-1"),
                issuer: Some(Z2_RELEASER),
            }
        )],
        "one line, at the beat, and none for red-2's word"
    );
    assert_eq!(
        runs[0]
            .finals
            .iter()
            .find(|row| row.0 == RED)
            .map(|row| row.5),
        Some(MachineState::Withdraw),
        "red-2's release left red-1's second order standing"
    );
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "a release is not an expiry"
    );
}

// ---- (d) the charge lost ----------------------------------------------------

/// A supporter under `controller` and its charge — a pilot's tank nobody
/// flies — two cells apart.
fn z2_charge_field(controller: Controller) -> Arena {
    arena_of(
        vec![
            tank(
                Z2_CHARGE,
                Team::Red,
                centre(8, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                Z2_SUPPORTER,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                controller,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 15),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    )
}

/// How the charge is lost at 10 s, for [`z2_charge_lost`].
#[derive(Clone, Copy, Debug)]
enum Z2Loss {
    /// The script takes the charge out of the arena.
    Leaves,
    /// The charge is wrecked and the supporter's own hold on it is marked the
    /// way the kill line's own arm marks it (`known_dead`): a charge killed in
    /// a fight the supporter watched, which stays in the arena as a wreck.
    KilledInSight,
    /// The charge is wrecked and nobody tells the supporter: its hold is not
    /// marked and shows no destroyed section.
    WreckedUnseen,
}

/// One replay: `SUPPORT` the charge at beat 0 (the supporter's hold on it
/// planted, so the kill has a row to mark), the charge lost at 10 s by `loss`,
/// the supporter under `controller` — **17b cut M4.4**: a mind for the two
/// ported arms, its thinks kept; the reflex for the `Leaves` STOP. Answers
/// the replay, the kept thinks and whether the supporter's machine still
/// holds the order at the end: the reflex row's standing order, or the mind
/// row's.
fn z2_charge_lost(
    cadence_ms: i64,
    loss: Z2Loss,
    controller: Controller,
) -> (Replay, Vec<Kept>, bool) {
    let mut arena = z2_charge_field(controller);
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_CHARGE, Team::Red, 0);
    let stock = Stock::on(&arena);
    // The recorder flies only a mind's field; the reflex residue's runner
    // stays the arena's own.
    if controller == Controller::Mind {
        arena.mind_runner = stock.clone();
    }
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        None,
    )));
    let mut ordered_at_end = false;
    let run = replay_with(
        cadence_ms,
        20_000,
        arena,
        &|arena, pulse| {
            if pulse == 0 {
                z2_fan(arena, &[Z2_SUPPORTER], &told)
            } else {
                Vec::new()
            }
        },
        &mut |arena: &mut Arena| {
            if arena.now_ms == 10_000 {
                match loss {
                    Z2Loss::Leaves => arena.units.retain(|unit| unit.id != Z2_CHARGE),
                    Z2Loss::KilledInSight => {
                        wreck_it(arena, Z2_CHARGE);
                        learn_the_kill(arena, Z2_SUPPORTER, Z2_CHARGE);
                    }
                    Z2Loss::WreckedUnseen => wreck_it(arena, Z2_CHARGE),
                }
            }
            ordered_at_end = match controller {
                Controller::Mind => arena
                    .minds
                    .get(&Z2_SUPPORTER)
                    .is_some_and(|row| row.order.is_some()),
                _ => arena.reflex.order(Z2_SUPPORTER).is_some(),
            };
        },
    );
    (run, stock.kept(), ordered_at_end)
}

/// **16b cut Z.2 (d), 16c N6 (16b §11: mandatory)**: a `SUPPORT` whose charge
/// **leaves** the arena ends for the supporter through `B-41`'s route — one
/// `order_expired` line naming `support`, at the supporter's first tick after
/// the loss, the same arena millisecond at every cadence — and the supporter
/// is on doctrine, holding no order.
///
/// **17b cut M4.4 — a STOP, left on the reflex controller.** A charge gone
/// from `arena.units` has no row on a mind's view (`frame_scope` drops a held
/// contact whose unit left the match), so T1's `ends` test — the charge's own
/// row reading dead — is false, the leash reads a charge it does not hold and
/// the supporter takes the blind stand under the standing order for the
/// order's whole life; the arena ends a mind's order only at its
/// `expires_ms`. The fixture's two other ways of losing a charge are ported
/// as
/// [`t1_a_support_whose_charge_reads_dead_stops_obeying_and_an_unseen_wreck_ends_nothing`].
/// The owner's remedy: an arena-side end for a mind's `Support` whose charge
/// left the match (beside the expiry; M4.6 territory), or accept the blind
/// stand and delete this arm with the reason recorded.
#[test]
fn z2_a_support_whose_charge_leaves_ends_at_one_millisecond() {
    let loss = Z2Loss::Leaves;
    let what = format!("the charge lost ({loss:?})");
    let replays: Vec<(Replay, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            let (run, _, ordered) = z2_charge_lost(cadence, loss, Controller::Reflex);
            (run, ordered)
        })
        .collect();
    for (_, ordered) in &replays {
        assert!(
            !*ordered,
            "{what}: the supporter is on doctrine, under no order"
        );
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
    z2_one_log(&runs, &what);
    let expired = z2_lines(&runs[0], "order_expired");
    assert_eq!(expired.len(), 1, "{what}: one line: {expired:?}");
    let (t_ms, line) = &expired[0];
    assert!(
        *t_ms > 10_000 && *t_ms <= 10_000 + i64::from(reflex_cfg().decide_ms),
        "{what}: at the supporter's first tick after the loss: {t_ms}"
    );
    assert_eq!(
        *line,
        tacsim::arena::ledger::EventKind::OrderExpired {
            unit: Z2_SUPPORTER,
            squad: SmolStr::new("red-1"),
            objective: SmolStr::new("support"),
        },
        "{what}"
    );
}

/// **16b cut Z.2 (d), 16c N6 (16b §11: mandatory), on the unit mind** (17b
/// cut M4.4): a `SUPPORT` whose charge the supporter's own hold reads dead
/// stops obeying, and a wreck nobody told the supporter of ends nothing.
///
/// The two arms of the reflex fixture a mind can reach, every claim kept but
/// the line, which is re-pinned to the T1 shape — the `order_end` divergence
/// (doc 17 §17): a mind cannot end an order, only stop obeying it. The way a
/// support usually ends in a fight (16b cut Z.2's review): the charge is
/// **killed where the supporter watched it**, stays in the arena as a wreck,
/// and the supporter's own hold reads it dead (`learn_the_kill`, the kill
/// line's own `known_dead`, is the view row's `reads_dead`): T1's `ends`
/// test holds at the supporter's first think after the loss — at most a
/// think period after 10 s, the same lattice millisecond at every cadence —
/// and it writes `ignore_until_ms` at the order's deadline and obeys it no
/// more. What the reflex journalled, the arena never does for a mind: NO
/// `order_expired` line (a mind's order expires only at its `expires_ms`),
/// and the mind row's order still standing at the end. And a charge wrecked
/// where **nobody told the supporter** leaves the order standing for the
/// whole replay — never a key, never a line — because what ends it is what
/// the supporter knows (its own row's `reads_dead`) and never the charge's
/// live wreck flag (doc 16 §1): its first think after the wreck reads the
/// charge alive, and once the hold expires (the sight does not see wrecks)
/// the row is gone and it stands blind under the order, as for any charge
/// it cannot see; no think reads the charge dead. The third way of losing a charge (it
/// **leaves** the arena) stays on the reflex as
/// [`z2_a_support_whose_charge_leaves_ends_at_one_millisecond`], a STOP.
/// Three cadences under `step_settled`: one event list, one hash sequence,
/// one final field, one think sequence.
#[test]
fn t1_a_support_whose_charge_reads_dead_stops_obeying_and_an_unseen_wreck_ends_nothing() {
    use tacsim::arena::mind::STOCK_THINK_MS;
    let charge = draw_contact_id(SEED, Z2_SUPPORTER, Z2_CHARGE);
    let charge_reads = |think: &Kept| {
        think
            .2
            .contacts
            .iter()
            .find(|row| row.id == charge)
            .map(|row| row.reads_dead)
    };

    // Killed in sight: the first think after the loss gives the order up.
    let what = "the charge killed in sight";
    let replays: Vec<(Replay, Vec<Kept>, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| z2_charge_lost(cadence, Z2Loss::KilledInSight, Controller::Mind))
        .collect();
    let kept = replays[0].1.clone();
    for (_, thinks, ordered) in &replays {
        assert!(
            *ordered,
            "{what}: the arena still holds the order: a mind cannot end one"
        );
        assert_eq!(*thinks, kept, "{what}: the think sequence diverged");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _)| run).collect();
    z2_one_log(&runs, what);
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "{what}: a mind cannot end an order: no line"
    );
    let gave_up = kept
        .iter()
        .find(|think| kept_memory(think).ignore_until_ms.is_some())
        .expect("the order given up");
    assert!(
        gave_up.1 > 10_000 && gave_up.1 <= 10_000 + STOCK_THINK_MS,
        "{what}: at the supporter's first think after the loss: {}",
        gave_up.1
    );
    assert_eq!(
        kept_memory(gave_up).ignore_until_ms,
        Some(600_000),
        "{what}: the order's own deadline"
    );
    assert_eq!(
        charge_reads(gave_up),
        Some(Some(true)),
        "{what}: off its own row reading the charge dead: {:?}",
        gave_up.2.contacts
    );
    assert!(
        kept.iter()
            .filter(|think| think.1 > gave_up.1)
            .all(|think| kept_memory(think).ignore_until_ms == Some(600_000)),
        "{what}: obeyed no more while the order stands on the view"
    );

    // The wreck nobody told the supporter about: no end, at any cadence.
    let what = "the charge wrecked unseen";
    let replays: Vec<(Replay, Vec<Kept>, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| z2_charge_lost(cadence, Z2Loss::WreckedUnseen, Controller::Mind))
        .collect();
    let kept = replays[0].1.clone();
    for (_, thinks, ordered) in &replays {
        assert!(
            *ordered,
            "the supporter learned of the wreck through knowledge it does not hold"
        );
        assert!(
            thinks
                .iter()
                .all(|think| kept_memory(think).ignore_until_ms.is_none()),
            "{what}: never given up"
        );
        // And the wreck is never known: the first think after it reads the
        // charge's row alive (the order obeyed through the station arm),
        // and no think reads it dead — once the hold expires (the sight does
        // not see wrecks) the row is gone and the supporter stands blind,
        // still under the order, as for any charge it cannot see.
        let after: Vec<&Kept> = thinks
            .iter()
            .filter(|think| think.0 == Z2_SUPPORTER && think.1 > 10_000)
            .collect();
        assert_eq!(
            after.first().map(|think| charge_reads(think)),
            Some(Some(Some(false))),
            "{what}: the first think after the wreck reads the charge alive"
        );
        assert!(
            after
                .iter()
                .all(|think| charge_reads(think) != Some(Some(true))),
            "{what}: no think reads the charge dead"
        );
        assert_eq!(*thinks, kept, "{what}: the think sequence diverged");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _)| run).collect();
    z2_one_log(&runs, what);
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "a wreck the supporter does not know of ends nothing"
    );
}

// ---------------------------------------------------------------------------
// 16b cut I.3 — a placed machine re-plans (16c N10)
// ---------------------------------------------------------------------------

/// **16b cut I.3**: a machine under `Advance`, placed twenty cells aside by
/// staff, searches again at its next tick — `path_queries` moves within one
/// `decide_ms` of the place — and still reaches its goal from where it now
/// stands. `Arena::resolve_place` clears the machine's goal, route and search
/// stamp and keeps its order, so the next tick re-derives the goal from the
/// order and plans from the new cell.
///
/// On a window of its own, 32 cells square, because the fixture window is
/// sixteen and twenty cells aside does not fit in it.
#[test]
fn i3_a_placed_machine_re_plans() {
    const WIDE: u16 = 32;
    let definition = ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex.wide"),
        terrain: ArenaTerrain::uniform(WIDE, WIDE, CELL_CM, ground()).expect("the wide window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: WIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: WIDE - 2,
                y0: 0,
                x1: WIDE - 1,
                y1: WIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: graph(),
        modes: Vec::new(),
    };
    let mut arena = arena_on(
        definition,
        vec![tank(
            RED,
            Team::Red,
            centre(2, 3),
            Vec::new(),
            Controller::Reflex,
        )],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let goal = (28_u16, 3_u16);
    e4_advance(&mut arena, RED, goal, 600_000);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 8_000, &mut bytes);
    assert!(arena.reflex.unit(RED).is_some(), "the machine");
    let drive = motor(&arena, RED);
    assert_eq!(drive.goal, Some(goal), "under way to the ordered cell");
    assert!(!drive.route.is_empty(), "with a route");
    let (x, y) = arena.index.cell_of_unit(RED).expect("on the window");
    assert!(x > 2, "it has moved off its start: {x}");
    // Twenty cells aside, as the unit's own seat on `helm` would carry it.
    let aside = (x, y + 20);
    let epoch = arena.epochs.current(RED, Channel::Helm);
    let out = arena.step(
        arena.now_ms,
        &[],
        &[tacsim::arena::intents::Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch,
            kind: tacsim::arena::intents::IntentKind::Place {
                x: aside.0,
                y: aside.1,
            },
            seq: 1,
        }],
    );
    absorb(&out, &mut bytes);
    assert_eq!(arena.index.cell_of_unit(RED), Some(aside), "placed");
    let row = arena.reflex.unit(RED).expect("the machine");
    let drive = motor(&arena, RED);
    assert!(drive.goal.is_none() && drive.route.is_empty() && drive.searched_ms.is_none());
    assert!(row.order.is_some(), "the order stands");
    // Its next tick searches again.
    let tick = arena.now_ms + 500;
    let next = run(&mut arena, tick, &mut bytes);
    assert!(
        next.path_queries >= 1,
        "no search within one decide tick of the place"
    );
    assert_eq!(
        arena.motors.row(RED).and_then(|row| row.goal),
        Some(goal),
        "the goal is the order's again"
    );
    // And it gets there.
    let until = arena.now_ms + 60_000;
    run(&mut arena, until, &mut bytes);
    let (x, y) = arena.index.cell_of_unit(RED).expect("on the window");
    assert!(
        x.abs_diff(goal.0).max(y.abs_diff(goal.1)) <= tacsim::arena::motor::ARRIVE_CELLS,
        "the placed machine never reached {goal:?}: it stands at ({x}, {y})"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "placed"), 1);
    assert_eq!(count(&log, "place_refused"), 0);
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — the machines in a Hold match (16c N11; plan items 2, 16, 32,
// 33, 34)
// ---------------------------------------------------------------------------

/// A third red machine, for the O.1 order test.
const O1_RED3: ObjId = ObjId(7);

/// The pilot who types the O.1 `ORDER … HOLD` and its `RELEASE`.
const O1_PILOT: ObjId = ObjId(1_500);

/// **This file's window with a Hold declared**: one key spot at `spot`,
/// `radius_cells` round it, `hold_ms` long, over `paint`ed ground.
fn o1_ground(
    paint: &[(u16, u16, TerrainCell)],
    spot: (u16, u16),
    radius_cells: u16,
    hold_ms: u32,
) -> ArenaDefinition {
    ArenaDefinition {
        key_spots: vec![tacsim::arena::state::KeySpot {
            name: SmolStr::new("The Spot"),
            x: spot.0,
            y: spot.1,
        }],
        modes: vec![tacsim::arena::ArenaMode::Hold(tacsim::arena::HoldRule {
            spot: 0,
            radius_cells,
            hold_ms,
        })],
        ..definition(paint)
    }
}

/// [`arena_on`] at `MatchMode::Hold`.
fn o1_arena(
    definition: ArenaDefinition,
    units: Vec<CustodyUnit>,
    cfg: &ArenaConfig,
    seated: &[ObjId],
) -> Arena {
    let pilots = seated
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: *unit,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition,
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Hold,
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the hold arena")
}

/// Whether `unit` stands inside the arena's Hold square.
fn o1_inside(arena: &Arena, unit: ObjId) -> bool {
    arena
        .unit(unit)
        .and_then(|state| arena.definition.terrain.cell_of(&state.pose))
        .is_some_and(|cell| arena.hold.is_some_and(|hold| hold.square_contains(cell)))
}

/// **16b cut O.1 (16c N11; plan items 2, 16)**: in a Hold match the machines
/// drive to the key spot and park anywhere inside `radius − 1`, and then stop
/// searching; one under a pilot-shaped `ORDER … HOLD` stays where it stands
/// until the `RELEASE`, and then drives to the spot too.
///
/// Red's three machines against a blue tank that stands cold and out of every
/// red sight (so no engagement interrupts), under an hour-long `hold_ms` (so no
/// win does): every free machine ends INSIDE the square, the searches of the
/// second minute are a small fraction of the first's, and the ordered machine
/// has not moved a cell until its release.
#[test]
fn o1_hold_mode_machines_drive_to_the_spot_and_park_inside_the_radius() {
    let ground = o1_ground(&[], (10, 8), 3, 3_600_000);
    let mut arena = o1_arena(
        ground,
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 4),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                RED2,
                Team::Red,
                centre(1, 12),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                O1_RED3,
                Team::Red,
                centre(1, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 0),
                vec![cannon()],
                Controller::Pilot,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let squad = arena
        .squad_of(O1_RED3)
        .map(|roster| roster.id)
        .expect("the machine is in a squad");
    let hold = tacsim::arena::intents::IntentKind::Order(Box::new(tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Hold,
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints {
            hold_fire: false,
            max_range_band: None,
        },
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer: Some(O1_PILOT),
        by: author(squad, Some(O1_PILOT)),
        spot: None,
    }));
    let intent =
        |arena: &Arena, kind: &tacsim::arena::intents::IntentKind| tacsim::arena::intents::Intent {
            unit: O1_RED3,
            channel: Channel::Helm,
            epoch: arena.epochs.current(O1_RED3, Channel::Helm),
            kind: kind.clone(),
            seq: 1,
        };
    let mut bytes: Vec<u8> = Vec::new();
    let order = intent(&arena, &hold);
    absorb(&arena.step(0, &[], &[order]), &mut bytes);
    let first = run(&mut arena, 60_000, &mut bytes);
    let second = run(&mut arena, 120_000, &mut bytes);
    for unit in [RED, RED2] {
        assert!(
            o1_inside(&arena, unit),
            "{unit} did not park inside the square: {:?}",
            arena
                .unit(unit)
                .map(|state| (state.pose.x_cm, state.pose.y_cm))
        );
        let row = arena.reflex.unit(unit).cloned().expect("a machine");
        let drive = motor(&arena, unit);
        assert_eq!(row.state, MachineState::Patrol);
        let here = arena
            .unit(unit)
            .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
        assert_eq!(
            drive.goal, here,
            "{unit} parked: its goal is where it stands"
        );
        assert!(row.hunt.is_none(), "no hunt in a Hold match");
    }
    assert!(first.path_queries > 0, "{first:?}");
    assert!(
        second.path_queries * 5 <= first.path_queries,
        "the searches did not plateau once the side had arrived: {} then {}",
        first.path_queries,
        second.path_queries
    );
    assert_eq!(
        arena.unit(BLUE).map(|state| state.power),
        Some(Power::Off),
        "blue stood cold throughout"
    );
    // The ordered machine stood where it was told to.
    assert_eq!(
        arena
            .unit(O1_RED3)
            .and_then(|state| arena.definition.terrain.cell_of(&state.pose)),
        Some((1, 8)),
        "a machine under a pilot's HOLD does not drive to the spot"
    );
    // The release hands it back to doctrine, and doctrine drives it there.
    let release = tacsim::arena::intents::IntentKind::ReleaseOrder {
        squad,
        issuer: Some(O1_PILOT),
    };
    let order = intent(&arena, &release);
    let wall = arena.now_ms;
    absorb(&arena.step(wall, &[], &[order]), &mut bytes);
    run(&mut arena, 180_000, &mut bytes);
    assert!(
        o1_inside(&arena, O1_RED3),
        "the released machine did not drive to the spot"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "match_ended"), 0, "no win inside an hour");
}

/// **16b cut O.1 (plan item 32): a holder fights from inside the square.** A
/// machine parked on the spot, and an armed hostile it holds past its
/// `engage_cm` and inside its `release_cm` with a line — the band in which a
/// machine outside the square drives toward a stand-off cell. The holder takes
/// `release_cm` as its edge instead, as a machine that cannot close does: it
/// fires within seconds without leaving its cell. The same field at a
/// skirmish is the control — that machine drives out of the cell to close.
/// (This file's window is 160 m across, so the band is the file's own seven to
/// ten cells rather than the shipped tank's 420 to 600 m.)
#[test]
fn o1_a_holder_fights_from_inside_the_square() {
    let units = || {
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(4, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                SIGHT_CELLS,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(13, 8),
                vec![cannon()],
                Controller::Pilot,
            ),
        ]
    };
    // Drive a field for up to fifteen arena seconds with the hostile held,
    // answering when the machine first fired and every cell it stood in.
    let fight = |arena: &mut Arena| -> (Option<i64>, Vec<Option<(u16, u16)>>) {
        let mut bytes: Vec<u8> = Vec::new();
        let mut fired_at: Option<i64> = None;
        let mut cells = Vec::new();
        let mut wall = 0_i64;
        while arena.now_ms < 15_000 && fired_at.is_none() {
            let now = arena.now_ms;
            plant_contact(arena, RED, BLUE, Team::Blue, now);
            let out = arena.step(wall, &[], &[]);
            absorb(&out, &mut bytes);
            cells.push(
                arena
                    .unit(RED)
                    .and_then(|state| arena.definition.terrain.cell_of(&state.pose)),
            );
            if out.motor.shots > 0 {
                fired_at = Some(arena.now_ms);
            }
            wall += 500;
        }
        (fired_at, cells)
    };
    let mut arena = o1_arena(
        o1_ground(&[], (4, 8), 1, 3_600_000),
        units(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > ENGAGE_CM && range <= RELEASE_CM,
        "the fixture needs the hostile between the two edges: {range}"
    );
    let (fired_at, cells) = fight(&mut arena);
    let fired_at = fired_at.expect("the holder fired within fifteen arena seconds");
    assert!(fired_at <= 10_000, "it fired at {fired_at} ms");
    assert!(
        cells.iter().all(|cell| *cell == Some((4, 8))),
        "the holder never left its cell on the spot: {cells:?}"
    );
    assert!(o1_inside(&arena, RED));
    assert_eq!(state_of(&arena, RED), Some(MachineState::Engage));
    // The control: at a skirmish the same machine closes, leaving the cell.
    let mut skirmish = arena_of(units(), &[], &arena_cfg(reflex_cfg()), &[BLUE]);
    let (_, cells) = fight(&mut skirmish);
    assert!(
        cells.iter().any(|cell| cell.is_some_and(|(x, _)| x > 5)),
        "a machine with no spot to hold closes on the hostile: {cells:?}"
    );
}

/// **16b cut O.1 (plan item 33): an `Advance` into the square ends on
/// arrival.** A machine sent by its commander to the key spot from outside the
/// square: the moment it stands inside the square its order ends — one
/// `order_expired` line, the row back on doctrine — and doctrine's Hold goal
/// takes it from there, parking it inside the square with its goal where it
/// stands and its speed ordered to nought.
#[test]
fn o1_an_advance_into_the_square_ends_on_arrival() {
    let ground = o1_ground(&[], (10, 8), 3, 3_600_000);
    let mut arena = o1_arena(
        ground,
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 0),
                vec![cannon()],
                Controller::Pilot,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let squad = arena
        .squad_of(RED)
        .map(|roster| roster.id)
        .expect("the machine is in a squad");
    let advance =
        tacsim::arena::intents::IntentKind::Order(Box::new(tacsim::deliberation::Order {
            squad,
            objective: tacsim::deliberation::Objective::Advance { to: (10, 8) },
            target: None,
            horizon_ms: 600_000,
            constraints: tacsim::deliberation::Constraints {
                hold_fire: false,
                max_range_band: None,
            },
            ttl_ms: 600_000,
            issued_ms: 0,
            issuer: None,
            by: author(squad, None),
            spot: None,
        }));
    let order = tacsim::arena::intents::Intent {
        unit: RED,
        channel: Channel::Helm,
        epoch: arena.epochs.current(RED, Channel::Helm),
        kind: advance,
        seq: 1,
    };
    let mut bytes: Vec<u8> = Vec::new();
    absorb(&arena.step(0, &[], &[order]), &mut bytes);
    // Drive until the order ends, reading where the machine stood when it did.
    let mut ended_inside: Option<bool> = None;
    let mut wall = 500_i64;
    while arena.now_ms < 30_000 && ended_inside.is_none() {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if out.motor.orders_expired > 0 {
            ended_inside = Some(o1_inside(&arena, RED));
        }
        wall += 500;
    }
    assert_eq!(
        ended_inside,
        Some(true),
        "the order ended on arrival in the square"
    );
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert!(row.order.is_none(), "the machine is on doctrine");
    // Doctrine's Hold goal parks it inside the square.
    run(&mut arena, 40_000, &mut bytes);
    assert!(arena.reflex.unit(RED).is_some(), "a machine");
    let drive = motor(&arena, RED);
    let here = arena
        .unit(RED)
        .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
    assert!(o1_inside(&arena, RED));
    assert_eq!(drive.goal, here, "parked where it stands");
    assert_eq!(
        arena.unit(RED).map(|state| state.desired_speed_cms),
        Some(0)
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "order_expired"), 1, "one order_expired line");
}

/// **16b cut O.1 (plan items 16, 34; `B-41`'s lesson): a spot no route reaches
/// is given up after one search and tried again** — `max(hunt_after_ms,
/// repath_ms)` later, with the fallback waypoint holding in between; and at
/// `hunt_after_ms` 0 (the documented "off") the retry is `repath_ms` after each
/// failure, with the fallback still holding between two of them.
#[test]
fn o1_a_sealed_spot_is_given_up_after_one_search_and_tried_again() {
    // A pocket walled on every side, the spot in its middle.
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 8..=12_u16 {
        paint.push((x, 6, wall()));
        paint.push((x, 10, wall()));
    }
    for y in 6..=10_u16 {
        paint.push((8, y, wall()));
        paint.push((12, y, wall()));
    }
    let field = |cfg: ReflexConfig| {
        o1_arena(
            o1_ground(&paint, (10, 8), 1, 3_600_000),
            vec![tank(
                RED,
                Team::Red,
                centre(2, 3),
                vec![cannon()],
                Controller::Reflex,
            )],
            &arena_cfg(cfg),
            &[],
        )
    };
    let nodes = |arena: &Arena| -> Vec<(u16, u16)> {
        arena
            .definition
            .waypoints
            .nodes()
            .iter()
            .map(|node| (node.x, node.y))
            .collect()
    };

    // At the shipped hunt clock, ninety seconds: one failed search, the
    // fallback holds, and one more at the retry.
    let cfg = reflex_cfg();
    let retry = i64::from(cfg.hunt_after_ms.max(cfg.repath_ms));
    assert_eq!(retry, 90_000);
    let mut arena = field(cfg);
    let mut bytes: Vec<u8> = Vec::new();
    let early = run(&mut arena, 60_000, &mut bytes);
    assert_eq!(early.path_failures, 1, "one failed search: {early:?}");
    assert!(arena.reflex.unit(RED).is_some(), "a machine");
    let drive = motor(&arena, RED);
    let given_up = drive
        .hold_retry_ms
        .expect("the spot is given up until a retry");
    assert!(
        drive.goal.is_some_and(|goal| nodes(&arena).contains(&goal)),
        "the fallback waypoint holds: {:?}",
        drive.goal
    );
    let failed_at = given_up - retry;
    assert!(failed_at > 0 && failed_at < 5_000, "failed at {failed_at}");
    let late = run(&mut arena, failed_at + retry + 2_000, &mut bytes);
    assert_eq!(late.path_failures, 1, "one more at the retry: {late:?}");
    assert_eq!(
        arena.motors.row(RED).and_then(|row| row.hold_retry_ms),
        Some(failed_at + 2 * retry),
        "given up again for the same stretch"
    );

    // At `hunt_after_ms` 0, the "off" value: the retry is floored at
    // `repath_ms`. The floor does not change how often the spot is searched —
    // `repath_ms` already throttles every search (`follow`'s `fresh_enough`),
    // so twenty seconds buy five failures either way — but what the machine
    // does BETWEEN two failures: each failure gives the spot up until exactly
    // `repath_ms` after it, and on every tick in between the fallback waypoint
    // is the goal and the sealed spot is not. Unfloored, the retry would be the
    // failure's own millisecond, `hold_goal` would re-assert the spot on the
    // very next tick, and the machine would steer at the pocket between
    // searches — `B-41`'s defect.
    let off = ReflexConfig {
        hunt_after_ms: 0,
        repath_ms: 4_000,
        ..reflex_cfg()
    };
    let mut arena = field(off);
    let mut bytes: Vec<u8> = Vec::new();
    let mut failures = 0_u32;
    let mut between = 0_u32;
    let mut wall = arena.now_ms;
    while arena.now_ms < 20_500 {
        let opened = arena.now_ms;
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        failures += out.motor.path_failures;
        assert!(arena.reflex.unit(RED).is_some(), "a machine");
        let drive = motor(&arena, RED);
        if out.motor.path_failures > 0 {
            // The failure fell in this pulse, `(opened, now]`; its retry is
            // `repath_ms` after it.
            let failed_at = drive.hold_retry_ms.map(|until| until - 4_000);
            assert!(
                failed_at.is_some_and(|at| at >= opened && at <= arena.now_ms),
                "the retry is not the failure plus `repath_ms`: {:?} in {opened}..{}",
                drive.hold_retry_ms,
                arena.now_ms
            );
        } else if drive
            .hold_retry_ms
            .is_some_and(|until| arena.now_ms < until)
        {
            between += 1;
            assert!(
                drive.goal.is_some_and(|goal| nodes(&arena).contains(&goal)),
                "between two failures the fallback waypoint holds, at {}: {:?}",
                arena.now_ms,
                drive.goal
            );
        }
        wall += 500;
    }
    assert!(
        (4..=6).contains(&failures),
        "one failed search per `repath_ms`: {failures}"
    );
    assert!(
        between >= 10,
        "the fallback held on only {between} ticks between failures"
    );
}

/// What [`o1_a_holder_with_no_line_waits_on_the_spot`] reads of one field: every
/// cell the machine stood in, the states it was in, and the rounds it fired.
type O1Wait = (Vec<Option<(u16, u16)>>, Vec<MachineState>, u32);

/// **16b cut O.1 (plan item 32): a holder with no line waits on the spot.**
/// A machine parked on the key spot holds a hostile between its `engage_cm`
/// and its `release_cm`, with a wall cell squarely between them. Inside the
/// square its edge is `release_cm`, so the hostile is inside it; with no line
/// there, a machine off the spot closes on the plot's own cell (E.1's reading)
/// — and a holder that did would walk off the square cell by cell. It stands
/// instead, in `Approach`, for as long as the line is blocked. The same field
/// at a skirmish is the control: that machine leaves its cell.
#[test]
fn o1_a_holder_with_no_line_waits_on_the_spot() {
    let units = || {
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(4, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                SIGHT_CELLS,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                vec![cannon()],
                Controller::Pilot,
            ),
        ]
    };
    let curtain = [(8_u16, 8_u16, wall())];
    // Fifteen arena seconds with the hostile held: every cell the machine
    // stood in, the states it was in, and the rounds it fired.
    let wait = |arena: &mut Arena| -> O1Wait {
        let mut bytes: Vec<u8> = Vec::new();
        let mut cells = Vec::new();
        let mut states = Vec::new();
        let mut shots = 0_u32;
        let mut wall = 0_i64;
        while arena.now_ms < 15_000 {
            let now = arena.now_ms;
            plant_contact(arena, RED, BLUE, Team::Blue, now);
            let out = arena.step(wall, &[], &[]);
            absorb(&out, &mut bytes);
            shots += out.motor.shots;
            cells.push(
                arena
                    .unit(RED)
                    .and_then(|state| arena.definition.terrain.cell_of(&state.pose)),
            );
            states.extend(state_of(arena, RED));
            wall += 500;
        }
        (cells, states, shots)
    };
    let mut arena = o1_arena(
        o1_ground(&curtain, (4, 8), 1, 3_600_000),
        units(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > ENGAGE_CM && range <= RELEASE_CM,
        "the fixture needs the hostile between the two edges: {range}"
    );
    let (cells, states, shots) = wait(&mut arena);
    assert_eq!(shots, 0, "no line, no shot");
    assert!(
        states.contains(&MachineState::Approach),
        "the machine never took the hostile: {states:?}"
    );
    assert!(
        !states.contains(&MachineState::Engage),
        "the wall is between them: {states:?}"
    );
    assert!(
        cells.iter().all(|cell| *cell == Some((4, 8))),
        "the holder left its cell on the spot: {cells:?}"
    );
    assert!(o1_inside(&arena, RED));
    // The control: at a skirmish the same machine closes on the hostile.
    let mut skirmish = arena_of(units(), &curtain, &arena_cfg(reflex_cfg()), &[BLUE]);
    let (cells, _, _) = wait(&mut skirmish);
    assert!(
        cells.iter().any(|cell| cell.is_some_and(|(x, _)| x > 5)),
        "a machine with no spot to hold closes on the hostile: {cells:?}"
    );
}

/// **16b cut O.1 (plan item 33): an `Advance` to a cell on the square's edge
/// hands over one cell short of it.** The ordered cell is the west edge of the
/// square; a machine driving at it from the west stops within
/// [`tacsim::arena::motor::ARRIVE_CELLS`] of it — which can be outside the
/// square — and an order ended only by standing inside would then stand for
/// its whole TTL. Ended at `follow`'s own arrival instead: one
/// `order_expired`, and doctrine's Hold goal drives it the last cells in.
#[test]
fn o1_an_advance_to_the_squares_edge_hands_over_where_it_stops() {
    let ground = o1_ground(&[], (10, 8), 3, 3_600_000);
    let mut arena = o1_arena(
        ground,
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 0),
                vec![cannon()],
                Controller::Pilot,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let edge = (7_u16, 8_u16);
    assert!(arena.hold.is_some_and(
        |hold| hold.square_contains(edge) && !hold.square_contains((edge.0 - 1, edge.1))
    ));
    let order = o1_advance(&arena, RED, edge);
    let mut bytes: Vec<u8> = Vec::new();
    absorb(&arena.step(0, &[], &[order]), &mut bytes);
    let mut ended_at: Option<(u16, u16)> = None;
    let mut wall = 500_i64;
    while arena.now_ms < 30_000 && ended_at.is_none() {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if out.motor.orders_expired > 0 {
            ended_at = arena
                .unit(RED)
                .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
        }
        wall += 500;
    }
    let ended_at = ended_at.expect("the order ended within thirty arena seconds");
    assert!(
        ended_at.0.abs_diff(edge.0).max(ended_at.1.abs_diff(edge.1))
            <= tacsim::arena::motor::ARRIVE_CELLS,
        "it ended where `follow` stops: {ended_at:?}"
    );
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.order.is_none()),
        "the machine is on doctrine"
    );
    run(&mut arena, 45_000, &mut bytes);
    assert!(o1_inside(&arena, RED), "doctrine drove it into the square");
    assert!(arena.reflex.unit(RED).is_some(), "a machine");
    let drive = motor(&arena, RED);
    let here = arena
        .unit(RED)
        .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
    assert_eq!(drive.goal, here, "parked where it stands");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "order_expired"), 1, "one order_expired line");
}

/// A squad `Advance { to }` for `unit`, on the helm, as the commander sends it.
fn o1_advance(arena: &Arena, unit: ObjId, to: (u16, u16)) -> tacsim::arena::intents::Intent {
    let squad = arena
        .squad_of(unit)
        .map(|roster| roster.id)
        .expect("the machine is in a squad");
    tacsim::arena::intents::Intent {
        unit,
        channel: Channel::Helm,
        epoch: arena.epochs.current(unit, Channel::Helm),
        kind: tacsim::arena::intents::IntentKind::Order(Box::new(tacsim::deliberation::Order {
            squad,
            objective: tacsim::deliberation::Objective::Advance { to },
            target: None,
            horizon_ms: 600_000,
            constraints: tacsim::deliberation::Constraints {
                hold_fire: false,
                max_range_band: None,
            },
            ttl_ms: 600_000,
            issued_ms: 0,
            issuer: None,
            by: author(squad, None),
            spot: None,
        })),
        seq: 1,
    }
}

/// **The Hold machines' two fields for the cross-tempo property** (16b cut
/// O.1; 16b §11): (a) a machine on doctrine driving to the spot and parking,
/// beside one sent there by an `Advance` at pulse `0` that hands over on
/// arrival; (b) a machine whose spot is sealed, giving it up and retrying
/// every `max(hunt_after_ms, repath_ms)` with the fallback between.
fn o1_drive_and_hand_over() -> Arena {
    o1_arena(
        o1_ground(&[], (10, 8), 3, 3_600_000),
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 4),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                RED2,
                Team::Red,
                centre(1, 12),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 0),
                vec![cannon()],
                Controller::Pilot,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    )
}

fn o1_sealed_spot() -> Arena {
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 8..=12_u16 {
        paint.push((x, 6, wall()));
        paint.push((x, 10, wall()));
    }
    for y in 6..=10_u16 {
        paint.push((8, y, wall()));
        paint.push((12, y, wall()));
    }
    o1_arena(
        o1_ground(&paint, (10, 8), 1, 3_600_000),
        vec![tank(
            RED,
            Team::Red,
            centre(2, 3),
            vec![cannon()],
            Controller::Reflex,
        )],
        &arena_cfg(ReflexConfig {
            hunt_after_ms: 10_000,
            repath_ms: 4_000,
            ..reflex_cfg()
        }),
        &[],
    )
}

/// One 5,000 ms boundary of the drive: the arena millisecond, and whether each
/// red machine stands inside the square.
type O1Parked = (i64, bool, bool);

/// One 5,000 ms boundary of the sealed spot: the millisecond, and the retry
/// clock.
type O1Retry = (i64, Option<i64>);

/// **16b cut O.1 (16b §11: mandatory): the Hold machines are one event list and
/// one hash sequence at 5,000, 1,000 and 500 ms.** `hold_goal`'s park, the
/// `Advance` handed over on arrival, and the sealed spot's give-up and retry
/// (`MotorRow::hold_retry_ms`, which the hash folds) each happen at an arena
/// millisecond no cadence can move. Sampled at every 5,000 ms boundary — a
/// multiple of all three — the two red machines' squares and the sealed
/// machine's retry clock are the same too, and the arcs really happen.
#[test]
fn o1_the_hold_machines_are_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let cadences = [5_000_i64, 1_000, 500];
    let advance = |arena: &Arena, pulse: u32| {
        if pulse == 0 {
            vec![o1_advance(arena, RED2, (10, 8))]
        } else {
            Vec::new()
        }
    };
    let mut drives: Vec<(Replay, Vec<O1Parked>)> = Vec::new();
    for cadence in cadences {
        let mut seen: Vec<O1Parked> = Vec::new();
        let replay = replay_with(
            cadence,
            40_000,
            o1_drive_and_hand_over(),
            &advance,
            &mut |arena: &mut Arena| {
                if arena.now_ms % 5_000 == 0 {
                    seen.push((arena.now_ms, o1_inside(arena, RED), o1_inside(arena, RED2)));
                }
            },
        );
        drives.push((replay, seen));
    }
    let mut sealed: Vec<(Replay, Vec<O1Retry>)> = Vec::new();
    for cadence in cadences {
        let mut seen: Vec<O1Retry> = Vec::new();
        let replay = replay_with(
            cadence,
            35_000,
            o1_sealed_spot(),
            &|_, _| Vec::new(),
            &mut |arena: &mut Arena| {
                if arena.now_ms % 5_000 == 0 {
                    seen.push((
                        arena.now_ms,
                        arena.motors.row(RED).and_then(|row| row.hold_retry_ms),
                    ));
                }
            },
        );
        sealed.push((replay, seen));
    }
    // The arcs happened: the ordered machine handed over, and both ended on the
    // spot; the sealed machine gave the spot up more than once.
    let (first, parked) = drives.first().expect("three replays");
    assert!(
        first
            .events
            .iter()
            .any(|(_, kind, _)| kind.contains("order_expired")),
        "the Advance never handed over: {:?}",
        first.events
    );
    assert_eq!(
        parked.last(),
        Some(&(40_000, true, true)),
        "both machines ended inside the square: {parked:?}"
    );
    let (_, retries) = sealed.first().expect("three replays");
    let mut stamps: Vec<i64> = retries.iter().filter_map(|(_, until)| *until).collect();
    stamps.dedup();
    assert!(
        stamps.len() >= 2,
        "the sealed spot was given up fewer than twice: {retries:?}"
    );
    for (name, runs) in [
        (
            "drive",
            drives.iter().map(|(run, _)| run).collect::<Vec<_>>(),
        ),
        (
            "sealed",
            sealed.iter().map(|(run, _)| run).collect::<Vec<_>>(),
        ),
    ] {
        let first = runs.first().expect("three replays");
        assert!(
            first.hashes.len() >= 100,
            "{name}: {} substeps",
            first.hashes.len()
        );
        for (at, run) in runs.iter().enumerate().skip(1) {
            let cadence = cadences.get(at).copied().unwrap_or(0);
            assert_ne!(run.pulses, first.pulses, "{name}: one run, not three");
            assert_eq!(
                run.events, first.events,
                "{name}: the event list diverged at {cadence} ms"
            );
            assert_eq!(
                run.hashes, first.hashes,
                "{name}: the hash sequence diverged at {cadence} ms"
            );
            assert_eq!(
                run.finals, first.finals,
                "{name}: the field ended differently at {cadence} ms"
            );
        }
    }
    for (at, (_, seen)) in drives.iter().enumerate().skip(1) {
        assert_eq!(seen, parked, "the squares diverged at run {at}");
    }
    for (at, (_, seen)) in sealed.iter().enumerate().skip(1) {
        assert_eq!(seen, retries, "the retry clock diverged at run {at}");
    }
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — the inert rule (16c N13)
// ---------------------------------------------------------------------------

/// **A `target_mk1` off the fielded catalog**, checked out by the production
/// functions (`match_::custody_unit`) under the reflex controller: the real
/// five sections, the power role alone, no weapon, no sensor, zero speed.
fn o3_target(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let key = mush_core::CanonicalId::new(tacsim::definitions::content::TARGET_MK1)
        .expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the target is fielded");
    let binding = catalog.bindings.get(&key).expect("its binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("the target");
    tacsim::match_::custody_unit(
        &catalog,
        cells,
        id,
        team,
        pose,
        0,
        Controller::Reflex,
        1,
        &tacsim::match_::SealTerms::fixture(),
    )
    .expect("the fixture terms seal")
}

/// **Destroy one of `unit`'s sections the way an impact does**: the M32 section
/// condition set to `destroyed`, then every one of the section's authored
/// consequences applied through `Arena::apply_consequence` and the disabled
/// check run through `Arena::settle_disabled` — the two calls the impact path
/// makes, so what the arena concludes from them is its own.
fn o3_destroy_section(arena: &mut Arena, unit: ObjId, section: &str) {
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetSectionCondition {
            section: SectionId::new(section).expect("a section"),
            condition: mush_tactics::ConditionId::new(tacsim::arena::damage::DESTROYED)
                .expect("a condition"),
        }],
        None,
        Some(SmolStr::new("test:destroy")),
    )
    .expect("a one-op plan");
    let (cells, consequences) = {
        let state = arena.unit(unit).expect("the unit");
        let cells = mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0;
        let consequences = state
            .profile
            .section(section)
            .map(|row| row.consequences.clone())
            .unwrap_or_default();
        (cells, consequences)
    };
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = cells;
    }
    let t_ms = arena.now_ms;
    let mut journal = Vec::new();
    for consequence in &consequences {
        arena.apply_consequence(unit, consequence, t_ms, None, &mut journal);
    }
    arena.settle_disabled(unit, t_ms, &mut journal);
}

/// **The inert rule** (16b cut O.3; 16c N13; plan items 7 and 41(d)): a unit that
/// cannot move and carries no weapon starts up and then does nothing at all.
///
/// A `target_mk1` under the reflex controller, on a window with a patrol graph —
/// which a machine that could move would walk, spending path searches — comes
/// online after `match.startup_ms` and then, over a minute of arena time, makes
/// no path search and stages no order but the one `STARTUP`, and stands in
/// `Patrol` with no goal. An armed machine beside it engages it. And it is out
/// of action by each of doc 16 §3.8's three routes for its hull — wrecked, both
/// tracks gone, the engine killed through the hull's rear — and by nothing less:
/// one track, the turret (whose `remove_weapons` has nothing to remove) or a
/// hull hit that leaves the hull standing leave it in action.
#[test]
fn o3_an_inert_target_starts_up_and_searches_nothing() {
    let target = || o3_target(BLUE, Team::Blue, centre(9, 8));
    // Alone: nothing to see, nothing to shoot, a graph to walk.
    let mut arena = arena_of(vec![target()], &[], &arena_cfg(reflex_cfg()), &[]);
    assert_eq!(arena.unit(BLUE).map(|unit| unit.power), Some(Power::Off));
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 60_000, &mut bytes);
    assert_eq!(
        arena.unit(BLUE).map(|unit| unit.power),
        Some(Power::Online),
        "a cold target starts up, so a thermal sight reads it hot"
    );
    assert_eq!(counters.path_queries, 0, "it searches for nothing");
    assert_eq!(counters.intents, 1, "the one `STARTUP`, and no other order");
    assert_eq!(state_of(&arena, BLUE), Some(MachineState::Patrol));
    let row = arena.reflex.unit(BLUE).cloned().expect("a machine");
    let drive = motor(&arena, BLUE);
    assert_eq!((drive.goal, row.target, row.hunt), (None, None, None));
    assert!(drive.route.is_empty() && row.last_plot.is_none());
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "startup"), 1);
    assert_eq!(
        arena
            .unit(BLUE)
            .map(|unit| (unit.pose.x_cm, unit.pose.y_cm)),
        Some((9_500, 8_500)),
        "it stands where its pod put it"
    );

    // Beside an armed machine: the machine engages it; it still does nothing.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(5, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                10,
            ),
            target(),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 30_000, &mut bytes);
    let log = events(&mut arena, &mut bytes);
    assert!(count(&log, "fired") > 0, "the armed machine shoots at it");
    assert!(
        log.iter().all(|event| !matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::Fired { unit, .. } if *unit == BLUE
        )),
        "and it never shoots back"
    );

    // The three routes out of action, each on a fresh target, and nothing less.
    let fresh = || arena_of(vec![target()], &[], &arena_cfg(reflex_cfg()), &[]);
    // A wreck: the hull.
    let mut arena = fresh();
    o3_destroy_section(&mut arena, BLUE, "turret");
    o3_destroy_section(&mut arena, BLUE, "left_track");
    assert!(
        !arena.out_of_action(BLUE),
        "the turret and one track leave it in action"
    );
    o3_destroy_section(&mut arena, BLUE, "hull");
    assert!(arena.out_of_action(BLUE), "a wreck is out of action");
    assert!(arena.unit(BLUE).is_some_and(|unit| unit.limits.wreck));
    // Both tracks.
    let mut arena = fresh();
    o3_destroy_section(&mut arena, BLUE, "right_track");
    assert!(!arena.out_of_action(BLUE), "one track is not enough");
    o3_destroy_section(&mut arena, BLUE, "left_track");
    assert!(arena.out_of_action(BLUE), "both tracks gone: disabled");
    assert!(
        arena
            .unit(BLUE)
            .is_some_and(|unit| unit.disabled && !unit.limits.wreck)
    );
    // The engine, the section behind the hull's rear facet.
    let mut arena = fresh();
    o3_destroy_section(&mut arena, BLUE, "engine");
    assert!(arena.out_of_action(BLUE), "a dead engine: disabled");
    assert!(
        arena
            .unit(BLUE)
            .is_some_and(|unit| unit.disabled && !unit.limits.wreck)
    );
    assert_eq!(
        arena
            .unit(BLUE)
            .and_then(|unit| unit.profile.section("engine"))
            .and_then(|row| row.struck_through.clone()),
        Some((SmolStr::new("hull"), SmolStr::new("rear"))),
        "struck through the hull's rear"
    );
}

/// **The inert rule under three cadences** (16b §11): a target beside a
/// patrolling machine — the machine on the waypoint graph, the target standing
/// — is one event list and one hash sequence at 5,000, 1,000 and 500 ms, and the
/// target's machine row is the same parked row at every one of them.
#[test]
fn o3_an_inert_target_is_the_same_at_three_cadences() {
    let mut first: Option<(Vec<String>, Vec<u64>)> = None;
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = arena_of(
            vec![
                tank(RED, Team::Red, centre(3, 8), Vec::new(), Controller::Reflex),
                o3_target(BLUE, Team::Blue, centre(12, 3)),
            ],
            &[],
            &arena_cfg(reflex_cfg()),
            &[],
        );
        let mut bytes: Vec<u8> = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        let mut wall = 0_i64;
        while arena.now_ms < 30_000 {
            arena.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = arena.step(wall, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            hashes.extend(arena.substep_hashes.iter().copied());
            wall += cadence;
        }
        let log: Vec<String> = events(&mut arena, &mut bytes)
            .iter()
            .map(|event| {
                format!(
                    "{} {} {}",
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.hash
                )
            })
            .collect();
        assert_eq!(state_of(&arena, BLUE), Some(MachineState::Patrol));
        match &first {
            None => first = Some((log, hashes)),
            Some((events, sequence)) => {
                assert_eq!(&log, events, "the event list differs at {cadence} ms");
                assert_eq!(
                    &hashes, sequence,
                    "the hash sequence differs at {cadence} ms"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 16b cut AE.3 — the drone under the machine
// ---------------------------------------------------------------------------

/// **A drone** (16b cut AE.3): [`tank`] with no weapon and a rules blob that
/// declares `"class": "drone"`, so `Arena::new` reads it as the class the end
/// rules key on — and nothing about the machine flying it reads the class at
/// all, which is what the tests below are about. **17b cut M4.4**: under
/// `controller` — a `Controller::Mind` for the ported fields, the reflex for
/// the Hold-match STOP.
fn ae3_drone(
    id: ObjId,
    team: Team,
    cell: (u16, u16),
    sight: u16,
    share: u16,
    controller: Controller,
) -> CustodyUnit {
    let mut unit = with_mesh(
        tank(id, team, centre(cell.0, cell.1), Vec::new(), controller),
        sight,
        share,
    );
    unit.unit = tactical_unit_ruled(
        id,
        RulesValue::new(
            dref("schema", "tacsim.arena.test"),
            serde_json::json!({ "class": "drone" }),
        )
        .expect("a rules blob"),
    );
    unit.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    unit
}

/// **A drone patrols, hunts, withdraws hurt and shares what it sees — and
/// never approaches, engages or fires — on the unit mind** (16b cut AE.3;
/// doc 16 §2 W2; 16c N15; 17b cut M4.4), under the existing rules and with
/// no line of the mind changed.
///
/// Every claim of the reflex fixture's three fields is kept; what moved is
/// the drone, a `Controller::Mind` on Standard Doctrine (the `"class":
/// "drone"` rules blob, `with_mesh` and the fixture reseal unchanged),
/// stepped with `step_settled`, its state read off the memory per landing
/// (`machine_state_of`) and its lock off the unit. Three fields, one claim
/// each:
///
/// 1. **alone**, nothing held: startup, then `Patrol` towards a node of the
///    graph, then — with nothing held for `hunt_after_ms` — `hunt_goal`'s
///    crossing to the far zone (`SIDE − 2`, the hunt in the memory), and the
///    arrival there, which clears it. Its `fight_limit_cm` is nought (no
///    gun), so `best_hostile_ranked` is empty and the `Approach` gate never
///    opens, and no lock is taken outside `engage_orders`;
/// 2. **on the mesh** (the arena's `share_pass`): it holds a hostile its tank
///    cannot see, never enters `Approach` or `Engage` for it and stages no
///    `FIRE`, and the tank beside it holds the hostile as a shared plot
///    (`own_seen` empty) — `share_ops` counted — which is what a drone is
///    for;
/// 3. **hurt**: under `withdraw_permille` it withdraws towards its own zone
///    (`Goal(Cell)` at `x ≤ 1`), and with no reach (`ranges.limit_cm` 0) it
///    has nothing to stand for (`withdraw_stand` answers none).
///
/// The reflex fixture's fourth field — the drone in a Hold match drives to
/// the spot and holds nothing — is a STOP (the Hold match is not on the
/// view) and stays on the reflex as
/// [`ae3_a_released_drone_in_a_hold_match_drives_to_the_spot_and_holds_nothing`].
#[test]
fn t1_a_drone_patrols_hunts_withdraws_and_shares_what_it_sees() {
    use tacsim::definitions::UnitClass;
    let never_fights = |arena: &Arena, unit: ObjId| {
        let state = machine_state_of(arena, unit);
        assert!(
            !matches!(state, Some(MachineState::Approach | MachineState::Engage)),
            "a drone has no gun to close with: {state:?} at {} ms",
            arena.now_ms
        );
        assert_eq!(
            arena.unit(unit).and_then(|state| state.lock),
            None,
            "and it locks nothing"
        );
    };

    // (1) Alone: the patrol, then the hunt, then the arrival.
    let mut arena = arena_of(
        vec![
            ae3_drone(RED, Team::Red, (6, 3), E4_SIGHT_CELLS, 0, Controller::Mind),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 15),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &e4_cfg(0, 3_000),
        &[BLUE],
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.class),
        Some(UnitClass::Drone)
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.fight_limit_cm(CELL_CM)),
        Some(0),
        "no gun, no reach"
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    let mut patrolled: Option<(u16, u16)> = None;
    let mut hunted: Option<(u16, u16)> = None;
    while arena.now_ms < 20_000 {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        never_fights(&arena, RED);
        if patrolled.is_none() && machine_state_of(&arena, RED) == Some(MachineState::Patrol) {
            patrolled = goal_of(&arena, RED);
        }
        if let Some(cell) = hunt_of(&arena, RED) {
            hunted = Some(cell);
            break;
        }
        wall += 500;
    }
    let patrolled = patrolled.expect("the drone started itself up and walked");
    assert!(
        waypoint_cells().contains(&patrolled),
        "its first goal is a node of the graph: {patrolled:?}"
    );
    let hunted = hunted.expect("a drone that holds nothing crosses to look");
    assert_eq!(hunted.0, SIDE - 2, "the far zone's edge: {hunted:?}");
    let until = arena.now_ms + 25_000;
    let mut arrived = false;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        never_fights(&arena, RED);
        if hunt_of(&arena, RED).is_none() {
            arrived = true;
            break;
        }
        wall += 500;
    }
    assert!(arrived, "and it crossed and arrived");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0, "a drone fires nothing");

    // (2) On the mesh: the drone sees what its tank cannot, and says so.
    let mut arena = arena_of(
        vec![
            ae3_drone(RED, Team::Red, (8, 8), E4_SIGHT_CELLS, 10, Controller::Mind),
            with_mesh(
                tank(
                    RED2,
                    Team::Red,
                    centre(3, 8),
                    vec![cannon()],
                    Controller::Pilot,
                ),
                2,
                10,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(13, 11),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &e4_cfg(0, 0),
        &[RED2, BLUE],
    );
    for id in [RED2, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    let mut share_ops = 0_u32;
    let mut shared = None;
    while arena.now_ms < 6_000 {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        share_ops = share_ops.saturating_add(out.perception.share_ops);
        never_fights(&arena, RED);
        if hold_on(&arena, RED, BLUE).is_some()
            && let Some(copy) = hold_on(&arena, RED2, BLUE)
        {
            shared = Some(copy);
            break;
        }
        wall += 500;
    }
    let copy = shared.expect("the drone's contact reached its tank through the mesh");
    assert!(share_ops > 0, "the mesh was asked: {share_ops}");
    assert!(
        copy.own_seen.is_none(),
        "the tank's two-cell sight never saw it: this is the drone's plot"
    );
    let drone_holds = hold_on(&arena, RED, BLUE).expect("the drone holds it");
    assert_eq!(
        copy.seen_at, drone_holds.seen_at,
        "the shared plot is the drone's sighting and no more"
    );
    // Held, and still never closed on: run on with the hostile in sight.
    let until = arena.now_ms + 6_000;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        never_fights(&arena, RED);
        wall += 500;
    }
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0, "and it staged no FIRE");

    // (3) Hurt: it breaks off for home, and has no reach to stand for. The
    // next think reads the wound and lands `Withdraw` a substep on.
    hurt(&mut arena, RED);
    let until = arena.now_ms + 5_000;
    let mut withdrew = false;
    while arena.now_ms < until {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if machine_state_of(&arena, RED) == Some(MachineState::Withdraw) {
            withdrew = true;
            break;
        }
        wall += 500;
    }
    assert!(withdrew, "a hurt drone withdraws like any machine");
    assert!(arena.minds.contains_key(&RED), "a machine");
    let drive = motor(&arena, RED);
    assert!(
        drive.goal.is_some_and(|(x, _)| x <= 1),
        "towards its own entry zone: {:?}",
        drive.goal
    );
    never_fights(&arena, RED);
}

/// **16b cut AE.3's fourth field, left on the reflex controller** (17b cut
/// M4.4: a STOP — the Hold match, as the o1 fixtures above): a released drone
/// in a Hold match drives to the spot like any machine — `hold_goal` is not
/// gated on arms, and the pack note and the help say so — and on the square
/// it holds nothing. O.1's field: the spot at (10, 8), three cells round it,
/// an hour-long clock, and a blue tank standing cold out of the drone's
/// three-cell sight at (15, 0). The match mode and the Hold rule are not on
/// a mind's view (`NativeStockMind::think` hands `decide` `hold: None`), so
/// a mind's drive to the spot is unreachable until the Hold rule reaches the
/// view (a keyframe field: M5). The fixture's other three fields are ported
/// as [`t1_a_drone_patrols_hunts_withdraws_and_shares_what_it_sees`].
#[test]
fn ae3_a_released_drone_in_a_hold_match_drives_to_the_spot_and_holds_nothing() {
    let never_fights = |arena: &Arena, unit: ObjId| {
        let state = state_of(arena, unit);
        assert!(
            !matches!(state, Some(MachineState::Approach | MachineState::Engage)),
            "a drone has no gun to close with: {state:?} at {} ms",
            arena.now_ms
        );
        assert_eq!(
            arena.unit(unit).and_then(|state| state.lock),
            None,
            "and it locks nothing"
        );
    };
    let mut arena = o1_arena(
        o1_ground(&[], (10, 8), 3, 3_600_000),
        vec![
            ae3_drone(RED, Team::Red, (1, 4), 3, 0, Controller::Reflex),
            tank(
                BLUE,
                Team::Blue,
                centre(15, 0),
                vec![cannon()],
                Controller::Pilot,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    let mut reached = false;
    while arena.now_ms < 60_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        never_fights(&arena, RED);
        if o1_inside(&arena, RED) {
            reached = true;
            break;
        }
        wall += 500;
    }
    assert!(
        reached,
        "the drone never drove onto the square: it stands at {:?}",
        arena
            .unit(RED)
            .and_then(|state| arena.definition.terrain.cell_of(&state.pose))
    );
    // On it, and on it for a while: no presence, no clock, no end.
    let until = arena.now_ms + 20_000;
    run(&mut arena, until, &mut bytes);
    assert!(o1_inside(&arena, RED), "the drone left the square");
    assert_eq!(
        arena.hold_presence(),
        None,
        "a drone alone on the square holds nothing"
    );
    assert_eq!(
        arena.hold.and_then(|hold| hold.held_by),
        None,
        "no Hold clock opened for red"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "match_ended"), 0, "and nothing ended");
    assert_eq!(count(&log, "fired"), 0, "a drone fires nothing");
}
