//! **The heightfield producer** (doc 21 §8, D-21-15; cut G.7a): the
//! first-person picture of ground with relief, the second bucket-A producer
//! beside [`raycast`](crate::raycast).
//!
//! Voxel space, as Comanche drew it: the ground is a height per cell and a
//! colour per cell, and the picture is cast **front to back** — for each
//! distance from the eye, a line of samples across the view, each sample
//! projected to a screen row and drawn as a vertical run from there down to
//! the highest row the column has drawn so far, so nearer ground hides
//! farther ground without a depth sort. The march starts a quarter cell out
//! and its steps grow with distance, to the fog's limit.
//!
//! What it draws on top of that:
//!
//! - **Smooth relief**: a sample's height is interpolated between the four
//!   nearest cell centres, so a hill is a slope and not a staircase.
//! - **Blocks**: a wall stands 3 m above its cell, light woods 6 m and heavy
//!   woods 8 m — the sim's heights ([`Ground::height_cm`]), as is the eye's
//!   1.5 m — so the arena's buildings and tree lines read as solid; they are
//!   drawn as raised cells, flat-topped.
//! - **Light**: each drawn sample is lit or shaded by its slope toward a sun
//!   in the north-west — two levels, no more, so a frame stays inside the
//!   palette PNG's 256 colours with the fog and the instruments.
//! - **Sprites**: the units stand on the ground at their own cell's height,
//!   and a per-pixel depth (the distance each pixel was drawn at) lets a
//!   ridge hide half a tank — but never all of a contact the sim reports,
//!   which is then bracketed where it stands (the raycaster's rule). Where
//!   a sprite lands is one function (`project`, the crate's own), the same
//!   the cockpit's HUD boxes the lock with, so the box lands on the tank it
//!   marks (doc
//!   21a `X-03`: the box was placed by the raycaster's flat-ground
//!   projection, well off a target down a slope).
//!
//! The colours are the raycaster's closed set (`Shades`), so the two
//! producers draw the same ground the same colour.
//!
//! **The baked colour map** (Voxel Space's other half, the owner's ask of
//! 2026-10-22). [`Terrain::bake`] computes, once a map, a colour for every
//! [`TEXELS`]×[`TEXELS`] texel of every cell (2.5 m on the arena's 10 m
//! cells) from the heights and the terrain classes alone — what the viewer's
//! own map feed carries, never anything the feed does not (D-21-1): smooth
//! light from the interpolated surface's normal against a north-west sun,
//! darker on any steep slope whichever way it faces, darker in hollows, and a
//! soft seeded mottle in blobs of two to five cells, quantized to a few light
//! levels so a frame stays inside the palette and compresses like a flat
//! one. A baked terrain is drawn from the map (one lookup a sample); an
//! unbaked one with the per-cell light above.

use crate::Frame;
use crate::frame::Rgb;
use crate::math::Det as _;
use crate::raycast::{
    Base, Camera, EYE_HEIGHT_CM, FOG_CELLS, Ground, HORIZON, Map, PLANE, SKY_STEPS, Shades, Sprite,
    SpriteAt, focal, fog_step,
};

/// **How far the heightfield sees, in cells** (doc 21 cut G.7b): 1.5 km on
/// the arena's 10 m cells, fogging to the horizon over the whole of it — two
/// and a half times the raycaster's 60-cell fog, because a landscape's hills
/// are the picture and a 600 m horizon hid them.
pub const VIEW_CELLS: f32 = 150.0;

/// How far out the march begins, in cells.
const NEAR: f32 = 0.25;
/// The march's first step, in cells; each step after is [`GROWTH`] times
/// the last, so the near ground is sampled finely and the far ground less so.
const FIRST_STEP: f32 = 0.04;
/// See [`FIRST_STEP`].
const GROWTH: f32 = 1.015;

/// Ground with relief: the arena's cell grid ([`Map`]) and an elevation a
/// cell, in centimetres above the region's datum.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    map: Map,
    elev_cm: Vec<i32>,
    /// The same heights in cells (`elev_cm / cell_cm`), what every sample
    /// reads; kept beside the integers so a sample does not divide.
    height: Vec<f32>,
    /// Each cell's light (faces the north-west sun), kept up to date as
    /// heights are set rather than worked out a sample at a time.
    lit: Vec<bool>,
    /// The baked colour map, once [`Terrain::bake`] has run (setting a height
    /// afterwards drops it).
    baked: Option<ColourMap>,
}

impl Terrain {
    /// Flat ground over `map`.
    #[must_use]
    pub fn new(map: Map) -> Terrain {
        let n = map.width() * map.height();
        Terrain {
            map,
            elev_cm: vec![0; n],
            height: vec![0.0; n],
            lit: vec![true; n],
            baked: None,
        }
    }

    /// The cell grid.
    #[must_use]
    pub fn map(&self) -> &Map {
        &self.map
    }

    /// Set one cell's elevation; out of range is ignored.
    pub fn set_elev_cm(&mut self, x: usize, y: usize, cm: i32) {
        let w = self.map.width();
        if x >= w || y >= self.map.height() {
            return;
        }
        let cell = self.map.cell_cm() as f32;
        self.baked = None;
        if let Some(e) = self.elev_cm.get_mut(y * w + x) {
            *e = cm;
        }
        if let Some(h) = self.height.get_mut(y * w + x) {
            *h = cm as f32 / cell;
        }
        // This cell's light and its four neighbours' read this height.
        let (x, y) = (x as i64, y as i64);
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            self.relight(x + dx, y + dy);
        }
    }

    /// Work out one cell's light from its neighbours' heights: lit when the
    /// ground does not fall toward the south-east (away from a north-west
    /// sun) — level ground is lit.
    fn relight(&mut self, x: i64, y: i64) {
        let w = self.map.width();
        let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if ux >= w || uy >= self.map.height() {
            return;
        }
        let e = |x: i64, y: i64| self.elev_cm(x, y);
        let gx = e(x + 1, y) - e(x - 1, y);
        let gy = e(x, y + 1) - e(x, y - 1);
        if let Some(l) = self.lit.get_mut(uy * w + ux) {
            *l = gy - gx >= 0;
        }
    }

    /// One cell's height in cells, the nearest edge cell's outside the map.
    fn cell_height(&self, x: i64, y: i64) -> f32 {
        let (w, h) = (self.map.width(), self.map.height());
        if w == 0 || h == 0 {
            return 0.0;
        }
        let cx = usize::try_from(x.max(0)).unwrap_or(0).min(w - 1);
        let cy = usize::try_from(y.max(0)).unwrap_or(0).min(h - 1);
        self.height.get(cy * w + cx).copied().unwrap_or(0.0)
    }

    /// One cell's elevation, centimetres; a cell outside the map takes the
    /// nearest edge cell's, so the ground runs on past the edge.
    #[must_use]
    pub fn elev_cm(&self, x: i64, y: i64) -> i32 {
        let (w, h) = (self.map.width(), self.map.height());
        if w == 0 || h == 0 {
            return 0;
        }
        let cx = usize::try_from(x.max(0)).unwrap_or(0).min(w - 1);
        let cy = usize::try_from(y.max(0)).unwrap_or(0).min(h - 1);
        self.elev_cm.get(cy * w + cx).copied().unwrap_or(0)
    }

    /// The ground's height at a point, in cells, interpolated between the
    /// four nearest cell centres.
    #[must_use]
    pub fn height(&self, px: f32, py: f32) -> f32 {
        let (gx, gy) = (px - 0.5, py - 0.5);
        let (x0, y0) = (gx.floor(), gy.floor());
        let (fx, fy) = (gx - x0, gy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let w = self.map.width() as i64;
        let (sw, se, nw, ne) =
            if x0 >= 0 && y0 >= 0 && x0 + 1 < w && y0 + 1 < self.map.height() as i64 {
                // Inside: four reads from two rows, no clamping.
                let i = (y0 * w + x0) as usize;
                let j = i + w as usize;
                match (self.height.get(i..i + 2), self.height.get(j..j + 2)) {
                    (Some(&[a, b]), Some(&[c, d])) => (a, b, c, d),
                    _ => (0.0, 0.0, 0.0, 0.0),
                }
            } else {
                (
                    self.cell_height(x0, y0),
                    self.cell_height(x0 + 1, y0),
                    self.cell_height(x0, y0 + 1),
                    self.cell_height(x0 + 1, y0 + 1),
                )
            };
        let south = sw + (se - sw) * fx;
        let north = nw + (ne - nw) * fx;
        south + (north - south) * fy
    }

    /// What a sample at a point is, and how high its top stands, in cells:
    /// the ground, or a solid masonry wall standing on it (`Woods` and
    /// `WoodsHeavy` render as discrete 3D conifer trees over the forest floor).
    fn surface(&self, px: f32, py: f32) -> (Ground, f32) {
        let ground = self.map.get(px.floor() as i64, py.floor() as i64);
        let raised = if ground == Ground::Wall {
            ground.height_cm() as f32 / self.map.cell_cm() as f32
        } else {
            0.0
        };
        (ground, self.height(px, py) + raised)
    }

    /// Whether the cell under a point faces the sun (see [`Self::relight`]).
    fn lit(&self, px: f32, py: f32) -> bool {
        let (x, y) = (px.floor() as i64, py.floor() as i64);
        let w = self.map.width() as i64;
        if x < 0 || y < 0 || x >= w || y >= self.map.height() as i64 {
            return true;
        }
        self.lit.get((y * w + x) as usize).copied().unwrap_or(true)
    }
}

