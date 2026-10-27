//! K0 boot-order proof: quarantine integrity is established before the object
//! store can be migrated, seeded, or handed to a module boot hook.

use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{LedgerChunk, LedgerEntry, ModuleOp, ModuleWrite, chunk_key};
use mush_server::quarantine::{LEGACY_QUARANTINE_RECORD_VERSION, seal_identity_hash};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};

fn scratch(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rustmush-quarantine-boot-{tag}-{}",
        std::process::id()
    ))
}

async fn assert_boot_refused_without_store_mutation(dir: &std::path::Path, expected_error: &str) {
    let mut config = Config::default();
    config.store.store = "memory".to_owned();
    config.server.data_dir = dir.to_owned();
    let store = Arc::new(InMemoryStore::new());

    let error = mush_server::serve_with_store(
        config,
        None,
        Some(Arc::clone(&store) as Arc<dyn ObjectStore>),
    )
    .await
    .expect_err("quarantine state must stop boot");
    assert!(error.contains(expected_error), "{error}");
    assert_eq!(
        store.get_schema_version().await.unwrap(),
        None,
        "core migrations must not stamp the store"
    );
    assert!(
        store.load_world().await.unwrap().is_none(),
        "world seeding must not run"
    );
}

#[tokio::test]
async fn partial_quarantine_spool_prevents_every_store_mutation() {
    let dir = scratch("partial");
    let _ = std::fs::remove_dir_all(&dir);
    let spool = dir.join("quarantine");
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join("crash-left.tmp"), b"partial").unwrap();

    assert_boot_refused_without_store_mutation(&dir, "quarantine spool preflight failed").await;

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn valid_unresolved_record_prevents_every_store_mutation() {
    let dir = scratch("unresolved");
    let _ = std::fs::remove_dir_all(&dir);
    let spool = dir.join("quarantine");
    std::fs::create_dir_all(&spool).unwrap();

    let key = chunk_key("economy", 0).to_string();
    let payload = serde_json::to_value(LedgerChunk {
        chunk: 0,
        sealed: true,
        entries: vec![LedgerEntry {
            seq: 0,
            value: serde_json::json!({"transfer": "unresolved"}),
        }],
    })
    .unwrap();
    let identity = seal_identity_hash("economy", &key, &payload);
    let record = serde_json::json!({
        "record_version": LEGACY_QUARANTINE_RECORD_VERSION,
        "ns": "economy",
        "key": key.clone(),
        "serial": 9,
        "payload_hash": identity.clone(),
        "payload": payload,
        "store_error": "deterministic fixture",
        "error_transient": false,
        "isolation_attempts": 5,
        "quarantined_unix": 1,
        "server_version": env!("CARGO_PKG_VERSION"),
    });
    std::fs::write(
        spool.join(format!("{identity}.json")),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();

    assert_boot_refused_without_store_mutation(&dir, "quarantine recovery required").await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn exact_target_is_archived_before_any_boot_mutation() {
    let dir = scratch("exact-target");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(dir.join("quarantine-resolved"));
    let active_dir = dir.join("quarantine");
    std::fs::create_dir_all(&active_dir).unwrap();

    let key = chunk_key("economy", 0).to_string();
    let payload = serde_json::to_value(LedgerChunk {
        chunk: 0,
        sealed: true,
        entries: vec![LedgerEntry {
            seq: 0,
            value: serde_json::json!({"transfer": "ack-lost"}),
        }],
    })
    .unwrap();
    let identity = seal_identity_hash("economy", &key, &payload);
    let record = serde_json::json!({
        "record_version": LEGACY_QUARANTINE_RECORD_VERSION,
        "ns": "economy",
        "key": key.clone(),
        "serial": 10,
        "payload_hash": identity.clone(),
        "payload": payload.clone(),
        "store_error": "acknowledgement lost fixture",
        "error_transient": false,
        "isolation_attempts": 5,
        "quarantined_unix": 1,
        "server_version": env!("CARGO_PKG_VERSION"),
    });
    std::fs::write(
        active_dir.join(format!("{identity}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();

    let store = Arc::new(InMemoryStore::new());
    store
        .write(WriteBatch {
            serial: 1,
            module_ops: vec![ModuleOp {
                ns: "economy".into(),
                op: ModuleWrite::JsonSet {
                    key: key.clone().into(),
                    value: payload.clone(),
                },
            }],
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    let mut config = Config::default();
    config.store.store = "memory".to_owned();
    config.server.data_dir = dir.clone();
    let mut spool = mush_server::quarantine::QuarantineSpool::open(&active_dir).unwrap();

    mush_server::prepare_quarantine_for_writable_boot(&config, &mut spool, Some(&*store))
        .await
        .unwrap();

    spool.require_resolved_for_boot().unwrap();
    assert_eq!(store.get_schema_version().await.unwrap(), None);
    assert_eq!(
        store.module_json_get("economy", &key).await.unwrap(),
        Some(payload),
        "the recovery pass observes but never rewrites the target"
    );
    assert_eq!(
        std::fs::read_dir(&active_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(dir.join("quarantine-resolved"))
            .unwrap()
            .count(),
        1
    );

    drop(spool);
    std::fs::remove_dir_all(dir.join("quarantine-resolved")).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
