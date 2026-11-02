//! **Baked 8-facing 3D spaceship/asteroid sprite shader and 6-DOF docking hull wireframes**
//! (doc 35 cut SF.14, D-35-15).
//!
//! Renders spacecraft (`freighter`, `corvette`, `raider`, `drone`, `torpedo`,
//! `derelict`, `shipyard`) and irregular 3D `asteroid` meshes baked from
//! `MSGDI` FBX assets (`scripts/ship-bake/bake.py`) into `crates/mush-pixel/ships/*.png`.
//! Uses O(1) stored-block `IDAT` scanline lookup and 4x4 Bayer ordered dithering
//! over small fixed colour ramps so every frame remains strictly within the
//! 256-colour indexed PNG and DEC sixel limit across native and WASM builds.

use std::sync::OnceLock;

use serde_json::Value;

use crate::Frame;
use crate::docking::{DockingCameraPose, project_collar_point};
use crate::frame::Rgb;
use crate::overlay::line;

const CELL_SIZE: usize = 64;
const FACINGS: usize = 8;
const SHEET_W: usize = CELL_SIZE * FACINGS; // 512
const SHEET_H: usize = CELL_SIZE; // 64
const ROW_STRIDE: usize = 1 + SHEET_W; // 513
const SCANLINE_LEN: usize = SHEET_H * ROW_STRIDE; // 32,832

const IDX_TRANSPARENT: u8 = 0;
const IDX_OUTLINE: u8 = 240;
const IDX_PLUME: u8 = 241;

const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

const BAKED_FREIGHTER_PNG: &[u8] = include_bytes!("../ships/freighter.png");
const BAKED_CORVETTE_PNG: &[u8] = include_bytes!("../ships/corvette.png");
const BAKED_RAIDER_PNG: &[u8] = include_bytes!("../ships/raider.png");
const BAKED_DRONE_PNG: &[u8] = include_bytes!("../ships/drone.png");
const BAKED_TORPEDO_PNG: &[u8] = include_bytes!("../ships/torpedo.png");
const BAKED_ASTEROID_PNG: &[u8] = include_bytes!("../ships/asteroid.png");
const BAKED_DERELICT_PNG: &[u8] = include_bytes!("../ships/derelict.png");
const BAKED_SHIPYARD_PNG: &[u8] = include_bytes!("../ships/shipyard.png");
const BAKED_HALCYON_PNG: &[u8] = include_bytes!("../ships/halcyon.png");
const BAKED_REFINERY_PNG: &[u8] = include_bytes!("../ships/refinery.png");
const SHIP_INDEX_JSON: &str = include_str!("../ships/index.json");

const OUTLINE_RGB: Rgb = [14, 18, 26];
const PLUME_CORE_RGB: Rgb = [180, 232, 255];
const PLUME_IDLE_RGB: Rgb = [72, 148, 236];

const FREIGHTER_RAMP: [Rgb; 6] = [
    [24, 30, 38],
    [52, 64, 80],
    [92, 110, 132],
    [140, 162, 188],
    [188, 208, 228],
    [232, 242, 252],
];

const CORVETTE_RAMP: [Rgb; 6] = [
    [20, 32, 44],
    [44, 72, 98],
    [78, 124, 164],
    [120, 174, 214],
    [172, 214, 242],
    [224, 244, 255],
];

const RAIDER_RAMP: [Rgb; 6] = [
    [38, 20, 20],
    [78, 38, 36],
    [128, 62, 56],
    [176, 92, 80],
    [216, 136, 116],
    [246, 196, 176],
];

const DRONE_RAMP: [Rgb; 6] = [
    [36, 28, 16],
    [80, 62, 32],
    [136, 108, 52],
    [188, 152, 76],
    [224, 192, 112],
    [248, 228, 168],
];

const TORPEDO_RAMP: [Rgb; 6] = [
    [44, 18, 16],
    [96, 36, 32],
    [156, 58, 48],
    [208, 92, 72],
    [240, 148, 116],
    [255, 216, 192],
];

const ASTEROID_RAMP: [Rgb; 6] = [
    [26, 24, 22],
    [58, 54, 48],
    [98, 92, 82],
    [142, 134, 120],
    [184, 176, 160],
    [224, 216, 202],
];

