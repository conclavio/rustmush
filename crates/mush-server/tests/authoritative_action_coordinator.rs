use mush_contract_tests::{
    TOY_OWNER, install_toy_focus, install_toy_reservation_book, install_toy_scheduled_expiry,
    toy_action_window, toy_brace_invocation, toy_brace_offers, toy_definitions, toy_scene_turn,
    toy_window_invocation, toy_world_with_state,
};
use mush_core::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, ActionWindowPhase, AuthoritativeBundle,
    CanonicalHash, CanonicalId, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef,
    DurableSchedule, DurableScheduleDraft, Engine, GOD, IdempotencyKey, IdempotencyScope,
    IngressEnvelope, IngressPayload, IngressSurfaceVersion, OfferExecutionContext, OfferKeyRing,
    OfferSecret, OfferSubmission, OwnerId, PerceivedRef, PrincipalId, ProjectionScopeId,
    RequestUid, ResolutionKeyRing, ResolutionSecret, RevisionSet, ScheduleCatchUpOrder,
    ScheduleDriveCause, ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy,
    ScopedIdempotencyKey,
};
use mush_proto::ProjectedActionStatus;
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight, SemanticActionStatus};
use mush_server::modules::boot_modules;
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_tactics::{BRACE_ACTION_ID, TacticsModule};
use mush_testkit::{TestWorld, boot_world};
use serde_json::{Value, json};

fn fresh_engine() -> Engine {
    let (mut world, registry) = boot_world();
    let _ = world.take_dirty();
    Engine::with_modules(world, registry, vec![Box::new(TacticsModule::new())])
        .expect("tactics composition")
}

fn brace_action(engine: &Engine) -> DefinitionRef {
    let key = DefinitionKey::new(
        OwnerId::new("tactics").unwrap(),
        DefinitionKind::new("action").unwrap(),
        DefinitionId::new(BRACE_ACTION_ID).unwrap(),
    );
    engine
        .registry
        .definitions()
        .unwrap()
        .current(&key)
        .expect("current brace action")
        .clone()
}

fn request_identity(byte: u8) -> (RequestUid, PrincipalId, ScopedIdempotencyKey) {
    let request = RequestUid::from_bytes([byte; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new("tactics").unwrap(),
            namespace: CanonicalId::new("brace").unwrap(),
        },
        0,
        IdempotencyKey::Request(request),
    );
    (request, principal, idempotency)
}

fn ingress(byte: u8, payload: Value) -> IngressEnvelope {
    let (request, principal, idempotency) = request_identity(byte);
    IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(CanonicalId::new("test.structured").unwrap(), 1).unwrap(),
        IngressPayload::structured(payload).unwrap(),
        idempotency,
    )
    .unwrap()
}

fn invocation(byte: u8) -> ActionInvocation {
    let (request, principal, idempotency) = request_identity(byte);
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal,
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([0x51; 32])),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({}),
        idempotency,
    })
    .unwrap()
}

fn guards(engine: &Engine, action: &DefinitionRef) -> mush_core::GuardSet {
    let definitions = engine.registry.definitions().unwrap();
    let mut builder = engine
        .registry
        .server_guard_set_builder(RevisionSet::default())
        .unwrap();
    builder.add_definition(action.clone(), definitions).unwrap();
    builder.finish().unwrap()
}

