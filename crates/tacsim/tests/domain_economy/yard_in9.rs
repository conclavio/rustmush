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
//!    fittings, the four parts and, doc 48 unit PL.7, her line core's module) and two days;
//!    `+yard/order tv_escort` books it.
//! 3. `+yard/deliver` brings the bill from the Dray-P's hold in two deliveries — the bulk, then
//!    the five part lots.
//! 4. Two IC days later `+yard/collect` writes the commission, and **staff stop it** after its
//!    first steps (between the kit's consumption and the title); the next port fire resumes it and
//!    it finishes: the escort stands on the workshop floor titled to them, berthed with no
//!    carrier, its magazines empty, the kit and parts consumed, the provenance naming them.
//!    Doc 48 unit PL.7: the core module is consumed with them, and her core is the governed
//!    Class II core her design carries: no die was drawn for it.
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
/// The line core's module (doc 48 unit PL.7): what the escort and the line tanks bill for their
/// stock Class II core.
const LINE_MODULE: &str = "tacsim.part.core.line";

/// Where `lot` stands, or `None` once it is consumed.
fn lot_at(rig: &mut Rig, lot: LotId) -> Option<mush_inventory::record::EndpointRef> {
    let inventory = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .expect("inventory");
    crate::driving_rig::typed(&mut rig.tw, &inventory.lots, &lot).map(|l| l.endpoint())
}

/// **The yard's stock core** (doc 48 unit PL.7, Q-48-48, the owner's call): `hull` carries the
/// governed Class II core her design names, and it is no titled object, so no die was drawn and
/// no core title was born for it.
fn assert_stock_core(rig: &Rig, hull: ObjId) {
    let unit = mush_tactics::module::unit_of(rig.tw.world(), hull).expect("the unit");
    let core = tacsim::arena::core::read_core(unit).expect("a core");
    assert_eq!(
        (core.class, core.governed),
        (tacsim::arena::core::CoreClass::II, true),
        "a governed Class II core"
    );
    let economy = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy");
    let held = rig
        .tw
        .world()
        .get(hull)
        .map(|o| o.contents.clone())
        .unwrap_or_default();
    assert!(
        held.iter()
            .all(|thing| economy.core_title_birth(*thing).ok().flatten().is_none()),
        "no core title rides in her"
    );
}

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

