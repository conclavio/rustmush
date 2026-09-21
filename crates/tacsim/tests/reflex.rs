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
//! - **(f)** the `blocked` edge: a tank held against an impassable band throws its
//!   route away and searches again;
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
//! # Where a test reaches into the arena's own state
//!
//! The arena's fields are public (see `arena/state.rs`), and three of these use
//! that deliberately: **(d)** applies one `mush_tactics::UnitMutationPlan` to take
//! a tank's armour down, because the claim under test is *what the controller does
//! at a health figure* and not how it got there; **(g)** and **(h)** write the
//! shadow contact store, because the claim is about the machine's edges and not
//! about how the sensor pass minted a contact — that is `perception.rs`'s test.

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellAddr, CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
    TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::intents::{Control, ControlState};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::perception::{SensorProfile, SignatureFactor};
use tacsim::arena::reflex::ReflexState;
use tacsim::arena::rng::draw_contact_id;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, Controller, CustodyOut,
    CustodyUnit, EntryZone, Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell, TrackedRules,
    Waypoint, Waypoints, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};

/// The window's side, in cells.
const SIDE: u16 = 16;

/// One cell, centimetres.
const CELL_CM: u16 = 1_000;

/// The match seed every fixture runs on.
const SEED: u64 = 2_350;

/// The red tank.
const RED: ObjId = ObjId(4);

/// The blue tank.
const BLUE: ObjId = ObjId(9);

/// A second red tank, for the cross-tempo property.
const RED2: ObjId = ObjId(6);

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn dref(kind: &str, id: &str) -> DefinitionRef {
    DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim").expect("a canonical owner"),
            DefinitionKind::new(kind).expect("a canonical kind"),
            DefinitionId::new(id).expect("a canonical id"),
        ),
        1,
        DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero")
}

fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
        .expect("an empty rules blob")
}

/// The smallest unit the arena can carry: one hull section, one front facet, one
/// armour cell of 10 and 8 internals — an eighteen-point tank, which is what
/// makes test (d)'s arithmetic readable.
fn tactical_unit(id: ObjId) -> TacticalUnit {
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
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

fn wall() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Wall,
        elev_cm: 0,
        move_cost: 0,
    }
}

fn pose(x_cm: i64, y_cm: i64) -> Pose {
    Pose::at(x_cm, y_cm, 0, rules())
}

/// The cell centre of `(x, y)`.
fn centre(x: u16, y: u16) -> Pose {
    pose(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
    )
}

/// The two-node patrol graph every fixture walks: the west and east ends of row
/// 8, joined. The smallest graph on which `Patrol` produces a route.
fn graph() -> Waypoints {
    Waypoints::new(
        vec![
            Waypoint {
                id: SmolStr::new("west"),
                x: 1,
                y: 8,
            },
            Waypoint {
                id: SmolStr::new("east"),
                x: 14,
                y: 8,
            },
        ],
        &[(SmolStr::new("west"), SmolStr::new("east"))],
    )
}

/// **A bluff**: passable ground thirty metres up, which every stock slope limit
/// refuses either way (16b cut X.6, read by cut Q.7's one-cell follower test).
fn bluff() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 3_000,
        move_cost: 100,
    }
}

/// The window, with `paint`ed cells over uniform ground.
fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    definition_with(paint, graph())
}

/// The same, with a named waypoint graph — **16b cut Q.7** needs one window with
/// no graph at all, so that the `unreachable` edge has nowhere to fall back to
/// and the follower's own one-cell test is the only thing left between the tank
/// and the bluff.
fn definition_with(paint: &[(u16, u16, TerrainCell)], waypoints: Waypoints) -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, cell) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *cell;
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex"),
        terrain: ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the test window"),
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
                x0: SIDE - 2,
                y0: 0,
                x1: SIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints,
    }
}

/// A five-cell sight, so a sixteen-cell window has an outside and a contact can
/// be made to expire.
fn sight() -> SensorProfile {
    use mush_atlas::TerrainClass as C;
    let row = |class: C, moving: u32, stopped: u32| SignatureFactor {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    SensorProfile {
        max_range_cells: 5,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: 0,
        signature: vec![row(C::Ground, 1_000, 1_000), row(C::Grass, 1_000, 800)],
    }
}

/// A cannon whose one band reaches the whole window, so it is never the gun that
/// decides where the machine fights.
///
/// **16b cut E.1**: what decides is the platform's own scope. With [`sight`]'s
/// five cells the fighting reach is `min(250,000, 5,000)` — 5,000 cm — and
/// `engage_cm` is seven tenths of it, 3,500 cm. A fixture that wants a readable
/// band says so with [`with_sight`], and [`SIGHT_CELLS`] is the one cut E.1's own
/// tests use.
fn cannon() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![WeaponBand {
            max_range_cm: 250_000,
            dispersion_mrad: 3,
        }],
        count_per_load: 40,
        damage: 120,
        mount_section: SmolStr::new("turret"),
    }
}

/// **A two-cell carbine**: a hostile four cells away is out of band, so the
/// machine has an `Approach` to make. Everything else is the cannon's shape.
fn carbine() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("carbine"),
        max_range_cm: 2_000,
        bands: vec![WeaponBand {
            max_range_cm: 2_000,
            dispersion_mrad: 3,
        }],
        ..cannon()
    }
}

/// A tank that sees, with `weapons` for what it shoots with and `controller` for
/// who flies it.
fn tank(
    id: ObjId,
    team: Team,
    pose: Pose,
    weapons: Vec<WeaponSummary>,
    controller: Controller,
) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 500,
            brake_cms2: 500,
            turn_rate_mdeg_s: 180_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor: Some(sight()),
        weapons,
        profile: tacsim::arena::resolution::UnitProfile::default(),
        gunnery: 0,
        controller,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: tactical_unit(id),
    }
}

/// The same tank with a longer sight, so a hostile stays held while the machine
/// drives the other way — which is what an `Approach` on a sixteen-cell window
/// needs.
fn with_sight(mut unit: CustodyUnit, cells: u16) -> CustodyUnit {
    unit.sensor = Some(SensorProfile {
        max_range_cells: cells,
        ..sight()
    });
    unit
}

/// A match short enough to test: power comes up in half a second.
fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// The doctrine these tests run on: decide every 500 ms and re-plan whenever the
/// route is gone, so an edge is visible in a handful of pulses rather than in
/// four seconds of arena time.
fn reflex_cfg() -> ReflexConfig {
    ReflexConfig {
        decide_ms: 500,
        repath_ms: 0,
        ..ReflexConfig::default()
    }
}

fn arena_cfg(reflex: ReflexConfig) -> ArenaConfig {
    ArenaConfig {
        reflex,
        ..ArenaConfig::default()
    }
}

/// An arena over `paint`ed ground, with `pilots` seated (by unit id, `#1000` up).
fn arena_of(
    units: Vec<CustodyUnit>,
    paint: &[(u16, u16, TerrainCell)],
    cfg: &ArenaConfig,
    seated: &[ObjId],
) -> Arena {
    arena_on(definition(paint), units, cfg, seated)
}

