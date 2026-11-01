//! **Analytical 3D sphere and ring raycaster for solar-system bodies** (doc 35
//! cut SF.2, D-35-15).
//!
//! Renders planets, moons, and ring systems into a [`Frame`] using per-pixel
//! ray-sphere and ray-plane intersection, solar phase illumination, and 4x4
//! Bayer ordered dithering over a small fixed palette so every frame remains
//! strictly within the 256-colour indexed PNG and DEC sixel limit.

use crate::Frame;
use crate::frame::Rgb;
use crate::math::Det as _;

/// 4x4 Bayer ordered dither thresholds in `0..16`.
const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Visual archetype for a solar-system body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanetStyle {
    /// Deep cobalt-azure methane ice giant with zonal bands, Great Dark Spot,
    /// bright high-altitude cirrus streaks, and faint ring arcs.
    Neptune,
    /// Pale cyan-aquamarine ice giant with high axial tilt and faint rings.
    Uranus,
    /// Golden-ochre gas giant with prominent A/B/C rings and Cassini Division.
    Saturn,
    /// Banded ammonia-cloud gas giant with the Great Red Spot.
    Jupiter,
    /// Rust-ochre terrestrial planet with basalt basins, canyon scar, and polar cap.
    Mars,
    /// Blue-ocean and cloud-streaked terrestrial planet.
    Earth,
    /// Dense amber-orange nitrogen haze moon (Titan).
    Titan,
    /// Retrograde pinkish-nitrogen ice moon with dark geyser streaks (Triton).
    Triton,
    /// Volcanic sulfur-yellow and orange moon (Io).
    Io,
    /// Cracked icy lithosphere moon (Europa).
    Europa,
    /// Cratered grey-albedo dwarf planet or asteroid (Ceres, Vesta, Pallas).
    Rocky,
    /// Cratered lunar maria and highlands (Luna, Ganymede, Callisto, Mercury).
    Luna,
    /// Brilliant solar disc.
    Sun,
}

impl PlanetStyle {
    /// Select the shader style from a canonical body id.
    #[must_use]
    pub fn from_id(id: &str) -> PlanetStyle {
        match id {
            "neptune" => PlanetStyle::Neptune,
            "uranus" => PlanetStyle::Uranus,
            "saturn" => PlanetStyle::Saturn,
            "jupiter" => PlanetStyle::Jupiter,
            "mars" => PlanetStyle::Mars,
            "earth" => PlanetStyle::Earth,
            "titan" => PlanetStyle::Titan,
            "triton" => PlanetStyle::Triton,
            "io" => PlanetStyle::Io,
            "europa" => PlanetStyle::Europa,
            "luna" | "moon" | "ganymede" | "callisto" | "mercury" => PlanetStyle::Luna,
            "sun" | "sol" => PlanetStyle::Sun,
            _ => PlanetStyle::Rocky,
        }
    }
}

/// Parameters for rendering a celestial body on the canopy viewport.
#[derive(Clone, Copy, Debug)]
pub struct CelestialBodyDraw {
    /// Screen-space centre `(cx, cy)` in pixels.
    pub center_px: (i64, i64),
    /// Apparent disc radius in pixels.
    pub radius_px: i64,
    /// Visual style of the body.
    pub style: PlanetStyle,
    /// Normalized direction toward the Sun in camera space `(lx, ly, lz)`.
    pub sun_dir: (f64, f64, f64),
    /// Rotation phase in degrees (`0.0..360.0`) for surface features.
    pub spin_deg: f64,
    /// Ring plane tilt in degrees (`0.0` = edge-on, `25.0` = classic Saturn view).
    pub ring_tilt_deg: f64,
}

// Fixed 6-step colour ramps per planet archetype (dark shadow -> bright lit).
const NEPTUNE_RAMP: [Rgb; 6] = [
    [8, 16, 48],
    [16, 40, 108],
    [24, 72, 168],
    [36, 112, 216],
    [64, 156, 244],
    [140, 212, 255],
];

const URANUS_RAMP: [Rgb; 6] = [
    [12, 32, 44],
    [28, 76, 96],
    [56, 132, 156],
    [88, 180, 200],
    [136, 216, 228],
    [192, 240, 248],
];

const SATURN_RAMP: [Rgb; 6] = [
    [32, 24, 14],
    [76, 58, 32],
    [132, 104, 58],
    [184, 148, 88],
    [224, 188, 120],
    [248, 224, 168],
];

