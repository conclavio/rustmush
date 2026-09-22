//! **Deliberation, end to end** (16b cut N.2, N3; doc 16 §6 "Deliberation",
//! "Provider failure", "The cost run"; 16a Q9b; 16b §8's cut N row).
//!
//! Six claims, in the order the cut brief makes them:
//!
//! - **(1)** an order steers a squad: `advance` moves the patrol target, `hold`
//!   parks, `withdraw` withdraws — and `hold_fire` and `max_range_band` bite;
//! - **(2)** an expired order drops the squad to doctrine, with the
//!   `order_expired` ledger event *and* the one in-world notice (the arena writes
//!   the record, the world says the sentence — two authorities, one fact);
//! - **(3)** the floor holds: no second request for one squad inside
//!   `[tacsim.arena].deliberate_min_ms`;
//! - **(4)** a request whose reply never comes leaves the squad on doctrine and
//!   **no pulse waited** — the arena stepped on schedule while it was pending;
//! - **(5)** the mock-provider cost table has the right shape;
//! - **(6)** a malformed structured reply is refused, and the refusal reaches the
//!   audit record;
//! - **(7)** the cross-tempo property with an order in play: one order, delivered
//!   at one arena millisecond, obeyed and then run out, is one event list and one
//!   hash sequence at 5,000, 1,000 and 500 ms.
//!
//! (1), (3) and (4) drive `Arena::step` directly, the way `reflex.rs` does. (2)
//! does both halves: the arena's in the same way, and the world's over the
//! production composition root with a running match, because the sentence needs an
//! output sink and the arena has none. (5) and (6) drive the real
//! `mush_llm::LlmService` with a `MockProvider` behind it, so the protocol under
//! test is the shipped one and nothing but the network is faked.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, Ctx, GOD, ObjId, Outbox, ParsedArgs, SwitchSet};
use mush_proto::OutputEvent;
use mush_sim::data::Pose;
use mush_tactics::{
    CellAddr, CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
    TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::intents::{Intent, IntentKind, IntentOutcome, Refusal, apply_intent};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::perception::{SensorProfile, SignatureFactor};
use tacsim::arena::reflex::ReflexState;
use tacsim::arena::rng::draw_contact_id;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, Controller, CustodyOut,
    CustodyUnit, EntryZone, Pilot, SeenPose, ShadowContact, Team, TerrainCell, TrackedRules,
    Waypoint, Waypoints, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
use tacsim::deliberation::{
    Constraints, MIND_LINK_DROPPED, Objective, Order, RangeBand, SquadId, squads_of,
};

// ---------------------------------------------------------------------------
// The fixtures (reflex.rs's, with the order's two extra needs)
// ---------------------------------------------------------------------------

const SIDE: u16 = 16;
const CELL_CM: u16 = 1_000;
const SEED: u64 = 2_350;

/// Red's pair: `#4` commands `red-1`.
const RED_A: ObjId = ObjId(4);
/// Red's second tank.
const RED_B: ObjId = ObjId(6);
/// Blue's single tank, which is `blue-1` on its own.
const BLUE: ObjId = ObjId(9);

/// `red-1`, the squad every steering test orders.
fn red_one() -> SquadId {
    SquadId {
        team: Team::Red,
        index: 1,
    }
}

fn dref(kind: &str, id: &str) -> DefinitionRef {
    use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, OwnerId};
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

fn pose(x_cm: i64, y_cm: i64) -> Pose {
    Pose::at(x_cm, y_cm, 0, rules())
}

fn centre(x: u16, y: u16) -> Pose {
    pose(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
    )
}

/// The two-node patrol graph, west and east ends of row 8.
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

/// **An impassable cell** — 16b cut E.2 (16c `B-41`) is about a goal no route
/// reaches.
fn wall() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Wall,
        elev_cm: 0,
        move_cost: 0,
    }
}

fn definition() -> ArenaDefinition {
    definition_painted(&[])
}

/// The window with `paint`ed cells over uniform ground.
fn definition_painted(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, cell) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *cell;
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.deliberation"),
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

/// The scope a fixture tank carries, in cells — and **which one a test wants is
/// now the test's own choice** (16b cut E.1; see [`field`] and
/// [`field_seeing`]).
///
/// Every distance the controller fights at is capped by the platform's own
/// **nominal** sight since that cut, so the two scopes this file uses answer two
/// different questions:
///
/// * [`WIDE_SIGHT_CELLS`], thirty-five, for the tests about an **order's band**:
///   it puts the ordered ceilings they assert about (`close` at 11,666 cm,
///   `effective` at 23,333 cm) around the eleven cells the fixture stands its
///   tanks at, so the band is the thing under test. Its cost is that the three
///   tanks then hold each other on sight.
/// * [`NARROW_SIGHT_CELLS`], three, for the tests whose claim is about an order
///   **expiring** and needs *nothing is held unless a test planted it*: doctrine
///   with a hostile in view is a live machine with several states to be in, and
///   a claim about what an expiry restores is only sharp on a field where the
///   only thing doctrine can do is patrol.
///
/// `plant_contact` stays where it is used in both: it makes a contact
/// **classified** at a known arena millisecond rather than three seconds later.
fn sight(cells: u16) -> SensorProfile {
    use mush_atlas::TerrainClass as C;
    SensorProfile {
        max_range_cells: cells,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: 0,
        signature: vec![SignatureFactor {
            class: C::Ground,
            moving_permille: 1_000,
            stopped_permille: 1_000,
        }],
    }
}

/// A gun that reaches the whole window, so `Engage` is a matter of the order and
/// not of the geometry.
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

/// The wide scope: an order's ceiling is inside it, so a band is a band.
const WIDE_SIGHT_CELLS: u16 = 35;
/// The narrow scope: eleven cells is outside it, so nothing is held unless a
/// test plants it.
const NARROW_SIGHT_CELLS: u16 = 3;

fn tank(id: ObjId, team: Team, pose: Pose, controller: Controller) -> CustodyUnit {
    tank_seeing(id, team, pose, controller, WIDE_SIGHT_CELLS)
}

fn tank_seeing(
    id: ObjId,
    team: Team,
    pose: Pose,
    controller: Controller,
    sight_cells: u16,
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
        sensor: Some(sight(sight_cells)),
        weapons: vec![cannon()],
        profile: tacsim::arena::resolution::UnitProfile::default(),
        gunnery: 0,
        controller,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: tactical_unit(id),
    }
}

fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// Decide every 500 ms and re-plan freely, so an edge is visible in a handful of
/// pulses.
fn arena_cfg(deliberate_min_ms: u32) -> ArenaConfig {
    ArenaConfig {
        deliberate_min_ms,
        reflex: ReflexConfig {
            decide_ms: 500,
            repath_ms: 0,
            ..ReflexConfig::default()
        },
        ..ArenaConfig::default()
    }
}

/// The three-tank field every direct test runs on: red's pair (`red-1`) and one
/// blue tank (`blue-1`), all on the reflex controller, each carrying
/// [`WIDE_SIGHT_CELLS`] of scope.
fn field(cfg: &ArenaConfig) -> Arena {
    field_seeing(cfg, WIDE_SIGHT_CELLS)
}

/// The same field with a named scope — [`sight`] says which test wants which.
fn field_seeing(cfg: &ArenaConfig, sight_cells: u16) -> Arena {
    field_placed(cfg, sight_cells, &[], [(2, 8), (2, 9), (13, 8)])
}

/// The same field over `paint`ed ground with the three tanks where a test wants
/// them — **16b cut E.2** (16c `B-41`) needs a cell no route reaches, and it
/// needs the red pair somewhere a wall can seal them off from one.
fn field_placed(
    cfg: &ArenaConfig,
    sight_cells: u16,
    paint: &[(u16, u16, TerrainCell)],
    cells: [(u16, u16); 3],
) -> Arena {
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition_painted(paint),
            units: vec![
                tank_seeing(
                    RED_A,
                    Team::Red,
                    centre(cells[0].0, cells[0].1),
                    Controller::Reflex,
                    sight_cells,
                ),
                tank_seeing(
                    RED_B,
                    Team::Red,
                    centre(cells[1].0, cells[1].1),
                    Controller::Reflex,
                    sight_cells,
                ),
                tank_seeing(
                    BLUE,
                    Team::Blue,
                    centre(cells[2].0, cells[2].1),
                    Controller::Reflex,
                    sight_cells,
                ),
            ],
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    arena
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
}

