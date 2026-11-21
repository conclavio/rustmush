//! **Doc 50 cut OP.5 — the import duty at sale** (Q-50-29, Q-50-59, Q-50-64), over the composed
//! world (`driving`'s rig).
//!
//! A pilot flies her Dray-P from Ceres by Stickney to the Isidis Concession and sells propellant
//! from her docked hull's hold into the tank farm's standing bid:
//! - **under the Freeport**, who levies nothing there, she is paid the sale, less only the
//!   Co-op's share (hers is a Co-op hull);
//! - **under the Republic**, whose schedule row lists propellant among the Belt's exports, 5% of
//!   the goods' catalog value comes out of the proceeds, paid to Republic Customs, and the sale's
//!   line names the duty with its reason.
//!
//! Doc 50b item 8: **an organization's listing pays it too.** An officer lists a lot of
//! propellant from her locker at Isidis for her organization (`as=`); under the Republic the
//! buyer's purchase pays Republic Customs the duty out of the organization's proceeds.
//!
//! **What is a fixture.** The propellant seeded in her hold (or her locker) at Isidis for each
//! sale, the stores for her two hops, the organization and its account (staff's `@org`), the
//! buyer's credit (`@econ/mint`), and the Republic's de facto hold on the Concession through
//! staff's `@faction/control`, as the operation's resolution writes it.

use mush_core::{GOD, ObjId, World};
use mush_economy::{AccountId, EconomyStore};
use mush_inventory::LotId;

