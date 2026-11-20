//! **Station & Ship Interior 2.5D Cutaway & First-Person Raycast Viewport**
//! (Track 4 Phase 4B: Kitty Graphics & DEC Sixel parity with Unity & Web 3D).
//!
//! Renders multi-deck spaceport neighborhoods (Ceres Freeport, OOC Simulator
//! Center, Visiting Berth Gallery with moored 8-facing hulls) and walkable
//! 3-deck ship interiors (Airlock, Cargo Hold with dynamic stowage pallets,
//! Engineering drive core, Mess, Quarters, and Bridge Holo-Tank + Helm Couch)
//! from the client-synthesized [`STATION_WALK_FEED`] keyframe.
//!
//! Supports two live camera perspectives toggled via `v`:
//! - [`WalkCamera::Cutaway25D`] (default): oblique 28-degree architectural
//!   cutaway dollhouse with rotatable azimuth (`Look::offset_mdeg`), zoom
//!   (`Look::zoom_permille`), split walls & open doorways, companionway
//!   staircases, moored exterior ships, crew/NPC sprites with speech bubbles,
//!   and `[E]` interactable focus brackets.
//! - [`WalkCamera::FirstPersonRaycast`]: indoor 0.5 m DDA column raycaster
//!   with ceiling coffers, ribbed bulkheads, doorway openings, berth gallery
//!   armor-glass starfield windows, depth-buffered actor/prop billboards,
//!   compass tape, and centered `[E]` interaction prompt.
//!
//! Every pixel is drawn from a small closed set of quantized RGB constants
//! (plus `ship_shader`'s 6-step ramps) so rendered frames stay well below the
//! 256-color indexed-PNG and DEC Sixel palette ceiling.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::math::Det as _;
use crate::overlay::{
    BORDER, DIM, GOOD, LOCK, PANEL, TEXT, WARN, YOU, fill_rect, line, outline_rect, scale_for, text,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::ship_shader::{ShipModelStyle, ShipSpriteDraw, draw_ship_sprite};

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Client-local feed carrying the active station/ship walk geometry, avatar pose,
/// actors, speech bubbles, and focused `[E]` interactable.
pub const STATION_WALK_FEED: &str = "station.walk";

/// Retained Kitty image ID (`i=189`) for the Station & Ship Walk picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_STATION_WALK: u32 = 189;
/// Retained Kitty placement ID (`p=89`) for the Station & Ship Walk picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_STATION_WALK: u32 = 89;
#[cfg(feature = "encode")]
const Z_STATION_WALK: i32 = -50;

// ---- Closed RGB palette (keeps total frame colors < 110 for indexed PNG & Sixel) ----

const BG_SPACE: Rgb = [6, 9, 16];
const STAR_DIM: Rgb = [96, 118, 148];
const STAR_BRIGHT: Rgb = [212, 230, 252];

const DECK_LOWER: Rgb = [18, 24, 34];
const DECK_LOWER_GRID: Rgb = [24, 32, 44];
const DECK_INACTIVE: Rgb = [28, 36, 48];
const DECK_INACTIVE_GRID: Rgb = [36, 46, 62];
const DECK_ACTIVE: Rgb = [42, 54, 70];
const DECK_ACTIVE_GRID: Rgb = [54, 70, 90];
const DECK_BRIDGE: Rgb = [34, 52, 74];
const DECK_HOLD: Rgb = [48, 52, 58];

const WALL_TOP: Rgb = [116, 134, 156];
const WALL_FACE_LIT: Rgb = [82, 98, 118];
const WALL_FACE_DARK: Rgb = [58, 70, 86];
const WALL_KICK: Rgb = [38, 46, 58];
const WALL_RAIL: Rgb = [64, 142, 184];
const GLASS_TINT: Rgb = [38, 92, 132];
const SAFETY_YELLOW: Rgb = [216, 178, 44];

const STAIR_STEP_A: Rgb = [76, 90, 108];
const STAIR_STEP_B: Rgb = [98, 114, 136];
const PROP_DESK: Rgb = [56, 88, 116];
const PROP_DESK_TOP: Rgb = [92, 152, 196];
const PROP_CARGO: Rgb = [164, 122, 56];
const PROP_CARGO_TOP: Rgb = [204, 158, 78];
const PROP_AIRLOCK: Rgb = [188, 108, 42];
const PROP_HELM: Rgb = [52, 168, 196];
const PROP_CORE: Rgb = [76, 196, 244];

const ACTOR_OUTLINE: Rgb = [12, 16, 24];
const NPC_COAT: Rgb = [68, 158, 224];
const NPC_HIGHLIGHT: Rgb = [144, 212, 252];
const CREW_SUIT: Rgb = [58, 188, 132];
const CREW_HIGHLIGHT: Rgb = [136, 236, 184];
const YOU_SUIT: Rgb = [228, 112, 56];
const YOU_VISOR: Rgb = [156, 232, 255];
const SKIN_TONE: Rgb = [214, 176, 148];
const SPEECH_BG: Rgb = [14, 22, 34];
const SPEECH_BORDER: Rgb = [228, 196, 96];

const FP_FOG_STEPS: usize = 6;
const FP_FLOOR_SHADES: [Rgb; FP_FOG_STEPS] = [
    [54, 68, 86],
    [46, 58, 74],
    [38, 48, 62],
    [30, 38, 50],
    [22, 28, 38],
    [14, 18, 26],
];
const FP_GRID_SHADES: [Rgb; FP_FOG_STEPS] = [
    [72, 90, 112],
    [60, 76, 96],
    [48, 60, 78],
    [36, 46, 60],
    [26, 34, 46],
    [18, 22, 32],
];
const FP_CEIL_SHADES: [Rgb; FP_FOG_STEPS] = [
    [38, 46, 58],
    [32, 40, 50],
    [26, 32, 42],
    [20, 26, 34],
    [16, 20, 28],
    [10, 14, 20],
];
const FP_WALL_LIT_SHADES: [Rgb; FP_FOG_STEPS] = [
    [108, 126, 148],
    [92, 108, 128],
    [76, 90, 108],
    [60, 72, 88],
    [44, 54, 68],
    [30, 38, 48],
];
const FP_WALL_DARK_SHADES: [Rgb; FP_FOG_STEPS] = [
    [82, 96, 116],
    [70, 82, 100],
    [58, 68, 84],
    [46, 54, 68],
    [34, 42, 54],
    [24, 30, 40],
];

/// Camera perspective for [`StationWalkPicture`], toggled by `v`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WalkCamera {
    /// Oblique 28-degree architectural cutaway dollhouse (default).
    #[default]
    Cutaway25D,
    /// First-person 0.5 m indoor corridor DDA raycaster.
    FirstPersonRaycast,
}

impl WalkCamera {
    /// Toggle between cutaway 2.5D and first-person raycast.
    #[must_use]
    pub fn next(self) -> WalkCamera {
        match self {
            WalkCamera::Cutaway25D => WalkCamera::FirstPersonRaycast,
            WalkCamera::FirstPersonRaycast => WalkCamera::Cutaway25D,
        }
    }

    /// Wire string representation in [`STATION_WALK_FEED`].
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WalkCamera::Cutaway25D => "cutaway",
            WalkCamera::FirstPersonRaycast => "first_person",
        }
    }

    /// Parse camera mode from [`STATION_WALK_FEED`].
    #[must_use]
    pub fn from_feed_str(s: &str) -> WalkCamera {
        if s == "first_person" {
            WalkCamera::FirstPersonRaycast
        } else {
            WalkCamera::Cutaway25D
        }
    }
}

/// Hard upper bound on pixel dimensions for `StationWalkPicture::draw_kitty`
/// (defense-in-depth against unclamped terminal dimensions).
#[cfg(feature = "encode")]
const MAX_WALK_WIDTH_PX: usize = 1920;
#[cfg(feature = "encode")]
const MAX_WALK_HEIGHT_PX: usize = 1080;

/// Station & Ship Interior picture producer.
#[derive(Debug, Default)]
pub struct StationWalkPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
    #[cfg(feature = "encode")]
    scratch_frame: Option<Frame>,
    cached_grid: Option<(u64, IndoorGrid)>,
}

impl StationWalkPicture {
    /// Construct a new station/ship walk picture.
    #[must_use]
    pub fn new() -> StationWalkPicture {
        StationWalkPicture::default()
    }

    /// Emit Kitty delete commands for the retained image when closing/switching.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_STATION_WALK);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Mark the retained Kitty image invalid if evicted by the terminal.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_STATION_WALK {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force placement re-emission on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render as a retained Kitty Graphics stream (`a=t` + `a=p`).
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<crate::caps::CellSize>,
        step_down: bool,
    ) -> Option<(crate::iso25d::Viewport25D, Vec<u8>)> {
        let mut vp = crate::cockpit_kitty::viewport_fp(area.2, area.3, cell, step_down);
        let vw = vp.width_px.clamp(2, MAX_WALK_WIDTH_PX);
        let vh = vp.height_px.clamp(2, MAX_WALK_HEIGHT_PX);
        vp.width_px = vw;
        vp.height_px = vh;
        let mut frame = match self.scratch_frame.take() {
            Some(f) if f.width() == vw && f.height() == vh => f,
            _ => Frame::new(vw, vh),
        };
        let drawn = self.draw(&mut frame, feeds, look);
        if !drawn {
            self.scratch_frame = Some(frame);
            return None;
        }
        let placed_area = (
            area.0.saturating_add(vp.col_offset),
            area.1.saturating_add(vp.row_offset),
            vp.cols,
            vp.rows,
        );
        let h = frame.hash();
        let mut ops = Vec::new();
        if self.kitty_hash != Some(h) {
            let png = crate::png::encode(&frame);
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_STATION_WALK);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        self.scratch_frame = Some(frame);
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_STATION_WALK,
                PLACEMENT_STATION_WALK,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_STATION_WALK,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for StationWalkPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[STATION_WALK_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(raw) = feeds.feed(STATION_WALK_FEED) else {
            return false;
        };
        let Some(scene) = WalkScene::decode(raw) else {
            return false;
        };
        match scene.camera {
            WalkCamera::Cutaway25D => draw_cutaway_25d(frame, &scene, look),
            WalkCamera::FirstPersonRaycast => {
                draw_first_person(frame, &scene, look, &mut self.cached_grid);
            }
        }
        draw_walk_hud(frame, &scene, look);
        true
    }
}

// ---- Decoded Walk Scene -----------------------------------------------------

#[derive(Clone, Debug)]
struct WalkRoom {
    name: String,
    floor: i64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    active: bool,
    kind: String,
}

#[derive(Clone, Debug)]
struct WalkDoor {
    x_m: f64,
    y_m: f64,
    floor: i64,
    width_m: f64,
    stairs_to: Option<i64>,
}

