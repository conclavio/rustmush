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
async fn ban_expiry() {
    suite::ban_expiry(&InMemoryStore::new()).await;
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
async fn module_kv_round_trip() {
    suite::module_kv_round_trip(&InMemoryStore::new()).await;
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
