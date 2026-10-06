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
//! **16b cut Q.6** paid two debts this file owed. Four of the five items above
//! shipped without the cross-tempo test 16b §11 makes mandatory for any cut that
//! changes arena behaviour, and X.4 left the second half of doc 16 §2 W6 — *and
//! tactical comms are LOS mesh jammed by the same ECM* — unbuilt. Both are at the
//! end of this file, prefixed `q6_`, on fixtures of their own: the replays need a
//! corridor rather than the 16 by 16 window the X items wanted, and the mesh
//! property needs a **blind** teammate, whose only possible way of holding
//! anything is the link under test.
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
use tacsim::arena::perception::FrameNotice;
use tacsim::arena::resolution::FULL_ATTENUATION;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, CustodyOut, CustodyUnit, EntryZone,
    Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell, UnitState, Waypoint, Waypoints,
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
///
/// **17b cut M3.1** moved `tacsim.pack.line_tank_laser` (version 3): a core
/// component, the line core in the turret. Was `558a1cef…` at version 2.
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
        "e4f008666f33da22ab4c3a66fec6fb3c9e0bf4da72438dcee3f1de30ac54affa",
    ),
];

/// The extension pack files, by their path under `content/`.
const PACK_FILES: [&str; 3] = [
    "ammunition/laser_charge.json",
    "weapons/laser_mk1.json",
    "units/line_tank_laser.json",
];

/// **The biped's digest pin** (16b cut X.2), the way X.1 pins the laser's.
///
/// **17b cut M3.1** moved it (version 2): a core component, the line core in
/// the torso. Was `6d2272a0…` at version 1.
const DIGESTS_BIPED: [(&str, &str); 1] = [(
    "tacsim.pack.biped_mk1",
    "cf38ed133b16e510880215d365b8743f3e16bf19c0462b3de2a089dc273df837",
)];

/// The biped's pack file, by its path under `content/`.
const PACK_FILES_BIPED: [&str; 1] = ["units/biped_mk1.json"];

/// **The thermal sight's digest pins** (16b cut X.3), the way X.1 pins the
/// laser's: the sight, then the tank that mounts it.
///
/// **17b cut M3.1** moved the tank's (version 3): a core component, the line
/// core in the turret. Was `cecaa090…` at version 2.
const DIGESTS_THERMAL: [(&str, &str); 2] = [
    (
        "tacsim.pack.thermal_mk1",
        "921be78771d4ce4c07a6289bc448d1b39c622dc68dcadbe266aa4f405629c48b",
    ),
    (
        "tacsim.pack.line_tank_thermal",
        "b9f0ce77fb0f82228341b17846ab16b0eba8d2234fa19355d8b2706f2940bdc3",
    ),
];

/// The thermal sight's pack files, by their paths under `content/`.
const PACK_FILES_THERMAL: [&str; 2] = ["sensors/thermal_mk1.json", "units/line_tank_thermal.json"];

/// **The jamming pod's digest pins** (16b cut X.4), the way X.1 pins the laser's:
/// the pod's row, then the tank that carries it.
///
/// **17b cut M3.1** moved the tank's (version 3): a core component, the line
/// core in the turret. Was `227d8de5…` at version 2.
const DIGESTS_ECM: [(&str, &str); 2] = [
    (
        "tacsim.pack.ecm_mk1",
        "0ce4a2787d83f226b3bad46c9d09b3d960d55c78f00c79c888ac9f08d416ba22",
    ),
    (
        "tacsim.pack.line_tank_ecm",
        "77b9fcd26cb97c97498fd13f62533ee98247cf76d8eb96d7b826d62049efee61",
    ),
];

/// The pod's pack files, by their paths under `content/`.
const PACK_FILES_ECM: [&str; 2] = ["sensors/ecm_mk1.json", "units/line_tank_ecm.json"];

/// **The filter's digest pins** (16b cut X.5), the way X.1 pins the laser's: the
/// filter's row, then the tank that carries it.
///
/// **17b cut M3.1** moved the tank's (version 3): a core component, the line
/// core in the turret (and its note's M.4 sentence shortened to fit the
/// 1,024-byte bound). Was `ba7bc418…` at version 2.
const DIGESTS_ECCM: [(&str, &str); 2] = [
    (
        "tacsim.pack.eccm_mk1",
        "4bf6842ee80f746b62d669680b0e32b941ca04f8b0bae2c6a940ede26f6d3de1",
    ),
    (
        "tacsim.pack.line_tank_eccm",
        "b4be914cec9033367a800c250cd222675f2e661fa3aae412193bd85391564215",
    ),
];

/// The filter's pack files, by their paths under `content/`.
const PACK_FILES_ECCM: [&str; 2] = ["sensors/eccm_mk1.json", "units/line_tank_eccm.json"];

/// **The range's target's digest pin** (16b cut O.3; 16c N13), the way X.1 pins
/// the laser's: one unit schema, requiring nothing.
const DIGESTS_TARGET: [(&str, &str); 1] = [(
    "tacsim.pack.target_mk1",
    "f376460f54bef4c862ff30d152f515917fb135356d9e7523149f0cfadf012068",
)];

/// The target's pack file, by its path under `content/`.
const PACK_FILES_TARGET: [&str; 1] = ["units/target_mk1.json"];

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
/// pins above only ever catch a content one. The value was this crate's answer at
/// `e000f06a`, the commit before the cut, taken there and pinned here.
///
/// **16b cut M.4 moved it, with the content** (16c `B-47`): the tank's two tracks
/// gained `disable_motion_when_all_of` in `tacsim.pack.line_tank` version 2, so
/// the definition is a new one by content, not by code. A persisted `line_tank`
/// pointing at the old digest is refitted at boot (`match_/reset.rs`'s
/// `refit_unit` of a roster tank whose schema is not the loaded binding's). Was
/// `72a52d55e73e3d1d4308c8d69ebd3b0a91504a5935571f87ae319c69cd131660`.
///
/// **17b cut M3.1 moved it again, with the content** (doc 17 §9, D-UM-33): the
/// line tank fits the line core in its turret from `tacsim.pack.line_tank`
/// version 3 — a `core` component, its `core_class` cell and its `tacsim.core`
/// capability — so the definition is a new one by content again; a persisted
/// `line_tank` is refitted at the next server-half restart by the same
/// `reset_stock_tanks` rule (`tests/match_lifecycle.rs`,
/// `m3_a_roster_tank_without_a_core_is_refitted_with_one_at_the_reset`). Was
/// `d908bc1f8d1a5b950ab2802db0e0ddc96c38360611d8f7df25cd35a4d853bb63`. (Within
/// the cut, the line core's minted definition is referenced by the tank's
/// `core` component, so the cores pack's note wording — reworded to placement
/// alone before it was ever committed — is part of this digest.)
const LINE_TANK_DIGEST: &str = "b8fa366613da1a39345a0a74eacc662aba00f954213972bc4ff135aaa9ef74dc";

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
        modes: Vec::new(),
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
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: cells,
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
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
            mode: tacsim::arena::MatchMode::Skirmish,
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
    arena
        .resolve_lock(RED, contact, arena.now_ms)
        .expect("the lock holds");
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
        .resolve_fire(RED, "laser", arena.now_ms)
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
    direct
        .resolve_lock(RED, contact, direct.now_ms)
        .expect("the lock holds");
    let seq = direct
        .resolve_fire(RED, "cannon", direct.now_ms)
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
    arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("the first shot");
    assert_eq!(heat(&arena), Some(20), "one pulse, one heat_per_shot");
    ready(&mut arena);
    arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("the second shot");
    assert_eq!(heat(&arena), Some(40));
    ready(&mut arena);
    arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("the third shot");
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
        arena.resolve_fire(RED, "laser", arena.now_ms),
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
        .resolve_fire(RED, "laser", arena.now_ms)
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
        arena
            .resolve_fire(RED, "laser", arena.now_ms)
            .expect("one pulse of heat");
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
    let seq = arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("the shot");
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
    arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("one pulse of heat");
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
        arena
            .resolve_fire(RED, "laser", arena.now_ms)
            .expect("a pulse");
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
        arena.apply_consequence(unit, consequence, 0, None, &mut journal);
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
        let (section, _) = arena
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
    // 16b cut U.2: and the jamming memo with it, exactly as phase 3(b) empties
    // it beside its own rebuild — a pass driven directly reads through the memo.
    arena.jam_fold.clear();
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
    assert_eq!(counters.horizon_probes, 0);
    assert_eq!(counters.detected, 0);

    let mut hot = watch(&catalog, content::LINE_TANK_THERMAL, MARK_CELL);
    target_state(&mut hot, Power::Online, 1_000);
    let counters = thermal_pass(&mut hot, RED, 1, 0);
    assert_eq!(counters.candidates, 1);
    assert_eq!(counters.traces, 1, "the same candidate, traced once");
    // 16b cut J.2: a line the horizon index proves clear reads no cell, so the
    // look along it is a block read or a cell read, never neither.
    assert!(
        counters.cell_probes + counters.horizon_probes > 0,
        "the line was looked along, by block or by cell"
    );
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
    shot.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");

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

// ---------------------------------------------------------------------------
// X.6 — elevation
// ---------------------------------------------------------------------------

/// **A ground cell at `elev_cm`** (16b cut X.6): [`ground`] with a height.
fn at_elev(elev_cm: i16) -> TerrainCell {
    TerrainCell {
        elev_cm,
        ..ground()
    }
}

/// **A relief window** (16b cut X.6): `width` by `height` cells of [`CELL_CM`],
/// every one of them [`ground`]'s class and cost, at the elevation `profile`
/// gives it.
///
/// A window of its own rather than [`definition`]'s sixteen cells a side,
/// because this item needs room: the line tank does 15 m/s, so a lane has to be
/// long enough for it to reach a sustained speed on a slope and still have
/// ground to be refused on. Forty-eight by eight is that lane.
fn relief(width: u16, height: u16, profile: &dyn Fn(u16, u16) -> i16) -> ArenaDefinition {
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            cells.push(at_elev(profile(x, y)));
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.relief"),
        terrain: ArenaTerrain::new(width, height, CELL_CM, cells).expect("the relief window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: height - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: width - 2,
                y0: 0,
                x1: width - 1,
                y1: height - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::default(),
        modes: Vec::new(),
    }
}

/// A patrol graph of exactly one node (16b cut X.6): a unit whose only node it
/// can never reach keeps that node as its goal — the graph has no other — so it
/// drives at the ground between, which is the failure mode test 9 measures.
fn relief_waypoint(at: (u16, u16)) -> Waypoints {
    Waypoints::new(
        vec![Waypoint {
            id: SmolStr::new("far"),
            x: at.0,
            y: at.1,
        }],
        &[],
    )
}

/// A two-node patrol graph joining `a` and `b` — the smallest graph on which
/// `Patrol` produces a route (16b cut X.6).
fn relief_waypoints(a: (u16, u16), b: (u16, u16)) -> Waypoints {
    Waypoints::new(
        vec![
            Waypoint {
                id: SmolStr::new("start"),
                x: a.0,
                y: a.1,
            },
            Waypoint {
                id: SmolStr::new("far"),
                x: b.0,
                y: b.1,
            },
        ],
        &[(SmolStr::new("start"), SmolStr::new("far"))],
    )
}

/// An arena over an explicit `definition` and `cfg` (16b cut X.6), with every
/// unit seated — [`arena_of`] for a window this file's [`definition`] does not
/// make.
fn arena_over(
    catalog: &Catalog,
    units: Vec<CustodyUnit>,
    definition: ArenaDefinition,
    cfg: &ArenaConfig,
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
            definition,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        cfg,
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the relief arena")
}

/// The same unit under the reflex controller (16b cut X.6).
fn under_reflex(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        controller: tacsim::arena::state::Controller::Reflex,
        ..unit
    }
    .sealed()
}

/// The same unit with no sight at all (16b cut X.6) — what a fixture about
/// motion wants, so that no contact, no share and no tempo signal is mixed into
/// the event list it compares.
fn blind(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        sensor: None,
        ..unit
    }
    .sealed()
}

/// The same unit with `tests/reflex.rs`'s own fixture helm (16b cut X.6): 10 m/s
/// and 180 degrees a second.
///
/// The shipped tank turns at 30 degrees a second and does 15 m/s, which makes it
/// a sloppy follower — it overshoots a corner and spends six seconds coming back
/// round — and every reflex fixture in this tree gives its tanks a nimbler helm
/// for exactly that reason. The mobility **profile** is untouched, so the unit's
/// slope limit is still the shipped `tracked` row's, which is what the fixture is
/// about.
fn nimble(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        mobility: tacsim::arena::state::TrackedRules {
            max_speed_cms: 1_000,
            turn_rate_mdeg_s: 180_000,
            ..unit.mobility
        },
        ..unit
    }
    .sealed()
}

/// Wake `id` and point it: online, at `heading_mdeg`, already at `speed_cms` —
/// the shape `arena::motion`'s own tests pre-set a unit in (16b cut X.6).
fn under_way(arena: &mut Arena, id: mush_core::ObjId, heading_mdeg: u32, speed_cms: i32) {
    if let Some(state) = arena.unit_mut(id) {
        state.power = Power::Online;
        state.pose.heading_mdeg = heading_mdeg;
        state.desired_heading_mdeg = heading_mdeg;
        state.speed_cms = speed_cms;
        state.desired_speed_cms = speed_cms;
    }
}

/// Run `pulses` pulses of `period` ms, absorbing the ledger (16b cut X.6).
///
/// [`run`] drops the step's output, and these tests need the ledger lines that
/// came out of it.
fn run_logged(
    arena: &mut Arena,
    wall: &mut i64,
    period: i64,
    pulses: usize,
    bytes: &mut Vec<u8>,
) -> tacsim::arena::reflex::ReflexCounters {
    let mut counters = tacsim::arena::reflex::ReflexCounters::default();
    if *wall == 0 {
        let out = arena.step(0, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
    }
    for _ in 0..pulses {
        *wall += period;
        let out = arena.step(*wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
    }
    counters
}

/// Every notice line the pilot in `seat` was told over `pulses` pulses of
/// `period` ms (16b cut X.6).
///
/// [`run_logged`] for the cockpit's own words rather than the ledger's — the
/// shape `tests/arena.rs`'s `heard` takes, which is where cut F.2's three stop
/// sentences are pinned.
fn run_heard(
    arena: &mut Arena,
    wall: &mut i64,
    period: i64,
    pulses: usize,
    seat: mush_core::ObjId,
) -> Vec<String> {
    let mut told: Vec<String> = Vec::new();
    let mut hear = |frames: &[(mush_core::ObjId, tacsim::arena::step::Frame)]| {
        told.extend(
            frames
                .iter()
                .filter(|(who, _)| *who == seat)
                .flat_map(|(_, frame)| cockpit::notice_lines(frame)),
        );
    };
    if *wall == 0 {
        hear(&arena.step(0, &[], &[]).frames);
    }
    for _ in 0..pulses {
        *wall += period;
        hear(&arena.step(*wall, &[], &[]).frames);
    }
    told
}

/// Every ledger line so far, flushed (16b cut X.6).
fn ledger_of(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<tacsim::arena::ledger::LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(bytes.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).ok())
        .collect()
}

/// Every `blocked` line naming `unit`, as `(x, y)` (16b cut X.6).
fn blocked_cells(
    events: &[tacsim::arena::ledger::LedgerEvent],
    unit: mush_core::ObjId,
) -> Vec<(u16, u16)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::Blocked { unit: at, x, y, .. } if *at == unit => {
                Some((*x, *y))
            }
            _ => None,
        })
        .collect()
}

/// Run `pulses` pulses of `period` ms and answer the **steepest edge `id` came
/// up** on the way (16b cut X.6).
///
/// `grade_pct` is the slope of the *last* edge crossed, so a tank that has
/// climbed a ledge and driven on across the flat reads zero again; what a
/// fixture about a ledge wants to know is what it paid to get up it.
fn run_watching_grade(
    arena: &mut Arena,
    wall: &mut i64,
    period: i64,
    pulses: usize,
    id: mush_core::ObjId,
    bytes: &mut Vec<u8>,
) -> u16 {
    let mut steepest = 0_u16;
    for _ in 0..pulses {
        run_logged(arena, wall, period, 1, bytes);
        steepest = steepest.max(arena.unit(id).map_or(0, |unit| unit.grade_pct));
    }
    steepest
}

/// A unit's cell, its z and the slope it came up (16b cut X.6).
fn standing(arena: &Arena, id: mush_core::ObjId) -> ((u16, u16), i32, u16) {
    let unit = arena.unit(id).expect("the unit");
    let cell = arena
        .definition
        .terrain
        .cell_of(&unit.pose)
        .expect("on the window");
    (cell, unit.pose.z_cm, unit.grade_pct)
}

/// The `tracked` row's own slope limit — the number every line tank is held to.
fn tracked_limit() -> u32 {
    mush_atlas::physics::LocomotionTable::shipped()
        .max_slope_pct("tracked", &[])
        .expect("the shipped table prices tracked")
}

/// **The M32 unit a profile-less check-out carries** (16b cut X.6): one hull
/// section and `mobility: None`, which is every engine fixture in this tree.
fn bare_tactical_unit(id: mush_core::ObjId) -> mush_tactics::TacticalUnit {
    let reference = mush_tactics::DefinitionRef::new(
        mush_core::DefinitionKey::new(
            mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
            mush_core::DefinitionKind::new("unit").expect("a canonical kind"),
            mush_core::DefinitionId::new("tacsim.unit.bare").expect("a canonical id"),
        ),
        1,
        mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero");
    let spec = mush_tactics::UnitSpec {
        definition: reference,
        sections: vec![mush_tactics::SectionSpec {
            id: mush_tactics::SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: Vec::new(),
            slots: Vec::new(),
            cells: Vec::new(),
        }],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    mush_tactics::TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

// ---------------------------------------------------------------------------
// X.6 (1) the authored heightfield
// ---------------------------------------------------------------------------

/// The production roster, composed exactly as `mush_server::boot` composes it —
/// `tests/definitions.rs`'s own helper, copied here because the heightfield this
/// item imports comes out of the atlas records that roster's content module
/// lays down (16b cut X.6).
fn x6_production_roster() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
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
        std::sync::Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.extend(comp::sheet_modules(&onboarding, &game));
    mods.push(comp::tacsim_module(cfg));
    mods.push(comp::content_module_with(false, true).expect("the content module composes"));
    mods
}

/// **The authored heightfield reaches the arena verbatim, and the one ground a
/// shipped unit can be refused at is The Bluff** (16b cut X.6; doc 16 §9 item
/// 6; rewritten on purpose by 16b cut AE.1, 16c `T-18`).
///
/// The first half is the import: `from_pack` copies `elev_cm` straight off the
/// tile, so the levels the proving ground's generator writes are the levels the
/// window holds, at the cells that hold them — the relief's nine, a hundred
/// centimetres apart, and since 16b cut AE.1 The Bluff's three: its top at 500
/// and its ramp at 270 and 40.
///
/// The second half is why A1 does not move, and what 16b cut AE.1 changed about
/// it. X.6 pinned that the steepest edge between two cells a tank may stand on
/// was 20 per cent orthogonal and 21 diagonal against the `tracked` row's 45, so
/// the slope refusal, the search's shoulder rule and the follower's cliff test
/// ran only in fixtures (16c `T-18`). Off The Bluff that is still so: measured
/// with the bluff's box (`x 140..=148, y 104..=121`) left out, the steepest is
/// 20 and 21. And every edge past 45 on the whole map is one of The Bluff's 107
/// — its rims, its corner and row 119's two flanks — so nothing A1 drives is
/// touched: row 97 and every muster slot's straight run meet no edge past 45.
/// (The 30 per cent 16c's `T-18` row names is the grass-to-deep-water bank at
/// (61, 161), impassable to every row, which this never measured.)
#[test]
fn x6_the_authored_heightfield_reaches_the_arena_verbatim() {
    let tw = mush_testkit::TestWorld::with_modules(x6_production_roster());
    let records = tw
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| state.records.clone())
        .expect("the atlas is composed");
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, def) = stock
        .arenas
        .iter()
        .next()
        .expect("the proving ground is shipped");
    let arena = tacsim::definitions::from_pack(id.as_str(), def, &records)
        .expect("the arena's ground is in this world's records");
    let terrain = &arena.terrain;
    assert_eq!((terrain.width(), terrain.height()), (256, 256));

    // The relief's nine levels and The Bluff's three (16b cut AE.1: the ramp's
    // 40 and 270 and the top's 500), and the three cells the design note names.
    let levels: std::collections::BTreeSet<i16> =
        terrain.cells().iter().map(|cell| cell.elev_cm).collect();
    assert_eq!(
        levels.iter().copied().collect::<Vec<i16>>(),
        vec![-400, -300, -200, -100, 0, 40, 100, 200, 270, 300, 400, 500],
        "the authored relief is nine levels a hundred centimetres apart, and The Bluff's three"
    );
    let elev = |x: u16, y: u16| terrain.get(x, y).map(|cell| cell.elev_cm);
    assert_eq!(
        elev(208, 148),
        Some(-400),
        "the floor of the south-east bowl"
    );
    assert_eq!(elev(56, 97), Some(300), "the crest west of the ford");
    assert_eq!(
        elev(64, 97),
        Some(0),
        "the ford itself, cut two levels down"
    );

    // No edge between two cells a tank may stand on is steeper than the `tracked`
    // row's own limit — in either direction, which is what X.6 refuses on.
    let limit = tracked_limit();
    assert_eq!(limit, 45);
    let cell_cm = u32::from(terrain.cell_cm());
    // 16b cut AE.1: The Bluff's box — the mesa, its rim neighbours, the ramp
    // and its foot.
    let on_bluff = |(x, y): (u16, u16)| (140..=148).contains(&x) && (104..=121).contains(&y);
    let (mut orthogonal, mut diagonal) = (0_u32, 0_u32);
    let mut worst = ((0_u16, 0_u16), (0_u16, 0_u16));
    let mut past_the_limit = 0_usize;
    for y in 0..terrain.height() {
        for x in 0..terrain.width() {
            let Some(here) = terrain.get(x, y).filter(|cell| cell.passable()) else {
                continue;
            };
            for (dx, dy) in [(1_i32, 0_i32), (0, 1), (1, 1), (1, -1)] {
                let (Some(nx), Some(ny)) = (
                    u16::try_from(i32::from(x) + dx).ok(),
                    u16::try_from(i32::from(y) + dy).ok(),
                ) else {
                    continue;
                };
                let Some(there) = terrain.get(nx, ny).filter(|cell| cell.passable()) else {
                    continue;
                };
                let Some(kind) = tacsim::arena::slope::step_kind((x, y), (nx, ny)) else {
                    continue;
                };
                let steepness = tacsim::arena::slope::steepness_pct(here, there, kind, cell_cm);
                // Every edge past the limit, on the whole map, is The Bluff's.
                if steepness > limit {
                    assert!(
                        on_bluff((x, y)) && on_bluff((nx, ny)),
                        "the shipped map has a {steepness} per cent edge at ({x}, {y}) -> \
                         ({nx}, {ny}), off The Bluff"
                    );
                    past_the_limit += 1;
                }
                if on_bluff((x, y)) || on_bluff((nx, ny)) {
                    continue;
                }
                let steepest = if kind == mush_atlas::physics::StepKind::Diagonal {
                    &mut diagonal
                } else {
                    &mut orthogonal
                };
                if steepness > *steepest {
                    *steepest = steepness;
                    worst = ((x, y), (nx, ny));
                }
            }
        }
    }
    assert!(
        orthogonal.max(diagonal) <= limit,
        "off The Bluff the shipped map has an edge at {worst:?} a line tank would be \
         refused at"
    );
    assert_eq!(
        (orthogonal, diagonal),
        (20, 21),
        "the steepest passable edge off The Bluff moved"
    );
    assert_eq!(
        past_the_limit, 107,
        "every edge past 45 is one of The Bluff's: its 104 rims, its corner and row 119's \
         two flanks"
    );

    // And the ground A1 drives never meets an edge past 45: the ford road, one
    // level a cell, and every muster slot's straight run at its opposite
    // number.
    let run = |row: u16, from: u16, to: u16| {
        for x in from..to {
            let (Some(here), Some(next)) = (terrain.get(x, row), terrain.get(x + 1, row)) else {
                continue;
            };
            let steepness = tacsim::arena::slope::steepness_pct(
                here,
                next,
                mush_atlas::physics::StepKind::Orthogonal,
                cell_cm,
            );
            assert!(
                steepness <= 20,
                "row {row} rises {steepness} per cent at {x}"
            );
        }
    };
    run(97, 0, terrain.width().saturating_sub(1));
    for red in def.tanks.iter().filter(|tank| tank.team == Team::Red) {
        let slot = red.id.as_str().trim_start_matches("red_");
        let blue = def
            .tanks
            .iter()
            .find(|tank| tank.id.as_str() == format!("blue_{slot}"))
            .unwrap_or_else(|| panic!("blue_{slot} is authored"));
        assert_eq!(red.y, blue.y, "slot {slot} faces along one row");
        run(red.y, red.x, blue.x);
    }
    // The muster columns of cut F.4's ford road, which is where A1's tanks stand.
    assert_eq!(elev(6, 97), Some(100));
    assert_eq!(elev(249, 97), Some(-100));

    // X.6 authors no content, so the shipped tank's identity has not moved.
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
}

// ---------------------------------------------------------------------------
// X.6 (2) z at check-out
// ---------------------------------------------------------------------------

/// **A unit opens the match at its cell's elevation** (16b cut X.6; doc 16 §3.2:
/// *`z_cm` equals the cell's `elev_cm`*).
///
/// `CustodyOut` is the only door into an arena and the arena is the authority for
/// its own grid, so an inbound z is normalised at check-out rather than trusted
/// — the world's placements are written flat, and before this cut a tank stood
/// three metres under the hill it was parked on until it happened to cross a cell
/// boundary. Every range, every plot and the opening state hash read the
/// corrected number.
#[test]
fn x6_a_unit_opens_the_match_at_its_cells_elevation() {
    let catalog = catalog();
    let high: (u16, u16) = (4, 4);
    let low: (u16, u16) = (12, 4);
    let profile = move |x: u16, _y: u16| match x {
        4 => 300,
        12 => -200,
        _ => 0,
    };
    let mut red = unit_of(&catalog, content::LINE_TANK, RED, Team::Red, high);
    let mut blue = unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, low);
    // What the world hands over: a flat z on one, and a stale one on the other.
    assert_eq!(red.pose.z_cm, 0, "the check-out pose is written flat");
    blue.pose.z_cm = 9_999;
    red.pose.z_cm = 0;
    let mut arena = arena_over(
        &catalog,
        vec![red, blue],
        relief(48, 8, &profile),
        &ArenaConfig::default(),
    );

    assert_eq!(standing(&arena, RED), (high, 300, 0));
    assert_eq!(
        standing(&arena, BLUE),
        (low, -200, 0),
        "a stale inbound z is corrected and not carried"
    );

    // The five hundred centimetres between them are in the range, from the first
    // instant of the match.
    let red_pose = arena.unit(RED).map(|unit| unit.pose.clone()).expect("red");
    let blue_pose = arena
        .unit(BLUE)
        .map(|unit| unit.pose.clone())
        .expect("blue");
    let solid = tacsim::arena::index::range_cm(&red_pose, &blue_pose);
    let flat = tacsim::arena::index::range_between(4_500, 4_500, 0, 12_500, 4_500, 0);
    assert_eq!(flat, 8_000);
    assert_eq!(
        solid,
        tacsim::arena::index::range_between(4_500, 4_500, 300, 12_500, 4_500, -200)
    );
    assert_eq!(
        solid, 8_015,
        "the five hundred centimetres are in the range"
    );

    // And the ledger's own summary says so, at the first cadence.
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 12, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    let summary = events
        .iter()
        .find_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::PositionSummary { units } => Some(units.clone()),
            _ => None,
        })
        .expect("a position summary");
    assert_eq!(
        summary
            .iter()
            .map(|(unit, _, _, z, _)| (*unit, *z))
            .collect::<Vec<(mush_core::ObjId, i32)>>(),
        vec![(RED, 300), (BLUE, -200)]
    );
}

// ---------------------------------------------------------------------------
// X.6 (3) the surcharge
// ---------------------------------------------------------------------------

/// **A climb is slower than the flat, and a descent is not faster** (16b cut
/// X.6; doc 16 §3.4 step 2).
///
/// Three lanes of the same ground: one rising 300 cm a cell, one level, one
/// falling 300 cm a cell. The climber's ceiling is `max_speed × 100 /
/// effective_cost(move_cost, 30)` and it sits there; the descender's is the flat
/// one, because the surcharge is uphill-only — a tank does not go faster
/// downhill, it simply is not slowed. And at every sample the z is the cell's,
/// which is the other half of doc 16 §3.2's rule.
#[test]
fn x6_a_climb_is_slower_than_the_flat_and_a_descent_is_not_faster() {
    const CLIMBER: mush_core::ObjId = mush_core::ObjId(4);
    const LEVEL: mush_core::ObjId = mush_core::ObjId(9);
    const FALLER: mush_core::ObjId = mush_core::ObjId(11);

    let catalog = catalog();
    let profile = |x: u16, y: u16| -> i16 {
        let step = i32::from(x) * 300;
        match y {
            1 => i16::try_from(step).unwrap_or(i16::MAX),
            6 => i16::try_from(-step).unwrap_or(i16::MIN),
            _ => 0,
        }
    };
    let mut arena = arena_over(
        &catalog,
        vec![
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                CLIMBER,
                Team::Red,
                (1, 1),
            )),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                LEVEL,
                Team::Red,
                (1, 4),
            )),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                FALLER,
                Team::Red,
                (1, 6),
            )),
        ],
        relief(48, 8, &profile),
        &ArenaConfig::default(),
    );
    for id in [CLIMBER, LEVEL, FALLER] {
        under_way(&mut arena, id, 90_000, 1_500);
    }

    // The ceiling on the ramp, off the one rule: 1,500 × 100 / 130.
    let ramp_cost = tacsim::arena::slope::effective_cost(100, 30);
    assert_eq!(ramp_cost, 130);
    let ramp_speed = i32::try_from(1_500 * 100 / ramp_cost).unwrap_or(0);
    assert_eq!(ramp_speed, 1_153);

    let mut wall = 0_i64;
    for _ in 0..40 {
        run(&mut arena, &mut wall, 500, 1);
        // Doc 16 §3.2, at every sample: a unit's z is the cell's elevation.
        for id in [CLIMBER, LEVEL, FALLER] {
            let (cell, z, _) = standing(&arena, id);
            assert_eq!(
                Some(z),
                arena
                    .definition
                    .terrain
                    .get(cell.0, cell.1)
                    .map(|cell| i32::from(cell.elev_cm)),
                "unit {id} is not standing on its own cell"
            );
        }
    }

    let (climber_cell, climber_z, climber_grade) = standing(&arena, CLIMBER);
    let (level_cell, level_z, level_grade) = standing(&arena, LEVEL);
    let (faller_cell, faller_z, faller_grade) = standing(&arena, FALLER);
    assert_eq!(climber_grade, 30, "the climber came up a 30 per cent edge");
    assert_eq!(level_grade, 0);
    assert_eq!(faller_grade, 0, "a descent is not a climb");
    assert_eq!(climber_z, i32::from(climber_cell.0) * 300);
    assert_eq!(level_z, 0);
    assert_eq!(faller_z, -i32::from(faller_cell.0) * 300);

    // The climber is held at the surcharged ceiling; the other two are at the
    // flat one, and the descender has covered exactly the level tank's ground.
    assert_eq!(
        arena.unit(CLIMBER).map(|unit| unit.speed_cms),
        Some(ramp_speed)
    );
    assert_eq!(arena.unit(LEVEL).map(|unit| unit.speed_cms), Some(1_500));
    assert_eq!(arena.unit(FALLER).map(|unit| unit.speed_cms), Some(1_500));
    let x_of = |id: mush_core::ObjId| arena.unit(id).map(|unit| unit.pose.x_cm);
    assert_eq!(
        x_of(FALLER),
        x_of(LEVEL),
        "the descender covered exactly the flat tank's distance"
    );
    assert!(
        x_of(CLIMBER) < x_of(LEVEL),
        "the climber fell behind: {:?} against {:?}",
        x_of(CLIMBER),
        x_of(LEVEL)
    );
    assert!(climber_cell.0 < level_cell.0);
}

// ---------------------------------------------------------------------------
// X.6 (4) the limit
// ---------------------------------------------------------------------------

