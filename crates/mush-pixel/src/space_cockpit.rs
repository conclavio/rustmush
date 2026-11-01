//! **The First-Person Space Bridge Canopy** (doc 35 cut SF.2, §8.2, D-35-15).
//!
//! Renders the 20 Hz first-person view from a spacecraft's bridge canopy
//! (`space.cockpit` feed): deterministic 3D starfield rotated by ship attitude
//! `(yaw, pitch, roll)`, analytical raycast planets, moons, and rings via
//! [`crate::planet_shader`], prograde (`⊕`) and retrograde (`⊗`) velocity
//! reticles, target lock box with 3D railgun lead pip, bridge canopy struts and
//! cowling gauges, and a top-left mini 3D Holo-Tank inset.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::holotank::{SPACE_HOLOTANK_FEED, draw_holotank_inset};
use crate::math::Det as _;
use crate::overlay::{
    BAD, BORDER, DIM, FRIEND, GOOD, HOSTILE, LOCK, ORE, PANEL, TEXT, WARN, YOU, draw_card,
    fill_rect, line, outline_rect, ramp, scale_for, text,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::planet_shader::{CelestialBodyDraw, PlanetStyle, draw_celestial_body};

/// The first-person space bridge cockpit feed.
pub const SPACE_COCKPIT_FEED: &str = "space.cockpit";

/// Horizontal field of view of the bridge canopy camera, degrees.
pub const CANOPY_FOV_DEG: f64 = 70.0;

const DEEP_SPACE: Rgb = [4, 6, 12];
const STAR_DIM: Rgb = [96, 108, 132];
const STAR_MID: Rgb = [168, 180, 208];
const STAR_BRIGHT: Rgb = [236, 244, 255];
const PLUME_CORE: Rgb = [180, 232, 255];
const PLUME_HALO: Rgb = [72, 148, 236];
const PROGRADE_COL: Rgb = [80, 224, 140];
const RETROGRADE_COL: Rgb = [236, 140, 64];
const STRUT_FILL: Rgb = [14, 18, 26];
const STRUT_EDGE: Rgb = [52, 64, 84];

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Retained Kitty image ID (`i=187`) for the Space Bridge Canopy picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_SPACE_COCKPIT: u32 = 187;
/// Retained Kitty placement ID (`p=87`) for the Space Bridge Canopy picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_SPACE_COCKPIT: u32 = 87;
#[cfg(feature = "encode")]
const Z_SPACE_COCKPIT: i32 = -50;

/// The First-Person Space Bridge Canopy as a [`Picture`].
#[derive(Debug, Default)]
pub struct SpaceCockpitPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
}

impl SpaceCockpitPicture {
    /// A new Space Bridge Canopy picture.
    #[must_use]
    pub fn new() -> SpaceCockpitPicture {
        SpaceCockpitPicture::default()
    }

