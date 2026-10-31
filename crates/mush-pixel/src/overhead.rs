//! **The overhead** (doc 21 §7.4, cut G.12b): the cockpit's second camera,
//! bucket B's scene model (the replay's, G.8) at the cockpit picture's own
//! clock and size. North up, the pilot's own tank at the centre with its
//! heading; around it the held `arena.map`'s ground (its colour map, shaded
//! by G.7b's heights where the feed carries them, flat otherwise), the named
//! spots lettered, the frame's range bands as faint rings, the look as a
//! wedge from the tank, and every contact the frame lists in its relation's
//! colour (doc 32 cut CF.0, Q-CF-12, as the first-person picture draws it),
//! its party letter beside its health pips, the lock ringed.
//!
//! **It draws only what the frame says** (D-21-14): the contacts are the ones
//! the sim detected, placed where the frame's bearing and coarse range put
//! them, and a contact's heading is drawn only where the frame carries one
//! (`heading_deg` on the row; today's frame does not, so none is guessed off
//! the `aspect` word). The tank's own heading is the helm's, as the
//! first-person picture turns it: the frame's carried by the viewer's
//! prediction ([`Look::hull_offset_mdeg`]), and an unreached order ticked.
//!
//! The ground is drawn once a map, size and scale into an image the scene
//! places under the tank, so a tank driving across it costs a blit a frame.
//!
//! **The inset** (cut G.13, §7.4): the same scene drawn small —
//! [`OverheadPicture::draw_inset`] — in a corner of the first-person
//! picture, over the view and under the flight HUD: the picture's own
//! heading at the top ([`Up::Camera`], so what the pilot looks at is up and
//! the hull's ray parts from it as the tape's caret does; [`Up::North`] is
//! the camera's way), the camera's reach rule at the inset's half-side, the
//! ground flat (at under a pixel a cell relief is noise, and the
//! first-person frame's palette has little room), `N` on the rim and the
//! reach in metres in the corner, no strip and no ring metres.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::cockpit::{
    ARENA_MAP_FEED, COCKPIT_FEED, MapStamp, contact_health, contact_kind, contact_party,
    map_from_feed, map_stamp, terrain_from_feed,
};
use crate::frame::Rgb;
#[cfg(feature = "encode")]
use crate::fx::CombatFxKind;
use crate::fx::{self, CombatFx, ContactTracker};
use crate::heightfield::Terrain;
use crate::math::Det as _;
use crate::overlay::{
    BORDER, DIM, FRIEND, GONE, HEALTH_PIPS, HOSTILE, LOCK, TEXT, WRECK, YOU, draw_card, ground_rgb,
    outline_rect,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::raycast::{Base, Biome, Ground, Map, SpriteKind};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// Kitty image IDs (`160..=162`) and placement IDs (`60`, `62`) for the
/// retained Overhead tactical map (`v` camera mode 2).
#[cfg(feature = "encode")]
pub const IMAGE_ID_OH_GROUND_A: u32 = 160;
/// Alternate double-buffered ground image ID for seamless re-centring.
#[cfg(feature = "encode")]
pub const IMAGE_ID_OH_GROUND_B: u32 = 161;
/// Retained RGBA overlay image ID for rings, wedge, contacts, and HUD label.
#[cfg(feature = "encode")]
pub const IMAGE_ID_OH_HUD: u32 = 162;

/// Kitty placement ID for the panning ground layer (`z = -90`).
#[cfg(feature = "encode")]
pub const PLACEMENT_OH_GROUND: u32 = 60;
/// Kitty placement ID for the tactical overlay layer (`z = -10`).
#[cfg(feature = "encode")]
pub const PLACEMENT_OH_HUD: u32 = 62;

#[cfg(feature = "encode")]
const Z_OH_GROUND: i32 = -90;
#[cfg(feature = "encode")]
const Z_OH_HUD: i32 = -10;
#[cfg(feature = "encode")]
const CHROMA_KEY_OH: Rgb = [1, 0, 1];

/// The sight assumed when the frame names none (`sight_m`), metres: the
/// stock tank's.
pub const DEFAULT_SIGHT_M: f64 = 600.0;
/// How far past the sight ring the view reaches, so the ring and a contact
/// on it sit inside the frame.
const VIEW_MARGIN: f64 = 1.15;
/// The view's radius is kept inside this band, metres.
const VIEW_BAND_M: (f64, f64) = (150.0, 3_000.0);
/// Half the first-person picture's field of view, degrees (the raycaster's
/// camera plane, 0.66, is about 33°): the look's wedge.
pub const LOOK_HALF_DEG: f64 = 33.0;
/// How many steps of relief shading the ground is drawn in (each a colour a
/// ground kind, so the PNG stays indexed).
const SHADES: i64 = 5;
/// A height step between neighbour cells worth one shade, centimetres.
const SHADE_STEP_CM: i64 = 120;

const RING: Rgb = [74, 80, 94];
const WEDGE: Rgb = [196, 196, 120];
const UNKNOWN_CONTACT: Rgb = [170, 170, 162];
const OFF_MAP: Rgb = [10, 12, 16];

/// A contact's mark colour by the viewer's relation to it (doc 32 cut CF.0,
/// Q-CF-12): friend, hostile, grey for neutral and unclassified, a wreck's.
fn relation_rgb(kind: SpriteKind) -> Rgb {
    match kind {
        SpriteKind::Hostile => HOSTILE,
        SpriteKind::Friend => FRIEND,
        SpriteKind::Unknown => UNKNOWN_CONTACT,
        SpriteKind::Wreck => WRECK,
    }
}
/// The inset's `N` sits this far inside its rim, pixels.
const RIM_INSET_PX: f64 = 6.0;

/// Which way is up in an overhead drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Up {
    /// North at the top: the camera's own way (§7.4).
    #[default]
    North,
    /// The picture's heading at the top (G.13, the HUD's inset): what the
    /// first-person picture looks at is up, the look's wedge straight up,
    /// the hull's ray parting from it where a look has swung the camera, as
    /// the heading tape's caret does.
    Camera,
}

/// How a scene is dressed: the camera's own full frame, the retained Kitty
/// HUD overlay, or the inset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dress {
    /// The full camera: north up on the world's own coordinates, the ground
    /// blitted from the cached image, the rings' metres and the strip.
    Camera,
    /// Retained Kitty HUD overlay: same tactical markers, rings, wedge,
    /// contacts, ego ray, and strip as [`Dress::Camera`], but over a
    /// chroma-key background without ground/spots (which live on the panning
    /// ground layer).
    #[cfg(feature = "encode")]
    CameraHud,
    /// The inset (G.13): `up` at the top about the tank, the ground sampled
    /// a pixel, markers at the small scale, `N` on the rim, the reach in the
    /// corner.
    Inset {
        /// Which way is up.
        up: Up,
    },
}

impl Dress {
    fn is_camera_scale(self) -> bool {
        match self {
            Dress::Camera => true,
            #[cfg(feature = "encode")]
            Dress::CameraHud => true,
            Dress::Inset { .. } => false,
        }
    }
}

/// How world points land in a scene: as they are (the camera: north up on
/// the world's own coordinates, metres, the viewport placed around the
/// tank), or about the tank, turned so a bearing is at the top and scaled
/// to pixels (the inset: the viewport is then the frame itself, centred on
/// the origin, so the tank lands on its centre pixel exactly).
#[derive(Clone, Copy, Debug)]
struct Placing {
    /// The tank, metres east and north.
    you: (f64, f64),
    /// The bearing at the top, degrees, with its sine and cosine, and the
    /// pixels a metre; `None` for the camera's untouched coordinates.
    turn: Option<(f64, (f64, f64), f64)>,
}

impl Placing {
    /// A world point as the scene places it.
    fn place(&self, p: (f64, f64)) -> (f64, f64) {
        match self.turn {
            None => p,
            Some((_, (sin, cos), px_per_m)) => {
                let (dx, dy) = (p.0 - self.you.0, p.1 - self.you.1);
                (
                    (dx * cos - dy * sin) * px_per_m,
                    (dx * sin + dy * cos) * px_per_m,
                )
            }
        }
    }

