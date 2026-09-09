//! WS04-S1: the one boot-time store open, and the one archive scheduler.
//!
//! Before this the two roots each held half of `open_store`: the split server
//! validated `[store].receipt_policy` and never opened a ledger archive, and
//! `mushd` opened the archive and (until WS04-S0) never read the policy. Both
//! halves now come from `mush_server::boot`, so these cases assert both halves
//! through the ONE entry point:
//!
//! - the archive tier is attached to the backend the config chose, so a chunk
//!   the primary never held still answers — the READ half of J5;
//! - `[store].receipt_policy` is still parsed and still refused where it cannot
//!   be honoured — the validation the move had to carry across;
//! - the Redis failure still carries the operator remediation text, which is
//!   the richer of the two messages the fork had drifted into;
//! - `spawn_ledger_archive` refuses a malformed window instead of silently not
//!   scheduling, and when it does schedule it publishes the archive gauge — the
//!   WRITE/purge half, the one whose absence was costing unbounded Redis growth.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_config::metrics::Metrics;
use mush_server::boot;
use mush_store::{InMemoryStore, LedgerArchive, ObjectStore};

const NS: &str = "economy";
const LOG: &str = "journal";
/// A chunk index the primary store never holds in any of these cases.
const CHUNK: u64 = 7;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-bootstore-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A memory-backed config on a scratch data dir.
fn memory_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg
}

/// The text of one sealed chunk, as the ledger writes it.
fn sealed_chunk_text(chunk: u64) -> String {
    serde_json::json!({
        "chunk": chunk,
        "sealed": true,
        "entries": [{ "seq": chunk * 2, "value": { "n": chunk * 2 } }],
    })
    .to_string()
}

/// Seed `<data_dir>/<archive dir>` with one sealed chunk, as a previous night's
/// pass would have left it, and return the key it is addressed by.
fn seed_archived_chunk(cfg: &Config) -> String {
    let root = cfg.server.data_dir.join(&cfg.store.archive.dir);
    let archive = LedgerArchive::open(&root).unwrap();
    let admitted = archive
        .store_chunks(NS, LOG, vec![(CHUNK, sealed_chunk_text(CHUNK))], 1_000, 8)
        .unwrap();
    assert_eq!(admitted, vec![CHUNK], "the fixture chunk was admitted");
    assert!(archive.holds(NS, LOG, CHUNK).is_some());
    drop(archive);
    format!("log:{LOG}:{CHUNK}")
}

