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
use tacsim::arena::rng::{draw_contact_id, stagger_ms};
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
        component: smol_str::SmolStr::new("visual_sensor"),
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

/// **The same tank on the mesh**: `cells` of scope and `share` cells of link.
///
/// [`sight`]'s own `share_range_cells` is nought, so every other fixture in this
/// file runs with the mesh switched off and [`Arena::share_pass`] returns at its
/// first guard. 16b cut E.2 (16c `B-45`) needs the opposite: a teammate whose
/// hold was built by the mesh and by nothing its own sensors took.
fn with_mesh(mut unit: CustodyUnit, cells: u16, share: u16) -> CustodyUnit {
    unit.sensor = Some(SensorProfile {
        max_range_cells: cells,
        share_range_cells: share,
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
    // One second: the first tick, at `stagger_ms(SEED, RED, 500)` — 482 ms since
    // 16b cut U.1 seeded the stagger (it was 4, `id % decide_ms`) — stages
    // `STARTUP`, which resolves at the tick and completes `match.startup_ms`
    // (500) later, at 982: the moment the second tick finds it online and sets
    // off, 18 ms before the check below. **16b cut M.2** (16c `T-02`)
    // resolves the machine's order at its tick; it resolved at the substep's end
    // (250) until then, so the second tick found it still starting and the walk
    // began a tick later, which is why this read `1_500` before the cut. At 1,500
    // the tank has been turning for a second and its next plot is taken off a
    // pose north of the row, so the heading below would no longer read due west.
    assert_eq!(
        stagger_ms(SEED, RED, 500),
        482,
        "the timings above are this offset's"
    );
    run(&mut arena, 1_000, &mut bytes);
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
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the machine back to `Patrol`"
    );
    assert!(
        arena.contacts.of(RED).count() == 0,
        "the contact should have expired"
    );
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(row.target, None);
    // **16b cut E.4 (16c `B-04`)**: the state is still `Patrol` and always was,
    // but the **goal** is no longer a node of the waypoint graph. It is the cell
    // the machine last saw the tank in, for `reflex.search_ms` of arena time or
    // until it gets there — `Patrol`'s own second answer. Before this cut the
    // walk took over at this tick, and its fixed cycle per unit id could lead
    // the machine directly away from the tank that had just driven over a ridge.
    assert_eq!(
        row.goal,
        Some((9, 8)),
        "the goal after the `lost` edge is the last plot's own cell"
    );
    assert_eq!(row.last_plot.map(|(cell, _)| cell), Some((9, 8)));

    // And when it has been there and found nothing, the walk has it back.
    run(&mut arena, 40_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    let row = arena.reflex.unit(RED).cloned().expect("a machine");
    assert_eq!(row.last_plot, None, "arrival ends the search");
    let goal = row.goal.expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the waypoint walk resumes: {goal:?}"
    );
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
    /// How many pulses it took — **16b cut E.4**'s guard, the shape
    /// `perception.rs`'s replay carries: three cadences really are three
    /// different numbers of pulses, so an equality of the three is a property
    /// and not three copies of one run.
    pulses: u32,
}

/// How a replay is driven (16b cut U.1): at a fixed cadence of wall time, the
/// three drivers the (j) property has always compared; or the way the task
/// drives it, waking at [`tacsim::arena::step::StepOutput::next_due_ms`] with the
/// `Quiet` period set to the figure given — the substep lattice in absolute
/// arena time, where the controller's ticks and the passes are due off it.
#[derive(Clone, Copy, Debug)]
enum Drive {
    /// `wall += cadence` after every pulse.
    Every(i64),
    /// `wall = out.next_due_ms` after every pulse, `tempo.quiet_ms` set first.
    Task(u32),
}

fn replay(cadence_ms: i64, until_ms: i64) -> Replay {
    replay_driven(Drive::Every(cadence_ms), until_ms).0
}

/// The (j) field replayed under `drive`; answers the replay, every arena
/// millisecond a pulse after the opening one ended at, and the substep length.
fn replay_driven(drive: Drive, until_ms: i64) -> (Replay, Vec<i64>, i64) {
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
    // The task driver's period is set on the arena's own frozen tempo block,
    // which `next_due_ms` reads, before the opening pulse (the pattern
    // `arena.rs`'s `replay_as_the_task` uses; the hash does not fold it).
    if let Drive::Task(period_ms) = drive {
        arena.arena_cfg.tempo.quiet_ms = period_ms;
    }
    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &arena.arena_cfg.tempo,
        arena.cell_cm(),
        arena.max_speed_cms(),
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    let mut woke_at: Vec<i64> = Vec::new();
    loop {
        let out = arena.step(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        if pulses > 1 {
            woke_at.push(arena.now_ms);
        }
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            Drive::Task(_) => {
                assert!(
                    out.next_due_ms > wall,
                    "the task was asked to wake in the past"
                );
                out.next_due_ms
            }
        };
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
    (
        Replay {
            events,
            hashes,
            finals,
            pulses,
        },
        woke_at,
        substep,
    )
}

/// **16b cut U.1's cross-tempo proof on an effectful field** (16b §11; doc 16
/// §3.1 "Rate independence", D-16-2; 16c `T-22`). `arena.rs`'s task-driver test
/// replays a script whose off-lattice due events change nothing; this is the
/// (j) field — two reflex machines, one of them armed, and a seated tank they
/// detect, classify, lock and shoot — whose ticks fall at 482 and 4 ms into
/// each decide window and whose passes fall off the lattice too. Driven the way
/// the task drives it, waking only on the 250 ms lattice (one substep a pulse,
/// a partition no fixed cadence uses) with the `Quiet` period at 5,000, 1,000
/// and 500 ms, it is one event list, one per-substep hash sequence and one set
/// of finals, all three the fixed 5,000 ms replay's.
#[test]
fn u1_two_reflex_units_on_the_task_s_own_wakes_are_the_fixed_cadence_replay() {
    let fixed = replay(5_000, 30_000);
    // The run is worth comparing: the machines acted, off the lattice.
    for want in ["startup", "detected", "classified", "locked", "fired"] {
        assert!(
            fixed.events.iter().any(|(_, kind, _)| kind.contains(want)),
            "the fixed replay never journalled a {want}"
        );
    }
    let decide = i64::from(reflex_cfg().decide_ms);
    for machine in [RED, RED2] {
        assert_ne!(
            stagger_ms(SEED, machine, decide) % 250,
            0,
            "the precondition: #{}'s tick is off the lattice",
            machine.0
        );
    }
    for period in [5_000_u32, 1_000, 500] {
        let (run, woke_at, substep) = replay_driven(Drive::Task(period), 30_000);
        assert_eq!(substep, 250, "the fixture's lattice");
        let off: Vec<i64> = woke_at
            .iter()
            .copied()
            .filter(|now| now % substep != 0)
            .collect();
        assert!(
            off.is_empty(),
            "the task woke off the lattice at a {period} ms period: {off:?}"
        );
        assert!(
            woke_at.windows(2).any(|pair| {
                let [a, b] = pair else { return false };
                b - a == substep
            }),
            "the task never woke a substep apart at a {period} ms period, so no pulse was one \
             substep"
        );
        assert_eq!(
            run.events, fixed.events,
            "the event list diverged on the task's wakes at a {period} ms period"
        );
        assert_eq!(
            run.hashes, fixed.hashes,
            "the hash sequence diverged on the task's wakes at a {period} ms period"
        );
        assert_eq!(
            run.finals, fixed.finals,
            "the field ended differently on the task's wakes at a {period} ms period"
        );
        assert_ne!(
            run.pulses, fixed.pulses,
            "the task driver ran the fixed replay's pulses at a {period} ms period"
        );
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
    // **16b cut M.2** (16c `T-02`): every machine action is stamped at its own
    // tick, its stagger into each decide window — `stagger_ms(seed, id,
    // decide_ms)` since 16b cut U.1, `RED`'s at 482 and `RED2`'s at 4 at seed
    // 2,350 (they were 4 and 6, `id % decide_ms`), neither of them a substep
    // boundary. Before M.2 these lines carried the end of the substep the tick
    // ran in, a multiple of 250.
    let decide = i64::from(reflex_cfg().decide_ms);
    for machine in [RED, RED2] {
        assert_ne!(
            stagger_ms(SEED, machine, decide) % 250,
            0,
            "the precondition: #{}'s tick is not a substep boundary",
            machine.0
        );
    }
    let mut stamped = 0_usize;
    for (t_ms, json, _) in &first.events {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        let kind = value.get("kind").and_then(serde_json::Value::as_str);
        if !matches!(kind, Some("fired" | "locked" | "startup")) {
            continue;
        }
        let Some(unit) = value.get("unit").and_then(serde_json::Value::as_i64) else {
            continue;
        };
        if unit == i64::from(BLUE.0) {
            continue;
        }
        let id = ObjId(u32::try_from(unit).unwrap_or(0));
        assert_eq!(
            t_ms.rem_euclid(decide),
            stagger_ms(SEED, id, decide),
            "a {kind:?} line of #{unit} off its tick: {t_ms}"
        );
        stamped += 1;
    }
    assert!(stamped >= 3, "the machines stamped {stamped} actions");

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
                tacsim::arena::ledger::EventKind::Blocked { unit: on, x, y, .. }
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

/// **16b cut M.3** (16c `T-04`; B-43's guarantee under the one-edge rule): a
/// machine still held against a wreck longer than `REFLEX_AVOID_TTL_MS` still
/// lists the wreck's cell at its next tick, and one that is no longer held
/// against it lets it age out as before.
///
/// The `blocked` line and `Arena::note_blocked` fire once per edge now, so the
/// per-repeat stamp refresh a pinned machine used to get from every re-collision
/// is gone; the tick refreshes the stamp off `UnitState::held` instead, before its
/// TTL sweep. The two halves differ in nothing but `held`, set by hand to the
/// value motion leaves a unit pinned against an occupied cell with, and the tick
/// is driven at its own arena millisecond — thirty-one seconds after the stop.
#[test]
fn m3_a_machine_held_against_a_wreck_past_the_ttl_still_avoids_its_cell() {
    use tacsim::arena::motion::{BlockedCause, Held};

    let wreck_cell = (5_u16, 8_u16);
    for still_held in [true, false] {
        let mut arena = arena_of(
            vec![
                tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
                tank(
                    BLUE,
                    Team::Blue,
                    centre(wreck_cell.0, wreck_cell.1),
                    Vec::new(),
                    Controller::Pilot,
                ),
            ],
            &[],
            &arena_cfg(reflex_cfg()),
            &[BLUE],
        );
        wreck_it(&mut arena, BLUE);
        let stopped_at = arena.now_ms;
        arena.note_blocked(RED, wreck_cell, BlockedCause::Occupied, stopped_at);
        assert_eq!(
            avoided(&arena, RED),
            vec![wreck_cell],
            "the stop's own edge"
        );
        if let Some(unit) = arena.unit_mut(RED) {
            unit.held = still_held.then_some(Held {
                x: wreck_cell.0,
                y: wreck_cell.1,
                cause: BlockedCause::Occupied,
            });
        }
        let tick_at = stopped_at + tacsim::arena::reflex::REFLEX_AVOID_TTL_MS + 1_000;
        let mut counters = tacsim::arena::ReflexCounters::default();
        assert!(
            arena.reflex_tick(RED, tick_at, &mut counters),
            "still the controller's"
        );
        let stamps: Vec<((u16, u16), i64)> = arena
            .reflex
            .unit(RED)
            .map(|row| row.avoid.clone())
            .unwrap_or_default();
        if still_held {
            assert_eq!(
                stamps,
                vec![(wreck_cell, tick_at)],
                "a machine still pushing at the wreck forgot it"
            );
        } else {
            assert!(
                stamps.is_empty(),
                "a machine no longer held against the wreck kept it past the TTL: {stamps:?}"
            );
        }
    }
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

    // (c) **a shot that would go past it** — **16b cut E.2 (16c `T-55`)
    // reversed this case.** Twenty on a platform carrying fifty of sixty is a
    // shot the engine accepts and clamps (`Arena::raise_heat` writes
    // `min(now + per_shot, max)`), so the machine takes it too: the two halves
    // of one platform may not disagree about whether the gun works, which is
    // the sentence `would_overheat`'s doc used to make the other argument with.
    // It lands ON the ceiling, and the next tick is (a).
    let mut arena = heat_field();
    heated(&mut arena, RED, 60, 50, 20);
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run(&mut arena, 8_000, &mut bytes);
    assert_eq!(
        counters.shots, 1,
        "the engine would have taken this shot and clamped: {counters:?}"
    );
    assert_eq!(
        arena.unit(RED).and_then(|unit| unit.heat_now()),
        Some(60),
        "it landed on the ceiling rather than past it"
    );
    // And it held its fire afterwards, which is the other half of the same
    // line: eight seconds is two cannon cycles.
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 1, "the second shot was refused");
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
    mut arena: Arena,
    beat: &dyn Fn(&Arena, u32) -> Vec<tacsim::arena::intents::Intent>,
    after: &mut dyn FnMut(&mut Arena),
) -> Replay {
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    loop {
        let intents = beat(&arena, pulses);
        let out = arena.step(wall, &[], &intents);
        pulses = pulses.saturating_add(1);
        absorb(&out, &mut bytes);
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
        pulses,
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

/// **What one run of [`wreck_detour`] did** (16b cut E.3 gave it a third field).
struct Detour {
    /// Every cell the machine remembered being stopped at, sorted.
    avoided: Vec<(u16, u16)>,
    /// The west-most centimetre it reached.
    west_most: i64,
    /// The arena millisecond the hold against the wreck opened at, and the one
    /// the stop it matured into was journalled at.
    matured: Option<(i64, i64)>,
}

/// The cells one run of [`wreck_detour`] avoided, sorted, the west-most place it
/// reached, and — **16b cut E.3** — the arena milliseconds the hold against the
/// wreck opened at and the stop it matured into was journalled at.
///
/// The hold is read at each pulse boundary, which is coarser than the substep it
/// opens on; that is enough, because a hold lives two seconds and every pulse
/// here is under six hundred milliseconds, so the reading taken before the stop
/// is the hold that matured into it.
fn avoided_and_arrival(cadence_ms: i64) -> Detour {
    let mut arena = wreck_detour();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut seen: Vec<(u16, u16)> = Vec::new();
    let mut west_most = i64::MAX;
    let mut holds: Vec<(i64, i64)> = Vec::new();
    while arena.now_ms < 40_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        for cell in avoided(&arena, RED) {
            if !seen.contains(&cell) {
                seen.push(cell);
            }
        }
        if let Some(hold) = arena.unit(RED).and_then(|unit| unit.occupied_hold) {
            holds.push((arena.now_ms, hold.since_ms));
        }
        west_most = west_most.min(arena.unit(RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall += cadence_ms;
    }
    let log = events(&mut arena, &mut bytes);
    let stopped_at = log
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Blocked { unit, .. } if unit == RED
            )
        })
        .map(|event| event.t_ms);
    let matured = stopped_at.and_then(|at| {
        holds
            .iter()
            .rfind(|(read_at, _)| *read_at <= at)
            .map(|(_, since_ms)| (*since_ms, at))
    });
    seen.sort_unstable();
    Detour {
        avoided: seen,
        west_most,
        matured,
    }
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
    //
    // **16b cut E.3 put a traffic hold's maturity across the ragged partition.**
    // The hold opens at a substep boundary and matures at the first boundary at
    // or after `since_ms + occupied_stop_ms`, so under a 370 ms pulse the stamp
    // moves with the partition exactly as every other event timestamp does — and
    // a difference between the two runs' timestamps is expected and is not a
    // defect. What each run promises on its own is that the stop is not early and
    // is inside one substep of its due time.
    let regular = avoided_and_arrival(500);
    let ragged = avoided_and_arrival(370);
    assert!(
        !regular.avoided.is_empty(),
        "nothing was avoided, so nothing was compared"
    );
    assert_eq!(
        regular.avoided, ragged.avoided,
        "the set of avoided cells moved with the partition"
    );
    assert!(
        regular.west_most < 4_000 && ragged.west_most < 4_000,
        "both runs are through the causeway: {} cm regular, {} cm ragged",
        regular.west_most,
        ragged.west_most
    );
    for (what, run) in [("regular", regular.matured), ("ragged", ragged.matured)] {
        let (since_ms, stopped_at) =
            run.unwrap_or_else(|| panic!("the {what} run never matured a hold into a stop"));
        let due = since_ms + 2_000;
        assert!(
            stopped_at >= due,
            "the {what} run stopped at {stopped_at} ms, before the hold opened at {since_ms} ms \
             was due at {due} ms"
        );
        assert!(
            stopped_at - due < 250,
            "the {what} run stopped {} ms past its due time, which is more than one substep",
            stopped_at - due
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

/// What the machine settled on.
fn target_of(arena: &Arena, unit: ObjId) -> Option<ObjId> {
    arena
        .reflex
        .unit(unit)
        .and_then(|row| row.target)
        .map(|(_, target)| target)
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

#[test]
fn e2_b45_a_target_that_dies_out_of_sight_is_held_until_the_contact_expires() {
    // (a) The machine holds a planted classified contact on a tank behind a
    // curtain of wall: no line, no sight, so nothing will refresh or correct it.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Reflex,
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
    run(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));
    assert_eq!(target_of(&arena, RED), Some(BLUE));

    // Killed by nobody this machine could have watched: the rig's own destroy,
    // with no shooter, so no `Kill` line and no `known_dead`.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, at, None, &mut journal);
    run(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(
        target_of(&arena, RED),
        Some(BLUE),
        "the machine dropped a target on a fact no platform on the field had"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Approach));
    let held = hold_on(&arena, RED, BLUE).expect("the hold is still there");
    assert!(!held.known_dead, "nobody told it");
    assert!(!arena.contact_reads_dead(&held));

    // And it lets go when the track does, three passes after the last sighting
    // and not a millisecond sooner.
    run(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(arena.contacts.of(RED).count(), 0, "the track expired");
    assert_eq!(target_of(&arena, RED), None);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
}

#[test]
fn e2_b45_a_target_that_leaves_the_match_is_dropped_at_once() {
    // (d) *Gone* is not *dead*: there is no unit for the contact to be a fact
    // about, so the `lost` edge takes it on the next tick whatever the hold says.
    let mut arena = arena_of(
        vec![
            with_sight(
                tank(
                    RED,
                    Team::Red,
                    centre(6, 8),
                    vec![cannon()],
                    Controller::Reflex,
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
    run(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(target_of(&arena, RED), Some(BLUE));

    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    arena.units.retain(|unit| unit.id != BLUE);
    run(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(target_of(&arena, RED), None);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
}

#[test]
fn e2_b45_a_reading_that_shows_the_body_gone_drops_the_target_and_a_turret_does_not() {
    // (c) The knowledge a close look earns. The blue tank is NOT a wreck in
    // either half: what the machine acts on is its own reading, masked with the
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
                        Controller::Reflex,
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
        run(&mut arena, 2_000, &mut bytes);
        let now = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
        run(&mut arena, now + 1_000, &mut bytes);
        assert_eq!(target_of(&arena, RED), Some(BLUE));

        let at = arena.now_ms;
        plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
        plant_reading(&mut arena, RED, BLUE, destroyed, at);
        let held = hold_on(&arena, RED, BLUE).expect("the hold");
        assert!(!held.known_dead, "this half is about the reading alone");
        assert_eq!(arena.contact_reads_dead(&held), !still_held);
        run(&mut arena, at + 1_000, &mut bytes);
        assert_eq!(
            target_of(&arena, RED).is_some(),
            still_held,
            "section {destroyed} destroyed: the machine read it wrong"
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

#[test]
fn e2_b45_a_kill_starts_the_rally_clock_and_a_death_out_of_sight_does_not() {
    // (e) `holds_a_hostile` through the rally. A hurt machine standing in its own
    // entry zone has arrived, so the only thing between it and its quiet clock is
    // whether it still holds a live hostile — and *live* is what it knows.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 8),
                vec![cannon()],
                Controller::Reflex,
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
    let quiet = |arena: &Arena| arena.reflex.unit(RED).and_then(|row| row.quiet_since_ms);

    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    run(&mut arena, now + 1_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert_eq!(quiet(&arena), None, "it holds a live hostile");

    // Killed by somebody else, out of this machine's sight: the board is not
    // quiet yet, because nothing has told it.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, at);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(BLUE, at, None, &mut journal);
    run(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(
        quiet(&arena),
        None,
        "the clock started on a fact the machine had not been given"
    );

    // The same board with the kill written onto the hold, which is what the
    // `Kill` line the shooter's pilot reads says: the clock starts at once.
    learn_the_kill(&mut arena, RED, BLUE);
    let from = arena.now_ms;
    run(&mut arena, from + 1_000, &mut bytes);
    assert!(
        quiet(&arena).is_some(),
        "a machine that watched its own round finish the tank is still waiting"
    );
}

#[test]
fn e2_b45_the_shooter_knows_its_own_kill_and_a_teammate_waits_for_the_track() {
    // (b) The whole of it through the production path: a real `FIRE`, resolved by
    // `Arena::resolve_fire`, whose `destroyed` line carries `by`. The writer is
    // the notice scan's own arm, beside the `Kill` line the shooter's pilot gets.
    //
    // The second red machine here holds the tank through its **own** sight —
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
                    Controller::Reflex,
                ),
                SIGHT_CELLS,
            ),
            with_sight(
                tank(
                    RED2,
                    Team::Red,
                    centre(2, 8),
                    vec![pistol()],
                    Controller::Reflex,
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
            // Out of both machines' ten-cell scopes, so nothing ever holds it
            // and the only thing it does is keep the match from ending.
            bystander((15, 0)),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut ended = None;
    while arena.now_ms < 90_000 {
        let out = arena.step(wall, &[], &[]);
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
        "the machine never killed it, so nothing was proved"
    );

    // The shooter's hold carries the kill, and the shooter's alone: the second
    // red machine watched nothing and holds a track that still says the tank is
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

    // One tick later the shooter is off it and the teammate is not.
    let at = arena.now_ms;
    run(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(target_of(&arena, RED), None, "the shooter is still on it");
    assert_eq!(
        target_of(&arena, RED2),
        Some(BLUE),
        "the teammate let go of a track nothing had refreshed or contradicted"
    );

    // And the teammate lets go when the track expires: `sees_wrecks` is false, so
    // nothing refreshes it once the hull is a wreck.
    run(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(target_of(&arena, RED2), None);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "unit_destroyed"),
        1,
        "the kill went through the engine's own line, once"
    );
}

#[test]
fn e2_b45_a_share_carries_the_track_and_never_the_kill() {
    // (b)'s other half, and the one the mesh's own arm is about. In the test
    // above the second red machine has its **own** sighting of the tank; here it
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
                    Controller::Reflex,
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
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
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
    run(&mut arena, at + 1_500, &mut bytes);
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
    assert_eq!(target_of(&arena, RED2), Some(BLUE));
    assert_eq!(state_of(&arena, RED2), Some(ReflexState::Approach));

    // And the passes after it take `share_pass`'s **update** arm on the very
    // same row, which is where a later tidy-up would most easily let the kill
    // across. It stays out of it, and the machine keeps closing on a tank that
    // is not there — the cost this cut's predicate states.
    run(&mut arena, at + 2_500, &mut bytes);
    let mate = hold_on(&arena, RED2, BLUE).expect("the track is still inside its time to live");
    assert!(mate.own_seen.is_none(), "still a mesh copy");
    assert!(
        !mate.known_dead,
        "a share updated the copy and carried the kill with it"
    );
    assert_eq!(target_of(&arena, RED2), Some(BLUE));

    // It lets go when the track expires, and not one millisecond sooner: the
    // sharer's own sighting is what the copy's stamp is, and a wreck is never
    // sighted again (`sees_wrecks: false`).
    run(&mut arena, at + 6_000, &mut bytes);
    assert_eq!(hold_on(&arena, RED2, BLUE), None, "the track expired");
    assert_eq!(target_of(&arena, RED2), None);
    assert_eq!(state_of(&arena, RED2), Some(ReflexState::Patrol));
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
/// `e2_b45_a_share_carries_the_track_and_never_the_kill` is the test of what the
/// share carries; this is the test that it carries it at every cadence.
fn a_kill_under_a_withdraw_order() -> Arena {
    let mut arena = arena_of(
        vec![
            with_mesh(
                tank(
                    RED,
                    Team::Red,
                    centre(8, 8),
                    vec![cannon()],
                    Controller::Reflex,
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
                    Controller::Reflex,
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
    );
    let squad = arena
        .squad_of(RED)
        .map(|roster| roster.id)
        .expect("red's pair is a squad");
    let told = tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Withdraw,
        target: None,
        horizon_ms: 30_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 25_000,
        issued_ms: 0,
        issuer: None,
        spot: None,
    };
    for unit in [RED, RED2] {
        arena.reflex.set_order(
            unit,
            tacsim::deliberation::StandingOrder::accepted(told.clone(), 0),
        );
    }
    arena
}

#[test]
fn e2_a_kill_under_an_expiring_withdraw_order_is_one_log_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_of(cadence, 40_000, a_kill_under_a_withdraw_order, no_script))
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

/// The arena millisecond the wingman's choice is read at: a multiple of all
/// three cadences, so every replay has a pulse that ends there.
const Y1_READ_AT_MS: i64 = 5_000;

/// **A pilot points their wingman at the tank the machine would not have
/// chosen.**
///
/// Red's squad is a pair of machines: the commander (`RED`, the lowest id) out
/// on the western edge holding nothing at all, and the wingman (`RED2`) holding
/// **two** classified hostiles on row 8 east of it — `BLUE2` two cells off
/// showing its rear, and `BLUE` four cells off showing its front, which the
/// utility score ranks last on both terms. The seated red tank (`Y1_PILOT`)
/// holds `BLUE` from four cells south of it, and nothing it can see ever
/// leaves its sight, so its contact id stays good for the whole replay.
///
/// Blue's two tanks are cold and seated, so neither moves; the pair's cannon
/// never fires because the order holds fire, so neither target is ever shot
/// out from under the choice this replay is about.
fn y1_a_wingman_holding_two_hostiles() -> Arena {
    let mut hinted = tank(
        BLUE,
        Team::Blue,
        centre(11, 8),
        Vec::new(),
        Controller::Pilot,
    );
    // Facing west, at the wingman: its front.
    hinted.pose.heading_mdeg = 270_000;
    let mut other = tank(
        BLUE2,
        Team::Blue,
        centre(9, 8),
        Vec::new(),
        Controller::Pilot,
    );
    // Facing east, away from the wingman: its rear.
    other.pose.heading_mdeg = 90_000;
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(1, 1),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                Y1_PILOT,
                Team::Red,
                centre(11, 12),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                RED2,
                Team::Red,
                centre(7, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            hinted,
            other,
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[Y1_PILOT, BLUE, BLUE2],
    );
    plant_contact(&mut arena, RED2, BLUE, Team::Blue, 0);
    plant_contact(&mut arena, RED2, BLUE2, Team::Blue, 0);
    plant_contact(&mut arena, Y1_PILOT, BLUE, Team::Blue, 0);
    arena
}

/// **One scripted replay of a pilot's `ENGAGE` order**, delivered at beat 0 —
/// the opening pulse, whose phase 2 runs at arena millisecond 0 at every
/// cadence — as the world delivers one: an [`IntentKind::Order`] per unit of
/// the squad, on each unit's own `helm` epoch. `issuer` is the order's
/// [`tacsim::deliberation::Order::issuer`]; the hint is always the **pilot's**
/// contact id on `BLUE`.
///
/// Answers the replay and the wingman's target at [`Y1_READ_AT_MS`].
fn y1_replay(cadence_ms: i64, until_ms: i64, issuer: Option<ObjId>) -> (Replay, Option<ObjId>) {
    let mut arena = y1_a_wingman_holding_two_hostiles();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let squad = arena
        .squad_of(RED2)
        .map(|roster| roster.id)
        .expect("red's pair is a squad");
    assert_eq!(
        arena.squad_of(RED2).map(|roster| roster.commander),
        Some(RED),
        "the commander is the lowest id, and it is not the wingman"
    );
    assert!(
        hold_on(&arena, RED, BLUE).is_none(),
        "the commander holds nothing on the hinted tank"
    );
    let hint = draw_contact_id(SEED, Y1_PILOT, BLUE);
    let told = tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Engage {
            target_hint: Some(hint),
        },
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints {
            hold_fire: true,
            max_range_band: None,
        },
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer,
        spot: None,
    };
    let beat: Vec<tacsim::arena::intents::Intent> = [RED, RED2]
        .into_iter()
        .zip(1_u64..)
        .map(|(unit, seq)| tacsim::arena::intents::Intent {
            unit,
            channel: Channel::Helm,
            epoch: arena.epochs.current(unit, Channel::Helm),
            kind: tacsim::arena::intents::IntentKind::Order(Box::new(told.clone())),
            seq,
        })
        .collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    let mut read: Option<Option<ObjId>> = None;
    loop {
        let intents: &[tacsim::arena::intents::Intent] = if pulses == 0 { &beat } else { &[] };
        let out = arena.step(wall, &[], intents);
        pulses = pulses.saturating_add(1);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms == Y1_READ_AT_MS {
            read = Some(target_of(&arena, RED2));
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
    (
        Replay {
            events,
            hashes,
            finals,
            pulses,
        },
        read.expect("a pulse ended at the reading's millisecond"),
    )
}

/// **16b cut Y.1 (16b §11: mandatory)**: a pilot's `ENGAGE` with a hint is one
/// log at three cadences, and the hint is resolved on **the issuer's** scope.
///
/// At 5,000, 1,000 and 500 ms the event list — the two `ordered` lines at
/// arena millisecond 0 among it — the hash sequence (which now folds the
/// issuer) and the field's end are one. The wingman, holding both hostiles,
/// takes `BLUE`, which its own score ranks last: the pilot's contact id named
/// it, resolved on the pilot's store into an object, and the wingman found its
/// own contact on that object. **And an otherwise identical replay with `issuer:
/// None` takes `BLUE2`**: resolved on the commander's store, the pilot's id
/// names nothing (contact ids are drawn per viewer), so the wingman chooses on
/// its own score. The test fails if the issuer is ignored or resolved on the
/// commander, and if it is resolved on the wingman itself.
#[test]
fn y1_a_pilots_order_with_an_issuer_hint_is_one_log_at_three_cadences() {
    let cadences = [5_000_i64, 1_000, 500];
    let runs: Vec<(Replay, Option<ObjId>)> = cadences
        .into_iter()
        .map(|cadence| y1_replay(cadence, 10_000, Some(Y1_PILOT)))
        .collect();
    let (first, chosen) = runs.first().expect("three replays");
    let ordered: Vec<&(i64, String, u64)> = first
        .events
        .iter()
        .filter(|(_, kind, _)| kind.contains("\"kind\":\"ordered\""))
        .collect();
    assert_eq!(
        ordered.len(),
        2,
        "one line per unit of the squad: {ordered:?}"
    );
    assert!(
        ordered.iter().all(|(t_ms, kind, hash)| *t_ms == 0
            && kind.contains("\"issuer\":5")
            && *hash == ordered[0].2),
        "at beat 0, the issuer on the line, one phase's hash: {ordered:?}"
    );
    assert_eq!(
        *chosen,
        Some(BLUE),
        "the wingman took the tank the pilot pointed at"
    );
    assert!(first.hashes.len() >= 10, "{} substeps", first.hashes.len());
    assert!(
        runs.iter()
            .map(|(run, _)| run.pulses)
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "three cadences, three different numbers of pulses"
    );
    for ((run, target), cadence) in runs.iter().zip(cadences).skip(1) {
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
        assert_eq!(*target, *chosen, "the choice diverged at {cadence} ms");
    }

    // The same replay with the order's issuer left to the squad's commander.
    let (_, commanders) = y1_replay(1_000, 10_000, None);
    assert_eq!(
        commanders,
        Some(BLUE2),
        "resolved on the commander's scope the pilot's id names nothing, and the \
         wingman chose on its own score"
    );
}

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
fn an_unreachable_order_and_a_hot_gun() -> Arena {
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
        &a_pocket(),
        &arena_cfg(reflex_cfg()),
        &[BLUE],
    );
    heated(&mut arena, RED, 60, 50, 20);
    let squad = arena
        .squad_of(RED)
        .map(|roster| roster.id)
        .expect("one reflex machine is a squad of one");
    let told = tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Advance { to: POCKET },
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer: None,
        spot: None,
    };
    arena
        .reflex
        .set_order(RED, tacsim::deliberation::StandingOrder::accepted(told, 0));
    arena
}

#[test]
fn e2_an_unreachable_order_and_a_hot_gun_are_one_log_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                20_000,
                an_unreachable_order_and_a_hot_gun,
                no_script,
            )
        })
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in [
        "startup",
        "detected",
        "classified",
        "fired",
        "order_expired",
    ] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}, so nothing was compared: {kinds:?}"
        );
    }
    // `B-41`'s line and not an expiry: the order was written for ten minutes and
    // is over inside the first two seconds, at the tick whose search failed.
    let dropped_at = first
        .events
        .iter()
        .find(|(_, kind, _)| kind.contains("order_expired"))
        .map(|(t_ms, _, _)| *t_ms)
        .expect("a line");
    assert!(
        dropped_at < 2_000,
        "the drop waited for the time to live: {dropped_at} ms"
    );
    // `T-55`'s shot: the machine takes the one the engine would take, and the
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
// 16b cut E.3 — driving (16c `B-42`, `B-43`, `B-44`, `B-46`; §5's Q10)
// ---------------------------------------------------------------------------

/// **A one-cell-wide causeway**: walls over columns 6 to 9 on every row but 8, so
/// the only way west is a four-cell corridor a column has to file through.
///
/// It is the ford of the shipped proving ground in fixture form — the one place
/// every solo match crosses, and the one `B-46` is about.
fn causeway() -> Vec<(u16, u16, TerrainCell)> {
    (0..SIDE)
        .flat_map(|y| (6..10_u16).map(move |x| (x, y, wall())))
        .filter(|(_, y, _)| *y != 8)
        .collect()
}

/// A window whose patrol graph is **one node**, so every machine on it drives the
/// same way rather than splitting at the nearest waypoint.
fn one_way_west(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    definition_with(
        paint,
        Waypoints::new(
            vec![Waypoint {
                id: SmolStr::new("west"),
                x: 1,
                y: 8,
            }],
            &[],
        ),
    )
}

/// **A column of `count` machines nose to tail on row 8, east of the causeway,
/// with the lowest id at the REAR** — which is the worst case for the release
/// (16b cut E.3).
///
/// Units integrate in unit-id order, so a follower whose id sorts *below* its
/// leader's integrates before the leader has moved and reaches a cell the leader
/// is still standing in. Putting the lowest id at the rear therefore makes every
/// follower meet its leader's cell, one substep per unit down the column: the
/// case the hold exists for, and the case the parked-head measurement below is
/// about.
fn column_of(count: u16) -> Arena {
    let units: Vec<CustodyUnit> = (0..count)
        .map(|n| {
            // `n` is the place in the column from the head: head at (10, 8), rear
            // furthest east, and the id falls as the place rises.
            tank(
                in_column(n),
                Team::Red,
                centre(10 + n, 8),
                Vec::new(),
                Controller::Reflex,
            )
        })
        .collect();
    arena_on(
        one_way_west(&causeway()),
        units,
        &arena_cfg(reflex_cfg()),
        &[],
    )
}

/// The machine `n` places behind the head of a [`column_of`].
fn in_column(n: u16) -> ObjId {
    ObjId(200 - u32::from(n))
}

/// **`B-46`: a follower at the ford is held and never stopped** (16b cut E.3).
///
/// Two machines nose to tail on the causeway, the follower with the lower id so
/// that it integrates first and meets the cell its leader has not vacated. Before
/// this cut it was stopped dead at every cell of the corridor, journalled a
/// `blocked` line for each, was told `Held: something is in the way.` and taught
/// its own machine to avoid the cell its leader was about to leave.
#[test]
fn a_follower_at_the_ford_is_held_and_journals_nothing() {
    let mut arena = column_of(2);
    let (head, follower) = (in_column(0), in_column(1));
    assert!(follower.0 < head.0, "the follower must integrate first");
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut stalled: Vec<i64> = Vec::new();
    let mut rolling = false;
    let mut learned: Vec<(u16, u16)> = Vec::new();
    // **The window is the crossing, and it ends when the head arrives**: once the
    // head has stopped on the node they are both driving at, the follower is
    // standing behind a machine that is not going to move again, and doc 16 §3.4
    // step 4's stop is exactly what it is owed. What the cut is about is the
    // ford, not the destination.
    let mut arrived_at: Option<i64> = None;
    while arena.now_ms < 30_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let head_speed = arena.unit(head).map_or(0, |unit| unit.speed_cms);
        let follower_speed = arena.unit(follower).map_or(0, |unit| unit.speed_cms);
        rolling |= follower_speed > 0;
        if arrived_at.is_none() && arena.unit(head).is_some_and(|unit| unit.pose.x_cm < 3_000) {
            arrived_at = Some(arena.now_ms);
        }
        if arrived_at.is_none() {
            for cell in avoided(&arena, follower) {
                if !learned.contains(&cell) {
                    learned.push(cell);
                }
            }
            if rolling && head_speed > 0 && follower_speed == 0 {
                stalled.push(arena.now_ms);
            }
        }
        wall += 500;
    }
    let arrived_at = arrived_at.expect("the head crossed the causeway and reached its node");
    let log = events(&mut arena, &mut bytes);
    assert!(
        stalled.is_empty(),
        "the follower lost its way while the leader was still moving, at {stalled:?} ms"
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "blocked" && event.t_ms <= arrived_at)
            .count(),
        0,
        "a column that never stopped journalled a stop before the head arrived"
    );
    assert!(
        learned.is_empty(),
        "the follower learned to avoid a cell its own leader was standing in: {learned:?}"
    );
    assert!(
        arena
            .unit(follower)
            .is_some_and(|unit| unit.pose.x_cm < 6_000),
        "the follower never got through the causeway: {:?}",
        arena.unit(follower).map(|unit| unit.pose.x_cm)
    );
}

/// **The long column** (16b cut E.3, review E-04): six machines nose to tail on
/// the causeway, the lowest id at the rear, the head free to move.
///
/// Two seconds of hold covers a release that travels rearward one substep per
/// unit, and six is inside the shipped roster's four a side either way. Nothing
/// is stopped, nothing is journalled and nobody learns a teammate's transient
/// cell.
#[test]
fn a_six_machine_column_files_through_the_ford_learning_nothing() {
    let mut arena = column_of(6);
    let ids: Vec<ObjId> = (0..6).map(in_column).collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut rolling = false;
    let mut stalled: Vec<(i64, usize)> = Vec::new();
    let mut arrived_at: Option<i64> = None;
    while arena.now_ms < 60_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let speeds: Vec<i32> = ids
            .iter()
            .map(|id| arena.unit(*id).map_or(0, |unit| unit.speed_cms))
            .collect();
        rolling |= speeds.iter().all(|speed| *speed > 0);
        if arrived_at.is_none()
            && arena
                .unit(ids[0])
                .is_some_and(|unit| unit.pose.x_cm < 3_000)
        {
            arrived_at = Some(arena.now_ms);
        }
        if rolling && arrived_at.is_none() {
            for (place, speed) in speeds.iter().enumerate() {
                if *speed == 0 {
                    stalled.push((arena.now_ms, place));
                }
            }
        }
        wall += 500;
    }
    let arrived_at = arrived_at.expect("the head crossed the causeway and reached its node");
    let log = events(&mut arena, &mut bytes);
    assert!(
        stalled.is_empty(),
        "machines in the column lost their way before the head arrived: {stalled:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "blocked" && event.t_ms <= arrived_at)
            .count(),
        0,
        "the column journalled a stop before the head arrived"
    );
    // The corridor is (6, 8) to (9, 8) and the column's own cells are (10, 8)
    // east: no machine ever learned one of them.
    for id in &ids {
        let learned = avoided(&arena, *id);
        assert!(
            learned
                .iter()
                .all(|(x, y)| !(*y == 8 && (6..SIDE).contains(x))),
            "#{} learned a cell a teammate was standing in: {learned:?}",
            id.0
        );
    }
}

/// **The parked head, MEASURED and not asserted** (16b cut E.3, review E-04).
///
/// The same six, with the head held where it stands for 5,000 ms of arena time
/// and then released. Two seconds of hold is clean for a column of up to eight
/// being released; a column whose head is itself stopped for longer than that
/// matures its tail into stops, and each of those teaches a teammate's transient
/// cell for `REFLEX_AVOID_TTL_MS`. That is HEAD's behaviour delayed and made
/// rare, not removed, and the three numbers this prints are the evidence for or
/// against the second-stop rule 16b cut E.3 deliberately did not build.
#[test]
fn e3_a_parked_head_is_measured_and_reported() {
    let mut arena = column_of(6);
    let ids: Vec<ObjId> = (0..6).map(in_column).collect();
    let head = ids[0];
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    // (1) the column gets under way and files into the corridor.
    while arena.now_ms < 8_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        wall += 500;
    }
    // (2) the head stops where it stands for five seconds — a pilot's tank in
    // the ford, which is longer than the hold and so compresses the column into
    // real stops.
    let parked_from = arena.now_ms;
    arena.set_controller(head, Controller::Pilot, ReflexState::Patrol);
    while arena.now_ms < parked_from + 5_000 {
        if let Some(unit) = arena.unit_mut(head) {
            unit.desired_speed_cms = 0;
        }
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        wall += 500;
    }
    let released_at = arena.now_ms;
    arena.set_controller(head, Controller::Reflex, ReflexState::Patrol);
    let mut through: Option<i64> = None;
    // Every teammate's cell any of the six ever remembered, over the whole
    // episode: the entries age out after `REFLEX_AVOID_TTL_MS`, so a reading
    // taken at the end would under-count.
    let mut taught: Vec<(ObjId, (u16, u16))> = Vec::new();
    while arena.now_ms < released_at + 60_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        // Only while the release is still travelling down the column: past that
        // the six converge on one node and queue at the destination, which is
        // doc 16 §3.4 step 4 doing its job and not what this measures.
        if arena.now_ms <= released_at + 10_000 {
            for id in &ids {
                for cell in avoided(&arena, *id) {
                    if !taught.contains(&(*id, cell)) {
                        taught.push((*id, cell));
                    }
                }
            }
        }
        if through.is_none()
            && ids
                .iter()
                .all(|id| arena.unit(*id).is_some_and(|unit| unit.pose.x_cm < 6_000))
        {
            through = Some(arena.now_ms);
        }
        wall += 500;
    }
    let log = events(&mut arena, &mut bytes);
    let matured = log
        .iter()
        .filter(|event| {
            event.kind.as_str() == "blocked"
                && (parked_from..=released_at + 10_000).contains(&event.t_ms)
        })
        .count();
    let learners: usize = ids
        .iter()
        .filter(|id| taught.iter().any(|(who, _)| who == *id))
        .count();
    println!(
        "e3_parked_head: parked at {parked_from} ms, released at {released_at} ms; \
         {matured} stops matured between the park and ten seconds past the release, \
         {learners} of 6 machines learned a cell in that window ({taught:?}), the last was \
         through {} ms after the release",
        through.map_or(-1, |at| at - released_at)
    );
    assert!(
        through.is_some(),
        "the column never cleared the causeway after the release"
    );
}

/// **`B-46` at a wreck**: the machine learns the cell two seconds later and takes
/// the other gap, which is
/// `a_wreck_in_the_causeway_is_learned_and_the_other_gap_is_taken` with the hold
/// in front of it (16b cut E.3).
#[test]
fn a_wreck_in_the_ford_is_held_against_for_two_seconds_and_then_learned() {
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
    let mut wall = 0_i64;
    let mut met_at: Option<i64> = None;
    let mut west_most = i64::MAX;
    // The entries age out after `REFLEX_AVOID_TTL_MS`, so what the machine
    // learned is the union over the run and not a reading taken at the end.
    let mut learned: Vec<(u16, u16)> = Vec::new();
    while arena.now_ms < 60_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        // The first pulse that ended with the machine standing against a cell.
        if met_at.is_none()
            && arena
                .unit(RED)
                .is_some_and(|unit| unit.occupied_hold.is_some())
        {
            met_at = Some(arena.now_ms);
        }
        for cell in avoided(&arena, RED) {
            if !learned.contains(&cell) {
                learned.push(cell);
            }
        }
        west_most = west_most.min(arena.unit(RED).map_or(i64::MAX, |unit| unit.pose.x_cm));
        wall += 500;
    }
    let met_at = met_at.expect("the machine never met the wreck");
    let log = events(&mut arena, &mut bytes);
    let stops: Vec<i64> = log
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Blocked { unit, x, y, .. }
                    if unit == RED && (x, y) == (4, 8)
            )
        })
        .map(|event| event.t_ms)
        .collect();
    assert_eq!(
        stops.len(),
        1,
        "one wreck is one stop, and the hold does not add another: {stops:?}"
    );
    assert!(
        stops[0] >= met_at + 1_500,
        "the stop arrived {} ms after the machine was first read standing against the wreck, \
         which is inside the two-second hold (the hold is read at the end of the pulse it opened \
         in, so the margin a 500 ms pulse can promise is a pulse short of 2,000)",
        stops[0] - met_at
    );
    assert_eq!(
        learned,
        vec![(4, 8)],
        "one remembered cell, and it is the wreck's"
    );
    assert!(
        west_most < 4_000,
        "it is through the other gap: west-most {west_most} cm"
    );
}

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
    }
}

/// **`B-42`: a machine brakes for its goal instead of driving through it** (16b
/// cut E.3).
///
/// Forty cells of road and a tank with the shipped ratio — fifteen metres a
/// second, two metres a second squared of brake, which is fifty-six metres and
/// seven and a half seconds to stop. Before this cut the controller ordered
/// `max_speed_cms` flat until the unit was inside the one-cell arrive window and
/// then ordered zero, so the machine coasted five cells past everywhere it meant
/// to be.
#[test]
fn a_machine_brakes_into_its_goal_and_does_not_overshoot_it() {
    let mut heavy = tank(
        RED,
        Team::Red,
        centre(44, 4),
        Vec::new(),
        Controller::Reflex,
    );
    heavy.mobility = TrackedRules {
        max_speed_cms: 1_500,
        accel_cms2: 100,
        brake_cms2: 200,
        turn_rate_mdeg_s: 30_000,
        reverse_factor_pct: 40,
    };
    // **16b cut E.4**: the hunt is off. This claim is about braking into one
    // goal and nothing else, and the run is 180 s: with the shipped 90 s clock
    // the machine, idle on its node and holding nothing, would cross to Blue's
    // zone at the east end of the road and come back, and "standing on it at
    // the end" would be a second arrival the test knows nothing about, held by
    // a few seconds of margin.
    let mut arena = arena_on(
        road(48, (4, 4)),
        vec![heavy],
        &arena_cfg(ReflexConfig {
            hunt_after_ms: 0,
            ..reflex_cfg()
        }),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut at_arrival: Option<i32> = None;
    let mut west_most = u16::MAX;
    let mut top = 0_i32;
    while arena.now_ms < 180_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let Some(unit) = arena.unit(RED) else {
            break;
        };
        top = top.max(unit.speed_cms);
        if let Some((x, y)) = arena.definition.terrain.cell_of(&unit.pose) {
            west_most = west_most.min(x);
            // The tick it first reads arrived: inside `ARRIVE_CELLS` of (4, 4).
            if at_arrival.is_none() && x.abs_diff(4).max(y.abs_diff(4)) <= 1 {
                at_arrival = Some(unit.speed_cms);
            }
        }
        wall += 500;
    }
    let at_arrival = at_arrival.expect("the machine never reached its goal");
    println!(
        "e3_braking: top {top} cm/s, {at_arrival} cm/s on the tick it first read arrived, \
         west-most cell {west_most}"
    );
    assert!(
        top >= 1_400,
        "the machine never got up to speed, so nothing was measured: {top} cm/s"
    );
    // **The measured figure is 415 cm/s**, and the number is worth being exact
    // about rather than rounding to a slogan. The profile is the maximum-braking
    // curve, `v = isqrt(2 × brake × d)`, and a tank on it sheds exactly
    // `brake_cms2` a second — which is all the tank has. The controller decides
    // twice a second, so the last tick before the arrive window still carries up
    // to a tick of the curve it cannot have shed yet. A quarter of `max_speed`
    // at the window, against the whole of it before this cut, is the claim; the
    // tank is standing still a second and a half later, which the last assertion
    // is.
    assert!(
        at_arrival <= 500,
        "it arrived at {at_arrival} cm/s, which is not braking for the goal"
    );
    assert!(
        west_most >= 3,
        "it overshot its goal at (4, 4) to cell {west_most}, past the arrive window"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.speed_cms),
        Some(0),
        "and it is standing on it at the end"
    );
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

/// **`B-42`'s other half: a withdrawn machine's rally clock is not restarted by
/// an overshoot** (16b cut E.3).
///
/// A hurt machine runs for its entry zone. Once it has arrived and has nothing
/// hostile on its board the quiet clock starts; a machine that drove through its
/// own entry zone and out the far side would stop being *arrived*, and the clock
/// would start again from nothing every time it came back.
///
/// **The fixture has to make the overshoot possible, or it proves nothing.** This
/// file's default tank stops in 1,000 cm — one cell, which is the arrive window —
/// and its entry zone is against the west wall, so before this cut it could not
/// leave the window whatever it did, and a test built on it would stay green with
/// the braking profile reverted. So: the road window with Red's zone eight cells
/// in from the wall, and the shipped ratio of 1,500 cm/s against 200 cm/s² of
/// brake — 5.6 cells of stopping distance against a one-cell window. Ordering
/// `max_speed_cms` flat until the window and zero inside it, which is what the
/// controller did before this cut, puts the tank several cells past its own zone
/// and starts the clock again when it comes back.
#[test]
fn a_withdrawn_machines_rally_clock_is_not_restarted_by_an_overshoot() {
    let mut heavy = tank(
        RED,
        Team::Red,
        centre(40, 4),
        vec![cannon()],
        Controller::Reflex,
    );
    heavy.mobility = TrackedRules {
        max_speed_cms: 1_500,
        accel_cms2: 100,
        brake_cms2: 200,
        turn_rate_mdeg_s: 30_000,
        reverse_factor_pct: 40,
    };
    let mut arena = arena_on(
        road_home(48, 8, (44, 4)),
        vec![heavy],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    hurt(&mut arena, RED);
    let mut wall = arena.now_ms;
    let mut started: Option<i64> = None;
    let mut restarts: Vec<(i64, i64)> = Vec::new();
    let mut withdrew = false;
    let mut west_most = u16::MAX;
    while arena.now_ms < 90_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        let quiet = arena.reflex.unit(RED).and_then(|row| row.quiet_since_ms);
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
        let running = state_of(&arena, RED) == Some(ReflexState::Withdraw);
        withdrew |= running;
        // The rally is what ends the run home, and this test is about the clock
        // that leads to it: stop once the machine has come back.
        if withdrew && !running {
            break;
        }
        wall += 500;
    }
    println!("e3_rally: west-most cell {west_most}, clock started at {started:?}");
    assert!(
        withdrew,
        "the machine never broke off, so nothing was measured"
    );
    assert!(
        started.is_some(),
        "the machine never arrived home, so the clock never started"
    );
    assert!(
        restarts.is_empty(),
        "the rally clock was restarted after the machine had arrived: {restarts:?}"
    );
    // And the reason it was not: the machine braked into its zone instead of
    // driving through it. The goal is the clamped corner `(9, 4)` and the arrive
    // window is a cell wide, so cell 8 is the west edge of *home*.
    assert!(
        west_most >= 8,
        "it drove out of its own entry zone to cell {west_most}, which is what restarts the clock"
    );
}

/// **The convoy and the wreck, at three cadences** (16b cut E.3; 16b §11's
/// mandatory cross-tempo test).
///
/// One scripted log in which a follower holds behind its leader and is released,
/// the leader is then wrecked in front of it, the follower holds against the
/// wreck, matures into a stop, learns the cell and detours round it. One event
/// list and one per-substep hash sequence at 5,000, 1,000 and 500 ms — which is
/// the whole of the rate-independence claim for the hold, because all three
/// cadences are whole multiples of the 250 ms substep and therefore share one
/// partition.
fn convoy_and_wreck() -> Arena {
    let band: Vec<(u16, u16, TerrainCell)> = (0..SIDE)
        .filter(|y| *y != 8 && *y != 2)
        .map(|y| (4_u16, y, wall()))
        .collect();
    arena_on(
        one_way_west(&band),
        vec![
            // The follower sorts below the leader, so it integrates first.
            tank(RED, Team::Red, centre(7, 8), Vec::new(), Controller::Reflex),
            tank(
                RED2,
                Team::Red,
                centre(6, 8),
                Vec::new(),
                Controller::Reflex,
            ),
        ],
        &arena_cfg(reflex_cfg()),
        &[],
    )
}

/// The leader is wrecked where it stands, so the follower behind it meets a hole
/// in the map that does not move.
fn the_leader_is_wrecked(arena: &mut Arena) {
    wreck_it(arena, RED2);
}

#[test]
fn the_convoy_and_the_wreck_are_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| replay_of(cadence, 60_000, convoy_and_wreck, the_leader_is_wrecked))
        .collect();
    let first = runs.first().expect("three replays");
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    assert!(
        kinds.iter().any(|kind| kind.contains("blocked")),
        "the follower never matured a stop against the wreck, so nothing was compared: {kinds:?}"
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

/// **`B-44`: a routeless machine is checked every substep, not every tick** (16b
/// cut E.3).
///
/// The case `follow` cannot see, because it happens between two of its ticks. A
/// stop throws the machine's route away and puts the refused cell on its
/// remembered list, both at the substep the stop happened on — and the machine is
/// then routeless, with a goal, with a standing speed order and pointed straight
/// at a cell it has just learned to avoid. Until this cut it re-accelerated and
/// pushed at that cell for the rest of the decide tick; now the pre-pass at the
/// top of the next substep orders `Speed 0` through the same door a pilot's own
/// order goes through, and the machine stands until its next tick can plan.
///
/// The measure is the arena time between the `blocked` line and the speed order
/// coming off: one substep, not one `decide_ms`. The decide grid is printed and
/// asserted with it, so that the reading cannot be a tick that happened to fall
/// inside the substep — this is the fixture 16c's `B-44` row is closed on, and
/// `arena::reflex`'s own
/// `e3_the_pre_pass_holds_a_routeless_machine_and_skips_everyone_else` is what
/// holds the pass's five guards.
#[test]
fn a_routeless_machine_is_ordered_to_stop_within_one_substep_of_a_stop() {
    // A whole second between decisions, so "the next tick" and "the next substep"
    // are four substeps apart and the difference is visible.
    let cfg = arena_cfg(ReflexConfig {
        decide_ms: 1_000,
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
    let mut wall = 0_i64;
    // Pulses of 250 ms, so every pulse is exactly one substep and the reading is
    // at the granularity the pre-pass runs at.
    let mut stopped_at: Option<i64> = None;
    let mut ordered_off_at: Option<i64> = None;
    let mut intents_on_the_order = 0_u32;
    while arena.now_ms < 30_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        // The stop is read off the unit's own per-substep flag rather than off the
        // ledger, because a 250 ms pulse is one substep and the ledger's chunks
        // are flushed on their own cadence.
        if stopped_at.is_none() {
            if arena.unit(RED).is_some_and(|unit| unit.blocked) {
                stopped_at = Some(arena.now_ms);
            }
        } else if ordered_off_at.is_none()
            && arena
                .unit(RED)
                .is_some_and(|unit| unit.desired_speed_cms == 0)
        {
            ordered_off_at = Some(arena.now_ms);
            intents_on_the_order = out.reflex.intents;
        }
        if ordered_off_at.is_some() {
            break;
        }
        wall += 250;
    }
    let stopped_at = stopped_at.expect("the parked tank stopped the machine");
    let ordered_off_at = ordered_off_at.expect("the machine was never ordered to stand");
    // **The decide grid, so that the 250 ms reading cannot be a coincidence.** A
    // unit's ticks are at its stagger — `stagger_ms(seed, id, decide_ms)` since
    // 16b cut U.1, `id % decide_ms` before it — and every `decide_ms` after it
    // (`Arena::schedule_reflex_tick`), so this is exactly the arena time at which
    // `follow` could have ordered the same speed on its own.
    let grid = |at: i64| (at - stagger_ms(SEED, RED, 1_000)).rem_euclid(1_000);
    println!(
        "e3_b44: stopped at {stopped_at} ms, ordered off at {ordered_off_at} ms, \
         {} ms past that pulse's decide tick, {intents_on_the_order} intents on the pulse",
        grid(ordered_off_at)
    );
    assert!(
        ordered_off_at - stopped_at <= 250,
        "the machine kept its speed order for {} ms after it was stopped, which is the decide \
         tick and not the substep: stopped at {stopped_at} ms, ordered off at {ordered_off_at} ms",
        ordered_off_at - stopped_at
    );
    assert_ne!(
        grid(ordered_off_at),
        0,
        "the order came off on a decide tick, where `follow` would have ordered it too: this \
         fixture is not measuring the pre-pass"
    );
    assert!(
        intents_on_the_order > 0,
        "the pulse the order came off on counted no intent, so nothing went through `apply_intent`"
    );
    // And the order came through the pilot's own door: the controller counted it.
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.route.is_empty() && row.goal.is_some()),
        "the machine was not routeless with a goal, so this is not `B-44`'s case"
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

/// `Ranges::engage_cm` at [`E4_SIGHT_CELLS`]: seven tenths of 7,000 cm.
const E4_ENGAGE_CM: i64 = 4_900;

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

/// A machine that sees seven cells and carries the cannon, against a parked
/// pilot tank, on `paint`ed ground under `cfg`.
fn e4_field(
    red: (u16, u16),
    blue: (u16, u16),
    paint: &[(u16, u16, TerrainCell)],
    cfg: &ArenaConfig,
) -> Arena {
    e4_pinned(arena_of(
        e4_units(red, blue, vec![cannon()]),
        paint,
        cfg,
        &[BLUE],
    ))
}

/// **A fixture on one tempo** (16b cut E.4).
///
/// What makes a pulse of these tests one decide tick is **not** this pin: a
/// pulse's `dt` is the wall interval (`Clock::advance`, clamped only by
/// `max_catch_up_steps` periods of the current tempo, which every tempo's
/// period clears), and [`run`], [`e4_run_until_state`] and
/// [`e4_run_until_hunt`] all step the wall 500 ms at a time — which is
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
    arena.reflex.unit(unit).and_then(|row| row.goal)
}

/// The cell it last saw something in, and when.
fn last_plot_of(arena: &Arena, unit: ObjId) -> Option<((u16, u16), i64)> {
    arena.reflex.unit(unit).and_then(|row| row.last_plot)
}

/// The far entry cell it is crossing to.
fn hunt_of(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena.reflex.unit(unit).and_then(|row| row.hunt)
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
    state: ReflexState,
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

/// **Step until `unit` sets a hunt**, answering the cell it was standing in and
/// the cell it chose.
///
/// Both are needed together and neither afterwards: the hunt's cell is the
/// clamped corner of the far zone **on the machine's own row**, and a machine
/// walking its patrol changes rows while the clock runs, so a test that read
/// the cell at the end of a run would be asserting against a row the machine
/// was not standing on when it chose.
fn e4_run_until_hunt(
    arena: &mut Arena,
    unit: ObjId,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> Option<((u16, u16), (u16, u16))> {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        if let Some(cell) = hunt_of(arena, unit) {
            let here = arena
                .unit(unit)
                .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
            return here.map(|here| (here, cell));
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    None
}

// ---------------------------------------------------------------------------
// Fix A — the search
// ---------------------------------------------------------------------------

/// **A machine that loses its contact drives to where it last saw it** (16b cut
/// E.4; 16c `B-04`), and not to its waypoint.
///
/// Three claims in one run, because they are three moments of one arc:
///
/// * the goal after the `lost` edge is [`ReflexUnit::last_plot`]'s cell and not
///   a node of the graph;
/// * **the observer rule from the machine's side**: the target is moved again
///   *after* its contact has expired, and the goal does not follow it. The
///   search is the machine's own memory of a plot, never a reading of where
///   anything now is (doc 16 §1, 16b cut F.3's rule read once more);
/// * on **arrival** `last_plot` is cleared and the next goal is a waypoint — it
///   went and looked, and there was nothing there. Read at the pulse the clear
///   happens and not at the end of a run, with the machine's cell and the
///   plot's age asserted, so an arrival cannot be mistaken for an age-out.
///
/// The fourth moment of the arc — a hostile found on the way — is
/// [`e4_a_hostile_found_while_searching_pre_empts_it`].
#[test]
fn e4_a_lost_contact_is_searched_for_where_it_was_last_seen() {
    // Twenty seconds of search, no hunt: this test is about the first of the two
    // clocks and nothing else.
    //
    // **16b cut M.2** (16c `T-02`): the machine starts at (1, 8), not (2, 8). It
    // starts up at its own tick now rather than at the substep's end, so it sets
    // off a decide tick sooner and halts in `Engage` five metres further east —
    // at (4, 8) from (2, 8), one cell from the plot, which the guard below reads
    // as an arrival. From (1, 8) it halts at (3, 8), the margin the test was
    // written with.
    let mut arena = e4_field((1, 8), (6, 8), &[], &e4_cfg(20_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
        "4,000 cm is inside the {E4_ENGAGE_CM} cm band, so the machine holds it"
    );
    assert_eq!(
        last_plot_of(&arena, RED).map(|(cell, _)| cell),
        Some((6, 8)),
        "and an `Engage` tick writes the plot it is holding"
    );

    // Over the hill: (15, 15) is more than seven cells from anywhere on row 8
    // between x = 2 and x = 6, so nothing refreshes the contact and the
    // time-to-live takes it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the machine back to `Patrol`"
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
    run(&mut arena, until, &mut bytes);
    let here = cell_of_unit(&arena, RED).expect("the machine is on the window");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::reflex::ARRIVE_CELLS,
        "it has already arrived at the plot, so the claim below is about nothing: {here:?}"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some((6, 8)),
        "the search is the machine's own last plot, never a reading of where the target now is"
    );

    // It arrives, finds nothing, and goes back to its patrol. Read at the pulse
    // the search is given up, so what is asserted is *why* it was given up.
    let stamped = last_plot_of(&arena, RED)
        .map(|(_, at)| at)
        .expect("the search is live");
    let until = arena.now_ms + 25_000;
    let mut wall = arena.now_ms;
    let mut cleared: Option<(i64, (u16, u16))> = None;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if last_plot_of(&arena, RED).is_none() {
            cleared = cell_of_unit(&arena, RED).map(|here| (arena.now_ms, here));
            break;
        }
        wall += 500;
    }
    let (at_ms, here) = cleared.expect("the search never ended");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) <= tacsim::arena::reflex::ARRIVE_CELLS,
        "the search was given up at {here:?}, which is not an arrival at (6, 8)"
    );
    assert!(
        at_ms - stamped < 20_000,
        "and it was given up {} ms after the plot was stamped, which is an age-out and not an          arrival",
        at_ms - stamped
    );
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "after the search the waypoint walk resumes: {goal:?}"
    );
}

