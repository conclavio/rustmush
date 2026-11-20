//! **Doc 50 cut OP.5 — the import duty at sale** (Q-50-29, Q-50-59, Q-50-64), over the composed
//! world (`driving`'s rig).
//!
//! A pilot flies her Dray-P from Ceres by Stickney to the Isidis Concession and sells propellant
//! from her docked hull's hold into the tank farm's standing bid:
//! - **under the Freeport**, who levies nothing there, she is paid the whole sale;
//! - **under the Republic**, whose schedule row lists propellant among the Belt's exports, 5% of
//!   the goods' catalog value comes out of the proceeds, paid to Republic Customs, and the sale's
//!   line names the duty with its reason.
//!
//! **What is a fixture.** The propellant seeded in her hold at Isidis for each sale, the stores
//! for her two hops, and the Republic's de facto hold on the Concession through staff's
//! `@faction/control`, as the operation's resolution writes it.

use mush_core::{GOD, ObjId, World};
use mush_economy::{AccountId, EconomyStore};
use mush_inventory::LotId;

use crate::driving_rig::{
    Rig, advance_to, board, cid, file_and_launch, hold_vault_of, land_quietly, load, port_map,
    prospector, run, seed_pool, station_room, stocked, teleport,
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

/// One hop, filed and flown to its arrival, its landings landed.
fn hop(rig: &mut Rig, pilot: ObjId, hull: ObjId, to: &str) {
    let ticket = file_and_launch(rig, pilot, hull, &format!("+ship/undock {to}"));
    let (_, queued) = advance_to(rig, ticket.plan.depart_ic + ticket.plan.total_s + 3_600);
    land_quietly(rig, &queued);
    crate::in10_pools::settle(rig, 3);
}

/// Sell [`QTY`] t of propellant from her hold into the tank farm's bid; the sale's line.
fn sell(rig: &mut Rig, pilot: ObjId, hold: mush_economy::VaultId, lot: u64) -> String {
    seed_pool(rig, hold, "propellant", QTY, LotId(lot));
    let bid = port_map(&rig.tw, "isidis_port").vendors[&cid("isidis_tank_farm")].bids
        [&cid("propellant")]
        .listing;
    let sold = run(
        &mut rig.tw,
        pilot,
        &format!("+market/sell {bid}={QTY} from={}", hold.0),
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
    let (pilot, row) = prospector(&mut rig, "Seller", 91, "Brine Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    load(&mut rig, &row, 60, 40, 0x0f51);
    board(&mut rig, pilot);
    hop(&mut rig, pilot, hull, "stickney");
    load(&mut rig, &row, 20, 4, 0x0f53);
    hop(&mut rig, pilot, hull, "isidis");
    // Ashore, at the Concession's market.
    let ashore = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(!ashore.contains("Usage"), "{ashore}");
    let market = station_room(&rig.tw, "isidis_port", "room.i03_market");
    teleport(&mut rig.tw, pilot, market);

    // Under the Freeport: nothing levied.
    let before = (
        balance(rig.tw.world(), own_account(&rig, pilot)),
        balance(rig.tw.world(), customs(&rig)),
    );
    let line = sell(&mut rig, pilot, hold, 0x0f55);
    assert!(!line.contains("Import duty"), "{line}");
    assert_eq!(
        balance(rig.tw.world(), own_account(&rig, pilot)) - before.0,
        gross,
        "the whole sale is hers"
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

    // Under the Republic: 5% of the catalog value, to Republic Customs, out of her proceeds.
    let before = (
        balance(rig.tw.world(), own_account(&rig, pilot)),
        balance(rig.tw.world(), customs(&rig)),
    );
    let line = sell(&mut rig, pilot, hold, 0x0f57);
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
        balance(rig.tw.world(), own_account(&rig, pilot)) - before.0,
        gross - duty,
        "it comes out of her proceeds"
    );
}
