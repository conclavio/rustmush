//! **The sixteen-cell fixture rig** (lifted out of `tests/reflex.rs` by 17b cut
//! M4.4 so `tests/motor.rs` can field the motor family on it): the window, the
//! eighteen-point tank, the two-node graph, the doctrine config, the arena
//! builders, the stepping loop and the black-box readers every controller
//! fixture in this crate was written against. Nothing here is production
//! code; see `tests/reflex.rs`' head for why a test may plant a contact or a
//! wound by hand.

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellAddr, CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
    TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::doctrine::MachineState;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::motor::MotorRow;
use tacsim::arena::perception::{SensorProfile, SignatureFactor};
use tacsim::arena::rng::draw_contact_id;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, Controller, CustodyOut, CustodyUnit,
    EntryZone, Pilot, Power, SeenPose, ShadowContact, PartyId, TerrainCell, TrackedRules, Waypoint,
    Waypoints, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};

/// The window's side, in cells.
pub const SIDE: u16 = 16;

/// One cell, centimetres.
pub const CELL_CM: u16 = 1_000;

/// The match seed every fixture runs on.
pub const SEED: u64 = 2_350;

/// The red tank.
pub const RED: ObjId = ObjId(4);

/// The blue tank.
pub const BLUE: ObjId = ObjId(9);

/// A second red tank, for the cross-tempo property.
pub const RED2: ObjId = ObjId(6);

/// **17b cut L.1**: the scripted pilot character a pilot-shaped order names as
/// its author; no unit of these fixtures uses the id.
pub const PILOT_CHARACTER: ObjId = ObjId(9_001);

/// **17b cut L.1** (D-UM-52): an order's author by its shape — a machine order
/// (`issuer: None`) by the squad's lance-leader persona, a pilot's by
/// [`PILOT_CHARACTER`]. The arena reads neither; the world's admission does.
pub fn author(
    squad: tacsim::deliberation::SquadId,
    issuer: Option<ObjId>,
) -> tacsim::deliberation::CharacterRef {
    match issuer {
        None => tacsim::deliberation::CharacterRef::Persona { squad },
        Some(_) => tacsim::deliberation::CharacterRef::Character {
            id: PILOT_CHARACTER,
        },
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

pub fn dref(kind: &str, id: &str) -> DefinitionRef {
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

pub fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
        .expect("an empty rules blob")
}

/// The smallest unit the arena can carry: one hull section, one front facet, one
/// armour cell of 10 and 8 internals — an eighteen-point tank, which is what
/// makes test (d)'s arithmetic readable.
pub fn tactical_unit(id: ObjId) -> TacticalUnit {
    tactical_unit_ruled(id, rules())
}

/// [`tactical_unit`]'s one shape under any rules blob — **16b cut AE.3** reads
/// a declared class off it (`UnitClass::read`), so a fixture can field a drone.
pub fn tactical_unit_ruled(id: ObjId, rules: RulesValue) -> TacticalUnit {
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
        rules,
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

pub fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

pub fn wall() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Wall,
        elev_cm: 0,
        move_cost: 0,
    }
}

pub fn pose(x_cm: i64, y_cm: i64) -> Pose {
    Pose::at(x_cm, y_cm, 0, rules())
}

/// The cell centre of `(x, y)`.
pub fn centre(x: u16, y: u16) -> Pose {
    pose(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
    )
}

/// The two-node patrol graph every fixture walks: the west and east ends of row
/// 8, joined. The smallest graph on which `Patrol` produces a route.
pub fn graph() -> Waypoints {
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
pub fn bluff() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 3_000,
        move_cost: 100,
    }
}

/// The window, with `paint`ed cells over uniform ground.
pub fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    definition_with(paint, graph())
}

/// The same, with a named waypoint graph — **16b cut Q.7** needs one window with
/// no graph at all, so that the `unreachable` edge has nowhere to fall back to
/// and the follower's own one-cell test is the only thing left between the tank
/// and the bluff.
pub fn definition_with(paint: &[(u16, u16, TerrainCell)], waypoints: Waypoints) -> ArenaDefinition {
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
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE - 1,
            },
            EntryZone {
                team: PartyId::BLUE,
                x0: SIDE - 2,
                y0: 0,
                x1: SIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints,
        modes: Vec::new(),
    }
}

