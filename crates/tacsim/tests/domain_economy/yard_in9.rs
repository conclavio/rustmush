//! **Doc 39 cut IN.9 — the yard and the handover** (§4.7, §4.13; Q-39-14, Q-39-18..20, Q-39-33,
//! Q-39-36), over the composed world (`driving`'s keyed rig), through the production command
//! paths.
//!
//! **The yard half (this test).** A captain who enrolled and handed the charter straight back has
//! an account and no hull. At Stickney Yards:
//!
//! 1. `+yard/order dray_p` books a Dray-P; its bill is laid in the escrow (a fixture: "a Dray-P
//!    the yard built for them earlier in the story"); twelve IC days later `+yard/collect` runs
//!    the commission. The Dray-P stands docked at Stickney titled to them under a fresh `SYB-`
//!    mark, her hold theirs; she is a register row (`tacsim.owned_hulls`) and a provenance
//!    record, never a programme row (the applier's preflight still passes).
//! 2. `+yard/quote tv_escort` prints the escort's bill from its design (3 t of alloys, 1 t of
//!    fittings, the four parts) and two days; `+yard/order tv_escort` books it.
//! 3. `+yard/deliver` brings the bill from the Dray-P's hold in two deliveries — the bulk, then
//!    the four part lots.
//! 4. Two IC days later `+yard/collect` writes the commission, and **staff stop it** after its
//!    first steps (between the kit's consumption and the title); the next port fire resumes it and
//!    it finishes: the escort stands on the workshop floor titled to them, berthed with no
//!    carrier, its magazines empty, the kit and parts consumed, the provenance naming them.
//! 5. `+bay <escort>` quotes her clean but for the rearm; `+bay/order <escort>=rearm` loads her
//!    from the Dray-P's hold.
//! 6. `+bay/stow <escort>=<Dray-P>` puts her in the Dray-P's bay (the register's carrier scan):
//!    the bay's facility is the Dray-P's.
//! 7. **The Co-op's escort through an officer**: the captain, granted office in the Co-op,
//!    orders a second escort `=coop` and collects it titled to the Co-op; as its officer they stow
//!    it in their Dray-P and board it by its title view (`acts_for`), and their crewmate without
//!    office cannot.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the Dray-P's bill in its escrow; the
//! escort's bill and the munitions in the Dray-P's hold; the money; the officer's grant; the
//! captain's passage to Stickney (a move). Every claim is reached by the players' commands and
//! the port tick.

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_org::PrincipalId;
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::commission::{Step, YardCommission};
use tacsim::port::works::{Close, WorksContract, WorksState};

use crate::driving_rig::{
    Rig, cid, hold_pool, ic_now, pilot, port_map, run, seed_lot, seed_part, seed_pool,
    station_room, stocked, store, teleport,
};

const STICKNEY: &str = "stickney_port";
const ALLOYS: &str = "structural_alloys";
const FITTINGS: &str = "industrial_parts";
const DAY: i64 = 86_400;

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
        .unwrap_or_else(|| panic!("commission {order}"))
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

/// One port fire (6 s on), everything it staged landed.
fn fire_all(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    pump(rig)
}

