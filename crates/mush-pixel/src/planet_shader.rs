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
const BAYER_4X4: [[u8; 4]; 4] = [
    [0, 8, 2, 10],
    [12, 4, 14, 6],
    [3, 11, 1, 9],
    [15, 7, 13, 5],
];

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
    /// Cratered grey-albedo dwarf planet or rocky moon (Ceres, Vesta, Luna, etc.).
    Rocky,
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

const ICE_RING_LIT: [Rgb; 4] = [
    [24, 52, 84],
    [48, 96, 144],
    [84, 148, 204],
    [140, 204, 244],
];

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

    // Spherical longitude & latitude for procedural bands, storms, and craters
    let lon = nz.det_atan2(nx) + spin_rad;
    let lat = ny; // -1.0 (north pole) .. +1.0 (south pole)
    let limb_glow = (1.0 - nz).max(0.0);

    match style {
        PlanetStyle::Neptune => {
            // Deep methane zonal bands + Great Dark Spot + high-altitude white cirrus streaks
            let band = 0.08 * (lat * 14.0).det_cos() + 0.05 * (lat * 28.0 + lon * 0.5).det_sin();
            let dlon = (lon.det_sin()) * 1.4;
            let dlat = (lat - 0.32) * 3.8;
            let dark_spot_dist = dlon * dlon + dlat * dlat;
            let mut level = light * (0.78 + band);
            if dark_spot_dist < 1.0 {
                // Great Dark Spot oval
                level *= 0.52 + 0.25 * dark_spot_dist;
            } else if (1.0..1.45).contains(&dark_spot_dist) && (lat - 0.26).abs() < 0.09 {
                // Bright companion methane cirrus cloud ("Scooter" / high cirrus)
                level = (level + 0.32 * ndotl).min(1.0);
            }
            // Rayleigh/methane limb brightening on the sunlit edge
            if limb_glow > 0.55 && ndotl > 0.15 {
                level = (level + 0.22 * (limb_glow - 0.55)).min(1.0);
            }
            sample_ramp(&NEPTUNE_RAMP, level, px, py)
        }
        PlanetStyle::Uranus => {
            let band = 0.04 * (nx * 10.0).det_cos();
            let level = (light * (0.82 + band) + 0.12 * limb_glow * ndotl).clamp(0.0, 1.0);
            sample_ramp(&URANUS_RAMP, level, px, py)
        }
        PlanetStyle::Saturn => {
            let band = 0.12 * (lat * 18.0).det_cos() + 0.05 * (lat * 36.0).det_sin();
            let level = (light * (0.80 + band)).clamp(0.0, 1.0);
            sample_ramp(&SATURN_RAMP, level, px, py)
        }
        PlanetStyle::Jupiter => {
            let band = 0.16 * (lat * 20.0).det_cos() + 0.07 * (lat * 40.0 + lon.det_sin()).det_sin();
            let dlon = (lon - 0.4).det_sin() * 1.6;
            let dlat = (lat - 0.36) * 4.2;
            if dlon * dlon + dlat * dlat < 1.0 && ndotl > 0.15 {
                return JUPITER_SPOT;
            }
            let level = (light * (0.78 + band)).clamp(0.0, 1.0);
            sample_ramp(&JUPITER_RAMP, level, px, py)
        }
        PlanetStyle::Mars => {
            // Polar ice cap at |lat| > 0.84
            if lat.abs() > 0.84 && ndotl > 0.1 {
                return [240, 244, 248];
            }
            // Valles Marineris equatorial canyon & Syrtis Major dark basin
            let basin = (lon * 2.0).det_sin() * (lat * 3.0).det_cos();
            let canyon = if (lat - 0.08).abs() < 0.05 && lon.det_cos() > 0.3 {
                -0.18
            } else {
                0.0
            };
            let level = (light * (0.76 + 0.14 * basin + canyon)).clamp(0.0, 1.0);
            sample_ramp(&MARS_RAMP, level, px, py)
        }
        PlanetStyle::Earth => {
            let cloud = (lon * 3.0 + lat * 5.0).det_sin() * (lat * 7.0).det_cos();
            let level = (light * (0.75 + 0.20 * cloud) + 0.15 * limb_glow * ndotl).clamp(0.0, 1.0);
            sample_ramp(&EARTH_RAMP, level, px, py)
        }
        PlanetStyle::Titan => {
            // Orange nitrogen haze with detached high-altitude cyan limb ring
            if limb_glow > 0.68 && ndotl > 0.1 {
                return sample_ramp(&TITAN_RAMP, 0.95, px, py);
            }
            let haze = 0.05 * (lat * 8.0).det_cos();
            sample_ramp(&TITAN_RAMP, (light * (0.72 + haze)).clamp(0.0, 0.82), px, py)
        }
        PlanetStyle::Triton => {
            // Cantaloupe terrain + dark nitrogen geyser streaks in southern hemisphere
            let geyser = if lat > 0.2 && ((lon * 6.0 + lat * 4.0).det_sin() > 0.65) {
                -0.22
            } else {
                0.08 * (lon * 3.0).det_cos()
            };
            sample_ramp(&TRITON_RAMP, (light * (0.80 + geyser)).clamp(0.0, 1.0), px, py)
        }
        PlanetStyle::Io => {
            let caldera = (lon * 4.0).det_sin() * (lat * 4.0).det_cos();
            sample_ramp(&IO_RAMP, (light * (0.80 + 0.18 * caldera)).clamp(0.0, 1.0), px, py)
        }
        PlanetStyle::Europa => {
            // Lineae cracks across the ice shell
            let crack = ((lon * 7.0 + lat * 9.0).det_sin()).abs();
            let mod_albedo = if crack < 0.12 { -0.18 } else { 0.06 };
            sample_ramp(&EUROPA_RAMP, (light * (0.84 + mod_albedo)).clamp(0.0, 1.0), px, py)
        }
        PlanetStyle::Rocky | PlanetStyle::Sun => {
            // Cratered regolith + Ceres Occator bright faculae spot
            let crater = (lon * 5.0).det_sin() * (lat * 6.0).det_cos();
            let facula = if (lat + 0.18).abs() < 0.07 && (lon - 0.5).det_cos() > 0.82 {
                0.35
            } else {
                0.0
            };
            let level = (light * (0.74 + 0.16 * crater + facula)).clamp(0.0, 1.0);
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
        assert!(colours <= 32, "expected <= 32 palette colours, got {colours}");
    }
}
