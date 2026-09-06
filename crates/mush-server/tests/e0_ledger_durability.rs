//! E0 exit-criterion proof: a sealed ledger chunk whose batch is **dropped**
//! deterministically is re-emitted by the World-owned retained-write table and
//! becomes durable, and the durable-commit high-water clears it — end to end,
//! against the real persistence writer and a fault-injecting store.
//!
//! This drives the exact sequence `flush_dirty` runs (ack → fold re-emit →
//! capture → send) against [`mush_server::persist::spawn_store_writer`] and
//! [`mush_server::retained::RetainedSeals`], with a store that drops any batch
//! carrying a poison op (a deterministic, non-transient failure — the collateral
//! drop that strands a seal today). The seal must survive.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::metrics::Metrics;
use mush_core::{LedgerChunk, LedgerEntry, ModuleOp, ModuleWrite, chunk_key};
use mush_server::persist::{PersistenceIntegrityLatch, StoreJob, StoreWriter, spawn_store_writer};
use mush_server::quarantine::{QUARANTINE_RECORD_VERSION, QuarantineSpool};
use mush_server::retained::RetainedSeals;
use mush_server::world_task::{WriterSignal, WriterSignalReceiver, writer_signal_channel};
use mush_store::{
    AccountRec, BanRec, InMemoryStore, ObjectStore, SshKeyRec, StoreError, TokenRecord,
    WorldSnapshot, WriteBatch, WriteReceipt,
};
use smol_str::SmolStr;
use tokio::sync::oneshot;

/// An [`ObjectStore`] that deterministically DROPS any batch whose module ops
/// contain a poison key, delegating everything else to an inner in-memory store.
struct FaultStore {
    inner: InMemoryStore,
    poison_key: Mutex<Option<SmolStr>>,
    drops: AtomicUsize,
    bgsaves: AtomicUsize,
}

impl FaultStore {
    fn new(poison: Option<&str>) -> Arc<FaultStore> {
        Arc::new(FaultStore {
            inner: InMemoryStore::new(),
            poison_key: Mutex::new(poison.map(SmolStr::new)),
            drops: AtomicUsize::new(0),
            bgsaves: AtomicUsize::new(0),
        })
    }

    fn key_of(op: &ModuleOp) -> &SmolStr {
        match &op.op {
            ModuleWrite::JsonSet { key, .. }
            | ModuleWrite::Del { key }
            | ModuleWrite::StreamAppend { key, .. } => key,
        }
    }
}

impl ObjectStore for FaultStore {
    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            let poison = self.poison_key.lock().unwrap().clone();
            if let Some(pk) = poison
                && batch
                    .module_ops
                    .iter()
                    .any(|op| FaultStore::key_of(op) == &pk)
            {
                self.drops.fetch_add(1, Ordering::Relaxed);
                // Non-transient → the writer DROPS the batch (collateral seal loss).
                return Err(StoreError::Invalid(format!("poison op {pk:?} present")));
            }
            self.inner.write(batch).await
        })
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        self.inner.module_json_get(ns, key)
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.bgsaves.fetch_add(1, Ordering::Relaxed);
        self.inner.bgsave()
    }

    // The writer + this test never call anything else.
    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        self.inner.load_world()
    }
    fn create_account(&self, _: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn get_account<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn link_character<'a>(
        &'a self,
        _: &'a str,
        _: mush_core::ObjId,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn set_account_password<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn set_account_ssh_keys<'a>(
        &'a self,
        _: &'a str,
        _: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn set_account_discord<'a>(
        &'a self,
        _: &'a str,
        _: Option<&'a mush_store::DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_user_reserve<'a>(
        &'a self,
        _: u64,
        _: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_user_release(&self, _: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn account_by_discord_user(
        &self,
        _: u64,
    ) -> BoxFuture<'_, Result<Option<mush_store::AccountRec>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_code_put(
        &self,
        _: String,
        _: mush_store::DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_code_take<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<mush_store::DiscordCodeRecord>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_ban_put(
        &self,
        _: mush_store::DiscordBanRec,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_ban_get(
        &self,
        _: u64,
    ) -> BoxFuture<'_, Result<Option<mush_store::DiscordBanRec>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn discord_ban_del(&self, _: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn set_ban<'a>(
        &'a self,
        _: &'a str,
        _: Option<i64>,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn clear_ban<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn token_put(&self, _: String, _: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn token_get<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn token_touch<'a>(&'a self, _: &'a str, _: i64) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn token_del<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn tokens_del_account<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<u64, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn module_json_list<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn stream_range<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn set_schema_version(&self, _: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        unimplemented!("fault store: unused")
    }
    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.drain()
    }
    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.close()
    }
}

/// Drain every `LedgerBreach` currently buffered on the world channel,
/// returning each message's `seal` field. The writer sends these synchronously
/// (awaited) before it replies with the batch result, so by the time the caller
/// has the result they are all present.
fn drain_breaches(rx: &mut WriterSignalReceiver) -> Vec<Option<(SmolStr, SmolStr, String, bool)>> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let WriterSignal::LedgerBreach { seal, .. } = msg {
            out.push(seal);
        }
    }
    out
}

