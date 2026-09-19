//! **Doc 16 §9's extension acceptances** (16b §8 "Cut X"), one test apiece.
//!
//! Doc 16 §9: *each is a test that passes without changing A1.* That is the whole
//! rule of the cut and the reason this file exists at all — an extension proves
//! itself here, against content the stock game does not ship, and the story test,
//! the harness and every test written before it run unchanged on either side of
//! the commit that adds one.
//!
//! **X.1, lasers** (doc 16 §9 item 1, D-16-18's first): a weapon row with
//! `delivery: beam`, `impact_at_ms = now` and `heat_per_shot > 0`; the beam's
//! attenuation as a term in the error cone; a `heat` cell and a dissipation rate on
//! the unit, heat rising per shot and falling with arena time; a shot refused at the
//! ceiling with a named refusal; and W2 holding, because a laser's damage against
//! the line tank's armour is small by definition.
//!
//! **X.2, bipeds** (doc 16 §9 item 2, D-18-16's second): a unit of six sections
//! — two legs, a torso, a head and two arms — walking under a `legged` mobility
//! mode of its own, presenting a silhouette eleven metres tall from a sight nine
//! metres up, struck on the front band in its torso and its arms, slowed by the
//! loss of one leg and stopped by the loss of both. Doc 16 §3.4 is untouched:
//! the same four steps read the same `MotionLimits` and the same five mode
//! numbers, off a mode nobody had authored before.
//!
//! **X.3, the thermal sight** (doc 16 §9 item 3, D-16-18's third): a sensor row
//! with its own range, its own attenuation budget, its own cadences and a
//! signature table deliberately flatter than the eye's, plus one new signature
//! term keyed on the **target's** power and speed — a tank under way makes a
//! plume, an idling one is dim, a shut-down one is nearly dark — multiplied into
//! the ground's factor inside step 3's range test, so it costs **no new trace**.
//! The shipped sight's own table is asserted byte for byte, and a stock fixture's
//! perception counters are asserted identical with the extension loaded and
//! without it.
//!
//! **X.4, ECM** (doc 16 §9 item 4, D-16-18's fourth): a sensor row whose `jamming`
//! block is non-null — the hook doc 16 §3.6 left null — carried by a pod mounted
//! through a component role of its own, and two terms on the pass of every observer
//! standing inside its bubble: the step 2 attenuation budget loses the pod's cost,
//! and every target's step 3 signature factor is multiplied down, which is doc 16 §2
//! W1 as arithmetic. A target inside the bubble is lost to an observer inside it and
//! held, unchanged, by the same observer outside it; a pod that is shut down or burnt
//! out radiates nothing; and a field with no pod on it never scans for one.
//!
//! **X.5, ECCM** (doc 16 §9 item 5, D-16-18's fifth): a sensor row whose `emission`
//! block is non-null — the *other* hook doc 16 §3.6 left null — fitted through a
//! component role of its own to the sight the tank already has, and one subtraction
//! and one addition on X.4's own fold before either of its terms is read. The
//! observer inside the bubble holds the target it had lost, out to the recovery and
//! no further; a filter rated against the pod exactly is the unjammed pass to the
//! centimetre; a filter with no bubble on it, or one shot off its mount, is no
//! filter at all; and it needs no power, because a filter radiates nothing.
//!
//! # What this file loads, and what it deliberately does not
//!
//! The three extension packs go through the **operator's** route —
//! [`tacsim::pack::load_pack`] into [`Catalog::build_game`], which is one loader,
//! one validation and one set of C1 descriptors, the same route
//! `[tacsim].pack_paths` takes. They are not in `content::SOURCES`, so the shipped
//! catalog is the two weapons and the one unit it has always been; the first test
//! below asserts exactly that, because "the extension changes nothing" is the
//! acceptance and not an aside.
//!
//! Each item loads **its own** rows through that one route
//! (`content::EXTENSIONS` for X.1, `content::EXTENSIONS_BIPED` for X.2) rather
//! than a growing union of all of them, because X.1's acceptance asserts the
//! whole of what the catalog holds with its content loaded — three weapons, two
//! unit schemas — and that assertion is one no later item may move.
//!
//! Run with `TACSIM_WRITE_CONTENT=1` to re-sign the extension packs in place, as
//! `content.rs` does for the shipped ones; the pins below are still updated by
//! hand, which is the point.

use std::path::{Path, PathBuf};

use mush_atlas::TerrainClass;
use mush_core::CanonicalId;
use smol_str::SmolStr;
use tacsim::arena::damage::UnitReturn;
use tacsim::arena::intents::Refusal;
use tacsim::arena::resolution::FULL_ATTENUATION;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, CustodyOut, CustodyUnit, EntryZone,
    Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell, UnitState, Waypoints,
};
use tacsim::config::{ArenaConfig, LoadedPack, MatchConfig, SensorsConfig};
use tacsim::definitions::unit::{HEAT_CELL, HEAT_PER_SHOT_CELL, HEAT_SINK_CELL};
use tacsim::definitions::{Catalog, content};
use tacsim::match_::cockpit;

// ---------------------------------------------------------------------------
// The content
// ---------------------------------------------------------------------------

/// The placeholder an unsigned pack carries, as `tests/content.rs` spells it.
const UNSIGNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// **The extension packs' digest pins** (16b cut X.1), the way C.2 pins the
/// shipped ones: a content change moves one of these and moving one is a reviewed
/// diff.
///
/// They are here rather than in `tests/content.rs`'s table because that table is
/// driven by `content::SOURCES` and asserts it names every shipped pack; a row for
/// a pack the game does not ship would fail it. The pin is the same pin.
const DIGESTS: [(&str, &str); 3] = [
    (
        "tacsim.pack.laser_charge",
        "1128f466f9c5839bbebef59a0f1e57a5e5aafca5e19dc9f0406913b077abeceb",
    ),
    (
        "tacsim.pack.laser_mk1",
        "3fd9565ac274989fec9e4f99784dda4cf6e63733a92b43a343e38c08ff96fae5",
    ),
    (
        "tacsim.pack.line_tank_laser",
        "211cabe64b5940e4092cca1009edf11263c03ef2f202ecc0705075dc27c32a1b",
    ),
];

/// The extension pack files, by their path under `content/`.
const PACK_FILES: [&str; 3] = [
    "ammunition/laser_charge.json",
    "weapons/laser_mk1.json",
    "units/line_tank_laser.json",
];

/// **The biped's digest pin** (16b cut X.2), the way X.1 pins the laser's.
const DIGESTS_BIPED: [(&str, &str); 1] = [(
    "tacsim.pack.biped_mk1",
    "6d2272a002628d03e864e50966a79922427dbdff037d20f1d0eb33c7ca51935b",
)];

/// The biped's pack file, by its path under `content/`.
const PACK_FILES_BIPED: [&str; 1] = ["units/biped_mk1.json"];

/// **The thermal sight's digest pins** (16b cut X.3), the way X.1 pins the
/// laser's: the sight, then the tank that mounts it.
const DIGESTS_THERMAL: [(&str, &str); 2] = [
    (
        "tacsim.pack.thermal_mk1",
        "921be78771d4ce4c07a6289bc448d1b39c622dc68dcadbe266aa4f405629c48b",
    ),
    (
        "tacsim.pack.line_tank_thermal",
        "11a9019b18d7d4d21ce1850bfcf82a2f53ec869bcab4337d9ff01d2ed82ae178",
    ),
];

/// The thermal sight's pack files, by their paths under `content/`.
const PACK_FILES_THERMAL: [&str; 2] = ["sensors/thermal_mk1.json", "units/line_tank_thermal.json"];

/// **The jamming pod's digest pins** (16b cut X.4), the way X.1 pins the laser's:
/// the pod's row, then the tank that carries it.
const DIGESTS_ECM: [(&str, &str); 2] = [
    (
        "tacsim.pack.ecm_mk1",
        "0ce4a2787d83f226b3bad46c9d09b3d960d55c78f00c79c888ac9f08d416ba22",
    ),
    (
        "tacsim.pack.line_tank_ecm",
        "8f83e07a3a509385b23ff13eee1984ce4a93187d856abcda4713709a6aa4596a",
    ),
];

/// The pod's pack files, by their paths under `content/`.
const PACK_FILES_ECM: [&str; 2] = ["sensors/ecm_mk1.json", "units/line_tank_ecm.json"];

/// **The filter's digest pins** (16b cut X.5), the way X.1 pins the laser's: the
/// filter's row, then the tank that carries it.
const DIGESTS_ECCM: [(&str, &str); 2] = [
    (
        "tacsim.pack.eccm_mk1",
        "4bf6842ee80f746b62d669680b0e32b941ca04f8b0bae2c6a940ede26f6d3de1",
    ),
    (
        "tacsim.pack.line_tank_eccm",
        "2f48fb1438a00ff4e4bcf67f0c8244e16701de3cb320b2ad2c6e7564a6f41c8a",
    ),
];

/// The filter's pack files, by their paths under `content/`.
const PACK_FILES_ECCM: [&str; 2] = ["sensors/eccm_mk1.json", "units/line_tank_eccm.json"];

/// **The shipped sight's signature table, verbatim** (16b cut X.3).
///
/// Doc 16 §9's rule read for this item: the thermal sight is an addition and the
/// Mk1 visual sight is what it was. X.3 widened the *sensor mount*
/// (`ComponentRole::Sensor`) and not [`tacsim::definitions::SensorDef`], and this
/// is the assertion that says so in bytes rather than in prose — a column added
/// to a signature row, or a field added to the sensor row itself, moves this
/// string and with it the reach of every sensor pass in the game.
const VISUAL_MK1_SIGNATURE: &str = concat!(
    r#"[{"class":"road","moving_permille":1000,"stopped_permille":1000},"#,
    r#"{"class":"ground","moving_permille":1000,"stopped_permille":900},"#,
    r#"{"class":"grass","moving_permille":1000,"stopped_permille":800},"#,
    r#"{"class":"rubble","moving_permille":950,"stopped_permille":700},"#,
    r#"{"class":"woods_light","moving_permille":900,"stopped_permille":650},"#,
    r#"{"class":"woods_heavy","moving_permille":800,"stopped_permille":500}]"#,
);

/// **The shipped tank's exact identity** (16b cut X.2), which is the whole of
/// what "no migration" means: a `line_tank` persisted before this cut points at
/// this digest, and a `line_tank` bound after it must point at the same one.
///
/// X.2 widened `UnitSpec::mobility` — `tracked` became optional and `legged`
/// joined it — and the two `skip_serializing_if`s are what keep the tank's
/// `{"tracked": {…}}` the same bytes, hence the same payload, hence the same
/// digest. A pin is the only way to say that about a *code* change; the content
/// pins above only ever catch a content one. The value is this crate's answer at
/// `e000f06a`, the commit before the cut, taken there and pinned here.
const LINE_TANK_DIGEST: &str = "72a52d55e73e3d1d4308c8d69ebd3b0a91504a5935571f87ae319c69cd131660";

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content")
}

fn writing() -> bool {
    std::env::var("TACSIM_WRITE_CONTENT").is_ok_and(|v| v == "1")
}

/// The whole catalog: the shipped content plus the extension packs, through the
/// operator's route.
fn catalog() -> Catalog {
    catalog_of(&content::extension_sources())
}

/// **The shipped content plus one item's extension rows** (16b cut X.2), through
/// the operator's route — one [`tacsim::pack::load_pack`], one
/// [`Catalog::build_game`], one set of C1 descriptors.
fn catalog_of(sources: &[tacsim::config::PackSource]) -> Catalog {
    let operator: Vec<LoadedPack> = sources
        .iter()
        .map(|source| LoadedPack {
            pack: tacsim::pack::load_pack(&source.bytes)
                .unwrap_or_else(|error| panic!("{}: {error}", source.name)),
            source: source.name.clone(),
        })
        .collect();
    Catalog::build_game(&operator, tacsim::NS).expect("the extension content binds")
}

/// The shipped content plus the biped (16b cut X.2).
fn biped_catalog() -> Catalog {
    catalog_of(&content::extension_sources_of(&content::EXTENSIONS_BIPED))
}

/// The shipped content plus the thermal sight and its tank (16b cut X.3).
fn thermal_catalog() -> Catalog {
    catalog_of(&content::extension_sources_of(&content::EXTENSIONS_THERMAL))
}

/// Re-sign one extension pack in place and print its pin, as X.1's own writing
/// branch does for the three laser files.
fn resign(relative: &str) {
    let path = content_dir().join(relative);
    let raw = std::fs::read_to_string(&path).expect("the content file exists");
    let declared = mush_content::parse_pack(&raw)
        .expect("the pack parses")
        .manifest()
        .digest()
        .to_hex();
    let unsigned = raw.replace(&declared, UNSIGNED);
    let signed = mush_content::parse_pack(&unsigned)
        .expect("the unsigned pack parses")
        .computed_digest()
        .expect("the pack digests")
        .to_hex();
    std::fs::write(&path, unsigned.replace(UNSIGNED, &signed))
        .expect("the content file is writable");
    println!("    (\"{relative}\", \"{signed}\"),");
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm, the shape `tests/resolution.rs` uses.
const SIDE: u16 = 16;
/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;
/// The match seed.
const SEED: u64 = 2_350;
/// The shooter.
const RED: mush_core::ObjId = mush_core::ObjId(4);
/// The target.
const BLUE: mush_core::ObjId = mush_core::ObjId(9);
/// Red's cell, four cells due south of blue's.
const RED_CELL: (u16, u16) = (8, 5);
/// Blue's cell.
const BLUE_CELL: (u16, u16) = (8, 9);

fn ground() -> TerrainCell {
    TerrainCell {
        class: TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// A cell of the light woods that attenuates a line without blocking it.
fn woods() -> TerrainCell {
    TerrainCell {
        class: TerrainClass::WoodsLight,
        elev_cm: 0,
        move_cost: 200,
    }
}

fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, painted) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *painted;
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.test"),
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
        waypoints: Waypoints::default(),
    }
}

