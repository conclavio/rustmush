//! **The sensor pass, the shadow contact store and the comms mesh** (16b cut S.1;
//! doc 16 §3.6 "Perception", §3.2 "Line of sight"; D-16-10, W1, W4, W5, W6).
//!
//! Ten properties, driven through the arena library directly — no host, no store,
//! no task, which is what doc 16 §3.1's *the arena core is a library* is for:
//!
//! - **(a)** a target in the open at half a sensor's reach is detected on the first
//!   pass and classified after `classify_after_ms`;
//! - **(b)** a target behind a wall is not detected at any range;
//! - **(c)** a target past the heavy-woods stopped factor's reach is not detected,
//!   and the same target moving on a road is;
//! - **(d)** a light-woods belt in the line is a detection and a canopy is not,
//!   which is what `max_attenuation` buys;
//! - **(e)** a lost contact expires at the TTL and re-acquires under the **same**
//!   id;
//! - **(f)** the gap rule: a detection interrupted for longer than `pass_ms`
//!   restarts the classification clock;
//! - **(g)** the mesh reaches a teammate in range with line of sight and not one
//!   behind a wall;
//! - **(h)** the pass runs exactly once per `pass_ms` per unit, staggered by unit
//!   id;
//! - **(i)** the draw label of a classification is the pinned
//!   `classify:<observer>:<target>:<pass_seq>` and its value is `draw`'s;
//! - **(j)** a contact id is a function of the pair and the match seed: stable
//!   inside one match, different across seeds, and one platform's id resolves for
//!   nobody else.
//!
//! The cross-tempo property — one scripted approach replayed at 5,000, 1,000 and
//! 500 ms yielding one `detected`/`classified`/`lost` sequence — lives in
//! `crates/tacsim/tests/arena.rs` beside the rest of the rate-independence
//! property, because it is the *same* property and shares its replay driver.

use mush_atlas::TerrainClass;
use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use std::collections::BTreeMap;

use tacsim::arena::clock::Tempo;
use tacsim::arena::condition::{Condition, ConditionBand, Reading};
use tacsim::arena::intents::{Control, Intent, IntentKind};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::{
    FrameNotice, Notice, NoticeKind, PerceptionCounters, SensorProfile, SignatureFactor,
    bearing_mdeg,
};
use tacsim::arena::rng::{DrawLabel, draw, draw_contact_id, stagger_ms};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, CustodyOut, CustodyUnit,
    Designator, DueKind, EntryZone, Pilot, SeenPose, Team, TerrainCell, TrackedRules,
    WeaponSummary,
};
use tacsim::arena::step::Frame;
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::cockpit;

// ---------------------------------------------------------------------------
// Fixtures
//
// `tacsim::arena`'s own `test_support` is `pub(crate)` and invisible here, and the
// crate's panic budget forbids a shared helper under `src/`, so this file builds
// its own — the same shapes as `tests/arena.rs`', over a window wide enough to put
// a target at half the shipped sensor's 600 m reach.
// ---------------------------------------------------------------------------

/// The window: 80 by 80 cells of 1,000 cm, which is 800 m square — wide enough
/// that a 60-cell sensor has an outside.
const SIDE: u16 = 80;

/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;

/// The observer, the target, and a teammate of the observer.
const EYE: ObjId = ObjId(4);
const MARK: ObjId = ObjId(9);
const MATE: ObjId = ObjId(13);

/// A second teammate, inside `MATE`'s mesh and outside `EYE`'s (16b cut F.3's
/// relay case).
const RELAY: ObjId = ObjId(21);

/// A teammate that carries no sight at all (16b cut F.3's no-pass viewer).
const BLIND: ObjId = ObjId(27);

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

/// A pose at `(x_cm, y_cm)` facing north.
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

/// Passable ground at cost 100.
fn ground() -> TerrainCell {
    TerrainCell {
        class: TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// One painted cell of `class`, passable unless it is a wall.
fn cell(class: TerrainClass) -> TerrainCell {
    TerrainCell {
        class,
        elev_cm: 0,
        move_cost: if class == TerrainClass::Wall { 0 } else { 150 },
    }
}

/// **The shipped sensor's numbers** (`content/sensors/visual_mk1.json`), as the
/// arena carries them. Every test below starts from this and overrides the one
/// field it is about.
fn visual() -> SensorProfile {
    let row = |class: TerrainClass, moving: u32, stopped: u32| SignatureFactor {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    SensorProfile {
        component: smol_str::SmolStr::new("visual_sensor"),
        max_range_cells: 60,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: 30,
        signature: vec![
            row(TerrainClass::Road, 1_000, 1_000),
            row(TerrainClass::Ground, 1_000, 900),
            row(TerrainClass::Grass, 1_000, 800),
            row(TerrainClass::Rubble, 950, 700),
            row(TerrainClass::WoodsLight, 900, 650),
            row(TerrainClass::WoodsHeavy, 800, 500),
        ],
    }
}

/// The shipped cannon's row, for its one number: the derived
/// `tempo.alert_range_cm` is priced off the longest reach in the match, held to
/// the window's own diagonal since 16b cut E.1 (16b §9).
fn cannon() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![
            tacsim::arena::weapons::WeaponBand {
                max_range_cm: 150_000,
                dispersion_mrad: 3,
            },
            tacsim::arena::weapons::WeaponBand {
                max_range_cm: 250_000,
                dispersion_mrad: 6,
            },
        ],
        count_per_load: 40,
        // 16b cut W.2: the round's damage and the section it is mounted in.
        damage: 120,
        mount_section: SmolStr::new("turret"),
    }
}

/// A tank with `sensor` and the shipped cannon.
fn tank(id: ObjId, team: Team, pose: Pose, sensor: Option<SensorProfile>) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 200,
            brake_cms2: 400,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor,
        weapons: vec![cannon()],
        // 16b cut W.2: these fixtures present nothing, so every shot resolves at
        // zero — which is what keeps a perception test about perception.
        profile: tacsim::arena::resolution::UnitProfile::default(),
        gunnery: 0,
        controller: tacsim::arena::state::Controller::Pilot,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: tactical_unit(id),
    }
}

/// The 80 by 80 window with `paint`ed cells and one pilot per unit.
fn arena_of(units: Vec<CustodyUnit>, paint: &[(u16, u16, TerrainCell)]) -> Arena {
    arena_with_sensors(units, paint, &SensorsConfig::default())
}

/// The same, over a named `[tacsim.sensors]` block (16b cut Q.1's switch).
fn arena_with_sensors(
    units: Vec<CustodyUnit>,
    paint: &[(u16, u16, TerrainCell)],
    sensors: &SensorsConfig,
) -> Arena {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, painted) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *painted;
        }
    }
    let pilots: Vec<Pilot> = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: ArenaDefinition {
                id: SmolStr::new("tacsim.arena.perception"),
                terrain: ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the test window"),
                entry_zones: vec![
                    EntryZone {
                        team: Team::Red,
                        x0: 0,
                        y0: 0,
                        x1: 3,
                        y1: SIDE - 1,
                    },
                    EntryZone {
                        team: Team::Blue,
                        x0: SIDE - 4,
                        y0: 0,
                        x1: SIDE - 1,
                        y1: SIDE - 1,
                    },
                ],
                // 16b cut N.1: no authored furniture on this fixture window — the
                // reflex controller is not what these tests are about, and every unit
                // on them is a `Pilot`.
                key_spots: Vec::new(),
                waypoints: tacsim::arena::state::Waypoints::default(),
            },
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        sensors,
    )
    .expect("the perception arena opens")
}

/// **One pass, applied directly at `t_ms`** — the index rebuilt first, exactly as
/// phase 3(b) rebuilds it before phase 3(c) applies a due event.
///
/// Driving `sensor_pass` rather than `Arena::step` is deliberate for the
/// properties about *what is detected*: a pose and a speed can be put exactly
/// where the property needs them, and the pulse's own wiring — the due event, the
/// stagger, the frame, the tempo — is asserted by `(h)` below and by
/// `arena::step`'s own tests.
fn pass(
    arena: &mut Arena,
    observer: ObjId,
    pass_seq: u64,
    t_ms: i64,
) -> (PerceptionCounters, Vec<Notice>) {
    let mut counters = PerceptionCounters::default();
    let mut journal: Vec<Notice> = Vec::new();
    let units = arena.units.clone();
    arena.index.rebuild(&arena.definition.terrain, &units);
    // 16b cut U.2: and the jamming memo with it, exactly as phase 3(b) empties
    // it beside its own rebuild — a pass driven directly reads through the memo.
    arena.jam_fold.clear();
    let shares = arena.sensor_pass(observer, pass_seq, t_ms, &mut counters, &mut journal);
    if shares {
        arena.share_pass(observer, t_ms, &mut counters, &mut journal);
    }
    (counters, journal)
}

/// The kind words of a pass's journal, in order.
fn kinds(journal: &[Notice]) -> Vec<&'static str> {
    journal.iter().map(|(_, kind, _)| kind.as_str()).collect()
}

/// Put a unit exactly where a property needs it, and at what speed.
fn place(arena: &mut Arena, id: ObjId, at: Pose, speed_cms: i32) {
    if let Some(unit) = arena.unit_mut(id) {
        unit.pose = at;
        unit.speed_cms = speed_cms;
    }
}

// ---------------------------------------------------------------------------
// (a) the open field
// ---------------------------------------------------------------------------

#[test]
fn a_target_in_the_open_at_half_range_is_detected_and_then_classified() {
    // 30 cells apart on open ground: the sensor reaches 60 cells at factor 1,000
    // and `ground` stopped is 900, so 30,000 cm is comfortably inside 54,000.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    let (counters, journal) = pass(&mut arena, EYE, 1, 0);
    assert_eq!(counters.candidates, 1);
    assert_eq!(counters.traces, 1);
    assert_eq!(counters.detected, 1);
    assert_eq!(counters.classified, 0);
    assert_eq!(kinds(&journal), vec!["detected"]);

    let id = draw_contact_id(arena.seed, EYE, MARK);
    let held = arena
        .contacts
        .resolve(EYE, id)
        .copied()
        .unwrap_or_else(|| panic!("the contact is held under the minted id"));
    assert_eq!(held.target, MARK);
    assert_eq!(held.class, Classification::Unknown);
    assert_eq!((held.first_seen_ms, held.last_seen_ms), (0, 0));

    // Refreshed at each pass, and classified on the pass at or after
    // `classify_after_ms` of continuous detection — 3,000 ms, so the fourth pass.
    for (seq, t_ms) in [(2_u64, 1_000_i64), (3, 2_000)] {
        let (counters, journal) = pass(&mut arena, EYE, seq, t_ms);
        assert_eq!(counters.detected, 0, "the id was minted twice");
        assert_eq!(counters.classified, 0, "classified early at {t_ms}");
        assert!(journal.is_empty(), "{:?}", kinds(&journal));
    }
    let (counters, journal) = pass(&mut arena, EYE, 4, 3_000);
    assert_eq!(counters.classified, 1);
    assert_eq!(kinds(&journal), vec!["classified"]);
    let held = arena
        .contacts
        .resolve(EYE, id)
        .copied()
        .unwrap_or_else(|| panic!("still held"));
    assert_eq!(
        held.class,
        Classification::Classified { team: Team::Blue },
        "the viewer resolved the wrong side"
    );
    // And it does not classify twice.
    let (counters, _) = pass(&mut arena, EYE, 5, 4_000);
    assert_eq!(counters.classified, 0);

    // The frame the pilot is built carries exactly that one contact, at the
    // bearing and range doc 16 §3.6 names.
    let frame = arena.frame_contacts(EYE);
    assert_eq!(frame.len(), 1);
    let row = frame
        .first()
        .copied()
        .unwrap_or_else(|| panic!("one contact"));
    assert_eq!(row.id, id);
    assert_eq!(row.class, Classification::Classified { team: Team::Blue });
    assert_eq!(row.range_cm, 30_000);
    assert_eq!(
        row.bearing_mdeg,
        bearing_mdeg(&centre(10, 10), &centre(40, 10))
    );
    assert_eq!(row.bearing_mdeg, 90_000, "due east");
    // The target holds nothing: it carries no sensor, so it ran no pass.
    assert!(arena.frame_contacts(MARK).is_empty());
}

// ---------------------------------------------------------------------------
// (b) the wall
// ---------------------------------------------------------------------------

#[test]
fn a_target_behind_a_wall_is_never_detected() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        // One wall cell on the line, well inside the sensor's reach.
        &[(25, 10, cell(TerrainClass::Wall))],
    );
    let (counters, journal) = pass(&mut arena, EYE, 1, 0);
    // The index offered the candidate and the range test passed it: the trace is
    // what refused it, which is the point.
    assert_eq!(counters.candidates, 1);
    assert_eq!(counters.traces, 1);
    assert!(counters.cell_probes >= 30);
    assert_eq!(counters.detected, 0);
    assert!(journal.is_empty());
    assert!(arena.contacts.is_empty());
    // Not at any range: one cell short of the wall, and one cell past it.
    for (x, held) in [(24_u16, true), (26, false)] {
        place(&mut arena, MARK, centre(x, 10), 0);
        let (counters, _) = pass(&mut arena, EYE, 2, 1_000);
        assert_eq!(
            counters.detected,
            u32::from(held),
            "column {x} answered the wrong way"
        );
        arena.contacts.clear_viewer(EYE);
    }
}

// ---------------------------------------------------------------------------
// (c) the signature table
// ---------------------------------------------------------------------------

#[test]
fn heavy_woods_stopped_is_out_of_reach_and_a_road_moving_is_not() {
    // 40 cells out. Heavy woods stopped is 500 per mille, so the reach is 30,000 cm
    // and 40,000 is outside it; a road is 1,000 either way, so the same distance is
    // inside the sensor's full 60,000.
    for (class, speed_cms, detected) in [
        // Stopped in heavy woods: 500 per mille, 30,000 cm of reach, out of it.
        (TerrainClass::WoodsHeavy, 0_i32, false),
        // The same tank under way: 800 per mille, 48,000 cm, inside it. This pair
        // *is* doc 16 §3.6 step 3's sentence.
        (TerrainClass::WoodsHeavy, 700, true),
        // A road is 1,000 either way.
        (TerrainClass::Road, 700, true),
        (TerrainClass::Road, 0, true),
    ] {
        let mut arena = arena_of(
            vec![
                tank(EYE, Team::Red, centre(5, 10), Some(visual())),
                tank(MARK, Team::Blue, centre(45, 10), None),
            ],
            // Only the target's own cell is painted: the endpoints of a trace never
            // block their own line, so the line itself is open ground either way
            // and the *only* thing that differs is the signature row.
            &[(45, 10, cell(class))],
        );
        place(&mut arena, MARK, centre(45, 10), speed_cms);
        let (counters, _) = pass(&mut arena, EYE, 1, 0);
        assert_eq!(counters.candidates, 1, "{class:?}/{speed_cms}");
        assert_eq!(
            counters.detected,
            u32::from(detected),
            "{class:?} at {speed_cms} cm/s answered the wrong way"
        );
        // Heavy woods stopped never reaches the trace at all: the range test is the
        // cheap conjunct and it runs first (doc 16 §2 W4).
        assert_eq!(
            counters.traces,
            u32::from(detected),
            "{class:?}/{speed_cms}"
        );
    }
    // And the boundary is exactly where the row puts it: stopped in heavy woods is
    // held at 30,000 cm and not at 30,001.
    for (x_cm, detected) in [(35_000_i64, 1_u32), (35_001, 0)] {
        let mut arena = arena_of(
            vec![
                tank(EYE, Team::Red, centre(5, 10), Some(visual())),
                tank(MARK, Team::Blue, centre(45, 10), None),
            ],
            &[(35, 10, cell(TerrainClass::WoodsHeavy))],
        );
        // The observer stands at 5,500 cm, so 35,500 cm is 30,000 cm out; one
        // centimetre further is one centimetre past the 500-per-mille reach.
        place(&mut arena, MARK, pose(x_cm + 500, 10_500), 0);
        assert_eq!(
            pass(&mut arena, EYE, 1, 0).0.detected,
            detected,
            "the reach is not exactly 30,000 cm at {x_cm}"
        );
    }
}

// ---------------------------------------------------------------------------
// (d) attenuation
// ---------------------------------------------------------------------------

#[test]
fn a_belt_of_light_woods_is_a_detection_and_a_canopy_is_not() {
    // `max_attenuation` 40 against the atlas's own table: light woods cost 25 a
    // cell and heavy woods 60, so one light belt passes, two do not, and one heavy
    // cell does not.
    for (paint, detected) in [
        (vec![(20_u16, TerrainClass::WoodsLight)], true),
        (
            vec![
                (20, TerrainClass::WoodsLight),
                (21, TerrainClass::WoodsLight),
            ],
            false,
        ),
        (vec![(20, TerrainClass::WoodsHeavy)], false),
    ] {
        let painted: Vec<(u16, u16, TerrainCell)> = paint
            .iter()
            .map(|(x, class)| (*x, 10_u16, cell(*class)))
            .collect();
        let mut arena = arena_of(
            vec![
                tank(EYE, Team::Red, centre(10, 10), Some(visual())),
                tank(MARK, Team::Blue, centre(30, 10), None),
            ],
            &painted,
        );
        let (counters, _) = pass(&mut arena, EYE, 1, 0);
        assert_eq!(counters.traces, 1);
        assert_eq!(
            counters.detected,
            u32::from(detected),
            "{paint:?} answered the wrong way"
        );
    }
}

// ---------------------------------------------------------------------------
// (e) the time-to-live and the re-acquisition
// ---------------------------------------------------------------------------

#[test]
fn a_contact_expires_at_the_ttl_and_re_acquires_under_the_same_id() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    assert_eq!(arena.contact_ttl_ms(), 3_000, "1,000 ms x 3 passes");
    let id = draw_contact_id(arena.seed, EYE, MARK);
    pass(&mut arena, EYE, 1, 0);
    assert!(arena.contacts.resolve(EYE, id).is_some());

    // The target steps behind a wall — put one in the line rather than moving it,
    // so the *loss* and not the range is what is under test.
    let walled = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[(25, 10, cell(TerrainClass::Wall))],
    );
    arena.definition = walled.definition.clone();

    // Two passes inside the band keep it; the pass at the band loses it.
    for (seq, t_ms) in [(2_u64, 1_000_i64), (3, 2_000)] {
        let (counters, journal) = pass(&mut arena, EYE, seq, t_ms);
        assert_eq!(counters.lost, 0, "lost early at {t_ms}");
        assert!(journal.is_empty());
        assert!(arena.contacts.resolve(EYE, id).is_some());
    }
    let (counters, journal) = pass(&mut arena, EYE, 4, 3_000);
    assert_eq!(counters.lost, 1);
    assert_eq!(kinds(&journal), vec!["lost"]);
    assert!(
        arena.contacts.resolve(EYE, id).is_none(),
        "an expired id resolves, so the refusal would leak that it once existed"
    );
    assert!(arena.frame_contacts(EYE).is_empty());

    // The wall comes down: re-acquired, and under the same id, because the id is a
    // function of the pair and the match seed and of nothing that counts upward.
    let open = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    arena.definition = open.definition.clone();
    let (counters, journal) = pass(&mut arena, EYE, 5, 4_000);
    assert_eq!(counters.detected, 1);
    assert_eq!(kinds(&journal), vec!["detected"]);
    let held = arena
        .contacts
        .resolve(EYE, id)
        .copied()
        .unwrap_or_else(|| panic!("re-acquired under the same id"));
    assert_eq!(held.class, Classification::Unknown, "it starts over");
    assert_eq!((held.first_seen_ms, held.last_seen_ms), (4_000, 4_000));
}

// ---------------------------------------------------------------------------
// (f) the gap rule
// ---------------------------------------------------------------------------

#[test]
fn a_gap_longer_than_a_pass_restarts_the_classification_clock() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    let id = draw_contact_id(arena.seed, EYE, MARK);
    pass(&mut arena, EYE, 1, 0);
    // Two passes' worth of silence — inside the TTL (3,000 ms) so the contact is
    // still held, but a gap longer than `pass_ms` (1,000 ms). The clock restarts,
    // so a pass at 2,000 ms does not classify even though 2,000 ms has gone by.
    let (counters, _) = pass(&mut arena, EYE, 2, 2_500);
    assert_eq!(counters.classified, 0);
    let held = arena
        .contacts
        .resolve(EYE, id)
        .copied()
        .unwrap_or_else(|| panic!("still held"));
    assert_eq!(
        (held.first_seen_ms, held.last_seen_ms),
        (2_500, 2_500),
        "the gap did not restart the clock"
    );
    // Now three continuous passes from the restart.
    for (seq, t_ms) in [(3_u64, 3_500_i64), (4, 4_500)] {
        assert_eq!(pass(&mut arena, EYE, seq, t_ms).0.classified, 0);
    }
    assert_eq!(pass(&mut arena, EYE, 5, 5_500).0.classified, 1);
}

// ---------------------------------------------------------------------------
// (g) the comms mesh
// ---------------------------------------------------------------------------

#[test]
fn the_mesh_reaches_a_teammate_in_line_of_sight_and_not_one_behind_a_wall() {
    // A short-sighted teammate three cells from the observer: it cannot see the
    // target itself (five cells of reach against twenty-seven), so anything it
    // holds came over the mesh.
    let short = SensorProfile {
        max_range_cells: 5,
        ..visual()
    };
    // The teammate is three cells NORTH of the observer and the target thirty cells
    // east of it, so a wall between the two red tanks is not also a wall across the
    // observer's own line to the target.
    for (wall, shared) in [(None, true), (Some((10_u16, 11_u16)), false)] {
        let mut paint: Vec<(u16, u16, TerrainCell)> = Vec::new();
        if let Some((x, y)) = wall {
            paint.push((x, y, cell(TerrainClass::Wall)));
        }
        let mut arena = arena_of(
            vec![
                tank(EYE, Team::Red, centre(10, 10), Some(visual())),
                tank(MARK, Team::Blue, centre(40, 10), None),
                tank(MATE, Team::Red, centre(10, 13), Some(short.clone())),
            ],
            &paint,
        );
        let target_id = draw_contact_id(arena.seed, MATE, MARK);
        // Four passes carry the observer's own contact to `classified`, and every
        // one of them runs the share.
        for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
            pass(&mut arena, EYE, seq, t_ms);
        }
        assert_eq!(
            arena
                .contacts
                .resolve(EYE, draw_contact_id(arena.seed, EYE, MARK))
                .map(|held| held.class),
            Some(Classification::Classified { team: Team::Blue })
        );
        let held = arena.contacts.resolve(MATE, target_id).copied();
        assert_eq!(
            held.is_some(),
            shared,
            "a wall at {wall:?} answered the wrong way"
        );
        if let Some(held) = held {
            // What arrives is classified, because only classified contacts travel.
            assert_eq!(held.class, Classification::Classified { team: Team::Blue });
            assert_eq!(held.target, MARK);
            assert_eq!(held.first_seen_ms, 3_000, "the pass that classified it");
        }
        // The teammate's own pass cannot reach the target at all: five cells of
        // reach is a 7,500 cm cull, and the target is 27,000 cm away, so it is not
        // even a candidate. The only thing its own eyes find is the observer three
        // cells away — which is why anything it holds on the *target* came over the
        // mesh and nowhere else.
        let (counters, _) = pass(&mut arena, MATE, 1, 3_500);
        assert_eq!(
            counters.candidates, 1,
            "the short sensor's cull offered the wrong set"
        );
        assert_eq!(
            arena.contacts.resolve(MATE, target_id).is_some(),
            shared,
            "its own pass reached the target"
        );
        // And nothing was ever shared to the other side.
        assert!(
            arena
                .contacts
                .resolve(MARK, draw_contact_id(arena.seed, MARK, EYE))
                .is_none(),
            "the mesh crossed the line"
        );
    }
}

