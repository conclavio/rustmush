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

// 17b cut M4.5a: the presenting profile (ruling 1) and the stock recorder
// (`Stock`, `think_at`) the ported fixtures read the mind's own figures off.
mod support;

use mush_core::{
    AccountId, AccountIdentity, Audience, Ctx, GOD, ObjId, Outbox, ParsedArgs, SwitchSet,
};
use mush_proto::OutputEvent;
use mush_sim::data::Pose;
use mush_tactics::{
    CellAddr, CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
    TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::doctrine::MachineState;
use tacsim::arena::intents::{Intent, IntentKind, IntentOutcome, Refusal, apply_intent};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::perception::{SensorProfile, SignatureFactor};
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
    CharacterRef, Constraints, Objective, Order, RangeBand, SquadId, squads_of,
};

// `Stock::on` records the arena-wide fallback runner every native-sealed mind
// on these fields thinks on — the native stock mind, answered untouched — so
// swapping it into `Arena::mind_runner` changes no mind's answer, blue's
// included.
use support::scripted::{Stock, think_at};
use tacsim::arena::mind::STOCK_THINK_MS;

// ---------------------------------------------------------------------------
// The fixtures (reflex.rs's, with the order's two extra needs)
// ---------------------------------------------------------------------------

const SIDE: u16 = 16;
const CELL_CM: u16 = 1_000;
const SEED: u64 = 2_350;
// 17b cut M4.5a: `think_at` (and so `lands_at`) staggers on the shared rig's
// seed, and every exact lattice pin below reads it against arenas built on
// this one — so the two are one figure, asserted.
const _: () = assert!(SEED == support::rig::SEED);

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

/// **17b cut L.1**: `squad`'s lance-leader as the OOC sim mints it — the author
/// of every scripted machine order and every deliberated brief here.
fn persona(squad: SquadId) -> CharacterRef {
    CharacterRef::Persona { squad }
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
    // 17b cut M3.1 (plan F6): a Class I core in the hull, through
    // `tacsim::definitions::core::core_component` — the shape `bind` lays
    // down from content — so the hull holds two condition slots and a mind
    // on it thinks at Class I's figures, as it did before cores existed.
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
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: Vec::new(),
        components: vec![core],
        capabilities: vec![capability],
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
        modes: Vec::new(),
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
        component: smol_str::SmolStr::new("visual_sensor"),
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
        // 17b cut M4.5a (Q17, ruling 1): every tank PRESENTS its bands, so a
        // mind's armed slot has odds to clear its floor with — a
        // `UnitProfile::default()` presents none and a mind never fires at it,
        // where the reflex fired regardless. Every `fired_by > 0` claim below,
        // and every `== 0` control it makes discriminating, leans on it.
        profile: support::rig::presenting_profile(),
        gunnery: 0,
        controller,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: tactical_unit(id),
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// Decide every 500 ms and re-plan freely, so an edge is visible in a handful of
/// pulses.
///
/// **17b cut M4.5a** (F2): the tanks are minds, and a mind is paced by its
/// class row (Class I: a think every `STOCK_THINK_MS` on its stagger
/// [`think_at`], its answer landing one 250 ms substep later), not by
/// `decide_ms` — which now paces the motor's `MotorTick` alone. The cfg is
/// kept (the motor's tick, the free re-plan); every window below that was
/// sized for the reflex's 500 ms tick is grown by at most one think plus a
/// landing where the test says so.
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
/// blue tank (`blue-1`), all flown by unit minds on Standard Doctrine (17b cut
/// M4.5a; the reflex controller before it), each carrying
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
    field_flown(cfg, sight_cells, paint, cells, Controller::Mind)
}