/// An empty opaque posture blob.
fn rules() -> mush_tactics::RulesValue {
    let reference = mush_tactics::DefinitionRef::new(
        mush_core::DefinitionKey::new(
            mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
            mush_core::DefinitionKind::new("schema").expect("a canonical kind"),
            mush_core::DefinitionId::new("tacsim.arena.test").expect("a canonical id"),
        ),
        1,
        mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero");
    mush_tactics::RulesValue::new(reference, serde_json::json!({})).expect("an empty blob")
}

/// The middle of cell `(x, y)`, facing north.
fn pose(cell: (u16, u16)) -> mush_sim::data::Pose {
    let middle = |n: u16| i64::from(n) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
    mush_sim::data::Pose::at(middle(cell.0), middle(cell.1), 0, rules())
}

/// **One unit of `entry`, checked out exactly as `match_::start` checks one out**
/// (doc 16 §3.1 "Custody"): the M32 unit from the catalog's own binding, and every
/// row the arena reads decoded by the same functions the production check-out
/// calls.
fn unit_of(
    catalog: &Catalog,
    entry: &str,
    id: mush_core::ObjId,
    team: Team,
    cell: (u16, u16),
) -> CustodyUnit {
    let key = CanonicalId::new(entry).expect("a canonical entry id");
    let spec = catalog.specs.get(&key).expect("the unit entry");
    let binding = catalog.bindings.get(&key).expect("the unit binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("the unit");
    CustodyUnit {
        id,
        team,
        pose: pose(cell),
        mobility: tacsim::match_::tracked_rules(&cells),
        sensor_height_cm: tacsim::match_::sensor_height_cm(catalog, &cells),
        sensor: tacsim::match_::sensor_profile(catalog, &cells),
        weapons: tacsim::match_::weapon_summaries(catalog, &cells),
        profile: tacsim::match_::unit_profile(catalog, &cells),
        gunnery: 0,
        controller: tacsim::arena::state::Controller::Pilot,
        unit: cells,
    }
}

/// An arena of `units` over a window with `paint`ed cells, every unit seated.
fn arena_of(
    catalog: &Catalog,
    units: Vec<CustodyUnit>,
    paint: &[(u16, u16, TerrainCell)],
) -> Arena {
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: mush_core::ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: definition(paint),
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(catalog),
            seed: SEED,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// A laser tank facing a stock line tank four cells away, both awake, red locked
/// on blue.
fn duel(paint: &[(u16, u16, TerrainCell)]) -> Arena {
    let catalog = catalog();
    let mut arena = arena_of(
        &catalog,
        vec![
            unit_of(&catalog, content::LINE_TANK_LASER, RED, Team::Red, RED_CELL),
            unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        paint,
    );
    for id in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    let contact = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
    arena.contacts.insert(
        RED,
        contact,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            0,
            plot_of(&arena, BLUE),
        ),
    );
    arena.resolve_lock(RED, contact).expect("the lock holds");
    arena
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
fn plot_of(arena: &Arena, target: mush_core::ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

/// One cell value off a unit.
fn cell(arena: &Arena, unit: mush_core::ObjId, addr: &mush_tactics::CellAddr) -> Option<i64> {
    arena
        .unit(unit)?
        .unit
        .cell(addr)
        .map(|scalar| scalar.current)
}

/// The heat cell's address on the laser tank's hull.
fn heat_addr(cell: &str) -> mush_tactics::CellAddr {
    mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("a section"),
        cell: mush_tactics::CellId::new(cell).expect("a cell"),
    }
}

/// **Run `pulses` pulses of `period` ms of wall time through `Arena::step`.**
///
/// `wall` is the caller's monotonic reading, because arena time is not it: doc 16
/// §3.1 makes the first pulse establish the reference and integrate nothing, and a
/// loop that drove `step` off `arena.now_ms` would never get past that first call.
/// One extra pulse is spent here to establish it, once per arena, which is what
/// `wall == 0` means on entry.
fn run(arena: &mut Arena, wall: &mut i64, period: i64, pulses: usize) {
    if *wall == 0 {
        arena.step(0, &[], &[]);
    }
    for _ in 0..pulses {
        *wall += period;
        arena.step(*wall, &[], &[]);
    }
}

/// Clear the laser's cycle so the next `FIRE` is not refused for it.
fn ready(arena: &mut Arena) {
    if let Some(state) = arena.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("laser"), 0);
    }
}

// ---------------------------------------------------------------------------
// X.1 — lasers
// ---------------------------------------------------------------------------

/// **The extension changes nothing about the stock game** — doc 16 §9's own rule,
/// asserted rather than assumed.
///
/// The shipped catalog is what `tests/definitions.rs` says it is, the extension
/// packs verify and carry their own digests, and loading them beside the shipped
/// content leaves every shipped definition exactly where it was — the same
/// `DefinitionRef`, digest and all, which is what makes it true that a persisted
/// `line_tank` needs no migration for this cut (16b §8 cut X).
#[test]
fn x1_the_laser_content_loads_beside_the_stock_game_without_moving_it() {
    if writing() {
        for relative in PACK_FILES {
            let path = content_dir().join(relative);
            let raw = std::fs::read_to_string(&path).expect("the content file exists");
            let declared = mush_content::parse_pack(&raw)
                .expect("the pack parses")
                .manifest()
                .digest()
                .to_hex();
            let unsigned = raw.replace(&declared, UNSIGNED);
            let signed = mush_content::parse_pack(&unsigned)
                .expect("the unsigned pack parses")
                .computed_digest()
                .expect("the pack digests")
                .to_hex();
            std::fs::write(&path, unsigned.replace(UNSIGNED, &signed))
                .expect("the content file is writable");
            println!("    (\"{relative}\", \"{signed}\"),");
        }
        return;
    }

    // The shipped catalog, untouched by this cut.
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");

    // The extension packs verify, and each carries the digest pinned here.
    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS.into_iter().collect();
    for (name, json) in content::EXTENSIONS {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS.len());

    // And with them loaded: one more weapon, one more ammunition, one more unit,
    // and every shipped definition at the identity it already had.
    let both = catalog();
    assert_eq!(
        both.weapons.len(),
        3,
        "the cannon, the coaxial and the laser"
    );
    assert_eq!(both.specs.len(), 2, "the line tank and its laser variant");
    assert_eq!(both.arenas.len(), 1, "the extension adds no arena");
    for (id, reference) in &stock.references {
        assert_eq!(
            both.references.get(id),
            Some(reference),
            "{id} moved when the extension loaded; a persisted unit points at it"
        );
    }
}

/// **A beam arrives at once** (doc 16 §9 item 1: `impact_at_ms = now`), and the
/// weapon says so in its own row.
#[test]
fn x1_a_beam_is_a_weapon_with_no_flight_time_and_its_impact_is_scheduled_at_now() {
    let catalog = catalog();
    let key = CanonicalId::new(content::LASER_MK1).expect("a canonical id");
    let def = catalog.weapons.get(&key).expect("the laser entry");
    assert_eq!(
        def.delivery,
        tacsim::definitions::weapon::Delivery::Beam,
        "doc 16 §9 item 1's `delivery: beam`"
    );
    assert_eq!(def.muzzle_speed_cms, 0, "a beam arrives at once");
    assert!(def.heat_per_shot > 0, "doc 16 §9 item 1's `heat_per_shot`");

    let mut arena = duel(&[]);
    let seq = arena
        .resolve_fire(RED, "laser")
        .expect("the shot is accepted");
    let shot = arena.shot(seq).cloned().expect("the shot is in flight");
    assert_eq!(
        shot.impact_at_ms, shot.fired_at_ms,
        "a beam's impact is scheduled at the moment it was fired, at any range"
    );
    assert_eq!(shot.range_cm, 4_000, "four cells of 1,000 cm");
    // And the cannon's round, over the same four cells, is not: the difference is
    // the whole of what `delivery` decides.
    let stock = catalog;
    let mut direct = arena_of(
        &stock,
        vec![
            unit_of(&stock, content::LINE_TANK, RED, Team::Red, RED_CELL),
            unit_of(&stock, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[],
    );
    if let Some(state) = direct.unit_mut(RED) {
        state.power = Power::Online;
    }
    let contact = tacsim::arena::rng::draw_contact_id(direct.seed, RED, BLUE);
    direct.contacts.insert(
        RED,
        contact,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            0,
            plot_of(&direct, BLUE),
        ),
    );
    direct.resolve_lock(RED, contact).expect("the lock holds");
    let seq = direct
        .resolve_fire(RED, "cannon")
        .expect("the shot is accepted");
    let round = direct.shot(seq).cloned().expect("the round is in flight");
    assert!(
        round.impact_at_ms > round.fired_at_ms,
        "a 1 km/s round over 40 m is in flight for 40 ms"
    );
}

/// **The cells doc 16 §9 item 1 asks for** — and their absence from the stock
/// tank, which is what "without changing A1" means at the level of the definition.
#[test]
fn x1_the_heat_cells_are_laid_down_for_the_laser_tank_and_for_nothing_else() {
    let arena = duel(&[]);
    assert_eq!(cell(&arena, RED, &heat_addr(HEAT_CELL)), Some(0), "cold");
    assert_eq!(
        arena
            .unit(RED)
            .and_then(|state| state.unit.cell(&heat_addr(HEAT_SINK_CELL)))
            .map(|scalar| scalar.current),
        Some(3),
        "the authored dissipation rate, as a cell on the unit"
    );
    assert_eq!(
        arena
            .unit(RED)
            .and_then(|state| state.unit.cell(&mush_tactics::CellAddr::Component {
                component: mush_tactics::ComponentId::new("laser").expect("a component"),
                cell: mush_tactics::CellId::new(HEAT_PER_SHOT_CELL).expect("a cell"),
            }))
            .map(|scalar| scalar.current),
        Some(20),
        "what one pulse costs, as a cell on the weapon's own component"
    );
    // The ceiling is the cell's own maximum, so there is no second number.
    assert_eq!(arena.unit(RED).and_then(UnitState::heat_max), Some(60));
    // And the stock tank carries none of it.
    assert_eq!(cell(&arena, BLUE, &heat_addr(HEAT_CELL)), None);
    assert!(
        arena.unit(BLUE).is_some_and(|state| state.heat.is_none()),
        "a unit with no ceiling has no heat state at all"
    );
    assert_eq!(arena.unit(BLUE).and_then(UnitState::heat_now), None);
}

/// **Heat rises per shot, falls with arena time, and refuses the shot at the
/// ceiling** (doc 16 §9 item 1), all three through `Arena::step` rather than by
/// poking the cells.
#[test]
fn x1_heat_rises_per_shot_falls_per_pulse_and_refuses_the_shot_at_the_ceiling() {
    let mut arena = duel(&[]);
    let heat = |arena: &Arena| arena.unit(RED).and_then(UnitState::heat_now);

    // Three pulses of 20 against a ceiling of 60. The cycle is cleared between
    // them because the cycle is not what this test is about.
    arena.resolve_fire(RED, "laser").expect("the first shot");
    assert_eq!(heat(&arena), Some(20), "one pulse, one heat_per_shot");
    ready(&mut arena);
    arena.resolve_fire(RED, "laser").expect("the second shot");
    assert_eq!(heat(&arena), Some(40));
    ready(&mut arena);
    arena.resolve_fire(RED, "laser").expect("the third shot");
    assert_eq!(heat(&arena), Some(60), "at the ceiling");

    // And the fourth is refused, in its own word, with rounds and a cycle to
    // spare — so the refusal is the heat's and nothing else's.
    ready(&mut arena);
    assert!(
        arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ammo("laser") > 0),
        "the magazine is not what stops it"
    );
    assert_eq!(
        arena.resolve_fire(RED, "laser"),
        Err(Refusal::Overheated),
        "doc 16 §9 item 1: a shot refused above a heat ceiling"
    );
    assert_eq!(Refusal::Overheated.as_str(), "overheated");

    // The fall is a rate over arena time: three a second, so two seconds of
    // pulses take it to 54 and the weapon comes back.
    let mut wall = 0_i64;
    run(&mut arena, &mut wall, 500, 4);
    assert_eq!(heat(&arena), Some(54), "two seconds at three a second");
    ready(&mut arena);
    arena
        .resolve_fire(RED, "laser")
        .expect("cooled enough to fire");
    assert_eq!(heat(&arena), Some(60), "and straight back to the ceiling");

    // Twenty seconds empties it, and it floors at zero rather than going under.
    run(&mut arena, &mut wall, 1_000, 30);
    assert_eq!(heat(&arena), Some(0), "cold again, and no further");
}

/// **The fall is rate-independent** (D-16-2), which is the reason it carries its
/// remainder: three a second is nothing at all per 250 ms substep in whole units.
#[test]
fn x1_the_dissipation_is_the_same_at_every_tempo() {
    let mut fine = duel(&[]);
    let mut coarse = duel(&[]);
    for arena in [&mut fine, &mut coarse] {
        arena.resolve_fire(RED, "laser").expect("one pulse of heat");
        assert_eq!(arena.unit(RED).and_then(UnitState::heat_now), Some(20));
    }
    // Four seconds of arena time: eight 500 ms pulses, and two 2,000 ms ones
    // (`max_catch_up_steps` is four, so 2,000 is the most one `Engaged` pulse
    // may carry without lagging).
    let mut fine_wall = 0_i64;
    let mut coarse_wall = 0_i64;
    run(&mut fine, &mut fine_wall, 500, 8);
    run(&mut coarse, &mut coarse_wall, 2_000, 2);
    assert_eq!(fine.now_ms, coarse.now_ms, "the same four seconds");
    assert_eq!(
        fine.unit(RED).and_then(UnitState::heat_now),
        Some(8),
        "20 less four seconds at three a second"
    );
    assert_eq!(
        fine.unit(RED).and_then(UnitState::heat_now),
        coarse.unit(RED).and_then(UnitState::heat_now),
        "the same four seconds, chopped two different ways"
    );
}

/// **The beam's attenuation is a term in the error cone** (doc 16 §9 item 1:
/// *resolution uses `beam_path` attenuation as a cover term*).
///
/// The same shot over the same range, with and without light woods in the line:
/// the cone widens by the attenuation the walk gathered, and by nothing else —
/// and a direct-fire round over the same attenuated line is not widened at all,
/// because the term is a beam's alone.
#[test]
fn x1_the_ground_a_beam_crosses_widens_its_cone_and_a_rounds_is_untouched() {
    let clear = duel(&[]);
    let obscured = duel(&[(8, 6, woods()), (8, 7, woods()), (8, 8, woods())]);
    let clear_cone = clear
        .resolve_shot(RED, BLUE, "laser", 100_000, mush_atlas::Cover::None)
        .error_urad;
    let attenuation = obscured.fire_attenuation(RED, BLUE);
    assert!(
        attenuation > 0 && attenuation <= FULL_ATTENUATION,
        "light woods attenuate a line without blocking it: {attenuation}"
    );
    let obscured_cone = obscured
        .resolve_shot(RED, BLUE, "laser", 100_000, mush_atlas::Cover::None)
        .error_urad;
    let weight = obscured.resolution.error_terms.beam_attenuation;
    assert_eq!(
        obscured_cone - clear_cone,
        attenuation * weight / FULL_ATTENUATION,
        "the weight against a fully attenuated line, pro rata"
    );
    assert!(
        obscured
            .resolve_shot(RED, BLUE, "laser", 100_000, mush_atlas::Cover::None)
            .p_permille
            < clear
                .resolve_shot(RED, BLUE, "laser", 100_000, mush_atlas::Cover::None)
                .p_permille,
        "a wider cone is a worse shot"
    );
    // The stock cannon fires down the same obscured line and its cone does not
    // move: the term is keyed on `delivery`, not on the ground.
    let stock = catalog();
    let rounds = arena_of(
        &stock,
        vec![
            unit_of(&stock, content::LINE_TANK, RED, Team::Red, RED_CELL),
            unit_of(&stock, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[(8, 6, woods()), (8, 7, woods()), (8, 8, woods())],
    );
    let open = arena_of(
        &stock,
        vec![
            unit_of(&stock, content::LINE_TANK, RED, Team::Red, RED_CELL),
            unit_of(&stock, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[],
    );
    assert_eq!(
        rounds
            .resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None)
            .error_urad,
        open.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None)
            .error_urad,
        "a direct-fire weapon pays nothing for the beam term"
    );
}

/// **W2 holds**: the laser's damage against the line tank's armour is small by
/// definition, and one pulse cannot take a facet — which is what makes a laser a
/// thing you wear a tank down with.
#[test]
fn x1_a_laser_pulse_against_the_line_tanks_armour_is_small_by_definition() {
    let catalog = catalog();
    let laser = catalog
        .ammunition
        .get(&CanonicalId::new("tacsim.ammunition.laser_charge").expect("an id"))
        .expect("the charge");
    let he = catalog
        .ammunition
        .get(&CanonicalId::new("tacsim.ammunition.he_105").expect("an id"))
        .expect("the shell");
    let tank = catalog
        .specs
        .get(&CanonicalId::new(content::LINE_TANK).expect("an id"))
        .expect("the line tank");
    let thinnest = tank
        .sections
        .iter()
        .flat_map(|section| section.facets.iter())
        .map(|facet| facet.armor)
        .min()
        .expect("the tank has armour");
    assert!(
        laser.damage < he.damage,
        "a pulse is smaller than a 105 shell: {} against {}",
        laser.damage,
        he.damage
    );
    assert!(
        laser.damage < thinnest,
        "and smaller than the thinnest facet on the line tank, so no one pulse \
         takes one: {} against {thinnest}",
        laser.damage
    );

    // And through the pipeline: a pulse that lands spends its damage on armour and
    // leaves the facet standing.
    let mut arena = duel(&[]);
    let mut journal = Vec::new();
    let seq = arena.resolve_fire(RED, "laser").expect("the shot");
    let struck = arena
        .resolve_shot(RED, BLUE, "laser", 100_000, mush_atlas::Cover::None)
        .band
        .clone();
    arena.resolve_impact(seq, arena.now_ms, &mut journal);
    let spent: i64 = tank
        .sections
        .iter()
        .filter_map(|section| {
            let facet = section
                .facets
                .iter()
                .find(|facet| facet.id.as_str() == struck)?;
            let addr = mush_tactics::CellAddr::Facet {
                section: mush_tactics::SectionId::new(section.id.as_str()).ok()?,
                facet: mush_tactics::FacetId::new(facet.id.as_str()).ok()?,
                cell: mush_tactics::CellId::new("armor").ok()?,
            };
            Some(i64::from(facet.armor) - cell(&arena, BLUE, &addr)?)
        })
        .sum();
    assert!(
        spent == 0 || spent == i64::from(laser.damage),
        "the pulse either missed or spent exactly its damage on armour: {spent}"
    );
    assert!(
        !journal.iter().any(|(_, kind, _)| matches!(
            kind,
            tacsim::arena::ledger::EventKind::UnitDestroyed { .. }
        )),
        "and no pulse of this weapon is ever a kill by itself"
    );
}

/// **The check-in hands the heat back** (doc 16 §3.1's `CustodyIn`), because it is
/// one of the unit's cells and not a number the arena kept beside them.
#[test]
fn x1_the_heat_goes_home_with_the_cells() {
    let mut arena = duel(&[]);
    arena.resolve_fire(RED, "laser").expect("one pulse of heat");
    let returned: Vec<UnitReturn> = arena.returned_units();
    let red = returned
        .iter()
        .find(|unit| unit.unit == RED)
        .expect("the laser tank comes home");
    assert_eq!(
        red.cells.cell(&heat_addr(HEAT_CELL)).map(|c| c.current),
        Some(20),
        "the heat the match left it with"
    );
    let blue = returned
        .iter()
        .find(|unit| unit.unit == BLUE)
        .expect("the line tank comes home");
    assert!(
        blue.cells.cell(&heat_addr(HEAT_CELL)).is_none(),
        "and the stock tank comes home with the cells it went out with"
    );
}

/// **The cockpit's line carries the arena's own word** — the rule K.2 set for
/// `POWER_NOT_ONLINE` and W.1 set for `CYCLING` and `EMPTY`, read for the refusal
/// this cut adds.
///
/// It is pinned here rather than beside the other three because that test is one
/// this cut may not touch (16b §8 cut X); the property is the same property.
#[test]
fn x1_the_overheated_refusal_carries_the_arenas_own_word() {
    let word = Refusal::Overheated.as_str().replace('_', " ");
    assert!(
        cockpit::OVERHEATED.to_ascii_lowercase().contains(&word),
        "`{}` does not contain `{word}`",
        cockpit::OVERHEATED
    );
    // And the frame the cockpit reads it off answers the same question the arena
    // does, over the same pair of numbers.
    let mut arena = duel(&[]);
    for _ in 0..3 {
        ready(&mut arena);
        arena.resolve_fire(RED, "laser").expect("a pulse");
    }
    // The opening pulse: doc 16 §3.1 makes it establish the wall reference and
    // integrate nothing, so the frame it sends carries the heat the three shots
    // made and not that heat less the first substep's worth of cooling.
    let out = arena.step(0, &[], &[]);
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == mush_core::ObjId(1_000))
        .map(|(_, frame)| frame)
        .expect("the laser tank's pilot is sent a frame");
    let heat = frame.heat.expect("the frame carries the platform's heat");
    assert_eq!((heat.heat, heat.max), (60, 60));
    assert!(heat.overheated());
    assert!(
        arena.unit(RED).is_some_and(UnitState::overheated),
        "the cockpit and the arena agree"
    );
}

// ---------------------------------------------------------------------------
// X.2 — bipeds
// ---------------------------------------------------------------------------

/// The biped's cell on the marching window.
const WALK_CELL: (u16, u16) = (3, 3);

/// The tank's, nine cells east of it, so neither is ever near the other.
const DRIVE_CELL: (u16, u16) = (12, 3);

/// The legged mode's five numbers, as `content/units/biped_mk1.json` authors
/// them — and every one of them different from the tank's, which is what lets a
/// test tell which mode a unit is driving under.
const LEGGED: tacsim::arena::state::TrackedRules = tacsim::arena::state::TrackedRules {
    max_speed_cms: 1_200,
    accel_cms2: 160,
    brake_cms2: 240,
    turn_rate_mdeg_s: 45_000,
    reverse_factor_pct: 40,
};

/// **A biped and a stock line tank on open ground**, both awake, neither ordered
/// anywhere yet.
///
/// Nine cells apart and both facing north: this is a test about how two units
/// move, so they are placed where neither can ever be the reason the other
/// stopped.
fn march() -> (Catalog, Arena) {
    let catalog = biped_catalog();
    let mut arena = arena_of(
        &catalog,
        vec![
            unit_of(&catalog, content::BIPED_MK1, RED, Team::Red, WALK_CELL),
            unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, DRIVE_CELL),
        ],
        &[],
    );
    for id in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    (catalog, arena)
}

/// A stock tank four cells south of `target`, with the target turned to face it
/// so the band it presents is its `front`.
fn face_off(catalog: &Catalog, target: &str) -> Arena {
    let mut arena = arena_of(
        catalog,
        vec![
            unit_of(catalog, content::LINE_TANK, RED, Team::Red, RED_CELL),
            unit_of(catalog, target, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[],
    );
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose.heading_mdeg = 180_000;
        state.desired_heading_mdeg = 180_000;
    }
    arena
}

/// **Put one section in the state an impact leaves it in**: its internals at
/// their floor and its condition `destroyed`, in one plan through M32's own
/// `apply`.
///
/// It is exactly the pair of ops `arena/damage.rs`'s `plan_hit` builds for the
/// round that takes a section out, and it is spelled here because *which* section
/// a round takes is a weighted draw the arena owns (`choose_section`): a test
/// outside the crate can fire all day without being able to say which leg goes
/// first, and the rule under test is the consequence, not the draw.
fn destroy_section(arena: &mut Arena, unit: mush_core::ObjId, section: &str) {
    let id = mush_tactics::SectionId::new(section).expect("a section id");
    let addr = mush_tactics::CellAddr::Section {
        section: id.clone(),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
            .expect("a cell id"),
    };
    let before = arena.unit(unit).expect("the unit is in the arena");
    let floor = before
        .unit
        .cell(&addr)
        .map(|scalar| scalar.min)
        .expect("the section carries internals");
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            mush_tactics::UnitMutationOp::SetCell { addr, value: floor },
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section: id,
                condition: mush_tactics::ConditionId::new(tacsim::arena::damage::DESTROYED)
                    .expect("a condition id"),
            },
        ],
        None,
        None,
    )
    .expect("a two-op plan");
    let (post, _) = mush_tactics::apply(&before.unit, &plan, 0).expect("the plan applies");
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = post;
    }
}

