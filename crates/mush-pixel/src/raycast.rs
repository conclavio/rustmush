//! A Wolfenstein-style software raycaster over the arena's cell grid.
//!
//! The arena is a 2D grid of cells (`cell_cm` on a side; 10 m today), each
//! a [`Ground`]. Walls and woods block the eye; grass, water, road and open
//! ground are floor the eye sees past. The viewer stands at a point in
//! centimetres with a heading in millidegrees clockwise from north, exactly
//! the `x_cm`/`y_cm`/`heading_mdeg` the `arena.cockpit` frame carries, and
//! the other units are billboard sprites placed by bearing and range.
//!
//! One ray per screen column (DDA through the grid) finds the first
//! blocking cell and its perpendicular distance; above the wall slice the
//! column is sky, below it the floor is cast per pixel back onto the grid so
//! the ground is coloured by the cell it is (with a faint cell grid for
//! parallax). Sprites are drawn last, far to near, against a per-column
//! depth buffer so a wall in front of a unit hides it.
//!
//! **Colours are a closed set by construction** — [`Base`] times
//! [`FOG_STEPS`] plus the sky's [`SKY_STEPS`] — so a rendered frame never
//! has more than 256 colours and the PNG encoder always takes the palette
//! path. A wrong constant here shows up as the truecolor fallback, which a
//! test pins against.

use crate::Frame;
use crate::frame::Rgb;

/// What a cell is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ground {
    /// Bare ground; also what an unknown legend word maps to.
    #[default]
    Open,
    /// Grass.
    Grass,
    /// Water (floor; a tank sees across it).
    Water,
    /// Road.
    Road,
    /// Woods: block the eye, drawn as a green wall.
    Woods,
    /// A wall, a building, rock: blocks the eye.
    Wall,
}

impl Ground {
    /// The `arena.map` legend word for a glyph, mapped to a ground.
    #[must_use]
    pub fn from_legend_name(name: &str) -> Ground {
        match name {
            "grass" => Ground::Grass,
            "water" => Ground::Water,
            "road" => Ground::Road,
            "wall" | "rock" | "building" | "cliff" => Ground::Wall,
            n if n.starts_with("woods") || n.starts_with("forest") => Ground::Woods,
            _ => Ground::Open,
        }
    }

    /// Whether a ray stops here.
    #[must_use]
    pub fn blocks(self) -> bool {
        matches!(self, Ground::Wall | Ground::Woods)
    }
}

/// The arena grid. Cell `(x, y)` covers `[x * cell_cm, (x + 1) * cell_cm)`
/// east and the same north: `y = 0` is the SOUTH row, as `arena.map` rows
/// and the cockpit map count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    width: usize,
    height: usize,
    cell_cm: u32,
    cells: Vec<Ground>,
}

impl Map {
    /// An all-open map.
    #[must_use]
    pub fn new(width: usize, height: usize, cell_cm: u32) -> Map {
        Map {
            width,
            height,
            cell_cm: cell_cm.max(1),
            cells: vec![Ground::Open; width * height],
        }
    }

    /// From glyph rows (south row first) and a legend.
    pub fn from_glyph_rows(rows: &[Vec<char>], legend: &dyn Fn(char) -> Ground, cell_cm: u32) -> Map {
        let height = rows.len();
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut map = Map::new(width, height, cell_cm);
        for (y, row) in rows.iter().enumerate() {
            for (x, &g) in row.iter().enumerate() {
                map.set(x, y, legend(g));
            }
        }
        map
    }

    /// Set one cell; out of range is ignored.
    pub fn set(&mut self, x: usize, y: usize, ground: Ground) {
        if x < self.width
            && y < self.height
            && let Some(cell) = self.cells.get_mut(y * self.width + x)
        {
            *cell = ground;
        }
    }

    /// Read one cell; outside the map is a wall, so no ray leaves it.
    #[must_use]
    pub fn get(&self, x: i64, y: i64) -> Ground {
        if x < 0 || y < 0 {
            return Ground::Wall;
        }
        let (x, y) = (x as usize, y as usize);
        if x < self.width && y < self.height {
            self.cells
                .get(y * self.width + x)
                .copied()
                .unwrap_or(Ground::Wall)
        } else {
            Ground::Wall
        }
    }