/// Texels a cell side in a baked colour map: 2.5 m texels on 10 m cells.
pub const TEXELS: usize = 4;
/// Beyond this distance (cells) a baked terrain is drawn from its coarse,
/// one-texel-a-cell map: 400 m of crisp 2.5 m texels with smooth normal
/// shading and 4 m topographic contours.
const FINE_DETAIL_CELLS: f32 = 40.0;
/// Light levels a baked texel is quantized to.
const LEVELS: usize = 5;
/// Fog steps a baked texel is drawn in (the sprites and blocks of the
/// unbaked path keep the raycaster's twelve).
const BAKED_FOG: usize = 5;
/// The light levels' brightness against the base colour.
const LEVEL_GAIN: [f32; LEVELS] = [0.56, 0.74, 0.90, 1.05, 1.18];
/// How much steeper the light sees the ground than it is, so the arena's
/// metre-high terraces read.
const RELIEF_GAIN: f32 = 4.0;
/// The ground kinds a texel can be, in table order.
const KINDS: [Ground; 7] = [
    Ground::Open,
    Ground::Grass,
    Ground::Water,
    Ground::Road,
    Ground::Wall,
    Ground::Woods,
    Ground::WoodsHeavy,
];

/// One level of a baked colour map: `kind * LEVELS + level` per texel,
/// `per_cell` texels a cell side, the southern row first.
#[derive(Clone, Debug, PartialEq)]
struct BakedLevel {
    per_cell: usize,
    w: usize,
    h: usize,
    index: Vec<u8>,
}

impl BakedLevel {
    fn get(&self, px: f32, py: f32) -> Option<u8> {
        let k = self.per_cell as f32;
        let tx = ((px * k).floor().max(0.0) as usize).min(self.w.saturating_sub(1));
        let ty = ((py * k).floor().max(0.0) as usize).min(self.h.saturating_sub(1));
        self.index.get(ty * self.w + tx).copied()
    }
}

/// A baked colour map: the fine level (2.5 m texels) for near ground, a
/// coarse one (a texel a cell) for far ground, and the colours their
/// indices stand for at each fog step.
#[derive(Clone, Debug, PartialEq)]
struct ColourMap {
    fine: BakedLevel,
    coarse: BakedLevel,
    /// Each index's colour at each fog step.
    table: Vec<[Rgb; BAKED_FOG]>,
}

impl ColourMap {
    /// The colour at a point (in cells) seen from `z` cells away, at fog
    /// `step` (of the raycaster's twelve).
    fn colour(&self, px: f32, py: f32, z: f32, step: usize) -> Rgb {
        let level = if z < FINE_DETAIL_CELLS {
            &self.fine
        } else {
            &self.coarse
        };
        let fog = (step * BAKED_FOG / crate::raycast::FOG_STEPS).min(BAKED_FOG - 1);
        level
            .get(px, py)
            .and_then(|k| self.table.get(usize::from(k)))
            .and_then(|row| row.get(fog))
            .copied()
            .unwrap_or(HORIZON)
    }

    /// Look up a specific `(kind_idx, level)` entry at fog `step` so 3D
    /// conifer trees reuse the baked terrain palette with zero extra colours.
    fn entry(&self, kind_idx: usize, level: usize, step: usize) -> Rgb {
        let fog = (step * BAKED_FOG / crate::raycast::FOG_STEPS).min(BAKED_FOG - 1);
        let idx = kind_idx * LEVELS + level.min(LEVELS - 1);
        self.table
            .get(idx)
            .and_then(|row| row.get(fog))
            .copied()
            .unwrap_or(HORIZON)
    }
}

