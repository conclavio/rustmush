//! **The shipped content, generated and pinned** (16b cut C.2; doc 16 §4
//! "Content packs").
//!
//! Two jobs in one file, because they are the same job:
//!
//! 1. **The generator.** The proving ground's ground is a 256 by 256 authored
//!    class grid and a seeded relief — 65,536 cells of run-length-coded base64
//!    that no human writes by hand. `support::ground::atlas_pack_at` is the
//!    authoring source (16b cut AE.1 moved it there from this file), and
//!    [`the_shipped_atlas_pack_is_this_generators_output`] asserts the file in
//!    the tree is exactly what it produces, so the bytes are reproducible and a
//!    hand edit is a failing test rather than a mystery.
//! 2. **The pins.** Every shipped pack's declared digest is its content's, and
//!    every digest is written down here, so a content change is a reviewed diff
//!    of a hex string and not a silent re-signing.
//!
//! Run with `TACSIM_WRITE_CONTENT=1` to rewrite the generated file and re-sign
//! every manifest in place; run without it — which is what CI does — to assert.
//! The digests below still have to be updated by hand, which is the point.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mush_atlas::TerrainClass;
use mush_core::ObjId;
use mush_testkit::TestWorld;
use tacsim::arena::state::Team;
use tacsim::definitions::{Catalog, ResourceTable, content};
use tacsim::match_::{PODS_PER_SIDE, PROVING_GROUND as PROVING_GROUND_ARENA};

mod support;

// ---------------------------------------------------------------------------
// The map
// ---------------------------------------------------------------------------

// **16b cut AE.1** moved the generator — the class grid, the relief, the two
// quadrant encoders and the pack itself — into `tests/support/ground.rs`, where
// the shelf test in `tests/match_lifecycle.rs` can build the version-1 ground it
// boots on. What this file asserts is unchanged: the shipped atlas pack is the
// generator's output (version 2, with The Bluff), byte for byte.
use support::ground::{
    BLUFF_RAMP, BLUFF_TOP, BLUFF_TOP_CM, BUILDINGS, FORD, RIVER, ROAD_EW, ROAD_NS, SIDE, UNSIGNED,
    atlas_pack_at, class_at, elev_at, sign, within,
};

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content")
}

fn writing() -> bool {
    std::env::var("TACSIM_WRITE_CONTENT").is_ok_and(|v| v == "1")
}

/// Assert `path` holds `want`, or write it when `TACSIM_WRITE_CONTENT=1`.
fn pin_file(relative: &str, want: &str) {
    let path = content_dir().join(relative);
    if writing() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the content directory");
        }
        std::fs::write(&path, want).expect("the content file is writable");
        return;
    }
    let have = std::fs::read_to_string(&path).expect("the content file exists");
    assert_eq!(
        have.replace("\r\n", "\n"),
        want.replace("\r\n", "\n"),
        "{relative} is not this generator's output; re-run with TACSIM_WRITE_CONTENT=1"
    );
}

/// Every shipped pack file, by its path under `content/`.
const PACK_FILES: [&str; 9] = [
    "ammunition/he_105.json",
    "ammunition/ball_762.json",
    "weapons/cannon_105.json",
    "weapons/mg_coax.json",
    "sensors/visual_mk1.json",
    // 17b cut M3.1: the cores pack, ahead of the line tank that requires it.
    "cores/cores.json",
    "units/line_tank.json",
    "resolution.json",
    "arenas/proving_ground.json",
];

/// **The digest pins.** A content change moves one of these, and moving one is a
/// reviewed diff.
///
/// **17b cut M3.1** added the cores pack and moved the line tank's, for a core
/// component (the line core in the turret; version 3). Was `607a1d6c…` at
/// version 2. **17b cut M3.6** moved the line tank's again for the core
/// consequence words (`destroy_core` on the turret, and its note's M.4
/// sentence shortened to fit the 1,024-byte bound; version 4). Was
/// `2cdf945e…` at version 3.
const DIGESTS: [(&str, &str); 10] = [
    (
        "tacsim.pack.he_105",
        "c377ca63e4385d29ed4ce63c78fcd1c9e626e1944da4c2aca819c09b6acb67e2",
    ),
    (
        "tacsim.pack.ball_762",
        "59f3da8fbc7d0f656c8b9b94bd06106fa9e071b52109cebc101af0f087697bab",
    ),
    (
        "tacsim.pack.cannon_105",
        "87aec3ba2c8cf8aeefc50212c6f76762ca667d04785c3bb3dd3acf930dedf7d9",
    ),
    (
        "tacsim.pack.mg_coax",
        "d5a24fd7275500b239367b1da521c13a67ec8b767b800a8f9c7cf3797437766c",
    ),
    (
        "tacsim.pack.visual_mk1",
        "78776cb2738a1c210cd55f9a2aba94b42eaf1754750195772eab66f58c65d32e",
    ),
    (
        "tacsim.pack.cores",
        "3ad7f6910ec8ba53e82a93b852b3d82ef771206516b084e3238696a675233903",
    ),
    (
        "tacsim.pack.line_tank",
        "2d80e63eecf5167fc8d803746d507ae861db25c4d6d76dcdbe2fd5c1c4bff2b8",
    ),
    (
        "tacsim.pack.resolution",
        "aa298a115be0a5a242b907a5acf7007274e44170c02483600819523253ad331e",
    ),
    (
        "tacsim.pack.proving_ground",
        "74f4bd85dd91d4afaa78d21f45d2d19be5d02b50b0fb71dd40bcc2d7939d8017",
    ),
    (
        "tacsim.proving_ground",
        "9be6c634353c82e576be61395ef2228a927ab3d47ae952708b174e4e6f41097d",
    ),
];

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_atlas_pack_is_this_generators_output() {
    pin_file("atlas/proving_ground.json", &atlas_pack_at(true));
}

#[test]
fn every_shipped_pack_carries_its_own_digest() {
    // The hand-authored packs are signed in place; the generated one signs
    // itself. Both go through the same `sign`, so "shipped" and "signed" are one
    // statement.
    for relative in PACK_FILES {
        let path = content_dir().join(relative);
        let raw = std::fs::read_to_string(&path).expect("the content file exists");
        if writing() {
            // Re-sign **in place**: the declared digest's own hex is swapped for
            // the placeholder and then for the computed value, so a
            // hand-authored file keeps its authored formatting and the diff of a
            // re-sign is one line.
            let declared = mush_content::parse_pack(&raw)
                .expect("the pack parses")
                .manifest()
                .digest()
                .to_hex();
            let unsigned = raw.replace(&declared, UNSIGNED);
            std::fs::write(&path, sign(&unsigned)).expect("the content file is writable");
            continue;
        }
        tacsim::pack::load_pack(&raw)
            .unwrap_or_else(|error| panic!("{relative} does not verify: {error}"));
    }
}

#[test]
fn the_pinned_digests_are_the_shipped_ones() {
    if writing() {
        // The pins are updated by hand on purpose; print them and move on.
        for (name, json) in content::SOURCES {
            let pack = mush_content::parse_pack(json).expect("parses");
            println!(
                "    (\"{}\", \"{}\"), // {name}",
                pack.manifest().id(),
                pack.manifest().digest().to_hex()
            );
        }
        let atlas = mush_content::parse_pack(content::PROVING_GROUND_ATLAS_JSON).expect("parses");
        println!(
            "    (\"{}\", \"{}\"), // the atlas pack",
            atlas.manifest().id(),
            atlas.manifest().digest().to_hex()
        );
        return;
    }
    let pinned: BTreeMap<&str, &str> = DIGESTS.into_iter().collect();
    let mut seen = 0;
    for (_, json) in content::SOURCES {
        let pack = tacsim::pack::load_pack(json).expect("a shipped pack verifies");
        let id = pack.manifest().id().to_string();
        assert_eq!(
            pinned.get(id.as_str()).copied(),
            Some(pack.manifest().digest().to_hex().as_str()),
            "{id}'s digest moved; re-review the content and re-pin it"
        );
        seen += 1;
    }
    let atlas = mush_content::load_pack(content::PROVING_GROUND_ATLAS_JSON)
        .expect("the shipped atlas pack verifies");
    assert_eq!(
        pinned
            .get(atlas.manifest().id().to_string().as_str())
            .copied(),
        Some(atlas.manifest().digest().to_hex().as_str()),
        "the atlas pack's digest moved"
    );
    seen += 1;
    assert_eq!(seen, DIGESTS.len(), "every pin names a shipped pack");
}

#[test]
fn the_authored_map_is_the_map_the_arena_entry_describes() {
    // The five key spots, the entry zones and every waypoint must stand on
    // ground a tracked unit can enter, or the arena is a pack that cannot start
    // a match. This is the one property the JSON cannot assert about itself.
    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let passable = |x: u16, y: u16| table.passable(class_at(x, y).label(), "tracked", &[]);
    for spot in &arena.key_spots {
        assert!(
            passable(spot.x, spot.y),
            "the key spot `{}` is on {:?}, which a tracked unit cannot enter",
            spot.name,
            class_at(spot.x, spot.y)
        );
    }
    for node in &arena.waypoints.nodes {
        assert!(
            passable(node.x, node.y),
            "waypoint `{}` is on {:?}",
            node.id,
            class_at(node.x, node.y)
        );
    }
    for tank in &arena.tanks {
        assert!(
            passable(tank.x, tank.y),
            "`{}` opens on {:?}",
            tank.id,
            class_at(tank.x, tank.y)
        );
    }
    for zone in &arena.entry_zones {
        for y in zone.y0..=zone.y1 {
            for x in zone.x0..=zone.x1 {
                assert!(
                    passable(x, y),
                    "the {} entry zone contains {x},{y}, which is {:?}",
                    zone.team.as_str(),
                    class_at(x, y)
                );
            }
        }
    }
}

#[test]
fn the_ford_is_the_only_way_across_the_river() {
    // The tactical claim the map makes: a tracked unit crossing from the red
    // side to the blue side goes through the ford or does not go.
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut crossings = 0;
    for y in 0..SIDE {
        let mut clear = true;
        for x in RIVER.0..=RIVER.1 {
            if !table.passable(class_at(x, y).label(), "tracked", &[]) {
                clear = false;
            }
        }
        if clear {
            crossings += 1;
        }
    }
    assert_eq!(
        crossings,
        usize::from(FORD.1 - FORD.0 + 1),
        "the river is crossable on exactly the ford's rows"
    );
}

#[test]
fn no_building_stands_on_a_road() {
    for building in BUILDINGS {
        for y in building.2..=building.3 {
            for x in building.0..=building.1 {
                assert!(
                    !((y >= ROAD_EW.0 && y <= ROAD_EW.1) || (x >= ROAD_NS.0 && x <= ROAD_NS.1)),
                    "a compound covers the road at {x},{y}"
                );
            }
        }
    }
}

#[test]
fn the_ten_commodity_families_decode() {
    let table = ResourceTable::load("resources.json", content::RESOURCES_JSON)
        .expect("the shipped resource table decodes");
    assert_eq!(table.instruments().len(), 10);
    for family in tacsim::definitions::resources::FAMILIES {
        assert!(
            table.resources.iter().any(|row| row.family == family),
            "{family} is missing"
        );
    }
}