    /// Cells east.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Cells north.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Centimetres a cell.
    #[must_use]
    pub fn cell_cm(&self) -> u32 {
        self.cell_cm
    }
}

/// The eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camera {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Clockwise from north, millidegrees.
    pub heading_mdeg: u32,
}

/// What a sprite is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteKind {
    /// A hostile contact.
    Hostile,
    /// A friendly contact.
    Friendly,
    /// An unclassified contact.
    Unknown,
    /// A wreck.
    Wreck,
}

/// A unit on the ground, drawn as a tank silhouette facing the eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sprite {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Drawn as.
    pub kind: SpriteKind,
}

impl Sprite {
    /// A sprite `range_m` from the eye on the absolute `bearing_deg`
    /// (clockwise from north), which is how the cockpit frame places a
    /// contact.
    #[must_use]
    pub fn at_bearing(cam: &Camera, bearing_deg: f64, range_m: f64, kind: SpriteKind) -> Sprite {
        let b = bearing_deg.to_radians();
        let range_cm = range_m * 100.0;
        Sprite {
            x_cm: cam.x_cm + (range_cm * b.sin()).round() as i64,
            y_cm: cam.y_cm + (range_cm * b.cos()).round() as i64,
            kind,
        }
    }
}

/// The closed set of things a pixel can be, before fog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Open,
    Grass,
    Water,
    Road,
    OpenLine,
    GrassLine,
    WaterLine,
    RoadLine,
    WallLit,
    WallDark,
    Woods,
    Hostile,
    Friendly,
    Unknown,
    Wreck,
    Outline,
}

impl Base {
    fn rgb(self) -> Rgb {
        match self {
            Base::Open => [112, 96, 70],
            Base::Grass => [78, 120, 52],
            Base::Water => [44, 78, 130],
            Base::Road => [120, 112, 100],
            Base::OpenLine => [88, 74, 52],
            Base::GrassLine => [58, 92, 38],
            Base::WaterLine => [34, 62, 106],
            Base::RoadLine => [96, 90, 80],
            Base::WallLit => [150, 150, 158],
            Base::WallDark => [104, 104, 112],
            Base::Woods => [40, 84, 44],
            Base::Hostile => [178, 54, 48],
            Base::Friendly => [64, 124, 204],
            Base::Unknown => [168, 160, 96],
            Base::Wreck => [72, 62, 52],
            Base::Outline => [18, 18, 22],
        }
    }

    fn floor(ground: Ground, line: bool) -> Base {
        match (ground, line) {
            (Ground::Grass, false) => Base::Grass,
            (Ground::Grass, true) => Base::GrassLine,
            (Ground::Water, false) => Base::Water,
            (Ground::Water, true) => Base::WaterLine,
            (Ground::Road, false) => Base::Road,
            (Ground::Road, true) => Base::RoadLine,
            (_, false) => Base::Open,
            (_, true) => Base::OpenLine,
        }
    }
}

/// Fog bands between a thing and the horizon colour.
pub const FOG_STEPS: usize = 12;
/// Sky gradient bands.
pub const SKY_STEPS: usize = 16;
/// Where fog is total, in cells (600 m at 10 m a cell: the cockpit's
/// `sight_m`).
const FOG_CELLS: f32 = 60.0;
/// The colour at the horizon, what fog fades to.
const HORIZON: Rgb = [150, 165, 185];
/// The colour at the top of the sky.
const ZENITH: Rgb = [24, 36, 72];
/// The eye's height above the ground, in cells (2.5 m).
const EYE: f32 = 0.25;
/// A wall's height, in cells (8 m).
const WALL_H: f32 = 0.8;
/// A sprite's height and width, in cells (3.5 m tall, 5 m wide).
const SPRITE_H: f32 = 0.35;
const SPRITE_W: f32 = 0.5;
/// Half the field of view as the camera plane's length (66 degrees).
const PLANE: f32 = 0.66;
/// How far a ray walks before giving up (cells).
const MAX_STEPS: usize = 160;
/// How wide the floor's cell lines are, as a fraction of a cell.
const LINE: f32 = 0.07;

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