/// A seeded value in `0..1` for a lattice point.
fn lattice(seed: u64, x: i64, y: i64) -> f32 {
    let mut h = seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Value noise at a point, smooth between lattice points `period` cells
/// apart.
fn value_noise(seed: u64, px: f32, py: f32, period: f32) -> f32 {
    let (gx, gy) = (px / period, py / period);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - x0, gy - y0);
    let (fx, fy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let a = lattice(seed, x0, y0);
    let b = lattice(seed, x0 + 1, y0);
    let c = lattice(seed, x0, y0 + 1);
    let d = lattice(seed, x0 + 1, y0 + 1);
    let south = a + (b - a) * fx;
    let north = c + (d - c) * fx;
    south + (north - south) * fy
}

impl Terrain {
    /// Bake the colour map (see the module docs): `seed` picks the mottle,
    /// so one map always bakes the same. Runs once a map; a later height
    /// change drops it.
    pub fn bake(&mut self, seed: u64) {
        let fine = self.bake_level(seed, TEXELS);
        let coarse = self.bake_level(seed, 1);
        let mut table = Vec::with_capacity(KINDS.len() * LEVELS);
        for kind in KINDS {
            let rgb = base(kind, true).rgb();
            for gain in LEVEL_GAIN {
                let lit = rgb.map(|c| (f32::from(c) * gain).round().min(255.0) as u8);
                let mut row = [[0u8; 3]; BAKED_FOG];
                for (step, px) in row.iter_mut().enumerate() {
                    let t = step as f32 / (BAKED_FOG - 1) as f32;
                    *px = [0, 1, 2].map(|i| {
                        let (a, b) = (
                            f32::from(lit.get(i).copied().unwrap_or(0)),
                            f32::from(HORIZON.get(i).copied().unwrap_or(0)),
                        );
                        (a + (b - a) * t).round() as u8
                    });
                }
                table.push(row);
            }
        }
        self.baked = Some(ColourMap {
            fine,
            coarse,
            table,
        });
    }

    /// One level of the bake at `per_cell` texels a cell side: each texel's
    /// light from the surface at its centre, with 4 m topographic contour
    /// lines on sloped ground, cliff stratification, and dappled forest-floor
    /// canopy shading under woods (see the module docs).
    fn bake_level(&self, seed: u64, per_cell: usize) -> BakedLevel {
        let (cw, ch) = (self.map.width(), self.map.height());
        let (w, h) = (cw * per_cell, ch * per_cell);
        let cell_m = (self.map.cell_cm() as f32 * 0.01).max(1.0);
        // The neighbourhood's mean height a cell, for the hollows.
        let mut mean = vec![0.0f32; cw * ch];
        for y in 0..ch as i64 {
            for x in 0..cw as i64 {
                let mut sum = 0.0;
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        sum += self.cell_height(x + dx, y + dy);
                    }
                }
                if let Some(m) = mean.get_mut(y as usize * cw + x as usize) {
                    *m = sum / 25.0;
                }
            }
        }
        // The sun: from the north-west, 45° up.
        let (lx, ly, lz) = (-0.5f32, 0.5f32, std::f32::consts::FRAC_1_SQRT_2);
        // The slope is read across a quarter cell whatever the level, so the
        // two levels light the same ground alike.
        let d = 1.0 / TEXELS as f32;
        let step = 1.0 / per_cell as f32;
        let mut index = vec![0u8; w * h];
        for ty in 0..h {
            for tx in 0..w {
                let (px, py) = ((tx as f32 + 0.5) * step, (ty as f32 + 0.5) * step);
                let ground = self.map.get(px.floor() as i64, py.floor() as i64);
                let kind = KINDS.iter().position(|k| *k == ground).unwrap_or(0);
                let h_here = self.height(px, py);
                let dhdx =
                    (self.height(px + d, py) - self.height(px - d, py)) / (2.0 * d) * RELIEF_GAIN;
                let dhdy =
                    (self.height(px, py + d) - self.height(px, py - d)) / (2.0 * d) * RELIEF_GAIN;
                let norm = (dhdx * dhdx + dhdy * dhdy + 1.0).sqrt();
                let lambert = ((-dhdx * lx - dhdy * ly + lz) / norm).max(0.0);
                let steep = (dhdx * dhdx + dhdy * dhdy).sqrt();
                let cell = (py.floor().max(0.0) as usize).min(ch.saturating_sub(1)) * cw
                    + (px.floor().max(0.0) as usize).min(cw.saturating_sub(1));
                let hollow = mean.get(cell).map_or(0.0, |m| h_here - m);
                let mottle =
                    0.6 * value_noise(seed, px, py, 2.0) + 0.4 * value_noise(seed ^ 1, px, py, 5.0);

                // Subtle 4 m topographic contour line on sloped ground
                let elev_m = h_here * cell_m;
                let contour_phase = (elev_m * 0.25).rem_euclid(1.0);
                let contour_drop = if per_cell > 1 && steep > 0.14 && contour_phase < 0.08 {
                    -0.14
                } else {
                    0.0
                };

                // Dappled woodland canopy shadow on the forest floor under 3D trees
                let canopy_dapple = if matches!(ground, Ground::Woods | Ground::WoodsHeavy) {
                    (value_noise(seed ^ 3, px, py, 0.75) - 0.55) * 0.22
                } else {
                    0.0
                };

                let light = 0.30 + 0.75 * lambert - 0.25 * steep.min(1.2)
                    + (hollow * RELIEF_GAIN).clamp(-0.15, 0.08)
                    + (mottle - 0.5) * 0.14
                    + contour_drop
                    + canopy_dapple;
                let level = if light < 0.44 {
                    0
                } else if light < 0.58 {
                    1
                } else if light < 0.73 {
                    2
                } else if light < 0.89 {
                    3
                } else {
                    4
                };
                if let Some(slot) = index.get_mut(ty * w + tx) {
                    *slot = u8::try_from(kind * LEVELS + level).unwrap_or(0);
                }
            }
        }
        BakedLevel {
            per_cell,
            w,
            h,
            index,
        }
    }

    /// Whether [`bake`](Self::bake) has run since the last height change.
    #[must_use]
    pub fn is_baked(&self) -> bool {
        self.baked.is_some()
    }
}

/// The colour base a sample is drawn in, lit or shaded.
fn base(ground: Ground, lit: bool) -> Base {
    match ground {
        Ground::Wall if lit => Base::WallLit,
        Ground::Wall => Base::WallDark,
        Ground::Woods => Base::Woods,
        Ground::WoodsHeavy => Base::WoodsHeavy,
        // The floor's "line" shade is its darker twin: the shaded slope.
        g => Base::floor(g, !lit),
    }
}

/// The camera in cell units and the picture's scale: where the eye stands,
/// the way it looks, the camera plane across it, the eye's height over the
/// datum, the focal length and the horizon row. The ground march, the
/// sprites and the HUD's lock box ([`project`]) all read the one `View`, so
/// a point of the world lands on the same pixel for each of them.
#[derive(Clone, Copy)]
struct View {
    cell: f32,
    posx: f32,
    posy: f32,
    dirx: f32,
    diry: f32,
    planex: f32,
    planey: f32,
    /// The eye's height over the datum, cells.
    eye: f32,
    /// The focal length, pixels ([`focal`]).
    hf: f32,
    /// The horizon's row.
    horizon: f32,
    /// Half the frame's width, pixels.
    halfw: f32,
    /// The inverse of the camera matrix's determinant, for the sprites.
    invdet: f32,
}

impl View {
    /// The view from `cam` over `terrain` into a `w` by `h` frame, the eye at
    /// `eye_cm` over the datum when given, else 1.5 m (the stock sensor
    /// height) above the ground under the camera.
    fn new(terrain: &Terrain, cam: &Camera, eye_cm: Option<i64>, w: usize, h: usize) -> View {
        let cell = terrain.map.cell_cm() as f32;
        let posx = cam.x_cm as f32 / cell;
        let posy = cam.y_cm as f32 / cell;
        let heading = (cam.heading_mdeg % 360_000) as f32 / 1000.0;
        let (dirx, diry) = heading.to_radians().det_sin_cos();
        let (planex, planey) = (diry * PLANE, -dirx * PLANE);
        let eye = eye_cm.map_or_else(
            || terrain.height(posx, posy) + EYE_HEIGHT_CM as f32 / cell,
            |z| z as f32 / cell,
        );
        View {
            cell,
            posx,
            posy,
            dirx,
            diry,
            planex,
            planey,
            eye,
            hf: focal(w),
            horizon: (h / 2) as f32,
            halfw: w as f32 / 2.0,
            invdet: 1.0 / (planex * diry - dirx * planey),
        }
    }

    /// Where `sprite` stands in the picture: its centre column, the row its
    /// base stands on (the ground's height at its own point, seen from the
    /// eye), its distance and its size in pixels; `None` behind the eye.
    fn place(&self, terrain: &Terrain, sprite: &Sprite) -> Option<SpriteAt> {
        let (wx, wy) = (
            sprite.x_cm as f32 / self.cell,
            sprite.y_cm as f32 / self.cell,
        );
        let (relx, rely) = (wx - self.posx, wy - self.posy);
        let tx = self.invdet * (self.diry * relx - self.dirx * rely);
        let ty = self.invdet * (-self.planey * relx + self.planex * rely);
        if ty <= 0.05 {
            return None;
        }
        let ground = terrain.height(wx, wy);
        let (wcm, hcm) = sprite.size_cm();
        Some(SpriteAt {
            screenx: self.halfw * (1.0 + tx / ty),
            bottom: self.horizon + (self.eye - ground) * self.hf / ty,
            depth: ty,
            fog_depth: ty * FOG_CELLS / VIEW_CELLS,
            height: hcm / self.cell * self.hf / ty,
            width: wcm / self.cell * self.hf / ty,
            kind: sprite.kind,
            pose: sprite.pose,
        })
    }
}