/// **Which of the four band words the shipped content can actually print** (16b
/// cut F.3).
///
/// `CONTACTS`' band is read against the authored bands of the weapon a bare
/// `FIRE` fires, held down to the platform's own sight
/// (`cockpit::contact_band`), and what a pilot *can* be shown is therefore a
/// fact about two packs at once: the cannon's band edges and how far a sight can
/// hold a contact. Cut S.2's band was read against the platform's longest reach
/// and could only ever say `close`.
///
/// **16b cut E.1** deleted `range_band`, the commander's second distance (16c
/// `T-81`), so this test now asks the one function there is — and on the shipped
/// content all four words are reachable: the sight is 600 m, a mesh hop carries
/// a plot to 900 m, and `beyond` is what a plot past the cannon's 3 km outer
/// band would read (which nothing on this map can be, so the set below is three
/// words and the fourth is pinned by its own assertion).
///
/// The shape of the gap is **recorded here rather than argued here**: whether
/// the sight should reach further or the bands should sit closer is content,
/// which is the owner's. What this test is for is that it cannot change silently
/// — widen the sight, shorten the cannon's inner band, or give the tank a
/// different default weapon, and this fails and somebody reads the note.
#[test]
fn the_band_word_the_shipped_content_can_print() {
    use tacsim::match_::cockpit::{DEFAULT_WEAPON, contact_band};

    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let cell_cm = u64::from(arena.cell_cm);
    // The stock sight, and the furthest a contact can be held at: that sight plus
    // one mesh hop off a teammate at the far edge of its own reach.
    let sight_cm = catalog
        .sensors
        .values()
        .map(|sensor| u64::from(sensor.max_range_cells).saturating_mul(cell_cm))
        .max()
        .expect("the shipped sights");
    let furthest_cm = catalog
        .sensors
        .values()
        .map(|sensor| {
            u64::from(sensor.max_range_cells)
                .saturating_add(u64::from(sensor.share_range_cells))
                .saturating_mul(cell_cm)
        })
        .max()
        .expect("the shipped sights");
    assert_eq!(sight_cm, 60_000, "600 m of scope");
    assert_eq!(furthest_cm, 90_000, "a 600 m sight and a 300 m mesh hop");
    let sight = u32::try_from(sight_cm).expect("600 m fits");

    // The weapon a bare `FIRE` fires, found the way `UnitState::default_weapon`
    // finds it: by component name.
    let bands: Vec<u32> = catalog
        .weapons
        .iter()
        .find(|(id, _)| id.as_str().contains(DEFAULT_WEAPON))
        .map(|(_, weapon)| weapon.bands.iter().map(|band| band.max_range_cm).collect())
        .expect("the stock tank's cannon is shipped");
    assert_eq!(bands, vec![150_000, 300_000], "the cannon's authored edges");

    // Every range a real detection can be at, at the metre, and the words they
    // produce. Three of the four, and each at a range two tanks on this map will
    // routinely be at.
    let words: std::collections::BTreeSet<&str> = (0..=furthest_cm)
        .step_by(100)
        .map(|range_cm| contact_band(range_cm, &bands, sight))
        .collect();
    assert_eq!(
        words.into_iter().collect::<Vec<_>>(),
        vec!["close", "effective", "far"],
        "the band vocabulary a pilot can see on the shipped proving ground"
    );
    assert_eq!(contact_band(15_000, &bands, sight), "close", "150 m");
    assert_eq!(contact_band(30_000, &bands, sight), "effective", "300 m");
    assert_eq!(contact_band(51_000, &bands, sight), "far", "510 m");
    // And `beyond` is what a plot past the cannon's own outer band would read —
    // which nothing on this map can be, so it never prints.
    assert_eq!(contact_band(300_001, &bands, sight), "beyond");
}

/// **The ranges the shipped machines fight at** (16b cut E.1; 16c `T-46`, Q9).
///
/// What the reflex controller prices every distance off is
/// `UnitState::fight_inner_cm` and `UnitState::fight_limit_cm` — the lesser of
/// the platform's own **nominal sight** and, respectively, the **first band** and
/// the whole reach of the weapon it would actually fire. So this pins that table
/// for every unit schema the game fields, loaded and with the default weapon's
/// magazine emptied, and it pins it on units built the way production builds them
/// (`match_::custody_unit`) rather than on the JSON read twice.
///
/// It is deliberately **not** a check of each weapon's outer reach, which no
/// controller distance reads, and it is **not** an invariant that a first band
/// reaches the sight: the shipped coaxial's first band is 600 m against a 600 m
/// sight, exactly nought margin, and a future short gun would make its machine
/// close further, which is this cut working rather than a violation.
///
/// Move a sight, a first band or a default weapon and this fails — and with it
/// the figure `docs/help/simulator.md` quotes to a pilot in "Fighting the
/// machines": *about seven tenths of what its own sight and its gun's inner band
/// allow (420 m for the stock tank)*.
///
/// **16b cut O.3** (16c N13) fields a seventh schema, the range's target, which
/// carries no weapon and no sight: its row is `(0, 0, 0)` — the sight and both
/// reaches answer nought with nothing mounted and nothing loaded ("then nothing
/// is in range of it") — the dry half skips it (it has no default weapon to
/// empty), and the closing loop expects it to have nothing to fall back to.
///
/// **16b cut AE.1** (16c N15) fields three more: the heavy, whose row is the
/// stock tank's; the Mk2 biped, whose thermal head holds both its reaches to 450
/// m loaded and dry; and the scout car, whose one gun was the coaxial, so
/// emptying it left a unit that still **saw** 600 m (the sight is the
/// sensor's alone) and fought at nothing — `(60_000, 0, 0)`.
///
/// **16b cut AE.2** gave the scout car its autocannon, first in component
/// order: that is its bare `FIRE` and the one emptied, and dry it falls to its
/// coaxial as every tank does — `(60_000, 60_000, 60_000)`.
///
/// **16b cut AE.3** fields an eleventh, the drone: a 60-cell visual sight and
/// no gun, so it **sees** 600 m and fights at nothing — `(60_000, 0, 0)`,
/// loaded and dry alike — and it has no default weapon to empty (a `None` row
/// in the dry half, as the target's).
#[test]
fn e1_the_ranges_the_shipped_machines_fight_at() {
    use mush_core::ObjId;
    use smol_str::SmolStr;
    use tacsim::arena::state::{Arena, Controller, CustodyOut, Team, WeaponSummary};
    use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
    use tacsim::match_::{ARENA_CELL_CM, STAND_IN, custody_unit, stock_definition};

    let catalog =
        tacsim::definitions::Catalog::build_fielded(&[], "tacsim").expect("the fielded content");
    assert_eq!(
        catalog.specs.len(),
        11,
        "the shipped unit schemas: the line tank, doc 16 §9's five extensions, \
         the range's target, 16b cut AE.1's three chassis and 16b cut AE.3's drone"
    );
    let definition = stock_definition(&SmolStr::new_static(STAND_IN)).expect("the stand-in window");
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .expect("the engine's own passive posture");
    let half = i64::from(ARENA_CELL_CM / 2);

    // One of each, two cells apart so no two share a cell (doc 16 §3.4 step 5).
    let mut names: Vec<String> = Vec::new();
    let mut units = Vec::new();
    for (at, (entry, spec)) in catalog.specs.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        let binding = catalog
            .bindings
            .get(entry)
            .unwrap_or_else(|| panic!("{entry} binds"));
        let cells = tacsim::definitions::unit::to_tactical_unit(spec, binding, id)
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
        let column = i64::try_from(at).unwrap_or(0) * 2 + 2;
        let pose = mush_sim::data::Pose {
            x_cm: column * i64::from(ARENA_CELL_CM) + half,
            y_cm: 4 * i64::from(ARENA_CELL_CM) + half,
            z_cm: 0,
            heading_mdeg: 90_000,
            posture: posture.clone(),
        };
        names.push(entry.as_str().to_owned());
        units.push(
            custody_unit(
                &catalog,
                cells,
                id,
                Team::Red,
                pose,
                0,
                Controller::Reflex,
                0,
                &tacsim::match_::SealTerms::fixture(),
            )
            .expect("the fixture terms seal"),
        );
    }
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("one of every shipped unit checks out");

    // `(schema, sight_cm, fight_inner_cm, fight_limit_cm)`, loaded. Everything
    // the game fields carries a 60-cell visual sight except the thermal tank,
    // whose channel sees 45 — and on every one of them the sight is the lesser,
    // which is the whole of cut E.1's Q9 ruling.
    let table = |arena: &Arena| -> Vec<(String, u32, u64, u64)> {
        names
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
                let Some(state) = arena.unit(id) else {
                    panic!("{name} is in the arena")
                };
                (
                    name.clone(),
                    state.sight_cm(ARENA_CELL_CM),
                    state.fight_inner_cm(ARENA_CELL_CM),
                    state.fight_limit_cm(ARENA_CELL_CM),
                )
            })
            .collect()
    };
    assert_eq!(
        table(&arena),
        vec![
            ("tacsim.unit.biped_mk1".to_owned(), 60_000, 60_000, 60_000),
            // 16b cut AE.1: the Mk2 sees with the thermal sight's 45 cells, and
            // its beam's 1.2 km first band is past it.
            ("tacsim.unit.biped_mk2".to_owned(), 45_000, 45_000, 45_000),
            // 16b cut AE.3: the drone sees 600 m and carries no gun, so nothing
            // is in its reach.
            ("tacsim.unit.drone_mk1".to_owned(), 60_000, 0, 0),
            ("tacsim.unit.heavy_mk1".to_owned(), 60_000, 60_000, 60_000),
            ("tacsim.unit.line_tank".to_owned(), 60_000, 60_000, 60_000),
            (
                "tacsim.unit.line_tank_eccm".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_ecm".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_laser".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_thermal".to_owned(),
                45_000,
                45_000,
                45_000
            ),
            // 16b cut AE.2: the scout's gun is the autocannon, whose first band
            // is 1 km, past its 600 m sight.
            ("tacsim.unit.scout_mk1".to_owned(), 60_000, 60_000, 60_000),
            // 16b cut O.3: no sight and nothing loaded, so nothing is in range.
            ("tacsim.unit.target_mk1".to_owned(), 0, 0, 0),
        ],
        "the reaches the shipped machines fight at, loaded"
    );

    // The stock tank's doctrine band, which is the number the help quotes.
    let engage_cm = u64::from(ReflexConfig::default().engage_band_permille) * 60_000 / 1_000;
    assert_eq!(
        engage_cm, 42_000,
        "`docs/help/simulator.md` says 420 m for the stock tank"
    );

    // Two structural claims, before the dry half.
    for (at, name) in names.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        let Some(state) = arena.unit(id) else {
            panic!("{name} is in the arena")
        };
        for weapon in &state.weapons {
            let edges = weapon.bands_cm();
            assert!(!edges.is_empty(), "{name}: {} has no band", weapon.id);
            assert!(
                edges.windows(2).all(|pair| pair[0] < pair[1]),
                "{name}: {}'s bands do not run outward: {edges:?}",
                weapon.id
            );
            assert_eq!(
                weapon.inner_band_cm(),
                edges.first().copied().unwrap_or(0),
                "{name}: {}'s inner band is `bands_cm().first()`",
                weapon.id
            );
        }
        // Whatever `release_cm` admits, `engage_orders`' own `max_range_cm >=
        // range` filter admits the loaded weapon — `T-46`'s proof at content
        // scale.
        let reach = state
            .loaded_weapon()
            .map_or(0, |weapon| u64::from(weapon.max_range_cm));
        assert!(
            state.fight_inner_cm(ARENA_CELL_CM) <= reach,
            "{name}: the band it closes to is past the gun it would fire"
        );
        assert!(state.fight_limit_cm(ARENA_CELL_CM) <= reach);
    }
    // 16b cut O.3: the target passes that loop with nothing in it — no weapon
    // row to band, nothing loaded, and both reaches nought — which is pinned
    // here rather than left to an empty loop.
    let target_at = names
        .iter()
        .position(|name| name == "tacsim.unit.target_mk1")
        .expect("the range's target is fielded");
    let target_id = ObjId(u32::try_from(target_at).unwrap_or(0) + 10);
    let target = arena.unit(target_id).expect("the target is in the arena");
    assert!(target.weapons.is_empty() && target.loaded_weapon().is_none());
    assert_eq!(
        (
            target.fight_inner_cm(ARENA_CELL_CM),
            target.fight_limit_cm(ARENA_CELL_CM)
        ),
        (0, 0)
    );

    // And again with the weapon a bare `FIRE` fires emptied. Every shipped
    // armed unit carries a second gun, so it keeps fighting — at its coaxial's
    // own first band, which on this content is 600 m and therefore exactly the
    // sight: nought margin, and the day a shorter gun is authored this table is
    // where it shows. **16b cut AE.2**: that includes the scout car, whose
    // autocannon is its bare `FIRE` and whose coaxial is now a second gun (in
    // 16b cut AE.1 the coaxial was its only gun, and emptied it fought at
    // nothing). **16b cut O.3**: a unit with no default weapon — the range's
    // target — has nothing to empty and is skipped.
    let dry: Vec<Option<SmolStr>> = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
            let Some(state) = arena.unit(id) else {
                panic!("{name} is in the arena")
            };
            state.default_weapon().map(|weapon| weapon.id.clone())
        })
        .collect();
    for (at, weapon) in dry.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        if let Some(weapon) = weapon
            && let Some(state) = arena.unit_mut(id)
        {
            state.ammo.insert(weapon.clone(), 0);
        }
    }
    assert_eq!(
        dry.iter()
            .map(|weapon| weapon.as_ref().map(SmolStr::to_string))
            .collect::<Vec<_>>(),
        vec![
            Some("cannon".to_owned()),
            Some("laser".to_owned()),
            // 16b cut AE.3: the drone carries no gun.
            None,
            Some("cannon".to_owned()),
            Some("cannon".to_owned()),
            Some("cannon".to_owned()),
            Some("cannon".to_owned()),
            Some("laser".to_owned()),
            Some("cannon".to_owned()),
            Some("autocannon".to_owned()),
            None,
        ],
        "the weapon a bare `FIRE` fires: the cannon, or the first in component \
         order on a hull that carries none (the Mk2's beam, the scout's \
         autocannon), and nothing on the drone or the range's target"
    );
    assert_eq!(
        table(&arena),
        vec![
            ("tacsim.unit.biped_mk1".to_owned(), 60_000, 60_000, 60_000),
            // 16b cut AE.1: the Mk2 falls from its beam to its coaxial, held to
            // its 45-cell sight as the thermal tank is.
            ("tacsim.unit.biped_mk2".to_owned(), 45_000, 45_000, 45_000),
            // 16b cut AE.3: nothing to empty, so the drone's row is its loaded one.
            ("tacsim.unit.drone_mk1".to_owned(), 60_000, 0, 0),
            ("tacsim.unit.heavy_mk1".to_owned(), 60_000, 60_000, 60_000),
            ("tacsim.unit.line_tank".to_owned(), 60_000, 60_000, 60_000),
            (
                "tacsim.unit.line_tank_eccm".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_ecm".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_laser".to_owned(),
                60_000,
                60_000,
                60_000
            ),
            (
                "tacsim.unit.line_tank_thermal".to_owned(),
                45_000,
                45_000,
                45_000
            ),
            // 16b cut AE.2: the scout's autocannon emptied, it falls to its
            // coaxial as every tank does (in 16b cut AE.1, with the coaxial its
            // only gun, this row was `(60_000, 0, 0)`).
            ("tacsim.unit.scout_mk1".to_owned(), 60_000, 60_000, 60_000),
            // 16b cut O.3: no sight and nothing loaded, so nothing is in range.
            ("tacsim.unit.target_mk1".to_owned(), 0, 0, 0),
        ],
        "and with the default weapon dry: every one with a second gun fights on its \
         coaxial"
    );
    for (at, name) in names.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        let Some(state) = arena.unit(id) else {
            panic!("{name} is in the arena")
        };
        let loaded = state
            .loaded_weapon()
            .map(|weapon: &WeaponSummary| weapon.id.to_string());
        // 16b cut O.3: a unit with no weapon has nothing to fall to. **16b cut
        // AE.1**: and a unit whose only weapon was the emptied one has nothing to
        // fall to either (no fielded unit is one since 16b cut AE.2 gave the
        // scout its autocannon). A unit that still has a loaded weapon falls to
        // `machine_gun`.
        let emptied = dry.get(at).cloned().flatten();
        let expected = state
            .weapons
            .iter()
            .any(|weapon| Some(&weapon.id) != emptied.as_ref())
            .then(|| "machine_gun".to_owned());
        assert_eq!(
            loaded, expected,
            "{name}: a dry default weapon falls to the next loaded one"
        );
    }
}

