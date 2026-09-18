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
    Pilot, Power, ShadowContact, Team, TerrainCell, UnitState, Waypoints,
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
        ShadowContact {
            target: BLUE,
            class: Classification::Classified { team: Team::Blue },
            first_seen_ms: 0,
            last_seen_ms: 0,
        },
    );
    arena.resolve_lock(RED, contact).expect("the lock holds");
    arena
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
        ShadowContact {
            target: BLUE,
            class: Classification::Classified { team: Team::Blue },
            first_seen_ms: 0,
            last_seen_ms: 0,
        },
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