/// **A hostile found on the way to the last plot pre-empts the search** (16b
/// cut E.4; the brief's search item, *a hostile detected on the way*).
///
/// The search is `Patrol`'s goal, and the hostile edges run **above** the
/// state's own body, so a machine driving to where it last saw something turns
/// on anything it finds before it gets there. The claim is made while the
/// search is live — `last_plot` still set and the machine short of it — which
/// is the only moment it says anything: after the search is over the hostile
/// edges would pre-empt a waypoint walk, and that is a different claim.
///
/// The second tank is handed to the machine's store as a classified contact
/// (this file's head says why a test may) so that the pre-emption lands on the
/// next tick rather than after `classify_after_ms`, inside which a machine
/// four cells from its plot would have arrived.
#[test]
fn e4_a_hostile_found_while_searching_pre_empts_it() {
    // 16b cut M.2: from (1, 8), for the reason
    // `e4_a_lost_contact_is_searched_for_where_it_was_last_seen` gives.
    let mut arena = e4_field((1, 8), (6, 8), &[], &e4_cfg(20_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Patrol, 20_000, &mut bytes),
        "the contact should have expired and dropped the machine back to `Patrol`"
    );
    assert_eq!(
        goal_of(&arena, RED),
        Some((6, 8)),
        "the search has the goal"
    );
    let here = cell_of_unit(&arena, RED).expect("the machine is on the window");
    assert!(
        here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::reflex::ARRIVE_CELLS,
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
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert!(
        matches!(
            state_of(&arena, RED),
            Some(ReflexState::Approach | ReflexState::Engage)
        ),
        "a hostile found on the way outranks the search: {:?}",
        state_of(&arena, RED)
    );
    assert_eq!(
        target_of(&arena, RED),
        Some(BLUE),
        "and the machine has turned on it"
    );
    assert_eq!(
        last_plot_of(&arena, RED).map(|(cell, _)| cell),
        Some((4, 12)),
        "and the plot it now remembers is the new one, not the search it gave up"
    );
}

/// **A plot older than `search_ms`, and `search_ms = 0`, are both the old `lost`
/// edge** (16b cut E.4) — the pre-E.4 edge, reproduced two ways, each read one
/// pulse at a time.
///
/// The two cases are one table because the claim is one claim: the search is a
/// clock and nothing else, and a machine whose clock has run out (or was never
/// wound) is the machine this file has always tested. What each case counts is
/// how many `Patrol` ticks drove at the plot, and where the machine stood when
/// the plot was dropped — which must not be an arrival.
#[test]
fn e4_an_aged_plot_and_a_search_switched_off_are_both_the_old_lost_edge() {
    for (search_ms, driven, why) in [
        // The validated floor, one decide tick. The plot was written by the last
        // tick that held the target and the `lost` edge is read one tick later,
        // so the first `Patrol` tick finds it exactly `search_ms` old: the age
        // is compared inclusively, so that tick drives at it — the one tick the
        // floor promises — and the next drops it, four cells short.
        (500_u32, 1_usize, "a plot aged past `search_ms`"),
        // Off: the first `Patrol` tick drops it unread.
        (0, 0, "`search_ms` switched off"),
    ] {
        // 16b cut M.2: from (1, 8), for the reason
        // `e4_a_lost_contact_is_searched_for_where_it_was_last_seen` gives.
        let mut arena = e4_field((1, 8), (6, 8), &[], &e4_cfg(search_ms, 0));
        let mut bytes: Vec<u8> = Vec::new();
        run(&mut arena, 6_000, &mut bytes);
        assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage), "{why}");
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose = centre(15, 15);
        }
        let until = arena.now_ms + 5_000;
        let mut wall = arena.now_ms;
        let mut drove = 0_usize;
        let mut dropped_at: Option<(u16, u16)> = None;
        while arena.now_ms < until {
            let out = arena.step(wall, &[], &[]);
            absorb(&out, &mut bytes);
            if state_of(&arena, RED) == Some(ReflexState::Patrol) {
                if goal_of(&arena, RED) == Some((6, 8)) {
                    drove += 1;
                }
                if dropped_at.is_none() && last_plot_of(&arena, RED).is_none() {
                    dropped_at = cell_of_unit(&arena, RED);
                }
            }
            wall += 500;
        }
        assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol), "{why}");
        assert_eq!(
            drove, driven,
            "{why}: the machine drove at the plot for {drove} ticks"
        );
        let here = dropped_at.unwrap_or_else(|| panic!("{why}: the plot was never dropped"));
        assert!(
            here.0.abs_diff(6).max(here.1.abs_diff(8)) > tacsim::arena::reflex::ARRIVE_CELLS,
            "{why}: the plot was dropped at {here:?}, which is an arrival and not an age-out"
        );
        let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
        assert!(
            waypoint_cells().contains(&goal),
            "{why}: the goal should be a waypoint, and it is {goal:?}"
        );
    }
}

