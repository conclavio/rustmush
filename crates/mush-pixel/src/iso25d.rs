//! **The fixed-angle 2.5D tactical view and retained Kitty compositor**.
//!
//! Cycling the camera (`v` in the pixel cockpit) steps across three views:
//! first-person (`cockpit`), overhead (`overhead`), and this oblique 2.5D view
//! (`iso25d`).
//!
//! In SSH terminals with Kitty graphics, the 2.5D view drives the Kitty
//! protocol as a **retained hardware sprite and layer compositor**:
//!
//! - **Upload once (`a=t,i=<id>,q=1`)**: unit sprite sheets (8 facings × 6
//!   poses: still, four moving frames, wreck) baked from the 3D models at six
//!   geometric mipmap scales (`96, 86, 78, 70, 64, 56` px, ~10 % steps) are
//!   uploaded lazily once per `(UnitModel, PaintKey, mip)`. Any number of units
//!   sharing a sheet are placed simultaneously under distinct `placement_id`s
//!   (`p=100`, `p=101`, …).
//! - **Place and move (`a=p,i=<id>,p=<pid>,x=…,y=…,w=…,h=…,X=0,Y=0,z=…,C=1,q=1`)**:
//!   each tick updates every visible unit's position, facing, animation frame,
//!   and depth `z` in a single short escape sequence using padded source-rect
//!   selection (`x,y,w,h`).
//! - **Pure-translation terrain scrolling (`z = -100`) and `a=f` patching**:
//!   because the 2.5D camera is a fixed oblique angle, camera motion over the
//!   voxel terrain is pure 2D translation. An oversized `2.5×` terrain canvas
//!   is baked and uploaded once, then scrolled each tick by moving the
//!   placement's source rectangle `x=<cam_x>,y=<cam_y>,w=<vp_w>,h=<vp_h>`.
//!   When the camera approaches the `16 px` canvas edge margin, a new canvas is
//!   centered on the mech under the alternate terrain image ID and the old one
//!   is deleted. Local terrain modifications patch the stored image in place via
//!   `a=f` ([`kitty::patch_frame_bytes`]).
//!
//! In the web client (`mush-pixel-wasm` / `<canvas>`) and Sixel fallback,
//! [`Iso25DPicture::render_frame`] composites the same `2.5×` cached terrain
//! canvas, 3D-baked mipmapped unit models, ground shadows, silhouette rims,
//! per-pixel tree and building occlusion (`occluder_base_y`), hull/look heading
//! vectors, and target-lock brackets directly into an RGB [`Frame`].

use std::collections::BTreeMap;
#[cfg(feature = "encode")]
use std::collections::BTreeSet;

use serde_json::Value;

#[cfg(feature = "encode")]
use crate::caps::CellSize;
use crate::cockpit::{
    ARENA_MAP_FEED, COCKPIT_FEED, contact_health, contact_kind, map_from_feed, terrain_from_feed,
};
use crate::frame::{Frame, Rgb};
use crate::heightfield::{Terrain, material_from_code, mip_pack};
#[cfg(feature = "encode")]
use crate::kitty::{self, Placement};
use crate::look::{Facing, Material};
use crate::math::Det as _;
use crate::overlay::{
    self, BLUE_TEAM, BORDER, GONE, HEALTH_PIPS, Health, LOCK, PANEL, RED_TEAM, TEXT, ground_rgb,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
#[cfg(feature = "encode")]
use crate::png;
use crate::raycast::{Ground, Map, SpriteKind};

/// Geometric mipmap frame sizes in pixels (~10 % steps: 1.00, 0.90, 0.81,
/// 0.73, 0.66, 0.58).
pub const MIP_SIZES: [usize; 6] = [96, 86, 78, 70, 64, 56];

/// Default cell pixel width when neither SSH `pty-req` nor `CSI 16 t` has
/// reported the terminal's cell size yet.
#[cfg(feature = "encode")]
const DEFAULT_CELL_W: u16 = 8;
/// Default cell pixel height when neither SSH `pty-req` nor `CSI 16 t` has
/// reported the terminal's cell size yet.
#[cfg(feature = "encode")]
const DEFAULT_CELL_H: u16 = 16;

/// Maximum viewport width in pixels at full resolution.
pub const MAX_WIDTH_PX: usize = 1280;
/// Maximum viewport height in pixels at full resolution.
pub const MAX_HEIGHT_PX: usize = 720;
/// Minimum viewport width in pixels.
pub const MIN_WIDTH_PX: usize = 320;
/// Minimum viewport height in pixels.
pub const MIN_HEIGHT_PX: usize = 180;

/// Horizontal transparent padding (in pixels) around each cell in an uploaded
/// sprite sheet so sub-cell horizontal positioning can shift `src_x` without
/// using Kitty's `X=` (which `wezterm-term` applies as per-cell padding).
#[cfg(feature = "encode")]
const SHEET_PAD_X: usize = 16;
/// Vertical transparent padding (in pixels) around each cell in an uploaded
/// sprite sheet so sub-cell vertical positioning can shift `src_y` without
/// using Kitty's `Y=` (which `wezterm-term` applies as per-cell padding).
#[cfg(feature = "encode")]
const SHEET_PAD_Y: usize = 24;

// ---- Stable Kitty Image & Placement IDs ------------------------------------

/// Ping-pong image IDs for the oversized `2.5×` 2.5D terrain ground canvas.
#[cfg(feature = "encode")]
const IMAGE_ID_TERRAIN_A: u32 = 100;
#[cfg(feature = "encode")]
const IMAGE_ID_TERRAIN_B: u32 = 101;
/// Image ID for the compact center tactical HUD vector overlay (`z = -1`).
#[cfg(feature = "encode")]
const IMAGE_ID_HUD: u32 = 130;
/// Image ID for the target-lock bracket overlay (`z = -1`).
#[cfg(feature = "encode")]
const IMAGE_ID_LOCK: u32 = 131;
/// Image ID for transient combat FX (weapon tracers, muzzle flash, impact burst, `z = -1`).
#[cfg(feature = "encode")]
const IMAGE_ID_FX: u32 = 132;
/// Base image ID (`200..`) for lazily uploaded `(UnitModel, PaintKey, mip)`
/// sprite sheets (one image per `(model, paint, mip)` shared by all units via
/// distinct `placement_id`s).
#[cfg(feature = "encode")]
const IMAGE_ID_SHEET_BASE: u32 = 200;

/// Placement ID (`p=1`) for the terrain ground canvas.
#[cfg(feature = "encode")]
const PLACEMENT_TERRAIN: u32 = 1;
/// Placement ID (`p=30`) for the center tactical HUD vector overlay.
#[cfg(feature = "encode")]
const PLACEMENT_HUD: u32 = 30;
/// Placement ID (`p=31`) for the target-lock corner bracket overlay.
#[cfg(feature = "encode")]
const PLACEMENT_LOCK: u32 = 31;
/// Placement ID (`p=32`) for transient combat FX overlay.
#[cfg(feature = "encode")]
const PLACEMENT_FX: u32 = 32;
/// Base unit placement ID (`p=100` = player's own unit, `p=101..` = contacts
/// keyed by contact `"id"`).
const PLACEMENT_UNIT_BASE: u32 = 100;

/// Z-index of the ground terrain placement (below all units and text).
#[cfg(feature = "encode")]
const Z_TERRAIN: i32 = -100;
/// Base Z-index for wrecks (above ground, below all live units).
#[cfg(feature = "encode")]
const Z_WRECK_BASE: i32 = -80;
/// Base Z-index for live units (`-50..-2`).
#[cfg(feature = "encode")]
const Z_DEPTH_BASE: i32 = -50;
/// Z-index for the tactical HUD overlay (above units, below terminal text).
#[cfg(feature = "encode")]
const Z_HUD: i32 = -1;

/// Off-map void colour.
const OFF_MAP_RGB: Rgb = [10, 12, 16];

/// Which baked 2.5D unit sheet set a unit uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitModel {
    /// Walker (`biped_mk1`, `biped_mk2`) — baked from the 3D Fox light mech.
    Biped = 0,
    /// Tracked or wheeled vehicle (`line_tank` family, `heavy_mk1`,
    /// `target_mk1`, `scout_mk1`) — baked from the 3D Tracked Tank.
    Tank = 1,
    /// Unclassified contact or drone/other — reduced from `look::unknown()`.
    Unknown = 2,
}

impl UnitModel {
    /// Choose the 2.5D model family for a classified `unit_type`.
    #[must_use]
    pub fn from_unit_type(unit_type: Option<&str>) -> Self {
        let Some(ut) = unit_type else {
            return Self::Unknown;
        };
        if ut.contains("biped") {
            Self::Biped
        } else if ut.contains("tank")
            || ut.contains("heavy")
            || ut.contains("target")
            || ut.contains("scout")
        {
            Self::Tank
        } else {
            Self::Unknown
        }
    }
}

/// Key uniquely identifying one pre-baked mipmap sprite sheet in terminal
/// storage. All units sharing `(model, paint, mip)` reference this single
/// image via distinct `placement_id`s (`p=100`, `p=101`, …).
#[cfg(feature = "encode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SheetKey {
    model: UnitModel,
    paint: PaintKey,
    mip: u8,
}

#[cfg(feature = "encode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum PaintKey {
    Red = 0,
    Blue = 1,
    Unknown = 2,
    Wreck = 3,
}

#[cfg(feature = "encode")]
impl PaintKey {
    fn from_kind(k: SpriteKind) -> Self {
        match k {
            SpriteKind::Red => Self::Red,
            SpriteKind::Blue => Self::Blue,
            SpriteKind::Unknown => Self::Unknown,
            SpriteKind::Wreck => Self::Wreck,
        }
    }

    fn to_kind(self) -> SpriteKind {
        match self {
            Self::Red => SpriteKind::Red,
            Self::Blue => SpriteKind::Blue,
            Self::Unknown => SpriteKind::Unknown,
            Self::Wreck => SpriteKind::Wreck,
        }
    }
}

#[cfg(feature = "encode")]
impl SheetKey {
    /// Deterministic Kitty image ID (`>= 200`) for this `(model, paint, mip)`.
    fn image_id(self) -> u32 {
        let m = self.model as u32;
        let p = self.paint as u32;
        let mip = u32::from(self.mip);
        IMAGE_ID_SHEET_BASE + (m * 24) + (p * 6) + mip
    }
}

fn material_rgb(mat: Material, kind: SpriteKind) -> Rgb {
    // Unfogged RGB colours matching raycast::Paint's sprite palette.
    let (td, tm, tl) = match kind {
        SpriteKind::Red => ([120, 36, 32], [178, 54, 48], [214, 98, 84]),
        SpriteKind::Blue => ([40, 78, 140], [64, 124, 204], [118, 166, 228]),
        SpriteKind::Unknown => ([92, 92, 88], [132, 132, 126], [174, 174, 166]),
        SpriteKind::Wreck => ([44, 36, 30], [74, 58, 46], [104, 84, 68]),
    };
    match mat {
        Material::TeamDark => td,
        Material::Team => tm,
        Material::TeamLight => tl,
        Material::Black => [20, 22, 26],
        Material::Steel => [118, 124, 132],
        Material::Pale => [216, 216, 198],
    }
}

