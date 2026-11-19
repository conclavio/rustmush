//! **Doc 48 unit EX.9 — the balance** (Q-48-19, -21, -22, -34, -36, -38..41, -55, -59, -60), over
//! the composed world (`driving`'s keyed rig), through the port tick and the production command
//! paths.
//!
//! - **Consumption** (`ex9_the_consumers_use_up_what_they_buy_and_the_faucet_pays_them_back`):
//!   Syrtis Bonded uses up 40 t of structural alloys an IC day from what its bid bought, until it
//!   holds none; the Lunar Habitat Board uses up 16 crew-days of its own provisions, drawn off its
//!   ask so the listing and the pool never disagree. The export faucet pays each back the good's
//!   price for what it used, within its daily share. The matter census counts it as `consumed` and
//!   its balance holds.
//! - **Running costs** (`ex9_the_running_cost_follows_the_work_and_an_insolvent_house_mothballs`):
//!   a captain's twenty-day order runs in one of the Stickney works' two slots. The day's charge is
//!   that slot worked and the other idle at a tenth; the idle workshop and yard pay their standing
//!   share; all of it reaches the `tacsim.upkeep` sink. The Yards' account drained, the next day's
//!   charge is a debt: every facility mothballs, the house is throttled, and the mothballed works
//!   still books a customer's order and charges only its worked slot. Money in pays the debt first;
//!   the works reopens once the Yards can pay seven days ahead.
//! - **The mill-run refinery** (`ex9_a_mill_run_refinery_pays_by_the_batch`): Kasei refines two
//!   batches of concentrate and pays one slot-hour of its refinery's daily cost a batch; a day with
//!   no batch costs it nothing.
//! - **The measured run** (`ex9_the_lane_holds_its_money_within_one_percent_a_real_day`): the
//!   shipped lane runs 72 IC days, three real days at the live 24:1, with nobody playing. Over each
//!   24 IC days the lane's money changes by less than 1% of its capital.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the goods in Syrtis's and Kasei's storefronts (stand-ins for
//! what their bids bought), the captain's hull, money and bill (IN.3's), staff moves of the Yards'
//! money. Every charge, act and payment is the port tick's.

use std::collections::BTreeMap;

use mush_core::{GOD, ObjId, World};
use mush_economy::{AccountId, EconomyStore, VaultId};
use mush_industry::{FacilityCostDef, IndustryStore, WorkOrderState};
use mush_inventory::LotId;
use mush_org::{OrgId, PrincipalId, SystemPrincipalId};
use tacsim::port::consume::{ConsumeHandles, ConsumeId, ConsumeLedger};
use tacsim::port::upkeep::{
    EXPORT_FAUCET, UPKEEP_SINK, UpkeepRow, batch_charge, day_charge, standing_day,
};

use crate::driving_rig::{
    Rig, cid, drain, hold_pool, ic_now, land_quietly, port_map, prospector, run, seed_hold,
    seed_pool, station_room, stocked, stocked_tuned, store, sweep, to_ic, typed,
};
use crate::in3_order_contract::{
    booked, mint, order_of, recipes, sail_to_stickney, settle, typed_in,
};

const DAY: i64 = 86_400;
const LUNA: &str = "luna_port";
const DEIMOS: &str = "deimos_port";
const STICKNEY: &str = "stickney_port";
const ALLOYS: &str = "structural_alloys";
const PROVISIONS: &str = "provisions";

// ---- readings ------------------------------------------------------------------------------

fn balance(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .map_or(0, |a| a.posted_balance.minor())
}

/// A System account's balance (0 before it opens).
fn system(w: &World, name: &str) -> i64 {
    let owner = PrincipalId::System(SystemPrincipalId::new(name).expect("a system name"));
    w.module_state::<EconomyStore>()
        .map(|e| {
            e.snapshot()
                .accounts_of(&owner)
                .map(|a| a.posted_balance.minor())
                .sum()
        })
        .unwrap_or(0)
}

