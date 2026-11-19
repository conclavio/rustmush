//! **Doc 48 cut EX.1b — the seller, at the venue** (Q-48-49, Q-48-56), over the composed world.
//!
//! - A player with no hull opens a locker at Ceres and buys into it; the locker is held to its
//!   capacity and owes its charge by the IC day.
//! - A Co-op-chartered hold cannot post a personal ask; an unchartered source can.
//! - A captain's standing buy reads the houses' asks, and a player's cheaper ask only under a
//!   named ceiling, taking its lot whole.

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;

use crate::driving_rig::{
    cid, drain, fire, hold_pool, hold_vault_of, land_all, pilot, prospector, run, seed_lot,
    station_room, store, teleport,
};
use crate::in5_production::{buy, mint};

const CERES: &str = "ceres_port";
const SIGHT: &str = "tacsim.part.sensor.visual_mk1";

/// A player with a credit account (staff open it, as onboarding would).
fn with_account(rig: &mut crate::driving_rig::Rig, who: &str, seed: i64) -> ObjId {
    let player = pilot(rig, who, seed);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", player.0),
    );
    player
}

/// The vault a lot lies in.
fn lot_vault(rig: &mut crate::driving_rig::Rig, lot: LotId) -> Option<VaultId> {
    let families = rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()?
        .0
        .clone();
    crate::driving_rig::typed(&mut rig.tw, &families.lots, &lot).map(|l| l.endpoint().vault())
}

/// The player's locker at Ceres, once open.
fn locker_vault(rig: &crate::driving_rig::Rig, player: ObjId) -> Option<VaultId> {
    let venue = mush_market::MarketId::new("ceres")?;
    tacsim::port::locker::locker_of(rig.tw.world(), player, &venue).map(|l| VaultId(l.vault))
}

/// The owed charge recorded on the player's Ceres locker, minor units.
/// Doc 48 cut EX.4: the locker's charge, as the player-owed obligation that collects it.
fn locker_charge(rig: &crate::driving_rig::Rig, player: ObjId) -> Option<mush_economy::Obligation> {
    let ns = tacsim::port::berth::locker_namespace("ceres", player);
    mush_economy::player_obligations_of(rig.tw.world(), &mush_org::PrincipalId::Object(player), &ns)
        .into_iter()
        .next()
}

