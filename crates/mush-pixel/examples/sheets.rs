//! Render every shipped sprite sheet (doc 22 §3.2) through the raycaster, the
//! way the cockpit picture draws a contact, and write one PNG a sheet a range
//! into the directory given as the first argument: `<look>_<range>m.png`, a
//! 640x480 frame over grass with the sheet's poses side by side — front
//! still, the front moving cycle, right still, right moving, rear still, the
//! left (the right mirrored) and the wreck — at 100 m, 300 m and 1 km, the
//! ranges a pilot judges the art at (cut U.5). The red team for the tank, the
//! blue for the biped, unknown grey for the `unknown` look.
//!
//! Then the web parity golden's six cases (`tests/web_golden.rs`: the
//! cockpit fixture, flat and with relief, three looks each) as
//! `golden_<map>_<look>.png`, so a re-blessed golden can be looked at before
//! it is accepted.
//!
//! ```text
//! cargo run -p mush-pixel --example sheets -- out/
//! ```

use std::collections::HashMap;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::look::{self, Facing, Look};
use mush_pixel::picture::{Look as View, Picture};
use mush_pixel::raycast::{Camera, Ground, Map, Sprite, SpriteKind, SpritePose, render};
use mush_pixel::{Frame, png};
use serde_json::Value;

/// The ranges rendered, metres.
const RANGES: [f64; 3] = [100.0, 300.0, 1000.0];

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let mut ground = Map::new(256, 256, 1000);
    for y in 0..256 {
        for x in 0..256 {
            ground.set(x, y, Ground::Grass);
        }
    }
    let cam = Camera {
        x_cm: 128_000,
        y_cm: 20_000,
        heading_mdeg: 0,
    };
    for look in look::shipped() {
        // The text sheet alone (its own, else its model's), never the bake:
        // this is the sheets' line-up.
        let Some(base) = SpritePose::of_look(look, Facing::Front, false, 0) else {
            continue;
        };
        let sheet = base.sheet;
        let kind = team_of(look);
        let name = look
            .units
            .first()
            .map_or("look", |u| u.rsplit('.').next().unwrap_or(u));
        for range in RANGES {
            // The poses across the view, spread by bearing so none overlaps
            // another at 100 m (a tank is 34 px wide there).
            let mut poses: Vec<(Facing, bool, usize, SpriteKind)> = Vec::new();
            poses.push((Facing::Front, false, 0, kind));
            let cycle = sheet.facings[&Facing::Front].moving.len();
            for step in 0..cycle {
                poses.push((Facing::Front, true, step, kind));
            }
            poses.push((Facing::Right, false, 0, kind));
            poses.push((Facing::Right, true, 1, kind));
            poses.push((Facing::Rear, false, 0, kind));
            poses.push((Facing::Left, false, 0, kind));
            poses.push((Facing::Front, false, 0, SpriteKind::Wreck));
            let n = poses.len() as f64;
            let spread = 56.0;
            let sprites: Vec<Sprite> = poses
                .iter()
                .enumerate()
                .map(|(i, &(facing, moving, step, kind))| Sprite {
                    pose: Some(SpritePose {
                        facing,
                        moving,
                        step,
                        model: None,
                        ..base
                    }),
                    ..Sprite::at_bearing(
                        &cam,
                        -spread / 2.0 + spread * (i as f64 + 0.5) / n,
                        range,
                        kind,
                    )
                })
                .collect();
            let mut frame = Frame::new(640, 480);
            render(&mut frame, &ground, &cam, &sprites);
            write(&format!("{out_dir}/{name}_{}m.png", range as i64), &frame);
        }
    }
    golden_cases(&out_dir);
}

/// Write `frame` as a PNG at `path`, saying so.
fn write(path: &str, frame: &Frame) {
    if let Err(err) = std::fs::write(path, png::encode(frame)) {
        eprintln!("{path}: {err}");
    } else {
        println!("{path}");
    }
}

/// The web golden's cases, drawn as the golden draws them: the fixture
/// cockpit over the flat yard (its heights dropped) and over the relief
/// fixture the bless wrote, straight ahead, swung right, swung left.
fn golden_cases(out_dir: &str) {
    let Ok(cockpit) = serde_json::from_str::<Value>(include_str!(
        "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
    )) else {
        eprintln!("the cockpit fixture did not parse");
        return;
    };
    let Ok(mut flat) = serde_json::from_str::<Value>(include_str!(
        "../../../web/src/lib/fixtures/cockpit/arena_map.json"
    )) else {
        eprintln!("the arena fixture did not parse");
        return;
    };
    if let Some(map) = flat.as_object_mut() {
        map.remove("elev");
    }
    let relief_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/src/lib/fixtures/pixel/arena_map_relief.json"
    );
    let relief = std::fs::read_to_string(relief_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let maps = [("flat", Some(flat)), ("relief", relief)];
    for (map_name, map) in maps {
        let Some(map) = map else {
            eprintln!("no relief fixture at {relief_path}: PIXEL_BLESS=1 writes it");
            continue;
        };
        let feeds: HashMap<String, Value> = [
            (COCKPIT_FEED.to_string(), cockpit.clone()),
            (ARENA_MAP_FEED.to_string(), map),
        ]
        .into_iter()
        .collect();
        for offset_mdeg in [0, 45_000, -90_000] {
            let mut frame = Frame::new(640, 480);
            let view = View {
                offset_mdeg,
                ..View::default()
            };
            if !CockpitPicture::new().draw(&mut frame, &feeds, &view) {
                eprintln!("the fixture did not draw ({map_name}, {offset_mdeg})");
                continue;
            }
            write(
                &format!("{out_dir}/golden_{map_name}_{offset_mdeg}.png"),
                &frame,
            );
        }
    }
}

/// The relation a look is shown as here (doc 32 cut CF.0): the tank hostile,
/// the biped friend, the `unknown` look grey.
fn team_of(look: &Look) -> SpriteKind {
    if look.units.iter().any(|u| u == look::UNKNOWN) {
        SpriteKind::Unknown
    } else if look.units.iter().any(|u| u.contains("biped")) {
        SpriteKind::Friend
    } else {
        SpriteKind::Hostile
    }
}
