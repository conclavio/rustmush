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
//!   which is then bracketed where it stands (the raycaster's rule).
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
use crate::raycast::{
    Base, Camera, EYE_HEIGHT_CM, FOG_CELLS, Ground, HORIZON, Map, PLANE, SKY_STEPS,
    SPRITE_HEIGHT_CM, SPRITE_WIDTH_CM, Shades, Sprite, SpriteAt, SpriteKind, draw_sprite, fog_step,
};

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
    /// the ground, or a block standing on it.
    fn surface(&self, px: f32, py: f32) -> (Ground, f32) {
        let ground = self.map.get(px.floor() as i64, py.floor() as i64);
        let raised = ground.height_cm() as f32 / self.map.cell_cm() as f32;
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
/// one-texel-a-cell map: the fine texels would alias there, a light level's
/// edge shimmering into dashes across neighbouring columns.
const FINE_DETAIL_CELLS: f32 = 8.0;
/// Light levels a baked texel is quantized to.
const LEVELS: usize = 4;
/// Fog steps a baked texel is drawn in (the sprites and blocks of the
/// unbaked path keep the raycaster's twelve).
const BAKED_FOG: usize = 6;
/// The light levels' brightness against the base colour.
const LEVEL_GAIN: [f32; LEVELS] = [0.60, 0.80, 1.00, 1.15];
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
    /// light from the surface at its centre (see the module docs).
    fn bake_level(&self, seed: u64, per_cell: usize) -> BakedLevel {
        let (cw, ch) = (self.map.width(), self.map.height());
        let (w, h) = (cw * per_cell, ch * per_cell);
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
                let dhdx =
                    (self.height(px + d, py) - self.height(px - d, py)) / (2.0 * d) * RELIEF_GAIN;
                let dhdy =
                    (self.height(px, py + d) - self.height(px, py - d)) / (2.0 * d) * RELIEF_GAIN;
                let norm = (dhdx * dhdx + dhdy * dhdy + 1.0).sqrt();
                let lambert = ((-dhdx * lx - dhdy * ly + lz) / norm).max(0.0);
                let steep = (dhdx * dhdx + dhdy * dhdy).sqrt();
                let cell = (py.floor().max(0.0) as usize).min(ch.saturating_sub(1)) * cw
                    + (px.floor().max(0.0) as usize).min(cw.saturating_sub(1));
                let hollow = mean.get(cell).map_or(0.0, |m| self.height(px, py) - m);
                let mottle =
                    0.6 * value_noise(seed, px, py, 2.0) + 0.4 * value_noise(seed ^ 1, px, py, 5.0);
                let light = 0.30 + 0.75 * lambert - 0.25 * steep.min(1.0)
                    + (hollow * RELIEF_GAIN).clamp(-0.15, 0.08)
                    + (mottle - 0.5) * 0.14;
                let level = if light < 0.48 {
                    0
                } else if light < 0.66 {
                    1
                } else if light < 0.88 {
                    2
                } else {
                    3
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

/// Draw `terrain` from `cam` into `frame`, with `sprites` standing on it.
/// The eye stands 1.5 m (the sim's sensor height) above `eye_cm` when it is given (the cockpit
/// frame's `z_cm`, the sim's own elevation for the unit), else above the
/// ground under the camera.
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
    let cell = terrain.map.cell_cm() as f32;
    let posx = cam.x_cm as f32 / cell;
    let posy = cam.y_cm as f32 / cell;
    let heading = (cam.heading_mdeg % 360_000) as f32 / 1000.0;
    let (dirx, diry) = heading.to_radians().sin_cos();
    let (planex, planey) = (diry * PLANE, -dirx * PLANE);
    let hf = h as f32;
    let horizon = (h / 2) as f32;
    let eye = eye_cm.map_or_else(|| terrain.height(posx, posy), |z| z as f32 / cell)
        + EYE_HEIGHT_CM as f32 / cell;
    let shades = Shades::new();

    // The lowest row each column has drawn down to (exclusive): ground
    // above it is hidden by nearer ground below it.
    let mut ybuf = vec![h; w];
    // The distance each pixel was drawn at, for the sprites.
    let mut depth = vec![f32::INFINITY; w * h];
    let wf = w as f32;

    let mut z = NEAR;
    let mut dz = FIRST_STEP;
    while z < FOG_CELLS {
        let (lx, ly) = (posx + (dirx - planex) * z, posy + (diry - planey) * z);
        let (sx, sy) = (2.0 * planex * z / wf, 2.0 * planey * z / wf);
        let step = fog_step(z);
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

    draw_sprites(frame, terrain, sprites, &depth, &shades, eye, cam);
}

/// The sprites, far to near, each standing on the ground at its own point
/// and hidden where nearer ground was drawn.
fn draw_sprites(
    frame: &mut Frame,
    terrain: &Terrain,
    sprites: &[Sprite],
    depth: &[f32],
    shades: &Shades,
    eye: f32,
    cam: &Camera,
) {
    let (w, h) = (frame.width(), frame.height());
    let cell = terrain.map.cell_cm() as f32;
    let posx = cam.x_cm as f32 / cell;
    let posy = cam.y_cm as f32 / cell;
    let heading = (cam.heading_mdeg % 360_000) as f32 / 1000.0;
    let (dirx, diry) = heading.to_radians().sin_cos();
    let (planex, planey) = (diry * PLANE, -dirx * PLANE);
    let hf = h as f32;
    let horizon = (h / 2) as f32;
    let invdet = 1.0 / (planex * diry - dirx * planey);
    let mut placed: Vec<(f32, f32, f32, SpriteKind)> = sprites
        .iter()
        .map(|s| {
            let (wx, wy) = (s.x_cm as f32 / cell, s.y_cm as f32 / cell);
            let (relx, rely) = (wx - posx, wy - posy);
            let tx = invdet * (diry * relx - dirx * rely);
            let ty = invdet * (-planey * relx + planex * rely);
            (tx, ty, terrain.height(wx, wy), s.kind)
        })
        .filter(|(_, ty, _, _)| *ty > 0.05)
        .collect();
    placed.sort_by(|a, b| b.1.total_cmp(&a.1));
    let depth_at = |x: usize, y: usize| depth.get(y * w + x).copied().unwrap_or(f32::INFINITY);
    for (tx, ty, ground, kind) in placed {
        draw_sprite(
            frame,
            shades,
            SpriteAt {
                screenx: (w as f32 / 2.0) * (1.0 + tx / ty),
                bottom: horizon + (eye - ground) * hf / ty,
                depth: ty,
                height: SPRITE_HEIGHT_CM as f32 / cell * hf / ty,
                width: SPRITE_WIDTH_CM as f32 / cell * hf / ty,
                kind,
            },
            &depth_at,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                let d = (((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt()) / 8.0;
                let cm = if d < 1.0 {
                    -1200.0 + 1800.0 * d * d
                } else {
                    600.0 * (-(d - 1.0) * 2.0).exp()
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
            count(&hidden, shades.shade(Base::Hostile, fog_step(20.0))),
            0
        );
        assert!(
            count(&hidden, shades.shade(Base::Hostile, 0)) >= 8,
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
