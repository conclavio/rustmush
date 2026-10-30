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
//! **Two passes (doc 21 cut G.1).** The rays are cast column by column, and
//! each column keeps only where its sky ends and the blocks it shows; the
//! frame is then written row by row, top to bottom, so the writes run along
//! memory and everything that is constant along a row (the sky band, the
//! floor's distance, its fog step) is worked out once a row rather than once
//! a pixel. Every colour comes from tables built once a render.
//!
//! **The sim's heights** (2026-10-22, the owner): the eye stands at a unit's
//! sensor height (1.5 m), walls are 3 m, light woods 6 m and heavy woods 8 m —
//! the sim's own line-of-sight numbers ([`EYE_HEIGHT_CM`], [`Ground::height_cm`]),
//! so what the picture hides behind a wall is what the sim's sight hides. A
//! ray walks on past a block lower than one behind it (a wall shows the woods
//! that rise over it), up to four blocks a column; units are hidden pixel by
//! pixel against what was drawn nearer (a turret shows over a low wall); and a
//! **contact the sim reports is never invisible**: one the picture covers, or
//! too far off to draw, is bracketed where it stands.
//!
//! **Colours are a closed set by construction** — the private `Base` set times
//! [`FOG_STEPS`], plus the sky's [`SKY_STEPS`], plus the sprites' paints
//! (the three teams' ramps and the fixed materials, doc 22 cut U.3) times
//! their coarser `SPRITE_FOG` — 14 x 12 + 16 + 12 x 6 = 256, so a rendered
//! frame never has more than 256 colours and the PNG encoder always takes
//! the palette path. A wrong constant here shows up as the truecolor
//! fallback, which a test pins against.

use crate::Frame;
use crate::bake;
use crate::frame::Rgb;
use crate::look::{Facing, Material, Sheet};
use crate::math::Det as _;

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
    /// Light woods (trees with sight between them, 6 m): block the eye,
    /// drawn as a green block.
    Woods,
    /// Heavy woods (canopy and undergrowth, 8 m): block the eye, drawn
    /// darker and taller.
    WoodsHeavy,
    /// A wall, a building, rock (3 m): blocks the eye.
    Wall,
}

/// Planetary / regional surface biome for sky, horizon haze, ground palettes,
/// and 3D surface scatter across all three cockpit views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Biome {
    /// Earthlike temperate valley (`proving_ground` default).
    #[default]
    Temperate,
    /// Martian red-ochre regolith canyon & oxidized dust atmosphere.
    Mars,
    /// Airless lunar / asteroid grey basalt crater & vacuum black sky.
    Lunar,
    /// Glacial ice shelf / Europa frozen plains & deep crevasse water.
    Ice,
}

impl Biome {
    /// Parse an explicit biome identifier or surface keyword.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Biome> {
        let s = name.trim().to_ascii_lowercase();
        if s.contains("mars") || s.contains("canyon") || s.contains("red_planet") {
            Some(Biome::Mars)
        } else if s.contains("lunar")
            || s.contains("moon")
            || s.contains("asteroid")
            || s.contains("crater")
            || s.contains("regolith")
        {
            Some(Biome::Lunar)
        } else if s.contains("ice")
            || s.contains("glacier")
            || s.contains("europa")
            || s.contains("arctic")
        {
            Some(Biome::Ice)
        } else if s.contains("temperate") || s.contains("earth") || s.contains("proving_ground") {
            Some(Biome::Temperate)
        } else {
            None
        }
    }

    /// Sky colour at the zenith.
    #[must_use]
    pub fn zenith_rgb(self) -> Rgb {
        match self {
            Biome::Temperate => ZENITH,
            Biome::Mars => [68, 34, 28],
            Biome::Lunar => [6, 8, 14],
            Biome::Ice => [28, 48, 76],
        }
    }

    /// Horizon haze colour that distant terrain and sprites fog toward.
    #[must_use]
    pub fn horizon_rgb(self) -> Rgb {
        match self {
            Biome::Temperate => HORIZON,
            Biome::Mars => [186, 112, 78],
            Biome::Lunar => [26, 30, 38],
            Biome::Ice => [164, 198, 224],
        }
    }
}

impl Ground {
    /// The `arena.map` legend word for a glyph, mapped to a ground.
    #[must_use]
    pub fn from_legend_name(name: &str) -> Ground {
        match name {
            "grass" | "sand" | "regolith" | "snow" => Ground::Grass,
            "water" | "water_deep" | "ice" => Ground::Water,
            "road" | "paved" => Ground::Road,
            "wall" | "rock" | "building" | "cliff" => Ground::Wall,
            "woods_heavy" => Ground::WoodsHeavy,
            n if n.starts_with("forest") => Ground::WoodsHeavy,
            n if n.starts_with("woods") => Ground::Woods,
            _ => Ground::Open,
        }
    }

    /// Whether a ray stops here.
    #[must_use]
    pub fn blocks(self) -> bool {
        self.height_cm() > 0
    }

    /// How high it stands above the ground, centimetres: the sim's own
    /// obstacle heights (`mush_atlas::physics::obstacle_height_cm`), so the
    /// picture hides behind a wall exactly what the sim's line of sight does.
    #[must_use]
    pub fn height_cm(self) -> i32 {
        match self {
            Ground::Wall => WALL_HEIGHT_CM,
            Ground::Woods => WOODS_LIGHT_HEIGHT_CM,
            Ground::WoodsHeavy => WOODS_HEAVY_HEIGHT_CM,
            _ => 0,
        }
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
    biome: Biome,
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
            biome: Biome::Temperate,
            cells: vec![Ground::Open; width * height],
        }
    }

    /// The planetary/regional biome of this map.
    #[must_use]
    pub fn biome(&self) -> Biome {
        self.biome
    }

    /// Set the planetary/regional biome of this map.
    pub fn set_biome(&mut self, biome: Biome) {
        self.biome = biome;
    }

    /// From glyph rows (south row first) and a legend.
    pub fn from_glyph_rows(
        rows: &[Vec<char>],
        legend: &dyn Fn(char) -> Ground,
        cell_cm: u32,
    ) -> Map {
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

/// What a sprite is drawn as: **its team's colour** (doc 22 D-22-3) — the red
/// team red and the blue team blue whoever is looking, so a pilot on red is
/// never told that red means enemy; friend or foe is the markers' job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteKind {
    /// A contact classified on the red team.
    Red,
    /// A contact classified on the blue team.
    Blue,
    /// An unclassified contact: a neutral grey.
    Unknown,
    /// A wreck: burnt brown, no team.
    Wreck,
}

/// A unit on the ground: drawn from its look's sprite sheet where it has a
/// [`SpritePose`] (doc 22 cut U.3), else as a tank silhouette facing the eye,
/// in its team's colour either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sprite {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Drawn as.
    pub kind: SpriteKind,
    /// Its sheet and pose, where its type has a look with sprites.
    pub pose: Option<SpritePose>,
}