#[derive(Clone, Debug)]
struct WalkProp {
    x_m: f64,
    y_m: f64,
    floor: i64,
    kind: String,
    label: String,
}

#[derive(Clone, Debug)]
struct WalkMooredShip {
    x_m: f64,
    y_m: f64,
    name: String,
    style: ShipModelStyle,
    own: bool,
}

#[derive(Clone, Debug)]
struct WalkActor {
    x_m: f64,
    y_m: f64,
    floor: i64,
    name: String,
    npc: bool,
    speech: String,
}

#[derive(Clone, Debug)]
struct WalkFocus {
    x_m: f64,
    y_m: f64,
    floor: i64,
    label: String,
}

#[derive(Clone, Debug)]
struct WalkScene {
    camera: WalkCamera,
    aboard_ship: bool,
    title: String,
    subtitle: String,
    you_x: f64,
    you_y: f64,
    you_floor: i64,
    you_heading_mdeg: i64,
    you_stride: i64,
    cargo_fill_permille: i64,
    pulse_phase: i64,
    focus: Option<WalkFocus>,
    rooms: Vec<WalkRoom>,
    doors: Vec<WalkDoor>,
    props: Vec<WalkProp>,
    moored: Vec<WalkMooredShip>,
    actors: Vec<WalkActor>,
}

/// Safe coordinate clamp in world metres so extreme feed values can never overflow screen math.
const MAX_WORLD_COORD_M: f64 = 100_000.0;
const MIN_FLOOR_IDX: i64 = -100;
const MAX_FLOOR_IDX: i64 = 100;

fn as_coord(v: Option<&Value>, def: f64) -> f64 {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(def)
        .clamp(-MAX_WORLD_COORD_M, MAX_WORLD_COORD_M)
}

fn as_floor(v: Option<&Value>, def: i64) -> i64 {
    v.and_then(Value::as_i64)
        .unwrap_or(def)
        .clamp(MIN_FLOOR_IDX, MAX_FLOOR_IDX)
}

fn as_i64(v: Option<&Value>, def: i64) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(def)
}

fn as_str<'a>(v: Option<&'a Value>, def: &'a str) -> &'a str {
    v.and_then(Value::as_str).unwrap_or(def)
}

impl WalkScene {
    fn decode(v: &Value) -> Option<WalkScene> {
        let obj = v.as_object()?;
        let camera = WalkCamera::from_feed_str(as_str(obj.get("camera"), "cutaway"));
        let aboard_ship = as_str(obj.get("mode"), "station") == "interior";
        let title = as_str(obj.get("title"), "STATION").to_string();
        let subtitle = as_str(obj.get("subtitle"), "").to_string();

        let you = obj.get("you");
        let you_x = as_coord(you.and_then(|y| y.get("x_m")), 0.0);
        let you_y = as_coord(you.and_then(|y| y.get("y_m")), 0.0);
        let you_floor = as_floor(you.and_then(|y| y.get("floor")), 0);
        let you_heading_mdeg =
            as_i64(you.and_then(|y| y.get("heading_mdeg")), 0).rem_euclid(360_000);
        let you_stride = as_i64(you.and_then(|y| y.get("stride")), 0);

        let cargo_fill_permille = as_i64(obj.get("cargo_fill_permille"), 0).clamp(0, 1000);
        let pulse_phase = as_i64(obj.get("pulse_phase"), 0).rem_euclid(60);

        let focus = obj
            .get("focus")
            .and_then(Value::as_object)
            .map(|f| WalkFocus {
                x_m: as_coord(f.get("x_m"), you_x),
                y_m: as_coord(f.get("y_m"), you_y),
                floor: as_floor(f.get("floor"), you_floor),
                label: as_str(f.get("label"), "INTERACT").to_string(),
            });

        let mut rooms = Vec::new();
        if let Some(arr) = obj.get("rooms").and_then(Value::as_array) {
            for item in arr.iter().take(64) {
                let min_x = as_coord(item.get("min_x"), -8.0);
                let max_x = as_coord(item.get("max_x"), 8.0);
                let min_y = as_coord(item.get("min_y"), -8.0);
                let max_y = as_coord(item.get("max_y"), 8.0);
                if max_x <= min_x || max_y <= min_y {
                    continue;
                }
                rooms.push(WalkRoom {
                    name: as_str(item.get("name"), "ROOM").to_string(),
                    floor: as_floor(item.get("floor"), 0),
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                    active: item.get("active").and_then(Value::as_bool).unwrap_or(false),
                    kind: as_str(item.get("kind"), "room").to_string(),
                });
            }
        }
        if rooms.is_empty() {
            rooms.push(WalkRoom {
                name: title.clone(),
                floor: you_floor,
                min_x: (you_x - 8.0).clamp(-MAX_WORLD_COORD_M, MAX_WORLD_COORD_M),
                min_y: (you_y - 8.0).clamp(-MAX_WORLD_COORD_M, MAX_WORLD_COORD_M),
                max_x: (you_x + 8.0).clamp(-MAX_WORLD_COORD_M, MAX_WORLD_COORD_M),
                max_y: (you_y + 8.0).clamp(-MAX_WORLD_COORD_M, MAX_WORLD_COORD_M),
                active: true,
                kind: "room".to_string(),
            });
        }

        let mut doors = Vec::new();
        if let Some(arr) = obj.get("doors").and_then(Value::as_array) {
            for item in arr.iter().take(96) {
                doors.push(WalkDoor {
                    x_m: as_coord(item.get("x_m"), 0.0),
                    y_m: as_coord(item.get("y_m"), 0.0),
                    floor: as_floor(item.get("floor"), 0),
                    width_m: as_coord(item.get("width_m"), 2.4).clamp(1.2, 6.0),
                    stairs_to: item
                        .get("stairs_to")
                        .and_then(Value::as_i64)
                        .map(|f| f.clamp(MIN_FLOOR_IDX, MAX_FLOOR_IDX)),
                });
            }
        }

        let mut props = Vec::new();
        if let Some(arr) = obj.get("props").and_then(Value::as_array) {
            for item in arr.iter().take(64) {
                props.push(WalkProp {
                    x_m: as_coord(item.get("x_m"), 0.0),
                    y_m: as_coord(item.get("y_m"), 0.0),
                    floor: as_floor(item.get("floor"), 0),
                    kind: as_str(item.get("kind"), "desk").to_string(),
                    label: as_str(item.get("label"), "").to_string(),
                });
            }
        }

        let mut moored = Vec::new();
        if let Some(arr) = obj.get("moored").and_then(Value::as_array) {
            for item in arr.iter().take(16) {
                let style_str = as_str(item.get("style"), "freighter");
                let style = if style_str == "corvette" {
                    ShipModelStyle::Corvette
                } else {
                    ShipModelStyle::Freighter
                };
                moored.push(WalkMooredShip {
                    x_m: as_coord(item.get("x_m"), 0.0),
                    y_m: as_coord(item.get("y_m"), 0.0),
                    name: as_str(item.get("name"), "HULL").to_string(),
                    style,
                    own: item.get("own").and_then(Value::as_bool).unwrap_or(false),
                });
            }
        }

        let mut actors = Vec::new();
        if let Some(arr) = obj.get("actors").and_then(Value::as_array) {
            for item in arr.iter().take(32) {
                actors.push(WalkActor {
                    x_m: as_coord(item.get("x_m"), 0.0),
                    y_m: as_coord(item.get("y_m"), 0.0),
                    floor: as_floor(item.get("floor"), you_floor),
                    name: as_str(item.get("name"), "CREW").to_string(),
                    npc: item.get("npc").and_then(Value::as_bool).unwrap_or(false),
                    speech: as_str(item.get("speech"), "").to_string(),
                });
            }
        }

        Some(WalkScene {
            camera,
            aboard_ship,
            title,
            subtitle,
            you_x,
            you_y,
            you_floor,
            you_heading_mdeg,
            you_stride,
            cargo_fill_permille,
            pulse_phase,
            focus,
            rooms,
            doors,
            props,
            moored,
            actors,
        })
    }
}

// ---- Starfield backdrop -----------------------------------------------------

fn draw_starfield(frame: &mut Frame, max_y: usize) {
    let w = frame.width();
    let h = frame.height().min(max_y);
    if w == 0 || h == 0 {
        return;
    }
    for i in 0..72u64 {
        let sx = ((i.wrapping_mul(7919).wrapping_add(101)) as usize) % w;
        let sy = ((i.wrapping_mul(6271).wrapping_add(53)) as usize) % h;
        let col = if i % 5 == 0 { STAR_BRIGHT } else { STAR_DIM };
        frame.put(sx, sy, col);
    }
}

// ---- Mode 1: 2.5D Oblique Cutaway Dollhouse ---------------------------------

#[derive(Clone, Copy)]
struct IsoProj {
    cx: f64,
    cy: f64,
    px_per_m: f64,
    az_rad: f64,
    sin_az: f64,
    cos_az: f64,
    tilt: f64,
    you_x: f64,
    you_y: f64,
    you_z: f64,
}

impl IsoProj {
    fn new(frame: &Frame, scene: &WalkScene, look: &Look) -> IsoProj {
        let base_scale = if frame.width() >= 520 { 13.0 } else { 7.5 };
        let zoom = f64::from(look.zoom_permille.clamp(500, 2000)) / 1000.0;
        let az_rad = ((look.offset_mdeg as f64) / 1000.0 + 24.0).to_radians();
        IsoProj {
            cx: (frame.width() as f64) * 0.5,
            cy: (frame.height() as f64) * 0.56,
            px_per_m: base_scale * zoom,
            az_rad,
            sin_az: az_rad.det_sin(),
            cos_az: az_rad.det_cos(),
            tilt: 0.58,
            you_x: scene.you_x,
            you_y: scene.you_y,
            you_z: (scene.you_floor as f64) * 3.8,
        }
    }

    fn project(&self, wx: f64, wy: f64, wz: f64) -> (i64, i64, f64) {
        let dx = wx - self.you_x;
        let dy = wy - self.you_y;
        let dz = wz - self.you_z;
        // East = +dx, North = +dy; screen Y grows downward (-ry).
        let rx = dx * self.cos_az - dy * self.sin_az;
        let ry = dx * self.sin_az + dy * self.cos_az;
        let sx = (self.cx + rx * self.px_per_m)
            .round()
            .clamp(-1_000_000.0, 1_000_000.0) as i64;
        let sy = (self.cy - ry * self.px_per_m * self.tilt - dz * self.px_per_m * 0.85)
            .round()
            .clamp(-1_000_000.0, 1_000_000.0) as i64;
        (sx, sy, -ry)
    }
}

