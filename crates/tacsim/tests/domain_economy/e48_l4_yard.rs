//! **Doc 48e, lane L4: the yard's orders** (the 48c and 48d reviews of docs 48 and 49), over the
//! composed world (`driving`'s stocked rig), through the production command paths and the port
//! tick.
//!
//! 1. **A House's order rides out risen asks and a re-termed yard**
//!    (`e48_a_house_order_rides_out_risen_asks_and_a_re_termed_yard`; production G10 and G11).
//!    Meridiani's officer orders a Dray at Stickney, the stores holding her hauler plant. Once the
//!    agent is funded and booked, Kasei's alloy ask triples (EX.8's weekly step, in one go) and
//!    staff raise the yard's margin. The agent buys the alloys at the live ask, the House's
//!    treasury topping its purse up by the rise; with the bill covered, the yard cancels the
//!    order rather than charge the fee it raised after the booking. The agent books the order
//!    again at the new fee, carries the first escrow's bill into the second, and the yard builds
//!    her: she is handed over titled to the House. Before, the purse ran short at the first rise
//!    and the cancel stood at the head of the House's queue for good.
//! 2. **A Ceres yard order buys its part from Occator's stores**
//!    (`e48_a_ceres_yard_order_buys_its_part_from_occator_s_stores`; production G13). The Ceres
//!    yard is the Freeport Works', but the parts are the workshop's (Occator's): a captain who
//!    books a Dray there buys the hauler plant in Occator's storefront with `+yard/buy`. Before,
//!    the line read the yard operator's stores, which hold no part, and answered that none was
//!    there. A worn plant in the same stores (the scrap desk's, production G12) is not served:
//!    it is scrap, not a new hull's part.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the House, its treasury, its officer's grant and its money
//! (staff's, as the bring-up makes them); the plant lots in the stores (staff's); Kasei's price
//! (EX.8's step, by hand) and the yard's margin (staff's `@facility/terms`).

use mush_core::GOD;
use mush_economy::{AccountId, VaultId};
use mush_inventory::LotId;
use mush_org::PrincipalId;
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::bay_work::WORN_GRADE;
use tacsim::port::pools::Phase;
use tacsim::port::works::{Close, WorksState};

use crate::driving_rig::{
    Rig, cid, port_map, run, seed_part_at, seed_store_parts, station_room, stocked, teleport,
};
use crate::in10_pools::{days_on, settle};
use crate::pl5_escrow::{
    available, booked, buyer, fires_until, house_pool, kasei_alloys, org_named, phase,
};

const STICKNEY: &str = "stickney_port";
const CERES: &str = "ceres_port";
const PLANT: &str = "tacsim.part.plant.hauler_mk1";

