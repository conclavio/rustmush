//! **Doc 48 unit PL.2 — producers for the seeded goods** (Q-48-23, its corrections; Q-48-41), over
//! the composed world (`driving`'s keyed rig), through the production command paths.
//!
//! The story: a captain at Ceres presses carbon ore into carbon feedstock at the Ceres works'
//! public slot (`+works/order carbon_feedstock`, the recipe pack's v3 row), then sails the Luna leg
//! with water, strategic ore and metals in the hold. At Aitken Highport:
//!
//! 1. The Lunar Habitat Board's farm and Crisium Instruments' fab stand idle at their floors
//!    (1,000 crew-days of provisions, 2,080 t of boards).
//! 2. **The inputs bought**: the captain sells water into the Habitat Board's standing bid and
//!    strategic ore into Crisium's new one. At their floors, the houses hold them unmilled.
//! 3. A crew-day of provisions and a tonne of boards bought take both houses under their floors;
//!    still no batch is queued, each process waiting for its second input. The captain sells the
//!    pressed feedstock into the Habitat Board's other bid and metals into Crisium's.
//! 4. **The processes run on the port tick**: the farm makes 1,000 crew-days of provisions of
//!    5 t of water and 1 t of feedstock; the fab makes 4 t of boards of 2 t of strategic ore and
//!    4 t of metals. Each ledger records its second input. **The output listed**: both asks are
//!    restocked by what was made, their counters in step, and nothing pages drift.
//! 5. **The output bought**: a crew-day and a tonne off the restocked asks.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the captain's money (`@econ/mint`), the carbon ore, water,
//! strategic ore and metals in the hold (miners' and traders' goods; staff's `seed_hold`), and
//! each passage's propellant and provisions (`load`). Every claim — the quote, the order, the
//! delivery, the collection, the sales into the bids, the purchases and the processes — is reached
//! by the player's commands and the port tick.

use mush_core::{GOD, ObjId};
use mush_economy::{ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use tacsim::definitions::content::LANE_PACK;
use tacsim::port::HullRow;
use tacsim::port::mill::{MillId, MillLedger};
use tacsim::port::works::{WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, cid, drain, file_and_launch, fire, hold_pool, hold_vault_of, ic_now, land_all,
    land_quietly, load, no_refused_writes, port_map, prospector, run, seed_hold, station_room,
    stocked, store, sweep, teleport, to_ic, typed,
};

const CERES: &str = "ceres_port";
const LUNA: &str = "luna_port";
const HABITAT: &str = "habitat_board";
const CRISIUM: &str = "crisium_instruments";
const WATER: &str = "volatiles";
const FEEDSTOCK: &str = "carbon_feedstock";
const CARBON_ORE: &str = "carbon_ore";
const STRATEGIC: &str = "strategic_ore";
const METALS: &str = "metals";
const PROVISIONS: &str = "provisions";
const BOARDS: &str = "instrument_boards";

fn market(rig: &Rig) -> &MarketStore {
    rig.tw
        .world()
        .module_state::<MarketStore>()
        .expect("the market")
}

pub(crate) fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

fn storefront(rig: &Rig, entry: &str, org: &str) -> VaultId {
    VaultId(port_map(&rig.tw, entry).vendors[&cid(org)].vault)
}

/// `org`'s storefront units of `good` at `entry`.
fn pooled(rig: &mut Rig, entry: &str, org: &str, good: &str) -> u64 {
    let vault = storefront(rig, entry, org);
    hold_pool(&mut rig.tw, vault, good)
}

/// The quantity `org`'s drawn ask of `good` at `entry` lists, if it stands.
fn listed(rig: &Rig, entry: &str, org: &str, good: &str) -> Option<u64> {
    let endpoint = EndpointRef(storefront(rig, entry, org));
    let market = market(rig);
    market
        .lot_listing_of(endpoint, &cid(good))
        .and_then(|id| market.listing(id))
        .map(|listing| listing.quantity)
}

/// The ledger of `org`'s mill keyed by `key` at `entry` (a process's product, doc 48 unit PL.3).
fn ledger(rig: &mut Rig, entry: &str, org: &str, ore: &str) -> Option<MillLedger> {
    let handle = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .expect("the port handles")
        .families
        .mills
        .clone();
    let id = MillId::derive(&cid(LANE_PACK), &cid(entry), &cid(org), &cid(ore)).expect("a mill id");
    typed(&mut rig.tw, &handle, &id)
}

/// `org`'s `mill_listed` counter for its mill keyed by `key` at `entry`.
fn counter(rig: &Rig, entry: &str, org: &str, ore: &str) -> Option<u64> {
    port_map(&rig.tw, entry).vendors[&cid(org)]
        .mill_listed
        .get(&cid(ore))
        .copied()
}

/// One port fire, everything it queued landed; the mill batches it queued.
fn fire_landing(rig: &mut Rig) -> usize {
    let (_, queued) = fire(rig);
    let batches = queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == tacsim::port::mill::MILL_ACTION)
        .count();
    land_quietly(rig, &queued);
    batches
}

