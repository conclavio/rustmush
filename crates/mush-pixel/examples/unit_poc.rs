//! **THROWAWAY PoC**: flat sprite sheets vs voxel models for a unit, at 48
//! and 96. Reads the bake's reduced data from the first argument, writes
//! pictures and `results.json` into the second.
//!
//! ```text
//! cargo run -p mush-pixel --release --features poc-units --example unit_poc -- <poc_data> <out>
//! ```
//!
//! Scenes, on a 640x480 frame (the cockpit's):
//! - `field`: 20 units in view from 12 m to 600 m, 80 more behind the eye;
//! - `brawl`: 20 units in view from 10 m to 60 m, the worst case for pixels;
//! - `lineup` and `turntable`: pictures only.
//!
//! Each mode is timed over 200 frames of each timed scene (half the units
//! walking and turning), and every frame is encoded as the palette PNG, as a
//! kitty frame and as sixel, for the bytes a frame costs on the wire.

use std::path::Path;
use std::time::Instant;

use mush_pixel::poc_units::{Mode, PocSheet, PocUnit, PocVoxels, render};
use mush_pixel::raycast::{Camera, Ground, Map, SpriteKind};
use mush_pixel::{Frame, kitty, png, sixel};

const W: usize = 640;
const H: usize = 480;
const FRAMES: usize = 200;

/// A tiny deterministic generator.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn map() -> Map {
    let mut m = Map::new(160, 160, 1000);
    let mut r = Lcg(7);
    for y in 0..160 {
        for x in 0..160 {
            m.set(x, y, Ground::Grass);
        }
    }
    // A few walls and woods past 200 m, so units stand in front of and
    // behind something.
    for _ in 0..40 {
        let (x, y) = (
            (r.next() * 160.0) as usize,
            40 + (r.next() * 110.0) as usize,
        );
        let g = if r.next() < 0.5 {
            Ground::Wall
        } else {
            Ground::Woods
        };
        for d in 0..3 {
            m.set(x + d, y, g);
        }
    }
    m
}

fn cam() -> Camera {
    Camera {
        x_cm: 80_000,
        y_cm: 15_000,
        heading_mdeg: 0,
    }
}

fn at(c: &Camera, bearing_deg: f64, range_m: f64) -> (i64, i64) {
    let b = bearing_deg.to_radians();
    (
        c.x_cm + (range_m * 100.0 * b.sin()).round() as i64,
        c.y_cm + (range_m * 100.0 * b.cos()).round() as i64,
    )
}

fn scene(ranges: &[f64], behind: usize, seed: u64) -> Vec<PocUnit> {
    let c = cam();
    let mut r = Lcg(seed);
    let mut units = Vec::new();
    let n = ranges.len();
    for (i, &range) in ranges.iter().enumerate() {
        let bearing = -26.0 + 52.0 * (i as f64 + 0.5) / n as f64 + (r.next() - 0.5) * 4.0;
        let (x_cm, y_cm) = at(&c, bearing, range);
        units.push(PocUnit {
            x_cm,
            y_cm,
            heading_mdeg: (r.next() * 360_000.0) as u32,
            kind: if i % 2 == 0 {
                SpriteKind::Red
            } else {
                SpriteKind::Blue
            },
            moving: i % 2 == 0,
            step: i,
        });
    }
    for i in 0..behind {
        let (x_cm, y_cm) = at(&c, 100.0 + r.next() * 160.0, 20.0 + r.next() * 500.0);
        units.push(PocUnit {
            x_cm,
            y_cm,
            heading_mdeg: (r.next() * 360_000.0) as u32,
            kind: SpriteKind::Blue,
            moving: i % 2 == 0,
            step: i,
        });
    }
    units
}

/// One frame on: the walkers step every third frame and turn 3 degrees.
fn advance(units: &mut [PocUnit], frame: usize) {
    for u in units.iter_mut().filter(|u| u.moving) {
        if frame.is_multiple_of(3) {
            u.step += 1;
        }
        u.heading_mdeg = (u.heading_mdeg + 3_000) % 360_000;
    }
}

struct Stats {
    mean_ms: f64,
    p95_ms: f64,
    png: usize,
    kitty: usize,
    sixel: usize,
}

fn bench(map: &Map, units0: &[PocUnit], mode: &Mode<'_>) -> Stats {
    let c = cam();
    let mut units = units0.to_vec();
    let mut frame = Frame::new(W, H);
    // Warm up.
    for _ in 0..10 {
        render(&mut frame, map, &c, &units, mode);
    }
    let mut times = Vec::with_capacity(FRAMES);
    let (mut png_b, mut kitty_b, mut sixel_b) = (0, 0, 0);
    for f in 0..FRAMES {
        advance(&mut units, f);
        let t = Instant::now();
        render(&mut frame, map, &c, &units, mode);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        let p = png::encode(&frame);
        kitty_b += kitty::frame_bytes(&p, 1, 0, 0, 80, 30).len();
        png_b += p.len();
        sixel_b += sixel::encode(&frame).len();
    }
    times.sort_by(f64::total_cmp);
    Stats {
        mean_ms: times.iter().sum::<f64>() / FRAMES as f64,
        p95_ms: times[FRAMES * 95 / 100],
        png: png_b / FRAMES,
        kitty: kitty_b / FRAMES,
        sixel: sixel_b / FRAMES,
    }
}

