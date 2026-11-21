//! **Doc 50 cut OP.7 — `+operation`, the notices and the board's tags** (Q-50-30), over the
//! composed world (`driving`'s rig).
//!
//! TacSim has no campaign edge, so a **fake frame** stands where the composition root seats the
//! campaign's (TacSim's own `OperationFrame` trait), as OP.5's arm story does: it records the
//! declaration, and the story sets its phase and outcome. The real frame is OP.0's.
//!
//! - **Before**, a newcomer's `+operation` says nothing is declared, shows the Review short of
//!   full, and says the Freeport administers Isidis, where cargo pays no import duty.
//! - **The Review fills and the Republic declares.** A pilot with a hull on the lane has the
//!   official notice at once; the newcomer, with no hull, has none. `+operation` shows who
//!   declared against whom, the stake as each side's terms, the notice and the window in IC and
//!   real time, both clocks and the tank farm's level.
//! - **The Freeport arm's work** is on the page, and the job board tags it as the operation's.
//! - **The Republic wins.** The pilot has the resolution's notice, naming the duty. The newcomer,
//!   who never joined, reads on `+operation` that the Republic won and what it puts on a sale
//!   there; the board's tag is gone.
//!
//! **What is a fixture.** The fake frame, its phase and its outcome; staff's `@clock/advance` to
//! fill the Review and `@tacsim/port arm on` to open the arm.

use std::sync::{Arc, Mutex};

use mush_core::{Audience, Ctx, GOD, ObjId, World};
use tacsim::op_page::{JOB_TAG, OPERATION_VERB};
use tacsim::operations::{
    OperationFrame, OperationFrameSlot, OperationOutcome, OperationPhase, OperationRequest,
    OperationStatus,
};
use tacsim::port_tick::pending_notices;

use crate::driving_rig::{
    OutputEvent, Rig, cid, drain, land_quietly, pilot, port_map, prospector, reboot, run, said,
    station_room, stocked,
};

/// The operation number the fake frame hands out.
const OP: u64 = 11;

/// **A stand-in for the composition root's frame.**
#[derive(Default)]
struct FakeFrame {
    declared: Mutex<Vec<(OperationRequest, i64)>>,
    phase: Mutex<Option<OperationPhase>>,
    outcome: Mutex<Option<OperationOutcome>>,
    /// Whom the campaign's own notice tells (doc 50b item 9).
    told: Mutex<Vec<ObjId>>,
}