/// **Buy `qty` of `good` from `vendor` at `entry`** through the market's quote, accept and draw
/// (`in7_part_recipes`' shape); the mill batches the fires on the way queued.
fn buy(rig: &mut Rig, pilot: ObjId, entry: &str, vendor: &str, good: &str, qty: u64) -> usize {
    let endpoint = EndpointRef(storefront(rig, entry, vendor));
    let listing = market(rig)
        .lot_listing_of(endpoint, &cid(good))
        .unwrap_or_else(|| panic!("{vendor} lists {good}"));
    let quoted = run(
        &mut rig.tw,
        pilot,
        &format!("+market/quote {listing}={qty}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(rig)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(pilot))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    let draw = drain(&mut rig.tw);
    assert!(!draw.is_empty(), "{accepted}");
    land_all(rig, &draw);
    let mut batches = 0;
    for _ in 0..30 {
        if market(rig)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some())
        {
            return batches;
        }
        batches += fire_landing(rig);
    }
    panic!("the {good} was not drawn into the hold");
}

/// **Sell `qty` into `org`'s standing bid of `good` at `entry`**, the sale settled.
fn sell(rig: &mut Rig, pilot: ObjId, entry: &str, org: &str, good: &str, qty: u64) -> String {
    let bid = port_map(&rig.tw, entry).vendors[&cid(org)].bids[&cid(good)].listing;
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}={qty}"));
    assert!(out.contains("Sold "), "{out}");
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    let contract = market(rig)
        .quotes_of_seller(&mush_org::PrincipalId::Object(pilot))
        .last()
        .and_then(|q| q.contract)
        .expect("a sale contract");
    settle(rig, contract);
    out
}

fn settle(rig: &mut Rig, contract: ContractId) {
    let state = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(contract).map(|c| c.state))
    };
    for _ in 0..12 {
        if matches!(state(rig), Some(ContractState::Settled { .. })) {
            return;
        }
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
    panic!("contract #{contract} did not settle: {:?}", state(rig));
}

/// **Every due production occurrence**, landed as the server's schedule driver lands it
/// (IN.3's and IN.5's `produce`).
pub(crate) fn produce(rig: &mut Rig) {
    for _ in 0..4_096 {
        let page = rig
            .tw
            .engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 64)
            .expect("the due schedules");
        let Some(candidate) = page
            .candidates()
            .iter()
            .find(|c| c.action().key().owner.as_str() == "industry")
            .cloned()
        else {
            return;
        };
        let ingress =
            mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)
                .expect("the schedule's ingress");
        let plan = rig
            .tw
            .engine()
            .plan_scheduled_action(&candidate, &ingress)
            .expect("the schedule's plan");
        let (_, invocation, _) = plan.into_parts();
        let token = crate::driving_rig::next_token();
        let (prepared, _) = rig
            .tw
            .engine_mut()
            .prepare_registered_scheduled_authoritative_action(
                &candidate,
                &invocation,
                mush_core::CommitToken::from_bytes(token),
            )
            .expect("the occurrence prepares");
        let (ops, install) = prepared.into_parts();
        rig.tw
            .engine_mut()
            .install_prepared_action(install)
            .expect("the occurrence installs");
        crate::driving_rig::keep(rig, ops);
    }
}

/// Every submission staged (industry's included), landed in order; the refusals.
pub(crate) fn pump(rig: &mut Rig) -> Vec<String> {
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    queued
        .iter()
        .filter_map(|submit| {
            crate::driving_rig::land(rig, submit)
                .err()
                .map(|e| format!("{}: {e}", submit.action.key().id))
        })
        .collect()
}

/// One port fire (6 s on), production landed, everything staged landed.
pub(crate) fn works_fire(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    pump(rig)
}

