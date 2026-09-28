//! **The proving ground's generator** (16b cut C.2; moved here by 16b cut
//! AE.1).
//!
//! The arena's ground is a 256 by 256 authored class grid and a seeded relief —
//! 65,536 cells of run-length-coded base64 that no human writes by hand — and
//! this module is its authoring source. `tests/content.rs` asserts the file in
//! the tree is exactly [`atlas_pack_at`]`(true)`'s output and re-writes it under
//! `TACSIM_WRITE_CONTENT=1`.
//!
//! **Why it lives in `tests/support` now** (16b cut AE.1): the atlas pack moved
//! from version 1 to version 2 (The Bluff), and a world that booted version 1
//! and takes version 2 off the shelf is the live-world path the deploy runs.
//! `tests/match_lifecycle.rs`'s shelf test has to boot version 1, and version 1
//! is not in the tree any more — it is **this generator with the bluff turned
//! off**, [`atlas_pack_at`]`(false)`, held to its old pin
//! (`support::ATLAS_V1_DIGEST`). An integration test cannot import another
//! integration test, so the generator is here, where every test binary that
//! declares `mod support` can reach it.

use mush_atlas::TerrainClass;
use mush_atlas::cells::base64_encode;
use tacsim::definitions::content;

/// The window's side, in cells (D-16-5).
pub const SIDE: u16 = 256;
/// One cell, centimetres (doc 16 §3.2).
pub const CELL_CM: u16 = 1_000;
/// One heightfield entry's side. Four quadrants tile the window; a 256 by 256
/// field of little-endian samples is 175 KB of base64 and `mush_content`'s
/// `MAX_ENTRY_BYTES` is 64 KiB.
pub const QUAD: u16 = 128;
/// The region's south-west corner, metres. Offset well clear of the shipped lot
/// (`region.limbo`, 0 to 400 m) and of the harness's own relief (10,000 m), so
/// no window can span two regions.
pub const ORIGIN_M: i64 = 20_000;
/// Where the river runs, in columns, inclusive.
pub const RIVER: (u16, u16) = (62, 66);
/// Where the ford crosses it, in rows, inclusive.
pub const FORD: (u16, u16) = (92, 102);
/// The east-west road, in rows, inclusive.
pub const ROAD_EW: (u16, u16) = (96, 98);
/// The north-south road, in columns, inclusive.
pub const ROAD_NS: (u16, u16) = (128, 130);

/// The three built compounds: `(x0, x1, y0, y1)`, inclusive. Each is a ring of
/// `Wall` around `Floor` with one `Door` in the middle of its south wall. None
/// of them touches a road band, which is checked by a test in
/// `tests/content.rs` rather than by the author's eye.
pub const BUILDINGS: [(u16, u16, u16, u16); 3] = [
    (150, 170, 36, 52),
    (96, 116, 186, 202),
    (200, 216, 140, 156),
];

/// Light woods: `(x0, x1, y0, y1)`.
pub const WOODS_LIGHT: [(u16, u16, u16, u16); 2] = [(20, 58, 150, 200), (160, 210, 40, 80)];
/// Heavy woods, inside the light ones.
pub const WOODS_HEAVY: [(u16, u16, u16, u16); 2] = [(30, 48, 165, 185), (175, 193, 50, 70)];
/// Broken ground.
pub const RUBBLE: [(u16, u16, u16, u16); 1] = [(100, 112, 60, 72)];
/// Bare earth.
pub const BARE: [(u16, u16, u16, u16); 2] = [(210, 255, 0, 36), (0, 40, 0, 24)];

/// **The Bluff's top** (16b cut AE.1; 16c `T-18`): `(x0, x1, y0, y1)`,
/// inclusive — a grass mesa north-east of the crossroads, clear of both entry
/// zones (`x ≤ 15`, `x ≥ 240`), the Hold square (`123..=135 × 91..=103`; `141
/// > 135`), both roads, the ford, every waypoint node and every muster slot.
///
/// This is the authored site, rows 105 to 118. The lance lane's blue rows 105
/// and 107 at column 156 meet its east rim and route round it; the lane's W8
/// figures at its defaults did not move for that (16b cut AE.1's probes ran it
/// on this mesa, on a shorter one and on none, and read the same fight — the
/// move was the per-row prices, 16c `B-58`). The shorter mesa, rows 109 to 118
/// (clear of every lance row, 107 and below), is the alternative owner question
/// `Q-AE5` (16b section 9, cut AE) keeps open, not the default.
pub const BLUFF_TOP: (u16, u16, u16, u16) = (141, 147, 105, 118);

/// The top's elevation, centimetres against the region datum — **absolute**,
/// not added to the relief under it.
///
/// The seeded relief round the mesa is a −200 shelf: every rim neighbour (west
/// `x 140`, south `y 104`, east `x 148`, for `y 105..=118`) stands at −200, so
/// **every rim edge rises 700 cm — 70 per cent orthogonal, 49.5 diagonal** (36
/// orthogonal and 68 diagonal edges, 104 in all). A tracked hull (45) and a car
/// (25) are refused every way; a walker (70, and `too_steep` is strictly
/// greater) climbs it orthogonally and through the corners. The one corner
/// `(147, 118) → (148, 119)` rises 800 (`(148, 119)` is −300; 56.6 per cent
/// diagonal): a tank is refused that diagonal by its own slope and a walker is
/// not. **The search's shoulder rule has its own
/// cases here**, diagonals that are inside the limit themselves and refused
/// only because a shoulder is a rim: `(140, 118) → (141, 119)` (a 33 per cent
/// diagonal whose shoulder `(141, 118)` is 70 per cent up) and `(148, 118) →
/// (147, 119)` (flat, beside the top's corner `(147, 118)`).
pub const BLUFF_TOP_CM: i16 = 500;

