//! **The overhead's golden** (doc 21 cut G.12b, §7.4): the cockpit's second
//! camera drawn over the committed cockpit and arena fixtures — the flat yard
//! and the yard with relief (G.7b's heights shade it), two looks, the text
//! cockpit's 640x276 box and the full 640x480 — hashed and pinned in
//! `tests/golden/overhead_hashes.json`, so a change to the scene is a
//! deliberate bless and never a surprise.
//!
//! `PIXEL_BLESS=1 cargo test -p mush-pixel --test overhead_golden` writes it.

use std::collections::HashMap;

use mush_pixel::Frame;
use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED};
use mush_pixel::overhead::OverheadPicture;
use mush_pixel::picture::{Look, Picture};
use serde_json::Value;

/// The looks pinned: straight ahead, and swung right with the hull a little
/// round toward an order.
const LOOKS: [(i64, i64, Option<i64>); 2] = [(0, 0, None), (45_000, 10_000, Some(90_000))];
/// The sizes pinned: the text cockpit's box at 120 by 40, and the 4:3 target.
const SIZES: [(usize, usize); 2] = [(640, 276), (640, 480)];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/fixtures")
}

fn feeds(map: &str) -> HashMap<String, Value> {
    let read = |path: &str| -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(fixture_dir().join(path)).expect("the fixture"),
        )
        .expect("json")
    };
    let mut arena = match map {
        "relief" => read("pixel/arena_map_relief.json"),
        _ => read("cockpit/arena_map.json"),
    };
    if map == "flat"
        && let Some(m) = arena.as_object_mut()
    {
        m.remove("elev");
    }
    [
        (COCKPIT_FEED.to_string(), read("cockpit/cockpit_frame.json")),
        (ARENA_MAP_FEED.to_string(), arena),
    ]
    .into_iter()
    .collect()
}

fn golden() -> String {
    let mut cases = Vec::new();
    for map in ["flat", "relief"] {
        let feeds = feeds(map);
        for (w, h) in SIZES {
            for (offset, hull, ordered) in LOOKS {
                let mut frame = Frame::new(w, h);
                let look = Look {
                    offset_mdeg: offset,
                    hull_offset_mdeg: hull,
                    ordered_mdeg: ordered,
                    ..Look::default()
                };
                assert!(
                    OverheadPicture::new().draw(&mut frame, &feeds, &look),
                    "the fixture draws"
                );
                cases.push(serde_json::json!({
                    "map": map,
                    "size": [w, h],
                    "look_mdeg": offset,
                    "hash": format!("{:016x}", frame.hash()),
                }));
            }
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({ "cases": cases })).expect("json") + "\n"
}

#[test]
fn the_overhead_golden_is_pinned() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/overhead_hashes.json");
    let now = golden();
    if std::env::var("PIXEL_BLESS").is_ok() {
        std::fs::write(&path, &now).expect("write the golden");
    }
    let pinned = std::fs::read_to_string(&path).expect("the golden (PIXEL_BLESS=1 writes it)");
    assert_eq!(
        pinned.replace("\r\n", "\n"),
        now,
        "PIXEL_BLESS=1 after an overhead change"
    );
    let doc: Value = serde_json::from_str(&now).expect("json");
    let hashes: std::collections::BTreeSet<&str> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter_map(|c| c["hash"].as_str())
        .collect();
    assert_eq!(hashes.len(), 8, "eight pictures, all different");
}
