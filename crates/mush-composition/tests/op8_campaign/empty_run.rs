//! **Doc 50 cut OP.8: the empty run** (Q-50-47: "one run with nobody connected: both fronts move
//! and the operation resolves"; doc 11 §6's "no faction-head bottleneck" test).
//!
//! Nobody is connected and nobody types a line after the rig's setup. The world runs on its own
//! causes at the production rate:
//! - the Assembly votes once an IC month, and the Concession Review fills from two of six;
//! - the declaring arm declares the Isidis operation through the campaign's frame and activates
//!   it, and the campaign gives notice, opens the window and reads the tank farm at the close;
//! - the Freeport arm posts its hauls for Occator, and Ceres' hauling house carries those no
//!   player takes, through the Kirkwood picket, which her house's mind answers;
//! - the campaign settles who won, and the arm writes it into control and the import duty.
//!
//! **What is a fixture.** The rig's setup alone: the stock rig's world, the Freeport arm and Ceres'
//! house opened again (the stock rig lays them up; a production world has both open) and the rate
//! set to 24:1.

use std::collections::BTreeSet;

use mush_core::{GOD, ObjId};
use tacsim::concession::{ASSEMBLY_CAUSE, hold_clock, review_clock};
use tacsim::operations::{OperationOutcome, OperationPhase};
use tacsim::services::haul::{self, HaulStage};
use tacsim::stockpile;

use crate::driving_rig::ic_now;
use crate::rig::{DAY, cid, composed, house_hulls, status, step};

/// The longest the run may take, IC days: four votes, the notice and the window are 176.
const MAX_DAYS: i64 = 240;
/// One step, IC seconds: six IC hours, a quarter real hour at 24:1.
const STEP: i64 = 6 * 3_600;

/// **Doc 50 cut OP.8: the empty run** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one run from the first vote to the control write
async fn op8_with_nobody_connected_the_review_fills_and_the_operation_resolves() {
    let mut rig = composed();
    let w = rig.tw.world();
    assert!(
        w.iter().all(|o| o.id == GOD || !w.is_connected(o.id)),
        "no player is connected"
    );
    let start = ic_now(&rig.tw);
    let review0 = review_clock(w).expect("the Review");
    let hold0 = hold_clock(w).expect("Hold the Concession");
    assert_eq!((review0.filled, hold0.filled), (2, 2), "both fronts at two");

    let mut timeline: Vec<String> = Vec::new();
    let mut phases: Vec<OperationPhase> = Vec::new();
    let mut levels: Vec<(i64, u64)> = Vec::new();
    let mut crossings: BTreeSet<(ObjId, i64, String)> = BTreeSet::new();
    let mut delivered: BTreeSet<u64> = BTreeSet::new();
    let mut taken: BTreeSet<u64> = BTreeSet::new();
    let mut review_filled = review0.filled;
    let mut hold_filled = hold0.filled;
    while ic_now(&rig.tw) - start < MAX_DAYS * DAY {
        let met = step(&mut rig, STEP);
        let w = rig.tw.world();
        let day = (ic_now(&rig.tw) - start).div_euclid(DAY);
        let review = review_clock(w).expect("the Review");
        if review.filled != review_filled {
            review_filled = review.filled;
            timeline.push(format!("day {day}: the Review at {}", review.filled));
        }
        let hold = hold_clock(w).expect("Hold the Concession");
        if hold.filled != hold_filled {
            hold_filled = hold.filled;
            timeline.push(format!("day {day}: Hold at {}", hold.filled));
        }
        if let Some(s) = status(w)
            && phases.last() != Some(&s.phase)
        {
            phases.push(s.phase);
            timeline.push(format!("day {day}: the operation {:?}", s.phase));
        }
        if let Some(level) = stockpile::arm(w).and_then(|a| a.level)
            && levels.last().is_none_or(|(_, l)| *l != level)
        {
            levels.push((day, level));
        }
        let hulls = house_hulls(&rig);
        for encounter in met {
            if hulls.contains(&encounter.hull) {
                crossings.insert((encounter.hull, day, format!("{:?}", encounter.outcome)));
            }
        }
        for job in haul::hauls(w) {
            if !stockpile::is_arm_haul(w, job.contract) {
                continue;
            }
            if job.hull.is_some_and(|h| hulls.contains(&h)) {
                taken.insert(job.contract);
                if matches!(job.stage, HaulStage::Delivered | HaulStage::Closed) {
                    delivered.insert(job.contract);
                }
            }
        }
        if tacsim::isidis_op::record(w).is_some_and(|r| r.outcome.is_some()) {
            timeline.push(format!("day {day}: the outcome written"));
            break;
        }
    }
    let w = rig.tw.world();
    let report = format!(
        "{timeline:#?}\nlevels {levels:?}\ncrossings {crossings:?}\ntaken {taken:?}\ndelivered \
         {delivered:?}\nrecord {:?}\nstatus {:?}",
        tacsim::isidis_op::record(w),
        status(w)
    );
    eprintln!("{report}");

    // The Review filled on the Assembly's votes alone.
    let review = review_clock(w).expect("the Review");
    assert!(review.completed, "the Review filled: {report}");
    assert!(
        review
            .causes
            .iter()
            .all(|cause| cause.starts_with(ASSEMBLY_CAUSE)),
        "every segment an Assembly vote: {:?}",
        review.causes
    );

    // The arm declared, the campaign ran it to its close and settled it, and the arm wrote it.
    let record = tacsim::isidis_op::record(w).expect("the arm's record");
    assert!(record.operation.is_some(), "declared: {report}");
    let s = status(w).expect("the campaign's operation");
    assert_eq!(s.phase, OperationPhase::Settled, "{report}");
    for phase in [
        OperationPhase::Notice,
        OperationPhase::Running,
        OperationPhase::Settled,
    ] {
        assert!(phases.contains(&phase), "{phase:?} in {phases:?}");
    }
    let outcome = s.outcome.expect("an outcome");

    // The house carried the arm's hauls, and her house's mind answered the picket.
    assert!(!taken.is_empty(), "the house took an arm's haul: {report}");
    assert!(
        !delivered.is_empty(),
        "the house delivered into the farm: {report}"
    );
    assert!(
        crossings.iter().any(|(_, _, o)| o.contains("Cleared")),
        "a house hull cleared the picket: {report}"
    );

    // The outcome, into control and the import duty.
    let republic = tacsim::polity::polity(w, "republic").expect("the Republic");
    let freeport = tacsim::polity::polity(w, "freeport").expect("the Freeport");
    let control = tacsim::polity::control_at(w, &cid("isidis_port")).expect("Isidis");
    let terms = tacsim::polity::terms_at(w, &cid("isidis_port")).expect("Isidis's terms");
    match outcome {
        OperationOutcome::DeclarerSucceeded => {
            assert_eq!(record.outcome.as_deref(), Some("republic"));
            assert_eq!(control.de_facto, Some(republic), "{report}");
            let row = terms.row.as_ref().expect("the Republic's terms row");
            assert!(row.import_duty_permille > 0, "the import duty: {row:?}");
        }
        OperationOutcome::DeclarerFailed => {
            assert_eq!(record.outcome.as_deref(), Some("freeport"));
            assert_eq!(control.de_facto, Some(freeport), "{report}");
        }
        other => panic!("neither side won: {other:?}\n{report}"),
    }
}