#[test]
fn the_mesh_refreshes_what_it_carries_so_a_shared_contact_outlives_the_ttl() {
    // Doc 16 §3.6 step 5 is a continuous datalink: a teammate keeps what the mesh
    // keeps telling it, and the receiver's own pass does not expire it.
    let short = SensorProfile {
        max_range_cells: 5,
        ..visual()
    };
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
            tank(MATE, Team::Red, centre(10, 13), Some(short)),
        ],
        &[],
    );
    let id = draw_contact_id(arena.seed, MATE, MARK);
    let mut t_ms = 0_i64;
    for seq in 1..=12_u64 {
        pass(&mut arena, EYE, seq, t_ms);
        // The teammate's own pass, after the share, is the one that would expire it.
        pass(&mut arena, MATE, seq, t_ms + 10);
        t_ms += 1_000;
    }
    assert!(
        arena.contacts.resolve(MATE, id).is_some(),
        "the mesh let go of a contact the sharer is still holding"
    );
    // Nine seconds past the classification and three times the TTL.
    assert_eq!(t_ms, 12_000);
}

// ---------------------------------------------------------------------------
// (h) the cadence and the stagger
// ---------------------------------------------------------------------------

#[test]
fn the_pass_runs_once_per_cadence_per_unit_staggered_by_unit_id() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), Some(visual())),
        ],
        &[],
    );
    // Doc 16 §3.6: staggered by unit id. **16b cut U.1** (16c `T-25`): the
    // offsets are `stagger_ms(seed, id, pass_ms)` — the object number mixed with
    // the match seed, where they were the number modulo `sensors.pass_ms` (4 ms
    // and 9 ms) — so `MARK`'s pass at 189 ms comes before `EYE`'s at 482 at seed
    // 2,350; and exactly one pass per unit is ever scheduled at a time.
    let cadence = i64::from(arena.sensors_cfg.pass_ms);
    let mut staggered = vec![
        (stagger_ms(arena.seed, EYE, cadence), EYE),
        (stagger_ms(arena.seed, MARK, cadence), MARK),
    ];
    staggered.sort_unstable();
    assert_ne!(
        staggered.first().map(|(t_ms, _)| *t_ms),
        staggered.get(1).map(|(t_ms, _)| *t_ms),
        "the precondition: two passes, at two milliseconds"
    );
    let scheduled: Vec<(i64, ObjId)> = arena
        .due
        .iter()
        .filter_map(|event| match event.kind {
            DueKind::SensorPass { unit, .. } => Some((event.t_ms, unit)),
            _ => None,
        })
        .collect();
    assert_eq!(scheduled, staggered);

    // Ten seconds of arena time, one pulse a second: ten passes each and not one
    // more, whatever the substep boundary the event landed on.
    let mut applied = 0_u32;
    let mut wall = 0_i64;
    arena.step(wall, &[], &[]);
    for _ in 0..10 {
        wall += 1_000;
        let out = arena.step(wall, &[], &[]);
        applied += out.perception.passes;
        let outstanding = arena
            .due
            .iter()
            .filter(|event| matches!(event.kind, DueKind::SensorPass { .. }))
            .count();
        assert_eq!(outstanding, 2, "one pass outstanding per unit, always");
    }
    assert_eq!(arena.now_ms, 10_000);
    assert_eq!(applied, 20, "two units, ten cadences, one pass each");
    // Both hold the other, classified, and the tempo rose for it.
    assert_eq!(arena.contacts.len(), 2);
    assert!(arena.contact_within_alert_range());
    // **16b cut E.1** (16c `T-32`) capped the derivation at the window's own
    // diagonal: twice the cannon's 2.5 km is 5 km, and the 80 by 80 window of
    // 1,000 cm cells is `isqrt(80² + 80²) = 113` cells across its longest line.
    assert_eq!(arena.alert_range_cm, 113_000, "the window's own diagonal");
}

// ---------------------------------------------------------------------------
// (i) the draw label
// ---------------------------------------------------------------------------

#[test]
fn a_classification_carries_the_pinned_draw_label_and_its_value() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    let (_, journal) = pass(&mut arena, EYE, 4, 3_000);
    let (_, kind, drawn) = journal
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("the classification"));
    assert_eq!(kind.as_str(), "classified");
    let label = DrawLabel::Classify {
        observer: EYE,
        target: MARK,
        pass_seq: 4,
    };
    assert_eq!(
        drawn,
        vec![(label.encode(), draw(arena.seed, &label))],
        "the draw is not the pinned label's"
    );
    assert_eq!(label.encode(), "classify:4:9:4");

    // And the same through the ledger, where the wire form is what a replay reads.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), Some(visual())),
        ],
        &[],
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    arena.step(wall, &[], &[]);
    for _ in 0..5 {
        wall += 1_000;
        for chunk in &arena.step(wall, &[], &[]).ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let log: Vec<LedgerEvent> = String::from_utf8(bytes)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect();
    let classified: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| event.kind.as_str() == "classified")
        .collect();
    assert_eq!(classified.len(), 2, "both sides classified: {log:?}");
    for event in classified {
        // 16b cut M.3: a list, and a sensor classification makes one draw.
        let [(label, value)] = event.draws.as_slice() else {
            panic!("a classification carries its one draw: {event:?}");
        };
        let (label, value) = (label.clone(), *value);
        assert!(label.starts_with("classify:"), "{label}");
        let fields: Vec<&str> = label.split(':').collect();
        assert_eq!(fields.len(), 4, "{label}");
        let rebuilt = DrawLabel::Classify {
            observer: ObjId(fields.get(1).and_then(|f| f.parse().ok()).unwrap_or(0)),
            target: ObjId(fields.get(2).and_then(|f| f.parse().ok()).unwrap_or(0)),
            pass_seq: fields.get(3).and_then(|f| f.parse().ok()).unwrap_or(0),
        };
        assert_eq!(rebuilt.encode(), label);
        assert_eq!(value, draw(arena.seed, &rebuilt));
    }
    // The three kinds A.1 reserved carry their fields, and nothing else changed
    // shape: every line still parses as a `LedgerEvent`.
    let detected: Vec<&EventKind> = log
        .iter()
        .map(|event| &event.kind)
        .filter(|kind| kind.as_str() == "detected")
        .collect();
    assert_eq!(detected.len(), 2);
    assert!(detected.iter().any(|kind| matches!(
        kind,
        EventKind::Detected {
            viewer: EYE,
            target: MARK,
            ..
        }
    )));
}

// ---------------------------------------------------------------------------
// (j) the contact identity space
// ---------------------------------------------------------------------------

#[test]
fn a_contact_id_is_the_pair_and_the_seed_and_resolves_for_nobody_else() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), Some(visual())),
        ],
        &[],
    );
    pass(&mut arena, EYE, 1, 0);
    pass(&mut arena, MARK, 1, 0);
    let mine = draw_contact_id(arena.seed, EYE, MARK);
    let theirs = draw_contact_id(arena.seed, MARK, EYE);
    assert_ne!(mine, theirs, "the pair is ordered");
    assert!(arena.contacts.resolve(EYE, mine).is_some());
    assert!(arena.contacts.resolve(MARK, theirs).is_some());
    // **The one refusal.** An id minted for another platform, an id nothing ever
    // minted, and an id this platform has let expire are one answer: `None`, which
    // the cockpit renders as the single line doc 16 §3.6 demands.
    assert!(arena.contacts.resolve(EYE, theirs).is_none());
    assert!(arena.contacts.resolve(MARK, mine).is_none());
    assert!(arena.contacts.resolve(EYE, ContactId(1)).is_none());
    assert!(arena.contacts.resolve(ObjId(999), mine).is_none());
    assert_eq!(
        tacsim::arena::perception::contact_refusal(),
        tacsim::match_::NO_SUCH_CONTACT
    );

    // Stable inside one match, however many times it is asked.
    assert_eq!(mine, draw_contact_id(arena.seed, EYE, MARK));
    assert_eq!(
        arena.frame_contacts(EYE).first().map(|contact| contact.id),
        Some(mine)
    );

    // And different across seeds: two matches over the same units never share an
    // id, so a ledger replayed against the wrong match resolves nothing.
    let mut other = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), Some(visual())),
        ],
        &[],
    );
    other.seed = 9_001;
    pass(&mut other, EYE, 1, 0);
    let elsewhere = draw_contact_id(other.seed, EYE, MARK);
    assert_ne!(mine, elsewhere);
    assert!(other.contacts.resolve(EYE, elsewhere).is_some());
    assert!(other.contacts.resolve(EYE, mine).is_none());
}

// ---------------------------------------------------------------------------
// (k) what a pilot sees — 16b cut S.2's frame rendering
//
// The rows, the bands, the two bearings and the map markers are read through
// `tacsim::match_::cockpit`'s pure renderers, driven off frames `Arena::step`
// actually produced. Nothing here needs a host: the cockpit's rendering half takes
// a `Frame` and answers, which is what makes it testable beside the pass that
// fills one.
// ---------------------------------------------------------------------------

/// One wall millisecond per arena millisecond, a pulse a second: the cadence that
/// puts each of `EYE`'s passes (its stagger, then a second after each; 482,
/// 1,482, 2,482, … at seed 2,350 since 16b cut U.1) in a pulse of its own, so a
/// test can say *which pulse* a notice arrived in.
const PULSE_MS: i64 = 1_000;

/// Step the arena to `wall_ms`, spool whatever ledger chunks closed onto `tape`,
/// and hand back the frame the first pilot was sent when their view changed.
fn pulse(arena: &mut Arena, wall_ms: i64, tape: &mut Vec<u8>) -> Option<Frame> {
    let out = arena.step(wall_ms, &[], &[]);
    for chunk in &out.ledger_chunks {
        tape.extend_from_slice(&chunk.bytes);
    }
    out.frames
        .into_iter()
        .find(|(pilot, _)| *pilot == ObjId(1_000))
        .map(|(_, frame)| frame)
}

/// Every event on `tape` plus whatever the ledger is still holding, parsed.
fn events(arena: &mut Arena, tape: &mut Vec<u8>) -> Vec<LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        tape.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(tape.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

/// The arena millisecond of every event of one kind, in order.
fn event_times(arena: &mut Arena, tape: &mut Vec<u8>, kind: &str) -> Vec<i64> {
    events(arena, tape)
        .iter()
        .filter(|event| event.kind.as_str() == kind)
        .map(|event| event.t_ms)
        .collect()
}

/// The notice words of one frame, in order.
fn told(frame: &Frame) -> Vec<&'static str> {
    frame
        .notices
        .iter()
        .map(|notice| notice.kind().as_str())
        .collect()
}

/// `EYE`'s own pass offset at the fixture's seed and the default one-second
/// cadence (16b cut U.1: `stagger_ms`, 482 ms), asserted strictly inside the
/// first pulse so every story below that is told in whole-second pulses still
/// puts each pass in a pulse of its own.
fn eye_offset(arena: &Arena) -> i64 {
    let offset = stagger_ms(arena.seed, EYE, i64::from(arena.sensors_cfg.pass_ms));
    assert!(
        offset > 0 && offset < PULSE_MS,
        "the precondition: EYE's pass is inside its pulse, not on its edge ({offset})"
    );
    offset
}

/// A two-tank arena with `EYE` looking and `MARK` thirty cells east, opened and
/// stepped past its reference pulse.
fn watching() -> Arena {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    // The opening pulse establishes the wall reference and integrates nothing.
    let _ = arena.step(0, &[], &[]);
    arena
}

#[test]
fn the_frame_renders_a_contact_as_a_designator_a_band_and_two_bearings() {
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse sends a frame");
    assert_eq!(told(&frame), vec!["acquired"], "one notice, once");

    let rows = cockpit::frame_contacts(&frame);
    assert_eq!(rows.len(), 1, "the platform holds one return");
    let row = rows.first().cloned().expect("one row");
    let id = draw_contact_id(arena.seed, EYE, MARK);
    assert_eq!(row.id, id, "the opaque id is the arena's own");
    // **16b cut Q.1**: the row's own name is the short designator, and while the
    // return is unknown that is this platform's first provisional number. No
    // alias: the number *is* the designator.
    assert_eq!(row.designator, Designator::Provisional(1));
    assert_eq!(row.alias, None);
    assert_eq!(row.class.as_str(), "unknown", "not told apart yet");
    assert!(!row.friendly, "an unknown return is nobody's");
    // Due east of a platform facing north: 090 absolute, 090 relative.
    assert_eq!(row.bearing_deg, 90);
    assert_eq!(row.relative_deg, 90);
    // **16b cut D.1**: the pilot's word is thirds of the LESSER of the sight and
    // the fired weapon's reach. The fixture sight is 60 cells of 1,000 cm, so the
    // limit is 60,000 cm and 30,000 cm is the middle third.
    assert_eq!(row.band.as_str(), "effective");
    assert_eq!(row.cell, Some((40, 10)), "the marker's cell");
    assert_eq!(
        row.age_ms,
        arena.now_ms - eye_offset(&arena),
        "held since its first pass"
    );
    // **16b cut Q.1**: the line names the designator where it used to name the
    // sixteen digits, and the digits are not in it at all.
    assert!(
        row.render().starts_with("X1  "),
        "the line names the designator: {}",
        row.render()
    );
    assert!(
        !row.render().contains(&id.to_string()),
        "and never the sixteen digits: {}",
        row.render()
    );
    // No dbref of any shape: the id is the only handle, and a `#` in this line
    // could only be an object number.
    assert!(
        !row.render().contains('#'),
        "and never the target's object number: {}",
        row.render()
    );

    // The relative bearing is off the platform's own nose, and the absolute one is
    // not: turning the tank moves one and leaves the other.
    let mut turned = frame.clone();
    turned.pose.heading_mdeg = 90_000;
    let turned_row = cockpit::frame_contacts(&turned)
        .first()
        .cloned()
        .expect("one row");
    assert_eq!(turned_row.bearing_deg, 90);
    assert_eq!(turned_row.relative_deg, 0, "dead ahead");

    // Classification arrives at `classify_after_ms`, and the class word becomes the
    // side rather than the state.
    let mut seen: Vec<&'static str> = Vec::new();
    let mut classified = None;
    for n in 2..=5 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            seen.extend(told(&frame));
            if cockpit::frame_contacts(&frame)
                .first()
                .is_some_and(|row| row.class.as_str() == "blue")
            {
                classified = Some(frame);
            }
        }
    }
    assert_eq!(seen, vec!["classified"], "one notice per transition");
    let frame = classified.expect("the return is told apart");
    let row = cockpit::frame_contacts(&frame)
        .first()
        .cloned()
        .expect("one row");
    assert_eq!(row.class.as_str(), "blue", "the side, not the state");
    assert!(!row.friendly, "a red platform's blue contact is hostile");
    assert_eq!(row.glyph(), cockpit::HOSTILE_GLYPH);
}

#[test]
fn a_range_band_is_read_against_the_fired_weapons_own_bands() {
    // **16b cut F.3.** Cut S.2 banded in thirds of the platform's longest reach,
    // which for the shipped tank is the cannon's 3 km — so `close` covered
    // everything inside 1 km and the shipped 600 m sight could never produce any
    // other word. The bands are the fired weapon's own, at their boundaries.
    //
    // **16b cut E.1** deleted `cockpit::range_band`, the commander's second
    // distance (16c `T-81`), and left `contact_band` as the one word. This test
    // asks it at `sight_cm = 0` — a hull with no scope, where the limit is the
    // weapon's own last band — which is the reading closest to what the deleted
    // function answered, so what stays pinned here is the boundary arithmetic
    // and the `beyond` rules. `contact_band`'s own thirds are pinned by
    // `cockpit.rs`'s `d1_contact_band_is_thirds_of_the_lesser_of_the_sight_and_the_reach`
    // and its inverse by `order.rs`'s `the_bands_agree_with_the_cockpits_own_words`.
    let bands = [300_u32, 600];
    assert_eq!(cockpit::contact_band(0, &bands, 0), "close");
    assert_eq!(cockpit::contact_band(200, &bands, 0), "close");
    assert_eq!(cockpit::contact_band(201, &bands, 0), "effective");
    assert_eq!(cockpit::contact_band(400, &bands, 0), "effective");
    assert_eq!(cockpit::contact_band(401, &bands, 0), "far");
    assert_eq!(cockpit::contact_band(600, &bands, 0), "far");
    assert_eq!(cockpit::contact_band(601, &bands, 0), "beyond");
    // A platform with nothing to shoot with has nothing in range.
    assert_eq!(cockpit::contact_band(0, &[], 0), "beyond");
    assert_eq!(cockpit::contact_band(u64::MAX, &[u32::MAX], 0), "beyond");
    // One band is a legal weapon: everything past its edge is `beyond`.
    assert_eq!(cockpit::contact_band(100, &[300], 0), "close");
    assert_eq!(cockpit::contact_band(200, &[300], 0), "effective");
    assert_eq!(cockpit::contact_band(301, &[300], 0), "beyond");
}

#[test]
fn a_contact_row_reports_its_range_in_metres_rounded_to_fifty() {
    // 16b cut F.3: the one named constant, and half a step rounds up.
    //
    // **16b cut D.1** (`B-19`) floored the answer at one step: everything inside
    // half a step used to print `0 m`, and `0 m` is the one distance a contact
    // cannot be at.
    assert_eq!(cockpit::CONTACT_RANGE_STEP_M, 50);
    for (range_cm, metres) in [
        (0_u64, 50_u64),
        (2_400, 50),
        (2_500, 50),
        (3_000, 50),
        (7_400, 50),
        (7_500, 100),
        (30_000, 300),
        (32_499, 300),
        (32_500, 350),
        (60_000, 600),
    ] {
        assert_eq!(
            // 16b cut Q.1 made the step an argument: a contact row still reads
            // fifty, and a lock reads ten.
            cockpit::range_m_rounded(range_cm, cockpit::CONTACT_RANGE_STEP_M),
            metres,
            "{range_cm} cm rounded the wrong way"
        );
    }
}

#[test]
fn a_teammate_is_a_friendly_marker_and_an_unknown_return_is_neither() {
    // `MATE` carries a sensor too, so the mesh has somebody to talk to; both are
    // red, so `EYE`'s classified contact on `MATE` is its own side.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(20, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    let _ = arena.step(0, &[], &[]);
    let mut tape: Vec<u8> = Vec::new();
    let mut last = None;
    for n in 1..=6 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            last = Some(frame);
        }
    }
    let frame = last.expect("the platform is sent frames");
    let rows = cockpit::frame_contacts(&frame);
    assert_eq!(rows.len(), 2, "the teammate and the mark: {rows:?}");
    let friendly = rows
        .iter()
        .find(|row| row.friendly)
        .expect("the teammate classifies as this platform's own side");
    assert_eq!(friendly.class.as_str(), "red");
    assert_eq!(friendly.glyph(), cockpit::FRIEND_GLYPH);
    let other = rows
        .iter()
        .find(|row| !row.friendly)
        .expect("and the mark does not");
    assert!(matches!(other.class.as_str(), "blue" | "unknown"));
    if other.class.as_str() == "unknown" {
        assert_eq!(other.glyph(), cockpit::UNKNOWN_GLYPH);
    }
    // Three glyphs, three meanings, no overlap.
    assert_ne!(cockpit::UNKNOWN_GLYPH, cockpit::FRIEND_GLYPH);
    assert_ne!(cockpit::FRIEND_GLYPH, cockpit::HOSTILE_GLYPH);
    assert_ne!(cockpit::UNKNOWN_GLYPH, cockpit::YOU_GLYPH);
}

// ---------------------------------------------------------------------------
// (l) the notice hysteresis — 16b cut S.2
//
// The rule, in one sentence: a `lost` notice is held for one further pass, and a
// re-acquisition inside that window cancels it and is itself not announced. The
// ledger's `lost` event is unaffected, which is what these two tests assert
// together — the black box always records the loss, and the pilot is told about it
// only when it was a real one.
// ---------------------------------------------------------------------------

/// Where a target goes to be out of every reach: the far corner of the 80-cell
/// window, 97,000 cm from `EYE`, well past the 54,000 cm a full-signature sensor
/// reaches on stopped ground.
fn away() -> Pose {
    centre(SIDE - 1, SIDE - 1)
}

#[test]
fn a_loss_is_announced_one_pass_late_and_the_ledger_records_it_at_once() {
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse");
    assert_eq!(told(&frame), vec!["acquired"]);
    place(&mut arena, MARK, away(), 0);

    // The pass at 3,000 and `EYE`'s offset expires the contact the pass at the
    // offset minted: three passes at the shipped `lost_after_passes`.
    let mut words: Vec<(i64, &'static str)> = Vec::new();
    for n in 2..=6 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            for word in told(&frame) {
                words.push((arena.now_ms, word));
            }
        }
    }
    assert_eq!(
        words,
        vec![(5_000, "lost")],
        "the loss is said once, in the pulse after the one that expired it"
    );

    // And the black box wrote it at the moment of expiry, three passes in, not when
    // the pilot was told.
    assert_eq!(
        event_times(&mut arena, &mut tape, "lost"),
        vec![3_000 + eye_offset(&arena)],
        "the ledger's own timestamp"
    );
}

#[test]
fn a_contact_that_flickers_costs_the_pilot_no_lines_at_all() {
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse");
    assert_eq!(told(&frame), vec!["acquired"]);
    place(&mut arena, MARK, away(), 0);

    // Out of reach through the pass that expires the contact (3,000 and `EYE`'s
    // offset) — and back before the next one a second later, which is the window
    // the rule names.
    let mut words: Vec<&'static str> = Vec::new();
    for n in 2..=7 {
        if arena.now_ms >= 3_500 {
            place(&mut arena, MARK, centre(40, 10), 0);
        }
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            words.extend(told(&frame));
        }
    }
    assert!(
        words.is_empty(),
        "a contact lost and re-acquired inside one pass is announced neither way: {words:?}"
    );
    // It is held again, under the same id, and the ledger has both halves.
    let id = draw_contact_id(arena.seed, EYE, MARK);
    assert!(arena.contacts.resolve(EYE, id).is_some());
    let offset = eye_offset(&arena);
    assert_eq!(
        event_times(&mut arena, &mut tape, "lost"),
        vec![3_000 + offset]
    );
    assert_eq!(
        event_times(&mut arena, &mut tape, "detected"),
        vec![offset, 4_000 + offset],
        "the record is complete even though the pilot heard nothing about the flicker"
    );
}

