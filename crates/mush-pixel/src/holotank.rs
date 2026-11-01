//! **The 3D Holo-Tank tactical & orbital plot** (doc 35 cut SF.2, §8.1, D-35-15).
//!
//! Draws either a local-space tactical bubble (`space.holotank`) or the
//! solar-system ephemeris (`sky.chart`, including its out-of-plane `z_au` and
//! `lat_deg` coordinates) as a rotatable 3D cylindrical holographic plot with
//! vertical elevation stalks, velocity/burn vectors, torpedo tracks with TTI
//! countdown tags, and PDC/weapon range rings.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::chart::SKY_CHART_FEED;
use crate::frame::Rgb;
use crate::math::Det as _;
use crate::overlay::{
    BAD, BORDER, DIM, FRIEND, GOOD, HOSTILE, LOCK, ORE, PANEL, TEXT, WARN, YOU, draw_card,
    fill_rect, line, outline_rect, text,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// The local-space tactical holo-tank feed.
pub const SPACE_HOLOTANK_FEED: &str = "space.holotank";

/// Default elevation tilt of the holo-tank disc, degrees from edge-on.
pub const DEFAULT_TILT_DEG: f64 = 32.0;
/// Default azimuth rotation of the holo-tank disc, degrees.
pub const DEFAULT_AZIMUTH_DEG: f64 = 28.0;

const HOLO_BG: Rgb = [6, 10, 18];
const HOLO_RING: Rgb = [28, 56, 84];
const HOLO_AXIS: Rgb = [20, 40, 64];
const STALK_UP: Rgb = [64, 180, 220];
const STALK_DOWN: Rgb = [140, 96, 180];
const VELOCITY_VEC: Rgb = [80, 210, 240];
const BURN_VEC: Rgb = [250, 140, 40];
const PDC_RING: Rgb = [56, 140, 168];
const SUN_COL: Rgb = [250, 210, 90];

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Retained Kitty image ID (`i=186`) for the 3D Holo-Tank picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_HOLOTANK: u32 = 186;
/// Retained Kitty placement ID (`p=86`) for the 3D Holo-Tank picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_HOLOTANK: u32 = 86;
#[cfg(feature = "encode")]
const Z_HOLOTANK: i32 = -50;

/// The 3D Holo-Tank as a [`Picture`].
#[derive(Debug, Default)]
pub struct HolotankPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
}

impl HolotankPicture {
    /// A new 3D Holo-Tank picture.
    #[must_use]
    pub fn new() -> HolotankPicture {
        HolotankPicture::default()
    }

    /// Return Kitty Graphics escape bytes that delete the retained holo-tank image.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_HOLOTANK);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Invalidate the retained Kitty image if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_HOLOTANK {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force the retained Kitty placement to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render the 3D Holo-Tank as a retained Kitty Graphics stream (`a=t` + `a=p`).
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
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_HOLOTANK);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_HOLOTANK,
                PLACEMENT_HOLOTANK,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_HOLOTANK,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for HolotankPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[SPACE_HOLOTANK_FEED, SKY_CHART_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        if let Some(bubble) = feeds.feed(SPACE_HOLOTANK_FEED) {
            if let Some(scene) = bubble_holotank_scene(frame, bubble, look) {
                scene.draw(frame);
            } else {
                draw_card(frame, "NO HOLOTANK TELEMETRY");
            }
            return true;
        }
        if let Some(chart) = feeds.feed(SKY_CHART_FEED) {
            if let Some(scene) = sky_holotank_scene(frame, chart, look) {
                scene.draw(frame);
            } else {
                draw_card(frame, "NO HOLOTANK FROM HERE");
            }
            return true;
        }
        false
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// Project a 3D world point `(x, y, z)` onto the tilted 2D holo-tank plane:
/// returns `(foot_xy, elevated_xy)`.
#[must_use]
pub fn project_3d(
    (x, y, z): (f64, f64, f64),
    azimuth_deg: f64,
    tilt_deg: f64,
) -> ((f64, f64), (f64, f64)) {
    let az = azimuth_deg.to_radians();
    let tilt = tilt_deg.clamp(10.0, 85.0).to_radians();
    let (sin_az, cos_az) = az.det_sin_cos();
    let (sin_tilt, cos_tilt) = tilt.det_sin_cos();

    let rx = x * cos_az - y * sin_az;
    let ry = x * sin_az + y * cos_az;
    let foot_x = rx;
    let foot_y = ry * sin_tilt;
    let elev_y = foot_y + z * cos_tilt;
    ((foot_x, foot_y), (foot_x, elev_y))
}

fn push_tilted_ring(
    scene: &mut Scene,
    radius: f64,
    tilt_deg: f64,
    colour: Rgb,
    dash: Option<(u8, u8)>,
) {
    let tilt = tilt_deg.clamp(10.0, 85.0).to_radians();
    let sin_tilt = tilt.det_sin();
    let steps = 48usize;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let theta = (i as f64) * std::f64::consts::TAU / (steps as f64);
        let (s, c) = theta.det_sin_cos();
        points.push((radius * c, radius * s * sin_tilt));
    }
    scene.push(Item::Polyline {
        points,
        colour,
        dash,
    });
}

