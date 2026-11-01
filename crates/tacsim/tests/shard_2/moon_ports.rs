//! **Doc 19 cut A.8 acceptance — the ports on the moons** (doc 19 §3.8, D-19-19 …
//! D-19-25; the design's §5, §6 and §9 step 7).
//!
//! Pure: the shipped lane through `pack::rules_pack_of`, the Dray's drive from the
//! fielded catalog, the sky from `Sky::from_pack` under an explicit ephemeris
//! configuration. No world. The world-side claims live beside the rigs that can
//! make them: the clerks mint and a v7-shaped world takes v8 by the shelf in
//! `tests/commerce.rs` (`a8_the_moon_ports_stock_list_and_bid`,
//! `a8_a_world_on_lane_v7_takes_v8_by_the_shelf`), the bid-only vendors read Ready
//! in `tests/ports.rs`, and the `Throttle:` line on a real plan in `tests/voyage.rs`
//! (`a8_the_plan_names_the_throttle_that_fits`).
//!
//! No test here types a vertex quantity (D-18-39): the ladder's loads are solved
//! from the authored prices, the goods table and the hold by `support/vertex.rs`.
//! None asserts a passage duration as an invariant (D-18-50): the Titan sweep
//! asserts that the advertised throttle fits the tank and the locker, and prints the
//! engine's ceilings and leg lengths for the help to quote.

use crate::support;
use crate::vertex;

use mush_core::CanonicalId;
use serde_json::Value;
use tacsim::definitions::Catalog;
use tacsim::definitions::content::{DRAY, LANE_JSON, LANE_ROUTE_BACK, LANE_ROUTE_OUT};
use tacsim::definitions::goods::goods_table;
use tacsim::definitions::logistics::VOLUME_L;
use tacsim::pack::{PackEntry, RulesPack, rules_pack_of};
use tacsim::port::capacity::Compartment;
use tacsim::port::{HullDrive, PassagePlan, PortDef, plan_passage};
use tacsim::{EphemerisConfig, Sky};

use support::ground::{UNSIGNED, sign};

/// The Dray's dry mass, kilograms.
const DRY_KG: u64 = 240_000;
/// The tank's capacity, tonnes (the Dray's `propellant` compartment).
const TANK_T: u64 = 80;
/// The locker's capacity, crew-days (480 kg / 1,200 L at 4 kg / 10 L a day).
const LOCKER_DAYS: u64 = 120;
/// A full hold, a full tank, a full locker and one soul: the heaviest honest
/// passage (100 t + 80 t + 480 kg + 100 kg).
const LADEN_FULL_KG: u64 = 100_000 + TANK_T * 1_000 + LOCKER_DAYS * 4 + 100;
/// The same with two souls.
const LADEN_FULL_TWO_KG: u64 = LADEN_FULL_KG + 100;
/// An empty hold with a full tank and locker, one soul.
const EMPTY_FULL_KG: u64 = TANK_T * 1_000 + LOCKER_DAYS * 4 + 100;
/// **The advertised Titan word** (D-19-22): one soul, every listed Titan leg. The
/// sweep below keeps it at least [`THROTTLE_MARGIN`] under the engine's worst
/// ceiling; if the engine moves the ceiling under that, the word becomes 440 here,
/// in the help and in the three stations' speech.
const TITAN_THROTTLE: u32 = 450;
/// The margin the word keeps under the worst-season ceiling, permille (Q14).
const THROTTLE_MARGIN: u32 = 15;
/// The throttle band.
const MIN_THROTTLE: u32 = 250;
const MAX_THROTTLE: u32 = 1_000;
const DAY: i64 = 86_400;
/// The six direct moon routes, `(id, from, to)`.
const MOON_ROUTES: [(&str, &str, &str); 6] = [
    ("ceres_aitken", "ceres_freeport", "aitken_highport"),
    ("aitken_voltaire", "aitken_highport", "voltaire_anchorage"),
    ("aitken_ligeia", "aitken_highport", "ligeia_terminal"),
    ("ligeia_aitken", "ligeia_terminal", "aitken_highport"),
    ("ligeia_voltaire", "ligeia_terminal", "voltaire_anchorage"),
    ("voltaire_stickney", "voltaire_anchorage", "stickney_yards"),
];
/// A port's `routes` map as `(venue, route ids)` pairs.
type Listing = Vec<(String, Vec<String>)>;
/// The three Titan legs the Dray flies throttled.
const TITAN_LEGS: [&str; 3] = ["aitken_ligeia", "ligeia_aitken", "ligeia_voltaire"];

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// The shipped lane through the content parser and `rules_pack_of`.
fn lane() -> RulesPack {
    rules_pack_of(mush_content::parse_pack(LANE_JSON).expect("the lane parses"))
        .expect("the lane decodes")
}

