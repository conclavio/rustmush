//! The proving ground's shipped relief (`crates/tacsim/content/atlas/
//! proving_ground.json`), decoded into a [`Terrain`] for the heightfield
//! producer's test and the fixture example (doc 21 cut G.7a).
//!
//! The atlas format belongs to `mush-atlas` (doc 14 §3.5, its
//! `HeightfieldDef`): each `field.*` entry is a tile of `cols × rows`
//! little-endian signed 16-bit samples in base64, the southern row first,
//! scaled by `scale_cm` and raised by `datum_cm`, its south-west corner at
//! `origin` (region metres), with its terrain classes run-length coded as
//! `[cols u16][rows u16]` then `[count u16][class u8]` runs. `mush-pixel` does
//! not depend on the atlas, so this decoder lives with the tests: the engine
//! will be handed heights by a feed (G.7b), not by the content pack.

use mush_pixel::heightfield::Terrain;
use mush_pixel::raycast::{Ground, Map};
use serde_json::Value;

/// The pack, as shipped.
pub const PROVING_GROUND: &str = include_str!("../../../tacsim/content/atlas/proving_ground.json");

/// Standard base64, padding allowed; anything else is skipped.
fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter_map(value) {
        acc = (acc << 6) | c;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// An atlas terrain class code as the raycaster's ground.
fn ground_of(class: u8) -> Ground {
    match class {
        3 => Ground::Grass,
        4 | 15 => Ground::Water,
        1 | 7 => Ground::Road,
        9 | 10 => Ground::Wall,
        13 | 14 => Ground::Woods,
        _ => Ground::Open,
    }
}

/// The proving ground: 256 cells a side at 10 m, every tile's elevations
/// and classes in place, the region's south-west corner at cell (0, 0).
pub fn proving_ground() -> Terrain {
    let doc: Value = serde_json::from_str(PROVING_GROUND).expect("the shipped pack parses");
    let entries = doc["entries"].as_object().expect("entries");
    let region = &entries["region.proving_ground"]["def"];
    let (ox, oy) = (
        region["extent"][0][0].as_i64().expect("extent"),
        region["extent"][0][1].as_i64().expect("extent"),
    );
    let side = ((region["extent"][1][0].as_i64().expect("extent") - ox) / 10) as usize;
    let mut map = Map::new(side, side, 1000);
    let mut elev = vec![(0usize, 0usize, 0i32); 0];
    for (name, entry) in entries {
        if !name.starts_with("field.") {
            continue;
        }
        let def = &entry["def"];
        let cols = def["cols"].as_u64().expect("cols") as usize;
        let rows = def["rows"].as_u64().expect("rows") as usize;
        let res_m = def["resolution_cm"].as_i64().expect("resolution") / 100;
        assert_eq!(res_m, 10, "the arena's 10 m cells");
        let x0 = ((def["origin"][0].as_i64().expect("origin") - ox) / res_m) as usize;
        let y0 = ((def["origin"][1].as_i64().expect("origin") - oy) / res_m) as usize;
        let scale = def["scale_cm"].as_i64().unwrap_or(1) as i32;
        let datum = def["datum_cm"].as_i64().unwrap_or(0) as i32;
        let samples = base64_decode(def["data"].as_str().expect("data"));
        for (i, pair) in samples
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .take(cols * rows)
        {
            let v = i16::from_le_bytes(*pair) as i32;
            elev.push((x0 + i % cols, y0 + i / cols, v * scale + datum));
        }
        if let Some(classes) = def["classes"].as_str() {
            let bytes = base64_decode(classes);
            let mut i = 0usize;
            for run in bytes[4..].as_chunks::<3>().0 {
                let count = u16::from_le_bytes([run[0], run[1]]) as usize;
                let ground = ground_of(run[2]);
                for k in i..i + count {
                    if run[2] != 0 {
                        map.set(x0 + k % cols, y0 + k / cols, ground);
                    }
                }
                i += count;
            }
        }
    }
    let mut terrain = Terrain::new(map);
    for (x, y, cm) in elev {
        terrain.set_elev_cm(x, y, cm);
    }
    terrain
}
