//! **The shipped content, generated and pinned** (16b cut C.2; doc 16 §4
//! "Content packs").
//!
//! Two jobs in one file, because they are the same job:
//!
//! 1. **The generator.** The proving ground's ground is a 256 by 256 authored
//!    class grid and a seeded relief — 65,536 cells of run-length-coded base64
//!    that no human writes by hand. [`atlas_pack`] is the authoring source, and
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
use mush_atlas::cells::base64_encode;
use mush_core::ObjId;
use mush_testkit::TestWorld;
use tacsim::arena::state::Team;
use tacsim::definitions::{Catalog, ResourceTable, content};
use tacsim::match_::{PODS_PER_SIDE, PROVING_GROUND as PROVING_GROUND_ARENA};

mod support;

// ---------------------------------------------------------------------------
// The map
// ---------------------------------------------------------------------------

/// The window's side, in cells (D-16-5).
const SIDE: u16 = 256;
/// One cell, centimetres (doc 16 §3.2).
const CELL_CM: u16 = 1_000;
/// One heightfield entry's side. Four quadrants tile the window; a 256 by 256
/// field of little-endian samples is 175 KB of base64 and `mush_content`'s
/// `MAX_ENTRY_BYTES` is 64 KiB.
const QUAD: u16 = 128;
/// The region's south-west corner, metres. Offset well clear of the shipped lot
/// (`region.limbo`, 0 to 400 m) and of the harness's own relief (10,000 m), so
/// no window can span two regions.
const ORIGIN_M: i64 = 20_000;
/// Where the river runs, in columns, inclusive.
const RIVER: (u16, u16) = (62, 66);
/// Where the ford crosses it, in rows, inclusive.
const FORD: (u16, u16) = (92, 102);
/// The east-west road, in rows, inclusive.
const ROAD_EW: (u16, u16) = (96, 98);
/// The north-south road, in columns, inclusive.
const ROAD_NS: (u16, u16) = (128, 130);

/// The three built compounds: `(x0, x1, y0, y1)`, inclusive. Each is a ring of
/// `Wall` around `Floor` with one `Door` in the middle of its south wall. None
/// of them touches a road band, which is checked by a test below rather than by
/// the author's eye.
const BUILDINGS: [(u16, u16, u16, u16); 3] = [
    (150, 170, 36, 52),
    (96, 116, 186, 202),
    (200, 216, 140, 156),
];

/// Light woods: `(x0, x1, y0, y1)`.
const WOODS_LIGHT: [(u16, u16, u16, u16); 2] = [(20, 58, 150, 200), (160, 210, 40, 80)];
/// Heavy woods, inside the light ones.
const WOODS_HEAVY: [(u16, u16, u16, u16); 2] = [(30, 48, 165, 185), (175, 193, 50, 70)];
/// Broken ground.
const RUBBLE: [(u16, u16, u16, u16); 1] = [(100, 112, 60, 72)];
/// Bare earth.
const BARE: [(u16, u16, u16, u16); 2] = [(210, 255, 0, 36), (0, 40, 0, 24)];

fn within(x: u16, y: u16, box_: (u16, u16, u16, u16)) -> bool {
    x >= box_.0 && x <= box_.1 && y >= box_.2 && y <= box_.3
}

/// The authored class of one cell. The order of the tests **is** the paint
/// order: a building beats a road, a road beats the ground under it, and the
/// ford beats the river it crosses.
fn class_at(x: u16, y: u16) -> TerrainClass {
    for building in BUILDINGS {
        if within(x, y, building) {
            let door_x = (building.0 + building.1) / 2;
            if y == building.2 && x == door_x {
                return TerrainClass::Door;
            }
            let edge = x == building.0 || x == building.1 || y == building.2 || y == building.3;
            return if edge {
                TerrainClass::Wall
            } else {
                TerrainClass::Floor
            };
        }
    }
    let in_river = x >= RIVER.0 && x <= RIVER.1;
    if in_river {
        return if y >= FORD.0 && y <= FORD.1 {
            // The crossing: shallow enough for a tracked unit, at a cost.
            TerrainClass::Water
        } else {
            TerrainClass::WaterDeep
        };
    }
    if (y >= ROAD_EW.0 && y <= ROAD_EW.1) || (x >= ROAD_NS.0 && x <= ROAD_NS.1) {
        return TerrainClass::Road;
    }
    for patch in WOODS_HEAVY {
        if within(x, y, patch) {
            return TerrainClass::WoodsHeavy;
        }
    }
    for patch in WOODS_LIGHT {
        if within(x, y, patch) {
            return TerrainClass::WoodsLight;
        }
    }
    for patch in RUBBLE {
        if within(x, y, patch) {
            return TerrainClass::Rubble;
        }
    }
    for patch in BARE {
        if within(x, y, patch) {
            return TerrainClass::Ground;
        }
    }
    TerrainClass::Grass
}