/// **The escort's load on `component`, read off the content** (doc 49 cut CC.0, Q-49-16):
/// the `count_per_load` of the round the shipped `tv_escort`'s mount there fires, so a retune
/// of the round's pack moves no line here.
fn escort_load(component: &str) -> i64 {
    use tacsim::definitions::unit::ComponentRole;
    let catalog = tacsim::builtin_definitions()
        .unwrap_or_else(|error| panic!("the shipped content binds: {error}"));
    let key = mush_core::CanonicalId::new(tacsim::definitions::content::TV_ESCORT)
        .unwrap_or_else(|_| panic!("{} is an id", tacsim::definitions::content::TV_ESCORT));
    let spec = catalog
        .specs
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is fielded"));
    spec.components
        .iter()
        .find_map(|slot| match &slot.role {
            ComponentRole::Weapon { weapon, .. } if slot.id.as_str() == component => {
                catalog.weapons.get(weapon)
            }
            _ => None,
        })
        .and_then(|def| catalog.ammunition.get(&def.ammunition))
        .map(|round| i64::from(round.count_per_load))
        .unwrap_or_else(|| panic!("{key} carries a loaded {component}"))
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
    let out = run(&mut rig.tw, captain, "+coop/enroll Long Ox");
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
    // 48e review rounds:G7 (Q-39-36): a space hull's magazines are empty too; her magazine
    // record holds each at 0, where its absence read every one full.
    let mags = tacsim::port::bay_work::magazines::magazines(
        mush_tactics::module::unit_of(rig.tw.world(), dray).expect("her unit"),
    );
    assert!(!mags.is_empty(), "the Dray-P carries magazines");
    assert!(
        mags.iter().all(|magazine| magazine.left == 0),
        "her magazines are empty (Q-39-36): {mags:?}"
    );
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
        "never a program row"
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
    // Doc 48 unit PL.7 (Q-48-48): her stock core bills its module.
    assert!(quote.contains(&format!("1 {LINE_MODULE}")), "{quote}");
    let escort_order = booked(&mut rig, captain, "+yard/order tv_escort");

    // ---- (3) The bill, from the Dray-P's hold, in two deliveries ----
    seed_pool(&mut rig, hold, ALLOYS, 3, LotId(9_710));
    seed_pool(&mut rig, hold, FITTINGS, 1, LotId(9_711));
    let parts = [
        "tacsim.part.weapon.cannon_105",
        "tacsim.part.weapon.mg_coax",
        "tacsim.part.sensor.visual_mk1",
        "tacsim.part.plant.vehicle_mk1",
        LINE_MODULE,
    ];
    let module_lot = LotId(9_724);
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
    assert_eq!(
        lot_at(&mut rig, module_lot),
        Some(mush_inventory::record::EndpointRef(VaultId(escort_order))),
        "the core module is in the escrow"
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
    assert_eq!(provenance.parts.len(), 5, "{provenance:?}");
    assert_eq!(provenance.hull, escort);
    // Doc 48 unit PL.7 (Q-48-48): the core module was consumed with the kit and the parts, and
    // her core is the governed Class II core her design carries, outside the die quota.
    assert!(provenance.parts.contains(&module_lot), "{provenance:?}");
    assert_eq!(
        lot_at(&mut rig, module_lot),
        None,
        "the core module is consumed"
    );
    assert_stock_core(&rig, escort);

    // ---- (5) `+bay` quotes her clean but for the rearm; the rearm loads her from the hold ----
    let name = name_of(&rig, escort);
    let quote = run(&mut rig.tw, captain, &format!("+bay {name}"));
    eprintln!("{quote}");
    // A whole load to rearm on the cannon, and the coaxial empty, at the content's loads.
    let (cannon_load, coaxial_load) = (escort_load("cannon"), escort_load("machine_gun"));
    assert!(
        quote.contains(&format!("rearm cannon ({cannon_load} rounds"))
            && quote.contains(&format!("0/{coaxial_load} rounds"))
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
    let out = run(&mut rig.tw, captain, "+coop/enroll Long Ox");
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
            LINE_MODULE,
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
            LINE_MODULE,
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
            "tacsim.part.core.doctrine",
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
    // Doc 48 unit PL.7: each consumed its core module, and the line tank carries her governed
    // Class II core outside the die quota.
    // `build` lays the parts from `lot + 2`, the module fifth.
    for (design, module) in [("line_tank", 9_916), ("scout_mk1", 9_926)] {
        assert_eq!(
            lot_at(&mut rig, LotId(module)),
            None,
            "the {design}'s module is consumed"
        );
    }
    assert_stock_core(&rig, tank.object().expect("the tank"));
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

/// **Doc 49 §4a (from the second review, doc 48b): the yard leaves a laser's beam charge
/// full.** Q-39-36 empties a commissioned hull's magazines, and beam charge is a cell, not a
/// magazine: no munitions good loads it. A line tank (laser) built at Stickney stands on the
/// floor with her coaxial gun's magazine empty and her laser's charge full; the commission's
/// Emptied step used to zero both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in9_a_yard_built_laser_tank_keeps_her_beam_charge() {
    let mut rig = stocked();
    let captain = free_captain(&mut rig, "LaserBuilder", 2);
    let tank = build(
        &mut rig,
        captain,
        "line_tank_laser",
        (21, 7),
        &[
            // Doc 49 unit CC.3 (Q-49-6): the 105 beside the laser.
            "tacsim.part.weapon.cannon_105",
            "tacsim.part.weapon.laser_mk1",
            "tacsim.part.weapon.mg_coax",
            "tacsim.part.sensor.visual_mk1",
            "tacsim.part.plant.vehicle_mk1",
            LINE_MODULE,
        ],
        9_930,
    )
    .object()
    .expect("the tank");
    let ammo = |local: &str| {
        mush_tactics::module::unit_of(rig.tw.world(), tank)
            .expect("the unit")
            .cells()
            .find(|(addr, _)| {
                matches!(addr, mush_tactics::CellAddr::Component { component, cell }
                    if cell.as_str() == "ammo"
                        && component.as_str().rsplit('.').next() == Some(local))
            })
            .map(|(_, cell)| (cell.current, cell.max))
            .unwrap_or_else(|| panic!("no ammo cell on the {local}"))
    };
    let (charge, full) = ammo("laser");
    assert!(
        full > 0 && charge == full,
        "the laser's beam charge is full: {charge} of {full}"
    );
    assert_eq!(
        ammo("machine_gun").0,
        0,
        "the coaxial gun's magazine is empty"
    );
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
        "1 tacsim.part.core.line",
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
        LINE_MODULE,
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

// ---- doc 48 unit PL.9: facilities as titled assets --------------------------------------------

/// The yard's price in the PL.9 stories, minor (50,000.00cr).
const YARD_PRICE: i64 = 5_000_000;

/// The Stickney yard's room, which is the facility `+yard` books at.
fn yard_room(rig: &Rig) -> ObjId {
    station_room(&rig.tw, STICKNEY, "room.p12_observation")
}

fn org_of(rig: &Rig, entry: &str, org: &str) -> mush_org::OrgId {
    mush_org::OrgId(port_map(&rig.tw, entry).orgs[&cid(org)].org)
}

fn facility_of(rig: &Rig, object: ObjId) -> mush_industry::FacilityRecord {
    rig.tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.facility(object).cloned())
        .expect("the facility")
}

fn posted(rig: &Rig, account: mush_economy::AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account")
}

fn credit_of(rig: &Rig, who: ObjId) -> mush_economy::AccountId {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .map(|a| a.id)
        .expect("a credit account")
}

/// The `org`'s officer, made once: a player granted `economy.account.post` on the org, so they
/// act for it.
fn officer_of(rig: &mut Rig, entry: &str, org: &str) -> ObjId {
    let name = format!("Officer{}", org.replace('_', ""));
    if let Some(who) = rig.tw.world().resolve_player(&name) {
        return who;
    }
    let who = rig.tw.spawn_player(&name);
    let id = org_of(rig, entry, org);
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=#{} economy.account.post", who.0, id.0),
    );
    assert!(
        mush_economy::acts_for(rig.tw.world(), who, &PrincipalId::Organization(id)),
        "{granted}"
    );
    who
}

/// `who` lists `object` at the venue's exchange with `+market/post … as=<org>`: the listing, or
/// what the command said.
fn post_as(
    rig: &mut Rig,
    who: ObjId,
    (entry, org, venue): (&str, &str, &str),
    object: ObjId,
    price: i64,
) -> Result<mush_market::ListingId, String> {
    let id = org_of(rig, entry, org);
    let exchange = if venue == "ceres" {
        station_room(&rig.tw, "ceres_port", "room.c07_exchange")
    } else {
        station_room(&rig.tw, STICKNEY, "room.p08_exchange")
    };
    teleport(&mut rig.tw, who, exchange);
    let out = run(
        &mut rig.tw,
        who,
        &format!(
            "+market/post #{} price={price} market={venue} days=3 as=#{}",
            object.0, id.0
        ),
    );
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.live_listings()
                .filter(|l| l.object == Some(object))
                .map(|l| l.id)
                .max()
        })
        .ok_or(out)
}