/// The works contract `id`, as stored.
fn contract(rig: &Rig, id: u64) -> tacsim::port::works::WorksContract {
    match crate::driving_rig::store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn e48_a_house_order_rides_out_risen_asks_and_a_re_termed_yard() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (officer, _) = buyer(&mut rig, "Officer Vance", 1);
    for line in [
        "@org/create Meridiani".to_owned(),
        format!("@account/open org:Meridiani={}", tacsim::CREDIT),
        format!("@org/grant #{}=Meridiani economy.account.post", officer.0),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    let house = org_named(&rig, "Meridiani");
    let treasury = tacsim::port::pools::treasury_of(rig.tw.world(), house)
        .expect("Meridiani's treasury on its root scope");
    let _ = seed_store_parts(&mut rig, STICKNEY, "stickney_yards", PLANT, 1, 7_670_000);
    let mint = port_map(&rig.tw, STICKNEY).treasury.expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {mint}={} {}", treasury.0, 600_000 * 100),
    );
    assert!(!out.contains("Usage"), "{out}");
    let funded = available(&rig, treasury);

    let out = run(&mut rig.tw, officer, "+yard/fund dray=org:Meridiani");
    assert!(
        out.contains("Meridiani orders a dray at the yard for "),
        "{out}"
    );
    assert!(
        fires_until(&mut rig, 40, |rig| phase(rig, house)
            == Some(Phase::Supplying)),
        "funded and booked: {:?}",
        house_pool(&rig, house)
    );
    let record = house_pool(&rig, house).expect("the House's record");
    let price = record.orders[0].funded_minor;
    let first = record.orders[0].contract.expect("the yard order");

    // The board's alloy ask triples after the funding (EX.8's weekly step, in one go).
    let kasei = kasei_alloys(&rig).expect("Kasei lists its alloys");
    let tripled = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        let listing = ctx
            .w
            .module_state::<mush_market::MarketStore>()
            .and_then(|m| m.listing(kasei).cloned())
            .expect("Kasei's ask");
        let tripled = mush_economy::Amount::from_minor(listing.price.minor() * 3);
        mush_market::reprice_listing(ctx, kasei, tripled).map(|()| tripled)
    })
    .expect("the ask reprices");
    // Staff raise the yard's margin after the booking: its fee rises past the pinned one.
    let yard = station_room(&rig.tw, STICKNEY, "room.p12_observation");
    let cost = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.facility(yard))
        .and_then(|f| f.cost().copied())
        .expect("a priced yard");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@facility/terms #{}=public {}/{}/{}/{}/{}/{}",
            yard.0,
            cost.capital_major,
            cost.lifetime_years,
            cost.upkeep_permille,
            cost.staff,
            cost.wage_major,
            cost.margin_permille + 100
        ),
    );
    assert!(out.contains("policy revision"), "{out}");

    // The agent buys at the live ask, topped up; the covered order is cancelled at the submit,
    // and the agent books it again.
    assert!(
        fires_until(&mut rig, 400, |rig| house_pool(rig, house)
            .and_then(|p| p.orders.first().and_then(|o| o.contract))
            .is_some_and(|c| c != first)),
        "booked again after the yard's cancel: {:?} (Kasei at {tripled:?})",
        house_pool(&rig, house)
    );
    let order = house_pool(&rig, house).expect("the record").orders[0].clone();
    assert!(
        order.topped_minor > 0,
        "the treasury topped the purse up by the rise: {order:?}"
    );
    let stale = contract(&rig, first);
    assert!(
        stale.stale,
        "the yard cancelled it for its risen fee: {stale:?}"
    );
    // The first escrow's bill is carried into the second before anything more is bought.
    assert!(
        fires_until(&mut rig, 120, |rig| contract(rig, first).closed
            == Some(Close::Cancelled)),
        "the first escrow carried and closed: {:?}",
        contract(&rig, first)
    );
    assert!(
        fires_until(&mut rig, 200, |rig| phase(rig, house)
            == Some(Phase::Building)),
        "the bill is in the second escrow: {:?}",
        house_pool(&rig, house)
    );

    // The yard builds her at its new fee and hands her over titled to the House.
    let second = house_pool(&rig, house)
        .and_then(|p| p.orders.first().and_then(|o| o.contract))
        .expect("the second yard order");
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
    let hull = tacsim::port::commission::commission(rig.tw.world(), second)
        .and_then(|c| c.object())
        .expect("her hull");
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(PrincipalId::Organization(house)),
        "titled to the House"
    );
    let spent = funded - available(&rig, treasury);
    assert!(
        spent > i64::try_from(price).expect("a price"),
        "the House paid the risen asks and fee, past its funding: {spent} against {price}"
    );
    let purse = house_pool(&rig, house)
        .and_then(|p| p.account)
        .map(AccountId)
        .expect("the agent's purse");
    assert_eq!(available(&rig, purse), 0, "the unspent purse is home");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e48_a_ceres_yard_order_buys_its_part_from_occator_s_stores() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (captain, _) = buyer(&mut rig, "Wren", 1);
    let gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, captain, gallery);
    let out = run(&mut rig.tw, captain, "+yard/order dray");
    let order = booked(&out);
    settle(&mut rig, 2);
    // Production G12: a worn plant the scrap desk took in is scrap, never a new hull's part.
    let occator = VaultId(port_map(&rig.tw, CERES).vendors[&cid("occator")].vault);
    seed_part_at(
        &mut rig,
        occator,
        PLANT,
        LotId(7_679_000),
        (ComponentCondition::Damaged, WORN_GRADE),
        ProvenanceLegalityStatus::CleanTitle,
    );
    let out = run(&mut rig.tw, captain, &format!("+yard/buy {order}={PLANT}"));
    assert!(
        out.contains(&tacsim::port::works::NO_SELLER.replace("{good}", PLANT)),
        "the worn plant is not served: {out}"
    );
    let lots = seed_store_parts(&mut rig, CERES, "occator", PLANT, 1, 7_680_000);
    let out = run(&mut rig.tw, captain, &format!("+yard/buy {order}={PLANT}"));
    assert!(
        out.contains(&format!("lists you 1 {PLANT} at ")) || out.contains("You buy 1 "),
        "the plant in Occator's stores: {out}"
    );
    if out.contains("lists you 1") {
        settle(&mut rig, 2);
        let out = run(&mut rig.tw, captain, &format!("+yard/buy {order}={PLANT}"));
        assert!(out.contains(&format!("You buy 1 {PLANT} from ")), "{out}");
    }
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
        "the intact plant moves into the order's escrow"
    );
}