#[test]
fn scheduled_ingress_is_stable_and_owner_scoped() {
    let engine = fresh_engine();
    let action = brace_action(&engine);
    let owner = OwnerId::new("tactics").unwrap();
    let schedule = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(&owner, &CanonicalId::new("brace_once").unwrap()).unwrap(),
        owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 5,
        },
        cadence: Some(5),
        generation: 8,
        next_ordinal: 12,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    let ScheduleEvaluation::Advance(advance) = schedule
        .plan_due(
            1,
            &mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 5,
            },
            ScheduleDriveCause::Live,
        )
        .unwrap()
    else {
        panic!("one-shot schedule was not due")
    };
    let first = ActionCoordinator::scheduled_ingress(&action, &advance, 42).unwrap();
    let retry = ActionCoordinator::scheduled_ingress(&action, &advance, 42).unwrap();
    assert_eq!(first.request(), retry.request());
    assert_eq!(first.idempotency(), retry.idempotency());
    assert_eq!(first.ingress_hash(), retry.ingress_hash());
    assert_eq!(first.idempotency().epoch(), 42);
    assert!(matches!(
        first.idempotency().key(),
        IdempotencyKey::Deterministic(_)
    ));
    let ScheduleEvaluation::Advance(later_advance) = schedule
        .plan_due(
            1,
            &mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 15,
            },
            ScheduleDriveCause::Live,
        )
        .unwrap()
    else {
        panic!("later schedule observation was not due")
    };
    assert_ne!(advance.post_image(), later_advance.post_image());
    let later = ActionCoordinator::scheduled_ingress(&action, &later_advance, 42).unwrap();
    assert_eq!(first.request(), later.request());
    assert_eq!(first.idempotency(), later.idempotency());
    assert_eq!(first.ingress_hash(), later.ingress_hash());

    let foreign_owner = OwnerId::new("other").unwrap();
    let foreign = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(&foreign_owner, &CanonicalId::new("brace_once").unwrap()).unwrap(),
        owner: foreign_owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 5,
        },
        cadence: None,
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    let ScheduleEvaluation::Advance(foreign) = foreign
        .plan_due(
            1,
            &mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 5,
            },
            ScheduleDriveCause::Live,
        )
        .unwrap()
    else {
        panic!("foreign schedule was not due")
    };
    assert!(
        ActionCoordinator::scheduled_ingress(&action, &foreign, 42)
            .unwrap_err()
            .to_string()
            .contains("different owners")
    );
}

async fn commit_toy_window_operation(
    store: &InMemoryStore,
    world: &mut TestWorld,
    invocation: &ActionInvocation,
) -> (AuthoritativeBundle, IngressEnvelope) {
    let envelope = toy_window_envelope(invocation);
    let permit = match ActionCoordinator::preflight(store, envelope.clone())
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("window operation was not vacant: {other:?}"),
    };
    let action = toy_definitions().window_action;
    let action_guards = guards(world.engine(), &action);
    ActionCoordinator::prepare_and_stage(
        permit,
        world.engine_mut(),
        &action,
        invocation,
        action_guards,
    )
    .unwrap();
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("window operation staged")
        .into_parts();
    let bundle = *bundle;
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .unwrap();
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert!(receipt.matches_bundle(&bundle));
    world
        .engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    let checkpoint = store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert!(checkpoint.receipt.matches_bundle(&bundle));
    (bundle, envelope)
}

fn toy_window_envelope(invocation: &ActionInvocation) -> IngressEnvelope {
    IngressEnvelope::new(
        invocation.request(),
        invocation.principal().clone(),
        IngressSurfaceVersion::new(CanonicalId::new("test.window").unwrap(), 1).unwrap(),
        IngressPayload::structured(json!({
            "action": "window_operation",
            "parameters": invocation.parameters(),
        }))
        .unwrap(),
        invocation.idempotency().clone(),
    )
    .unwrap()
}

