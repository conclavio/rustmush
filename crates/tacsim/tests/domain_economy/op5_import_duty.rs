//! **Doc 50 cut OP.5 — the import duty at sale** (Q-50-29, Q-50-59, Q-50-64), over the composed
//! world (`driving`'s rig).
//!
//! A seller standing at the Isidis Concession sells propellant from their locker into the tank
//! farm's standing bid:
//! - **under the Freeport**, who levies nothing there, the seller is paid the whole sale;
//! - **under the Republic**, whose schedule row lists propellant among the Belt's exports, 5% of
//!   the goods' catalog value comes out of the proceeds, paid to Republic Customs, and the sale's
//!   line names the duty with its reason.
//!
//! **What is a fixture.** The propellant in the seller's locker, and the Republic's de facto hold
//! on the Concession through staff's `@faction/control`, as the operation's resolution writes it.

use mush_core::{GOD, ObjId, World};
use mush_economy::{AccountId, EconomyStore, VaultId};
use mush_inventory::LotId;

use crate::driving_rig::{
    Rig, cid, pilot, port_map, run, seed_pool, station_room, stocked, teleport,
};

const QTY: u64 = 10;

fn balance(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .map_or(0, |a| a.posted_balance.minor())
}

fn own_account(rig: &Rig, who: ObjId) -> AccountId {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .map(|a| a.id)
        .expect("an account")
}

fn customs(rig: &Rig) -> AccountId {
    tacsim::polity::lane_org_account(rig.tw.world(), &cid("republic_customs"))
        .expect("the customs office's account")
        .1
}

/// A seller at Isidis's market with [`QTY`] t of propellant in their locker there.
fn seller(rig: &mut Rig, who: &str, seed: i64, lot: u64) -> (ObjId, VaultId) {
    let seller = pilot(rig, who, seed);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", seller.0),
    );
    let market = station_room(&rig.tw, "isidis_port", "room.i03_market");
    teleport(&mut rig.tw, seller, market);
    let _ = run(&mut rig.tw, seller, "+locker/open");
    let venue = mush_market::MarketId::new("isidis").expect("a venue");
    let locker = tacsim::port::locker::locker_of(rig.tw.world(), seller, &venue)
        .map(|l| VaultId(l.vault))
        .expect("a locker at Isidis");
    seed_pool(rig, locker, "propellant", QTY, LotId(lot));
    (seller, locker)
}

/// Sell into the tank farm's propellant bid from `locker`; the line, once the sale has settled.
fn sell(rig: &mut Rig, seller: ObjId, locker: VaultId) -> String {
    let bid = port_map(&rig.tw, "isidis_port").vendors[&cid("isidis_tank_farm")].bids
        [&cid("propellant")]
        .listing;
    let sold = run(
        &mut rig.tw,
        seller,
        &format!("+market/sell {bid}={QTY} from={}", locker.0),
    );
    assert!(sold.contains("Sold "), "{sold}");
    crate::in10_pools::settle(rig, 6);
    sold
}

/// **Doc 50 cut OP.5: the import duty follows the holder** (Q-50-29, Q-50-59, Q-50-64).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op5_the_import_duty_follows_the_concessions_holder() {
    let mut rig = stocked();
    // The bid pays 50 cr a tonne; the catalog value is 40 cr a tonne.
    let gross = i64::try_from(QTY).expect("fits") * 50 * 100;
    let duty = i64::try_from(QTY).expect("fits") * 40 * 100 * 50 / 1_000;

    // Under the Freeport: nothing levied.
    let (free, locker) = seller(&mut rig, "Free", 91, 0x0f51);
    let before = (
        balance(rig.tw.world(), own_account(&rig, free)),
        balance(rig.tw.world(), customs(&rig)),
    );
    let line = sell(&mut rig, free, locker);
    assert!(!line.contains("Import duty"), "{line}");
    assert_eq!(
        balance(rig.tw.world(), own_account(&rig, free)) - before.0,
        gross,
        "the whole sale is the seller's"
    );
    assert_eq!(
        balance(rig.tw.world(), customs(&rig)),
        before.1,
        "customs is paid nothing"
    );

    // The Republic takes the Concession, as the operation's resolution writes it.
    let held = run(
        &mut rig.tw,
        GOD,
        "@faction/control Isidis Concession=Republic/defacto",
    );
    assert!(!held.contains("Usage"), "{held}");

    // Under the Republic: 5% of the catalog value, to Republic Customs, out of the proceeds.
    let (taxed, locker) = seller(&mut rig, "Taxed", 92, 0x0f52);
    let before = (
        balance(rig.tw.world(), own_account(&rig, taxed)),
        balance(rig.tw.world(), customs(&rig)),
    );
    let line = sell(&mut rig, taxed, locker);
    assert!(
        line.contains("Import duty")
            && line.contains("5% of the catalog value")
            && line.contains("Republic"),
        "the line names the duty and why: {line}"
    );
    assert_eq!(
        balance(rig.tw.world(), customs(&rig)) - before.1,
        duty,
        "the duty is Republic Customs'"
    );
    assert_eq!(
        balance(rig.tw.world(), own_account(&rig, taxed)) - before.0,
        gross - duty,
        "it comes out of the seller's proceeds"
    );
}