fn save(frame: &Frame, path: &Path) {
    std::fs::write(path, png::encode(frame)).expect("write png");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = Path::new(args.get(1).map_or(".", String::as_str));
    let out = Path::new(args.get(2).map_or(".", String::as_str));
    std::fs::create_dir_all(out).expect("out dir");
    let s48 = PocSheet::load(data, 48).expect("sheet48");
    let s96 = PocSheet::load(data, 96).expect("sheet96");
    let v48 = PocVoxels::load(data, 48, s48.size_cm).expect("vox48");
    let v96 = PocVoxels::load(data, 96, s96.size_cm).expect("vox96");
    let modes: Vec<(&str, Mode<'_>)> = vec![
        ("none", Mode::None),
        ("sprite48", Mode::Sprite(&s48)),
        ("sprite96", Mode::Sprite(&s96)),
        (
            "voxel48_snap8",
            Mode::Voxel {
                model: &v48,
                snap8: true,
            },
        ),
        (
            "voxel96_snap8",
            Mode::Voxel {
                model: &v96,
                snap8: true,
            },
        ),
        (
            "voxel48_free",
            Mode::Voxel {
                model: &v48,
                snap8: false,
            },
        ),
        (
            "voxel96_free",
            Mode::Voxel {
                model: &v96,
                snap8: false,
            },
        ),
    ];
    let map = map();
    let field = scene(
        &[
            12.0, 18.0, 25.0, 35.0, 50.0, 70.0, 90.0, 120.0, 150.0, 180.0, 220.0, 260.0, 300.0,
            350.0, 400.0, 450.0, 500.0, 550.0, 580.0, 600.0,
        ],
        80,
        11,
    );
    let brawl = scene(
        &[
            10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 25.0, 28.0, 30.0, 33.0, 36.0, 39.0, 42.0,
            45.0, 48.0, 51.0, 54.0, 57.0, 60.0,
        ],
        0,
        23,
    );

    let mut json = String::from("{\"frame\":[640,480],\"frames\":200,\"results\":[");
    let mut first = true;
    for (scene_name, units) in [("field", &field), ("brawl", &brawl)] {
        for (name, mode) in &modes {
            let s = bench(&map, units, mode);
            println!(
                "{scene_name:6} {name:14} mean {:7.3} ms  p95 {:7.3} ms  png {:6} B  kitty {:6} B  sixel {:6} B",
                s.mean_ms, s.p95_ms, s.png, s.kitty, s.sixel
            );
            json.push_str(&format!(
                "{}{{\"scene\":\"{scene_name}\",\"mode\":\"{name}\",\"mean_ms\":{:.3},\"p95_ms\":{:.3},\"png\":{},\"kitty\":{},\"sixel\":{}}}",
                if first { "" } else { "," },
                s.mean_ms, s.p95_ms, s.png, s.kitty, s.sixel
            ));
            first = false;
            let mut frame = Frame::new(W, H);
            render(&mut frame, &map, &cam(), units, mode);
            save(&frame, &out.join(format!("{scene_name}_{name}.png")));
        }
    }
    json.push_str("]}");
    std::fs::write(out.join("results.json"), json).expect("results");

    // The lineup: one unit each at 10, 20, 40, 80, 150 and 300 m, three-quarter on.
    let c = cam();
    let lineup: Vec<PocUnit> = [
        (-24.0, 10.0),
        (-12.0, 20.0),
        (-3.0, 40.0),
        (5.0, 80.0),
        (12.0, 150.0),
        (20.0, 300.0),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(b, r))| {
        let (x_cm, y_cm) = at(&c, b, r);
        PocUnit {
            x_cm,
            y_cm,
            heading_mdeg: ((b + 180.0 - 35.0).rem_euclid(360.0) * 1000.0) as u32,
            kind: if i % 2 == 0 {
                SpriteKind::Red
            } else {
                SpriteKind::Blue
            },
            moving: false,
            step: 0,
        }
    })
    .collect();
    for (name, mode) in &modes[1..] {
        let mut frame = Frame::new(W, H);
        render(&mut frame, &map, &c, &lineup, mode);
        save(&frame, &out.join(format!("lineup_{name}.png")));
    }

    // The turntable: one unit at 20 m turning in 15-degree steps, each frame
    // cropped about it, 24 crops in a strip.
    let (tw, th) = (110usize, 120usize);
    for (name, mode) in &modes[1..] {
        let mut strip = Frame::new(tw * 12, th * 2);
        for i in 0..24 {
            let (x_cm, y_cm) = at(&c, 0.0, 20.0);
            let u = PocUnit {
                x_cm,
                y_cm,
                heading_mdeg: (i * 15_000) as u32,
                kind: SpriteKind::Red,
                moving: false,
                step: 0,
            };
            let mut frame = Frame::new(W, H);
            render(&mut frame, &map, &c, &[u], mode);
            let (ox, oy) = (W / 2 - tw / 2, 166);
            for y in 0..th {
                for x in 0..tw {
                    let src = ((oy + y) * W + ox + x) * 3;
                    let px = [frame.rgb()[src], frame.rgb()[src + 1], frame.rgb()[src + 2]];
                    strip.put((i % 12) * tw + x, (i / 12) * th + y, px);
                }
            }
        }
        save(&strip, &out.join(format!("turntable_{name}.png")));
    }
    println!("wrote {}", out.display());
}