#[tokio::test]
async fn action_window_close_restart_and_resolution_share_exact_authority() {
    let store = InMemoryStore::new();
    let (mut world, families) = toy_world_with_state();
    let _ = world.drain_write_batch();
    let window_id = toy_action_window().id();

    let open = toy_window_invocation(
        GOD,
        RequestUid::from_bytes([0xD0; 16]),
        json!({ "operation": "open" }),
    );
    let (open_bundle, _) = commit_toy_window_operation(&store, &mut world, &open).await;
    assert!(open_bundle.resolution().is_none());
    assert_eq!(
        world
            .engine()
            .action_state()
            .get(&families.windows, &window_id)
            .unwrap()
            .unwrap()
            .phase(),
        ActionWindowPhase::Open
    );

    let first = toy_window_invocation(
        mush_core::ObjId(2),
        RequestUid::from_bytes([0xD1; 16]),
        json!({
            "operation": "submit",
            "expected_revision": 1,
            "now": toy_scene_turn(11),
            "order": 10,
        }),
    );
    commit_toy_window_operation(&store, &mut world, &first).await;
    let second = toy_window_invocation(
        mush_core::ObjId(3),
        RequestUid::from_bytes([0xD2; 16]),
        json!({
            "operation": "submit",
            "expected_revision": 2,
            "now": toy_scene_turn(12),
            "order": 1,
        }),
    );
    commit_toy_window_operation(&store, &mut world, &second).await;

    let close = toy_window_invocation(
        GOD,
        RequestUid::from_bytes([0xD3; 16]),
        json!({
            "operation": "close",
            "expected_revision": 3,
            "now": toy_scene_turn(20),
        }),
    );
    let (close_bundle, _) = commit_toy_window_operation(&store, &mut world, &close).await;
    assert!(close_bundle.resolution().is_none());
    assert_eq!(close_bundle.outcome().data()["intent_count"], 2);

    // The frozen set must be independently durable before any resolver runs.
    let (mut restarted, restarted_families) = toy_world_with_state();
    boot_modules(restarted.engine_mut(), &store).await.unwrap();
    let _ = restarted.engine_mut().world.take_dirty();
    assert!(restarted.engine_mut().take_module_ops().is_empty());
    let closed = restarted
        .engine()
        .action_state()
        .get(&restarted_families.windows, &window_id)
        .unwrap()
        .unwrap();
    assert_eq!(closed.phase(), ActionWindowPhase::Closed);
    assert_eq!(closed.revision(), 4);
    assert_eq!(
        restarted.engine().resolution_key_rotation_references(),
        std::collections::BTreeSet::from([1])
    );
    assert_eq!(
        closed
            .resolution_order()
            .unwrap()
            .into_iter()
            .map(|intent| intent.invocation().actor().0)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );

    let resolve = toy_window_invocation(
        GOD,
        RequestUid::from_bytes([0xD4; 16]),
        json!({
            "operation": "resolve",
            "expected_revision": 4,
            "now": toy_scene_turn(21),
        }),
    );

    // A deployment that pruned the pinned key fails before staging and
    // leaves the closed post-image untouched.
    restarted.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(2, [0x72; 32]).unwrap(), []).unwrap(),
    );
    let missing_envelope = toy_window_envelope(&resolve);
    let missing_permit = match ActionCoordinator::preflight(&store, missing_envelope)
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("unattempted resolution was not vacant: {other:?}"),
    };
    let window_action = toy_definitions().window_action;
    let window_guards = guards(restarted.engine(), &window_action);
    let error = ActionCoordinator::prepare_and_stage(
        missing_permit,
        restarted.engine_mut(),
        &window_action,
        &resolve,
        window_guards,
    )
    .expect_err("missing pinned secret must fail closed");
    assert!(error.to_string().contains("resolution RNG is unavailable"));
    assert!(
        restarted
            .engine_mut()
            .take_staged_action()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        restarted
            .engine()
            .action_state()
            .get(&restarted_families.windows, &window_id)
            .unwrap()
            .unwrap()
            .phase(),
        ActionWindowPhase::Closed
    );

    // Rotate after close while retaining the live reference. Resolution must
    // use the version pinned when the window opened, not the new current key.
    restarted.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(2, [0x72; 32]).unwrap(),
            [ResolutionSecret::new(1, [0x71; 32]).unwrap()],
        )
        .unwrap(),
    );
    let (resolve_bundle, resolve_envelope) =
        commit_toy_window_operation(&store, &mut restarted, &resolve).await;
    assert_eq!(resolve_bundle.writes().len(), 1);
    assert_eq!(resolve_bundle.outcome().data()["order"], json!([3, 2]));
    let resolution = resolve_bundle
        .resolution()
        .expect("window resolution retains its RNG transcript");
    assert_eq!(resolution.action(), resolve_bundle.token());
    assert_eq!(resolution.secret_version(), 1);
    assert_eq!(resolution.outcome(), resolve_bundle.outcome());
    assert_eq!(resolution.draws().len(), 1);
    let consumed = restarted
        .engine()
        .action_state()
        .get(&restarted_families.windows, &window_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        consumed.phase(),
        ActionWindowPhase::Consumed {
            commit: resolve_bundle.token(),
        }
    );
    assert_eq!(consumed.revision(), 5);
    assert!(
        restarted
            .engine()
            .resolution_key_rotation_references()
            .is_empty()
    );

    match ActionCoordinator::preflight(&store, resolve_envelope)
        .await
        .unwrap()
    {
        ActionPreflight::Finalized(receipt) => {
            assert_eq!(receipt.resolution(), resolve_bundle.resolution());
            assert_eq!(receipt.outcome(), resolve_bundle.outcome());
        }
        other => panic!("resolved-window retry was not compact and final: {other:?}"),
    }

    let (mut after_resolution, after_families) = toy_world_with_state();
    boot_modules(after_resolution.engine_mut(), &store)
        .await
        .unwrap();
    let restored = after_resolution
        .engine()
        .action_state()
        .get(&after_families.windows, &window_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored, consumed);
    assert_eq!(
        restored.phase(),
        ActionWindowPhase::Consumed {
            commit: resolve_bundle.token(),
        }
    );
}

