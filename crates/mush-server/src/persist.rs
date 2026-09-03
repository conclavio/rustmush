//! The write-through persistence pipeline (M2, PLAN "Persistence").
//!
//! Layout:
//!
//! - [`load_or_seed_world`] — boot: hydrate the [`World`] from the store
//!   ([`mush_store::ObjectStore::load_world`] →
//!   [`World::from_objects`]); a completely empty store is seeded with the
//!   minimal world (Limbo `#0` + God `#1`) **and the seed batch is written
//!   through immediately**, so even the very first boot is durable.
//! - [`batch_from_dirty`] — after every engine step that dirtied the world,
//!   the world task snapshots the dirty objects into one [`WriteBatch`]
//!   (full-object upserts, tombstones as deletes, `next_id` last).
//!   `mush-core` never learns the store exists: the seam is exactly
//!   `World::take_dirty()` + `Obj: Clone`.
//! - [`spawn_store_writer`] — the dedicated writer task. One bounded queue
//!   of [`StoreJob`]s, applied **in order**, each awaited to its ack before
//!   the next (write-through per command; no timer batching). On a
//!   *transient* write failure (connection, timeout — see
//!   [`StoreError::is_transient`]) it retries the same batch with
//!   exponential backoff forever — order is preserved, nothing is dropped —
//!   and tells the world task to pause mutations
//!   ([`WriterSignal::StoreStatus`]) until the store recovers (PLAN risk 3). A
//!   *deterministic* failure (serialization, a type-clobbered key) would
//!   fail identically forever, so the batch is dropped with a loud error
//!   instead of wedging the writer behind it. **Self-healing is partial**:
//!   object upserts are full snapshots, so RAM heals the store on the next
//!   write touching those objects — but a dropped batch's *module ops* do
//!   not heal (a dropped `Del` resurrects its doc at next boot; a dropped
//!   `StreamAppend` is lost), so the affected `mod:*` keys are logged for
//!   manual repair.
//!
//! Durability-critical batches (`durable: true`, e.g. Player creation)
//! carry a receipt oneshot; the caller gates its confirmation output on it,
//! and a dropped batch reports `Err` — never a false success. `@dump` rides
//! the same queue as [`StoreJob::Bgsave`] so the snapshot is ordered behind
//! every batch queued before it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mush_core::{ObjId, World};
use mush_config::metrics::Metrics;
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, StoreError, WriteBatch};
use smol_str::SmolStr;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use mush_config::unix_now_s;
use crate::world_task::WriterSignal;

/// How many times a sealed chunk is retried **in isolation** (alone, in its own
/// batch) before it is declared a deterministic poison and quarantined (E0). The
/// isolation is what separates a genuine poison seal from a seal merely caught
/// as collateral in a batch some *other* op poisoned — the collateral one
/// commits alone on the first attempt. Small: a deterministic failure repeats
/// identically, so a handful of attempts is diagnosis, not hope.
const POISON_ISOLATION_ATTEMPTS: u32 = 5;

/// Capacity of the writer's job queue. The world task `await`s its sends,
/// so a full queue back-pressures the single producer instead of dropping
/// batches (and read-only mode keeps it from filling during a detected
/// outage).
///
/// Deliberately small: every queued batch is a command whose output the
/// player has already seen but whose write has not yet been acked, so this
/// bounds the worst-case `kill -9` loss window while the store is merely
/// *slow* (a stalled BGSAVE, a saturated disk). In the steady state the
/// queue is empty and the loss window is exactly the in-flight command.
const WRITER_QUEUE: usize = 64;

/// First retry delay after a failed write; doubles up to [`RETRY_MAX`].
const RETRY_INITIAL: Duration = Duration::from_millis(100);

/// Retry backoff ceiling.
const RETRY_MAX: Duration = Duration::from_secs(5);

/// One unit of work for the store-writer task.
pub enum StoreJob {
    /// Apply one batch (acked in order; retried forever on failure).
    Write {
        /// The batch.
        batch: WriteBatch,
        /// Receipt for ack-gated flows (durable batches); `None` for
        /// fire-and-forget ordinary command batches.
        notify: Option<oneshot::Sender<Result<u64, String>>>,
    },
    /// Trigger a background snapshot (`@dump`), ordered behind everything
    /// queued before it.
    Bgsave {
        /// Resolved when the store acknowledged the BGSAVE request.
        done: oneshot::Sender<Result<(), String>>,
    },
}