/// The same over a whole [`ArenaDefinition`] — **16b cut Q.7** needs a window
/// with no waypoint graph.
fn arena_on(
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
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// Step `until_ms` of arena time at 500 ms, absorbing the ledger and the reflex
/// counters.
fn run(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> tacsim::arena::reflex::ReflexCounters {
    let mut wall = arena.now_ms;
    let mut total = tacsim::arena::reflex::ReflexCounters::default();
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        total.absorb(&out.reflex);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    total
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
}

fn events(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(bytes.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

fn count(events: &[LedgerEvent], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.kind.as_str() == kind)
        .count()
}

/// What the controller is doing with `unit`.
fn state_of(arena: &Arena, unit: ObjId) -> Option<ReflexState> {
    arena.reflex.unit(unit).map(|row| row.state)
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
fn plot_of(arena: &Arena, target: ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

/// Make `viewer` hold `target` as a classified hostile from `at_ms`, whatever the
/// ground between them says. See this file's head for why a test may do this.
fn plant_contact(arena: &mut Arena, viewer: ObjId, target: ObjId, team: Team, at_ms: i64) {
    let id = draw_contact_id(SEED, viewer, target);
    arena.contacts.insert(
        viewer,
        id,
        ShadowContact::sighted(
            target,
            Classification::Classified { team },
            at_ms,
            plot_of(arena, target),
        ),
    );
}

// ---------------------------------------------------------------------------
// (a) `Startup -> Patrol`
// ---------------------------------------------------------------------------

#[test]
fn a_cold_ai_tank_starts_itself_up_and_then_walks_the_waypoint_graph() {
    // One AI tank, nobody else on the window: nothing to detect, nothing to
    // approach, so what is left is doc 16 §6's first two states.
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Reflex,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Startup));
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Off));

    let mut bytes: Vec<u8> = Vec::new();
    // One second: the first tick stages `STARTUP` and `match.startup_ms` is 500.
    run(&mut arena, 1_500, &mut bytes);
    assert_eq!(arena.unit(RED).map(|unit| unit.power), Some(Power::Online));
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "startup"),
        1,
        "the controller started it up once, not once a tick"
    );

    // It is making for the nearest node — `west`, at (1, 8), four cells away — and
    // it has a route and an order to drive.
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(row.goal, Some((1, 8)));
    assert_eq!(row.waypoint, Some(0));
    assert!(!row.route.is_empty(), "a patrol with a goal has a route");
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

    // Given the time to get there, it arrives and takes the graph's other edge.
    run(&mut arena, 40_000, &mut bytes);
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(
        (row.waypoint, row.from_waypoint),
        (Some(1), Some(0)),
        "it reached `west` and set off for `east`"
    );
    assert_eq!(row.goal, Some((14, 8)));
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
}

// ---------------------------------------------------------------------------
// (b) `Patrol -> Approach`
// ---------------------------------------------------------------------------

#[test]
fn a_classified_hostile_out_of_reach_is_closed_on_to_the_stand_off_cell() {
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
                    Controller::Reflex,
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
    // Online, detected, and classified: `sensors.classify_after_ms` is 3,000.
    run(&mut arena, 5_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(row.target.map(|(_, unit)| unit), Some(BLUE));
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
    assert_ne!(row.goal, Some((13, 8)), "it does not drive onto the target");
    assert_eq!(
        row.goal,
        Some((12, 8)),
        "the stand-off cell is one cell short of the plot's own"
    );
    // And doc 16 §3.1's `Alert` trigger *a controller has entered `Approach`* is
    // live at last.
    assert!(arena.reflex.any_approaching());
    assert!(arena.clock.tempo() >= Tempo::Alert);
}

// ---------------------------------------------------------------------------
// (j) the controller reads the plot and never the target — 16b cut F.3
// ---------------------------------------------------------------------------

#[test]
fn the_controller_approaches_the_plot_and_not_the_target() {
    // Doc 16 §3.6 read as the owner's principle: the contact store is the whole
    // of what a unit knows. Before 16b cut F.3 every read in this module went
    // through the contact to the **unit** and then read its live pose, so a
    // reflex tank pursued a target it could no longer see, through a wall, with
    // perfect knowledge, for the whole time to live.
    //
    // The red tank carries a two-cell carbine, so a hostile seven cells away is
    // out of band and the machine is in `Approach` — the state whose goal is a
    // position, which is what makes the difference visible.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![carbine()],
                    Controller::Reflex,
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
    run(&mut arena, 5_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));
    let goal = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.goal)
        .unwrap_or_else(|| panic!("a goal"));

    // The target is teleported to the far corner and the red tank's sight is
    // taken away, so nothing refreshes the plot. The contact is still held for
    // the rest of its time to live.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(1, 1);
    }
    if let Some(state) = arena.unit_mut(RED) {
        state.sensor = None;
    }
    run(&mut arena, 7_000, &mut bytes);
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(
        row.target.map(|(_, unit)| unit),
        Some(BLUE),
        "the contact is still held, which is what makes the plot readable"
    );
    assert_eq!(
        row.goal,
        Some(goal),
        "the approach goal followed the target instead of holding on the plot"
    );
    let (gx, gy) = row.goal.unwrap_or((0, 0));
    assert!(
        gx > 4 && gy > 4,
        "the goal moved toward the target's new corner: {:?}",
        row.goal
    );
}

// ---------------------------------------------------------------------------
// (c) `Approach -> Engage`
// ---------------------------------------------------------------------------

#[test]
fn in_band_with_a_line_it_locks_and_fires_and_both_lines_are_in_the_black_box() {
    // **16b cut E.1**: three cells apart, inside a band of 3,500 cm — the
    // fixture's default five-cell [`sight`] against the cannon's 2,500 m, times
    // doctrine's seven tenths. It is the scope and not the gun that decides, and
    // the premise is asserted below rather than left to the reader, because half
    // a cell of margin is not something a comment should be trusted with.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
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
    let mut bytes: Vec<u8> = Vec::new();
    // The premise: the platform's fighting reach is its scope, and the two tanks
    // are inside seven tenths of it.
    let reach = arena.unit(RED).map(|unit| unit.fight_inner_cm(CELL_CM));
    assert_eq!(
        reach,
        Some(5_000),
        "five cells of scope, not the gun's 2.5 km"
    );
    let counters = run(&mut arena, 6_000, &mut bytes);
    assert!(
        plot_range(&arena, RED, BLUE) <= 3_500,
        "the tanks are inside `engage_cm`, which is what makes this the in-band case: {} cm",
        plot_range(&arena, RED, BLUE)
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert!(counters.shots > 0, "{counters:?}");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "locked"), 1, "one lock, not one a tick");
    assert!(count(&log, "fired") > 0);
    // It holds the band rather than driving on to it.
    assert_eq!(arena.unit(RED).map(|unit| unit.desired_speed_cms), Some(0));
    // The cycle is four seconds and the tick is half a second, so a `Cycling`
    // refusal is never staged: the controller reads readiness before it stages.
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

#[test]
fn under_the_health_threshold_it_breaks_off_toward_its_own_entry_zone() {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
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
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert_eq!(arena.reflex_health_permille(RED), 1_000);

    // Eighteen points of armour and internals; three left is 166 per mille, under
    // the stock 350. See this file's head for why the plan is applied here.
    let ops = vec![
        UnitMutationOp::SetCell {
            addr: CellAddr::Facet {
                section: SectionId::new("hull").expect("a section"),
                facet: FacetId::new("front").expect("a facet"),
                cell: mush_tactics::CellId::new("armor").expect("a cell"),
            },
            value: 1,
        },
        UnitMutationOp::SetCell {
            addr: CellAddr::Section {
                section: SectionId::new("hull").expect("a section"),
                cell: mush_tactics::CellId::new("internals").expect("a cell"),
            },
            value: 2,
        },
    ];
    let plan =
        UnitMutationPlan::new(ops, None, Some(SmolStr::new("test:hurt"))).expect("a two-op plan");
    let hurt = {
        let state = arena.unit(RED).expect("the red tank");
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    if let Some(state) = arena.unit_mut(RED) {
        state.unit = hurt;
    }
    assert_eq!(arena.reflex_health_permille(RED), 166);

    // **16b cut Q.7**: the hostile has to be *gone* for this test's claim to be
    // about the health threshold at all. A withdrawing tank that still holds a
    // classified hostile inside its reach with a line stands and shoots back —
    // that is the cut's own `stand` edge, and it has its own test below. So the
    // blue tank drives over the ridge first (a five-cell sight has an outside on
    // a sixteen-cell window) and the contact expires before the damage lands.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    // Where it stood when the damage landed, so "it drove west" is measured from
    // the engagement and not from wherever the run home had already reached.
    let west_of = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    run(&mut arena, 12_000, &mut bytes);
    assert_eq!(
        arena.contacts.of(RED).count(),
        0,
        "the contact expired, so what is left is the health edge"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");
    // **16b cut E.1** shortened the horizon from 32,000 ms, and the reason is the
    // behaviour under test working: at 166 per mille this tank is under
    // `stand_min_permille`, so it no longer stops for three seconds to shoot at a
    // contact it is about to lose — it turns and drives, arrives home sooner, and
    // its `rally_ms` of quiet therefore runs out sooner. Twenty-four seconds is
    // comfortably inside the rally and comfortably past the arrival, which is the
    // window this test's claims are about.
    run(&mut arena, 24_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    // It is making for its **own** entry zone — the red one, columns 0 and 1 —
    // and not the blue one it was closing on.
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert!(
        row.goal.is_some_and(|(x, _)| x <= 1),
        "the goal is in the red entry zone: {:?}",
        row.goal
    );
    assert_eq!(row.target, None, "it gave the target up");
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
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));

    if let Some(state) = arena.unit_mut(RED) {
        state.limits.wreck = true;
        state.desired_speed_cms = 0;
        state.speed_cms = 0;
    }
    let counters = run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Disabled));
    assert_eq!(
        counters.intents, 0,
        "a wreck is not the subject of an order"
    );
    assert_eq!(counters.shots, 0);
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!((row.goal, row.target), (None, None));
    assert!(row.route.is_empty());

    // The same for a tank that is merely disabled.
    if let Some(state) = arena.unit_mut(RED) {
        state.limits.wreck = false;
        state.disabled = true;
    }
    let counters = run(&mut arena, 9_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Disabled));
    assert_eq!(counters.intents, 0);
}