/// Step to `until_ms` of arena time at 500 ms of wall, collecting the ledger and
/// every deliberation signal the pulses reported.
fn run(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> Vec<tacsim::arena::command::SquadSignal> {
    let mut wall = arena.now_ms;
    let mut signals = Vec::new();
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        signals.extend(out.deliberation);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    signals
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

/// How many shots `unit` was accepted for — the ledger's `fired` lines are
/// counted per shooter, because a field of three reflex tanks has two shooters
/// that are not the squad under test.
fn fired_by(events: &[LedgerEvent], unit: ObjId) -> usize {
    events
        .iter()
        .filter(|event| event.kind.as_str() == "fired")
        .filter(|event| {
            serde_json::to_value(&event.kind)
                .ok()
                .and_then(|value| value.get("unit").and_then(serde_json::Value::as_u64))
                == Some(u64::from(unit.0))
        })
        .count()
}

/// How many locks `unit` took — counted per platform for the reason
/// [`fired_by`] gives.
fn locked_by(events: &[LedgerEvent], unit: ObjId) -> usize {
    events
        .iter()
        .filter(|event| event.kind.as_str() == "locked")
        .filter(|event| {
            serde_json::to_value(&event.kind)
                .ok()
                .and_then(|value| value.get("unit").and_then(serde_json::Value::as_u64))
                == Some(u64::from(unit.0))
        })
        .count()
}

fn state_of(arena: &Arena, unit: ObjId) -> Option<ReflexState> {
    arena.reflex.unit(unit).map(|row| row.state)
}

fn goal_of(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena.reflex.unit(unit).and_then(|row| row.goal)
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
fn plot_of(arena: &Arena, target: ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

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

fn order(objective: Objective, ttl_ms: u32) -> Order {
    Order {
        squad: red_one(),
        objective,
        target: None,
        horizon_ms: 60_000,
        constraints: Constraints::default(),
        ttl_ms,
        issued_ms: 1,
    }
}

/// Put one order on every unit of `red-1`, the way the bridge fans a reply out.
fn issue(arena: &mut Arena, order: &Order) -> Vec<IntentOutcome> {
    let units: Vec<ObjId> = arena
        .squads
        .iter()
        .filter(|roster| roster.id == order.squad)
        .flat_map(|roster| roster.units.clone())
        .collect();
    assert!(!units.is_empty(), "the squad is on the arena's roster");
    units
        .into_iter()
        .map(|unit| {
            let intent = Intent {
                unit,
                channel: Channel::Helm,
                epoch: arena.epochs.current(unit, Channel::Helm),
                kind: IntentKind::Order(Box::new(order.clone())),
                seq: 0,
            };
            apply_intent(arena, &intent)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// (0) The grouping the whole cut rests on
// ---------------------------------------------------------------------------

#[test]
fn the_arena_and_the_grouping_agree_on_who_commands() {
    let arena = field(&arena_cfg(30_000));
    let by_hand = squads_of(&[
        (RED_A, Team::Red, true),
        (RED_B, Team::Red, true),
        (BLUE, Team::Blue, true),
    ]);
    assert_eq!(
        arena.squads, by_hand,
        "`Arena::new` derives the squads with the same function the match record files them with"
    );
    assert_eq!(arena.squads.len(), 2);
    assert_eq!(arena.squads[0].id.to_string(), "red-1");
    assert_eq!(arena.squads[0].commander, RED_A);
    assert_eq!(arena.squads[1].id.to_string(), "blue-1");
    assert_eq!(
        arena.squad_of(RED_B).map(|roster| roster.id),
        Some(red_one())
    );
}

// ---------------------------------------------------------------------------
// (1) An order steers a squad
// ---------------------------------------------------------------------------

#[test]
fn an_advance_order_moves_the_patrol_target() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    // Online, and patrolling its own graph: the west node of row 8.
    run(&mut arena, 2_000, &mut bytes);
    assert_eq!(state_of(&arena, RED_A), Some(ReflexState::Patrol));
    let doctrine = goal_of(&arena, RED_A);
    assert!(
        doctrine == Some((1, 8)) || doctrine == Some((14, 8)),
        "doctrine walks the authored graph, not {doctrine:?}"
    );

    let told = order(Objective::Advance { to: (9, 3) }, 60_000);
    assert!(
        issue(&mut arena, &told)
            .iter()
            .all(|outcome| *outcome == IntentOutcome::AppliedContinuous),
        "an order to a reflex unit is accepted, latest-wins, and never queues"
    );
    run(&mut arena, 4_000, &mut bytes);
    assert_eq!(
        goal_of(&arena, RED_A),
        Some((9, 3)),
        "the commander makes for the cell the order named"
    );
    assert_eq!(
        goal_of(&arena, RED_B),
        Some((9, 3)),
        "and so does the rest of the squad"
    );
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Patrol),
        "an advance is a movement order: the machine stays in Patrol"
    );
    // The squad on the other side was never ordered: whatever it is doing, it is
    // not making for the cell red-1 was sent to.
    assert_ne!(
        goal_of(&arena, BLUE),
        Some((9, 3)),
        "an order reaches its own squad and no other"
    );
}

#[test]
fn a_hold_order_parks_the_squad_and_a_movement_order_does_not_pursue() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    // A classified hostile the squad would ordinarily close on.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    plant_contact(&mut arena, RED_B, BLUE, Team::Blue, at);
    let told = order(Objective::Hold, 60_000);
    issue(&mut arena, &told);
    run(&mut arena, 5_000, &mut bytes);

    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Patrol),
        "hold parks: the machine does not take the target it can see"
    );
    assert_eq!(
        arena.reflex.unit(RED_A).and_then(|row| row.target),
        None,
        "a movement order drops the target rather than pursuing it"
    );
    assert_eq!(
        arena.unit(RED_A).map(|unit| unit.desired_speed_cms),
        Some(0),
        "and it is ordered to a stop"
    );
    let cell = arena
        .definition
        .terrain
        .cell_of(&arena.unit(RED_A).expect("the tank").pose);
    assert_eq!(goal_of(&arena, RED_A), cell, "its goal is where it stands");
}

#[test]
fn a_withdraw_order_withdraws_and_stands_when_it_is_cornered() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    let told = order(Objective::Withdraw, 60_000);
    issue(&mut arena, &told);
    run(&mut arena, 4_000, &mut bytes);

    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Withdraw),
        "withdraw is the state a commander can reach without being shot at"
    );
    assert_eq!(state_of(&arena, RED_B), Some(ReflexState::Withdraw));
    let goal = goal_of(&arena, RED_A).expect("a withdrawing tank has somewhere to be");
    assert!(
        goal.0 <= 1,
        "it makes for its OWN entry zone (columns 0..=1), not {goal:?}"
    );
    // **16b cut Q.7** changed this. Before it, a commanded withdrawal held its
    // fire all the way home; now a tank that still holds a classified hostile
    // inside its reach, with a line, stops and shoots back — **without leaving
    // `Withdraw` and without touching the goal**, which the two assertions above
    // are exactly what pin. `hold_fire` is the constraint a commander who means
    // "do not shoot" writes, and `hold_fire_stops_the_trigger_and_leaves_the_lock`
    // below is its test.
    let fired = events(&mut arena, &mut bytes);
    assert!(
        fired_by(&fired, RED_A) > 0,
        "cornered on the way out, it stands and shoots back"
    );
}

/// **`hold_fire` still means "do not stop to fight"** (16b cut Q.7).
///
/// The stand above is worth stopping for because the tank shoots back. A tank
/// whose commander wrote `hold_fire` would stop, turn onto the plot, stage
/// nothing and re-arm the dwell for as long as it held the contact — a parked
/// target under an order to break off. The constraint is read where the stand is
/// decided, not only at the trigger.
#[test]
fn a_withdraw_under_hold_fire_keeps_driving() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    let mut told = order(Objective::Withdraw, 60_000);
    told.constraints.hold_fire = true;
    issue(&mut arena, &told);
    run(&mut arena, 6_000, &mut bytes);

    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Withdraw),
        "the order still puts it in `Withdraw`"
    );
    assert_eq!(
        arena.reflex.unit(RED_A).and_then(|row| row.stand_until_ms),
        None,
        "it never stopped to fight a fight it was told not to have"
    );
    let fired = events(&mut arena, &mut bytes);
    assert_eq!(
        fired_by(&fired, RED_A),
        0,
        "and `hold_fire` stages no FIRE, cornered or not"
    );

    // The same field at the same arena millisecond with the constraint lifted
    // does stand, which is what makes the assertions above about the constraint
    // and not about the geometry.
    let mut free = field(&cfg);
    let mut free_bytes = Vec::new();
    run(&mut free, 2_000, &mut free_bytes);
    let at = free.now_ms;
    plant_contact(&mut free, RED_A, BLUE, Team::Blue, at);
    issue(&mut free, &order(Objective::Withdraw, 60_000));
    run(&mut free, 6_000, &mut free_bytes);
    assert!(
        free.reflex
            .unit(RED_A)
            .is_some_and(|row| row.stand_until_ms.is_some()),
        "the cornered stand is there when nothing forbids it"
    );
}

#[test]
fn hold_fire_stops_the_trigger_and_leaves_the_lock() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);

    let mut told = order(Objective::Engage { target_hint: None }, 60_000);
    told.constraints.hold_fire = true;
    issue(&mut arena, &told);
    run(&mut arena, 6_000, &mut bytes);
    let under_order = events(&mut arena, &mut bytes);
    assert_eq!(fired_by(&under_order, RED_A), 0, "hold_fire stages no FIRE");
    assert!(
        under_order.iter().any(|e| e.kind.as_str() == "locked"),
        "but the squad still closes and locks, so it is one order from shooting"
    );

    // The same field with the constraint lifted does shoot, which is what makes the
    // assertion above about the constraint and not about the geometry.
    let mut free = field(&cfg);
    let mut free_bytes = Vec::new();
    run(&mut free, 2_000, &mut free_bytes);
    let at = free.now_ms;
    plant_contact(&mut free, RED_A, BLUE, Team::Blue, at);
    issue(
        &mut free,
        &order(Objective::Engage { target_hint: None }, 60_000),
    );
    run(&mut free, 6_000, &mut free_bytes);
    assert!(
        fired_by(&events(&mut free, &mut free_bytes), RED_A) > 0,
        "with no constraint the same squad fires"
    );
}