/// Apply one section's authored consequences, in authored order, the way an
/// impact that emptied it does.
fn fire_consequences(arena: &mut Arena, unit: mush_core::ObjId, section: &str) {
    let consequences = arena
        .unit(unit)
        .and_then(|state| state.profile.section(section))
        .map(|section| section.consequences.clone())
        .expect("the section is in the unit's profile");
    let mut journal = Vec::new();
    for consequence in &consequences {
        arena.apply_consequence(unit, consequence, 0, &mut journal);
    }
}

/// **The extension changes nothing about the stock game** — doc 16 §9's own rule,
/// asserted for the biped as X.1 asserts it for the laser.
///
/// The shipped catalog is what `tests/definitions.rs` says it is, the biped pack
/// verifies and carries its pinned digest, and loading it beside the shipped
/// content leaves every shipped definition at the identity it already had. The
/// last two assertions are the ones this cut needs that X.1's did not: X.2
/// widened `UnitSpec::mobility` itself, so the tank's own digest is pinned
/// outright and its serialised spec is checked to carry no trace of the mode it
/// has not got.
#[test]
fn x2_the_biped_content_loads_beside_the_stock_game_without_moving_it() {
    if writing() {
        for relative in PACK_FILES_BIPED {
            resign(relative);
        }
        return;
    }

    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");

    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_BIPED.into_iter().collect();
    for (name, json) in content::EXTENSIONS_BIPED {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_BIPED.len());

    // With it loaded: one more unit schema, and not one thing else.
    let both = biped_catalog();
    assert_eq!(
        both.weapons.len(),
        2,
        "the biped carries the shipped guns and authors none of its own"
    );
    assert_eq!(both.specs.len(), 2, "the line tank and the biped");
    assert_eq!(
        both.units.len(),
        2,
        "and the tactics catalog carries both, so `@unit/create` resolves either"
    );
    assert_eq!(both.arenas.len(), 1, "the extension adds no arena");
    for (id, reference) in &stock.references {
        assert_eq!(
            both.references.get(id),
            Some(reference),
            "{id} moved when the extension loaded; a persisted unit points at it"
        );
    }

    // And the tank's identity is where it was across the *code* change too: a
    // unit persisted before this cut resolves its definition by this digest.
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved; a persisted unit needs a migration"
    );
    let spec = stock.specs.get(&tank).expect("the line tank is shipped");
    let json = serde_json::to_value(spec).expect("the spec serialises");
    assert_eq!(
        json.get("mobility")
            .and_then(serde_json::Value::as_object)
            .map(|mobility| mobility.keys().cloned().collect::<Vec<String>>()),
        Some(vec!["tracked".to_owned()]),
        "a unit with no legs serialises exactly the mobility it did before legs \
         existed"
    );
}

/// **The legged mode, as the content authors it and as M32 carries it** (doc 16
/// §9 item 2: *a `legged` mobility mode with its own rules*).
///
/// The spec declares legs and no tracks; the two readings of the five numbers —
/// the authored spec's and the instantiated unit's opaque `ModeRules` — agree,
/// which is the property `tests/definitions.rs` pins for the tank, read for a
/// unit that has no tracked mode to fall back on; and the mode reaches M32 under
/// its own locomotion and its own schema.
#[test]
fn x2_the_biped_declares_a_legged_mode_of_its_own() {
    let catalog = biped_catalog();
    let key = CanonicalId::new(content::BIPED_MK1).expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the biped entry");
    assert!(spec.mobility.tracked.is_none(), "a biped has no tracks");
    assert!(
        spec.mobility.legged.is_some(),
        "doc 16 §9 item 2's `legged`"
    );
    assert_eq!(tacsim::definitions::tracked_rules(spec), LEGGED);

    let binding = catalog.bindings.get(&key).expect("the biped binding");
    let unit = tacsim::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(21))
        .expect("the biped instantiates");
    assert_eq!(
        tacsim::match_::tracked_rules(&unit),
        LEGGED,
        "the two readings of one mode's rules agree for a legged unit too"
    );

    let profile = unit
        .mobility()
        .expect("the biped carries a mobility profile");
    assert_eq!(profile.modes().len(), 1, "one mode, and it is not tracked");
    let mode = profile.modes().first().expect("the legged mode");
    assert_eq!(mode.id.as_str(), "ground.legged");
    assert_eq!(mode.locomotion.as_str(), "legged");
    assert_eq!(
        mode.rules.schema().key().id.as_str(),
        tacsim::definitions::unit::LEGGED_RULES_SCHEMA,
        "its own schema, not the tracked mode's"
    );
    assert_eq!(
        mode.rules
            .data()
            .get("turn_rate_mdeg_s")
            .and_then(serde_json::Value::as_u64),
        Some(45_000),
        "and the five numbers are in the blob the game crate decodes"
    );

    // The stock tank is untouched by all of it.
    let tank = catalog
        .specs
        .get(&CanonicalId::new(content::LINE_TANK).expect("a canonical id"))
        .expect("the line tank");
    assert_eq!(
        tacsim::definitions::tracked_rules(tank).max_speed_cms,
        1_500
    );
    assert!(tank.mobility.legged.is_none());
}