/// [`field_placed`] with the three tanks under `controller` — `Mind` for every
/// test since 17b cut M4.6b ported the last reflex fielding (the STOP B legs
/// of [`e1_an_ordered_close_caps_the_squad_at_the_pilots_own_close`], which
/// flew the reflex's default-profile tanks through a `reflex_field` from 17b
/// cut M4.5a's Q18 until then).
fn field_flown(
    cfg: &ArenaConfig,
    sight_cells: u16,
    paint: &[(u16, u16, TerrainCell)],
    cells: [(u16, u16); 3],
    controller: Controller,
) -> Arena {
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition_painted(paint),
            units: vec![
                tank_seeing(
                    RED_A,
                    Team::Red,
                    centre(cells[0].0, cells[0].1),
                    controller,
                    sight_cells,
                ),
                tank_seeing(
                    RED_B,
                    Team::Red,
                    centre(cells[1].0, cells[1].1),
                    controller,
                    sight_cells,
                ),
                tank_seeing(
                    BLUE,
                    Team::Blue,
                    centre(cells[2].0, cells[2].1),
                    controller,
                    sight_cells,
                ),
            ],
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
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
///
/// 17b cut M4.5a: settled (`Arena::step_settled`), so the mind rows the tests
/// read between pulses hold every think the pulse collected.
fn run(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> Vec<tacsim::arena::command::SquadSignal> {
    let mut wall = arena.now_ms;
    let mut signals = Vec::new();
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
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
/// counted per shooter, because a field of three machine tanks has two shooters
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

/// **What the machine flying `unit` is doing** — 17b cut M4.5a: the mind's
/// cached doctrine state through `Arena::machine_state` (`Startup` before its
/// first landing, `None` for a pilot's hull), where it was the reflex row's.
fn state_of(arena: &Arena, unit: ObjId) -> Option<MachineState> {
    arena.machine_state(unit)
}

/// **The stock memory `unit`'s mind holds** (17b cut M4.5a): Standard
/// Doctrine's keys off the mind row — `target`, `stand_until_ms`,
/// `quiet_since_ms`, `rallied` and `ignore_until_ms` are read here where the
/// reflex row carried them.
fn memory_of(arena: &Arena, unit: ObjId) -> tacsim::arena::stock_mind::StockMemory {
    support::rig::stock_memory_of(arena, unit)
}

/// **The standing order on `unit`'s mind row** (17b cut M4.5a; D-UM-15): the
/// squad's order re-keyed to the hull when it was written — where the reflex
/// row's `order` was. The arena clears it at the first landing at or past its
/// `expires_ms`; a mind that ends it early only stops obeying it
/// (`StockMemory::ignore_until_ms`, the divergence `order_end`), so it stands
/// here until then.
fn order_on(arena: &Arena, unit: ObjId) -> Option<&tacsim::arena::perception::FrameOrder> {
    arena.minds.get(&unit).and_then(|row| row.order.as_ref())
}

/// **How many minds stand under an order at `t_ms`** (17b cut M4.5a) — the
/// reflex's `under_order` over the mind rows: an order whose `expires_ms` is
/// past `t_ms`.
fn under_order(arena: &Arena, t_ms: i64) -> usize {
    arena
        .minds
        .values()
        .filter(|row| {
            row.order
                .as_ref()
                .is_some_and(|order| order.expires_ms > t_ms)
        })
        .count()
}

/// **Whether a mind still OBEYS its standing order** (17b cut M4.5a): the
/// order stands on the row and its `expires_ms` is not the one the mind wrote
/// to `ignore_until_ms` when it ended it (the stock mind's `obeyed`).
fn obeys(arena: &Arena, unit: ObjId) -> bool {
    order_on(arena, unit)
        .is_some_and(|order| Some(order.expires_ms) != memory_of(arena, unit).ignore_until_ms)
}

/// The goal a machine is driving to — its hull's motor row's since 17b cut
/// M1a.1.
fn goal_of(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    arena.motors.row(unit).and_then(|motor| motor.goal)
}

/// **The landing of `unit`'s think `n`** (17b cut M4.5a): a mind thinks at
/// [`think_at`] — every `STOCK_THINK_MS` (1,000 ms) on its stagger at the
/// fixtures' seed (`#4` at 482, `#6` at 4, `#9` at 189) — and its answer lands
/// one substep (250 ms) later, where it acts: the millisecond every window
/// below that was sized for the reflex's own tick is re-derived against.
fn lands_at(arena: &Arena, unit: ObjId, n: i64) -> i64 {
    think_at(unit, n).saturating_add(arena.substep_len_ms())
}

/// **The goal `unit`'s mind named** (17b cut M4.5a): the memory's `goal` —
/// the cell the mind last handed its motor, which the motor may have fanned
/// (`Arena::fan_goal`, the M1a plan's departure (d)) when it lies in the
/// other side's entry zone beside a squadmate's goal, and which the motor
/// drops (and the mind forgets) once the hull holds (`Goal(Hold)`).
fn named_goal(arena: &Arena, unit: ObjId) -> Option<(u16, u16)> {
    memory_of(arena, unit).goal
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
        issuer: None,
        // 17b cut L.1: the squad's lance-leader gives it, in the commander's scope.
        by: persona(red_one()),
        spot: None,
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
            apply_intent(arena, &intent, tacsim::arena::intents::IntentSource::Seat)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// (0) The grouping the whole cut rests on
// ---------------------------------------------------------------------------

#[test]
fn the_arena_and_the_grouping_agree_on_who_commands() {
    let cfg = arena_cfg(30_000);
    let arena = field(&cfg);
    let by_hand = squads_of(
        &[
            (RED_A, Team::Red, true),
            (RED_B, Team::Red, true),
            (BLUE, Team::Blue, true),
        ],
        cfg.squad_size,
    );
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

    // **16b cut Z.1** (16c N6): five machines on red at the shipped size — what
    // `SIM AI 6` with one human fields — are a three and a two, each commanded
    // by the lowest id of its own run, never a four and a one.
    assert_eq!(cfg.squad_size, 4, "the shipped lance");
    let lance = field_of(&cfg, &five_and_two());
    let by_hand = squads_of(
        &lance
            .units
            .iter()
            .map(|unit| (unit.id, unit.team, true))
            .collect::<Vec<_>>(),
        cfg.squad_size,
    );
    assert_eq!(lance.squads, by_hand);
    let shape: Vec<(String, ObjId, Vec<ObjId>)> = lance
        .squads
        .iter()
        .map(|roster| {
            (
                roster.id.to_string(),
                roster.commander,
                roster.units.clone(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                "red-1".to_string(),
                ObjId(20),
                vec![ObjId(20), ObjId(21), ObjId(22)]
            ),
            ("red-2".to_string(), ObjId(23), vec![ObjId(23), ObjId(24)]),
            ("blue-1".to_string(), ObjId(30), vec![ObjId(30), ObjId(31)]),
        ]
    );

    // **17b cut L.2** (D-UM-2, D-UM-53; `Controller::is_machine`): a hull a
    // mind flies is dealt as a reflex one is — here it is red's lowest id, so
    // it commands — while a seated pilot's hull is still in no squad. The
    // triples come from `CustodyUnit::squad_input`, the one builder
    // `match_::start` and `Arena::new` both call over the check-out, so the
    // record's grouping and the arena's are one function of one data with a
    // mind in it too (production fields no Mind hull before M4, so `start`
    // cannot be driven with one; the shared builder is what keeps its side
    // from drifting).
    //
    // **17b cut M4.5a**: the whole field is minds and the seat — RED_B was the
    // one reflex hull here, and every claim is kept on a mind in its place
    // (both are machines to `Controller::is_machine`); only the message's
    // "a reflex hull" half ceases, the controller it named being retired.
    const PILOTED: ObjId = ObjId(5);
    let units = vec![
        tank(RED_A, Team::Red, centre(2, 8), Controller::Mind),
        tank(PILOTED, Team::Red, centre(2, 10), Controller::Pilot),
        tank(RED_B, Team::Red, centre(2, 9), Controller::Mind),
        tank(BLUE, Team::Blue, centre(13, 8), Controller::Mind),
    ];
    let triples = CustodyUnit::squad_input(&units);
    assert_eq!(
        triples,
        vec![
            (RED_A, Team::Red, true),
            (PILOTED, Team::Red, false),
            (RED_B, Team::Red, true),
            (BLUE, Team::Blue, true),
        ],
        "a mind's hull is a machine, a pilot's is not"
    );
    let minded = Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots: vec![Pilot {
                unit: PILOTED,
                pilot: ObjId(1_000),
                station: SmolStr::new("cockpit"),
                epoch: 1,
            }],
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    assert_eq!(
        minded.squads,
        squads_of(&triples, cfg.squad_size),
        "the record's grouping is the arena's, with a mind in it"
    );
    let shape: Vec<(String, ObjId, Vec<ObjId>)> = minded
        .squads
        .iter()
        .map(|roster| {
            (
                roster.id.to_string(),
                roster.commander,
                roster.units.clone(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("red-1".to_string(), RED_A, vec![RED_A, RED_B]),
            ("blue-1".to_string(), BLUE, vec![BLUE]),
        ],
        "the minds are dealt and the lower id commands"
    );
    assert!(
        minded.squad_of(PILOTED).is_none(),
        "a pilot's tank is in no squad"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.1 — the squad's own picture, and a lance of five at three cadences
// (16c N6; doc 16 §3.6 read for machines, §6 "Deliberation")
// ---------------------------------------------------------------------------

/// An arena over the plain window with `units` (id, team, cell), every one flown
/// by a unit mind (17b cut M4.5a; the reflex controller before it) at the wide
/// scope — [`field_placed`]'s shape for any number of tanks.
fn field_of(cfg: &ArenaConfig, units: &[(ObjId, Team, (u16, u16))]) -> Arena {
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units: units
                .iter()
                .map(|(id, team, (x, y))| tank(*id, *team, centre(*x, *y), Controller::Mind))
                .collect(),
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    arena
}

/// Five machines on red down column 2 and two on blue in column 13: at the
/// shipped size red is `red-1` (`#20`–`#22`) and `red-2` (`#23`, `#24`).
fn five_and_two() -> Vec<(ObjId, Team, (u16, u16))> {
    vec![
        (ObjId(20), Team::Red, (2, 6)),
        (ObjId(21), Team::Red, (2, 7)),
        (ObjId(22), Team::Red, (2, 8)),
        (ObjId(23), Team::Red, (2, 9)),
        (ObjId(24), Team::Red, (2, 10)),
        (ObjId(30), Team::Blue, (13, 7)),
        (ObjId(31), Team::Blue, (13, 9)),
    ]
}

/// **16b cut Z.1: a reported squad's signal carries its own members.**
///
/// `red-1` is `#4` at (2, 8), `#6` taken out of the arena before the first pulse
/// (a unit that left the match stays on the roster; the arena skips it), `#7`
/// three cells east at (5, 8) and hurt, and `#8` one cell south at (2, 9). The
/// opening signal reads `#4` at ordinal 1 with `0`/`0`, then `#7` and `#8` at
/// ordinals 2 and 3 in roster order, each with its own health and its own place
/// from the commander, and the brief bands each in the word a pilot's own
/// `CONTACTS` row would use at that range.
#[test]
fn z1_a_reported_squad_carries_its_members_in_roster_order() {
    const GONE: ObjId = ObjId(6);
    const EAST: ObjId = ObjId(7);
    const SOUTH: ObjId = ObjId(8);
    let cfg = arena_cfg(30_000);
    let mut arena = field_of(
        &cfg,
        &[
            (RED_A, Team::Red, (2, 8)),
            (GONE, Team::Red, (2, 10)),
            (EAST, Team::Red, (5, 8)),
            (SOUTH, Team::Red, (2, 9)),
            (BLUE, Team::Blue, (13, 8)),
        ],
    );
    assert_eq!(
        arena.squads.first().map(|roster| roster.units.clone()),
        Some(vec![RED_A, GONE, EAST, SOUTH]),
        "four machines are one lance"
    );
    arena.units.retain(|unit| unit.id != GONE);
    hurt_to_withdraw(&mut arena, EAST);
    let hurt = arena.health_permille(EAST);
    assert!(hurt < 1_000, "the hurt member reads its own hull: {hurt}");
    let whole = arena.health_permille(SOUTH);

    // One pulse, the opening one, so the states the signal carries are the
    // states the controller holds as it is read.
    let signals = arena.step_settled(0, &[], &[]).deliberation;
    let signal = signals
        .iter()
        .find(|signal| signal.squad == red_one())
        .expect("red-1 is reported on the opening pulse");
    let rows: Vec<(ObjId, u16, u32)> = signal
        .members
        .iter()
        .map(|member| (member.unit, member.ordinal, member.health_permille))
        .collect();
    assert_eq!(
        rows,
        vec![(RED_A, 1, 1_000), (EAST, 2, hurt), (SOUTH, 3, whole)],
        "roster order, the commander first, a unit the arena no longer holds absent"
    );
    let lead = &signal.members[0];
    assert_eq!((lead.range_cm, lead.bearing_mdeg), (0, 0));
    let east = &signal.members[1];
    assert_eq!(east.range_cm, 3 * u64::from(CELL_CM), "three cells east");
    assert_eq!(east.bearing_mdeg, 90_000);
    let south = &signal.members[2];
    assert_eq!(south.range_cm, u64::from(CELL_CM));
    // The state each member holds of itself, `Startup` until its controller
    // has a row (17b cut M4.5a: a mind's cached doctrine state, `Startup`
    // until its first doctrine landing — nobody has thought at millisecond 0).
    for member in &signal.members {
        assert_eq!(
            member.state,
            state_of(&arena, member.unit).unwrap_or(MachineState::Startup),
            "{:?}",
            member.unit
        );
    }

    // The brief: the pilot's band word, `here` for the commander, and the
    // object ids on the seam but never in the prompt.
    let brief = tacsim::deliberation::SquadBrief::from_signal(1, signal, 3, persona(signal.squad));
    assert_eq!(brief.members.len(), 3);
    assert_eq!(brief.members[0].band, "here");
    assert_eq!(
        brief.members[1].band,
        tacsim::match_::cockpit::contact_band(east.range_cm, &signal.bands_cm, signal.sight_cm)
    );
    assert_eq!(
        brief.members[2].band,
        tacsim::match_::cockpit::contact_band(south.range_cm, &signal.bands_cm, signal.sight_cm)
    );
    assert_eq!(brief.members[1].bearing_deg, 90);
    assert_eq!(brief.members[1].unit, EAST.0);
    let prompt = tacsim::deliberation::request::user_prompt(&brief);
    // The opening pulse is before `startup_ms`: every machine is still starting,
    // and the prompt says so in the controller's own word.
    assert!(
        prompt.contains("member 1 (commander): state startup, health 1000\n"),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "member 2: state startup, health {hurt}, {}, bearing 90\n",
            brief.members[1].band
        )),
        "{prompt}"
    );
    assert!(!prompt.contains("member 4"), "{prompt}");
}

/// **16b cut Z.1: each member's state is its own machine's, read as the signal
/// is taken.**
///
/// The opening pulse above is before `startup_ms`, where every machine reads
/// `Startup` and a signal that wrote `Startup` for everyone — or the
/// commander's own state for every member — would pass. So this one steps the
/// same lance, at a zero floor so every change of the commander's contact set
/// is reported, pulse by pulse past `startup_ms + decide_ms`, and reads the
/// first `red-1` signal after it against the controller at that very pulse:
/// the hurt `#7` has withdrawn, the commander has not, and nobody is starting.
///
/// **17b cut M4.5a — on the unit mind.** The members are minds and each
/// signalled state is the mind's cached doctrine state (`Arena::machine_state`,
/// what the signal reads). One constant re-derived: the reflex was settled at
/// `startup_ms + decide_ms` (+ a pulse); a mind starts up at its first think's
/// landing, is online `startup_ms` later (inside its think period), and lands
/// its first doctrine state at its SECOND think's landing — so `settled` is the
/// last member's second landing on the think lattice (`#8` 1,508, `#4` 1,732,
/// `#7` 1,844), past which nobody can read `Startup`.
#[test]
fn z1_a_later_signal_reads_each_members_own_state() {
    const EAST: ObjId = ObjId(7);
    const SOUTH: ObjId = ObjId(8);
    let cfg = arena_cfg(0);
    let mut arena = field_of(
        &cfg,
        &[
            (RED_A, Team::Red, (2, 8)),
            (EAST, Team::Red, (5, 8)),
            (SOUTH, Team::Red, (2, 9)),
            (BLUE, Team::Blue, (13, 8)),
        ],
    );
    hurt_to_withdraw(&mut arena, EAST);
    // Each member's second landing — its first think after it came online.
    assert!(
        i64::from(match_cfg().startup_ms) + arena.substep_len_ms() <= STOCK_THINK_MS,
        "the precondition: a mind is online before its second think"
    );
    let settled = [RED_A, EAST, SOUTH]
        .into_iter()
        .map(|member| lands_at(&arena, member, 1))
        .max()
        .unwrap_or(0);
    let mut wall = arena.now_ms;
    let mut read = None;
    while arena.now_ms < 20_000 {
        let out = arena.step_settled(wall, &[], &[]);
        wall += 500;
        if let Some(signal) = out
            .deliberation
            .into_iter()
            .find(|signal| signal.squad == red_one() && signal.at_ms >= settled)
        {
            // The controller's states at the pulse the signal was taken at.
            let held: Vec<Option<MachineState>> = signal
                .members
                .iter()
                .map(|member| state_of(&arena, member.unit))
                .collect();
            read = Some((signal, held));
            break;
        }
        assert!(
            out.ended.is_none(),
            "the lance fought to the end unreported"
        );
    }
    let (signal, held) = read.expect("red-1 is reported again once its commander's contacts move");
    let states: Vec<(ObjId, MachineState)> = signal
        .members
        .iter()
        .map(|member| (member.unit, member.state))
        .collect();
    assert_eq!(
        held,
        states
            .iter()
            .map(|(_, state)| Some(*state))
            .collect::<Vec<_>>(),
        "each member's state is its own machine's: {states:?}"
    );
    assert_eq!(
        states.iter().map(|(unit, _)| *unit).collect::<Vec<_>>(),
        vec![RED_A, EAST, SOUTH]
    );
    assert_eq!(
        states[1].1,
        MachineState::Withdraw,
        "the hurt member has withdrawn: {states:?}"
    );
    assert!(
        states[0].1 != MachineState::Withdraw && states[0].1 != MachineState::Startup,
        "the commander is whole and running, so its state is not the hurt member's: {states:?}"
    );
    assert!(
        states
            .iter()
            .all(|(_, state)| *state != MachineState::Startup),
        "nobody is still starting past each member's second landing: {states:?}"
    );
    let brief = tacsim::deliberation::SquadBrief::from_signal(1, &signal, 3, persona(signal.squad));
    let prompt = tacsim::deliberation::request::user_prompt(&brief);
    assert!(
        prompt.contains(&format!(
            "member 2: state withdraw, health {}, {}, bearing {}\n",
            brief.members[1].health_permille, brief.members[1].band, brief.members[1].bearing_deg
        )),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "member 1 (commander): state {}, health ",
            states[0].1.as_str()
        )),
        "{prompt}"
    );
}

// The cross-tempo property for the grouping (16b §11): one scripted log over a
// five-machine side at 5,000, 1,000 and 500 ms. The squads are fixed at
// `Arena::new`, so the roster is compared as a guard that nothing about the
// cadence reaches the grouping; the signals are compared on the **opening** pulse
// only — arena millisecond 0, where every cadence coincides and every squad reads
// `NoOrder`. The later signal lists are not compared, on purpose:
// `take_deliberation_signals` runs once at the end of each pulse and `SquadWatch`
// is a view the hash excludes, so a `ContactsChanged` lands at a different
// millisecond at each cadence by design.

/// One lance replay.
struct LanceReplay {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    finals: Vec<OrderedFinal>,
    roster: Vec<(String, ObjId, Vec<ObjId>)>,
    opening: Vec<tacsim::arena::command::SquadSignal>,
}

fn replay_lance(cadence_ms: i64, until_ms: i64) -> LanceReplay {
    let cfg = ArenaConfig {
        squad_size: 4,
        ..arena_cfg(30_000)
    };
    let mut arena = field_of(&cfg, &five_and_two());
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let red_two = SquadId {
        team: Team::Red,
        index: 2,
    };
    // The script: `red-1` holds where it stands for twenty seconds and `red-2`
    // advances on the crossing for ten, both on the opening pulse.
    let hold = order(Objective::Hold, 20_000);
    let advance = Order {
        squad: red_two,
        ..order(Objective::Advance { to: (9, 3) }, 10_000)
    };
    let mut burst: Vec<Intent> = Vec::new();
    for told in [&hold, &advance] {
        for unit in arena
            .squads
            .iter()
            .filter(|roster| roster.id == told.squad)
            .flat_map(|roster| roster.units.clone())
        {
            burst.push(Intent {
                unit,
                channel: Channel::Helm,
                epoch: 0,
                kind: IntentKind::Order(Box::new(told.clone())),
                seq: burst.len() as u64,
            });
        }
    }
    assert_eq!(burst.len(), 5, "every red machine is ordered");
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut opening = Vec::new();
    let mut wall = 0_i64;
    let mut first = true;
    loop {
        // 17b cut M4.5a: every three-cadence replay steps `step_settled`, so
        // the arena between two pulses is the task's (no think unanswered).
        let out = if first {
            arena.step_settled(wall, &[], &burst)
        } else {
            arena.step_settled(wall, &[], &[])
        };
        if first {
            assert_eq!(arena.now_ms, 0, "the opening pulse is arena millisecond 0");
            opening = out.deliberation.clone();
            first = false;
        }
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
    let finals = finals_of(&arena);
    let roster = arena
        .squads
        .iter()
        .map(|roster| {
            (
                roster.id.to_string(),
                roster.commander,
                roster.units.clone(),
            )
        })
        .collect();
    LanceReplay {
        events,
        hashes,
        finals,
        roster,
        opening,
    }
}

/// **17b cut M4.5a — on unit minds**: the seven tanks are minds, every replay
/// steps `step_settled`, and the finals read each machine's state through
/// `Arena::machine_state` and the order flag off the mind rows (each order
/// cleared by the arena at its mind's first landing past its expiry). The
/// `fired` line needs the presenting profile (ruling 1). No window moved.
#[test]
fn z1_a_lance_of_five_is_one_grouping_at_three_cadences() {
    let cadences = [5_000_i64, 1_000, 500];
    let runs: Vec<LanceReplay> = cadences
        .into_iter()
        .map(|cadence| replay_lance(cadence, 30_000))
        .collect();
    let first = runs.first().expect("three replays");

    // Worth comparing before it is compared: the grouping is the dealt one, the
    // opening pulse reported every squad with its members, both orders landed
    // and ran out, and the sides fought.
    assert_eq!(
        first
            .roster
            .iter()
            .map(|(id, commander, units)| (id.as_str(), *commander, units.len()))
            .collect::<Vec<_>>(),
        vec![
            ("red-1", ObjId(20), 3),
            ("red-2", ObjId(23), 2),
            ("blue-1", ObjId(30), 2)
        ]
    );
    assert_eq!(first.opening.len(), 3, "every squad asks at millisecond 0");
    for signal in &first.opening {
        assert_eq!(signal.reason, tacsim::arena::command::SignalReason::NoOrder);
        assert_eq!(signal.at_ms, 0);
        assert_eq!(
            signal.members.first().map(|member| member.unit),
            Some(signal.commander)
        );
    }
    assert_eq!(
        first
            .opening
            .iter()
            .map(|signal| signal.members.len())
            .collect::<Vec<_>>(),
        vec![3, 2, 2]
    );
    let kinds: Vec<&str> = first
        .events
        .iter()
        .map(|(_, kind, _)| kind.as_str())
        .collect();
    for want in ["startup", "order_expired", "fired"] {
        assert!(
            kinds.iter().any(|kind| kind.contains(want)),
            "the replay never journalled a {want}: {kinds:?}"
        );
    }
    assert!(
        first.finals.iter().all(|row| !row.7),
        "every order had run out by the end"
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
        assert_eq!(
            run.roster, first.roster,
            "the roster differed at {cadence} ms"
        );
        assert_eq!(
            run.opening, first.opening,
            "the opening pulse's signals differed at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (1) An order steers a squad
// ---------------------------------------------------------------------------

/// **An `advance` moves the patrol target** — 17b cut M4.5a, on the unit
/// mind: red-1's minds take the order through `IntentKind::Order` (the row's
/// re-keyed `FrameOrder`) and make for the cell at their next landing (`#6`'s
/// think at 2,004 lands at 2,254, `#4`'s at 2,482 lands at 2,732: both inside
/// the 4,000 the original read at). What moved: doctrine's patrol goal is read
/// as the node the mind NAMED (its memory's `goal`), because the motor fans a
/// goal that lands inside the other side's entry zone off a squadmate's
/// (`Arena::fan_goal`, the M1a plan's departure (d)) — red's two minds both
/// walk to the east node (14, 8) and the second hull's motor row holds the
/// fanned (14, 11) — while the hull still drives (its motor holds a goal).
/// The ordered cell is in no entry zone, so it is the motor row's goal too.
#[test]
fn an_advance_order_moves_the_patrol_target() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    // Online, and patrolling its own graph: the west node of row 8.
    run(&mut arena, 2_000, &mut bytes);
    assert_eq!(state_of(&arena, RED_A), Some(MachineState::Patrol));
    let doctrine = named_goal(&arena, RED_A);
    assert!(
        doctrine == Some((1, 8)) || doctrine == Some((14, 8)),
        "doctrine walks the authored graph, not {doctrine:?}"
    );
    assert!(goal_of(&arena, RED_A).is_some(), "and the hull drives it");

    let told = order(Objective::Advance { to: (9, 3) }, 60_000);
    assert!(
        issue(&mut arena, &told)
            .iter()
            .all(|outcome| *outcome == IntentOutcome::AppliedContinuous),
        "an order to a machine is accepted, latest-wins, and never queues"
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
        Some(MachineState::Patrol),
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

/// **A `hold` parks the squad, and a movement order does not pursue** — 17b
/// cut M4.5a, on the unit mind: under the order the mind takes no target off
/// the hostile planted in its view (Patrol, the memory's `target` empty) and
/// stands. **One re-pin (Q17)**: the reflex parked by writing the cell it
/// stood in as its goal; the stock mind's Hold arm stops the hull with
/// `Goal(Hold)` (the M1a `Goal(Hold)` departure), which lets the motor's goal
/// go — so "its goal is where it stands" is asserted as **no goal on the motor
/// row, none named in the mind's memory, and a desired speed of 0**. The
/// window (5,000) holds three of `#4`'s parked landings (2,732, 3,732, 4,732).
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
        Some(MachineState::Patrol),
        "hold parks: the machine does not take the target it can see"
    );
    assert_eq!(
        memory_of(&arena, RED_A).target,
        None,
        "a movement order drops the target rather than pursuing it"
    );
    assert_eq!(
        arena.unit(RED_A).map(|unit| unit.desired_speed_cms),
        Some(0),
        "and it is ordered to a stop"
    );
    // Q17: the park is `Goal(Hold)` — the motor holds no goal, and the mind
    // names none — where the reflex wrote the cell it stood in.
    assert_eq!(goal_of(&arena, RED_A), None, "it holds where it stands");
    assert_eq!(
        named_goal(&arena, RED_A),
        None,
        "and the mind let its goal go"
    );
}

/// **A `withdraw` withdraws, and a cornered tank stands** — 17b cut M4.5a, on
/// the unit mind. Two things moved, under the orchestrator's ruling that Q17's
/// `Goal(Hold)` departure EXTENDS to the cornered `Withdraw` stand.
///
/// * **The stand lets the goal go.** The reflex kept its home goal on the
///   motor row through the stand, so the original read `#4`'s at 4,000 (cols
///   0..=1) and the Q.7 comment said the stand shoots back "without touching
///   the goal". The stock mind's `Withdraw` arm, cornered, sets `plan.stop`:
///   it emits `Goal(Hold)` (stop here, face there) and its memory's goal goes
///   to `None` (`stock_mind.rs`, the emission after `withdraw_stand`). `#4`,
///   holding the planted hostile, is cornered at its very first `Withdraw`
///   think (2,482, home already beside its zone) and never names a cell, so
///   no mind can show its home cell. `#4`'s stand is re-pinned as the
///   departure — no goal on its motor row, none named, a desired speed of 0
///   — and **the "goal untouched through the stand" half is LOST** (it is
///   the reflex's; recorded here, not re-asserted).
/// * **"It makes for its OWN entry zone" is read on its squadmate** under the
///   same order, and discriminates on both ends: before the order `#6`'s mind
///   names the EAST node (14, 8) and its motor drives it (so a home cell in
///   columns 0..=1 is the order's doing — the west node (1, 8) would pass a
///   bare `x <= 1`); `#6` reads the order at 2,004 before it holds a
///   classified hostile, names its zone's cell abreast of it, (1, 9), and is
///   read at the 2,500 pulse (its next landing, 3,254, is its own stand).
///   That the hull DRIVES the named cell is read off the motor's own word,
///   not its row: `#6` stands at (2, 9), within `ARRIVE_CELLS` of (1, 9), so
///   the goal's landing (2,254) drives it once at that millisecond, arrives
///   and drops it (`Fallback::Notice`) — measured at 1 ms resolution, the
///   motor row holds (14, 8) through 2,253 and nothing from 2,254, so a
///   `goal_of == named_goal` read exists at no millisecond. Instead: the
///   motor row has let the patrol leg go, and `#6`'s next think (3,004)
///   reads the motor's `drive_arrived` (and no `drive_no_route`), which its
///   previous think (2,004, the patrol leg) did not — the only goal the
///   motor held between them that a hull at (2, 9) can reach is (1, 9).
/// * **The shot** needs the presenting profile (ruling 1): `#4`'s first
///   `Withdraw` think locks and fires at its landing (2,732), inside the 4,000.
#[test]
fn a_withdraw_order_withdraws_and_stands_when_it_is_cornered() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    // Before the order `#6` patrols its graph to the EAST node, off red's
    // zone, and its hull drives it.
    let patrolling = named_goal(&arena, RED_B);
    assert_eq!(
        patrolling,
        Some((14, 8)),
        "before the order #6 walks to the east node, off its own zone"
    );
    assert_eq!(
        goal_of(&arena, RED_B),
        patrolling,
        "and its hull drives that patrol leg"
    );
    let at = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    let told = order(Objective::Withdraw, 60_000);
    issue(&mut arena, &told);
    // `#6`'s first `Withdraw` landing is in, its stand is not.
    let first = lands_at(&arena, RED_B, 2);
    let stand = lands_at(&arena, RED_B, 3);
    run(&mut arena, first.saturating_add(1), &mut bytes);
    assert!(arena.now_ms < stand, "read before #6's second landing");
    assert_eq!(state_of(&arena, RED_B), Some(MachineState::Withdraw));
    let goal = named_goal(&arena, RED_B).expect("a withdrawing tank has somewhere to be");
    assert_ne!(
        Some(goal),
        patrolling,
        "the order turned #6 off its patrol leg"
    );
    assert!(
        goal.0 <= 1,
        "it makes for its OWN entry zone (columns 0..=1), not {goal:?}"
    );
    assert_eq!(
        goal,
        (1, 9),
        "the home cell is its own zone's cell abreast of it"
    );
    assert_eq!(
        goal_of(&arena, RED_B),
        None,
        "the motor let the patrol leg go: it drove the home cell at the landing \
         and, one cell off it, arrived"
    );
    run(&mut arena, 4_000, &mut bytes);
    let arrived = |t_ms: i64| {
        stock
            .kept()
            .iter()
            .find(|(unit, kept_ms, _, _)| *unit == RED_B && *kept_ms == t_ms)
            .map(|(_, _, view, _)| {
                (
                    view.notices.iter().any(|notice| {
                        matches!(
                            notice,
                            tacsim::arena::perception::FrameNotice::Drive {
                                outcome: tacsim::arena::perception::DriveOutcome::Arrived,
                            }
                        )
                    }),
                    view.notices.iter().any(|notice| {
                        matches!(
                            notice,
                            tacsim::arena::perception::FrameNotice::Drive {
                                outcome: tacsim::arena::perception::DriveOutcome::NoRoute,
                            }
                        )
                    }),
                )
            })
    };
    assert_eq!(
        arrived(think_at(RED_B, 2)),
        Some((false, false)),
        "the think that named the home cell had not arrived anywhere"
    );
    assert_eq!(
        arrived(think_at(RED_B, 3)),
        Some((true, false)),
        "and the next one reads the motor's word that the hull reached it"
    );

    assert_eq!(
        state_of(&arena, RED_A),
        Some(MachineState::Withdraw),
        "withdraw is the state a commander can reach without being shot at"
    );
    assert_eq!(state_of(&arena, RED_B), Some(MachineState::Withdraw));
    // The cornered stand, on a mind: `Goal(Hold)` — stop here, face there —
    // which lets the goal go (the ruling above; the reflex's "goal untouched"
    // half is lost).
    assert_eq!(goal_of(&arena, RED_A), None, "the stand holds where it is");
    assert_eq!(named_goal(&arena, RED_A), None);
    assert_eq!(
        arena.unit(RED_A).map(|unit| unit.desired_speed_cms),
        Some(0)
    );
    // **16b cut Q.7** changed this. Before it, a commanded withdrawal held its
    // fire all the way home; now a tank that still holds a classified hostile
    // inside its reach, with a line, stops and shoots back — **without leaving
    // `Withdraw`**. (On the reflex also "without touching the goal"; a mind's
    // stand lets its goal go — the `Goal(Hold)` departure the assertions above
    // pin, and that half is lost.) `hold_fire` is the constraint a commander
    // who means "do not shoot" writes, and
    // `hold_fire_stops_the_trigger_and_leaves_the_lock` below is its test.
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
///
/// **17b cut M4.5a — on the unit mind**: the stand's dwell is the stock
/// memory's `stand_until_ms` (`withdraw_stand` answers nothing under
/// `hold_fire`), and the control's stand — which keeps the claim about the
/// constraint — fires and dwells only on a target that presents its bands
/// (ruling 1). The window (6,000) holds four of `#4`'s landings past the order.
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
        Some(MachineState::Withdraw),
        "the order still puts it in `Withdraw`"
    );
    assert_eq!(
        memory_of(&arena, RED_A).stand_until_ms,
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
        memory_of(&free, RED_A).stand_until_ms.is_some(),
        "the cornered stand is there when nothing forbids it"
    );
}

/// **17b cut M4.5a — on the unit mind.** Under `hold_fire` the stock mind's
/// `engage_orders` still plans the lock and returns before it arms a slot or
/// plans a shot, so the hull locks and never fires. The lock is now read as
/// `#4`'s own (`locked_by`), where the original counted any platform's — a
/// strengthening: the squad member whose shots are counted is the one that
/// locks. The free control fires, which needs the presenting profile (ruling
/// 1) and is what keeps the `== 0` about the constraint.
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
        locked_by(&under_order, RED_A) > 0,
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

/// **17b cut M4.5a — on the unit mind.** The claim is about the band's edge, so
/// under ruling 1 it is asserted on the figure the mind READS as well: every
/// think of `#4` after the order read its target inside the ordered ceiling
/// (the true 11,000 cm publishes 100 m; it then closes to 50 m and engages at
/// seven tenths of the ceiling), and it shoots — which needs the presenting
/// profile. The window (6,000) holds `#4`'s engaging landing (5,732).
#[test]
fn a_close_range_band_keeps_the_squad_from_shooting_across_the_window() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
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
    let read: Vec<u64> = stock
        .kept()
        .iter()
        .filter(|(unit, t_ms, _, _)| *unit == RED_A && *t_ms > at)
        .filter_map(|(_, _, view, result)| {
            let target = kept_memory(result).target;
            view.contacts
                .iter()
                .find(|row| Some(row.id) == target)
                .map(tacsim::arena::stock_mind::row_range_cm)
        })
        .collect();
    assert!(
        !read.is_empty()
            && read
                .iter()
                .all(|range| *range <= CLOSE_CEILING_CM.unsigned_abs()),
        "the mind read its target inside the ordered band: {read:?}"
    );
    assert!(
        fired_by(&events(&mut arena, &mut bytes), RED_A) > 0,
        "a target inside the ordered band is still shot at"
    );
}

/// **17b cut M4.5a**: blue's tank is a mind (the reflex's before it). **17b
/// cut M4.6c** renamed the pilot's refusal `Refusal::NotMachine` (word
/// `not_machine`; `NotReflex`, `not_reflex`, before it); its sentence is
/// unchanged.
#[test]
fn an_order_is_refused_for_a_unit_a_pilot_is_flying() {
    let cfg = arena_cfg(30_000);
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units: vec![
                tank(RED_A, Team::Red, centre(2, 8), Controller::Pilot),
                tank(BLUE, Team::Blue, centre(13, 8), Controller::Mind),
            ],
            pilots: vec![Pilot {
                unit: RED_A,
                pilot: ObjId(1_000),
                station: SmolStr::new("cockpit"),
                epoch: 1,
            }],
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
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
        apply_intent(
            &mut arena,
            &intent,
            tacsim::arena::intents::IntentSource::Seat
        ),
        IntentOutcome::Refused(Refusal::NotMachine),
        "an order steers a machine, and a pilot's tank has none"
    );
}

// ---------------------------------------------------------------------------
// (2) An expired order drops the squad to doctrine
// ---------------------------------------------------------------------------

/// **An expired order drops the squad to doctrine, with its ledger line** —
/// 17b cut M4.5a, on the unit mind. The expiry is the ARENA's, as it was the
/// reflex row's: the order (accepted at 2,000, two seconds to live) is cleared
/// at each mind's first landing at or past 4,000 (`#6` 4,254, `#4` 4,732),
/// which journals the `order_expired` line and owes the squad its signal
/// (`Arena::note_order_expired`); the next thinks name a waypoint again (by
/// 5,732). Every window holds (3,500 under the order, 7,000 past it). One read
/// moved: doctrine's goal is the node the mind NAMED (its memory's `goal`),
/// because the motor fans a goal in the other side's entry zone off a
/// squadmate's (`Arena::fan_goal`, departure (d)).
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
    assert!(under_order(&arena, arena.now_ms) >= 2);

    run(&mut arena, 7_000, &mut bytes);
    assert_eq!(
        under_order(&arena, arena.now_ms),
        0,
        "the order has run out"
    );
    // `under_order` counts only an order whose `expires_ms` is still ahead, so
    // it cannot see whether the ARENA cleared the spent order: read the rows.
    assert!(
        order_on(&arena, RED_A).is_none() && order_on(&arena, RED_B).is_none(),
        "and the arena cleared it off both mind rows"
    );
    // Doctrine is the fallback, not idle: the machine is back on its own waypoint
    // graph and no longer making for the cell it was sent to.
    let after = named_goal(&arena, RED_A);
    assert!(
        after == Some((1, 8)) || after == Some((14, 8)),
        "back on its own graph, not still making for the ordered cell: {after:?}"
    );
    // The node is the mind's NAME for it (the motor may fan an entry-zone
    // node, `Arena::fan_goal`: measured, `#4` names (14, 8) and its motor
    // holds the fanned (14, 11)); the hull drives it, not the ordered cell.
    let driven = goal_of(&arena, RED_A);
    assert!(
        driven.is_some_and(|goal| goal != (9, 3)),
        "the hull drives doctrine's node, not the ordered cell: {driven:?}"
    );
    assert_eq!(state_of(&arena, RED_A), Some(MachineState::Patrol));
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

/// **17b cut M4.5a — on the unit mind**: `#4`'s `Withdraw` lands at 2,732 (read
/// at 3,000); the arena clears the order at its first landing past 4,000
/// (4,732) and the next think restores the order's `Withdraw` to `Patrol` (the
/// memory's `ordered`, 17b cut M4.3), landing at 5,732 — inside the 7,000.
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
    assert_eq!(state_of(&arena, RED_A), Some(MachineState::Withdraw));
    run(&mut arena, 7_000, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(MachineState::Patrol),
        "`Withdraw` is terminal for a hurt tank, not for one a commander sent home"
    );
    assert!(
        arena.health_permille(RED_A) >= ReflexConfig::default().withdraw_permille,
        "and the hull is whole, which is what makes the assertion above about the order"
    );
}

// ---------------------------------------------------------------------------
// (3) The floor
// ---------------------------------------------------------------------------

/// **17b cut M4.5a**: on a field of minds (the signals are the arena's; a
/// mind's squad asks at the opening pulse as a reflex squad did).
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

/// **17b cut M4.5a — on unit minds**: "no order arrived" counts the mind rows
/// standing under an order, and the doctrine state is the mind's
/// (`Arena::machine_state`).
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
    assert_eq!(under_order(&arena, arena.now_ms), 0, "no order arrived");
    // Doctrine is the fallback doc 16 §6 names, and doctrine is the whole machine:
    // the squad is patrolling, closing or fighting on its own doctrine, and the one
    // thing it is not is idle.
    assert!(
        matches!(
            state_of(&arena, RED_A),
            Some(
                MachineState::Patrol
                    | MachineState::Approach
                    | MachineState::Engage
                    | MachineState::Withdraw
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
// (`crate::arena::reflex`'s module head says why; 17b cut M4.5a: a mind's order is
// cleared at its own landing, `Arena::expire_mind_order`); if it were counted in pulses, or
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
    MachineState,
    Option<(u16, u16)>,
    bool,
);

/// **Every unit as a replay ends** (17b cut M4.5a): the pose, the speed, the
/// machine's state (`Arena::machine_state`, `Startup` for a hull no machine
/// flies yet), the motor row's goal and whether an order stands on the mind
/// row — where the reflex row's state and order were read.
fn finals_of(arena: &Arena) -> Vec<OrderedFinal> {
    arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                arena
                    .machine_state(unit.id)
                    .unwrap_or(MachineState::Startup),
                // 17b cut M1a.1: the goal is the hull's motor row's.
                arena.motors.row(unit.id).and_then(|motor| motor.goal),
                order_on(arena, unit.id).is_some(),
            )
        })
        .collect()
}

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
        // The order rides the opening pulse, at arena millisecond zero. 17b
        // cut M4.5a: settled, as every three-cadence replay steps.
        let out = if first {
            first = false;
            arena.step_settled(wall, &[], &burst)
        } else {
            arena.step_settled(wall, &[], &[])
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
    let finals = finals_of(&arena);
    OrderedReplay {
        events,
        hashes,
        finals,
    }
}

/// **17b cut M4.5a — on unit minds**: every replay steps `step_settled` (a
/// snapshot right after `Arena::step` can be refused while a think is
/// unanswered); the order rides the opening pulse, is obeyed and is cleared by
/// the arena at each mind's first landing past its expiry, journalling
/// `order_expired` at the same millisecond at every cadence. The finals read
/// the machine state and the mind rows' orders, and the L.1 round trip is the
/// mind row's `FrameOrder` (see [`replay_ordered_hashes_resumed_at`]).
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

    // **17b cut L.1** (D-UM-52; `SNAPSHOT_FORMAT` 6): the named proof of the
    // standing order's round trip, which the `af1` pin cannot see. At each
    // cadence a snapshot is taken under the standing order (the first pulse at
    // or past 5,000 ms; the order stands to 10,000), every carried standing
    // order decodes field for field, and the arena resumed from it steps to the
    // same hash sequence the uninterrupted run did. (17b cut M4.5a, Q3: the
    // order rides the mind row, which carries its squad and objective and no
    // author — the `by` round trip this proved on the reflex row is LOST, not
    // re-asserted.)
    for (at, cadence) in cadences.into_iter().enumerate() {
        let resumed = replay_ordered_hashes_resumed_at(cadence, 30_000, 5_000);
        let whole = runs
            .get(at)
            .map(|run| run.hashes.clone())
            .unwrap_or_default();
        assert_eq!(
            resumed, whole,
            "the resumed run's hash sequence diverged at {cadence} ms"
        );
    }
}

/// [`replay_ordered`]'s loop, hashes only, with the arena swapped mid-run for the
/// one decoded from its own snapshot at the first pulse at or past `snap_ms`
/// (17b cut L.1). Asserts, at the swap, that the document is the current
/// format and that every standing order the arena carried came back field for
/// field (on the reflex row that included its author; see below).
///
/// **17b cut M4.5a** (Q3; the M4.3 precedent, `cockpit.rs`'
/// `l1_a_typed_order_carries_the_seats_character`): the squad's machines are
/// minds, so the snapshot carries each hull's order on its mind row as the
/// `FrameOrder` the mind reads — red-1's squad word, the objective, the
/// constraints and `expires_ms` — and every one is asserted back field for
/// field. A mind's order names no author: D-UM-52's `by` is the world's watch's
/// and the tape's, and the issuer is not mind-row state at M4.5 (since 17b cut
/// M4.6, the owner's STOP C, it is: `MindState::order_issuer`, carried and
/// unfolded; the author still is not), so **the
/// author half of the round trip is LOST** (Q3), where the reflex row carried
/// the `StandingOrder` whole. What is asserted is what the mind row carries:
/// the order's squad and objective. The squad check is not an author check —
/// `FrameOrder.squad` comes from `Order.squad`, not `Order.by`, and would read
/// red-1 whoever the author were. The loop steps
/// `step_settled`: a snapshot right after `Arena::step` can be refused while a
/// think is unanswered.
fn replay_ordered_hashes_resumed_at(cadence_ms: i64, until_ms: i64, snap_ms: i64) -> Vec<u64> {
    use tacsim::arena::snapshot::{SNAPSHOT_FORMAT, Snapshot};
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
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
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut first = true;
    let mut swapped = false;
    loop {
        let _ = if first {
            first = false;
            arena.step_settled(wall, &[], &burst)
        } else {
            arena.step_settled(wall, &[], &[])
        };
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms {
            break;
        }
        if !swapped && arena.now_ms >= snap_ms {
            swapped = true;
            let snapshot = Snapshot::encode(&arena).expect("the arena encodes under the order");
            assert_eq!(
                Snapshot::head(&snapshot.document).map(|head| head.format),
                Ok(SNAPSHOT_FORMAT)
            );
            // 17b cut M3.2 moved the format to 8, 17b cut M4.6 to 9 and 17b
            // cut M5.2 to 10.
            assert_eq!(SNAPSHOT_FORMAT, 10);
            let mut resumed = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
                .expect("the document decodes");
            let units: Vec<ObjId> = arena.units.iter().map(|unit| unit.id).collect();
            let carried = units
                .iter()
                .filter(|unit| order_on(&arena, **unit).is_some())
                .count();
            assert!(carried >= 2, "the snapshot is taken under the order");
            for unit in units {
                let written = order_on(&arena, unit);
                assert_eq!(
                    order_on(&resumed, unit),
                    written,
                    "{unit:?}'s standing order came back field for field"
                );
                if let Some(standing) = written {
                    // What the mind row carries: the order's squad (from
                    // `Order.squad` — red-1, the squad the fixture's persona
                    // leads; NOT the author, whose round trip is lost, Q3)
                    // and its objective.
                    let CharacterRef::Persona { squad } = persona(red_one()) else {
                        unreachable!("the fixture's author is a persona");
                    };
                    assert_eq!(standing.squad, squad.to_string());
                    assert_eq!(
                        standing.objective,
                        tacsim::arena::perception::MindObjective::Advance { to: (9, 3) }
                    );
                }
            }
            let _ = resumed.resume_at(wall);
            resumed.clock.pin_tempo_for_tests(Tempo::Quiet);
            arena = resumed;
        }
        wall += cadence_ms;
    }
    assert!(swapped, "the run reached the snapshot");
    hashes
}

// ---------------------------------------------------------------------------
// (5) and (6): the real structured call, with a mock provider behind it
//
// 17b cut M4.5a: the three tests that open a field here (the cost table, the
// malformed reply, the disabled service) field minds, whose squads ask at the
// opening pulse as the reflex's did.
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
        let brief = SquadBrief::from_signal(1, signal, 2, persona(signal.squad));
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
    let brief = SquadBrief::from_signal(1, signal, 2, persona(signal.squad));

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
        brief: SquadBrief::from_signal(1, signal, 2, persona(signal.squad)),
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
    roster_of(world_config())
}

