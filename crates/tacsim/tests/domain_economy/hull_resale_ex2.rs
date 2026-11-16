//! **Doc 48 cut EX.2 — a hull sold player to player** (Q-48-7, -52), over the composed lane
//! (`driving`'s rig), through the production command paths and the economy's sweep.
//!
//! 1. A first captain buys her Dray out of the Co-op (`+coop/buyout`) and, at the Ceres
//!    exchange, lists her: `+market/post #<hull> price=<n>`. The bay will not strip her or break
//!    her up while she is listed, and she is not listed twice. From then on the seller types
//!    nothing: a second captain quotes and buys her, the listing accepts the sale as its
//!    seller's standing acceptance, the next settlement pays the seller, passes the title, and
//!    **her hold goes with her** (the hold vault is the buyer's).
//! 2. A laden hold: a hull listed empty and laden afterwards is no longer quoted, and a laden
//!    hull is not listed.
//! 3. **No bait-and-strip**: a hull changed after the quote is not sold on it (the quote is
//!    stale); a hull changed after the purchase does not settle (the sale's `market.asset.state`
//!    fact is false) and the price comes back to the buyer at the window, the title and the hold
//!    staying the seller's.
//!
//! **What is a fixture.** The captains' money (minted by staff); the cargo seeded into the hold
//! (staff's path); the hull's damage after a quote (a staff section write standing in for what
//! the bay does not govern). Every act of the captains' is the production path.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, VaultId};
use mush_org::PrincipalId;
use tacsim::port::HullRow;

use crate::driving_rig::{
    Rig, hold_vault_of, pilot, port_map, run, seed_hold, seed_part, station_room, stocked, sweep,
    teleport,
};
use crate::field_host_rig::set_section;

/// Her price, minor (60,000.00cr).
const PRICE: i64 = 6_000_000;

fn mint(rig: &mut Rig, who: ObjId, minor: i64) {
    let account = account_of(rig, who);
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", account.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

fn account_of(rig: &Rig, who: ObjId) -> AccountId {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .expect("a credit account")
        .id
}

fn balance(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the balance")
}

fn title(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    tacsim::port::house_sweep::title_of(rig.tw.world(), hull)
}

fn owner(rig: &Rig, vault: VaultId) -> PrincipalId {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().vault(vault).map(|v| v.owner.clone()))
        .expect("the vault")
}

/// A captain chartered into `name`, at the desk.
fn captain(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let who = pilot(rig, who, seed);
    let out = run(&mut rig.tw, who, &format!("+coop/enrol {name}"));
    assert!(out.contains(&format!("now the {name}")), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), who).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    (who, row)
}

/// A captain who has bought `name` out of the Co-op: her title, her hold.
fn owner_of_her(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let (who, row) = captain(rig, who, seed, name);
    mint(rig, who, 10_000_000);
    let out = run(&mut rig.tw, who, "+coop/buyout");
    assert!(
        out.contains("The Registry writes the title transfer"),
        "{out}"
    );
    sweep(&mut rig.tw, 31);
    let hull = ObjId(row.hull);
    assert_eq!(title(rig, hull), Some(PrincipalId::Object(who)), "hers");
    assert_eq!(
        owner(rig, hold_vault_of(&rig.tw, &row)),
        PrincipalId::Object(who),
        "her hold"
    );
    (who, row)
}

fn exchange(rig: &mut Rig, who: ObjId) {
    let room = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, who, room);
}

/// `+market/post #<hull> price=<n>`; the listing.
fn list(rig: &mut Rig, seller: ObjId, hull: ObjId) -> mush_market::ListingId {
    exchange(rig, seller);
    let said = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price={PRICE}", hull.0),
    );
    assert!(said.contains("up for sale as #"), "{said}");
    assert!(said.contains("It trades while you are away."), "{said}");
    let id = said
        .split("up for sale as #")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no listing id in {said}"));
    mush_market::ListingId(id)
}

fn market(rig: &Rig) -> &mush_market::MarketStore {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("the market")
}

/// `+market/quote <listing>=1` at the exchange: the reply and the quote, if one was issued.
fn quote(
    rig: &mut Rig,
    buyer: ObjId,
    listing: mush_market::ListingId,
) -> (String, Option<mush_market::QuoteId>) {
    exchange(rig, buyer);
    let before = newest(rig, buyer);
    let said = run(&mut rig.tw, buyer, &format!("+market/quote {listing}=1"));
    let after = newest(rig, buyer);
    (said, after.filter(|q| Some(*q) != before))
}

fn newest(rig: &Rig, buyer: ObjId) -> Option<mush_market::QuoteId> {
    market(rig)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
}