/// The `org`'s officer lists `object` for it.
fn list_for(
    rig: &mut Rig,
    seller: (&str, &str, &str),
    object: ObjId,
    price: i64,
) -> Result<mush_market::ListingId, String> {
    let officer = officer_of(rig, seller.0, seller.1);
    post_as(rig, officer, seller, object, price)
}

/// `who` quotes and buys `listing` at Stickney's exchange; the economy's sweep settles it.
fn buy_listing(rig: &mut Rig, who: ObjId, listing: mush_market::ListingId) {
    let exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, who, exchange);
    let quoted = run(&mut rig.tw, who, &format!("+market/quote {listing}=1"));
    let quote = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.quotes_of_buyer(&PrincipalId::Object(who))
                .map(|q| q.id)
                .max()
        })
        .unwrap_or_else(|| panic!("no quote: {quoted}"));
    let accepted = run(&mut rig.tw, who, &format!("+market/accept {quote}"));
    assert!(!accepted.contains("refused"), "{accepted}");
    crate::driving_rig::sweep(&mut rig.tw, 31);
}

/// **A lane bump at `margin` per mille** over the Stickney yard and the Ceres works: the shipped
/// lane, both rows' margins changed and the pack re-signed, run through the real applier's
/// check and update for both ports, as a newer lane version applies.
fn bump_lane(rig: &mut Rig, margin: u32) {
    use mush_content::EntryApplier;
    let mut doc: serde_json::Value =
        serde_json::from_str(tacsim::definitions::content::LANE_JSON).expect("the lane");
    for (entry, block) in [("stickney_port", "yard"), ("ceres_port", "works")] {
        doc["entries"][entry]["def"][block]["cost"]["margin_permille"] = serde_json::json!(margin);
    }
    doc["manifest"]["digest"] = serde_json::json!("0".repeat(64));
    let parsed = mush_content::parse_pack(&doc.to_string()).expect("the bumped lane parses");
    doc["manifest"]["digest"] =
        serde_json::json!(parsed.computed_digest().expect("a digest").to_hex());
    let pack = mush_content::load_pack(&doc.to_string()).expect("the bumped lane verifies");
    let placed = rig
        .tw
        .world()
        .module_state::<mush_content::ContentState>()
        .and_then(|state| {
            state
                .latches
                .applied(&cid(tacsim::definitions::content::LANE_PACK))
        })
        .map(mush_content::Applied::placed)
        .expect("the lane is applied")
        .clone();
    for entry in ["stickney_port", "ceres_port"] {
        let id = cid(entry);
        let raw = pack.entry(&id).cloned().expect("a port entry");
        let applied = crate::support::with_ctx(&mut rig.tw, |ctx| {
            EntryApplier::check(&tacsim::port::apply::PortApplier, ctx, &pack, &id, &raw)?;
            EntryApplier::update(
                &tacsim::port::apply::PortApplier,
                ctx,
                &pack,
                &id,
                &raw,
                None,
                &placed,
            )
        });
        assert!(applied.is_ok(), "{entry}: {applied:?}");
    }
}