/// [`roster`] over `cfg` — **17b cut L.2**'s pilot-only match turns the fill
/// off.
fn roster_of(cfg: tacsim::TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
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

/// **The lapse sentence of a lance-leader's order** (17b cut L.2; D-UM-32 as
/// amended by R-C, D-UM-53; doc 16 §6 "Provider failure"). The lance-leader's
/// `hold` runs out unrenewed and the world says, once:
///
/// - to every pilot of the squad's own side — two red pilots here, so "every"
///   is more than "the first" — each once, as a `Sys` line, byte for byte
///   `Squad red-1: red lance leader 1's hold order has run out; back on
///   their minds.` (`back on doctrine.` until 17b cut M5.6, D9: one word for
///   every mind);
/// - to no pilot of the other side — the recipients are read off the
///   `OutputEvent`s' audiences, not only the text;
/// - to staff, the same sentence behind `Arena match <n>: `;
///
/// and nothing is said while the order stands, nothing a second time, and the
/// counts are what they were. No "link dropped" is said: that is a mind's own
/// failure's word (doc 17 §8).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l2_the_world_says_the_leaders_order_lapsed_once() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, pilots) = l2_running_match(&mut tw, &[Team::Red, Team::Red, Team::Blue]);
    let [(red_pilot, _), (wingmate, _), (blue_pilot, _)] = [pilots[0], pilots[1], pilots[2]];
    let (red, _) = l1_sides(&squads);
    let squad = red.id;

    // The reply the world would have got from the provider: one `hold` with a
    // one-second time to live, by the record's own persona for the squad.
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
            issuer: None,
            by: persona(squad),
            spot: None,
        },
        model: "mock".to_string(),
        prompt_hash: "hash".to_string(),
    };
    let payload = serde_json::to_string(&reply).expect("a reply encodes");
    let said = l2_said(&mut tw, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert!(
        l2_lapses(&said).is_empty(),
        "nothing is said while the order stands: {said:?}"
    );

    // Two seconds later the order has run out, and the next egress drain says so.
    let now = tw.world().now();
    tw.set_now(now + 2);
    let said = l2_said(&mut tw, tacsim::arena_bridge::drain_egress);
    let line = "Squad red-1: red lance leader 1's hold order has run out; back on their minds.";
    let lapses = l2_lapses(&said);
    // The side's pilots are told in the record's roster order, which this test
    // does not pin; each of them exactly once, then staff, and nobody else.
    assert_eq!(lapses.len(), 3, "two pilots and staff: {said:?}");
    for pilot in [red_pilot, wingmate] {
        assert_eq!(
            lapses
                .iter()
                .filter(|lapse| **lapse == (Audience::Actor(pilot), line.to_owned()))
                .count(),
            1,
            "every pilot of the squad's side, #{} among them, once: {said:?}",
            pilot.0
        );
        assert!(
            said.iter()
                .any(|(audience, event)| *audience == Audience::Actor(pilot)
                    && matches!(event, OutputEvent::Sys(_))
                    && render(event) == line),
            "a Sys line to #{}",
            pilot.0
        );
    }
    assert_eq!(
        lapses.last(),
        Some(&(
            Audience::Actor(GOD),
            format!("Arena match {match_id}: {line}")
        )),
        "and staff, after the side: {said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|(audience, _)| *audience == Audience::Actor(blue_pilot)),
        "the other side hears nothing of it: {said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|(_, event)| render(event).contains("link dropped")),
        "the retired sentence is never said: {said:?}"
    );

    // Said once, and once only.
    let again = l2_said(&mut tw, tacsim::arena_bridge::drain_egress);
    assert!(
        l2_lapses(&again).is_empty(),
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

/// **A pilot's own order lapses to that pilot alone** (17b cut L.2; the plan's
/// Q-L3: 16c Q38's "no radio line" never told the side a pilot ordered, so its
/// lapse is not told to them either — a declared narrowing). Two red pilots
/// and a blue one; the first red pilot's watch on red-1 runs out:
///
/// - the issuer reads `Squad red-1: your hold order has run out; back on
///   their minds.`;
/// - staff read it in the pilot's name, `Arena match <n>: Squad red-1:
///   Pilot0's hold order has run out; back on their minds.`;
///
/// (each `… back on doctrine.` until 17b cut M5.6, D9: one word for every
/// mind)
/// - the side's other pilot and the other side read nothing.
///
/// And a watch whose author's object is gone (blue-1's, here) is told to
/// nobody but staff, as `its leader's`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l2_a_pilots_own_order_lapse_is_told_to_them_alone() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, pilots) = l2_running_match(&mut tw, &[Team::Red, Team::Red, Team::Blue]);
    let [
        (issuer, issuer_tank),
        (wingmate, _),
        (blue_pilot, blue_tank),
    ] = [pilots[0], pilots[1], pilots[2]];
    let (red, blue) = l1_sides(&squads);
    let now_ms = tw.world().now() * 1_000;
    {
        let registry = tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the registry");
        registry.note_human_order(
            match_id,
            red.id,
            SmolStr::new("hold"),
            issuer_tank,
            CharacterRef::Character { id: issuer },
            now_ms + 1_000,
        );
        // An author with no object in the world: a character that has gone.
        registry.note_human_order(
            match_id,
            blue.id,
            SmolStr::new("withdraw"),
            blue_tank,
            CharacterRef::Character { id: ObjId(424_242) },
            now_ms + 1_000,
        );
    }
    let now = tw.world().now();
    tw.set_now(now + 2);
    let said = l2_said(&mut tw, tacsim::arena_bridge::drain_egress);
    let blue_line = format!(
        "Arena match {match_id}: Squad {}: its leader's withdraw order has run out; back on \
         their minds.",
        blue.id
    );
    assert_eq!(
        l2_lapses(&said),
        vec![
            (
                Audience::Actor(issuer),
                "Squad red-1: your hold order has run out; back on their minds.".to_owned()
            ),
            (
                Audience::Actor(GOD),
                format!(
                    "Arena match {match_id}: Squad red-1: Pilot0's hold order has run out; back \
                     on their minds."
                )
            ),
            (Audience::Actor(GOD), blue_line),
        ],
        "the issuer and staff, and nobody else: {said:?}"
    );
    for nobody in [wingmate, blue_pilot] {
        assert!(
            !said
                .iter()
                .any(|(audience, _)| *audience == Audience::Actor(nobody)),
            "#{} hears nothing of it: {said:?}",
            nobody.0
        );
    }
    let again = l2_said(&mut tw, tacsim::arena_bridge::drain_egress);
    assert!(l2_lapses(&again).is_empty(), "said once: {again:?}");
}

/// **`@arena/status` names who leads** (17b cut L.2; the plan's Q-L4). Through
/// the composition: the `orders:` line counts the lance leaders' orders (R-C:
/// a mind never orders) and the `leaders:` line counts the sim personas — `2
/// sim personas` on a two-squad match — and names a character leader by squad
/// and name; a match the fill did not field (pilots only, so no squad) reads
/// `leaders: none`; and a pilot still gets the verb's refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l2_status_prints_the_leaders() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, pilots) = l2_running_match(&mut tw, &[Team::Red, Team::Blue]);
    assert_eq!(squads.len(), 2, "a squad a side: {squads:?}");
    let staff = |tw: &mut TestWorld, actor: ObjId| {
        let events = tw.run(actor, "@arena/status");
        tw.seen_by(&events, actor)
    };
    let status = staff(&mut tw, GOD);
    assert!(
        status
            .iter()
            .any(|line| line == "  orders: 0 by pilots, 0 by lance leaders"),
        "{status:?}"
    );
    assert!(
        status
            .iter()
            .any(|line| line == "  leaders: 2 sim personas"),
        "{status:?}"
    );
    assert!(
        !status.iter().any(|line| line.contains("by the machines")),
        "{status:?}"
    );

    // A character leading red-1 is named by squad and name.
    let (red, _) = l1_sides(&squads);
    if let Some(record) = tw
        .world_mut()
        .module_state_mut::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        for leader in &mut record.leaders {
            if leader.squad == red.id {
                leader.by = CharacterRef::Character { id: pilots[0].0 };
            }
        }
    }
    let status = staff(&mut tw, GOD);
    assert!(
        status
            .iter()
            .any(|line| line == "  leaders: 1 sim persona, red-1 Pilot0"),
        "{status:?}"
    );

    // A pilot reads the refusal and nothing of the match.
    let refused = staff(&mut tw, pilots[1].0);
    assert!(
        refused
            .iter()
            .any(|line| line.contains("Permission denied.")),
        "{refused:?}"
    );
    assert!(
        !refused
            .iter()
            .any(|line| line.contains("leaders:") || line.contains("orders:")),
        "{refused:?}"
    );

    // Pilots only: no fill, no squad, no leader.
    let mut cfg = world_config();
    cfg.match_.ai_fill = false;
    let mut alone = TestWorld::with_modules(roster_of(cfg));
    let (_, squads, _) = l2_running_match(&mut alone, &[Team::Red, Team::Blue]);
    assert!(
        squads.is_empty(),
        "a pilot-only match has no squad: {squads:?}"
    );
    let status = staff(&mut alone, GOD);
    assert!(
        status.iter().any(|line| line == "  leaders: none"),
        "{status:?}"
    );
    assert!(
        status
            .iter()
            .any(|line| line == "  orders: 0 by pilots, 0 by lance leaders"),
        "{status:?}"
    );
}