#[test]
fn a_close_range_band_keeps_the_squad_from_shooting_across_the_window() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    // **16b cut E.1** re-based the ceiling: `close` is a third of the lesser of
    // the platform's own sight and the gun's last band, so on this fixture's
    // 35-cell scope it is 11,666 cm and not a third of the cannon's 2,500 m. The
    // two tanks are eleven cells (11,000 cm) apart — inside it either way, so the
    // band must NOT stop this one, which is still what the test says.
    let mut told = order(Objective::Engage { target_hint: None }, 60_000);
    told.constraints.max_range_band = Some(RangeBand::Close);
    issue(&mut arena, &told);
    run(&mut arena, 6_000, &mut bytes);
    assert!(
        fired_by(&events(&mut arena, &mut bytes), RED_A) > 0,
        "a target inside the ordered band is still shot at"
    );
}

#[test]
fn an_order_is_refused_for_a_unit_a_pilot_is_flying() {
    let cfg = arena_cfg(30_000);
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units: vec![
                tank(RED_A, Team::Red, centre(2, 8), Controller::Pilot),
                tank(BLUE, Team::Blue, centre(13, 8), Controller::Reflex),
            ],
            pilots: vec![Pilot {
                unit: RED_A,
                pilot: ObjId(1_000),
                station: SmolStr::new("cockpit"),
                epoch: 1,
            }],
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
        },
        &cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    assert!(
        arena.squad_of(RED_A).is_none(),
        "a seated tank is on no squad"
    );
    let intent = Intent {
        unit: RED_A,
        channel: Channel::Helm,
        epoch: arena.epochs.current(RED_A, Channel::Helm),
        kind: IntentKind::Order(Box::new(order(Objective::Hold, 30_000))),
        seq: 0,
    };
    assert_eq!(
        apply_intent(&mut arena, &intent),
        IntentOutcome::Refused(Refusal::NotReflex),
        "an order steers a machine, and a pilot's tank has none"
    );
}

// ---------------------------------------------------------------------------
// (2) An expired order drops the squad to doctrine
// ---------------------------------------------------------------------------

#[test]
fn an_expired_order_drops_the_squad_to_doctrine_with_its_ledger_event() {
    // The narrow scope on purpose: the claim below is *what the expiry hands
    // back*, and it is only a sharp claim on a field where doctrine's own answer
    // is one thing — the waypoint graph — rather than one of several.
    let cfg = arena_cfg(30_000);
    let mut arena = field_seeing(&cfg, NARROW_SIGHT_CELLS);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);

    // Two seconds of authority, then nothing.
    issue(&mut arena, &order(Objective::Advance { to: (9, 3) }, 2_000));
    run(&mut arena, 3_500, &mut bytes);
    assert_eq!(
        goal_of(&arena, RED_A),
        Some((9, 3)),
        "still under the order"
    );
    assert!(arena.reflex.under_order(arena.now_ms) >= 2);

    run(&mut arena, 7_000, &mut bytes);
    assert_eq!(
        arena.reflex.under_order(arena.now_ms),
        0,
        "the order has run out"
    );
    // Doctrine is the fallback, not idle: the machine is back on its own waypoint
    // graph and no longer making for the cell it was sent to.
    let after = goal_of(&arena, RED_A);
    assert!(
        after == Some((1, 8)) || after == Some((14, 8)),
        "back on its own graph, not still making for the ordered cell: {after:?}"
    );
    assert_eq!(state_of(&arena, RED_A), Some(ReflexState::Patrol));
    let log = events(&mut arena, &mut bytes);
    let expired: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| event.kind.as_str() == "order_expired")
        .collect();
    assert_eq!(
        expired.len(),
        2,
        "one line per unit dropped to doctrine, and the squad is a pair"
    );
    let text = serde_json::to_string(expired[0]).expect("the event encodes");
    assert!(text.contains("\"squad\":\"red-1\""), "{text}");
    assert!(text.contains("\"objective\":\"advance\""), "{text}");
    // The expiry is still *owed*: the squad asked at the opening pulse, so it is
    // inside `deliberate_min_ms` and the floor held its request back. The flag
    // waits rather than being dropped, which is what stops an order shorter than
    // the floor from leaving its squad silently on doctrine.
    assert!(
        arena.squads_expired.contains(&red_one()),
        "an expiry the floor held back is still owed"
    );
    let past = run(&mut arena, 31_000, &mut bytes);
    assert!(
        past.iter().any(|signal| signal.squad == red_one()
            && signal.reason == tacsim::arena::command::SignalReason::OrderExpired),
        "and it is asked for once the floor lifts"
    );
    assert!(
        !arena.squads_expired.contains(&red_one()),
        "then it is answered and forgotten"
    );
}

#[test]
fn an_expired_withdraw_order_comes_back_to_patrol_when_the_hull_is_whole() {
    // The narrow scope, for the same reason as the test above: `Withdraw` is
    // terminal for a hurt tank and not for one a commander sent home, and the
    // sharp form of that is the state it comes back to.
    let cfg = arena_cfg(30_000);
    let mut arena = field_seeing(&cfg, NARROW_SIGHT_CELLS);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    issue(&mut arena, &order(Objective::Withdraw, 2_000));
    run(&mut arena, 3_000, &mut bytes);
    assert_eq!(state_of(&arena, RED_A), Some(ReflexState::Withdraw));
    run(&mut arena, 7_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Patrol),
        "`Withdraw` is terminal for a hurt tank, not for one a commander sent home"
    );
    assert!(
        arena.reflex_health_permille(RED_A) >= ReflexConfig::default().withdraw_permille,
        "and the hull is whole, which is what makes the assertion above about the order"
    );
}

// ---------------------------------------------------------------------------
// (3) The floor
// ---------------------------------------------------------------------------

#[test]
fn the_floor_holds_and_a_zero_floor_lets_every_trigger_through() {
    let mut arena = field(&arena_cfg(30_000));
    let mut bytes = Vec::new();
    // The opening signal: no squad has ever had an order.
    let first = run(&mut arena, 1_000, &mut bytes);
    let red: Vec<_> = first
        .iter()
        .filter(|signal| signal.squad == red_one())
        .collect();
    assert_eq!(red.len(), 1, "one signal for red-1, not one per pulse");
    assert_eq!(red[0].reason, tacsim::arena::command::SignalReason::NoOrder);

    // Change the contact set twice inside the floor. Neither asks again.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    let inside = run(&mut arena, 6_000, &mut bytes);
    assert!(
        !inside.iter().any(|signal| signal.squad == red_one()),
        "no second request for one squad inside deliberate_min_ms"
    );

    // Past the floor, the same changed set asks once.
    let past = run(&mut arena, 31_500, &mut bytes);
    let again: Vec<_> = past
        .iter()
        .filter(|signal| signal.squad == red_one())
        .collect();
    assert_eq!(again.len(), 1, "exactly one request when the floor lifts");
    assert_eq!(
        again[0].reason,
        tacsim::arena::command::SignalReason::ContactsChanged
    );
    assert_eq!(again[0].commander, RED_A);
    assert!(
        again[0].digest.hostile >= 1,
        "the digest carries what the commander holds: {:?}",
        again[0].digest
    );
    assert_eq!(
        again[0].window,
        (SIDE, SIDE),
        "and the window, so an advance order can be bounded"
    );
    assert!(
        again[0].contacts.iter().all(|contact| contact.range_cm > 0),
        "contacts are carried with their range, nearest first"
    );

    // A zero floor is legal and means "every trigger asks".
    let mut eager = field(&arena_cfg(0));
    let mut eager_bytes = Vec::new();
    let opening = run(&mut eager, 1_000, &mut eager_bytes);
    let at = eager.now_ms;
    plant_contact(&mut eager, RED_A, BLUE, Team::Blue, at);
    let next = run(&mut eager, 2_500, &mut eager_bytes);
    assert!(
        opening.iter().any(|s| s.squad == red_one()) && next.iter().any(|s| s.squad == red_one()),
        "with no floor a changed contact set asks at once"
    );
}

// ---------------------------------------------------------------------------
// (4) A reply that never comes
// ---------------------------------------------------------------------------