#[test]
fn a_notice_is_one_sentence_wherever_it_is_rendered() {
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse");
    let lines = cockpit::notice_lines(&frame);
    let id = draw_contact_id(arena.seed, EYE, MARK);
    assert_eq!(lines.len(), 1);
    let line = lines.first().cloned().unwrap_or_default();
    // **16b cut Q.1**: the sentence names the short designator and never the
    // sixteen digits. The contact is still unknown on the acquiring pulse, so it
    // is this platform's first provisional number.
    assert!(line.contains("X1"), "{line}");
    assert!(
        !line.contains(&id.to_string()),
        "the sixteen digits are not a pilot's word any more: {line}"
    );
    // 16b cut D.1: the acquisition notice quotes the same word `CONTACTS` does,
    // which is now `contact_band`'s.
    assert!(
        line.contains("effective"),
        "the band the notice quotes: {line}"
    );
    assert!(
        !line.contains(&format!("#{}", MARK.0)),
        "and never the target: {line}"
    );
    // The same renderer answers the frame's own `notices` array, so a panel and a
    // line client read one sentence.
    for notice in &frame.notices {
        assert_eq!(
            cockpit::notice_line(&frame, notice, &cockpit::frame_contacts(&frame)),
            line
        );
    }
    // A `lost` notice names the designator and nothing else: the contact is not
    // in the frame's list any more, so there is no bearing to quote — and (16b
    // cut Q.1) the name it read as was captured at the expiry, because the row
    // it could have been read off is gone.
    let gone = FrameNotice::Contact {
        kind: NoticeKind::Lost,
        contact: tacsim::arena::perception::Named {
            id,
            designator: Designator::Provisional(1),
        },
        class: Classification::Unknown,
        was: None,
    };
    assert_eq!(cockpit::notice_line(&frame, &gone, &[]), "Contact X1 lost.");

    // **And one notice renders to nothing** (16b cut Q.1). A `classified` whose
    // contact was never provisional is a mesh arrival, whose acquisition line has
    // already read `New contact BA: …` — there is no rename to say, so the
    // sentence is empty. An empty sentence owes the pilot nothing, which is why
    // it is dropped from `notice_lines` **and** from the panel's own array: a
    // renderer drawing that array would otherwise draw a blank row. The
    // classification is still there to be read off the contact's class.
    let named = tacsim::arena::perception::Named {
        id,
        designator: Designator::Slot {
            team: Team::Blue,
            slot: 1,
        },
    };
    let silent = FrameNotice::Contact {
        kind: NoticeKind::Classified,
        contact: named,
        class: Classification::Classified { team: Team::Blue },
        was: None,
    };
    assert_eq!(cockpit::notice_line(&frame, &silent, &[]), "");
    let mut carrying = frame.clone();
    carrying.notices = vec![silent];
    assert!(
        cockpit::notice_lines(&carrying).is_empty(),
        "a notice with no sentence was said anyway"
    );
    let payload = cockpit::status_frame(tacsim::arena_task::MatchId(1), &carrying);
    assert_eq!(
        payload
            .get("notices")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0),
        "a notice with no sentence reached a panel as a blank row: {payload}"
    );
    // The same notice with a name to quote is one sentence, said and carried.
    carrying.notices = vec![FrameNotice::Contact {
        kind: NoticeKind::Classified,
        contact: named,
        class: Classification::Classified { team: Team::Blue },
        was: Some(Designator::Provisional(1)),
    }];
    assert_eq!(
        cockpit::notice_lines(&carrying),
        vec!["Contact X1 identified: BA, blue.".to_owned()]
    );
    let payload = cockpit::status_frame(tacsim::arena_task::MatchId(1), &carrying);
    assert_eq!(
        payload
            .get("notices")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{payload}"
    );
    // **And a renamed row's `alias` is spelled the way a pilot would type it**
    // (16b cut Q.1): `X1`, not the bare number — it is the other word `LOCK`
    // accepts, so a panel that offered `1` would be offering something the verb
    // refuses.
    for contact in &mut carrying.contacts {
        contact.designator = Designator::Slot {
            team: Team::Blue,
            slot: 1,
        };
        contact.alias = Some(1);
    }
    let payload = cockpit::status_frame(tacsim::arena_task::MatchId(1), &carrying);
    let row = payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| rows.first())
        .unwrap_or_else(|| panic!("the one row: {payload}"));
    assert_eq!(
        row.get("designator").and_then(serde_json::Value::as_str),
        Some("BA")
    );
    assert_eq!(
        row.get("alias").and_then(serde_json::Value::as_str),
        Some("X1"),
        "the alias is the word LOCK takes and not the number behind it: {payload}"
    );
}

// ---------------------------------------------------------------------------
// (m) what a platform knows — 16b cut F.3
//
// Two defects of the cut S store, and one usability decision:
//
// - **(m1)** the mesh had no provenance, so two teammates refreshed each other's
//   copy of a target for ever and a contact never expired;
// - **(m2)** a held contact was reported at the target's **live** pose, so a tank
//   that broke line of sight was tracked through it for the whole time to live;
// - **(m3)** the range band was thirds of the platform's longest weapon reach
//   against a sight that cannot see a third of it, so every row read `close`.
// ---------------------------------------------------------------------------

/// The shipped sight with a five-cell reach: a teammate that can talk over the
/// mesh (`share_range_cells` is untouched at 30) and cannot see the far side of
/// the window for itself.
fn short_sight() -> SensorProfile {
    SensorProfile {
        max_range_cells: 5,
        ..visual()
    }
}

/// `EYE` and `MATE` three cells apart with a mesh line between them, both holding
/// blue `MARK` thirty cells east of `EYE`.
fn meshed_pair() -> Arena {
    arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(10, 13), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    )
}

/// Run one pass for `EYE` and one for `MATE` at `t_ms`, and answer every journal
/// kind the two produced, in order.
fn mesh_round(arena: &mut Arena, seq: u64, t_ms: i64) -> Vec<&'static str> {
    let (_, mine) = pass(arena, EYE, seq, t_ms);
    let (_, theirs) = pass(arena, MATE, seq, t_ms + 10);
    let mut words = kinds(&mine);
    words.extend(kinds(&theirs));
    words
}

/// A wall down the whole of column `x`: a curtain a target can drive behind and
/// stay behind, rather than the one cell the time-to-live test puts on one line.
fn curtain(x: u16) -> Vec<(u16, u16, TerrainCell)> {
    (0..SIDE)
        .map(|y| (x, y, cell(TerrainClass::Wall)))
        .collect()
}

/// Whether `viewer` still holds a contact on `MARK`.
fn holds_mark(arena: &Arena, viewer: ObjId) -> bool {
    arena
        .contacts
        .resolve(viewer, draw_contact_id(arena.seed, viewer, MARK))
        .is_some()
}

#[test]
fn two_meshed_teammates_let_go_of_a_target_that_leaves_everyones_reach() {
    // **(m1), the regression.** Before 16b cut F.3 this loop had no exit: each
    // pass raised the other platform's `last_seen_ms` to *now*, so the cutoff was
    // never reached on either side and a `lost` was never written at all.
    let mut arena = meshed_pair();
    // Four rounds carry both platforms' own contacts to `classified`.
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        mesh_round(&mut arena, seq, t_ms);
    }
    assert!(holds_mark(&arena, EYE) && holds_mark(&arena, MATE));
    // The last real sighting is `MATE`'s, at 3,010 ms. The target then leaves
    // every reach.
    place(&mut arena, MARK, away(), 0);
    let mut lost_at: Vec<(ObjId, i64)> = Vec::new();
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000), (8, 7_000)] {
        let (_, mine) = pass(&mut arena, EYE, seq, t_ms);
        if kinds(&mine).contains(&"lost") {
            lost_at.push((EYE, t_ms));
        }
        let (_, theirs) = pass(&mut arena, MATE, seq, t_ms + 10);
        if kinds(&theirs).contains(&"lost") {
            lost_at.push((MATE, t_ms + 10));
        }
    }
    assert!(!holds_mark(&arena, EYE), "the loop kept EYE's copy alive");
    assert!(!holds_mark(&arena, MATE), "the loop kept MATE's copy alive");
    // And both losses fell inside the time to live of the last real sighting,
    // which is 3,010 ms (`MATE`'s own, the later of the two): the contact is not
    // refreshed past 6,010 ms, and the `lost` is written at the first pass at or
    // after that, which is the same rule an ordinary loss has always followed.
    let ttl = arena.contact_ttl_ms();
    let cadence = 1_000_i64;
    for (viewer, at_ms) in &lost_at {
        assert!(
            *at_ms <= 3_010 + ttl + cadence,
            "{viewer:?} let go at {at_ms}, past the TTL of the last sighting"
        );
    }
    assert_eq!(lost_at.len(), 2, "one loss each: {lost_at:?}");
}

#[test]
fn two_meshed_teammates_let_go_of_a_target_that_is_destroyed() {
    // The same loop, with the target a wreck rather than gone: `sees_wrecks` is
    // false, so neither platform's own pass finds it again, and the mesh has
    // nothing of its own left to carry.
    let mut arena = meshed_pair();
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        mesh_round(&mut arena, seq, t_ms);
    }
    assert!(holds_mark(&arena, EYE) && holds_mark(&arena, MATE));
    if let Some(state) = arena.unit_mut(MARK) {
        state.limits.wreck = true;
    }
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000), (8, 7_000)] {
        mesh_round(&mut arena, seq, t_ms);
    }
    assert!(
        !holds_mark(&arena, EYE) && !holds_mark(&arena, MATE),
        "a classified contact outlived the hull it was on"
    );
}

#[test]
fn a_contact_that_only_ever_arrived_over_the_mesh_is_never_shared_on() {
    // **Doc 16 §2 W6**: shared contacts fan out only within comms range *of the
    // platform that sees*. `EYE` sees `MARK`; `MATE` is inside `EYE`'s share reach
    // and holds `MARK` only over the mesh; `RELAY` is inside `MATE`'s reach and
    // outside `EYE`'s, and must never learn of `MARK`.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(10, 13), Some(short_sight())),
            tank(RELAY, Team::Red, centre(10, 43), Some(short_sight())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    for (seq, t_ms) in [
        (1_u64, 0_i64),
        (2, 1_000),
        (3, 2_000),
        (4, 3_000),
        (5, 4_000),
        (6, 5_000),
    ] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms + 10);
        pass(&mut arena, RELAY, seq, t_ms + 20);
    }
    // The first hop happened.
    assert!(holds_mark(&arena, MATE), "the mesh's first hop never ran");
    // The second did not.
    assert!(
        !holds_mark(&arena, RELAY),
        "a mesh copy was relayed on: the contact fanned out past the platform that sees"
    );
    // And the relay is genuinely inside the mesh: `MATE` shares what its **own**
    // five-cell sight holds — `EYE`, three cells away — and that reaches it.
    assert!(
        arena
            .contacts
            .resolve(RELAY, draw_contact_id(arena.seed, RELAY, EYE))
            .is_some(),
        "the relay is outside MATE's mesh, so the property above proves nothing"
    );
}

#[test]
fn a_share_never_raises_a_receivers_stamp_above_the_sharers_own_sighting() {
    let mut arena = meshed_pair();
    // `MATE` cannot see `MARK` for itself, so whatever it holds is the mesh's.
    if let Some(state) = arena.unit_mut(MATE) {
        state.sensor = Some(short_sight());
    }
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms + 10);
    }
    let theirs = draw_contact_id(arena.seed, MATE, MARK);
    let held = arena
        .contacts
        .resolve(MATE, theirs)
        .copied()
        .unwrap_or_else(|| panic!("the mesh reached the teammate"));
    assert_eq!(held.last_seen_ms, 3_000, "the sharer's own sighting");
    assert_eq!(held.own_seen, None, "nothing of its own saw it");

    // `MARK` steps behind a wall: `EYE` stops seeing it but still holds it, so the
    // mesh keeps carrying it (cut S's deviation (3)) — and carries the **old**
    // sighting, not the moment of transmission.
    let walled = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(10, 13), Some(short_sight())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[(25, 10, cell(TerrainClass::Wall))],
    );
    arena.definition = walled.definition.clone();
    pass(&mut arena, EYE, 5, 4_000);
    pass(&mut arena, MATE, 5, 4_010);
    let held = arena
        .contacts
        .resolve(MATE, theirs)
        .copied()
        .unwrap_or_else(|| panic!("still held one pass on"));
    assert_eq!(
        held.last_seen_ms, 3_000,
        "the share raised the receiver's stamp to the moment of transmission"
    );
    // Which is what makes it expire on time: three passes past 3,000 ms and no
    // sighting behind it.
    pass(&mut arena, EYE, 6, 5_000);
    pass(&mut arena, MATE, 6, 5_010);
    pass(&mut arena, EYE, 7, 6_000);
    let (_, journal) = pass(&mut arena, MATE, 7, 6_010);
    assert_eq!(kinds(&journal), vec!["lost"]);
    assert!(arena.contacts.resolve(MATE, theirs).is_none());
}

#[test]
fn a_viewer_that_runs_no_pass_of_its_own_still_expires_what_was_shared_to_it() {
    // A unit with no sight at all: content can field one (a logistics or command
    // variant), and before 16b cut F.3 it was never scheduled a pass, so the only
    // expiry in the engine never ran for it and a teammate's shares piled up for
    // the whole match.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(BLIND, Team::Red, centre(10, 13), None),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    // It has a pass of its own on the schedule, which is the whole fix.
    assert!(
        arena.due.iter().any(|event| matches!(
            event.kind,
            DueKind::SensorPass { unit, .. } if unit == BLIND
        )),
        "a platform that holds contacts has no pass at which to expire them"
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, BLIND, seq, t_ms + 10);
    }
    let theirs = draw_contact_id(arena.seed, BLIND, MARK);
    assert!(
        arena.contacts.resolve(BLIND, theirs).is_some(),
        "the mesh never reached the sightless teammate"
    );
    // The sharer loses its own sighting, so nothing refreshes the copy.
    place(&mut arena, MARK, away(), 0);
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000), (8, 7_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, BLIND, seq, t_ms + 10);
    }
    assert!(
        arena.contacts.resolve(BLIND, theirs).is_none(),
        "the sightless viewer kept a contact nobody is reporting"
    );
}

#[test]
fn a_held_contact_is_reported_where_it_was_last_seen_and_ages() {
    // **(m2), the regression.** Before 16b cut F.3 the frame read the target's
    // live pose for the whole time to live, so a tank that ducked behind a
    // building was tracked through it, cell by cell.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    pass(&mut arena, EYE, 1, 0);
    let row = arena
        .frame_contacts(EYE)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("one contact"));
    assert_eq!(row.cell, Some((40, 10)));
    assert_eq!(row.range_cm, 30_000);
    assert_eq!(row.age_ms, 0, "just seen");

    // The wall goes up and the target drives away behind it.
    let walled = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &curtain(25),
    );
    arena.definition = walled.definition.clone();
    for (seq, t_ms, x) in [(2_u64, 1_000_i64, 45_u16), (3, 2_000, 50)] {
        place(&mut arena, MARK, centre(x, 10), 1_000);
        arena.now_ms = t_ms;
        pass(&mut arena, EYE, seq, t_ms);
        let row = arena
            .frame_contacts(EYE)
            .first()
            .copied()
            .unwrap_or_else(|| panic!("still held at {t_ms}"));
        assert_eq!(
            row.cell,
            Some((40, 10)),
            "the plot followed the target through the wall at {t_ms}"
        );
        assert_eq!(row.range_cm, 30_000, "and so did the range at {t_ms}");
        assert_eq!(row.age_ms, t_ms, "the plot is not ageing");
    }

    // The contact is lost at the TTL and re-acquired under the same id, at the
    // new sighting's plot.
    arena.now_ms = 3_000;
    let (_, journal) = pass(&mut arena, EYE, 4, 3_000);
    assert_eq!(kinds(&journal), vec!["lost"]);
    let open = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(50, 10), None),
        ],
        &[],
    );
    arena.definition = open.definition.clone();
    arena.now_ms = 4_000;
    pass(&mut arena, EYE, 5, 4_000);
    let row = arena
        .frame_contacts(EYE)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("re-acquired"));
    assert_eq!(
        row.id,
        draw_contact_id(arena.seed, EYE, MARK),
        "the same id, as cut S.1 minted it"
    );
    assert_eq!(row.cell, Some((50, 10)), "the plot jumped to the sighting");
    assert_eq!(row.range_cm, 40_000);
    assert_eq!(row.age_ms, 0);
}

#[test]
fn a_contact_row_prints_a_band_and_a_range_in_metres() {
    // **(m3)**: the band is a word about the range, and the range is beside it.
    //
    // **16b cut D.1** changed which distance the word is about: thirds of the
    // LESSER of the platform's own sight and the fired weapon's last band. The
    // fixture sight is 60 cells of 1,000 cm, so the limit is 60,000 cm and not
    // the cannon's 250,000.
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse sends a frame");
    let row = cockpit::frame_contacts(&frame)
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("one row"));
    // 30,000 cm against a 60,000 cm sight: the middle third.
    assert_eq!(row.band.as_str(), "effective");
    assert_eq!(row.range_m, 300, "300 m, to the nearest fifty");
    let line = row.render();
    assert!(line.contains("effective"), "{line}");
    assert!(line.contains("300 m"), "{line}");
    assert!(!line.contains('#'), "never an object number: {line}");

    // Past two thirds of the sight the word changes again — which is the whole
    // point of `B-25`: before this cut `far` could not print at all on any
    // shipped platform, because the cannon's own bands put it at 1.5 km and
    // nothing can be plotted past 900 m.
    place(&mut arena, MARK, centre(61, 10), 0);
    let mut last = None;
    for n in 2..=6 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            last = Some(frame);
        }
    }
    let frame = last.expect("the platform is sent frames");
    let row = cockpit::frame_contacts(&frame)
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("one row"));
    assert_eq!(row.band.as_str(), "far");
    // 51,000 cm is 510 m, and 510 m to the nearest fifty is 500.
    assert_eq!(row.range_m, 500);
    assert!(row.render().contains("500 m"), "{}", row.render());
}

/// **A contact on a wreck holds the `Alert` tempo until its holder knows** (16b
/// cut E.2, 16c `B-45`; doc 16 §1, §3.1).
///
/// Cut F.3 gave this trigger a wreck filter, and read it off the **target's**
/// live `limits.wreck` — which is a fact no platform on the field has been told.
/// The tempo therefore fell at the arena millisecond a hull burned, through a
/// wall, for a platform whose track was three seconds stale; and how often a
/// pilot is sent a frame is something a pilot can count, so that was hidden
/// truth reaching a pilot on the one surface this trigger's own doc says it must
/// not. The filter is now [`Arena::contact_reads_dead`]: the hold's own
/// knowledge.
///
/// `sees_wrecks: false` is untouched and still does what it was meant to do —
/// the wreck stops being refreshed and the track expires — it simply takes the
/// time to live to do it rather than one instant.
#[test]
fn a_contact_on_a_wreck_holds_the_alert_tempo_until_its_holder_knows() {
    let mut arena = watching();
    pass(&mut arena, EYE, 9, 9_000);
    assert!(
        arena.contact_within_alert_range(),
        "the fixture never held a contact inside the alert range"
    );
    if let Some(state) = arena.unit_mut(MARK) {
        state.limits.wreck = true;
    }
    assert!(
        arena.contact_within_alert_range(),
        "the hull burned and the eye's track said so before the eye did"
    );

    // The knowledge, written where the kill line writes it: the eye's own round
    // finished the tank. Now the trigger lets go.
    let id = draw_contact_id(arena.seed, EYE, MARK);
    if let Some(held) = arena.contacts.get_mut(EYE, id) {
        held.known_dead = true;
    }
    assert!(
        !arena.contact_within_alert_range(),
        "a contact the holder knows is dead still raised Alert"
    );

    // And a contact whose unit has left the match is dropped at once, **with the
    // kill still written on the hold**: *gone* is not *dead*, there is no unit
    // for the fact to be about, and `Arena::contact_reads_dead` says so in that
    // order — the lookup before the flag, which is what its doc claims and what
    // every caller that tests `self.unit(target)` itself relies on.
    arena.units.retain(|unit| unit.id != MARK);
    let held = arena.contacts.get(EYE, id).copied().expect("the hold");
    assert!(held.known_dead, "the fixture lost the fact it is about");
    assert!(
        !arena.contact_reads_dead(&held),
        "a contact on a unit that has left the match read as dead"
    );
    assert!(!arena.contact_within_alert_range());
}

#[test]
fn nothing_a_pilot_is_shown_is_newer_than_the_platforms_last_sighting() {
    // The observer rule read through 16b cut F.3: every column of the row, and
    // the cell `VIEW` draws its marker in, come from one plot — so there is no
    // surface on which a pilot can read where a target *is* rather than where
    // their platform last saw it.
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse");
    let seen = cockpit::frame_contacts(&frame)
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("one row"));
    // The target moves behind a wall, and the platform is told nothing more.
    let walled = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &curtain(25),
    );
    arena.definition = walled.definition.clone();
    for n in 2..=3_u16 {
        place(&mut arena, MARK, centre(40 + n, 12), 1_000);
        let Some(frame) = pulse(&mut arena, PULSE_MS * i64::from(n), &mut tape) else {
            continue;
        };
        let row = cockpit::frame_contacts(&frame)
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("still held"));
        assert_eq!(
            row.cell, seen.cell,
            "VIEW's marker moved without a sighting"
        );
        assert_eq!(row.bearing_deg, seen.bearing_deg);
        assert_eq!(row.range_m, seen.range_m);
        assert!(row.age_ms > 0, "the age column says the plot is stale");
    }
}

