//! **Doc 19 cut A.1 acceptance — the moving sky** (doc 19 §3, D-19-1 … D-19-7).
//!
//! Pure: the lane pack from `LANE_JSON`, the sky from `Sky::from_pack`, the
//! golden table from `content/sky/golden.json` (JPL Horizons heliocentric
//! positions every 91 days over 2340–2400, written by
//! `content/sky/fetch_elements.py`). No world.

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use serde::Deserialize;
use tacsim::definitions::content::{LANE_JSON, LANE_ROUTE_BACK, LANE_ROUTE_OUT};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::{AU_KM, EphemerisConfig, PosKm, Sky, julian_day};

/// The golden table, as the script writes it.
const GOLDEN_JSON: &str = include_str!("../content/sky/golden.json");

/// The lane's epoch, 2351-05-13 on the game calendar.
const EPOCH_IC: i64 = 74_152_540_800;

/// **The per-body tolerance the script measured** (doc 19 §3.7), permille of
/// the body's heliocentric distance, rounded up to the next hundredth: the
/// element table against Horizons every 91 days over sixty years. Earth's is the
/// Moon's wobble on its osculating elements; a looser fit fails the cut.
const TOLERANCE_PERMILLE_HUNDREDTHS: [(&str, u64); 11] = [
    ("mercury", 6),
    ("venus", 6),
    ("earth", 110),
    ("mars", 3),
    ("jupiter", 1),
    ("saturn", 2),
    ("ceres", 8),
    ("pallas", 6),
    ("juno", 6),
    ("vesta", 5),
    ("hygiea", 8),
];

#[derive(Deserialize)]
struct Golden {
    epoch_ic: i64,
    bodies: BTreeMap<String, Vec<(i64, f64, f64, f64)>>,
}

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

/// The lane's sky at real motion.
fn sky() -> Sky {
    Sky::from_pack(
        &lane(),
        &EphemerisConfig {
            epoch: EPOCH_IC,
            rate: 1_000,
        },
    )
}

fn golden() -> Golden {
    serde_json::from_str(GOLDEN_JSON).expect("the golden table parses")
}

#[allow(clippy::cast_precision_loss)]
fn as_f64(at: PosKm) -> [f64; 3] {
    [at.x as f64, at.y as f64, at.z as f64]
}

/// **The sky is Kepler's, and every lane body moves on a table** (D-19-1): the
/// lane's twelve bodies all carry element tables, so the sky's model is `kepler`
/// and each body answers a position with latitude; a fixture pack without tables
/// is still the circular sky.
#[test]
fn a1_the_lane_sky_is_keplerian_and_the_fixtures_are_not() {
    let sky = sky();
    assert_eq!(sky.model(), "kepler");
    let bodies: Vec<CanonicalId> = lane()
        .entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Body(def) => Some((id.clone(), def)),
            _ => None,
        })
        .map(|(id, def)| {
            assert!(def.is_keplerian(), "{id} carries an element table");
            assert_eq!(def.epoch_ic, Some(EPOCH_IC), "{id}'s epoch");
            assert!(def.elements.len() >= 100, "{id} has a sixty-year table");
            id
        })
        .collect();
    assert_eq!(bodies.len(), 12, "{bodies:?}");
    for body in &bodies {
        assert!(sky.is_keplerian(body), "{body}");
    }
    let corridor = load_pack(tacsim::fixture::CORRIDOR_JSON).expect("the corridor verifies");
    let fixture = Sky::from_pack(&corridor, &EphemerisConfig::default());
    assert_eq!(fixture.model(), "circular");
    assert!(matches!(fixture, Sky::Circular(_)));
}