/// **A last plot no route reaches costs exactly one failed search** (16b cut
/// E.4; 16c `B-41`'s lesson applied to the search).
///
/// The blue tank sits in [`a_pocket`]'s sealed corner and the machine is handed
/// a classified contact on it by hand. The contact expires; the search goal is
/// [`POCKET`], which no route reaches; the `unreachable` edge fires once, and
/// because `follow` clears `last_plot` there the `Patrol` arm cannot re-assert
/// it on the next tick. Before that clear the cost would have been one failed
/// search every `repath_ms` for the whole of `search_ms` — which is `B-41`
/// exactly, in a second place.
#[test]
fn e4_a_last_plot_behind_a_wall_costs_one_failed_search_and_is_not_re_asserted() {
    let mut arena = e4_field((6, 8), POCKET, &a_pocket(), &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 1_000;
    let closing = run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Approach),
        "the pocket is 8,600 cm off, outside the 4,900 cm band"
    );
    assert_eq!(
        last_plot_of(&arena, RED).map(|(cell, _)| cell),
        Some(POCKET),
        "an `Approach` tick writes the plot it is closing on"
    );
    assert_eq!(
        closing.path_failures, 0,
        "the stand-off cell is ordinary ground and the route to it is found"
    );

    // Now let the contact go. The search takes the goal, fails once, and never
    // asks again.
    let until = arena.now_ms + 20_000;
    let searching = run(&mut arena, until, &mut bytes);
    assert_eq!(
        searching.path_failures, 1,
        "a sealed last plot should cost exactly one failed search, and it cost {}",
        searching.path_failures
    );
    assert_eq!(
        last_plot_of(&arena, RED),
        None,
        "the `unreachable` edge gives the search up, or the `Patrol` arm re-asserts it every tick"
    );
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the waypoint walk has it: {goal:?}"
    );
}