fn settle(rig: &mut Rig, fires: usize) {
    for _ in 0..fires {
        let _ = fire_all(rig);
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

/// The magazines' rounds on `unit`, summed.
fn rounds(rig: &Rig, unit: ObjId) -> i64 {
    mush_tactics::module::unit_of(rig.tw.world(), unit)
        .expect("the unit")
        .cells()
        .filter(|(addr, _)| {
            matches!(addr, mush_tactics::CellAddr::Component { cell, .. } if cell.as_str() == "ammo")
        })
        .map(|(_, cell)| cell.current)
        .sum()
}

fn name_of(rig: &Rig, obj: ObjId) -> String {
    rig.tw
        .world()
        .get(obj)
        .map(|o| o.name.to_string())
        .expect("a thing")
}

fn titled(rig: &Rig, obj: ObjId) -> PrincipalId {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().asset(obj).map(|a| a.title_owner.clone()))
        .expect("an asset")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in9_the_yard_builds_a_dray_p_and_an_escort_and_hands_them_over() {
    let mut rig = stocked();

    // ---- The captain: enrolled and released, an account and no hull ----
    let captain = pilot(&mut rig, "Builder", 1);
    let out = run(&mut rig.tw, captain, "+coop/enrol Long Ox");
    assert!(out.contains("now the Long Ox"), "{out}");
    let out = run(&mut rig.tw, captain, "+coop/release");
    assert!(!out.contains("Usage"), "{out}");
    assert!(
        tacsim::port::live_charter(rig.tw.world(), captain).is_none(),
        "the charter is handed back: {out}"
    );
    mint(&mut rig, captain);
    let floor = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, floor);

    // ---- The yard: lane v27's row, Stickney Yards' slipway counter over the drydocks ----
    let menu = run(&mut rig.tw, captain, "+yard");
    assert!(
        menu.contains("dray_p") && menu.contains("tv_escort"),
        "{menu}"
    );

    // ---- (1) The Dray-P, built for them earlier in the story ----
    let dray_order = booked(&mut rig, captain, "+yard/order dray_p");
    let bill = contract(&rig, dray_order).bill;
    assert!(
        bill.iter()
            .any(|g| g.good.as_str() == ALLOYS && g.units == 163)
            && bill
                .iter()
                .any(|g| g.good.as_str() == FITTINGS && g.units == 55)
            && bill
                .iter()
                .any(|g| g.good.as_str() == "tacsim.part.plant.hauler_mk1" && g.units == 1),
        "the Dray-P's bill is her design's: {bill:?}"
    );
    let escrow = VaultId(dray_order);
    seed_pool(&mut rig, escrow, ALLOYS, 163, LotId(9_700));
    seed_pool(&mut rig, escrow, FITTINGS, 55, LotId(9_701));
    seed_lot(
        &mut rig,
        escrow,
        "tacsim.part.plant.hauler_mk1",
        1,
        LotId(9_702),
    );
    settle(&mut rig, 2);
    days_on(&mut rig, 12);
    let out = typed_in(&mut rig, captain, &format!("+yard/collect {dray_order}"), 3);
    assert!(out.contains("commissions her as SYB-01"), "{out}");
    let built = commission(&rig, dray_order);
    assert_eq!(built.step, Step::Done, "{built:?}");
    let dray = built.object().expect("the Dray-P");
    assert_eq!(titled(&rig, dray), PrincipalId::Object(captain));
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), dray),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY),
        "docked at Stickney"
    );
    let owned = tacsim::port::owned::owned(rig.tw.world(), dray).expect("a register row");
    assert_eq!(owned.mark, "SYB-01");
    assert!(
        port_map(&rig.tw, "ceres_port")
            .hulls
            .iter()
            .all(|row| row.hull != dray.0),
        "never a programme row"
    );
    assert!(tacsim::port::owned::provenance(rig.tw.world(), dray_order).is_some());
    assert_eq!(
        tacsim::port::coop::helm_of(rig.tw.world(), captain).map(|h| h.hull),
        Some(dray),
        "the register gives the captain her helm"
    );
    // Doc 38a/b F-06: and her title holder commands her (a register hull is no programme row).
    assert_eq!(
        tacsim::port::coop::helm_holder(rig.tw.world(), dray),
        Some(captain),
        "the owner commands the yard-built hull"
    );
    assert_eq!(contract(&rig, dray_order).closed, Some(Close::Collected));
    let hold = VaultId(built.hold.expect("the Dray-P's hold"));

    // ---- (2) The escort's quote and order ----
    let quote = run(&mut rig.tw, captain, "+yard/quote tv_escort");
    assert!(
        quote.contains("3 structural_alloys, 1 industrial_parts") && quote.contains("2 IC days"),
        "{quote}"
    );
    let escort_order = booked(&mut rig, captain, "+yard/order tv_escort");

    // ---- (3) The bill, from the Dray-P's hold, in two deliveries ----
    seed_pool(&mut rig, hold, ALLOYS, 3, LotId(9_710));
    seed_pool(&mut rig, hold, FITTINGS, 1, LotId(9_711));
    let parts = [
        "tacsim.part.weapon.cannon_105",
        "tacsim.part.weapon.mg_coax",
        "tacsim.part.sensor.visual_mk1",
        "tacsim.part.plant.vehicle_mk1",
    ];
    for (i, part) in parts.iter().enumerate() {
        seed_part(
            &mut rig,
            hold,
            part,
            LotId(9_720 + i as u64),
            ComponentCondition::Intact,
            ProvenanceLegalityStatus::CleanTitle,
        );
    }
    for (good, units) in [(ALLOYS, 3), (FITTINGS, 1)] {
        let out = typed_in(
            &mut rig,
            captain,
            &format!("+yard/deliver {escort_order}={good} {units}"),
            1,
        );
        assert!(out.contains("escrow"), "{out}");
    }
    for part in parts {
        let out = typed_in(
            &mut rig,
            captain,
            &format!("+yard/deliver {escort_order}={part} 1"),
            1,
        );
        assert!(out.contains("escrow"), "{out}");
    }
    assert_eq!(
        hold_pool(&mut rig.tw, hold, ALLOYS),
        0,
        "the bulk left the hold"
    );

    // ---- (4) Two IC days, the collect, a commission stopped by staff and resumed ----
    settle(&mut rig, 2);
    days_on(&mut rig, 2);
    let out = run(
        &mut rig.tw,
        captain,
        &format!("+yard/collect {escort_order}"),
    );
    assert!(out.contains("commissions her as SYB-02"), "{out}");
    let refused = pump(&mut rig);
    assert!(refused.is_empty(), "{refused:?}");
    // The works arm settles the consumption on its next fire; staff then run only the lay-up
    // and the placement — the kit is gone, the title not yet written.
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let _ = pump(&mut rig);
    let at = commission(&rig, escort_order);
    assert_eq!(at.step, Step::Laying, "the consumption landed: {at:?}");
    let stopped = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        tacsim::port::commission::advance(ctx, escort_order, 2)
    });
    assert_eq!(stopped, Ok(Step::Titled), "staff stop her before the title");
    let halted = commission(&rig, escort_order);
    let half = halted.object().expect("the Thing is made");
    assert!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .is_some_and(|s| s.snapshot().asset(half).is_none()),
        "no title yet"
    );
    assert!(
        tacsim::port::owned::owned(rig.tw.world(), half).is_none(),
        "no register row yet"
    );
    settle(&mut rig, 2);
    let done = commission(&rig, escort_order);
    assert_eq!(done.step, Step::Done, "resumed and finished: {done:?}");
    let escort = done.object().expect("the escort");
    assert_eq!(titled(&rig, escort), PrincipalId::Object(captain));
    assert_eq!(
        rig.tw.world().get(escort).and_then(|o| o.location),
        Some(floor),
        "on the workshop floor"
    );
    assert_eq!(rounds(&rig, escort), 0, "her magazines are empty (Q-39-36)");
    let provenance =
        tacsim::port::owned::provenance(rig.tw.world(), escort_order).expect("the provenance");
    assert_eq!(provenance.parts.len(), 4, "{provenance:?}");
    assert_eq!(provenance.hull, escort);

    // ---- (5) `+bay` quotes her clean but for the rearm; the rearm loads her from the hold ----
    let name = name_of(&rig, escort);
    let quote = run(&mut rig.tw, captain, &format!("+bay {name}"));
    eprintln!("{quote}");
    assert!(
        quote.contains("rearm cannon (40 rounds")
            && quote.contains("0/2000 rounds")
            && quote.contains("Built on this floor"),
        "clean but for the rearm, her magazines empty: {quote}"
    );
    seed_pool(&mut rig, hold, "ammo_he_105", 40, LotId(9_730));
    seed_pool(&mut rig, hold, "ammo_ball_762", 20, LotId(9_731));
    let out = typed_in(&mut rig, captain, &format!("+bay/order {name}=rearm"), 1);
    eprintln!("{out}");
    days_on(&mut rig, 1);
    assert!(
        rounds(&rig, escort) > 0,
        "the rearm loaded her from the hold"
    );

    // ---- (6) Stowed in the Dray-P the yard built for them ----
    let dray_name = name_of(&rig, dray);
    let out = typed_in(
        &mut rig,
        captain,
        &format!("+bay/stow {name}=#{}", dray.0),
        1,
    );
    assert!(out.contains("is stowed in the"), "{out}");
    let facility = tacsim::port::bay_work::facility_of(rig.tw.world(), escort).expect("a bay");
    assert_eq!(
        facility.carrier,
        Some(dray),
        "{dray_name}'s bay, by the register"
    );

    // ---- (7) The Co-op's escort through an officer ----
    let crewmate = rig.tw.spawn_player("Crewmate");
    teleport(&mut rig.tw, crewmate, floor);
    let coop = port_map(&rig.tw, "ceres_port").orgs[&cid("coop")].org;
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=#{} economy.account.post", captain.0, coop),
    );
    assert!(!granted.contains("Permission denied"), "{granted}");
    let coop_order = booked(
        &mut rig,
        captain,
        &format!("+yard/order tv_escort=org:{coop}"),
    );
    assert_eq!(
        contract(&rig, coop_order).beneficiary,
        PrincipalId::Organization(mush_org::OrgId(coop))
    );
    let escrow = VaultId(coop_order);
    seed_pool(&mut rig, escrow, ALLOYS, 3, LotId(9_740));
    seed_pool(&mut rig, escrow, FITTINGS, 1, LotId(9_741));
    for (i, part) in parts.iter().enumerate() {
        seed_lot(&mut rig, escrow, part, 1, LotId(9_750 + i as u64));
    }
    settle(&mut rig, 2);
    days_on(&mut rig, 2);
    let _ = typed_in(&mut rig, captain, &format!("+yard/collect {coop_order}"), 3);
    let second = commission(&rig, coop_order)
        .object()
        .expect("the second escort");
    assert_eq!(
        titled(&rig, second),
        PrincipalId::Organization(mush_org::OrgId(coop))
    );
    let second_name = name_of(&rig, second);
    let out = typed_in(
        &mut rig,
        captain,
        &format!("+bay/stow {second_name}=#{}", dray.0),
        1,
    );
    assert!(out.contains("is stowed in the"), "{out}");
    let views = tacsim::port::owned::title_views_for(rig.tw.world(), captain);
    assert!(
        views.iter().any(|v| v.tv == second) && views.iter().any(|v| v.tv == escort),
        "the officer reaches both by title: {views:?}"
    );
    assert!(
        tacsim::port::owned::title_views_for(rig.tw.world(), crewmate).is_empty(),
        "a crewmate without office reaches neither"
    );
    let _ = ic_now(&rig.tw);
}