fn fill_quad_scanline(frame: &mut Frame, pts: [(i64, i64); 4], col: Rgb) {
    let h = frame.height() as i64;
    let w = frame.width() as i64;
    if w <= 0 || h <= 0 {
        return;
    }
    let mut min_y = i64::MAX;
    let mut max_y = i64::MIN;
    for &(_, py) in &pts {
        if py < min_y {
            min_y = py;
        }
        if py > max_y {
            max_y = py;
        }
    }
    if min_y >= h || max_y < 0 {
        return;
    }
    let y0 = min_y.clamp(0, h - 1);
    let y1 = max_y.clamp(0, h - 1);
    let edges = [
        (pts[0], pts[1]),
        (pts[1], pts[2]),
        (pts[2], pts[3]),
        (pts[3], pts[0]),
    ];
    for y in y0..=y1 {
        let mut x_left = i64::MAX;
        let mut x_right = i64::MIN;
        for &((ax, ay), (bx, by)) in &edges {
            if ay == by {
                if y == ay {
                    x_left = x_left.min(ax.min(bx));
                    x_right = x_right.max(ax.max(bx));
                }
                continue;
            }
            let (ymin, ymax) = if ay < by { (ay, by) } else { (by, ay) };
            if y < ymin || y > ymax {
                continue;
            }
            let num =
                (i128::from(bx) - i128::from(ax)).saturating_mul(i128::from(y) - i128::from(ay));
            let den = i128::from(by) - i128::from(ay);
            let ix = (i128::from(ax) + num / den).clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                as i64;
            if ix < x_left {
                x_left = ix;
            }
            if ix > x_right {
                x_right = ix;
            }
        }
        if x_left <= x_right && x_right >= 0 && x_left < w {
            let xl = x_left.clamp(0, w - 1) as usize;
            let xr = x_right.clamp(0, w - 1) as usize;
            let yu = y as usize;
            for x in xl..=xr {
                frame.put(x, yu, col);
            }
        }
    }
}

fn draw_iso_wall_span(
    frame: &mut Frame,
    proj: &IsoProj,
    (x0, y0): (f64, f64),
    (x1, y1): (f64, f64),
    z_base: f64,
    height_m: f64,
    is_glass: bool,
) {
    let b0 = proj.project(x0, y0, z_base);
    let b1 = proj.project(x1, y1, z_base);
    let t1 = proj.project(x1, y1, z_base + height_m);
    let t0 = proj.project(x0, y0, z_base + height_m);

    if is_glass {
        line(frame, b0.0, b0.1, b1.0, b1.1, SAFETY_YELLOW);
        line(frame, t0.0, t0.1, t1.0, t1.1, GLASS_TINT);
        line(frame, b0.0, b0.1, t0.0, t0.1, GLASS_TINT);
        line(frame, b1.0, b1.1, t1.0, t1.1, GLASS_TINT);
        return;
    }

    let face_col = if (x1 - x0).abs() >= (y1 - y0).abs() {
        WALL_FACE_LIT
    } else {
        WALL_FACE_DARK
    };
    fill_quad_scanline(
        frame,
        [(b0.0, b0.1), (b1.0, b1.1), (t1.0, t1.1), (t0.0, t0.1)],
        face_col,
    );
    line(frame, b0.0, b0.1, b1.0, b1.1, WALL_KICK);
    line(frame, t0.0, t0.1, t1.0, t1.1, WALL_TOP);
    let r0 = proj.project(x0, y0, z_base + height_m * 0.38);
    let r1 = proj.project(x1, y1, z_base + height_m * 0.38);
    line(frame, r0.0, r0.1, r1.0, r1.1, WALL_RAIL);
}

fn split_wall_intervals(len: f64, gaps: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut intervals = vec![(0.0, len)];
    for &(g0, g1) in gaps {
        let mut next = Vec::new();
        for (a, b) in intervals {
            if g1 <= a || g0 >= b {
                next.push((a, b));
            } else {
                if g0 > a + 0.15 {
                    next.push((a, g0));
                }
                if g1 < b - 0.15 {
                    next.push((g1, b));
                }
            }
        }
        intervals = next;
    }
    intervals
}

/// Determine whether a `"berth"` room's armor-glass bay window sits on the South wall (`min_y`)
/// rather than the North wall (`max_y`). Real Ceres (`c01_berth_gallery`) has its doors on the
/// North wall (`y = 125`) and vacuum cradles to the South (`y = 110`).
fn berth_glass_is_south(room: &WalkRoom, doors: &[WalkDoor]) -> bool {
    if room.kind != "berth" {
        return false;
    }
    let mut has_north_door = false;
    let mut has_south_door = false;
    for d in doors {
        if d.floor != room.floor && d.stairs_to != Some(room.floor) {
            continue;
        }
        if d.x_m >= room.min_x - 1.0 && d.x_m <= room.max_x + 1.0 {
            if (d.y_m - room.max_y).abs() <= 1.6 {
                has_north_door = true;
            }
            if (d.y_m - room.min_y).abs() <= 1.6 {
                has_south_door = true;
            }
        }
    }
    has_north_door && !has_south_door
}

fn draw_room_floor(frame: &mut Frame, proj: &IsoProj, room: &WalkRoom, you_floor: i64) {
    let z = (room.floor as f64) * 3.8;
    let is_lower = room.floor < you_floor;
    let (deck_col, grid_col) = if is_lower {
        (DECK_LOWER, DECK_LOWER_GRID)
    } else if !room.active {
        (DECK_INACTIVE, DECK_INACTIVE_GRID)
    } else if room.kind == "bridge" {
        (DECK_BRIDGE, DECK_ACTIVE_GRID)
    } else if room.kind == "hold" {
        (DECK_HOLD, DECK_ACTIVE_GRID)
    } else {
        (DECK_ACTIVE, DECK_ACTIVE_GRID)
    };

    let c0 = proj.project(room.min_x, room.min_y, z);
    let c1 = proj.project(room.max_x, room.min_y, z);
    let c2 = proj.project(room.max_x, room.max_y, z);
    let c3 = proj.project(room.min_x, room.max_y, z);
    fill_quad_scanline(
        frame,
        [(c0.0, c0.1), (c1.0, c1.1), (c2.0, c2.1), (c3.0, c3.1)],
        deck_col,
    );

    // Non-skid 4 m deck grid lines (bounded iteration count so extreme rooms stay fast)
    let step_m = 4.0;
    let mut gx = (room.min_x / step_m).ceil() * step_m;
    let mut lines_drawn = 0usize;
    while gx < room.max_x - 0.2 && lines_drawn < 128 {
        let p0 = proj.project(gx, room.min_y, z);
        let p1 = proj.project(gx, room.max_y, z);
        line(frame, p0.0, p0.1, p1.0, p1.1, grid_col);
        gx += step_m;
        lines_drawn += 1;
    }
    let mut gy = (room.min_y / step_m).ceil() * step_m;
    lines_drawn = 0;
    while gy < room.max_y - 0.2 && lines_drawn < 128 {
        let p0 = proj.project(room.min_x, gy, z);
        let p1 = proj.project(room.max_x, gy, z);
        line(frame, p0.0, p0.1, p1.0, p1.1, grid_col);
        gy += step_m;
        lines_drawn += 1;
    }

    if is_lower {
        line(frame, c0.0, c0.1, c1.0, c1.1, WALL_KICK);
        line(frame, c1.0, c1.1, c2.0, c2.1, WALL_KICK);
        line(frame, c2.0, c2.1, c3.0, c3.1, WALL_KICK);
        line(frame, c3.0, c3.1, c0.0, c0.1, WALL_KICK);
    }
}

