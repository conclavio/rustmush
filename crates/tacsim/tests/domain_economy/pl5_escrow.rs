//! **Doc 48 unit PL.5 — buying straight into an escrow** (Q-48-28 as corrected by doc 48b W-48-4),
//! over the composed world (`driving`'s stocked rig), through the production command paths and
//! the port tick.
//!
//! 1. **Any buyer** (`pl5_a_player_buys_a_works_bill_into_its_escrow_with_no_hold`): a player
//!    with no hull, standing on Stickney's exchange, books a fittings batch at the works and buys
//!    its bill with `+works/buy`, V7's seller and price: Kasei's alloys from its standing ask,
//!    then the Yards' carbon (stocked and bid for, never asked: the owner's IN.11 rule) from a
//!    private ask the port's clerk lists to them first. While their carbon ask stands, a second
//!    buyer's carbon waits; once it is drawn, the second buyer is listed their own. Each purchase
//!    is a trade on the tape, the escrow holds the bill, and the order runs. A stranger cannot
//!    buy into the order.
//! 2. **A House with funds orders a Dray with no hold** (`pl5_a_house_orders_a_dray_with_no_hold`):
//!    staff make Meridiani and its treasury (the bring-up's), and grant its officer the
//!    treasury's post. Short of the price, `+yard/fund dray=org:Meridiani` is refused with what
//!    the treasury holds; a deckhand who does not act for the House is refused. Funded, the
//!    officer's order stands: the House's yard agent is made at Stickney and funded from the
//!    treasury, books the yard order for the House, orders the hauler plant the stores lack at
//!    the works, and buys both bills on the board. Twelve IC days after the plant's twenty, the
//!    Dray is handed over docked at Stickney, titled to Meridiani; the agent's unspent purse is
//!    back in the treasury.
//! 3. **A part from the yard's stores** (`pl5_a_player_buys_a_stores_part_by_a_private_lot_ask`):
//!    a player books a Dray at the yard and buys its hauler plant with `+yard/buy`: the port's
//!    clerk lists the stores' one plant lot to them as a private exact-lot ask at its catalogue
//!    price, and they take it into the escrow. While it stands, a second buyer's plant waits, the
//!    only lot held for the first.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the players' and the House's money, the House, its treasury
//! and the officer's grant (staff's, as the bring-up makes them), the plant lot in the yard's
//! stores (staff's, the lane seeding no part since IN.7), the players' places. Every other claim
//! is reached by the players' commands and the port tick.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, EconomyStore, VaultId};
use mush_org::PrincipalId;
use tacsim::port::pools::Phase;
use tacsim::port::works::{AGENT_BUYS, BILL_COVERED, BUY_HELD, BUY_ON_BOARD, BUY_POOL, WorksState};

use crate::driving_rig::{Rig, hold_pool, ic_now, pilot, port_map, run, station_room, stocked};
use crate::in10_pools::{days_on, fire_all, settle};

const STICKNEY: &str = "stickney_port";
const ALLOYS: &str = "structural_alloys";
const CARBON: &str = "carbon_ore";
const DAY: i64 = 86_400;

/// A player with a credit account and 100,000 cr, standing on Stickney's exchange.
pub(crate) fn buyer(rig: &mut Rig, who: &str, seed: i64) -> (ObjId, AccountId) {
    let player = pilot(rig, who, seed);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", player.0),
    );
    let account = crate::in5_production::mint(rig, player);
    let exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    crate::driving_rig::teleport(&mut rig.tw, player, exchange);
    (player, account)
}

/// Kasei's public alloy ask at Stickney, while it stands.
pub(crate) fn kasei_alloys(rig: &Rig) -> Option<mush_market::ListingId> {
    let vault = port_map(&rig.tw, STICKNEY).vendors[&crate::driving_rig::cid("kasei")].vault;
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()?
        .lot_listings_of(
            mush_inventory::record::EndpointRef(VaultId(vault)),
            &crate::driving_rig::cid(ALLOYS),
        )
        .find(|listing| listing.private_to.is_none())
        .map(|listing| listing.id)
}