    /// A length in metres as the scene measures it.
    fn len(&self, m: f64) -> f64 {
        match self.turn {
            None => m,
            Some((_, _, px_per_m)) => m * px_per_m,
        }
    }

    /// A world bearing as the scene draws it.
    fn bearing(&self, deg: f64) -> f64 {
        match self.turn {
            None => deg,
            Some((top, _, _)) => deg - top,
        }
    }
}

/// Retained Kitty state for the oversized panning ground canvas.
#[cfg(feature = "encode")]
#[derive(Clone, Debug, PartialEq)]
struct KittyGroundKey {
    built_for: Option<MapStamp>,
    vw: usize,
    vh: usize,
    px_per_m_milli: i64,
    origin_you: (f64, f64),
}

#[cfg(feature = "encode")]
type KittyGroundPlacement = ((u16, u16, u16, u16), u32, u32);

/// The overhead as a [`Picture`]: drawn from the cockpit frame and the arena
/// map on bucket A's clock (the tank moves; the frame hash keeps an idle
/// picture off the wire). Keeps the map it built and the ground drawn at the
/// last scale.
#[derive(Debug, Default)]
pub struct OverheadPicture {
    /// What `map` was built from: the held `arena.map`'s [`MapStamp`].
    built_for: Option<MapStamp>,
    /// The held map's cells, if any.
    map: Option<Map>,
    /// Its heights, when the feed carries them (G.7b).
    terrain: Option<Terrain>,
    /// The ground drawn at a scale, and what it was drawn from.
    ground: Option<(GroundKey, Frame)>,
    /// 20 Hz sub-pulse contact dead-reckoning tracker.
    contact_tracker: ContactTracker,
    /// Last `t_ms` ingested into `active_fx`.
    last_fx_t_ms: Option<i64>,
    /// Active transient FORGE3D combat FX bursts.
    active_fx: Vec<CombatFx>,
    /// Persistent ground scorch / blast craters accumulated during the match.
    scorches: Vec<fx::ScorchMark>,
    /// Retained Kitty panning ground cache key (`2×` viewport around `origin_you`).
    #[cfg(feature = "encode")]
    kitty_ground_key: Option<KittyGroundKey>,
    /// Cached `2×` ground canvas for in-place `a=f` scorch patching.
    #[cfg(feature = "encode")]
    kitty_ground_canvas: Option<Frame>,
    /// Active Kitty image ID (`160` or `161`) for the retained ground canvas.
    #[cfg(feature = "encode")]
    kitty_ground_id: u32,
    /// Last placed `(area, src_x, src_y)` for `PLACEMENT_OH_GROUND`.
    #[cfg(feature = "encode")]
    kitty_ground_placement: Option<KittyGroundPlacement>,
    /// Hash of the last uploaded RGBA buffer for `IMAGE_ID_OH_HUD`.
    #[cfg(feature = "encode")]
    kitty_hud_hash: Option<u64>,
    /// Last placed terminal area for `PLACEMENT_OH_HUD`.
    #[cfg(feature = "encode")]
    kitty_hud_area: Option<(u16, u16, u16, u16)>,
}

/// What a drawn ground image was drawn from.
#[derive(Clone, Debug, PartialEq)]
struct GroundKey {
    built_for: Option<MapStamp>,
    px_per_m: f64,
}

impl OverheadPicture {
    /// An overhead with no map built yet.
    #[must_use]
    pub fn new() -> OverheadPicture {
        OverheadPicture::default()
    }

    /// **The inset** (G.13): this overhead drawn `side` pixels square with
    /// `up` at the top, into `frame` with its top-left at `(x, y)` and a
    /// border round it (`rect` as [`crate::overlay::inset_rect`] gives it);
    /// nothing outside that square and its border is touched. The caller
    /// (the cockpit picture) draws it before the flight HUD, so the tape,
    /// the lock's readout and the speed bar always draw over it.
    pub fn draw_inset(
        &mut self,
        frame: &mut Frame,
        cockpit: &Value,
        held: Option<&Value>,
        look: &Look,
        rect: (i64, i64, usize),
        up: Up,
    ) {
        let (x0, y0, side) = rect;
        let at = (x0, y0);
        if side == 0 {
            return;
        }
        self.ensure_map(held);
        let mut sub = Frame::new(side, side);
        overhead_scene(&sub, cockpit, held, look, self, Dress::Inset { up }).draw(&mut sub);
        for y in 0..side {
            for x in 0..side {
                let (fx, fy) = (at.0 + x as i64, at.1 + y as i64);
                if let (Ok(fx), Ok(fy)) = (usize::try_from(fx), usize::try_from(fy)) {
                    frame.put(fx, fy, sub.get(x, y));
                }
            }
        }
        let side = side as i64;
        outline_rect(frame, at.0 - 1, at.1 - 1, side + 2, side + 2, BORDER);
    }

    fn ensure_map(&mut self, map: Option<&Value>) {
        let wanted = map_stamp(map);
        if self.built_for == Some(wanted) && (map.is_some() || wanted.0.is_none()) {
            return;
        }
        self.map = map.map(map_from_feed);
        self.terrain = map.and_then(terrain_from_feed);
        self.built_for = Some(wanted);
        self.ground = None;
        #[cfg(feature = "encode")]
        {
            self.kitty_ground_key = None;
            self.kitty_ground_canvas = None;
            self.kitty_ground_placement = None;
        }
    }