/// Build a 3D Holo-Tank [`Scene`] from a `space.holotank` local-bubble keyframe.
#[must_use]
pub fn bubble_holotank_scene(frame: &Frame, holo: &Value, look: &Look) -> Option<Scene> {
    let range_km = num(holo.get("range_km"))
        .filter(|r| *r > 0.0)
        .unwrap_or(1_000.0);
    let zoom = if look.zoom_permille == 0 {
        1.0
    } else {
        (f64::from(look.zoom_permille) / 1_000.0).clamp(0.25, 4.0)
    };
    let r_max = (range_km / zoom).max(1.0);
    let az_deg = num(holo.get("azimuth_deg")).unwrap_or(DEFAULT_AZIMUTH_DEG)
        + (look.offset_mdeg as f64) / 1_000.0;
    let tilt_deg = num(holo.get("tilt_deg")).unwrap_or(DEFAULT_TILT_DEG);

    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    let strip = 11 * s;

    let mut scene = Scene::new(Viewport {
        world: (-r_max, -r_max, r_max, r_max),
        frame: (0, strip, w, h - 2 * strip),
        transform: Transform::Linear,
    });
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: HOLO_BG,
        border: None,
    });

    // Reference ecliptic disc rings & crosshairs
    for frac in [0.25, 0.50, 0.75, 1.0] {
        push_tilted_ring(
            &mut scene,
            r_max * frac,
            tilt_deg,
            HOLO_RING,
            if (frac - 1.0).abs() < 1e-6 {
                None
            } else {
                Some((2, 2))
            },
        );
    }
    let tilt_sin = tilt_deg.clamp(10.0, 85.0).to_radians().det_sin();
    scene.push(Item::Line {
        a: (-r_max, 0.0),
        b: (r_max, 0.0),
        colour: HOLO_AXIS,
        dash: Some((2, 3)),
    });
    scene.push(Item::Line {
        a: (0.0, -r_max * tilt_sin),
        b: (0.0, r_max * tilt_sin),
        colour: HOLO_AXIS,
        dash: Some((2, 3)),
    });

    // PDC envelope ring around origin if present
    if let Some(pdc_km) = num(holo.get("pdc_range_km")).filter(|r| *r > 0.0) {
        push_tilted_ring(&mut scene, pdc_km, tilt_deg, PDC_RING, Some((1, 2)));
    }

    // Contacts (ships, asteroids, stations, derelicts)
    scene.layer();
    if let Some(contacts) = holo.get("contacts").and_then(Value::as_array) {
        for c in contacts {
            let (Some(x), Some(y)) = (num(c.get("x_km")), num(c.get("y_km"))) else {
                continue;
            };
            let z = num(c.get("z_km")).unwrap_or(0.0);
            let ((fx, fy), (ex, ey)) = project_3d((x, y, z), az_deg, tilt_deg);

            let kind = c.get("kind").and_then(Value::as_str).unwrap_or("ship");
            let relation = c
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("neutral");
            let is_you = c.get("you").and_then(Value::as_bool) == Some(true);
            let locked = c.get("locked").and_then(Value::as_bool) == Some(true);

            let col = if is_you {
                YOU
            } else if kind == "asteroid" || kind == "rock" {
                ORE
            } else if relation == "hostile" {
                HOSTILE
            } else if relation == "friend" || relation == "ally" {
                FRIEND
            } else {
                GOOD
            };

            // Elevation stalk from footpoint (fx, fy) to 3D position (ex, ey)
            if (ey - fy).abs() > r_max * 0.01 {
                let stalk_col = if z >= 0.0 { STALK_UP } else { STALK_DOWN };
                scene.push(Item::Marker {
                    at: (fx, fy),
                    half: s,
                    shape: Shape::Ring,
                    colour: stalk_col,
                });
                scene.push(Item::Line {
                    a: (fx, fy),
                    b: (ex, ey),
                    colour: stalk_col,
                    dash: Some((2, 2)),
                });
            }

            // Velocity vector arrow
            if let (Some(vx), Some(vy)) = (num(c.get("vx_kms")), num(c.get("vy_kms"))) {
                let vz = num(c.get("vz_kms")).unwrap_or(0.0);
                let v_scale = r_max * 0.04;
                let (_, (vex, vey)) = project_3d(
                    (x + vx * v_scale, y + vy * v_scale, z + vz * v_scale),
                    az_deg,
                    tilt_deg,
                );
                scene.push(Item::Line {
                    a: (ex, ey),
                    b: (vex, vey),
                    colour: VELOCITY_VEC,
                    dash: None,
                });
            }

            // Burn vector plume
            if let (Some(ax), Some(ay)) = (num(c.get("ax_mps2")), num(c.get("ay_mps2"))) {
                let az_v = num(c.get("az_mps2")).unwrap_or(0.0);
                let a_scale = r_max * 0.002;
                let (_, (aex, aey)) = project_3d(
                    (x - ax * a_scale, y - ay * a_scale, z - az_v * a_scale),
                    az_deg,
                    tilt_deg,
                );
                scene.push(Item::Line {
                    a: (ex, ey),
                    b: (aex, aey),
                    colour: BURN_VEC,
                    dash: None,
                });
            }

            let shape = match kind {
                "asteroid" | "rock" => Shape::Square,
                "station" | "port" => Shape::Ring,
                _ => Shape::Disc,
            };
            scene.push(Item::Marker {
                at: (ex, ey),
                half: 2 * s,
                shape,
                colour: col,
            });
            if locked {
                scene.push(Item::Marker {
                    at: (ex, ey),
                    half: 5 * s,
                    shape: Shape::Ring,
                    colour: LOCK,
                });
            }
            if let Some(label) = c.get("label").and_then(Value::as_str) {
                scene.push(Item::Text {
                    at: (ex, ey),
                    offset: (4 * s, -3 * s),
                    anchor: Anchor::TopLeft,
                    text: label.to_owned(),
                    colour: if is_you {
                        YOU
                    } else if locked {
                        LOCK
                    } else {
                        TEXT
                    },
                    scale: s as usize,
                });
            }
        }
    }

    // Torpedoes & ordnance tracks with TTI countdowns
    scene.layer();
    if let Some(torpedoes) = holo.get("torpedoes").and_then(Value::as_array) {
        for t in torpedoes {
            let (Some(x), Some(y)) = (num(t.get("x_km")), num(t.get("y_km"))) else {
                continue;
            };
            let z = num(t.get("z_km")).unwrap_or(0.0);
            let ((fx, fy), (ex, ey)) = project_3d((x, y, z), az_deg, tilt_deg);

            if (ey - fy).abs() > r_max * 0.01 {
                scene.push(Item::Line {
                    a: (fx, fy),
                    b: (ex, ey),
                    colour: BAD,
                    dash: Some((1, 2)),
                });
            }
            if let (Some(tx), Some(ty)) = (num(t.get("target_x_km")), num(t.get("target_y_km"))) {
                let tz = num(t.get("target_z_km")).unwrap_or(0.0);
                let (_, (tex, tey)) = project_3d((tx, ty, tz), az_deg, tilt_deg);
                scene.push(Item::Line {
                    a: (ex, ey),
                    b: (tex, tey),
                    colour: BAD,
                    dash: Some((2, 2)),
                });
            }
            scene.push(Item::Marker {
                at: (ex, ey),
                half: 2 * s,
                shape: Shape::Square,
                colour: WARN,
            });
            if let Some(tti) = t.get("tti_s").and_then(Value::as_i64) {
                scene.push(Item::Text {
                    at: (ex, ey),
                    offset: (3 * s, 3 * s),
                    anchor: Anchor::TopLeft,
                    text: format!("T-{tti}S"),
                    colour: BAD,
                    scale: s as usize,
                });
            }
        }
    }

    // Header & footer strips
    scene.layer();
    let title = holo
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("3D HOLO-TANK");
    scene.push(Item::Label {
        at: (4 * s, 2 * s),
        text: format!("{title}  R:{r_max:.0}KM  AZ:{az_deg:.0}"),
        colour: TEXT,
        scale: s as usize,
    });
    scene.push(Item::Label {
        at: (4 * s, h - 9 * s),
        text: "STALKS: +Z CYAN / -Z VIOLET   VEC: CYAN   BURN: AMBER".to_owned(),
        colour: DIM,
        scale: s as usize,
    });
    Some(scene)
}