// ---- the ground half --------------------------------------------------------------------------

/// **A free captain**: enrolled for the account, the charter handed straight back, minted, and
/// moved to Stickney's Service Counter (a fixture standing for a passage).
fn free_captain(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    let captain = pilot(rig, who, seed);
    let out = run(&mut rig.tw, captain, "+coop/enrol Long Ox");
    assert!(out.contains("now the Long Ox"), "{out}");
    let out = run(&mut rig.tw, captain, "+coop/release");
    assert!(
        tacsim::port::live_charter(rig.tw.world(), captain).is_none(),
        "the charter is handed back: {out}"
    );
    mint(rig, captain);
    let floor = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, floor);
    captain
}

/// **The Stickney yard** (lane v27's row): the captain on the Service Counter's floor, where a
/// rig the yard builds stands; `+yard` finds the yard in the port.
fn open_yard(rig: &mut Rig) -> ObjId {
    station_room(&rig.tw, STICKNEY, "room.p11_service")
}

/// **One hull built and handed over**: `+yard/order <design>`, its bill laid in the escrow (the
/// fixture), its days stepped, `+yard/collect`; the commission's record, finished.
fn build(
    rig: &mut Rig,
    captain: ObjId,
    design: &str,
    bulk: (u64, u64),
    parts: &[&str],
    lot: u64,
) -> YardCommission {
    let order = booked(rig, captain, &format!("+yard/order {design}"));
    let escrow = VaultId(order);
    seed_pool(rig, escrow, ALLOYS, bulk.0, LotId(lot));
    seed_pool(rig, escrow, FITTINGS, bulk.1, LotId(lot + 1));
    for (i, part) in parts.iter().enumerate() {
        seed_lot(rig, escrow, part, 1, LotId(lot + 2 + i as u64));
    }
    settle(rig, 2);
    let days = contract(rig, order).work_units.div_ceil(24);
    days_on(rig, i64::try_from(days).expect("days"));
    let _ = typed_in(rig, captain, &format!("+yard/collect {order}"), 3);
    let done = commission(rig, order);
    assert_eq!(done.step, Step::Done, "{done:?}");
    done
}

