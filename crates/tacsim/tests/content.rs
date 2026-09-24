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
        "607a1d6c9ab1dfb182df4b64983719d7e83df492fe0d63e6102679c216d904ee",
    ),
    (
        "tacsim.pack.resolution",
        "aa298a115be0a5a242b907a5acf7007274e44170c02483600819523253ad331e",
    ),
    (
        "tacsim.pack.proving_ground",
        "0ecc97a0922b2fef3d18fd2b510c2a17cdef1613de10ee56b9fce99520924160",
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
/// wide and the muster needs four, so **all eight** slots stand in it now, and
/// the run asserted below is **every** slot's.
///
/// Every assertion below is read off the pack's own data rather than written
/// down twice, and every slot is found by the id the roster keys on
/// (`red_1` … `blue_4`) rather than by where it happens to sit in the array, so
/// the next person who edits the map cannot put a muster back behind the river
/// without this failing.
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
    for index in 1..=4_u8 {
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

    // And no two slots of a side are on the same or an adjacent row, so a tank
    // pulling away has a clear cell beside it.
    for team in ["red", "blue"] {
        let mut rows: Vec<u16> = (1..=4_u8)
            .map(|index| slot(&format!("{team}_{index}")).y)
            .collect();
        rows.sort_unstable();
        for pair in rows.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert!(
                upper.saturating_sub(lower) >= 2,
                "the {team} muster puts rows {lower} and {upper} side by side"
            );
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
