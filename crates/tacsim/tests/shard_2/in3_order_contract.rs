//! **Doc 39 cut IN.3 — the order contract**, over the composed world (`driving`'s keyed rig),
//! through the production command paths.
//!
//! The story (doc 39 §6, the IN.3 row): a captain sails a Dray-P to Stickney Yards with two
//! plants' bills in the hold. At the Stickney works (a two-slot works on §4.4's row, Public):
//!
//! 1. `+works/quote hauler_plant` prints the bill, twenty IC days, the fee and the booking fee.
//! 2. `+works/order` twice books orders A and B: an escrow each, the booking fee held for each.
//! 3. `+works/deliver` moves each bill from the docked hold into its escrow; on the next fire the
//!    arm submits both, one slot each, and the fee is held at `fee(order)`.
//! 4. A stranger with an account (no tie to the Yards) orders a fittings batch and supplies it:
//!    no slot is free, so it queues. The Yards' own officer cannot cancel it.
//! 5. Day 3: the captain cancels B. The whole bill comes back to the hold; the booking fee and
//!    the three days of work are forfeit to the Yards; the freed slot runs the stranger's order.
//! 6. The world clock passes day 7: A's materials stay reserved (the deadline is the order's
//!    duration plus the TTL).
//! 7. Day 11: cancelling A is refused (past half its work).
//! 8. Day 20: the arm delivers A; `+works/collect` puts the hauler plant in the hold, the booking
//!    fee is released and the fee is paid to the Yards.
//! 9. The stranger never collects: 28 IC days after the delivery, storage is charged a slot-day
//!    a day.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the works itself — a facility at the Service Counter owned
//! by Stickney Yards, Public, on §4.4's works row (`@facility/open`, `/owner`, `/terms`); the
//! lane's works rows and rooms are IN.5's — and its two recipes (a test pack: the plant and
//! fittings recipes are IN.5's and IN.7's content); the holds' goods (`seed_hold`); the
//! officer's grant over the Yards. Every claim — the quote, the bookings, the deliveries, the
//! queue, the refusals, the cancel's arithmetic, the deadline, the delivery, the collection and
//! the storage — is reached by the players' commands and the port tick.
//!
//! **No site**: the order contract is port-side, so it has no 256/512 pair (IN.2's and IN.4's
//! precedent; doc 39 §6 runs a story at both sizes "where the ground is in it").

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, EconomyStore};
use mush_industry::{IndustryHandles, WorkOrder, WorkOrderId, WorkOrderState};
use mush_inventory::record::EndpointRef;
use tacsim::port::HullRow;
use tacsim::port::works::{Close, WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, cid, file_and_launch, hold_pool, hold_vault_of, ic_now, load, port_map, prospector,
    run, seed_hold, station_room, stocked_tuned, store, to_ic, typed,
};

const STICKNEY: &str = "stickney_port";
const PLANT: &str = "tacsim.part.plant.hauler_mk1";
const ALLOYS: &str = "structural_alloys";
const FITTINGS: &str = "industrial_parts";
const DAY: i64 = 86_400;

/// The test pack: a 20-day hauler plant (480 work units) and a one-day fittings batch, both at
/// a works. Signed at load (the content layer's digest over the authored bytes).
fn recipes() -> String {
    let body = r#"{
  "manifest": { "id": "tacsim.test.works", "version": 1, "kind": "rules_definition",
                "digest": "DIGEST", "requires": [] },
  "entries": {
    "hauler_plant": { "kind": "recipe", "def": {
      "label": "Hauler power plant",
      "inputs": [ { "commodity": "structural_alloys", "units": 24 },
                  { "commodity": "industrial_parts", "units": 6 } ],
      "outputs": [ { "commodity": "tacsim.part.plant.hauler_mk1", "units": 1 } ],
      "requires": ["tacsim.works"], "work_units": 480, "output_lots": "each" } },
    "fittings_batch": { "kind": "recipe", "def": {
      "label": "Fittings batch",
      "inputs": [ { "commodity": "structural_alloys", "units": 5 } ],
      "outputs": [ { "commodity": "industrial_parts", "units": 5 } ],
      "requires": ["tacsim.works"], "work_units": 6 } }
  }
}"#;
    let text = body.replace("DIGEST", &"0".repeat(64));
    let parsed = mush_content::parse_pack(&text).expect("the pack parses");
    let digest = parsed.computed_digest().expect("digests").to_hex();
    body.replace("DIGEST", &digest)
}

fn mint(rig: &mut Rig, pilot: ObjId) -> AccountId {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
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

/// An account's posted balance and held amount, minor units.
fn money(rig: &Rig, account: AccountId) -> (i64, i64) {
    let a = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).cloned())
        .expect("the account");
    (a.posted_balance.minor(), a.held_amount.minor())
}

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