fn org_account(rig: &Rig, entry: &str, org: &str) -> AccountId {
    port_map(&rig.tw, entry).orgs[&cid(org)]
        .account
        .map(AccountId)
        .unwrap_or_else(|| panic!("{org}'s account"))
}

fn vault(rig: &Rig, entry: &str, org: &str) -> VaultId {
    VaultId(port_map(&rig.tw, entry).vendors[&cid(org)].vault)
}

fn row(rig: &Rig, entry: &str, room: ObjId) -> UpkeepRow {
    store(&rig.tw)
        .port_upkeep(&cid(entry))
        .and_then(|doc| doc.facilities.get(&room.0).cloned())
        .unwrap_or_else(|| panic!("#{} keeps a running-cost row", room.0))
}

fn cost(rig: &Rig, room: ObjId) -> (FacilityCostDef, u32) {
    let record = rig
        .tw
        .world()
        .module_state::<IndustryStore>()
        .and_then(|store| store.facility(room).cloned())
        .expect("the facility");
    (
        *record.cost().expect("a costed facility"),
        record.maximum_slots(),
    )
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
    typed(&mut rig.tw, &handles.ledgers, &id)
}

/// The census line's terms, by name.
fn census(rig: &mut Rig) -> BTreeMap<String, i128> {
    let out = run(&mut rig.tw, GOD, "@industry/census");
    let line = out
        .lines()
        .find(|l| l.contains(tacsim::port::census::CENSUS_LINE))
        .unwrap_or_else(|| panic!("a census line: {out}"));
    line.split_whitespace()
        .filter_map(|word| word.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.parse().expect("a number")))
        .collect()
}

/// Port fires (31 s apart), every submission they stage landed.
fn fires(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
}

/// The IC instant `secs` into the next IC day, a few fires landed there.
fn into_next_day(rig: &mut Rig, secs: i64) -> i64 {
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(rig, day * DAY + secs);
    fires(rig, 8);
    day
}

/// **Whether the Habitat Board's provisions ask reads ready** on the readiness report (its
/// pool and listing agree, the day's draw counted as drawn).
fn board_ready(rig: &mut Rig) -> bool {
    let status = run(&mut rig.tw, GOD, "@tacsim/port status");
    let lines: Vec<&str> = status
        .lines()
        .filter(|line| line.contains("ask habitat_board provisions"))
        .collect();
    !lines.is_empty() && lines.iter().all(|line| line.ends_with("[ready]"))
}

/// [`into_next_day`], the Board's Listings line read after every fire of the day: the draw for
/// its consumption never reads as drift.
fn checked_next_day(rig: &mut Rig) -> i64 {
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(rig, day * DAY + 600);
    assert!(
        board_ready(rig),
        "IC day {day}'s first fire: the Board's ask reads ready"
    );
    for n in 0..8 {
        fires(rig, 1);
        assert!(
            board_ready(rig),
            "fire {n} of IC day {day}: the Board's ask reads ready"
        );
    }
    day
}

fn kg(rig: &Rig, good: &str, units: u64) -> i128 {
    let per = store(&rig.tw)
        .goods()
        .and_then(|goods| goods.good(&cid(good)).and_then(|g| g.mass_kg_per_unit()))
        .unwrap_or_else(|| panic!("{good} weighs"));
    i128::from(per * units)
}