// ---------------------------------------------------------------------------
// Fix B — the hunt
// ---------------------------------------------------------------------------

/// **A machine that has seen nobody crosses to the other side's entry zone**
/// (16b cut E.4; 16c `B-04`), and comes back to its patrol when it finds
/// nothing there.
///
/// Red stands on row 3 and Blue's zone is the two east columns, so the clamped
/// corner is (14, 3) — which is deliberately **not** a node of the waypoint
/// graph (the two nodes are (1, 8) and (14, 8)), or the test could not tell a
/// hunt from a walk.
#[test]
fn e4_a_machine_that_has_seen_nobody_crosses_to_look() {
    // Three seconds of nothing held, no search: this test is about the second
    // clock alone. The blue tank sits in the far corner, out of the seven-cell
    // sight from every cell of the crossing, so what ends the hunt is arrival
    // and not a contact.
    let mut arena = e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    // Read one pulse at a time: the tick the clock first reads, and the pulse
    // the hunt is set on, so the claim is *at* `hunt_after_ms` and not merely
    // *by* the end of a run. A clock seeded at check-out, or one that fired
    // early, would pass a test that only waited for the crossing.
    let mut wall = arena.now_ms;
    let mut started: Option<i64> = None;
    // The pulse the hunt was set on, the cell the machine stood in, the cell
    // it chose.
    type Chosen = (i64, (u16, u16), (u16, u16));
    let mut chosen: Option<Chosen> = None;
    while arena.now_ms < 20_000 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if started.is_none() {
            started = arena.reflex.unit(RED).and_then(|row| row.unheld_since_ms);
        }
        if let Some(cell) = hunt_of(&arena, RED) {
            chosen = cell_of_unit(&arena, RED).map(|here| (arena.now_ms, here, cell));
            break;
        }
        wall += 500;
    }
    let (set_ms, here, cell) =
        chosen.expect("a machine that has seen nobody for three seconds should cross");
    let started = started.expect("the clock started at the first `Patrol` tick");
    let due = started + 3_000;
    assert!(
        set_ms - 500 < due && due <= set_ms,
        "the clock started at {started} ms, so the hunt is due at {due} ms, and it was set in the          pulse ending {set_ms} ms"
    );
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    assert_eq!(
        cell,
        (SIDE - 2, here.1),
        "the clamped corner of the OTHER side's zone, on the machine's own row: it was standing          in {here:?}"
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
    // far edge. **Read at the arrival and not at the end of a long run**: `hunt_after_ms`
    // is three seconds here, so a machine left to itself for twenty of them has
    // arrived, walked on and set out again, and the last pulse would say only
    // the last of that. Successive hunts are the design (the walk carries the
    // machine to a different row each time, so the crossings sweep the zone);
    // what this asserts is the clear that ends one of them.
    let mut arrived: Option<i64> = None;
    let until = arena.now_ms + 25_000;
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if hunt_of(&arena, RED).is_none() {
            arrived = Some(arena.now_ms);
            break;
        }
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    let at_ms = arrived.expect("the machine should have crossed and arrived");
    let here = cell_of_unit(&arena, RED).expect("the machine is on the window");
    assert!(
        here.0.abs_diff(cell.0).max(here.1.abs_diff(cell.1)) <= tacsim::arena::reflex::ARRIVE_CELLS,
        "the hunt was given up at {here:?}, which is not an arrival at {cell:?}"
    );
    let since = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.unheld_since_ms)
        .expect("the clock is restarted at the arrival tick");
    assert!(
        at_ms - 500 < since && since <= at_ms,
        "the clock should be restarted at the arrival tick (in the pulse ending {at_ms} ms) and          it reads {since} ms"
    );
    // The walk resumes at once, and from the node **nearest the far edge**,
    // (14, 8) — not the one it was walking to when the clock ran out. It set
    // out from (6, 3), whose nearest node is (1, 8) on its own side: a machine
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

