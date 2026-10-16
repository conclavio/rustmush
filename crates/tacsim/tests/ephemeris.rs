//! **S7 (doc 10 T17) — the constant sky, under the E lens.**
//!
//! Can any path make a distance change while `rate = 0`? Can two routes disagree
//! about one pair? The sky here is the corridor fixture's — Earth, Mars and Ceres
//! on circular, coplanar orbits at authored phases — and every distance is
//! derived from the same body positions at the same instant.

use mush_core::CanonicalId;
use mush_sim::{DistanceLaw, DriveProfile, LegDef, RouteDef, route_duration, route_duration_at};
use tacsim::{AU_KM, Body, Ephemeris, EphemerisConfig, PackEntry, fixture, load_pack};

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a canonical id")
}

/// The corridor's sky, as the pack authors it.
fn corridor_sky(rate: u32) -> Ephemeris {
    let pack = load_pack(fixture::CORRIDOR_JSON).expect("the corridor loads");
    Ephemeris::from_pack(
        &pack,
        &EphemerisConfig {
            epoch: 1_900_000_000,
            rate,
        },
    )
}

/// The law of cosines, computed here independently of the crate.
fn expected_km(r1_au: f64, r2_au: f64, delta_deg: f64) -> f64 {
    let au = AU_KM as f64;
    let (r1, r2) = (r1_au * au, r2_au * au);
    (r1 * r1 + r2 * r2 - 2.0 * r1 * r2 * delta_deg.to_radians().cos()).sqrt()
}

fn within_two_percent(actual: u64, expected: f64) -> bool {
    let actual = actual as f64;
    (actual - expected).abs() <= expected * 0.02
}

#[test]
fn the_corridor_pack_carries_its_sky() {
    let pack = load_pack(fixture::CORRIDOR_JSON).expect("the corridor loads");
    let bodies: Vec<&str> = pack
        .entries()
        .iter()
        .filter(|(_, e)| matches!(e, PackEntry::Body(_)))
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(bodies, ["ceres", "earth", "mars"]);
    let PackEntry::Location(ceres) = pack.entry(&cid("ceres_extraction")).expect("a place") else {
        panic!("ceres_extraction is a place");
    };
    assert_eq!(ceres.body.as_ref().map(CanonicalId::as_str), Some("ceres"));
    let sky = corridor_sky(0);
    assert_eq!(sky.bodies().count(), 3);
    assert_eq!(
        sky.body(&cid("mars")),
        Some(&Body {
            orbit_uau: 1_524_000,
            period_mdays: 686_980,
            phase_mdeg: 130_000,
            epoch_ic: None,
        })
    );
}

#[test]
fn mars_ceres_and_earth_mars_match_the_geometry_at_the_epoch_within_two_percent() {
    let sky = corridor_sky(0);
    let mars_ceres = sky.pinned_km(&cid("mars"), &cid("ceres")).expect("pinned");
    let earth_mars = sky.pinned_km(&cid("earth"), &cid("mars")).expect("pinned");
    // The authored phases: Earth 300°, Mars 130°, Ceres 140° — the epoch chosen
    // for the corridor, Mars and Ceres ten degrees apart.
    assert!(
        within_two_percent(mars_ceres, expected_km(1.524, 2.766, 10.0)),
        "Mars–Ceres {mars_ceres} km"
    );
    assert!(
        within_two_percent(earth_mars, expected_km(1.0, 1.524, 170.0)),
        "Earth–Mars {earth_mars} km"
    );
    // The favourable pair really is the near one.
    assert!(mars_ceres < earth_mars);
    // And a pair is the same pair either way round.
    assert_eq!(
        sky.pinned_km(&cid("ceres"), &cid("mars")),
        Some(mars_ceres),
        "no pair contradicts itself"
    );
}

#[test]
fn with_the_sky_frozen_a_distance_is_identical_across_any_ic_span() {
    let sky = corridor_sky(0);
    let pinned = sky.pinned_km(&cid("mars"), &cid("ceres")).expect("pinned");
    let year = 365 * 86_400;
    for t in [
        i64::MIN / 4,
        0,
        1_900_000_000 - 50 * year,
        1_900_000_000,
        1_900_000_000 + 1,
        1_900_000_000 + year,
        1_900_000_000 + 1_000 * year,
        i64::MAX / 4,
    ] {
        assert_eq!(
            sky.distance_km_at(&cid("mars"), &cid("ceres"), t),
            Some(pinned),
            "at t = {t}"
        );
        assert_eq!(sky.phase_mdeg_at(&cid("earth"), t), Some(300_000));
    }
}

