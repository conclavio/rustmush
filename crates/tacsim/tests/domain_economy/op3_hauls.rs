//! **Doc 50 cut OP.3 — an organization's hauls, their escorts, and a loss** (Q-50-23, Q-50-25,
//! Q-50-63; doc 50a A-8b), over the composed world (`driving`'s rig).
//!
//! - **Posting as an organization.** A member who holds `tacsim.jobs.post` on Occator posts a haul
//!   `as=` Occator, from its storefront at Ceres into the Isidis tank farm's at Isidis (vault to
//!   vault), the fee from Occator's account; a player without the capability is refused.
//! - **An escort for the haul**, posted with it before anyone took it, binds to the hauler's hull
//!   when the haul is taken, and is watched across both hops (Ceres to Stickney, Stickney to
//!   Isidis): docking at Stickney is a hop, not the end, so it is kept only at Isidis.
//! - **A loss.** A hauler's hull wrecked with the goods aboard owes the lost leg: the hold is
//!   released, so the goods are part of the wreck, and the job fails with the bond to the shipper.
//!
//! **What is a fixture.** The capability by staff's `@org/grant`; the hull's wreck by staff's
//! `@unit/damage`, every cell of her unit to its least, which the haul's `wrecked` reads.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::services::escort::{self, EscortStage};
use tacsim::services::haul::{self, HaulStage};

use crate::driving_rig::{
    Rig, advance_to, board, file_and_launch, fire, ic_now, land_quietly, load, pilot, port_map,
    prospector, run, set_rate, station_room, stocked, sweep, teleport,
};

const GOOD: &str = "volatiles";
const QTY: u64 = 20;

fn org_of(rig: &Rig, entry: &str, id: &str) -> u64 {
    port_map(&rig.tw, entry).orgs[&mush_core::CanonicalId::new(id).expect("an id")].org
}

fn org_account(rig: &Rig, org: u64) -> AccountId {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&PrincipalId::Organization(mush_org::OrgId(org)))
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("a credit account")
}

/// Posted balance less what is held.
fn balance(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor() - a.held_amount.minor())
        })
        .expect("the account")
}

fn state(rig: &Rig, contract: u64) -> ContractState {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract)).map(|c| c.state))
        .expect("the contract")
}

fn haul_job(rig: &Rig, contract: u64) -> haul::HaulJob {
    haul::haul_of(rig.tw.world(), contract).expect("the haul")
}

fn escort_job(rig: &Rig, contract: u64) -> escort::EscortJob {
    escort::escort_of(rig.tw.world(), contract).expect("the escort")
}

/// Several port fires, what they stage landed, and the economy's sweep settled.
fn settle(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
        sweep(&mut rig.tw, 60);
        crate::in10_pools::settle(rig, 1);
    }
}

/// Occator's factor, standing at Ceres' exchange, granted `tacsim.jobs.post` on Occator, posts a
/// haul of [`QTY`] [`GOOD`] `as=` Occator into the tank farm at Isidis. `(factor, haul contract)`.
fn org_haul(rig: &mut Rig) -> (ObjId, u64) {
    let factor = pilot(rig, "Factor", 81);
    let exchange = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, factor, exchange);
    let occator = org_of(rig, "ceres_port", "occator");
    let farm = org_of(rig, "isidis_port", "isidis_tank_farm");
    let line = format!("+haul/post {QTY} {GOOD} to=isidis fee=500 as=#{occator} into=#{farm} go");
    // Without the capability: refused.
    let refused = run(&mut rig.tw, factor, &line);
    assert!(refused.contains(tacsim::services::NOT_POSTER), "{refused}");
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@org/grant #{}=#{occator} {}",
            factor.0,
            tacsim::services::JOBS_POST_CAPABILITY
        ),
    );
    assert!(!granted.contains("Usage"), "{granted}");
    let before = balance(rig, org_account(rig, occator));
    let posted = run(&mut rig.tw, factor, &line);
    assert!(posted.contains("posted"), "{posted}");
    let job = haul::hauls(rig.tw.world())
        .into_iter()
        .filter(|j| j.shipper == factor)
        .max_by_key(|j| j.contract)
        .expect("the haul");
    assert_eq!(job.shipper_org, Some(occator), "posted for Occator");
    assert_eq!(
        Some(job.source),
        tacsim::services::org_vault(rig.tw.world(), mush_org::OrgId(occator), "ceres"),
        "from Occator's vault at Ceres"
    );
    assert_eq!(
        Some(job.drop),
        tacsim::services::org_vault(rig.tw.world(), mush_org::OrgId(farm), "isidis"),
        "into the tank farm's vault at Isidis"
    );
    assert_eq!(
        balance(rig, org_account(rig, occator)),
        before - 50_000,
        "the fee is escrowed from Occator's account"
    );
    (factor, job.contract)
}