/// Render an RGBA sprite sheet for `(model, paint, mip)` with a shared
/// `(SHEET_PAD_X, SHEET_PAD_Y)` transparent margin around each of the `6 × 8`
/// frames, a soft ground shadow under the anchor, and a 1-px dark silhouette rim.
///
/// The transparent margin around every frame allows sub-cell smooth positioning
/// by shifting the placement's source rectangle `(src_x, src_y)` while keeping
/// Kitty's `X=0, Y=0` and `c=cols, r=rows` (avoiding `wezterm-term`'s per-cell
/// `X/Y` padding bug).
#[cfg(feature = "encode")]
fn render_sheet_rgba(key: SheetKey) -> (usize, usize, Vec<u8>) {
    let pack = mip_pack();
    let model_idx = key.model as usize;
    let mip_idx = usize::from(key.mip).min(5);
    let Some(mip) = pack.models.get(model_idx).and_then(|m| m.get(mip_idx)) else {
        return (1, 1, vec![0, 0, 0, 0]);
    };
    let n = mip.size;
    let raw_w = 6 * n;
    let stride_x = n + SHEET_PAD_X;
    let stride_y = n + SHEET_PAD_Y;
    let sheet_w = 6 * stride_x + SHEET_PAD_X;
    let sheet_h = 8 * stride_y + SHEET_PAD_Y;
    let mut rgba = vec![0u8; sheet_w * sheet_h * 4];
    let kind = key.paint.to_kind();

    let rx = (n as i32 * 3 / 10).max(4);
    let ry = (n as i32 / 8).max(2);

    for fi in 0..8usize {
        let dst_y0 = SHEET_PAD_Y + fi * stride_y;
        for pi in 0..6usize {
            let dst_x0 = SHEET_PAD_X + pi * stride_x;
            let col_kind = if pi == 5 { SpriteKind::Wreck } else { kind };

            // 1. Soft dark ground shadow ellipse around each cell's anchor.
            let cx = mip.anchor_x as i32;
            let cy = mip.anchor_y as i32;
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
                    let d = (dx * dx * 100) / (rx * rx) + (dy * dy * 100) / (ry * ry);
                    if d <= 100 {
                        let off = ((dst_y0 + ly as usize) * sheet_w + (dst_x0 + lx as usize)) * 4;
                        if let Some(dst) = rgba.get_mut(off..off + 4) {
                            dst.copy_from_slice(&[10, 12, 14, 150]);
                        }
                    }
                }
            }

            // 2. Opaque sprite pixels + 1px dark silhouette rim on immediate neighbors.
            for lr in 0..n {
                let src_r = fi * n + lr;
                let dst_r = dst_y0 + lr;
                for lc in 0..n {
                    let src_c = pi * n + lc;
                    let dst_c = dst_x0 + lc;
                    let code = mip.pixels.get(src_r * raw_w + src_c).copied().unwrap_or(0);
                    if let Some(mat) = material_from_code(code) {
                        let [rr, gg, bb] = material_rgb(mat, col_kind);
                        let off = (dst_r * sheet_w + dst_c) * 4;
                        if let Some(dst) = rgba.get_mut(off..off + 4) {
                            dst.copy_from_slice(&[rr, gg, bb, 255]);
                        }
                    } else {
                        let mut adj = false;
                        for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                            let nr = lr as isize + dr;
                            let nc = lc as isize + dc;
                            if nr >= 0
                                && nr < n as isize
                                && nc >= 0
                                && nc < n as isize
                                && mip
                                    .pixels
                                    .get((fi * n + nr as usize) * raw_w + (pi * n + nc as usize))
                                    .copied()
                                    .unwrap_or(0)
                                    != 0
                            {
                                adj = true;
                                break;
                            }
                        }
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

// ---- Dynamic Viewport Sizing (up to 1280px wide) ---------------------------

/// Pixel and cell geometry for the 2.5D viewport inside `picture_area`.
#[cfg(feature = "encode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport25D {
    /// Viewport width in render pixels (`<= 1280`, exact multiple of `cols` and even).
    pub width_px: usize,
    /// Viewport height in render pixels (`<= 720`, exact multiple of `rows` and even).
    pub height_px: usize,
    /// Viewport width in terminal columns (fills the aspect-preserving box in `picture_area`).
    pub cols: u16,
    /// Viewport height in terminal rows (fills the aspect-preserving box in `picture_area`).
    pub rows: u16,
    /// 0-based column offset from the left of `picture_area`.
    pub col_offset: u16,
    /// 0-based row offset from the top of `picture_area`.
    pub row_offset: u16,
    /// Render pixels per terminal column (`width_px == cols * cell_w`).
    pub cell_w: u16,
    /// Render pixels per terminal row (`height_px == rows * cell_h`).
    pub cell_h: u16,
}

/// Compute the 2.5D viewport dimensions so the cell box `(cols, rows)` fills
/// the player's aspect-preserving `picture_area` (matching `FirstPerson` and
/// `Overhead` even on high-DPI displays), while scaling internal render pixels
/// per cell `(cell_w, cell_h)` cleanly up to `1280×720` (`640×360` when
/// `step_down` is true).
#[cfg(feature = "encode")]
#[must_use]
pub fn viewport_25d(
    area_cols: u16,
    area_rows: u16,
    cell: Option<CellSize>,
    step_down: bool,
) -> Viewport25D {
    let phys_cw = usize::from(cell.map_or(DEFAULT_CELL_W, |c| c.width.max(4)));
    let phys_ch = usize::from(cell.map_or(DEFAULT_CELL_H, |c| c.height.max(6)));
    let avail_cols = usize::from(area_cols.max(2));
    let avail_rows = usize::from(area_rows.max(2));
    let avail_w = avail_cols * phys_cw;
    let avail_h = avail_rows * phys_ch;

    // Match the 4:1 .. 4:3 aspect-preserving cell box inside `picture_area`
    // so the 2.5D view fills the full terminal viewport box on any DPI.
    let (raw_cols, raw_rows) = if avail_h * 4 < avail_w {
        ((avail_h * 4 / phys_cw).clamp(2, avail_cols), avail_rows)
    } else if avail_h * 4 > avail_w * 3 {
        (avail_cols, (avail_w * 3 / 4 / phys_ch).clamp(2, avail_rows))
    } else {
        (avail_cols, avail_rows)
    };
    let box_cols = (raw_cols & !1).max(2);
    let box_rows = (raw_rows & !1).max(2);

    let max_w = if step_down { 640 } else { MAX_WIDTH_PX };
    let max_h = if step_down { 360 } else { MAX_HEIGHT_PX };

    let mut cw_px = phys_cw.clamp(2, SHEET_PAD_X);
    let mut ch_px = ((cw_px * phys_ch + phys_cw / 2) / phys_cw.max(1)).clamp(2, SHEET_PAD_Y);

    if box_cols * cw_px > max_w {
        cw_px = (max_w / box_cols).clamp(2, SHEET_PAD_X);
        ch_px = ((cw_px * phys_ch + phys_cw / 2) / phys_cw.max(1)).clamp(2, SHEET_PAD_Y);
    }
    if box_rows * ch_px > max_h {
        ch_px = (max_h / box_rows).clamp(2, SHEET_PAD_Y);
        cw_px = ((ch_px * phys_cw + phys_ch / 2) / phys_ch.max(1)).clamp(2, SHEET_PAD_X);
    }

    let width_px = (box_cols * cw_px).max(2);
    let height_px = (box_rows * ch_px).max(2);
    let cols = u16::try_from(box_cols).unwrap_or(area_cols);
    let rows = u16::try_from(box_rows).unwrap_or(area_rows);
    let col_offset = area_cols.saturating_sub(cols) / 2;
    let row_offset = 0;

    Viewport25D {
        width_px,
        height_px,
        cols,
        rows,
        col_offset,
        row_offset,
        cell_w: u16::try_from(cw_px).unwrap_or(DEFAULT_CELL_W),
        cell_h: u16::try_from(ch_px).unwrap_or(DEFAULT_CELL_H),
    }
}

// ---- 2.5D Oblique Voxel Terrain + Occluder Depth Baking --------------------

/// Projection scale for a given viewport width.
#[derive(Clone, Copy, Debug)]
struct IsoProj {
    /// Horizontal pixels per world metre (East).
    px_per_m_x: f32,
    /// Vertical ground-plane pixels per world metre (North).
    px_per_m_y: f32,
    /// Vertical extrusion pixels per metre of elevation (`z`).
    px_per_m_z: f32,
}

impl IsoProj {
    fn for_viewport_zoomed(vp_w: usize, zoom_permille: u16) -> Self {
        let z_raw = if zoom_permille == 0 {
            1000
        } else {
            zoom_permille.clamp(500, 2000)
        };
        let s = (vp_w as f32 / 640.0).sqrt().clamp(0.7, 1.45) * (f32::from(z_raw) * 0.001);
        Self {
            px_per_m_x: 1.15 * s,
            px_per_m_y: 0.78 * s,
            px_per_m_z: 1.25 * s,
        }
    }

    /// Map a world position `(wx_cm, wy_cm)` and elevation delta `dz_m` relative
    /// to `(origin_x_cm, origin_y_cm)` onto canvas pixel coordinates `(cx, cy)`
    /// inside a `(canvas_w, canvas_h)` canvas.
    fn world_to_canvas(
        self,
        (wx_cm, wy_cm): (i64, i64),
        dz_m: f32,
        (origin_x_cm, origin_y_cm): (i64, i64),
        (canvas_w, canvas_h): (usize, usize),
    ) -> (i32, i32) {
        let dx_m = (wx_cm - origin_x_cm) as f32 * 0.01;
        let dy_m = (wy_cm - origin_y_cm) as f32 * 0.01;
        let x = (canvas_w as f32 * 0.5) + dx_m * self.px_per_m_x;
        let y = (canvas_h as f32 * 0.5) - dy_m * self.px_per_m_y - dz_m * self.px_per_m_z;
        (x.round() as i32, y.round() as i32)
    }
}

/// Interpolated ground elevation in metres at `(wx_m, wy_m)`.
fn ground_z_m(map: &Map, terrain: Option<&Terrain>, wx_m: f32, wy_m: f32) -> f32 {
    let Some(t) = terrain else {
        return 0.0;
    };
    let cell_m = (map.cell_cm().max(1) as f32) * 0.01;
    let px = wx_m / cell_m;
    let py = wy_m / cell_m;
    t.height(px, py) * cell_m
}

fn scale_rgb(rgb: Rgb, gain: f32) -> Rgb {
    rgb.map(|c| ((f32::from(c) * gain).round().clamp(0.0, 255.0)) as u8)
}

/// Oversized 2.5D terrain ground canvas (`2.5×` viewport, up to `2560×1440`)
/// plus per-pixel `occluder_base_y` depth table for software/WASM compositing.
#[derive(Debug)]
struct TerrainCanvas {
    canvas_w: usize,
    canvas_h: usize,
    vp_w: usize,
    vp_h: usize,
    zoom_permille: u16,
    origin_x_cm: i64,
    origin_y_cm: i64,
    proj: IsoProj,
    ground_frame: Frame,
    /// Base canvas Y (`by`) of any tall feature (`Wall`, `Woods`, `WoodsHeavy`)
    /// drawn at each canvas pixel (`0` for plain ground), enabling pixel-exact
    /// occlusion of units walking behind buildings and trees in `render_frame`.
    occluder_base_y: Vec<u16>,
}

impl TerrainCanvas {
    fn bake(
        map: &Map,
        terrain: Option<&Terrain>,
        origin_x_cm: i64,
        origin_y_cm: i64,
        vp_w: usize,
        vp_h: usize,
        zoom_permille: u16,
    ) -> Self {
        let canvas_w = ((vp_w * 5 / 2) & !1).min(2560).max(vp_w + 64);
        let canvas_h = ((vp_h * 5 / 2) & !1).min(1440).max(vp_h + 64);
        let proj = IsoProj::for_viewport_zoomed(vp_w, zoom_permille);
        let origin_x_m = origin_x_cm as f32 * 0.01;
        let origin_y_m = origin_y_cm as f32 * 0.01;
        let ref_z_m = ground_z_m(map, terrain, origin_x_m, origin_y_m);

        let cell_m = (map.cell_cm().max(1) as f32) * 0.01;
        let map_w_m = (map.width() as f32) * cell_m;
        let map_h_m = (map.height() as f32) * cell_m;

        let mut ground_frame = Frame::new(canvas_w, canvas_h);
        ground_frame.clear(OFF_MAP_RGB);
        let mut occluder_base_y = vec![0u16; canvas_w * canvas_h];

        let half_w = canvas_w as f32 * 0.5;
        let half_h = canvas_h as f32 * 0.5;
        let inv_px_x = 1.0 / proj.px_per_m_x;
        let inv_px_y = 1.0 / proj.px_per_m_y;

        // Precompute a 2.5 m sub-cell texel lookup table (4 texels per cell)
        // covering the visible world bounding box so `t.height()`, normal finite
        // differences, and `sun_gain` are evaluated once per 2.5 m texel instead
        // of four times per canvas pixel (~6× faster bakes).
        let tex_m = (cell_m * 0.25).max(0.25);
        let inv_tex_m = 1.0 / tex_m;
        let min_wx_m = (origin_x_m - half_w * inv_px_x - cell_m).max(0.0);
        let max_wx_m = (origin_x_m + half_w * inv_px_x + cell_m).min(map_w_m);
        let min_wy_m = (origin_y_m - (half_h + 32.0) * inv_px_y - cell_m).max(0.0);
        let max_wy_m = (origin_y_m + (half_h + 32.0) * inv_px_y + cell_m).min(map_h_m);

        let tx0 = ((min_wx_m * inv_tex_m).floor() as i32) - 2;
        let tx1 = ((max_wx_m * inv_tex_m).ceil() as i32) + 2;
        let ty0 = ((min_wy_m * inv_tex_m).floor() as i32) - 2;
        let ty1 = ((max_wy_m * inv_tex_m).ceil() as i32) + 2;
        let grid_w = (tx1 - tx0 + 1).max(4) as usize;
        let grid_h = (ty1 - ty0 + 1).max(4) as usize;

        let mut tex_z = vec![0.0f32; grid_w * grid_h];
        if let Some(t) = terrain {
            for gy in 0..grid_h {
                let py = ((ty0 + gy as i32) as f32) * 0.25;
                let row_off = gy * grid_w;
                for gx in 0..grid_w {
                    let px = ((tx0 + gx as i32) as f32) * 0.25;
                    if let Some(slot) = tex_z.get_mut(row_off + gx) {
                        *slot = t.height(px, py) * cell_m;
                    }
                }
            }
        }

        let mut tex_col = vec![OFF_MAP_RGB; grid_w * grid_h];
        for gy in 0..grid_h {
            let ty = ty0 + gy as i32;
            let wy_m = (ty as f32 + 0.5) * tex_m;
            let cell_y = (wy_m / cell_m).floor() as i64;
            let gy_m1 = gy.saturating_sub(1);
            let gy_p1 = (gy + 1).min(grid_h - 1);
            let row_off = gy * grid_w;
            for gx in 0..grid_w {
                let tx = tx0 + gx as i32;
                let wx_m = (tx as f32 + 0.5) * tex_m;
                let cell_x = (wx_m / cell_m).floor() as i64;
                let ground = map.get(cell_x, cell_y);
                let ground_under = match ground {
                    Ground::Wall => Ground::Road,
                    other => other,
                };
                let checker = ((cell_x as i32) ^ (cell_y as i32)) & 1 != 0;
                let h = (tx as u32)
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add((ty as u32).wrapping_mul(0x85EB_CA6B));
                let mottle = (((h >> 28) as i32) - 8) as f32 * 0.004;

                let (slope_light, contour_darken) = if terrain.is_some() {
                    let gx_m1 = gx.saturating_sub(1);
                    let gx_p1 = (gx + 1).min(grid_w - 1);
                    let z_e = tex_z.get(row_off + gx_p1).copied().unwrap_or(0.0);
                    let z_w = tex_z.get(row_off + gx_m1).copied().unwrap_or(0.0);
                    let z_n = tex_z.get(gy_p1 * grid_w + gx).copied().unwrap_or(0.0);
                    let z_s = tex_z.get(gy_m1 * grid_w + gx).copied().unwrap_or(0.0);
                    let z_c = tex_z.get(row_off + gx).copied().unwrap_or(0.0);
                    // Scale 2-texel (5 m) difference to match 0.7-cell (7 m) span
                    let dz_dx = (z_e - z_w) * 1.4;
                    let dz_dy = (z_n - z_s) * 1.4;
                    let sun_dot = (dz_dy - dz_dx) * 0.14;
                    let steepness = (dz_dx * dz_dx + dz_dy * dz_dy).sqrt();
                    let steep_penalty = (steepness * 0.04).min(0.18);
                    let in_contour = steepness > 0.25 && z_c.rem_euclid(4.0) < 0.28;
                    (
                        (sun_dot - steep_penalty).clamp(-0.35, 0.28),
                        if in_contour { -0.10 } else { 0.0 },
                    )
                } else {
                    (0.0, 0.0)
                };
                let raw = 1.0
                    + slope_light
                    + contour_darken
                    + mottle
                    + if checker { -0.025 } else { 0.0 };
                let q = ((raw - 0.60) / 0.60 * 7.0).round().clamp(0.0, 7.0);
                let sun = 0.60 + (q / 7.0) * 0.60;
                if let Some(slot) = tex_col.get_mut(row_off + gx) {
                    *slot = scale_rgb(ground_rgb(ground_under), sun);
                }
            }
        }

        // Column-by-column back-to-front (North to South, i.e. top of canvas to
        // bottom of canvas) voxel extrusion so nearer ridges naturally overwrite
        // further ground.
        for cx in 0..canvas_w {
            let wx_m = origin_x_m + ((cx as f32) - half_w) * inv_px_x;
            let mut last_sy: usize = 0;
            let fx = wx_m * inv_tex_m - (tx0 as f32);
            let gx0 = (fx.floor() as i32).clamp(0, grid_w as i32 - 2) as usize;
            let tx_frac = (fx - fx.floor()).clamp(0.0, 1.0);
            let cell_x = (wx_m / cell_m).floor() as i64;

            let steps = canvas_h + 64;
            for step in 0..steps {
                let flat_y = (step as f32) - 32.0;
                let wy_m = origin_y_m + (half_h - flat_y) * inv_px_y;

                if wx_m < 0.0 || wy_m < 0.0 || wx_m >= map_w_m || wy_m >= map_h_m {
                    let sy = (flat_y.round() as i32).clamp(0, canvas_h as i32) as usize;
                    for y in last_sy..sy.min(canvas_h) {
                        ground_frame.put(cx, y, OFF_MAP_RGB);
                    }
                    last_sy = sy.min(canvas_h);
                    continue;
                }

                let fy = wy_m * inv_tex_m - (ty0 as f32);
                let gy0 = (fy.floor() as i32).clamp(0, grid_h as i32 - 2) as usize;
                let ty_frac = (fy - fy.floor()).clamp(0.0, 1.0);
                let r0 = gy0 * grid_w + gx0;
                let r1 = (gy0 + 1) * grid_w + gx0;
                let z00 = tex_z.get(r0).copied().unwrap_or(0.0);
                let z10 = tex_z.get(r0 + 1).copied().unwrap_or(z00);
                let z01 = tex_z.get(r1).copied().unwrap_or(z00);
                let z11 = tex_z.get(r1 + 1).copied().unwrap_or(z00);
                let gz = (z00 * (1.0 - tx_frac) + z10 * tx_frac) * (1.0 - ty_frac)
                    + (z01 * (1.0 - tx_frac) + z11 * tx_frac) * ty_frac;

                let dz = gz - ref_z_m;
                let screen_y_i32 = (flat_y - dz * proj.px_per_m_z).round() as i32;
                let sy = screen_y_i32.clamp(0, canvas_h as i32) as usize;

                let cell_y = (wy_m / cell_m).floor() as i64;
                let ground = map.get(cell_x, cell_y);
                let surf_col = tex_col.get(r0).copied().unwrap_or(OFF_MAP_RGB);

                if sy > last_sy + 2 {
                    // Steep South-facing cliff/escarpment facing the camera:
                    // shade the vertical cliff span with stratified rock/earth tone.
                    let cliff_base: Rgb = [64, 72, 54];
                    let cliff_dark: Rgb = [50, 58, 44];
                    for y in last_sy..sy.min(canvas_h) {
                        let col = if (y & 2) == 0 { cliff_base } else { cliff_dark };
                        ground_frame.put(cx, y, col);
                    }
                    if sy < canvas_h {
                        ground_frame.put(cx, sy, surf_col);
                    }
                } else if sy >= last_sy {
                    for y in last_sy..=sy.min(canvas_h.saturating_sub(1)) {
                        ground_frame.put(cx, y, surf_col);
                    }
                } else {
                    for y in sy..=last_sy.min(canvas_h.saturating_sub(1)) {
                        ground_frame.put(cx, y, surf_col);
                    }
                }
                last_sy = sy.min(canvas_h);

                // Solid 3D Sci-Fi Bunker / Outpost Wall extrusion (Manufactura K4 style:
                // bevelled perimeter parapets, vertical structural buttresses, recessed
                // blast doors, and hazard-striped base plinths).
                if ground == Ground::Wall {
                    let occ_top_m = (ground.height_cm() as f32) * 0.01;
                    let h_px = ((occ_top_m * proj.px_per_m_z).round() as i32).clamp(4, 48);
                    let y_bot = screen_y_i32.clamp(0, canvas_h as i32 - 1);
                    let y_top = (screen_y_i32 - h_px).clamp(0, canvas_h as i32 - 1);
                    let south_cell_y = ((wy_m - 1.2) / cell_m).floor() as i64;
                    let north_cell_y = ((wy_m + 1.2) / cell_m).floor() as i64;
                    let west_cell_x = ((wx_m - 1.2) / cell_m).floor() as i64;
                    let east_cell_x = ((wx_m + 1.2) / cell_m).floor() as i64;
                    let is_south_face = map.get(cell_x, south_cell_y) != Ground::Wall;
                    let is_parapet = is_south_face
                        || map.get(cell_x, north_cell_y) != Ground::Wall
                        || map.get(west_cell_x, cell_y) != Ground::Wall
                        || map.get(east_cell_x, cell_y) != Ground::Wall;
                    let sub_x_m = wx_m.rem_euclid(cell_m);
                    let is_buttress = sub_x_m < 1.2 || sub_x_m > (cell_m - 1.2);
                    let is_blast_door = is_south_face
                        && ((cell_x ^ cell_y) & 1 == 0)
                        && (3.2..=6.8).contains(&sub_x_m);
                    let checker = ((cell_x as i32) ^ (cell_y as i32)) & 1 != 0;
                    if y_top < y_bot {
                        let base_u16 = u16::try_from(y_bot.max(0)).unwrap_or(0);
                        for y in y_top..=y_bot {
                            let col: Rgb = if y <= y_top + 1 {
                                if is_parapet {
                                    [158, 164, 172]
                                } else {
                                    [140, 144, 150]
                                }
                            } else if !is_south_face {
                                if is_parapet {
                                    [134, 138, 146]
                                } else if checker {
                                    [118, 122, 128]
                                } else {
                                    [124, 128, 134]
                                }
                            } else if y == y_bot - 1 {
                                // Hazard chevron plinth stripe along South-facing outpost base
                                if ((cx + (y as usize)) & 2) == 0 {
                                    [196, 152, 42]
                                } else {
                                    [46, 50, 56]
                                }
                            } else if y >= y_bot - 2 {
                                [62, 66, 72]
                            } else if is_blast_door && y > y_top + 2 {
                                [54, 60, 68]
                            } else if is_buttress {
                                [108, 112, 120]
                            } else {
                                [88, 92, 98]
                            };
                            let uy = y as usize;
                            ground_frame.put(cx, uy, col);
                            if y < y_bot
                                && let Some(occ) = occluder_base_y.get_mut(uy * canvas_w + cx)
                            {
                                *occ = (*occ).max(base_u16);
                            }
                        }
                    }
                }
            }
        }

        // Back-to-front (North-to-South) 3D conifer tree groves (`Woods`, `WoodsHeavy`)
        // and baked Manufactura K4 sci-fi rooftop structures (`Ground::Wall` compounds).
        let cx0 = (min_wx_m / cell_m).floor() as i64;
        let cx1 = (max_wx_m / cell_m).floor() as i64;
        let cy0 = (min_wy_m / cell_m).floor() as i64;
        let cy1 = (max_wy_m / cell_m).floor() as i64;

        for cy in (cy0..=cy1).rev() {
            for cx_cell in cx0..=cx1 {
                let g = map.get(cx_cell, cy);
                if g == Ground::Wall {
                    if let Some((prop_idx, prop_h_m)) =
                        crate::heightfield::scifi_prop_for_cell(map, cx_cell, cy)
                        && let Some(grid) = crate::heightfield::K4_SCIFI_PROPS.get(prop_idx)
                    {
                        let wall_top_m = (Ground::Wall.height_cm() as f32) * 0.01;
                        let pwx_m = (cx_cell as f32 + 0.5) * cell_m;
                        let pwy_m = (cy as f32 + 0.5) * cell_m;
                        let roof_dz_m =
                            ground_z_m(map, terrain, pwx_m, pwy_m) + wall_top_m - ref_z_m;
                        let (bx, by) = proj.world_to_canvas(
                            ((pwx_m * 100.0) as i64, (pwy_m * 100.0) as i64),
                            roof_dz_m,
                            (origin_x_cm, origin_y_cm),
                            (canvas_w, canvas_h),
                        );
                        let base_u16 = u16::try_from(by.clamp(0, u16::MAX as i32)).unwrap_or(0);
                        for (r_idx, row) in grid.iter().enumerate() {
                            let off_y_m = ((7.5 - r_idx as f32) / 16.0) * 6.5;
                            let row_sy = by - (off_y_m * proj.px_per_m_y).round() as i32;
                            for (c_idx, &cell_val) in row.iter().enumerate() {
                                let hq = cell_val & 0x1F;
                                if hq == 0 {
                                    continue;
                                }
                                let shade_q = cell_val >> 5;
                                let off_x_m = ((c_idx as f32 - 7.5) / 16.0) * 6.5;
                                let px = bx + (off_x_m * proj.px_per_m_x).round() as i32;
                                if px < 0 || (px as usize) >= canvas_w {
                                    continue;
                                }
                                let ux = px as usize;
                                let h_m = (f32::from(hq) / 31.0) * prop_h_m;
                                let col_h_px =
                                    ((h_m * proj.px_per_m_z).round() as i32).clamp(1, 18);
                                for dy in 0..=col_h_px {
                                    let py = row_sy - dy;
                                    if py < 0 || (py as usize) >= canvas_h {
                                        continue;
                                    }
                                    let uy = py as usize;
                                    let col: Rgb = if dy == col_h_px && hq >= 28 && prop_idx < 3 {
                                        // Reactor core / cooling conduit status strip
                                        [86, 188, 214]
                                    } else if dy >= col_h_px - 1 || shade_q >= 6 {
                                        [168, 174, 184]
                                    } else if shade_q >= 4 {
                                        [132, 138, 148]
                                    } else if shade_q >= 2 {
                                        [96, 102, 112]
                                    } else {
                                        [68, 72, 80]
                                    };
                                    ground_frame.put(ux, uy, col);
                                    if let Some(occ) = occluder_base_y.get_mut(uy * canvas_w + ux) {
                                        *occ = (*occ).max(base_u16);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                let tree_count: u32 = match g {
                    Ground::Woods => 3,
                    Ground::WoodsHeavy => 6,
                    _ => continue,
                };
                // Generate deterministic sub-cell trees sorted North-to-South (descending oy_m)
                let mut trees: [(f32, f32, u32); 6] = [(0.0, 0.0, 0); 6];
                for k in 0..tree_count {
                    let seed = ((cx_cell as u32).wrapping_mul(0x9E37_79B9))
                        ^ ((cy as u32).wrapping_mul(0x85EB_CA6B))
                        ^ (k.wrapping_mul(0xC2B2_AE35));
                    let h1 = seed.wrapping_mul(0x27D4_EB2D);
                    let h2 = h1.rotate_left(13).wrapping_mul(0x1656_67B1);
                    let ox_m = 1.4 + ((h1 & 0xFF) as f32 / 255.0) * 7.2;
                    let oy_m = 1.4 + (((h1 >> 8) & 0xFF) as f32 / 255.0) * 7.2;
                    if let Some(slot) = trees.get_mut(k as usize) {
                        *slot = (ox_m, oy_m, h2);
                    }
                }
                let n_trees = tree_count as usize;
                if let Some(slice) = trees.get_mut(..n_trees) {
                    slice.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
                }

                for &(ox_m, oy_m, h2) in trees.iter().take(n_trees) {
                    let twx_m = (cx_cell as f32) * cell_m + ox_m;
                    let twy_m = (cy as f32) * cell_m + oy_m;
                    let tgz_m = ground_z_m(map, terrain, twx_m, twy_m);
                    let (bx, by) = proj.world_to_canvas(
                        ((twx_m * 100.0) as i64, (twy_m * 100.0) as i64),
                        tgz_m - ref_z_m,
                        (origin_x_cm, origin_y_cm),
                        (canvas_w, canvas_h),
                    );
                    let base_h_m = if g == Ground::WoodsHeavy { 8.5 } else { 7.0 };
                    let var_h_m = ((h2 & 0x3F) as f32 / 63.0) * 3.0;
                    let h_px =
                        (((base_h_m + var_h_m) * proj.px_per_m_z).round() as i32).clamp(7, 22);
                    let max_r = (h_px * 3 / 8).clamp(3, 8);
                    let trunk_h = (h_px / 5).clamp(1, 4);
                    let base_u16 = u16::try_from(by.clamp(0, u16::MAX as i32)).unwrap_or(0);

                    // 1. Subtle ground shadow ellipse to SE of trunk base (fixed palette colour)
                    for sdy in -1..=2i32 {
                        for sdx in 0..=(max_r + 2) {
                            let sx = bx + sdx;
                            let sy = by + sdy;
                            if sx >= 0
                                && (sx as usize) < canvas_w
                                && sy >= 0
                                && (sy as usize) < canvas_h
                            {
                                ground_frame.put(sx as usize, sy as usize, [32, 52, 28]);
                            }
                        }
                    }

                    // 2. Trunk
                    for dy in 0..=trunk_h {
                        for dx in -1..=1i32 {
                            let px = bx + dx;
                            let py = by - dy;
                            if px >= 0
                                && (px as usize) < canvas_w
                                && py >= 0
                                && (py as usize) < canvas_h
                            {
                                let col: Rgb = if dx <= 0 { [62, 44, 30] } else { [42, 30, 20] };
                                let ux = px as usize;
                                let uy = py as usize;
                                ground_frame.put(ux, uy, col);
                                if dy > 0
                                    && let Some(occ) = occluder_base_y.get_mut(uy * canvas_w + ux)
                                {
                                    *occ = (*occ).max(base_u16);
                                }
                            }
                        }
                    }

                    // 3. 3-tiered conifer canopy from trunk_h up to h_px
                    let crown_h = (h_px - trunk_h).max(6);
                    for dy in 0..=crown_h {
                        let t = dy as f32 / crown_h as f32;
                        let tier_local = (t * 3.0).fract();
                        let tier_flare = (1.0 - tier_local * 0.45) * (1.05 - 0.78 * t);
                        let r_row = ((max_r as f32) * tier_flare).round() as i32;
                        let py = by - trunk_h - dy;
                        if py < 0 || (py as usize) >= canvas_h {
                            continue;
                        }
                        let uy = py as usize;
                        for dx in -r_row..=r_row {
                            let px = bx + dx;
                            if px < 0 || (px as usize) >= canvas_w {
                                continue;
                            }
                            let edge_hash = ((dx as u32).wrapping_mul(73856093))
                                ^ ((dy as u32).wrapping_mul(19349663))
                                ^ h2;
                            if dx.abs() == r_row && r_row > 2 && (edge_hash & 1) != 0 {
                                continue;
                            }
                            let nx = (dx as f32) / (r_row.max(1) as f32);
                            let light = -nx * 0.55 + (t - 0.4) * 0.35 + (tier_local - 0.3) * 0.25;
                            let col: Rgb = if tier_local < 0.16 && t < 0.85 {
                                [24, 50, 24]
                            } else if light > 0.22 {
                                [72, 126, 54]
                            } else if light > -0.08 {
                                [50, 96, 40]
                            } else {
                                [34, 68, 30]
                            };
                            let ux = px as usize;
                            ground_frame.put(ux, uy, col);
                            if let Some(occ) = occluder_base_y.get_mut(uy * canvas_w + ux) {
                                *occ = (*occ).max(base_u16);
                            }
                        }
                    }
                }
            }
        }

        // Subtle tactical 100 m grid marks on the ground canvas so pure-translation
        // scrolling reads crisply even across open flats.
        let grid_step_cm = 10_000i64; // 100 m
        let min_x_cm = origin_x_cm - ((canvas_w as f32 * inv_px_x * 60.0) as i64);
        let max_x_cm = origin_x_cm + ((canvas_w as f32 * inv_px_x * 60.0) as i64);
        let min_y_cm = origin_y_cm - ((canvas_h as f32 * inv_px_y * 60.0) as i64);
        let max_y_cm = origin_y_cm + ((canvas_h as f32 * inv_px_y * 60.0) as i64);
        let gx0 = (min_x_cm / grid_step_cm) * grid_step_cm;
        let gy0 = (min_y_cm / grid_step_cm) * grid_step_cm;
        let grid_col = ground_rgb(Ground::Road);
        let mut gx = gx0;
        while gx <= max_x_cm {
            let mut gy = gy0;
            while gy <= max_y_cm {
                let wx_m = gx as f32 * 0.01;
                let wy_m = gy as f32 * 0.01;
                if wx_m >= 0.0 && wy_m >= 0.0 && wx_m < map_w_m && wy_m < map_h_m {
                    let dz = ground_z_m(map, terrain, wx_m, wy_m) - ref_z_m;
                    let (px, py) = proj.world_to_canvas(
                        (gx, gy),
                        dz,
                        (origin_x_cm, origin_y_cm),
                        (canvas_w, canvas_h),
                    );
                    for d in -1..=1 {
                        for (x, y) in [(px + d, py), (px, py + d)] {
                            if let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) {
                                ground_frame.put(ux, uy, grid_col);
                            }
                        }
                    }
                }
                gy += grid_step_cm;
            }
            gx += grid_step_cm;
        }

        Self {
            canvas_w,
            canvas_h,
            vp_w,
            vp_h,
            zoom_permille,
            origin_x_cm,
            origin_y_cm,
            proj,
            ground_frame,
            occluder_base_y,
        }
    }

    /// Compute the viewport's top-left offset `(cam_x, cam_y)` inside the `2.5×`
    /// canvas for camera world center `(cam_wx_cm, cam_wy_cm)`. Returns `None`
    /// when the camera is within `16 px` of the canvas edge so a fresh canvas
    /// centered on the mech is baked.
    fn camera_window(&self, cam_wx_cm: i64, cam_wy_cm: i64) -> Option<(u32, u32)> {
        let (cx, cy) = self.proj.world_to_canvas(
            (cam_wx_cm, cam_wy_cm),
            0.0,
            (self.origin_x_cm, self.origin_y_cm),
            (self.canvas_w, self.canvas_h),
        );
        let left = cx - (self.vp_w as i32 / 2);
        let top = cy - (self.vp_h as i32 / 2);
        let margin_x = 16i32;
        let margin_y = 16i32;
        let max_left = (self.canvas_w as i32) - (self.vp_w as i32) - margin_x;
        let max_top = (self.canvas_h as i32) - (self.vp_h as i32) - margin_y;
        if left < margin_x || left > max_left || top < margin_y || top > max_top {
            return None;
        }
        Some((
            u32::try_from(left.max(0)).unwrap_or(0),
            u32::try_from(top.max(0)).unwrap_or(0),
        ))
    }
}

// ---- Cockpit Pose Helper & Visible Unit Descriptor -------------------------

#[derive(Clone, Copy, Debug)]
struct Pose25D {
    x_cm: i64,
    y_cm: i64,
    body_heading_mdeg: u32,
    moving: bool,
}

fn read_pose_25d(frame: &Value) -> Pose25D {
    let sub = frame.get("pose");
    let get_i64 = |k: &str| -> Option<i64> {
        frame
            .get(k)
            .and_then(Value::as_i64)
            .or_else(|| sub.and_then(|p| p.get(k)).and_then(Value::as_i64))
    };
    let x_cm = get_i64("x_cm").unwrap_or(0);
    let y_cm = get_i64("y_cm").unwrap_or(0);
    let heading_raw = get_i64("heading_mdeg").unwrap_or(0).rem_euclid(360_000);
    let body_heading_mdeg = u32::try_from(heading_raw).unwrap_or(0);
    let speed_cms = get_i64("speed_cms").unwrap_or(0);
    let speed_kph = get_i64("speed_kph").unwrap_or(0);
    let moving = speed_cms.abs() > 5 || speed_kph.abs() > 0;
    Pose25D {
        x_cm,
        y_cm,
        body_heading_mdeg,
        moving,
    }
}

#[derive(Clone, Debug)]
struct Unit25D {
    placement_id: u32,
    contact_id: Option<String>,
    designator: Option<String>,
    health: Option<Health>,
    range_m: i64,
    percent: Option<i64>,
    model: UnitModel,
    kind: SpriteKind,
    facing: Facing,
    pose_col: usize,
    mip_idx: u8,
    /// Anchor screen pixel `(x, y)` relative to the viewport top-left.
    screen_x: i32,
    screen_y: i32,
    locked: bool,
}

/// Duration of transient combat FX in 20 Hz ticks (`8` ticks = `400 ms`).
const FX_DURATION_TICKS: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CombatFxKind {
    /// Outgoing weapon volley from player toward a target contact (`id` or `designator`).
    Fire {
        target_key: Option<String>,
        hit: bool,
        kill: bool,
    },
    /// Incoming weapon hit on the player's own hull.
    Struck { severe: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CombatFx25D {
    kind: CombatFxKind,
    ticks_left: u8,
}

#[cfg(feature = "encode")]
struct FxOverlayRgba {
    col_off: u16,
    row_off: u16,
    cols: u16,
    rows: u16,
    width_px: usize,
    height_px: usize,
    rgba: Vec<u8>,
}

/// Convert a unit's world heading (`0..360_000` mdeg clockwise from North) into
/// one of the 8 sprite sheet facings (`Facing::Front` = facing South toward the
/// viewer at `180°`).
fn heading_to_facing_25d(heading_mdeg: u32) -> Facing {
    let deg = ((heading_mdeg % 360_000) as f32) / 1000.0;
    let rel = (180.0 - deg).rem_euclid(360.0);
    let sector = (((rel + 22.5) / 45.0).floor() as usize) & 7;
    Facing::ALL.get(sector).copied().unwrap_or(Facing::Front)
}

/// Pick the geometric mipmap index (`0..=5`, corresponding to `96, 86, 78, 70, 64, 56` px)
/// from the unit's vertical screen position `screen_y` inside `vp_h` and the
/// viewport width `vp_w`.
fn pick_mip_level(screen_y: i32, vp_w: usize, vp_h: usize) -> u8 {
    let y_clamped = screen_y.clamp(0, vp_h as i32) as usize;
    let depth_step = ((vp_h.saturating_sub(y_clamped)) * 3) / vp_h.max(1);
    let res_bias = if vp_w >= 1080 { 2 } else { 3 };
    u8::try_from((depth_step + res_bias).min(5)).unwrap_or(5)
}

// ---- Retained Kitty & Software/WASM 2.5D Picture Producer ------------------

/// Stateful 2.5D tactical producer and retained Kitty graphics compositor.
#[derive(Debug)]
pub struct Iso25DPicture {
    match_id: Option<Option<i64>>,
    map: Map,
    terrain: Option<Terrain>,
    canvas: Option<TerrainCanvas>,
    /// Stable mapping from contact `"id"` (or designator) to Kitty `placement_id`
    /// (`>= 101`) so gaining or losing a contact never reshuffles existing slots.
    contact_pids: BTreeMap<String, u32>,
    next_contact_pid: u32,
    #[cfg(feature = "encode")]
    terrain_image_id: u32,
    #[cfg(feature = "encode")]
    terrain_uploaded: bool,
    #[cfg(feature = "encode")]
    last_terrain_placement: Option<Placement>,
    /// Set of sheet image IDs (`>= 200`) currently stored in terminal memory.
    #[cfg(feature = "encode")]
    uploaded_sheets: BTreeSet<u32>,
    /// Active unit placements from the previous tick: `placement_id -> Placement`.
    #[cfg(feature = "encode")]
    active_unit_placements: BTreeMap<u32, Placement>,
    /// Whether the center HUD vector overlay has been uploaded since the last reset.
    #[cfg(feature = "encode")]
    hud_uploaded: bool,
    #[cfg(feature = "encode")]
    last_hud_signature: u64,
    #[cfg(feature = "encode")]
    last_hud_placement: Option<Placement>,
    /// Whether the target-lock bracket image has been uploaded since the last reset.
    #[cfg(feature = "encode")]
    lock_uploaded: bool,
    #[cfg(feature = "encode")]
    last_lock_signature: u64,
    #[cfg(feature = "encode")]
    last_lock_placement: Option<Placement>,
    #[cfg(feature = "encode")]
    fx_uploaded: bool,
    #[cfg(feature = "encode")]
    last_fx_placement: Option<Placement>,
    #[cfg(feature = "encode")]
    last_viewport: Option<Viewport25D>,
    #[cfg(feature = "encode")]
    last_seated: Option<bool>,
    /// Dead-reckoned ego position offset `(dx_cm, dy_cm)` from the keyframe's `(x_cm, y_cm)`.
    offset_cm: (i64, i64),
    /// Camera zoom in permille (`500..=2000`, `1000` = `1.0×` default).
    zoom_permille: u16,
    /// Last observed `t_ms` so combat FX only trigger on new keyframe pulses.
    last_fx_t_ms: Option<i64>,
    /// Active transient combat FX (weapon tracers, muzzle flash, hit burst, struck ring).
    active_fx: Vec<CombatFx25D>,
    anim_tick: usize,
}

impl Iso25DPicture {
    /// Create a new 2.5D tactical producer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            match_id: None,
            map: Map::new(512, 512, 1000),
            terrain: None,
            canvas: None,
            contact_pids: BTreeMap::new(),
            next_contact_pid: PLACEMENT_UNIT_BASE + 1,
            #[cfg(feature = "encode")]
            terrain_image_id: IMAGE_ID_TERRAIN_A,
            #[cfg(feature = "encode")]
            terrain_uploaded: false,
            #[cfg(feature = "encode")]
            last_terrain_placement: None,
            #[cfg(feature = "encode")]
            uploaded_sheets: BTreeSet::new(),
            #[cfg(feature = "encode")]
            active_unit_placements: BTreeMap::new(),
            #[cfg(feature = "encode")]
            hud_uploaded: false,
            #[cfg(feature = "encode")]
            last_hud_signature: 0,
            #[cfg(feature = "encode")]
            last_hud_placement: None,
            #[cfg(feature = "encode")]
            lock_uploaded: false,
            #[cfg(feature = "encode")]
            last_lock_signature: 0,
            #[cfg(feature = "encode")]
            last_lock_placement: None,
            #[cfg(feature = "encode")]
            fx_uploaded: false,
            #[cfg(feature = "encode")]
            last_fx_placement: None,
            #[cfg(feature = "encode")]
            last_viewport: None,
            #[cfg(feature = "encode")]
            last_seated: None,
            offset_cm: (0, 0),
            zoom_permille: 1000,
            last_fx_t_ms: None,
            active_fx: Vec::new(),
            anim_tick: 0,
        }
    }

    /// Set the dead-reckoned ego position offset `(dx_cm, dy_cm)` from the held
    /// keyframe's `(x_cm, y_cm)` for 20 Hz smooth camera and unit scrolling.
    pub fn set_offset_cm(&mut self, dx_cm: i64, dy_cm: i64) {
        self.offset_cm = (dx_cm, dy_cm);
    }

    /// Set the 2.5D camera zoom in permille (`500..=2000`; `0` or `1000` = `1.0×`).
    pub fn set_zoom_permille(&mut self, zoom_permille: u16) {
        let clamped = if zoom_permille == 0 {
            1000
        } else {
            zoom_permille.clamp(500, 2000)
        };
        if self.zoom_permille != clamped {
            self.zoom_permille = clamped;
            self.canvas = None;
            #[cfg(feature = "encode")]
            {
                self.terrain_uploaded = false;
                self.last_terrain_placement = None;
            }
        }
    }

    /// Current 2.5D camera zoom in permille (`500..=2000`).
    #[must_use]
    pub fn zoom_permille(&self) -> u16 {
        self.zoom_permille
    }

    /// Synchronise the cached heightfield terrain from `feeds` (`arena.map`),
    /// re-baking only when the match ID changes or no map was loaded yet.
    pub fn sync_map(&mut self, feeds: &dyn Feeds) {
        let raw_map = feeds.feed(ARENA_MAP_FEED);
        let wanted = raw_map.and_then(|m| m.get("match").and_then(Value::as_i64));
        let keep = self.match_id == Some(wanted) && (raw_map.is_some() || wanted.is_none());
        if !keep {
            self.map = raw_map.map_or_else(|| Map::new(512, 512, 1000), map_from_feed);
            self.terrain = raw_map.and_then(terrain_from_feed);
            self.match_id = Some(wanted);
            self.canvas = None;
            self.contact_pids.clear();
            self.next_contact_pid = PLACEMENT_UNIT_BASE + 1;
            self.last_fx_t_ms = None;
            self.active_fx.clear();
            #[cfg(feature = "encode")]
            {
                self.terrain_uploaded = false;
                self.last_terrain_placement = None;
            }
        }
    }

    /// Synchronise feeds and render one tick of the retained Kitty compositor
    /// (`CSI ? 2026 h` … `CSI ? 2026 l`), returning `None` if no cockpit frame
    /// is held in `feeds`. When nothing changed since the previous tick, the
    /// returned byte vector is empty so the caller skips sending a frame.
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<CellSize>,
        step_down: bool,
    ) -> Option<(Viewport25D, Vec<u8>)> {
        let cockpit = feeds.feed(COCKPIT_FEED)?;
        self.sync_map(feeds);
        self.offset_cm = (look.dx_cm, look.dy_cm);
        if look.zoom_permille != 0 {
            self.set_zoom_permille(look.zoom_permille);
        }
        let look_off = i32::try_from(look.offset_mdeg).unwrap_or(0);
        let hull_off = i32::try_from(look.hull_offset_mdeg).unwrap_or(0);
        Some(self.render_kitty_bytes(cockpit, (hull_off, look_off), area, cell, step_down))
    }

    /// Load or replace the arena's map directly.
    pub fn set_map(&mut self, map: Map) {
        self.map = map;
        self.terrain = None;
        self.match_id = Some(None);
        self.canvas = None;
        self.contact_pids.clear();
        self.next_contact_pid = PLACEMENT_UNIT_BASE + 1;
        #[cfg(feature = "encode")]
        {
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
        }
    }

    /// Force all retained placements to be re-emitted on the next tick (e.g.
    /// after a text overlay or diagnostic clock switch).
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.last_terrain_placement = None;
        self.active_unit_placements.clear();
        self.last_hud_placement = None;
        self.last_lock_placement = None;
        self.last_fx_placement = None;
    }

    /// Mark an image ID as evicted after a `q=1` terminal error reply so the
    /// next tick re-uploads and re-places it automatically.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == self.terrain_image_id {
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
        } else if image_id == IMAGE_ID_HUD {
            self.hud_uploaded = false;
            self.last_hud_placement = None;
        } else if image_id == IMAGE_ID_LOCK {
            self.lock_uploaded = false;
            self.last_lock_placement = None;
        } else if image_id == IMAGE_ID_FX {
            self.fx_uploaded = false;
            self.last_fx_placement = None;
        } else {
            self.uploaded_sheets.remove(&image_id);
            self.active_unit_placements
                .retain(|_, p| p.image_id != image_id);
        }
    }

    /// Emit Kitty delete commands for every retained image and placement owned
    /// by the 2.5D compositor, resetting terminal state cleanly when switching
    /// views or closing the pixel view.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for p in self.active_unit_placements.values() {
            kitty::delete_placement_bytes(&mut out, p.image_id, p.placement_id);
        }
        self.active_unit_placements.clear();

        if self.terrain_uploaded || self.last_terrain_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_TERRAIN_A);
            kitty::delete_image_bytes(&mut out, IMAGE_ID_TERRAIN_B);
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
        }
        if self.hud_uploaded || self.last_hud_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_HUD);
            self.hud_uploaded = false;
            self.last_hud_placement = None;
        }
        if self.lock_uploaded || self.last_lock_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_LOCK);
            self.lock_uploaded = false;
            self.last_lock_placement = None;
        }
        if self.fx_uploaded || self.last_fx_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_FX);
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

    /// Patch a rectangular region `(dst_x, dst_y, w, h)` of the stored terrain
    /// image in place using Kitty's frame-editing command (`a=f`).
    #[cfg(feature = "encode")]
    pub fn patch_terrain_rect(
        &self,
        out: &mut Vec<u8>,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
    ) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        if !self.terrain_uploaded
            || w == 0
            || h == 0
            || dst_x + w > canvas.canvas_w
            || dst_y + h > canvas.canvas_h
        {
            return;
        }
        let mut sub = Frame::new(w, h);
        for r in 0..h {
            for c in 0..w {
                sub.put(c, r, canvas.ground_frame.get(dst_x + c, dst_y + r));
            }
        }
        let png = png::encode(&sub);
        kitty::patch_frame_bytes(
            out,
            &png,
            self.terrain_image_id,
            u32::try_from(dst_x).unwrap_or(0),
            u32::try_from(dst_y).unwrap_or(0),
        );
    }

    /// Gather the player's own unit and all visible contacts inside the
    /// current 2.5D viewport, assigning each contact a stable `placement_id`
    /// keyed by its `"id"` (or `"designator"`).
    ///
    /// Contacts are anchored in world coordinates at the keyframe's `(pose.x_cm, pose.y_cm)`
    /// while projected relative to the dead-reckoned ego camera `(pose.x_cm + dx_cm, pose.y_cm + dy_cm)`,
    /// so contacts stay fixed to the ground as the player drives at 20 Hz between pulses.
    fn collect_units(
        &mut self,
        frame: &Value,
        pose: &Pose25D,
        hull_offset_mdeg: i32,
        vp_w: usize,
        vp_h: usize,
        step: usize,
    ) -> Vec<Unit25D> {
        let mut units = Vec::new();
        let proj = IsoProj::for_viewport_zoomed(vp_w, self.zoom_permille);
        let base_x_m = pose.x_cm as f32 * 0.01;
        let base_y_m = pose.y_cm as f32 * 0.01;
        let ego_x_cm = pose.x_cm.saturating_add(self.offset_cm.0);
        let ego_y_cm = pose.y_cm.saturating_add(self.offset_cm.1);
        let ego_x_m = ego_x_cm as f32 * 0.01;
        let ego_y_m = ego_y_cm as f32 * 0.01;
        let ref_z_m = ground_z_m(&self.map, self.terrain.as_ref(), ego_x_m, ego_y_m);

        // 1. Player's own unit at the center of the viewport.
        let own_wreck = frame.get("wreck").and_then(Value::as_bool) == Some(true);
        let own_kind = if own_wreck {
            SpriteKind::Wreck
        } else {
            match frame.get("team").and_then(Value::as_str) {
                Some("red") => SpriteKind::Red,
                Some("blue") => SpriteKind::Blue,
                _ => SpriteKind::Blue,
            }
        };
        let own_model = UnitModel::from_unit_type(frame.get("unit_type").and_then(Value::as_str));
        let own_heading = (i64::from(pose.body_heading_mdeg) + i64::from(hull_offset_mdeg))
            .rem_euclid(360_000) as u32;
        let own_facing = heading_to_facing_25d(own_heading);
        let own_moving = pose.moving || (self.offset_cm.0 != 0 || self.offset_cm.1 != 0);
        let own_pose_col = if own_wreck {
            5
        } else if own_moving {
            1 + (step & 3)
        } else {
            0
        };
        let own_sx = (vp_w / 2) as i32;
        let own_sy = (vp_h / 2) as i32;
        units.push(Unit25D {
            placement_id: PLACEMENT_UNIT_BASE,
            contact_id: None,
            designator: None,
            health: None,
            range_m: 0,
            percent: None,
            model: own_model,
            kind: own_kind,
            facing: own_facing,
            pose_col: own_pose_col,
            mip_idx: pick_mip_level(own_sy, vp_w, vp_h),
            screen_x: own_sx,
            screen_y: own_sy,
            locked: false,
        });

        // 2. Contacts from the cockpit frame.
        let lock_id = frame.get("lock").and_then(Value::as_str).or_else(|| {
            frame
                .get("locked")
                .and_then(|l| l.get("id"))
                .and_then(Value::as_str)
        });
        let lock_desig = frame.get("lock_designator").and_then(Value::as_str);
        let lock_range_override = frame.get("lock_range_m").and_then(Value::as_i64);
        let lock_pct = frame.get("lock_percent").and_then(Value::as_i64);
        let Some(contacts) = frame.get("contacts").and_then(Value::as_array) else {
            return units;
        };

        for (ci, c) in contacts.iter().enumerate() {
            let Some(bearing_deg) = c.get("bearing_deg").and_then(Value::as_f64) else {
                continue;
            };
            let Some(range_m) = c.get("range_m").and_then(Value::as_f64) else {
                continue;
            };
            let id = c.get("id").and_then(Value::as_str);
            let desig = c.get("designator").and_then(Value::as_str);
            let key_str = id
                .or(desig)
                .map_or_else(|| format!("idx:{ci}"), str::to_owned);
            let pid = if let Some(&existing) = self.contact_pids.get(&key_str) {
                existing
            } else {
                let assigned = self.next_contact_pid;
                self.next_contact_pid = self.next_contact_pid.saturating_add(1);
                self.contact_pids.insert(key_str, assigned);
                assigned
            };

            let kind = contact_kind(c);
            let model = if kind == SpriteKind::Wreck {
                UnitModel::from_unit_type(c.get("unit_type").and_then(Value::as_str))
                    .min(UnitModel::Tank)
            } else {
                UnitModel::from_unit_type(c.get("unit_type").and_then(Value::as_str))
            };

            let rad = (bearing_deg as f32).to_radians();
            let (sin_b, cos_b) = rad.det_sin_cos();
            let r_f = range_m as f32;
            let wx_m = base_x_m + r_f * sin_b;
            let wy_m = base_y_m + r_f * cos_b;
            let dx_m = wx_m - ego_x_m;
            let dy_m = wy_m - ego_y_m;
            let dz_m = ground_z_m(&self.map, self.terrain.as_ref(), wx_m, wy_m) - ref_z_m;

            let sx = ((vp_w as f32 * 0.5) + dx_m * proj.px_per_m_x).round() as i32;
            let sy = ((vp_h as f32 * 0.5) - dy_m * proj.px_per_m_y - dz_m * proj.px_per_m_z).round()
                as i32;

            // Keep contacts within the viewport plus a 96-px margin for partial clipping.
            if sx < -80 || sx > (vp_w as i32 + 80) || sy < -80 || sy > (vp_h as i32 + 80) {
                continue;
            }

            let moving = c.get("moving").and_then(Value::as_bool).unwrap_or(false);
            let facing_word = c
                .get("facing")
                .or_else(|| c.get("aspect"))
                .and_then(Value::as_str)
                .unwrap_or("front");
            let facing = Facing::from_aspect(facing_word);
            let pose_col = if kind == SpriteKind::Wreck {
                5
            } else if moving {
                1 + (step & 3)
            } else {
                0
            };
            let locked = (lock_id.is_some() && lock_id == id)
                || (lock_desig.is_some() && lock_desig == desig);
            let disp_desig = if locked {
                lock_desig.or(desig).or(id).map(str::to_owned)
            } else {
                desig.or(id).map(str::to_owned)
            };
            let live_range_m = ((dx_m * dx_m + dy_m * dy_m).sqrt()).round() as i64;
            let disp_range_m = if locked {
                lock_range_override.unwrap_or(live_range_m)
            } else {
                live_range_m
            };

            units.push(Unit25D {
                placement_id: pid,
                contact_id: id.map(str::to_owned),
                designator: disp_desig,
                health: contact_health(c),
                range_m: disp_range_m,
                percent: if locked { lock_pct } else { None },
                model,
                kind,
                facing,
                pose_col,
                mip_idx: pick_mip_level(sy, vp_w, vp_h),
                screen_x: sx,
                screen_y: sy,
                locked,
            });
        }

        units
    }

    /// Ingest any new combat `notices` from `frame` into `self.active_fx` when a new
    /// keyframe pulse arrives (`t_ms` advances beyond the previously seen pulse).
    fn sync_combat_fx(&mut self, frame: &Value) {
        let t_ms = frame.get("t_ms").and_then(Value::as_i64).unwrap_or(0);
        let Some(prev_t) = self.last_fx_t_ms else {
            self.last_fx_t_ms = Some(t_ms);
            return;
        };
        if t_ms <= prev_t {
            return;
        }
        self.last_fx_t_ms = Some(t_ms);

        let Some(notices) = frame.get("notices").and_then(Value::as_array) else {
            return;
        };
        if notices.is_empty() {
            return;
        }

        let fallback_target = frame
            .get("lock")
            .and_then(Value::as_str)
            .or_else(|| frame.get("lock_designator").and_then(Value::as_str))
            .map(str::to_owned);

        for n in notices {
            let Some(kind_str) = n.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let target_key = n
                .get("contact")
                .and_then(Value::as_str)
                .or_else(|| n.get("designator").and_then(Value::as_str))
                .map(str::to_owned)
                .or_else(|| fallback_target.clone());
            match kind_str {
                "hit" | "target_section_lost" => {
                    self.active_fx.push(CombatFx25D {
                        kind: CombatFxKind::Fire {
                            target_key,
                            hit: true,
                            kill: false,
                        },
                        ticks_left: FX_DURATION_TICKS,
                    });
                }
                "kill" => {
                    self.active_fx.push(CombatFx25D {
                        kind: CombatFxKind::Fire {
                            target_key,
                            hit: true,
                            kill: true,
                        },
                        ticks_left: FX_DURATION_TICKS,
                    });
                }
                "miss" | "slot_fired" => {
                    self.active_fx.push(CombatFx25D {
                        kind: CombatFxKind::Fire {
                            target_key,
                            hit: false,
                            kill: false,
                        },
                        ticks_left: FX_DURATION_TICKS,
                    });
                }
                "struck" | "struck_spent" => {
                    self.active_fx.push(CombatFx25D {
                        kind: CombatFxKind::Struck { severe: false },
                        ticks_left: FX_DURATION_TICKS,
                    });
                }
                "section_lost" => {
                    self.active_fx.push(CombatFx25D {
                        kind: CombatFxKind::Struck { severe: true },
                        ticks_left: FX_DURATION_TICKS,
                    });
                }
                _ => {}
            }
        }
    }

    fn step_combat_fx(&mut self) {
        for fx in &mut self.active_fx {
            fx.ticks_left = fx.ticks_left.saturating_sub(1);
        }
        self.active_fx.retain(|fx| fx.ticks_left > 0);
    }

    /// Render the compact center tactical HUD overlay (`hud_w × hud_h` RGBA,
    /// ~`128×128` px) with 2-px thick hull and look heading lines.
    #[cfg(feature = "encode")]
    fn render_center_hud_rgba(
        &self,
        hud_w: usize,
        hud_h: usize,
        vp_w: usize,
        pose: &Pose25D,
        hull_offset_mdeg: i32,
        look_offset_mdeg: i32,
    ) -> (u64, Vec<u8>) {
        let hull_mdeg = (i64::from(pose.body_heading_mdeg) + i64::from(hull_offset_mdeg))
            .rem_euclid(360_000) as u32;
        let look_mdeg = (i64::from(pose.body_heading_mdeg) + i64::from(look_offset_mdeg))
            .rem_euclid(360_000) as u32;

        let sig: u64 = (u64::from(hull_mdeg / 1000) << 48)
            ^ (u64::from(look_mdeg / 1000) << 32)
            ^ ((hud_w as u64) << 16)
            ^ (hud_h as u64);

        let mut rgba = vec![0u8; hud_w * hud_h * 4];
        let put = |buf: &mut [u8], x: i32, y: i32, col: [u8; 4]| {
            if x >= 0 && x < hud_w as i32 && y >= 0 && y < hud_h as i32 {
                let off = (y as usize * hud_w + x as usize) * 4;
                if let Some(dst) = buf.get_mut(off..off + 4) {
                    dst.copy_from_slice(&col);
                }
            }
        };

        let cx = (hud_w / 2) as i32;
        let cy = (hud_h / 2) as i32;
        let proj = IsoProj::for_viewport_zoomed(vp_w, self.zoom_permille);

        let vectors = [
            (hull_mdeg, 24.0f32, [90u8, 215, 115, 220]),
            (look_mdeg, 32.0f32, [248u8, 216, 72, 235]),
        ];
        for &(mdeg, len_m, _col) in &vectors {
            let rad = ((mdeg as f32) * 0.001).to_radians();
            let (sin_h, cos_h) = rad.det_sin_cos();
            let steps = 80i32;
            for s in 18..=steps {
                let d_m = len_m * (s as f32 / steps as f32);
                let px = cx + (d_m * sin_h * proj.px_per_m_x).round() as i32;
                let py = cy - (d_m * cos_h * proj.px_per_m_y).round() as i32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        put(&mut rgba, px + dx, py + dy, [12, 14, 18, 170]);
                    }
                }
            }
        }
        for &(mdeg, len_m, col) in &vectors {
            let rad = ((mdeg as f32) * 0.001).to_radians();
            let (sin_h, cos_h) = rad.det_sin_cos();
            let steps = 80i32;
            for s in 18..=steps {
                let d_m = len_m * (s as f32 / steps as f32);
                let px = cx + (d_m * sin_h * proj.px_per_m_x).round() as i32;
                let py = cy - (d_m * cos_h * proj.px_per_m_y).round() as i32;
                put(&mut rgba, px, py, col);
                put(&mut rgba, px + 1, py, col);
                put(&mut rgba, px, py + 1, col);
            }
        }

        (sig, rgba)
    }

    /// Render a compact target-lock corner bracket image (`lock_w × lock_h` RGBA)
    /// with the locked contact's callsign, range, hit-odds `%`, and 3-pip health
    /// gauge baked into the top header strip.
    #[cfg(feature = "encode")]
    fn render_lock_bracket_rgba(
        lock_w: usize,
        lock_h: usize,
        label: &str,
        health: Option<Health>,
    ) -> Vec<u8> {
        let mut rgba = vec![0u8; lock_w * lock_h * 4];
        let put = |buf: &mut [u8], x: i32, y: i32, col: [u8; 4]| {
            if x >= 0 && x < lock_w as i32 && y >= 0 && y < lock_h as i32 {
                let off = (y as usize * lock_w + x as usize) * 4;
                if let Some(dst) = buf.get_mut(off..off + 4) {
                    dst.copy_from_slice(&col);
                }
            }
        };
        let header_h = 14i32;
        let lx0 = 4i32;
        let ly0 = header_h + 2;
        let lx1 = (lock_w as i32 - 5).max(lx0 + 4);
        let ly1 = (lock_h as i32 - 5).max(ly0 + 4);
        let corner = ((lx1 - lx0).min(ly1 - ly0) / 3).max(5);
        let yel = [248u8, 216, 64, 245];
        for d in 0..=corner {
            for t in 0..=1 {
                put(&mut rgba, lx0 + d, ly0 + t, yel);
                put(&mut rgba, lx0 + t, ly0 + d, yel);
                put(&mut rgba, lx1 - d, ly0 + t, yel);
                put(&mut rgba, lx1 - t, ly0 + d, yel);
                put(&mut rgba, lx0 + d, ly1 - t, yel);
                put(&mut rgba, lx0 + t, ly1 - d, yel);
                put(&mut rgba, lx1 - d, ly1 - t, yel);
                put(&mut rgba, lx1 - t, ly1 - d, yel);
            }
        }

        // Top tactical badge: dark backing pill + 5x7 text + optional 3-pip health bar.
        if !label.is_empty() {
            let text_w = (label.chars().count() as i32) * 6 - 1;
            let pips_w = if health.is_some() { 16 } else { 0 };
            let pill_w = (text_w + pips_w + 6).min(lock_w as i32 - 2);
            let px0 = ((lock_w as i32 - pill_w) / 2).max(1);
            let py0 = 1i32;
            for yy in py0..(py0 + 11) {
                for xx in px0..(px0 + pill_w) {
                    let is_border =
                        yy == py0 || yy == py0 + 10 || xx == px0 || xx == px0 + pill_w - 1;
                    let col = if is_border {
                        [96, 104, 120, 235]
                    } else {
                        [16, 18, 24, 225]
                    };
                    put(&mut rgba, xx, yy, col);
                }
            }
            let mut cx = px0 + 3;
            for ch in label.chars() {
                if cx + 5 > px0 + pill_w - pips_w - 1 {
                    break;
                }
                let rows = overlay::glyph(ch)
                    .or_else(|| overlay::glyph('?'))
                    .unwrap_or([0; 7]);
                for (r, bits) in rows.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (0x10 >> col) != 0 {
                            put(&mut rgba, cx + col, py0 + 2 + r as i32, yel);
                        }
                    }
                }
                cx += 6;
            }
            if let Some(h) = health {
                let lit = h.lit();
                let [hr, hg, hb] = h.rgb();
                let pip_x0 = px0 + pill_w - 15;
                let pip_y0 = py0 + 3;
                for i in 0..HEALTH_PIPS {
                    let c = if i < lit {
                        [hr, hg, hb, 250]
                    } else {
                        [GONE[0], GONE[1], GONE[2], 220]
                    };
                    let sx = pip_x0 + (i as i32) * 4;
                    for dy in 0..5 {
                        for dx in 0..3 {
                            put(&mut rgba, sx + dx, pip_y0 + dy, c);
                        }
                    }
                }
            }
        }
        rgba
    }

    /// Render active combat FX (weapon tracers, muzzle flash, impact burst, struck ring)
    /// into a bounded RGBA overlay for the retained Kitty compositor (`IMAGE_ID_FX`).
    #[cfg(feature = "encode")]
    fn render_combat_fx_rgba(&self, units: &[Unit25D], vp: &Viewport25D) -> Option<FxOverlayRgba> {
        if self.active_fx.is_empty() {
            return None;
        }
        let vcx = (vp.width_px / 2) as i32;
        let vcy = (vp.height_px / 2) as i32;
        let gun_x = vcx;
        let gun_y = vcy - 14;

        let mut min_x = (gun_x - 28).max(0);
        let mut max_x = (gun_x + 28).min(vp.width_px as i32 - 1);
        let mut min_y = (gun_y - 28).max(0);
        let mut max_y = (gun_y + 28).min(vp.height_px as i32 - 1);

        for fx in &self.active_fx {
            if let CombatFxKind::Fire { target_key, .. } = &fx.kind
                && let Some(tu) = Self::find_fx_target(units, target_key.as_deref())
            {
                min_x = min_x.min(tu.screen_x - 28).max(0);
                max_x = max_x.max(tu.screen_x + 28).min(vp.width_px as i32 - 1);
                min_y = min_y.min(tu.screen_y - 36).max(0);
                max_y = max_y.max(tu.screen_y + 20).min(vp.height_px as i32 - 1);
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
        let mut rgba = vec![0u8; fx_w * fx_h * 4];

        let mut put = |x: i32, y: i32, col: [u8; 4]| {
            let lx = x - ox;
            let ly = y - oy;
            if lx >= 0 && lx < fx_w as i32 && ly >= 0 && ly < fx_h as i32 {
                let off = (ly as usize * fx_w + lx as usize) * 4;
                if let Some(dst) = rgba.get_mut(off..off + 4) {
                    dst.copy_from_slice(&col);
                }
            }
        };

        Self::paint_combat_fx(&self.active_fx, units, (gun_x, gun_y), &mut put);

        Some(FxOverlayRgba {
            col_off: u16::try_from(c0).unwrap_or(0),
            row_off: u16::try_from(r0).unwrap_or(0),
            cols,
            rows,
            width_px: fx_w,
            height_px: fx_h,
            rgba,
        })
    }

    fn find_fx_target<'a>(units: &'a [Unit25D], target_key: Option<&str>) -> Option<&'a Unit25D> {
        if let Some(tk) = target_key
            && let Some(u) = units.iter().find(|u| {
                u.placement_id != PLACEMENT_UNIT_BASE
                    && (u.contact_id.as_deref() == Some(tk) || u.designator.as_deref() == Some(tk))
            })
        {
            return Some(u);
        }
        units
            .iter()
            .find(|u| u.locked)
            .or_else(|| units.iter().find(|u| u.placement_id != PLACEMENT_UNIT_BASE))
    }

    fn paint_combat_fx<F>(
        active_fx: &[CombatFx25D],
        units: &[Unit25D],
        (gun_x, gun_y): (i32, i32),
        put: &mut F,
    ) where
        F: FnMut(i32, i32, [u8; 4]),
    {
        for fx in active_fx {
            let progress =
                (FX_DURATION_TICKS.saturating_sub(fx.ticks_left)) as f32 / FX_DURATION_TICKS as f32;
            match &fx.kind {
                CombatFxKind::Fire {
                    target_key,
                    hit,
                    kill,
                } => {
                    // 1. Muzzle flash at player's gun mount during the first half of the burst.
                    if fx.ticks_left >= FX_DURATION_TICKS / 2 {
                        let r_m = if fx.ticks_left >= FX_DURATION_TICKS - 2 {
                            5i32
                        } else {
                            3i32
                        };
                        for dy in -r_m..=r_m {
                            for dx in -r_m..=r_m {
                                let d2 = dx * dx + dy * dy;
                                if d2 <= 4 {
                                    put(gun_x + dx, gun_y + dy, [255, 248, 196, 250]);
                                } else if d2 <= r_m * r_m {
                                    put(gun_x + dx, gun_y + dy, [248, 156, 48, 225]);
                                }
                            }
                        }
                    }

                    let Some(tu) = Self::find_fx_target(units, target_key.as_deref()) else {
                        continue;
                    };
                    let tx = tu.screen_x + if *hit { 0 } else { 14 };
                    let ty = tu.screen_y - 14 + if *hit { 0 } else { -10 };

                    // 2. Glowing weapon tracer line from ego gun mount to target.
                    let steps = 48i32;
                    let t_start = (progress * 0.35).clamp(0.0, 0.7);
                    let t_end = (progress * 1.35 + 0.35).clamp(0.25, 1.0);
                    let s0 = (t_start * steps as f32) as i32;
                    let s1 = (t_end * steps as f32) as i32;
                    let tracer_col = if *hit {
                        [255, 226, 96, 245]
                    } else {
                        [214, 186, 118, 210]
                    };
                    for s in s0..=s1 {
                        if !*hit && (s & 3) == 2 {
                            continue;
                        }
                        let f = s as f32 / steps as f32;
                        let px = gun_x + ((tx - gun_x) as f32 * f).round() as i32;
                        let py = gun_y + ((ty - gun_y) as f32 * f).round() as i32;
                        put(px, py, tracer_col);
                        put(px + 1, py, tracer_col);
                        put(px, py + 1, [245, 140, 42, 200]);
                    }

                    // 3. Target impact explosion burst on hit or kill.
                    if *hit {
                        let max_r = if *kill { 15i32 } else { 10i32 };
                        let r_outer = ((0.45 + 0.55 * progress) * max_r as f32)
                            .round()
                            .clamp(4.0, max_r as f32) as i32;
                        let r_core = (r_outer / 2).max(2);
                        for dy in -r_outer..=r_outer {
                            for dx in -r_outer..=r_outer {
                                let d2 = dx * dx + dy * dy;
                                if d2 <= r_core * r_core {
                                    put(tx + dx, ty + dy, [255, 248, 192, 250]);
                                } else if d2 <= (r_outer - 2) * (r_outer - 2) {
                                    put(tx + dx, ty + dy, [246, 122, 36, 235]);
                                } else if d2 <= r_outer * r_outer && ((dx ^ dy) & 1 == 0) {
                                    put(tx + dx, ty + dy, [68, 54, 48, 210]);
                                }
                            }
                        }
                    }
                }
                CombatFxKind::Struck { severe } => {
                    let r_ring = if *severe { 20i32 } else { 15i32 };
                    let col = if *severe {
                        [245, 68, 48, 240]
                    } else {
                        [248, 148, 48, 225]
                    };
                    for deg in (0..360).step_by(8) {
                        let rad = (deg as f32).to_radians();
                        let (s, c) = rad.det_sin_cos();
                        let px = gun_x + (r_ring as f32 * s).round() as i32;
                        let py = gun_y + (r_ring as f32 * 0.75 * c).round() as i32;
                        put(px, py, col);
                        put(px + 1, py, col);
                    }
                }
            }
        }
    }

    /// Produce one tick's retained Kitty compositor command stream (`CSI ? 2026 h`
    /// … `CSI ? 2026 l`), plus the current [`Viewport25D`] geometry.
    ///
    /// Returns an empty `Vec<u8>` when nothing changed since the previous tick so
    /// stationary scenes send zero bytes and never churn terminal cell state.
    #[cfg(feature = "encode")]
    pub fn render_kitty_bytes(
        &mut self,
        frame: &Value,
        (hull_offset_mdeg, look_offset_mdeg): (i32, i32),
        (area_col, area_row, area_cols, area_rows): (u16, u16, u16, u16),
        cell: Option<CellSize>,
        step_down: bool,
    ) -> (Viewport25D, Vec<u8>) {
        let vp = viewport_25d(area_cols, area_rows, cell, step_down);
        let mut ops = Vec::new();

        if self.last_viewport != Some(vp) {
            ops.extend_from_slice(&self.cleanup_kitty_bytes());
            self.canvas = None;
            self.last_viewport = Some(vp);
        }

        let seated = frame.get("seated").and_then(Value::as_bool) != Some(false);
        if !seated {
            if self.last_seated != Some(false) {
                ops.extend_from_slice(&self.cleanup_kitty_bytes());
                self.last_viewport = Some(vp);
                self.last_seated = Some(false);
            }
            if ops.is_empty() {
                return (vp, Vec::new());
            }
            let mut out = Vec::with_capacity(ops.len() + 16);
            out.extend_from_slice(kitty::SYNC_BEGIN);
            out.extend_from_slice(&ops);
            out.extend_from_slice(kitty::SYNC_END);
            return (vp, out);
        }
        self.last_seated = Some(true);

        let pose = read_pose_25d(frame);
        let ego_x_cm = pose.x_cm.saturating_add(self.offset_cm.0);
        let ego_y_cm = pose.y_cm.saturating_add(self.offset_cm.1);
        let ego_moving = pose.moving || (self.offset_cm.0 != 0 || self.offset_cm.1 != 0);
        let any_contact_moving =
            frame
                .get("contacts")
                .and_then(Value::as_array)
                .is_some_and(|cs| {
                    cs.iter()
                        .any(|c| c.get("moving").and_then(Value::as_bool) == Some(true))
                });
        let step = self.anim_tick / 3;
        if ego_moving || any_contact_moving {
            self.anim_tick = self.anim_tick.wrapping_add(1);
        }
        self.sync_combat_fx(frame);

        let base_col = area_col.saturating_add(vp.col_offset);
        let base_row = area_row.saturating_add(vp.row_offset);

        // 1. Ensure the oversized 2.5× terrain canvas covers the mech's current
        //    position; if within 16 px of the edge, bake a new canvas centered on
        //    the mech and ping-pong the terrain image ID.
        let needs_rebake = self
            .canvas
            .as_ref()
            .filter(|c| c.zoom_permille == self.zoom_permille)
            .and_then(|c| c.camera_window(ego_x_cm, ego_y_cm))
            .is_none();

        if needs_rebake {
            let new_canvas = TerrainCanvas::bake(
                &self.map,
                self.terrain.as_ref(),
                ego_x_cm,
                ego_y_cm,
                vp.width_px,
                vp.height_px,
                self.zoom_permille,
            );
            let old_id = self.terrain_image_id;
            let new_id = if self.terrain_uploaded && old_id == IMAGE_ID_TERRAIN_A {
                IMAGE_ID_TERRAIN_B
            } else {
                IMAGE_ID_TERRAIN_A
            };
            let png = png::encode(&new_canvas.ground_frame);
            kitty::transmit_bytes(&mut ops, &png, new_id);
            if self.terrain_uploaded && old_id != new_id {
                kitty::delete_image_bytes(&mut ops, old_id);
            }
            self.terrain_image_id = new_id;
            self.terrain_uploaded = true;
            self.last_terrain_placement = None;
            self.canvas = Some(new_canvas);
        } else if !self.terrain_uploaded
            && let Some(canvas) = &self.canvas
        {
            let png = png::encode(&canvas.ground_frame);
            kitty::transmit_bytes(&mut ops, &png, self.terrain_image_id);
            self.terrain_uploaded = true;
            self.last_terrain_placement = None;
        }

        let (cam_x, cam_y) = self
            .canvas
            .as_ref()
            .and_then(|c| c.camera_window(ego_x_cm, ego_y_cm))
            .unwrap_or((
                u32::try_from(vp.width_px / 2).unwrap_or(0),
                u32::try_from(vp.height_px / 2).unwrap_or(0),
            ));

        // Place the terrain ground window (`z = -100`, `placement_id = PLACEMENT_TERRAIN`)
        // only when its placement parameters (`cam_x`, `cam_y`, `image_id`, `cols`, `rows`) changed.
        let terrain_p = Placement {
            image_id: self.terrain_image_id,
            placement_id: PLACEMENT_TERRAIN,
            col: base_col,
            row: base_row,
            cols: vp.cols,
            rows: vp.rows,
            cell_x: 0,
            cell_y: 0,
            src_x: cam_x,
            src_y: cam_y,
            src_w: u32::try_from(vp.width_px).unwrap_or(640),
            src_h: u32::try_from(vp.height_px).unwrap_or(360),
            z: Z_TERRAIN,
        };
        if self.last_terrain_placement != Some(terrain_p) {
            kitty::place_bytes(&mut ops, &terrain_p);
            self.last_terrain_placement = Some(terrain_p);
        }

        // 2. Collect visible units, lazily upload needed mipmap sprite sheets
        //    (one image per `(UnitModel, PaintKey, mip)`), and emit retained
        //    `a=p,i=<img_id>,p=<pid>` placements using padded source-rect shifts.
        let units = self.collect_units(
            frame,
            &pose,
            hull_offset_mdeg,
            vp.width_px,
            vp.height_px,
            step,
        );
        let pack = mip_pack();
        let mut next_placements = BTreeMap::new();
        let cw_i32 = i32::from(vp.cell_w.max(1));
        let ch_i32 = i32::from(vp.cell_h.max(1));

        for u in &units {
            let paint = PaintKey::from_kind(u.kind);
            let key = SheetKey {
                model: u.model,
                paint,
                mip: u.mip_idx,
            };
            let img_id = key.image_id();
            if !self.uploaded_sheets.contains(&img_id) {
                let (sw, sh, rgba) = render_sheet_rgba(key);
                let png = png::encode_rgba(sw, sh, &rgba);
                kitty::transmit_bytes(&mut ops, &png, img_id);
                self.uploaded_sheets.insert(img_id);
            }

            let Some(mip) = pack
                .models
                .get(u.model as usize)
                .and_then(|m| m.get(usize::from(u.mip_idx)))
            else {
                continue;
            };
            let n = mip.size as i32;
            let top_left_x = u.screen_x - (mip.anchor_x as i32);
            let top_left_y = u.screen_y - (mip.anchor_y as i32);

            let dc = top_left_x.div_euclid(cw_i32);
            let dr = top_left_y.div_euclid(ch_i32);
            let sub_x = top_left_x.rem_euclid(cw_i32);
            let sub_y = top_left_y.rem_euclid(ch_i32);
            let total_cols = (n + sub_x + cw_i32 - 1) / cw_i32;
            let total_rows = (n + sub_y + ch_i32 - 1) / ch_i32;

            let vis_c0 = dc.max(0);
            let vis_r0 = dr.max(0);
            let vis_c1 = (dc + total_cols).min(i32::from(vp.cols));
            let vis_r1 = (dr + total_rows).min(i32::from(vp.rows));
            if vis_c1 <= vis_c0 || vis_r1 <= vis_r0 {
                continue;
            }

            let skip_cols = vis_c0 - dc;
            let skip_rows = vis_r0 - dr;
            let vis_cols = vis_c1 - vis_c0;
            let vis_rows = vis_r1 - vis_r0;

            let stride_x = n + SHEET_PAD_X as i32;
            let stride_y = n + SHEET_PAD_Y as i32;
            let origin_x = (SHEET_PAD_X as i32) + (u.pose_col as i32) * stride_x;
            let origin_y = (SHEET_PAD_Y as i32) + (u.facing as i32) * stride_y;
            let raw_src_x = origin_x - sub_x + skip_cols * cw_i32;
            let raw_src_y = origin_y - sub_y + skip_rows * ch_i32;

            let src_x = u32::try_from(raw_src_x.max(0)).unwrap_or(0);
            let src_y = u32::try_from(raw_src_y.max(0)).unwrap_or(0);
            let src_w = u32::try_from(vis_cols * cw_i32).unwrap_or(1);
            let src_h = u32::try_from(vis_rows * ch_i32).unwrap_or(1);
            let col = base_col.saturating_add(u16::try_from(vis_c0).unwrap_or(0));
            let row = base_row.saturating_add(u16::try_from(vis_r0).unwrap_or(0));
            let unit_cols = u16::try_from(vis_cols).unwrap_or(1);
            let unit_rows = u16::try_from(vis_rows).unwrap_or(1);

            let y_clamped = u.screen_y.clamp(0, vp.height_px as i32);
            let z = if u.kind == SpriteKind::Wreck {
                Z_WRECK_BASE + ((y_clamped * 24) / (vp.height_px.max(1) as i32))
            } else {
                Z_DEPTH_BASE + ((y_clamped * 46) / (vp.height_px.max(1) as i32))
            };

            let unit_p = Placement {
                image_id: img_id,
                placement_id: u.placement_id,
                col,
                row,
                cols: unit_cols,
                rows: unit_rows,
                cell_x: 0,
                cell_y: 0,
                src_x,
                src_y,
                src_w,
                src_h,
                z,
            };

            if let Some(old_p) = self.active_unit_placements.get(&u.placement_id)
                && old_p.image_id != img_id
            {
                kitty::delete_placement_bytes(&mut ops, old_p.image_id, old_p.placement_id);
            }
            if self.active_unit_placements.get(&u.placement_id) != Some(&unit_p) {
                kitty::place_bytes(&mut ops, &unit_p);
            }
            next_placements.insert(u.placement_id, unit_p);
        }

        for (&old_pid, old_p) in &self.active_unit_placements {
            if !next_placements.contains_key(&old_pid) {
                kitty::delete_placement_bytes(&mut ops, old_p.image_id, old_p.placement_id);
            }
        }
        self.active_unit_placements = next_placements;

        // 3. Compact center tactical HUD vector overlay (`z = -1`, `placement_id = PLACEMENT_HUD`).
        let hud_cols = u16::try_from(
            (128usize.div_ceil(usize::from(vp.cell_w.max(1))) & !1)
                .clamp(2, usize::from(vp.cols.max(2))),
        )
        .unwrap_or(12);
        let hud_rows = u16::try_from(
            (128usize.div_ceil(usize::from(vp.cell_h.max(1))) & !1)
                .clamp(2, usize::from(vp.rows.max(2))),
        )
        .unwrap_or(6);
        let hud_col = base_col.saturating_add(vp.cols.saturating_sub(hud_cols) / 2);
        let hud_row = base_row.saturating_add(vp.rows.saturating_sub(hud_rows) / 2);
        let hud_w = usize::from(hud_cols) * usize::from(vp.cell_w.max(1));
        let hud_h = usize::from(hud_rows) * usize::from(vp.cell_h.max(1));

        let (hud_sig, hud_rgba) = self.render_center_hud_rgba(
            hud_w,
            hud_h,
            vp.width_px,
            &pose,
            hull_offset_mdeg,
            look_offset_mdeg,
        );
        let hud_p = Placement {
            image_id: IMAGE_ID_HUD,
            placement_id: PLACEMENT_HUD,
            col: hud_col,
            row: hud_row,
            cols: hud_cols,
            rows: hud_rows,
            cell_x: 0,
            cell_y: 0,
            src_x: 0,
            src_y: 0,
            src_w: u32::try_from(hud_w).unwrap_or(128),
            src_h: u32::try_from(hud_h).unwrap_or(128),
            z: Z_HUD,
        };
        if !self.hud_uploaded
            || hud_sig != self.last_hud_signature
            || self.last_hud_placement != Some(hud_p)
        {
            let png = png::encode_rgba(hud_w, hud_h, &hud_rgba);
            kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_HUD);
            kitty::place_bytes(&mut ops, &hud_p);
            self.hud_uploaded = true;
            self.last_hud_signature = hud_sig;
            self.last_hud_placement = Some(hud_p);
        }

        // 4. Optional target-lock corner bracket (`IMAGE_ID_LOCK`, `placement_id = PLACEMENT_LOCK`)
        //    with baked callsign, range, hit-odds `%`, and 3-pip health gauge.
        if let Some(lu) = units.iter().find(|u| u.locked) {
            let lock_cols = u16::try_from(
                (96usize.div_ceil(usize::from(vp.cell_w.max(1))) & !1)
                    .clamp(2, usize::from(vp.cols.max(2))),
            )
            .unwrap_or(10);
            let lock_rows = u16::try_from(
                (84usize.div_ceil(usize::from(vp.cell_h.max(1))) & !1)
                    .clamp(2, usize::from(vp.rows.max(2))),
            )
            .unwrap_or(5);
            let lock_w = usize::from(lock_cols) * usize::from(vp.cell_w.max(1));
            let lock_h = usize::from(lock_rows) * usize::from(vp.cell_h.max(1));
            let dc = (lu.screen_x / cw_i32 - i32::from(lock_cols) / 2)
                .clamp(0, i32::from(vp.cols.saturating_sub(lock_cols)));
            let dr = (lu.screen_y / ch_i32 - i32::from(lock_rows) * 2 / 3)
                .clamp(0, i32::from(vp.rows.saturating_sub(lock_rows)));
            let desig_str = lu.designator.as_deref().unwrap_or("LOCK");
            let label = match lu.percent {
                Some(pct) => format!("{desig_str} {}M {pct}%", lu.range_m),
                None => format!("{desig_str} {}M", lu.range_m),
            };
            let mut lock_sig = ((lock_w as u64) << 40)
                ^ ((lock_h as u64) << 24)
                ^ (u64::from(lu.health.map_or(0u8, |h| h.lit() as u8)) << 16);
            for b in label.bytes() {
                lock_sig = (lock_sig ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
            if !self.lock_uploaded || self.last_lock_signature != lock_sig {
                let lock_rgba = Self::render_lock_bracket_rgba(lock_w, lock_h, &label, lu.health);
                let png = png::encode_rgba(lock_w, lock_h, &lock_rgba);
                kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_LOCK);
                self.lock_uploaded = true;
                self.last_lock_signature = lock_sig;
            }
            let lock_p = Placement {
                image_id: IMAGE_ID_LOCK,
                placement_id: PLACEMENT_LOCK,
                col: base_col.saturating_add(u16::try_from(dc).unwrap_or(0)),
                row: base_row.saturating_add(u16::try_from(dr).unwrap_or(0)),
                cols: lock_cols,
                rows: lock_rows,
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(lock_w).unwrap_or(96),
                src_h: u32::try_from(lock_h).unwrap_or(84),
                z: Z_HUD,
            };
            if self.last_lock_placement != Some(lock_p) {
                kitty::place_bytes(&mut ops, &lock_p);
                self.last_lock_placement = Some(lock_p);
            }
        } else if self.last_lock_placement.take().is_some() {
            kitty::delete_placement_bytes(&mut ops, IMAGE_ID_LOCK, PLACEMENT_LOCK);
        }

        // 5. Transient combat FX overlay (`IMAGE_ID_FX`, `placement_id = PLACEMENT_FX`).
        if let Some(fx) = self.render_combat_fx_rgba(&units, &vp) {
            let png = png::encode_rgba(fx.width_px, fx.height_px, &fx.rgba);
            kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_FX);
            let fx_p = Placement {
                image_id: IMAGE_ID_FX,
                placement_id: PLACEMENT_FX,
                col: base_col.saturating_add(fx.col_off),
                row: base_row.saturating_add(fx.row_off),
                cols: fx.cols,
                rows: fx.rows,
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(fx.width_px).unwrap_or(64),
                src_h: u32::try_from(fx.height_px).unwrap_or(64),
                z: Z_HUD,
            };
            kitty::place_bytes(&mut ops, &fx_p);
            self.fx_uploaded = true;
            self.last_fx_placement = Some(fx_p);
            self.step_combat_fx();
        } else if self.last_fx_placement.take().is_some() || self.fx_uploaded {
            kitty::delete_image_bytes(&mut ops, IMAGE_ID_FX);
            self.fx_uploaded = false;
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

    /// Software & WASM `<canvas>` compositor (`Frame`): draws the cached `2.5×`
    /// terrain window, unit ground shadows, 3D-baked mipmapped unit sprites with
    /// 1px silhouette rims, pixel-exact tree and building occlusion via
    /// `occluder_base_y`, center hull/look heading vectors, and target-lock
    /// corner brackets.
    pub fn render_frame(
        &mut self,
        frame: &Value,
        hull_offset_mdeg: i32,
        look_offset_mdeg: i32,
        width: usize,
        height: usize,
    ) -> Frame {
        let mut out = Frame::new(width, height);
        if frame.get("seated").and_then(Value::as_bool) == Some(false) {
            overlay::draw_card(&mut out, "NOT IN A COCKPIT");
            return out;
        }
        let pose = read_pose_25d(frame);
        let ego_x_cm = pose.x_cm.saturating_add(self.offset_cm.0);
        let ego_y_cm = pose.y_cm.saturating_add(self.offset_cm.1);
        let ego_moving = pose.moving || (self.offset_cm.0 != 0 || self.offset_cm.1 != 0);
        let any_contact_moving =
            frame
                .get("contacts")
                .and_then(Value::as_array)
                .is_some_and(|cs| {
                    cs.iter()
                        .any(|c| c.get("moving").and_then(Value::as_bool) == Some(true))
                });
        let step = self.anim_tick / 3;
        if ego_moving || any_contact_moving {
            self.anim_tick = self.anim_tick.wrapping_add(1);
        }
        self.sync_combat_fx(frame);

        let needs_rebake = self
            .canvas
            .as_ref()
            .filter(|c| {
                c.vp_w == width && c.vp_h == height && c.zoom_permille == self.zoom_permille
            })
            .and_then(|c| c.camera_window(ego_x_cm, ego_y_cm))
            .is_none();
        if needs_rebake {
            self.canvas = Some(TerrainCanvas::bake(
                &self.map,
                self.terrain.as_ref(),
                ego_x_cm,
                ego_y_cm,
                width,
                height,
                self.zoom_permille,
            ));
        }

        let mut units = self.collect_units(frame, &pose, hull_offset_mdeg, width, height, step);
        units.sort_by_key(|u| (u.kind != SpriteKind::Wreck, u.screen_y));

        let Some(canvas) = &self.canvas else {
            return out;
        };
        let (cam_x, cam_y) = canvas
            .camera_window(ego_x_cm, ego_y_cm)
            .unwrap_or(((width / 2) as u32, (height / 2) as u32));
        let cx0 = cam_x as usize;
        let cy0 = cam_y as usize;

        for r in 0..height {
            for c in 0..width {
                out.put(c, r, canvas.ground_frame.get(cx0 + c, cy0 + r));
            }
        }

        let pack = mip_pack();
        for u in &units {
            let Some(mip) = pack
                .models
                .get(u.model as usize)
                .and_then(|m| m.get(usize::from(u.mip_idx)))
            else {
                continue;
            };
            let n = mip.size;
            let raw_w = 6 * n;
            let top_left_x = u.screen_x - (mip.anchor_x as i32);
            let top_left_y = u.screen_y - (mip.anchor_y as i32);
            let unit_canvas_y =
                u16::try_from((cy0 as i32 + u.screen_y).clamp(0, u16::MAX as i32)).unwrap_or(0);

            let is_occluded = |sx: usize, sy: usize| -> bool {
                canvas
                    .occluder_base_y
                    .get((cy0 + sy) * canvas.canvas_w + (cx0 + sx))
                    .copied()
                    .unwrap_or(0)
                    > unit_canvas_y
            };

            // 1. Soft ground shadow ellipse around unit anchor.
            let rx = (n as i32 * 3 / 10).max(4);
            let ry = (n as i32 / 8).max(2);
            for dy in -ry..=ry {
                let sy_i = u.screen_y + dy;
                if sy_i < 0 || sy_i >= height as i32 {
                    continue;
                }
                let sy = sy_i as usize;
                for dx in -rx..=rx {
                    let sx_i = u.screen_x + dx;
                    if sx_i < 0 || sx_i >= width as i32 {
                        continue;
                    }
                    let d = (dx * dx * 100) / (rx * rx) + (dy * dy * 100) / (ry * ry);
                    if d <= 100 {
                        let sx = sx_i as usize;
                        if !is_occluded(sx, sy) {
                            let bg = out.get(sx, sy);
                            out.put(
                                sx,
                                sy,
                                [
                                    (u16::from(bg[0]) * 55 / 100) as u8,
                                    (u16::from(bg[1]) * 55 / 100) as u8,
                                    (u16::from(bg[2]) * 55 / 100) as u8,
                                ],
                            );
                        }
                    }
                }
            }

            // 2. Opaque sprite pixels + 1px dark silhouette rim, occluded by
            //    any nearer 3D trees or building walls (`occluder_base_y > unit_canvas_y`).
            let col_kind = if u.pose_col == 5 {
                SpriteKind::Wreck
            } else {
                u.kind
            };
            let fi = u.facing as usize;
            let pi = u.pose_col;
            for r in 0..n {
                let sy_i = top_left_y + r as i32;
                if sy_i < 0 || sy_i >= height as i32 {
                    continue;
                }
                let sy = sy_i as usize;
                let src_row = (fi * n + r) * raw_w + pi * n;
                for c in 0..n {
                    let sx_i = top_left_x + c as i32;
                    if sx_i < 0 || sx_i >= width as i32 {
                        continue;
                    }
                    let sx = sx_i as usize;
                    if is_occluded(sx, sy) {
                        continue;
                    }
                    let code = mip.pixels.get(src_row + c).copied().unwrap_or(0);
                    if let Some(mat) = material_from_code(code) {
                        out.put(sx, sy, material_rgb(mat, col_kind));
                    } else {
                        let mut adj = false;
                        for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                            let nr = r as isize + dr;
                            let nc = c as isize + dc;
                            if nr >= 0
                                && nr < n as isize
                                && nc >= 0
                                && nc < n as isize
                                && mip
                                    .pixels
                                    .get((fi * n + nr as usize) * raw_w + (pi * n + nc as usize))
                                    .copied()
                                    .unwrap_or(0)
                                    != 0
                            {
                                adj = true;
                                break;
                            }
                        }
                        if adj {
                            out.put(sx, sy, [12, 14, 18]);
                        }
                    }
                }
            }
        }

        // 3. Center hull (emerald) and look (gold) heading vectors.
        let hull_mdeg = (i64::from(pose.body_heading_mdeg) + i64::from(hull_offset_mdeg))
            .rem_euclid(360_000) as u32;
        let look_mdeg = (i64::from(pose.body_heading_mdeg) + i64::from(look_offset_mdeg))
            .rem_euclid(360_000) as u32;
        let vcx = (width / 2) as i32;
        let vcy = (height / 2) as i32;
        let proj = IsoProj::for_viewport_zoomed(width, self.zoom_permille);
        let vectors = [
            (hull_mdeg, 24.0f32, [90u8, 215, 115]),
            (look_mdeg, 32.0f32, [248u8, 216, 72]),
        ];
        for &(mdeg, len_m, _col) in &vectors {
            let rad = ((mdeg as f32) * 0.001).to_radians();
            let (sin_h, cos_h) = rad.det_sin_cos();
            let steps = 80i32;
            for s in 18..=steps {
                let d_m = len_m * (s as f32 / steps as f32);
                let px = vcx + (d_m * sin_h * proj.px_per_m_x).round() as i32;
                let py = vcy - (d_m * cos_h * proj.px_per_m_y).round() as i32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if let (Ok(ux), Ok(uy)) =
                            (usize::try_from(px + dx), usize::try_from(py + dy))
                        {
                            out.put(ux, uy, [12, 14, 18]);
                        }
                    }
                }
            }
        }
        for &(mdeg, len_m, col) in &vectors {
            let rad = ((mdeg as f32) * 0.001).to_radians();
            let (sin_h, cos_h) = rad.det_sin_cos();
            let steps = 80i32;
            for s in 18..=steps {
                let d_m = len_m * (s as f32 / steps as f32);
                let px = vcx + (d_m * sin_h * proj.px_per_m_x).round() as i32;
                let py = vcy - (d_m * cos_h * proj.px_per_m_y).round() as i32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
                    if let (Ok(ux), Ok(uy)) = (usize::try_from(px + dx), usize::try_from(py + dy)) {
                        out.put(ux, uy, col);
                    }
                }
            }
        }

        // 4. Rising smoke + ember plumes on wrecks, on-unit callsign & health
        //    badges on live contacts, and target-lock bracket + readout on locked unit.
        for u in &units {
            if u.kind == SpriteKind::Wreck {
                for p in 0..5i32 {
                    let phase = ((step as i32) + p * 2) & 7;
                    let sx = u.screen_x + ((p & 1) * 2 - 1) * ((phase + 1) / 3);
                    let sy = u.screen_y - 14 - p * 4 - (phase / 2);
                    let r_smoke = (1 + p / 2).min(3);
                    let col: Rgb = if p == 0 && (phase & 1 == 0) {
                        [235, 118, 42]
                    } else if p <= 1 {
                        [56, 50, 48]
                    } else {
                        [38, 38, 42]
                    };
                    for dy in -r_smoke..=r_smoke {
                        for dx in -r_smoke..=r_smoke {
                            if dx * dx + dy * dy <= r_smoke * r_smoke
                                && let (Ok(ux), Ok(uy)) =
                                    (usize::try_from(sx + dx), usize::try_from(sy + dy))
                            {
                                out.put(ux, uy, col);
                            }
                        }
                    }
                }
                continue;
            }
            if u.placement_id == PLACEMENT_UNIT_BASE || u.locked {
                continue;
            }
            // Non-locked live contact badge: compact callsign + 3-pip health gauge.
            let label = u.designator.as_deref().unwrap_or("");
            if label.is_empty() && u.health.is_none() {
                continue;
            }
            let text_w = if label.is_empty() {
                0
            } else {
                (label.chars().count() as i64) * 6 - 1
            };
            let pips_w = if u.health.is_some() { 14 } else { 0 };
            let sep = if text_w > 0 && pips_w > 0 { 3 } else { 0 };
            let pill_w = (text_w + sep + pips_w + 4).max(10);
            let pill_x = i64::from(u.screen_x) - pill_w / 2;
            let pill_y = i64::from(u.screen_y) - 42;
            overlay::fill_rect(&mut out, pill_x, pill_y, pill_w, 9, PANEL);
            let mut cx = pill_x + 2;
            if !label.is_empty() {
                let team_col = match u.kind {
                    SpriteKind::Red => RED_TEAM,
                    SpriteKind::Blue => BLUE_TEAM,
                    _ => TEXT,
                };
                cx += overlay::text(&mut out, cx, pill_y + 1, label, team_col, 1) + 2;
            }
            if let Some(h) = u.health {
                let lit = h.lit();
                let h_rgb = h.rgb();
                for i in 0..HEALTH_PIPS {
                    let c = if i < lit { h_rgb } else { GONE };
                    overlay::fill_rect(&mut out, cx + i * 4, pill_y + 2, 3, 5, c);
                }
            }
        }

        if let Some(lu) = units.iter().find(|u| u.locked) {
            let half = 32i32;
            let lx0 = lu.screen_x - half;
            let ly0 = lu.screen_y - half - 8;
            let lx1 = lu.screen_x + half;
            let ly1 = lu.screen_y + half - 8;
            let corner = 12i32;
            for d in 0..=corner {
                for t in 0..=1 {
                    for (px, py) in [
                        (lx0 + d, ly0 + t),
                        (lx0 + t, ly0 + d),
                        (lx1 - d, ly0 + t),
                        (lx1 - t, ly0 + d),
                        (lx0 + d, ly1 - t),
                        (lx0 + t, ly1 - d),
                        (lx1 - d, ly1 - t),
                        (lx1 - t, ly1 - d),
                    ] {
                        if let (Ok(ux), Ok(uy)) = (usize::try_from(px), usize::try_from(py)) {
                            out.put(ux, uy, LOCK);
                        }
                    }
                }
            }
            let desig_str = lu.designator.as_deref().unwrap_or("LOCK");
            let label = match lu.percent {
                Some(pct) => format!("{desig_str} {}M {pct}%", lu.range_m),
                None => format!("{desig_str} {}M", lu.range_m),
            };
            let text_w = (label.chars().count() as i64) * 6 - 1;
            let pips_w = if lu.health.is_some() { 15 } else { 0 };
            let pill_w = text_w + pips_w + 6;
            let pill_x = i64::from(lu.screen_x) - pill_w / 2;
            let pill_y = i64::from(ly0) - 12;
            overlay::fill_rect(&mut out, pill_x, pill_y, pill_w, 11, PANEL);
            overlay::outline_rect(&mut out, pill_x, pill_y, pill_w, 11, BORDER);
            let drawn = overlay::text(&mut out, pill_x + 3, pill_y + 2, &label, LOCK, 1);
            if let Some(h) = lu.health {
                let lit = h.lit();
                let h_rgb = h.rgb();
                let px0 = pill_x + 3 + drawn + 2;
                for i in 0..HEALTH_PIPS {
                    let c = if i < lit { h_rgb } else { GONE };
                    overlay::fill_rect(&mut out, px0 + i * 4, pill_y + 3, 3, 5, c);
                }
            }
        }

        // 5. Active combat FX (weapon tracers, muzzle flash, target impact burst, struck ring).
        if !self.active_fx.is_empty() {
            let mut put_rgb = |x: i32, y: i32, col: [u8; 4]| {
                if let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) {
                    out.put(ux, uy, [col[0], col[1], col[2]]);
                }
            };
            Self::paint_combat_fx(&self.active_fx, &units, (vcx, vcy - 14), &mut put_rgb);
            self.step_combat_fx();
        }

        out
    }
}