/// **A slope past the limit is refused in both directions** (16b cut X.6; doc 16
/// §3.4 step 4).
///
/// The one deliberate divergence from `mush_atlas::physics::move_cost`, which
/// refuses only the climb. A 500 cm step at 10 m cells is 50 per cent and the
/// `tracked` row stops at 45: the tank is refused climbing it **and** refused
/// driving off the top of it, because an uphill-only limit would make the top of
/// that ledge a cell nothing could ever leave. A 450 cm step is exactly the
/// limit and is taken, up and down, the down side at the flat ceiling.
#[test]
fn x6_a_slope_past_the_limit_blocks_both_ways() {
    let catalog = catalog();
    let ledge = |height: i16| move |x: u16, _y: u16| if x >= 24 { height } else { 0 };

    // (a) climbing a 50 per cent ledge.
    let mut arena = arena_over(
        &catalog,
        vec![blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (20, 4),
        ))],
        relief(48, 8, &ledge(500)),
        &ArenaConfig::default(),
    );
    under_way(&mut arena, RED, 90_000, 1_500);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
    assert_eq!(
        standing(&arena, RED),
        ((23, 4), 0, 0),
        "the climber is held in the cell below the ledge, at its own elevation"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.pose.x_cm),
        Some(23_999),
        "it stopped at the last centimetre of its own cell"
    );
    assert_eq!(arena.unit(RED).map(|unit| unit.speed_cms), Some(0));
    assert_eq!(arena.unit(RED).map(|unit| unit.blocked), Some(true));
    assert_eq!(
        blocked_cells(&ledger_of(&mut arena, &mut bytes), RED),
        vec![(24, 4)],
        "one notice per stop, naming the cell it could not enter"
    );

    // (b) driving off the top of the same ledge — refused the same way.
    let mut arena = arena_over(
        &catalog,
        vec![blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (28, 4),
        ))],
        relief(48, 8, &ledge(500)),
        &ArenaConfig::default(),
    );
    under_way(&mut arena, RED, 270_000, 1_500);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
    assert_eq!(
        standing(&arena, RED),
        ((24, 4), 500, 0),
        "the descent is refused too: the ledge is not a one-way door"
    );
    assert_eq!(arena.unit(RED).map(|unit| unit.pose.x_cm), Some(24_000));
    assert_eq!(arena.unit(RED).map(|unit| unit.speed_cms), Some(0));
    assert_eq!(
        blocked_cells(&ledger_of(&mut arena, &mut bytes), RED),
        vec![(23, 4)]
    );

    // (c) a 450 cm step is exactly the limit: climbed, and descended at the flat
    // ceiling, because the surcharge is uphill-only.
    let mut arena = arena_over(
        &catalog,
        vec![
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                RED,
                Team::Red,
                (20, 2),
            )),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (28, 5),
            )),
        ],
        relief(48, 8, &ledge(450)),
        &ArenaConfig::default(),
    );
    under_way(&mut arena, RED, 90_000, 1_500);
    under_way(&mut arena, BLUE, 270_000, 1_500);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let climbed = run_watching_grade(&mut arena, &mut wall, 500, 20, RED, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_cells(&events, RED).is_empty(),
        "45 per cent is not past a limit of 45"
    );
    assert!(blocked_cells(&events, BLUE).is_empty());
    assert_eq!(standing(&arena, RED).1, 450, "the climber is on the ledge");
    assert_eq!(climbed, 45, "and it paid the forty-five per cent to get up");
    assert_eq!(standing(&arena, BLUE).1, 0, "the descender is off it");
    assert_eq!(standing(&arena, BLUE).2, 0);
    assert_eq!(arena.unit(BLUE).map(|unit| unit.speed_cms), Some(1_500));

    // (d) and the property the (a)/(b) pair is an instance of, swept over a table
    // of ledges either side of the limit and driven through **motion** rather
    // than through the arithmetic: what a limited tank may enter, it may leave.
    //
    // Through motion deliberately. Asking `steepness_pct` the same question twice
    // would be asking whether `max(a, b) == max(b, a)`, which holds for every
    // implementation of anything; what is worth a sweep is the refusal itself,
    // which is the one place the choice between the edge's steepness and the
    // uphill grade is made. A refusal that read the grade would let a tank down a
    // 500 cm ledge it cannot climb, and the 500 and 700 cm rows below say so.
    let column_after = |from: (u16, u16), heading_mdeg: u32, height: i16| -> u16 {
        let mut arena = arena_over(
            &catalog,
            vec![blind(unit_of(
                &catalog,
                content::LINE_TANK,
                RED,
                Team::Red,
                from,
            ))],
            relief(48, 8, &ledge(height)),
            &ArenaConfig::default(),
        );
        under_way(&mut arena, RED, heading_mdeg, 1_500);
        let mut wall = 0_i64;
        let mut bytes: Vec<u8> = Vec::new();
        run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
        standing(&arena, RED).0.0
    };
    let mut crossed = 0_usize;
    let mut refused = 0_usize;
    for height in [100_i16, 300, 450, 460, 500, 700, 1_200] {
        let entered = column_after((20, 4), 90_000, height) >= 24;
        let left = column_after((28, 4), 270_000, height) <= 23;
        assert_eq!(
            entered, left,
            "a {height} cm ledge is a one-way door: entered {entered}, left {left}"
        );
        if entered {
            crossed += 1;
        } else {
            refused += 1;
        }
    }
    // And the sweep is not vacuous in either direction: three of those ledges are
    // inside the tracked row's 45 per cent and four are past it.
    assert_eq!((crossed, refused), (3, 4));
}

/// **A slope refusal has its own word and its own sentence** (16b cut X.6; doc 16
/// §3.4 step 4, §3.6).
///
/// The fourth `BlockedCause` is the cut's only new player-facing surface, so it
/// is pinned the way cut F.2 pinned the other three: drive a tank at a ledge it
/// cannot climb and read what the cockpit told its pilot. The distinction is
/// worth the sentence — the ground ahead *would* take the tank and the slope onto
/// it will not, so what the pilot does about it is drive round rather than give
/// the cell up — and a refusal that fell back to
/// [`tacsim::arena::motion::BlockedCause::Impassable`] would tell him the
/// opposite. The line names terrain and nothing else, so the observer rule is
/// untouched.
#[test]
fn x6_a_slope_refusal_is_its_own_word_and_its_own_sentence() {
    use tacsim::arena::motion::BlockedCause;

    // The wire word every renderer keys on, and the four the arena ships.
    assert_eq!(BlockedCause::TooSteep.as_str(), "too_steep");
    assert_eq!(
        [
            BlockedCause::Impassable,
            BlockedCause::Occupied,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ]
        .map(|cause| cause.as_str()),
        ["impassable", "occupied", "edge", "too_steep"]
    );

    // Test 4(a)'s own fixture: a 50 per cent ledge, and a tank driving at it.
    let catalog = catalog();
    let mut arena = arena_over(
        &catalog,
        vec![blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (20, 4),
        ))],
        relief(48, 8, &|x: u16, _y: u16| if x >= 24 { 500 } else { 0 }),
        &ArenaConfig::default(),
    );
    under_way(&mut arena, RED, 90_000, 1_500);
    let mut wall = 0_i64;
    let told = run_heard(&mut arena, &mut wall, 500, 20, mush_core::ObjId(1_000));
    let stops: Vec<&String> = told
        .iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        stops,
        vec![&"Held: the slope ahead is too steep for the tank.".to_owned()],
        "one line per stop, and it is the slope's own: {told:?}"
    );
    assert_eq!(standing(&arena, RED).0, (23, 4));

    // And the ground's own refusal still reads as the ground's: the same window
    // level, with the cell at (24, 4) made impassable instead of steep.
    let mut cells: Vec<TerrainCell> = Vec::new();
    for y in 0..8_u16 {
        for x in 0..48_u16 {
            cells.push(if (x, y) == (24, 4) {
                TerrainCell {
                    move_cost: 0,
                    ..ground()
                }
            } else {
                ground()
            });
        }
    }
    let mut impassable = relief(48, 8, &|_x: u16, _y: u16| 0);
    impassable.terrain =
        ArenaTerrain::new(48, 8, CELL_CM, cells).expect("the level window with one hole in it");
    let mut arena = arena_over(
        &catalog,
        vec![blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (20, 4),
        ))],
        impassable,
        &ArenaConfig::default(),
    );
    under_way(&mut arena, RED, 90_000, 1_500);
    let mut wall = 0_i64;
    let told = run_heard(&mut arena, &mut wall, 500, 20, mush_core::ObjId(1_000));
    assert_eq!(
        told.iter()
            .filter(|line| line.starts_with("Held:"))
            .collect::<Vec<&String>>(),
        vec![&"Held: the ground ahead will not take the tank.".to_owned()],
        "{told:?}"
    );
}

// ---------------------------------------------------------------------------
// X.6 (5) whose limit it is
// ---------------------------------------------------------------------------

/// **The limit is the locomotion's own** (16b cut X.6; doc 16 §9 item 6).
///
/// The same 600 cm ledge, the same window, two units: the biped's `legged` row
/// stops at 70 per cent and it walks up; the tank's `tracked` row stops at 45 and
/// it is held. The limit is read off the shipped locomotion table at check-out,
/// through the same mode ladder `tracked_rules` reads the kinematics off, so for
/// every shipped unit the slope limit and the speed come from one mode (only a
/// profile the ladder does not recognise falls back to its first mode for the
/// limit; see `slope::driving_mode`). A unit with no mobility profile at all —
/// every engine fixture in this tree — has no limit and still pays the surcharge.
#[test]
fn x6_the_limit_is_the_locomotions_own() {
    let catalog = biped_catalog();
    let ledge = |x: u16, _y: u16| if x >= 24 { 600_i16 } else { 0 };
    let mut arena = arena_over(
        &catalog,
        vec![
            blind(unit_of(
                &catalog,
                content::BIPED_MK1,
                RED,
                Team::Red,
                (20, 2),
            )),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (20, 5),
            )),
        ],
        relief(48, 8, &ledge),
        &ArenaConfig::default(),
    );
    assert_eq!(
        arena.unit(RED).and_then(|unit| unit.slope_limit_pct),
        Some(70),
        "the biped walks under the legged row"
    );
    assert_eq!(
        arena.unit(BLUE).and_then(|unit| unit.slope_limit_pct),
        Some(45),
        "the tank drives under the tracked row"
    );

    under_way(&mut arena, RED, 90_000, 1_200);
    under_way(&mut arena, BLUE, 90_000, 1_500);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let climbed = run_watching_grade(&mut arena, &mut wall, 500, 24, RED, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_cells(&events, RED).is_empty(),
        "60 per cent is inside the legged limit"
    );
    assert_eq!(standing(&arena, RED).1, 600, "the biped is on the ledge");
    assert_eq!(climbed, 60, "and it paid the sixty per cent to get up");
    assert_eq!(
        blocked_cells(&events, BLUE),
        vec![(24, 5)],
        "60 per cent is past the tracked limit"
    );
    assert_eq!(standing(&arena, BLUE), ((23, 5), 0, 0));

    // And a unit whose content declares no locomotion the table knows: no limit
    // at all, which is not a limit of zero.
    assert_eq!(
        tacsim::arena::slope::SlopeLimit::read(&bare_tactical_unit(mush_core::ObjId(77))),
        None
    );
}

// ---------------------------------------------------------------------------
// X.6 (6) the route goes round
// ---------------------------------------------------------------------------

/// The cliff-and-ramp window tests 6 and 9 share: a plateau 800 cm up behind an
/// unbroken cliff line at column 20, reached only by a 200 cm-a-cell ramp along
/// the last row (16b cut X.6).
///
/// `ramp` false seals it: the plateau is then a cell no limited unit can reach
/// from the low ground, in any direction.
fn plateau(ramp: bool) -> ArenaDefinition {
    relief(48, 8, &move |x: u16, y: u16| {
        if x < 20 {
            return 0;
        }
        if ramp && y == 7 {
            return match x {
                20 => 200,
                21 => 400,
                22 => 600,
                _ => 800,
            };
        }
        800
    })
}

/// **The route goes round what motion would refuse** (16b cut X.6; doc 16 §6).
///
/// The point of putting the slope in the search at all: a route is a promise
/// about edges motion will accept, and a route over a cliff is a tank driving at
/// that cliff and re-planning at it every `repath_ms` until the match ends. With
/// the limit in force the search takes the ramp — every consecutive pair of the
/// route is inside the limit, and so is every shoulder of every diagonal on it —
/// and a reflex tank told to go there arrives having been blocked by nothing.
/// Without a limit the same search goes straight over, which is what the
/// unlimited `path` has always done and still does.
///
/// **What the end-to-end half is conditional on, said here rather than in a
/// comment half way down.** The zero-`Blocked` arrival holds at `decide_ms` 250,
/// `repath_ms` 0 and `tests/reflex.rs`'s own 10 m/s, 180°-a-second helm. At the
/// shipped doctrine and the shipped helm it does not, and the reason is the
/// follower and not the route: a tank doing 15 m/s and turning at 30°/s
/// overshoots a corner and steers at the goal, which on this window is at the
/// cliff. **16b cut Q.7 closed half of that** — a route the unit has wandered
/// off now spends the search it came from, so the re-plan is the next tick and
/// not the next `repath_ms`, and a follower with no route at all tests the one
/// cell the crossing would enter before it moves (`tests/reflex.rs`'s
/// `the_follower_holds_rather_than_drive_at_a_bluff`). The other half is the
/// cadence itself: a 1 Hz decision at 15 m/s commits a cell and a half of
/// travel, and the cell the tank crosses in the middle of one is nobody's
/// choice. Measured on this window at cut Q.7, the stock doctrine still records
/// seven stops. Test 9 measures what that costs, with the numbers printed for
/// the ledger. What *this* test is about is whether the route is one motion
/// accepts, so it asks for a route that is never stale.
#[test]
fn x6_the_route_goes_round_what_motion_refuses() {
    let limit = tracked_limit();
    let start = (2_u16, 4_u16);
    let goal = (40_u16, 2_u16);

    let terrain = plateau(true).terrain;
    let cell_cm = u32::from(terrain.cell_cm());
    let scale = tacsim::arena::min_move_cost(&terrain);
    let mut scratch = tacsim::arena::PathScratch::default();

    let limited = tacsim::arena::path_with(
        &terrain,
        &mut scratch,
        start,
        goal,
        65_536,
        scale,
        Some(limit),
        tacsim::arena::CostRow::Tracked,
        &[],
    )
    .expect("the ramp is a way up");
    assert!(
        limited.cells.iter().any(|(x, y)| *x == 20 && *y == 7),
        "the limited route takes the ramp: {:?}",
        limited.cells
    );

    // Every edge of it is one motion would accept, shoulders and all.
    let mut walked = vec![start];
    walked.extend(limited.cells.iter().copied());
    for pair in walked.windows(2) {
        let (Some(from), Some(to)) = (pair.first().copied(), pair.get(1).copied()) else {
            continue;
        };
        let (Some(here), Some(there), Some(kind)) = (
            terrain.get(from.0, from.1),
            terrain.get(to.0, to.1),
            tacsim::arena::slope::step_kind(from, to),
        ) else {
            panic!("{from:?} -> {to:?} is not a step");
        };
        assert!(
            !tacsim::arena::slope::too_steep(
                Some(limit),
                tacsim::arena::slope::steepness_pct(here, there, kind, cell_cm)
            ),
            "{from:?} -> {to:?} is past the limit"
        );
        if kind == mush_atlas::physics::StepKind::Diagonal {
            for shoulder in [(to.0, from.1), (from.0, to.1)] {
                let Some(cell) = terrain
                    .get(shoulder.0, shoulder.1)
                    .filter(|cell| cell.passable())
                else {
                    continue;
                };
                for (a, b) in [(here, cell), (cell, there)] {
                    assert!(
                        !tacsim::arena::slope::too_steep(
                            Some(limit),
                            tacsim::arena::slope::steepness_pct(
                                a,
                                b,
                                mush_atlas::physics::StepKind::Orthogonal,
                                cell_cm
                            )
                        ),
                        "the shoulder {shoulder:?} of {from:?} -> {to:?} is past the limit"
                    );
                }
            }
        }
    }

    // The unlimited search goes straight over the cliff, as it always has.
    let plain = tacsim::arena::path::path(&terrain, &mut scratch, start, goal, 65_536, scale)
        .expect("a route over the top");
    assert!(
        plain.cells.iter().all(|(_, y)| *y < 7),
        "the unlimited route never goes near the ramp: {:?}",
        plain.cells
    );
    assert_ne!(plain.cells, limited.cells);

    // With the ramp sealed there is no route at all for a limited unit, and the
    // unlimited one still finds its way over.
    let sealed = plateau(false).terrain;
    assert_eq!(
        tacsim::arena::path_with(
            &sealed,
            &mut scratch,
            start,
            goal,
            65_536,
            scale,
            Some(limit),
            tacsim::arena::CostRow::Tracked,
            &[]
        ),
        Err(tacsim::arena::PathFailure::NoRoute)
    );
    assert!(tacsim::arena::path::path(&sealed, &mut scratch, start, goal, 65_536, scale).is_ok());

    // And the controller counts that failure rather than swallowing it: the same
    // sealed window under the **stock** doctrine, driven long enough for one
    // search, answers a failure the ledger can read.
    let catalog = catalog();
    let mut definition = plateau(false);
    definition.waypoints = relief_waypoint(goal);
    let mut arena = arena_over(
        &catalog,
        vec![under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            start,
        )))],
        definition,
        &ArenaConfig::default(),
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let sealed_counters = run_logged(&mut arena, &mut wall, 500, 60, &mut bytes);
    assert!(
        sealed_counters.path_failures >= 1,
        "the sealed plateau was searched {} times and never reported a failure",
        sealed_counters.path_queries
    );

    // And the controller, end to end: a reflex tank whose patrol node is on the
    // plateau gets there over the ramp, blocked by nothing on the way.
    let mut definition = plateau(true);
    definition.waypoints = relief_waypoints(start, goal);
    // The doctrine `tests/reflex.rs` runs its own edges on: think every 250 ms
    // and re-plan whenever the route is gone. A tank doing 15 m/s covers a cell
    // and a half between decisions at the stock cadence, and the cell it crosses
    // in the middle of one is nobody's choice — **16b cut Q.7** gave the
    // follower back the two decisions that were its own (a wandered-off route
    // now spends its search, and a follower with no route tests the cell the
    // crossing would enter) and this window still records stops at the stock
    // cadence. See this test's head. What it is about is whether the *route* is
    // one motion accepts, so the fixture asks for a fresh one.
    let cfg = ArenaConfig {
        reflex: tacsim::config::ReflexConfig {
            decide_ms: 250,
            repath_ms: 0,
            ..tacsim::config::ReflexConfig::default()
        },
        ..ArenaConfig::default()
    };
    let mut arena = arena_over(
        &catalog,
        vec![nimble(under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            start,
        ))))],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run_logged(&mut arena, &mut wall, 500, 260, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_cells(&events, RED).is_empty(),
        "the reflex tank drove at something the route promised: {:?}",
        blocked_cells(&events, RED)
    );
    assert_eq!(counters.path_failures, 0);
    let (cell, z, _) = standing(&arena, RED);
    assert!(
        cell.0 >= 20 && z == 800,
        "the reflex tank never reached the plateau: {cell:?} at {z} cm"
    );
}

// ---------------------------------------------------------------------------
// X.6 (7) the ridge
// ---------------------------------------------------------------------------

/// **A ridge blocks the line both ways, and a taller eye sees over it** (16b cut
/// X.6; doc 16 §3.2 "Line of sight").
///
/// Elevation has been in the line walk since cut S.1 — this is the acceptance
/// that says so on a window X.6 could author, and the reason X.6 changes no line
/// of sight code at all. A 400 cm ridge across the whole window blocks a pair of
/// 150 cm eyes in **both** directions; the sensor pass therefore offers the
/// candidate, traces it and holds nothing, where the same field flat holds a
/// contact; a 100 cm ridge does not block a 150 cm eye at all; a nine-metre eye
/// looks over the 400 cm ridge and the tank looks back up at it; and a shot over
/// it is refused the line, spending its round and its cycle exactly as doc 16
/// §3.5's re-validation says.
#[test]
fn x6_a_ridge_blocks_the_line_both_ways_and_a_taller_eye_sees_over_it() {
    use mush_atlas::trace::{Blocker, Eye, Sight, trace_cells};

    /// Two 150 cm eyes at `a` and `b`: the walk must answer the ridge, and name a
    /// cell on `ridge_y` as the thing that blocked it.
    fn blocked_by_the_ridge(terrain: &ArenaTerrain, a: (u16, u16), b: (u16, u16), ridge_y: u16) {
        let trace = trace_cells(
            terrain,
            Eye {
                cell: a,
                height_cm: 150,
            },
            Eye {
                cell: b,
                height_cm: 150,
            },
        );
        match trace.sight {
            Sight::BlockedBy(Blocker::Elevation { at, elev_cm }) => {
                assert_eq!(at.1, ridge_y, "the blocker is a ridge cell: {at:?}");
                assert_eq!(elev_cm, 400);
            }
            other => panic!("{a:?} -> {b:?} is not blocked by the ridge: {other:?}"),
        }
    }

    const RIDGE_Y: u16 = 7;
    let ridge = |height: i16| move |_x: u16, y: u16| if y == RIDGE_Y { height } else { 0 };

    // (a) the geometry, in both directions, over a sweep of offsets.
    let terrain = relief(16, 16, &ridge(400)).terrain;
    for (dx, dy) in [(0_i32, 0_i32), (2, 1), (-3, 2), (5, -2), (1, 4)] {
        let Some(shooter) = u16::try_from(8 + dx).ok().map(|x| (x, 3_u16)) else {
            continue;
        };
        let Some(target) = u16::try_from(8 - dx + dy).ok().map(|x| (x, 11_u16)) else {
            continue;
        };
        for (a, b) in [(shooter, target), (target, shooter)] {
            blocked_by_the_ridge(&terrain, a, b, RIDGE_Y);
        }
    }

    // (a2) and the **shallow** lines, which the sweep above does not produce: its
    // five pairs all span eight rows, and until 16b cut M.4 (16c `B-56`)
    // `mush_atlas::trace::line_cells` was not symmetric under swapping its
    // endpoints on a line like two across and one down — (0,0) to (2,1) walked
    // (1,1) where the reverse walked (1,0). M.4 draws every line from its lesser
    // end, so the two walks are one cell set; against a ridge every walk must
    // cross they must agree, and these are the shapes that would say otherwise.
    for (from, to) in [
        ((6_u16, 6_u16), (10_u16, 8_u16)),
        ((6, 6), (2, 8)),
        ((2, 6), (14, 8)),
        ((12, 6), (6, 8)),
        ((4, 6), (10, 8)),
    ] {
        for (a, b) in [(from, to), (to, from)] {
            blocked_by_the_ridge(&terrain, a, b, RIDGE_Y);
        }
    }

    // (b) a 100 cm ridge does not block a 150 cm eye.
    let low = relief(16, 16, &ridge(100)).terrain;
    assert!(
        trace_cells(
            &low,
            Eye {
                cell: (8, 3),
                height_cm: 150
            },
            Eye {
                cell: (8, 11),
                height_cm: 150
            },
        )
        .sight
        .is_clear_or_partial(),
        "a ridge under the eye is not a ridge"
    );

    // (c) the sensor pass: the candidate is offered and traced over the ridge,
    // and nothing is held; the same field flat holds a contact.
    let catalog = catalog();
    let sensor_run = |profile: &dyn Fn(u16, u16) -> i16| {
        let mut arena = arena_over(
            &catalog,
            vec![
                unit_of(&catalog, content::LINE_TANK, RED, Team::Red, (8, 3)),
                unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, (8, 11)),
            ],
            relief(16, 16, profile),
            &ArenaConfig::default(),
        );
        for id in [RED, BLUE] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        let mut wall = 0_i64;
        let mut counters = tacsim::arena::perception::PerceptionCounters::default();
        if wall == 0 {
            arena.step(0, &[], &[]);
        }
        for _ in 0..12 {
            wall += 500;
            counters.absorb(&arena.step(wall, &[], &[]).perception);
        }
        let held = arena.contacts.of(RED).count();
        (counters, held)
    };
    let (over, held_over) = sensor_run(&ridge(400));
    let (flat, held_flat) = sensor_run(&|_x, _y| 0);
    assert_eq!(held_over, 0, "the ridge is between them");
    assert!(held_flat > 0, "the same field flat acquires a contact");
    assert!(
        over.candidates > 0 && over.traces > 0,
        "the pass offered the candidate and traced it: {over:?}"
    );
    assert!(flat.candidates > 0 && flat.traces > 0);

    // (d) a nine-metre eye looks over the 400 cm ridge, and the tank looks back.
    let biped = biped_catalog();
    let mut arena = arena_over(
        &biped,
        vec![
            unit_of(&biped, content::BIPED_MK1, RED, Team::Red, (8, 3)),
            unit_of(&biped, content::LINE_TANK, BLUE, Team::Blue, (8, 11)),
        ],
        relief(16, 16, &ridge(400)),
        &ArenaConfig::default(),
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.sensor_height_cm),
        Some(900),
        "the biped's eye is nine metres up"
    );
    for id in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    let mut wall = 0_i64;
    run(&mut arena, &mut wall, 500, 12);
    assert!(
        arena.contacts.of(RED).count() > 0,
        "the biped looks over the ridge"
    );
    assert!(
        arena.contacts.of(BLUE).count() > 0,
        "and the tank looks up at the biped"
    );

    // (e) and a shot over the ridge: the line is refused, the round and the cycle
    // are spent, and the miss names the blocker's own reason.
    let mut arena = arena_over(
        &catalog,
        vec![
            unit_of(&catalog, content::LINE_TANK, RED, Team::Red, (8, 3)),
            unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, (8, 11)),
        ],
        relief(16, 16, &ridge(400)),
        &ArenaConfig::default(),
    );
    for id in [RED, BLUE] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    assert!(
        !arena.fire_line(RED, BLUE).0.is_clear_or_partial(),
        "the ridge is in the way of the round too"
    );
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
    arena
        .resolve_lock(RED, contact, arena.now_ms)
        .expect("the lock holds");
    let cannon = SmolStr::new("cannon");
    let before = arena
        .unit(RED)
        .and_then(|unit| unit.ammo.get(&cannon).copied())
        .expect("the cannon's rounds");
    arena
        .resolve_fire(RED, "cannon", arena.now_ms)
        .expect("the shot is taken");
    assert_eq!(
        arena
            .unit(RED)
            .and_then(|unit| unit.ammo.get(&cannon).copied()),
        Some(before - 1),
        "the round is spent whatever the line does"
    );
    assert!(
        arena
            .unit(RED)
            .and_then(|unit| unit.ready_at_ms.get(&cannon).copied())
            .is_some_and(|ready| ready > arena.now_ms),
        "and so is the cycle"
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 12, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    let reasons: Vec<String> = events
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::Miss { reason, .. } => Some(reason.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![tacsim::arena::damage::BLOCKED.to_owned()],
        "the round found the ridge"
    );
}

// ---------------------------------------------------------------------------
// X.6 (8) the cross-tempo property
// ---------------------------------------------------------------------------

/// One entry of a scripted log, keyed by **arena** time (16b cut X.6) — the
/// shape `tests/arena.rs`'s own driver takes, copied here because this file may
/// not edit that one.
struct ReliefBeat {
    t_ms: i64,
    intents: Vec<tacsim::arena::intents::Intent>,
}

/// One ledger line, normalised: arena timestamp, kind and fields as JSON, the
/// draw where one was made, and the substep hash.
type ReliefLine = (i64, String, Vec<(String, u64)>, u64);

/// One unit as a replay left it, **including the two numbers X.6 added**:
/// `(unit, x_cm, y_cm, z_cm, grade_pct, heading_mdeg, speed_cms, blocked)`.
type ReliefFinal = (mush_core::ObjId, i64, i64, i32, u16, u32, i32, bool);

/// What one replay produced.
struct ReliefReplay {
    events: Vec<ReliefLine>,
    hashes: Vec<u64>,
    finals: Vec<ReliefFinal>,
    /// Every distinct elevation the mover stood at, in order — the evidence that
    /// the run climbed anything at all. Sampled once a **pulse**, so it is a
    /// property of one cadence and is never compared across them.
    climbed: Vec<i32>,
    /// The mover's speed at the last sample of the northward leg: the sustained
    /// ceiling on the ramp, once the acceleration has finished with it.
    ramp_speed: i32,
    /// What the reflex controller did over the replay — the evidence that the
    /// slope-limited A\* was inside the property and not merely seated beside it.
    /// A per-run counter, never compared across cadences.
    reflex: tacsim::arena::reflex::ReflexCounters,
}

/// One order on the channel its kind travels on.
fn relief_intent(
    unit: mush_core::ObjId,
    kind: tacsim::arena::intents::IntentKind,
    seq: u64,
) -> tacsim::arena::intents::Intent {
    tacsim::arena::intents::Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

/// Drive one arena through `script` at `cadence_ms` until arena time reaches
/// `until_ms`, with the tempo pinned — [`replay`] in `tests/arena.rs`, over this
/// item's own final tuple.
fn relief_replay(
    arena: &mut Arena,
    mover: mush_core::ObjId,
    cadence_ms: i64,
    until_ms: i64,
    script: &[ReliefBeat],
) -> ReliefReplay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut climbed: Vec<i32> = Vec::new();
    let mut ramp_speed = 0_i32;
    let mut reflex = tacsim::arena::reflex::ReflexCounters::default();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    loop {
        arena
            .clock
            .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
        let mut intents: Vec<tacsim::arena::intents::Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            intents.extend(beat.intents.iter().cloned());
            next += 1;
        }
        let out = arena.step(wall, &[], &intents);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        reflex.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        if let Some(unit) = arena.unit(mover) {
            if climbed.last() != Some(&unit.pose.z_cm) {
                climbed.push(unit.pose.z_cm);
            }
            // The northward leg only: once the mover turns east it is braking
            // into a cliff, and the slope it last came up is still on it.
            if unit.grade_pct > 0 && unit.speed_cms > 0 && unit.pose.heading_mdeg == 0 {
                ramp_speed = unit.speed_cms;
            }
        }
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    ReliefReplay {
        events: String::from_utf8(bytes)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).ok()
            })
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.draws,
                    event.hash,
                )
            })
            .collect(),
        hashes,
        finals: arena
            .units
            .iter()
            .map(|unit| {
                (
                    unit.id,
                    unit.pose.x_cm,
                    unit.pose.y_cm,
                    unit.pose.z_cm,
                    unit.grade_pct,
                    unit.pose.heading_mdeg,
                    unit.speed_cms,
                    unit.blocked,
                )
            })
            .collect(),
        climbed,
        ramp_speed,
        reflex,
    }
}

/// The window the scripted climb runs on: a 30 per cent ramp northward from row
/// 10 to row 19, a plateau above it, and an 800 cm cliff eastward at column 24
/// that no `tracked` unit may cross in either direction (16b cut X.6).
fn scripted_relief() -> ArenaDefinition {
    let mut definition = relief(32, 32, &|x: u16, y: u16| {
        let base = match y {
            0..=9 => 0_i32,
            10..=19 => (i32::from(y) - 9) * 300,
            _ => 3_000,
        };
        let step = if x >= 24 { 800 } else { 0 };
        i16::try_from(base + step).unwrap_or(i16::MAX)
    });
    definition.waypoints = relief_waypoints((5, 2), (20, 28));
    definition
}

/// The script: start up cold, drive north up the ramp, then turn east into the
/// cliff and stand against it (16b cut X.6).
///
/// Every timestamp is a multiple of 5,000 ms, which is a pulse boundary at all
/// three cadences, so one script means one thing in all three replays.
fn scripted_climb(mover: mush_core::ObjId) -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    vec![
        ReliefBeat {
            t_ms: 0,
            intents: vec![relief_intent(mover, IntentKind::Startup, 1)],
        },
        ReliefBeat {
            t_ms: 10_000,
            intents: vec![
                relief_intent(mover, IntentKind::Heading { mdeg: 0 }, 2),
                relief_intent(mover, IntentKind::Speed { cms: 1_500 }, 3),
            ],
        },
        ReliefBeat {
            t_ms: 25_000,
            intents: vec![
                relief_intent(mover, IntentKind::Heading { mdeg: 90_000 }, 4),
                relief_intent(mover, IntentKind::Speed { cms: 1_500 }, 5),
            ],
        },
    ]
}

/// The two units the script drives: a pilot's tank on the ramp and a reflex tank
/// patrolling the same relief, so the A\* with a slope limit is inside the
/// property and not beside it (16b cut X.6).
fn scripted_climb_arena(catalog: &Catalog, cfg: &ArenaConfig) -> Arena {
    const MOVER: mush_core::ObjId = mush_core::ObjId(4);
    const WALKER: mush_core::ObjId = mush_core::ObjId(9);
    arena_over(
        catalog,
        vec![
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                MOVER,
                Team::Red,
                (5, 5),
            )),
            under_reflex(blind(unit_of(
                catalog,
                content::LINE_TANK,
                WALKER,
                Team::Red,
                (5, 2),
            ))),
        ],
        scripted_relief(),
        cfg,
    )
}

/// **One scripted climb at three cadences is one event list and one hash
/// sequence** (16b §11's mandatory cross-tempo test for this cut; doc 16 §3.1
/// "Rate independence", D-16-2).
///
/// The tempo never decides an outcome, and X.6 put two new numbers into the
/// world that a cadence could have moved: the slope a unit entered its cell by,
/// which its next ceiling is measured against, and the elevation it stands at.
/// Both are folded into the substep hash, so a replay that took the ramp one
/// substep early diverges at that substep rather than at the end of the match.
/// The reflex tank in the same replay puts the slope-limited A\* inside the
/// property too.
#[test]
fn x6_one_scripted_climb_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    const MOVER: mush_core::ObjId = mush_core::ObjId(4);
    const WALKER: mush_core::ObjId = mush_core::ObjId(9);
    let catalog = catalog();
    let mut runs: Vec<(i64, ReliefReplay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_climb_arena(&catalog, &ArenaConfig::default());
        let run = relief_replay(&mut arena, MOVER, cadence, 60_000, &scripted_climb(MOVER));
        assert_eq!(
            arena.now_ms, 60_000,
            "cadence {cadence} ran to a different time"
        );
        // The second tank has to have driven the slope-limited search, or the
        // three replays would agree about a machine that never started: an empty
        // history is trivially equal to an empty history, and the A\* would be
        // seated beside the property rather than inside it.
        assert!(
            run.reflex.path_queries > 0,
            "the walker asked for no route at {cadence} ms"
        );
        assert_eq!(run.reflex.path_failures, 0, "at {cadence} ms");
        assert_ne!(
            arena
                .definition
                .terrain
                .cell_of(&arena.unit(WALKER).expect("the walker").pose),
            Some((5, 2)),
            "the walker never left its opening cell at {cadence} ms"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth asserting before it is compared.
    assert!(
        first.climbed.len() >= 4 && first.climbed.iter().any(|z| *z >= 2_700),
        "the mover barely changed height: {:?}",
        first.climbed
    );
    assert_eq!(
        first.ramp_speed, 1_153,
        "the mover never held the surcharged ceiling"
    );
    let blocked = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.starts_with("{\"kind\":\"blocked\",\"unit\":4,"))
        .count();
    assert_eq!(
        blocked,
        1,
        "exactly one stop, and it is the cliff: {:?}",
        first
            .events
            .iter()
            .map(|(_, kind, _, _)| kind.clone())
            .collect::<Vec<String>>()
    );
    let mover = first
        .finals
        .iter()
        .find(|(id, ..)| *id == MOVER)
        .copied()
        .expect("the mover");
    assert_eq!(mover.1, 23_999, "the mover is not against the cliff");
    assert_eq!(mover.6, 0, "the mover is not stopped");
    assert!(mover.7, "the mover is not held");

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(
            run.finals, first.finals,
            "a final pose, z, grade or speed differs at {cadence} ms"
        );
    }
}

