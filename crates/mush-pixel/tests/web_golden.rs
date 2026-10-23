//! **The web parity golden** (doc 21 cut G.3b): the cockpit picture the web
//! draws — `CockpitPicture::new()` at 640x480 over the committed cockpit and
//! arena fixtures, a fresh picture a case, the look offset alone — hashed
//! natively and pinned in `web/src/lib/fixtures/pixel/cockpit_hashes.json`.
//! The browser's wasm build renders the same cases and must hash the same
//! (`web/src/lib/pixel/pixel.test.ts`), which is "the fixture renders
//! pixel-equal in the browser and the TUI".
//!
//! Since doc 21 cut G.7b the golden has a second map: the same fixture yard
//! with relief (`web/src/lib/fixtures/pixel/arena_map_relief.json`, an
//! `elev` field the bless writes from a formula), so the browser is held to
//! the heightfield and its baked colour map as well as to the raycaster.
//!
//! `PIXEL_BLESS=1 cargo test -p mush-pixel --test web_golden` writes both.

use std::collections::HashMap;

use mush_pixel::Frame;
use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::picture::{Look, Picture};
use serde_json::Value;

/// The looks pinned: straight ahead, swung right, swung left.
const LOOKS: [i64; 3] = [0, 45_000, -90_000];
const W: usize = 640;
const H: usize = 480;

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/fixtures")
}

/// The fixture yard with relief: rolling ground and a hill, every cell's
/// height in the feed's own encoding (zigzag-LEB128 deltas, base64).
fn relief_map() -> Value {
    let mut map: Value = serde_json::from_str(include_str!(
        "../../../web/src/lib/fixtures/cockpit/arena_map.json"
    ))
    .expect("fixture");
    let (w, h) = (
        map["width"].as_i64().unwrap_or(0),
        map["height"].as_i64().unwrap_or(0),
    );
    let mut bytes = Vec::new();
    let mut last = 0i64;
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64, y as f64);
            let roll = 600.0 * (fx / 9.0).sin() * (fy / 7.0).cos();
            let hill = 1_800.0 * (-((fx - 60.0).powi(2) + (fy - 30.0).powi(2)) / 150.0).exp();
            let elev = (roll + hill).round() as i64;
            let delta = elev - last;
            last = elev;
            let mut zig = ((delta << 1) ^ (delta >> 63)) as u64;
            loop {
                let low = (zig & 0x7f) as u8;
                zig >>= 7;
                if zig == 0 {
                    bytes.push(low);
                    break;
                }
                bytes.push(low | 0x80);
            }
        }
    }
    map["elev"] = serde_json::json!({
        "encoding": "zigzag-leb128-delta",
        "unit_cm": 1,
        "data": mush_pixel::kitty::base64(&bytes),
    });
    map
}

fn feeds(map: &str) -> HashMap<String, Value> {
    let read = |text: &str| -> Value { serde_json::from_str(text).expect("fixture") };
    let arena = match map {
        "relief" => read(
            &std::fs::read_to_string(fixture_dir().join("pixel/arena_map_relief.json"))
                .expect("the relief fixture (PIXEL_BLESS=1 writes it)"),
        ),
        // The flat case is the raycaster's: the yard without its (all-zero)
        // heights, as a server before G.7b sends it.
        _ => {
            let mut flat = read(include_str!(
                "../../../web/src/lib/fixtures/cockpit/arena_map.json"
            ));
            if let Some(map) = flat.as_object_mut() {
                map.remove("elev");
            }
            flat
        }
    };
    [
        (
            COCKPIT_FEED.to_string(),
            read(include_str!(
                "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
            )),
        ),
        (ARENA_MAP_FEED.to_string(), arena),
    ]
    .into_iter()
    .collect()
}

/// The golden's text: the size, then a hash (hex, 16 digits) a map and look.
fn golden() -> String {
    let cases: Vec<Value> = ["flat", "relief"]
        .into_iter()
        .flat_map(|map| LOOKS.iter().map(move |&look| (map, look)))
        .map(|(map, look_mdeg)| {
            let feeds = feeds(map);
            let mut frame = Frame::new(W, H);
            let look = Look {
                offset_mdeg: look_mdeg,
                ..Look::default()
            };
            assert!(
                CockpitPicture::new().draw(&mut frame, &feeds, &look),
                "the fixture draws"
            );
            serde_json::json!({ "map": map, "look_mdeg": look_mdeg, "hash": format!("{:016x}", frame.hash()) })
        })
        .collect();
    let doc = serde_json::json!({ "width": W, "height": H, "cases": cases });
    serde_json::to_string_pretty(&doc).expect("json") + "\n"
}

#[test]
fn the_web_golden_is_the_native_render() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/lib/fixtures/pixel/cockpit_hashes.json");
    if std::env::var("PIXEL_BLESS").is_ok() {
        std::fs::create_dir_all(path.parent().expect("dir")).expect("fixtures/pixel");
        let relief = serde_json::to_string_pretty(&relief_map()).expect("json") + "\n";
        std::fs::write(fixture_dir().join("pixel/arena_map_relief.json"), relief)
            .expect("write the relief fixture");
    }
    let now = golden();
    if std::env::var("PIXEL_BLESS").is_ok() {
        std::fs::write(&path, &now).expect("write the golden");
    }
    let pinned = std::fs::read_to_string(&path).expect("the golden (PIXEL_BLESS=1 writes it)");
    assert_eq!(
        pinned.replace("\r\n", "\n"),
        now,
        "PIXEL_BLESS=1 after a picture change"
    );
    // Three looks, three pictures.
    let doc: Value = serde_json::from_str(&now).expect("json");
    let hashes: std::collections::BTreeSet<&str> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter_map(|c| c["hash"].as_str())
        .collect();
    assert_eq!(hashes.len(), 2 * LOOKS.len(), "six pictures, all different");
}