/// The IC clock to `target`, everything staged on the way landed (production included).
pub(crate) fn ic_to(rig: &mut Rig, target: i64) {
    let rate = i64::from(crate::driving_rig::world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let _ = pump(rig);
    produce(rig);
}

/// A `+works` command, its submissions landed and `fires` works fires after it.
pub(crate) fn works(rig: &mut Rig, who: ObjId, line: &str, fires: usize) -> String {
    let out = run(&mut rig.tw, who, line);
    let refused = pump(rig);
    assert!(refused.is_empty(), "{line}: {refused:?}\n{out}");
    for _ in 0..fires {
        let _ = works_fire(rig);
    }
    out
}

pub(crate) fn order_of(rig: &mut Rig, id: u64) -> Option<mush_industry::WorkOrder> {
    let order = contract(rig, id).order?;
    let families = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    typed(
        &mut rig.tw,
        &families.work_orders,
        &mush_industry::WorkOrderId(order),
    )
}

pub(crate) fn contract(rig: &Rig, id: u64) -> WorksContract {
    match store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

/// Sail `row` from where it lies to `entry` (its propellant and provisions loaded), docked.
fn sail(rig: &mut Rig, pilot: ObjId, row: &HullRow, line: &str, entry: &str, seed: u64) {
    load(rig, row, 60, 40, seed);
    let t = file_and_launch(rig, pilot, ObjId(row.hull), line);
    let arrive = t.plan.depart_ic + t.plan.total_s;
    let _ = to_ic(rig, arrive);
    let _ = to_ic(rig, arrive + 3_600);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry
        ),
        "docked at {entry}"
    );
}

/// Off the ship.
fn ashore(rig: &mut Rig, who: ObjId) {
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(!out.contains("Usage"), "{out}");
}