/// Spawn the store-writer task. `signals` is the dedicated UNBOUNDED channel the
/// writer publishes health transitions ([`WriterSignal::StoreStatus`]) and
/// ledger breaches on — unbounded so the writer can never block sending one and
/// deadlock against the world's job-queue back-pressure. The task drains its
/// queue and exits when every job sender is dropped.
pub fn spawn_store_writer(
    store: Arc<dyn ObjectStore>,
    signals: mpsc::UnboundedSender<WriterSignal>,
    metrics: Arc<Metrics>,
    high_water: Arc<AtomicU64>,
    quarantine_dir: Option<PathBuf>,
) -> (mpsc::Sender<StoreJob>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(WRITER_QUEUE);
    let join = tokio::spawn(writer_loop(store, rx, signals, metrics, high_water, quarantine_dir));
    (tx, join)
}

async fn writer_loop(
    store: Arc<dyn ObjectStore>,
    mut rx: mpsc::Receiver<StoreJob>,
    signals: mpsc::UnboundedSender<WriterSignal>,
    metrics: Arc<Metrics>,
    high_water: Arc<AtomicU64>,
    quarantine_dir: Option<PathBuf>,
) {
    tracing::info!("store writer running");
    let mut healthy = true;
    // E0 poison-seal bookkeeping, persisted across jobs so a re-emitted seal is
    // recognized rather than re-isolated/re-spooled/re-frozen every cycle.
    let mut poison = PoisonState::default();
    while let Some(job) = rx.recv().await {
        match job {
            StoreJob::Write { batch, notify } => {
                let result = write_with_retry(
                    &*store,
                    batch,
                    &mut healthy,
                    &signals,
                    quarantine_dir.as_deref(),
                    &mut poison,
                )
                .await;
                // The batch has left the queue (acked or dropped): drop the
                // depth the world task incremented on enqueue, and stamp the
                // last-ack time on success (the metrics surface's store health).
                metrics.add_store_write_queue_depth(-1);
                if let Ok(&serial) = result.as_ref() {
                    metrics.set_store_last_ack_unix(unix_now_s());
                    // Publish the durable-commit high-water (E0): the world loop
                    // reads it to clear retained sealed-chunk re-emits. Monotone
                    // — batches commit in FIFO order, and `fetch_max` tolerates
                    // any reordering. A dropped batch's serial is simply skipped.
                    high_water.fetch_max(serial, Ordering::Relaxed);
                }
                if let Some(notify) = notify {
                    // Forward the truth — a dropped batch must never ack as
                    // success (the ack gate for creations rides on this).
                    // A dropped receiver just means the requester gave up.
                    let _ = notify.send(result);
                }
            }
            StoreJob::Bgsave { done } => {
                let result = store.bgsave().await.map_err(|e| e.to_string());
                if let Err(e) = &result {
                    tracing::error!(error = %e, "bgsave failed");
                }
                let _ = done.send(result);
            }
        }
    }
    tracing::info!("store writer drained and exiting");
}