fn fog_step(dist: f32) -> usize {
    ((dist / FOG_CELLS) * FOG_STEPS as f32)
        .floor()
        .clamp(0.0, (FOG_STEPS - 1) as f32) as usize
}

fn shade(base: Base, step: usize) -> Rgb {
    lerp(base.rgb(), HORIZON, step as f32 / (FOG_STEPS - 1) as f32)
}

fn sky(step: usize) -> Rgb {
    lerp(ZENITH, HORIZON, step as f32 / (SKY_STEPS - 1) as f32)
}

/// Draw the scene into `frame`.
pub fn render(frame: &mut Frame, map: &Map, cam: &Camera, sprites: &[Sprite]) {
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return;
    }
    let cell = map.cell_cm() as f32;
    let posx = cam.x_cm as f32 / cell;
    let posy = cam.y_cm as f32 / cell;
    let heading = (cam.heading_mdeg % 360_000) as f32 / 1000.0;
    let (dirx, diry) = heading.to_radians().sin_cos();
    let (planex, planey) = (diry * PLANE, -dirx * PLANE);
    let hf = h as f32;
    let horizon = h / 2;
    let mut zbuf = vec![f32::INFINITY; w];

    for (x, z) in zbuf.iter_mut().enumerate() {
        let camx = 2.0 * x as f32 / w as f32 - 1.0;
        let rdx = dirx + planex * camx;
        let rdy = diry + planey * camx;
        let (perp, side) = cast(map, posx, posy, rdx, rdy);
        *z = perp;

        let (top, bottom) = if perp.is_finite() {
            let top = horizon as f32 - (WALL_H - EYE) * hf / perp;
            let bottom = horizon as f32 + EYE * hf / perp;
            (top.max(0.0) as usize, (bottom.min(hf)) as usize)
        } else {
            (horizon, horizon)
        };

        for y in 0..top.min(horizon) {
            frame.put(x, y, sky(y * SKY_STEPS / horizon.max(1)));
        }
        if perp.is_finite() {
            let hit = {
                let dist = perp;
                let hx = (posx + rdx * dist).floor() as i64;
                let hy = (posy + rdy * dist).floor() as i64;
                map.get(hx, hy)
            };
            let base = match (hit, side) {
                (Ground::Woods, _) => Base::Woods,
                (_, 0) => Base::WallLit,
                _ => Base::WallDark,
            };
            let px = shade(base, fog_step(perp));
            for y in top..bottom.max(top) {
                frame.put(x, y, px);
            }
        }
        for y in bottom.max(horizon + 1)..h {
            let dist = EYE * hf / (y - horizon) as f32;
            let fx = posx + rdx * dist;
            let fy = posy + rdy * dist;
            let ground = map.get(fx.floor() as i64, fy.floor() as i64);
            let line = fx.fract().abs() < LINE || fy.fract().abs() < LINE;
            frame.put(x, y, shade(Base::floor(ground, line), fog_step(dist)));
        }
    }

    // Sprites, far to near.
    let invdet = 1.0 / (planex * diry - dirx * planey);
    let mut placed: Vec<(f32, f32, SpriteKind)> = sprites
        .iter()
        .map(|s| {
            let relx = s.x_cm as f32 / cell - posx;
            let rely = s.y_cm as f32 / cell - posy;
            let tx = invdet * (diry * relx - dirx * rely);
            let ty = invdet * (-planey * relx + planex * rely);
            (tx, ty, s.kind)
        })
        .filter(|(_, ty, _)| *ty > 0.05)
        .collect();
    placed.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (tx, ty, kind) in placed {
        let screenx = (w as f32 / 2.0) * (1.0 + tx / ty);
        let hpx = SPRITE_H * hf / ty;
        let wpx = SPRITE_W * hf / ty;
        if hpx < 1.0 || wpx < 1.0 {
            continue;
        }
        let bottom = horizon as f32 + EYE * hf / ty;
        let top = bottom - hpx;
        let left = screenx - wpx / 2.0;
        let step = fog_step(ty);
        let body = shade(
            match kind {
                SpriteKind::Hostile => Base::Hostile,
                SpriteKind::Friendly => Base::Friendly,
                SpriteKind::Unknown => Base::Unknown,
                SpriteKind::Wreck => Base::Wreck,
            },
            step,
        );
        let outline = shade(Base::Outline, step);
        let x0 = left.floor().max(0.0) as usize;
        let x1 = ((left + wpx).ceil().max(0.0) as usize).min(w);
        let y0 = top.floor().max(0.0) as usize;
        let y1 = ((bottom).ceil().max(0.0) as usize).min(h);
        let (du, dv) = (1.0 / wpx, 1.0 / hpx);
        for (x, &z) in zbuf.iter().enumerate().take(x1).skip(x0) {
            if ty >= z {
                continue;
            }
            let u = (x as f32 + 0.5 - left) / wpx;
            for y in y0..y1 {
                let v = (y as f32 + 0.5 - top) / hpx;
                if !tank(u, v) {
                    continue;
                }
                let edge = !tank(u - du, v) || !tank(u + du, v) || !tank(u, v - dv) || !tank(u, v + dv);
                frame.put(x, y, if edge { outline } else { body });
            }
        }
    }
}