const JUPITER_RAMP: [Rgb; 6] = [
    [36, 22, 16],
    [88, 52, 36],
    [148, 92, 60],
    [196, 136, 92],
    [228, 184, 140],
    [248, 228, 196],
];

const JUPITER_SPOT: Rgb = [196, 68, 44];

const MARS_RAMP: [Rgb; 6] = [
    [36, 14, 10],
    [84, 32, 20],
    [140, 56, 32],
    [188, 84, 48],
    [220, 124, 76],
    [244, 188, 148],
];

const EARTH_RAMP: [Rgb; 6] = [
    [10, 24, 56],
    [20, 56, 124],
    [36, 104, 188],
    [56, 148, 112],
    [164, 204, 236],
    [240, 248, 255],
];

const TITAN_RAMP: [Rgb; 6] = [
    [36, 20, 8],
    [84, 48, 16],
    [144, 84, 24],
    [196, 124, 36],
    [232, 164, 64],
    [120, 196, 232],
];

const TRITON_RAMP: [Rgb; 6] = [
    [28, 24, 32],
    [68, 56, 72],
    [116, 96, 116],
    [168, 144, 160],
    [212, 188, 200],
    [244, 228, 236],
];

const IO_RAMP: [Rgb; 6] = [
    [44, 24, 8],
    [108, 60, 16],
    [176, 104, 24],
    [220, 168, 36],
    [244, 216, 64],
    [252, 244, 148],
];

const EUROPA_RAMP: [Rgb; 6] = [
    [24, 32, 44],
    [64, 80, 100],
    [116, 136, 156],
    [168, 188, 204],
    [212, 224, 236],
    [244, 250, 255],
];

const ROCKY_RAMP: [Rgb; 6] = [
    [20, 22, 26],
    [52, 56, 64],
    [92, 98, 108],
    [136, 142, 152],
    [180, 186, 196],
    [228, 232, 240],
];

const RING_LIT: [Rgb; 4] = [
    [84, 72, 52],
    [140, 120, 84],
    [196, 172, 124],
    [232, 212, 164],
];

const ICE_RING_LIT: [Rgb; 4] = [[24, 52, 84], [48, 96, 144], [84, 148, 204], [140, 204, 244]];

// Baked 128x64 8-bit indexed-colour equirectangular planetary maps (`scripts/planet-bake/bake.py`).
const BAKED_W: usize = 128;
const BAKED_H: usize = 64;
const BAKED_ROW_STRIDE: usize = 1 + BAKED_W;
const BAKED_SCANLINE_LEN: usize = BAKED_H * BAKED_ROW_STRIDE;

const FEAT_JUPITER_SPOT: u8 = 240;
const FEAT_MARS_ICE: u8 = 241;
const FEAT_CERES_FACULA: u8 = 242;
const FEAT_EARTH_CLOUD: u8 = 243;

const BAKED_NEPTUNE_PNG: &[u8] = include_bytes!("../planets/neptune.png");
const BAKED_URANUS_PNG: &[u8] = include_bytes!("../planets/uranus.png");
const BAKED_SATURN_PNG: &[u8] = include_bytes!("../planets/saturn.png");
const BAKED_JUPITER_PNG: &[u8] = include_bytes!("../planets/jupiter.png");
const BAKED_MARS_PNG: &[u8] = include_bytes!("../planets/mars.png");
const BAKED_EARTH_PNG: &[u8] = include_bytes!("../planets/earth.png");
const BAKED_TITAN_PNG: &[u8] = include_bytes!("../planets/titan.png");
const BAKED_TRITON_PNG: &[u8] = include_bytes!("../planets/triton.png");
const BAKED_IO_PNG: &[u8] = include_bytes!("../planets/io.png");
const BAKED_EUROPA_PNG: &[u8] = include_bytes!("../planets/europa.png");
const BAKED_CERES_PNG: &[u8] = include_bytes!("../planets/ceres.png");
const BAKED_LUNA_PNG: &[u8] = include_bytes!("../planets/luna.png");

/// Extract the contiguous `64 * 129`-byte stored DEFLATE scanline payload from
/// a baked `128x64` indexed-colour PNG without heap allocation or `flate2`.
fn stored_idat_scanlines(png: &[u8]) -> Option<&[u8]> {
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
            // 2-byte zlib header + 5-byte RFC 1951 stored-block header (`BTYPE=00`)
            return data.get(7..7 + BAKED_SCANLINE_LEN);
        }
        at = at.saturating_add(12).saturating_add(len);
    }
    None
}