/// **A biped moves under the legged rules** (doc 16 §9 item 2), through
/// `Arena::step` and doc 16 §3.4's four unchanged steps.
///
/// Two units on one window, ordered the same thing, answering with their own
/// numbers: the biped accelerates at 160 cm/s² where the tank accelerates at
/// 100, and turns at 45°/s where the tank slews at 30. The distance is pinned as
/// well as the speed, because "the mode reached motion" and "motion used it" are
/// two claims and the second is the one doc 16 §3.4 step 3 makes.
#[test]
fn x2_the_biped_walks_at_its_own_pace_beside_a_tank_driving_at_its() {
    let (_catalog, mut arena) = march();
    assert_eq!(arena.unit(RED).map(|state| state.mobility), Some(LEGGED));
    assert_eq!(
        arena.unit(BLUE).map(|state| state.mobility.max_speed_cms),
        Some(1_500),
        "the tank is checked out on its own mode, as ever"
    );

    // Both ordered flat out, north: two seconds is eight 250 ms substeps.
    for (id, ceiling) in [(RED, LEGGED.max_speed_cms), (BLUE, 1_500_u32)] {
        if let Some(state) = arena.unit_mut(id) {
            state.desired_speed_cms = i32::try_from(ceiling).expect("a speed");
        }
    }
    let start = arena.unit(RED).map(|state| state.pose.y_cm);
    let mut wall = 0_i64;
    run(&mut arena, &mut wall, 500, 4);
    assert_eq!(arena.now_ms, 2_000);
    assert_eq!(
        arena.unit(RED).map(|state| state.speed_cms),
        Some(320),
        "eight substeps at 160 cm/s²"
    );
    assert_eq!(
        arena.unit(BLUE).map(|state| state.speed_cms),
        Some(200),
        "and eight at the tank's 100, in the same eight substeps"
    );
    assert_eq!(
        arena
            .unit(RED)
            .map(|state| state.pose.y_cm)
            .zip(start)
            .map(|(now, start)| now - start),
        Some(360),
        "40 cm/s more each substep, 10 cm more of northing each substep"
    );

    // And the turn, on a fresh window so the heading is the only thing moving.
    let (_catalog, mut arena) = march();
    for id in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.desired_heading_mdeg = 90_000;
        }
    }
    let mut wall = 0_i64;
    run(&mut arena, &mut wall, 500, 1);
    assert_eq!(
        arena.unit(RED).map(|state| state.pose.heading_mdeg),
        Some(22_500),
        "45°/s over two 250 ms substeps"
    );
    assert_eq!(
        arena.unit(BLUE).map(|state| state.pose.heading_mdeg),
        Some(15_000),
        "the tank's 30°/s over the same two"
    );
}

/// **The taller silhouette, in the resolution arithmetic** (doc 16 §9 item 2:
/// *taller silhouettes and a taller `sensor_height_cm`*).
///
/// The same shooter, the same weapon, the same range, the same band and the same
/// error cone: the only thing that differs between the two shots is what the
/// target presents, and the biped presents half again the tank's frontage and
/// four times its height. Doc 16 §3.5 step 1 takes the angular size off the
/// *width*, so that is what moves the per-mille — and the flank tells the other
/// half of the story, where a biped is the thinner target of the two.
#[test]
fn x2_the_biped_is_a_taller_target_than_the_tank_and_an_easier_one_head_on() {
    let catalog = biped_catalog();
    let against_tank = face_off(&catalog, content::LINE_TANK);
    let against_biped = face_off(&catalog, content::BIPED_MK1);

    let tank = against_tank.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None);
    let biped = against_biped.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None);
    assert_eq!(
        tank.band, "front",
        "the target is turned to face the shooter"
    );
    assert_eq!(biped.band, "front");
    assert_eq!(
        (tank.presented_width_cm, biped.presented_width_cm),
        (350, 520)
    );
    assert_eq!(
        tank.error_urad, biped.error_urad,
        "one shooter, one weapon, one range: the cone is the same cone"
    );
    assert!(
        biped.angular_urad > tank.angular_urad
            && biped.presented_area_cm2 > tank.presented_area_cm2
            && biped.p_permille > tank.p_permille,
        "a bigger target is an easier shot: {} against {} per mille",
        biped.p_permille,
        tank.p_permille
    );

    // The height is authored and carried even though the width is what the
    // per-mille reads: eleven metres against two and a half.
    let heights = |arena: &Arena| {
        arena
            .unit(BLUE)
            .and_then(|state| state.profile.band("front"))
            .map(|band| band.height_cm)
    };
    assert_eq!(
        (heights(&against_tank), heights(&against_biped)),
        (Some(250), Some(1_100))
    );

    // From the flank it is the other way round, which is the fact that makes a
    // biped want to stand side on and a tank not.
    let flank = |arena: &Arena| {
        arena
            .unit(BLUE)
            .and_then(|state| state.profile.band("left"))
            .map(|band| band.width_cm)
    };
    assert_eq!(
        (flank(&against_tank), flank(&against_biped)),
        (Some(700), Some(340))
    );

    // And the sight sits nine metres up rather than one and a half (doc 16 §3.2
    // "Line of sight"), all the way through the check-out.
    assert_eq!(
        against_biped.unit(BLUE).map(|state| state.sensor_height_cm),
        Some(900)
    );
    assert_eq!(
        against_tank.unit(BLUE).map(|state| state.sensor_height_cm),
        Some(150)
    );
}

/// **The front band is the torso and the arms** (doc 16 §9 item 2), and an arm
/// that is shot off takes the gun it was carrying.
#[test]
fn x2_a_hit_on_the_front_finds_the_torso_and_the_arms() {
    let (_catalog, mut arena) = march();
    let mut struck: Vec<SmolStr> = Vec::new();
    for shot_seq in 0..400_u64 {
        let section = arena
            .choose_section(RED, "front", shot_seq)
            .expect("the front band strikes something");
        if !struck.contains(&section) {
            struck.push(section);
        }
    }
    struck.sort_unstable();
    assert_eq!(
        struck,
        vec![
            SmolStr::new("head"),
            SmolStr::new("left_arm"),
            SmolStr::new("right_arm"),
            SmolStr::new("torso"),
        ],
        "a shot from ahead finds the torso, the arms and the rare head — and \
         never a leg, which is struck from the flank and from behind"
    );

    // The guns are carried in the arms, so that is the section a `remove_weapons`
    // on an arm names — the weapon rows say `turret`, which this unit has not
    // got, and the check-out reads the section it was actually installed in.
    let mounts = |arena: &Arena, unit: mush_core::ObjId| {
        arena.unit(unit).map(|state| {
            state
                .weapons
                .iter()
                .map(|weapon| (weapon.id.to_string(), weapon.mount_section.to_string()))
                .collect::<Vec<(String, String)>>()
        })
    };
    assert_eq!(
        mounts(&arena, RED),
        Some(vec![
            ("cannon".to_owned(), "right_arm".to_owned()),
            ("machine_gun".to_owned(), "left_arm".to_owned()),
        ])
    );
    assert_eq!(
        mounts(&arena, BLUE),
        Some(vec![
            ("cannon".to_owned(), "turret".to_owned()),
            ("machine_gun".to_owned(), "turret".to_owned()),
        ]),
        "and the tank's are where they have always been"
    );

    destroy_section(&mut arena, RED, "right_arm");
    fire_consequences(&mut arena, RED, "right_arm");
    assert_eq!(
        mounts(&arena, RED),
        Some(vec![("machine_gun".to_owned(), "left_arm".to_owned())]),
        "the arm went and took the cannon with it"
    );
}

/// **One leg slows the biped and both stop it** (doc 16 §9 item 2: *§3.4
/// unchanged except the leg consequences*), through the authored consequences and
/// doc 16 §3.4's own motion.
///
/// The pair rule is authored on each leg, so the order they are lost in does not
/// matter, and it reads the M32 section conditions the impact plan sets: one leg
/// destroyed is `slow(50)` and a biped at half pace, the second is
/// `disable_motion` and a biped that does not move at all.
#[test]
fn x2_one_leg_slows_the_biped_and_both_legs_stop_it() {
    let (catalog, mut arena) = march();
    // The authored pair, as the content states it.
    let authored = catalog
        .specs
        .get(&CanonicalId::new(content::BIPED_MK1).expect("a canonical id"))
        .and_then(|spec| {
            spec.section(&CanonicalId::new("left_leg").expect("a canonical id"))
                .map(|section| section.consequences.clone())
        })
        .expect("the left leg is authored");
    assert_eq!(
        authored
            .iter()
            .map(tacsim::definitions::unit::Consequence::label)
            .collect::<Vec<&str>>(),
        vec!["slow", "disable_motion_when_all_of"],
        "doc 16 §9 item 2's two leg consequences, in authored order"
    );

    if let Some(state) = arena.unit_mut(RED) {
        state.desired_speed_cms = i32::try_from(LEGGED.max_speed_cms).expect("a speed");
    }
    let mut wall = 0_i64;
    run(&mut arena, &mut wall, 500, 15);
    assert_eq!(
        arena.unit(RED).map(|state| state.speed_cms),
        Some(1_200),
        "thirty substeps at 40 cm/s each is exactly the walking ceiling"
    );

    // One leg: slowed, and still walking.
    destroy_section(&mut arena, RED, "left_leg");
    fire_consequences(&mut arena, RED, "left_leg");
    assert_eq!(arena.unit(RED).map(|state| state.limits.slow_pct), Some(50));
    assert_eq!(
        arena.unit(RED).map(|state| state.limits.motion_disabled),
        Some(false),
        "one leg is a limp, not a stop"
    );
    // **16b cut F.2**: and the line the pilot reads says only what is true. The
    // `section_destroyed` ledger line carries both authored words, because the
    // black box records the content; the notice is built from the consequences
    // that actually fired, so a biped still walking is not told it cannot move.
    assert_eq!(
        arena.fired_consequences(RED, "left_leg"),
        vec![smol_str::SmolStr::new("slow")],
        "the pair rule has not fired: the pilot is told the limp and nothing else"
    );
    run(&mut arena, &mut wall, 500, 8);
    assert_eq!(
        arena.unit(RED).map(|state| state.speed_cms),
        Some(600),
        "half of the walking ceiling, reached at the braking rate"
    );
    let limping = arena.unit(RED).map(|state| state.pose.y_cm);
    run(&mut arena, &mut wall, 500, 2);
    assert!(
        arena.unit(RED).map(|state| state.pose.y_cm) > limping,
        "and it is still covering ground"
    );

    // The second leg: stopped, where it stands, at once.
    destroy_section(&mut arena, RED, "right_leg");
    fire_consequences(&mut arena, RED, "right_leg");
    assert_eq!(
        arena.unit(RED).map(|state| state.limits.motion_disabled),
        Some(true),
        "doc 16 §9 item 2: `disable_motion` for both"
    );
    assert_eq!(
        arena.fired_consequences(RED, "right_leg"),
        vec![
            smol_str::SmolStr::new("slow"),
            smol_str::SmolStr::new("disable_motion_when_all_of"),
        ],
        "both legs are gone, so now the pilot is told the tank cannot move"
    );
    assert_eq!(arena.unit(RED).map(|state| state.speed_cms), Some(0));
    let stopped = arena.unit(RED).map(|state| state.pose.y_cm);
    run(&mut arena, &mut wall, 500, 8);
    assert_eq!(
        arena.unit(RED).map(|state| state.pose.y_cm),
        stopped,
        "a unit with no legs is an obstacle, and doc 16 §3.4 step 2 forces its \
         speed to zero every substep"
    );
    // It is not a wreck: doc 10 3A's disabled precedes destroyed, and a biped
    // that has lost its legs is still a gun that works.
    assert_eq!(arena.unit(RED).map(|state| state.limits.wreck), Some(false));
    assert!(
        arena
            .unit(RED)
            .is_some_and(|state| !state.weapons.is_empty())
    );
}

/// **A unit with no hull is still a wreck when it carries the mark** (16b cut
/// X.2's one widening of the damage pipeline).
///
/// Doc 16 §3.3's wreck is a named condition on the unit's body, and `wreck_ops`
/// has always put it on the hull *or on the triggering section* for a unit that
/// has no hull. A biped is the first unit in the tree with no hull, so this is
/// the first time the second half of that sentence has been true of the
/// predicate as well as of the plan.
#[test]
fn x2_a_biped_with_no_hull_still_reads_as_a_wreck() {
    let (_catalog, arena) = march();
    assert!(
        arena
            .unit(RED)
            .is_some_and(|state| state.profile.section("hull").is_none()),
        "a biped has a torso and no hull"
    );
    let unit = arena.unit(RED).expect("the biped").unit.clone();
    assert!(!tacsim::arena::damage::is_wreck(&unit));
    let marked = mush_tactics::apply(
        &unit,
        &mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetSectionCondition {
                section: mush_tactics::SectionId::new("torso").expect("a section"),
                condition: mush_tactics::ConditionId::new(tacsim::arena::damage::WRECK_CONDITION)
                    .expect("a condition"),
            }],
            None,
            None,
        )
        .expect("a one-op plan"),
        0,
    )
    .map(|(post, _)| post)
    .expect("the mark applies");
    assert!(
        tacsim::arena::damage::is_wreck(&marked),
        "the condition is the mark, wherever the unit's body is"
    );
}

// ---------------------------------------------------------------------------
// X.3 — the thermal sight
// ---------------------------------------------------------------------------

/// The observer's cell on the thermal window.
const WATCH_CELL: (u16, u16) = (8, 2);

/// The target's, ten cells due north of it — 100 m.
const MARK_CELL: (u16, u16) = (8, 12);

/// A second target cell four cells out — 40 m, inside even a cold tank's reach.
const NEAR_CELL: (u16, u16) = (8, 6);

/// **What the mount authors** (`content/units/line_tank_thermal.json`), so the
/// arithmetic below is read off the content rather than restated by it.
const POWERED_PERMILLE: u32 = 650;
/// The factor a shut-down target presents to it.
const COLD_PERMILLE: u32 = 120;

/// **One sensor pass, applied directly**, with the spatial index rebuilt first —
/// the order `Arena::step`'s phase 3 uses (`tests/perception.rs` drives its own
/// properties the same way).
///
/// Driving `sensor_pass` rather than the pulse is deliberate for a property about
/// *what is detected*: the target's power and speed can be put exactly where the
/// property needs them, and the pulse's own wiring is asserted by cut S's tests.
fn thermal_pass(
    arena: &mut Arena,
    observer: mush_core::ObjId,
    pass_seq: u64,
    t_ms: i64,
) -> tacsim::arena::perception::PerceptionCounters {
    let mut counters = tacsim::arena::perception::PerceptionCounters::default();
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    let units = arena.units.clone();
    arena.index.rebuild(&arena.definition.terrain, &units);
    arena.sensor_pass(observer, pass_seq, t_ms, &mut counters, &mut journal);
    counters
}

