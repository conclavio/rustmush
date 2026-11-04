//! **Doc 39 cut IN.10 — the pools** (§4.7; Q-39-7, Q-39-21 as revised by Q-39-34), over the
//! composed world (`driving`'s keyed rig), through the production command paths and the port
//! tick.
//!
//! 1. Eight captains `+coop/enrol`: the eight programme Drays are chartered and none is free.
//!    The ninth captain hears `NO_HULL` and "the Trust has no Dray free and cannot yet fund one":
//!    the Trust's 250,000 posted eight 25,000 stakes and is at 50,000.
//! 2. Two captains `+coop/buyout`: 90,000 each comes back to the Trust, which now covers a Dray's
//!    order, and **the Trust orders** on the next port fire — its DARK helm funded from its account,
//!    the yard order booked at Stickney as the helm for the Trust, the bill bought at Stickney's
//!    asks (Kasei's alloys, the Yards' fittings, a hauler plant from the Yards' stores) into the
//!    escrow. The ninth captain now hears "the Trust has a Dray on order at Stickney, due at Ceres
//!    in n IC days".
//! 3. Twelve IC days later the yard has built her; the helm collects her into the commission, and
//!    she stands at Stickney titled to the Trust under a fresh `SYB-` mark, a register row; her
//!    stores are bought into her hold; the helm files the passage home as itself.
//! 4. She docks at Ceres under the Trust's title: a free hull of the pool, a register row and
//!    never a programme row (the map still holds eight). The helm's unspent purse is back with the
//!    Trust.
//! 5. The desk enrols the ninth captain into her.
//!
//! **Seeds** (doc 43 rule 2.3 #4): one hauler plant in the Yards' stores (IN.7 retired the
//! seeded part stock; the house buys it there at its catalogue price), the captains' money for
//! the buyouts, and the Trust's 250,000 raise (lane v28) held back in the treasury by staff from
//! boot until the ninth enrolment, so the story reads the world before the raise, as the row
//! does. Every other claim is reached by the players' commands and the port tick.

use mush_core::{GOD, ObjId};
use mush_org::PrincipalId;
use tacsim::port::pools::Phase;

use crate::driving_rig::{Rig, cid, ic_now, pilot, port_map, run, stocked, store};

const CERES: &str = "ceres_port";
const TRUST: &str = "coop_trust";
const DAY: i64 = 86_400;
const NAMES: [&str; 9] = [
    "Amber Reach",
    "Brine Lark",
    "Copper Wick",
    "Dun Heron",
    "Ember Tally",
    "Flint Moth",
    "Grey Sparrow",
    "Hollow Bell",
    "Iron Plover",
];

fn mint(rig: &mut Rig, account: u64, major: u64) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} {}", major * 100),
    );
    assert!(!out.contains("Usage"), "{out}");
}

/// Every submission staged, the industry's included, landed in order.
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

/// Every due production occurrence, landed as the server's schedule driver lands it.
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

fn fire_all(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    let _ = pump(rig);
}

fn settle(rig: &mut Rig, fires: usize) {
    for _ in 0..fires {
        fire_all(rig);
    }
}

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

/// The IC clock `days` on, in jumps the production catch-up can take, firing between.
fn days_on(rig: &mut Rig, days: i64) {
    let target = ic_now(&rig.tw) + days * DAY + 3_600;
    while ic_now(&rig.tw) < target {
        let next = (ic_now(&rig.tw) + 4 * DAY).min(target);
        ic_to(rig, next);
        settle(rig, 2);
    }
}

/// Fires until `done`, at most `limit`.
fn fires_until(rig: &mut Rig, limit: usize, mut done: impl FnMut(&Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        fire_all(rig);
    }
    done(rig)
}

fn order_of(rig: &mut Rig, order: Option<u64>) -> Option<mush_industry::WorkOrder> {
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
        &mush_industry::WorkOrderId(order?),
    )
}

/// The Trust's 250,000 raise held back in the treasury (`true`), or given back.
fn hold(rig: &mut Rig, back: bool) {
    let trust = trust_account(rig);
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let (from, to) = if back {
        (trust, treasury)
    } else {
        (treasury, trust)
    };
    let out = run(&mut rig.tw, GOD, &format!("@econ/pay {from}={to} 25000000"));
    assert!(!out.contains("Usage"), "{out}");
    settle(rig, 1);
}

fn trust_pool(rig: &Rig) -> Option<tacsim::port::pools::HousePool> {
    tacsim::port::pools::pool(rig.tw.world(), TRUST).cloned()
}