// ---------------------------------------------------------------------------
// (f) the `blocked` edge
// ---------------------------------------------------------------------------

#[test]
fn a_tank_held_against_a_band_throws_its_route_away_and_searches_again() {
    // An impassable band down column 4 with one gap at row 0: `west` is reachable
    // round it, and a tank that drives into the band is stopped (doc 16 §3.4 step
    // 4's `blocked`).
    let band: Vec<(u16, u16, TerrainCell)> = (1..SIDE).map(|y| (4_u16, y, wall())).collect();
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Reflex,
        )],
        &band,
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 30_000, &mut bytes);
    // It found its way round the band rather than sitting against it: the route
    // goes through the gap at row 0.
    assert!(counters.path_queries > 0, "{counters:?}");
    let log = events(&mut arena, &mut bytes);
    let blocked = count(&log, "blocked");
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < 4_000) || blocked > 0,
        "either it got past the band or it was stopped by it at least once"
    );
    // Whatever happened, the machine never gave up: it is still patrolling toward
    // a goal it has a route to.
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert!(row.goal.is_some());
    // And every cell of the route it holds is passable ground.
    for (x, y) in &row.route {
        assert!(
            arena
                .definition
                .terrain
                .get(*x, *y)
                .is_some_and(TerrainCell::passable),
            "the route crosses the band at ({x}, {y})"
        );
    }
}

// ---------------------------------------------------------------------------
// (g) the `lost` edge
// ---------------------------------------------------------------------------

#[test]
fn a_contact_that_expires_drops_the_machine_back_to_patrol() {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
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
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));

    // The blue tank drives over the ridge: a five-cell sight has an outside on a
    // sixteen-cell window, so the contact stops being refreshed and the
    // time-to-live takes it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    run(&mut arena, 20_000, &mut bytes);
    assert!(
        arena.contacts.of(RED).count() == 0,
        "the contact should have expired"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(row.target, None);
    assert!(row.goal.is_some(), "a patrol has a node to walk to");
    let log = events(&mut arena, &mut bytes);
    assert!(count(&log, "lost") > 0);
}

// ---------------------------------------------------------------------------
// (h) the `unreachable` edge
// ---------------------------------------------------------------------------

#[test]
fn a_goal_behind_a_sealed_wall_falls_back_to_the_nearest_waypoint() {
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
                Controller::Reflex,
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
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 1_500, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let counters = run(&mut arena, 6_000, &mut bytes);

    assert!(
        counters.path_failures > 0,
        "the sealed pocket is unreachable: {counters:?}"
    );
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    // The fallback is a node of the graph, and the machine says so.
    let nodes: Vec<(u16, u16)> = arena
        .definition
        .waypoints
        .nodes()
        .iter()
        .map(|node| (node.x, node.y))
        .collect();
    assert!(
        row.goal.is_some_and(|goal| nodes.contains(&goal)),
        "the goal fell back to a waypoint: {:?} of {nodes:?}",
        row.goal
    );
    assert!(row.waypoint.is_some());
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
#[test]
fn a_sealed_goal_escalates_without_cycling_and_keeps_its_avoided_cells() {
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
                Controller::Reflex,
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
    // First the stop: nothing is held yet, so the machine patrols west into the
    // parked tank and learns the cell.
    run(&mut arena, 6_000, &mut bytes);
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
    let mut counters = tacsim::arena::ReflexCounters::default();
    while arena.now_ms < until {
        let now = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        counters.absorb(&out.reflex);
        wall_ms += 500;
    }

    assert!(
        counters.path_failures > 0,
        "the sealed pocket is unreachable: {counters:?}"
    );
    // Thirty-two decide ticks in sixteen seconds; the damper is four.
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
    // state re-asks `stand_off_cell` off the contact's plot on every tick, which
    // is the cut brief's own rule and what makes the bound above the interesting
    // number. What the fallback leaves behind is the node it stepped to.
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.waypoint.is_some()),
        "the fallback never took"
    );
}

// ---------------------------------------------------------------------------
// (i) the path budget
// ---------------------------------------------------------------------------

#[test]
fn the_path_budget_is_a_bound_the_arena_applies_per_window_of_arena_time() {
    // Four AI tanks, one search a window between them: three of the four are
    // refused each window and keep the path they have.
    let cfg = arena_cfg(ReflexConfig {
        max_path_queries: 1,
        ..reflex_cfg()
    });
    let mut arena = arena_of(
        vec![
            tank(
                ObjId(4),
                Team::Red,
                centre(6, 6),
                Vec::new(),
                Controller::Reflex,
            ),
            tank(
                ObjId(5),
                Team::Red,
                centre(7, 6),
                Vec::new(),
                Controller::Reflex,
            ),
            tank(
                ObjId(6),
                Team::Blue,
                centre(8, 6),
                Vec::new(),
                Controller::Reflex,
            ),
            tank(
                ObjId(7),
                Team::Blue,
                centre(9, 6),
                Vec::new(),
                Controller::Reflex,
            ),
        ],
        &[],
        &cfg,
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut worst = 0_u32;
    let mut deferred = 0_u32;
    while arena.now_ms < 6_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        // The window is `decide_ms` (500) and the pulse is 500, so one pulse holds
        // at most one window and the per-pulse count is the per-window bound.
        assert!(
            out.reflex.path_queries <= 1,
            "{} searches in one pulse at a budget of 1",
            out.reflex.path_queries
        );
        worst = worst.max(out.reflex.path_queries);
        deferred = deferred.saturating_add(out.reflex.path_deferred);
        wall += 500;
    }
    assert!(worst > 0, "no search ran at all, so nothing was bounded");
    assert!(
        deferred > 0,
        "nobody was ever refused, so the bound was never reached"
    );
    // Every tank is still patrolling: a unit the budget refused keeps its path and
    // carries on rather than stopping.
    for id in [ObjId(4), ObjId(5), ObjId(6), ObjId(7)] {
        assert_eq!(state_of(&arena, id), Some(ReflexState::Patrol), "{id}");
    }
}

// ---------------------------------------------------------------------------
// (j) the cross-tempo property, with two reflex units
// ---------------------------------------------------------------------------

/// One unit as a replay ends: where it is, how fast, and what its machine was
/// doing — the state the hash folds plus the two fields it deliberately does not
/// (`speed_cms` is in the hash; the goal is what the machine *decided*).
type Final = (ObjId, i64, i64, u32, i32, ReflexState, Option<(u16, u16)>);

/// What one replay produced (the shape `arena.rs` uses).
struct Replay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<Final>,
}

fn replay(cadence_ms: i64, until_ms: i64) -> Replay {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Reflex,
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
    // D-16-2: the tempo decides only how often the arena steps. It is pinned so
    // that the three replays differ in **cadence** and in nothing else.
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
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
            let row = arena.reflex.unit(unit.id);
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                row.map_or(ReflexState::Startup, |row| row.state),
                row.and_then(|row| row.goal),
            )
        })
        .collect();
    Replay {
        events,
        hashes,
        finals,
    }
}

#[test]
fn two_reflex_units_at_three_cadences_are_one_event_list_and_one_hash_sequence() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay(cadence, 30_000))
        .collect();
    let first = runs.first().expect("three replays");

    // The run is worth comparing before it is compared: a controller that did
    // nothing would compare equal three times and prove nothing.
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "detected", "classified", "locked", "fired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}: {kinds:?}"
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
// (k) doc 16 section 3.8's released seat
// ---------------------------------------------------------------------------