/// The work order a contract drafted, read.
fn order_state(rig: &mut Rig, id: u64) -> mush_industry::workorder::WorkOrderState {
    let order = contract(rig, id).order.expect("the draft landed");
    let families = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    *crate::driving_rig::typed(
        &mut rig.tw,
        &families.work_orders,
        &mush_industry::workorder::WorkOrderId(order),
    )
    .expect("the work order")
    .state()
}

/// A wrecker's bill laid in `escrow` (the fixture: staff stand in for the deliveries).
fn lay_wrecker_bill(rig: &mut Rig, escrow: VaultId, lot: u64) {
    seed_pool(rig, escrow, ALLOYS, 9, LotId(lot));
    seed_pool(rig, escrow, FITTINGS, 3, LotId(lot + 1));
    for (i, part) in [
        "tacsim.part.plant.light_mk1",
        "tacsim.part.sensor.visual_mk1",
    ]
    .iter()
    .enumerate()
    {
        seed_lot(rig, escrow, part, 1, LotId(lot + 2 + i as u64));
    }
}

/// **Doc 48 unit PL.9 (Q-48-26, -37, -45, -53): a captain buys the Stickney yard, prices it, and
/// keeps it across a lane bump; a margin raised after a booking cancels that order.**
/// 1. The lane titled the yard to Stickney Yards. The captain, who does not act for the Yards,
///    cannot list it for them (`NOT_FOR_ORG`). The Yards' officer lists it, idle, for 50,000cr
///    with `+market/post … as=`, and the captain quotes and buys it at the exchange. The sweep pays the Yards, passes the title,
///    and passes the facility: its owner is the captain, owner-held and kept public (a sold
///    yard), and its cost model is the authored one.
/// 2. The captain sets its margin to 400 per mille. Its doors stay open (`TERMS_KEPT_PUBLIC`),
///    and its cost model is staff's (`TERMS_COST_STAFF`).
/// 3. A lane bump at another margin re-terms the Ceres works (still authored) and leaves the
///    captain's yard, owner and margin, as they were.
/// 4. A customer books a wrecker. The owner raises the margin to 600, the bill is laid in the
///    escrow, and the arm finds the fee risen past the booked one: the yard cancels the order,
///    and the booking fee comes back. Booked again and the margin lowered to 100, the order runs
///    at the lower fee.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it() {
    let mut rig = stocked();
    let yard = yard_room(&rig);
    let yards = org_of(&rig, STICKNEY, "stickney_yards");
    assert_eq!(titled(&rig, yard), PrincipalId::Organization(yards));
    let authored = facility_of(&rig, yard);
    assert_eq!(authored.holding(), mush_industry::Holding::Authored);
    let owner = free_captain(&mut rig, "YardOwner", 81);
    let owner_account = credit_of(&rig, owner);
    let yards_account = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")]
        .account
        .map(mush_economy::AccountId)
        .expect("the Yards' account");
    let (owner_before, yards_before) = (posted(&rig, owner_account), posted(&rig, yards_account));

    // The yard's stores, as the works arm opens them on first need: empty, the Yards'.
    let root = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")].root_scope;
    let opened = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create YardStores=container:#{} owner=org:{} scope={root}",
            yard.0, yards.0
        ),
    );
    let stores = tacsim::port::facility_sale::stores_of(rig.tw.world(), yard)
        .unwrap_or_else(|| panic!("the yard's stores: {opened}"));

    // ---- (1) Listed by the Yards' officer, bought by the captain ----
    let outsider = post_as(
        &mut rig,
        owner,
        (STICKNEY, "stickney_yards", "stickney"),
        yard,
        YARD_PRICE,
    )
    .expect_err("the captain does not act for the Yards");
    assert!(
        outsider.contains(mush_market::seller::NOT_FOR_ORG),
        "{outsider}"
    );
    assert!(!mush_market::asset_on_sale(rig.tw.world(), yard));
    // `as=` is a listing's alone: a bid that names one is refused with the bid's usage, where the
    // same bid without it is not.
    let bid_line = format!("+market/bid tacsim:iron qty=1 price=1 into={}", stores.0);
    let plain = run(&mut rig.tw, owner, &bid_line);
    assert!(!plain.contains(mush_market::seller::BID_USAGE), "{plain}");
    let named = run(&mut rig.tw, owner, &format!("{bid_line} as=#{}", yards.0));
    assert!(named.contains(mush_market::seller::BID_USAGE), "{named}");
    let listing = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        yard,
        YARD_PRICE,
    )
    .expect("the idle yard lists");
    buy_listing(&mut rig, owner, listing);
    assert_eq!(titled(&rig, yard), PrincipalId::Object(owner), "the title");
    let sold = facility_of(&rig, yard);
    assert_eq!(sold.owner(), &PrincipalId::Object(owner), "the facility");
    assert_eq!(sold.holding(), mush_industry::Holding::OwnerPublic);
    assert_eq!(sold.customers(), mush_industry::Customers::Public);
    assert_eq!(
        sold.cost(),
        authored.cost(),
        "the authored cost model goes with it"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().vault(stores).map(|v| v.owner.clone())),
        Some(PrincipalId::Object(owner)),
        "its stores went with it"
    );
    // Doc 48 cut EX.4 (Q-48-8): the Yards are Stickney's operator, so the listing fee and the
    // 3% cut come back to them; only the 1% levy leaves for the system sink.
    assert_eq!(
        posted(&rig, yards_account) - yards_before,
        YARD_PRICE - YARD_PRICE * mush_market::fees::LEVY_PERMILLE / 1_000
    );
    assert_eq!(owner_before - posted(&rig, owner_account), YARD_PRICE);

    // ---- (2) The owner's terms ----
    let set = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=public margin=400", yard.0),
    );
    assert!(set.contains("margin 400 per mille"), "{set}");
    let closed = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=owner", yard.0),
    );
    assert!(
        closed.contains(mush_industry::commands::TERMS_KEPT_PUBLIC),
        "{closed}"
    );
    let cost = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=public 1/1/0/0/0/0", yard.0),
    );
    assert!(
        cost.contains(mush_industry::commands::TERMS_COST_STAFF),
        "{cost}"
    );
    let priced = facility_of(&rig, yard);
    let mut expected = authored.cost().copied().expect("a priced yard");
    expected.margin_permille = 400;
    assert_eq!(
        priced.cost().copied(),
        Some(expected),
        "only the margin moved"
    );

    // ---- (3) A lane bump keeps the owner's terms ----
    bump_lane(&mut rig, 300);
    assert_eq!(
        facility_of(&rig, yard),
        priced,
        "the owner's yard, as it was"
    );
    let ceres = station_room(&rig.tw, "ceres_port", "room.c15_maintenance");
    assert_eq!(
        facility_of(&rig, ceres).cost().map(|c| c.margin_permille),
        Some(300),
        "an authored works re-termed"
    );

    // ---- (4) The fee pinned at booking ----
    let customer = free_captain(&mut rig, "Customer", 82);
    let customer_account = credit_of(&rig, customer);
    let before = posted(&rig, customer_account);
    let held_before = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(customer_account)
                .map(|a| a.held_amount.minor())
        })
        .expect("the account");
    let raised = booked(&mut rig, customer, "+yard/order tv_wrecker");
    assert!(contract(&rig, raised).fee_minor.is_some(), "the fee pinned");
    let out = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=public margin=600", yard.0),
    );
    assert!(out.contains("margin 600"), "{out}");
    let owner_mid = posted(&rig, owner_account);
    lay_wrecker_bill(&mut rig, VaultId(raised), 98_400);
    settle(&mut rig, 3);
    assert_eq!(
        order_state(&mut rig, raised),
        mush_industry::workorder::WorkOrderState::Cancelled,
        "the risen fee cancels the order"
    );
    let c = contract(&rig, raised);
    // The customer has no hold docked, so the bill waits in the escrow for `/collect` and the
    // contract stays open until then.
    assert!(c.stale && c.closed.is_none(), "{c:?}");
    assert_eq!(
        posted(&rig, customer_account),
        before,
        "the booking fee is not taken"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|s| s
                .snapshot()
                .account(customer_account)
                .map(|a| a.held_amount.minor())),
        Some(held_before),
        "nothing stays held"
    );
    assert_eq!(
        posted(&rig, owner_account),
        owner_mid,
        "the owner is paid nothing"
    );
    let lowered = booked(&mut rig, customer, "+yard/order tv_wrecker");
    let out = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=public margin=100", yard.0),
    );
    assert!(out.contains("margin 100"), "{out}");
    lay_wrecker_bill(&mut rig, VaultId(lowered), 98_410);
    settle(&mut rig, 3);
    assert_ne!(
        order_state(&mut rig, lowered),
        mush_industry::workorder::WorkOrderState::Cancelled,
        "a fee that fell is charged as it stands"
    );
    assert!(!contract(&rig, lowered).stale);
}

