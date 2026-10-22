//! Actual registered continuations over the parent's genuine acquired licence.
//! These exercise selector custody and real paired installation; the observation
//! latch is a consumer fixture, not proof of when the server may emit NoIntent.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, OfferExecutionContext, OfferSubmission};
use mush_economy::managed_renewal::RenewalServiceCloseReason;
use std::sync::atomic::{AtomicBool, Ordering};
const SERVICE_CLOSE: &str = "close_mind_renewal_service";
const SERVICE_START: u128 = 9_100_000;
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
fn branch(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    let mut world = PlayerWorld::boot_renewal_service(f, saved)?;
    world.engine.set_submission_runtime(Arc::new(Runtime))?;
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x79; 32])?,
        [],
    )?);
    Ok(world)
}
fn fire(world: &mut PlayerWorld) -> TestResult {
    let now = world.engine.world.now();
    // Ordinary ticks enqueue exact continuations, then the actual scheduler
    // runs their registered handlers; no timer manufactures a player command.
    for _ in 0..4 {
        world.engine.fire_due(
            now.checked_mul(1000).ok_or("clock overflow")?,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        );
    }
    world.commit()?;
    Ok(())
}
fn next(
    world: &mut PlayerWorld,
    actor: ObjId,
    expected: &str,
) -> TestResult<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)> {
    let mut selected = world.engine.take_observed_authoritative_submit();
    for _ in 0..12 {
        if selected.is_some() {
            break;
        }
        tick_later(world)?;
        selected = world.engine.take_observed_authoritative_submit();
    }
    let driven = selected.ok_or("actual renewal selector")?;
    check!(driven.is_observed());
    let (request, proof) = driven.into_parts();
    check_eq!(request.actor, actor);
    check_eq!(request.action.key().owner.as_str(), "market");
    check_eq!(request.action.key().id.as_str(), expected);
    check_eq!(request.subject, PerceivedRef::SelfActor);
    check!(request.targets.is_empty());
    let proof = *proof
        .ok_or("runtime proof")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "proof type")?;
    check!(!proof.load(Ordering::Acquire));
    Ok((request, proof))
}
fn none(world: &mut PlayerWorld) {
    assert!(
        world.engine.take_observed_authoritative_submit().is_none(),
        "no second or automatic payment selector"
    );
}
fn land(world: &mut PlayerWorld, request: &AuthoritativeSubmit, number: u128) -> TestResult {
    let principal = PrincipalId::Object { id: request.actor };
    let query = world.engine.legal_action_query(
        principal.clone(),
        request.actor,
        request.subject.clone(),
    )?;
    let offers = world.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&request.action)
        .ok_or("actual renewal projection")?;
    let resolved = world.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(number.to_be_bytes()),
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
    let (action, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(action, request.action);
    let before = balances(world)?;
    let (prepared, _) = world
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(number),
        )?;
    check_eq!(balances(world)?, before, "prepare is inert");
    let (writes, install) = prepared.into_parts();
    apply(world, &writes, install)
}
fn advance(world: &mut PlayerWorld, at: i64) -> TestResult {
    world.engine.world.set_now(at);
    fire(world)
}
fn due(world: &mut PlayerWorld, birth: &EntitlementAllocationV1) -> TestResult {
    let current = licence(world, birth.licence())?;
    let term = term(world, current.id(), current.term())?;
    advance(world, term.window().term_end().get())
}
fn tick_later(world: &mut PlayerWorld) -> TestResult {
    advance(
        world,
        world.engine.world.now().checked_add(1).ok_or("clock")?,
    )
}
fn status(world: &mut PlayerWorld, actor: ObjId, id: LicenceId) -> TestResult<String> {
    let answer = world.say(actor, &format!("@mindrenewal/status {}", id.get()))?;
    let text = PlayerWorld::private_text(&answer, actor);
    check!(
        !text.contains(SOURCE) && !text.contains("this.landed"),
        "status is source-free"
    );
    Ok(text)
}
fn closed(
    world: &PlayerWorld,
    operation: CommitToken,
    reason: RenewalServiceCloseReason,
) -> TestResult {
    let row = paired(world, operation)?;
    let RenewalStateV1::UnfundedServiceClosed { closure } = row.state() else {
        return Err("actual service closure absent".into());
    };
    check_eq!(closure.reason(), reason);
    check_eq!(closure.actor(), GOD);
    check_eq!(closure.action().key().id.as_str(), SERVICE_CLOSE);
    Ok(())
}