#[tokio::test]
async fn real_provider_is_exactly_once_and_hydrates_before_the_next_action() {
    let store = InMemoryStore::new();
    let mut engine = fresh_engine();
    boot_modules(&mut engine, &store).await.unwrap();
    let _ = engine.world.take_dirty();
    assert!(engine.take_module_ops().is_empty());

    let envelope = ingress(1, json!({"action": "brace"}));
    let action = brace_action(&engine);
    let legacy_error = engine
        .prepare_registered_action(&action, &invocation(1))
        .expect_err("a migrated provider must refuse the legacy K2 lane");
    assert!(
        legacy_error
            .to_string()
            .contains("requires the authoritative action coordinator")
    );
    let permit = match ActionCoordinator::preflight(&store, envelope.clone())
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("first request was not vacant: {other:?}"),
    };
    let action_guards = guards(&engine, &action);
    let staged = ActionCoordinator::prepare_and_stage(
        permit,
        &mut engine,
        &action,
        &invocation(1),
        action_guards,
    )
    .unwrap();
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    let bundle = *bundle;
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
    assert_eq!(bundle.outcome().data()["revision"], 1);
    assert_eq!(bundle.semantic_events().len(), 1);
    assert_eq!(bundle.semantic_events()[0].kind().as_str(), BRACE_ACTION_ID);

    let intent = store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .unwrap();
    assert!(!intent.resumed);
    match ActionCoordinator::preflight(&store, envelope.clone())
        .await
        .unwrap()
    {
        ActionPreflight::Pending { token, bundle_hash } => {
            assert_eq!(token, bundle.token());
            assert_eq!(bundle_hash, bundle.bundle_hash());
        }
        other => panic!("reserved retry did not join its pending intent: {other:?}"),
    }
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert!(receipt.matches_bundle(&bundle));
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();

    let public_status = ActionCoordinator::status_id(envelope.idempotency()).unwrap();
    let projections = engine
        .registry
        .action_providers()
        .unwrap()
        .receipt_projection_table();
    let ProjectedActionStatus::Committed(before_restart) = ActionCoordinator::projected_status(
        &store,
        &projections,
        &PrincipalId::Object { id: GOD },
        &public_status,
    )
    .await
    .unwrap() else {
        panic!("checkpointed action did not return a public committed receipt");
    };
    assert_eq!(
        before_restart.result().unwrap().summary(),
        "Brace committed."
    );

    match ActionCoordinator::semantic_status(
        &store,
        &PrincipalId::Object { id: GOD },
        envelope.idempotency(),
        envelope.ingress_hash(),
    )
    .await
    .unwrap()
    {
        SemanticActionStatus::Finalized(status) => {
            assert_eq!(status.token(), bundle.token());
            assert_eq!(status.outcome().data()["revision"], 1);
        }
        other => panic!("eligible viewer did not receive semantic status: {other:?}"),
    }
    assert!(matches!(
        ActionCoordinator::semantic_status(
            &store,
            &PrincipalId::Object {
                id: mush_core::LIMBO
            },
            envelope.idempotency(),
            envelope.ingress_hash(),
        )
        .await
        .unwrap(),
        SemanticActionStatus::Forbidden
    ));

    match ActionCoordinator::preflight(&store, envelope.clone())
        .await
        .unwrap()
    {
        ActionPreflight::Finalized(retry) => {
            assert_eq!(retry.token(), bundle.token());
            assert_eq!(retry.outcome().data()["revision"], 1);
        }
        other => panic!("exact retry did not return its receipt: {other:?}"),
    }
    match ActionCoordinator::preflight(
        &store,
        ingress(1, json!({"action": "brace", "changed": true})),
    )
    .await
    .unwrap()
    {
        ActionPreflight::Conflict => {}
        other => panic!("conflicting ingress was not rejected: {other:?}"),
    }

    let mut restarted = fresh_engine();
    boot_modules(&mut restarted, &store).await.unwrap();
    let _ = restarted.world.take_dirty();
    assert!(restarted.take_module_ops().is_empty());
    let restarted_projections = restarted
        .registry
        .action_providers()
        .unwrap()
        .receipt_projection_table();
    assert_eq!(
        ActionCoordinator::projected_status(
            &store,
            &restarted_projections,
            &PrincipalId::Object { id: GOD },
            &public_status,
        )
        .await
        .unwrap(),
        ProjectedActionStatus::Committed(before_restart),
        "checkpointed receipt projection must be stable across composition restart"
    );
    let action = brace_action(&restarted);
    let permit = match ActionCoordinator::preflight(&store, ingress(2, json!({"action": "brace"})))
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("new request was not vacant after restart: {other:?}"),
    };
    let action_guards = guards(&restarted, &action);
    ActionCoordinator::prepare_and_stage(
        permit,
        &mut restarted,
        &action,
        &invocation(2),
        action_guards,
    )
    .unwrap();
    let (bundle, _installation) = restarted
        .take_staged_action()
        .unwrap()
        .expect("second staged action")
        .into_parts();
    assert_eq!(bundle.outcome().data()["revision"], 2);
}