/// The plot `viewer` holds on `target`, if it holds one (16b cut F.3).
fn plot_of(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<SeenPose> {
    arena
        .contacts
        .resolve(viewer, draw_contact_id(arena.seed, viewer, target))
        .map(|held| held.seen_at)
}

#[test]
fn a_share_carries_the_sharers_own_plot_and_never_a_teammates() {
    // **(m1), the second half.** A platform may relay a contact only when its own
    // sensors saw it — and what it relays is *that* sighting's plot. A platform
    // whose own sighting has gone stale while a teammate's share keeps refreshing
    // its copy holds a fresh plot it never took; handing that on would put a
    // position one hop further than the platform that saw it, which is the
    // transitive relay doc 16 §2 W6 forbids, wearing somebody else's name.
    //
    // `EYE` sees `MARK` throughout. `MATE` sees it at first and then goes blind to
    // it. `RELAY` starts outside every mesh and joins `MATE`'s after the two have
    // diverged, so what it is told is one share and not an older one.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(10, 13), Some(visual())),
            tank(RELAY, Team::Red, centre(10, 70), Some(short_sight())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms + 10);
        pass(&mut arena, RELAY, seq, t_ms + 20);
    }
    let own = plot_of(&arena, MATE, MARK).unwrap_or_else(|| panic!("MATE saw it for itself"));
    assert_eq!(
        (own.x_cm, own.y_cm),
        (centre(40, 10).x_cm, centre(40, 10).y_cm),
        "MATE's own sighting is where MARK stood"
    );
    assert!(
        !holds_mark(&arena, RELAY),
        "RELAY is outside every mesh until it is moved"
    );

    // `MATE` goes blind to `MARK` — a shorter sight is the cheapest wall — and
    // `MARK` drives twenty cells north, where only `EYE` sees it.
    if let Some(state) = arena.unit_mut(MATE) {
        state.sensor = Some(short_sight());
    }
    place(&mut arena, MARK, centre(40, 30), 0);
    pass(&mut arena, EYE, 5, 4_000);
    pass(&mut arena, MATE, 5, 4_010);

    // The hard case really is set up: `MATE`'s copy now plots `MARK` at the new
    // position, off `EYE`'s share, while its own sensors have not seen it since.
    let theirs = plot_of(&arena, MATE, MARK).unwrap_or_else(|| panic!("the mesh still carries it"));
    assert_eq!(
        (theirs.x_cm, theirs.y_cm),
        (centre(40, 30).x_cm, centre(40, 30).y_cm),
        "the share never refreshed MATE's plot, so this test proves nothing"
    );

    // `RELAY` joins the formation: thirty cells from `MATE`, which is its mesh
    // exactly, and thirty-three from `EYE`, which is outside its.
    place(&mut arena, RELAY, centre(10, 43), 0);
    pass(&mut arena, EYE, 6, 5_000);
    pass(&mut arena, MATE, 6, 5_010);
    pass(&mut arena, RELAY, 6, 5_020);
    // And the relay is genuinely inside `MATE`'s mesh and outside `EYE`'s: it
    // learns of `EYE`, which only `MATE` can tell it about.
    assert!(
        arena
            .contacts
            .resolve(RELAY, draw_contact_id(arena.seed, RELAY, EYE))
            .is_some(),
        "RELAY is outside MATE's mesh"
    );

    // The property. `MATE` may relay `MARK` — its own sighting at 3,010 ms is
    // still inside the time to live — but only at the plot that sighting took.
    let relayed = plot_of(&arena, RELAY, MARK)
        .unwrap_or_else(|| panic!("MATE's own sighting is inside the TTL, so the mesh carries it"));
    assert_eq!(
        (relayed.x_cm, relayed.y_cm),
        (own.x_cm, own.y_cm),
        "a teammate's plot was relayed a second hop"
    );
    assert_ne!(
        (relayed.x_cm, relayed.y_cm),
        (theirs.x_cm, theirs.y_cm),
        "RELAY was told a position only EYE ever observed"
    );
}

// ---------------------------------------------------------------------------
// (n) the short designator — 16b cut Q.1
//
// The owner played the centre by hand and found that `lock 29b81a…` is not a
// thing a person types while being shot at. A contact is named by a short
// designator now: a viewer-scoped `X<n>` until the platform has told it apart,
// then the target's public callsign — its team's letter and its centre slot.
//
// The property that costs the most to get right is the one this block spends
// most of its lines on: **the name is read off the live contact and never off a
// flag**. A contact is not durable — expiry removes the row and the next
// detection re-inserts it `Unknown` under the same id — so anything remembered
// about "this has been classified" would put the team and the pod on a blip the
// platform has just re-found.
// ---------------------------------------------------------------------------

/// What one viewer's frame calls a target right now.
fn calls(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<String> {
    let id = draw_contact_id(arena.seed, viewer, target);
    arena
        .frame_contacts(viewer)
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.designator.to_string())
}

/// The alias beside it — the number a renamed row still answers to.
fn alias_of(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<u16> {
    let id = draw_contact_id(arena.seed, viewer, target);
    arena
        .frame_contacts(viewer)
        .into_iter()
        .find(|row| row.id == id)
        .and_then(|row| row.alias)
}

#[test]
fn provisional_numbers_are_per_viewer_in_first_seen_order_and_never_reused() {
    // `EYE` looks at two targets thirty cells out, and `MATE` at one of them.
    // `MARK` is numbered first because the pass walks the index in unit-id order
    // and `MARK` (#9) sorts before `RELAY` (#21) — the whole point of that order
    // being deterministic.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
            tank(RELAY, Team::Blue, centre(40, 20), None),
            tank(MATE, Team::Red, centre(12, 10), Some(visual())),
        ],
        &[],
    );
    pass(&mut arena, EYE, 1, 0);
    // One number per target its own sensors found, in unit-id order: `MARK` (#9),
    // then its own teammate `MATE` (#13), then `RELAY` (#21). A teammate is a
    // return like any other until the classification tells it apart.
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("X1"));
    assert_eq!(calls(&arena, EYE, MATE).as_deref(), Some("X2"));
    assert_eq!(calls(&arena, EYE, RELAY).as_deref(), Some("X3"));
    // A second viewer counts from one of its own: `MATE`'s `X1` is not `EYE`'s.
    pass(&mut arena, MATE, 1, 10);
    assert_eq!(calls(&arena, MATE, EYE).as_deref(), Some("X1"));
    // A number never moves: two more passes change nothing.
    for (seq, t_ms) in [(2_u64, 1_000_i64), (3, 2_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("X1"));
    assert_eq!(calls(&arena, EYE, RELAY).as_deref(), Some("X3"));
}

#[test]
fn a_contact_is_named_by_its_live_class_across_a_loss_and_a_re_detection() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
    );
    // Detect, then classify at 3,000 ms of continuous holding.
    pass(&mut arena, EYE, 1, 0);
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("X1"));
    assert_eq!(alias_of(&arena, EYE, MARK), None, "the number IS the name");
    for (seq, t_ms) in [(2_u64, 1_000_i64), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    // Blue's only unit takes blue's first free slot, in unit-id order.
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("BA"));
    assert_eq!(
        alias_of(&arena, EYE, MARK),
        Some(1),
        "the number the lock was taken under stays typable"
    );

    // Out of every reach, and held until the time to live runs out.
    place(&mut arena, MARK, centre(SIDE - 1, SIDE - 1), 0);
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    assert_eq!(calls(&arena, EYE, MARK), None, "the contact never expired");

    // **The critical case.** It comes back, and it is an unresolved return
    // again: the row reads the provisional number it always had, and NOT the
    // team and the slot.
    place(&mut arena, MARK, centre(40, 10), 0);
    pass(&mut arena, EYE, 8, 7_000);
    assert_eq!(
        calls(&arena, EYE, MARK).as_deref(),
        Some("X1"),
        "a re-acquired blip was named by team and slot"
    );
    assert_eq!(alias_of(&arena, EYE, MARK), None);
    // And it earns the name back only at its SECOND classification.
    for (seq, t_ms) in [(9_u64, 8_000_i64), (10, 9_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        assert_eq!(
            calls(&arena, EYE, MARK).as_deref(),
            Some("X1"),
            "named early at {t_ms}"
        );
    }
    pass(&mut arena, EYE, 11, 10_000);
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("BA"));
}

#[test]
fn a_mesh_arrival_carries_the_slot_name_and_consumes_no_number() {
    // `MATE` sees `MARK` and shares it; `EYE` is inside `MATE`'s mesh and its own
    // sight cannot reach `MARK`.
    // `MATE`'s sight reaches sixteen cells, which at `ground` stopped is 14,400 cm:
    // it holds `MARK` twelve cells east and cannot see `RELAY` sixteen cells west,
    // so the only thing that reaches `EYE` over the mesh is `MARK`. `EYE`'s own
    // sight is five cells, which reaches `RELAY` and nothing else.
    let mate_sight = SensorProfile {
        max_range_cells: 16,
        ..visual()
    };
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(short_sight())),
            tank(MATE, Team::Red, centre(30, 10), Some(mate_sight)),
            tank(MARK, Team::Blue, centre(42, 10), None),
            tank(RELAY, Team::Blue, centre(14, 10), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, MATE, seq, t_ms);
    }
    // A mesh copy arrives classified, so it renders the slot name at once — and
    // no number was issued for it, which is what makes the sequence mean "things
    // my own sensors found, in order".
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("BA"));
    assert_eq!(
        alias_of(&arena, EYE, MARK),
        None,
        "a mesh arrival was given a number it never showed"
    );
    assert_eq!(arena.designators.provisional(EYE, MARK), None);

    // **Contiguity.** `EYE`'s own short sight finds the near target, which takes
    // the FIRST number: the mesh-first one consumed none.
    pass(&mut arena, EYE, 1, 0);
    assert_eq!(calls(&arena, EYE, RELAY).as_deref(), Some("X1"));

    // And when the mesh copy expires and `EYE`'s **own** sight later finds the
    // same target, that is the insert that issues a number — the NEXT one, so the
    // sequence has no hole in it. A row refreshed in place issues nothing, which
    // is why the copy has to go first.
    place(&mut arena, MARK, centre(SIDE - 1, SIDE - 1), 0);
    for (seq, t_ms) in [
        (2_u64, 4_000_i64),
        (3, 5_000),
        (4, 6_000),
        (5, 7_000),
        (6, 8_000),
    ] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    assert_eq!(
        calls(&arena, EYE, MARK),
        None,
        "the mesh copy never expired"
    );
    if let Some(state) = arena.unit_mut(EYE) {
        state.sensor = Some(visual());
    }
    place(&mut arena, MARK, centre(40, 10), 0);
    pass(&mut arena, EYE, 7, 9_000);
    assert_eq!(
        arena.designators.provisional(EYE, MARK),
        Some(2),
        "a target this platform's own sight found takes the next number"
    );
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("X2"));
}

#[test]
fn a_slot_is_taken_explicitly_then_by_unit_id_and_a_duplicate_is_refused() {
    // Explicit slots win, and the slot-0 units take the lowest free ones in
    // unit-id order. `RELAY` (#21) is fielded from pod 1, so `MARK` (#9) — which
    // sorts first — has to take pod 2.
    let mut units = vec![
        tank(MARK, Team::Blue, centre(40, 10), None),
        tank(RELAY, Team::Blue, centre(41, 10), None),
        tank(BLIND, Team::Blue, centre(42, 10), None),
        tank(EYE, Team::Red, centre(10, 10), Some(visual())),
    ];
    if let Some(unit) = units.get_mut(1) {
        unit.slot = 1;
    }
    let arena = arena_of(units, &[]);
    let named = |id: ObjId| {
        arena
            .unit(id)
            .map(|state| state.callsign.to_string())
            .unwrap_or_default()
    };
    assert_eq!(named(RELAY), "BA", "the explicit slot");
    assert_eq!(named(MARK), "BB");
    assert_eq!(named(BLIND), "BC");
    assert_eq!(named(EYE), "RA", "a side counts its own slots");

    // Twenty-seven a side rolls into two letters.
    let mut wide: Vec<CustodyUnit> = Vec::new();
    for n in 0..27_u32 {
        let mut unit = tank(
            ObjId(100 + n),
            Team::Red,
            centre(u16::try_from(n).unwrap_or(0), 40),
            None,
        );
        unit.slot = u8::try_from(n + 1).unwrap_or(0);
        wide.push(unit);
    }
    let wide = arena_of(wide, &[]);
    assert_eq!(
        wide.unit(ObjId(126))
            .map(|state| state.callsign.to_string()),
        Some("RAA".to_owned()),
        "slot 27 rolls past Z"
    );

    // Two units of one side in one pod would answer to one name, and that is a
    // refusal and not a rename.
    let mut clash = vec![
        tank(MARK, Team::Blue, centre(40, 10), None),
        tank(RELAY, Team::Blue, centre(41, 10), None),
    ];
    for unit in &mut clash {
        unit.slot = 2;
    }
    let refused = Arena::new(
        CustodyOut {
            definition: arena.definition.clone(),
            units: clash,
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    );
    assert!(
        matches!(
            refused,
            Err(tacsim::arena::state::ArenaError::DuplicateSlot {
                team: "blue",
                slot: 2
            })
        ),
        "two units in one pod: {refused:?}"
    );
}

#[test]
fn the_switch_names_a_contact_at_detection_and_issues_no_number() {
    let sensors = SensorsConfig {
        designate_on_detection: true,
        ..SensorsConfig::default()
    };
    let mut arena = arena_with_sensors(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &[],
        &sensors,
    );
    let (_, journal) = pass(&mut arena, EYE, 1, 0);
    assert_eq!(kinds(&journal), vec!["detected"]);
    // The team and the pod, at the first pass — `classify_after_ms` before doc 16
    // W5 says identification is earned, which is exactly what the key costs.
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("BA"));
    assert_eq!(alias_of(&arena, EYE, MARK), None);
    assert_eq!(
        arena.designators.provisional(EYE, MARK),
        None,
        "a provisional number was issued that nothing could ever show"
    );
    // The classification still happens in the arena; only the naming stopped
    // waiting for it.
    for (seq, t_ms) in [(2_u64, 1_000_i64), (3, 2_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    let (_, journal) = pass(&mut arena, EYE, 4, 3_000);
    assert_eq!(kinds(&journal), vec!["classified"]);
    assert_eq!(calls(&arena, EYE, MARK).as_deref(), Some("BA"));
}

// ---------------------------------------------------------------------------
// (o) the cross-tempo property, with the designators in it — 16b cut Q.1
//
// 16b §11 makes this mandatory for every cut that changes arena behaviour, and
// this cut changes three things the property is about: the `power_online` event
// (a new ledger line, so every later `seq` and therefore every later hash
// moves), the designator book (folded into the state hash), and what a pilot is
// told about a contact (the notice lines).
//
// So the run below is scripted to contain all three — a `STARTUP` that finishes,
// a classification, and a loss — and the three cadences have to agree on the
// event list, the hash sequence, the per-pilot lines **and** the designator
// column each pilot's scope reads at every checkpoint.
// ---------------------------------------------------------------------------

/// One order, at one arena millisecond.
fn intent(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

/// The seed `arena_of` opens every fixture window with, so the script can name
/// the contact the lock below is taken on.
const SCRIPT_SEED: u64 = 2_350;

/// The scripted log: both tanks start up, the watcher locks the return three
/// seconds after it has been told apart, and fifteen seconds in — five seconds
/// after the ten-second startup finishes — the target drives east out of the
/// watcher's reach, which is what produces the loss.
///
/// The lock is in the script because the **name a loss is said under** is the one
/// thing in this cut that cannot be read off any surviving state: the row is gone
/// by the time the notice is staged, and the designator has to have been captured
/// at the expiry. A loss and a broken lock are the two sentences that prove that
/// capture, and both of them are here.
///
/// **Every beat is at a multiple of five seconds of arena time**, which is a
/// pulse boundary at 5,000, 1,000 and 500 ms alike. A beat at any other
/// millisecond would be delivered at a different arena time at each cadence and
/// the three runs would diverge for a reason that is the script's and not the
/// engine's — the same rule `resolution.rs`' own replay driver follows.
fn designator_script() -> Vec<(i64, Vec<Intent>)> {
    vec![
        (
            0,
            vec![
                intent(EYE, IntentKind::Startup, 1),
                intent(MARK, IntentKind::Startup, 2),
            ],
        ),
        (
            10_000,
            vec![intent(
                EYE,
                IntentKind::Lock {
                    contact: draw_contact_id(SCRIPT_SEED, EYE, MARK),
                },
                3,
            )],
        ),
        (
            15_000,
            vec![
                intent(MARK, IntentKind::Heading { mdeg: 90_000 }, 4),
                intent(MARK, IntentKind::Speed { cms: 1_000 }, 5),
            ],
        ),
    ]
}

/// The arena the script is run on: a ten-cell sight, so the target is held at
/// five cells, classified three seconds later, and lost once it has driven past
/// a hundred metres.
fn designator_arena() -> Arena {
    arena_of(
        vec![
            tank(
                EYE,
                Team::Red,
                centre(10, 10),
                // Ten cells of reach, and seven seconds to tell a return apart
                // rather than the shipped three: a five-second checkpoint then
                // falls inside the provisional phase and another inside the
                // classified one, so the column this test compares really moves.
                Some(SensorProfile {
                    max_range_cells: 10,
                    classify_after_ms: 7_000,
                    ..visual()
                }),
            ),
            tank(MARK, Team::Blue, centre(15, 10), None),
        ],
        &[],
    )
}

/// One ledger line, normalised for comparison: its arena timestamp, its kind and
/// fields as JSON, the draw where one was made, and the substep hash. The `seq`
/// is dropped because it is the one field a different number of pulses could
/// legitimately renumber.
type Line = (i64, String, Vec<(String, u64)>, u64);

/// What one replay of the script produced.
#[derive(Debug, PartialEq, Eq)]
struct Cadenced {
    /// Every ledger line, in order.
    events: Vec<Line>,
    /// Every substep hash, in order.
    hashes: Vec<u64>,
    /// Every line each pilot was sent, in order — rendered, so read only for the
    /// sentence assertions on the first run.
    notices: BTreeMap<ObjId, Vec<String>>,
    /// **Every notice each pilot was staged**, in order, as the arena staged it
    /// (16b cut M.2, 16c `T-84`). The three-cadence comparison is made on this
    /// and not on `notices`: a rendered line reads a bearing and a band off the
    /// frame of the pulse it arrived in, so the lines agree across cadences only
    /// while the observers stand still.
    staged: BTreeMap<ObjId, Vec<FrameNotice>>,
    /// **The designator column each platform's scope read** at every checkpoint
    /// — a multiple of five seconds of arena time, which is a pulse boundary at
    /// all three cadences.
    columns: Vec<(i64, ObjId, Vec<String>)>,
    /// How many pulses it took, which is the guard that the three runs really
    /// were three cadences.
    pulses: u32,
}

/// **One beat of a replay script** (16b cut Q.1; 16b cut D.4 gave it `places`):
/// the orders it delivers, and where it puts units at the same arena
/// millisecond.
///
/// The placements are the fixture's own hand on the field. They are applied when
/// the beat is consumed, which is at exactly the arena millisecond the script
/// names **at every cadence** — the beats are all multiples of five seconds, and
/// 5,000, 1,000 and 500 ms all divide it — so the three replays put the target in
/// the same cell at the same moment and any difference in what follows is the
/// engine's. A treeline flicker cannot be scripted any other way: it needs the
/// target outside the reach for exactly three passes and inside it for the
/// fourth, and an accelerating tank cannot be steered that finely.
struct Beat {
    t_ms: i64,
    intents: Vec<Intent>,
    places: Vec<(ObjId, Pose)>,
}

/// Drive `arena` through the script at `cadence_ms` until `until_ms` of arena
/// time, with the tempo pinned so the cadence under test is the cadence that
/// runs.
fn cadenced(cadence_ms: i64, until_ms: i64) -> Cadenced {
    let script: Vec<Beat> = designator_script()
        .into_iter()
        .map(|(t_ms, intents)| Beat {
            t_ms,
            intents,
            places: Vec::new(),
        })
        .collect();
    replayed(designator_arena(), &script, cadence_ms, until_ms)
}

/// The driver itself (16b cut D.4 split it out of [`cadenced`], which is now one
/// fixture's call of it).
fn replayed(arena: Arena, script: &[Beat], cadence_ms: i64, until_ms: i64) -> Cadenced {
    replayed_driven(arena, script, Drive::Every(cadence_ms), until_ms).0
}

/// How [`replayed_driven`] advances the wall (16b cut U.1).
#[derive(Clone, Copy, Debug)]
enum Drive {
    /// A fixed cadence: `wall += cadence` after every pulse.
    Every(i64),
    /// The task's own wake rule: `wall = out.next_due_ms`, with the `Quiet`
    /// period set to this figure on the arena's frozen tempo block first. Every
    /// beat is a multiple of 5,000 ms and the period term is absolute, so every
    /// beat is a wake at 5,000, 1,000 and 500 alike, and so is every checkpoint.
    Task(u32),
}

/// [`replayed`] under `drive`; answers the replay and every arena millisecond a
/// pulse after the opening one ended at.
fn replayed_driven(
    mut arena: Arena,
    script: &[Beat],
    drive: Drive,
    until_ms: i64,
) -> (Cadenced, Vec<i64>) {
    if let Drive::Task(period_ms) = drive {
        arena.arena_cfg.tempo.quiet_ms = period_ms;
    }
    let mut next_due = 0_i64;
    let mut woke_at: Vec<i64> = Vec::new();
    let mut tape: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut notices: BTreeMap<ObjId, Vec<String>> = BTreeMap::new();
    let mut staged: BTreeMap<ObjId, Vec<FrameNotice>> = BTreeMap::new();
    let mut columns: Vec<(i64, ObjId, Vec<String>)> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let mut orders: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            orders.extend(beat.intents.iter().cloned());
            // 16b cut D.4: the fixture's own hand, at the beat's arena time.
            for (unit, pose) in &beat.places {
                place(&mut arena, *unit, pose.clone(), 0);
            }
            next += 1;
        }
        if !orders.is_empty() {
            // A zero-length pulse, so a scripted order resolves at exactly the
            // millisecond the script names at every cadence.
            let out = arena.step(wall, &[] as &[Control], &orders);
            for chunk in &out.ledger_chunks {
                tape.extend_from_slice(&chunk.bytes);
            }
            for (pilot, frame) in &out.frames {
                let lines = cockpit::notice_lines(frame);
                if !lines.is_empty() {
                    notices.entry(*pilot).or_default().extend(lines);
                }
                if !frame.notices.is_empty() {
                    staged
                        .entry(*pilot)
                        .or_default()
                        .extend(frame.notices.iter().cloned());
                }
            }
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses = pulses.saturating_add(1);
            next_due = out.next_due_ms;
        }
        if arena.now_ms >= until_ms {
            break;
        }
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            // The opening pulse is at wall nought, as the task's is.
            Drive::Task(_) if pulses == 0 => 0,
            Drive::Task(_) => {
                assert!(next_due > wall, "the task was asked to wake in the past");
                next_due
            }
        };
        let out = arena.step(wall, &[] as &[Control], &[]);
        next_due = out.next_due_ms;
        if pulses > 0 {
            woke_at.push(arena.now_ms);
        }
        for chunk in &out.ledger_chunks {
            tape.extend_from_slice(&chunk.bytes);
        }
        for (pilot, frame) in &out.frames {
            let lines = cockpit::notice_lines(frame);
            if !lines.is_empty() {
                notices.entry(*pilot).or_default().extend(lines);
            }
            if !frame.notices.is_empty() {
                staged
                    .entry(*pilot)
                    .or_default()
                    .extend(frame.notices.iter().cloned());
            }
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses = pulses.saturating_add(1);
        // The checkpoint: every five seconds of ARENA time, which is a pulse
        // boundary at 5,000, 1,000 and 500 ms alike.
        if arena.now_ms % 5_000 == 0 {
            for platform in [EYE, MARK] {
                let column: Vec<String> = arena
                    .frame_contacts(platform)
                    .into_iter()
                    .map(|row| row.designator.to_string())
                    .collect();
                columns.push((arena.now_ms, platform, column));
            }
        }
        if out.ended.is_some() || arena.now_ms >= until_ms {
            break;
        }
    }
    let log = events(&mut arena, &mut tape);
    let replay = Cadenced {
        events: log
            .iter()
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.draws.clone(),
                    event.hash,
                )
            })
            .collect(),
        hashes,
        notices,
        staged,
        columns,
        pulses,
    };
    (replay, woke_at)
}