/// **The muster is on the ford road, and a straight drive at the enemy reaches
/// the ford** (16b cut F.4; arena pack version 2).
///
/// Version 1 mustered both sides on rows 118 to 130. The river runs down columns
/// 62 to 66 and is `water_deep` — impassable to a tracked unit — everywhere but
/// the ford's rows, so a pilot who pointed at the enemy and drove was stopped
/// after about 550 m of grass, on their own bank, with the only crossing sixteen
/// to twenty-eight rows away and outside `VIEW`'s twenty-row window. Both slot-1
/// tanks did it, in opposite directions, and neither pilot could see why.
///
/// The first pass of this cut fixed that for slot 1 alone: rows 97, 93, 101 and
/// 89 put slot 4 of each side back outside the ford band (92 to 102) and into
/// the same trap, and only slot 1's run was asserted. The ford is eleven rows
/// wide and the muster needed four, so **all eight** of version 2's slots stood in it, and
/// the run asserted below is **every** slot's.
///
/// **16b cut Z.3** adds slots 5 and 6 a side (the arena pack's version 3): row
/// 101 in the muster column and row 94 one column behind it. Six rows a side do
/// not fit the ford one clear row apart in one column, so the spacing rule below
/// is the one it always stood for — a tank pulling away has a clear cell beside
/// it — read as **a clear cell between any two slots of a side** (no two within
/// one cell of each other in either axis), and no two slots of a side share a
/// row, so every straight run east or west is one slot's own.
///
/// Every assertion below is read off the pack's own data rather than written
/// down twice, and every slot is found by the id the roster keys on
/// (`red_1` … `blue_6`, walked over `1..=PODS_PER_SIDE`) rather than by where it
/// happens to sit in the array, so the next person who edits the map cannot put
/// a muster back behind the river without this failing.
#[test]
fn the_muster_stands_on_the_road_and_the_run_at_the_enemy_reaches_the_ford() {
    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let passable = |x: u16, y: u16| table.passable(class_at(x, y).label(), "tracked", &[]);

    // (1) Every slot cell is passable ground inside its own team's entry zone.
    for tank in &arena.tanks {
        let zone = arena
            .entry_zones
            .iter()
            .find(|zone| zone.team == tank.team)
            .unwrap_or_else(|| panic!("`{}` has no entry zone for its side", tank.id));
        assert!(
            tank.x >= zone.x0 && tank.x <= zone.x1 && tank.y >= zone.y0 && tank.y <= zone.y1,
            "`{}` musters at {},{} outside its own {} entry zone",
            tank.id,
            tank.x,
            tank.y,
            tank.team.as_str()
        );
        assert!(
            passable(tank.x, tank.y),
            "`{}` musters on {:?}",
            tank.id,
            class_at(tank.x, tank.y)
        );
    }

    // (2) No two slots share a cell, and no slot stands on a waypoint node — a
    //     patrol that starts on top of its own first node has nowhere to go.
    let mut cells: BTreeMap<(u16, u16), String> = BTreeMap::new();
    for tank in &arena.tanks {
        if let Some(other) = cells.insert((tank.x, tank.y), tank.id.to_string()) {
            panic!("`{}` and `{other}` muster on the same cell", tank.id);
        }
        for node in &arena.waypoints.nodes {
            assert!(
                (node.x, node.y) != (tank.x, tank.y),
                "`{}` musters on waypoint `{}`",
                tank.id,
                node.id
            );
        }
    }

    // (3) Every slot faces its opposite number along one row, and the straight
    //     run along that row from one to the other is passable the whole way —
    //     which is to say it reaches the river's one crossing and goes over it.
    //     By id, never by array order: the roster binds `red_1` to red slot 1.
    let slot = |id: &str| {
        arena
            .tanks
            .iter()
            .find(|tank| tank.id.as_str() == id)
            .unwrap_or_else(|| panic!("`{id}` is not in the roster"))
    };
    for index in 1..=PODS_PER_SIDE {
        let red = slot(&format!("red_{index}"));
        let blue = slot(&format!("blue_{index}"));
        assert_eq!(
            red.y, blue.y,
            "the two slot-{index} tanks do not face along one row"
        );
        assert!(
            red.y >= FORD.0 && red.y <= FORD.1,
            "slot {index} musters on row {}, outside the ford's rows {} to {}, so its straight \
             run at the enemy ends at deep water",
            red.y,
            FORD.0,
            FORD.1
        );
        for x in red.x..=blue.x {
            assert!(
                passable(x, red.y),
                "slot {index}'s run along row {} is stopped at column {x} by {:?}",
                red.y,
                class_at(x, red.y)
            );
        }
    }

    // Slot 1 stands on the east-west road itself, which is what makes the
    // opening approach a road march rather than a crawl over grass.
    let red_one = slot("red_1");
    assert!(
        red_one.y >= ROAD_EW.0 && red_one.y <= ROAD_EW.1,
        "slot 1 musters on row {}, which is not the east-west road",
        red_one.y
    );

    // And a tank pulling away has a clear cell beside it: no two slots of a side
    // within one cell of each other in either axis (16b cut Z.3 put slot 6 one
    // column behind the others, so this is read over cells and not rows alone),
    // and no two on one row, so no slot stands on another's straight run.
    for team in ["red", "blue"] {
        let cells: Vec<(u8, u16, u16)> = (1..=PODS_PER_SIDE)
            .map(|index| {
                let tank = slot(&format!("{team}_{index}"));
                (index, tank.x, tank.y)
            })
            .collect();
        for (i, (a, ax, ay)) in cells.iter().enumerate() {
            for (b, bx, by) in cells.iter().skip(i + 1) {
                assert!(
                    ax.abs_diff(*bx) >= 2 || ay.abs_diff(*by) >= 2,
                    "the {team} muster puts slots {a} ({ax},{ay}) and {b} ({bx},{by}) side by \
                     side"
                );
                assert_ne!(
                    ay, by,
                    "{team} slots {a} and {b} share row {ay}, so one stands on the other's run"
                );
            }
        }
    }

    // (4) Both muster nodes are inside their side's entry zone, on ground a tank
    //     can stand on, and off every slot cell.
    let node_of = |id: &str| {
        arena
            .waypoints
            .nodes
            .iter()
            .find(|node| node.id.as_str() == id)
            .unwrap_or_else(|| panic!("`{id}` is not in the graph"))
    };
    for (id, team) in [("muster_red", "red"), ("muster_blue", "blue")] {
        let node = node_of(id);
        let zone = arena
            .entry_zones
            .iter()
            .find(|zone| zone.team.as_str() == team)
            .unwrap_or_else(|| panic!("no {team} entry zone"));
        assert!(
            node.x >= zone.x0 && node.x <= zone.x1 && node.y >= zone.y0 && node.y <= zone.y1,
            "`{id}` is at {},{} outside the {team} entry zone",
            node.x,
            node.y
        );
        assert!(
            passable(node.x, node.y),
            "`{id}` is on {:?}",
            class_at(node.x, node.y)
        );
        assert!(
            !arena
                .tanks
                .iter()
                .any(|tank| (tank.x, tank.y) == (node.x, node.y)),
            "`{id}` is on a slot's own cell"
        );
    }

    // (5) And the graph is one graph: every node is reachable from `muster_red`,
    //     which is where a red tank's `Patrol` starts. A muster node with no edge
    //     is a tank that holds where it is for ever.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut queue = vec!["muster_red"];
    while let Some(node) = queue.pop() {
        if !seen.insert(node) {
            continue;
        }
        for edge in &arena.waypoints.edges {
            if edge[0].as_str() == node {
                queue.push(edge[1].as_str());
            } else if edge[1].as_str() == node {
                queue.push(edge[0].as_str());
            }
        }
    }
    assert_eq!(
        seen.len(),
        arena.waypoints.nodes.len(),
        "the waypoint graph is not connected from the red muster: reached {} of {}",
        seen.len(),
        arena.waypoints.nodes.len()
    );
    assert!(
        seen.contains("muster_blue"),
        "and the blue muster is not on it"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.8 — the fielded source list
// ---------------------------------------------------------------------------

/// **`fielded_sources` is `sources` plus the five tables, in `requires` order,
/// and nothing latched moved** (16b cut Q.8) — and, since **16b cut O.3**, a
/// sixth table after them: the range's target, which requires nothing — and,
/// since **16b cut AE.1**, a seventh: the three chassis, whose Mk2 biped
/// requires the laser (the first table) and the thermal sight (the third), both
/// ahead of it — and, since **16b cut AE.2**, an eighth between the target and
/// the chassis: the autocannon, the missile rack and their two rounds, which
/// the scout car's and the heavy's version 2 require — and, since **16b cut
/// AE.3**, a ninth, last: the drone, which requires only the stock sight.
///
/// The order claim is the one worth a test: [`content::SOURCES`] is authored as
/// "a pack after every pack it names", the five extension tables each carry the
/// same promise inside themselves, and the question this answers is whether
/// concatenating them keeps it. It does, because no extension pack requires a
/// pack from another extension table — every one of them requires only the
/// stock cannon, coaxial and sight, plus, for the laser tank, the two laser
/// packs ahead of it in its own table.
#[test]
fn q8_the_fielded_sources_are_the_shipped_ones_and_then_the_five_in_requires_order() {
    let stock = content::sources();
    let fielded = content::fielded_sources();
    // 17b cut M3.1: the stock game is nine packs — the cores pack joined the
    // eight, ahead of the line tank that requires it.
    assert_eq!(
        stock.len(),
        9,
        "nine shipped packs, the cores pack among them"
    );
    assert_eq!(
        fielded.len(),
        stock.len() + 19,
        "nine shipped packs and nineteen extension files: doc 16 section 9's ten, \
         the range's target, 16b cut AE.2's two weapons and their two rounds, \
         16b cut AE.1's three chassis and 16b cut AE.3's drone"
    );
    for (shipped, both) in stock.iter().zip(fielded.iter()) {
        assert_eq!(shipped.name, both.name, "the shipped prefix is unmoved");
        assert_eq!(shipped.bytes, both.bytes);
    }
    for source in &fielded {
        assert!(
            source.name.starts_with("<built-in "),
            "a fielded pack is shipped content, not an operator's file: {}",
            source.name
        );
    }

    // A pack after every pack it names.
    let mut behind: BTreeSet<(String, u32)> = BTreeSet::new();
    for source in &fielded {
        let pack = tacsim::pack::load_pack(&source.bytes)
            .unwrap_or_else(|error| panic!("{} does not verify: {error}", source.name));
        for required in pack.manifest().requires() {
            assert!(
                behind.contains(&(required.id.to_string(), required.version)),
                "{} requires {}@{}, which is not ahead of it",
                source.name,
                required.id,
                required.version
            );
        }
        behind.insert((pack.manifest().id().to_string(), pack.manifest().version()));
    }

    // Nothing is latched by this: the two boot packs are the same two.
    let boot = content::boot_packs().expect("the boot packs verify");
    let ids: Vec<String> = boot
        .iter()
        .map(|pack| pack.manifest().id().to_string())
        .collect();
    assert_eq!(
        ids,
        vec![
            content::PROVING_GROUND_ATLAS_PACK.to_owned(),
            content::PROVING_GROUND_PACK.to_owned(),
        ],
        "16b cut Q.8 latches nothing new"
    );
}

// ---------------------------------------------------------------------------
// 16b cut M.2 (16c `T-05`) — the shipped substep divides the three periods
// ---------------------------------------------------------------------------

/// **Every fielded unit's substep divides all three tempo periods** (16b cut
/// M.2; doc 16 §3.1 as amended, Q24).
///
/// Substeps are cut from each pulse's own start with the remainder last, so the
/// three cadences cut arena time on one set of boundaries only when the shipped
/// substep divides every shipped period — which is why the scripted cross-tempo
/// replays agree substep for substep. A production pulse also ends at due
/// events and at wall jitter, so its boundaries are not period-aligned and rate
/// independence rests on the due-event rule, not on this; the absolute lattice
/// that would make it hold for any unit is not built. What this pins is that no
/// fielded unit — tracked or legged — is fast enough on the shipped 10 m cell to
/// shorten the substep to a length that does not divide them: above 3,996 cm/s
/// the crossing bound goes under 250 ms, and only some shorter lengths divide
/// 500, 1,000 and 5,000.
#[test]
fn m2_every_fielded_units_substep_divides_the_three_periods() {
    let tempo = tacsim::config::TempoConfig::default();
    let catalog = Catalog::build_fielded(&[], "tacsim").expect("the fielded content binds");
    assert!(!catalog.specs.is_empty(), "the fielded catalog has units");
    let mut checked = 0_usize;
    for (entry, spec) in &catalog.specs {
        let modes = spec.mobility.modes();
        assert!(!modes.is_empty(), "{entry} declares a mode");
        for mode in modes {
            let speed = mode.rules.max_speed_cms;
            let substep = tacsim::arena::clock::substep_ms(&tempo, 1_000, speed);
            // 16b cut O.3: the range's target authors a zero ceiling, and a zero
            // ceiling has no cell crossing to bound — it answers the cap.
            if speed == 0 {
                assert_eq!(
                    substep, tempo.substep_max_ms,
                    "{entry}: a zero ceiling answers `substep_max_ms`"
                );
            }
            for (name, period) in [
                ("quiet_ms", tempo.quiet_ms),
                ("alert_ms", tempo.alert_ms),
                ("engaged_ms", tempo.engaged_ms),
            ] {
                assert_eq!(
                    period % substep,
                    0,
                    "{entry}'s {} mode at {speed} cm/s cuts {substep} ms substeps, which do \
                     not divide {name} = {period}",
                    mode.mode
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 11,
        "every fielded unit's modes were read, the three chassis' and the drone's among \
         them: {checked}"
    );
    assert!(
        catalog.specs.contains_key(
            &mush_core::CanonicalId::new(content::TARGET_MK1).expect("a canonical id")
        ),
        "and the zero-ceiling arm above was reached by the range's target"
    );
}

/// **16b cut M.4 (16c `B-47`): every fielded tank's two tracks carry the pair
/// rule.**
///
/// Walks the fielded catalog's unit specs — the stock tank and the four
/// extension tanks — and asserts that each unit with a `left_track` and a
/// `right_track` authors `disable_motion_when_all_of: [left_track, right_track]`
/// on **both** of them, so a tank on no tracks neither turns nor drives
/// whichever track goes last. Five tanks, exactly; the biped's legs are X.2's
/// and are asserted by its own test. **16b cut O.3**: and the range's target,
/// which carries the stock hull's tracks verbatim — both gone is one of the
/// three ways it is out of action.
#[test]
fn m4_every_fielded_tanks_two_tracks_carry_the_pair_rule() {
    use tacsim::definitions::unit::Consequence;

    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let pair = Consequence::DisableMotionWhenAllOf {
        sections: vec![
            mush_core::CanonicalId::new("left_track").expect("a section id"),
            mush_core::CanonicalId::new("right_track").expect("a section id"),
        ],
    };
    let mut tanks: Vec<String> = Vec::new();
    for (id, spec) in &fielded.specs {
        let track = |name: &str| spec.sections.iter().find(|row| row.id.as_str() == name);
        let (Some(left), Some(right)) = (track("left_track"), track("right_track")) else {
            continue;
        };
        for row in [left, right] {
            assert!(
                row.consequences.contains(&pair),
                "{id}'s {} does not carry the pair rule: {:?}",
                row.id,
                row.consequences
            );
        }
        tanks.push(id.as_str().to_owned());
    }
    tanks.sort();
    assert_eq!(
        tanks,
        vec![
            content::HEAVY_MK1,
            content::LINE_TANK,
            content::LINE_TANK_ECCM,
            content::LINE_TANK_ECM,
            content::LINE_TANK_LASER,
            content::LINE_TANK_THERMAL,
            content::TARGET_MK1,
        ],
        "the five tanks, the heavy and the target, and nothing else with two tracks"
    );

    // **16b cut AE.1**: and the scout car's two wheels carry the same rule, over
    // their own names — one wheel gone is `slow(50)` and no turn toward it, both
    // gone is no motion at all.
    let wheels = Consequence::DisableMotionWhenAllOf {
        sections: vec![
            mush_core::CanonicalId::new("left_wheels").expect("a section id"),
            mush_core::CanonicalId::new("right_wheels").expect("a section id"),
        ],
    };
    let mut cars: Vec<String> = Vec::new();
    for (id, spec) in &fielded.specs {
        let wheel = |name: &str| spec.sections.iter().find(|row| row.id.as_str() == name);
        let (Some(left), Some(right)) = (wheel("left_wheels"), wheel("right_wheels")) else {
            continue;
        };
        for row in [left, right] {
            assert!(
                row.consequences.contains(&wheels),
                "{id}'s {} does not carry the pair rule: {:?}",
                row.id,
                row.consequences
            );
        }
        cars.push(id.as_str().to_owned());
    }
    assert_eq!(cars, vec![content::SCOUT_MK1], "the one wheeled chassis");
}

/// **16b cut M.4 (16c `B-47`), then 17b cut M3.1: every chassis pack but the
/// target's moved, the five tanks to version 3.** M.4 moved the five tank
/// packs to version 2 (the pair rule); M3.1 moves every chassis that fits a
/// core one version more (its `core` component, D-UM-33): the five line
/// tanks, the heavy and the scout 2 → 3, both bipeds and the drone 1 → 2 —
/// and each now requires `tacsim.pack.cores` at version 1. `boot_reconcile`
/// refuses a published `(id, version)` whose digest moved, so the versions
/// had to. The range's target fits no core (Class 0, the ROM alone) and is
/// not moved. (Was `m4_the_five_tank_packs_are_version_two_and_the_biped_is_not_moved`:
/// the five at 2 and the biped at 1.)
///
/// **17b cut M3.6** moved the seven that carry a core consequence word one
/// version more (16b cut M.4's rule applied a second time in cut M3): the
/// five line tanks and the scout 3 → 4 (`destroy_core` on the turret, where
/// their core sits), the heavy 3 → 4 (`degrade_core` on the turret; its core
/// sits in the hull). The bipeds and the drone carry no core word — their
/// core's section is `destroy_unit` — and stay at 2. (Was
/// `m4_the_tank_packs_are_version_three_and_every_chassis_but_the_target_moved`.)
#[test]
fn m4_the_tank_packs_are_version_four_and_every_chassis_but_the_target_moved() {
    let cores = mush_core::CanonicalId::new(content::CORES).expect("a canonical id");
    for (json, id, version) in [
        (content::LINE_TANK_JSON, "tacsim.pack.line_tank", 4),
        (
            content::LINE_TANK_LASER_JSON,
            "tacsim.pack.line_tank_laser",
            4,
        ),
        (
            content::LINE_TANK_THERMAL_JSON,
            "tacsim.pack.line_tank_thermal",
            4,
        ),
        (content::LINE_TANK_ECM_JSON, "tacsim.pack.line_tank_ecm", 4),
        (
            content::LINE_TANK_ECCM_JSON,
            "tacsim.pack.line_tank_eccm",
            4,
        ),
        (content::HEAVY_MK1_JSON, "tacsim.pack.heavy_mk1", 4),
        (content::SCOUT_MK1_JSON, "tacsim.pack.scout_mk1", 4),
        (content::BIPED_MK1_JSON, "tacsim.pack.biped_mk1", 2),
        (content::BIPED_MK2_JSON, "tacsim.pack.biped_mk2", 2),
        (content::DRONE_MK1_JSON, "tacsim.pack.drone_mk1", 2),
    ] {
        let pack = tacsim::pack::load_pack(json).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(pack.manifest().id().as_str(), id);
        assert_eq!(pack.manifest().version(), version, "{id}'s version");
        assert!(
            pack.manifest()
                .requires()
                .iter()
                .any(|required| required.id == cores && required.version == 1),
            "{id} requires the cores pack"
        );
    }
    let target =
        tacsim::pack::load_pack(content::TARGET_MK1_JSON).expect("the target's pack verifies");
    assert_eq!(target.manifest().version(), 1, "the target is not moved");
    assert!(
        target.manifest().requires().is_empty(),
        "the target requires nothing: it fits no core"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.3 — six a side, and the applier on a standing world
// ---------------------------------------------------------------------------

/// **The arena's version 3 fields six a side, in the authored order** (16b cut
/// Z.3): twelve tanks, red 1 to 6 then blue 1 to 6 as a side's slot order reads
/// them, the four new ones on the cells the note names, every opening cell
/// passable with a way out (16b cut M.4's rule, read off the authored class grid
/// the same way `ArenaTerrain::has_way_out` reads the built one), and inside
/// the arena's unit bound.
#[test]
fn z3_the_arena_fields_six_a_side_in_the_authored_order() {
    let pack = tacsim::pack::load_pack(content::PROVING_GROUND_JSON).expect("the arena verifies");
    // 16b cut O.1 moved the version to 4 (the Hold declaration) and 16b cut
    // AE.1 to 5 (The Bluff, a fifth key spot); the twelve tanks are Z.3's and
    // unchanged.
    assert_eq!(pack.manifest().version(), 5, "16b cut AE.1's version");
    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    assert_eq!(arena.tanks.len(), 12);
    assert!(arena.tanks.len() <= tacsim::definitions::arena::MAX_ARENA_UNITS);
    for (team, word) in [(Team::Red, "red"), (Team::Blue, "blue")] {
        let side: Vec<&str> = arena
            .tanks
            .iter()
            .filter(|tank| tank.team == team)
            .map(|tank| tank.id.as_str())
            .collect();
        let want: Vec<String> = (1..=PODS_PER_SIDE)
            .map(|index| format!("{word}_{index}"))
            .collect();
        assert_eq!(side, want, "slot n of a side is its n-th authored tank");
    }
    let cell = |id: &str| {
        arena
            .tanks
            .iter()
            .find(|tank| tank.id.as_str() == id)
            .map(|tank| (tank.name.as_str(), tank.x, tank.y, tank.heading_mdeg))
            .unwrap_or_else(|| panic!("`{id}` is authored"))
    };
    assert_eq!(cell("red_5"), ("Red Tank 5", 6, 101, 90_000));
    assert_eq!(cell("red_6"), ("Red Tank 6", 4, 94, 90_000));
    assert_eq!(cell("blue_5"), ("Blue Tank 5", 249, 101, 270_000));
    assert_eq!(cell("blue_6"), ("Blue Tank 6", 251, 94, 270_000));

    let table = mush_atlas::physics::LocomotionTable::shipped();
    let passable = |x: u16, y: u16| table.passable(class_at(x, y).label(), "tracked", &[]);
    for tank in &arena.tanks {
        let (x, y) = (tank.x, tank.y);
        let out = [
            x.checked_sub(1).map(|x| (x, y)),
            x.checked_add(1).filter(|x| *x < SIDE).map(|x| (x, y)),
            y.checked_sub(1).map(|y| (x, y)),
            y.checked_add(1).filter(|y| *y < SIDE).map(|y| (x, y)),
        ]
        .into_iter()
        .flatten()
        .any(|(x, y)| passable(x, y));
        assert!(
            passable(x, y) && out,
            "`{}` opens on {x},{y}, which is not a passable cell with a way out",
            tank.id
        );
    }
}

/// Every object in the world that carries a tactical unit — on a world whose
/// only units are the arena's stock tanks, the count "no thirteenth" is about.
fn units_in(tw: &TestWorld) -> Vec<ObjId> {
    tw.world()
        .iter()
        .filter(|object| !object.flags.contains(mush_core::Flags::GOING))
        .filter(|object| mush_tactics::module::unit_of(tw.world(), object.id).is_some())
        .map(|object| object.id)
        .collect()
}

/// The arena pack's latched version on `tw`.
fn arena_latch(tw: &TestWorld) -> Option<u32> {
    tw.world()
        .module_state::<mush_content::ContentState>()
        .expect("content")
        .latches
        .applied(&mush_core::CanonicalId::new(content::PROVING_GROUND_PACK).expect("an id"))
        .map(mush_content::Applied::version)
}

/// A world that booted the arena's version 2 (and no lot: this is the arena's
/// business alone).
fn world_at_arena_v2() -> TestWorld {
    let cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    let tw = TestWorld::with_modules(support::roster_booting(
        cfg,
        support::boot_packs_at_arena_v2(false),
    ));
    assert_eq!(arena_latch(&tw), Some(2));
    assert_eq!(units_in(&tw).len(), 8, "version 2 made eight tanks");
    tw
}

/// **A renamed stock tank is adopted by the roster, and no thirteenth is
/// made** (16b cut Z.3). Before this cut the applier adopted by name, so a
/// staffer's renamed Red Tank 3 would have had a new Red Tank 3 made beside it
/// and the old one left an orphan on the muster. And the name is never a way to
/// reach a tank the roster already names: a tank renamed to a *new* slot's name
/// is not taken for that slot.
#[test]
fn z3_a_renamed_stock_tank_is_adopted_by_the_roster_and_no_thirteenth_is_made() {
    let mut tw = world_at_arena_v2();
    let red_one = tacsim::match_::tank_for_slot(tw.world(), Team::Red, 1).expect("red 1");
    let red_two = tacsim::match_::tank_for_slot(tw.world(), Team::Red, 2).expect("red 2");
    let red_three = tacsim::match_::tank_for_slot(tw.world(), Team::Red, 3).expect("red 3");
    let before: Vec<(Team, u8, ObjId)> = Team::ALL
        .into_iter()
        .flat_map(|team| (1..=4_u8).map(move |index| (team, index)))
        .map(|(team, index)| {
            let tank = tacsim::match_::tank_for_slot(tw.world(), team, index).expect("a slot");
            (team, index, tank)
        })
        .collect();
    // The stock tanks belong to somebody other than the staffer who will type
    // the verb: the boot made them as God and the apply below runs as God, so
    // without this the owner rule could not be told from `ctx.actor`.
    let keeper = tw.spawn_player("Keeper");
    for (_, _, tank) in &before {
        tw.world_mut()
            .chown(*tank, keeper)
            .expect("staff hands the stock tanks over");
    }
    tw.world_mut()
        .set_name(red_three, "Old Faithful")
        .expect("staff renames a tank");
    tw.world_mut()
        .set_name(red_two, "Red Tank 5")
        .expect("and another, to the name a new slot authors");
    // Off its cell, too: the apply puts every tank back on its authored cell.
    let (domain, held) = mush_sim::pose_of(tw.world(), red_one).expect("placed");
    let elsewhere = mush_sim::data::Pose {
        x_cm: held.x_cm + 40_000,
        ..held.clone()
    };
    support::with_ctx(&mut tw, |ctx| {
        mush_sim::place(ctx, red_one, domain, elsewhere).expect("staff moves a tank");
    });

    let (planned, applied) =
        support::take_from_shelf(&mut tw, mush_core::GOD, content::PROVING_GROUND_PACK);
    assert!(
        planned.contains("0 create, 1 update, 0 retire"),
        "the arena entry is one update:\n{planned}"
    );
    // 16b cut O.1: the shipped arena is version 4 now, so a version 2 world
    // that takes it lands at 4 — and at 5 since 16b cut AE.1.
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@5"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(arena_latch(&tw), Some(5));
    // Every write the applier made under the atlas's identity was taken: the
    // roster document, the four units' documents and the placements all go
    // through `ctx.out`, which no namespace census refuses.
    assert_eq!(
        tw.world().refused_namespace_writes(),
        Vec::new(),
        "the hand apply wrote across a namespace it may not"
    );

    // Twelve units, not thirteen: the renamed ones were adopted, by the roster.
    assert_eq!(units_in(&tw).len(), 12, "four made, and no more");
    for (team, index, tank) in &before {
        assert_eq!(
            tacsim::match_::tank_for_slot(tw.world(), *team, *index),
            Some(*tank),
            "{} slot {index} kept its object",
            team.as_str()
        );
    }
    assert_eq!(
        tw.world()
            .get(red_three)
            .map(|object| object.name.to_string()),
        Some("Old Faithful".to_owned()),
        "the applier adopts; it does not rename"
    );
    let red_five = tacsim::match_::tank_for_slot(tw.world(), Team::Red, 5).expect("red 5");
    assert_ne!(
        red_five, red_two,
        "a tank the roster names for slot 2 is not taken for slot 5 by its new name"
    );
    // Every one of the twelve on its authored cell, and the roster names them.
    let arena = mush_core::CanonicalId::new(PROVING_GROUND_ARENA).expect("a canonical id");
    let roster = tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .and_then(|store| store.arena_roster(&arena).cloned())
        .expect("the roster is written");
    assert_eq!(roster.units.len(), 12, "{:?}", roster.units);
    for team in Team::ALL {
        for index in 1..=PODS_PER_SIDE {
            let slot = format!("{}_{index}", team.as_str());
            let tank = tacsim::match_::tank_for_slot(tw.world(), team, index)
                .unwrap_or_else(|| panic!("{slot} resolves"));
            assert_eq!(roster.unit(&slot), Some(tank));
            let (_, pose) = mush_sim::pose_of(tw.world(), tank).expect("placed");
            assert_eq!(
                (pose.x_cm, pose.y_cm, pose.heading_mdeg),
                support::authored_pose_of(&slot),
                "{slot} stands on its authored cell"
            );
        }
    }
    // Made as the roster's are: owned as Red Tank 1 is, and not by the staffer
    // who typed the verb.
    let owner = |tank: ObjId| tw.world().get(tank).map(|object| object.owner);
    assert_eq!(
        owner(red_one),
        Some(keeper),
        "adoption leaves the owner alone"
    );
    for (team, index) in [
        (Team::Red, 5),
        (Team::Red, 6),
        (Team::Blue, 5),
        (Team::Blue, 6),
    ] {
        let made = tacsim::match_::tank_for_slot(tw.world(), team, index).expect("made");
        assert_eq!(
            owner(made),
            Some(keeper),
            "{} slot {index} is owned as the roster's tanks are",
            team.as_str()
        );
        assert_ne!(owner(made), Some(mush_core::GOD), "not by the applier");
    }
}

/// **A match holding a roster tank refuses the apply, and writes nothing**
/// (16b cut Z.3): the apply re-places every tank and `mush_sim::place` does
/// not refuse a deployed one, so the applier refuses first — in its
/// before-any-write check, so the latch does not move and nothing is made.
#[test]
fn z3_a_deployed_roster_tank_refuses_the_apply_and_writes_nothing() {
    let mut tw = world_at_arena_v2();
    let blue_two = tacsim::match_::tank_for_slot(tw.world(), Team::Blue, 2).expect("blue 2");
    mush_sim::deployed::mark(
        tw.world_mut(),
        blue_two,
        mush_sim::deployed::DeployedMark::new(7, PROVING_GROUND_ARENA),
    );
    let census = |tw: &TestWorld| -> Vec<(u32, String, Option<ObjId>)> {
        tw.world()
            .iter()
            .map(|object| (object.id.0, object.name.to_string(), object.location))
            .collect()
    };
    let before = census(&tw);

    let events = tw.run(
        mush_core::GOD,
        &format!("@atlas/pack plan {}", content::PROVING_GROUND_PACK),
    );
    let planned = tw.seen_by(&events, mush_core::GOD).join("\n");
    let fingerprint = planned
        .lines()
        .find_map(|line| line.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("the plan is readable while a match runs:\n{planned}"))
        .to_owned();
    let events = tw.run(mush_core::GOD, &format!("@atlas/pack apply {fingerprint}"));
    let refused = tw.seen_by(&events, mush_core::GOD).join("\n");
    assert!(
        refused.contains(
            "tacsim.arena.proving_ground: a match is running; apply with the centre idle"
        ),
        "{refused}"
    );
    assert!(!refused.contains("Applied"), "{refused}");

    assert_eq!(arena_latch(&tw), Some(2), "the latch did not move");
    assert_eq!(census(&tw), before, "nothing was made, renamed or moved");
    assert_eq!(units_in(&tw).len(), 8);
    assert_eq!(
        tacsim::match_::tank_for_slot(tw.world(), Team::Red, 5),
        None
    );

    // With the centre idle again the same plan applies.
    mush_sim::deployed::clear(tw.world_mut(), blue_two);
    let (_, applied) =
        support::take_from_shelf(&mut tw, mush_core::GOD, content::PROVING_GROUND_PACK);
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@5"),
        "{applied}"
    );
    assert_eq!(units_in(&tw).len(), 12);
}

/// **16b cut O.1 (16c N11)**: the shipped ground declares Hold at The
/// Crossroads — the parsed declaration, the spot resolved to its index 1 and
/// map letter `B`, six cells round it, 120 s — and the hold square stands clear
/// of both entry zones.
#[test]
fn o1_the_shipped_ground_declares_hold_at_the_crossroads() {
    use tacsim::definitions::arena::ArenaModeDef;
    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    assert_eq!(
        arena.modes,
        vec![ArenaModeDef::Hold {
            spot: "The Crossroads".to_owned(),
            radius_cells: 6,
            hold_ms: 120_000,
        }]
    );
    let resolved = arena.arena_modes();
    assert_eq!(
        resolved,
        vec![tacsim::arena::ArenaMode::Hold(tacsim::arena::HoldRule {
            spot: 1,
            radius_cells: 6,
            hold_ms: 120_000,
        })]
    );
    assert_eq!(tacsim::match_::cockpit::spot_letter(1), Some('B'));
    let spot = arena.key_spots.get(1).expect("the second spot");
    assert_eq!(
        (spot.name.as_str(), spot.x, spot.y),
        ("The Crossroads", 129, 97)
    );
    // The square, 123..=135 by 91..=103, against the zones' own columns.
    for zone in &arena.entry_zones {
        assert!(
            123 > zone.x1 || zone.x0 > 135,
            "the hold square reaches the {} entry zone",
            zone.team.as_str()
        );
    }
    // 120 cells from the blue muster (column 249) and 123 from the red (6).
    let muster = |team: Team| {
        arena
            .tanks
            .iter()
            .find(|tank| tank.team == team && tank.id.as_str().ends_with("_1"))
            .map(|tank| tank.x)
            .expect("slot 1")
    };
    assert_eq!(muster(Team::Blue) - spot.x, 120);
    assert_eq!(spot.x - muster(Team::Red), 123);
    let note = &arena.note;
    assert!(note.contains("VERSION 4 (16b cut O.1)"), "{note}");
    assert!(note.len() <= 1_024, "{} bytes", note.len());
}

// ---------------------------------------------------------------------------
// 16b cut AE.1 — The Bluff (16c T-18)
// ---------------------------------------------------------------------------

/// The box every edge of The Bluff lies in: the mesa, its rim neighbours, the
/// ramp and its foot, `x 140..=148, y 104..=121`.
fn in_bluff_box(cell: (u16, u16)) -> bool {
    (140..=148).contains(&cell.0) && (104..=121).contains(&cell.1)
}

/// **The Bluff is where the note says and what the note says** (16b cut AE.1;
/// 16c `T-18`; doc 16 §9 item 6).
///
/// Decoded from the shipped atlas pack through the production composition and
/// `definition_for`, the way `x6_the_authored_heightfield_reaches_the_arena_
/// verbatim` decodes it: the file and the generator agree cell for cell; every
/// edge past 45 per cent between two cells a tank may stand on is one of The
/// Bluff's 107 — the 104 rim edges at 700 cm (36 orthogonal, 68 diagonal), the
/// one corner at 800 and row 119's two flanks at 470 — and off the bluff the
/// steepest is still 20 orthogonal and 21 diagonal; the ramp steps 230, 230 and
/// 240; row 120's flanks 240 west and 340 east; the mesa clear of both zones,
/// the Hold square, the roads, the ford, every waypoint node and every slot;
/// the fifth key spot at (144, 111), lettered off `SPOT_LETTERS`, on grass with
/// a way out for every row; the atlas at version 2 and the arena at 5, the note
/// under its bound and carrying its own sentence.
#[test]
fn ae1_the_bluff_is_where_and_what_the_note_says() {
    use tacsim::arena::CostRow;
    use tacsim::arena::slope::{steepness_pct, step_kind};

    let tw = support::booted_world();
    let definition =
        tacsim::match_::definition_for(tw.world(), &smol_str::SmolStr::new(PROVING_GROUND_ARENA))
            .expect("the proving ground's ground builds")
            .expect("the proving ground is shipped");
    let terrain = &definition.terrain;
    let cell_cm = u32::from(terrain.cell_cm());
    let elev = |x: u16, y: u16| terrain.get(x, y).map(|cell| cell.elev_cm);

    // The file is the generator, cell for cell, and the mesa is grass.
    for y in 0..SIDE {
        for x in 0..SIDE {
            assert_eq!(elev(x, y), Some(elev_at(true, x, y)), "({x}, {y})");
        }
    }
    for y in BLUFF_TOP.2..=BLUFF_TOP.3 {
        for x in BLUFF_TOP.0..=BLUFF_TOP.1 {
            assert_eq!(elev(x, y), Some(BLUFF_TOP_CM));
            assert_eq!(class_at(x, y), TerrainClass::Grass, "({x}, {y})");
        }
    }
    for ((x0, x1, y0, y1), cm) in BLUFF_RAMP {
        for y in y0..=y1 {
            for x in x0..=x1 {
                assert_eq!(elev(x, y), Some(cm), "the ramp at ({x}, {y})");
                assert_eq!(class_at(x, y), TerrainClass::Grass);
            }
        }
    }

    // Every edge between two cells a tank may stand on, measured.
    let mut past: BTreeMap<(bool, i32), usize> = BTreeMap::new();
    let (mut off_orthogonal, mut off_diagonal) = (0_u32, 0_u32);
    for y in 0..SIDE {
        for x in 0..SIDE {
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
                let Some(kind) = step_kind((x, y), (nx, ny)) else {
                    continue;
                };
                let diagonal = kind == mush_atlas::physics::StepKind::Diagonal;
                let steepness = steepness_pct(here, there, kind, cell_cm);
                if steepness > 45 {
                    assert!(
                        in_bluff_box((x, y)) && in_bluff_box((nx, ny)),
                        "({x}, {y}) -> ({nx}, {ny}) is {steepness} per cent off The Bluff"
                    );
                    let rise = (i32::from(here.elev_cm) - i32::from(there.elev_cm)).abs();
                    *past.entry((diagonal, rise)).or_default() += 1;
                }
                if !in_bluff_box((x, y)) && !in_bluff_box((nx, ny)) {
                    if diagonal {
                        off_diagonal = off_diagonal.max(steepness);
                    } else {
                        off_orthogonal = off_orthogonal.max(steepness);
                    }
                }
            }
        }
    }
    assert_eq!(
        past.into_iter().collect::<Vec<_>>(),
        vec![
            ((false, 470), 2),
            ((false, 700), 36),
            ((true, 700), 68),
            ((true, 800), 1),
        ],
        "the 107: row 119's two flanks, the rims (36 orthogonal and 68 diagonal) and the corner"
    );
    assert_eq!(
        (off_orthogonal, off_diagonal),
        (20, 21),
        "off The Bluff the steepest passable edge is still 20 orthogonal and 21 diagonal"
    );

    // The figures the note and the help quote, edge by edge.
    let edge = |from: (u16, u16), to: (u16, u16)| {
        let (Some(a), Some(b), Some(kind)) = (
            terrain.get(from.0, from.1),
            terrain.get(to.0, to.1),
            step_kind(from, to),
        ) else {
            panic!("{from:?} -> {to:?} is not a step on the window");
        };
        (
            (i32::from(a.elev_cm) - i32::from(b.elev_cm)).abs(),
            steepness_pct(a, b, kind, cell_cm),
        )
    };
    // A rim edge, each way: 700 cm, 70 orthogonal and 49 diagonal.
    assert_eq!(edge((140, 111), (141, 111)), (700, 70), "the west face");
    assert_eq!(edge((144, 104), (144, 105)), (700, 70), "the south face");
    assert_eq!(edge((148, 111), (147, 111)), (700, 70), "the east face");
    assert_eq!(
        edge((140, 104), (141, 105)),
        (700, 49),
        "a diagonal onto the rim"
    );
    // The one corner that rises 800: (148, 119) is -300, so a tank is refused
    // the diagonal by its own slope and a walker is not.
    assert_eq!(elev(148, 119), Some(-300));
    assert_eq!(edge((147, 118), (148, 119)), (800, 56), "the corner");
    // The ramp, from the top down to the grass at row 121.
    assert_eq!(edge((144, 118), (144, 119)), (230, 23));
    assert_eq!(edge((144, 119), (144, 120)), (230, 23));
    assert_eq!(edge((144, 120), (144, 121)), (240, 24));
    assert_eq!(elev(144, 121), Some(-200));
    // Row 119's flanks: refused to tanks and cars, open to legs.
    assert_eq!(edge((140, 119), (141, 119)), (470, 47));
    assert_eq!(edge((147, 119), (146, 119)), (470, 47));
    // Row 120's: any chassis from the west, a tank (not a car) from the east
    // along the row. (147, 121) is -300 as well, and meets the ramp's foot only
    // diagonally: 24 per cent, which motion allows a car (the search refuses it
    // by the shoulder rule; see `support::ground::BLUFF_RAMP`).
    assert_eq!(edge((140, 120), (141, 120)), (240, 24));
    assert_eq!(edge((147, 120), (146, 120)), (340, 34));
    assert_eq!(elev(147, 121), Some(-300));

    // Clear of everything the ground is fought over.
    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let mesa = |x: u16, y: u16| within(x, y, (BLUFF_TOP.0, BLUFF_TOP.1, BLUFF_TOP.2, 120));
    for zone in &arena.entry_zones {
        assert!(
            BLUFF_TOP.1 < zone.x0 || zone.x1 < BLUFF_TOP.0,
            "The Bluff reaches the {} entry zone",
            zone.team.as_str()
        );
    }
    for y in BLUFF_TOP.2..=120 {
        for x in BLUFF_TOP.0..=BLUFF_TOP.1 {
            assert!(
                !within(x, y, (123, 135, 91, 103)),
                "({x}, {y}) is in the Hold square"
            );
            assert!(
                !(ROAD_EW.0..=ROAD_EW.1).contains(&y) && !(ROAD_NS.0..=ROAD_NS.1).contains(&x),
                "({x}, {y}) is on a road"
            );
            assert!(
                !(RIVER.0..=RIVER.1).contains(&x) && !(FORD.0..=FORD.1).contains(&y),
                "({x}, {y}) is in the river or the ford's rows"
            );
        }
    }
    for node in &arena.waypoints.nodes {
        assert!(
            !mesa(node.x, node.y),
            "waypoint `{}` is on The Bluff",
            node.id
        );
    }
    for tank in &arena.tanks {
        assert!(!mesa(tank.x, tank.y), "`{}` musters on The Bluff", tank.id);
    }

    // The fifth key spot: The Bluff at (144, 111), lettered off the constant.
    assert_eq!(arena.key_spots.len(), 5, "five key spots");
    let spot = arena.key_spots.get(4).expect("the fifth spot");
    assert_eq!(
        (spot.name.as_str(), spot.x, spot.y),
        ("The Bluff", 144, 111)
    );
    assert_eq!(
        tacsim::match_::cockpit::spot_letter(4),
        Some('F'),
        "SPOT_LETTERS skips D, so the fifth is F"
    );
    assert_eq!(class_at(144, 111), TerrainClass::Grass);
    for row in CostRow::ALL {
        assert!(
            terrain.has_way_out_for(row, 144, 111),
            "the spot has a way out for the {} row",
            row.as_str()
        );
    }

    // The versions, and the note.
    let atlas = mush_content::load_pack(content::PROVING_GROUND_ATLAS_JSON)
        .expect("the shipped atlas pack verifies");
    assert_eq!(atlas.manifest().version(), 2, "the atlas with The Bluff");
    let pack = tacsim::pack::load_pack(content::PROVING_GROUND_JSON).expect("the arena verifies");
    assert_eq!(
        pack.manifest().version(),
        5,
        "the arena with its fifth spot"
    );
    let note = &arena.note;
    assert!(note.contains("VERSION 5 (16b cut AE.1)"), "{note}");
    assert!(note.len() <= 1_024, "{} bytes", note.len());
}

/// **The two rebuilt fixtures are the versions they say** (16b cut AE.1): the
/// generator with the bluff turned off is the atlas's version 1 to the byte —
/// the proof that moving it into `tests/support/ground.rs` changed nothing —
/// and the arena rebuilt one version back is version 4, with its four key
/// spots and its own note; version 3 is still rebuilt from it.
#[test]
fn ae1_the_rebuilt_fixtures_are_version_one_of_the_ground_and_four_of_the_arena() {
    let ground = mush_content::parse_pack(&atlas_pack_at(false)).expect("version 1 parses");
    assert_eq!(ground.manifest().version(), 1);
    assert_eq!(
        ground.computed_digest().expect("digests").to_hex(),
        support::ATLAS_V1_DIGEST,
        "the generator with no bluff is not version 1"
    );
    assert_eq!(
        ground.manifest().digest().to_hex(),
        support::ATLAS_V1_DIGEST,
        "and it signs itself as version 1"
    );
    let v4 = tacsim::pack::load_pack(&support::arena_pack_v4()).expect("version 4 verifies");
    assert_eq!(v4.manifest().version(), 4);
    assert_eq!(v4.manifest().digest().to_hex(), support::ARENA_V4_DIGEST);
    assert!(support::ARENA_V4_NOTE.len() <= 1_024);
    let v3 = tacsim::pack::load_pack(&support::arena_pack_v3()).expect("version 3 verifies");
    assert_eq!(v3.manifest().digest().to_hex(), support::ARENA_V3_DIGEST);
}

// ---------------------------------------------------------------------------
// 17b cut M2.5 — Standard Doctrine's source, pinned
// ---------------------------------------------------------------------------

/// **The digest `content/minds/standard.rhai` ships with**, written down by
/// hand (doc 17 §13): a script change is a reviewed re-bless of this string
/// and of `tacsim::minds::STANDARD_SHA256`.
///
/// **Re-blessed by 17b cut M2.6 — the fire half written** (doc 17 §5,
/// D-UM-16, D-UM-17, D-UM-18): the stubs M2.5 left (`arming`, the citation
/// past `engage_orders`' lock) became `arming_for` (the slots on change, the
/// slot number excluded, with `aspect_mask`) and `engage_orders`' gates, arm
/// ceiling and citation beside a fresh lock, and the emission cites
/// `fire(weapon, slot)` after `lock`. **And four cost-only rewrites of the
/// helm half M2.5 landed**, behaviour unchanged (Gate A asserts every think
/// of every lane): `read_notices` reads a notice's kind through one `switch`
/// rather than an else-if chain, `hostile_edges` reads the leash's two radii
/// only under a leash, `stand` and `withdraw_stand` read the think's `p.dry`
/// rather than calling `is_dry` again, and `engage_orders` reads `p.dry` and
/// the held row's range inline — a reviewer of this digest's history
/// re-reads those four helm-half functions too. Was
/// `95ee741e70fa804e671e8b34f8fb62ea317b15f6a4bc2bdc5190971180802ecd` (17b
/// cut M2.5, the helm half).
const STANDARD_DOCTRINE_SHA256: &str =
    "102cc7d27eb411c493b2402b20a589e37330bc36bad6eeef0eadffd0eb6e80ec";

/// **Standard Doctrine's source is pinned** (17b cut M2.5; doc 17 §11, §13,
/// §22's italics; 17b §5's feas F9):
///
/// 1. the SHA-256 of `content/minds/standard.rhai` — the file on disk and
///    the bytes `include_str!` compiled in alike — is the digest written
///    down here, `STANDARD_SHA256` and `standard_digest()`;
/// 2. the manifest is the six fields, the role `combat` among them, at the
///    API, stock and ceiling constants the crate holds, and the script's
///    header line says the same;
/// 3. it compiles on the process's sandbox under `MIND_ABI`, and twice on a
///    fresh one caches ONE compilation;
/// 4. **the state codes are integer literals pinned against
///    `ReflexState::ALL`**: the header's table names every state at its
///    `code()` and `as_str()`, and every literal the script compares or
///    writes `this.state` with is one of them;
/// 5. **the binding limits** — at most 32 functions, every body at
///    expression depth 24 (the smallest depth it compiles at is measured and
///    printed), the widest scope (parameters plus bound names) at most 64 —
///    and no FLOAT literal in the code.
#[test]
fn m2_the_standard_doctrine_source_is_pinned() {
    use mush_mind::{EngineLimits, Sandbox};
    use sha2::{Digest as _, Sha256};
    use tacsim::arena::reflex::ReflexState;
    use tacsim::minds::{
        API_VERSION, MIND_ABI, MindManifest, STANDARD_RHAI, STANDARD_SHA256, STOCK_OPS_PER_THINK,
        STOCK_VERSION, register_api, standard_digest, standard_manifest, standard_sandbox,
    };

    // 1. The digest.
    let on_disk = std::fs::read(content_dir().join("minds/standard.rhai"))
        .expect("the script is in the tree");
    assert_eq!(
        on_disk,
        STANDARD_RHAI.as_bytes(),
        "the compiled-in bytes are the file's"
    );
    let digest: [u8; 32] = Sha256::digest(STANDARD_RHAI.as_bytes()).into();
    let lowercase: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(
        lowercase, STANDARD_DOCTRINE_SHA256,
        "standard.rhai moved; re-review the port and re-pin its digest"
    );
    assert_eq!(STANDARD_SHA256, STANDARD_DOCTRINE_SHA256);
    assert_eq!(standard_digest().0, digest);

    // 2. The manifest, and the header that says it.
    let manifest = standard_manifest().expect("the manifest decodes");
    assert_eq!(
        manifest,
        MindManifest {
            name: "Standard Doctrine".to_owned(),
            role: "combat".to_owned(),
            api: 1,
            stock: 1,
            rated_class: "I".to_owned(),
            ops_ceiling: 1_500,
        }
    );
    assert_eq!(
        (manifest.api, manifest.stock, manifest.ops_ceiling),
        (API_VERSION, STOCK_VERSION, STOCK_OPS_PER_THINK)
    );
    let header = STANDARD_RHAI.lines().next().unwrap_or_default();
    assert!(
        header.starts_with("// Standard Doctrine \u{2014} the combat role's stock mind")
            && header.contains(
                "name: Standard Doctrine; role: combat; api: 1; stock: 1; rated: class I \
                 (1,500 ops a think)"
            )
            && header.contains("T1 is the oracle (Gate A)"),
        "{header}"
    );

    // 3. It compiles; a second compile is the cached one.
    assert!(
        standard_sandbox()
            .compile(STANDARD_RHAI, standard_digest(), MIND_ABI)
            .is_ok()
    );
    let fresh = Sandbox::new(
        &EngineLimits {
            max_ops_backstop: STOCK_OPS_PER_THINK,
        },
        register_api,
    );
    for _ in 0..2 {
        assert!(
            fresh
                .compile(STANDARD_RHAI, standard_digest(), MIND_ABI)
                .is_ok()
        );
    }
    assert_eq!(fresh.cached(), 1);

    // 4. The state codes.
    let table = STANDARD_RHAI
        .lines()
        .find_map(|line| {
            line.trim_start_matches("//")
                .trim()
                .strip_prefix("state codes: ")
        })
        .expect("the header's state-code table");
    let named: Vec<(u8, &str)> = table
        .split(", ")
        .map(|pair| {
            let (code, word) = pair.split_once(' ').expect("`code word`");
            (code.parse().expect("an integer code"), word)
        })
        .collect();
    let all: Vec<(u8, &str)> = ReflexState::ALL
        .iter()
        .map(|state| (state.code(), state.as_str()))
        .collect();
    assert_eq!(named, all, "the script's state codes are ReflexState's");
    let mut literals = 0;
    for line in STANDARD_RHAI.lines() {
        let code = line.split("//").next().unwrap_or_default();
        for operator in ["this.state = ", "this.state == ", "this.state != "] {
            for after in code.split(operator).skip(1) {
                let literal: String = after.chars().take_while(char::is_ascii_digit).collect();
                let literal: u8 = literal.parse().expect("a state literal");
                assert!(
                    all.iter().any(|(code, _)| *code == literal),
                    "state {literal} is no ReflexState: {line}"
                );
                literals += 1;
            }
        }
    }
    assert!(literals > 20, "{literals} state literals read");

    // 5. The binding limits, measured.
    let fns = support::script_fns(STANDARD_RHAI);
    assert!(
        fns.len() <= 32,
        "{} functions: max_functions is 32",
        fns.len()
    );
    assert!(
        fns.iter()
            .any(|(name, params, _)| name == "think" && params.len() == 2)
    );
    let (widest, width) = fns
        .iter()
        .map(|(name, params, body)| {
            let mut names: BTreeSet<String> = params.iter().cloned().collect();
            names.extend(support::bound_names(body));
            (name.clone(), names.len())
        })
        .max_by_key(|(_, width)| *width)
        .expect("functions");
    assert!(
        width <= 64,
        "{widest} binds {width} names: max_variables is 64"
    );
    let compiles_at = |depth: usize| {
        Sandbox::new(
            &EngineLimits {
                max_ops_backstop: STOCK_OPS_PER_THINK,
            },
            |engine| {
                register_api(engine);
                engine.set_max_expr_depths(48, depth);
            },
        )
        .compile(STANDARD_RHAI, standard_digest(), MIND_ABI)
        .is_ok()
    };
    let depth = (1..=24)
        .find(|depth| compiles_at(*depth))
        .expect("the script compiles at the binding depth of 24");
    assert!(!compiles_at(depth - 1));
    // Each function's own depth: its text alone (beside an empty `think`
    // where it is not `think`) at the smallest depth it compiles at.
    let sandboxes: Vec<Sandbox> = (1..=24)
        .map(|depth| {
            Sandbox::new(
                &EngineLimits {
                    max_ops_backstop: STOCK_OPS_PER_THINK,
                },
                |engine| {
                    register_api(engine);
                    engine.set_max_expr_depths(48, depth);
                },
            )
        })
        .collect();
    let mut table: Vec<String> = Vec::new();
    for (name, params, body) in &fns {
        let own = format!("fn {name}({}) {{\n{}\n", params.join(", "), body.join("\n"));
        let source = if name == "think" {
            own
        } else {
            format!("fn think(a, b) {{ }}\n{own}")
        };
        let digest = mush_mind::Digest(Sha256::digest(source.as_bytes()).into());
        let own_depth = sandboxes
            .iter()
            .position(|sandbox| sandbox.compile(&source, digest, MIND_ABI).is_ok())
            .map(|at| at + 1)
            .unwrap_or_else(|| panic!("{name} compiles alone at depth 24"));
        table.push(format!("{name}={own_depth}"));
    }
    println!("m2_standard_doctrine_fn_depths: {}", table.join(" "));
    for line in STANDARD_RHAI.lines() {
        let code: Vec<char> = line
            .split("//")
            .next()
            .unwrap_or_default()
            .chars()
            .collect();
        assert!(
            !code
                .windows(3)
                .any(|w| w[0].is_ascii_digit() && w[1] == '.' && w[2].is_ascii_digit()),
            "a FLOAT literal: {line}"
        );
    }
    println!(
        "m2_standard_doctrine: functions={} max_fn_expr_depth={depth} widest_scope={width} \
         ({widest}) lines={} sha256={STANDARD_DOCTRINE_SHA256}",
        fns.len(),
        STANDARD_RHAI.lines().count()
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.1 — the cores, fitted to the shipped chassis
// ---------------------------------------------------------------------------

/// **The `tacsim.power.states` schema's minted digest** (17b cut M3.1): the
/// fourth schema's, which the sixth — `tacsim.core.profile`, appended last
/// (AE.1's rule) — must not move. A `DefinitionRef` digest is a function of
/// its own `(owner, kind, id, version, payload, references)` alone. The value
/// was taken at `634ce9dc`, the commit before the cut, and is unmoved.
const POWER_STATES_SCHEMA_DIGEST: &str =
    "7b126afa67bdfc00be74e463d6ec387520408c2309e10b17e8457e137d288c40";

/// **Every shipped chassis binds its core** (17b cut M3.1; doc 17 §9,
/// D-UM-33, D-UM-34): the fielded catalog's eleven unit schemas, each with
/// the core content places on it — the five line tanks and both bipeds the
/// line core (II), the heavy the command core (III), the scout and the drone
/// the doctrine core (I), the range's target none. On each: the authored spec
/// names exactly that core; the bound unit carries a `core` component in the
/// section content placed it in, its `core_class` cell born at the class's
/// ordinal, and one `tacsim.core` capability sourced on it whose payload is
/// `profile_payload(class, true)` under the minted `tacsim.core.profile`
/// schema; `tacsim::arena::core::read_core` answers the class, governed.
/// The cores pack mints three `core` definitions, the component's reference
/// names its core's, and `tacsim.power.states`' minted reference is the
/// bytes it was.
#[test]
fn m3_every_shipped_chassis_binds_its_core() {
    use tacsim::arena::core::{CoreClass, CoreHardware, read_core};
    use tacsim::definitions::core::profile_payload;
    use tacsim::definitions::unit::{CORE_CAPABILITY, CORE_CLASS_CELL, CORE_RULES_SCHEMA};

    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let id = |word: &str| mush_core::CanonicalId::new(word).expect("a canonical id");

    // The three cores, minted as `core` definitions.
    assert_eq!(fielded.cores.len(), 3, "the cores pack's three");
    for (core, class) in [
        (content::CORE_DOCTRINE, CoreClass::I),
        (content::CORE_LINE, CoreClass::II),
        (content::CORE_COMMAND, CoreClass::III),
    ] {
        let def = fielded.cores.get(&id(core)).expect("the core is loaded");
        assert_eq!(def.class, class, "{core}'s class");
        assert!(def.governed, "{core} is governed");
        let reference = fielded.references.get(&id(core)).expect("minted");
        assert_eq!(reference.key().kind.as_str(), "core", "{core}");
    }

    /// The core a chassis fits: its id, its section and its class.
    type Fitted<'a> = Option<(&'a str, &'a str, CoreClass)>;
    let line = Some((content::CORE_LINE, "turret", CoreClass::II));
    let expected: [(&str, Fitted<'_>); 11] = [
        (content::LINE_TANK, line),
        (content::LINE_TANK_LASER, line),
        (content::LINE_TANK_THERMAL, line),
        (content::LINE_TANK_ECM, line),
        (content::LINE_TANK_ECCM, line),
        (
            content::BIPED_MK1,
            Some((content::CORE_LINE, "torso", CoreClass::II)),
        ),
        (
            content::BIPED_MK2,
            Some((content::CORE_LINE, "torso", CoreClass::II)),
        ),
        (
            content::HEAVY_MK1,
            Some((content::CORE_COMMAND, "hull", CoreClass::III)),
        ),
        (
            content::SCOUT_MK1,
            Some((content::CORE_DOCTRINE, "turret", CoreClass::I)),
        ),
        (
            content::DRONE_MK1,
            Some((content::CORE_DOCTRINE, "body", CoreClass::I)),
        ),
        (content::TARGET_MK1, None),
    ];
    assert_eq!(fielded.specs.len(), expected.len(), "every fielded chassis");
    for (entry, fitted) in expected {
        let key = id(entry);
        let spec = fielded.specs.get(&key).expect("the chassis is fielded");
        let binding = fielded.bindings.get(&key).expect("its binding");
        let unit = tacsim::definitions::to_tactical_unit(spec, binding, ObjId(21))
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
        let named: Vec<&str> = spec.cores().map(mush_core::CanonicalId::as_str).collect();
        let Some((core, section, class)) = fitted else {
            assert!(named.is_empty(), "{entry} fits no core: {named:?}");
            assert_eq!(read_core(&unit), None, "{entry}: Class 0, the ROM alone");
            continue;
        };
        assert_eq!(named, vec![core], "{entry} names its core");
        let component = mush_tactics::ComponentId::new("core").expect("a component id");
        let installed = unit.component(&component).expect("the core component");
        assert_eq!(
            installed.install,
            mush_tactics::InstallPoint::Section(
                mush_tactics::SectionId::new(section).expect("a section id")
            ),
            "{entry}'s core sits in its {section}"
        );
        let cell = unit
            .cell(&mush_tactics::CellAddr::Component {
                component: component.clone(),
                cell: mush_tactics::CellId::new(CORE_CLASS_CELL).expect("a cell id"),
            })
            .expect("the core_class cell");
        assert_eq!(
            (cell.min, cell.max, cell.current),
            (0, 3, i64::from(class.ordinal())),
            "{entry}'s cell"
        );
        let keys: Vec<&mush_tactics::CapabilityKey> = unit
            .capabilities()
            .filter(|key| key.id.as_str() == CORE_CAPABILITY)
            .collect();
        assert_eq!(keys.len(), 1, "{entry}: one core capability");
        let key = *keys.first().expect("the key");
        assert_eq!(key.source.as_ref(), Some(&component), "{entry}'s source");
        let rules = unit.capability(key).expect("its rules");
        assert_eq!(
            rules.data(),
            &profile_payload(class, true),
            "{entry}'s payload is the fitting's record"
        );
        assert_eq!(
            rules.schema().key().id.as_str(),
            CORE_RULES_SCHEMA,
            "{entry}'s schema"
        );
        assert_eq!(
            read_core(&unit),
            Some(CoreHardware {
                component: smol_str::SmolStr::new("core"),
                class,
                governed: true,
            }),
            "{entry} reads its core"
        );
        // The component's definition names its core's.
        let core_ref = fielded.references.get(&id(core)).expect("minted");
        let component_def = binding
            .descriptors
            .iter()
            .find(|descriptor| descriptor.id().as_str() == format!("{entry}.core"))
            .expect("the core component's descriptor");
        assert_eq!(
            component_def.references(),
            std::slice::from_ref(core_ref),
            "{entry}"
        );
    }

    // The fitting's record pinned literally, not through the helper `bind`
    // itself calls: the line core's payload is Class II's row of doc 17 §9,
    // figure by figure under its own key, and its keys are exactly the ones
    // the minted `tacsim.core.profile` schema says it carries.
    let line_payload = serde_json::json!({
        "class": "II", "governed": true, "think_ms": 500, "ops": 3000,
        "rows": 16, "odds_rows": 4, "slots": 4, "memory_keys": 64,
        "line_to": 4, "profiles": 3
    });
    assert_eq!(profile_payload(CoreClass::II, true), line_payload);
    let schema = fielded
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.kind().as_str() == "schema" && descriptor.id().as_str() == CORE_RULES_SCHEMA
        })
        .expect("the minted core schema");
    let carries: std::collections::BTreeSet<&str> = schema
        .payload()
        .get("carries")
        .and_then(serde_json::Value::as_array)
        .expect("the schema's carries")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let keys: std::collections::BTreeSet<&str> = line_payload
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(carries, keys, "the payload carries what its schema says");

    // `tacsim.power.states` is the bytes it was: the sixth schema moved no
    // other.
    let tank = fielded
        .bindings
        .get(&id(content::LINE_TANK))
        .expect("the tank");
    let unit = tacsim::definitions::to_tactical_unit(
        fielded
            .specs
            .get(&id(content::LINE_TANK))
            .expect("the spec"),
        tank,
        ObjId(21),
    )
    .expect("the tank instantiates");
    let power = unit
        .capabilities()
        .find(|key| key.id.as_str() == "tacsim.power")
        .and_then(|key| unit.capability(key))
        .expect("the power capability");
    assert_eq!(
        power.schema().key().id.as_str(),
        tacsim::definitions::unit::POWER_RULES_SCHEMA
    );
    assert_eq!(
        power.schema().digest().to_hex(),
        POWER_STATES_SCHEMA_DIGEST,
        "tacsim.power.states' minted reference moved"
    );
}

/// **The stock class table is doc 17 §9's table** (17b cut M3.1; D-UM-35,
/// D-UM-36): the four rows' figures, the ceilings per second (400, 1,500,
/// 6,000 and 18,000), the engine backstop equal to the largest class's
/// `ops`, `[tacsim.minds].classes`' default equal to the compiled-in table,
/// and Standard Doctrine's manifest rated at `STOCK_RATED_CLASS`.
#[test]
fn m3_the_stock_class_table_is_the_doc_17_table() {
    use tacsim::arena::core::{ClassTable, CoreClass, MAX_CONDITION_SLOTS};
    let figures = |class: CoreClass| {
        let row = ClassTable::STOCK.row(class);
        assert_eq!(row.class, class);
        (
            row.think_ms,
            row.ops,
            row.rows,
            row.odds_rows,
            row.slots,
            row.memory_keys,
            row.line_to,
            row.profiles,
        )
    };
    assert_eq!(figures(CoreClass::Zero), (1_000, 400, 8, 0, 1, 16, 1, 1));
    assert_eq!(figures(CoreClass::I), (1_000, 1_500, 12, 2, 2, 32, 2, 2));
    assert_eq!(figures(CoreClass::II), (500, 3_000, 16, 4, 4, 64, 4, 3));
    assert_eq!(figures(CoreClass::III), (250, 4_500, 24, 8, 6, 128, 6, 4));
    assert_eq!(
        CoreClass::ALL.map(|class| ClassTable::STOCK.ceiling_per_s(class)),
        [400, 1_500, 6_000, 18_000]
    );
    assert_eq!(
        ClassTable::STOCK.row(CoreClass::III).ops,
        mush_mind::MAX_OPS_ANY_CLASS,
        "the engine backstop is the largest class's ceiling"
    );
    assert_eq!(
        mush_mind::EngineLimits::default().max_ops_backstop,
        mush_mind::MAX_OPS_ANY_CLASS
    );
    assert_eq!(
        ClassTable::STOCK.row(CoreClass::III).memory_keys,
        mush_mind::MAX_MEMORY_ANY_CLASS.keys
    );
    assert_eq!(
        ClassTable::STOCK.row(CoreClass::III).slots,
        MAX_CONDITION_SLOTS
    );
    assert_eq!(
        tacsim::config::MindsConfig::default().classes,
        ClassTable::STOCK,
        "the config's default is the compiled-in table"
    );
    let manifest = tacsim::minds::standard_manifest().expect("the manifest decodes");
    assert_eq!(
        CoreClass::parse(&manifest.rated_class),
        Some(tacsim::arena::mind::STOCK_RATED_CLASS),
        "Standard Doctrine is rated at the arena's stock rating"
    );
    assert_eq!(
        manifest.ops_ceiling,
        ClassTable::STOCK
            .row(tacsim::arena::mind::STOCK_RATED_CLASS)
            .ops
    );
}