/// Where `sprite` stands in a `w` by `h` picture of `terrain` from `cam`
/// with the eye at `eye_cm` (as [`render`] takes it): its centre column, the
/// row its base stands on, its distance and its size, exactly as [`render`]
/// draws it; `None` behind the eye. The cockpit's HUD boxes the lock with it
/// (doc 21a `X-03`).
pub(crate) fn project(
    terrain: &Terrain,
    cam: &Camera,
    eye_cm: Option<i64>,
    w: usize,
    h: usize,
    sprite: &Sprite,
) -> Option<SpriteAt> {
    View::new(terrain, cam, eye_cm, w, h).place(terrain, sprite)
}

/// Draw `terrain` from `cam` into `frame`, with `sprites` standing on it.
/// The eye is at `eye_cm` when it is given — the cockpit frame's `z_cm` plus
/// its `sensor_height_cm`, the very eye the sim traces the unit's sight from
/// (doc 21 cut G.7b) — else 1.5 m (the stock sensor height) above the ground
/// under the camera.
pub fn render(
    frame: &mut Frame,
    terrain: &Terrain,
    cam: &Camera,
    eye_cm: Option<i64>,
    sprites: &[Sprite],
) {
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return;
    }
    let view = View::new(terrain, cam, eye_cm, w, h);
    let View {
        posx,
        posy,
        dirx,
        diry,
        planex,
        planey,
        eye,
        hf,
        horizon,
        ..
    } = view;
    let shades = Shades::new();

    // The lowest row each column has drawn down to (exclusive): ground
    // above it is hidden by nearer ground below it.
    let mut ybuf = vec![h; w];
    // The distance each pixel was drawn at, for the sprites.
    let mut depth = vec![f32::INFINITY; w * h];
    let wf = w as f32;

    let mut z = NEAR;
    let mut dz = FIRST_STEP;
    while z < VIEW_CELLS {
        let (lx, ly) = (posx + (dirx - planex) * z, posy + (diry - planey) * z);
        let (sx, sy) = (2.0 * planex * z / wf, 2.0 * planey * z / wf);
        let step = fog_step(z * FOG_CELLS / VIEW_CELLS);
        for (i, low) in ybuf.iter_mut().enumerate() {
            let t = i as f32 + 0.5;
            let (px, py) = (lx + sx * t, ly + sy * t);
            let (ground, top) = terrain.surface(px, py);
            let y = horizon + (eye - top) * hf / z;
            let start = y.max(0.0).ceil() as usize;
            if start >= *low {
                continue;
            }
            let px_rgb = match &terrain.baked {
                Some(map) => map.colour(px, py, z, step),
                None => shades.shade(base(ground, terrain.lit(px, py)), step),
            };
            for row in start..*low {
                frame.put(i, row, px_rgb);
                if let Some(d) = depth.get_mut(row * w + i) {
                    *d = z;
                }
            }
            *low = start;
        }
        z += dz;
        dz *= GROWTH;
    }

    // The sky above every column's ground, and the horizon's haze between
    // the sky and ground too far to draw.
    for (i, &low) in ybuf.iter().enumerate() {
        for row in 0..low {
            let band = row * SKY_STEPS / (h / 2).max(1);
            let px = if row < h / 2 {
                shades.sky(band)
            } else {
                HORIZON
            };
            frame.put(i, row, px);
        }
    }

    draw_trees(frame, terrain, &mut depth, &shades, &view);
    draw_sprites(frame, terrain, sprites, &depth, &shades, &view);
}

/// Maximum distance (in cells) at which discrete 3D conifer trees are rendered
/// over `Ground::Woods` and `Ground::WoodsHeavy` cells.
const TREE_VIEW_CELLS: f32 = 85.0;

#[derive(Clone, Copy)]
struct PlacedTree {
    screenx: f32,
    bottom: f32,
    ty: f32,
    h_px: i32,
    heavy: bool,
    seed_h2: u32,
}