impl OperationFrame for FakeFrame {
    fn declare(&self, ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String> {
        self.declared
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .push((request.clone(), ctx.w.world_ic_now().at));
        Ok(OP)
    }

    fn status(&self, _w: &World, operation: u64) -> Option<OperationStatus> {
        if operation != OP {
            return None;
        }
        let (request, at) = self.declared.lock().ok()?.first()?.clone();
        let open = at + request.notice_secs;
        Some(OperationStatus {
            phase: self.phase.lock().ok()?.unwrap_or(OperationPhase::Notice),
            declarer: request.declarer,
            sides: request.sides,
            defenders: request.defenders,
            legal_targets: request.legal_targets,
            window: (open, open + request.window_secs),
            outcome: *self.outcome.lock().ok()?,
            told: self.told.lock().ok()?.clone(),
        })
    }
}

/// Port fires (31 s apart), every submission landed: what they said.
fn fires(rig: &mut Rig, n: usize) -> Vec<(Audience, OutputEvent)> {
    let mut out = Vec::new();
    for _ in 0..n {
        let t = rig.tw.world().now() + 31;
        rig.tw.set_now(t);
        out.extend(rig.tw.fire_due(t.saturating_mul(1_000)));
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
    out
}

/// What `who` heard of `events` (connected) and has queued (absent), one string.
fn notices(rig: &Rig, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    let mut out = vec![said(&rig.tw, events, who)];
    out.extend(pending_notices(rig.tw.world(), who));
    out.join("\n")
}

/// **Doc 50 cut OP.7: following the operation** (Q-50-30).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op7_the_lane_follows_the_operation_and_a_newcomer_reads_who_won() {
    let mut rig = stocked();
    let frame = Arc::new(FakeFrame::default());
    let seated: Arc<dyn OperationFrame> = frame.clone();
    rig.tw
        .world_mut()
        .module_state_insert(OperationFrameSlot::new(seated));
    let opened = run(&mut rig.tw, GOD, "@tacsim/port arm on");
    assert!(opened.contains("Freeport arm: open"), "{opened}");
    let (hauler, _row) = prospector(&mut rig, "Hauler", 91, "Brine Moth");
    let newcomer = pilot(&mut rig, "Newcomer", 92);
    // Doc 50b item 9: a pilot with a hull on the lane whom the campaign's own notice tells.
    let (told, _) = prospector(&mut rig, "Told", 93, "Grey Moth");
    *frame.told.lock().expect("lock") = vec![told];

    // Before: nothing declared, the Review short, the Freeport holding a duty-free Isidis. A
    // fresh farm stands at its threshold (doc 50 cut OP.8's retune), so Hold may be one up.
    let page = run(&mut rig.tw, newcomer, OPERATION_VERB);
    let hold = tacsim::concession::hold_clock(rig.tw.world()).expect("Hold the Concession");
    let hold_line = format!("Hold the Concession (the Freeport): {} of 6.", hold.filled);
    for want in [
        "No operation is declared at Isidis",
        "The Concession Review (the Republic): 2 of 6.",
        hold_line.as_str(),
        "Isidis now: the Freeport administers the Concession",
        "cargo sold at Isidis pays no import duty",
    ] {
        assert!(page.contains(want), "{want:?} in {page}");
    }
    assert!(!page.contains("Work for it"), "{page}");

    // The Review fills; on the Assembly's tick the Republic declares.
    let review = tacsim::concession::review_clock(rig.tw.world()).expect("the Review");
    let said = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@clock/advance {}={}",
            review.id.0,
            review.segments - review.filled
        ),
    );
    assert!(!said.contains("Usage"), "{said}");
    let declared = fires(&mut rig, 4);
    assert_eq!(
        frame.declared.lock().expect("lock").len(),
        1,
        "declared once"
    );

    // The notice: at once to a player with a hull on the lane; nothing to one without.
    let heard = notices(&rig, &declared, hauler);
    for want in [
        "Official notice: Republic Customs Office has declared an operation for the Republic to \
         assume the administration of the Isidis Concession, against Occator Water & Propellant \
         and Meridiani Security Group.",
        "and runs 28 IC days",
        "real hours",
        "+operation",
    ] {
        assert!(
            heard.contains(want),
            "{want:?} in {heard}; audience {:?}; hulls {:?}; on watch {}",
            tacsim::op_page::lane_audience(rig.tw.world()),
            tacsim::port::coop::trade_hulls(rig.tw.world(), hauler),
            tacsim::port_tick::on_watch(rig.tw.world(), hauler)
        );
    }
    assert_eq!(
        heard.matches("has declared an operation").count(),
        1,
        "told once: {heard}"
    );
    assert!(
        !notices(&rig, &declared, newcomer).contains("Official notice"),
        "a player with no hull on the lane is told nothing"
    );
    assert!(
        !notices(&rig, &declared, told).contains("Official notice"),
        "doc 50b item 9: one the campaign tells has no second notice from the lane"
    );

    // The page, in notice.
    let page = run(&mut rig.tw, hauler, OPERATION_VERB);
    for want in [
        "The Isidis operation (operation 11): Republic Customs Office, for the Republic, against \
         Occator Water & Propellant and Meridiani Security Group, for the Freeport, defending.",
        "If the Republic wins, it administers the Isidis Concession, and",
        "pay an import duty of 5% of their catalog value to Republic Customs Office",
        "If the Freeport holds, it keeps the Concession, and cargo sold at Isidis pays no import \
         duty.",
        "Notice runs: the window opens",
        "real hours",
        "the Freeport holds if Occator's tank farm at Isidis stands at 2960 t",
        "It holds ",
        "The Concession Review (the Republic): 6 of 6, filled.",
        "Work for it:",
    ] {
        assert!(page.contains(want), "{want:?} in {page}");
    }

    // The Freeport arm posts its haul: the page lists it, and the board tags it.
    for _ in 0..60 {
        if tacsim::stockpile::arm(rig.tw.world()).is_some_and(|a| !a.posts.is_empty()) {
            break;
        }
        let _ = fires(&mut rig, 1);
    }
    let post = tacsim::stockpile::arm(rig.tw.world())
        .and_then(|a| a.posts.first().cloned())
        .expect("the arm posted");
    let page = run(&mut rig.tw, hauler, OPERATION_VERB);
    assert!(
        page.contains(&format!("Haul {} for the Freeport", post.haul))
            && page.contains(&format!("+haul/accept {}", post.haul)),
        "{page}"
    );
    let board = run(&mut rig.tw, hauler, "+market/jobs haul market=ceres");
    assert!(board.contains(JOB_TAG.trim()), "{board}");

    // The window runs, then the Republic wins.
    *frame.phase.lock().expect("lock") = Some(OperationPhase::Running);
    let page = run(&mut rig.tw, hauler, OPERATION_VERB);
    assert!(page.contains("The window is open: it closes"), "{page}");
    *frame.phase.lock().expect("lock") = Some(OperationPhase::Settled);
    *frame.outcome.lock().expect("lock") = Some(OperationOutcome::DeclarerSucceeded);
    let decided = fires(&mut rig, 4);
    let heard = notices(&rig, &decided, hauler);
    assert!(
        heard.contains(
            "Official notice: the Isidis operation is decided: The Republic won, and assumed the \
             Concession's administration; "
        ) && heard.contains("5% of their catalog value to Republic Customs Office"),
        "{heard}"
    );
    assert_eq!(heard.matches("is decided").count(), 1, "told once: {heard}");

    // The newcomer, who never joined, reads who won and what it puts on a sale there.
    let page = run(&mut rig.tw, newcomer, OPERATION_VERB);
    for want in [
        "Decided: the Republic won, and assumed the Concession's administration.",
        "Isidis now: the Republic administers the Concession (since",
        "sold at Isidis pay an import duty of 5% of their catalog value to Republic Customs Office.",
    ] {
        assert!(page.contains(want), "{want:?} in {page}");
    }
    assert!(!page.contains("Work for it"), "{page}");
    let board = run(&mut rig.tw, hauler, "+market/jobs haul market=ceres");
    assert!(
        !board.contains(JOB_TAG.trim()),
        "the tag ends with it: {board}"
    );
}

