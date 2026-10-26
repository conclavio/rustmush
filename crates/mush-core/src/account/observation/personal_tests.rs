#![cfg(test)]

//! Single-account observations through the real registered provider/capsule.
use super::*;

#[test]
fn personal_capture_is_actual_requester_opaque_match_only_and_install_uses_latches() {
    let mut r = personal_rig();
    // Registry admission needs only its actor's account, not another customer.
    assert!(r.backend.unlink(r.other));
    let (prepared, _) = r.prepare().unwrap();
    r.unchanged();
    // The provider verifies matching/nonmatching identities and redacted Debug.
    // Caller JSON still names requester999 and cannot choose this capture.
    assert_eq!(r.answers.lock().unwrap().as_slice(), [Ok((GOD, GOD))]);
    assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    r.backend.mode.store(1, Ordering::SeqCst);
    let (_, install) = prepared.into_parts();
    r.engine
        .install_composed_action_announcing(
            install,
            &mut Outbox::new(),
            &mut crate::engine::Effects::new(),
        )
        .unwrap();
    assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.engine.action_state().get(&r.handles.family, &1).unwrap(),
        Some(&42)
    );
    assert_eq!(r.engine.world.module_state::<EffectState>().unwrap().0, 11);
}

#[test]
fn personal_and_pair_captures_share_one_exact_limit_and_caught_refusal_is_sticky() {
    for exceeded in [false, true] {
        let mut r = personal_rig();
        r.mode.store(if exceeded { 8 } else { 7 }, Ordering::SeqCst);
        let result = r.prepare();
        let answers = r.answers.lock().unwrap();
        assert_eq!(
            answers.iter().filter(|row| **row == Ok((GOD, GOD))).count(),
            32
        );
        assert_eq!(
            answers
                .iter()
                .filter(|row| **row == Ok((GOD, r.other)))
                .count(),
            32
        );
        assert_eq!(
            r.backend.observations.load(Ordering::SeqCst),
            MAX_ACCOUNT_OBSERVATIONS
        );
        if exceeded {
            assert!(result.is_err());
            assert_eq!(answers.len(), MAX_ACCOUNT_OBSERVATIONS + 1);
            assert_eq!(answers.last(), Some(&Err(ParticipantError::Authority)));
            r.unchanged();
            assert!(!r.engine.action_state().has_prepared());
            assert!(!r.engine.action_state().reservation_lost());
        } else {
            assert!(result.is_ok());
            assert_eq!(answers.len(), MAX_ACCOUNT_OBSERVATIONS);
        }
    }
}

#[test]
fn personal_unknown_foreign_capture_race_and_exhaustion_refuse_even_if_provider_catches() {
    for case in 0..4 {
        let mut r = personal_rig();
        match case {
            0 => assert!(r.backend.unlink(GOD)),
            1 => r.backend.mode.store(1, Ordering::SeqCst),
            2 => r.backend.mode.store(2, Ordering::SeqCst),
            _ => r.backend.epoch.0.store(u64::MAX, Ordering::SeqCst),
        }
        assert!(r.prepare().is_err(), "case {case}");
        assert_eq!(
            r.answers.lock().unwrap().as_slice(),
            [Err(ParticipantError::Authority)]
        );
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
        r.unchanged();
        assert!(!r.engine.action_state().reservation_lost());
    }
}

#[test]
fn personal_principal_player_and_legacy_capture_support_are_required() {
    struct Legacy;
    impl AccountIdentity for Legacy {
        fn account_of(&self, _: ObjId) -> Option<AccountId> {
            Some(AccountId::mint("requester", 1).unwrap())
        }
    }
    for case in 0..6 {
        let mut r = rig_kind(true, case != 4);
        match case {
            0 => r.invocation = invocation(PrincipalId::Object { id: r.other }),
            1 => r
                .engine
                .world
                .get_mut(GOD)
                .unwrap()
                .flags
                .insert(Flags::GOING),
            2 => r.engine.world.get_mut(GOD).unwrap().kind = ObjKind::Thing,
            3 => r.engine.set_account_identity(Arc::new(Legacy)),
            4 => {}
            _ => {
                r.invocation = invocation(PrincipalId::Scoped {
                    owner: OwnerId::new("account_test").unwrap(),
                    class: CanonicalId::new("service").unwrap(),
                    id: crate::action::IdempotencyKeyText::parse_canonical("requester").unwrap(),
                })
            }
        }
        assert!(r.prepare().is_err(), "case {case}");
        assert!(r.answers.lock().unwrap().last().unwrap().is_err());
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 0);
        r.unchanged();
    }
}

#[test]
fn personal_direct_unlink_relink_and_oracle_replacement_invalidate_old_capture() {
    for replacement in [false, true] {
        let mut r = personal_rig();
        let (prepared, _) = r.prepare().unwrap();
        if replacement {
            r.engine
                .set_account_identity(Arc::new(Backend::new(r.other)));
        } else {
            r.backend.unlink_relink(GOD);
        }
        assert!(prepared.recheck(&r.engine.world).is_err());
        let (_, install) = prepared.into_parts();
        assert!(
            r.engine
                .install_composed_action_announcing(
                    install,
                    &mut Outbox::new(),
                    &mut crate::engine::Effects::new(),
                )
                .is_err()
        );
        r.unchanged();
        assert!(r.engine.action_state().reservation_lost());
    }
}

#[test]
fn personal_final_owner_recheck_or_bind_cannot_invalidate_then_install_any_image() {
    for mode in [3, 4] {
        let mut r = personal_rig();
        let (prepared, _) = r.prepare().unwrap();
        r.mode.store(mode, Ordering::SeqCst);
        if mode == 3 {
            assert!(prepared.recheck(&r.engine.world).is_err());
        }
        let (_, install) = prepared.into_parts();
        assert!(
            r.engine
                .install_composed_action_announcing(
                    install,
                    &mut Outbox::new(),
                    &mut crate::engine::Effects::new(),
                )
                .is_err()
        );
        r.unchanged();
        assert!(r.engine.action_state().reservation_lost());
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn personal_stage_and_take_refuse_stale_capture_with_known_abort() {
    for after_stage in [false, true] {
        let mut r = personal_rig();
        let (prepared, output) = r.prepare().unwrap();
        let bundle = bundle(&r, &prepared, output);
        let guards = crate::action::ServerGuardSetBuilder::new(
            crate::action::RevisionSet::default(),
            r.engine.registry.mechanics_owner_readiness.clone().unwrap(),
        )
        .finish()
        .unwrap();
        let _ = r.engine.world.take_dirty();
        if after_stage {
            r.engine
                .stage_authoritative_action(prepared, guards, bundle)
                .unwrap();
            r.backend.unlink_relink(GOD);
            assert!(r.engine.take_staged_action().is_err());
        } else {
            r.backend.unlink_relink(GOD);
            assert!(
                r.engine
                    .stage_authoritative_action(prepared, guards, bundle)
                    .is_err()
            );
        }
        r.unchanged();
        assert!(!r.engine.has_staged_action());
        assert!(!r.engine.action_state().has_prepared());
        assert!(!r.engine.action_state().reservation_lost());
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    }
}
