//! **The fixed-angle 2.5D tactical view and retained Kitty compositor**.
//!
//! Cycling the camera (`v` in the pixel cockpit) steps across three views:
//! first-person (`cockpit`), overhead (`overhead`), and this oblique 2.5D view
//! (`iso25d`). Unlike the first two views — which rasterise a complete
//! indexed [`Frame`] and push a fresh `a=T` PNG every 50 ms — the 2.5D view
//! drives the Kitty graphics protocol as a **retained hardware sprite and
//! layer compositor**:
//!
//! - **Upload once (`a=t,i=<id>,q=1`)**: unit sprite sheets (8 facings × 6
//!   poses: still, four moving frames, wreck) baked from the 3D models at six
//!   geometric mipmap scales (`96, 86, 78, 70, 64, 56` px, ~10 % steps) are
//!   uploaded lazily the first time a unit type, team paint, and depth scale
//!   appear.
//! - **Place and move (`a=p,i=<id>,p=<pid>,x=…,y=…,w=…,h=…,X=…,Y=…,z=…,C=1,q=1`)**:
//!   each tick updates every visible unit's position, facing, animation frame,
//!   and depth `z` in a single short escape sequence using source-rectangle
//!   selection (`x,y,w,h`) and sub-cell pixel offsets (`X,Y`).
//! - **Pure-translation terrain scrolling (`z = -100`) and `a=f` patching**:
//!   because the 2.5D camera is a fixed oblique angle, camera motion over the
//!   voxel terrain is pure 2D translation. An oversized `2×` terrain canvas is
//!   rendered and uploaded once, then scrolled each tick by moving the
//!   placement's source rectangle `x=<cam_x>,y=<cam_y>,w=<vp_w>,h=<vp_h>`.
//!   When the camera approaches the canvas margin, a new canvas is centered on
//!   the mech under the alternate terrain image ID and the old one is deleted.
//!   Local terrain modifications patch the stored image in place via `a=f`
//!   ([`kitty::patch_frame_bytes`]).
//! - **Depth-sliced terrain occlusion (`z = -50..-2`)**: tall features
//!   (`Wall`, `Woods`, `WoodsHeavy`) are split into horizontal depth strips
//!   whose `z` tracks screen Y, interleaved with unit `z` values so mechs and
//!   tanks walk behind buildings, walls, and trees.
//! - **Dynamic viewport scaling up to 1280 px wide**: scales to ~85 % of the
//!   player's terminal pixel width (from SSH `pty-req` / `window-change` pixel
//!   dimensions or `CSI 16 t`) up to `1280×720` (`640×360` when stepped down).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::caps::CellSize;
use crate::cockpit::{
    ARENA_MAP_FEED, COCKPIT_FEED, contact_kind, map_from_feed, terrain_from_feed,
};
use crate::frame::{Frame, Rgb};
use crate::heightfield::Terrain;
use crate::kitty::{self, Placement};
use crate::look::{self, Facing, Material, Sheet};
use crate::math::Det as _;
use crate::overlay::{self, ground_rgb};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::png;
use crate::raycast::{Ground, Map, SpriteKind};

/// Embedded 6-level 2.5D mipmap sheets (`96, 86, 78, 70, 64, 56` px) baked from
/// the licensed 3D models (`biped` and `tracked_tank`) at a 26° oblique
/// elevation angle.
const EMBEDDED_MIPS: &[u8] = include_bytes!("../looks/iso25d_mips.bin");

/// Geometric mipmap frame sizes in pixels (~10 % steps: 1.00, 0.90, 0.81,
/// 0.73, 0.66, 0.58).
pub const MIP_SIZES: [usize; 6] = [96, 86, 78, 70, 64, 56];

/// Number of horizontal depth strips used to interleave tall terrain occluders
/// (`Wall`, `Woods`, `WoodsHeavy`) with units by screen Y.
const OCCLUDER_STRIPS: usize = 6;

/// Default cell pixel width when neither SSH `pty-req` nor `CSI 16 t` has
/// reported the terminal's cell size yet.
const DEFAULT_CELL_W: u16 = 8;
/// Default cell pixel height when neither SSH `pty-req` nor `CSI 16 t` has
/// reported the terminal's cell size yet.
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
const SHEET_PAD_X: usize = 16;
/// Vertical transparent padding (in pixels) around each cell in an uploaded
/// sprite sheet so sub-cell vertical positioning can shift `src_y` without
/// using Kitty's `Y=` (which `wezterm-term` applies as per-cell padding).
const SHEET_PAD_Y: usize = 24;

// ---- Stable Kitty Image & Placement IDs ------------------------------------

/// Ping-pong image IDs for the oversized 2.5D terrain ground canvas.
const IMAGE_ID_TERRAIN_A: u32 = 100;
const IMAGE_ID_TERRAIN_B: u32 = 101;
/// Shared image ID for the transparent terrain occluder canvas (`Wall`, `Woods`,
/// `WoodsHeavy`); all depth-sliced occluder strip placements window into this
/// single image so terminal GPU memory stays well under `w11term`'s 32 MiB cap.
const IMAGE_ID_OCC_BASE: u32 = 110;
/// Image ID for the compact center tactical HUD vector overlay (`z = -1`).
const IMAGE_ID_HUD: u32 = 130;
/// Image ID for the target-lock bracket overlay (`z = -1`).
const IMAGE_ID_LOCK: u32 = 131;
/// Base image ID (`200..`) for lazily uploaded `(UnitModel, PaintKey, mip)`
/// sprite sheets.
const IMAGE_ID_SHEET_BASE: u32 = 200;