// ---- consumption ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ex9_the_consumers_use_up_what_they_buy_and_the_faucet_pays_them_back() {
    let mut rig = stocked();
    let syrtis = org_account(&rig, DEIMOS, "syrtis_bonded");
    let board = org_account(&rig, LUNA, "habitat_board");
    let syrtis_vault = vault(&rig, DEIMOS, "syrtis_bonded");
    let board_vault = vault(&rig, LUNA, "habitat_board");

    // Nothing in Syrtis's storefront: nothing is used up, and nothing is paid.
    let _ = into_next_day(&mut rig, 600);
    assert!(ledger(&mut rig, DEIMOS, "syrtis_bonded", ALLOYS).is_none());

    // What its bid bought: 100 t of alloys (seeded, a stand-in for the sales).
    seed_pool(&mut rig, syrtis_vault, ALLOYS, 100, LotId(9_901));
    let census_before = census(&mut rig);
    let syrtis_before = balance(rig.tw.world(), syrtis);
    let board_before = balance(rig.tw.world(), board);
    let faucet_before = system(rig.tw.world(), EXPORT_FAUCET);
    let provisions_before =
        ledger(&mut rig, LUNA, "habitat_board", PROVISIONS).map_or(0, |l| l.units);

    // Three IC days: 40, 40, then the last 20; the faucet pays 40 × 460 cr each full day (its
    // share is exactly that), then 20 × 460.
    let mut held = 100;
    let mut paid = 0;
    for (n, used) in [40_u64, 40, 20].into_iter().enumerate() {
        let day = checked_next_day(&mut rig);
        held -= used;
        assert_eq!(
            hold_pool(&mut rig.tw, syrtis_vault, ALLOYS),
            held,
            "day {n}: Syrtis has used up {used} t"
        );
        let l = ledger(&mut rig, DEIMOS, "syrtis_bonded", ALLOYS).expect("Syrtis's ledger");
        assert_eq!((l.acts, l.day), (n as u64 + 1, day), "one act an IC day");
        paid += i64::try_from(used).expect("fits") * 460 * 100;
        assert_eq!(
            balance(rig.tw.world(), syrtis) - syrtis_before,
            paid,
            "day {n}: the faucet paid Syrtis for what it used"
        );
    }
    // Nothing left: no act, no payment.
    let _ = into_next_day(&mut rig, 600);
    let l = ledger(&mut rig, DEIMOS, "syrtis_bonded", ALLOYS).expect("Syrtis's ledger");
    assert_eq!((l.acts, l.units), (3, 100), "nothing more to use up");
    assert_eq!(balance(rig.tw.world(), syrtis) - syrtis_before, paid);

    // The Habitat Board ate 16 crew-days of its own provisions each of those four days, drawn
    // off its ask: the listing and the pool agree, and the faucet paid it 160 cr a day.
    let board_used =
        ledger(&mut rig, LUNA, "habitat_board", PROVISIONS).expect("the Board's ledger");
    assert_eq!(
        board_used.units - provisions_before,
        4 * 16,
        "16 crew-days an IC day"
    );
    assert_eq!(
        balance(rig.tw.world(), board) - board_before,
        4 * 16 * 10 * 100,
        "the faucet paid the Board for its provisions"
    );
    let pool = hold_pool(&mut rig.tw, board_vault, PROVISIONS);
    let listed = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.lot_listing_of(
                mush_inventory::record::EndpointRef(board_vault),
                &cid(PROVISIONS),
            )
            .and_then(|id| m.listing(id).map(|l| l.quantity))
        })
        .expect("the Board's provisions ask stands");
    assert!(
        listed <= pool && pool <= listed + 16,
        "the ask carries the pool less at most a day's draw: listed {listed}, pool {pool}"
    );
    assert!(
        board_ready(&mut rig),
        "the Board's provisions Listings line reads ready"
    );

    // Nothing paged: the drift check read the drawn units as drawn.
    assert!(
        tacsim::port_tick::drift_pages(rig.tw.world()).is_empty(),
        "no storefront and listing disagreed"
    );

    // The faucet's money is what it paid out.
    assert_eq!(
        faucet_before - system(rig.tw.world(), EXPORT_FAUCET),
        paid + 4 * 16 * 10 * 100,
        "the export faucet paid exactly the consumers' refunds"
    );

    // The census counts what was used up, and its balance holds across it.
    let census_after = census(&mut rig);
    assert_eq!(
        census_after["consumed"] - census_before["consumed"],
        kg(&rig, ALLOYS, 100) + kg(&rig, PROVISIONS, 4 * 16),
        "consumed: Syrtis's alloys and the Board's provisions"
    );
    assert_eq!(
        census_after["balance"], census_before["balance"],
        "the matter census holds across consumption"
    );
}

