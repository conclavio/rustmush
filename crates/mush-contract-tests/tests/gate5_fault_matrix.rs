//! K8.1: reusable persistence cut points exercised by real Gate-1, tactical,
//! and delayed-decision lifecycle bundles rather than only store-local
//! synthetic writes.

use mush_contract_tests::tactical_gate::{
    install_tactical_gate_unit, tactical_gate_fire_invocation, tactical_gate_world,
};
use mush_contract_tests::{
    install_toy_focus, toy_brace_invocation, toy_definitions, toy_world_with_state,
};
use mush_core::{
    ActionInvocation, ActionOfferId, AuthoritativeBundle, AuthoritativeBundleDraft,
    AuthoritativeSemanticEvent, CanonicalHash, CanonicalId, ClockRef, ClockStamp, CommitToken,
    DecisionAdmissionIndex, DecisionAttemptFailureKind, DecisionCaptureFamilies,
    DecisionCapturePlan, DecisionCause, DecisionCompletionFence, DecisionControllerId, DecisionId,
    DecisionKeyRing, DecisionPrompt, DecisionPromptRecord, DecisionPromptStateCodec,
    DecisionSecret, DecisionTerminalReceiptStateCodec, DecisionToken, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DurableScheduleStateCodec, Engine, FallbackPolicyRef, GameModule, IdempotencyKey,
    IdempotencyKeyText, IdempotencyScope, ModError, OwnerId, PendingDecision,
    PendingDecisionStateCodec, PendingDecisionStatus, PendingOfferWitness,
    PreparedAuthoritativeAction, PreparedCausalRef, PreparedTxn, PrincipalId,
    ProjectedDecisionContext, ProjectedLegalAction, ProjectionScopeId, Registrar, Registry,
    RequestUid, RulesValue, ScopedIdempotencyKey, StateFamilyHandle, StateFamilyId, SubjectKey,
    World, decision_prompt_state_family_descriptor,
    decision_terminal_receipt_state_family_descriptor, durable_schedule_state_family_descriptor,
    pending_decision_state_family_descriptor,
};
use mush_store::{
    AuthoritativeFaultMode, AuthoritativeFaultStore, AuthoritativeOutboxAttemptResult,
    AuthoritativeStore, BundleStatus,
};
use mush_tactics::TacticalUnit;
use std::sync::{Arc, Mutex};

fn bundle_from_prepared(
    engine: &mut Engine,
    action: DefinitionRef,
    invocation: &ActionInvocation,
    token: CommitToken,
    prepared: PreparedTxn,
    output: PreparedAuthoritativeAction,
) -> AuthoritativeBundle {
    let provider = prepared.owner().clone();
    let writes = prepared.authoritative_replay_writes();
    engine.abort_prepared_transaction(prepared);
    let (
        outcome,
        prepared_resolution,
        mut definitions,
        mut eligible_witnesses,
        mut durable_outbox,
        semantic_events,
    ) = output.into_parts();
    definitions.push(action.clone());
    definitions.push(outcome.schema().clone());
    definitions.sort();
    definitions.dedup();
    if eligible_witnesses.is_empty() {
        eligible_witnesses.push(invocation.principal().clone());
    }
    eligible_witnesses.sort();
    eligible_witnesses.dedup();
    durable_outbox.sort_by_key(mush_core::DurableOutboxIntent::ordinal);
    let resolution = prepared_resolution.map(|prepared| prepared.bind(token));
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: invocation.request(),
        idempotency: invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([0x51; 32]),
        request_hash: invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([0x52; 32]),
        action,
        provider,
        cause: PreparedCausalRef::new(invocation.request(), None, 0).expect("root cause"),
        definitions,
        writes,
        outcome,
        resolution,
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })
    .expect("real provider output forms an authoritative bundle")
}

fn gate1_bundle() -> AuthoritativeBundle {
    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, mush_core::GOD, 3);
    let action = toy_definitions().brace;
    let invocation = toy_brace_invocation(mush_core::GOD, 11, RequestUid::from_bytes([0x81; 16]));
    let token = CommitToken::from_bytes([0x82; 16]);
    let (prepared, output) = world
        .engine_mut()
        .prepare_registered_authoritative_action(&action, &invocation, token)
        .expect("Gate-1 provider prepares");
    bundle_from_prepared(
        world.engine_mut(),
        action,
        &invocation,
        token,
        prepared,
        output,
    )
}