impl Default for Iso25DPicture {
    fn default() -> Self {
        Self::new()
    }
}

impl Picture for Iso25DPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[COCKPIT_FEED, ARENA_MAP_FEED]
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(cockpit) = feeds.feed(COCKPIT_FEED) else {
            return false;
        };
        self.sync_map(feeds);
        self.offset_cm = (look.dx_cm, look.dy_cm);
        if look.zoom_permille != 0 {
            self.set_zoom_permille(look.zoom_permille);
        }
        let hull_off = i32::try_from(look.hull_offset_mdeg).unwrap_or(0);
        let look_off = i32::try_from(look.offset_mdeg).unwrap_or(0);
        *frame = self.render_frame(cockpit, hull_off, look_off, frame.width(), frame.height());
        true
    }

    fn interval(&self) -> std::time::Duration {
        BUCKET_A_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn embedded_3d_mip_pack_loads_all_six_scales_for_all_models() {
        let pack = mip_pack();
        for model in &pack.models {
            for (mi, &expected_n) in MIP_SIZES.iter().enumerate() {
                let Some(mip) = model.get(mi) else {
                    return;
                };
                assert_eq!(mip.size, expected_n);
                assert_eq!(mip.pixels.len(), 6 * expected_n * 8 * expected_n);
                assert!(mip.pixels.iter().any(|&p| p != 0));
            }
        }
    }

    #[test]
    fn software_compositor_renders_terrain_units_hud_and_occlusion() {
        let mut pic = Iso25DPicture::new();
        pic.set_map(Map::new(128, 128, 1000));
        let frame_json = json!({
            "seated": true,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_000,
            "z_cm": 0,
            "heading_mdeg": 45_000,
            "speed_cms": 800,
            "lock": "RA",
            "contacts": [
                {
                    "id": "RA",
                    "class": "red",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": 45.0,
                    "range_m": 40.0,
                    "facing": "front_left",
                    "moving": true
                }
            ]
        });
        let f1 = pic.render_frame(&frame_json, 0, 15_000, 640, 480);
        assert_eq!(f1.width(), 640);
        assert_eq!(f1.height(), 480);
        assert_ne!(f1.hash(), 0);
    }

    #[cfg(feature = "encode")]
    #[test]
    fn viewport_25d_fills_cell_box_and_scales_up_to_1280() {
        let cell = Some(CellSize {
            width: 10,
            height: 20,
        });
        let hi = viewport_25d(180, 50, cell, false);
        assert_eq!(hi.cols, 180);
        assert_eq!(hi.rows, 50);
        assert!(hi.width_px <= 1280 && hi.width_px >= 1080);
        assert!(hi.height_px <= 720);
        assert_eq!(hi.width_px, usize::from(hi.cols) * usize::from(hi.cell_w));
        assert_eq!(hi.height_px, usize::from(hi.rows) * usize::from(hi.cell_h));

        let lo = viewport_25d(180, 50, cell, true);
        assert_eq!(lo.cols, 180);
        assert_eq!(lo.rows, 50);
        assert!(lo.width_px <= 640);
        assert!(lo.height_px <= 360);
    }

    #[cfg(feature = "encode")]
    #[test]
    fn retained_kitty_compositor_scrolls_by_translation_reuses_sheets_and_skips_idle_ticks() {
        let mut pic = Iso25DPicture::new();
        pic.set_map(Map::new(128, 128, 1000));
        let cell = Some(CellSize {
            width: 8,
            height: 16,
        });

        // Put 8 identical red line_tanks at the same depth to verify all 8 share
        // ONE uploaded sheet image via distinct `p=<pid>` placement IDs (`p=101..108`).
        let mut contacts = Vec::new();
        for i in 0..8 {
            contacts.push(json!({
                "id": format!("R{}", i + 1),
                "class": "red",
                "unit_type": "tacsim.unit.line_tank",
                "bearing_deg": -35.0 + (i as f64) * 10.0,
                "range_m": 40.0,
                "facing": "front_left",
                "moving": false
            }));
        }

        let frame1 = json!({
            "seated": true,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_000,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 0,
            "contacts": contacts
        });

        let (_vp, bytes1) = pic.render_kitty_bytes(&frame1, (0, 0), (1, 2, 120, 40), cell, false);
        let s1 = String::from_utf8_lossy(&bytes1);
        assert!(s1.contains("a=t,f=100,t=d,i=100,q=1"));
        assert!(s1.contains("a=p,i=100,p=1,x="));
        assert!(s1.contains(",p=100,"));
        for pid in 101..=108 {
            assert!(
                s1.contains(&format!(",p={pid},")),
                "missing placement p={pid}"
            );
        }
        assert!(s1.contains("X=0,Y=0,"));
        // Only 2 unit sheets uploaded total: 1 for blue biped + 1 shared by all 8 red tanks!
        assert_eq!(pic.uploaded_sheets.len(), 2);

        // Stationary tick with no changes emits ZERO bytes!
        let (_vp, idle_bytes) =
            pic.render_kitty_bytes(&frame1, (0, 0), (1, 2, 120, 40), cell, false);
        assert!(idle_bytes.is_empty());

        // Moving tick: mech steps north (y_cm = 50_400). Terrain and unit sheets
        // are already uploaded, so steady-state tick emits zero terrain/sheet
        // `a=t` uploads and scrolls via `a=p,i=100,p=1` source-rect!
        let frame2 = json!({
            "seated": true,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_400,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 800,
            "contacts": [
                {
                    "id": "R1",
                    "class": "red",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": 45,
                    "range_m": 38,
                    "facing": "front_left",
                    "moving": true
                }
            ]
        });
        let (_vp, bytes2) = pic.render_kitty_bytes(&frame2, (0, 0), (1, 2, 120, 40), cell, false);
        let s2 = String::from_utf8_lossy(&bytes2);
        assert!(!s2.contains("a=t,f=100,t=d,i=100"));
        assert!(s2.contains("a=p,i=100,p=1,x="));
        // Contacts R2..R8 left the contact list -> deleted by placement_id `p=102..108`!
        assert!(s2.contains("a=d,d=i,i="));
        assert!(s2.contains(",p=102,q=2"));
        assert!(bytes2.len() < bytes1.len() / 5);

        // Simulated eviction of a sheet image ID triggers re-upload on the next tick.
        let Some(&evicted_id) = pic.uploaded_sheets.iter().next() else {
            return;
        };
        pic.invalidate_image(evicted_id);
        let (_vp, bytes3) = pic.render_kitty_bytes(&frame2, (0, 0), (1, 2, 120, 40), cell, false);
        let s3 = String::from_utf8_lossy(&bytes3);
        assert!(s3.contains(&format!("a=t,f=100,t=d,i={evicted_id},q=1")));
    }

    #[test]
    fn dead_reckoning_badges_and_combat_fx_render_in_software_and_kitty() {
        let mut pic = Iso25DPicture::new();
        pic.set_map(Map::new(128, 128, 1000));
        let base_frame = json!({
            "seated": true,
            "t_ms": 1000,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_000,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 1200,
            "lock": "c1",
            "lock_designator": "RA",
            "lock_range_m": 45,
            "lock_percent": 68,
            "contacts": [
                {
                    "id": "c1",
                    "designator": "RA",
                    "class": "red",
                    "health": "damaged",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": 30.0,
                    "range_m": 45.0,
                    "facing": "front_left",
                    "moving": true
                },
                {
                    "id": "c2",
                    "designator": "RB",
                    "class": "red",
                    "health": "crippled",
                    "unit_type": "tacsim.unit.biped_mk1",
                    "bearing_deg": -25.0,
                    "range_m": 55.0,
                    "facing": "front",
                    "moving": false
                }
            ]
        });

        let f0 = pic.render_frame(&base_frame, 0, 0, 640, 480);
        pic.set_offset_cm(300, 400);
        let f_dr = pic.render_frame(&base_frame, 0, 0, 640, 480);
        // Dead-reckoned ego offset scrolls the 2.5D terrain and shifts world-anchored contacts.
        assert_ne!(f0.hash(), f_dr.hash());

        // Trigger combat FX via a new keyframe pulse carrying `hit` and `struck` notices.
        let fx_frame = json!({
            "seated": true,
            "t_ms": 2000,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_000,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 1200,
            "lock": "c1",
            "lock_designator": "RA",
            "lock_range_m": 45,
            "lock_percent": 68,
            "contacts": base_frame["contacts"],
            "notices": [
                { "kind": "hit", "contact": "c1", "designator": "RA", "weapon": "cannon" },
                { "kind": "struck", "direction": "front" }
            ]
        });
        pic.set_offset_cm(0, 0);
        let f_fx = pic.render_frame(&fx_frame, 0, 0, 640, 480);
        assert_ne!(f0.hash(), f_fx.hash());
    }

    #[test]
    fn zoom_and_scifi_outpost_props_render_in_25d() {
        let mut pic = Iso25DPicture::new();
        let mut map = Map::new(32, 32, 1000);
        // Place a 6x6 Ground::Wall outpost compound so interior cells receive
        // Manufactura K4 rooftop generators/silos and perimeter cells get blast doors.
        for cy in 12..18 {
            for cx in 12..18 {
                map.set(cx, cy, Ground::Wall);
            }
        }
        pic.set_map(map);
        let frame = json!({
            "seated": true,
            "t_ms": 1000,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 15_000,
            "y_cm": 10_000,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 0,
            "contacts": [
                {
                    "id": "c1",
                    "designator": "RA",
                    "class": "red",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": 0.0,
                    "range_m": 40.0,
                    "facing": "front",
                    "moving": false
                }
            ]
        });

        let f_100 = pic.render_frame(&frame, 0, 0, 640, 480);
        assert!(pic.canvas.is_some());

        pic.set_zoom_permille(1500);
        assert_eq!(pic.zoom_permille(), 1500);
        let f_150 = pic.render_frame(&frame, 0, 0, 640, 480);
        assert_ne!(f_100.hash(), f_150.hash());

        pic.set_zoom_permille(500);
        assert_eq!(pic.zoom_permille(), 500);
        let f_50 = pic.render_frame(&frame, 0, 0, 640, 480);
        assert_ne!(f_100.hash(), f_50.hash());
        assert_ne!(f_150.hash(), f_50.hash());
    }
}
