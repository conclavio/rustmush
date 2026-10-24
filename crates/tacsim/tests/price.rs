//! **Doc 19 cuts A.4 and A.4b acceptance — the reach premium and the emergency
//! bids** (doc 19 §6, D-19-14; doc 18 Q-18-39).
//!
//! On the production composition with the game on (the shipped lane latched at
//! boot): the five ports carry `premium { k 300, cap 500 }` and an `emergency`
//! block (this suite reads Ceres's and Stickney's; lane v8's moon ports are
//! `tests/moon_ports.rs`'s); at the constant sky (rate 0, every body at the
//! lane's epoch) the reach ratio is one, every bid posts at its authored price
//! and no emergency bid posts; the old ports' suppliers under lane v8 are the
//! v7 graph's plus the Deimos hop to Stickney (`a8_…`); at real motion from a test world's own date the bids post at the price
//! the premium function gives for the planner's reach — the same function the
//! applier and the stock clerk call — and never below the authored price; a lane
//! whose ratio the reach crosses has the clerk post the emergency bids at the
//! cap, and a revision whose ratio it does not has the clerk withdraw them.
//! Nothing is minted: a bid draws on the house's capital.

mod support;

use std::sync::Arc;

use mush_core::{CanonicalId, GOD, ObjId};
use mush_market::{ListingId, MarketStore};
use mush_testkit::{TestWorld, fold_module_ops};
use serde_json::json;
use support::ground::{UNSIGNED, sign};
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION, game_seed_sources};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::{
    MAX_SCAN_DAYS, PortDef, PortMap, PortMapState, bid_price_major, emergency_readings, median_of,
    premium_permille, priced, reach_for, scan_days, synodic_days,
};
use tacsim::{CREDIT_SCALE, EphemerisConfig, Sky, TacsimConfig, TacsimStore, fixture};

/// The lane's epoch, 2351-05-13 (doc 19 §3.3).
const LANE_EPOCH_IC: i64 = 74_152_540_800;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// A test shelf with the ten station packs ahead of it (`tests/hulls.rs`'s copy):
/// the `PortApplier` resolves the vendors' storefront items and signatories from
/// the station packs on the atlas shelf.
fn with_stations(shelf: &[mush_content::Pack]) -> Vec<mush_content::Pack> {
    let mut full = tacsim::definitions::content::station_packs().expect("the station packs");
    full.extend(shelf.iter().cloned());
    full
}

/// How a test roster boots its lane.
enum Lane<'a> {
    /// The shipped lane, as production.
    Shipped,
    /// A lane booted instead of the shipped one — an operator seed with the
    /// game's other sources beside it, and the content module's boot list with
    /// the shipped lane swapped for it (`tests/commerce.rs`'s `seed_lane`). With
    /// `shelf`, the atlas shelf offers it for `@atlas/pack plan` too, so a
    /// later version than the latched one is taken as an update.
    Seeded { json: &'a str, shelf: bool },
}

/// The production roster (`tests/lane.rs`'s copy), its lane as `lane` says.
fn roster(mut cfg: TacsimConfig, lane: &Lane<'_>) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let game_content = matches!(lane, Lane::Shipped);
    if let Lane::Seeded { json, .. } = lane {
        cfg.seeds.extend(
            game_seed_sources()
                .into_iter()
                .filter(|source| !source.name.contains("lane/ceres_phobos")),
        );
        cfg.seeds.push(PackSource {
            name: "ops/lane.json".to_owned(),
            bytes: (*json).to_owned(),
        });
    }
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    if let Lane::Seeded { json, shelf: true } = lane {
        let pack = mush_content::load_pack(json).expect("the seeded lane verifies");
        let at = mods
            .iter()
            .position(|m| m.name() == mush_atlas::NS)
            .expect("the atlas composes");
        mods[at] = Box::new(
            mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                .with_shelf(with_stations(&[pack])),
        );
    }
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module_wired(cfg, game_content, inventory));
    match lane {
        Lane::Shipped => {
            mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        }
        Lane::Seeded { json, .. } => {
            let packs = support::boot_packs_with_lane(
                mush_content::load_pack(json).expect("the seeded lane verifies"),
            );
            let mut appliers = mush_content::Appliers::new();
            mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
            mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
            tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
            mods.push(Box::new(
                mush_content::ContentModule::new()
                    .with_appliers(appliers)
                    .with_boot_packs(packs),
            ));
        }
    }
    mods
}

fn production_roster(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    roster(cfg, &Lane::Shipped)
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

/// The shipped lane's port entry.
fn port_def(entry: &str) -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port: {other:?}"),
    }
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = json!(version);
    doc["manifest"]["digest"] = json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// The lane at `version` with both ports' emergency ratio at `ratio`, and
