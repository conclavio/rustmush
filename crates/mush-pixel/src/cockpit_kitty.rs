//! **Retained Kitty graphics compositor for the First-Person cockpit view**.
//!
//! When the pilot views the arena from `CockpitCamera::FirstPerson` in a
//! Kitty-capable terminal, the First-Person view drives the Kitty graphics
//! protocol as a **retained multi-layer hardware compositor** (`a=t` upload +
//! `a=p` placement movement), matching the 2.5D tactical view (`iso25d`) and
//! scaling up to `1280×720` (`640×360` step-down fallback) when terminal cell
//! pixel geometry is reported (`640` / `320` fallback when unreported):
//!
//! 1. **Layer 1 — Overscanned `2×` Panoramic World Canvas (`IMAGE_ID_FP_WORLD_A`
//!    / `IMAGE_ID_FP_WORLD_B`, `PLACEMENT_FP_WORLD`, `z = -100`)**:
//!    - Renders a `2×`-wide panoramic ground + sky + 3D conifer trees +
//!      outpost structures canvas (`canvas_w = vp.width_px * 2`,
//!      `canvas_h = vp.height_px`) centered on `origin_cam` and `origin_eye_cm`,
//!      retaining the per-pixel `depth` buffer for unit billboard occlusion.
//!    - Pure camera yaw (`q`/`e` look or `a`/`d` turn at fixed `x_cm, y_cm,
//!      eye_cm`) within the `±30°` overscan window pans the world horizontally
//!      via a single `a=p` source-rect shift (`src_x`) with **zero**
//!      re-raycasting and **zero** PNG re-uploads.
//!    - Camera translation or yaw beyond the overscan margin bakes a new
//!      panoramic canvas centered on `cam` and ping-pongs the world image ID
//!      (`140` ↔ `141`).
//! 2. **Layer 2 — Retained First-Person Unit Billboard Sprites
//!    (`IMAGE_ID_FP_SHEET_BASE = 5000..`, `PLACEMENT_FP_UNIT_BASE = 101..`,
//!    `z = -85..-10`)**:
//!    - Lazily uploads `(view, art, paint, size)` RGBA sprite sheets (`View::Fp`
//!      or `View::Iso` via [`bake::View::for_depression`]) once via `a=t` with
//!      `(SHEET_PAD_X, SHEET_PAD_Y)` transparent padding.
//!    - Each tick projects visible contacts, checks `PanoramicWorld::depth` for
//!      hull-down ridge occlusion, clips `src_h` at the ridge crest when
//!      partially hull-down, and moves/animates the contact via `a=p`.
//!    - Contacts that are 100% occluded or tiny/faint (`< MARK_FAR_PX` or in the
//!      last third of fog) are bracketed on the HUD layer so no contact reported
//!      by the sim is ever invisible.
//! 3. **Layer 3 — Retained Minimap Inset (`IMAGE_ID_FP_INSET = 142`,
//!    `PLACEMENT_FP_INSET = 42`, `z = -5`)**.
//! 4. **Layer 4 — Retained Flight HUD Overlay (`IMAGE_ID_FP_HUD = 143`,
//!    `PLACEMENT_FP_HUD = 43`, `z = -3`)**.
//! 5. **Layer 5 — Movable Target-Lock Bracket (`IMAGE_ID_FP_LOCK = 144`,
//!    `PLACEMENT_FP_LOCK = 44`, `z = -2`)**:
//!    - Moves across the viewport via `a=p` as the locked target moves or the
//!      camera turns without re-uploading the bracket image.
//! 6. **Layer 6 — Retained RGBA Canopy Glass Overlay (`IMAGE_ID_FP_GLASS = 145`,
//!    `PLACEMENT_FP_GLASS = 45`, `z = -1`)**:
//!    - True translucent RGBA canopy glass reflections, frost vignette, angled
//!      mullions, status lamps, and ballistic fractures; toggling `b` adds or
//!      deletes `PLACEMENT_FP_GLASS` in a single escape sequence.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::bake::{self, material_from_code};
use crate::caps::CellSize;
use crate::cockpit::{
    ANIMATION_DRAWS, ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture, INSET_UP, camera,
    canopy_glass_from, contact_sprite_at_bearing_range, eye_at_cam_cm, hud, sensor_height_cm,
};
use crate::frame::{Frame, Rgb};
use crate::fx::{self, CombatFxKind};
use crate::heightfield::{self, Terrain};
use crate::iso25d::{Viewport25D, material_rgb, viewport_25d};
use crate::kitty::{self, Placement};
use crate::look::Facing;
use crate::math::Det as _;
use crate::overlay::{self, CanopyGlass, Hud, LockAt, PANEL};
use crate::picture::{Feeds, Look};
use crate::png;
use crate::raycast::{
    self, Camera, FOG_STEPS, MARK_FAR_PX, MARK_MIN_PX, Map, SpriteAt, SpriteKind, fog_step,
};

/// Horizontal transparent padding around each cell in an uploaded First-Person
/// sprite sheet (`X=0` sub-cell shifts via `src_x`).
const SHEET_PAD_X: usize = 16;
/// Vertical transparent padding around each cell in an uploaded First-Person
/// sprite sheet (`Y=0` sub-cell shifts via `src_y`).
const SHEET_PAD_Y: usize = 24;

/// Chroma-key background colour used when rasterizing opaque HUD/glass primitives
/// into an RGBA overlay layer.
const CHROMA_KEY: Rgb = [254, 1, 253];

// ---- Stable Kitty Image & Placement IDs for First-Person -------------------

/// Ping-pong image IDs for the overscanned `2×` First-Person panoramic world canvas.
pub const IMAGE_ID_FP_WORLD_A: u32 = 140;
/// Secondary ping-pong image ID for the First-Person panoramic world canvas.
pub const IMAGE_ID_FP_WORLD_B: u32 = 141;
/// Image ID for the retained top-left minimap inset (`z = -5`).
pub const IMAGE_ID_FP_INSET: u32 = 142;
/// Image ID for the retained Flight HUD overlay (`z = -3`).
pub const IMAGE_ID_FP_HUD: u32 = 143;
/// Image ID for the movable target-lock bracket + readout (`z = -2`).
pub const IMAGE_ID_FP_LOCK: u32 = 144;
/// Image ID for the retained RGBA Canopy Glass overlay (`z = -1`).
pub const IMAGE_ID_FP_GLASS: u32 = 145;
/// Image ID for transient FORGE3D weapon tracer / muzzle flash / impact FX (`z = -4`).
pub const IMAGE_ID_FP_FX: u32 = 146;
/// Base image ID (`5000..`) for lazily uploaded `(View, Art, Paint, size)`
/// First-Person unit sprite sheets.
pub const IMAGE_ID_FP_SHEET_BASE: u32 = 5000;

/// Placement ID (`p=40`) for the panoramic world canvas.
pub const PLACEMENT_FP_WORLD: u32 = 40;
/// Placement ID (`p=42`) for the minimap inset.
pub const PLACEMENT_FP_INSET: u32 = 42;
/// Placement ID (`p=43`) for the Flight HUD overlay.
pub const PLACEMENT_FP_HUD: u32 = 43;
/// Placement ID (`p=44`) for the movable target-lock bracket.
pub const PLACEMENT_FP_LOCK: u32 = 44;
/// Placement ID (`p=45`) for the RGBA Canopy Glass overlay.
pub const PLACEMENT_FP_GLASS: u32 = 45;
/// Placement ID (`p=46`) for transient FORGE3D combat FX.
pub const PLACEMENT_FP_FX: u32 = 46;
/// Base placement ID (`p=101..`) for visible First-Person contact sprites.
pub const PLACEMENT_FP_UNIT_BASE: u32 = 101;

const Z_FP_WORLD: i32 = -100;
const Z_FP_WRECK_BASE: i32 = -85;
const Z_FP_UNIT_BASE: i32 = -60;
const Z_FP_INSET: i32 = -5;
const Z_FP_FX: i32 = -4;
const Z_FP_HUD: i32 = -3;
const Z_FP_LOCK: i32 = -2;
const Z_FP_GLASS: i32 = -1;