const DERELICT_RAMP: [Rgb; 6] = [
    [22, 24, 28],
    [48, 52, 60],
    [82, 88, 100],
    [120, 128, 142],
    [160, 168, 182],
    [204, 212, 224],
];

const HALCYON_RAMP: [Rgb; 6] = [
    [22, 34, 48],
    [48, 76, 108],
    [88, 132, 178],
    [136, 184, 226],
    [186, 220, 246],
    [234, 248, 255],
];

const REFINERY_RAMP: [Rgb; 6] = [
    [32, 26, 18],
    [68, 56, 38],
    [114, 94, 62],
    [162, 136, 92],
    [206, 178, 128],
    [242, 222, 182],
];

/// Canonical baked 3D spaceship or space-object archetype.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShipModelStyle {
    /// Dray-P / Class II industrial cargo freighter (`freighter.png`).
    Freighter,
    /// Republic Customs Corvette / Hammerhead patrol frigate (`corvette.png`).
    Corvette,
    /// Belt Raider / Corsair heavy pirate frigate (`raider.png`).
    Raider,
    /// Mining collector drone / skiff (`drone.png`).
    Drone,
    /// Heavy space torpedo (`torpedo.png`).
    Torpedo,
    /// Irregular faceted 3D asteroid (`asteroid.png`).
    Asteroid,
    /// Shattered derelict light destroyer (`derelict.png`).
    Derelict,
    /// Orbital frigate shipyard / docking berth (`shipyard.png`).
    Shipyard,
    /// Halcyon-class deep-space explorer / carrier (`halcyon.png`, `SF.20`).
    Halcyon,
    /// Orbital resource refinery / processing outpost (`refinery.png`, `SF.20`).
    Refinery,
}

impl ShipModelStyle {
    /// Select the ship model archetype from contact `kind`, `relation`, and `id`/`label`.
    #[must_use]
    pub fn from_contact(kind: &str, relation: &str, id: &str, label: &str) -> ShipModelStyle {
        let lower_id = id.to_ascii_lowercase();
        let lower_label = label.to_ascii_lowercase();
        if kind == "asteroid" || kind == "rock" {
            return ShipModelStyle::Asteroid;
        }
        if kind == "torpedo" || kind == "missile" {
            return ShipModelStyle::Torpedo;
        }
        if kind == "drone" || lower_id.contains("drone") || lower_label.contains("drone") {
            return ShipModelStyle::Drone;
        }
        if kind == "derelict"
            || kind == "disabled_derelict"
            || lower_id.contains("derelict")
            || lower_label.contains("derelict")
        {
            return ShipModelStyle::Derelict;
        }
        if kind == "refinery"
            || lower_id.contains("refinery")
            || lower_id.contains("outpost")
            || lower_label.contains("refinery")
            || lower_label.contains("outpost")
        {
            return ShipModelStyle::Refinery;
        }
        if kind == "station"
            || kind == "port_berth"
            || kind == "shipyard"
            || lower_id.contains("station")
            || lower_id.contains("port")
        {
            return ShipModelStyle::Shipyard;
        }
        if kind == "halcyon"
            || kind == "carrier"
            || lower_id.contains("halcyon")
            || lower_id.contains("carrier")
            || lower_label.contains("halcyon")
            || lower_label.contains("carrier")
        {
            return ShipModelStyle::Halcyon;
        }
        if lower_id.contains("dray")
            || lower_id.contains("hauler")
            || lower_id.contains("freighter")
            || lower_label.contains("dray")
            || lower_label.contains("hauler")
        {
            return ShipModelStyle::Freighter;
        }
        if relation == "hostile"
            || lower_id.contains("raider")
            || lower_id.contains("pirate")
            || lower_id.contains("kestrel")
            || lower_label.contains("raider")
            || lower_label.contains("kestrel")
        {
            return ShipModelStyle::Raider;
        }
        ShipModelStyle::Corvette
    }