/// Stickney's vendors stripped of their stock rows and asks: this rig lands no
/// authoritative submission, so a clerk with a stock mint in flight can never
/// act again — Stickney's clerk is left nothing to mint, while Ceres keeps its
/// asks, which are Stickney's suppliers; the Co-op's margin references that read
/// Stickney's asks go with them. The emergency references move with the asks:
/// propellant to the goods table's 40, provisions to its 10.
fn stickney_variant(version: u32, ratio: u32) -> String {
    lane_variant(version, |doc| {
        // Doc 19 cut A.8: without the moons' trade Ceres is again Stickney's only
        // supplier (Deimos's three-hour hop would make its provisions and
        // propellant readings a coin flip at ratio 1,000).
        support::moon::strip_moon_trade(doc);
        for entry in ["ceres_port", "stickney_port"] {
            doc["entries"][entry]["def"]["emergency"]["ratio_permille"] = json!(ratio);
        }
        let vendors = doc["entries"]["stickney_port"]["def"]["vendors"]
            .as_array_mut()
            .expect("Stickney's vendors");
        for vendor in vendors {
            vendor["stock"] = json!([]);
            vendor["asks"] = json!([]);
        }
        // The Co-op's margin references that read Stickney's asks go with them.
        let reference = doc["entries"]["ceres_port"]["def"]["programme"]["reference"]
            .as_object_mut()
            .expect("the programme's references");
        reference.retain(|_, listing| {
            !listing
                .as_str()
                .is_some_and(|listing| listing.starts_with("stickney_port/"))
        });
    })
}

/// A port entry of a variant's JSON.
fn variant_port_def(json: &str, entry: &str) -> PortDef {
    let doc: serde_json::Value = serde_json::from_str(json).expect("json");
    serde_json::from_value(doc["entries"][entry]["def"].clone()).expect("a port def")
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the layer's state")
}

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world()
        .module_state::<MarketStore>()
        .expect("the market")
}

fn port_map(tw: &TestWorld, port: &str) -> PortMap {
    let PortMapState::Present(map) = store(tw).port_map(&cid(LANE_PACK), &cid(port)) else {
        panic!("{port}'s map is present");
    };
    map.clone()
}