/// Base unit slot key (`100` = player's own unit, `101..` = contacts).
const PLACEMENT_UNIT_BASE: u32 = 100;

/// Z-index of the ground terrain placement (below all units and text).
const Z_TERRAIN: i32 = -100;
/// Base Z-index for wrecks (above ground, below all live units and occluders).
const Z_WRECK_BASE: i32 = -80;
/// Base Z-index for live units and depth-sliced occluder strips (`-50..-2`).
const Z_DEPTH_BASE: i32 = -50;
/// Z-index for the tactical HUD overlay (above units and occluders, below
/// terminal text).
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
/// storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SheetKey {
    model: UnitModel,
    paint: PaintKey,
    mip: u8,
    slot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum PaintKey {
    Red = 0,
    Blue = 1,
    Unknown = 2,
    Wreck = 3,
}

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

impl SheetKey {
    /// Deterministic Kitty image ID (`>= 200`) for this `(model, paint, mip, slot)`.
    fn image_id(self) -> u32 {
        let m = self.model as u32;
        let p = self.paint as u32;
        let mip = u32::from(self.mip);
        let slot = u32::from(self.slot);
        IMAGE_ID_SHEET_BASE + (m * 96) + (p * 24) + (mip * 4) + slot
    }
}

/// Decoded single mip level (`(6 * n) × (8 * n)` material codes `0..=6`).
#[derive(Clone, Debug)]
struct BakedMip {
    size: usize,
    anchor_x: usize,
    anchor_y: usize,
    /// Row-major `(8 * size)` rows by `(6 * size)` columns; `0` = transparent,
    /// `1..=6` = [`Material`].
    pixels: Vec<u8>,
}

/// All 6 mipmap levels for the three unit model families.
struct BakedMipPack {
    models: [[BakedMip; 6]; 3],
}

static MIP_PACK: std::sync::OnceLock<BakedMipPack> = std::sync::OnceLock::new();

fn mip_pack() -> &'static BakedMipPack {
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