fn tactical_bundle() -> AuthoritativeBundle {
    let (mut engine, state, definitions, spec) = tactical_gate_world();
    let source = mush_core::ObjId(901);
    let target = mush_core::ObjId(902);
    install_tactical_gate_unit(
        &mut engine,
        &state,
        TacticalUnit::from_spec(&spec, source).expect("valid source"),
    );
    install_tactical_gate_unit(
        &mut engine,
        &state,
        TacticalUnit::from_spec(&spec, target).expect("valid target"),
    );
    let action = definitions.fire;
    let invocation =
        tactical_gate_fire_invocation(source, target, 0, 0, RequestUid::from_bytes([0x91; 16]));
    let token = CommitToken::from_bytes([0x92; 16]);
    let (prepared, output) = engine
        .prepare_registered_authoritative_action(&action, &invocation, token)
        .expect("tactical provider prepares");
    bundle_from_prepared(&mut engine, action, &invocation, token, prepared, output)
}

type CapturedDecisionFamilies = (
    DecisionCaptureFamilies,
    StateFamilyHandle<DecisionId, PendingDecision>,
);

#[derive(Debug)]
struct DecisionCaptureModule {
    captured: Arc<Mutex<Option<CapturedDecisionFamilies>>>,
}

impl GameModule for DecisionCaptureModule {
    fn name(&self) -> &'static str {
        "test_decisions"
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        let pending = registrar.state_family(
            pending_decision_state_family_descriptor(
                StateFamilyId::new("pending_decisions").expect("static pending family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
        )?;
        let prompts = registrar.state_family(
            decision_prompt_state_family_descriptor(
                StateFamilyId::new("decision_prompts").expect("static prompt family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
        )?;
        let deadlines = registrar.state_family(
            durable_schedule_state_family_descriptor(
                StateFamilyId::new("decision_deadlines").expect("static deadline family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
        )?;
        let terminal = registrar.state_family(
            decision_terminal_receipt_state_family_descriptor(
                StateFamilyId::new("decision_terminal_receipts")
                    .expect("static terminal family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
        )?;
        let families = DecisionCaptureFamilies::new(pending.clone(), prompts, deadlines, terminal)
            .map_err(|error| ModError::msg(error.to_string()))?;
        *self.captured.lock().expect("capture mutex poisoned") = Some((families, pending));
        Ok(())
    }
}

fn decision_sibling_bundle(
    base: &AuthoritativeBundle,
    identity_byte: u8,
    phase: &str,
    writes: Vec<mush_core::ReplayWrite>,
) -> AuthoritativeBundle {
    let request = RequestUid::from_bytes([identity_byte; 16]);
    let cause = PreparedCausalRef::new(request, None, 0).unwrap();
    let outcome = RulesValue::new(
        base.outcome().schema().clone(),
        serde_json::json!({ "phase": phase }),
    )
    .unwrap();
    let event = AuthoritativeSemanticEvent::new(
        cause,
        base.provider().clone(),
        CanonicalId::new("decision_lifecycle").unwrap(),
        outcome.clone(),
        base.eligible_witnesses().to_vec(),
    )
    .unwrap();
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([identity_byte.wrapping_add(1); 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            base.idempotency().scope().clone(),
            base.idempotency().epoch(),
            IdempotencyKey::Deterministic(CanonicalHash::from_bytes([identity_byte; 32])),
        ),
        ingress_hash: CanonicalHash::from_bytes([identity_byte.wrapping_add(2); 32]),
        request_hash: CanonicalHash::from_bytes([identity_byte.wrapping_add(3); 32]),
        plan_hash: CanonicalHash::from_bytes([identity_byte.wrapping_add(4); 32]),
        action: base.action().clone(),
        provider: base.provider().clone(),
        cause,
        definitions: base.definitions().to_vec(),
        writes,
        outcome,
        resolution: None,
        semantic_events: vec![event],
        eligible_witnesses: base.eligible_witnesses().to_vec(),
        durable_outbox: vec![],
    })
    .unwrap()
}

fn delayed_capture_bundles() -> Vec<(&'static str, AuthoritativeBundle)> {
    let owner = OwnerId::new("test_decisions").unwrap();
    let actor = mush_core::ObjId(7);
    let principal = PrincipalId::Object { id: actor };
    let controller = DecisionControllerId {
        owner: owner.clone(),
        key: CanonicalId::new("guard_7").unwrap(),
    };
    let cause = DecisionCause {
        kind: CanonicalId::new("npc_turn").unwrap(),
        instance: IdempotencyKeyText::parse_canonical("scene-4/turn-11").unwrap(),
        replaceable_key: None,
    };
    let action_idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: owner.clone(),
            namespace: CanonicalId::new("decision_action").unwrap(),
        },
        3,
        IdempotencyKey::Deterministic(CanonicalHash::from_bytes([0x22; 32])),
    );
    let id = DecisionId::derive(&controller, &cause, &action_idempotency).unwrap();
    let offer = ActionOfferId(CanonicalHash::from_bytes([0x44; 32]));
    let keys = DecisionKeyRing::new(
        DecisionSecret::new(2, [0xA5; 32]).unwrap(),
        [DecisionSecret::new(1, [0x5A; 32]).unwrap()],
    )
    .unwrap();
    let projected: ProjectedLegalAction = serde_json::from_value(serde_json::json!({
        "offer": offer,
        "subject": { "kind": "self_actor" },
        "parameters": [],
        "targets": { "kind": "none" },
        "required_channels": [],
        "expires": null,
        "presentation": {
            "verb": "withdraw",
            "label": "Withdraw",
            "help": "Move toward safety."
        }
    }))
    .unwrap();
    let context = ProjectedDecisionContext::new(
        CanonicalId::new("tactical_turn.v1").unwrap(),
        serde_json::json!({ "situation": "under fire" }),
    )
    .unwrap();
    let mut prompt = DecisionPrompt {
        token: DecisionToken {
            id,
            revision: 1,
            prompt_digest: CanonicalHash::from_bytes([1; 32]),
            key_version: 2,
            tag: CanonicalHash::from_bytes([1; 32]),
        },
        actions: vec![projected],
        context,
    };
    let prompt_digest = prompt.digest().unwrap();
    prompt.token = keys.sign(id, 1, prompt_digest);
    let prompt = DecisionPromptRecord::new(prompt).unwrap();

    let mut fallback_definitions = DefinitionRegistryBuilder::new();
    let fallback = FallbackPolicyRef(
        fallback_definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("fallback").unwrap(),
                    DefinitionId::new("withdraw").unwrap(),
                    1,
                    serde_json::json!({ "policy": "withdraw" }),
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let pending = PendingDecision {
        id,
        revision: 1,
        controller,
        principal: principal.clone(),
        actor,
        truth_subject: SubjectKey::Object { id: actor },
        viewer_scope: ProjectionScopeId::derive(&principal, 9).unwrap(),
        cause,
        offered: vec![PendingOfferWitness {
            offer,
            common_revisions: Vec::new(),
            target_revisions: Vec::new(),
            bridge_definitions: Vec::new(),
        }],
        prompt_digest,
        token_key_version: 2,
        deadline: ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: 4,
                generation: 2,
            },
            at: 20,
        },
        fallback,
        action_idempotency: action_idempotency.clone(),
        malformed_attempts: 0,
        attempt_audit: Vec::new(),
        status: PendingDecisionStatus::Ready { attempt: 1 },
    };
    pending.validate().unwrap();
    let lifecycle_pending = pending.clone();
    let lifecycle_prompt = prompt.clone();
    let admission = DecisionAdmissionIndex::default()
        .admit(&pending, &prompt, &keys)
        .unwrap();
    let plan = DecisionCapturePlan::new(
        &owner,
        pending,
        prompt,
        admission,
        &keys,
        CanonicalId::new("decision_wake").unwrap(),
        0,
    )
    .unwrap();
    let wake_record = plan.wake();

    let captured = Arc::new(Mutex::new(None));
    let module = DecisionCaptureModule {
        captured: Arc::clone(&captured),
    };
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(module)],
    )
    .unwrap();
    let (capture_families, pending_family) = captured
        .lock()
        .expect("capture mutex poisoned")
        .clone()
        .expect("decision families captured");
    let mut txn = engine.action_transaction(&pending_family).unwrap();
    let wake = plan.stage(&mut txn, &capture_families).unwrap();
    let prepared = txn.prepare().unwrap();
    let writes = prepared.authoritative_replay_writes();
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);

    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("capture_decision").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome_definition = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").unwrap(),
                DefinitionId::new("decision_captured").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome = RulesValue::new(
        outcome_definition.clone(),
        serde_json::json!({ "decision": id }),
    )
    .unwrap();
    let request = RequestUid::from_bytes([0xA1; 16]);
    let root_cause = PreparedCausalRef::new(request, None, 0).unwrap();
    let event = AuthoritativeSemanticEvent::new(
        root_cause,
        owner.clone(),
        CanonicalId::new("capture_decision").unwrap(),
        outcome.clone(),
        vec![principal.clone()],
    )
    .unwrap();
    let capture = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([0xA2; 16]),
        request,
        idempotency: action_idempotency,
        ingress_hash: CanonicalHash::from_bytes([0xA3; 32]),
        request_hash: CanonicalHash::from_bytes([0xA4; 32]),
        plan_hash: CanonicalHash::from_bytes([0xA5; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: root_cause,
        definitions: vec![action, outcome_definition],
        writes,
        outcome,
        resolution: None,
        semantic_events: vec![event],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![wake],
    })
    .unwrap();