/// `+market/accept <quote>`: the reply and the contract it filed, if any.
fn accept(
    rig: &mut Rig,
    buyer: ObjId,
    quote: mush_market::QuoteId,
) -> (String, Option<mush_economy::ContractId>) {
    let said = run(&mut rig.tw, buyer, &format!("+market/accept {quote}"));
    let contract = market(rig).quote(quote).and_then(|q| q.contract);
    (said, contract)
}

/// A section of `hull`'s, and a condition it is not in.
fn a_change(rig: &Rig, hull: ObjId) -> (String, String) {
    let unit = mush_tactics::module::unit_of(rig.tw.world(), hull).expect("her unit");
    let (section, state) = unit.sections().next().expect("a section");
    let to = if state.condition.as_str() == "damaged" {
        "intact"
    } else {
        "damaged"
    };
    (section.as_str().to_owned(), to.to_owned())
}

/// **A captain sells her hull to another while she is away, and her hold goes with her.**
#[test]
fn ex2_a_captain_sells_her_hull_while_away_and_her_hold_goes_with_her() {
    let mut rig = stocked();
    let (seller, row) = owner_of_her(&mut rig, "Seller", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    let (buyer, buyer_row) = captain(&mut rig, "Buyer", 2, "Brine Lark");
    mint(&mut rig, buyer, PRICE + 100_000);
    // A traced part, in another hold, for the fit the bay must refuse.
    let part = mush_inventory::LotId(0xe2_02);
    let other_hold = hold_vault_of(&rig.tw, &buyer_row);
    seed_part(
        &mut rig,
        other_hold,
        "tacsim.part.sensor.visual_mk1",
        part,
        mush_tactics::field_ops::ComponentCondition::Intact,
        mush_tactics::field_ops::ProvenanceLegalityStatus::CleanTitle,
    );
    let listing = list(&mut rig, seller, hull);
    let posted = market(&rig).listing(listing).cloned().expect("listed");
    assert!(posted.is_personal(), "a person's listing");
    assert_eq!(posted.object, Some(hull));
    assert_eq!(posted.seller, PrincipalId::Object(seller));
    assert_eq!(
        posted.catalog,
        tacsim::port::hull_sale::catalog_of(
            mush_tactics::module::unit_of(rig.tw.world(), hull)
                .expect("her unit")
                .schema()
                .key()
                .id
                .as_str()
        ),
        "listed as her design's hull entry"
    );
    // Listed, she is locked in the bay: no strip, no scrap, no fit; and she is not listed
    // twice.
    let strip = run(
        &mut rig.tw,
        seller,
        &format!("+bay/strip #{}=sensor", hull.0),
    );
    assert!(
        strip.contains(tacsim::port::bay_work::UNDER_SALE),
        "{strip}"
    );
    let scrap = run(&mut rig.tw, seller, &format!("+bay/scrap #{}", hull.0));
    assert!(
        scrap.contains(tacsim::port::bay_work::UNDER_SALE),
        "{scrap}"
    );
    let fit = run(
        &mut rig.tw,
        seller,
        &format!("+bay/fit #{}=visual_sensor {part}", hull.0),
    );
    assert!(fit.contains(tacsim::port::bay_work::UNDER_SALE), "{fit}");
    let twice = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price=100", hull.0),
    );
    assert!(
        twice.contains(mush_market::seller::ALREADY_FOR_SALE),
        "{twice}"
    );

    // From here the seller types nothing.
    let seller_before = balance(&rig, account_of(&rig, seller));
    let buyer_account = account_of(&rig, buyer);
    let buyer_before = balance(&rig, buyer_account);
    let (said, quoted) = quote(&mut rig, buyer, listing);
    let quoted = quoted.unwrap_or_else(|| panic!("no quote: {said}"));
    let (said, contract) = accept(&mut rig, buyer, quoted);
    let contract = contract.unwrap_or_else(|| panic!("no contract: {said}"));
    let sale = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().contract(contract).cloned())
        .expect("the sale");
    assert!(
        sale.acceptances
            .iter()
            .any(|a| a.party == PrincipalId::Object(seller)),
        "the listing accepted for its seller: {:?}",
        sale.acceptances
    );
    assert!(
        mush_market::sells_title_of(&sale, hull),
        "the one asset sale"
    );
    sweep(&mut rig.tw, 31);
    assert_eq!(
        title(&rig, hull),
        Some(PrincipalId::Object(buyer)),
        "her title passed"
    );
    assert_eq!(
        owner(&rig, hold),
        PrincipalId::Object(buyer),
        "her hold went with her"
    );
    assert_eq!(
        balance(&rig, account_of(&rig, seller)) - seller_before,
        PRICE,
        "the seller was paid"
    );
    assert_eq!(
        buyer_before - balance(&rig, buyer_account),
        PRICE,
        "the buyer paid"
    );
    assert!(market(&rig).listing(listing).is_none(), "sold, unlisted");
    assert_eq!(
        tacsim::port::coop::helm_of(rig.tw.world(), seller),
        None,
        "sold, she is no longer the seller's to fly"
    );
    // Sold, she is no longer locked, and the seller can no longer list her.
    assert!(!tacsim::port::bay_work::under_sale(rig.tw.world(), hull));
    let again = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price=100", hull.0),
    );
    assert!(
        again.contains(mush_market::seller::NO_SUCH_ASSET),
        "{again}"
    );
}