/// An account's posted balance, minor units.
fn posted(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account")
}

/// An account's available balance, minor units.
pub(crate) fn available(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .and_then(|a| a.available())
        .map_or(0, mush_economy::Amount::minor)
}

/// `who`'s settled trades on the tape.
fn trades(rig: &Rig, who: ObjId) -> usize {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .map_or(0, |m| {
            m.quotes_of_buyer(&PrincipalId::Object(who))
                .filter(|q| q.is_trade())
                .count()
        })
}

/// The order a booking line names: "Order {n} booked at the …".
pub(crate) fn booked(out: &str) -> u64 {
    out.split("Order ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("a booking: {out}"))
}

/// Fires until `done`, at most `limit`.
pub(crate) fn fires_until(
    rig: &mut Rig,
    limit: usize,
    mut done: impl FnMut(&mut Rig) -> bool,
) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        fire_all(rig);
    }
    done(rig)
}

/// `+works/buy {order}` until it answers something other than the wait for the last purchase,
/// firing between; the answer.
fn buy_settled(rig: &mut Rig, who: ObjId, order: u64) -> String {
    for _ in 0..200 {
        let out = run(&mut rig.tw, who, &format!("+works/buy {order}"));
        if !out.contains(BUY_ON_BOARD) && !out.contains("is free to buy here just now") {
            return out;
        }
        fire_all(rig);
    }
    panic!("order {order}'s purchase never settled")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl5_a_player_buys_a_works_bill_into_its_escrow_with_no_hold() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (first, account) = buyer(&mut rig, "Quill", 1);
    let (second, _) = buyer(&mut rig, "Rook", 2);
    let (stranger, _) = buyer(&mut rig, "Sable", 3);
    assert!(
        tacsim::port::coop::helm_of(rig.tw.world(), first).is_none(),
        "the buyer holds no hull and no hold"
    );

    let out = run(&mut rig.tw, first, "+works/order fittings");
    let order = booked(&out);
    let escrow = VaultId(order);
    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(
        out.contains(tacsim::port::works::BUY_BUSY),
        "the booking's draft is still queued: {out}"
    );
    settle(&mut rig, 2);
    let out = run(&mut rig.tw, stranger, &format!("+works/buy {order}"));
    assert!(out.contains("You have no such order here."), "{out}");
    let before = posted(&rig, account);

    // A good asked here is bought from its standing ask alone: with Kasei's withdrawn, the buyer
    // waits for the clerk to list it again, and no private ask goes on Kasei's pool.
    let kasei = kasei_alloys(&rig).expect("Kasei lists its alloys");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@market/vendor withdraw {kasei}"),
    );
    assert!(kasei_alloys(&rig).is_none(), "{out}");
    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(
        out.contains(&BUY_POOL.replace("{good}", ALLOYS)),
        "the buyer waits for the standing ask: {out}"
    );
    assert!(
        fires_until(&mut rig, 20, |rig| kasei_alloys(rig).is_some()),
        "the clerk lists Kasei's alloys again"
    );

    // Kasei's alloys from its standing ask, straight into the escrow.
    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(
        out.contains("You buy 5 structural_alloys from ")
            && out.contains(&format!("order {order}")),
        "{out}"
    );
    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(out.contains(BUY_ON_BOARD), "one purchase at a time: {out}");
    assert!(
        fires_until(&mut rig, 200, |rig| hold_pool(&mut rig.tw, escrow, ALLOYS)
            == 5),
        "the alloys land in the escrow"
    );
    assert!(
        fires_until(&mut rig, 60, |rig| trades(rig, first) == 1),
        "a trade on the tape: {}",
        trades(&rig, first)
    );

    // The Yards' carbon: stocked and bid for, never asked. The clerk lists the buyer a private
    // ask at the bid first.
    let out = buy_settled(&mut rig, first, order);
    assert!(
        out.contains("lists you 1 carbon_ore at ") && out.contains("buy again to take it"),
        "{out}"
    );
    settle(&mut rig, 2);
    // A player's private ask lapses in minutes, so another buyer of the pool waits no longer.
    let yards =
        port_map(&rig.tw, STICKNEY).vendors[&crate::driving_rig::cid("stickney_yards")].vault;
    let lapses = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|market| {
            market
                .lot_listings_of(
                    mush_inventory::record::EndpointRef(VaultId(yards)),
                    &crate::driving_rig::cid(CARBON),
                )
                .find(|listing| listing.private_to == Some(PrincipalId::Object(first)))
                .map(|listing| listing.expires.at)
        })
        .expect("the first buyer's private ask");
    assert!(
        lapses - rig.tw.world().now() <= tacsim::port::pools::PLAYER_ASK_TTL_SECS,
        "a player's private ask lapses in ten minutes: {}",
        lapses - rig.tw.world().now()
    );

    // While it stands, a second buyer's carbon waits on the pool.
    let out = run(&mut rig.tw, second, "+works/order fittings");
    let theirs = booked(&out);
    settle(&mut rig, 2);
    let out = run(
        &mut rig.tw,
        second,
        &format!("+works/buy {theirs}={CARBON}"),
    );
    assert!(
        out.contains(&BUY_HELD.replace("{good}", CARBON)),
        "another buyer's private ask holds the pool: {out}"
    );

    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(out.contains("You buy 1 carbon_ore from "), "{out}");
    assert!(
        fires_until(&mut rig, 200, |rig| hold_pool(&mut rig.tw, escrow, CARBON)
            == 1),
        "the carbon lands in the escrow"
    );
    // Covered, before the arm's next fire starts the order.
    let out = run(&mut rig.tw, first, &format!("+works/buy {order}"));
    assert!(
        out.contains(&BILL_COVERED.replace("{n}", &order.to_string())),
        "{out}"
    );
    assert!(
        fires_until(&mut rig, 60, |rig| trades(rig, first) == 2),
        "two trades on the tape: {}",
        trades(&rig, first)
    );
    // The bill at the listings' prices, nothing else posted.
    let paid: i64 = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .map_or(0, |m| {
            m.quotes_of_buyer(&PrincipalId::Object(first))
                .filter(|q| q.is_trade())
                .map(|q| q.price.minor() * i64::try_from(q.quantity).expect("a quantity"))
                .sum()
        });
    assert!(paid > 0, "the trades carry their prices");
    assert_eq!(
        before - posted(&rig, account),
        paid,
        "the buyer paid the listings' prices"
    );

    // Drawn, the first buyer's ask leaves the pool: the second is listed their own.
    let out = run(
        &mut rig.tw,
        second,
        &format!("+works/buy {theirs}={CARBON}"),
    );
    assert!(out.contains("lists you 1 carbon_ore at "), "{out}");

    // The order runs from its escrow.
    assert!(
        fires_until(&mut rig, 60, |rig| {
            run(&mut rig.tw, first, "+works")
                .contains(&format!("Order {order}: fittings x1, running"))
        }),
        "the order runs: {}",
        run(&mut rig.tw, first, "+works")
    );
}