#[test]
fn a_request_that_is_never_answered_costs_the_pulse_nothing() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();

    // The arena is stepped exactly as it would be with a provider that never
    // answers: every signal it reports is asked for by nobody, no intent ever comes
    // back, and the schedule is asserted pulse by pulse. The **opening** pulse only
    // establishes the wall reference (`StepOutput::substeps` is zero on it), so the
    // per-pulse advance is asserted from the second one on.
    let period = i64::from(cfg.tempo.engaged_ms);
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    let mut asked = 0_usize;
    while arena.now_ms < 20_000 {
        let before = arena.now_ms;
        wall += period;
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        asked += out.deliberation.len();
        pulses += 1;
        if pulses > 1 {
            assert_eq!(
                arena.now_ms,
                before.saturating_add(period),
                "the pulse advanced exactly one period while a request was pending"
            );
        }
        assert!(
            out.next_due_ms <= wall.saturating_add(period),
            "and it asked to be stepped again no later than one period on"
        );
        assert!(
            !out.lagged,
            "nothing about a pending request makes a pulse lag"
        );
    }
    assert!(
        asked > 0,
        "the field asked for an order, and nothing will answer"
    );
    assert_eq!(
        arena.now_ms,
        i64::from(pulses.saturating_sub(1)).saturating_mul(period),
        "arena time is the pulses after the opening one times the period, whatever the provider          did"
    );
    assert_eq!(
        arena.reflex.under_order(arena.now_ms),
        0,
        "no order arrived"
    );
    // Doctrine is the fallback doc 16 §6 names, and doctrine is the whole machine:
    // the squad is patrolling, closing or fighting on its own reflexes, and the one
    // thing it is not is idle.
    assert!(
        matches!(
            state_of(&arena, RED_A),
            Some(
                ReflexState::Patrol
                    | ReflexState::Approach
                    | ReflexState::Engage
                    | ReflexState::Withdraw
            )
        ),
        "the squad is on doctrine, not idle: {:?}",
        state_of(&arena, RED_A)
    );
    assert!(
        arena
            .unit(RED_A)
            .is_some_and(|unit| unit.power == tacsim::arena::state::Power::Online),
        "it brought itself up without being told to"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "order_expired")
            .count(),
        0,
        "an order that never arrived never expires"
    );
}

// ---------------------------------------------------------------------------
// (7) The cross-tempo property, with an order in play
// ---------------------------------------------------------------------------
//
// 16b §8's cut N row asks for the cross-tempo property *extended with two reflex
// units*, and `reflex.rs` (j) is that. This is the same property with the thing
// this cut added on top of them: **one order, delivered at one arena millisecond,
// obeyed and then run out.**
//
// It is the assertion that makes the design's choice of clock honest. An order's
// time to live is counted in arena milliseconds and read at the machine's own tick
// (`crate::arena::reflex`'s module head says why); if it were counted in pulses, or
// swept at a pulse boundary, the `order_expired` line would carry a different
// timestamp at every cadence and this test would fail. The order is delivered on
// the **opening** pulse, which is at arena millisecond zero whatever the cadence,
// so the three replays differ in cadence and in nothing else.

/// One unit as a replay ends — `reflex.rs` (j)'s shape, plus what the order left
/// on the machine.
type OrderedFinal = (
    ObjId,
    i64,
    i64,
    u32,
    i32,
    ReflexState,
    Option<(u16, u16)>,
    bool,
);

struct OrderedReplay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<OrderedFinal>,
}

fn replay_ordered(cadence_ms: i64, until_ms: i64) -> OrderedReplay {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    // D-16-2: the tempo decides only how often the arena steps, so it is pinned and
    // the three replays differ in cadence alone.
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let told = order(Objective::Advance { to: (9, 3) }, 10_000);
    let burst: Vec<Intent> = arena
        .squads
        .iter()
        .filter(|roster| roster.id == told.squad)
        .flat_map(|roster| roster.units.clone())
        .enumerate()
        .map(|(seq, unit)| Intent {
            unit,
            channel: Channel::Helm,
            epoch: 0,
            kind: IntentKind::Order(Box::new(told.clone())),
            seq: seq as u64,
        })
        .collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut first = true;
    loop {
        // The order rides the opening pulse, at arena millisecond zero.
        let out = if first {
            first = false;
            arena.step(wall, &[], &burst)
        } else {
            arena.step(wall, &[], &[])
        };
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
                row.is_some_and(|row| row.order.is_some()),
            )
        })
        .collect();
    OrderedReplay {
        events,
        hashes,
        finals,
    }
}

#[test]
fn an_order_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    let cadences = [5_000_i64, 1_000, 500];
    let runs: Vec<OrderedReplay> = cadences
        .into_iter()
        .map(|cadence| replay_ordered(cadence, 30_000))
        .collect();
    let first = runs.first().expect("three replays");

    // The run is worth comparing before it is compared: a replay in which the order
    // never landed, or never ran out, would compare equal three times and prove
    // nothing.
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "order_expired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}: {kinds:?}"
        );
    }
    assert!(
        first.finals.iter().all(|row| !row.7),
        "every order had run out by the end, which is what makes the expiry comparable"
    );
    assert!(first.hashes.len() >= 50, "{} substeps", first.hashes.len());

    for (at, run) in runs.iter().enumerate().skip(1) {
        let cadence = cadences.get(at).copied().unwrap_or(0);
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
// (5) and (6): the real structured call, with a mock provider behind it
// ---------------------------------------------------------------------------

/// An `LlmService` with the shipped route and a `MockProvider` behind it. Nothing
/// but the network is faked: the forced tool, the repair turn, the budget, the
/// admission and the audit row are all the shipped ones.
fn mock_service() -> (Arc<mush_llm::LlmService>, Arc<mush_llm::MockProvider>) {
    let mock = Arc::new(mush_llm::MockProvider::new("openrouter").with_caps(
        mush_llm::ProviderCaps {
            tool_calling: true,
            forced_tool_choice: true,
            ..mush_llm::ProviderCaps::default()
        },
    ));
    let provider: Arc<dyn mush_llm::LlmProvider> = mock.clone();
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .provider("openrouter", provider)
            .route(
                "tacsim_deliberate",
                mush_llm::RouteConfig {
                    model: "z-ai/glm-5.3-flash".to_string(),
                    max_tokens: 512,
                    timeout_s: 30,
                    reasoning: None,
                    reasoning_max_tokens: None,
                },
            )
            .global_enabled(true)
            .build(),
    );
    (llm, mock)
}

fn tool_use(input: serde_json::Value, cost_usd: f64) -> mush_llm::CompletionResponse {
    mush_llm::CompletionResponse {
        content: vec![mush_llm::ContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "issue_order".to_string(),
            input,
        }],
        stop_reason: mush_llm::StopReason::ToolUse,
        usage: mush_llm::Usage {
            input_tokens: 400,
            output_tokens: 40,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: Some(cost_usd),
        },
        model: "z-ai/glm-5.3-flash".to_string(),
    }
}

#[tokio::test]
async fn the_mock_provider_cost_table_has_the_right_shape() {
    use tacsim::deliberation::{CostRow, SquadBrief, cost_table, deliberate};

    let (llm, mock) = mock_service();
    let mut arena = field(&arena_cfg(0));
    let mut bytes = Vec::new();
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, 0);
    let signals = run(&mut arena, 2_000, &mut bytes);
    assert!(!signals.is_empty(), "the field asked");

    let squads: Vec<(String, ObjId)> = arena
        .squads
        .iter()
        .map(|roster| (roster.id.to_string(), roster.commander))
        .collect();
    let mut asked = 0_u64;
    for signal in signals.iter().take(4) {
        mock.push_response(tool_use(
            serde_json::json!({"objective": "engage", "ttl_ms": 20_000}),
            0.000_25,
        ));
        let brief = SquadBrief::from_signal(1, signal, 2);
        let order = deliberate(&llm, &brief).await.expect("the mock answers");
        assert_eq!(
            order.squad, signal.squad,
            "an order is for the squad it was asked for"
        );
        assert_eq!(order.ttl_ms, 20_000);
        asked += 1;
    }
    assert!(asked >= 2, "at least two squads asked");

    let table = cost_table(
        llm.ledger(),
        llm.audit_log(),
        &squads,
        3,
        2.0,
        Some(1.0),
        false,
    );
    assert_eq!(
        table.rows.len(),
        squads.len() + 1,
        "one row per squad plus the match total"
    );
    assert_eq!(table.route, "tacsim_deliberate");
    assert_eq!(table.model, "z-ai/glm-5.3-flash");
    assert_eq!(table.units, 3);
    assert_eq!(table.unit_minutes, 6.0);
    assert_eq!(table.cap_usd, Some(1.0));
    let total = table.total().expect("a match row");
    assert_eq!(total.calls, asked, "the match row's calls are the audit's");
    assert_eq!(total.total_tokens, asked * 440);
    assert!((total.cost_usd - 0.000_25 * asked as f64).abs() < 1e-9);
    assert!((total.usd_per_unit_minute - total.cost_usd / 6.0).abs() < 1e-12);
    // Per squad, from the commander's actor scope.
    let squad_calls: u64 = table
        .rows
        .iter()
        .filter(|row| row.scope == "squad")
        .map(|row| row.calls)
        .sum();
    assert_eq!(squad_calls, asked, "every call is attributed to a squad");
    // The exact columns, in order, are part of the contract: the harness prints
    // this array above the table and the JSON report carries it.
    assert_eq!(
        CostRow::COLUMNS,
        [
            "scope",
            "key",
            "calls",
            "refusals",
            "errors",
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "cost_usd",
            "calls_per_unit_minute",
            "tokens_per_unit_minute",
            "usd_per_unit_minute",
        ]
    );
    assert!(total.line().contains("scope=match key=tacsim_deliberate"));
}

