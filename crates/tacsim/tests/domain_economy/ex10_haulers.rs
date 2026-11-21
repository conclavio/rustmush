//! **Doc 48 unit EX.10 — the NPC hauling house** (Q-48-20, Q-48-41), over the composed world
//! (`driving`'s keyed rig), through the port tick, the IC11 executor and the voyage path.
//!
//! Lane v43 gives Ceres the Vesta Carrying Company: two Drays, `VCC-01` based at Ceres and
//! `VCC-02` at Luna, and lane v54 a third, `VCC-03` at Ceres (doc 50 cut OP.8's second Ceres
//! hull), that carry water, carbon feedstock, alloys and boards from one lane port's
//! house ask to another's house bid whenever the gap pays for the trip. It also gives the Deimos
//! reserve, at a port that produces nothing, its people's use of water and carbon. The house
//! trades from the IC day after the tick first sees it.
//!
//! - `ex10_the_house_seeds_its_hulls_and_trades_where_a_gap_pays`: each hull is seeded at its
//!   home, laid up and titled to the house, gets a helm, its grants and a purse, and on the
//!   house's first day sets out on a trip the gap pays for. A trip runs as a sailing plan the
//!   executor flies as the helm: the lot bought, the stores filled, the passage, the sale. The
//!   bidder takes the lot, and what the trip took in is past what it paid.
//! - `ex10_a_hauler_meets_the_kirkwood_picket`: Ceres's hauler sails for Stickney and is promoted
//!   into the Kirkwood picket's bubble at the interdicted leg, with her house's mind at her helm:
//!   she can be intercepted like any crewed hull.
//! - `ex10_consumption_switches_on_at_a_port_with_no_producer`: the Deimos reserve uses up what
//!   it holds of water and carbon, 15 t and 5 t an IC day, and the export faucet pays it back.
//! - `ex10_a_hauler_supplies_deimos_from_luna`: Luna's hauler carries Luna's boards or alloys to
//!   Deimos, a port that produces nothing, sells them into Syrtis Bonded's bid, and Syrtis uses them
//!   up.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the goods in the reserve's storefront in the third story (a
//! stand-in for a delivery); staff moves of the Habitat Board's money. Every trip, purchase,
//! passage, sale and payment is the port tick's.

use mush_core::{ObjId, World};
use mush_economy::{AccountId, EconomyStore, VaultId};
use mush_inventory::LotId;
use mush_org::{OrgId, PrincipalId};
use tacsim::port::consume::{ConsumeHandles, ConsumeId, ConsumeLedger};
use tacsim::port::hauler::{HaulerHouse, HaulerRow};
use tacsim::port::ship::Berth;

use crate::driving_rig::{
    Rig, cid, drain, hold_pool, ic_now, land_quietly, port_map, run, seed_pool, stocked, store,
    sweep, to_ic,
};

const DAY: i64 = 86_400;
const CERES: &str = "ceres_port";
const DEIMOS: &str = "deimos_port";

// ---- readings --------------------------------------------------------------------------

fn house(rig: &Rig) -> HaulerHouse {
    store(&rig.tw)
        .hauler(&cid(CERES))
        .cloned()
        .expect("Ceres keeps its hauling house")
}

fn row(rig: &Rig, mark: &str) -> HaulerRow {
    house(rig)
        .rows
        .into_iter()
        .find(|row| row.mark == mark)
        .unwrap_or_else(|| panic!("a row for {mark}"))
}

fn balance(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .map_or(0, |a| a.posted_balance.minor())
}

fn vesta(rig: &Rig) -> OrgId {
    OrgId(port_map(&rig.tw, CERES).orgs[&cid("vesta_carriers")].org)
}

fn ledger(rig: &mut Rig, entry: &str, org: &str, good: &str) -> Option<ConsumeLedger> {
    let map = port_map(&rig.tw, entry);
    let id = ConsumeId::derive(&map.pack, &map.entry, &cid(org), &cid(good)).expect("an id");
    let handles = rig
        .tw
        .world()
        .module_state::<ConsumeHandles>()
        .cloned()
        .expect("the consumption handles");
    crate::driving_rig::typed(&mut rig.tw, &handles.ledgers, &id)
}