/// How a unit with a sprite sheet stands (doc 22 cut U.3): which face it
/// shows, whether it is moving, and the animation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpritePose {
    /// The sheet.
    pub sheet: &'static Sheet,
    /// The face it shows the eye.
    pub facing: Facing,
    /// Whether it plays its moving cycle.
    pub moving: bool,
    /// The animation step (the moving cycle's frame is `step` modulo its
    /// length).
    pub step: usize,
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
            x_cm: cam.x_cm + (range_cm * b.det_sin()).round() as i64,
            y_cm: cam.y_cm + (range_cm * b.det_cos()).round() as i64,
            kind,
            pose: None,
        }
    }

    /// How big it stands in the world, `(width, height)` centimetres: its
    /// sheet's `size_cm`, or the silhouette's.
    pub(crate) fn size_cm(&self) -> (f32, f32) {
        self.pose
            .map_or((SPRITE_WIDTH_CM as f32, SPRITE_HEIGHT_CM as f32), |pose| {
                (pose.sheet.size_cm.0 as f32, pose.sheet.size_cm.1 as f32)
            })
    }
}

/// A sprite's colour before fog (doc 22 D-22-7): the three teams' ramps —
/// red, blue, and the grey of a contact not told apart yet — and the fixed
/// materials. Black is [`Base::Outline`] and a wreck's body [`Base::Wreck`].
/// Fogged in [`SPRITE_FOG`] steps, so with the ground's bases and the sky the
/// picture never has more than 256 colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    RedDark,
    Red,
    RedLight,
    BlueDark,
    Blue,
    BlueLight,
    GreyDark,
    Grey,
    GreyLight,
    Steel,
    Pale,
    Shadow,
}

impl Paint {
    /// How many paints there are: the rows of the sprite shade table.
    const COUNT: usize = 12;
    const ALL: [Paint; Paint::COUNT] = [
        Paint::RedDark,
        Paint::Red,
        Paint::RedLight,
        Paint::BlueDark,
        Paint::Blue,
        Paint::BlueLight,
        Paint::GreyDark,
        Paint::Grey,
        Paint::GreyLight,
        Paint::Steel,
        Paint::Pale,
        Paint::Shadow,
    ];

    fn rgb(self) -> Rgb {
        match self {
            Paint::RedDark => [120, 36, 32],
            Paint::Red => [178, 54, 48],
            Paint::RedLight => [214, 98, 84],
            Paint::BlueDark => [40, 78, 140],
            Paint::Blue => [64, 124, 204],
            Paint::BlueLight => [118, 166, 228],
            Paint::GreyDark => [92, 92, 88],
            Paint::Grey => [132, 132, 126],
            Paint::GreyLight => [174, 174, 166],
            Paint::Steel => [96, 100, 108],
            Paint::Pale => [216, 216, 198],
            Paint::Shadow => [34, 40, 28],
        }
    }
}

/// The sprite fog's steps: half the ground's.
pub(crate) const SPRITE_FOG: usize = FOG_STEPS / 2;

/// The colour a sprite of `kind` paints `material` at fog `step` (of the
/// ground's [`FOG_STEPS`]): the team slots by its team, a wreck's body burnt,
/// black the outline's.
pub(crate) fn sprite_colour(
    shades: &Shades,
    kind: SpriteKind,
    material: Material,
    step: usize,
) -> Rgb {
    let team = |dark, mid, light| match material {
        Material::TeamDark => Some(dark),
        Material::Team => Some(mid),
        _ => Some(light),
    };
    let paint = match (material, kind) {
        (Material::Black, _) | (Material::TeamDark, SpriteKind::Wreck) => None,
        (Material::Steel, _) | (Material::TeamLight, SpriteKind::Wreck) => Some(Paint::Steel),
        (Material::Pale, _) => Some(Paint::Pale),
        (Material::Team, SpriteKind::Wreck) => return shades.shade(Base::Wreck, step),
        (_, SpriteKind::Red) => team(Paint::RedDark, Paint::Red, Paint::RedLight),
        (_, SpriteKind::Blue) => team(Paint::BlueDark, Paint::Blue, Paint::BlueLight),
        (_, SpriteKind::Unknown) => team(Paint::GreyDark, Paint::Grey, Paint::GreyLight),
    };
    match paint {
        Some(p) => shades.paint(p, step),
        None => shades.shade(Base::Outline, step),
    }
}

/// The closed set of things a pixel can be, before fog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base {
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
    WoodsHeavy,
    Wreck,
    Outline,
}

impl Base {
    /// How many bases there are: the rows of the shade table.
    const COUNT: usize = 14;

    pub(crate) fn rgb_for_biome(self, biome: Biome) -> Rgb {
        match biome {
            Biome::Temperate => match self {
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
                Base::WoodsHeavy => [28, 62, 34],
                Base::Wreck => [72, 62, 52],
                Base::Outline => [18, 18, 22],
            },
            Biome::Mars => match self {
                Base::Open => [156, 78, 52],
                Base::Grass => [136, 64, 42],
                Base::Water => [58, 42, 40],
                Base::Road => [124, 82, 64],
                Base::OpenLine => [126, 60, 38],
                Base::GrassLine => [108, 50, 32],
                Base::WaterLine => [44, 32, 30],
                Base::RoadLine => [98, 64, 50],
                Base::WallLit => [164, 108, 86],
                Base::WallDark => [116, 74, 58],
                Base::Woods => [112, 62, 46],
                Base::WoodsHeavy => [88, 48, 36],
                Base::Wreck => [72, 62, 52],
                Base::Outline => [18, 18, 22],
            },
            Biome::Lunar => match self {
                Base::Open => [122, 126, 134],
                Base::Grass => [96, 100, 108],
                Base::Water => [48, 52, 60],
                Base::Road => [144, 148, 156],
                Base::OpenLine => [94, 98, 106],
                Base::GrassLine => [74, 78, 86],
                Base::WaterLine => [36, 40, 48],
                Base::RoadLine => [116, 120, 128],
                Base::WallLit => [164, 168, 176],
                Base::WallDark => [112, 116, 124],
                Base::Woods => [106, 110, 118],
                Base::WoodsHeavy => [82, 86, 94],
                Base::Wreck => [72, 62, 52],
                Base::Outline => [18, 18, 22],
            },
            Biome::Ice => match self {
                Base::Open => [196, 218, 236],
                Base::Grass => [168, 198, 224],
                Base::Water => [38, 82, 132],
                Base::Road => [148, 172, 196],
                Base::OpenLine => [156, 182, 206],
                Base::GrassLine => [134, 164, 192],
                Base::WaterLine => [28, 64, 106],
                Base::RoadLine => [118, 142, 166],
                Base::WallLit => [216, 234, 248],
                Base::WallDark => [148, 174, 198],
                Base::Woods => [68, 112, 122],
                Base::WoodsHeavy => [48, 84, 96],
                Base::Wreck => [72, 62, 52],
                Base::Outline => [18, 18, 22],
            },
        }
    }

