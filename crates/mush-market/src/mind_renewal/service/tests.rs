//! Volatile custody and strict data tests. The observed queue is real; its
//! backend latch models consumer disposition only, not durable money or grants.
use super::*;
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::submission_observation::{
    SubmissionDisposition, SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{Engine, GameModule, Inbound, Outbox, Registry};
use std::sync::atomic::{AtomicBool, Ordering};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<SubmissionObservation>>>);
struct Fixture(Captured);
impl GameModule for Fixture {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let descriptor = DefinitionDescriptor::new(
            DefinitionKind::new("attempt_fixture").map_err(super::super::config::error)?,
            DefinitionId::new("renewal_custody").map_err(super::super::config::error)?,
            1,
            serde_json::json!({"scope":"volatile selector only"}),
        )
        .map_err(super::super::config::error)?;
        reg.definition_validator(
            descriptor.kind().clone(),
            crate::lot::exact_validator(&[descriptor.clone()]),
        )?;
        let action = reg.definition(descriptor)?;
        let capture = self.0.clone();
        reg.classify_commands(CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "@capture-renewal".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let observed = ctx.submit_authoritative_observed(
                    action.clone(),
                    PerceivedRef::SelfActor,
                    vec![],
                    serde_json::json!({}),
                    IdempotencyKeyText::parse_canonical("renewal.capture")
                        .map_err(|_| unavailable())?,
                )?;
                capture.0.lock().map_err(|_| unavailable())?.push(observed);
                Ok(())
            }),
        )
    }
}
struct Reader(Arc<AtomicBool>);
impl SubmissionDispositionReader for Reader {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Backend;
impl SubmissionRuntime for Backend {
    fn observe(&self, _: &mush_core::AuthoritativeSubmit) -> SubmissionRuntimeState {
        let proof = Arc::new(AtomicBool::new(false));
        SubmissionRuntimeState::new(Arc::new(Reader(proof.clone())), Box::new(proof))
    }
}
fn capture(
    engine: &mut Engine,
    slot: &Captured,
) -> TestResult<(DefinitionRef, SubmissionObservation, Arc<AtomicBool>)> {
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor: GOD,
            line: "@capture-renewal".into(),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    let observed = slot
        .0
        .lock()
        .map_err(|_| "capture lock")?
        .pop()
        .ok_or("actual observation")?;
    let driven = engine
        .take_observed_authoritative_submit()
        .ok_or("real queue")?;
    let (request, proof) = driven.into_parts();
    let proof = *proof
        .ok_or("runtime observer")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "proof type")?;
    Ok((request.action, observed, proof))
}
fn token(n: u128) -> CommitToken {
    CommitToken::from_bytes(n.to_be_bytes())
}
fn config() -> TestResult<RenewalConfig> {
    let row = super::super::tests::fixture()?;
    let [e, r] = row.participants();
    Ok(RenewalConfig::new(
        row.manifest().clone(),
        mush_roll::commerce::purchase::PurchaseParticipants::new(e.clone(), r.clone())?,
        mush_roll::commerce::renewal::writer::RenewalWriter::new(),
        super::super::RenewalPolicy {
            registry: mush_economy::managed_entitlement::GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            venue: None,
            offer_ic_seconds: 100,
        },
    )?)
}
fn intent() -> TestResult<CloseIntent> {
    let before = super::super::tests::fixture()?;
    Ok(CloseIntent {
        before,
        action: config()?
            .with_service()?
            .service_action()?
            .ok_or("sixth action")?,
        reason: Reason::DeadlineReached,
        key: IdempotencyKeyText::parse_canonical("exact.private.intent")?,
    })
}
#[test]
fn renewal_service_keeps_sixty_four_independent_unknown_observations_at_saturation() -> TestResult {
    let capture_slot = Captured::default();
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Fixture(capture_slot.clone()))],
    )?;
    engine.set_submission_runtime(Arc::new(Backend))?;
    let mut runtime = Runtime::default();
    let mut proofs = BTreeMap::new();
    for n in 1..=MAX_ENTRIES {
        let selection = Selection::Pending(token(u128::try_from(n)?));
        admit(&mut runtime, selection, GOD)?;
        let (action, observation, proof) = capture(&mut engine, &capture_slot)?;
        runtime
            .entries
            .get_mut(&key(selection))
            .ok_or("entry")?
            .attempt = Some((action, observation));
        proofs.insert(key(selection), proof);
    }
    let (_, observation, close_proof) = capture(&mut engine, &capture_slot)?;
    runtime.close = Some(intent()?);
    runtime.close_observation = Some(observation);
    assert_eq!(
        runtime.entries.len() + usize::from(runtime.close.is_some()),
        64
    );
    let before = runtime.close.as_ref().ok_or("intent")?.guard()?;
    assert!(admit(&mut runtime, Selection::Pending(token(64)), GOD).is_err());
    assert!(admit(&mut runtime, Selection::Pending(token(1)), ObjId(9)).is_err());
    assert!(admit(&mut runtime, Selection::Pending(token(1)), GOD).is_ok());
    for entry in runtime.entries.values_mut() {
        assert_eq!(
            entry.attempt.as_mut().ok_or("retained")?.1.take_terminal(),
            None
        );
    }
    assert_eq!(
        runtime
            .close_observation
            .as_mut()
            .ok_or("close")?
            .take_terminal(),
        None
    );
    assert_eq!(runtime.close.as_ref().ok_or("intent")?.guard()?, before);
    let selected = key(Selection::Pending(token(31)));
    proofs
        .get(&selected)
        .ok_or("exact proof")?
        .store(true, Ordering::Release);
    for (id, entry) in &mut runtime.entries {
        let observed = &mut entry.attempt.as_mut().ok_or("retained")?.1;
        assert_eq!(
            observed.take_terminal(),
            if id == &selected {
                Some(SubmissionDisposition::NoIntent)
            } else {
                None
            }
        );
        assert_eq!(observed.take_terminal(), None);
    }
    assert!(!close_proof.load(Ordering::Acquire));
    assert_eq!(
        runtime
            .close_observation
            .as_mut()
            .ok_or("close")?
            .take_terminal(),
        None
    );
    assert_eq!(
        runtime.entries.len(),
        MAX_ENTRIES,
        "absence observation alone does not evict custody"
    );
    Ok(())
}
#[test]
fn renewal_service_opt_in_preserves_five_actions_and_private_intent_is_exact() -> TestResult {
    let old = config()?;
    let refs = old.actions()?;
    let descriptors = old.declarations()?;
    assert!(old.service_action()?.is_none());
    let service = old.with_service()?;
    assert_eq!(service.actions()?, refs);
    let all = service.declarations()?;
    assert_eq!(all.len(), descriptors.len() + 1);
    for (a, b) in descriptors.iter().zip(all.iter()) {
        assert_eq!(a, b);
    }
    assert!(service.with_service().is_err());
    let original = intent()?;
    let guard = original.guard()?;
    let AuthorityGuard::Capability { capability, .. } = &guard else {
        return Err("expected exact service capability".into());
    };
    assert_eq!(
        capability.as_str().len(),
        mush_core::mechanics::CANONICAL_ID_MAX_BYTES
    );
    for changed in 0..3 {
        let mut next = original.clone();
        match changed {
            0 => next.key = IdempotencyKeyText::parse_canonical("different.private.intent")?,
            1 => next.reason = Reason::ClockInvalidated,
            _ => {
                let mut v = serde_json::to_value(&next.before)?;
                v.as_object_mut()
                    .ok_or("row")?
                    .insert("operation".into(), serde_json::json!(token(80)));
                next.before = serde_json::from_value(v)?;
            }
        }
        assert_ne!(next.guard()?, guard);
    }
    Ok(())
}
#[test]
fn renewal_service_state_closes_only_exact_reason_clock_and_receipt_shapes() -> TestResult {
    let original = intent()?;
    for invalidated in [false, true] {
        let mut value = serde_json::to_value(&original.before)?;
        let stamp = mush_core::ClockStamp {
            clock: mush_core::ClockRef::WorldIc {
                generation: if invalidated { 2 } else { 1 },
            },
            at: if invalidated { 100 } else { 200 },
        };
        let closure = serde_json::json!({"format":1,"commit":token(80),"action":original.action,"actor":GOD,"at_ic":stamp,"posted_at_utc":1001,"reason":if invalidated{"clock_invalidated"}else{"deadline_reached"}});
        value.as_object_mut().ok_or("row")?.insert(
            "state".into(),
            serde_json::json!({"kind":"unfunded_service_closed","closure":closure}),
        );
        let row: MindRenewalOperationV1 = serde_json::from_value(value.clone())?;
        row.receipt()?.matches_operation(&row)?;
        assert!(row.state().retryable());
        assert!(row.state().funding().is_none());
        let mut bad = value.clone();
        bad.pointer_mut("/state/closure")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("proof")?
            .insert("actor".into(), serde_json::json!(2));
        assert!(serde_json::from_value::<MindRenewalOperationV1>(bad).is_err());
        let mut bad = value.clone();
        bad.pointer_mut("/state/closure")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("proof")?
            .insert("authority".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<MindRenewalOperationV1>(bad).is_err());
        let mut bad = value.clone();
        bad.pointer_mut("/state/closure")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("proof")?
            .insert(
                "reason".into(),
                serde_json::json!(if invalidated {
                    "deadline_reached"
                } else {
                    "clock_invalidated"
                }),
            );
        assert!(serde_json::from_value::<MindRenewalOperationV1>(bad).is_err());
        let mut bad = value.clone();
        bad.pointer_mut("/state/closure")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("proof")?
            .insert("posted_at_utc".into(), serde_json::json!(999));
        assert!(serde_json::from_value::<MindRenewalOperationV1>(bad).is_err());
        let mut bad = value.clone();
        bad.pointer_mut("/state")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("state")?
            .insert("funding".into(), serde_json::json!({}));
        assert!(serde_json::from_value::<MindRenewalOperationV1>(bad).is_err());
        assert!(serde_json::to_vec(&row)?.len() < MindRenewalOperationV1::MAX_BYTES);
    }
    Ok(())
}

#[test]
fn renewal_rotation_visits_each_same_buyer_selection_and_preserves_generation_identity()
-> TestResult {
    let mut runtime = Runtime::default();
    let licence = LicenceId::new(1)?;
    let first = TermGeneration::new(1)?;
    let second = TermGeneration::new(2)?;
    let selections = [
        Selection::Due(licence, first, 1),
        Selection::Due(licence, second, 1),
        Selection::Due(licence, first, 2),
        Selection::Retry(licence, first, token(9)),
        Selection::Pending(token(9)),
    ];
    for selection in selections {
        admit(&mut runtime, selection, GOD)?;
    }
    assert_eq!(runtime.entries.len(), 5);
    let mut starts = BTreeSet::new();
    for _ in 0..5 {
        let order = rotation_order(&mut runtime);
        assert_eq!(order.len(), 5);
        starts.insert(order.first().ok_or("rotation")?.clone());
    }
    assert_eq!(
        starts,
        runtime.entries.keys().cloned().collect(),
        "an earlier unresolved selection cannot consume every same-buyer turn"
    );
    assert_eq!(
        runtime.entries.len(),
        5,
        "rotation never discards an unresolved entry"
    );
    Ok(())
}
