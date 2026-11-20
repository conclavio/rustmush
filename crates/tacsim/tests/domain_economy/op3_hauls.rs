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
