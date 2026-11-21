//! **Doc 50 cut OP.5 — the Republic's declaring arm and the resolution** (Q-50-18, Q-50-19,
//! Q-50-28, Q-50-29, Q-50-57), over the production roster.
//!
//! TacSim has no campaign edge, so a **fake frame** stands where the composition root seats the
//! campaign's ([`FakeFrame`], TacSim's own `OperationFrame` trait): it records what the arm
//! declares and answers the outcome the story sets. The real frame, over the campaign, is OP.0's
//! (`mush-composition`'s `op0_frame`); the probe it reads is OP.4's.
//!
//! - **The Review fills**, and on the Assembly's next tick the arm declares once: the Republic's
//!   customs office, the tank farm's close probe with the defenders' polarity, Occator defending,
//!   28 IC days of notice and of window, and a bond forfeit to Occator.
//! - **The Republic wins**: the arm writes the Concession's de facto holder once.
//! - **The defenders hold**: the Freeport stays.
//! - **Unwired**, nothing is declared.

use std::sync::{Arc, Mutex};

use mush_core::{CanonicalId, Ctx, World};
use mush_faction::FactionId;
use mush_org::PrincipalId;
use mush_testkit::TestWorld;
use tacsim::operations::{
    MetFavors, OperationFrame, OperationFrameSlot, OperationOutcome, OperationPhase,
    OperationRequest, OperationStatus,
};

use crate::support;

/// The operation number the fake frame hands out.
const OP: u64 = 7;

/// **A stand-in for the composition root's frame**: it records each declaration and answers the
/// outcome the story sets.
#[derive(Default)]
struct FakeFrame {
    declared: Mutex<Vec<OperationRequest>>,
    outcome: Mutex<Option<OperationOutcome>>,
}

impl OperationFrame for FakeFrame {
    fn declare(&self, _ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String> {
        self.declared
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .push(request.clone());
        Ok(OP)
    }

    fn status(&self, _w: &World, operation: u64) -> Option<OperationStatus> {
        if operation != OP {
            return None;
        }
        let request = self.declared.lock().ok()?.first()?.clone();
        let outcome = *self.outcome.lock().ok()?;
        Some(OperationStatus {
            phase: if outcome.is_some() {
                OperationPhase::Settled
            } else {
                OperationPhase::Notice
            },
            declarer: request.declarer,
            sides: request.sides,
            defenders: request.defenders,
            legal_targets: request.legal_targets,
            window: (0, request.window_secs),
            outcome,
            told: Vec::new(),
        })
    }
}

/// The production roster booted, the fake frame seated in the slot the composition root fills.
fn booted(frame: Option<&Arc<FakeFrame>>) -> TestWorld {
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, support::production_roster())
        .expect("the roster registers");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("the world boots");
    let mut tw = TestWorld::from_engine(engine);
    if let Some(frame) = frame {
        let seated: Arc<dyn OperationFrame> = frame.clone();
        tw.world_mut()
            .module_state_insert(OperationFrameSlot::new(seated));
    }
    tw
}