/// **Doc 50 cut OP.3 acceptance: an organization's haul and its escort, vault to vault across
/// two hops** (Q-50-23, Q-50-63; doc 50a A-8b).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn op3_an_org_haul_and_its_escort_run_ceres_to_isidis_vault_to_vault() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    let (factor, haul_contract) = org_haul(&mut rig);
    let occator = org_of(&rig, "ceres_port", "occator");

    // The escort, posted with the haul before anyone took it: unbound.
    let posted = run(
        &mut rig.tw,
        factor,
        &format!("+escort/post haul={haul_contract} fee=300 as=#{occator} go"),
    );
    assert!(posted.contains("posted"), "{posted}");
    let escort_contract = escort::escorts(rig.tw.world())
        .into_iter()
        .find(|j| j.haul == Some(haul_contract))
        .map(|j| j.contract)
        .expect("the escort for the haul");
    assert_eq!(escort_job(&rig, escort_contract).protected, escort::UNBOUND);
    assert_eq!(escort_job(&rig, escort_contract).poster_org, Some(occator));

    // A guard takes the escort; a hauler takes the haul at Ceres and the goods load.
    let (guard, guard_row) = prospector(&mut rig, "Guard", 82, "Sentry");
    let (hauler, hauler_row) = prospector(&mut rig, "Hauler", 83, "Tallow");
    let (guard_hull, hull) = (ObjId(guard_row.hull), ObjId(hauler_row.hull));
    let took = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {escort_contract} with=#{} go", guard_hull.0),
    );
    assert!(took.contains("You take escort"), "{took}");
    let took = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took.contains("You take haul"), "{took}");
    settle(&mut rig, 3);
    assert_eq!(haul_job(&rig, haul_contract).stage, HaulStage::Aboard);
    assert_eq!(
        escort_job(&rig, escort_contract).protected,
        hull,
        "the escort binds to the hauler's hull when the haul is taken"
    );

    // Both fly Ceres to Stickney together.
    load(&mut rig, &hauler_row, 60, 40, 0x0b31);
    load(&mut rig, &guard_row, 60, 40, 0x0b33);
    board(&mut rig, hauler);
    board(&mut rig, guard);
    let first = file_and_launch(&mut rig, hauler, hull, "+ship/undock stickney");
    let second = file_and_launch(&mut rig, guard, guard_hull, "+ship/undock stickney");
    let arrived = (first.plan.depart_ic + first.plan.total_s)
        .max(second.plan.depart_ic + second.plan.total_s);
    let (_, queued) = advance_to(&mut rig, arrived + 3_600);
    land_quietly(&mut rig, &queued);
    settle(&mut rig, 3);
    let at_stickney = escort_job(&rig, escort_contract);
    assert_eq!(
        at_stickney.stage,
        EscortStage::Watching,
        "Stickney is a hop, not the end: {at_stickney:?}"
    );

    // Stickney to Isidis, together; kept at the destination.
    load(&mut rig, &hauler_row, 20, 4, 0x0b35);
    load(&mut rig, &guard_row, 20, 4, 0x0b37);
    let first = file_and_launch(&mut rig, hauler, hull, "+ship/undock isidis");
    let second = file_and_launch(&mut rig, guard, guard_hull, "+ship/undock isidis");
    let arrived = (first.plan.depart_ic + first.plan.total_s)
        .max(second.plan.depart_ic + second.plan.total_s);
    let (_, queued) = advance_to(&mut rig, arrived + 3_600);
    land_quietly(&mut rig, &queued);
    settle(&mut rig, 3);
    let kept = escort_job(&rig, escort_contract);
    assert!(
        matches!(kept.stage, EscortStage::Kept | EscortStage::Closed) && kept.why.is_none(),
        "kept at Isidis: {kept:?}"
    );

    // The haul delivers into the tank farm's vault; the contract settles.
    let delivering = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {haul_contract} go"),
    );
    assert!(!delivering.contains("not docked"), "{delivering}");
    settle(&mut rig, 4);
    let done = haul_job(&rig, haul_contract);
    assert!(
        matches!(done.stage, HaulStage::Delivered | HaulStage::Closed),
        "{done:?}"
    );
    assert!(
        matches!(state(&rig, haul_contract), ContractState::Settled { .. }),
        "{:?}",
        state(&rig, haul_contract)
    );
    let _ = ic_now(&rig.tw);
}

