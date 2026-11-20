//! **Doc 48 cut EX.6 — the haul** (Q-48-42, -57, -58; doc 48b W-48-3b, W-48-3c), over the
//! composed world (`driving`'s rig).
//!
//! - **The post.** A shipper at Ceres posts six tonnes of metals from their locker to Stickney.
//!   A declared value over the reference price is refused; the fee is escrowed; the job is on
//!   Ceres's board as a haul.
//! - **The load.** A hauler docked at Ceres takes it: the bond (a quarter of the declared value)
//!   is escrowed, and the goods leave the locker and land in her hold held for the job, so none
//!   of it is hers to sell. A restart keeps the job.
//! - **The delivery.** She sails to Stickney and hands the goods over with the delivery verb: they
//!   land free in the shipper's Stickney locker, and the sweep pays her the fee and her bond back.
//! - **The deadline.** A second haul taken and never delivered: when its term ends the contract
//!   expires, the bond goes to the shipper with the fee back, and the goods still aboard stay held
//!   until the return leg lands them in the shipper's locker at the hull's port.
//!
//! **What is a fixture.** Money by `@econ/mint` and the shipper's stock by staff's seed, as
//! EX.5's stories.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore, VaultId};
use mush_market::MarketId;
use mush_org::PrincipalId;
use tacsim::services::haul::{self, HaulStage};

use crate::driving_rig::{
    Rig, advance_to, cid, file_and_launch, fire, hold_vault_of, land_all, load, pilot, port_map,
    run, seed_pool, station_room, stocked, teleport, to_ic,
};

const CERES: &str = "ceres_port";

fn account(rig: &Rig, who: ObjId) -> AccountId {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&PrincipalId::Object(who))
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("a credit account")
}

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

fn mint(rig: &mut Rig, to: AccountId, minor: i64) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", to.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

/// A pool's quantity and what of it is free (not held).
fn pool(rig: &mut Rig, vault: VaultId, good: &str) -> (u64, u64) {
    let families = rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory")
        .0
        .clone();
    let mut txn = rig
        .tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("an inventory transaction");
    families
        .pool(&mut txn, mush_inventory::EndpointRef(vault), &cid(good))
        .expect("the pool reads")
        .map_or((0, 0), |p| (p.quantity, p.available))
}

fn locker(rig: &Rig, who: ObjId, venue: &str) -> VaultId {
    let venue = MarketId::new(venue).expect("a venue");
    tacsim::port::locker::locker_of(rig.tw.world(), who, &venue)
        .map(|l| VaultId(l.vault))
        .expect("a locker")
}

fn state(rig: &Rig, contract: u64) -> ContractState {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract)).map(|c| c.state))
        .expect("the contract")
}

/// The newest haul the shipper posted.
fn newest(rig: &Rig, shipper: ObjId) -> haul::HaulJob {
    haul::hauls(rig.tw.world())
        .into_iter()
        .filter(|j| j.shipper == shipper)
        .max_by_key(|j| j.contract)
        .expect("a haul")
}

/// The shipper with a credit account and six tonnes of metals in a Ceres locker.
fn shipper(rig: &mut Rig) -> ObjId {
    let who = pilot(rig, "Shipper", 61);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}={}", who.0, tacsim::CREDIT),
    );
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, who, exchange);
    let opened = run(&mut rig.tw, who, "+locker/open");
    assert!(opened.contains("is open"), "{opened}");
    let vault = locker(rig, who, "ceres");
    seed_pool(rig, vault, "metals", 12, mush_inventory::LotId(0xe6a1));
    let wallet = account(rig, who);
    mint(rig, wallet, 1_000_000);
    who
}

