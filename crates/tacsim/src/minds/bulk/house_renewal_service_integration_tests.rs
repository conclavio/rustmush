//! Real House service continuations over the parent's genuine paid birth.
//! Submission disposition is a consumer fixture; it is not a server NoIntent claim.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, IdempotencyKeyText};
use mush_economy::managed_house_renewal::RenewalServiceCloseReason;
use std::sync::atomic::{AtomicBool, Ordering};
const SERVICE_CLOSE: &str = "close_house_mind_renewal_service";
const SERIAL: u128 = START + 1000;
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
fn branch(f: &Fixture, snapshot: &Durable) -> TestResult<Box<PlayerWorld>> {
    let mut w = boot(f, snapshot)?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x79; 32])?,
        [],
    )?);
    Ok(w)
}
fn fire(w: &mut PlayerWorld) -> TestResult {
    let now = w.engine.world.now();
    for _ in 0..4 {
        w.engine.fire_due(
            now.checked_mul(1000).ok_or("clock")?,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        );
    }
    w.commit()?;
    Ok(())
}
fn tick(w: &mut PlayerWorld) -> TestResult {
    w.engine
        .world
        .set_now(w.engine.world.now().checked_add(1).ok_or("clock")?);
    fire(w)
}
fn next(
    w: &mut PlayerWorld,
    actor: ObjId,
    expected: &str,
) -> TestResult<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)> {
    let mut selected = w.engine.take_observed_authoritative_submit();
    for _ in 0..12 {
        if selected.is_some() {
            break;
        }
        tick(w)?;
        selected = w.engine.take_observed_authoritative_submit();
    }
    let driven = selected.ok_or("actual House renewal selector")?;
    assert!(driven.is_observed());
    let (request, proof) = driven.into_parts();
    assert_eq!(request.actor, actor);
    assert_eq!(request.action.key().owner.as_str(), "market");
    assert_eq!(request.action.key().id.as_str(), expected);
    assert_eq!(request.subject, PerceivedRef::SelfActor);
    assert!(request.targets.is_empty());
    let proof = *proof
        .ok_or("proof")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "proof type")?;
    assert!(!proof.load(Ordering::Acquire));
    Ok((request, proof))
}
fn none(w: &mut PlayerWorld) {
    assert!(
        w.engine.take_observed_authoritative_submit().is_none(),
        "Unknown or lack of acceptance never permits another selector"
    );
}
fn land(w: &mut PlayerWorld, request: &AuthoritativeSubmit, n: u128) -> TestResult {
    let principal = PrincipalId::Object { id: request.actor };
    let query =
        w.engine
            .legal_action_query(principal.clone(), request.actor, request.subject.clone())?;
    let offers = w.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&request.action)
        .ok_or("actual signed offer")?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(n.to_be_bytes()),
            principal: principal.clone(),
            actor: request.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: request.action.key().owner.clone(),
                    namespace: CanonicalId::new(request.action.key().id.as_str())?,
                },
                1,
                IdempotencyKey::Caller(request.key.clone()),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: projected.offer(),
            subject: request.subject.clone(),
            targets: request.targets.clone(),
            parameters: request.parameters.clone(),
        },
    )?;
    let (action, invocation, _) = resolved.into_authoritative_parts();
    assert_eq!(action, request.action);
    let before = disk(w);
    let (prepared, _) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &invocation,
            token(n),
        )?;
    assert!(disk(w) == before, "prepare publishes no private rows");
    apply(w, prepared)?;
    Ok(())
}
fn status(w: &mut PlayerWorld, actor: ObjId, id: LicenceId) -> TestResult<String> {
    let events = w.say(actor, &format!("@housemindrenewal/status {}", id.get()))?;
    let text = PlayerWorld::private_text(&events, actor);
    assert!(
        !text.contains(SOURCE) && !text.contains("this.landed"),
        "source-free status"
    );
    Ok(text)
}
fn closed(w: &PlayerWorld, n: u128, reason: RenewalServiceCloseReason) -> TestResult {
    let row = operation(w, n)?;
    let RenewalStateV1::UnfundedServiceClosed { closure } = row.state() else {
        return Err("actual House service close".into());
    };
    assert_eq!(closure.actor(), GOD);
    assert_eq!(closure.reason(), reason);
    assert_eq!(closure.action().key().id.as_str(), SERVICE_CLOSE);
    Ok(())
}
pub(super) fn exercise(f: &Fixture, snapshot: &Durable, id: LicenceId) -> TestResult {
    let mut w = branch(f, snapshot)?;
    let old = licence(&w, id)?;
    let predecessor = paid(&w, id, 1)?;
    let payer = wallet(&w, EconomicOwner::Object(BUYER), None)?;
    let money = balance(&w, payer)?;
    assert!(status(&mut w, BUYER, id)?.contains("No renewal offer"));
    assert!(!status(&mut w, GOD, id)?.contains("Price:"));
    assert!(
        direct(
            &mut w,
            SERVICE_CLOSE,
            GOD,
            SERIAL,
            json!({"operation":token(SERIAL),"reason":"deadline_reached"})
        )
        .is_err()
    );
    fire(&mut w)?;
    none(&mut w);
    set_ic(&mut w, predecessor.window().term_end().get())?;
    fire(&mut w)?;
    let (draft, unknown) = next(&mut w, BUYER, DRAFT)?;
    assert_eq!(draft.parameters, json!({"licence":id.get()}));
    tick(&mut w)?;
    none(&mut w);
    land(&mut w, &draft, SERIAL)?;
    assert!(!unknown.load(Ordering::Acquire));
    tick(&mut w)?;
    none(&mut w);
    assert!(status(&mut w, BUYER, id)?.contains("explicit acceptance"));
    assert_eq!(balance(&w, payer)?, money, "automatic offer is unfunded");
    let drafted = saved(&w);
    w.say(BUYER, &format!("@housemindrenewal/accept {}", id.get()))?;
    let (fund, _) = next(&mut w, BUYER, FUND)?;
    land(&mut w, &fund, SERIAL + 1)?;
    tick(&mut w)?;
    let (grant, grant_unknown) = next(&mut w, BUYER, GRANT)?;
    let held = balance(&w, payer)?;
    let expires = operation(&w, SERIAL)?.offer().terms.expires_at.at;
    set_ic(&mut w, expires)?;
    fire(&mut w)?;
    none(&mut w);
    assert_eq!(balance(&w, payer)?, held, "Unknown Grant retains escrow");
    grant_unknown.store(true, Ordering::Release);
    tick(&mut w)?;
    let (refund, _) = next(&mut w, BUYER, REFUND)?;
    assert_ne!(grant.action, refund.action);
    land(&mut w, &refund, SERIAL + 2)?;
    tick(&mut w)?;
    none(&mut w);
    assert_eq!(balance(&w, payer)?, money);
    current_same(&w, &old, &predecessor)?;
    let _cold = boot(f, &saved(&w))?;

    // Successful accepted funding is advanced by exact installed readback.
    let mut healthy = branch(f, &drafted)?;
    healthy.say(BUYER, &format!("@housemindrenewal/accept {}", id.get()))?;
    let (fund, _) = next(&mut healthy, BUYER, FUND)?;
    land(&mut healthy, &fund, SERIAL + 10)?;
    tick(&mut healthy)?;
    let (grant, _) = next(&mut healthy, BUYER, GRANT)?;
    land(&mut healthy, &grant, SERIAL + 11)?;
    tick(&mut healthy)?;
    none(&mut healthy);
    assert_eq!(licence(&healthy, id)?.term().get(), 2);
    let _cold = boot(f, &saved(&healthy))?;

    for invalidated in [false, true] {
        let mut close = branch(f, &drafted)?;
        if invalidated {
            let epoch = close.engine.world.next_world_timeline_epoch(1)?;
            close.engine.world.append_world_timeline_epoch(epoch)?;
        } else {
            close
                .engine
                .world
                .get_mut(BUYER)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            set_ic(&mut close, expires)?;
        }
        fire(&mut close)?;
        let (request, proof) = next(&mut close, GOD, SERVICE_CLOSE)?;
        assert_eq!(
            request.parameters,
            json!({}),
            "private service has no public selector fields"
        );
        for corrupt in 0..3 {
            let mut wrong = (*request).clone();
            match corrupt {
                0 => wrong.key = IdempotencyKeyText::parse_canonical("unowned.house.request")?,
                1 => {
                    wrong.parameters =
                        json!({"operation":token(SERIAL+99),"reason":"deadline_reached"})
                }
                _ => wrong.actor = BUYER,
            }
            assert!(land(&mut close, &wrong, SERIAL + 80 + corrupt).is_err());
            assert_eq!(balance(&close, payer)?, money);
        }
        tick(&mut close)?;
        none(&mut close);
        proof.store(true, Ordering::Release);
        tick(&mut close)?;
        let (retry, _) = next(&mut close, GOD, SERVICE_CLOSE)?;
        assert_eq!(retry.key, request.key);
        assert_eq!(retry.action, request.action);
        assert_eq!(retry.parameters, request.parameters);
        land(&mut close, &retry, SERIAL + 20 + u128::from(invalidated))?;
        closed(
            &close,
            SERIAL,
            if invalidated {
                RenewalServiceCloseReason::ClockInvalidated
            } else {
                RenewalServiceCloseReason::DeadlineReached
            },
        )?;
        current_same(&close, &old, &predecessor)?;
        assert_eq!(balance(&close, payer)?, money);
        let _cold = boot(f, &saved(&close))?;
    }
    Ok(())
}
/// Actual registered GC must preserve both terminal holds needed for cold proof.
pub(super) fn retention(f: &Fixture, snapshot: &Durable, n: u128) -> TestResult {
    let mut w = boot(f, snapshot)?;
    let before = operation(&w, n)?;
    let hold = match before.state() {
        RenewalStateV1::Granted { funding, .. } | RenewalStateV1::Refunded { funding, .. } => {
            funding.hold()
        }
        _ => return Err("terminal renewal required".into()),
    };
    let at = w
        .engine
        .world
        .now()
        .checked_add(mush_economy::HOLD_GC_AFTER_SECS + 60)
        .ok_or("GC clock")?;
    w.engine.world.set_now(at);
    fire(&mut w)?;
    assert_eq!(
        economy(&w)?.last_sweep().0,
        at,
        "actual registered Economy sweep ran"
    );
    assert!(
        economy(&w)?.snapshot().hold(hold).is_some(),
        "retained terminal hold"
    );
    let audit = economy(&w)?.house_renewal_audit()?;
    before.verify_history(&audit.get(token(n))?)?;
    let cold = boot(f, &saved(&w))?;
    assert_eq!(operation(&cold, n)?, before);
    assert!(economy(&cold)?.snapshot().hold(hold).is_some());
    Ok(())
}