    /// Return Kitty Graphics escape bytes that delete all retained Overhead
    /// image IDs (`160..=162`), and reset cached placement state.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_ground_id != 0 || self.kitty_hud_hash.is_some() {
            for id in [IMAGE_ID_OH_GROUND_A, IMAGE_ID_OH_GROUND_B, IMAGE_ID_OH_HUD] {
                crate::kitty::delete_image_bytes(&mut out, id);
            }
        }
        self.kitty_ground_key = None;
        self.kitty_ground_canvas = None;
        self.kitty_ground_id = 0;
        self.kitty_ground_placement = None;
        self.kitty_hud_hash = None;
        self.kitty_hud_area = None;
        out
    }

    /// Invalidate a specific retained Kitty image ID if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_OH_GROUND_A || image_id == IMAGE_ID_OH_GROUND_B {
            self.kitty_ground_key = None;
            self.kitty_ground_canvas = None;
            self.kitty_ground_placement = None;
        } else if image_id == IMAGE_ID_OH_HUD {
            self.kitty_hud_hash = None;
            self.kitty_hud_area = None;
        }
    }

    /// Force all retained Kitty placements to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_ground_placement = None;
        self.kitty_hud_area = None;
    }

    /// Render the Overhead tactical map as a retained 2-layer Kitty Graphics
    /// stream (`1280x` when `cell` is known and `!step_down`, `640x` fallback
    /// otherwise):
    /// - Layer 1 (`IMAGE_ID_OH_GROUND_A`/`B`, `PLACEMENT_OH_GROUND`, `z = -90`):
    ///   oversized `2×` ground + named spots canvas that pans smoothly via
    ///   sub-rectangle `a=p` as the player drives.
    /// - Layer 2 (`IMAGE_ID_OH_HUD`, `PLACEMENT_OH_HUD`, `z = -10`):
    ///   chroma-keyed RGBA tactical overlay (range rings, look wedge,
    ///   dead-reckoned contacts, ego ray, strip, and FORGE3D combat FX).
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<crate::caps::CellSize>,
        step_down: bool,
    ) -> Option<(crate::iso25d::Viewport25D, Vec<u8>)> {
        let cockpit = feeds.feed(COCKPIT_FEED)?;
        let vp = crate::cockpit_kitty::viewport_fp(area.2, area.3, cell, step_down);
        let (vw, vh) = (vp.width_px, vp.height_px);
        let placed_area = (
            area.0.saturating_add(vp.col_offset),
            area.1.saturating_add(vp.row_offset),
            vp.cols,
            vp.rows,
        );
        let mut ops = Vec::new();

        if cockpit.get("seated").and_then(Value::as_bool) == Some(false) {
            if self.kitty_ground_id != 0 {
                crate::kitty::delete_image_bytes(&mut ops, self.kitty_ground_id);
                self.kitty_ground_id = 0;
                self.kitty_ground_key = None;
                self.kitty_ground_placement = None;
            }
            let mut card = Frame::new(vw, vh);
            draw_card(&mut card, "NOT IN A COCKPIT");
            let rgba = frame_to_rgba_opaque(&card);
            let h = fnv1a_bytes(&rgba);
            if self.kitty_hud_hash != Some(h) {
                let png = crate::png::encode_rgba(vw, vh, &rgba);
                crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_OH_HUD);
                self.kitty_hud_hash = Some(h);
                self.kitty_hud_area = None;
            }
            if self.kitty_hud_area != Some(placed_area) {
                place_kitty_subrect(
                    &mut ops,
                    IMAGE_ID_OH_HUD,
                    PLACEMENT_OH_HUD,
                    placed_area,
                    (0, 0, vw as u32, vh as u32),
                    Z_OH_HUD,
                );
                self.kitty_hud_area = Some(placed_area);
            }
            return Some((vp, wrap_kitty_sync(ops)));
        }

        let held = feeds.feed(ARENA_MAP_FEED);
        self.ensure_map(held);
        fx::sync_combat_fx(&mut self.last_fx_t_ms, &mut self.active_fx, cockpit);

        let base_you = (
            num(cockpit.get("x_cm")) / 100.0,
            num(cockpit.get("y_cm")) / 100.0,
        );
        self.contact_tracker
            .step_frame(cockpit, base_you.0 as f32, base_you.1 as f32);
        let new_scorches = fx::sync_scorch_marks(
            &mut self.scorches,
            cockpit,
            &self.active_fx,
            &self.contact_tracker,
            base_you.0 as f32,
            base_you.1 as f32,
        );
        let you = (
            base_you.0 + look.dx_cm as f64 / 100.0,
            base_you.1 + look.dy_cm as f64 / 100.0,
        );
        let sight = cockpit
            .get("sight_m")
            .and_then(Value::as_f64)
            .filter(|m| *m > 0.0)
            .unwrap_or(DEFAULT_SIGHT_M);
        let reach = effective_reach(sight, look.zoom_permille);
        let px_per_m = (vw.min(vh) as f64 / 2.0) / reach;
        let px_per_m_milli = (px_per_m * 1_000.0).round() as i64;

        // 1. Panning oversized 2x ground + spots canvas (`PLACEMENT_OH_GROUND`, z = -90).
        let (gw, gh) = (vw * 2, vh * 2);
        let need_ground_rebuild = match self.kitty_ground_key.as_ref() {
            None => true,
            Some(k) => {
                if k.built_for != self.built_for
                    || k.vw != vw
                    || k.vh != vh
                    || k.px_per_m_milli != px_per_m_milli
                {
                    true
                } else {
                    let dx_px = ((you.0 - k.origin_you.0) * px_per_m).round() as i64;
                    let dy_px = ((k.origin_you.1 - you.1) * px_per_m).round() as i64;
                    let sx = vw as i64 / 2 + dx_px;
                    let sy = vh as i64 / 2 + dy_px;
                    sx < 0 || sy < 0 || sx + vw as i64 > gw as i64 || sy + vh as i64 > gh as i64
                }
            }
        };

        if need_ground_rebuild {
            let new_key = KittyGroundKey {
                built_for: self.built_for,
                vw,
                vh,
                px_per_m_milli,
                origin_you: you,
            };
            let ground_canvas = build_kitty_ground_canvas(self, held, you, px_per_m, (vw, vh));
            let next_id = if self.kitty_ground_id == IMAGE_ID_OH_GROUND_A {
                IMAGE_ID_OH_GROUND_B
            } else {
                IMAGE_ID_OH_GROUND_A
            };
            let prev_id = self.kitty_ground_id;
            let png = crate::png::encode(&ground_canvas);
            crate::kitty::transmit_bytes(&mut ops, &png, next_id);
            let sx = (vw / 2) as u32;
            let sy = (vh / 2) as u32;
            place_kitty_subrect(
                &mut ops,
                next_id,
                PLACEMENT_OH_GROUND,
                placed_area,
                (sx, sy, vw as u32, vh as u32),
                Z_OH_GROUND,
            );
            if prev_id != 0 && prev_id != next_id {
                crate::kitty::delete_image_bytes(&mut ops, prev_id);
            }
            self.kitty_ground_id = next_id;
            self.kitty_ground_key = Some(new_key);
            self.kitty_ground_canvas = Some(ground_canvas);
            self.kitty_ground_placement = Some((placed_area, sx, sy));
        } else if let Some(k) = self.kitty_ground_key.as_ref() {
            let origin_you = k.origin_you;
            let dx_px = ((you.0 - origin_you.0) * px_per_m).round() as i64;
            let dy_px = ((origin_you.1 - you.1) * px_per_m).round() as i64;
            let sx = (vw as i64 / 2 + dx_px).clamp(0, vw as i64) as u32;
            let sy = (vh as i64 / 2 + dy_px).clamp(0, vh as i64) as u32;
            if !new_scorches.is_empty()
                && self.kitty_ground_id != 0
                && let Some(canvas) = self.kitty_ground_canvas.as_mut()
            {
                for sc in &new_scorches {
                    if let Some((rx, ry, rw, rh)) =
                        stamp_scorch_on_overhead_canvas(canvas, origin_you, px_per_m, sc)
                    {
                        let mut sub = Frame::new(rw, rh);
                        for r in 0..rh {
                            for c in 0..rw {
                                sub.put(c, r, canvas.get(rx + c, ry + r));
                            }
                        }
                        let png = crate::png::encode(&sub);
                        crate::kitty::patch_frame_bytes(
                            &mut ops,
                            &png,
                            self.kitty_ground_id,
                            rx as u32,
                            ry as u32,
                        );
                    }
                }
            }
            if self.kitty_ground_placement != Some((placed_area, sx, sy))
                && self.kitty_ground_id != 0
            {
                place_kitty_subrect(
                    &mut ops,
                    self.kitty_ground_id,
                    PLACEMENT_OH_GROUND,
                    placed_area,
                    (sx, sy, vw as u32, vh as u32),
                    Z_OH_GROUND,
                );
                self.kitty_ground_placement = Some((placed_area, sx, sy));
            }
        }

        // 2. Chroma-keyed tactical overlay + contacts + FX (`IMAGE_ID_OH_HUD`, z = -10).
        let mut hud_frame = Frame::new(vw, vh);
        let scene = overhead_scene(&hud_frame, cockpit, held, look, self, Dress::CameraHud);
        scene.draw(&mut hud_frame);
        let mut rgba = frame_to_rgba_chroma(&hud_frame, CHROMA_KEY_OH);
        self.paint_overhead_fx_rgba(&mut rgba, (vw, vh), cockpit, base_you, you, px_per_m);

        let hud_hash = fnv1a_bytes(&rgba);
        if self.kitty_hud_hash != Some(hud_hash) {
            let png = crate::png::encode_rgba(vw, vh, &rgba);
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_OH_HUD);
            self.kitty_hud_hash = Some(hud_hash);
            self.kitty_hud_area = None;
        }
        if self.kitty_hud_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_OH_HUD,
                PLACEMENT_OH_HUD,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_OH_HUD,
            );
            self.kitty_hud_area = Some(placed_area);
        }

        fx::step_combat_fx(&mut self.active_fx);
        Some((vp, wrap_kitty_sync(ops)))
    }

    #[cfg(feature = "encode")]
    fn paint_overhead_fx_rgba(
        &self,
        rgba: &mut [u8],
        (vw, vh): (usize, usize),
        cockpit: &Value,
        base_you: (f64, f64),
        you: (f64, f64),
        px_per_m: f64,
    ) {
        if self.active_fx.is_empty() {
            return;
        }
        let cx = (vw / 2) as i32;
        let cy = (vh / 2) as i32;
        let contacts = cockpit
            .get("contacts")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for fx_item in &self.active_fx {
            let target_xy = match &fx_item.kind {
                CombatFxKind::Fire { target_key, .. } => target_key.as_deref().and_then(|tk| {
                    contacts.iter().enumerate().find_map(|(ci, c)| {
                        let id = c.get("id").and_then(Value::as_str);
                        let des = c.get("designator").and_then(Value::as_str);
                        if id == Some(tk) || des == Some(tk) {
                            let (b, r) = self.contact_tracker.contact_bearing_range(c, ci);
                            let pos = at_bearing(base_you, b, r);
                            let tx = cx + ((pos.0 - you.0) * px_per_m).round() as i32;
                            let ty = cy - ((pos.1 - you.1) * px_per_m).round() as i32;
                            Some((tx, ty, 12))
                        } else {
                            None
                        }
                    })
                }),
                CombatFxKind::Struck { .. } => None,
            };
            let w_i = vw as i32;
            let h_i = vh as i32;
            let mut put = |x: i32, y: i32, col: [u8; 4]| {
                if x >= 0 && x < w_i && y >= 0 && y < h_i {
                    let idx = (y as usize * vw + x as usize) * 4;
                    if let Some(dst) = rgba.get_mut(idx..idx + 4) {
                        dst.copy_from_slice(&col);
                    }
                }
            };
            fx::paint_single_fx(fx_item, (cx, cy), target_xy, (cx, cy), &mut put);
        }
    }
}

