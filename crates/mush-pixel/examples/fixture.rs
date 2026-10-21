//! Render the canonical cockpit scene (`web/src/lib/fixtures/cockpit`) and
//! write `fixture.png` (to look at) and `fixture.kitty` (to `cat` in a
//! terminal that speaks the kitty graphics protocol) into the directory
//! given as the first argument, then print render / encode timings.
//!
//! ```text
//! cargo run -p mush-pixel --example fixture -- out/
//! ```

use std::time::Instant;

use mush_pixel::raycast::{Camera, Ground, Map, Sprite, SpriteKind, render};
use mush_pixel::{Frame, kitty, png};
use serde_json::Value;

const MAP: &str = include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json");
const COCKPIT: &str = include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");

fn expand_rows(rows: &[Value], width: usize) -> Vec<Vec<char>> {
    rows.iter()
        .map(|row| {
            let mut out = Vec::with_capacity(width);
            for run in row.as_array().into_iter().flatten() {
                let glyph = run
                    .get(0)
                    .and_then(Value::as_str)
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let n = run.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                out.extend(std::iter::repeat_n(glyph, n));
            }
            out.resize(width, ' ');
            out
        })
        .collect()
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let (Ok(map_json), Ok(cockpit)) = (
        serde_json::from_str::<Value>(MAP),
        serde_json::from_str::<Value>(COCKPIT),
    ) else {
        eprintln!("the cockpit fixtures did not parse");
        return;
    };

    let width = map_json["width"].as_u64().unwrap_or(0) as usize;
    let cell_cm = map_json["cell_cm"].as_u64().unwrap_or(1000) as u32;
    let legend: Vec<(char, Ground)> = map_json["legend"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let g = pair.get(0)?.as_str()?.chars().next()?;
            let name = pair.get(1)?.as_str()?;
            Some((g, Ground::from_legend_name(name)))
        })
        .collect();
    let rows = expand_rows(map_json["rows"].as_array().map_or(&[][..], Vec::as_slice), width);
    let map = Map::from_glyph_rows(
        &rows,
        &|g| {
            legend
                .iter()
                .find(|(c, _)| *c == g)
                .map_or(Ground::Open, |(_, ground)| *ground)
        },
        cell_cm,
    );

    let cam = Camera {
        x_cm: cockpit["x_cm"].as_i64().unwrap_or(0),
        y_cm: cockpit["y_cm"].as_i64().unwrap_or(0),
        heading_mdeg: cockpit["heading_mdeg"].as_u64().unwrap_or(0) as u32,
    };
    let sprites: Vec<Sprite> = cockpit["contacts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let kind = if c["reads_dead"].as_bool() == Some(true) {
                SpriteKind::Wreck
            } else if c["friendly"].as_bool() == Some(true) {
                SpriteKind::Friendly
            } else if c["class"].as_str().is_some_and(|s| s != "?" && !s.is_empty()) {
                SpriteKind::Hostile
            } else {
                SpriteKind::Unknown
            };
            Sprite::at_bearing(
                &cam,
                c["bearing_deg"].as_f64().unwrap_or(0.0),
                c["range_m"].as_f64().unwrap_or(0.0),
                kind,
            )
        })
        .collect();

    // 640x480: the target frame (the owner's ruling of 2026-10-21).
    let mut frame = Frame::new(640, 480);
    const N: u32 = 200;
    let t0 = Instant::now();
    for _ in 0..N {
        render(&mut frame, &map, &cam, &sprites);
    }
    let render_us = t0.elapsed().as_micros() / u128::from(N);
    let t1 = Instant::now();
    let mut bytes = Vec::new();
    for _ in 0..N {
        bytes = png::encode(&frame);
    }
    let encode_us = t1.elapsed().as_micros() / u128::from(N);
    let stream = kitty::frame_bytes(&bytes, 1, 1, 2, 120, 30);

    if let Err(err) = std::fs::write(format!("{out_dir}/fixture.png"), &bytes) {
        eprintln!("fixture.png: {err}");
    }
    if let Err(err) = std::fs::write(format!("{out_dir}/fixture.kitty"), &stream) {
        eprintln!("fixture.kitty: {err}");
    }
    println!(
        "map {}x{} cells, {} sprites, frame {}x{}, {} colours",
        map.width(),
        map.height(),
        sprites.len(),
        frame.width(),
        frame.height(),
        frame.count_colours(300)
    );
    println!(
        "render {render_us} us, png encode {encode_us} us, png {} bytes, kitty stream {} bytes",
        bytes.len(),
        stream.len()
    );
}