    pub(crate) fn floor(ground: Ground, line: bool) -> Base {
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
pub(crate) const FOG_CELLS: f32 = 60.0;
/// The colour at the horizon, what fog fades to.
pub(crate) const HORIZON: Rgb = [150, 165, 185];
/// The colour at the top of the sky.
const ZENITH: Rgb = [24, 36, 72];
/// The eye's height above the ground, centimetres: a unit's sensor height,
/// the eye the sim's line of sight is drawn from (`mush_atlas::trace::
/// EYE_HEIGHT_CM`, the stock `sensor_height_cm`), so the picture's horizon
/// and what a wall hides are the sim's. A contract test pins every height
/// here to the sim's (`mush-pixel` does not depend on the atlas).
pub const EYE_HEIGHT_CM: i32 = 150;
/// A wall's height, centimetres (`mush_atlas::physics::WALL_HEIGHT_CM`).
pub const WALL_HEIGHT_CM: i32 = 300;
/// Light woods' height, centimetres (`obstacle_height_cm(WoodsLight)`).
pub const WOODS_LIGHT_HEIGHT_CM: i32 = 600;
/// Heavy woods' height, centimetres (`obstacle_height_cm(WoodsHeavy)`).
pub const WOODS_HEAVY_HEIGHT_CM: i32 = 800;
/// A unit's drawn height and width, centimetres (3.5 m tall, 5 m wide).
pub const SPRITE_HEIGHT_CM: i32 = 350;
/// See [`SPRITE_HEIGHT_CM`].
pub const SPRITE_WIDTH_CM: i32 = 500;
/// The most blocks one column shows, nearest first: a wall, the woods that
/// rise over it, the taller woods over those.
const MAX_SPANS: usize = 4;
/// A contact the sim reports is never invisible: one the picture's ground
/// covers, or too far off to draw, is bracketed at least this many pixels a
/// side.
pub(crate) const MARK_MIN_PX: f32 = 6.0;
/// A contact drawn but too small or too fogged to read — under this many
/// pixels tall, or in the last third of the fog — is bracketed as well, at
/// least this many pixels a side (the dev2 smoke of 2026-10-22: a tank at
/// 550 m was three fogged pixels under the reticle's centre).
pub(crate) const MARK_FAR_PX: f32 = 10.0;
/// Half the field of view as the camera plane's length (66 degrees).
pub(crate) const PLANE: f32 = 0.66;

/// The picture's vertical scale: pixels a cell of height spans at a cell's
/// distance. Three-quarters of the frame's width, which is the height of a
/// 4:3 frame and within 1 % of the horizontal scale [`PLANE`] gives, so the
/// pixels are square and a frame shaped to a wide box (the text cockpit's
/// top two-thirds, 2026-10-22) shows the same world as a 4:3 one, cropped
/// above and below rather than squashed.
pub(crate) fn focal(w: usize) -> f32 {
    w as f32 * 0.75
}
/// How far a ray walks before giving up (cells).
const MAX_STEPS: usize = 160;
/// How wide the floor's cell lines are, as a fraction of a cell.
const LINE: f32 = 0.07;

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

pub(crate) fn fog_step(dist: f32) -> usize {
    ((dist / FOG_CELLS) * FOG_STEPS as f32)
        .floor()
        .clamp(0.0, (FOG_STEPS - 1) as f32) as usize
}

fn shade(base: Base, biome: Biome, step: usize) -> Rgb {
    lerp(
        base.rgb_for_biome(biome),
        biome.horizon_rgb(),
        step as f32 / (FOG_STEPS - 1) as f32,
    )
}

fn sky(biome: Biome, step: usize) -> Rgb {
    lerp(
        biome.zenith_rgb(),
        biome.horizon_rgb(),
        step as f32 / (SKY_STEPS - 1) as f32,
    )
}

/// Every colour a render can draw, worked out once: each base at each fog
/// step, and each sky band. The values are exactly [`shade`] and [`sky`]
/// (built by calling them), so a table read and a lerp agree to the bit.
pub(crate) struct Shades {
    horizon: Rgb,
    base: [[Rgb; FOG_STEPS]; Base::COUNT],
    sky: [Rgb; SKY_STEPS],
    paint: [[Rgb; SPRITE_FOG]; Paint::COUNT],
}

impl Shades {
    #[allow(dead_code)]
    pub(crate) fn new() -> Shades {
        Self::for_biome(Biome::Temperate)
    }

    pub(crate) fn for_biome(biome: Biome) -> Shades {
        const ALL: [Base; Base::COUNT] = [
            Base::Open,
            Base::Grass,
            Base::Water,
            Base::Road,
            Base::OpenLine,
            Base::GrassLine,
            Base::WaterLine,
            Base::RoadLine,
            Base::WallLit,
            Base::WallDark,
            Base::Woods,
            Base::WoodsHeavy,
            Base::Wreck,
            Base::Outline,
        ];
        let horizon = biome.horizon_rgb();
        let mut base = [[[0; 3]; FOG_STEPS]; Base::COUNT];
        for (row, b) in base.iter_mut().zip(ALL) {
            for (step, px) in row.iter_mut().enumerate() {
                *px = shade(b, biome, step);
            }
        }
        let mut bands = [[0; 3]; SKY_STEPS];
        for (step, px) in bands.iter_mut().enumerate() {
            *px = sky(biome, step);
        }
        let mut paint = [[[0; 3]; SPRITE_FOG]; Paint::COUNT];
        for (row, p) in paint.iter_mut().zip(Paint::ALL) {
            for (step, px) in row.iter_mut().enumerate() {
                *px = lerp(p.rgb(), horizon, step as f32 / (SPRITE_FOG - 1) as f32);
            }
        }
        Shades {
            horizon,
            base,
            sky: bands,
            paint,
        }
    }

    /// Sprite paint `p` at the ground's fog `step`, in the sprite fog's
    /// coarser steps.
    pub(crate) fn paint(&self, p: Paint, step: usize) -> Rgb {
        self.paint
            .get(p as usize)
            .and_then(|row| row.get(step * SPRITE_FOG / FOG_STEPS))
            .copied()
            .unwrap_or(self.horizon)
    }

    /// `base` at fog `step` (the horizon colour past the last step, which
    /// [`fog_step`] never yields).
    pub(crate) fn shade(&self, base: Base, step: usize) -> Rgb {
        self.base
            .get(base as usize)
            .and_then(|row| row.get(step))
            .copied()
            .unwrap_or(self.horizon)
    }

    /// Sky band `step` (the horizon colour past the last band).
    pub(crate) fn sky(&self, step: usize) -> Rgb {
        self.sky.get(step).copied().unwrap_or(self.horizon)
    }
}

/// One block a column's ray met and shows: rows `top..end` of the
/// column, drawn `px`, `perp` cells away.
#[derive(Clone, Copy, Default)]
struct Span {
    top: usize,
    end: usize,
    px: Rgb,
    perp: f32,
}

/// What the column pass leaves the row pass: the ray, and the blocks it
/// shows, nearest first. Rows above the topmost block are sky (or the
/// horizon's haze); rows below the horizon that no block covers are floor.
#[derive(Clone, Copy)]
struct Column {
    rdx: f32,
    rdy: f32,
    spans: [Span; MAX_SPANS],
    n: usize,
    /// Rows `0..sky_end` are sky.
    sky_end: usize,
}

impl Column {
    /// The block covering row `y`, if one does.
    fn span_at(&self, y: usize) -> Option<&Span> {
        self.spans
            .get(..self.n)
            .and_then(|spans| spans.iter().find(|s| (s.top..s.end).contains(&y)))
    }
}

/// The camera in cell units: where the eye stands, the way it looks, the
/// camera plane across it, and the heights that matter, in cells.
#[derive(Clone, Copy)]
struct Eye {
    cell: f32,
    posx: f32,
    posy: f32,
    dirx: f32,
    diry: f32,
    planex: f32,
    planey: f32,
    /// The eye's height above the ground, cells.
    eye: f32,
}

impl Eye {
    /// See [`project_from`].
    fn project(&self, w: usize, h: usize, s: &Sprite) -> Option<SpriteAt> {
        let invdet = 1.0 / (self.planex * self.diry - self.dirx * self.planey);
        let relx = s.x_cm as f32 / self.cell - self.posx;
        let rely = s.y_cm as f32 / self.cell - self.posy;
        let tx = invdet * (self.diry * relx - self.dirx * rely);
        let ty = invdet * (-self.planey * relx + self.planex * rely);
        if ty <= 0.05 {
            return None;
        }
        let hf = focal(w);
        let (wcm, hcm) = s.size_cm();
        Some(SpriteAt {
            screenx: (w as f32 / 2.0) * (1.0 + tx / ty),
            bottom: (h / 2) as f32 + self.eye * hf / ty,
            depth: ty,
            fog_depth: ty,
            height: hcm / self.cell * hf / ty,
            width: wcm / self.cell * hf / ty,
            depression_deg: 0.0,
            kind: s.kind,
            pose: s.pose,
        })
    }

    fn with_height(map: &Map, cam: &Camera, eye_cm: Option<i64>) -> Eye {
        let cell = map.cell_cm() as f32;
        let heading = (cam.heading_mdeg % 360_000) as f32 / 1000.0;
        let (dirx, diry) = heading.to_radians().det_sin_cos();
        let eye_cm = eye_cm.unwrap_or(i64::from(EYE_HEIGHT_CM)).max(1) as f32;
        Eye {
            cell,
            posx: cam.x_cm as f32 / cell,
            posy: cam.y_cm as f32 / cell,
            dirx,
            diry,
            planex: diry * PLANE,
            planey: -dirx * PLANE,
            eye: eye_cm / cell,
        }
    }
}

/// The colour base of a block a ray met, by its kind and the face it hit.
fn block_base(ground: Ground, side: u8) -> Base {
    match (ground, side) {
        (Ground::Woods, _) => Base::Woods,
        (Ground::WoodsHeavy, _) => Base::WoodsHeavy,
        (_, 0) => Base::WallLit,
        _ => Base::WallDark,
    }
}

/// Draw the scene into `frame` from the stock [`EYE_HEIGHT_CM`].
pub fn render(frame: &mut Frame, map: &Map, cam: &Camera, sprites: &[Sprite]) {
    render_from(frame, map, cam, None, sprites);
}

/// Draw the scene into `frame`, with the eye at `eye_cm` above the ground
/// (or [`EYE_HEIGHT_CM`] when `None`; doc 21a `W-10`).
pub fn render_from(
    frame: &mut Frame,
    map: &Map,
    cam: &Camera,
    eye_cm: Option<i64>,
    sprites: &[Sprite],
) {
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return;
    }
    let e = Eye::with_height(map, cam, eye_cm);
    let hf = focal(w);
    let horizon = h / 2;
    let hz = horizon as f32;
    let shades = Shades::for_biome(map.biome());
    let horizon_rgb = map.biome().horizon_rgb();

    // The column pass: one ray a column, walked past a block lower than one
    // behind it, so a wall shows the woods that rise over it.
    let mut columns = Vec::with_capacity(w);
    for x in 0..w {
        let camx = 2.0 * x as f32 / w as f32 - 1.0;
        let rdx = e.dirx + e.planex * camx;
        let rdy = e.diry + e.planey * camx;
        let mut col = Column {
            rdx,
            rdy,
            spans: [Span::default(); MAX_SPANS],
            n: 0,
            sky_end: horizon,
        };
        let mut min_top = f32::INFINITY;
        cast(map, e.posx, e.posy, rdx, rdy, |perp, side, ground| {
            let block = ground.height_cm() as f32 / e.cell;
            let top = hz - (block - e.eye) * hf / perp;
            if top >= min_top {
                // Hidden behind a nearer, taller-looking block.
                return true;
            }
            let end = if col.n == 0 {
                (hz + e.eye * hf / perp).min(h as f32) as usize
            } else {
                min_top.max(0.0) as usize
            };
            let top_row = top.max(0.0) as usize;
            if let Some(span) = col.spans.get_mut(col.n) {
                *span = Span {
                    top: top_row,
                    end: end.max(top_row),
                    px: shades.shade(block_base(ground, side), fog_step(perp)),
                    perp,
                };
                col.n += 1;
            }
            min_top = top;
            top > 0.0 && col.n < MAX_SPANS
        });
        if col.n > 0 {
            col.sky_end = min_top.max(0.0) as usize;
        }
        columns.push(col);
    }

    // The row pass: top to bottom, along memory. A row's sky band, and a
    // floor row's distance and fog step, are one value for the whole row.
    for (y, row) in frame.rgb_mut().chunks_exact_mut(w * 3).enumerate() {
        let sky_px = shades.sky(y * SKY_STEPS / horizon.max(1));
        let floor = (y > horizon).then(|| {
            let dist = e.eye * hf / (y - horizon) as f32;
            (dist, fog_step(dist))
        });
        for (px, c) in row.as_chunks_mut::<3>().0.iter_mut().zip(&columns) {
            *px = if y < c.sky_end && y < horizon {
                sky_px
            } else if let Some(span) = c.span_at(y) {
                span.px
            } else if let Some((dist, step)) = floor {
                let fx = e.posx + c.rdx * dist;
                let fy = e.posy + c.rdy * dist;
                let ground = map.get(fx.floor() as i64, fy.floor() as i64);
                let line = fx.fract().abs() < LINE || fy.fract().abs() < LINE;
                shades.shade(Base::floor(ground, line), step)
            } else {
                horizon_rgb
            };
        }
    }

    // A sprite's pixel shows where nothing nearer was drawn at it.
    let depth = |x: usize, y: usize| -> f32 {
        match columns.get(x) {
            Some(c) => match c.span_at(y) {
                Some(span) => span.perp,
                None if y > horizon => e.eye * hf / (y - horizon) as f32,
                None => f32::INFINITY,
            },
            None => f32::INFINITY,
        }
    };
    let mut placed: Vec<SpriteAt> = sprites.iter().filter_map(|s| e.project(w, h, s)).collect();
    placed.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    for at in placed {
        let _ = draw_sprite(frame, &shades, at, &depth);
    }
}

/// Where `sprite` stands in a `w` by `h` picture of flat `map` from `cam`:
/// its centre column, base row, distance and size, as [`render`] draws it;
/// `None` behind the eye. The cockpit's HUD boxes the lock with it.
#[cfg(test)]
pub(crate) fn project(
    map: &Map,
    cam: &Camera,
    w: usize,
    h: usize,
    sprite: &Sprite,
) -> Option<SpriteAt> {
    project_from(map, cam, None, w, h, sprite)
}

/// Where `sprite` stands in a `w` by `h` picture of flat `map` from `cam`
/// with the eye at `eye_cm` above the ground (or [`EYE_HEIGHT_CM`] when
/// `None`; doc 21a `W-10`), as [`render_from`] draws it.
pub(crate) fn project_from(
    map: &Map,
    cam: &Camera,
    eye_cm: Option<i64>,
    w: usize,
    h: usize,
    sprite: &Sprite,
) -> Option<SpriteAt> {
    Eye::with_height(map, cam, eye_cm).project(w, h, sprite)
}

/// A sprite placed on the screen: its centre column, the row its base
/// stands on, its distance, its size in pixels.
#[derive(Clone, Copy)]
pub(crate) struct SpriteAt {
    pub(crate) screenx: f32,
    pub(crate) bottom: f32,
    pub(crate) depth: f32,
    /// The distance its fog is read at, on [`FOG_CELLS`]' scale: its depth
    /// for the raycaster, and the heightfield's longer view scaled down to it
    /// (doc 21 cut G.7b).
    pub(crate) fog_depth: f32,
    pub(crate) height: f32,
    pub(crate) width: f32,
    /// How far below the eye's level the line to the unit's middle runs,
    /// degrees (negative above it): the heightfield's chain pick (doc 22 §5).
    /// Zero in the raycaster, whose ground is flat and which draws `fp`.
    pub(crate) depression_deg: f32,
    pub(crate) kind: SpriteKind,
    pub(crate) pose: Option<SpritePose>,
}

/// What a sprite's draw put in the frame: the unit's pixels (its body and
/// rim; the shadow's are not counted, a shadow alone is not a contact seen)
/// and the rows its body's own pixels reach — the hull-down check's measure
/// (doc 22 §6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Drawn {
    pub(crate) pixels: usize,
    pub(crate) rows: usize,
}

