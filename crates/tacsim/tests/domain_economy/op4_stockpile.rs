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
//!   by the Kirkwood route, and delivered into the tank farm. Doc 50b item 1: she sails home to
//!   Ceres on the stores she filled for the leg back, and carries a second haul.
//! - **A house hull at the picket** pays the transit duty from the house's own account.
//!
//! **What is a fixture.** Propellant seeded into the tank farm's vault, a stand-in for the
//! deliveries that would fill it; staff's `@tacsim/port haulers`, `@econ/pay` and
//! `@faction/control` (the Republic holding the approach, as KC.1's fill makes it).

use std::sync::{Arc, Mutex};

use mush_core::{Ctx, GOD, ObjId, World};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::LotId;
use tacsim::operations::{
    OperationFrame, OperationFrameSlot, OperationOutcome, OperationPhase, OperationRequest,
    OperationStatus,
};
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

/// The stock rig with the Freeport arm opened again (`@tacsim/port arm on`): the stock rig lays
/// it up, as it does the hauling house.
fn opened() -> Rig {
    let mut rig = stocked();
    let said = run(&mut rig.tw, GOD, "@tacsim/port arm on");
    assert!(said.contains("Freeport arm: open"), "{said}");
    rig
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
    let mut rig = opened();
    let occator = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].org;
    let farm = port_map(&rig.tw, "isidis_port").orgs[&cid("isidis_tank_farm")].org;
    let first = until_posts(&mut rig, 1);
    assert!(first.agent.is_some() && first.grant.is_some(), "{first:?}");
    let level = first.level.expect("the farm's level is read");
    assert!(level < stockpile::STOCKPILE_THRESHOLD_T, "{level}");
    assert_eq!(
        tacsim::operations::close_probe(rig.tw.world(), stockpile::TANK_FARM_PROBE, 1),
        None,
        "no operation has closed: the probe waits (doc 50b item 3)"
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
    let mut rig = opened();
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
/// delivers it into the tank farm; the contract settles. Then (doc 50b item 1) she sails home by
/// Stickney to Ceres and takes another of the arm's hauls.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_an_untaken_haul_is_carried_by_the_hauling_house() {
    let mut rig = opened();
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
    // She flies it there: by Stickney, then to Isidis, and delivers. Where she lay, each time it
    // changed, is kept for a failure's message.
    let hull = ObjId(row.hull.expect("a hull"));
    let mut whereabouts: Vec<String> = Vec::new();
    for _ in 0..(30 * 24) {
        ahead(&mut rig, 3_600);
        let here = match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
            tacsim::port::ship::Berth::Docked { def, .. } => format!("docked at {}", def.venue),
            tacsim::port::ship::Berth::UnderWay => "under way".to_owned(),
            tacsim::port::ship::Berth::Elsewhere => "elsewhere".to_owned(),
        };
        if whereabouts.last().is_none_or(|last| !last.ends_with(&here)) {
            whereabouts.push(format!(
                "IC day {}: {here}",
                ic_now(&rig.tw).div_euclid(DAY)
            ));
        }
        let stage = haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage);
        if matches!(stage, Some(HaulStage::Delivered | HaulStage::Closed)) {
            break;
        }
    }
    let done = haul::haul_of(rig.tw.world(), job.contract).expect("the haul");
    assert!(
        matches!(done.stage, HaulStage::Delivered | HaulStage::Closed),
        "{done:?}\n{whereabouts:?}\n{:?}\n{}",
        house(&rig),
        plan_log(&rig, hull)
    );
    assert!(
        matches!(
            contract_state(&rig, job.contract),
            ContractState::Settled { .. }
        ),
        "delivered, not lapsed: {:?}",
        contract_state(&rig, job.contract)
    );

    // Doc 50b item 1: Isidis sells her no stores, and nothing is bid there that she carries. She
    // filled for the leg home before she left Stickney, so she sails back by Stickney to Ceres,
    // where the trip ends, and carries another of the arm's hauls.
    let delivered_at = whereabouts.len();
    let mut second = None;
    for _ in 0..(40 * 12) {
        ahead(&mut rig, 2 * 3_600);
        let here = match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
            tacsim::port::ship::Berth::Docked { def, .. } => format!("docked at {}", def.venue),
            tacsim::port::ship::Berth::UnderWay => "under way".to_owned(),
            tacsim::port::ship::Berth::Elsewhere => "elsewhere".to_owned(),
        };
        if whereabouts.last().is_none_or(|last| !last.ends_with(&here)) {
            whereabouts.push(format!(
                "IC day {}: {here}",
                ic_now(&rig.tw).div_euclid(DAY)
            ));
        }
        if let Some(next) = haul::hauls(rig.tw.world()).into_iter().find(|j| {
            stockpile::is_arm_haul(rig.tw.world(), j.contract)
                && j.contract != job.contract
                && j.hull.map(|h| h.0) == Some(hull.0)
        }) {
            second = Some(next);
            break;
        }
    }
    let home = &whereabouts[delivered_at..];
    let second = second.unwrap_or_else(|| {
        panic!(
            "she never carried a second haul: {whereabouts:?}
{:?}
{}",
            vcc01(&rig),
            plan_log(&rig, hull)
        )
    });
    assert!(
        home.iter().any(|w| w.ends_with("docked at stickney"))
            && home.iter().any(|w| w.ends_with("docked at ceres")),
        "home by Stickney to Ceres: {whereabouts:?}"
    );
    let row = vcc01(&rig);
    assert!(row.trips >= 1, "the first trip ended: {row:?}");
    assert_eq!(
        row.trip.as_ref().and_then(|t| t.haul),
        Some(second.contract),
        "the second haul is her trip in hand: {row:?}"
    );
}