/// **The hunt reads the map and no unit's pose** (16b cut E.4; doc 16 §1, the
/// observer rule from the machine's side).
///
/// Two fields differing in one thing only: whether the enemy is standing in the
/// zone the machine crosses to. The machine holds no contact on it either way —
/// both cells are well outside its seven-cell sight — and the cell it picks is
/// the same, because [`Arena::far_entry_cell`] reads the arena definition's
/// entry zones and nothing else. A hunt that went *where the enemy is* rather
/// than *where a side comes in* would be hidden truth steering a unit.
#[test]
fn e4_the_hunt_reads_the_arena_definition_and_never_a_unit() {
    let crossing = |blue: (u16, u16)| {
        let mut arena = e4_field((6, 3), blue, &[], &e4_cfg(0, 3_000));
        let mut bytes: Vec<u8> = Vec::new();
        let found = e4_run_until_hunt(&mut arena, RED, 20_000, &mut bytes);
        assert_eq!(
            arena.contacts.of(RED).count(),
            0,
            "the machine must hold nothing when it chooses, or the fields differ in two things"
        );
        found
    };
    // (15, 0) is inside Blue's own entry zone; (6, 15) is nowhere near it. Both
    // are more than seven cells from every cell the machine stands in.
    let standing_there = crossing((15, 0)).expect("the machine crosses");
    let standing_elsewhere = crossing((6, 15)).expect("the machine crosses");
    assert_eq!(
        standing_there, standing_elsewhere,
        "the hunt followed the enemy's pose instead of the arena's own entry zones"
    );
}

/// **What the hunt's clock reads is `holds_a_hostile`** (16b cut E.4; 16c
/// `B-04`, on 16b cut E.2's reading of *hostile*).
///
/// The machine carries no weapon, so `best_hostile` gives it no target and it
/// stays in `Patrol` whatever it can see — which is the one shape in which a
/// machine both holds a hostile and reaches the hunt's own arm. It sees the
/// **whole window** (sixteen cells), so the hostile it holds is never lost to
/// the waypoint walk carrying it out of sight: what the clock reads is the
/// classification, and nothing about range. Three arenas, each read **every
/// pulse**, because a machine that crossed, arrived and was cleared again
/// would end a long run in exactly the state one that never crossed does:
///
/// * a hostile in sight — it never crosses, on any pulse, and once the
///   classification lands the clock is held at `None`;
/// * the same hostile, on a hold the machine **knows** is dead (16b cut E.2's
///   `known_dead`) — not a hostile, so it crosses, with the tank still in plain
///   sight;
/// * `hunt_after_ms` at nought — never, on any pulse.
#[test]
fn e4_a_hostile_held_stops_the_clock_and_a_dead_one_does_not() {
    let unarmed = |cfg: &ArenaConfig, blue: (u16, u16)| {
        let mut units = e4_units((6, 3), blue, Vec::new());
        if let Some(red) = units.first_mut() {
            red.sensor = Some(SensorProfile {
                max_range_cells: SIDE,
                ..sight()
            });
        }
        e4_pinned(arena_of(units, &[], cfg, &[BLUE]))
    };
    // Every pulse of `run_ms`, with `check` asked of each.
    let watch = |arena: &mut Arena, run_ms: i64, check: &dyn Fn(&Arena)| {
        let mut bytes: Vec<u8> = Vec::new();
        let until = arena.now_ms + run_ms;
        let mut wall = arena.now_ms;
        while arena.now_ms < until {
            let out = arena.step(wall, &[], &[]);
            absorb(&out, &mut bytes);
            check(arena);
            wall += 500;
        }
    };

    // (a) A hostile in sight, held and classified. Six seconds of clock, which
    // is longer than the three `classify_after_ms` takes after the first pass
    // detects it, so the classification lands before the clock could run out
    // and the claim is about the hold and not about a race.
    let mut arena = unarmed(&e4_cfg(0, 6_000), (9, 3));
    watch(&mut arena, 20_000, &|arena| {
        assert_eq!(
            hunt_of(arena, RED),
            None,
            "a machine that can see the enemy does not go looking for it (at {} ms)",
            arena.now_ms
        );
    });
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Patrol),
        "an unarmed machine has no target and patrols whatever it sees"
    );
    assert!(
        arena.contacts.of(RED).any(|(_, held)| matches!(
            held.class,
            Classification::Classified { team } if team == Team::Blue
        )),
        "and it really is holding the tank: the fixture says nothing otherwise"
    );
    assert_eq!(
        arena.reflex.unit(RED).and_then(|row| row.unheld_since_ms),
        None,
        "and its clock is not merely ignored, it is held at `None`"
    );

    // (b) The same hold, marked the way the kill line marks it. It is no longer
    // a hostile, so the clock runs and the machine crosses — with the tank
    // still in its sight, so it is the mark and not a lost contact that let it
    // go.
    let mut arena = unarmed(&e4_cfg(0, 6_000), (9, 3));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 5_000, &mut bytes);
    learn_the_kill(&mut arena, RED, BLUE);
    let (here, cell) = e4_run_until_hunt(&mut arena, RED, 20_000, &mut bytes)
        .expect("a contact that reads dead is not a hostile, so the clock runs");
    assert!(
        arena
            .contacts
            .of(RED)
            .any(|(_, held)| held.target == BLUE && arena.contact_reads_dead(held)),
        "the hold it crossed under is the one it knows is dead, still held"
    );
    assert_eq!(
        cell,
        (SIDE - 2, here.1),
        "and the crossing is the far zone's clamped corner on its own row"
    );

    // (c) The key at nought: never, on any pulse.
    let mut arena = unarmed(&e4_cfg(0, 0), (15, 15));
    watch(&mut arena, 30_000, &|arena| {
        assert_eq!(
            hunt_of(arena, RED),
            None,
            "`hunt_after_ms = 0` never hunts (at {} ms)",
            arena.now_ms
        );
    });
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and it is on the waypoint walk: {goal:?}"
    );
}

/// **What `far_entry_cell` answers, and what it refuses to answer** (16b cut
/// E.4; 16c `B-04`).
///
/// Three windows, each differing from the fixture only in its entry zones and
/// the ground inside them:
///
/// * **the clamped corner is impassable** — the answer is the nearest passable
///   cell of that zone by Manhattan distance, reading the rows nearest the
///   machine's own first, and **never the corner**. `entry_cell`'s own
///   `.or(Some(clamped))` would have answered the wall;
/// * **nothing in the zone is passable** — `None`, and the machine keeps its
///   waypoint walk rather than driving at a cell no route reaches;
/// * **no other side has a zone at all** — `None`, same.
#[test]
fn e4_the_hunt_never_answers_a_cell_the_ground_refused() {
    // (a) The whole of the zone's near column is wall, so the clamped corner is
    // impassable whatever row the machine has walked to by the time the clock
    // runs out. The scan then reads the machine's own row first, finds (14, r)
    // refused and (15, r) open at a Manhattan distance nothing on the rows
    // either side can beat, and answers that. `entry_cell`'s own
    // `.or(Some(clamped))` would have answered the wall.
    // The wall stops at row 12, so the far column is **reachable** round the
    // bottom of it. A column walled end to end would seal the zone off, and the
    // `unreachable` edge would take the hunt back inside the same tick: the
    // fixture would then be the island case below wearing this one's name.
    let walled_column: Vec<(u16, u16, TerrainCell)> =
        (0..=12_u16).map(|y| (SIDE - 2, y, wall())).collect();
    let walled_corner = e4_definition(
        &walled_column,
        vec![red_zone(), blue_zone(SIDE - 2, SIDE - 1, 0, SIDE - 1)],
    );
    let mut arena = e4_pinned(arena_on(
        walled_corner,
        e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let (here, cell) = e4_run_until_hunt(&mut arena, RED, 20_000, &mut bytes)
        .expect("the zone has passable cells, so the hunt has a goal");
    assert!(
        here.1 <= 12,
        "the machine walked past the wall's end, so its clamped corner is passable and the          fixture asks nothing: {here:?}"
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
        "the nearest passable cell of the machine's own row, {here:?} being where it stood"
    );

    // (b) The whole zone is wall. `None`, and the walk goes on.
    let sealed: Vec<(u16, u16, TerrainCell)> = (0..=5_u16).map(|y| (11_u16, y, wall())).collect();
    let no_way_in = e4_definition(&sealed, vec![red_zone(), blue_zone(11, 11, 0, 5)]);
    let mut arena = e4_pinned(arena_on(
        no_way_in,
        e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let counted = run(&mut arena, 15_000, &mut bytes);
    assert_eq!(
        hunt_of(&arena, RED),
        None,
        "a zone with no passable cell is honestly nothing, not a wall to drive at"
    );
    // Both nodes are reachable on this window, so a failed search can only
    // be a hunt at a cell the ground refused — which the `unreachable` edge
    // would clear inside the same tick, leaving the two assertions above
    // green. The count is what tells `None` from *the wall, given up*.
    assert_eq!(
        counted.path_failures, 0,
        "the hunt answered a refused cell and paid a failed search for it"
    );
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "and the machine keeps its waypoint walk: {goal:?}"
    );

    // (c) No other side has a zone. Same answer, for a different reason.
    let one_zone = e4_definition(&[], vec![red_zone()]);
    let mut arena = e4_pinned(arena_on(
        one_zone,
        e4_units((6, 3), (15, 15), vec![cannon()]),
        &e4_cfg(0, 3_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    let counted = run(&mut arena, 15_000, &mut bytes);
    assert_eq!(hunt_of(&arena, RED), None);
    assert_eq!(counted.path_failures, 0, "and no search was paid for it");
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(waypoint_cells().contains(&goal), "{goal:?}");
}

/// **A hunt cell no route reaches costs one failed search per `hunt_after_ms`
/// and no more** (16b cut E.4) — the island the adversarial review named.
///
/// The other side's zone is the one cell of [`POCKET`], which [`a_pocket`]
/// seals: passable ground, so `far_entry_cell` answers it honestly, and no
/// route at all. The `unreachable` edge gives the hunt up and restarts the
/// clock, so the cost is bounded by the clock and not by `repath_ms` — which at
/// this fixture's doctrine (`repath_ms = 0`) would be one failure every 500 ms
/// of arena time for ever.
#[test]
fn e4_an_island_hunt_cell_costs_one_failed_search_per_clock() {
    let hunt_after_ms = 5_000_i64;
    let island = e4_definition(
        &a_pocket(),
        vec![
            red_zone(),
            blue_zone(POCKET.0, POCKET.0, POCKET.1, POCKET.1),
        ],
    );
    let mut arena = e4_pinned(arena_on(
        island,
        e4_units((6, 8), (15, 0), vec![cannon()]),
        &e4_cfg(0, 5_000),
        &[BLUE],
    ));
    let mut bytes: Vec<u8> = Vec::new();
    // Past the first crossing, so the count below is of steady-state failures.
    run(&mut arena, 6_000, &mut bytes);
    let from = arena.now_ms;
    let until = from + 40_000;
    let counted = run(&mut arena, until, &mut bytes);
    let elapsed = arena.now_ms - from;
    let ceiling = u32::try_from(elapsed / hunt_after_ms + 1).unwrap_or(u32::MAX);
    println!(
        "e4_island_hunt: {} failed searches in {elapsed} ms of arena time, ceiling {ceiling}",
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
        spot: None,
    };
    arena.reflex.set_order(
        unit,
        tacsim::deliberation::StandingOrder::accepted(told, arena.now_ms),
    );
}

/// Step until `unit`'s order has expired, answering the pulse it went on.
fn e4_run_until_unordered(
    arena: &mut Arena,
    unit: ObjId,
    limit_ms: i64,
    bytes: &mut Vec<u8>,
) -> Option<i64> {
    let until = arena.now_ms.saturating_add(limit_ms);
    let mut wall = arena.now_ms;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        if arena
            .reflex
            .unit(unit)
            .is_some_and(|row| row.order.is_none())
        {
            return Some(arena.now_ms);
        }
        wall += 500;
    }
    None
}

/// **A movement order outranks both clocks, and neither clock's goal survives
/// it** (16b cut E.4, on 16b cut N.2's order steering).
///
/// `Patrol`'s goal is a list in order, and the movement order is the head of it:
/// a commander who says *go there* is not answered with *I would rather look
/// over here* — and not at the order's expiry either. Two arcs, each run
/// **through** a short order to the tick after it expires:
///
/// * **the search** — the order lands with a live `last_plot` (twenty seconds
///   old at expiry, well inside the minute of `search_ms`); at expiry the goal
///   is a waypoint and not the plot;
/// * **the hunt** — the order lands with the machine already crossing; at
///   expiry `hunt` is `None`, the goal is a waypoint, and the hunt's wait is
///   counted from the order's end, not through it.
#[test]
fn e4_a_movement_order_outranks_the_search_and_the_hunt() {
    // (a) The search.
    let mut arena = e4_field((2, 8), (6, 8), &[], &e4_cfg(60_000, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    let (plot, _) = last_plot_of(&arena, RED).expect("the machine has something to search for");

    // The tank goes over the hill and a commander names a cell in the same
    // breath. The order steers: `Advance` drops the target, the machine goes to
    // `Patrol`, and the goal is the commander's cell — not the plot, and not the
    // far entry zone once the hunt's three seconds have gone by.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 15);
    }
    e4_advance(&mut arena, RED, (2, 2), 20_000);
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Patrol));
    assert_eq!(
        last_plot_of(&arena, RED),
        None,
        "the order's first tick gives the search up"
    );
    let until = arena.now_ms + 10_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        hunt_of(&arena, RED),
        None,
        "the hunt is never set under a movement order"
    );
    let goal = goal_of(&arena, RED).expect("an ordered machine has the commander's cell");
    assert!(
        goal == (2, 2) || (goal.0.abs_diff(2).max(goal.1.abs_diff(2)) <= 1),
        "the goal should be the ordered cell and it is {goal:?}"
    );
    let expired =
        e4_run_until_unordered(&mut arena, RED, 20_000, &mut bytes).expect("the order runs out");
    assert_eq!(
        last_plot_of(&arena, RED),
        None,
        "the search does not come back when the order runs out"
    );
    assert_eq!(hunt_of(&arena, RED), None);
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert_ne!(
        goal, plot,
        "the machine turned round for a plot it saw before the order"
    );
    assert!(
        waypoint_cells().contains(&goal),
        "at expiry the walk resumes: {goal:?}"
    );
    let since = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.unheld_since_ms)
        .expect("the hunt's clock is running");
    assert!(
        since > expired - 1_000,
        "the hunt's wait should count from the order's end ({expired} ms), and it reads {since} ms"
    );

    // (b) The hunt: a machine already crossing when the order lands.
    let mut arena = e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    let (_, crossing) =
        e4_run_until_hunt(&mut arena, RED, 20_000, &mut bytes).expect("the machine crosses");
    e4_advance(&mut arena, RED, (2, 2), 5_000);
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        hunt_of(&arena, RED),
        None,
        "the order's first tick gives the hunt up"
    );
    assert_eq!(goal_of(&arena, RED), Some((2, 2)));
    let expired =
        e4_run_until_unordered(&mut arena, RED, 10_000, &mut bytes).expect("the order runs out");
    assert_eq!(
        hunt_of(&arena, RED),
        None,
        "the crossing does not resume when the order runs out"
    );
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert_ne!(
        goal, crossing,
        "the machine went back to the crossing it was ordered off"
    );
    assert!(
        waypoint_cells().contains(&goal),
        "at expiry the walk resumes: {goal:?}"
    );
    let since = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.unheld_since_ms)
        .expect("the hunt's clock is running");
    assert!(
        since > expired - 1_000,
        "the hunt's wait should count from the order's end ({expired} ms), and it reads {since} ms"
    );
}

