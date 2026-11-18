//! **Doc 48 cut EX.8 — the NPC price rule and the price history** (Q-48-18, -34, -35, -39), over
//! the composed world (`driving`'s rig) at Ceres Freeport, on the Co-op's anti-armour missiles
//! (120 in stock at 1,500 cr; no mill feeds them, so only a purchase moves the pool).
//!
//! - **The step.** A captain buys 40 of the 120. The ask's target is then 1,333‰ (a third short),
//!   and once an IC week the price steps toward it by at most 5% of itself: 1,575, 1,653, ... to
//!   1,999 cr, where it stays. The Co-op's bid, still wanting its whole intake, stays at par.
//! - **The history.** `+market/history` shows a row an IC week, newest first, and keeps 24.
//! - **The throttle.** A house gone delinquent on a debt raises its asks 10% an IC week whatever
//!   its stock, the one exception to the 5% step, and posts and steps no bids.
//!
//! The clock runs at 3,600:1 so an IC week passes in under three real minutes of sweeps.

use mush_economy::{AccountId, Amount};
use mush_market::{MarketId, PriceSide};

use crate::driving_rig::{
    Rig, cid, ic_now, port_map, prospector, run, set_rate, staff_hand, stocked, to_ic,
};
use crate::in5_production::{buy, market, mint, storefront};

const CERES: &str = "ceres_port";
const WEEK: i64 = mush_market::PERIOD_IC_SECS;
const MISSILE: &str = "ammo_missile";

fn venue() -> MarketId {
    MarketId::new("ceres").expect("Ceres's venue")
}

fn catalog(good: &str) -> mush_economy::CatalogRef {
    tacsim::port::price::catalog_of_good(&cid(good))
}

/// The Co-op's standing missile ask's price, minor units.
fn missile_ask(rig: &Rig) -> i64 {
    let endpoint = mush_inventory::EndpointRef(storefront(rig, CERES, "coop"));
    let m = market(rig);
    m.lot_listing_of(endpoint, &cid(MISSILE))
        .and_then(|id| m.listing(id))
        .map(|l| l.price.minor())
        .expect("the missile ask")
}

/// To a little past the start of the IC week `weeks` after `from`'s.
fn to_week(rig: &mut Rig, from: i64, weeks: i64) {
    let target = (mush_market::period_of(from) + weeks) * WEEK + 3_600;
    let _ = to_ic(rig, target);
}

fn factor(rig: &Rig, good: &str, side: PriceSide) -> Option<(i64, u16)> {
    mush_market::latest_point(rig.tw.world(), &venue(), &catalog(good), side)
        .map(|(p, point)| (p, point.factor_permille))
}

/// **An NPC ask moves with stock by at most 5% an IC week, within the band, and the history keeps
/// 24 IC weeks**; a bid still wanting its intake stays at par.
#[test]
fn ex8_a_house_ask_steps_with_its_stock_five_percent_an_ic_week() {
    let mut rig = stocked();
    let (captain, _) = prospector(&mut rig, "Trader", 91, "Price Moth");
    mint(&mut rig, captain);
    set_rate(&mut rig.tw, 3_600);
    assert_eq!(missile_ask(&rig), 150_000, "the authored price at boot");
    buy(&mut rig, captain, CERES, "coop", MISSILE, 40);
    let start = ic_now(&rig.tw);
    let mut last = missile_ask(&rig);
    assert_eq!(last, 150_000, "a purchase moves no price by itself");
    let mut prices = Vec::new();
    for week in 1..=8 {
        to_week(&mut rig, start, week);
        let now = missile_ask(&rig);
        assert!(now >= last, "a scarce ask never falls: {last} then {now}");
        assert!(
            now * 1_000 <= last * 1_050,
            "at most 5% an IC week: {last} then {now}"
        );
        prices.push(now);
        last = now;
    }
    assert_eq!(
        &prices[..2],
        &[157_500, 165_300],
        "5% of itself, an IC week"
    );
    // A third short of its 120, the ask's target is 1,333‰: 1,999 cr, where it stays.
    assert_eq!(*prices.last().expect("prices"), 199_900);
    assert_eq!(
        factor(&rig, MISSILE, PriceSide::Ask).map(|(_, f)| f),
        Some(1_333)
    );
    // The Co-op's parts bid still wants its whole intake: par.
    assert_eq!(
        factor(&rig, "industrial_parts", PriceSide::Bid).map(|(_, f)| f),
        Some(1_000)
    );
    // `+market/history` shows a row an IC week, newest first, within 120 columns.
    let shown = run(
        &mut rig.tw,
        captain,
        "+market/history tacsim:ammo_missile market=ceres",
    );
    assert!(shown.contains("this IC week: ask 1999.00cr"), "{shown}");
    assert!(shown.contains("last IC week: ask"), "{shown}");
    for line in shown.lines() {
        assert!(line.chars().count() <= 120, "over 120 columns: {line}");
    }
    // The window keeps 24 IC weeks.
    for week in 9..=30 {
        to_week(&mut rig, start, week);
    }
    let series =
        mush_market::price_series(rig.tw.world(), &venue(), &catalog(MISSILE)).expect("the series");
    assert_eq!(series.periods.len(), mush_market::HISTORY_PERIODS);
    let now = mush_market::period_of(ic_now(&rig.tw));
    assert_eq!(series.periods.last().map(|p| p.period), Some(now));
}

/// **The throttle is the one exception** (Q-48-39): a house delinquent on a debt raises its asks
/// 10% an IC week whatever its stock, and posts and steps no bids.
#[test]
fn ex8_a_throttled_house_raises_its_asks_ten_percent_an_ic_week() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 3_600);
    let map = port_map(&rig.tw, CERES);
    let coop = AccountId(map.orgs[&cid("coop")].account.expect("the Co-op's account"));
    let treasury = AccountId(map.treasury.expect("a treasury"));
    // A debt far past the Co-op's capital: unpaid, it goes delinquent at the next sweep.
    staff_hand(&mut rig, |ctx| {
        mush_economy::create_debt(
            ctx,
            coop,
            treasury,
            Amount::from_minor(i64::MAX / 4),
            "tacsim.test.ex8",
        )
        .expect("the debt")
    });
    let start = ic_now(&rig.tw);
    to_week(&mut rig, start, 1);
    // Delinquent from here: what the bid stood at once the throttle bit.
    let bid_before = factor(&rig, "industrial_parts", PriceSide::Bid);
    assert!(
        tacsim::port::price::throttled(rig.tw.world(), mush_org::OrgId(map.orgs[&cid("coop")].org)),
        "the Co-op is delinquent"
    );
    let mut last = missile_ask(&rig);
    for week in 2..=4 {
        to_week(&mut rig, start, week);
        let now = missile_ask(&rig);
        assert!(
            now * 1_000 >= last * 1_095 && now * 1_000 <= last * 1_101,
            "10% an IC week, though its stock is whole: {last} then {now}"
        );
        last = now;
    }
    assert_eq!(
        factor(&rig, "industrial_parts", PriceSide::Bid).map(|(p, _)| p),
        bid_before.map(|(p, _)| p),
        "a throttled house steps no bid"
    );
}