/// **16b cut U.1's cross-tempo proof where the passes change things** (16b §11;
/// doc 16 §3.1 "Rate independence", D-16-2; 16c `T-22`). The approach and the
/// flicker of this file, driven the way the task drives them — waking only on
/// the 250 ms substep lattice in absolute arena time, one substep a pulse
/// wherever a pass falls, with the `Quiet` period at 5,000, 1,000 and 500 ms —
/// against their fixed 5,000 ms replays. `EYE`'s passes are off the lattice
/// (482 ms into each second on the approach, 2,482 into each five on the
/// flicker, both asserted), and they detect, classify, expire and re-acquire:
/// one event list, one per-substep hash sequence, one staged notice list and
/// one designator column, at all three periods.
#[test]
fn u1_the_passes_on_the_task_s_own_wakes_are_the_fixed_cadence_replay() {
    let approach: Vec<Beat> = designator_script()
        .into_iter()
        .map(|(t_ms, intents)| Beat {
            t_ms,
            intents,
            places: Vec::new(),
        })
        .collect();
    let flicker = flicker_replay_script();
    type Fixture = fn() -> Arena;
    let cases: [(&str, Fixture, &[Beat], i64, i64); 2] = [
        ("approach", designator_arena, &approach, 35_000, 1_000),
        ("flicker", flicker_replay_arena, &flicker, 25_000, 5_000),
    ];
    for (name, fixture, script, until_ms, pass_ms) in cases {
        let offset = stagger_ms(2_350, EYE, pass_ms);
        assert_ne!(
            offset % 250,
            0,
            "the precondition: {name}'s passes are off the lattice"
        );
        let (fixed, _) = replayed_driven(fixture(), script, Drive::Every(5_000), until_ms);
        let kinds: Vec<&str> = ["detected", "classified", "lost"]
            .into_iter()
            .filter(|want| {
                !fixed
                    .events
                    .iter()
                    .any(|(_, json, _, _)| json.contains(&format!("\"kind\":\"{want}\"")))
            })
            .collect();
        assert!(kinds.is_empty(), "{name} journalled no {kinds:?}");
        for period in [5_000_u32, 1_000, 500] {
            let (run, woke_at) = replayed_driven(fixture(), script, Drive::Task(period), until_ms);
            let off: Vec<i64> = woke_at
                .iter()
                .copied()
                .filter(|now| now % 250 != 0)
                .collect();
            assert!(
                off.is_empty(),
                "{name}: the task woke off the lattice at a {period} ms period: {off:?}"
            );
            assert_ne!(
                run.pulses, fixed.pulses,
                "{name}: the task driver ran the fixed replay's pulses at {period} ms"
            );
            assert_eq!(
                run.events, fixed.events,
                "{name}: the events differ on the task's wakes at a {period} ms period"
            );
            assert_eq!(
                run.hashes, fixed.hashes,
                "{name}: the hashes differ on the task's wakes at a {period} ms period"
            );
            assert_eq!(
                run.staged, fixed.staged,
                "{name}: the staged notices differ on the task's wakes at a {period} ms period"
            );
            assert_eq!(
                run.columns, fixed.columns,
                "{name}: the designator column differs on the task's wakes at a {period} ms \
                 period"
            );
        }
    }
}

#[test]
fn one_scripted_approach_at_three_cadences_is_one_list_one_hash_and_one_column() {
    let mut runs: Vec<(i64, Cadenced)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        runs.push((cadence, cadenced(cadence, 35_000)));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: it really did carry all three of the things
    // this cut moved.
    let kinds: Vec<String> = first
        .events
        .iter()
        .filter_map(|(_, json, _, _)| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            Some(value.get("kind")?.as_str()?.to_owned())
        })
        .collect();
    for wanted in ["power_online", "detected", "classified", "lost"] {
        assert!(
            kinds.iter().any(|kind| kind == wanted),
            "the script produced no {wanted}: {kinds:?}"
        );
    }
    // `power_online` is one line per started unit, at one arena millisecond.
    let online: Vec<i64> = first
        .events
        .iter()
        .filter(|(_, json, _, _)| json.contains("\"power_online\""))
        .map(|(t_ms, _, _, _)| *t_ms)
        .collect();
    assert_eq!(
        online,
        vec![10_000, 10_000],
        "both tanks came online at the startup's own timestamp"
    );
    // And the pilot was told, once, in the tank's own words.
    assert!(
        first.notices.get(&ObjId(1_000)).is_some_and(|lines| lines
            .iter()
            .filter(|line| *line == "Power online.")
            .count()
            == 1),
        "{:?}",
        first.notices
    );
    // The designator column really moved: `X1`, then the slot name, then nothing.
    let watcher: Vec<(i64, Vec<String>)> = first
        .columns
        .iter()
        .filter(|(_, platform, _)| *platform == EYE)
        .map(|(t_ms, _, column)| (*t_ms, column.clone()))
        .collect();
    assert!(
        watcher
            .iter()
            .any(|(_, column)| column == &["X1".to_owned()]),
        "the scope never read a provisional number: {watcher:?}"
    );
    assert!(
        watcher
            .iter()
            .any(|(_, column)| column == &["BA".to_owned()]),
        "the scope never read the slot name: {watcher:?}"
    );
    assert!(
        watcher.iter().any(|(_, column)| column.is_empty()),
        "the contact was never lost: {watcher:?}"
    );

    // **The name a loss is said under** (16b cut Q.1's critical mechanism). The
    // row is gone by the time `Lost` and `LockLost` are staged, so if the
    // designator were not captured at the expiry the pilot who had been calling
    // this tank `BA` for a quarter of a minute would be told `Contact X1 lost.`
    // — and all three cadences would agree on the wrong name, which is why the
    // equality below cannot be what proves it.
    let told = first
        .notices
        .get(&ObjId(1_000))
        .cloned()
        .unwrap_or_default();
    assert!(
        told.iter().any(|line| line.starts_with("Locked on BA")),
        "the lock was never taken under the slot name: {told:?}"
    );
    assert!(
        told.iter()
            .any(|line| line == "Lock on BA broken: contact gone."),
        "the broken lock was said under the name the row had at the expiry: {told:?}"
    );
    assert!(
        told.iter().any(|line| line == "Contact BA lost."),
        "the loss was said under the name the row had at the expiry: {told:?}"
    );

    // The guard the sibling tests carry: three cadences really are three
    // different numbers of pulses, so an equality below is a property and not a
    // tautology.
    let counts: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair.first() != pair.get(1)),
        "the three replays ran the same number of pulses: {counts:?}"
    );

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the events differ at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hashes differ at {cadence} ms"
        );
        // 16b cut M.2 (`T-84`): what was staged, not what it rendered as.
        assert_eq!(
            run.staged, first.staged,
            "the pilots' staged notices differ at {cadence} ms"
        );
        assert_eq!(
            run.columns, first.columns,
            "the designator column differs at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (p) the close look, the reading it earns, and what the mesh does with one
//     — 16b cut Q.2
//
// The shipped sight reaches sixty cells at factor 1,000 and `ground` stopped is
// 900, so its effective reach on this window is 54,000 cm — and
// `condition_reach_permille` of 500 makes the close look 27,000 cm. Twenty cells
// is therefore a look and thirty is a detection with no look in it, which is the
// pair every test below is built on.
// ---------------------------------------------------------------------------

/// Twenty cells from `EYE`: detected, and inside the close-look reach.
const NEAR: (u16, u16) = (30, 10);

/// Thirty cells: detected, and outside it.
const FAR: (u16, u16) = (40, 10);

/// The reading `viewer` holds on `target`, straight out of the store.
fn reading(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<Reading> {
    let id = draw_contact_id(arena.seed, viewer, target);
    arena
        .contacts
        .get(viewer, id)
        .and_then(|held| held.seen_condition)
}

/// The reading `viewer`'s **own** sensors took, which is what the mesh may carry.
fn own_reading(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<Reading> {
    let id = draw_contact_id(arena.seed, viewer, target);
    arena
        .contacts
        .get(viewer, id)
        .and_then(|held| held.own_seen)
        .and_then(|own| own.condition)
}

#[test]
fn a_reading_is_taken_only_inside_the_close_look_and_only_once_classified() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    // The first pass detects, and the contact is `Unknown`: an unresolved return
    // has no section vocabulary to read bands into, so there is no reading even
    // though the target is well inside the close look.
    pass(&mut arena, EYE, 1, 0);
    assert_eq!(
        arena
            .contacts
            .get(EYE, draw_contact_id(arena.seed, EYE, MARK))
            .map(|held| held.class),
        Some(Classification::Unknown)
    );
    assert_eq!(reading(&arena, EYE, MARK), None, "an unknown return read");
    assert_eq!(own_reading(&arena, EYE, MARK), None);

    // **The pass that classifies is also the pass that reads.** The reading is
    // written after the classification arm, so the pilot is never told
    // `Contact X1 identified: BA, blue.` and then `No reading on BA yet` about a
    // tank they are looking straight at.
    pass(&mut arena, EYE, 2, 1_000);
    pass(&mut arena, EYE, 3, 2_000);
    pass(&mut arena, EYE, 4, 3_000);
    let read = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the classifying pass read"));
    assert_eq!(read.at_ms, 3_000, "the look's own stamp");
    assert_eq!(
        read.condition.sections, 1,
        "the fixture tank has one section"
    );
    assert_eq!(read.condition.letters(), "g");
    // What its own sensors took is what it holds, because nothing else has told
    // it anything.
    assert_eq!(own_reading(&arena, EYE, MARK), Some(read));
}

#[test]
fn a_detection_outside_the_close_look_keeps_the_old_reading_and_its_old_stamp() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    let close = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("a look was taken"));
    assert_eq!(close.at_ms, 3_000);

    // The target backs off to thirty cells: still detected, no longer close.
    place(&mut arena, MARK, centre(FAR.0, FAR.1), 0);
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    // Still held, and still classified: the plot is being refreshed every pass.
    let id = draw_contact_id(arena.seed, EYE, MARK);
    let held = arena
        .contacts
        .get(EYE, id)
        .copied()
        .unwrap_or_else(|| panic!("still held"));
    assert_eq!(held.last_seen_ms, 6_000, "the plot is current");
    // And the reading is the one taken at 3,000 — carried forward, with its own
    // stamp, on both fields. `SCAN` therefore says how old the *look* is, which
    // is three seconds older than the track.
    assert_eq!(held.seen_condition, Some(close), "the look was lost");
    assert_eq!(own_reading(&arena, EYE, MARK), Some(close));

    // The frame renders that age, and it is the look's and not the plot's.
    let row = arena
        .frame_contacts(EYE)
        .into_iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("the row is on the frame"));
    let carried = row
        .condition
        .unwrap_or_else(|| panic!("the row carries it"));
    assert_eq!(carried.age_ms, arena.now_ms.saturating_sub(3_000));
}

#[test]
fn a_target_that_breaks_contact_keeps_its_last_reading_until_the_row_dies() {
    // The teammate is deliberately **thirty cells** from the target and only ten
    // from the watcher: far enough that no pass of its own can take a close look
    // (the reach is twenty-seven), near enough to keep handing the watcher's row
    // over the mesh. So the only reading in this run is the watcher's own, and
    // nothing can quietly refresh it.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(0, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms);
    }
    let watched = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the watcher read"));
    assert_eq!(watched.at_ms, 3_000, "the watcher's own look");
    // The teammate holds a copy of the watcher's reading — that is the mesh
    // working — but its **own** sensors read nothing, and a share carries only
    // what a platform's own sensors read. So nothing it hands back can ever be
    // newer than what the watcher already has.
    assert_eq!(
        own_reading(&arena, MATE, MARK),
        None,
        "the teammate is outside its close look and read nothing itself"
    );

    // The watcher's eye is put out — it detects nothing from here on — while the
    // teammate keeps holding the target and keeps handing the row over. And the
    // target is taken apart meanwhile, so a picture that *did* silently refresh
    // would read differently rather than identically.
    if let Some(state) = arena.unit_mut(EYE) {
        state.sensor = None;
    }
    if let Some(state) = arena.unit_mut(MARK) {
        state.condition = state.condition.with_band(0, ConditionBand::Destroyed);
    }
    for (seq, t_ms) in [(5_u64, 4_000_i64), (6, 5_000), (7, 6_000), (8, 7_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms);
    }
    let still = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the row is meshed alive"));
    assert_eq!(
        still, watched,
        "the picture changed with nobody looking: it is the same look, bands and stamp"
    );
    assert_eq!(
        still.condition.letters(),
        "g",
        "the destroyed section is the one the watcher cannot see happen"
    );
    // The row itself is still there, which is what makes the equality above a
    // claim about the reading and not about an empty scope.
    let id = draw_contact_id(arena.seed, EYE, MARK);
    assert!(arena.contacts.get(EYE, id).is_some(), "the row is alive");

    // ...**until the row dies.** With nobody holding it the mesh stops, the time
    // to live runs out and the row goes — and the reading goes with it, because
    // a reading is a fact about a contact and not about a target.
    if let Some(state) = arena.unit_mut(MATE) {
        state.sensor = None;
    }
    for (seq, t_ms) in [(9_u64, 8_000_i64), (10, 9_000), (11, 10_000), (12, 11_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms);
    }
    assert!(
        arena.contacts.get(EYE, id).is_none(),
        "the row outlived its time to live"
    );
    assert_eq!(reading(&arena, EYE, MARK), None, "the reading outlived it");
}

#[test]
fn a_section_the_gunner_watched_go_survives_a_teammates_older_look() {
    // **The one thing a share does not overwrite.** A gunner's written-in
    // `Destroyed` keeps the *old look's* stamp, because it is not a look — so a
    // teammate's sighting taken between that look and the fire is stamped newer
    // while knowing strictly less, and adopting it whole would make `SCAN`
    // contradict the line the pilot was just sent half a second earlier.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(11, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    // Both are inside the close look, so both read — the teammate's a second
    // later than the watcher's, which is the whole of the trap.
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
        pass(&mut arena, MATE, seq, t_ms);
    }
    pass(&mut arena, MATE, 5, 4_000);
    let mate = reading(&arena, MATE, MARK).unwrap_or_else(|| panic!("the teammate read"));
    assert_eq!(mate.at_ms, 4_000, "the teammate's look is the newer one");
    assert_eq!(mate.condition.letters(), "g", "and it reads a whole tank");

    // The watcher's own gunner then watches the one section come apart. The
    // write-in **leaves the stamp alone** — it is not a look — so any sighting a
    // teammate takes between that stamp and the fire is stamped newer while
    // knowing strictly less, which is the trap.
    let id = draw_contact_id(arena.seed, EYE, MARK);
    let before = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the watcher holds one"));
    if let Some(held) = arena.contacts.get_mut(EYE, id)
        && let Some(read) = held.seen_condition.as_mut()
    {
        read.condition = read.condition.with_band(0, ConditionBand::Destroyed);
    }
    assert_eq!(
        reading(&arena, EYE, MARK).map(|read| (read.at_ms, read.condition.letters())),
        Some((before.at_ms, "d".to_owned())),
        "the write-in moved the stamp"
    );

    // The teammate's next look arrives over the mesh. It is newer, so it is
    // taken — but the section the gunner watched go is still gone, because a
    // section does not come back and `SCAN` may not contradict the line the
    // pilot was just sent.
    pass(&mut arena, MATE, 6, 5_000);
    let held = reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the row still holds one"));
    assert_eq!(held.at_ms, 5_000, "the newer share was refused outright");
    assert_eq!(
        held.condition.letters(),
        "d",
        "a teammate's look put a destroyed section back"
    );
}

#[test]
fn a_share_carries_the_sharers_own_reading_and_never_replaces_a_newer_one() {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            // Blind: everything on its scope arrived over the mesh.
            tank(MATE, Team::Red, centre(11, 10), None),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    let shared = reading(&arena, MATE, MARK).unwrap_or_else(|| panic!("the mesh carried it"));
    assert_eq!(
        shared,
        reading(&arena, EYE, MARK).unwrap_or_else(|| panic!("the sharer read")),
        "the receiver holds the sharer's own reading, stamp and all"
    );
    // **And no second hop.** What the receiver's own sensors read is still
    // nothing, which is what stops it relaying the picture onward.
    assert_eq!(own_reading(&arena, MATE, MARK), None);

    // A newer reading the receiver already holds is not overwritten by an older
    // share. The rule is the plot's, applied to the reading's own stamp.
    let id = draw_contact_id(arena.seed, MATE, MARK);
    let newer = Reading {
        at_ms: 9_000,
        condition: Condition::default().with_band(0, ConditionBand::Red),
    };
    if let Some(held) = arena.contacts.get_mut(MATE, id) {
        held.seen_condition = Some(newer);
    }
    pass(&mut arena, EYE, 5, 4_000);
    assert_eq!(
        reading(&arena, MATE, MARK),
        Some(newer),
        "an older share overwrote a newer reading"
    );
    // ...and a share that IS newer is taken.
    pass(&mut arena, EYE, 6, 10_000);
    assert_eq!(
        reading(&arena, MATE, MARK).map(|read| read.at_ms),
        Some(10_000),
        "a newer share was refused"
    );
}

#[test]
fn a_mesh_copy_of_a_contact_nobody_has_read_carries_no_reading_and_no_chassis() {
    // The sharer holds the target classified but has never been close enough to
    // read it: thirty cells is a detection and not a look. So the receiver's row
    // is classified, carries no reading — and the frame publishes no section
    // vocabulary for it at all, which is the whole of the `scan_tables` rule.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MATE, Team::Red, centre(11, 10), None),
            tank(MARK, Team::Blue, centre(FAR.0, FAR.1), None),
        ],
        &[],
    );
    for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        pass(&mut arena, EYE, seq, t_ms);
    }
    for viewer in [EYE, MATE] {
        let id = draw_contact_id(arena.seed, viewer, MARK);
        assert!(
            arena
                .contacts
                .get(viewer, id)
                .is_some_and(|held| matches!(held.class, Classification::Classified { .. })),
            "#{} does not hold it classified",
            viewer.0
        );
        assert_eq!(reading(&arena, viewer, MARK), None);
    }
    for platform in [EYE, MATE] {
        let id = draw_contact_id(arena.seed, platform, MARK);
        let row = arena
            .frame_contacts(platform)
            .into_iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("#{} holds the row", platform.0));
        assert!(
            row.condition.is_none(),
            "a row published a reading nobody took"
        );
    }
    // And the blind teammate's whole frame carries no table either, because the
    // one contact on it is the mesh copy. (The watcher's does: it is a cell away
    // from its own teammate and has read *that* tank, which is exactly what the
    // table list is for.)
    let scope = arena.frame_scan_tables(MATE);
    assert!(
        scope.is_empty(),
        "a contact with no reading published a chassis: {scope:?}"
    );
    assert!(
        !arena.frame_scan_tables(EYE).is_empty(),
        "the watcher read its own teammate, so its frame must name a chassis"
    );
}

#[test]
fn the_state_hash_moves_with_both_readings() {
    let base = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    let id = draw_contact_id(base.seed, EYE, MARK);
    let plant = |change: &dyn Fn(&mut tacsim::arena::state::ShadowContact)| -> u64 {
        let mut arena = arena_of(
            vec![
                tank(EYE, Team::Red, centre(10, 10), Some(visual())),
                tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
            ],
            &[],
        );
        arena.contacts.insert(
            EYE,
            id,
            tacsim::arena::state::ShadowContact::sighted(
                MARK,
                Classification::Classified { team: Team::Blue },
                0,
                SeenPose::default(),
            ),
        );
        if let Some(held) = arena.contacts.get_mut(EYE, id) {
            change(held);
        }
        tacsim::arena::hash::state_hash(&arena)
    };
    let none = plant(&|_| {});
    let green = Reading {
        at_ms: 1_000,
        condition: Condition::default().with_band(0, ConditionBand::Green),
    };
    let red = Reading {
        at_ms: 1_000,
        condition: Condition::default().with_band(0, ConditionBand::Red),
    };
    let later = Reading {
        at_ms: 2_000,
        ..green
    };

    // A reading where there was none, a different band, and a different stamp:
    // each is a different state, because each is a different thing a pilot would
    // be shown and none of the three writes an event of its own.
    let held_green = plant(&|held| held.seen_condition = Some(green));
    let held_red = plant(&|held| held.seen_condition = Some(red));
    let held_later = plant(&|held| held.seen_condition = Some(later));
    assert_ne!(held_green, none, "a reading went unhashed");
    assert_ne!(held_red, held_green, "the bands went unhashed");
    assert_ne!(held_later, held_green, "the look's stamp went unhashed");

    // And the own reading is folded apart from the held one: two platforms that
    // hold the same picture but may relay different things are two states.
    let own_green = plant(&|held| {
        if let Some(own) = held.own_seen.as_mut() {
            own.condition = Some(green);
        }
    });
    assert_ne!(own_green, none, "the own reading went unhashed");
    assert_ne!(own_green, held_green, "the two readings fold as one field");

    // The base arena holds no contact at all, which is a third state again.
    assert_ne!(tacsim::arena::hash::state_hash(&base), none);
}

// ---------------------------------------------------------------------------
// (q) every sentence `SCAN` says — 16b cut Q.2
//
// The three answers about a contact the platform **holds**. The fourth — the one
// byte-identical `No such contact.` for a contact it does not — is the verb's and
// is asserted through the real command table in `crates/tacsim/tests/cockpit.rs`,
// beside `RANGE`'s and `BEARING`'s, because the point of that one is that the
// three verbs cannot be told apart.
// ---------------------------------------------------------------------------

/// A real frame off a real pulse, with `EYE` watching `MARK` at `at`.
fn scan_frame(at: (u16, u16)) -> (Arena, Frame) {
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(at.0, at.1), None),
        ],
        &[],
    );
    let mut tape: Vec<u8> = Vec::new();
    let mut frame = None;
    for step in 1..=8_i64 {
        if let Some(sent) = pulse(&mut arena, step * PULSE_MS, &mut tape) {
            frame = Some(sent);
        }
    }
    let frame = frame.unwrap_or_else(|| panic!("the watcher was sent a frame"));
    (arena, frame)
}