#[test]
fn a_moving_sky_moves_every_pair_sharing_a_body_together_and_no_pair_contradicts_another() {
    let sky = corridor_sky(1_000);
    let (earth, mars, ceres) = (cid("earth"), cid("mars"), cid("ceres"));
    let t0 = sky.epoch();
    let t1 = t0 + 100 * 86_400;
    let at = |a: &CanonicalId, b: &CanonicalId, t: i64| sky.distance_km_at(a, b, t).expect("known");
    // Every pair changed, because every body moved.
    assert_ne!(at(&earth, &mars, t0), at(&earth, &mars, t1));
    assert_ne!(at(&mars, &ceres, t0), at(&mars, &ceres, t1));
    assert_ne!(at(&earth, &ceres, t0), at(&earth, &ceres, t1));
    // And they changed together: each pair's distance at t1 is what the bodies'
    // positions at t1 give, computed here from the phases the sky reports.
    let position = |id: &CanonicalId| {
        let body = sky.body(id).expect("a body");
        let r = body.orbit_uau as f64 * AU_KM as f64 / 1_000_000.0;
        let theta = (sky.phase_mdeg_at(id, t1).expect("a phase") as f64 / 1_000.0).to_radians();
        (r * theta.cos(), r * theta.sin())
    };
    let bodies = [&earth, &mars, &ceres];
    for a in bodies {
        for b in bodies {
            let (xa, ya) = position(a);
            let (xb, yb) = position(b);
            let from_positions = ((xa - xb).powi(2) + (ya - yb).powi(2)).sqrt();
            let reported = at(a, b, t1) as f64;
            assert!(
                (reported - from_positions).abs() <= 1.0,
                "{a}–{b}: {reported} vs {from_positions}"
            );
        }
    }
    // The triangle inequality holds at every sampled instant: three distances
    // derived from one set of positions cannot contradict one another.
    for t in [t0, t1, t0 + 700 * 86_400, t0 + 5_000 * 86_400] {
        let (em, mc, ec) = (
            at(&earth, &mars, t),
            at(&mars, &ceres, t),
            at(&earth, &ceres, t),
        );
        assert!(
            ec <= em + mc + 1 && em <= ec + mc + 1 && mc <= em + ec + 1,
            "at t = {t}"
        );
    }
}

#[test]
fn a_torch_routes_time_scales_with_the_square_root_of_its_distance() {
    let route = RouteDef::new(
        cid("ceres_phobos"),
        (mush_core::ObjId(10), mush_core::ObjId(12)),
        vec![
            LegDef::new(mush_core::ObjId(11), 86_400).expect("leg"),
            LegDef::new(mush_core::ObjId(12), 172_800).expect("leg"),
        ],
        cid("fusion_torch"),
        1_200,
    )
    .expect("a route")
    .pinned(100_000_000, (cid("ceres"), cid("mars")));
    let torch = DriveProfile::new(cid("fusion_torch"), 1_000, 4_000)
        .expect("a drive")
        .with_law(DistanceLaw::Torch);
    let coast = DriveProfile::new(cid("fusion_torch"), 1_000, 4_000)
        .expect("a drive")
        .with_law(DistanceLaw::Coast);
    let authored = route_duration(&route, &torch, 0).expect("evaluates");
    assert_eq!(authored, 86_400 + 172_800);
    // At the pinned distance the law is silent: the authored time stands.
    assert_eq!(
        route_duration_at(&route, &torch, 0, 100_000_000),
        Ok(authored)
    );
    assert_eq!(
        route_duration_at(&route, &coast, 0, 100_000_000),
        Ok(authored)
    );
    // Four times the distance: a torch takes twice as long, a coaster four times.
    assert_eq!(
        route_duration_at(&route, &torch, 0, 400_000_000),
        Ok(authored * 2)
    );
    assert_eq!(
        route_duration_at(&route, &coast, 0, 400_000_000),
        Ok(authored * 4)
    );
    // A quarter: half, and a quarter.
    assert_eq!(
        route_duration_at(&route, &torch, 0, 25_000_000),
        Ok(authored / 2)
    );
    assert_eq!(
        route_duration_at(&route, &coast, 0, 25_000_000),
        Ok(authored / 4)
    );
    // An unpinned route answers the authored time at any distance.
    let unpinned = RouteDef::new(
        cid("local"),
        (mush_core::ObjId(10), mush_core::ObjId(12)),
        vec![LegDef::new(mush_core::ObjId(12), 600).expect("leg")],
        cid("fusion_torch"),
        0,
    )
    .expect("a route");
    assert_eq!(
        route_duration_at(&unpinned, &torch, 0, 400_000_000),
        Ok(600)
    );
}

#[test]
fn the_nominal_drive_of_a_torch_class_carries_the_torch_law() {
    let torch = tacsim::store::nominal_drive(&cid("fusion_torch")).expect("a drive");
    assert_eq!(torch.law(), DistanceLaw::Torch);
    let hauler = tacsim::store::nominal_drive(&cid("ion_hauler")).expect("a drive");
    assert_eq!(hauler.law(), DistanceLaw::Coast);
}

#[test]
fn a_pack_whose_place_names_an_undeclared_body_is_refused() {
    let tampered = fixture::CORRIDOR_JSON.replace("\"body\": \"earth\"", "\"body\": \"venus\"");
    assert_ne!(tampered, fixture::CORRIDOR_JSON, "the fixture names Earth");
    let error = load_pack(&tampered).expect_err("refused");
    let text = error.to_string();
    assert!(
        text.contains("venus") && text.contains("transfer_node"),
        "{text}"
    );
}

#[test]
fn a_body_with_a_nonsense_orbit_is_refused_at_the_wire() {
    let tampered = fixture::CORRIDOR_JSON.replace("\"orbit_au\": 1.524", "\"orbit_au\": -1.524");
    assert_ne!(tampered, fixture::CORRIDOR_JSON);
    let error = load_pack(&tampered).expect_err("refused");
    assert!(error.to_string().contains("orbit_au"), "{error}");
}
