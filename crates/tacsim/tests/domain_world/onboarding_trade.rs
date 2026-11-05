//! The optional trade favor follows real purchases, voyages and settled deliveries.
use super::onboarding_quests::{accept, giver, progress, pump, step, visit};
use super::*;
use mush_economy::{ContractId, ContractState};
use mush_market::{ListingId, MarketStore, Quote, QuoteId};
use mush_org::PrincipalId;

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world().module_state::<MarketStore>().expect("market")
}

fn vendor_room(tw: &TestWorld, port: &str, vendor: &str) -> ObjId {
    // Stock cages live in maintenance rooms, outside the public market venue.
    // Use the actual board/counter where a player makes the trade.
    let room = match (port, vendor) {
        ("ceres_port", "coop") => "room.c08_chandlery",
        ("ceres_port", "freeport_exchange") => "room.c07_exchange",
        ("stickney_port", "kasei") => "room.p08_exchange",
        ("stickney_port", "stickney_yards") => "room.p11_service",
        _ => panic!("unsupported fixture market {port}/{vendor}"),
    };
    station_room(tw, port, room)
}

fn ask(tw: &TestWorld, port: &str, vendor: &str, good: &str) -> ListingId {
    let vault = VaultId(port_map(tw, port).vendors[&cid(vendor)].vault);
    market(tw)
        .lot_listing_of(EndpointRef(vault), &cid(good))
        .expect("live ask")
}

fn latest(tw: &TestWorld, actor: ObjId, sale: bool) -> Quote {
    let owner = PrincipalId::Object(actor);
    if sale {
        market(tw).quotes_of_seller(&owner).max_by_key(|q| q.id)
    } else {
        market(tw).quotes_of_buyer(&owner).max_by_key(|q| q.id)
    }
    .cloned()
    .expect("real market quote")
}

fn state(tw: &TestWorld, contract: ContractId) -> ContractState {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .contract(contract)
        .expect("trade contract")
        .state
}

pub(super) fn settle_trade(rig: &mut Rig, quote: QuoteId) {
    let contract = market(&rig.tw).quote(quote).unwrap().contract.unwrap();
    for _ in 0..12 {
        if market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.settled.is_some())
        {
            assert!(matches!(
                state(&rig.tw, contract),
                ContractState::Settled { .. }
            ));
            pump(rig);
            return;
        }
        let next = ic_now(&rig.tw) + 61;
        let (_, queued) = advance_to(rig, next);
        land_quietly(rig, &queued);
    }
    panic!(
        "trade #{quote} did not settle: {:?}",
        state(&rig.tw, contract)
    );
}

pub(super) fn buy(rig: &mut Rig, actor: ObjId, port: &str, vendor: &str, good: &str) -> Quote {
    buy_quantity(rig, actor, port, vendor, good, 1)
}

pub(super) fn buy_quantity(
    rig: &mut Rig,
    actor: ObjId,
    port: &str,
    vendor: &str,
    good: &str,
    quantity: u64,
) -> Quote {
    let room = vendor_room(&rig.tw, port, vendor);
    teleport(&mut rig.tw, actor, room);
    let listing = ask(&rig.tw, port, vendor, good);
    let (out, queued) = command(rig, actor, &format!("+market/quote {listing}={quantity}"));
    assert!(queued.is_empty(), "a quote does not deliver goods: {out}");
    assert!(out.contains("Quote #"), "{out}");
    let quote = latest(&rig.tw, actor, false).id;
    let (out, queued) = command(rig, actor, &format!("+market/accept {quote}"));
    assert_eq!(queued.len(), 1, "one authentic Draw: {out}");
    assert_eq!(queued.first().unwrap().actor, actor);
    land_all(rig, &queued);
    settle_trade(rig, quote);
    let row = market(&rig.tw).quote(quote).unwrap().clone();
    assert!(
        row.drawn_at.is_some(),
        "the paid purchase must be drawn aboard"
    );
    row
}