use crate::driving_rig::{
    Rig, advance_to, board, cid, drain, file_and_launch, hold_vault_of, land_all, land_quietly,
    load, pilot, port_map, prospector, run, seed_lot, seed_pool, station_room, stocked, sweep,
    teleport,
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

/// The Co-op shares counted on her enrollment, minor units.
fn shares(rig: &Rig, pilot: ObjId) -> i64 {
    crate::driving_rig::store(&rig.tw)
        .enrolments()
        .find(|e| e.pilot == pilot)
        .map_or(0, |e| e.shares_minor)
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

/// The tank farm's propellant bid.
fn farm_bid(rig: &Rig) -> u64 {
    port_map(&rig.tw, "isidis_port").vendors[&cid("isidis_tank_farm")].bids[&cid("propellant")]
        .listing
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
    // Her delivery leg, then the sweeps that settle the sale.
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    for _ in 0..6 {
        sweep(&mut rig.tw, 31);
    }
    crate::in10_pools::settle(rig, 3);
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

    // The stock rig lays the Freeport arm up; the farm is read all the same, so the operation's
    // close probe has its answer.
    assert!(
        tacsim::stockpile::arm(rig.tw.world()).is_some_and(|a| a.laid_up && a.level.is_some()),
        "the tank farm is read though the arm is laid up"
    );

    // Under the Freeport: nothing levied.
    let before = (
        balance(rig.tw.world(), own_account(&rig, pilot)),
        balance(rig.tw.world(), customs(&rig)),
    );
    let bid = farm_bid(&rig);
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {bid}={QTY}"));
    assert!(!quoted.contains("Out of it"), "nothing levied: {quoted}");
    let counted = shares(&rig, pilot);
    let line = sell(&mut rig, pilot, hold, 0x0f55);
    let share = shares(&rig, pilot) - counted;
    assert!(share > 0, "the Co-op's share is counted: {line}");
    assert!(!line.contains("Import duty"), "{line}");
    let free_net = balance(rig.tw.world(), own_account(&rig, pilot)) - before.0;
    assert!(
        free_net > 0 && free_net <= gross,
        "she is paid the sale, less only the Co-op's share: {free_net} of {gross}"
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
    let bid = farm_bid(&rig);
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {bid}={QTY}"));
    assert!(
        quoted.contains(&format!(
            "Out of it you pay import duty {:.2}cr",
            f64::from(i32::try_from(duty).expect("fits")) / 100.0
        )) && quoted.contains("5% of the catalog value"),
        "doc 50b item 5: the bid's quote shows the duty and why: {quoted}"
    );
    let counted = shares(&rig, pilot);
    let line = sell(&mut rig, pilot, hold, 0x0f57);
    assert_eq!(
        shares(&rig, pilot) - counted,
        share,
        "doc 50b item 6: the duty is never counted among her Co-op shares"
    );
    assert!(
        line.contains("Import duty")
            && line.contains("5% of the catalog value")
            && line.contains("Republic"),
        "the line names the duty and why: {line}"
    );
    assert!(
        line.contains(&format!(
            "you receive {:.2}cr",
            f64::from(i32::try_from(free_net - duty).expect("fits")) / 100.0
        )),
        "what she receives is the sale less the share and the duty: {line}"
    );
    assert_eq!(
        balance(rig.tw.world(), customs(&rig)) - before.1,
        duty,
        "the duty is Republic Customs'"
    );
    assert_eq!(
        balance(rig.tw.world(), own_account(&rig, pilot)) - before.0,
        free_net - duty,
        "it comes out of her proceeds: the same sale pays her the duty less"
    );
}

/// A player at the Isidis market with a credit account and an open locker there: the player and
/// the locker's vault.
fn at_isidis(rig: &mut Rig, who: &str, seed: i64) -> (ObjId, mush_economy::VaultId) {
    let player = pilot(rig, who, seed);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", player.0),
    );
    let market = station_room(&rig.tw, "isidis_port", "room.i03_market");
    teleport(&mut rig.tw, player, market);
    let opened = run(&mut rig.tw, player, "+locker/open");
    let venue = mush_market::MarketId::new("isidis").expect("venue");
    let locker = tacsim::port::locker::locker_of(rig.tw.world(), player, &venue)
        .map(|l| mush_economy::VaultId(l.vault))
        .unwrap_or_else(|| panic!("a locker at Isidis: {opened}"));
    (player, locker)
}

/// **Doc 50b item 8: an organization's listing pays the import duty** (Q-50-59: "into a bid or
/// from a listing"). Under the Republic an officer lists 10 t of propellant from her own locker at
/// Isidis for her organization (`+market/post … as=`), and a buyer takes it up: Republic Customs
/// is paid 5% of the catalog value, out of the organization's proceeds. Before the fix the market
/// asked the proceeds policy only of a person's listing, and the sale went duty-free.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op5_an_organizations_listing_pays_the_import_duty() {
    let mut rig = stocked();
    let price = 6_000_i64;
    let gross = i64::try_from(QTY).expect("fits") * price;
    let duty = i64::try_from(QTY).expect("fits") * 40 * 100 * 50 / 1_000;
    let held = run(
        &mut rig.tw,
        GOD,
        "@faction/control Isidis Concession=Republic/defacto",
    );
    assert!(!held.contains("Usage"), "{held}");
    let (officer, locker) = at_isidis(&mut rig, "Officer", 92);
    for line in [
        "@org/create Lark".to_owned(),
        format!("@org/grant #{}=Lark economy.account.post", officer.0),
        format!("@account/open org:Lark={}", tacsim::CREDIT),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    // The organization pays its listing fee from its own account.
    let org = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == "Lark")
                .map(|org| mush_org::PrincipalId::Organization(org.id))
        })
        .expect("Lark");
    let org_account = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&org)
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("Lark's account");
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 100000", org_account.0),
    );
    seed_lot(&mut rig, locker, "propellant", QTY, LotId(0x0f59));
    let listed = run(
        &mut rig.tw,
        officer,
        &format!("+market/post {} price={price} as=Lark", LotId(0x0f59)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
    let id: u64 = listed
        .split("Listed for sale as #")
        .nth(1)
        .and_then(|rest| {
            rest.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or_else(|| panic!("a listing id: {listed}"));
    let lark = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| m.listing(mush_market::ListingId(id)).cloned())
        .map(|l| {
            assert!(
                matches!(l.seller, mush_org::PrincipalId::Organization(_)),
                "listed for the organization: {l:?}"
            );
            l.account.expect("the organization's account")
        })
        .expect("the organization's listing");

    let (buyer, into) = at_isidis(&mut rig, "Buyer", 93);
    let wallet = own_account(&rig, buyer);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", wallet.0),
    );
    let before = (
        balance(rig.tw.world(), lark),
        balance(rig.tw.world(), customs(&rig)),
    );
    let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {id}"));
    assert!(
        quoted.contains("Out of it the seller pays import duty 20.00cr")
            && quoted.contains("5% of the catalog value"),
        "doc 50b item 5: the quote shows the duty and why: {quoted}"
    );
    let quote = quoted
        .split("Quote #")
        .nth(1)
        .and_then(|rest| rest.split(':').next())
        .unwrap_or_else(|| panic!("a quote: {quoted}"))
        .to_owned();
    let accepted = run(
        &mut rig.tw,
        buyer,
        &format!("+market/accept {quote} to={}", into.0),
    );
    assert!(!accepted.contains("cannot"), "{accepted}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..6 {
        sweep(&mut rig.tw, 31);
    }
    crate::in10_pools::settle(&mut rig, 3);
    assert_eq!(
        balance(rig.tw.world(), customs(&rig)) - before.1,
        duty,
        "Republic Customs is paid the duty on the organization's sale: {accepted}"
    );
    let paid = balance(rig.tw.world(), lark) - before.0;
    assert!(
        paid > 0 && paid <= gross - duty,
        "the duty comes out of the organization's proceeds: {paid} of {gross}"
    );
}