/// Compute the First-Person Kitty viewport dimensions:
/// - When terminal cell pixel geometry is reported (`cell == Some(c)`), scales
///   up to `1280×720` (`640×360` when `step_down` is true) via [`viewport_25d`].
/// - When `cell == None` (unreported cell pixel dimensions), falls back to
///   `640` wide (`320` wide when `step_down` is true) at the aspect-preserving
///   cell box inside `(area_cols, area_rows)`.
#[must_use]
pub fn viewport_fp(
    area_cols: u16,
    area_rows: u16,
    cell: Option<CellSize>,
    step_down: bool,
) -> Viewport25D {
    if let Some(c) = cell {
        return viewport_25d(area_cols, area_rows, Some(c), step_down);
    }
    let width_px: usize = if step_down { 320 } else { 640 };
    let def = CellSize::DEFAULT;
    let avail_cols = usize::from(area_cols.max(1));
    let avail_rows = usize::from(area_rows.max(1));
    let aw = avail_cols * usize::from(def.width.max(1));
    let ah = avail_rows * usize::from(def.height.max(1));
    let height_px = ((width_px * ah / aw.max(1)).clamp(width_px / 4, width_px * 3 / 4) & !1).max(2);

    let cw_u64 = u64::from(def.width.max(1));
    let ch_u64 = u64::from(def.height.max(1));
    let fw = width_px.max(1) as u64;
    let fh = height_px.max(1) as u64;
    let ac = avail_cols as u64;
    let ar = avail_rows as u64;
    let wide = (ar * ch_u64 * fw / fh / cw_u64).max(1);
    let (c, r) = if wide <= ac {
        (wide, ar)
    } else {
        (ac, (ac * cw_u64 * fh / fw / ch_u64).clamp(1, ar))
    };
    let cols = u16::try_from(c).unwrap_or(1);
    let rows = u16::try_from(r).unwrap_or(1);
    let col_offset = u16::try_from((ac.saturating_sub(c)) / 2).unwrap_or(0);
    let cell_w = u16::try_from((width_px / usize::from(cols.max(1))).max(1)).unwrap_or(8);
    let cell_h = u16::try_from((height_px / usize::from(rows.max(1))).max(1)).unwrap_or(16);

    Viewport25D {
        width_px,
        height_px,
        cols,
        rows,
        col_offset,
        row_offset: 0,
        cell_w,
        cell_h,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum FpPaintKey {
    Hostile = 0,
    Friend = 1,
    Unknown = 2,
    Wreck = 3,
}

impl FpPaintKey {
    fn from_kind(k: SpriteKind) -> Self {
        match k {
            SpriteKind::Hostile => Self::Hostile,
            SpriteKind::Friend => Self::Friend,
            SpriteKind::Unknown => Self::Unknown,
            SpriteKind::Wreck => Self::Wreck,
        }
    }

    fn to_kind(self) -> SpriteKind {
        match self {
            Self::Hostile => SpriteKind::Hostile,
            Self::Friend => SpriteKind::Friend,
            Self::Unknown => SpriteKind::Unknown,
            Self::Wreck => SpriteKind::Wreck,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FpSheetKey {
    view: bake::View,
    art: bake::Art,
    paint: FpPaintKey,
    size: u8,
}

impl FpSheetKey {
    fn image_id(self) -> u32 {
        let v = match self.view {
            bake::View::Fp => 0u32,
            bake::View::Iso => 1u32,
        };
        let l = match self.art {
            bake::Art::Model(slot) => u32::try_from(slot.min(127)).unwrap_or(127),
            bake::Art::Sheet(look) => 128 + u32::try_from(look.min(127)).unwrap_or(127),
        };
        let p = self.paint as u32;
        let size = u32::from(self.size.min(15));
        IMAGE_ID_FP_SHEET_BASE + (v * 16_384) + (l * 64) + (p * 16) + size
    }
}

fn look_index(unit_type: Option<&str>) -> usize {
    unit_type
        .and_then(crate::look::index_for_unit)
        .or_else(|| crate::look::index_for_unit(crate::look::UNKNOWN))
        .unwrap_or(0)
}

fn render_fp_sheet_rgba(key: FpSheetKey) -> (usize, usize, Vec<u8>) {
    let Some(sized) = bake::art_chain_for_view(key.art, key.view)
        .and_then(|(chain, _)| chain.sizes.get(usize::from(key.size)))
    else {
        return (1, 1, vec![0, 0, 0, 0]);
    };
    let n = usize::from(sized.size);
    let stride_x = n + SHEET_PAD_X;
    let stride_y = n + SHEET_PAD_Y;
    let sheet_w = 6 * stride_x + SHEET_PAD_X;
    let sheet_h = 8 * stride_y + SHEET_PAD_Y;
    let mut rgba = vec![0u8; sheet_w * sheet_h * 4];
    let kind = key.paint.to_kind();

    let rx = (n as i32 * 3 / 10).max(3);
    let ry = (n as i32 / 8).max(1);

    for (fi, &facing) in Facing::ALL.iter().enumerate() {
        let dst_y0 = SHEET_PAD_Y + fi * stride_y;
        for pi in 0..bake::POSES.len() {
            let dst_x0 = SHEET_PAD_X + pi * stride_x;
            let col_kind = if pi == bake::POSE_WRECK {
                SpriteKind::Wreck
            } else {
                kind
            };
            let Some(f) = sized.frame(facing, pi) else {
                continue;
            };

            let cx = i32::from(f.anchor.0);
            let cy = i32::from(f.anchor.1);
            for dy in -ry..=ry {
                let ly = cy + dy;
                if ly < 0 || ly >= n as i32 {
                    continue;
                }
                for dx in -rx..=rx {
                    let lx = cx + dx;
                    if lx < 0 || lx >= n as i32 {
                        continue;
                    }
                    let d = (dx * dx * 100) / (rx * rx).max(1) + (dy * dy * 100) / (ry * ry).max(1);
                    if d <= 100 {
                        let off = ((dst_y0 + ly as usize) * sheet_w + (dst_x0 + lx as usize)) * 4;
                        if let Some(dst) = rgba.get_mut(off..off + 4) {
                            dst.copy_from_slice(&[10, 12, 14, 150]);
                        }
                    }
                }
            }

            let rim = n >= 16;
            for lr in 0..n {
                let dst_r = dst_y0 + lr;
                for lc in 0..n {
                    let dst_c = dst_x0 + lc;
                    let code = f.code(lc as i64, lr as i64);
                    if let Some(mat) = material_from_code(code) {
                        let [rr, gg, bb] = material_rgb(mat, col_kind);
                        let off = (dst_r * sheet_w + dst_c) * 4;
                        if let Some(dst) = rgba.get_mut(off..off + 4) {
                            dst.copy_from_slice(&[rr, gg, bb, 255]);
                        }
                    } else if rim {
                        let adj = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)]
                            .iter()
                            .any(|&(dr, dc)| f.code(lc as i64 + dc, lr as i64 + dr) != 0);
                        if adj {
                            let off = (dst_r * sheet_w + dst_c) * 4;
                            if let Some(dst) = rgba.get_mut(off..off + 4) {
                                dst.copy_from_slice(&[12, 14, 18, 220]);
                            }
                        }
                    }
                }
            }
        }
    }

    (sheet_w, sheet_h, rgba)
}

/// Overscanned `2×`-wide First-Person panoramic world canvas (`canvas_w = vp_w * 2`)
/// and per-pixel depth buffer.
#[derive(Debug)]
struct PanoramicWorld {
    canvas_w: usize,
    vp_w: usize,
    vp_h: usize,
    origin_cam: Camera,
    origin_eye_cm: Option<i64>,
    origin_sensor_eye_cm: Option<i64>,
    has_terrain: bool,
    ground_frame: Frame,
    depth: Vec<f32>,
}

impl PanoramicWorld {
    fn bake(
        arena: &Map,
        terrain: Option<&Terrain>,
        origin_cam: Camera,
        origin_eye_cm: Option<i64>,
        origin_sensor_eye_cm: Option<i64>,
        vp_w: usize,
        vp_h: usize,
    ) -> Self {
        let canvas_w = (vp_w * 2).max(4);
        let canvas_h = vp_h.max(2);
        let mut ground_frame = Frame::new(canvas_w, canvas_h);
        let depth = match terrain {
            Some(t) => heightfield::render_panoramic_ground(
                &mut ground_frame,
                t,
                &origin_cam,
                origin_eye_cm,
                vp_w,
            ),
            None => raycast::render_panoramic_ground(
                &mut ground_frame,
                arena,
                &origin_cam,
                origin_sensor_eye_cm,
                vp_w,
            ),
        };
        Self {
            canvas_w,
            vp_w,
            vp_h,
            origin_cam,
            origin_eye_cm,
            origin_sensor_eye_cm,
            has_terrain: terrain.is_some(),
            ground_frame,
            depth,
        }
    }

    /// Return the horizontal source-rect offset `src_x` inside `ground_frame`
    /// for `cam` when the camera is at the same `(x_cm, y_cm, eye_cm)` and
    /// within the `±30°` panoramic overscan window; `None` if a rebake is needed.
    fn src_x_for(
        &self,
        cam: &Camera,
        eye_cm: Option<i64>,
        sensor_eye_cm: Option<i64>,
        has_terrain: bool,
    ) -> Option<u32> {
        if self.has_terrain != has_terrain
            || cam.x_cm != self.origin_cam.x_cm
            || cam.y_cm != self.origin_cam.y_cm
            || eye_cm != self.origin_eye_cm
            || sensor_eye_cm != self.origin_sensor_eye_cm
        {
            return None;
        }
        let delta_mdeg = (i64::from(cam.heading_mdeg) - i64::from(self.origin_cam.heading_mdeg)
            + 180_000)
            .rem_euclid(360_000)
            - 180_000;
        if delta_mdeg.abs() > 30_000 {
            return None;
        }
        let delta_rad = (delta_mdeg as f32 * 0.001).to_radians();
        let (sin_d, cos_d) = delta_rad.det_sin_cos();
        let tan_d = sin_d / cos_d.max(0.001);
        let shift_px = ((tan_d / crate::raycast::PLANE) * (self.vp_w as f32 * 0.5)).round() as i32;
        let center_x = ((self.canvas_w.saturating_sub(self.vp_w)) / 2) as i32;
        let src_x = center_x + shift_px;
        let max_src_x = self.canvas_w.saturating_sub(self.vp_w) as i32;
        if src_x < 0 || src_x > max_src_x {
            return None;
        }
        u32::try_from(src_x).ok()
    }
}

/// Bracket drawn on the HUD layer for a contact that is 100% occluded by terrain
/// or faint/distant (`< MARK_FAR_PX` or in the last third of fog).
#[derive(Clone, Copy, Debug)]
struct FaintBracket {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: SpriteKind,
}

/// Retained Kitty state owned by [`CockpitPicture`].
#[derive(Debug)]
pub(crate) struct CockpitKittyState {
    last_viewport: Option<Viewport25D>,
    last_seated: Option<bool>,
    world: Option<PanoramicWorld>,
    world_image_id: u32,
    world_uploaded: bool,
    last_world_placement: Option<Placement>,
    uploaded_sheets: BTreeSet<u32>,
    active_unit_placements: BTreeMap<u32, Placement>,
    contact_pids: BTreeMap<String, u32>,
    next_contact_pid: u32,
    inset_uploaded: bool,
    last_inset_hash: u64,
    last_inset_placement: Option<Placement>,
    hud_uploaded: bool,
    last_hud_sig: u64,
    last_hud_placement: Option<Placement>,
    lock_uploaded: bool,
    last_lock_sig: u64,
    last_lock_dims: (usize, usize),
    last_lock_placement: Option<Placement>,
    glass_uploaded: bool,
    last_glass_sig: u64,
    last_glass_placement: Option<Placement>,
    fx_uploaded: bool,
    last_fx_placement: Option<Placement>,
}

impl Default for CockpitKittyState {
    fn default() -> Self {
        Self::new()
    }
}

impl CockpitKittyState {
    pub(crate) fn new() -> Self {
        Self {
            last_viewport: None,
            last_seated: None,
            world: None,
            world_image_id: IMAGE_ID_FP_WORLD_A,
            world_uploaded: false,
            last_world_placement: None,
            uploaded_sheets: BTreeSet::new(),
            active_unit_placements: BTreeMap::new(),
            contact_pids: BTreeMap::new(),
            next_contact_pid: PLACEMENT_FP_UNIT_BASE,
            inset_uploaded: false,
            last_inset_hash: 0,
            last_inset_placement: None,
            hud_uploaded: false,
            last_hud_sig: 0,
            last_hud_placement: None,
            lock_uploaded: false,
            last_lock_sig: 0,
            last_lock_dims: (0, 0),
            last_lock_placement: None,
            glass_uploaded: false,
            last_glass_sig: 0,
            last_glass_placement: None,
            fx_uploaded: false,
            last_fx_placement: None,
        }
    }

    pub(crate) fn invalidate_world(&mut self) {
        self.world = None;
        self.world_uploaded = false;
        self.last_world_placement = None;
    }

    pub(crate) fn invalidate_placements(&mut self) {
        self.last_world_placement = None;
        self.active_unit_placements.clear();
        self.last_inset_placement = None;
        self.last_hud_placement = None;
        self.last_lock_placement = None;
        self.last_glass_placement = None;
        self.last_fx_placement = None;
    }

    pub(crate) fn invalidate_image(&mut self, image_id: u32) {
        if image_id == self.world_image_id
            || image_id == IMAGE_ID_FP_WORLD_A
            || image_id == IMAGE_ID_FP_WORLD_B
        {
            self.world_uploaded = false;
            self.last_world_placement = None;
        } else if image_id == IMAGE_ID_FP_INSET {
            self.inset_uploaded = false;
            self.last_inset_placement = None;
        } else if image_id == IMAGE_ID_FP_HUD {
            self.hud_uploaded = false;
            self.last_hud_placement = None;
        } else if image_id == IMAGE_ID_FP_LOCK {
            self.lock_uploaded = false;
            self.last_lock_placement = None;
        } else if image_id == IMAGE_ID_FP_GLASS {
            self.glass_uploaded = false;
            self.last_glass_placement = None;
        } else if image_id == IMAGE_ID_FP_FX {
            self.fx_uploaded = false;
            self.last_fx_placement = None;
        } else {
            self.uploaded_sheets.remove(&image_id);
            self.active_unit_placements
                .retain(|_, p| p.image_id != image_id);
        }
    }

    pub(crate) fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for p in self.active_unit_placements.values() {
            kitty::delete_placement_bytes(&mut out, p.image_id, p.placement_id);
        }
        self.active_unit_placements.clear();

        if self.world_uploaded || self.last_world_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_WORLD_A);
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_WORLD_B);
            self.world_uploaded = false;
            self.last_world_placement = None;
        }
        if self.inset_uploaded || self.last_inset_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_INSET);
            self.inset_uploaded = false;
            self.last_inset_placement = None;
        }
        if self.hud_uploaded || self.last_hud_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_HUD);
            self.hud_uploaded = false;
            self.last_hud_placement = None;
        }
        if self.lock_uploaded || self.last_lock_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_LOCK);
            self.lock_uploaded = false;
            self.last_lock_placement = None;
        }
        if self.glass_uploaded || self.last_glass_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_GLASS);
            self.glass_uploaded = false;
            self.last_glass_placement = None;
        }
        if self.fx_uploaded || self.last_fx_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FP_FX);
            self.fx_uploaded = false;
            self.last_fx_placement = None;
        }
        for &sheet_id in &self.uploaded_sheets {
            kitty::delete_image_bytes(&mut out, sheet_id);
        }
        self.uploaded_sheets.clear();
        self.last_viewport = None;
        self.last_seated = None;
        out
    }
}