/// The silhouette: a hull across the lower half, a turret on top, a barrel
/// out to the right. `u` runs left to right and `v` top to bottom, both
/// `0..1` over the sprite's box.
fn tank(u: f32, v: f32) -> bool {
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return false;
    }
    let hull = (0.5..0.92).contains(&v) && (0.04..0.96).contains(&u);
    let turret = (0.22..0.5).contains(&v) && (u - 0.45).abs() < 0.22;
    let barrel = (0.3..0.38).contains(&v) && (0.6..0.98).contains(&u);
    let tracks = v >= 0.92 && (0.08..0.92).contains(&u);
    hull || turret || barrel || tracks
}

/// DDA from `(posx, posy)` along `(rdx, rdy)`: the perpendicular distance to
/// the first blocking cell and which side (0 = an east/west face, 1 = a
/// north/south face) was hit; infinite when nothing blocks within
/// [`MAX_STEPS`].
fn cast(map: &Map, posx: f32, posy: f32, rdx: f32, rdy: f32) -> (f32, u8) {
    let mut mapx = posx.floor() as i64;
    let mut mapy = posy.floor() as i64;
    let deltax = if rdx == 0.0 { f32::INFINITY } else { (1.0 / rdx).abs() };
    let deltay = if rdy == 0.0 { f32::INFINITY } else { (1.0 / rdy).abs() };
    let (stepx, mut sidex) = if rdx < 0.0 {
        (-1, (posx - mapx as f32) * deltax)
    } else {
        (1, (mapx as f32 + 1.0 - posx) * deltax)
    };
    let (stepy, mut sidey) = if rdy < 0.0 {
        (-1, (posy - mapy as f32) * deltay)
    } else {
        (1, (mapy as f32 + 1.0 - posy) * deltay)
    };
    let mut side = 0;
    for _ in 0..MAX_STEPS {
        if sidex < sidey {
            sidex += deltax;
            mapx += stepx;
            side = 0;
        } else {
            sidey += deltay;
            mapy += stepy;
            side = 1;
        }
        if map.get(mapx, mapy).blocks() {
            let perp = if side == 0 { sidex - deltax } else { sidey - deltay };
            return (perp.max(1e-4), side);
        }
    }
    (f32::INFINITY, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yard() -> Map {
        // A 16x16 grass yard with a wall across the north edge at y = 12 and
        // a pond.
        let mut map = Map::new(16, 16, 1000);
        for y in 0..16 {
            for x in 0..16 {
                map.set(x, y, Ground::Grass);
            }
        }
        for x in 0..16 {
            map.set(x, 12, Ground::Wall);
        }
        map.set(5, 6, Ground::Water);
        map
    }

    #[test]
    fn legend_words_and_blocking() {
        assert_eq!(Ground::from_legend_name("woods_light"), Ground::Woods);
        assert_eq!(Ground::from_legend_name("anything"), Ground::Open);
        assert!(Ground::Wall.blocks());
        assert!(!Ground::Water.blocks());
        let map = yard();
        assert_eq!(map.get(-1, 0), Ground::Wall, "outside is wall");
        assert_eq!(map.get(5, 6), Ground::Water);
    }

    #[test]
    fn a_ray_north_hits_the_wall_at_its_distance() {
        let map = yard();
        // From the centre of cell (8, 4) looking north: the wall's south face
        // at y = 12 is 7.5 cells away.
        let (perp, side) = cast(&map, 8.5, 4.5, 0.0, 1.0);
        assert!((perp - 7.5).abs() < 1e-4, "{perp}");
        assert_eq!(side, 1);
        // Looking east: the map edge at x = 16, 7.5 cells away.
        let (perp, side) = cast(&map, 8.5, 4.5, 1.0, 0.0);
        assert!((perp - 7.5).abs() < 1e-4, "{perp}");
        assert_eq!(side, 0);
    }

    #[test]
    fn the_frame_is_sky_wall_floor_and_never_more_than_256_colours() {
        let map = yard();
        let cam = Camera {
            x_cm: 8_500,
            y_cm: 4_500,
            heading_mdeg: 0,
        };
        let sprites = [
            Sprite::at_bearing(&cam, 10.0, 30.0, SpriteKind::Hostile),
            Sprite::at_bearing(&cam, 350.0, 50.0, SpriteKind::Friendly),
        ];
        let mut frame = Frame::new(320, 220);
        render(&mut frame, &map, &cam, &sprites);
        // Top row is sky, the middle column has a wall slice around the
        // horizon, the bottom row is grass-ish floor.
        assert_eq!(frame.get(160, 0), sky(0));
        let mid = frame.get(160, 110);
        assert_ne!(mid, sky(SKY_STEPS - 1));
        let floor = frame.get(160, 219);
        assert!(floor[1] > floor[0] && floor[1] > floor[2], "grass is green: {floor:?}");
        assert!(frame.count_colours(256) <= 256);
        // The hostile sprite at 30 m dead-ish ahead paints red somewhere.
        let red = (0..320usize)
            .flat_map(|x| (0..220usize).map(move |y| (x, y)))
            .any(|(x, y)| {
                let p = frame.get(x, y);
                p[0] > 150 && p[1] < 90 && p[2] < 90
            });
        assert!(red, "the hostile sprite is drawn");
    }

    #[test]
    fn a_sprite_behind_a_wall_is_hidden() {
        let map = yard();
        let cam = Camera {
            x_cm: 8_500,
            y_cm: 4_500,
            heading_mdeg: 0,
        };
        // 100 m north is past the wall at y = 12 (75 m away).
        let behind = [Sprite::at_bearing(&cam, 0.0, 100.0, SpriteKind::Hostile)];
        let mut frame = Frame::new(160, 110);
        render(&mut frame, &map, &cam, &behind);
        let red = (0..160usize)
            .flat_map(|x| (0..110usize).map(move |y| (x, y)))
            .any(|(x, y)| {
                let p = frame.get(x, y);
                p[0] > 150 && p[1] < 90 && p[2] < 90
            });
        assert!(!red, "hidden by the wall");
    }

    #[test]
    fn bearing_placement_is_clockwise_from_north() {
        let cam = Camera {
            x_cm: 0,
            y_cm: 0,
            heading_mdeg: 0,
        };
        let east = Sprite::at_bearing(&cam, 90.0, 10.0, SpriteKind::Unknown);
        assert_eq!((east.x_cm, east.y_cm), (1000, 0));
        let north = Sprite::at_bearing(&cam, 0.0, 10.0, SpriteKind::Unknown);
        assert_eq!((north.x_cm, north.y_cm), (0, 1000));
    }
}
