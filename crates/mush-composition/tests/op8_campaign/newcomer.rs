//! **Doc 50 cut OP.8: the second acceptance story** (Q-50-47: "a player who never joined arrives
//! at Isidis after the operation, and reads who won from the terms"), on the production command
//! paths with two accounts, and no staff hand in the outcome. **Players decide it** (OP.8's
//! retune): it is the empty run with one player haul more, and that haul keeps the farm at its
//! threshold through the close, so the Freeport holds where the empty run's Republic won.
//!
//! - **The hauler** (the first account), a Co-op pilot with a Dray-P at Ceres, takes the Freeport
//!   arm's first haul for Occator (`+haul/accept`), flies it by the direct run, heaves to at the
//!   Kirkwood picket and submits (`+helm/thrust`, `+helm/submit`), carries on to Stickney and
//!   Isidis and delivers it into the tank farm (`+haul/deliver`). Beside her, Ceres' house
//!   carries the hauls nobody takes; hers is the third the arm keeps open beside the house's
//!   two hulls at Ceres.
//! - **The world runs on.** The Assembly's votes fill the Concession Review; the Republic
//!   declares and the hauler, who has a hull on the lane, is told; the campaign reads the farm at
//!   the window's close, at its threshold, and settles for the Freeport; the hauler is told.
//! - **The newcomer** (the second account) is made only after the operation, arrives at Isidis,
//!   and reads on `+operation` that the Freeport held, and that a sale there pays no import
//!   duty.
//!
//! **What is a fixture.** The rig's setup (as the empty run's), the hauler's propellant and stores
//! aboard at each port (a stand-in for buying them), and the newcomer's arrival at Isidis by
//! teleport. No staff line fills the Review or decides the operation.

use mush_core::ObjId;
use tacsim::concession::{ASSEMBLY_CAUSE, review_clock};
use tacsim::op_page::OPERATION_VERB;
use tacsim::operations::{OperationOutcome, OperationPhase};
use tacsim::port::ship::{Berth, berth_of};
use tacsim::port_tick::pending_notices;
use tacsim::services::haul::{self, HaulStage};
use tacsim::stockpile;

use crate::driving_rig::{
    Rig, board, file_and_launch, ic_now, land_quietly, load, pilot, prospector, run, station_room,
    teleport, to_ic,
};
use crate::rig::{DAY, bubbles, composed, customs_ruling, drain_space, fires, status, step};

/// The longest the run may take, IC days.
const MAX_DAYS: i64 = 240;
/// One step, IC seconds.
const STEP: i64 = 6 * 3_600;

/// Where `hull` lies, in words.
fn berth(rig: &Rig, hull: ObjId) -> String {
    match berth_of(rig.tw.world(), hull) {
        Berth::Docked { def, .. } => format!("docked at {}", def.venue),
        Berth::UnderWay => "under way".to_owned(),
        Berth::Elsewhere => "elsewhere".to_owned(),
    }
}

/// Step until `hull` is docked at `venue`.
fn until_docked(rig: &mut Rig, hull: ObjId, venue: &str) {
    for _ in 0..(40 * 4) {
        if berth(rig, hull) == format!("docked at {venue}") {
            return;
        }
        let _ = step(rig, STEP);
    }
    panic!("never docked at {venue}: {}", berth(rig, hull));
}