/// **An observer of `watcher` looking at a stock line tank**, both checked out
/// the way production checks one out, on the open window.
fn watch(catalog: &Catalog, watcher: &str, target: (u16, u16)) -> Arena {
    let mut arena = arena_of(
        catalog,
        vec![
            unit_of(catalog, watcher, RED, Team::Red, WATCH_CELL),
            unit_of(catalog, content::LINE_TANK, BLUE, Team::Blue, target),
        ],
        &[],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    arena
}

/// Put the target in one of doc 16 §9 item 3's three states.
fn target_state(arena: &mut Arena, power: Power, speed_cms: i32) {
    if let Some(state) = arena.unit_mut(BLUE) {
        state.power = power;
        state.speed_cms = speed_cms;
    }
}

/// Whether the observer is holding the target after one pass.
fn holds(arena: &mut Arena) -> bool {
    thermal_pass(arena, RED, 1, 0);
    let id = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
    arena.contacts.resolve(RED, id).is_some()
}

/// **The extension changes nothing about the stock game** — doc 16 §9's own rule,
/// asserted for the thermal sight as X.1 asserts it for the laser.
///
/// The shipped catalog is what `tests/definitions.rs` says it is, both packs
/// verify and carry their pinned digests, and loading them beside the shipped
/// content leaves every shipped definition at the identity it already had. The
/// last two assertions are this cut's own: X.3 widened `ComponentRole::Sensor`,
/// which is a field of the unit spec and therefore of the tank's own payload, so
/// the tank's digest is pinned outright and its serialised mount is checked to
/// carry no trace of the channel it has not got.
#[test]
fn x3_the_thermal_content_loads_beside_the_stock_game_without_moving_it() {
    if writing() {
        for relative in PACK_FILES_THERMAL {
            resign(relative);
        }
        return;
    }

    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");
    assert_eq!(stock.sensors.len(), 1, "one sight");

    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_THERMAL.into_iter().collect();
    for (name, json) in content::EXTENSIONS_THERMAL {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_THERMAL.len());

    // With them loaded: one more sight, one more unit schema, and not one thing
    // else — no weapon, no ammunition, no arena.
    let both = thermal_catalog();
    assert_eq!(
        both.sensors.len(),
        2,
        "the visual sight and the thermal one"
    );
    assert_eq!(both.specs.len(), 2, "the line tank and its thermal variant");
    assert_eq!(both.units.len(), 2, "and the tactics catalog carries both");
    assert_eq!(
        both.weapons.len(),
        2,
        "the thermal tank carries the shipped guns and authors none of its own"
    );
    assert_eq!(both.arenas.len(), 1, "the extension adds no arena");
    for (id, reference) in &stock.references {
        assert_eq!(
            both.references.get(id),
            Some(reference),
            "{id} moved when the extension loaded; a persisted unit points at it"
        );
    }

    // And the tank's identity is where X.2 pinned it across this *code* change
    // too: `ComponentRole::Sensor` grew a field, and the `skip_serializing_if` is
    // what keeps a mount that authors no thermal channel the bytes it was.
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved; a persisted unit needs a migration"
    );
    let spec = stock.specs.get(&tank).expect("the line tank is shipped");
    let json = serde_json::to_value(spec).expect("the spec serialises");
    let mount = json
        .get("components")
        .and_then(serde_json::Value::as_array)
        .and_then(|components| {
            components.iter().find_map(|component| {
                let role = component.get("role")?.as_object()?;
                (role.get("kind").and_then(serde_json::Value::as_str) == Some("sensor"))
                    .then(|| role.keys().cloned().collect::<Vec<String>>())
            })
        });
    assert_eq!(
        mount,
        Some(vec!["kind".to_owned(), "sensor".to_owned()]),
        "a sight with no thermal channel serialises exactly the mount it did \
         before the channel existed"
    );
}

/// **The shipped sight's signature table, byte for byte** (doc 16 §9: *a test
/// that passes without changing A1*), before and after the extension loads.
///
/// The reach of every sensor pass in the game is this table times the sensor's
/// range, so it is the one thing this cut must be able to say it did not touch —
/// in bytes, and against a string written down here rather than against itself.
#[test]
fn x3_the_shipped_sights_signature_table_is_untouched_by_the_extension() {
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let both = thermal_catalog();
    let id = CanonicalId::new("tacsim.sensor.visual_mk1").expect("a canonical id");
    let table = |catalog: &Catalog| {
        let def = catalog.sensors.get(&id).expect("the shipped sight").clone();
        serde_json::to_string(&def.signature).expect("the table serialises")
    };
    assert_eq!(
        table(&stock),
        VISUAL_MK1_SIGNATURE,
        "the shipped table moved"
    );
    assert_eq!(
        table(&both),
        VISUAL_MK1_SIGNATURE,
        "the extension moved the shipped table by loading"
    );
    // And the whole row with it: the range, the attenuation, the cadences and the
    // two hooks the ECM and ECCM extensions own are the numbers cut C.2 authored.
    let def = stock.sensors.get(&id).expect("the shipped sight");
    assert_eq!(
        (
            def.max_range_cells,
            def.max_attenuation,
            def.classify_after_ms,
            def.share_range_cells,
            def.sees_wrecks
        ),
        (60, 40, 3_000, 30, false)
    );
    assert!(def.emission.is_none() && def.jamming.is_none());
}

/// **The thermal sight is its own row** (doc 16 §9 item 3: *a sensor row with its
/// own range and signature factors*), and the mount carries the new term.
///
/// Its range is not the eye's, its attenuation budget is not the eye's, and its
/// signature table is deliberately *flat* where the eye's falls away: woods hide a
/// silhouette and do very little to a heat source. The term that replaces that
/// fall is the one keyed on the target's power and speed, and it reaches the arena
/// on the unit's own cells — `None` for the stock tank, which is what makes it an
/// `Option` test per candidate for every unit the game ships.
#[test]
fn x3_the_thermal_sight_declares_its_own_range_and_signature_factors() {
    let catalog = thermal_catalog();
    let thermal = catalog
        .sensors
        .get(&CanonicalId::new(content::THERMAL_MK1).expect("a canonical id"))
        .expect("the thermal sight");
    let visual = catalog
        .sensors
        .get(&CanonicalId::new("tacsim.sensor.visual_mk1").expect("a canonical id"))
        .expect("the shipped sight");
    assert_eq!(thermal.max_range_cells, 45, "its own range");
    assert!(
        thermal.max_range_cells < visual.max_range_cells,
        "a thermal channel is shorter-legged than a daylight optic"
    );
    assert_eq!(
        thermal.max_attenuation, 30,
        "one belt of light woods, and no canopy"
    );
    assert!(
        thermal.classify_after_ms > visual.classify_after_ms,
        "a hot blob takes longer to tell apart than an outline"
    );
    let stopped = |def: &tacsim::definitions::SensorDef, class: TerrainClass| {
        def.signature
            .iter()
            .find(|row| row.class == class)
            .map(|row| row.stopped_permille)
    };
    assert_eq!(stopped(thermal, TerrainClass::WoodsHeavy), Some(880));
    assert_eq!(stopped(visual, TerrainClass::WoodsHeavy), Some(500));

    // The mount's term, decoded at check-out onto the arena's unit, and absent
    // from the stock tank beside it.
    let arena = watch(&catalog, content::LINE_TANK_THERMAL, MARK_CELL);
    let band = arena
        .unit(RED)
        .and_then(|state| state.thermal)
        .expect("the thermal tank's sight carries the channel");
    assert_eq!(band.powered_permille, POWERED_PERMILLE);
    assert_eq!(band.cold_permille, COLD_PERMILLE);
    assert_eq!(
        arena.unit(BLUE).and_then(|state| state.thermal),
        None,
        "the stock sight has no thermal channel"
    );
    // The ladder itself: under way is the plume, stopped is the idle, and
    // anything not online is cold metal.
    assert_eq!(band.permille(Power::Online, 1_000), 1_000);
    assert_eq!(band.permille(Power::Online, 0), POWERED_PERMILLE);
    assert_eq!(band.permille(Power::Off, 0), COLD_PERMILLE);
    assert_eq!(band.permille(Power::Starting, 0), COLD_PERMILLE);
}

/// **The acceptance** (doc 16 §9 item 3): a thermal sight holds a moving tank at a
/// range where a stopped, powered-down one is invisible — and the visual sight
/// beside it, at that same range, holds all three.
///
/// Ten cells of open ground, 100 m. The sight reaches 45 cells at factor 1,000 and
/// `ground` costs it almost nothing, so the arithmetic is the thermal term's
/// alone: 45,000 cm against a tank under way, about 28,600 against an idling one,
/// about 5,200 against a cold one. The stock tank's eye reaches 54,000 cm at the
/// same range whatever the target's power is doing, which is doc 16 §9's *without
/// changing A1* stated as a fact about this fixture.
#[test]
fn x3_a_thermal_sight_holds_a_moving_tank_where_a_cold_one_is_invisible() {
    let catalog = thermal_catalog();
    for (power, speed_cms, held, what) in [
        (Power::Online, 1_000_i32, true, "a tank under way"),
        (Power::Online, 0, true, "an idling tank"),
        (Power::Off, 0, false, "a tank that has shut down"),
    ] {
        let mut arena = watch(&catalog, content::LINE_TANK_THERMAL, MARK_CELL);
        target_state(&mut arena, power, speed_cms);
        assert_eq!(
            holds(&mut arena),
            held,
            "the thermal sight answered the wrong way about {what} ten cells out"
        );

        // The same three states, seen by the sight the game ships: the eye does
        // not care what the target's power is doing, and this cut did not teach
        // it to.
        let mut eye = watch(&catalog, content::LINE_TANK, MARK_CELL);
        target_state(&mut eye, power, speed_cms);
        assert!(
            holds(&mut eye),
            "the visual sight lost {what} ten cells out"
        );
    }

    // And the cold tank is not invisible everywhere — it is invisible at range.
    // Four cells out is inside the 5,200 cm the term leaves, so the same sight
    // holds the same shut-down tank.
    let mut near = watch(&catalog, content::LINE_TANK_THERMAL, NEAR_CELL);
    target_state(&mut near, Power::Off, 0);
    assert!(
        holds(&mut near),
        "a cold tank is dark at a hundred metres, not at forty"
    );
}

/// **No new trace** (doc 16 §9 item 3's last three words).
///
/// The term is a multiply inside step 3's range test, which runs *before* step 2's
/// line of sight: a candidate the sight cannot see for heat is culled by
/// arithmetic over one cell read, and the trace the doc bounds per pulse (W4) is
/// never walked. The counters say it in the only way that is not an estimate.
#[test]
fn x3_the_cold_target_costs_the_pass_a_candidate_and_no_trace() {
    let catalog = thermal_catalog();
    let mut cold = watch(&catalog, content::LINE_TANK_THERMAL, MARK_CELL);
    target_state(&mut cold, Power::Off, 0);
    let counters = thermal_pass(&mut cold, RED, 1, 0);
    assert_eq!(counters.passes, 1);
    assert_eq!(counters.candidates, 1, "the index still offered it");
    assert_eq!(counters.traces, 0, "and the range test culled it first");
    assert_eq!(counters.cell_probes, 0);
    assert_eq!(counters.detected, 0);

    let mut hot = watch(&catalog, content::LINE_TANK_THERMAL, MARK_CELL);
    target_state(&mut hot, Power::Online, 1_000);
    let counters = thermal_pass(&mut hot, RED, 1, 0);
    assert_eq!(counters.candidates, 1);
    assert_eq!(counters.traces, 1, "the same candidate, traced once");
    assert!(counters.cell_probes > 0);
    assert_eq!(counters.detected, 1);
}

/// **The perception counters of a fixture are byte-identical before and after the
/// extension loads** (16b §8 cut X: *the story test and the harness unchanged
/// before and after each*), which is the cost claim as well as the behaviour one.
///
/// Two stock tanks, the same window, the same seed, four passes each way: the same
/// passes, candidates, traces, cell probes, detections and classifications, and
/// the same contact held at the end. The term is inside the `Option` and not a
/// multiply by 1,000 outside it, so a stock match runs the arithmetic it ran
/// before this cut to the centimetre — and this fixture is where that would show.
#[test]
fn x3_a_stock_fixtures_pass_is_counter_identical_when_the_extension_loads() {
    let run_pair = |catalog: &Catalog| {
        let mut arena = watch(catalog, content::LINE_TANK, MARK_CELL);
        target_state(&mut arena, Power::Online, 0);
        let mut total = tacsim::arena::perception::PerceptionCounters::default();
        for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
            for observer in [RED, BLUE] {
                total.absorb(&thermal_pass(&mut arena, observer, seq, t_ms));
            }
        }
        let id = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
        let held = arena.contacts.resolve(RED, id).copied();
        (total, held)
    };
    let stock = run_pair(&catalog_of(&[]));
    let with_thermal = run_pair(&thermal_catalog());
    assert_eq!(
        stock.0, with_thermal.0,
        "the extension moved a stock fixture's perception cost"
    );
    assert_eq!(
        stock.1.map(|held| held.class),
        with_thermal.1.map(|held| held.class),
        "the extension moved what a stock fixture sees"
    );
    assert!(stock.0.traces > 0, "the fixture traced at all");
}