#[test]
fn a_released_seat_hands_the_tank_to_the_controller_in_withdraw() {
    // A pilot's tank, running, out in the middle of the window. Its seat is
    // released past the linkdead grace — doc 16 §3.8's `Control { Released }` —
    // and the reflex controller takes it home.
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
    assert!(
        arena.reflex.unit(RED).is_none(),
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
        Some(Controller::Reflex)
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));

    // And it drives itself home, on the epoch the world published — not on one of
    // its own.
    let start = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    run(&mut arena, 16_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert!(
        row.goal.is_some_and(|(x, _)| x <= 1),
        "the goal is the red entry zone: {:?}",
        row.goal
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < start),
        "it drove west"
    );
    // **16b cut Q.7**: and then it comes back. A seat released past the linkdead
    // grace used to leave a tank parked in the entry zone for the rest of the
    // match; after `arena.reflex.rally_ms` of quiet it is an AI teammate again.
    run(&mut arena, 60_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    assert!(
        arena.reflex.unit(RED).is_some_and(|row| row.rallied),
        "it rallied once"
    );

    // A reclaimed seat takes it back, and the machine is forgotten.
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
    assert!(arena.reflex.unit(RED).is_none());
    assert!(
        !arena
            .due
            .iter()
            .any(|event| matches!(event.kind, tacsim::arena::state::DueKind::ReflexTick { .. })),
        "the reclaimed seat took its ticks off the heap"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — the blocked edge learned at the event
// ---------------------------------------------------------------------------

/// The cells one unit's machine is avoiding, without their stamps.
fn avoided(arena: &Arena, unit: ObjId) -> Vec<(u16, u16)> {
    arena
        .reflex
        .unit(unit)
        .map(|row| row.avoid.iter().map(|(cell, _)| *cell).collect())
        .unwrap_or_default()
}

/// How many times the black box says `unit` was stopped **at this cell** — the
/// lens doc 16 §3.4 step 4's *one notice per stop* is read through, and the one
/// that does not move with the substep partition the way a raw count does (16b
/// cut Q.6's `blocked` note).
fn blocked_at(events: &[LedgerEvent], unit: ObjId, cell: (u16, u16)) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Blocked { unit: on, x, y }
                    if on == unit && (x, y) == cell
            )
        })
        .count()
}

/// Turn a unit into a wreck the way a killing round would, so the spatial index
/// keeps holding its cell and nothing targets it.
fn wreck_it(arena: &mut Arena, unit: ObjId) {
    if let Some(state) = arena.unit_mut(unit) {
        state.limits.wreck = true;
        state.limits.motion_disabled = true;
        state.power = Power::Off;
        state.speed_cms = 0;
        state.desired_speed_cms = 0;
    }
}

#[test]
fn a_reflex_tank_stopped_by_a_parked_one_learns_the_cell_and_goes_round_it() {
    // Doc 16 §3.4 step 5: one unit per cell. The blue tank is parked in the red
    // one's way and nobody is driving it, so before this cut the red tank
    // re-planned the same route into it every `repath_ms` for the rest of the
    // match — the owner's "an AI tank queues for ever behind a parked unit".
    //
    // The red tank carries nothing, so it has no target and no `Approach`: what
    // is left is `Patrol` walking to `west`, straight through (5, 8).
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(
                BLUE,
                Team::Blue,
                centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    // The tank walks the graph, so "did it get past" is the WEST-most place it
    // ever reached and not where it happens to be standing at the end.
    let mut wall_ms = 0_i64;
    let mut west_most = i64::MAX;
    let mut counters = tacsim::arena::ReflexCounters::default();
    while arena.now_ms < 30_000 {
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        counters.absorb(&out.reflex);
        west_most = west_most.min(arena.unit(RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall_ms += 500;
    }
    let log = events(&mut arena, &mut bytes);

    assert!(
        blocked_at(&log, RED, (5, 8)) <= 2,
        "one obstacle is one stop, not a stop every pulse: {} stops at (5, 8)",
        blocked_at(&log, RED, (5, 8))
    );
    assert!(
        counters.path_queries >= 2,
        "it searched again after the stop: {counters:?}"
    );
    assert!(
        west_most < 3_000,
        "it went round the parked tank and on to `west` at (1, 8), which `ARRIVE_CELLS` \
         makes cell 2: west-most {west_most} cm"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
}

/// **The re-plan is within one `decide_ms` of the stop at the *shipped*
/// `repath_ms`** (16b cut Q.7), which is the clearing of `searched_ms` and
/// nothing else.
///
/// Every other test in this file runs on [`reflex_cfg`], whose `repath_ms` is
/// zero — and a route-empty tick searches at `repath_ms` zero whether or not
/// anything cleared the stamp, so none of them can see the clearing at all. This
/// one runs at the shipped four seconds, where the damper is real: without the
/// clearing the tank would sit behind the parked one for up to four seconds a
/// time, which is the owner's "an AI tank queues for ever behind a parked unit"
/// with a number on it.
///
/// Both clearing sites are here. (a) the stop, through `Arena::note_blocked`;
/// (b) the shove, through `follow`'s "wandered off the route" branch — a tank
/// moved four cells off its route holds a route whose next cell is not a
/// neighbour, and it must be allowed to say so at once rather than steering
/// sideways until `repath_ms` runs out.
#[test]
fn a_stop_and_a_shove_are_both_re_planned_within_one_decide_tick() {
    const DECIDE_MS: i64 = 500;
    let cfg = arena_cfg(ReflexConfig {
        decide_ms: 500,
        repath_ms: 4_000,
        ..ReflexConfig::default()
    });
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(
                BLUE,
                Team::Blue,
                centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &cfg,
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let searched = |arena: &Arena| arena.reflex.unit(RED).and_then(|row| row.searched_ms);

    // (a) the stop. The stamp the stop threw away is read from the pulse before
    // it, because `Arena::note_blocked` is applied ahead of the due loop and the
    // very substep that stopped the tank may already carry the tick that
    // re-plans it.
    let mut wall_ms = 0_i64;
    let mut held = searched(&arena);
    let mut learned_at: Option<i64> = None;
    let mut spent: Option<i64> = None;
    let mut replanned: Option<i64> = None;
    while arena.now_ms < 30_000 {
        let before = held;
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        wall_ms += 100;
        held = searched(&arena);
        if learned_at.is_none() && !avoided(&arena, RED).is_empty() {
            learned_at = Some(arena.now_ms);
            spent = before;
        }
        if learned_at.is_some() && held.is_some() && held != spent {
            replanned = held;
            break;
        }
    }
    let learned = learned_at.expect("the parked tank stopped it");
    assert!(
        spent.is_some(),
        "it was following a route it had searched for"
    );
    let replanned = replanned.expect("it never searched again");
    assert!(
        replanned - learned <= DECIDE_MS,
        "the re-plan is the next tick and not the next `repath_ms` ({} ms): \
         stopped at {learned} ms, searched at {replanned} ms",
        cfg.reflex.repath_ms
    );

    // (b) the shove. It holds a fresh route now; four cells north of it is a
    // route whose next cell is nobody's neighbour.
    let stamp = searched(&arena);
    assert_eq!(
        stamp,
        Some(replanned),
        "it holds the search it just paid for"
    );
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| !row.route.is_empty()),
        "and the route that came out of it"
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(6, 12);
    }
    let shoved_at = arena.now_ms;
    let mut re_searched: Option<i64> = None;
    while arena.now_ms < shoved_at + 3_000 {
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        wall_ms += 100;
        if searched(&arena) != stamp && searched(&arena).is_some() {
            re_searched = Some(arena.now_ms);
            break;
        }
    }
    let re_searched = re_searched.expect("a shoved tank never asked for a new route");
    assert!(
        re_searched - shoved_at <= DECIDE_MS,
        "a tank pushed off its route re-plans on its next tick and not in \
         {}: shoved at {shoved_at} ms, searched at {re_searched} ms",
        cfg.reflex.repath_ms
    );
}

#[test]
fn a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken() {
    // A band down column 4 with exactly two gaps, at rows 8 and 2. The red tank
    // is on row 8 and the shorter way is through the gap in front of it — which
    // a wreck is standing in. It has to learn that and take the other one.
    let band: Vec<(u16, u16, TerrainCell)> = (0..SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, wall()))
        .collect();
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(RED2, Team::Red, centre(4, 8), Vec::new(), Controller::Pilot),
        ],
        &band,
        &arena_cfg(reflex_cfg()),
        &[],
    );
    wreck_it(&mut arena, RED2);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut west_most = i64::MAX;
    while arena.now_ms < 60_000 {
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        west_most = west_most.min(arena.unit(RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall_ms += 500;
    }
    let log = events(&mut arena, &mut bytes);
    assert!(
        blocked_at(&log, RED, (4, 8)) <= 2,
        "one wreck is one stop: {} stops at (4, 8)",
        blocked_at(&log, RED, (4, 8))
    );
    assert!(
        west_most < 4_000,
        "it is through the other gap: west-most {west_most} cm"
    );
}

#[test]
fn the_avoided_cells_survive_an_approach_whose_goal_moves_every_tick() {
    // The case the owner hit. `Approach` recomputes its goal from the contact's
    // plot at **every** tick, so a list cleared on a goal change would be empty
    // again about once a second — and the tank would drive back into the wreck it
    // had just learned about, for ever.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![carbine()],
                    Controller::Reflex,
                ),
                14,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(12, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(RED2, Team::Red, centre(7, 8), Vec::new(), Controller::Pilot),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    wreck_it(&mut arena, RED2);
    let mut bytes: Vec<u8> = Vec::new();
    // Classified, and closing: the carbine reaches two cells and the target is
    // six away, so this is `Approach` and its goal is a stand-off cell.
    run(&mut arena, 5_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));

    // The target walks north and south a cell at a time, so the goal moves under
    // the machine on every single tick.
    let mut wall_ms = arena.now_ms;
    let mut north = true;
    let mut ever_avoided = false;
    for _ in 0..40 {
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose = centre(12, if north { 9 } else { 7 });
        }
        north = !north;
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        ever_avoided |= !avoided(&arena, RED).is_empty();
        wall_ms += 500;
    }
    assert!(
        ever_avoided,
        "the wreck's cell was never remembered at all, so nothing was proved"
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm > 7_999),
        "it got past the wreck rather than queueing behind it: {:?}",
        arena.unit(RED).map(|unit| unit.pose.x_cm)
    );
}

