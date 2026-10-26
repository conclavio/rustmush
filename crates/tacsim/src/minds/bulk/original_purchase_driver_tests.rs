//! Consumer tests reuse the parent's one genuine full Registry certificate.
//! The observation latches test consumer behavior only; server disposition
//! tests prove when the real runtime is permitted to emit NoIntent.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, Inbound, OfferExecutionContext, OfferSubmission};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Proof(Arc<AtomicBool>);
impl SubmissionDispositionReader for Proof {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Runtime;
impl SubmissionRuntime for Runtime {
    fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        let proof = Arc::new(AtomicBool::new(false));
        SubmissionRuntimeState::new(Arc::new(Proof(proof.clone())), Box::new(proof))
    }
}
struct Editor(Arc<AtomicUsize>);
impl mush_core::module::InputContext for Editor {
    fn prompt(&self, _: &World) -> mush_proto::PromptSpec {
        mush_proto::PromptSpec {
            mode: mush_proto::InputMode::Normal,
            text: None,
        }
    }
    fn feed(&mut self, _: &mut mush_core::Ctx<'_>, _: &str) -> mush_core::module::ContextOutcome {
        self.0.fetch_add(1, Ordering::Relaxed);
        mush_core::module::ContextOutcome::Consumed
    }
}
fn branch(f: &PurchaseFixture, saved: &Durable) -> TestResult<PurchaseFixture> {
    let mut result = f.reboot(saved)?;
    result.engine.set_submission_runtime(Arc::new(Runtime))?;
    result.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x63; 32])?,
        [],
    )?);
    Ok(result)
}
fn fire(f: &mut PurchaseFixture, now: i64) -> TestResult {
    f.engine.world.set_now(now);
    // The first drain queues the opaque continuation; the second fires it.
    for _ in 0..2 {
        f.engine.fire_due(
            now.checked_mul(1000).ok_or("time overflow")?,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        );
    }
    module_ops(&mut f.disk, &f.engine.take_module_ops())?;
    Ok(())
}
fn command(f: &mut PurchaseFixture, actor: ObjId, operation: CommitToken) {
    f.engine.step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: format!("@mindpurchase/continue {operation}"),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
}
fn take(
    f: &mut PurchaseFixture,
    operation: CommitToken,
    id: &str,
) -> TestResult<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)> {
    let driven = f
        .engine
        .take_observed_authoritative_submit()
        .ok_or("real service selector")?;
    check!(driven.is_observed());
    let (request, driver) = driven.into_parts();
    check_eq!(request.actor, BUYER);
    check_eq!(request.action.key().owner.as_str(), "market");
    check_eq!(request.action.key().id.as_str(), id);
    check_eq!(request.subject, PerceivedRef::SelfActor);
    check!(request.targets.is_empty());
    check_eq!(request.parameters, json!({"operation":operation}));
    let proof = *driver
        .ok_or("actual runtime observer")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "runtime proof type")?;
    check!(!proof.load(Ordering::Acquire));
    Ok((request, proof))
}
fn no_request(f: &mut PurchaseFixture) {
    assert!(f.engine.take_observed_authoritative_submit().is_none());
}
fn land(f: &mut PurchaseFixture, submit: &AuthoritativeSubmit, serial: u128) -> TestResult {
    let principal = PrincipalId::Object { id: submit.actor };
    let query =
        f.engine
            .legal_action_query(principal.clone(), submit.actor, submit.subject.clone())?;
    let offers = f.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&submit.action)
        .ok_or("actual terminal offer")?;
    let resolved = f.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(serial.to_be_bytes()),
            principal: principal.clone(),
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: submit.action.key().owner.clone(),
                    namespace: CanonicalId::new(submit.action.key().id.as_str())?,
                },
                1,
                IdempotencyKey::Caller(submit.key.clone()),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: projected.offer(),
            subject: submit.subject.clone(),
            targets: submit.targets.clone(),
            parameters: submit.parameters.clone(),
        },
    )?;
    let (action, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(action, submit.action);
    let before = f.balances()?;
    let roots = f.roots()?;
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(serial),
        )?;
    check_eq!(f.balances()?, before);
    check_eq!(f.roots()?, roots);
    f.install(prepared)?;
    Ok(())
}
fn assert_funded(
    f: &PurchaseFixture,
    operation: CommitToken,
    money: &[(Amount, Amount)],
) -> TestResult {
    check_eq!(f.balances()?, money);
    check!(
        f.economy()?
            .purchase_receipt_audit()
            .get(operation)?
            .terminal()
            .is_none()
    );
    check!(f.rows("roll", "roll_mind_personal_paid_licence").is_empty());
    check!(f.rows("market", "market_mind_purchase_receipt").is_empty());
    Ok(())
}