/// **A factor past 1,000 is refused by name** (16b cut X.3), the way cut S.1's
/// signature rows are.
///
/// The new term shortens a reach and never lengthens one — that is
/// `[tacsim.sensors].max_signature_factor`'s job and not content's — so the bound
/// is re-run where every other component bound is, and the refusal names the
/// component and the number. The spec under test is the authored thermal tank's
/// own, decoded and then spoiled, so this is a test about the bound and not about
/// a hand-made unit.
#[test]
fn x3_a_thermal_factor_past_a_thousand_is_refused_naming_the_mount() {
    let pack: serde_json::Value =
        serde_json::from_str(content::LINE_TANK_THERMAL_JSON).expect("the pack is json");
    let def = pack
        .pointer("/entries/tacsim.unit.line_tank_thermal/def")
        .expect("the unit entry")
        .clone();
    let spec: tacsim::definitions::UnitSpec =
        serde_json::from_value(def.clone()).expect("the authored spec decodes");
    spec.check("authored").expect("the authored spec is valid");

    let mut spoiled = def;
    let mount = spoiled
        .get_mut("components")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|components| {
            components.iter_mut().find(|component| {
                component.get("id").and_then(serde_json::Value::as_str) == Some("thermal_sensor")
            })
        })
        .and_then(|component| component.pointer_mut("/role/thermal/powered_permille"))
        .expect("the authored mount carries the term");
    *mount = serde_json::json!(1_001);
    let spec: tacsim::definitions::UnitSpec =
        serde_json::from_value(spoiled).expect("the spoiled spec still decodes");
    let refused = spec
        .check("spoiled")
        .expect_err("a factor past 1,000 is refused");
    let text = refused.to_string();
    assert!(
        text.contains("thermal_sensor")
            && text.contains("powered_permille")
            && text.contains("1001"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// X.4 — ECM
// ---------------------------------------------------------------------------

/// The unit that carries the jamming pod, when a fixture fields one beside the
/// observer and the target.
const POD: mush_core::ObjId = mush_core::ObjId(7);

/// The pod's cell: five cells from the observer at [`WATCH_CELL`] and five from
/// the target at [`MARK_CELL`], so **both ends of the look** stand inside its
/// six-cell bubble — which is the fixture doc 16 §9 item 4's own sentence needs.
const POD_CELL: (u16, u16) = (8, 7);

/// A cell the pod reaches neither end from: about 139 m from [`WATCH_CELL`], where
/// a six-cell bubble is a pod parked in the corner of the window.
const POD_PARKED_CELL: (u16, u16) = (1, 14);

/// Where the observer stands to be **outside** the bubble while the target stays
/// inside it: seven cells from the pod, twelve from the target.
const OUTSIDE_CELL: (u16, u16) = (8, 0);

/// **What the pod authors** (`content/sensors/ecm_mk1.json`), so the arithmetic
/// below is read off the content rather than restated by it.
const JAM_RADIUS_CELLS: u16 = 6;
/// What it takes out of an observer's attenuation budget — one belt of light woods
/// on the atlas's own opacity table.
const JAM_ATTENUATION_COST: u8 = 25;
/// What it leaves of every target's signature factor, per mille.
const JAM_SIGNATURE_PERMILLE: u32 = 150;

/// The Mk1 visual sight's reach against a tank stopped on open ground: 60 cells at
/// the `ground` row's 900 per mille.
const EYE_REACH_CM: u64 = 54_000;

/// The shipped content plus the pod and its tank (16b cut X.4).
fn ecm_catalog() -> Catalog {
    catalog_of(&content::extension_sources_of(&content::EXTENSIONS_ECM))
}

/// One sensor pass with the spatial index rebuilt first — X.3's [`thermal_pass`],
/// under the name this section reads by. The jam scan reads the same index the
/// candidate cull does, so the rebuild is what makes a pass driven directly the
/// pass `Arena::step`'s phase 3 runs.
fn jam_pass(
    arena: &mut Arena,
    observer: mush_core::ObjId,
    pass_seq: u64,
    t_ms: i64,
) -> tacsim::arena::perception::PerceptionCounters {
    thermal_pass(arena, observer, pass_seq, t_ms)
}

/// **An observer of `watcher` at `at`, a stock line tank at `target`, and the
/// pod's tank at `pod` when a fixture fields one**, every unit checked out the way
/// production checks one out.
///
/// The observer and the pod are powered up, because a pod is an emitter and a hull
/// that has not started up radiates nothing; the target is left as it was checked
/// out, which is stopped and cold — and nothing here reads either, because the Mk1
/// sight has no thermal channel.
fn jam_field(
    catalog: &Catalog,
    watcher: &str,
    at: (u16, u16),
    target: (u16, u16),
    pod: Option<(u16, u16)>,
    paint: &[(u16, u16, TerrainCell)],
) -> Arena {
    let mut units = vec![
        unit_of(catalog, watcher, RED, Team::Red, at),
        unit_of(catalog, content::LINE_TANK, BLUE, Team::Blue, target),
    ];
    if let Some(cell) = pod {
        units.push(unit_of(
            catalog,
            content::LINE_TANK_ECM,
            POD,
            Team::Blue,
            cell,
        ));
    }
    let mut arena = arena_of(catalog, units, paint);
    for id in [RED, POD] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    arena
}

/// **The extension changes nothing about the stock game** — doc 16 §9's own rule,
/// asserted for the pod as X.1 asserts it for the laser.
///
/// The last assertions are this cut's own, and they are about a *code* change
/// rather than a content one: `SensorDef::jamming` stopped being an opaque
/// `Option<Value>` and became a typed `Option<JammingDef>`, and what keeps every
/// shipped sight's payload — and therefore its `DefinitionRef`, and therefore every
/// persisted unit that points at one — exactly where it was is that a `None` still
/// serialises as `"jamming": null`. The row's whole key set is pinned here so that
/// a later field added to it cannot slip past unreviewed.
#[test]
fn x4_the_ecm_content_loads_beside_the_stock_game_without_moving_it() {
    if writing() {
        for relative in PACK_FILES_ECM {
            resign(relative);
        }
        return;
    }

    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");
    assert_eq!(stock.sensors.len(), 1, "one sight");

    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_ECM.into_iter().collect();
    for (name, json) in content::EXTENSIONS_ECM {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_ECM.len());

    // With them loaded: one more sensor row, one more unit schema, and not one
    // thing else — no weapon, no ammunition, no arena.
    let both = ecm_catalog();
    assert_eq!(both.sensors.len(), 2, "the Mk1 sight and the pod");
    assert_eq!(both.specs.len(), 2, "the line tank and its ECM variant");
    assert_eq!(both.units.len(), 2, "and the tactics catalog carries both");
    assert_eq!(
        both.weapons.len(),
        2,
        "the ECM tank carries the shipped guns and authors none of its own"
    );
    assert_eq!(both.arenas.len(), 1, "the extension adds no arena");
    for (id, reference) in &stock.references {
        assert_eq!(
            both.references.get(id),
            Some(reference),
            "{id} moved when the extension loaded; a persisted unit points at it"
        );
    }

    // The tank's identity, where X.2 pinned it, across a change to the *sensor*
    // row's type — which the tank's own payload carries a reference to.
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved; a persisted unit needs a migration"
    );

    // And the shipped sight's row, key for key: `jamming` is still the null hook
    // doc 16 §3.6 requires of a row that is not a pod, and it still serialises as
    // `null` now that it has a type.
    let sight = stock
        .sensors
        .get(&CanonicalId::new("tacsim.sensor.visual_mk1").expect("a canonical id"))
        .expect("the shipped sight");
    assert!(sight.jamming.is_none() && sight.emission.is_none());
    let json = serde_json::to_value(sight).expect("the row serialises");
    let row = json.as_object().expect("a row is an object");
    assert!(
        row.get("jamming").is_some_and(serde_json::Value::is_null),
        "a sight with no jamming block no longer serialises as null"
    );
    assert!(row.get("emission").is_some_and(serde_json::Value::is_null));
    let keys: Vec<&str> = row.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "classify_after_ms",
            "emission",
            "jamming",
            "label",
            "max_attenuation",
            "max_range_cells",
            "note",
            "sees_wrecks",
            "share_range_cells",
            "signature",
        ],
        "the sensor row's shape moved; every shipped sight's digest moved with it"
    );
    // The signature table X.3 pinned is untouched by this cut as well.
    assert_eq!(
        serde_json::to_string(&sight.signature).expect("the table serialises"),
        VISUAL_MK1_SIGNATURE
    );
}

/// **The pod is a sensor row whose `jamming` is non-null** (doc 16 §9 item 4's own
/// words), reaching the arena as cells on the component that radiates it.
///
/// And mounting one does not change what the tank *sees* with: `sensor_profile`
/// reads the first `sensor` mount, the pod is an `ecm` mount, and the ECM tank
/// looks out of the same Mk1 sight the stock tank does.
#[test]
fn x4_the_pod_is_a_sensor_row_whose_jamming_block_is_non_null() {
    let catalog = ecm_catalog();
    let pod = catalog
        .sensors
        .get(&CanonicalId::new(content::ECM_MK1).expect("a canonical id"))
        .expect("the pod's row");
    assert_eq!(
        pod.jamming,
        Some(tacsim::definitions::sensor::JammingDef {
            radius_cells: JAM_RADIUS_CELLS,
            attenuation_cost: JAM_ATTENUATION_COST,
            signature_permille: JAM_SIGNATURE_PERMILLE,
        }),
        "doc 16 §9 item 4: the `jamming` field becomes non-null"
    );
    assert!(pod.emission.is_none(), "the other hook stays null");

    // The cells, decoded at check-out onto the arena's own unit, in centimetres.
    let arena = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let jammer = arena
        .unit(POD)
        .and_then(|state| state.jammer)
        .expect("the ECM tank carries a pod");
    assert_eq!(
        jammer,
        tacsim::arena::perception::Jammer {
            radius_cm: u32::from(JAM_RADIUS_CELLS) * u32::from(CELL_CM),
            attenuation_cost: JAM_ATTENUATION_COST,
            signature_permille: JAM_SIGNATURE_PERMILLE,
        }
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.jammer),
        None,
        "a stock tank carries no pod"
    );
    assert_eq!(
        arena.jam_radius_cm,
        u32::from(JAM_RADIUS_CELLS) * u32::from(CELL_CM),
        "the match's scan radius is the widest bubble on the field"
    );
    assert_eq!(
        jam_field(
            &catalog,
            content::LINE_TANK,
            WATCH_CELL,
            MARK_CELL,
            None,
            &[]
        )
        .jam_radius_cm,
        0,
        "a field with no pod never scans for one"
    );

    // The pod is not a sight: the ECM tank looks out of the Mk1 visual sight, and
    // its own row's floor numbers reach nothing.
    let looking = arena
        .unit(POD)
        .and_then(|state| state.sensor.as_ref())
        .expect("the ECM tank sees with something");
    assert_eq!(
        (looking.max_range_cells, looking.max_attenuation),
        (60, 40),
        "mounting a pod changed what the tank sees with"
    );

    // The two terms, as arithmetic, on their own.
    let jamming = tacsim::arena::perception::Jamming {
        attenuation_cost: u32::from(JAM_ATTENUATION_COST),
        signature_permille: JAM_SIGNATURE_PERMILLE,
    };
    assert_eq!(jamming.budget(40), 15, "40 of budget less the pod's 25");
    assert_eq!(jamming.budget(20), 0, "and it floors at zero, never wraps");
    assert_eq!(
        jamming.scale(EYE_REACH_CM),
        8_100,
        "540 m of reach becomes 81 m"
    );
}

/// **The acceptance** (doc 16 §9 item 4): a target inside the bubble is lost to an
/// observer whose budget the pod is eating, and **the same observer outside the
/// radius sees as before**.
///
/// Ten cells of open ground, 100 m, with the pod parked halfway between: the Mk1
/// sight reaches 54,000 cm against a tank stopped on `ground` and the pod leaves
/// 8,100 of it, so the target 10,000 cm out is gone and the pod 5,000 cm out is the
/// one thing still on the scope — which is the shape of a fight inside a bubble.
///
/// Then the same field with the observer stepped back out of the bubble: the target
/// is still *in* it, twelve cells away now instead of ten, and it is held exactly as
/// it was. That is the reading this cut implements, asserted in the one way that
/// tells it apart from the other: the pod degrades what is looked *from*, not what
/// is looked *at*.
#[test]
fn x4_a_target_in_the_bubble_is_lost_from_inside_it_and_held_from_outside() {
    let catalog = ecm_catalog();

    // Inside: the pod eats the reach and the target is gone.
    let mut inside = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    assert!(
        !holds(&mut inside),
        "the observer inside the bubble still held a target 100 m out"
    );
    let pod_contact = tacsim::arena::rng::draw_contact_id(inside.seed, RED, POD);
    assert!(
        inside.contacts.resolve(RED, pod_contact).is_some(),
        "the pod itself is 50 m out and inside what the jamming leaves"
    );

    // Outside: the same pod, the same target still inside its bubble, and the
    // observer two cells further back — held, as it was before the pod existed.
    let mut outside = jam_field(
        &catalog,
        content::LINE_TANK,
        OUTSIDE_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    assert!(
        holds(&mut outside),
        "an observer outside the bubble lost a target that is inside it"
    );

    // And with the pod parked in the far corner, the field answers exactly as a
    // field with no pod at all does.
    for pod in [Some(POD_PARKED_CELL), None] {
        let mut clear = jam_field(
            &catalog,
            content::LINE_TANK,
            WATCH_CELL,
            MARK_CELL,
            pod,
            &[],
        );
        assert!(
            holds(&mut clear),
            "a pod 139 m away jammed an observer it cannot reach"
        );
    }

    // The target is not invisible everywhere — it is invisible at range. Four cells
    // out is inside the 8,100 cm the jamming leaves, so the same observer under the
    // same pod holds the same tank: what ECM buys is that it has to come closer,
    // which is *detected later* read as a fact about a fixture.
    let mut near = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        NEAR_CELL,
        Some(POD_CELL),
        &[],
    );
    assert!(
        holds(&mut near),
        "a jammed sight is blind at a hundred metres, not at forty"
    );
}

/// **The budget half, on its own** (doc 16 §9 item 4: *adds to the observer's
/// `max_attenuation` budget*), with the pod on the observer's own hull.
///
/// A belt of light woods on the line and a target four cells out, which is well
/// inside what the jamming leaves of the reach — so step 3 passes, the trace is
/// walked, and the only thing that can refuse the detection is step 2's budget: 40
/// of it against the belt's 25 is a detection, and 15 is not.
///
/// The pod is the observer's **own**, which is the other half of the reading: a
/// bubble is a bubble, the unit carrying the pod is standing in it, and there is no
/// exception for the tank that switched it on.
#[test]
fn x4_the_bubble_eats_the_observers_attenuation_budget() {
    let catalog = ecm_catalog();
    let belt = [(8_u16, 4_u16, woods())];
    for (watcher, held, checks, what) in [
        (content::LINE_TANK, true, 0, "a stock sight"),
        // Two, and they are the cost claim: with a pod on the field the scan is one
        // index query at its radius, and inside 6,000 cm of the observer there are
        // exactly two units — the observer itself and the target four cells out.
        (
            content::LINE_TANK_ECM,
            false,
            2,
            "a sight inside its own bubble",
        ),
    ] {
        let mut arena = jam_field(&catalog, watcher, WATCH_CELL, NEAR_CELL, None, &belt);
        let counters = jam_pass(&mut arena, RED, 1, 0);
        assert_eq!(counters.candidates, 1, "{what}: no candidate at all");
        assert_eq!(
            counters.traces, 1,
            "{what}: no trace was walked, so the range test culled the candidate and this is \
             not a test about the budget after all"
        );
        assert_eq!(
            counters.detected,
            u32::from(held),
            "{what} answered the wrong way through one belt of light woods"
        );
        assert_eq!(
            counters.jam_checks, checks,
            "{what}: the scan looked at the wrong number of units"
        );
    }
}

/// **A pod radiates while it is alive and powered, and not otherwise** (16b cut
/// X.4).
///
/// Doc 16 §3.3 revokes a wreck's capabilities and W3 makes emission a deliberate
/// act, so the scan reads both at scan time: the same field that loses its target
/// with the pod online holds it with the pod shut down, and holds it with the pod
/// burnt out. Both are read where the pod is *read*, not baked in at check-out,
/// which is what lets a pod be silenced inside a running match.
#[test]
fn x4_a_shut_down_or_wrecked_pod_jams_nothing() {
    let catalog = ecm_catalog();
    let field = || {
        jam_field(
            &catalog,
            content::LINE_TANK,
            WATCH_CELL,
            MARK_CELL,
            Some(POD_CELL),
            &[],
        )
    };
    let mut online = field();
    assert!(!holds(&mut online), "the online pod did not jam at all");

    let mut cold = field();
    if let Some(state) = cold.unit_mut(POD) {
        state.power = Power::Off;
    }
    assert!(
        holds(&mut cold),
        "a pod that has not started up still jammed"
    );

    let mut wreck = field();
    if let Some(state) = wreck.unit_mut(POD) {
        state.limits.wreck = true;
    }
    assert!(holds(&mut wreck), "a burnt-out pod still jammed");

    // And the scan still ran in both cases, which is what says the pod was refused
    // for the reason under test rather than never looked at.
    assert!(
        jam_pass(&mut cold, RED, 2, 1_000).jam_checks > 0,
        "the scan did not run at all"
    );
}

