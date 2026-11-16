//! **The First-Person Space Bridge Canopy** (doc 35 cut SF.2, §8.2, D-35-15).
//!
//! Renders the 20 Hz first-person view from a spacecraft's bridge canopy
//! (`space.cockpit` feed): deterministic 3D starfield rotated by ship attitude
//! `(yaw, pitch, roll)`, analytical raycast planets, moons, and rings via
//! [`crate::planet_shader`], prograde (`⊕`) and retrograde (`⊗`) velocity
//! reticles, target lock box, bridge canopy struts and
//! cowling gauges, and a top-left mini 3D Holo-Tank inset.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::holotank::{SPACE_HOLOTANK_FEED, draw_holotank_inset};
use crate::math::Det as _;
use crate::overlay::{
    BAD, BORDER, DIM, FRIEND, GOOD, HOSTILE, LOCK, ORE, PANEL, TEXT, WARN, draw_card, fill_rect,
    line, outline_rect, ramp, scale_for, text,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::planet_shader::{CelestialBodyDraw, PlanetStyle, draw_celestial_body};
use crate::ship_shader::{ShipModelStyle, ShipSpriteDraw, draw_ship_sprite};

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

// ---------------------------------------------------------------------------
// Doc 49 unit CC.9: the fire rows
// ---------------------------------------------------------------------------

/// **The fire rows' head** (doc 49 unit CC.9): the columns every mount row is laid in.
pub const FIRE_HEAD: &str = "Mount          Kind      Rounds/mode     Ready       At the lock";

/// The widest a lock row's callsign is printed, characters: a longer one is cut, so the row
/// stays within the 120-column floor.
pub const FIRE_LABEL_MAX: usize = 20;

/// **What a fire row says at a glance** (doc 49 unit CC.9), for the colour a surface gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireTone {
    /// The column head.
    Head,
    /// A mount that fires now.
    Ready,
    /// A mount whose cycle is running.
    Cycling,
    /// A mount held back by something a wait does not cure on its own: empty, down, too
    /// hot or past reach.
    Held,
    /// The lock's row.
    Lock,
}

/// **One fire row**: its words and its tone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FireRow {
    /// The row, without any indent.
    pub text: String,
    /// What it says at a glance.
    pub tone: FireTone,
}

/// Milliseconds as seconds to a tenth, rounded up, so a cycle with time left never reads
/// nought: `2.4 s`.
#[must_use]
pub fn tenths(ms: u64) -> String {
    let tenths = ms.div_ceil(100);
    format!("{}.{} s", tenths / 10, tenths % 10)
}

/// Metres as kilometres to a tenth, rounded down: `42.0 km`.
#[must_use]
pub fn kilometres(m: u64) -> String {
    format!("{}.{} km", m / 1_000, (m % 1_000) / 100)
}