    let lease = lifecycle_pending
        .plan_worker_lease(
            &lifecycle_prompt,
            &wake_record,
            &keys,
            &ClockStamp {
                clock: ClockRef::SceneTurn {
                    scene: 4,
                    generation: 2,
                },
                at: 10,
            },
            5,
        )
        .unwrap();
    let leased = lease.post_image().clone();
    let mut txn = engine.action_transaction(&pending_family).unwrap();
    let worker = lease.stage(&mut txn, &capture_families).unwrap();
    let prepared = txn.prepare().unwrap();
    let lease_writes = prepared.authoritative_replay_writes();
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
    let lease_bundle = decision_sibling_bundle(&capture, 0xB0, "leased", lease_writes);

    let fence = DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        worker.attempt(),
        worker.lease_until().clone(),
    )
    .unwrap();
    let attempt = leased
        .plan_attempt_failure(&fence, DecisionAttemptFailureKind::Unavailable, 3)
        .unwrap();
    let retry_ready = attempt.post_image().clone();
    let mut txn = engine.action_transaction(&pending_family).unwrap();
    attempt.stage(&mut txn, &capture_families).unwrap();
    let prepared = txn.prepare().unwrap();
    let attempt_writes = prepared.authoritative_replay_writes();
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
    let attempt_bundle = decision_sibling_bundle(&capture, 0xC0, "attempt_failed", attempt_writes);

    let terminal = retry_ready
        .plan_cancellation(
            &retry_ready.controller,
            retry_ready.id,
            retry_ready.revision,
        )
        .unwrap();
    let terminal_pending = terminal.post_image().clone();
    let mut txn = engine.action_transaction(&pending_family).unwrap();
    terminal.stage(&mut txn, &capture_families).unwrap();
    let prepared = txn.prepare().unwrap();
    let terminal_writes = prepared.authoritative_replay_writes();
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
    let terminal_bundle =
        decision_sibling_bundle(&capture, 0xD0, "terminal_cancelled", terminal_writes);

    let compaction = terminal_pending.plan_compaction().unwrap();
    let mut txn = engine.action_transaction(&pending_family).unwrap();
    compaction.stage(&mut txn, &capture_families).unwrap();
    let prepared = txn.prepare().unwrap();
    let compaction_writes = prepared.authoritative_replay_writes();
    engine.abort_prepared_transaction(prepared);
    let compaction_bundle =
        decision_sibling_bundle(&capture, 0xE0, "terminal_compacted", compaction_writes);

    vec![
        ("Gate-4 delayed capture", capture),
        ("Gate-4 worker lease", lease_bundle),
        ("Gate-4 provider attempt", attempt_bundle),
        ("Gate-4 terminalization", terminal_bundle),
        ("Gate-4 compaction", compaction_bundle),
    ]
}