fn fallback_mip_from_sheet(sheet: Option<&Sheet>, n: usize) -> BakedMip {
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
    // Scale the hand-drawn sheet into the bottom-center of the n x n cell.
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

    let unk_sheet = look::unknown().and_then(|l| l.sprites.as_ref());
    let biped_sheet = look::for_unit("tacsim.unit.biped_mk1").and_then(|l| l.sprites.as_ref());
    let tank_sheet = look::for_unit("tacsim.unit.line_tank").and_then(|l| l.sprites.as_ref());

    let build_six = |idx: usize, fallback: Option<&Sheet>| -> [BakedMip; 6] {
        std::array::from_fn(|mi| {
            let target_n = MIP_SIZES.get(mi).copied().unwrap_or(64);
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

fn material_from_code(code: u8) -> Option<Material> {
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

// ---- 2.5D Oblique Voxel Terrain + Occluder Strip Baking --------------------

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
    fn for_viewport(vp_w: usize) -> Self {
        let s = (vp_w as f32 / 640.0).clamp(0.5, 2.0);
        Self {
            px_per_m_x: 3.8 * s,
            px_per_m_y: 2.5 * s,
            px_per_m_z: 3.0 * s,
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

/// Sun shade factor (`0.60..=1.20`, quantized to 4 steps so the ground canvas
/// stays within the 256-colour indexed PNG palette) at `(wx_m, wy_m)`.
fn sun_gain(map: &Map, terrain: Option<&Terrain>, wx_m: f32, wy_m: f32, checker: bool) -> f32 {
    let cell_m = (map.cell_cm().max(1) as f32) * 0.01;
    let cx = (wx_m / cell_m).floor() as i64;
    let cy = (wy_m / cell_m).floor() as i64;
    let step = match terrain {
        Some(t) => {
            let rise = i64::from(t.elev_cm(cx, cy)) - i64::from(t.elev_cm(cx - 1, cy + 1));
            (rise / 120).clamp(-2, 2)
        }
        None => 0,
    };
    let base = 1.0 + (step as f32) * 0.08;
    if checker { base * 0.96 } else { base }
}

fn scale_rgb(rgb: Rgb, gain: f32) -> Rgb {
    rgb.map(|c| ((f32::from(c) * gain).round().clamp(0.0, 255.0)) as u8)
}

/// Oversized 2.5D terrain ground canvas (`1.5×` viewport) plus shared
/// transparent occluder layer.
#[derive(Debug)]
struct TerrainCanvas {
    canvas_w: usize,
    canvas_h: usize,
    vp_w: usize,
    vp_h: usize,
    origin_x_cm: i64,
    origin_y_cm: i64,
    proj: IsoProj,
    ground_frame: Frame,
    /// Full-canvas RGBA occluder layer (`canvas_w * canvas_h * 4`).
    occluder_rgba: Vec<u8>,
    /// Whether any tall feature (`Wall`, `Woods`, `WoodsHeavy`) wrote to `occluder_rgba`.
    has_occluders: bool,
}

impl TerrainCanvas {
    fn bake(
        map: &Map,
        terrain: Option<&Terrain>,
        origin_x_cm: i64,
        origin_y_cm: i64,
        vp_w: usize,
        vp_h: usize,
    ) -> Self {
        let canvas_w = ((vp_w * 3 / 2) & !1).max(vp_w + 16);
        let canvas_h = ((vp_h * 3 / 2) & !1).max(vp_h + 16);
        let proj = IsoProj::for_viewport(vp_w);
        let origin_x_m = origin_x_cm as f32 * 0.01;
        let origin_y_m = origin_y_cm as f32 * 0.01;
        let ref_z_m = ground_z_m(map, terrain, origin_x_m, origin_y_m);

        let cell_m = (map.cell_cm().max(1) as f32) * 0.01;
        let map_w_m = (map.width() as f32) * cell_m;
        let map_h_m = (map.height() as f32) * cell_m;

        let mut ground_frame = Frame::new(canvas_w, canvas_h);
        ground_frame.clear(OFF_MAP_RGB);
        let mut occluder_rgba = vec![0u8; canvas_w * canvas_h * 4];
        let mut has_occluders = false;

        // Column-by-column back-to-front (North to South, i.e. top of canvas to
        // bottom of canvas) voxel extrusion so nearer ridges naturally overwrite
        // further ground, while tall features (Walls, Woods) also populate the
        // transparent occluder buffer for unit depth-sorting.
        let half_w = canvas_w as f32 * 0.5;
        let half_h = canvas_h as f32 * 0.5;
        let inv_px_x = 1.0 / proj.px_per_m_x;
        let inv_px_y = 1.0 / proj.px_per_m_y;

        for cx in 0..canvas_w {
            let wx_m = origin_x_m + ((cx as f32) - half_w) * inv_px_x;
            let mut prev_draw_y: usize = 0;

            let steps = canvas_h + 64;
            for step in 0..steps {
                let flat_y = (step as f32) - 32.0;
                let wy_m = origin_y_m + (half_h - flat_y) * inv_px_y;

                if wx_m < 0.0 || wy_m < 0.0 || wx_m >= map_w_m || wy_m >= map_h_m {
                    let sy = (flat_y.round() as i32).clamp(0, canvas_h as i32) as usize;
                    for y in prev_draw_y..sy.min(canvas_h) {
                        ground_frame.put(cx, y, OFF_MAP_RGB);
                    }
                    prev_draw_y = prev_draw_y.max(sy.min(canvas_h));
                    continue;
                }

                let gz = ground_z_m(map, terrain, wx_m, wy_m);
                let dz = gz - ref_z_m;
                let screen_y_i32 = (flat_y - dz * proj.px_per_m_z).round() as i32;
                let sy = screen_y_i32.clamp(0, canvas_h as i32) as usize;

                let cell_x = (wx_m / cell_m).floor() as i64;
                let cell_y = (wy_m / cell_m).floor() as i64;
                let ground = map.get(cell_x, cell_y);
                let checker = ((cell_x as i32) ^ (cell_y as i32)) & 1 != 0;
                let sun = sun_gain(map, terrain, wx_m, wy_m, checker);

                let ground_under = match ground {
                    Ground::Wall => Ground::Road,
                    other => other,
                };
                let surf_col = scale_rgb(ground_rgb(ground_under), sun);

                if sy >= prev_draw_y {
                    for y in prev_draw_y..=sy.min(canvas_h.saturating_sub(1)) {
                        ground_frame.put(cx, y, surf_col);
                    }
                    prev_draw_y = (sy + 1).min(canvas_h);
                } else {
                    let cliff_col =
                        scale_rgb(ground_rgb(ground_under), (sun * 0.72).clamp(0.4, 1.0));
                    for y in sy..prev_draw_y.min(canvas_h) {
                        ground_frame.put(cx, y, cliff_col);
                    }
                }

                // Tall occluders: Wall (buildings/walls) and Woods / WoodsHeavy
                let occ_top_m = (ground.height_cm() as f32) * 0.01;
                if occ_top_m >= 1.5 {
                    let h_px = ((occ_top_m * proj.px_per_m_z).round() as i32).clamp(2, 48);
                    let y_bot = screen_y_i32.clamp(0, canvas_h as i32 - 1);
                    let y_top = (screen_y_i32 - h_px).clamp(0, canvas_h as i32 - 1);
                    let hash = ((cx as u32).wrapping_mul(2654435761))
                        ^ ((step as u32).wrapping_mul(2246822519));
                    let draw_column = match ground {
                        Ground::Wall => true,
                        Ground::WoodsHeavy => (hash & 3) != 0,
                        Ground::Woods => (hash & 1) == 0,
                        _ => false,
                    };
                    if draw_column && y_top < y_bot {
                        has_occluders = true;
                        let base_ground_col = scale_rgb(ground_rgb(ground), sun);
                        for y in y_top..=y_bot {
                            let frac = (y - y_top) as f32 / (y_bot - y_top).max(1) as f32;
                            let (r, g, b, a) = match ground {
                                Ground::Wall => {
                                    if frac < 0.25 {
                                        let v = (128.0_f32 * sun).clamp(60.0_f32, 175.0_f32) as u8;
                                        (v, v.saturating_add(4), v.saturating_add(8), 255u8)
                                    } else {
                                        let v = (96.0_f32 * sun).clamp(42.0_f32, 140.0_f32) as u8;
                                        (v, v, v.saturating_add(4), 255u8)
                                    }
                                }
                                Ground::WoodsHeavy => {
                                    if frac > 0.78 {
                                        (46, 34, 24, 235)
                                    } else if frac < 0.35 {
                                        (44, 88, 42, 245)
                                    } else {
                                        (30, 64, 30, 245)
                                    }
                                }
                                _ => {
                                    if frac > 0.75 {
                                        (54, 40, 28, 220)
                                    } else if frac < 0.40 {
                                        (58, 108, 50, 235)
                                    } else {
                                        (40, 80, 36, 235)
                                    }
                                }
                            };
                            let idx = y as usize * canvas_w + cx;
                            let off = idx * 4;
                            if let Some(dst) = occluder_rgba.get_mut(off..off + 4) {
                                dst.copy_from_slice(&[r, g, b, a]);
                            }
                            ground_frame.put(cx, y as usize, base_ground_col);
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
            origin_x_cm,
            origin_y_cm,
            proj,
            ground_frame,
            occluder_rgba,
            has_occluders,
        }
    }

    /// Whether the rectangular sub-window `(x0..x0+w, y0..y0+h)` inside
    /// `occluder_rgba` contains any non-transparent pixel.
    fn strip_window_has_alpha(&self, x0: usize, y0: usize, w: usize, h: usize) -> bool {
        if !self.has_occluders {
            return false;
        }
        let y_end = (y0 + h).min(self.canvas_h);
        let x_end = (x0 + w).min(self.canvas_w);
        if x0 >= x_end || y0 >= y_end {
            return false;
        }
        for r in y0..y_end {
            let row_start = (r * self.canvas_w + x0) * 4;
            let row_end = (r * self.canvas_w + x_end) * 4;
            if let Some(sl) = self.occluder_rgba.get(row_start..row_end)
                && sl.iter().skip(3).step_by(4).any(|&a| a > 0)
            {
                return true;
            }
        }
        false
    }

    /// Compute the viewport's top-left offset `(cam_x, cam_y)` inside the `1.5×`
    /// canvas for camera world center `(cam_wx_cm, cam_wy_cm)`. Returns `None`
    /// when the camera is close enough to the canvas edge that a fresh canvas
    /// centered on the mech should be baked.
    fn camera_window(&self, cam_wx_cm: i64, cam_wy_cm: i64) -> Option<(u32, u32)> {
        let (cx, cy) = self.proj.world_to_canvas(
            (cam_wx_cm, cam_wy_cm),
            0.0,
            (self.origin_x_cm, self.origin_y_cm),
            (self.canvas_w, self.canvas_h),
        );
        let left = cx - (self.vp_w as i32 / 2);
        let top = cy - (self.vp_h as i32 / 2);
        let margin_x = (self.vp_w as i32) / 12;
        let margin_y = (self.vp_h as i32) / 12;
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
    let depth_step = ((vp_h.saturating_sub(y_clamped)) * 4) / vp_h.max(1);
    let res_bias = if vp_w >= 1080 {
        0
    } else if vp_w >= 800 {
        1
    } else {
        2
    };
    u8::try_from((depth_step + res_bias).min(5)).unwrap_or(5)
}

// ---- Retained Kitty 2.5D Picture Producer ----------------------------------

/// Stateful 2.5D tactical producer and retained Kitty graphics compositor.
#[derive(Debug)]
pub struct Iso25DPicture {
    match_id: Option<Option<i64>>,
    map: Map,
    terrain: Option<Terrain>,
    canvas: Option<TerrainCanvas>,
    terrain_image_id: u32,
    terrain_uploaded: bool,
    last_terrain_placement: Option<Placement>,
    /// Whether each depth-sliced occluder strip (`IMAGE_ID_OCC_BASE + s`) is uploaded for `canvas`.
    occluder_uploaded: [bool; OCCLUDER_STRIPS],
    last_occ_placements: [Option<Placement>; OCCLUDER_STRIPS],
    /// Set of sheet image IDs (`>= 200`) currently stored in terminal memory.
    uploaded_sheets: BTreeSet<u32>,
    /// Active unit placements from the previous tick: `unit_slot -> Placement`.
    active_unit_placements: BTreeMap<u32, Placement>,
    /// Whether the center HUD vector overlay has been uploaded since the last reset.
    hud_uploaded: bool,
    last_hud_signature: u64,
    last_hud_placement: Option<Placement>,
    /// Whether the target-lock bracket image has been uploaded since the last reset.
    lock_uploaded: bool,
    last_lock_signature: u64,
    last_lock_placement: Option<Placement>,
    last_viewport: Option<Viewport25D>,
    last_seated: Option<bool>,
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
            terrain_image_id: IMAGE_ID_TERRAIN_A,
            terrain_uploaded: false,
            last_terrain_placement: None,
            occluder_uploaded: [false; OCCLUDER_STRIPS],
            last_occ_placements: [None; OCCLUDER_STRIPS],
            uploaded_sheets: BTreeSet::new(),
            active_unit_placements: BTreeMap::new(),
            hud_uploaded: false,
            last_hud_signature: 0,
            last_hud_placement: None,
            lock_uploaded: false,
            last_lock_signature: 0,
            last_lock_placement: None,
            last_viewport: None,
            last_seated: None,
            anim_tick: 0,
        }
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
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
            self.occluder_uploaded = [false; OCCLUDER_STRIPS];
            self.last_occ_placements = [None; OCCLUDER_STRIPS];
        }
    }

    /// Synchronise feeds and render one tick of the retained Kitty compositor
    /// (`CSI ? 2026 h` … `CSI ? 2026 l`), returning `None` if no cockpit frame
    /// is held in `feeds`. When nothing changed since the previous tick, the
    /// returned byte vector is empty so the caller skips sending a frame.
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
        self.terrain_uploaded = false;
        self.last_terrain_placement = None;
        self.occluder_uploaded = [false; OCCLUDER_STRIPS];
        self.last_occ_placements = [None; OCCLUDER_STRIPS];
    }

    /// Force all retained placements to be re-emitted on the next tick (e.g.
    /// after a text overlay or diagnostic clock switch).
    pub fn invalidate_placements(&mut self) {
        self.last_terrain_placement = None;
        self.last_occ_placements = [None; OCCLUDER_STRIPS];
        self.active_unit_placements.clear();
        self.last_hud_placement = None;
        self.last_lock_placement = None;
    }

    /// Mark an image ID as evicted after a `q=1` terminal error reply so the
    /// next tick re-uploads and re-places it automatically.
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == self.terrain_image_id {
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
        } else if (IMAGE_ID_OCC_BASE..IMAGE_ID_OCC_BASE + OCCLUDER_STRIPS as u32)
            .contains(&image_id)
        {
            let idx = (image_id - IMAGE_ID_OCC_BASE) as usize;
            if let Some(u) = self.occluder_uploaded.get_mut(idx) {
                *u = false;
            }
            if let Some(p) = self.last_occ_placements.get_mut(idx) {
                *p = None;
            }
        } else if image_id == IMAGE_ID_HUD {
            self.hud_uploaded = false;
            self.last_hud_placement = None;
        } else if image_id == IMAGE_ID_LOCK {
            self.lock_uploaded = false;
            self.last_lock_placement = None;
        } else {
            self.uploaded_sheets.remove(&image_id);
            self.active_unit_placements
                .retain(|_, p| p.image_id != image_id);
        }
    }

    /// Emit Kitty delete commands for every retained image and placement owned
    /// by the 2.5D compositor, resetting terminal state cleanly when switching
    /// views or closing the pixel view.
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for p in self.active_unit_placements.values() {
            kitty::delete_placement_bytes(&mut out, p.image_id, 0);
        }
        self.active_unit_placements.clear();

        if self.terrain_uploaded || self.last_terrain_placement.is_some() {
            kitty::delete_image_bytes(&mut out, IMAGE_ID_TERRAIN_A);
            kitty::delete_image_bytes(&mut out, IMAGE_ID_TERRAIN_B);
            self.terrain_uploaded = false;
            self.last_terrain_placement = None;
        }
        for s in 0..OCCLUDER_STRIPS {
            if self.occluder_uploaded.get(s).copied().unwrap_or(false)
                || self.last_occ_placements.get(s).copied().flatten().is_some()
            {
                kitty::delete_image_bytes(&mut out, IMAGE_ID_OCC_BASE + s as u32);
            }
        }
        self.occluder_uploaded = [false; OCCLUDER_STRIPS];
        self.last_occ_placements = [None; OCCLUDER_STRIPS];

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
    /// current 2.5D viewport.
    fn collect_units(
        &self,
        frame: &Value,
        pose: &Pose25D,
        hull_offset_mdeg: i32,
        vp_w: usize,
        vp_h: usize,
        step: usize,
    ) -> Vec<Unit25D> {
        let mut units = Vec::new();
        let proj = IsoProj::for_viewport(vp_w);
        let eye_x_m = pose.x_cm as f32 * 0.01;
        let eye_y_m = pose.y_cm as f32 * 0.01;
        let ref_z_m = ground_z_m(&self.map, self.terrain.as_ref(), eye_x_m, eye_y_m);

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
        let own_pose_col = if own_wreck {
            5
        } else if pose.moving {
            1 + (step & 3)
        } else {
            0
        };
        let own_sx = (vp_w / 2) as i32;
        let own_sy = (vp_h / 2) as i32;
        units.push(Unit25D {
            placement_id: PLACEMENT_UNIT_BASE,
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
            let id = c.get("id").and_then(Value::as_str).unwrap_or("?");
            let desig = c.get("designator").and_then(Value::as_str);
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
            let dx_m = r_f * sin_b;
            let dy_m = r_f * cos_b;
            let wx_m = eye_x_m + dx_m;
            let wy_m = eye_y_m + dy_m;
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
            let locked = (lock_id.is_some() && lock_id == Some(id))
                || (lock_desig.is_some() && lock_desig == desig);

            units.push(Unit25D {
                placement_id: PLACEMENT_UNIT_BASE + 1 + u32::try_from(ci).unwrap_or(0),
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

    /// Render the compact center tactical HUD overlay (`hud_w × hud_h` RGBA,
    /// ~`128×128` px) with 2-px thick hull and look heading lines.
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
        let proj = IsoProj::for_viewport(vp_w);

        // Draw dark 1px outline first, then 2px thick bright vector core so
        // hull (emerald) and look (gold) vectors read as solid lines.
        let vectors = [
            (hull_mdeg, 13.0f32, [90u8, 215, 115, 220]),
            (look_mdeg, 17.0f32, [248u8, 216, 72, 235]),
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

    /// Render a compact target-lock corner bracket image (`lock_w × lock_h` RGBA).
    fn render_lock_bracket_rgba(lock_w: usize, lock_h: usize) -> Vec<u8> {
        let mut rgba = vec![0u8; lock_w * lock_h * 4];
        let put = |buf: &mut [u8], x: i32, y: i32, col: [u8; 4]| {
            if x >= 0 && x < lock_w as i32 && y >= 0 && y < lock_h as i32 {
                let off = (y as usize * lock_w + x as usize) * 4;
                if let Some(dst) = buf.get_mut(off..off + 4) {
                    dst.copy_from_slice(&col);
                }
            }
        };
        let lx0 = 4i32;
        let ly0 = 4i32;
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
        rgba
    }

    /// Produce one tick's retained Kitty compositor command stream (`CSI ? 2026 h`
    /// … `CSI ? 2026 l`), plus the current [`Viewport25D`] geometry.
    ///
    /// Returns an empty `Vec<u8>` when nothing changed since the previous tick so
    /// stationary scenes send zero bytes and never churn terminal cell state.
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
        let any_contact_moving =
            frame
                .get("contacts")
                .and_then(Value::as_array)
                .is_some_and(|cs| {
                    cs.iter()
                        .any(|c| c.get("moving").and_then(Value::as_bool) == Some(true))
                });
        let step = self.anim_tick / 3;
        if pose.moving || any_contact_moving {
            self.anim_tick = self.anim_tick.wrapping_add(1);
        }

        let base_col = area_col.saturating_add(vp.col_offset);
        let base_row = area_row.saturating_add(vp.row_offset);

        // 1. Ensure the oversized 2.5D terrain canvas covers the mech's current
        //    position; if approaching the margin, bake a new canvas centered on
        //    the mech and ping-pong the terrain image ID.
        let needs_rebake = self
            .canvas
            .as_ref()
            .and_then(|c| c.camera_window(pose.x_cm, pose.y_cm))
            .is_none();

        if needs_rebake {
            let new_canvas = TerrainCanvas::bake(
                &self.map,
                self.terrain.as_ref(),
                pose.x_cm,
                pose.y_cm,
                vp.width_px,
                vp.height_px,
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
            for s in 0..OCCLUDER_STRIPS {
                if self.occluder_uploaded.get(s).copied().unwrap_or(false)
                    || self.last_occ_placements.get(s).copied().flatten().is_some()
                {
                    kitty::delete_image_bytes(&mut ops, IMAGE_ID_OCC_BASE + s as u32);
                }
            }
            self.occluder_uploaded = [false; OCCLUDER_STRIPS];
            self.last_occ_placements = [None; OCCLUDER_STRIPS];
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
            .and_then(|c| c.camera_window(pose.x_cm, pose.y_cm))
            .unwrap_or((
                u32::try_from(vp.width_px / 4).unwrap_or(0),
                u32::try_from(vp.height_px / 4).unwrap_or(0),
            ));

        // Place the terrain ground window (`z = -100`, `placement_id = 0`) only
        // when its placement parameters (`cam_x`, `cam_y`, `image_id`, `cols`, `rows`) changed!
        let terrain_p = Placement {
            image_id: self.terrain_image_id,
            placement_id: 0,
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

        // 2. Depth-sliced occluder strips (`OCCLUDER_STRIPS = 6`), each with its
        //    own `IMAGE_ID_OCC_BASE + s` and `placement_id = 0` (`p=` omitted),
        //    uploaded lazily as a cropped horizontal band and placed only when
        //    non-transparent occluder pixels are visible in that strip's window.
        if let Some(canvas) = &self.canvas {
            if canvas.has_occluders {
                let strip_rows = (vp.rows / u16::try_from(OCCLUDER_STRIPS).unwrap_or(6)).max(1);
                let max_cam_y = canvas.canvas_h.saturating_sub(vp.height_px);
                for s in 0..OCCLUDER_STRIPS {
                    let row_off = u16::try_from(s).unwrap_or(0) * strip_rows;
                    if row_off >= vp.rows {
                        break;
                    }
                    let rows_this = if s + 1 == OCCLUDER_STRIPS {
                        vp.rows.saturating_sub(row_off)
                    } else {
                        strip_rows.min(vp.rows.saturating_sub(row_off))
                    };
                    let y0 = usize::from(row_off) * usize::from(vp.cell_h);
                    let h_px = usize::from(rows_this) * usize::from(vp.cell_h);
                    if h_px == 0 || y0 >= canvas.canvas_h {
                        continue;
                    }
                    let vis_alpha = canvas.strip_window_has_alpha(
                        cam_x as usize,
                        cam_y as usize + y0,
                        vp.width_px,
                        h_px,
                    );
                    let occ_img_id = IMAGE_ID_OCC_BASE + s as u32;
                    if vis_alpha {
                        if !self.occluder_uploaded.get(s).copied().unwrap_or(false) {
                            let band_h = (max_cam_y + h_px).min(canvas.canvas_h.saturating_sub(y0));
                            let row_bytes = canvas.canvas_w * 4;
                            let byte_start = y0 * row_bytes;
                            let byte_end = (y0 + band_h) * row_bytes;
                            if let Some(band_slice) = canvas.occluder_rgba.get(byte_start..byte_end)
                            {
                                let png = png::encode_rgba(canvas.canvas_w, band_h, band_slice);
                                kitty::transmit_bytes(&mut ops, &png, occ_img_id);
                            }
                            if let Some(u) = self.occluder_uploaded.get_mut(s) {
                                *u = true;
                            }
                            if let Some(slot) = self.last_occ_placements.get_mut(s) {
                                *slot = None;
                            }
                        }
                        let strip_row = base_row.saturating_add(row_off);
                        let strip_bottom_y = (y0 + h_px) as i32;
                        let z_strip =
                            Z_DEPTH_BASE + ((strip_bottom_y * 46) / (vp.height_px.max(1) as i32));
                        let occ_p = Placement {
                            image_id: occ_img_id,
                            placement_id: 0,
                            col: base_col,
                            row: strip_row,
                            cols: vp.cols,
                            rows: rows_this,
                            cell_x: 0,
                            cell_y: 0,
                            src_x: cam_x,
                            src_y: cam_y,
                            src_w: u32::try_from(vp.width_px).unwrap_or(640),
                            src_h: u32::try_from(h_px).unwrap_or(60),
                            z: z_strip,
                        };
                        if self.last_occ_placements.get(s).copied().flatten() != Some(occ_p) {
                            kitty::place_bytes(&mut ops, &occ_p);
                            if let Some(slot) = self.last_occ_placements.get_mut(s) {
                                *slot = Some(occ_p);
                            }
                        }
                    } else if let Some(slot) = self.last_occ_placements.get_mut(s)
                        && slot.take().is_some()
                    {
                        kitty::delete_placement_bytes(&mut ops, occ_img_id, 0);
                    }
                }
            } else {
                for s in 0..OCCLUDER_STRIPS {
                    if self.occluder_uploaded.get(s).copied().unwrap_or(false)
                        || self.last_occ_placements.get(s).copied().flatten().is_some()
                    {
                        kitty::delete_image_bytes(&mut ops, IMAGE_ID_OCC_BASE + s as u32);
                    }
                }
                self.occluder_uploaded = [false; OCCLUDER_STRIPS];
                self.last_occ_placements = [None; OCCLUDER_STRIPS];
            }
        }

        // 3. Collect visible units, lazily upload needed mipmap sprite sheets,
        //    and emit retained `a=p` placements using padded source-rect shifts
        //    (`X=0, Y=0, c=cols, r=rows, placement_id=0`) and delta-only updates!
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
        let mut slot_counts: BTreeMap<(UnitModel, PaintKey, u8), u8> = BTreeMap::new();
        let cw_i32 = i32::from(vp.cell_w.max(1));
        let ch_i32 = i32::from(vp.cell_h.max(1));

        for u in &units {
            let paint = PaintKey::from_kind(u.kind);
            let slot_entry = slot_counts.entry((u.model, paint, u.mip_idx)).or_insert(0);
            let slot = *slot_entry;
            if slot >= 4 {
                continue;
            }
            *slot_entry = slot + 1;

            let key = SheetKey {
                model: u.model,
                paint,
                mip: u.mip_idx,
                slot,
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
                placement_id: 0,
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
                kitty::delete_placement_bytes(&mut ops, old_p.image_id, 0);
            }
            if self.active_unit_placements.get(&u.placement_id) != Some(&unit_p) {
                kitty::place_bytes(&mut ops, &unit_p);
            }
            next_placements.insert(u.placement_id, unit_p);
        }

        for (&old_pid, old_p) in &self.active_unit_placements {
            if !next_placements.contains_key(&old_pid) {
                kitty::delete_placement_bytes(&mut ops, old_p.image_id, 0);
            }
        }
        self.active_unit_placements = next_placements;

        // 4. Compact center tactical HUD vector overlay (`z = -1`, `placement_id = 0`).
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
            placement_id: 0,
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

        // 5. Optional target-lock corner bracket (`IMAGE_ID_LOCK`, `placement_id = 0`).
        if let Some(lu) = units.iter().find(|u| u.locked) {
            let lock_cols = u16::try_from(
                (72usize.div_ceil(usize::from(vp.cell_w.max(1))) & !1)
                    .clamp(2, usize::from(vp.cols.max(2))),
            )
            .unwrap_or(8);
            let lock_rows = u16::try_from(
                (72usize.div_ceil(usize::from(vp.cell_h.max(1))) & !1)
                    .clamp(2, usize::from(vp.rows.max(2))),
            )
            .unwrap_or(4);
            let lock_w = usize::from(lock_cols) * usize::from(vp.cell_w.max(1));
            let lock_h = usize::from(lock_rows) * usize::from(vp.cell_h.max(1));
            let dc = (lu.screen_x / cw_i32 - i32::from(lock_cols) / 2)
                .clamp(0, i32::from(vp.cols.saturating_sub(lock_cols)));
            let dr = (lu.screen_y / ch_i32 - i32::from(lock_rows) * 2 / 3)
                .clamp(0, i32::from(vp.rows.saturating_sub(lock_rows)));
            let lock_sig = ((lock_w as u64) << 16) ^ (lock_h as u64);
            if !self.lock_uploaded || self.last_lock_signature != lock_sig {
                let lock_rgba = Self::render_lock_bracket_rgba(lock_w, lock_h);
                let png = png::encode_rgba(lock_w, lock_h, &lock_rgba);
                kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_LOCK);
                self.lock_uploaded = true;
                self.last_lock_signature = lock_sig;
            }
            let lock_p = Placement {
                image_id: IMAGE_ID_LOCK,
                placement_id: 0,
                col: base_col.saturating_add(u16::try_from(dc).unwrap_or(0)),
                row: base_row.saturating_add(u16::try_from(dr).unwrap_or(0)),
                cols: lock_cols,
                rows: lock_rows,
                cell_x: 0,
                cell_y: 0,
                src_x: 0,
                src_y: 0,
                src_w: u32::try_from(lock_w).unwrap_or(72),
                src_h: u32::try_from(lock_h).unwrap_or(72),
                z: Z_HUD,
            };
            if self.last_lock_placement != Some(lock_p) {
                kitty::place_bytes(&mut ops, &lock_p);
                self.last_lock_placement = Some(lock_p);
            }
        } else if self.last_lock_placement.take().is_some() {
            kitty::delete_placement_bytes(&mut ops, IMAGE_ID_LOCK, 0);
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

    /// Software fallback (`Frame`) for Sixel terminals or headless tests.
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
        let step = self.anim_tick / 3;
        self.anim_tick = self.anim_tick.wrapping_add(1);

        let needs_rebake = self
            .canvas
            .as_ref()
            .filter(|c| c.vp_w == width && c.vp_h == height)
            .and_then(|c| c.camera_window(pose.x_cm, pose.y_cm))
            .is_none();
        if needs_rebake {
            self.canvas = Some(TerrainCanvas::bake(
                &self.map,
                self.terrain.as_ref(),
                pose.x_cm,
                pose.y_cm,
                width,
                height,
            ));
        }

        let Some(canvas) = &self.canvas else {
            return out;
        };
        let (cam_x, cam_y) = canvas
            .camera_window(pose.x_cm, pose.y_cm)
            .unwrap_or(((width / 4) as u32, (height / 4) as u32));
        let cx0 = cam_x as usize;
        let cy0 = cam_y as usize;

        for r in 0..height {
            for c in 0..width {
                out.put(c, r, canvas.ground_frame.get(cx0 + c, cy0 + r));
            }
        }

        let mut units = self.collect_units(frame, &pose, hull_offset_mdeg, width, height, step);
        units.sort_by_key(|u| (u.kind != SpriteKind::Wreck, u.screen_y));

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
            let sheet_w = 6 * n;
            let top_left_x = u.screen_x - (mip.anchor_x as i32);
            let top_left_y = u.screen_y - (mip.anchor_y as i32);
            for r in 0..n {
                let sy = top_left_y + r as i32;
                if sy < 0 || sy >= height as i32 {
                    continue;
                }
                let src_row = (u.facing as usize * n + r) * sheet_w + u.pose_col * n;
                for c in 0..n {
                    let sx = top_left_x + c as i32;
                    if sx < 0 || sx >= width as i32 {
                        continue;
                    }
                    let code = mip.pixels.get(src_row + c).copied().unwrap_or(0);
                    if let Some(mat) = material_from_code(code) {
                        out.put(sx as usize, sy as usize, material_rgb(mat, u.kind));
                    }
                }
            }
        }

        let _ = look_offset_mdeg;
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

    #[test]
    fn retained_kitty_compositor_scrolls_by_translation_reuses_sheets_and_skips_idle_ticks() {
        let mut pic = Iso25DPicture::new();
        pic.set_map(Map::new(128, 128, 1000));
        let cell = Some(CellSize {
            width: 8,
            height: 16,
        });

        let frame1 = json!({
            "seated": true,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 50_000,
            "y_cm": 50_000,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 0,
            "contacts": [
                {
                    "id": "RA",
                    "class": "red",
                    "unit_type": "tacsim.unit.line_tank",
                    "bearing_deg": 45,
                    "range_m": 40,
                    "facing": "front_left",
                    "moving": false
                }
            ]
        });

        let (_vp, bytes1) = pic.render_kitty_bytes(&frame1, (0, 0), (1, 2, 120, 40), cell, false);
        let s1 = String::from_utf8_lossy(&bytes1);
        assert!(s1.contains("a=t,f=100,t=d,i=100,q=1"));
        assert!(s1.contains("a=p,i=100,x="));
        assert!(!s1.contains(",p="));
        assert!(s1.contains("X=0,Y=0,"));

        // Stationary tick with no changes emits ZERO bytes!
        let (_vp, idle_bytes) =
            pic.render_kitty_bytes(&frame1, (0, 0), (1, 2, 120, 40), cell, false);
        assert!(idle_bytes.is_empty());

        // Moving tick: mech steps north (y_cm = 50_400). Terrain and unit sheets
        // are already uploaded, so steady-state tick emits zero terrain/sheet
        // `a=t` uploads and scrolls via `a=p` source-rect!
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
                    "id": "RA",
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
        assert!(s2.contains("a=p,i=100,x="));
        assert!(!s2.contains(",p="));
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
}