#[derive(Clone, Copy, Debug)]
struct CellPlacementBox {
    dc: u16,
    dr: u16,
    cols: u16,
    rows: u16,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
}

/// Convert a pixel box `(top_left_x, top_left_y, w_px, h_px)` drawn at padded
/// image origin `(origin_x, origin_y)` into a cell-aligned [`CellPlacementBox`]
/// inside `vp`.
fn cell_placement_box(
    vp: &Viewport25D,
    (top_left_x, top_left_y, w_px, h_px): (i32, i32, i32, i32),
    (origin_x, origin_y): (i32, i32),
) -> Option<CellPlacementBox> {
    if w_px <= 0 || h_px <= 0 {
        return None;
    }
    let vp_w = vp.width_px.max(1) as i32;
    let vp_h = vp.height_px.max(1) as i32;
    let vp_cols = i32::from(vp.cols.max(1));
    let vp_rows = i32::from(vp.rows.max(1));
    let col_px = |c: i32| (c * vp_w).div_euclid(vp_cols);
    let row_px = |r: i32| (r * vp_h).div_euclid(vp_rows);

    let c0 = (top_left_x * vp_cols).div_euclid(vp_w);
    let r0 = (top_left_y * vp_rows).div_euclid(vp_h);
    let c1 = ((top_left_x + w_px) * vp_cols + vp_w - 1).div_euclid(vp_w);
    let r1 = ((top_left_y + h_px) * vp_rows + vp_h - 1).div_euclid(vp_h);

    let vis_c0 = c0.max(0);
    let vis_r0 = r0.max(0);
    let vis_c1 = c1.min(vp_cols);
    let vis_r1 = r1.min(vp_rows);
    if vis_c1 <= vis_c0 || vis_r1 <= vis_r0 {
        return None;
    }

    let raw_src_x = origin_x + (col_px(vis_c0) - top_left_x);
    let raw_src_y = origin_y + (row_px(vis_r0) - top_left_y);
    let src_w = (col_px(vis_c1) - col_px(vis_c0)).max(1);
    let src_h = (row_px(vis_r1) - row_px(vis_r0)).max(1);

    Some(CellPlacementBox {
        dc: u16::try_from(vis_c0).unwrap_or(0),
        dr: u16::try_from(vis_r0).unwrap_or(0),
        cols: u16::try_from(vis_c1 - vis_c0).unwrap_or(1),
        rows: u16::try_from(vis_r1 - vis_r0).unwrap_or(1),
        src_x: u32::try_from(raw_src_x.max(0)).unwrap_or(0),
        src_y: u32::try_from(raw_src_y.max(0)).unwrap_or(0),
        src_w: u32::try_from(src_w).unwrap_or(1),
        src_h: u32::try_from(src_h).unwrap_or(1),
    })
}