/// The one row on that frame, as the cockpit renders it.
fn scan_row(frame: &Frame) -> cockpit::ContactRow {
    cockpit::frame_contacts(frame)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("the frame carries a row"))
}

#[test]
fn scan_of_an_unidentified_return_says_there_is_nothing_to_read() {
    // One pulse: the return is held and `Unknown`, which is a blip and not a
    // chassis. Section ids are a chassis vocabulary, so there is nothing to read
    // bands into — and the sentence says so rather than refusing, because the
    // pilot does hold the contact.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    let mut tape: Vec<u8> = Vec::new();
    // The opening pulse establishes the wall reference and integrates nothing;
    // the next one runs the pass that finds the return.
    let _ = arena.step(0, &[], &[]);
    let frame = pulse(&mut arena, PULSE_MS, &mut tape)
        .unwrap_or_else(|| panic!("the watcher was sent a frame"));
    let row = scan_row(&frame);
    assert_eq!(row.designator.to_string(), "X1");
    assert_eq!(
        cockpit::scan_lines(&frame, &row),
        vec!["X1 is not identified yet; there is nothing to read.".to_owned()]
    );
}

#[test]
fn scan_of_a_classified_contact_nobody_has_looked_at_says_to_close_the_range() {
    // Thirty cells: detected and classified, and outside the close look. The
    // sentence names the price, because a pilot who reads `No reading` has to be
    // told the answer is to drive closer.
    let (_, frame) = scan_frame(FAR);
    let row = scan_row(&frame);
    assert_eq!(row.designator.to_string(), "BA");
    assert_eq!(
        cockpit::scan_lines(&frame, &row),
        vec!["No reading on BA yet: close the range for a look.".to_owned()]
    );
}

#[test]
fn scan_of_a_contact_that_has_been_looked_at_prints_its_bands_and_the_looks_age() {
    let (_, frame) = scan_frame(NEAR);
    let row = scan_row(&frame);
    assert_eq!(row.designator.to_string(), "BA");
    let lines = cockpit::scan_lines(&frame, &row);
    // The fixture tank has one section, `hull`, and it is whole.
    assert_eq!(
        lines,
        vec![
            "Scan of BA, read 0 s ago:".to_owned(),
            "  hull green".to_owned()
        ]
    );
    // Never a number other than the age: no armour, no internals, no per mille.
    for line in lines.iter().skip(1) {
        assert!(
            !line.chars().any(|c| c.is_ascii_digit()),
            "a band line carried a number: {line}"
        );
    }
}

#[test]
fn a_scan_of_a_wide_chassis_wraps_at_the_damage_blocks_own_width() {
    // The stock tank has five sections and the extension biped seven, so nothing
    // the game ships fills a line. The rule still has to hold for the sixteen a
    // unit may declare, which is what this builds: a real frame, with its one
    // row's reading replaced by sixteen sections of long names.
    let (_, mut frame) = scan_frame(NEAR);
    let names: Vec<SmolStr> = (0..16)
        .map(|n| SmolStr::new(format!("left_forward_section_{n:02}")))
        .collect();
    let mut bands = Condition::default();
    for index in 0..16 {
        bands = bands.with_band(index, ConditionBand::Destroyed);
    }
    frame.scan_tables = vec![names.clone()];
    for row in &mut frame.contacts {
        row.condition = Some(tacsim::arena::perception::FrameCondition {
            table: 0,
            bands,
            age_ms: 12_000,
        });
    }
    let row = scan_row(&frame);
    let lines = cockpit::scan_lines(&frame, &row);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Scan of BA, read 12 s ago:")
    );
    let runs: Vec<&String> = lines.iter().skip(1).collect();
    assert!(
        runs.len() > 1,
        "sixteen long sections fit on one line: {runs:?}"
    );
    for run in &runs {
        assert!(
            run.len() <= 76,
            "a wrapped run ran past the damage block's width: {} ({run})",
            run.len()
        );
        assert!(run.starts_with("  "), "a run lost its indent: {run}");
    }
    // Every section is named exactly once, and every one of them is destroyed.
    let joined = runs
        .iter()
        .map(|run| run.as_str())
        .collect::<Vec<&str>>()
        .join(" ");
    for name in &names {
        assert_eq!(
            joined.matches(&format!("{name} destroyed")).count(),
            1,
            "{name} is not in the run exactly once: {joined}"
        );
    }
}

#[test]
fn a_reading_whose_table_is_not_on_the_frame_names_no_section() {
    // The index is handed out by the walk that builds the list, so the arena
    // cannot produce this — and the renderer answers with no sections rather
    // than with a guess, because a guessed chassis is the one thing the table's
    // whole shape exists to prevent.
    let (_, mut frame) = scan_frame(NEAR);
    for row in &mut frame.contacts {
        row.condition = row
            .condition
            .map(|reading| tacsim::arena::perception::FrameCondition {
                table: 9,
                ..reading
            });
    }
    let row = scan_row(&frame);
    assert_eq!(
        cockpit::scan_lines(&frame, &row),
        vec!["Scan of BA, read 0 s ago:".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// (r) the raw frame, on the wire — 16b cut Q.2
//
// The observer rule, read off the JSON a panel renderer decodes rather than off
// the sentence a line client prints: a row with no reading carries `condition:
// null`, and `scan_tables` lists no table for it — so a renderer cannot label a
// marker with a chassis the platform has not earned even if it wanted to.
// ---------------------------------------------------------------------------

/// The contact rows of a `status_frame` payload, as a panel would read them.
fn json_contacts(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_wire_frame_gives_an_unread_row_a_null_condition_and_no_table() {
    let match_id = tacsim::arena_task::MatchId(1);

    // (1) A return that is held and `Unknown`.
    let mut arena = arena_of(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(NEAR.0, NEAR.1), None),
        ],
        &[],
    );
    let mut tape: Vec<u8> = Vec::new();
    let _ = arena.step(0, &[], &[]);
    let frame = pulse(&mut arena, PULSE_MS, &mut tape)
        .unwrap_or_else(|| panic!("the watcher was sent a frame"));
    let payload = cockpit::status_frame(match_id, &frame);
    let rows = json_contacts(&payload);
    assert_eq!(rows.len(), 1);
    for row in &rows {
        assert_eq!(
            row.get("class").and_then(serde_json::Value::as_str),
            Some("unknown")
        );
        assert_eq!(
            row.get("condition"),
            Some(&serde_json::Value::Null),
            "an unresolved return published a reading: {row}"
        );
    }
    assert_eq!(
        payload.get("scan_tables"),
        Some(&serde_json::json!([])),
        "an unresolved return published a chassis: {payload}"
    );

    // (2) Classified, and outside the close look: still no reading, still no
    //     table. This is the row `scan_tables` exists for — building the list
    //     from the *classified* contacts would have published the chassis here.
    let (_, far) = scan_frame(FAR);
    let payload = cockpit::status_frame(match_id, &far);
    for row in json_contacts(&payload) {
        assert_eq!(
            row.get("class").and_then(serde_json::Value::as_str),
            Some("blue")
        );
        assert_eq!(row.get("condition"), Some(&serde_json::Value::Null));
    }
    assert_eq!(payload.get("scan_tables"), Some(&serde_json::json!([])));

    // (3) Classified and looked at: the reading, its table index, its compact
    //     band spelling and the age of the look — and exactly one table.
    let (_, near) = scan_frame(NEAR);
    let payload = cockpit::status_frame(match_id, &near);
    let rows = json_contacts(&payload);
    let read = rows
        .first()
        .and_then(|row| row.get("condition"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        read.get("table").and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        read.get("bands").and_then(serde_json::Value::as_str),
        Some("g"),
        "the fixture tank has one section and it is whole"
    );
    assert!(
        read.get("age_ms")
            .and_then(serde_json::Value::as_i64)
            .is_some(),
        "the look's age is on the wire: {read}"
    );
    assert_eq!(
        payload.get("scan_tables"),
        Some(&serde_json::json!([["hull"]])),
        "the one chassis the platform has read: {payload}"
    );
    // Still no exact figure anywhere on the row: doc 16 §3.6's coarse reading.
    for row in &rows {
        for key in ["armor", "armor_max", "internals", "internals_max"] {
            assert!(row.get(key).is_none(), "a contact row carried {key}: {row}");
        }
    }
}

// ---------------------------------------------------------------------------
// (q) what a contact is called when it flickers, and what a mesh re-arrival
//     costs the pilot in lines — 16b cut D.4 (`B-20`, `B-21`, `T-19`)
//
// Cut Q.1 made the name a function of the LIVE contact, which is right and has
// two rough edges the owner's play found:
//
// * a contact that had been told apart and comes straight back is silently
//   renamed from `BA` to `X2`, and `LOCK BA` then answers `No such contact.` for
//   three seconds with nothing to say why (`B-20`, 16c section 5 Q16);
// * a target this platform numbered in an EARLIER hold and now receives over the
//   mesh prints `New contact BA` and at once `Contact X2 identified: BA`, because
//   the `was` field read the permanent book instead of the hold in front of the
//   pilot (`B-21`).
//
// Both are read off the pilot's own lines here, through `Arena::step` and the one
// renderer, because what the cut is about is what a person reads.
// ---------------------------------------------------------------------------

/// A red teammate nearer to `EYE` than `MARK` is, so that `MARK` takes the
/// platform's **second** provisional number and the sentences below read `X2`.
const DECOY: ObjId = ObjId(7);

/// The three tanks the flicker is played out between. `EYE` is seat 0, so its
/// pilot is `ObjId(1_000)` and [`pulse`] hands back its frames.
fn flicker_units() -> Vec<CustodyUnit> {
    vec![
        tank(EYE, Team::Red, centre(10, 10), Some(visual())),
        tank(DECOY, Team::Red, centre(10, 14), None),
        tank(MARK, Team::Blue, centre(15, 10), None),
    ]
}

/// Every line `EYE`'s pilot was sent over one-second pulses `pulses`, with
/// whatever pose `at` names applied to `MARK` before each.
///
/// The placement is a function of arena time rather than a script of intents
/// because the property is about a *treeline*: what matters is that the contact
/// is out of reach for exactly three passes and inside it for the fourth, and
/// driving a tank there and back inside 3,000 ms is a fixture about acceleration.
fn flicker_lines(
    arena: &mut Arena,
    pulses: std::ops::RangeInclusive<i64>,
    at: impl Fn(i64) -> Option<Pose>,
) -> (Vec<String>, Option<Frame>) {
    let mut tape: Vec<u8> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut last: Option<Frame> = None;
    for n in pulses {
        if let Some(pose) = at(arena.now_ms) {
            place(arena, MARK, pose, 0);
        }
        if let Some(frame) = pulse(arena, PULSE_MS * n, &mut tape) {
            lines.extend(cockpit::notice_lines(&frame));
            last = Some(frame);
        }
    }
    (lines, last)
}

#[test]
fn a_classified_contact_that_flickers_back_to_a_number_says_so_once() {
    let mut arena = arena_of(flicker_units(), &[]);
    let _ = arena.step(0, &[] as &[Control], &[]);
    // EYE's passes fall at its seeded offset inside each second (4 under
    // `id % pass_ms` before 16b cut U.1), and the times below are
    // whole seconds plus that offset; `eye_offset` holds it inside the pulse.
    let offset = eye_offset(&arena);

    // Held from the first pass and told apart at 3,000 + offset; out of reach
    // through the pass that expires it (6,000 + offset) and back for the next
    // one (7,000 + offset), which is the hysteresis window the flicker rule
    // names.
    let (lines, _) = flicker_lines(&mut arena, 1..=12, |now| {
        if (3_500..6_500).contains(&now) {
            Some(away())
        } else if now >= 6_500 {
            Some(centre(15, 10))
        } else {
            None
        }
    });

    // The regression is said, once, under both names — the one the pilot has
    // been typing and the one the row reads now.
    assert_eq!(
        lines
            .iter()
            .filter(|line| *line == "BA is unidentified again: X2.")
            .count(),
        1,
        "the regression was not said exactly once: {lines:?}"
    );
    // And nothing was said about the loss or the re-acquisition themselves: the
    // flicker is still a flicker.
    assert!(
        !lines.iter().any(|line| line == "Contact BA lost."),
        "the cancelled loss was announced: {lines:?}"
    );
    let regression = lines
        .iter()
        .position(|line| line == "BA is unidentified again: X2.")
        .unwrap_or_else(|| panic!("the regression was never said: {lines:?}"));
    assert!(
        !lines
            .iter()
            .skip(regression)
            .any(|line| line.starts_with("New contact X2")),
        "the re-acquisition was announced: {lines:?}"
    );
    // The name comes back at the SECOND classification, `classify_after_ms`
    // after the re-detection at 7,000 + offset — and it is the same sentence cut
    // Q.1 writes for a first classification, because that is what it is.
    assert_eq!(
        lines
            .iter()
            .filter(|line| *line == "Contact X2 identified: BA, blue.")
            .count(),
        2,
        "once at {} and once at {}: {lines:?}",
        3_000 + offset,
        10_000 + offset
    );

    // **The old name is never an alias** (16c section 6). Between the two
    // classifications the row reads `X2` and nothing answers to `BA`, so
    // `LOCK BA` is the one byte-identical refusal: accepting it would tell the
    // pilot BA is still out there.
    let mut rewound = arena_of(flicker_units(), &[]);
    let _ = rewound.step(0, &[] as &[Control], &[]);
    let (_, held) = flicker_lines(&mut rewound, 1..=9, |now| {
        if (3_500..6_500).contains(&now) {
            Some(away())
        } else if now >= 6_500 {
            Some(centre(15, 10))
        } else {
            None
        }
    });
    let held = held.unwrap_or_else(|| panic!("the re-acquiring pulse sent a frame"));
    let rows = cockpit::frame_contacts(&held);
    assert!(
        rows.iter()
            .any(|row| row.designator == Designator::Provisional(2)),
        "the row does not read X2 at all: {rows:?}"
    );
    assert_eq!(
        cockpit::resolve_designator(&rows, "BA"),
        None,
        "the old name resolved: {rows:?}"
    );
    assert_eq!(
        cockpit::resolve_designator(&rows, "X2"),
        Some(draw_contact_id(rewound.seed, EYE, MARK)),
        "the number the row reads now does not resolve: {rows:?}"
    );
}

/// **16b cut B.1** (`T-86`): the old name the wire publishes is one the pilot was
/// sent — read off frames `Arena::step` staged, not frames a test wrote.
///
/// The same flicker as above, every frame published through
/// [`cockpit::status_frame`] the way the bridge publishes it. Each notice's `was`
/// must be a designator one of the pilot's EARLIER frames held as a row, and the
/// notice's `designator` the one the row reads on this frame; and the only
/// notices that carry a `was` at all are the renames the flicker makes: the red
/// teammate told apart (`X1` → `RB`, marked friendly — the case 16b cut B.1's
/// `(FRIENDLY)` fix in the web ring is for), `MARK`'s two classifications
/// (`X2` → `BA`) and the regression between them (`BA` → `X2`).
#[test]
fn b1_the_published_old_name_is_a_name_the_pilot_was_sent() {
    let mut arena = arena_of(flicker_units(), &[]);
    let _ = arena.step(0, &[] as &[Control], &[]);
    let mut tape: Vec<u8> = Vec::new();
    // Every designator an earlier frame showed this pilot as a row.
    let mut sent: Vec<String> = Vec::new();
    let mut quoted: Vec<(String, String, String, bool)> = Vec::new();
    for n in 1..=12 {
        let now = arena.now_ms;
        if (3_500..6_500).contains(&now) {
            place(&mut arena, MARK, away(), 0);
        } else if now >= 6_500 {
            place(&mut arena, MARK, centre(15, 10), 0);
        }
        let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) else {
            continue;
        };
        let rows: Vec<String> = cockpit::frame_contacts(&frame)
            .iter()
            .map(|row| row.designator.to_string())
            .collect();
        let payload = cockpit::status_frame(tacsim::arena_task::MatchId(1), &frame);
        for notice in payload["notices"].as_array().into_iter().flatten() {
            let Some(was) = notice["was"].as_str() else {
                assert!(
                    notice["was"].is_null(),
                    "`was` is a string or null: {notice}"
                );
                continue;
            };
            let kind = notice["kind"].as_str().unwrap_or_default();
            let designator = notice["designator"].as_str().unwrap_or_default();
            assert!(
                sent.iter().any(|name| name == was),
                "at {now} ms `{kind}` quotes {was}, which no earlier frame held ({sent:?}): \
                 {notice}"
            );
            assert!(
                rows.iter().any(|name| name == designator),
                "at {now} ms `{kind}` names {designator}, which this frame's rows do not \
                 read ({rows:?}): {notice}"
            );
            assert_ne!(was, designator, "a rename to the same name: {notice}");
            // A designator is a few letters and digits; the row key is sixteen.
            assert!(
                was.len() < 16 && was.chars().all(|c| c.is_ascii_alphanumeric()),
                "`was` is not a designator: {notice}"
            );
            quoted.push((
                kind.to_owned(),
                was.to_owned(),
                designator.to_owned(),
                notice["friendly"] == serde_json::Value::Bool(true),
            ));
        }
        sent.extend(rows);
    }
    let quoted: Vec<(&str, &str, &str, bool)> = quoted
        .iter()
        .map(|(kind, was, now, friendly)| (kind.as_str(), was.as_str(), now.as_str(), *friendly))
        .collect();
    assert_eq!(
        quoted,
        [
            ("classified", "X1", "RB", true),
            ("classified", "X2", "BA", false),
            ("unidentified", "BA", "X2", false),
            ("classified", "X2", "BA", false),
        ],
        "the flicker's renames, each quoting the name the pilot had"
    );
}

#[test]
fn a_flicker_that_keeps_its_name_is_silent_however_the_name_was_earned() {
    // (1) A contact that was never told apart: it reads `X2` before the loss and
    //     `X2` after it, so there is no regression to say and cut Q.1's silence
    //     is intact.
    let mut arena = arena_of(flicker_units(), &[]);
    let _ = arena.step(0, &[] as &[Control], &[]);
    let (lines, _) = flicker_lines(&mut arena, 1..=6, |now| {
        if (500..3_500).contains(&now) {
            Some(away())
        } else if now >= 3_500 {
            Some(centre(15, 10))
        } else {
            None
        }
    });
    assert!(
        !lines.iter().any(|line| line.contains("unidentified again")),
        "an X-row that flickered was announced: {lines:?}"
    );

    // (2) And the same flicker of a CLASSIFIED contact under
    //     `designate_on_detection`, where a row is called `BA` from the first
    //     pass: the name does not move, so nothing is said.
    let sensors = SensorsConfig {
        designate_on_detection: true,
        ..SensorsConfig::default()
    };
    let mut named = arena_with_sensors(flicker_units(), &[], &sensors);
    let _ = named.step(0, &[] as &[Control], &[]);
    let (lines, _) = flicker_lines(&mut named, 1..=9, |now| {
        if (3_500..6_500).contains(&now) {
            Some(away())
        } else if now >= 6_500 {
            Some(centre(15, 10))
        } else {
            None
        }
    });
    assert!(
        !lines.iter().any(|line| line.contains("unidentified again")),
        "the switch's own rows are never renamed: {lines:?}"
    );
}

#[test]
fn a_loss_that_outlasts_the_window_is_still_a_loss_and_then_a_new_contact() {
    let mut arena = arena_of(flicker_units(), &[]);
    let _ = arena.step(0, &[] as &[Control], &[]);
    // Away far longer than the window, so the held loss is swept and said, and
    // the return is an acquisition of a row nothing remembers.
    let (lines, _) = flicker_lines(&mut arena, 1..=16, |now| {
        if (3_500..12_500).contains(&now) {
            Some(away())
        } else if now >= 12_500 {
            Some(centre(15, 10))
        } else {
            None
        }
    });
    let loss = lines
        .iter()
        .position(|line| line == "Contact BA lost.")
        .unwrap_or_else(|| panic!("the loss was never said: {lines:?}"));
    // The LAST of them: the row was acquired once at the opening pass too, and
    // what this property is about is the acquisition after the loss.
    let again = lines
        .iter()
        .rposition(|line| line.starts_with("New contact X2:"))
        .unwrap_or_else(|| panic!("the return was never said: {lines:?}"));
    assert!(loss < again, "out of order: {lines:?}");
    assert!(
        !lines.iter().any(|line| line.contains("unidentified again")),
        "a loss past the window said the regression as well: {lines:?}"
    );
}

/// **The mesh fixture** (16b cut D.4, `B-21` and `T-19`).
///
/// `EYE` is slow to tell a return apart (ten seconds against `MATE`'s three) and
/// sees far; `MATE` stands between the two and sees twenty-five cells. So the
/// share is what classifies `EYE`'s **own** hold in the opening seconds, and a
/// later re-arrival over the same mesh is a row `EYE`'s own sensors never opened.
fn mesh_rename_arena() -> Arena {
    arena_of(
        vec![
            tank(
                EYE,
                Team::Red,
                centre(10, 10),
                Some(SensorProfile {
                    classify_after_ms: 10_000,
                    ..visual()
                }),
            ),
            tank(
                MATE,
                Team::Red,
                centre(20, 10),
                Some(SensorProfile {
                    max_range_cells: 25,
                    ..visual()
                }),
            ),
            tank(MARK, Team::Blue, centre(42, 10), None),
        ],
        &[],
    )
}

/// Whether the hold `viewer` has on `target` was opened under a number.
fn numbered(arena: &Arena, viewer: ObjId, target: ObjId) -> Option<bool> {
    let id = draw_contact_id(arena.seed, viewer, target);
    arena.contacts.get(viewer, id).map(|held| held.numbered)
}

#[test]
fn a_share_renames_a_hold_this_platform_opened_and_a_mesh_arrival_is_one_line() {
    let mut arena = mesh_rename_arena();
    let _ = arena.step(0, &[] as &[Control], &[]);
    let mut tape: Vec<u8> = Vec::new();
    let mut said: Vec<String> = Vec::new();
    let mut acquired_class: Vec<Classification> = Vec::new();

    // ---- phase 1: this platform's own sight opens the hold, the mesh names it
    for n in 1..=5 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            said.extend(cockpit::notice_lines(&frame));
        }
    }
    assert_eq!(
        numbered(&arena, EYE, MARK),
        Some(true),
        "this platform's own sensors opened the hold"
    );
    assert!(
        said.contains(&"Contact X1 identified: BA, blue.".to_owned()),
        "a share that classified this platform's own row said no rename: {said:?}"
    );

    // ---- phase 2: everybody loses it, well past the hysteresis window
    place(&mut arena, MARK, away(), 0);
    for n in 6..=14 {
        let _ = pulse(&mut arena, PULSE_MS * n, &mut tape);
    }
    assert_eq!(numbered(&arena, EYE, MARK), None, "the row never expired");

    // ---- phase 3: it comes back where only `MATE` can see it
    if let Some(state) = arena.unit_mut(EYE) {
        state.sensor = Some(SensorProfile {
            max_range_cells: 15,
            classify_after_ms: 10_000,
            ..visual()
        });
    }
    place(&mut arena, MARK, centre(42, 10), 0);
    let mut arrival: Vec<String> = Vec::new();
    for n in 15..=25 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            arrival.extend(cockpit::notice_lines(&frame));
            for notice in &frame.notices {
                if let FrameNotice::Contact {
                    kind: NoticeKind::Acquired,
                    contact,
                    class,
                    ..
                } = notice
                    && contact.id == draw_contact_id(arena.seed, EYE, MARK)
                {
                    acquired_class.push(*class);
                }
            }
        }
    }

    // **`B-21`**: the copy arrived classified, so the pilot reads one line.
    assert_eq!(
        numbered(&arena, EYE, MARK),
        Some(false),
        "a mesh insert was marked as numbered"
    );
    assert_eq!(
        arrival
            .iter()
            .filter(|line| line.starts_with("New contact BA"))
            .count(),
        1,
        "the mesh arrival was not one line: {arrival:?}"
    );
    assert!(
        !arrival.iter().any(|line| line.contains("identified:")),
        "the mesh arrival was announced as a rename of a row nobody is looking at: {arrival:?}"
    );
    // The book still holds the number from the earlier hold — which is exactly
    // why the `was` may not be read off it.
    assert_eq!(arena.designators.provisional(EYE, MARK), Some(1));

    // **`T-19`**: and the acquisition carries the class the row actually has.
    assert_eq!(
        acquired_class,
        vec![Classification::Classified { team: Team::Blue }],
        "a mesh arrival's acquisition said `unknown` about a row already told apart"
    );
}