/// The Dray's drive as the fielded catalog holds it, at `throttle`.
fn dray(throttle: u32) -> HullDrive {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let spec = catalog.specs.get(&cid(DRAY)).expect("the Dray is fielded");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let law = *catalog
        .drive_for_class(&logistics.drive.as_ref().expect("the Dray's drive").class)
        .expect("the Dray's class flies a law");
    HullDrive {
        class: logistics
            .drive
            .as_ref()
            .expect("the Dray's drive")
            .class
            .clone(),
        law,
        payload_ceiling_kg: logistics.payload_ceiling_kg,
        throttle_permille: throttle,
    }
}

/// The Dray's hold (its `cargo` compartment) as the fielded catalog holds it.
fn cargo_hold() -> Compartment {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let spec = catalog.specs.get(&cid(DRAY)).expect("the Dray is fielded");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let cargo = logistics
        .stowage
        .get(&cid("cargo"))
        .expect("the cargo compartment");
    Compartment::from_stowage(&cid("cargo"), cargo)
}

/// The lane's common body epoch.
fn lane_epoch(pack: &RulesPack) -> i64 {
    let sky = Sky::from_pack(pack, &EphemerisConfig::default());
    sky.epoch_of(&cid("ceres")).expect("Ceres is in the sky")
}

fn sky_at(pack: &RulesPack, rate: u32) -> Sky {
    Sky::from_pack(
        pack,
        &EphemerisConfig {
            epoch: lane_epoch(pack),
            rate,
        },
    )
}

fn port(pack: &RulesPack, entry: &str) -> PortDef {
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port: {other:?}"),
    }
}

/// A port's ask for `good`, major units.
fn ask(def: &PortDef, good: &str) -> u64 {
    def.vendors
        .iter()
        .flat_map(|v| v.asks.iter())
        .find(|a| a.good.as_str() == good)
        .unwrap_or_else(|| panic!("{} asks {good}", def.venue))
        .price_major
}

/// A port's bid for `good`, major units.
fn bid(def: &PortDef, good: &str) -> u64 {
    def.vendors
        .iter()
        .flat_map(|v| v.bids.iter())
        .find(|b| b.good.as_str() == good)
        .unwrap_or_else(|| panic!("{} bids {good}", def.venue))
        .price_major
}

/// Plan `route` on `sky` at `depart` with `payload` kg, `souls` aboard, at `throttle`.
fn plan(
    sky: &Sky,
    pack: &RulesPack,
    route: &str,
    throttle: u32,
    payload: u64,
    souls: u32,
    depart: i64,
) -> PassagePlan {
    plan_passage(
        sky,
        pack,
        &cid(route),
        &dray(throttle),
        DRY_KG,
        payload,
        souls,
        depart,
    )
    .unwrap_or_else(|e| panic!("{route} at {throttle}‰ plans: {e}"))
}

/// The highest throttle in the band whose burn fits the tank, by bisection; `None`
/// when the floor does not fit.
fn ceiling(
    sky: &Sky,
    pack: &RulesPack,
    route: &str,
    payload: u64,
    souls: u32,
    depart: i64,
) -> Option<u32> {
    let fits = |t: u32| plan(sky, pack, route, t, payload, souls, depart).burn_t <= TANK_T;
    if !fits(MIN_THROTTLE) {
        return None;
    }
    if fits(MAX_THROTTLE) {
        return Some(MAX_THROTTLE);
    }
    let (mut lo, mut hi) = (MIN_THROTTLE, MAX_THROTTLE);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(lo)
}