/// Draw one unit: its silhouette wherever the picture drew nothing nearer
/// (`depth_at(x, y)` is the distance the pixel was drawn at), and — when
/// none of it shows, or it is under two pixels tall — a bracket in its
/// unfogged colour around where it stands: **a contact the sim reports is
/// never invisible** in the picture, whatever the picture's own ground or
/// walls make of the line between (the sim's sight decides what is seen;
/// the picture only draws it). A unit off the side of the view is not drawn.
///
/// A unit whose look is baked (doc 22 D-22-10) draws from its chain — the
/// `fp` chain, or the `iso` one past [`bake::CROSSOVER_DEG`] of depression
/// ([`bake::View::for_depression`]) — at the size nearest above its height
/// ([`draw_chain`]); a look with no chain draws its text sheet,
/// nearest-sampled ([`draw_sheet`], rule 3's fallback); a type with no sheet
/// the silhouette.
pub(crate) fn draw_sprite(
    frame: &mut Frame,
    shades: &Shades,
    at: SpriteAt,
    depth_at: &dyn Fn(usize, usize) -> f32,
) -> Drawn {
    let (w, h) = (frame.width(), frame.height());
    let wf = w as f32;
    if at.screenx + at.width / 2.0 < 0.0 || at.screenx - at.width / 2.0 >= wf {
        return Drawn::default();
    }
    let top = at.bottom - at.height;
    let left = at.screenx - at.width / 2.0;
    let mut drawn = Drawn::default();
    if at.height >= 2.0 && at.width >= 1.0 {
        let step = fog_step(at.fog_depth);
        drawn = match at.pose {
            Some(pose) => match bake::bake_of_sheet(pose.sheet).and_then(|b| {
                b.chain(bake::View::for_depression(at.depression_deg))
                    .map(|c| (b, c))
            }) {
                Some((b, chain)) => {
                    draw_chain(frame, shades, &at, pose, (b.size_cm.1, chain), depth_at)
                }
                None => draw_sheet(frame, shades, &at, pose, step, depth_at),
            },
            None => {
                let body = sprite_colour(shades, at.kind, Material::Team, step);
                let outline = shades.shade(Base::Outline, step);
                let x0 = left.floor().max(0.0) as usize;
                let x1 = ((left + at.width).ceil().max(0.0) as usize).min(w);
                let y0 = top.floor().max(0.0) as usize;
                let y1 = (at.bottom.ceil().max(0.0) as usize).min(h);
                let (du, dv) = (1.0 / at.width, 1.0 / at.height);
                let mut n = Drawn::default();
                let mut rows = vec![false; y1.saturating_sub(y0)];
                for x in x0..x1 {
                    let u = (x as f32 + 0.5 - left) / at.width;
                    for y in y0..y1 {
                        if at.depth >= depth_at(x, y) {
                            continue;
                        }
                        let v = (y as f32 + 0.5 - top) / at.height;
                        if !tank(u, v) {
                            continue;
                        }
                        let edge = !tank(u - du, v)
                            || !tank(u + du, v)
                            || !tank(u, v - dv)
                            || !tank(u, v + dv);
                        frame.put(x, y, if edge { outline } else { body });
                        n.pixels += 1;
                        if let Some(r) = rows.get_mut(y - y0) {
                            *r = true;
                        }
                    }
                }
                n.rows = rows.iter().filter(|&&r| r).count();
                n
            }
        };
    }
    let faint = at.height < MARK_FAR_PX || fog_step(at.fog_depth) >= FOG_STEPS * 2 / 3;
    if drawn.pixels == 0 || faint {
        let least = if drawn.pixels == 0 {
            MARK_MIN_PX
        } else {
            MARK_FAR_PX
        };
        let bw = at.width.max(least);
        let bh = at.height.max(least);
        let cy = at.bottom - at.height / 2.0;
        mark(
            frame,
            (at.screenx - bw / 2.0, cy - bh / 2.0, bw, bh),
            sprite_colour(shades, at.kind, Material::Team, 0),
        );
    }
    drawn
}

