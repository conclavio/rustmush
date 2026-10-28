//! **THROWAWAY PoC, not for main**: a unit drawn as a flat sprite sheet
//! (eight facings) or as a voxel model, into the raycaster's picture against
//! its depth and fog, so the two can be timed and their frames encoded side
//! by side. The data is read from a directory the Unity bake and the reduce
//! script wrote (`sheet{N}.json`, `vox{N}_{pose}.bin`); nothing here ships.
//!
//! Both forms span the same box: `size_cm` across and `size_cm` tall, the
//! unit standing at its centre. A sheet frame is `n` by `n` pixels over it; a
//! voxel model is `n` cells on a side over it, so a 48 sheet and a 48 model
//! resolve the unit alike.

// Throwaway: indexes by the bake's fixed layouts; never built without the
// `poc-units` feature.
#![allow(clippy::indexing_slicing)]

use std::path::Path;

use serde_json::Value;

use crate::Frame;
use crate::look::Material;
use crate::math::Det as _;
use crate::raycast::{
    Camera, Eye, Map, Paint, Shades, SpriteKind, focal, fog_step, render_with, sprite_colour,
};

/// The eight facings, in the bake's order: the unit's heading relative to
/// the line of sight is 180 for `front`, then 45 degrees less each.
pub const FACINGS: [&str; 8] = [
    "front",
    "front_right",
    "right",
    "rear_right",
    "rear",
    "rear_left",
    "left",
    "front_left",
];

/// The poses: still, the four-frame moving cycle, the wreck.
const POSES: [&str; 6] = ["still", "moving0", "moving1", "moving2", "moving3", "wreck"];

/// Voxel classes, as the reduce script writes them.
const PAINT: u8 = 1;
const SECOND: u8 = 2;
const DARK: u8 = 3;
const ACCENT: u8 = 4;
const BRIGHT: u8 = 5;

/// A sheet with all eight facings.
pub struct PocSheet {
    /// Frame side, pixels.
    pub n: usize,
    /// The box's side, centimetres.
    pub size_cm: f32,
    anchor: (usize, usize),
    /// Per facing, per pose (still, moving 0..4, wreck): `n * n` pixels.
    frames: Vec<Vec<Vec<Option<Material>>>>,
}

/// One voxel pose: `n^3` classes, `(y * n + z) * n + x`, x the unit's right,
/// z its front, y up; and the box its solid cells fill.
struct VoxPose {
    cells: Vec<u8>,
    lo: [f32; 3],
    hi: [f32; 3],
}

/// A voxel model, one grid a pose.
pub struct PocVoxels {
    /// Cells a side.
    pub n: usize,
    /// The box's side, centimetres.
    pub size_cm: f32,
    poses: Vec<VoxPose>,
}

/// A unit in the PoC's scenes.
#[derive(Clone, Copy, Debug)]
pub struct PocUnit {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Its heading, millidegrees clockwise from north.
    pub heading_mdeg: u32,
    /// Its team colour.
    pub kind: SpriteKind,
    /// Whether it plays the moving cycle.
    pub moving: bool,
    /// The cycle's step.
    pub step: usize,
}

/// How the units are drawn.
pub enum Mode<'a> {
    /// No units: the terrain alone, the baseline.
    None,
    /// The sheet.
    Sprite(&'a PocSheet),
    /// The voxel model; `snap8` turns it to the nearest 45 degrees of its
    /// aspect, so it says no more about its heading than an 8-facing sheet.
    Voxel {
        /// The model.
        model: &'a PocVoxels,
        /// Snap the heading to the eight facings.
        snap8: bool,
    },
}

fn material(name: &str) -> Option<Material> {
    Some(match name {
        "team-dark" => Material::TeamDark,
        "team" => Material::Team,
        "team-light" => Material::TeamLight,
        "black" => Material::Black,
        "steel" => Material::Steel,
        "pale" => Material::Pale,
        _ => return None,
    })
}

