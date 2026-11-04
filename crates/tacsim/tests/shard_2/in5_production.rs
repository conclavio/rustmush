//! **Doc 39 cut IN.5 — the first production story, port side**, over the composed world
//! (`driving`'s keyed rig), through the production command paths.
//!
//! The story (doc 39 §6, the IN.5 row; the bot plays the whole of it over SSH at 256 and 512 in
//! `mush-bot/tests/mining_cohort.rs`, `in5_`): a captain at Ceres buys concentrate and a tonne of
//! carbon ore at the Exchange's asks, sails to Stickney Yards and sells the concentrate into
//! Kasei's bid. Kasei refines it on the next fires (five tonnes to four), and its alloy ask
//! restocks by the milled tonnes. The captain buys five tonnes of alloys at that ask, books a
//! fittings batch at the Stickney works through `+works`, delivers the alloys and the carbon from
//! the hold, collects five tonnes of fittings and sails home to sell them to the Co-op at 600. On
//! the way the Stickney works' house purchase buys Kasei's alloys into its stores, a lot an IC
//! day, to its floor, paid from Stickney Yards' account. The drift check never pages and nothing
//! the clerks write is refused.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the captain's money (`@econ/mint`) and the passage's
//! propellant and provisions (`load`). Every claim — the lane's facilities, the sales, the
//! refine and its restock, the order, the delivery, the collection, the Co-op's sale and the
//! house purchase — is reached by the players' commands and the port tick.
//!
//! **No site**: the port half has no 256/512 pair; the ground half is the bot's story.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore, VaultId};
use mush_industry::IndustryStore;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use tacsim::port::HullRow;
use tacsim::port::works::{WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, cid, drain, file_and_launch, fire, hold_pool, hold_vault_of, ic_now, land_all,
    land_quietly, load, no_refused_writes, port_map, prospector, run, station_room, stocked,
    store, sweep, teleport, to_ic,
};

const CERES: &str = "ceres_port";
const STICKNEY: &str = "stickney_port";
const ALLOYS: &str = "structural_alloys";
const CARBON: &str = "carbon_ore";
const FITTINGS: &str = "industrial_parts";
const METALS: &str = "metals";

/// Kasei's refine ledger, as the engine holds it.
fn refine_ledger(rig: &Rig) -> Option<tacsim::port::mill::MillLedger> {
    let handle = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .expect("the port handles")
        .families
        .mills
        .clone();
    let id = tacsim::port::mill::MillId::derive(
        &cid(tacsim::definitions::content::LANE_PACK),
        &cid(STICKNEY),
        &cid("kasei"),
        &cid(METALS),
    )
    .expect("a mill id");
    rig.tw
        .engine()
        .action_state()
        .get(&handle, &id)
        .ok()
        .flatten()
        .cloned()
}

/// A `MakeWriter` that collects formatted log lines (`boot_parity.rs`'s).
#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// A dispatcher writing errors into `capture`.
fn capture_dispatch(capture: Capture) -> tracing::Dispatch {
    tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(capture)
            .finish(),
    )
}

fn market(rig: &Rig) -> &MarketStore {
    rig.tw
        .world()
        .module_state::<MarketStore>()
        .expect("the market")
}

/// Mint the captain ten million minor units from Ceres' treasury.
fn mint(rig: &mut Rig, pilot: ObjId) -> AccountId {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    account
}

/// An account's posted balance, minor units.
fn posted(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).cloned())
        .expect("the account")
        .posted_balance
        .minor()
}

fn storefront(rig: &Rig, entry: &str, org: &str) -> VaultId {
    VaultId(port_map(&rig.tw, entry).vendors[&cid(org)].vault)
}

/// The quantity `vendor`'s pooled ask of `good` lists, if one stands.
fn listed(rig: &Rig, entry: &str, vendor: &str, good: &str) -> Option<u64> {
    let endpoint = EndpointRef(storefront(rig, entry, vendor));
    let m = market(rig);
    m.lot_listing_of(endpoint, &cid(good))
        .and_then(|id| m.listing(id))
        .map(|l| l.quantity)
}

/// One port fire, everything it queued landed.
fn fire_landing(rig: &mut Rig) {
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
}

/// **Buy `qty` of `good` from `vendor` at `entry`** through the market's own quote, accept and
/// draw, into the buyer's docked hold.
fn buy(rig: &mut Rig, pilot: ObjId, entry: &str, vendor: &str, good: &str, qty: u64) {
    let endpoint = EndpointRef(storefront(rig, entry, vendor));
    let listing = market(rig)
        .lot_listing_of(endpoint, &cid(good))
        .unwrap_or_else(|| panic!("{vendor} lists {good}"));
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {listing}={qty}"));
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
        if market(rig).quote(quote).is_some_and(|q| q.drawn_at.is_some()) {
            return;
        }
        fire_landing(rig);
    }
    panic!("the {good} was not drawn into the hold");
}