/// **The carbon press at the Ceres works** (the recipe pack's v3 row, a public slot): quote,
/// order, deliver the five tonnes of carbon ore, let the arm run and deliver, collect the four
/// tonnes of feedstock into the docked hold.
fn press_carbon(rig: &mut Rig, captain: ObjId, hold: VaultId) {
    let quote = run(&mut rig.tw, captain, "+works/quote carbon_feedstock");
    assert!(
        quote.contains("bill 5 carbon_ore") && quote.contains("(4 work units)"),
        "{quote}"
    );
    let before: Vec<u64> = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let booked = works(rig, captain, "+works/order carbon_feedstock", 1);
    assert!(booked.contains("booked at the works"), "{booked}");
    let id = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("the order");
    works(
        rig,
        captain,
        &format!("+works/deliver {id}={CARBON_ORE} 5"),
        1,
    );
    let start = ic_now(&rig.tw);
    for hour in 1..=24 {
        let done = order_of(rig, id).is_some_and(|o| {
            matches!(
                o.state(),
                mush_industry::WorkOrderState::AwaitingDelivery
                    | mush_industry::WorkOrderState::Completed { .. }
            )
        });
        if done {
            break;
        }
        ic_to(rig, start + hour * 3_600);
    }
    for _ in 0..6 {
        if contract(rig, id).delivered_ic.is_some() {
            break;
        }
        let _ = works_fire(rig);
    }
    assert!(
        contract(rig, id).delivered_ic.is_some(),
        "the press delivered"
    );
    // A pooled output is dissolved in the escrow before it can be collected (IN.5's rule).
    let mut collected = String::new();
    for _ in 0..12 {
        collected = works(rig, captain, &format!("+works/collect {id}"), 2);
        if collected.contains(tacsim::port::works::INTO_HOLD) {
            break;
        }
    }
    assert!(
        collected.contains(tacsim::port::works::INTO_HOLD),
        "{collected}"
    );
    assert_eq!(hold_pool(&mut rig.tw, hold, CARBON_ORE), 0);
    assert_eq!(
        hold_pool(&mut rig.tw, hold, FEEDSTOCK),
        4,
        "pressed five to four"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl2_carbon_pressed_at_ceres_feeds_the_luna_farm_and_the_fab_one_full_cycle() {
    let mut rig = stocked();
    assert_eq!(listed(&rig, LUNA, HABITAT, PROVISIONS), Some(1_000));
    assert_eq!(listed(&rig, LUNA, CRISIUM, BOARDS), Some(2_080));

    // ---- Ceres: the carbon press, a public slot at the works ----
    let (captain, row) = prospector(&mut rig, "Mendez", 1, "Kettle Ox");
    mint(&mut rig, captain);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, CARBON_ORE, 5, 7_801);
    press_carbon(&mut rig, captain, hold);

    // ---- The Luna leg, with the farm's and the fab's other inputs ----
    seed_hold(&mut rig, &row, WATER, 5, 7_802);
    seed_hold(&mut rig, &row, STRATEGIC, 2, 7_803);
    seed_hold(&mut rig, &row, METALS, 4, 7_804);
    board(&mut rig, captain);
    sail(&mut rig, captain, &row, "+ship/undock luna", LUNA, 9_700);
    ashore(&mut rig, captain);
    let exchange = station_room(&rig.tw, LUNA, "room.l07_exchange");
    teleport(&mut rig.tw, captain, exchange);

    // ---- (1)-(2) The ores bought, held unmilled at the floors ----
    sell(&mut rig, captain, LUNA, HABITAT, WATER, 5);
    sell(&mut rig, captain, LUNA, CRISIUM, STRATEGIC, 2);
    let mut batches = 0;
    for _ in 0..6 {
        batches += fire_landing(&mut rig);
    }
    assert_eq!(batches, 0, "the farm and the fab at their floors");

    // ---- (3) Under the floors, each waits for its second input to be bought ----
    batches += buy(&mut rig, captain, LUNA, HABITAT, PROVISIONS, 1);
    batches += buy(&mut rig, captain, LUNA, CRISIUM, BOARDS, 1);
    for _ in 0..6 {
        batches += fire_landing(&mut rig);
    }
    assert_eq!(batches, 0, "no batch without the second input");
    assert!(ledger(&mut rig, LUNA, HABITAT, PROVISIONS).is_none());
    assert!(ledger(&mut rig, LUNA, CRISIUM, BOARDS).is_none());
    sell(&mut rig, captain, LUNA, HABITAT, FEEDSTOCK, 1);
    sell(&mut rig, captain, LUNA, CRISIUM, METALS, 4);

    // ---- (4) The processes run on the tick; the output collected and listed ----
    // Doc 48 cut EX.0: the matter census holds across them, each process's ore and second input
    // in, its product out, the difference the declared loss (`milled`).
    let balance = crate::in11_matter::census(&mut rig)["balance"];
    for _ in 0..48 {
        if counter(&rig, LUNA, HABITAT, PROVISIONS) == Some(1_000)
            && counter(&rig, LUNA, CRISIUM, BOARDS) == Some(4)
        {
            break;
        }
        fire_landing(&mut rig);
    }
    assert_eq!(
        crate::in11_matter::census(&mut rig)["balance"],
        balance,
        "the processes conserve matter"
    );
    let farm = ledger(&mut rig, LUNA, HABITAT, PROVISIONS).expect("the farm ran");
    assert_eq!(
        (
            farm.batches,
            farm.ore_t,
            farm.product_t,
            farm.with_t.clone()
        ),
        (1, 5, 1_000, vec![1]),
        "5 t of water and 1 t of feedstock into 1,000 crew-days"
    );
    assert_eq!(farm.into, cid(PROVISIONS));
    let fab = ledger(&mut rig, LUNA, CRISIUM, BOARDS).expect("the fab ran");
    assert_eq!(
        (fab.batches, fab.ore_t, fab.product_t, fab.with_t.clone()),
        (1, 2, 4, vec![4]),
        "2 t of strategic ore and 4 t of metals into 4 t of boards"
    );
    assert_eq!(
        (
            pooled(&mut rig, LUNA, HABITAT, WATER),
            pooled(&mut rig, LUNA, HABITAT, FEEDSTOCK),
            pooled(&mut rig, LUNA, HABITAT, PROVISIONS),
        ),
        (0, 0, 1_999)
    );
    assert_eq!(
        (
            pooled(&mut rig, LUNA, CRISIUM, STRATEGIC),
            pooled(&mut rig, LUNA, CRISIUM, METALS),
            pooled(&mut rig, LUNA, CRISIUM, BOARDS),
        ),
        (0, 0, 2_083)
    );
    assert_eq!(listed(&rig, LUNA, HABITAT, PROVISIONS), Some(1_999));
    assert_eq!(listed(&rig, LUNA, CRISIUM, BOARDS), Some(2_083));
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());

    // ---- (5) The output bought ----
    buy(&mut rig, captain, LUNA, HABITAT, PROVISIONS, 1);
    buy(&mut rig, captain, LUNA, CRISIUM, BOARDS, 1);
    assert_eq!(listed(&rig, LUNA, HABITAT, PROVISIONS), Some(1_998));
    assert_eq!(listed(&rig, LUNA, CRISIUM, BOARDS), Some(2_082));
    assert_eq!(
        hold_pool(&mut rig.tw, hold, BOARDS),
        2,
        "both tonnes aboard"
    );
    for _ in 0..6 {
        fire_landing(&mut rig);
    }
    let farm = ledger(&mut rig, LUNA, HABITAT, PROVISIONS).expect("the farm's ledger");
    let fab = ledger(&mut rig, LUNA, CRISIUM, BOARDS).expect("the fab's ledger");
    assert_eq!(
        (farm.batches, fab.batches),
        (1, 1),
        "nothing left to process"
    );
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    no_refused_writes(&rig.tw);
}