/// **Doc 50 cut OP.4: a house hull at the picket pays from the house** (Q-50-56): Ceres' hauler,
/// laden with water for Stickney, heaves to at Kirkwood under her house's mind; the transit duty
/// on her cargo comes out of the house's own account, not her helm's purse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_a_house_hull_pays_the_kirkwood_duty_from_the_house() {
    let mut rig = opened();
    // The Republic holds the approach, as Customs Assertion's fill makes it (KC.1): its terms row
    // levies the duty.
    let held = run(
        &mut rig.tw,
        GOD,
        "@faction/control Kirkwood approach=Republic/defacto",
    );
    assert!(!held.contains("Usage"), "{held}");
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

// ---- doc 50b item 3: the close probe reads the farm at the close ------------------------

/// The operation number [`ClosingFrame`] hands out.
const OP: u64 = 11;

/// **A stand-in operation frame** whose window closes at the instant the story sets: running
/// before it, settling after.
#[derive(Default)]
struct ClosingFrame {
    declared: Mutex<Vec<OperationRequest>>,
    close: Mutex<i64>,
}

impl OperationFrame for ClosingFrame {
    fn declare(&self, _ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String> {
        self.declared
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .push(request.clone());
        Ok(OP)
    }

    fn status(&self, w: &World, operation: u64) -> Option<OperationStatus> {
        if operation != OP {
            return None;
        }
        let request = self.declared.lock().ok()?.first()?.clone();
        let close = *self.close.lock().ok()?;
        Some(OperationStatus {
            phase: if w.world_ic_now().at < close {
                OperationPhase::Running
            } else {
                OperationPhase::Settling
            },
            declarer: request.declarer,
            sides: request.sides,
            defenders: request.defenders,
            legal_targets: request.legal_targets,
            window: (0, close),
            outcome: None::<OperationOutcome>,
            told: Vec::new(),
        })
    }
}

/// **Doc 50b item 3: the close probe reads the tank farm at the window's close** (Q-50-21 as
/// corrected: "read once, at the window's close"). The Republic declares; before the close the
/// probe waits. Propellant reaches the farm, enough for its threshold, after the port tick's
/// last reading before the close: the probe still waits, where the cached reading would have
/// answered "not met". On the first fire after the close the farm is read once for the
/// operation, and the probe is met.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op4_the_close_probe_reads_the_farm_at_the_close() {
    let mut rig = stocked();
    let frame = Arc::new(ClosingFrame::default());
    *frame.close.lock().expect("lock") = i64::MAX;
    let seated: Arc<dyn OperationFrame> = frame.clone();
    rig.tw
        .world_mut()
        .module_state_insert(OperationFrameSlot::new(seated));
    let review = tacsim::concession::review_clock(rig.tw.world()).expect("the Review");
    let left = review.segments - review.filled;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@clock/advance {}={left}", review.id.0),
    );
    for _ in 0..6 {
        fires(&mut rig, 1);
        if tacsim::isidis_op::record(rig.tw.world()).and_then(|r| r.operation) == Some(OP) {
            break;
        }
    }
    assert_eq!(
        tacsim::isidis_op::record(rig.tw.world()).and_then(|r| r.operation),
        Some(OP),
        "the operation is declared"
    );
    let close = ic_now(&rig.tw) + DAY / 2;
    *frame.close.lock().expect("lock") = close;
    let probe =
        |rig: &Rig| tacsim::operations::close_probe(rig.tw.world(), stockpile::TANK_FARM_PROBE, OP);
    fires(&mut rig, 2);
    assert!(
        arm(&rig)
            .level
            .is_some_and(|l| l < stockpile::STOCKPILE_THRESHOLD_T),
        "{:?}",
        arm(&rig)
    );
    assert_eq!(probe(&rig), None, "before the close the probe waits");

    // Propellant lands in the farm after the last reading before the close.
    let vault = farm_vault(&rig);
    let level = hold_pool(&mut rig.tw, vault, stockpile::STOCKPILE_GOOD);
    seed_pool(
        &mut rig,
        vault,
        stockpile::STOCKPILE_GOOD,
        stockpile::STOCKPILE_THRESHOLD_T - level + 100,
        LotId(9_963),
    );
    assert_eq!(
        probe(&rig),
        None,
        "no reading at the close yet: the probe waits, never the cached reading"
    );

    // The close passes; the first fire reads the farm once, for this operation.
    let _ = to_ic(&mut rig, close + 60);
    fires(&mut rig, 3);
    let read = arm(&rig).close_read.expect("the farm read at the close");
    assert_eq!(read.operation, OP);
    assert!(read.read_ic >= close, "{read:?} against {close}");
    assert!(read.level >= stockpile::STOCKPILE_THRESHOLD_T, "{read:?}");
    assert_eq!(
        probe(&rig),
        Some(true),
        "at its threshold at the close: met"
    );
    assert_eq!(
        tacsim::operations::close_probe(rig.tw.world(), stockpile::TANK_FARM_PROBE, OP + 1),
        None,
        "another operation's close is not this one's"
    );
}