/// A hull's sailing plan's log and its orders' states, for a failure's message.
fn plan_log(rig: &Rig, hull: ObjId) -> String {
    match tacsim::port::orders::orders_state(rig.tw.world(), hull) {
        tacsim::port::orders::OrdersState::Present(doc) => format!(
            "state {:?}, hold {:?}, orders {:?}, log {:?}",
            doc.state,
            doc.hold,
            doc.orders
                .iter()
                .map(|o| (o.id, o.state, format!("{:?}", o.what)))
                .collect::<Vec<_>>(),
            doc.log.iter().map(|l| l.text.clone()).collect::<Vec<_>>()
        ),
        _ => "no plan".to_owned(),
    }
}

/// Every row and its plan, for a failure's message.
fn the_house(rig: &Rig) -> String {
    let rows = house(rig).rows;
    let logs: Vec<String> = rows
        .iter()
        .filter_map(|row| row.hull.map(ObjId))
        .map(|hull| plan_log(rig, hull))
        .collect();
    format!("{rows:?}\n{}", logs.join("\n"))
}

/// The keyed rig, its hauling house laid up by the stock rig and opened again by staff: the
/// `@tacsim/port haulers` reply names each hull.
fn opened() -> Rig {
    let mut rig = stocked();
    let said = run(
        &mut rig.tw,
        mush_core::GOD,
        "@tacsim/port haulers ceres_port on",
    );
    assert!(said.contains("ceres_port hauling house: open"), "{said}");
    rig
}

/// Port fires (31 s apart), every submission they stage landed.
fn fires(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
}

/// Step the IC clock `secs` on in one jump, then a few fires landed.
fn ahead(rig: &mut Rig, secs: i64) {
    let now = ic_now(&rig.tw);
    let _ = to_ic(rig, now + secs);
    fires(rig, 3);
}

/// Staff move all of `org`'s money at `entry` to the port's treasury.
fn drain_org(rig: &mut Rig, entry: &str, org: &str) {
    let account = port_map(&rig.tw, entry).orgs[&cid(org)]
        .account
        .map(AccountId)
        .expect("an account");
    let treasury = port_map(&rig.tw, entry).treasury.expect("a treasury");
    let all = balance(rig.tw.world(), account);
    let _ = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/pay {}={treasury} {all}", account.0),
    );
    assert_eq!(balance(rig.tw.world(), account), 0, "{org} is drained");
}

/// The house's hulls: VCC-01 and VCC-03 at Ceres, VCC-02 at Luna (lane v54).
const HULLS: usize = 3;

/// The house's first day, its hulls laid up and their first trips planned.
fn first_trips(rig: &mut Rig) -> Vec<HaulerRow> {
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(rig, day * DAY + 600);
    for _ in 0..240 {
        let rows = house(rig).rows;
        if rows.len() == HULLS && rows.iter().all(|row| row.trip.is_some()) {
            return rows;
        }
        fires(rig, 1);
    }
    panic!("the hauling house never set out: {}", the_house(rig));
}

// ---- the stories ---------------------------------------------------------------------------

/// **Staff lay the house up** (`@tacsim/port haulers <port> off`): past its opening day it
/// plans no trip, and `on` sets it trading again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex10_a_laid_up_house_plans_no_trip() {
    let mut rig = opened();
    // On its boot day the house fits its hulls out (hull, helm, grants, purse) but sets
    // nothing out: it opens the day after.
    let mut fitted = false;
    for _ in 0..120 {
        fires(&mut rig, 1);
        let rows = house(&rig).rows;
        if rows.len() == HULLS
            && rows.iter().all(|row| {
                row.hull.is_some()
                    && row.helm.is_some()
                    && row.grant.is_some()
                    && row.account.is_some()
            })
        {
            fitted = true;
            break;
        }
    }
    assert!(fitted, "the house fits its hulls out: {}", the_house(&rig));
    assert!(
        house(&rig).rows.iter().all(|row| row.trip.is_none()),
        "nothing sets out before the house opens: {}",
        the_house(&rig)
    );
    // Laid up with its hulls fitted out, past its opening day, it still sets nothing out.
    let said = run(
        &mut rig.tw,
        mush_core::GOD,
        "@tacsim/port haulers ceres_port off",
    );
    assert!(said.contains("ceres_port hauling house: laid up"), "{said}");
    assert!(house(&rig).laid_up);
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(&mut rig, day * DAY + 600);
    fires(&mut rig, 40);
    let rows = house(&rig).rows;
    assert!(
        rows.iter().all(|row| row.trip.is_none() && row.trips == 0),
        "laid up, nothing set out: {}",
        the_house(&rig)
    );
    let said = run(
        &mut rig.tw,
        mush_core::GOD,
        "@tacsim/port haulers deimos_port off",
    );
    assert!(said.contains("deimos_port has no hauling house."), "{said}");
    let said = run(
        &mut rig.tw,
        mush_core::GOD,
        "@tacsim/port haulers ceres_port on",
    );
    assert!(said.contains("ceres_port hauling house: open"), "{said}");
    let rows = first_trips(&mut rig);
    assert!(
        rows.iter()
            .all(|row| row.trip.as_ref().is_some_and(|trip| trip.n == 1)),
        "opened again, each hull sets out on its first trip: {}",
        the_house(&rig)
    );
}

