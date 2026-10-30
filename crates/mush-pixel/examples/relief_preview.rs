//! **The relief preview** (doc 21 cut G.7b): first-person, overhead, and 2.5D
//! tactical views of an arena map frame — heights and all — from a few vantage
//! points, drawn by the production path (`cockpit::terrain_from_feed`,
//! `CockpitPicture`, `OverheadPicture`, and `Iso25DPicture`), to PNGs.
//!
//! ```text
//! RELIEF_PREVIEW=map.json cargo test -p tacsim --test content g7b_
//! cargo run -p mush-pixel --release --example relief_preview -- map.json out/
//! ```

use std::collections::HashMap;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture, terrain_from_feed};
use mush_pixel::iso25d::Iso25DPicture;
use mush_pixel::overhead::OverheadPicture;
use mush_pixel::picture::{Look, Picture};
use mush_pixel::raycast::{Camera, Sprite, SpriteKind};
use mush_pixel::{Frame, heightfield, png};
use serde_json::{Value, json};

/// `(name, column, row, heading in degrees, eye above the ground in cm)`.
const VIEWS: &[(&str, f64, f64, u32, i64)] = &[
    ("red_muster_east", 6.5, 97.5, 90, 150),
    ("ford_pass_east", 58.5, 97.5, 90, 150),
    ("crossroads_north", 129.5, 108.5, 0, 150),
    ("bowl_rim_south", 98.5, 116.5, 180, 150),
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
    let _ = std::fs::create_dir_all(&out);
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

        // Also render the three live cockpit views (FirstPerson + HUD, Overhead, and 2.5D Tactical)
        let cockpit_frame = json!({
            "seated": true,
            "team": "red",
            "unit_type": if eye > 300 { "tacsim.unit.biped_mk1" } else { "tacsim.unit.line_tank" },
            "x_cm": cam.x_cm,
            "y_cm": cam.y_cm,
            "z_cm": ground,
            "sensor_height_cm": eye,
            "heading_mdeg": cam.heading_mdeg,
            "speed_cms": 700,
            "max_speed_cms": 1000,
            "sight_m": 600.0,
            "lock": "BA",
            "lock_designator": "BA",
            "weapons": [
                {"name": "cannon", "bands_cm": [40_000, 80_000, 120_000]}
            ],
            "contacts": [
                {
                    "id": "BA",
                    "designator": "BA",
                    "class": "blue",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": (heading + 15) % 360,
                    "range_m": 65.0,
                    "heading_deg": (heading + 195) % 360,
                    "facing": "front_left",
                    "moving": true,
                    "health": "damaged"
                },
                {
                    "id": "BB",
                    "designator": "BB",
                    "class": "blue",
                    "unit_type": "tacsim.unit.biped_mk1",
                    "bearing_deg": (heading + 335) % 360,
                    "range_m": 95.0,
                    "heading_deg": (heading + 160) % 360,
                    "facing": "front_right",
                    "moving": true,
                    "health": "intact"
                },
                {
                    "id": "RB",
                    "designator": "RB",
                    "class": "red",
                    "unit_type": "tacsim.unit.scout_mk1",
                    "bearing_deg": (heading + 353) % 360,
                    "range_m": 135.0,
                    "heading_deg": heading,
                    "facing": "rear_right",
                    "moving": true
                }
            ]
        });
        let mut feeds: HashMap<String, Value> = HashMap::new();
        feeds.insert(COCKPIT_FEED.to_string(), cockpit_frame);
        feeds.insert(ARENA_MAP_FEED.to_string(), map.clone());
        let look = Look::default();

        let mut fp_hud = Frame::new(640, 360);
        let _ = CockpitPicture::new().draw(&mut fp_hud, &feeds, &look);
        let _ = std::fs::write(format!("{out}/{name}_fp_hud.png"), png::encode(&fp_hud));

        let mut ov = Frame::new(640, 360);
        let _ = OverheadPicture::new().draw(&mut ov, &feeds, &look);
        let _ = std::fs::write(format!("{out}/{name}_overhead.png"), png::encode(&ov));

        let mut iso = Frame::new(640, 360);
        let _ = Iso25DPicture::new().draw(&mut iso, &feeds, &look);
        let _ = std::fs::write(format!("{out}/{name}_25d.png"), png::encode(&iso));

        println!("{name}: {us} us, ground {ground} cm");
    }
}
