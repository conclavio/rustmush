//! The shared behavior suite against [`InMemoryStore`] — runs everywhere,
//! no Redis required. `tests/redis_suite.rs` runs the identical functions
//! against a live server when `REDIS_URL` is set.

use mush_store::{InMemoryStore, ObjectStore, StoreError, suite};

#[tokio::test]
async fn empty_seed_load_round_trip() {
    suite::empty_seed_load_round_trip(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn batch_atomicity() {
    suite::batch_atomicity(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn next_id_persistence() {
    suite::next_id_persistence(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_reserve_race() {
    suite::account_reserve_race(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_lifecycle() {
    suite::account_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_password_update() {
    suite::account_password_update(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_ssh_keys_update() {
    suite::account_ssh_keys_update(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_discord_link() {
    suite::account_discord_link(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_id_round_trips_and_is_immutable() {
    suite::account_id_round_trips_and_is_immutable(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn account_id_backfill_is_idempotent() {
    suite::account_id_backfill_is_idempotent(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn ban_expiry() {
    suite::ban_expiry(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn token_lifecycle() {
    suite::token_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn token_sliding_ttl() {
    suite::token_sliding_ttl(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn token_del_account() {
    suite::token_del_account(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn migration_version_bump() {
    suite::migration_version_bump(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn serde_obj_round_trip() {
    suite::serde_obj_round_trip(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_batch_atomicity() {
    suite::module_batch_atomicity(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn over_deep_module_doc_refuses_the_whole_batch() {
    suite::over_deep_module_doc_refuses_the_whole_batch(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_module_prefix_is_store_owned() {
    suite::authoritative_module_prefix_is_store_owned(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_exact_lifecycle() {
    suite::authoritative_exact_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_mixed_ledger_lifecycle() {
    suite::authoritative_mixed_ledger_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_resolution_checkpoint() {
    suite::authoritative_resolution_checkpoint(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_single_pending_admission() {
    suite::authoritative_single_pending_admission(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_quarantine_lifecycle() {
    suite::authoritative_quarantine_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_outbox_lifecycle() {
    suite::authoritative_outbox_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_epoch_lifecycle() {
    suite::authoritative_epoch_lifecycle(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_conflicts() {
    suite::authoritative_conflict_contract(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_retention_generation() {
    suite::authoritative_retention_generation_contract(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_number_shaped_post_images() {
    suite::authoritative_number_shaped_post_image_round_trips(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_finalize_survives_renormalisation() {
    suite::authoritative_finalize_survives_a_renormalised_post_image(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn authoritative_contract_checkpoint_accepts_renormalisation() {
    suite::authoritative_checkpoint_accepts_a_renormalised_post_image(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_kv_round_trip() {
    suite::module_kv_round_trip(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_docs_splits_the_ledger_family() {
    suite::module_docs_splits_the_ledger_family(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_keys_text_and_purge() {
    suite::module_keys_text_and_purge(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_docs_from_narrows_to_the_floors() {
    suite::module_docs_from_narrows_to_the_floors(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_stream_ordering() {
    suite::module_stream_ordering(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn module_empty_namespace_list() {
    suite::module_empty_namespace_list(&InMemoryStore::new()).await;
}

#[tokio::test]
async fn closed_store_refuses_work() {
    let store = InMemoryStore::new();
    store.close().await.expect("close");
    assert!(matches!(store.load_world().await, Err(StoreError::Closed)));
    // An empty batch short-circuits before touching state; a real one must
    // fail closed.
    assert!(store.write(mush_store::WriteBatch::default()).await.is_ok());
    let batch = mush_store::WriteBatch {
        serial: 1,
        next_id: Some(2),
        ..Default::default()
    };
    assert!(matches!(store.write(batch).await, Err(StoreError::Closed)));
    assert!(matches!(
        store.get_account("x").await,
        Err(StoreError::Closed)
    ));
}