impl Picture for OverheadPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[COCKPIT_FEED, ARENA_MAP_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(cockpit) = feeds.feed(COCKPIT_FEED) else {
            return false;
        };
        if cockpit.get("seated").and_then(Value::as_bool) == Some(false) {
            draw_card(frame, "NOT IN A COCKPIT");
            return true;
        }
        let held = feeds.feed(ARENA_MAP_FEED);
        self.ensure_map(held);
        fx::sync_combat_fx(&mut self.last_fx_t_ms, &mut self.active_fx, cockpit);
        let base_you = (
            num(cockpit.get("x_cm")) / 100.0,
            num(cockpit.get("y_cm")) / 100.0,
        );
        self.contact_tracker
            .step_frame(cockpit, base_you.0 as f32, base_you.1 as f32);
        let new_scorches = fx::sync_scorch_marks(
            &mut self.scorches,
            cockpit,
            &self.active_fx,
            &self.contact_tracker,
            base_you.0 as f32,
            base_you.1 as f32,
        );
        if !new_scorches.is_empty()
            && let (Some(map), Some((gkey, gimg))) = (self.map.as_ref(), self.ground.as_mut())
        {
            let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
            let mh = map.height() as f64 * cell_m;
            for sc in &new_scorches {
                stamp_scorch_on_map_ground(gimg, mh, gkey.px_per_m, sc);
            }
        }
        let scene = overhead_scene(frame, cockpit, held, look, self, Dress::Camera);
        scene.draw(frame);
        if !self.active_fx.is_empty() {
            let (w, h) = (frame.width() as i64, frame.height() as i64);
            let base_you = (
                num(cockpit.get("x_cm")) / 100.0,
                num(cockpit.get("y_cm")) / 100.0,
            );
            let you = (
                base_you.0 + look.dx_cm as f64 / 100.0,
                base_you.1 + look.dy_cm as f64 / 100.0,
            );
            let sight = cockpit
                .get("sight_m")
                .and_then(Value::as_f64)
                .filter(|m| *m > 0.0)
                .unwrap_or(DEFAULT_SIGHT_M);
            let reach = effective_reach(sight, look.zoom_permille);
            let px_per_m = (w.min(h) as f64 / 2.0) / reach;
            let cx = (w / 2) as i32;
            let cy = (h / 2) as i32;
            let contacts = cockpit
                .get("contacts")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let tracker = &self.contact_tracker;
            fx::draw_fx_on_frame(frame, &self.active_fx, (cx, cy), (cx, cy), |tk| {
                let tk = tk?;
                contacts.iter().enumerate().find_map(|(ci, c)| {
                    let id = c.get("id").and_then(Value::as_str);
                    let des = c.get("designator").and_then(Value::as_str);
                    if id == Some(tk) || des == Some(tk) {
                        let (b, r) = tracker.contact_bearing_range(c, ci);
                        let pos = at_bearing(base_you, b, r);
                        let tx = cx + ((pos.0 - you.0) * px_per_m).round() as i32;
                        let ty = cy - ((pos.1 - you.1) * px_per_m).round() as i32;
                        Some((tx, ty, 10))
                    } else {
                        None
                    }
                })
            });
            fx::step_combat_fx(&mut self.active_fx);
        }
        true
    }
}

/// A number off the frame, `0` when absent.
fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// `(east, north)` metres `range_m` from `from` on the absolute bearing
/// `bearing_deg` (clockwise from north), as the frame places a contact.
fn at_bearing(from: (f64, f64), bearing_deg: f64, range_m: f64) -> (f64, f64) {
    let (sin, cos) = bearing_deg.to_radians().det_sin_cos();
    (from.0 + range_m * sin, from.1 + range_m * cos)
}