/// The seeded relief, centimetres against the region datum, quantised to whole
/// metres so a tile's runs stay long. Three summed sinusoids at three periods:
/// deterministic, smooth enough that no adjacent pair is past a tracked unit's
/// 45 % slope limit, and two metres lower wherever the river runs.
fn elev_at(x: u16, y: u16) -> i16 {
    let fx = f64::from(x);
    let fy = f64::from(y);
    let raw =
        200.0 * (fx / 40.0).sin() + 150.0 * (fy / 55.0).cos() + 100.0 * ((fx + fy) / 70.0).sin();
    let mut cm = (raw / 100.0).round() * 100.0;
    if matches!(
        class_at(x, y),
        TerrainClass::Water | TerrainClass::WaterDeep
    ) {
        cm -= 200.0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let cm = cm as i32;
    i16::try_from(cm).unwrap_or(0)
}

/// One quadrant's samples as base64 over little-endian signed 16-bit
/// centimetres, the southern row first (`HeightfieldEncoding::Le16`).
fn quadrant_le16(qx: u16, qy: u16) -> String {
    let mut bytes = Vec::with_capacity(usize::from(QUAD) * usize::from(QUAD) * 2);
    for y in 0..QUAD {
        for x in 0..QUAD {
            bytes.extend_from_slice(&elev_at(qx * QUAD + x, qy * QUAD + y).to_le_bytes());
        }
    }
    base64_encode(&bytes)
}

/// One quadrant's classes, in C.1's companion encoding: the tile cell RLE with
/// a class code as its only payload — `[cols u16][rows u16]` and then
/// `[count u16][class u8]` per run, all little-endian, southern row first.
fn quadrant_classes(qx: u16, qy: u16) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&QUAD.to_le_bytes());
    bytes.extend_from_slice(&QUAD.to_le_bytes());
    let mut run: Option<(u8, u16)> = None;
    let flush = |bytes: &mut Vec<u8>, run: Option<(u8, u16)>| {
        if let Some((code, n)) = run {
            bytes.extend_from_slice(&n.to_le_bytes());
            bytes.push(code);
        }
    };
    for y in 0..QUAD {
        for x in 0..QUAD {
            let code = class_at(qx * QUAD + x, qy * QUAD + y).code();
            match run {
                Some((prev, n)) if prev == code && n < u16::MAX => run = Some((prev, n + 1)),
                other => {
                    flush(&mut bytes, other);
                    run = Some((code, 1));
                }
            }
        }
    }
    flush(&mut bytes, run);
    base64_encode(&bytes)
}