fn word<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn whole(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

/// One mount's row: its id, its kind, its rounds (or a laser's mode), when it is ready, and
/// what it does to the lock.
fn mount_row(mount: &Value) -> FireRow {
    let id = word(mount, "id").unwrap_or("?");
    let kind = match word(mount, "family").unwrap_or("?") {
        "point_defence" => "point def",
        other => other,
    };
    let held = match (
        word(mount, "mode"),
        whole(mount, "rounds"),
        whole(mount, "rounds_max"),
    ) {
        (Some(mode), _, _) => mode.to_owned(),
        (None, Some(left), Some(max)) => format!("{left}/{max}"),
        (None, Some(left), None) => format!("{left} left"),
        (None, None, _) => "-".to_owned(),
    };
    let state = word(mount, "state").unwrap_or("?");
    let (ready, tone) = match state {
        "ready" => ("ready".to_owned(), FireTone::Ready),
        "cycling" => (
            format!("in {}", tenths(whole(mount, "ready_in_ms").unwrap_or(0))),
            FireTone::Cycling,
        ),
        "too_hot" => ("too hot".to_owned(), FireTone::Held),
        "past_reach" => ("past reach".to_owned(), FireTone::Held),
        other => (other.to_owned(), FireTone::Held),
    };
    let fact = if let Some(damage) = whole(mount, "damage") {
        format!("{damage} a pulse")
    } else if let Some(ms) = whole(mount, "flight_ms") {
        format!("flies {}", tenths(ms))
    } else {
        String::new()
    };
    let text = format!("{id:<14} {kind:<9} {held:<15} {ready:<11} {fact}");
    FireRow {
        text: text.trim_end().to_owned(),
        tone,
    }
}

/// The lock's row: how far off she is and the band of her hull that faces you.
fn lock_row(fire: &Value) -> FireRow {
    let text = match (
        word(fire, "lock_id"),
        fire.get("lock").filter(|lock| !lock.is_null()),
    ) {
        (_, Some(lock)) => {
            let label: String = word(lock, "label")
                .unwrap_or("")
                .chars()
                .take(FIRE_LABEL_MAX)
                .collect();
            format!(
                "Lock {} ({label}): {}, her {} band faces you.",
                word(lock, "id").unwrap_or("?"),
                kilometres(whole(lock, "range_m").unwrap_or(0)),
                word(lock, "facet").unwrap_or("?"),
            )
        }
        (Some(id), None) => format!("Lock {id}: not held by your sensors."),
        (None, None) => "No lock.".to_owned(),
    };
    FireRow {
        text,
        tone: FireTone::Lock,
    }
}

/// **The fire rows** (doc 49 unit CC.9) of the `space.cockpit` feed's `fire` object: the head,
/// one row a mount, then the lock's. What line mode's `+tac/status`, the TUI's space panel and
/// this canopy print, the same words on each, every row within the 120-column floor.
///
/// A mount row reads its id, its kind, its rounds left of what her fit stows (a laser's mode
/// instead, as a laser keeps no magazine), whether it is ready or how long its cycle has to run
/// (or what holds it back: `empty`, `down`, `too hot`, `past reach`), and, against a lock her
/// sensors hold within its reach, what one laser pulse lands there or how long a spinal round
/// flies there. The lock row reads how far off she is and the band of her hull that faces you,
/// of five. Space has no chance roll, so no row carries odds.
#[must_use]
pub fn fire_rows(fire: &Value) -> Vec<FireRow> {
    let mut rows = vec![FireRow {
        text: FIRE_HEAD.to_owned(),
        tone: FireTone::Head,
    }];
    rows.extend(
        fire.get("mounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(mount_row),
    );
    rows.push(lock_row(fire));
    rows
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
            .clamp(2, (h * 3 / 4).max(2));

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

    // 3. Space contacts (ships, asteroids, torpedoes)
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

            let id_str = c.get("id").and_then(Value::as_str).unwrap_or("");
            let label_str = c.get("label").and_then(Value::as_str).unwrap_or("");
            let model_or_kind = c.get("pixel_model").and_then(Value::as_str).unwrap_or(kind);
            let ship_style =
                ShipModelStyle::from_contact(model_or_kind, relation, id_str, label_str);
            let aspect_yaw = num(c.get("aspect_yaw_deg"))
                .or_else(|| num(c.get("yaw_deg")).map(|y| y - bearing))
                .unwrap_or(180.0 - (bearing - cam_yaw));

            if kind == "asteroid" || kind == "rock" {
                let r_px = c
                    .get("radius_px")
                    .and_then(Value::as_i64)
                    .unwrap_or(10 * s)
                    .clamp(4 * s, 32 * s);
                draw_ship_sprite(
                    frame,
                    &ShipSpriteDraw {
                        center_px: (cx, cy),
                        half_size_px: r_px,
                        yaw_deg: aspect_yaw,
                        style: ShipModelStyle::Asteroid,
                        burning: false,
                    },
                );
            } else {
                let half_px = c
                    .get("radius_px")
                    .and_then(Value::as_i64)
                    .unwrap_or(if locked { 14 * s } else { 10 * s })
                    .clamp(5 * s, 32 * s);
                // Actinic fusion drive plume if contact is burning
                if burning {
                    fill_rect(frame, cx - 3 * s, cy - s, 6 * s, 2 * s, PLUME_HALO);
                    fill_rect(frame, cx - s, cy - s, 3 * s, 2 * s, PLUME_CORE);
                }
                draw_ship_sprite(
                    frame,
                    &ShipSpriteDraw {
                        center_px: (cx, cy),
                        half_size_px: half_px,
                        yaw_deg: aspect_yaw,
                        style: ship_style,
                        burning,
                    },
                );
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

    // 6b. Doc 49 unit CC.9: the fire rows over the cowling, bottom left
    draw_fire_panel(frame, cockpit, s);

    // 7. Top-left Mini 3D Holo-Tank inset
    let inset_side = (w.min(h) / 4).max(48);
    draw_holotank_inset(frame, (8 * s, 16 * s, inset_side, inset_side), feeds, look);
}

/// **The fire rows over the cowling** (doc 49 unit CC.9): [`fire_rows`] of the feed's `fire`
/// object, at the font's own size so every row fits a 640-pixel canopy, coloured by its tone.
/// A feed without one (outside a bubble) draws nothing here.
fn draw_fire_panel(frame: &mut Frame, cockpit: &Value, s: i64) {
    let Some(fire) = cockpit.get("fire").filter(|fire| fire.is_object()) else {
        return;
    };
    let rows = fire_rows(fire);
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let line_h = 9;
    let widest = rows
        .iter()
        .map(|row| row.text.chars().count() as i64)
        .max()
        .unwrap_or(0)
        * 6;
    let panel_w = (widest + 8).min(w - 12 * s).max(0);
    let panel_h = rows.len() as i64 * line_h + 6;
    let x = 6 * s;
    let y = (h - 22 * s - panel_h - 2 * s).max(14 * s);
    fill_rect(frame, x, y, panel_w, panel_h, STRUT_FILL);
    outline_rect(frame, x, y, panel_w, panel_h, BORDER);
    for (i, row) in rows.iter().enumerate() {
        let col = match row.tone {
            FireTone::Head => DIM,
            FireTone::Ready => GOOD,
            FireTone::Cycling => TEXT,
            FireTone::Held => WARN,
            FireTone::Lock => LOCK,
        };
        let _ = text(frame, x + 4, y + 4 + i as i64 * line_h, &row.text, col, 1);
    }
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
                        "pitch_deg": 8.0
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

    /// A fire object as `tacsim::space::view::fire_feed` writes it: two lasers (one cycling in
    /// defence), the tubes, a spinal gun part spent and cycling, a point-defence rotary, and a
    /// lock with her top band toward you.
    fn fire_fixture() -> Value {
        json!({
            "mounts": [
                {"id": "laser_dorsal", "family": "laser", "mode": "offence",
                 "ready_in_ms": 0, "state": "ready", "damage": 98},
                {"id": "laser_ventral", "family": "laser", "mode": "defence",
                 "ready_in_ms": 600, "state": "cycling", "damage": 98},
                {"id": "torp_bow", "family": "tubes", "rounds": 16, "rounds_max": 16,
                 "ready_in_ms": 0, "state": "ready"},
                {"id": "rail_spinal", "family": "spinal", "rounds": 31, "rounds_max": 32,
                 "ready_in_ms": 2_350, "state": "cycling", "flight_ms": 2_101},
                {"id": "pdc_port", "family": "point_defence", "rounds": 4_000,
                 "ready_in_ms": 0, "state": "ready"}
            ],
            "lock_id": "raider.7.1",
            "lock": {"id": "raider.7.1", "label": "Belt Raider", "range_m": 42_050,
                     "facet": "top"}
        })
    }

    /// **Doc 49 unit CC.9: a row a mount, then the lock, and no odds.** Rounds of what her fit
    /// stows (a laser's mode instead), ready or the cycle left, what a pulse lands at the lock
    /// and how long a spinal round flies there; the lock's range and her band of five.
    #[test]
    fn cc9_the_fire_rows_read_rounds_mode_ready_and_the_lock_with_no_odds() {
        let rows = fire_rows(&fire_fixture());
        let text: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(
            text,
            [
                FIRE_HEAD,
                "laser_dorsal   laser     offence         ready       98 a pulse",
                "laser_ventral  laser     defence         in 0.6 s    98 a pulse",
                "torp_bow       tubes     16/16           ready",
                "rail_spinal    spinal    31/32           in 2.4 s    flies 2.2 s",
                "pdc_port       point def 4000 left       ready",
                "Lock raider.7.1 (Belt Raider): 42.0 km, her top band faces you.",
            ]
        );
        let tones: Vec<FireTone> = rows.iter().map(|row| row.tone).collect();
        assert_eq!(
            tones,
            [
                FireTone::Head,
                FireTone::Ready,
                FireTone::Cycling,
                FireTone::Ready,
                FireTone::Cycling,
                FireTone::Ready,
                FireTone::Lock,
            ]
        );
        for row in &text {
            let low = row.to_ascii_lowercase();
            assert!(
                !row.contains('%') && !low.contains("odds") && !low.contains("chance"),
                "{row}"
            );
        }
        // A cycle with time left never reads nought; a lock her sensors lost, and none.
        assert_eq!(tenths(1), "0.1 s");
        let mut lost = fire_fixture();
        lost["lock"] = Value::Null;
        assert_eq!(
            fire_rows(&lost).last().map(|row| row.text.clone()),
            Some("Lock raider.7.1: not held by your sensors.".to_owned())
        );
        lost["lock_id"] = Value::Null;
        assert_eq!(
            fire_rows(&lost).last().map(|row| row.text.clone()),
            Some("No lock.".to_owned())
        );
        // Held back: what holds it is the word.
        let held = json!({"mounts": [
            {"id": "a", "family": "spinal", "rounds": 0, "rounds_max": 32,
             "ready_in_ms": 0, "state": "empty"},
            {"id": "b", "family": "laser", "mode": "offence", "ready_in_ms": 0,
             "state": "too_hot"},
            {"id": "c", "family": "laser", "mode": "offence", "ready_in_ms": 0,
             "state": "past_reach"},
            {"id": "d", "family": "point_defence", "rounds": 9, "ready_in_ms": 0,
             "state": "down"}
        ]});
        let words: Vec<String> = fire_rows(&held)[1..5]
            .iter()
            .map(|row| {
                assert_eq!(row.tone, FireTone::Held);
                row.text[41..].trim().to_owned()
            })
            .collect();
        assert_eq!(words, ["empty", "too hot", "past reach", "down"]);
    }

    /// **Doc 49 unit CC.9: the lock row within the 120-column floor at its widest fill**, two
    /// spaces in as `+tac/status` prints it: a 32-character bubble id (`customs.` and two
    /// ten-digit numbers is 29), a callsign past [`FIRE_LABEL_MAX`], the farthest range a
    /// `u64` holds.
    #[test]
    fn cc9_the_lock_row_fits_120_columns_at_its_widest_fill() {
        let fire = json!({
            "mounts": [],
            "lock_id": "x".repeat(32),
            "lock": {"id": "x".repeat(32), "label": "W".repeat(40), "range_m": u64::MAX,
                     "facet": "front"}
        });
        let rows = fire_rows(&fire);
        let lock = &rows.last().expect("the lock row").text;
        assert!(lock.contains(&"W".repeat(FIRE_LABEL_MAX)) && !lock.contains(&"W".repeat(21)));
        assert!(
            2 + lock.chars().count() <= 120,
            "{}: {lock}",
            lock.chars().count()
        );
        assert!(2 + FIRE_HEAD.len() <= 120);
    }

    /// **Doc 49 unit CC.9: the canopy draws the fire rows over its cowling** when the feed
    /// carries them, inside the colour budget, and nothing more without them.
    #[test]
    fn cc9_the_canopy_draws_the_fire_rows_when_the_feed_carries_them() {
        let draw = |fire: Option<Value>| {
            let mut cockpit = json!({
                "callsign": "UNS-HALCYON",
                "yaw_deg": 0.0,
                "pitch_deg": 0.0,
                "roll_deg": 0.0,
                "contacts": []
            });
            if let Some(fire) = fire {
                cockpit["fire"] = fire;
            }
            let mut feeds = HashMap::new();
            feeds.insert(SPACE_COCKPIT_FEED.to_owned(), cockpit);
            let mut frame = Frame::new(640, 480);
            assert!(SpaceCockpitPicture::new().draw(&mut frame, &feeds, &Look::default()));
            frame
        };
        let bare = draw(None);
        let with = draw(Some(fire_fixture()));
        assert_ne!(bare.hash(), with.hash(), "the rows are drawn");
        let colours = with.count_colours(256);
        assert!((10..=64).contains(&colours), "{colours}");
        // The widest row fits the 640-pixel canopy at the font's own size.
        let widest = fire_rows(&fire_fixture())
            .iter()
            .map(|row| row.text.chars().count())
            .max()
            .unwrap_or(0);
        assert!(widest * 6 + 8 <= 640 - 24, "{widest}");
    }
}