/// The exchange's live metals listing.
fn metals_listing(rig: &crate::driving_rig::Rig) -> mush_market::ListingId {
    let vault =
        crate::driving_rig::port_map(&rig.tw, CERES).vendors[&cid("freeport_exchange")].vault;
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.lot_listing_of(
                mush_inventory::record::EndpointRef(VaultId(vault)),
                &cid("metals"),
            )
        })
        .expect("the exchange lists metals")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex1b_a_hull_less_player_buys_into_their_locker() {
    let mut rig = crate::driving_rig::stocked();
    let walker = with_account(&mut rig, "Walker", 41);
    let _ = mint(&mut rig, walker);
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, walker, exchange);

    // No locker yet: +locker says how to open one; +locker/open opens it.
    let shown = run(&mut rig.tw, walker, "+locker");
    assert!(shown.contains("+locker/open opens one."), "{shown}");
    let opened = run(&mut rig.tw, walker, "+locker/open");
    assert!(opened.contains("Your locker at ceres is open."), "{opened}");
    let again = run(&mut rig.tw, walker, "+locker/open");
    assert!(again.contains("is already open."), "{again}");
    let vault = locker_vault(&rig, walker).expect("the locker is recorded");
    // The record outlives a restart.
    let mut rig = crate::driving_rig::reboot(rig, &[walker]);
    assert_eq!(
        locker_vault(&rig, walker),
        Some(vault),
        "the locker reads back"
    );

    // A purchase with no hull docked lands in the locker.
    buy(&mut rig, walker, CERES, "freeport_exchange", "metals", 3);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "metals"),
        3,
        "into the locker"
    );

    // Capacity-bound: 18 more tonnes would make 21 t in a 20 t locker.
    let listing = metals_listing(&rig);
    let quoted = run(&mut rig.tw, walker, &format!("+market/quote {listing}=18"));
    let accepted = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.quotes_of_buyer(&mush_org::PrincipalId::Object(walker))
                .map(|q| q.id)
                .max()
        })
        .map(|quote| run(&mut rig.tw, walker, &format!("+market/accept {quote}")));
    let landed = drain(&mut rig.tw);
    land_all(&mut rig, &landed);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "metals"),
        3,
        "an overfill is refused: {quoted} / {accepted:?}"
    );

    // Doc 48 cut EX.4: three tonnes are charged 3 cr an IC day, collected from the walker's
    // account as a player-owed obligation; two IC days on, it has been paid and nothing is owed.
    crate::in10_pools::days_on(&mut rig, 2);
    let shown = run(&mut rig.tw, walker, "+locker");
    assert!(
        shown.contains("charged 3.00cr an IC day; 0.00cr in arrears."),
        "{shown}"
    );
    let charge = locker_charge(&rig, walker).expect("the locker's charge");
    assert_eq!(charge.amount.minor(), 300, "3 t at 1 cr a day");
    assert!(charge.settled_total >= 1, "collected: {charge:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex1b_a_chartered_hold_and_a_locker_both_post_a_personal_ask() {
    let mut rig = crate::driving_rig::stocked();
    let (captain, row) = prospector(&mut rig, "Charter", 42, "Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, SIGHT, 1, LotId(0xe1b1));
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    // Doc 48 cut EX.4 (Q-48-56): a chartered hold's lot lists; the Co-op's share runs on its sale.
    let listed = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {} price=500", LotId(0xe1b1)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);

    // The same captain's locker is no chartered hold: its lot lists.
    let _ = run(&mut rig.tw, captain, "+locker/open");
    let vault = locker_vault(&rig, captain).expect("the locker");
    seed_lot(&mut rig, vault, SIGHT, 1, LotId(0xe1b2));
    let listed = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {} price=500", LotId(0xe1b2)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex1b_an_offline_buy_takes_a_players_ask_only_under_a_ceiling() {
    let mut rig = crate::driving_rig::stocked();
    let listing = metals_listing(&rig);
    let house_minor = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| m.listing(listing).map(|l| l.price.minor()))
        .expect("the house's price");
    // A player lists two tonnes of metals from their locker, one credit under the house.
    let seller = with_account(&mut rig, "Seller", 43);
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, seller, exchange);
    let _ = run(&mut rig.tw, seller, "+locker/open");
    let locker = locker_vault(&rig, seller).expect("the locker");
    seed_lot(&mut rig, locker, "metals", 2, LotId(0xe1b3));
    let price = house_minor / 100 - 1;
    // Doc 48 cut EX.4: the seller pays the listing fee (its 10 cr floor).
    let account = tacsim::port::credit::own_account(rig.tw.world(), seller)
        .expect("the seller's account")
        .id;
    let treasury = crate::driving_rig::port_map(&rig.tw, CERES)
        .treasury
        .expect("a treasury");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 1000", account.0),
    );
    let listed = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {} price={price}", LotId(0xe1b3)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);

    // A captain docked at Ceres, with funds.
    let (captain, row) = prospector(&mut rig, "Buyer", 44, "Petrel");
    let _ = mint(&mut rig, captain);
    let hold = hold_vault_of(&rig.tw, &row);
    let hull = ObjId(row.hull);
    let drive = |rig: &mut crate::driving_rig::Rig,
                 done: &dyn Fn(&crate::driving_rig::Rig) -> bool| {
        for _ in 0..40 {
            if done(rig) {
                return;
            }
            let (_, queued) = fire(rig);
            land_all(rig, &queued);
        }
    };
    let state = |rig: &crate::driving_rig::Rig, id: u32| match store(&rig.tw).orders_state(hull) {
        tacsim::port::orders::OrdersState::Present(doc) => doc.order(id).map(|o| o.state),
        _ => None,
    };

    // No ceiling: the house's ask, though the player's is cheaper.
    let said = run(&mut rig.tw, captain, "+ship/order buy metals 2");
    assert!(!said.contains("Usage"), "{said}");
    drive(&mut rig, &|rig| {
        state(rig, 1) == Some(tacsim::port::orders::OrderState::Done)
    });
    assert_eq!(
        hold_pool(&mut rig.tw, hold, "metals"),
        2,
        "the house's two tonnes"
    );
    assert_eq!(
        lot_vault(&mut rig, LotId(0xe1b3)),
        Some(locker),
        "the player's lot unsold"
    );
    let still_listed = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|m| {
            m.personal_listings_of(&mush_org::PrincipalId::Object(seller))
                .count()
                == 1
        });
    assert!(still_listed, "the player's ask still stands");

    // A ceiling at the player's price for one tonne: their lot is two, sold whole, so it is
    // not taken, and the house is over the ceiling: nothing is bought.
    let said = run(
        &mut rig.tw,
        captain,
        &format!("+ship/order buy metals 1 max={price} short=partial"),
    );
    assert!(!said.contains("Usage"), "{said}");
    drive(&mut rig, &|rig| {
        state(rig, 2) == Some(tacsim::port::orders::OrderState::Done)
    });
    assert_eq!(
        lot_vault(&mut rig, LotId(0xe1b3)),
        Some(locker),
        "a lot larger than the order is never taken"
    );
    assert_eq!(
        hold_pool(&mut rig.tw, hold, "metals"),
        2,
        "nothing bought over the ceiling"
    );

    // The same ceiling for two tonnes: their lot, whole.
    let said = run(
        &mut rig.tw,
        captain,
        &format!("+ship/order buy metals 2 max={price}"),
    );
    assert!(!said.contains("Usage"), "{said}");
    drive(&mut rig, &|rig| {
        state(rig, 3) == Some(tacsim::port::orders::OrderState::Done)
    });
    assert_eq!(
        lot_vault(&mut rig, LotId(0xe1b3)),
        Some(hold),
        "the player's lot, whole, in the captain's hold"
    );
    let gone = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|m| {
            m.personal_listings_of(&mush_org::PrincipalId::Object(seller))
                .count()
                == 0
        });
    assert!(gone, "the player's lot sold whole");
}

