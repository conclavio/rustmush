//! The shared behavior suite (plus Redis-only tests: index bootstrap +
//! search, native TTL bans) against a **live Redis 8**, gated on the
//! `REDIS_URL` env var.
//!
//! Without `REDIS_URL` every test skips cleanly with a note — the dev box
//! has no Redis; Linux CI runs a `redis:8` service container.
//!
//! **WARNING:** these tests delete every `obj:*`, `account:*`, `ban:*`,
//! `token:*`, `meta:*`, `mod:*`, and `auth:*` key in the configured database
//! between phases — module data (channels, mail, jobs) and retained
//! authoritative intents are wiped along with the world. Point `REDIS_URL`
//! at a disposable database.
//!
//! Tests in this file share one database, so they serialize on a static
//! async mutex (cargo runs `#[tokio::test]`s in one process concurrently).

use mush_core::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, CanonicalHash, CanonicalId,
    CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    DurableOutboxIntent, GOD, IdempotencyKey, IdempotencyScope, Obj, ObjId, ObjKind, OwnerId,
    PreparedCausalRef, PrincipalId, ReplayWrite, RequestUid, RulesValue, ScopedIdempotencyKey,
    World,
};
use mush_store::{
    AuthoritativeOutboxAttemptResult, AuthoritativeRetentionUsage, AuthoritativeScanCursor,
    AuthoritativeStore, BundleStatus, IdempotencyStatus, ObjectStore, RedisStore, StoreError,
    WriteBatch, suite,
};
use tokio::sync::Mutex;

static REDIS_LOCK: Mutex<()> = Mutex::const_new(());

/// Connect and wipe, or `None` (with a note) when `REDIS_URL` is unset
/// (or set but empty — `REDIS_URL= cargo test` must skip, not panic).
async fn fresh_store(test: &str) -> Option<RedisStore> {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipping {test}: REDIS_URL not set (no live Redis on this machine)");
        return None;
    };
    let store = RedisStore::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("REDIS_URL set but connect failed: {e}"));
    store.wipe_namespaces().await.expect("wipe test namespaces");
    Some(store)
}

