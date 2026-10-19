//! **Doc 18 cut IC9 acceptance — the passage planner** (doc 18 §4.4 "Passage: the
//! planner over the sky at departure", "The fuel function is the planner's
//! output", §4.8; D-18-44 … D-18-47; lead rulings L1, L2, L10, L19, L22 of the IC9
//! brief).
//!
//! Pure: the lane pack from `LANE_JSON` through `pack::rules_pack_of`, the Dray's
//! drive from the fielded catalog, the sky from `Sky::from_pack` under an explicit
//! ephemeris configuration. No world.
//!
//! The figures pinned are **lane v4's** (doc 19 cut A.1: the three-dimensional
//! chords of the Kepler sky at the epoch, Kirkwood trailing Ceres by 2°, 226 km/s
//! legs under `route_duration`'s floor arithmetic, lead rulings L1/L2): out
//! 110,305 + 1,683,834 s (1,794,139), 41 t, 22 crew-days; back 1,496,209 + 98,014 s
//! (1,594,223), 37 t, 20 crew-days. Lane v3's were 959,361 + 735,600 / 653,642 +
//! 852,473 (21 / 18 crew-days); doc 18's 965,623 + 730,645 were lane v1's.

mod support;

use mush_core::CanonicalId;
use serde_json::Value;
use tacsim::definitions::Catalog;
use tacsim::definitions::content::{DRAY, LANE_JSON, LANE_ROUTE_BACK, LANE_ROUTE_OUT};
use tacsim::pack::{PackEntry, RulesPack, rules_pack_of};
use tacsim::port::{
    HullDrive, INTERCEPT_PASSES, PlanError, PortDef, choose_route, intercept_passes, plan_passage,
};
use tacsim::{EphemerisConfig, Sky};

use support::ground::{UNSIGNED, sign};

/// The Dray's dry mass, kilograms (Dray v2: 240 t).
const DRY_KG: u64 = 240_000;
/// The outbound full load's payload (§4.2/§4.4): the both-limits cargo (100 t),
/// 42 t of reaction mass, 22 crew-days (88 kg; 21 at lane v3) and one occupant's
/// 100 kg.
const OUT_PAYLOAD: u64 = 142_188;
/// The return load's payload (§4.4).
const BACK_PAYLOAD: u64 = 106_176;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// The shipped lane through the content parser and `rules_pack_of`.
fn lane() -> RulesPack {
    rules_pack_of(mush_content::parse_pack(LANE_JSON).expect("the lane parses"))
        .expect("the lane decodes")
}