    fn png_bytes(self) -> &'static [u8] {
        match self {
            ShipModelStyle::Freighter => BAKED_FREIGHTER_PNG,
            ShipModelStyle::Corvette => BAKED_CORVETTE_PNG,
            ShipModelStyle::Raider => BAKED_RAIDER_PNG,
            ShipModelStyle::Drone => BAKED_DRONE_PNG,
            ShipModelStyle::Torpedo => BAKED_TORPEDO_PNG,
            ShipModelStyle::Asteroid => BAKED_ASTEROID_PNG,
            ShipModelStyle::Derelict => BAKED_DERELICT_PNG,
            ShipModelStyle::Shipyard => BAKED_SHIPYARD_PNG,
            ShipModelStyle::Halcyon => BAKED_HALCYON_PNG,
            ShipModelStyle::Refinery => BAKED_REFINERY_PNG,
        }
    }

    fn ramp(self) -> &'static [Rgb; 6] {
        match self {
            ShipModelStyle::Freighter | ShipModelStyle::Shipyard => &FREIGHTER_RAMP,
            ShipModelStyle::Corvette => &CORVETTE_RAMP,
            ShipModelStyle::Raider => &RAIDER_RAMP,
            ShipModelStyle::Drone => &DRONE_RAMP,
            ShipModelStyle::Torpedo => &TORPEDO_RAMP,
            ShipModelStyle::Asteroid => &ASTEROID_RAMP,
            ShipModelStyle::Derelict => &DERELICT_RAMP,
            ShipModelStyle::Halcyon => &HALCYON_RAMP,
            ShipModelStyle::Refinery => &REFINERY_RAMP,
        }
    }
}

/// Parameters for drawing a baked 3D ship or asteroid sprite onto a [`Frame`].
#[derive(Clone, Copy, Debug)]
pub struct ShipSpriteDraw {
    /// Screen-space center `(cx, cy)` in pixels.
    pub center_px: (i64, i64),
    /// Half-width/height of the rendered sprite in pixels (`4..=64`).
    pub half_size_px: i64,
    /// Yaw orientation in degrees (`0.0..360.0`), quantized to one of the 8 baked facings.
    pub yaw_deg: f64,
    /// Visual hull/object archetype.
    pub style: ShipModelStyle,
    /// Whether the ship's main fusion drive is actively burning.
    pub burning: bool,
}

fn bayer_threshold(x: i64, y: i64) -> f64 {
    let row = BAYER_4X4
        .get(y.rem_euclid(4) as usize)
        .copied()
        .unwrap_or([0; 4]);
    let val = row.get(x.rem_euclid(4) as usize).copied().unwrap_or(0);
    (f64::from(val) + 0.5) / 16.0
}

fn sample_ramp<const N: usize>(ramp: &[Rgb; N], level: f64, x: i64, y: i64) -> Rgb {
    let max_idx = (N.saturating_sub(1)) as f64;
    let scaled = (level.clamp(0.0, 1.0) * max_idx).max(0.0);
    let base = scaled.floor();
    let frac = scaled - base;
    let idx = (base as usize).saturating_add(usize::from(frac > bayer_threshold(x, y)));
    ramp.get(idx.min(N.saturating_sub(1)))
        .copied()
        .unwrap_or([0, 0, 0])
}

fn stored_sheet_scanlines(png: &[u8]) -> Option<&[u8]> {
    let mut at = 8usize;
    while let Some(head) = png.get(at..at + 8) {
        let b0 = u32::from(head.first().copied().unwrap_or(0));
        let b1 = u32::from(head.get(1).copied().unwrap_or(0));
        let b2 = u32::from(head.get(2).copied().unwrap_or(0));
        let b3 = u32::from(head.get(3).copied().unwrap_or(0));
        let len = ((b0 << 24) | (b1 << 16) | (b2 << 8) | b3) as usize;
        let kind = head.get(4..8).unwrap_or(&[]);
        let data = png.get(at + 8..(at + 8).saturating_add(len))?;
        if kind == b"IDAT" {
            return data.get(7..7 + SCANLINE_LEN);
        }
        at = at.saturating_add(12).saturating_add(len);
    }
    None
}

