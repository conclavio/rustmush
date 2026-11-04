//! **Doc 39 cut IN.7 — the part recipes**, over the composed world (`driving`'s keyed rig),
//! through the production command paths.
//!
//! The story (doc 39 §6, the IN.7 row): a captain at Ceres with strategic ore in the hold and a
//! titled scout in their Dray-P's bay sails the Luna leg — Ceres to Aitken Highport, where
//! Crisium Instruments' ask sells a tonne of instrument boards, then by Voltaire Anchorage to
//! Stickney Yards. At Stickney:
//!
//! 1. The scout's sight is wrecked and the workshop's stores have none: the seeds retired, so
//!    `+bay` quotes `NO_PART`.
//! 2. A tonne of strategic ore goes into Stickney Yards' new bid at 700; three tonnes of
//!    fittings come off the Yards' ask.
//! 3. `+works/quote sight_visual` prints the bill (2 strategic ore, 1 boards, 3 fittings) and
//!    sixty work units; `+works/order`, three `+works/deliver`s, and the arm runs it.
//! 4. Delivered, the escrow holds one ten-unit industry lot with no provenance row: `+bay/fit`
//!    refuses it and `+works/collect` is still settling.
//! 5. The arm owes the part commission under the works' bay ledger; the world restarts before
//!    it lands, and after the restart it lands once: ten one-unit lots at grade 9, each with a
//!    `Manufactured` row naming the order and the works, the industry lot gone.
//! 6. `+works/collect` puts the ten in the hold; `+bay` now quotes nothing from the stores, and
//!    `+bay/fit` fits one to the scout. Its row reads `Manufactured`, fitted into the scout.
//! 7. A disputed part — the scout's sight stripped again and listed under an open claim — is
//!    refused at the fit's custody.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the captain's money (`@econ/mint`), the strategic ore in the
//! hold (a miner's good the bot story mines; staff's `seed_hold`), each passage's propellant and
//! provisions (`load`), the scout's unit and title (`@unit/create`, `@asset/title`) and its
//! placement in the bay, the wrecked sight (staff damage through the unit's plan path, GC.8's
//! fixture) and the open claim (staff's `list_under_claim`, the seam that pairs with
//! `vacate_claim`: no ruled path strips under a claim yet). Every claim — the bid, the sales,
//! the boards' purchase, the quote, the order, the deliveries, the commission and its replay, the
//! collection, both refusals and the fit — is reached by the player's commands and the port
//! tick.
//!
//! **No site**: the cut is port-side, so it has no 256/512 pair (IN.2's, IN.4's and IN.6's
//! precedent).