/// **17b cut L.2**: start one match on the production composition root with
/// one pilot per entry of `sides` (named `Pilot0`, `Pilot1`, … in that order)
/// and, with the fill on, machines to three a side (`SIM AI 3`), every pilot
/// boarded, and answer its id, its squads and
/// each pilot with their tank — [`y1_running_match`] for any number of pilots
/// a side.
fn l2_running_match(
    tw: &mut TestWorld,
    sides: &[Team],
) -> (
    tacsim::arena_task::MatchId,
    Vec<tacsim::deliberation::SquadRoster>,
    Vec<(ObjId, ObjId)>,
) {
    let lobby = tacsim::match_::centre_object(tw.world(), tacsim::match_::LOBBY_ENTRY)
        .expect("the OOC pack carries the simulator lobby");
    let mut map = Accounts::default();
    let mut pilots = Vec::new();
    for (n, _) in sides.iter().enumerate() {
        let pilot = tw.spawn_player(&format!("Pilot{n}"));
        tw.world_mut()
            .move_to(pilot, lobby)
            .expect("a player walks into the lobby");
        let n = i64::try_from(n).expect("a handful of pilots");
        map.0.insert(
            pilot,
            AccountId::mint(&format!("account-{n}"), n).expect("an account id"),
        );
        pilots.push(pilot);
    }
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
    // The control-lease mirror is refreshed by `mush-sim`'s own coarse tick,
    // and a seat activation fails closed on an unread one: one second of world
    // time first, as `tests/cockpit.rs`' rig does.
    let now = tw.world().now().max(1);
    tw.set_now(now);
    let _ = tw.fire_due(now.saturating_mul(1_000));
    for (pilot, team) in pilots.iter().zip(sides) {
        tw.run(*pilot, &format!("sim team {}", team.as_str()));
    }
    // Three a side, so a side of two pilots still fields a machine; refused,
    // and harmless, on a server with the fill off.
    tw.run(pilots[0], "sim ai 3");
    tw.run(pilots[0], "sim start");
    let match_id = tacsim::match_::centre_of(tw.world())
        .expect("the centre")
        .match_id
        .expect("a boarding centre has a match id");
    // Each pilot walks to their side's room and boards, as a player does
    // (`tests/cockpit.rs`' `fly`): the match is checked out at the last
    // `BOARD`, with every claimed tank seated, so the record's rosters carry
    // the pilots and their tanks are the pilots' and in no squad.
    let mut seated = Vec::new();
    for (pilot, team) in pilots.iter().zip(sides) {
        let entry = match team {
            Team::Red => tacsim::match_::RED_ROOM_ENTRY,
            Team::Blue => tacsim::match_::BLUE_ROOM_ENTRY,
        };
        let room = tacsim::match_::centre_object(tw.world(), entry).expect("a centre room");
        tw.world_mut()
            .move_to(*pilot, room)
            .expect("the centre's rooms take anybody");
        tw.run(*pilot, "board");
        let pod = tacsim::match_::pod_of(tw.world(), *pilot).expect("a claimed slot locks a pod");
        let unit =
            tacsim::match_::tank_of(tw.world(), pod).expect("the pod's tank is in the world");
        seated.push((*pilot, unit));
    }
    assert_eq!(
        tacsim::match_::centre_of(tw.world())
            .expect("the centre")
            .state,
        tacsim::match_::MatchState::Running,
        "every pilot boarded"
    );
    let squads = tw
        .world()
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.squads.clone())
        .expect("the record carries its squads");
    (match_id, squads, seated)
}

/// Run one closure with a `Ctx` over the test world as staff and return every
/// event it produced with its audience (17b cut L.2: the lapse's recipients
/// are part of the claim).
fn l2_said(tw: &mut TestWorld, body: impl FnOnce(&mut Ctx<'_>)) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    {
        let engine = tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        body(&mut ctx);
    }
    out.events().to_vec()
}

