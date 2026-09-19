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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mush_atlas::TerrainClass;
use mush_atlas::cells::base64_encode;
use tacsim::definitions::{Catalog, ResourceTable, content};

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
        "6208ec938711ca2f8b2bd878ee059aa03e5e79efbedd215c12d849a50cfb29c1",
    ),
    (
        "tacsim.pack.resolution",
        "aa298a115be0a5a242b907a5acf7007274e44170c02483600819523253ad331e",
    ),
    (
        "tacsim.pack.proving_ground",
        "4a479adb817b217aa17f64e71c6631ea27c427f5c833feb1b5e7139bc3a6c6f6",
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
/// `FIRE` fires (`cockpit::range_band`), and what a pilot *can* be shown is
/// therefore a fact about two packs at once: the cannon's band edges and how far
/// a sight can hold a contact. Cut S.2's band was read against the platform's
/// longest reach and could only ever say `close`; this cut re-based it and the
/// word now reaches `effective` — but not, on the content the game ships, `far`
/// or `beyond`, because the cannon's inner band is 1.5 km and nothing can be held
/// past 900 m (a 600 m sight plus one 300 m mesh hop).
///
/// That gap is **recorded here rather than argued here**: the derivation is cut
/// F.3's brief's own decision, and whether the sight should reach further or the
/// bands should sit closer is content, which is the owner's. What this test is
/// for is that the gap cannot change silently — widen the sight, shorten the
/// cannon's inner band, or give the tank a different default weapon, and this
/// fails and somebody reads the note.
#[test]
fn the_band_word_the_shipped_content_can_print() {
    use tacsim::match_::cockpit::{DEFAULT_WEAPON, range_band};

    let catalog = Catalog::build_game(&[], "tacsim").expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let cell_cm = u64::from(arena.cell_cm);
    // The furthest a contact can be held at: the widest shipped sight, plus one
    // mesh hop off a teammate at the far edge of its own reach.
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
    assert_eq!(furthest_cm, 90_000, "a 600 m sight and a 300 m mesh hop");

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
    // produce. Both of the reachable words really are reachable — which is the
    // half cut S.2 did not have — and each is reachable at a range two tanks on
    // this map will routinely be at.
    let words: std::collections::BTreeSet<&str> = (0..=furthest_cm)
        .step_by(100)
        .map(|range_cm| range_band(range_cm, &bands))
        .collect();
    assert_eq!(
        words.into_iter().collect::<Vec<_>>(),
        vec!["close", "effective"],
        "the band vocabulary a pilot can see on the shipped proving ground"
    );
    assert_eq!(range_band(30_000, &bands), "close", "300 m");
    assert_eq!(range_band(51_000, &bands), "effective", "510 m");
}