#[test]
fn a_remembered_stop_ages_out_and_the_cell_is_planned_through_again() {
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(
                BLUE,
                Team::Blue,
                centre(5, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    let mut learned: Option<(u16, u16)> = None;
    for _ in 0..16 {
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        if let Some(cell) = avoided(&arena, RED).first().copied() {
            learned = Some(cell);
            break;
        }
        wall_ms += 500;
    }
    assert_eq!(learned, Some((5, 8)), "the cell it could not enter");
    // The blocker leaves, and thirty seconds of arena time pass. An entry that
    // aged out is a cell the search may use again, which is what stops a wreck
    // that has been cleared from being a permanent hole in the map.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    let at = arena.now_ms;
    run(
        &mut arena,
        at + tacsim::arena::REFLEX_AVOID_TTL_MS + 2_000,
        &mut bytes,
    );
    assert!(
        avoided(&arena, RED).is_empty(),
        "the entry should have aged out at {} ms: {:?}",
        tacsim::arena::REFLEX_AVOID_TTL_MS,
        avoided(&arena, RED)
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — the follower never steers at a refused edge
// ---------------------------------------------------------------------------

#[test]
fn the_follower_holds_rather_than_drive_at_a_bluff() {
    // 16b cut X.6's deviation 13, closed. With no route and no search this tick
    // the follower used to aim at the **goal**, which on relief is a direction and
    // not a way: the tank drove at the bluff, motion refused the edge, and the
    // whole thing happened again on the next tick.
    //
    // The window has no waypoint graph, so the `unreachable` edge has nowhere to
    // fall back to and the one-cell test is the only thing between the tank and
    // the cliff. The tank is DRY — it came in with a cannon and no rounds — so
    // its goal is its own entry zone, the far side of the bluff.
    let bluff_wall: Vec<(u16, u16, TerrainCell)> = (0..SIDE).map(|y| (4_u16, y, bluff())).collect();
    let mut arena = arena_on(
        definition_with(&bluff_wall, Waypoints::new(Vec::new(), &[])),
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Reflex,
        )],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
        // The stock locomotion's own limit; the fixture's hand-made unit declares
        // no mobility, so it is written on here rather than read off a spec.
        state.slope_limit_pct = Some(30);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 30_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Withdraw),
        "a tank with nothing to fire withdraws"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "blocked"),
        0,
        "it never drove at the bluff, so motion never refused it an edge"
    );
    let cell = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose));
    assert_eq!(
        cell,
        Some((5, 8)),
        "it walked up to the last cell it could enter and stopped there"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "and it is waiting for a route rather than pushing"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — Withdraw is not terminal
// ---------------------------------------------------------------------------

/// Take a tank's armour and internals down to 166 per mille, under the stock
/// `withdraw_permille` of 350 **and under 16b cut E.1's `stand_min_permille` of
/// 175**: a tank hurt this far breaks off and will not turn and fight anything on
/// the way home. See this file's head for why a test may do this.
fn hurt(arena: &mut Arena, unit: ObjId) {
    set_health(arena, unit, 1, 2);
}

/// **Hurt, and still willing to fight**: 277 per mille, under
/// `withdraw_permille` (350) so it breaks off and over `stand_min_permille`
/// (175) so the en-route stand is still open to it (16b cut E.1).
///
/// The fixture tank is eighteen points — ten of armour and eight of internals —
/// so two and three is five of eighteen. Every test whose claim is *what a
/// withdrawing tank does when something is close* uses this one; [`hurt`] is for
/// the tests whose claim is the health edge itself, and the two are three
/// hundred metres apart in what a machine does.
fn hurt_but_fighting(arena: &mut Arena, unit: ObjId) {
    set_health(arena, unit, 2, 3);
}

/// One plan, one armour cell and one internals cell.
fn set_health(arena: &mut Arena, unit: ObjId, armor: i64, internals: i64) {
    let ops = vec![
        UnitMutationOp::SetCell {
            addr: CellAddr::Facet {
                section: SectionId::new("hull").expect("a section"),
                facet: FacetId::new("front").expect("a facet"),
                cell: mush_tactics::CellId::new("armor").expect("a cell"),
            },
            value: armor,
        },
        UnitMutationOp::SetCell {
            addr: CellAddr::Section {
                section: SectionId::new("hull").expect("a section"),
                cell: mush_tactics::CellId::new("internals").expect("a cell"),
            },
            value: internals,
        },
    ];
    let plan =
        UnitMutationPlan::new(ops, None, Some(SmolStr::new("test:hurt"))).expect("a two-op plan");
    let hurt = {
        let state = arena.unit(unit).expect("the tank");
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = hurt;
    }
}

#[test]
fn a_cornered_withdrawal_stands_and_fights_without_leaving_the_state() {
    // The flicker: the hostile crosses in and out of the band the withdrawal will
    // stand for, once a pulse for ten seconds. A machine that answered it with a
    // **state** change would throw its route away and buy a path query every
    // second; this one stands where it is, shoots when the line is there, and
    // keeps the run home it had planned.
    //
    // **16b cut E.1** changed two things about this fixture, and both are the new
    // rule rather than a dodge. (a) The gun is the cannon and not the carbine: an
    // en-route stand is inside the `close` band, and a third of the carbine's
    // 2,000 cm reach is 666 cm — less than one cell, so on a 1,000 cm grid no
    // hostile could ever be inside it and the flicker would have nothing to
    // flick. With the cannon and a fourteen-cell sight the band is 4,666 cm, so
    // the near cell (1,000 cm) is a stand and the far one (6,000 cm) is not,
    // which is the flicker this test wants. (b) The damage is
    // [`hurt_but_fighting`], because at 166 per mille the tank is under
    // `stand_min_permille` and would drive past a hostile in its lap — the case
    // `a_dry_withdrawal_drives_on_where_an_armed_one_stands_and_fights` covers.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                14,
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
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 5_000, &mut bytes);
    // Hurt it under the threshold, so the next tick is a `Withdraw`.
    hurt_but_fighting(&mut arena, RED);
    run(&mut arena, 8_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert_eq!(
        arena.reflex_health_permille(RED),
        277,
        "over `stand_min_permille`, so the stand is open to it"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");

    let mut wall_ms = arena.now_ms;
    let mut queries = 0_u32;
    let mut near = true;
    for _ in 0..20 {
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose = centre(if near { 9 } else { 14 }, 8);
        }
        near = !near;
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        queries = queries.saturating_add(out.reflex.path_queries);
        assert_eq!(
            state_of(&arena, RED),
            Some(ReflexState::Withdraw),
            "the flicker moved the machine out of `Withdraw` at {} ms",
            arena.now_ms
        );
        wall_ms += 500;
    }
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

#[test]
fn a_withdrawn_tank_rallies_once_and_then_fights_to_the_end() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Reflex,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    hurt(&mut arena, RED);
    run(&mut arena, 12_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert!(
        arena.reflex.unit(RED).is_some_and(|row| !row.rallied),
        "it has not come back yet"
    );

    // Home, and quiet. `arena.reflex.rally_ms` later it turns round.
    run(&mut arena, 60_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    assert!(arena.reflex.unit(RED).is_some_and(|row| row.rallied));
    assert!(
        arena.reflex_health_permille(RED) < ReflexConfig::default().withdraw_permille,
        "it is still hurt, which is what makes the next assertion mean something"
    );

    // And it does not withdraw again: the health edge is spent.
    run(&mut arena, 120_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Patrol),
        "a rallied tank fights to the end"
    );
}

#[test]
fn a_tank_with_nothing_to_fire_withdraws_and_never_rallies() {
    let mut arena = arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(9, 8),
            vec![cannon()],
            Controller::Reflex,
        )],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 5_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    // Four times the rally lull, at full health and with nothing hostile to see:
    // every other reason to stay out is gone, and it stays out.
    run(&mut arena, 90_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert!(arena.reflex.unit(RED).is_some_and(|row| !row.rallied));
}

/// A hurt tank one cell from a classified hostile, with `rounds` in the
/// magazine: how far west it got over twenty seconds of `Withdraw`, and how many
/// rounds it fired.
fn withdrawing_beside_a_hostile(rounds: u32) -> (i64, usize) {
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(9, 8),
                    vec![cannon()],
                    Controller::Reflex,
                ),
                14,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(10, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), rounds);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    // **16b cut E.1**: the milder recipe, because this fixture's whole subject is
    // what an ARMED withdrawal does when a hostile is on top of it — and at 166
    // per mille the answer would be "keeps driving", for the health and not for
    // the magazine, which is the control's own claim stolen from it.
    hurt_but_fighting(&mut arena, RED);
    assert_eq!(
        arena.reflex_health_permille(RED),
        277,
        "under `withdraw_permille` and over `stand_min_permille`"
    );
    let from = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 20_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Withdraw),
        "the fixture is about what a withdrawal does, so it has to be one"
    );
    let west = from - arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    (
        west,
        count(&events(&mut arena, &mut bytes), "fired") - before,
    )
}