/// **Doc 50 cut OP.3: a hull lost with a haul's goods** (Q-50-25): the lost leg releases the
/// hold, so the goods are part of the wreck, and the job fails with the hauler's bond to the
/// shipper.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op3_a_hull_wrecked_with_a_hauls_goods_releases_them_and_fails_the_job() {
    let mut rig = stocked();
    let (_, haul_contract) = org_haul(&mut rig);
    let (hauler, hauler_row) = prospector(&mut rig, "Hauler", 84, "Tallow");
    let hull = ObjId(hauler_row.hull);
    let took = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took.contains("You take haul"), "{took}");
    settle(&mut rig, 3);
    let aboard = haul_job(&rig, haul_contract);
    assert_eq!(aboard.stage, HaulStage::Aboard);
    let hold = aboard.hold.expect("the hold");
    assert_eq!(pool(&mut rig, hold), (QTY, 0), "aboard, all of it held");

    // Her hull is wrecked with the goods aboard: staff's damage through the unit's one plan
    // path, every cell to its least.
    let unit = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("her hull is a unit");
    let ops: Vec<mush_tactics::UnitMutationOp> = unit
        .cells()
        .filter(|(_, cell)| cell.current > cell.min)
        .map(|(addr, cell)| mush_tactics::UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: cell.min,
        })
        .collect();
    let plan = mush_tactics::UnitMutationPlan::new(
        ops,
        Some(unit.revision()),
        Some("the hull is lost".into()),
    )
    .expect("a plan");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@unit/damage #{}={}",
            hull.0,
            serde_json::to_string(&plan).expect("the plan's text")
        ),
    );
    assert!(!out.contains("Usage"), "{out}");
    assert!(escort::wrecked(rig.tw.world(), hull));
    settle(&mut rig, 4);
    let lost = haul_job(&rig, haul_contract);
    assert_eq!(lost.stage, HaulStage::Closed, "{lost:?}");
    assert!(
        matches!(state(&rig, haul_contract), ContractState::Expired),
        "the job lapsed: {:?}",
        state(&rig, haul_contract)
    );
    // The goods are free in the wreck's hold: aboard, and none of it held for the job.
    assert_eq!(
        pool(&mut rig, hold),
        (QTY, QTY),
        "the job's hold is released"
    );
}

/// The hold's pool of [`GOOD`]: `(quantity, available)`.
fn pool(rig: &mut Rig, hold: u64) -> (u64, u64) {
    let families = rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let mut txn = rig
        .tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("an inventory transaction");
    families
        .pool(
            &mut txn,
            mush_inventory::record::EndpointRef(mush_economy::VaultId(hold)),
            &mush_core::CanonicalId::new(GOOD).expect("a good"),
        )
        .expect("the pool reads")
        .map_or((0, 0), |p| (p.quantity, p.available))
}

