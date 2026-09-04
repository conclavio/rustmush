use mush_core::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, CanonicalId,
    DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef, Engine, GOD, IdempotencyKey,
    IdempotencyScope, IngressEnvelope, IngressPayload, IngressSurfaceVersion, OwnerId,
    PerceivedRef, PrincipalId, RequestUid, RevisionSet, ScopedIdempotencyKey,
    StagedActionPersistence,
};
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight, SemanticActionStatus};
use mush_server::modules::boot_modules;
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_tactics::{BRACE_ACTION_ID, TacticsModule};
use mush_testkit::boot_world;
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
    let (persistence, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    let bundle = match persistence {
        StagedActionPersistence::Authoritative(bundle) => *bundle,
        StagedActionPersistence::Legacy(_) => panic!("brace used the legacy persistence lane"),
    };
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
    assert_eq!(bundle.outcome().data()["revision"], 1);

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
    engine.install_prepared_action(installation);
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();

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
    let (persistence, _installation) = restarted
        .take_staged_action()
        .unwrap()
        .expect("second staged action")
        .into_parts();
    let bundle = match persistence {
        StagedActionPersistence::Authoritative(bundle) => bundle,
        StagedActionPersistence::Legacy(_) => panic!("brace used the legacy persistence lane"),
    };
    assert_eq!(bundle.outcome().data()["revision"], 2);
}