fn contracts(rig: &Rig) -> Vec<WorksContract> {
    store(&rig.tw).works_contracts()
}

fn contract(rig: &Rig, id: u64) -> WorksContract {
    match store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

fn order_of(rig: &mut Rig, id: u64) -> WorkOrder {
    let order = contract(rig, id).order.expect("bound to its work order");
    let families = rig
        .tw
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    typed(&mut rig.tw, &families.work_orders, &WorkOrderId(order)).expect("the work order")
}

/// A command, its submissions landed, then `n` port fires landed.
fn typed_in(rig: &mut Rig, who: ObjId, line: &str, fires: usize) -> String {
    let out = run(&mut rig.tw, who, line);
    let refused = pump(rig);
    assert!(
        refused.is_empty(),
        "{line}: {refused:?}
{out}"
    );
    settle(rig, fires);
    out
}

/// Every submission staged, the industry's included (the rig's `drain` keeps tacsim's,
/// inventory's and the market's only), landed in order; refusals are the story's to read.
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

/// **Every due production occurrence**, landed as the server's schedule driver lands it (the
/// test world drives no schedule source of its own; `m8_world`'s and `ic_slice_loop`'s idiom).
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

/// One port fire (6 s on), everything it staged landed.
fn fire_all(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    pump(rig)
}

/// The IC clock to `target`, everything staged on the way landed.
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

/// Two of the industry sweep's 60-second rotations, every submission landed: a cancelled or
/// completed order's budget settles there.
fn swept(rig: &mut Rig) {
    for _ in 0..2 {
        let t = rig.tw.world().now().saturating_add(61);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let _ = pump(rig);
    }
}

fn settle(rig: &mut Rig, fires: usize) {
    for _ in 0..fires {
        let _ = fire_all(rig);
    }
}

/// Step the IC clock to `target` in jumps the production catch-up can take, firing between.
fn to_day(rig: &mut Rig, start: i64, day: f64) {
    #[allow(clippy::cast_possible_truncation)]
    let target = start + (day * DAY as f64) as i64;
    while ic_now(&rig.tw) < target {
        let next = (ic_now(&rig.tw) + 8 * DAY).min(target);
        ic_to(rig, next);
        settle(rig, 2);
    }
}

fn new_id(rig: &Rig, before: &[u64]) -> u64 {
    contracts(rig)
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("a new order")
}

fn booked(rig: &mut Rig, who: ObjId, line: &str) -> u64 {
    let before: Vec<u64> = contracts(rig).into_iter().map(|c| c.id).collect();
    let out = typed_in(rig, who, line, 1);
    assert!(out.contains("booked at the works"), "{out}");
    let id = new_id(rig, &before);
    assert!(
        contract(rig, id).order.is_some(),
        "the draft landed and bound"
    );
    id
}

fn sail_to_stickney(rig: &mut Rig, crews: &[(ObjId, &HullRow, u64)]) {
    let mut arrive = 0;
    for (pilot, row, seed) in crews {
        load(rig, row, 60, 40, *seed);
        board(rig, *pilot);
        let t = file_and_launch(rig, *pilot, ObjId(row.hull), "+ship/undock stickney");
        arrive = arrive.max(t.plan.depart_ic + t.plan.total_s);
    }
    let _ = to_ic(rig, arrive);
    let _ = to_ic(rig, arrive + 3_600);
    for (_, row, _) in crews {
        assert!(
            matches!(
                tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)),
                tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY
            ),
            "docked at Stickney Yards"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in3_a_twenty_day_plant_order_at_stickney_runs_to_collection() {
    let mut rig = stocked_tuned(|cfg| {
        cfg.packs.push(tacsim::PackSource {
            name: "<test tacsim.test.works>".into(),
            bytes: recipes(),
        });
    });

    // ---- The captain and the stranger, both to Stickney with their bills ----
    let (captain, captain_row) = prospector(&mut rig, "Hauler", 1, "Long Ox");
    let captain_account = mint(&mut rig, captain);
    let (stranger, stranger_row) = prospector(&mut rig, "Stranger", 2, "Grey Wren");
    let stranger_account = mint(&mut rig, stranger);
    seed_hold(&mut rig, &captain_row, ALLOYS, 48, 9_600);
    seed_hold(&mut rig, &captain_row, FITTINGS, 12, 9_601);
    seed_hold(&mut rig, &stranger_row, ALLOYS, 5, 9_602);
    sail_to_stickney(
        &mut rig,
        &[
            (captain, &captain_row, 9_500),
            (stranger, &stranger_row, 9_510),
        ],
    );
    let captain_hold = hold_vault_of(&rig.tw, &captain_row);
    let stranger_hold = hold_vault_of(&rig.tw, &stranger_row);

    // ---- The seed: the Stickney works, owned by the Yards, Public, on §4.4's works row ----
    let counter = station_room(&rig.tw, STICKNEY, "room.p11_service");
    let works = made(&mut rig, "Stickney works");
    rig.tw
        .world_mut()
        .move_to(works, counter)
        .expect("the works into the Service Counter");
    let vendor = port_map(&rig.tw, STICKNEY).vendors[&cid("stickney_yards")]
        .vendor
        .expect("the yard's vendor");
    let yards = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| m.vendor(mush_market::VendorId(vendor)).map(|v| v.org))
        .expect("the yard's org");
    for line in [
        format!("@facility/open {}=2 tacsim.works", works.0),
        format!("@facility/owner {}=org:{}", works.0, yards.0),
        format!("@facility/terms {}=public 3000000/25/20/8/150/250", works.0),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Usage"), "{line}: {out}");
    }
    let officer = rig.tw.spawn_player("Officer");
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@org/grant #{}=#{} economy.account.post",
            officer.0, yards.0
        ),
    );
    assert!(!granted.contains("Permission denied"), "{granted}");
    let yards_account = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")]
        .account
        .map(AccountId)
        .expect("the Yards' account");
    for who in [captain, stranger, officer] {
        rig.tw
            .world_mut()
            .move_to(who, counter)
            .expect("to the counter");
    }

    // (1) The quote.
    let quote = run(&mut rig.tw, captain, "+works/quote hauler_plant");
    assert!(
        quote.contains("bill 24 structural_alloys, 6 industrial_parts")
            && quote.contains("20 IC days (480 work units)"),
        "{quote}"
    );
    let menu = run(&mut rig.tw, captain, "+works");
    assert!(
        menu.contains("2 slots, 2 free") && menu.contains("hauler_plant"),
        "{menu}"
    );

    // (2) Two bookings: an escrow each and the booking fee held for each.
    let (_, held_before) = money(&rig, captain_account);
    let a = booked(&mut rig, captain, "+works/order hauler_plant");
    let b = booked(&mut rig, captain, "+works/order hauler_plant");
    let booking = i64::try_from(contract(&rig, a).booking_minor).expect("fits");
    assert_eq!(
        booking, 84_700,
        "one slot-day of the works row: 846.58 cr, billed 847"
    );
    assert_eq!(
        money(&rig, captain_account).1 - held_before,
        2 * booking,
        "both booking fees held"
    );
    let c = contract(&rig, a);
    assert_eq!(c.payer, mush_org::PrincipalId::Object(captain));
    assert_eq!(c.beneficiary, mush_org::PrincipalId::Object(captain));
    assert_eq!(c.requester, captain);

    // (3) The bills, delivered from the docked hold.
    for id in [a, b] {
        for (good, units) in [(ALLOYS, 24), (FITTINGS, 6)] {
            let out = typed_in(
                &mut rig,
                captain,
                &format!("+works/deliver {id}={good} {units}"),
                1,
            );
            assert!(out.contains("moves into order"), "{out}");
            assert_eq!(
                hold_pool(&mut rig.tw, mush_economy::VaultId(id), good),
                units,
                "order {id}'s escrow holds the {good}"
            );
        }
    }
    assert_eq!(hold_pool(&mut rig.tw, captain_hold, ALLOYS), 0);
    settle(&mut rig, 2);
    let start = ic_now(&rig.tw);
    for id in [a, b] {
        let order = order_of(&mut rig, id);
        assert_eq!(*order.state(), WorkOrderState::Running, "order {id} runs");
    }

    // (4) The stranger orders and supplies; no slot is free, so it queues.
    let s = booked(&mut rig, stranger, "+works/order fittings_batch");
    let out = typed_in(
        &mut rig,
        stranger,
        &format!("+works/deliver {s}={ALLOYS} 5"),
        2,
    );
    assert!(out.contains("moves into order"), "{out}");
    assert_eq!(*order_of(&mut rig, s).state(), WorkOrderState::Draft);
    let shown = run(&mut rig.tw, stranger, "+works");
    assert!(shown.contains("supplied, waiting for a slot"), "{shown}");
    // The Yards' officer cannot cancel the stranger's order.
    let refused = run(&mut rig.tw, officer, &format!("+works/cancel {s}"));
    assert!(refused.contains("Only the one who booked"), "{refused}");
    assert!(pump(&mut rig).is_empty(), "the refusal stages nothing");

    // (5) Day 3: B is cancelled; the bill comes back whole.
    to_day(&mut rig, start, 3.0);
    let (posted_before, _) = money(&rig, captain_account);
    let (yards_before, _) = money(&rig, yards_account);
    let ran = order_of(&mut rig, b)
        .schedule()
        .map(mush_industry::ProductionSchedule::progress_units)
        .expect("a schedule");
    assert!((70..=72).contains(&ran), "three days of work: {ran}");
    let out = typed_in(&mut rig, captain, &format!("+works/cancel {b}"), 3);
    assert!(out.contains("cancelled"), "{out}");
    settle(&mut rig, 2);
    swept(&mut rig);
    assert_eq!(*order_of(&mut rig, b).state(), WorkOrderState::Cancelled);
    assert_eq!(contract(&rig, b).closed, Some(Close::Cancelled));
    assert_eq!(
        hold_pool(&mut rig.tw, captain_hold, ALLOYS),
        24,
        "the alloys back"
    );
    assert_eq!(
        hold_pool(&mut rig.tw, captain_hold, FITTINGS),
        6,
        "the fittings back"
    );
    let worked = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.facility(works))
        .and_then(|f| f.fee_minor(ran))
        .expect("priced");
    let forfeit = booking + i64::try_from(worked).expect("fits");
    assert_eq!(
        posted_before - money(&rig, captain_account).0,
        forfeit,
        "the booking fee and {ran} work units' fee forfeit"
    );
    assert_eq!(
        money(&rig, yards_account).0 - yards_before,
        forfeit,
        "paid to the Yards"
    );
    // The freed slot ran the stranger's order.
    let queued = *order_of(&mut rig, s).state();
    assert!(
        matches!(
            queued,
            WorkOrderState::Running
                | WorkOrderState::AwaitingDelivery
                | WorkOrderState::Completed { .. }
        ),
        "the queued order took the slot: {queued:?}"
    );

    // (6) Past day 7: A's materials are still reserved, to its duration plus the TTL.
    to_day(&mut rig, start, 8.0);
    let a_order = order_of(&mut rig, a);
    assert_eq!(*a_order.state(), WorkOrderState::Running);
    let inventory = rig
        .tw
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry")
        .0
        .inventory
        .clone();
    let key = mush_inventory::pool_key(EndpointRef(mush_economy::VaultId(a)), &cid(ALLOYS))
        .expect("a key");
    let book = typed(&mut rig.tw, &inventory.books, &key).expect("the escrow's book");
    let held = book
        .records()
        .iter()
        .find(|r| r.state() == mush_core::ReservationState::Held)
        .expect("the alloys are held");
    assert!(
        held.expires().at >= start + 20 * DAY + mush_inventory::RESERVATION_TTL_SECS - DAY,
        "the deadline is the work plus the TTL"
    );

    // (7) Day 11: past half its work, A cannot be cancelled.
    to_day(&mut rig, start, 11.0);
    let refused = run(&mut rig.tw, captain, &format!("+works/cancel {a}"));
    assert!(refused.contains("past half its work"), "{refused}");
    assert!(pump(&mut rig).is_empty(), "the refusal stages nothing");

    // (8) Day 20: delivered by the arm, collected into the hold.
    to_day(&mut rig, start, 20.5);
    settle(&mut rig, 3);
    assert!(
        matches!(
            order_of(&mut rig, a).state(),
            WorkOrderState::Completed { .. }
        ),
        "the arm delivered A"
    );
    assert!(contract(&rig, a).delivered_ic.is_some());
    let out = typed_in(&mut rig, captain, &format!("+works/collect {a}"), 2);
    swept(&mut rig);
    assert!(out.contains("move into your hold"), "{out}");
    assert_eq!(contract(&rig, a).closed, Some(Close::Collected));
    assert_eq!(
        hold_pool(&mut rig.tw, captain_hold, PLANT)
            + typed(
                &mut rig.tw,
                &inventory.lot_tally,
                &mush_inventory::pool_key(EndpointRef(captain_hold), &cid(PLANT)).expect("key"),
            )
            .unwrap_or(0),
        1,
        "the hauler plant is in the hold"
    );
    let fee = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.facility(works))
        .and_then(|f| f.fee_minor(480))
        .map(|f| i64::try_from(f).expect("fits"))
        .expect("priced");
    assert_eq!(
        money(&rig, captain_account),
        (posted_before - forfeit - fee, 0),
        "A's fee paid, its booking fee released, nothing left held"
    );

    // (9) The stranger never collects: storage after 28 days.
    let delivered = contract(&rig, s).delivered_ic.expect("delivered");
    let (stranger_before, _) = money(&rig, stranger_account);
    let target = delivered + 28 * DAY + 2 * DAY + 3_600;
    while ic_now(&rig.tw) < target {
        let next = (ic_now(&rig.tw) + 8 * DAY).min(target);
        ic_to(&mut rig, next);
        settle(&mut rig, 2);
    }
    assert_eq!(contract(&rig, s).storage_days, 2, "two days charged");
    assert_eq!(
        stranger_before - money(&rig, stranger_account).0,
        2 * booking,
        "a slot-day a day"
    );
    assert_eq!(hold_pool(&mut rig.tw, stranger_hold, FITTINGS), 0);
}