/// Render 3D multi-tiered conifer tree groves (`3` trees per `Ground::Woods`
/// cell, `6` per `Ground::WoodsHeavy` cell, at the exact same deterministic
/// sub-cell positions as [`crate::iso25d`]) standing on the sloped terrain and
/// writing into `depth` so trees and ridges properly occlude one another and
/// unit contacts.
fn draw_trees(
    frame: &mut Frame,
    terrain: &Terrain,
    depth: &mut [f32],
    shades: &Shades,
    view: &View,
) {
    let (w, h) = (frame.width(), frame.height());
    let (mw, mh) = (terrain.map.width() as i64, terrain.map.height() as i64);
    let r_cells = TREE_VIEW_CELLS.ceil() as i64;
    let cx0 = (view.posx.floor() as i64 - r_cells).max(0);
    let cx1 = (view.posx.floor() as i64 + r_cells).min(mw - 1);
    let cy0 = (view.posy.floor() as i64 - r_cells).max(0);
    let cy1 = (view.posy.floor() as i64 + r_cells).min(mh - 1);
    if cx0 > cx1 || cy0 > cy1 {
        return;
    }

    let cell_m = (view.cell * 0.01).max(1.0);
    let mut placed = Vec::with_capacity(256);

    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            let g = terrain.map.get(cx, cy);
            let tree_count: u32 = match g {
                Ground::Woods => 3,
                Ground::WoodsHeavy => 6,
                _ => continue,
            };
            // Quick cell-centre frustum culling before expanding individual trees
            let crx = (cx as f32 + 0.5) - view.posx;
            let cry = (cy as f32 + 0.5) - view.posy;
            let cty = view.invdet * (-view.planey * crx + view.planex * cry);
            if cty < -0.5 || cty > TREE_VIEW_CELLS + 1.0 {
                continue;
            }
            let ctx = view.invdet * (view.diry * crx - view.dirx * cry);
            if ctx.abs() > cty.max(1.0) * 1.25 + 1.5 {
                continue;
            }

            let heavy = g == Ground::WoodsHeavy;
            let base_h_m = if heavy { 8.5 } else { 7.0 };

            for k in 0..tree_count {
                let seed = ((cx as u32).wrapping_mul(0x9E37_79B9))
                    ^ ((cy as u32).wrapping_mul(0x85EB_CA6B))
                    ^ (k.wrapping_mul(0xC2B2_AE35));
                let h1 = seed.wrapping_mul(0x27D4_EB2D);
                let h2 = h1.rotate_left(13).wrapping_mul(0x1656_67B1);
                let ox = 0.14 + ((h1 & 0xFF) as f32 / 255.0) * 0.72;
                let oy = 0.14 + (((h1 >> 8) & 0xFF) as f32 / 255.0) * 0.72;
                let wx = cx as f32 + ox;
                let wy = cy as f32 + oy;
                let relx = wx - view.posx;
                let rely = wy - view.posy;
                let ty = view.invdet * (-view.planey * relx + view.planex * rely);
                if ty <= 0.35 || ty > TREE_VIEW_CELLS {
                    continue;
                }
                let tx = view.invdet * (view.diry * relx - view.dirx * rely);
                if tx.abs() > ty * 1.15 + 0.8 {
                    continue;
                }
                let var_h_m = ((h2 & 0x3F) as f32 / 63.0) * 3.0;
                let tree_h_cells = (base_h_m + var_h_m) / cell_m;
                let h_px = (tree_h_cells * view.hf / ty).round() as i32;
                if h_px < 3 {
                    continue;
                }
                let gz = terrain.height(wx, wy);
                let screenx = view.halfw * (1.0 + tx / ty);
                let bottom = view.horizon + (view.eye - gz) * view.hf / ty;
                placed.push(PlacedTree {
                    screenx,
                    bottom,
                    ty,
                    h_px: h_px.min(h as i32 * 2),
                    heavy,
                    seed_h2: h2,
                });
            }
        }
    }

    // Sort front-to-back so nearer trees write `depth` first and skip occluded pixels.
    placed.sort_unstable_by(|a, b| a.ty.total_cmp(&b.ty));

    // World-space NW sun projected onto camera right (`sun_side`) and camera back (`sun_back`).
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    let sun_side = -inv_sqrt2 * view.diry - inv_sqrt2 * view.dirx;
    let sun_back = inv_sqrt2 * view.dirx - inv_sqrt2 * view.diry;

    for tree in placed {
        let bx = tree.screenx.round() as i32;
        let by = tree.bottom.round() as i32;
        let h_px = tree.h_px;
        let max_r = ((h_px as f32) * 0.30).round() as i32;
        let max_r = max_r.clamp(1, (w as i32 / 2).max(1));
        if bx + max_r < 0 || bx - max_r >= w as i32 || by < 0 || by - h_px >= h as i32 {
            continue;
        }

        let step = fog_step(tree.ty * FOG_CELLS / VIEW_CELLS);
        let (trunk_lit, trunk_dark, needle_0, needle_1, needle_2, needle_3, needle_4) =
            match &terrain.baked {
                Some(map) => (
                    map.entry(0, 1, step), // Open level 1 (sunlit bark)
                    map.entry(0, 0, step), // Open level 0 (shaded bark)
                    map.entry(6, 0, step), // WoodsHeavy level 0 (deep bough shadow)
                    map.entry(6, 1, step), // WoodsHeavy level 1 (inner conifer shade)
                    map.entry(5, 1, step), // Woods level 1 (mid conifer)
                    map.entry(5, 3, step), // Woods level 3 (sunlit conifer bough)
                    map.entry(1, 3, step), // Grass level 3 (warm sunlit needle tip)
                ),
                None => (
                    shades.shade(Base::Open, step),
                    shades.shade(Base::OpenLine, step),
                    shades.shade(Base::Outline, step),
                    shades.shade(Base::WoodsHeavy, step),
                    shades.shade(Base::Woods, step),
                    shades.shade(Base::GrassLine, step),
                    shades.shade(Base::Grass, step),
                ),
            };

        let trunk_h = (h_px / 6).max(1);
        let trunk_r = (max_r / 5).clamp(0, 3);

        // 1. Bark trunk at the base of the tree
        for dy in 0..=trunk_h {
            let py = by - dy;
            if py < 0 || py >= h as i32 {
                continue;
            }
            let py_u = py as usize;
            for dx in -trunk_r..=trunk_r {
                let px = bx + dx;
                if px < 0 || px >= w as i32 {
                    continue;
                }
                let px_u = px as usize;
                let idx = py_u * w + px_u;
                if tree.ty < depth.get(idx).copied().unwrap_or(f32::INFINITY) {
                    let lit = (dx as f32) * sun_side >= 0.0;
                    frame.put(px_u, py_u, if lit { trunk_lit } else { trunk_dark });
                    if let Some(d) = depth.get_mut(idx) {
                        *d = tree.ty;
                    }
                }
            }
        }

        // 2. 3-tiered conical conifer canopy from `trunk_h` to `h_px`
        let crown_h = (h_px - trunk_h).max(2);
        for dy in 0..=crown_h {
            let py = by - trunk_h - dy;
            if py < 0 || py >= h as i32 {
                continue;
            }
            let py_u = py as usize;
            let t = dy as f32 / crown_h as f32;
            let tier_local = (t * 3.0).rem_euclid(1.0);
            let tier_flare = (1.0 - tier_local * 0.42) * (1.04 - 0.80 * t);
            let r_row = ((max_r as f32) * tier_flare).round() as i32;

            for dx in -r_row..=r_row {
                let px = bx + dx;
                if px < 0 || px >= w as i32 {
                    continue;
                }
                let edge_hash = ((dx as u32).wrapping_mul(73_856_093))
                    ^ ((dy as u32).wrapping_mul(19_349_663))
                    ^ tree.seed_h2;
                if dx.abs() == r_row && r_row >= 2 && (edge_hash & 1) != 0 {
                    continue;
                }
                let px_u = px as usize;
                let idx = py_u * w + px_u;
                if tree.ty >= depth.get(idx).copied().unwrap_or(f32::INFINITY) {
                    continue;
                }

                let nx = if r_row > 0 {
                    dx as f32 / r_row as f32
                } else {
                    0.0
                };
                let sun_lat = nx * sun_side + 0.22 * sun_back;
                let vertical_shade = 0.20 * tier_local + 0.14 * t;
                let noise = (((edge_hash >> 4) & 0x7) as f32 / 7.0 - 0.5) * 0.16;
                let heavy_bias = if tree.heavy { -0.06 } else { 0.0 };
                let illum = 0.48 + 0.42 * sun_lat + vertical_shade + noise + heavy_bias;

                let col = if tier_local < 0.15 && illum < 0.48 {
                    needle_0
                } else if illum < 0.44 {
                    needle_1
                } else if illum < 0.62 {
                    needle_2
                } else if illum < 0.82 {
                    needle_3
                } else {
                    needle_4
                };

                frame.put(px_u, py_u, col);
                if let Some(d) = depth.get_mut(idx) {
                    *d = tree.ty;
                }
            }
        }
    }
}

/// The sprites, far to near, each standing on the ground at its own point
/// ([`View::place`]) and hidden where nearer ground or trees were drawn.
fn draw_sprites(
    frame: &mut Frame,
    terrain: &Terrain,
    sprites: &[Sprite],
    depth: &[f32],
    shades: &Shades,
    view: &View,
) {
    let w = frame.width();
    let mut placed: Vec<SpriteAt> = sprites
        .iter()
        .filter_map(|s| view.place(terrain, s))
        .collect();
    placed.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let depth_at = |x: usize, y: usize| depth.get(y * w + x).copied().unwrap_or(f32::INFINITY);
    for at in placed {
        draw_mip_sprite_fp(frame, shades, at, &depth_at);
    }
}

const EMBEDDED_MIPS: &[u8] = include_bytes!("../looks/iso25d_mips.bin");
const MIP_FALLBACK_SIZES: [usize; 6] = [96, 86, 78, 70, 64, 56];

/// Decoded single mip level (`(6 * n) × (8 * n)` material codes `0..=6`).
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "encode"), allow(dead_code))]
pub(crate) struct BakedMip {
    pub(crate) size: usize,
    pub(crate) anchor_x: usize,
    pub(crate) anchor_y: usize,
    /// Row-major `(8 * size)` rows by `(6 * size)` columns; `0` = transparent,
    /// `1..=6` = [`crate::look::Material`].
    pub(crate) pixels: Vec<u8>,
}

/// All 6 mipmap levels for the three unit model families.
pub(crate) struct BakedMipPack {
    pub(crate) models: [[BakedMip; 6]; 3],
}

static MIP_PACK: std::sync::OnceLock<BakedMipPack> = std::sync::OnceLock::new();

pub(crate) fn mip_pack() -> &'static BakedMipPack {
    MIP_PACK.get_or_init(load_mip_pack)
}