impl PocSheet {
    /// Read `sheet{n}.json` from `dir`.
    pub fn load(dir: &Path, n: usize) -> Result<PocSheet, String> {
        let path = dir.join(format!("sheet{n}.json"));
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let num = |p: &str, i: usize| v[p][i].as_f64().unwrap_or(0.0);
        let palette: Vec<(char, Option<Material>)> = v["palette"]
            .as_object()
            .ok_or("palette")?
            .iter()
            .map(|(k, m)| {
                (
                    k.chars().next().unwrap_or('.'),
                    m.as_str().and_then(material),
                )
            })
            .collect();
        let rows = |r: &Value| -> Vec<Option<Material>> {
            r.as_array()
                .into_iter()
                .flatten()
                .flat_map(|row| row.as_str().unwrap_or("").chars().collect::<Vec<_>>())
                .map(|c| palette.iter().find(|(k, _)| *k == c).and_then(|(_, m)| *m))
                .collect()
        };
        let mut frames = Vec::new();
        for f in FACINGS {
            let fv = &v["facings"][f];
            let mut poses = vec![rows(&fv["still"])];
            for i in 0..4 {
                poses.push(rows(&fv["moving"][i]));
            }
            poses.push(rows(&fv["wreck"]));
            if poses.iter().any(|p| p.len() != n * n) {
                return Err(format!("{f}: a frame is not {n}x{n}"));
            }
            frames.push(poses);
        }
        Ok(PocSheet {
            n,
            size_cm: num("size_cm", 1) as f32,
            anchor: (num("anchor", 0) as usize, num("anchor", 1) as usize),
            frames,
        })
    }
}

impl PocVoxels {
    /// Read `vox{n}_{pose}.bin` from `dir`; `size_cm` is the box's side.
    pub fn load(dir: &Path, n: usize, size_cm: f32) -> Result<PocVoxels, String> {
        let mut poses = Vec::new();
        for p in POSES {
            let path = dir.join(format!("vox{n}_{p}.bin"));
            let cells = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if cells.len() != n * n * n {
                return Err(format!("{}: not {n}^3", path.display()));
            }
            let (mut lo, mut hi) = ([n; 3], [0usize; 3]);
            for (i, &c) in cells.iter().enumerate() {
                if c != 0 {
                    let (x, z, y) = (i % n, (i / n) % n, i / (n * n));
                    for (a, v) in [x, y, z].into_iter().enumerate() {
                        lo[a] = lo[a].min(v);
                        hi[a] = hi[a].max(v + 1);
                    }
                }
            }
            poses.push(VoxPose {
                cells,
                lo: lo.map(|v| v as f32),
                hi: hi.map(|v| v as f32),
            });
        }
        Ok(PocVoxels { n, size_cm, poses })
    }
}

/// Draw the scene with `units` drawn as `mode`.
pub fn render(frame: &mut Frame, map: &Map, cam: &Camera, units: &[PocUnit], mode: &Mode<'_>) {
    render_with(frame, map, cam, &[], &mut |frame, shades, eye, depth| {
        let mut placed: Vec<(f32, f32, &PocUnit)> = units
            .iter()
            .filter_map(|u| {
                let (tx, ty) = view(eye, u.x_cm as f32 / eye.cell, u.y_cm as f32 / eye.cell);
                (ty > 0.05).then_some((tx, ty, u))
            })
            .collect();
        placed.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (tx, ty, u) in placed {
            match mode {
                Mode::None => {}
                Mode::Sprite(sheet) => draw_sprite(frame, shades, eye, depth, u, (tx, ty), sheet),
                Mode::Voxel { model, snap8 } => {
                    draw_voxel(frame, shades, eye, depth, u, (tx, ty), model, *snap8);
                }
            }
        }
    });
}

/// A point's place in the camera: across (camera-plane units) and depth.
fn view(e: &Eye, wx: f32, wy: f32) -> (f32, f32) {
    let invdet = 1.0 / (e.planex * e.diry - e.dirx * e.planey);
    let (relx, rely) = (wx - e.posx, wy - e.posy);
    (
        invdet * (e.diry * relx - e.dirx * rely),
        invdet * (-e.planey * relx + e.planex * rely),
    )
}