#[tokio::test]
async fn gate_one_offer_is_preflighted_regenerated_and_routed_without_hidden_client_fields() {
    let store = InMemoryStore::new();
    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, GOD, 3);
    let _ = world.drain_write_batch();
    let keys = OfferKeyRing::new(OfferSecret::new(1, [0x42; 32]).unwrap(), []).unwrap();
    world.engine_mut().set_offer_keys(keys.clone());
    let initially_projected = toy_brace_offers(world.engine(), GOD, 11, 7)
        .unwrap()
        .projected()
        .pop()
        .unwrap();
    let request = RequestUid::from_bytes([0xA7; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new(TOY_OWNER).unwrap(),
            namespace: CanonicalId::new("brace").unwrap(),
        },
        0,
        IdempotencyKey::Request(request),
    );
    let envelope = IngressEnvelope::new(
        request,
        principal.clone(),
        IngressSurfaceVersion::new(CanonicalId::new("test.offer").unwrap(), 1).unwrap(),
        IngressPayload::structured(json!({
            "offer": initially_projected.offer(),
            "subject": initially_projected.subject(),
            "targets": [],
            "parameters": {},
        }))
        .unwrap(),
        idempotency.clone(),
    )
    .unwrap();
    let permit = match ActionCoordinator::preflight(&store, envelope)
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("offered action was not vacant: {other:?}"),
    };

    // This is the execution-time regeneration. It happens only after the
    // durable preflight and reconstructs hidden clock/guard/action data.
    let regenerated = toy_brace_offers(world.engine(), GOD, 11, 7).unwrap();
    let resolved = world
        .engine()
        .resolve_legal_action(
            &regenerated,
            OfferExecutionContext {
                request,
                principal,
                actor: GOD,
                idempotency,
                viewer_scope: ProjectionScopeId::derive(&PrincipalId::Object { id: GOD }, 7)
                    .unwrap(),
                now: None,
            },
            OfferSubmission {
                offer: initially_projected.offer(),
                subject: initially_projected.subject().clone(),
                targets: Vec::new(),
                parameters: json!({}),
            },
        )
        .unwrap();
    ActionCoordinator::prepare_and_stage_resolved(permit, world.engine_mut(), resolved).unwrap();
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    let bundle = *bundle;
    assert_eq!(bundle.writes().len(), 2);
    assert_eq!(bundle.outcome().data()["focus"], 2);
    let resolution = bundle.resolution().expect("toy brace records its RNG");
    assert_eq!(resolution.action(), bundle.token());
    assert_eq!(resolution.outcome(), bundle.outcome());
    assert_eq!(resolution.draws().len(), 1);
    world
        .engine_mut()
        .abort_prepared_action(installation)
        .expect("the fixture aborts");
}