/// The lapse sentences among `said`, each with its audience, in the order said.
fn l2_lapses(said: &[(Audience, OutputEvent)]) -> Vec<(Audience, String)> {
    said.iter()
        .map(|(audience, event)| (audience.clone(), render(event)))
        .filter(|(_, line)| line.contains("order has run out"))
        .collect()
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

/// **An ordered `close` caps a mind's squad at the pilot's own `close`, and
/// keeps its hysteresis** — 17b cut M4.5a, the first leg of
/// [`e1_an_ordered_close_caps_the_squad_at_the_pilots_own_close`] on the unit
/// mind (its two parked-machine legs stayed on the reflex there, STOP B, until
/// 17b cut M4.6b ported them under the owner's ruling).
///
/// Both claims of the leg are kept as the reflex's — sampled every pulse, the
/// first pulse `#4` spends in `Engage` reads its target's TRUE plot inside
/// seven tenths of the ordered ceiling (8,166 cm), the widest inside the
/// ceiling (11,666 cm), and it shoots — and, under ruling 1 (the claim is
/// about two edges), each is asserted again on the figure the mind READS: the
/// view rounds a contact's range to 50 m and holds a published figure within
/// 3,125 cm, so a mind compares 50 m figures against the two edges. The think
/// that first answers `Engage` read its target at or inside 8,166 cm (the
/// 50 m figure: a mind that engaged at the ceiling instead would have read
/// 100 m), every think that answers `Engage` read it at or inside 11,666 cm
/// (no 150 m figure), and the first think after the order read it past the
/// ceiling (the premise: 13,891 cm on the diagonal publishes 150 m), so both
/// edges were crossed on the figures the mind read. The shot needs the
/// presenting profile (ruling 1). Stepped settled, sampled at the same forty
/// pulses.
#[test]
fn e1_an_ordered_close_caps_a_minds_squad_at_the_pilots_own_close() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
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
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if state_of(&arena, RED_A) == Some(MachineState::Engage) {
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

    // Ruling 1: the same two edges on the figures the mind read — its target's
    // row, rounded and held, at every think after the order.
    let read: Vec<(MachineState, i64)> = stock
        .kept()
        .iter()
        .filter(|(unit, t_ms, _, _)| *unit == RED_A && *t_ms > at)
        .filter_map(|(_, _, view, result)| {
            let answer = kept_memory(result);
            let row = view
                .contacts
                .iter()
                .find(|row| Some(row.id) == answer.target)?;
            let range = tacsim::arena::stock_mind::row_range_cm(row);
            Some((answer.state, i64::try_from(range).unwrap_or(i64::MAX)))
        })
        .collect();
    assert!(
        read.first()
            .is_some_and(|(_, range)| *range > CLOSE_CEILING_CM),
        "the premise: the first think read its target past the ordered ceiling: {read:?}"
    );
    let engaged: Vec<i64> = read
        .iter()
        .filter(|(state, _)| *state == MachineState::Engage)
        .map(|(_, range)| *range)
        .collect();
    assert!(
        engaged
            .first()
            .is_some_and(|range| *range <= CLOSE_ENGAGE_CM),
        "the mind entered `Engage` reading past seven tenths of the ceiling: {read:?}"
    );
    assert!(
        engaged.iter().all(|range| *range <= CLOSE_CEILING_CM),
        "the mind held `Engage` reading past the ordered ceiling: {read:?}"
    );
    let fired = fired_by(&events(&mut arena, &mut bytes), RED_A);
    assert!(fired > 0, "and inside the ordered band it shoots");
}

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
/// `release_cm > engage_cm` for all three bands was pinned arithmetically by
/// the reflex's own e1 test (`arena/reflex.rs`'
/// `e1_the_stock_tanks_three_rows_of_the_range_table`) until 17b cut M4.6c
/// deleted it, and is pinned row for row off a mind's view by
/// `arena/stock_mind.rs`'
/// `e1_the_stock_tanks_three_rows_of_the_range_table_off_the_view`.
///
/// **17b cut M4.6b — on the unit mind** (the owner's STOP B ruling of
/// 2026-10-09: the mind's view reads `immobile` for a hull with no authored
/// speed once its power is `Online`). The first leg (the squad that closes)
/// was ported in 17b cut M4.5a as
/// [`e1_an_ordered_close_caps_a_minds_squad_at_the_pilots_own_close`]; this
/// test keeps the two legs that pin `#4` with `mobility.max_speed_cms = 0`
/// — authored zero speed, which until this cut a mind's view did not read as
/// `immobile` (it was `limits.motion_disabled` alone), so the AA.1 witness
/// these legs carry (a machine that cannot close enters `Engage` at
/// `release_cm`, and the band cap still holds it) was unreachable on a mind
/// and the legs stayed on the reflex as a STOP (M4.5a's Q18), fielded by
/// the reflex's own `reflex_field`. Both legs now field [`field`]'s three
/// minds — the presenting profile the first leg's shot needs (a mind's armed
/// slot fires at odds) — and every claim is kept: once online, `#4`'s view
/// reads it `immobile`, so its edge into `Engage` is `release_cm`; under the
/// `close` order that edge is the ordered ceiling, and the target parked past
/// it (150 m on the view) is never inside it, so `#4` never locks and never
/// fires; with no band, the same parked mind's edge is doctrine's own reach
/// and it fires from where it stands. As on the reflex, the claims alone do
/// not tell the two edges apart — without the immobile reading the parked
/// mind's edge would be `engage_cm` (8,166 cm), short of the target too, and
/// the legs pass either way (17b cut M4.6b's check with the view's STOP B
/// reading switched off); they are kept for the witness they carry with it.
/// The shared [`run`] steps `step_settled`.
#[test]
fn e1_an_ordered_close_caps_the_squad_at_the_pilots_own_close() {
    let cfg = arena_cfg(30_000);
    let mut told = order(Objective::Engage { target_hint: None }, 60_000);
    told.constraints.max_range_band = Some(RangeBand::Close);

    // And the same field with the same order and a target parked **past** the
    // ceiling, with nowhere to close from, never fires at all: the squad holds
    // its fire rather than shooting at the distance doctrine alone would allow.
    let mut held = field(&cfg);
    let mut held_bytes = Vec::new();
    // **16b cut M.2** (16c `T-02`): pinned BEFORE the two opening seconds, not
    // after them. The machine's own `STARTUP` resolves at its tick now, so it is
    // online a quarter of a second sooner and is already rolling at 6 m/s when
    // the two seconds are up — pinned then, it coasted to 11,493 cm, inside the
    // ceiling, and shot. Pinned from the start it never leaves its cell, which is
    // the parked machine this case was written about.
    if let Some(state) = held.unit_mut(RED_A) {
        state.mobility.max_speed_cms = 0;
    }
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
    // 16b cut M.2: pinned from the start too, so the control stands where the
    // held case stands.
    if let Some(state) = free.unit_mut(RED_A) {
        state.mobility.max_speed_cms = 0;
    }
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
/// `B-41`'s claim is about how many searches a failed goal costs. 17b cut
/// M4.5a: settled, as [`run`].
fn run_counted(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> (
    Vec<tacsim::arena::command::SquadSignal>,
    tacsim::arena::motor::MotorCounters,
) {
    let mut wall = arena.now_ms;
    let mut signals = Vec::new();
    let mut counters = tacsim::arena::motor::MotorCounters::default();
    while arena.now_ms < until_ms {
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        counters.absorb(&out.motor);
        signals.extend(out.deliberation);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    (signals, counters)
}

/// [`run_counted`]'s counters over every event at or before `last_ms` and no
/// later one (17b cut M4.5a): the same 500 ms pulses, the last one cut to end
/// at `last_ms` exactly, so a window can be split at a landing that falls off
/// the 500 ms grid. Settled, as [`run`] is.
fn run_counted_through(
    arena: &mut Arena,
    last_ms: i64,
    bytes: &mut Vec<u8>,
) -> tacsim::arena::motor::MotorCounters {
    let mut wall = arena.now_ms;
    let mut counters = tacsim::arena::motor::MotorCounters::default();
    while arena.now_ms < last_ms {
        wall = wall.saturating_add(500).min(last_ms);
        let out = arena.step_settled(wall, &[], &[]);
        absorb(&out, bytes);
        counters.absorb(&out.motor);
        if out.ended.is_some() {
            break;
        }
    }
    assert_eq!(arena.now_ms, last_ms, "the window ends where it was cut");
    counters
}

/// **Take blue's tank off the controller** so it stands where the fixture put
/// it.
///
/// Every one of 16b cut E.2's deliberation tests is about what RED's squad does
/// over tens of seconds of arena time, and a third machine walking the
/// two-node patrol graph crosses the whole window in that time — it drives onto
/// the red pair, is held, and resets the very clocks and edges the tests are
/// about. A seat with nobody in it is the cheapest way to have a field that
/// stays where it was built.
fn park(arena: &mut Arena, unit: ObjId) {
    arena.set_controller(unit, Controller::Pilot, MachineState::Withdraw);
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

/// One ending think, as [`ending_think`] reads it: its lattice millisecond,
/// whether its view carried the motor's `drive_no_route`, and the goal its
/// memory held going in.
type Ending = (i64, bool, Option<(u16, u16)>);

/// **The think that ended `unit`'s order** (17b cut M4.5a; Q15, D7), off a
/// stock recorder's kept thinks: the first of `unit`'s answers that writes
/// the order's `expires_ms` to `ignore_until_ms` — its lattice millisecond,
/// whether its view carried the motor's `drive_no_route`, and the goal its
/// memory held going in (the answer before it).
fn ending_think(kept: &[support::scripted::Kept], unit: ObjId, expires_ms: i64) -> Option<Ending> {
    let mut going_in: Option<(u16, u16)> = None;
    for (who, t_ms, view, result) in kept {
        if *who != unit {
            continue;
        }
        let answer = kept_memory(result);
        if answer.ignore_until_ms == Some(expires_ms) {
            let no_route = view.notices.iter().any(|notice| {
                matches!(
                    notice,
                    tacsim::arena::perception::FrameNotice::Drive {
                        outcome: tacsim::arena::perception::DriveOutcome::NoRoute,
                    }
                )
            });
            return Some((*t_ms, no_route, going_in));
        }
        going_in = answer.goal;
    }
    None
}

/// **Every `NoRoute` `unit`'s mind read** (17b cut M4.5a), off a stock
/// recorder's kept thinks: the lattice millisecond of each think whose view
/// carried the motor's `drive_no_route`, and the goal its memory held going in
/// (the goal the mind had handed the motor — the one that failed).
fn no_routes_read(kept: &[support::scripted::Kept], unit: ObjId) -> Vec<(i64, Option<(u16, u16)>)> {
    let mut going_in: Option<(u16, u16)> = None;
    let mut read = Vec::new();
    for (who, t_ms, view, result) in kept {
        if *who != unit {
            continue;
        }
        if view.notices.iter().any(|notice| {
            matches!(
                notice,
                tacsim::arena::perception::FrameNotice::Drive {
                    outcome: tacsim::arena::perception::DriveOutcome::NoRoute,
                }
            )
        }) {
            read.push((*t_ms, going_in));
        }
        going_in = kept_memory(result).goal;
    }
    read
}

/// A kept think's answer, as the stock memory it wrote (17b cut M4.5a).
fn kept_memory(
    result: &tacsim::arena::mind::ThinkResult,
) -> tacsim::arena::stock_mind::StockMemory {
    tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
        &result.memory,
    ))
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

/// **`B-41`: an ordered cell no route reaches ends the order for the unit** —
/// 17b cut M4.5a, on the unit mind, under the M4.4 row-54 `order_end` ruling
/// extended to these ends (Q15, D7).
///
/// A mind cannot end an arena order, only stop obeying it: the think that
/// reads the motor's `drive_no_route` for the ordered cell writes the order's
/// `expires_ms` to its memory's `ignore_until_ms` and names the fallback
/// waypoint (the `fallback` departure: a think late). So every claim is kept
/// with three re-pins:
///
/// * **the end** — no `order_expired` line and `orders_expired` 0 (the
///   arena's expiry is ten minutes off), the order still standing on each mind
///   row, and each mind's `ignore_until_ms` its `expires_ms`: no mind OBEYS it;
/// * **when, and in which order** — the ending think, read off the stock
///   recorder (the first answer that writes `ignore_until_ms`, its view
///   carrying the `NoRoute` and its memory's goal the pocket): one per unit,
///   in the order the two minds' thinks fall on their stagger (`#6` at 3,004,
///   `#4` at 3,482 — the reflex's `stagger_ms(.., 500)` read is now the think
///   lattice's), each landing (3,254, 3,732) inside `at + 2,000`;
/// * **doctrine's goal** — the node the mind names (its memory's `goal`; the
///   motor may fan a node in the other side's zone, departure (d)).
///
/// **Lost, not re-pinned (Q15)**: the reflex's `B-41` end told the commander at
/// once (`SignalReason::OrderExpired` on the next signal); a mind's end raises
/// nothing until the arena's own expiry at the order's TTL — a production
/// consequence since 17b cut M4.3 (a squad leader is not told an unreachable
/// order ended for up to `MAX_TTL_MS`). The test was
/// `e2_b41_an_unreachable_ordered_cell_ends_the_order_and_the_commander_hears`.
#[test]
fn e2_b41_an_unreachable_ordered_cell_ends_the_order() {
    let cfg = arena_cfg(0);
    let mut arena = field_placed(
        &cfg,
        NARROW_SIGHT_CELLS,
        &sealed_pocket(),
        [(2, 8), (2, 9), (13, 13)],
    );
    park(&mut arena, BLUE);
    // The recorder over the native stock mind every red hull thinks on.
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);

    // Ten minutes of authority — `MAX_TTL_MS`, the worst case the row names.
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );
    let at = arena.now_ms;
    let expires = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the order stands");
    let (_, counters) = run_counted(&mut arena, at + 20_000, &mut bytes);

    // One failed search per unit, over the whole of what would have been the
    // order's life. At HEAD this was one per unit per `repath_ms` for ten
    // minutes, and the goal flapped between the cell and the fallback every tick.
    assert_eq!(
        counters.path_failures, 2,
        "the squad is a pair and each of them pays exactly one: {counters:?}"
    );
    // `order_end`: the arena's own expiry is ten minutes off.
    assert_eq!(counters.orders_expired, 0);
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));

    // One ending think per unit, in the order the two minds' thinks fall
    // (16b cut U.1's stagger, read on the think lattice: `#6` at 4, `#4` at
    // 482), each reading the motor's `NoRoute` for the pocket it was driving
    // at, and each landing inside two seconds of the order — the first think
    // past the failed search, not the order's nominal expiry.
    let kept = stock.kept();
    let mut ticking = [RED_A, RED_B];
    ticking.sort_by_key(|unit| think_at(*unit, 0));
    assert_ne!(
        think_at(RED_A, 0),
        think_at(RED_B, 0),
        "the precondition: two thinks, not one"
    );
    let ends: Vec<(ObjId, i64)> = ticking
        .iter()
        .map(|unit| {
            let (t_ms, read_no_route, going_in) = ending_think(&kept, *unit, expires)
                .unwrap_or_else(|| panic!("#{} never ended the order", unit.0));
            assert!(
                read_no_route,
                "#{} ended it without a failed search",
                unit.0
            );
            assert_eq!(
                going_in,
                Some(POCKET),
                "#{} was driving at the pocket",
                unit.0
            );
            (*unit, t_ms)
        })
        .collect();
    assert!(
        ends.windows(2).all(|pair| pair[0].1 < pair[1].1),
        "the ends fall in the thinks' order: {ends:?}"
    );
    for (unit, t_ms) in &ends {
        let dropped_at = t_ms.saturating_add(arena.substep_len_ms());
        assert!(
            dropped_at < at + 2_000,
            "#{}'s end waited for something: {dropped_at} ms against an order taken at {at} ms",
            unit.0
        );
    }

    // The order stands on both rows and neither mind obeys it, and doctrine's
    // own goal stands: a waypoint, and the machine is driving to it.
    for unit in [RED_A, RED_B] {
        assert_eq!(
            order_on(&arena, unit).map(|order| order.expires_ms),
            Some(expires),
            "#{}: the arena still holds the order (`order_end`)",
            unit.0
        );
        assert_eq!(
            memory_of(&arena, unit).ignore_until_ms,
            Some(expires),
            "#{} still obeys it",
            unit.0
        );
        assert!(!obeys(&arena, unit));
        let goal = named_goal(&arena, unit).expect("doctrine has somewhere to be");
        assert!(
            nodes(&arena).contains(&goal),
            "#{} is still steering at the cell it cannot reach: {goal:?}",
            unit.0
        );
        // ... and the machine is driving to it: the node is the mind's NAME
        // for it (the motor may fan the east node, `Arena::fan_goal`;
        // measured, both name and drive the west node (1, 8)), and a failed
        // goal is dropped off the motor row, so a hull that drives nothing —
        // or still the pocket — is caught here.
        let driven = goal_of(&arena, unit);
        assert!(
            driven.is_some_and(|goal| goal != POCKET),
            "#{} drives doctrine's node: {driven:?}",
            unit.0
        );
    }
    assert_eq!(
        under_order(&arena, arena.now_ms),
        2,
        "the arena still holds both orders; no mind obeys one"
    );

    // Re-sent, the same cell costs exactly one more search per unit: a newer
    // order (another deadline) is obeyed at once and ended again — the
    // deliberation floor is what bounds how often a commander may do that, and
    // here it is nought, so this is the worst case.
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );
    let from = arena.now_ms;
    let again_expires = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the newer order stands");
    assert_ne!(again_expires, expires, "a newer order, another deadline");
    let (_, again) = run_counted(&mut arena, from + 10_000, &mut bytes);
    assert_eq!(again.path_failures, 2, "{again:?}");
    assert_eq!(again.orders_expired, 0);
    let kept = stock.kept();
    for unit in [RED_A, RED_B] {
        let (_, read_no_route, going_in) = ending_think(&kept, unit, again_expires)
            .unwrap_or_else(|| panic!("#{} never ended the newer order", unit.0));
        assert!(read_no_route && going_in == Some(POCKET));
        assert_eq!(memory_of(&arena, unit).ignore_until_ms, Some(again_expires));
    }
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));
}

/// **17b cut M4.5a — on the unit mind** (Q15, D7). The sealed cell's end is a
/// mind's: `#4`'s think that reads the motor's `NoRoute` (3,482, landing
/// 3,732) writes the order's `expires_ms` to `ignore_until_ms` — read here at
/// `at + 5,000`, a think and more past it, as the order standing on the row
/// and not obeyed. The second order carries another deadline, so the mind
/// obeys it at once (`m1b5_a_newer_order_is_obeyed_after_an_early_end`,
/// `tests/minds.rs`): it stands, is obeyed (`ignore_until_ms` spent), costs no
/// failed search and is driven to arrival.
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
    let sealed = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the order stands");
    run_counted(&mut arena, at + 5_000, &mut bytes);
    assert_eq!(
        memory_of(&arena, RED_A).ignore_until_ms,
        Some(sealed),
        "the sealed cell dropped"
    );
    assert!(!obeys(&arena, RED_A));

    // A second order, naming a cell a route reaches.
    issue(
        &mut arena,
        &order(Objective::Advance { to: (6, 3) }, 600_000),
    );
    let from = arena.now_ms;
    let (_, counters) = run_counted(&mut arena, from + 30_000, &mut bytes);
    assert_eq!(counters.path_failures, 0, "{counters:?}");
    assert!(
        order_on(&arena, RED_A).is_some_and(|order| order.expires_ms != sealed),
        "the second order stands"
    );
    assert_eq!(memory_of(&arena, RED_A).ignore_until_ms, None);
    assert!(obeys(&arena, RED_A), "and is obeyed");
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