// ---- running costs -------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ex9_the_running_cost_follows_the_work_and_an_insolvent_house_mothballs() {
    let mut rig = stocked_tuned(|cfg| {
        cfg.packs.push(tacsim::PackSource {
            name: "<test tacsim.test.works>".into(),
            bytes: recipes(),
        });
    });
    let (captain, captain_row) = prospector(&mut rig, "Hauler", 1, "Long Ox");
    let _ = mint(&mut rig, captain);
    seed_hold(&mut rig, &captain_row, ALLOYS, 29, 9_600);
    seed_hold(&mut rig, &captain_row, "industrial_parts", 6, 9_601);
    sail_to_stickney(&mut rig, &[(captain, &captain_row, 9_500)]);
    let counter = station_room(&rig.tw, STICKNEY, "room.p11_service");
    rig.tw
        .world_mut()
        .move_to(captain, counter)
        .expect("to the counter");
    let works = station_room(&rig.tw, STICKNEY, "room.p15_maintenance");
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    let yard = station_room(&rig.tw, STICKNEY, "room.p12_observation");
    let yards = org_account(&rig, STICKNEY, "stickney_yards");
    let yards_org = OrgId(port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")].org);

    // A twenty-day order runs in one of the works' two slots.
    let a = booked(&mut rig, captain, "+works/order hauler_plant");
    for (good, units) in [(ALLOYS, 24), ("industrial_parts", 6)] {
        let out = typed_in(
            &mut rig,
            captain,
            &format!("+works/deliver {a}={good} {units}"),
            1,
        );
        assert!(out.contains("moves into order"), "{out}");
    }
    settle(&mut rig, 2);
    assert_eq!(*order_of(&mut rig, a).state(), WorkOrderState::Running);

    // (1) A whole IC day with it running: fire inside the day, then past its end.
    let day = into_next_day(&mut rig, 600);
    let sink_before = system(rig.tw.world(), UPKEEP_SINK);
    let _ = into_next_day(&mut rig, 600);
    let (works_cost, works_slots) = cost(&rig, works);
    let charged = row(&rig, STICKNEY, works);
    assert_eq!(charged.day, day, "the works charged the day just closed");
    // The tally sums the busy slots fire to fire, so the day's first minutes before its first
    // fire may read idle: within an IC hour of the whole day.
    let whole = day_charge(&works_cost, works_slots, DAY.unsigned_abs(), false);
    let hour_short = day_charge(
        &works_cost,
        works_slots,
        (DAY - 3_600).unsigned_abs(),
        false,
    );
    assert!(
        (hour_short..=whole).contains(&charged.charged_minor),
        "one slot worked the day at the slot-hour, the other idle at a tenth: {} in \
         {hour_short}..={whole}",
        charged.charged_minor
    );
    for room in [workshop, yard] {
        let (c, slots) = cost(&rig, room);
        let open = rig
            .tw
            .world()
            .module_state::<IndustryStore>()
            .map_or(0, |s| s.open_orders_at(room));
        assert_eq!(open, 0, "#{} stood idle", room.0);
        assert_eq!(
            row(&rig, STICKNEY, room).charged_minor,
            standing_day(&c, slots),
            "an idle facility pays its standing share"
        );
    }
    // Every port's charges for that day went to the sink, all of it.
    let charged_total: i64 = store(&rig.tw)
        .port_upkeeps()
        .flat_map(|doc| doc.facilities.values())
        .filter(|r| r.day == day)
        .map(|r| r.charged_minor)
        .sum();
    assert!(charged_total > 0);
    assert_eq!(
        system(rig.tw.world(), UPKEEP_SINK) - sink_before,
        charged_total,
        "the running costs reach the system's sink"
    );

    // (2) The Yards cannot pay: their account drained, the next day's charge is a debt.
    let treasury = port_map(&rig.tw, STICKNEY).treasury.expect("a treasury");
    let all = balance(rig.tw.world(), yards);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={treasury} {all}", yards.0),
    );
    assert_eq!(balance(rig.tw.world(), yards), 0, "{out}");
    let broke = into_next_day(&mut rig, 600);
    let mut debt = 0;
    for room in [works, workshop, yard] {
        let r = row(&rig, STICKNEY, room);
        assert_eq!(r.day, broke - 1, "the day just closed");
        assert!(r.mothballed, "#{} is mothballed", room.0);
        assert_eq!(
            r.debt_minor, r.charged_minor,
            "nothing paid: the charge is owed"
        );
        debt += r.debt_minor;
    }
    assert!(debt > 0);
    // Nothing is minted to pay it: the Yards hold only what came in since (the berth fees they
    // take as Stickney's authority).
    assert!(
        balance(rig.tw.world(), yards) < debt,
        "nothing is minted to pay it"
    );
    assert!(
        tacsim::port::price::throttled(rig.tw.world(), yards_org),
        "a house in debt is throttled"
    );

    // (3) Mothballed: the works charges its worked slot only, the idle facilities nothing; and a
    // customer's order still books, while the house's own is refused.
    let s = booked(&mut rig, captain, "+works/order fittings_batch");
    assert!(s > 0);
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@org/grant #{}=#{} economy.account.post",
            captain.0, yards_org.0
        ),
    );
    assert!(!granted.contains("Permission denied"), "{granted}");
    let own = run(
        &mut rig.tw,
        captain,
        &format!("+works/order fittings_batch=org:{}", yards_org.0),
    );
    assert!(
        own.contains(tacsim::port::works::MOTHBALLED),
        "a mothballed facility's house places no order of its own: {own}"
    );
    let quiet = into_next_day(&mut rig, 600);
    let r = row(&rig, STICKNEY, works);
    assert_eq!(r.day, quiet - 1, "the day just closed");
    assert!(
        r.charged_minor >= day_charge(&works_cost, works_slots, DAY.unsigned_abs(), true)
            && r.charged_minor
                <= day_charge(&works_cost, works_slots, 2 * DAY.unsigned_abs(), true),
        "only worked slots are charged while mothballed: {}",
        r.charged_minor
    );
    for room in [workshop, yard] {
        assert_eq!(
            row(&rig, STICKNEY, room).charged_minor,
            0,
            "no standing share"
        );
    }
    let debt: i64 = [works, workshop, yard]
        .into_iter()
        .map(|room| row(&rig, STICKNEY, room).debt_minor)
        .sum();

    // (4) Money in pays the debt first; under seven days ahead the works stays mothballed.
    let standing: i64 = [works, workshop, yard]
        .into_iter()
        .map(|room| {
            let (c, slots) = cost(&rig, room);
            standing_day(&c, slots)
        })
        .sum();
    let worked_day = day_charge(&works_cost, works_slots, 2 * DAY.unsigned_abs(), true);
    let income = debt + worked_day + standing / 4;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {treasury}={} {income}", yards.0),
    );
    let sink_before = system(rig.tw.world(), UPKEEP_SINK);
    let _ = into_next_day(&mut rig, 600);
    let repaid: i64 = [works, workshop, yard]
        .into_iter()
        .map(|room| row(&rig, STICKNEY, room).debt_minor)
        .sum();
    assert_eq!(repaid, 0, "the debt is paid first");
    assert!(
        system(rig.tw.world(), UPKEEP_SINK) - sink_before >= debt,
        "the debt reached the sink"
    );
    assert!(
        row(&rig, STICKNEY, works).mothballed,
        "not yet seven days ahead"
    );

    // (5) Seven days ahead: it reopens, and the throttle lifts.
    let ahead = 8 * standing + 2 * worked_day;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {treasury}={} {ahead}", yards.0),
    );
    let _ = into_next_day(&mut rig, 600);
    for room in [works, workshop, yard] {
        let r = row(&rig, STICKNEY, room);
        assert!(!r.mothballed, "#{} reopens", room.0);
        assert_eq!(r.debt_minor, 0);
    }
    assert!(!tacsim::port::price::throttled(rig.tw.world(), yards_org));
}