/// **Doc 50 cut OP.3: the departure grace holds at a high clock rate** (Q-50-63; doc 50a A-8b).
/// At 3,600:1 one port fire is past [`escort::TOGETHER_IC`]; an escort still docked at the origin
/// one fire after the protected hull launched is setting out, not leaving, inside
/// [`escort::LAUNCH_GRACE_WALL_S`] real seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op3_an_escort_one_fire_behind_at_a_high_rate_has_not_left() {
    let mut rig = stocked();
    let (poster, row) = prospector(&mut rig, "Poster", 85, "Ward");
    let (guard, guard_row) = prospector(&mut rig, "Guard", 86, "Sentry");
    let (hull, with) = (ObjId(row.hull), ObjId(guard_row.hull));
    let posted = run(
        &mut rig.tw,
        poster,
        &format!("+escort/post #{} to=stickney fee=300 go", hull.0),
    );
    assert!(posted.contains("posted"), "{posted}");
    let contract = escort::escorts(rig.tw.world())
        .into_iter()
        .filter(|j| j.poster == poster)
        .map(|j| j.contract)
        .max()
        .expect("the escort");
    let took = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {contract} with=#{} go", with.0),
    );
    assert!(took.contains("You take escort"), "{took}");
    settle(&mut rig, 1);
    set_rate(&mut rig.tw, 3_600);
    load(&mut rig, &row, 60, 40, 0x0b41);
    board(&mut rig, poster);
    let _ = file_and_launch(&mut rig, poster, hull, "+ship/undock stickney");
    let mut started = None;
    for _ in 0..10 {
        if let Some(watch) = escort_job(&rig, contract).watch {
            started = Some(watch.started_ic);
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let started = started.expect("the watch starts when her journey does");
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(
        ic_now(&rig.tw) - started > escort::TOGETHER_IC,
        "one fire is past the IC gap at this rate"
    );
    let watching = escort_job(&rig, contract);
    assert_eq!(
        watching.stage,
        EscortStage::Watching,
        "one fire behind is setting out: {watching:?}"
    );
}

/// Wreck `hull`: staff's damage through the unit's one plan path, every cell to its least.
fn wreck(rig: &mut Rig, hull: ObjId) {
    let unit = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("the hull is a unit");
    let ops: Vec<mush_tactics::UnitMutationOp> = unit
        .cells()
        .filter(|(_, cell)| cell.current > cell.min)
        .map(|(addr, cell)| mush_tactics::UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: cell.min,
        })
        .collect();
    let plan = mush_tactics::UnitMutationPlan::new(
        ops,
        Some(unit.revision()),
        Some("the hull is lost".into()),
    )
    .expect("a plan");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@unit/damage #{}={}",
            hull.0,
            serde_json::to_string(&plan).expect("the plan's text")
        ),
    );
    assert!(!out.contains("Usage"), "{out}");
    assert!(escort::wrecked(rig.tw.world(), hull));
}

/// **Doc 50b (OP.3): a haul taken in its escort's own hull calls the escort off.** A pilot takes
/// the escort posted for a haul with her hull, then takes the haul in the same hull: a hull does
/// not escort herself, so the escort is called off and its contract cancelled, and the haul runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op50b_a_haul_taken_in_its_escorts_own_hull_calls_the_escort_off() {
    let mut rig = stocked();
    let (factor, haul_contract) = org_haul(&mut rig);
    let occator = org_of(&rig, "ceres_port", "occator");
    let posted = run(
        &mut rig.tw,
        factor,
        &format!("+escort/post haul={haul_contract} fee=300 as=#{occator} go"),
    );
    assert!(posted.contains("posted"), "{posted}");
    let escort_contract = escort::escorts(rig.tw.world())
        .into_iter()
        .find(|j| j.haul == Some(haul_contract))
        .map(|j| j.contract)
        .expect("the escort for the haul");
    let (both, row) = prospector(&mut rig, "Both", 87, "Selfsame");
    let hull = ObjId(row.hull);
    let took = run(
        &mut rig.tw,
        both,
        &format!("+escort/accept {escort_contract} with=#{} go", hull.0),
    );
    assert!(took.contains("You take escort"), "{took}");
    let took = run(
        &mut rig.tw,
        both,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took.contains("You take haul"), "{took}");
    settle(&mut rig, 3);
    let called_off = escort_job(&rig, escort_contract);
    assert_eq!(called_off.stage, EscortStage::Closed, "{called_off:?}");
    assert_ne!(called_off.protected, hull, "never bound to herself");
    assert!(
        !state(&rig, escort_contract).is_live(),
        "the escort's contract is cancelled: {:?}",
        state(&rig, escort_contract)
    );
    assert_eq!(haul_job(&rig, haul_contract).stage, HaulStage::Aboard);
}