/// The Assembly's tick fires (a minute and a second of wall time).
fn tick(tw: &mut TestWorld) {
    let t = tw.world().now() + 61;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

/// Staff fill the Concession Review (`@clock/advance`).
fn fill_the_review(tw: &mut TestWorld) {
    let review = tacsim::concession::review_clock(tw.world()).expect("the Review");
    let left = review.segments - review.filled;
    let _ = tw.run(
        mush_core::GOD,
        &format!("@clock/advance {}={left}", review.id.0),
    );
    assert!(
        tacsim::concession::review_clock(tw.world()).is_some_and(|c| c.completed),
        "the Review is full"
    );
}

fn isidis_holder(tw: &TestWorld) -> Option<FactionId> {
    tacsim::polity::control_at(tw.world(), &CanonicalId::new("isidis_port").expect("an id"))
        .and_then(|c| c.de_facto)
}

fn polity(tw: &TestWorld, tag: &str) -> FactionId {
    tacsim::polity::polity(tw.world(), tag).expect("a polity")
}

fn lane_org(tw: &TestWorld, id: &str) -> PrincipalId {
    PrincipalId::Organization(
        tacsim::polity::lane_org_account(tw.world(), &CanonicalId::new(id).expect("an id"))
            .expect("a lane organization")
            .0,
    )
}

/// **Doc 50 cut OP.5: the Review fills, the Republic declares, and its success takes Isidis**
/// (Q-50-19, Q-50-28, Q-50-29, Q-50-57).
#[test]
fn op5_the_republic_declares_on_the_reviews_fill_and_its_success_takes_isidis() {
    let frame = Arc::new(FakeFrame::default());
    let mut tw = booted(Some(&frame));
    let (republic, freeport) = (polity(&tw, "republic"), polity(&tw, "freeport"));
    assert_eq!(
        isidis_holder(&tw),
        Some(freeport),
        "the Freeport holds Isidis"
    );

    // Nothing is declared while the Review is short.
    tick(&mut tw);
    assert!(frame.declared.lock().expect("lock").is_empty());

    // It fills: the next tick declares once.
    fill_the_review(&mut tw);
    tick(&mut tw);
    tick(&mut tw);
    let declared = frame.declared.lock().expect("lock").clone();
    assert_eq!(declared.len(), 1, "declared once: {declared:?}");
    let request = &declared[0];
    let customs = lane_org(&tw, "republic_customs");
    let occator = lane_org(&tw, "occator");
    assert_eq!(
        request.declarer, customs,
        "the Republic's customs office declares"
    );
    assert_eq!(request.probe, tacsim::stockpile::TANK_FARM_PROBE);
    assert_eq!(
        request.met_favors,
        MetFavors::Defenders,
        "at its threshold the defenders hold"
    );
    // Doc 50b item 9 (Q-50-19 as amended): Meridiani defends beside Occator; the bond and the
    // Freeport's side stay Occator's.
    let meridiani = lane_org(&tw, "meridiani");
    assert_eq!(request.defenders, vec![occator.clone(), meridiani]);
    assert_eq!(
        request.sides,
        vec![
            ("republic".to_owned(), customs),
            ("freeport".to_owned(), occator.clone())
        ]
    );
    assert_eq!(request.notice_secs, 28 * 86_400);
    assert_eq!(request.window_secs, 28 * 86_400);
    let bond = request.bond.as_ref().expect("a bond");
    assert_eq!(bond.amount.minor(), tacsim::isidis_op::BOND_CR * 100);
    assert_eq!(bond.forfeit_to.as_ref().map(|(who, _)| who), Some(&occator));
    assert_eq!(
        tacsim::isidis_op::record(tw.world()).and_then(|r| r.operation),
        Some(OP)
    );

    // Running: nothing changes hands.
    tick(&mut tw);
    assert_eq!(isidis_holder(&tw), Some(freeport));

    // The Republic's objective is met: it assumes the Concession, once.
    *frame.outcome.lock().expect("lock") = Some(OperationOutcome::DeclarerSucceeded);
    tick(&mut tw);
    assert_eq!(
        isidis_holder(&tw),
        Some(republic),
        "the Republic holds Isidis"
    );
    assert_eq!(
        tacsim::isidis_op::record(tw.world())
            .and_then(|r| r.outcome.clone())
            .as_deref(),
        Some("republic")
    );
    tick(&mut tw);
    assert_eq!(
        frame.declared.lock().expect("lock").len(),
        1,
        "never declared again"
    );
}

/// **Doc 50 cut OP.5: the defenders hold** (Q-50-28): the Freeport keeps Isidis.
#[test]
fn op5_when_the_defenders_hold_the_freeport_keeps_isidis() {
    let frame = Arc::new(FakeFrame::default());
    let mut tw = booted(Some(&frame));
    let freeport = polity(&tw, "freeport");
    fill_the_review(&mut tw);
    tick(&mut tw);
    *frame.outcome.lock().expect("lock") = Some(OperationOutcome::DeclarerFailed);
    tick(&mut tw);
    assert_eq!(
        isidis_holder(&tw),
        Some(freeport),
        "the Freeport keeps Isidis"
    );
    assert_eq!(
        tacsim::isidis_op::record(tw.world())
            .and_then(|r| r.outcome.clone())
            .as_deref(),
        Some("freeport")
    );
}

/// **Doc 50 cut OP.5: unwired, nothing is declared** (Q-50-57 item 4): with no frame composed, a
/// full Review waits.
#[test]
fn op5_with_no_frame_composed_nothing_is_declared() {
    let mut tw = booted(None);
    fill_the_review(&mut tw);
    tick(&mut tw);
    assert_eq!(
        tacsim::isidis_op::record(tw.world()).and_then(|r| r.operation),
        None
    );
}