#[tokio::test]
async fn toy_trigger_cascade_survives_bundle_checkpoint_and_restart() {
    let store = InMemoryStore::new();
    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, GOD, 3);
    let reservation_book = install_toy_reservation_book(world.engine_mut(), &families, GOD);
    let _ = world.drain_write_batch();
    let request = RequestUid::from_bytes([0xB1; 16]);
    let invocation = toy_brace_invocation(GOD, 11, request);
    let envelope = IngressEnvelope::new(
        request,
        invocation.principal().clone(),
        IngressSurfaceVersion::new(CanonicalId::new("test.structured").unwrap(), 1).unwrap(),
        IngressPayload::structured(json!({"action": "brace", "scene_turn": 11})).unwrap(),
        invocation.idempotency().clone(),
    )
    .unwrap();
    let permit = match ActionCoordinator::preflight(&store, envelope)
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("toy request was not vacant: {other:?}"),
    };
    let action = toy_definitions().brace;
    let action_guards = guards(world.engine(), &action);
    ActionCoordinator::prepare_and_stage(
        permit,
        world.engine_mut(),
        &action,
        &invocation,
        action_guards,
    )
    .unwrap();
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("toy cascade staged")
        .into_parts();
    let bundle = *bundle;
    assert_eq!(bundle.writes().len(), 3);
    assert_eq!(bundle.semantic_events().len(), 2);
    assert_eq!(bundle.semantic_events()[0].kind().as_str(), "brace");
    assert_eq!(bundle.semantic_events()[1].kind().as_str(), "apply_guarded");
    assert_eq!(
        bundle.semantic_events()[1].cause().parent(),
        Some(bundle.semantic_events()[0].cause().event_id())
    );
    let exact_resolution = bundle
        .resolution()
        .cloned()
        .expect("Gate-1 brace records resolution");

    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .unwrap();
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert_eq!(receipt.semantic_events(), bundle.semantic_events());
    assert_eq!(receipt.resolution(), Some(&exact_resolution));
    world
        .engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    assert_eq!(
        world
            .engine()
            .action_state()
            .get(&families.focus, &GOD)
            .unwrap()
            .unwrap()
            .amount(),
        2
    );
    assert_eq!(
        world
            .engine()
            .action_state()
            .get(&families.guarded, &GOD)
            .unwrap()
            .unwrap()
            .expires_at(),
        &toy_scene_turn(13)
    );
    let checkpoint = store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert_eq!(
        checkpoint.receipt.semantic_events(),
        bundle.semantic_events()
    );
    assert_eq!(checkpoint.receipt.resolution(), Some(&exact_resolution));

    let (mut restarted, restarted_families) = toy_world_with_state();
    boot_modules(restarted.engine_mut(), &store).await.unwrap();
    let _ = restarted.engine_mut().world.take_dirty();
    assert!(restarted.engine_mut().take_module_ops().is_empty());
    assert_eq!(
        restarted
            .engine()
            .action_state()
            .get(&restarted_families.focus, &GOD)
            .unwrap()
            .unwrap()
            .amount(),
        2
    );
    assert_eq!(
        restarted
            .engine()
            .action_state()
            .get(&restarted_families.guarded, &GOD)
            .unwrap()
            .unwrap()
            .expires_at(),
        &toy_scene_turn(13)
    );
    let restarted_book = restarted
        .engine()
        .action_state()
        .get(&restarted_families.reservations, &reservation_book)
        .unwrap()
        .unwrap();
    assert_eq!(restarted_book.revision(), 1);
    assert_eq!(restarted_book.resource_revision(), 1);
    assert_eq!(
        restarted_book
            .checked_available(
                restarted
                    .engine()
                    .action_state()
                    .get(&restarted_families.focus, &GOD)
                    .unwrap()
                    .unwrap()
                    .value,
            )
            .unwrap(),
        2
    );

    let effect = restarted
        .engine()
        .action_state()
        .get(&restarted_families.guarded, &GOD)
        .unwrap()
        .unwrap()
        .effect
        .id();
    let expiry_action = toy_definitions().guarded_expire;
    let schedule_owner = OwnerId::new(TOY_OWNER).unwrap();
    let schedule = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(
            &schedule_owner,
            &CanonicalId::new("guarded_expiry_god").unwrap(),
        )
        .unwrap(),
        owner: schedule_owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 0,
        },
        cadence: None,
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    let schedule_id = schedule.id();
    install_toy_scheduled_expiry(
        restarted.engine_mut(),
        &restarted_families,
        schedule,
        GOD,
        effect,
    );
    let page = restarted
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Restart, None, 1)
        .unwrap();
    let candidate = page.candidates()[0].clone();
    assert_eq!(candidate.advance().occurrences().len(), 1);
    let expiry_envelope =
        ActionCoordinator::scheduled_ingress(&expiry_action, candidate.advance(), 1).unwrap();
    let expiry_permit = match ActionCoordinator::preflight(&store, expiry_envelope.clone())
        .await
        .unwrap()
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("expiry request was not vacant: {other:?}"),
    };
    ActionCoordinator::prepare_and_stage_scheduled(
        expiry_permit,
        restarted.engine_mut(),
        &candidate,
    )
    .unwrap();
    let (expiry_bundle, installation) = restarted
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("expiry staged")
        .into_parts();
    let expiry_bundle = *expiry_bundle;
    assert_eq!(expiry_bundle.writes().len(), 2);
    assert_eq!(
        expiry_bundle.semantic_events()[0].kind().as_str(),
        "guarded_expire"
    );
    store
        .reserve_authoritative_intent(expiry_bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(expiry_bundle.token(), expiry_bundle.bundle_hash())
        .await
        .unwrap();
    restarted
        .engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(expiry_bundle.token(), expiry_bundle.bundle_hash())
        .await
        .unwrap();
    assert!(
        restarted
            .engine()
            .action_state()
            .get(&restarted_families.guarded, &GOD)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        restarted
            .engine()
            .action_state()
            .get(&restarted_families.schedules, &schedule_id)
            .unwrap()
            .unwrap()
            .state(),
        mush_core::ScheduleState::Completed
    );
    assert!(matches!(
        ActionCoordinator::preflight(&store, expiry_envelope)
            .await
            .unwrap(),
        ActionPreflight::Finalized(_)
    ));

    let (mut after_expiry_restart, after_expiry_families) = toy_world_with_state();
    boot_modules(after_expiry_restart.engine_mut(), &store)
        .await
        .unwrap();
    assert!(
        after_expiry_restart
            .engine()
            .action_state()
            .get(&after_expiry_families.guarded, &GOD)
            .unwrap()
            .is_none()
    );
}