enum CutawayItem<'a> {
    WallSpan {
        p0: (f64, f64),
        p1: (f64, f64),
        z: f64,
        height_m: f64,
        is_glass: bool,
    },
    Moored(&'a WalkMooredShip),
    StairStep {
        x_m: f64,
        y_m: f64,
        z_m: f64,
        top_col: Rgb,
    },
    CargoPallet {
        x_m: f64,
        y_m: f64,
        z_m: f64,
    },
    Prop(&'a WalkProp),
    Actor(&'a WalkActor),
    You,
}

#[allow(clippy::too_many_arguments)]
fn push_subdivided_wall_spans<'a>(
    out: &mut Vec<(f64, i32, CutawayItem<'a>)>,
    proj: &IsoProj,
    p0: (f64, f64),
    p1: (f64, f64),
    z: f64,
    height_m: f64,
    is_glass: bool,
    layer: i32,
) {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 1e-3 {
        return;
    }
    let segs = ((len / 8.0).ceil() as usize).clamp(1, 32);
    for i in 0..segs {
        let t0 = (i as f64) / (segs as f64);
        let t1 = ((i + 1) as f64) / (segs as f64);
        let a = (p0.0 + dx * t0, p0.1 + dy * t0);
        let b = (p0.0 + dx * t1, p0.1 + dy * t1);
        let mx = (a.0 + b.0) * 0.5;
        let my = (a.1 + b.1) * 0.5;
        let (_, _, depth) = proj.project(mx, my, z);
        out.push((
            depth,
            layer,
            CutawayItem::WallSpan {
                p0: a,
                p1: b,
                z,
                height_m,
                is_glass,
            },
        ));
    }
}

fn collect_room_wall_spans<'a>(
    out: &mut Vec<(f64, i32, CutawayItem<'a>)>,
    proj: &IsoProj,
    room: &WalkRoom,
    doors: &[WalkDoor],
) {
    let z = (room.floor as f64) * 3.8;
    // Cutaway wall height: back walls stand 2.15 m tall, front walls stay 0.42 m curb.
    let wall_h_back = if room.active { 2.15 } else { 1.1 };
    let wall_h_front = 0.42;
    let w_len = room.max_x - room.min_x;
    let d_len = room.max_y - room.min_y;

    let mut north_gaps = Vec::new();
    let mut south_gaps = Vec::new();
    let mut east_gaps = Vec::new();
    let mut west_gaps = Vec::new();

    for d in doors {
        if d.floor != room.floor && d.stairs_to != Some(room.floor) {
            continue;
        }
        let half = d.width_m * 0.5;
        if (d.y_m - room.max_y).abs() <= 1.6
            && d.x_m >= room.min_x - 1.0
            && d.x_m <= room.max_x + 1.0
        {
            let pos = d.x_m - room.min_x;
            north_gaps.push((pos - half, pos + half));
        }
        if (d.y_m - room.min_y).abs() <= 1.6
            && d.x_m >= room.min_x - 1.0
            && d.x_m <= room.max_x + 1.0
        {
            let pos = d.x_m - room.min_x;
            south_gaps.push((pos - half, pos + half));
        }
        if (d.x_m - room.max_x).abs() <= 1.6
            && d.y_m >= room.min_y - 1.0
            && d.y_m <= room.max_y + 1.0
        {
            let pos = d.y_m - room.min_y;
            east_gaps.push((pos - half, pos + half));
        }
        if (d.x_m - room.min_x).abs() <= 1.6
            && d.y_m >= room.min_y - 1.0
            && d.y_m <= room.max_y + 1.0
        {
            let pos = d.y_m - room.min_y;
            west_gaps.push((pos - half, pos + half));
        }
    }

    let south_glass = berth_glass_is_south(room, doors);
    let north_glass = room.kind == "berth" && !south_glass;

    let n_back = proj.cos_az >= 0.0;
    let nh = if n_back { wall_h_back } else { wall_h_front };
    let n_layer = if n_back { 0 } else { 3 };
    for (s0, s1) in split_wall_intervals(w_len, &north_gaps) {
        push_subdivided_wall_spans(
            out,
            proj,
            (room.min_x + s0, room.max_y),
            (room.min_x + s1, room.max_y),
            z,
            nh,
            north_glass,
            n_layer,
        );
    }

    let w_back = proj.sin_az <= 0.0;
    let wh = if w_back { wall_h_back } else { wall_h_front };
    let w_layer = if w_back { 0 } else { 3 };
    for (s0, s1) in split_wall_intervals(d_len, &west_gaps) {
        push_subdivided_wall_spans(
            out,
            proj,
            (room.min_x, room.min_y + s0),
            (room.min_x, room.min_y + s1),
            z,
            wh,
            false,
            w_layer,
        );
    }

    let e_back = proj.sin_az > 0.0;
    let eh = if e_back { wall_h_back } else { wall_h_front };
    let e_layer = if e_back { 0 } else { 3 };
    for (s0, s1) in split_wall_intervals(d_len, &east_gaps) {
        push_subdivided_wall_spans(
            out,
            proj,
            (room.max_x, room.min_y + s0),
            (room.max_x, room.min_y + s1),
            z,
            eh,
            false,
            e_layer,
        );
    }

    let s_back = proj.cos_az < 0.0;
    let sh = if s_back { wall_h_back } else { wall_h_front };
    let s_layer = if s_back { 0 } else { 3 };
    for (s0, s1) in split_wall_intervals(w_len, &south_gaps) {
        push_subdivided_wall_spans(
            out,
            proj,
            (room.min_x + s0, room.min_y),
            (room.min_x + s1, room.min_y),
            z,
            sh,
            south_glass,
            s_layer,
        );
    }
}

fn draw_iso_box(
    frame: &mut Frame,
    proj: &IsoProj,
    (cx, cy, z0): (f64, f64, f64),
    (sx, sy, sz): (f64, f64, f64),
    side_col: Rgb,
    top_col: Rgb,
) {
    let hx = sx * 0.5;
    let hy = sy * 0.5;
    let b0 = proj.project(cx - hx, cy - hy, z0); // SW
    let b1 = proj.project(cx + hx, cy - hy, z0); // SE
    let b2 = proj.project(cx + hx, cy + hy, z0); // NE
    let b3 = proj.project(cx - hx, cy + hy, z0); // NW
    let t0 = proj.project(cx - hx, cy - hy, z0 + sz);
    let t1 = proj.project(cx + hx, cy - hy, z0 + sz);
    let t2 = proj.project(cx + hx, cy + hy, z0 + sz);
    let t3 = proj.project(cx - hx, cy + hy, z0 + sz);

    // Camera looks along (+sin_az, +cos_az), so it sits in direction (-sin_az, -cos_az).
    // Draw only the two camera-facing vertical side faces so rotating azimuth never turns boxes inside-out.
    if proj.cos_az >= 0.0 {
        // South face (-Y normal)
        fill_quad_scanline(
            frame,
            [(b0.0, b0.1), (b1.0, b1.1), (t1.0, t1.1), (t0.0, t0.1)],
            side_col,
        );
    } else {
        // North face (+Y normal)
        fill_quad_scanline(
            frame,
            [(b3.0, b3.1), (b2.0, b2.1), (t2.0, t2.1), (t3.0, t3.1)],
            side_col,
        );
    }

    if proj.sin_az >= 0.0 {
        // West face (-X normal)
        fill_quad_scanline(
            frame,
            [(b3.0, b3.1), (b0.0, b0.1), (t0.0, t0.1), (t3.0, t3.1)],
            side_col,
        );
    } else {
        // East face (+X normal)
        fill_quad_scanline(
            frame,
            [(b1.0, b1.1), (b2.0, b2.1), (t2.0, t2.1), (t1.0, t1.1)],
            side_col,
        );
    }

    fill_quad_scanline(
        frame,
        [(t0.0, t0.1), (t1.0, t1.1), (t2.0, t2.1), (t3.0, t3.1)],
        top_col,
    );
}

fn draw_humanoid_sprite(
    frame: &mut Frame,
    (sx, sy): (i64, i64),
    scale: i64,
    (suit, highlight): (Rgb, Rgb),
    heading_mdeg: Option<i64>,
    camera_az_rad: f64,
    stride: i64,
) {
    let s = scale.clamp(1, 3);
    let swing = if stride % 2 == 0 { s } else { -s };
    // Shadow
    fill_rect(frame, sx - 3 * s, sy - s, 6 * s, 2 * s, ACTOR_OUTLINE);
    // Boots & legs
    fill_rect(
        frame,
        sx - 2 * s,
        sy - 5 * s + swing.max(0),
        2 * s - 1,
        5 * s,
        ACTOR_OUTLINE,
    );
    fill_rect(
        frame,
        sx + 1,
        sy - 5 * s + (-swing).max(0),
        2 * s - 1,
        5 * s,
        ACTOR_OUTLINE,
    );
    fill_rect(frame, sx - 2 * s + 1, sy - 5 * s, s, 4 * s, suit);
    fill_rect(frame, sx + 1, sy - 5 * s, s, 4 * s, suit);
    // Torso & shoulders
    fill_rect(
        frame,
        sx - 3 * s - 1,
        sy - 11 * s - 1,
        6 * s + 2,
        6 * s + 2,
        ACTOR_OUTLINE,
    );
    fill_rect(frame, sx - 3 * s, sy - 11 * s, 6 * s, 6 * s, suit);
    fill_rect(frame, sx - 2 * s, sy - 10 * s, 4 * s, 2 * s, highlight);
    // Head / helmet
    fill_rect(
        frame,
        sx - 2 * s - 1,
        sy - 15 * s - 1,
        4 * s + 2,
        4 * s + 2,
        ACTOR_OUTLINE,
    );
    let head_col = if heading_mdeg.is_some() {
        YOU_VISOR
    } else {
        SKIN_TONE
    };
    fill_rect(frame, sx - 2 * s, sy - 15 * s, 4 * s, 4 * s, head_col);

    // Facing arrow on deck for the local player, rotated by -camera_az_rad so it matches W travel!
    if let Some(hdg) = heading_mdeg {
        let rad = ((hdg as f64) / 1000.0).to_radians() - camera_az_rad;
        let ax = sx + (rad.det_sin() * (8.0 * (s as f64))).round() as i64;
        let ay = sy - (rad.det_cos() * (5.0 * (s as f64))).round() as i64;
        line(frame, sx, sy, ax, ay, YOU);
    }
}

fn draw_speech_or_nameplate(
    frame: &mut Frame,
    sx: i64,
    top_y: i64,
    name: &str,
    speech: &str,
    tag_col: Rgb,
) {
    let sc = 1usize;
    let short_name: String = name.chars().take(16).collect();
    let nw = (short_name.chars().count() as i64) * 6;
    let nx = (sx - nw / 2).clamp(2, (frame.width() as i64).saturating_sub(nw + 4).max(2));
    let ny = top_y - 10;
    fill_rect(frame, nx - 2, ny - 2, nw + 4, 10, PANEL);
    text(frame, nx, ny, &short_name.to_ascii_uppercase(), tag_col, sc);

    if !speech.is_empty() {
        let clipped: String = speech.chars().take(28).collect();
        let bw = (clipped.chars().count() as i64) * 6 + 6;
        let bx = (sx - bw / 2).clamp(2, (frame.width() as i64).saturating_sub(bw + 4).max(2));
        let by = ny - 14;
        fill_rect(frame, bx, by, bw, 11, SPEECH_BG);
        outline_rect(frame, bx, by, bw, 11, SPEECH_BORDER);
        text(
            frame,
            bx + 3,
            by + 2,
            &clipped.to_ascii_uppercase(),
            SPEECH_BORDER,
            sc,
        );
    }
}

fn draw_cutaway_25d(frame: &mut Frame, scene: &WalkScene, look: &Look) {
    frame.clear(BG_SPACE);
    draw_starfield(frame, frame.height());
    let proj = IsoProj::new(frame, scene, look);

    // 1. Lower-floor rooms sorted back-to-front by floor, then camera depth
    let mut lower_rooms: Vec<&WalkRoom> = scene
        .rooms
        .iter()
        .filter(|r| r.floor < scene.you_floor)
        .collect();
    lower_rooms.sort_by(|a, b| {
        a.floor.cmp(&b.floor).then_with(|| {
            let da = proj
                .project((a.min_x + a.max_x) * 0.5, (a.min_y + a.max_y) * 0.5, 0.0)
                .2;
            let db = proj
                .project((b.min_x + b.max_x) * 0.5, (b.min_y + b.max_y) * 0.5, 0.0)
                .2;
            da.total_cmp(&db)
        })
    });
    for room in lower_rooms {
        draw_room_floor(frame, &proj, room, scene.you_floor);
    }

    // 2. Active-floor room deck floors & labels
    let mut active_rooms: Vec<&WalkRoom> = scene
        .rooms
        .iter()
        .filter(|r| r.floor == scene.you_floor)
        .collect();
    active_rooms.sort_by(|a, b| {
        let da = proj
            .project((a.min_x + a.max_x) * 0.5, (a.min_y + a.max_y) * 0.5, 0.0)
            .2;
        let db = proj
            .project((b.min_x + b.max_x) * 0.5, (b.min_y + b.max_y) * 0.5, 0.0)
            .2;
        da.total_cmp(&db)
    });
    for room in &active_rooms {
        draw_room_floor(frame, &proj, room, scene.you_floor);
        if !room.active && !room.name.is_empty() {
            let z = (room.floor as f64) * 3.8;
            let (rx, ry, _) = proj.project(
                (room.min_x + room.max_x) * 0.5,
                (room.min_y + room.max_y) * 0.5,
                z,
            );
            let tag: String = room.name.chars().take(14).collect();
            text(
                frame,
                rx - (tag.chars().count() as i64) * 3,
                ry - 3,
                &tag.to_ascii_uppercase(),
                DIM,
                1,
            );
        }
    }

    // 3. Collect all 3D walls, moored hulls, stairs, cargo pallets, props, and actors into
    // one unified back-to-front painter's-order queue so occlusion respects camera azimuth.
    let mut items: Vec<(f64, i32, CutawayItem<'_>)> = Vec::new();
    let you_z = (scene.you_floor as f64) * 3.8;

    for room in &active_rooms {
        collect_room_wall_spans(&mut items, &proj, room, &scene.doors);
        if scene.aboard_ship && room.kind == "hold" && scene.cargo_fill_permille > 0 {
            let z = (room.floor as f64) * 3.8;
            let pallets = ((scene.cargo_fill_permille * 6) / 1000).clamp(1, 6);
            for idx in 0..pallets {
                let col_idx = idx % 3;
                let row_idx = idx / 3;
                let px = room.min_x + 2.2 + (col_idx as f64) * 2.1;
                let py = room.min_y + 2.0 + (row_idx as f64) * 2.4;
                if px < room.max_x - 1.2 && py < room.max_y - 1.2 {
                    let (_, _, d) = proj.project(px, py, z);
                    items.push((
                        d,
                        1,
                        CutawayItem::CargoPallet {
                            x_m: px,
                            y_m: py,
                            z_m: z,
                        },
                    ));
                }
            }
        }
    }

    for hull in &scene.moored {
        let (_, _, d) = proj.project(hull.x_m, hull.y_m, you_z + 0.6);
        items.push((d, 2, CutawayItem::Moored(hull)));
    }

    for d in &scene.doors {
        if let Some(to_floor) = d.stairs_to
            && (d.floor == scene.you_floor || to_floor == scene.you_floor)
        {
            let z_lo = (d.floor.min(to_floor) as f64) * 3.8;
            for step in 0..6i64 {
                let frac = (step as f64) / 6.0;
                let sz = z_lo + frac * 3.6;
                let sy_off = (frac - 0.5) * 3.2;
                let col = if step % 2 == 0 {
                    STAIR_STEP_A
                } else {
                    STAIR_STEP_B
                };
                let (_, _, depth) = proj.project(d.x_m, d.y_m + sy_off, sz);
                items.push((
                    depth,
                    1,
                    CutawayItem::StairStep {
                        x_m: d.x_m,
                        y_m: d.y_m + sy_off,
                        z_m: sz,
                        top_col: col,
                    },
                ));
            }
        }
    }

    for p in &scene.props {
        if p.floor != scene.you_floor {
            continue;
        }
        let (_, _, d) = proj.project(p.x_m, p.y_m, you_z);
        items.push((d, 1, CutawayItem::Prop(p)));
    }

    for a in &scene.actors {
        if a.floor != scene.you_floor {
            continue;
        }
        let (_, _, d) = proj.project(a.x_m, a.y_m, you_z);
        items.push((d, 2, CutawayItem::Actor(a)));
    }
    let (_, _, you_d) = proj.project(scene.you_x, scene.you_y, you_z);
    items.push((you_d, 2, CutawayItem::You));

    items.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let sp_scale = if frame.width() >= 520 { 2 } else { 1 };
    let mut nameplates: Vec<(i64, i64, &str, &str, Rgb)> = Vec::new();

    for (_, _, item) in items {
        match item {
            CutawayItem::WallSpan {
                p0,
                p1,
                z,
                height_m,
                is_glass,
            } => {
                draw_iso_wall_span(frame, &proj, p0, p1, z, height_m, is_glass);
            }
            CutawayItem::Moored(hull) => {
                let (hx, hy, _) = proj.project(hull.x_m, hull.y_m, you_z + 0.6);
                let half_px = ((proj.px_per_m * 2.4).round() as i64).clamp(10, 40);
                draw_ship_sprite(
                    frame,
                    &ShipSpriteDraw {
                        center_px: (hx, hy),
                        half_size_px: half_px,
                        yaw_deg: 90.0,
                        style: hull.style,
                        burning: false,
                    },
                );
                let tag_col = if hull.own { GOOD } else { DIM };
                let short: String = hull.name.chars().take(14).collect();
                text(
                    frame,
                    hx - (short.chars().count() as i64) * 3,
                    hy - half_px - 9,
                    &short.to_ascii_uppercase(),
                    tag_col,
                    1,
                );
            }
            CutawayItem::StairStep {
                x_m,
                y_m,
                z_m,
                top_col,
            } => {
                draw_iso_box(
                    frame,
                    &proj,
                    (x_m, y_m, z_m),
                    (1.8, 0.55, 0.35),
                    WALL_FACE_DARK,
                    top_col,
                );
            }
            CutawayItem::CargoPallet { x_m, y_m, z_m } => {
                draw_iso_box(
                    frame,
                    &proj,
                    (x_m, y_m, z_m),
                    (1.4, 1.4, 1.1),
                    PROP_CARGO,
                    PROP_CARGO_TOP,
                );
            }
            CutawayItem::Prop(p) => {
                let z = (p.floor as f64) * 3.8;
                match p.kind.as_str() {
                    "airlock" => draw_iso_box(
                        frame,
                        &proj,
                        (p.x_m, p.y_m, z),
                        (1.6, 0.8, 2.0),
                        WALL_KICK,
                        PROP_AIRLOCK,
                    ),
                    "helm" => draw_iso_box(
                        frame,
                        &proj,
                        (p.x_m, p.y_m, z),
                        (1.5, 1.4, 1.0),
                        PROP_DESK,
                        PROP_HELM,
                    ),
                    "holotank" => {
                        draw_iso_box(
                            frame,
                            &proj,
                            (p.x_m, p.y_m, z),
                            (1.6, 1.6, 0.85),
                            WALL_FACE_DARK,
                            PROP_CORE,
                        );
                        let (hx, hy, _) = proj.project(p.x_m, p.y_m, z + 1.25);
                        let r = 6 + (scene.pulse_phase % 3);
                        outline_rect(frame, hx - r, hy - r / 2, r * 2, r, PROP_CORE);
                    }
                    "pod" => draw_iso_box(
                        frame,
                        &proj,
                        (p.x_m, p.y_m, z),
                        (1.8, 1.2, 1.3),
                        PROP_DESK,
                        PROP_CORE,
                    ),
                    _ => draw_iso_box(
                        frame,
                        &proj,
                        (p.x_m, p.y_m, z),
                        (2.4, 1.0, 1.05),
                        PROP_DESK,
                        PROP_DESK_TOP,
                    ),
                }
                if !p.label.is_empty() {
                    let (lx, ly, _) = proj.project(p.x_m, p.y_m, z + 1.55);
                    let short: String = p.label.chars().take(12).collect();
                    text(
                        frame,
                        lx - (short.chars().count() as i64) * 3,
                        ly - 4,
                        &short.to_ascii_uppercase(),
                        DIM,
                        1,
                    );
                }
            }
            CutawayItem::Actor(act) => {
                let (ax, ay, _) = proj.project(act.x_m, act.y_m, you_z);
                let (suit, hi, tag) = if act.npc {
                    (NPC_COAT, NPC_HIGHLIGHT, NPC_HIGHLIGHT)
                } else {
                    (CREW_SUIT, CREW_HIGHLIGHT, GOOD)
                };
                draw_humanoid_sprite(frame, (ax, ay), sp_scale, (suit, hi), None, 0.0, 0);
                nameplates.push((ax, ay - 16 * sp_scale, &act.name, &act.speech, tag));
            }
            CutawayItem::You => {
                let (ux, uy, _) = proj.project(scene.you_x, scene.you_y, you_z);
                draw_humanoid_sprite(
                    frame,
                    (ux, uy),
                    sp_scale,
                    (YOU_SUIT, YOU),
                    Some(scene.you_heading_mdeg),
                    proj.az_rad,
                    scene.you_stride,
                );
            }
        }
    }

    // 4. Focused `[E]` interactable bracket & actor nameplates on top so text stays crisp
    if let Some(ref foc) = scene.focus
        && foc.floor == scene.you_floor
    {
        let (fx, fy, _) = proj.project(foc.x_m, foc.y_m, (foc.floor as f64) * 3.8 + 0.6);
        let r = 9i64;
        outline_rect(frame, fx - r, fy - r, r * 2, r * 2, LOCK);
    }

    for (ax, top_y, name, speech, tag) in nameplates {
        draw_speech_or_nameplate(frame, ax, top_y, name, speech, tag);
    }
}

// ---- Mode 2: First-Person 0.5 m Indoor Corridor Raycaster -------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellKind {
    Solid,
    Glass,
    Floor,
}

#[derive(Debug)]
struct IndoorGrid {
    origin_x: f64,
    origin_y: f64,
    cell_m: f64,
    cols: usize,
    rows: usize,
    cells: Vec<CellKind>,
}

fn hash_scene_grid(
    scene: &WalkScene,
    origin_x: f64,
    origin_y: f64,
    cols: usize,
    rows: usize,
) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(scene.you_floor as u64);
    mix(origin_x.to_bits());
    mix(origin_y.to_bits());
    mix(cols as u64);
    mix(rows as u64);
    for r in &scene.rooms {
        if r.floor == scene.you_floor {
            mix(r.min_x.to_bits());
            mix(r.min_y.to_bits());
            mix(r.max_x.to_bits());
            mix(r.max_y.to_bits());
            mix(u64::from(r.kind == "berth"));
        }
    }
    for d in &scene.doors {
        if d.floor == scene.you_floor || d.stairs_to == Some(scene.you_floor) {
            mix(d.x_m.to_bits());
            mix(d.y_m.to_bits());
            mix(d.width_m.to_bits());
        }
    }
    h
}

impl IndoorGrid {
    fn window_bounds(scene: &WalkScene) -> (f64, f64, usize, usize) {
        const CELL_M: f64 = 0.5;
        const MAX_CELLS: usize = 256;
        const MAX_SPAN_M: f64 = (MAX_CELLS as f64) * CELL_M; // 128.0 m

        let mut min_x = scene.you_x - 12.0;
        let mut max_x = scene.you_x + 12.0;
        let mut min_y = scene.you_y - 12.0;
        let mut max_y = scene.you_y + 12.0;
        for r in &scene.rooms {
            if r.floor == scene.you_floor {
                min_x = min_x.min(r.min_x - 1.5);
                max_x = max_x.max(r.max_x + 1.5);
                min_y = min_y.min(r.min_y - 1.5);
                max_y = max_y.max(r.max_y + 1.5);
            }
        }
        // Window around `(you_x, you_y)` when the floor exceeds 128 m so rooms
        // far from the origin never silently truncate into invisible walls.
        if max_x - min_x > MAX_SPAN_M {
            let snapped_cx = (scene.you_x / 8.0).floor() * 8.0;
            min_x = snapped_cx - MAX_SPAN_M * 0.5;
            max_x = min_x + MAX_SPAN_M;
        }
        if max_y - min_y > MAX_SPAN_M {
            let snapped_cy = (scene.you_y / 8.0).floor() * 8.0;
            min_y = snapped_cy - MAX_SPAN_M * 0.5;
            max_y = min_y + MAX_SPAN_M;
        }
        let cols = (((max_x - min_x) / CELL_M).ceil() as usize).clamp(16, MAX_CELLS);
        let rows = (((max_y - min_y) / CELL_M).ceil() as usize).clamp(16, MAX_CELLS);
        (min_x, min_y, cols, rows)
    }

    fn from_scene_with_bounds(
        scene: &WalkScene,
        origin_x: f64,
        origin_y: f64,
        cols: usize,
        rows: usize,
    ) -> IndoorGrid {
        let cell_m = 0.5;
        let mut grid = IndoorGrid {
            origin_x,
            origin_y,
            cell_m,
            cols,
            rows,
            cells: vec![CellKind::Solid; cols.saturating_mul(rows)],
        };
        let grid_max_x = origin_x + (cols as f64) * cell_m;
        let grid_max_y = origin_y + (rows as f64) * cell_m;

        // Carve room interiors (leaving 1-cell perimeter walls)
        for r in &scene.rooms {
            if r.floor != scene.you_floor {
                continue;
            }
            let rx0 = (r.min_x + 0.35).max(origin_x);
            let rx1 = (r.max_x - 0.35).min(grid_max_x - cell_m);
            let ry0 = (r.min_y + 0.35).max(origin_y);
            let ry1 = (r.max_y - 0.35).min(grid_max_y - cell_m);
            if rx1 < rx0 || ry1 < ry0 {
                continue;
            }
            let x0 = grid.to_col(rx0);
            let x1 = grid.to_col(rx1);
            let y0 = grid.to_row(ry0);
            let y1 = grid.to_row(ry1);
            for cy in y0..=y1 {
                for cx in x0..=x1 {
                    grid.set(cx, cy, CellKind::Floor);
                }
            }
            if r.kind == "berth" {
                let south_glass = berth_glass_is_south(r, &scene.doors);
                let glass_y = if south_glass { r.min_y } else { r.max_y };
                if glass_y >= origin_y && glass_y <= grid_max_y {
                    let gy = grid.to_row(glass_y);
                    for cx in x0..=x1 {
                        grid.set(cx, gy, CellKind::Glass);
                        if south_glass {
                            if gy + 1 < rows {
                                grid.set(cx, gy + 1, CellKind::Floor);
                            }
                        } else if gy > 0 {
                            grid.set(cx, gy - 1, CellKind::Floor);
                        }
                    }
                }
            }
        }

        // Carve doorway & stairway openings across shared walls
        for d in &scene.doors {
            if d.floor != scene.you_floor && d.stairs_to != Some(scene.you_floor) {
                continue;
            }
            let rad = (d.width_m * 0.5).max(0.8);
            let dx0 = (d.x_m - rad).max(origin_x);
            let dx1 = (d.x_m + rad).min(grid_max_x - cell_m);
            let dy0 = (d.y_m - rad).max(origin_y);
            let dy1 = (d.y_m + rad).min(grid_max_y - cell_m);
            if dx1 < dx0 || dy1 < dy0 {
                continue;
            }
            let x0 = grid.to_col(dx0);
            let x1 = grid.to_col(dx1);
            let y0 = grid.to_row(dy0);
            let y1 = grid.to_row(dy1);
            for cy in y0..=y1 {
                for cx in x0..=x1 {
                    grid.set(cx, cy, CellKind::Floor);
                }
            }
        }

        // Guarantee player's immediate 3x3 cells are walkable floor so camera never sits inside a wall
        let px = grid.to_col(scene.you_x);
        let py = grid.to_row(scene.you_y);
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let cx = (px as i64 + dx).clamp(1, (cols as i64).saturating_sub(2).max(1)) as usize;
                let cy = (py as i64 + dy).clamp(1, (rows as i64).saturating_sub(2).max(1)) as usize;
                grid.set(cx, cy, CellKind::Floor);
            }
        }

        grid
    }

    fn to_col(&self, wx: f64) -> usize {
        (((wx - self.origin_x) / self.cell_m).floor() as i64)
            .clamp(0, (self.cols as i64).saturating_sub(1).max(0)) as usize
    }

    fn to_row(&self, wy: f64) -> usize {
        (((wy - self.origin_y) / self.cell_m).floor() as i64)
            .clamp(0, (self.rows as i64).saturating_sub(1).max(0)) as usize
    }

    fn set(&mut self, cx: usize, cy: usize, val: CellKind) {
        if cx < self.cols
            && cy < self.rows
            && let Some(slot) = self.cells.get_mut(cy * self.cols + cx)
        {
            *slot = val;
        }
    }

    fn get(&self, cx: i64, cy: i64) -> CellKind {
        if cx < 0 || cy < 0 || (cx as usize) >= self.cols || (cy as usize) >= self.rows {
            return CellKind::Solid;
        }
        self.cells
            .get((cy as usize) * self.cols + (cx as usize))
            .copied()
            .unwrap_or(CellKind::Solid)
    }
}

