//! **Doc 50 cut OP.4 — the Isidis stockpile, its drain, the Freeport arm and the hauling house**
//! (Q-50-21, Q-50-22, Q-50-24, Q-50-56, Q-50-60), over the composed world (`driving`'s rig),
//! through the port tick.
//!
//! - **The arm posts** for Occator, through the players' verbs, as its own agent: a haul of
//!   propellant from Occator's vault at Ceres into the tank farm's at Isidis, and an escort for it,
//!   once an IC day while the farm is short, at most two open at once.
//! - **The farm drains** 20 t an IC day (lane v49's `consume` row), and the port tick reads its
//!   level. At the threshold the close probe is met and Hold the Concession moves once that month;
//!   the arm posts nothing more while the farm stands there.
//! - **An untaken haul** is carried by Ceres' hauling house after seven IC days, in a real hull,
//!   by the Kirkwood route, and delivered into the tank farm.
//! - **A house hull at the picket** pays the transit duty from the house's own account.
//!
//! **What is a fixture.** Propellant seeded into the tank farm's vault, a stand-in for the
//! deliveries that would fill it; staff's `@tacsim/port haulers` and `@econ/pay`.

use mush_core::{GOD, ObjId, World};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::LotId;
use tacsim::port::hauler::{HaulerHouse, HaulerRow};
use tacsim::services::escort;
use tacsim::services::haul::{self, HaulStage};
use tacsim::stockpile::{self, FreeportArm};

use crate::driving_rig::{
    Rig, cid, drain, hold_pool, ic_now, land_quietly, port_map, run, seed_pool, stocked, store,
    sweep, to_ic,
};

const DAY: i64 = 86_400;

// ---- readings --------------------------------------------------------------------------

fn arm(rig: &Rig) -> FreeportArm {
    stockpile::arm(rig.tw.world()).cloned().unwrap_or_default()
}

fn balance(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .map_or(0, |a| a.posted_balance.minor())
}

fn org_account(rig: &Rig, entry: &str, org: &str) -> AccountId {
    port_map(&rig.tw, entry).orgs[&cid(org)]
        .account
        .map(AccountId)
        .expect("an account")
}

fn farm_vault(rig: &Rig) -> VaultId {
    VaultId(stockpile::tank_farm_vault(rig.tw.world()).expect("the tank farm's vault"))
}

fn contract_state(rig: &Rig, contract: u64) -> ContractState {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract)).map(|c| c.state))
        .expect("the contract")
}

fn hold_clock(rig: &Rig) -> mush_faction::Clock {
    tacsim::concession::hold_clock(rig.tw.world()).expect("Hold the Concession")
}

fn house(rig: &Rig) -> HaulerHouse {
    store(&rig.tw)
        .hauler(&cid("ceres_port"))
        .cloned()
        .expect("Ceres keeps its hauling house")
}

fn vcc01(rig: &Rig) -> HaulerRow {
    house(rig)
        .rows
        .into_iter()
        .find(|row| row.mark == "VCC-01")
        .expect("VCC-01's row")
}

/// A hull's sailing plan's log and its orders' states, for a failure's message.
fn plan_log(rig: &Rig, hull: ObjId) -> String {
    match tacsim::port::orders::orders_state(rig.tw.world(), hull) {
        tacsim::port::orders::OrdersState::Present(doc) => format!(
            "state {:?}, hold {:?}, orders {:?}, log {:?}",
            doc.state,
            doc.hold,
            doc.orders
                .iter()
                .map(|o| (o.id, o.state, format!("{:?}", o.what)))
                .collect::<Vec<_>>(),
            doc.log.iter().map(|l| l.text.clone()).collect::<Vec<_>>()
        ),
        _ => "no plan".to_owned(),
    }
}

/// Port fires (31 s apart), every submission they stage landed.
fn fires(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
}

/// Step the IC clock `secs` on in one jump, then a few fires landed.
fn ahead(rig: &mut Rig, secs: i64) {
    let now = ic_now(&rig.tw);
    let _ = to_ic(rig, now + secs);
    fires(rig, 3);
}

/// Fires until the arm has made `n` posts.
fn until_posts(rig: &mut Rig, n: usize) -> FreeportArm {
    for _ in 0..60 {
        if arm(rig).posts.len() >= n {
            return arm(rig);
        }
        fires(rig, 1);
    }
    panic!("the arm never made {n} posts: {:?}", arm(rig));
}

// ---- the stories ---------------------------------------------------------------------------