/// **The climb does not depend on the substep length either** (16b cut X.6).
///
/// The three cadences of the test above all run 250 ms substeps, because
/// `substep_ms` is `min(substep_max_ms, (cell_cm - 1) × 1000 / max_speed_cms)`
/// (16b cut Q.6 took that centimetre back) and depends on neither the tempo nor
/// the pulse. So the substep length is covered here instead, by moving the
/// table's own `substep_max_ms`: the sustained speed on the ramp and the cell the
/// mover is refused at are the same at 250 ms and at 150 ms.
///
/// What is **not** asserted identical, deliberately: the transient — a shorter
/// substep quantises the acceleration finer, so the pose at a given arena
/// millisecond differs by centimetres.
///
/// **The *count* of `blocked` lines is asserted, since 16b cut M.3** (16c
/// `T-04`). At 150 ms a 5,000 ms pulse is thirty-three substeps of 150 and one
/// of 50, and on that 50 ms piece a held tank gains 5 cm/s and travels no whole
/// centimetre. Until M.3 the ledger's line fired on the rising edge of the
/// per-substep `UnitState::blocked` flag, which such a substep cannot set, so
/// the next refusal was a second rising edge: 250 ms printed one and 150 ms
/// printed five (16b cut Q.6 measured it, and it was not the truncation it had
/// been read as — the carries did not move it). The pilot's line, on the `held`
/// **condition**, was said once at every partition. M.3 moved the ledger line
/// onto `held`'s new edge — the cell and the cause the unit is held against,
/// folded into `state_hash` — so the line, the notice and the machine's lesson
/// share one edge, and the cliff is **one** `blocked` line at either substep
/// length (`arena::motion`'s `one_obstacle_is_one_stop_at_every_partition` is
/// the same property over four partitions).
#[test]
fn x6_the_climb_does_not_depend_on_the_substep_length() {
    const MOVER: mush_core::ObjId = mush_core::ObjId(4);
    let catalog = catalog();
    let mut answers: Vec<(i32, i64)> = Vec::new();
    let mut blocked_counts: Vec<usize> = Vec::new();
    for substep_max_ms in [250_u32, 150] {
        let cfg = ArenaConfig {
            tempo: tacsim::config::TempoConfig {
                substep_max_ms,
                ..tacsim::config::TempoConfig::default()
            },
            ..ArenaConfig::default()
        };
        let mut arena = scripted_climb_arena(&catalog, &cfg);
        let run = relief_replay(&mut arena, MOVER, 5_000, 60_000, &scripted_climb(MOVER));
        let mover = run
            .finals
            .iter()
            .find(|(id, ..)| *id == MOVER)
            .copied()
            .expect("the mover");
        let cell = arena
            .definition
            .terrain
            .cell_at(mover.1, mover.2)
            .expect("on the window");
        answers.push((run.ramp_speed, i64::from(cell.0)));
        blocked_counts.push(
            run.events
                .iter()
                .filter(|(_, kind, _, _)| kind.starts_with("{\"kind\":\"blocked\",\"unit\":4,"))
                .count(),
        );
    }
    assert_eq!(
        answers.first(),
        answers.get(1),
        "the ramp's ceiling or the cell of the refusal moved with the substep length"
    );
    assert_eq!(
        answers.first().map(|answer| answer.0),
        Some(1_153),
        "the surcharged ceiling is the one rule's own number"
    );
    assert_eq!(
        answers.first().map(|answer| answer.1),
        Some(23),
        "the mover is refused at the cliff either way"
    );
    // 16b cut M.3 (16c `T-04`): one stop at the cliff is one line at either
    // substep length — this was printed, `1` against `5`, until the line moved
    // onto `held`'s edge.
    assert_eq!(
        blocked_counts,
        vec![1, 1],
        "one `blocked` line at the cliff at 250 ms and at 150 ms"
    );
}

// ---------------------------------------------------------------------------
// X.6 (9) a refused route is bounded work
// ---------------------------------------------------------------------------

/// **A route the ground refuses is bounded work** (16b cut X.6; doc 16 §1 A2,
/// §6's path budget).
///
/// The failure mode X.6 could have introduced: a reflex unit whose goal is behind
/// a cliff it cannot climb, re-planning at the cliff for the rest of the match.
/// The two bounds that stop it are the controller's own and predate this cut —
/// one search per unit per `repath_ms`, and `max_path_queries` per `decide_ms`
/// window — and this is the measurement that they still hold when the search can
/// fail on a slope rather than on a wall. The numbers are printed for the ledger;
/// whatever they are, they are the answer.
#[test]
fn x6_a_refused_route_is_bounded_work() {
    let catalog = catalog();
    let cfg = ArenaConfig::default();
    let span_ms = 60_000_i64;
    let pulses = usize::try_from(span_ms / 500).unwrap_or(0);
    let bound = u32::try_from(span_ms / i64::from(cfg.reflex.repath_ms) + 1).unwrap_or(u32::MAX);

    // (a) a limited reflex tank whose goal is behind an unbroken cliff line, and
    // whose graph offers it nowhere else to be: the worst case, a unit that will
    // keep asking for a route to a cell no route reaches for the whole match.
    let mut definition = plateau(false);
    definition.waypoints = relief_waypoint((40, 2));
    let mut arena = arena_over(
        &catalog,
        vec![under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (2, 4),
        )))],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run_logged(&mut arena, &mut wall, 500, pulses, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    let (cell, z, _) = standing(&arena, RED);
    assert!(
        cell.0 < 20 && z == 0,
        "the tank got onto a plateau nothing can climb: {cell:?} at {z} cm"
    );
    assert!(
        counters.path_queries <= bound,
        "{} searches in {span_ms} ms of arena time, against a bound of {bound}",
        counters.path_queries
    );
    let speed = arena.unit(RED).map(|unit| unit.speed_cms).unwrap_or(0);
    let held = arena.unit(RED).is_some_and(|unit| unit.held.is_some());
    assert!(
        speed == 0 || held,
        "the tank ended neither at rest nor held: {speed} cm/s"
    );
    println!(
        "harness_x6_refused_a: blocked={} path_queries={} path_deferred={} path_failures={}",
        blocked_cells(&events, RED).len(),
        counters.path_queries,
        counters.path_deferred,
        counters.path_failures
    );

    // (b) two tanks and one ramp, the second parked on the ramp.
    let mut definition = plateau(true);
    definition.waypoints = relief_waypoints((2, 4), (40, 2));
    let mut arena = arena_over(
        &catalog,
        vec![
            under_reflex(blind(unit_of(
                &catalog,
                content::LINE_TANK,
                RED,
                Team::Red,
                (2, 4),
            ))),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                BLUE,
                Team::Red,
                (21, 7),
            )),
        ],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run_logged(&mut arena, &mut wall, 500, pulses, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        counters.path_queries <= bound.saturating_mul(2),
        "{} searches for two units against a bound of {}",
        counters.path_queries,
        bound.saturating_mul(2)
    );
    println!(
        "harness_x6_refused_b: blocked={} path_queries={} path_deferred={} path_failures={}",
        blocked_cells(&events, RED).len(),
        counters.path_queries,
        counters.path_deferred,
        counters.path_failures
    );
}

// ---------------------------------------------------------------------------
// X.6 (10) what the limited search costs
// ---------------------------------------------------------------------------

/// A deterministic 256 by 256 relief: eight terraces 700 cm apart — a 70 per cent
/// seam no `tracked` unit may cross — with a landing half way up the seam on the
/// ramp rows, which is two 35 per cent steps and is the only way between terraces
/// (16b cut X.6).
fn terraces() -> ArenaTerrain {
    relief(256, 256, &|x: u16, y: u16| {
        let terrace = i32::from(x / 32);
        let base = terrace * 700;
        if x.is_multiple_of(32) && terrace > 0 && y % 16 < 4 {
            return i16::try_from(base - 350).unwrap_or(i16::MAX);
        }
        i16::try_from(base).unwrap_or(i16::MAX)
    })
    .terrain
}

/// **The limited search costs what it costs** (16b cut X.6; doc 16 §1 A2).
///
/// The harness measures the surcharge and nothing else — every harness unit is
/// `mobility: None`, so it has no limit and the edge rule never runs on the
/// production window — so this is the one measurement of the limit and the
/// shoulder rule, on a window deliberately harder than the shipped one. Both
/// searches stay inside the 65,536-node budget, every route the limited one
/// returns is a route motion would accept, and the two expansion totals are
/// printed for the ledger.
#[test]
fn x6_the_limited_search_costs_what_it_costs() {
    let terrain = terraces();
    let cell_cm = u32::from(terrain.cell_cm());
    let scale = tacsim::arena::min_move_cost(&terrain);
    let limit = tracked_limit();
    let mut scratch = tacsim::arena::PathScratch::default();
    let pairs = [
        ((2_u16, 2_u16), (250_u16, 250_u16)),
        ((4, 200), (240, 20)),
        ((16, 128), (200, 130)),
        ((120, 8), (130, 248)),
        ((60, 60), (196, 196)),
    ];

    let mut plain_total = 0_u64;
    let mut limited_total = 0_u64;
    // The node counts alone read the wrong way round — a refused edge is a branch
    // never pushed, so the limit *narrows* the frontier here — while the work per
    // edge goes up: a limited expansion reads two more slopes an edge and, on a
    // diagonal, two shoulders. The harness cannot see any of that, because every
    // harness unit is profile-less, so the wall clock is taken here as well. It
    // is printed, never asserted: this is a measurement for the ledger, not a
    // gate, and a gate on a shared machine's clock would be a flake.
    let mut plain_us = 0_u128;
    let mut limited_us = 0_u128;
    for (start, goal) in pairs {
        let at = std::time::Instant::now();
        let plain = tacsim::arena::path::path(&terrain, &mut scratch, start, goal, 65_536, scale)
            .unwrap_or_else(|why| panic!("{start:?} -> {goal:?} unlimited: {why:?}"));
        plain_us += at.elapsed().as_micros();
        let at = std::time::Instant::now();
        let limited = tacsim::arena::path_with(
            &terrain,
            &mut scratch,
            start,
            goal,
            65_536,
            scale,
            Some(limit),
            tacsim::arena::CostRow::Tracked,
            &[],
        )
        .unwrap_or_else(|why| panic!("{start:?} -> {goal:?} limited: {why:?}"));
        limited_us += at.elapsed().as_micros();
        plain_total += u64::from(plain.expanded);
        limited_total += u64::from(limited.expanded);

        // Every edge of the limited route is one motion would accept, shoulders
        // and all: the promise the search makes is the promise the crossing keeps.
        let mut walked = vec![start];
        walked.extend(limited.cells.iter().copied());
        for pair in walked.windows(2) {
            let (Some(from), Some(to)) = (pair.first().copied(), pair.get(1).copied()) else {
                continue;
            };
            let (Some(here), Some(there), Some(kind)) = (
                terrain.get(from.0, from.1),
                terrain.get(to.0, to.1),
                tacsim::arena::slope::step_kind(from, to),
            ) else {
                panic!("{from:?} -> {to:?} is not a step");
            };
            assert!(
                !tacsim::arena::slope::too_steep(
                    Some(limit),
                    tacsim::arena::slope::steepness_pct(here, there, kind, cell_cm)
                ),
                "{from:?} -> {to:?} is past the limit"
            );
            if kind == mush_atlas::physics::StepKind::Diagonal {
                for shoulder in [(to.0, from.1), (from.0, to.1)] {
                    let Some(cell) = terrain
                        .get(shoulder.0, shoulder.1)
                        .filter(|cell| cell.passable())
                    else {
                        continue;
                    };
                    for (a, b) in [(here, cell), (cell, there)] {
                        assert!(
                            !tacsim::arena::slope::too_steep(
                                Some(limit),
                                tacsim::arena::slope::steepness_pct(
                                    a,
                                    b,
                                    mush_atlas::physics::StepKind::Orthogonal,
                                    cell_cm
                                )
                            ),
                            "the shoulder {shoulder:?} of {from:?} -> {to:?} is past the limit"
                        );
                    }
                }
            }
        }
    }
    println!(
        "harness_x6_search: pairs={} expanded_unlimited={plain_total} expanded_limited={limited_total} us_unlimited={plain_us} us_limited={limited_us}",
        pairs.len()
    );
}

// ---------------------------------------------------------------------------
// Q.2 — the band cache has one writer, and it is true of the unit at every step
// ---------------------------------------------------------------------------

/// **`UnitState::condition` equals `Condition::of` its own unit, for every unit
/// in the match** (16b cut Q.2).
///
/// The whole reason `Arena::set_unit` exists: cut W.2's rule was that a unit's
/// cells move only through one `mush_tactics::UnitMutationPlan`, but the
/// *assignment* had two production sites — the impact and the heat plan — and a
/// cache that either of them forgot would be a `SCAN` telling a gunner a spent
/// turret is green. This is that invariant, asserted rather than grepped for.
fn cache_agrees(arena: &Arena, at: &str) {
    let sensors = SensorsConfig::default();
    for state in &arena.units {
        assert_eq!(
            state.condition,
            tacsim::arena::condition::Condition::of(
                &state.unit,
                sensors.condition_green_permille,
                sensors.condition_yellow_permille,
            ),
            "{at}: the band cache on #{} has rotted away from its unit",
            state.id.0
        );
    }
}

/// The laser duel with blue's left track already spent at check-out, so the match
/// opens on 16b cut F.1's entry consequences with a section that is already gone.
fn duel_with_a_spent_track() -> Arena {
    let catalog = catalog();
    let mut blue = unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL);
    let addr = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("left_track").expect("a section"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL).expect("a cell"),
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 0 }],
        None,
        Some(SmolStr::new("test")),
    )
    .expect("a one-op plan");
    let (post, _) = mush_tactics::apply(&blue.unit, &plan, 0).expect("the track is spent");
    blue.unit = post;
    blue.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    let mut arena = arena_of(
        &catalog,
        vec![
            unit_of(&catalog, content::LINE_TANK_LASER, RED, Team::Red, RED_CELL),
            blue,
        ],
        &[],
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
    arena
        .resolve_lock(RED, contact, arena.now_ms)
        .expect("the lock holds");
    arena
}

#[test]
fn q2_the_band_cache_survives_an_entry_consequence_a_beam_and_a_heat_cycle() {
    let mut arena = duel_with_a_spent_track();

    // (1) **The entry consequence.** `Arena::new` computes the first value off
    //     the record the world handed over, so a tank that checks in with a
    //     spent track reads `destroyed` there from the opening frame rather than
    //     from the first round that lands.
    cache_agrees(&arena, "at check-out");
    let bands = arena
        .unit(BLUE)
        .map(|state| state.condition.letters())
        .unwrap_or_default();
    assert_eq!(
        bands, "ggdgg",
        "the stock tank reads engine, hull, left_track, right_track, turret"
    );

    // (2) **The heat cycle.** A laser pulse writes the hull's heat cell through
    //     `crate::arena::heat`, which is the second of the two production
    //     writers `Arena::set_unit` took over. A heat cell moves no band — which
    //     is exactly why it is the writer that would have been forgotten.
    ready(&mut arena);
    arena
        .resolve_fire(RED, "laser", arena.now_ms)
        .expect("the shot");
    assert_eq!(
        arena.unit(RED).and_then(UnitState::heat_now),
        Some(20),
        "the shot really heated the tank"
    );
    cache_agrees(&arena, "after the shot's heat");
    let mut wall = 0_i64;
    for _ in 0..8 {
        run(&mut arena, &mut wall, 500, 1);
        cache_agrees(&arena, "through the cooling");
    }
    assert!(
        arena
            .unit(RED)
            .and_then(UnitState::heat_now)
            .is_some_and(|heat| heat < 20),
        "the tank really cooled"
    );

    // (3) **The hit.** The impact is the other production writer, and this is
    //     the one that does move a band: enough beams into one tank take its
    //     armour down through the two floors.
    let before = arena
        .unit(BLUE)
        .map(|state| state.condition)
        .unwrap_or_default();
    for _ in 0..60 {
        ready(&mut arena);
        // The heat ceiling is what stops a laser firing on demand, so a refused
        // shot is answered by letting it cool rather than by clearing the cell:
        // the ceiling is `x1_heat_rises_per_shot...`'s property, and driving it
        // honestly here is what keeps both writers in the run.
        if arena.resolve_fire(RED, "laser", arena.now_ms).is_err() {
            run(&mut arena, &mut wall, 1_000, 4);
            cache_agrees(&arena, "while the laser cooled");
            continue;
        }
        run(&mut arena, &mut wall, 500, 2);
        cache_agrees(&arena, "through the beams");
    }
    let after = arena
        .unit(BLUE)
        .map(|state| state.condition)
        .unwrap_or_default();
    assert_ne!(
        after, before,
        "forty beams moved no band at all, so the invariant above proves nothing"
    );
}

// ---------------------------------------------------------------------------
// Q.6 — the four cross-tempo tests 16b §11 owes for X.2 to X.5, and the jammed
// share mesh (doc 16 W6's last clause)
// ---------------------------------------------------------------------------

/// The mate on the share mesh (16b cut Q.6).
const MESH_MATE: mush_core::ObjId = mush_core::ObjId(5);
/// The second target of the thermal replay, the far one.
const FAR_TARGET: mush_core::ObjId = mush_core::ObjId(11);

/// **A window of any shape for the Q.6 replays** (16b cut Q.6).
///
/// [`definition`] is 16 by 16 because that is the shape every X-cut fixture
/// wants; the thermal sight reads to 450 m and the jamming bubble is 60 m wide,
/// so three of the four replays below need a corridor rather than a square. The
/// entry zones are the two ends of it, as [`definition`]'s are of its own.
fn q6_window(width: u16, height: u16) -> ArenaDefinition {
    let cells = vec![ground(); usize::from(width) * usize::from(height)];
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.q6"),
        terrain: ArenaTerrain::new(width, height, CELL_CM, cells).expect("the q6 window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: width - 1,
                y1: 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: 0,
                y0: height - 2,
                x1: width - 1,
                y1: height - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::default(),
        modes: Vec::new(),
    }
}

/// [`arena_of`] over a window the caller chose (16b cut Q.6).
fn q6_arena(catalog: &Catalog, units: Vec<CustodyUnit>, definition: ArenaDefinition) -> Arena {
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
            definition,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the q6 arena")
}

/// One unit as a Q.6 replay left it: `(unit, x_cm, y_cm, heading_mdeg, speed_cms,
/// slow_pct, motion_disabled, blocked)`.
type Q6Final = (mush_core::ObjId, i64, i64, u32, i32, u8, bool, bool);

/// What one Q.6 replay produced — [`ReliefReplay`] without X.6's climb
/// instruments, which none of these four fixtures climbs anything to measure.
///
/// **16b cut D.4** gave it the two things `tests/perception.rs`' own `Cadenced`
/// has had since cut Q.1 and this one lacked: the ordered list of notices each
/// **pilot** was staged, and the pulse count. A ledger line and the notice the
/// pilot reads for it are two different projections — the notice is staged by
/// `Arena::stage_pilot_notice` and drained per pulse — so a change that staged a
/// notice to the wrong platform, or on every pass instead of on the transition,
/// moves no event and no hash and would go unseen here. Every fixture in this
/// file seats a pilot on every unit ([`q6_arena`]), so the lists exist on all
/// four replays and cost nothing to compare.
///
/// **What is compared is the staged [`FrameNotice`] and not the rendered
/// sentence**, and the difference matters on exactly these fixtures. A contact
/// notice renders as `New contact X2: bearing 129°, close.`, and the bearing and
/// the band are read off the **frame** at render time
/// (`cockpit::notice_line`) — that is, off where the observer stood at the pulse
/// boundary the frame was built at. Every fixture here drives its observer, so
/// the same staged notice legitimately renders a different bearing at 5,000 ms
/// and at 500 ms. A frame is a per-pulse view by design (doc 16 §3.1) and is in
/// no cross-tempo comparison; the notice behind it is arena state and is.
struct Q6Replay {
    events: Vec<ReliefLine>,
    hashes: Vec<u64>,
    finals: Vec<Q6Final>,
    /// Every notice each pilot was staged, in order. Compared across cadences.
    notices: std::collections::BTreeMap<mush_core::ObjId, Vec<FrameNotice>>,
    /// The same notices as the pilot read them. **Not** compared across
    /// cadences, for the reason above; it is here so a property about a sentence
    /// can be asserted against a run of the real engine rather than a hand-built
    /// `FrameNotice`.
    lines: std::collections::BTreeMap<mush_core::ObjId, Vec<String>>,
    /// How many pulses it took, which is the guard that the three runs really
    /// were three cadences.
    pulses: u32,
    /// **What the run's perception cost, summed over every pulse** (16b cut
    /// U.2, 16c `T-29`). Its `jam_checks` and `traces` are compared across
    /// cadences: the jamming memo is cleared at substep boundaries and at a
    /// pod's power and wreck, all of them arena timestamps, so how often a
    /// bubble is scanned may not depend on how often the arena was woken.
    perception: tacsim::arena::perception::PerceptionCounters,
}

impl Q6Replay {
    /// Every ledger line of `kind` whose JSON contains all of `needles`, as
    /// `(t_ms, json)` — the instrument these four properties read, because what
    /// each is about is *when* a particular contact or consequence arrived, and a
    /// fixture with three units in it journals several lines of each kind.
    fn lines(&self, kind: &str, needles: &[&str]) -> Vec<(i64, String)> {
        let head = format!("{{\"kind\":\"{kind}\",");
        self.events
            .iter()
            .filter(|(_, json, _, _)| {
                json.starts_with(&head) && needles.iter().all(|needle| json.contains(needle))
            })
            .map(|(t_ms, json, _, _)| (*t_ms, json.clone()))
            .collect()
    }
}

/// Drive one arena through `script` at `cadence_ms` until arena time reaches
/// `until_ms`, with the tempo pinned — [`relief_replay`] over [`Q6Final`].
fn q6_replay(arena: &mut Arena, cadence_ms: i64, until_ms: i64, script: &[ReliefBeat]) -> Q6Replay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    // 16b cut D.4: the pilots' own notices, collected as one ordered list per
    // pilot over the whole run and never grouped by frame — a pulse that carries
    // three substeps' notices at 5,000 ms carries them one at a time at 500.
    let mut notices: std::collections::BTreeMap<mush_core::ObjId, Vec<FrameNotice>> =
        std::collections::BTreeMap::new();
    let mut lines: std::collections::BTreeMap<mush_core::ObjId, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut pulses = 0_u32;
    // 16b cut U.2: the run's perception cost, every pulse's summed.
    let mut perception = tacsim::arena::perception::PerceptionCounters::default();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut ended = false;
    loop {
        arena
            .clock
            .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
        let mut intents: Vec<tacsim::arena::intents::Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            intents.extend(beat.intents.iter().cloned());
            next += 1;
        }
        // Each beat goes in a pulse of **no wall time at all**, which is
        // `tests/resolution.rs`'s rule and the reason these replays compare at
        // all: phase 4 resolves a discrete action at the arena millisecond its
        // pulse ended at, so a beat handed to a pulse that advances nothing
        // resolves at exactly the millisecond the script names — at every cadence.
        if !intents.is_empty() {
            let out = arena.step(wall, &[], &intents);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            q6_absorb_notices(&out, &mut notices, &mut lines);
            perception.absorb(&out.perception);
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses = pulses.saturating_add(1);
            ended = out.ended.is_some();
        }
        if ended || arena.now_ms >= until_ms {
            break;
        }
        wall += cadence_ms;
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        q6_absorb_notices(&out, &mut notices, &mut lines);
        perception.absorb(&out.perception);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses = pulses.saturating_add(1);
        if out.ended.is_some() || arena.now_ms >= until_ms {
            break;
        }
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    Q6Replay {
        events: String::from_utf8(bytes)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).ok()
            })
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.draws,
                    event.hash,
                )
            })
            .collect(),
        hashes,
        finals: arena
            .units
            .iter()
            .map(|unit| {
                (
                    unit.id,
                    unit.pose.x_cm,
                    unit.pose.y_cm,
                    unit.pose.heading_mdeg,
                    unit.speed_cms,
                    unit.limits.slow_pct,
                    unit.limits.motion_disabled,
                    unit.blocked,
                )
            })
            .collect(),
        notices,
        lines,
        pulses,
        perception,
    }
}

/// One pulse's frames folded into the two per-pilot lists (16b cut D.4): the
/// staged notices, which are arena state and are compared across cadences, and
/// the sentences the pilot read, which are a per-pulse render and are not.
fn q6_absorb_notices(
    out: &tacsim::arena::step::StepOutput,
    notices: &mut std::collections::BTreeMap<mush_core::ObjId, Vec<FrameNotice>>,
    lines: &mut std::collections::BTreeMap<mush_core::ObjId, Vec<String>>,
) {
    for (pilot, frame) in &out.frames {
        if frame.notices.is_empty() {
            continue;
        }
        notices
            .entry(*pilot)
            .or_default()
            .extend(frame.notices.iter().cloned());
        let said = cockpit::notice_lines(frame);
        if !said.is_empty() {
            lines.entry(*pilot).or_default().extend(said);
        }
    }
}

/// **The three cadences, compared** (16b §11): one scripted log at 5,000, 1,000
/// and 500 ms gives one event list, one per-substep hash sequence, one per-pilot
/// notice list, one final state and (16b cut U.2) one total of jam checks and
/// traces (and, 16b cut J.1, of the cells those traces read) — with the sibling
/// tests' differing-pulses
/// guard, which is that every replay ran to exactly the same arena time **on a
/// different number of pulses**.
///
/// What a notice *renders as* is not in the comparison and [`Q6Replay`] says
/// why.
fn q6_three_cadences(
    label: &str,
    until_ms: i64,
    mut build: impl FnMut() -> Arena,
    script: &[ReliefBeat],
) -> Q6Replay {
    let mut runs: Vec<(i64, Q6Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = build();
        let run = q6_replay(&mut arena, cadence, until_ms, script);
        assert_eq!(
            arena.now_ms, until_ms,
            "{label}: cadence {cadence} ran to a different arena time"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("{label}: three replays");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "{label}: the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "{label}: the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        // 16b cut D.4: and what was staged to each pilot, which no hash folds.
        assert_eq!(
            run.notices, first.notices,
            "{label}: a pilot's notice list differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "{label}: a final pose, limit or speed differs at {cadence} ms"
        );
        // 16b cut U.2 (16c `T-29`): and what the perception cost. The jamming
        // memo is cleared at substep boundaries and at a pod's power and wreck,
        // all of them arena timestamps, so the scans it saves are the same at
        // every cadence — as are the traces the passes and the mesh walked.
        // **16b cut J.1**: and the cells those traces read, now that a trace
        // stops at its first blocker: a pass runs at its own arena timestamp
        // against the substep boundary's positions at every cadence, so the same
        // lines are walked and the same cells read up to the same blocker.
        // **16b cut J.2**: and the same blocks of the horizon index, which is
        // read-only and timestamp-free.
        assert_eq!(
            (
                run.perception.jam_checks,
                run.perception.traces,
                run.perception.cell_probes,
                run.perception.horizon_probes
            ),
            (
                first.perception.jam_checks,
                first.perception.traces,
                first.perception.cell_probes,
                first.perception.horizon_probes
            ),
            "{label}: the run's jam checks, traces, or cells or blocks read differ at \
             {cadence} ms"
        );
        assert_ne!(
            run.pulses, first.pulses,
            "{label}: {cadence} ms ran the same number of pulses as 5,000 ms, so the \
             comparison above is a tautology"
        );
    }
    let Some((_, first)) = runs.into_iter().next() else {
        panic!("{label}: three replays");
    };
    first
}

// --- X.2: a biped losing one leg and then the other ------------------------

/// **The scripted leg fight** (16b cut Q.6 for X.2): a stock tank seven cells
/// south of a walking biped, with the biped's whole facet table pointed at its
/// two legs so that the weighted draw can only ever choose between them.
///
/// The draw is still the arena's own (`choose_section` off the shot sequence), so
/// *which* leg a round takes is not the fixture's to say — what the fixture
/// guarantees is that every round takes one of them, and that both eventually go.
fn q6_leg_arena(catalog: &Catalog) -> Arena {
    let mut arena = q6_arena(
        catalog,
        vec![
            unit_of(catalog, content::LINE_TANK, RED, Team::Red, (8, 1)),
            unit_of(catalog, content::BIPED_MK1, BLUE, Team::Blue, (8, 8)),
        ],
        q6_window(16, 48),
    );
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| tacsim::arena::resolution::BandProfile {
                sections: vec![
                    (SmolStr::new("left_leg"), 1),
                    (SmolStr::new("right_leg"), 1),
                ],
                ..band.clone()
            })
            .collect();
    }
    // The shooter opens the match already holding its target, the way [`duel`]
    // does: this property is about the legs, not about acquisition.
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
    arena
}

/// Start up, walk north, turn about, walk back, and be shot at all the way
/// through — every timestamp a multiple of 5,000 ms, so one script means one
/// thing at all three cadences.
fn q6_leg_script() -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    let contact = tacsim::arena::rng::draw_contact_id(SEED, RED, BLUE);
    let mut beats = vec![
        ReliefBeat {
            t_ms: 0,
            intents: vec![
                relief_intent(RED, IntentKind::Startup, 1),
                relief_intent(BLUE, IntentKind::Startup, 1),
            ],
        },
        ReliefBeat {
            t_ms: 10_000,
            intents: vec![relief_intent(BLUE, IntentKind::Speed { cms: 1_200 }, 2)],
        },
        ReliefBeat {
            t_ms: 25_000,
            intents: vec![relief_intent(BLUE, IntentKind::Speed { cms: 0 }, 3)],
        },
        ReliefBeat {
            t_ms: 60_000,
            intents: vec![
                relief_intent(BLUE, IntentKind::Heading { mdeg: 180_000 }, 4),
                relief_intent(BLUE, IntentKind::Speed { cms: 1_200 }, 5),
            ],
        },
        ReliefBeat {
            t_ms: 70_000,
            intents: vec![relief_intent(BLUE, IntentKind::Speed { cms: 0 }, 6)],
        },
    ];
    // The cannon's cycle is 6,000 ms, so a round every ten seconds is a round
    // every time it is ready, and each leg takes four of them through the front
    // facet (260 of armour and 160 of internals against a 120-round).
    for (n, at) in (15_000_i64..=135_000).step_by(10_000).enumerate() {
        let seq = u64::try_from(n).unwrap_or(0).saturating_mul(2) + 10;
        beats.push(ReliefBeat {
            t_ms: at,
            intents: vec![
                relief_intent(RED, IntentKind::Lock { contact }, seq),
                relief_intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    seq + 1,
                ),
            ],
        });
    }
    // The driver walks the script in order and stops at the first beat the arena
    // has not reached, so the beats must be in **arena-time order**: they are
    // written above in two groups, the helm's and the gunner's.
    beats.sort_by_key(|beat| beat.t_ms);
    beats
}

/// **One scripted leg fight at three cadences is one event list and one hash
/// sequence** (16b §11's mandatory cross-tempo test, owed by 16b cut X.2; doc 16
/// §9 item 2, §3.1 "Rate independence").
///
/// X.2 put a mobility mode and a **pair** consequence into the world —
/// `disable_motion_when_all_of`, which fires on the second leg and not on the
/// first — and neither had a cross-tempo test. Both are in this one: the biped
/// walks under the legged numbers while it is being shot at, `slow(50)` fires on
/// the first leg, and the pair rule fires on the second at one arena millisecond
/// in all three replays.
#[test]
fn q6_one_scripted_leg_fight_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    let catalog = biped_catalog();
    let script = q6_leg_script();
    let first = q6_three_cadences("x2 legs", 140_000, || q6_leg_arena(&catalog), &script);

    // The run is worth comparing before it is compared: both legs went, and they
    // went at two different arena milliseconds.
    let legs: Vec<(i64, String)> = first
        .lines("section_destroyed", &["\"unit\":9"])
        .into_iter()
        .filter(|(_, json)| json.contains("_leg"))
        .collect();
    assert_eq!(legs.len(), 2, "the fight did not take both legs: {legs:?}");
    assert!(
        legs.get(1).map(|(at, _)| *at) > legs.first().map(|(at, _)| *at),
        "the two legs went at the same instant: {legs:?}"
    );
    // And the pair rule fired on the **second** of them. The ledger cannot show
    // that — a `section_destroyed` line carries the section's *authored*
    // consequences whatever actually fired, which is F.2's own ruling — so it is
    // read off the unit instead, at an arena time between the two losses. That is
    // a second three-cadence property in its own right: the biped is limping, and
    // not stopped, at the same arena millisecond at every cadence.
    let limping = q6_three_cadences(
        "x2 legs, one down",
        60_000,
        || q6_leg_arena(&catalog),
        &script,
    );
    assert_eq!(
        limping
            .lines("section_destroyed", &["\"unit\":9"])
            .iter()
            .filter(|(_, json)| json.contains("_leg"))
            .count(),
        1,
        "the halfway run did not take exactly one leg"
    );
    let one_down = limping
        .finals
        .iter()
        .find(|(id, ..)| *id == BLUE)
        .copied()
        .expect("the biped");
    assert_eq!(
        one_down.5, 50,
        "one leg is a limp: doc 16 §9 item 2's slow(50)"
    );
    assert!(
        !one_down.6,
        "the pair rule fired on the first leg: one leg is a limp, not a stop"
    );
    // And with both gone: a biped with no legs is still slowed to half and cannot
    // move at all.
    let biped = first
        .finals
        .iter()
        .find(|(id, ..)| *id == BLUE)
        .copied()
        .expect("the biped");
    assert_eq!(biped.5, 50, "the first leg's slow is still in force");
    assert!(biped.6, "the pair rule did not disable the biped's motion");
    assert_eq!(biped.4, 0, "a biped with no legs is standing still");
}

// --- X.3: the thermal signature against power and speed --------------------

/// **The scripted thermal watch** (16b cut Q.6 for X.3): one thermal sight and
/// two blind stock tanks, at the two ranges doc 16 §9 item 3's three states
/// separate.
///
/// The Mk1 thermal sight reads 45 cells, and the mount's own factors are 120 per
/// mille for a cold hull and 650 for an idling one, over the `ground` row's 980
/// stopped and 1,000 moving. So the reach against a stock tank on open ground is
/// 52 m cold, 286 m idling and 450 m under way — and 200 m and 340 m are the two
/// ranges that tell all three apart.
fn q6_thermal_arena(catalog: &Catalog) -> Arena {
    let mut arena = q6_arena(
        catalog,
        vec![
            unit_of(catalog, content::LINE_TANK_THERMAL, RED, Team::Red, (8, 1)),
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (8, 21),
            )),
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                FAR_TARGET,
                Team::Blue,
                (8, 35),
            )),
        ],
        q6_window(16, 48),
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    arena
}

/// Cold and stopped, then both started up, then the far one under way.
fn q6_thermal_script() -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    vec![
        ReliefBeat {
            t_ms: 20_000,
            intents: vec![
                relief_intent(BLUE, IntentKind::Startup, 1),
                relief_intent(FAR_TARGET, IntentKind::Startup, 1),
            ],
        },
        ReliefBeat {
            t_ms: 40_000,
            intents: vec![
                relief_intent(FAR_TARGET, IntentKind::Heading { mdeg: 180_000 }, 2),
                relief_intent(FAR_TARGET, IntentKind::Speed { cms: 1_500 }, 3),
            ],
        },
    ]
}

/// **One scripted thermal watch at three cadences is one event list and one hash
/// sequence** (16b §11's mandatory cross-tempo test, owed by 16b cut X.3; doc 16
/// §9 item 3).
///
/// X.3's term is keyed on the **target's** power and speed, both of which move
/// inside a pulse: a startup completes at a due event's own arena millisecond and
/// a tank is under way the substep its speed leaves zero. So the acquisition it
/// causes has to land at one arena millisecond whatever the cadence, which is
/// what the shared event list asserts — a cold, stopped tank at 200 m is dark, it
/// is found the moment it idles, and its neighbour at 340 m stays dark until it
/// is under way.
#[test]
fn q6_one_scripted_thermal_watch_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    let catalog = thermal_catalog();
    let script = q6_thermal_script();
    let first = q6_three_cadences("x3 thermal", 70_000, || q6_thermal_arena(&catalog), &script);

    let near = first.lines("detected", &["\"viewer\":4", "\"target\":9"]);
    let far = first.lines("detected", &["\"viewer\":4", "\"target\":11"]);
    assert_eq!(
        near.len(),
        1,
        "the near tank was found once and only once: {near:?}"
    );
    assert_eq!(far.len(), 1, "the far tank likewise: {far:?}");
    let (Some(near_ms), Some(far_ms)) = (
        near.first().map(|(at, _)| *at),
        far.first().map(|(at, _)| *at),
    ) else {
        panic!("one acquisition each");
    };
    assert!(
        near_ms >= 20_000,
        "a cold, stopped tank at 200 m was seen before it started up: {near_ms}"
    );
    assert!(
        near_ms < 40_000,
        "the near tank was not found when it began to idle: {near_ms}"
    );
    assert!(
        far_ms > 40_000,
        "the far tank was found before it was under way: {far_ms}"
    );
}

