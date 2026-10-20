//! **Doc 20 cut SM.2 acceptance — the landing law, the ascent term and the hop** (doc 20 §3.3,
//! §11's Landing and Hop rows; D-20-3; the SM.2 brief's items 5 and 14, lead rulings 3 and 4,
//! decisions 2, 6 and 7).
//!
//! Pure: the shipped lane through `pack::rules_pack_of`, the Dray's drive from the fielded
//! catalog, the sky from `Sky::from_pack` under an explicit ephemeris. No world: the world-side
//! claims (the ticket, the berth, the stores, the recall) live in `tests/sites.rs` and
//! `mush-server/tests/ic_buyout_dormancy.rs`.
//!
//! The figures (Dray: dry 240,000 kg, rail 181,000 kg, `flip {50, 6000}`): thrust-to-weight
//! permille at full throttle, laden (the rail) / empty-holded (80,580 kg: a full 80 t tank, a
//! 480 kg locker, one 100 kg soul) — Ceres 1,785 / 2,345, Vesta 2,000 / 2,626, Pallas 2,380 /
//! 3,126, Phobos 50,000 / 65,662 (all land); Luna 303 / 397, Titan 362 / 475, Mars 131 / 173
//! (all refused). The ascent: Ceres 360 m/s, Vesta 254.

mod support;

use mush_core::CanonicalId;
use serde_json::Value;
use tacsim::definitions::Catalog;
use tacsim::definitions::content::{DRAY, LANE_JSON, LANE_ROUTE_BACK, LANE_ROUTE_OUT};
use tacsim::pack::{PackEntry, RulesPack, rules_pack_of};
use tacsim::port::passage::{
    DriveLaw, LANDING_TWR_PERMILLE, MIN_THROTTLE_PERMILLE, ascent_mps, crew_days,
    landing_twr_permille, least_landing_throttle, passage_burn, route_landing_floor,
};
use tacsim::port::{
    HullDrive, PLAN_WINDOW_DAYS, PassagePlan, PlanError, WINDOW_DAYS, home_reserve, plan_passage,
};
use tacsim::{EphemerisConfig, Sky};

use support::ground::{UNSIGNED, sign};
use support::sites::strip_sites;

/// The Dray's dry mass, kilograms.
const DRY_KG: u64 = 240_000;
/// The Dray's payload rail, kilograms.
const RAIL_KG: u64 = 181_000;
/// An empty hold with a full 80 t tank, a full 480 kg locker and one 100 kg soul.
const EMPTY_HOLD_KG: u64 = 80_000 + 480 + 100;
/// The tank, tonnes.
const TANK_T: u64 = 80;
const DAY: i64 = 86_400;
/// The eight routes of lane v8.
const PORT_ROUTES: [&str; 8] = [
    LANE_ROUTE_OUT,
    LANE_ROUTE_BACK,
    "ceres_aitken",
    "aitken_voltaire",
    "aitken_ligeia",
    "ligeia_aitken",
    "ligeia_voltaire",
    "voltaire_stickney",
];
/// The five Ceres hops, out and back.
const HOPS: [&str; 10] = [
    "ceres_commons",
    "commons_ceres",
    "ceres_kerwan",
    "kerwan_ceres",
    "ceres_ahuna",
    "ahuna_ceres",
    "ceres_juling",
    "juling_ceres",
    "ceres_occator",
    "occator_ceres",
];

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// The shipped lane.
fn lane() -> RulesPack {
    rules_pack_of(mush_content::parse_pack(LANE_JSON).expect("the lane parses"))
        .expect("the lane decodes")
}