/// `secs` on, and the due ticks fired.
fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    tw.seen_by(&events, who).join("\n")
}

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane.
fn plan_and_apply(tw: &mut TestWorld) -> (String, String) {
    let plan = run(tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let apply = run(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    (plan, apply)
}

/// The posted Wanted listing of `org`'s bid for `good` at `port`, and its price in
/// major units.
fn posted_bid(tw: &TestWorld, port: &str, org: &str, good: &str) -> (ListingId, u64) {
    let map = port_map(tw, port);
    let row = &map.vendors[&cid(org)].bids[&cid(good)];
    let listing = ListingId(row.listing);
    let price = market(tw)
        .listing(listing)
        .unwrap_or_else(|| panic!("{org}'s {good} bid is posted"))
        .price
        .minor();
    let scale = 10_i64.pow(u32::from(CREDIT_SCALE));
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

// ---------------------------------------------------------------------------
// A.4: the reach premium
// ---------------------------------------------------------------------------

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
    stickney.emergency = None;
    let bid = authored_bid(&stickney, "metals");
    assert_eq!(bid_price_major(tw.world(), &stickney, bid), bid.price_major);
    // And the wire refuses a premium past its bound.
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("json");
    doc["entries"]["stickney_port"]["def"]["premium"]["k_permille"] = json!(10_001);
    let def: Result<PortDef, _> =
        serde_json::from_value(doc["entries"]["stickney_port"]["def"].clone());
    let def = def.expect("the shape decodes");
    assert!(def.check().is_err(), "k past 10,000 permille is refused");
}

// ---------------------------------------------------------------------------
// A.4b: the emergency bids
// ---------------------------------------------------------------------------

/// **Lane v6 authors the emergency block on both ports, and `check` bounds it**
/// (doc 19 §6, cut A.4b): the Exchange at Ceres and the Yards at Stickney bid for
/// provisions, volatiles and propellant at 1,500 ‰ of the median reach, 100 units
/// deep; refused: a ratio under 1,000 ‰ or past the bound, no goods, a good named
/// twice, an intake of zero, an org that is not a vendor, an org that already asks
/// or bids one of the goods, an emergency without a premium, a good the goods
/// table does not know.
#[test]
fn a4b_the_lane_authors_the_emergency_block_and_check_bounds_it() {
    for (entry, org) in [
        ("ceres_port", "freeport_exchange"),
        ("stickney_port", "stickney_yards"),
    ] {
        let def = port_def(entry);
        let emergency = def
            .emergency
            .clone()
            .expect("lane v6 authors an emergency block");
        assert_eq!(emergency.org, cid(org), "{entry}");
        assert_eq!(emergency.ratio_permille, 1_500, "{entry}");
        assert_eq!(
            emergency.goods,
            vec![cid("provisions"), cid("volatiles"), cid("propellant")],
            "{entry}"
        );
        assert_eq!(emergency.intake, 100, "{entry}");
        assert!(def.check().is_ok(), "{entry}");
    }
    let base = port_def("stickney_port");
    let refused = |name: &str, edit: &dyn Fn(&mut PortDef), words: &str| {
        let mut def = base.clone();
        edit(&mut def);
        let error = def.check().expect_err(&format!("{name} is refused"));
        assert!(error.contains(words), "{name}: {error}");
    };
    refused(
        "ratio 999",
        &|def| def.emergency.as_mut().expect("block").ratio_permille = 999,
        "ratio_permille",
    );
    refused(
        "ratio past the bound",
        &|def| def.emergency.as_mut().expect("block").ratio_permille = 100_001,
        "ratio_permille",
    );
    refused(
        "no goods",
        &|def| def.emergency.as_mut().expect("block").goods.clear(),
        "goods",
    );
    refused(
        "a good twice",
        &|def| {
            def.emergency
                .as_mut()
                .expect("block")
                .goods
                .push(cid("provisions"));
        },
        "named twice",
    );
    refused(
        "intake 0",
        &|def| def.emergency.as_mut().expect("block").intake = 0,
        "intake",
    );
    refused(
        "an org that is not a vendor",
        &|def| def.emergency.as_mut().expect("block").org = cid("nobody"),
        "not one of this port's vendors",
    );
    refused(
        "an org that asks a good",
        &|def| def.emergency.as_mut().expect("block").org = cid("kasei"),
        "already asks propellant",
    );
    refused(
        "an emergency without a premium",
        &|def| def.premium = None,
        "needs a premium",
    );
    let unknown = lane_variant(LANE_VERSION, |doc| {
        doc["entries"]["stickney_port"]["def"]["emergency"]["goods"] =
            json!(["provisions", "unobtainium"]);
    });
    assert!(
        load_pack(&unknown).is_err(),
        "a good the goods table does not know is refused at load"
    );
}

/// **The constant sky posts no emergency bids** (D-19-14): at rate 0 the reach is
/// its own median, so no good crosses the ratio; after the clerk's fires neither
/// port map records an emergency bid, and the readings say so — every Stickney
/// good has a reach (Ceres supplies volatiles; Ceres and, since lane v8, the
/// Deimos hop supply provisions and propellant, which
/// `a8_the_old_ports_suppliers_move_as_designed` pins), Ceres's volatiles has
/// none (no port that lists a route to Ceres asks volatiles). The prices the
/// bids would post at are the port's own highest authored price for the good
/// under the cap.
#[test]
fn a4b_the_constant_sky_posts_no_emergency_bids() {
    let mut tw = TestWorld::with_modules(production_roster(config(0)));
    for _ in 0..4 {
        sweep(&mut tw, 3_600);
    }
    for port in ["ceres_port", "stickney_port"] {
        assert!(
            port_map(&tw, port).emergencies.is_empty(),
            "{port} posted an emergency bid under the constant sky"
        );
    }
    let stickney = port_def("stickney_port");
    let readings = emergency_readings(tw.world(), &stickney);
    assert_eq!(readings.len(), 3);
    for reading in &readings {
        let reach = reading.reach.expect("a supplier is listed");
        assert_eq!(reach.now_s, reach.median_s, "{}", reading.good);
        assert!(!reading.crossed, "{}", reading.good);
    }
    let ceres = port_def("ceres_port");
    let readings = emergency_readings(tw.world(), &ceres);
    let volatiles = readings
        .iter()
        .find(|r| r.good.as_str() == "volatiles")
        .expect("a volatiles reading");
    assert_eq!(
        volatiles.reach, None,
        "no port with a route to Ceres asks volatiles"
    );
    assert!(!volatiles.crossed);
    // Stickney's volatiles 200 → 300, provisions 10 → 15, propellant 60 → 90;
    // Ceres's provisions 10 → 15, propellant 40 → 60.
    let price = |def: &PortDef, good: &str| {
        emergency_readings(tw.world(), def)
            .into_iter()
            .find(|r| r.good.as_str() == good)
            .and_then(|r| r.price_major)
    };
    assert_eq!(price(&stickney, "volatiles"), Some(300));
    assert_eq!(price(&stickney, "provisions"), Some(15));
    assert_eq!(price(&stickney, "propellant"), Some(90));
    assert_eq!(price(&ceres, "provisions"), Some(15));
    assert_eq!(price(&ceres, "propellant"), Some(60));
}

/// **The clerk posts the emergency bids at the cap when the reach crosses the
/// ratio, and withdraws them when it falls back** (doc 19 §6, cut A.4b): a lane
/// at 1,000 ‰ (every reach is at or past its median) booted under the moving sky
/// has Stickney's clerk post, one a fire, a Wanted of the Yards' vendor for each
/// of the three goods — 100 units at the reference under the cap (propellant 40
/// → 60, provisions 10 → 15, volatiles 200 → 300) — and record it in the map;
/// nothing is minted (the house's capital row is unchanged). A reboot into a
/// revision at 100,000 ‰, taken as an update, has the clerk withdraw them, one a
/// fire, and forget the rows. (Ceres's clerk has stock mints in flight this rig
/// never lands, so it defers; see `stickney_variant`.)
#[test]
fn a4b_the_clerk_posts_emergency_bids_at_the_cap_and_withdraws_them_when_reach_falls_back() {
    let crossed = stickney_variant(LANE_VERSION, 1_000);
    let lane = Lane::Seeded {
        json: &crossed,
        shelf: false,
    };
    let mut tw = TestWorld::with_modules(roster(config(1_000), &lane));
    assert!(
        port_map(&tw, "stickney_port").emergencies.is_empty(),
        "nothing posts before the clerk's first fire"
    );
    let capital_before = port_map(&tw, "stickney_port").orgs.clone();
    for _ in 0..6 {
        sweep(&mut tw, 3_600);
    }
    let map = port_map(&tw, "stickney_port");
    let expected = [
        ("propellant", 40_u64),
        ("provisions", 10),
        ("volatiles", 200),
    ];
    let posted: Vec<String> = map.emergencies.keys().map(ToString::to_string).collect();
    let want: Vec<String> = expected
        .iter()
        .map(|(good, _)| (*good).to_owned())
        .collect();
    assert_eq!(posted, want, "the emergency bids posted");
    let vendor = map.vendors[&cid("stickney_yards")]
        .vendor
        .expect("the Yards' vendor");
    let scale = 10_i64.pow(u32::from(CREDIT_SCALE));
    for (good, reference) in &expected {
        let row = &map.emergencies[&cid(good)];
        assert_eq!(row.price_major, *reference, "{good}: the reference");
        assert_eq!(row.intake, 100, "{good}");
        let listing = market(&tw)
            .listing(ListingId(row.listing))
            .unwrap_or_else(|| panic!("{good}: the emergency bid stands"));
        assert!(listing.source.is_wanted(), "{good}");
        assert_eq!(
            listing.vendor.0, vendor,
            "{good}: the Yards' vendor posts it"
        );
        assert_eq!(listing.quantity, 100, "{good}");
        let want_minor = i64::try_from(priced(*reference, 500)).expect("fits") * scale;
        assert_eq!(listing.price.minor(), want_minor, "{good}: the cap");
    }
    let def = variant_port_def(&crossed, "stickney_port");
    let readings = emergency_readings(tw.world(), &def);
    assert_eq!(readings.len(), 3);
    assert!(readings.iter().all(|r| r.crossed), "{readings:?}");
    assert_eq!(
        port_map(&tw, "stickney_port").orgs,
        capital_before,
        "nothing minted: the capital rows stand"
    );

    // The reach falls back: a revision at 100,000 permille, taken as an update
    // after a reboot that keeps the maps and the market.
    let fallen = stickney_variant(LANE_VERSION + 1, 100_000);
    let lane = Lane::Seeded {
        json: &fallen,
        shelf: true,
    };
    let store = fold_module_ops(tw.take_module_ops());
    let mut tw = tw
        .reboot_into(roster(config(1_000), &lane), store)
        .expect("the reboot boots");
    let standing_before: Vec<u64> = port_map(&tw, "stickney_port")
        .emergencies
        .values()
        .map(|row| row.listing)
        .collect();
    assert_eq!(standing_before.len(), 3, "the rows survive the reboot");
    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    for _ in 0..6 {
        sweep(&mut tw, 3_600);
    }
    assert!(
        port_map(&tw, "stickney_port").emergencies.is_empty(),
        "the emergency bids are withdrawn and forgotten"
    );
    for listing in standing_before {
        assert!(
            market(&tw).listing(ListingId(listing)).is_none(),
            "#{listing} is withdrawn"
        );
    }
}

// ---------------------------------------------------------------------------
// A.8: the moon ports and the old ports' suppliers
// ---------------------------------------------------------------------------

/// **The old ports' suppliers move as designed** (doc 19 §6's supplier table,
/// D-19-23): on the shipped lane v8 at the constant sky, Stickney's emergency
/// provisions and propellant readings take the Deimos hop (Voltaire Anchorage
/// asks both and lists `voltaire_stickney`), a reach under five IC hours where
/// the v7-shaped lane (the same bytes with `support::moon::strip_moon_trade`) has
/// Ceres's passage; Stickney's volatiles reading keeps Ceres's reach; Ceres's
/// volatiles reading has none; Ceres's provisions and propellant readings and
/// the four old bids — Kasei's metals and volatiles at Stickney, the Exchange's
/// alloys and the Co-op's parts at Ceres — read the reach they read on v7 (Luna
/// asks alloys but lists no `ceres`; no moon port lists `stickney` but Deimos,
/// which asks neither metals nor volatiles).
#[test]
fn a8_the_old_ports_suppliers_move_as_designed() {
    const HOP_CEILING_S: i64 = 5 * 3_600;
    let shipped = TestWorld::with_modules(production_roster(config(0)));
    let stripped = lane_variant(LANE_VERSION, |doc| {
        // Doc 20 cut SM.2: the v7 shape has no sites either.
        support::sites::strip_sites(doc);
        support::moon::strip_moon_trade(doc);
    });
    let lane = Lane::Seeded {
        json: &stripped,
        shelf: false,
    };
    let v7 = TestWorld::with_modules(roster(config(0), &lane));
    let reading = |tw: &TestWorld, def: &PortDef, good: &str| {
        emergency_readings(tw.world(), def)
            .into_iter()
            .find(|r| r.good.as_str() == good)
            .unwrap_or_else(|| panic!("{}: a {good} reading", def.venue))
            .reach
    };
    for (entry, good, hop) in [
        ("stickney_port", "provisions", true),
        ("stickney_port", "propellant", true),
        ("stickney_port", "volatiles", false),
        ("ceres_port", "provisions", false),
        ("ceres_port", "propellant", false),
        ("ceres_port", "volatiles", false),
    ] {
        let now = reading(&shipped, &port_def(entry), good);
        let then = reading(&v7, &variant_port_def(&stripped, entry), good);
        println!("{entry} emergency {good}: v8 {now:?}, v7 {then:?}");
        if entry == "ceres_port" && good == "volatiles" {
            assert_eq!(now, None, "Ceres's volatiles: no supplier on v8");
            assert_eq!(then, None, "Ceres's volatiles: no supplier on v7");
            continue;
        }
        let now = now.unwrap_or_else(|| panic!("{entry} {good}: a v8 supplier"));
        let then = then.unwrap_or_else(|| panic!("{entry} {good}: a v7 supplier"));
        assert!(
            then.now_s > HOP_CEILING_S,
            "{entry} {good}: v7's supplier is Ceres or Stickney, days away: {then:?}"
        );
        if hop {
            assert!(
                now.now_s < HOP_CEILING_S,
                "{entry} {good}: the Deimos hop supplies it: {now:?}"
            );
            assert_eq!(now.now_s, now.median_s, "{entry} {good}: rate 0");
        } else {
            assert_eq!(now, then, "{entry} {good}: the v7 reach stands");
        }
    }
    let stickney = port_def("stickney_port");
    let ceres = port_def("ceres_port");
    for (entry, def, good) in [
        ("stickney_port", &stickney, "metals"),
        ("stickney_port", &stickney, "volatiles"),
        ("ceres_port", &ceres, "structural_alloys"),
        ("ceres_port", &ceres, "industrial_parts"),
    ] {
        let now = reach_for(shipped.world(), def, &cid(good))
            .unwrap_or_else(|| panic!("{entry} {good}: a v8 supplier"));
        let v7_def = variant_port_def(&stripped, entry);
        let then = reach_for(v7.world(), &v7_def, &cid(good))
            .unwrap_or_else(|| panic!("{entry} {good}: a v7 supplier"));
        println!("{entry} bid {good}: v8 {now:?}, v7 {then:?}");
        assert_eq!(now, then, "{entry} {good}: the bid's suppliers are v7's");
        assert_eq!(
            bid_price_major(shipped.world(), def, authored_bid(def, good)),
            bid_price_major(v7.world(), &v7_def, authored_bid(&v7_def, good)),
            "{entry} {good}: the posted price is v7's"
        );
    }
}