/// **Sell into `org`'s standing bid of `good` at `entry`** (`+market/sell <bid>=all`), the
/// deliver leg landed and the sale settled; what the pilot heard.
fn sell(rig: &mut Rig, pilot: ObjId, entry: &str, org: &str, good: &str) -> String {
    let bid = port_map(&rig.tw, entry).vendors[&cid(org)].bids[&cid(good)].listing;
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=all"));
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

/// **Every due production occurrence**, landed as the server's schedule driver lands it (the
/// test world drives no schedule source of its own; IN.3's `produce`).
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

/// Every submission staged, the industry's included (the rig's `drain` keeps tacsim's,
/// inventory's and the market's only), landed in order; the refusals.
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
    crate::driving_rig::typed(
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

/// Off the ship (`+ship/leave`).
fn ashore(rig: &mut Rig, who: ObjId) {
    let out = run(&mut rig.tw, who, "+ship/leave");
    assert!(!out.contains("Usage"), "{out}");
}

/// The facility a room of `entry` is, by its local key.
fn facility_at(rig: &Rig, entry: &str, local: &str) -> mush_industry::FacilityRecord {
    let room = station_room(&rig.tw, entry, local);
    rig.tw
        .world()
        .module_state::<IndustryStore>()
        .and_then(|s| s.facility(room))
        .cloned()
        .unwrap_or_else(|| panic!("{entry} {local} is a facility"))
}

/// The works' stores vault at `entry` (the vault on the facility object).
fn stores(rig: &Rig, entry: &str, local: &str) -> Option<VaultId> {
    let room = station_room(&rig.tw, entry, local);
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .vault_by_backing(&mush_economy::CustodyRef::Container(room))
                .map(|v| v.id)
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in5_concentrate_refined_at_kasei_becomes_fittings_sold_at_ceres() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone());
    let _guard = tracing::dispatcher::set_default(&dispatch);
    let mut rig = stocked();

    // ---- The lane's rows, applied ----
    let refinery = facility_at(&rig, STICKNEY, "room.p07_receiving");
    assert!(refinery.offers(&[cid("smelt")].into_iter().collect()));
    for (entry, local) in [(STICKNEY, "room.p15_maintenance"), (CERES, "room.c15_maintenance")] {
        let works = facility_at(&rig, entry, local);
        assert!(works.offers(&[cid("tacsim.works")].into_iter().collect()));
        assert_eq!(works.maximum_slots(), 2);
        assert!(!works.customers().is_owner_only(), "{entry}: the works is public");
    }
    let freeport_works = &port_map(&rig.tw, CERES).vendors[&cid("freeport_works")];
    assert_eq!(
        freeport_works.bids.keys().map(|g| g.as_str()).collect::<Vec<_>>(),
        [CARBON, ALLOYS],
        "the alloy and carbon bids are Freeport Works'"
    );
    assert_eq!(
        Some(freeport_works.vault),
        stores(&rig, CERES, "room.c15_maintenance").map(|v| v.0),
        "Freeport Works' bids fill the works' stores"
    );

    // ---- Ceres: the concentrate and the carbon at the Exchange's asks ----
    let (captain, row) = prospector(&mut rig, "Smith", 1, "Long Ox");
    let account = mint(&mut rig, captain);
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    buy(&mut rig, captain, CERES, "freeport_exchange", METALS, 20);
    buy(&mut rig, captain, CERES, "freeport_exchange", CARBON, 1);
    let hold = hold_vault_of(&rig.tw, &row);
    assert_eq!(hold_pool(&mut rig.tw, hold, METALS), 20);
    assert_eq!(hold_pool(&mut rig.tw, hold, CARBON), 1);

    // ---- Stickney: the concentrate into Kasei's bid, and the refine ----
    board(&mut rig, captain);
    sail(&mut rig, captain, &row, "+ship/undock stickney", STICKNEY, 9_500);
    ashore(&mut rig, captain);
    let kasei_exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, kasei_exchange);
    // The house purchase may hold Kasei's ask down for a fire or two: it comes back up.
    for _ in 0..20 {
        if listed(&rig, STICKNEY, "kasei", ALLOYS).is_some() {
            break;
        }
        fire_landing(&mut rig);
    }
    let alloys_before = listed(&rig, STICKNEY, "kasei", ALLOYS).expect("Kasei's alloy ask");
    let sold = sell(&mut rig, captain, STICKNEY, "kasei", METALS);
    assert!(sold.contains("20"), "{sold}");
    let ledger = refine_ledger;
    for _ in 0..40 {
        if ledger(&rig).is_some_and(|l| l.ore_t == 20)
            && listed(&rig, STICKNEY, "kasei", ALLOYS) == Some(alloys_before + 16)
        {
            break;
        }
        fire_landing(&mut rig);
    }
    let refined = ledger(&rig).expect("Kasei's refine ledger");
    assert_eq!((refined.ore_t, refined.product_t), (20, 16), "five to four");
    assert_eq!(
        listed(&rig, STICKNEY, "kasei", ALLOYS),
        Some(alloys_before + 16),
        "Kasei's ask restocked by the milled tonnes"
    );

    // ---- The works: fittings on the captain's own alloys and carbon ----
    buy(&mut rig, captain, STICKNEY, "kasei", ALLOYS, 5);
    assert_eq!(hold_pool(&mut rig.tw, hold, ALLOYS), 5);
    let quote = run(&mut rig.tw, captain, "+works/quote fittings");
    assert!(
        quote.contains("bill 5 structural_alloys, 1 carbon_ore") && quote.contains("265"),
        "{quote}"
    );
    let before: Vec<u64> = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let booked = works(&mut rig, captain, "+works/order fittings", 1);
    assert!(booked.contains("booked at the works"), "{booked}");
    let id = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("the order");
    for (good, units) in [(ALLOYS, 5), (CARBON, 1)] {
        works(
            &mut rig,
            captain,
            &format!("+works/deliver {id}={good} {units}"),
            1,
        );
    }
    // Six work units: the order runs six IC hours, then the arm delivers.
    let start = ic_now(&rig.tw);
    for hour in 1..=24 {
        if contract(&rig, id).delivered_ic.is_some() {
            break;
        }
        ic_to(&mut rig, start + hour * 3_600);
        let _ = works_fire(&mut rig);
    }
    assert!(
        contract(&rig, id).delivered_ic.is_some(),
        "the fittings are in the escrow: {:?}
{:?}",
        contract(&rig, id),
        order_of(&mut rig, id)
    );
    let collected = works(&mut rig, captain, &format!("+works/collect {id}"), 2);
    assert_eq!(hold_pool(&mut rig.tw, hold, FITTINGS), 5, "{collected}");
    assert_eq!(hold_pool(&mut rig.tw, hold, ALLOYS), 0);
    assert_eq!(hold_pool(&mut rig.tw, hold, CARBON), 0);

    // ---- Home: the fittings to the Co-op at 600 ----
    let gallery = station_room(&rig.tw, STICKNEY, "room.p01_berth_gallery");
    teleport(&mut rig.tw, captain, gallery);
    let out = run(&mut rig.tw, captain, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
    sail(&mut rig, captain, &row, "+ship/undock ceres", CERES, 9_510);
    ashore(&mut rig, captain);
    let chandlery = station_room(&rig.tw, CERES, "room.c08_chandlery");
    teleport(&mut rig.tw, captain, chandlery);
    let before_sale = posted(&rig, account);
    let _ = sell(&mut rig, captain, CERES, "coop", FITTINGS);
    // Five tonnes at 600.00cr, less the Trust's share on a programme hull: 250 per mille of the
    // 80.00cr margin over the fittings' reference (Stickney Yards' 520 ask) a tonne.
    assert_eq!(
        posted(&rig, account) - before_sale,
        300_000 - 10_000,
        "five tonnes of fittings at 600.00cr, the Trust's 100.00cr share out"
    );

    // ---- The house purchase: the Stickney works' stores at their floor ----
    let works_stores = stores(&rig, STICKNEY, "room.p15_maintenance").expect("the works' stores");
    assert_eq!(
        hold_pool(&mut rig.tw, works_stores, ALLOYS),
        10,
        "two lots of five, one an IC day, to the floor"
    );
    // Two purchases moved under the works ledger (keyed by the stores), and no third paid. (The
    // first's payment key has aged out of the economy's RAM window by now: the rig runs at world
    // rate 1, so the two passages are some 23 real days.)
    let ledgers = rig
        .tw
        .world()
        .module_state::<tacsim::port::works::WorksHandles>()
        .expect("the works handles")
        .ledgers
        .clone();
    let moved = crate::driving_rig::typed(&mut rig.tw, &ledgers, &works_stores.0)
        .map(|ledger| ledger.acts);
    assert_eq!(moved, Some(2), "two lots moved into the works' stores");
    let paid = |act: u32| {
        let key = mush_economy::IdempotencyKey::new(
            tacsim::port::supply::SUPPLY_KEY,
            format!("{}/{act}/{ALLOYS}/5", works_stores.0),
        );
        rig.tw
            .world()
            .module_state::<EconomyStore>()
            .is_some_and(|s| s.snapshot().idempotency(&key).is_some())
    };
    assert!(paid(2) && !paid(3), "the second purchase paid, no third");

    // ---- The matter pin and the drift check held ----
    no_refused_writes(&rig.tw);
    let errors = capture.text();
    assert!(
        !errors.contains("a port storefront's stock and its listing disagree"),
        "the drift check paged:\n{errors}"
    );
}
