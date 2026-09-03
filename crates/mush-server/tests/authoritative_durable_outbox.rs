use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, CanonicalHash, CanonicalId,
    CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    DurableOutboxIntent, GOD, IdempotencyKey, IdempotencyScope, OwnerId, PreparedCausalRef,
    PrincipalId, ReplayWrite, RequestUid, RulesValue, ScopedIdempotencyKey, World,
};
use mush_server::durable_outbox::{
    DurableDeliveryError, DurableOutboxAdapter, DurableOutboxPolicy, DurableOutboxRegistry,
    dispatch_authoritative_outbox,
};
use mush_store::{
    AuthoritativeOutboxRecord, AuthoritativeStore, InMemoryStore, ObjectStore, WriteBatch,
};

struct RetryOnceAdapter {
    owner: OwnerId,
    adapter: CanonicalId,
    calls: AtomicUsize,
}

impl DurableOutboxAdapter for RetryOnceAdapter {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn adapter(&self) -> &CanonicalId {
        &self.adapter
    }

    fn deliver<'a>(
        &'a self,
        _item: &'a AuthoritativeOutboxRecord,
    ) -> BoxFuture<'a, Result<(), DurableDeliveryError>> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(DurableDeliveryError::Retryable(
                    "temporary adapter outage".to_owned(),
                ))
            } else {
                Ok(())
            }
        })
    }
}

fn bundle() -> AuthoritativeBundle {
    let owner = OwnerId::new("delivery_test").unwrap();
    let adapter = CanonicalId::new("webhook").unwrap();
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("notify").unwrap(),
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
                DefinitionId::new("notified").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let request = RequestUid::from_bytes([0x91; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let outbox = DurableOutboxIntent::new(
        0,
        owner.clone(),
        adapter,
        serde_json::json!({"message": "hello"}),
    )
    .unwrap();
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([0x92; 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("notify").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([0x93; 32]),
        request_hash: CanonicalHash::from_bytes([0x94; 32]),
        plan_hash: CanonicalHash::from_bytes([0x95; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: PreparedCausalRef::new(request, None, 0).unwrap(),
        definitions: vec![action, outcome_definition.clone()],
        writes: vec![ReplayWrite::ModuleJsonSet {
            namespace: owner,
            key: AuthoritativeKey::parse_canonical("action-state/v1/notified/1").unwrap(),
            value: serde_json::json!({"sent": true}),
        }],
        outcome: RulesValue::new(outcome_definition, serde_json::json!({"ok": true})).unwrap(),
        eligible_witnesses: vec![principal],
        durable_outbox: vec![outbox],
    })
    .unwrap()
}

#[tokio::test]
async fn finalized_outbox_is_rediscovered_retried_and_then_checkpoints_parent() {
    let store = InMemoryStore::new();
    let world = World::new_minimal();
    store
        .write(WriteBatch {
            serial: 1,
            upserts: world.iter().cloned().collect(),
            next_id: Some(world.next_id()),
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    let bundle = bundle();
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    store.reserve_authoritative_intent(bundle).await.unwrap();
    store.reconcile_authoritative(token, hash).await.unwrap();

    let empty = DurableOutboxRegistry::new();
    let missing = dispatch_authoritative_outbox(&store, &empty, DurableOutboxPolicy::default())
        .await
        .unwrap();
    assert_eq!(missing.missing_adapters, 1);
    assert_eq!(
        store.pending_authoritative_outbox().await.unwrap()[0].attempts(),
        0
    );

    let adapter = Arc::new(RetryOnceAdapter {
        owner: OwnerId::new("delivery_test").unwrap(),
        adapter: CanonicalId::new("webhook").unwrap(),
        calls: AtomicUsize::new(0),
    });
    let mut registry = DurableOutboxRegistry::new();
    registry.register(adapter.clone()).unwrap();
    let policy = DurableOutboxPolicy::new(3, Duration::from_secs(1)).unwrap();
    let first = dispatch_authoritative_outbox(&store, &registry, policy)
        .await
        .unwrap();
    assert_eq!(first.retry_pending, 1);
    assert_eq!(first.checkpointed_parents, 0);
    assert_eq!(
        store.pending_authoritative_outbox().await.unwrap()[0].attempts(),
        1
    );

    let second = dispatch_authoritative_outbox(&store, &registry, policy)
        .await
        .unwrap();
    assert_eq!(second.delivered, 1);
    assert_eq!(second.checkpointed_parents, 1);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    assert!(
        store
            .pending_authoritative_outbox()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .scan_authoritative_intents(None, 1)
            .await
            .unwrap()
            .intents
            .is_empty(),
        "terminal delivery releases the parent's full replay intent"
    );
}