/// Fires until every hull is fitted out: hull, helm, grants and purse.
fn fitted(rig: &mut Rig) {
    for _ in 0..120 {
        fires(rig, 1);
        let rows = house(rig).rows;
        if rows.len() == HULLS
            && rows.iter().all(|row| {
                row.hull.is_some()
                    && row.helm.is_some()
                    && row.grant.is_some()
                    && row.account.is_some()
            })
        {
            return;
        }
    }
    panic!("the house fits its hulls out: {}", the_house(rig));
}

/// **48e services:G18, G19: the house loses a hull.** One whose title passes to another and one
/// that is gone each leave the house's row: the helm no longer trades her hold, the purse goes
/// home, and the mark is neither re-adopted nor sails again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex10_a_hull_the_house_loses_leaves_its_row_and_sends_the_purse_home() {
    let mut rig = opened();
    fitted(&mut rig);
    let first = row(&rig, "VCC-01");
    let second = row(&rig, "VCC-02");
    let (hull, helm) = (
        ObjId(first.hull.expect("a hull")),
        ObjId(first.helm.expect("a helm")),
    );
    assert_eq!(
        tacsim::port::hauler::hauler_hull(rig.tw.world(), helm),
        Some(hull)
    );
    let (purse, purse_2) = (
        AccountId(first.account.expect("a purse")),
        AccountId(second.account.expect("a purse")),
    );
    // Money in each purse, to see it go home.
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    for account in [purse, purse_2] {
        let out = run(
            &mut rig.tw,
            mush_core::GOD,
            &format!("@econ/mint {treasury}={} 12345", account.0),
        );
        assert!(!out.contains("Usage"), "{out}");
    }
    let house_account = port_map(&rig.tw, CERES).orgs[&cid("vesta_carriers")]
        .account
        .map(AccountId)
        .expect("the house's account");
    let house_before = balance(rig.tw.world(), house_account);

    // G18: VCC-01's title passes to another.
    let moved = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, mush_core::GOD.0),
    );
    assert_ne!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(PrincipalId::Organization(vesta(&rig))),
        "the title moved: {moved}"
    );
    fires(&mut rig, 3);
    let lost = row(&rig, "VCC-01");
    assert!(
        lost.lost && lost.hull.is_none() && lost.trip.is_none(),
        "{lost:?}"
    );
    assert_eq!(
        tacsim::port::hauler::hauler_hull(rig.tw.world(), helm),
        None
    );
    assert_eq!(balance(rig.tw.world(), purse), 0, "the purse went home");

    // G19: VCC-02's hull is gone.
    let gone = ObjId(second.hull.expect("a hull"));
    rig.tw.world_mut().destroy(gone).expect("destroyed");
    fires(&mut rig, 3);
    let lost_2 = row(&rig, "VCC-02");
    assert!(lost_2.lost && lost_2.hull.is_none(), "{lost_2:?}");
    assert_eq!(balance(rig.tw.world(), purse_2), 0, "the purse went home");
    assert!(
        balance(rig.tw.world(), house_account) >= house_before + 2 * 12_345,
        "both purses are the house's again"
    );

    // Past the opening day neither mark is re-adopted, re-seeded or sets out.
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(&mut rig, day * DAY + 600);
    fires(&mut rig, 20);
    for mark in ["VCC-01", "VCC-02"] {
        let row = row(&rig, mark);
        assert!(
            row.lost && row.hull.is_none() && row.trip.is_none() && row.trips == 0,
            "{mark} stays lost: {}",
            the_house(&rig)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ex10_the_house_seeds_its_hulls_and_trades_where_a_gap_pays() {
    let mut rig = opened();
    // On the boot day the house sets nothing out.
    assert!(
        house(&rig).rows.iter().all(|row| row.trip.is_none()),
        "the house opens the day after: {}",
        the_house(&rig)
    );
    let rows = first_trips(&mut rig);
    for row in &rows {
        let hull = ObjId(row.hull.expect("a hull"));
        let owned = tacsim::port::owned::rows(rig.tw.world())
            .find(|owned| owned.hull == hull)
            .cloned()
            .expect("a register row");
        assert_eq!(owned.mark, row.mark, "seeded under its mark");
        let title = rig
            .tw
            .world()
            .module_state::<EconomyStore>()
            .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()));
        assert_eq!(
            title,
            Some(PrincipalId::Organization(vesta(&rig))),
            "{} is titled to the house",
            row.mark
        );
        assert!(
            row.grant.is_some() && row.account.is_some(),
            "a helm's grant and purse"
        );
        let trip = row.trip.clone().expect("a trip in hand");
        let home = if row.mark == "VCC-02" {
            "luna"
        } else {
            "ceres"
        };
        assert_eq!(
            trip.from.as_str(),
            home,
            "{} trades from her home",
            row.mark
        );
        // VCC-03 (lane v54) finds Ceres's own gap taken by VCC-01 and sails on what pays from
        // there; the two original hulls each trade a gap.
        if row.mark != "VCC-03" {
            assert!(trip.bid_major > trip.ask_major, "a gap: {trip:?}");
        }
        assert!(trip.profit_minor > 0, "the gap pays for the trip: {trip:?}");
        // The plan is the helm's, and the executor flies it.
        let plan = match tacsim::port::orders::orders_state(rig.tw.world(), hull) {
            tacsim::port::orders::OrdersState::Present(doc) => doc.clone(),
            _ => panic!("{}: no plan", row.mark),
        };
        assert_eq!(plan.author, ObjId(row.helm.expect("a helm")));
    }

    // Ceres's hull's trip, start to end: Ceres sells water, its one carried good.
    let first = row(&rig, "VCC-01");
    let hull = ObjId(first.hull.expect("a hull"));
    let trip = first.trip.clone().expect("a trip");
    let good = trip.good.clone().expect("a good");
    assert_eq!(good.as_str(), "volatiles");
    let (to_entry, bidder) = match trip.to.as_str() {
        "stickney" => ("stickney_port", "kasei"),
        _ => ("luna_port", "habitat_board"),
    };
    let bidder_vault = VaultId(port_map(&rig.tw, to_entry).vendors[&cid(bidder)].vault);
    let bidder_before = hold_pool(&mut rig.tw, bidder_vault, good.as_str());
    // Laid up mid-trip, the house still flies the plan in hand, closes it and sends the purse's
    // surplus over its float home; it sets out on nothing more.
    let said = run(
        &mut rig.tw,
        mush_core::GOD,
        "@tacsim/port haulers ceres_port off",
    );
    assert!(said.contains("ceres_port hauling house: laid up"), "{said}");
    let purse = AccountId(first.account.expect("a purse"));
    let mut done = false;
    for _ in 0..(30 * 24) {
        ahead(&mut rig, 3_600);
        if row(&rig, "VCC-01").trips >= 1 {
            done = true;
            break;
        }
    }
    assert!(done, "the trip ends: {}", the_house(&rig));
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            Berth::Docked { map, .. } if map.entry.as_str() == to_entry
        ),
        "she is docked where she sold: {}",
        the_house(&rig)
    );
    let delivered = hold_pool(&mut rig.tw, bidder_vault, good.as_str()) - bidder_before;
    assert!(
        delivered > 0,
        "the bidder took the lot: {delivered} of {}",
        trip.units
    );
    let view = tacsim::port::ship::hull_view(rig.tw.world(), hull).expect("a hull view");
    let hold = tacsim::port::ship::hold_vault(rig.tw.world(), &view).expect("a hold");
    assert_eq!(
        hold_pool(&mut rig.tw, hold, good.as_str()),
        0,
        "she sold it all"
    );
    // Lane v43's float is 2,000.00cr: the rest of the sale went home to the house.
    let kept = balance(rig.tw.world(), purse);
    assert!(
        kept <= 2_000 * 100,
        "the purse keeps its float and sends the rest home: {kept}"
    );
    assert!(
        row(&rig, "VCC-01").trip.is_none(),
        "laid up, she sets out on nothing more: {}",
        the_house(&rig)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex10_a_hauler_meets_the_kirkwood_picket() {
    let mut rig = opened();
    // The Habitat Board cannot pay for water: Ceres's water goes to Kasei at Stickney, by the
    // interdicted route.
    drain_org(&mut rig, "luna_port", "habitat_board");
    let rows = first_trips(&mut rig);
    let ceres = rows
        .iter()
        .find(|row| row.mark == "VCC-01")
        .cloned()
        .expect("Ceres's hull");
    let trip = ceres.trip.clone().expect("a trip");
    assert_eq!(trip.to.as_str(), "stickney", "{}", the_house(&rig));
    let hull = ObjId(ceres.hull.expect("a hull"));
    let mut met = None;
    for _ in 0..(30 * 24) {
        ahead(&mut rig, 3_600);
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("the bubble book");
        if let Some(active) = book.active_for_hull(hull) {
            met = Some(active.picket.is_some());
            break;
        }
    }
    let Some(picket) = met else {
        panic!(
            "Ceres's hauler bound for Stickney is promoted at Kirkwood: {}",
            the_house(&rig)
        );
    };
    assert!(picket, "it is the Kirkwood customs picket's bubble");
    let book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("the bubble book");
    assert!(
        book.linkdead_state(hull)
            .is_some_and(tacsim::space::SpaceLinkdeadState::backup_has_helm),
        "her house's mind holds her helm"
    );
    assert_eq!(
        book.active_for_hull(hull).map(|a| a.bubble.ships.len()),
        Some(3),
        "she, the customs corvette and the Belt Raider"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex10_consumption_switches_on_at_a_port_with_no_producer() {
    let mut rig = opened();
    let reserve_vault = VaultId(port_map(&rig.tw, DEIMOS).vendors[&cid("strategic_reserve")].vault);
    let reserve = port_map(&rig.tw, DEIMOS).orgs[&cid("strategic_reserve")]
        .account
        .map(AccountId)
        .expect("the reserve's account");
    ahead(&mut rig, DAY);
    assert!(
        ledger(&mut rig, DEIMOS, "strategic_reserve", "volatiles").is_none(),
        "it holds none, so it uses none"
    );
    seed_pool(&mut rig, reserve_vault, "volatiles", 30, LotId(9_951));
    seed_pool(
        &mut rig,
        reserve_vault,
        "carbon_feedstock",
        10,
        LotId(9_952),
    );
    let before = balance(rig.tw.world(), reserve);
    ahead(&mut rig, DAY);
    ahead(&mut rig, 600);
    let water = ledger(&mut rig, DEIMOS, "strategic_reserve", "volatiles").expect("water used");
    let carbon =
        ledger(&mut rig, DEIMOS, "strategic_reserve", "carbon_feedstock").expect("carbon used");
    assert_eq!((water.units, carbon.units), (15, 5), "a day's use of each");
    assert_eq!(hold_pool(&mut rig.tw, reserve_vault, "volatiles"), 15);
    assert_eq!(
        balance(rig.tw.world(), reserve) - before,
        (15 * 190 + 5 * 380) * 100,
        "the export faucet paid the reserve for what it used"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex10_a_hauler_supplies_deimos_from_luna() {
    let mut rig = opened();
    let rows = first_trips(&mut rig);
    let luna = rows
        .iter()
        .find(|row| row.mark == "VCC-02")
        .cloned()
        .expect("Luna's hull");
    let trip = luna.trip.clone().expect("a trip");
    assert_eq!(trip.from.as_str(), "luna");
    assert_eq!(trip.to.as_str(), "deimos", "{}", the_house(&rig));
    let good = trip.good.clone().expect("a good");
    assert!(
        ["structural_alloys", "instrument_boards"].contains(&good.as_str()),
        "{trip:?}"
    );
    let used = |rig: &mut Rig| {
        ledger(rig, DEIMOS, "syrtis_bonded", good.as_str()).map_or(0, |ledger| ledger.units)
    };
    let before = used(&mut rig);
    let mut supplied = false;
    for _ in 0..(30 * 4) {
        ahead(&mut rig, DAY / 4);
        if used(&mut rig) > before {
            supplied = true;
            break;
        }
    }
    assert!(
        supplied,
        "Luna's hauler brought {good} to Deimos, and Syrtis Bonded used it: {}",
        the_house(&rig)
    );
    ahead(&mut rig, DAY / 4);
    assert!(
        row(&rig, "VCC-02").trips >= 1,
        "the trip closed: {}",
        the_house(&rig)
    );
}