/// The lane with `edit` applied to its JSON, re-signed.
fn variant(edit: impl FnOnce(&mut Value)) -> RulesPack {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("json");
    edit(&mut doc);
    doc["manifest"]["digest"] = Value::String(UNSIGNED.to_owned());
    let signed = sign(&serde_json::to_string(&doc).expect("json"));
    rules_pack_of(mush_content::parse_pack(&signed).expect("parses")).expect("decodes")
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

fn epoch(pack: &RulesPack) -> i64 {
    Sky::from_pack(pack, &EphemerisConfig::default())
        .epoch_of(&cid("ceres"))
        .expect("Ceres is in the sky")
}

fn sky_at(pack: &RulesPack, rate: u32) -> Sky {
    Sky::from_pack(
        pack,
        &EphemerisConfig {
            epoch: epoch(pack),
            rate,
        },
    )
}

fn plan_with(
    sky: &Sky,
    pack: &RulesPack,
    route: &str,
    drive: &HullDrive,
    payload: u64,
    depart: i64,
) -> Result<PassagePlan, PlanError> {
    plan_passage(sky, pack, &cid(route), drive, DRY_KG, payload, 1, depart)
}

/// A body's surface gravity from the shipped lane.
fn surface_mg(pack: &RulesPack, body: &str) -> u32 {
    match pack.entry(&cid(body)) {
        Some(PackEntry::Body(def)) => def.surface_mg.expect("surface figures"),
        other => panic!("{body} is a body: {other:?}"),
    }
}

/// `mush-sim`'s laden step, written out: `⌊base × (rail + payload) / rail⌋`.
fn laden(base: u64, payload: u64) -> i64 {
    i64::try_from(base * (RAIL_KG + payload) / RAIL_KG).expect("fits")
}

// ---------------------------------------------------------------------------

/// **The landing law** (D-20-3): thrust-to-weight is `accel_mg × (dry + rail) × throttle /
/// ((dry + payload) × surface_mg)`, floored permille, at the plan's throttle; every figure of
/// the brief's table, read off the shipped lane's bodies; either side of the line at 671 and
/// 672 permille; the least landing throttle is its exact ceiling; a coast drive has none.
#[test]
fn sm2_the_landing_law_reads_thrust_to_weight_at_the_plans_throttle() {
    let pack = lane();
    let full = dray(1_000);
    let twr = |body: &str, payload: u64| {
        landing_twr_permille(&full, DRY_KG, payload, surface_mg(&pack, body)).expect("a TWR")
    };
    for (body, laden, empty, lands) in [
        ("ceres", 1_785, 2_345, true),
        ("vesta", 2_000, 2_626, true),
        ("pallas", 2_380, 3_126, true),
        ("phobos", 50_000, 65_662, true),
        ("deimos", 50_000, 65_662, true),
        ("luna", 303, 397, false),
        ("titan", 362, 475, false),
        ("mars", 131, 173, false),
    ] {
        assert_eq!(
            (twr(body, RAIL_KG), twr(body, EMPTY_HOLD_KG)),
            (laden, empty),
            "{body}"
        );
        assert_eq!(laden >= LANDING_TWR_PERMILLE, lands, "{body}");
        assert_eq!(empty >= LANDING_TWR_PERMILLE, lands, "{body} empty");
    }
    let ceres = surface_mg(&pack, "ceres");
    let vesta = surface_mg(&pack, "vesta");
    assert_eq!(
        landing_twr_permille(&dray(672), DRY_KG, RAIL_KG, ceres),
        Some(1_200)
    );
    assert_eq!(
        landing_twr_permille(&dray(671), DRY_KG, RAIL_KG, ceres),
        Some(1_198)
    );
    assert_eq!(
        landing_twr_permille(&dray(600), DRY_KG, RAIL_KG, vesta),
        Some(1_200)
    );
    assert_eq!(
        landing_twr_permille(&dray(599), DRY_KG, RAIL_KG, vesta),
        Some(1_198)
    );
    assert_eq!(
        least_landing_throttle(&full, DRY_KG, RAIL_KG, ceres),
        Some(672)
    );
    assert_eq!(
        least_landing_throttle(&full, DRY_KG, RAIL_KG, vesta),
        Some(600)
    );
    for body in ["luna", "titan", "mars"] {
        assert_eq!(
            least_landing_throttle(&full, DRY_KG, RAIL_KG, surface_mg(&pack, body)),
            None,
            "{body}"
        );
    }
    let coast = HullDrive {
        law: DriveLaw::Coast {
            kms: 226,
            burn_permille: 105,
        },
        ..full.clone()
    };
    assert_eq!(landing_twr_permille(&coast, DRY_KG, RAIL_KG, ceres), None);
    assert_eq!(least_landing_throttle(&coast, DRY_KG, RAIL_KG, ceres), None);
}

/// Ahuna Flank moved onto `body` (given Ceres's geology table when it carries none, so the
/// validator's site rule holds).
fn ahuna_on(body: &str) -> RulesPack {
    variant(|doc| {
        let table = doc["entries"]["ceres"]["def"]["geology"].clone();
        let entries = doc["entries"].as_object_mut().expect("entries");
        if let Some(def) = entries
            .get_mut(body)
            .and_then(|entry| entry["def"].as_object_mut())
            && !def.contains_key("geology")
        {
            def.insert("geology".to_owned(), table);
        }
        entries["ahuna_flank"]["def"]["body"] = Value::String(body.to_owned());
    })
}

/// **A Luna site refuses by TWR at planning** (§13's acceptance cell): Ahuna Flank moved onto
/// Luna, Titan and Mars refuses `ceres_ahuna` laden and empty-holded with exactly the TWR
/// `landing_twr_permille` reads there; on Vesta, Pallas and Phobos it plans.
#[test]
fn sm2_a_luna_site_refuses_at_planning() {
    for (body, laden, empty) in [("luna", 303, 397), ("titan", 362, 475), ("mars", 131, 173)] {
        let pack = ahuna_on(body);
        let sky = sky_at(&pack, 0);
        let at = epoch(&pack);
        for (payload, want) in [(RAIL_KG, laden), (EMPTY_HOLD_KG, empty)] {
            let refused = plan_with(&sky, &pack, "ceres_ahuna", &dray(1_000), payload, at);
            assert_eq!(
                refused,
                Err(PlanError::Landing {
                    place: "ahuna_flank".to_owned(),
                    body: body.to_owned(),
                    twr_permille: want,
                }),
                "{body} at {payload} kg"
            );
            assert_eq!(
                landing_twr_permille(&dray(1_000), DRY_KG, payload, surface_mg(&pack, body)),
                Some(want),
                "{body}"
            );
        }
    }
    for body in ["vesta", "pallas", "phobos"] {
        let pack = ahuna_on(body);
        let sky = sky_at(&pack, 0);
        let plan = plan_with(
            &sky,
            &pack,
            "ceres_ahuna",
            &dray(1_000),
            EMPTY_HOLD_KG,
            epoch(&pack),
        )
        .unwrap_or_else(|e| panic!("{body} plans: {e}"));
        assert!(plan.burn_t >= 1, "{body}");
    }
}

/// **The hop** (acceptance 1; lead ruling 4): `+ship/plan ahuna` with a full tank, 6 t of hold
/// cargo standing in for one TV, a 120 crew-day locker and one soul — payload 86,580 kg —
/// plans `base_s` through the laden step, `⌊3,600 × 267,580 / 181,000⌋` = 5,322 s, burns one
/// tonne (the ascent's few kilograms, rounded up), one crew-day, the leg 0 km with the Ahuna
/// end's 360 m/s; the hop home with a Dray-P's full 86 t hold, 79 t of tank, 476 kg of locker
/// and one soul (165,576 kg) is the formula's 6,893 s and a tonne too.
#[test]
fn sm2_the_hop_plans_base_s_through_the_laden_step_and_burns_a_tonne() {
    let pack = lane();
    let sky = sky_at(&pack, 0);
    let at = epoch(&pack);
    let payload = 80_000 + 480 + 100 + 6_000;
    assert_eq!(payload, 86_580);
    let out = plan_with(&sky, &pack, "ceres_ahuna", &dray(1_000), payload, at).expect("plans");
    assert_eq!(out.total_s, laden(3_600, payload), "the binding assertion");
    assert_eq!(out.total_s, 5_322);
    assert_eq!((out.burn_t, out.crew_days), (1, 1));
    assert_eq!(out.legs.len(), 1);
    assert_eq!(out.legs[0].dist_km, 0);
    assert_eq!(out.legs[0].ascent_mps, 360);
    assert_eq!(out.landed_crew_days, 0);
    let home_payload = 86_000 + 79_000 + 476 + 100;
    assert_eq!(home_payload, 165_576);
    let home =
        plan_with(&sky, &pack, "ahuna_ceres", &dray(1_000), home_payload, at).expect("plans");
    assert_eq!(home.total_s, laden(3_600, home_payload));
    assert_eq!(home.total_s, 6_893);
    assert_eq!(home.burn_t, 1);
    assert_eq!(home.legs[0].ascent_mps, 360);
}

/// **The ascent term** (decision 2): √(g·R) ceiled, only at a site end — Ceres 360, Vesta 254;
/// the hops and the Rheasilvia passage carry one end's each; a leg between two sites carries
/// both; the ascent only adds burn; every site leg burns at least a tonne.
#[test]
fn sm2_the_ascent_term_is_the_circular_speed_at_each_site_end() {
    assert_eq!(ascent_mps(28, 470), Some(360));
    assert_eq!(ascent_mps(25, 263), Some(254));
    let pack = lane();
    let sky = sky_at(&pack, 0);
    let at = epoch(&pack);
    let drive = dray(1_000);
    for route in HOPS {
        let plan = plan_with(&sky, &pack, route, &drive, EMPTY_HOLD_KG, at).expect(route);
        assert_eq!(plan.legs[0].ascent_mps, 360, "{route}");
        assert!(plan.burn_t >= 1, "{route}");
    }
    for route in ["ceres_rheasilvia", "rheasilvia_ceres"] {
        let plan = plan_with(&sky, &pack, route, &drive, EMPTY_HOLD_KG, at).expect(route);
        assert_eq!(plan.legs[0].ascent_mps, 254, "{route}");
        assert!(plan.burn_t >= 1, "{route}");
        let mut bare = plan.legs.clone();
        for leg in &mut bare {
            leg.ascent_mps = 0;
        }
        let without = passage_burn(&drive, DRY_KG, EMPTY_HOLD_KG, &bare).expect("burns");
        assert!(
            plan.burn_t >= without,
            "{route}: {} against {without}",
            plan.burn_t
        );
    }
    // A hand-built leg between two Ceres sites pays both ends.
    let two = variant(|doc| {
        doc["entries"].as_object_mut().expect("entries").insert(
            "ahuna_kerwan".to_owned(),
            serde_json::json!({
                "kind": "route",
                "def": {
                    "from": "ahuna_flank",
                    "to": "kerwan_floor",
                    "legs": [{ "to": "kerwan_floor", "base_s": 3600 }],
                    "drive_class": "coast_hauler",
                    "propellant": 0
                }
            }),
        );
    });
    let sky = sky_at(&two, 0);
    let plan = plan_with(
        &sky,
        &two,
        "ahuna_kerwan",
        &drive,
        EMPTY_HOLD_KG,
        epoch(&two),
    )
    .expect("plans");
    assert_eq!(plan.legs[0].ascent_mps, 720);
}

/// **No port leg changes** (acceptance 6; lead ruling 3): each of the eight v8 routes plans
/// under v9 exactly as on the v8 shape (`strip_sites`), field for field, laden and empty; the
/// two new plan fields are zero and absent from the plan's JSON; each route's landing floor is
/// the band's own.
#[test]
fn sm2_no_port_leg_changes() {
    let v9 = lane();
    let v8 = variant(|doc| {
        strip_sites(doc);
        doc["manifest"]["version"] = Value::from(8);
    });
    let (sky9, sky8) = (sky_at(&v9, 0), sky_at(&v8, 0));
    let at = epoch(&v9);
    for route in PORT_ROUTES {
        for payload in [RAIL_KG, EMPTY_HOLD_KG] {
            let new = plan_with(&sky9, &v9, route, &dray(1_000), payload, at).expect(route);
            let old = plan_with(&sky8, &v8, route, &dray(1_000), payload, at).expect(route);
            assert_eq!(new, old, "{route} at {payload} kg");
            assert!(new.legs.iter().all(|leg| leg.ascent_mps == 0), "{route}");
            assert_eq!(new.landed_crew_days, 0);
            let json = serde_json::to_value(&new).expect("serializes");
            assert!(json.get("landed_crew_days").is_none(), "{route}");
            assert!(
                json["legs"]
                    .as_array()
                    .expect("legs")
                    .iter()
                    .all(|leg| leg.get("ascent_mps").is_none()),
                "{route}"
            );
            assert_eq!(
                route_landing_floor(&v9, &cid(route), &dray(1_000), DRY_KG, payload),
                Ok(MIN_THROTTLE_PERMILLE),
                "{route}"
            );
        }
    }
}

/// **The burn's prepare recomputes the frozen legs** (T1, D-18-46): `passage_burn` over a site
/// plan's legs is its `burn_t`, and a plan given a landed charge keeps `crew_days(legs) +
/// landed == crew_days` and its own check.
#[test]
fn sm2_the_burn_recomputes_the_frozen_legs() {
    let pack = lane();
    let sky = sky_at(&pack, 0);
    let at = epoch(&pack);
    let drive = dray(1_000);
    for route in [
        "ceres_ahuna",
        "ahuna_ceres",
        "ceres_rheasilvia",
        "rheasilvia_ceres",
    ] {
        let plan = plan_with(&sky, &pack, route, &drive, EMPTY_HOLD_KG, at).expect(route);
        assert_eq!(
            passage_burn(&drive, DRY_KG, EMPTY_HOLD_KG, &plan.legs),
            Ok(plan.burn_t),
            "{route}"
        );
        let mut charged = plan.clone();
        charged.landed_crew_days = 3;
        charged.crew_days += 3;
        assert!(charged.check().is_ok());
        assert_eq!(
            crew_days(1, &charged.legs).map(|days| days + charged.landed_crew_days),
            Ok(charged.crew_days)
        );
        let mut over = plan.clone();
        over.landed_crew_days = plan.crew_days + 1;
        assert!(over.check().is_err(), "the stay is part of the crew-days");
    }
}

/// **Rheasilvia over a year** (decision 7's owner item): each IC day of the window from the
/// epoch under the moving sky, the full-throttle `ceres_rheasilvia` burn at a full tank, full
/// locker, empty hold and one soul, and the reserve's home burn — `rheasilvia_ceres` at the home
/// payload and its landing floor, the most over the 30 IC days after arrival. Printed for the
/// report, with the same count when the passage out is flown at its own landing floor (the
/// least throttle that lands); the epoch day's figures are pinned.
#[test]
fn sm2_rheasilvia_reachability_over_a_year() {
    let pack = lane();
    let sky = sky_at(&pack, 1_000);
    let at = epoch(&pack);
    let full = dray(1_000);
    let out_floor = route_landing_floor(
        &pack,
        &cid("ceres_rheasilvia"),
        &full,
        DRY_KG,
        EMPTY_HOLD_KG,
    )
    .expect("lands");
    let day = |offset: i64, out_drive: &HullDrive| -> (u64, u64, u32) {
        let depart = at + offset * DAY;
        let out = plan_with(
            &sky,
            &pack,
            "ceres_rheasilvia",
            out_drive,
            EMPTY_HOLD_KG,
            depart,
        )
        .expect("out plans");
        let home_payload = EMPTY_HOLD_KG - out.burn_t * 1_000 - u64::from(out.crew_days) * 4;
        let floor =
            route_landing_floor(&pack, &cid("rheasilvia_ceres"), &full, DRY_KG, home_payload)
                .expect("lifts");
        let home = dray(floor);
        let mut worst = 0_u64;
        for k in 0..=30 {
            let leave = depart + out.total_s + k * DAY;
            let plan = plan_with(&sky, &pack, "rheasilvia_ceres", &home, home_payload, leave)
                .expect("home plans");
            worst = worst.max(plan.burn_t);
        }
        (out.burn_t, worst, floor)
    };
    let (out0, home0, floor0) = day(0, &full);
    println!("epoch: out {out0} t at full throttle, reserve {home0} t at throttle={floor0}");
    let slow = dray(out_floor);
    let (slow0, slow_home0, _) = day(0, &slow);
    println!("epoch: out {slow0} t at throttle={out_floor}, reserve {slow_home0} t");
    let (mut fits, mut fits_slow) = (0, 0);
    for offset in 0..WINDOW_DAYS {
        let (out, home, _) = day(offset, &full);
        if out + home <= TANK_T {
            fits += 1;
        }
        let (out, home, _) = day(offset, &slow);
        if out + home <= TANK_T {
            fits_slow += 1;
        }
    }
    println!(
        "days of {WINDOW_DAYS} on which out + reserve <= {TANK_T} t: {fits} at full throttle, {fits_slow} at throttle={out_floor}"
    );
    assert_eq!((out0, home0, floor0), EPOCH_FIGURES);
    assert_eq!((out_floor, slow0, slow_home0), EPOCH_FLOOR_FIGURES);
    // The sweep's reserve is the production one (the SM.2 fixer, the lead's item 5): on the
    // epoch day, `home_reserve` — the function every filing's `site_reserve` calls — over the
    // same sky, home payload and arrival reads the sweep's worst burn and throttle.
    let out = plan_with(&sky, &pack, "ceres_rheasilvia", &full, EMPTY_HOLD_KG, at).expect("out");
    let home_payload = EMPTY_HOLD_KG - out.burn_t * 1_000 - u64::from(out.crew_days) * 4;
    let reserve = home_reserve(
        &sky,
        &pack,
        &[cid("rheasilvia_ceres")],
        &full,
        DRY_KG,
        home_payload,
        1,
        at + out.total_s,
        PLAN_WINDOW_DAYS,
    )
    .expect("the reserve plans");
    assert_eq!((reserve.burn_t, reserve.throttle), (home0, floor0));
    assert_eq!(PLAN_WINDOW_DAYS, 30, "the sweep's window");
}

/// The epoch day's figures `sm2_rheasilvia_reachability_over_a_year` prints: (out t, reserve t,
/// the reserve's throttle).
const EPOCH_FIGURES: (u64, u64, u32) = (64, 37, 366);
/// The same with the passage out at its landing floor: (the floor, out t, reserve t).
const EPOCH_FLOOR_FIGURES: (u32, u64, u64) = (457, 45, 40);