pub(super) fn exercise(f: &PurchaseFixture, saved: &Durable, operation: CommitToken) -> TestResult {
    let money = f.balances()?;
    let roots = f.roots()?;
    // Healthy funded boot resumes Grant using the actual buyer, including when
    // that buyer is actively editing. No ticket text enters their editor.
    let mut healthy = branch(f, saved)?;
    command(&mut healthy, GOD, operation);
    no_request(&mut healthy);
    let input = Arc::new(AtomicUsize::new(0));
    healthy.engine.world.push_input_context(
        BUYER,
        Some("market".into()),
        Box::new(Editor(input.clone())),
    );
    fire(&mut healthy, saved.now + 1)?;
    let (grant, unknown) = take(&mut healthy, operation, "settle_original_mind")?;
    check_eq!(input.load(Ordering::Relaxed), 0);
    check_eq!(healthy.engine.world.input_depth(BUYER), 1);
    assert_funded(&healthy, operation, &money)?;
    land(&mut healthy, &grant, 4_000_001)?;
    check!(
        !unknown.load(Ordering::Acquire),
        "actual readback, not a made-up ACK, completes the entry"
    );
    fire(&mut healthy, saved.now + 2)?;
    no_request(&mut healthy);
    check_eq!(healthy.roots()?, roots);
    check_eq!(
        healthy
            .rows("roll", "roll_mind_personal_paid_licence")
            .len(),
        1
    );
    healthy.reboot(&healthy.snapshot())?;

    // Missing/GOING actual buyer has no substitute GOD identity. The funded
    // obligation remains until that buyer is again an admissible actor.
    let mut absent = branch(f, saved)?;
    absent
        .engine
        .world
        .get_mut(BUYER)
        .ok_or("buyer")?
        .flags
        .insert(mush_core::Flags::GOING);
    fire(&mut absent, saved.now + 1)?;
    no_request(&mut absent);
    assert_funded(&absent, operation, &money)?;
    absent
        .engine
        .world
        .get_mut(BUYER)
        .ok_or("buyer")?
        .flags
        .remove(mush_core::Flags::GOING);
    fire(&mut absent, saved.now + 2)?;
    take(&mut absent, operation, "settle_original_mind")?;

    // A generic source/owner read refusal does not mean refund. A successful
    // later read can resume the same real grant without changing its consent.
    let mut owner = branch(f, saved)?;
    owner
        .engine
        .world
        .get_mut(GOD)
        .ok_or("seller")?
        .flags
        .insert(mush_core::Flags::GOING);
    fire(&mut owner, saved.now + 1)?;
    no_request(&mut owner);
    assert_funded(&owner, operation, &money)?;
    owner
        .engine
        .world
        .get_mut(GOD)
        .ok_or("seller")?
        .flags
        .remove(mush_core::Flags::GOING);
    fire(&mut owner, saved.now + 2)?;
    take(&mut owner, operation, "settle_original_mind")?;

    let mut uncertain = branch(f, saved)?;
    fire(&mut uncertain, saved.now + 1)?;
    let (_, first_proof) = take(&mut uncertain, operation, "settle_original_mind")?;
    let expired = saved.now + QUOTE_SECONDS + 5;
    fire(&mut uncertain, expired)?;
    no_request(&mut uncertain);
    assert_funded(&uncertain, operation, &money)?;
    first_proof.store(true, Ordering::Release); // consumer fixture: exact NoIntent
    fire(&mut uncertain, expired + 1)?;
    let (refund, second_proof) = take(&mut uncertain, operation, "refund_original_mind")?;
    fire(&mut uncertain, expired + 2)?;
    no_request(&mut uncertain);
    check!(
        !second_proof.load(Ordering::Acquire),
        "old consumed absence cannot finish the replacement"
    );
    land(&mut uncertain, &refund, 4_000_002)?;
    fire(&mut uncertain, expired + 3)?;
    no_request(&mut uncertain);
    check!(
        uncertain
            .rows("roll", "roll_mind_personal_paid_licence")
            .is_empty()
    );
    check!(uncertain.rows("roll", "roll_mind_paid_term").is_empty());
    check_eq!(uncertain.roots()?, roots);
    check!(
        uncertain
            .economy()?
            .purchase_receipt_audit()
            .get(operation)?
            .terminal()
            .is_some_and(|t| !t.granted())
    );
    uncertain.reboot(&uncertain.snapshot())?;

    // NoIntent while still eligible retries Grant, rather than treating every
    // definitive refusal as a policy instruction to refund.
    let mut retry = branch(f, saved)?;
    fire(&mut retry, saved.now + 1)?;
    let (first, proof) = take(&mut retry, operation, "settle_original_mind")?;
    proof.store(true, Ordering::Release);
    fire(&mut retry, saved.now + 2)?;
    let (second, _) = take(&mut retry, operation, "settle_original_mind")?;
    check_eq!(first, second);

    // The earlier unresolved purchase must not starve another funded purchase
    // belonging to this same buyer. Both observations remain independently held.
    let mut fair = branch(f, saved)?;
    let quote = fair.config.quotes.quote_action()?;
    fair.perform(
        &quote,
        BUYER,
        4_000_010,
        json!({"listing":token(1_000_002)}),
    )?;
    let [fund, _, _] = fair.config.purchases.actions()?;
    fair.perform(&fund, BUYER, 4_000_011, json!({"quote":token(4_000_010)}))?;
    fire(&mut fair, saved.now + 1)?;
    let (_, earlier) = take(&mut fair, operation, "settle_original_mind")?;
    fire(&mut fair, saved.now + 2)?;
    let (_, later) = take(&mut fair, token(4_000_011), "settle_original_mind")?;
    for now in saved.now + 3..=saved.now + 5 {
        fire(&mut fair, now)?;
        no_request(&mut fair);
    }
    check!(!earlier.load(Ordering::Acquire));
    check!(!later.load(Ordering::Acquire));
    check!(
        fair.economy()?
            .purchase_receipt_audit()
            .get(operation)?
            .terminal()
            .is_none()
    );
    check!(
        fair.economy()?
            .purchase_receipt_audit()
            .get(token(4_000_011))?
            .terminal()
            .is_none()
    );

    // A known venue change may select Refund while the actual buyer remains
    // available. The closed refund itself still proves all frozen custody.
    let mut away = branch(f, saved)?;
    let outside = away
        .engine
        .world
        .create(ObjKind::Room, "Outside purchase", GOD, None)?;
    away.engine.world.move_to(BUYER, outside)?;
    fire(&mut away, saved.now + 1)?;
    let (refund, _) = take(&mut away, operation, "refund_original_mind")?;
    land(&mut away, &refund, 4_000_012)?;
    check!(
        away.rows("roll", "roll_mind_personal_paid_licence")
            .is_empty()
    );
    check_eq!(away.roots()?, roots);

    // Fill the actual Engine's unrelated pre-ingress queue. The paid selector
    // is prepared by its real registered continuation but is never handed to
    // a runtime/coordinator: dropping that exact core request produces the
    // genuine NotDispatched observation, without a test completion latch.
    let mut blocked = branch(f, saved)?;
    let [_, grant_action, _] = blocked.config.purchases.actions()?;
    let mut queued_actors = BTreeSet::new();
    let mut out = Outbox::new();
    for n in 0..mush_core::AUTHORITATIVE_SUBMIT_QUEUE_MAX {
        let actor = blocked.engine.world.create(
            ObjKind::Player,
            &format!("Queue actor {n}"),
            GOD,
            Some(ROOM),
        )?;
        queued_actors.insert(actor);
        // These ordinary raw selectors exercise queue admission only. They are
        // discarded below and never purport to own or execute this purchase.
        out.defer(mush_core::Deferred::AuthoritativeSubmit(Box::new(
            AuthoritativeSubmit {
                actor,
                action: grant_action.clone(),
                subject: PerceivedRef::SelfActor,
                targets: vec![],
                parameters: json!({"operation":operation}),
                key: mush_core::IdempotencyKeyText::parse_canonical(format!("capacity.{n}"))?,
            },
        )));
    }
    blocked.engine.step(
        Inbound::Cmd {
            conn: 0,
            actor: GOD,
            line: String::new(),
        },
        &mut out,
        &mut mush_core::engine::Effects::new(),
    );
    check_eq!(queued_actors.len(), 64);
    for actor in &queued_actors {
        check!(blocked.engine.has_authoritative_submit_for(*actor));
    }
    check!(!blocked.engine.has_authoritative_submit_for(BUYER));
    fire(&mut blocked, saved.now + 1)?;
    check!(!blocked.engine.has_authoritative_submit_for(BUYER));
    assert_funded(&blocked, operation, &money)?;
    for _ in 0..64 {
        let queued = blocked
            .engine
            .take_observed_authoritative_submit()
            .ok_or("incumbent queued actor")?;
        check!(
            !queued.is_observed(),
            "paid attempt never entered the driver"
        );
        check!(queued_actors.remove(&queued.request().actor));
    }
    check!(queued_actors.is_empty());
    no_request(&mut blocked);
    fire(&mut blocked, saved.now + 2)?;
    let (retry, proof) = take(&mut blocked, operation, "settle_original_mind")?;
    check!(!proof.load(Ordering::Acquire));
    land(&mut blocked, &retry, 4_000_013)?;
    fire(&mut blocked, saved.now + 3)?;
    no_request(&mut blocked);
    check_eq!(blocked.roots()?, roots);
    check_eq!(
        blocked
            .rows("roll", "roll_mind_personal_paid_licence")
            .len(),
        1
    );
    blocked.reboot(&blocked.snapshot())?;

    // A cold already-expired funded operation chooses a real buyer refund.
    let mut cold = saved.clone();
    cold.now = expired;
    let mut cold = branch(f, &cold)?;
    fire(&mut cold, expired + 1)?;
    let (refund, _) = take(&mut cold, operation, "refund_original_mind")?;
    land(&mut cold, &refund, 4_000_003)?;
    check!(
        cold.economy()?
            .purchase_receipt_audit()
            .get(operation)?
            .terminal()
            .is_some_and(|t| !t.granted())
    );
    cold.reboot(&cold.snapshot())?;
    Ok(())
}