/// **17b cut M4.5a — on the unit mind** (Q15, D7). The reflex walked past the
/// contact, failed the ordered cell's search and ended the order inside one
/// tick, so it read all three at `at + 500`. A mind does them a think apart,
/// so the reads are re-derived against `#4`'s landings: the order-live think
/// (2,482) drops the planted target and lands `Patrol` (2,732) — read at
/// `at + 1,000`, before the next landing, and proved to discriminate off the
/// stock recorder: that think's view HELD the planted contact, classified
/// blue (the reflex's tick fell at most 500 ms after the plant; a mind's
/// view is 482 ms later, so it is asserted, not assumed); the motor's `NoRoute` for the pocket
/// is read by the next think (3,482), which ends the order (`ignore_until_ms`
/// its `expires_ms`, the order still standing on the row: `order_end`) —
/// read at `at + 1,750`, past that landing (3,732). On doctrine the
/// re-planted contact is a target within one think plus a landing (`+1,000`
/// grown to `+1,250`): `Approach`, which a mind still obeying the `advance`
/// could not reach.
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
    // The recorder, so the walk-past is proved to have SEEN the contact.
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    issue(
        &mut arena,
        &order(Objective::Advance { to: POCKET }, 600_000),
    );

    // Under the live order the contact is walked past: the objective drops the
    // target on every think.
    let at = arena.now_ms;
    let expires = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the order stands");
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, at);
    run_counted(&mut arena, at + 1_000, &mut bytes);
    assert!(
        arena.now_ms >= lands_at(&arena, RED_A, 2) && arena.now_ms < lands_at(&arena, RED_A, 3),
        "read between the order-live landing and the next"
    );
    assert_eq!(
        state_of(&arena, RED_A),
        Some(MachineState::Patrol),
        "a movement order does not pursue"
    );
    assert_eq!(memory_of(&arena, RED_A).target, None);
    // ... and it discriminates: the order-live think HELD the planted hostile
    // in its view (classified blue) and still answered no target and `Patrol`.
    let planted = draw_contact_id(SEED, RED_A, BLUE);
    let live = stock
        .kept()
        .into_iter()
        .find(|(unit, t_ms, _, _)| *unit == RED_A && *t_ms == think_at(RED_A, 2))
        .expect("#4's order-live think was kept");
    assert!(
        live.2
            .contacts
            .iter()
            .any(|row| row.id == planted
                && row.class == Classification::Classified { team: Team::Blue }),
        "the order-live think saw the planted hostile"
    );
    let answer = kept_memory(&live.3);
    assert_eq!(
        (answer.state, answer.target),
        (MachineState::Patrol, None),
        "and walked past it"
    );
    run_counted(&mut arena, at + 1_750, &mut bytes);
    assert_eq!(
        memory_of(&arena, RED_A).ignore_until_ms,
        Some(expires),
        "and the order is over"
    );
    assert!(!obeys(&arena, RED_A));

    // On doctrine, the same contact is a target.
    let from = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, from);
    run_counted(&mut arena, from + 1_250, &mut bytes);
    assert_eq!(
        state_of(&arena, RED_A),
        Some(MachineState::Approach),
        "the machine is on doctrine and fighting what it meets"
    );
}

/// **17b cut M4.5a — on the unit mind.** On a mind the arena holds the order
/// whatever the mind does with it (`order_end`), so "the order stands" is
/// asserted as the mind still OBEYING it: the order on the row and its
/// `ignore_until_ms` never written (the mind's one way of ending an order),
/// beside no `order_expired` line. The stand-off cell and the run home fail
/// their searches as before (`path_failures > 0`); only an `Advance`'s own
/// cell ends an order.
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
    let mut counters = tacsim::arena::motor::MotorCounters::default();
    let mut ended = None;
    while arena.now_ms < at + 6_000 {
        let now = arena.now_ms;
        plant_contact(&mut arena, RED_A, BLUE, Team::Blue, now);
        // 17b cut M4.5a: settled, so the mind rows read below (`order_on`,
        // `ignore_until_ms`, `obeys`) hold the window's last think too.
        let out = arena.step_settled(wall_ms, &[], &[]);
        absorb(&out, &mut bytes);
        counters.absorb(&out.motor);
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
        order_on(&arena, RED_A).is_some(),
        "the arena holds the `engage` order"
    );
    assert_eq!(
        memory_of(&arena, RED_A).ignore_until_ms,
        None,
        "a stand-off goal that failed its search ended an `engage` order"
    );
    assert!(obeys(&arena, RED_A));
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
        Some(MachineState::Withdraw),
        "the order still puts it in `Withdraw`"
    );
    assert!(
        counters.path_failures > 0,
        "the entry zone was reachable, so nothing was proved: {counters:?}"
    );
    assert!(
        order_on(&arena, RED_A).is_some(),
        "the arena holds the `withdraw` order"
    );
    assert_eq!(
        memory_of(&arena, RED_A).ignore_until_ms,
        None,
        "an unreachable entry cell ended a `withdraw` order"
    );
    assert!(obeys(&arena, RED_A));
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));
}

/// **17b cut M4.5a — on the unit mind** (Q15, D7). Every claim is kept; what
/// moved is the mind's latency and its end.
///
/// * **The searches.** The reflex replaced its patrol goal with the ordered
///   cell inside the tick that read the order, so the only searches the
///   remembered stop failed in the window were the ordered cell's — two. A
///   mind's order lands a substep after its think (`#6` 2,254, `#4` 2,732), and
///   until then the motor follows the patrol node it was walking, whose route
///   through the gap the same remembered stop has just sealed: that search
///   fails first. So the window counts **four**, and the stock recorder
///   attributes every one: each mind's first think after the order reads a
///   `NoRoute` for its patrol node, and its next reads one for the ordered
///   cell — exactly one per unit, as the row says — and ends the order. The
///   window is cut to `at + 1,000` (both ordered-cell searches in, `#6`'s
///   fallback landing at 3,254 out): past it the far-side fallback waypoints
///   are unreachable too, doctrine's own business, as before. **The pinned
///   count moved 2 -> 4 under the orchestrator's recorded ruling** (2 stale
///   patrol + 2 ordered cell), and the split is pinned directly rather than
///   leaning on the motor dropping a failed goal: the window is counted in two
///   halves cut at `#4`'s order landing (2,732) — `[at, 2,732)` holds both
///   stale-node failures and `#6`'s ordered cell (**3**), `[2,732, at +
///   1,000]` holds `#4`'s ordered cell alone (**1**) — so `#4`'s ordered-cell
///   failure is proved inside the counted window.
/// * **The end** (`order_end`): the ending thinks (`#6` 3,004, `#4` 3,482) write
///   the order's `expires_ms` to `ignore_until_ms` — read at `at + 1,750`,
///   past `#4`'s landing (3,732) — with no `order_expired` line and the order
///   still standing on the rows. The commander is not told until the TTL
///   (Q15's production consequence).
#[test]
fn e2_b41_a_cell_sealed_only_by_a_remembered_stop_ends_the_order_too() {
    // **The recorded cost of the rule.** The search that fails runs with the
    // unit's own `avoid` list (16b cut Q.7), whose entries age out after
    // `REFLEX_AVOID_TTL_MS` — so a cell sealed for thirty seconds by a remembered
    // stop ends the order where a retry half a minute later would have kept it.
    // The commander is told within one signal either way, which is the better
    // half of that trade; a `SignalReason` that says WHY is new prompt vocabulary
    // and is not built. (17b cut M4.5a: on a mind, told at the order's TTL.)
    let cfg = arena_cfg(0);
    // A band down column 8 with one gap, at row 8: every route east goes through
    // (8, 8).
    let band: Vec<(u16, u16, TerrainCell)> = (0..SIDE)
        .filter(|y| *y != 8)
        .map(|y| (8_u16, y, wall()))
        .collect();
    let mut arena = field_placed(&cfg, NARROW_SIGHT_CELLS, &band, [(2, 8), (2, 9), (13, 13)]);
    park(&mut arena, BLUE);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut bytes = Vec::new();
    run_counted(&mut arena, 2_000, &mut bytes);
    // Each mind walks its graph, east past the gap (the precondition that makes
    // its stale route the remembered stop's to seal).
    let walking: Vec<(u16, u16)> = [RED_A, RED_B]
        .iter()
        .map(|unit| named_goal(&arena, *unit).expect("a patrol node"))
        .collect();
    assert!(walking.iter().all(|node| node.0 > 8), "{walking:?}");

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
    let expires = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the order stands");
    // A short window on purpose: past it the fallback waypoints on the far side
    // of the band are unreachable too, which is doctrine's own business and not
    // this row's.
    // Split at `#4`'s order landing: everything before it, then the landing
    // itself to the window's end.
    let split = lands_at(&arena, RED_A, 2);
    assert!(
        at < split && split < at + 1_000,
        "the split falls inside the window"
    );
    let before = run_counted_through(&mut arena, split - 1, &mut bytes);
    let after = run_counted_through(&mut arena, at + 1_000, &mut bytes);
    assert_eq!(
        before.path_failures, 3,
        "before #4's order lands: both stale patrol nodes and #6's ordered cell: {before:?}"
    );
    assert_eq!(
        after.path_failures, 1,
        "#4's ordered cell fails at its landing, inside the window: {after:?}"
    );
    assert_eq!(
        before.path_failures + after.path_failures,
        4,
        "four over the window: two stale patrol nodes and two ordered cells"
    );
    run_counted(&mut arena, at + 1_750, &mut bytes);
    let kept = stock.kept();
    for (unit, node) in [RED_A, RED_B].into_iter().zip(walking) {
        // The goals going in of the two thinks that read the window's four
        // failures: the stale patrol node, then the ordered cell.
        let read: Vec<Option<(u16, u16)>> = no_routes_read(&kept, unit)
            .into_iter()
            .filter(|(t_ms, _)| *t_ms > at && *t_ms <= think_at(unit, 3))
            .map(|(_, going_in)| going_in)
            .collect();
        assert_eq!(
            read,
            vec![Some(node), Some((13, 3))],
            "#{}: one failed search for its stale patrol node, one for the ordered cell",
            unit.0
        );
        let (ended, _, going_in) =
            ending_think(&kept, unit, expires).expect("a transiently sealed route kept the order");
        assert_eq!(
            (ended, going_in),
            (think_at(unit, 3), Some((13, 3))),
            "#{}: the ordered cell's failure ended it",
            unit.0
        );
        assert_eq!(memory_of(&arena, unit).ignore_until_ms, Some(expires));
        assert!(!obeys(&arena, unit));
        assert_eq!(
            order_on(&arena, unit).map(|order| order.expires_ms),
            Some(expires),
            "`order_end`: the arena still holds it"
        );
    }
    let log = events(&mut arena, &mut bytes);
    assert!(expiries(&log).is_empty(), "{:?}", expiries(&log));
}

// ---------------------------------------------------------------------------
// 16b cut E.2 — a standing withdraw order is not rallied out of (16c `B-40`)
// ---------------------------------------------------------------------------

/// **17b cut M4.5a — on the unit mind**: the quiet clock and the comeback are
/// the stock memory's `quiet_since_ms` and `rallied` (the Withdraw arm's
/// `ordered_home` keeps the clock stopped), and the order still stands on the
/// mind row and is obeyed (its `expires_ms` is not the memory's
/// `ignore_until_ms`). No window moved: three rally periods are thirty-odd
/// thinks.
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
        Some(MachineState::Withdraw),
        "three rally periods of quiet and it left the state its commander put it in"
    );
    let row = memory_of(&arena, RED_A);
    assert_eq!(
        row.quiet_since_ms, None,
        "the quiet clock ran under a standing order"
    );
    assert!(!row.rallied, "the order spent the tank's one comeback");
    assert!(obeys(&arena, RED_A), "the order stands, and is obeyed");
}

/// **17b cut M4.5a — on the unit mind.** The arena clears the order at each
/// mind's first landing at or past its expiry (`at + 10,000`: `#4`'s at
/// `at + 10,732`), and the restoration (a healthy `Withdraw` an order put the
/// hull in, under no order, back to `Patrol`) is the NEXT think's, which
/// lands one think later (`at + 11,732`) — so the reflex's `at + 11,000` (its
/// tick after the expiry) grows by one think plus a landing to
/// `at + 12,250`, for both hulls. The hurt hull's quiet clock (the memory's
/// `quiet_since_ms`) starts at that think, after the expiry, and its
/// `rallied` is the memory's.
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
    run_counted(&mut whole, at + 12_250, &mut bytes);
    assert_eq!(
        state_of(&whole, RED_A),
        Some(MachineState::Patrol),
        "a whole hull comes back the moment the order stops standing"
    );

    let mut hurt = field_placed(&cfg, NARROW_SIGHT_CELLS, &[], [(1, 8), (1, 9), (13, 13)]);
    park(&mut hurt, BLUE);
    hurt_to_withdraw(&mut hurt, RED_A);
    let mut bytes = Vec::new();
    run_counted(&mut hurt, 2_000, &mut bytes);
    issue(&mut hurt, &order(Objective::Withdraw, 10_000));
    let at = hurt.now_ms;
    run_counted(&mut hurt, at + 12_250, &mut bytes);
    assert_eq!(
        state_of(&hurt, RED_A),
        Some(MachineState::Withdraw),
        "`Withdraw` is terminal for a hurt tank until it has rallied"
    );
    let started = memory_of(&hurt, RED_A)
        .quiet_since_ms
        .expect("the quiet clock starts at the expiry and not before it");
    assert!(
        started >= at + 10_000,
        "the clock had been running under the order: {started} ms"
    );
    let from = hurt.now_ms;
    run_counted(&mut hurt, from + rally_ms + 2_000, &mut bytes);
    assert_eq!(state_of(&hurt, RED_A), Some(MachineState::Patrol));
    assert!(
        memory_of(&hurt, RED_A).rallied,
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
/// into the order vocabulary's own bounds. 17b cut M4.5a: on a field of minds
/// (blue's parked by the `Mind → Pilot` transition).
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
        let brief =
            tacsim::deliberation::SquadBrief::from_signal(1, signal, 2, persona(signal.squad));
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
///
/// **17b cut M4.5a — on the unit mind** (Q15, D7). The stock mind ports the
/// AA.1 end (`immobile` is the view's `cannot_close`, which reads
/// `limits.motion_disabled`): `#4`'s first think after the order (2,482)
/// ends it — read off the stock recorder — by writing its `expires_ms` to
/// `ignore_until_ms`, so the end is re-pinned to that memory key with **no**
/// `order_expired` line and the order still standing on its row
/// (`order_end`), and `#6`, which never ends it, still obeys it and drives the
/// cell. The window grows from the reflex's tick (`+500`) to `#4`'s landing
/// (2,732: one think plus a landing, `+1,250`). The shot needs the presenting
/// profile (ruling 1). **Lost, not re-pinned (Q15)**: "the commander hears" —
/// a mind's end raises no `OrderExpired` signal before the order's own TTL, a
/// production consequence since 17b cut M4.3.
#[test]
fn aa1_an_advance_ends_for_the_machine_that_cannot_carry_it_out() {
    let cfg = arena_cfg(0);
    let mut arena = field(&cfg);
    park(&mut arena, BLUE);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
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
    let expires = order_on(&arena, RED_A)
        .map(|order| order.expires_ms)
        .expect("the order stands");
    run(&mut arena, at + 1_250, &mut bytes);

    let log = events(&mut arena, &mut bytes);
    assert!(
        expiries(&log).is_empty(),
        "`order_end`: no line before the TTL: {:?}",
        expiries(&log)
    );
    let kept = stock.kept();
    let (ended, _, _) = ending_think(&kept, RED_A, expires).expect("the order is over for RED_A");
    assert_eq!(
        ended,
        think_at(RED_A, 2),
        "for the machine that cannot close, on its first think after the order"
    );
    assert!(
        kept.iter()
            .any(|(unit, t_ms, view, _)| *unit == RED_A && *t_ms == ended && view.immobile),
        "the think read it could not close"
    );
    assert_eq!(memory_of(&arena, RED_A).ignore_until_ms, Some(expires));
    assert!(!obeys(&arena, RED_A), "the order is over for RED_A");
    assert!(
        order_on(&arena, RED_A).is_some(),
        "the arena still holds it"
    );
    // The mobile squadmate keeps it and is making for the cell.
    assert!(ending_think(&kept, RED_B, expires).is_none());
    assert!(
        obeys(&arena, RED_B),
        "RED_B can carry the order out and still holds it"
    );
    assert_eq!(goal_of(&arena, RED_B), Some(to));

    // On doctrine, it fights what it holds — from where it stands.
    let now = arena.now_ms;
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, now);
    run(&mut arena, now + 10_000, &mut bytes);
    assert!(
        fired_by(&events(&mut arena, &mut bytes), RED_A) > 0,
        "off the order, the immobile machine shoots at the hostile it holds"
    );
    assert_eq!(state_of(&arena, RED_A), Some(MachineState::Engage));
    assert_eq!(
        arena.unit(RED_A).map(|unit| unit.pose.clone()),
        stands,
        "and it never moved"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Y.1 — a pilot's order outranks the machines' own commander
// ---------------------------------------------------------------------------

/// Start one match on the production composition root with a pilot a side and
/// `ai_fill` fielding the rest, and answer it with its squads and the red
/// pilot's tank — the setup of the lapse tests (17b cut L.2's
/// `l2_the_world_says_the_leaders_order_lapsed_once` generalises it as
/// `l2_running_match`).
///
/// **17b cut L.1**: it answers the two pilots too — `[red, blue]`, each the
/// lead of their side (the admission reads who is at a seat).
fn y1_running_match(
    tw: &mut TestWorld,
) -> (
    tacsim::arena_task::MatchId,
    Vec<tacsim::deliberation::SquadRoster>,
    ObjId,
    [ObjId; 2],
) {
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
    tw.run(pilots[0], "sim team red");
    tw.run(pilots[1], "sim team blue");
    tw.run(pilots[0], "sim start");
    let mut red_tank = None;
    for pilot in &pilots {
        let pod = tacsim::match_::pod_of(tw.world(), *pilot).expect("a claimed slot locks a pod");
        let unit =
            tacsim::match_::tank_of(tw.world(), pod).expect("the pod's tank is in the world");
        red_tank.get_or_insert(unit);
        let match_id = tacsim::match_::centre_of(tw.world())
            .expect("the centre")
            .match_id
            .expect("a boarding centre has a match id");
        let pilot = *pilot;
        let _ = with_ctx(tw, pilot, |ctx| {
            tacsim::match_::slot_seated(ctx, match_id, pilot, unit).expect("the seat is reported");
        });
    }
    let match_id = tacsim::match_::centre_of(tw.world())
        .expect("the centre")
        .match_id
        .expect("the match started");
    let squads = tw
        .world()
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.squads.clone())
        .expect("the record carries its squads");
    (
        match_id,
        squads,
        red_tank.expect("the red pilot's tank"),
        [pilots[0], pilots[1]],
    )
}

/// One signal for `squad`, as a pulse of the arena would raise it.
fn y1_signal(roster: &tacsim::deliberation::SquadRoster) -> tacsim::arena::command::SquadSignal {
    tacsim::arena::command::SquadSignal {
        squad: roster.id,
        commander: roster.commander,
        team: roster.id.team,
        reason: tacsim::arena::command::SignalReason::NoOrder,
        at_ms: 0,
        digest: tacsim::arena::command::SquadDigest::default(),
        contacts: Vec::new(),
        bands_cm: Vec::new(),
        sight_cm: 0,
        default_ttl_ms: 30_000,
        health_permille: 1_000,
        states: [0; 7],
        cell: None,
        window: (16, 16),
        // 16b cut Z.1: the members ride the signal; the world never reads them.
        members: Vec::new(),
        objective: None,
    }
}

/// The machines' own commander's reply for `squad`: a `withdraw` standing a
/// minute.
fn y1_reply(match_id: tacsim::arena_task::MatchId, squad: SquadId) -> String {
    serde_json::to_string(&tacsim::deliberation::DeliberationReply::Ok {
        match_id: match_id.0,
        order: Order {
            squad,
            objective: Objective::Withdraw,
            target: None,
            horizon_ms: 60_000,
            constraints: Constraints::default(),
            ttl_ms: 60_000,
            issued_ms: 0,
            issuer: None,
            // 17b cut L.1: the squad's own lance-leader, as the record minted it.
            by: persona(squad),
            spot: None,
        },
        model: "mock".to_string(),
        prompt_hash: "hash".to_string(),
    })
    .expect("a reply encodes")
}

/// `(issued, refused, in_flight)`, the watch's issuer and (17b cut L.1) the
/// watch's author, as the registry holds them.
fn y1_state(
    tw: &TestWorld,
    match_id: tacsim::arena_task::MatchId,
    squad: SquadId,
) -> ((u64, u64, usize), Option<ObjId>, Option<CharacterRef>) {
    tw.world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| {
            (
                registry.deliberation_counts(match_id),
                registry.order_issuer(match_id, squad),
                registry.order_author(match_id, squad),
            )
        })
        .expect("the registry")
}