/// Build a system-scale 3D Holo-Tank [`Scene`] from a `sky.chart` keyframe,
/// rendering the `z_au` out-of-plane elevation stalks that the 2D chart flattens.
#[must_use]
pub fn sky_holotank_scene(frame: &Frame, sky: &Value, look: &Look) -> Option<Scene> {
    let chart = sky.get("chart")?;
    let scale = chart.get("scale")?;
    let r_max = num(scale.get("au_max")).filter(|r| *r > 0.0)?;
    let r0 = num(scale.get("r0_au")).filter(|r| *r > 0.0).unwrap_or(0.3);
    let bodies_json = chart.get("bodies")?.as_array()?;

    let az_deg = DEFAULT_AZIMUTH_DEG + (look.offset_mdeg as f64) / 1_000.0;
    let tilt_deg = DEFAULT_TILT_DEG;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    let strip = 11 * s;

    let mut scene = Scene::new(Viewport {
        world: (-r_max, -r_max, r_max, r_max),
        frame: (0, strip, w, h - 2 * strip),
        transform: Transform::LogRadial { r0, r_max },
    });
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: HOLO_BG,
        border: None,
    });

    // Sun at origin
    scene.push(Item::Marker {
        at: (0.0, 0.0),
        half: 3 * s,
        shape: Shape::Disc,
        colour: SUN_COL,
    });

    scene.layer();
    for b in bodies_json {
        if b.get("parent").is_some_and(|p| !p.is_null()) {
            continue;
        }
        let (Some(x), Some(y)) = (num(b.get("x_au")), num(b.get("y_au"))) else {
            continue;
        };
        let z = num(b.get("z_au")).unwrap_or(0.0);
        let r_au = num(b.get("r_au")).unwrap_or_else(|| x.det_hypot(y));
        let here = b.get("here").and_then(Value::as_bool) == Some(true);
        let label = b.get("label").and_then(Value::as_str).unwrap_or("?");

        push_tilted_ring(&mut scene, r_au, tilt_deg, HOLO_RING, Some((2, 3)));

        // Exaggerate z slightly on log-radial plot so 5°-35° inclinations read clearly
        let ((fx, fy), (ex, ey)) = project_3d((x, y, z * 2.5), az_deg, tilt_deg);
        if (ey - fy).abs() > 0.01 {
            let stalk_col = if z >= 0.0 { STALK_UP } else { STALK_DOWN };
            scene.push(Item::Marker {
                at: (fx, fy),
                half: s,
                shape: Shape::Ring,
                colour: stalk_col,
            });
            scene.push(Item::Line {
                a: (fx, fy),
                b: (ex, ey),
                colour: stalk_col,
                dash: Some((2, 2)),
            });
        }
        scene.push(Item::Marker {
            at: (ex, ey),
            half: if here { 3 * s } else { 2 * s },
            shape: if here { Shape::Ring } else { Shape::Disc },
            colour: if here { YOU } else { TEXT },
        });
        scene.push(Item::Text {
            at: (ex, ey),
            offset: (4 * s, -3 * s),
            anchor: Anchor::TopLeft,
            text: label.to_owned(),
            colour: if here { YOU } else { TEXT },
            scale: s as usize,
        });
    }

    scene.layer();
    let date = sky.get("date").and_then(Value::as_str).unwrap_or("");
    scene.push(Item::Label {
        at: (4 * s, 2 * s),
        text: format!("SYSTEM 3D HOLO-TANK  {date}"),
        colour: TEXT,
        scale: s as usize,
    });
    Some(scene)
}