/// The J5 read fall-through is live on whatever backend the config names: a
/// chunk only the archive holds answers through the store `open_store` returns.
///
/// The key shape is asserted FIRST. `mush_store::archive::tiered_text` returns
/// `Ok(None)` for any key `mush_core::parse_chunk_key` rejects, so a mis-shaped
/// fixture would make the interesting assertion unreachable and the case would
/// pass vacuously against a tree with no archive wired in at all.
#[tokio::test]
async fn open_store_serves_a_purged_chunk_from_the_archive() {
    let mut cfg = memory_config("archive-read");
    cfg.store.archive.enabled = true;
    let key = seed_archived_chunk(&cfg);
    assert!(
        mush_core::parse_chunk_key(&key).is_some(),
        "the fixture key {key:?} must be one tiered_text will consult the archive for"
    );

    let opened = boot::open_store(&cfg).await.unwrap();
    assert!(
        opened.archive.is_some(),
        "[store.archive].enabled must yield the tier handle for the write pass"
    );
    let text = opened.store.module_doc_text(NS, &key).await.unwrap();
    assert!(
        text.is_some(),
        "the archive is not attached to the store: {key} read as {text:?}"
    );
    let doc = opened.store.module_json_get(NS, &key).await.unwrap();
    assert_eq!(
        doc.expect("the same chunk answers as JSON")["chunk"],
        CHUNK,
        "the tier served a different chunk"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// With the tier off there is no handle and no fall-through: the same key that
/// answers above reads as absent.
#[tokio::test]
async fn open_store_archive_is_none_when_archive_disabled() {
    let cfg = memory_config("archive-off");
    // Seed the directory anyway: only `enabled` decides.
    let key = seed_archived_chunk(&cfg);

    let opened = boot::open_store(&cfg).await.unwrap();
    assert!(opened.archive.is_none());
    assert_eq!(
        opened.store.module_doc_text(NS, &key).await.unwrap(),
        None,
        "a disabled tier must not be consulted"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// The receipt-policy validation survived the move: an unknown spelling refuses
/// the boot naming the value rather than quietly downgrading to `applied`.
#[tokio::test]
async fn open_store_rejects_a_bogus_receipt_policy() {
    let mut cfg = memory_config("policy-bogus");
    cfg.store.receipt_policy = "hostpowerloss".to_string();
    let error = boot::open_store(&cfg)
        .await
        .expect_err("an unknown receipt policy must refuse the boot");
    assert!(
        error.contains("hostpowerloss"),
        "the error must name the rejected value: {error}"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// … and the promise nothing in RAM can keep is refused outright.
#[tokio::test]
async fn open_store_refuses_host_power_loss_on_memory() {
    let mut cfg = memory_config("policy-memory");
    cfg.store.receipt_policy = "host_power_loss".to_string();
    let error = boot::open_store(&cfg)
        .await
        .expect_err("host_power_loss on the memory backend must refuse the boot");
    assert!(
        error.contains("requires the Redis backend"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// The merge kept the RICHER of the two Redis failure messages: `mushd`'s
/// multi-line remediation, not the split half's terse `cannot reach Redis`.
/// dev1/dev2 boot on this text.
#[tokio::test]
async fn open_store_keeps_the_operator_remediation_text() {
    let mut cfg = memory_config("redis-down");
    cfg.store.store = "redis".to_string();
    // Port 1 is never a Redis: the connection is refused, not timed out.
    cfg.store.redis_url = "redis://127.0.0.1:1".to_string();
    let error = tokio::time::timeout(Duration::from_secs(60), boot::open_store(&cfg))
        .await
        .expect("connect_read_only sets fail_fast; an unreachable store must not hang")
        .expect_err("an unreachable Redis must refuse the boot");
    assert!(
        error.contains("cannot reach Redis at redis://127.0.0.1:1"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("set [store].store"),
        "the operator remediation text was lost in the merge: {error}"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// A window the parser cannot read is a config error, not a pass that quietly
/// never runs.
#[tokio::test]
async fn spawn_ledger_archive_reports_a_malformed_window() {
    let mut cfg = memory_config("window-bad");
    cfg.store.archive.enabled = true;
    cfg.store.archive.window = "3pm-4pm".to_string();
    let root = cfg.server.data_dir.join(&cfg.store.archive.dir);
    let archive = Arc::new(LedgerArchive::open(&root).unwrap());
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let metrics = Arc::new(Metrics::new());

    let error =
        boot::spawn_ledger_archive(&cfg, &store, Some(archive), vec![NS.to_string()], &metrics)
            .expect_err("a malformed window must refuse rather than skip the pass");
    assert!(error.contains("3pm"), "unexpected error: {error}");
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// The WRITE/purge half is live, and the caller is handed its join handle.
///
/// The scheduled job publishes the archive gauge on the passed registry the
/// moment it starts, so an untouched registry proves nothing was spawned.
#[tokio::test]
async fn spawn_ledger_archive_schedules_when_enabled() {
    let mut cfg = memory_config("schedule-on");
    cfg.store.archive.enabled = true;
    seed_archived_chunk(&cfg);

    let opened = boot::open_store(&cfg).await.unwrap();
    let metrics = Arc::new(Metrics::new());
    assert_eq!(metrics.snapshot().ledger_archive_chunks, 0, "untouched");

    let handle = boot::spawn_ledger_archive(
        &cfg,
        &opened.store,
        opened.archive,
        vec![NS.to_string()],
        &metrics,
    )
    .unwrap()
    .expect("an enabled tier schedules the nightly pass");

    // The job publishes before its first sleep; give the scheduler a turn.
    let mut snapshot = metrics.snapshot();
    for _ in 0..200 {
        if snapshot.ledger_archive_chunks != 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        snapshot = metrics.snapshot();
    }
    assert_eq!(
        snapshot.ledger_archive_chunks, 1,
        "the spawned pass never published its gauge; the write half is dead"
    );
    assert_eq!(
        snapshot.ledger_resident_bytes, -1,
        "the start-up sample reports resident bytes as unmeasured"
    );

    handle.abort();
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// With the tier off there is no job and no error.
#[tokio::test]
async fn spawn_ledger_archive_is_none_when_disabled() {
    let cfg = memory_config("schedule-off");
    let opened = boot::open_store(&cfg).await.unwrap();
    let metrics = Arc::new(Metrics::new());
    let handle = boot::spawn_ledger_archive(
        &cfg,
        &opened.store,
        opened.archive,
        vec![NS.to_string()],
        &metrics,
    )
    .unwrap();
    assert!(handle.is_none());
    assert_eq!(metrics.snapshot().ledger_archive_chunks, 0);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}