/// **The other direction of the same cancel, and it is silent** (16b cut D.4,
/// `B-20`).
///
/// `B-20`'s line is staged where a held loss is cancelled, and a cancel has two
/// directions. [`Arena::share_pass`] journals a `detected` for a row it inserts
/// already `Classified`, keyed on the same contact id the loss was keyed on, so a
/// mesh copy landing inside the hysteresis window cancels the loss with the live
/// row reading `BA` where the loss carried `X1`. The **name moved** — the wrong
/// way. A contact that has just been told apart is not `unidentified again`, so
/// the arm is gated on the live row still being `Unknown` as well as on the name,
/// and this pilot reads no regression at all.
///
/// The fixture is the stagger itself: passes are scheduled at
/// `stagger_ms(seed, id, pass_ms)` (`Arena::schedule_first_passes`; `id %
/// pass_ms` until 16b cut U.1), so this test's teammate is `#79`, picked so that
/// it passes seven milliseconds after `EYE` (489 against 482 at seed 2,350),
/// and its share lands well inside the one-pass window `EYE`'s expiry opened.
/// That `Contact X1 lost.` is never said is the proof the window really was
/// open when the copy arrived.
#[test]
fn a_mesh_share_inside_the_window_cancels_the_loss_and_says_no_regression() {
    // 16b cut U.1: re-picked for the seeded stagger, and asserted below.
    const MATE: ObjId = ObjId(79);
    let mut arena = arena_of(
        vec![
            // Sees eight cells and never tells a return apart on its own, so its
            // hold on `MARK` is an `X1` for the whole of that hold's life.
            tank(
                EYE,
                Team::Red,
                centre(10, 10),
                Some(SensorProfile {
                    max_range_cells: 8,
                    classify_after_ms: 10_000_000,
                    ..visual()
                }),
            ),
            // A relay and not an eye: one cell of sight, so it detects nothing of
            // its own and cannot classify `EYE`'s row in place before the loss —
            // the mesh range is the sight's `share_range_cells`, which is why it
            // carries a profile at all. It stands fifteen cells off, outside
            // `EYE`'s own eight, so the only number `EYE` ever issues is `MARK`'s.
            tank(
                MATE,
                Team::Red,
                centre(25, 10),
                Some(SensorProfile {
                    max_range_cells: 1,
                    ..visual()
                }),
            ),
            tank(MARK, Team::Blue, centre(15, 10), None),
        ],
        &[],
    );
    // The precondition the story is built on: `MATE` passes after `EYE`, inside
    // the same one-second pulse.
    let eye = eye_offset(&arena);
    let mate = stagger_ms(arena.seed, MATE, i64::from(arena.sensors_cfg.pass_ms));
    assert!(
        eye < mate && mate < PULSE_MS,
        "the precondition: MATE's pass ({mate}) is after EYE's ({eye}) in one pulse"
    );
    let _ = arena.step(0, &[] as &[Control], &[]);
    let mut tape: Vec<u8> = Vec::new();
    let mut said: Vec<String> = Vec::new();

    // ---- this platform's own sight opens the hold, and it stays a number
    for n in 1..=4 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            said.extend(cockpit::notice_lines(&frame));
        }
    }
    assert_eq!(
        numbered(&arena, EYE, MARK),
        Some(true),
        "this platform's own sensors opened the hold"
    );
    assert!(
        said.iter().any(|line| line.starts_with("New contact X1:")),
        "the number was never on the pilot's scope: {said:?}"
    );

    // ---- it goes away, and `EYE`'s hold runs out its three passes
    place(&mut arena, MARK, away(), 0);
    for n in 5..=6 {
        if let Some(frame) = pulse(&mut arena, PULSE_MS * n, &mut tape) {
            said.extend(cockpit::notice_lines(&frame));
        }
    }
    assert!(
        numbered(&arena, EYE, MARK).is_some(),
        "the hold expired a pulse early, so the plant below is outside the window"
    );

    // ---- the teammate's classified copy, planted a pulse before it is shared
    //
    // The hand is the fixture's: what the property is about is what the *engine*
    // does with a share that lands inside the window, and a legitimate
    // classification here would have been shared a pass earlier and renamed
    // `EYE`'s own row before it was ever lost.
    let theirs = draw_contact_id(arena.seed, MATE, MARK);
    arena.contacts.insert(
        MATE,
        theirs,
        tacsim::arena::state::ShadowContact::sighted(
            MARK,
            Classification::Classified { team: Team::Blue },
            6_000,
            SeenPose::default(),
        ),
    );

    // ---- one pulse carries both: `EYE` expires at 6,482 and the share lands at
    // 6,489, seven milliseconds inside the thousand the window is open for.
    let mut after: Vec<String> = Vec::new();
    let mut staged: Vec<FrameNotice> = Vec::new();
    if let Some(frame) = pulse(&mut arena, PULSE_MS * 7, &mut tape) {
        after.extend(cockpit::notice_lines(&frame));
        staged.extend(frame.notices.iter().cloned());
    }
    // 16b cut M.2 (`B-74`): the notice itself, as staged — an identification
    // quoting the number the pilot had, first, and no regression. (The mesh
    // insert also journals its own `classified`, whose notice carries no `was`
    // because the row was never provisional in this hold, and renders as
    // nothing: the one rendered line asserted below is all the pilot reads.)
    assert_eq!(
        staged.first().cloned(),
        Some(FrameNotice::Contact {
            kind: NoticeKind::Classified,
            contact: tacsim::arena::perception::Named {
                id: draw_contact_id(arena.seed, EYE, MARK),
                designator: Designator::Slot {
                    team: Team::Blue,
                    slot: 1,
                },
            },
            class: Classification::Classified { team: Team::Blue },
            was: Some(Designator::Provisional(1)),
        }),
        "the cancel said something other than the rename: {staged:?}"
    );
    assert!(
        !staged.iter().any(|notice| matches!(
            notice,
            FrameNotice::Contact {
                kind: NoticeKind::Unidentified,
                ..
            }
        )),
        "a regression was staged"
    );
    assert_eq!(
        numbered(&arena, EYE, MARK),
        Some(false),
        "the row `EYE` holds now is not the mesh's copy, so the fixture missed its window"
    );
    assert!(
        !after.iter().any(|line| line.contains("unidentified again")),
        "a contact that had just been told apart was called unidentified: {after:?}"
    );
    assert!(
        !after.iter().any(|line| line.contains("lost")),
        "the loss was said, so the share landed outside the window and this fixture \
         proves nothing about the cancel: {after:?}"
    );
    // What the pilot reads for the whole exchange, in full: the cancel is a
    // cancel, so the acquisition is not said either, exactly as it was before this
    // cut — but **16b cut M.2** (16c `B-74`) says the rename. D.4 left it unsaid
    // (this assertion read `after.is_empty()` until that cut), and the pilot's
    // column went from `X1` to `BA` with no line; the cancel arm now stages the
    // `classified` notice with the old name, the sentence a share that
    // classifies a still-held row says.
    assert_eq!(
        after,
        vec!["Contact X1 identified: BA, blue.".to_owned()],
        "the renamed row was not said as one identification: {after:?}"
    );
    // And the column reads the live name alone: the number is gone with the hold
    // that carried it, and `EYE` never learned an alias either way.
    let column: Vec<String> = arena
        .frame_contacts(EYE)
        .into_iter()
        .map(|row| row.designator.to_string())
        .collect();
    assert!(column.contains(&"BA".to_owned()), "the column: {column:?}");
    assert!(!column.contains(&"X1".to_owned()), "the column: {column:?}");
}

// ---------------------------------------------------------------------------
// (r) the flicker at three cadences — 16b cut D.4's mandatory cross-tempo test
//
// 16b section 11 makes this mandatory for every cut that changes arena
// behaviour, and D.4 changes three things the property is about: what a pilot is
// told when a classified contact comes back a number (`B-20`), whether the hold
// was opened under one (`B-21`, folded into the state hash) and the class an
// acquisition carries (`T-19`).
//
// The cadence is five seconds a pass and one pass of loss band, so that the
// whole flicker — detect, classify, expire, re-detect inside the hysteresis
// window — happens between beats that are multiples of five seconds and
// therefore land at one arena millisecond at 5,000, 1,000 and 500 ms alike.
// ---------------------------------------------------------------------------

/// `EYE`'s offset on the five-second pass (16b cut U.1), asserted strictly inside
/// the five seconds between two beats, which is what puts every one of its
/// passes between the beats the (r) and (s) stories are told on.
fn flicker_offset() -> i64 {
    let offset = stagger_ms(2_350, EYE, i64::from(flicker_sensors().pass_ms));
    assert!(
        offset > 0 && offset < 5_000,
        "the precondition: EYE's pass is between the beats ({offset})"
    );
    offset
}

/// A five-second pass and a one-pass loss band (16b cut D.4), so the whole
/// flicker fits between two beats the three cadences agree on.
fn flicker_sensors() -> SensorsConfig {
    SensorsConfig {
        pass_ms: 5_000,
        lost_after_passes: 1,
        ..SensorsConfig::default()
    }
}

/// `EYE` sees ten cells and takes a whole pass to tell a return apart; `MARK`
/// stands five cells east of it, which is inside that reach.
fn flicker_replay_arena() -> Arena {
    arena_with_sensors(
        vec![
            tank(
                EYE,
                Team::Red,
                centre(10, 10),
                Some(SensorProfile {
                    max_range_cells: 10,
                    classify_after_ms: 5_000,
                    ..visual()
                }),
            ),
            tank(MARK, Team::Blue, centre(15, 10), None),
        ],
        &[],
        &flicker_sensors(),
    )
}

/// Out of every reach at ten seconds, back at fifteen: `EYE`'s five-second
/// pass falls at 2,482 and every five seconds after it (16b cut U.1:
/// `stagger_ms(2_350, EYE, 5_000)`; it was 4), so the pass at 12,482 expires a
/// contact last seen at 7,482, and the pass at 17,482 re-acquires it inside the
/// one-pass hysteresis window that ends at 17,482.
fn flicker_replay_script() -> Vec<Beat> {
    vec![
        Beat {
            t_ms: 10_000,
            intents: Vec::new(),
            places: vec![(MARK, away())],
        },
        Beat {
            t_ms: 15_000,
            intents: Vec::new(),
            places: vec![(MARK, centre(15, 10))],
        },
    ]
}

#[test]
fn one_scripted_flicker_at_three_cadences_is_one_list_one_hash_and_one_column() {
    let script = flicker_replay_script();
    let mut runs: Vec<(i64, Cadenced)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        runs.push((
            cadence,
            replayed(flicker_replay_arena(), &script, cadence, 25_000),
        ));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: the pilot really did read the whole story.
    let told = first
        .notices
        .get(&ObjId(1_000))
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        told,
        vec![
            "New contact X1: bearing 090°, effective.".to_owned(),
            "Contact X1 identified: BA, blue.".to_owned(),
            "BA is unidentified again: X1.".to_owned(),
            "Contact X1 identified: BA, blue.".to_owned(),
        ],
        "the flicker's own four lines, in order"
    );
    // And the black box has the loss the pilot was never told about, at the
    // first of `EYE`'s passes after the ten-second beat.
    let offset = flicker_offset();
    assert_eq!(
        first
            .events
            .iter()
            .filter(|(_, json, _, _)| json.contains("\"kind\":\"lost\""))
            .map(|(t_ms, _, _, _)| *t_ms)
            .collect::<Vec<i64>>(),
        vec![10_000 + offset],
        "the expiry is recorded at its own arena millisecond"
    );
    // The designator column really moved: `X1`, the slot name, nothing, `X1`.
    let watcher: Vec<Vec<String>> = first
        .columns
        .iter()
        .filter(|(_, platform, _)| *platform == EYE)
        .map(|(_, _, column)| column.clone())
        .collect();
    assert!(
        watcher.iter().any(|column| column == &["BA".to_owned()]),
        "the scope never read the slot name: {watcher:?}"
    );
    assert!(
        watcher.iter().any(Vec::is_empty),
        "the contact was never lost: {watcher:?}"
    );

    // The guard: three cadences really are three different numbers of pulses.
    let counts: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair.first() != pair.get(1)),
        "the three replays ran the same number of pulses: {counts:?}"
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the events differ at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hashes differ at {cadence} ms"
        );
        // 16b cut M.2 (`T-84`): what was staged, not what it rendered as.
        assert_eq!(
            run.staged, first.staged,
            "the pilots' staged notices differ at {cadence} ms"
        );
        assert_eq!(
            run.columns, first.columns,
            "the designator column differs at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (s) the end of the hysteresis at three cadences — 16b cut M.2 (16c `T-83`,
//     `T-84`)
//
// Until this cut a sweep at the end of every pulse said the held losses whose
// window had run out, so the end of the hysteresis was a pulse boundary: a
// contact re-detected after its deadline was cancelled at a cadence whose
// pulse held both the deadline and the re-detection, and said lost and then
// new at a cadence whose boundary fell between them. The expiry is a
// `DueKind::LossSettles` at the deadline's own millisecond now, so the story
// is one story at every cadence — and each test below compares what was
// STAGED (`Cadenced::staged`), not what it rendered as.
//
// **Which of them is the regression proof.** Only the late relay's share (a
// return 266 ms past the deadline, inside one 1,000 ms pulse and across a
// 500 ms boundary) told two stories under the old sweep; the return one whole
// pass after the deadline, at 22,482, was said lost first at every cadence
// then as now, and pins the rule rather than the fix.
//
// All but the last two run on the (r) fixture: a five-second pass and a
// one-pass loss band, `MARK` lost at 12,482 and its window ending at 17,482
// (16b cut U.1's seeded stagger moved `EYE`'s pass from 4 to 2,482 of each
// five seconds; the relays below were re-picked to keep their stories, each
// asserted against `EYE`'s offset).
// ---------------------------------------------------------------------------

/// The kinds of every contact notice `EYE`'s pilot was staged, in order.
fn eye_contact_kinds(run: &Cadenced) -> Vec<NoticeKind> {
    run.staged
        .get(&ObjId(1_000))
        .into_iter()
        .flatten()
        .filter_map(|notice| match notice {
            FrameNotice::Contact { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect()
}

/// Replay `script` on `arena()` at 5,000, 1,000 and 500 ms, assert the three
/// runs are one event list, one hash sequence and one staged story, and hand
/// back the first.
fn three_cadences(arena: impl Fn() -> Arena, script: &[Beat], until_ms: i64) -> Cadenced {
    let mut runs: Vec<(i64, Cadenced)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        runs.push((cadence, replayed(arena(), script, cadence, until_ms)));
    }
    let counts: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair.first() != pair.get(1)),
        "the three replays ran the same number of pulses: {counts:?}"
    );
    let mut runs = runs.into_iter();
    let Some((_, first)) = runs.next() else {
        panic!("three replays");
    };
    for (cadence, run) in runs {
        assert_eq!(
            run.events, first.events,
            "the events differ at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hashes differ at {cadence} ms"
        );
        assert_eq!(
            run.staged, first.staged,
            "the staged notices differ at {cadence} ms"
        );
    }
    first
}

/// **Re-detected one pass after the deadline** (16c `T-83`): the loss is said
/// at the deadline, the return is a new contact, and both are said at every
/// cadence.
#[test]
fn m2_a_return_one_pass_after_the_deadline_is_a_loss_then_a_contact_at_every_cadence() {
    let script = vec![
        Beat {
            t_ms: 10_000,
            intents: Vec::new(),
            places: vec![(MARK, away())],
        },
        // The pass at 17,482 is the deadline's and finds nothing; the one at
        // 22,482 finds it.
        Beat {
            t_ms: 20_000,
            intents: Vec::new(),
            places: vec![(MARK, centre(15, 10))],
        },
    ];
    let first = three_cadences(flicker_replay_arena, &script, 30_000);
    let kinds = eye_contact_kinds(&first);
    let lost = kinds
        .iter()
        .position(|kind| *kind == NoticeKind::Lost)
        .unwrap_or_else(|| panic!("the loss was never said: {kinds:?}"));
    assert!(
        kinds
            .iter()
            .skip(lost)
            .any(|kind| *kind == NoticeKind::Acquired),
        "the return was not said after the loss: {kinds:?}"
    );
    assert!(
        !kinds.contains(&NoticeKind::Unidentified),
        "a return past the window said the regression: {kinds:?}"
    );
}

/// **Re-detected on the deadline itself** (16c `T-83`): the platform's own pass
/// at 17,482 is applied before the expiry (it was scheduled a pass earlier, so
/// its sequence is lower), so the loss is cancelled — no `Lost`, and no second
/// `Acquired` — at every cadence. This is the (r) fixture's own script, read on
/// what was staged.
#[test]
fn m2_a_return_on_the_deadline_is_cancelled_at_every_cadence() {
    let first = three_cadences(flicker_replay_arena, &flicker_replay_script(), 25_000);
    let kinds = eye_contact_kinds(&first);
    assert!(
        !kinds.contains(&NoticeKind::Lost),
        "a loss re-detected on its deadline was said: {kinds:?}"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == NoticeKind::Acquired)
            .count(),
        1,
        "the return was said as a new contact: {kinds:?}"
    );
}

/// A red relay for the mesh cases below: sees ten cells, tells a return apart
/// in a second, and starts seventy cells away from `EYE`, out of every reach.
fn relay(id: ObjId) -> CustodyUnit {
    tank(
        id,
        Team::Red,
        centre(60, 60),
        Some(SensorProfile {
            max_range_cells: 10,
            classify_after_ms: 1_000,
            ..visual()
        }),
    )
}

/// The (r) fixture with a relay whose passes fall at `stagger_ms(2_350, id,
/// 5_000)` (16b cut U.1; `id % 5_000` before it).
fn relayed_arena(relay_id: ObjId) -> Arena {
    arena_with_sensors(
        vec![
            tank(
                EYE,
                Team::Red,
                centre(10, 10),
                Some(SensorProfile {
                    max_range_cells: 10,
                    classify_after_ms: 5_000,
                    ..visual()
                }),
            ),
            tank(MARK, Team::Blue, centre(15, 10), None),
            relay(relay_id),
        ],
        &[],
        &flicker_sensors(),
    )
}

/// `MARK` goes where only the relay sees it at ten seconds, and at fifteen the
/// relay and `MARK` come within the mesh's reach of `EYE` — but never within
/// `EYE`'s own sight — so the only way `EYE` can hold `MARK` again is a share.
fn relayed_script(relay_id: ObjId) -> Vec<Beat> {
    vec![
        Beat {
            t_ms: 10_000,
            intents: Vec::new(),
            places: vec![(MARK, centre(60, 55))],
        },
        Beat {
            t_ms: 15_000,
            intents: Vec::new(),
            places: vec![(relay_id, centre(30, 10)), (MARK, centre(35, 10))],
        },
    ]
}

/// **A mesh share landing exactly on the deadline** (16c `T-83`, the one
/// millisecond the two arms can differ on). The relay's id is 7,156, picked so
/// that its stagger collides with `EYE`'s (16b cut U.1; it was 5,004 under `id %
/// 5_000`), so its pass falls at 17,482 with `EYE`'s own; the share it schedules
/// there is sequenced AFTER the `LossSettles` that `EYE`'s loss scheduled at
/// 12,482, so the loss is said first and the share is then a new contact — the
/// same story at every cadence.
#[test]
fn m2_a_share_landing_on_the_deadline_is_one_story_at_every_cadence() {
    const RELAY: ObjId = ObjId(7_156);
    let offset = flicker_offset();
    assert_eq!(
        stagger_ms(2_350, RELAY, 5_000),
        offset,
        "the precondition: the relay's pass collides with EYE's"
    );
    let first = three_cadences(|| relayed_arena(RELAY), &relayed_script(RELAY), 25_000);
    let kinds = eye_contact_kinds(&first);
    let lost = kinds
        .iter()
        .position(|kind| *kind == NoticeKind::Lost)
        .unwrap_or_else(|| panic!("the loss was never said: {kinds:?}"));
    assert!(
        kinds
            .iter()
            .skip(lost)
            .any(|kind| *kind == NoticeKind::Acquired),
        "the share was not said as a contact after the loss: {kinds:?}"
    );
    // And the share really landed on the deadline's own millisecond.
    let shared_at: Vec<i64> = first
        .events
        .iter()
        .filter(|(_, json, _, _)| {
            json.contains("\"kind\":\"detected\"")
                && json.contains(&format!("\"viewer\":{}", EYE.0))
        })
        .map(|(t_ms, _, _, _)| *t_ms)
        .collect();
    assert!(
        shared_at.contains(&(15_000 + offset)),
        "the share did not land on the deadline: {shared_at:?}"
    );
}