/// Sample a baked `128x64` equirectangular PNG at spherical coordinates
/// `(lon_rad, ny)` where `ny` is in `-1.0..=1.0` (`-1.0` = north pole).
fn sample_baked_equirect(png: &[u8], lon_rad: f64, ny: f64) -> Option<u8> {
    let scanlines = stored_idat_scanlines(png)?;
    let clamped_y = ny.clamp(-1.0, 1.0);
    let lat_rad = clamped_y.det_atan2((1.0 - clamped_y * clamped_y).max(0.0).sqrt());
    let u = (lon_rad / core::f64::consts::TAU).rem_euclid(1.0);
    let v = (lat_rad / core::f64::consts::PI + 0.5).clamp(0.0, 0.999_999);
    let col = ((u * (BAKED_W as f64)) as usize).min(BAKED_W - 1);
    let row = ((v * (BAKED_H as f64)) as usize).min(BAKED_H - 1);
    scanlines.get(row * BAKED_ROW_STRIDE + 1 + col).copied()
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

fn normalize3((x, y, z): (f64, f64, f64)) -> (f64, f64, f64) {
    let len = (x * x + y * y + z * z).sqrt();
    if len <= 1e-9 {
        (0.577, -0.577, 0.577)
    } else {
        (x / len, y / len, z / len)
    }
}

/// Render a celestial body (and its ring system, if any) onto `frame`.
pub fn draw_celestial_body(frame: &mut Frame, spec: &CelestialBodyDraw) {
    let r = spec.radius_px;
    if r <= 0 {
        return;
    }
    let (cx, cy) = spec.center_px;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let has_rings = matches!(
        spec.style,
        PlanetStyle::Saturn | PlanetStyle::Uranus | PlanetStyle::Neptune
    );
    let ring_outer_ratio = match spec.style {
        PlanetStyle::Saturn => 2.35,
        PlanetStyle::Uranus => 1.75,
        PlanetStyle::Neptune => 1.60,
        _ => 1.0,
    };
    let bound = if has_rings {
        ((r as f64) * ring_outer_ratio).ceil() as i64 + 2
    } else {
        r + 2
    };
    let x_min = (cx - bound).max(0);
    let x_max = (cx + bound).min(w - 1);
    let y_min = (cy - bound).max(0);
    let y_max = (cy + bound).min(h - 1);
    if x_min > x_max || y_min > y_max {
        return;
    }

    let (lx, ly, lz) = normalize3(spec.sun_dir);
    let inv_r = 1.0 / (r as f64);
    let tilt_rad = spec.ring_tilt_deg.to_radians();
    let (sin_tilt, cos_tilt) = tilt_rad.det_sin_cos();
    let spin_rad = spec.spin_deg.to_radians();

    for py in y_min..=y_max {
        let ny = (py - cy) as f64 * inv_r;
        for px in x_min..=x_max {
            let nx = (px - cx) as f64 * inv_r;
            let r2 = nx * nx + ny * ny;

            // Check ring intersection in front of or behind the sphere.
            let ring_hit = if has_rings && sin_tilt.abs() > 0.04 {
                let rz = -ny / sin_tilt;
                let rx = nx;
                let ry_plane = ny * cos_tilt / sin_tilt;
                let ring_r = (rx * rx + ry_plane * ry_plane).sqrt();
                ring_sample(spec.style, ring_r, (rx, ry_plane), (lx, ly), (px, py)).map(|c| (rz, c))
            } else {
                None
            };

            if r2 <= 1.0 {
                // Ray hits the planet's sphere at z = +sqrt(1 - r2).
                let nz = (1.0 - r2).sqrt();
                // If the ring plane is in front of the sphere at this pixel (rz > nz), draw ring.
                if let Some((rz, ring_col)) = ring_hit
                    && rz > nz
                {
                    frame.put(px as usize, py as usize, ring_col);
                    continue;
                }
                let col = shade_sphere_pixel(
                    spec.style,
                    (nx, ny, nz),
                    (lx, ly, lz),
                    (sin_tilt, cos_tilt),
                    spin_rad,
                    has_rings,
                    (px, py),
                );
                frame.put(px as usize, py as usize, col);
            } else if let Some((_rz, ring_col)) = ring_hit {
                frame.put(px as usize, py as usize, ring_col);
            }
        }
    }
}

fn ring_sample(
    style: PlanetStyle,
    ring_r: f64,
    (rx, ry_plane): (f64, f64),
    (lx, ly): (f64, f64),
    (px, py): (i64, i64),
) -> Option<Rgb> {
    match style {
        PlanetStyle::Saturn => {
            // C ring: 1.23..1.52, B ring: 1.52..1.95, Cassini gap: 1.95..2.03, A ring: 2.03..2.28
            if !(1.23..=2.28).contains(&ring_r) || (1.95..=2.03).contains(&ring_r) {
                return None;
            }
            let density = if (1.52..1.95).contains(&ring_r) {
                // B ring: dense with fine concentric ringlets
                0.78 + 0.18 * (ring_r * 42.0).det_sin()
            } else if ring_r >= 2.03 {
                // A ring
                0.58 + 0.12 * (ring_r * 55.0).det_cos()
            } else {
                // C crêpe ring (semi-transparent via Bayer dither)
                if bayer_threshold(px, py) > 0.45 {
                    return None;
                }
                0.32
            };
            // Planet shadow on the far side of the ring
            let dot_sun = rx * lx + ry_plane * ly;
            let cross_sq = (rx * rx + ry_plane * ry_plane) - dot_sun * dot_sun;
            let in_planet_shadow = dot_sun < 0.0 && cross_sq < 0.92;
            let lit = if in_planet_shadow {
                density * 0.15
            } else {
                density
            };
            Some(sample_ramp(&RING_LIT, lit, px, py))
        }
        PlanetStyle::Uranus => {
            if !(1.45..=1.72).contains(&ring_r) {
                return None;
            }
            let ringlet = ((ring_r - 1.45) * 38.0).det_sin();
            if ringlet < 0.1 || bayer_threshold(px, py) > 0.65 {
                return None;
            }
            Some(sample_ramp(&ICE_RING_LIT, 0.55 + 0.25 * ringlet, px, py))
        }
        PlanetStyle::Neptune => {
            // Faint Adams & Le Verrier ring arcs
            if !(1.38..=1.58).contains(&ring_r) {
                return None;
            }
            let angle = ry_plane.det_atan2(rx);
            let arc = (angle * 3.0).det_cos();
            if arc < 0.15 || bayer_threshold(px, py) > 0.55 {
                return None;
            }
            Some(sample_ramp(&ICE_RING_LIT, 0.45 + 0.30 * arc, px, py))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn shade_sphere_pixel(
    style: PlanetStyle,
    (nx, ny, nz): (f64, f64, f64),
    (lx, ly, lz): (f64, f64, f64),
    (sin_tilt, cos_tilt): (f64, f64),
    spin_rad: f64,
    has_rings: bool,
    (px, py): (i64, i64),
) -> Rgb {
    if style == PlanetStyle::Sun {
        let limb = nz.sqrt();
        return sample_ramp(&IO_RAMP, (0.65 + 0.35 * limb).clamp(0.0, 1.0), px, py);
    }

    // Diffuse solar illumination + ambient floor
    let ndotl = (nx * lx + ny * ly + nz * lz).max(0.0);
    let mut light = 0.08 + 0.92 * ndotl;

    // Ring shadow cast onto Saturn's cloud tops
    if has_rings && style == PlanetStyle::Saturn && ly.abs() > 0.05 {
        let t_ring = -ny / (ly * sin_tilt + 0.001);
        if t_ring > 0.0 {
            let rx = nx + lx * t_ring * 0.35;
            let rz = nz + lz * t_ring * 0.35;
            let r_ring = (rx * rx + (rz * cos_tilt) * (rz * cos_tilt)).sqrt();
            if (1.52..=2.25).contains(&r_ring) && !(1.95..=2.03).contains(&r_ring) {
                light *= 0.48;
            }
        }
    }

    // Spherical longitude & latitude for baked equirectangular surface lookup
    let lon = nz.det_atan2(nx) + spin_rad;
    let lat = ny; // -1.0 (north pole) .. +1.0 (south pole)
    let limb_glow = (1.0 - nz).max(0.0);

    let baked_png = match style {
        PlanetStyle::Neptune => BAKED_NEPTUNE_PNG,
        PlanetStyle::Uranus => BAKED_URANUS_PNG,
        PlanetStyle::Saturn => BAKED_SATURN_PNG,
        PlanetStyle::Jupiter => BAKED_JUPITER_PNG,
        PlanetStyle::Mars => BAKED_MARS_PNG,
        PlanetStyle::Earth => BAKED_EARTH_PNG,
        PlanetStyle::Titan => BAKED_TITAN_PNG,
        PlanetStyle::Triton => BAKED_TRITON_PNG,
        PlanetStyle::Io => BAKED_IO_PNG,
        PlanetStyle::Europa => BAKED_EUROPA_PNG,
        PlanetStyle::Luna => BAKED_LUNA_PNG,
        PlanetStyle::Rocky | PlanetStyle::Sun => BAKED_CERES_PNG,
    };
    let raw_idx = sample_baked_equirect(baked_png, lon, lat).unwrap_or(140);
    let albedo = if raw_idx <= 239 {
        f64::from(raw_idx) / 239.0
    } else {
        1.0
    };
    let tex_mod = 0.54 + 0.52 * albedo;

    match style {
        PlanetStyle::Neptune => {
            let mut level = (light * tex_mod).clamp(0.0, 1.0);
            if limb_glow > 0.55 && ndotl > 0.15 {
                level = (level + 0.22 * (limb_glow - 0.55)).min(1.0);
            }
            sample_ramp(&NEPTUNE_RAMP, level, px, py)
        }
        PlanetStyle::Uranus => {
            let level = (light * (0.68 + 0.34 * albedo) + 0.12 * limb_glow * ndotl).clamp(0.0, 1.0);
            sample_ramp(&URANUS_RAMP, level, px, py)
        }
        PlanetStyle::Saturn => {
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&SATURN_RAMP, level, px, py)
        }
        PlanetStyle::Jupiter => {
            if raw_idx == FEAT_JUPITER_SPOT && ndotl > 0.12 {
                return JUPITER_SPOT;
            }
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&JUPITER_RAMP, level, px, py)
        }
        PlanetStyle::Mars => {
            if (raw_idx == FEAT_MARS_ICE || lat.abs() > 0.86) && ndotl > 0.10 {
                return [240, 244, 248];
            }
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&MARS_RAMP, level, px, py)
        }
        PlanetStyle::Earth => {
            let boost = if raw_idx == FEAT_EARTH_CLOUD {
                0.22 * ndotl
            } else {
                0.0
            };
            let level = (light * tex_mod + boost + 0.14 * limb_glow * ndotl).clamp(0.0, 1.0);
            sample_ramp(&EARTH_RAMP, level, px, py)
        }
        PlanetStyle::Titan => {
            // Orange nitrogen haze with detached high-altitude cyan limb ring
            if limb_glow > 0.68 && ndotl > 0.1 {
                return sample_ramp(&TITAN_RAMP, 0.95, px, py);
            }
            sample_ramp(
                &TITAN_RAMP,
                (light * (0.58 + 0.26 * albedo)).clamp(0.0, 0.82),
                px,
                py,
            )
        }
        PlanetStyle::Triton => {
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&TRITON_RAMP, level, px, py)
        }
        PlanetStyle::Io => {
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&IO_RAMP, level, px, py)
        }
        PlanetStyle::Europa => {
            let level = (light * (0.60 + 0.44 * albedo)).clamp(0.0, 1.0);
            sample_ramp(&EUROPA_RAMP, level, px, py)
        }
        PlanetStyle::Luna => {
            let level = (light * tex_mod).clamp(0.0, 1.0);
            sample_ramp(&ROCKY_RAMP, level, px, py)
        }
        PlanetStyle::Rocky | PlanetStyle::Sun => {
            let facula = if raw_idx == FEAT_CERES_FACULA {
                0.28 * ndotl
            } else {
                0.0
            };
            let level = (light * tex_mod + facula).clamp(0.0, 1.0);
            sample_ramp(&ROCKY_RAMP, level, px, py)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neptune_and_saturn_stay_within_256_colour_palette_and_draw_rings() {
        let mut frame = Frame::new(320, 240);
        draw_celestial_body(
            &mut frame,
            &CelestialBodyDraw {
                center_px: (100, 120),
                radius_px: 42,
                style: PlanetStyle::Neptune,
                sun_dir: (0.7, -0.3, 0.65),
                spin_deg: 25.0,
                ring_tilt_deg: 22.0,
            },
        );
        draw_celestial_body(
            &mut frame,
            &CelestialBodyDraw {
                center_px: (230, 120),
                radius_px: 36,
                style: PlanetStyle::Saturn,
                sun_dir: (0.7, -0.3, 0.65),
                spin_deg: 0.0,
                ring_tilt_deg: 24.0,
            },
        );
        // Core centre pixels are non-black
        assert_ne!(frame.get(100, 120), [0, 0, 0], "Neptune disc drawn");
        assert_ne!(frame.get(230, 120), [0, 0, 0], "Saturn disc drawn");
        // Ring pixel outside Saturn's 36px globe radius (at dx = +60 px) is lit
        assert_ne!(frame.get(290, 120), [0, 0, 0], "Saturn B/A ring drawn");
        // Total unique colours across both rendered giants stays well below 256
        let colours = frame.count_colours(256);
        assert!(
            colours <= 32,
            "expected <= 32 palette colours, got {colours}"
        );
    }

    #[test]
    fn baked_planet_maps_match_index_provenance_and_rotate_surface_geography() {
        let catalog: [(&str, &[u8], PlanetStyle); 12] = [
            ("neptune", BAKED_NEPTUNE_PNG, PlanetStyle::Neptune),
            ("uranus", BAKED_URANUS_PNG, PlanetStyle::Uranus),
            ("saturn", BAKED_SATURN_PNG, PlanetStyle::Saturn),
            ("jupiter", BAKED_JUPITER_PNG, PlanetStyle::Jupiter),
            ("mars", BAKED_MARS_PNG, PlanetStyle::Mars),
            ("earth", BAKED_EARTH_PNG, PlanetStyle::Earth),
            ("titan", BAKED_TITAN_PNG, PlanetStyle::Titan),
            ("triton", BAKED_TRITON_PNG, PlanetStyle::Triton),
            ("io", BAKED_IO_PNG, PlanetStyle::Io),
            ("europa", BAKED_EUROPA_PNG, PlanetStyle::Europa),
            ("ceres", BAKED_CERES_PNG, PlanetStyle::Rocky),
            ("luna", BAKED_LUNA_PNG, PlanetStyle::Luna),
        ];

        let index_json: serde_json::Value =
            serde_json::from_str(include_str!("../planets/index.json")).unwrap_or_default();
        let entries = index_json
            .get("planets")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(entries.len(), 12);

        for (id, png_bytes, style) in catalog {
            let scanlines = stored_idat_scanlines(png_bytes);
            assert!(
                scanlines.is_some(),
                "baked {id}.png must carry a contiguous stored IDAT block"
            );
            assert_eq!(scanlines.map(<[u8]>::len), Some(BAKED_SCANLINE_LEN));

            #[cfg(feature = "encode")]
            {
                use sha2::{Digest as _, Sha256};
                let (w, h, rgba) = crate::png::decode_rgba(png_bytes).unwrap_or((0, 0, Vec::new()));
                assert_eq!((w, h), (BAKED_W, BAKED_H), "{id}.png dimensions");
                assert_eq!(rgba.len(), BAKED_W * BAKED_H * 4);

                let digest: String = Sha256::digest(png_bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let recorded = entries
                    .iter()
                    .find(|e| e.get("id").and_then(serde_json::Value::as_str) == Some(id))
                    .and_then(|e| e.get("sha256"))
                    .and_then(serde_json::Value::as_str);
                assert_eq!(recorded, Some(digest.as_str()), "{id}.png sha256 mismatch");
            }

            let mut f0 = Frame::new(160, 160);
            let mut f90 = Frame::new(160, 160);
            draw_celestial_body(
                &mut f0,
                &CelestialBodyDraw {
                    center_px: (80, 80),
                    radius_px: 48,
                    style,
                    sun_dir: (0.65, -0.25, 0.72),
                    spin_deg: 0.0,
                    ring_tilt_deg: 20.0,
                },
            );
            draw_celestial_body(
                &mut f90,
                &CelestialBodyDraw {
                    center_px: (80, 80),
                    radius_px: 48,
                    style,
                    sun_dir: (0.65, -0.25, 0.72),
                    spin_deg: 90.0,
                    ring_tilt_deg: 20.0,
                },
            );
            assert_ne!(
                f0.hash(),
                f90.hash(),
                "{id} surface geography must rotate with spin_deg"
            );
            assert!(
                f0.count_colours(256) <= 16,
                "{id} must remain within small fixed palette ramp"
            );
        }
    }
}