/// **Doc 50 cut OP.4: the Freeport arm posts for Occator** (Q-50-22, Q-50-23): its agent makes
/// a haul of propellant from Occator's vault at Ceres into the tank farm's, and an escort for it,
/// both paid from Occator's account; a day later a second; never more than two open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_the_freeport_arm_posts_a_haul_and_its_escort_for_occator() {
    let mut rig = stocked();
    let occator = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].org;
    let farm = port_map(&rig.tw, "isidis_port").orgs[&cid("isidis_tank_farm")].org;
    let first = until_posts(&mut rig, 1);
    assert!(first.agent.is_some() && first.grant.is_some(), "{first:?}");
    let level = first.level.expect("the farm's level is read");
    assert!(level < stockpile::STOCKPILE_THRESHOLD_T, "{level}");
    assert_eq!(
        tacsim::operations::close_probe(rig.tw.world(), stockpile::TANK_FARM_PROBE, 1),
        Some(false),
        "short of its threshold, the probe is not met"
    );
    let post = first.posts[0].clone();
    let job = haul::haul_of(rig.tw.world(), post.haul).expect("the haul");
    assert_eq!(job.shipper, ObjId(first.agent.expect("the agent")));
    assert_eq!(job.shipper_org, Some(occator), "posted for Occator");
    assert_eq!(
        (job.good.as_str(), job.quantity),
        (stockpile::STOCKPILE_GOOD, stockpile::ARM_HAUL_T)
    );
    assert_eq!(
        Some(job.source),
        tacsim::services::org_vault(rig.tw.world(), mush_org::OrgId(occator), "ceres"),
        "from Occator's vault at Ceres"
    );
    assert_eq!(
        Some(job.drop),
        tacsim::services::org_vault(rig.tw.world(), mush_org::OrgId(farm), "isidis"),
        "into the tank farm's at Isidis"
    );
    let escort_contract = post.escort.expect("an escort with it");
    let guard = escort::escort_of(rig.tw.world(), escort_contract).expect("the escort");
    assert_eq!(guard.haul, Some(post.haul), "posted for the haul");
    assert_eq!(guard.poster_org, Some(occator));
    assert_eq!(
        post.spent_minor,
        (stockpile::ARM_HAUL_FEE_CR + stockpile::ARM_ESCORT_FEE_CR) * 100
    );

    // One a day: nothing more today; tomorrow a second; then the two open are the most.
    fires(&mut rig, 6);
    assert_eq!(arm(&rig).posts.len(), 1, "one post an IC day");
    ahead(&mut rig, DAY);
    let _ = until_posts(&mut rig, 2);
    ahead(&mut rig, DAY);
    fires(&mut rig, 6);
    assert_eq!(
        arm(&rig).posts.len(),
        stockpile::ARM_MAX_OPEN,
        "at most two of the arm's hauls open at once: {:?}",
        arm(&rig)
    );
}

/// **Doc 50 cut OP.4: the farm drains, and at its threshold Hold the Concession moves**
/// (Q-50-21, Q-50-60): 20 t an IC day are used up; seeded past its threshold, the next reading
/// meets the close probe and moves Hold once that month, its line the chronicle's; the arm posts
/// nothing while the farm stands there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_the_tank_farm_drains_and_at_its_threshold_hold_advances() {
    let mut rig = stocked();
    let vault = farm_vault(&rig);
    let _ = until_posts(&mut rig, 1);
    let start = hold_pool(&mut rig.tw, vault, stockpile::STOCKPILE_GOOD);
    ahead(&mut rig, DAY);
    ahead(&mut rig, 600);
    let after = hold_pool(&mut rig.tw, vault, stockpile::STOCKPILE_GOOD);
    assert_eq!(start - after, 20, "a day's use of propellant at the farm");

    let before = hold_clock(&rig);
    seed_pool(
        &mut rig,
        vault,
        stockpile::STOCKPILE_GOOD,
        stockpile::STOCKPILE_THRESHOLD_T,
        LotId(9_961),
    );
    let posts = arm(&rig).posts.len();
    ahead(&mut rig, DAY);
    ahead(&mut rig, 600);
    let level = arm(&rig).level.expect("a reading");
    assert!(level >= stockpile::STOCKPILE_THRESHOLD_T, "{level}");
    assert_eq!(
        tacsim::operations::close_probe(rig.tw.world(), stockpile::TANK_FARM_PROBE, 1),
        Some(true),
        "at its threshold, the probe is met"
    );
    let moved = hold_clock(&rig);
    assert_eq!(
        moved.filled,
        before.filled + 1,
        "Hold the Concession moves: {moved:?}"
    );
    let month = tacsim::concession::ic_month(ic_now(&rig.tw));
    assert!(
        moved
            .causes
            .iter()
            .any(|c| c == &format!("{}{month}", stockpile::THRESHOLD_CAUSE)),
        "{:?}",
        moved.causes
    );
    // Another reading the same month moves it no further, and the arm posts nothing.
    ahead(&mut rig, DAY);
    ahead(&mut rig, 600);
    assert_eq!(hold_clock(&rig).filled, moved.filled, "once a month");
    assert_eq!(
        arm(&rig).posts.len(),
        posts,
        "nothing posted at the threshold"
    );
}

