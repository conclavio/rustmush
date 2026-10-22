//! Render the canonical cockpit scene (`web/src/lib/fixtures/cockpit`) and
//! write `fixture.png` (to look at), `fixture.kitty` (to `cat` in a terminal
//! that speaks the kitty graphics protocol) and `fixture.sixel` (to `cat` or
//! `type` in one that draws sixel: Windows Terminal, foot, xterm, mintty)
//! into the directory given as the first argument, then print render and
//! encode timings.
//!
//! ```text
//! cargo run -p mush-pixel --example fixture -- out/
//! ```

use std::collections::HashMap;
use std::time::Instant;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture, map_from_feed};
use mush_pixel::picture::{Look, Picture};
use mush_pixel::{Frame, kitty, png, sixel};
use serde_json::Value;

const MAP: &str = include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json");
const COCKPIT: &str = include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let (Ok(map_json), Ok(cockpit)) = (
        serde_json::from_str::<Value>(MAP),
        serde_json::from_str::<Value>(COCKPIT),
    ) else {
        eprintln!("the cockpit fixtures did not parse");
        return;
    };
    let map = map_from_feed(&map_json);
    let contacts = cockpit["contacts"].as_array().map_or(0, Vec::len);
    // The feeds a renderer would hold, by name: exactly what the TUI hands
    // the same picture.
    let feeds: HashMap<String, Value> = [
        (COCKPIT_FEED.to_string(), cockpit),
        (ARENA_MAP_FEED.to_string(), map_json),
    ]
    .into_iter()
    .collect();
    let mut picture = CockpitPicture::new();

    // 640x480: the target frame (the owner's ruling of 2026-10-21).
    let mut frame = Frame::new(640, 480);
    const N: u32 = 200;
    let t0 = Instant::now();
    for _ in 0..N {
        picture.draw(&mut frame, &feeds, &Look::default());
    }
    let render_us = t0.elapsed().as_micros() / u128::from(N);
    let t1 = Instant::now();
    let mut bytes = Vec::new();
    for _ in 0..N {
        bytes = png::encode(&frame);
    }
    let encode_us = t1.elapsed().as_micros() / u128::from(N);
    let stream = kitty::frame_bytes(&bytes, 1, 1, 2, 120, 30);
    let t2 = Instant::now();
    let mut six = Vec::new();
    for _ in 0..N {
        six = sixel::encode(&frame);
    }
    let sixel_us = t2.elapsed().as_micros() / u128::from(N);
    // Home the cursor first so the picture lands at the top left of the
    // screen it is typed into.
    let six_stream = sixel::frame_bytes(&six, 1, 1);

    if let Err(err) = std::fs::write(format!("{out_dir}/fixture.png"), &bytes) {
        eprintln!("fixture.png: {err}");
    }
    if let Err(err) = std::fs::write(format!("{out_dir}/fixture.kitty"), &stream) {
        eprintln!("fixture.kitty: {err}");
    }
    if let Err(err) = std::fs::write(format!("{out_dir}/fixture.sixel"), &six_stream) {
        eprintln!("fixture.sixel: {err}");
    }
    println!(
        "map {}x{} cells, {contacts} contacts, frame {}x{}, {} colours",
        map.width(),
        map.height(),
        frame.width(),
        frame.height(),
        frame.count_colours(300)
    );
    println!(
        "render + instruments {render_us} us, png encode {encode_us} us, png {} bytes, kitty stream {} bytes",
        bytes.len(),
        stream.len()
    );
    // What SSH's own zlib makes of each, at a level close to OpenSSH's.
    let squeeze = |b: &[u8]| {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        let _ = z.write_all(b);
        z.finish().map_or(0, |v| v.len())
    };
    println!(
        "sixel encode {sixel_us} us, sixel stream {} bytes; after zlib: kitty {} bytes, sixel {} bytes",
        six_stream.len(),
        squeeze(&stream),
        squeeze(&six_stream)
    );
}