fn decode_rle(rle: &[u8], expected_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected_len);
    for &b in rle {
        let mat = b & 0x07;
        let run = usize::from(b >> 3) + 1;
        let limit = run.min(expected_len.saturating_sub(out.len()));
        out.resize(out.len() + limit, mat);
    }
    if out.len() < expected_len {
        out.resize(expected_len, 0);
    }
    out
}

fn fallback_mip_from_sheet(sheet: Option<&crate::look::Sheet>, n: usize) -> BakedMip {
    use crate::look::{Facing, Material};
    let sheet_w = 6 * n;
    let sheet_h = 8 * n;
    let mut pixels = vec![0u8; sheet_w * sheet_h];
    let Some(s) = sheet else {
        return BakedMip {
            size: n,
            anchor_x: n / 2,
            anchor_y: (n * 3) / 4,
            pixels,
        };
    };
    let (sw, sh) = (s.width.max(1), s.height.max(1));
    let scale_w = (n * 3 / 4).max(1);
    let scale_h = (scale_w * sh / sw).clamp(1, (n * 3 / 4).max(1));
    let off_x = (n.saturating_sub(scale_w)) / 2;
    let off_y = n.saturating_sub(scale_h + n / 8);

    for (fi, &facing) in Facing::ALL.iter().enumerate() {
        for pi in 0..6usize {
            let resolved = if pi == 5 {
                s.wreck_frame(facing).map(|f| (f, false))
            } else if pi == 0 {
                s.frame(facing, false, 0)
            } else {
                s.frame(facing, true, pi - 1)
            };
            let Some((frame_pixels, mirror)) = resolved else {
                continue;
            };
            for dy in 0..scale_h {
                let sy = (dy * sh / scale_h).min(sh.saturating_sub(1));
                let dst_r = fi * n + off_y + dy;
                for dx in 0..scale_w {
                    let sx_raw = (dx * sw / scale_w).min(sw.saturating_sub(1));
                    let sx = if mirror {
                        sw.saturating_sub(1 + sx_raw)
                    } else {
                        sx_raw
                    };
                    if let Some(Some(mat)) = frame_pixels.get(sy * sw + sx) {
                        let code = match mat {
                            Material::TeamDark => 1,
                            Material::Team => 2,
                            Material::TeamLight => 3,
                            Material::Black => 4,
                            Material::Steel => 5,
                            Material::Pale => 6,
                        };
                        if let Some(dst) = pixels.get_mut(dst_r * sheet_w + pi * n + off_x + dx) {
                            *dst = code;
                        }
                    }
                }
            }
        }
    }
    BakedMip {
        size: n,
        anchor_x: n / 2,
        anchor_y: off_y + scale_h.saturating_sub(1),
        pixels,
    }
}

fn load_mip_pack() -> BakedMipPack {
    let mut cursor = EMBEDDED_MIPS;
    let mut parsed_models: Vec<Vec<BakedMip>> = Vec::new();
    if cursor.len() >= 6 && cursor.get(..4) == Some(b"M25D") {
        let unit_count = cursor.get(5).copied().unwrap_or(0) as usize;
        cursor = cursor.get(6..).unwrap_or(&[]);
        for _ in 0..unit_count {
            let mip_count = cursor.first().copied().unwrap_or(0) as usize;
            cursor = cursor.get(1..).unwrap_or(&[]);
            let mut mips = Vec::with_capacity(mip_count);
            for _ in 0..mip_count {
                if cursor.len() < 10 {
                    break;
                }
                let n = u16::from_le_bytes([
                    cursor.first().copied().unwrap_or(0),
                    cursor.get(1).copied().unwrap_or(0),
                ]) as usize;
                let ax = u16::from_le_bytes([
                    cursor.get(2).copied().unwrap_or(0),
                    cursor.get(3).copied().unwrap_or(0),
                ]) as usize;
                let ay = u16::from_le_bytes([
                    cursor.get(4).copied().unwrap_or(0),
                    cursor.get(5).copied().unwrap_or(0),
                ]) as usize;
                let rle_len = u32::from_le_bytes([
                    cursor.get(6).copied().unwrap_or(0),
                    cursor.get(7).copied().unwrap_or(0),
                    cursor.get(8).copied().unwrap_or(0),
                    cursor.get(9).copied().unwrap_or(0),
                ]) as usize;
                cursor = cursor.get(10..).unwrap_or(&[]);
                let rle = cursor.get(..rle_len).unwrap_or(&[]);
                cursor = cursor.get(rle_len..).unwrap_or(&[]);
                let pixels = decode_rle(rle, 6 * n * 8 * n);
                mips.push(BakedMip {
                    size: n,
                    anchor_x: ax,
                    anchor_y: ay,
                    pixels,
                });
            }
            parsed_models.push(mips);
        }
    }

    let unk_sheet = crate::look::unknown().and_then(|l| l.sprites.as_ref());
    let biped_sheet =
        crate::look::for_unit("tacsim.unit.biped_mk1").and_then(|l| l.sprites.as_ref());
    let tank_sheet =
        crate::look::for_unit("tacsim.unit.line_tank").and_then(|l| l.sprites.as_ref());

    let build_six = |idx: usize, fallback: Option<&crate::look::Sheet>| -> [BakedMip; 6] {
        std::array::from_fn(|mi| {
            let target_n = MIP_FALLBACK_SIZES.get(mi).copied().unwrap_or(64);
            parsed_models
                .get(idx)
                .and_then(|m| m.get(mi))
                .cloned()
                .unwrap_or_else(|| fallback_mip_from_sheet(fallback, target_n))
        })
    };

    BakedMipPack {
        models: [
            build_six(0, biped_sheet),
            build_six(1, tank_sheet),
            build_six(2, unk_sheet),
        ],
    }
}

pub(crate) fn material_from_code(code: u8) -> Option<crate::look::Material> {
    use crate::look::Material;
    match code {
        1 => Some(Material::TeamDark),
        2 => Some(Material::Team),
        3 => Some(Material::TeamLight),
        4 => Some(Material::Black),
        5 => Some(Material::Steel),
        6 => Some(Material::Pale),
        _ => None,
    }
}