/// **Doc 50 cut OP.4: an untaken haul is carried by the hauling house** (Q-50-24): seven IC days
/// after the arm posts it, Ceres' house takes it in VCC-01, flies it by Stickney to Isidis and
/// delivers it into the tank farm; the contract settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_an_untaken_haul_is_carried_by_the_hauling_house() {
    let mut rig = stocked();
    let first = until_posts(&mut rig, 1).posts[0].haul;
    ahead(&mut rig, stockpile::FALLBACK_IC_DAYS * DAY);
    assert_eq!(
        haul::haul_of(rig.tw.world(), first).map(|j| j.stage),
        Some(HaulStage::Posted),
        "no player took it"
    );
    let said = run(&mut rig.tw, GOD, "@tacsim/port haulers ceres_port on");
    assert!(said.contains("ceres_port hauling house: open"), "{said}");
    let mut taken = None;
    for _ in 0..240 {
        fires(&mut rig, 1);
        if let Some(job) = haul::hauls(rig.tw.world())
            .into_iter()
            .find(|j| stockpile::is_arm_haul(rig.tw.world(), j.contract) && j.hull.is_some())
        {
            taken = Some(job);
            break;
        }
    }
    let job = taken.unwrap_or_else(|| panic!("the house never took a haul: {:?}", house(&rig)));
    let row = vcc01(&rig);
    assert_eq!(job.hull.map(|h| h.0), row.hull, "in Ceres' house hull");
    assert_eq!(
        row.trip.as_ref().and_then(|t| t.haul),
        Some(job.contract),
        "{row:?}"
    );
    // She flies it there: by Stickney, then to Isidis, and delivers.
    for _ in 0..(30 * 24) {
        ahead(&mut rig, 3_600);
        let stage = haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage);
        if matches!(stage, Some(HaulStage::Delivered | HaulStage::Closed)) {
            break;
        }
    }
    let done = haul::haul_of(rig.tw.world(), job.contract).expect("the haul");
    assert!(
        matches!(done.stage, HaulStage::Delivered | HaulStage::Closed),
        "{done:?}\n{:?}\n{}",
        house(&rig),
        plan_log(&rig, ObjId(vcc01(&rig).hull.expect("a hull")))
    );
    assert!(
        matches!(
            contract_state(&rig, job.contract),
            ContractState::Settled { .. }
        ),
        "delivered, not lapsed: {:?}",
        contract_state(&rig, job.contract)
    );
}

/// **Doc 50 cut OP.4: a house hull at the picket pays from the house** (Q-50-56): Ceres' hauler,
/// laden with water for Stickney, heaves to at Kirkwood under her house's mind; the transit duty
/// on her cargo comes out of the house's own account, not her helm's purse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_a_house_hull_pays_the_kirkwood_duty_from_the_house() {
    let mut rig = stocked();
    let said = run(&mut rig.tw, GOD, "@tacsim/port haulers ceres_port on");
    assert!(said.contains("ceres_port hauling house: open"), "{said}");
    // The Habitat Board cannot pay for water: Ceres's water goes to Kasei at Stickney.
    let board = org_account(&rig, "luna_port", "habitat_board");
    let treasury = port_map(&rig.tw, "luna_port").treasury.expect("a treasury");
    let all = balance(rig.tw.world(), board);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={treasury} {all}", board.0),
    );
    let day = ic_now(&rig.tw).div_euclid(DAY) + 1;
    let _ = to_ic(&mut rig, day * DAY + 600);
    let mut row = None;
    for _ in 0..240 {
        let r = vcc01(&rig);
        if r.trip.is_some() {
            row = Some(r);
            break;
        }
        fires(&mut rig, 1);
    }
    let row = row.expect("VCC-01 sets out");
    assert_eq!(
        row.trip.as_ref().map(|t| t.to.as_str()),
        Some("stickney"),
        "{row:?}"
    );
    let hull = ObjId(row.hull.expect("a hull"));
    let purse = AccountId(row.account.expect("a purse"));
    let vesta = org_account(&rig, "ceres_port", "vesta_carriers");
    let customs = org_account(&rig, "stickney_port", "republic_customs");
    let mut met = false;
    let (mut house_at, mut purse_at, mut customs_at) = (0, 0, 0);
    for _ in 0..(30 * 24) {
        house_at = balance(rig.tw.world(), vesta);
        purse_at = balance(rig.tw.world(), purse);
        customs_at = balance(rig.tw.world(), customs);
        ahead(&mut rig, 3_600);
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("the bubble book");
        if book
            .active_for_hull(hull)
            .is_some_and(|a| a.picket.is_some())
        {
            met = true;
            break;
        }
    }
    assert!(met, "she meets the Kirkwood picket");
    // In the bubble a tick a wall second: her house's mind brakes her against the corvette,
    // heaves to and submits her to inspection.
    let mut paid = 0;
    for _ in 0..900 {
        sweep(&mut rig.tw, 1);
        let queued = drain(&mut rig.tw);
        land_quietly(&mut rig, &queued);
        paid = balance(rig.tw.world(), customs) - customs_at;
        if paid > 0 {
            break;
        }
    }
    assert!(paid > 0, "the duty is paid to Republic Customs");
    assert_eq!(
        house_at - balance(rig.tw.world(), vesta),
        paid,
        "from the house's own account"
    );
    assert_eq!(
        balance(rig.tw.world(), purse),
        purse_at,
        "not from her helm's purse"
    );
}