/// **The ramp down the Bluff's north side**, each box `(x0, x1, y0, y1)` and its
/// absolute elevation: row 119 at 270 and row 120 at 40, under a row 121 at
/// −200. Its three steps rise 230, 230 and 240 (23 to 24 per cent), under a
/// car's 25, so every chassis takes the ramp from the north.
///
/// **Its flanks, which the tests pin.** Row 119's, `(140, 119)` and `(147,
/// 119)`, stand at −200: 470 cm, 47 per cent, refused to tanks and cars and
/// open to legs. Row 120's west flank `(140, 120)` is −200 (240 cm, 24 per
/// cent: every chassis may enter the ramp's foot from the west). Its east flank
/// `(147, 120)` is −300, a 340 cm step at 34 per cent onto `(146, 120)`: a tank
/// driven west along row 120 takes it, and a car does not. `(147, 121)` is −300
/// too, but it meets the foot only diagonally (340 cm over the diagonal, 24 per
/// cent), as `(147, 119)` does from −200 (240 cm, 17 per cent): motion lets a
/// pilot's car take either diagonal, and the car's search refuses both by the
/// shoulder rule, since `(147, 120)`'s 34 per cent is a shoulder of the one and
/// `(146, 119)`'s 47 of the other. Every edge on the whole map past 45 per cent
/// between two passable cells is one of these 107 — the 104 rims, the corner,
/// the two row-119 flanks — all inside `x 140..=148, y 104..=121`.
pub const BLUFF_RAMP: [((u16, u16, u16, u16), i16); 2] =
    [((141, 146, 119, 119), 270), ((141, 146, 120, 120), 40)];

/// Whether `(x, y)` is inside `box_`, `(x0, x1, y0, y1)` inclusive.
#[must_use]
pub fn within(x: u16, y: u16, box_: (u16, u16, u16, u16)) -> bool {
    x >= box_.0 && x <= box_.1 && y >= box_.2 && y <= box_.3
}

/// The authored class of one cell. The order of the tests **is** the paint
/// order: a building beats a road, a road beats the ground under it, and the
/// ford beats the river it crosses. (The Bluff is grass: it moves no class.)
#[must_use]
pub fn class_at(x: u16, y: u16) -> TerrainClass {
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
///
/// **With `bluff`** (16b cut AE.1; version 2 of the atlas pack), then The
/// Bluff: a cell inside [`BLUFF_TOP`] stands at [`BLUFF_TOP_CM`] and a cell
/// inside one of [`BLUFF_RAMP`]'s boxes at that box's centimetres, both
/// absolute. Without it this is version 1's relief to the centimetre.
#[must_use]
pub fn elev_at(bluff: bool, x: u16, y: u16) -> i16 {
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
    let cm = i16::try_from(cm).unwrap_or(0);
    if bluff {
        if within(x, y, BLUFF_TOP) {
            return BLUFF_TOP_CM;
        }
        for (box_, ramp_cm) in BLUFF_RAMP {
            if within(x, y, box_) {
                return ramp_cm;
            }
        }
    }
    cm
}

/// One quadrant's samples as base64 over little-endian signed 16-bit
/// centimetres, the southern row first (`HeightfieldEncoding::Le16`).
fn quadrant_le16(bluff: bool, qx: u16, qy: u16) -> String {
    let mut bytes = Vec::with_capacity(usize::from(QUAD) * usize::from(QUAD) * 2);
    for y in 0..QUAD {
        for x in 0..QUAD {
            bytes.extend_from_slice(&elev_at(bluff, qx * QUAD + x, qy * QUAD + y).to_le_bytes());
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
///
/// **`bluff`** (16b cut AE.1): `true` is **version 2**, the shipped pack, with
/// The Bluff in its relief; `false` is **version 1** to the byte — the pack a
/// world that has not taken version 2 off the shelf still holds, pinned by
/// `support::ATLAS_V1_DIGEST`. The class grid and the tile sizes are the same
/// in both: the bluff moves samples in `field.q1_0` alone (`x 141..=147 ≥ 128`,
/// `y 105..=120 < 128`).
#[must_use]
pub fn atlas_pack_at(bluff: bool) -> String {
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
                        "data": quadrant_le16(bluff, qx, qy),
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
            "version": if bluff { 2 } else { 1 },
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
pub const UNSIGNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Compute a pack's own digest and stamp it into its manifest, exactly as the
/// authoring half of `mush_content::load_pack` does.
#[must_use]
pub fn sign(json: &str) -> String {
    let parsed = mush_content::parse_pack(json).expect("the authored pack parses");
    let digest = parsed.computed_digest().expect("the pack digests");
    json.replace(UNSIGNED, &digest.to_hex())
}
