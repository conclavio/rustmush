//! **A rig built at Stickney's yard for a field story** (doc 47 unit CO.10b): the yard's own
//! command path (`+yard/quote`, `+yard/order`, `+yard/deliver`, `+yard/collect`, `+bay/stow`)
//! driven from a chartered Dray-P, as doc 39 cut IN.9's yard story drives it from a yard-built
//! one. The helpers below are that story's (`domain_economy/yard_in9.rs`), which lives in another
//! test binary.

use mush_core::{GOD, ObjId};
use mush_inventory::LotId;
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::commission::{Step, YardCommission};
use tacsim::port::works::{WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, hold_vault_of, ic_now, load, port_map, run, seed_pool, station_room, store,
    teleport,
};

const STICKNEY: &str = "stickney_port";
const DAY: i64 = 86_400;

fn contracts(rig: &Rig) -> Vec<WorksContract> {
    store(&rig.tw).works_contracts()
}

fn contract(rig: &Rig, id: u64) -> WorksContract {
    match store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

fn commission(rig: &Rig, order: u64) -> YardCommission {
    tacsim::port::commission::commission(rig.tw.world(), order)
        .cloned()
        .unwrap_or_else(|| panic!("the commission of order {order}"))
}

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

/// One port fire (6 s on), everything it staged landed.
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

/// A command, its submissions landed, then `fires` port fires.
fn typed_in(rig: &mut Rig, who: ObjId, line: &str, fires: usize) -> String {
    let out = run(&mut rig.tw, who, line);
    let refused = pump(rig);
    assert!(refused.is_empty(), "{line}: {refused:?}\n{out}");
    settle(rig, fires);
    out
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

fn booked(rig: &mut Rig, who: ObjId, line: &str) -> u64 {
    let before: Vec<u64> = contracts(rig).into_iter().map(|c| c.id).collect();
    let out = typed_in(rig, who, line, 1);
    assert!(out.contains("booked at the yard"), "{line}: {out}");
    let id = contracts(rig)
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("a new order");
    assert!(
        contract(rig, id).order.is_some(),
        "the draft landed and bound"
    );
    id
}

/// Money for the yard's work (a fixture standing for the captain's earnings).
fn mint(rig: &mut Rig, pilot: ObjId) {
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
}

/// Sail `hull` from where `pilot` stands aboard her with `line`, to the landing and an hour on.
fn sail(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str, entry: &str) {
    let t = crate::driving_rig::file_and_launch(rig, pilot, hull, line);
    let _ = crate::driving_rig::to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    let _ = crate::driving_rig::to_ic(rig, t.plan.depart_ic + t.plan.total_s + 3_600);
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry),
        "{line}: docked at {entry}"
    );
}

/// `+ship/leave` from the airlock the pilot boarded into: ashore at the berth's gallery.
fn ashore(rig: &mut Rig, pilot: ObjId) {
    let aboard = rig.tw.world().get(pilot).and_then(|o| o.location);
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert_ne!(
        rig.tw.world().get(pilot).and_then(|o| o.location),
        aboard,
        "ashore: {out}"
    );
}

/// **The wrecker, built at Stickney and stowed in `pilot`'s chartered Dray-P** (CO.10b), which
/// is docked at Ceres and comes home there with her in the bay. The Dray-P sails to Stickney;
/// `+yard/quote tv_wrecker` prints the bill the design gives (9 t of alloys, 3 t of fittings, a
/// plant and a sight, two IC days); `+yard/order tv_wrecker` books it; `+yard/deliver` brings
/// the bill out of the Dray-P's hold (the fixture: the goods in the hold, bought earlier);
/// `+yard/collect` commissions her, titled to the pilot; `+bay/stow` puts her in the Dray-P's
/// bay; the Dray-P sails home. Returns the wrecker.
pub(crate) fn built_wrecker(rig: &mut Rig, pilot: ObjId) -> ObjId {
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let hull = charter.hull;
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == hull.0)
        .expect("the hull's row");
    mint(rig, pilot);
    // The trip runs at the tests' 3,600:1: the Co-op's dormancy counts real days, and the round
    // trip to Phobos at the rig's own rate passes seven of them (CO.9's note), ending the charter.
    let rate = crate::driving_rig::world_rate(&rig.tw);
    crate::driving_rig::set_rate(&mut rig.tw, 3_600);
    // The round trip's propellant and provisions, and the bill, in the hold.
    load(rig, &row, 140, 120, 9_700);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_pool(rig, hold, "structural_alloys", 9, LotId(9_702));
    seed_pool(rig, hold, "industrial_parts", 3, LotId(9_703));
    let parts = [
        "tacsim.part.plant.light_mk1",
        "tacsim.part.sensor.visual_mk1",
    ];
    for (i, part) in parts.iter().enumerate() {
        crate::driving_rig::seed_part(
            rig,
            hold,
            part,
            LotId(9_704 + i as u64),
            ComponentCondition::Intact,
            ProvenanceLegalityStatus::CleanTitle,
        );
    }
    board(rig, pilot);
    sail(rig, pilot, hull, "+ship/undock stickney", STICKNEY);

    // The yard: the quote, the order, the bill delivered from the hold, the collect.
    ashore(rig, pilot);
    let floor = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, pilot, floor);
    let quote = run(&mut rig.tw, pilot, "+yard/quote tv_wrecker");
    assert!(
        quote.contains("9 structural_alloys, 3 industrial_parts") && quote.contains("2 IC days"),
        "the wrecker's bill, from its design: {quote}"
    );
    let order = booked(rig, pilot, "+yard/order tv_wrecker");
    for (good, units) in [("structural_alloys", 9), ("industrial_parts", 3)] {
        let out = typed_in(
            rig,
            pilot,
            &format!("+yard/deliver {order}={good} {units}"),
            1,
        );
        assert!(out.contains("escrow"), "{out}");
    }
    for part in parts {
        let out = typed_in(rig, pilot, &format!("+yard/deliver {order}={part} 1"), 1);
        assert!(out.contains("escrow"), "{out}");
    }
    settle(rig, 2);
    days_on(rig, 2);
    let out = typed_in(rig, pilot, &format!("+yard/collect {order}"), 3);
    assert!(out.contains("commissions her as SYB-"), "{out}");
    let done = commission(rig, order);
    assert_eq!(done.step, Step::Done, "{done:?}");
    let wrecker = done.object().expect("the wrecker");
    assert_eq!(
        mush_tactics::module::unit_of(rig.tw.world(), wrecker)
            .map(|cells| cells.schema().key().id.as_str().to_owned())
            .as_deref(),
        Some(tacsim::definitions::content::TV_WRECKER),
        "the yard built a wrecker"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|s| s.snapshot().asset(wrecker).map(|a| a.title_owner.clone())),
        Some(mush_org::PrincipalId::Object(pilot)),
        "titled to the pilot"
    );
    assert!(
        tacsim::port::owned::owned(rig.tw.world(), wrecker).is_some(),
        "a register row"
    );
    let out = typed_in(
        rig,
        pilot,
        &format!("+bay/stow #{}=#{}", wrecker.0, hull.0),
        1,
    );
    assert!(out.contains("is stowed in the"), "{out}");

    // Home to Ceres with her in the bay.
    let gallery = station_room(&rig.tw, STICKNEY, "room.p01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
    sail(rig, pilot, hull, "+ship/undock ceres", "ceres_port");
    ashore(rig, pilot);
    crate::driving_rig::set_rate(&mut rig.tw, rate);
    wrecker
}