    /// Return Kitty Graphics escape bytes that delete the retained image.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_SPACE_COCKPIT);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Invalidate the retained Kitty image if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_SPACE_COCKPIT {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force the retained Kitty placement to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render the Space Bridge Canopy as a retained Kitty Graphics stream (`a=t` + `a=p`).
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<crate::caps::CellSize>,
        step_down: bool,
    ) -> Option<(crate::iso25d::Viewport25D, Vec<u8>)> {
        let vp = crate::cockpit_kitty::viewport_fp(area.2, area.3, cell, step_down);
        let (vw, vh) = (vp.width_px, vp.height_px);
        let mut frame = Frame::new(vw, vh);
        if !self.draw(&mut frame, feeds, look) {
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
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_SPACE_COCKPIT);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_SPACE_COCKPIT,
                PLACEMENT_SPACE_COCKPIT,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_SPACE_COCKPIT,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for SpaceCockpitPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[SPACE_COCKPIT_FEED, SPACE_HOLOTANK_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(cockpit) = feeds.feed(SPACE_COCKPIT_FEED) else {
            return false;
        };
        if cockpit.get("refused").and_then(Value::as_bool) == Some(true) {
            draw_card(frame, "NO BRIDGE VIEW");
            return true;
        }
        draw_space_cockpit(frame, cockpit, feeds, look);
        true
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn wrap_signed_deg(deg: f64) -> f64 {
    let w = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if w == -180.0 { 180.0 } else { w }
}

/// Project a sky/world bearing and pitch `(bearing_deg, pitch_deg)` relative to
/// the camera's `(cam_yaw_deg, cam_pitch_deg, cam_roll_deg)` onto screen pixels.
#[must_use]
pub fn project_canopy(
    (w, h): (i64, i64),
    (bearing_deg, pitch_deg): (f64, f64),
    (cam_yaw_deg, cam_pitch_deg, cam_roll_deg): (f64, f64, f64),
    fov_deg: f64,
) -> Option<(i64, i64)> {
    let dyaw = wrap_signed_deg(bearing_deg - cam_yaw_deg);
    let dpitch = pitch_deg - cam_pitch_deg;
    if dyaw.abs() > fov_deg * 0.85 || dpitch.abs() > fov_deg * 0.75 {
        return None;
    }
    let scale = (w as f64) / fov_deg.max(10.0);
    let dx = dyaw * scale;
    let dy = -dpitch * scale;
    let (sin_r, cos_r) = (-cam_roll_deg.to_radians()).det_sin_cos();
    let rx = dx * cos_r - dy * sin_r;
    let ry = dx * sin_r + dy * cos_r;
    Some((w / 2 + rx.round() as i64, h / 2 + ry.round() as i64))
}

/// Render the full First-Person Space Bridge Canopy into `frame`.
pub fn draw_space_cockpit(frame: &mut Frame, cockpit: &Value, feeds: &dyn Feeds, look: &Look) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let sc = scale_for(frame);
    let s = sc as i64;
    frame.clear(DEEP_SPACE);

    let cam_yaw = (num(cockpit.get("yaw_deg")).unwrap_or(0.0)
        + (look.offset_mdeg as f64) / 1_000.0)
        .rem_euclid(360.0);
    let cam_pitch = num(cockpit.get("pitch_deg"))
        .unwrap_or(0.0)
        .clamp(-89.0, 89.0);
    let cam_roll = num(cockpit.get("roll_deg")).unwrap_or(0.0);
    let zoom = if look.zoom_permille == 0 {
        1.0
    } else {
        (f64::from(look.zoom_permille) / 1_000.0).clamp(0.5, 4.0)
    };
    let fov_deg = CANOPY_FOV_DEG / zoom;

    // 1. Deterministic 3D starfield
    draw_starfield(frame, (cam_yaw, cam_pitch, cam_roll), fov_deg);

    // 2. Solar-system planets, moons & rings sorted back-to-front by distance
    if let Some(bodies) = cockpit.get("bodies").and_then(Value::as_array) {
        let mut sorted: Vec<&Value> = bodies.iter().collect();
        sorted.sort_by(|a, b| {
            let da = num(a.get("distance_km")).unwrap_or(1e12);
            let db = num(b.get("distance_km")).unwrap_or(1e12);
            db.total_cmp(&da)
        });
        for body in sorted {
            let (Some(bearing), Some(pitch)) =
                (num(body.get("bearing_deg")), num(body.get("pitch_deg")))
            else {
                continue;
            };
            let Some((cx, cy)) = project_canopy(
                (w, h),
                (bearing, pitch),
                (cam_yaw, cam_pitch, cam_roll),
                fov_deg,
            ) else {
                continue;
            };
            let id = body.get("id").and_then(Value::as_str).unwrap_or("rocky");
            let radius_px = if let Some(r_px) = body.get("radius_px").and_then(Value::as_i64) {
                (r_px as f64 * zoom).round() as i64
            } else {
                let r_km = num(body.get("radius_km")).unwrap_or(1_000.0);
                let dist_km = num(body.get("distance_km")).unwrap_or(100_000.0).max(1.0);
                let ang_deg = (r_km / dist_km).det_atan2(1.0).to_degrees() * 2.0;
                ((ang_deg / fov_deg) * (w as f64)).round() as i64
            }
            .clamp(2, h * 3 / 4);

            let sun_dir = (
                num(body.get("sun_x")).unwrap_or(0.70),
                num(body.get("sun_y")).unwrap_or(-0.28),
                num(body.get("sun_z")).unwrap_or(0.65),
            );
            let spin_deg = num(body.get("spin_deg")).unwrap_or(0.0);
            let ring_tilt_deg = num(body.get("ring_tilt_deg")).unwrap_or(24.0);

            draw_celestial_body(
                frame,
                &CelestialBodyDraw {
                    center_px: (cx, cy),
                    radius_px,
                    style: PlanetStyle::from_id(id),
                    sun_dir,
                    spin_deg,
                    ring_tilt_deg,
                },
            );
            if let Some(label) = body.get("label").and_then(Value::as_str) {
                let _ = text(
                    frame,
                    cx - (label.len() as i64 * 3 * s),
                    cy + radius_px + 4 * s,
                    label,
                    DIM,
                    sc,
                );
            }
        }
    }

    // 3. Space contacts (ships, asteroids, torpedoes, railgun lead pips)
    if let Some(contacts) = cockpit.get("contacts").and_then(Value::as_array) {
        for c in contacts {
            let (Some(bearing), Some(pitch)) = (num(c.get("bearing_deg")), num(c.get("pitch_deg")))
            else {
                continue;
            };
            let Some((cx, cy)) = project_canopy(
                (w, h),
                (bearing, pitch),
                (cam_yaw, cam_pitch, cam_roll),
                fov_deg,
            ) else {
                continue;
            };
            let kind = c.get("kind").and_then(Value::as_str).unwrap_or("ship");
            let relation = c
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("neutral");
            let locked = c.get("locked").and_then(Value::as_bool) == Some(true);
            let burning = c.get("burning").and_then(Value::as_bool) == Some(true);

            let col = if kind == "asteroid" || kind == "rock" {
                ORE
            } else if kind == "torpedo" {
                BAD
            } else if relation == "hostile" {
                HOSTILE
            } else if relation == "friend" || relation == "ally" {
                FRIEND
            } else {
                GOOD
            };

            if kind == "asteroid" || kind == "rock" {
                let r_px = c
                    .get("radius_px")
                    .and_then(Value::as_i64)
                    .unwrap_or(6 * s)
                    .clamp(3 * s, 32 * s);
                draw_celestial_body(
                    frame,
                    &CelestialBodyDraw {
                        center_px: (cx, cy),
                        radius_px: r_px,
                        style: PlanetStyle::Rocky,
                        sun_dir: (0.7, -0.3, 0.65),
                        spin_deg: (cx + cy) as f64,
                        ring_tilt_deg: 0.0,
                    },
                );
            } else {
                // Actinic fusion drive plume if contact is burning
                if burning {
                    fill_rect(frame, cx - 3 * s, cy - s, 6 * s, 2 * s, PLUME_HALO);
                    fill_rect(frame, cx - s, cy - s, 3 * s, 2 * s, PLUME_CORE);
                }
                let box_h = if locked { 8 * s } else { 5 * s };
                outline_rect(
                    frame,
                    cx - box_h,
                    cy - box_h,
                    box_h * 2 + 1,
                    box_h * 2 + 1,
                    if locked { LOCK } else { col },
                );
            }

            if let Some(label) = c.get("label").and_then(Value::as_str) {
                let _ = text(
                    frame,
                    cx + 7 * s,
                    cy - 3 * s,
                    label,
                    if locked { LOCK } else { col },
                    sc,
                );
            }
            if let Some(tti) = c.get("tti_s").and_then(Value::as_i64) {
                let _ = text(frame, cx + 7 * s, cy + 5 * s, &format!("T-{tti}S"), BAD, sc);
            }

            // 3D Railgun Lead Pip for the locked target
            if locked
                && let (Some(lb), Some(lp)) =
                    (num(c.get("lead_bearing_deg")), num(c.get("lead_pitch_deg")))
                && let Some((lx, ly)) =
                    project_canopy((w, h), (lb, lp), (cam_yaw, cam_pitch, cam_roll), fov_deg)
            {
                line(frame, cx, cy, lx, ly, LOCK);
                outline_rect(frame, lx - 4 * s, ly - 4 * s, 9 * s, 9 * s, LOCK);
                fill_rect(frame, lx, ly, s, s, YOU);
            }
        }
    }

    // 4. Prograde (⊕) and Retrograde (⊗) velocity vector reticles
    if let (Some(pb), Some(pp)) = (
        num(cockpit.get("prograde_bearing_deg")),
        num(cockpit.get("prograde_pitch_deg")),
    ) {
        if let Some((px, py)) =
            project_canopy((w, h), (pb, pp), (cam_yaw, cam_pitch, cam_roll), fov_deg)
        {
            draw_prograde_marker(frame, (px, py), s);
        }
        let retro_b = (pb + 180.0).rem_euclid(360.0);
        let retro_p = -pp;
        if let Some((rx, ry)) = project_canopy(
            (w, h),
            (retro_b, retro_p),
            (cam_yaw, cam_pitch, cam_roll),
            fov_deg,
        ) {
            draw_retrograde_marker(frame, (rx, ry), s);
        }
    }

    // 5. Center boresight reticle
    let (mx, my) = (w / 2, h / 2);
    line(frame, mx - 6 * s, my, mx - 2 * s, my, DIM);
    line(frame, mx + 2 * s, my, mx + 6 * s, my, DIM);
    line(frame, mx, my - 6 * s, mx, my - 2 * s, DIM);
    line(frame, mx, my + 2 * s, mx, my + 6 * s, DIM);

    // 6. Bridge canopy structural struts & cowling instrument strip
    draw_canopy_frame_and_cowling(frame, cockpit, (cam_yaw, cam_pitch), s, sc);

    // 7. Top-left Mini 3D Holo-Tank inset
    let inset_side = (w.min(h) / 4).max(48);
    draw_holotank_inset(frame, (8 * s, 16 * s, inset_side, inset_side), feeds, look);
}

fn draw_starfield_star(frame: &mut Frame, x: i64, y: i64, tier: u64) {
    let col = match tier % 4 {
        0 => STAR_BRIGHT,
        1 => STAR_MID,
        _ => STAR_DIM,
    };
    frame.put(x as usize, y as usize, col);
    if tier.is_multiple_of(7) {
        frame.put((x + 1) as usize, y as usize, STAR_DIM);
    }
}

fn draw_starfield(
    frame: &mut Frame,
    (cam_yaw, cam_pitch, cam_roll): (f64, f64, f64),
    fov_deg: f64,
) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..220 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let bearing = ((seed >> 16) % 36_000) as f64 / 100.0;
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let pitch = (((seed >> 16) % 16_000) as f64 / 100.0) - 80.0;
        if let Some((sx, sy)) = project_canopy(
            (w, h),
            (bearing, pitch),
            (cam_yaw, cam_pitch, cam_roll),
            fov_deg,
        ) && sx >= 0
            && sx < w
            && sy >= 0
            && sy < h
        {
            draw_starfield_star(frame, sx, sy, seed >> 48);
        }
    }
}