/// **A mesh share landing after the deadline** — the case the pulse-end sweep
/// split by tempo, and the one test in this section that fails on the old
/// sweep. At 5,000 and 1,000 ms the share at 17,748 falls in the same pulse as
/// the deadline at 17,482 (15,000 to 20,000, and 17,000 to 18,000) and would
/// cancel the loss before a pulse-end sweep said it; at 500 ms a sweep at 17,500
/// would say it first and the share would be a new contact. The due event says
/// it at the deadline at every cadence. The relay's id is 51, picked for the
/// seeded stagger (16b cut U.1; it was 604, whose share fell at 15,604 against
/// a deadline at 15,004) and asserted: its pass is after `EYE`'s, past the next
/// 500 ms boundary and inside the same second.
#[test]
fn m2_a_share_landing_after_the_deadline_is_a_loss_then_a_contact_at_every_cadence() {
    const RELAY: ObjId = ObjId(51);
    let offset = flicker_offset();
    let relay_at = stagger_ms(2_350, RELAY, 5_000);
    let boundary = (offset / 500 + 1) * 500;
    assert!(
        relay_at > boundary && relay_at / 1_000 == offset / 1_000,
        "the precondition: the relay's pass ({relay_at}) is across a 500 ms boundary from \
         EYE's ({offset}) and inside its second"
    );
    let first = three_cadences(|| relayed_arena(RELAY), &relayed_script(RELAY), 25_000);
    let kinds = eye_contact_kinds(&first);
    let lost = kinds
        .iter()
        .position(|kind| *kind == NoticeKind::Lost)
        .unwrap_or_else(|| panic!("the loss was never said: {kinds:?}"));
    assert!(
        kinds
            .iter()
            .skip(lost)
            .any(|kind| *kind == NoticeKind::Acquired),
        "the share was not said as a contact after the loss: {kinds:?}"
    );
}

/// **A moving observer** (16c `T-84`): the (r) flicker with `EYE` started up
/// and driven north while it happens. A rendered line reads its bearing and its
/// band off the frame of the pulse it arrived in, so with the observer moving
/// the rendered lines are a function of the cadence; what was STAGED is not,
/// and that is the comparison `three_cadences` makes.
#[test]
fn m2_a_moving_observer_stages_one_story_at_every_cadence() {
    let mut script = vec![
        Beat {
            t_ms: 0,
            intents: vec![intent(EYE, IntentKind::Startup, 1)],
            places: Vec::new(),
        },
        // Online at 10,000 (the ten-second startup): creep north at two metres a
        // second, slow enough that `MARK` is still in reach when it comes back.
        Beat {
            t_ms: 10_000,
            intents: vec![intent(EYE, IntentKind::Speed { cms: 200 }, 2)],
            places: Vec::new(),
        },
    ];
    // The flicker itself, merged into the same two beats' timeline.
    for beat in flicker_replay_script() {
        if let Some(same) = script.iter_mut().find(|mine| mine.t_ms == beat.t_ms) {
            same.places.extend(beat.places);
        } else {
            script.push(beat);
        }
    }
    script.sort_by_key(|beat| beat.t_ms);
    let first = three_cadences(flicker_replay_arena, &script, 25_000);
    // The run is worth comparing: the observer really moved, and the pilot was
    // told about the contact.
    let kinds = eye_contact_kinds(&first);
    assert!(
        kinds.contains(&NoticeKind::Acquired) && kinds.contains(&NoticeKind::Classified),
        "the moving observer was told nothing: {kinds:?}"
    );
    assert!(
        first
            .events
            .iter()
            .any(|(_, json, _, _)| json.contains("\"kind\":\"power_online\"")),
        "the observer never came online, so it never moved"
    );
    // **And it moved** (the M.2 review's `M2T-7`): `EYE`'s northing off the
    // black box's own position summaries, first against last. A `Speed` refused
    // on its beat would leave it standing, and this test would be the static
    // (r) fixture again.
    let northing: Vec<(i64, i64)> = first
        .events
        .iter()
        .filter_map(
            |(t_ms, json, _, _)| match serde_json::from_str::<EventKind>(json) {
                Ok(EventKind::PositionSummary { units }) => units
                    .iter()
                    .find(|(unit, ..)| *unit == EYE)
                    .map(|(_, _, y_cm, _, _)| (*t_ms, *y_cm)),
                _ => None,
            },
        )
        .collect();
    let (Some((_, from)), Some((until, to))) = (northing.first(), northing.last()) else {
        panic!("no position summary carried EYE");
    };
    // Two metres a second from ten seconds on, less the time to get up to it.
    assert!(
        to - from >= 2_000,
        "EYE crept {} cm north by {until}: it never really moved ({northing:?})",
        to - from
    );
}

/// **A cancel that renames the row, at every cadence** (16c `B-74`, the M.2
/// review's `M2T-5`). `EYE` numbers `MARK` `X1` at 2,482 and, taking ten
/// seconds to tell a contact apart, still holds it as `X1` when `MARK` goes out
/// of its sight at ten seconds: lost at 12,482, the window ending at 17,482. A
/// red relay, far out of the mesh's reach, sees `MARK` at its pass at 12,481;
/// at fifteen seconds the relay is put twenty cells east of `EYE`, inside the
/// mesh's reach, with `MARK` five cells beyond it and still out of `EYE`'s own
/// sight, and the relay's pass at 17,481 tells `MARK` apart and shares it —
/// one millisecond inside the window. (16b cut U.1: the times were 4, 10,004,
/// 15,004 and a relay at 10,003 and 15,003 under `id % 5_000`; the relay is
/// re-picked, `#9627`, whose stagger is `EYE`'s less one, and asserted.) So the cancel arm meets a mesh insert
/// that reads `BA` where the loss read `X1`, and says so as one identification:
/// the same staged story at 5,000, 1,000 and 500 ms, with no loss, no second
/// acquisition and no regression.
#[test]
fn m2_a_share_that_renames_a_held_loss_is_one_identification_at_every_cadence() {
    const RELAY: ObjId = ObjId(9_627);
    assert_eq!(
        stagger_ms(2_350, RELAY, 5_000),
        flicker_offset() - 1,
        "the precondition: the relay passes one millisecond before EYE"
    );
    let arena = || {
        arena_with_sensors(
            vec![
                tank(
                    EYE,
                    Team::Red,
                    centre(10, 10),
                    Some(SensorProfile {
                        max_range_cells: 10,
                        classify_after_ms: 10_000,
                        ..visual()
                    }),
                ),
                tank(MARK, Team::Blue, centre(15, 10), None),
                relay(RELAY),
            ],
            &[],
            &flicker_sensors(),
        )
    };
    let script = vec![
        Beat {
            t_ms: 10_000,
            intents: Vec::new(),
            places: vec![(MARK, centre(60, 55))],
        },
        Beat {
            t_ms: 15_000,
            intents: Vec::new(),
            places: vec![(RELAY, centre(30, 10)), (MARK, centre(35, 10))],
        },
    ];
    let first = three_cadences(arena, &script, 20_000);
    let kinds = eye_contact_kinds(&first);
    for never in [NoticeKind::Lost, NoticeKind::Unidentified] {
        assert!(
            !kinds.contains(&never),
            "the cancel said {never:?}: {kinds:?}"
        );
    }
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == NoticeKind::Acquired)
            .count(),
        1,
        "the return was said as a new contact: {kinds:?}"
    );
    let renamed: Vec<&FrameNotice> = first
        .staged
        .get(&ObjId(1_000))
        .into_iter()
        .flatten()
        .filter(|notice| {
            matches!(
                notice,
                FrameNotice::Contact {
                    kind: NoticeKind::Classified,
                    was: Some(_),
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        renamed.len(),
        1,
        "the rename was not said exactly once: {:?}",
        first.staged.get(&ObjId(1_000))
    );
    assert!(
        matches!(
            renamed.first(),
            Some(FrameNotice::Contact {
                class: Classification::Classified { team: Team::Blue },
                was: Some(Designator::Provisional(1)),
                ..
            })
        ),
        "the identification did not quote X1: {renamed:?}"
    );
    // The share really landed inside the window, and the pilot read the rename.
    assert!(
        first.notices.get(&ObjId(1_000)).is_some_and(|lines| lines
            .iter()
            .any(|line| line.as_str() == "Contact X1 identified: BA, blue.")),
        "the pilot did not read the identification: {:?}",
        first.notices.get(&ObjId(1_000))
    );
}

/// **A window shorter than the substep** (the M.2 review's `M2-R4`):
/// `[tacsim.sensors].pass_ms` may be as short as 50 against a 250 ms substep,
/// so a loss can open and close its hysteresis inside one substep. Its expiry
/// is then settled by the staging scan that staged it, at its own place, and
/// never scheduled into the past — where it popped a boundary late and, at a
/// pulse's end, made the phase-4 drain run a substep at one cadence that the
/// others never ran. With a 100 ms pass and a four-pass loss band `MARK` is
/// lost a little after 10,250, in the substep that ends a 500 ms pulse, and the
/// three cadences are one event list, one hash sequence and one staged story:
/// the loss, and the return at fifteen seconds.
#[test]
fn m2_a_window_shorter_than_the_substep_is_one_story_at_every_cadence() {
    let arena = || {
        arena_with_sensors(
            vec![
                tank(
                    EYE,
                    Team::Red,
                    centre(10, 10),
                    Some(SensorProfile {
                        max_range_cells: 10,
                        classify_after_ms: 5_000,
                        ..visual()
                    }),
                ),
                tank(MARK, Team::Blue, centre(15, 10), None),
            ],
            &[],
            &SensorsConfig {
                pass_ms: 100,
                lost_after_passes: 4,
                ..SensorsConfig::default()
            },
        )
    };
    let first = three_cadences(arena, &flicker_replay_script(), 20_000);
    let lost_at: Vec<i64> = first
        .events
        .iter()
        .filter(|(_, json, _, _)| {
            json.contains("\"kind\":\"lost\"") && json.contains(&format!("\"viewer\":{}", EYE.0))
        })
        .map(|(t_ms, ..)| *t_ms)
        .collect();
    assert_eq!(lost_at.len(), 1, "one loss: {lost_at:?}");
    let lost = lost_at.first().copied().unwrap_or(0);
    // The fixture's premise: the window (one 100 ms pass) opens and closes
    // inside the substep that ends the 500 ms pulse at 10,500.
    assert!(
        lost > 10_250 && lost + 100 <= 10_500,
        "the fixture needs the loss's whole window inside 10,250..10,500: {lost}"
    );
    let kinds = eye_contact_kinds(&first);
    let said = kinds
        .iter()
        .position(|kind| *kind == NoticeKind::Lost)
        .unwrap_or_else(|| panic!("the loss was never said: {kinds:?}"));
    assert!(
        kinds
            .iter()
            .skip(said)
            .any(|kind| *kind == NoticeKind::Acquired),
        "the return was not said after the loss: {kinds:?}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut U.2 — a frame when a printed figure moves (16c `T-30`, Q27, Q28)
//
// `Frame::same_as` used to compare every contact row in full, age included, so a
// pilot holding anything was sent a whole keyframe every wake. It now compares
// the age at the resolutions a surface prints it at — the whole second, the
// stale clause past `pass_ms`, and on the locked row the lock's range step — and
// the keyframe still carries `age_ms` to the millisecond. The two properties
// below drive the real pulse and read the frames it sends.
//
// A target in continuous view is re-plotted every pass and its age never leaves
// the first second, so both fixtures break the line of sight **inside the loss
// window** (`lost_after_passes` raised to ten, so the plot is held, unrefreshed,
// for the whole run) and let the held plot's age really climb.
// ---------------------------------------------------------------------------

/// The `[tacsim.sensors]` block both fixtures run under: the shipped one with a
/// ten-pass time to live, so a plot the wall cut off is held for ten seconds.
fn u2_long_hold() -> SensorsConfig {
    SensorsConfig {
        lost_after_passes: 10,
        ..SensorsConfig::default()
    }
}

/// `EYE` watching `MARK` at `mark`, the tempo pinned so the one thing that can
/// move on a parked pair is the plot's age, and the opening pulse run.
fn u2_parked_pair(mark: Pose) -> Arena {
    u2_parked_pair_under(mark, &u2_long_hold())
}

/// [`u2_parked_pair`] under a named `[tacsim.sensors]` block.
fn u2_parked_pair_under(mark: Pose, sensors: &SensorsConfig) -> Arena {
    let mut arena = arena_with_sensors(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, mark, None),
        ],
        &[],
        sensors,
    );
    if let Some(state) = arena.unit_mut(EYE) {
        state.power = tacsim::arena::state::Power::Online;
    }
    arena.clock.pin_tempo_for_tests(Tempo::Alert);
    let _ = arena.step(0, &[], &[]);
    arena
}

/// Put the wall up between the pair — the same window with `curtain(25)` on it —
/// so `EYE`'s later passes see nothing and its plot of `MARK` is never refreshed.
fn u2_break_the_line(arena: &mut Arena) {
    let walled = arena_with_sensors(
        vec![
            tank(EYE, Team::Red, centre(10, 10), Some(visual())),
            tank(MARK, Team::Blue, centre(40, 10), None),
        ],
        &curtain(25),
        &u2_long_hold(),
    );
    arena.definition = walled.definition.clone();
}

/// The age of the one plot `EYE` holds, as the store has it now.
fn u2_held_age(arena: &Arena) -> i64 {
    arena
        .frame_contacts(EYE)
        .first()
        .map(|row| row.age_ms)
        .unwrap_or_else(|| panic!("EYE still holds its contact at {}", arena.now_ms))
}

/// **A parked watcher holding an unrefreshed plot is sent a frame on the pulse
/// the second moves and on no pulse between** (16b cut U.2, 16c `T-30`), across
/// five arena seconds of 250 ms pulses — and each frame it is sent carries the
/// plot's true age to the millisecond, so what the pilot may read did not change;
/// only how often they are sent it did.
#[test]
fn u2_a_parked_watcher_is_sent_a_frame_when_the_plots_second_moves_and_not_between() {
    use tacsim::arena::perception::age_key;
    let mut arena = u2_parked_pair(centre(40, 10));
    let offset = eye_offset(&arena);
    let mut tape: Vec<u8> = Vec::new();
    let first = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse sends a frame");
    let row = first
        .contacts
        .first()
        .copied()
        .unwrap_or_else(|| panic!("the acquiring frame carries the contact"));
    assert_eq!(row.age_ms, PULSE_MS - offset, "plotted at EYE's own pass");
    u2_break_the_line(&mut arena);

    let pass_ms = i64::from(arena.sensors_cfg.pass_ms);
    let mut read = row.age_ms;
    let mut sent: Vec<i64> = Vec::new();
    let mut silent = 0_u32;
    let mut wall = PULSE_MS;
    while wall < PULSE_MS + 5_000 {
        wall += 250;
        let frame = pulse(&mut arena, wall, &mut tape);
        let age = u2_held_age(&arena);
        assert_eq!(
            age,
            arena.now_ms - offset,
            "the plot was refreshed through the wall at {}",
            arena.now_ms
        );
        let moved = age_key(age, pass_ms) != age_key(read, pass_ms);
        match frame {
            Some(frame) => {
                assert!(
                    moved,
                    "a frame at {} ({age} ms) printed what the pilot already read ({read} ms)",
                    arena.now_ms
                );
                let row = frame
                    .contacts
                    .first()
                    .copied()
                    .unwrap_or_else(|| panic!("the frame at {} lost its row", arena.now_ms));
                assert_eq!(row.age_ms, age, "the keyframe carries the true age");
                read = row.age_ms;
                sent.push(arena.now_ms);
            }
            None => {
                assert!(
                    !moved,
                    "no frame at {} although the pilot's {read} ms now prints as {age} ms",
                    arena.now_ms
                );
                silent += 1;
            }
        }
    }
    // One a second: the plot crosses 1, 2, 3, 4 and 5 s old inside the run, and
    // the stale clause flips on the first of those pulses.
    assert_eq!(
        sent,
        vec![1_500, 2_500, 3_500, 4_500, 5_500],
        "one frame a second, on the pulse the second moved"
    );
    assert_eq!(silent, 15, "and nothing on the fifteen pulses between");
}

/// **A locked, parked pair is sent a frame on the pulse the plot's age crosses
/// 648 ms**, because the lock's published metres move there (16b cut U.2; 16c
/// `T-20`, Q28): at 305 m the 10 m step reads `310` and the 20 m step `300`, so
/// a pilot left on the first frame would be reading a digit the plot cannot
/// carry. Nothing is sent on the pulse before, where the step has not moved.
#[test]
fn u2_a_locked_pair_is_sent_a_frame_when_the_locks_range_step_moves() {
    use tacsim::arena::perception::lock_range_step_m;
    // 305 m east: a figure the two steps round apart.
    let mut arena = u2_parked_pair(pose(41_000, 10_500));
    let offset = eye_offset(&arena);
    let contact = draw_contact_id(arena.seed, EYE, MARK);
    // The acquiring pulse, with the lock in it: the intent waits in phase 2, the
    // pass at `offset` inserts the contact, and phase 4 takes the lock at 1,000.
    let out = arena.step(
        PULSE_MS,
        &[],
        &[intent(EYE, IntentKind::Lock { contact }, 1)],
    );
    let first = out
        .frames
        .into_iter()
        .find(|(pilot, _)| *pilot == ObjId(1_000))
        .map(|(_, frame)| frame)
        .unwrap_or_else(|| panic!("the acquiring pulse sends a frame"));
    assert_eq!(first.lock, Some(contact), "the lock was taken");
    let lock_range = |frame: &Frame| -> serde_json::Value {
        cockpit::status_frame(tacsim::arena_task::MatchId(1), frame)
            .get("lock_range_m")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    let age = PULSE_MS - offset;
    assert!(
        age < 648,
        "the fixture needs the first frame under the step: {age}"
    );
    assert_eq!(lock_range_step_m(age), 10);
    assert_eq!(lock_range(&first), serde_json::json!(310));
    u2_break_the_line(&mut arena);

    let mut tape: Vec<u8> = Vec::new();
    // The pulse before the crossing: nothing a pilot prints has moved.
    let before = offset + 647 - 60;
    assert!(
        before > PULSE_MS,
        "the fixture's quiet pulse is after the lock: {before}"
    );
    assert!(
        pulse(&mut arena, before, &mut tape).is_none(),
        "a frame at {before} ({} ms old) although the step is still 10 m",
        u2_held_age(&arena)
    );
    // The pulse after it.
    let after = offset + 700;
    let frame = pulse(&mut arena, after, &mut tape).unwrap_or_else(|| {
        panic!(
            "no frame when the plot turned {} ms old",
            u2_held_age(&arena)
        )
    });
    let row = frame
        .contacts
        .first()
        .copied()
        .unwrap_or_else(|| panic!("the locked row"));
    assert_eq!(row.age_ms, 700, "the true age, as ever");
    assert_eq!(frame.lock, Some(contact));
    assert_eq!(lock_range_step_m(row.age_ms), 20);
    assert_eq!(
        lock_range(&frame),
        serde_json::json!(300),
        "the lock's metres moved with the step"
    );
    assert!(
        frame.notices.is_empty(),
        "sent for the step alone: {:?}",
        frame.notices
    );
}

/// **The stale clause is compared at the arena's own `pass_ms`** (16b cut U.2,
/// 16c `T-30`): under `[tacsim.sensors].pass_ms = 1500` a parked watcher holding
/// an unrefreshed plot is sent a frame on the pulse its age crosses 1,500 ms —
/// halfway through a second, where no whole second moved — and that frame's
/// `stale_clause` says `, plotted 1 s ago` where the one before said nothing.
///
/// At the default cadence the stale flip and the whole-second flip fall on the
/// same pulse, so the two properties above cannot tell the arena's `pass_ms`
/// from a constant `1_000` at `Frame::same_as`'s call site; this one can: under
/// `age_key(…, 1_000)` the two ages either side of the crossing read alike, and
/// a pilot left on the earlier frame would be missing the clause.
#[test]
fn u2_the_stale_clause_moves_a_frame_at_the_arenas_own_pass_ms() {
    use tacsim::arena::perception::age_key;
    let sensors = SensorsConfig {
        pass_ms: 1_500,
        ..u2_long_hold()
    };
    let mut arena = u2_parked_pair_under(centre(40, 10), &sensors);
    let pass_ms = i64::from(arena.sensors_cfg.pass_ms);
    assert_eq!(pass_ms, 1_500, "the arena runs the named cadence");
    let offset = stagger_ms(arena.seed, EYE, pass_ms);
    let mut tape: Vec<u8> = Vec::new();

    // Pulse at 250 ms until `EYE`'s first pass has plotted `MARK`.
    let mut wall = 0_i64;
    let row = loop {
        wall += 250;
        assert!(
            wall <= offset + 250,
            "no acquiring frame by {wall} although EYE's pass is at {offset}"
        );
        if let Some(row) =
            pulse(&mut arena, wall, &mut tape).and_then(|frame| frame.contacts.first().copied())
        {
            break row;
        }
    };
    assert_eq!(
        row.age_ms,
        arena.now_ms - offset,
        "plotted at EYE's own pass"
    );
    u2_break_the_line(&mut arena);

    let acquired = wall;
    let mut read = row.age_ms;
    // The one frame sent for the stale clause alone: (what was read, what is sent).
    let mut clause_only: Vec<(i64, i64)> = Vec::new();
    while wall < acquired + 4_000 {
        wall += 250;
        let frame = pulse(&mut arena, wall, &mut tape);
        let age = u2_held_age(&arena);
        assert_eq!(
            age,
            arena.now_ms - offset,
            "the plot was refreshed through the wall at {}",
            arena.now_ms
        );
        let moved = age_key(age, pass_ms) != age_key(read, pass_ms);
        match frame {
            Some(frame) => {
                assert!(
                    moved,
                    "a frame at {} ({age} ms) printed what the pilot already read ({read} ms)",
                    arena.now_ms
                );
                let row = frame
                    .contacts
                    .first()
                    .copied()
                    .unwrap_or_else(|| panic!("the frame at {} lost its row", arena.now_ms));
                assert_eq!(row.age_ms, age, "the keyframe carries the true age");
                if age / 1_000 == read / 1_000 {
                    clause_only.push((read, age));
                }
                read = age;
            }
            None => assert!(
                !moved,
                "no frame at {} although the pilot's {read} ms now prints as {age} ms",
                arena.now_ms
            ),
        }
    }
    // Exactly one frame moved for the clause and not for a second, and it was
    // the crossing of the arena's own `pass_ms`.
    let [(before, after)] = clause_only.as_slice() else {
        panic!("one frame for the stale clause alone, not {clause_only:?}");
    };
    assert!(
        *before <= pass_ms && *after > pass_ms,
        "the clause frame straddles {pass_ms}: {before} -> {after}"
    );
    assert_eq!(
        age_key(*before, 1_000),
        age_key(*after, 1_000),
        "at the default cadence these two ages read alike, so only the arena's own pass_ms sends this frame"
    );
    assert_eq!(cockpit::stale_clause(*before, pass_ms), "");
    assert_eq!(
        cockpit::stale_clause(*after, pass_ms),
        ", plotted 1 s ago",
        "the frame carries the clause the earlier one could not"
    );
}