// --- X.4 and X.5: the jamming fold, and the filter against it --------------

/// **The scripted drive through a bubble** (16b cut Q.6 for X.4 and X.5): an
/// observer driving north up column 2, a blind stock tank standing at 90 m, and
/// the Mk1 pod parked two cells off the observer's track.
///
/// The Mk1 pod's bubble is 60 m and leaves 150 per mille of a target's signature.
/// The Mk1 eye reads 540 m against a stock tank stopped on open ground, so inside
/// the bubble it reads 81 m and the target at 90 m is gone; the Mk1 filter adds
/// 75 per mille on top of what the bubble left, which is 121 m, and the same
/// target is held throughout. That gap is the whole of X.5's acceptance, and this
/// fixture is what puts arena time through it.
fn q6_bubble_arena(catalog: &Catalog, watcher: &str) -> Arena {
    let mut arena = q6_arena(
        catalog,
        vec![
            unit_of(catalog, watcher, RED, Team::Red, (2, 1)),
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (11, 8),
            )),
            blind(unit_of(
                catalog,
                content::LINE_TANK_ECM,
                POD,
                Team::Blue,
                (4, 8),
            )),
        ],
        q6_window(16, 48),
    );
    for id in [RED, POD] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    arena
}

/// Drive north through the bubble and out the far side of it.
fn q6_bubble_script() -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    vec![
        ReliefBeat {
            t_ms: 10_000,
            intents: vec![relief_intent(RED, IntentKind::Speed { cms: 1_500 }, 1)],
        },
        ReliefBeat {
            t_ms: 40_000,
            intents: vec![relief_intent(RED, IntentKind::Speed { cms: 0 }, 2)],
        },
    ]
}

/// **One scripted drive through a jamming bubble at three cadences is one event
/// list and one hash sequence** (16b §11's mandatory cross-tempo test, owed by
/// 16b cut X.4; doc 16 §9 item 4).
///
/// X.4's fold is taken at the **observer's** pose, which moves every substep, so
/// the moment a bubble swallows a contact is a function of where the tank was
/// when its pass ran. A cadence that ran the pass half a pulse later would lose
/// the contact at a different arena millisecond, and the shared event list is
/// what forbids it.
#[test]
fn q6_one_scripted_drive_through_a_bubble_at_three_cadences_is_one_event_list() {
    let catalog = ecm_catalog();
    let script = q6_bubble_script();
    let first = q6_three_cadences(
        "x4 bubble",
        60_000,
        || q6_bubble_arena(&catalog, content::LINE_TANK),
        &script,
    );

    let found = first.lines("detected", &["\"viewer\":4", "\"target\":9"]);
    let lost = first.lines("lost", &["\"viewer\":4", "\"target\":9"]);
    // 16b cut U.2 (16c `T-29`): the pod was scanned for, so the three cadences'
    // equal jam-check totals are a comparison of something.
    assert!(
        first.perception.jam_checks > 0,
        "the drive through the bubble scanned for no pod: {:?}",
        first.perception
    );
    assert_eq!(
        lost.len(),
        1,
        "the bubble did not take the contact exactly once: {lost:?}"
    );
    assert_eq!(
        found.len(),
        2,
        "the contact was not found, lost and found again: {found:?}"
    );
    let (Some(first_ms), Some(lost_ms), Some(again_ms)) = (
        found.first().map(|(at, _)| *at),
        lost.first().map(|(at, _)| *at),
        found.get(1).map(|(at, _)| *at),
    ) else {
        panic!("two acquisitions and one loss");
    };
    assert!(
        first_ms < lost_ms && lost_ms < again_ms,
        "found {first_ms}, lost {lost_ms}, found again {again_ms}"
    );
}

/// **The filter holds what the bubble took, at every cadence** (16b §11's
/// mandatory cross-tempo test, owed by 16b cut X.5; doc 16 §9 item 5).
///
/// The same script, the same pod and the same target, driven twice: once by a
/// stock-sighted tank and once by its twin carrying the Mk1 filter. The
/// unfiltered observer loses the contact inside the bubble and finds it again on
/// the far side; the filtered one never loses it at all, which is `recovered`'s
/// 75 per mille as arena time reads it. Each run is its own three-cadence
/// property, so the recovery is rate-independent and not merely present.
#[test]
fn q6_the_filter_holds_the_contact_the_bubble_takes_at_three_cadences() {
    let catalog = pod_and_filter_catalog();
    let script = q6_bubble_script();

    let bare = q6_three_cadences(
        "x5 unfiltered",
        60_000,
        || q6_bubble_arena(&catalog, content::LINE_TANK),
        &script,
    );
    let filtered = q6_three_cadences(
        "x5 filtered",
        60_000,
        || q6_bubble_arena(&catalog, content::LINE_TANK_ECCM),
        &script,
    );

    assert_eq!(
        bare.lines("lost", &["\"viewer\":4", "\"target\":9"]).len(),
        1,
        "the unfiltered twin did not lose the contact"
    );
    assert!(
        filtered
            .lines("lost", &["\"viewer\":4", "\"target\":9"])
            .is_empty(),
        "the filter did not hold the contact the bubble took: {:?}",
        filtered.lines("lost", &["\"viewer\":4", "\"target\":9"])
    );
    assert_eq!(
        filtered
            .lines("detected", &["\"viewer\":4", "\"target\":9"])
            .len(),
        1,
        "the filtered observer found the target once and kept it"
    );
}

// --- doc 16 W6's last clause: the share mesh is jammed too -----------------

/// **One share pass, applied directly**, with the spatial index rebuilt first —
/// [`thermal_pass`]'s shape for the mesh. The jam scan reads the same index the
/// mate scan does, so the rebuild is what makes a pass driven directly the pass
/// `Arena::step`'s phase 3 runs.
fn q6_share(
    arena: &mut Arena,
    sharer: mush_core::ObjId,
    t_ms: i64,
) -> tacsim::arena::perception::PerceptionCounters {
    let mut counters = tacsim::arena::perception::PerceptionCounters::default();
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    let units = arena.units.clone();
    arena.index.rebuild(&arena.definition.terrain, &units);
    // 16b cut U.2: and the jamming memo with it, exactly as phase 3(b) empties
    // it beside its own rebuild — a pass driven directly reads through the memo.
    arena.jam_fold.clear();
    arena.share_pass(sharer, t_ms, &mut counters, &mut journal);
    counters
}

/// **A sharer, a blind mate and a target only the sharer can see** (16b cut Q.6),
/// with the pod parked where a fixture puts it.
///
/// The mate carries no sight at all, so the only way it can come to hold the
/// target is the mesh — which is what makes "the link was cut" and "the mate holds
/// nothing" the same sentence.
fn q6_mesh_arena(
    catalog: &Catalog,
    sharer_cell: (u16, u16),
    mate_cell: (u16, u16),
    pod: Option<(u16, u16)>,
    paint: &[(u16, u16, TerrainCell)],
) -> Arena {
    let mut units = vec![
        unit_of(catalog, content::LINE_TANK, RED, Team::Red, sharer_cell),
        blind(unit_of(
            catalog,
            content::LINE_TANK,
            MESH_MATE,
            Team::Red,
            mate_cell,
        )),
        blind(unit_of(
            catalog,
            content::LINE_TANK,
            BLUE,
            Team::Blue,
            (3, 2),
        )),
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
    for id in [RED, MESH_MATE, POD] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
        }
    }
    // The sharer opens holding the target, classified and seen by its own eye,
    // which is `ShadowContact::shareable`'s whole precondition.
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
    arena
}

/// Whether the mate came to hold the target after one share pass.
fn q6_meshed(arena: &mut Arena) -> bool {
    q6_share(arena, RED, 0);
    let id = tacsim::arena::rng::draw_contact_id(arena.seed, MESH_MATE, BLUE);
    arena.contacts.resolve(MESH_MATE, id).is_some()
}

/// Put the filter's recovery on one end of the link, by hand — the filter is a
/// fitting, and what this property is about is *which end carries it*.
fn q6_fit_filter(arena: &mut Arena, unit: mush_core::ObjId) {
    if let Some(state) = arena.unit_mut(unit) {
        state.eccm = Some(authored_filter());
    }
}

/// **A pod cuts the mesh by range** (doc 16 W6: *tactical comms are LOS mesh
/// jammed by the same ECM*; 16b cut Q.6).
///
/// Sixty metres of open ground between two teammates is well inside the Mk1
/// sight's 300 m share reach (30 cells), and a pod beside the receiver leaves 45 m
/// of it.
/// The link therefore goes, and it goes on the **range** term with the line still
/// perfectly clear — which is the half of the fold a clear trace cannot test.
#[test]
fn q6_a_pod_beside_one_end_cuts_the_mesh_by_range() {
    let catalog = ecm_catalog();
    // No pod: the link is made, and the mate holds what the sharer saw.
    let mut open = q6_mesh_arena(&catalog, (3, 8), (9, 8), None, &[]);
    assert!(q6_meshed(&mut open), "the unjammed link was not made");
    // A stock match never reaches the index for a pod that is not there, which is
    // what makes the share pass of a field with no ECM on it bit-identical to the
    // one cut F.3 left.
    let mut stock = q6_mesh_arena(&catalog, (3, 8), (9, 8), None, &[]);
    let counters = q6_share(&mut stock, RED, 0);
    assert_eq!(
        counters.jam_checks, 0,
        "a field with no pod on it scanned for one"
    );

    // The pod, four cells from the mate and eight from the sharer: the receiver
    // is inside the bubble and the sharer is not.
    let mut jammed = q6_mesh_arena(&catalog, (3, 8), (9, 8), Some((13, 8)), &[]);
    assert!(
        !q6_meshed(&mut jammed),
        "the pod beside the receiver did not cut the link"
    );
    // And it is the range and not the line: the same pod, the same open ground
    // and the same clear trace, with the two teammates three cells apart instead
    // of six, links — so what cut the link above was the sixty metres against the
    // forty-five the bubble left.
    let mut nearer = q6_mesh_arena(&catalog, (6, 8), (9, 8), Some((13, 8)), &[]);
    assert!(
        q6_meshed(&mut nearer),
        "thirty metres of the same clear ground did not link, so the cut was not the range"
    );

    // The filter goes on the end that is in the bubble, and nowhere else: a pod
    // is jamming the receiver, so the receiver is what has to recover.
    let mut wrong_end = q6_mesh_arena(&catalog, (3, 8), (9, 8), Some((13, 8)), &[]);
    q6_fit_filter(&mut wrong_end, RED);
    assert!(
        !q6_meshed(&mut wrong_end),
        "a filter on the unjammed end recovered somebody else's bubble"
    );
    let mut right_end = q6_mesh_arena(&catalog, (3, 8), (9, 8), Some((13, 8)), &[]);
    q6_fit_filter(&mut right_end, MESH_MATE);
    assert!(
        q6_meshed(&mut right_end),
        "the filter did not buy back the reach the pod took"
    );
}

/// **The end in the bubble may be the sharer's, and both terms cut the link just
/// the same** (doc 16 W6; 16b cut Q.6).
///
/// Every other mesh fixture in this file stands the pod beside the *receiver*, so
/// the sharer's own fold — taken once for the whole pass, before the mate loop —
/// is `None` in all of them and neither half of it decides anything. This is its
/// mirror image, and it is the case the rule exists for: a relay that jammed only
/// the ends it transmitted *to* would let a pod parked on top of an enemy spotter
/// blind its sights and still leave it free to hand the whole team every contact
/// it already held.
///
/// Both terms, because the sharer contributes to both: the **range**, at sixty
/// metres against the 45 m the bubble leaves of its reach, and the **budget**, at
/// thirty metres through one belt of light woods that costs 25 of the 15 the pod
/// leaves it. And the filter is the exact inverse of the pair beside it — it buys
/// nothing on the end that is not in the bubble, and buys back exactly what
/// `recovered` recovers on the end that is.
#[test]
fn q6_a_pod_beside_the_sharer_cuts_the_mesh_at_both_terms() {
    let catalog = ecm_catalog();

    // (1) The range. The sharer is four cells from the pod and the mate is ten, so
    //     this time it is the transmitting end that stands in the bubble.
    let mut open = q6_mesh_arena(&catalog, (9, 8), (3, 8), None, &[]);
    assert!(q6_meshed(&mut open), "the unjammed link was not made");
    let mut jammed = q6_mesh_arena(&catalog, (9, 8), (3, 8), Some((13, 8)), &[]);
    assert!(
        !q6_meshed(&mut jammed),
        "the pod beside the sharer did not cut the link"
    );
    let mut nearer = q6_mesh_arena(&catalog, (9, 8), (6, 8), Some((13, 8)), &[]);
    assert!(
        q6_meshed(&mut nearer),
        "thirty metres of the same clear ground did not link, so the cut was not the range"
    );
    let mut wrong_end = q6_mesh_arena(&catalog, (9, 8), (3, 8), Some((13, 8)), &[]);
    q6_fit_filter(&mut wrong_end, MESH_MATE);
    assert!(
        !q6_meshed(&mut wrong_end),
        "a filter on the unjammed end recovered somebody else's bubble"
    );
    let mut right_end = q6_mesh_arena(&catalog, (9, 8), (3, 8), Some((13, 8)), &[]);
    q6_fit_filter(&mut right_end, RED);
    assert!(
        q6_meshed(&mut right_end),
        "the filter did not buy back the reach the pod took from the sharer"
    );

    // (2) The budget, at a range the jammed reach still covers: one belt of light
    //     woods three cells from the sharer, which the Mk1 sight sees through and
    //     the jammed one does not.
    let belt = [(7_u16, 8_u16, woods())];
    let mut clear = q6_mesh_arena(&catalog, (9, 8), (6, 8), None, &belt);
    assert!(
        q6_meshed(&mut clear),
        "one belt of light woods is inside the Mk1 sight's budget"
    );
    let mut opaque = q6_mesh_arena(&catalog, (9, 8), (6, 8), Some((13, 8)), &belt);
    assert!(
        !q6_meshed(&mut opaque),
        "the pod did not take the budget the belt needed from the sharer"
    );
    let mut filtered = q6_mesh_arena(&catalog, (9, 8), (6, 8), Some((13, 8)), &belt);
    q6_fit_filter(&mut filtered, RED);
    assert!(
        q6_meshed(&mut filtered),
        "the filter did not buy back the budget the pod took from the sharer"
    );
}

/// **A pod cuts the mesh by the attenuation budget too** (doc 16 W6; 16b cut
/// Q.6), which is the other half of the same fold.
///
/// Thirty metres apart with one belt of light woods between them: the atlas
/// prices that belt at 25 and the Mk1 sight's budget is 40, so the link is made.
/// The Mk1 pod takes 25 of the budget and leaves 15, and the same belt is now
/// opaque — while the range term, at 30 m against the jammed reach of 45 m, is
/// still satisfied, so the budget is demonstrably what cut it.
#[test]
fn q6_a_pod_beside_one_end_cuts_the_mesh_by_the_budget() {
    let catalog = ecm_catalog();
    let belt = [(7_u16, 8_u16, woods())];
    let mut open = q6_mesh_arena(&catalog, (6, 8), (9, 8), None, &belt);
    assert!(
        q6_meshed(&mut open),
        "one belt of light woods is inside the Mk1 sight's budget"
    );

    let mut jammed = q6_mesh_arena(&catalog, (6, 8), (9, 8), Some((13, 8)), &belt);
    assert!(
        !q6_meshed(&mut jammed),
        "the pod did not take the budget the belt needed"
    );
    // The range term is not what cut it: the same pair with a clear line between
    // them still links.
    let mut clear = q6_mesh_arena(&catalog, (6, 8), (9, 8), Some((13, 8)), &[]);
    assert!(
        q6_meshed(&mut clear),
        "thirty metres is inside the jammed reach, so the range did not cut it"
    );

    let mut filtered = q6_mesh_arena(&catalog, (6, 8), (9, 8), Some((13, 8)), &belt);
    q6_fit_filter(&mut filtered, MESH_MATE);
    assert!(
        q6_meshed(&mut filtered),
        "the filter did not buy back the budget the pod took"
    );
}

/// **A pod that is shut down or burnt out jams no radio either** (16b cut Q.6),
/// which is X.4's own rule for sight, read once and applied twice.
#[test]
fn q6_a_shut_down_or_wrecked_pod_leaves_the_mesh_alone() {
    let catalog = ecm_catalog();
    for cold in [true, false] {
        let mut arena = q6_mesh_arena(&catalog, (3, 8), (9, 8), Some((13, 8)), &[]);
        if let Some(state) = arena.unit_mut(POD) {
            if cold {
                state.power = Power::Off;
            } else {
                state.limits.wreck = true;
            }
        }
        assert!(
            q6_meshed(&mut arena),
            "a pod that is not radiating cut the mesh (cold {cold})"
        );
    }
}

/// **The scripted jammed-then-unjammed share** (16b cut Q.6; 16b §11's mandatory
/// cross-tempo test for this cut's own behaviour change).
///
/// A pod parked between two teammates, driven out of the way half way through:
/// the blind mate loses the contact the mesh was keeping alive, and gets it back
/// when the pod leaves. Both the loss and the return are journalled, and both
/// have to land at one arena millisecond at 5,000, 1,000 and 500 ms.
fn q6_mesh_replay_arena(catalog: &Catalog) -> Arena {
    let mut arena = q6_mesh_arena(catalog, (3, 8), (9, 8), Some((13, 8)), &[]);
    // The mate opens holding the contact, put there by the mesh before the pod
    // arrived: what the replay is about is losing it and getting it back.
    let id = tacsim::arena::rng::draw_contact_id(arena.seed, MESH_MATE, BLUE);
    arena.contacts.insert(
        MESH_MATE,
        id,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            0,
            plot_of(&arena, BLUE),
        ),
    );
    // The mate's copy is a mesh copy: nothing of its own ever saw the target, so
    // it cannot relay and it expires when the mesh stops refreshing it.
    if let Some(held) = arena.contacts.get_mut(MESH_MATE, id) {
        held.own_seen = None;
    }
    arena
}

/// The pod drives east out of the bubble's reach of the mate.
fn q6_mesh_script() -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    vec![ReliefBeat {
        t_ms: 20_000,
        intents: vec![
            relief_intent(POD, IntentKind::Heading { mdeg: 90_000 }, 1),
            relief_intent(POD, IntentKind::Speed { cms: 1_500 }, 2),
        ],
    }]
}

/// **One scripted jammed-then-unjammed share at three cadences is one event list
/// and one hash sequence** (16b §11, mandatory for this cut).
#[test]
fn q6_one_scripted_jammed_share_at_three_cadences_is_one_event_list() {
    let catalog = ecm_catalog();
    let script = q6_mesh_script();
    let first = q6_three_cadences(
        "q6 mesh",
        60_000,
        || q6_mesh_replay_arena(&catalog),
        &script,
    );
    let lost = first.lines("lost", &["\"viewer\":5", "\"target\":9"]);
    let found = first.lines("detected", &["\"viewer\":5", "\"target\":9"]);
    // 16b cut U.2 (16c `T-29`): the mesh's two ends were folded through the
    // memo, so the equal totals at three cadences compare a real count.
    assert!(
        first.perception.jam_checks > 0 && first.perception.share_ops > 0,
        "the jammed share scanned for no pod: {:?}",
        first.perception
    );
    assert_eq!(
        lost.len(),
        1,
        "the jammed mate did not lose its mesh copy exactly once: {lost:?}"
    );
    assert_eq!(
        found.len(),
        1,
        "the mate did not get the contact back when the pod left: {found:?}"
    );
    let (Some(lost_ms), Some(found_ms)) = (
        lost.first().map(|(at, _)| *at),
        found.first().map(|(at, _)| *at),
    ) else {
        panic!("one loss and one return");
    };
    assert!(lost_ms < found_ms, "lost {lost_ms}, found again {found_ms}");
}

// --- 16b cut O.2: the pod's switch (16c N12) --------------------------------

/// **The ridge of [`o2_emcon_arena`]** (16b cut O.2): a three-cell block across
/// the pod's line to the target — (13, 8) to (3, 2) crosses columns 7 to 9 at
/// rows 4 to 6 — and nowhere near the sharer's column or the mates' row.
const O2_RIDGE: [(u16, u16); 9] = [
    (7, 4),
    (8, 4),
    (9, 4),
    (7, 5),
    (8, 5),
    (9, 5),
    (7, 6),
    (8, 6),
    (9, 6),
];

/// The ridge's height: thirty metres, far over any sight the catalog mounts.
const O2_RIDGE_CM: i16 = 3_000;

/// **The ECM tank inside its own bubble, with a teammate inside it too** (16b
/// cut O.2): the sharer three cells in from the west edge, holding the target
/// classified by its own eye; the blind mate six cells east of it; and the
/// *red* pod four cells east of the mate — so the mate and the pod's own sight
/// stand inside the 60 m bubble and the sharer, 100 m off, does not.
///
/// Sixty metres between sharer and mate is well inside the Mk1 sight's 300 m
/// share reach and outside the 45 m the pod leaves of it at the mate's end, so
/// while the pod radiates the link is cut by range (16b cut Q.6) and the pod's
/// own receiver reads degraded (a pod is in its own bubble). The switch is the
/// only thing the script moves.
///
/// **A ridge stands between the pod and the target** ([`O2_RIDGE`]), so the
/// silent pod's own sight never takes the target and the one sighting that can
/// reach the mate is the sharer's: the mate's first `detected` is the share the
/// radiating pod refused, completing, and not a fresh sighting relayed by the
/// pod (a sighting travels one hop, 16b cut F.3). The ridge is off the sharer's
/// line to the target (column 3) and off row 8, the sharer's and the pod's
/// lines to the mate.
fn o2_emcon_arena(catalog: &Catalog) -> Arena {
    let ridge: Vec<(u16, u16, TerrainCell)> = O2_RIDGE
        .iter()
        .map(|(x, y)| (*x, *y, at_elev(O2_RIDGE_CM)))
        .collect();
    let mut arena = arena_of(
        catalog,
        vec![
            unit_of(catalog, content::LINE_TANK, RED, Team::Red, (3, 8)),
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                MESH_MATE,
                Team::Red,
                (9, 8),
            )),
            blind(unit_of(
                catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (3, 2),
            )),
            unit_of(catalog, content::LINE_TANK_ECM, POD, Team::Red, (13, 8)),
        ],
        &ridge,
    );
    for id in [RED, MESH_MATE, POD] {
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
    arena
}

/// The pod's pilot switches it off at 10 s and on again at 30 s — both beats on
/// the 5,000 ms lattice, so each lands at its own millisecond at every cadence.
fn o2_emcon_script() -> Vec<ReliefBeat> {
    use tacsim::arena::intents::IntentKind;
    vec![
        ReliefBeat {
            t_ms: 10_000,
            intents: vec![relief_intent(POD, IntentKind::Emcon { on: false }, 1)],
        },
        ReliefBeat {
            t_ms: 30_000,
            intents: vec![relief_intent(POD, IntentKind::Emcon { on: true }, 2)],
        },
    ]
}

/// **`EMCON off` at a beat clears the receiver and the mesh, and `EMCON on`
/// re-degrades it, at three cadences** (16b §11's mandatory cross-tempo test,
/// owed by 16b cut O.2; 16c N12).
///
/// One event list, one hash sequence (item 1.19 folds the switch) and one
/// notice list at 5,000, 1,000 and 500 ms; the two `emcon` lines at the beats'
/// own milliseconds; the pod's own receiver degraded from its first pass,
/// cleared by exactly one `receiver` line at its first pass after the off
/// beat, and degraded again by exactly one after the on beat; and the share the
/// radiating pod refused completing — the blind mate's first `detected` — at
/// the sharer's first pass after the switch went off, with a control replay
/// (the sharer blinded) proving that sighting was the sharer's and not the
/// silent pod's own.
#[test]
fn o2_emcon_off_at_a_beat_clears_the_receiver_and_the_mesh_at_three_cadences() {
    let catalog = ecm_catalog();
    let script = o2_emcon_script();
    let first = q6_three_cadences("o2 emcon", 45_000, || o2_emcon_arena(&catalog), &script);
    let emcon = first.lines("emcon", &[]);
    assert_eq!(
        emcon,
        vec![
            (
                10_000,
                format!("{{\"kind\":\"emcon\",\"unit\":{},\"on\":false}}", POD.0)
            ),
            (
                30_000,
                format!("{{\"kind\":\"emcon\",\"unit\":{},\"on\":true}}", POD.0)
            ),
        ],
        "the switch is journalled at the beats' own milliseconds"
    );
    let receiver = first.lines("receiver", &[&format!("\"unit\":{}", POD.0)]);
    let degraded: Vec<(i64, bool)> = receiver
        .iter()
        .map(|(at, json)| (*at, json.contains("\"degraded\":true")))
        .collect();
    assert_eq!(
        degraded.len(),
        3,
        "the pod's own receiver: degraded, cleared once, degraded once: {receiver:?}"
    );
    let (Some((opened, true)), Some((cleared, false)), Some((again, true))) = (
        degraded.first().copied(),
        degraded.get(1).copied(),
        degraded.get(2).copied(),
    ) else {
        panic!("degraded, clear, degraded: {degraded:?}");
    };
    let pass_ms = i64::from(SensorsConfig::default().pass_ms);
    assert!(
        opened < pass_ms,
        "the first pass ran in the bubble: {opened}"
    );
    assert!(
        cleared > 10_000 && cleared <= 10_000 + pass_ms,
        "the receiver cleared at the pod's first pass after the off beat: {cleared}"
    );
    assert!(
        again > 30_000 && again <= 30_000 + pass_ms,
        "the receiver re-degraded at the first pass after the on beat: {again}"
    );
    // Only the pod's own sight stands in the bubble with a sensor: the sharer is
    // outside it and the mate is blind, so no other receiver moved.
    assert_eq!(
        first.lines("receiver", &[]).len(),
        3,
        "a receiver outside the pod moved"
    );
    // The mesh: the mate was told nothing while the pod radiated, and the
    // refused share completed once the switch went off.
    let told = first.lines("detected", &["\"viewer\":5", "\"target\":9"]);
    let Some((found_ms, _)) = told.first() else {
        panic!("the silent pod never let the share through: {told:?}");
    };
    // At the sharer's first pass after the off beat: the flip cleared the memo,
    // so the pass that follows it reads the silent pod at the mate's end, and a
    // share is applied in the substep of the pass that scheduled it.
    assert!(
        *found_ms > 10_000 && *found_ms <= 10_000 + pass_ms,
        "the mate was told at {found_ms}, not at the sharer's first pass after the \
         off beat (10000, {}]",
        10_000 + pass_ms
    );
    // And it was the *sharer's* sighting that arrived. With the sharer blinded the
    // mate is told nothing in the whole run, so nothing but the refused share
    // completing can have told it: the ridge keeps the silent pod's own sight off
    // the target, and a sighting travels one hop.
    let mut unshared = o2_emcon_arena(&catalog);
    if let Some(state) = unshared.unit_mut(RED) {
        state.sensor = None;
    }
    let control = q6_replay(&mut unshared, 1_000, 45_000, &script);
    assert!(
        control
            .lines("detected", &["\"viewer\":5", "\"target\":9"])
            .is_empty()
            && control
                .lines("detected", &["\"viewer\":7", "\"target\":9"])
                .is_empty(),
        "with the sharer blind, the mate or the pod still took the target, so the \
         mate's contact above is not the refused share completing"
    );
    assert!(
        first.perception.jam_checks > 0 && first.perception.share_ops > 0,
        "the switch was read by a scan: {:?}",
        first.perception
    );
    // And the pod's pilot was told both flips, with the pod's own radius.
    let pod_pilot = mush_core::ObjId(1_003);
    let flips: Vec<&FrameNotice> = first
        .notices
        .get(&pod_pilot)
        .map(|notices| {
            notices
                .iter()
                .filter(|notice| matches!(notice, FrameNotice::Emcon { .. }))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        flips,
        vec![
            &FrameNotice::Emcon {
                on: false,
                radius_m: 60
            },
            &FrameNotice::Emcon {
                on: true,
                radius_m: 60
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// D.2 — a cold gun is not hot (`B-16`)
// ---------------------------------------------------------------------------

/// Put the platform's heat cell on its ceiling, through one
/// [`mush_tactics::UnitMutationPlan`] — which is the only way a unit's cells move
/// (D-16-6) — so the fixture starts overheated with nothing in flight and no
/// ledger line ahead of the replay.
fn at_the_ceiling(arena: &mut Arena, unit: mush_core::ObjId) {
    let max = arena
        .unit(unit)
        .and_then(UnitState::heat_max)
        .unwrap_or_else(|| panic!("#{} carries no heat", unit.0));
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: heat_addr(HEAT_CELL),
            value: i64::from(max),
        }],
        None,
        Some(SmolStr::new("test:hot")),
    )
    .expect("a one-op plan");
    let hot = {
        let state = arena.unit(unit).unwrap_or_else(|| panic!("the unit"));
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = hot;
    }
    assert!(
        arena.unit(unit).is_some_and(UnitState::overheated),
        "the fixture is at its ceiling"
    );
}

/// What one cadence of the zero-heat replay produced: the cadence, the pilot's
/// own notice lines, the per-substep hashes and the ledger.
type D2Run = (i64, Vec<String>, Vec<u64>, Vec<String>);

/// **A cold gun on a cooked platform fires at every cadence, and the hot one is
/// refused at every cadence** (16b cut D.2, `B-16`; doc 16 §9 item 1).
///
/// One scripted log at 5,000 / 1,000 / 500 ms: the coaxial, whose
/// `heat_per_shot` is nought, is accepted and journals a `fired` line; the laser,
/// which makes twenty a pulse, is refused `overheated` and journals nothing. Both
/// answers are the same at all three, which is what makes the rule a fact of
/// arena state rather than of the pulse it was asked in.
#[test]
fn d2_a_zero_heat_weapon_fires_from_a_cooked_platform_at_three_cadences() {
    let mut runs: Vec<D2Run> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = duel(&[]);
        at_the_ceiling(&mut arena, RED);
        let mut bytes: Vec<u8> = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        let mut told: Vec<String> = Vec::new();
        let mut wall = 0_i64;
        let mut ordered = false;
        loop {
            arena
                .clock
                .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
            // One beat, at arena time 0, carrying both orders — delivered in a
            // pulse of no wall time, so phase 4 resolves both at exactly that
            // millisecond (`tests/resolution.rs`'s `replay` has the argument).
            // They go together because the fall is a rate over arena time: five
            // seconds later the platform has shed fifteen and is not at its
            // ceiling any more, which would make this a test of dissipation.
            let mut intents: Vec<tacsim::arena::intents::Intent> = Vec::new();
            if !ordered {
                ordered = true;
                intents.push(relief_intent(
                    RED,
                    tacsim::arena::intents::IntentKind::Fire {
                        weapon: SmolStr::new("machine_gun"),
                        slot: None,
                    },
                    2,
                ));
                intents.push(relief_intent(
                    RED,
                    tacsim::arena::intents::IntentKind::Fire {
                        weapon: SmolStr::new("laser"),
                        slot: None,
                    },
                    3,
                ));
            }
            let out = if intents.is_empty() {
                wall += cadence;
                arena.step(wall, &[], &[])
            } else {
                arena.step(wall, &[], &intents)
            };
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            hashes.extend(arena.substep_hashes.iter().copied());
            for (pilot, frame) in &out.frames {
                if *pilot == mush_core::ObjId(1_000) {
                    told.extend(tacsim::match_::cockpit::notice_lines(frame));
                }
            }
            if arena.now_ms >= 15_000 || out.ended.is_some() {
                break;
            }
        }
        arena.ledger.flush();
        for chunk in arena.ledger.take_chunks() {
            bytes.extend_from_slice(&chunk.bytes);
        }
        let events: Vec<String> = String::from_utf8(bytes)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).ok()
            })
            .map(|event| {
                format!(
                    "{} {}",
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default()
                )
            })
            .collect();
        runs.push((cadence, told, hashes, events));
    }
    let Some((_, first_told, first_hashes, first_events)) = runs.first() else {
        panic!("three replays");
    };
    // The coaxial fired: one `fired` line, and it names the coaxial.
    let fired: Vec<&String> = first_events
        .iter()
        .filter(|line| line.contains("machine_gun"))
        .collect();
    assert_eq!(fired.len(), 1, "{first_events:?}");
    assert!(
        !first_events.iter().any(|line| line.contains("laser")),
        "the laser was accepted from a cooked platform: {first_events:?}"
    );
    // And the laser's refusal reached the pilot, naming the weapon and the pair
    // of numbers (`B-16`'s one sentence for every `FIRE` refusal).
    assert_eq!(
        first_told
            .iter()
            .filter(|line| line.starts_with("FIRE laser refused. Overheated: 60 of 60."))
            .count(),
        1,
        "{first_told:?}"
    );
    assert!(
        !first_told
            .iter()
            .any(|line| line.starts_with("FIRE machine_gun refused")),
        "the cold gun was refused: {first_told:?}"
    );
    for (cadence, told, hashes, events) in runs.iter().skip(1) {
        assert_eq!(told, first_told, "the notices differ at {cadence} ms");
        assert_eq!(
            events, first_events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            hashes, first_hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
    }
}