/// Draw a unit from its sheet (doc 22 cut U.3): a shadow on the ground under
/// its anchor, then the frame its pose picks — its facing (the left mirrored
/// from the right where the sheet has no left), its moving cycle at the
/// pose's step, or its wreck — scaled over the sprite's box with the anchor
/// pixel standing on `at.bottom` at `at.screenx`, every pixel hidden where
/// the picture drew something nearer. Returns the unit's pixels drawn (the
/// shadow's are not counted: a shadow alone is not a contact seen).
fn draw_sheet(
    frame: &mut Frame,
    shades: &Shades,
    at: &SpriteAt,
    pose: SpritePose,
    step: usize,
    depth_at: &dyn Fn(usize, usize) -> f32,
) -> Drawn {
    let (w, h) = (frame.width(), frame.height());
    let sheet = pose.sheet;
    let picked = if at.kind == SpriteKind::Wreck {
        sheet.wreck_frame(pose.facing).map(|f| (f, false))
    } else {
        sheet.frame(pose.facing, pose.moving, pose.step)
    };
    let Some((pixels, mirrored)) = picked else {
        return Drawn::default();
    };
    let (sw, sh) = (sheet.width as f32, sheet.height as f32);
    let (ax, ay) = (sheet.anchor.0 as f32, sheet.anchor.1 as f32);
    let ax = if mirrored { sw - 1.0 - ax } else { ax };
    let left = at.screenx - (ax + 0.5) / sw * at.width;
    let top = at.bottom - (ay + 1.0) / sh * at.height;

    // The shadow: a flat ellipse on the ground about the anchor, drawn only
    // over ground at about the unit's own distance.
    let rx = at.width * 0.42;
    let ry = (rx * 0.16).max(1.0);
    let shadow = shades.paint(Paint::Shadow, step);
    let (sx0, sx1) = (
        (at.screenx - rx).floor().max(0.0) as usize,
        ((at.screenx + rx).ceil().max(0.0) as usize).min(w),
    );
    let (sy0, sy1) = (
        (at.bottom - ry).floor().max(0.0) as usize,
        ((at.bottom + ry).ceil().max(0.0) as usize).min(h),
    );
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            let (dx, dy) = (
                (x as f32 + 0.5 - at.screenx) / rx,
                (y as f32 + 0.5 - at.bottom) / ry,
            );
            if dx * dx + dy * dy <= 1.0 && depth_at(x, y) >= at.depth * 0.85 {
                frame.put(x, y, shadow);
            }
        }
    }

    let x0 = left.floor().max(0.0) as usize;
    let x1 = ((left + at.width).ceil().max(0.0) as usize).min(w);
    let y0 = top.floor().max(0.0) as usize;
    let y1 = ((top + at.height).ceil().max(0.0) as usize).min(h);
    let mut drawn = Drawn::default();
    for y in y0..y1 {
        let sy = (((y as f32 + 0.5 - top) / at.height * sh).floor().max(0.0) as usize)
            .min(sheet.height - 1);
        let mut row = false;
        for x in x0..x1 {
            if at.depth >= depth_at(x, y) {
                continue;
            }
            let u = ((x as f32 + 0.5 - left) / at.width * sw).floor().max(0.0) as usize;
            let sx = u.min(sheet.width - 1);
            let sx = if mirrored { sheet.width - 1 - sx } else { sx };
            if let Some(Some(material)) = pixels.get(sy * sheet.width + sx) {
                frame.put(x, y, sprite_colour(shades, at.kind, *material, step));
                drawn.pixels += 1;
                row = true;
            }
        }
        drawn.rows += usize::from(row);
    }
    drawn
}