fn quarantine_records(path: &std::path::Path) -> Vec<serde_json::Value> {
    let mut records: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(entry.path()).unwrap())
                .unwrap()
        })
        .collect();
    records.sort_by(|left, right| {
        left["content"]["kind"]
            .as_str()
            .cmp(&right["content"]["kind"].as_str())
    });
    records
}

/// One sealed-chunk `ModuleOp` (`log:{log}:{chunk}` with `sealed: true`).
fn seal_op(ns: &str, log: &str, chunk: u64) -> ModuleOp {
    let value = serde_json::to_value(LedgerChunk {
        chunk,
        sealed: true,
        entries: vec![LedgerEntry {
            seq: 0,
            value: serde_json::json!("event"),
        }],
    })
    .unwrap();
    ModuleOp {
        ns: SmolStr::new(ns),
        op: ModuleWrite::JsonSet {
            key: chunk_key(log, chunk),
            value,
        },
    }
}

/// A poison non-ledger op.
fn poison_op(ns: &str, key: &str) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(ns),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new(key),
            value: serde_json::json!({"x": 1}),
        },
    }
}

/// Mimic ONE `flush_dirty` for a module-ops-only batch: apply the high-water,
/// fold the re-emit, capture, send durable, and return the writer's result.
/// Returns `None` when the flush would be empty (nothing to do).
async fn flush(
    retained: &mut RetainedSeals,
    writer: &StoreWriter,
    serial: &mut u64,
    fresh: Vec<ModuleOp>,
    high_water: &AtomicU64,
) -> Option<Result<u64, String>> {
    retained.ack(high_water.load(Ordering::Relaxed));
    let mut ops = fresh;
    ops.extend(retained.reemit_ops());
    if ops.is_empty() {
        return None;
    }
    *serial += 1;
    let batch = WriteBatch {
        serial: *serial,
        module_ops: ops,
        durable: true,
        ..WriteBatch::default()
    };
    retained
        .capture(&batch.module_ops, *serial)
        .expect("test batch must not contain conflicting immutable seals");
    let (tx, rx) = oneshot::channel();
    writer
        .send(StoreJob::Write {
            batch,
            notify: Some(tx),
        })
        .await
        .unwrap();
    Some(rx.await.unwrap())
}

/// The wiring one spawned writer exposes to a test.
type Rig = (
    Arc<FaultStore>,
    StoreWriter,
    tokio::task::JoinHandle<()>,
    Arc<AtomicU64>,
    WriterSignalReceiver,
);

/// Spawn a writer over a fault store, returning everything the tests poke.
fn spawn(poison: Option<&str>, quarantine_spool: QuarantineSpool) -> Rig {
    let store = FaultStore::new(poison);
    let high_water = Arc::new(AtomicU64::new(0));
    let (signal_tx, signal_rx) = writer_signal_channel();
    let (writer, join) = spawn_store_writer(
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        signal_tx,
        Arc::new(Metrics::new()),
        Arc::clone(&high_water),
        quarantine_spool,
        PersistenceIntegrityLatch::new(),
    );
    (store, writer, join, high_water, signal_rx)
}