/// **Doc 50 cut OP.8: the second acceptance story** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the haul's run, the operation, the newcomer's page
async fn op8_a_player_who_never_joined_reads_who_won_from_the_terms() {
    let mut rig = composed();
    let start = ic_now(&rig.tw);
    let (hauler, row) = prospector(&mut rig, "Hauler", 91, "Brine Moth");
    let hull = ObjId(row.hull);

    // ---- 1. The hauler takes the arm's first haul at Ceres ----
    let mut haul_contract = None;
    for _ in 0..60 {
        if let Some(post) = stockpile::arm(rig.tw.world()).and_then(|a| a.posts.first()) {
            haul_contract = Some(post.haul);
            break;
        }
        fires(&mut rig, 1);
    }
    let haul_contract = haul_contract.expect("the arm posts a haul");
    let took = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/accept {haul_contract} go"),
    );
    assert!(took.contains("You take haul"), "{took}");
    fires(&mut rig, 3);
    let job = haul::haul_of(rig.tw.world(), haul_contract).expect("the haul");
    assert_eq!(
        job.stage,
        HaulStage::Aboard,
        "the goods are aboard: {job:?}"
    );

    // ---- 2. The direct run: heave to at the picket and submit, on to Stickney ----
    load(&mut rig, &row, 60, 40, 0x0c81);
    board(&mut rig, hauler);
    let ticket = file_and_launch(&mut rig, hauler, hull, "+ship/undock stickney");
    let leg1_due = ticket.plan.depart_ic + i64::from(ticket.plan.legs[0].secs);
    let _ = to_ic(&mut rig, leg1_due);
    assert!(
        bubbles(rig.tw.world()) > 0,
        "the picket stops her at Kirkwood"
    );
    let _ = run(&mut rig.tw, hauler, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next = rig.tw.world().now() + 1;
        rig.tw.set_now(next);
        let _ = rig.tw.fire_due(next * 1_000);
    }
    let _ = run(&mut rig.tw, hauler, "+helm/thrust 0");
    let brake = drain_space(&mut rig.tw);
    land_quietly(&mut rig, &brake);
    let ruled = customs_ruling(&mut rig, hauler, "+helm/submit");
    assert!(ruled.contains("clear"), "the picket clears her: {ruled}");
    until_docked(&mut rig, hull, "stickney");

    // ---- 3. Stickney to Isidis, and the delivery into the tank farm ----
    load(&mut rig, &row, 20, 4, 0x0c83);
    let _ = file_and_launch(&mut rig, hauler, hull, "+ship/undock isidis");
    until_docked(&mut rig, hull, "isidis");
    let delivering = run(
        &mut rig.tw,
        hauler,
        &format!("+haul/deliver {haul_contract} go"),
    );
    assert!(!delivering.contains("not docked"), "{delivering}");
    fires(&mut rig, 4);
    let done = haul::haul_of(rig.tw.world(), haul_contract).expect("the haul");
    assert!(
        matches!(done.stage, HaulStage::Delivered | HaulStage::Closed),
        "the hauler's haul is in the farm: {done:?}"
    );

    // She logs off: what the lane tells her now is queued for her login.
    rig.tw.world_mut().set_connected(hauler, false);

    // ---- 4. The world runs on: the Review fills, the operation is declared and settled ----
    let mut phases: Vec<OperationPhase> = Vec::new();
    while ic_now(&rig.tw) - start < MAX_DAYS * DAY {
        let _ = step(&mut rig, STEP);
        let w = rig.tw.world();
        if let Some(s) = status(w)
            && phases.last() != Some(&s.phase)
        {
            phases.push(s.phase);
        }
        if tacsim::isidis_op::record(w).is_some_and(|r| r.outcome.is_some()) {
            break;
        }
    }
    let w = rig.tw.world();
    let review = review_clock(w).expect("the Review");
    assert!(
        review.completed
            && review
                .causes
                .iter()
                .all(|cause| cause.starts_with(ASSEMBLY_CAUSE)),
        "the Review filled on the Assembly's votes alone: {review:?}"
    );
    let s = status(w).expect("the operation");
    assert_eq!(s.phase, OperationPhase::Settled, "{phases:?}");
    let outcome = s.outcome.expect("decided");
    // The hauler's haul kept the farm at its threshold through the close: the Freeport holds.
    let close = stockpile::arm(w)
        .and_then(|a| a.close_read)
        .expect("the farm read at the close");
    assert!(
        close.level >= stockpile::STOCKPILE_THRESHOLD_T,
        "at its threshold at the close: {close:?}"
    );
    assert_eq!(
        outcome,
        OperationOutcome::DeclarerFailed,
        "the Freeport holds"
    );
    let freeport = tacsim::polity::polity(w, "freeport").expect("the Freeport");
    let control = tacsim::polity::control_at(w, &crate::rig::cid("isidis_port")).expect("Isidis");
    assert_eq!(
        control.de_facto,
        Some(freeport),
        "the Freeport holds Isidis"
    );
    // The hauler, with a hull on the lane, has the lane's notices of the declaration and of who
    // won queued for her login.
    let queued = pending_notices(rig.tw.world(), hauler);
    assert!(
        queued
            .iter()
            .filter(|n| n.contains("Official notice"))
            .count()
            >= 2,
        "the declaration and the resolution: {queued:?}"
    );

    // ---- 5. The newcomer arrives at Isidis after the operation and reads who won ----
    let newcomer = pilot(&mut rig, "Newcomer", 92);
    let market = station_room(&rig.tw, "isidis_port", "room.i03_market");
    teleport(&mut rig.tw, newcomer, market);
    assert!(
        pending_notices(rig.tw.world(), newcomer).is_empty(),
        "the newcomer never joined, and was told nothing"
    );
    let page = run(&mut rig.tw, newcomer, OPERATION_VERB);
    for want in [
        "Decided: the Freeport held the Concession",
        "Isidis now: the Freeport administers the Concession",
        "cargo sold at Isidis pays no import duty",
    ] {
        assert!(page.contains(want), "{want:?} in {page}");
    }
}