/// **Doc 48e, exchange G6 (48d H-05): charter cargo is listed by its pilot.** A chartered pilot
/// who officers an organization posts the chartered hold's lot `as=<org>`: refused, since the
/// Co-op's share on a personal ask is decided by its seller and an org seller would pay the Trust
/// nothing. As themself, the same lot lists. Before the fix the org ask listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g6_charter_cargo_is_not_listed_for_an_org() {
    let mut rig = crate::driving_rig::stocked();
    let (captain, row) = prospector(&mut rig, "Officer", 44, "Wren");
    for line in [
        "@org/create Lark".to_owned(),
        format!("@org/grant #{}=Lark economy.account.post", captain.0),
        format!("@account/open org:Lark={}", tacsim::CREDIT),
    ] {
        let out = run(&mut rig.tw, mush_core::GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, SIGHT, 1, LotId(0xe1b6));
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    let refused = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {} price=500 as=Lark", LotId(0xe1b6)),
    );
    assert!(
        refused.contains(&tacsim::port::proceeds::CHARTER_ASK_AS_ORG.replace("{name}", "Wren")),
        "{refused}"
    );
    let listed = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {} price=500", LotId(0xe1b6)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
}

/// **Doc 48e, exchange G11 (48d H-01): a bid delivers only into a hold docked at its venue.** A
/// captain bids for a part into her hold at Ceres, then sails. A seller at Ceres sells a part
/// into the bid: refused, the hold is under way, and nothing moves. Before the fix the sale was
/// admitted (only the seller's source was gated), and the part was delivered into a ship in
/// flight (against Q-48-6, local delivery).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g11_a_bid_into_a_hold_that_has_sailed_takes_nothing() {
    let mut rig = crate::driving_rig::stocked();
    let (captain, row) = prospector(&mut rig, "Bidder", 45, "Kestrel");
    mint(&mut rig, captain);
    let hold = hold_vault_of(&rig.tw, &row);
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    let said = run(
        &mut rig.tw,
        captain,
        &format!(
            "+market/bid tacsim:{SIGHT} qty=1 price=1000 into={}",
            hold.0
        ),
    );
    assert!(said.contains(" is held from your account."), "{said}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let bid = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.personal_listings_of(&mush_org::PrincipalId::Object(captain))
                .find(|l| l.source.is_wanted())
                .map(|l| l.id)
        })
        .expect("the bid");
    // She sails.
    crate::driving_rig::load(&mut rig, &row, 60, 20, 98_601);
    crate::driving_rig::board(&mut rig, captain);
    let (said, queued) = crate::driving_rig::command(&mut rig, captain, "+ship/undock stickney");
    assert!(said.contains("Clearance filed"), "{said}");
    crate::driving_rig::land_quietly(&mut rig, &queued);
    for _ in 0..48 {
        if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), mush_core::ObjId(row.hull)),
            tacsim::port::ship::Berth::UnderWay
        ) {
            break;
        }
        crate::driving_rig::sweep(&mut rig.tw, 600);
        let queued = drain(&mut rig.tw);
        crate::driving_rig::land_quietly(&mut rig, &queued);
    }
    // A seller at Ceres, a part in their locker, sells into her bid: refused.
    let seller = with_account(&mut rig, "Seller", 46);
    teleport(&mut rig.tw, seller, exchange);
    let _ = run(&mut rig.tw, seller, "+locker/open");
    let locker = locker_vault(&rig, seller).expect("the locker");
    seed_lot(&mut rig, locker, SIGHT, 1, LotId(0xe1b7));
    let refused = run(
        &mut rig.tw,
        seller,
        &format!("+market/sell {}=1 from={}", bid.0, locker.0),
    );
    assert!(
        refused.contains("under way") || refused.contains("Kestrel"),
        "{refused}"
    );
    assert!(!refused.contains("Sold "), "{refused}");
    assert_eq!(
        hold_pool(&mut rig.tw, hold, SIGHT),
        0,
        "nothing reached the hold"
    );
}