/// Compute a fast 64-bit FNV-1a hash of a byte slice.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Convert a chroma-keyed [`Frame`] into an RGBA buffer where [`CHROMA_KEY`]
/// pixels are transparent (`alpha = 0`), [`PANEL`] pixels have `panel_alpha`,
/// and all other pixels have `fg_alpha`.
fn frame_to_rgba(frame: &Frame, panel_alpha: u8, fg_alpha: u8) -> Vec<u8> {
    let (w, h) = (frame.width(), frame.height());
    let mut rgba = vec![0u8; w * h * 4];
    for (dst, px) in rgba
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(frame.rgb().as_chunks::<3>().0)
    {
        if *px == CHROMA_KEY {
            *dst = [0, 0, 0, 0];
        } else if *px == PANEL {
            *dst = [px[0], px[1], px[2], panel_alpha];
        } else {
            *dst = [px[0], px[1], px[2], fg_alpha];
        }
    }
    rgba
}

/// Render the Flight HUD layer (heading tape, center reticle, off-screen lock
/// arrows, health pips, speed bar, and occluded/faint contact brackets) into
/// an RGBA buffer.
fn render_hud_rgba(
    w: usize,
    h: usize,
    hud_model: &Hud,
    faint_brackets: &[FaintBracket],
) -> (u64, Vec<u8>) {
    let mut hud_no_box = hud_model.clone();
    if matches!(
        hud_no_box.lock.as_ref().map(|l| &l.at),
        Some(LockAt::Box(..))
    ) {
        hud_no_box.lock = None;
    }

    let mut sig: u64 = ((w as u64) << 48)
        ^ ((h as u64) << 32)
        ^ ((hud_no_box.heading_mdeg / 1_000) as u64)
        ^ (((hud_no_box.hull_mdeg / 1_000) as u64) << 12)
        ^ ((hud_no_box.speed_kph as u64) << 24);
    if let Some(ord) = hud_no_box.ordered_mdeg {
        sig = sig.wrapping_mul(0x0100_0000_01b3) ^ ((ord / 1_000) as u64);
    }
    for m in &hud_no_box.marks {
        sig = sig.wrapping_mul(0x0100_0000_01b3)
            ^ ((m.bearing_mdeg / 1_000) as u64)
            ^ ((m.kind as u64) << 16);
    }
    for hm in &hud_no_box.health {
        sig = sig.wrapping_mul(0x0100_0000_01b3)
            ^ (hm.x as u64)
            ^ ((hm.top as u64) << 16)
            ^ ((hm.health.lit() as u64) << 32)
            ^ (u64::from(hm.party.unwrap_or(' ') as u32) << 40);
    }
    if let Some(lock) = &hud_no_box.lock {
        sig = sig.wrapping_mul(0x0100_0000_01b3) ^ fnv1a(lock.designator.as_bytes());
        sig ^= (lock.range_m as u64) << 16;
        sig ^= (lock.percent.unwrap_or(-1) as u64) << 32;
        match lock.at {
            LockAt::Beyond(x, s) => {
                sig ^= (x as u64) ^ ((s as i64 as u64) << 20) ^ 0xA5A5;
            }
            LockAt::Off(s) => {
                sig ^= ((s as i64 as u64) << 20) ^ 0x5A5A;
            }
            LockAt::Box(..) => {}
        }
    }
    for fb in faint_brackets {
        sig = sig.wrapping_mul(0x0100_0000_01b3)
            ^ (fb.x.round() as i64 as u64)
            ^ ((fb.y.round() as i64 as u64) << 16)
            ^ ((fb.w.round() as i64 as u64) << 32)
            ^ ((fb.kind as u64) << 48);
    }

    let mut frame = Frame::new(w, h);
    frame.clear(CHROMA_KEY);
    overlay::draw_hud(&mut frame, &hud_no_box);
    for fb in faint_brackets {
        let col = match fb.kind {
            SpriteKind::Hostile => [178, 54, 48],
            SpriteKind::Friend => [64, 124, 204],
            SpriteKind::Unknown => [132, 132, 126],
            SpriteKind::Wreck => [74, 58, 46],
        };
        raycast::mark(&mut frame, (fb.x, fb.y, fb.w, fb.h), col);
    }

    (sig, frame_to_rgba(&frame, 225, 255))
}

/// Render a padded RGBA target-lock bracket + label image (`IMAGE_ID_FP_LOCK`)
/// so moving the locked target across the viewport updates only `a=p`.
fn render_fp_lock_rgba(
    box_w: usize,
    box_h: usize,
    label: &str,
    s: usize,
) -> (usize, usize, Vec<u8>) {
    let s_i = s.max(1) as i64;
    let lw = (label.len() as i64 * 6 * s_i).max(16);
    let inner_w = (box_w as i64).max(lw + 4 * s_i) as usize;
    let inner_h = box_h + (12 * s.max(1));
    let img_w = inner_w + 2 * SHEET_PAD_X;
    let img_h = inner_h + 2 * SHEET_PAD_Y;

    let mut frame = Frame::new(img_w, img_h);
    frame.clear(CHROMA_KEY);

    let bx = (SHEET_PAD_X + (inner_w.saturating_sub(box_w)) / 2) as i64;
    let by = SHEET_PAD_Y as i64;
    let bw = box_w as i64;
    let bh = box_h as i64;
    overlay::outline_rect(&mut frame, bx, by, bw, bh, overlay::LOCK);
    overlay::outline_rect(&mut frame, bx + 1, by + 1, bw - 2, bh - 2, overlay::LOCK);

    let lx = (SHEET_PAD_X as i64 + (inner_w as i64 - lw) / 2).max(0);
    let ly = by + bh + 2 * s_i;
    overlay::fill_rect(&mut frame, lx - 2, ly - 1, lw + 4, 7 * s_i + 2, PANEL);
    overlay::text(&mut frame, lx, ly, label, overlay::LOCK, s.max(1));

    (img_w, img_h, frame_to_rgba(&frame, 215, 255))
}

/// Render the RGBA Canopy Glass overlay (`IMAGE_ID_FP_GLASS`), combining
/// translucent cyan glass reflections, corner frost vignette, structural
/// mullions, glacis cowling, status lamps, and ballistic glass cracks.
fn render_canopy_glass_rgba(w: usize, h: usize, glass: &CanopyGlass) -> (u64, Vec<u8>) {
    let mut sig: u64 = ((w as u64) << 48)
        ^ ((h as u64) << 32)
        ^ (((glass.look_mdeg / 1_000) as u64) & 0xFFFF)
        ^ (u64::from(glass.power_on) << 16)
        ^ (u64::from(glass.weapons_free) << 17)
        ^ (u64::from(glass.damage) << 20);
    if let Some(des) = &glass.lock_designator {
        sig = sig.wrapping_mul(0x0100_0000_01b3) ^ fnv1a(des.as_bytes());
    }

    let mut frame = Frame::new(w, h);
    frame.clear(CHROMA_KEY);
    overlay::draw_canopy_glass(&mut frame, glass);
    let mut rgba = frame_to_rgba(&frame, 235, 245);

    // Add true translucent cyan canopy reflection streaks and corner frost
    // vignette on transparent glass pixels (`alpha == 0`).
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    for y in 0..h {
        let ny = y as f32 / hf;
        for x in 0..w {
            let off = (y * w + x) * 4;
            let Some(px) = rgba.get_mut(off..off + 4) else {
                continue;
            };
            if px.get(3).copied() != Some(0) {
                continue;
            }
            let nx = x as f32 / wf;
            let dx = (nx - 0.5).abs() * 2.0;
            let dy = (ny - 0.5).abs() * 2.0;
            // Subtle diagonal canopy reflection bands
            let diag1 = (nx * 1.15 + ny * 0.85) - (glass.look_mdeg as f32 / 360_000.0) * 0.25;
            let band = (diag1 > 0.34 && diag1 < 0.39) || (diag1 > 1.22 && diag1 < 1.26);
            if band && ny > 0.06 && ny < 0.88 {
                px.copy_from_slice(&[135, 215, 255, 20]);
            } else if dx * dx + dy * dy > 1.38 {
                // Corner frost vignette
                px.copy_from_slice(&[14, 24, 38, 64]);
            }
        }
    }

    (sig, rgba)
}