// ---- the mill-run refinery -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex9_a_mill_run_refinery_pays_by_the_batch() {
    let mut rig = stocked();
    let refinery = station_room(&rig.tw, STICKNEY, "room.p07_receiving");
    let (refinery_cost, _) = cost(&rig, refinery);
    let _ = into_next_day(&mut rig, 600);
    // A day with no batch costs the refinery nothing: no standing share.
    let _ = into_next_day(&mut rig, 600);
    let idle = row(&rig, STICKNEY, refinery);
    assert_eq!(
        idle.charged_minor, 0,
        "a mill-run refinery pays no standing share"
    );
    // Kasei's concentrate (a stand-in for what its bid bought): two 20 t batches today.
    let kasei = vault(&rig, STICKNEY, "kasei");
    seed_pool(&mut rig, kasei, "metals", 40, LotId(9_902));
    fires(&mut rig, 30);
    let _ = into_next_day(&mut rig, 600);
    let worked = row(&rig, STICKNEY, refinery);
    let batches = worked.batches - idle.batches;
    assert_eq!(batches, 2, "two batches milled");
    assert_eq!(
        worked.charged_minor,
        batch_charge(&refinery_cost, 2),
        "one slot-hour of the refinery's daily cost a batch"
    );
}

// ---- the measured run ----------------------------------------------------------------------