/// [`duel`]'s laser tank **under the reflex controller** (16b cut D.2, `B-16`),
/// with no lock pre-taken: the machine stages its own, which is the whole of what
/// is being watched here.
fn reflex_duel() -> Arena {
    let catalog = catalog();
    let mut arena = arena_of(
        &catalog,
        vec![
            under_reflex(unit_of(
                &catalog,
                content::LINE_TANK_LASER,
                RED,
                Team::Red,
                RED_CELL,
            )),
            unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[],
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
    arena
}

/// **The zero-heat exemption in the machine's half** (16b cut D.2, `B-16`):
/// `would_overheat` is the controller's copy of the engine's question, so a
/// cooked platform's cold gun must be as available to the machine as it is to a
/// pilot — otherwise the two halves of one platform disagree about whether the
/// gun works.
///
/// The controller is **driven**, not asked: `would_overheat` is private to
/// `arena::reflex` and its only production caller is `engage_orders`, so the
/// claim is made over the ledger a cooked reflex-held laser tank writes. Every
/// `fired` line names the coaxial and none names the laser.
///
/// **16b cut E.2 (16c `T-55`) made the platform's heat have to be held there.**
/// The machine's question is now the engine's exactly — refuse AT the ceiling,
/// accept and clamp below it — so a tank one point below its ceiling is a tank
/// whose laser is legally available again, and a window over a hull that sheds
/// three a second was quietly becoming a test of dissipation and of which
/// substep the decide tick lands in. The fixture is held at the ceiling by
/// **being a fixture that cannot cool** (`heat.per_s` of nought), and the last
/// assertion is that it never left it — so "cooked" is a property of the tank
/// under test and not of the arithmetic of a 500 ms pulse.
#[test]
fn d2_the_reflex_controller_takes_the_cold_gun_from_a_cooked_platform() {
    // The engine's own answer, which is what the controller has to agree with.
    let mut engine = duel(&[]);
    at_the_ceiling(&mut engine, RED);
    assert_eq!(
        engine.resolve_fire(RED, "laser", engine.now_ms),
        Err(tacsim::arena::intents::Refusal::Overheated),
        "the hot gun is still refused at the ceiling"
    );
    engine
        .resolve_fire(RED, "machine_gun", engine.now_ms)
        .expect("the cold gun fires from a cooked platform");

    // And the machine's, over five seconds of arena time.
    let mut arena = reflex_duel();
    // A hull that cannot shed: `line_tank_laser` carries a `heat_sink` of three
    // a second, and three a second is one whole point part-way through every
    // 500 ms pulse — which under `T-55` is the difference between a laser the
    // machine may not fire and one it may. `HeatState` is read once, by
    // `Arena::new`, so this holds for the run.
    if let Some(heat) = arena.unit_mut(RED).and_then(|state| state.heat.as_mut()) {
        heat.per_s = 0;
    }
    at_the_ceiling(&mut arena, RED);
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let _ = run_logged(&mut arena, &mut wall, 500, 10, &mut bytes);
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let log = String::from_utf8(bytes).unwrap_or_default();
    let fired: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("\"fired\""))
        .collect();
    assert!(
        !fired.is_empty(),
        "the machine never pulled a trigger at all: {log}"
    );
    assert!(
        fired.iter().all(|line| line.contains("machine_gun")),
        "the machine reached for a gun that is not the coaxial: {fired:?}"
    );
    assert!(
        !fired.iter().any(|line| line.contains("laser")),
        "the machine fired the laser from a cooked platform: {fired:?}"
    );
    // And the platform was cooked for every one of them — it opened on its
    // ceiling, it cannot cool and the coaxial adds nothing, so the laser was
    // refused at the engine's own number at every decide tick in the window and
    // not merely at the one this fixture happened to start on. Without this the
    // three assertions above would pass over a tank that had quietly dropped a
    // point and simply never cycled its laser.
    assert_eq!(
        arena.unit(RED).and_then(UnitState::heat_now),
        arena.unit(RED).and_then(UnitState::heat_max),
        "the platform left its ceiling, so the run stopped being about a cooked one"
    );
    assert!(fired.len() >= 2, "one shot is not a habit: {fired:?}");
}

// ---------------------------------------------------------------------------
// 16b cut D.4 — a platform knows its own receiver is degraded (`B-22`)
//
// 16c section 5 Q17's default read literally: the platform knows its own
// receiver and nothing about who is jamming it or from where. The fold is the
// one `Arena::detections` already made for its two terms, carried out of the
// function that made it, so `jamming_at` still runs once a pass and no
// perception counter moves — which is asserted here against the very numbers
// X.4 and X.5 pinned.
// ---------------------------------------------------------------------------

/// One sensor pass, with the journal it wrote — [`thermal_pass`] for a property
/// about a ledger line rather than about a counter.
fn receiver_pass(
    arena: &mut Arena,
    observer: mush_core::ObjId,
    pass_seq: u64,
    t_ms: i64,
) -> (
    tacsim::arena::perception::PerceptionCounters,
    Vec<tacsim::arena::perception::Notice>,
) {
    let mut counters = tacsim::arena::perception::PerceptionCounters::default();
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    let units = arena.units.clone();
    arena.index.rebuild(&arena.definition.terrain, &units);
    // 16b cut U.2: and the jamming memo with it, exactly as phase 3(b) empties
    // it beside its own rebuild — a pass driven directly reads through the memo.
    arena.jam_fold.clear();
    arena.sensor_pass(observer, pass_seq, t_ms, &mut counters, &mut journal);
    (counters, journal)
}

/// Every `receiver` line one pass journalled, as `(t_ms, degraded)`.
fn receiver_lines(journal: &[tacsim::arena::perception::Notice]) -> Vec<(i64, bool)> {
    journal
        .iter()
        .filter_map(|(t_ms, kind, _)| match kind {
            tacsim::arena::ledger::EventKind::Receiver { degraded, .. } => Some((*t_ms, *degraded)),
            _ => None,
        })
        .collect()
}

/// What the observer's own receiver reads after a pass.
fn receiver_of(
    arena: &Arena,
    unit: mush_core::ObjId,
) -> Option<tacsim::arena::perception::Receiver> {
    arena.unit(unit).and_then(|state| state.receiver)
}

/// Put a unit in a cell, the way `tests/perception.rs`' own `place` does.
fn put(arena: &mut Arena, unit: mush_core::ObjId, cell: (u16, u16)) {
    if let Some(state) = arena.unit_mut(unit) {
        state.pose = pose(cell);
    }
}

/// What one pilot was told about their receiver over `pulses` half-second pulses,
/// and what the **same frame** carried in [`tacsim::arena::step::Frame::receiver`]
/// (16b cut D.4).
fn drive_receiver(
    arena: &mut Arena,
    wall: &mut i64,
    pulses: u32,
    told: &mut Vec<(bool, Option<tacsim::arena::perception::Receiver>)>,
) {
    for _ in 0..pulses {
        *wall += 500;
        let out = arena.step(*wall, &[], &[]);
        for (seat, frame) in &out.frames {
            if *seat != mush_core::ObjId(1_000) {
                continue;
            }
            for notice in &frame.notices {
                if let FrameNotice::Receiver { degraded } = notice {
                    told.push((*degraded, frame.receiver));
                }
            }
        }
    }
}

/// **The pilot is told, on a frame that carries what he is being told about**
/// (16b cut D.4, `B-22`), off a **running arena** and not a hand-built `Frame`.
///
/// Two seams meet here and neither has another test that drives it: `Arena::step`
/// projects `EventKind::Receiver` into `FrameNotice::Receiver` for the platform
/// that crossed the edge (`Arena::stage_pilot_notice`), and `Arena::frames` fills
/// `Frame::receiver` from `UnitState::receiver`. Everything else that mentions a
/// receiver reads the pass's journal or builds the frame by hand, so without this
/// the whole pilot-facing half of `B-22` could be deleted and stay green.
///
/// Half-second pulses, so no one pulse can hold both crossings: the frame that
/// says `Receiver degraded` carries the numbers, and the frame that says
/// `Receiver clear.` carries `None`.
#[test]
fn d4_the_frame_that_tells_the_pilot_carries_the_receiver_it_is_telling_him_about() {
    let catalog = ecm_catalog();
    let mut arena = jam_field(
        &catalog,
        content::LINE_TANK,
        OUTSIDE_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let mut wall = 0_i64;
    let mut told: Vec<(bool, Option<tacsim::arena::perception::Receiver>)> = Vec::new();

    // Outside the bubble: nothing to say.
    drive_receiver(&mut arena, &mut wall, 4, &mut told);
    assert!(told.is_empty(), "a clean drive told the pilot something");

    // In: one notice, on a frame carrying the fold's own numbers.
    put(&mut arena, RED, WATCH_CELL);
    drive_receiver(&mut arena, &mut wall, 4, &mut told);
    assert_eq!(
        told,
        vec![(
            true,
            Some(tacsim::arena::perception::Receiver {
                budget: 40 - JAM_ATTENUATION_COST,
                budget_max: 40,
                reach_permille: u16::try_from(JAM_SIGNATURE_PERMILLE).unwrap_or(0),
            })
        )],
        "the degraded frame did not carry the platform's own receiver"
    );
    assert_eq!(
        told.first().and_then(|(_, receiver)| *receiver),
        receiver_of(&arena, RED),
        "the frame's receiver is not the unit's"
    );

    // And out: one notice the other way, on a frame carrying nothing.
    put(&mut arena, RED, OUTSIDE_CELL);
    drive_receiver(&mut arena, &mut wall, 4, &mut told);
    assert_eq!(
        told.get(1)
            .map(|(degraded, receiver)| (*degraded, *receiver)),
        Some((false, None)),
        "leaving the bubble: {told:?}"
    );
    assert_eq!(
        told.len(),
        2,
        "the edge was crossed more than twice: {told:?}"
    );
    assert_eq!(receiver_of(&arena, RED), None);
}

#[test]
fn d4_the_edge_of_a_bubble_is_one_line_in_and_one_line_out() {
    let catalog = ecm_catalog();
    // The observer starts outside the pod's reach, at [`OUTSIDE_CELL`].
    let mut arena = jam_field(
        &catalog,
        content::LINE_TANK,
        OUTSIDE_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let (_, clean) = receiver_pass(&mut arena, RED, 1, 0);
    assert!(
        receiver_lines(&clean).is_empty(),
        "a clean pass journalled a receiver line"
    );
    assert_eq!(receiver_of(&arena, RED), None, "an unjammed platform");

    // Into the bubble: one line, and the numbers are the fold's own.
    put(&mut arena, RED, WATCH_CELL);
    let (_, entering) = receiver_pass(&mut arena, RED, 2, 1_000);
    assert_eq!(
        receiver_lines(&entering),
        vec![(1_000, true)],
        "the transition is one line at the pass's own timestamp"
    );
    assert_eq!(
        receiver_of(&arena, RED),
        Some(tacsim::arena::perception::Receiver {
            // 40 of the Mk1 sight's budget less the pod's 25.
            budget: 40 - JAM_ATTENUATION_COST,
            budget_max: 40,
            reach_permille: u16::try_from(JAM_SIGNATURE_PERMILLE).unwrap_or(0),
        })
    );

    // Deeper into the same bubble: the numbers do not move, so nothing is said.
    put(&mut arena, RED, NEAR_CELL);
    let (_, inside) = receiver_pass(&mut arena, RED, 3, 2_000);
    assert!(
        receiver_lines(&inside).is_empty(),
        "moving inside one bubble said something"
    );

    // And out again: one line the other way.
    put(&mut arena, RED, OUTSIDE_CELL);
    let (_, leaving) = receiver_pass(&mut arena, RED, 4, 3_000);
    assert_eq!(receiver_lines(&leaving), vec![(3_000, false)]);
    assert_eq!(receiver_of(&arena, RED), None);
}

#[test]
fn d4_two_stacked_pods_move_the_numbers_and_say_nothing() {
    let catalog = ecm_catalog();
    let mut arena = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let (one_pod, first) = receiver_pass(&mut arena, RED, 1, 0);
    assert_eq!(receiver_lines(&first), vec![(0, true)]);
    let one = receiver_of(&arena, RED).unwrap_or_else(|| panic!("one pod is a fold"));

    // The second pod is the target's own hull, parked in the cell beside the
    // first — the spatial index holds one occupant a cell, so two emitters have
    // to stand in two of them. The costs sum and the factors multiply, so the
    // observer reads deeper.
    let pod = arena.unit(POD).and_then(|pod| pod.jammer);
    if let Some(state) = arena.unit_mut(BLUE) {
        state.jammer = pod;
        state.power = Power::Online;
    }
    put(&mut arena, BLUE, (POD_CELL.0 - 1, POD_CELL.1));
    let (two_pods, second) = receiver_pass(&mut arena, RED, 2, 1_000);
    assert!(
        receiver_lines(&second).is_empty(),
        "a change of degree inside a bubble is not a transition"
    );
    let two = receiver_of(&arena, RED).unwrap_or_else(|| panic!("two pods are a fold"));
    assert_ne!(
        two, one,
        "two stacked pods read the same as one: the numbers are a monotone read of \
         the fold and this is the declared limit under 16c section 5 Q17"
    );
    assert!(two.budget < one.budget && two.reach_permille < one.reach_permille);
    assert_eq!(
        two.budget_max, one.budget_max,
        "the sight itself did not move"
    );
    // The scan looked at more units, because there are more of them in reach —
    // and it still ran exactly once a pass.
    assert!(two_pods.jam_checks >= one_pod.jam_checks);
}

#[test]
fn d4_reading_the_receiver_moved_no_perception_counter() {
    // The very numbers X.4 and X.5 pinned, re-asserted here: the fold is carried
    // out of `Arena::detections` rather than made a second time, so `jamming_at`
    // still runs once a pass and `jam_checks` is what it was.
    let catalog = ecm_catalog();
    let belt = [(8_u16, 4_u16, woods())];
    let mut arena = jam_field(
        &catalog,
        content::LINE_TANK_ECM,
        WATCH_CELL,
        NEAR_CELL,
        None,
        &belt,
    );
    let (counters, _) = receiver_pass(&mut arena, RED, 1, 0);
    assert_eq!(
        counters.jam_checks, 2,
        "X.4 pinned two: the observer inside its own bubble and the target four cells out"
    );
    assert_eq!(counters.candidates, 1);
    assert_eq!(counters.traces, 1);

    // And a field with no pod on it never scans at all, receiver or no receiver.
    let mut stock = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        MARK_CELL,
        None,
        &[],
    );
    let (bare, journal) = receiver_pass(&mut stock, RED, 1, 0);
    assert_eq!(bare.jam_checks, 0);
    assert!(receiver_lines(&journal).is_empty());
    assert_eq!(receiver_of(&stock, RED), None);
}

#[test]
fn d4_a_filter_that_answers_the_whole_bubble_leaves_the_receiver_clean() {
    let catalog = pod_and_filter_catalog();
    // A partly recovering filter: the shipped Mk1 filter against the shipped Mk1
    // pod, which recovers some of both terms and not all of either.
    let mut partly = jam_field(
        &catalog,
        content::LINE_TANK_ECCM,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let (_, journal) = receiver_pass(&mut partly, RED, 1, 0);
    assert_eq!(receiver_lines(&journal), vec![(0, true)]);
    let recovered = receiver_of(&partly, RED).unwrap_or_else(|| panic!("a partial recovery"));
    let filter = authored_filter();
    assert_eq!(
        recovered,
        tacsim::arena::perception::Receiver {
            budget: 40_u8
                .saturating_sub(JAM_ATTENUATION_COST.saturating_sub(filter.attenuation_recovery)),
            budget_max: 40,
            reach_permille: u16::try_from(
                JAM_SIGNATURE_PERMILLE.saturating_add(filter.signature_recovery_permille)
            )
            .unwrap_or(0),
        },
        "the receiver reads the RECOVERED fold and not the raw one"
    );

    // A filter rated past the whole bubble: the pass is the unjammed pass to the
    // centimetre, so the receiver is not degraded and nothing is said.
    let mut whole = jam_field(
        &catalog,
        content::LINE_TANK_ECCM,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    if let Some(state) = whole.unit_mut(RED) {
        state.eccm = Some(tacsim::arena::perception::Eccm {
            attenuation_recovery: u8::MAX,
            signature_recovery_permille: tacsim::arena::perception::FULL_SIGNATURE_PERMILLE,
        });
    }
    let (_, silent) = receiver_pass(&mut whole, RED, 1, 0);
    assert!(
        receiver_lines(&silent).is_empty(),
        "a platform that answered the whole bubble was told it was jammed"
    );
    assert_eq!(receiver_of(&whole, RED), None);
}

#[test]
fn d4_a_pass_that_does_not_run_turns_the_receiver_off_in_silence() {
    let catalog = ecm_catalog();
    let mut arena = jam_field(
        &catalog,
        content::LINE_TANK,
        WATCH_CELL,
        MARK_CELL,
        Some(POD_CELL),
        &[],
    );
    let (_, first) = receiver_pass(&mut arena, RED, 1, 0);
    assert_eq!(receiver_lines(&first), vec![(0, true)]);
    assert!(receiver_of(&arena, RED).is_some());

    // The tank is destroyed while degraded, **through the engine's own
    // `destroy_unit`** and not by setting the flag by hand: that is the call the
    // arena makes, and it is the call that takes this unit's sensor pass off the
    // due heap for good. Nothing is ever going to run a pass for this hull again,
    // so the receiver has to go off where the sight does.
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.destroy_unit(RED, 500, None, &mut journal);
    assert!(
        receiver_lines(&journal).is_empty(),
        "a wreck was told its receiver was clear"
    );
    assert_eq!(
        receiver_of(&arena, RED),
        None,
        "the wreck kept the reading its last live pass took"
    );
    assert!(
        !arena.due.iter().any(|event| matches!(
            event.kind,
            tacsim::arena::state::DueKind::SensorPass { unit, .. } if unit == RED
        )),
        "the wreck still has a pass scheduled, so this property is about the wrong call"
    );

    // And the pass that does not run — this one cannot be scheduled any more, so
    // it is driven by hand — writes `None` in the same silence.
    let (_, wrecked) = receiver_pass(&mut arena, RED, 2, 1_000);
    assert!(
        receiver_lines(&wrecked).is_empty(),
        "a wreck was told its receiver was clear"
    );
    assert_eq!(
        receiver_of(&arena, RED),
        None,
        "the receiver is off, and the state moved silently"
    );
}

/// **The receiver's two lines are one list at three cadences** (16b section 11's
/// mandatory cross-tempo test for 16b cut D.4, `B-22`; doc 16 section 3.1 "Rate
/// independence").
///
/// The same seated tank driving through X.4's bubble and out the far side, at
/// 5,000, 1,000 and 500 ms. Both of the things this cut added to that drive are
/// functions of where the tank stood when its pass ran: the `receiver` ledger
/// line, and `UnitState::receiver` itself, which is folded into the state hash —
/// so a cadence that ran the pass half a pulse later would journal the
/// transition at a different arena millisecond and diverge on the hash a substep
/// after that. [`q6_three_cadences`] compares both.
#[test]
fn d4_one_scripted_drive_through_a_bubble_journals_the_receiver_at_three_cadences() {
    let catalog = ecm_catalog();
    let script = q6_bubble_script();
    let first = q6_three_cadences(
        "d4 receiver",
        60_000,
        || q6_bubble_arena(&catalog, content::LINE_TANK),
        &script,
    );

    let lines = first.lines("receiver", &["\"unit\":4"]);
    assert_eq!(
        lines.len(),
        2,
        "the drive did not cross the edge exactly twice: {lines:?}"
    );
    let (Some((into_ms, into)), Some((out_ms, out))) = (lines.first(), lines.get(1)) else {
        panic!("two crossings");
    };
    assert!(
        into.contains("\"degraded\":true"),
        "the first crossing is into the bubble: {into}"
    );
    assert!(
        out.contains("\"degraded\":false"),
        "and the second is out of it: {out}"
    );
    assert!(into_ms < out_ms, "in at {into_ms}, out at {out_ms}");
    // Nothing on the line places the pod or names it: the record may carry the
    // platform, and the pilot's own notice carries neither.
    for (_, json) in &lines {
        for leak in ["\"x\"", "\"y\"", "cell", "bearing", "team", "\"target\""] {
            assert!(
                !json.contains(leak),
                "a receiver line carried {leak}: {json}"
            );
        }
    }

    // **And the pilot's half of `B-22`**, which the ledger cannot speak for: the
    // seated pilot of the tank that made the drive was staged the two notices, in
    // that order and once each. `q6_three_cadences` has already compared this
    // very list across the three cadences, so the assertion below holds at all
    // three.
    let pilot = mush_core::ObjId(1_000);
    let staged = first
        .notices
        .get(&pilot)
        .cloned()
        .unwrap_or_else(|| panic!("the seated pilot was staged nothing at all"));
    let receiver_staged: Vec<FrameNotice> = staged
        .iter()
        .filter(|notice| matches!(notice, FrameNotice::Receiver { .. }))
        .cloned()
        .collect();
    assert_eq!(
        receiver_staged,
        vec![
            FrameNotice::Receiver { degraded: true },
            FrameNotice::Receiver { degraded: false }
        ],
        "the pilot's own receiver notices: {staged:?}"
    );
    // Nobody else's pilot was staged one: the notice is the platform's own and
    // goes to its own seat alone.
    for (seat, theirs) in &first.notices {
        assert!(
            *seat == pilot
                || !theirs
                    .iter()
                    .any(|notice| matches!(notice, FrameNotice::Receiver { .. })),
            "pilot {seat:?} was told about somebody else's receiver: {theirs:?}"
        );
    }
    // And what the pilot actually read for them, in order: the two sentences,
    // off a running arena and not a hand-built notice.
    let said = first
        .lines
        .get(&pilot)
        .cloned()
        .unwrap_or_else(|| panic!("the seated pilot read nothing at all"));
    let receiver_said: Vec<&str> = said
        .iter()
        .map(String::as_str)
        .filter(|line| *line == cockpit::RECEIVER_DEGRADED || *line == cockpit::RECEIVER_CLEAR)
        .collect();
    assert_eq!(
        receiver_said,
        vec![cockpit::RECEIVER_DEGRADED, cockpit::RECEIVER_CLEAR],
        "the pilot's own receiver lines: {said:?}"
    );
    // The sentence itself carries no source: no side, no cell, no bearing, no
    // object number and no designator.
    for line in receiver_said {
        for leak in ["red", "blue", "bearing", "cell", "#", "X1", "BA"] {
            assert!(
                !line.contains(leak),
                "the receiver sentence carried {leak}: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 16b cut E.3 (16c `B-44`) — a routeless machine is checked every substep
// ---------------------------------------------------------------------------

/// **A routeless machine no longer drives at a bluff between its decide ticks**
/// (16b cut E.3; 16c `B-44`).
///
/// 16b cut Q.7 put the crossing's own question into `follow`: a machine steering
/// at a goal with no route is steering at a *direction* nobody has checked the
/// ground along, so the one cell it would cross first is tested and a machine
/// that would be refused it is ordered to stop. But `follow` runs at the decide
/// tick — once a second of arena time under the shipped doctrine — and motion
/// runs four times in that second at the substep ceiling, so for up to a whole
/// tick the machine drove at the cliff with nothing between it and the refusal.
/// 16c's `B-44` row counted five `blocked` lines on exactly this window.
///
/// The fixture is `x6_a_refused_route_is_bounded_work`'s worst case: the sealed
/// plateau, one node behind the cliff, a limited tank with nowhere else to be, so
/// the machine's goal is a cell no route reaches and it steers at the seam for
/// the whole run. Over thirty seconds of arena time it is refused nothing at all,
/// against the five `blocked` lines 16c's row counted.
///
/// **What this window really measured, corrected by 16b cut M.2.** E.3 wrote
/// here that the pre-pass fired zero times because `follow`'s tick-time check
/// and E.3's braking profile caught every approach. Neither did: the goal is past
/// the cliff, so the braking profile never slows the tank for the seam, and a
/// one-cell check cannot stop a tank whose stopping distance is five and a half
/// cells. The window simply closed five hundred milliseconds before the first
/// approach arrived at the seam at fifteen metres a second. M.2's machine
/// `STARTUP` at its own tick moved that approach one decide tick earlier and
/// this test went red — with one `blocked` line at `(20, 3)` — which is what
/// exposed it. The pre-pass now looks as far ahead as the tank needs to stop
/// ([`Arena::hold_routeless`]), and it is the pre-pass that holds this tank:
/// deleting it turns this test red again. The whole approach, to a standstill,
/// is `m2_a_routeless_machine_stops_short_of_the_seam_it_is_steering_at`.
#[test]
fn e3_a_routeless_machine_is_held_off_the_cliff_every_substep() {
    let catalog = catalog();
    let cfg = ArenaConfig::default();
    let pulses = usize::try_from(30_000_i64 / 500).unwrap_or(0);
    let mut definition = plateau(false);
    definition.waypoints = relief_waypoint((40, 2));
    let mut arena = arena_over(
        &catalog,
        vec![under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (2, 4),
        )))],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run_logged(&mut arena, &mut wall, 500, pulses, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    let stops = blocked_cells(&events, RED);
    println!(
        "e3_routeless: blocked={} intents={} ticks={} path_failures={}",
        stops.len(),
        counters.intents,
        counters.ticks,
        counters.path_failures
    );
    assert!(
        stops.is_empty(),
        "a routeless machine was refused a cell it should never have driven at: {stops:?}"
    );
    // The tank never got up onto the plateau either, which is the other half of
    // the claim: it was held off the seam, not carried over it.
    let (cell, z, _) = standing(&arena, RED);
    assert!(
        cell.0 < 20 && z == 0,
        "the tank got onto a plateau nothing can climb: {cell:?} at {z} cm"
    );
    // What is asserted is that the controller ran at all, so that
    // `stops.is_empty()` is a fact about a machine that was being driven and not
    // about one that never moved. (E.3 left out "`ReflexCounters::intents` shows
    // the holds" because it measured none; the holds are real — see the header —
    // and the standstill they end in is asserted by the M.2 test below.)
    assert!(
        counters.ticks > 0,
        "the controller never ran, so nothing was measured"
    );
}

/// **A routeless machine at full speed stops short of the seam it steers at**
/// (16b cut M.2, reopening E.3's `B-44`; the M.2 review's finding).
///
/// The E.3 fixture run on past its first approach, to a standstill. The tank
/// reaches fifteen metres a second on open ground steering at a goal behind the
/// cliff; braking from there at two metres a second squared takes fifty-six
/// metres, five and a half cells, so a pre-pass that asked only the cell in front
/// ordered the stop with the seam already inside the stopping distance and the
/// tank was refused at `(20, 3)` anyway. Looking as far ahead as it needs to
/// stop, it is held early, brakes on its own profile, and comes to rest in the
/// last cell before the cliff with no `blocked` line — and stays there, because
/// the next decide tick orders it on only while the cell in front is open.
///
/// Without the look-ahead this is red: the refusal at `(20, 3)` at 29,500.
#[test]
fn m2_a_routeless_machine_stops_short_of_the_seam_it_is_steering_at() {
    let catalog = catalog();
    let cfg = ArenaConfig::default();
    let mut definition = plateau(false);
    definition.waypoints = relief_waypoint((40, 2));
    let mut arena = arena_over(
        &catalog,
        vec![under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (2, 4),
        )))],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    // Forty-five seconds: the tank is on the move from ten, flat out by
    // twenty-five, and at rest by thirty-four.
    let pulses = usize::try_from(45_000_i64 / 500).unwrap_or(0);
    let mut fastest = 0_i32;
    for _ in 0..pulses {
        run_logged(&mut arena, &mut wall, 500, 1, &mut bytes);
        fastest = fastest.max(arena.unit(RED).map_or(0, |unit| unit.speed_cms));
    }
    let top = arena
        .unit(RED)
        .map_or(0, |unit| unit.mobility.max_speed_cms);
    assert_eq!(
        u32::try_from(fastest).unwrap_or(0),
        top,
        "the fixture needs the tank at full speed on the approach"
    );
    let events = ledger_of(&mut arena, &mut bytes);
    let stops = blocked_cells(&events, RED);
    assert!(
        stops.is_empty(),
        "the tank outran its brakes into the seam: {stops:?}"
    );
    let (cell, z, _) = standing(&arena, RED);
    assert_eq!(
        (cell.0, z),
        (19, 0),
        "the tank stopped in the last cell before the cliff: {cell:?} at {z} cm"
    );
    assert_eq!(
        arena.unit(RED).map(|unit| unit.speed_cms),
        Some(0),
        "and it is standing there"
    );
}

// ---------------------------------------------------------------------------
// 16b cut M.2 (16c `B-51`) — a pilot's beam lands in the pulse that fired it
// ---------------------------------------------------------------------------

/// What one cadence of the beam replay produced: the ledger lines as
/// `(t_ms, kind json)`, the per-substep hashes, and for the pulse that fired,
/// `(substeps run, substeps the partition cut)`.
type M2BeamRun = (Vec<(i64, String)>, Vec<u64>, Option<(u32, u32)>);

/// [`duel`] through **a pulse that ends on the beat**: the order rides the pulse
/// whose end is the beat's millisecond, which is the production shape — a
/// pilot's order resolves at the end of the pulse that accepted it. (The
/// zero-`dt` driver the other replays use cannot show the drain: a beat's pulse
/// there runs no substep, so the drain's `ran > 0` guard holds it back and the
/// impact pops in the next pulse's first substep, as the opening pulse's events
/// do.)
///
/// The pulse carrying the order is 4,500 → 5,000 at every cadence — the wall
/// is stepped to 4,500 first whatever the cadence — because phase 2 counts a
/// seat's order against the arena second the pulse **starts** in (16b cut M.1's
/// rate, which the hash folds), so an order delivered to 0 → 5,000 at one
/// cadence and to 4,500 → 5,000 at another would be two different states for
/// a reason that is the driver's and not the engine's.
fn m2_beam_replay(cadence_ms: i64) -> M2BeamRun {
    let mut arena = duel(&[]);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut firing: Option<(u32, u32)> = None;
    let mut wall = 0_i64;
    arena
        .clock
        .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
    let out = arena.step(wall, &[], &[]);
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
    while arena.now_ms < 10_000 {
        arena
            .clock
            .pin_tempo_for_tests(tacsim::arena::clock::Tempo::Quiet);
        wall = if wall < 4_500 {
            (wall + cadence_ms).min(4_500)
        } else if wall < 5_000 {
            5_000
        } else {
            wall + cadence_ms
        };
        let intents = if wall == 5_000 {
            vec![relief_intent(
                RED,
                tacsim::arena::intents::IntentKind::Fire {
                    weapon: SmolStr::new("laser"),
                    slot: None,
                },
                2,
            )]
        } else {
            Vec::new()
        };
        let out = arena.step(wall, &[], &intents);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        if !intents.is_empty() {
            // 4,500 → 5,000: two substeps of 250 at every cadence.
            let partition = 2;
            firing = Some((out.substeps, partition));
        }
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let events = String::from_utf8(bytes)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).ok())
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
            )
        })
        .collect();
    (events, hashes, firing)
}

/// **A pilot's beam lands in the pulse that fired it** (16b cut M.2, 16c
/// `B-51`). Phase 4 schedules a beam's impact at `now_ms` and drains nothing, so
/// the round used to resolve in the next pulse's first substep, against later
/// positions under the fire timestamp. The phase-4 drain — one zero-length
/// substep — resolves it in the pulse that fired: the `impact` and its `hit` or
/// `miss` carry the `fired` line's millisecond, that pulse ran one substep more
/// than its partition, and the three cadences are one event list and one hash
/// sequence.
#[test]
fn m2_a_pilots_beam_lands_in_the_pulse_that_fired_it_at_three_cadences() {
    let runs: Vec<(i64, M2BeamRun)> = [5_000_i64, 1_000, 500]
        .into_iter()
        .map(|cadence| (cadence, m2_beam_replay(cadence)))
        .collect();
    let Some((_, (events, hashes, _))) = runs.first() else {
        panic!("three replays");
    };
    let at = |kind: &str| -> Vec<i64> {
        let head = format!("{{\"kind\":\"{kind}\",");
        events
            .iter()
            .filter(|(_, json)| json.starts_with(&head))
            .map(|(t_ms, _)| *t_ms)
            .collect()
    };
    assert_eq!(at("fired"), vec![5_000], "one laser pulse, at the beat");
    assert_eq!(
        at("impact"),
        vec![5_000],
        "the beam arrived as it was fired"
    );
    let outcome: Vec<i64> = at("hit").into_iter().chain(at("miss")).collect();
    assert_eq!(outcome, vec![5_000], "and was resolved then");
    for (cadence, (_, _, firing)) in &runs {
        let (ran, partition) = firing.unwrap_or_else(|| panic!("{cadence}: no firing pulse"));
        assert_eq!(
            ran,
            partition + 1,
            "at {cadence} ms the firing pulse ran its {partition} substeps and the drain"
        );
    }
    for (cadence, (run_events, run_hashes, _)) in runs.iter().skip(1) {
        assert_eq!(run_events, events, "the event list differs at {cadence} ms");
        assert_eq!(
            run_hashes, hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut M.4: the sight the gunnery term reads, and a pod that is gone
// ---------------------------------------------------------------------------

/// `unit`'s `component` with its `condition` cell at `value`, through one plan
/// on the record — the way a hit dims a component, applied before check-out.
fn with_condition(mut unit: CustodyUnit, component: &str, value: i64) -> CustodyUnit {
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new(component).expect("a component id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::CONDITION_CELL)
            .expect("a cell id"),
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell { addr, value }],
        None,
        None,
    )
    .expect("a one-op plan");
    let (post, _) = mush_tactics::apply(&unit.unit, &plan, 0).expect("the cell takes the value");
    unit.unit = post;
    unit.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    unit
}

/// **16b cut M.4 (16c `B-53`): the gunnery term reads the sight, not the pod.**
///
/// The ECM tank carries two components with a `condition` cell, and `ecm_pod`
/// sorts before `visual_sensor`, so before the cut the term read the pod: a
/// shot-off pod widened the gun's cone and a shot-off sight did not. Now the
/// check-out names the sight (`SensorProfile::component`) and the term reads it:
/// with the pod at nought and the sight whole the cone is the stock tank's to
/// the microradian, and with the sight at nought it is wider by the
/// `sensor_condition` weight.
#[test]
fn m4_the_ecm_tanks_gunnery_term_reads_its_sight_and_not_its_pod() {
    let catalog = ecm_catalog();
    let cone = |shooter: CustodyUnit| {
        let mut arena = arena_of(
            &catalog,
            vec![
                shooter,
                unit_of(&catalog, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
            ],
            &[],
        );
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
        }
        arena
            .resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None)
            .error_urad
    };
    let stock = cone(unit_of(
        &catalog,
        content::LINE_TANK,
        RED,
        Team::Red,
        RED_CELL,
    ));
    let ecm = || unit_of(&catalog, content::LINE_TANK_ECM, RED, Team::Red, RED_CELL);
    assert_eq!(
        ecm().sensor.map(|sensor| sensor.component),
        Some(SmolStr::new("visual_sensor")),
        "the check-out names the sight"
    );
    assert_eq!(cone(ecm()), stock, "a whole ECM tank aims as a stock one");
    assert_eq!(
        cone(with_condition(ecm(), "ecm_pod", 0)),
        stock,
        "a shot-off pod does not touch the gun's cone"
    );
    let resolution = tacsim::match_::resolution_for(&catalog);
    let widened = u32::try_from(
        u64::from(resolution.error_terms.sensor_condition)
            * u64::from(tacsim::arena::resolution::TERM_URAD)
            / 1_000,
    )
    .unwrap_or(0);
    assert!(widened > 0, "the shipped weight is not nought");
    assert_eq!(
        cone(with_condition(ecm(), "visual_sensor", 0)),
        stock + widened,
        "a shot-off sight widens it by the sensor_condition term"
    );
}

/// **16b cut M.4 (16c `B-54`): a shot-off pod is no pod.** `Jammer::read`
/// takes the guard `Eccm::read` always had: the pod's own `condition` at nought
/// reads `None`, at one it still radiates. Read at check-out only (the named
/// limit in both docs).
#[test]
fn m4_a_pod_at_condition_nought_reads_as_no_pod_and_at_one_still_jams() {
    use tacsim::arena::perception::Jammer;
    let catalog = ecm_catalog();
    let pod = || unit_of(&catalog, content::LINE_TANK_ECM, POD, Team::Blue, POD_CELL);
    let whole = Jammer::read(&pod().unit, CELL_CM);
    assert!(whole.is_some(), "the shipped pod radiates");
    assert_eq!(
        Jammer::read(&with_condition(pod(), "ecm_pod", 0).unit, CELL_CM),
        None,
        "a pod shot off its mount radiates nothing"
    );
    assert_eq!(
        Jammer::read(&with_condition(pod(), "ecm_pod", 1).unit, CELL_CM),
        whole,
        "a pod hanging on by one point of condition still jams, whole"
    );
    // And through the check-out: the arena opens with no pod and no bubble.
    let arena = arena_of(
        &catalog,
        vec![
            unit_of(&catalog, content::LINE_TANK, RED, Team::Red, RED_CELL),
            with_condition(pod(), "ecm_pod", 0),
        ],
        &[],
    );
    assert_eq!(arena.unit(POD).and_then(|state| state.jammer), None);
    assert_eq!(arena.jam_radius_cm, 0, "no pod, no bubble to query");
}

// ---------------------------------------------------------------------------
// O.3 — the range's target (16c N13)
// ---------------------------------------------------------------------------

/// **The range's target binds, is made, is refitted to and from, and checks out
/// and back in as the thing it says it is** (16b cut O.3; 16c N13).
///
/// `tacsim.unit.target_mk1` is the fielded catalog's sixth table
/// (`content::EXTENSIONS_TARGET`): the stock hull with the power role alone, no
/// weapon, no sensor and a zero speed. Everything the range needs of it is
/// asserted here on the production routes — the pin, `Catalog::build_fielded`,
/// `mush_tactics::module::create_unit` and `refit_unit` in the production
/// composition, `custody_unit` and `Arena::new` on the stand-in window, and the
/// check-in — so the lobby's refit at `SIM START` and the arena's elimination
/// rule have nothing to discover about it.
#[test]
fn o3_the_target_binds_creates_refits_and_checks_out() {
    if writing() {
        for relative in PACK_FILES_TARGET {
            resign(relative);
        }
        return;
    }

    // The pin, and the pack requires nothing.
    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_TARGET.into_iter().collect();
    for (name, json) in content::EXTENSIONS_TARGET {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        assert!(
            pack.manifest().requires().is_empty(),
            "a target mounts nothing, so it requires no pack"
        );
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_TARGET.len());

    // Its own table adds one unit schema and nothing else.
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let alone = catalog_of(&content::extension_sources_of(&content::EXTENSIONS_TARGET));
    assert_eq!(alone.specs.len(), stock.specs.len() + 1, "one unit schema");
    assert_eq!(alone.weapons.len(), stock.weapons.len(), "and no weapon");
    assert_eq!(alone.sensors.len(), stock.sensors.len(), "and no sensor");
    assert_eq!(alone.arenas.len(), stock.arenas.len(), "and no arena");

    // It binds through the fielded route every production site builds from.
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let target_key = CanonicalId::new(content::TARGET_MK1).expect("a canonical id");
    let tank_key = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    let spec = fielded
        .bindings
        .get(&target_key)
        .map(|binding| binding.spec.clone())
        .expect("the target is in the fielded catalog");
    let tank_spec = fielded
        .bindings
        .get(&tank_key)
        .map(|binding| binding.spec.clone())
        .expect("the stock tank is in the fielded catalog");

    // `create_unit` of a fresh object, in the production composition: the unit
    // carries the power component alone, and its document names no weapon.
    let mut tw = mush_testkit::TestWorld::with_modules(x6_production_roster());
    let target = tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Range Target",
            mush_core::GOD,
            Some(mush_core::ObjId(0)),
        )
        .expect("a thing");
    let hull = tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Spare Hull",
            mush_core::GOD,
            Some(mush_core::ObjId(0)),
        )
        .expect("a thing");
    // Every call answers the unit and the text of what it queued to persist.
    let mut act = |obj: mush_core::ObjId,
                   spec: &mush_tactics::UnitSpec,
                   refit: bool|
     -> (mush_tactics::TacticalUnit, String) {
        let mut out = mush_core::Outbox::new();
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: mush_core::GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let made = if refit {
            mush_tactics::module::refit_unit(&mut ctx, obj, spec)
        } else {
            mush_tactics::module::create_unit(&mut ctx, obj, spec)
        }
        .unwrap_or_else(|error| panic!("#{} refused: {error}", obj.0));
        let written = format!("{:?}", out.take_deferred());
        (made, written)
    };
    let (made, document) = act(target, &spec, false);
    let components: Vec<String> = made.components().map(|(id, _)| id.to_string()).collect();
    assert_eq!(components, ["power"], "the power role alone");
    assert!(
        document.contains("tacsim.unit.target_mk1") && !document.contains("tacsim.weapon"),
        "the target's document names no weapon: {document}"
    );

    // `refit_unit` of a line tank to the target and back, both ways accepted.
    let (_, tank_document) = act(hull, &tank_spec, false);
    assert!(
        tank_document.contains("tacsim.weapon.cannon_105"),
        "the needle above means something: a tank's document names its gun"
    );
    let (refitted, refitted_document) = act(hull, &spec, true);
    assert_eq!(refitted.schema(), &spec.definition, "the tank is a target");
    assert!(!refitted_document.contains("tacsim.weapon"));
    let (back, back_document) = act(hull, &tank_spec, true);
    assert_eq!(back.schema(), &tank_spec.definition, "and a tank again");
    assert!(back_document.contains("tacsim.weapon.cannon_105"));

    // The check-out, on the stand-in window, at `MatchMode::Range`: no weapon, no
    // sensor, not armed at check-out, cannot close, no speed — and the arena's
    // speed ceiling, hence its substep, is the stock tank's with or without it.
    let stand_in = tacsim::match_::stock_definition(&SmolStr::new(tacsim::match_::STAND_IN))
        .expect("the stand-in window");
    let open = |units: Vec<CustodyUnit>| {
        Arena::new(
            CustodyOut {
                definition: stand_in.clone(),
                units,
                pilots: Vec::new(),
                resolution: tacsim::match_::resolution_for(&fielded),
                seed: SEED,
                mode: tacsim::arena::MatchMode::Range,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("a range opens on any ground")
    };
    let checked_out = |cells: mush_tactics::TacticalUnit, id, team, cell| {
        tacsim::match_::custody_unit(
            &fielded,
            cells,
            id,
            team,
            pose(cell),
            0,
            tacsim::arena::state::Controller::Reflex,
            1,
            &tacsim::match_::SealTerms::fixture(),
        )
        .expect("the fixture terms seal")
    };
    let shooter = || {
        let cells = tacsim::definitions::to_tactical_unit(
            fielded.specs.get(&tank_key).expect("the stock tank"),
            fielded.bindings.get(&tank_key).expect("its binding"),
            RED,
        )
        .expect("the stock tank's cells");
        checked_out(cells, RED, Team::Red, RED_CELL)
    };
    let arena = open(vec![
        shooter(),
        checked_out(made, target, Team::Blue, BLUE_CELL),
    ]);
    let state = arena.unit(target).expect("the target is checked out");
    assert!(state.weapons.is_empty(), "no weapon");
    assert!(state.sensor.is_none(), "no sensor");
    assert!(
        !state.armed_at_checkout(),
        "so it can never make a range `spent`"
    );
    assert!(state.cannot_close(), "and it cannot close");
    assert_eq!(state.mobility.max_speed_cms, 0, "zero speed");
    assert_eq!(state.power, Power::Off, "cold at check-out, like any unit");
    assert!(
        !arena.out_of_action(target),
        "an intact target is in action"
    );
    let without = open(vec![shooter()]);
    assert_eq!(
        arena.max_speed_cms(),
        without.max_speed_cms(),
        "the target does not move the arena's speed ceiling"
    );
    let tempo = ArenaConfig::default().tempo;
    assert_eq!(
        tacsim::arena::clock::substep_ms(&tempo, CELL_CM, arena.max_speed_cms()),
        tacsim::arena::clock::substep_ms(&tempo, CELL_CM, without.max_speed_cms()),
        "nor its substep"
    );

    // The check-in returns it intact.
    let returned = arena.returned_units();
    let home = returned
        .iter()
        .find(|unit| unit.unit == target)
        .expect("the target comes home");
    assert!(!home.wreck && !home.disabled, "intact");
    assert_eq!((home.shots, home.hits), (0, 0));
    assert_eq!(home.cells.schema(), &spec.definition);
}

// ---------------------------------------------------------------------------
// AE.1 — three chassis (16c N15)
// ---------------------------------------------------------------------------

/// **The three chassis' digest pins** (16b cut AE.1; 16c N15), the way X.1 pins
/// the laser's: the scout car, the heavy tank and the second biped.
///
/// **16b cut AE.2** moved the first two with the carriers' version 2 (the
/// autocannon and the rack). Were `20b59cec…` (scout) and `fbb5c73c…` (heavy),
/// both at version 1.
///
/// **17b cut M3.1** moved all three for a core component: the scout to version
/// 3 (the doctrine core in its turret; two sentences of its note shortened to
/// fit the 1,024-byte bound), the heavy to 3 (the command core in its hull),
/// the Mk2 to 2 (the line core in its torso). Were `8c76e3b2…`, `86b6fbfb…`
/// and `0a7346e0…`.
const DIGESTS_CHASSIS: [(&str, &str); 3] = [
    (
        "tacsim.pack.scout_mk1",
        "294b48743c9fd1e34ff01cfcab5240afdebcf714a3d40d72f6aa1f520724834d",
    ),
    (
        "tacsim.pack.heavy_mk1",
        "6f08cfc9129a9ebabecf4af0d826b9e6c37d16b9106e285f39249ba65679e590",
    ),
    (
        "tacsim.pack.biped_mk2",
        "c0bb3689551f5b7f914ed9249516ef955a0f8cd55eefe3dbe300a94d5dcaf217",
    ),
];

/// The three chassis' pack files, by their paths under `content/`.
const PACK_FILES_CHASSIS: [&str; 3] = [
    "units/scout_mk1.json",
    "units/heavy_mk1.json",
    "units/biped_mk2.json",
];

/// **The three chassis load beside the stock game without moving it, and each
/// drives, climbs and pays on its own row** (16b cut AE.1; 16c N15; doc 16 §9's
/// rule read for a table of the fielded catalog).
///
/// X.2's shape, one table: the pins and the writing branch; the stock game is
/// the two weapons, one unit and one arena it always was, and every stock
/// definition keeps its identity; the fielded catalog holds eleven unit schemas
/// (ten at AE.1; AE.3's drone is the eleventh).
/// Then each chassis as the production check-out reads it: the scout binds a
/// `wheeled` mode under its own schema, is held to the wheeled row's 25 per cent
/// ([`tacsim::arena::slope::SlopeLimit::read`]) and pays the wheeled row
/// ([`tacsim::arena::CostRow::read`]); the Mk2 legged, 70 and `Legged`; the
/// heavy tracked, 45 and `Tracked`. The Mk2's laser is mounted in the arm it is
/// installed in (X.2's mount fallback: the laser's own mount names a turret the
/// biped has not got), the scout's wheels, the heavy's tracks and the Mk2's
/// legs carry the pair rule, and the Mk2's heat cells are laid down at the
/// laser tank's 60 and 3.
#[test]
fn ae1_the_three_chassis_load_beside_the_stock_game_without_moving_it() {
    use tacsim::arena::CostRow;
    use tacsim::arena::slope::SlopeLimit;
    use tacsim::definitions::unit::Consequence;

    if writing() {
        for relative in PACK_FILES_CHASSIS {
            resign(relative);
        }
        return;
    }

    // The pins.
    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_CHASSIS.into_iter().collect();
    for (name, json) in content::EXTENSIONS_CHASSIS {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        // 16b cut AE.2 moved the scout car and the heavy to version 2 (their
        // guns); 17b cut M3.1 moved all three one more for their cores — the
        // scout and the heavy to 3, the Mk2 to 2 (was "1 else 2").
        let version = if id == "tacsim.pack.biped_mk2" { 2 } else { 3 };
        assert_eq!(pack.manifest().version(), version, "{id}'s version");
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_CHASSIS.len());

    // The stock game is what it was.
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");
    assert_eq!(stock.arenas.len(), 1, "one arena");

    // The fielded catalog: eleven unit schemas since 16b cut AE.3's drone (ten
    // at AE.1), and no stock identity moved.
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    assert_eq!(
        fielded.specs.len(),
        11,
        "the line tank, doc 16 section 9's five, the range's target, the three chassis and \
         16b cut AE.3's drone"
    );
    for (id, reference) in &stock.references {
        assert_eq!(
            fielded.references.get(id),
            Some(reference),
            "{id} moved when the chassis loaded; a persisted unit points at it"
        );
    }
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved: the wheeled field must serialise away"
    );

    // Each chassis, instantiated as production instantiates it.
    let cells_of = |entry: &str, id: u32| {
        let key = CanonicalId::new(entry).expect("a canonical id");
        let spec = fielded.specs.get(&key).expect("the chassis is fielded");
        let binding = fielded.bindings.get(&key).expect("its binding");
        let cells = tacsim::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(id))
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
        (spec.clone(), cells)
    };
    for (entry, locomotion, schema, limit, row, speed) in [
        (
            content::SCOUT_MK1,
            "wheeled",
            tacsim::definitions::unit::WHEELED_RULES_SCHEMA,
            25,
            CostRow::Wheeled,
            3_000,
        ),
        (
            content::HEAVY_MK1,
            "tracked",
            tacsim::definitions::unit::TRACKED_RULES_SCHEMA,
            45,
            CostRow::Tracked,
            1_000,
        ),
        (
            content::BIPED_MK2,
            "legged",
            tacsim::definitions::unit::LEGGED_RULES_SCHEMA,
            70,
            CostRow::Legged,
            1_000,
        ),
    ] {
        let (spec, cells) = cells_of(entry, 21);
        let profile = cells.mobility().expect("a mobility profile");
        assert_eq!(profile.modes().len(), 1, "{entry}: one mode");
        let mode = profile.modes().first().expect("the mode");
        assert_eq!(mode.locomotion.as_str(), locomotion, "{entry}");
        assert_eq!(mode.rules.schema().key().id.as_str(), schema, "{entry}");
        assert_eq!(SlopeLimit::read(&cells), Some(limit), "{entry}'s limit");
        assert_eq!(CostRow::read(&cells), row, "{entry}'s row");
        // The two readings of one mode's five numbers agree, wheels included.
        assert_eq!(
            tacsim::match_::tracked_rules(&cells),
            tacsim::definitions::tracked_rules(&spec),
            "{entry}: the spec and the instantiated unit decode one mode"
        );
        assert_eq!(
            tacsim::match_::tracked_rules(&cells).max_speed_cms,
            speed,
            "{entry}: its own authored speed, not the stock tank's fallback"
        );
    }
    // The scout serialises its wheels and nothing else.
    let scout = fielded
        .specs
        .get(&CanonicalId::new(content::SCOUT_MK1).expect("a canonical id"))
        .expect("the scout");
    let json = serde_json::to_value(scout).expect("the spec serialises");
    assert_eq!(
        json.get("mobility")
            .and_then(serde_json::Value::as_object)
            .map(|mobility| mobility.keys().cloned().collect::<Vec<String>>()),
        Some(vec!["wheeled".to_owned()])
    );

    // The pair rule on the scout's wheels, the heavy's tracks and the Mk2's legs.
    for (entry, left, right) in [
        (content::SCOUT_MK1, "left_wheels", "right_wheels"),
        (content::HEAVY_MK1, "left_track", "right_track"),
        (content::BIPED_MK2, "left_leg", "right_leg"),
    ] {
        let spec = fielded
            .specs
            .get(&CanonicalId::new(entry).expect("a canonical id"))
            .expect("the chassis");
        let pair = Consequence::DisableMotionWhenAllOf {
            sections: vec![
                CanonicalId::new(left).expect("a section id"),
                CanonicalId::new(right).expect("a section id"),
            ],
        };
        for side in [left, right] {
            let section = spec
                .sections
                .iter()
                .find(|row| row.id.as_str() == side)
                .unwrap_or_else(|| panic!("{entry} has {side}"));
            assert!(
                section.consequences.contains(&pair),
                "{entry}'s {side} does not carry the pair rule: {:?}",
                section.consequences
            );
        }
    }

    // The Mk2's beam is in its right arm (X.2's mount fallback), its coaxial in
    // its left; the scout's guns are, since cut AE.2, the autocannon first and
    // the coaxial slaved to it.
    let (_, mk2) = cells_of(content::BIPED_MK2, 22);
    let weapons = tacsim::match_::weapon_summaries(&fielded, &mk2);
    let mount = |id: &str| {
        weapons
            .iter()
            .find(|weapon| weapon.id.as_str() == id)
            .map(|weapon| weapon.mount_section.to_string())
    };
    assert_eq!(mount("laser"), Some("right_arm".to_owned()));
    assert_eq!(mount("machine_gun"), Some("left_arm".to_owned()));
    let (_, scout_cells) = cells_of(content::SCOUT_MK1, 23);
    assert_eq!(
        tacsim::match_::weapon_summaries(&fielded, &scout_cells)
            .iter()
            .map(|weapon| weapon.id.to_string())
            .collect::<Vec<_>>(),
        vec!["autocannon".to_owned(), "machine_gun".to_owned()],
        "the scout's guns in component order, since cut AE.2"
    );

    // And the Mk2's heat cells are laid down at check-out, on the legged row.
    let arena = arena_of(
        &fielded,
        vec![unit_of(
            &fielded,
            content::BIPED_MK2,
            RED,
            Team::Red,
            RED_CELL,
        )],
        &[],
    );
    // On the torso: a biped's body is its first section, which carries the
    // heat cells (`UnitSpec::heat_section`).
    let torso = |name: &str| mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("torso").expect("a section"),
        cell: mush_tactics::CellId::new(name).expect("a cell"),
    };
    assert_eq!(cell(&arena, RED, &torso(HEAT_CELL)), Some(0), "cold");
    assert_eq!(cell(&arena, RED, &torso(HEAT_SINK_CELL)), Some(3));
    assert_eq!(arena.unit(RED).and_then(UnitState::heat_max), Some(60));
    assert_eq!(
        arena.unit(RED).map(|unit| unit.cost_row),
        Some(CostRow::Legged),
        "and it checks out on the legged row"
    );
}