/// Post one haul of six tonnes to Stickney for 500 cr; the job.
fn post(rig: &mut Rig, who: ObjId) -> haul::HaulJob {
    let out = run(
        &mut rig.tw,
        who,
        "+haul/post 6 metals to=stickney fee=500 go",
    );
    assert!(out.contains("posted: 6 "), "{out}");
    newest(rig, who)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex6_a_haul_carries_held_cargo_and_pays_on_the_delivery_marker() {
    let mut rig = stocked();
    let shipper = shipper(&mut rig);
    let wallet = account(&rig, shipper);
    let ceres_locker = locker(&rig, shipper, "ceres");

    // ---- The post: the cap on declared value, then the job ----
    let preview = run(
        &mut rig.tw,
        shipper,
        "+haul/post 6 metals to=stickney fee=500",
    );
    assert!(preview.contains("Post a haul of 6 "), "{preview}");
    let over = run(
        &mut rig.tw,
        shipper,
        "+haul/post 6 metals to=stickney fee=500 value=99999999 go",
    );
    assert!(
        over.contains("A declared value is at most the reference price"),
        "an inflated bond is refused: {over}"
    );
    let before = balance(&rig, wallet);
    let job = post(&mut rig, shipper);
    assert_eq!(job.stage, HaulStage::Posted);
    assert!(job.declared_minor > 0 && job.bond_minor == job.declared_minor / 4);
    assert_eq!(
        before - balance(&rig, wallet),
        50_000,
        "the fee is escrowed"
    );
    let board = run(&mut rig.tw, shipper, "+market/jobs haul");
    assert!(
        board.contains("carry 6 ") && board.contains(&format!("+haul/accept {}", job.contract)),
        "on Ceres's board as a haul: {board}"
    );

    // ---- The load: the hauler docked at Ceres takes it ----
    let (hauler, row) = crate::driving_rig::prospector(&mut rig, "Hauler", 62, "Wren");
    let hauler_wallet = account(&rig, hauler);
    mint(&mut rig, hauler_wallet, 1_000_000);
    let hold = hold_vault_of(&rig.tw, &row);
    let own = run(
        &mut rig.tw,
        shipper,
        &format!("+haul/accept {} go", job.contract),
    );
    assert!(own.contains("That haul is your own."), "{own}");
    let preview = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {}", job.contract),
    );
    assert!(preview.contains("Take the haul: carry 6 "), "{preview}");
    let before = balance(&rig, hauler_wallet);
    let taken = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {} go", job.contract),
    );
    assert!(taken.contains("You take haul"), "{taken}");
    let queued = crate::driving_rig::drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one load leg: {taken}");
    land_all(&mut rig, &queued);
    assert_eq!(
        before - balance(&rig, hauler_wallet),
        job.bond_minor,
        "the bond is escrowed at the join"
    );
    assert_eq!(
        pool(&mut rig, ceres_locker, "metals"),
        (6, 6),
        "six left the locker"
    );
    assert_eq!(
        pool(&mut rig, hold, "metals"),
        (6, 0),
        "aboard and held: none of it is the hauler's to sell or burn"
    );
    let _ = fire(&mut rig);
    assert_eq!(
        haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage),
        Some(HaulStage::Aboard)
    );
    // A restart keeps the job and the hold.
    let mut rig = crate::driving_rig::reboot(rig, &[shipper, hauler]);
    assert_eq!(
        haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage),
        Some(HaulStage::Aboard),
        "the record reads back"
    );
    assert_eq!(pool(&mut rig, hold, "metals"), (6, 0), "still held");

    // ---- The delivery: sail to Stickney and hand over ----
    let early = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {} go", job.contract),
    );
    assert!(
        early.contains("not docked at"),
        "only at the destination: {early}"
    );
    load(&mut rig, &row, 60, 40, 0xe6b0);
    crate::driving_rig::board(&mut rig, hauler);
    let hull = ObjId(row.hull);
    let ticket = file_and_launch(&mut rig, hauler, hull, "+ship/undock stickney");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let _ = to_ic(
        &mut rig,
        ticket.plan.depart_ic + ticket.plan.total_s + 3_600,
    );
    let stickney_locker = locker(&rig, shipper, "stickney");
    let stranger = pilot(&mut rig, "Stranger", 63);
    let theirs = run(
        &mut rig.tw,
        stranger,
        &format!("+haul/deliver {} go", job.contract),
    );
    assert!(theirs.contains("You carry no goods"), "{theirs}");
    let handed = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {} go", job.contract),
    );
    assert!(handed.contains("are handed over"), "{handed}");
    let queued = crate::driving_rig::drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one delivery leg");
    land_all(&mut rig, &queued);
    assert_eq!(
        pool(&mut rig, hold, "metals"),
        (0, 0),
        "nothing left aboard"
    );
    assert_eq!(
        pool(&mut rig, stickney_locker, "metals"),
        (6, 6),
        "free in the shipper's Stickney locker"
    );
    let before = balance(&rig, hauler_wallet);
    // The inventory mirror refreshes on its minute sweep; the economy's sweep then settles.
    for _ in 0..3 {
        crate::driving_rig::sweep(&mut rig.tw, 60);
        crate::in10_pools::settle(&mut rig, 1);
    }
    assert!(
        matches!(state(&rig, job.contract), ContractState::Settled { .. }),
        "the marker settles the job: {:?}",
        state(&rig, job.contract)
    );
    assert_eq!(
        balance(&rig, hauler_wallet) - before,
        50_000 + job.bond_minor,
        "the fee and the bond back"
    );
    assert_eq!(
        haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage),
        Some(HaulStage::Closed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex6_cargo_still_aboard_at_the_deadline_stays_held_and_goes_back() {
    let mut rig = stocked();
    let shipper = shipper(&mut rig);
    let wallet = account(&rig, shipper);
    let ceres_locker = locker(&rig, shipper, "ceres");
    let job = post(&mut rig, shipper);
    let (hauler, row) = crate::driving_rig::prospector(&mut rig, "Laggard", 64, "Stint");
    let hauler_wallet = account(&rig, hauler);
    mint(&mut rig, hauler_wallet, 1_000_000);
    let hold = hold_vault_of(&rig.tw, &row);
    let taken = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {} go", job.contract),
    );
    assert!(taken.contains("You take haul"), "{taken}");
    let queued = crate::driving_rig::drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let _ = fire(&mut rig);
    let job = haul::haul_of(rig.tw.world(), job.contract).expect("the job");
    assert_eq!(job.stage, HaulStage::Aboard);
    let shipper_before = balance(&rig, wallet);

    // The term ends with the goods aboard: they stay aboard, held for the job.
    let due = job.due_ic.expect("a term");
    let (_, queued) = advance_to(&mut rig, due + 60);
    assert_eq!(
        pool(&mut rig, hold, "metals"),
        (6, 0),
        "at the deadline the cargo is still aboard and still held"
    );
    land_all(&mut rig, &queued);
    // The contract expires (the fee back and the bond to the shipper), and at the hull's port
    // (she never left Ceres) the return leg lands the goods back in the shipper's locker, free.
    crate::in10_pools::settle(&mut rig, 4);
    assert!(
        matches!(state(&rig, job.contract), ContractState::Expired),
        "an unperformed haul expires: {:?}",
        state(&rig, job.contract)
    );
    // The fee and the bond, less the locker's own charges over the term.
    let gained = balance(&rig, wallet) - shipper_before;
    assert!(
        gained > 50_000 + job.bond_minor - 10_000 && gained <= 50_000 + job.bond_minor,
        "the fee back and the bond to the shipper: {gained}"
    );
    let late = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {} go", job.contract),
    );
    assert!(
        !late.contains("are handed over"),
        "no delivery after the term: {late}"
    );
    assert_eq!(
        pool(&mut rig, ceres_locker, "metals"),
        (12, 12),
        "the goods are back in the shipper's Ceres locker"
    );
    assert_eq!(
        pool(&mut rig, hold, "metals"),
        (0, 0),
        "nothing left aboard"
    );
    assert_eq!(
        haul::haul_of(rig.tw.world(), job.contract).map(|j| j.stage),
        Some(HaulStage::Closed)
    );
    let told = run(&mut rig.tw, shipper, "+haul");
    assert!(told.contains("You have no hauls."), "{told}");
}