/// **A tank with nothing to fire is never cornered** (16b cut Q.7).
///
/// The stand stops a withdrawing tank in the open and turns it onto the plot,
/// which is worth doing because it shoots back. A dry tank would stop, turn,
/// stage nothing and re-arm the dwell every tick for as long as it held the
/// contact — a parked target in front of a gun, which is the debt this cut was
/// paid to remove and not a stand. The armed half is the control: same ground,
/// same hostile, same damage, and the only difference is the magazine.
#[test]
fn a_dry_withdrawal_drives_on_where_an_armed_one_stands_and_fights() {
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
    assert!(
        dry_west > 5_000 && dry_west > armed_west + 3_000,
        "a dry tank drives home instead of stopping in front of the gun: \
         {dry_west} cm against the armed tank's {armed_west} cm"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — heat
// ---------------------------------------------------------------------------

/// A tank with a real `heat` cell on its hull, carrying `now` of a ceiling of
/// `max` and paying `per_shot` for every round of the cannon.
///
/// The cell is a real one, because the controller and the resolver must be asked
/// the **same** question: [`tacsim::arena::state::UnitState::overheated`] reads
/// the cell, and a platform that declares heat but carries no cell is a platform
/// the engine never refuses.
fn heated(arena: &mut Arena, unit: ObjId, max: u32, now: u32, per_shot: u32) {
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
        components: Vec::new(),
        capabilities: Vec::new(),
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

/// The two tanks the heat tests share: the machine in reach of a parked target.
fn heat_field() -> Arena {
    arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
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
    )
}

#[test]
fn an_overheated_machine_holds_its_fire_rather_than_feeding_a_refusal() {
    // (a) **at the ceiling**: the platform is carrying its whole ten, which is
    // the one reading `apply_intent`'s `Overheated` refusal takes. The
    // controller reads it too and stages nothing at all.
    let mut arena = heat_field();
    heated(&mut arena, RED, 10, 10, 1);
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 8_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert_eq!(counters.shots, 0, "the hot gun stayed quiet: {counters:?}");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0);
    assert!(count(&log, "locked") > 0, "it still closed and locked");

    // (b) **one shot short of it**: a cold platform whose shot costs its whole
    // ceiling. The engine's rule is that a shot which would carry a unit past
    // the maximum leaves it exactly at it and the *next* one is refused
    // (`UnitState::overheated`), so this is a gun that works once — and the
    // machine must take that shot, or a pilot and a machine in the same tank
    // would disagree about whether the gun works at all.
    let mut arena = heat_field();
    heated(&mut arena, RED, 10, 0, 10);
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 8_000, &mut bytes);
    assert_eq!(
        counters.shots, 1,
        "the engine's last legal shot: {counters:?}"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 1);
    assert_eq!(
        arena.unit(RED).and_then(|unit| unit.heat_now()),
        Some(10),
        "and it is at the ceiling now, which is what stopped the second"
    );

    // (c) **past it**: the same shot against a platform already carrying one.
    // Eleven of a ceiling of ten is a shot the engine would accept and the heat
    // would cap, and the controller declines it rather than spending the round
    // for a refusal on the next tick.
    let mut arena = heat_field();
    heated(&mut arena, RED, 10, 1, 10);
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 8_000, &mut bytes);
    assert_eq!(counters.shots, 0, "it would have gone past: {counters:?}");
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — `EndReason::Spent`
// ---------------------------------------------------------------------------

/// Two armed tanks facing each other, one the machine's and one a pilot's.
fn dry_pair(red_rounds: u32, blue_rounds: u32) -> Arena {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                vec![cannon()],
                Controller::Pilot,
            ),
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

#[test]
fn two_dry_sides_end_in_a_draw_and_one_dry_side_plays_on() {
    // Both magazines empty and both tanks whole: doc 16 §3.8's elimination is
    // untouched — neither is a wreck and neither is disabled — so what ends it is
    // that nobody can shoot.
    let mut arena = dry_pair(0, 0);
    let mut bytes: Vec<u8> = Vec::new();
    let mut ended = None;
    let mut wall_ms = 0_i64;
    while arena.now_ms < 20_000 {
        let out = arena.step(wall_ms, &[], &[]);
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
    let mut arena = dry_pair(0, 40);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    while arena.now_ms < 15_000 {
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        assert!(out.ended.is_none(), "a dry side plays on: {:?}", out.ended);
        wall_ms += 500;
    }
}

#[test]
fn a_round_still_in_flight_holds_the_draw_off() {
    // The last round has left the breech and has not landed: it can still decide
    // the match, so calling a draw over it would be calling a draw over a kill.
    let mut arena = dry_pair(1, 0);
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
        let out = arena.step(wall_ms, &[], &[]);
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
    let mut arena = build();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms == SCRIPT_AT_MS {
            script(&mut arena);
        }
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
            let row = arena.reflex.unit(unit.id);
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                row.map_or(ReflexState::Startup, |row| row.state),
                row.and_then(|row| row.goal),
            )
        })
        .collect();
    Replay {
        events,
        hashes,
        finals,
    }
}

/// The detour of `a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken`,
/// as a function nothing but the cadence can vary.
fn wreck_detour() -> Arena {
    let band: Vec<(u16, u16, TerrainCell)> = (0..SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, wall()))
        .collect();
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(RED2, Team::Red, centre(4, 8), Vec::new(), Controller::Pilot),
        ],
        &band,
        &arena_cfg(reflex_cfg()),
        &[],
    );
    wreck_it(&mut arena, RED2);
    arena
}