/// The tug's tow on `entry`'s record.
fn towing(rig: &mut Rig, entry: &str, tug: ObjId) -> Option<ObjId> {
    crate::field_host_rig::site_record_at(rig, entry)
        .and_then(|record| record.tows.towing(tug).map(|op| op.casualty()))
}

/// Seconds of ticks until `done`, at most `limit`.
fn seconds_until(rig: &mut Rig, limit: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let _ = crate::field_host_rig::second(rig);
    }
    done(rig)
}

/// `line` as `who`, its one submission landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = crate::driving_rig::command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one submission: {out}");
    crate::driving_rig::land(rig, &queued[0]).unwrap_or_else(|e| panic!("{line}: {e}"));
    out
}

/// Staff destroy `unit`'s plant: a casualty (GC.4's fixture, standing for a fight).
fn powerless(rig: &mut Rig, unit: ObjId) {
    crate::driving_rig::staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:in9-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
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

/// **The yard's hulls on the ground** (the row's second half): the escort the yard built rides
/// the yard-built Dray-P to the site, boards and launches; a second carrier's tug tows her in
/// when she is disabled; and that carrier's programme Dray-P is bought out at Ceres with the
/// titled rig still in her bay.
#[allow(clippy::too_many_lines)]
async fn ground_leg(ground: &crate::grounds_rig::Ground) {
    let mut rig = crate::driving_rig::stocked_tuned(|cfg| {
        crate::field_host_rig::tune(cfg);
        cfg.recovery.inspect_s = 30;
        cfg.recovery.attach_s = 30;
        cfg.recovery.load_s = 60;
    });
    // The second carrier: a chartered Dray-P on the pad with one leased rig.
    let (salvor, salvor_row, salvor_tvs) = crate::driving_rig::on_pad_with_at(
        &mut rig,
        ("Salvor", 2, "Grey Wren"),
        ground.site(),
        |rig, pilot| {
            mint(rig, pilot);
            crate::driving_rig::lease_one(rig, pilot);
        },
    );
    let (salvor_hull, tug) = (ObjId(salvor_row.hull), salvor_tvs[0]);

    // The captain, the yard, the Dray-P and the escort, built at Stickney.
    let captain = free_captain(&mut rig, "Builder", 1);
    let _ = open_yard(&mut rig);
    let dray = build(
        &mut rig,
        captain,
        "dray_p",
        (163, 55),
        &["tacsim.part.plant.hauler_mk1"],
        9_800,
    )
    .object()
    .expect("the Dray-P");
    let escort = build(
        &mut rig,
        captain,
        "tv_escort",
        (3, 1),
        &[
            "tacsim.part.weapon.cannon_105",
            "tacsim.part.weapon.mg_coax",
            "tacsim.part.sensor.visual_mk1",
            "tacsim.part.plant.vehicle_mk1",
        ],
        9_810,
    )
    .object()
    .expect("the escort");
    let out = typed_in(
        &mut rig,
        captain,
        &format!("+bay/stow #{}=#{}", escort.0, dray.0),
        1,
    );
    assert!(out.contains("is stowed in the"), "{out}");

    // Aboard her at Stickney's gallery, and to the site by Ceres.
    let row = tacsim::port::owned::ship_row(rig.tw.world(), dray)
        .cloned()
        .expect("the Dray-P's row");
    crate::driving_rig::load(&mut rig, &row, 80, 120, 9_830);
    let gallery = station_room(&rig.tw, STICKNEY, "room.p01_berth_gallery");
    teleport(&mut rig.tw, captain, gallery);
    let out = run(&mut rig.tw, captain, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
    sail(&mut rig, captain, dray, "+ship/undock ceres", "ceres_port");
    sail(
        &mut rig,
        captain,
        dray,
        &format!("+ship/undock {}", ground.venue),
        ground.entry,
    );
    crate::driving_rig::warm(&mut rig);

    // `+tv/1 board` and `launch` on the ground, by the title view.
    // From the airlock to the hold, then the internal bay access (`escort_lease`'s walk).
    for step in ["inner", "hold"] {
        let _ = run(&mut rig.tw, captain, step);
    }
    let bay = run(&mut rig.tw, captain, "+tv/bay in");
    assert!(bay.contains("entered vehicle bay"), "{bay}");
    let out = run(&mut rig.tw, captain, "+tv/1 board");
    assert!(
        out.contains("You are at rig 1's"),
        "the title holder boards: {out}"
    );
    let out = run(&mut rig.tw, captain, "+tv/1 leave");
    assert!(out.contains("give back your rig billet"), "{out}");
    let _ = crate::driving_rig::act_go(&mut rig, captain, "+tv/1 launch go");
    let _ = crate::driving_rig::finish(&mut rig, escort, captain);
    assert!(
        matches!(
            crate::driving_rig::parent(&rig.tw, escort),
            mush_sim::data::SpatialParent::Direct { .. }
        ),
        "the escort stands on {}'s ground",
        ground.entry
    );

    // Disabled and left on the index (GC.6's staff fixture, standing for a fight); the second
    // carrier's tug takes her in tow and loads her into its own bay.
    powerless(&mut rig, escort);
    let escort_cell = mush_sim::pose_of(rig.tw.world(), escort)
        .map(|(_, at)| (at.x_cm.div_euclid(1_000), at.y_cm.div_euclid(1_000)))
        .expect("the escort's cell");
    let s = crate::site_record_rig::staff_member(&mut rig);
    crate::site_record_rig::staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {} leave #{} casualty {},{}",
            ground.entry, escort.0, escort_cell.0, escort_cell.1
        ),
    );
    let _ = crate::driving_rig::act_go(&mut rig, salvor, "+tv/1 launch go");
    let _ = crate::driving_rig::finish(&mut rig, tug, salvor);
    let at = crate::driving_rig::ramp_at(&rig.tw, ground.entry);
    crate::driving_rig::to_cell_on(
        &mut rig,
        ground.entry,
        tug,
        (escort_cell.0 + 1, escort_cell.1),
    );
    let out = go(&mut rig, salvor, &format!("+tv/1 tow #{} go", escort.0));
    assert!(
        seconds_until(&mut rig, 120, |rig| towing(rig, ground.entry, tug)
            == Some(escort)),
        "the tow attaches: {out}"
    );
    crate::driving_rig::to_cell_on(
        &mut rig,
        ground.entry,
        tug,
        (at.0.div_euclid(1_000), at.1.div_euclid(1_000)),
    );
    let out = go(&mut rig, salvor, "+tv/1 load go");
    let in_bay = |rig: &mut Rig| {
        matches!(
            crate::driving_rig::parent(&rig.tw, escort),
            mush_sim::data::SpatialParent::Interior { .. }
        )
    };
    assert!(
        seconds_until(&mut rig, 180, in_bay),
        "the load lands: {out}"
    );
    let carrier = tacsim::port::bay_work::facility_of(rig.tw.world(), escort)
        .ok()
        .and_then(|f| f.carrier);
    eprintln!(
        "recovered into {carrier:?} (salvor #{}, dray #{})",
        salvor_hull.0, dray.0
    );
    assert_eq!(
        carrier,
        Some(salvor_hull),
        "the second carrier recovered her"
    );

    // The programme Dray-P sails home and is bought out with the titled rig in her bay.
    let _ = go(&mut rig, salvor, "+tv/1 recover go");
    let _ = crate::driving_rig::finish(&mut rig, tug, salvor);
    sail(
        &mut rig,
        salvor,
        salvor_hull,
        "+ship/undock ceres",
        "ceres_port",
    );
    let out = run(&mut rig.tw, salvor, "+ship/leave");
    assert!(out.contains("cycle the lock out"), "{out}");
    // Her own leased tug goes back to Occator; the titled escort stays in her bay.
    crate::driving_rig::at_counter(&mut rig, salvor);
    let out = run(&mut rig.tw, salvor, "+tv/lease return go");
    assert!(out.contains("back across Occator's counter"), "{out}");
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, salvor, desk);
    mint(&mut rig, salvor);
    let out = run(&mut rig.tw, salvor, "+coop/buyout");
    assert!(
        !out.contains("is still in the") && out.contains("writes the title transfer"),
        "the titled rig in her bay does not refuse the buyout: {out}"
    );
    assert!(
        matches!(
            crate::driving_rig::parent(&rig.tw, escort),
            mush_sim::data::SpatialParent::Interior { .. }
        ),
        "the escort still rides her bay"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in9_ahuna_the_yard_built_escort_boards_launches_and_is_recovered() {
    ground_leg(&crate::grounds_rig::AHUNA_FLANK).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in9_urvara_the_yard_built_escort_boards_launches_and_is_recovered() {
    ground_leg(&crate::grounds_rig::URVARA_REACH).await;
}

/// **Doc 39 cut IN.9b — the class templates at the yard**: a line tank and a scout car, simulator
/// designs that author no cab, are built at Stickney by their templates (`armour_cab`) and stand
/// on the floor with a cab and a kit locker. Each keeps its field-ops mass: the line tank's
/// 29,900 kg is too much for a Dray-P's 20 t rig slot, and `+bay/stow` says so; the scout's
/// 15,900 kg fits, and she rides the bay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in9b_the_yard_builds_a_line_tank_and_a_scout_by_their_templates() {
    let mut rig = stocked();
    let captain = free_captain(&mut rig, "Builder", 1);
    let floor = open_yard(&mut rig);
    let dray = build(
        &mut rig,
        captain,
        "dray_p",
        (163, 55),
        &["tacsim.part.plant.hauler_mk1"],
        9_900,
    )
    .object()
    .expect("the Dray-P");
    let tank = build(
        &mut rig,
        captain,
        "line_tank",
        (20, 7),
        &[
            "tacsim.part.weapon.cannon_105",
            "tacsim.part.weapon.mg_coax",
            "tacsim.part.sensor.visual_mk1",
            "tacsim.part.plant.vehicle_mk1",
        ],
        9_910,
    );
    let scout = build(
        &mut rig,
        captain,
        "scout_mk1",
        (12, 4),
        &[
            "tacsim.part.weapon.autocannon_30",
            "tacsim.part.weapon.mg_coax",
            "tacsim.part.sensor.visual_mk1",
            "tacsim.part.plant.light_mk1",
        ],
        9_920,
    );
    for (built, design) in [(&tank, "line_tank"), (&scout, "scout_mk1")] {
        let obj = built.object().expect("a hull");
        let row = tacsim::port::owned::rig_row(rig.tw.world(), obj)
            .unwrap_or_else(|| panic!("{design} is a register rig"));
        assert!(
            row.rooms.contains_key(&cid("cab")) && row.rooms.contains_key(&cid("locker")),
            "{design} has the template's cab and locker: {row:?}"
        );
        assert_eq!(
            rig.tw.world().get(obj).and_then(|o| o.location),
            Some(floor),
            "{design} stands on the workshop floor"
        );
        assert_eq!(titled(&rig, obj), PrincipalId::Object(captain));
    }
    // The templated designs keep their field-ops mass.
    let mass = |design: &str| {
        let store = store(&rig.tw);
        let spec = &store.definitions().specs[&cid(&format!("tacsim.unit.{design}"))];
        assert!(spec.logistics.as_ref().is_some_and(|l| l.derived_mass));
        tacsim::field_ops::dry_mass_kg_for_spec(spec)
    };
    assert_eq!((mass("line_tank"), mass("scout_mk1")), (29_900, 15_900));

    // The Dray-P's 20 t slot refuses the line tank and takes the scout.
    let tank_obj = tank.object().expect("the tank");
    let refused = run(
        &mut rig.tw,
        captain,
        &format!("+bay/stow #{}=#{}", tank_obj.0, dray.0),
    );
    assert!(
        refused.contains("she masses 30 t and the slot takes 20 t"),
        "the line tank is too heavy for the rig slot: {refused}"
    );
    let scout_obj = scout.object().expect("the scout");
    let out = typed_in(
        &mut rig,
        captain,
        &format!("+bay/stow #{}=#{}", scout_obj.0, dray.0),
        1,
    );
    assert!(out.contains("is stowed in the"), "{out}");
    let facility = tacsim::port::bay_work::facility_of(rig.tw.world(), scout_obj).expect("her bay");
    assert_eq!(facility.carrier, Some(dray));
}

// ---- the review fixes (docs 38a/38b, 2026-11-05) ----------------------------------------------

/// The review's captain at the yard: enrolled, released, minted, on the Service Counter's floor.
fn review_captain(rig: &mut Rig) -> ObjId {
    free_captain(rig, "ReviewBuilder", 71)
}

/// **A yard order is one hull** (doc 38a #3) and **its quote names the parts** (doc 38b
/// F-18(a)): `+yard/quote` and `+yard/order` refuse a quantity of two (one kit commissions one
/// hull), and book nothing; the quote of one lists the design's four part lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_the_yard_takes_one_hull_an_order_and_quotes_her_parts() {
    let mut rig = stocked();
    let captain = review_captain(&mut rig);
    let before = contracts(&rig).len();
    let quote = run(&mut rig.tw, captain, "+yard/quote tv_escort 2");
    assert!(
        quote.contains(tacsim::port::works::YARD_ONE_HULL),
        "{quote}"
    );
    let order = run(&mut rig.tw, captain, "+yard/order tv_escort 2");
    assert!(
        order.contains(tacsim::port::works::YARD_ONE_HULL),
        "{order}"
    );
    settle(&mut rig, 2);
    assert_eq!(contracts(&rig).len(), before, "nothing is booked");
    let quote = run(&mut rig.tw, captain, "+yard/quote tv_escort");
    for part in [
        "1 tacsim.part.weapon.cannon_105",
        "1 tacsim.part.weapon.mg_coax",
        "1 tacsim.part.sensor.visual_mk1",
        "1 tacsim.part.plant.vehicle_mk1",
    ] {
        assert!(quote.contains(part), "the quote lists {part}: {quote}");
    }
}