/// The unit's heading relative to the line of sight, degrees `0..360`: 180
/// is facing the eye.
fn aspect_deg(e: &Eye, u: &PocUnit) -> f32 {
    let (relx, rely) = (
        u.x_cm as f32 / e.cell - e.posx,
        u.y_cm as f32 / e.cell - e.posy,
    );
    let bearing = libm::atan2f(relx, rely).to_degrees();
    (u.heading_mdeg as f32 / 1000.0 - bearing).rem_euclid(360.0)
}

fn facing_index(aspect: f32) -> usize {
    ((((180.0 - aspect) / 45.0).round() as i32).rem_euclid(8)) as usize
}

fn pose_index(u: &PocUnit) -> usize {
    match u.kind {
        SpriteKind::Wreck => 5,
        _ if u.moving => 1 + u.step % 4,
        _ => 0,
    }
}

/// The light, from the eye's upper left and toward it, as the bake lit the
/// sheet (`reduce.py`'s `L`), in world axes (east, north, up).
fn light(e: &Eye) -> [f32; 3] {
    let (rx, ry) = (e.diry, -e.dirx);
    let l = [-0.4 * rx - 0.45 * e.dirx, -0.4 * ry - 0.45 * e.diry, 0.8];
    let len = libm::sqrtf(l[0] * l[0] + l[1] * l[1] + l[2] * l[2]);
    l.map(|v| v / len)
}

/// A class lit at `lit` (0..1) as one of the six materials, as `reduce.py`.
fn class_material(class: u8, lit: f32) -> Option<Material> {
    Some(match class {
        PAINT if lit > 0.72 => Material::TeamLight,
        PAINT if lit > 0.35 => Material::Team,
        PAINT => Material::TeamDark,
        SECOND if lit > 0.45 => Material::Steel,
        SECOND => Material::TeamDark,
        DARK => Material::Black,
        ACCENT => Material::TeamLight,
        BRIGHT => Material::Pale,
        _ => return None,
    })
}

/// The ground shadow under a unit, as the raycaster's `draw_sheet`.
fn shadow(
    frame: &mut Frame,
    shades: &Shades,
    depth_at: &dyn Fn(usize, usize) -> f32,
    (screenx, bottom, width, depth): (f32, f32, f32, f32),
    step: usize,
) {
    let (w, h) = (frame.width(), frame.height());
    let rx = width * 0.3;
    let ry = (rx * 0.16).max(1.0);
    let px = shades.paint(Paint::Shadow, step);
    let (x0, x1) = (
        (screenx - rx).floor().max(0.0) as usize,
        ((screenx + rx).ceil().max(0.0) as usize).min(w),
    );
    let (y0, y1) = (
        (bottom - ry).floor().max(0.0) as usize,
        ((bottom + ry).ceil().max(0.0) as usize).min(h),
    );
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (
                (x as f32 + 0.5 - screenx) / rx,
                (y as f32 + 0.5 - bottom) / ry,
            );
            if dx * dx + dy * dy <= 1.0 && depth_at(x, y) >= depth * 0.85 {
                frame.put(x, y, px);
            }
        }
    }
}