/// **The table tracks Horizons** (D-19-2, D-19-7): every golden row of every
/// body, propagated through `Sky::position_km` at real motion, lies within the
/// script's measured tolerance of Horizons' own position — eleven bodies, 241
/// rows each, sixty years.
#[test]
fn a1_sky_kepler_tracks_horizons() {
    let sky = sky();
    let golden = golden();
    assert_eq!(golden.epoch_ic, EPOCH_IC);
    let tolerance: BTreeMap<&str, u64> = TOLERANCE_PERMILLE_HUNDREDTHS.into_iter().collect();
    let mut checked = 0usize;
    for (name, rows) in &golden.bodies {
        let body = cid(name);
        let allowed = *tolerance
            .get(name.as_str())
            .unwrap_or_else(|| panic!("{name} has a tolerance"));
        assert!(rows.len() >= 240, "{name}: {} rows", rows.len());
        let mut worst = 0.0_f64;
        for &(ic, x, y, z) in rows {
            let at = sky
                .position_km(&body, ic)
                .unwrap_or_else(|| panic!("{name} at {ic}"));
            let p = as_f64(at);
            let error = ((p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2)).sqrt();
            let r = (x * x + y * y + z * z).sqrt();
            let permille_hundredths = error / r * 100_000.0;
            worst = worst.max(permille_hundredths);
            checked += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let allowed_f = allowed as f64;
        assert!(
            worst <= allowed_f,
            "{name}: worst {worst:.2} hundredths of a permille, allowed {allowed}"
        );
    }
    assert_eq!(checked, 11 * 241);
}

/// **The real sky is the slice's sky at the epoch** (D-19-6; doc 18 D-18-48): at
/// 2351-05-13 every lane body's heliocentric distance and longitude from its
/// table equal the four circular numbers the slice authored (within the
/// kilometre and the milli-degree), and the latitude the slice flattened is
/// what moves the chords — measured here for the cut note.
#[test]
fn a1_sky_kepler_is_the_circular_sky_at_the_epoch() {
    let pack = lane();
    let sky = sky();
    let circular = tacsim::Ephemeris::from_pack(
        &pack,
        &EphemerisConfig {
            epoch: EPOCH_IC,
            rate: 0,
        },
    );
    for (id, body) in circular.bodies() {
        let (r, lon, lat) = sky.heliocentric(id, EPOCH_IC).expect("a position");
        #[allow(clippy::cast_precision_loss)]
        let authored_km = body.orbit_uau as f64 * AU_KM as f64 / 1_000_000.0;
        assert!(
            (r - authored_km).abs() <= authored_km * 1e-6 + 1.0,
            "{id}: r {r} km against the authored {authored_km}"
        );
        #[allow(clippy::cast_precision_loss)]
        let authored_deg = body.phase_mdeg as f64 / 1_000.0;
        let gap = (lon - authored_deg).rem_euclid(360.0);
        assert!(
            gap.min(360.0 - gap) <= 1e-3,
            "{id}: λ {lon} against the authored {authored_deg}"
        );
        println!(
            "{id}: r {:.6} AU, λ {lon:.3}°, β {lat:+.3}°",
            r / AU_KM as f64
        );
    }
    // Pallas is the owner's example: far off the ecliptic.
    let (_, _, pallas) = sky.heliocentric(&cid("pallas"), EPOCH_IC).expect("Pallas");
    assert!(pallas.abs() > 10.0, "Pallas at β {pallas}");
    // The chord the slice flattened: Ceres–Mars at the epoch, 3D against 2D.
    let (ceres, mars) = (cid("ceres"), cid("mars"));
    let flat = circular
        .distance_km_at(&ceres, &mars, EPOCH_IC)
        .expect("the circular chord");
    let real = sky
        .distance_km(&ceres, EPOCH_IC, &mars, EPOCH_IC)
        .expect("the 3D chord");
    println!("Ceres–Mars at the epoch: circular {flat} km, Kepler {real} km");
    assert!(real > flat, "latitude lengthens the chord");
    assert!(real < flat * 11 / 10, "by less than a tenth");
}

/// **The distance is the integer distance between the two positions** (D-19-4):
/// for every pair of lane bodies, at the epoch and a year either side,
/// `Sky::distance_km` equals `PosKm::distance_km` of the two `position_km`s —
/// one rounding path — and the two instants of a mixed pair are honoured.
#[test]
fn a1_sky_kepler_distance_is_the_integer_euclidean_one() {
    let sky = sky();
    let ids: Vec<CanonicalId> = golden().bodies.keys().map(|name| cid(name)).collect();
    let year = 365 * 86_400;
    for t in [EPOCH_IC - year, EPOCH_IC, EPOCH_IC + year] {
        for (i, a) in ids.iter().enumerate() {
            for b in ids.iter().skip(i + 1) {
                let km = sky.distance_km(a, t, b, t).expect("a distance");
                let (pa, pb) = (
                    sky.position_km(a, t).expect("a"),
                    sky.position_km(b, t).expect("b"),
                );
                assert_eq!(km, pa.distance_km(pb), "{a}-{b} at {t}");
            }
        }
    }
    let (a, b) = (cid("ceres"), cid("mars"));
    let mixed = sky
        .distance_km(&a, EPOCH_IC, &b, EPOCH_IC + year)
        .expect("a distance");
    let (pa, pb) = (
        sky.position_km(&a, EPOCH_IC).expect("a"),
        sky.position_km(&b, EPOCH_IC + year).expect("b"),
    );
    assert_eq!(mixed, pa.distance_km(pb));
}

/// **The calendar rule** (D-19-3): the lane's epoch label is JD 2579876.5, and a
/// year of game days later the sky has moved a year's worth — Mars about half a
/// turn, Ceres about 78° of mean longitude, which its eccentricity (the equation
/// of the centre, up to 2e ≈ 8.7°) spreads across 69°–87° of true longitude.
#[test]
fn a1_the_sky_reads_the_calendar_by_its_label() {
    assert!((julian_day(EPOCH_IC) - 2_579_876.5).abs() < 1e-9);
    let sky = sky();
    let year = 365 * 86_400;
    let (_, mars0, _) = sky.heliocentric(&cid("mars"), EPOCH_IC).expect("Mars");
    let (_, mars1, _) = sky
        .heliocentric(&cid("mars"), EPOCH_IC + year)
        .expect("Mars");
    let moved = (mars1 - mars0).rem_euclid(360.0);
    assert!(
        (150.0..=235.0).contains(&moved),
        "Mars moved {moved}° in a year"
    );
    let (_, ceres0, _) = sky.heliocentric(&cid("ceres"), EPOCH_IC).expect("Ceres");
    let (_, ceres1, _) = sky
        .heliocentric(&cid("ceres"), EPOCH_IC + year)
        .expect("Ceres");
    let moved = (ceres1 - ceres0).rem_euclid(360.0);
    assert!(
        (65.0..=92.0).contains(&moved),
        "Ceres moved {moved}° in a year"
    );
}

/// **Kirkwood trails Ceres** (D-19-6; doc 18 Q-18-36's default): at every
/// instant of the window the depot is within 0.07–0.13 AU of Ceres, behind it
/// in longitude, never leading — the hop is a constant short leg whatever the
/// season.
#[test]
fn a1_kirkwood_trails_ceres_through_the_window() {
    let sky = sky();
    let (ceres, kirkwood) = (cid("ceres"), cid("kirkwood"));
    let year = 365 * 86_400;
    for k in -11..=48 {
        let t = EPOCH_IC + k * year;
        let km = sky
            .distance_km(&ceres, t, &kirkwood, t)
            .expect("a distance");
        let au = km as f64 / AU_KM as f64;
        assert!((0.07..=0.13).contains(&au), "year {k}: {au} AU apart");
        let (_, lc, _) = sky.heliocentric(&ceres, t).expect("Ceres");
        let (_, lk, _) = sky.heliocentric(&kirkwood, t).expect("Kirkwood");
        let behind = (lc - lk).rem_euclid(360.0);
        assert!(
            (0.5..=4.0).contains(&behind),
            "year {k}: Kirkwood {behind}° behind"
        );
    }
}

/// **The lane's legs at the epoch, printed for the pin** (the cut note's
/// figures): each route walked as body pairs at the epoch, `ceil(km / 226)`.
#[test]
fn a1_lane_legs_at_the_epoch_are_printed() {
    let pack = lane();
    let sky = sky();
    let body_of = |place: &CanonicalId| match pack.entry(place) {
        Some(PackEntry::Location(def)) => def.body.clone().expect("a body"),
        other => panic!("{place}: {other:?}"),
    };
    for id in [LANE_ROUTE_OUT, LANE_ROUTE_BACK] {
        let Some(PackEntry::Route(route)) = pack.entry(&cid(id)) else {
            panic!("{id} is a route");
        };
        let mut from = body_of(&route.from);
        for leg in &route.legs {
            let to = body_of(&leg.to);
            let km = sky
                .distance_km(&from, EPOCH_IC, &to, EPOCH_IC)
                .expect("a distance");
            println!(
                "{id} -> {}: {from} to {to} {km} km ({:.4} AU), {} s at 226 km/s (authored {})",
                leg.to,
                km as f64 / AU_KM as f64,
                km.div_ceil(226),
                leg.base_s
            );
            from = to;
        }
    }
}