#[tokio::test]
async fn a_malformed_structured_reply_is_refused_and_the_refusal_is_audited() {
    use tacsim::deliberation::{RefusedReason, SquadBrief, deliberate};

    let (llm, mock) = mock_service();
    let mut arena = field(&arena_cfg(0));
    let mut bytes = Vec::new();
    let signals = run(&mut arena, 2_000, &mut bytes);
    let signal = signals
        .iter()
        .find(|signal| signal.squad == red_one())
        .expect("red-1 asked");
    let brief = SquadBrief::from_signal(1, signal, 2);

    // Both attempts malformed: an objective outside the closed vocabulary, then an
    // `advance` off the window. The pipeline's own repair round trip is what makes
    // this two responses and not one.
    mock.push_response(tool_use(
        serde_json::json!({"objective": "nuke_everything", "ttl_ms": 20_000}),
        0.000_10,
    ));
    mock.push_response(tool_use(
        serde_json::json!({"objective": "advance", "to_x": 900, "to_y": 900, "ttl_ms": 20_000}),
        0.000_10,
    ));
    let error = deliberate(&llm, &brief)
        .await
        .expect_err("a malformed order is never issued");
    assert_eq!(
        RefusedReason::of(&error),
        RefusedReason::Malformed,
        "and it is refused by name, so the world can say why"
    );
    assert_eq!(mock.call_count(), 2, "the original attempt plus one repair");

    // The refusal reaches the audit record: `LlmService` writes one row per call
    // whatever the outcome, which is what the cost table's `refusals`/`errors`
    // columns are read from.
    let rows = llm.audit_log().recent(16);
    assert_eq!(rows.len(), 2, "one audit row per provider call");
    assert!(
        rows.iter().all(|row| row.feature == "tacsim_deliberate"
            && row.actor == Some(RED_A)
            && row.model == "z-ai/glm-5.3-flash"),
        "every row is attributed to the route and to the squad's commander: {rows:?}"
    );
    let totals = llm
        .audit_log()
        .totals()
        .into_iter()
        .find(|row| row.feature == "tacsim_deliberate")
        .expect("the feature's totals");
    assert_eq!(totals.calls, 2);
    assert_eq!(totals.refusals, 0, "the model answered; the schema refused");
    assert!(
        !error.prompt_hash().is_empty(),
        "the failure carries the prompt hash its audit row can be found by"
    );
    assert!(
        llm.ledger().actor(RED_A).total_tokens() > 0,
        "and the tokens the two attempts spent are metered against the commander"
    );
}

#[tokio::test]
async fn a_disabled_service_issues_no_order_and_the_squad_stays_on_doctrine() {
    use tacsim::deliberation::{
        DeliberationJob, DeliberationReply, SquadBrief, run_deliberation_async,
    };

    // The shipped default: `[llm].enabled = false`, no route, no provider. This is
    // what every test and every CI run does.
    let llm = mush_llm::LlmService::builder()
        .global_enabled(false)
        .build();
    let mut arena = field(&arena_cfg(0));
    let mut bytes = Vec::new();
    let signals = run(&mut arena, 2_000, &mut bytes);
    let signal = signals.first().expect("the field asked");
    let job = DeliberationJob::Deliberate {
        brief: SquadBrief::from_signal(1, signal, 2),
    };
    let payload = serde_json::to_string(&job).expect("a job encodes");
    let text = run_deliberation_async(&llm, &payload).await;
    let reply: DeliberationReply = serde_json::from_str(&text).expect("a reply decodes");
    assert!(
        matches!(reply, DeliberationReply::Refused { ref reason, .. } if reason == "unavailable"),
        "{reply:?}"
    );
    // And the same shape a host with no service at all answers with.
    let refusal: DeliberationReply =
        serde_json::from_str(&tacsim::deliberation::unavailable_reply(&payload))
            .expect("a reply decodes");
    assert_eq!(refusal.match_id(), 1);
}

// ---------------------------------------------------------------------------
// (2b) The world's half of the notice
// ---------------------------------------------------------------------------
//
// The sentence needs an output sink, and the arena has none (D-16-1), so this half
// runs over the production composition root with a real running match: the reply
// goes in through `arena_bridge::on_deliberation_reply`, the world clock moves past
// the order's time to live, and the egress drain says the line.

/// `[tacsim]` with the AI fill on, so `SIM START` fields the unclaimed pods and
/// the match has squads at all.
fn world_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.match_.startup_ms = 200;
    cfg.match_.board_timeout_ms = 5_000;
    cfg.match_.ai_fill = true;
    cfg
}

fn roster() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let cfg = world_config();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module(cfg));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

#[derive(Default)]
struct Accounts(BTreeMap<ObjId, AccountId>);

impl AccountIdentity for Accounts {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.get(&character).copied()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_world_says_the_mind_link_dropped_when_an_order_runs_out() {
    let mut tw = TestWorld::with_modules(roster());
    let lobby = tacsim::match_::centre_object(tw.world(), tacsim::match_::LOBBY_ENTRY)
        .expect("the OOC pack carries the simulator lobby");
    let mut map = Accounts::default();
    let mut pilots = Vec::new();
    for n in 0..2 {
        let pilot = tw.spawn_player(&format!("Pilot{n}"));
        tw.world_mut()
            .move_to(pilot, lobby)
            .expect("a player walks into the lobby");
        map.0.insert(
            pilot,
            AccountId::mint(&format!("account-{n}"), i64::from(n)).expect("an account id"),
        );
        pilots.push(pilot);
    }
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));

    // Claim one slot a side and seat both pilots; `ai_fill` fields the rest.
    tw.run(pilots[0], "sim team red");
    tw.run(pilots[1], "sim team blue");
    tw.run(pilots[0], "sim start");
    for pilot in &pilots {
        let pod = tacsim::match_::pod_of(tw.world(), *pilot).expect("a claimed slot locks a pod");
        let unit =
            tacsim::match_::tank_of(tw.world(), pod).expect("the pod's tank is in the world");
        let match_id = tacsim::match_::centre_of(tw.world())
            .expect("the centre")
            .match_id
            .expect("a boarding centre has a match id");
        let mut out = Outbox::new();
        let engine = tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: *pilot,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        tacsim::match_::slot_seated(&mut ctx, match_id, *pilot, unit)
            .expect("the seat is reported");
    }
    let record = tacsim::match_::centre_of(tw.world())
        .expect("the centre")
        .clone();
    let match_id = record.match_id.expect("the match started");
    let squads = tw
        .world()
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.squads.clone())
        .expect("the record carries its squads");
    assert!(
        !squads.is_empty(),
        "with ai_fill on, the unclaimed pods' tanks are commanded"
    );
    let squad = squads[0].id;

    // The reply the world would have got from the provider: one `hold` with a
    // one-second time to live.
    let reply = tacsim::deliberation::DeliberationReply::Ok {
        match_id: match_id.0,
        order: Order {
            squad,
            objective: Objective::Hold,
            target: None,
            horizon_ms: 30_000,
            constraints: Constraints::default(),
            ttl_ms: 1_000,
            issued_ms: 0,
        },
        model: "mock".to_string(),
        prompt_hash: "hash".to_string(),
    };
    let payload = serde_json::to_string(&reply).expect("a reply encodes");
    let said = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert!(
        !said.iter().any(|line| line.contains(MIND_LINK_DROPPED)),
        "nothing is said while the order stands: {said:?}"
    );

    // Two seconds later the order has run out, and the next egress drain says so.
    let now = tw.world().now();
    tw.set_now(now + 2);
    let said = with_ctx(&mut tw, GOD, tacsim::arena_bridge::drain_egress);
    let line = said
        .iter()
        .find(|line| line.contains(MIND_LINK_DROPPED))
        .unwrap_or_else(|| panic!("the one in-world notice was not said: {said:?}"));
    assert!(line.contains(&squad.to_string()), "{line}");
    assert!(line.contains("hold order has run out"), "{line}");

    // Said once, and once only.
    let again = with_ctx(&mut tw, GOD, tacsim::arena_bridge::drain_egress);
    assert!(
        !again.iter().any(|said| said.contains(MIND_LINK_DROPPED)),
        "a notice is said once: {again:?}"
    );
    let (issued, refused, in_flight) = tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.deliberation_counts(match_id))
        .expect("the registry");
    assert_eq!((issued, refused), (1, 0));
    assert_eq!(in_flight, 0, "the reply gave its in-flight slot back");
}

