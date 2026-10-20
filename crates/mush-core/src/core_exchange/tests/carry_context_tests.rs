#![cfg(test)]

use super::*;
use crate::action_state::{DetachedTypedState, TypedStateLens};
fn require_context(r: &Rig) {
    *r.handles.hook.expected_exchange.lock().unwrap() = Some(ExpectedExchange {
        facts: r.data.0.lock().unwrap().0.clone(),
        action: action_ref(),
        commit: token(7),
        participants: [owner_ref::<1>(), owner_ref::<2>()],
    });
}
#[test]
fn exact_exchange_context_reaches_both_real_hooks_until_custody_only() {
    for separate in [false, true] {
        let mut r = rig(true, separate);
        require_context(&r);
        let prepared = r.prepare(&[]).unwrap();
        let prepared_calls = r.handles.hook.exchange_calls.load(Ordering::SeqCst);
        assert!(prepared_calls >= 2);
        r.stage(prepared, &[]).unwrap();
        let staged_calls = r.handles.hook.exchange_calls.load(Ordering::SeqCst);
        assert!(staged_calls > prepared_calls);
        let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
        let custody_calls = r.handles.hook.exchange_calls.load(Ordering::SeqCst);
        assert!(custody_calls > staged_calls);
        r.engine.install_composed_action(installation).unwrap();
        assert_eq!(
            r.handles.hook.exchange_calls.load(Ordering::SeqCst),
            custody_calls
        );
        assert_eq!(r.handles.hook.ordinary_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            r.engine.world.get(r.incoming).unwrap().location,
            Some(r.hull)
        );
        assert_eq!(
            r.engine.world.get(r.outgoing.unwrap()).unwrap().location,
            r.return_to
        );
    }
}
#[test]
fn ordinary_and_detached_carries_cannot_claim_exchange_context_from_same_tuple_or_parameters() {
    let mut r = rig(true, false);
    require_context(&r);
    let before = r.objects();
    for (object, destination) in [
        (r.incoming, r.hull),
        (r.outgoing.unwrap(), r.return_to.unwrap()),
    ] {
        assert!(
            crate::module::check_carry_attempt(
                &r.engine.registry,
                &r.engine.world,
                r.actor,
                object,
                destination,
                TypedStateLens::Live(r.engine.action_state())
            )
            .is_err()
        );
    }
    let detached = crate::HookReadContext::without_typed_state(&r.engine.world, "industry");
    assert!(detached.core_exchange_attempt().is_none());
    assert_eq!(r.handles.hook.exchange_calls.load(Ordering::SeqCst), 0);
    assert_eq!(r.handles.hook.ordinary_calls.load(Ordering::SeqCst), 2);
    // Public request parameters cannot install a context or replace its actual
    // commit/order. Even this permissive fixture sees only Core's real capture.
    let actual = r.invocation(&[]);
    let inv = ActionInvocation::new(ActionInvocationDraft {
        request: actual.request(), principal: actual.principal().clone(), actor: actual.actor(),
        offered_action: actual.offered_action(), subject: actual.subject().clone(),
        targets: actual.targets().to_vec(), parameters: json!({"order":7,"core_exchange_attempt":{"order":999,"commit":token(99),"action":action_ref()}}),
        idempotency: actual.idempotency().clone(),
    }).unwrap();
    let prepared = r
        .engine
        .prepare_registered_composed_authoritative_action(&r.handles.action, &inv, token(7))
        .unwrap();
    assert!(r.handles.hook.exchange_calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(r.objects(), before);
    r.install(prepared).unwrap();
}
#[test]
fn captured_exchange_rejects_crossed_move_tuple_and_repeats_exact_identity_at_stage_and_take() {
    let r = rig(true, false);
    require_context(&r);
    let attempt = ExchangeCarryAttempt::capture(
        &r.data.0.lock().unwrap().0,
        &action_ref(),
        token(7),
        &[owner_ref::<1>(), owner_ref::<2>()],
    )
    .unwrap();
    for (actor, object, destination) in [
        (GOD, r.incoming, r.hull),
        (r.actor, r.outgoing.unwrap(), r.hull),
        (r.actor, r.incoming, r.return_to.unwrap()),
    ] {
        assert!(attempt.require_move(actor, object, destination).is_err());
    }
    let ordinary = crate::core_delivery::CarryCheck::capture(
        Arc::from([]),
        &r.engine.world,
        TypedStateLens::Detached(DetachedTypedState::CompatibilityCtx),
        r.actor,
        r.incoming,
        r.hull,
    );
    assert!(ordinary.is_err());
    for stage in [false, true] {
        let mut r = rig(true, false);
        require_context(&r);
        let before = r.objects();
        let prepared = r.prepare(&[]).unwrap();
        if stage {
            r.stage(prepared, &[]).unwrap();
            r.handles
                .hook
                .expected_exchange
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .commit = token(99);
            assert!(r.engine.take_staged_action().is_err());
        } else {
            r.handles
                .hook
                .expected_exchange
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .commit = token(99);
            assert!(r.stage(prepared, &[]).is_err());
        }
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
    }
}