/// Render a baked 8-facing 3D ship or asteroid sprite onto `frame`.
pub fn draw_ship_sprite(frame: &mut Frame, spec: &ShipSpriteDraw) {
    let half = spec.half_size_px.clamp(3, 64);
    let diam = (half * 2).max(1);
    let (cx, cy) = spec.center_px;
    let (w, h) = (frame.width() as i64, frame.height() as i64);

    let x_min = (cx - half).max(0);
    let x_max = (cx + half - 1).min(w - 1);
    let y_min = (cy - half).max(0);
    let y_max = (cy + half - 1).min(h - 1);
    if x_min > x_max || y_min > y_max {
        return;
    }

    let Some(scanlines) = stored_sheet_scanlines(spec.style.png_bytes()) else {
        return;
    };
    let ramp = spec.style.ramp();
    let facing_idx = (((spec.yaw_deg.rem_euclid(360.0) + 22.5) / 45.0).floor() as usize) % FACINGS;
    let cell_x0 = facing_idx * CELL_SIZE;

    for py in y_min..=y_max {
        let ty = (((py - (cy - half)) * (CELL_SIZE as i64)) / diam).clamp(0, (CELL_SIZE as i64) - 1)
            as usize;
        let row_base = ty * ROW_STRIDE + 1 + cell_x0;
        for px in x_min..=x_max {
            let tx = (((px - (cx - half)) * (CELL_SIZE as i64)) / diam)
                .clamp(0, (CELL_SIZE as i64) - 1) as usize;
            let Some(code) = scanlines.get(row_base + tx).copied() else {
                continue;
            };
            if code == IDX_TRANSPARENT {
                continue;
            }
            let col = if code == IDX_OUTLINE {
                OUTLINE_RGB
            } else if code == IDX_PLUME {
                if spec.burning {
                    PLUME_CORE_RGB
                } else {
                    PLUME_IDLE_RGB
                }
            } else {
                let shade = f64::from(code.saturating_sub(1)) / 238.0;
                sample_ramp(ramp, shade, px, py)
            };
            frame.put(px as usize, py as usize, col);
        }
    }
}

type EdgeMm = ([f64; 3], [f64; 3]);

struct DockingWireframeCatalog {
    freighter: Vec<EdgeMm>,
    corvette: Vec<EdgeMm>,
    derelict: Vec<EdgeMm>,
    shipyard: Vec<EdgeMm>,
    halcyon: Vec<EdgeMm>,
    refinery: Vec<EdgeMm>,
}

fn docking_wireframes() -> &'static DockingWireframeCatalog {
    static CATALOG: OnceLock<DockingWireframeCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let doc: Value = serde_json::from_str(SHIP_INDEX_JSON).unwrap_or_default();
        let parse_for = |target_id: &str| -> Vec<EdgeMm> {
            let Some(ships) = doc.get("ships").and_then(Value::as_array) else {
                return Vec::new();
            };
            let Some(entry) = ships
                .iter()
                .find(|s| s.get("id").and_then(Value::as_str) == Some(target_id))
            else {
                return Vec::new();
            };
            let Some(edges) = entry.get("docking_wireframe_mm").and_then(Value::as_array) else {
                return Vec::new();
            };
            let mut out = Vec::with_capacity(edges.len());
            for seg in edges {
                let Some(pair) = seg.as_array() else {
                    continue;
                };
                let (Some(p0), Some(p1)) = (
                    pair.first().and_then(Value::as_array),
                    pair.get(1).and_then(Value::as_array),
                ) else {
                    continue;
                };
                let coord = |arr: &[Value], k: usize| -> f64 {
                    arr.get(k).and_then(Value::as_f64).unwrap_or(0.0)
                };
                out.push((
                    [coord(p0, 0), coord(p0, 1), coord(p0, 2)],
                    [coord(p1, 0), coord(p1, 1), coord(p1, 2)],
                ));
            }
            out
        };
        DockingWireframeCatalog {
            freighter: parse_for("freighter"),
            corvette: parse_for("corvette"),
            derelict: parse_for("derelict"),
            shipyard: parse_for("shipyard"),
            halcyon: parse_for("halcyon"),
            refinery: parse_for("refinery"),
        }
    })
}