// ---------------------------------------------------------------------------
// AE.1 — the cost per row (16c B-58) and The Bluff (16c T-18)
// ---------------------------------------------------------------------------

/// A cell of `class` priced as the loader prices it (16b cut AE.1): the shipped
/// table's `tracked` row as `move_cost`, `0` where it is `null`.
fn priced_cell(class: TerrainClass) -> TerrainCell {
    TerrainCell {
        class,
        elev_cm: 0,
        move_cost: mush_atlas::physics::LocomotionTable::shipped()
            .cost(class.label(), "tracked", &[])
            .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
    }
}

/// One painted box of [`priced_window`]: `(x0, x1, y0, y1)` inclusive, and its
/// class.
type PricedBox = ((u16, u16, u16, u16), TerrainClass);

/// **A level window priced per row** (16b cut AE.1): 48 by 8 of grass, each
/// `(x0, x1, y0, y1)` box of `paint` its class, through `ArenaTerrain::priced`
/// and the shipped table — the terrain `from_pack` builds over those classes —
/// with [`relief`]'s two entry zones.
fn priced_window(paint: &[PricedBox]) -> ArenaDefinition {
    let (width, height) = (48_u16, 8_u16);
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            let class = paint
                .iter()
                .rev()
                .find(|((x0, x1, y0, y1), _)| (*x0..=*x1).contains(&x) && (*y0..=*y1).contains(&y))
                .map_or(TerrainClass::Grass, |(_, class)| *class);
            cells.push(priced_cell(class));
        }
    }
    let mut definition = relief(width, height, &|_x: u16, _y: u16| 0);
    definition.terrain = ArenaTerrain::priced(
        width,
        height,
        CELL_CM,
        cells,
        mush_atlas::physics::LocomotionTable::shipped(),
    )
    .expect("the priced window");
    definition
}

/// Every `blocked` line naming `unit`, as `(x, y, cause)` (16b cut AE.1).
fn blocked_with_cause(
    events: &[tacsim::arena::ledger::LedgerEvent],
    unit: mush_core::ObjId,
) -> Vec<(u16, u16, tacsim::arena::motion::BlockedCause)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::Blocked {
                unit: at,
                x,
                y,
                cause,
            } if *at == unit => Some((*x, *y, *cause)),
            _ => None,
        })
        .collect()
}

/// Run `pulses` pulses of 500 ms and answer every speed `id` had while it stood
/// in a column of `columns` (16b cut AE.1).
fn speeds_in(
    arena: &mut Arena,
    wall: &mut i64,
    pulses: usize,
    id: mush_core::ObjId,
    columns: std::ops::RangeInclusive<u16>,
    bytes: &mut Vec<u8>,
) -> Vec<i32> {
    let mut seen = Vec::new();
    for _ in 0..pulses {
        run_logged(arena, wall, 500, 1, bytes);
        let (cell, _, _) = standing(arena, id);
        if columns.contains(&cell.0) {
            seen.extend(arena.unit(id).map(|unit| unit.speed_cms));
        }
    }
    seen
}

/// **The loader prices every row, and a fixture prices none** (16b cut AE.1;
/// 16c `B-58`).
///
/// Over the shipped ground, built from the production roster's records through
/// `from_pack`, every cell answers, for every row, exactly the shipped table's
/// price for its class (`null` → `0`), and the rows' cheapest cells are 100, 80
/// and 100 — the road, for all three, which the wheeled row prices at 80. And a
/// window built through `ArenaTerrain::new` is a fixture's word for every unit:
/// a `WoodsHeavy` cell authored at 150 answers 150 on every row (the table
/// would refuse a tank and charge a walker 200), a `Grass` cell authored at 0
/// answers 0 on every row, and the three minima agree.
#[test]
fn ae1_the_loader_prices_every_row_and_a_fixture_prices_none() {
    use tacsim::arena::CostRow;

    let tw = mush_testkit::TestWorld::with_modules(x6_production_roster());
    let records = tw
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| state.records.clone())
        .expect("the atlas is composed");
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, def) = stock
        .arenas
        .iter()
        .next()
        .expect("the proving ground is shipped");
    let arena = tacsim::definitions::from_pack(id.as_str(), def, &records)
        .expect("the arena's ground is in this world's records");
    let terrain = &arena.terrain;
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut classes = std::collections::BTreeSet::new();
    for y in 0..terrain.height() {
        for x in 0..terrain.width() {
            let cell = terrain.get(x, y).expect("on the window");
            classes.insert(cell.class.label());
            for row in CostRow::ALL {
                let want = table
                    .cost(cell.class.label(), row.as_str(), &[])
                    .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX));
                assert_eq!(
                    terrain.cost(row, x, y),
                    want,
                    "({x}, {y}) on {:?} for the {} row",
                    cell.class,
                    row.as_str()
                );
                assert_eq!(terrain.passable_for(row, x, y), want != 0);
            }
            // The tracked price is the cell's own, as it always was.
            assert_eq!(terrain.cost(CostRow::Tracked, x, y), cell.move_cost);
        }
    }
    assert!(
        classes.contains("woods_heavy") && classes.contains("rubble") && classes.contains("water"),
        "the shipped ground carries the classes the rows disagree on: {classes:?}"
    );
    assert_eq!(
        CostRow::ALL.map(|row| terrain.min_cost(row)),
        [100, 80, 100],
        "the road, which the wheeled row prices at 80"
    );
    assert_eq!(
        tacsim::arena::min_move_cost(terrain),
        100,
        "the tracked reading"
    );

    // A fixture's window prices every row at the authored number.
    let fixture = ArenaTerrain::new(
        2,
        1,
        CELL_CM,
        vec![
            TerrainCell {
                class: TerrainClass::WoodsHeavy,
                elev_cm: 0,
                move_cost: 150,
            },
            TerrainCell {
                class: TerrainClass::Grass,
                elev_cm: 0,
                move_cost: 0,
            },
        ],
    )
    .expect("a two-cell window");
    for row in CostRow::ALL {
        assert_eq!(fixture.cost(row, 0, 0), 150, "{}", row.as_str());
        assert_eq!(fixture.cost(row, 1, 0), 0, "{}", row.as_str());
        assert!(!fixture.passable_for(row, 1, 0));
        assert_eq!(fixture.min_cost(row), 150, "{}", row.as_str());
    }
}

/// **A biped walks the thick woods a tank is refused** (16b cut AE.1; the
/// `B-58` witness), on a window priced per row through `ArenaTerrain::priced`.
///
/// A band of `WoodsHeavy` across a level window of grass, a Mk1 biped and a
/// stock line tank driving east at it side by side. The walker's `legged` row
/// prices the woods at 200, so it crosses at `max × 100 / 200` — 600 of its
/// 1,200 cm/s — and stands beyond the band; the tank's `tracked` row prices it
/// `null`, so the tank is held at the band's edge: one `blocked` line, cause
/// `impassable`, and `Held: the ground ahead will not take the tank.` And on
/// `Rubble` each pays its own row: the walker 150 (800 of its 1,200) and the
/// tank 200 (750 of its 1,500).
#[test]
fn ae1_a_biped_walks_the_thick_woods_a_tank_is_refused() {
    use tacsim::arena::CostRow;
    use tacsim::arena::motion::BlockedCause;

    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let woods = || priced_window(&[((20, 27, 0, 7), TerrainClass::WoodsHeavy)]);
    let pair = |definition: ArenaDefinition| {
        let mut arena = arena_over(
            &catalog,
            vec![
                blind(unit_of(
                    &catalog,
                    content::BIPED_MK1,
                    RED,
                    Team::Red,
                    (16, 2),
                )),
                blind(unit_of(
                    &catalog,
                    content::LINE_TANK,
                    BLUE,
                    Team::Blue,
                    (16, 5),
                )),
            ],
            definition,
            &ArenaConfig::default(),
        );
        under_way(&mut arena, RED, 90_000, 1_200);
        under_way(&mut arena, BLUE, 90_000, 1_500);
        arena
    };

    let mut arena = pair(woods());
    assert_eq!(
        arena.unit(RED).map(|unit| unit.cost_row),
        Some(CostRow::Legged)
    );
    assert_eq!(
        arena.unit(BLUE).map(|unit| unit.cost_row),
        Some(CostRow::Tracked)
    );
    let terrain = arena.definition.terrain.clone();
    assert_eq!(terrain.cost(CostRow::Legged, 22, 2), 200);
    assert_eq!(terrain.cost(CostRow::Tracked, 22, 5), 0);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let in_woods = speeds_in(&mut arena, &mut wall, 40, RED, 24..=27, &mut bytes);
    assert!(
        !in_woods.is_empty(),
        "the walker never stood deep in the woods"
    );
    assert!(
        in_woods.iter().all(|speed| *speed == 1_200 * 100 / 200),
        "the walker's speed deep in the woods is its own row's 600: {in_woods:?}"
    );
    let ((x, _), _, _) = standing(&arena, RED);
    assert!(x >= 28, "the walker stands beyond the band: column {x}");
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_with_cause(&events, RED).is_empty(),
        "the walker was refused nothing"
    );
    assert_eq!(
        blocked_with_cause(&events, BLUE),
        vec![(20, 5, BlockedCause::Impassable)],
        "one stop, the ground's own"
    );
    assert_eq!(standing(&arena, BLUE).0, (19, 5));

    // What the tank's pilot is told: the ground's sentence, once.
    let mut arena = pair(woods());
    let mut wall = 0_i64;
    let told = run_heard(&mut arena, &mut wall, 500, 40, mush_core::ObjId(1_001));
    assert_eq!(
        told.iter()
            .filter(|line| line.starts_with("Held:"))
            .collect::<Vec<&String>>(),
        vec![&"Held: the ground ahead will not take the tank.".to_owned()],
        "{told:?}"
    );

    // On rubble each pays its own row.
    let mut arena = pair(priced_window(&[((20, 27, 0, 7), TerrainClass::Rubble)]));
    let rubble = arena.definition.terrain.clone();
    assert_eq!(
        (
            rubble.cost(CostRow::Legged, 22, 2),
            rubble.cost(CostRow::Tracked, 22, 5)
        ),
        (150, 200)
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut walker = Vec::new();
    let mut hull = Vec::new();
    for _ in 0..30 {
        run_logged(&mut arena, &mut wall, 500, 1, &mut bytes);
        if (24..=27).contains(&standing(&arena, RED).0.0) {
            walker.extend(arena.unit(RED).map(|unit| unit.speed_cms));
        }
        if (24..=27).contains(&standing(&arena, BLUE).0.0) {
            hull.extend(arena.unit(BLUE).map(|unit| unit.speed_cms));
        }
    }
    assert!(
        !walker.is_empty() && !hull.is_empty(),
        "both stood deep in the rubble"
    );
    assert!(
        walker.iter().all(|speed| *speed == 1_200 * 100 / 150),
        "the walker's factor is 100/150: {walker:?}"
    );
    assert!(
        hull.iter().all(|speed| *speed == 1_500 * 100 / 200),
        "the hull's factor is 100/200: {hull:?}"
    );
}

/// **A wheeled car is stopped by rubble and takes the ford slowly** (16b cut
/// AE.1; 16c `B-58`, N15), on windows priced per row.
///
/// Rubble, rock and the thick woods all refuse the scout car — each one
/// `blocked` line, cause `impassable` — where a tank crosses the first two.
/// Shallow water prices 400 for it (the one number the engine's table moved)
/// and 300 for a tank, and it crosses a ford at `3,000 × 100 / 400`; a road is
/// 80. Its authored 3,000 cm/s is its top speed on the road — an order for
/// more is clamped there — although the arena's ceiling on that window, which
/// sizes the substep, is 3,750. A staff place of the car onto rubble is refused
/// `impassable`, where the same place of a tank lands; and a car whose world
/// pose is on rubble falls through `opening_pose` to its zone, and the arena
/// opens.
#[test]
fn ae1_a_wheeled_car_is_stopped_by_rubble_and_takes_the_ford_slowly() {
    use tacsim::arena::CostRow;
    use tacsim::arena::motion::BlockedCause;
    use tacsim::arena::state::Channel;

    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let car = |id: mush_core::ObjId, cell: (u16, u16)| {
        blind(unit_of(&catalog, content::SCOUT_MK1, id, Team::Red, cell))
    };

    // Three classes, three cars, three stops.
    let third = mush_core::ObjId(11);
    let mut arena = arena_over(
        &catalog,
        vec![car(RED, (16, 1)), car(BLUE, (16, 3)), car(third, (16, 5))],
        priced_window(&[
            ((20, 23, 0, 1), TerrainClass::Rubble),
            ((20, 23, 2, 3), TerrainClass::Rock),
            ((20, 23, 4, 7), TerrainClass::WoodsHeavy),
        ]),
        &ArenaConfig::default(),
    );
    for id in [RED, BLUE, third] {
        assert_eq!(
            arena.unit(id).map(|unit| unit.cost_row),
            Some(CostRow::Wheeled)
        );
        under_way(&mut arena, id, 90_000, 1_000);
    }
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    for (id, y, what) in [
        (RED, 1, "rubble"),
        (BLUE, 3, "rock"),
        (third, 5, "thick woods"),
    ] {
        assert_eq!(
            blocked_with_cause(&events, id),
            vec![(20, y, BlockedCause::Impassable)],
            "{what} refuses the car"
        );
    }
    let terrain = arena.definition.terrain.clone();
    assert!(
        terrain.passable_for(CostRow::Tracked, 21, 1),
        "a tank crosses rubble"
    );
    assert!(terrain.passable_for(CostRow::Tracked, 21, 3), "and rock");

    // The ford, and the road.
    let ford = priced_window(&[
        ((20, 27, 0, 3), TerrainClass::Water),
        ((0, 47, 6, 6), TerrainClass::Road),
    ]);
    assert_eq!(
        ford.terrain.cost(CostRow::Wheeled, 22, 1),
        400,
        "a ford is 400 to a car"
    );
    assert_eq!(
        ford.terrain.cost(CostRow::Tracked, 22, 1),
        300,
        "and 300 to a tank"
    );
    assert_eq!(
        ford.terrain.cost(CostRow::Wheeled, 30, 6),
        80,
        "a road is 80"
    );
    let mut arena = arena_over(
        &catalog,
        vec![car(RED, (16, 1)), car(BLUE, (2, 6))],
        ford,
        &ArenaConfig::default(),
    );
    assert_eq!(
        arena.max_speed_cms(),
        3_750,
        "the arena sizes the substep on 3,000 x 100 / 80"
    );
    under_way(&mut arena, RED, 90_000, 1_000);
    // The road car is ordered through the pilot's own door, and asked for more
    // than it has: the order clamp holds it at its authored 3,000.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.power = Power::Online;
        state.pose.heading_mdeg = 90_000;
        state.desired_heading_mdeg = 90_000;
    }
    let kind = tacsim::arena::intents::IntentKind::Speed { cms: 5_000 };
    let intent = tacsim::arena::intents::Intent {
        unit: BLUE,
        channel: Channel::Helm,
        epoch: arena.epochs.current(BLUE, Channel::Helm),
        kind,
        seq: 1,
    };
    arena.step(0, &[], &[intent]);
    assert_eq!(
        arena.unit(BLUE).map(|unit| unit.desired_speed_cms),
        Some(3_000),
        "an order for more than the authored speed is clamped to it"
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut in_ford = Vec::new();
    let mut on_road = 0_i32;
    for _ in 0..30 {
        run_logged(&mut arena, &mut wall, 500, 1, &mut bytes);
        if (24..=27).contains(&standing(&arena, RED).0.0) {
            in_ford.extend(arena.unit(RED).map(|unit| unit.speed_cms));
        }
        on_road = on_road.max(arena.unit(BLUE).map_or(0, |unit| unit.speed_cms));
    }
    assert!(!in_ford.is_empty(), "the car stood deep in the ford");
    assert!(
        in_ford.iter().all(|speed| *speed == 3_000 * 100 / 400),
        "the car takes the ford at its own row's 750: {in_ford:?}"
    );
    assert_eq!(on_road, 3_000, "108 kph on the road, and not 3,750");
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_with_cause(&events, RED).is_empty(),
        "the ford refuses no car"
    );

    // A staff place onto rubble: `impassable` for the car, taken by the tank.
    let rubble = || priced_window(&[((30, 33, 0, 7), TerrainClass::Rubble)]);
    let mut arena = arena_over(
        &catalog,
        vec![
            car(RED, (16, 1)),
            blind(unit_of(
                &catalog,
                content::LINE_TANK,
                BLUE,
                Team::Blue,
                (16, 5),
            )),
        ],
        rubble(),
        &ArenaConfig::default(),
    );
    let place = |unit: mush_core::ObjId, arena: &Arena| {
        let kind = tacsim::arena::intents::IntentKind::Place { x: 31, y: 3 };
        tacsim::arena::intents::Intent {
            unit,
            channel: kind.channel_required(),
            epoch: arena.epochs.current(unit, kind.channel_required()),
            kind,
            seq: 1,
        }
    };
    let intents = [place(RED, &arena)];
    let mut bytes: Vec<u8> = Vec::new();
    let out = arena.step(0, &[], &intents);
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        events.iter().any(|event| matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::PlaceRefused { unit, reason, .. }
                if *unit == RED && reason.as_str() == "impassable"
        )),
        "the car's place onto rubble is refused `impassable`: {events:?}"
    );
    assert_eq!(standing(&arena, RED).0, (16, 1), "and it did not move");
    let intents = [place(BLUE, &arena)];
    arena.step(500, &[], &intents);
    assert_eq!(standing(&arena, BLUE).0, (31, 3), "the tank's place lands");

    // `opening_pose`: a car left on rubble opens in its zone; a tank there keeps it.
    let definition = rubble();
    let key = CanonicalId::new(content::SCOUT_MK1).expect("a canonical id");
    let cells = tacsim::definitions::to_tactical_unit(
        catalog.specs.get(&key).expect("the scout"),
        catalog.bindings.get(&key).expect("its binding"),
        RED,
    )
    .expect("the scout's cells");
    let row = CostRow::read(&cells);
    assert_eq!(row, CostRow::Wheeled);
    let on_rubble = pose((31, 3));
    let mut taken = Vec::new();
    let opened = tacsim::match_::opening_pose(
        &definition,
        Team::Red,
        Some(on_rubble.clone()),
        &mut taken,
        row,
    )
    .expect("the zone has room");
    let cell = definition.terrain.cell_of(&opened).expect("on the window");
    assert!(
        cell.0 <= 1,
        "the car opens in its zone, not on rubble: {cell:?}"
    );
    let mut taken = Vec::new();
    let kept = tacsim::match_::opening_pose(
        &definition,
        Team::Red,
        Some(on_rubble),
        &mut taken,
        CostRow::Tracked,
    )
    .expect("a tank's pose");
    assert_eq!(
        definition.terrain.cell_of(&kept),
        Some((31, 3)),
        "a tank keeps it"
    );
    let mut unit = car(RED, (16, 1));
    unit.pose = opened;
    let arena = arena_over(&catalog, vec![unit], definition, &ArenaConfig::default());
    assert_eq!(standing(&arena, RED).0, cell, "and the arena opens there");
}