/// Apply one batch, retrying transient failures with backoff until the
/// store acks it. Health transitions (ok→failing, failing→ok) are reported
/// to the world task so it can enter/leave read-only mode.
///
/// Returns `Ok(serial)` only when the store actually acknowledged the
/// batch; `Err` when the batch was **dropped** — the store is closed
/// (abnormal teardown) or the failure is deterministic
/// ([`StoreError::is_transient`] is false) and retrying it forever would
/// wedge every batch queued behind it. Dropped batches are loudly logged;
/// RAM remains ahead of the store until the affected objects are next
/// written (full-object upserts self-heal).
async fn write_with_retry(
    store: &dyn ObjectStore,
    batch: WriteBatch,
    healthy: &mut bool,
    signals: &mpsc::UnboundedSender<WriterSignal>,
    quarantine_dir: Option<&Path>,
    poison: &mut PoisonState,
) -> Result<u64, String> {
    let mut delay = RETRY_INITIAL;
    loop {
        match store.write(batch.clone()).await {
            Ok(receipt) => {
                if !*healthy {
                    *healthy = true;
                    tracing::info!(serial = receipt.serial, "store writes recovered");
                    notify_status(signals, true, String::new());
                }
                return Ok(receipt.serial);
            }
            Err(StoreError::Closed) => {
                // Shutdown already released the store; nothing to retry
                // against. (The orderly path drains this writer first, so
                // this only happens on abnormal teardown.)
                tracing::error!(serial = batch.serial, "store closed; batch dropped");
                return Err("store closed; batch dropped".to_string());
            }
            Err(e) if !e.is_transient() => {
                // Deterministic failure: every retry would fail identically,
                // wedging persistence behind one poisoned batch. Drop it,
                // scream, and keep the pipeline moving. Object upserts
                // self-heal (full snapshots re-sent on the next touch), but
                // dropped MODULE ops do NOT: a dropped `Del` resurrects its
                // document at next boot and a dropped `StreamAppend` is
                // simply gone — log the affected keys so an operator can
                // repair by hand.
                if !batch.module_ops.is_empty() {
                    let mut keys: Vec<String> = Vec::new();
                    let mut ledger_keys: Vec<String> = Vec::new();
                    for op in &batch.module_ops {
                        let key = match &op.op {
                            mush_store::ModuleWrite::JsonSet { key, .. }
                            | mush_store::ModuleWrite::Del { key }
                            | mush_store::ModuleWrite::StreamAppend { key, .. } => key,
                        };
                        let full = format!("mod:{}:{}", op.ns, key);
                        if mush_core::is_log_key(key) {
                            ledger_keys.push(full.clone());
                        }
                        keys.push(full);
                    }
                    tracing::error!(
                        serial = batch.serial,
                        ?keys,
                        "dropped batch carried module ops that will NOT self-heal \
                         (deletes may resurrect at next boot; stream appends are lost)"
                    );
                    if !ledger_keys.is_empty() {
                        // Authoritative-ledger breach: an open-chunk rewrite
                        // self-heals on the log's next append, but a dropped
                        // seal-and-roll-over op strands that chunk's entries —
                        // the in-RAM ledger has already advanced to the next
                        // chunk and will never re-persist the sealed one.
                        tracing::error!(
                            serial = batch.serial,
                            ?ledger_keys,
                            "DROPPED BATCH CARRIED AUTHORITATIVE LEDGER APPENDS — failing closed; \
                             isolating each sealed chunk to tell poison from collateral (E0)"
                        );
                    }
                }
                // E0 poison-seal posture: a deterministic drop of a batch
                // carrying sealed chunks means the ledger's durability is in
                // question. Fail closed immediately, then isolate each seal to
                // separate a true poison (quarantine + preserve) from a seal
                // merely caught as collateral (heals in isolation).
                let seals = sealed_chunks_in(&batch);
                if !seals.is_empty() {
                    handle_poison_seals(store, signals, quarantine_dir, batch.serial, &e, seals, poison)
                        .await;
                }
                tracing::error!(
                    serial = batch.serial, error = %e,
                    "store write failed with a non-retryable error; BATCH DROPPED \
                     (object docs are behind RAM until those objects are next written)"
                );
                return Err(format!("non-retryable store error: {e}"));
            }
            Err(e) => {
                tracing::error!(serial = batch.serial, error = %e, "store write failed; retrying");
                if *healthy {
                    *healthy = false;
                    notify_status(signals, false, e.to_string());
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Non-blocking send of a store-health transition on the unbounded signal
/// channel (E0). The writer must never block on this (see [`WriterSignal`]);
/// unbounded `send` returns immediately, and `Err` means the world is gone.
fn notify_status(signals: &mpsc::UnboundedSender<WriterSignal>, healthy: bool, detail: String) {
    if signals.send(WriterSignal::StoreStatus { healthy, detail }).is_err() {
        tracing::debug!("world task gone; store status transition dropped");
    }
}

/// Non-blocking signal of an authoritative-ledger breach to the world (E0).
/// `seal` names a specific isolated seal to drop from re-emit (quarantined poison
/// or healed collateral); `None` is the initial fail-closed freeze. Rides the
/// unbounded signal channel so it can never block the writer.
fn notify_ledger_breach(
    signals: &mpsc::UnboundedSender<WriterSignal>,
    detail: String,
    seal: Option<(SmolStr, SmolStr, bool)>,
) {
    if signals.send(WriterSignal::LedgerBreach { detail, seal }).is_err() {
        tracing::debug!("world task gone; ledger breach signal dropped");
    }
}

/// Writer-local bookkeeping for the poison-seal posture, persisted across jobs.
/// The world re-emits an unresolved seal on every flush until it processes the
/// resolution signal, so without this the writer would re-isolate, re-spool, and
/// re-freeze the *same* seal every cycle (a storm, and extra WorldMsg pressure).
#[derive(Default)]
struct PoisonState {
    /// Whether the fail-closed freeze has already been signaled this process
    /// life — the freeze never auto-lifts, so it is signaled exactly once.
    frozen_signaled: bool,
    /// Seals already isolated: value = whether durably resolved (quarantine
    /// spooled + signaled, or healed + signaled). A `false` entry is a confirmed
    /// poison whose spool write failed — re-attempt the spool (only), never
    /// re-isolate. A missing entry means "not yet seen; isolate it".
    handled: std::collections::HashMap<(SmolStr, SmolStr), bool>,
}

/// The sealed chunks a batch carried, as `(ns, key, value)` — the retention/
/// isolation unit, recognized purely by the op bytes ([`mush_core::is_sealed_chunk`]).
fn sealed_chunks_in(batch: &WriteBatch) -> Vec<(SmolStr, SmolStr, serde_json::Value)> {
    batch
        .module_ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value } if mush_core::is_sealed_chunk(key, value) => {
                Some((op.ns.clone(), key.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Handle the sealed chunks of a batch that a deterministic store rejection just
/// dropped (E0). Fail closed once, then — for each seal not already handled —
/// isolate it: a seal that commits **alone** was collateral (heal it); a seal
/// that fails alone [`POISON_ISOLATION_ATTEMPTS`] times is a deterministic poison
/// (durably quarantine it, then drop it from re-emit). A transient failure during
/// isolation is the store's fault, not the seal's — leave it for normal re-emit.
/// `poison` makes this idempotent across the world's re-emits of the same seal:
/// an already-resolved seal is skipped; a poison whose spool failed re-attempts
/// only the spool, never the whole isolation storm.
async fn handle_poison_seals(
    store: &dyn ObjectStore,
    signals: &mpsc::UnboundedSender<WriterSignal>,
    quarantine_dir: Option<&Path>,
    serial: u64,
    orig_err: &StoreError,
    seals: Vec<(SmolStr, SmolStr, serde_json::Value)>,
    poison: &mut PoisonState,
) {
    // Fail closed on the first confirmed deterministic ledger failure, exactly
    // once (the freeze never auto-lifts, so re-sending it every cycle is noise).
    if !poison.frozen_signaled {
        poison.frozen_signaled = true;
        notify_ledger_breach(
            signals,
            format!("deterministic drop of batch {serial}: {orig_err}"),
            None,
        );
    }

    for (ns, key, value) in seals {
        let id = (ns.clone(), key.clone());
        match poison.handled.get(&id) {
            // Already resolved (quarantined or healed + signaled): the world will
            // drop it from re-emit as soon as it processes that signal. Skip.
            Some(true) => continue,
            // Known poison whose spool write failed before: re-attempt ONLY the
            // spool (cheap), never the whole isolation. If it now lands, resolve.
            Some(false) => {
                if spool_quarantine(quarantine_dir, &ns, &key, &value, serial, orig_err, POISON_ISOLATION_ATTEMPTS) {
                    poison.handled.insert(id, true);
                    notify_ledger_breach(
                        signals,
                        format!("seal {ns}/{key} quarantined as poison (spool recovered)"),
                        Some((ns, key, true)),
                    );
                }
                continue;
            }
            // First encounter: isolate below.
            None => {}
        }

        let mut attempts: u32 = 0;
        loop {
            attempts += 1;
            let single = WriteBatch {
                // `serial` is informational here; an isolated write never
                // advances the commit high-water (that would clear sibling seals
                // sharing this serial). The world clears an isolated seal by its
                // (ns, key), via the breach signal below.
                serial,
                module_ops: vec![ModuleOp {
                    ns: ns.clone(),
                    op: ModuleWrite::JsonSet { key: key.clone(), value: value.clone() },
                }],
                ..WriteBatch::default()
            };
            match store.write(single).await {
                Ok(_) => {
                    tracing::warn!(
                        %ns, %key, attempts,
                        "ledger seal committed in isolation — collateral, not poison; healed"
                    );
                    poison.handled.insert(id, true);
                    notify_ledger_breach(
                        signals,
                        format!("seal {ns}/{key} healed in isolation"),
                        Some((ns, key, false)),
                    );
                    break;
                }
                Err(e2) if !e2.is_transient() => {
                    if attempts >= POISON_ISOLATION_ATTEMPTS {
                        let preserved =
                            spool_quarantine(quarantine_dir, &ns, &key, &value, serial, &e2, attempts);
                        if preserved {
                            tracing::error!(
                                %ns, %key, attempts,
                                "ledger seal is a confirmed deterministic POISON — quarantined to \
                                 the spool and dropped from re-emit; the world stays frozen until \
                                 staff repair and restart"
                            );
                            poison.handled.insert(id, true);
                            notify_ledger_breach(
                                signals,
                                format!("seal {ns}/{key} quarantined as poison"),
                                Some((ns, key, true)),
                            );
                        } else {
                            // Could not preserve it durably: NEVER discard. Keep
                            // it in the world's re-emit (do not signal quarantine)
                            // and keep the world frozen; remember it as poison so
                            // future re-emits only re-attempt the spool, never the
                            // whole isolation storm. Page for manual intervention.
                            tracing::error!(
                                %ns, %key,
                                "CRITICAL: a poison ledger seal could NOT be preserved in the \
                                 quarantine spool; it is NOT dropped from re-emit and the world \
                                 stays frozen — manual intervention required"
                            );
                            poison.handled.insert(id, false);
                        }
                        break;
                    }
                    // Same-fingerprint isolated retry: pure diagnosis, no backoff
                    // (a deterministic failure repeats identically and instantly).
                }
                Err(_transient) => {
                    tracing::warn!(
                        %ns, %key,
                        "transient store error while isolating a ledger seal; leaving it for \
                         normal re-emit once the store recovers"
                    );
                    // Do NOT record it: once the store recovers the normal re-emit
                    // commits it and the high-water clears it.
                    break;
                }
            }
        }
    }
}

/// Durably preserve a confirmed-poison sealed chunk in the quarantine spool — a
/// separate failure domain from the (possibly-damaged) store — so it is never
/// discarded and can be replayed after repair. Writes a full recovery record and
/// **fsyncs** it. Returns whether it was durably preserved.
///
/// With no spool directory configured (tests / the compat `spawn`), the full
/// payload is logged at critical severity and this reports success so the world
/// stops re-emitting a seal that can never commit — a degraded mode the real
/// server never uses (it always configures a spool under the data dir).
fn spool_quarantine(
    dir: Option<&Path>,
    ns: &str,
    key: &str,
    value: &serde_json::Value,
    serial: u64,
    err: &StoreError,
    attempts: u32,
) -> bool {
    let identity = seal_identity_hash(ns, key, value);
    let record = serde_json::json!({
        "ns": ns,
        "key": key,
        "serial": serial,
        "payload_hash": identity,
        "payload": value,
        "store_error": err.to_string(),
        "error_transient": err.is_transient(),
        "isolation_attempts": attempts,
        "quarantined_unix": unix_now_s(),
        "server_version": env!("CARGO_PKG_VERSION"),
    });

    let Some(dir) = dir else {
        tracing::error!(
            ns, key, serial, %record,
            "no quarantine spool configured — the poison seal is preserved ONLY in this log \
             line; the world drops it from re-emit to avoid a busy-loop (real deployments \
             always configure a spool)"
        );
        return true;
    };

    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::error!(dir = %dir.display(), error = %e, "could not create the quarantine spool dir");
        return false;
    }
    // Content-addressed filename: two quarantines of the SAME seal (same ns/key/
    // payload) map to the same file, so a re-quarantine is idempotent and can
    // never truncate a different record. A distinct seal hashes to a distinct
    // name — no collision from serial reuse or a non-injective sanitizer.
    let path = dir.join(format!("{identity}.json"));
    if path.exists() {
        // Already written (first write wins, keeping the original record). The
        // content was fsynced before its rename, but a PRIOR attempt may have
        // failed the directory fsync (rename ok, dir fsync EIO) — so re-confirm
        // the directory entry is durable before reporting preserved. Never
        // falsely report a record whose name could vanish on a crash.
        return match fsync_dir(dir) {
            Ok(()) => {
                tracing::error!(path = %path.display(), "poison ledger seal already quarantined (idempotent)");
                true
            }
            Err(e) => {
                tracing::error!(
                    path = %path.display(), error = %e,
                    "quarantine record exists but its directory entry is NOT durable (dir fsync \
                     failed) — keeping the seal frozen and re-emitting until it is"
                );
                false
            }
        };
    }
    match write_record_atomic(dir, &path, &identity, &record) {
        Ok(()) => {
            tracing::error!(path = %path.display(), "poison ledger seal quarantined to the spool");
            true
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "FAILED to durably write the quarantine spool record");
            false
        }
    }
}

/// Durably write a quarantine record: serialize to a temp file, fsync its
/// contents, atomically rename it onto the content-addressed name, and fsync the
/// directory so the new name survives a crash. Crash-atomic — a crash mid-write
/// leaves only a stale `.tmp` (overwritten next time), never a partial record
/// under the real name.
fn write_record_atomic(
    dir: &Path,
    final_path: &Path,
    identity: &str,
    record: &serde_json::Value,
) -> std::io::Result<()> {
    use std::io::Write as _;
    let bytes = serde_json::to_vec_pretty(record).map_err(std::io::Error::other)?;
    // Single writer, so a fixed temp name is safe; truncating a stale temp from a
    // prior crash is harmless (temps are disposable).
    let tmp = dir.join(format!("{identity}.tmp"));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, final_path)?;
    fsync_dir(dir)?;
    Ok(())
}

/// fsync a directory so a just-created/renamed entry's name is durable. On the
/// Linux deploy target this is **mandatory** (a failure fails the write, so the
/// caller keeps the seal frozen + re-emitting rather than falsely reporting it
/// preserved). Non-Unix platforms cannot open a directory as a file, so it is a
/// no-op there (dev/test only).
#[cfg(unix)]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}
#[cfg(not(unix))]
fn fsync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// A stable, collision-resistant identity for a sealed chunk (its `ns`, key, and
/// canonical payload bytes) — SHA-256 hex. Used for the content-addressed spool
/// filename and recorded so staff can confirm a replayed op matches. serde_json
/// object key order is stable (BTreeMap, `preserve_order` off), so the same seal
/// hashes identically across runs.
fn seal_identity_hash(ns: &str, key: &str, value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(ns.as_bytes());
    h.update([0u8]);
    h.update(key.as_bytes());
    h.update([0u8]);
    h.update(serde_json::to_vec(value).unwrap_or_default());
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Count the unresolved quarantine records in the spool (`*.json`, ignoring
/// in-flight `*.tmp`). A non-zero count at boot means an authoritative-ledger
/// poison has NOT been repaired: the world must start frozen (E0 never
/// auto-resumes). `None` (no spool) and an unreadable dir both yield 0.
#[must_use]
pub fn spool_unresolved_count(dir: Option<&Path>) -> usize {
    let Some(dir) = dir else { return 0 };
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count()
}

/// Snapshot one drained dirty set into a [`WriteBatch`]: live objects are
/// cloned as full-object upserts, tombstoned (or never-live) ids become
/// deletes, and `next_id` rides along so the id allocator resumes exactly.
#[must_use]
pub fn batch_from_dirty(
    world: &World,
    dirty: BTreeSet<ObjId>,
    serial: u64,
    durable: bool,
) -> WriteBatch {
    let mut batch = WriteBatch {
        serial,
        durable,
        ..WriteBatch::default()
    };
    for id in dirty {
        match world.get(id) {
            Some(obj) => batch.upserts.push(obj.clone()),
            None => batch.deletes.push(id),
        }
    }
    if !batch.is_empty() {
        batch.next_id = Some(world.next_id());
    }
    batch
}

/// The largest tolerated gap between `meta:next_id` and the number of
/// stored objects. Tombstones make `next_id > object count` normal, but a
/// corrupt `next_id` (say 4e9) would make `World::from_objects` allocate a
/// slot per id before any other validation ran — hundreds of GB. A million
/// tombstones beyond the live set is already absurd for a MUSH; past that,
/// the key is corrupt and boot must refuse before allocating.
const MAX_TOMBSTONE_GAP: usize = 1_000_000;

/// Boot-time world hydration. A populated store loads via
/// [`World::from_objects`]; a completely empty one is seeded with the
/// minimal world and the seed batch is **written through (and acked)**
/// before this returns, so the first boot persists without waiting for the
/// first command. Either way the returned world carries an empty dirty set
/// and the current wall clock.
///
/// Fails fast (naming any corrupt docs the load skipped, so the operator
/// can connect cause to effect) when:
/// - `meta:next_id` is implausibly far beyond the stored object count
///   ([`MAX_TOMBSTONE_GAP`]) — checked **before** hydration allocates;
/// - hydration itself rejects the data (dangling references, duplicate
///   ids, name collisions);
/// - the hydrated world is missing Limbo `#0` or God `#1` — without them
///   the server would boot an unusable husk (every create fails, every
///   login targets a missing start room). This happens when the seed docs
///   themselves were corrupt/skipped but `meta:next_id` survived.
pub async fn load_or_seed_world(store: &dyn ObjectStore) -> Result<World, String> {
    let world = match store
        .load_world()
        .await
        .map_err(|e| format!("loading world from store: {e}"))?
    {
        Some(snapshot) => {
            let count = snapshot.objects.len();
            let corrupt_note = if snapshot.corrupt_docs > 0 {
                format!(
                    " ({} corrupt object doc(s) were skipped during the load — \
                     likely the cause; see the log)",
                    snapshot.corrupt_docs
                )
            } else {
                String::new()
            };
            if (snapshot.next_id as usize).saturating_sub(count) > MAX_TOMBSTONE_GAP {
                return Err(format!(
                    "persisted next_id {} is implausibly far beyond the {count} stored \
                     object(s); refusing to allocate — meta:next_id looks corrupt{corrupt_note}",
                    snapshot.next_id
                ));
            }
            let world = World::from_objects(snapshot.objects, snapshot.next_id)
                .map_err(|e| format!("persisted world is inconsistent: {e}{corrupt_note}"))?;
            for (id, name) in [(mush_core::LIMBO, "Limbo"), (mush_core::GOD, "God")] {
                if world.get(id).is_none() {
                    return Err(format!(
                        "persisted world is missing {name} ({id}); refusing to boot an \
                         unusable world{corrupt_note}"
                    ));
                }
            }
            tracing::info!(
                objects = count,
                next_id = snapshot.next_id,
                "world loaded from store"
            );
            world
        }
        None => {
            let mut world = World::new_minimal();
            let dirty = world.take_dirty();
            let seed = batch_from_dirty(&world, dirty, 0, true);
            store
                .write(seed)
                .await
                .map_err(|e| format!("writing world seed batch: {e}"))?;
            tracing::info!("empty store: seeded minimal world (Limbo #0, God #1)");
            world
        }
    };
    let mut world = world;
    world.set_now(unix_now_s());
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO, ObjKind};
    use mush_store::InMemoryStore;

    #[test]
    fn seal_identity_hash_is_stable_and_distinguishes_seals() {
        let v = serde_json::json!({"chunk": 0, "sealed": true, "entries": []});
        let a = seal_identity_hash("scenes", "log:s:0", &v);
        // Stable across calls (content-addressed).
        assert_eq!(a, seal_identity_hash("scenes", "log:s:0", &v));
        // Distinct on ns, key, or payload — no collision from a non-injective
        // sanitizer (log:s:0 vs log_s_0 differ, as do different namespaces).
        assert_ne!(a, seal_identity_hash("scenes", "log_s_0", &v));
        assert_ne!(a, seal_identity_hash("econ", "log:s:0", &v));
        assert_ne!(a, seal_identity_hash("scenes", "log:s:0", &serde_json::json!({"chunk": 1})));
        assert_eq!(a.len(), 64, "sha-256 hex");
    }

    #[test]
    fn spool_unresolved_count_ignores_tmp_and_missing() {
        assert_eq!(spool_unresolved_count(None), 0);
        let dir = std::env::temp_dir().join(format!("rustmush-spool-count-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(spool_unresolved_count(Some(&dir)), 0, "a missing dir is 0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("abc.json"), b"{}").unwrap();
        std::fs::write(dir.join("def.json"), b"{}").unwrap();
        std::fs::write(dir.join("wip.tmp"), b"{}").unwrap(); // in-flight, not counted
        assert_eq!(spool_unresolved_count(Some(&dir)), 2, "counts *.json, ignores *.tmp");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_snapshots_upserts_deletes_and_next_id() {
        let mut w = World::new_minimal();
        let _ = w.take_dirty();
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO)).unwrap();
        let doomed = w
            .create(ObjKind::Thing, "doomed", GOD, Some(LIMBO))
            .unwrap();
        w.destroy(doomed).unwrap();
        w.sweep();

        let dirty = w.take_dirty();
        let batch = batch_from_dirty(&w, dirty, 7, false);
        assert_eq!(batch.serial, 7);
        assert!(!batch.durable);
        let upserted: Vec<ObjId> = batch.upserts.iter().map(|o| o.id).collect();
        assert!(upserted.contains(&ball) && upserted.contains(&LIMBO));
        assert_eq!(batch.deletes, vec![doomed]);
        assert_eq!(batch.next_id, Some(w.next_id()));

        // Nothing dirty -> an empty batch with no next_id.
        let dirty = w.take_dirty();
        let empty = batch_from_dirty(&w, dirty, 8, false);
        assert!(empty.is_empty());
        assert_eq!(empty.next_id, None);
    }

    #[tokio::test]
    async fn boot_refuses_world_missing_limbo_or_god() {
        // A store that has data (so it is not seeded fresh) but whose seed
        // docs are gone — e.g. every doc corrupt/skipped except one thing —
        // must refuse to boot rather than serve an unusable husk.
        let store = InMemoryStore::new();
        let mut orphan = mush_core::Obj::new(ObjId(5), ObjKind::Thing, "orphan", ObjId(5), 0);
        orphan.location = None;
        store
            .write(mush_store::WriteBatch {
                serial: 1,
                upserts: vec![orphan],
                next_id: Some(6),
                durable: true,
                ..mush_store::WriteBatch::default()
            })
            .await
            .unwrap();
        let err = load_or_seed_world(&store).await.unwrap_err();
        assert!(
            err.contains("Limbo") || err.contains("God"),
            "error must name the missing anchor object: {err}"
        );
    }

    #[tokio::test]
    async fn boot_refuses_implausible_next_id_before_allocating() {
        // A corrupt meta:next_id must be refused BEFORE hydration tries to
        // allocate a slot per id (a 4e9 next_id would be hundreds of GB).
        let store = InMemoryStore::new();
        let world = World::new_minimal();
        let objects: Vec<_> = world.iter().cloned().collect();
        store
            .write(mush_store::WriteBatch {
                serial: 1,
                upserts: objects,
                next_id: Some(50_000_000),
                durable: true,
                ..mush_store::WriteBatch::default()
            })
            .await
            .unwrap();
        let err = load_or_seed_world(&store).await.unwrap_err();
        assert!(
            err.contains("next_id") && err.contains("corrupt"),
            "error must blame meta:next_id: {err}"
        );
    }

    #[tokio::test]
    async fn seed_writes_through_and_reloads() {
        let store = InMemoryStore::new();
        let seeded = load_or_seed_world(&store).await.unwrap();
        assert!(seeded.get(GOD).is_some() && seeded.get(LIMBO).is_some());

        // The seed batch really landed: a second boot loads it back.
        let reloaded = load_or_seed_world(&store).await.unwrap();
        assert_eq!(reloaded.next_id(), seeded.next_id());
        assert_eq!(reloaded.get(GOD).unwrap().name, "God");
        assert_eq!(reloaded.get(LIMBO).unwrap().name, "Limbo");
    }
}