/// **Doc 48e, exchange G12 (48c #5, 48d H-10): concurrent fills cannot overfill a bid's
/// destination.** A buyer bids for 30 t of metals into her 20 t locker. Two sellers each sell 15 t
/// into it in the same step, before either delivery lands, so each sale's room check reads the
/// locker empty. The deliveries carry the locker's plan: the first lands, the second is refused
/// for room inside its own bundle, and the locker holds 15 t. Before the fix both landed: 30 t in a
/// 20 t locker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g12_two_fills_into_one_locker_are_each_asked_for_room() {
    let mut rig = crate::driving_rig::stocked();
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    let buyer = with_account(&mut rig, "Buyer", 47);
    mint(&mut rig, buyer);
    teleport(&mut rig.tw, buyer, exchange);
    let _ = run(&mut rig.tw, buyer, "+locker/open");
    let locker = locker_vault(&rig, buyer).expect("the locker");
    let said = run(
        &mut rig.tw,
        buyer,
        &format!(
            "+market/bid tacsim:metals qty=30 price=100 into={}",
            locker.0
        ),
    );
    assert!(said.contains(" is held from your account."), "{said}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let bid = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.personal_listings_of(&mush_org::PrincipalId::Object(buyer))
                .find(|l| l.source.is_wanted())
                .map(|l| l.id)
        })
        .expect("the bid");
    let mut sellers = Vec::new();
    for (n, name) in ["Ana", "Bram"].into_iter().enumerate() {
        let seller = with_account(&mut rig, name, 48 + i64::try_from(n).expect("n"));
        teleport(&mut rig.tw, seller, exchange);
        let _ = run(&mut rig.tw, seller, "+locker/open");
        let theirs = locker_vault(&rig, seller).expect("a locker");
        crate::driving_rig::seed_pool(
            &mut rig,
            theirs,
            "metals",
            15,
            LotId(0xe1c0 + u64::try_from(n).expect("n")),
        );
        sellers.push((seller, theirs));
    }
    // Both sell before either delivery lands.
    let mut queued = Vec::new();
    for (seller, theirs) in &sellers {
        let out = run(
            &mut rig.tw,
            *seller,
            &format!("+market/sell {}=15 from={}", bid.0, theirs.0),
        );
        assert!(out.contains("Sold "), "{out}");
        queued.extend(drain(&mut rig.tw));
    }
    crate::driving_rig::land_quietly(&mut rig, &queued);
    for _ in 0..20 {
        let (_, more) = fire(&mut rig);
        crate::driving_rig::land_quietly(&mut rig, &more);
    }
    assert_eq!(
        hold_pool(&mut rig.tw, locker, "metals"),
        15,
        "the locker takes one fill; the second has no room"
    );
}