/// Every lane institution's credit, minor units.
fn lane_money(w: &World, orgs: &[OrgId]) -> i64 {
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return 0;
    };
    let instrument = mush_economy::InstrumentId::new("tacsim.credit").expect("the credit");
    let snapshot = economy.snapshot();
    let mut total = 0;
    for org in orgs {
        total += snapshot
            .accounts_of(&PrincipalId::Organization(*org))
            .filter(|a| a.instrument == instrument)
            .map(|a| a.posted_balance.minor())
            .sum::<i64>();
    }
    total
}

/// **Q-48-36's measure**: 72 IC days (three real days at the live 24:1) of the shipped lane with
/// nobody playing, read every 24 IC days. The lane's money moves by less than 1% of its capital
/// in each, while the running costs and the faucet both run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex9_the_lane_holds_its_money_within_one_percent_a_real_day() {
    let mut rig = stocked();
    let orgs: Vec<OrgId> = store(&rig.tw)
        .port_maps()
        .flat_map(|map| map.orgs.values().map(|row| OrgId(row.org)))
        .collect();
    let lane = crate::driving_rig::lane();
    let capital: i64 = lane
        .entries()
        .values()
        .filter_map(|entry| match entry {
            tacsim::pack::PackEntry::Port(def) => Some(def),
            _ => None,
        })
        .flat_map(|def| def.orgs.iter())
        .map(|org| i64::try_from(org.capital_total().expect("a capital")).expect("fits") * 100)
        .sum();
    let bound = capital / 100;
    let _ = into_next_day(&mut rig, 600);
    let start = ic_now(&rig.tw);
    let mut before = lane_money(rig.tw.world(), &orgs);
    let sink_start = system(rig.tw.world(), UPKEEP_SINK);
    let faucet_start = system(rig.tw.world(), EXPORT_FAUCET);
    for window in 1..=3_i64 {
        // Every six IC hours, a few fires landed.
        let end = start + window * 24 * DAY;
        while ic_now(&rig.tw) < end {
            let next = (ic_now(&rig.tw) + DAY / 4).min(end);
            let _ = to_ic(&mut rig, next);
            fires(&mut rig, 2);
        }
        let after = lane_money(rig.tw.world(), &orgs);
        let change = after - before;
        eprintln!(
            "ex9 measured run: real day {window} at 24:1: lane money {before} -> {after} \
             ({change:+} minor, bound ±{bound}); sink {} faucet {}",
            system(rig.tw.world(), UPKEEP_SINK) - sink_start,
            faucet_start - system(rig.tw.world(), EXPORT_FAUCET),
        );
        assert!(
            change.abs() <= bound,
            "real day {window}: the lane's money moved {change} minor, past 1% of its capital \
             ({bound})"
        );
        before = after;
    }
    assert!(
        system(rig.tw.world(), UPKEEP_SINK) > sink_start,
        "the running costs ran"
    );
    assert!(
        system(rig.tw.world(), EXPORT_FAUCET) < faucet_start,
        "the faucet ran"
    );
}
