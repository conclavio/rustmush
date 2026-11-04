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
//! **Seeds** (doc 43 rule 2.3 #4): the yard facility (`@facility/open`, `/owner`, `/terms`: the
//! lane's `yard` row is lane v24's, after IN.5's v23); the Dray-P's bill in its escrow; the
//! escort's bill and the munitions in the Dray-P's hold; the money; the officer's grant; the
//! captain's passage to Stickney (a move). Every claim is reached by the players' commands and
//! the port tick.

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_org::PrincipalId;
use tacsim::port::commission::{Step, YardCommission};
use tacsim::port::works::{Close, WorksContract, WorksState};

use crate::driving_rig::{
    Rig, cid, hold_pool, ic_now, pilot, port_map, run, seed_lot, seed_pool, station_room, stocked,
    store, teleport,
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

    // ---- The seed: the Stickney yard, the Yards', Public, a 10M slipway over 30 years ----
    let yard = made(&mut rig, "Stickney slipway");
    rig.tw
        .world_mut()
        .move_to(yard, floor)
        .expect("the yard onto the floor");
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
        format!("@facility/open {}=2 tacsim.yard", yard.0),
        format!("@facility/owner {}=org:{}", yard.0, yards.0),
        format!(
            "@facility/terms {}=public 10000000/30/20/12/150/250",
            yard.0
        ),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Usage"), "{line}: {out}");
    }

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
        seed_lot(&mut rig, hold, part, 1, LotId(9_720 + i as u64));
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