/// **Doc 48e, exchange G4 (48c #2, 48d H-04; Q-48-8): a sale into a person's bid pays the
/// venue's cut and levy.** A buyer bids 10 cr a tonne for 10 t of metals into her locker; a
/// seller sells 10 t into it from his. Once the goods are in, the port authority has the 3%
/// cut, the treasury the 1% levy, and the seller the other 96%. Before the fix the seller was
/// paid the whole 100 cr and the venue nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g4_a_sale_into_a_persons_bid_pays_the_venue_cut_and_levy() {
    let mut rig = crate::driving_rig::stocked();
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    let buyer = with_account(&mut rig, "Buyer", 51);
    mint(&mut rig, buyer);
    teleport(&mut rig.tw, buyer, exchange);
    let _ = run(&mut rig.tw, buyer, "+locker/open");
    let locker = locker_vault(&rig, buyer).expect("the locker");
    let said = run(
        &mut rig.tw,
        buyer,
        &format!(
            "+market/bid tacsim:metals qty=10 price=1000 into={}",
            locker.0
        ),
    );
    assert!(said.contains(" is held from your account."), "{said}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let bid = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.personal_listings_of(&mush_org::PrincipalId::Object(buyer))
                .find(|l| l.source.is_wanted())
                .map(|l| l.id)
        })
        .expect("the bid");
    let seller = with_account(&mut rig, "Seller", 52);
    teleport(&mut rig.tw, seller, exchange);
    let _ = run(&mut rig.tw, seller, "+locker/open");
    let theirs = locker_vault(&rig, seller).expect("a locker");
    crate::driving_rig::seed_pool(&mut rig, theirs, "metals", 10, LotId(0xe1d0));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let (operator, sink) = (
        crate::ex4_charges::authority(&rig),
        crate::ex4_charges::treasury(&rig),
    );
    let paid = crate::hull_resale_ex2::account_of(&rig, seller);
    let out = run(
        &mut rig.tw,
        seller,
        &format!("+market/sell {}=10 from={}", bid.0, theirs.0),
    );
    assert!(out.contains("Sold "), "{out}");
    assert!(
        out.contains("venue fees "),
        "the line names the fees: {out}"
    );
    let queued = drain(&mut rig.tw);
    crate::driving_rig::land_quietly(&mut rig, &queued);
    for _ in 0..20 {
        let (_, more) = fire(&mut rig);
        crate::driving_rig::land_quietly(&mut rig, &more);
    }
    assert_eq!(hold_pool(&mut rig.tw, locker, "metals"), 10, "delivered");
    // The sale contract's payouts (read from the contract, not the balances, which the
    // lockers' rent also moves).
    let world = rig.tw.world();
    let contract = world
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.quotes_of_seller(&mush_org::PrincipalId::Object(seller))
                .last()
                .and_then(|q| q.contract)
        })
        .expect("the sale's contract");
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy");
    let c = economy.snapshot().contract(contract).expect("the contract");
    assert!(
        matches!(c.state, mush_economy::ContractState::Settled { .. }),
        "settled: {:?}",
        c.state
    );
    let exact = |to, minor| mush_economy::PayoutShare {
        to,
        share: mush_economy::Share::Exact(mush_economy::Amount::from_minor(minor)),
    };
    assert_eq!(
        c.consent.terms.outcomes[0].payouts,
        vec![exact(paid, 9_600), exact(operator, 300), exact(sink, 100)],
        "the seller 96%, the port authority the 3% cut, the treasury the 1% levy"
    );
}