/// Project and draw the baked 3D FBX feature-edge wireframe of the target hull
/// behind the docking collar in the 60 fps 6-DOF Docking Camera (`docking.rs`).
pub fn draw_docking_hull_wireframe(
    frame: &mut Frame,
    target_kind: &str,
    pose: &DockingCameraPose,
    fov_deg: f64,
    col: Rgb,
) {
    let cat = docking_wireframes();
    let edges = match target_kind {
        "disabled_derelict" | "derelict" | "wreck" => &cat.derelict,
        "refinery" | "outpost" => &cat.refinery,
        "halcyon" | "carrier" => &cat.halcyon,
        "port_berth" | "station" | "shipyard" => &cat.shipyard,
        "freighter" | "hauler" | "dray" => &cat.freighter,
        _ => &cat.corvette,
    };
    let dims = (frame.width() as i64, frame.height() as i64);
    for &(p0, p1) in edges {
        if let (Some(s0), Some(s1)) = (
            project_collar_point(dims, pose, (p0[0], p0[1], p0[2]), fov_deg),
            project_collar_point(dims, pose, (p1[0], p1[1], p1[2]), fov_deg),
        ) {
            line(frame, s0.0, s0.1, s1.0, s1.1, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_ship_sheets_match_index_provenance_and_render_all_8_facings_within_palette() {
        let catalog: [(&str, &[u8], ShipModelStyle); 10] = [
            ("freighter", BAKED_FREIGHTER_PNG, ShipModelStyle::Freighter),
            ("corvette", BAKED_CORVETTE_PNG, ShipModelStyle::Corvette),
            ("raider", BAKED_RAIDER_PNG, ShipModelStyle::Raider),
            ("drone", BAKED_DRONE_PNG, ShipModelStyle::Drone),
            ("torpedo", BAKED_TORPEDO_PNG, ShipModelStyle::Torpedo),
            ("asteroid", BAKED_ASTEROID_PNG, ShipModelStyle::Asteroid),
            ("derelict", BAKED_DERELICT_PNG, ShipModelStyle::Derelict),
            ("shipyard", BAKED_SHIPYARD_PNG, ShipModelStyle::Shipyard),
            ("halcyon", BAKED_HALCYON_PNG, ShipModelStyle::Halcyon),
            ("refinery", BAKED_REFINERY_PNG, ShipModelStyle::Refinery),
        ];

        let index_json: Value = serde_json::from_str(SHIP_INDEX_JSON).unwrap_or_default();
        let entries = index_json
            .get("ships")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(entries.len(), 10);

        for (id, png_bytes, style) in catalog {
            let scanlines = stored_sheet_scanlines(png_bytes);
            assert!(
                scanlines.is_some(),
                "baked {id}.png must carry a contiguous stored IDAT block"
            );
            assert_eq!(scanlines.map(<[u8]>::len), Some(SCANLINE_LEN));

            #[cfg(feature = "encode")]
            {
                use sha2::{Digest as _, Sha256};
                let (w, h, rgba) = crate::png::decode_rgba(png_bytes).unwrap_or((0, 0, Vec::new()));
                assert_eq!((w, h), (SHEET_W, SHEET_H), "{id}.png sheet dimensions");
                assert_eq!(rgba.len(), SHEET_W * SHEET_H * 4);

                let digest: String = Sha256::digest(png_bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let recorded = entries
                    .iter()
                    .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
                    .and_then(|e| e.get("sha256"))
                    .and_then(Value::as_str);
                assert_eq!(recorded, Some(digest.as_str()), "{id}.png sha256 mismatch");
            }

            let mut f0 = Frame::new(128, 128);
            let mut f90 = Frame::new(128, 128);
            draw_ship_sprite(
                &mut f0,
                &ShipSpriteDraw {
                    center_px: (64, 64),
                    half_size_px: 28,
                    yaw_deg: 0.0,
                    style,
                    burning: true,
                },
            );
            draw_ship_sprite(
                &mut f90,
                &ShipSpriteDraw {
                    center_px: (64, 64),
                    half_size_px: 28,
                    yaw_deg: 90.0,
                    style,
                    burning: true,
                },
            );
            assert_ne!(
                f0.hash(),
                f90.hash(),
                "{id} 3D silhouette must rotate across facings"
            );
            assert!(
                f0.count_colours(256) <= 12,
                "{id} must remain within small fixed palette ramp"
            );
        }
    }
}
