//! **The web parity golden** (doc 21 cut G.3b): the cockpit picture the web
//! draws — `CockpitPicture::new()` at 640x480 over the committed cockpit and
//! arena fixtures, a fresh picture a case, the look offset alone — hashed
//! natively and pinned in `web/src/lib/fixtures/pixel/cockpit_hashes.json`.
//! The browser's wasm build renders the same cases and must hash the same
//! (`web/src/lib/pixel/pixel.test.ts`), which is "the fixture renders
//! pixel-equal in the browser and the TUI".
//!
//! `PIXEL_BLESS=1 cargo test -p mush-pixel --test web_golden` writes it.

use std::collections::HashMap;

use mush_pixel::Frame;
use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::picture::{Look, Picture};
use serde_json::Value;

/// The looks pinned: straight ahead, swung right, swung left.
const LOOKS: [i64; 3] = [0, 45_000, -90_000];
const W: usize = 640;
const H: usize = 480;

fn feeds() -> HashMap<String, Value> {
    let read = |text: &str| -> Value { serde_json::from_str(text).expect("fixture") };
    [
        (
            COCKPIT_FEED.to_string(),
            read(include_str!(
                "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
            )),
        ),
        (
            ARENA_MAP_FEED.to_string(),
            read(include_str!(
                "../../../web/src/lib/fixtures/cockpit/arena_map.json"
            )),
        ),
    ]
    .into_iter()
    .collect()
}

/// The golden's text: the size, then a hash (hex, 16 digits) a look.
fn golden() -> String {
    let feeds = feeds();
    let cases: Vec<Value> = LOOKS
        .iter()
        .map(|&look_mdeg| {
            let mut frame = Frame::new(W, H);
            let look = Look {
                offset_mdeg: look_mdeg,
                ..Look::default()
            };
            assert!(
                CockpitPicture::new().draw(&mut frame, &feeds, &look),
                "the fixture draws"
            );
            serde_json::json!({ "look_mdeg": look_mdeg, "hash": format!("{:016x}", frame.hash()) })
        })
        .collect();
    let doc = serde_json::json!({ "width": W, "height": H, "cases": cases });
    serde_json::to_string_pretty(&doc).expect("json") + "\n"
}

#[test]
fn the_web_golden_is_the_native_render() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/lib/fixtures/pixel/cockpit_hashes.json");
    let now = golden();
    if std::env::var("PIXEL_BLESS").is_ok() {
        std::fs::create_dir_all(path.parent().expect("dir")).expect("fixtures/pixel");
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
    assert_eq!(hashes.len(), LOOKS.len());
}