/// **Doc 48 unit PL.9 (Q-48-45, Q-48-53): a busy or unsellable facility is refused with its
/// reason.** The Ceres works' stores are Freeport Works' storefront. The Stickney yard with a
/// captain's order open is busy. The Stickney works' stores hold goods. The Stickney workshop,
/// titled to the Yards by staff so it reaches the seam, is a programme workshop. Nothing is
/// listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pl9_a_busy_or_unsellable_facility_is_refused_with_its_reason() {
    let mut rig = stocked();
    let line = |template: &str, rig: &Rig, room: ObjId| {
        let name = rig
            .tw
            .world()
            .get(room)
            .map(|o| o.name.to_string())
            .expect("the room");
        template.replace("{name}", &name)
    };
    let ceres = station_room(&rig.tw, "ceres_port", "room.c15_maintenance");
    let storefront = list_for(
        &mut rig,
        ("ceres_port", "freeport_works", "ceres"),
        ceres,
        1_000,
    )
    .expect_err("a storefront's facility");
    assert!(
        storefront.contains(&line(
            tacsim::port::facility_sale::FACILITY_STOREFRONT,
            &rig,
            ceres
        )),
        "{storefront}"
    );
    let customer = free_captain(&mut rig, "Customer", 83);
    let _ = booked(&mut rig, customer, "+yard/order tv_wrecker");
    let yard = yard_room(&rig);
    let busy = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        yard,
        1_000,
    )
    .expect_err("a busy yard");
    assert!(
        busy.contains(&line(
            tacsim::port::facility_sale::FACILITY_BUSY,
            &rig,
            yard
        )),
        "{busy}"
    );
    let works = station_room(&rig.tw, STICKNEY, "room.p15_maintenance");
    let works_stores = tacsim::port::facility_sale::stores_of(rig.tw.world(), works)
        .expect("the works' stores, which its house purchase opened");
    seed_pool(&mut rig, works_stores, ALLOYS, 1, LotId(98_420));
    let laden = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        works,
        1_000,
    )
    .expect_err("a works with goods in its stores");
    assert!(
        laden.contains(&line(
            tacsim::port::facility_sale::FACILITY_STORES_LADEN,
            &rig,
            works
        )),
        "{laden}"
    );
    let shop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    let untitled = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        shop,
        1_000,
    )
    .expect_err("an untitled workshop");
    assert!(
        untitled.contains(mush_market::seller::NO_SUCH_ASSET),
        "{untitled}"
    );
    let yards = org_of(&rig, STICKNEY, "stickney_yards");
    let titled_out = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=org:{}", shop.0, yards.0),
    );
    assert!(!titled_out.contains("Usage"), "{titled_out}");
    let workshop = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        shop,
        1_000,
    )
    .expect_err("a workshop");
    assert!(
        workshop.contains(&line(
            tacsim::port::facility_sale::FACILITY_WORKSHOP,
            &rig,
            shop
        )),
        "{workshop}"
    );
    assert!(
        ![ceres, yard, works, shop]
            .iter()
            .any(|object| mush_market::asset_on_sale(rig.tw.world(), *object)),
        "nothing was listed"
    );
}