/// **The arena's ground, as an ordinary atlas region pack.**
///
/// One `region`, one `locality` over it, and four `heightfield` entries whose
/// tiles cover the window exactly. Nothing in it is TacSim's: the atlas's own
/// appliers make every record, which is what lets
/// `tacsim::definitions::arena::from_pack` read them back with no private
/// channel between the two crates.
fn atlas_pack() -> String {
    let lo = ORIGIN_M;
    let hi = ORIGIN_M + i64::from(SIDE) * i64::from(CELL_CM) / 100;
    let mut entries = serde_json::Map::new();
    entries.insert(
        "region.proving_ground".to_string(),
        serde_json::json!({
            "kind": "region",
            "def": {
                "name": "The Proving Ground",
                "kind": "surface",
                "body": "luna",
                "extent": [[lo, lo], [hi, hi]],
                "datum_m": 0.0,
                "background": {
                    "surface": "ground",
                    "mix": [],
                    "relief_cm": 0,
                    "relief_period_cm": 1000,
                    "water_cm": null,
                    "seed": 2350
                },
                "ground_resolution_cm": CELL_CM
            }
        }),
    );
    entries.insert(
        "locality.proving_ground".to_string(),
        serde_json::json!({
            "kind": "locality",
            "def": {
                "region": "region.proving_ground",
                "name": "The Proving Ground",
                "kind": "lot",
                "footprint": [[lo, lo], [hi, lo], [hi, hi], [lo, hi]]
            }
        }),
    );
    for qy in 0..(SIDE / QUAD) {
        for qx in 0..(SIDE / QUAD) {
            let ox = ORIGIN_M + i64::from(qx) * i64::from(QUAD) * i64::from(CELL_CM) / 100;
            let oy = ORIGIN_M + i64::from(qy) * i64::from(QUAD) * i64::from(CELL_CM) / 100;
            entries.insert(
                format!("field.q{qx}_{qy}"),
                serde_json::json!({
                    "kind": "heightfield",
                    "def": {
                        "region": "region.proving_ground",
                        "origin": [ox, oy],
                        "resolution_cm": CELL_CM,
                        "cols": QUAD,
                        "rows": QUAD,
                        "encoding": "le16",
                        "data": quadrant_le16(qx, qy),
                        "scale_cm": 1,
                        "datum_cm": 0,
                        "classes": quadrant_classes(qx, qy)
                    }
                }),
            );
        }
    }
    let doc = serde_json::json!({
        "manifest": {
            "id": content::PROVING_GROUND_ATLAS_PACK,
            "version": 1,
            "kind": "region",
            "digest": UNSIGNED,
            "requires": []
        },
        "entries": entries
    });
    let unsigned = serde_json::to_string_pretty(&doc).expect("the pack serializes");
    sign(&format!("{unsigned}\n"))
}

/// The placeholder a generated or hand-authored pack carries until it is signed.
const UNSIGNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Compute a pack's own digest and stamp it into its manifest, exactly as the
/// authoring half of `mush_content::load_pack` does.
fn sign(json: &str) -> String {
    let parsed = mush_content::parse_pack(json).expect("the authored pack parses");
    let digest = parsed.computed_digest().expect("the pack digests");
    json.replace(UNSIGNED, &digest.to_hex())
}

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
const PACK_FILES: [&str; 8] = [
    "ammunition/he_105.json",
    "ammunition/ball_762.json",
    "weapons/cannon_105.json",
    "weapons/mg_coax.json",
    "sensors/visual_mk1.json",
    "units/line_tank.json",
    "resolution.json",
    "arenas/proving_ground.json",
];

