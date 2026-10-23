//! Render the canonical cockpit scene (`web/src/lib/fixtures/cockpit`) and
//! write `fixture.png` (to look at), `fixture.kitty` (to `cat` in a terminal
//! that speaks the kitty graphics protocol) and `fixture.sixel` (to `cat` or
//! `type` in one that draws sixel: Windows Terminal, foot, xterm, mintty)
//! into the directory given as the first argument, then print render and
//! encode timings. The scene is the text cockpit's (2026-10-22): the flight
//! HUD over a 640x256 frame, the shape of the top two-thirds of a 160x50
//! terminal; `fixture_instruments.png` is the 640x480 frame with every
//! instrument inside it, as the 2026-10-21 cockpit drew it.
//!
//! ```text
//! cargo run -p mush-pixel --example fixture -- out/
//! ```

use std::collections::HashMap;
use std::time::Instant;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture, Overlay, map_from_feed};
use mush_pixel::heightfield;
use mush_pixel::picture::{Look, Picture};
use mush_pixel::raycast::{Camera, Sprite, SpriteKind};
use mush_pixel::{Frame, kitty, png, sixel};

/// The proving ground's shipped relief, decoded as the heightfield test does
/// (doc 21 cut G.7a).
#[path = "../tests/support/atlas.rs"]
mod atlas;
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

    // 640 wide (the owner's ruling of 2026-10-21), shaped to the text
    // cockpit's picture box: 160 by 32 cells of 10x20 pixels.
    let mut frame = Frame::new(640, 256);
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
    let stream = kitty::frame_bytes(&bytes, 1, 1, 2, 160, 32);
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
    let mut full = Frame::new(640, 480);
    CockpitPicture::with(Overlay::Instruments).draw(&mut full, &feeds, &Look::default());
    if let Err(err) = std::fs::write(
        format!("{out_dir}/fixture_instruments.png"),
        png::encode(&full),
    ) {
        eprintln!("fixture_instruments.png: {err}");
    }

    // G.7a: the heightfield producer over the proving ground's real relief,
    // from the middle of the ground looking north, a tank 300 m out.
    let ground = atlas::proving_ground();
    let cam = Camera {
        x_cm: 128_000,
        y_cm: 96_000,
        heading_mdeg: 20_000,
    };
    let tank = Sprite {
        x_cm: 138_000,
        y_cm: 124_000,
        kind: SpriteKind::Red,
        pose: None,
    };
    let mut relief = Frame::new(640, 480);
    let t3 = Instant::now();
    for _ in 0..N {
        heightfield::render(&mut relief, &ground, &cam, None, &[tank]);
    }
    let relief_us = t3.elapsed().as_micros() / u128::from(N);
    let relief_png = png::encode(&relief);
    if let Err(err) = std::fs::write(format!("{out_dir}/relief.png"), &relief_png) {
        eprintln!("relief.png: {err}");
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
        "render + HUD {render_us} us, png encode {encode_us} us, png {} bytes, kitty stream {} bytes",
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
    println!(
        "heightfield 640x480 over the proving ground: render {relief_us} us, {} colours, png {} bytes",
        relief.count_colours(300),
        relief_png.len()
    );
    // The baked colour map (Voxel Space's other half): the same pose, once
    // baked.
    let mut baked_ground = ground.clone();
    let t4 = Instant::now();
    baked_ground.bake(2350);
    let bake_ms = t4.elapsed().as_millis();
    let mut baked = Frame::new(640, 480);
    let t5 = Instant::now();
    for _ in 0..N {
        heightfield::render(&mut baked, &baked_ground, &cam, None, &[tank]);
    }
    let baked_us = t5.elapsed().as_micros() / u128::from(N);
    let baked_png = png::encode(&baked);
    if let Err(err) = std::fs::write(format!("{out_dir}/relief_baked.png"), &baked_png) {
        eprintln!("relief_baked.png: {err}");
    }
    println!(
        "heightfield baked: bake {bake_ms} ms once, render {baked_us} us, {} colours, png {} bytes",
        baked.count_colours(300),
        baked_png.len()
    );
    // A crater 240 m across and 30 m deep on open regolith, the kind of
    // ground a Ceres site would be: unbaked and baked, from its floor
    // looking north-east across to the far wall.
    let mut crater_map = mush_pixel::raycast::Map::new(128, 128, 1000);
    for y in 0..128 {
        for x in 0..128 {
            crater_map.set(x, y, mush_pixel::raycast::Ground::Open);
        }
    }
    let mut crater = heightfield::Terrain::new(crater_map);
    for y in 0..128 {
        for x in 0..128 {
            let d = ((x as f32 - 64.0).powi(2) + (y as f32 - 72.0).powi(2)).sqrt() / 12.0;
            let cm = if d < 1.0 {
                -3000.0 + 3800.0 * d * d
            } else {
                800.0 * (-(d - 1.0) * 1.5).exp()
            };
            crater.set_elev_cm(x, y, cm as i32);
        }
    }
    let rim = Camera {
        x_cm: 58_000,
        y_cm: 66_000,
        heading_mdeg: 35_000,
    };
    for (name, bake) in [("crater", false), ("crater_baked", true)] {
        let mut t = crater.clone();
        if bake {
            t.bake(9);
        }
        let mut f = Frame::new(640, 480);
        heightfield::render(&mut f, &t, &rim, None, &[]);
        let bytes = png::encode(&f);
        if let Err(err) = std::fs::write(format!("{out_dir}/{name}.png"), &bytes) {
            eprintln!("{name}.png: {err}");
        }
        println!(
            "{name}: {} colours, png {} bytes",
            f.count_colours(300),
            bytes.len()
        );
    }
    // Doc 22 cut U.3: the sprite line-up — the shipped sheets across both
    // teams, their facings and a wreck, standing on grass in the text
    // cockpit's frame shape — written to `sprites.png`.
    {
        use mush_pixel::look::{self, Facing};
        use mush_pixel::raycast::{Ground, Map, SpritePose, render};
        let mut ground = Map::new(96, 96, 1000);
        for y in 0..96 {
            for x in 0..96 {
                ground.set(x, y, Ground::Grass);
            }
        }
        let cam = Camera {
            x_cm: 48_000,
            y_cm: 20_000,
            heading_mdeg: 0,
        };
        let tank = look::for_unit("tacsim.unit.line_tank").and_then(|l| l.sprites.as_ref());
        let biped = look::for_unit("tacsim.unit.biped_mk1").and_then(|l| l.sprites.as_ref());
        let unknown = look::unknown().and_then(|l| l.sprites.as_ref());
        let at = |bearing: f64,
                  range: f64,
                  kind: SpriteKind,
                  sheet: Option<&'static look::Sheet>,
                  facing: Facing,
                  moving: bool| Sprite {
            pose: sheet.map(|sheet| SpritePose {
                sheet,
                facing,
                moving,
                step: 1,
            }),
            ..Sprite::at_bearing(&cam, bearing, range, kind)
        };
        let lineup = [
            at(-24.0, 70.0, SpriteKind::Red, tank, Facing::Right, true),
            at(-12.0, 90.0, SpriteKind::Blue, tank, Facing::Front, false),
            at(-3.0, 60.0, SpriteKind::Blue, biped, Facing::Front, true),
            at(6.0, 80.0, SpriteKind::Red, biped, Facing::Left, true),
            at(
                15.0,
                120.0,
                SpriteKind::Unknown,
                unknown,
                Facing::Front,
                false,
            ),
            at(24.0, 75.0, SpriteKind::Wreck, tank, Facing::Right, false),
        ];
        let mut f = Frame::new(640, 256);
        render(&mut f, &ground, &cam, &lineup);
        if let Err(err) = std::fs::write(format!("{out_dir}/sprites.png"), png::encode(&f)) {
            eprintln!("sprites.png: {err}");
        }
        println!("sprites: {} colours", f.count_colours(300));
    }
    // Doc 21 cut G.5b: the orbital view, the chart fixture drawn as a scene,
    // in the TUI's picture shape and at 4:3 — `chart.png` and `chart43.png`.
    {
        use mush_pixel::chart::{ChartPicture, SKY_CHART_FEED};
        let sky: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/chart/sky_chart.json"
        ))
        .unwrap_or_default();
        let feeds: HashMap<String, Value> =
            [(SKY_CHART_FEED.to_string(), sky)].into_iter().collect();
        for (name, w, h) in [("chart", 640, 256), ("chart43", 640, 480)] {
            let mut f = Frame::new(w, h);
            let t = Instant::now();
            ChartPicture::new().draw(&mut f, &feeds, &Look::default());
            let us = t.elapsed().as_micros();
            let bytes = png::encode(&f);
            if let Err(err) = std::fs::write(format!("{out_dir}/{name}.png"), &bytes) {
                eprintln!("{name}.png: {err}");
            }
            println!(
                "{name}: {w}x{h} drawn in {us} us, {} colours, png {} bytes",
                f.count_colours(300),
                bytes.len()
            );
        }
    }
    // Sixel filling a box (G.2b): Windows Terminal's 120x40 at its virtual
    // 10x20 cell, and a high-DPI w11term pane of 240x48 at 15x28.
    for (ow, oh) in [(1013, 760), (1717, 1288)] {
        let t = Instant::now();
        let mut scaled = Vec::new();
        for _ in 0..N / 4 {
            scaled = sixel::encode_scaled(&frame, ow, oh);
        }
        let us = t.elapsed().as_micros() / u128::from(N / 4);
        println!(
            "sixel shown {ow}x{oh}: encode {us} us, {} bytes, {} after zlib",
            scaled.len(),
            squeeze(&scaled)
        );
    }
}