/// **A laden hold is not sold**: laden after her quote, she is not bought on it and not quoted
/// again; laden, she is not listed.
#[test]
fn ex2_a_laden_hull_is_neither_listed_nor_quoted() {
    let mut rig = stocked();
    let (seller, row) = owner_of_her(&mut rig, "Seller", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let (buyer, _) = captain(&mut rig, "Buyer", 2, "Brine Lark");
    mint(&mut rig, buyer, PRICE + 100_000);
    let listing = list(&mut rig, seller, hull);
    let (said, quoted) = quote(&mut rig, buyer, listing);
    let quoted = quoted.unwrap_or_else(|| panic!("no quote: {said}"));
    seed_hold(&mut rig, &row, "metals", 5, 0xe2_01);
    let (said, contract) = accept(&mut rig, buyer, quoted);
    assert!(contract.is_none(), "a laden hull is not bought: {said}");
    assert!(said.contains("no longer stands"), "{said}");
    let (said, again) = quote(&mut rig, buyer, listing);
    assert!(again.is_none(), "a laden hull is not quoted: {said}");
    assert!(said.contains("no longer stands"), "{said}");
    let out = run(&mut rig.tw, seller, &format!("+market/withdraw {listing}"));
    assert!(market(&rig).listing(listing).is_none(), "{out}");
    let out = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price={PRICE}", hull.0),
    );
    assert!(
        out.contains(&tacsim::port::hull_sale::HULL_HOLD_LADEN.replace("{name}", "Salt Moth")),
        "{out}"
    );
}

/// **No bait-and-strip**: a hull changed after her quote is not sold on it, and one changed
/// after her purchase does not settle; the buyer's money comes back at the window.
#[test]
fn ex2_a_hull_changed_after_the_quote_is_not_sold() {
    let mut rig = stocked();
    let (seller, row) = owner_of_her(&mut rig, "Seller", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    let (buyer, _) = captain(&mut rig, "Buyer", 2, "Brine Lark");
    mint(&mut rig, buyer, PRICE + 100_000);
    let listing = list(&mut rig, seller, hull);
    let (section, to) = a_change(&rig, hull);
    // Changed after the quote: the quote is stale.
    let (said, quoted) = quote(&mut rig, buyer, listing);
    let quoted = quoted.unwrap_or_else(|| panic!("no quote: {said}"));
    set_section(&mut rig, hull, &section, Some(&to), None);
    let (said, contract) = accept(&mut rig, buyer, quoted);
    assert!(contract.is_none(), "not bought on a stale quote: {said}");
    assert!(said.contains("no longer stands"), "{said}");
    // Quoted again on her state now, bought, then changed back before the settlement.
    let (said, quoted) = quote(&mut rig, buyer, listing);
    let quoted = quoted.unwrap_or_else(|| panic!("no second quote: {said}"));
    let buyer_account = account_of(&rig, buyer);
    let before = balance(&rig, buyer_account);
    let (said, contract) = accept(&mut rig, buyer, quoted);
    let contract = contract.unwrap_or_else(|| panic!("no contract: {said}"));
    let back = if to == "damaged" { "intact" } else { "damaged" };
    set_section(&mut rig, hull, &section, Some(back), None);
    sweep(&mut rig.tw, 31);
    let state = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().contract(contract).map(|c| c.state.kind()))
            .expect("the sale")
    };
    assert_ne!(
        state(&rig),
        mush_economy::ContractStateKind::Settled,
        "a drifted sale does not settle"
    );
    assert_eq!(title(&rig, hull), Some(PrincipalId::Object(seller)));
    // Past the window the escrow comes back; the title and the hold stay the seller's.
    sweep(&mut rig.tw, mush_market::DEAL_WINDOW_SECS + 120);
    sweep(&mut rig.tw, 31);
    assert_eq!(
        state(&rig),
        mush_economy::ContractStateKind::Expired,
        "the drifted sale lapsed"
    );
    assert_eq!(balance(&rig, buyer_account), before, "refunded");
    assert_eq!(title(&rig, hull), Some(PrincipalId::Object(seller)));
    assert_eq!(owner(&rig, hold), PrincipalId::Object(seller));
    // The purchase took her listing; lapsed, she is neither listed nor locked.
    assert!(market(&rig).listing(listing).is_none());
    assert!(!tacsim::port::bay_work::under_sale(rig.tw.world(), hull));
}