impl CockpitPicture {
    /// Force all retained First-Person Kitty placements to be re-emitted on the
    /// next tick.
    pub fn invalidate_placements(&mut self) {
        self.kitty.invalidate_placements();
    }

    /// Mark an image ID as evicted after a `q=1` terminal error reply so the
    /// next tick re-uploads and re-places it automatically.
    pub fn invalidate_image(&mut self, image_id: u32) {
        self.kitty.invalidate_image(image_id);
    }

    /// Emit Kitty delete commands for every retained image and placement owned
    /// by the First-Person cockpit compositor.
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        self.kitty.cleanup_kitty_bytes()
    }

    /// Synchronise feeds and render one tick of the retained First-Person Kitty
    /// compositor (`CSI ? 2026 h` … `CSI ? 2026 l`), returning `None` if no
    /// cockpit frame is held in `feeds`. When nothing changed since the
    /// previous tick, the returned byte vector is empty so the caller skips
    /// sending a frame.
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<CellSize>,
        step_down: bool,
        glass_on: bool,
    ) -> Option<(Viewport25D, Vec<u8>)> {
        let cockpit = feeds.feed(COCKPIT_FEED)?;
        let map = feeds.feed(ARENA_MAP_FEED);
        self.ensure_map(map);
        let look_mdeg =
            (look.offset_mdeg - look.hull_offset_mdeg + 180_000).rem_euclid(360_000) - 180_000;
        Some(self.render_kitty_bytes(
            cockpit, map, look, area, cell, step_down, glass_on, look_mdeg,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_kitty_bytes(
        &mut self,
        cockpit: &Value,
        map: Option<&Value>,
        look: &Look,
        (area_col, area_row, area_cols, area_rows): (u16, u16, u16, u16),
        cell: Option<CellSize>,
        step_down: bool,
        glass_on: bool,
        look_mdeg: i64,
    ) -> (Viewport25D, Vec<u8>) {
        let vp = viewport_fp(area_cols, area_rows, cell, step_down);
        let mut ops = Vec::new();

        if self.kitty.last_viewport != Some(vp) {
            ops.extend_from_slice(&self.kitty.cleanup_kitty_bytes());
            self.kitty.world = None;
            self.kitty.last_viewport = Some(vp);
        }

        let seated = cockpit.get("seated").and_then(Value::as_bool) != Some(false);
        if !seated {
            if self.kitty.last_seated != Some(false) {
                ops.extend_from_slice(&self.kitty.cleanup_kitty_bytes());
                self.kitty.last_viewport = Some(vp);
                self.kitty.last_seated = Some(false);
            }
            if ops.is_empty() {
                return (vp, Vec::new());
            }
            let mut out = Vec::with_capacity(ops.len() + 20);
            out.extend_from_slice(kitty::SYNC_BEGIN);
            out.extend_from_slice(b"\x1b7");
            out.extend_from_slice(&ops);
            out.extend_from_slice(b"\x1b8");
            out.extend_from_slice(kitty::SYNC_END);
            return (vp, out);
        }
        self.kitty.last_seated = Some(true);

        let any_contact_moving = cockpit
            .get("contacts")
            .and_then(Value::as_array)
            .is_some_and(|cs| {
                cs.iter()
                    .any(|c| c.get("moving").and_then(Value::as_bool) == Some(true))
            });
        let step = self.draws / ANIMATION_DRAWS;
        if any_contact_moving {
            self.draws = self.draws.wrapping_add(1);
        }

        let cam = camera(cockpit, look);
        let base_cam = Camera {
            x_cm: cockpit.get("x_cm").and_then(Value::as_i64).unwrap_or(0),
            y_cm: cockpit.get("y_cm").and_then(Value::as_i64).unwrap_or(0),
            heading_mdeg: cam.heading_mdeg,
        };
        self.contact_tracker.step_frame(
            cockpit,
            base_cam.x_cm as f32 * 0.01,
            base_cam.y_cm as f32 * 0.01,
        );
        fx::sync_combat_fx(&mut self.last_fx_t_ms, &mut self.active_fx, cockpit);
        let arena = &self.arena;
        let terrain = self.terrain.as_ref();
        let eye = eye_at_cam_cm(cockpit, arena, terrain, look, &cam);
        let sensor_eye = sensor_height_cm(cockpit);

        let base_col = area_col.saturating_add(vp.col_offset);
        let base_row = area_row.saturating_add(vp.row_offset);

        // 1. Layer 1: Overscanned 2× Panoramic World Canvas (`z = -100`).
        let src_x_opt = self
            .kitty
            .world
            .as_ref()
            .filter(|w| w.vp_w == vp.width_px && w.vp_h == vp.height_px)
            .and_then(|w| w.src_x_for(&cam, eye, sensor_eye, terrain.is_some()));

        let src_x = if let Some(sx) = src_x_opt {
            if !self.kitty.world_uploaded
                && let Some(world) = &self.kitty.world
            {
                let png = png::encode(&world.ground_frame);
                kitty::transmit_bytes(&mut ops, &png, self.kitty.world_image_id);
                self.kitty.world_uploaded = true;
                self.kitty.last_world_placement = None;
            }
            sx
        } else {
            let new_world = PanoramicWorld::bake(
                arena,
                terrain,
                cam,
                eye,
                sensor_eye,
                vp.width_px,
                vp.height_px,
            );
            let old_id = self.kitty.world_image_id;
            let new_id = if self.kitty.world_uploaded && old_id == IMAGE_ID_FP_WORLD_A {
                IMAGE_ID_FP_WORLD_B
            } else {
                IMAGE_ID_FP_WORLD_A
            };
            let png = png::encode(&new_world.ground_frame);
            kitty::transmit_bytes(&mut ops, &png, new_id);
            if self.kitty.world_uploaded && old_id != new_id {
                kitty::delete_image_bytes(&mut ops, old_id);
            }
            self.kitty.world_image_id = new_id;
            self.kitty.world_uploaded = true;
            self.kitty.last_world_placement = None;
            let center_sx =
                u32::try_from((new_world.canvas_w.saturating_sub(vp.width_px)) / 2).unwrap_or(0);
            self.kitty.world = Some(new_world);
            center_sx
        };

        let world_p = Placement {
            image_id: self.kitty.world_image_id,
            placement_id: PLACEMENT_FP_WORLD,
            col: base_col,
            row: base_row,
            cols: vp.cols,
            rows: vp.rows,
            cell_x: 0,
            cell_y: 0,
            src_x,
            src_y: 0,
            src_w: u32::try_from(vp.width_px).unwrap_or(640),
            src_h: u32::try_from(vp.height_px).unwrap_or(360),
            z: Z_FP_WORLD,
        };
        if self.kitty.last_world_placement != Some(world_p) {
            kitty::place_bytes(&mut ops, &world_p);
            self.kitty.last_world_placement = Some(world_p);
        }

        // 2. Layer 2: Retained First-Person Unit Billboard Sprites (`z = -85..-10`).
        let mut next_placements = BTreeMap::new();
        let mut faint_brackets = Vec::new();
        let mut vis_targets = Vec::new();
        let lock_id = cockpit.get("lock").and_then(Value::as_str);
        let lock_desig = cockpit.get("lock_designator").and_then(Value::as_str);
        let contacts = cockpit
            .get("contacts")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        if let Some(world) = &self.kitty.world {
            for (ci, c) in contacts.iter().enumerate() {
                let cid = c.get("id").and_then(Value::as_str);
                let cdes = c.get("designator").and_then(Value::as_str);
                let id_str = cid
                    .or(cdes)
                    .map_or_else(|| format!("idx:{ci}"), str::to_owned);
                let pid = if let Some(&existing) = self.kitty.contact_pids.get(&id_str) {
                    existing
                } else {
                    let assigned = self.kitty.next_contact_pid;
                    self.kitty.next_contact_pid = self.kitty.next_contact_pid.saturating_add(1);
                    self.kitty.contact_pids.insert(id_str, assigned);
                    assigned
                };

                let origin_base_cam = Camera {
                    x_cm: base_cam.x_cm,
                    y_cm: base_cam.y_cm,
                    heading_mdeg: world.origin_cam.heading_mdeg,
                };
                let (b_deg, r_m) = self.contact_tracker.contact_bearing_range(c, ci);
                let sprite = contact_sprite_at_bearing_range(c, &origin_base_cam, step, b_deg, r_m);
                let at_opt: Option<SpriteAt> = match terrain {
                    Some(t) => heightfield::project_panoramic(
                        t,
                        &world.origin_cam,
                        world.origin_eye_cm,
                        world.canvas_w,
                        vp.width_px,
                        vp.height_px,
                        &sprite,
                    ),
                    None => raycast::project_panoramic_from(
                        arena,
                        &world.origin_cam,
                        world.origin_sensor_eye_cm,
                        world.canvas_w,
                        vp.width_px,
                        vp.height_px,
                        &sprite,
                    ),
                };
                let Some(at) = at_opt else {
                    continue;
                };

                let vp_screenx = at.screenx - (src_x as f32);
                if vp_screenx + at.width * 0.5 < 0.0
                    || vp_screenx - at.width * 0.5 >= vp.width_px as f32
                {
                    continue;
                }
                let is_locked = (lock_id.is_some() && cid == lock_id)
                    || (lock_desig.is_some() && cdes == lock_desig);
                vis_targets.push((
                    cid.map(str::to_owned),
                    cdes.map(str::to_owned),
                    is_locked,
                    vp_screenx.round() as i32,
                    (at.bottom - at.height * 0.5).round() as i32,
                    (at.height * 0.35).round().clamp(8.0, 26.0) as i32,
                ));

                // Check hull-down depth occlusion against `world.depth`.
                let cx0 = ((at.screenx - at.width * 0.35).floor().max(0.0) as usize)
                    .min(world.canvas_w.saturating_sub(1));
                let cx1 = ((at.screenx + at.width * 0.35).ceil().max(0.0) as usize)
                    .min(world.canvas_w.saturating_sub(1));
                let top_f = at.bottom - at.height;
                let y0 = (top_f.floor().max(0.0) as usize).min(vp.height_px.saturating_sub(1));
                let y1 = (at.bottom.ceil().max(0.0) as usize).min(vp.height_px);
                let mut vis_rows = 0usize;
                let mut max_vis_y = y0;
                for y in y0..y1 {
                    let row_off = y * world.canvas_w;
                    let row_vis = (cx0..=cx1).any(|x| {
                        at.depth
                            < world
                                .depth
                                .get(row_off + x)
                                .copied()
                                .unwrap_or(f32::INFINITY)
                    });
                    if row_vis {
                        vis_rows += 1;
                        max_vis_y = y + 1;
                    }
                }

                let faint = at.height < MARK_FAR_PX || fog_step(at.fog_depth) >= FOG_STEPS * 2 / 3;
                if vis_rows == 0 || faint {
                    let least = if vis_rows == 0 {
                        MARK_MIN_PX
                    } else {
                        MARK_FAR_PX
                    };
                    let bw = at.width.max(least);
                    let bh = at.height.max(least);
                    let cy = at.bottom - at.height * 0.5;
                    faint_brackets.push(FaintBracket {
                        x: vp_screenx - bw * 0.5,
                        y: cy - bh * 0.5,
                        w: bw,
                        h: bh,
                        kind: sprite.kind,
                    });
                    if vis_rows == 0 || at.height < 2.0 {
                        continue;
                    }
                }

                let view = bake::View::for_depression(at.depression_deg);
                let look_idx = look_index(c.get("unit_type").and_then(Value::as_str));
                let Some(art) = bake::view_art(look_idx, view) else {
                    continue;
                };
                let Some((chain, height_cm)) = bake::art_chain_for_view(art, view) else {
                    continue;
                };
                let px_per_m = at.height / (height_cm.max(1) as f32 / 100.0);
                let Some((size_idx_usize, sized)) = chain.pick(px_per_m) else {
                    continue;
                };
                let size_idx = u8::try_from(size_idx_usize).unwrap_or(0);
                let paint = FpPaintKey::from_kind(sprite.kind);
                let key = FpSheetKey {
                    view,
                    art,
                    paint,
                    size: size_idx,
                };
                let img_id = key.image_id();
                if !self.kitty.uploaded_sheets.contains(&img_id) {
                    let (sw, sh, rgba) = render_fp_sheet_rgba(key);
                    let png = png::encode_rgba(sw, sh, &rgba);
                    kitty::transmit_bytes(&mut ops, &png, img_id);
                    self.kitty.uploaded_sheets.insert(img_id);
                }

                let facing = sprite.pose.map_or(Facing::Front, |p| p.facing);
                let moving = sprite.pose.is_some_and(|p| p.moving);
                let pose_col = bake::pose_index(moving, step, sprite.kind == SpriteKind::Wreck);
                let Some(f) = sized.frame(facing, pose_col) else {
                    continue;
                };
                let n = i32::from(sized.size);
                let sx_i = vp_screenx.round() as i32;
                let sy_i = at.bottom.round() as i32;
                let top_left_x = sx_i - i32::from(f.anchor.0);
                let top_left_y = sy_i - i32::from(f.anchor.1);

                // Clip bottom at `max_vis_y` when partially hull-down behind a ridge.
                let clip_bottom = (max_vis_y as i32 + 2).min(top_left_y + n);
                let vis_h_px = (clip_bottom - top_left_y).clamp(1, n);

                let stride_x = n + SHEET_PAD_X as i32;
                let stride_y = n + SHEET_PAD_Y as i32;
                let origin_x = (SHEET_PAD_X as i32) + (pose_col as i32) * stride_x;
                let origin_y = (SHEET_PAD_Y as i32) + (facing as i32) * stride_y;

                let Some(cb) = cell_placement_box(
                    &vp,
                    (top_left_x, top_left_y, n, vis_h_px),
                    (origin_x, origin_y),
                ) else {
                    continue;
                };

                let dist_clamped = (at.depth.round() as i32).clamp(1, 120);
                let z = if sprite.kind == SpriteKind::Wreck {
                    Z_FP_WRECK_BASE - (dist_clamped / 6)
                } else {
                    Z_FP_UNIT_BASE + (48 - (dist_clamped * 48 / 120))
                };

                let unit_p = Placement {
                    image_id: img_id,
                    placement_id: pid,
                    col: base_col.saturating_add(cb.dc),
                    row: base_row.saturating_add(cb.dr),
                    cols: cb.cols,
                    rows: cb.rows,
                    cell_x: 0,
                    cell_y: 0,
                    src_x: cb.src_x,
                    src_y: cb.src_y,
                    src_w: cb.src_w,
                    src_h: cb.src_h,
                    z,
                };
                if self.kitty.active_unit_placements.get(&pid) != Some(&unit_p) {
                    kitty::place_bytes(&mut ops, &unit_p);
                }
                next_placements.insert(pid, unit_p);
            }
        }

        for (&old_pid, old_p) in &self.kitty.active_unit_placements {
            if !next_placements.contains_key(&old_pid) {
                kitty::delete_placement_bytes(&mut ops, old_p.image_id, old_pid);
            }
        }
        self.kitty.active_unit_placements = next_placements;

        // 3. Layer 3: Retained Minimap Inset (`IMAGE_ID_FP_INSET = 142`, `z = -5`).
        let dummy_frame = Frame::new(vp.width_px, vp.height_px);
        if let Some((ix, iy, side)) = overlay::inset_rect(&dummy_frame) {
            let vp_w = vp.width_px.max(1);
            let vp_h = vp.height_px.max(1);
            let vp_cols = usize::from(vp.cols.max(1));
            let vp_rows = usize::from(vp.rows.max(1));
            let c0 = ((ix.max(0) as usize) * vp_cols) / vp_w;
            let r0 = ((iy.max(0) as usize) * vp_rows) / vp_h;
            let c1 = (((ix.max(0) as usize + side) * vp_cols).div_ceil(vp_w)).min(vp_cols);
            let r1 = (((iy.max(0) as usize + side) * vp_rows).div_ceil(vp_h)).min(vp_rows);
            let icols = c1.saturating_sub(c0).max(1);
            let irows = r1.saturating_sub(r0).max(1);
            let px_x0 = (c0 * vp_w) / vp_cols;
            let px_y0 = (r0 * vp_h) / vp_rows;
            let px_x1 = (c1 * vp_w) / vp_cols;
            let px_y1 = (r1 * vp_h) / vp_rows;
            let iw = px_x1.saturating_sub(px_x0).max(side);
            let ih = px_y1.saturating_sub(px_y0).max(side);

            let mut inset_frame = Frame::new(iw, ih);
            inset_frame.clear(CHROMA_KEY);
            let local_rect = ((ix - px_x0 as i64).max(0), (iy - px_y0 as i64).max(0), side);
            self.inset
                .draw_inset(&mut inset_frame, cockpit, map, look, local_rect, INSET_UP);
            let inset_hash = inset_frame.hash();
            if !self.kitty.inset_uploaded || self.kitty.last_inset_hash != inset_hash {
                let rgba = frame_to_rgba(&inset_frame, 255, 255);
                let png = png::encode_rgba(iw, ih, &rgba);
                kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FP_INSET);
                self.kitty.inset_uploaded = true;
                self.kitty.last_inset_hash = inset_hash;
            }
            let inset_p = Placement {
                image_id: IMAGE_ID_FP_INSET,
                placement_id: PLACEMENT_FP_INSET,
                col: base_col.saturating_add(u16::try_from(c0).unwrap_or(0)),
                row: base_row.saturating_add(u16::try_from(r0).unwrap_or(0)),
                cols: u16::try_from(icols).unwrap_or(1),
                rows: u16::try_from(irows).unwrap_or(1),
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(iw).unwrap_or(64),
                src_h: u32::try_from(ih).unwrap_or(64),
                z: Z_FP_INSET,
            };
            if self.kitty.last_inset_placement != Some(inset_p) {
                kitty::place_bytes(&mut ops, &inset_p);
                self.kitty.last_inset_placement = Some(inset_p);
            }
        } else if self.kitty.last_inset_placement.take().is_some() {
            kitty::delete_placement_bytes(&mut ops, IMAGE_ID_FP_INSET, PLACEMENT_FP_INSET);
        }

        // 4. Layer 4: Retained Flight HUD Overlay (`IMAGE_ID_FP_HUD = 143`, `z = -3`).
        let hud_model = hud(&dummy_frame, arena, terrain, eye, cockpit, look, &cam);
        let (hud_sig, hud_rgba) =
            render_hud_rgba(vp.width_px, vp.height_px, &hud_model, &faint_brackets);
        if !self.kitty.hud_uploaded || self.kitty.last_hud_sig != hud_sig {
            let png = png::encode_rgba(vp.width_px, vp.height_px, &hud_rgba);
            kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FP_HUD);
            self.kitty.hud_uploaded = true;
            self.kitty.last_hud_sig = hud_sig;
        }
        let hud_p = Placement {
            image_id: IMAGE_ID_FP_HUD,
            placement_id: PLACEMENT_FP_HUD,
            col: base_col,
            row: base_row,
            cols: vp.cols,
            rows: vp.rows,
            cell_x: 0,
            cell_y: 0,
            src_x: 0,
            src_y: 0,
            src_w: u32::try_from(vp.width_px).unwrap_or(640),
            src_h: u32::try_from(vp.height_px).unwrap_or(360),
            z: Z_FP_HUD,
        };
        if self.kitty.last_hud_placement != Some(hud_p) {
            kitty::place_bytes(&mut ops, &hud_p);
            self.kitty.last_hud_placement = Some(hud_p);
        }

        // 5. Layer 5: Movable Target-Lock Bracket (`IMAGE_ID_FP_LOCK = 144`, `z = -2`).
        if let Some(lock) = &hud_model.lock
            && let LockAt::Box(x, y, bw, bh) = lock.at
        {
            let s = overlay::scale_for(&dummy_frame);
            let s_i = s as i64;
            let m = 2 * s_i;
            let q_bw = (((bw + 2 * m).max(8 * s_i) as usize + 7) & !7).clamp(16, 320);
            let q_bh = (((bh + 2 * m).max(8 * s_i) as usize + 7) & !7).clamp(16, 320);
            let mut label = format!("{} {}M", lock.designator.to_ascii_uppercase(), lock.range_m);
            if let Some(pct) = lock.percent {
                label.push_str(&format!(" {pct}%"));
            }
            let lock_sig = ((q_bw as u64) << 48) ^ ((q_bh as u64) << 32) ^ fnv1a(label.as_bytes());
            if !self.kitty.lock_uploaded || self.kitty.last_lock_sig != lock_sig {
                let (lw_px, lh_px, lock_rgba) = render_fp_lock_rgba(q_bw, q_bh, &label, s);
                let png = png::encode_rgba(lw_px, lh_px, &lock_rgba);
                kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FP_LOCK);
                self.kitty.lock_uploaded = true;
                self.kitty.last_lock_sig = lock_sig;
                self.kitty.last_lock_dims = (
                    lw_px.saturating_sub(2 * SHEET_PAD_X),
                    lh_px.saturating_sub(2 * SHEET_PAD_Y),
                );
            }
            let (inner_w, inner_h) = self.kitty.last_lock_dims;
            let cx = x + bw / 2;
            let top_left_x = (cx - (inner_w as i64) / 2) as i32;
            let top_left_y = (y - m) as i32;
            if let Some(cb) = cell_placement_box(
                &vp,
                (top_left_x, top_left_y, inner_w as i32, inner_h as i32),
                (SHEET_PAD_X as i32, SHEET_PAD_Y as i32),
            ) {
                let lock_p = Placement {
                    image_id: IMAGE_ID_FP_LOCK,
                    placement_id: PLACEMENT_FP_LOCK,
                    col: base_col.saturating_add(cb.dc),
                    row: base_row.saturating_add(cb.dr),
                    cols: cb.cols,
                    rows: cb.rows,
                    cell_x: 0,
                    cell_y: 0,
                    src_x: cb.src_x,
                    src_y: cb.src_y,
                    src_w: cb.src_w,
                    src_h: cb.src_h,
                    z: Z_FP_LOCK,
                };
                if self.kitty.last_lock_placement != Some(lock_p) {
                    kitty::place_bytes(&mut ops, &lock_p);
                    self.kitty.last_lock_placement = Some(lock_p);
                }
            } else if self.kitty.last_lock_placement.take().is_some() {
                kitty::delete_placement_bytes(&mut ops, IMAGE_ID_FP_LOCK, PLACEMENT_FP_LOCK);
            }
        } else if self.kitty.last_lock_placement.take().is_some() {
            kitty::delete_placement_bytes(&mut ops, IMAGE_ID_FP_LOCK, PLACEMENT_FP_LOCK);
        }

        // 6. Layer 6: Retained RGBA Canopy Glass Overlay (`IMAGE_ID_FP_GLASS = 145`, `z = -1`).
        if glass_on {
            let glass = canopy_glass_from(cockpit, look_mdeg);
            let (glass_sig, glass_rgba) =
                render_canopy_glass_rgba(vp.width_px, vp.height_px, &glass);
            if !self.kitty.glass_uploaded || self.kitty.last_glass_sig != glass_sig {
                let png = png::encode_rgba(vp.width_px, vp.height_px, &glass_rgba);
                kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FP_GLASS);
                self.kitty.glass_uploaded = true;
                self.kitty.last_glass_sig = glass_sig;
            }
            let glass_p = Placement {
                image_id: IMAGE_ID_FP_GLASS,
                placement_id: PLACEMENT_FP_GLASS,
                col: base_col,
                row: base_row,
                cols: vp.cols,
                rows: vp.rows,
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(vp.width_px).unwrap_or(640),
                src_h: u32::try_from(vp.height_px).unwrap_or(360),
                z: Z_FP_GLASS,
            };
            if self.kitty.last_glass_placement != Some(glass_p) {
                kitty::place_bytes(&mut ops, &glass_p);
                self.kitty.last_glass_placement = Some(glass_p);
            }
        } else if self.kitty.last_glass_placement.take().is_some() {
            kitty::delete_placement_bytes(&mut ops, IMAGE_ID_FP_GLASS, PLACEMENT_FP_GLASS);
        }

        // 7. Layer 7: Transient FORGE3D Combat FX Overlay (`IMAGE_ID_FP_FX = 146`, `z = -4`).
        if !self.active_fx.is_empty() {
            let gun_x = (vp.width_px / 2) as i32;
            let gun_y = ((vp.height_px * 6) / 7) as i32;
            let struck_x = (vp.width_px / 2) as i32;
            let struck_y = (vp.height_px / 2) as i32;

            let find_target = |tk: Option<&str>| -> Option<(i32, i32, i32)> {
                if let Some(k) = tk
                    && let Some(t) = vis_targets
                        .iter()
                        .find(|t| t.0.as_deref() == Some(k) || t.1.as_deref() == Some(k))
                {
                    return Some((t.3, t.4, t.5));
                }
                vis_targets
                    .iter()
                    .find(|t| t.2)
                    .or_else(|| vis_targets.first())
                    .map(|t| (t.3, t.4, t.5))
            };

            let mut min_x = (gun_x - 36).min(struck_x - 36).max(0);
            let mut max_x = (gun_x + 36).max(struck_x + 36).min(vp.width_px as i32 - 1);
            let mut min_y = (gun_y - 36).min(struck_y - 36).max(0);
            let mut max_y = (gun_y + 36).max(struck_y + 36).min(vp.height_px as i32 - 1);

            for fx_item in &self.active_fx {
                if let CombatFxKind::Fire { target_key, .. } = &fx_item.kind
                    && let Some((tx, ty, tr)) = find_target(target_key.as_deref())
                {
                    let pad = tr + 24;
                    min_x = min_x.min(tx - pad).max(0);
                    max_x = max_x.max(tx + pad).min(vp.width_px as i32 - 1);
                    min_y = min_y.min(ty - pad).max(0);
                    max_y = max_y.max(ty + pad).min(vp.height_px as i32 - 1);
                }
            }

            let cw = i32::from(vp.cell_w.max(1));
            let ch = i32::from(vp.cell_h.max(1));
            let c0 = (min_x / cw).clamp(0, i32::from(vp.cols).saturating_sub(1));
            let r0 = (min_y / ch).clamp(0, i32::from(vp.rows).saturating_sub(1));
            let c1 = ((max_x + cw) / cw).clamp(c0 + 1, i32::from(vp.cols));
            let r1 = ((max_y + ch) / ch).clamp(r0 + 1, i32::from(vp.rows));
            let cols = u16::try_from(c1 - c0).unwrap_or(1);
            let rows = u16::try_from(r1 - r0).unwrap_or(1);
            let ox = c0 * cw;
            let oy = r0 * ch;
            let fx_w = usize::from(cols) * usize::from(vp.cell_w.max(1));
            let fx_h = usize::from(rows) * usize::from(vp.cell_h.max(1));
            let mut fx_rgba = vec![0u8; fx_w * fx_h * 4];

            {
                let mut put = |x: i32, y: i32, col: [u8; 4]| {
                    let lx = x - ox;
                    let ly = y - oy;
                    if lx >= 0 && lx < fx_w as i32 && ly >= 0 && ly < fx_h as i32 {
                        let off = (ly as usize * fx_w + lx as usize) * 4;
                        if let Some(dst) = fx_rgba.get_mut(off..off + 4) {
                            dst.copy_from_slice(&col);
                        }
                    }
                };
                for fx_item in &self.active_fx {
                    let target_xy = match &fx_item.kind {
                        CombatFxKind::Fire { target_key, .. } => find_target(target_key.as_deref()),
                        CombatFxKind::Struck { .. } => None,
                    };
                    fx::paint_single_fx(
                        fx_item,
                        (gun_x, gun_y),
                        target_xy,
                        (struck_x, struck_y),
                        &mut put,
                    );
                }
            }

            let png = png::encode_rgba(fx_w, fx_h, &fx_rgba);
            kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FP_FX);
            let fx_p = Placement {
                image_id: IMAGE_ID_FP_FX,
                placement_id: PLACEMENT_FP_FX,
                col: base_col.saturating_add(u16::try_from(c0).unwrap_or(0)),
                row: base_row.saturating_add(u16::try_from(r0).unwrap_or(0)),
                cols,
                rows,
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(fx_w).unwrap_or(64),
                src_h: u32::try_from(fx_h).unwrap_or(64),
                z: Z_FP_FX,
            };
            kitty::place_bytes(&mut ops, &fx_p);
            self.kitty.fx_uploaded = true;
            self.kitty.last_fx_placement = Some(fx_p);
            fx::step_combat_fx(&mut self.active_fx);
        } else if self.kitty.last_fx_placement.take().is_some() || self.kitty.fx_uploaded {
            kitty::delete_image_bytes(&mut ops, IMAGE_ID_FP_FX);
            self.kitty.fx_uploaded = false;
        }

        if ops.is_empty() {
            return (vp, Vec::new());
        }

        let mut out = Vec::with_capacity(ops.len() + 20);
        out.extend_from_slice(kitty::SYNC_BEGIN);
        out.extend_from_slice(b"\x1b7");
        out.extend_from_slice(&ops);
        out.extend_from_slice(b"\x1b8");
        out.extend_from_slice(kitty::SYNC_END);
        (vp, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn viewport_fp_scales_up_to_1280_with_cell_size_and_falls_back_to_640() {
        let cell = CellSize {
            width: 14,
            height: 28,
        };
        let vp_1280 = viewport_fp(128, 28, Some(cell), false);
        assert_eq!(vp_1280.width_px, 1280);
        assert_eq!(vp_1280.height_px, 560);

        let vp_step = viewport_fp(128, 28, Some(cell), true);
        assert_eq!(vp_step.width_px, 640);
        assert_eq!(vp_step.height_px, 280);

        // Fallback when terminal cell pixel dimensions are not reported (`None`):
        let vp_fallback = viewport_fp(120, 26, None, false);
        assert_eq!(vp_fallback.width_px, 640);
        assert_eq!(vp_fallback.height_px, 276);
        assert_eq!((vp_fallback.cols, vp_fallback.rows), (120, 26));

        let vp_fallback_step = viewport_fp(120, 26, None, true);
        assert_eq!(vp_fallback_step.width_px, 320);
        assert_eq!(vp_fallback_step.height_px, 138);
    }

    #[test]
    fn draw_kitty_pans_panoramic_world_and_moves_retained_sprites_via_ap() {
        let cockpit: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture");
        let map: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/arena_map.json"
        ))
        .expect("map fixture");
        let feeds: HashMap<String, Value> = [
            (crate::cockpit::COCKPIT_FEED.to_string(), cockpit),
            (crate::cockpit::ARENA_MAP_FEED.to_string(), map),
        ]
        .into_iter()
        .collect();

        let mut pic = CockpitPicture::new();
        let area = (1, 2, 120, 26);
        let look0 = Look::default();

        // 1. Initial frame uploads panoramic world (`i=140`), sprite sheets, inset, HUD, lock, and glass.
        let (vp0, wire0) = pic
            .draw_kitty(&feeds, &look0, area, None, false, true)
            .expect("first-person kitty frame");
        assert_eq!((vp0.width_px, vp0.height_px), (640, 276));
        let s0 = String::from_utf8_lossy(&wire0);
        assert!(s0.contains("a=t,f=100,t=d,i=140"));
        assert!(s0.contains("a=p,i=140,p=40,x=320,y=0,w=640,h=276"));
        assert!(s0.contains("a=p,i=142,p=42"));
        assert!(s0.contains("a=p,i=143,p=43"));
        assert!(s0.contains("a=p,i=144,p=44"));
        assert!(s0.contains("a=p,i=145,p=45"));

        // 2. Identical second tick emits zero bytes.
        let (_, wire_idle) = pic
            .draw_kitty(&feeds, &look0, area, None, false, true)
            .expect("idle tick");
        assert!(wire_idle.is_empty());

        // 3. Yawing the camera within the ±30° panoramic overscan window shifts `x=` on `p=40`
        //    and moves the contact sprite (`p=101`) and lock bracket (`p=44`) via `a=p` without
        //    re-uploading the world (`i=140`/`141`), sprite sheets, or lock bracket (`i=144`).
        let look_yaw = Look {
            offset_mdeg: 10_000,
            ..Look::default()
        };
        let (_, wire_yaw) = pic
            .draw_kitty(&feeds, &look_yaw, area, None, false, true)
            .expect("yawed tick");
        let s_yaw = String::from_utf8_lossy(&wire_yaw);
        assert!(s_yaw.contains("a=p,i=140,p=40,x="));
        assert!(!s_yaw.contains("a=p,i=140,p=40,x=320,"));
        assert!(!s_yaw.contains("a=t,f=100,t=d,i=140"));
        assert!(!s_yaw.contains("a=t,f=100,t=d,i=141"));
        assert!(!s_yaw.contains("a=t,f=100,t=d,i=144"));
        assert!(s_yaw.contains("a=p,i=144,p=44"));

        // 4. Turning beyond the overscan margin bakes a new panoramic canvas and ping-pongs to `i=141`.
        let look_far = Look {
            offset_mdeg: 55_000,
            ..Look::default()
        };
        let (_, wire_far) = pic
            .draw_kitty(&feeds, &look_far, area, None, false, true)
            .expect("far yaw tick");
        let s_far = String::from_utf8_lossy(&wire_far);
        assert!(s_far.contains("a=t,f=100,t=d,i=141"));
        assert!(s_far.contains("a=d,d=I,i=140"));
        assert!(s_far.contains("a=p,i=141,p=40,x=320,"));

        // 5. Toggling canopy glass off deletes placement 45 without touching the world.
        let (_, wire_noglass) = pic
            .draw_kitty(&feeds, &look_far, area, None, false, false)
            .expect("glass off tick");
        let s_noglass = String::from_utf8_lossy(&wire_noglass);
        assert!(s_noglass.contains("a=d,d=i,i=145,p=45"));
    }
}