// ---- doc 48e review fixes ------------------------------------------------------------------

/// The IC clock to 600 s into the next IC day, a few fires landed there.
fn next_day(rig: &mut Rig) {
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    ic_to(rig, day * DAY + 600);
    settle(rig, 8);
}

/// A contract's work order, read.
fn work_order(rig: &mut Rig, id: u64) -> mush_industry::workorder::WorkOrder {
    let order = contract(rig, id).order.expect("the draft landed");
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
        &mush_industry::workorder::WorkOrderId(order),
    )
    .expect("the work order")
}

fn upkeep_row(rig: &Rig, room: ObjId) -> tacsim::port::upkeep::UpkeepRow {
    tacsim::port::upkeep::row_of(rig.tw.world(), room)
        .unwrap_or_else(|| panic!("#{} keeps a running-cost row", room.0))
}

/// **Doc 48e (production G1, G2, G4, G24, Q-48-45, -53): a facility owing running costs is not
/// sold; one on sale takes no order; its buyer books at it as a customer; and a margin raised
/// under a running order does not raise its forfeit.**
/// 1. The Yards' account drained, the next day's charge is a debt, and the yard does not list
///    (`FACILITY_IN_DEBT`): its buyer would inherit the debt. Paid, it lists.
/// 2. Listed, it takes no new order (`works::ON_SALE`).
/// 3. Bought, its owner's own order is a customer's, not a house order the arm would collect.
/// 4. A day into that order, the owner raises the margin: a cancel would forfeit what it would
///    have before (the fee pinned in the hold).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g1_a_facility_owing_running_costs_is_not_sold_and_one_on_sale_takes_no_order() {
    let mut rig = stocked();
    let yard = yard_room(&rig);
    let yards_account = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")]
        .account
        .map(mush_economy::AccountId)
        .expect("the Yards' account");
    let treasury = port_map(&rig.tw, STICKNEY).treasury.expect("a treasury");
    let name = name_of(&rig, yard);

    // ---- (1) In debt: not sold ----
    next_day(&mut rig);
    let all = posted(&rig, yards_account);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={treasury} {all}", yards_account.0),
    );
    assert_eq!(posted(&rig, yards_account), 0, "{out}");
    next_day(&mut rig);
    assert!(upkeep_row(&rig, yard).debt_minor > 0, "the yard owes");
    let refused = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        yard,
        YARD_PRICE,
    )
    .expect_err("a yard in debt");
    assert!(
        refused.contains(&tacsim::port::facility_sale::FACILITY_IN_DEBT.replace("{name}", &name)),
        "{refused}"
    );
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {treasury}={} 50000000", yards_account.0),
    );
    next_day(&mut rig);
    assert_eq!(upkeep_row(&rig, yard).debt_minor, 0, "the debt is paid");
    let listing = list_for(
        &mut rig,
        (STICKNEY, "stickney_yards", "stickney"),
        yard,
        YARD_PRICE,
    )
    .expect("paid, it lists");

    // ---- (2) On sale: no new order ----
    let customer = free_captain(&mut rig, "Customer", 84);
    let out = run(&mut rig.tw, customer, "+yard/order tv_wrecker");
    assert!(out.contains(tacsim::port::works::ON_SALE), "{out}");

    // ---- (3) Bought: the owner books as a customer ----
    let owner = free_captain(&mut rig, "YardOwner", 85);
    buy_listing(&mut rig, owner, listing);
    assert_eq!(facility_of(&rig, yard).owner(), &PrincipalId::Object(owner));
    let floor = open_yard(&mut rig);
    teleport(&mut rig.tw, owner, floor);
    let own = booked(&mut rig, owner, "+yard/order tv_wrecker");
    assert!(
        !contract(&rig, own).house,
        "a bought yard's owner books as a customer"
    );

    // ---- (4) A margin raised under a running order ----
    lay_wrecker_bill(&mut rig, VaultId(own), 98_430);
    settle(&mut rig, 3);
    days_on(&mut rig, 1);
    let order = work_order(&mut rig, own);
    assert_eq!(
        *order.state(),
        mush_industry::workorder::WorkOrderState::Running,
        "the order runs"
    );
    let ran = order
        .schedule()
        .map_or(0, mush_industry::ProductionSchedule::progress_units);
    assert!(ran > 0, "a day of work");
    let held = mush_industry::money::order_price_minor(rig.tw.world(), &order);
    let before = mush_industry::money::cancel_forfeit_minor(rig.tw.world(), &order, held);
    assert!(before > 0, "the work done is forfeit on a cancel");
    let out = run(
        &mut rig.tw,
        owner,
        &format!("@facility/terms #{}=public margin=900", yard.0),
    );
    assert!(out.contains("margin 900"), "{out}");
    assert_eq!(
        mush_industry::money::cancel_forfeit_minor(rig.tw.world(), &order, held),
        before,
        "a margin raised under a running order does not raise its forfeit"
    );
}