/// **Draw a unit from its baked chain** (doc 22 D-22-10, cut U.6): the size
/// nearest above the unit's height on screen ([`bake::Chain::pick`]: every
/// frame of a chain spans the same world, so the unit's pixels a metre pick
/// the size), scaled down to it — never up from a smaller size while a larger
/// exists — with the frame's anchor on `at.bottom` at `at.screenx`; a shadow
/// on the ground under it, a one-pixel dark rim around it once it is eight
/// pixels tall, and every pixel hidden where the picture drew something
/// nearer (the hull-down clip, doc 22 §5). `height_cm` is the baked model's
/// height.
fn draw_chain(
    frame: &mut Frame,
    shades: &Shades,
    at: &SpriteAt,
    pose: SpritePose,
    (height_cm, chain): (u32, &bake::Chain),
    depth_at: &dyn Fn(usize, usize) -> f32,
) -> Drawn {
    let (w, h) = (frame.width(), frame.height());
    if height_cm == 0 {
        return Drawn::default();
    }
    let px_per_m = at.height / (height_cm as f32 / 100.0);
    let Some((_, sized)) = chain.pick(px_per_m) else {
        return Drawn::default();
    };
    let pose_i = bake::pose_index(pose.moving, pose.step, at.kind == SpriteKind::Wreck);
    let Some(f) = sized.frame(pose.facing, pose_i) else {
        return Drawn::default();
    };
    if f.w == 0 || f.h == 0 {
        return Drawn::default();
    }
    let step = fog_step(at.fog_depth);
    // Screen pixels a frame pixel.
    let k = chain.needed_px(px_per_m) / f32::from(sized.size.max(1));
    let left = at.screenx - (f32::from(f.anchor.0) + 0.5) * k;
    let top = at.bottom - (f32::from(f.anchor.1) + 1.0) * k;
    let (bx0, bx1) = (left + f32::from(f.x) * k, left + f32::from(f.x + f.w) * k);
    let (by0, by1) = (top + f32::from(f.y) * k, top + f32::from(f.y + f.h) * k);

    // The shadow: a flat ellipse on the ground under the anchor.
    let rx = (bx1 - bx0) * 0.44;
    let ry = (rx * 0.16).max(1.0);
    let shadow = shades.paint(Paint::Shadow, step);
    let (sx0, sx1) = (
        (at.screenx - rx).floor().max(0.0) as usize,
        ((at.screenx + rx).ceil().max(0.0) as usize).min(w),
    );
    let (sy0, sy1) = (
        (at.bottom - ry).floor().max(0.0) as usize,
        ((at.bottom + ry).ceil().max(0.0) as usize).min(h),
    );
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            let (dx, dy) = (
                (x as f32 + 0.5 - at.screenx) / rx.max(0.5),
                (y as f32 + 0.5 - at.bottom) / ry,
            );
            if dx * dx + dy * dy <= 1.0 && depth_at(x, y) >= at.depth * 0.85 {
                frame.put(x, y, shadow);
            }
        }
    }

    // The body, and its rim once it is eight pixels tall.
    let rim = (by1 - by0) >= 8.0;
    let pad = if rim { 1.0 } else { 0.0 };
    let outline = shades.shade(Base::Outline, step);
    let x0 = (bx0 - pad).floor().max(0.0) as usize;
    let x1 = ((bx1 + pad).ceil().max(0.0) as usize).min(w);
    let y0 = (by0 - pad).floor().max(0.0) as usize;
    let y1 = ((by1 + pad).ceil().max(0.0) as usize).min(h);
    let mut drawn = Drawn::default();
    for y in y0..y1 {
        let sy = ((y as f32 + 0.5 - top) / k).floor() as i64;
        let mut row = false;
        for x in x0..x1 {
            if at.depth >= depth_at(x, y) {
                continue;
            }
            let sx = ((x as f32 + 0.5 - left) / k).floor() as i64;
            if let Some(material) = bake::material_from_code(f.code(sx, sy)) {
                frame.put(x, y, sprite_colour(shades, at.kind, material, step));
                drawn.pixels += 1;
                row = true;
            } else if rim
                && [(0, -1), (0, 1), (-1, 0), (1, 0)]
                    .iter()
                    .any(|&(dx, dy)| f.code(sx + dx, sy + dy) != 0)
            {
                frame.put(x, y, outline);
                drawn.pixels += 1;
            }
        }
        drawn.rows += usize::from(row);
    }
    drawn
}