/// **`Withdraw` forgets the search, and a stand writes one** (16b cut E.4).
///
/// Three of the search's writers and clears that no other test reaches:
///
/// * **the health edge** clears `last_plot` on entry — a hurt machine that ran
///   home and rallied inside `search_ms` would otherwise drive straight back to
///   the plot where it was hurt;
/// * **a commander's `Withdraw`** clears it on entry, for the same reason;
/// * **a `Withdraw` that stands and fights writes it**, from the plot it turned
///   onto — the one writer that cannot read `row.target`, which `Withdraw`
///   clears, and which the builder carries in a local of its own.
#[test]
fn e4_withdraw_forgets_the_search_and_a_stand_writes_one() {
    // (a) The health edge. Four cells apart, inside the 4,900 cm band: the
    // machine holds it and remembers the plot.
    let mut arena = e4_field((6, 8), (10, 8), &[], &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert_eq!(
        last_plot_of(&arena, RED).map(|(cell, _)| cell),
        Some((10, 8))
    );
    // 166 per mille: under `stand_min_permille`, so nothing on the way home
    // writes a plot back.
    hurt(&mut arena, RED);
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Withdraw, 2_000, &mut bytes),
        "a machine hurt this far breaks off"
    );
    assert_eq!(
        last_plot_of(&arena, RED),
        None,
        "the health edge's first tick gives the search up"
    );
    // Home, out of sight of the tank (nine cells off), and quiet: it rallies,
    // and the walk it rallies to is the graph's and not the plot it was hurt
    // at.
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Patrol, 60_000, &mut bytes),
        "the machine should have rallied"
    );
    // The rally is decided inside `Withdraw`'s own arm and leaves the goal
    // empty; the first `Patrol` tick is the one after it.
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert_eq!(last_plot_of(&arena, RED), None);
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert!(
        waypoint_cells().contains(&goal),
        "a rallied machine walks the graph, and it is driving at {goal:?}"
    );

    // (b) A commander's `Withdraw`, on a healthy machine: the tank is 4,000 cm
    // off, outside the running stand's 2,333 cm, so the first tick in
    // `Withdraw` writes nothing back either.
    let mut arena = e4_field((6, 8), (10, 8), &[], &e4_cfg(60_000, 0));
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 6_000, &mut bytes);
    assert!(last_plot_of(&arena, RED).is_some());
    e4_order(
        &mut arena,
        RED,
        tacsim::deliberation::Objective::Withdraw,
        30_000,
    );
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Withdraw, 2_000, &mut bytes),
        "the order puts the machine in `Withdraw`"
    );
    assert_eq!(
        last_plot_of(&arena, RED),
        None,
        "the order's first tick gives the search up"
    );

    // (c) The stand. `withdraw_and_rally`'s machine breaks off at 277 per
    // mille beside a classified tank one cell away, and stands on its first
    // `Withdraw` tick. It never held a target before it broke off (the health
    // edge takes it straight out of `Patrol`), so a plot it remembers can only
    // be the stand's.
    let mut arena = e4_pinned(withdraw_and_rally());
    let mut bytes: Vec<u8> = Vec::new();
    let until = arena.now_ms + 5_000;
    let mut wall = arena.now_ms;
    let mut stood: Option<((u16, u16), Option<ObjId>)> = None;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if state_of(&arena, RED) == Some(ReflexState::Withdraw)
            && let Some((cell, _)) = last_plot_of(&arena, RED)
        {
            stood = Some((cell, target_of(&arena, RED)));
            break;
        }
        wall += 500;
    }
    let (cell, target) = stood.expect("a machine that stood and fought wrote no plot");
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

/// **A fight gives the hunt up, and the wait starts again from it** (16b cut
/// E.4) — `hunt` belongs to `Patrol` and to nothing else.
#[test]
fn e4_a_fight_gives_the_hunt_up() {
    let mut arena = e4_field((6, 3), (15, 15), &[], &e4_cfg(0, 3_000));
    let mut bytes: Vec<u8> = Vec::new();
    let (here, _) =
        e4_run_until_hunt(&mut arena, RED, 20_000, &mut bytes).expect("the machine crosses");
    // Something turns up three cells ahead of it on the crossing, classified
    // (this file's head says why a test may hand it over).
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(here.0 + 3, here.1);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert!(
        matches!(
            state_of(&arena, RED),
            Some(ReflexState::Approach | ReflexState::Engage)
        ),
        "the machine turned on what it found: {:?}",
        state_of(&arena, RED)
    );
    assert_eq!(
        hunt_of(&arena, RED),
        None,
        "leaving `Patrol` gives the crossing up"
    );
    let since = arena
        .reflex
        .unit(RED)
        .and_then(|row| row.unheld_since_ms)
        .expect("the clock is restarted, not stopped");
    assert!(
        arena.now_ms - 500 < since && since <= arena.now_ms,
        "the wait restarts at the tick it left `Patrol` (in the pulse ending {} ms), and it          reads {since} ms",
        arena.now_ms
    );
}

// ---------------------------------------------------------------------------
// The cross-tempo property (16b §11)
// ---------------------------------------------------------------------------

/// The whole arc in one fixture: contact, lost, search, arrival, quiet, hunt,
/// arrival in the far zone, contact.
///
/// The clocks are short enough that all of it fits inside 45 s of arena time,
/// and the ground is the plain fixture window, so nothing in the replay depends
/// on a search finding a way round anything.
fn a_search_and_a_hunt() -> Arena {
    e4_field((2, 8), (6, 8), &[], &e4_cfg(20_000, 10_000))
}

/// The one thing the script does: the blue tank pulls out to the far corner of
/// its own entry zone, where the hunt will eventually find it.
fn blue_pulls_into_its_zone(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(15, 14);
    }
}

/// What [`e4_the_arc_at_500`] saw of the middle of the arc.
#[derive(Debug, Default)]
struct Arc {
    /// A `Patrol` tick whose goal was the live `last_plot`'s cell.
    searched: bool,
    /// A pulse on which `hunt` was set.
    hunted: bool,
    /// A pulse, once the hunt had been set, on which the machine stood within
    /// [`tacsim::arena::reflex::ARRIVE_CELLS`] of the far zone's near column.
    crossed: bool,
}

/// [`replay_of`]'s loop at 500 ms over [`a_search_and_a_hunt`] and
/// [`blue_pulls_into_its_zone`], read each pulse for the search and the hunt.
fn e4_the_arc_at_500() -> Arc {
    let mut arena = a_search_and_a_hunt();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut arc = Arc::default();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        if arena.now_ms == SCRIPT_AT_MS {
            blue_pulls_into_its_zone(&mut arena);
        }
        if let Some(row) = arena.reflex.unit(RED) {
            arc.searched |= row.state == ReflexState::Patrol
                && row
                    .last_plot
                    .is_some_and(|(cell, _)| row.goal == Some(cell));
            arc.hunted |= row.hunt.is_some();
            arc.crossed |=
                arc.hunted && cell_of_unit(&arena, RED).is_some_and(|here| here.0 >= SIDE - 3);
        }
        if arena.now_ms >= 45_000 || out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    arc
}

/// **The search and the hunt at three cadences are one event list and one hash
/// sequence** (16b §11; doc 16 §3.1's D-16-2).
///
/// Both clocks are read at an arena millisecond, off fields written at an arena
/// millisecond, inside a due event that lands at the same arena millisecond at
/// every tempo — so a replay of this arc at 5,000, 1,000 and 500 ms is one
/// match. A clock read once per *pulse* would give three different matches, and
/// that is the one thing this cut was not allowed to do.
#[test]
fn e4_the_search_and_the_hunt_are_one_log_at_three_cadences() {
    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                45_000,
                a_search_and_a_hunt,
                blue_pulls_into_its_zone,
            )
        })
        .collect();
    let first = runs.first().expect("three replays");
    // The arc really did happen: a machine that never lost a contact, or never
    // crossed, would compare equal three ways and prove nothing. The log says
    // the two ends of it — a `lost`, and a `detected` after it.
    let kinds: Vec<(i64, &str)> = first
        .events
        .iter()
        .filter_map(|(t_ms, kind, _)| kind.split('"').nth(3).map(|kind| (*t_ms, kind)))
        .collect();
    println!("e4_search_and_hunt: {} events, {kinds:?}", kinds.len());
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
        "nothing was found again after the `lost` at {lost_at} ms, so the arc never closed:          {kinds:?}"
    );
    assert!(
        first.hashes.len() > 3,
        "a replay of one pulse is not a cross-tempo property"
    );
    // The guard: three cadences really are three different numbers of pulses.
    let counts: Vec<u32> = runs.iter().map(|run| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair[0] != pair[1]),
        "the three replays ran the same number of pulses: {counts:?}"
    );
    // And the middle of the arc, which the log does not name: the same script
    // at the finest cadence (the replay's own loop, so the same match as the
    // third replay above), read pulse by pulse for the search's goal and the
    // hunt's. A replay whose hunt never fired would still agree with itself
    // three ways.
    let arc = e4_the_arc_at_500();
    println!("e4_search_and_hunt: {arc:?}");
    assert!(
        arc.searched,
        "the machine never drove at its last plot: {arc:?}"
    );
    assert!(
        arc.hunted,
        "the hunt never fired inside the replay: {arc:?}"
    );
    assert!(
        arc.crossed,
        "the machine never reached the far zone on its hunt: {arc:?}"
    );
    for (at, run) in runs.iter().enumerate().skip(1) {
        assert_eq!(
            run.events, first.events,
            "cadence {at}: the event list differs"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "cadence {at}: the per-substep hash sequence differs"
        );
        assert_eq!(run.finals, first.finals, "cadence {at}: the field differs");
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
        Some(ReflexState::Engage),
        "it holds the band on a tank it can see"
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");

    // Behind the wall. The contact is not refreshed but it lives another three
    // passes, and the machine keeps firing at where it last saw it.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(11, 8);
    }
    assert!(
        e4_run_until_state(&mut arena, RED, ReflexState::Approach, 3_000, &mut bytes),
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

/// **The `avoid` clause of `first_step_is_open`** (16c `T-42`): a routeless
/// machine holds rather than drive into a cell it was stopped at, and drives
/// again when the entry ages out.
///
/// The path budget is nought, so the machine never has a route and every step
/// it takes is a step on a bearing nobody has checked — which is the whole case
/// the one-cell test exists for. The cell in front of it is then written into
/// [`ReflexUnit::avoid`] through [`Arena::note_blocked`], the public door the
/// `Blocked` ledger event uses.
#[test]
fn e4_t42_a_routeless_machine_holds_at_a_cell_it_was_stopped_at() {
    let mut arena = e4_field(
        (6, 8),
        (15, 15),
        &[],
        &arena_cfg(ReflexConfig {
            // No search at all: a machine that never routes is the subject.
            max_path_queries: 0,
            ..e4_cfg(0, 0).reflex
        }),
    );
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    let here = arena
        .unit(RED)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
        .expect("the machine is on the window");
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.route.is_empty()),
        "with no path budget the machine has no route, which is what makes this the one-cell case"
    );
    assert!(
        arena
            .unit(RED)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "and it is driving east at the far waypoint"
    );

    // The cell in front of it — the one step on the bearing to its goal, which
    // on this fixture is the west node and therefore the cell to the **west**.
    // Taking `here.0 + 1` on faith would write a cell behind the machine into
    // the list and assert nothing at all.
    let goal = goal_of(&arena, RED).expect("a patrol has a node to walk to");
    assert_eq!(goal.1, here.1, "the walk is along the machine's own row");
    let ahead = if goal.0 < here.0 {
        (here.0.saturating_sub(1), here.1)
    } else {
        (here.0.saturating_add(1), here.1)
    };
    let at_ms = arena.now_ms;
    arena.note_blocked(
        RED,
        ahead,
        tacsim::arena::motion::BlockedCause::Occupied,
        at_ms,
    );
    let until = arena.now_ms + 3_000;
    run(&mut arena, until, &mut bytes);
    let held = arena.unit(RED).map_or(-1, |unit| unit.desired_speed_cms);
    assert_eq!(
        held, 0,
        "a routeless machine whose next cell is one it was stopped at should hold, and it was \
         ordered {held} cm/s at {ahead:?}"
    );

    // And it goes again when the entry ages out — the list ages, it is not
    // cleared on a goal change.
    let until = at_ms + tacsim::arena::reflex::REFLEX_AVOID_TTL_MS + 3_000;
    run(&mut arena, until, &mut bytes);
    assert!(
        arena
            .unit(RED)
            .is_some_and(|unit| unit.desired_speed_cms > 0),
        "the remembered stop should have aged out and the machine driven on"
    );
}

/// The `Spent` draw, as a function nothing but the cadence can vary.
fn both_sides_dry() -> Arena {
    dry_pair(0, 0)
}

/// The same with the last round in the air — `a_round_still_in_flight_holds_the
/// _draw_off`'s own fixture, so the replay is of the case the draw is held off
/// for and not only of the easy one.
fn one_round_in_flight() -> Arena {
    let mut arena = dry_pair(1, 0);
    if let Some(weapon) = arena
        .unit_mut(RED)
        .and_then(|state| state.weapons.first_mut())
    {
        weapon.muzzle_speed_cms = 1_000;
    }
    arena
}

/// **Both sides fire their last round in the middle of the match**: two machines
/// facing each other with one round each, so the draw is called by the arena
/// after both rounds have left the breech and landed — several pulses in, at
/// every cadence, and not on the opening substep as [`both_sides_dry`]'s is.
fn both_fire_their_last() -> Arena {
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
                Controller::Reflex,
            ),
        ],
        &[],
        &arena_cfg(reflex_cfg()),
        &[],
    );
    for unit in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(unit) {
            state.ammo.insert(SmolStr::new("cannon"), 1);
        }
    }
    arena
}