/// **Doc 48e (production G5): a sold refinery never bills its buyer for its old operator's
/// batches.** Kasei's officer sells the Stickney refinery to a captain; Kasei still mills two
/// batches on its own storefront that day, and the captain's next charge is the refinery's idle
/// standing share, as any slot facility's, not those batches.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g5_a_sold_refinery_never_bills_its_buyer_for_the_operators_batches() {
    let mut rig = stocked();
    let refinery = station_room(&rig.tw, STICKNEY, "room.p07_receiving");
    next_day(&mut rig);
    let listing = list_for(
        &mut rig,
        (STICKNEY, "kasei", "stickney"),
        refinery,
        YARD_PRICE,
    )
    .expect("the idle refinery lists");
    let buyer = free_captain(&mut rig, "Smelter", 86);
    buy_listing(&mut rig, buyer, listing);
    let sold = facility_of(&rig, refinery);
    assert_eq!(sold.owner(), &PrincipalId::Object(buyer));
    assert!(!sold.holding().is_authored(), "owner-held");
    let kasei = VaultId(port_map(&rig.tw, STICKNEY).vendors[&cid("kasei")].vault);
    seed_pool(&mut rig, kasei, "metals", 40, LotId(98_440));
    settle(&mut rig, 30);
    next_day(&mut rig);
    let cost = *sold.cost().expect("a costed refinery");
    let r = upkeep_row(&rig, refinery);
    assert_eq!(
        r.charged_minor,
        tacsim::port::upkeep::standing_day(&cost, sold.maximum_slots()),
        "the buyer pays the refinery's idle day, not Kasei's batches"
    );
}