async fn assert_fault_matrix(name: &str, bundle: AuthoritativeBundle) {
    let mut modes = vec![
        AuthoritativeFaultMode::BeforeIntent,
        AuthoritativeFaultMode::AfterIntent,
    ];
    modes.extend((0..=bundle.writes().len()).map(AuthoritativeFaultMode::AfterNMaterializationOps));
    modes.extend([
        AuthoritativeFaultMode::AckLostAfterFinalize,
        AuthoritativeFaultMode::AckLostAfterCheckpoint,
        AuthoritativeFaultMode::TokenConflict,
    ]);

    for mode in modes {
        let mut fault = AuthoritativeFaultStore::new(mode);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        if fault
            .reserve(bundle.clone())
            .await
            .unwrap_or_else(|error| panic!("{name} {mode:?} reserve failed: {error}"))
            .is_none()
        {
            fault
                .reserve(bundle.clone())
                .await
                .unwrap_or_else(|error| panic!("{name} {mode:?} reserve retry failed: {error}"))
                .expect("healed reserve returns its exact receipt");
        }

        let first_reconcile = fault.reconcile(token, hash).await;
        if mode == AuthoritativeFaultMode::TokenConflict {
            assert!(first_reconcile.is_err(), "{name} token conflict must fail");
            assert_eq!(
                fault
                    .store()
                    .authoritative_status(token, hash)
                    .await
                    .expect("status after conflict"),
                BundleStatus::PendingExact,
                "{name} conflict cannot mutate the exact intent"
            );
        } else {
            first_reconcile
                .unwrap_or_else(|error| panic!("{name} {mode:?} reconcile failed: {error}"));
        }
        let receipt = fault
            .store()
            .reconcile_authoritative(token, hash)
            .await
            .unwrap_or_else(|error| panic!("{name} {mode:?} reconcile retry failed: {error}"));
        assert!(receipt.matches_bundle(&bundle), "{name} {mode:?}");

        for record in fault
            .store()
            .pending_authoritative_outbox()
            .await
            .unwrap_or_else(|error| panic!("{name} {mode:?} outbox discovery failed: {error}"))
        {
            let started = fault
                .store()
                .begin_authoritative_outbox_attempt(
                    record.item_id(),
                    record.intent().payload_hash(),
                    record.attempts(),
                )
                .await
                .unwrap_or_else(|error| panic!("{name} {mode:?} outbox start failed: {error}"));
            fault
                .store()
                .complete_authoritative_outbox_attempt(
                    started.item_id(),
                    started.intent().payload_hash(),
                    started.attempts(),
                    AuthoritativeOutboxAttemptResult::Delivered,
                )
                .await
                .unwrap_or_else(|error| {
                    panic!("{name} {mode:?} outbox completion failed: {error}")
                });
        }

        let first_checkpoint = fault
            .checkpoint(token, hash)
            .await
            .unwrap_or_else(|error| panic!("{name} {mode:?} checkpoint failed: {error}"));
        let checkpoint = if let Some(checkpoint) = first_checkpoint {
            checkpoint
        } else {
            fault
                .checkpoint(token, hash)
                .await
                .unwrap_or_else(|error| panic!("{name} {mode:?} checkpoint retry failed: {error}"))
                .expect("healed checkpoint returns its compact receipt")
        };
        assert_eq!(checkpoint.receipt, receipt, "{name} {mode:?}");
        assert!(
            fault
                .store()
                .reserve_authoritative_intent(bundle.clone())
                .await
                .unwrap_or_else(|error| panic!("{name} {mode:?} compact retry failed: {error}"))
                .resumed,
            "{name} {mode:?} must join the compact result"
        );
    }
}

#[tokio::test]
async fn gate1_tactical_and_delayed_lifecycle_bundles_survive_every_in_process_cut() {
    let gate1 = gate1_bundle();
    assert!(
        gate1
            .eligible_witnesses()
            .contains(&PrincipalId::Object { id: mush_core::GOD })
    );
    assert_fault_matrix("Gate-1 brace", gate1).await;
    assert_fault_matrix("Gate-2 tactical fire", tactical_bundle()).await;
    for (name, bundle) in delayed_capture_bundles() {
        assert_fault_matrix(name, bundle).await;
    }
}