/// The org named `name`.
pub(crate) fn org_named(rig: &Rig, name: &str) -> mush_org::OrgId {
    rig.tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == name)
                .map(|org| org.id)
        })
        .unwrap_or_else(|| panic!("no organization {name}"))
}

pub(crate) fn house_pool(
    rig: &Rig,
    org: mush_org::OrgId,
) -> Option<tacsim::port::pools::HousePool> {
    tacsim::port::pools::pool(rig.tw.world(), &tacsim::port::pools::house_key(org.0)).cloned()
}

pub(crate) fn phase(rig: &Rig, org: mush_org::OrgId) -> Option<Phase> {
    house_pool(rig, org).and_then(|p| p.orders.first().map(|o| o.phase))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl5_a_house_orders_a_dray_with_no_hold() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (officer, _) = buyer(&mut rig, "Officer Vance", 1);
    let (deckhand, _) = buyer(&mut rig, "Deckhand Ilo", 2);
    for line in [
        "@org/create Meridiani".to_owned(),
        format!("@account/open org:Meridiani={}", tacsim::CREDIT),
        format!("@org/grant #{}=Meridiani economy.account.post", officer.0),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    let house = org_named(&rig, "Meridiani");
    assert!(
        tacsim::port::coop::helm_of(rig.tw.world(), officer).is_none(),
        "the officer holds no hull: no hold carries the bill"
    );
    let treasury = tacsim::port::pools::treasury_of(rig.tw.world(), house)
        .expect("Meridiani's treasury on its root scope");

    // Short of the price, and not the House's to spend.
    let out = run(&mut rig.tw, officer, "+yard/fund dray=org:Meridiani");
    assert!(
        out.contains("The treasury has ") && out.contains("a dray costs "),
        "{out}"
    );
    let out = run(&mut rig.tw, deckhand, "+yard/fund dray=org:Meridiani");
    assert!(
        out.contains("You do not act for that organization."),
        "{out}"
    );
    assert!(house_pool(&rig, house).is_none(), "nothing is ordered");

    // Staff fund the House (Q-48-55's income is EX.9's).
    let price = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        tacsim::port::pools::order_price_minor(ctx, "dray", false)
    })
    .expect("the Dray's price");
    eprintln!("pl5: a House Dray costs {price} minor");
    // No ferry: a House's price leaves out the stores the Trust's ferry buys.
    let ferried = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        tacsim::port::pools::price_minor(ctx, &tacsim::port::pools::FLOORS[0])
    })
    .expect("the Trust's Dray's price");
    assert!(
        price < ferried,
        "a House's Dray carries no ferry stores: {price} against {ferried}"
    );
    assert!(
        (100_000 * 100..=200_000 * 100).contains(&price),
        "a Dray at the yard's asks: {price}"
    );
    let mint = port_map(&rig.tw, STICKNEY).treasury.expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {mint}={} {}", treasury.0, 200_000 * 100),
    );
    assert!(!out.contains("Usage"), "{out}");
    let funded = available(&rig, treasury);

    // The officer orders; the House pays.
    let out = run(&mut rig.tw, officer, "+yard/fund dray=org:Meridiani");
    assert!(
        out.contains("Meridiani orders a dray at the yard for ")
            && out.contains("paid from its treasury"),
        "{out}"
    );
    assert!(
        out.contains(AGENT_BUYS.split("{org}").next().expect("a head")),
        "{out}"
    );
    assert_eq!(phase(&rig, house), Some(Phase::Requested));
    // A second before the first is funded: the treasury counts what it has promised.
    let out = run(&mut rig.tw, officer, "+yard/fund dray=org:Meridiani");
    assert!(
        out.contains("The treasury has ") && out.contains("a dray costs "),
        "the first order's price is promised: {out}"
    );
    assert_eq!(
        house_pool(&rig, house).map(|p| p.orders.len()),
        Some(1),
        "one order stands"
    );
    assert!(
        fires_until(&mut rig, 40, |rig| phase(rig, house)
            == Some(Phase::Supplying)),
        "funded and booked: {:?}",
        house_pool(&rig, house)
    );
    let record = house_pool(&rig, house).expect("the House's record");
    let agent = record.helm.expect("the yard agent");
    assert!(
        rig.tw
            .world()
            .get(agent)
            .is_some_and(|o| o.flags.contains(mush_core::Flags::DARK)
                && o.name.as_str() == "Meridiani yard agent"),
        "the House's agent"
    );
    assert_eq!(
        crate::ex3_board::admitted(&rig, agent, "stickney"),
        Ok(()),
        "the agent at the yard's venue"
    );
    assert!(
        crate::ex3_board::admitted(&rig, agent, "luna").is_err(),
        "and at no other"
    );
    assert_eq!(
        funded - available(&rig, treasury),
        i64::try_from(price).expect("a price"),
        "the treasury funded its agent with the price"
    );
    let contract = record.orders[0].contract.expect("the yard order");
    let c = match crate::driving_rig::store(&rig.tw).works_contract(contract) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(c.requester, agent, "the agent booked it");
    assert_eq!(
        c.beneficiary,
        PrincipalId::Organization(house),
        "for the House"
    );

    // The stores hold no hauler plant: the agent orders one at the works and buys its bill on
    // the board; twenty IC days on it moves into the Dray's escrow.
    assert!(
        fires_until(&mut rig, 200, |rig| house_pool(rig, house)
            .is_some_and(|p| p.orders.first().is_some_and(|o| o.part.is_some()))),
        "the plant is ordered at the works: {:?}",
        house_pool(&rig, house)
    );
    // The Dray's alloys and fittings were bought first, each a trade on the tape; then the
    // plant's bill, its carbon from a private ask the clerk lists the agent at the Yards' bid.
    assert!(
        fires_until(&mut rig, 60, |rig| trades(rig, agent) >= 2),
        "the Dray's bulk lines are trades on the tape: {}",
        trades(&rig, agent)
    );
    assert!(
        fires_until(&mut rig, 300, |rig| trades(rig, agent) >= 5),
        "the plant's bill is bought on the board: {} trades",
        trades(&rig, agent)
    );
    let ordered = record.orders[0].ordered_ic;
    for _ in 0..40 {
        if phase(&rig, house) != Some(Phase::Supplying) {
            break;
        }
        days_on(&mut rig, 1);
    }
    assert!(
        fires_until(&mut rig, 200, |rig| phase(rig, house)
            == Some(Phase::Building)),
        "the bill is bought into the escrow, the plant among it: {:?}",
        house_pool(&rig, house)
    );

    // The yard builds her and hands her over docked here, titled to the House.
    for _ in 0..30 {
        if house_pool(&rig, house).is_some_and(|p| p.orders.is_empty()) {
            break;
        }
        days_on(&mut rig, 1);
    }
    assert!(
        fires_until(&mut rig, 120, |rig| house_pool(rig, house)
            .is_some_and(|p| p.orders.is_empty())),
        "handed over: {:?}",
        house_pool(&rig, house)
    );
    let took = (ic_now(&rig.tw) - ordered) / DAY;
    eprintln!("pl5: the yard handed the House her hull {took} IC days after the order");
    assert!(
        (32..=40).contains(&took),
        "the plant's twenty days and the Dray's twelve: {took}"
    );
    let commission =
        tacsim::port::commission::commission(rig.tw.world(), contract).expect("the commission");
    let hull = commission.object().expect("her hull");
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(PrincipalId::Organization(house)),
        "titled to the House"
    );
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY),
        "docked at the yard"
    );
    let purse = house_pool(&rig, house)
        .and_then(|p| p.account)
        .map(AccountId)
        .expect("the agent's purse");
    assert_eq!(available(&rig, purse), 0, "the unspent purse is home");
    assert!(
        available(&rig, treasury) > funded - i64::try_from(price).expect("a price"),
        "the unspent purse went back to the treasury"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pl5_a_player_buys_a_stores_part_by_a_private_lot_ask() {
    const PLANT: &str = "tacsim.part.plant.hauler_mk1";
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (first, _) = buyer(&mut rig, "Tamsin", 1);
    let (second, _) = buyer(&mut rig, "Ulric", 2);
    let out = run(&mut rig.tw, first, "+yard/order dray");
    let order = booked(&out);
    let out = run(&mut rig.tw, second, "+yard/order dray");
    let theirs = booked(&out);
    settle(&mut rig, 2);
    // Doc 48 unit PL.8: the workshop lists the sound parts in its stores as public lot asks, one
    // a good. The plant arrives just before the first buyer asks, so the clerk lists it to them
    // privately first, and the private ask holds it from the public list.
    let lots = crate::driving_rig::seed_store_parts(
        &mut rig,
        STICKNEY,
        "stickney_yards",
        PLANT,
        1,
        7_650_000,
    );

    let out = run(&mut rig.tw, first, &format!("+yard/buy {order}={PLANT}"));
    assert!(
        out.contains(&format!("lists you 1 {PLANT} at ")) && out.contains("buy again to take it"),
        "{out}"
    );
    settle(&mut rig, 2);
    let out = run(&mut rig.tw, second, &format!("+yard/buy {theirs}={PLANT}"));
    assert!(
        out.contains(&BUY_HELD.replace("{good}", PLANT)),
        "the only lot is held for the first buyer: {out}"
    );
    let out = run(&mut rig.tw, first, &format!("+yard/buy {order}={PLANT}"));
    assert!(out.contains(&format!("You buy 1 {PLANT} from ")), "{out}");
    let escrow = mush_inventory::record::EndpointRef(VaultId(order));
    assert!(
        fires_until(&mut rig, 120, |rig| {
            let families = rig
                .tw
                .world()
                .module_state::<mush_inventory::InventoryHandles>()
                .expect("inventory")
                .0
                .clone();
            crate::driving_rig::typed(&mut rig.tw, &families.lots, &lots[0])
                .is_some_and(|lot| lot.endpoint() == escrow)
        }),
        "the plant lot moves into the order's escrow"
    );
    assert!(
        fires_until(&mut rig, 60, |rig| trades(rig, first) == 1),
        "a trade on the tape: {}",
        trades(&rig, first)
    );
}

/// **Doc 48e, exchange G14 (48d H-03): an abandoned private part ask is released.** The first
/// buyer's `+yard/buy` lists them the yard's only plant privately for 600 s, and they never take
/// it. The ask lapses, and the port's clerk, acting for the yard, releases its hold, so the second
/// buyer is listed the plant in turn. Before the fix the lapsed ask's hold was never released (the
/// release arm acted only for player vaults), so the plant stayed held from everyone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g14_an_abandoned_private_part_ask_is_released_for_the_next_buyer() {
    const PLANT: &str = "tacsim.part.plant.hauler_mk1";
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (first, _) = buyer(&mut rig, "Vesna", 3);
    let (second, _) = buyer(&mut rig, "Wolfe", 4);
    let out = run(&mut rig.tw, first, "+yard/order dray");
    let order = booked(&out);
    let out = run(&mut rig.tw, second, "+yard/order dray");
    let theirs = booked(&out);
    settle(&mut rig, 2);
    crate::driving_rig::seed_store_parts(&mut rig, STICKNEY, "stickney_yards", PLANT, 1, 7_650_000);
    let out = run(&mut rig.tw, first, &format!("+yard/buy {order}={PLANT}"));
    assert!(out.contains(&format!("lists you 1 {PLANT} at ")), "{out}");
    settle(&mut rig, 2);
    let out = run(&mut rig.tw, second, &format!("+yard/buy {theirs}={PLANT}"));
    assert!(out.contains(&BUY_HELD.replace("{good}", PLANT)), "{out}");
    // The first buyer walks away: past the ask's 600 s, the clerk releases it.
    let released = fires_until(&mut rig, 400, |rig| {
        let out = run(&mut rig.tw, second, &format!("+yard/buy {theirs}={PLANT}"));
        out.contains(&format!("lists you 1 {PLANT} at "))
    });
    assert!(
        released,
        "the abandoned ask's plant is listed to the next buyer"
    );
}