/// **The whole of `Withdraw`'s new life, in one scripted log**: a hurt machine
/// breaks off, is cornered on the way out and stands and fights, loses the
/// contact when the other tank pulls out at [`SCRIPT_AT_MS`], drives home and
/// rallies twenty quiet seconds later.
fn withdraw_and_rally() -> Arena {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(8, 8),
                vec![cannon()],
                Controller::Reflex,
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
    // **16b cut E.1**: the milder recipe, so the stand this script exists to
    // replay is one the machine's health still allows.
    hurt_but_fighting(&mut arena, RED);
    // And the contact is planted classified at nought, which it was not before.
    // The stand is now inside the `close` band — a third of the lesser of the
    // five-cell sight and the cannon's reach, 1,666 cm — and the two tanks are
    // 1,000 cm apart, so the stand is open on the very first tick and gone three
    // seconds later once the machine has driven a cell and a half west. At HEAD
    // the ceiling was the cannon's whole 3 km, so `classify_after_ms`'s three
    // seconds did not matter; now they do, and a script whose subject is the
    // stand has to start with the classification rather than wait for it.
    plant_contact(&mut arena, RED, BLUE, Team::Blue, 0);
    arena
}

/// Nothing at all — the wreck detour writes its own story.
fn no_script(_arena: &mut Arena) {}

/// The blue tank pulls out to the far corner, off the red one's five-cell sight.
fn blue_pulls_out(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
}

#[test]
fn the_wreck_detour_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_of(cadence, 40_000, wreck_detour, no_script))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    assert!(
        kinds.iter().any(|kind| kind.contains("blocked")),
        "the replay never hit the wreck, so nothing was compared: {kinds:?}"
    );
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

#[test]
fn withdraw_stand_and_rally_are_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_of(cadence, 70_000, withdraw_and_rally, blue_pulls_out))
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
                && !matches!(state, ReflexState::Withdraw)),
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

/// The cells one run of [`wreck_detour`] avoided, sorted, and the west-most
/// place it reached.
fn avoided_and_arrival(cadence_ms: i64) -> (Vec<(u16, u16)>, i64) {
    let mut arena = wreck_detour();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut seen: Vec<(u16, u16)> = Vec::new();
    let mut west_most = i64::MAX;
    while arena.now_ms < 40_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        for cell in avoided(&arena, RED) {
            if !seen.contains(&cell) {
                seen.push(cell);
            }
        }
        west_most = west_most.min(arena.unit(RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall += cadence_ms;
    }
    seen.sort_unstable();
    (seen, west_most)
}

#[test]
fn a_ragged_partition_reaches_the_same_place_with_the_same_avoided_cells() {
    // **What is true of a ragged pulse, and what is not.** 370 ms is not a whole
    // multiple of the 250 ms substep, so the substep boundaries fall elsewhere and
    // every event timestamp moves with them: the event list and the hash sequence
    // are NOT the same, and 16b cut Q.6 says why (an absolute substep lattice is
    // the owner's open question 10), and the exact pose is not either (16b cut
    // Q.6: position while accelerating or turning is first-order in the substep).
    // What the model does promise is the OUTCOME — the tank learns the same cell,
    // goes round the same wreck and gets through the same causeway.
    let regular = avoided_and_arrival(500);
    let ragged = avoided_and_arrival(370);
    assert!(
        !regular.0.is_empty(),
        "nothing was avoided, so nothing was compared"
    );
    assert_eq!(
        regular.0, ragged.0,
        "the set of avoided cells moved with the partition"
    );
    assert!(
        regular.1 < 4_000 && ragged.1 < 4_000,
        "both runs are through the causeway: {} cm regular, {} cm ragged",
        regular.1,
        ragged.1
    );
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
            let row = arena.reflex.unit(unit.id);
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                row.map_or(ReflexState::Startup, |row| row.state),
                row.and_then(|row| row.goal),
            )
        })
        .collect();
    Replay {
        events,
        hashes,
        finals,
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
#[test]
fn a_released_cold_tank_starts_itself_up_and_stays_in_withdraw() {
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
        Some(Controller::Reflex)
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert_eq!(
        arena.unit(RED).map(|unit| unit.power),
        Some(Power::Off),
        "the release itself starts nothing"
    );

    // Its first tick orders the startup, `match.startup_ms` carries it to
    // `Online`, and it is in `Withdraw` the whole way — a machine that had gone
    // through `Startup` would be in `Patrol` by the end of this block, because
    // that is the only state the `Online` arm re-enters `Patrol` from.
    let released_at = arena.now_ms;
    let start = arena.unit(RED).map(|unit| unit.pose.x_cm).unwrap_or(0);
    let until =
        released_at + i64::from(reflex_cfg().decide_ms) + i64::from(match_cfg().startup_ms) + 1_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.power),
        Some(Power::Online),
        "the machine never ordered a startup"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));

    // And it drives home rather than standing where its pilot left it. The
    // horizon stops short of `arena.reflex.rally_ms`, which is a later edge of
    // its own (16b cut Q.7) and not this row's.
    let until = arena.now_ms + 8_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < start),
        "it came up and stood still"
    );
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert!(
        row.goal.is_some_and(|(x, _)| x <= 1),
        "the goal is the red entry zone: {:?}",
        row.goal
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
#[test]
fn a_released_cold_tank_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
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
            .any(|(unit, _, _, _, _, state, _)| *unit == RED && *state == ReflexState::Withdraw),
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
/// and an en-route `Withdraw` stands inside the `close` band of 3,333 cm. Every
/// test below says which of those four it is about.
const SIGHT_CELLS: u16 = 10;
/// `UnitState::fight_inner_cm` and `fight_limit_cm` at [`SIGHT_CELLS`].
const REACH_CM: i64 = 10_000;
/// `Ranges::engage_cm` at [`SIGHT_CELLS`]: seven tenths of [`REACH_CM`].
const ENGAGE_CM: i64 = 7_000;
/// `Ranges::release_cm` at [`SIGHT_CELLS`], which is [`REACH_CM`] itself.
const RELEASE_CM: i64 = REACH_CM;
/// `Ranges::stand_cm` at [`SIGHT_CELLS`]: a third of [`REACH_CM`].
const STAND_CM: i64 = 3_333;

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

/// **`Approach` with range and no line closes on the plot's own cell** (16b cut
/// E.1; 16c's Q9), and takes the trace again on the next tick.
///
/// Before this cut a machine inside its (3 km) reach with no line held where it
/// stood: the `Engage` arm needed a line, and the `else` arm projected a
/// stand-off cell up to 2.1 km behind it. A crew that has lost the line drives at
/// where it last saw the tank, and that is what the goal is now.
#[test]
fn e1_with_range_and_no_line_it_closes_on_the_plot() {
    // One wall cell squarely between them, and four cells of separation — well
    // inside `ENGAGE_CM`, so the only thing stopping the engagement is the line.
    let mut arena = duel((6, 8), (10, 8), &[(8, 8, wall())]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let start = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    run(&mut arena, 4_000, &mut bytes);

    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Approach),
        "the wall is between them, so there is no engagement to be had"
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range <= ENGAGE_CM,
        "and it is inside the band, which is what makes this the no-line case: {range} cm"
    );
    assert_eq!(
        arena.reflex.unit(RED).and_then(|row| row.goal),
        Some((10, 8)),
        "the goal is the plot's own cell"
    );
    assert!(
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm > start),
        "and it is driving at it"
    );

    // Now give it a line at the same sort of range: the trace is taken again on
    // the next tick and the machine engages.
    let here = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .expect("the machine is on the window");
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(here.0, here.1.saturating_sub(3));
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 1_500;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
        "with a line inside the band it locks and fires"
    );
}

