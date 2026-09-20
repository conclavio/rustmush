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
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
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

fn definition() -> ArenaDefinition {
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.deliberation"),
        terrain: ArenaTerrain::new(
            SIDE,
            SIDE,
            CELL_CM,
            vec![ground(); usize::from(SIDE) * usize::from(SIDE)],
        )
        .expect("the test window"),
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

/// A **three-cell** scope by default. The three tanks stand eleven cells apart, so
/// nothing is held unless a test plants it — which is what lets a steering test
/// say "this is the order and not the geometry".
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

fn tank(id: ObjId, team: Team, pose: Pose, controller: Controller) -> CustodyUnit {
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
        sensor: Some(sight(3)),
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
/// blue tank (`blue-1`), all on the reflex controller.
fn field(cfg: &ArenaConfig) -> Arena {
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units: vec![
                tank(RED_A, Team::Red, centre(2, 8), Controller::Reflex),
                tank(RED_B, Team::Red, centre(2, 9), Controller::Reflex),
                tank(BLUE, Team::Blue, centre(13, 8), Controller::Reflex),
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
fn a_withdraw_order_withdraws_and_holds_fire() {
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
    let fired = events(&mut arena, &mut bytes);
    assert_eq!(
        fired_by(&fired, RED_A) + fired_by(&fired, RED_B),
        0,
        "and it holds its fire on the way"
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
    // The cannon reaches 2,500 m; `close` is a third of that, and the two tanks are
    // eleven cells (110 m) apart — well inside `close`, so the band must NOT stop
    // this one. `RangeBand::Close` on a 2,500 m gun is 833 m.
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
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
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
    let after = goal_of(&arena, RED_A);
    assert!(
        after == Some((1, 8)) || after == Some((14, 8)),
        "doctrine is the fallback, not idle: the machine is back on its own graph, not {after:?}"
    );
    assert_eq!(
        state_of(&arena, RED_A),
        Some(ReflexState::Patrol),
        "and it is patrolling, not holding where the order left it"
    );
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
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
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