/// The distances the frame's range bands end at, metres, nearest first: each
/// weapon's `bands_cm` edges, and the platform's own sight. Distinct, and
/// only the ones a ring of `reach` metres could show.
#[must_use]
pub fn band_rings(cockpit: &Value, reach: f64) -> Vec<f64> {
    let mut rings: Vec<f64> = cockpit
        .get("weapons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| w.get("bands_cm").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_f64)
        .map(|cm| cm / 100.0)
        .collect();
    if let Some(sight) = cockpit.get("sight_m").and_then(Value::as_f64) {
        rings.push(sight);
    }
    rings.retain(|r| *r > 0.0 && *r <= reach * 1.5);
    rings.sort_by(f64::total_cmp);
    rings.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    rings
}

/// Compute the overhead radar reach in metres, scaled inversely by `zoom_permille`
/// (`500..=2000`; `0` or `1000` leaves the stock reach untouched).
#[must_use]
fn effective_reach(sight: f64, zoom_permille: u16) -> f64 {
    let base = (sight * VIEW_MARGIN).clamp(VIEW_BAND_M.0, VIEW_BAND_M.1);
    if zoom_permille == 0 || zoom_permille == 1000 {
        base
    } else {
        let z = f64::from(zoom_permille.clamp(500, 2000));
        (base * 1000.0 / z).clamp(75.0, 6_000.0)
    }
}

/// The overhead's scene for `frame`: `cockpit` drawn over `held` (the
/// `arena.map` keyframe, with `pic`'s map and heights built from it), the
/// tank turned and the look swung by `look`, dressed as `dress` says.
fn overhead_scene(
    frame: &Frame,
    cockpit: &Value,
    held: Option<&Value>,
    look: &Look,
    pic: &mut OverheadPicture,
    dress: Dress,
) -> Scene {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if dress.is_camera_scale() && w.min(h) >= 360 {
        2
    } else {
        1
    };
    let base_you = (
        num(cockpit.get("x_cm")) / 100.0,
        num(cockpit.get("y_cm")) / 100.0,
    );
    let you = (
        base_you.0 + look.dx_cm as f64 / 100.0,
        base_you.1 + look.dy_cm as f64 / 100.0,
    );
    let heading = num(cockpit.get("heading_mdeg"));
    let hull_deg = (heading + look.hull_offset_mdeg as f64) / 1_000.0;
    let camera_deg = (heading + look.offset_mdeg as f64) / 1_000.0;
    let sight = cockpit
        .get("sight_m")
        .and_then(Value::as_f64)
        .filter(|m| *m > 0.0)
        .unwrap_or(DEFAULT_SIGHT_M);
    let reach = if dress.is_camera_scale() {
        effective_reach(sight, look.zoom_permille)
    } else {
        (sight * VIEW_MARGIN).clamp(VIEW_BAND_M.0, VIEW_BAND_M.1)
    };
    // The view: `reach` metres from the tank to the frame's nearer edge, the
    // frame's own shape.
    let px_per_m = (w.min(h) as f64 / 2.0) / reach;
    // How the world lands in the frame: the camera's untouched coordinates,
    // or the inset's, about the tank with `up` at the top, in pixels.
    let pl = Placing {
        you,
        turn: match dress {
            Dress::Camera => None,
            #[cfg(feature = "encode")]
            Dress::CameraHud => None,
            Dress::Inset { up } => {
                let top = match up {
                    Up::North => 0.0,
                    Up::Camera => camera_deg,
                };
                Some((top, top.to_radians().det_sin_cos(), px_per_m))
            }
        },
    };
    // The scene's units and where the tank is in them: the camera's are
    // metres about the tank, north up; the inset's pixels about the origin.
    let (half_w, half_h, centre) = if dress.is_camera_scale() {
        (w as f64 / 2.0 / px_per_m, h as f64 / 2.0 / px_per_m, you)
    } else {
        (w as f64 / 2.0, h as f64 / 2.0, (0.0, 0.0))
    };
    let viewport = Viewport {
        world: (
            centre.0 - half_w,
            centre.1 - half_h,
            centre.0 + half_w,
            centre.1 + half_h,
        ),
        frame: (0, 0, w, h),
        transform: Transform::Linear,
    };
    let mut scene = Scene::new(viewport);
    let bg_fill = match dress {
        #[cfg(feature = "encode")]
        Dress::CameraHud => CHROMA_KEY_OH,
        _ => OFF_MAP,
    };
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: bg_fill,
        border: None,
    });

    // The ground: the camera's is the whole map drawn once at this scale,
    // placed by its north-west corner; the inset's is sampled a pixel,
    // turned about the tank. (Retained Kitty's `CameraHud` skips ground and
    // spots because they live on the panning `IMAGE_ID_OH_GROUND` layer.)
    let draw_ground_and_spots = match dress {
        #[cfg(feature = "encode")]
        Dress::CameraHud => false,
        _ => true,
    };
    if draw_ground_and_spots && let Some(map) = pic.map.as_ref() {
        let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
        let mh = map.height() as f64 * cell_m;
        match pl.turn {
            None => {
                let key = GroundKey {
                    built_for: pic.built_for,
                    px_per_m,
                };
                if pic.ground.as_ref().is_none_or(|(k, _)| *k != key) {
                    let mut image = ground_image(map, pic.terrain.as_ref(), px_per_m);
                    for sc in &pic.scorches {
                        stamp_scorch_on_map_ground(&mut image, mh, px_per_m, sc);
                    }
                    pic.ground = Some((key, image));
                }
                if let Some((_, image)) = pic.ground.as_ref() {
                    let at = viewport.to_px((0.0, mh));
                    scene.push(Item::Image {
                        at,
                        frame: image.clone(),
                    });
                }
            }
            Some((_, sin_cos, _)) => scene.push(Item::Image {
                at: (0, 0),
                frame: sampled_ground(map, you, sin_cos, px_per_m, (w, h)),
            }),
        }
        for spot in held
            .and_then(|m| m.get("spots"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let cell = spot.get("cell").and_then(Value::as_array);
            let n = |i: usize| cell.and_then(|c| c.get(i)).and_then(Value::as_f64);
            let (Some(x), Some(y)) = (n(0), n(1)) else {
                continue;
            };
            let at = pl.place(((x + 0.5) * cell_m, (y + 0.5) * cell_m));
            scene.push(Item::Marker {
                at,
                half: 5 * s,
                shape: Shape::Ring,
                colour: DIM,
            });
            scene.push(Item::Text {
                at,
                offset: (0, 0),
                anchor: Anchor::Centre,
                text: spot
                    .get("letter")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                colour: TEXT,
                scale: s as usize,
            });
        }
    }

    // The range bands, faint, each with its metres at its top (the camera
    // only: the inset has no room for them).
    scene.layer();
    for r in band_rings(cockpit, reach) {
        scene.push(Item::Circle {
            centre,
            radius: pl.len(r),
            colour: RING,
            dash: Some((2, 3)),
        });
        if dress.is_camera_scale() {
            scene.push(Item::Text {
                at: (you.0, you.1 + r),
                offset: (2 * s, -9 * s),
                anchor: Anchor::TopLeft,
                text: format!("{}", r.round() as i64),
                colour: RING,
                scale: s as usize,
            });
        }
    }

    // The look: a wedge from the tank as wide as the first-person picture
    // sees, as long as the sight.
    let edge = sight.min(reach);
    for side in [-LOOK_HALF_DEG, LOOK_HALF_DEG] {
        scene.push(Item::Line {
            a: centre,
            b: pl.place(at_bearing(you, camera_deg + side, edge)),
            colour: WEDGE,
            dash: Some((4, 4)),
        });
    }
    scene.push(Item::Arc {
        centre,
        radius: pl.len(edge),
        start_deg: pl.bearing(camera_deg) - LOOK_HALF_DEG,
        sweep_deg: 2.0 * LOOK_HALF_DEG,
        colour: WEDGE,
        filled: false,
    });

    // The contacts, the lock ringed, the designators over them.
    scene.layer();
    let lock = cockpit.get("lock").and_then(Value::as_str);
    let lock_name = cockpit.get("lock_designator").and_then(Value::as_str);
    let contacts: Vec<&Value> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let placed: Vec<((f64, f64), &Value)> = contacts
        .iter()
        .enumerate()
        .map(|(ci, c)| {
            let (b_deg, r_m) = if dress.is_camera_scale() {
                pic.contact_tracker.contact_bearing_range(c, ci)
            } else {
                (num(c.get("bearing_deg")), num(c.get("range_m")))
            };
            (pl.place(at_bearing(base_you, b_deg, r_m)), *c)
        })
        .collect();
    for (at, c) in &placed {
        let kind = contact_kind(c);
        let colour = relation_rgb(kind);
        if kind == SpriteKind::Wreck {
            scene.push(Item::Marker {
                at: *at,
                half: 2 * s,
                shape: Shape::Square,
                colour,
            });
            continue;
        }
        if let Some(b) = c.get("heading_deg").and_then(Value::as_f64) {
            scene.push(Item::Ray {
                at: *at,
                bearing_deg: pl.bearing(b) as f32,
                len_px: 8.0 * s as f32,
                colour,
            });
        }
        scene.push(Item::Marker {
            at: *at,
            half: 3 * s,
            shape: Shape::Disc,
            colour,
        });
    }
    for (at, c) in &placed {
        let id = c.get("id").and_then(Value::as_str);
        let name = c.get("designator").and_then(Value::as_str);
        let locked = (lock.is_some() && id == lock) || (lock_name.is_some() && name == lock_name);
        if locked {
            scene.push(Item::Marker {
                at: *at,
                half: 7 * s,
                shape: Shape::Ring,
                colour: LOCK,
            });
        }
        // The designators: the camera's only. At the inset's scale they
        // crowd the tank (a contact 200 m out is 15 pixels off), and they
        // are on the tape and the lock's readout already. The health pips
        // (G.14) under the designator, the camera's only for the same
        // reason — the inset's contacts carry theirs over their sprites in
        // the picture itself. The party letter (doc 32 cut CF.0, Q-CF-12)
        // right of the pips, centred on them, in the relation's colour, so
        // parties stay distinct inside one relation colour.
        if dress.is_camera_scale() {
            scene.push(Item::Text {
                at: *at,
                offset: (6 * s, -10 * s),
                anchor: Anchor::TopLeft,
                text: name.unwrap_or("?").to_ascii_uppercase(),
                colour: if locked { LOCK } else { TEXT },
                scale: s as usize,
            });
            if let Some(health) = contact_health(c) {
                scene.push(Item::Pips {
                    at: *at,
                    offset: (6 * s, -s),
                    lit: health.lit(),
                    of: HEALTH_PIPS,
                    size: 2 * s,
                    colour: health.rgb(),
                    dim: GONE,
                });
                if let Some(letter) = contact_party(c) {
                    scene.push(Item::Text {
                        at: *at,
                        offset: (16 * s, -3 * s),
                        anchor: Anchor::TopLeft,
                        text: letter.to_string(),
                        colour: relation_rgb(contact_kind(c)),
                        scale: s as usize,
                    });
                }
            }
        }
    }

    // The tank itself, over everything: its heading as the helm has it, an
    // unreached order ticked beside it.
    if let Some(ordered) = look.ordered_mdeg {
        scene.push(Item::Ray {
            at: centre,
            bearing_deg: pl.bearing(ordered as f64 / 1_000.0) as f32,
            len_px: 10.0 * s as f32,
            colour: LOCK,
        });
    }
    scene.push(Item::Ray {
        at: centre,
        bearing_deg: pl.bearing(hull_deg).rem_euclid(360.0) as f32,
        len_px: 14.0 * s as f32,
        colour: YOU,
    });
    scene.push(Item::Marker {
        at: centre,
        half: 3 * s,
        shape: Shape::Disc,
        colour: YOU,
    });

    if dress.is_camera_scale() {
        scene.push(Item::Label {
            at: (3 * s, 3 * s),
            text: format!("OVERHEAD  N UP  {} M", reach.round() as i64),
            colour: TEXT,
            scale: s as usize,
        });
    } else {
        let rim_m = (w.min(h) as f64 / 2.0 - RIM_INSET_PX) / px_per_m;
        scene.push(Item::Text {
            at: pl.place((you.0, you.1 + rim_m)),
            offset: (0, 0),
            anchor: Anchor::Centre,
            text: "N".to_string(),
            colour: TEXT,
            scale: 1,
        });
        let reach_m = format!("{}M", reach.round() as i64);
        scene.push(Item::Label {
            at: (w - reach_m.len() as i64 * 6 - 1, h - 9),
            text: reach_m,
            colour: TEXT,
            scale: 1,
        });
    }
    scene
}