/// Draw a first-person unit sprite (`at`) using the 3D-baked 8-facing × 6-pose
/// mipmapped models when `at.pose` is a tank or biped, falling back to
/// [`crate::raycast::draw_sprite`] for `None` or unclassified silhouettes.
fn draw_mip_sprite_fp(
    frame: &mut Frame,
    shades: &Shades,
    at: SpriteAt,
    depth_at: &dyn Fn(usize, usize) -> f32,
) {
    use crate::look::{Facing, Material};
    use crate::raycast::SpriteKind;

    let Some(pose) = at.pose else {
        crate::raycast::draw_sprite(frame, shades, at, depth_at);
        return;
    };
    let biped_sheet =
        crate::look::for_unit("tacsim.unit.biped_mk1").and_then(|l| l.sprites.as_ref());
    let tank_sheet =
        crate::look::for_unit("tacsim.unit.line_tank").and_then(|l| l.sprites.as_ref());
    let model_idx = if biped_sheet.is_some_and(|s| std::ptr::eq(pose.sheet, s))
        || pose.sheet.size_cm == (520, 1100)
    {
        0usize // UnitModel::Biped
    } else if tank_sheet.is_some_and(|s| std::ptr::eq(pose.sheet, s))
        || pose.sheet.size_cm == (700, 250)
    {
        1usize // UnitModel::Tank
    } else {
        crate::raycast::draw_sprite(frame, shades, at, depth_at);
        return;
    };

    let (w, h) = (frame.width(), frame.height());
    let wf = w as f32;
    if at.screenx + at.width / 2.0 < 0.0 || at.screenx - at.width / 2.0 >= wf {
        return;
    }

    let mut drawn = 0usize;
    let step = fog_step(at.fog_depth);
    if at.height >= 2.0 && at.width >= 1.0 {
        let pack = mip_pack();
        let target_px = at.width.max(at.height);
        let mut chosen_mi = 0usize;
        if let Some(mips) = pack.models.get(model_idx) {
            for (mi, m) in mips.iter().enumerate() {
                if (m.size as f32) * 0.72 >= target_px * 0.85 {
                    chosen_mi = mi;
                }
            }
        }
        if let Some(mip) = pack.models.get(model_idx).and_then(|m| m.get(chosen_mi)) {
            let n = mip.size.max(1);
            let raw_w = 6 * n;
            let fi = Facing::ALL.iter().position(|&f| f == pose.facing).unwrap_or(0);
            let pi = if at.kind == SpriteKind::Wreck {
                5
            } else if !pose.moving {
                0
            } else {
                1 + (pose.step % 4)
            };

            let mut min_x = n;
            let mut max_x = 0usize;
            let mut min_y = n;
            let mut max_y = 0usize;
            for lr in 0..n {
                let row_off = (fi * n + lr) * raw_w + pi * n;
                for lc in 0..n {
                    if mip.pixels.get(row_off + lc).copied().unwrap_or(0) != 0 {
                        min_x = min_x.min(lc);
                        max_x = max_x.max(lc);
                        min_y = min_y.min(lr);
                        max_y = max_y.max(lr);
                    }
                }
            }

            if min_x <= max_x && min_y <= max_y {
                let mut ref_max_w = (max_x - min_x + 1) as f32;
                for f_scan in 0..8usize {
                    let mut f_min_x = n;
                    let mut f_max_x = 0usize;
                    for lr in 0..n {
                        let row_off = (f_scan * n + lr) * raw_w;
                        for lc in 0..n {
                            if mip.pixels.get(row_off + lc).copied().unwrap_or(0) != 0 {
                                f_min_x = f_min_x.min(lc);
                                f_max_x = f_max_x.max(lc);
                            }
                        }
                    }
                    if f_min_x <= f_max_x {
                        ref_max_w = ref_max_w.max((f_max_x - f_min_x + 1) as f32);
                    }
                }

                let src_w = (max_x - min_x + 1) as f32;
                let src_h = (max_y - min_y + 1) as f32;
                let width_ratio = (src_w / ref_max_w.max(1.0)).clamp(0.58, 1.0);
                let draw_w = (at.width * width_ratio).max(1.0);
                let draw_h = at.height.max(2.0);
                let left = at.screenx - draw_w * 0.5;
                let top = at.bottom - draw_h;

                // 1. Ground shadow ellipse under the unit's base
                let rx = draw_w * 0.44;
                let ry = (rx * 0.16).max(1.0);
                let shadow = shades.paint(crate::raycast::Paint::Shadow, step);
                let sx0 = (at.screenx - rx).floor().max(0.0) as usize;
                let sx1 = ((at.screenx + rx).ceil().max(0.0) as usize).min(w);
                let sy0 = (at.bottom - ry).floor().max(0.0) as usize;
                let sy1 = ((at.bottom + ry).ceil().max(0.0) as usize).min(h);
                for y in sy0..sy1 {
                    for x in sx0..sx1 {
                        let dx = (x as f32 + 0.5 - at.screenx) / rx.max(0.5);
                        let dy = (y as f32 + 0.5 - at.bottom) / ry.max(0.5);
                        if dx * dx + dy * dy <= 1.0 && depth_at(x, y) >= at.depth * 0.85 {
                            frame.put(x, y, shadow);
                        }
                    }
                }

                // 2. 3D-baked unit body + 1px dark silhouette rim
                let pad_src = if draw_h >= 8.0 { 1.0 } else { 0.0 };
                let x0 = (left - 1.0).floor().max(0.0) as usize;
                let x1 = ((left + draw_w + 1.0).ceil().max(0.0) as usize).min(w);
                let y0 = (top - 1.0).floor().max(0.0) as usize;
                let y1 = ((top + draw_h + 1.0).ceil().max(0.0) as usize).min(h);
                let outline = shades.shade(Base::Outline, step);

                for y in y0..y1 {
                    let v = (y as f32 + 0.5 - top) / draw_h;
                    let sy_f = min_y as f32 + v * src_h;
                    let lr = sy_f.floor() as isize;
                    if lr < (min_y as isize - 1) || lr > (max_y as isize + 1) {
                        continue;
                    }
                    for x in x0..x1 {
                        if at.depth >= depth_at(x, y) {
                            continue;
                        }
                        let u = (x as f32 + 0.5 - left) / draw_w;
                        let sx_f = min_x as f32 + u * src_w;
                        let lc = sx_f.floor() as isize;
                        if lc < (min_x as isize - 1) || lc > (max_x as isize + 1) {
                            continue;
                        }
                        let code = if lr >= 0
                            && (lr as usize) < n
                            && lc >= 0
                            && (lc as usize) < n
                        {
                            mip.pixels
                                .get((fi * n + lr as usize) * raw_w + pi * n + lc as usize)
                                .copied()
                                .unwrap_or(0)
                        } else {
                            0
                        };
                        if let Some(material) = material_from_code(code) {
                            frame.put(
                                x,
                                y,
                                crate::raycast::sprite_colour(shades, at.kind, material, step),
                            );
                            drawn += 1;
                        } else if pad_src > 0.0 {
                            let mut adj = false;
                            for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                                let nr = lr + dr;
                                let nc = lc + dc;
                                if nr >= 0
                                    && (nr as usize) < n
                                    && nc >= 0
                                    && (nc as usize) < n
                                    && mip
                                        .pixels
                                        .get((fi * n + nr as usize) * raw_w + pi * n + nc as usize)
                                        .copied()
                                        .unwrap_or(0)
                                        != 0
                                {
                                    adj = true;
                                    break;
                                }
                            }
                            if adj {
                                frame.put(x, y, outline);
                                drawn += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    let faint = at.height < crate::raycast::MARK_FAR_PX
        || step >= crate::raycast::FOG_STEPS * 2 / 3;
    if drawn == 0 || faint {
        let least = if drawn == 0 {
            crate::raycast::MARK_MIN_PX
        } else {
            crate::raycast::MARK_FAR_PX
        };
        let bw = at.width.max(least);
        let bh = at.height.max(least);
        let cy = at.bottom - at.height / 2.0;
        crate::raycast::mark(
            frame,
            (at.screenx - bw / 2.0, cy - bh / 2.0, bw, bh),
            crate::raycast::sprite_colour(shades, at.kind, Material::Team, 0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Material;
    use crate::raycast::{SpriteKind, sprite_colour};

    /// A 64-cell square of grass with a crater in the middle: a bowl 12 m
    /// deep and 16 cells across, its rim 6 m above the plain.
    fn crater() -> Terrain {
        let mut map = Map::new(64, 64, 1000);
        for y in 0..64 {
            for x in 0..64 {
                map.set(x, y, Ground::Grass);
            }
        }
        let mut t = Terrain::new(map);
        for y in 0..64 {
            for x in 0..64 {
                let d = (((x as f32 - 32.0) * (x as f32 - 32.0)
                    + (y as f32 - 32.0) * (y as f32 - 32.0))
                    .sqrt())
                    / 8.0;
                let cm = if d < 1.0 {
                    -1200.0 + 1800.0 * d * d
                } else {
                    600.0 * (-(d - 1.0) * 2.0).det_exp()
                };
                t.set_elev_cm(x, y, cm as i32);
            }
        }
        t
    }

    /// A baked map is the same for the same seed, darkens a slope that faces
    /// across the sun (which the per-cell light leaves lit), and keeps a
    /// frame inside the palette.
    #[test]
    fn a_baked_map_reads_every_slope_and_stays_in_the_palette() {
        let mut t = crater();
        t.bake(7);
        assert!(t.is_baked());
        let again = {
            let mut u = crater();
            u.bake(7);
            u.baked
        };
        assert!(again == t.baked, "the same seed bakes the same map");
        // A ramp rising toward the south-west, 1.5 m a cell each way, faces
        // north-east: across a north-west sun. The per-cell light calls it
        // lit, the baked map shades it.
        let mut map = Map::new(32, 32, 1000);
        for y in 0..32 {
            for x in 0..32 {
                map.set(x, y, Ground::Grass);
            }
        }
        let mut ramp = Terrain::new(map);
        for y in 0..32 {
            for x in 0..32 {
                let cm = if (8..24).contains(&x) && (8..24).contains(&y) {
                    (48 - x as i32 - y as i32) * 150
                } else {
                    0
                };
                ramp.set_elev_cm(x, y, cm);
            }
        }
        assert!(ramp.lit(15.5, 15.5), "per cell: lit");
        ramp.bake(1);
        let flat = ramp.baked.as_ref().map(|m| m.colour(2.5, 2.5, 1.0, 0));
        let slope = ramp.baked.as_ref().map(|m| m.colour(15.5, 15.5, 1.0, 0));
        assert!(flat != slope, "baked: the slope reads");
        for (w, h) in [(640, 480), (320, 220)] {
            let mut f = Frame::new(w, h);
            let cam = Camera {
                x_cm: 32_000,
                y_cm: 32_000,
                heading_mdeg: 30_000,
            };
            render(&mut f, &t, &cam, None, &[]);
            assert!(
                f.count_colours(256) <= 160,
                "{w}x{h}: {}",
                f.count_colours(300)
            );
        }
        t.set_elev_cm(0, 0, 5);
        assert!(!t.is_baked(), "a height change drops the bake");
    }

    /// A terrace's drop toward the north faces away from a north-west sun:
    /// the cells on both sides of the step are shaded, level ground lit.
    #[test]
    fn a_step_down_to_the_north_is_shaded() {
        let mut t = Terrain::new(Map::new(8, 8, 1000));
        for y in 4..8 {
            for x in 0..8 {
                t.set_elev_cm(x, y, -100);
            }
        }
        assert!(!t.lit(3.5, 3.5) && !t.lit(3.5, 4.5), "the step's two cells");
        assert!(t.lit(3.5, 1.5) && t.lit(3.5, 6.5), "level ground");
    }

    #[test]
    fn heights_interpolate_between_cell_centres() {
        let mut t = Terrain::new(Map::new(4, 4, 1000));
        t.set_elev_cm(1, 1, 1000);
        assert_eq!(t.height(1.5, 1.5), 1.0, "a cell centre is its own height");
        assert!(
            (t.height(2.0, 1.5) - 0.5).abs() < 1e-6,
            "half way to a flat neighbour"
        );
        assert_eq!(t.height(3.5, 3.5), 0.0);
        assert_eq!(
            t.elev_cm(-5, 99),
            t.elev_cm(0, 3),
            "outside: the nearest edge"
        );
    }

    /// From the crater's floor the rim stands above the horizon all round;
    /// from the plain outside, looking in, the ground falls away below it.
    #[test]
    fn a_crater_rim_rises_above_the_horizon_from_its_floor() {
        let t = crater();
        let mut f = Frame::new(320, 220);
        let floor = Camera {
            x_cm: 32_000,
            y_cm: 32_000,
            heading_mdeg: 0,
        };
        render(&mut f, &t, &floor, None, &[]);
        assert!(f.count_colours(256) <= 256);
        // Straight ahead, the column's ground reaches above the horizon row.
        let horizon = 110;
        let col = 160;
        let first_ground = (0..220)
            .find(|&y| f.get(col, y) != f.get(col, 0) && y > 2 && f.get(col, y) != HORIZON)
            .unwrap_or(220);
        assert!(
            first_ground < horizon,
            "the rim above the eye: row {first_ground}"
        );
        // Standing on the plain far out, looking at the crater: the ground
        // stays below the horizon.
        let plain = Camera {
            x_cm: 32_000,
            y_cm: 2_000,
            heading_mdeg: 0,
        };
        render(&mut f, &t, &plain, None, &[]);
        let first_ground = (0..220)
            .find(|&y| f.get(col, y) != HORIZON && y >= horizon)
            .unwrap_or(220);
        assert!(first_ground >= horizon, "row {first_ground}");
    }

    /// A tank behind a ridge is hidden by it and bracketed instead; the same
    /// tank on open ground is drawn.
    #[test]
    fn a_ridge_hides_a_tank_and_brackets_it() {
        let mut map = Map::new(64, 64, 1000);
        for y in 0..64 {
            for x in 0..64 {
                map.set(x, y, Ground::Grass);
            }
        }
        let mut t = Terrain::new(map);
        let cam = Camera {
            x_cm: 32_000,
            y_cm: 10_000,
            heading_mdeg: 0,
        };
        let tank_at = Sprite {
            x_cm: 32_000,
            y_cm: 30_000,
            kind: SpriteKind::Red,
            pose: None,
        };
        let reddish = |f: &Frame| {
            (0..f.height()).any(|y| {
                (140..180).any(|x| {
                    let [r, g, b] = f.get(x, y);
                    r > g + 40 && r > b + 40
                })
            })
        };
        let mut open = Frame::new(320, 220);
        render(&mut open, &t, &cam, None, &[tank_at]);
        assert!(reddish(&open), "the tank in the open is drawn");
        for x in 0..64 {
            t.set_elev_cm(x, 20, 1500);
        }
        let mut hidden = Frame::new(320, 220);
        render(&mut hidden, &t, &cam, None, &[tank_at]);
        // The silhouette is hidden (its fogged body colour nowhere), and the
        // contact, which the sim reports, is bracketed in its unfogged one.
        let shades = Shades::new();
        let count = |f: &Frame, px: crate::frame::Rgb| {
            (0..f.width())
                .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
                .filter(|&(x, y)| f.get(x, y) == px)
                .count()
        };
        assert_eq!(
            count(
                &hidden,
                sprite_colour(&shades, SpriteKind::Red, Material::Team, fog_step(20.0))
            ),
            0
        );
        assert!(
            count(
                &hidden,
                sprite_colour(&shades, SpriteKind::Red, Material::Team, 0)
            ) >= 8,
            "the bracket"
        );
    }

    #[test]
    fn walls_stand_and_frames_stay_in_the_palette() {
        let mut map = Map::new(32, 32, 1000);
        for x in 0..32 {
            map.set(x, 20, Ground::Wall);
            map.set(x, 10, Ground::Woods);
        }
        let mut t = Terrain::new(map);
        for y in 0..32 {
            for x in 0..32 {
                t.set_elev_cm(x, y, ((x * 37 + y * 53) % 400) as i32 - 200);
            }
        }
        let cam = Camera {
            x_cm: 16_000,
            y_cm: 2_000,
            heading_mdeg: 10_000,
        };
        for (w, h) in [(640, 480), (320, 220)] {
            let mut f = Frame::new(w, h);
            render(&mut f, &t, &cam, Some(0), &[]);
            assert!(f.count_colours(256) <= 256, "{w}x{h}");
        }
    }
}