fn draw_prograde_marker(frame: &mut Frame, (x, y): (i64, i64), s: i64) {
    let r = 5 * s;
    outline_rect(frame, x - r, y - r, 2 * r + 1, 2 * r + 1, PROGRADE_COL);
    line(frame, x - r - 3 * s, y, x - r, y, PROGRADE_COL);
    line(frame, x + r, y, x + r + 3 * s, y, PROGRADE_COL);
    line(frame, x, y - r - 3 * s, x, y - r, PROGRADE_COL);
    fill_rect(frame, x, y, s, s, PROGRADE_COL);
}

fn draw_retrograde_marker(frame: &mut Frame, (x, y): (i64, i64), s: i64) {
    let r = 5 * s;
    outline_rect(frame, x - r, y - r, 2 * r + 1, 2 * r + 1, RETROGRADE_COL);
    line(frame, x - r, y - r, x + r, y + r, RETROGRADE_COL);
    line(frame, x - r, y + r, x + r, y - r, RETROGRADE_COL);
}

fn draw_canopy_frame_and_cowling(
    frame: &mut Frame,
    cockpit: &Value,
    (cam_yaw, cam_pitch): (f64, f64),
    s: i64,
    sc: usize,
) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);

    // Angled canopy structural struts
    for k in 0..(3 * s) {
        line(frame, k, 12 * s, w / 5 + k, 0, STRUT_EDGE);
        line(frame, w - 1 - k, 12 * s, w - 1 - w / 5 - k, 0, STRUT_EDGE);
        line(frame, k, h - 24 * s, w / 6 + k, h - 1, STRUT_EDGE);
        line(
            frame,
            w - 1 - k,
            h - 24 * s,
            w - 1 - w / 6 - k,
            h - 1,
            STRUT_EDGE,
        );
    }

    // Top heading & attitude bar
    fill_rect(frame, 0, 0, w, 12 * s, STRUT_FILL);
    line(frame, 0, 12 * s, w - 1, 12 * s, BORDER);
    let callsign = cockpit
        .get("callsign")
        .and_then(Value::as_str)
        .unwrap_or("BRIDGE");
    let speed_mps = cockpit
        .get("speed_mps")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let header = format!("{callsign}  HDG:{cam_yaw:03.0} PIT:{cam_pitch:+03.0}  V:{speed_mps}M/S");
    let _ = text(frame, 6 * s, 2 * s, &header, TEXT, sc);

    // Bottom cowling instrument strip
    let cowl_h = 22 * s;
    let cowl_y = h - cowl_h;
    fill_rect(frame, 0, cowl_y, w, cowl_h, PANEL);
    line(frame, 0, cowl_y, w - 1, cowl_y, BORDER);

    let dv_mps = cockpit
        .get("delta_v_mps")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let heat_permille = cockpit
        .get("heat_permille")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let hull_pct = cockpit
        .get("hull_pct")
        .and_then(Value::as_i64)
        .unwrap_or(100);
    let pdc_mode = cockpit
        .get("pdc_mode")
        .and_then(Value::as_str)
        .unwrap_or("AUTO");
    let emcon = cockpit
        .get("emcon")
        .and_then(Value::as_str)
        .unwrap_or("STD");

    let _ = text(
        frame,
        6 * s,
        cowl_y + 3 * s,
        &format!("HULL:{hull_pct}%"),
        ramp(hull_pct, 100),
        sc,
    );
    let _ = text(
        frame,
        68 * s,
        cowl_y + 3 * s,
        &format!("DV:{dv_mps}M/S"),
        GOOD,
        sc,
    );
    let heat_col = if heat_permille >= 800 {
        BAD
    } else if heat_permille >= 500 {
        WARN
    } else {
        GOOD
    };
    let _ = text(
        frame,
        148 * s,
        cowl_y + 3 * s,
        &format!("HEAT:{}%", heat_permille / 10),
        heat_col,
        sc,
    );
    let _ = text(
        frame,
        220 * s,
        cowl_y + 3 * s,
        &format!("PDC:{pdc_mode} EMCON:{emcon}"),
        TEXT,
        sc,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn space_cockpit_renders_neptune_triton_hud_and_holotank_inset_under_256_colours() {
        let mut feeds = HashMap::new();
        feeds.insert(
            SPACE_COCKPIT_FEED.to_owned(),
            json!({
                "callsign": "UNS-HALCYON",
                "yaw_deg": 45.0,
                "pitch_deg": 5.0,
                "roll_deg": 0.0,
                "speed_mps": 4820,
                "delta_v_mps": 18500,
                "heat_permille": 240,
                "hull_pct": 94,
                "pdc_mode": "AUTO",
                "emcon": "QUIET",
                "prograde_bearing_deg": 48.0,
                "prograde_pitch_deg": 4.0,
                "bodies": [
                    {
                        "id": "neptune",
                        "label": "NEPTUNE",
                        "bearing_deg": 42.0,
                        "pitch_deg": 6.0,
                        "distance_km": 95_000.0,
                        "radius_km": 24_622.0,
                        "spin_deg": 40.0,
                        "ring_tilt_deg": 22.0
                    },
                    {
                        "id": "triton",
                        "label": "TRITON",
                        "bearing_deg": 54.0,
                        "pitch_deg": 2.0,
                        "distance_km": 68_000.0,
                        "radius_km": 1_353.0
                    }
                ],
                "contacts": [
                    {
                        "id": "raider_1",
                        "label": "CORVETTE",
                        "kind": "ship",
                        "relation": "hostile",
                        "locked": true,
                        "burning": true,
                        "bearing_deg": 35.0,
                        "pitch_deg": 8.0,
                        "lead_bearing_deg": 38.0,
                        "lead_pitch_deg": 9.0
                    }
                ]
            }),
        );
        feeds.insert(
            SPACE_HOLOTANK_FEED.to_owned(),
            json!({
                "range_km": 500.0,
                "contacts": [
                    { "x_km": 120.0, "y_km": -80.0, "z_km": 60.0, "relation": "hostile" }
                ]
            }),
        );

        let mut pic = SpaceCockpitPicture::new();
        let mut frame = Frame::new(640, 480);
        assert!(pic.draw(&mut frame, &feeds, &Look::default()));
        let colours = frame.count_colours(256);
        assert!(
            (10..=64).contains(&colours),
            "expected rich space cockpit within 256-colour limit, got {colours}"
        );
    }
}
