//! **Doc 19 cut A.4 acceptance — the reach premium** (doc 19 §6, D-19-14; doc 18
//! Q-18-39).
//!
//! On the production composition with the game on (lane v5 latched at boot): the
//! two ports carry `premium { k 300, cap 500 }`; at the constant sky (rate 0,
//! every body at the lane's epoch) the reach ratio is one and every bid posts at
//! its authored price; at real motion from a test world's own date the bids post
//! at the price the premium function gives for the planner's reach — the same
//! function the applier and the stock clerk call — and never below the authored
//! price. Nothing is minted: the bid still draws on the house's capital.

use std::sync::Arc;

use mush_core::CanonicalId;
use mush_market::{ListingId, MarketStore};
use mush_testkit::TestWorld;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::{
    MAX_SCAN_DAYS, PortDef, PortMapState, bid_price_major, median_of, premium_permille, priced,
    reach_for, scan_days, synodic_days,
};
use tacsim::{EphemerisConfig, Sky, TacsimConfig, TacsimStore, fixture};

/// The lane's epoch, 2351-05-13 (doc 19 §3.3).
const LANE_EPOCH_IC: i64 = 74_152_540_800;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// The production roster (`tests/lane.rs`'s copy).
fn production_roster(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

fn config(rate: u32) -> TacsimConfig {
    let mut cfg = fixture::config();
    cfg.seeds = vec![PackSource {
        name: "<built-in tacsim.corridor>".to_owned(),
        bytes: fixture::CORRIDOR_JSON.to_owned(),
    }];
    cfg.time.ephemeris = EphemerisConfig {
        epoch: LANE_EPOCH_IC,
        rate,
    };
    cfg
}

/// **The scan is the synodic period of the port's body and its supplier's**
/// (doc 19 §6): Ceres's and Mars's periods read from the tables (4.60 and 1.88
/// years), their synodic period about 1,162 days, the scan that many days;
/// Kirkwood rides Ceres's rows, so the pair never closes and the scan is capped;
/// the lower median.
#[test]
fn a4_the_scan_is_the_synodic_period_of_the_port_and_its_supplier() {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    let sky = Sky::from_pack(&pack, &EphemerisConfig::default());
    let ceres = sky
        .period_days(&cid("ceres"), LANE_EPOCH_IC)
        .expect("ceres has a period");
    let mars = sky
        .period_days(&cid("mars"), LANE_EPOCH_IC)
        .expect("mars has a period");
    assert!((ceres - 1_681.6).abs() < 17.0, "Ceres: {ceres} days");
    assert!((mars - 687.0).abs() < 7.0, "Mars: {mars} days");
    let synodic = synodic_days(ceres, mars).expect("the pair closes");
    assert!((synodic - 1_162.0).abs() < 40.0, "synodic: {synodic} days");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let want = synodic.ceil() as u32;
    assert_eq!(
        scan_days(&sky, &cid("mars"), &cid("ceres"), LANE_EPOCH_IC),
        want
    );
    assert_eq!(
        scan_days(&sky, &cid("ceres"), &cid("kirkwood"), LANE_EPOCH_IC),
        MAX_SCAN_DAYS,
        "the same orbit never closes"
    );
    assert_eq!(
        scan_days(&sky, &cid("ceres"), &cid("nowhere"), LANE_EPOCH_IC),
        MAX_SCAN_DAYS,
        "an unknown body"
    );
    assert_eq!(median_of(&[4, 1, 3, 2]), Some(2));
}

/// The shipped lane's port entry.
fn port_def(entry: &str) -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port: {other:?}"),
    }
}

/// The posted Wanted listing of `org`'s bid for `good` at `port`, and its price in
/// major units.
fn posted_bid(tw: &TestWorld, port: &str, org: &str, good: &str) -> (ListingId, u64) {
    let store = tw
        .world()
        .module_state::<TacsimStore>()
        .expect("the layer's state");
    let PortMapState::Present(map) = store.port_map(&cid(LANE_PACK), &cid(port)) else {
        panic!("{port}'s map is present");
    };
    let row = &map.vendors[&cid(org)].bids[&cid(good)];
    let listing = ListingId(row.listing);
    let market = tw
        .world()
        .module_state::<MarketStore>()
        .expect("the market");
    let price = market
        .listing(listing)
        .unwrap_or_else(|| panic!("{org}'s {good} bid is posted"))
        .price
        .minor();
    let scale = 10_i64.pow(u32::from(tacsim::CREDIT_SCALE));
    assert_eq!(price % scale, 0, "{org}'s {good} bid is whole major units");
    (listing, u64::try_from(price / scale).expect("major units"))
}