/// **The route is the row's own** (16b cut AE.1; 16c `B-58`; doc 16 §6).
///
/// One window priced per row: a band of thick woods with a one-row gap at its
/// north end, and a rubble cell. The walker's search, scaled on its own row's
/// cheapest cell, goes straight through the woods; the tank's goes round
/// through the gap, because the woods are not ground to it; a car's search to
/// the rubble cell is `OffGround`, because it is not ground to a car. The
/// per-row property that every search step is one motion accepts is
/// `crate::arena::path`'s own in-source test, over every row.
#[test]
fn ae1_the_route_is_the_rows_own() {
    use tacsim::arena::CostRow;

    let definition = priced_window(&[
        ((20, 23, 0, 6), TerrainClass::WoodsHeavy),
        ((40, 40, 3, 3), TerrainClass::Rubble),
    ]);
    let terrain = &definition.terrain;
    let search = |row: CostRow, goal: (u16, u16)| {
        tacsim::arena::path_with(
            terrain,
            &mut tacsim::arena::PathScratch::default(),
            (2, 3),
            goal,
            65_536,
            terrain.min_cost(row),
            Some(45),
            row,
            &[],
        )
    };
    let walker = search(CostRow::Legged, (30, 3)).expect("a walker's route");
    assert!(
        walker
            .cells
            .iter()
            .any(|&(x, y)| (20..=23).contains(&x) && y <= 6),
        "the walker's route goes through the woods: {:?}",
        walker.cells
    );
    let tank = search(CostRow::Tracked, (30, 3)).expect("a tank's route");
    assert!(
        tank.cells
            .iter()
            .filter(|(x, _)| (20..=23).contains(x))
            .all(|(_, y)| *y == 7),
        "the tank's route goes round, through the gap on row 7: {:?}",
        tank.cells
    );
    assert!(
        tank.cells.iter().any(|(x, _)| (20..=23).contains(x)),
        "the tank's route does cross the band's columns, at the gap: {:?}",
        tank.cells
    );
    assert_eq!(
        search(CostRow::Wheeled, (40, 3)),
        Err(tacsim::arena::PathFailure::OffGround),
        "rubble is no goal for a car"
    );
    assert!(
        search(CostRow::Tracked, (40, 3)).is_ok(),
        "and is one for a tank"
    );
}

/// Whether the x6 hand walk's shoulder check refuses the diagonal `from → to`
/// at `limit` (16b cut AE.1): a shoulder that is on the window and passable
/// must be within the limit orthogonally from `from` and onto `to`.
fn shoulder_refused(terrain: &ArenaTerrain, limit: u32, from: (u16, u16), to: (u16, u16)) -> bool {
    let cell_cm = u32::from(terrain.cell_cm());
    let (Some(here), Some(there)) = (terrain.get(from.0, from.1), terrain.get(to.0, to.1)) else {
        return false;
    };
    [(to.0, from.1), (from.0, to.1)]
        .into_iter()
        .any(|shoulder| {
            let Some(cell) = terrain
                .get(shoulder.0, shoulder.1)
                .filter(|cell| cell.passable())
            else {
                return false;
            };
            [(here, cell), (cell, there)].into_iter().any(|(a, b)| {
                tacsim::arena::slope::too_steep(
                    Some(limit),
                    tacsim::arena::slope::steepness_pct(
                        a,
                        b,
                        mush_atlas::physics::StepKind::Orthogonal,
                        cell_cm,
                    ),
                )
            })
        })
}

/// **The Bluff refuses a tank and a car and admits a walker, on the shipped
/// ground** (16b cut AE.1; 16c `T-18`; doc 16 §9 item 6).
///
/// The x6 production roster over the real records, and an arena built through
/// `definition_for`'s own `from_pack`. The rules X.6 wrote and only fixtures
/// ever ran now run on the shipped map:
///
/// * a **pilot-driven** tank driven east at the west face from (140, 111) is
///   refused the 700 cm rim by the slope — one `too_steep` `blocked` line and
///   `Held: the slope ahead is too steep for the tank.`; a **scout car** driven
///   the same way is refused at the same cell by the same rule (its row's 25)
///   and told the same sentence;
/// * a walker driven the same way climbs it (70 per cent, which its row's 70
///   does not pass) and stands on the top at `z_cm` 500;
/// * **row 120's east flank**, (147, 120) up onto the ramp's foot at (146,
///   120), rises 340 cm (34 per cent): a tank driven west along row 120 takes
///   it with no stop, and a car is refused it — one `too_steep` line at (146,
///   120);
/// * the tank's search from (144, 125) to the top at (144, 111) goes up the
///   ramp, and every edge of the route passes the x6 hand walk — the slope and
///   the shoulders;
/// * **the production search's shoulder rule decides on the shipped ground**:
///   the diagonals (140, 118) → (141, 119) (33 per cent) and (148, 118) →
///   (147, 119) (flat) are each inside a tank's 45 and refused only because a
///   shoulder is a 70 per cent rim, so a tracked `path_with` goes round and a
///   legged one (70) takes the one diagonal step; the corner (147, 118) →
///   (148, 119) rises 800 (56.6 per cent), past a tank's own limit and not a
///   walker's;
/// * and a **routeless** reflex tank steered at the face is held by
///   `first_step_is_open`, which is 16b cut Q.7's cliff test: slope refusals
///   counted, no `blocked` line, and an empty avoid list — `note_blocked`
///   records nothing for a slope, so the openness test is the whole of the
///   machine's cliff memory.
#[test]
fn ae1_the_bluff_refuses_a_tank_and_a_car_and_admits_a_walker_on_the_shipped_ground() {
    use tacsim::arena::CostRow;
    use tacsim::arena::motion::BlockedCause;

    let tw = mush_testkit::TestWorld::with_modules(x6_production_roster());
    let shipped =
        tacsim::match_::definition_for(tw.world(), &SmolStr::new(tacsim::match_::PROVING_GROUND))
            .expect("the proving ground's ground builds")
            .expect("the proving ground is shipped");
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let terrain = shipped.terrain.clone();
    assert_eq!(terrain.get(141, 111).map(|cell| cell.elev_cm), Some(500));
    assert_eq!(terrain.get(140, 111).map(|cell| cell.elev_cm), Some(-200));

    // The pilot-driven tank at the west face.
    let at_face = |entry: &str| {
        let mut arena = arena_over(
            &catalog,
            vec![blind(unit_of(&catalog, entry, RED, Team::Red, (138, 111)))],
            shipped.clone(),
            &ArenaConfig::default(),
        );
        let top = arena.unit(RED).map_or(0, |unit| {
            i32::try_from(unit.mobility.max_speed_cms).unwrap_or(0)
        });
        under_way(&mut arena, RED, 90_000, top);
        arena
    };
    let mut arena = at_face(content::LINE_TANK);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert_eq!(
        blocked_with_cause(&events, RED),
        vec![(141, 111, BlockedCause::TooSteep)],
        "one stop, the slope's own"
    );
    assert_eq!(standing(&arena, RED).0, (140, 111));
    let mut arena = at_face(content::LINE_TANK);
    let mut wall = 0_i64;
    let told = run_heard(&mut arena, &mut wall, 500, 20, mush_core::ObjId(1_000));
    assert_eq!(
        told.iter()
            .filter(|line| line.starts_with("Held:"))
            .collect::<Vec<&String>>(),
        vec![&"Held: the slope ahead is too steep for the tank.".to_owned()],
        "{told:?}"
    );

    // The scout car at the same face: the same cell, the same cause, its own
    // row's limit, and the tank's sentence (the cockpit's word names no
    // chassis).
    let mut arena = at_face(content::SCOUT_MK1);
    assert_eq!(
        arena
            .unit(RED)
            .map(|unit| (unit.cost_row, unit.slope_limit_pct)),
        Some((CostRow::Wheeled, Some(25)))
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    run_logged(&mut arena, &mut wall, 500, 20, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert_eq!(
        blocked_with_cause(&events, RED),
        vec![(141, 111, BlockedCause::TooSteep)],
        "the car's one stop, the slope's own"
    );
    assert_eq!(standing(&arena, RED).0, (140, 111));
    let mut arena = at_face(content::SCOUT_MK1);
    let mut wall = 0_i64;
    let told = run_heard(&mut arena, &mut wall, 500, 20, mush_core::ObjId(1_000));
    assert_eq!(
        told.iter()
            .filter(|line| line.starts_with("Held:"))
            .collect::<Vec<&String>>(),
        vec![&"Held: the slope ahead is too steep for the tank.".to_owned()],
        "{told:?}"
    );

    // Row 120's east flank: a tank takes it, a car is refused it.
    assert_eq!(terrain.get(147, 120).map(|cell| cell.elev_cm), Some(-300));
    assert_eq!(terrain.get(146, 120).map(|cell| cell.elev_cm), Some(40));
    let at_flank = |entry: &str| {
        let mut arena = arena_over(
            &catalog,
            vec![blind(unit_of(&catalog, entry, RED, Team::Red, (150, 120)))],
            shipped.clone(),
            &ArenaConfig::default(),
        );
        let top = arena.unit(RED).map_or(0, |unit| {
            i32::try_from(unit.mobility.max_speed_cms).unwrap_or(0)
        });
        under_way(&mut arena, RED, 270_000, top);
        let mut wall = 0_i64;
        let mut bytes: Vec<u8> = Vec::new();
        run_logged(&mut arena, &mut wall, 500, 12, &mut bytes);
        let events = ledger_of(&mut arena, &mut bytes);
        (blocked_with_cause(&events, RED), standing(&arena, RED).0)
    };
    let (stops, (x, y)) = at_flank(content::LINE_TANK);
    assert!(
        stops.is_empty(),
        "the tank was refused the flank: {stops:?}"
    );
    assert!(
        y == 120 && x <= 145,
        "the tank is on the ramp's foot, west of the flank: ({x}, {y})"
    );
    let (stops, cell) = at_flank(content::SCOUT_MK1);
    assert_eq!(
        stops,
        vec![(146, 120, BlockedCause::TooSteep)],
        "the car's one stop is the flank"
    );
    assert_eq!(cell, (147, 120));

    // The walker climbs it and stands on the top.
    let mut arena = at_face(content::BIPED_MK1);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.cost_row),
        Some(CostRow::Legged)
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut highest = i32::MIN;
    for _ in 0..16 {
        run_logged(&mut arena, &mut wall, 500, 1, &mut bytes);
        highest = highest.max(arena.unit(RED).map_or(i32::MIN, |unit| unit.pose.z_cm));
    }
    assert_eq!(highest, 500, "the walker stood on the top");
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        blocked_with_cause(&events, RED).is_empty(),
        "and was refused nothing"
    );

    // The tank's route to the top goes up the ramp, and every edge of it passes
    // the hand walk.
    let limit = tracked_limit();
    let start = (144_u16, 125_u16);
    let goal = (144_u16, 111_u16);
    let route = tacsim::arena::path_with(
        &terrain,
        &mut tacsim::arena::PathScratch::default(),
        start,
        goal,
        65_536,
        terrain.min_cost(CostRow::Tracked),
        Some(limit),
        CostRow::Tracked,
        &[],
    )
    .expect("the ramp is a way up");
    assert!(
        route
            .cells
            .iter()
            .any(|&(x, y)| y == 120 && (141..=146).contains(&x))
            && route
                .cells
                .iter()
                .any(|&(x, y)| y == 119 && (141..=146).contains(&x)),
        "the route takes the ramp: {:?}",
        route.cells
    );
    let cell_cm = u32::from(terrain.cell_cm());
    let mut walked = vec![start];
    walked.extend(route.cells.iter().copied());
    for pair in walked.windows(2) {
        let (Some(from), Some(to)) = (pair.first().copied(), pair.get(1).copied()) else {
            continue;
        };
        let (Some(here), Some(there), Some(kind)) = (
            terrain.get(from.0, from.1),
            terrain.get(to.0, to.1),
            tacsim::arena::slope::step_kind(from, to),
        ) else {
            panic!("{from:?} -> {to:?} is not a step");
        };
        assert!(
            !tacsim::arena::slope::too_steep(
                Some(limit),
                tacsim::arena::slope::steepness_pct(here, there, kind, cell_cm)
            ),
            "{from:?} -> {to:?} is past the limit"
        );
        if kind == mush_atlas::physics::StepKind::Diagonal {
            assert!(
                !shoulder_refused(&terrain, limit, from, to),
                "a shoulder of {from:?} -> {to:?} is past the limit"
            );
        }
    }

    // The production search's shoulder rule, on the shipped ground: two
    // diagonals inside a tank's limit that only a rim shoulder refuses, and the
    // 800 cm corner, which a tank's own limit refuses. Each is one step for a
    // walker and a way round for a tank.
    let one_step = |from: (u16, u16), to: (u16, u16), limit: u32, row: CostRow| {
        tacsim::arena::path_with(
            &terrain,
            &mut tacsim::arena::PathScratch::default(),
            from,
            to,
            65_536,
            terrain.min_cost(row),
            Some(limit),
            row,
            &[],
        )
        .map(|found| found.cells)
    };
    let diagonal_pct = |from: (u16, u16), to: (u16, u16)| {
        let (Some(here), Some(there)) = (terrain.get(from.0, from.1), terrain.get(to.0, to.1))
        else {
            panic!("{from:?} -> {to:?} is off the window");
        };
        tacsim::arena::slope::steepness_pct(
            here,
            there,
            mush_atlas::physics::StepKind::Diagonal,
            cell_cm,
        )
    };
    for (from, to) in [
        ((140_u16, 118_u16), (141_u16, 119_u16)),
        ((148, 118), (147, 119)),
    ] {
        assert!(
            !tacsim::arena::slope::too_steep(Some(limit), diagonal_pct(from, to)),
            "{from:?} -> {to:?} is inside a tank's limit on its own"
        );
        assert!(
            shoulder_refused(&terrain, limit, from, to),
            "and a shoulder of it is a rim"
        );
        let tank = one_step(from, to, limit, CostRow::Tracked).expect("a tank's way round");
        assert_ne!(
            tank,
            vec![to],
            "the tank's search took the diagonal {from:?} -> {to:?} past its rim shoulder"
        );
        assert_eq!(tank.last(), Some(&to));
        assert_eq!(
            one_step(from, to, 70, CostRow::Legged),
            Ok(vec![to]),
            "the walker's search takes {from:?} -> {to:?} in one step"
        );
    }
    assert!(tacsim::arena::slope::too_steep(
        Some(limit),
        diagonal_pct((147, 118), (148, 119))
    ));
    assert_ne!(
        one_step((147, 118), (148, 119), limit, CostRow::Tracked),
        Ok(vec![(148, 119)]),
        "the 800 cm corner is no step for a tank"
    );
    assert_eq!(
        one_step((147, 118), (148, 119), 70, CostRow::Legged),
        Ok(vec![(148, 119)]),
        "and one for a walker"
    );

    // The routeless machine, held at the face by the one-cell test.
    let mut definition = shipped.clone();
    definition.waypoints = relief_waypoint(goal);
    let cfg = ArenaConfig {
        reflex: tacsim::config::ReflexConfig {
            // A search that cannot finish, so the machine steers straight at
            // the node on the top with no route at all.
            path_max_cells: 1,
            ..tacsim::config::ReflexConfig::default()
        },
        ..ArenaConfig::default()
    };
    let mut arena = arena_over(
        &catalog,
        vec![under_reflex(blind(unit_of(
            &catalog,
            content::LINE_TANK,
            RED,
            Team::Red,
            (130, 111),
        )))],
        definition,
        &cfg,
    );
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let counters = run_logged(&mut arena, &mut wall, 500, 120, &mut bytes);
    let events = ledger_of(&mut arena, &mut bytes);
    assert!(
        counters.path_failures > 0,
        "the search never failed, so the machine was never routeless"
    );
    assert!(
        counters.slope_refusals > 0,
        "the one-cell test never refused the face"
    );
    assert!(
        blocked_with_cause(&events, RED).is_empty(),
        "the routeless machine was refused at the face: {:?}",
        blocked_with_cause(&events, RED)
    );
    let (cell, z, _) = standing(&arena, RED);
    assert!(
        cell.0 <= 140 && z < 500,
        "the machine got onto the top: {cell:?} at {z} cm"
    );
    assert_eq!(
        // 17b cut M1a.1: the avoid list is the hull's motor row's.
        arena.motors.row(RED).map(|motor| motor.avoid.is_empty()),
        Some(true),
        "a slope teaches the avoid list nothing"
    );
}

/// **The reflex steers each machine on its own row, at every cadence** (16b cut
/// AE.1; 16c `B-58`; 16b §11's cross-tempo property for the machine's half of
/// the cut).
///
/// Every earlier AE.1 test that drives a machine drives a tank, so this one is
/// the controller's own proof that it reads the unit's row — in the search, in
/// the heuristic's scale, and in the routeless follower's openness tests
/// (`first_step_is_open`, `way_ahead_is_open`). On level windows priced per row
/// (`ArenaTerrain::priced`), each machine patrolling at one node at (40, 3):
///
/// * **routed**, the doctrine `x6_the_route_goes_round_what_motion_refuses`
///   runs its edges on (think every 250 ms, re-plan whenever the route is
///   gone) and the nimble helm: a walker plans *through* a thick-woods band
///   across the whole window, which a tank's row has no route over at all; and
///   a car plans *round* a rubble box by the gap on rows 6 and 7 (a road on row
///   7): rubble is no ground of its row, so its route never enters a cell
///   motion would refuse it. Neither search fails and neither machine is
///   stopped;
/// * **routeless** (`path_max_cells` 1, so every search fails on its budget and
///   the machine steers straight at the node): the walker's one-cell test lets
///   it into the woods and it walks through them; the car's refuses it the
///   rubble wall across the window, so it is held short of it — no `blocked`
///   line, and an empty avoid list;
/// * and the car's first search expands exactly what `path_with` expands at
///   the **wheeled** row's own scale (80, the road), which is not what it
///   expands at the tracked 100 — the reflex scales its heuristic on the unit's
///   row.
///
/// Each of the four scenes is replayed at 5,000, 1,000 and 500 ms, and the
/// three replays are one event list, one hash sequence and one set of finals.
#[test]
fn ae1_a_machine_walker_and_a_machine_car_steer_on_their_own_rows_at_every_cadence() {
    use tacsim::arena::CostRow;
    const UNTIL: i64 = 60_000;

    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let routed = ArenaConfig {
        reflex: tacsim::config::ReflexConfig {
            decide_ms: 250,
            repath_ms: 0,
            ..tacsim::config::ReflexConfig::default()
        },
        ..ArenaConfig::default()
    };
    let routeless = ArenaConfig {
        reflex: tacsim::config::ReflexConfig {
            path_max_cells: 1,
            ..tacsim::config::ReflexConfig::default()
        },
        ..ArenaConfig::default()
    };
    let woods: &[PricedBox] = &[((20, 27, 0, 7), TerrainClass::WoodsHeavy)];
    let rubble_box: &[PricedBox] = &[
        ((20, 27, 0, 5), TerrainClass::Rubble),
        ((0, 47, 7, 7), TerrainClass::Road),
    ];
    let rubble_wall: &[PricedBox] = &[((20, 27, 0, 7), TerrainClass::Rubble)];
    let start = (2_u16, 3_u16);
    let node = (40_u16, 3_u16);
    let open = |entry: &str, paint: &[PricedBox], cfg: &ArenaConfig| {
        let mut definition = priced_window(paint);
        definition.waypoints = relief_waypoint(node);
        arena_over(
            &catalog,
            vec![nimble(under_reflex(blind(unit_of(
                &catalog,
                entry,
                RED,
                Team::Red,
                start,
            ))))],
            definition,
            cfg,
        )
    };

    // One scene at three cadences: the same history, and where it ended.
    let three = |entry: &str, paint: &[PricedBox], cfg: &ArenaConfig, row: CostRow| {
        let mut runs: Vec<(i64, ReliefReplay, (u16, u16), bool)> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = open(entry, paint, cfg);
            assert_eq!(arena.unit(RED).map(|unit| unit.cost_row), Some(row));
            let run = relief_replay(&mut arena, RED, cadence, UNTIL, &[]);
            assert_eq!(arena.now_ms, UNTIL, "cadence {cadence} ran short");
            // 17b cut M1a.1: the avoid list is the hull's motor row's.
            let avoid_empty = arena
                .motors
                .row(RED)
                .is_some_and(|motor| motor.avoid.is_empty());
            runs.push((cadence, run, standing(&arena, RED).0, avoid_empty));
        }
        let Some((_, first, cell, avoid_empty)) = runs.first() else {
            panic!("three replays");
        };
        for (cadence, run, _, _) in runs.iter().skip(1) {
            assert_eq!(
                run.events, first.events,
                "{entry}: the event list differs at {cadence} ms"
            );
            assert_eq!(
                run.hashes,
                first.hashes,
                "{entry}: the per-substep hash sequence differs at {cadence} ms ({} against {})",
                run.hashes.len(),
                first.hashes.len()
            );
            assert_eq!(
                run.finals, first.finals,
                "{entry}: a final pose, z, grade or speed differs at {cadence} ms"
            );
        }
        let stops = first
            .events
            .iter()
            .filter(|(_, kind, _, _)| kind.starts_with("{\"kind\":\"blocked\""))
            .count();
        (
            first.reflex.path_queries,
            first.reflex.path_failures,
            stops,
            *cell,
            *avoid_empty,
        )
    };

    // Routed: the walker plans through the woods.
    let (queries, failures, stops, cell, _) =
        three(content::BIPED_MK1, woods, &routed, CostRow::Legged);
    assert!(queries > 0, "the walker asked for no route");
    assert_eq!(
        failures, 0,
        "the walker's search found no way over the woods"
    );
    assert_eq!(stops, 0, "the walker was stopped");
    assert!(
        cell.0 >= 28,
        "the walker never got past the woods: {cell:?}"
    );

    // Routed: the car plans round the rubble.
    let (queries, failures, stops, cell, _) =
        three(content::SCOUT_MK1, rubble_box, &routed, CostRow::Wheeled);
    assert!(queries > 0, "the car asked for no route");
    assert_eq!(
        failures, 0,
        "the car's search found no way round the rubble"
    );
    assert_eq!(stops, 0, "the car's route took it into the rubble");
    assert!(cell.0 >= 28, "the car never got past the rubble: {cell:?}");

    // Routeless: the walker's one-cell test lets it into the woods.
    let (_, failures, stops, cell, _) =
        three(content::BIPED_MK1, woods, &routeless, CostRow::Legged);
    assert!(failures > 0, "the walker was never routeless");
    assert_eq!(stops, 0, "the routeless walker was stopped");
    assert!(
        cell.0 >= 28,
        "the routeless walker was held at the woods: {cell:?}"
    );
    // And it is never held on the way: the look-ahead (`way_ahead_is_open`)
    // asks every cell within the walker's stopping distance on the walker's
    // row, so it slows for the woods once, on the ground's own price, and walks
    // through them at `1,000 × 100 / 200`. Asked on a tank's row, the woods two
    // cells ahead hold it, it brakes, is let go, speeds up and is held again.
    let mut arena = open(content::BIPED_MK1, woods, &routeless);
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let near = speeds_in(&mut arena, &mut wall, 120, RED, 14..=19, &mut bytes);
    let mut arena = open(content::BIPED_MK1, woods, &routeless);
    let mut wall = 0_i64;
    let deep = speeds_in(&mut arena, &mut wall, 120, RED, 22..=26, &mut bytes);
    assert!(
        !near.is_empty() && !deep.is_empty(),
        "the routeless walker never came up to the woods and through them"
    );
    assert!(
        near.windows(2)
            .all(|pair| pair.get(1).copied() <= pair.first().copied()),
        "the routeless walker was held short of the woods and let go: {near:?}"
    );
    assert!(
        deep.iter().all(|speed| *speed == 1_000 * 100 / 200),
        "the routeless walker was held in the woods: {deep:?}"
    );

    // Routeless: the car's refuses it the rubble, and it is held short of it.
    let (_, failures, stops, cell, avoid_empty) = three(
        content::SCOUT_MK1,
        rubble_wall,
        &routeless,
        CostRow::Wheeled,
    );
    assert!(failures > 0, "the car was never routeless");
    assert_eq!(stops, 0, "the routeless car drove into the rubble");
    assert!(
        cell.0 > start.0 && cell.0 < 20,
        "the routeless car is not held short of the rubble: {cell:?}"
    );
    assert!(avoid_empty, "a held car learnt a stop it never had");

    // The heuristic's scale: the car's first search, from its opening cell,
    // expands what the wheeled row's own scale expands and not what the
    // tracked row's would.
    let definition = {
        let mut definition = priced_window(rubble_box);
        definition.waypoints = relief_waypoint(node);
        definition
    };
    let terrain = definition.terrain.clone();
    assert_eq!(
        (
            terrain.min_cost(CostRow::Tracked),
            terrain.min_cost(CostRow::Wheeled)
        ),
        (100, 80)
    );
    let expanded = |scale: u32| {
        tacsim::arena::path_with(
            &terrain,
            &mut tacsim::arena::PathScratch::default(),
            start,
            node,
            tacsim::config::ReflexConfig::default().path_max_cells,
            scale,
            Some(25),
            CostRow::Wheeled,
            &[],
        )
        .map(|found| u64::from(found.expanded))
        .expect("the car's route round the rubble")
    };
    let (own, tracked) = (expanded(80), expanded(100));
    assert_ne!(
        own, tracked,
        "the two scales expand alike here, so this window cannot tell them apart"
    );
    let mut arena = open(content::SCOUT_MK1, rubble_box, &ArenaConfig::default());
    let mut wall = 0_i64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut first = None;
    // The machine starts its engine first (ten seconds), and searches once it
    // is online, from the cell it opened on.
    for _ in 0..60 {
        let counters = run_logged(&mut arena, &mut wall, 500, 1, &mut bytes);
        if counters.path_queries > 0 {
            first = Some(counters);
            break;
        }
    }
    let first = first.expect("the car searched within thirty seconds");
    assert_eq!(
        standing(&arena, RED).0,
        start,
        "it searched from its opening cell"
    );
    assert_eq!(first.path_queries, 1, "one search in the pulse");
    assert_eq!(
        first.path_cells, own,
        "the reflex scaled the car's search on another row's cheapest cell"
    );
}

// ---------------------------------------------------------------------------
// AE.2 — the autocannon, and the missile with a flight time (16c N15)
// ---------------------------------------------------------------------------

/// **The two weapons' and their rounds' digest pins** (16b cut AE.2; 16c N15),
/// the way X.1 pins the laser's, in `requires` order: the 30 mm round, the
/// autocannon, the missile round and the rack.
const DIGESTS_ORDNANCE: [(&str, &str); 4] = [
    (
        "tacsim.pack.ap_30",
        "8704ce199381569e8e9171eabc31143fe10f0f0d85f7755b5a3afa164b47d394",
    ),
    (
        "tacsim.pack.autocannon_30",
        "1aee8939d0ee05e780919a5c1b06abdb455c0af6bc9fc6b15512bd15190f8d35",
    ),
    (
        "tacsim.pack.missile_round",
        "902758dc2f22fa442a81828f30f9718f9aa35dd785801f83a0856919bb8f372a",
    ),
    (
        "tacsim.pack.missile_lr",
        "2437c519457287ec4d5ac2fba9a54e1da973f6619f7db6f088fe0694fae851a4",
    ),
];

/// The ordnance's pack files, by their paths under `content/`.
const PACK_FILES_ORDNANCE: [&str; 4] = [
    "ammunition/ap_30.json",
    "weapons/autocannon_30.json",
    "ammunition/missile_round.json",
    "weapons/missile_lr.json",
];

/// **The two carriers 16b cut AE.2 moves to version 2**, re-signed by the same
/// writing branch; their pins are [`DIGESTS_CHASSIS`]' first two rows.
const PACK_FILES_CARRIERS: [&str; 2] = ["units/scout_mk1.json", "units/heavy_mk1.json"];

/// **The ordnance loads beside the stock game without moving it, and the two
/// carriers take it at version 2** (16b cut AE.2; 16c N15; doc 16 §9's rule
/// read for a table of the fielded catalog).
///
/// X.1's shape, one table: the four pins and the writing branch, which re-signs
/// the scout car and the heavy too; the carriers at version 2, requiring the
/// autocannon and the rack at version 1, the Mk2 still at 1; the stock game is
/// the two weapons, two rounds, one unit and one arena it always was, and every
/// stock definition keeps its identity; the fielded catalog holds five weapons,
/// five rounds and eleven unit schemas (ten at AE.2; AE.3's drone is the
/// eleventh); [`tacsim::definitions::WeaponDef::check`]
/// accepts both new weapons; and each is read off its carrier exactly as the
/// production check-out reads it, in component order — so the scout car's bare
/// `FIRE` is its autocannon and the heavy's is still its cannon, by the one
/// rule HEAD always had. The rack's flight at the ranges a sight allows is
/// pinned here too: 2,000 ms at 500 m, 2,400 at a sight's edge, 3,600 on a
/// teammate's plot at 900 m, and the 6,000 of the arena-level test's 1.5 km.
#[test]
fn ae2_the_ordnance_loads_beside_the_stock_game_without_moving_it() {
    use tacsim::arena::state::WeaponSummary;

    if writing() {
        for relative in PACK_FILES_ORDNANCE.iter().chain(PACK_FILES_CARRIERS.iter()) {
            resign(relative);
        }
        return;
    }

    // The pins.
    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_ORDNANCE.into_iter().collect();
    for (name, json) in content::EXTENSIONS_ORDNANCE {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        assert_eq!(pack.manifest().version(), 1, "{id} is version 1");
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_ORDNANCE.len());

    // The carriers at version 2, requiring the new weapons; the Mk2 at 1 —
    // and, since 17b cut M3.1, each one version more for its core: the
    // carriers at 3, the Mk2 at 2 (were 2, 2 and 1).
    let chassis: std::collections::BTreeMap<&str, &str> = DIGESTS_CHASSIS.into_iter().collect();
    for (json, id, version, requires) in [
        (
            content::SCOUT_MK1_JSON,
            "tacsim.pack.scout_mk1",
            3,
            Some("tacsim.pack.autocannon_30"),
        ),
        (
            content::HEAVY_MK1_JSON,
            "tacsim.pack.heavy_mk1",
            3,
            Some("tacsim.pack.missile_lr"),
        ),
        (content::BIPED_MK2_JSON, "tacsim.pack.biped_mk2", 2, None),
    ] {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{id} does not verify: {error}"));
        assert_eq!(pack.manifest().id().as_str(), id);
        assert_eq!(pack.manifest().version(), version, "{id}'s version");
        assert_eq!(
            chassis.get(id).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        if let Some(weapon) = requires {
            assert!(
                pack.manifest()
                    .requires()
                    .iter()
                    .any(|row| row.id.as_str() == weapon && row.version == 1),
                "{id} does not require {weapon}@1"
            );
        }
    }

    // The stock game is what it was.
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.ammunition.len(), 2, "the 105 and the ball");
    assert_eq!(stock.specs.len(), 1, "one unit schema");

    // The fielded catalog: five weapons, five rounds, eleven unit schemas (ten
    // at AE.2; AE.3's drone is the eleventh), and no
    // stock identity moved.
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let ids = |keys: Vec<&CanonicalId>| -> Vec<String> {
        keys.into_iter().map(|id| id.as_str().to_owned()).collect()
    };
    assert_eq!(
        ids(fielded.weapons.keys().collect()),
        vec![
            content::AUTOCANNON_30.to_owned(),
            "tacsim.weapon.cannon_105".to_owned(),
            content::LASER_MK1.to_owned(),
            "tacsim.weapon.mg_coax".to_owned(),
            content::MISSILE_LR.to_owned(),
        ],
        "the cannon, the coaxial, the laser, the autocannon and the rack"
    );
    assert_eq!(
        ids(fielded.ammunition.keys().collect()),
        vec![
            "tacsim.ammunition.ap_30".to_owned(),
            "tacsim.ammunition.ball_762".to_owned(),
            "tacsim.ammunition.he_105".to_owned(),
            "tacsim.ammunition.laser_charge".to_owned(),
            "tacsim.ammunition.missile_round".to_owned(),
        ]
    );
    assert_eq!(
        fielded.specs.len(),
        11,
        "no unit schema of its own: the ten AE.1 left and 16b cut AE.3's drone"
    );
    for (id, reference) in &stock.references {
        assert_eq!(
            fielded.references.get(id),
            Some(reference),
            "{id} moved when the ordnance loaded; a persisted unit points at it"
        );
    }

    // `WeaponDef::check` accepts both, which is every bound the loader holds a
    // weapon to — a floor-legal cycle, outward bands, a direct weapon's speed.
    for id in [content::AUTOCANNON_30, content::MISSILE_LR] {
        let def = fielded
            .weapons
            .get(&CanonicalId::new(id).expect("a canonical id"))
            .expect("the weapon is fielded");
        def.check(id)
            .unwrap_or_else(|error| panic!("{id} is refused: {error}"));
        assert_eq!(
            def.max_range_cm(),
            if id == content::MISSILE_LR {
                300_000
            } else {
                200_000
            }
        );
    }
    // The longest reach fielded is still the cannon's 3 km, which is what
    // `tempo.alert_range_cm` is derived from: the rack's outer edge is equal
    // to it on purpose.
    assert_eq!(
        fielded
            .weapons
            .values()
            .map(tacsim::definitions::WeaponDef::max_range_cm)
            .max(),
        Some(300_000)
    );

    // Each carrier's weapons as the production check-out reads them, in
    // component order.
    let summaries_of = |entry: &str| -> Vec<WeaponSummary> {
        let key = CanonicalId::new(entry).expect("a canonical id");
        let spec = fielded.specs.get(&key).expect("the carrier is fielded");
        let binding = fielded.bindings.get(&key).expect("its binding");
        let cells = tacsim::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(31))
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
        tacsim::match_::weapon_summaries(&fielded, &cells)
    };
    let names = |rows: &[WeaponSummary]| -> Vec<String> {
        rows.iter().map(|weapon| weapon.id.to_string()).collect()
    };
    let scout = summaries_of(content::SCOUT_MK1);
    assert_eq!(names(&scout), vec!["autocannon", "machine_gun"]);
    let heavy = summaries_of(content::HEAVY_MK1);
    assert_eq!(names(&heavy), vec!["cannon", "machine_gun", "missiles"]);
    let find = |rows: &[WeaponSummary], id: &str| -> WeaponSummary {
        rows.iter()
            .find(|weapon| weapon.id.as_str() == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id}"))
    };
    let autocannon = find(&scout, "autocannon");
    assert_eq!(
        (
            autocannon.burst_rounds,
            autocannon.cycle_ms,
            autocannon.damage,
            autocannon.count_per_load,
            autocannon.muzzle_speed_cms,
            autocannon.bands_cm(),
        ),
        (3, 3_000, 20, 240, 105_000, vec![100_000, 200_000])
    );
    assert_eq!(
        autocannon.flight_ms(60_000),
        571,
        "0.57 s at a sight's edge"
    );
    let rack = find(&heavy, "missiles");
    assert_eq!(
        (
            rack.burst_rounds,
            rack.cycle_ms,
            rack.damage,
            rack.count_per_load,
            rack.muzzle_speed_cms,
            rack.bands_cm(),
        ),
        (1, 10_000, 350, 6, 25_000, vec![60_000, 300_000])
    );
    assert!(
        !rack.is_beam(),
        "a rack is a direct weapon with a flight time"
    );
    assert_eq!(
        [50_000_u64, 60_000, 90_000, 150_000].map(|range| rack.flight_ms(range)),
        [2_000, 2_400, 3_600, 6_000],
        "500 m, a sight's edge, a teammate's plot, and the arena test's 1.5 km"
    );
}

