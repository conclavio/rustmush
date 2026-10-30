//! **The U.6 line-ups** (doc 22 D-22-10, cut U.6): every shipped look drawn
//! the way the two pictures draw a contact, from whatever this build draws it
//! from — its model's baked chains when `models/` holds them, its text sheet
//! nearest-sampled when it does not — written as PNGs into the directory
//! given (`docs/design/images/22-u6/` once the owner's bake lands):
//!
//! - `fp_<range>m.png`: the heightfield's first person, 640x480 over grass,
//!   the six looks side by side at 100 m, 300 m and 1 km, red, facing the
//!   eye's front-right, the ranges a pilot judges the art at (cut U.5);
//!   `fp_lineup.png` the three frames' horizon bands enlarged 3x, stacked;
//! - `iso_1000.png` and `iso_500.png`: the 2.5D at 1.0x and 0.5x zoom, the
//!   six looks on one row (a drone and a heavy stand in the ratio of their
//!   heights; half the zoom draws them half the size).
//!
//! ```text
//! cargo run -p mush-pixel --example unit_lineup -- docs/design/images/22-u6
//! ```

use std::path::PathBuf;

use mush_pixel::heightfield::{self, Terrain};
use mush_pixel::iso25d::Iso25DPicture;
use mush_pixel::look::{self, Facing};
use mush_pixel::raycast::{Camera, Ground, Map, Sprite, SpriteKind, SpritePose};
use mush_pixel::{Frame, png};
use serde_json::json;

/// The ranges rendered, metres.
const RANGES: [f64; 3] = [100.0, 300.0, 1000.0];
/// The looks, left to right, by the unit each draws.
const UNITS: [&str; 6] = [
    "tacsim.unit.drone_mk1",
    "tacsim.unit.scout_mk1",
    "tacsim.unit.line_tank",
    "tacsim.unit.heavy_mk1",
    "tacsim.unit.biped_mk1",
    "unknown",
];

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".to_string()));
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("unit_lineup: {}: {e}", out.display());
        std::process::exit(1);
    }
    let baked: Vec<&str> = mush_pixel::bake::models()
        .iter()
        .map(|e| e.model.as_str())
        .collect();
    println!("baked models: {baked:?} (the rest draw their text sheets)");

    // The first person over level grass, 256 cells square.
    let mut map = Map::new(256, 256, 1000);
    for y in 0..256 {
        for x in 0..256 {
            map.set(x, y, Ground::Grass);
        }
    }
    let terrain = Terrain::new(map);
    let cam = Camera {
        x_cm: 128_000,
        y_cm: 10_000,
        heading_mdeg: 0,
    };
    let mut bands = Vec::new();
    for range in RANGES {
        let sprites: Vec<Sprite> = UNITS
            .iter()
            .enumerate()
            .filter_map(|(i, unit)| {
                let pose =
                    SpritePose::of_look(look::for_unit(unit)?, Facing::FrontRight, false, 0)?;
                // Spread across the view, a sixth of the field apart.
                let bearing = -25.0 + 10.0 * i as f64;
                Some(Sprite {
                    pose: Some(pose),
                    ..Sprite::at_bearing(&cam, bearing, range, SpriteKind::Red)
                })
            })
            .collect();
        let mut frame = Frame::new(640, 480);
        heightfield::render(&mut frame, &terrain, &cam, None, &sprites);
        write(&out, &format!("fp_{range}m.png"), &frame);
        bands.push(band(&frame, 3));
    }
    let (w, h) = (
        bands.first().map_or(0, Frame::width),
        bands.iter().map(Frame::height).sum(),
    );
    let mut lineup = Frame::new(w, h);
    let mut y0 = 0;
    for b in &bands {
        for y in 0..b.height() {
            for x in 0..b.width() {
                lineup.put(x, y0 + y, b.get(x, y));
            }
        }
        y0 += b.height();
    }
    write(&out, "fp_lineup.png", &lineup);

    // The 2.5D: the six looks on one row, east and west of a walker.
    let contacts: Vec<serde_json::Value> = UNITS
        .iter()
        .enumerate()
        .map(|(i, unit)| {
            let (bearing, range) = if i < 3 {
                (270.0, 210.0 - 70.0 * i as f64)
            } else {
                (90.0, 70.0 * (i - 2) as f64)
            };
            let mut c = json!({
                "id": format!("C{i}"),
                "class": "red",
                "bearing_deg": bearing,
                "range_m": range,
                "facing": "front_right",
                "moving": false
            });
            if *unit != look::UNKNOWN {
                c["unit_type"] = json!(unit);
            } else {
                c["class"] = json!("unknown");
            }
            c
        })
        .collect();
    let cockpit = json!({
        "seated": true,
        "team": "blue",
        "unit_type": "tacsim.unit.biped_mk1",
        "x_cm": 128_000,
        "y_cm": 128_000,
        "z_cm": 0,
        "heading_mdeg": 0,
        "speed_cms": 0,
        "contacts": contacts
    });
    for zoom in [1000u16, 500] {
        let mut pic = Iso25DPicture::new();
        let mut ground = Map::new(256, 256, 1000);
        for y in 0..256 {
            for x in 0..256 {
                ground.set(x, y, Ground::Grass);
            }
        }
        pic.set_map(ground);
        pic.set_zoom_permille(zoom);
        let frame = pic.render_frame(&cockpit, 0, 0, 640, 360);
        write(&out, &format!("iso_{zoom}.png"), &frame);
    }
}

/// The frame's band about the horizon (a fifth of its height above, a tenth
/// below), enlarged `k` times, nearest.
fn band(frame: &Frame, k: usize) -> Frame {
    let (w, h) = (frame.width(), frame.height());
    let (y0, y1) = (h / 2 - h / 5, h / 2 + h / 10);
    let mut out = Frame::new(w * k, (y1 - y0) * k);
    for y in 0..(y1 - y0) * k {
        for x in 0..w * k {
            out.put(x, y, frame.get(x / k, y0 + y / k));
        }
    }
    out
}

fn write(dir: &std::path::Path, name: &str, frame: &Frame) {
    let path = dir.join(name);
    match std::fs::write(&path, png::encode(frame)) {
        Ok(()) => println!("{}", path.display()),
        Err(e) => eprintln!("unit_lineup: {}: {e}", path.display()),
    }
}