/// Draw a compact 3D Holo-Tank inset into `rect = (x, y, w, h)` on `frame`
/// (used in the top-left corner of the Space Bridge Canopy).
pub fn draw_holotank_inset(
    frame: &mut Frame,
    rect: (i64, i64, i64, i64),
    feeds: &dyn Feeds,
    look: &Look,
) {
    let (rx, ry, rw, rh) = rect;
    if rw < 32 || rh < 32 {
        return;
    }
    fill_rect(frame, rx, ry, rw, rh, PANEL);
    outline_rect(frame, rx, ry, rw, rh, BORDER);

    let cx = rx + rw / 2;
    let cy = ry + rh / 2 + 4;
    let rad_x = (rw * 2) / 5;
    let rad_y = (rh * 2) / 9;

    // Draw reference ellipse & axes
    let steps = 24usize;
    let mut prev = None;
    for i in 0..=steps {
        let theta = (i as f64) * std::f64::consts::TAU / (steps as f64);
        let (s, c) = theta.det_sin_cos();
        let px = cx + ((rad_x as f64) * c).round() as i64;
        let py = cy + ((rad_y as f64) * s).round() as i64;
        if let Some((ox, oy)) = prev {
            line(frame, ox, oy, px, py, HOLO_RING);
        }
        prev = Some((px, py));
    }
    line(frame, cx - rad_x, cy, cx + rad_x, cy, HOLO_AXIS);
    line(frame, cx, cy - rad_y, cx, cy + rad_y, HOLO_AXIS);
    fill_rect(frame, cx - 1, cy - 1, 3, 3, YOU);

    let az_deg = DEFAULT_AZIMUTH_DEG + (look.offset_mdeg as f64) / 1_000.0;
    if let Some(holo) = feeds.feed(SPACE_HOLOTANK_FEED) {
        let range_km = num(holo.get("range_km"))
            .filter(|r| *r > 0.0)
            .unwrap_or(1_000.0);
        if let Some(contacts) = holo.get("contacts").and_then(Value::as_array) {
            for c in contacts {
                let (Some(x), Some(y)) = (num(c.get("x_km")), num(c.get("y_km"))) else {
                    continue;
                };
                let z = num(c.get("z_km")).unwrap_or(0.0);
                let ((fx, fy), (ex, ey)) = project_3d(
                    (x / range_km, y / range_km, z / range_km),
                    az_deg,
                    DEFAULT_TILT_DEG,
                );
                let pfx = cx + (fx.clamp(-1.0, 1.0) * (rad_x as f64)).round() as i64;
                let pfy = cy - (fy.clamp(-1.0, 1.0) * (rad_x as f64)).round() as i64;
                let pex = cx + (ex.clamp(-1.0, 1.0) * (rad_x as f64)).round() as i64;
                let pey = cy - (ey.clamp(-1.0, 1.0) * (rad_x as f64)).round() as i64;
                let col = match c.get("relation").and_then(Value::as_str) {
                    Some("hostile") => HOSTILE,
                    Some("friend" | "ally") => FRIEND,
                    _ => GOOD,
                };
                if (pey - pfy).abs() > 1 {
                    line(frame, pfx, pfy, pex, pey, STALK_UP);
                }
                fill_rect(frame, pex - 1, pey - 1, 3, 3, col);
            }
        }
    }
    let _ = text(frame, rx + 4, ry + 3, "HOLO", DIM, 1);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn holotank_draws_bubble_contacts_torpedoes_and_elevation_stalks() {
        let mut feeds = HashMap::new();
        feeds.insert(
            SPACE_HOLOTANK_FEED.to_owned(),
            json!({
                "title": "KIRKWOOD PICKET",
                "range_km": 1000.0,
                "pdc_range_km": 50.0,
                "contacts": [
                    {
                        "id": "you",
                        "label": "DRAY-P",
                        "x_km": 0.0,
                        "y_km": 0.0,
                        "z_km": 0.0,
                        "vx_kms": 2.0,
                        "vy_kms": 1.0,
                        "vz_kms": 0.0,
                        "you": true
                    },
                    {
                        "id": "raider",
                        "label": "RAIDER",
                        "kind": "ship",
                        "relation": "hostile",
                        "locked": true,
                        "x_km": 420.0,
                        "y_km": -180.0,
                        "z_km": 260.0,
                        "vx_kms": -8.0,
                        "vy_kms": 4.0,
                        "vz_kms": -2.0,
                        "ax_mps2": -120.0,
                        "ay_mps2": 60.0,
                        "az_mps2": 0.0
                    },
                    {
                        "id": "rock_1",
                        "label": "M-12",
                        "kind": "asteroid",
                        "x_km": -300.0,
                        "y_km": 220.0,
                        "z_km": -190.0
                    }
                ],
                "torpedoes": [
                    {
                        "id": "torp_1",
                        "x_km": 210.0,
                        "y_km": -90.0,
                        "z_km": 130.0,
                        "target_x_km": 0.0,
                        "target_y_km": 0.0,
                        "target_z_km": 0.0,
                        "tti_s": 14
                    }
                ]
            }),
        );

        let mut pic = HolotankPicture::new();
        let mut frame = Frame::new(640, 480);
        let look = Look::default();
        assert!(pic.draw(&mut frame, &feeds, &look));
        let colours = frame.count_colours(256);
        assert!(
            (6..=32).contains(&colours),
            "expected rich holo-tank within 256-colour limit, got {colours}"
        );
    }

    #[test]
    fn holotank_falls_back_to_sky_chart_with_z_elevation_stalks() {
        let mut feeds = HashMap::new();
        feeds.insert(
            SKY_CHART_FEED.to_owned(),
            json!({
                "date": "2351-05-13",
                "chart": {
                    "scale": { "au_max": 32.0, "r0_au": 0.3 },
                    "bodies": [
                        { "id": "ceres", "label": "CERES", "x_au": 0.58, "y_au": -2.80, "z_au": -0.23, "r_au": 2.86, "here": true },
                        { "id": "pallas", "label": "PALLAS", "x_au": -1.90, "y_au": 2.10, "z_au": 1.15, "r_au": 3.05, "here": false },
                        { "id": "neptune", "label": "NEPTUNE", "x_au": 24.5, "y_au": -17.2, "z_au": -0.65, "r_au": 30.07, "here": false }
                    ]
                }
            }),
        );

        let mut pic = HolotankPicture::new();
        let mut frame = Frame::new(640, 480);
        assert!(pic.draw(&mut frame, &feeds, &Look::default()));
        assert!(frame.count_colours(256) <= 32);
    }
}