/// Build the oversized `2 * vw × 2 * vh` ground + named spots canvas centred
/// on `origin_you` for retained Kitty sub-rectangle `a=p` panning.
#[cfg(feature = "encode")]
fn build_kitty_ground_canvas(
    pic: &mut OverheadPicture,
    held: Option<&Value>,
    origin_you: (f64, f64),
    px_per_m: f64,
    (vw, vh): (usize, usize),
) -> Frame {
    let (gw, gh) = (vw * 2, vh * 2);
    let (w, h) = (gw as i64, gh as i64);
    let s: i64 = if vw.min(vh) >= 360 { 2 } else { 1 };
    let half_w = w as f64 / 2.0 / px_per_m;
    let half_h = h as f64 / 2.0 / px_per_m;
    let viewport = Viewport {
        world: (
            origin_you.0 - half_w,
            origin_you.1 - half_h,
            origin_you.0 + half_w,
            origin_you.1 + half_h,
        ),
        frame: (0, 0, w, h),
        transform: Transform::Linear,
    };
    let mut scene = Scene::new(viewport);
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: OFF_MAP,
        border: None,
    });
    if let Some(map) = pic.map.as_ref() {
        let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
        let mh = map.height() as f64 * cell_m;
        let key = GroundKey {
            built_for: pic.built_for,
            px_per_m,
        };
        if pic.ground.as_ref().is_none_or(|(k, _)| *k != key) {
            let image = ground_image(map, pic.terrain.as_ref(), px_per_m);
            pic.ground = Some((key, image));
        }
        if let Some((_, image)) = pic.ground.as_ref() {
            let at = viewport.to_px((0.0, mh));
            scene.push(Item::Image {
                at,
                frame: image.clone(),
            });
        }
        for spot in held
            .and_then(|m| m.get("spots"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let cell = spot.get("cell").and_then(Value::as_array);
            let n = |i: usize| cell.and_then(|c| c.get(i)).and_then(Value::as_f64);
            let (Some(x), Some(y)) = (n(0), n(1)) else {
                continue;
            };
            let at = ((x + 0.5) * cell_m, (y + 0.5) * cell_m);
            scene.push(Item::Marker {
                at,
                half: 5 * s,
                shape: Shape::Ring,
                colour: DIM,
            });
            scene.push(Item::Text {
                at,
                offset: (0, 0),
                anchor: Anchor::Centre,
                text: spot
                    .get("letter")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                colour: TEXT,
                scale: s as usize,
            });
        }
    }
    let mut canvas = Frame::new(gw, gh);
    scene.draw(&mut canvas);
    for sc in &pic.scorches {
        let _ = stamp_scorch_on_overhead_canvas(&mut canvas, origin_you, px_per_m, sc);
    }
    canvas
}

fn stamp_scorch_on_map_ground(
    image: &mut Frame,
    map_height_m: f64,
    px_per_m: f64,
    sc: &fx::ScorchMark,
) {
    let cx = (f64::from(sc.x_m) * px_per_m).round() as i32;
    let cy = ((map_height_m - f64::from(sc.y_m)) * px_per_m).round() as i32;
    let r = if sc.large { 5 } else { 3 };
    let w = image.width() as i32;
    let h = image.height() as i32;
    fx::stamp_scorch_mark(cx, cy, r, sc.large, &mut |x, y, rgb| {
        if x >= 0 && x < w && y >= 0 && y < h {
            image.put(x as usize, y as usize, rgb);
        }
    });
}

#[cfg(feature = "encode")]
fn stamp_scorch_on_overhead_canvas(
    canvas: &mut Frame,
    origin_you: (f64, f64),
    px_per_m: f64,
    sc: &fx::ScorchMark,
) -> Option<(usize, usize, usize, usize)> {
    let cw = canvas.width();
    let ch = canvas.height();
    let cx = (cw as i32 / 2) + ((f64::from(sc.x_m) - origin_you.0) * px_per_m).round() as i32;
    let cy = (ch as i32 / 2) - ((f64::from(sc.y_m) - origin_you.1) * px_per_m).round() as i32;
    let r = if sc.large { 6 } else { 4 };
    let mut min_x = cw;
    let mut min_y = ch;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut touched = false;
    fx::stamp_scorch_mark(cx, cy, r, sc.large, &mut |x, y, rgb| {
        if x >= 0 && y >= 0 {
            let ux = x as usize;
            let uy = y as usize;
            if ux < cw && uy < ch {
                canvas.put(ux, uy, rgb);
                min_x = min_x.min(ux);
                min_y = min_y.min(uy);
                max_x = max_x.max(ux);
                max_y = max_y.max(uy);
                touched = true;
            }
        }
    });
    if touched {
        Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    } else {
        None
    }
}

#[cfg(feature = "encode")]
pub(crate) fn place_kitty_subrect(
    ops: &mut Vec<u8>,
    image_id: u32,
    placement_id: u32,
    (col, row, cols, rows): (u16, u16, u16, u16),
    (src_x, src_y, src_w, src_h): (u32, u32, u32, u32),
    z: i32,
) {
    crate::kitty::place_bytes(
        ops,
        &crate::kitty::Placement {
            image_id,
            placement_id,
            col,
            row,
            cols,
            rows,
            cell_x: 0,
            cell_y: 0,
            src_x,
            src_y,
            src_w,
            src_h,
            z,
        },
    );
}

#[cfg(feature = "encode")]
pub(crate) fn wrap_kitty_sync(ops: Vec<u8>) -> Vec<u8> {
    if ops.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(ops.len() + 20);
    out.extend_from_slice(crate::kitty::SYNC_BEGIN);
    out.extend_from_slice(b"\x1b7");
    out.extend_from_slice(&ops);
    out.extend_from_slice(b"\x1b8");
    out.extend_from_slice(crate::kitty::SYNC_END);
    out
}