/// The highest throttle that fits, bisected from `from` (which must fit) up.
#[allow(clippy::too_many_arguments)]
fn ceiling_from(
    sky: &Sky,
    pack: &RulesPack,
    route: &str,
    from: u32,
    payload: u64,
    souls: u32,
    depart: i64,
) -> u32 {
    let fits = |t: u32| plan(sky, pack, route, t, payload, souls, depart).burn_t <= TANK_T;
    assert!(fits(from), "{route}: {from}‰ does not fit");
    if fits(MAX_THROTTLE) {
        return MAX_THROTTLE;
    }
    let (mut lo, mut hi) = (from, MAX_THROTTLE);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The shipped lane with one more single-leg route, re-signed (a pair the lane does
/// not list, planned on its own).
fn with_route(id: &str, from: &str, to: &str) -> RulesPack {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("json");
    let entries = doc["entries"].as_object_mut().expect("entries");
    entries.insert(
        id.to_owned(),
        serde_json::json!({
            "kind": "route",
            "def": {
                "from": from,
                "to": to,
                "legs": [{ "to": to, "base_s": 1 }],
                "drive_class": "coast_hauler",
                "propellant": 0
            }
        }),
    );
    doc["manifest"]["digest"] = Value::String(UNSIGNED.to_owned());
    let signed = sign(&serde_json::to_string(&doc).expect("json"));
    rules_pack_of(mush_content::parse_pack(&signed).expect("parses")).expect("decodes")
}

// ---------------------------------------------------------------------------
// 1. The lane
// ---------------------------------------------------------------------------

/// **The lane carries five ports and six direct routes** (D-19-19): the three
/// places on their moons, the six routes one leg each, the graph exactly — Ceres
/// lists Luna and Stickney, Stickney lists Ceres alone, Luna lists Deimos and
/// Titan, Deimos lists Stickney alone, Titan lists Deimos and Luna — and the two
/// declared one-ways and the unlisted Ceres–Titan pair are absent, so a later cut
/// cannot list them by accident.
#[test]
fn a8_the_lane_carries_five_ports_and_six_direct_routes() {
    let pack = lane();
    for (place, body, label) in [
        ("aitken_highport", "luna", "Aitken Highport"),
        ("voltaire_anchorage", "deimos", "Voltaire Anchorage"),
        ("ligeia_terminal", "titan", "Ligeia Terminal"),
    ] {
        match pack.entry(&cid(place)) {
            Some(PackEntry::Location(def)) => {
                assert_eq!(
                    def.body.as_ref().map(CanonicalId::as_str),
                    Some(body),
                    "{place}"
                );
                assert_eq!(def.label, label, "{place}");
                assert_eq!(def.role.to_string(), "port", "{place}");
            }
            other => panic!("{place}: {other:?}"),
        }
    }
    for (id, from, to) in MOON_ROUTES {
        match pack.entry(&cid(id)) {
            Some(PackEntry::Route(route)) => {
                assert_eq!(route.from.as_str(), from, "{id}");
                assert_eq!(route.to.as_str(), to, "{id}");
                assert_eq!(route.legs.len(), 1, "{id}: one leg, direct");
                assert_eq!(route.legs[0].to.as_str(), to, "{id}");
            }
            other => panic!("{id}: {other:?}"),
        }
    }
    let graph: Vec<(String, Listing)> = [
        "ceres_port",
        "deimos_port",
        "luna_port",
        "stickney_port",
        "titan_port",
    ]
    .iter()
    .map(|entry| {
        let def = port(&pack, entry);
        (
            def.venue.to_string(),
            def.routes
                .iter()
                .map(|(venue, routes)| {
                    (
                        venue.to_string(),
                        routes.iter().map(ToString::to_string).collect(),
                    )
                })
                .collect(),
        )
    })
    .collect();
    let v = |s: &str| s.to_owned();
    assert_eq!(
        graph,
        vec![
            (
                v("ceres"),
                // Doc 20 cut SM.2: Ceres's listing gains the six sites (venue order).
                vec![
                    (v("ahuna"), vec![v("ceres_ahuna")]),
                    (v("commons"), vec![v("ceres_commons")]),
                    (v("juling"), vec![v("ceres_juling")]),
                    (v("kerwan"), vec![v("ceres_kerwan")]),
                    (v("luna"), vec![v("ceres_aitken")]),
                    (v("occator"), vec![v("ceres_occator")]),
                    (v("rheasilvia"), vec![v("ceres_rheasilvia")]),
                    (v("stickney"), vec![v(LANE_ROUTE_OUT)]),
                    // Doc 32 cut S.1: and the 512 site.
                    (v("urvara"), vec![v("ceres_urvara")])
                ]
            ),
            (
                v("deimos"),
                vec![(v("stickney"), vec![v("voltaire_stickney")])]
            ),
            (
                v("luna"),
                vec![
                    (v("deimos"), vec![v("aitken_voltaire")]),
                    (v("titan"), vec![v("aitken_ligeia")])
                ]
            ),
            (v("stickney"), vec![(v("ceres"), vec![v(LANE_ROUTE_BACK)])]),
            (
                v("titan"),
                vec![
                    (v("deimos"), vec![v("ligeia_voltaire")]),
                    (v("luna"), vec![v("ligeia_aitken")])
                ]
            ),
        ]
    );
    // The declared absences (D-19-19): no Luna → Ceres, no Stickney → Deimos, no
    // Ceres ↔ Titan — by route and by listing.
    let routes: Vec<(String, String)> = pack
        .entries()
        .values()
        .filter_map(|entry| match entry {
            PackEntry::Route(route) => Some((route.from.to_string(), route.to.to_string())),
            _ => None,
        })
        .collect();
    for (from, to) in [
        ("aitken_highport", "ceres_freeport"),
        ("stickney_yards", "voltaire_anchorage"),
        ("ceres_freeport", "ligeia_terminal"),
        ("ligeia_terminal", "ceres_freeport"),
    ] {
        assert!(
            !routes.iter().any(|(f, t)| f == from && t == to),
            "{from} -> {to} is listed; cut A.8 declared it absent"
        );
    }
    assert_eq!(
        mush_content::parse_pack(LANE_JSON)
            .expect("the lane parses")
            .entries()
            .len(),
        64,
        "the sixty-four entries (thirty-six at cut A.8; doc 20 cut SM.2's twenty-four; doc 32 cut S.1's four)"
    );
    // The Titan bunkers (L8): reaction mass for sixty-odd fills, provisions at the
    // Company's price.
    let titan = port(&pack, "titan_port");
    let stock = |good: &str| -> u64 {
        titan
            .vendors
            .iter()
            .flat_map(|v| v.stock.iter())
            .filter(|s| s.good.as_str() == good)
            .map(|s| s.qty)
            .sum()
    };
    assert!(
        stock("propellant") >= 60 * TANK_T,
        "{} t of reaction mass",
        stock("propellant")
    );
    assert!(
        stock("provisions") >= 2_000,
        "{} crew-days",
        stock("provisions")
    );
    assert_eq!(ask(&titan, "propellant"), 30);
    assert_eq!(ask(&titan, "provisions"), 20);
}

// ---------------------------------------------------------------------------
// 2. The hop on the real route
// ---------------------------------------------------------------------------

/// **The hop is measured in Mars's frame** (D-19-21) on the lane's own
/// `voltaire_stickney`: under the moving sky, every four IC days over forty, it
/// plans under six IC hours, a tonne, a crew-day.
#[test]
fn a8_the_hop_plans_in_hours_on_the_real_route() {
    let pack = lane();
    let moving = sky_at(&pack, 1_000);
    let epoch = lane_epoch(&pack);
    for k in 0..=10 {
        let p = plan(
            &moving,
            &pack,
            "voltaire_stickney",
            MAX_THROTTLE,
            LADEN_FULL_KG,
            1,
            epoch + k * 4 * DAY,
        );
        assert!(p.total_s < 6 * 3_600, "day {}: {} s", k * 4, p.total_s);
        assert!(p.burn_t <= 2, "day {}: {} t", k * 4, p.burn_t);
        assert_eq!(p.crew_days, 1);
        println!(
            "voltaire_stickney day {}: {} s ({:.1} IC hours), {} t",
            k * 4,
            p.total_s,
            p.total_s as f64 / 3_600.0,
            p.burn_t
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Titan, throttled
// ---------------------------------------------------------------------------

/// **Titan fits the tank only throttled** (D-19-22): at the epoch, laden with a
/// full tank and locker and one soul, every Titan leg at full throttle burns more
/// than the tank holds; at the advertised word it fits with the margin; two souls
/// exceed the locker when the leg runs past sixty days.
#[test]
fn a8_titan_fits_the_tank_only_throttled() {
    let pack = lane();
    let still = sky_at(&pack, 0);
    let epoch = lane_epoch(&pack);
    for route in TITAN_LEGS {
        let full = plan(&still, &pack, route, MAX_THROTTLE, LADEN_FULL_KG, 1, epoch);
        assert!(
            full.burn_t > TANK_T,
            "{route} at full throttle: {} t fits the tank",
            full.burn_t
        );
        let word = plan(
            &still,
            &pack,
            route,
            TITAN_THROTTLE,
            LADEN_FULL_KG,
            1,
            epoch,
        );
        assert!(
            word.burn_t <= TANK_T,
            "{route} at {TITAN_THROTTLE}‰: {} t",
            word.burn_t
        );
        assert!(
            u64::from(word.crew_days) <= LOCKER_DAYS,
            "{route}: {} crew-days",
            word.crew_days
        );
        let c = ceiling(&still, &pack, route, LADEN_FULL_KG, 1, epoch).expect("the floor fits");
        assert!(
            c >= TITAN_THROTTLE + THROTTLE_MARGIN,
            "{route}: ceiling {c}‰ at the epoch"
        );
        let two = plan(
            &still,
            &pack,
            route,
            TITAN_THROTTLE,
            LADEN_FULL_TWO_KG,
            2,
            epoch,
        );
        if word.total_s > 60 * DAY {
            assert!(
                u64::from(two.crew_days) > LOCKER_DAYS,
                "{route}: two souls fit the locker over {} days",
                word.total_s / DAY
            );
        }
        println!(
            "{route} at the epoch: full throttle {} t; {TITAN_THROTTLE}‰ {} t, {} crew-days, {} IC days; ceiling {c}‰",
            full.burn_t,
            word.burn_t,
            word.crew_days,
            word.total_s / DAY
        );
    }
}

/// **The advertised word fits a Saturn year** (D-19-22, Q14): twenty-four
/// departures evenly spaced across 11,000 IC days from the epoch, plus a ±14-day
/// window at one-day steps around each replica worst departure, laden with a
/// full tank and locker, one soul: every Titan leg at the word burns no more than
/// the tank and needs no more crew-days than the locker, and at the word plus
/// [`THROTTLE_MARGIN`] still burns no more than the tank — the burn rises with the
/// throttle, so that is the leg's minimum ceiling keeping the margin. The ceiling is
/// bisected once, at the departure with the largest burn at the margin throttle,
/// and printed with each leg's range of IC days at the word for the help to quote.
#[test]
fn a8_titan_throttle_fits_a_saturn_year() {
    let pack = lane();
    let moving = sky_at(&pack, 1_000);
    let epoch = lane_epoch(&pack);
    let margin = TITAN_THROTTLE + THROTTLE_MARGIN;
    let day_of = |depart: i64| (depart - epoch) / DAY;
    let spaced: Vec<i64> = (0..24).map(|k| epoch + k * 11_000 * DAY / 24).collect();
    // Each leg's replica worst departure (the design's 1-day refinement), ±14 days.
    for (route, worst) in [
        ("aitken_ligeia", 7_526_i64),
        ("ligeia_aitken", 7_106),
        ("ligeia_voltaire", 7_812),
    ] {
        let mut departures = spaced.clone();
        departures.extend((-14..=14).map(|d| epoch + (worst + d) * DAY));
        // The hardest departure at the margin throttle: the largest burn, ties to
        // the longer passage.
        let mut hardest = (0_u64, 0_i64, epoch);
        let (mut shortest_s, mut longest_s) = (i64::MAX, 0_i64);
        let (mut fewest_days, mut most_days) = (u32::MAX, 0_u32);
        let mut heaviest = (0_u64, 0_i64, epoch);
        for &depart in &departures {
            let p = plan(
                &moving,
                &pack,
                route,
                TITAN_THROTTLE,
                LADEN_FULL_KG,
                1,
                depart,
            );
            assert!(
                p.burn_t <= TANK_T,
                "{route} departing day {}: {} t at {TITAN_THROTTLE}‰",
                day_of(depart),
                p.burn_t
            );
            assert!(
                u64::from(p.crew_days) <= LOCKER_DAYS,
                "{route} departing day {}: {} crew-days",
                day_of(depart),
                p.crew_days
            );
            let m = plan(&moving, &pack, route, margin, LADEN_FULL_KG, 1, depart);
            assert!(
                m.burn_t <= TANK_T,
                "{route} departing day {}: {} t at {margin}‰, so the word {TITAN_THROTTLE}‰ keeps less than {THROTTLE_MARGIN}‰ under the engine's ceiling (D-19-22: the word becomes 440)",
                day_of(depart),
                m.burn_t
            );
            if (m.burn_t, m.total_s) > (hardest.0, hardest.1) {
                hardest = (m.burn_t, m.total_s, depart);
            }
            shortest_s = shortest_s.min(p.total_s);
            longest_s = longest_s.max(p.total_s);
            fewest_days = fewest_days.min(p.crew_days);
            most_days = most_days.max(p.crew_days);
            if (p.burn_t, p.total_s) > (heaviest.0, heaviest.1) {
                heaviest = (p.burn_t, p.total_s, depart);
            }
        }
        let worst_at = hardest.2;
        let c = ceiling_from(&moving, &pack, route, margin, LADEN_FULL_KG, 1, worst_at);
        let laden = plan(
            &moving,
            &pack,
            route,
            TITAN_THROTTLE,
            LADEN_FULL_KG,
            1,
            worst_at,
        );
        println!(
            "{route}: laden one-soul ceiling {c}‰ on day +{} (the largest burn at {margin}‰, {} t; {} t and {:.1} IC days at {TITAN_THROTTLE}‰); over {} departures at {TITAN_THROTTLE}‰ a leg runs {:.1}–{:.1} IC days, {fewest_days}–{most_days} crew-days a soul, the worst burn {} t on day +{}",
            day_of(worst_at),
            hardest.0,
            laden.burn_t,
            laden.total_s as f64 / DAY as f64,
            departures.len(),
            shortest_s as f64 / DAY as f64,
            longest_s as f64 / DAY as f64,
            heaviest.0,
            day_of(heaviest.2)
        );
        // Empty with a full tank at the hardest departure: faster, and the ceiling
        // higher (one past the laden ceiling still fits the tank).
        let e = plan(
            &moving,
            &pack,
            route,
            TITAN_THROTTLE,
            EMPTY_FULL_KG,
            1,
            worst_at,
        );
        assert!(
            e.burn_t <= TANK_T && e.total_s < laden.total_s,
            "{route} empty on day +{}: {} t over {} s against {} s laden",
            day_of(worst_at),
            e.burn_t,
            e.total_s,
            laden.total_s
        );
        if c < MAX_THROTTLE {
            let above = plan(&moving, &pack, route, c + 1, EMPTY_FULL_KG, 1, worst_at);
            assert!(
                above.burn_t <= TANK_T,
                "{route} empty on day +{}: {} t at {}‰, the laden ceiling plus one",
                day_of(worst_at),
                above.burn_t,
                c + 1
            );
        }
    }
    // Ceres → Titan is unlisted (D-19-22). Only that direction is swept: the
    // margin throttle fits at the epoch, and over the Saturn year a sampled season
    // comes where it does not — the reason the design unlisted the pair.
    let unlisted = with_route("ceres_ligeia", "ceres_freeport", "ligeia_terminal");
    let moving = sky_at(&unlisted, 1_000);
    let fits_margin = |depart: i64| {
        plan(
            &moving,
            &unlisted,
            "ceres_ligeia",
            margin,
            LADEN_FULL_KG,
            1,
            depart,
        )
        .burn_t
            <= TANK_T
    };
    assert!(
        fits_margin(epoch),
        "Ceres → Titan does not fit {margin}‰ at the epoch"
    );
    let failing = (1..48)
        .map(|k| epoch + k * 11_000 * DAY / 48)
        .find(|&depart| !fits_margin(depart))
        .unwrap_or_else(|| {
            panic!(
                "Ceres → Titan fits {margin}‰ in every sampled season; the design unlisted it for its worst season"
            )
        });
    let c = ceiling(
        &moving,
        &unlisted,
        "ceres_ligeia",
        LADEN_FULL_KG,
        1,
        failing,
    );
    println!(
        "ceres_ligeia (unlisted): the first sampled departure past {margin}‰ is day +{}, its laden one-soul ceiling {}",
        day_of(failing),
        c.map_or_else(|| "under the band".to_owned(), |c| format!("{c}‰"))
    );
}

// ---------------------------------------------------------------------------
// 4. The ladder at the epoch
// ---------------------------------------------------------------------------

/// A circuit's net credits over its IC days: the authored margins less reaction
/// mass at the origin's ask and stores at the origin's ask, laden with a full tank
/// and locker (the planner's burn, never a replica's).
struct Circuit {
    name: &'static str,
    net: i64,
    days: f64,
}

impl Circuit {
    fn per_day(&self) -> f64 {
        self.net as f64 / self.days
    }
}

/// **The ring outranks the shuttle at the epoch, and both outrank an empty
/// Luna-and-back** (the design's §5, pinned as an ordering only — the seasons move
/// it, and no multiplier is asserted): reference Ceres → Stickney → Ceres; the
/// Concord ring Ceres → Luna (concentrate and ice) → Deimos (alloy and boards) →
/// the hop → Stickney → Ceres (alloys and fittings); an empty way home from Luna on
/// a synthetic route the lane does not list. Every leg's load is solved from the
/// authored asks and bids, the goods table and the hold (`support/vertex.rs`;
/// D-18-39) — the exhaustive best of the leg's pair, asserted to be the pair's
/// both-limits vertex — never typed.
#[test]
fn a8_the_ring_outranks_the_shuttle_by_ordering_at_the_epoch() {
    let with_way_home = {
        let mut doc: Value = serde_json::from_str(LANE_JSON).expect("json");
        let entries = doc["entries"].as_object_mut().expect("entries");
        entries.insert(
            "aitken_ceres_empty".to_owned(),
            serde_json::json!({
                "kind": "route",
                "def": {
                    "from": "aitken_highport",
                    "to": "ceres_freeport",
                    "legs": [{ "to": "ceres_freeport", "base_s": 1 }],
                    "drive_class": "coast_hauler",
                    "propellant": 0
                }
            }),
        );
        doc["manifest"]["digest"] = Value::String(UNSIGNED.to_owned());
        let signed = sign(&serde_json::to_string(&doc).expect("json"));
        rules_pack_of(mush_content::parse_pack(&signed).expect("parses")).expect("decodes")
    };
    let pack = &with_way_home;
    let still = sky_at(pack, 0);
    let epoch = lane_epoch(pack);
    let (ceres, stickney, luna, deimos, titan) = (
        port(pack, "ceres_port"),
        port(pack, "stickney_port"),
        port(pack, "luna_port"),
        port(pack, "deimos_port"),
        port(pack, "titan_port"),
    );
    let goods = goods_table().expect("the shipped goods table loads");
    let hold = cargo_hold();
    let litres = |good: &CanonicalId| -> u64 {
        goods
            .good(good)
            .and_then(|row| row.measures.get(&cid(VOLUME_L)).copied())
            .unwrap_or_else(|| panic!("{good} packs to a volume"))
    };
    // A leg's best whole-tonne load of `pair`, bought at `origin`'s asks and sold
    // at `to`'s bids: the exhaustive best, which must be the pair's both-limits
    // vertex (the design's §6); returns its margin.
    let cargo_margin = |leg: &str, origin: &PortDef, to: &PortDef, pair: [&str; 2]| -> i64 {
        let margins: Vec<(CanonicalId, i64)> = pair
            .iter()
            .map(|good| {
                let m = i64::try_from(bid(to, good)).expect("small")
                    - i64::try_from(ask(origin, good)).expect("small");
                (cid(good), m)
            })
            .collect();
        let (load, value) = vertex::best_integer_load(&goods, &hold, &margins);
        let mut by_litres: Vec<CanonicalId> = pair.iter().map(|good| cid(good)).collect();
        by_litres.sort_by_key(|good| litres(good));
        let (dense, bulky) = (&by_litres[0], &by_litres[1]);
        let quantity = |good: &CanonicalId| {
            load.iter()
                .find(|(g, _)| g == good)
                .map(|(_, n)| *n)
                .expect("the good is in the load")
        };
        assert_eq!(
            vertex::both_limits_vertex(&goods, &hold, dense, bulky),
            Some((quantity(dense), quantity(bulky))),
            "{leg}: the best load {load:?} on margins {margins:?} is the both-limits vertex"
        );
        println!(
            "{leg}: {} t {dense} + {} t {bulky} = {value} cr of margin",
            quantity(dense),
            quantity(bulky)
        );
        value
    };
    let legs_cost = |p: &PassagePlan, origin: &PortDef| -> i64 {
        i64::try_from(
            p.burn_t * ask(origin, "propellant")
                + u64::from(p.crew_days) * ask(origin, "provisions"),
        )
        .expect("small")
    };
    let fly = |route: &str, throttle: u32, payload: u64| {
        plan(&still, pack, route, throttle, payload, 1, epoch)
    };
    let days =
        |plans: &[&PassagePlan]| plans.iter().map(|p| p.total_s).sum::<i64>() as f64 / DAY as f64;
    // The cargo margins, each leg once.
    let ore_out = ["metals", "volatiles"];
    let yard_goods = ["structural_alloys", "industrial_parts"];
    let concord = ["structural_alloys", "instrument_boards"];
    let selk = ["carbon_feedstock", "volatiles"];
    let to_stickney = cargo_margin("ceres → stickney", &ceres, &stickney, ore_out);
    let to_ceres = cargo_margin("stickney → ceres", &stickney, &ceres, yard_goods);
    let to_luna_cargo = cargo_margin("ceres → luna", &ceres, &luna, ore_out);
    let to_deimos_cargo = cargo_margin("luna → deimos", &luna, &deimos, concord);
    let to_titan_cargo = cargo_margin("luna → titan", &luna, &titan, concord);
    let from_titan_cargo = cargo_margin("titan → luna", &titan, &luna, selk);
    // Reference: Ceres → Stickney and back, both laden.
    let out = fly(LANE_ROUTE_OUT, MAX_THROTTLE, LADEN_FULL_KG);
    let back = fly(LANE_ROUTE_BACK, MAX_THROTTLE, LADEN_FULL_KG);
    let reference = Circuit {
        name: "reference",
        net: to_stickney + to_ceres - legs_cost(&out, &ceres) - legs_cost(&back, &stickney),
        days: days(&[&out, &back]),
    };
    // The ring: Ceres → Luna, Luna → Deimos, the empty hop, Stickney → Ceres.
    let to_luna = fly("ceres_aitken", MAX_THROTTLE, LADEN_FULL_KG);
    let to_deimos = fly("aitken_voltaire", MAX_THROTTLE, LADEN_FULL_KG);
    let hop = fly("voltaire_stickney", MAX_THROTTLE, EMPTY_FULL_KG);
    let ring = Circuit {
        name: "ring",
        net: to_luna_cargo + to_deimos_cargo + to_ceres
            - legs_cost(&to_luna, &ceres)
            - legs_cost(&to_deimos, &luna)
            - legs_cost(&hop, &deimos)
            - legs_cost(&back, &stickney),
        days: days(&[&to_luna, &to_deimos, &hop, &back]),
    };
    // Luna and back empty: the outbound load, then nothing home on the unlisted way.
    let home = fly("aitken_ceres_empty", MAX_THROTTLE, EMPTY_FULL_KG);
    let luna_and_back = Circuit {
        name: "luna and back empty",
        net: to_luna_cargo - legs_cost(&to_luna, &ceres) - legs_cost(&home, &luna),
        days: days(&[&to_luna, &home]),
    };
    // The Titan shuttle at the word, for the record: Luna → Titan and back laden.
    let out_t = fly("aitken_ligeia", TITAN_THROTTLE, LADEN_FULL_KG);
    let back_t = fly("ligeia_aitken", TITAN_THROTTLE, LADEN_FULL_KG);
    let titan_shuttle = Circuit {
        name: "titan shuttle",
        net: to_titan_cargo + from_titan_cargo
            - legs_cost(&out_t, &luna)
            - legs_cost(&back_t, &titan),
        days: days(&[&out_t, &back_t]),
    };
    for c in [&reference, &ring, &luna_and_back, &titan_shuttle] {
        println!(
            "{}: {} cr net over {:.1} IC days = {:.0} cr a day",
            c.name,
            c.net,
            c.days,
            c.per_day()
        );
    }
    assert!(
        ring.per_day() > reference.per_day(),
        "the ring pays less a day than the shuttle at the epoch"
    );
    assert!(
        reference.per_day() > luna_and_back.per_day(),
        "an empty way home pays more a day than the shuttle"
    );
    assert!(
        titan_shuttle.net > 0,
        "the Titan shuttle loses at the epoch"
    );
}