/// **Deliver takes only clean, whole lots** (doc 38b F-07) and **a cancel returns them** (doc
/// 38a #4): from the Dray-P's hold, a sight with no provenance and a damaged one are refused and
/// stay (a hot lot is refused by the same filter; only a stripped part can be hot); the clean
/// sight goes into the escrow; `+yard/cancel` hands that very lot back to the hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_the_yard_refuses_untraced_or_damaged_parts_and_a_cancel_returns_the_lot() {
    let mut rig = stocked();
    let captain = review_captain(&mut rig);
    let dray = build(
        &mut rig,
        captain,
        "dray_p",
        (163, 55),
        &["tacsim.part.plant.hauler_mk1"],
        98_000,
    );
    let hold = VaultId(dray.hold.expect("the Dray-P's hold"));
    let order = booked(&mut rig, captain, "+yard/order tv_escort");
    let good = "tacsim.part.sensor.visual_mk1";
    let clean = ProvenanceLegalityStatus::CleanTitle;
    let deliver = format!("+yard/deliver {order}={good} 1");
    for (lot, damaged) in [(98_101, false), (98_102, true)] {
        if damaged {
            let condition = ComponentCondition::Damaged;
            seed_part(&mut rig, hold, good, LotId(lot), condition, clean.clone());
        } else {
            // No provenance row: an untraced lot.
            seed_lot(&mut rig, hold, good, 1, LotId(lot));
        }
        settle(&mut rig, 1);
        let out = run(&mut rig.tw, captain, &deliver);
        assert!(out.contains("fit to build with"), "{out}");
        settle(&mut rig, 2);
    }
    let clean_lot = LotId(98_103);
    seed_part(
        &mut rig,
        hold,
        good,
        clean_lot,
        ComponentCondition::Intact,
        clean,
    );
    settle(&mut rig, 1);
    let out = typed_in(&mut rig, captain, &deliver, 2);
    assert!(out.contains("moves into order"), "{out}");
    let at = |rig: &mut Rig, lot: LotId| {
        let inventory = rig
            .tw
            .world()
            .module_state::<tacsim::port::bay_work::act::BayActHandles>()
            .and_then(|h| h.inventory.clone())
            .expect("inventory");
        crate::driving_rig::typed(&mut rig.tw, &inventory.lots, &lot)
            .expect("the lot")
            .endpoint()
    };
    assert_eq!(
        at(&mut rig, clean_lot),
        mush_inventory::record::EndpointRef(VaultId(order))
    );
    for lot in [98_101, 98_102] {
        assert_eq!(
            at(&mut rig, LotId(lot)),
            mush_inventory::record::EndpointRef(hold),
            "a refused lot stays in the hold"
        );
    }
    let _ = typed_in(&mut rig, captain, &format!("+yard/cancel {order}"), 4);
    assert_eq!(contract(&rig, order).closed, Some(Close::Cancelled));
    assert_eq!(
        at(&mut rig, clean_lot),
        mush_inventory::record::EndpointRef(hold),
        "the cancel hands back the very lot: {:?}",
        contract(&rig, order)
    );
}