fn phase(rig: &Rig) -> Option<Phase> {
    trust_pool(rig).and_then(|p| p.orders.first().map(|o| o.phase))
}

fn trust_account(rig: &Rig) -> u64 {
    port_map(&rig.tw, CERES).orgs[&cid(TRUST)]
        .account
        .expect("the Trust's account")
}

fn available(rig: &Rig, account: u64) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(mush_economy::AccountId(account))
                .cloned()
        })
        .and_then(|a| a.available())
        .map(|a| a.minor())
        .expect("the account")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in10_the_trust_restocks_its_pool_from_the_yard_and_ferries_her_home() {
    let mut rig = stocked();
    // IN.7 retired the seeded part stock: staff stock the Yards' stores with the hauler plant a
    // Dray's bill takes (`seed_store_parts`, the seed).
    let _ = crate::driving_rig::seed_store_parts(
        &mut rig,
        "stickney_port",
        "stickney_yards",
        "tacsim.part.plant.hauler_mk1",
        1,
        9_100,
    );
    // The Yards' part stores seed after the asks (IN.6): let the clerk finish.
    settle(&mut rig, 40);
    // The Trust before its raise: lane v28 posts it 250,000 more at boot, which staff hold back
    // in the treasury until the story's last step (the world the row reads, before v28).
    assert_eq!(
        available(&rig, trust_account(&rig)),
        500_000 * 100,
        "the base and the raise"
    );
    hold(&mut rig, true);

    // ---- (1) Eight charters leave no free Dray ----
    let captains: Vec<ObjId> = NAMES
        .iter()
        .enumerate()
        .map(|(i, _)| {
            pilot(
                &mut rig,
                &format!("Captain{i}"),
                i64::try_from(i).unwrap() + 1,
            )
        })
        .collect();
    for (captain, name) in captains.iter().zip(NAMES).take(8) {
        let out = run(&mut rig.tw, *captain, &format!("+coop/enrol {name}"));
        assert!(out.contains(&format!("now the {name}")), "{out}");
        settle(&mut rig, 1);
    }
    let trust = trust_account(&rig);
    assert_eq!(
        available(&rig, trust),
        50_000 * 100,
        "eight stakes off 250,000"
    );
    assert!(
        trust_pool(&rig).is_none_or(|p| p.orders.is_empty()),
        "the floor is breached but the Trust cannot fund a Dray"
    );
    let ninth = captains[8];
    let out = run(&mut rig.tw, ninth, &format!("+coop/enrol {}", NAMES[8]));
    assert!(
        out.contains(tacsim::port::coop::NO_HULL)
            && out.contains(tacsim::port::coop::NO_HULL_UNFUNDED),
        "{out}"
    );

    // ---- (2) Two buyouts later the Trust orders ----
    let floor = tacsim::port::pools::FLOORS[0];
    let price = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        tacsim::port::pools::price_minor(ctx, &floor)
    })
    .expect("the Dray's price");
    eprintln!("in10: a house Dray costs the Trust {price} minor");
    assert!(
        price > 50_000 * 100 && price <= 230_000 * 100,
        "the Trust cannot fund one at 50,000 and can after two buyouts: {price}"
    );
    for captain in &captains[..2] {
        let account = tacsim::port::credit::own_account(rig.tw.world(), *captain)
            .map(|a| a.id.0)
            .expect("the captain's account");
        mint(&mut rig, account, 100_000);
    }
    settle(&mut rig, 2);
    for captain in &captains[..2] {
        let out = run(&mut rig.tw, *captain, "+coop/buyout");
        assert!(
            out.contains("The Registry writes the title transfer"),
            "{out}"
        );
    }
    assert!(
        fires_until(&mut rig, 60, |rig| phase(rig).is_some()),
        "the Trust orders: {:?}",
        trust_pool(&rig)
    );
    let record = trust_pool(&rig).expect("the Trust's pool");
    let helm = record.helm.expect("the helm");
    assert!(
        rig.tw
            .world()
            .get(helm)
            .is_some_and(|o| o.flags.contains(mush_core::Flags::DARK)),
        "the helm is DARK"
    );
    assert!(
        fires_until(&mut rig, 40, |rig| phase(rig) == Some(Phase::Supplying)),
        "booked: {:?}",
        trust_pool(&rig)
    );
    let out = run(&mut rig.tw, ninth, &format!("+coop/enrol {}", NAMES[8]));
    assert!(
        out.contains(tacsim::port::coop::NO_HULL)
            && out.contains("The Trust has a Dray on order at Stickney Yards")
            && out.contains("due at Ceres in"),
        "{out}"
    );
    assert!(
        fires_until(&mut rig, 200, |rig| phase(rig) == Some(Phase::Building)),
        "the bill is bought into the escrow: {:?}",
        trust_pool(&rig)
    );
    let contract = trust_pool(&rig).unwrap().orders[0]
        .contract
        .expect("the yard order");
    let c = match store(&rig.tw).works_contract(contract) {
        Some(tacsim::port::works::WorksState::Present(c)) => (**c).clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(c.requester, helm, "the helm booked it");
    assert_eq!(
        c.beneficiary,
        PrincipalId::Organization(mush_org::OrgId(
            port_map(&rig.tw, CERES).orgs[&cid(TRUST)].org
        )),
        "for the Trust"
    );

    // ---- (3) Twelve IC days later: the commission, the stores, the passage ----
    let ordered = trust_pool(&rig).unwrap().orders[0].ordered_ic;
    for _ in 0..20 {
        if phase(&rig) != Some(Phase::Building) {
            break;
        }
        days_on(&mut rig, 1);
    }
    let built_in = (ic_now(&rig.tw) - ordered) / DAY;
    eprintln!("in10: the yard handed her over {built_in} IC days after the order");
    assert!(built_in <= 14, "fourteen IC days at most: {built_in}");
    let sailing = fires_until(&mut rig, 120, |rig| phase(rig) == Some(Phase::Sailing));
    if !sailing {
        let c = store(&rig.tw).works_contract(contract).cloned();
        let order = order_of(
            &mut rig,
            c.as_ref().and_then(|c| match c {
                tacsim::port::works::WorksState::Present(c) => c.order,
                tacsim::port::works::WorksState::Unreadable => None,
            }),
        );
        panic!(
            "commissioned, provisioned and filed: {:?}
contract {c:?}
order {order:?}
commission {:?}",
            trust_pool(&rig),
            tacsim::port::commission::commission(rig.tw.world(), contract)
        );
    }
    let hull = trust_pool(&rig).unwrap().orders[0].hull.expect("the hull");
    let owned = tacsim::port::owned::owned(rig.tw.world(), hull)
        .cloned()
        .expect("a register row");
    assert!(owned.mark.starts_with("SYB-"), "{owned:?}");
    let pending = tacsim::port::voyage::pending_of(rig.tw.world(), hull)
        .cloned()
        .expect("the passage filed");
    let arrival = pending.plan.depart_ic + pending.plan.total_s;

    // ---- (4) She docks at Ceres under the Trust's title ----
    let _ = crate::driving_rig::to_ic(&mut rig, arrival + 3_600);
    assert!(
        fires_until(&mut rig, 60, |rig| trust_pool(rig)
            .is_some_and(|p| p.orders.is_empty())),
        "home: {:?}",
        trust_pool(&rig)
    );
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == CERES),
        "docked at Ceres"
    );
    assert_eq!(
        port_map(&rig.tw, CERES).hulls.len(),
        8,
        "never a programme row"
    );
    let port = tacsim::port::coop::programme_port(rig.tw.world()).expect("the programme");
    assert!(
        tacsim::port::coop::free_hulls(rig.tw.world(), &port)
            .iter()
            .any(|row| row.hull == hull.0),
        "a free hull of the pool"
    );
    let purse = trust_pool(&rig).unwrap().account.expect("the helm's purse");
    assert_eq!(available(&rig, purse), 0, "the unspent purse is home");

    // ---- (5) The desk enrols the ninth captain ----
    // The Trust's raise comes in (the hold-back given back).
    hold(&mut rig, false);
    let out = run(&mut rig.tw, ninth, &format!("+coop/enrol {}", NAMES[8]));
    assert!(out.contains(&format!("now the {}", NAMES[8])), "{out}");
    // The desk lends the lowest free hull: over these weeks the dormancy recall (seven real days on
    // the rig's clock) may hand a programme Dray back first, so the ninth captain sails one of
    // the two, and the Trust's built Dray is chartered or still free in the pool.
    let chartered = tacsim::port::live_charter(rig.tw.world(), ninth)
        .map(|c| c.hull)
        .expect("the ninth captain is chartered");
    let port = tacsim::port::coop::programme_port(rig.tw.world()).expect("the programme");
    assert!(
        chartered == hull
            || tacsim::port::coop::free_hulls(rig.tw.world(), &port)
                .iter()
                .any(|row| row.hull == hull.0),
        "the Trust's ninth Dray is the ninth captain's or still free"
    );
}