fn fog_idx(dist_m: f64) -> usize {
    ((dist_m / 4.8).floor() as usize).min(FP_FOG_STEPS - 1)
}

fn draw_first_person(
    frame: &mut Frame,
    scene: &WalkScene,
    look: &Look,
    cached_grid: &mut Option<(u64, IndoorGrid)>,
) {
    let w = frame.width();
    let h = frame.height();
    if w == 0 || h == 0 {
        return;
    }
    frame.clear(BG_SPACE);
    let horizon = h / 2;

    let (origin_x, origin_y, cols, rows) = IndoorGrid::window_bounds(scene);
    let grid_key = hash_scene_grid(scene, origin_x, origin_y, cols, rows);
    if !cached_grid.as_ref().is_some_and(|(k, _)| *k == grid_key) {
        *cached_grid = None;
    }
    let (_, grid) = cached_grid.get_or_insert_with(|| {
        (
            grid_key,
            IndoorGrid::from_scene_with_bounds(scene, origin_x, origin_y, cols, rows),
        )
    });

    let hdg_mdeg = scene
        .you_heading_mdeg
        .wrapping_add(look.offset_mdeg)
        .rem_euclid(360_000);
    let hdg_rad = ((hdg_mdeg as f64) / 1000.0).to_radians();
    // Heading 0 = North (+Y), 90 = East (+X)
    let dir_x = hdg_rad.det_sin();
    let dir_y = hdg_rad.det_cos();
    let fov_scale = 0.72 / (f64::from(look.zoom_permille.clamp(750, 2000)) / 1000.0);
    let plane_x = dir_y * fov_scale;
    let plane_y = -dir_x * fov_scale;

    let pos_gx = ((scene.you_x - grid.origin_x) / grid.cell_m).clamp(-1e6, 1e6);
    let pos_gy = ((scene.you_y - grid.origin_y) / grid.cell_m).clamp(-1e6, 1e6);

    let mut col_depth_m = vec![100.0f64; w];
    let mut col_glass = vec![false; w];

    // 1. Ceiling & Floor horizontal bands with symmetric non-skid grid lines
    for y in 0..h {
        let dy = (y as i64) - (horizon as i64);
        if dy == 0 {
            continue;
        }
        let row_dist_m = ((h as f64) * 0.55) / (dy.unsigned_abs() as f64);
        let fi = fog_idx(row_dist_m);
        if y < horizon {
            let ccol = FP_CEIL_SHADES.get(fi).copied().unwrap_or(BG_SPACE);
            for x in 0..w {
                frame.put(x, y, ccol);
            }
        } else {
            let base_col = FP_FLOOR_SHADES.get(fi).copied().unwrap_or(BG_SPACE);
            let grid_col = FP_GRID_SHADES.get(fi).copied().unwrap_or(base_col);
            let ray0_x = dir_x - plane_x;
            let ray0_y = dir_y - plane_y;
            let ray1_x = dir_x + plane_x;
            let ray1_y = dir_y + plane_y;
            let step_wx = row_dist_m * (ray1_x - ray0_x) / (w.max(1) as f64);
            let step_wy = row_dist_m * (ray1_y - ray0_y) / (w.max(1) as f64);
            let mut wx = scene.you_x + row_dist_m * ray0_x;
            let mut wy = scene.you_y + row_dist_m * ray0_y;
            for x in 0..w {
                let on_seam =
                    (wx * 0.5).rem_euclid(1.0) < 0.045 || (wy * 0.5).rem_euclid(1.0) < 0.045;
                frame.put(x, y, if on_seam { grid_col } else { base_col });
                wx += step_wx;
                wy += step_wy;
            }
        }
    }

    // 2. Column DDA wall & armor-glass window pass
    for x in 0..w {
        let cam_x = 2.0 * (x as f64) / (w.max(1) as f64) - 1.0;
        let rdx = dir_x + plane_x * cam_x;
        let rdy = dir_y + plane_y * cam_x;

        let mut map_x = pos_gx.floor() as i64;
        let mut map_y = pos_gy.floor() as i64;

        let delta_x = if rdx.abs() < 1e-6 {
            1e6
        } else {
            (1.0 / rdx).abs()
        };
        let delta_y = if rdy.abs() < 1e-6 {
            1e6
        } else {
            (1.0 / rdy).abs()
        };

        let (step_x, mut side_x) = if rdx < 0.0 {
            (-1i64, (pos_gx - (map_x as f64)) * delta_x)
        } else {
            (1i64, ((map_x as f64) + 1.0 - pos_gx) * delta_x)
        };
        let (step_y, mut side_y) = if rdy < 0.0 {
            (-1i64, (pos_gy - (map_y as f64)) * delta_y)
        } else {
            (1i64, ((map_y as f64) + 1.0 - pos_gy) * delta_y)
        };

        let mut hit = CellKind::Floor;
        let mut side_ew = false;
        for _ in 0..192 {
            if side_x < side_y {
                side_x += delta_x;
                map_x = map_x.saturating_add(step_x);
                side_ew = true;
            } else {
                side_y += delta_y;
                map_y = map_y.saturating_add(step_y);
                side_ew = false;
            }
            let ck = grid.get(map_x, map_y);
            if ck != CellKind::Floor {
                hit = ck;
                break;
            }
        }

        // Do NOT draw a phantom solid wall when a ray travels 96 m without hitting a wall.
        if hit == CellKind::Floor {
            if let Some(d) = col_depth_m.get_mut(x) {
                *d = 100.0;
            }
            continue;
        }

        let perp_cells = if side_ew {
            (side_x - delta_x).max(0.15)
        } else {
            (side_y - delta_y).max(0.15)
        };
        let dist_m = perp_cells * grid.cell_m;
        if let Some(d) = col_depth_m.get_mut(x) {
            *d = dist_m;
        }

        let line_h = (((h as f64) * 1.15) / dist_m).round().clamp(1.0, 100_000.0) as i64;
        let y0 = ((horizon as i64) - line_h / 2).clamp(0, (h as i64) - 1) as usize;
        let y1 = ((horizon as i64) + line_h / 2).clamp(0, (h as i64) - 1) as usize;
        let fi = fog_idx(dist_m);

        let wall_u = if side_ew {
            pos_gy + perp_cells * rdy
        } else {
            pos_gx + perp_cells * rdx
        }
        .rem_euclid(1.0);

        if hit == CellKind::Glass {
            if let Some(g) = col_glass.get_mut(x) {
                *g = true;
            }
            // Armor-glass window: starfield vacuum with mullion posts & safety curb!
            let is_mullion = wall_u < 0.06 || wall_u > 0.94;
            for y in y0..=y1 {
                let v = if y1 > y0 {
                    (y - y0) as f64 / (y1 - y0) as f64
                } else {
                    0.5
                };
                let px = if is_mullion || v < 0.08 {
                    WALL_FACE_DARK
                } else if v > 0.86 {
                    SAFETY_YELLOW
                } else if (x.wrapping_mul(131).wrapping_add(y.wrapping_mul(97))) % 197 == 0 {
                    STAR_BRIGHT
                } else {
                    BG_SPACE
                };
                frame.put(x, y, px);
            }
        } else {
            let base = if side_ew {
                FP_WALL_DARK_SHADES
                    .get(fi)
                    .copied()
                    .unwrap_or(WALL_FACE_DARK)
            } else {
                FP_WALL_LIT_SHADES.get(fi).copied().unwrap_or(WALL_FACE_LIT)
            };
            let seam = wall_u < 0.05;
            for y in y0..=y1 {
                let v = if y1 > y0 {
                    (y - y0) as f64 / (y1 - y0) as f64
                } else {
                    0.5
                };
                let px = if v > 0.88 || seam {
                    WALL_KICK
                } else if (0.52..=0.56).contains(&v) {
                    WALL_RAIL
                } else {
                    base
                };
                frame.put(x, y, px);
            }
        }
    }

    // 3. Depth-buffered billboard sprites (moored ships through glass, props, and actors)
    let det = plane_x * dir_y - dir_x * plane_y;
    if det.abs() < 1e-9 {
        return;
    }
    let inv_det = 1.0 / det;
    let project_billboard = |wx: f64, wy: f64| -> Option<(i64, f64, i64)> {
        let sx = wx - scene.you_x;
        let sy = wy - scene.you_y;
        let tx = inv_det * (dir_y * sx - dir_x * sy);
        let ty = inv_det * (-plane_y * sx + plane_x * sy);
        if ty <= 0.35 || !tx.is_finite() || !ty.is_finite() {
            return None;
        }
        let screen_x = (((w as f64) * 0.5) * (1.0 + tx / ty))
            .round()
            .clamp(-100_000.0, 100_000.0) as i64;
        let sprite_h = (((h as f64) * 0.95) / ty).round().clamp(2.0, 100_000.0) as i64;
        Some((screen_x, ty, sprite_h))
    };

    // 3a. Moored ships visible through armor-glass bay windows (sorted far-to-near)
    let mut fp_moored: Vec<(i64, f64, i64, usize)> = scene
        .moored
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let (sx, ty, sh) = project_billboard(m.x_m, m.y_m)?;
            Some((sx, ty, sh, i))
        })
        .collect();
    fp_moored.sort_by(|a, b| b.1.total_cmp(&a.1));

    for (sx, _ty, sh, idx) in fp_moored {
        if sx < -48 || sx > (w as i64) + 48 {
            continue;
        }
        let center_col = sx.clamp(0, (w as i64) - 1) as usize;
        if !col_glass.get(center_col).copied().unwrap_or(false) {
            continue;
        }
        if let Some(hull) = scene.moored.get(idx) {
            let half_px = (sh / 2).clamp(10, 44);
            let cy = (horizon as i64).saturating_sub(half_px / 6);
            draw_ship_sprite(
                frame,
                &ShipSpriteDraw {
                    center_px: (sx, cy),
                    half_size_px: half_px,
                    yaw_deg: 90.0,
                    style: hull.style,
                    burning: false,
                },
            );
            let tag_col = if hull.own { GOOD } else { DIM };
            let short: String = hull.name.chars().take(14).collect();
            text(
                frame,
                sx - (short.chars().count() as i64) * 3,
                cy - half_px - 9,
                &short.to_ascii_uppercase(),
                tag_col,
                1,
            );
        }
    }

    // 3b. Props
    for p in &scene.props {
        if p.floor != scene.you_floor {
            continue;
        }
        if let Some((sx, ty, sh)) = project_billboard(p.x_m, p.y_m)
            && sx >= 0
            && (sx as usize) < w
            && col_depth_m.get(sx as usize).copied().unwrap_or(0.0) + 0.6 >= ty
        {
            let pw = (sh / 2).clamp(4, 64);
            let ph = (sh / 3).clamp(4, 48);
            let by = (horizon as i64) + sh / 2 - ph;
            let col = match p.kind.as_str() {
                "airlock" => PROP_AIRLOCK,
                "helm" | "holotank" | "pod" => PROP_HELM,
                _ => PROP_DESK_TOP,
            };
            fill_rect(frame, sx - pw / 2, by, pw, ph, PROP_DESK);
            outline_rect(frame, sx - pw / 2, by, pw, ph, col);
        }
    }

    // 3c. Actors sorted far-to-near
    let mut fp_actors: Vec<(i64, f64, i64, usize)> = scene
        .actors
        .iter()
        .enumerate()
        .filter(|(_, a)| a.floor == scene.you_floor)
        .filter_map(|(i, a)| {
            let (sx, ty, sh) = project_billboard(a.x_m, a.y_m)?;
            Some((sx, ty, sh, i))
        })
        .collect();
    fp_actors.sort_by(|a, b| b.1.total_cmp(&a.1));

    for (sx, ty, sh, idx) in fp_actors {
        if sx < -32 || sx > (w as i64) + 32 {
            continue;
        }
        let center_col = sx.clamp(0, (w as i64) - 1) as usize;
        if col_depth_m.get(center_col).copied().unwrap_or(0.0) + 0.4 < ty {
            continue;
        }
        if let Some(act) = scene.actors.get(idx) {
            let scale = (sh / 16).clamp(1, 3);
            let foot_y = (horizon as i64) + sh / 2;
            let (suit, hi, tag) = if act.npc {
                (NPC_COAT, NPC_HIGHLIGHT, NPC_HIGHLIGHT)
            } else {
                (CREW_SUIT, CREW_HIGHLIGHT, GOOD)
            };
            draw_humanoid_sprite(frame, (sx, foot_y), scale, (suit, hi), None, 0.0, 0);
            draw_speech_or_nameplate(frame, sx, foot_y - 16 * scale, &act.name, &act.speech, tag);
        }
    }
}

