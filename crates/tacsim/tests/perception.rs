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
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::{
    FrameNotice, Notice, NoticeKind, PerceptionCounters, SensorProfile, SignatureFactor,
    bearing_mdeg,
};
use tacsim::arena::rng::{DrawLabel, draw, draw_contact_id};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, CustodyOut, CustodyUnit,
    DueKind, EntryZone, Pilot, Team, TerrainCell, TrackedRules, WeaponSummary,
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
/// `tempo.alert_range_cm` is twice the longest reach in the match (16b §9).
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
        unit: tactical_unit(id),
    }
}

/// The 80 by 80 window with `paint`ed cells and one pilot per unit.
fn arena_of(units: Vec<CustodyUnit>, paint: &[(u16, u16, TerrainCell)]) -> Arena {
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
        &SensorsConfig::default(),
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
    // Doc 16 §3.6: staggered by unit id. The offsets are the object numbers modulo
    // `sensors.pass_ms`, so 4 ms and 9 ms, and exactly one pass per unit is ever
    // scheduled at a time.
    let scheduled: Vec<(i64, ObjId)> = arena
        .due
        .iter()
        .filter_map(|event| match event.kind {
            DueKind::SensorPass { unit, .. } => Some((event.t_ms, unit)),
            _ => None,
        })
        .collect();
    assert_eq!(scheduled, vec![(4, EYE), (9, MARK)]);

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
    assert_eq!(arena.alert_range_cm, 500_000, "twice the cannon's reach");
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
        Some((label.encode(), draw(arena.seed, &label))),
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
        let (label, value) = event
            .draw
            .clone()
            .unwrap_or_else(|| panic!("a classification carries its draw"));
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
/// puts each of `EYE`'s passes (`4`, `1_004`, `2_004`, …) in a pulse of its own, so
/// a test can say *which pulse* a notice arrived in.
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
        .map(|notice| notice.kind.as_str())
        .collect()
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
fn the_frame_renders_a_contact_as_an_opaque_id_a_band_and_two_bearings() {
    let mut arena = watching();
    let mut tape: Vec<u8> = Vec::new();
    let frame = pulse(&mut arena, PULSE_MS, &mut tape).expect("the acquiring pulse sends a frame");
    assert_eq!(told(&frame), vec!["acquired"], "one notice, once");

    let rows = cockpit::frame_contacts(&frame);
    assert_eq!(rows.len(), 1, "the platform holds one return");
    let row = rows.first().cloned().expect("one row");
    let id = draw_contact_id(arena.seed, EYE, MARK);
    assert_eq!(row.id, id, "the opaque id is the arena's own");
    assert_eq!(row.class.as_str(), "unknown", "not told apart yet");
    assert!(!row.friendly, "an unknown return is nobody's");
    // Due east of a platform facing north: 090 absolute, 090 relative.
    assert_eq!(row.bearing_deg, 90);
    assert_eq!(row.relative_deg, 90);
    // 30,000 cm against the cannon's 250,000 cm reach: inside a third of it.
    assert_eq!(row.band.as_str(), "close");
    assert_eq!(row.cell, Some((40, 10)), "the marker's cell");
    assert_eq!(row.age_ms, arena.now_ms - 4, "held since its first pass");
    assert!(
        row.render().contains(&id.to_string()),
        "the line names the opaque id: {}",
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
fn a_range_band_is_read_against_the_platforms_own_reach() {
    // Thirds of a 300 cm reach, at the boundaries.
    assert_eq!(cockpit::range_band(0, 300), "close");
    assert_eq!(cockpit::range_band(100, 300), "close");
    assert_eq!(cockpit::range_band(101, 300), "effective");
    assert_eq!(cockpit::range_band(300, 300), "effective");
    assert_eq!(cockpit::range_band(301, 300), "far");
    assert_eq!(cockpit::range_band(600, 300), "far");
    assert_eq!(cockpit::range_band(601, 300), "beyond");
    // A platform with nothing to shoot with has nothing in range.
    assert_eq!(cockpit::range_band(0, 0), "beyond");
    assert_eq!(cockpit::range_band(u64::MAX, u32::MAX), "beyond");
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

    // The pass at 3,004 expires the contact the pass at 4 minted: three passes at
    // the shipped `lost_after_passes`.
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
        vec![3_004],
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

    // Out of reach through the pass that expires the contact (3,004) — and back
    // before the next one (4,004), which is the window the rule names.
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
    assert_eq!(event_times(&mut arena, &mut tape, "lost"), vec![3_004]);
    assert_eq!(
        event_times(&mut arena, &mut tape, "detected"),
        vec![4, 4_004],
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
    assert!(line.contains(&id.to_string()), "{line}");
    assert!(line.contains("close"), "the band the notice quotes: {line}");
    assert!(
        !line.contains(&format!("#{}", MARK.0)),
        "and never the target: {line}"
    );
    // The same renderer answers the frame's own `notices` array, so a panel and a
    // line client read one sentence.
    for notice in &frame.notices {
        assert_eq!(
            cockpit::notice_line(notice, &cockpit::frame_contacts(&frame)),
            line
        );
    }
    // A `lost` notice names the id and nothing else: the contact is not in the
    // frame's list any more, so there is no bearing to quote.
    let gone = FrameNotice {
        kind: NoticeKind::Lost,
        contact: id,
        class: Classification::Unknown,
    };
    assert_eq!(
        cockpit::notice_line(&gone, &[]),
        format!("Contact {id} lost.")
    );
}