/// **The digest pins.** A content change moves one of these, and moving one is a
/// reviewed diff.
const DIGESTS: [(&str, &str); 9] = [
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
        "tacsim.pack.line_tank",
        "607a1d6c9ab1dfb182df4b64983719d7e83df492fe0d63e6102679c216d904ee",
    ),
    (
        "tacsim.pack.resolution",
        "aa298a115be0a5a242b907a5acf7007274e44170c02483600819523253ad331e",
    ),
    (
        "tacsim.pack.proving_ground",
        "fd5860b51a60376725ec5fbe86797e8acef969db9a34fc70c87a23ea56c21b08",
    ),
    (
        "tacsim.proving_ground",
        "b0bcabf34632799f1468821c1ddfeefb4acbe30282b626ca5e02fbf32f659706",
    ),
];

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_atlas_pack_is_this_generators_output() {
    pin_file("atlas/proving_ground.json", &atlas_pack());
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
    // The four key spots, the entry zones and every waypoint must stand on
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
        6,
        "the shipped unit schemas: the line tank and doc 16 §9's five extensions"
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
        units.push(custody_unit(
            &catalog,
            cells,
            id,
            Team::Red,
            pose,
            0,
            Controller::Reflex,
            0,
        ));
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

    // And again with the weapon a bare `FIRE` fires emptied. Every shipped unit
    // carries a second gun, so every one of them keeps fighting — at its
    // coaxial's own first band, which on this content is 600 m and therefore
    // exactly the sight: nought margin, and the day a shorter gun is authored
    // this table is where it shows.
    let dry: Vec<SmolStr> = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
            let Some(state) = arena.unit(id) else {
                panic!("{name} is in the arena")
            };
            state
                .default_weapon()
                .map(|weapon| weapon.id.clone())
                .unwrap_or_else(|| panic!("{name} carries a default weapon"))
        })
        .collect();
    for (at, weapon) in dry.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        if let Some(state) = arena.unit_mut(id) {
            state.ammo.insert(weapon.clone(), 0);
        }
    }
    assert_eq!(
        dry.iter().map(SmolStr::to_string).collect::<Vec<_>>(),
        vec!["cannon", "cannon", "cannon", "cannon", "laser", "cannon"],
        "the weapon a bare `FIRE` fires: the cannon, or the first in component \
         order on a hull that carries none"
    );
    assert_eq!(
        table(&arena),
        vec![
            ("tacsim.unit.biped_mk1".to_owned(), 60_000, 60_000, 60_000),
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
        ],
        "and with the default weapon dry: every one of them fights on its coaxial"
    );
    for (at, name) in names.iter().enumerate() {
        let id = ObjId(u32::try_from(at).unwrap_or(0) + 10);
        let Some(state) = arena.unit(id) else {
            panic!("{name} is in the arena")
        };
        let loaded = state
            .loaded_weapon()
            .map(|weapon: &WeaponSummary| weapon.id.to_string());
        assert_eq!(
            loaded,
            Some("machine_gun".to_owned()),
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
/// and nothing latched moved** (16b cut Q.8).
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
    assert_eq!(
        fielded.len(),
        stock.len() + 10,
        "eight shipped packs and doc 16 section 9's ten extension files"
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
        checked >= 6,
        "every fielded unit's modes were read: {checked}"
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
/// and are asserted by its own test.
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
            content::LINE_TANK,
            content::LINE_TANK_ECCM,
            content::LINE_TANK_ECM,
            content::LINE_TANK_LASER,
            content::LINE_TANK_THERMAL,
        ],
        "the five tanks, and nothing else with two tracks"
    );
}

/// **16b cut M.4 (16c `B-47`): the five tank packs are version 2.** Their bytes
/// moved (the pair rule), and `boot_reconcile` refuses a published `(id,
/// version)` whose digest moved, so the version had to. The biped is not among
/// them: its legs carried the pair rule since X.2, and nothing in its file moved.
#[test]
fn m4_the_five_tank_packs_are_version_two_and_the_biped_is_not_moved() {
    for (json, id) in [
        (content::LINE_TANK_JSON, "tacsim.pack.line_tank"),
        (content::LINE_TANK_LASER_JSON, "tacsim.pack.line_tank_laser"),
        (
            content::LINE_TANK_THERMAL_JSON,
            "tacsim.pack.line_tank_thermal",
        ),
        (content::LINE_TANK_ECM_JSON, "tacsim.pack.line_tank_ecm"),
        (content::LINE_TANK_ECCM_JSON, "tacsim.pack.line_tank_eccm"),
    ] {
        let pack = tacsim::pack::load_pack(json).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(pack.manifest().id().as_str(), id);
        assert_eq!(pack.manifest().version(), 2, "{id} is version 2");
    }
    let biped = content::extension_sources_of(&content::EXTENSIONS_BIPED)
        .into_iter()
        .find_map(|source| {
            let pack = tacsim::pack::load_pack(&source.bytes).ok()?;
            (pack.manifest().id().as_str() == "tacsim.pack.biped_mk1").then_some(pack)
        })
        .unwrap_or_else(|| panic!("the biped ships as an extension"));
    assert_eq!(
        biped.manifest().version(),
        1,
        "the biped is not re-versioned"
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
    // 16b cut O.1 moved the version to 4 (the Hold declaration); the twelve
    // tanks are Z.3's and unchanged.
    assert_eq!(pack.manifest().version(), 4, "16b cut O.1's version");
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
    // that takes it lands at 4.
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@4"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(arena_latch(&tw), Some(4));
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
        applied.contains("Applied tacsim.pack.proving_ground@4"),
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