/// **A stock fixture's pass is counter-identical when the extension loads** (16b §8
/// cut X: *the story test and the harness unchanged before and after each*), which
/// is the cost claim as well as the behaviour one.
///
/// Two stock tanks, the same window, the same seed, four passes each way: the same
/// passes, candidates, traces, cell probes, detections and classifications, and the
/// same contact held at the end — including `jam_checks`, which is zero, because a
/// match with no pod on the field has `jam_radius_cm == 0` and the scan returns
/// before it touches the index.
#[test]
fn x4_a_stock_fixtures_pass_is_counter_identical_when_the_extension_loads() {
    let run_pair = |catalog: &Catalog| {
        let mut arena = jam_field(
            catalog,
            content::LINE_TANK,
            WATCH_CELL,
            MARK_CELL,
            None,
            &[],
        );
        let mut total = tacsim::arena::perception::PerceptionCounters::default();
        for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
            for observer in [RED, BLUE] {
                total.absorb(&jam_pass(&mut arena, observer, seq, t_ms));
            }
        }
        let id = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
        let held = arena.contacts.resolve(RED, id).copied();
        (total, held)
    };
    let stock = run_pair(&catalog_of(&[]));
    let with_ecm = run_pair(&ecm_catalog());
    assert_eq!(
        stock.0, with_ecm.0,
        "the extension moved a stock fixture's perception cost"
    );
    assert_eq!(
        stock.1.map(|held| held.class),
        with_ecm.1.map(|held| held.class),
        "the extension moved what a stock fixture sees"
    );
    assert!(stock.0.traces > 0, "the fixture traced at all");
    assert_eq!(stock.0.jam_checks, 0, "a field with no pod scanned for one");
}

/// **Both of the pod's own refusals, by name** (16b cut X.4), the way cut S.1's
/// signature rows and cut X.3's thermal band are refused.
///
/// The bounds are re-run on the row where every other sensor bound is, and the
/// cross-reference — a mount that radiates a row with no `jamming` block — is
/// checked where cut X.1 checks that a laser's tank declares a heat ceiling: at
/// bind time, because the two are authored in different files and neither can check
/// the other alone.
#[test]
fn x4_a_pod_past_its_bounds_and_a_pod_radiating_a_sight_are_both_refused() {
    let pack: serde_json::Value =
        serde_json::from_str(content::ECM_MK1_JSON).expect("the pack is json");
    let def = pack
        .pointer("/entries/tacsim.sensor.ecm_mk1/def")
        .expect("the pod's entry")
        .clone();
    let row: tacsim::definitions::SensorDef =
        serde_json::from_value(def.clone()).expect("the authored row decodes");
    row.check("authored").expect("the authored row is valid");

    for (field, value) in [
        ("radius_cells", serde_json::json!(0)),
        ("signature_permille", serde_json::json!(1_001)),
        ("attenuation_cost", serde_json::json!(101)),
    ] {
        let mut spoiled = def.clone();
        *spoiled
            .pointer_mut(&format!("/jamming/{field}"))
            .expect("the authored block carries the field") = value;
        let row: tacsim::definitions::SensorDef =
            serde_json::from_value(spoiled).expect("the spoiled row still decodes");
        let refused = row.check("spoiled").expect_err("the bound is re-run");
        assert!(refused.to_string().contains(field), "{refused}");
    }

    // And a pod that radiates a sight: the ECM tank with its pod pointed at the Mk1
    // visual row, which declares no `jamming` and would have jammed nothing for a
    // whole match.
    let bad = content::LINE_TANK_ECM_JSON.replace(
        "\"sensor\": \"tacsim.sensor.ecm_mk1\"",
        "\"sensor\": \"tacsim.sensor.visual_mk1\"",
    );
    let operator = vec![LoadedPack {
        pack: tacsim::pack::load_pack(&sign(&bad))
            .expect("an unsigned pack is not what is under test"),
        source: "<spoiled>".to_owned(),
    }];
    let refused = Catalog::build_game(&operator, tacsim::NS)
        .expect_err("a pod radiating a sight is refused at bind");
    let text = refused.to_string();
    assert!(
        text.contains("ecm_pod") && text.contains("jamming"),
        "{text}"
    );
}

/// One edited pack's bytes, re-signed, so a refusal under test is the **rule** and
/// never the signature the edit also broke (`tests/definitions.rs` calls this
/// `tampered`).
fn sign(raw: &str) -> String {
    let declared = mush_content::parse_pack(raw)
        .expect("the pack parses")
        .manifest()
        .digest()
        .to_hex();
    let unsigned = raw.replace(&declared, UNSIGNED);
    let signed = mush_content::parse_pack(&unsigned)
        .expect("the unsigned pack parses")
        .computed_digest()
        .expect("the pack digests")
        .to_hex();
    unsigned.replace(UNSIGNED, &signed)
}

// ---------------------------------------------------------------------------
// X.5 — ECCM
// ---------------------------------------------------------------------------

/// A target cell the **recovered** reach does not quite reach: thirteen cells from
/// [`WATCH_CELL`], 130 m, against the 121.5 m the authored filter buys back — and
/// well inside the 540 m an unjammed Mk1 sight has.
///
/// It is the cell that makes *up to the recovery* a fact about a fixture rather
/// than a sentence: one target range the filter recovers ([`MARK_CELL`], 100 m) and
/// one it does not.
const FAR_CELL: (u16, u16) = (8, 15);

/// **What the filter authors** (`content/sensors/eccm_mk1.json`), so the arithmetic
/// below is read off the content rather than restated by it: one Mk1 pod's whole
/// attenuation cost.
const ECCM_ATTENUATION_RECOVERY: u8 = 25;
/// What it adds back to every target's signature factor, per mille — deliberately
/// a part of what the pod took and not the whole of it.
const ECCM_SIGNATURE_RECOVERY_PERMILLE: u32 = 75;

/// What a filter rated against the Mk1 pod **exactly** would author: the whole of
/// the signature floor the pod leaves, so the fold comes back to 1,000.
const FULL_SIGNATURE_RECOVERY_PERMILLE: u32 = 1_000 - JAM_SIGNATURE_PERMILLE;

/// The Mk1 sight's reach against a tank stopped on open ground **inside one Mk1
/// bubble, with the authored filter fitted**: `54,000 × (150 + 75) / 1000`.
const FILTERED_REACH_CM: u64 = 12_150;

/// And with no filter: `54,000 × 150 / 1000`, which is X.4's own number.
const JAMMED_REACH_CM: u64 = 8_100;

/// The shipped content plus the filter and its tank (16b cut X.5).
fn eccm_catalog() -> Catalog {
    catalog_of(&content::extension_sources_of(&content::EXTENSIONS_ECCM))
}

/// **The shipped content plus X.4's pod and this cut's filter** (16b cut X.5).
///
/// What the acceptance needs, because the item's own sentence is about *the X.4
/// observer*: the fixture fields X.4's jammer and this cut's filter, each from its
/// own item's table, through the one operator route X.1 established. The count
/// assertions above are still made against `EXTENSIONS_ECCM` alone, so no earlier
/// item's statement of what its content adds is touched.
fn pod_and_filter_catalog() -> Catalog {
    let mut sources = content::extension_sources_of(&content::EXTENSIONS_ECM);
    sources.extend(content::extension_sources_of(&content::EXTENSIONS_ECCM));
    catalog_of(&sources)
}

/// The filter the content authors, as the arena carries it.
fn authored_filter() -> tacsim::arena::perception::Eccm {
    tacsim::arena::perception::Eccm {
        attenuation_recovery: ECCM_ATTENUATION_RECOVERY,
        signature_recovery_permille: ECCM_SIGNATURE_RECOVERY_PERMILLE,
    }
}

/// One Mk1 pod's jamming, as the fold hands it to the two terms.
fn mk1_jamming() -> tacsim::arena::perception::Jamming {
    tacsim::arena::perception::Jamming {
        attenuation_cost: u32::from(JAM_ATTENUATION_COST),
        signature_permille: JAM_SIGNATURE_PERMILLE,
    }
}

/// **The extension changes nothing about the stock game** — doc 16 §9's own rule,
/// asserted for the filter as X.1 asserts it for the laser.
///
/// The last assertions are this cut's own, and they are about a *code* change rather
/// than a content one: `SensorDef::emission` stopped being an opaque `Option<Value>`
/// and became a typed hook, and what keeps every shipped sight's payload — and
/// therefore its `DefinitionRef`, and therefore every persisted unit that points at
/// one — exactly where it was is that a `None` still serialises as
/// `"emission": null`. The row's whole key set is pinned again here, for the same
/// reason X.4 pinned it: a field added to the row moves every sight's digest.
#[test]
fn x5_the_eccm_content_loads_beside_the_stock_game_without_moving_it() {
    if writing() {
        for relative in PACK_FILES_ECCM {
            resign(relative);
        }
        return;
    }

    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");
    assert_eq!(stock.sensors.len(), 1, "one sight");

    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_ECCM.into_iter().collect();
    for (name, json) in content::EXTENSIONS_ECCM {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_ECCM.len());

    // With them loaded: one more sensor row, one more unit schema, and not one thing
    // else — no weapon, no ammunition, no arena, and no second pod.
    let both = eccm_catalog();
    assert_eq!(both.sensors.len(), 2, "the Mk1 sight and the filter");
    assert_eq!(both.specs.len(), 2, "the line tank and its ECCM variant");
    assert_eq!(both.units.len(), 2, "and the tactics catalog carries both");
    assert_eq!(
        both.weapons.len(),
        2,
        "the ECCM tank carries the shipped guns and authors none of its own"
    );
    assert_eq!(both.arenas.len(), 1, "the extension adds no arena");
    for (id, reference) in &stock.references {
        assert_eq!(
            both.references.get(id),
            Some(reference),
            "{id} moved when the extension loaded; a persisted unit points at it"
        );
    }

    // The tank's identity, where X.2 pinned it, across a change to the *sensor*
    // row's type — which the tank's own payload carries a reference to.
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved; a persisted unit needs a migration"
    );

    // And the shipped sight's row, key for key: `emission` is still the null hook doc
    // 16 §3.6 requires of a row that is neither a pod nor a filter, and it still
    // serialises as `null` now that it has a type.
    let sight = stock
        .sensors
        .get(&CanonicalId::new("tacsim.sensor.visual_mk1").expect("a canonical id"))
        .expect("the shipped sight");
    assert!(sight.emission.is_none() && sight.jamming.is_none());
    assert!(sight.eccm().is_none(), "the shipped sight filters nothing");
    let json = serde_json::to_value(sight).expect("the row serialises");
    let row = json.as_object().expect("a row is an object");
    assert!(
        row.get("emission").is_some_and(serde_json::Value::is_null),
        "a sight with no filter block no longer serialises as null"
    );
    assert!(row.get("jamming").is_some_and(serde_json::Value::is_null));
    let keys: Vec<&str> = row.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "classify_after_ms",
            "emission",
            "jamming",
            "label",
            "max_attenuation",
            "max_range_cells",
            "note",
            "sees_wrecks",
            "share_range_cells",
            "signature",
        ],
        "the sensor row's shape moved; every shipped sight's digest moved with it"
    );
    // The signature table X.3 pinned is untouched by this cut as well.
    assert_eq!(
        serde_json::to_string(&sight.signature).expect("the table serialises"),
        VISUAL_MK1_SIGNATURE
    );
}

/// **The filter is a sensor row whose `emission` is non-null** (doc 16 §3.6's other
/// hook), reaching the arena as cells on the component that carries it.
///
/// And fitting one does not change what the tank *sees* with: `sensor_profile` reads
/// the first `sensor` mount, the filter is an `eccm` mount, and the ECCM tank looks
/// out of the same Mk1 sight the stock tank does. Nor does it make the tank a
/// jammer: a field of filters and no pod never scans for one.
#[test]
fn x5_the_filter_is_a_sensor_row_whose_emission_block_is_non_null() {
    let catalog = eccm_catalog();
    let row = catalog
        .sensors
        .get(&CanonicalId::new(content::ECCM_MK1).expect("a canonical id"))
        .expect("the filter's row");
    let authored = tacsim::definitions::sensor::EccmDef {
        attenuation_recovery: ECCM_ATTENUATION_RECOVERY,
        signature_recovery_permille: ECCM_SIGNATURE_RECOVERY_PERMILLE,
    };
    assert_eq!(
        row.emission,
        Some(tacsim::definitions::sensor::EmissionHook::Eccm(authored)),
        "doc 16 §9 item 5: the `emission` field becomes non-null"
    );
    assert_eq!(row.eccm(), Some(&authored));
    assert!(row.jamming.is_none(), "the filter jams nobody");

    // The cells, decoded at check-out onto the arena's own unit.
    let arena = jam_field(
        &catalog,
        content::LINE_TANK_ECCM,
        WATCH_CELL,
        MARK_CELL,
        None,
        &[],
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.eccm),
        Some(authored_filter()),
        "the ECCM tank carries the filter it authors"
    );
    assert_eq!(
        arena.unit(BLUE).and_then(|state| state.eccm),
        None,
        "a stock tank carries no filter"
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.jammer),
        None,
        "the filter is not a pod"
    );
    assert_eq!(
        arena.jam_radius_cm, 0,
        "a field with a filter and no pod still never scans for one"
    );

    // The filter is not a sight: the ECCM tank looks out of the Mk1 visual row, and
    // its own row's floor numbers reach nothing.
    let looking = arena
        .unit(RED)
        .and_then(|state| state.sensor.as_ref())
        .expect("the ECCM tank sees with something");
    assert_eq!(
        (looking.max_range_cells, looking.max_attenuation),
        (60, 40),
        "fitting a filter changed what the tank sees with"
    );

    // The two terms, as arithmetic, on their own — X.4's fold, recovered.
    let jam = mk1_jamming();
    assert_eq!(
        jam.recovered(None),
        jam,
        "a fold with no filter on it is the fold X.4 left"
    );
    assert_eq!(
        jam.recovered(Some(authored_filter())),
        tacsim::arena::perception::Jamming {
            attenuation_cost: 0,
            signature_permille: JAM_SIGNATURE_PERMILLE + ECCM_SIGNATURE_RECOVERY_PERMILLE,
        }
    );
    let filtered = jam.recovered(Some(authored_filter()));
    assert_eq!(filtered.budget(40), 40, "the pod's whole 25 bought back");
    assert_eq!(
        filtered.scale(EYE_REACH_CM),
        FILTERED_REACH_CM,
        "81 m of reach becomes 121 m"
    );

    // And a filter can never see *better* than no bubble at all: the cost floors at
    // zero and the fraction ceilings at 1,000, so an absurdly over-rated filter is
    // the unjammed pass and nothing more.
    let over = jam.recovered(Some(tacsim::arena::perception::Eccm {
        attenuation_recovery: 100,
        signature_recovery_permille: 1_000,
    }));
    assert_eq!(over.budget(40), 40);
    assert_eq!(
        over.scale(EYE_REACH_CM),
        EYE_REACH_CM,
        "a filter lengthened a sight past its own reach"
    );
}