pub(super) fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = branch(f, saved)?;
    let holder = birth.holder();
    let old_money = balances(&world)?;
    let initial = licence(&world, birth.licence())?;
    let old_term = term(&world, initial.id(), initial.term())?;
    let old_economics = economics(&world, initial.id(), initial.term())?;
    check!(status(&mut world, holder, birth.licence())?.contains("No renewal offer"));
    let foreign = if holder == GOD { BUYER } else { GOD };
    check!(!status(&mut world, foreign, birth.licence())?.contains("Price:"));
    check!(
        super::super::prepare(
            &mut world,
            "market",
            SERVICE_CLOSE,
            1,
            GOD,
            SERVICE_START,
            json!({"operation":token(START),"reason":"deadline_reached"})
        )
        .is_err(),
        "GOD has no service intent merely by invoking the action"
    );
    fire(&mut world)?;
    none(&mut world);
    due(&mut world, birth)?;
    let (draft, unknown) = next(&mut world, holder, DRAFT)?;
    check_eq!(draft.parameters, json!({"licence":birth.licence().get()}));
    tick_later(&mut world)?;
    none(&mut world);
    check_eq!(balances(&world)?, old_money);
    land(&mut world, &draft, SERVICE_START)?;
    check!(!unknown.load(Ordering::Acquire));
    tick_later(&mut world)?;
    none(&mut world);
    check!(matches!(
        paired(&world, token(SERVICE_START))?.state(),
        RenewalStateV1::Drafted {}
    ));
    let text = status(&mut world, holder, birth.licence())?;
    check!(
        text.contains("explicit acceptance")
            && text.contains("Price:")
            && text.contains("parent royalties:")
    );
    check_eq!(
        balances(&world)?,
        old_money,
        "automatic offer did not debit"
    );
    let draft_image = capture(&world, &disk(&world));
    world.say(
        holder,
        &format!("@mindrenewal/accept {}", birth.licence().get()),
    )?;
    let (fund, fund_unknown) = next(&mut world, holder, FUND)?;
    tick_later(&mut world)?;
    none(&mut world);
    land(&mut world, &fund, SERVICE_START + 1)?;
    check!(!fund_unknown.load(Ordering::Acquire));
    check!(matches!(
        paired(&world, token(SERVICE_START))?.state(),
        RenewalStateV1::Funded { .. }
    ));
    let held = balances(&world)?;
    tick_later(&mut world)?;
    let (grant, grant_unknown) = next(&mut world, holder, GRANT)?;
    let expiry = market_operation(&world, token(SERVICE_START))?
        .offer()
        .terms
        .expires_at
        .at;
    advance(&mut world, expiry)?;
    none(&mut world);
    check_eq!(
        balances(&world)?,
        held,
        "Unknown Grant does not authorize Refund or a second debit"
    );
    grant_unknown.store(true, Ordering::Release);
    tick_later(&mut world)?;
    let (refund, _) = next(&mut world, holder, REFUND)?;
    check_ne!(grant.action, refund.action);
    land(&mut world, &refund, SERVICE_START + 2)?;
    tick_later(&mut world)?;
    none(&mut world);
    check_eq!(balances(&world)?, old_money);
    assert_current_unchanged(&world, &initial, &old_term, &old_economics)?;
    let end = capture(&world, &disk(&world));
    let _ = PlayerWorld::boot_renewal_service(f, &end)?;

    // A healthy exact funded readback advances even if its acceptance ACK is
    // unknown. This is independent from the expiry/refund branch above.
    let mut healthy = branch(f, &draft_image)?;
    healthy.say(
        holder,
        &format!("@mindrenewal/accept {}", birth.licence().get()),
    )?;
    let (fund, _) = next(&mut healthy, holder, FUND)?;
    land(&mut healthy, &fund, SERVICE_START + 11)?;
    tick_later(&mut healthy)?;
    let (grant, _) = next(&mut healthy, holder, GRANT)?;
    land(&mut healthy, &grant, SERVICE_START + 12)?;
    tick_later(&mut healthy)?;
    none(&mut healthy);
    check!(matches!(
        paired(&healthy, token(SERVICE_START))?.state(),
        RenewalStateV1::Granted { .. }
    ));
    let end = capture(&healthy, &disk(&healthy));
    let _ = PlayerWorld::boot_renewal_service(f, &end)?;

    let mut declined = branch(f, &draft_image)?;
    declined.say(
        holder,
        &format!("@mindrenewal/decline {}", birth.licence().get()),
    )?;
    let (decline, _) = next(&mut declined, holder, CLOSE)?;
    land(&mut declined, &decline, SERVICE_START + 13)?;
    tick_later(&mut declined)?;
    none(&mut declined);
    check!(matches!(
        paired(&declined, token(SERVICE_START))?.state(),
        RenewalStateV1::UnfundedClosed { .. }
    ));
    check_eq!(balances(&declined)?, old_money);

    // Ordinary five-action retained offer upgrades to optional service cleanup
    // without changing the original offer or inferring money consent.
    let mut old = due_world(f, saved, birth)?;
    check!(
        super::super::action(&old, "market", SERVICE_CLOSE, 1).is_err(),
        "ordinary five-action composition has no service authority"
    );
    step(
        &mut old,
        DRAFT,
        SERVICE_START + 20,
        json!({"licence":birth.licence().get()}),
    )?;
    let original = market_operation(&old, token(SERVICE_START + 20))?
        .offer()
        .clone();
    let old_saved = capture(&old, &disk(&old));
    for invalidated in [false, true] {
        let mut close = branch(f, &old_saved)?;
        if invalidated {
            let epoch = close.engine.world.next_world_timeline_epoch(1)?;
            close.engine.world.append_world_timeline_epoch(epoch)?;
        } else {
            close.engine.world.set_now(original.terms.expires_at.at);
        }
        fire(&mut close)?;
        let (request, proof) = next(&mut close, GOD, SERVICE_CLOSE)?;
        check_eq!(
            request.parameters,
            json!({}),
            "service offer has no public parameters"
        );
        let unchanged = balances(&close)?;
        for corrupt in 0..3 {
            let mut wrong = (*request).clone();
            match corrupt {
                0 => {
                    wrong.key = mush_core::action::IdempotencyKeyText::parse_canonical(
                        "unowned.service.request",
                    )?
                }
                1 => {
                    wrong.parameters =
                        json!({"operation":token(SERVICE_START+80),"reason":"deadline_reached"})
                }
                _ => wrong.actor = BUYER,
            }
            check!(
                crate::test_checks::refused(land(
                    &mut close,
                    &wrong,
                    SERVICE_START + 80 + corrupt
                ))?,
                "only the actual private intent can publish cleanup"
            );
            check_eq!(balances(&close)?, unchanged);
        }
        tick_later(&mut close)?;
        none(&mut close);
        let reason = if invalidated {
            RenewalServiceCloseReason::ClockInvalidated
        } else {
            RenewalServiceCloseReason::DeadlineReached
        };
        let request = if invalidated {
            proof.store(true, Ordering::Release);
            tick_later(&mut close)?;
            let (retry, _) = next(&mut close, GOD, SERVICE_CLOSE)?;
            check_eq!(retry.key, request.key);
            check_eq!(retry.action, request.action);
            check_eq!(retry.parameters, request.parameters);
            retry
        } else {
            request
        };
        land(
            &mut close,
            &request,
            SERVICE_START + 21 + u128::from(invalidated),
        )?;
        closed(&close, token(SERVICE_START + 20), reason)?;
        check_eq!(proof.load(Ordering::Acquire), invalidated);
        check_eq!(
            market_operation(&close, token(SERVICE_START + 20))?.offer(),
            &original
        );
        check_eq!(balances(&close)?, old_money);
        tick_later(&mut close)?;
        none(&mut close);
        let ended = capture(&close, &disk(&close));
        let _ = PlayerWorld::boot_renewal_service(f, &ended)?;
        check!(
            crate::test_checks::refused(PlayerWorld::boot_renewals(f, &ended))?,
            "five-action composition cannot erase service proof"
        );
        if !invalidated {
            close.say(
                holder,
                &format!("@mindrenewal/retry {}", birth.licence().get()),
            )?;
            let (retry, _) = next(&mut close, holder, DRAFT)?;
            check_eq!(
                retry.parameters,
                json!({"previous_operation":token(SERVICE_START+20)})
            );
            land(&mut close, &retry, SERVICE_START + 24)?;
            tick_later(&mut close)?;
            none(&mut close);
            check!(matches!(
                paired(&close, token(SERVICE_START + 24))?.state(),
                RenewalStateV1::Drafted {}
            ));
            check_eq!(
                balances(&close)?,
                old_money,
                "retry is an unfunded offer only"
            );
        }
    }
    Ok(())
}

