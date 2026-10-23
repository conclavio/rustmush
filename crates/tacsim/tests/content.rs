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
    V3_BLUFF_KEEP, V3_CORRIDOR, V3_SIGNAL_HILL, atlas_pack_at, atlas_pack_v3, class_at, elev_at,
    elev_v3, sign, within,
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
        "c7202e25766303689c252737a58a97240159d95b1df9967645a4f674151d3b4c",
    ),
];

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_atlas_pack_is_this_generators_output() {
    // Doc 21 cut G.7b: version 3, the showcase landscape.
    pin_file("atlas/proving_ground.json", &atlas_pack_v3());
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
///
/// **17b cut M4.5a**: the eleven schemas are checked out `Controller::Mind` —
/// what production fields since 17b cut M4.3's flip, the range's target and the
/// drone included — where they were checked out on the reflex controller. The
/// arena is never stepped, so no mind thinks: every reach read here is the
/// hull's (`UnitState`), derived at check-out and controller-blind.
#[test]
fn e1_the_ranges_the_shipped_machines_fight_at() {
    use mush_core::ObjId;
    use smol_str::SmolStr;
    use tacsim::arena::state::{Arena, Controller, CustodyOut, Team, WeaponSummary};
    use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
    use tacsim::match_::{ARENA_CELL_CM, STAND_IN, custody_unit, stock_definition};

    let catalog =
        tacsim::definitions::Catalog::build_fielded(&[], "tacsim").expect("the fielded content");
    // Doc 18 cut IC2: the fielded catalog holds the Dray too, which no arena
    // fields (class `hauler`; `Arena::new` refuses it). The eleven an arena can
    // field are every spec whose class is fieldable — the same eleven as before.
    // Doc 20 cut SM.3a: and the terrain vehicle and the Dray-P, neither fieldable
    // (was 12).
    assert_eq!(
        catalog.specs.len(),
        14,
        "the shipped unit schemas: the line tank, doc 16 §9's five extensions, \
         the range's target, 16b cut AE.1's three chassis, 16b cut AE.3's drone, \
         doc 18 cut IC2's Dray and doc 20 cut SM.3a's terrain vehicle and Dray-P"
    );
    let fieldable: Vec<(&mush_core::CanonicalId, &tacsim::definitions::UnitSpec)> = catalog
        .specs
        .iter()
        .filter(|(_, spec)| spec.class_of().fieldable())
        .collect();
    assert_eq!(
        fieldable.len(),
        11,
        "the shipped unit schemas an arena fields: the line tank, doc 16 §9's five \
         extensions, the range's target, 16b cut AE.1's three chassis and 16b cut AE.3's drone"
    );
    let definition = stock_definition(&SmolStr::new_static(STAND_IN)).expect("the stand-in window");
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .expect("the engine's own passive posture");
    let half = i64::from(ARENA_CELL_CM / 2);

    // One of each, two cells apart so no two share a cell (doc 16 §3.4 step 5).
    let mut names: Vec<String> = Vec::new();
    let mut units = Vec::new();
    for (at, (entry, spec)) in fieldable.iter().copied().enumerate() {
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
                Controller::Mind,
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
    // Doc 18 cut IC2: and the Dray, a twentieth, last (was `+ 19`). Doc 20 cut
    // SM.3a: and the terrain vehicle and the Dray-P after it (was `+ 20`).
    assert_eq!(
        fielded.len(),
        stock.len() + 22,
        "nine shipped packs and twenty-two extension files: doc 16 section 9's ten, \
         the range's target, 16b cut AE.2's two weapons and their two rounds, \
         16b cut AE.1's three chassis, 16b cut AE.3's drone, doc 18 cut IC2's Dray and \
         doc 20 cut SM.3a's terrain vehicle and Dray-P"
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
        // M5r: version 4, its note describes the restored public Hold goal.
        (content::DRONE_MK1_JSON, "tacsim.pack.drone_mk1", 4),
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
/// **The showcase relief** (doc 21 cut G.7b, version 3 of the atlas pack):
/// the shipped ground is the generator's, cell for cell; the fought-over
/// ground — the corridor, the north-south road, The Bluff — is version 2's to
/// the centimetre; no passable edge off The Bluff is steeper than a tank's 45
/// per cent; a tank reaches every key spot, every waypoint and the other
/// muster from its own, and a car drives the road from muster to muster
/// through the crossroads and the ford; the landscape spans at least 30
/// metres; and the map frame with its heights stays inside its bound.
#[test]
fn g7b_the_showcase_relief_is_drivable_and_the_fought_ground_is_unchanged() {
    use std::collections::VecDeque;
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

    let (mut lo, mut hi) = (i16::MAX, i16::MIN);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let e = elev(x, y);
            assert_eq!(e, Some(elev_v3(x, y)), "({x}, {y}) is the generator's");
            let in_corridor = (V3_CORRIDOR.0..=V3_CORRIDOR.1).contains(&y);
            let on_road = (ROAD_NS.0..=ROAD_NS.1).contains(&x);
            if in_corridor || on_road || within(x, y, V3_BLUFF_KEEP) {
                assert_eq!(e, Some(elev_at(true, x, y)), "({x}, {y}) is version 2's");
            }
            let e = e.unwrap_or(0);
            lo = lo.min(e);
            hi = hi.max(e);
        }
    }
    assert!(
        i32::from(hi) - i32::from(lo) >= 3_000,
        "the landscape spans {lo}..{hi} cm"
    );
    assert!(
        elev(V3_SIGNAL_HILL.0 as u16, V3_SIGNAL_HILL.1 as u16).unwrap_or(0) >= 3_000,
        "Signal Hill stands"
    );

    // Off The Bluff, nothing a tank may stand on is too steep for it.
    let in_keep = |(x, y): (u16, u16)| within(x, y, V3_BLUFF_KEEP);
    let mut steepest = 0;
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
                if in_keep((x, y)) || in_keep((nx, ny)) {
                    continue;
                }
                let steepness = steepness_pct(here, there, kind, cell_cm);
                steepest = steepest.max(steepness);
                assert!(
                    steepness <= 45,
                    "({x}, {y}) -> ({nx}, {ny}) is {steepness} per cent"
                );
            }
        }
    }
    println!("g7b: relief {lo}..{hi} cm, steepest off The Bluff {steepest} per cent");

    // Who reaches what: an orthogonal flood within a row's slope limit.
    let flood = |row: CostRow, limit: u32, from: (u16, u16)| -> BTreeSet<(u16, u16)> {
        let mut seen = BTreeSet::from([from]);
        let mut queue = VecDeque::from([from]);
        while let Some((x, y)) = queue.pop_front() {
            let Some(here) = terrain.get(x, y) else {
                continue;
            };
            for (dx, dy) in [(1_i32, 0_i32), (-1, 0), (0, 1), (0, -1)] {
                let (Some(nx), Some(ny)) = (
                    u16::try_from(i32::from(x) + dx).ok(),
                    u16::try_from(i32::from(y) + dy).ok(),
                ) else {
                    continue;
                };
                if seen.contains(&(nx, ny)) || !terrain.passable_for(row, nx, ny) {
                    continue;
                }
                let (Some(there), Some(kind)) = (terrain.get(nx, ny), step_kind((x, y), (nx, ny)))
                else {
                    continue;
                };
                if steepness_pct(here, there, kind, cell_cm) > limit {
                    continue;
                }
                seen.insert((nx, ny));
                queue.push_back((nx, ny));
            }
        }
        seen
    };
    let (red, blue) = ((6_u16, 97_u16), (249_u16, 97_u16));
    let tank = flood(CostRow::Tracked, 45, red);
    assert!(tank.contains(&blue), "a tank drives muster to muster");
    for spot in &definition.key_spots {
        assert!(
            tank.contains(&(spot.x, spot.y)),
            "a tank reaches {}",
            spot.name
        );
    }
    for node in definition.waypoints.nodes() {
        assert!(
            tank.contains(&(node.x, node.y)),
            "a tank reaches waypoint {}",
            node.id
        );
    }
    let car = flood(CostRow::Wheeled, 25, red);
    for (name, cell) in [
        ("the ford", (64, 97)),
        ("the crossroads", (129, 97)),
        ("blue's muster", blue),
    ] {
        assert!(car.contains(&cell), "a car reaches {name}");
    }

    // The map frame, heights and all, inside its bound.
    let frame = tacsim::match_::cockpit::map_frame(
        tacsim::arena_task::MatchId(u64::MAX),
        PROVING_GROUND_ARENA,
        terrain,
        &definition.key_spots,
    );
    let bytes = serde_json::to_vec(&frame).expect("the frame encodes").len();
    println!("g7b: the map frame is {bytes} bytes");
    assert!(
        bytes <= tacsim::match_::cockpit::MAP_MAX_BYTES,
        "{bytes} bytes"
    );
    if let Ok(path) = std::env::var("RELIEF_PREVIEW") {
        std::fs::write(path, serde_json::to_string(&frame).expect("json")).expect("the preview");
    }
}

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

    // The file is the generator, cell for cell (version 3 since doc 21 cut
    // G.7b, which keeps The Bluff's box at version 2's heights), and the mesa
    // is grass.
    for y in 0..SIDE {
        for x in 0..SIDE {
            assert_eq!(elev(x, y), Some(elev_v3(x, y)), "({x}, {y})");
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
    // Doc 21 cut G.7b: version 3's landscape is steeper than version 2's
    // terraces, and nowhere off The Bluff past a tank's 45 per cent.
    assert_eq!(
        (off_orthogonal, off_diagonal),
        (41, 34),
        "off The Bluff the steepest passable edge is 41 orthogonal and 34 diagonal"
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
    // Doc 21 cut G.7b: version 3 keeps The Bluff under its landscape.
    assert_eq!(
        atlas.manifest().version(),
        3,
        "the atlas with The Bluff and the landscape"
    );
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
    "9b330c0f5ccd861078d4928fbc30dda7a132df6c8f1b027d4a5a00ca452cf69b";

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
///    `MachineState::DOCTRINE`** (`MachineState::ALL` until 17b cut M4.6
///    appended `Unknown` to it): the header's table names every doctrine
///    state at its `code()` and `as_str()`, and every literal the script
///    compares or writes `this.state` with is one of them;
/// 5. **the binding limits** — at most 32 functions, every body at
///    expression depth 24 (the smallest depth it compiles at is measured and
///    printed), the widest scope (parameters plus bound names) at most 64 —
///    and no FLOAT literal in the code.
#[test]
fn m2_the_standard_doctrine_source_is_pinned() {
    use mush_mind::{EngineLimits, Sandbox};
    use sha2::{Digest as _, Sha256};
    use tacsim::arena::doctrine::MachineState;
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
            api: 4,
            stock: 2,
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
                "name: Standard Doctrine; role: combat; api: 4; stock: 2; rated: class I \
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
    // 17b cut M4.6: the six the doctrine writes (`MachineState::DOCTRINE`),
    // not `ALL`, which carries `Unknown` since that cut — a state read off the
    // seal, never written by the script (whose header, under the pinned
    // digest, is not edited).
    let all: Vec<(u8, &str)> = MachineState::DOCTRINE
        .iter()
        .map(|state| (state.code(), state.as_str()))
        .collect();
    assert_eq!(named, all, "the script's state codes are MachineState's");
    let mut literals = 0;
    for line in STANDARD_RHAI.lines() {
        let code = line.split("//").next().unwrap_or_default();
        for operator in ["this.state = ", "this.state == ", "this.state != "] {
            for after in code.split(operator).skip(1) {
                let literal: String = after.chars().take_while(char::is_ascii_digit).collect();
                let literal: u8 = literal.parse().expect("a state literal");
                assert!(
                    all.iter().any(|(code, _)| *code == literal),
                    "state {literal} is no MachineState: {line}"
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
    // Doc 18 cut IC2: twelve with the Dray, which fits no core (Class 0, the
    // drive ROM) — was eleven. Doc 20 cut SM.3a: fourteen with the terrain vehicle
    // and the Dray-P, neither fitting a core (Class 0, as the Dray).
    let expected: [(&str, Fitted<'_>); 14] = [
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
        (content::DRAY, None),
        (content::TV_PROSPECTOR, None),
        (content::DRAY_P, None),
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

// ---------------------------------------------------------------------------
// Doc 18 cut IC2 — the Dray and the goods table
// ---------------------------------------------------------------------------

/// **The Dray's pin** (doc 18 cut IC2; doc 18 §4.2, D-18-29). Re-signed in
/// place under `TACSIM_WRITE_CONTENT=1` and copied here by hand.
const DRAY_DIGEST: &str = "b40cfff4943616ec798534fee10a7a995433477ef248a5875a87b69b53b9e372";

/// **The goods table's pin** (doc 18 cut IC2; doc 18 §4.3): the canonical digest
/// of `content/market/goods.json` under `tacsim.goods.v1`. Printed under
/// `TACSIM_WRITE_CONTENT=1` and copied here by hand.
const GOODS_DIGEST: &str = "d8e79a2cfdd8468eb1555673babea71cb27847604b6251f991581720b68f088e";

/// A canonical id, for the tests below.
fn cid(word: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(word).expect("a canonical id")
}

/// The shipped Dray's authored spec, decoded through `load_pack` (so through
/// `UnitSpec::check`).
fn dray_spec() -> tacsim::definitions::UnitSpec {
    let pack = tacsim::pack::load_pack(content::DRAY_JSON).expect("the Dray verifies");
    match pack.entry(&cid(content::DRAY)) {
        Some(tacsim::pack::PackEntry::Unit(spec)) => spec.clone(),
        other => panic!("the Dray's entry is a unit: {other:?}"),
    }
}

/// The refusal `spec.check` gives, as its sentence.
fn refusal(spec: &tacsim::definitions::UnitSpec) -> String {
    spec.check(content::DRAY)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "accepted".to_owned())
}

/// **The Dray is pinned, and its authored numbers are doc 18 §4.2's** (doc 18
/// cut IC2; D-18-29). `load_pack` runs `UnitSpec::check`, so the note's 1,024-byte
/// bound and every logistics and interior rule fail this test and never a boot.
#[test]
fn ic2_the_dray_is_pinned_and_checks() {
    use tacsim::definitions::{DriveLaw, InteriorRoomKind, MAX_NOTE_BYTES, UnitClass};

    let relative = "units/dray.json";
    if writing() {
        let path = content_dir().join(relative);
        let raw = std::fs::read_to_string(&path).expect("the content file exists");
        let declared = mush_content::parse_pack(&raw)
            .expect("the pack parses")
            .manifest()
            .digest()
            .to_hex();
        let signed = sign(&raw.replace(&declared, UNSIGNED));
        std::fs::write(&path, &signed).expect("the content file is writable");
        let digest = mush_content::parse_pack(&signed)
            .expect("the signed pack parses")
            .manifest()
            .digest()
            .to_hex();
        println!("    const DRAY_DIGEST: &str = \"{digest}\";");
        return;
    }
    let pack = tacsim::pack::load_pack(content::DRAY_JSON)
        .unwrap_or_else(|error| panic!("{relative} does not verify: {error}"));
    assert_eq!(pack.manifest().id().as_str(), content::DRAY_PACK);
    assert_eq!(
        pack.manifest().version(),
        3,
        "doc 19 cut A.2 re-published the Dray at v3 (the flip law); doc 18 cut IC8's was v2"
    );
    assert!(
        pack.manifest().requires().is_empty(),
        "the Dray requires nothing"
    );
    assert_eq!(
        pack.manifest().digest().to_hex(),
        DRAY_DIGEST,
        "the Dray's digest moved; re-review the content and re-pin it"
    );
    let spec = dray_spec();
    assert!(
        spec.note.len() <= MAX_NOTE_BYTES,
        "the note is {} bytes",
        spec.note.len()
    );
    assert_eq!(spec.class, Some(UnitClass::Hauler));
    assert_eq!(spec.class_of(), UnitClass::Hauler);

    let logistics = spec
        .logistics
        .as_ref()
        .expect("the Dray declares logistics");
    assert_eq!(logistics.dry_mass_t, 240);
    let compartment = |id: &str| {
        let def = logistics
            .stowage
            .get(&cid(id))
            .unwrap_or_else(|| panic!("the {id} compartment"));
        (
            def.accepts
                .iter()
                .map(|tag| tag.as_str().to_owned())
                .collect::<Vec<_>>(),
            def.limits
                .iter()
                .map(|(measure, limit)| (measure.as_str().to_owned(), *limit))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(logistics.stowage.len(), 3);
    assert_eq!(
        compartment("cargo"),
        (
            vec!["stow.bulk".to_owned()],
            vec![
                ("si.mass_kg".to_owned(), 100_000),
                ("si.volume_l".to_owned(), 120_000)
            ]
        )
    );
    assert_eq!(
        compartment("propellant"),
        (
            vec!["stow.propellant".to_owned()],
            vec![("si.mass_kg".to_owned(), 80_000)]
        )
    );
    assert_eq!(
        compartment("provisions"),
        (
            vec!["stow.provisions".to_owned()],
            vec![
                ("si.mass_kg".to_owned(), 480),
                ("si.volume_l".to_owned(), 1_200)
            ]
        )
    );
    assert_eq!(logistics.berths, 2);
    assert_eq!(logistics.occupant_allowance_kg, 100);
    assert_eq!(logistics.payload_ceiling_kg, 181_000);
    assert_eq!(
        logistics
            .drive
            .as_ref()
            .expect("the Dray's drive")
            .class
            .as_str(),
        "coast_hauler"
    );
    // Doc 19 cut A.2 (D-19-10): the flip law, a twentieth of a gravity at the
    // rated 421 t, exhaust at 2 % of c; the slice's coast {226, 105} was v2's.
    assert_eq!(
        logistics.drive.as_ref().expect("the Dray's drive").law,
        DriveLaw::Flip {
            accel_mg: 50,
            exhaust_kms: 6_000
        }
    );

    let interior = spec
        .interior
        .as_ref()
        .expect("the Dray declares an interior");
    assert_eq!(interior.space.as_str(), "airlock");
    assert_eq!(interior.store.as_str(), "stowage");
    assert_eq!(interior.berths_slot.id.as_str(), "berths");
    assert_eq!(interior.berths_slot.maximum_items, 2);
    let rooms: Vec<(&str, &str, InteriorRoomKind)> = interior
        .rooms
        .iter()
        .map(|room| (room.id.as_str(), room.name.as_str(), room.kind))
        .collect();
    assert_eq!(
        rooms,
        vec![
            ("airlock", "Airlock", InteriorRoomKind::Walkable),
            ("passage", "Central Passage", InteriorRoomKind::Walkable),
            ("bridge", "Bridge", InteriorRoomKind::Walkable),
            ("crew", "Crew Compartment", InteriorRoomKind::Walkable),
            ("hold", "Cargo Hold", InteriorRoomKind::Walkable),
            (
                "engineering",
                "Engineering Access",
                InteriorRoomKind::Walkable
            ),
            ("stowage", "Hold Stowage", InteriorRoomKind::Store),
        ],
        "doc 18's seven rooms, in its order"
    );
    // 18a section 3's text rides the template (lead ruling R6): every walkable
    // room a short desc, a desc and details; the store a desc and nothing else.
    for room in &interior.rooms {
        assert!(!room.desc.is_empty(), "{} has its desc", room.id);
        match room.kind {
            InteriorRoomKind::Walkable => {
                assert!(
                    !room.short_desc.is_empty(),
                    "{} has its short desc",
                    room.id
                );
                assert!(!room.details.is_empty(), "{} has its details", room.id);
            }
            InteriorRoomKind::Store => {
                assert!(room.short_desc.is_empty() && room.details.is_empty());
            }
            // Doc 20 cut SM.3a: the bay is the Dray-P's alone.
            InteriorRoomKind::Bay => panic!("the Dray has no bay"),
        }
    }
    let exits: Vec<(&str, &str, Vec<&str>)> = interior
        .exits
        .iter()
        .map(|exit| {
            (
                exit.from.as_str(),
                exit.to.as_str(),
                exit.names.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        exits,
        vec![
            (
                "airlock",
                "passage",
                vec![
                    "Inner Hatch <I>;inner;in;passage",
                    "Airlock <A>;airlock;forward"
                ]
            ),
            (
                "passage",
                "bridge",
                vec!["Bridge <U>;bridge;up;u", "Passage <D>;passage;down;d"]
            ),
            (
                "passage",
                "crew",
                vec![
                    "Crew Compartment <C>;crew;galley;bunks;aft",
                    "Passage <P>;passage;out;forward"
                ]
            ),
            (
                "passage",
                "hold",
                vec!["Cargo Hold <H>;hold;down;d", "Passage <U>;passage;up;u"]
            ),
            (
                "hold",
                "engineering",
                vec![
                    "Engineering <E>;engineering;eng;down;d",
                    "Hold <H>;hold;up;u"
                ]
            ),
        ],
        "doc 18's five exit pairs, name for name"
    );
    let gauges: Vec<(&str, Vec<&str>)> = interior
        .gauges
        .iter()
        .map(|(gauge, rooms)| (gauge.as_str(), rooms.iter().map(|r| r.as_str()).collect()))
        .collect();
    assert_eq!(
        gauges,
        vec![
            ("cargo", vec!["hold", "passage", "bridge"]),
            ("propellant", vec!["engineering", "passage", "bridge"]),
            ("provisions", vec!["crew", "passage", "bridge"]),
        ]
    );
}

/// **A note past the bound fails the load, not the boot** (doc 18 cut IC2; doc
/// 18 §4.2): the shipped JSON with its note padded to 1,025 bytes and re-signed
/// in memory is refused by `load_pack` with the note's sentence.
#[test]
fn ic2_a_dray_note_past_the_bound_fails_the_load() {
    let mut doc: serde_json::Value =
        serde_json::from_str(content::DRAY_JSON).expect("the Dray is JSON");
    let note = doc
        .pointer("/entries/tacsim.unit.dray/def/note")
        .and_then(serde_json::Value::as_str)
        .expect("the note")
        .to_owned();
    let padded = format!("{note}{}", " ".repeat(1_025 - note.len()));
    assert_eq!(padded.len(), 1_025);
    *doc.pointer_mut("/entries/tacsim.unit.dray/def/note")
        .expect("the note") = serde_json::Value::String(padded);
    *doc.pointer_mut("/manifest/digest").expect("the digest") =
        serde_json::Value::String(UNSIGNED.to_owned());
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("the pack serializes"));
    let Err(error) = tacsim::pack::load_pack(&signed) else {
        panic!("a 1,025-byte note loaded");
    };
    assert!(
        error.to_string().contains("a note is at most 1024 bytes"),
        "{error}"
    );
}

/// **The Dray is a hauler, fits no core, and moved nothing fielded before it**
/// (doc 18 cut IC2; doc 18 §4.2 schema additions (1) and (2), D-18-29): the
/// fielded catalog carries it, its class is `Hauler` off the spec and off the
/// bound unit, it fits no core (Class 0, the drive ROM), and every reference the
/// fielded catalog held before it joined — the eleven other unit schemas, every
/// weapon, sensor, ammunition and core — is the same `DefinitionRef`, because
/// `bind()` never reads `logistics` or `interior`.
#[test]
fn ic2_the_dray_is_a_hauler_and_binds_class_zero() {
    use tacsim::arena::core::read_core;
    use tacsim::config::LoadedPack;
    use tacsim::definitions::UnitClass;

    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let key = cid(content::DRAY);
    let spec = fielded.specs.get(&key).expect("the Dray is fielded");
    assert_eq!(spec.class_of(), UnitClass::Hauler);
    assert!(
        !spec.class_of().fieldable(),
        "a hauler is never fielded in an arena"
    );
    let binding = fielded.bindings.get(&key).expect("its binding");
    let unit = tacsim::definitions::to_tactical_unit(spec, binding, ObjId(41))
        .unwrap_or_else(|why| panic!("the Dray instantiates: {why}"));
    assert_eq!(UnitClass::read(&unit), UnitClass::Hauler);
    assert_eq!(
        unit.rules().data().get("class"),
        Some(&serde_json::json!("hauler")),
        "the declared word rides the blob"
    );
    assert_eq!(spec.cores().count(), 0, "the Dray fits no core");
    assert_eq!(read_core(&unit), None, "Class 0, the drive ROM alone");

    // Nothing fielded before the Dray moved: the catalog the nine earlier
    // extension tables give beside the stock game holds the same references.
    let earlier: Vec<(&str, &str)> = [
        content::EXTENSIONS.as_slice(),
        content::EXTENSIONS_BIPED.as_slice(),
        content::EXTENSIONS_THERMAL.as_slice(),
        content::EXTENSIONS_ECM.as_slice(),
        content::EXTENSIONS_ECCM.as_slice(),
        content::EXTENSIONS_TARGET.as_slice(),
        content::EXTENSIONS_ORDNANCE.as_slice(),
        content::EXTENSIONS_CHASSIS.as_slice(),
        content::EXTENSIONS_DRONE.as_slice(),
    ]
    .concat();
    let operator: Vec<LoadedPack> = content::extension_sources_of(&earlier)
        .iter()
        .map(|source| LoadedPack {
            pack: tacsim::pack::load_pack(&source.bytes)
                .unwrap_or_else(|error| panic!("{}: {error}", source.name)),
            source: source.name.clone(),
        })
        .collect();
    let before = Catalog::build_game(&operator, tacsim::NS).expect("the earlier content binds");
    assert_eq!(
        before.specs.len(),
        11,
        "the eleven the fielded catalog held"
    );
    assert!(!before.specs.contains_key(&key));
    // Doc 20 cut SM.3a: the terrain vehicle and the Dray-P were fielded after the
    // Dray (was `+ 1`, the Dray only).
    assert_eq!(
        fielded.specs.len(),
        before.specs.len() + 3,
        "the Dray, the terrain vehicle and the Dray-P, and only they"
    );
    for (id, reference) in &before.references {
        assert_eq!(
            fielded.references.get(id),
            Some(reference),
            "{id} moved when the Dray loaded; a persisted unit points at it"
        );
    }
    assert_eq!(fielded.weapons.len(), before.weapons.len());
    assert_eq!(fielded.sensors.len(), before.sensors.len());
    assert_eq!(fielded.ammunition.len(), before.ammunition.len());
    assert_eq!(fielded.cores.len(), before.cores.len());
}

/// **An interior refuses what the hull template cannot build** (doc 18 cut IC2;
/// doc 18 §4.2, 18a §3): from the shipped Dray, one mutation per rule, each
/// refused with its own sentence.
#[test]
fn ic2_an_interior_refuses_what_the_template_cannot_build() {
    use tacsim::definitions::{InteriorExitDef, InteriorRoomKind};

    let base = dray_spec();
    assert_eq!(refusal(&base), "accepted", "the shipped Dray checks");
    let mutate = |change: &dyn Fn(&mut tacsim::definitions::InteriorDef)| {
        let mut spec = base.clone();
        change(spec.interior.as_mut().expect("the interior"));
        refusal(&spec)
    };

    // A walkable room no exit reaches.
    let lost = mutate(&|interior| {
        interior
            .exits
            .retain(|exit| exit.to.as_str() != "engineering");
    });
    assert!(
        lost.contains("interior room `engineering` cannot be reached from `airlock`"),
        "{lost}"
    );
    // An exit into the store.
    let store = mutate(&|interior| {
        interior.exits.push(InteriorExitDef {
            from: cid("passage"),
            to: cid("stowage"),
            names: vec![
                "Stowage <S>;stowage".to_owned(),
                "Passage;passage".to_owned(),
            ],
        });
    });
    assert!(
        store.contains(
            "the exit `passage` -> `stowage` touches the store `stowage`; a store has no exits"
        ),
        "{store}"
    );
    // A duplicate alias out of the passage: the crew hatch answering `up`.
    let alias = mutate(&|interior| {
        if let Some(exit) = interior
            .exits
            .iter_mut()
            .find(|exit| exit.to.as_str() == "crew")
            && let Some(name) = exit.names.first_mut()
        {
            *name = "Crew Compartment <C>;crew;up".to_owned();
        }
    });
    assert!(
        alias.contains("the alias `up` leaves `passage` twice"),
        "{alias}"
    );
    // A second store.
    let second = mutate(&|interior| {
        if let Some(room) = interior
            .rooms
            .iter_mut()
            .find(|room| room.id.as_str() == "engineering")
        {
            room.kind = InteriorRoomKind::Store;
        }
    });
    assert!(
        second.contains("an interior has one store room; `engineering` and `stowage` are both"),
        "{second}"
    );
    // The berths slot disagreeing with the berths.
    let berths = mutate(&|interior| interior.berths_slot.maximum_items = 3);
    assert!(
        berths.contains("berths_slot.maximum_items is 3 and logistics.berths is 2; they agree"),
        "{berths}"
    );
    // A gauge naming a compartment the hull does not have.
    let gauge = mutate(&|interior| {
        interior.gauges.insert(cid("ballast"), vec![cid("hold")]);
    });
    assert!(
        gauge.contains("the gauge `ballast` names no logistics.stowage compartment"),
        "{gauge}"
    );
    // And an interior without logistics, and a hauler without them.
    let mut bare = base.clone();
    bare.logistics = None;
    assert!(refusal(&bare).contains("an interior requires logistics"));
    bare.interior = None;
    assert!(refusal(&bare).contains("a hauler declares its logistics"));
}

/// **The payload rail sits at or above the stowage maxima plus the occupants**
/// (doc 18 cut IC2; doc 18 §4.2: 100,000 + 80,000 + 480 + 2 × 100 = 180,680),
/// and a drive and a compartment are bounded by their own rules.
#[test]
fn ic2_logistics_refuses_a_rail_below_the_stowage_maxima() {
    use tacsim::definitions::DriveLaw;

    let base = dray_spec();
    let with = |change: &dyn Fn(&mut tacsim::definitions::LogisticsDef)| {
        let mut spec = base.clone();
        change(spec.logistics.as_mut().expect("the logistics"));
        refusal(&spec)
    };
    let low = with(&|logistics| logistics.payload_ceiling_kg = 180_679);
    assert!(
        low.contains(
            "logistics.payload_ceiling_kg 180679 is below the stowage mass limits plus the \
             occupants, 180680"
        ),
        "{low}"
    );
    assert_eq!(
        with(&|logistics| logistics.payload_ceiling_kg = 180_680),
        "accepted",
        "the floor itself is a legal rail"
    );
    let still = with(&|logistics| {
        logistics.drive.as_mut().expect("the Dray's drive").law = DriveLaw::Coast {
            kms: 0,
            burn_permille: 105,
        };
    });
    assert!(
        still.contains("drive `coast_hauler` coasts at 1 to 299792 km/s; 0 is not"),
        "{still}"
    );
    let weightless = with(&|logistics| {
        if let Some(cargo) = logistics.stowage.get_mut(&cid("cargo")) {
            cargo.limits.remove(&cid("si.mass_kg"));
        }
    });
    assert!(
        weightless.contains(
            "compartment `cargo` does not limit si.mass_kg; every compartment says what it weighs"
        ),
        "{weightless}"
    );
}

/// **The goods table is pinned** (doc 18 cut IC2; doc 18 §4.3): its canonical
/// digest under `tacsim.goods.v1`, the eight rows in the table's order with their
/// per-unit figures, and the cross-check against the Dray the fielded catalog
/// holds.
#[test]
fn ic2_the_goods_table_is_pinned() {
    use tacsim::definitions::goods::{GOODS_JSON, goods_digest, goods_table};
    use tacsim::definitions::{Consumable, GoodClass};

    let digest = goods_digest(GOODS_JSON)
        .expect("the table digests")
        .to_hex();
    if writing() {
        println!("    const GOODS_DIGEST: &str = \"{digest}\";");
        return;
    }
    assert_eq!(
        digest, GOODS_DIGEST,
        "goods.json moved; re-review the content and re-pin it"
    );
    let table = goods_table().expect("the shipped goods table loads");
    // Doc 19 cut A.8: v2 (the two moon goods); doc 20 cut SM.3a: v3 (the four
    // run-of-mine ores under `stow.ore`).
    assert_eq!(table.version, 3);
    let measures: Vec<(&str, &str, &str)> = table
        .measures
        .iter()
        .map(|m| (m.id.as_str(), m.unit.as_str(), m.word.as_str()))
        .collect();
    assert_eq!(
        measures,
        vec![
            ("si.mass_kg", "kg", "mass"),
            ("si.volume_l", "L", "packed volume")
        ]
    );
    type Row<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        GoodClass,
        &'a str,
        u64,
        u64,
        &'a str,
        Option<Consumable>,
        u64,
    );
    let rows: Vec<Row<'_>> = table
        .goods
        .iter()
        .map(|good| {
            (
                good.id.as_str(),
                good.label.as_str(),
                good.family.as_ref().map(|f| f.as_str()),
                good.class,
                good.ledger_unit.as_str(),
                good.per_unit(&cid("si.mass_kg")).unwrap_or(0),
                good.per_unit(&cid("si.volume_l")).unwrap_or(0),
                good.stowage.as_str(),
                good.consumable,
                good.reference_major,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "metals",
                "metal concentrate",
                Some("metals"),
                GoodClass::Trade,
                "t",
                1_000,
                400,
                "stow.bulk",
                None,
                200
            ),
            (
                "volatiles",
                "water ice",
                Some("volatiles"),
                GoodClass::Trade,
                "t",
                1_000,
                1_400,
                "stow.bulk",
                None,
                100
            ),
            (
                "structural_alloys",
                "structural alloy stock",
                Some("structural_alloys"),
                GoodClass::Trade,
                "t",
                1_000,
                300,
                "stow.bulk",
                None,
                400
            ),
            (
                "industrial_parts",
                "yard fittings, crated",
                Some("industrial_parts"),
                GoodClass::Trade,
                "t",
                1_000,
                2_300,
                "stow.bulk",
                None,
                400
            ),
            (
                "propellant",
                "reaction mass",
                Some("propellant"),
                GoodClass::Trade,
                "t",
                1_000,
                1_000,
                "stow.propellant",
                Some(Consumable::Propellant),
                40
            ),
            (
                "provisions",
                "voyage provisions",
                None,
                GoodClass::ShipSupply,
                "crew_day",
                4,
                10,
                "stow.provisions",
                Some(Consumable::Provisions),
                10
            ),
            // Doc 19 cut A.8 (D-19-20): the moon ports' two goods.
            (
                "instrument_boards",
                "instrument boards, crated",
                Some("electronics"),
                GoodClass::Trade,
                "t",
                1_000,
                2_550,
                "stow.bulk",
                None,
                900
            ),
            (
                "carbon_feedstock",
                "carbon billet, pressed",
                Some("carbon_feedstock"),
                GoodClass::Trade,
                "t",
                1_000,
                600,
                "stow.bulk",
                None,
                250
            ),
            // Doc 20 cut SM.3a (§7.3, Q-20-1): the four ores, bought only at
            // Ceres, under their own stowage tag.
            (
                "metal_ore",
                "metal ore, run-of-mine",
                Some("metals"),
                GoodClass::Trade,
                "t",
                1_000,
                500,
                "stow.ore",
                None,
                70
            ),
            (
                "ice_ore",
                "ice ore, run-of-mine",
                Some("volatiles"),
                GoodClass::Trade,
                "t",
                1_000,
                1_000,
                "stow.ore",
                None,
                35
            ),
            (
                "carbon_ore",
                "carbon ore, run-of-mine",
                Some("carbon_feedstock"),
                GoodClass::Trade,
                "t",
                1_000,
                1_200,
                "stow.ore",
                None,
                50
            ),
            (
                "strategic_ore",
                "strategic ore, run-of-mine",
                Some("strategic_elements"),
                GoodClass::Trade,
                "t",
                1_000,
                600,
                "stow.ore",
                None,
                600
            ),
        ],
        "doc 18 section 4.3's six rows, doc 19 cut A.8's two and doc 20 cut SM.3a's \
         four, in its order"
    );
    for good in &table.goods {
        assert_eq!(good.ledger_unit, good.quote_unit, "{}", good.id);
        assert_eq!(good.mass_kg_per_unit(), good.per_unit(&cid("si.mass_kg")));
    }
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let dray = fielded
        .specs
        .get(&cid(content::DRAY))
        .and_then(|spec| spec.logistics.as_ref())
        .expect("the Dray's logistics");
    table
        .check_stowage(content::DRAY, dray)
        .expect("the goods table fits the Dray's stowage");
}

// ---------------------------------------------------------------------------
// Doc 20 cut SM.3a — the terrain vehicle, the Dray-P and the tools
// ---------------------------------------------------------------------------

/// **The terrain vehicle's pin** (doc 20 cut SM.3a; §4.1, D-20-6). Re-signed in
/// place under `TACSIM_WRITE_CONTENT=1` and copied here by hand.
const TV_DIGEST: &str = "c55b9b5e30fa4ac21050f5b48bb6bfa9fa8ac2b2d2e7fca3dbfdbeca455f6c1f";

/// **The Dray-P's pin** (doc 20 cut SM.3a; §4.3, D-20-7). Re-signed in place
/// under `TACSIM_WRITE_CONTENT=1` and copied here by hand.
const DRAY_P_DIGEST: &str = "67c6558731e386077d89009ad10997585b981de79285e1af990c6d57cbd9ab31";

/// **The tools' pins** (doc 20 cut SM.3a; §4.1): each tool's canonical digest
/// under `tacsim.tools.v1`. Printed under `TACSIM_WRITE_CONTENT=1` and copied here
/// by hand.
const TOOL_DIGESTS: [(&str, &str); 3] = [
    (
        content::DRILL_CORE_MK1,
        "8120813eafbf68e340df41361dd2e3d6f39711310c91aeead7140aa65806cce8",
    ),
    (
        content::SCANNER_MK1,
        "e25ff198480f973b4b80435f0fa5ea81f0ab5780c0f4958f6cc5cf8805c6e6ac",
    ),
    (
        content::BATTERY_MK1,
        "198bf543540cbf24a32bbbfce910b43511b3e42ed481e53cae6465267d755c05",
    ),
];

/// Re-sign one unit pack in place (the Dray test's writing branch) and print its
/// pin under `name`.
fn resign(relative: &str, name: &str) {
    let path = content_dir().join(relative);
    let raw = std::fs::read_to_string(&path).expect("the content file exists");
    let declared = mush_content::parse_pack(&raw)
        .expect("the pack parses")
        .manifest()
        .digest()
        .to_hex();
    let signed = sign(&raw.replace(&declared, UNSIGNED));
    std::fs::write(&path, &signed).expect("the content file is writable");
    let digest = mush_content::parse_pack(&signed)
        .expect("the signed pack parses")
        .manifest()
        .digest()
        .to_hex();
    println!("    const {name}: &str = {digest:?};");
}

/// The shipped spec of `entry` in `json`, decoded through `load_pack`.
fn unit_spec(json: &str, entry: &str) -> tacsim::definitions::UnitSpec {
    let pack = tacsim::pack::load_pack(json).expect("the pack verifies");
    match pack.entry(&cid(entry)) {
        Some(tacsim::pack::PackEntry::Unit(spec)) => spec.clone(),
        other => panic!("{entry} is a unit: {other:?}"),
    }
}

/// **The tools are pinned and check** (doc 20 cut SM.3a; §4.1, decision D3/D4):
/// each tool's canonical digest, every figure verbatim from doc 20 §4.1 (and the
/// battery's charge figures as data), and `tools_table()` holds the three.
#[test]
fn sm3a_the_tools_are_pinned_and_check() {
    use tacsim::definitions::tools::{ToolRole, tool_digest, tools_table};
    let sources: BTreeMap<&str, &str> = [
        (content::DRILL_CORE_MK1, content::DRILL_CORE_MK1_JSON),
        (content::SCANNER_MK1, content::SCANNER_MK1_JSON),
        (content::BATTERY_MK1, content::BATTERY_MK1_JSON),
    ]
    .into_iter()
    .collect();
    if writing() {
        for (id, json) in &sources {
            let digest = tool_digest(json).expect("the tool digests").to_hex();
            println!("    ({id}, {digest:?}),");
        }
        return;
    }
    for (id, pinned) in TOOL_DIGESTS {
        let json = sources.get(id).expect("a shipped tool");
        assert_eq!(
            tool_digest(json).expect("the tool digests").to_hex(),
            pinned,
            "{id} moved; re-review the content and re-pin it"
        );
    }
    let table = tools_table().expect("the shipped tools load");
    assert_eq!(table.len(), 3, "the drill, the mast and the battery");
    let tool = |id: &str| table.get(&cid(id)).expect("a shipped tool").clone();
    let drill = tool(content::DRILL_CORE_MK1);
    assert_eq!((drill.version, drill.label.as_str()), (1, "Mk1 core drill"));
    match drill.role {
        ToolRole::Drill(d) => {
            assert_eq!((d.cycle_s, d.rock_factor, d.energy_cu), (1_800, 2, 2_000));
            assert_eq!((d.yield_t.lean, d.yield_t.fair, d.yield_t.rich), (1, 2, 3));
        }
        other => panic!("the drill is a drill: {other:?}"),
    }
    let mast = tool(content::SCANNER_MK1);
    assert_eq!((mast.version, mast.label.as_str()), (1, "Mk1 survey mast"));
    match mast.role {
        ToolRole::Scanner(s) => {
            assert_eq!(
                (s.scan_s, s.scan_radius_cells, s.mast_cm, s.scan_cu),
                (600, 48, 250, 1_000)
            );
            assert_eq!(
                (
                    s.sound_s,
                    s.sound_radius_cells,
                    s.quiet_radius_cells,
                    s.sound_cu
                ),
                (1_200, 16, 8, 2_000)
            );
        }
        other => panic!("the mast is a scanner: {other:?}"),
    }
    let battery = tool(content::BATTERY_MK1);
    assert_eq!(
        (battery.version, battery.label.as_str()),
        (1, "Mk1 rig battery")
    );
    match battery.role {
        ToolRole::Battery(b) => assert_eq!(
            (
                b.capacity_cu,
                b.bay_recharge_cu_per_ic_hour,
                b.drive_cu_per_nominal_cell,
                b.reserve_permille
            ),
            (100_000, 50_000, 15, 50)
        ),
        other => panic!("the battery is a battery: {other:?}"),
    }
    for tool in table.values() {
        assert!(
            tool.note.contains("per axis"),
            "{}: every note states the per-axis radius rule",
            tool.id
        );
    }
}

/// **A tool out of bounds is refused** (doc 20 cut SM.3a): from the shipped
/// files, one mutation per rule, each refused with its own sentence.
#[test]
fn sm3a_a_tool_out_of_bounds_is_refused() {
    use tacsim::definitions::tools::ToolDef;
    let refused = |json: &str, edit: &dyn Fn(&mut serde_json::Value)| {
        let mut doc: serde_json::Value = serde_json::from_str(json).expect("a tool is JSON");
        edit(&mut doc);
        match ToolDef::load("tool.json", &doc.to_string()) {
            Ok(_) => "accepted".to_owned(),
            Err(error) => error.to_string(),
        }
    };
    let quiet = refused(content::SCANNER_MK1_JSON, &|doc| {
        doc["role"]["scanner"]["quiet_radius_cells"] = serde_json::json!(9);
        doc["role"]["scanner"]["sound_radius_cells"] = serde_json::json!(8);
    });
    assert!(
        quiet.contains("quiet 9, sound 8, scan 48 is not"),
        "{quiet}"
    );
    let wide = refused(content::SCANNER_MK1_JSON, &|doc| {
        doc["role"]["scanner"]["scan_radius_cells"] = serde_json::json!(129);
    });
    assert!(
        wide.contains("quiet 8, sound 16, scan 129 is not"),
        "{wide}"
    );
    // Rich under fair: fair 3, rich 2.
    let under = refused(content::DRILL_CORE_MK1_JSON, &|doc| {
        doc["role"]["drill"]["yield_t"]["fair"] = serde_json::json!(3);
        doc["role"]["drill"]["yield_t"]["rich"] = serde_json::json!(2);
    });
    assert!(under.contains("lean 1, fair 3, rich 2 is not"), "{under}");
    let instant = refused(content::DRILL_CORE_MK1_JSON, &|doc| {
        doc["role"]["drill"]["cycle_s"] = serde_json::json!(0);
    });
    assert!(
        instant.contains("drill.cycle_s is 1 to 86400; 0 is not"),
        "{instant}"
    );
    let reserve = refused(content::BATTERY_MK1_JSON, &|doc| {
        doc["role"]["battery"]["reserve_permille"] = serde_json::json!(1_000);
    });
    assert!(
        reserve.contains("battery.reserve_permille is 1 to 999; 1000 is not"),
        "{reserve}"
    );
    let unknown = refused(content::BATTERY_MK1_JSON, &|doc| {
        doc["surprise"] = serde_json::json!(1);
    });
    assert!(unknown.contains("unknown field `surprise`"), "{unknown}");
}

/// **The terrain vehicle is pinned and checks** (doc 20 cut SM.3a; §4.1, D-20-6):
/// its digest, version 1, every authored number, class `vehicle` off the spec
/// and off the bound unit and never fieldable, no drive, the payload floor, and
/// the goods table fits its hopper.
#[test]
fn sm3a_the_terrain_vehicle_is_pinned_and_checks() {
    use tacsim::definitions::{InteriorRoomKind, MAX_NOTE_BYTES, UnitClass};
    let relative = "units/tv_prospector.json";
    if writing() {
        resign(relative, "TV_DIGEST");
        return;
    }
    let pack = tacsim::pack::load_pack(content::TV_PROSPECTOR_JSON)
        .unwrap_or_else(|error| panic!("{relative} does not verify: {error}"));
    assert_eq!(pack.manifest().id().as_str(), content::TV_PROSPECTOR_PACK);
    assert_eq!(pack.manifest().version(), 1);
    let requires: Vec<(String, u32)> = pack
        .manifest()
        .requires()
        .iter()
        .map(|r| (r.id.to_string(), r.version))
        .collect();
    assert_eq!(requires, vec![("tacsim.pack.visual_mk1".to_owned(), 1)]);
    assert_eq!(
        pack.manifest().digest().to_hex(),
        TV_DIGEST,
        "the terrain vehicle's digest moved; re-review the content and re-pin it"
    );
    let spec = unit_spec(content::TV_PROSPECTOR_JSON, content::TV_PROSPECTOR);
    assert!(
        spec.note.len() <= MAX_NOTE_BYTES,
        "the note is {} bytes",
        spec.note.len()
    );
    assert_eq!(spec.label, "Prospector terrain vehicle");
    assert_eq!(spec.class, Some(UnitClass::Vehicle));
    assert_eq!(spec.class_of(), UnitClass::Vehicle);
    assert!(
        !spec.class_of().fieldable(),
        "a vehicle is never fielded in phase 1"
    );
    assert_eq!(spec.sensor_height_cm, 250);
    let sections: Vec<(&str, u32)> = spec
        .sections
        .iter()
        .map(|s| (s.id.as_str(), s.internals))
        .collect();
    assert_eq!(
        sections,
        vec![("chassis", 120), ("hopper", 80), ("boom", 40)]
    );
    let logistics = spec.logistics.as_ref().expect("stowage-only logistics");
    assert!(
        logistics.drive.is_none(),
        "no drive: it never plans a passage"
    );
    assert_eq!(logistics.dry_mass_t, 6);
    assert_eq!(logistics.berths, 1);
    assert_eq!(logistics.occupant_allowance_kg, 100);
    assert_eq!(logistics.payload_ceiling_kg, 12_100);
    assert_eq!(logistics.payload_floor_kg(), Some(12_100));
    let hopper = logistics.stowage.get(&cid("hopper")).expect("the hopper");
    assert_eq!(
        hopper
            .accepts
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>(),
        vec!["stow.ore"]
    );
    assert_eq!(hopper.limits.get(&cid("si.mass_kg")), Some(&12_000));
    assert_eq!(hopper.limits.get(&cid("si.volume_l")), Some(&13_200));
    let interior = spec.interior.as_ref().expect("the cab and the hopper");
    assert_eq!(
        (interior.space.as_str(), interior.store.as_str()),
        ("cab", "hopper")
    );
    assert_eq!(interior.berths_slot.maximum_items, 1);
    let rooms: Vec<(&str, &str, InteriorRoomKind)> = interior
        .rooms
        .iter()
        .map(|room| (room.id.as_str(), room.name.as_str(), room.kind))
        .collect();
    assert_eq!(
        rooms,
        vec![
            ("cab", "Cab", InteriorRoomKind::Walkable),
            ("hopper", "Hopper", InteriorRoomKind::Store),
        ]
    );
    assert!(interior.exits.is_empty() && interior.bays.is_empty());
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let key = cid(content::TV_PROSPECTOR);
    let bound = fielded
        .specs
        .get(&key)
        .expect("the terrain vehicle is fielded");
    let binding = fielded.bindings.get(&key).expect("its binding");
    let unit = tacsim::definitions::to_tactical_unit(bound, binding, ObjId(43))
        .unwrap_or_else(|why| panic!("the terrain vehicle instantiates: {why}"));
    assert_eq!(UnitClass::read(&unit), UnitClass::Vehicle);
    assert!(
        fielded.references.contains_key(&key),
        "it mints a DefinitionRef"
    );
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    goods
        .check_stowage(content::TV_PROSPECTOR, logistics)
        .expect("the goods table fits the hopper");
}

/// **The Dray-P is the Dray with a bay** (doc 20 cut SM.3a; §4.3, D-20-7, §11):
/// its digest; `refit_compatible(Dray, Dray-P)`; its hold, bay and egress; the
/// goods table fits it; and the payload floor with two empty rigs stays under
/// the unmoved rail (166,680 + 2 × the TV's dry mass = 178,680 ≤ 181,000).
#[test]
fn sm3a_the_dray_p_is_the_dray_with_a_bay() {
    use tacsim::definitions::{InteriorRoomKind, MAX_NOTE_BYTES, UnitClass};
    let relative = "units/dray_p.json";
    if writing() {
        resign(relative, "DRAY_P_DIGEST");
        return;
    }
    let pack = tacsim::pack::load_pack(content::DRAY_P_JSON)
        .unwrap_or_else(|error| panic!("{relative} does not verify: {error}"));
    assert_eq!(pack.manifest().id().as_str(), content::DRAY_P_PACK);
    assert_eq!(pack.manifest().version(), 1);
    assert!(
        pack.manifest().requires().is_empty(),
        "the Dray-P requires nothing"
    );
    assert_eq!(
        pack.manifest().digest().to_hex(),
        DRAY_P_DIGEST,
        "the Dray-P's digest moved; re-review the content and re-pin it"
    );
    let dray = dray_spec();
    let spec = unit_spec(content::DRAY_P_JSON, content::DRAY_P);
    assert!(
        spec.note.len() <= MAX_NOTE_BYTES,
        "the note is {} bytes",
        spec.note.len()
    );
    assert_eq!(spec.label, "Dray-P prospecting hauler");
    assert_eq!(spec.class_of(), UnitClass::Hauler);
    assert_eq!(
        tacsim::port::hull::refit_compatible(&dray, &spec),
        Ok(()),
        "the Dray-P re-binds a Dray hull in place"
    );
    assert_eq!(
        (
            &spec.sections,
            &spec.components,
            &spec.facets,
            &spec.silhouettes
        ),
        (
            &dray.sections,
            &dray.components,
            &dray.facets,
            &dray.silhouettes
        ),
        "the combat anatomy is the Dray's"
    );
    let logistics = spec.logistics.as_ref().expect("the logistics");
    let cargo = logistics
        .stowage
        .get(&cid("cargo"))
        .expect("the cargo hold");
    assert_eq!(
        cargo.accepts.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
        vec!["stow.bulk", "stow.ore"]
    );
    assert_eq!(cargo.limits.get(&cid("si.mass_kg")), Some(&86_000));
    assert_eq!(cargo.limits.get(&cid("si.volume_l")), Some(&103_200));
    assert_eq!(logistics.payload_ceiling_kg, 181_000, "never re-rated");
    assert_eq!(logistics.payload_floor_kg(), Some(166_680));
    let interior = spec.interior.as_ref().expect("the interior");
    let bay_room = interior.room(&cid("bay")).expect("the bay room");
    assert_eq!(
        (bay_room.name.as_str(), bay_room.kind),
        ("Vehicle Bay", InteriorRoomKind::Bay)
    );
    let bay = interior.bay().expect("one bay");
    assert_eq!(interior.bays.len(), 1);
    assert_eq!(
        (
            bay.room.as_str(),
            bay.slot.id.as_str(),
            bay.slot.maximum_items
        ),
        ("bay", "rigs", 2)
    );
    assert_eq!(
        (
            bay.egress.label.as_str(),
            bay.egress.x_cm,
            bay.egress.y_cm,
            bay.egress.heading_mdeg
        ),
        ("stern ramp", 0, -3_500, 180_000)
    );
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    goods
        .check_stowage(content::DRAY_P, logistics)
        .expect("the goods table fits the Dray-P");
    // The TV's dry mass, read from its spec.
    let tv = unit_spec(content::TV_PROSPECTOR_JSON, content::TV_PROSPECTOR);
    let tv_kg = u64::from(
        tv.logistics
            .as_ref()
            .expect("the TV's logistics")
            .dry_mass_t,
    ) * 1_000;
    let laden = 166_680 + 2 * tv_kg;
    assert_eq!(laden, 178_680);
    assert!(
        laden <= logistics.payload_ceiling_kg,
        "2,320 kg of headroom"
    );
}

/// **Nothing fielded before the vehicles moved** (doc 20 cut SM.3a): the
/// `ic2_the_dray_is_a_hauler_and_binds_class_zero` method — every reference the
/// fielded catalog held through `EXTENSIONS_HAULER` is the same `DefinitionRef`,
/// and the vehicles add exactly two specs.
#[test]
fn sm3a_nothing_fielded_before_the_vehicles_moved() {
    use tacsim::config::LoadedPack;
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let earlier: Vec<(&str, &str)> = [
        content::EXTENSIONS.as_slice(),
        content::EXTENSIONS_BIPED.as_slice(),
        content::EXTENSIONS_THERMAL.as_slice(),
        content::EXTENSIONS_ECM.as_slice(),
        content::EXTENSIONS_ECCM.as_slice(),
        content::EXTENSIONS_TARGET.as_slice(),
        content::EXTENSIONS_ORDNANCE.as_slice(),
        content::EXTENSIONS_CHASSIS.as_slice(),
        content::EXTENSIONS_DRONE.as_slice(),
        content::EXTENSIONS_HAULER.as_slice(),
    ]
    .concat();
    let operator: Vec<LoadedPack> = content::extension_sources_of(&earlier)
        .iter()
        .map(|source| LoadedPack {
            pack: tacsim::pack::load_pack(&source.bytes)
                .unwrap_or_else(|error| panic!("{}: {error}", source.name)),
            source: source.name.clone(),
        })
        .collect();
    let before = Catalog::build_game(&operator, tacsim::NS).expect("the earlier content binds");
    assert_eq!(
        fielded.specs.len(),
        before.specs.len() + 2,
        "the terrain vehicle and the Dray-P, and only they"
    );
    for (id, reference) in &before.references {
        assert_eq!(
            fielded.references.get(id),
            Some(reference),
            "{id} moved when the vehicles loaded; a persisted unit points at it"
        );
    }
    assert_eq!(fielded.weapons.len(), before.weapons.len());
    assert_eq!(fielded.sensors.len(), before.sensors.len());
    assert_eq!(fielded.ammunition.len(), before.ammunition.len());
    assert_eq!(fielded.cores.len(), before.cores.len());
}

/// **A bay refuses what the template cannot build** (doc 20 cut SM.3a; §4.3):
/// from the shipped Dray-P, one mutation per rule, each refused with its own
/// sentence.
#[test]
fn sm3a_a_bay_refuses_what_the_template_cannot_build() {
    use tacsim::definitions::{InteriorExitDef, InteriorRoomKind};
    let base = unit_spec(content::DRAY_P_JSON, content::DRAY_P);
    let refusal = |spec: &tacsim::definitions::UnitSpec| {
        spec.check(content::DRAY_P)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "accepted".to_owned())
    };
    assert_eq!(refusal(&base), "accepted", "the shipped Dray-P checks");
    let mutate = |change: &dyn Fn(&mut tacsim::definitions::InteriorDef)| {
        let mut spec = base.clone();
        change(spec.interior.as_mut().expect("the interior"));
        refusal(&spec)
    };
    let first = |i: &tacsim::definitions::InteriorDef| i.bays.first().cloned().expect("a bay");
    let with_first = |i: &mut tacsim::definitions::InteriorDef,
                      edit: &dyn Fn(&mut tacsim::definitions::BayDef)| {
        if let Some(bay) = i.bays.first_mut() {
            edit(bay);
        }
    };
    let cases: Vec<(&str, String)> = vec![
        (
            "an interior has at most 2 bays; 3 is not",
            mutate(&|i| {
                let bay = first(i);
                i.bays.push(bay.clone());
                i.bays.push(bay);
            }),
        ),
        (
            "the bay room `bay` is named by two bays",
            mutate(&|i| {
                let bay = first(i);
                i.bays.push(bay);
            }),
        ),
        (
            "the bay room `bay` is named by no bay",
            mutate(&|i| i.bays.clear()),
        ),
        (
            "the bay `hold` names no bay room",
            mutate(&|i| with_first(i, &|bay| bay.room = cid("hold"))),
        ),
        (
            "the bay `stowage` is the interior's space or store; a bay is its own room",
            mutate(&|i| with_first(i, &|bay| bay.room = cid("stowage"))),
        ),
        (
            "the exit `passage` -> `bay` touches the bay `bay`; a bay has no exits",
            mutate(&|i| {
                i.exits.push(InteriorExitDef {
                    from: cid("passage"),
                    to: cid("bay"),
                    names: vec!["Bay <B>;bay".to_owned(), "Passage;passage".to_owned()],
                });
            }),
        ),
        (
            "the bay `bay` holds 1 to 4 rigs; 0 is not",
            mutate(&|i| with_first(i, &|bay| bay.slot.maximum_items = 0)),
        ),
        (
            "the bay `bay` holds 1 to 4 rigs; 5 is not",
            mutate(&|i| with_first(i, &|bay| bay.slot.maximum_items = 5)),
        ),
        (
            "the bay `bay`'s slot `berths` is the berths slot's; a bay has its own",
            mutate(&|i| with_first(i, &|bay| bay.slot.id = cid("berths"))),
        ),
        (
            "the bay `bay`'s egress label is 1 to 120 bytes; 0 is not",
            mutate(&|i| with_first(i, &|bay| bay.egress.label = String::new())),
        ),
        (
            "the bay `bay`'s egress y_cm is within 10000 cm of the hull; -10001 is not",
            mutate(&|i| with_first(i, &|bay| bay.egress.y_cm = -10_001)),
        ),
        (
            "the bay `bay`'s egress heading_mdeg is below 360000; 360000 is not",
            mutate(&|i| with_first(i, &|bay| bay.egress.heading_mdeg = 360_000)),
        ),
    ];
    for (want, got) in cases {
        assert!(got.contains(want), "want {want:?}, got {got:?}");
    }
    // A second bay room no bay names.
    let stray = mutate(&|i| {
        if let Some(room) = i.rooms.iter().find(|r| r.kind == InteriorRoomKind::Bay) {
            let mut spare = room.clone();
            spare.id = cid("spare");
            i.rooms.push(spare);
        }
    });
    assert!(
        stray.contains("the bay room `spare` is named by no bay"),
        "{stray}"
    );
}