/// **`EndReason::Spent` at three cadences** (16c `T-42`; doc 16 §3.8; 16b §11).
///
/// `two_dry_sides_end_in_a_draw_and_one_dry_side_plays_on` runs one cadence, so
/// nothing in the tree said that the arena millisecond a match runs out of
/// ammunition at is a fact of arena time rather than of how often the arena is
/// stepped. Three scripts: the plain draw (both magazines empty at check-out,
/// so it is called on the opening substep and the three runs are one call —
/// kept because it is the case the other two grow from), the one whose last
/// round is still in the air when the magazines empty, and the one where both
/// sides fire their last round mid-match. The two that fire are guarded: the
/// rounds really left the breech before the end, and the replay really was
/// several pulses at three different counts.
#[test]
fn e4_t42_the_spent_draw_is_one_log_at_three_cadences() {
    for (build, what, rounds) in [
        (both_sides_dry as fn() -> Arena, "both sides dry", 0_usize),
        (one_round_in_flight as fn() -> Arena, "a round in flight", 1),
        (
            both_fire_their_last as fn() -> Arena,
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
        println!("e4_spent ({what}): ended {ended:?}");
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
            // the substep that fired it, so both of `both_fire_their_last`'s
            // land at the millisecond they left and the draw is called there —
            // thirteen seconds in, which is what makes it mid-match.
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
                "{what}, cadence {at}: the per-substep hash sequence differs"
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

/// **The pose every case below starts from** (16b cut AA.1): RED at `(4, 8)`,
/// outside its entry zone, and BLUE at `blue`, both put there **after** the
/// power-up run for the reason [`a_withdrawal_beside`]'s doc gives — a machine
/// drives toward its nearest waypoint through its first seconds, so a case
/// written as "RED at (4, 8)" would otherwise be measured wherever the patrol
/// had drifted to. The contact is planted classified, and one decide tick is
/// stepped so the hostile edge has put the machine in `Approach`.
fn aa1_posed(weapons: Vec<WeaponSummary>, blue: (u16, u16)) -> (Arena, Vec<u8>) {
    let mut arena = duel_with(weapons, (4, 8), blue, &[]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(4, 8);
        state.speed_cms = 0;
    }
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(blue.0, blue.1);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Approach),
        "the recipe: a classified hostile past `engage_cm` puts the machine in `Approach`"
    );
    (arena, bytes)
}

/// Step `run_ms` more of arena time, a pulse at a time, and answer RED's plot
/// range at the first pulse that carried a new `fired` line — the range the
/// first shot was taken at, read at the pulse that carried it.
fn aa1_first_shot_range(arena: &mut Arena, bytes: &mut Vec<u8>, run_ms: i64) -> Option<i64> {
    let before = count(&events(arena, bytes), "fired");
    let until = arena.now_ms + run_ms;
    let mut wall = arena.now_ms;
    let mut first: Option<i64> = None;
    while arena.now_ms < until {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        if first.is_none() && count(&events(arena, bytes), "fired") > before {
            first = Some(plot_range(arena, RED, BLUE));
        }
        wall += 500;
    }
    first
}

/// Write `objective` for RED's squad under `constraints`, live for a minute.
fn aa1_order(
    arena: &mut Arena,
    objective: tacsim::deliberation::Objective,
    constraints: tacsim::deliberation::Constraints,
) {
    let squad = arena
        .squad_of(RED)
        .map(|roster| roster.id)
        .expect("one reflex machine is a squad of one");
    let told = tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: 60_000,
        constraints,
        ttl_ms: 60_000,
        issued_ms: arena.now_ms,
        issuer: None,
        spot: None,
    };
    arena.reflex.set_order(
        RED,
        tacsim::deliberation::StandingOrder::accepted(told, arena.now_ms),
    );
}

/// **A machine that cannot close engages from where it stands** (16b cut AA.1;
/// 16c `B-76`), and its mobile twin still closes to `engage_cm` first.
///
/// The recipe leaves RED in `Approach` 8,000 cm from BLUE — past `ENGAGE_CM`,
/// inside `RELEASE_CM`. With its engine killed there, the machine's edge into
/// `Engage` is `release_cm`: it locks, fires and never moves. At HEAD it
/// re-planned a stand-off cell every tick, was refused the drive, and never shot
/// again.
///
/// The twin is the contrast that makes it a proof: the same recipe with the
/// engine running drives in and takes its first shot inside `ENGAGE_CM` —
/// firing on the move while closing is not built (16b row E), and AA.1 does not
/// change it.
#[test]
fn aa1_a_machine_that_cannot_close_engages_from_where_it_stands() {
    let (mut arena, mut bytes) = aa1_posed(vec![cannon()], (12, 8));
    disable_motion(&mut arena, RED);
    let pose = arena.unit(RED).map(|unit| unit.pose.clone());
    let first = aa1_first_shot_range(&mut arena, &mut bytes, 6_000);

    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
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
    let (mut twin, mut twin_bytes) = aa1_posed(vec![cannon()], (12, 8));
    let first = aa1_first_shot_range(&mut twin, &mut twin_bytes, 30_000)
        .expect("the mobile twin closed and fired");
    assert!(
        first <= ENGAGE_CM,
        "a machine that can close still holds its fire to `engage_cm`: its first shot was at \
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
        && !(state_of(&arena, RED) == Some(ReflexState::Engage)
            && count(&events(&mut arena, &mut bytes), "fired") > 0)
    {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        wall += 500;
    }
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
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

/// **The ceiling holds for an immobile machine** (16b cut AA.1): its edge is
/// `release_cm` and no further, on a gun whose reach is shorter than the sight.
///
/// The gun is the five-cell [`coaxial`], for the reason
/// [`close_engage_and_release`]'s doc gives: with the cannon the release edge IS
/// the sight, so a planted contact past it dies after `contact_ttl_ms` and the
/// `lost` edge drops the machine to `Patrol` for a reason that has nothing to do
/// with the ceiling. At 7,000 cm BLUE is past `COAXIAL_RELEASE_CM` and inside the
/// sight, so RED's own sensor keeps the contact live and nothing expires.
#[test]
fn aa1_the_ceiling_past_release_holds_for_an_immobile_machine() {
    let (mut arena, mut bytes) = aa1_posed(vec![coaxial()], (11, 8));
    disable_motion(&mut arena, RED);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > COAXIAL_RELEASE_CM,
        "the fixture put the target at {range} cm, which is not past the release edge"
    );
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Approach),
        "past its firing ceiling an immobile machine stays in `Approach`"
    );
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
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "inside the release edge it shoots from where it stands"
    );
}

/// **An ordered `close` caps the immobile edge too** (16b cut AA.1; 16b cut
/// N.2): the order's band caps `release_cm` inside `ranges()`, so the edge a
/// machine that cannot close enters `Engage` at is the ORDERED ceiling.
#[test]
fn aa1_a_close_band_order_caps_the_immobile_edge_too() {
    use tacsim::deliberation::{Constraints, Objective, RangeBand};

    // `ranges()`'s rule for this fixture: the order's band against the cannon's
    // edges and the ten-cell sight, and seven tenths of it for `engage_cm`.
    let sight = u32::from(SIGHT_CELLS) * u32::from(CELL_CM);
    let close_release =
        i64::try_from(RangeBand::Close.ceiling_cm(&cannon().bands_cm(), sight)).unwrap_or(0);
    let close_engage = close_release * 700 / 1_000;
    assert_eq!(close_release, 3_333);

    let (mut arena, mut bytes) = aa1_posed(vec![cannon()], (12, 8));
    disable_motion(&mut arena, RED);
    aa1_order(
        &mut arena,
        Objective::Engage { target_hint: None },
        Constraints {
            hold_fire: false,
            max_range_band: Some(RangeBand::Close),
        },
    );
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > close_release && range <= RELEASE_CM,
        "the target is at {range} cm: inside doctrine's ceiling and past the order's"
    );
    let log = events(&mut arena, &mut bytes);
    let before = count(&log, "fired");
    let locked_before = count(&log, "locked");
    // Sampled every pulse, and the `locked` lines counted: `engage_orders` stages
    // its lock before its own `release_cm` return, so an edge wider than the
    // ordered ceiling would show as a machine flicking into `Engage` for a tick,
    // locking, and being thrown back — with nothing fired and the end state
    // `Approach`, which the last two assertions alone cannot tell from a held
    // edge.
    let until = arena.now_ms + 6_000;
    while arena.now_ms < until {
        let next = arena.now_ms + 500;
        run(&mut arena, next, &mut bytes);
        assert_ne!(
            state_of(&arena, RED),
            Some(ReflexState::Engage),
            "past the ordered ceiling it never reads `Engage` (at {} ms)",
            arena.now_ms
        );
    }
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Approach),
        "past the ordered ceiling it does not engage"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "locked"), locked_before, "and locks nothing");
    assert_eq!(count(&log, "fired"), before, "and fires nothing");

    // Inside the ordered ceiling, past its seven tenths: the immobile edge.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(7, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let range = plot_range(&arena, RED, BLUE);
    assert!(
        range > close_engage && range <= close_release,
        "the target is at {range} cm, which is not between the ordered edges"
    );
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "inside the ordered ceiling it shoots"
    );
}

/// **`hold_fire` still holds an immobile machine** (16b cut AA.1; 16b cut N.2):
/// the edge brings it into `Engage` and `engage_orders` keeps the trigger.
#[test]
fn aa1_hold_fire_still_holds_an_immobile_machine() {
    use tacsim::deliberation::{Constraints, Objective};

    let (mut arena, mut bytes) = aa1_posed(vec![cannon()], (12, 8));
    disable_motion(&mut arena, RED);
    aa1_order(
        &mut arena,
        Objective::Engage { target_hint: None },
        Constraints {
            hold_fire: true,
            max_range_band: None,
        },
    );
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Engage),
        "the edge still brings it into `Engage`"
    );
    assert_eq!(
        count(&events(&mut arena, &mut bytes), "fired"),
        before,
        "`hold_fire` stages no FIRE, whether or not the machine can move"
    );
}

/// **A hurt immobile machine stands and shoots back at any health** (16b cut
/// AA.1; the E row's Q8 departure, extended).
///
/// `hurt` is 166 per mille — under `withdraw_permille` AND under
/// `stand_min_permille`, so the en-route stand is closed to it. With its engine
/// killed it is home wherever it is (`arrived`), so its ceiling is `release_cm`:
/// it stands and fires at BLUE 8,000 cm out.
///
/// The mobile hurt twin is E.1's rule re-asserted, and the proof that `(4, 8)` is
/// en route and not home: it withdraws, never stands and fires nothing while it
/// drives. That is what makes the immobile half a new rule rather than the
/// `arrived` arm E.1 already had. The twin is read over two seconds, before it
/// can have covered the two cells that would put it inside `ARRIVE_CELLS` of its
/// entry zone.
#[test]
fn aa1_a_hurt_immobile_machine_stands_and_shoots_back_at_any_health() {
    let (mut arena, mut bytes) = aa1_posed(vec![cannon()], (12, 8));
    hurt(&mut arena, RED);
    disable_motion(&mut arena, RED);
    assert_eq!(arena.reflex_health_permille(RED), 166);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Withdraw));
    assert!(
        arena
            .reflex
            .unit(RED)
            .is_some_and(|row| row.stand_until_ms.is_some()),
        "it stood"
    );
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and shot back from where it stands, at 166 per mille"
    );

    // The mobile hurt twin.
    let (mut twin, mut twin_bytes) = aa1_posed(vec![cannon()], (12, 8));
    hurt(&mut twin, RED);
    let from = twin.unit(RED).map_or(0, |unit| unit.pose.x_cm);
    let before = count(&events(&mut twin, &mut twin_bytes), "fired");
    let until = twin.now_ms + 2_000;
    run(&mut twin, until, &mut twin_bytes);
    assert_eq!(state_of(&twin, RED), Some(ReflexState::Withdraw));
    let here = twin
        .unit(RED)
        .and_then(|unit| twin.definition.terrain.cell_of(&unit.pose))
        .expect("the twin is on the window");
    let goal = twin
        .reflex
        .unit(RED)
        .and_then(|row| row.goal)
        .expect("a withdrawing machine has somewhere to be");
    assert!(
        here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) > tacsim::arena::reflex::ARRIVE_CELLS,
        "the twin is still en route: at {here:?}, home {goal:?}"
    );
    assert!(
        twin.unit(RED).is_some_and(|unit| unit.pose.x_cm < from),
        "and driving home"
    );
    assert_eq!(
        twin.reflex.unit(RED).and_then(|row| row.stand_until_ms),
        None,
        "en route and under `stand_min_permille` it never stands"
    );
    assert_eq!(
        count(&events(&mut twin, &mut twin_bytes), "fired"),
        before,
        "and fires nothing while it drives"
    );
}

/// **An immobile withdrawn machine rallies once, like one that is home** (16b
/// cut AA.1), and then fights through the immobile edge: the whole path a hurt
/// machine with a dead engine takes.
///
/// BLUE opens eleven cells out, past the ten-cell sight, so nothing is held while
/// the rally clock runs.
#[test]
fn aa1_an_immobile_withdrawn_machine_rallies_once_like_one_that_is_home() {
    let mut arena = duel((4, 8), (15, 8), &[]);
    let mut bytes: Vec<u8> = Vec::new();
    run(&mut arena, 4_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = centre(4, 8);
        state.speed_cms = 0;
    }
    hurt(&mut arena, RED);
    disable_motion(&mut arena, RED);
    let until = arena.now_ms + 2_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Withdraw),
        "hurt, it breaks off"
    );
    let rally = i64::from(reflex_cfg().rally_ms);
    let until = arena.now_ms + rally + 3_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(
        state_of(&arena, RED),
        Some(ReflexState::Patrol),
        "twenty quiet seconds later it has rallied where it stands"
    );
    assert!(arena.reflex.unit(RED).is_some_and(|row| row.rallied));

    // Now something comes: it approaches and engages from where it stands.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(12, 8);
    }
    let now = arena.now_ms;
    plant_contact(&mut arena, RED, BLUE, Team::Blue, now);
    let before = count(&events(&mut arena, &mut bytes), "fired");
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    assert!(
        matches!(
            state_of(&arena, RED),
            Some(ReflexState::Approach | ReflexState::Engage)
        ),
        "a rallied machine takes the hostile: {:?}",
        state_of(&arena, RED)
    );
    let until = arena.now_ms + 6_000;
    run(&mut arena, until, &mut bytes);
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    assert!(
        count(&events(&mut arena, &mut bytes), "fired") > before,
        "and fights to the end"
    );
}

/// **An immobile machine in the coaxial's annulus, as one scripted arena** (16b
/// cut AA.1; 16b §11's mandatory cross-tempo test): RED holds the [`coaxial`]
/// with its engine killed before the first step — so all three cadences share
/// the fact from arena time nought; the power-up is not motion and still runs —
/// 4,000 cm from BLUE, inside `COAXIAL_RELEASE_CM` and outside
/// `COAXIAL_ENGAGE_CM`. It engages from where it stands; at [`SCRIPT_AT_MS`]
/// BLUE is put past the release edge and the sight ([`blue_breaks_contact`]),
/// the contact expires, and the `lost` edge drops RED to `Patrol`.
fn immobile_in_the_annulus() -> Arena {
    let mut arena = duel_with(vec![coaxial()], (2, 8), (6, 8), &[]);
    disable_motion(&mut arena, RED);
    arena
}

#[test]
fn aa1_an_immobile_machine_in_the_annulus_is_one_log_at_three_cadences() {
    // First: the arc really happens, at one cadence, before anything is compared.
    let mut arena = immobile_in_the_annulus();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let mut bytes: Vec<u8> = Vec::new();
    let mut arc: Vec<(i64, ReflexState, i64)> = Vec::new();
    let mut wall = 0_i64;
    loop {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if arena.now_ms == SCRIPT_AT_MS {
            blue_breaks_contact(&mut arena);
        }
        if let Some(state) = state_of(&arena, RED) {
            arc.push((arena.now_ms, state, plot_range(&arena, RED, BLUE)));
        }
        if arena.now_ms >= 40_000 || out.ended.is_some() {
            break;
        }
        wall += 1_000;
    }
    assert!(
        arc.iter().any(|(at, state, range)| *at < SCRIPT_AT_MS
            && *state == ReflexState::Engage
            && *range > COAXIAL_ENGAGE_CM
            && *range <= COAXIAL_RELEASE_CM),
        "it never engaged from inside the annulus before the script: {arc:?}"
    );
    assert!(
        arc.iter()
            .any(|(at, state, _)| *at > SCRIPT_AT_MS && *state == ReflexState::Patrol),
        "it never dropped to `Patrol` once the contact was gone: {arc:?}"
    );

    let runs: Vec<Replay> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            replay_of(
                cadence,
                40_000,
                immobile_in_the_annulus,
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
// 16b cut M.2 (16c `T-02`) — a machine's action is stamped at its tick
// ---------------------------------------------------------------------------

/// The (j) duel without the second machine: `RED` under the controller with the
/// cannon, `BLUE` a parked pilot tank three cells east.
fn m2_duel() -> Arena {
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

/// `RED`'s next scheduled tick.
fn m2_next_tick(arena: &Arena) -> i64 {
    arena
        .due
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                tacsim::arena::state::DueKind::ReflexTick { unit, .. } if unit == RED
            )
        })
        .map(|event| event.t_ms)
        .expect("a machine has a next tick")
}