/// **Doc 50b (OP.3): a delivery committed before the hull is lost stays delivered.** The hauler
/// delivers at Isidis and the leg lands; her hull is wrecked before the haul's arm next looks.
/// The arm acknowledges the delivery first: the contract settles and owes no lost leg.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op50b_a_delivery_committed_before_the_hull_is_lost_stays_delivered() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    let (_, haul_contract) = org_haul(&mut rig);
    let (hauler, row) = prospector(&mut rig, "Hauler", 88, "Tallow");
    let hull = ObjId(row.hull);
    let took = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took.contains("You take haul"), "{took}");
    settle(&mut rig, 3);
    assert_eq!(haul_job(&rig, haul_contract).stage, HaulStage::Aboard);
    load(&mut rig, &row, 60, 40, 0x0b51);
    board(&mut rig, hauler);
    for (to, lot) in [("stickney", 0x0b53), ("isidis", 0x0b55)] {
        if to == "isidis" {
            load(&mut rig, &row, 20, 4, lot);
        }
        let ticket = file_and_launch(&mut rig, hauler, hull, &format!("+ship/undock {to}"));
        let (_, queued) = advance_to(
            &mut rig,
            ticket.plan.depart_ic + ticket.plan.total_s + 3_600,
        );
        land_quietly(&mut rig, &queued);
        settle(&mut rig, 3);
    }
    let delivering = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {haul_contract} go"),
    );
    assert!(!delivering.contains("not docked"), "{delivering}");
    // The delivery leg lands; the hull is lost before the arm's next fire.
    let queued = crate::driving_rig::drain(&mut rig.tw);
    land_quietly(&mut rig, &queued);
    wreck(&mut rig, hull);
    settle(&mut rig, 4);
    let done = haul_job(&rig, haul_contract);
    assert!(
        matches!(done.stage, HaulStage::Delivered | HaulStage::Closed),
        "delivered, not lost: {done:?}"
    );
    assert!(
        matches!(state(&rig, haul_contract), ContractState::Settled { .. }),
        "the contract settles: {:?}",
        state(&rig, haul_contract)
    );
}

/// **Doc 50b (doc 50 §7's cargo reading): a full Dray-P can neither mine more nor take an
/// organization's haul.** Her hold is filled with a bulk trade good held as a lot; the ore hold's reading counts
/// it, so no room is left for ore, and the haul's admission refuses her.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op50b_a_full_dray_p_can_neither_mine_more_nor_take_a_haul() {
    let mut rig = stocked();
    let (_, haul_contract) = org_haul(&mut rig);
    let (pilot, row) = prospector(&mut rig, "Full", 89, "Brimful");
    let hull = ObjId(row.hull);
    let cap = tacsim::space::mining::hull_hold_capacity_tonnes(rig.tw.world(), hull);
    assert!(cap > 0, "a Dray-P has a hold");
    // Held as a lot, as a buyer of a personal ask receives it: seeded pooled, then crystallized,
    // so a pool-only reading misses it.
    let hold = crate::driving_rig::hold_vault_of(&rig.tw, &row);
    crate::driving_rig::seed_hold(&mut rig, &row, "structural_alloys", cap, 0x0b61);
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("@inv/crystallize #{}/structural_alloys={cap}", hold.0),
    );
    let queued = crate::driving_rig::drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{out}");
    crate::driving_rig::land_all(&mut rig, &queued);
    assert_eq!(
        crate::driving_rig::hold_pool(&mut rig.tw, hold, "structural_alloys"),
        0,
        "none of it pooled"
    );
    // The haul's admission refuses her (it is the docked hull's helm she takes it in).
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(refused.contains(haul::NO_ROOM), "{refused}");
    // Aboard, the mining status reads the ore hold full: no room is left to mine into.
    board(&mut rig, pilot);
    let status = run(&mut rig.tw, pilot, "+mine/status");
    assert!(
        status.contains(&format!("hold: {cap}/{cap} t")),
        "the alloys fill the ore hold: {status}"
    );
    let _ = hull;
}

