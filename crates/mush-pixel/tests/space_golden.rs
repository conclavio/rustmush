//! **The space views' golden** (doc 35 cut SF.2, `35a` §8.1): pins the rendered
//! frame hashes of [`SpaceCockpitPicture`], [`HoloTankPicture`], and
//! [`DockingPicture`] across canonical sizes (`640×276` and `640×480`),
//! verifies small-frame (`8×8`, `16×16`) clamp safety, and checks planetary
//! ring geometry in `tests/golden/space_hashes.json`.
//!
//! `PIXEL_BLESS=1 cargo test -p mush-pixel --test space_golden` writes it.

use std::collections::HashMap;

use mush_pixel::Frame;
use mush_pixel::docking::{DockingPicture, SPACE_DOCKING_FEED};
use mush_pixel::holotank::{HolotankPicture, SPACE_HOLOTANK_FEED};
use mush_pixel::picture::{Look, Picture};
use mush_pixel::planet_shader::{CelestialBodyDraw, PlanetStyle, draw_celestial_body};
use mush_pixel::space_cockpit::{SPACE_COCKPIT_FEED, SpaceCockpitPicture};
use serde_json::{Value, json};

const SIZES: [(usize, usize); 2] = [(640, 276), (640, 480)];

fn neptune_cockpit_feed() -> HashMap<String, Value> {
    [(
        SPACE_COCKPIT_FEED.to_string(),
        json!({
            "callsign": "CSV FORTUNATE",
            "hull_id": "dray_p",
            "pixel_model": "freighter",
            "locale": "NEPTUNE HIGH ORBIT",
            "yaw_deg": 0.0,
            "pitch_deg": 0.0,
            "roll_deg": 0.0,
            "speed_mps": 850,
            "prograde_bearing_deg": 3.5,
            "prograde_pitch_deg": -1.0,
            "burning": true,
            "throttle_pct": 85,
            "delta_v_mps": 18400,
            "heat_permille": 120,
            "hull_pct": 100,
            "pdc_mode": "AUTO",
            "emcon": "STD",
            "bodies": [
                {
                    "id": "neptune",
                    "label": "NEPTUNE",
                    "bearing_deg": 348.0,
                    "pitch_deg": -2.0,
                    "radius_px": 42,
                    "distance_km": 95000.0,
                    "sun_x": 0.72,
                    "sun_y": -0.25,
                    "sun_z": 0.65,
                    "spin_deg": 18.0
                },
                {
                    "id": "triton",
                    "label": "TRITON",
                    "bearing_deg": 14.0,
                    "pitch_deg": 4.5,
                    "radius_px": 14,
                    "distance_km": 42000.0,
                    "sun_x": 0.72,
                    "sun_y": -0.25,
                    "sun_z": 0.65,
                    "spin_deg": 0.0
                }
            ],
            "contacts": [
                {
                    "id": "triton.relay",
                    "label": "TRITON RELAY",
                    "kind": "ship",
                    "pixel_model": "halcyon",
                    "relation": "friend",
                    "bearing_deg": 4.0,
                    "pitch_deg": 1.0,
                    "distance_km": 4.2,
                    "locked": true,
                    "burning": false,
                    "lead_bearing_deg": 3.2,
                    "lead_pitch_deg": 0.8
                },
                {
                    "id": "kestrel.1",
                    "label": "RAIDER KESTREL",
                    "kind": "ship",
                    "pixel_model": "raider",
                    "relation": "hostile",
                    "bearing_deg": 344.0,
                    "pitch_deg": 3.5,
                    "distance_km": 2.8,
                    "locked": false,
                    "burning": true
                }
            ]
        }),
    )]
    .into_iter()
    .collect()
}