/// The original-v3 branch supplies a genuine non-service buyer. Its missing
/// actor must not strand an unfunded offer or permit service-funded cleanup.
pub(in super::super) fn unavailable_buyer(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    check_eq!(birth.holder(), BUYER);
    let mut world = branch(f, saved)?;
    due(&mut world, birth)?;
    let (draft, _) = next(&mut world, BUYER, DRAFT)?;
    land(&mut world, &draft, SERVICE_START + 40)?;
    tick_later(&mut world)?;
    none(&mut world);
    let drafted = capture(&world, &disk(&world));
    let money = balances(&world)?;
    let expires = market_operation(&world, token(SERVICE_START + 40))?
        .offer()
        .terms
        .expires_at
        .at;
    world
        .engine
        .world
        .get_mut(BUYER)
        .ok_or("buyer")?
        .flags
        .insert(mush_core::Flags::GOING);
    advance(&mut world, expires)?;
    let (close, _) = next(&mut world, GOD, SERVICE_CLOSE)?;
    land(&mut world, &close, SERVICE_START + 41)?;
    closed(
        &world,
        token(SERVICE_START + 40),
        RenewalServiceCloseReason::DeadlineReached,
    )?;
    check_eq!(balances(&world)?, money);
    let _ = PlayerWorld::boot_renewal_service(f, &capture(&world, &disk(&world)))?;

    let mut held = branch(f, &drafted)?;
    held.say(
        BUYER,
        &format!("@mindrenewal/accept {}", birth.licence().get()),
    )?;
    let (fund, _) = next(&mut held, BUYER, FUND)?;
    land(&mut held, &fund, SERVICE_START + 42)?;
    let funded = balances(&held)?;
    held.engine
        .world
        .get_mut(BUYER)
        .ok_or("buyer")?
        .flags
        .insert(mush_core::Flags::GOING);
    advance(&mut held, expires)?;
    none(&mut held);
    check_eq!(balances(&held)?, funded);
    check!(matches!(
        paired(&held, token(SERVICE_START + 40))?.state(),
        RenewalStateV1::Funded { .. }
    ));
    held.engine
        .world
        .get_mut(BUYER)
        .ok_or("buyer")?
        .flags
        .remove(mush_core::Flags::GOING);
    tick_later(&mut held)?;
    let (refund, _) = next(&mut held, BUYER, REFUND)?;
    land(&mut held, &refund, SERVICE_START + 43)?;
    check_eq!(balances(&held)?, money);
    Ok(())
}