/// Send one durable batch and await the writer's result.
async fn send(writer: &StoreWriter, serial: u64, ops: Vec<ModuleOp>) -> Result<u64, String> {
    let batch = WriteBatch {
        serial,
        module_ops: ops,
        durable: true,
        ..WriteBatch::default()
    };
    let (tx, rx) = oneshot::channel();
    writer
        .send(StoreJob::Write {
            batch,
            notify: Some(tx),
        })
        .await
        .unwrap();
    rx.await.unwrap()
}

/// Happy path: a seal commits, the writer publishes the high-water, and the
/// retained table clears on the ack. No faults, no re-emit needed.
#[tokio::test]
async fn happy_path_commit_advances_high_water_and_clears_retained() {
    let (store, writer, join, high_water, _rx) = spawn(None, QuarantineSpool::in_memory());
    let mut retained = RetainedSeals::new();
    let mut serial = 0u64;

    let r = flush(
        &mut retained,
        &writer,
        &mut serial,
        vec![seal_op("scenes", "s", 0)],
        &high_water,
    )
    .await;
    assert_eq!(r, Some(Ok(1)));
    assert_eq!(high_water.load(Ordering::Relaxed), 1);
    retained.ack(high_water.load(Ordering::Relaxed));
    assert!(retained.is_empty(), "the commit high-water clears the seal");
    assert!(
        store
            .module_json_get("scenes", chunk_key("s", 0).as_str())
            .await
            .unwrap()
            .is_some()
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

/// The fail-closed barrier is not a blanket snapshot ban: before any breach,
/// an ordinary BGSAVE still runs in FIFO order through the backing store.
#[tokio::test]
async fn bgsave_reaches_store_while_integrity_is_clear() {
    let (store, writer, join, _high_water, _rx) = spawn(None, QuarantineSpool::in_memory());
    let (done, receipt) = oneshot::channel();
    writer.send(StoreJob::Bgsave { done }).await.unwrap();
    receipt.await.unwrap().unwrap();
    assert_eq!(store.bgsaves.load(Ordering::Relaxed), 1);

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

/// A seal caught as COLLATERAL of another op's deterministic poison: the writer
/// fails closed (freeze), isolates the seal — which commits ALONE — and signals
/// it healed. The seal ends up durable; the world is told to freeze then drop it
/// from re-emit.
#[tokio::test]
async fn collateral_seal_heals_in_isolation_and_freezes() {
    let (store, writer, join, high_water, mut rx) =
        spawn(Some("comsys:poison"), QuarantineSpool::in_memory());

    // A batch whose poison is a NON-seal op; the seal is collateral.
    let r = send(
        &writer,
        1,
        vec![
            poison_op("comsys", "comsys:poison"),
            seal_op("scenes", "s", 0),
        ],
    )
    .await;
    assert!(r.is_err(), "the poisoned batch drops: {r:?}");
    assert_eq!(
        high_water.load(Ordering::Relaxed),
        0,
        "a dropped batch never advances the high-water"
    );

    // The writer isolated the seal, which committed alone → durable.
    let stored = store
        .module_json_get("scenes", chunk_key("s", 0).as_str())
        .await
        .unwrap();
    let chunk: LedgerChunk =
        serde_json::from_value(stored.expect("seal healed in isolation")).unwrap();
    assert!(chunk.sealed && chunk.entries.len() == 1);

    // World signals: an initial freeze (None) then the healed seal (quarantined=false).
    let breaches = drain_breaches(&mut rx);
    assert!(
        breaches.iter().any(|s| s.is_none()),
        "an initial fail-closed freeze was signaled: {breaches:?}"
    );
    assert!(
        breaches
            .iter()
            .any(|s| matches!(s, Some((_, k, _, false)) if k == &chunk_key("s", 0))),
        "the collateral seal was signaled healed: {breaches:?}"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

/// A deterministic drop is an integrity breach even when no sealed ledger
/// chunk rode in the batch: RAM may already contain the uncommitted mutation.
#[tokio::test]
async fn non_ledger_deterministic_drop_latches_integrity_failure() {
    let spool = std::env::temp_dir().join(format!("rustmush-e0-non-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&spool);
    let (_store, writer, join, _high_water, mut rx) = spawn(
        Some("comsys:poison"),
        QuarantineSpool::open(spool.clone()).unwrap(),
    );
    let result = send(&writer, 1, vec![poison_op("comsys", "comsys:poison")]).await;
    assert!(result.is_err());

    let signals: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(signals.iter().any(|signal| matches!(
        signal,
        WriterSignal::DeterministicFailure { detail }
            if detail.contains("deterministic drop of batch 1")
    )));
    let records = quarantine_records(&spool);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["content"]["kind"], "legacy_write_batch");
    assert_eq!(records[0]["content"]["batch"]["serial"], 1);
    assert_eq!(
        records[0]["content"]["batch"]["module_ops"][0]["ns"],
        "comsys"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_dir_all(&spool);
}

/// A seal that is ITSELF the deterministic poison: isolated retries all fail, so
/// the writer durably QUARANTINES it to the spool (a separate failure domain),
/// signals the world to freeze and drop it from re-emit, and never lets it back.
#[tokio::test]
async fn poison_seal_is_quarantined_to_spool_and_frozen() {
    let spool = std::env::temp_dir().join(format!("rustmush-e0-spool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&spool);
    // The seal's own chunk key is the poison: every write carrying it fails.
    let poison_key = chunk_key("econ", 0);
    let (store, writer, join, high_water, mut rx) = spawn(
        Some(poison_key.as_str()),
        QuarantineSpool::open(spool.clone()).unwrap(),
    );

    let r = send(&writer, 1, vec![seal_op("econ", "econ", 0)]).await;
    assert!(r.is_err(), "the poison-seal batch drops: {r:?}");
    assert_eq!(high_water.load(Ordering::Relaxed), 0);
    // The original drop + POISON_ISOLATION_ATTEMPTS isolated retries all failed.
    assert!(
        store.drops.load(Ordering::Relaxed) >= 6,
        "isolation retried the poison: {}",
        store.drops.load(Ordering::Relaxed)
    );
    // Never committed.
    assert!(
        store
            .module_json_get("econ", poison_key.as_str())
            .await
            .unwrap()
            .is_none()
    );

    // World signals: freeze (None) + quarantine (quarantined=true).
    let breaches = drain_breaches(&mut rx);
    assert!(
        breaches.iter().any(|s| s.is_none()),
        "fail-closed freeze signaled: {breaches:?}"
    );
    assert!(
        breaches
            .iter()
            .any(|s| matches!(s, Some((_, k, _, true)) if k == &poison_key)),
        "the poison seal was signaled quarantined: {breaches:?}"
    );

    // The spool holds a durable recovery record with the full payload + hash.
    let records = quarantine_records(&spool);
    assert_eq!(records.len(), 2, "the complete batch and seal are spooled");
    let record = records
        .iter()
        .find(|record| record["content"]["kind"] == "ledger_seal")
        .expect("seal-specific recovery record");
    assert!(
        records
            .iter()
            .any(|record| record["content"]["kind"] == "legacy_write_batch"),
        "the complete causal batch is also preserved"
    );
    assert_eq!(record["record_version"], QUARANTINE_RECORD_VERSION);
    assert_eq!(record["content"]["kind"], "ledger_seal");
    assert_eq!(record["content"]["namespace"], "econ");
    assert_eq!(record["content"]["key"], poison_key.as_str());
    assert!(
        record["content"]["payload"]["sealed"].as_bool().unwrap(),
        "the full seal payload is preserved"
    );
    assert!(record["identity_hash"].as_str().is_some());
    assert_eq!(record["isolation_attempts"].as_u64().unwrap(), 5);

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_dir_all(&spool);
}

/// Re-quarantining the same poison seal is idempotent — one complete-batch
/// record plus one seal record, with no serial-dependent record per retained
/// re-emit. The content-addressed seal and writer PoisonState prevent an
/// isolation storm while retaining the original causal batch.
#[tokio::test]
async fn requarantining_a_poison_seal_has_bounded_record_custody() {
    let spool = std::env::temp_dir().join(format!("rustmush-e0-idem-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&spool);
    let poison_key = chunk_key("econ", 0);
    let (store, writer, join, _hw, mut rx) = spawn(
        Some(poison_key.as_str()),
        QuarantineSpool::open(spool.clone()).unwrap(),
    );

    // Send the SAME poison seal three times (as the world's re-emit would).
    for serial in 1..=3 {
        let r = send(&writer, serial, vec![seal_op("econ", "econ", 0)]).await;
        assert!(r.is_err());
    }
    let _ = store; // silence unused in case asserts change

    // Exactly two durable records, and exactly one quarantine signal (the
    // freeze + first quarantine); later re-sends are recognized and skipped.
    let records = quarantine_records(&spool);
    assert_eq!(
        records.len(),
        2,
        "one original batch plus one seal record; no retry duplicates"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["content"]["kind"] == "legacy_write_batch")
            .count(),
        1
    );
    let quarantines = drain_breaches(&mut rx)
        .into_iter()
        .filter(|s| matches!(s, Some((_, _, _, true))))
        .count();
    assert_eq!(
        quarantines, 1,
        "the poison is signaled quarantined exactly once, not per re-send"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_dir_all(&spool);
}

/// A content digest, not merely `(namespace, key)`, is the poison-resolution
/// identity. If corrupt input presents different sealed bytes under an already
/// handled key, the second payload is independently isolated and preserved.
#[tokio::test]
async fn distinct_payloads_under_one_poison_key_are_both_quarantined() {
    let spool =
        std::env::temp_dir().join(format!("rustmush-e0-generations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&spool);
    let poison_key = chunk_key("econ", 0);
    let (_store, writer, join, _hw, mut rx) = spawn(
        Some(poison_key.as_str()),
        QuarantineSpool::open(spool.clone()).unwrap(),
    );

    let first = seal_op("econ", "econ", 0);
    let mut second = seal_op("econ", "econ", 0);
    let ModuleWrite::JsonSet { value, .. } = &mut second.op else {
        unreachable!()
    };
    value["entries"][0]["value"] = serde_json::json!("different event bytes");
    // Both generations occur in the one rejected predecessor. After that job,
    // the writer deliberately halts and will not inspect later jobs.
    assert!(send(&writer, 1, vec![first, second]).await.is_err());

    let records = quarantine_records(&spool);
    assert_eq!(
        records.len(),
        3,
        "one complete batch plus each distinct poison payload must survive"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["content"]["kind"] == "ledger_seal")
            .count(),
        2
    );
    assert_eq!(
        drain_breaches(&mut rx)
            .into_iter()
            .filter(|seal| matches!(seal, Some((_, _, _, true))))
            .count(),
        2,
        "each payload receives its own durable quarantine resolution"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_dir_all(&spool);
}

/// The load-bearing guarantee: when a poison seal and a HEALTHY seal are bundled
/// in one dropped batch, per-seal isolation must quarantine ONLY the poison and
/// preserve the good one (it commits alone) — the good seal must never be lost.
#[tokio::test]
async fn a_good_seal_bundled_with_a_poison_seal_is_not_lost() {
    let spool = std::env::temp_dir().join(format!("rustmush-e0-multi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&spool);
    let poison_key = chunk_key("econ", 0);
    let (store, writer, join, _hw, mut rx) = spawn(
        Some(poison_key.as_str()),
        QuarantineSpool::open(spool.clone()).unwrap(),
    );

    // One batch carrying BOTH a poison seal (econ) and a healthy seal (scenes).
    let r = send(
        &writer,
        1,
        vec![seal_op("econ", "econ", 0), seal_op("scenes", "s", 0)],
    )
    .await;
    assert!(
        r.is_err(),
        "the batch drops because of the poison seal: {r:?}"
    );

    // The healthy seal committed in isolation → durable. The poison never did.
    assert!(
        store
            .module_json_get("scenes", chunk_key("s", 0).as_str())
            .await
            .unwrap()
            .is_some(),
        "the good seal bundled with poison must be preserved, not lost"
    );
    assert!(
        store
            .module_json_get("econ", poison_key.as_str())
            .await
            .unwrap()
            .is_none()
    );

    // Signals: the good seal healed (false), the poison quarantined (true).
    let breaches = drain_breaches(&mut rx);
    assert!(
        breaches
            .iter()
            .any(|s| matches!(s, Some((_, k, _, false)) if k == &chunk_key("s", 0))),
        "good seal healed: {breaches:?}"
    );
    assert!(
        breaches
            .iter()
            .any(|s| matches!(s, Some((_, k, _, true)) if k == &poison_key)),
        "poison seal quarantined: {breaches:?}"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_dir_all(&spool);
}

/// After an unresolved predecessor, later queued writes are completed with an
/// error without touching the store. Ordinary BGSAVE fails closed too; a
/// future forensic export is a separate operation and destination.
#[tokio::test]
async fn writer_halts_after_drop_instead_of_committing_past_it() {
    // The poison is a non-seal op present ONLY in flush 1; the re-emit batch
    // carries the seal alone, so it commits.
    let store = FaultStore::new(Some("scenes:once"));
    let high_water = Arc::new(AtomicU64::new(0));
    // No world listener at all: the heal/freeze signals are dropped on the floor.
    let (signal_tx, signal_rx) = writer_signal_channel();
    drop(signal_rx);
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (writer, join) = spawn_store_writer(
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        signal_tx,
        Arc::new(Metrics::new()),
        Arc::clone(&high_water),
        QuarantineSpool::in_memory(),
        persistence_integrity.clone(),
    );
    let mut retained = RetainedSeals::new();
    let mut serial = 0u64;

    // Flush 1: [one-shot poison + seal] drops; the seal is captured for re-emit.
    // (Isolation heals+writes the seal, but the heal SIGNAL is lost — no listener.)
    let r1 = flush(
        &mut retained,
        &writer,
        &mut serial,
        vec![
            poison_op("scenes", "scenes:once"),
            seal_op("scenes", "s", 0),
        ],
        &high_water,
    )
    .await;
    assert!(matches!(r1, Some(Err(_))), "the batch drops: {r1:?}");
    // The heal signal was lost, so the world still holds the seal pending.
    assert_eq!(retained.depth(), 1, "re-emit backstop still holds the seal");

    // Flush 2 would succeed at the backing store now that the one-shot poison
    // was consumed. The writer must nevertheless reject it locally: committing
    // serial 2 past missing serial 1 would violate FIFO truth.
    let r2 = flush(&mut retained, &writer, &mut serial, vec![], &high_water).await;
    assert!(matches!(r2, Some(Err(_))), "halted writer rejects: {r2:?}");
    assert_eq!(high_water.load(Ordering::Relaxed), 0);
    retained.ack(high_water.load(Ordering::Relaxed));
    assert!(
        !retained.is_empty(),
        "without its exact resolution signal, retained state waits for recovery"
    );
    assert!(
        store
            .module_json_get("scenes", chunk_key("s", 0).as_str())
            .await
            .unwrap()
            .is_some()
    );

    let (done, receipt) = oneshot::channel();
    writer.send(StoreJob::Bgsave { done }).await.unwrap();
    let bgsave_error = receipt.await.unwrap().unwrap_err();
    assert!(
        bgsave_error.contains("integrity is latched"),
        "ordinary BGSAVE must explain the fail-closed barrier: {bgsave_error}"
    );
    assert!(persistence_integrity.is_latched());
    assert_eq!(
        store.bgsaves.load(Ordering::Relaxed),
        0,
        "a rejected BGSAVE must not reach the backing store"
    );

    drop(writer);
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}