/// A five-cell sight, so a sixteen-cell window has an outside and a contact can
/// be made to expire.
pub fn sight() -> SensorProfile {
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
pub fn cannon() -> WeaponSummary {
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
pub fn carbine() -> WeaponSummary {
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
pub fn tank(
    id: ObjId,
    team: PartyId,
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
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// **The same custody sealed as the production fill is** (17b cut M4.3):
/// `SealTerms::production_fixture()` over the custody as it stands, for the
/// (j) field's production pin.
pub fn as_fill(mut unit: CustodyUnit) -> CustodyUnit {
    unit.reseal(&tacsim::match_::SealTerms::production_fixture())
        .expect("the production terms seal");
    unit
}

/// **The production installer over Standard Doctrine** (17b cut M4.3): every
/// scripted seal's runner installed from the compiled-in stock source, as a
/// production match installs it; a no-op when the fill engine is `Native`.
pub fn install_standard(arena: &mut Arena) -> usize {
    let cfg = arena.arena_cfg.clone();
    tacsim::minds::install_sealed_runners(arena, &cfg, &|_| {
        Some(tacsim::minds::STANDARD_RHAI.to_owned())
    })
    .expect("Standard Doctrine installs")
}

/// **A profile that presents every band** (17b cut M4.3; `tests/minds.rs`'
/// `every_band_profile`): four bands of one hull section and no
/// consequence, so a mind's slot has odds to clear its floor with — a
/// `UnitProfile::default()` target presents no band, and a slot never fires
/// at it, where the reflex machine fired regardless.
pub fn presenting_profile() -> tacsim::arena::resolution::UnitProfile {
    tacsim::arena::resolution::UnitProfile {
        bands: ["front", "left", "right", "rear"]
            .into_iter()
            .map(|band| tacsim::arena::resolution::BandProfile {
                band: SmolStr::new(band),
                width_cm: 300,
                height_cm: 200,
                sections: vec![(SmolStr::new("hull"), 1)],
            })
            .collect(),
        sections: vec![tacsim::arena::resolution::SectionProfile {
            id: SmolStr::new("hull"),
            facets: vec![SmolStr::new("front")],
            consequences: Vec::new(),
            struck_through: None,
        }],
    }
}

/// The same custody presenting every band ([`presenting_profile`]), sealed
/// again as it stands.
pub fn presenting(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        profile: presenting_profile(),
        ..unit
    }
    .sealed()
}

/// The same custody seeing `cells` (its sensor's range), sealed again.
pub fn seeing(unit: CustodyUnit, cells: u16) -> CustodyUnit {
    with_sight(unit, cells).sealed()
}

/// **What the machine flying `unit` is doing** (17b cut M4.3): its doctrine
/// state through `Arena::machine_state` — the mind's off its cached state
/// (`unknown` for a mind not sealed on Standard Doctrine since 17b cut M4.6,
/// which deleted the reflex and its row's arm) — `None` for a pilot's hull.
/// (The rig's `state_of`, the reflex row's state, was deleted with it.)
pub fn machine_state_of(arena: &Arena, unit: ObjId) -> Option<MachineState> {
    arena.machine_state(unit)
}

/// **The stock memory a mind's row holds** (17b cut M4.3): Standard
/// Doctrine's keys as the native mind reads them off the row's bytes.
pub fn stock_memory_of(arena: &Arena, unit: ObjId) -> tacsim::arena::stock_mind::StockMemory {
    arena
        .minds
        .get(&unit)
        .map(|row| {
            tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
                &row.memory,
            ))
        })
        .unwrap_or_default()
}

/// The same tank with a longer sight, so a hostile stays held while the machine
/// drives the other way — which is what an `Approach` on a sixteen-cell window
/// needs.
pub fn with_sight(mut unit: CustodyUnit, cells: u16) -> CustodyUnit {
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
pub fn with_mesh(mut unit: CustodyUnit, cells: u16, share: u16) -> CustodyUnit {
    unit.sensor = Some(SensorProfile {
        max_range_cells: cells,
        share_range_cells: share,
        ..sight()
    });
    unit
}

/// A match short enough to test: power comes up in half a second.
pub fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// The doctrine these tests run on: decide every 500 ms and re-plan whenever the
/// route is gone, so an edge is visible in a handful of pulses rather than in
/// four seconds of arena time.
pub fn reflex_cfg() -> ReflexConfig {
    ReflexConfig {
        decide_ms: 500,
        repath_ms: 0,
        ..ReflexConfig::default()
    }
}

pub fn arena_cfg(reflex: ReflexConfig) -> ArenaConfig {
    ArenaConfig {
        reflex,
        ..ArenaConfig::default()
    }
}

/// An arena over `paint`ed ground, with `pilots` seated (by unit id, `#1000` up).
pub fn arena_of(
    units: Vec<CustodyUnit>,
    paint: &[(u16, u16, TerrainCell)],
    cfg: &ArenaConfig,
    seated: &[ObjId],
) -> Arena {
    arena_on(definition(paint), units, cfg, seated)
}

/// The same over a whole [`ArenaDefinition`] — **16b cut Q.7** needs a window
/// with no waypoint graph.
pub fn arena_on(
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
            mode: tacsim::arena::MatchMode::Skirmish,
            host: tacsim::arena::FieldHost::all_hostile(),
        },
        cfg,
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// Step `until_ms` of arena time at 500 ms, absorbing the ledger and the
/// motor's counters (the reflex's, named so, until 17b cut M4.5).
pub fn run(
    arena: &mut Arena,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> tacsim::arena::motor::MotorCounters {
    let mut wall = arena.now_ms;
    let mut total = tacsim::arena::motor::MotorCounters::default();
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        total.absorb(&out.motor);
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    total
}

pub fn absorb(out: &StepOutput, bytes: &mut Vec<u8>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
}

pub fn events(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
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

pub fn count(events: &[LedgerEvent], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.kind.as_str() == kind)
        .count()
}

/// **The hull's motor row** (17b cut M1a.1): where the driving fields — the
/// goal, the route, the search, the remembered stops, the waypoint walk — live
/// since they left the reflex row (`Arena::motors`). A clone, or a default row
/// when the table holds none for `unit`, which is what the reflex row's own
/// fields read before the split.
pub fn motor(arena: &Arena, unit: ObjId) -> MotorRow {
    arena.motors.row(unit).cloned().unwrap_or_default()
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
pub fn plot_of(arena: &Arena, target: ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

/// Make `viewer` hold `target` as a classified hostile from `at_ms`, whatever the
/// ground between them says. See this file's head for why a test may do this.
pub fn plant_contact(arena: &mut Arena, viewer: ObjId, target: ObjId, team: PartyId, at_ms: i64) {
    let id = draw_contact_id(SEED, viewer, target);
    arena.contacts.insert(
        viewer,
        id,
        ShadowContact::sighted(
            target,
            Classification::Classified { party: team },
            at_ms,
            plot_of(arena, target),
        ),
    );
}

// ---------------------------------------------------------------------------
// (a) `Startup -> Patrol`
// ---------------------------------------------------------------------------

/// The cells one unit's machine is avoiding, without their stamps.
pub fn avoided(arena: &Arena, unit: ObjId) -> Vec<(u16, u16)> {
    arena
        .motors
        .row(unit)
        .map(|row| row.avoid.iter().map(|(cell, _)| *cell).collect())
        .unwrap_or_default()
}

/// How many times the black box says `unit` was stopped **at this cell** — the
/// lens doc 16 §3.4 step 4's *one notice per stop* is read through, and the one
/// that does not move with the substep partition the way a raw count does (16b
/// cut Q.6's `blocked` note).
pub fn blocked_at(events: &[LedgerEvent], unit: ObjId, cell: (u16, u16)) -> usize {
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
pub fn wreck_it(arena: &mut Arena, unit: ObjId) {
    if let Some(state) = arena.unit_mut(unit) {
        state.limits.wreck = true;
        state.limits.motion_disabled = true;
        state.power = Power::Off;
        state.speed_cms = 0;
        state.desired_speed_cms = 0;
    }
}

/// Take a tank's armour and internals down to 166 per mille, under the stock
/// `withdraw_permille` of 350 **and under 16b cut E.1's `stand_min_permille` of
/// 175**: a tank hurt this far breaks off and will not turn and fight anything on
/// the way home. See this file's head for why a test may do this.
pub fn hurt(arena: &mut Arena, unit: ObjId) {
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
pub fn hurt_but_fighting(arena: &mut Arena, unit: ObjId) {
    set_health(arena, unit, 2, 3);
}

/// One plan, one armour cell and one internals cell.
pub fn set_health(arena: &mut Arena, unit: ObjId, armor: i64, internals: i64) {
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