async fn seed_minimal_world(store: &RedisStore) {
    let world = World::new_minimal();
    store
        .write(WriteBatch {
            serial: 1,
            upserts: world.iter().cloned().collect(),
            next_id: Some(world.next_id()),
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .expect("seed minimal world");
}

fn authoritative_bundle(token_byte: u8, request_byte: u8) -> AuthoritativeBundle {
    authoritative_bundle_for_epoch(token_byte, request_byte, 0)
}

fn authoritative_bundle_for_epoch(
    token_byte: u8,
    request_byte: u8,
    epoch: u64,
) -> AuthoritativeBundle {
    authoritative_bundle_for_epoch_and_outbox(token_byte, request_byte, epoch, vec![])
}

fn authoritative_bundle_for_epoch_and_outbox(
    token_byte: u8,
    request_byte: u8,
    epoch: u64,
    durable_outbox: Vec<DurableOutboxIntent>,
) -> AuthoritativeBundle {
    let owner = OwnerId::new("redis_toy").unwrap();
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("brace").unwrap(),
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
                DefinitionId::new("braced").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let request = RequestUid::from_bytes([request_byte; 16]);
    let principal = PrincipalId::Object { id: GOD };
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([token_byte; 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("brace").unwrap(),
            },
            epoch,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([request_byte; 32]),
        request_hash: CanonicalHash::from_bytes([3; 32]),
        plan_hash: CanonicalHash::from_bytes([4; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: PreparedCausalRef::new(request, None, 0).unwrap(),
        definitions: vec![action, outcome_definition.clone()],
        writes: vec![
            ReplayWrite::ObjectSet {
                object: Obj::new(ObjId(2), ObjKind::Thing, "guard", GOD, 0),
            },
            ReplayWrite::ObjectAllocatorSet { next_id: 3 },
            ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: AuthoritativeKey::parse_canonical("action-state/v1/guarded/2").unwrap(),
                value: serde_json::json!({ "amount": 1 }),
            },
        ],
        outcome: RulesValue::new(outcome_definition, serde_json::json!({ "ok": true })).unwrap(),
        eligible_witnesses: vec![principal],
        durable_outbox,
    })
    .unwrap()
}

macro_rules! redis_gated {
    ($name:ident, $store:ident, $body:block) => {
        #[tokio::test]
        async fn $name() {
            let _serialize = REDIS_LOCK.lock().await;
            let Some($store) = fresh_store(stringify!($name)).await else {
                return;
            };
            $body
        }
    };
}

redis_gated!(empty_seed_load_round_trip, store, {
    suite::empty_seed_load_round_trip(&store).await;
});
redis_gated!(batch_atomicity, store, {
    suite::batch_atomicity(&store).await;
});
redis_gated!(next_id_persistence, store, {
    suite::next_id_persistence(&store).await;
});
redis_gated!(account_reserve_race, store, {
    suite::account_reserve_race(&store).await;
});
redis_gated!(account_lifecycle, store, {
    suite::account_lifecycle(&store).await;
});
redis_gated!(account_password_update, store, {
    suite::account_password_update(&store).await;
});
redis_gated!(account_ssh_keys_update, store, {
    suite::account_ssh_keys_update(&store).await;
});
redis_gated!(ban_expiry, store, {
    suite::ban_expiry(&store).await;
});
redis_gated!(token_lifecycle, store, {
    suite::token_lifecycle(&store).await;
});
redis_gated!(token_sliding_ttl, store, {
    suite::token_sliding_ttl(&store).await;
});
redis_gated!(token_del_account, store, {
    suite::token_del_account(&store).await;
});
redis_gated!(migration_version_bump, store, {
    suite::migration_version_bump(&store).await;
});
redis_gated!(serde_obj_round_trip, store, {
    suite::serde_obj_round_trip(&store).await;
});

// Module persistence twins (M3): same shared suite functions as
// tests/memory_suite.rs, against live Redis. The wipe between phases
// covers `mod:*` too.
redis_gated!(module_batch_atomicity, store, {
    suite::module_batch_atomicity(&store).await;
});
redis_gated!(authoritative_module_prefix_is_store_owned, store, {
    suite::authoritative_module_prefix_is_store_owned(&store).await;
});
redis_gated!(module_kv_round_trip, store, {
    suite::module_kv_round_trip(&store).await;
});
redis_gated!(module_stream_ordering, store, {
    suite::module_stream_ordering(&store).await;
});
redis_gated!(module_empty_namespace_list, store, {
    suite::module_empty_namespace_list(&store).await;
});

redis_gated!(module_list_rejects_type_clobbered_key, store, {
    use fred::interfaces::KeysInterface;
    use mush_store::StoreError;

    // JSON.GET reports the same wrong-type family for a legitimate stream
    // and an arbitrary Redis string. The listing must inspect TYPE and fail
    // closed instead of silently classifying this corrupt document as a
    // stream. module_empty_namespace_list above separately proves that a
    // real stream remains an accepted, skipped sibling.
    let _: () = store
        .client()
        .set(
            "mod:clobbered:authoritative-doc",
            "not a RedisJSON document",
            None,
            None,
            false,
        )
        .await
        .expect("clobber module document with a string");

    let err = store
        .module_json_list("clobbered")
        .await
        .expect_err("a non-JSON, non-stream module key must fail closed");
    assert!(
        matches!(err, StoreError::Corrupt(ref message) if message.contains("unexpected Redis type \"string\"")),
        "expected a corruption error naming the unexpected Redis type, got {err:?}"
    );
});

// ---------------------------------------------------------------------
// Redis-only behavior
// ---------------------------------------------------------------------

redis_gated!(host_power_loss_receipt_policy_validates_live_aof, store, {
    store
        .require_host_power_loss_receipts()
        .await
        .expect("test Redis uses AOF with fsync always");
});

redis_gated!(authoritative_exact_round_trip_and_discovery, store, {
    use fred::interfaces::SetsInterface;

    seed_minimal_world(&store).await;
    let bundle = authoritative_bundle(11, 21);

    assert_eq!(
        store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await
            .expect("empty idempotency lookup"),
        IdempotencyStatus::Vacant
    );

    let reservation = store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve exact intent");
    assert_eq!(reservation.token, bundle.token());
    assert_eq!(reservation.bundle_hash, bundle.bundle_hash());
    assert!(!reservation.resumed);
    assert_eq!(
        store
            .pending_authoritative_intents()
            .await
            .expect("pending index"),
        vec![mush_store::PendingAuthoritativeIntent {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        }]
    );

    // SCAN's count is a work hint rather than a strict result limit. Walk
    // every cursor anyway: boot recovery must never treat one page as the
    // complete retained-intent keyspace.
    let mut cursor: Option<AuthoritativeScanCursor> = None;
    let mut discovered = Vec::new();
    loop {
        let page = store
            .scan_authoritative_intents(cursor, 1)
            .await
            .expect("scan retained intents");
        discovered.extend(page.intents);
        let Some(next) = page.next_cursor else { break };
        cursor = Some(next);
    }
    assert_eq!(discovered, vec![bundle.clone()]);

    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("replay, read back, and finalize");
    assert!(receipt.matches_bundle(&bundle));
    assert_eq!(
        store
            .authoritative_status(bundle.token(), bundle.bundle_hash())
            .await
            .expect("final status"),
        BundleStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert_eq!(
        store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await
            .expect("final idempotency lookup"),
        IdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert!(
        store
            .pending_authoritative_intents()
            .await
            .expect("empty pending index")
            .is_empty()
    );

    let resumed = store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("exact retry joins prior result");
    assert!(resumed.resumed);
    assert_eq!(
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("finalization retry returns exact witness"),
        receipt
    );

    // Either exact retry path repairs a stale accelerator member left beside
    // an already-finalized witness.
    let added: i64 = store
        .client()
        .sadd("auth:pending", bundle.token().to_string())
        .await
        .expect("install stale pending accelerator member");
    assert_eq!(added, 1);
    assert!(
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("exact retry repairs stale accelerator")
            .resumed
    );
    assert!(
        store
            .pending_authoritative_intents()
            .await
            .expect("stale accelerator removed")
            .is_empty()
    );
    let added: i64 = store
        .client()
        .sadd("auth:pending", bundle.token().to_string())
        .await
        .expect("reinstall stale pending accelerator member");
    assert_eq!(added, 1);
    assert_eq!(
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("exact reconciliation repairs stale accelerator"),
        receipt
    );
    assert!(
        store
            .pending_authoritative_intents()
            .await
            .expect("reconciliation removed stale accelerator")
            .is_empty()
    );

    let world = store
        .load_world()
        .await
        .expect("load materialized world")
        .expect("world exists");
    assert_eq!(world.next_id, 3);
    assert!(world.objects.iter().any(|object| object.id == ObjId(2)));
    assert_eq!(
        store
            .module_json_get("redis_toy", "action-state/v1/guarded/2")
            .await
            .expect("load materialized module value"),
        Some(serde_json::json!({ "amount": 1 }))
    );
});

redis_gated!(authoritative_checkpoint_is_atomic_and_retryable, store, {
    use fred::interfaces::KeysInterface;

    seed_minimal_world(&store).await;
    let bundle = authoritative_bundle(12, 22);
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let intent_key = format!("auth:intent:{token}");
    let receipt_key = format!("auth:receipt:{token}");
    let intent_bytes = serde_json::to_string(&bundle).unwrap().len();

    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("reserve checkpoint candidate");
    assert!(matches!(
        store.checkpoint_authoritative(token, hash).await,
        Err(StoreError::AuthoritativeNotFinalized)
    ));
    let receipt = store
        .reconcile_authoritative(token, hash)
        .await
        .expect("finalize checkpoint candidate");
    let checkpoint = store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("checkpoint finalized intent");
    assert_eq!(checkpoint.receipt, receipt);
    assert_eq!(checkpoint.released_intent_bytes, intent_bytes);
    assert!(!checkpoint.resumed);
    assert_eq!(
        store.client().exists::<i64, _>(intent_key).await.unwrap(),
        0
    );
    assert_eq!(
        store.client().exists::<i64, _>(receipt_key).await.unwrap(),
        1
    );
    assert_eq!(
        store
            .client()
            .get::<usize, _>("auth:retained_count")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .client()
            .get::<usize, _>("auth:retained_bytes")
            .await
            .unwrap(),
        0
    );
    assert!(
        store
            .scan_authoritative_intents(None, 1)
            .await
            .unwrap()
            .intents
            .is_empty()
    );
    assert_eq!(
        store.authoritative_status(token, hash).await.unwrap(),
        BundleStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert_eq!(
        store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await
            .unwrap(),
        IdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
    );
    assert!(
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("exact checkpoint retry")
            .resumed
    );
    assert_eq!(
        store.reconcile_authoritative(token, hash).await.unwrap(),
        receipt
    );
    let resumed = store.checkpoint_authoritative(token, hash).await.unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.released_intent_bytes, 0);
});

redis_gated!(authoritative_durable_outbox_round_trip, store, {
    seed_minimal_world(&store).await;
    let intent = DurableOutboxIntent::new(
        0,
        OwnerId::new("redis_toy").unwrap(),
        CanonicalId::new("webhook").unwrap(),
        serde_json::json!({"event": "braced"}),
    )
    .unwrap();
    let payload_hash = intent.payload_hash();
    let bundle = authoritative_bundle_for_epoch_and_outbox(15, 25, 0, vec![intent]);
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let scope = bundle.idempotency().scope().clone();
    store
        .reserve_authoritative_intent(bundle)
        .await
        .expect("reserve outbox parent");
    assert!(
        store
            .pending_authoritative_outbox()
            .await
            .unwrap()
            .is_empty()
    );
    store
        .reconcile_authoritative(token, hash)
        .await
        .expect("finalize and materialize outbox");

    let pending = store.pending_authoritative_outbox().await.unwrap();
    assert_eq!(pending.len(), 1);
    let item_id = pending[0].item_id();
    assert!(matches!(
        store.checkpoint_authoritative(token, hash).await,
        Err(StoreError::AuthoritativeOutboxPending)
    ));
    let attempt = store
        .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
        .await
        .expect("persist attempt before send");
    assert_eq!(attempt.attempts(), 1);
    assert_eq!(
        store
            .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
            .await
            .expect("join lost start acknowledgement"),
        attempt
    );
    store
        .complete_authoritative_outbox_attempt(
            item_id,
            payload_hash,
            1,
            AuthoritativeOutboxAttemptResult::Delivered,
        )
        .await
        .expect("mark delivered");
    assert!(
        store
            .pending_authoritative_outbox()
            .await
            .unwrap()
            .is_empty()
    );
    store
        .complete_authoritative_outbox_attempt(
            item_id,
            payload_hash,
            1,
            AuthoritativeOutboxAttemptResult::Delivered,
        )
        .await
        .expect("join lost delivery acknowledgement");
    store
        .checkpoint_authoritative(token, hash)
        .await
        .expect("terminal outbox permits compaction");
    store
        .retire_authoritative_epochs(&scope, 1)
        .await
        .expect("parent epoch and terminal delivery retire together");
    use fred::interfaces::KeysInterface;
    assert_eq!(
        store
            .client()
            .get::<Option<String>, _>(format!("auth:outbox:item:{item_id}"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .client()
            .get::<Option<String>, _>(format!("auth:outbox:parent:{token}"))
            .await
            .unwrap(),
        None
    );
});

redis_gated!(authoritative_epoch_retirement_is_watermark_first, store, {
    seed_minimal_world(&store).await;
    let old = authoritative_bundle_for_epoch(13, 23, 0);
    let old_token = old.token();
    let old_hash = old.bundle_hash();
    let old_key = old.idempotency().clone();
    let scope = old_key.scope().clone();
    store
        .reserve_authoritative_intent(old.clone())
        .await
        .expect("reserve old epoch");
    store
        .reconcile_authoritative(old_token, old_hash)
        .await
        .expect("finalize old epoch");
    store
        .checkpoint_authoritative(old_token, old_hash)
        .await
        .expect("checkpoint old epoch");

    let retirement = store
        .retire_authoritative_epochs(&scope, 1)
        .await
        .expect("retire old epoch");
    assert_eq!(retirement.retired_tombstones, 1);
    assert!(!retirement.resumed);
    assert_eq!(
        store
            .authoritative_idempotency_status(&old_key, old.ingress_hash())
            .await
            .unwrap(),
        IdempotencyStatus::Retired {
            minimum_accepted_epoch: 1
        }
    );
    assert!(matches!(
        store.reserve_authoritative_intent(old).await,
        Err(StoreError::AuthoritativeEpochRetired {
            requested_epoch: 0,
            minimum_accepted_epoch: 1,
        })
    ));
    assert_eq!(
        store
            .authoritative_status(old_token, old_hash)
            .await
            .unwrap(),
        BundleStatus::Absent
    );

    let current = authoritative_bundle_for_epoch(14, 24, 1);
    let current_token = current.token();
    let current_hash = current.bundle_hash();
    store
        .reserve_authoritative_intent(current)
        .await
        .expect("reserve current epoch");
    assert!(matches!(
        store.retire_authoritative_epochs(&scope, 2).await,
        Err(StoreError::AuthoritativeEpochPending {
            requested_minimum_epoch: 2,
            retained_intents: 1,
        })
    ));
    store
        .reconcile_authoritative(current_token, current_hash)
        .await
        .expect("finalize current epoch");
    assert!(matches!(
        store.retire_authoritative_epochs(&scope, 2).await,
        Err(StoreError::AuthoritativeEpochPending { .. })
    ));
    store
        .checkpoint_authoritative(current_token, current_hash)
        .await
        .expect("checkpoint current epoch");
    assert_eq!(
        store
            .retire_authoritative_epochs(&scope, 2)
            .await
            .expect("retire current epoch")
            .retired_tombstones,
        1
    );
});

redis_gated!(
    authoritative_retention_rebuilds_wrong_typed_counters,
    store,
    {
        use fred::interfaces::{KeysInterface, SetsInterface};

        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(31, 41);
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("reserve exact intent");
        let usage = AuthoritativeRetentionUsage {
            count: 1,
            bytes: serde_json::to_string(&bundle).unwrap().len(),
        };

        let _: i64 = store
            .client()
            .del("auth:retained_count")
            .await
            .expect("remove count string");
        let _: i64 = store
            .client()
            .sadd("auth:retained_count", "wrong-type")
            .await
            .expect("replace count with set");
        let _: () = store
            .client()
            .set("auth:retained_bytes", "17", None, None, false)
            .await
            .expect("install stale byte count");

        assert!(
            store
                .repair_authoritative_retention(usage)
                .await
                .expect("repair derivative counters")
        );
        assert_eq!(
            store
                .client()
                .get::<usize, _>("auth:retained_count")
                .await
                .unwrap(),
            usage.count
        );
        assert_eq!(
            store
                .client()
                .get::<usize, _>("auth:retained_bytes")
                .await
                .unwrap(),
            usage.bytes
        );
        assert!(
            !store
                .repair_authoritative_retention(usage)
                .await
                .expect("exact counters are an idempotent no-op")
        );
    }
);

redis_gated!(
    authoritative_token_and_idempotency_conflicts_are_closed,
    store,
    {
        seed_minimal_world(&store).await;
        let first = authoritative_bundle(14, 24);
        store
            .reserve_authoritative_intent(first.clone())
            .await
            .expect("reserve first intent");

        let same_token_different_bundle = authoritative_bundle(14, 25);
        let token_error = store
            .reserve_authoritative_intent(same_token_different_bundle)
            .await
            .expect_err("one token cannot name different bytes");
        assert!(matches!(
            token_error,
            StoreError::AuthoritativeTokenConflict
        ));

        let same_idempotency_different_token = authoritative_bundle(15, 24);
        let idempotency_error = store
            .reserve_authoritative_intent(same_idempotency_different_token)
            .await
            .expect_err("one scoped idempotency key cannot name another token");
        assert!(matches!(
            idempotency_error,
            StoreError::AuthoritativeIdempotencyConflict
        ));

        let wrong_hash = CanonicalHash::from_bytes([99; 32]);
        assert_eq!(
            store
                .authoritative_status(first.token(), wrong_hash)
                .await
                .expect("hash-aware status"),
            BundleStatus::Conflict {
                stored_bundle_hash: first.bundle_hash(),
            }
        );
        assert!(matches!(
            store
                .reconcile_authoritative(first.token(), wrong_hash)
                .await
                .expect_err("reconcile must require the expected bundle hash"),
            StoreError::AuthoritativeTokenConflict
        ));
    }
);

redis_gated!(
    authoritative_unindexed_intent_is_found_by_full_scan,
    store,
    {
        use fred::interfaces::SetsInterface;

        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(12, 22);
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("reserve intent");

        // Simulate loss of the accelerator update. The immutable full intent is
        // still authoritative and exhaustive boot discovery must find it.
        let removed: i64 = store
            .client()
            .srem("auth:pending", bundle.token().to_string())
            .await
            .expect("remove pending accelerator member");
        assert_eq!(removed, 1);
        assert!(
            store
                .pending_authoritative_intents()
                .await
                .expect("pending accelerator")
                .is_empty()
        );

        let page = store
            .scan_authoritative_intents(None, 256)
            .await
            .expect("exhaustive intent scan");
        assert_eq!(page.intents, vec![bundle.clone()]);
        assert!(page.next_cursor.is_none());
        assert_eq!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .expect("intent status"),
            BundleStatus::PendingExact
        );
    }
);

redis_gated!(
    authoritative_wrong_type_partial_execution_keeps_exact_intent,
    store,
    {
        use fred::interfaces::KeysInterface;

        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(13, 23);
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("reserve intent");

        let module_key = "mod:redis_toy:action-state/v1/guarded/2";
        let _: () = store
            .client()
            .set(module_key, "wrong type", None, None, false)
            .await
            .expect("install deterministic poison");
        let error = store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect_err("RedisJSON must reject a string target");
        assert!(matches!(error, StoreError::Redis(_)), "got {error:?}");
        assert!(
            !error.is_transient(),
            "a deterministic WRONGTYPE must not enter an unbounded retry loop"
        );
        assert_eq!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .expect("intent survives partial execution"),
            BundleStatus::PendingExact
        );
        assert_eq!(
            store
                .pending_authoritative_intents()
                .await
                .expect("pending intent remains indexed")
                .len(),
            1
        );

        // Object and allocator precede module targets in canonical order, so
        // this proves the protocol can recover after a partially materialized
        // attempt without recomputing action logic.
        let world = store
            .load_world()
            .await
            .expect("load partially materialized world")
            .expect("world exists");
        assert_eq!(world.next_id, 3);
        assert!(world.objects.iter().any(|object| object.id == ObjId(2)));

        let deleted: i64 = store
            .client()
            .del(module_key)
            .await
            .expect("remove deterministic poison");
        assert_eq!(deleted, 1);
        let receipt = store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("resume exact replay after repair");
        assert!(receipt.matches_bundle(&bundle));
    }
);

redis_gated!(index_bootstrap_and_search, store, {
    // ensure_index already ran in connect(); running it again must be a
    // clean no-op ("Index already exists" is swallowed).
    store.ensure_index().await;

    // Write a distinctively named object and find it through FT.SEARCH.
    let mut obj = mush_core::Obj::new(
        mush_core::ObjId(7),
        mush_core::ObjKind::Thing,
        "Xyzzyplugh",
        mush_core::GOD,
        100,
    );
    obj.location = Some(mush_core::LIMBO);
    store
        .write(WriteBatch {
            serial: 1,
            upserts: vec![obj],
            next_id: Some(8),
            ..WriteBatch::default()
        })
        .await
        .expect("write searchable object");

    // Indexing is near-synchronous for JSON docs, but give it a moment.
    let mut found = Vec::new();
    for _ in 0..20 {
        found = store.search_names("Xyzzyplugh").await.expect("ft_search");
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        found,
        vec![mush_core::ObjId(7)],
        "FT index must resolve the object by name"
    );
});

redis_gated!(search_special_characters_are_literal, store, {
    // A query full of RediSearch syntax must be escaped into a literal
    // term: no syntax error, just no matches.
    let found = store
        .search_names("no)such|name* -neg @field:(x)")
        .await
        .expect("escaped query must not be a syntax error");
    assert!(found.is_empty());
});

redis_gated!(link_character_rejects_non_array_characters, store, {
    use fred::interfaces::RedisJsonInterface;
    use mush_store::{AccountRec, StoreError};

    store
        .create_account(AccountRec {
            name: "Mangled".into(),
            argon2_hash: "h".into(),
            created: 1,
            characters: vec![],
            ssh_keys: vec![],
        })
        .await
        .expect("create account");
    // Clobber the characters field with a non-array. JSON.ARRAPPEND then
    // replies [nil] for the matched path — which must surface as Corrupt,
    // not silently link nothing.
    let _: Option<String> = store
        .client()
        .json_set("account:mangled", "$.characters", "\"oops\"", None)
        .await
        .expect("clobber characters");
    let err = store
        .link_character("mangled", mush_core::ObjId(7), "Ghost")
        .await
        .expect_err("linking into a non-array must fail");
    assert!(
        matches!(err, StoreError::Corrupt(_)),
        "expected Corrupt, got {err:?}"
    );
});

redis_gated!(set_account_ssh_keys_on_legacy_doc, store, {
    use fred::interfaces::RedisJsonInterface;
    use mush_store::SshKeyRec;

    // Simulate an account doc that predates the ssh_keys field: write the
    // root JSON with no `ssh_keys` key at all. set_account_ssh_keys must
    // still place the array (its `$.ssh_keys` path does not yet exist, so
    // an XX set would have failed — the whole point of the create-or-replace
    // gate).
    let _: Option<String> = store
        .client()
        .json_set(
            "account:legacy",
            "$",
            r#"{"name":"Legacy","argon2_hash":"$argon2id$x","created":1,"characters":[]}"#,
            None,
        )
        .await
        .expect("write legacy account doc");

    // It deserializes (additive #[serde(default)]) with an empty key list.
    let rec = store
        .get_account("legacy")
        .await
        .expect("get")
        .expect("exists");
    assert!(rec.ssh_keys.is_empty(), "legacy doc has no keys");

    let keys = vec![SshKeyRec {
        openssh: "ssh-ed25519 AAAAlegacy nick@box".into(),
        fingerprint: "SHA256:legacy".into(),
        added: 42,
    }];
    assert!(
        store
            .set_account_ssh_keys("legacy", &keys)
            .await
            .expect("set on legacy doc must succeed"),
        "existing legacy account must report true"
    );
    let rec = store
        .get_account("legacy")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(rec.ssh_keys, keys, "keys written onto a pre-field doc");
    assert_eq!(rec.name, "Legacy", "rest of the doc intact");
});

redis_gated!(ban_native_ttl, store, {
    // Bans expire server-side via EXAT — no client filtering. Two bans, two
    // claims: a long-TTL ban proves visibility (immune to client/server clock
    // skew and WAN latency, both real when REDIS_URL crosses an SSH tunnel),
    // and a short-TTL ban proves per-key server-side expiry, polled with a
    // deadline rather than a fixed sleep for the same reason.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    store
        .set_ban("10.0.0.1", Some(now + 60), "long enough to observe")
        .await
        .expect("set long ban");
    store
        .set_ban("10.0.0.2", Some(now + 2), "blink and it's gone")
        .await
        .expect("set short ban");
    let bans = store.get_bans().await.expect("get_bans");
    assert!(
        bans.iter().any(|b| b.ip == "10.0.0.1"),
        "long ban visible before expiry: {bans:?}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let bans = store.get_bans().await.expect("get_bans while polling");
        if !bans.iter().any(|b| b.ip == "10.0.0.2") {
            assert!(
                bans.iter().any(|b| b.ip == "10.0.0.1"),
                "expiry must be per-key: long ban vanished with the short one"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Redis TTL never expired the short ban: {bans:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
});

redis_gated!(token_native_ttl, store, {
    // Tokens expire server-side via EXAT — same claims as ban_native_ttl:
    // a long-TTL token proves visibility despite clock skew/WAN latency,
    // a short-TTL token proves per-key server-side expiry (polled).
    use mush_store::TokenRecord;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let rec = |expires: i64| TokenRecord {
        account: "Nick".into(),
        created: now,
        expires,
    };
    store
        .token_put("longhash".into(), rec(now + 60))
        .await
        .expect("put long token");
    store
        .token_put("shorthash".into(), rec(now + 2))
        .await
        .expect("put short token");
    assert!(
        store
            .token_get("longhash")
            .await
            .expect("get long")
            .is_some(),
        "long token visible before expiry"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if store
            .token_get("shorthash")
            .await
            .expect("get short while polling")
            .is_none()
        {
            assert!(
                store
                    .token_get("longhash")
                    .await
                    .expect("get long after short expired")
                    .is_some(),
                "expiry must be per-key: long token vanished with the short one"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Redis TTL never expired the short token"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
});

redis_gated!(kill9_equivalent_reconnect_load, store, {
    // The store-side half of the M2 exit criterion: write a world, drop the
    // client (a kill -9'd server never says goodbye), reconnect fresh, and
    // load an identical world including the last acked batch.
    suite::empty_seed_load_round_trip(&store).await;
    store
        .write(WriteBatch {
            serial: 99,
            upserts: vec![{
                let mut o = mush_core::Obj::new(
                    mush_core::ObjId(2),
                    mush_core::ObjKind::Thing,
                    "last acked command",
                    mush_core::GOD,
                    100,
                );
                o.location = Some(mush_core::LIMBO);
                o
            }],
            next_id: Some(3),
            durable: true,
            ..WriteBatch::default()
        })
        .await
        .expect("last write");
    drop(store); // no quit(), no drain(): simulated hard kill

    let url = std::env::var("REDIS_URL").expect("checked earlier");
    let store = RedisStore::connect(&url).await.expect("reconnect");
    let snap = store
        .load_world()
        .await
        .expect("load after hard kill")
        .expect("world survived");
    assert_eq!(snap.next_id, 3);
    assert!(
        snap.objects.iter().any(|o| o.name == "last acked command"),
        "the last acked batch must be present after restart"
    );
    store.wipe_namespaces().await.expect("cleanup");
    store.close().await.expect("close");
});
