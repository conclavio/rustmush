//! Live-Redis proof that the K3 recovery barrier materializes a pending exact
//! intent before ordinary world hydration. Gated on `REDIS_URL`; the selected
//! database must be disposable because this test wipes RustMUSH namespaces.

use fred::interfaces::{KeysInterface, SetsInterface};
use mush_core::{
    AuthoritativeBundle, AuthoritativeBundleDraft, CanonicalHash, CanonicalId, CommitToken,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, GOD,
    IdempotencyKey, IdempotencyScope, Obj, ObjId, ObjKind, OwnerId, PreparedCausalRef, PrincipalId,
    ReplayWrite, RequestUid, RulesValue, ScopedIdempotencyKey,
};
use mush_server::persist::{load_or_seed_world, reconcile_authoritative_for_writable_boot};
use mush_store::{AuthoritativeStore, ObjectStore, RedisStore};

fn recovery_bundle() -> AuthoritativeBundle {
    let owner = OwnerId::new("redis_boot_test").unwrap();
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("recover_marker").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").unwrap(),
                DefinitionId::new("marker_recovered").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let request = RequestUid::from_bytes([81; 16]);
    let principal = PrincipalId::Object { id: GOD };
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([82; 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("recover_marker").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([83; 32]),
        request_hash: CanonicalHash::from_bytes([84; 32]),
        plan_hash: CanonicalHash::from_bytes([85; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: PreparedCausalRef::new(request, None, 0).unwrap(),
        definitions: vec![action.clone(), outcome.clone()],
        writes: vec![
            ReplayWrite::ObjectSet {
                object: Obj::new(ObjId(2), ObjKind::Thing, "redis recovered marker", GOD, 0),
            },
            ReplayWrite::ObjectAllocatorSet { next_id: 3 },
        ],
        outcome: RulesValue::new(outcome.clone(), serde_json::json!({ "created": 2 })).unwrap(),
        resolution: None,
        semantic_events: vec![
            mush_core::AuthoritativeSemanticEvent::root(
                PreparedCausalRef::new(request, None, 0).unwrap(),
                &action,
                RulesValue::new(outcome, serde_json::json!({ "created": 2 })).unwrap(),
                vec![principal.clone()],
            )
            .unwrap(),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![],
    })
    .unwrap()
}

#[tokio::test]
async fn pending_intent_is_reconciled_before_redis_world_load() {
    let Some(url) = std::env::var("REDIS_URL")
        .ok()
        .filter(|url| !url.is_empty())
    else {
        eprintln!("skipping pending_intent_is_reconciled_before_redis_world_load: REDIS_URL unset");
        return;
    };
    let store = RedisStore::connect_read_only(&url)
        .await
        .expect("connect to disposable Redis");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    load_or_seed_world(&store).await.expect("seed prior world");

    let bundle = recovery_bundle();
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve pending exact intent");

    // The counters are derived capacity indexes, not authority. Simulate a
    // torn/externally clobbered pair; boot must reconstruct both atomically
    // from the exhaustive validated intent scan before admitting writes.
    let _: i64 = store
        .client()
        .del("auth:retained_count")
        .await
        .expect("remove retained count");
    let _: i64 = store
        .client()
        .sadd("auth:retained_count", "wrong-type")
        .await
        .expect("clobber retained count type");
    let _: () = store
        .client()
        .set("auth:retained_bytes", "999999", None, None, false)
        .await
        .expect("clobber retained bytes");
    let before = store.load_world().await.unwrap().unwrap();
    assert!(before.objects.iter().all(|object| object.id != ObjId(2)));

    let report = reconcile_authoritative_for_writable_boot(&store)
        .await
        .expect("boot recovery barrier");
    assert_eq!(report.indexed, 1);
    assert_eq!(report.retained, 1);
    assert_eq!(
        report.retained_bytes,
        serde_json::to_string(&bundle).unwrap().len()
    );
    assert!(report.retention_counters_repaired);
    assert_eq!(report.reconciled, 1);
    assert_eq!(report.checkpointed, 1);
    let repaired_count: usize = store
        .client()
        .get("auth:retained_count")
        .await
        .expect("read repaired count");
    let repaired_bytes: usize = store
        .client()
        .get("auth:retained_bytes")
        .await
        .expect("read repaired bytes");
    assert_eq!(repaired_count, 0, "checkpoint releases the full intent");
    assert_eq!(repaired_bytes, 0, "checkpoint releases its byte charge");
    let world = load_or_seed_world(&store)
        .await
        .expect("hydrate after barrier");
    assert_eq!(world.next_id(), 3);
    assert_eq!(world.get(ObjId(2)).unwrap().name, "redis recovered marker");

    store
        .wipe_namespaces()
        .await
        .expect("clean test namespaces");
    store.close().await.expect("close test store");
}
