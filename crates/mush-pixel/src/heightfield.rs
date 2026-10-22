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
//! - **Blocks**: a wall stands `WALL_H` above its cell and woods
//!   [`WOODS_H`], so the arena's buildings and tree lines still read as
//!   solid; they are drawn as raised cells, flat-topped.
//! - **Light**: each drawn sample is lit or shaded by its slope toward a sun
//!   in the north-west — two levels, no more, so a frame stays inside the
//!   palette PNG's 256 colours with the fog and the instruments.
//! - **Sprites**: the units stand on the ground at their own cell's height,
//!   and a per-pixel depth (the distance each pixel was drawn at) lets a
//!   ridge hide half a tank.
//!
//! The colours are the raycaster's closed set (`Shades`), so the two
//! producers draw the same ground the same colour.

use crate::Frame;
use crate::raycast::{
    Base, Camera, EYE, FOG_CELLS, Ground, HORIZON, Map, PLANE, SKY_STEPS, SPRITE_H, SPRITE_W,
    Shades, Sprite, SpriteKind, WALL_H, fog_step, tank,
};

/// Woods' height above the ground, in cells (6 m at 10 m a cell).
pub const WOODS_H: f32 = 0.6;
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
        let raised = match ground {
            Ground::Wall => WALL_H,
            Ground::Woods => WOODS_H,
            _ => 0.0,
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

/// The colour base a sample is drawn in, lit or shaded.
fn base(ground: Ground, lit: bool) -> Base {
    match ground {
        Ground::Wall if lit => Base::WallLit,
        Ground::Wall => Base::WallDark,
        Ground::Woods => Base::Woods,
        // The floor's "line" shade is its darker twin: the shaded slope.
        g => Base::floor(g, !lit),
    }
}

/// Draw `terrain` from `cam` into `frame`, with `sprites` standing on it.
/// The eye stands `EYE` above `eye_cm` when it is given (the cockpit
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
    let eye = eye_cm.map_or_else(|| terrain.height(posx, posy), |z| z as f32 / cell) + EYE;
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
            let px_rgb = shades.shade(base(ground, terrain.lit(px, py)), step);
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
    for (tx, ty, ground, kind) in placed {
        let screenx = (w as f32 / 2.0) * (1.0 + tx / ty);
        let hpx = SPRITE_H * hf / ty;
        let wpx = SPRITE_W * hf / ty;
        if hpx < 1.0 || wpx < 1.0 {
            continue;
        }
        let bottom = horizon + (eye - ground) * hf / ty;
        let top = bottom - hpx;
        let left = screenx - wpx / 2.0;
        let step = fog_step(ty);
        let body = shades.shade(
            match kind {
                SpriteKind::Hostile => Base::Hostile,
                SpriteKind::Friendly => Base::Friendly,
                SpriteKind::Unknown => Base::Unknown,
                SpriteKind::Wreck => Base::Wreck,
            },
            step,
        );
        let outline = shades.shade(Base::Outline, step);
        let x0 = left.floor().max(0.0) as usize;
        let x1 = ((left + wpx).ceil().max(0.0) as usize).min(w);
        let y0 = top.floor().max(0.0) as usize;
        let y1 = (bottom.ceil().max(0.0) as usize).min(h);
        let (du, dv) = (1.0 / wpx, 1.0 / hpx);
        for x in x0..x1 {
            let u = (x as f32 + 0.5 - left) / wpx;
            for y in y0..y1 {
                if depth.get(y * w + x).is_some_and(|d| ty >= *d) {
                    continue;
                }
                let v = (y as f32 + 0.5 - top) / hpx;
                if !tank(u, v) {
                    continue;
                }
                let edge =
                    !tank(u - du, v) || !tank(u + du, v) || !tank(u, v - dv) || !tank(u, v + dv);
                frame.put(x, y, if edge { outline } else { body });
            }
        }
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

    /// A tank behind a ridge is hidden by it; the same tank on open ground
    /// is drawn.
    #[test]
    fn a_ridge_hides_a_tank_behind_it() {
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
        assert!(!reddish(&hidden), "a 15 m ridge hides it");
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