/// The Dray's drive as the fielded catalog holds it.
fn dray() -> HullDrive {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let spec = catalog.specs.get(&cid(DRAY)).expect("the Dray is fielded");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let law = *catalog
        .drive_for_class(&logistics.drive.class)
        .expect("the Dray's class flies a law");
    HullDrive {
        class: logistics.drive.class.clone(),
        law,
        payload_ceiling_kg: logistics.payload_ceiling_kg,
    }
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

fn body_of(pack: &RulesPack, place: &CanonicalId) -> CanonicalId {
    match pack.entry(place) {
        Some(PackEntry::Location(def)) => def.body.clone().expect("a body"),
        other => panic!("{place} is a location: {other:?}"),
    }
}

/// A route's places walked as body pairs `(from, to, base_s)`.
fn body_pairs(pack: &RulesPack, route: &str) -> Vec<(CanonicalId, CanonicalId, u32)> {
    let Some(PackEntry::Route(entry)) = pack.entry(&cid(route)) else {
        panic!("{route} is a route");
    };
    let mut from = body_of(pack, &entry.from);
    entry
        .legs
        .iter()
        .map(|leg| {
            let to = body_of(pack, &leg.to);
            let pair = (from.clone(), to.clone(), leg.base_s);
            from = to;
            pair
        })
        .collect()
}

fn secs(plan: &tacsim::port::PassagePlan) -> Vec<u32> {
    plan.legs.iter().map(|leg| leg.secs).collect()
}

// ---------------------------------------------------------------------------
// 1. The slice figures
// ---------------------------------------------------------------------------

/// **`plan_passage` reproduces the slice figures** at the constant sky: every
/// leg's unladen time is the pack's own `base_s` (the sky and the pack agree),
/// and the laden legs, totals, burns and crew-days are lane v3's (lead ruling L2).
#[test]
fn plan_passage_reproduces_the_slice_figures() {
    let pack = lane();
    let sky = sky_at(&pack, 0);
    let drive = dray();
    let depart = lane_epoch(&pack);
    // The sky and the pack agree: the unladen intercept is the authored base_s.
    for route in [LANE_ROUTE_OUT, LANE_ROUTE_BACK] {
        for (a, b, base) in body_pairs(&pack, route) {
            let passes = intercept_passes(&sky, &a, &b, depart, &drive, 0, 1).expect("plans");
            assert_eq!(
                passes.first().map(|(_, s)| *s),
                Some(base),
                "{route}: {a} -> {b}: the sky and the pack disagree"
            );
        }
    }
    let out = plan_passage(
        &sky,
        &pack,
        &cid(LANE_ROUTE_OUT),
        &drive,
        DRY_KG,
        OUT_PAYLOAD,
        1,
        depart,
    )
    .expect("the outbound plan");
    assert_eq!(secs(&out), vec![110_305, 1_683_834]);
    assert_eq!(out.total_s, 1_794_139);
    assert_eq!(out.burn_t, 41);
    assert_eq!(out.crew_days, 22);
    assert_eq!(out.law.as_str(), "coast");
    assert_eq!(out.route.as_str(), LANE_ROUTE_OUT);
    assert_eq!(out.depart_ic, depart);
    assert_eq!(
        out.legs.first().map(|l| l.to.as_str()),
        Some("kirkwood_depot")
    );
    assert_eq!(
        out.legs.last().map(|l| l.to.as_str()),
        Some("stickney_yards")
    );
    assert_eq!(out.sky_rev, sky.revision().expect("a revision"));
    out.check().expect("the plan checks");

    let back = plan_passage(
        &sky,
        &pack,
        &cid(LANE_ROUTE_BACK),
        &drive,
        DRY_KG,
        BACK_PAYLOAD,
        1,
        depart,
    )
    .expect("the return plan");
    assert_eq!(secs(&back), vec![1_496_209, 98_014]);
    assert_eq!(back.total_s, 1_594_223);
    assert_eq!(back.burn_t, 37);
    assert_eq!(back.crew_days, 20);

    // The rail: the ceiling plans, one kilogram past it refuses.
    let ceiling = drive.payload_ceiling_kg;
    assert_eq!(ceiling, 181_000);
    plan_passage(
        &sky,
        &pack,
        &cid(LANE_ROUTE_OUT),
        &drive,
        DRY_KG,
        ceiling,
        1,
        depart,
    )
    .expect("the ceiling plans");
    assert_eq!(
        plan_passage(
            &sky,
            &pack,
            &cid(LANE_ROUTE_OUT),
            &drive,
            DRY_KG,
            ceiling + 1,
            1,
            depart
        ),
        Err(PlanError::Overloaded {
            payload_kg: 181_001,
            ceiling_kg: 181_000
        })
    );
    // Another class does not fly the lane; an unknown route is refused.
    let wrong = HullDrive {
        class: cid("torch_hauler"),
        ..drive.clone()
    };
    assert!(matches!(
        plan_passage(
            &sky,
            &pack,
            &cid(LANE_ROUTE_OUT),
            &wrong,
            DRY_KG,
            0,
            1,
            depart
        ),
        Err(PlanError::UnknownDriveClass { .. })
    ));
    assert!(matches!(
        plan_passage(&sky, &pack, &cid("nowhere"), &drive, DRY_KG, 0, 1, depart),
        Err(PlanError::UnknownRoute { .. })
    ));
}

// ---------------------------------------------------------------------------
// 2. The constant sky
// ---------------------------------------------------------------------------

/// **A constant-sky plan is departure-invariant**: 400 days later every figure is
/// the same and the instants shift by exactly 400 days.
#[test]
fn constant_sky_plan_is_departure_invariant() {
    let pack = lane();
    let sky = sky_at(&pack, 0);
    let drive = dray();
    let depart = lane_epoch(&pack);
    let later = depart + 400 * 86_400;
    let plan = |at: i64| {
        plan_passage(
            &sky,
            &pack,
            &cid(LANE_ROUTE_OUT),
            &drive,
            DRY_KG,
            OUT_PAYLOAD,
            1,
            at,
        )
        .expect("plans")
    };
    let (a, b) = (plan(depart), plan(later));
    assert_eq!(secs(&a), secs(&b));
    assert_eq!(a.total_s, b.total_s);
    assert_eq!(a.burn_t, b.burn_t);
    assert_eq!(a.crew_days, b.crew_days);
    assert_eq!(a.sky_rev, b.sky_rev);
    for (x, y) in a.legs.iter().zip(&b.legs) {
        assert_eq!(x.dist_km, y.dist_km);
        assert_eq!(y.t0 - x.t0, 400 * 86_400);
        assert_eq!(y.t1 - x.t1, 400 * 86_400);
    }
}

// ---------------------------------------------------------------------------
// 3. The moving sky
// ---------------------------------------------------------------------------

/// **A moving circular sky moves the plan, and the intercept converges.**
///
/// At rate 1000 (the real sky's motion) half a Ceres–Mars synodic period after the
/// lane's epoch the bodies stand far apart, so the plan is strictly longer than
/// the constant sky's; two runs are bit-equal; the Coast burn is distance-free.
///
/// *Convergence, as contraction* (reviewer F5): the iteration `t1 ↦ t0 +
/// laden(d(t0, t1))` has a Lipschitz constant of about (the target body's speed
/// along the chord) × (the laden factor) / kms — for Mars at ≈ 24 km/s and the
/// full load's factor 323,184 / 181,000 ≈ 1.79 against 226 km/s, ≈ 0.19 — so each
/// pass shrinks the correction by about a factor of five, and a fourth pass moves
/// a leg by well under 2 % even at the half-synodic geometry. Asserted: the planner
/// stops at pass three (`s3 == leg.secs`), each correction is no larger than the
/// one before (`+ 1` absorbs the ceil and floor steps), and the fourth pass moves
/// the leg by at most 2 % of it. A fixed 0.5 % bound is not implied and is not
/// asserted.
#[test]
fn moving_circular_sky_moves_the_plan_and_converges() {
    let pack = lane();
    let drive = dray();
    let epoch = lane_epoch(&pack);
    let period = |body: &str| -> f64 {
        match pack.entry(&cid(body)) {
            Some(PackEntry::Body(def)) => def.period_mdays as f64 / 1_000.0,
            other => panic!("{body} is a body: {other:?}"),
        }
    };
    let synodic_days = 1.0 / (1.0 / period("mars") - 1.0 / period("ceres")).abs();
    let half = (synodic_days / 2.0 * 86_400.0) as i64;
    let depart = epoch + half;

    let still = sky_at(&pack, 0);
    let moving = sky_at(&pack, 1_000);
    let plan = |sky: &Sky| {
        plan_passage(
            sky,
            &pack,
            &cid(LANE_ROUTE_OUT),
            &drive,
            DRY_KG,
            OUT_PAYLOAD,
            1,
            depart,
        )
        .expect("plans")
    };
    let constant = plan(&still);
    let first = plan(&moving);
    let second = plan(&moving);
    assert_eq!(first, second, "two runs are bit-equal");
    assert!(
        first.total_s > constant.total_s,
        "the half-synodic plan ({}) is longer than the constant sky's ({})",
        first.total_s,
        constant.total_s
    );
    assert_eq!(
        first.burn_t, constant.burn_t,
        "the Coast burn is distance-free"
    );
    assert_ne!(first.sky_rev, constant.sky_rev);

    let pairs = body_pairs(&pack, LANE_ROUTE_OUT);
    assert_eq!(pairs.len(), first.legs.len());
    for ((a, b, _), leg) in pairs.iter().zip(&first.legs) {
        let passes = intercept_passes(
            &moving,
            a,
            b,
            leg.t0,
            &drive,
            OUT_PAYLOAD,
            INTERCEPT_PASSES + 1,
        )
        .expect("passes");
        let s: Vec<i64> = passes.iter().map(|(_, secs)| i64::from(*secs)).collect();
        assert_eq!(s.len(), 4);
        assert_eq!(
            s[2],
            i64::from(leg.secs),
            "{a} -> {b}: the planner stops at pass three"
        );
        assert!(
            (s[3] - s[2]).abs() <= (s[2] - s[1]).abs() + 1,
            "{a} -> {b}: {s:?} does not contract"
        );
        assert!(
            (s[2] - s[1]).abs() <= (s[1] - s[0]).abs() + 1,
            "{a} -> {b}: {s:?} does not contract"
        );
        assert!(
            (s[3] - s[2]).abs() <= s[2] / 50,
            "{a} -> {b}: {s:?}: a fourth pass moves the leg more than 2 %"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Route lists and the fastest route
// ---------------------------------------------------------------------------

/// The lane with a Pallas place, a detour route through it and a twin of the
/// outbound route, re-signed and decoded through `rules_pack_of`.
fn lane_with_choices() -> RulesPack {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is json");
    let entries = doc["entries"].as_object_mut().expect("entries");
    entries.insert(
        "pallas_rock".to_owned(),
        serde_json::json!({
            "kind": "location",
            "def": { "label": "Pallas Rock", "role": "depot", "body": "pallas" }
        }),
    );
    let mut long = entries[LANE_ROUTE_OUT].clone();
    long["def"]["legs"][0]["to"] = Value::String("pallas_rock".to_owned());
    entries.insert("ceres_stickney_long".to_owned(), long);
    let twin = entries[LANE_ROUTE_OUT].clone();
    entries.insert("ceres_stickney_twin".to_owned(), twin);
    doc["manifest"]["digest"] = Value::String(UNSIGNED.to_owned());
    let signed = sign(&serde_json::to_string(&doc).expect("json"));
    rules_pack_of(mush_content::parse_pack(&signed).expect("the test pack parses"))
        .expect("the test pack decodes")
}

/// **A port's routes are a list and the planner picks the fastest** (D-18-47): a
/// bare-string `routes` value is refused by the wire; a slower route listed first
/// loses; two identical routes tie to the earlier listing; `via` is reserved
/// (`+ship/plan stickney via=…` is `tests/voyage.rs`'s).
#[test]
fn port_routes_are_a_list_and_the_planner_picks_the_fastest() {
    // The wire: a bare string where the list belongs.
    let doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is json");
    let mut port = doc["entries"]["ceres_port"]["def"].clone();
    serde_json::from_value::<PortDef>(port.clone()).expect("the shipped port decodes");
    port["routes"]["stickney"] = Value::String(LANE_ROUTE_OUT.to_owned());
    assert!(serde_json::from_value::<PortDef>(port).is_err());

    let pack = lane_with_choices();
    let sky = sky_at(&pack, 0);
    let drive = dray();
    let depart = lane_epoch(&pack);
    let choose = |routes: &[&str], via: Option<&CanonicalId>| {
        let routes: Vec<CanonicalId> = routes.iter().map(|r| cid(r)).collect();
        choose_route(
            &sky,
            &pack,
            &routes,
            &drive,
            DRY_KG,
            OUT_PAYLOAD,
            1,
            depart,
            via,
        )
    };
    let long = choose(&["ceres_stickney_long"], None).expect("the detour plans");
    let fast = choose(&[LANE_ROUTE_OUT], None).expect("the direct route plans");
    assert!(long.total_s > fast.total_s, "the detour is slower");
    let picked = choose(&["ceres_stickney_long", LANE_ROUTE_OUT], None).expect("plans");
    assert_eq!(
        picked.route.as_str(),
        LANE_ROUTE_OUT,
        "the slower route listed first loses"
    );
    let tie = choose(&["ceres_stickney_twin", LANE_ROUTE_OUT], None).expect("plans");
    assert_eq!(
        tie.route.as_str(),
        "ceres_stickney_twin",
        "a tie keeps the earlier"
    );
    let tie = choose(&[LANE_ROUTE_OUT, "ceres_stickney_twin"], None).expect("plans");
    assert_eq!(
        tie.route.as_str(),
        LANE_ROUTE_OUT,
        "a tie keeps the earlier"
    );
    assert_eq!(
        choose(&[LANE_ROUTE_OUT], Some(&cid("kirkwood_depot"))),
        Err(PlanError::ViaReserved)
    );
    assert!(matches!(
        choose(&[], None),
        Err(PlanError::UnknownRoute { .. })
    ));
    // A listed route that cannot be planned refuses; it is never skipped (L22).
    assert!(matches!(
        choose(&[LANE_ROUTE_OUT, "nowhere"], None),
        Err(PlanError::UnknownRoute { .. })
    ));
}
