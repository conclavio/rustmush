//! **Doc 50 cut OP.6 — whose side a hull flies for** (Q-50-26, Q-50-61), over the production
//! roster, the operation read through a fake frame as OP.5's story reads it.
//!
//! Inside the operation's window, a Freeport-flagged Co-op pool hull and a hull titled to a
//! Republic organization are operation foes; two Freeport hulls are not, nor is a hull with no
//! flag a foe of anyone. Before the window opens, nobody is.
//!
//! **What is a fixture.** The fake frame (TacSim has no campaign edge; the real one is OP.0's),
//! staff's `@clock/advance` filling the Review, and a Republic organization's title issued on an
//! object, as OP.2's story issues it.

use std::sync::{Arc, Mutex};

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_testkit::TestWorld;
use tacsim::TacsimStore;
use tacsim::operations::{
    OperationFrame, OperationFrameSlot, OperationOutcome, OperationPhase, OperationRequest,
    OperationStatus,
};
use tacsim::space::op_sides::foes;

use crate::support;

const OP: u64 = 9;

/// **A stand-in for the composition root's frame**, answering the phase the story sets.
struct FakeFrame {
    declared: Mutex<Vec<OperationRequest>>,
    phase: Mutex<OperationPhase>,
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
        Some(OperationStatus {
            phase: *self.phase.lock().ok()?,
            declarer: request.declarer,
            sides: request.sides,
            defenders: request.defenders,
            legal_targets: request.legal_targets,
            window: (0, request.window_secs),
            outcome: None::<OperationOutcome>,
            told: Vec::new(),
        })
    }
}

/// The production roster booted, the fake frame seated, the Review filled and the operation
/// declared.
fn declared(frame: &Arc<FakeFrame>) -> TestWorld {
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, support::production_roster())
        .expect("the roster registers");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("the world boots");
    let mut tw = TestWorld::from_engine(engine);
    let seated: Arc<dyn OperationFrame> = frame.clone();
    tw.world_mut()
        .module_state_insert(OperationFrameSlot::new(seated));
    let review = tacsim::concession::review_clock(tw.world()).expect("the Review");
    let left = review.segments - review.filled;
    let _ = tw.run(
        mush_core::GOD,
        &format!("@clock/advance {}={left}", review.id.0),
    );
    let t = tw.world().now() + 61;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
    assert_eq!(
        tacsim::isidis_op::record(tw.world()).and_then(|r| r.operation),
        Some(OP),
        "the operation is declared"
    );
    tw
}

/// Ceres' Co-op pool hulls (titled to a Freeport organization).
fn pool_hulls(tw: &TestWorld) -> Vec<ObjId> {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the store")
        .port_maps()
        .find(|map| map.entry.as_str() == "ceres_port")
        .map(|map| map.hulls.iter().map(|row| ObjId(row.hull)).collect())
        .unwrap_or_default()
}

/// An object titled to Stickney Yards, a Republic organization: a Republic-flagged hull.
fn republic_hull(tw: &mut TestWorld) -> ObjId {
    let cutter = tw.spawn_player("Cutter");
    let yards = tacsim::polity::lane_org_account(
        tw.world(),
        &CanonicalId::new("stickney_yards").expect("an id"),
    )
    .expect("Stickney Yards")
    .0;
    support::with_ctx(tw, |ctx| {
        mush_economy::issue_title(
            ctx,
            cutter,
            mush_org::PrincipalId::Organization(yards),
            None,
            mush_org::SystemPrincipalId::new("tacsim.registry").expect("the issuer"),
            None,
        )
    })
    .expect("the title issues");
    cutter
}

/// **Doc 50 cut OP.6: in the operation's window a Freeport hull and a Republic hull are foes**
/// (Q-50-26, Q-50-61); before it, and between two hulls of one side or none, nobody is.
#[test]
fn op6_in_the_window_a_freeport_hull_and_a_republic_hull_are_operation_foes() {
    let frame = Arc::new(FakeFrame {
        declared: Mutex::new(Vec::new()),
        phase: Mutex::new(OperationPhase::Notice),
    });
    let mut tw = declared(&frame);
    let pools = pool_hulls(&tw);
    let (freeport_a, freeport_b) = (pools[0], pools[1]);
    let cutter = republic_hull(&mut tw);
    let stray = tw.spawn_player("Stray");

    assert!(
        !foes(tw.world(), freeport_a, cutter),
        "before the window opens nobody is a foe"
    );
    *frame.phase.lock().expect("lock") = OperationPhase::Running;
    assert!(
        foes(tw.world(), freeport_a, cutter),
        "inside the window they are"
    );
    assert!(foes(tw.world(), cutter, freeport_a), "both ways");
    assert!(
        !foes(tw.world(), freeport_a, freeport_b),
        "two hulls of one side are not"
    );
    assert!(
        !foes(tw.world(), freeport_a, stray),
        "a hull flying for no side is no foe"
    );
    *frame.phase.lock().expect("lock") = OperationPhase::Settling;
    assert!(
        !foes(tw.world(), freeport_a, cutter),
        "after the window closes nobody is"
    );
}