/// Four corner brackets around a box (`x`, `y`, width, height in pixels),
/// each arm a quarter of the side and at least two pixels.
pub(crate) fn mark(frame: &mut Frame, (x, y, bw, bh): (f32, f32, f32, f32), px: Rgb) {
    let (x0, y0) = (x.round() as i64, y.round() as i64);
    let (x1, y1) = ((x + bw).round() as i64 - 1, (y + bh).round() as i64 - 1);
    let arm_x = ((bw / 4.0).round() as i64).max(2);
    let arm_y = ((bh / 4.0).round() as i64).max(2);
    let mut put = |px_x: i64, px_y: i64| {
        if px_x >= 0 && px_y >= 0 {
            frame.put(px_x as usize, px_y as usize, px);
        }
    };
    for i in 0..arm_x {
        put(x0 + i, y0);
        put(x1 - i, y0);
        put(x0 + i, y1);
        put(x1 - i, y1);
    }
    for i in 0..arm_y {
        put(x0, y0 + i);
        put(x1, y0 + i);
        put(x0, y1 - i);
        put(x1, y1 - i);
    }
}

/// The silhouette: a hull across the lower half, a turret on top, a barrel
/// out to the right. `u` runs left to right and `v` top to bottom, both
/// `0..1` over the sprite's box.
pub(crate) fn tank(u: f32, v: f32) -> bool {
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return false;
    }
    let hull = (0.5..0.92).contains(&v) && (0.04..0.96).contains(&u);
    let turret = (0.22..0.5).contains(&v) && (u - 0.45).abs() < 0.22;
    let barrel = (0.3..0.38).contains(&v) && (0.6..0.98).contains(&u);
    let tracks = v >= 0.92 && (0.08..0.92).contains(&u);
    hull || turret || barrel || tracks
}