#[cfg(feature = "encode")]
pub(crate) fn frame_to_rgba_chroma(frame: &Frame, key: Rgb) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.width() * frame.height() * 4);
    for &[r, g, b] in frame.rgb().as_chunks::<3>().0 {
        if [r, g, b] == key {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

#[cfg(feature = "encode")]
pub(crate) fn frame_to_rgba_opaque(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.width() * frame.height() * 4);
    for &[r, g, b] in frame.rgb().as_chunks::<3>().0 {
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

#[cfg(feature = "encode")]
pub(crate) fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The inset's ground: a `(w, h)` image, every pixel the colour of the cell
/// under it with `(sin, cos)`'s bearing at the top about `you` at the
/// image's centre, [`OFF_MAP`] off the map. Flat: at under a pixel a cell
/// relief shading is noise, and the first-person frame's palette has little
/// room beside the raycaster's closed set.
pub(crate) fn biome_ground_rgb(biome: Biome, g: Ground) -> Rgb {
    if biome == Biome::Temperate {
        return ground_rgb(g);
    }
    match g {
        Ground::Wall => Base::WallLit.rgb_for_biome(biome),
        Ground::Woods => Base::Woods.rgb_for_biome(biome),
        Ground::WoodsHeavy => Base::WoodsHeavy.rgb_for_biome(biome),
        _ => Base::floor(g, false).rgb_for_biome(biome),
    }
}

fn sampled_ground(
    map: &Map,
    you: (f64, f64),
    (sin, cos): (f64, f64),
    px_per_m: f64,
    (w, h): (i64, i64),
) -> Frame {
    let biome = map.biome();
    let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
    let (cols, rows) = (map.width() as i64, map.height() as i64);
    let mut image = Frame::new(w.max(0) as usize, h.max(0) as usize);
    for py in 0..h {
        for px in 0..w {
            // The pixel's centre, metres right of and up from the tank in
            // the turned frame, then back to east and north.
            let dx = (px as f64 + 0.5 - w as f64 / 2.0) / px_per_m;
            let dy = (h as f64 / 2.0 - py as f64 - 0.5) / px_per_m;
            let east = you.0 + dx * cos + dy * sin;
            let north = you.1 - dx * sin + dy * cos;
            let (cx, cy) = (
                (east / cell_m).floor() as i64,
                (north / cell_m).floor() as i64,
            );
            let rgb = if (0..cols).contains(&cx) && (0..rows).contains(&cy) {
                biome_ground_rgb(biome, map.get(cx, cy))
            } else {
                OFF_MAP
            };
            image.put(px as usize, py as usize, rgb);
        }
    }
    image
}

/// The whole of `map` drawn at `px_per_m`, north up, its first row the
/// map's northern edge: a cell's ground colour, shaded by `terrain`'s relief
/// where it is held (lit from the north-west, in [`SHADES`] steps) and flat
/// where it is not.
fn ground_image(map: &Map, terrain: Option<&Terrain>, px_per_m: f64) -> Frame {
    let biome = map.biome();
    let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
    let px_per_cell = (cell_m * px_per_m).max(0.01);
    let fw = (map.width() as f64 * px_per_cell).ceil().max(1.0) as usize;
    let fh = (map.height() as f64 * px_per_cell).ceil().max(1.0) as usize;
    let mut image = Frame::new(fw, fh);
    let rows = map.height() as i64;
    for py in 0..fh {
        let cy = rows - 1 - (py as f64 / px_per_cell) as i64;
        for px in 0..fw {
            let cx = (px as f64 / px_per_cell) as i64;
            let base = biome_ground_rgb(biome, map.get(cx, cy));
            let rgb = match terrain {
                Some(t) => shade(base, relief(t, cx, cy)),
                None => base,
            };
            image.put(px, py, rgb);
        }
    }
    image
}

/// How lit a cell is, `-SHADES/2..=SHADES/2`: its height over its
/// north-west neighbour's, in [`SHADE_STEP_CM`] steps.
pub(crate) fn relief(t: &Terrain, x: i64, y: i64) -> i64 {
    let rise = i64::from(t.elev_cm(x, y)) - i64::from(t.elev_cm(x - 1, y + 1));
    (rise / SHADE_STEP_CM).clamp(-(SHADES / 2), SHADES / 2)
}

/// `rgb` a step of relief lighter or darker (a tenth a step).
pub(crate) fn shade(rgb: Rgb, step: i64) -> Rgb {
    let f = 10 + step;
    rgb.map(|c| u8::try_from((i64::from(c) * f / 10).clamp(0, 255)).unwrap_or(u8::MAX))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "cockpit" => include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
            _ => include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        };
        serde_json::from_str(text).expect("fixture")
    }

    fn feeds(map: bool) -> HashMap<String, Value> {
        let mut f = HashMap::new();
        f.insert(COCKPIT_FEED.to_string(), fixture("cockpit"));
        if map {
            f.insert(ARENA_MAP_FEED.to_string(), fixture("map"));
        }
        f
    }

    fn draw(f: &HashMap<String, Value>, look: &Look, w: usize, h: usize) -> Frame {
        let mut frame = Frame::new(w, h);
        assert!(OverheadPicture::new().draw(&mut frame, f, look));
        frame
    }

    fn has(f: &Frame, c: Rgb) -> bool {
        (0..f.height()).any(|y| (0..f.width()).any(|x| f.get(x, y) == c))
    }

    /// The fixture: the tank white at the centre, the blue and red contacts
    /// and the unknown ones in their colours, the lock ringed, the ground
    /// under them, a small palette.
    #[test]
    fn the_overhead_draws_the_tank_the_contacts_and_the_ground() {
        let f = draw(&feeds(true), &Look::default(), 640, 276);
        assert_eq!(f.get(320, 138), YOU, "the tank at the centre");
        for want in [
            FRIEND,
            HOSTILE,
            UNKNOWN_CONTACT,
            LOCK,
            WEDGE,
            RING,
            ground_rgb(crate::raycast::Ground::Grass),
            ground_rgb(crate::raycast::Ground::Water),
        ] {
            assert!(has(&f, want), "{want:?} drawn");
        }
        assert!(f.count_colours(256) <= 64, "a small palette");
        // The look swings the wedge and nothing else orders anything.
        let swung = draw(
            &feeds(true),
            &Look {
                offset_mdeg: 90_000,
                ..Look::default()
            },
            640,
            276,
        );
        assert_ne!(f.hash(), swung.hash());
        // The same feeds draw the same.
        assert_eq!(
            f.hash(),
            draw(&feeds(true), &Look::default(), 640, 276).hash()
        );
    }

    /// No map: the field is dark, the tank and its contacts still drawn; no
    /// frame: nothing; out of the seat: a card.
    #[test]
    fn missing_feeds_are_drawn_or_refused_as_the_cockpit_is() {
        let bare = draw(&feeds(false), &Look::default(), 320, 220);
        assert!(!has(&bare, ground_rgb(crate::raycast::Ground::Grass)));
        assert!(has(&bare, FRIEND));
        let mut frame = Frame::new(320, 220);
        let empty: HashMap<String, Value> = HashMap::new();
        assert!(!OverheadPicture::new().draw(&mut frame, &empty, &Look::default()));
        let mut out = feeds(true);
        if let Some(c) = out.get_mut(COCKPIT_FEED) {
            c["seated"] = false.into();
        }
        assert!(OverheadPicture::new().draw(&mut frame, &out, &Look::default()));
        assert!(!has(&frame, FRIEND), "a card, no contacts");
    }

    /// The inset (G.13): the overhead drawn small into a corner, nothing
    /// outside its square and border touched; the tank at the centre with
    /// its ray straight up when the picture's heading is up (the fixture
    /// faces 045), north-east-ish when north is; the contacts and the ground
    /// in it; a small palette; and the same feeds draw the same.
    #[test]
    fn the_inset_is_the_overhead_small_and_turned() {
        let f = feeds(true);
        let draw = |up: Up, look: &Look| {
            let mut frame = Frame::new(640, 256);
            OverheadPicture::new().draw_inset(
                &mut frame,
                f.get(COCKPIT_FEED).expect("cockpit"),
                f.get(ARENA_MAP_FEED),
                look,
                (8, 8, 104),
                up,
            );
            frame
        };
        let camera = draw(Up::Camera, &Look::default());
        let (cx, cy) = (8 + 52, 8 + 52);
        assert_eq!(camera.get(cx, cy), YOU, "the tank at the centre");
        for d in 4..12 {
            assert_eq!(camera.get(cx, cy - d), YOU, "its ray straight up, {d}");
        }
        assert_ne!(camera.get(cx + 6, cy - 6), YOU);
        let north = draw(Up::North, &Look::default());
        assert_eq!(north.get(cx, cy), YOU);
        assert_eq!(north.get(cx + 6, cy - 6), YOU, "045: up and right");
        assert_ne!(north.get(cx, cy - 10), YOU);
        for frame in [&camera, &north] {
            let inside = |c: Rgb| (8..112).any(|y| (8..112).any(|x| frame.get(x, y) == c));
            for want in [
                FRIEND,
                HOSTILE,
                UNKNOWN_CONTACT,
                LOCK,
                WEDGE,
                TEXT,
                ground_rgb(crate::raycast::Ground::Grass),
            ] {
                assert!(inside(want), "{want:?} drawn");
            }
            assert_eq!(frame.get(7, 7), BORDER, "the border round it");
            // Outside the square and its border the frame is as it was.
            for (x, y) in [(6, 6), (113, 113), (6, 60), (60, 113), (300, 100)] {
                assert_eq!(frame.get(x, y), [0, 0, 0], "({x}, {y}) untouched");
            }
            assert!(frame.count_colours(64) <= 40, "a small palette");
        }
        // A look swings the camera-up inset and leaves the tank's ray where
        // the hull is; north-up keeps the ground and swings the wedge.
        let look = Look {
            offset_mdeg: 90_000,
            ..Look::default()
        };
        let swung = draw(Up::Camera, &look);
        assert_ne!(swung.hash(), camera.hash());
        assert_eq!(
            swung.get(cx - 8, cy),
            YOU,
            "the hull's ray, now to the left"
        );
        assert_eq!(
            draw(Up::Camera, &look).hash(),
            swung.hash(),
            "deterministic"
        );
    }

    /// Doc 21 cut G.14: the camera marks an identified contact's health
    /// beside its mark, under the designator — the fixture's BA one red pip
    /// and two grey; RB (no word), X3 and X4 (unidentified) nothing — and
    /// draws none once the word is gone; the inset draws none, as it draws
    /// no designators.
    #[test]
    fn the_overhead_marks_a_contact_s_health_beside_it() {
        use crate::overlay::{BAD, GONE};
        let count = |f: &Frame, c: Rgb| {
            (0..f.height())
                .flat_map(|y| (0..f.width()).map(move |x| (x, y)))
                .filter(|&(x, y)| f.get(x, y) == c)
                .collect::<Vec<_>>()
        };
        let f = feeds(true);
        let full = draw(&f, &Look::default(), 640, 480);
        let red = count(&full, BAD);
        let grey = count(&full, GONE);
        assert_eq!(red.len(), 16, "one 4x4 pip at 2x: {red:?}");
        assert_eq!(grey.len(), 32, "two unlit: {grey:?}");
        let (x, y) = red[0];
        assert!(grey.contains(&(x + 6, y)) && grey.contains(&(x + 12, y)));
        // Beside BA's mark: right of it, and under its designator. BA is
        // hostile to the fixture's red pilot (doc 32 cut CF.0, Q-CF-12): its
        // mark is the hostile colour, and so is its party letter `B`, right
        // of the pips — the mark is the hostile pixels left of them.
        let hostile = count(&full, HOSTILE);
        let mark: Vec<_> = hostile.iter().copied().filter(|&(hx, _)| hx < x).collect();
        assert!(!mark.is_empty(), "BA's mark");
        let (bx, by) = mark
            .iter()
            .fold((0, 0), |(sx, sy), &(x, y)| (sx + x, sy + y));
        let (bx, by) = (bx / mark.len(), by / mark.len());
        let letter: Vec<_> = hostile
            .iter()
            .copied()
            .filter(|&(hx, hy)| hx >= x + 16 && hx < x + 30 && hy + 8 >= y && hy < y + 12)
            .collect();
        assert!(
            letter.len() > 20,
            "BA's party letter right of its pips: {letter:?}"
        );
        let (lx0, ly0) = letter
            .iter()
            .fold((usize::MAX, usize::MAX), |(a, b), &(x, y)| {
                (a.min(x), b.min(y))
            });
        assert_eq!(
            (lx0, ly0),
            (x + 20, y - 4),
            "the B's top-left, 2s past the pips"
        );
        assert!(
            x > bx && x < bx + 24,
            "right of BA ({bx}, {by}): ({x}, {y})"
        );
        assert!(
            y + 2 >= by && y <= by + 4,
            "level with BA ({bx}, {by}): ({x}, {y})"
        );

        let mut quiet = feeds(true);
        if let Some(c) = quiet.get_mut(COCKPIT_FEED) {
            c["contacts"][0]["health"] = Value::Null;
        }
        let none = draw(&quiet, &Look::default(), 640, 480);
        assert!(!has(&none, BAD) && !has(&none, GONE), "no word, no pips");

        let mut inset = Frame::new(640, 256);
        OverheadPicture::new().draw_inset(
            &mut inset,
            f.get(COCKPIT_FEED).expect("cockpit"),
            f.get(ARENA_MAP_FEED),
            &Look::default(),
            (8, 8, 104),
            Up::Camera,
        );
        assert!(!has(&inset, BAD), "the inset carries no pips");
    }

    /// The rings are the weapons' band edges and the sight, distinct, out to
    /// what the view could show.
    #[test]
    fn the_band_rings_are_the_frames_own() {
        let cockpit = fixture("cockpit");
        assert_eq!(band_rings(&cockpit, 690.0), vec![400.0, 600.0, 1_000.0]);
        assert_eq!(band_rings(&serde_json::json!({}), 690.0), Vec::<f64>::new());
    }

    /// Relief shades the ground in steps, flat ground unshaded.
    #[test]
    fn relief_shades_in_steps() {
        assert_eq!(shade([100, 100, 100], 0), [100, 100, 100]);
        assert_eq!(shade([100, 100, 100], 2), [120, 120, 120]);
        assert_eq!(shade([100, 100, 100], -2), [80, 80, 80]);
        let mut t = Terrain::new(Map::new(3, 3, 1000));
        t.set_elev_cm(1, 1, 600);
        assert_eq!(relief(&t, 1, 1), SHADES / 2, "a rise is lit");
        assert_eq!(relief(&t, 2, 0), -(SHADES / 2), "below it, shadow");
        assert_eq!(relief(&t, 0, 0), 0, "flat");
    }

    #[cfg(feature = "encode")]
    #[test]
    fn retained_kitty_overhead_scales_to_1280_and_pans_ground_via_subrect() {
        use crate::caps::CellSize;
        let f = feeds(true);
        let mut pic = OverheadPicture::new();
        let area = (1, 1, 128, 36);
        let cell = Some(CellSize {
            width: 10,
            height: 20,
        });

        // 1. Known cell geometry -> 1280x viewport.
        let (vp, bytes1) = pic
            .draw_kitty(&f, &Look::default(), area, cell, false)
            .expect("rendered");
        assert_eq!(vp.width_px, 1280);
        let s1 = String::from_utf8_lossy(&bytes1);
        assert!(s1.contains("i=160"), "uploads ground canvas id=160");
        assert!(s1.contains("p=60"), "places ground placement p=60");
        assert!(s1.contains("i=162"), "uploads HUD overlay id=162");
        assert!(s1.contains("p=62"), "places HUD overlay p=62");

        // 2. Identical second tick -> zero bytes.
        let (_, bytes2) = pic
            .draw_kitty(&f, &Look::default(), area, cell, false)
            .expect("rendered");
        assert!(bytes2.is_empty(), "idle tick emits 0 bytes");

        // 3. Driving offset -> pans ground via p=60 without re-uploading i=160/i=161.
        let look_moved = Look {
            dx_cm: 2_500,
            dy_cm: 1_500,
            ..Look::default()
        };
        let (_, bytes3) = pic
            .draw_kitty(&f, &look_moved, area, cell, false)
            .expect("rendered");
        let s3 = String::from_utf8_lossy(&bytes3);
        assert!(s3.contains("p=60"), "updates ground sub-rect placement");
        assert!(
            !s3.contains("a=t,f=100,t=d,i=160") && !s3.contains("a=t,f=100,t=d,i=161"),
            "does not re-upload ground PNG while inside 2x margin"
        );

        // 4. Unknown cell geometry -> 640x fallback.
        let (vp_fb, _) = pic
            .draw_kitty(&f, &Look::default(), area, None, false)
            .expect("fallback rendered");
        assert_eq!(vp_fb.width_px, 640);

        // 5. Overhead radar zoom changes reach in both software and retained Kitty.
        let norm_frame = draw(&f, &Look::default(), 640, 276);
        let zoom_in_look = Look {
            zoom_permille: 2000,
            ..Look::default()
        };
        let zoomed_frame = draw(&f, &zoom_in_look, 640, 276);
        assert_ne!(norm_frame.hash(), zoomed_frame.hash());
    }
}