fn draw_sprite(
    frame: &mut Frame,
    shades: &Shades,
    e: &Eye,
    depth_at: &dyn Fn(usize, usize) -> f32,
    u: &PocUnit,
    (tx, ty): (f32, f32),
    sheet: &PocSheet,
) {
    let (w, h) = (frame.width(), frame.height());
    let hf = focal(w);
    let screenx = (w as f32 / 2.0) * (1.0 + tx / ty);
    let bottom = (h / 2) as f32 + e.eye * hf / ty;
    let size = sheet.size_cm / e.cell * hf / ty;
    if size < 1.0 || screenx + size < 0.0 || screenx - size >= w as f32 {
        return;
    }
    let step = fog_step(ty);
    shadow(frame, shades, depth_at, (screenx, bottom, size, ty), step);
    let pixels = &sheet.frames[facing_index(aspect_deg(e, u))][pose_index(u)];
    let n = sheet.n;
    let nf = n as f32;
    let (ax, ay) = (sheet.anchor.0 as f32, sheet.anchor.1 as f32);
    let left = screenx - (ax + 0.5) / nf * size;
    let top = bottom - (ay + 1.0) / nf * size;
    let x0 = left.floor().max(0.0) as usize;
    let x1 = ((left + size).ceil().max(0.0) as usize).min(w);
    let y0 = top.floor().max(0.0) as usize;
    let y1 = ((top + size).ceil().max(0.0) as usize).min(h);
    for y in y0..y1 {
        let sy = (((y as f32 + 0.5 - top) / size * nf).floor().max(0.0) as usize).min(n - 1);
        for x in x0..x1 {
            if ty >= depth_at(x, y) {
                continue;
            }
            let sx = (((x as f32 + 0.5 - left) / size * nf).floor().max(0.0) as usize).min(n - 1);
            if let Some(Some(m)) = pixels.get(sy * n + sx) {
                frame.put(x, y, sprite_colour(shades, u.kind, *m, step));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_voxel(
    frame: &mut Frame,
    shades: &Shades,
    e: &Eye,
    depth_at: &dyn Fn(usize, usize) -> f32,
    u: &PocUnit,
    (_tx, ty): (f32, f32),
    model: &PocVoxels,
    snap8: bool,
) {
    let (w, h) = (frame.width(), frame.height());
    let hf = focal(w);
    let hz = (h / 2) as f32;
    let span = model.size_cm / e.cell;
    let (ux, uy) = (u.x_cm as f32 / e.cell, u.y_cm as f32 / e.cell);
    let pose = &model.poses[pose_index(u)];
    let n = model.n;
    let k = n as f32 / span;

    // The heading drawn: the true one, or snapped to the eight facings.
    let heading = if snap8 {
        let aspect = aspect_deg(e, u);
        let snapped = (aspect / 45.0).round() * 45.0;
        u.heading_mdeg as f32 / 1000.0 - aspect + snapped
    } else {
        u.heading_mdeg as f32 / 1000.0
    };
    let (sinh, cosh) = heading.to_radians().det_sin_cos();
    let (fx, fy) = (sinh, cosh); // forward
    let (rx, ry) = (cosh, -sinh); // right

    // Screen bounds: the pose's solid box's corners, projected.
    let (mut sx0, mut sx1, mut sy0, mut sy1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for corner in 0..8 {
        let gx = if corner & 1 == 0 {
            pose.lo[0]
        } else {
            pose.hi[0]
        };
        let gy = if corner & 2 == 0 {
            pose.lo[1]
        } else {
            pose.hi[1]
        };
        let gz = if corner & 4 == 0 {
            pose.lo[2]
        } else {
            pose.hi[2]
        };
        let (lx, lz) = ((gx - n as f32 / 2.0) / k, (gz - n as f32 / 2.0) / k);
        let (wx, wy) = (ux + lx * rx + lz * fx, uy + lx * ry + lz * fy);
        let (ctx, cty) = view(e, wx, wy);
        if cty <= 0.05 {
            return;
        }
        let sx = (w as f32 / 2.0) * (1.0 + ctx / cty);
        let sy = hz - (gy / k - e.eye) * hf / cty;
        sx0 = sx0.min(sx);
        sx1 = sx1.max(sx);
        sy0 = sy0.min(sy);
        sy1 = sy1.max(sy);
    }
    if sx1 < 0.0 || sx0 >= w as f32 || sx1 - sx0 < 1.0 {
        return;
    }
    let step = fog_step(ty);
    shadow(
        frame,
        shades,
        depth_at,
        (
            ux_screen(e, w, ux, uy),
            hz + e.eye * hf / ty,
            span * hf / ty,
            ty,
        ),
        step,
    );

    // The six faces' light: -x, +x, -y, +y, -z, +z in the unit's axes.
    let l = light(e);
    let lit_of = |nx: f32, ny: f32, nz: f32| (nx * l[0] + ny * l[1] + nz * l[2]).max(0.0);
    let faces = [
        lit_of(-rx, -ry, 0.0),
        lit_of(rx, ry, 0.0),
        lit_of(0.0, 0.0, -1.0),
        lit_of(0.0, 0.0, 1.0),
        lit_of(-fx, -fy, 0.0),
        lit_of(fx, fy, 0.0),
    ];
    let paints: Vec<[Option<crate::frame::Rgb>; 6]> = (0..=BRIGHT)
        .map(|c| {
            faces.map(|lit| class_material(c, lit).map(|m| sprite_colour(shades, u.kind, m, step)))
        })
        .collect();

    let ni = n as i32;
    let (x0, x1) = (
        sx0.floor().max(0.0) as usize,
        (sx1.ceil().max(0.0) as usize).min(w),
    );
    let (y0, y1) = (
        sy0.floor().max(0.0) as usize,
        (sy1.ceil().max(0.0) as usize).min(h),
    );
    // The ray's origin in the grid (t = 0, the eye), and its rate per unit
    // of t along the x and z axes for a column; y's rate is per row.
    let (ox, oy) = (e.posx - ux, e.posy - uy);
    let o = [
        (ox * rx + oy * ry) * k + n as f32 / 2.0,
        e.eye * k,
        (ox * fx + oy * fy) * k + n as f32 / 2.0,
    ];
    for x in x0..x1 {
        let camx = 2.0 * x as f32 / w as f32 - 1.0;
        let rdx = e.dirx + e.planex * camx;
        let rdy = e.diry + e.planey * camx;
        let dx = (rdx * rx + rdy * ry) * k;
        let dz = (rdx * fx + rdy * fy) * k;
        for y in y0..y1 {
            let dy = -(y as f32 + 0.5 - hz) / hf * k;
            let d = [dx, dy, dz];
            // Slab test against the pose's solid box.
            let (mut t0, mut t1, mut axis0) = (0.05f32, f32::MAX, 0usize);
            let mut miss = false;
            for a in 0..3 {
                if d[a].abs() < 1e-9 {
                    if o[a] < pose.lo[a] || o[a] >= pose.hi[a] {
                        miss = true;
                    }
                    continue;
                }
                let (mut ta, mut tb) = ((pose.lo[a] - o[a]) / d[a], (pose.hi[a] - o[a]) / d[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                if ta > t0 {
                    t0 = ta;
                    axis0 = a;
                }
                t1 = t1.min(tb);
            }
            if miss || t0 >= t1 {
                continue;
            }
            let near = depth_at(x, y);
            if t0 >= near {
                continue;
            }
            // Amanatides-Woo through the grid from t0.
            let p = [o[0] + d[0] * t0, o[1] + d[1] * t0, o[2] + d[2] * t0];
            let mut c = [0i32; 3];
            let mut st = [0i32; 3];
            let mut tmax = [f32::MAX; 3];
            let mut tdelta = [f32::MAX; 3];
            for a in 0..3 {
                c[a] = (p[a].floor() as i32).clamp(0, ni - 1);
                if d[a] > 0.0 {
                    st[a] = 1;
                    tdelta[a] = 1.0 / d[a];
                    tmax[a] = t0 + ((c[a] + 1) as f32 - p[a]) / d[a];
                } else if d[a] < 0.0 {
                    st[a] = -1;
                    tdelta[a] = -1.0 / d[a];
                    tmax[a] = t0 + (c[a] as f32 - p[a]) / d[a];
                }
            }
            let mut face = axis0;
            let mut t = t0;
            loop {
                let class = pose.cells[((c[1] * ni + c[2]) * ni + c[0]) as usize];
                if class != 0 {
                    if t < near {
                        let side = face * 2 + usize::from(st[face] < 0);
                        if let Some(Some(px)) = paints.get(class as usize).map(|p| p[side]) {
                            frame.put(x, y, px);
                        }
                    }
                    break;
                }
                let a = if tmax[0] < tmax[1] {
                    if tmax[0] < tmax[2] { 0 } else { 2 }
                } else if tmax[1] < tmax[2] {
                    1
                } else {
                    2
                };
                t = tmax[a];
                if t >= t1 || t >= near {
                    break;
                }
                c[a] += st[a];
                if c[a] < 0 || c[a] >= ni {
                    break;
                }
                tmax[a] += tdelta[a];
                face = a;
            }
        }
    }
}

/// The screen column the unit's centre stands at.
fn ux_screen(e: &Eye, w: usize, ux: f32, uy: f32) -> f32 {
    let (tx, ty) = view(e, ux, uy);
    (w as f32 / 2.0) * (1.0 + tx / ty)
}