fn saturn_uranus_cockpit_feed() -> HashMap<String, Value> {
    [(
        SPACE_COCKPIT_FEED.to_string(),
        json!({
            "callsign": "CSV HALCYON",
            "hull_id": "halcyon",
            "pixel_model": "halcyon",
            "locale": "OUTER GIANTS RING SURVEY",
            "yaw_deg": 0.0,
            "pitch_deg": 0.0,
            "roll_deg": 0.0,
            "speed_mps": 450,
            "prograde_bearing_deg": 0.0,
            "prograde_pitch_deg": 0.0,
            "burning": false,
            "throttle_pct": 0,
            "delta_v_mps": 22000,
            "heat_permille": 40,
            "hull_pct": 100,
            "pdc_mode": "AUTO",
            "emcon": "PASSIVE",
            "bodies": [
                {
                    "id": "saturn",
                    "label": "SATURN",
                    "bearing_deg": 344.0,
                    "pitch_deg": 1.5,
                    "radius_px": 32,
                    "distance_km": 180000.0,
                    "sun_x": 0.82,
                    "sun_y": -0.20,
                    "sun_z": 0.54,
                    "spin_deg": 0.0,
                    "ring_tilt_deg": 26.7
                },
                {
                    "id": "uranus",
                    "label": "URANUS",
                    "bearing_deg": 16.0,
                    "pitch_deg": -2.0,
                    "radius_px": 26,
                    "distance_km": 240000.0,
                    "sun_x": 0.65,
                    "sun_y": 0.30,
                    "sun_z": 0.70,
                    "spin_deg": 0.0,
                    "ring_tilt_deg": 97.8
                }
            ],
            "contacts": []
        }),
    )]
    .into_iter()
    .collect()
}

fn holotank_feed() -> HashMap<String, Value> {
    [(
        SPACE_HOLOTANK_FEED.to_string(),
        json!({
            "bubble_id": "ceres.gap31",
            "locale": "KIRKWOOD GAP 3:1 (2.50 AU)",
            "scale_m": 15000,
            "tick": 42,
            "own_ship_id": "ship.1",
            "own_pos_m": [0, 0, 0],
            "own_vel_mps": [40, 0, 180],
            "own_heading_mdeg": 15000,
            "delta_v_rem_mps": 16800,
            "delta_v_max_mps": 24000,
            "reserve_dv_mps": 6000,
            "contacts": [
                {
                    "id": "ship.1",
                    "label": "CSV FORTUNATE",
                    "kind": "ship",
                    "pixel_model": "freighter",
                    "relation": "self",
                    "pos_m": [0, 0, 0],
                    "vel_mps": [40, 0, 180],
                    "detected": true
                },
                {
                    "id": "picket.1",
                    "label": "CUSTOMS CORVETTE",
                    "kind": "ship",
                    "pixel_model": "corvette",
                    "relation": "hostile",
                    "pos_m": [4200, 1200, -3100],
                    "vel_mps": [-80, -15, 60],
                    "detected": true,
                    "locked": true
                },
                {
                    "id": "torp.1",
                    "label": "TORP-1",
                    "kind": "torpedo",
                    "pixel_model": "torpedo",
                    "relation": "hostile",
                    "pos_m": [2100, 600, -1500],
                    "vel_mps": [-420, -80, 310],
                    "detected": true
                },
                {
                    "id": "rock.1",
                    "label": "AST-31A",
                    "kind": "asteroid",
                    "pixel_model": "asteroid",
                    "relation": "neutral",
                    "pos_m": [-3800, -900, 2400],
                    "vel_mps": [0, 0, 0],
                    "detected": true
                }
            ]
        }),
    )]
    .into_iter()
    .collect()
}

fn docking_feed() -> HashMap<String, Value> {
    [(
        SPACE_DOCKING_FEED.to_string(),
        json!({
            "active": true,
            "target_id": "ceres.berth.2",
            "target_label": "CERES SPINDLE BERTH 02",
            "target_kind": "port_berth",
            "pixel_model": "refinery",
            "status": "APPROACH",
            "x_mm": 120,
            "y_mm": -85,
            "z_mm": 6400,
            "vx_mmps": -18,
            "vy_mmps": 12,
            "vz_mmps": -340,
            "pitch_mdeg": 600,
            "yaw_mdeg": -900,
            "roll_mdeg": 1400,
            "dampeners": true
        }),
    )]
    .into_iter()
    .collect()
}