/// **A machine's actions happen at its tick** (16b cut M.2, 16c `T-02`). `RED`'s
/// ticks are at `482 + 500·k` (`stagger_ms(2_350, RED, 500)` since 16b cut U.1;
/// `4 + 500·k` before it) — never a substep boundary on the 250 ms grid — and
/// the tick runs inside the substep after arena time was advanced to that
/// substep's end. Its `STARTUP`, its `fired` line, the round's flight and the
/// cycle are all read from the tick's own millisecond, not from the boundary:
/// before the cut the `startup` line read 250 and the `fired` line the next
/// multiple of 250.
#[test]
fn m2_a_machines_startup_and_shot_are_stamped_at_its_tick() {
    let mut arena = m2_duel();
    let mut bytes: Vec<u8> = Vec::new();
    let decide = i64::from(reflex_cfg().decide_ms);
    // 16b cut U.1: the seeded stagger, where it was `RED.0 % decide`.
    let stagger = stagger_ms(SEED, RED, decide);
    assert_ne!(
        stagger % 250,
        0,
        "the fixture needs a tick that is not a substep boundary"
    );
    let mut fired: Option<LedgerEvent> = None;
    while fired.is_none() && arena.now_ms < 30_000 {
        let until = arena.now_ms + 500;
        run(&mut arena, until, &mut bytes);
        fired = events(&mut arena, &mut bytes).into_iter().find(|event| {
            matches!(event.kind, tacsim::arena::ledger::EventKind::Fired { unit, .. } if unit == RED)
        });
    }
    let fired = fired.expect("the machine fired");
    // 16b cut U.1: the tick is 18 ms before a 500 ms boundary now, so the round
    // it fired lands in the next half second and not in the pulse that fired
    // it; one more pulse lets it arrive.
    let until = arena.now_ms + 500;
    run(&mut arena, until, &mut bytes);
    let log = events(&mut arena, &mut bytes);

    // The startup at the first tick, and its completion `startup_ms` after it.
    let startup = log
        .iter()
        .find(|event| event.kind.as_str() == "startup")
        .map(|event| event.t_ms);
    assert_eq!(startup, Some(stagger), "the `startup` line is the tick's");
    let online = log
        .iter()
        .find(|event| event.kind.as_str() == "power_online")
        .map(|event| event.t_ms);
    assert_eq!(
        online,
        Some(stagger + i64::from(arena.match_cfg.startup_ms)),
        "`StartupComplete` is scheduled from the tick"
    );

    // The shot.
    assert_eq!(
        fired.t_ms % decide,
        stagger,
        "the `fired` line is at a tick: {}",
        fired.t_ms
    );
    let tacsim::arena::ledger::EventKind::Fired {
        shot_seq, range_cm, ..
    } = fired.kind
    else {
        panic!("a fired line");
    };
    let impact = log
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                tacsim::arena::ledger::EventKind::Impact { shot_seq: seq, .. } if seq == shot_seq
            )
        })
        .map(|event| event.t_ms);
    assert_eq!(
        impact,
        Some(fired.t_ms + cannon().flight_ms(range_cm)),
        "the round flew from the tick"
    );
    assert_eq!(
        arena
            .unit(RED)
            .map(|unit| unit.weapon_ready_at_ms("cannon")),
        Some(fired.t_ms + i64::from(cannon().cycle_ms)),
        "the cycle runs from the tick"
    );
}

/// **The planner reads the tick's clock** (16b cut M.2, 16c `T-02`; review F10).
/// A cannon that comes ready after a tick and before the end of the substep it
/// runs in — halfway between them; it was a hundred milliseconds after the tick
/// until 16b cut U.1 moved `RED`'s tick to 18 ms before a boundary — is not
/// fired on that tick: the
/// planner and the resolver both read the tick's millisecond, so the machine
/// neither plans a shot the resolver would refuse `Cycling` nor takes one early.
///
/// **What tells the two clocks apart** (the M.2 review's `M2-R2`): `RED` also
/// carries the coaxial, ready at the tick and later in component order. Read at
/// the tick, the planner passes over the cycling cannon and fires the coaxial
/// ON the tick. Read at the substep's end, it would pick the cannon — ready by
/// then — the resolver would refuse it `Cycling` at the tick, `order` would drop
/// the refusal, and nothing would fire until the next tick. So the shot list is
/// the coaxial at the tick and the cannon at the tick after, and a planner put
/// back on `now_ms` fails it.
#[test]
fn m2_a_weapon_ready_between_the_tick_and_the_substep_end_waits_for_the_next_tick() {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon(), coaxial()],
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
    let decide = i64::from(reflex_cfg().decide_ms);
    // Bring it to the fight: the first shot.
    while count(&events(&mut arena, &mut bytes), "fired") == 0 && arena.now_ms < 30_000 {
        let until = arena.now_ms + 500;
        run(&mut arena, until, &mut bytes);
    }
    assert_eq!(state_of(&arena, RED), Some(ReflexState::Engage));
    let tick = m2_next_tick(&arena);
    let substep_end = tick - tick.rem_euclid(250) + 250;
    let ready = tick + (substep_end - tick) / 2;
    assert!(tick < ready && ready < substep_end, "the fixture's premise");
    // The cannon comes ready between the tick and the substep's end; the
    // coaxial is ready already.
    if let Some(unit) = arena.unit_mut(RED) {
        unit.ready_at_ms.insert(SmolStr::new("cannon"), ready);
        unit.ready_at_ms.insert(SmolStr::new("coaxial"), tick);
    }
    let before = count(&events(&mut arena, &mut bytes), "fired");
    run(&mut arena, tick + decide + 250, &mut bytes);
    let shots: Vec<(i64, SmolStr)> = events(&mut arena, &mut bytes)
        .into_iter()
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::Fired { unit, weapon, .. } if unit == RED => {
                Some((event.t_ms, weapon))
            }
            _ => None,
        })
        .skip(before)
        .collect();
    assert_eq!(
        shots,
        vec![
            (tick, SmolStr::new("coaxial")),
            (tick + decide, SmolStr::new("cannon")),
        ],
        "the cannon came ready at {ready}, after the tick at {tick}: the tick fires \
         the coaxial, and the cannon waits for the next one"
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
/// for. Both blue tanks are unarmed and nobody flies them.
fn z2_screen_field() -> Arena {
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
                10,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(8, 14),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(3, 8),
                Vec::new(),
                Controller::Pilot,
            ),
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
/// script, the red machine's cell sampled after every pulse.
fn z2_screen(cadence_ms: i64) -> (Replay, Z2Cells) {
    let arena = z2_screen_field();
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
    (run, cells)
}

/// **16b cut Z.2 (a), 16c N6 (16b §11: mandatory)**: a screen fires on what
/// enters its arc and never moves.
///
/// At 5,000, 1,000 and 500 ms one event list, one hash sequence (which now
/// folds the facing) and one final field. The machine's cell is the same after
/// every pulse of every replay. It locks and fires on the crossing tank only
/// once the script has put it inside the arc (the move at 10 s, to 45°), and
/// on nothing before; and the tank parked at 270° — inside `release_cm`, held
/// and classified the whole time, and nearer than the crossing tank — is never
/// locked at all.
#[test]
fn z2_a_screen_fires_on_what_enters_its_arc_and_never_moves() {
    let replays: Vec<(Replay, Z2Cells)> =
        [5_000_i64, 1_000, 500].into_iter().map(z2_screen).collect();
    for (_, cells) in &replays {
        assert!(!cells.is_empty());
        assert!(
            cells.iter().all(|cell| *cell == Some((8, 8))),
            "the screen never moves: {cells:?}"
        );
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
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
    // move at 25 s puts it at 170°; the machine's first tick after that reads
    // the new plot, so no round is fired from a tick after it. It does not
    // keep the last target it had, and it does not turn for the tank at 270°.
    let decide_ms = i64::from(reflex_cfg().decide_ms);
    assert!(
        fired.iter().all(|t_ms| *t_ms < 25_000 + decide_ms),
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
type Z2Read = (Option<ObjId>, Option<ObjId>, Option<ReflexState>, i64);

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
                    target_of(arena, Z2_SUPPORTER),
                    target_of(arena, Z2_CHARGE),
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
/// corner, out of the supporter's sight wherever the replay takes it.
fn z2_station_field() -> Arena {
    z2_station_field_on(&[])
}

/// [`z2_station_field`] with `paint`ed cells over its uniform ground (16b cut
/// Z.2's review: the unreachable station).
fn z2_station_field_on(paint: &[(u16, u16, TerrainCell)]) -> Arena {
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
                    Controller::Reflex,
                ),
                21,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(47, 15),
                Vec::new(),
                Controller::Pilot,
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

/// One replay of the station field: `SUPPORT` the charge at beat 0; at 5 s the
/// charge is moved twenty cells east; at 30 s the station is read and the
/// charge is moved out of the supporter's sight, and nothing refreshes the
/// supporter's hold on it, which expires; at 35 s the blue tank is put four
/// cells north of wherever the supporter stands.
fn z2_station(cadence_ms: i64) -> (Replay, Z2Station, Z2Samples) {
    let arena = z2_station_field();
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        Some(Z2_CHARGE),
    )));
    let mut read: Option<Z2Station> = None;
    let mut cells: Z2Samples = Vec::new();
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
        },
    );
    (run, read.expect("a pulse ended at 30 s"), cells)
}

/// **16b cut Z.2 (b), 16c N6 (16b §11: mandatory)**: a support prefers what its
/// charge is fighting, keeps station on it, and fights where it stands when it
/// cannot see it; a unit whose charge is itself fights on doctrine.
///
/// Every replay below is one event list, one hash sequence (which now folds the
/// charge) and one final field at 5,000, 1,000 and 500 ms.
///
/// 1. **The designation, for a pilot charge and for a machine charge.** The
///    supporter holds two classified hostiles, and its own score prefers the
///    nearer one showing its rear (the control replay under `ENGAGE` proves
///    it). Under `SUPPORT` its row targets the farther, front-on tank its charge
///    is fighting: the tank a pilot's lock names, read on the pilot's own store,
///    and the tank a machine's row targets, read off the memory the tick holds.
///    The machine charge — whose `of` is itself — fights on doctrine: it takes
///    that tank and closes on it as `ENGAGE` would.
/// 2. **The station.** The charge is moved twenty cells: the supporter, past its
///    leash, re-plans toward it and stops inside `SUPPORT_STATION_CELLS` of its
///    own plot of the charge, ordered to a standstill.
/// 3. **The blind stand.** The charge is moved out of the supporter's sight and
///    its hold expires: the supporter's cell never changes again, it stays in
///    `Patrol`, and when a hostile is put inside `release_cm` it fires on it from
///    where it stands. The order does not end: a charge it cannot see is not a
///    charge it has lost.
#[test]
fn z2_a_support_keeps_station_and_prefers_its_charges_target() {
    use tacsim::arena::reflex::{SUPPORT_LEASH_CELLS, SUPPORT_STATION_CELLS};
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
                    Some(ReflexState::Approach | ReflexState::Engage)
                ),
                "the charge is in the fight: {charge_state:?}"
            );
            assert!(
                charge_range < 4 * i64::from(CELL_CM),
                "the charge closed on its target as ENGAGE would: {charge_range} cm"
            );
        }
    }

    // 2 and 3. The station and the blind stand.
    let replays: Vec<(Replay, Z2Station, Z2Samples)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_station)
        .collect();
    let (cell, range, speed) = replays[0].1;
    assert!(
        replays.iter().all(|(_, read, _)| *read == replays[0].1),
        "the station reading diverged"
    );
    for (_, _, cells) in &replays {
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
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _, _)| run).collect();
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
    assert!(
        range.is_some_and(|cm| cm <= u64::from(SUPPORT_STATION_CELLS) * u64::from(CELL_CM)),
        "it stopped inside the station of its plot of the charge: {range:?}"
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
        Some(ReflexState::Patrol),
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
        Some(ReflexState::Patrol),
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

/// One replay of the station field with a moat of open water three cells wide
/// from edge to edge at columns 19 to 21: `SUPPORT` the charge at beat 0; at
/// 5 s the charge is moved twenty cells east, across the moat, where the
/// supporter still sees it and no route reaches its station. Answers the
/// replay and whether the supporter still holds the order at the end.
fn z2_moat_support(cadence_ms: i64) -> (Replay, bool) {
    let moat: Vec<(u16, u16, TerrainCell)> = (19..=21_u16)
        .flat_map(|x| (0..SIDE).map(move |y| (x, y, z2_water())))
        .collect();
    let arena = z2_station_field_on(&moat);
    let squad = z2_squad(&arena, Z2_SUPPORTER);
    let told = tacsim::arena::intents::IntentKind::Order(Box::new(z2_order(
        squad,
        tacsim::deliberation::Objective::Support { of: Z2_CHARGE },
        false,
        Some(Z2_CHARGE),
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
            if arena.now_ms == 5_000
                && let Some(state) = arena.unit_mut(Z2_CHARGE)
            {
                state.pose = centre(30, 8);
            }
            ordered_at_end = arena.reflex.order(Z2_SUPPORTER).is_some();
        },
    );
    (run, ordered_at_end)
}

/// **16b cut Z.2's review (16c N6; `B-41`'s lesson, 16b §11: mandatory)**: a
/// supporter whose station no route reaches ends its order.
///
/// Past its leash a supporter rejoins, and fires nothing while it does; a
/// station across a moat it cannot cross would hold it silent for the order's
/// whole life, paying a failed search every `repath_ms`. The failed search for
/// the station ends the order for this unit exactly as `B-41` ends an
/// `Advance`: one `order_expired` line naming `support` at the supporter's
/// first tick after the charge crossed, the same arena millisecond at every
/// cadence, and the supporter on doctrine, holding no order.
#[test]
fn z2_a_supporter_whose_station_no_route_reaches_ends_its_order() {
    let replays: Vec<(Replay, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_moat_support)
        .collect();
    for (_, ordered) in &replays {
        assert!(!*ordered, "the supporter is on doctrine, under no order");
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
    z2_one_log(&runs, "the unreachable station");
    let expired = z2_lines(&runs[0], "order_expired");
    assert_eq!(expired.len(), 1, "one line: {expired:?}");
    let (t_ms, line) = &expired[0];
    assert!(
        *t_ms > 5_000 && *t_ms <= 5_000 + i64::from(reflex_cfg().decide_ms),
        "at the supporter's first tick after its charge crossed: {t_ms}"
    );
    assert_eq!(
        *line,
        tacsim::arena::ledger::EventKind::OrderExpired {
            unit: Z2_SUPPORTER,
            squad: SmolStr::new("red-1"),
            objective: SmolStr::new("support"),
        }
    );
}

// ---- (c) the release -------------------------------------------------------

/// One red machine in the middle of the window and an unarmed blue tank in the
/// far corner that nobody flies.
fn z2_release_field() -> Arena {
    arena_of(
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
fn z2_release(cadence_ms: i64) -> (Replay, Option<ReflexState>) {
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
    let mut read: Option<Option<ReflexState>> = None;
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
                read = Some(state_of(arena, RED));
            }
        },
    );
    (run, read.expect("a pulse ended at 15 s"))
}

/// **16b cut Z.2 (c), 16c Q44 (16b §11: mandatory)**: a release ends an order at
/// one millisecond.
///
/// A healthy machine under a ten-minute `WITHDRAW` is released at a
/// pulse-aligned beat: one `order_released` line at that arena millisecond at
/// every cadence, carrying the squad and the issuer, and the machine reads
/// `Patrol` on the ticks after it — the expiry block's own restoration, so it
/// does not keep driving home. A release for another squad's word, at a later
/// beat and on a machine under red-1's order again, journals nothing and leaves
/// the order standing.
#[test]
fn z2_a_release_ends_an_order_at_one_millisecond() {
    let replays: Vec<(Replay, Option<ReflexState>)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(z2_release)
        .collect();
    for (_, read) in &replays {
        assert_eq!(
            *read,
            Some(ReflexState::Patrol),
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
        Some(ReflexState::Withdraw),
        "red-2's release left red-1's second order standing"
    );
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "a release is not an expiry"
    );
}

// ---- (d) the charge lost ----------------------------------------------------

/// A supporter and its charge — a pilot's tank nobody flies — two cells apart.
fn z2_charge_field() -> Arena {
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
                Controller::Reflex,
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
/// planted, so the kill has a row to mark), the charge lost at 10 s by `loss`.
/// Answers the replay and whether the supporter still holds the order at the
/// end.
fn z2_charge_lost(cadence_ms: i64, loss: Z2Loss) -> (Replay, bool) {
    let mut arena = z2_charge_field();
    plant_contact(&mut arena, Z2_SUPPORTER, Z2_CHARGE, Team::Red, 0);
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
            ordered_at_end = arena.reflex.order(Z2_SUPPORTER).is_some();
        },
    );
    (run, ordered_at_end)
}

/// **16b cut Z.2 (d), 16c N6 (16b §11: mandatory)**: a `SUPPORT` whose charge
/// is lost ends for the supporter through `B-41`'s route — one `order_expired`
/// line naming `support`, at the supporter's first tick after the loss, the
/// same arena millisecond at every cadence — and the supporter is on doctrine,
/// holding no order.
///
/// Two ways of losing it end the order: the charge **leaves** the arena, and —
/// the way a support usually ends in a fight (16b cut Z.2's review) — the
/// charge is **killed where the supporter watched it**, stays in the arena as a
/// wreck, and the supporter's own hold reads it dead. The third does not: a
/// charge wrecked where **nobody told the supporter** leaves the order standing
/// for the whole replay, because what ends it is what the supporter knows
/// (`contact_reads_dead`) and never the charge's live wreck flag (doc 16 §1).
#[test]
fn z2_a_support_whose_charge_leaves_ends_at_one_millisecond() {
    for loss in [Z2Loss::Leaves, Z2Loss::KilledInSight] {
        let what = format!("the charge lost ({loss:?})");
        let replays: Vec<(Replay, bool)> = [5_000_i64, 1_000, 500]
            .into_iter()
            .map(|cadence| z2_charge_lost(cadence, loss))
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

    // The wreck nobody told the supporter about: no end, at any cadence.
    let replays: Vec<(Replay, bool)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| z2_charge_lost(cadence, Z2Loss::WreckedUnseen))
        .collect();
    for (_, ordered) in &replays {
        assert!(
            *ordered,
            "the supporter learned of the wreck through knowledge it does not hold"
        );
    }
    let runs: Vec<Replay> = replays.into_iter().map(|(run, _)| run).collect();
    z2_one_log(&runs, "the charge wrecked unseen");
    assert!(
        z2_lines(&runs[0], "order_expired").is_empty(),
        "a wreck the supporter does not know of ends nothing"
    );
}