/// Run one closure with a `Ctx` over the test world and return every line of
/// output it produced, whoever it was addressed to.
fn with_ctx(tw: &mut TestWorld, actor: ObjId, body: impl FnOnce(&mut Ctx<'_>)) -> Vec<String> {
    let mut out = Outbox::new();
    {
        let engine = tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        body(&mut ctx);
    }
    out.events()
        .iter()
        .map(|(_, event)| render(event))
        .collect()
}

/// Every line an event carries, flattened — the notice under test arrives as a
/// `Sys` line to each pilot and as a `Notice` to each wizard, and the test cares
/// only that the sentence was said.
fn render(event: &OutputEvent) -> String {
    match event {
        OutputEvent::Sys(text) => text.to_plain_text(),
        OutputEvent::Notice { text, .. } => text.to_plain_text(),
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 16b cut E.1 — an ordered band is the pilot's own distance (`T-81`, `T-47`)
// ---------------------------------------------------------------------------

/// The fixture's own ceilings, so the numbers below are read once.
///
/// The tanks carry a 35-cell sight of 1,000 cm cells and a cannon whose one band
/// is 250,000 cm, so every band is thirds of the **sight**: `close` 11,666 cm,
/// `effective` 23,333 cm, `far` the cannon's own 250,000 cm. Doctrine's
/// `engage_band_permille` of 700 is applied to whichever of those the order
/// allows, **after** the cap, so the machine's own two edges under `close` are
/// 8,166 cm and 11,666 cm.
const CLOSE_CEILING_CM: i64 = 11_666;
/// Seven tenths of it, which is where `Approach` becomes `Engage`.
const CLOSE_ENGAGE_CM: i64 = 8_166;

/// **An ordered `close` caps the squad at the pilot's own `close`, and keeps its
/// hysteresis** (16b cut E.1; 16c `T-81`, `T-47`).
///
/// Two claims, and both were untestable before this cut because both edges read
/// the platform's whole 3 km reach:
///
/// 1. the ordered ceiling is `RangeBand::Close` of **the lesser of the sight and
///    the gun's last band** — 11,666 cm here — so a squad told to hold `close` is
///    held to the distance its pilots' `CONTACTS` rows call `close`, and fires
///    nowhere beyond it;
/// 2. the doctrine band is applied *after* that cap, so `Approach → Engage` is at
///    seven tenths of the ceiling and not at the ceiling itself. Capping the two
///    separately would give 11,666 and 11,666, and a target drifting about that
///    one distance would flip the machine every tick.
///
/// The midpoint claim is made for `close` alone because this window is 16,000 cm
/// across: `effective`'s two edges are 16,333 and 23,333 cm and `far`'s are
/// 24,500 and 35,000, so neither midpoint is a place two tanks on it can stand.
/// `release_cm > engage_cm` for all three bands is pinned arithmetically by
/// `arena/reflex.rs`'s own `e1_the_stock_tanks_three_rows_of_the_range_table`.
#[test]
fn e1_an_ordered_close_caps_the_squad_at_the_pilots_own_close() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    // Out past the ordered ceiling: 13,891 cm on the diagonal, which the 35-cell
    // sight holds comfortably.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(14, 15);
    }
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    let mut told = order(Objective::Engage { target_hint: None }, 60_000);
    told.constraints.max_range_band = Some(RangeBand::Close);
    issue(&mut arena, &told);

    // Twenty seconds, sampled every pulse: whenever the machine is in `Engage`
    // its plot range is inside the ordered ceiling, and the first tick it spends
    // there is inside seven tenths of it.
    let mut wall = arena.now_ms;
    let mut first_engage_cm: Option<i64> = None;
    let mut widest_engage_cm = 0_i64;
    for _ in 0..40 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if state_of(&arena, RED_A) == Some(ReflexState::Engage) {
            let range = arena
                .unit(RED_A)
                .map(|unit| plot_of(&arena, BLUE).range_from(&unit.pose))
                .and_then(|range| i64::try_from(range).ok())
                .unwrap_or(i64::MAX);
            first_engage_cm.get_or_insert(range);
            widest_engage_cm = widest_engage_cm.max(range);
        }
        wall += 500;
    }
    let first = first_engage_cm.expect("the squad closed and engaged");
    assert!(
        first <= CLOSE_ENGAGE_CM,
        "it engaged at {first} cm, past seven tenths of the ordered ceiling"
    );
    assert!(
        widest_engage_cm <= CLOSE_CEILING_CM,
        "it held the band out to {widest_engage_cm} cm, past the ordered ceiling"
    );
    let fired = fired_by(&events(&mut arena, &mut bytes), RED_A);
    assert!(fired > 0, "and inside the ordered band it shoots");

    // And the same field with the same order and a target parked **past** the
    // ceiling, with nowhere to close from, never fires at all: the squad holds
    // its fire rather than shooting at the distance doctrine alone would allow.
    let mut held = field(&cfg);
    let mut held_bytes = Vec::new();
    run(&mut held, 2_000, &mut held_bytes);
    if let Some(state) = held.unit_mut(RED_A) {
        // Pinned in place: a machine that may not close cannot walk into range.
        // **16b cut AA.1** (16c `B-76`): and a machine that cannot close enters
        // `Engage` at `release_cm` rather than at `engage_cm`, so this one fires
        // out to the ordered ceiling (11,666 cm) — and no further, which the
        // target past it proves: the case is now also a witness that an
        // immobile machine obeys the band cap. The witness is the `locked`
        // count, not the `fired` one: `engage_orders` holds its fire to the
        // ordered ceiling whatever edge the machine entered `Engage` at, but it
        // stages its lock first, so an edge past the ceiling shows as locks.
        state.mobility.max_speed_cms = 0;
    }
    if let Some(state) = held.unit_mut(BLUE) {
        state.pose = centre(14, 15);
    }
    let at = held.now_ms;
    plant_contact(&mut held, RED_A, BLUE, Team::Blue, at);
    issue(&mut held, &told);
    let until = held.now_ms + 20_000;
    run(&mut held, until, &mut held_bytes);
    let held_log = events(&mut held, &mut held_bytes);
    assert_eq!(
        fired_by(&held_log, RED_A),
        0,
        "past the ordered ceiling there is nothing to shoot at"
    );
    assert_eq!(
        locked_by(&held_log, RED_A),
        0,
        "and the pinned machine never entered `Engage` past it to lock"
    );
    // The control: the same parked machine with no band on its order does fire,
    // because doctrine's own ceiling is the 35,000 cm sight.
    let mut free = field(&cfg);
    let mut free_bytes = Vec::new();
    run(&mut free, 2_000, &mut free_bytes);
    if let Some(state) = free.unit_mut(RED_A) {
        state.mobility.max_speed_cms = 0;
    }
    if let Some(state) = free.unit_mut(BLUE) {
        state.pose = centre(14, 15);
    }
    let at = free.now_ms;
    plant_contact(&mut free, RED_A, BLUE, Team::Blue, at);
    issue(
        &mut free,
        &order(Objective::Engage { target_hint: None }, 60_000),
    );
    let until = free.now_ms + 20_000;
    run(&mut free, until, &mut free_bytes);
    assert!(
        fired_by(&events(&mut free, &mut free_bytes), RED_A) > 0,
        "with no band the same geometry is inside doctrine's own reach"
    );
}

// ---------------------------------------------------------------------------
// 16b cut E.2 — an ordered cell no route reaches ends the order for that unit
// (16c `B-41`; doc 16 §6)
//
// At HEAD the `unreachable` edge fell back to a waypoint and `Patrol` re-asserted
// the ordered cell on the very next tick, so the fallback never took: the unit
// steered straight at ground it could not reach and paid a failed search every
// `repath_ms` for the order's whole life. And because an `advance` suppresses the
// hostile edges, a squad sent to an unreachable cell was a pacifist patrol for up
// to `MAX_TTL_MS` with no way of telling the commander.
// ---------------------------------------------------------------------------

/// The five-by-five ring of wall that seals `(10, 8)` off from the rest of the
/// window — the cell no route reaches.
fn sealed_pocket() -> Vec<(u16, u16, TerrainCell)> {
    let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
    for x in 9..=11_u16 {
        paint.push((x, 7, wall()));
        paint.push((x, 9, wall()));
    }
    for y in 7..=9_u16 {
        paint.push((9, y, wall()));
        paint.push((11, y, wall()));
    }
    paint
}

/// The cell inside it.
const POCKET: (u16, u16) = (10, 8);