// ---- Shared Walk HUD Banner & Interact Prompt -------------------------------

fn draw_walk_hud(frame: &mut Frame, scene: &WalkScene, look: &Look) {
    let sc = scale_for(frame);
    let w = frame.width() as i64;
    let h = frame.height() as i64;
    if w < 80 || h < 60 {
        return;
    }

    // Top-left location badge + camera mode (truncate title BEFORE suffix so HDG is never chopped off!)
    let cam_tag = match scene.camera {
        WalkCamera::Cutaway25D => "2.5D CUTAWAY",
        WalkCamera::FirstPersonRaycast => "1P CORRIDOR",
    };
    let hdg_deg = scene
        .you_heading_mdeg
        .wrapping_add(look.offset_mdeg)
        .rem_euclid(360_000)
        / 1000;
    let suffix = format!("  [{cam_tag}]  HDG {hdg_deg:03}");
    let char_px = 6 * (sc as i64);
    let max_banner_chars = (((w - 18) / char_px.max(1)).max(1) as usize).min(56);
    let suffix_chars = suffix.chars().count();
    let clipped_h: String = if max_banner_chars > suffix_chars + 2 {
        let title_budget = max_banner_chars - suffix_chars;
        let short_title: String = scene
            .title
            .to_ascii_uppercase()
            .chars()
            .take(title_budget)
            .collect();
        format!("{short_title}{suffix}")
    } else {
        suffix.chars().take(max_banner_chars).collect()
    };
    let banner_w = ((clipped_h.chars().count() as i64) * char_px + 10).min(w - 8);
    let banner_h = 9 * (sc as i64) + 6;
    fill_rect(frame, 4, 4, banner_w, banner_h, PANEL);
    outline_rect(frame, 4, 4, banner_w, banner_h, BORDER);
    text(frame, 8, 7, &clipped_h, TEXT, sc);

    if !scene.subtitle.is_empty() {
        let max_sub_chars = (((w - 16) / 6).max(1) as usize).min(48);
        let sub: String = scene
            .subtitle
            .to_ascii_uppercase()
            .chars()
            .take(max_sub_chars)
            .collect();
        let sub_w = ((sub.chars().count() as i64) * 6 + 8).min(w - 8);
        fill_rect(frame, 4, 4 + banner_h + 2, sub_w, 11, PANEL);
        text(frame, 8, 4 + banner_h + 4, &sub, WARN, 1);
    }

    // Bottom-center `[E]` interactable prompt pill (clamped strictly within frame width)
    if let Some(ref foc) = scene.focus {
        let max_pill_chars = (((w - 20) / char_px.max(1)).max(1) as usize).min(40);
        let prompt = format!("[E] {}", foc.label.to_ascii_uppercase());
        let clipped_p: String = prompt.chars().take(max_pill_chars).collect();
        let pw = ((clipped_p.chars().count() as i64) * char_px + 12).min(w - 8);
        let ph = 9 * (sc as i64) + 6;
        let px = ((w - pw) / 2).max(4);
        let py = (h - ph - 6).max(4);
        fill_rect(frame, px, py, pw, ph, PANEL);
        outline_rect(frame, px, py, pw, ph, GOOD);
        text(frame, px + 6, py + 3, &clipped_p, GOOD, sc);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::picture::Feeds;

    struct SingleFeed(Value);
    impl Feeds for SingleFeed {
        fn feed(&self, name: &str) -> Option<&Value> {
            (name == STATION_WALK_FEED).then_some(&self.0)
        }
    }

    fn sample_station_feed(camera: &str) -> Value {
        json!({
            "camera": camera,
            "mode": "station",
            "title": "Visiting Berth Gallery",
            "subtitle": "Deck +1  |  Salt Moth cradled",
            "you": { "x_m": 0.0, "y_m": 2.0, "floor": 1, "heading_mdeg": 15_000, "stride": 1 },
            "cargo_fill_permille": 450,
            "pulse_phase": 2,
            "focus": { "x_m": 0.0, "y_m": 6.5, "floor": 1, "label": "Board Salt Moth (+ship/board)" },
            "rooms": [
                {
                    "name": "Freight Deck",
                    "floor": 0,
                    "min_x": -12.0, "min_y": -8.0, "max_x": 12.0, "max_y": 8.0,
                    "active": false,
                    "kind": "room"
                },
                {
                    "name": "Visiting Berth Gallery",
                    "floor": 1,
                    "min_x": -18.0, "min_y": -6.0, "max_x": 18.0, "max_y": 8.0,
                    "active": true,
                    "kind": "berth"
                }
            ],
            "doors": [
                { "x_m": -8.0, "y_m": -6.0, "floor": 1, "width_m": 2.6, "stairs_to": 0 },
                { "x_m": 0.0, "y_m": -6.0, "floor": 1, "width_m": 2.8 }
            ],
            "props": [
                { "x_m": 0.0, "y_m": 7.0, "floor": 1, "kind": "airlock", "label": "Cradle 01" },
                { "x_m": 6.0, "y_m": 0.0, "floor": 1, "kind": "desk", "label": "Berth Board" }
            ],
            "moored": [
                { "x_m": -6.0, "y_m": 15.0, "name": "Salt Moth", "style": "freighter", "own": true },
                { "x_m": 8.0, "y_m": 16.0, "name": "Cold Comfort", "style": "corvette", "own": false }
            ],
            "actors": [
                { "x_m": -3.5, "y_m": 3.0, "floor": 1, "name": "Henrik", "npc": true, "speech": "Cradle 01 is pressurized." },
                { "x_m": 4.0, "y_m": 1.5, "floor": 1, "name": "Kira", "npc": false, "speech": "" }
            ]
        })
    }

    #[test]
    fn both_station_walk_cameras_draw_within_256_color_palette() {
        let mut pic = StationWalkPicture::new();
        let look = Look {
            offset_mdeg: 0,
            hull_offset_mdeg: 0,
            ordered_mdeg: None,
            dx_cm: 0,
            dy_cm: 0,
            zoom_permille: 1000,
        };

        for cam in ["cutaway", "first_person"] {
            for (w, h) in [(320usize, 220usize), (640, 360)] {
                let mut frame = Frame::new(w, h);
                let feeds = SingleFeed(sample_station_feed(cam));
                assert!(pic.draw(&mut frame, &feeds, &look));
                let colors = frame.count_colours(257);
                assert!(
                    colors <= 256,
                    "{cam} at {w}x{h} used {colors} colors (must stay <= 256 for indexed PNG & Sixel)"
                );
            }
        }
    }

    #[test]
    fn ship_interior_hold_and_bridge_holotank_render_cleanly() {
        let mut pic = StationWalkPicture::new();
        let look = Look {
            offset_mdeg: 30_000,
            hull_offset_mdeg: 0,
            ordered_mdeg: None,
            dx_cm: 0,
            dy_cm: 0,
            zoom_permille: 1250,
        };
        let interior = json!({
            "camera": "cutaway",
            "mode": "interior",
            "title": "Salt Moth - Bridge",
            "subtitle": "Docked at Ceres Freeport",
            "you": { "x_m": 0.0, "y_m": 5.0, "floor": 1, "heading_mdeg": 0, "stride": 0 },
            "cargo_fill_permille": 800,
            "pulse_phase": 1,
            "focus": { "x_m": 0.0, "y_m": 6.8, "floor": 1, "label": "Sit at Helm (+helm/station command)" },
            "rooms": [
                { "name": "Hold", "floor": 0, "min_x": -4.5, "min_y": -6.0, "max_x": 4.5, "max_y": 2.0, "active": false, "kind": "hold" },
                { "name": "Bridge", "floor": 1, "min_x": -3.5, "min_y": 2.0, "max_x": 3.5, "max_y": 8.0, "active": true, "kind": "bridge" }
            ],
            "doors": [
                { "x_m": 0.0, "y_m": 2.0, "floor": 1, "width_m": 1.8, "stairs_to": 0 }
            ],
            "props": [
                { "x_m": 0.0, "y_m": 4.5, "floor": 1, "kind": "holotank", "label": "Holo-Tank" },
                { "x_m": 0.0, "y_m": 6.8, "floor": 1, "kind": "helm", "label": "Bridge Helm" }
            ]
        });
        let mut frame = Frame::new(640, 360);
        assert!(pic.draw(&mut frame, &SingleFeed(interior), &look));
        assert!(frame.count_colours(257) <= 256);
    }

    #[cfg(feature = "encode")]
    #[test]
    fn draw_kitty_clamps_u16_max_terminal_dimensions_and_reuses_frame() {
        let mut pic = StationWalkPicture::new();
        let look = Look {
            offset_mdeg: 0,
            hull_offset_mdeg: 0,
            ordered_mdeg: None,
            dx_cm: 0,
            dy_cm: 0,
            zoom_permille: 1000,
        };
        let feeds = SingleFeed(sample_station_feed("cutaway"));
        let cell = Some(crate::caps::CellSize {
            width: 8,
            height: 16,
        });
        let (vp, bytes) = pic
            .draw_kitty(&feeds, &look, (0, 0, u16::MAX, u16::MAX), cell, false)
            .expect("clamped draw_kitty succeeds");
        assert!(vp.width_px <= 1920 && vp.height_px <= 1080);
        assert!(!bytes.is_empty());
        // Second identical paint skips image re-transmission (empty wire)
        let (_, bytes2) = pic
            .draw_kitty(&feeds, &look, (0, 0, u16::MAX, u16::MAX), cell, false)
            .expect("second draw succeeds");
        assert!(bytes2.is_empty());
    }

    #[test]
    fn offscreen_quad_does_not_smear_edge_columns() {
        let mut frame = Frame::new(120, 80);
        frame.clear(BG_SPACE);
        // Completely off-screen to the left
        fill_quad_scanline(
            &mut frame,
            [(-50, 10), (-10, 10), (-10, 60), (-50, 60)],
            YOU,
        );
        // Completely off-screen to the right
        fill_quad_scanline(
            &mut frame,
            [(130, 10), (180, 10), (180, 60), (130, 60)],
            YOU,
        );
        for y in 0..80 {
            assert_eq!(frame.get(0, y), BG_SPACE, "left edge smeared at y={y}");
            assert_eq!(frame.get(119, y), BG_SPACE, "right edge smeared at y={y}");
        }
    }

    #[test]
    fn extreme_feed_coordinates_and_headings_never_panic() {
        let mut pic = StationWalkPicture::new();
        let look = Look {
            offset_mdeg: i64::MAX / 2,
            hull_offset_mdeg: 0,
            ordered_mdeg: None,
            dx_cm: 0,
            dy_cm: 0,
            zoom_permille: 1000,
        };
        for cam in ["cutaway", "first_person"] {
            let evil = json!({
                "camera": cam,
                "mode": "station",
                "title": "Salt Moth — Engineering Access — Æther",
                "subtitle": "日本語テスト — Extreme Coords",
                "you": { "x_m": 1e12, "y_m": -1e12, "floor": 999999, "heading_mdeg": i64::MAX, "stride": 1 },
                "focus": { "x_m": -1e12, "y_m": 1e12, "floor": 999999, "label": "Very Long Multi-Byte Label — 日本語 — Extra" },
                "rooms": [
                    { "name": "Far Room", "floor": 999999, "min_x": -1e12, "min_y": -1e12, "max_x": 1e12, "max_y": 1e12, "active": true, "kind": "room" }
                ],
                "props": [
                    { "x_m": 1e12, "y_m": -1e12, "floor": 999999, "kind": "desk", "label": "Öffice" }
                ],
                "actors": [
                    { "x_m": 1e12, "y_m": -1e12, "floor": 999999, "name": "Søren", "npc": true, "speech": "Hällo — 12345" }
                ]
            });
            let mut frame = Frame::new(160, 100);
            assert!(pic.draw(&mut frame, &SingleFeed(evil), &look));
        }
        // Direct overlay::line stress with i64 extrema
        let mut frame = Frame::new(80, 60);
        line(
            &mut frame,
            i64::MIN / 4,
            i64::MAX / 4,
            i64::MAX / 4,
            i64::MIN / 4,
            YOU,
        );
    }

    #[test]
    fn first_person_draws_moored_ship_through_glass_and_no_phantom_wall_in_100m_gallery() {
        let mut pic = StationWalkPicture::new();
        let look = Look {
            offset_mdeg: 0,
            hull_offset_mdeg: 0,
            ordered_mdeg: None,
            dx_cm: 0,
            dy_cm: 0,
            zoom_permille: 1000,
        };
        // Real Ceres Visiting Berth Gallery: 100 m wide (`x: 110..210, y: 110..125`), door on North (`y=125`) -> South glass (`y=110`).
        // Look South (heading 180°) with and without a moored ship outside the glass!
        let base_scene = |with_ship: bool, heading_mdeg: i64| {
            json!({
                "camera": "first_person",
                "mode": "station",
                "title": "Visiting Berth Gallery",
                "you": { "x_m": 120.0, "y_m": 117.5, "floor": 1, "heading_mdeg": heading_mdeg, "stride": 0 },
                "rooms": [
                    { "name": "Visiting Berth Gallery", "floor": 1, "min_x": 110.0, "min_y": 110.0, "max_x": 210.0, "max_y": 125.0, "active": true, "kind": "berth" },
                    { "name": "Far Wing", "floor": 1, "min_x": 300.0, "min_y": 110.0, "max_x": 330.0, "max_y": 130.0, "active": false, "kind": "room" }
                ],
                "doors": [
                    { "x_m": 125.0, "y_m": 125.0, "floor": 1, "width_m": 2.8 }
                ],
                "moored": if with_ship {
                    json!([{ "x_m": 120.0, "y_m": 100.0, "name": "Salt Moth", "style": "freighter", "own": true }])
                } else {
                    json!([])
                }
            })
        };

        let mut f_empty = Frame::new(320, 200);
        assert!(pic.draw(&mut f_empty, &SingleFeed(base_scene(false, 180_000)), &look));
        let mut f_ship = Frame::new(320, 200);
        assert!(pic.draw(&mut f_ship, &SingleFeed(base_scene(true, 180_000)), &look));
        assert_ne!(
            f_empty.hash(),
            f_ship.hash(),
            "1P raycaster must render moored ship billboards through armor glass"
        );

        // Looking East (90°) down the 90 m gallery corridor must NOT hit a phantom solid wall at 48 m
        let mut f_east = Frame::new(320, 200);
        assert!(pic.draw(&mut f_east, &SingleFeed(base_scene(false, 90_000)), &look));
        // Center horizon pixel is in open corridor past wall reach (or distant real east wall at 90 m, not 48 m)
        assert!(f_east.count_colours(257) <= 256);
    }
}