/// **Doc 48e, exchange G15 (48d H-06): a scrap vault takes only what the yard scraps.** A player
/// with a scrap vault at Ceres names it as a bid's destination and as a purchase's: both are
/// refused, and nothing is held or bought. Before the fix both were admitted (the scrap owner's
/// bypass let any use of the vault through), so it was rent-free, uncapped storage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g15_a_scrap_vault_is_no_purchase_destination() {
    let mut rig = crate::driving_rig::stocked();
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    let buyer = with_account(&mut rig, "Scrapper", 53);
    mint(&mut rig, buyer);
    teleport(&mut rig.tw, buyer, exchange);
    let crate_ = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Scrap crate", buyer, Some(buyer))
        .expect("a crate");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create scrap at ceres=container:#{} owner=#{}",
            crate_.0, buyer.0
        ),
    );
    let scrap = tacsim::port::bay_work::scrap::scrap_vault(rig.tw.world(), buyer, "ceres")
        .unwrap_or_else(|| panic!("the scrap vault: {out}"));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let refused = run(
        &mut rig.tw,
        buyer,
        &format!(
            "+market/bid tacsim:metals qty=10 price=100 into={}",
            scrap.0
        ),
    );
    assert!(
        refused.contains(tacsim::port::market::MOVE_INTO_SCRAP),
        "a bid into it: {refused}"
    );
    let listing = metals_listing(&rig);
    let said = run(
        &mut rig.tw,
        buyer,
        &format!("+market/quote {}=1", listing.0),
    );
    let quote = crate::hull_resale_ex2::newest(&rig, buyer).unwrap_or_else(|| panic!("{said}"));
    let refused = run(
        &mut rig.tw,
        buyer,
        &format!("+market/accept {}={}", quote.0, scrap.0),
    );
    assert!(
        refused.contains(tacsim::port::market::MOVE_INTO_SCRAP),
        "a purchase into it: {refused}"
    );
    assert!(drain(&mut rig.tw).is_empty(), "nothing was queued");
}

/// **Doc 48e, exchange G18 (48d H-11): a listing whose hold is never placed gives its fee
/// back.** A player posts a lot from her locker and pays the 10 cr listing fee; the hold's
/// submission is lost before it lands. The reconcile withdraws the listing and refunds the fee
/// it paid. Before the fix the listing was withdrawn and the fee kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g18_a_listing_whose_hold_never_lands_refunds_its_fee() {
    let mut rig = crate::driving_rig::stocked();
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    let seller = with_account(&mut rig, "Lister", 54);
    mint(&mut rig, seller);
    teleport(&mut rig.tw, seller, exchange);
    let _ = run(&mut rig.tw, seller, "+locker/open");
    let locker = locker_vault(&rig, seller).expect("the locker");
    seed_lot(&mut rig, locker, SIGHT, 1, LotId(0xe1d8));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let said = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {} price=500", LotId(0xe1d8)),
    );
    assert!(said.contains("Listed for sale as #"), "{said}");
    let listing = said
        .split("Listed for sale as #")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<u64>().ok())
        .map(mush_market::ListingId)
        .expect("a listing id");
    // The hold's submission is lost: never landed.
    let lost = drain(&mut rig.tw);
    assert!(!lost.is_empty(), "the hold was staged");
    let refunded = |rig: &crate::driving_rig::Rig| {
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .expect("economy")
            .transactions()
            .any(|t| t.description == format!("market listing #{} fee refunded", listing.0))
    };
    for _ in 0..12 {
        if refunded(&rig) {
            break;
        }
        crate::driving_rig::sweep(&mut rig.tw, 30);
        let queued = drain(&mut rig.tw);
        crate::driving_rig::land_quietly(&mut rig, &queued);
    }
    assert!(
        rig.tw
            .world()
            .module_state::<mush_market::MarketStore>()
            .and_then(|m| m.listing(listing))
            .is_none(),
        "the listing is withdrawn"
    );
    assert!(refunded(&rig), "its fee is refunded");
}