/// **16b cut Y.1 — the shield, both halves** (16c N3's default: *a human order
/// outranks a live LLM order*).
///
/// A request for the squad is in flight when the pilot speaks. Then:
///
/// 1. a signal for the squad under the pilot's live watch takes **no** in-flight
///    slot — `on_deliberate` spent nothing on it;
/// 2. the in-flight reply's `Ok` is **dropped** after its slot is given back:
///    `in_flight` is back to 0, nothing was issued (the fan-out to the task was
///    never reached), and the watch's issuer is still the pilot;
/// 3. once the watch has expired, the next signal is spent as it always was,
///    and a reply is issued and replaces the watch with the machines' own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_a_pilots_live_order_shields_the_squad_from_the_machines_commander() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, red_tank, [red_pilot, _]) = y1_running_match(&mut tw);
    // 17b cut L.1: the pilot's character, the author `note_human_order` records.
    let pilot_said = Some(CharacterRef::Character { id: red_pilot });
    let roster = squads
        .iter()
        .find(|roster| roster.id.team == Team::Red)
        .cloned()
        .expect("red fields machines");
    let squad = roster.id;
    let signal = y1_signal(&roster);

    // A request goes out before the pilot says anything.
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberate(ctx, match_id, std::slice::from_ref(&signal));
    });
    assert_eq!(y1_state(&tw, match_id, squad), ((0, 0, 1), None, None));

    // The pilot orders the squad: a watch with an issuer, ten minutes long.
    let now_ms = tw.world().now() * 1_000;
    tw.world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the registry")
        .note_human_order(
            match_id,
            squad,
            SmolStr::new("hold"),
            red_tank,
            CharacterRef::Character { id: red_pilot },
            now_ms + 600_000,
        );
    assert_eq!(
        y1_state(&tw, match_id, squad),
        ((0, 0, 1), Some(red_tank), pilot_said.clone())
    );

    // (1) A signal under the watch takes no slot.
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberate(ctx, match_id, std::slice::from_ref(&signal));
    });
    assert_eq!(
        y1_state(&tw, match_id, squad),
        ((0, 0, 1), Some(red_tank), pilot_said.clone()),
        "a squad under a pilot's order is not asked about"
    );

    // (2) The reply that was already in flight is dropped.
    let payload = y1_reply(match_id, squad);
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, squad),
        ((0, 0, 0), Some(red_tank), pilot_said),
        "the slot came back, nothing was issued and the pilot's watch stands"
    );

    // (3) Past the watch, the machines' commander is asked again, and its
    // answer lands.
    let now = tw.world().now();
    tw.set_now(now + 601);
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberate(ctx, match_id, std::slice::from_ref(&signal));
    });
    assert_eq!(
        y1_state(&tw, match_id, squad).0,
        (0, 0, 1),
        "after the expiry the next signal is spent"
    );
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, squad),
        ((1, 0, 0), None, Some(persona(squad))),
        "the machines' order is issued and its watch replaces the pilot's"
    );
}

// ---------------------------------------------------------------------------
// 17b cut L.1 — the lance-leader (doc 17 §4; D-UM-52, D-UM-53)
// ---------------------------------------------------------------------------

/// A reply for `squad` whose order is authored by `by` — [`y1_reply`]'s
/// `withdraw` with the author chosen (17b cut L.1).
fn l1_reply(match_id: tacsim::arena_task::MatchId, squad: SquadId, by: CharacterRef) -> String {
    serde_json::to_string(&tacsim::deliberation::DeliberationReply::Ok {
        match_id: match_id.0,
        order: Order {
            squad,
            objective: Objective::Withdraw,
            target: None,
            horizon_ms: 60_000,
            constraints: Constraints::default(),
            ttl_ms: 60_000,
            issued_ms: 0,
            issuer: None,
            by,
            spot: None,
        },
        model: "mock".to_string(),
        prompt_hash: "hash".to_string(),
    })
    .expect("a reply encodes")
}

/// `on_deliberate` over `signals`, answering the jobs it deferred (17b cut
/// L.1): what the world would hand the provider, one per request it spent.
fn l1_deliberate(
    tw: &mut TestWorld,
    match_id: tacsim::arena_task::MatchId,
    signals: &[tacsim::arena::command::SquadSignal],
) -> Vec<tacsim::deliberation::DeliberationJob> {
    let mut out = Outbox::new();
    {
        let engine = tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        tacsim::arena_bridge::on_deliberate(&mut ctx, match_id, signals);
    }
    out.deferred()
        .iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Effect(mush_core::Effect::ModuleAsync { payload, .. }) => {
                serde_json::from_str(payload).ok()
            }
            _ => None,
        })
        .collect()
}

/// The running match's red and blue rosters (17b cut L.1).
fn l1_sides(
    squads: &[tacsim::deliberation::SquadRoster],
) -> (
    tacsim::deliberation::SquadRoster,
    tacsim::deliberation::SquadRoster,
) {
    let side = |team: Team| {
        squads
            .iter()
            .find(|roster| roster.id.team == team)
            .cloned()
            .unwrap_or_else(|| panic!("{} fields machines", team.as_str()))
    };
    (side(Team::Red), side(Team::Blue))
}

/// **17b cut L.1 — the fixtures' orders carry an author and keep their scope.**
/// The file's `order` helper is a machine order, so it is authored by red-1's
/// lance-leader persona with `issuer: None`; issued, every unit of red-1 stands
/// under it with that author, and the hint scope is the commander's exactly as
/// before (`an_advance_order_moves_the_patrol_target` and the hint tests pass
/// unchanged): the squad makes for the cell.
///
/// **17b cut M4.5a — on the unit mind** (Q3; the M4.3 precedent,
/// `cockpit.rs`' `l1_a_typed_order_carries_the_seats_character`): at M4.5 the
/// author and the issuer are not mind-row state (the issuer is since 17b cut
/// M4.6, `MindState::order_issuer`; the author is not) — a mind's order is the
/// `FrameOrder` it reads (the squad word, the objective, the constraints and
/// the deadline), and D-UM-52's `by` is the world's watch's. So the author
/// and the issuer are asserted on the `Order` value issued (as before); the
/// "stands under it **with that author**" half is LOST (Q3) — every unit of
/// red-1 stands under the order on its mind row with red-1's squad word and
/// the objective, and the squad word comes from `Order.squad`, so it would
/// read the same for any `by`; and the issuer the order carried is the
/// tape's: one `ordered` line per unit naming red-1 and no issuer (`None`
/// against the fixture's `None`, as before — the commander's own scope).
#[test]
fn l1_the_deliberation_fixtures_carry_by_and_issuer_unchanged() {
    let cfg = arena_cfg(30_000);
    let mut arena = field(&cfg);
    let mut bytes = Vec::new();
    run(&mut arena, 2_000, &mut bytes);
    let told = order(Objective::Advance { to: (9, 3) }, 60_000);
    assert_eq!((&told.by, told.issuer), (&persona(red_one()), None));
    assert!(
        issue(&mut arena, &told)
            .iter()
            .all(|outcome| *outcome == IntentOutcome::AppliedContinuous)
    );
    let CharacterRef::Persona { squad } = &told.by else {
        unreachable!("the fixture's author is a persona");
    };
    for unit in [RED_A, RED_B] {
        let standing =
            order_on(&arena, unit).unwrap_or_else(|| panic!("{unit:?} stands under the order"));
        assert_eq!(standing.squad, squad.to_string(), "{unit:?}");
        assert_eq!(
            standing.objective,
            tacsim::arena::perception::MindObjective::Advance { to: (9, 3) },
            "{unit:?}"
        );
    }
    let ordered: Vec<(ObjId, String, Option<ObjId>)> = events(&mut arena, &mut bytes)
        .into_iter()
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::Ordered {
                unit,
                squad,
                issuer,
                ..
            } => Some((unit, squad.to_string(), issuer)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ordered,
        vec![
            (RED_A, "red-1".to_string(), None),
            (RED_B, "red-1".to_string(), None)
        ],
        "the tape carries the issuer, unchanged"
    );
    run(&mut arena, 4_000, &mut bytes);
    assert_eq!(goal_of(&arena, RED_A), Some((9, 3)));
    assert_eq!(goal_of(&arena, RED_B), Some((9, 3)));
}

/// **17b cut L.1 — a wrecked leader's hull drops its squad to doctrine**
/// (`squad.rs`'s head, D-UM-53): the lance-leader rides the commander, so a
/// commander that is a wreck raises no signal for its squad — at the opening
/// pulse, and for the next minute of arena time — while the other squad still
/// signals. 17b cut M4.5a: on a field of minds (a wreck's mind thinks
/// nothing, and the clause is the arena's).
#[test]
fn l1_a_wrecked_leaders_hull_drops_its_squad_to_doctrine() {
    let cfg = arena_cfg(30_000);
    let squads_of_signals = |signals: &[tacsim::arena::command::SquadSignal]| {
        signals
            .iter()
            .map(|signal| signal.squad.to_string())
            .collect::<Vec<String>>()
    };
    let mut whole = field(&cfg);
    assert_eq!(
        squads_of_signals(&whole.step(0, &[], &[]).deliberation),
        vec!["red-1", "blue-1"],
        "the control: both squads ask at the opening pulse"
    );

    let mut arena = field(&cfg);
    if let Some(commander) = arena.unit_mut(RED_A) {
        commander.limits.wreck = true;
    }
    assert_eq!(
        squads_of_signals(&arena.step(0, &[], &[]).deliberation),
        vec!["blue-1"],
        "the wrecked leader's squad asks nothing; the other still does"
    );
    let mut bytes = Vec::new();
    let later = run(&mut arena, 60_000, &mut bytes);
    assert!(
        later.iter().all(|signal| signal.squad != red_one()),
        "red-1 never asks again: {:?}",
        squads_of_signals(&later)
    );
}

/// **17b cut L.1 — a match mints one persona per squad** (D-UM-53), through
/// the lobby: the record's `leaders` is one `Persona` per `squads` row, in the
/// same order, named `red lance leader 1` and `blue lance leader 1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_match_mints_one_persona_per_squad() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, _, _) = y1_running_match(&mut tw);
    let record = tw
        .world()
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .cloned()
        .expect("the record");
    assert_eq!(record.squads, squads);
    assert!(!squads.is_empty(), "ai_fill fields machines");
    assert_eq!(record.leaders.len(), record.squads.len());
    for (leader, roster) in record.leaders.iter().zip(&record.squads) {
        assert_eq!(leader.squad, roster.id);
        assert_eq!(leader.by, persona(roster.id));
    }
    let names: Vec<String> = record
        .leaders
        .iter()
        .map(|leader| CharacterRef::persona_name(leader.squad))
        .collect();
    assert!(
        names.contains(&"red lance leader 1".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"blue lance leader 1".to_string()),
        "{names:?}"
    );
}

/// **17b cut L.1 — deliberation is requested per leader, and a leaderless
/// squad runs on doctrine** (D-UM-53). With blue's row cleared from the
/// record's `leaders`, `on_deliberate` over both squads' signals spends one
/// in-flight slot and defers one job — red's, whose brief carries red-1's
/// persona as its `leader` — and a reply for the leaderless squad is dropped:
/// counted `refused`, nothing issued, no watch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_deliberation_is_requested_per_leader_and_a_leaderless_squad_runs_on_doctrine() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, _, _) = y1_running_match(&mut tw);
    let (red, blue) = l1_sides(&squads);
    if let Some(record) = tw
        .world_mut()
        .module_state_mut::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        record.leaders.retain(|leader| leader.squad != blue.id);
    }

    let jobs = l1_deliberate(&mut tw, match_id, &[y1_signal(&red), y1_signal(&blue)]);
    let briefs: Vec<(SquadId, CharacterRef)> = jobs
        .into_iter()
        .map(
            |tacsim::deliberation::DeliberationJob::Deliberate { brief }| {
                (brief.squad, brief.leader)
            },
        )
        .collect();
    assert_eq!(
        briefs,
        vec![(red.id, persona(red.id))],
        "one job, red's, asked of its lance-leader"
    );
    assert_eq!(
        y1_state(&tw, match_id, blue.id),
        ((0, 0, 1), None, None),
        "the leaderless squad spent no slot"
    );

    let payload = l1_reply(match_id, blue.id, persona(blue.id));
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, blue.id),
        ((0, 1, 0), None, None),
        "a reply for a squad no character leads is dropped and counted"
    );
}

/// **17b cut L.1 — a reply whose author is not the squad's leader is dropped**
/// (D-UM-52, D-UM-53): `by` naming blue-1's persona for red-1, and `by` naming
/// an object that leads nothing, are each dropped — counted `refused`, nothing
/// issued, no watch. A reply for a match id the world never tracked returns
/// quietly, counted nowhere; and so does a reply for a squad of an **ended**
/// match — whose record stays on the book with its squads and leaders while
/// `untrack` has taken its deliberation row — whether its author is wrong (the
/// drop's count goes through `get_mut`, so no row is created) or right (the
/// fan-out finds no unit in a running match, so nothing is issued).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_reply_whose_by_is_not_the_squads_leader_is_dropped() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, _, _) = y1_running_match(&mut tw);
    let (red, blue) = l1_sides(&squads);
    let lobby =
        tacsim::match_::centre_object(tw.world(), tacsim::match_::LOBBY_ENTRY).expect("the lobby");
    // One request in flight, so the match's deliberation row exists: a drop is
    // counted through `get_mut` and never creates the row (a reply for a match
    // that never asked counts nowhere, as the stale one below shows).
    assert_eq!(
        l1_deliberate(&mut tw, match_id, &[y1_signal(&red)]).len(),
        1
    );
    assert_eq!(y1_state(&tw, match_id, red.id).0, (0, 0, 1));

    for (n, by) in [persona(blue.id), CharacterRef::Character { id: lobby }]
        .into_iter()
        .enumerate()
    {
        let payload = l1_reply(match_id, red.id, by.clone());
        let _ = with_ctx(&mut tw, GOD, |ctx| {
            tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
        });
        assert_eq!(
            y1_state(&tw, match_id, red.id),
            ((0, n as u64 + 1, 0), None, None),
            "{by:?} does not lead red-1"
        );
    }

    let gone = tacsim::arena_task::MatchId(match_id.0 + 1_000);
    let payload = l1_reply(gone, red.id, persona(red.id));
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, gone, red.id),
        ((0, 0, 0), None, None),
        "a reply for a match never tracked returns quietly"
    );
    assert_eq!(y1_state(&tw, match_id, red.id).0, (0, 2, 0));

    // The match ends through the real check-in: `untrack` takes the
    // deliberation row, and the finished record stays on the book.
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        assert!(
            tacsim::match_::finish(
                ctx,
                match_id,
                tacsim::arena::MatchEnd {
                    reason: tacsim::arena::EndReason::Abort,
                    t_ms: 12_000,
                },
                "0".repeat(64),
                &[],
            ),
            "the match had a live record to check in"
        );
    });
    assert_eq!(y1_state(&tw, match_id, red.id), ((0, 0, 0), None, None));
    let ended = tw
        .world()
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .cloned()
        .expect("a finished record stays on the book");
    assert!(!ended.state.is_live(), "the record is finished");
    assert!(
        ended.squads.iter().any(|roster| roster.id == red.id)
            && tacsim::deliberation::leader_of(&ended.leaders, red.id) == Some(&persona(red.id)),
        "the roster lookup and the leader check still find red-1"
    );
    for by in [persona(blue.id), persona(red.id)] {
        let payload = l1_reply(match_id, red.id, by.clone());
        let _ = with_ctx(&mut tw, GOD, |ctx| {
            tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
        });
        assert_eq!(
            y1_state(&tw, match_id, red.id),
            ((0, 0, 0), None, None),
            "{by:?}: a stale reply for an ended match counts nowhere and creates no row"
        );
    }
}