/// **An unpaid storage seizure takes the kit** (doc 38b F-17): an escort built and left
/// uncollected, the captain's account emptied by a staff hand (the seed), and 28 IC days of grace
/// past: the yard takes the goods, the kit lot among them, and the contract closes `Seized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_an_unpaid_storage_seizure_takes_the_hull_kit() {
    let mut rig = stocked();
    let captain = review_captain(&mut rig);
    let order = booked(&mut rig, captain, "+yard/order tv_escort");
    let escrow = VaultId(order);
    seed_pool(&mut rig, escrow, ALLOYS, 3, LotId(98_200));
    seed_pool(&mut rig, escrow, FITTINGS, 1, LotId(98_201));
    for (i, part) in [
        "tacsim.part.weapon.cannon_105",
        "tacsim.part.weapon.mg_coax",
        "tacsim.part.sensor.visual_mk1",
        "tacsim.part.plant.vehicle_mk1",
    ]
    .iter()
    .enumerate()
    {
        seed_part(
            &mut rig,
            escrow,
            part,
            LotId(98_202 + i as u64),
            ComponentCondition::Intact,
            ProvenanceLegalityStatus::CleanTitle,
        );
    }
    settle(&mut rig, 2);
    days_on(&mut rig, 3);
    assert!(
        contract(&rig, order).delivered_ic.is_some(),
        "the kit stands in the escrow: {:?}",
        contract(&rig, order)
    );
    // The staff hand: the captain's whole balance to the treasury.
    let account = tacsim::port::credit::own_account(rig.tw.world(), captain).expect("an account");
    let spend = account.posted_balance.minor() - account.held_amount.minor();
    let from = account.id.0;
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let paid = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay #{from}=#{treasury} {spend}"),
    );
    let account = tacsim::port::credit::own_account(rig.tw.world(), captain).expect("an account");
    assert_eq!(
        account.posted_balance.minor() - account.held_amount.minor(),
        0,
        "the account is empty: {paid}"
    );
    days_on(&mut rig, 30);
    let mut refusals = Vec::new();
    for _ in 0..6 {
        refusals.extend(fire_all(&mut rig));
    }
    assert_eq!(
        contract(&rig, order).closed,
        Some(Close::Seized),
        "the seizure lands: {refusals:?}; {:?}",
        contract(&rig, order)
    );
}