/// **P2-TSW-10**: a space passage escort refuses a leased ground rig (`with=<tv>`), never grants
/// `within_grace` without prior verified proximity (or from decoupled free-flight), and enforces
/// a non-renewable cumulative grace cap across linkdead-reconnect cycles.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn p2_tsw_10_passage_escort_rejects_ground_rig_and_bounds_linkdead_grace() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    let (poster, poster_row) = prospector(&mut rig, "Principal", 91, "Lantern");
    let (guard, guard_row) = prospector(&mut rig, "Guard", 92, "Vigil");
    let (hull, guard_hull) = (ObjId(poster_row.hull), ObjId(guard_row.hull));

    // Guard leases a ground TV at Ceres exchange and tries to take a space passage escort with it.
    crate::driving_rig::lease_one(&mut rig, guard);
    let tv = crate::driving_rig::rigs_of(&rig.tw, guard)[0];

    let posted = run(
        &mut rig.tw,
        poster,
        &format!("+escort/post #{} to=stickney fee=300 go", hull.0),
    );
    assert!(posted.contains("posted"), "{posted}");
    let contract = escort::escorts(rig.tw.world())
        .into_iter()
        .filter(|j| j.poster == poster)
        .map(|j| j.contract)
        .max()
        .expect("the escort");

    // (1) Cross-mode ground rig must be refused for a space passage escort.
    let refused_tv = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {contract} with=#{} go", tv.0),
    );
    assert!(
        refused_tv.contains("The escort could not be taken:") && refused_tv.contains("ship hull"),
        "a leased ground rig cannot accept a space passage escort: {refused_tv}"
    );
    assert_eq!(escort_job(&rig, contract).stage, EscortStage::Posted);

    // (2) Accept with a real ship hull, decouple into free-flight outside Ceres without ever
    // flying with the principal, and cycle real `ConnDown`/`ConnUp` linkdead drops while the
    // principal flies Ceres -> Stickney. Zero `within_grace` credit is awarded and the escort
    // fails on arrival.
    let took = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {contract} with=#{} go", guard_hull.0),
    );
    assert!(took.contains("You take escort"), "{took}");
    settle(&mut rig, 1);

    load(&mut rig, &guard_row, 60, 40, 0x0b73);
    board(&mut rig, guard);
    let _guard_ticket = file_and_launch(&mut rig, guard, guard_hull, "+ship/undock stickney");
    let decoupled = run(&mut rig.tw, guard, "+helm/decouple");
    assert!(
        !decoupled.contains("Usage"),
        "guard decouples into free-flight: {decoupled}"
    );
    let guard_conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(guard))
        .collect();
    assert!(!guard_conns.is_empty(), "guard holds a live connection");
    for &conn in &guard_conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: guard,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.linkdead_state(guard_hull))
            .is_some(),
        "guard has a genuine SpaceLinkdeadState in free-flight"
    );

    load(&mut rig, &poster_row, 60, 40, 0x0b71);
    board(&mut rig, poster);
    let ticket = file_and_launch(&mut rig, poster, hull, "+ship/undock stickney");
    // Wait for watch to start while guard is linkdead in free-flight.
    for _ in 0..10 {
        if escort_job(&rig, contract).stage == EscortStage::Watching {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(escort_job(&rig, contract).stage, EscortStage::Watching);
    assert!(
        !escort::within_grace(rig.tw.world(), &escort_job(&rig, contract), guard_hull),
        "within_grace must deny a decoupled free-flight linkdead escort with no prior proximity"
    );

    // Cycle real reconnect -> drop link across several port fires while principal is in transit.
    let conn = guard_conns[0];
    for _ in 0..4 {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnUp {
                conn,
                actor: guard,
                caps_structured: true,
                from_linkdead: true,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: guard,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let mid = escort_job(&rig, contract);
    assert_eq!(
        mid.watch.as_ref().map_or(-1, |w| w.near_ic),
        0,
        "linkdead-reconnect cycling in free-flight must credit zero near_ic: {mid:?}"
    );

    // Advance principal to arrival at Stickney: escort fails with WHY_FAR, never Kept.
    let (_, queued) = advance_to(
        &mut rig,
        ticket.plan.depart_ic + ticket.plan.total_s + 3_600,
    );
    land_quietly(&mut rig, &queued);
    settle(&mut rig, 3);
    let ended = escort_job(&rig, contract);
    assert_eq!(ended.stage, EscortStage::Failed, "{ended:?}");
    assert!(
        ended.why.as_deref().is_some_and(|w| w.contains("only 0%")),
        "failed for 0% proximity: {ended:?}"
    );
}

/// **P2-TSW-22**: when a haul is cancelled before loading (`no_room` during `HaulStage::Taken`),
/// a paired escort does not bind prematurely to the hauler's hull and is called off cleanly
/// (`TOLD_HAUL_GONE`, contract cancelled) instead of staying live for an empty hull.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p2_tsw_22_escort_cancels_when_haul_cancels_before_load() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    let (factor, haul_contract) = org_haul(&mut rig);
    let occator = org_of(&rig, "ceres_port", "occator");
    let occator_before_escort = balance(&rig, org_account(&rig, occator));

    let posted = run(
        &mut rig.tw,
        factor,
        &format!("+escort/post haul={haul_contract} fee=300 as=#{occator} go"),
    );
    assert!(posted.contains("posted"), "{posted}");
    let escort_contract = escort::escorts(rig.tw.world())
        .into_iter()
        .find(|j| j.haul == Some(haul_contract))
        .map(|j| j.contract)
        .expect("the escort for the haul");
    assert_eq!(
        balance(&rig, org_account(&rig, occator)),
        occator_before_escort - 30_000,
        "escort fee escrowed"
    );

    let (guard, guard_row) = prospector(&mut rig, "Guard", 93, "Sentry");
    let (hauler, hauler_row) = prospector(&mut rig, "Hauler", 94, "Shell");
    let (guard_hull, hull) = (ObjId(guard_row.hull), ObjId(hauler_row.hull));
    let took_escort = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {escort_contract} with=#{} go", guard_hull.0),
    );
    assert!(took_escort.contains("You take escort"), "{took_escort}");

    // Hauler accepts the haul -> stage is `Taken`, `Load` leg is queued but NOT landed yet.
    let took_haul = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took_haul.contains("You take haul"), "{took_haul}");
    assert_eq!(haul_job(&rig, haul_contract).stage, HaulStage::Taken);

    // Discard the unlanded initial Load submission (simulating authoritative back-pressure so Load
    // is delayed to a later tick) and step one port fire: escort must remain UNBOUND while Taken.
    let _dropped_load = crate::driving_rig::drain(&mut rig.tw);
    let (_, queued) = fire(&mut rig);
    assert_eq!(
        escort_job(&rig, escort_contract).protected,
        escort::UNBOUND,
        "escort must not bind before Load lands (HaulStage::Aboard)"
    );
    let _ = queued;

    // Now fill the hauler's hold to capacity so `haul::step` cancels the haul with `no_room`.
    let cap = tacsim::space::mining::hull_hold_capacity_tonnes(rig.tw.world(), hull);
    crate::driving_rig::seed_hold(&mut rig, &hauler_row, "structural_alloys", cap, 0x0b81);

    settle(&mut rig, 3);
    assert_eq!(
        haul_job(&rig, haul_contract).stage,
        HaulStage::Closed,
        "haul cancelled on no_room before load"
    );
    let cancelled_escort = escort_job(&rig, escort_contract);
    assert_eq!(
        cancelled_escort.stage,
        EscortStage::Closed,
        "paired escort called off with the cancelled haul: {cancelled_escort:?}"
    );
    assert_eq!(
        cancelled_escort.protected,
        escort::UNBOUND,
        "never bound to the empty hull"
    );
    assert!(
        matches!(state(&rig, escort_contract), ContractState::Cancelled),
        "escort contract cancelled: {:?}",
        state(&rig, escort_contract)
    );
    assert_eq!(
        balance(&rig, org_account(&rig, occator)),
        occator_before_escort + 50_000,
        "both the cancelled haul fee (500 cr) and escort fee (300 cr) returned to Occator in full"
    );
}