/// **17b cut L.1 — a forged reply with no `by` is undecodable and gives its
/// slot back, once.** With two slots in flight (red's and blue's), a
/// hand-written `ok` reply whose order lacks `by` — what a provider path from
/// before this cut would write — issues nothing, is not counted `refused` (it
/// was never decoded) and gives back exactly one slot: in flight 2 → 1, not
/// leaked for the life of the match as an undecodable reply's was until this
/// cut, and not two (a double release would free another squad's outstanding
/// request). A second forged reply takes it 1 → 0; and a decodable reply, with
/// two in flight again, also gives back exactly one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_forged_reply_with_no_by_is_undecodable_and_releases_its_slot() {
    let mut tw = TestWorld::with_modules(roster());
    let (match_id, squads, _, _) = y1_running_match(&mut tw);
    let (red, blue) = l1_sides(&squads);
    let jobs = l1_deliberate(&mut tw, match_id, &[y1_signal(&red), y1_signal(&blue)]);
    assert_eq!(jobs.len(), 2);
    assert_eq!(y1_state(&tw, match_id, red.id).0, (0, 0, 2));

    let mut forged: serde_json::Value =
        serde_json::from_str(&l1_reply(match_id, red.id, persona(red.id))).expect("json");
    if let Some(order) = forged
        .get_mut("order")
        .and_then(serde_json::Value::as_object_mut)
    {
        order.remove("by");
    }
    assert!(
        serde_json::from_value::<tacsim::deliberation::DeliberationReply>(forged.clone()).is_err(),
        "a reply with no author does not decode"
    );
    let payload = forged.to_string();
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, red.id),
        ((0, 0, 1), None, None),
        "nothing issued, nothing counted, one slot back and only one"
    );
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, red.id),
        ((0, 0, 0), None, None),
        "the second forged reply gives back the second slot"
    );

    // The decoded path releases once too: two in flight, one well-formed reply
    // from red-1's own leader, one slot back and the order issued.
    assert_eq!(
        l1_deliberate(&mut tw, match_id, &[y1_signal(&red), y1_signal(&blue)]).len(),
        2
    );
    assert_eq!(y1_state(&tw, match_id, red.id).0, (0, 0, 2));
    let payload = l1_reply(match_id, red.id, persona(red.id));
    let _ = with_ctx(&mut tw, GOD, |ctx| {
        tacsim::arena_bridge::on_deliberation_reply(ctx, &payload);
    });
    assert_eq!(
        y1_state(&tw, match_id, red.id),
        ((1, 0, 1), None, Some(persona(red.id))),
        "a decoded reply gives back exactly one slot"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.2 — the two squad roles on the LLM seam (16c N6)
// ---------------------------------------------------------------------------

/// **16b cut Z.2 (16c N6)**: the forced tool offers six objectives and the two
/// payload fields; a scripted reply through the whole structured pipeline maps
/// `screen` 90 to 90,000 millidegrees and `support` member 2 to the second
/// member's own unit; a bearing of 360 and an ordinal that names no member are
/// refused twice over (the original and the one repair) and never issued; and
/// the `support` order the reply produced, fanned to red-1 as the bridge fans a
/// reply out, lands one `ordered` line per unit with `objective=support`.
/// 17b cut M4.5a: red-1's hulls are minds, and a mind's order journals the same
/// `ordered` line as the reflex's did (`apply_intent`'s order arm).
#[tokio::test]
async fn z2_a_scripted_screen_and_support_map_and_a_support_lands_on_every_unit() {
    use tacsim::deliberation::{RefusedReason, SquadBrief, deliberate, order_tool};

    let schema = order_tool().input_schema;
    let objectives: Vec<&str> = schema
        .pointer("/properties/objective/enum")
        .and_then(serde_json::Value::as_array)
        .map(|words| words.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        objectives,
        vec!["hold", "advance", "engage", "withdraw", "screen", "support"],
        "the tool lists six objectives"
    );
    for field in ["screen_bearing_deg", "support_member"] {
        assert!(
            schema.pointer(&format!("/properties/{field}")).is_some(),
            "the tool carries `{field}`"
        );
    }

    let (llm, mock) = mock_service();
    let mut arena = field(&arena_cfg(0));
    let mut bytes = Vec::new();
    plant_contact(&mut arena, RED_A, BLUE, Team::Blue, 0);
    let signals = run(&mut arena, 2_000, &mut bytes);
    let signal = signals
        .iter()
        .find(|signal| signal.squad == red_one())
        .expect("red-1 asked");
    let brief = SquadBrief::from_signal(1, signal, 2, persona(signal.squad));
    let second = brief
        .members
        .iter()
        .find(|member| member.ordinal == 2)
        .map(|member| ObjId(member.unit))
        .expect("red-1 reports its second member");
    assert_eq!(second, RED_B, "the second member is the wingman");

    mock.push_response(tool_use(
        serde_json::json!({"objective": "screen", "screen_bearing_deg": 90, "ttl_ms": 20_000}),
        0.000_10,
    ));
    let screen = deliberate(&llm, &brief).await.expect("a screen");
    assert_eq!(
        screen.objective,
        Objective::Screen {
            facing_mdeg: 90_000
        }
    );
    mock.push_response(tool_use(
        serde_json::json!({"objective": "support", "support_member": 2, "ttl_ms": 20_000}),
        0.000_10,
    ));
    let support = deliberate(&llm, &brief).await.expect("a support");
    assert_eq!(support.objective, Objective::Support { of: RED_B });

    for bad in [
        serde_json::json!({"objective": "screen", "screen_bearing_deg": 360, "ttl_ms": 20_000}),
        serde_json::json!({"objective": "support", "support_member": 9, "ttl_ms": 20_000}),
    ] {
        mock.push_response(tool_use(bad.clone(), 0.000_10));
        mock.push_response(tool_use(bad.clone(), 0.000_10));
        let error = deliberate(&llm, &brief)
            .await
            .expect_err("a malformed order is never issued");
        assert_eq!(RefusedReason::of(&error), RefusedReason::Malformed, "{bad}");
    }

    // The reply's order, fanned to every unit of the squad — the charge
    // included, whose `of` is itself and which fights on doctrine.
    let outcomes = issue(&mut arena, &support);
    assert!(
        outcomes
            .iter()
            .all(|outcome| *outcome == IntentOutcome::AppliedContinuous),
        "{outcomes:?}"
    );
    let lines: Vec<LedgerEvent> = events(&mut arena, &mut bytes)
        .into_iter()
        .filter(|event| event.kind.as_str() == "ordered")
        .collect();
    assert_eq!(lines.len(), 2, "one line per unit of red-1: {lines:?}");
    for line in &lines {
        assert!(
            matches!(
                &line.kind,
                tacsim::arena::ledger::EventKind::Ordered { objective, squad, .. }
                    if objective == "support" && squad == "red-1"
            ),
            "{line:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — the commander is told the objective (16c N11; plan items 14,
// 33)
// ---------------------------------------------------------------------------

/// [`field_placed`]'s three machines on a ground with a Hold declared at
/// `(8, 12)` (map letter `A`, the ground's first spot), six cells round it,
/// 120 s — fought at `mode`.
fn o1_field(cfg: &ArenaConfig, mode: tacsim::arena::MatchMode) -> Arena {
    let mut definition = definition();
    definition.key_spots = vec![tacsim::arena::state::KeySpot {
        name: SmolStr::new("The Spot"),
        x: 8,
        y: 12,
    }];
    definition.modes = vec![tacsim::arena::ArenaMode::Hold(tacsim::arena::HoldRule {
        spot: 0,
        radius_cells: 6,
        hold_ms: 120_000,
    })];
    let units = vec![
        tank_seeing(
            RED_A,
            Team::Red,
            centre(2, 4),
            Controller::Mind,
            WIDE_SIGHT_CELLS,
        ),
        tank_seeing(
            RED_B,
            Team::Red,
            centre(2, 6),
            Controller::Mind,
            WIDE_SIGHT_CELLS,
        ),
        tank_seeing(
            BLUE,
            Team::Blue,
            centre(14, 4),
            Controller::Mind,
            WIDE_SIGHT_CELLS,
        ),
    ];
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode,
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    arena
}

/// The text of a request's user turn.
fn o1_user_text(request: &mush_llm::CompletionRequest) -> String {
    request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            mush_llm::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **16b cut O.1 (16c N11; plan items 14, 33)**: a Hold arena's first
/// `NoOrder` signal carries the objective — the spot's cell, its letter, the
/// radius and the time in whole seconds — and the prompt the provider is sent
/// prints it on one `objective:` line in words that are none of the tool's
/// orders, beside the system turn's mapping sentence; a skirmish's signal
/// carries none and its prompt prints none. 17b cut M4.5a: on a field of minds
/// (the signal's objective is the arena's; no mind is asked to hold).
#[tokio::test]
async fn o1_a_hold_arenas_first_signal_tells_the_commander_the_objective() {
    use tacsim::deliberation::{SquadBrief, deliberate};

    let (llm, mock) = mock_service();
    let mut arena = o1_field(&arena_cfg(0), tacsim::arena::MatchMode::Hold);
    let mut bytes = Vec::new();
    let signals = run(&mut arena, 2_000, &mut bytes);
    let first = signals
        .iter()
        .find(|signal| signal.reason == tacsim::arena::command::SignalReason::NoOrder)
        .expect("a squad asked with no order");
    assert_eq!(
        first.objective,
        Some(tacsim::arena::BriefObjective {
            mode: "hold".to_owned(),
            cell: (8, 12),
            letter: Some('A'),
            radius_cells: 6,
            hold_s: 120,
        })
    );
    assert!(
        signals
            .iter()
            .all(|signal| signal.objective == first.objective),
        "every squad of both sides is told the same objective"
    );
    mock.push_response(tool_use(
        serde_json::json!({"objective": "engage", "ttl_ms": 20_000}),
        0.000_25,
    ));
    let brief = SquadBrief::from_signal(1, first, 2, persona(first.squad));
    deliberate(&llm, &brief).await.expect("the mock answers");
    let request = mock.last_request().expect("the provider was asked");
    let user = o1_user_text(&request);
    assert!(
        user.contains(
            "objective: key spot A at 8,12; the side alone inside 6 cells of it for 120 s wins \
             the match\n"
        ),
        "{user}"
    );
    // `In a key-spot match: engage keeps doctrine, which drives the squad to
    // the key spot …` until 17b cut M5.6 (OQ-6; STOP A, the text made true).
    let system = request.system.clone().unwrap_or_default();
    assert!(
        system.contains(
            "In a key-spot match: the squad does not drive for the key spot unless it is \
             ordered to advance on it;"
        ),
        "{system}"
    );

    // A skirmish of the same field: no objective, and no line.
    let mut skirmish = o1_field(&arena_cfg(0), tacsim::arena::MatchMode::Skirmish);
    let mut bytes = Vec::new();
    let signals = run(&mut skirmish, 2_000, &mut bytes);
    assert!(!signals.is_empty());
    assert!(signals.iter().all(|signal| signal.objective.is_none()));
    mock.push_response(tool_use(
        serde_json::json!({"objective": "engage", "ttl_ms": 20_000}),
        0.000_25,
    ));
    let brief = SquadBrief::from_signal(1, &signals[0], 2, persona(signals[0].squad));
    deliberate(&llm, &brief).await.expect("the mock answers");
    let request = mock.last_request().expect("the provider was asked");
    assert!(!o1_user_text(&request).contains("objective:"));
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — a range's targets ask the commander nothing (16c N13)
// ---------------------------------------------------------------------------

/// **A range**: red's pair flown by unit minds, armed, and six `target_mk1`s
/// off the fielded catalog on blue, checked out by the production functions at
/// [`tacsim::arena::MatchMode::Range`] — the targets under `targets` (minds in
/// a range, as `match_::start` checks them out since 17b cut M4.3; seated for
/// the baseline). 17b cut M4.5a: the reflex controller before it.
fn o3_range(cfg: &ArenaConfig, targets: Controller) -> Arena {
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let key = mush_core::CanonicalId::new(tacsim::definitions::content::TARGET_MK1)
        .expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the target is fielded");
    let binding = catalog.bindings.get(&key).expect("its binding");
    let mut units = vec![
        tank(RED_A, Team::Red, centre(2, 8), Controller::Mind),
        tank(RED_B, Team::Red, centre(2, 9), Controller::Mind),
    ];
    for (n, y) in (4_u16..10).enumerate() {
        let id = ObjId(40 + u32::try_from(n).unwrap_or(0));
        let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("a target");
        let slot = u8::try_from(n + 1).unwrap_or(1);
        units.push(
            tacsim::match_::custody_unit(
                &catalog,
                cells,
                id,
                Team::Blue,
                centre(12, y),
                0,
                targets,
                slot,
                &tacsim::match_::SealTerms::fixture(),
            )
            .expect("the fixture terms seal"),
        );
    }
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Range,
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the range");
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    arena
}

/// **A range raises no deliberation signal for its targets, and its shooters'
/// signals are what they would be without the targets in squads at all** (16b
/// cut O.3; 16c N13; plan item 14).
///
/// The targets are dealt into squads like any machine side — `blue-1` and
/// `blue-2`, three apiece at `squad_size` 4, both `squads_of` callers untouched —
/// and without the inert-commander clause each would ask the commander at its
/// first pulse and after every expiry. Over a minute of arena time at the
/// shipped floor, the range asks for red alone. The baseline is the same field
/// with the six targets seated — so in no squad — which is what "unchanged"
/// means: red's signals, reason, digest, contacts, members and all, are the
/// same list. And every one of them goes through the mock service: one provider
/// call per red signal, and not one attributed to a target.
///
/// **17b cut M4.5a — on unit minds**: red's pair and the six targets are minds
/// — the targets as production checks them out since 17b cut M4.3. Every claim
/// holds unchanged: the silence is the arena's inert-commander clause,
/// controller-free, and red's signals equal the seated-target baseline's —
/// what a target's mind does moves nothing a signal carries. (Until 17b cut
/// M4.6b a target's mind started up, searched and turned in place, its
/// authored zero speed not `immobile` on its view; since the owner's STOP B
/// ruling it starts up and then parks on the `Disabled` edge, closer still to
/// the seated baseline.)
#[tokio::test]
async fn o3_a_range_asks_the_commander_nothing_for_its_targets() {
    use tacsim::deliberation::{SquadBrief, deliberate};

    let cfg = ArenaConfig {
        squad_size: 4,
        ..arena_cfg(ArenaConfig::default().deliberate_min_ms)
    };
    let mut range = o3_range(&cfg, Controller::Mind);
    let blue_squads: Vec<(String, usize)> = range
        .squads
        .iter()
        .filter(|roster| roster.id.team == Team::Blue)
        .map(|roster| (roster.id.to_string(), roster.units.len()))
        .collect();
    assert_eq!(
        blue_squads,
        vec![("blue-1".to_owned(), 3), ("blue-2".to_owned(), 3)],
        "the targets are in squads; the clause, not the grouping, silences them"
    );
    let mut bytes = Vec::new();
    let signals = run(&mut range, 60_000, &mut bytes);
    assert!(
        signals.iter().all(|signal| signal.team == Team::Red),
        "a target's squad asked: {:?}",
        signals
            .iter()
            .filter(|signal| signal.team == Team::Blue)
            .map(|signal| signal.squad.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        signals
            .first()
            .is_some_and(|signal| signal.squad == red_one()
                && signal.reason == tacsim::arena::command::SignalReason::NoOrder),
        "red asks at its first pulse as any machine squad does"
    );

    let mut baseline = o3_range(&cfg, Controller::Pilot);
    assert!(
        baseline
            .squads
            .iter()
            .all(|roster| roster.id.team == Team::Red),
        "seated targets are in no squad"
    );
    let mut bytes = Vec::new();
    let unchanged = run(&mut baseline, 60_000, &mut bytes);
    assert_eq!(signals, unchanged, "red's signals are unchanged");

    // Through the mock service: one call per red signal, none for a target.
    let (llm, mock) = mock_service();
    for signal in &signals {
        mock.push_response(tool_use(
            serde_json::json!({"objective": "engage", "ttl_ms": 20_000}),
            0.000_25,
        ));
        let brief = SquadBrief::from_signal(1, signal, 2, persona(signal.squad));
        let order = deliberate(&llm, &brief).await.expect("the mock answers");
        assert_eq!(order.squad.team, Team::Red);
    }
    assert_eq!(mock.call_count(), signals.len());
    assert!(
        llm.audit_log()
            .recent(64)
            .iter()
            .all(|row| matches!(row.actor, Some(RED_A | RED_B))),
        "every call is a shooter's commander's"
    );
}