/// **An arena of one shooter and one line tank, both awake, the shooter locked
/// on the tank through a contact planted by hand** (16b cut AE.2) — the shape
/// `damage.rs`'s own tests take, because what is under test is the shot and not
/// how the platform came to hold its target (at 1.5 km no sight would).
fn ae2_duel(
    catalog: &Catalog,
    shooter: &str,
    shooter_cell: (u16, u16),
    target_cell: (u16, u16),
    definition: ArenaDefinition,
    reflex: bool,
) -> Arena {
    let red = unit_of(catalog, shooter, RED, Team::Red, shooter_cell);
    let red = if reflex { under_reflex(red) } else { red };
    let mut arena = arena_over(
        catalog,
        vec![
            red,
            unit_of(catalog, content::LINE_TANK, BLUE, Team::Blue, target_cell),
        ],
        definition,
        &ArenaConfig::default(),
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
    arena
        .resolve_lock(RED, contact, arena.now_ms)
        .expect("the lock holds");
    arena
}

/// **Make one shot in flight certain to land** (16b cut AE.2): its recorded
/// chance at the impact's own clamp ceiling and past it. The chance is not what
/// these tests are about — the flight, the re-validation and the launch point
/// are — and a per-mille draw a fixture would have to hunt a seed for is the
/// wrong thing to pin them on. The impact reads the recorded resolution for a
/// target that has not turned (doc 16 §3.5), so this is what it uses.
fn ae2_certain(arena: &mut Arena, seq: u64) {
    if let Some(shot) = arena.shots.get_mut(&seq) {
        shot.resolution.p_permille = 1_000;
    }
}

/// One shot's outcome off an impact's journal: `Ok((section, facet, rounds,
/// damage))` for a hit, `Err(reason)` for a miss.
fn ae2_outcome(
    journal: &[tacsim::arena::perception::Notice],
) -> Result<(String, String, u32, u32), String> {
    use tacsim::arena::ledger::EventKind;
    journal
        .iter()
        .find_map(|(_, kind, _)| match kind {
            EventKind::Hit {
                section,
                facet,
                rounds,
                damage,
                ..
            } => Some(Ok((
                section.to_string(),
                facet.to_string(),
                *rounds,
                *damage,
            ))),
            EventKind::Miss { reason, .. } => Some(Err(reason.to_string())),
            _ => None,
        })
        .unwrap_or_else(|| Err(format!("no outcome in {journal:?}")))
}

/// **Three rounds a pull, each its own draw** (16b cut AE.2; doc 16 §3.5 step
/// 4's burst): the scout car's autocannon spends three rounds of its 240 and
/// starts its 3,000 ms cycle; how many land is one binomial sample drawn once
/// under `burst:<seq>` — a count from nought to three, and every count reachable
/// as the chance moves; every landed round carries 20 into one `hit` line
/// through `apply_hit`; and a last pull with two rounds left fires the two.
#[test]
fn ae2_the_autocannon_is_a_burst_of_three_and_each_round_is_its_own_draw() {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let mut arena = ae2_duel(
        &catalog,
        content::SCOUT_MK1,
        RED_CELL,
        BLUE_CELL,
        definition(&[]),
        false,
    );
    let at = arena.now_ms;
    let seq = arena
        .resolve_fire(RED, "autocannon", at)
        .expect("the burst is accepted");
    let state = arena.unit(RED).expect("the car");
    assert_eq!(state.weapon_ammo("autocannon"), 237, "three of 240 spent");
    assert_eq!(state.weapon_ready_at_ms("autocannon"), at + 3_000);
    assert_eq!(
        state.weapon_ammo("machine_gun"),
        2_000,
        "the coaxial is its own magazine"
    );
    let shot = arena.shot(seq).cloned().expect("the burst in flight");
    assert_eq!((shot.rounds, shot.damage), (3, 20));

    // One draw, under `burst:<seq>`, and a count.
    let (label, value) = arena.shot_draw(seq, shot.rounds);
    assert_eq!(label, format!("burst:{seq}"));
    assert_eq!(
        arena.shot_draw(seq, shot.rounds),
        (label, value),
        "drawn once"
    );
    let landed = arena.rounds_landed(seq, 3, shot.resolution.p_permille);
    assert!(landed <= 3);
    assert_eq!(
        arena.rounds_landed(seq, 3, shot.resolution.p_permille),
        landed,
        "the same count every time it is asked"
    );
    let counts: std::collections::BTreeSet<u32> = (0..=1_000_u32)
        .map(|p| arena.rounds_landed(seq, 3, p))
        .collect();
    assert_eq!(
        counts,
        std::collections::BTreeSet::from([0, 1, 2, 3]),
        "every round is its own trial: what lands is a count, not all or nothing"
    );

    // Every landed round is 20, in one `hit` line.
    ae2_certain(&mut arena, seq);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.resolve_impact(seq, shot.impact_at_ms, &mut journal);
    let (_, _, rounds, damage) = ae2_outcome(&journal).expect("three certain rounds land");
    assert_eq!((rounds, damage), (3, 60));
    let draws: Vec<String> = journal
        .iter()
        .find(|(_, kind, _)| matches!(kind, tacsim::arena::ledger::EventKind::Hit { .. }))
        .map(|(_, _, draws)| draws.iter().map(|(label, _)| label.clone()).collect())
        .unwrap_or_default();
    assert_eq!(
        draws,
        vec![format!("burst:{seq}"), format!("facet:{seq}")],
        "the burst's own draw, then the facet's: no new label"
    );

    // A last pull with two rounds left fires the two.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("autocannon"), 2);
        state.ready_at_ms.insert(SmolStr::new("autocannon"), 0);
    }
    let last = arena
        .resolve_fire(RED, "autocannon", arena.now_ms)
        .expect("the last two are fired");
    assert_eq!(arena.shot(last).map(|shot| shot.rounds), Some(2));
    assert_eq!(arena.shot_draw(last, 2).0, format!("burst:{last}"));
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("autocannon")),
        Some(0)
    );
    ae2_certain(&mut arena, last);
    let impact = arena.shot(last).map(|shot| shot.impact_at_ms).unwrap_or(0);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.resolve_impact(last, impact, &mut journal);
    let (_, _, rounds, damage) = ae2_outcome(&journal).expect("two certain rounds land");
    assert_eq!((rounds, damage), (2, 40));
    if let Some(state) = arena.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("autocannon"), 0);
    }
    assert_eq!(
        arena.resolve_fire(RED, "autocannon", arena.now_ms),
        Err(Refusal::Empty)
    );
}

/// The 1.5 km corridor the missile test flies down (16b cut AE.2): the heavy at
/// row 2, the tank at row 152, 150 cells of flat ground between — a range no
/// sight on this map holds, which is why the contact is planted.
const AE2_LAUNCH: (u16, u16) = (8, 2);
/// Where the tank stands, 150 cells north of the launch.
const AE2_MARK: (u16, u16) = (8, 152);

/// **The missile lands where the target was** (16b cut AE.2; doc 16 §3.5
/// "Scheduled impact"): no guidance and no mechanism of its own, a longer
/// schedule and a due event.
///
/// At 1.5 km the heavy's rack is 6,000 ms in the air — the impact due at the
/// fired millisecond plus 6,000 and the rack ready again at plus 10,000 — and
/// at a sight's edge, 600 m, 2,400. The impact re-validates within one cell of
/// where the target was: one that never moved takes the 350 at the impact
/// millisecond, one moved by one cell still takes it, and one moved by two is a
/// `miss` for `moved`. And the target's pilot is told nothing between the
/// `fired` and the `impact` millisecond: no incoming warning, no bearing, no
/// weapon — the first thing it reads is its own hull being hit.
#[test]
fn ae2_the_missile_lands_where_the_target_was() {
    use tacsim::arena::ledger::EventKind;
    use tacsim::arena::state::DueKind;
    const TARGET_PILOT: mush_core::ObjId = mush_core::ObjId(1_001);

    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let build = || {
        ae2_duel(
            &catalog,
            content::HEAVY_MK1,
            AE2_LAUNCH,
            AE2_MARK,
            q6_window(16, 160),
            false,
        )
    };

    // The schedule.
    let mut arena = build();
    let rack = arena
        .unit(RED)
        .and_then(|state| state.weapon("missiles").cloned())
        .expect("the heavy carries the rack");
    assert_eq!(rack.flight_ms(60_000), 2_400, "a sight's edge");
    let at = arena.now_ms;
    let seq = arena
        .resolve_fire(RED, "missiles", at)
        .expect("the missile is away");
    let shot = arena.shot(seq).cloned().expect("the missile in flight");
    assert_eq!(shot.range_cm, 150_000, "1.5 km");
    assert_eq!(shot.fired_at_ms, at);
    assert_eq!(shot.impact_at_ms, at + 6_000, "250 m/s over 1.5 km");
    assert_eq!((shot.rounds, shot.damage), (1, 350));
    let state = arena.unit(RED).expect("the heavy");
    assert_eq!(state.weapon_ammo("missiles"), 5);
    assert_eq!(state.weapon_ready_at_ms("missiles"), at + 10_000);
    assert_eq!(
        state.weapon_ammo("cannon"),
        40,
        "the rack is its own magazine"
    );
    let due: Vec<(i64, DueKind)> = arena
        .due
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                DueKind::Impact { .. } | DueKind::WeaponReady { .. }
            )
        })
        .map(|event| (event.t_ms, event.kind.clone()))
        .collect();
    assert_eq!(
        due,
        vec![
            (at + 6_000, DueKind::Impact { shot_seq: seq }),
            (
                at + 10_000,
                DueKind::WeaponReady {
                    unit: RED,
                    weapon: SmolStr::new("missiles"),
                }
            ),
        ]
    );

    // The three outcomes, each flown through `Arena::step`.
    for (cells_moved, expected) in [
        (0_i64, Ok(350_u32)),
        (1, Ok(350)),
        (2, Err(tacsim::arena::damage::MOVED.to_owned())),
    ] {
        let mut arena = build();
        let seq = arena
            .resolve_fire(RED, "missiles", arena.now_ms)
            .expect("the missile is away");
        ae2_certain(&mut arena, seq);
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose.x_cm += cells_moved * i64::from(CELL_CM);
        }
        let mut bytes: Vec<u8> = Vec::new();
        let mut told: Vec<(i64, Vec<FrameNotice>)> = Vec::new();
        let mut wall = 0_i64;
        let out = arena.step(0, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        while arena.now_ms < 7_000 {
            wall += 500;
            let out = arena.step(wall, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            for (pilot, frame) in &out.frames {
                if *pilot == TARGET_PILOT && !frame.notices.is_empty() {
                    told.push((arena.now_ms, frame.notices.clone()));
                }
            }
            // **16b cut AP.2**: a missile that takes the tank's turret takes
            // its side's only gun, and a side that came in armed and has no
            // gun left is out — the match ends at the impact's substep, and an
            // ended arena's clock stands, so the loop stops there.
            if out.ended.is_some() {
                break;
            }
        }
        let events = ledger_of(&mut arena, &mut bytes);
        let outcome = events.iter().find_map(|event| match &event.kind {
            EventKind::Hit {
                shot_seq, damage, ..
            } if *shot_seq == seq => Some((event.t_ms, Ok(*damage))),
            EventKind::Miss { shot_seq, reason } if *shot_seq == seq => {
                Some((event.t_ms, Err(reason.to_string())))
            }
            _ => None,
        });
        assert_eq!(
            outcome,
            Some((6_000, expected.clone())),
            "moved {cells_moved} cell(s): the outcome, at the impact millisecond"
        );
        // Nothing reaches the target's pilot before the impact millisecond; a
        // hit is then its own hull and nothing else.
        assert!(
            told.iter().all(|(t_ms, _)| *t_ms >= 6_000),
            "the target was told something while the round was in the air: {told:?}"
        );
        match expected {
            Ok(_) => assert!(
                matches!(
                    told.first().and_then(|(_, notices)| notices.first()),
                    Some(FrameNotice::Struck { .. })
                ),
                "the hit is the first thing its pilot reads: {told:?}"
            ),
            Err(_) => assert!(told.is_empty(), "a miss tells its target nothing: {told:?}"),
        }
    }
}

/// **The round is traced from where it left** (16b cut AE.2; doc 16 §3.5
/// "Scheduled impact", §6's `no line` edge), on a 16 by 16 window with a 400 cm
/// ridge across row 7 and an 800 cm crest at (8, 3) south of it.
///
/// From the crest the heavy's 170 cm eye sees over the ridge to the tank at (8,
/// 11); from the flat cell (8, 2) behind it, it does not. So:
///
/// 1. a machine that fires from the crest and backs off it before the impact
///    still **hits** — the line is the launch eye's, and `fire_line` from where it
///    ends is blocked — and is **not** taught `no line` about a line it no
///    longer has;
/// 2. a machine that fires from behind the ridge and stays there reads
///    `blocked`, and **is** taught (the guard is not a switch that turned the
///    edge off);
/// 3. a machine that fires from behind the ridge and then climbs the crest
///    still reads `blocked` — the round never left the crest — and is not
///    taught;
/// 4. a shooter removed from the match mid-flight (a test's case only;
///    production never removes a unit from a running arena) resolves its round
///    through the stored eye: a hit from the crest, `blocked` from behind;
/// 5. on flat ground the facet struck is the one the target presented to the
///    launch point (its rear, to a shooter due south), not the one it presents
///    to where the shooter ended (its right, due east).
#[test]
fn ae2_the_round_is_traced_from_where_it_left() {
    const CREST: (u16, u16) = (8, 3);
    const BEHIND: (u16, u16) = (8, 2);
    const MARK: (u16, u16) = (8, 11);
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let ridged = || {
        relief(16, 16, &|x, y| {
            if y == 7 {
                400
            } else if (x, y) == CREST {
                800
            } else {
                0
            }
        })
    };
    // Fire the rack from `from`, then (optionally) put the machine at `to`,
    // and land the round: the outcome and whether the edge was fed.
    let fly = |from: (u16, u16), to: Option<(u16, u16)>, remove: bool| {
        let mut arena = ae2_duel(&catalog, content::HEAVY_MK1, from, MARK, ridged(), true);
        assert!(
            arena.reflex.unit(RED).is_some_and(|row| !row.no_line),
            "the machine starts with no lesson"
        );
        let seq = arena
            .resolve_fire(RED, "missiles", arena.now_ms)
            .expect("a shot is accepted whatever the line: no free probe");
        ae2_certain(&mut arena, seq);
        if let Some(cell) = to
            && let Some(state) = arena.unit_mut(RED)
        {
            state.pose = pose(cell);
        }
        if remove {
            arena.units.retain(|unit| unit.id != RED);
        }
        let present = arena
            .unit(RED)
            .map(|_| arena.fire_line(RED, BLUE).0.is_clear_or_partial());
        let impact = arena.shot(seq).map(|shot| shot.impact_at_ms).unwrap_or(0);
        let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
        arena.resolve_impact(seq, impact, &mut journal);
        let taught = arena.reflex.unit(RED).is_some_and(|row| row.no_line);
        (ae2_outcome(&journal), present, taught)
    };
    let blocked = Some(tacsim::arena::damage::BLOCKED);

    // (1) From the crest, then behind it: the round hits, though the line from
    // where the machine now stands is blocked, and nothing is learnt.
    let (outcome, present, taught) = fly(CREST, Some(BEHIND), false);
    assert!(
        outcome.is_ok(),
        "the round that cleared the crest: {outcome:?}"
    );
    assert_eq!(
        present,
        Some(false),
        "the line from behind the crest is blocked"
    );
    assert!(!taught, "a machine that has moved on is taught nothing");

    // (2) From behind the ridge, staying: blocked, and taught.
    let (outcome, present, taught) = fly(BEHIND, None, false);
    assert_eq!(outcome.err().as_deref(), blocked);
    assert_eq!(present, Some(false));
    assert!(
        taught,
        "a machine still where it fired from learns the line it had"
    );

    // (3) From behind the ridge, then up on the crest: still blocked — the
    // round never left the crest — and not taught.
    let (outcome, present, taught) = fly(BEHIND, Some(CREST), false);
    assert_eq!(outcome.err().as_deref(), blocked);
    assert_eq!(present, Some(true), "the crest sees the tank now");
    assert!(!taught);

    // (4) A shooter removed mid-flight: through the stored eye either way.
    let (outcome, present, _) = fly(CREST, None, true);
    assert_eq!(present, None, "the shooter is gone");
    assert!(outcome.is_ok(), "the crest's round lands: {outcome:?}");
    let (outcome, _, _) = fly(BEHIND, None, true);
    assert_eq!(outcome.err().as_deref(), blocked);

    // (5) The face turned to the launch point. The tank faces north; the
    // machine fires from due south of it (its rear) and ends due east of it
    // (its right).
    let mut arena = ae2_duel(
        &catalog,
        content::HEAVY_MK1,
        (8, 3),
        MARK,
        relief(16, 16, &|_x, _y| 0),
        false,
    );
    let seq = arena
        .resolve_fire(RED, "missiles", arena.now_ms)
        .expect("the missile is away");
    assert_eq!(
        arena.shot(seq).map(|shot| shot.resolution.band.to_string()),
        Some("rear".to_owned()),
        "fired at the tank's rear"
    );
    ae2_certain(&mut arena, seq);
    if let Some(state) = arena.unit_mut(RED) {
        state.pose = pose((15, 11));
    }
    let (Some(target), Some(shooter)) = (arena.unit(BLUE), arena.unit(RED)) else {
        panic!("both are in the match");
    };
    assert_eq!(
        tacsim::arena::resolution::facet_band(target, shooter),
        "right",
        "where the machine ended, the tank shows its right"
    );
    let impact = arena.shot(seq).map(|shot| shot.impact_at_ms).unwrap_or(0);
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    arena.resolve_impact(seq, impact, &mut journal);
    let (_, facet, _, damage) = ae2_outcome(&journal).expect("the certain round lands");
    assert_eq!(
        (facet.as_str(), damage),
        ("rear", 350),
        "the round struck the face turned to where it left"
    );
}

// ---------------------------------------------------------------------------
// AE.3 — one drone under W2's cap (16c N15; doc 16 §2 W2)
// ---------------------------------------------------------------------------

/// **The drone's digest pin** (16b cut AE.3; 16c N15), the way X.1 pins the
/// laser's.
///
/// **17b cut M3.1** moved it (version 2): a core component, the doctrine core
/// in its body. Was `f97b0a8b…` at version 1.
const DIGESTS_DRONE: [(&str, &str); 1] = [(
    "tacsim.pack.drone_mk1",
    "1bfeca3a34a13eeddc48b8fca7534d8082a21225735053a08eee94624a9d7b13",
)];

/// The drone's pack file, by its path under `content/`.
const PACK_FILES_DRONE: [&str; 1] = ["units/drone_mk1.json"];

/// **The drone loads beside the stock game without moving it, and it is the
/// one unit of class `drone`** (16b cut AE.3; 16c N15; doc 16 §2 W2).
///
/// X.2's shape, one table: the pin and the writing branch; the stock game is
/// the two weapons, one unit and one arena it always was, and the shipped
/// tank keeps [`LINE_TANK_DIGEST`] — the blob gains `class` only when a spec
/// declares one, so no unit authored before the class moves, and every
/// reference the fielded catalog held before the drone joined it is the same
/// reference now. The fielded catalog holds eleven unit schemas; `class_of`
/// reads eight `armour`, two `walker` and the one `drone`; a class word the
/// enum does not name is refused through the loader; the drone binds with one
/// section, no weapon, the visual sight and a 250 cm mast; and
/// [`tacsim::definitions::UnitClass::read`] answers the same class off the
/// bound units that `class_of` answers off the specs, and `Armour` off a
/// profile-less, class-less engine fixture.
#[test]
fn ae3_the_drone_loads_beside_the_stock_game_without_moving_it() {
    use tacsim::definitions::UnitClass;

    if writing() {
        for relative in PACK_FILES_DRONE {
            resign(relative);
        }
        return;
    }

    // The pin.
    let pinned: std::collections::BTreeMap<&str, &str> = DIGESTS_DRONE.into_iter().collect();
    for (name, json) in content::EXTENSIONS_DRONE {
        let pack = tacsim::pack::load_pack(json)
            .unwrap_or_else(|error| panic!("{name} does not verify: {error}"));
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        // 17b cut M3.1: version 2, for its core (was 1).
        assert_eq!(pack.manifest().version(), 2, "{id}'s version");
    }
    assert_eq!(pinned.len(), content::EXTENSIONS_DRONE.len());

    // The stock game is what it was.
    let stock = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(stock.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(stock.specs.len(), 1, "one unit schema");
    assert_eq!(stock.arenas.len(), 1, "one arena");
    let tank = CanonicalId::new(content::LINE_TANK).expect("a canonical id");
    assert_eq!(
        stock
            .references
            .get(&tank)
            .map(|reference| reference.digest().to_hex())
            .as_deref(),
        Some(LINE_TANK_DIGEST),
        "the shipped tank's definition moved: an undeclared class must write nothing"
    );

    // The fielded catalog: eleven unit schemas, and nothing fielded before the
    // drone moved — the same references as the eight earlier tables give alone.
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    assert_eq!(
        fielded.specs.len(),
        11,
        "the line tank, doc 16 section 9's five, the range's target, the three chassis and \
         the drone"
    );
    let earlier: Vec<(&str, &str)> = [
        content::EXTENSIONS.as_slice(),
        content::EXTENSIONS_BIPED.as_slice(),
        content::EXTENSIONS_THERMAL.as_slice(),
        content::EXTENSIONS_ECM.as_slice(),
        content::EXTENSIONS_ECCM.as_slice(),
        content::EXTENSIONS_TARGET.as_slice(),
        content::EXTENSIONS_ORDNANCE.as_slice(),
        content::EXTENSIONS_CHASSIS.as_slice(),
    ]
    .concat();
    let before = catalog_of(&content::extension_sources_of(&earlier));
    assert_eq!(before.specs.len(), 10, "the ten the fielded catalog held");
    for (id, reference) in &before.references {
        assert_eq!(
            fielded.references.get(id),
            Some(reference),
            "{id} moved when the drone loaded; a persisted unit points at it"
        );
    }
    let drone_key = CanonicalId::new(content::DRONE_MK1).expect("a canonical id");
    assert!(!before.specs.contains_key(&drone_key));

    // The class of every fielded spec.
    let mut classes: std::collections::BTreeMap<UnitClass, Vec<String>> =
        std::collections::BTreeMap::new();
    for (id, spec) in &fielded.specs {
        classes.entry(spec.class_of()).or_default().push(
            id.as_str()
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    let mut armour = classes.get(&UnitClass::Armour).cloned().unwrap_or_default();
    armour.sort();
    assert_eq!(
        armour,
        vec![
            "heavy_mk1",
            "line_tank",
            "line_tank_eccm",
            "line_tank_ecm",
            "line_tank_laser",
            "line_tank_thermal",
            "scout_mk1",
            "target_mk1",
        ],
        "eight armoured machines, the scout car's wheels included"
    );
    let mut walkers = classes.get(&UnitClass::Walker).cloned().unwrap_or_default();
    walkers.sort();
    assert_eq!(walkers, vec!["biped_mk1", "biped_mk2"], "both bipeds walk");
    assert_eq!(
        classes.get(&UnitClass::Drone).cloned().unwrap_or_default(),
        vec!["drone_mk1"],
        "the drone alone declares its class"
    );
    for (id, spec) in &fielded.specs {
        assert_eq!(
            spec.class.is_some(),
            *id == drone_key,
            "{id}: only the drone writes a class word"
        );
    }

    // A class word the enum does not name is refused through the loader: the
    // drone's own pack, the word changed and the pack re-signed so the digest
    // is not what refuses it.
    let flier_raw = content::DRONE_MK1_JSON.replace("\"class\": \"drone\"", "\"class\": \"flier\"");
    assert_ne!(flier_raw, content::DRONE_MK1_JSON, "the word was replaced");
    let declared = mush_content::parse_pack(&flier_raw)
        .expect("the pack parses")
        .manifest()
        .digest()
        .to_hex();
    let unsigned = flier_raw.replace(&declared, UNSIGNED);
    let signed = mush_content::parse_pack(&unsigned)
        .expect("the unsigned pack parses")
        .computed_digest()
        .expect("the pack digests")
        .to_hex();
    let flier = unsigned.replace(UNSIGNED, &signed);
    let refusal = match tacsim::pack::load_pack(&flier) {
        Err(error) => error.to_string(),
        Ok(pack) => Catalog::build_game(
            &[LoadedPack {
                pack,
                source: "<test flier>".to_owned(),
            }],
            tacsim::NS,
        )
        .err()
        .map(|error| error.to_string())
        .expect("a flier is refused"),
    };
    assert!(
        refusal.contains("flier"),
        "the refusal names the word it does not know: {refusal}"
    );

    // The drone, bound as production binds it.
    let bound = |entry: &str, id: u32| {
        let key = CanonicalId::new(entry).expect("a canonical id");
        let spec = fielded.specs.get(&key).expect("the unit is fielded");
        let binding = fielded.bindings.get(&key).expect("its binding");
        tacsim::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(id))
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"))
    };
    let drone = bound(content::DRONE_MK1, 31);
    let body = mush_tactics::SectionId::new("body").expect("a section");
    assert!(drone.section(&body).is_some(), "the body");
    for other in ["hull", "turret", "mast"] {
        let id = mush_tactics::SectionId::new(other).expect("a section");
        assert!(drone.section(&id).is_none(), "no {other}: one section");
    }
    assert!(
        tacsim::match_::weapon_summaries(&fielded, &drone).is_empty(),
        "no gun"
    );
    assert!(
        tacsim::match_::sensor_profile(&fielded, &drone).is_some(),
        "the visual sight"
    );
    assert_eq!(tacsim::match_::sensor_height_cm(&fielded, &drone), 250);
    assert_eq!(
        drone.rules().data().get("class"),
        Some(&serde_json::json!("drone")),
        "the declared word rides the blob"
    );
    assert_eq!(UnitClass::read(&drone), UnitClass::Drone);
    let biped = bound(content::BIPED_MK1, 32);
    assert_eq!(
        biped.rules().data().get("class"),
        None,
        "undeclared: derived"
    );
    assert_eq!(UnitClass::read(&biped), UnitClass::Walker);
    assert_eq!(
        UnitClass::read(&bound(content::LINE_TANK, 33)),
        UnitClass::Armour
    );
    assert_eq!(
        UnitClass::read(&bound(content::SCOUT_MK1, 34)),
        UnitClass::Armour
    );
    assert_eq!(
        UnitClass::read(&bare_tactical_unit(mush_core::ObjId(35))),
        UnitClass::Armour,
        "a fixture with no class word and no profile is armour"
    );

    // And it checks out as a drone with no weapon row: never armed.
    let arena = arena_of(
        &fielded,
        vec![
            unit_of(&fielded, content::DRONE_MK1, RED, Team::Red, RED_CELL),
            unit_of(&fielded, content::LINE_TANK, BLUE, Team::Blue, BLUE_CELL),
        ],
        &[],
    );
    let state = arena.unit(RED).expect("the drone");
    assert_eq!(state.class, UnitClass::Drone);
    assert!(state.weapons.is_empty());
    assert!(!state.armed_at_checkout());
    assert_eq!(
        arena.unit(BLUE).map(|unit| unit.class),
        Some(UnitClass::Armour)
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.1 — an operator's core (doc 17 §9, D-UM-33)
// ---------------------------------------------------------------------------

/// **An operator's cores pack**: one core, `house.core.relay`, of class II and
/// NOT governed, signed.
fn house_cores_pack() -> String {
    sign(&format!(
        r#"{{
  "manifest": {{
    "id": "house.pack.cores",
    "version": 1,
    "kind": "rules_definition",
    "digest": "{UNSIGNED}",
    "requires": []
  }},
  "entries": {{
    "house.core.relay": {{
      "kind": "core",
      "def": {{
        "label": "relay core",
        "class": "II",
        "governed": false,
        "note": "An operator's box, for 17b cut M3.1's test."
      }}
    }}
  }}
}}
"#
    ))
}

/// **The shipped line tank re-badged as an operator's unit** naming `core`
/// where the line tank names the line core, and with `extra` spliced in
/// before its power component (a second core, for the refusal), signed.
fn house_tank_pack(core: &str, extra: &str) -> String {
    let named = format!("\"core\": \"{core}\"");
    let raw = content::LINE_TANK_JSON
        .replace("\r\n", "\n")
        .replace("tacsim.pack.line_tank", "house.pack.relay_tank")
        .replace("tacsim.unit.line_tank", "house.unit.relay_tank")
        .replace("\"core\": \"tacsim.core.line\"", &named)
        .replace(
            "          {\n            \"id\": \"power\",",
            &format!("{extra}          {{\n            \"id\": \"power\","),
        );
    assert!(raw.contains(&named), "the core was renamed");
    assert!(raw.contains(extra), "the extra component was spliced in");
    sign(&raw)
}

/// One source loaded as an operator's pack.
fn operator(name: &str, raw: &str) -> Result<LoadedPack, String> {
    tacsim::pack::load_pack(raw)
        .map(|pack| LoadedPack {
            pack,
            source: name.to_owned(),
        })
        .map_err(|error| error.to_string())
}

/// **An operator's pack may carry a `core` and a unit naming it binds** (17b
/// cut M3.1; doc 17 §9, D-UM-33): the content kind is a pack kind like any
/// other — loaded, checked, minted as a `core` definition beside the shipped
/// three — and a unit that fits it reads its class and its governed bit off
/// the bound unit ([`tacsim::arena::core::read_core`]), the payload the
/// fitting's record.
#[test]
fn m3_an_operator_pack_carrying_a_core_binds_a_unit_naming_it() {
    use tacsim::arena::core::{CoreClass, read_core};
    let packs = [
        operator("<operator house cores>", &house_cores_pack()).expect("the cores pack loads"),
        operator(
            "<operator relay tank>",
            &house_tank_pack("house.core.relay", ""),
        )
        .expect("the tank pack loads"),
    ];
    let catalog = Catalog::build_game(&packs, tacsim::NS).expect("an operator's core binds");
    assert_eq!(
        catalog.cores.len(),
        4,
        "the shipped three and the operator's"
    );
    let relay = CanonicalId::new("house.core.relay").expect("a canonical id");
    assert_eq!(
        catalog
            .references
            .get(&relay)
            .map(|r| r.key().kind.as_str()),
        Some("core")
    );
    let key = CanonicalId::new("house.unit.relay_tank").expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the unit is loaded");
    let binding = catalog.bindings.get(&key).expect("its binding");
    let unit = tacsim::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(30))
        .expect("it instantiates");
    let core = read_core(&unit).expect("it carries the operator's core");
    assert_eq!(core.class, CoreClass::II);
    assert!(!core.governed, "the operator's governed bit");
}

/// **A unit naming a core no loaded pack declares is refused `Reference`**
/// (17b cut M3.1): the reference check at load names the unit, the kind
/// `core` and the id.
#[test]
fn m3_a_unit_naming_an_undeclared_core_is_refused_by_reference() {
    let packs = [operator(
        "<operator relay tank>",
        &house_tank_pack("house.core.missing", ""),
    )
    .expect("the tank pack loads")];
    let refused =
        Catalog::build_game(&packs, tacsim::NS).expect_err("an undeclared core is refused");
    assert_eq!(
        refused,
        tacsim::definitions::DefinitionError::Reference {
            entry: "house.unit.relay_tank".to_owned(),
            kind: "core".to_owned(),
            id: "house.core.missing".to_owned(),
        }
    );
}

/// **A unit with two cores is refused** (17b cut M3.1; D-UM-33: one core a
/// hull): the pack's own bounds refuse it at load, naming the count.
#[test]
fn m3_a_unit_with_two_cores_is_refused() {
    let second = concat!(
        "          {\n",
        "            \"id\": \"spare_core\",\n",
        "            \"section\": \"hull\",\n",
        "            \"role\": { \"kind\": \"core\", \"core\": \"house.core.relay\" }\n",
        "          },\n",
    );
    let refused = operator(
        "<operator relay tank>",
        &house_tank_pack("house.core.relay", second),
    )
    .err()
    .unwrap_or_else(|| panic!("two cores load"));
    assert!(
        refused.contains("a unit carries at most one core; 2 are declared"),
        "{refused}"
    );
}