/// The authored bid for `good` among `def`'s vendors.
fn authored_bid<'a>(def: &'a PortDef, good: &str) -> &'a tacsim::port::BidDef {
    def.vendors
        .iter()
        .flat_map(|vendor| vendor.bids.iter())
        .find(|bid| bid.good.as_str() == good)
        .unwrap_or_else(|| panic!("an authored {good} bid"))
}

/// **Both ports carry the premium block, and the constant sky posts authored
/// prices** (D-19-14): at rate 0 every body stands at the lane's epoch, so the
/// reach now is its own median, the premium is zero, and Kasei's metals bid and
/// the Exchange's alloys bid post at 280 and 520.
#[test]
fn a4_the_constant_sky_posts_the_authored_prices() {
    for entry in ["ceres_port", "stickney_port"] {
        let def = port_def(entry);
        let premium = def.premium.expect("lane v5 authors a premium");
        assert_eq!(
            (premium.k_permille, premium.cap_permille),
            (300, 500),
            "{entry}"
        );
    }
    let tw = TestWorld::with_modules(production_roster(config(0)));
    let (_, metals) = posted_bid(&tw, "stickney_port", "kasei", "metals");
    assert_eq!(metals, 280, "Kasei's metals bid at the epoch sky");
    let (_, alloys) = posted_bid(&tw, "ceres_port", "freeport_exchange", "structural_alloys");
    assert_eq!(alloys, 520, "the Exchange's alloys bid at the epoch sky");
    let stickney = port_def("stickney_port");
    let reach = reach_for(tw.world(), &stickney, &cid("metals")).expect("Ceres supplies metals");
    assert_eq!(reach.now_s, reach.median_s, "rate 0: every day is today");
    assert_eq!(
        bid_price_major(tw.world(), &stickney, authored_bid(&stickney, "metals")),
        280
    );
}

/// **Under real motion the bids post the premium the planner's reach gives**
/// (D-19-14): the posted price is `priced(authored, premium(k, cap, reach))` for
/// the reach `reach_for` reports — Kasei's metals and volatiles bids (the
/// Exchange and Occator ask them on Ceres), the Exchange's alloys and the Co-op's
/// parts bids (Kasei and the Yards ask them at Stickney) — never below the
/// authored price, never above the cap.
#[test]
fn a4_the_moving_sky_posts_the_premium_of_the_planners_reach() {
    let tw = TestWorld::with_modules(production_roster(config(1_000)));
    let stickney = port_def("stickney_port");
    let ceres = port_def("ceres_port");
    for (port, def, org, good, authored) in [
        ("stickney_port", &stickney, "kasei", "metals", 280_u64),
        ("stickney_port", &stickney, "kasei", "volatiles", 200),
        (
            "ceres_port",
            &ceres,
            "freeport_exchange",
            "structural_alloys",
            520,
        ),
        ("ceres_port", &ceres, "coop", "industrial_parts", 560),
    ] {
        let (_, posted) = posted_bid(&tw, port, org, good);
        let reach = reach_for(tw.world(), def, &cid(good))
            .unwrap_or_else(|| panic!("{port} {good}: a supplier is listed"));
        let premium = premium_permille(300, 500, reach);
        let want = priced(authored, premium);
        println!(
            "{port} {org} {good}: reach now {} s, median {} s, premium {premium} permille, \
             authored {authored}, posted {posted}",
            reach.now_s, reach.median_s
        );
        assert_eq!(
            posted, want,
            "{port} {org} {good}: reach {reach:?}, premium {premium}"
        );
        assert!(
            posted >= authored,
            "{port} {good}: never below the authored price"
        );
        assert!(
            posted <= priced(authored, 500),
            "{port} {good}: never above the cap"
        );
        assert_eq!(
            bid_price_major(tw.world(), def, authored_bid(def, good)),
            want
        );
    }
}

/// **A port without a premium block posts its authored prices** whatever the sky:
/// the shipped Stickney entry with `premium` removed, through the pure function.
#[test]
fn a4_a_port_without_a_premium_posts_authored_prices() {
    let tw = TestWorld::with_modules(production_roster(config(1_000)));
    let mut stickney = port_def("stickney_port");
    stickney.premium = None;
    let bid = authored_bid(&stickney, "metals");
    assert_eq!(bid_price_major(tw.world(), &stickney, bid), bid.price_major);
    // And the wire refuses a premium past its bound.
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("json");
    doc["entries"]["stickney_port"]["def"]["premium"]["k_permille"] = serde_json::json!(10_001);
    let def: Result<PortDef, _> =
        serde_json::from_value(doc["entries"]["stickney_port"]["def"].clone());
    let def = def.expect("the shape decodes");
    assert!(def.check().is_err(), "k past 10,000 permille is refused");
}