/// Seat a fake frame in the slot the composition root fills.
fn seat(rig: &mut Rig) -> Arc<FakeFrame> {
    let frame = Arc::new(FakeFrame::default());
    let seated: Arc<dyn OperationFrame> = frame.clone();
    rig.tw
        .world_mut()
        .module_state_insert(OperationFrameSlot::new(seated));
    frame
}

/// **Doc 50 cut OP.7b: the Republic bids at Stickney while the operation runs** (Q-50-27):
/// Stickney's clerk keeps the customs office's store, a restart keeps it there, and the arm's bid
/// stands from the declaration to the resolution.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op7b_the_republic_bids_at_stickney_while_the_operation_runs() {
    let rig = stocked();
    // The store: Stickney's DARK stock clerk in the staff spine backs the office's vault.
    let map = port_map(&rig.tw, "stickney_port");
    let row = map.vendors[&cid("republic_customs")].clone();
    let clerk = map
        .clerk
        .as_ref()
        .map(|c| ObjId(c.thing))
        .expect("the clerk");
    assert_eq!(ObjId(row.storefront), clerk, "the clerk keeps the store");
    let spine = station_room(&rig.tw, "stickney_port", "room.p15_maintenance");
    let obj = rig.tw.world().get(clerk).expect("the clerk").clone();
    assert_eq!(obj.location, Some(spine), "in the staff spine");
    assert!(
        obj.flags.contains(mush_core::Flags::DARK),
        "dark, staff-only"
    );
    assert!(row.vendor.is_some(), "the market vendor is registered");

    // A restart keeps the store with the clerk, through the map row.
    let mut rig = reboot(rig, &[]);
    let again = port_map(&rig.tw, "stickney_port").vendors[&cid("republic_customs")].clone();
    assert_eq!(again.storefront, row.storefront, "the same store");
    assert_eq!(again.vault, row.vault, "the same vault");

    // The Review fills and the Republic declares: its bid stands at Stickney.
    let frame = seat(&mut rig);
    let review = tacsim::concession::review_clock(rig.tw.world()).expect("the Review");
    let said = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@clock/advance {}={}",
            review.id.0,
            review.segments - review.filled
        ),
    );
    assert!(!said.contains("Usage"), "{said}");
    let _ = fires(&mut rig, 4);
    assert_eq!(frame.declared.lock().expect("lock").len(), 1);
    let listing = tacsim::isidis_op::record(rig.tw.world())
        .and_then(|r| r.bid)
        .expect("the Republic's bid is posted");
    let bid = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|s| s.listing(mush_market::ListingId(listing)).cloned())
        .expect("the bid's listing");
    assert_eq!(bid.price.minor(), tacsim::op_page::BID_CR * 100);
    assert_eq!(bid.quantity, tacsim::op_page::BID_T);
    assert_eq!(bid.market.as_str(), "stickney");
    // The office holds the bond and still funds the whole bid.
    let (_, account) = tacsim::polity::lane_org_account(rig.tw.world(), &cid("republic_customs"))
        .expect("the office's account");
    let available = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(account).and_then(|a| a.available()))
        .map_or(0, |a| a.minor());
    let whole =
        tacsim::op_page::BID_CR * i64::try_from(tacsim::op_page::BID_T).expect("fits") * 100;
    assert!(available >= whole, "{available} funds {whole}");
    let hauler = pilot(&mut rig, "Seller", 93);
    let page = run(&mut rig.tw, hauler, OPERATION_VERB);
    assert!(
        page.contains(&format!(
            "The Republic's bid: Republic Customs Office buys up to {} t of",
            tacsim::op_page::BID_T
        )) && page.contains(&format!("+market/sell {listing}=<qty>")),
        "{page}"
    );

    // The defenders hold: the bid is withdrawn with the operation.
    *frame.phase.lock().expect("lock") = Some(OperationPhase::Settled);
    *frame.outcome.lock().expect("lock") = Some(OperationOutcome::DeclarerFailed);
    let _ = fires(&mut rig, 4);
    assert!(
        rig.tw
            .world()
            .module_state::<mush_market::MarketStore>()
            .is_some_and(|s| s.listing(mush_market::ListingId(listing)).is_none()),
        "the bid is withdrawn"
    );
    assert_eq!(
        tacsim::isidis_op::record(rig.tw.world()).and_then(|r| r.bid),
        None
    );
}
