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

/// The window, with `paint`ed cells over uniform ground.
fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
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
        waypoints: graph(),
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

/// A cannon that reaches the whole window — so a classified hostile is always in
/// band and the machine goes straight to `Engage`.
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
            definition: definition(paint),
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
    // The red tank carries a two-cell carbine and the blue one is four cells away,
    // so the hostile is held, classified and **out of band**: doc 16 §6's
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
    // The goal is the stand-off cell at 70 per mille of the carbine's 2,000 cm,
    // measured from the target back along the line — not the target's own cell.
    assert_ne!(row.goal, Some((13, 8)), "it does not drive onto the target");
    assert!(
        row.goal.is_some_and(|(x, _)| (7..13).contains(&x)),
        "the stand-off cell is between them: {:?}",
        row.goal
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
    // The red tank carries a two-cell carbine, so a hostile four cells away is
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
    // The cannon reaches every cell of the window, so the moment the contact is
    // classified the machine is in the band.
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
    let counters = run(&mut arena, 6_000, &mut bytes);
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

    let before = count(&events(&mut arena, &mut bytes), "fired");
    run(&mut arena, 20_000, &mut bytes);
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
        arena.unit(RED).is_some_and(|unit| unit.pose.x_cm < 6_500),
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
    run(&mut arena, 30_000, &mut bytes);
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