/// **The hysteresis** (16b cut E.1; 16c `T-47`): `Approach → Engage` at
/// `engage_cm`, `Engage → Approach` past `release_cm`, and three tenths of the
/// reach between them where neither edge fires.
///
/// At HEAD both edges read one number — the platform's whole weapon reach — so
/// `Engage` could never be left and `engage_band_permille` governed nothing.
#[test]
fn e1_it_engages_at_the_band_releases_past_the_reach_and_holds_between_them() {
    let mut arena = duel((6, 8), (9, 8), &[]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 5_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
        "3,000 cm is inside the 7,000 cm band"
    );

    // (a) The midpoint of the two edges — 8,000 cm, above `ENGAGE_CM` and below
    //     `RELEASE_CM`. An engaged machine stays engaged there, and fires there.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(14, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        (ENGAGE_CM..=RELEASE_CM).contains(&range),
        "the fixture put the target at {range} cm, which is not between the edges"
    );
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
        "between the edges the machine holds the state it had"
    );
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and it may fire across the whole of its `Engage` region"
    );

    // (b) Past `RELEASE_CM`: it gives the band up and closes again, and its goal
    //     is nearer the plot than it is.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 1_000;
    run(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(range > RELEASE_CM, "the target is at {range} cm");
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));
    let goal = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.goal)
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
/// engaged instead of standing a few metres outside the band for ever.
#[test]
fn e1_an_approach_that_arrives_is_inside_the_band_and_stays_there() {
    // Thirteen cells apart: the machine has a real approach to make.
    let mut arena = duel((2, 8), (15, 8), &[]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 30_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
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
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        assert_eq!(
            state_of(&arena, RED),
            Some(ReflexState::Engage),
            "it fell out of the band at {} ms, {} cm out",
            arena.now_ms,
            plot_range(&arena, RED, BLUE)
        );
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
#[test]
fn e1_t46_a_dry_cannon_closes_to_the_coaxial_and_fires_it() {
    let mut arena = duel_with(vec![cannon(), coaxial()], (2, 8), (9, 8), &[]);
    // The cannon is empty; the five-cell coaxial is not.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
        state.ammo.insert(SmolStr::new("coaxial"), 40);
    }
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 30_000, &mut bytes);

    // The coaxial's band is 5,000 cm, inside the sight, so the machine's whole
    // fighting reach is 5,000 cm and its engagement band is 3,500 — where with a
    // loaded cannon it would have stopped at 7,000.
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range <= 3_500,
        "it should have closed to the coaxial's own band, and it is {range} cm out"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
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

/// A hurt withdrawing machine with a classified hostile `cells` east of it: how
/// many rounds it fired, how much ground west it made, and **whether it ever
/// stood** — `ReflexUnit::stand_until_ms`, which is the stand itself rather than
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
fn a_withdrawal_beside(
    gun: WeaponSummary,
    cells: u16,
    fighting: bool,
    home: bool,
    run_ms: i64,
) -> (usize, i64, bool) {
    let at = if home { 1 } else { 9 };
    let mut arena = duel_with(vec![gun], (at, 8), (at + cells, 8), &[]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(at, 8);
        state.speed_cms = 0;
    }
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(at + cells, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    if fighting {
        hurt_but_fighting(&mut arena, RED);
    } else {
        hurt(&mut arena, RED);
    }
    assert_eq!(
        arena.reflex_health_permille(RED),
        if fighting { 277 } else { 166 },
        "the recipe this case asked for"
    );
    let from = arena.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + run_ms;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Withdraw),
        "every case here is about what a withdrawal does, so it has to be one"
    );
    (
        count(&events(&mut arena, &mut bytes), "fired") - before,
        from - arena.unit(RED).map_or(0, |unit| unit.pose.x_cm),
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.stand_until_ms.is_some()),
    )
}

/// **The stands, and the ones that are not** (16b cut E.1; 16c `B-39`, 16c
/// section 5's Q8).
///
/// En route it turns only for something inside the `close` band and only while
/// it has plate left; once home it shoots back at anything inside its firing
/// ceiling, at any health. At HEAD the ceiling was the platform's whole 3 km
/// reach and there was no health floor, so every one of the five cases below
/// stood and fought.
#[test]
fn e1_a_withdrawal_stands_for_what_is_close_on_the_way_out_and_for_anything_at_home() {
    // The geometry the cases are built on, said once: two cells is inside the
    // en-route band and five is outside it, and both are inside the reach the
    // same machine would ENGAGE at.
    assert!(2 * i64::from(CELL_CM) <= STAND_CM);
    assert!(5 * i64::from(CELL_CM) > STAND_CM);
    assert!(5 * i64::from(CELL_CM) <= ENGAGE_CM);

    // (a) En route, above the floor, a hostile two cells out — 2,000 cm, inside
    //     `STAND_CM`'s 3,333: it stands and shoots.
    let (fired, west, stood) = a_withdrawal_beside(cannon(), 2, true, false, 8_000);
    assert!(fired > 0, "inside the close band it turns and fights");
    assert!(west < 2_000, "and it is standing: it made {west} cm");
    assert!(stood, "and the stand is a stand, not a lucky shot");

    // (b) En route, above the floor, a hostile five cells out — 5,000 cm, outside
    //     `STAND_CM`: it drives on. Five cells is still well inside the reach it
    //     would ENGAGE at (7,000 cm), which is the whole point of a narrower band
    //     for a machine that is running away.
    let (fired, west, stood) = a_withdrawal_beside(cannon(), 5, true, false, 6_000);
    assert_eq!(fired, 0, "past the close band it does not stop for it");
    assert!(west > 3_000, "it drove west instead: {west} cm");
    assert!(!stood, "and it never stopped for it either");

    // (c) En route, below the floor, point blank: it keeps driving. A machine
    //     down to its last plate does not turn round for anything. The horizon is
    //     short enough that it has not reached its entry zone, so this is the
    //     en-route arm and not the arrived one.
    let (fired, west, stood) = a_withdrawal_beside(cannon(), 1, false, false, 6_000);
    assert_eq!(fired, 0, "under `stand_min_permille` it will not turn");
    assert!(west > 3_000, "it drove west instead: {west} cm");
    assert!(!stood, "and it never stopped for it either");

    // (d) Home, below the floor, a hostile four cells out — outside the close
    //     band and inside `RELEASE_CM`: it stands and shoots back. It has nowhere
    //     further to run, and a machine shot at and never replying is a parked
    //     target.
    let (fired, _, stood) = a_withdrawal_beside(cannon(), 4, false, true, 8_000);
    assert!(fired > 0, "arrived home, it shoots back at any health");
    assert!(stood, "and it is the arrived stand that did it");

    // (e) Home, below the floor, a hostile past `RELEASE_CM`: it does not. The
    //     gun is the five-cell coaxial so that the case is a real one — the
    //     hostile is eight cells out, which its sight holds comfortably and its
    //     firing ceiling does not reach.
    //
    //     Both halves are asserted, and the second is the one with teeth: the
    //     firing ceiling alone would keep `fired` at nought even if the arrived
    //     arm range-gated at nothing at all, and a machine that stopped and
    //     turned here would be standing in front of a gun it cannot answer.
    let (fired, _, stood) = a_withdrawal_beside(coaxial(), 8, false, true, 8_000);
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
#[test]
fn e1_t32_the_alert_range_is_the_window_and_the_engaged_trigger_is_the_band() {
    // The sixteen-cell window of 1,000 cm cells: `isqrt(16² + 16²)` is 22, so
    // 22,000 cm — against the cannon's own 2 × 250,000 = 500,000 cm.
    let mut arena = duel((2, 8), (14, 8), &[]);
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
            Controller::Reflex,
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
fn close_engage_and_release() -> Arena {
    duel_with(vec![coaxial()], (2, 8), (7, 8), &[])
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
fn engage_release_arc(cadence_ms: i64, until_ms: i64) -> Vec<(i64, ReflexState, i64)> {
    let mut arena = close_engage_and_release();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut seen: Vec<(i64, ReflexState, i64)> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if arena.now_ms == SCRIPT_AT_MS {
            blue_breaks_contact(&mut arena);
        }
        if let Some(state) = state_of(&arena, RED) {
            seen.push((arena.now_ms, state, plot_range(&arena, RED, BLUE)));
        }
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    seen
}

#[test]
fn e1_the_band_taken_and_given_up_is_one_event_list_at_three_cadences() {
    // First: the arc really happens, at one cadence, before anything is compared.
    let arc = engage_release_arc(1_000, 40_000);
    assert!(
        arc.iter().any(|(at, state, range)| *at < SCRIPT_AT_MS
            && *state == ReflexState::Engage
            && *range <= COAXIAL_ENGAGE_CM),
        "it never engaged inside the band before the release: {arc:?}"
    );
    assert!(
        arc.iter().any(|(at, state, range)| *at > SCRIPT_AT_MS
            && *state == ReflexState::Approach
            && *range > COAXIAL_RELEASE_CM),
        "it never gave the band up when the tank opened the range past its firing ceiling: {arc:?}"
    );
    assert!(
        arc.iter().rev().any(|(at, state, range)| *at > SCRIPT_AT_MS
            && *state == ReflexState::Engage
            && *range <= COAXIAL_ENGAGE_CM),
        "it released the band and never closed again: {arc:?}"
    );

    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                40_000,
                close_engage_and_release,
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