fn golden() -> String {
    let mut cases = Vec::new();
    let look = Look::default();

    for (w, h) in SIZES {
        let mut f_nep = Frame::new(w, h);
        assert!(
            SpaceCockpitPicture::new().draw(&mut f_nep, &neptune_cockpit_feed(), &look),
            "neptune cockpit draws"
        );
        assert!(
            f_nep.count_colours(257) <= 256,
            "neptune cockpit must stay within 256-colour Frame budget"
        );
        cases.push(json!({
            "scene": "cockpit_neptune_triton",
            "size": [w, h],
            "hash": format!("{:016x}", f_nep.hash()),
        }));

        let mut f_rings = Frame::new(w, h);
        assert!(
            SpaceCockpitPicture::new().draw(&mut f_rings, &saturn_uranus_cockpit_feed(), &look),
            "saturn+uranus cockpit draws"
        );
        assert!(
            f_rings.count_colours(257) <= 256,
            "saturn+uranus cockpit must stay within 256-colour Frame budget"
        );
        cases.push(json!({
            "scene": "cockpit_saturn_uranus_rings",
            "size": [w, h],
            "hash": format!("{:016x}", f_rings.hash()),
        }));

        let mut f_holo = Frame::new(w, h);
        assert!(
            HolotankPicture::new().draw(&mut f_holo, &holotank_feed(), &look),
            "holotank draws"
        );
        assert!(
            f_holo.count_colours(257) <= 256,
            "holotank must stay within 256-colour Frame budget"
        );
        cases.push(json!({
            "scene": "holotank_kirkwood_bubble",
            "size": [w, h],
            "hash": format!("{:016x}", f_holo.hash()),
        }));

        let mut f_dock = Frame::new(w, h);
        assert!(
            DockingPicture::new().draw(&mut f_dock, &docking_feed(), &look),
            "docking draws"
        );
        assert!(
            f_dock.count_colours(257) <= 256,
            "docking must stay within 256-colour Frame budget"
        );
        cases.push(json!({
            "scene": "docking_refinery_approach",
            "size": [w, h],
            "hash": format!("{:016x}", f_dock.hash()),
        }));
    }

    serde_json::to_string_pretty(&json!({ "cases": cases })).expect("json") + "\n"
}

#[test]
fn the_space_golden_is_pinned() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/space_hashes.json");
    let now = golden();
    if std::env::var("PIXEL_BLESS").is_ok() {
        std::fs::write(&path, &now).expect("write the golden");
    }
    let pinned = std::fs::read_to_string(&path).expect("the golden (PIXEL_BLESS=1 writes it)");
    assert_eq!(
        pinned.replace("\r\n", "\n"),
        now,
        "PIXEL_BLESS=1 after a space view change"
    );
    let doc: Value = serde_json::from_str(&now).expect("json");
    let hashes: std::collections::BTreeSet<&str> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter_map(|c| c["hash"].as_str())
        .collect();
    assert_eq!(hashes.len(), 8, "eight space pictures, all distinct");
}

#[test]
fn tiny_frames_never_panic_on_clamp() {
    let look = Look::default();
    for (w, h) in [(1, 1), (8, 8), (16, 16), (32, 24)] {
        let mut f1 = Frame::new(w, h);
        let _ = SpaceCockpitPicture::new().draw(&mut f1, &neptune_cockpit_feed(), &look);
        let mut f2 = Frame::new(w, h);
        let _ = HolotankPicture::new().draw(&mut f2, &holotank_feed(), &look);
        let mut f3 = Frame::new(w, h);
        let _ = DockingPicture::new().draw(&mut f3, &docking_feed(), &look);
    }
}

#[test]
fn saturn_rings_project_horizontally_wider_than_vertically() {
    let mut frame = Frame::new(240, 240);
    draw_celestial_body(
        &mut frame,
        &CelestialBodyDraw {
            center_px: (120, 120),
            radius_px: 36,
            style: PlanetStyle::Saturn,
            sun_dir: (0.85, 0.20, 0.48),
            spin_deg: 0.0,
            ring_tilt_deg: 26.7,
        },
    );
    let mut min_x = 240usize;
    let mut max_x = 0usize;
    let mut min_y = 240usize;
    let mut max_y = 0usize;
    for y in 0..240 {
        for x in 0..240 {
            let px = frame.get(x, y);
            if px != [0, 0, 0] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    let span_x = max_x.saturating_sub(min_x);
    let span_y = max_y.saturating_sub(min_y);
    assert!(
        span_x > 140 && span_x > span_y + 40,
        "Saturn rings at 26.7 deg tilt must have a wide horizontal major axis (got span_x={span_x}, span_y={span_y})"
    );
}