/// **The acceptance** (doc 16 §9 item 5): X.4's observer, standing in X.4's bubble,
/// **holds the target again** once it carries a filter — out to the recovery and no
/// further.
///
/// The same fixture X.4's acceptance loses its target in: ten cells of open ground,
/// 100 m, with the Mk1 pod parked halfway between. The Mk1 sight reaches 54,000 cm
/// against a tank stopped on `ground`, the pod leaves 8,100 of it, and the filter
/// gives back 75 per mille of the floor — 12,150 cm — so the target at 10,000 cm is
/// back on the scope and the same target at 13,000 cm is not. Which is *up to the
/// recovery*: ECCM buys back some of the range ECM took, never all of it.
///
/// Then the last row of the table, and the other half of the item's own sentence: a
/// filter rated against this pod **exactly** — the whole 850 the pod leaves — reaches
/// 13,000 cm as well, because `scale` of 1,000 per mille is the identity. A fully
/// rated filter is the unjammed pass.
#[test]
fn x5_the_filter_holds_the_jammed_target_again_up_to_the_recovery() {
    let catalog = pod_and_filter_catalog();
    let jammed = |watcher: &str, target: (u16, u16)| {
        jam_field(&catalog, watcher, WATCH_CELL, target, Some(POD_CELL), &[])
    };

    // The reaches the table below is an assertion about, from the content's own
    // numbers rather than from prose.
    assert_eq!(mk1_jamming().scale(EYE_REACH_CM), JAMMED_REACH_CM);
    assert!(
        JAMMED_REACH_CM < 10_000
            && (10_000..13_000).contains(&FILTERED_REACH_CM)
            && 13_000 < EYE_REACH_CM,
        "the fixture's three ranges no longer tell the three reaches apart"
    );

    // X.4's own row: jammed, the target 100 m out is gone.
    assert!(
        !holds(&mut jammed(content::LINE_TANK, MARK_CELL)),
        "the observer inside the bubble still held a target 100 m out"
    );
    // This cut's row: the same bubble, the same target, a filter on the observer.
    assert!(
        holds(&mut jammed(content::LINE_TANK_ECCM, MARK_CELL)),
        "the filter bought nothing back: the jammed target is still gone"
    );
    // Up to the recovery and no further: 130 m is past the 121.5 m it buys.
    assert!(
        !holds(&mut jammed(content::LINE_TANK_ECCM, FAR_CELL)),
        "the filter recovered more than it authors"
    );
    // And 130 m is well inside an unjammed sight, so the fixture is about the bubble.
    assert!(
        holds(&mut jam_field(
            &catalog,
            content::LINE_TANK,
            WATCH_CELL,
            FAR_CELL,
            None,
            &[]
        )),
        "a target 130 m out was lost with no pod on the field at all"
    );

    // Full recovery is the unjammed reach: the same field, the same pod, a filter
    // rated against it exactly. Set on the checked-out unit rather than authored as a
    // second content row, because doc 16 §9 item 5 asks for one filter and the point
    // of the authored one is that it is *partial*.
    let mut full = jammed(content::LINE_TANK_ECCM, FAR_CELL);
    if let Some(state) = full.unit_mut(RED) {
        state.eccm = Some(tacsim::arena::perception::Eccm {
            attenuation_recovery: JAM_ATTENUATION_COST,
            signature_recovery_permille: FULL_SIGNATURE_RECOVERY_PERMILLE,
        });
    }
    assert_eq!(
        mk1_jamming()
            .recovered(full.unit(RED).and_then(|state| state.eccm))
            .scale(EYE_REACH_CM),
        EYE_REACH_CM,
        "a filter rated against the pod exactly is not the unjammed reach"
    );
    assert!(
        holds(&mut full),
        "a fully rated filter lost a target an unjammed sight holds"
    );
}

/// **The budget half, on its own** (doc 16 §9 item 5 against §9 item 4's *adds to
/// the observer's `max_attenuation` budget*), with the pod on the other team's hull.
///
/// A belt of light woods on the line and a target four cells out, which is well
/// inside what even the unfiltered jamming leaves of the reach — so step 3 passes,
/// the trace is walked, and the only thing that can refuse the detection is step 2's
/// budget: the pod takes the Mk1 sight's 40 down to 15, which the belt's 25 defeats,
/// and the filter buys the whole 25 back.
///
/// The counters say the scan ran the same number of times either way: what the filter
/// changes is the arithmetic on the fold, never the walk that collects it.
#[test]
fn x5_the_filter_buys_back_the_observers_attenuation_budget() {
    let catalog = pod_and_filter_catalog();
    let belt = [(8_u16, 4_u16, woods())];
    for (watcher, held, what) in [
        (content::LINE_TANK, 0, "a stock sight in the bubble"),
        (content::LINE_TANK_ECCM, 2, "a filtered sight in the bubble"),
    ] {
        let mut arena = jam_field(
            &catalog,
            watcher,
            WATCH_CELL,
            NEAR_CELL,
            Some(POD_CELL),
            &belt,
        );
        let counters = jam_pass(&mut arena, RED, 1, 0);
        assert_eq!(
            counters.candidates, 2,
            "{what}: the target and the pod's tank are the candidates"
        );
        assert_eq!(
            counters.traces, 2,
            "{what}: a trace was not walked, so the range test culled a candidate and this is \
             not a test about the budget after all"
        );
        assert_eq!(
            counters.detected, held,
            "{what} answered the wrong way through one belt of light woods"
        );
        assert_eq!(
            counters.jam_checks, 3,
            "{what}: the scan looked at the wrong number of units"
        );
    }
}

/// **An observer no bubble reaches is the same observer with the filter and
/// without** (16b cut X.5), which is what makes the term invisible where it has
/// nothing to subtract from.
///
/// Two readings of *nothing else changes*, and both are counter-identity rather than
/// prose: the observer stepped out of the radius while the pod still radiates, and a
/// field with no pod on it at all — where `jam_radius_cm` is zero and the scan never
/// touches the index.
#[test]
fn x5_an_observer_outside_every_bubble_is_identical_with_the_filter_and_without() {
    let catalog = pod_and_filter_catalog();
    let run = |watcher: &str, pod: Option<(u16, u16)>| {
        let mut arena = jam_field(&catalog, watcher, OUTSIDE_CELL, MARK_CELL, pod, &[]);
        let mut total = tacsim::arena::perception::PerceptionCounters::default();
        for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000)] {
            total.absorb(&jam_pass(&mut arena, RED, seq, t_ms));
        }
        let id = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
        (
            total,
            arena.contacts.resolve(RED, id).map(|held| held.class),
        )
    };

    // Outside the radius, with the pod live: the pod's bubble reaches neither the
    // observer nor its arithmetic, so the filter has nothing to give back.
    let stock = run(content::LINE_TANK, Some(POD_CELL));
    let filtered = run(content::LINE_TANK_ECCM, Some(POD_CELL));
    assert_eq!(stock, filtered, "the filter moved a pass no bubble reaches");
    assert_eq!(
        stock.0.jam_checks, 3,
        "one unit on three passes: the scan looked at the observer alone, because the pod is 70 m \
         away and its bubble is 60 m wide"
    );
    assert!(stock.1.is_some(), "the fixture held its target at all");

    // And with no pod anywhere: the scan never runs, for either observer.
    let bare_stock = run(content::LINE_TANK, None);
    let bare_filtered = run(content::LINE_TANK_ECCM, None);
    assert_eq!(
        bare_stock, bare_filtered,
        "the filter moved a pass on a field with no jammer on it"
    );
    assert_eq!(
        bare_stock.0.jam_checks, 0,
        "a field with no pod scanned for one"
    );
}

/// **A filter recovers while it is still there, and does not need power** (16b cut
/// X.5).
///
/// The two halves of the rule, and they answer differently on purpose:
///
/// - **shot off its mount** — the `condition` cell of the filter's own component at
///   zero, which is the one thing that stops a filter working. The tank is then the
///   stock tank of X.4's acceptance and loses the same target.
/// - **shut down** — a pod is an emitter and a hull that has not started up radiates
///   nothing (X.4 asserts that), but a filter emits nothing at all: it discriminates
///   in the sight's own returns, and this module's rule is that optics are optics. A
///   cold tank with an intact sight and an intact filter holds what a warm one does.
#[test]
fn x5_a_filter_shot_off_its_mount_recovers_nothing_and_a_cold_one_recovers_everything() {
    let catalog = pod_and_filter_catalog();

    // The filter's own `condition` cell, taken to zero before check-out — the way
    // `arena::resolution`'s own test dims a sight.
    let mut units = vec![
        unit_of(
            &catalog,
            content::LINE_TANK_ECCM,
            RED,
            Team::Red,
            WATCH_CELL,
        ),
        unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, MARK_CELL),
        unit_of(&catalog, content::LINE_TANK_ECM, POD, Team::Blue, POD_CELL),
    ];
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new("eccm_pod").expect("a component id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::CONDITION_CELL)
            .expect("a cell id"),
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 0 }],
        None,
        None,
    )
    .expect("a one-op plan");
    let shot = units.first_mut().expect("the observer");
    let (post, _) = mush_tactics::apply(&shot.unit, &plan, 0).expect("the filter is shot off");
    shot.unit = post;

    let mut gone = arena_of(&catalog, units, &[]);
    for id in [RED, POD] {
        if let Some(state) = gone.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    assert_eq!(
        gone.unit(RED).and_then(|state| state.eccm),
        None,
        "a filter whose component is gone is still a filter"
    );
    assert!(
        !holds(&mut gone),
        "a tank whose filter is gone saw through the bubble anyway"
    );

    // Power: the filter is not an emitter, so a cold hull still filters.
    let mut cold = jam_field(
        &catalog,
        content::LINE_TANK_ECCM,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    if let Some(state) = cold.unit_mut(RED) {
        state.power = Power::Off;
    }
    assert_eq!(
        cold.unit(RED).and_then(|state| state.eccm),
        Some(authored_filter()),
        "the cold tank's filter was read away with its power"
    );
    assert!(
        holds(&mut cold),
        "a filter stopped working when its hull shut down, and a filter emits nothing"
    );
}

/// **A stock fixture's pass is counter-identical when the extension loads** (16b §8
/// cut X: *the story test and the harness unchanged before and after each*), which is
/// the cost claim as well as the behaviour one.
///
/// Two stock tanks, the same window, the same seed, four passes each way: the same
/// passes, candidates, traces, cell probes, detections and classifications, the same
/// `jam_checks` of zero, and the same contact held at the end.
#[test]
fn x5_a_stock_fixtures_pass_is_counter_identical_when_the_extension_loads() {
    let run_pair = |catalog: &Catalog| {
        let mut arena = jam_field(
            catalog,
            content::LINE_TANK,
            WATCH_CELL,
            MARK_CELL,
            None,
            &[],
        );
        let mut total = tacsim::arena::perception::PerceptionCounters::default();
        for (seq, t_ms) in [(1_u64, 0_i64), (2, 1_000), (3, 2_000), (4, 3_000)] {
            for observer in [RED, BLUE] {
                total.absorb(&jam_pass(&mut arena, observer, seq, t_ms));
            }
        }
        let id = tacsim::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
        let held = arena.contacts.resolve(RED, id).copied();
        (total, held)
    };
    let stock = run_pair(&catalog_of(&[]));
    let with_eccm = run_pair(&eccm_catalog());
    assert_eq!(
        stock.0, with_eccm.0,
        "the extension moved a stock fixture's perception cost"
    );
    assert_eq!(
        stock.1.map(|held| held.class),
        with_eccm.1.map(|held| held.class),
        "the extension moved what a stock fixture sees"
    );
    assert!(stock.0.traces > 0, "the fixture traced at all");
    assert_eq!(stock.0.jam_checks, 0, "a field with no pod scanned for one");
}

/// **Every one of the filter's refusals, by name** (16b cut X.5), the way cut S.1's
/// signature rows, cut X.3's thermal band and cut X.4's pod are refused.
///
/// The bounds are re-run on the row where every other sensor bound is, and the
/// cross-reference — a mount fitted to a row with no filter block — is checked where
/// cut X.4 checks a pod radiating a sight: at bind time, because the two are authored
/// in different files and neither can check the other alone.
///
/// The last two cases are the ones that shape the hook's *type*. Doc 16 §3.6's hook
/// has to refuse a value of the wrong shape **naming `emission`** —
/// `tests/definitions.rs` pins exactly that, and it is a test this cut may not edit —
/// so the refusal has to come from `SensorDef::check` and not from `serde`, whose own
/// message names the offending inner key and no path to it. That is why `emission` is
/// an untagged pair rather than a bare `Option<EccmDef>`: the decode stays total and
/// the refusal stays where it was. Both halves of that are asserted here.
#[test]
fn x5_a_filter_past_its_bounds_and_a_filter_fitted_to_a_sight_are_both_refused() {
    let pack: serde_json::Value =
        serde_json::from_str(content::ECCM_MK1_JSON).expect("the pack is json");
    let def = pack
        .pointer("/entries/tacsim.sensor.eccm_mk1/def")
        .expect("the filter's entry")
        .clone();
    let row: tacsim::definitions::SensorDef =
        serde_json::from_value(def.clone()).expect("the authored row decodes");
    row.check("authored").expect("the authored row is valid");

    for (pointer, value, needle) in [
        (
            "/emission/attenuation_recovery",
            serde_json::json!(101),
            "attenuation_recovery",
        ),
        (
            "/emission/signature_recovery_permille",
            serde_json::json!(1_001),
            "signature_recovery_permille",
        ),
        // A block that recovers nothing is refused the way a `jamming` block with a
        // zero radius is: an inert component is a component that does nothing for a
        // whole match, silently.
        ("/emission", serde_json::json!({}), "recovers nothing"),
        // And the shape rule, which is the pinned one.
        (
            "/emission",
            serde_json::json!({ "band": "radar" }),
            "emission",
        ),
    ] {
        let mut spoiled = def.clone();
        *spoiled
            .pointer_mut(pointer)
            .expect("the authored row carries the pointer") = value;
        let row: tacsim::definitions::SensorDef =
            serde_json::from_value(spoiled).expect("the spoiled row still decodes");
        let refused = row.check("spoiled").expect_err("the bound is re-run");
        assert!(refused.to_string().contains(needle), "{refused}");
    }

    // The other half of that last case: `serde`'s own refusal for the same value names
    // the key inside the hook and never the hook, which is the whole reason the hook is
    // typed the way it is.
    let bare = serde_json::from_value::<tacsim::definitions::sensor::EccmDef>(
        serde_json::json!({ "band": "radar" }),
    )
    .expect_err("a foreign shape is not a filter");
    assert!(
        bare.to_string().contains("band") && !bare.to_string().contains("emission"),
        "{bare}"
    );

    // And a filter fitted to a sight: the ECCM tank with its filter pointed at the Mk1
    // visual row, which declares no `emission` and would have recovered nothing for a
    // whole match.
    let bad = content::LINE_TANK_ECCM_JSON.replace(
        "\"sensor\": \"tacsim.sensor.eccm_mk1\"",
        "\"sensor\": \"tacsim.sensor.visual_mk1\"",
    );
    let operator = vec![LoadedPack {
        pack: tacsim::pack::load_pack(&sign(&bad))
            .expect("an unsigned pack is not what is under test"),
        source: "<spoiled>".to_owned(),
    }];
    let refused = Catalog::build_game(&operator, tacsim::NS)
        .expect_err("a filter fitted to a sight is refused at bind");
    let text = refused.to_string();
    assert!(
        text.contains("eccm_pod") && text.contains("emission"),
        "{text}"
    );
}