/// Step to `until_ms`, keeping the signals **and** the controller's counters —
/// `B-41`'s claim is about how many searches a failed goal costs.
fn run_counted(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> (
    Vec<tacsim::arena::command::SquadSignal>,
    tacsim::arena::reflex::ReflexCounters,
) {
    let mut wall = arena.now_ms;
    let mut signals = Vec::new();
    let mut counters = tacsim::arena::reflex::ReflexCounters::default();
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        counters.absorb(&out.reflex);
        signals.extend(out.deliberation);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    (signals, counters)
}

/// **Take blue's tank off the controller** so it stands where the fixture put
/// it.
///
/// Every one of 16b cut E.2's deliberation tests is about what RED's squad does
/// over tens of seconds of arena time, and a third reflex machine walking the
/// two-node patrol graph crosses the whole window in that time — it drives onto
/// the red pair, is held, and resets the very clocks and edges the tests are
/// about. A seat with nobody in it is the cheapest way to have a field that
/// stays where it was built.
fn park(arena: &mut Arena, unit: ObjId) {
    arena.set_controller(unit, Controller::Pilot, ReflexState::Withdraw);
}

/// The `order_expired` lines the black box holds, as `(unit, objective)`.
fn expiries(events: &[LedgerEvent]) -> Vec<(u64, String)> {
    events
        .iter()
        .filter(|event| event.kind.as_str() == "order_expired")
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            Some((
                value.get("unit")?.as_u64()?,
                value.get("objective")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// The waypoint cells doctrine can fall back on.
fn nodes(arena: &Arena) -> Vec<(u16, u16)> {
    arena
        .definition
        .waypoints
        .nodes()
        .iter()
        .map(|node| (node.x, node.y))
        .collect()
}

#[test]
fn e2_b41_an_unreachable_ordered_cell_ends_the_order_and_the_commander_hears() {
    // The floor is nought so the signal the drop asks for is reported on the
    // pulse it happened, which is what makes "the commander hears" a claim about
    // this cut and not about `deliberate_min_ms`.
    let cfg = arena_cfg(0);
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &sealed_pocket(),
        [(2, 8), (2, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);

    // Ten minutes of authority — `MAX_TTL_MS`, the worst case the row names.
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );
    let at = arena.now_ms;
    let (signals, counters) = run_counted(&mut arena, at + 20_000, &mut bytes);

    // One failed search per unit, over the whole of what would have been the
    // order's life. At HEAD this was one per unit per `repath_ms` for ten
    // minutes, and the goal flapped between the cell and the fallback every tick.
    assert_eq!(
        counters.path_failures, 2,
        "the squad is a pair and each of them pays exactly one: {counters:?}"
    );
    assert_eq!(counters.orders_expired, 2);

    // One line per unit, carrying the objective word, and nothing after it.
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        expiries(&log),
        vec![
            (u64::from(RED_A.0), "advance".to_string()),
            (u64::from(RED_B.0), "advance".to_string()),
        ]
    );
    // At the arena millisecond of the tick whose search failed, which is the
    // first decide tick past the order and not the order's nominal expiry.
    let dropped_at = log
        .iter()
        .find(|event| event.kind.as_str() == "order_expired")
        .map(|event| event.t_ms)
        .expect("a line");
    assert!(
        dropped_at < at + 2_000,
        "the drop waited for something: {dropped_at} ms against an order taken at {at} ms"
    );

    // The order is gone for both, and doctrine's own goal stands: a waypoint,
    // and the machine is driving to it.
    for unit in [RED_A, RED_B] {
        assert!(
            arena.reflex.order(unit).is_none(),
            "#{} still has one",
            unit.0
        );
        let goal = goal_of(&arena, unit).expect("doctrine has somewhere to be");
        assert!(
            nodes(&arena).contains(&goal),
            "#{} is still steering at the cell it cannot reach: {goal:?}",
            unit.0
        );
    }
    assert_eq!(arena.reflex.under_order(arena.now_ms), 0);

    // And the commander is told, through the one channel the expiry already had.
    assert!(
        signals.iter().any(|signal| signal.squad == red_one()
            && signal.reason == tacsim::arena::command::SignalReason::OrderExpired),
        "the squad never asked for a new order: {:?}",
        signals
            .iter()
            .map(|signal| (signal.squad, signal.reason))
            .collect::<Vec<_>>()
    );

    // Re-sent, the same cell costs exactly one more search and one more line per
    // unit — the deliberation floor is what bounds how often a commander may do
    // that, and here it is nought, so this is the worst case.
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );
    let from = arena.now_ms;
    let (_, again) = run_counted(&mut arena, from + 10_000, &mut bytes);
    assert_eq!(again.path_failures, 2, "{again:?}");
    assert_eq!(again.orders_expired, 2);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(expiries(&log).len(), 4);
}

#[test]
fn e2_b41_a_reachable_cell_is_still_obeyed_to_arrival() {
    // The other half of the rule: nothing about a sealed cell makes the machine
    // shy of an ordered one it can reach.
    let cfg = arena_cfg(0);
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &sealed_pocket(),
        [(2, 8), (2, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );
    let at = arena.now_ms;
    run_counted(&mut arena, at + 5_000, &mut bytes);
    assert!(
        arena.reflex.order(RED_A).is_none(),
        "the sealed cell dropped"
    );

    // A second order, naming a cell a route reaches.
    issue(
        &mut arena,
        &order(Objective::Advance { to: (6, 3) }, 600_000),
    );
    let from = arena.now_ms;
    let (_, counters) = run_counted(&mut arena, from + 30_000, &mut bytes);
    assert_eq!(counters.path_failures, 0, "{counters:?}");
    assert!(
        arena.reflex.order(RED_A).is_some(),
        "the second order stands"
    );
    let here = arena
        .definition
        .terrain
        .cell_of(&arena.unit(RED_A).expect("the tank").pose)
        .expect("on the window");
    assert!(
        here.0.abs_diff(6) <= 1 && here.1.abs_diff(3) <= 1,
        "it never arrived: {here:?}"
    );
}

#[test]
fn e2_b41_doctrine_is_back_the_tick_after_the_order_drops() {
    // The adversarial half of the row: an `advance` suppresses the hostile edges,
    // so a squad sent to a cell nobody can reach was a pacifist patrol for the
    // order's whole life. A hostile planted after the drop is closed on.
    let cfg = arena_cfg(0);
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &sealed_pocket(),
        [(2, 8), (2, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );

    // Under the live order the contact is walked past: the objective drops the
    // target on every tick.
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    run_counted(&mut arena, at + 500, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Patrol),
        "a movement order does not pursue"
    );
    assert!(arena.reflex.order(RED_A).is_none(), "and the order is over");

    // On doctrine, the same contact is a target.
    let from = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, from);
    run_counted(&mut arena, from + 1_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Approach),
        "the machine is on doctrine and fighting what it meets"
    );
}

#[test]
fn e2_b41_a_failed_goal_that_is_not_the_ordered_cell_drops_nothing() {
    // The pin review E-04 asked for. `follow` is the one steer for every state,
    // so its `Err` arm sees the stand-off cell, the run home and the patrol node
    // fail as well as the ordered cell — and only the ordered cell ends an order.
    let cfg = arena_cfg(0);

    // (a) An `engage` order whose stand-off goal is inside the sealed pocket,
    // because that is where the hostile the squad was pointed at is standing.
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &sealed_pocket(),
        [(2, 8), (2, 9), POCKET],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(
        &mut arena,
        &order(Objective::Engage { target_hint: None }, 600_000),
    );
    let at = arena.now_ms;
    let mut wall_ms = arena.now_ms;
    let mut counters = tacsim::arena::reflex::ReflexCounters::default();
    let mut ended = None;
    while arena.now_ms < at + 6_000 {
        let now = arena.now_ms;
        plant_contact(&mut arena, RED_A, BLUE, Team::Blue, now);
        let out = arena.step(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        counters.absorb(&out.reflex);
        // An ended match steps no further and its clock stops, so a loop that
        // watched only `now_ms` would spin for ever the day red's pair lands a
        // round on the tank parked in the pocket. Asserted, not merely broken
        // on: an early end would shorten this window in silence.
        if out.ended.is_some() {
            ended = out.ended;
            break;
        }
        wall_ms += 500;
    }
    assert!(
        ended.is_none(),
        "the match ended under the fixture: {ended:?}"
    );
    assert!(
        counters.path_failures > 0,
        "the stand-off cell was reachable, so nothing was proved: {counters:?}"
    );
    assert!(
        arena.reflex.order(RED_A).is_some(),
        "a stand-off goal that failed its search ended an `engage` order"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));

    // (b) A `withdraw` order whose entry cell is walled off from the squad. The
    // red zone is columns 0 and 1; a wall down column 2 puts it out of reach.
    let blocked = wall();
    let curtain: Vec<(u16, u16, TerrainCell)> = (0..SIDE).map(|y| (2_u16, y, blocked)).collect();
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &curtain,
        [(5, 8), (5, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(&mut arena, &order(Objective::Withdraw, 600_000));
    let at = arena.now_ms;
    let (_, counters) = run_counted(&mut arena, at + 8_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Withdraw),
        "the order still puts it in `Withdraw`"
    );
    assert!(
        counters.path_failures > 0,
        "the entry zone was reachable, so nothing was proved: {counters:?}"
    );
    assert!(
        arena.reflex.order(RED_A).is_some(),
        "an unreachable entry cell ended a `withdraw` order"
    );
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));
}

#[test]
fn e2_b41_a_cell_sealed_only_by_a_remembered_stop_ends_the_order_too() {
    // **The recorded cost of the rule.** The search that fails runs with the
    // unit's own `avoid` list (16b cut Q.7), whose entries age out after
    // `REFLEX_AVOID_TTL_MS` — so a cell sealed for thirty seconds by a remembered
    // stop ends the order where a retry half a minute later would have kept it.
    // The commander is told within one signal either way, which is the better
    // half of that trade; a `SignalReason` that says WHY is new prompt vocabulary
    // and is not built.
    let cfg = arena_cfg(0);
    // A band down column 8 with one gap, at row 8: every route east goes through
    // (8, 8).
    let band: Vec<(u16, u16, TerrainCell)> = (0..SIDE)
        .filter(|y| *y != 8)
        .map(|y| (8_u16, y, wall()))
        .collect();
    let mut arena = field_placed(&cfg, NARROW_SIGHT_CELLS, &band, [(2, 8), (2, 9), (13, 13)]);
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);

    // The gap is remembered as a stop — the production writer, off the `Blocked`
    // ledger event, is what a tank held against a parked teammate there would
    // have used.
    let now = arena.now_ms;
    for unit in [RED_A, RED_B] {
        arena.note_blocked(
            unit,
            (8, 8),
            tacsim::arena::motion::BlockedCause::Occupied,
            now,
        );
    }
    issue(
        &mut arena,
        &order(Objective::Advance { to: (13, 3) }, 600_000),
    );
    let at = arena.now_ms;
    // A short window on purpose: past it the fallback waypoints on the far side
    // of the band are unreachable too, which is doctrine's own business and not
    // this row's.
    let (_, counters) = run_counted(&mut arena, at + 1_500, &mut bytes);
    assert_eq!(counters.path_failures, 2, "{counters:?}");
    assert!(
        arena.reflex.order(RED_A).is_none(),
        "a transiently sealed route kept the order"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(expiries(&log).len(), 2);
}