use mush_core::{GOD, ObjId};
use mush_economy::{ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::BayHullState;
use tacsim::port::bay_work::act::{BayActHandles, PartOrigin, PartProvenance};
use tacsim::port::works::{WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, cid, drain, file_and_launch, fire, hold_pool, hold_vault_of, ic_now, land_all,
    land_quietly, load, no_refused_writes, port_map, prospector, reboot, run, seed_hold,
    staff_hand, station_room, stocked, store, sweep, teleport, to_ic, typed,
};

const CERES: &str = "ceres_port";
const LUNA: &str = "luna_port";
const STICKNEY: &str = "stickney_port";
const STRATEGIC: &str = "strategic_ore";
const BOARDS: &str = "instrument_boards";
const FITTINGS: &str = "industrial_parts";
const SIGHT: &str = "tacsim.part.sensor.visual_mk1";

fn market(rig: &Rig) -> &MarketStore {
    rig.tw
        .world()
        .module_state::<MarketStore>()
        .expect("the market")
}

fn mint(rig: &mut Rig, pilot: ObjId) {
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

/// One port fire, everything it queued landed.
fn fire_landing(rig: &mut Rig) {
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
}

/// **Buy `qty` of `good` from `vendor` at `entry`** through the market's quote, accept and draw.
fn buy(rig: &mut Rig, pilot: ObjId, entry: &str, vendor: &str, good: &str, qty: u64) {
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
    for _ in 0..30 {
        if market(rig)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some())
        {
            return;
        }
        fire_landing(rig);
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
fn produce(rig: &mut Rig) {
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
fn pump(rig: &mut Rig) -> Vec<String> {
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
fn works_fire(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    pump(rig)
}

/// The IC clock to `target`, everything staged on the way landed (production included).
fn ic_to(rig: &mut Rig, target: i64) {
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
fn works(rig: &mut Rig, who: ObjId, line: &str, fires: usize) -> String {
    let out = run(&mut rig.tw, who, line);
    let refused = pump(rig);
    assert!(refused.is_empty(), "{line}: {refused:?}\n{out}");
    for _ in 0..fires {
        let _ = works_fire(rig);
    }
    out
}

fn order_of(rig: &mut Rig, id: u64) -> Option<mush_industry::WorkOrder> {
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

fn contract(rig: &Rig, id: u64) -> WorksContract {
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

/// Off the ship, then aboard again for the next passage.
fn ashore(rig: &mut Rig, who: ObjId) {
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(!out.contains("Usage"), "{out}");
}

fn reboard(rig: &mut Rig, who: ObjId, entry: &str, gallery: &str) {
    let room = station_room(&rig.tw, entry, gallery);
    teleport(&mut rig.tw, who, room);
    let out = run(&mut rig.tw, who, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
}

/// Staff put `who` in `room`: into a hull's interior space, or into a plain port room.
fn put(rig: &mut Rig, who: ObjId, room: ObjId, interior: bool) {
    staff_hand(rig, |ctx| {
        if interior {
            mush_sim::place_interior(ctx, who, room, None).expect("into the space");
        } else {
            let _ = mush_sim::unplace(ctx, who, "staff");
            ctx.w.move_to(who, room).expect("into the room");
        }
    });
}

/// A thing made by staff, its id.
fn made(rig: &mut Rig, name: &str) -> ObjId {
    let out = run(&mut rig.tw, GOD, &format!("@create {name}"));
    out.split('#')
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(ObjId)
        .unwrap_or_else(|| panic!("a thing: {out}"))
}

fn unit(rig: &Rig, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("a unit")
}

fn component_id(u: &mush_tactics::TacticalUnit, local: &str) -> mush_tactics::ComponentId {
    u.components()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str().ends_with(&format!(".{local}")) || id.as_str() == local)
        .unwrap_or_else(|| panic!("a {local} part"))
}

/// Staff wreck `hull`'s visual sight through the unit's one plan path.
fn wreck_sight(rig: &mut Rig, hull: ObjId) {
    let u = unit(rig, hull);
    let sight = component_id(&u, "visual_sensor");
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentCondition {
            component: sight,
            condition: ComponentCondition::Destroyed
                .to_condition_id()
                .expect("destroyed"),
        }],
        Some(u.revision()),
        None,
    )
    .expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, hull, &plan).expect("the damage lands");
    });
}

fn handles(rig: &Rig) -> BayActHandles {
    rig.tw
        .world()
        .module_state::<BayActHandles>()
        .cloned()
        .expect("the bay's handles")
}

fn lot_record(rig: &mut Rig, lot: LotId) -> Option<mush_inventory::Lot> {
    let inventory = handles(rig).inventory.expect("the inventory");
    typed(&mut rig.tw, &inventory.lots, &lot)
}

fn provenance(rig: &mut Rig, lot: LotId) -> Option<PartProvenance> {
    let parts = handles(rig).parts;
    typed(&mut rig.tw, &parts, &lot)
}

/// The lots at `vault` of `good`, by a walk of the lot family.
fn lots_at(rig: &mut Rig, vault: VaultId, good: &str) -> Vec<mush_inventory::Lot> {
    let inventory = handles(rig).inventory.expect("the inventory");
    let state = rig.tw.engine().action_state();
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .scan_family(&inventory.lots, cursor.as_deref(), 256)
            .expect("the lots read");
        out.extend(
            page.entries()
                .iter()
                .map(|e| e.value().clone())
                .filter(|l| l.endpoint() == EndpointRef(vault) && l.commodity().as_str() == good),
        );
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    out
}

/// The landed acts on `ledger`'s bay ledger.
fn bay_acts(rig: &mut Rig, ledger: ObjId) -> u32 {
    let ledgers = handles(rig).ledgers;
    typed(&mut rig.tw, &ledgers, &ledger).map_or(0, |l| l.acts)
}

fn bay_owes(rig: &Rig, ledger: ObjId) -> bool {
    matches!(
        store(&rig.tw).bay_hull(ledger),
        Some(BayHullState::Present(r)) if r.owed.is_some()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in7_strategic_ore_boards_and_fittings_become_ten_sights_at_stickney() {
    let mut rig = stocked();

    // ---- The lane: the seeds retired, the Stickney strategic-ore bid ----
    for (entry, org) in [(CERES, "occator"), (STICKNEY, "stickney_yards")] {
        assert!(
            !port_map(&rig.tw, entry).vendors[&cid(org)]
                .stock
                .iter()
                .any(|s| tacsim::port_tick::is_part(&s.good)),
            "{org} seeds no parts"
        );
    }
    assert!(
        port_map(&rig.tw, STICKNEY).vendors[&cid("stickney_yards")]
            .bids
            .contains_key(&cid(STRATEGIC)),
        "Stickney Yards bids strategic ore"
    );

    // ---- Ceres: the captain, the ore and the scout ----
    let (captain, row) = prospector(&mut rig, "Smith", 1, "Long Ox");
    mint(&mut rig, captain);
    seed_hold(&mut rig, &row, STRATEGIC, 3, 7_701);
    let hold = hold_vault_of(&rig.tw, &row);
    let scout = made(&mut rig, "Quick Wren");
    let created = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.scout_mk1", scout.0),
    );
    assert!(unit_exists(&rig, scout), "{created}");
    let titled = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=#{}", scout.0, captain.0),
    );
    assert!(!titled.contains("refused"), "{titled}");
    put(&mut rig, scout, ObjId(row.rooms[&cid("bay")]), true);

    // ---- The Luna leg: boards at Crisium Instruments' ask, home by Voltaire ----
    board(&mut rig, captain);
    sail(&mut rig, captain, &row, "+ship/undock luna", LUNA, 9_700);
    ashore(&mut rig, captain);
    let luna_exchange = station_room(&rig.tw, LUNA, "room.l07_exchange");
    teleport(&mut rig.tw, captain, luna_exchange);
    buy(&mut rig, captain, LUNA, "crisium_instruments", BOARDS, 1);
    assert_eq!(hold_pool(&mut rig.tw, hold, BOARDS), 1);
    reboard(&mut rig, captain, LUNA, "room.l01_berth_gallery");
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock deimos",
        "deimos_port",
        9_710,
    );
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock stickney",
        STICKNEY,
        9_720,
    );
    ashore(&mut rig, captain);

    // ---- (1) The scout's sight is wrecked, and the stores have none ----
    put(&mut rig, scout, ObjId(row.rooms[&cid("bay")]), true);
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    put(&mut rig, captain, workshop, false);
    let berthed = run(&mut rig.tw, captain, &format!("+bay/berth #{}", scout.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    wreck_sight(&mut rig, scout);
    let quoted = run(&mut rig.tw, captain, &format!("+bay #{}", scout.0));
    assert!(
        quoted.contains("The workshop has no visual sight, mark 1 in its stores"),
        "the seeds retired: {quoted}"
    );

    // ---- (2) The strategic-ore bid, and the fittings ----
    let exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, exchange);
    let sold = sell(&mut rig, captain, STICKNEY, "stickney_yards", STRATEGIC, 1);
    assert!(
        sold.contains("700"),
        "Stickney Yards pays 700 a tonne: {sold}"
    );
    assert_eq!(hold_pool(&mut rig.tw, hold, STRATEGIC), 2);
    buy(&mut rig, captain, STICKNEY, "stickney_yards", FITTINGS, 3);
    assert_eq!(hold_pool(&mut rig.tw, hold, FITTINGS), 3);

    // ---- (3) The order ----
    put(&mut rig, captain, workshop, false);
    let quote = run(&mut rig.tw, captain, "+works/quote sight_visual");
    assert!(
        quote.contains("bill 2 strategic_ore, 1 instrument_boards, 3 industrial_parts")
            && quote.contains("(60 work units)"),
        "{quote}"
    );
    let too_many = run(&mut rig.tw, captain, "+works/quote sight_visual 4");
    assert!(too_many.contains("at most 32 parts"), "{too_many}");
    let before: Vec<u64> = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let booked = works(&mut rig, captain, "+works/order sight_visual", 1);
    assert!(booked.contains("booked at the works"), "{booked}");
    let id = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("the order");
    for (good, units) in [(STRATEGIC, 2), (BOARDS, 1), (FITTINGS, 3)] {
        works(
            &mut rig,
            captain,
            &format!("+works/deliver {id}={good} {units}"),
            1,
        );
    }
    // Sixty work units: the order runs sixty IC hours; the arm then delivers, one fire at a time
    // so the story stands between the delivery and the commission.
    // The clock runs by the hour (each step catches the port up several fires) until the work is
    // done, then one fire at a time.
    let start = ic_now(&rig.tw);
    for hour in 1..=96 {
        let done = order_of(&mut rig, id).is_some_and(|o| {
            matches!(
                o.state(),
                mush_industry::WorkOrderState::AwaitingDelivery
                    | mush_industry::WorkOrderState::Completed { .. }
            )
        });
        if done {
            break;
        }
        ic_to(&mut rig, start + hour * 3_600);
    }
    for _ in 0..6 {
        if contract(&rig, id).delivered_ic.is_some() {
            break;
        }
        let _ = works_fire(&mut rig);
    }
    let delivered = contract(&rig, id);
    assert!(delivered.delivered_ic.is_some(), "{delivered:?}");
    let facility = delivered.facility;
    let order = delivered.order.expect("the work order");
    let escrow = VaultId(id);

    // ---- (4) Delivered, untraced: the bay refuses it, the collection waits ----
    let industry = lots_at(&mut rig, escrow, SIGHT);
    assert_eq!(industry.len(), 1, "one industry lot: {industry:?}");
    let industry = industry[0].clone();
    assert_eq!(industry.quantity(), 10, "ten sights a batch");
    assert!(provenance(&mut rig, industry.id()).is_none(), "untraced");
    let refused = run(
        &mut rig.tw,
        captain,
        &format!("+bay/fit #{}=visual_sensor {}", scout.0, industry.id()),
    );
    assert!(
        refused.contains("carries no part provenance"),
        "the ten-unit lot is unfittable: {refused}"
    );
    let early = run(&mut rig.tw, captain, &format!("+works/collect {id}"));
    assert!(
        early.contains(tacsim::port::works::SETTLING),
        "the commission first: {early}"
    );
    assert!(bay_owes(&rig, facility), "the arm owes the commission");
    assert_eq!(bay_acts(&mut rig, facility), 0);

    // ---- (5) The commission is submitted, the world restarts before it lands ----
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut lost = 0;
    while rig.tw.engine_mut().take_authoritative_submit().is_some() {
        lost += 1;
    }
    assert!(lost > 0, "the commission was submitted");
    assert_eq!(bay_acts(&mut rig, facility), 0, "and never landed");
    let mut rig = reboot(rig, &[captain]);
    for _ in 0..6 {
        if !bay_owes(&rig, facility) {
            break;
        }
        let _ = works_fire(&mut rig);
    }
    assert!(!bay_owes(&rig, facility), "the commission settled");
    for _ in 0..4 {
        let _ = works_fire(&mut rig);
    }
    assert_eq!(bay_acts(&mut rig, facility), 1, "it landed once");
    assert!(lot_record(&mut rig, industry.id()).is_none(), "consumed");
    let sights = lots_at(&mut rig, escrow, SIGHT);
    assert_eq!(sights.len(), 10, "ten one-unit lots");
    for lot in &sights {
        assert_eq!((lot.quantity(), lot.grade().get()), (1, 9));
        let row = provenance(&mut rig, lot.id()).expect("a row");
        assert_eq!(
            row.origin,
            PartOrigin::Manufactured { order, facility },
            "made by the order at the works"
        );
        assert!(row.fitted_into.is_none());
    }

    // ---- (6) Collected, and one fitted to the scout ----
    let collected = works(&mut rig, captain, &format!("+works/collect {id}"), 2);
    assert!(
        collected.contains(tacsim::port::works::INTO_HOLD),
        "{collected}"
    );
    let mine = lots_at(&mut rig, hold, SIGHT);
    assert_eq!(mine.len(), 10, "the ten sights in the hold: {collected}");
    let lot = mine[0].id();
    let taken = run(
        &mut rig.tw,
        captain,
        &format!("+bay/fit #{}=visual_sensor {lot}", scout.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let due = match store(&rig.tw).bay_hull(scout) {
        Some(BayHullState::Present(r)) => r.order.as_ref().expect("an order").due_ic,
        other => panic!("{other:?}"),
    };
    for at in [60, 600, 1_200] {
        let _ = to_ic(&mut rig, due + at);
    }
    let fitted = provenance(&mut rig, lot).expect("the row");
    assert_eq!(fitted.fitted_into, Some(scout), "fitted into the scout");
    assert_eq!(fitted.origin_word(), "manufactured");
    assert!(lot_record(&mut rig, lot).is_none(), "the lot is consumed");
    let sight = component_id(&unit(&rig, scout), "visual_sensor");
    let assessment =
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, scout), None).expect("assessed");
    assert_eq!(
        assessment.component_conditions[&sight],
        ComponentCondition::Intact
    );

    // ---- (7) A disputed part is refused at the custody ----
    let stripped = run(
        &mut rig.tw,
        captain,
        &format!("+bay/strip #{}=visual_sensor", scout.0),
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..6 {
        if !bay_owes(&rig, scout) {
            break;
        }
        fire_landing(&mut rig);
    }
    assert!(
        stripped.contains("goes to the carrier's hold"),
        "{stripped}"
    );
    let acts = bay_acts(&mut rig, scout);
    let off = tacsim::port::bay_work::act::lot_id(scout, acts, 0).expect("the lot");
    assert_eq!(
        provenance(&mut rig, off).map(|p| p.origin_word()),
        Some("stripped")
    );
    staff_hand(&mut rig, |ctx| {
        tacsim::port::bay_work::strip::list_under_claim(ctx, scout, off, 7_007)
            .expect("listed under the claim");
    });
    let refused = run(
        &mut rig.tw,
        captain,
        &format!("+bay/fit #{}=visual_sensor {off}", scout.0),
    );
    assert!(refused.contains("That part is disputed"), "{refused}");
    assert!(drain(&mut rig.tw).is_empty(), "the refusal stages nothing");

    no_refused_writes(&rig.tw);
}

fn unit_exists(rig: &Rig, hull: ObjId) -> bool {
    mush_tactics::module::unit_of(rig.tw.world(), hull).is_some()
}
