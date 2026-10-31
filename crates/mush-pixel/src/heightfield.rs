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
    Base, Camera, EYE_HEIGHT_CM, FOG_CELLS, Ground, Map, PLANE, SKY_STEPS, Shades, Sprite,
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
const KINDS: [Ground; 8] = [
    Ground::Open,
    Ground::Grass,
    Ground::Water,
    Ground::Road,
    Ground::Wall,
    Ground::Woods,
    Ground::WoodsHeavy,
    Ground::Rock,
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
    horizon: Rgb,
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
            .unwrap_or(self.horizon)
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
            .unwrap_or(self.horizon)
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
        let biome = self.map.biome();
        let horizon = biome.horizon_rgb();
        let fine = self.bake_level(seed, TEXELS);
        let coarse = self.bake_level(seed, 1);
        let mut table = Vec::with_capacity(KINDS.len() * LEVELS);
        for kind in KINDS {
            let rgb = base(kind, true).rgb_for_biome(biome);
            for gain in LEVEL_GAIN {
                let lit = rgb.map(|c| (f32::from(c) * gain).round().min(255.0) as u8);
                let mut row = [[0u8; 3]; BAKED_FOG];
                for (step, px) in row.iter_mut().enumerate() {
                    let t = step as f32 / (BAKED_FOG - 1) as f32;
                    *px = [0, 1, 2].map(|i| {
                        let (a, b) = (
                            f32::from(lit.get(i).copied().unwrap_or(0)),
                            f32::from(horizon.get(i).copied().unwrap_or(0)),
                        );
                        (a + (b - a) * t).round() as u8
                    });
                }
                table.push(row);
            }
        }
        self.baked = Some(ColourMap {
            horizon,
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
        // The line from the eye to the unit's middle, below the level: the
        // chain pick's angle (doc 22 §5).
        let middle = ground + hcm / self.cell / 2.0;
        let depression_deg = (self.eye - middle).det_atan2(ty).to_degrees();
        Some(SpriteAt {
            screenx: self.halfw * (1.0 + tx / ty),
            bottom: self.horizon + (self.eye - ground) * self.hf / ty,
            depth: ty,
            fog_depth: ty * FOG_CELLS / VIEW_CELLS,
            height: hcm / self.cell * self.hf / ty,
            width: wcm / self.cell * self.hf / ty,
            depression_deg,
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
    let shades = Shades::for_biome(terrain.map.biome());
    let depth = draw_ground(frame, terrain, &view, &shades);
    draw_sprites(frame, terrain, sprites, &depth, &shades, &view);
}

/// **The hull-down probe** (doc 22 §6): how many rows of `sprite`'s own body
/// the picture [`render`] draws of `terrain` from `cam` (eye at `eye_cm`)
/// into a `w` by `h` frame shows — its rows above whatever nearer ground,
/// trees or structures cover it, the bracket and the shadow not counted. The
/// hull-down check holds it to the sim's ray: a contact the sim sees over a
/// crest shows at least one row, one it does not shows none.
#[must_use]
pub fn visible_rows(
    terrain: &Terrain,
    cam: &Camera,
    eye_cm: Option<i64>,
    (w, h): (usize, usize),
    sprite: &Sprite,
) -> usize {
    if w == 0 || h == 0 {
        return 0;
    }
    let mut frame = Frame::new(w, h);
    let view = View::new(terrain, cam, eye_cm, w, h);
    let shades = Shades::for_biome(terrain.map.biome());
    let depth = draw_ground(&mut frame, terrain, &view, &shades);
    let Some(at) = view.place(terrain, sprite) else {
        return 0;
    };
    let depth_at = |x: usize, y: usize| depth.get(y * w + x).copied().unwrap_or(f32::INFINITY);
    crate::raycast::draw_sprite(&mut frame, &shades, at, &depth_at).rows
}

/// The ground, the sky, the trees and the structures, far to near; the
/// distance each pixel was drawn at, for the sprites (infinite where only
/// sky is).
fn draw_ground(frame: &mut Frame, terrain: &Terrain, view: &View, shades: &Shades) -> Vec<f32> {
    let (w, h) = (frame.width(), frame.height());
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
    } = *view;
    let biome = terrain.map.biome();
    let horizon_rgb = biome.horizon_rgb();

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
                horizon_rgb
            };
            frame.put(i, row, px);
        }
    }

    draw_trees(frame, terrain, &mut depth, shades, view);
    draw_outpost_structures(frame, terrain, &mut depth, shades, view);
    depth
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

        // 2. 3-tiered conical conifer canopy
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

/// Baked 16x16 height-and-shade grids rasterized from Manufactura K4's
/// `Top-Down Sci-Fi` FBX models (`Generator_01.fbx`, `Generator_02.fbx`,
/// `Generator_03.fbx`, `Cylinder_01.fbx`).
/// Low 5 bits (`0..=31`): normalized height; high 3 bits (`0..=7`): NW sun shade.
pub(crate) const K4_SCIFI_PROPS: [[[u8; 16]; 16]; 4] = [
    // Generator_01.fbx (3.12m x 3.81m x 2.73m)
    [
        [
            0x00, 0x00, 0x1A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0xFA,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0x9A, 0xBA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9A, 0x9A, 0x7A, 0x7E, 0x9E, 0x9E, 0x9E, 0x9A, 0x9A, 0xBA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9A, 0x7A, 0x7E, 0x9F, 0xBF, 0xBF, 0xBF, 0xBE, 0xBA, 0xBA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9A, 0x7E, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xDA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9A, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xDA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x41, 0x06, 0x1A, 0x9A, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xDA, 0xF6,
            0xC6, 0x81,
        ],
        [
            0x41, 0x06, 0x1A, 0x9A, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xDA, 0xF6,
            0xE6, 0x81,
        ],
        [
            0x00, 0x00, 0x9A, 0x9A, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDE, 0xDA, 0xF6,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x1A, 0xBE, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDE, 0xFA, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x9A, 0x9A, 0xBE, 0xBF, 0xBF, 0xBF, 0xBF, 0xDE, 0xFA, 0xF6, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x9A, 0xBA, 0xBE, 0xDE, 0xDE, 0xDE, 0xDA, 0xFA, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0xFA, 0xBA, 0xDA, 0xFA, 0xFA, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0xF6, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE6, 0xE6, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0x81, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
    ],
    // Generator_02.fbx (3.47m x 4.60m x 2.64m)
    [
        [
            0x00, 0x00, 0x1C, 0x9D, 0xBD, 0xBD, 0xBD, 0xBA, 0x9A, 0x9D, 0xBD, 0xBD, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0xBD, 0xBD, 0xBD, 0xBA, 0x9A, 0x9D, 0xBD, 0xBD, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0xBD, 0xBD, 0xBD, 0x9A, 0x7A, 0x9D, 0xBD, 0xBD, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0xBD, 0x9D, 0x9E, 0x9F, 0x9F, 0xBE, 0xBD, 0xBD, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0x9D, 0x9E, 0x9F, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0x9E, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x46, 0x07, 0x1C, 0x9D, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xFC,
            0xC7, 0x46,
        ],
        [
            0x86, 0x07, 0x1C, 0x9D, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xFC,
            0xE7, 0xA6,
        ],
        [
            0x00, 0x00, 0x1C, 0x9D, 0x9E, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xBD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x9C, 0xFD, 0xBD, 0xBE, 0xBF, 0xBF, 0xBF, 0xBF, 0xBE, 0xDD, 0xFD, 0xFC,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x1A, 0xBE, 0xBE, 0xBF, 0xBF, 0xDE, 0xDE, 0xFA, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x9A, 0xBA, 0xBA, 0xBE, 0xDE, 0xDA, 0xDA, 0xF7, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0xFA, 0xBA, 0xDA, 0xFA, 0xF7, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x27, 0x9A, 0xF7, 0xE7, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xEB, 0xCB, 0xAB, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
    ],
    // Generator_03.fbx (3.04m x 4.35m x 2.89m)
    [
        [
            0x62, 0x62, 0x62, 0x02, 0x02, 0x02, 0x62, 0x62, 0x62, 0x62, 0x02, 0x02, 0x02, 0x02,
            0x62, 0x62,
        ],
        [
            0x62, 0x62, 0x02, 0x11, 0x11, 0x51, 0x42, 0x02, 0x02, 0x02, 0x11, 0x11, 0x11, 0x79,
            0xE2, 0x62,
        ],
        [
            0x62, 0x02, 0x19, 0x79, 0x79, 0x59, 0x19, 0x19, 0x19, 0x19, 0x59, 0x79, 0x79, 0xF9,
            0xE2, 0x62,
        ],
        [
            0x62, 0x02, 0x19, 0x99, 0x79, 0x5C, 0x7F, 0x9F, 0x9F, 0x9F, 0xBC, 0x99, 0xB9, 0xF5,
            0xE2, 0x62,
        ],
        [
            0x62, 0x02, 0x19, 0x79, 0x5E, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0x9F, 0xBE, 0xD9, 0xF5,
            0xE2, 0x62,
        ],
        [
            0x02, 0x02, 0x19, 0x5C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBC, 0xF5,
            0x82, 0x02,
        ],
        [
            0x31, 0x13, 0x59, 0x7F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xF5,
            0x33, 0x31,
        ],
        [
            0x91, 0x73, 0x39, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xFF, 0xB5,
            0xB3, 0x91,
        ],
        [
            0x91, 0x53, 0x5C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xFC,
            0xD3, 0x91,
        ],
        [
            0x91, 0x73, 0x79, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xFF, 0xF5,
            0xB3, 0x91,
        ],
        [
            0xF1, 0xD3, 0xF9, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xFF, 0xF5,
            0xF3, 0xF1,
        ],
        [
            0x00, 0xC2, 0x42, 0x1C, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xFC, 0xE2,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x02, 0x39, 0xBC, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xFC, 0xF1, 0xE2,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x02, 0x91, 0x99, 0xDC, 0xBE, 0xDF, 0xDF, 0xFE, 0xFC, 0xF1, 0xF1, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x62, 0xD1, 0xF1, 0xF9, 0xF9, 0xF9, 0xF2, 0xF1, 0xF1, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0xC2, 0xE2, 0xE2, 0xE2, 0xE2, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
    ],
    // Cylinder_01.fbx (0.75m x 1.41m x 0.75m)
    [
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x1A, 0x1B, 0x1B, 0x1B, 0x1B, 0x1B, 0x1B, 0x9A, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x1A, 0x9B, 0x9B, 0x9C, 0x9C, 0x9C, 0x9C, 0x9B, 0x9B, 0x9A, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x1A, 0x9B, 0x7C, 0x7F, 0x9F, 0x9F, 0x9F, 0xBF, 0xBC, 0xBC, 0xBB, 0x9A,
            0x00, 0x00,
        ],
        [
            0x00, 0x1A, 0x9B, 0x7C, 0x7F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBC, 0xBC, 0xBB,
            0xBA, 0x00,
        ],
        [
            0x00, 0x1B, 0x9B, 0x7F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBC, 0xBB,
            0xFB, 0x00,
        ],
        [
            0x00, 0x1B, 0x9C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBC, 0x9C,
            0xFB, 0x00,
        ],
        [
            0x00, 0x1B, 0x9C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0x9F, 0xBF,
            0xFB, 0x00,
        ],
        [
            0x00, 0x1B, 0x9C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF,
            0xFB, 0x00,
        ],
        [
            0x00, 0x1B, 0x9C, 0x9F, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDC, 0xDC,
            0xFB, 0x00,
        ],
        [
            0x00, 0x1B, 0x9B, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xBC, 0xBB,
            0xFB, 0x00,
        ],
        [
            0x00, 0x9A, 0x9B, 0xBC, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xDC, 0xBC, 0xBB,
            0xFA, 0x00,
        ],
        [
            0x00, 0x00, 0x9A, 0xBB, 0xBC, 0xBF, 0xBF, 0xBF, 0xBF, 0xDF, 0xDC, 0xBC, 0xBB, 0xFA,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x9A, 0xBB, 0xBB, 0xBC, 0xBC, 0xBC, 0xBC, 0xBB, 0xBB, 0xFA, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0xBA, 0xFB, 0xFB, 0xFB, 0xFB, 0xFB, 0xFB, 0xFA, 0x00, 0x00,
            0x00, 0x00,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
    ],
];

/// Deterministic selection of a baked Manufactura K4 sci-fi rooftop structure
/// (`0..=2`: `Generator_01..03`, `3`: pressurized `Cylinder_01` silo) and its
/// height in metres on interior `Ground::Wall` bunker/outpost cells.
pub(crate) fn scifi_prop_for_cell(map: &Map, cx: i64, cy: i64) -> Option<(usize, f32)> {
    if map.get(cx, cy) != Ground::Wall {
        return None;
    }
    // Require all 4 cardinal neighbours AND all 4 diagonal neighbours to be
    // `Ground::Wall` so only true interior compound rooftops (>= 5x5 bunkers/outposts)
    // host tall reactor domes/silos, leaving narrow walls and small 4x3 sheds untouched.
    for dy in -1..=1 {
        for dx in -1..=1 {
            if map.get(cx + dx, cy + dy) != Ground::Wall {
                return None;
            }
        }
    }
    let h = ((cx as u32).wrapping_mul(0x9E37_79B9))
        ^ ((cy as u32).wrapping_mul(0x85EB_CA6B))
        ^ 0x27D4_EB2D;
    if !h.is_multiple_of(3) {
        return None;
    }
    let idx = ((h >> 8) as usize) & 3;
    let height_m = match idx {
        0 => 2.8,
        1 => 3.2,
        2 => 3.0,
        _ => 2.4,
    };
    Some((idx, height_m))
}

#[derive(Clone, Copy)]
struct PlacedOutpostProp {
    screenx: f32,
    bottom: f32,
    ty: f32,
    w_px: i32,
    h_px: i32,
    prop_idx: usize,
}

/// Render 3D Manufactura K4 sci-fi rooftop structures (`Generator_01..03`,
/// `Cylinder_01`) on interior `Ground::Wall` outpost rooftops in First-Person,
/// reusing the baked `Ground::Wall` / `Ground::Road` palette entries so zero
/// extra colours are added to the frame.
fn draw_outpost_structures(
    frame: &mut Frame,
    terrain: &Terrain,
    depth: &mut [f32],
    shades: &Shades,
    view: &View,
) {
    let (w, h) = (frame.width(), frame.height());
    let (mw, mh) = (terrain.map.width() as i64, terrain.map.height() as i64);
    let r_cells = TREE_VIEW_CELLS.ceil() as i64;
    let cx0 = (view.posx.floor() as i64 - r_cells).max(1);
    let cx1 = (view.posx.floor() as i64 + r_cells).min(mw - 2);
    let cy0 = (view.posy.floor() as i64 - r_cells).max(1);
    let cy1 = (view.posy.floor() as i64 + r_cells).min(mh - 2);
    if cx0 > cx1 || cy0 > cy1 {
        return;
    }

    let cell_m = (view.cell * 0.01).max(1.0);
    let wall_h_cells = Ground::Wall.height_cm() as f32 / view.cell.max(1.0);
    let mut placed = Vec::new();

    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            let Some((prop_idx, prop_h_m)) = scifi_prop_for_cell(&terrain.map, cx, cy) else {
                continue;
            };
            let wx = cx as f32 + 0.5;
            let wy = cy as f32 + 0.5;
            let relx = wx - view.posx;
            let rely = wy - view.posy;
            let ty = view.invdet * (-view.planey * relx + view.planex * rely);
            if ty <= 0.5 || ty > TREE_VIEW_CELLS {
                continue;
            }
            let tx = view.invdet * (view.diry * relx - view.dirx * rely);
            if tx.abs() > ty * 1.15 + 0.8 {
                continue;
            }
            let prop_h_cells = prop_h_m / cell_m;
            let prop_w_cells = 0.62;
            let h_px = (prop_h_cells * view.hf / ty).round() as i32;
            let w_px = (prop_w_cells * view.hf / ty).round() as i32;
            if h_px < 2 || w_px < 2 {
                continue;
            }
            let roof_z = terrain.height(wx, wy) + wall_h_cells;
            let screenx = view.halfw * (1.0 + tx / ty);
            let bottom = view.horizon + (view.eye - roof_z) * view.hf / ty;
            placed.push(PlacedOutpostProp {
                screenx,
                bottom,
                ty,
                w_px: w_px.min(w as i32),
                h_px: h_px.min(h as i32),
                prop_idx,
            });
        }
    }

    placed.sort_unstable_by(|a, b| a.ty.total_cmp(&b.ty));

    for prop in placed {
        let bx = prop.screenx.round() as i32;
        let by = prop.bottom.round() as i32;
        let half_w = (prop.w_px / 2).max(1);
        if bx + half_w < 0 || bx - half_w >= w as i32 || by < 0 || by - prop.h_px >= h as i32 {
            continue;
        }
        let step = fog_step(prop.ty * FOG_CELLS / VIEW_CELLS);
        let (col_dark, col_mid, col_lit, col_hi) = match &terrain.baked {
            Some(map) => (
                map.entry(4, 0, step),
                map.entry(4, 2, step),
                map.entry(4, 3, step),
                map.entry(4, 4, step),
            ),
            None => (
                shades.shade(Base::Outline, step),
                shades.shade(Base::WallDark, step),
                shades.shade(Base::WallLit, step),
                shades.shade(Base::WallLit, step),
            ),
        };
        let Some(grid) = K4_SCIFI_PROPS.get(prop.prop_idx) else {
            continue;
        };

        for dx in -half_w..=half_w {
            let px = bx + dx;
            if px < 0 || px >= w as i32 {
                continue;
            }
            let px_u = px as usize;
            let gx = (((dx + half_w) * 15) / (2 * half_w).max(1)).clamp(0, 15) as usize;
            // Sample maximum height column and average shade across the 16-row FBX profile
            let mut max_hq: u8 = 0;
            let mut shade_q: u8 = 4;
            for row in grid {
                let cell = row.get(gx).copied().unwrap_or(0);
                let hq = cell & 0x1F;
                if hq > max_hq {
                    max_hq = hq;
                    shade_q = cell >> 5;
                }
            }
            if max_hq == 0 {
                continue;
            }
            let col_h = ((i32::from(max_hq) * prop.h_px) / 31).max(1);
            for dy in 0..=col_h {
                let py = by - dy;
                if py < 0 || py >= h as i32 {
                    continue;
                }
                let py_u = py as usize;
                let idx = py_u * w + px_u;
                if prop.ty < depth.get(idx).copied().unwrap_or(f32::INFINITY) {
                    let is_rib = (dy & 3) == 0 && prop.prop_idx < 3;
                    let col = if dy >= col_h - 1 {
                        col_hi
                    } else if is_rib || shade_q <= 2 {
                        col_dark
                    } else if shade_q <= 4 {
                        col_mid
                    } else {
                        col_lit
                    };
                    frame.put(px_u, py_u, col);
                    if let Some(d) = depth.get_mut(idx) {
                        *d = prop.ty;
                    }
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
        let _ = crate::raycast::draw_sprite(frame, shades, at, &depth_at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Material;
    use crate::raycast::{HORIZON, SpriteKind, sprite_colour};

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
            kind: SpriteKind::Hostile,
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
                sprite_colour(&shades, SpriteKind::Hostile, Material::Team, fog_step(20.0))
            ),
            0
        );
        assert!(
            count(
                &hidden,
                sprite_colour(&shades, SpriteKind::Hostile, Material::Team, 0)
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