// ---------------------------------------------------------------------------
// 16b cut E.2 — a standing withdraw order is not rallied out of (16c `B-40`)
// ---------------------------------------------------------------------------

#[test]
fn e2_b40_a_standing_withdraw_order_is_not_rallied_out_of() {
    // At HEAD a machine that got home under a `withdraw` order and waited out
    // `rally_ms` rallied to `Patrol`, and the very next tick the order put it
    // back into `Withdraw` and threw its route away: once per `rally_ms` for the
    // order's whole life.
    let cfg = arena_cfg(0);
    let rally_ms = i64::from(ReflexConfig::default().rally_ms);
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &[],
        // The red pair opens **in its own entry zone**, so it is home from the
        // first tick and the only thing between it and the rally is the order.
        [(1, 8), (1, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(&mut arena, &order(Objective::Withdraw, 600_000));

    let at = arena.now_ms;
    run_counted(&mut arena, at + 3 * rally_ms + 2_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Withdraw),
        "three rally periods of quiet and it left the state its commander put it in"
    );
    let row = arena.reflex.unit(RED_A).cloned().expect("a machine");
    assert_eq!(
        row.quiet_since_ms, None,
        "the quiet clock ran under a standing order"
    );
    assert!(!row.rallied, "the order spent the tank's one comeback");
    assert!(arena.reflex.order(RED_A).is_some());
}

#[test]
fn e2_b40_the_rally_runs_again_once_the_withdraw_order_has_expired() {
    // And what the order was holding back: at the expiry a healthy machine is in
    // `Patrol` at once, and a hurt one starts its quiet from that tick and
    // rallies `rally_ms` later — not from some quiet minute in the middle of the
    // order.
    let cfg = arena_cfg(0);
    let rally_ms = i64::from(ReflexConfig::default().rally_ms);

    let mut whole = field_placed(&cfg, NARROW_SIGHT_CELLS, &[], [(1, 8), (1, 9), (13, 13)]);
    park(&mut whole, BLUE);
    let mut bytes = Vec::new();
    run_counted(&mut whole, 2_000, &mut bytes);
    issue(&mut whole, &order(Objective::Withdraw, 10_000));
    let at = whole.now_ms;
    run_counted(&mut whole, at + 11_000, &mut bytes);
    assert_eq!(
        state_of(&whole, RED_A),
        Some(ReflexState::Patrol),
        "a whole hull comes back the moment the order stops standing"
    );

    let mut hurt = field_placed(&cfg, NARROW_SIGHT_CELLS, &[], [(1, 8), (1, 9), (13, 13)]);
    park(&mut hurt, BLUE);
    hurt_to_withdraw(&mut hurt, RED_A);
    let mut bytes = Vec::new();
    run_counted(&mut hurt, 2_000, &mut bytes);
    issue(&mut hurt, &order(Objective::Withdraw, 10_000));
    let at = hurt.now_ms;
    run_counted(&mut hurt, at + 11_000, &mut bytes);
    assert_eq!(
        state_of(&hurt, RED_A),
        Some(ReflexState::Withdraw),
        "`Withdraw` is terminal for a hurt tank until it has rallied"
    );
    let started = hurt
        .reflex
        .unit(RED_A)
        .and_then(|row| row.quiet_since_ms)
        .expect("the quiet clock starts at the expiry and not before it");
    assert!(
        started >= at + 10_000,
        "the clock had been running under the order: {started} ms"
    );
    let from = hurt.now_ms;
    run_counted(&mut hurt, from + rally_ms + 2_000, &mut bytes);
    assert_eq!(state_of(&hurt, RED_A), Some(ReflexState::Patrol));
    assert!(
        hurt.reflex.unit(RED_A).is_some_and(|row| row.rallied),
        "and it spent its one comeback here"
    );
}

/// Take a tank under `withdraw_permille` without touching anything else: one
/// plan, the hull's own two cells.
fn hurt_to_withdraw(arena: &mut Arena, unit: ObjId) {
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
        let state = arena.unit(unit).expect("the tank");
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = hurt;
    }
}

/// **16b cut E.2** (16c `T-56`): the signal carries the squad's own
/// deliberation floor as the time to live an order that names none gets, clamped
/// into the order vocabulary's own bounds.
#[test]
fn e2_t56_the_signal_carries_the_floor_as_the_default_time_to_live() {
    for (floor, want) in [
        (30_000_u32, 30_000_u32),
        // Under `MIN_TTL_MS`: an operator may set a floor of half a second, and
        // an order shorter than the vocabulary allows is not an order.
        (500, 1_000),
        // And over `MAX_TTL_MS`.
        (900_000, 600_000),
    ] {
        let mut arena = field(&arena_cfg(floor));
        park(&mut arena, BLUE);
        let mut bytes = Vec::new();
        let signals = run(&mut arena, 1_000, &mut bytes);
        let signal = signals
            .first()
            .unwrap_or_else(|| panic!("the opening signal at a floor of {floor}"));
        assert_eq!(signal.default_ttl_ms, want, "floor {floor}");
        // And the brief the world builds from it carries the same number, which
        // is what `order_of` reads across the async hop.
        let brief = tacsim::deliberation::SquadBrief::from_signal(1, signal, 2);
        assert_eq!(brief.default_ttl_ms, want);
    }
}

// ---------------------------------------------------------------------------
// 16b cut AA.1 — the `Advance` a machine that cannot close cannot carry out
// (16c `B-76`, the order layer)
// ---------------------------------------------------------------------------

/// **An `Advance` ends for the machine that cannot carry it out, and for no
/// other** (16b cut AA.1; 16c `B-76`).
///
/// An `Advance` suppresses the hostile edges (16b cut N.2), and `B-41` ends one
/// for a unit only when the search to the ordered cell FAILS. For a machine
/// whose engine is dead the search succeeds, the route stands and it never
/// arrives, so before this cut the order lived to its TTL — a pacifist for up to
/// `MAX_TTL_MS`, and the squad signal carries no mobility fact, so the commander
/// could send it again. Now it ends on the unit's next decide tick through
/// `B-41`'s own `plan.expired`: one `order_expired` line for that unit, the
/// squad's next signal says `OrderExpired`, and from that tick the unit is on
/// doctrine and fights what it holds.
///
/// RED_A is immobilised the way `damage.rs`'s `disable_motion` does it
/// (`limits.motion_disabled`, speed and ordered speed to nought); RED_B, its
/// mobile squadmate under the same order, keeps it. Blue's tank is parked so the
/// field stays where it was built; at eleven cells it is well inside RED_A's
/// 35-cell reach with a clear line on open ground. The floor is nought so the
/// signal the expiry asks for is reported on the pulse after it.
#[test]
fn aa1_an_advance_ends_for_the_machine_that_cannot_carry_it_out() {
    use tacsim::arena::ledger::EventKind;

    let cfg = arena_cfg(0);
    let mut arena = field(&cfg);
    park(&mut arena, BLUE);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    if let Some(state) = arena.unit_mut(RED_A) {
        state.limits.motion_disabled = true;
        state.speed_cms = 0;
        state.desired_speed_cms = 0;
    }
    let stands = arena.unit(RED_A).map(|unit| unit.pose.clone());

    // A plainly reachable cell four columns from red's muster, on open ground.
    let to = (6, 8);
    issue(&mut arena, &order(Objective::Advance { to }, 60_000));
    let at = arena.now_ms;
    let mut signals = run(&mut arena, at + 500, &mut bytes);

    let log = events(&mut arena, &mut bytes);
    let expired: Vec<(ObjId, String)> = log
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::OrderExpired {
                unit, objective, ..
            } => Some((*unit, objective.to_string())),
            _ => None,
        })
        .collect();
    assert_eq!(
        expired,
        vec![(RED_A, "advance".to_string())],
        "one line, for the machine that cannot close, on its first decide tick"
    );
    assert!(
        arena.reflex.order(RED_A).is_none(),
        "the order is over for RED_A"
    );
    // The mobile squadmate keeps it and is making for the cell.
    assert!(
        arena.reflex.order(RED_B).is_some(),
        "RED_B can carry the order out and still holds it"
    );
    assert_eq!(goal_of(&arena, RED_B), Some(to));

    // On doctrine, it fights what it holds — from where it stands.
    let now = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, now);
    signals.extend(run(&mut arena, now + 10_000, &mut bytes));
    assert!(
        fired_by(&events(&mut arena, &mut bytes), RED_A) > 0,
        "off the order, the immobile machine shoots at the hostile it holds"
    );
    assert_eq!(state_of(&arena, RED_A), Some(ReflexState::Engage));
    assert_eq!(
        arena.unit(RED_A).map(|unit| unit.pose.clone()),
        stands,
        "and it never moved"
    );

    // And the commander hears, through the one channel `B-41` already uses.
    assert!(
        signals.iter().any(|signal| signal.squad == red_one()
            && signal.reason == tacsim::arena::command::SignalReason::OrderExpired),
        "the squad never asked for a new order: {:?}",
        signals
            .iter()
            .map(|signal| (signal.squad, signal.reason))
            .collect::<Vec<_>>()
    );
}