/// DDA from `(posx, posy)` along `(rdx, rdy)`: `hit(perp, side, ground)` for
/// each blocking cell in order, its perpendicular distance and which side (0
/// = an east/west face, 1 = a north/south face) the ray entered by, until it
/// answers `false` or [`MAX_STEPS`] cells are walked.
fn cast(
    map: &Map,
    posx: f32,
    posy: f32,
    rdx: f32,
    rdy: f32,
    mut hit: impl FnMut(f32, u8, Ground) -> bool,
) {
    let mut mapx = posx.floor() as i64;
    let mut mapy = posy.floor() as i64;
    let deltax = if rdx == 0.0 {
        f32::INFINITY
    } else {
        (1.0 / rdx).abs()
    };
    let deltay = if rdy == 0.0 {
        f32::INFINITY
    } else {
        (1.0 / rdy).abs()
    };
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
    let mut side;
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
        let ground = map.get(mapx, mapy);
        if ground.blocks() {
            let perp = if side == 0 {
                sidex - deltax
            } else {
                sidey - deltay
            };
            if !hit(perp.max(1e-4), side, ground) {
                return;
            }
        }
        if (if sidex < sidey { sidex } else { sidey }) > FOG_CELLS * 1.5 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spike left the horizon row of a ray that hit nothing as it was,
    /// so whatever the last frame drew there stayed; every pixel is drawn
    /// now, whatever the frame held before.
    #[test]
    fn every_pixel_is_drawn_whatever_the_frame_held() {
        let open = Map::new(512, 512, 1000);
        let cam = Camera {
            x_cm: 256_000,
            y_cm: 256_000,
            heading_mdeg: 90_000,
        };
        let mut a = Frame::new(320, 220);
        a.clear([255, 0, 255]);
        render(&mut a, &open, &cam, &[]);
        let mut b = Frame::new(320, 220);
        b.clear([0, 255, 0]);
        render(&mut b, &open, &cam, &[]);
        assert!(a == b);
        assert_eq!(a.get(100, 110), HORIZON, "the horizon row is drawn");
    }

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

    fn first_hit(map: &Map, posx: f32, posy: f32, rdx: f32, rdy: f32) -> (f32, u8) {
        let mut found = (f32::INFINITY, 0);
        cast(map, posx, posy, rdx, rdy, |perp, side, _| {
            found = (perp, side);
            false
        });
        found
    }

    #[test]
    fn a_ray_north_hits_the_wall_at_its_distance() {
        let map = yard();
        // From the centre of cell (8, 4) looking north: the wall's south face
        // at y = 12 is 7.5 cells away.
        let (perp, side) = first_hit(&map, 8.5, 4.5, 0.0, 1.0);
        assert!((perp - 7.5).abs() < 1e-4, "{perp}");
        assert_eq!(side, 1);
        // Looking east: the map edge at x = 16, 7.5 cells away.
        let (perp, side) = first_hit(&map, 8.5, 4.5, 1.0, 0.0);
        assert!((perp - 7.5).abs() < 1e-4, "{perp}");
        assert_eq!(side, 0);
    }

    /// The sim's heights: a 3 m wall in front of 8 m woods shows the woods
    /// rising over it; the eye is at 1.5 m.
    #[test]
    fn woods_rise_over_a_low_wall_in_front_of_them() {
        let mut map = Map::new(32, 32, 1000);
        for y in 0..32 {
            for x in 0..32 {
                map.set(x, y, Ground::Grass);
            }
        }
        for x in 0..32 {
            map.set(x, 10, Ground::Wall);
            map.set(x, 14, Ground::WoodsHeavy);
        }
        let cam = Camera {
            x_cm: 16_500,
            y_cm: 4_500,
            heading_mdeg: 0,
        };
        let mut f = Frame::new(320, 220);
        render(&mut f, &map, &cam, &[]);
        let shades = Shades::new();
        let woods = (0..110)
            .any(|y| (0..FOG_STEPS).any(|s| f.get(160, y) == shades.shade(Base::WoodsHeavy, s)));
        assert!(woods, "the heavy woods show above the wall");
        assert_eq!(Ground::Wall.height_cm(), 300);
        assert_eq!(Ground::from_legend_name("woods_heavy").height_cm(), 800);
        assert_eq!(Ground::from_legend_name("woods_light").height_cm(), 600);
        assert_eq!(Ground::Grass.height_cm(), 0);
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
            Sprite::at_bearing(&cam, 10.0, 30.0, SpriteKind::Red),
            Sprite::at_bearing(&cam, 350.0, 50.0, SpriteKind::Blue),
        ];
        let mut frame = Frame::new(320, 220);
        render(&mut frame, &map, &cam, &sprites);
        // Top row is sky, the middle column has a wall slice around the
        // horizon, the bottom row is grass-ish floor.
        assert_eq!(frame.get(160, 0), sky(Biome::Temperate, 0));
        let mid = frame.get(160, 110);
        assert_ne!(mid, sky(Biome::Temperate, SKY_STEPS - 1));
        let floor = frame.get(160, 219);
        assert!(
            floor[1] > floor[0] && floor[1] > floor[2],
            "grass is green: {floor:?}"
        );
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

    /// A tank behind a wall taller than it is hidden by it — and, being a
    /// contact the sim reports, bracketed where it stands; a tank behind a
    /// lower wall shows its top over it.
    #[test]
    fn a_covered_contact_is_bracketed_and_a_turret_shows_over_a_low_wall() {
        let shades = Shades::new();
        let mut map = Map::new(32, 32, 1000);
        for y in 0..32 {
            for x in 0..32 {
                map.set(x, y, Ground::Grass);
            }
        }
        for x in 0..32 {
            map.set(x, 12, Ground::WoodsHeavy);
        }
        let cam = Camera {
            x_cm: 16_500,
            y_cm: 4_500,
            heading_mdeg: 0,
        };
        let behind = [Sprite::at_bearing(&cam, 0.0, 120.0, SpriteKind::Red)];
        let mut f = Frame::new(320, 220);
        render(&mut f, &map, &cam, &behind);
        let unfogged = sprite_colour(&shades, SpriteKind::Red, Material::Team, 0);
        let count = |f: &Frame, px: Rgb| {
            (0..f.width())
                .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
                .filter(|&(x, y)| f.get(x, y) == px)
                .count()
        };
        assert!(
            count(&f, unfogged) >= 8,
            "the bracket in its unfogged colour"
        );
        let fogged = sprite_colour(&shades, SpriteKind::Red, Material::Team, fog_step(11.5));
        assert_eq!(count(&f, fogged), 0, "no silhouette through 8 m woods");
        // A 3 m wall instead: the tank (3.5 m) shows its top over it.
        for x in 0..32 {
            map.set(x, 12, Ground::Wall);
        }
        render(&mut f, &map, &cam, &behind);
        let reddish = (0..320usize)
            .flat_map(|x| (0..220usize).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let [r, g, b] = f.get(x, y);
                r > g + 30 && r > b + 30
            })
            .count();
        assert!(reddish > 0, "the turret over the wall");
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

    /// The dev2 smoke of 2026-10-22: a contact drawn but too far to read —
    /// a few fogged pixels — is bracketed in its unfogged team colour too.
    #[test]
    fn a_far_contact_is_bracketed_even_when_drawn() {
        let mut map = Map::new(96, 96, 1000);
        for y in 0..96 {
            for x in 0..96 {
                map.set(x, y, Ground::Grass);
            }
        }
        let cam = Camera {
            x_cm: 48_000,
            y_cm: 10_000,
            heading_mdeg: 0,
        };
        let far = Sprite::at_bearing(&cam, 0.0, 550.0, SpriteKind::Blue);
        let mut f = Frame::new(640, 220);
        render(&mut f, &map, &cam, &[far]);
        let unfogged = sprite_colour(&Shades::new(), SpriteKind::Blue, Material::Team, 0);
        let marked = (0..f.width())
            .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| f.get(x, y) == unfogged)
            .count();
        assert!(marked >= 8, "bracketed: {marked} pixels");
    }

    /// The shade table, the sky and the sprites' paints together are the
    /// 256 colours a frame can hold, and no more (doc 22 cut U.3).
    #[test]
    fn the_palette_budget_is_256() {
        assert_eq!(
            Base::COUNT * FOG_STEPS + SKY_STEPS + Paint::COUNT * SPRITE_FOG,
            256
        );
    }

    /// Doc 22 cut U.3: a unit with a sheet is drawn from it — in its team's
    /// colour, its left face the right one mirrored, a shadow under it — and
    /// one the picture covers is still bracketed.
    #[test]
    fn a_sheet_sprite_draws_in_team_colour_mirrored_with_a_shadow() {
        let sheet = crate::look::for_unit("tacsim.unit.line_tank")
            .and_then(|l| l.sprites.as_ref())
            .expect("the tank's sheet");
        let mut map = Map::new(64, 64, 1000);
        for y in 0..64 {
            for x in 0..64 {
                map.set(x, y, Ground::Grass);
            }
        }
        let cam = Camera {
            x_cm: 32_000,
            y_cm: 10_000,
            heading_mdeg: 0,
        };
        let shades = Shades::new();
        let draw = |kind, facing| {
            let tank = Sprite {
                pose: Some(SpritePose {
                    sheet,
                    facing,
                    moving: false,
                    step: 0,
                }),
                ..Sprite::at_bearing(&cam, 0.0, 30.0, kind)
            };
            let mut f = Frame::new(320, 128);
            render(&mut f, &map, &cam, &[tank]);
            f
        };
        let count = |f: &Frame, px: Rgb| {
            (0..f.width())
                .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
                .filter(|&(x, y)| f.get(x, y) == px)
                .count()
        };
        let step = fog_step(3.0);
        let right = draw(SpriteKind::Red, Facing::Right);
        let red = sprite_colour(&shades, SpriteKind::Red, Material::Team, step);
        assert!(count(&right, red) > 20, "red-team body");
        let blue = draw(SpriteKind::Blue, Facing::Right);
        assert_eq!(count(&blue, red), 0, "a blue tank has no red in it");
        assert!(
            count(
                &blue,
                sprite_colour(&shades, SpriteKind::Blue, Material::Team, step)
            ) > 20
        );
        assert!(
            count(&right, shades.paint(Paint::Shadow, step)) > 0,
            "a shadow"
        );
        // The left face is the right one mirrored about the anchor column.
        let left = draw(SpriteKind::Red, Facing::Left);
        let columns = |f: &Frame| -> Vec<usize> {
            (0..f.width())
                .map(|x| (0..f.height()).filter(|&y| f.get(x, y) == red).count())
                .collect()
        };
        let (r, l) = (columns(&right), columns(&left));
        let (rmin, rmax) = (
            r.iter().position(|&n| n > 0).expect("drawn"),
            r.iter().rposition(|&n| n > 0).expect("drawn"),
        );
        let (lmin, lmax) = (
            l.iter().position(|&n| n > 0).expect("drawn"),
            l.iter().rposition(|&n| n > 0).expect("drawn"),
        );
        assert!(
            (rmax - rmin).abs_diff(lmax - lmin) <= 1,
            "the same width, to the pixel the sampling rounds"
        );
        assert_ne!(r, l, "but facing the other way");
        // A wreck has no team colour.
        let wreck = draw(SpriteKind::Wreck, Facing::Right);
        assert_eq!(count(&wreck, red), 0);
    }
}