/// **Doc 50 cut KC.3: the job board names the duty** (Q-50-8). A haul from Ceres to Stickney
/// crosses the Kirkwood picket; under the Co-op's hold its row carries no duty, and once the
/// Republic holds the approach it shows the duty its hauler will meet: 6 t of metals is 1,200 cr
/// at catalog, 2% of it is under the 50 cr floor, so 50 cr.
#[test]
fn kc3_the_job_board_shows_the_duty_a_hauler_will_meet() {
    let mut rig = stocked();
    let shipper = shipper(&mut rig);
    let _ = post(&mut rig, shipper);
    let board = run(&mut rig.tw, shipper, "+market/jobs haul");
    assert!(board.contains("carry 6 "), "{board}");
    assert!(
        !board.contains("Kirkwood duty"),
        "no duty under the Co-op: {board}"
    );

    let depot = mush_core::CanonicalId::new("kirkwood_depot").expect("an id");
    let region = tacsim::polity::control_at(rig.tw.world(), &depot)
        .expect("Kirkwood is seeded")
        .region;
    let republic = tacsim::polity::polity(rig.tw.world(), "republic").expect("the Republic");
    crate::support::with_ctx(&mut rig.tw, |ctx| {
        mush_faction::write_control(
            ctx,
            mush_faction::ControlWrite::new(region, republic, mush_faction::ControlKind::DeFacto),
        )
    })
    .expect("the flip");
    let board = run(&mut rig.tw, shipper, "+market/jobs haul");
    let row = board
        .lines()
        .find(|l| l.contains("carry 6 "))
        .expect("the haul's row");
    assert!(row.contains("(Kirkwood duty ~50 cr)"), "{board}");
    assert!(row.chars().count() <= 120, "{row}");
}