pub(super) fn offer_sale(
    rig: &mut Rig,
    actor: ObjId,
    vendor: &str,
    good: &str,
) -> (Quote, Vec<AuthoritativeSubmit>) {
    let room = vendor_room(&rig.tw, "ceres_port", vendor);
    teleport(&mut rig.tw, actor, room);
    let listing = port_map(&rig.tw, "ceres_port").vendors[&cid(vendor)].bids[&cid(good)].listing;
    let (out, queued) = command(rig, actor, &format!("+market/sell {listing}=1"));
    assert_eq!(queued.len(), 1, "one authentic delivery: {out}");
    let quote = latest(&rig.tw, actor, true);
    assert!(quote.settled.is_none());
    assert!(matches!(
        state(&rig.tw, quote.contract.unwrap()),
        ContractState::Active
    ));
    (quote, queued)
}

fn check_refused(rig: &mut Rig, actor: ObjId, dov: ObjId, reason: &str) {
    visit(rig, actor, dov);
    let (out, queued) = command(rig, actor, &format!("+quest/check #{}", dov.0));
    assert!(queued.is_empty(), "{reason}: {out}");
    assert!(
        progress(rig, actor, dov).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{reason}"
    );
}

fn relationship(tw: &TestWorld, npc: ObjId, actor: ObjId) -> (i16, i16) {
    let host = tw.world().module_state::<mush_npc::NpcHostState>().unwrap();
    let record = host.relationships.personal(npc, actor);
    (record.trust, record.disposition)
}

#[test]
fn dov_favor_requires_a_drawn_purchase_real_route_and_paid_delivery_without_gating_trade() {
    let mut rig = stocked();
    let (actor, hull) = passengers::charter(&mut rig, "OptionalTrader", 821, "Useful Cargo");
    let dov = giver(&rig.tw, "quest.dov_fittings");

    // Real ordinary commerce works before any quest has been accepted.
    let supplies = buy(&mut rig, actor, "ceres_port", "coop", "provisions");
    assert!(supplies.settled.is_some());
    assert!(progress(&mut rig, actor, dov).is_none());
    accept(&mut rig, actor, dov);
    pump(&mut rig);
    check_refused(
        &mut rig,
        actor,
        dov,
        "an unrelated earlier purchase is not the route trade",
    );

    load(&mut rig, &hull, 150, 200, 96_000);
    board(&mut rig, actor);
    let outbound = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock stickney");
    passengers::arrive(&mut rig, &outbound, "stickney_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let wrong = buy(
        &mut rig,
        actor,
        "stickney_port",
        "kasei",
        "structural_alloys",
    );
    let right = buy(
        &mut rig,
        actor,
        "stickney_port",
        "stickney_yards",
        "industrial_parts",
    );
    assert_ne!(wrong.id, right.id);

    // The accepted favor and its purchase evidence must survive a restart.
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, actor, gallery);
    let out = run(&mut rig.tw, actor, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
    let inbound = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock ceres");
    passengers::arrive(&mut rig, &inbound, "ceres_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");

    let (wrong_sale, queued) =
        offer_sale(&mut rig, actor, "freeport_exchange", "structural_alloys");
    land_all(&mut rig, &queued);
    settle_trade(&mut rig, wrong_sale.id);
    check_refused(
        &mut rig,
        actor,
        dov,
        "a paid sale of the wrong commodity is insufficient",
    );

    let (sale, queued) = offer_sale(&mut rig, actor, "coop", "industrial_parts");
    check_refused(
        &mut rig,
        actor,
        dov,
        "the open sale has not delivered or paid",
    );
    land_all(&mut rig, &queued);
    check_refused(
        &mut rig,
        actor,
        dov,
        "delivered cargo is not yet a settled payment",
    );
    settle_trade(&mut rig, sale.id);
    let before = relationship(&rig.tw, dov, actor);
    visit(&mut rig, actor, dov);
    step(&mut rig, actor, dov, "check");
    step(&mut rig, actor, dov, "finish");
    pump(&mut rig);
    let row = progress(&mut rig, actor, dov).unwrap();
    assert!(row["completed"].is_object());
    assert!(row["checks"][0]["proof"].is_object());
    assert_eq!(
        relationship(&rig.tw, dov, actor),
        (before.0 + 10, before.1 + 5)
    );

    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, actor, dov).unwrap(), row);
    assert_eq!(
        relationship(&rig.tw, dov, actor),
        (before.0 + 10, before.1 + 5)
    );
    let (out, queued) = command(&mut rig, actor, &format!("+quest/finish #{}", dov.0));
    assert!(
        queued.is_empty(),
        "the relationship bonus cannot be repeated: {out}"
    );
}
