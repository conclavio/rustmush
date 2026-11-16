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
fn owed(rig: &crate::driving_rig::Rig, player: ObjId) -> Option<i64> {
    let venue = mush_market::MarketId::new("ceres")?;
    tacsim::port::locker::locker_of(rig.tw.world(), player, &venue).map(|l| l.owed_minor)
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

    // Two IC days on: three tonnes owe 6 cr, accrued when shown.
    crate::in10_pools::days_on(&mut rig, 2);
    let shown = run(&mut rig.tw, walker, "+locker");
    assert!(shown.contains("owed (collection starts later)."), "{shown}");
    assert_eq!(owed(&rig, walker), Some(600), "{shown}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex1b_a_chartered_hold_cannot_post_a_personal_ask() {
    let mut rig = crate::driving_rig::stocked();
    let (captain, row) = prospector(&mut rig, "Charter", 42, "Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, SIGHT, 1, LotId(0xe1b1));
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    let refused = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {} price=500", LotId(0xe1b1)),
    );
    assert!(
        refused.contains(tacsim::port::proceeds::CHARTERED_ASK),
        "a chartered hold's lot is refused: {refused}"
    );

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
