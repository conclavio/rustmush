//! **The relief preview** (doc 21 cut G.7b): first-person views of an arena
//! map frame — heights and all — from a few vantage points, drawn by the
//! production path (`cockpit::terrain_from_feed` and the heightfield), to
//! PNGs. The authoring loop for an arena's landscape:
//!
//! ```text
//! RELIEF_PREVIEW=map.json cargo test -p tacsim --test content g7b_
//! cargo run -p mush-pixel --release --example relief_preview -- map.json out/
//! ```

use mush_pixel::cockpit::terrain_from_feed;
use mush_pixel::raycast::{Camera, Sprite, SpriteKind};
use mush_pixel::{Frame, heightfield, png};
use serde_json::Value;

/// `(name, column, row, heading in degrees, eye above the ground in cm)`.
const VIEWS: &[(&str, f64, f64, u32, i64)] = &[
    ("red_muster_east", 6.5, 97.5, 90, 150),
    ("crossroads_north", 129.5, 108.5, 0, 150),
    ("bowl_rim_south", 98.5, 50.5, 180, 150),
    ("signal_hill_from_barn", 106.5, 175.5, 70, 150),
    ("ford_south_valley", 64.5, 86.0, 180, 150),
    ("biped_crossroads_north", 129.5, 100.5, 20, 900),
    ("blue_muster_west", 249.5, 97.5, 270, 150),
    ("spine_from_shelf", 230.5, 20.5, 315, 150),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: relief_preview <map.json> <out-dir>");
        return;
    };
    let map: Value = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
    {
        Some(map) => map,
        None => {
            eprintln!("{path}: not a map frame");
            return;
        }
    };
    let Some(terrain) = terrain_from_feed(&map) else {
        eprintln!("{path}: no relief in it");
        return;
    };
    let cell = map.get("cell_cm").and_then(Value::as_i64).unwrap_or(1000);
    for &(name, cx, cy, heading, eye) in VIEWS {
        let cam = Camera {
            x_cm: (cx * cell as f64) as i64,
            y_cm: (cy * cell as f64) as i64,
            heading_mdeg: heading * 1_000,
        };
        let ground = i64::from(terrain.elev_cm(cx as i64, cy as i64));
        let tank = Sprite {
            x_cm: cam.x_cm + 4_000,
            y_cm: cam.y_cm + 2_000,
            kind: SpriteKind::Blue,
            pose: None,
        };
        let mut frame = Frame::new(640, 480);
        let t = std::time::Instant::now();
        heightfield::render(&mut frame, &terrain, &cam, Some(ground + eye), &[tank]);
        let us = t.elapsed().as_micros();
        if let Err(e) = std::fs::write(format!("{out}/{name}.png"), png::encode(&frame)) {
            eprintln!("{name}: {e}");
        }
        println!("{name}: {us} us, ground {ground} cm");
    }
}
