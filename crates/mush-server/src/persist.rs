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
//!   ([`crate::world_task::WriterSignal::StoreStatus`]) until the store recovers
//!   (PLAN risk 3). A
//!   *deterministic* failure (serialization, a type-clobbered key) would fail
//!   identically forever, so that batch is diagnosed/quarantined and the writer
//!   latches halted. Every later queued write receives `Err` without touching
//!   the store: committing past an unresolved predecessor would break FIFO
//!   durability and compound RAM/store divergence. Recovery happens from exact
//!   retained/quarantine material after restart, not speculative later writes.
//!
//! Durability-critical batches (`durable: true`, e.g. Player creation)
//! carry a receipt oneshot; the caller gates its confirmation output on it,
//! and a dropped batch reports `Err` — never a false success. `@dump` rides
//! the same queue as [`StoreJob::Bgsave`] so the snapshot is ordered behind
//! every batch queued before it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mush_config::metrics::Metrics;
use mush_core::{ObjId, World};
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, StoreError, WorldSnapshot, WriteBatch};
use smol_str::SmolStr;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::quarantine::QuarantineSpool;
use crate::world_task::WriterSignalSender;
use mush_config::unix_now_s;

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

/// A confirmed poison seal is allowed to leave RAM only after the independent
/// quarantine sink has durably verified it. Runtime I/O failures therefore
/// retry in place with bounded backoff while the world is frozen; returning to
/// the ordinary writer loop would make the retry unreachable behind the
/// integrity halt and a process crash could then erase the sole copy.
const QUARANTINE_RETRY_INITIAL: Duration = Duration::from_millis(100);
const QUARANTINE_RETRY_MAX: Duration = Duration::from_secs(5);

/// Process-local, monotonic persistence-integrity latch shared by the world,
/// writer, and shutdown coordinator.
///
/// This is deliberately independent of the asynchronously propagated
/// [`crate::world_task::WriterSignal`]: once either side observes a breach, ordinary snapshots
/// fail closed immediately and for the rest of this process lifetime.
#[derive(Clone, Debug, Default)]
pub struct PersistenceIntegrityLatch {
    latched: Arc<AtomicBool>,
}

impl PersistenceIntegrityLatch {
    /// Construct a healthy, not-yet-latched state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a persistence-integrity breach has occurred this process life.
    #[must_use]
    pub fn is_latched(&self) -> bool {
        self.latched.load(Ordering::Acquire)
    }

    pub(crate) fn latch(&self) {
        self.latched.store(true, Ordering::Release);
    }
}

/// One unit of work for the store-writer task.
pub enum StoreJob {
    /// Apply one batch, acknowledged in FIFO order. Transient failures retry;
    /// a deterministic rejection latches the writer and every later write is
    /// refused without touching the store.
    Write {
        /// The batch.
        batch: WriteBatch,
        /// Receipt for ack-gated flows (durable batches); `None` for
        /// fire-and-forget ordinary command batches.
        notify: Option<oneshot::Sender<Result<u64, String>>>,
    },
    /// Trigger an ordinary background snapshot (`@dump`), ordered behind
    /// everything queued before it and refused after an integrity breach.
    ///
    /// A future forensic export must be a separate operation with a separate
    /// destination and evidence-preservation contract. Ordinary Redis BGSAVE
    /// must never be relabeled as forensic output.
    Bgsave {
        /// Resolved when the store acknowledged the BGSAVE request.
        done: oneshot::Sender<Result<(), String>>,
    },
}

/// Spawn the store-writer task. `signals` is the dedicated bounded/coalescing
/// mailbox where the writer publishes health transitions
/// ([`crate::world_task::WriterSignal::StoreStatus`]) and ledger breaches. Publishing takes a
/// short synchronous lock and arms a capacity-one wake token; it never awaits
/// the world or deadlocks against job-queue back-pressure. The task drains its
/// queue and exits when every job sender is dropped.
pub fn spawn_store_writer(
    store: Arc<dyn ObjectStore>,
    signals: WriterSignalSender,
    metrics: Arc<Metrics>,
    high_water: Arc<AtomicU64>,
    quarantine_spool: QuarantineSpool,
    integrity: PersistenceIntegrityLatch,
) -> (mpsc::Sender<StoreJob>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(WRITER_QUEUE);
    let join = tokio::spawn(writer_loop(
        store,
        rx,
        signals,
        metrics,
        high_water,
        quarantine_spool,
        integrity,
    ));
    (tx, join)
}

async fn writer_loop(
    store: Arc<dyn ObjectStore>,
    mut rx: mpsc::Receiver<StoreJob>,
    signals: WriterSignalSender,
    metrics: Arc<Metrics>,
    high_water: Arc<AtomicU64>,
    quarantine_spool: QuarantineSpool,
    integrity: PersistenceIntegrityLatch,
) {
    tracing::info!("store writer running");
    let mut healthy = true;
    // E0 poison-seal bookkeeping, persisted across jobs so a re-emitted seal is
    // recognized rather than re-isolated/re-spooled/re-frozen every cycle.
    let mut poison = PoisonState::default();
    // Once a batch leaves RAM ahead of durable truth, no later write may commit
    // past it. Drain the bounded queue so every receipt completes, but fail
    // those jobs locally until an explicit recovery/restart builds a fresh
    // writer. Ordinary BGSAVE is refused too: snapshotting known-ambiguous
    // durable state would turn the breach into a misleading recovery point.
    let mut integrity_halt: Option<String> = None;
    while let Some(job) = rx.recv().await {
        match job {
            StoreJob::Write { batch, notify } => {
                let batch_serial = batch.serial;
                let result = if let Some(reason) = &integrity_halt {
                    integrity.latch();
                    tracing::error!(
                        serial = batch_serial,
                        halt_reason = reason,
                        "store writer integrity-halted; rejecting queued write without touching store"
                    );
                    let detail = format!(
                        "persistence integrity halted after an unresolved predecessor: {reason}"
                    );
                    preserve_terminal_seals(
                        &signals,
                        &quarantine_spool,
                        batch_serial,
                        &StoreError::Invalid(detail.clone()),
                        sealed_chunks_in(&batch),
                        &mut poison,
                    )
                    .await;
                    Err(detail)
                } else {
                    let result = write_with_retry(
                        &*store,
                        batch,
                        &mut healthy,
                        &signals,
                        &quarantine_spool,
                        &mut poison,
                        &integrity,
                    )
                    .await;
                    if let Err(detail) = &result {
                        // Defensive backstop: every terminal result must make
                        // the process-wide snapshot barrier visible before the
                        // writer considers another queued job.
                        integrity.latch();
                        let reason = bounded_detail(format!(
                            "batch {batch_serial} was not committed: {detail}"
                        ));
                        tracing::error!(
                            serial = batch_serial,
                            %reason,
                            "store writer latched integrity halt; later writes will be refused"
                        );
                        integrity_halt = Some(reason);
                    }
                    result
                };
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
                let result = if integrity.is_latched() {
                    Err("persistence integrity is latched; ordinary BGSAVE refused".to_owned())
                } else {
                    store.bgsave().await.map_err(|e| e.to_string())
                };
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
/// wedge every batch queued behind it. The caller latches a writer halt on any
/// `Err`; later batches complete with `Err` without touching the store, so
/// nothing can commit out of FIFO order past this unresolved predecessor.
async fn write_with_retry(
    store: &dyn ObjectStore,
    batch: WriteBatch,
    healthy: &mut bool,
    signals: &WriterSignalSender,
    quarantine_spool: &QuarantineSpool,
    poison: &mut PoisonState,
    integrity: &PersistenceIntegrityLatch,
) -> Result<u64, String> {
    let mut delay = RETRY_INITIAL;
    loop {
        match store.write(batch.clone()).await {
            Ok(receipt) => {
                if receipt.serial != batch.serial {
                    // Publish the fail-closed fact before preserving seals:
                    // quarantine I/O may retry for longer than the shutdown
                    // grace period, and shutdown must already know not to
                    // snapshot the ambiguous store.
                    integrity.latch();
                    let detail = format!(
                        "store acknowledged batch {} with mismatched receipt serial {}",
                        batch.serial, receipt.serial
                    );
                    tracing::error!(
                        expected = batch.serial,
                        received = receipt.serial,
                        "store violated exact write-receipt identity"
                    );
                    notify_deterministic_failure(signals, detail.clone());
                    let terminal = StoreError::Invalid(detail.clone());
                    preserve_terminal_seals(
                        signals,
                        quarantine_spool,
                        batch.serial,
                        &terminal,
                        sealed_chunks_in(&batch),
                        poison,
                    )
                    .await;
                    return Err(detail);
                }
                if !*healthy {
                    *healthy = true;
                    tracing::info!(serial = receipt.serial, "store writes recovered");
                    notify_status(signals, true, String::new());
                }
                return Ok(receipt.serial);
            }
            Err(StoreError::Closed) => {
                // The acknowledgement is now unknowable. Latch before any
                // potentially long preservation work so shutdown cannot race
                // ahead and produce an ordinary snapshot.
                integrity.latch();
                // Shutdown already released the store; nothing to retry
                // against. (The orderly path drains this writer first, so
                // this only happens on abnormal teardown.)
                tracing::error!(serial = batch.serial, "store closed; batch dropped");
                notify_deterministic_failure(
                    signals,
                    format!("store closed while dropping batch {}", batch.serial),
                );
                preserve_terminal_seals(
                    signals,
                    quarantine_spool,
                    batch.serial,
                    &StoreError::Closed,
                    sealed_chunks_in(&batch),
                    poison,
                )
                .await;
                return Err("store closed; batch dropped".to_string());
            }
            Err(e) if !e.is_transient() => {
                // A deterministic drop makes RAM/durable truth diverge. Make
                // that fact synchronously visible to the writer and shutdown
                // coordinator before diagnosis or quarantine can block.
                integrity.latch();
                // Deterministic failure: every retry would fail identically.
                // Diagnose and return it; the writer loop latches halted before
                // considering any later queued write. Dropped module ops need
                // exact recovery: a dropped `Del` could resurrect its document
                // and a dropped `StreamAppend` cannot be reconstructed from a
                // later object snapshot.
                notify_deterministic_failure(
                    signals,
                    format!("deterministic drop of batch {}: {e}", batch.serial),
                );
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
                    handle_poison_seals(
                        store,
                        signals,
                        quarantine_spool,
                        batch.serial,
                        &e,
                        seals,
                        poison,
                    )
                    .await;
                }
                tracing::error!(
                    serial = batch.serial, error = %e,
                    "store write failed with a non-retryable error; BATCH DROPPED \
                     (writer will reject all later writes until recovery/restart)"
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

/// Non-blocking publication of a coalesced store-health transition (E0).
fn notify_status(signals: &WriterSignalSender, healthy: bool, detail: String) {
    signals.store_status(healthy, detail);
}

/// A deterministic dropped batch means RAM may be ahead of durable truth even
/// when no authoritative ledger seal happened to ride in that batch. Latch the
/// same non-auto-lifting integrity barrier before the writer finishes the job.
fn notify_deterministic_failure(signals: &WriterSignalSender, detail: String) {
    signals.deterministic_failure(bounded_detail(detail));
}

fn bounded_detail(mut detail: String) -> String {
    const MAX_DETAIL_BYTES: usize = 4 * 1024;
    if detail.len() > MAX_DETAIL_BYTES {
        let mut boundary = MAX_DETAIL_BYTES;
        while !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        detail.truncate(boundary);
    }
    detail
}

/// Non-blocking signal of an authoritative-ledger breach to the world (E0).
/// `seal` names a specific isolated seal to drop from re-emit (quarantined poison
/// or healed collateral); `None` is the initial fail-closed freeze. Rides the
/// coalescing mailbox so it can never block the writer. `false` means an exact
/// resolution did not fit; the caller must retain its identity and republish it
/// when the corresponding seal is re-emitted.
#[must_use]
fn notify_ledger_breach(
    signals: &WriterSignalSender,
    detail: String,
    seal: Option<(SmolStr, SmolStr, String, bool)>,
) -> bool {
    signals.ledger_breach(detail, seal)
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
    /// Seals already isolated and durably resolved (quarantine spooled or
    /// healed). A missing entry means "not yet seen; isolate it". The outcome
    /// remains in this bounded map so every retained re-emit republishes an
    /// exact resolution if the coalescing mailbox was temporarily saturated.
    handled: std::collections::HashMap<(SmolStr, SmolStr, String), SealResolutionKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SealResolutionKind {
    Healed,
    Quarantined,
}

impl SealResolutionKind {
    const fn quarantined(self) -> bool {
        matches!(self, SealResolutionKind::Quarantined)
    }
}

fn remember_resolution(
    poison: &mut PoisonState,
    id: &(SmolStr, SmolStr, String),
    kind: SealResolutionKind,
) -> bool {
    if let Some(existing) = poison.handled.get_mut(id) {
        if *existing != kind {
            // A quarantine record is the conservative terminal truth if two
            // recovery paths ever disagree about one exact payload.
            *existing = SealResolutionKind::Quarantined;
        }
        return true;
    }
    if poison.handled.len() >= crate::retained::MAX_RETAINED_SEALS {
        // Production cannot reach this without the world violating the same
        // custody bound. Do not evict an older exact identity: the current
        // payload remains in world retention and will be retried/re-spooled.
        tracing::error!(
            ns = %id.0,
            key = %id.1,
            "writer exact-resolution custody exhausted; leaving seal retained for retry"
        );
        return false;
    }
    poison.handled.insert(id.clone(), kind);
    true
}

fn publish_resolution(
    signals: &WriterSignalSender,
    detail: String,
    id: &(SmolStr, SmolStr, String),
    kind: SealResolutionKind,
) -> bool {
    let accepted = notify_ledger_breach(
        signals,
        detail,
        Some((id.0.clone(), id.1.clone(), id.2.clone(), kind.quarantined())),
    );
    if !accepted {
        tracing::error!(
            ns = %id.0,
            key = %id.1,
            "exact seal resolution did not fit mailbox; retaining identity for re-emit retry"
        );
    }
    accepted
}

/// The sealed chunks a batch carried, as `(ns, key, value)` — the retention/
/// isolation unit, recognized purely by the op bytes ([`mush_core::is_sealed_chunk`]).
fn sealed_chunks_in(batch: &WriteBatch) -> Vec<(SmolStr, SmolStr, serde_json::Value)> {
    batch
        .module_ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value }
                if !op.ns.contains(':')
                    && op.ns.len() <= mush_core::MAX_MODULE_NAMESPACE_BYTES
                    && mush_core::is_sealed_chunk(key, value) =>
            {
                Some((op.ns.clone(), key.clone(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Preserve every authoritative seal in a batch whose acknowledgement can no
/// longer be trusted (closed store, wrong receipt identity, or another terminal
/// transport invariant). These are not diagnosed as poison: the quarantine
/// record is an exact unresolved spill, and writable boot remains blocked until
/// recovery determines whether the store also received the bytes.
async fn preserve_terminal_seals(
    signals: &WriterSignalSender,
    quarantine_spool: &QuarantineSpool,
    serial: u64,
    terminal_error: &StoreError,
    seals: Vec<(SmolStr, SmolStr, serde_json::Value)>,
    poison: &mut PoisonState,
) {
    if seals.is_empty() {
        return;
    }
    if !poison.frozen_signaled {
        poison.frozen_signaled = true;
        let _accepted = notify_ledger_breach(
            signals,
            format!(
                "untrusted acknowledgement for batch {serial}: {terminal_error}; preserving exact seals"
            ),
            None,
        );
    }
    for (ns, key, value) in seals {
        let id = (
            ns.clone(),
            key.clone(),
            crate::quarantine::seal_identity_hash(&ns, &key, &value),
        );
        if let Some(kind) = poison.handled.get(&id).copied() {
            let _accepted = publish_resolution(
                signals,
                format!("seal {ns}/{key} resolution republished after retained re-emit"),
                &id,
                kind,
            );
            continue;
        }
        spool_quarantine_until_preserved(
            quarantine_spool,
            &ns,
            &key,
            &value,
            serial,
            terminal_error,
            0,
        )
        .await;
        let _remembered = remember_resolution(poison, &id, SealResolutionKind::Quarantined);
        let _accepted = publish_resolution(
            signals,
            format!("seal {ns}/{key} preserved after an untrusted acknowledgement"),
            &id,
            SealResolutionKind::Quarantined,
        );
    }
}

/// Handle the sealed chunks of a batch that a deterministic store rejection just
/// dropped (E0). Fail closed once, then — for each seal not already handled —
/// isolate it: a seal that commits **alone** was collateral (heal it); a seal
/// that fails alone [`POISON_ISOLATION_ATTEMPTS`] times is a deterministic poison
/// (durably quarantine it, then drop it from re-emit). A transient failure during
/// isolation is the store's fault, not the seal's — leave it for normal re-emit.
/// `poison` makes this idempotent across the world's re-emits of the same seal:
/// an already-resolved seal is skipped. A poison whose spool is temporarily
/// unavailable never returns to ordinary processing; it retries preservation
/// in place while the bounded writer queue applies backpressure.
async fn handle_poison_seals(
    store: &dyn ObjectStore,
    signals: &WriterSignalSender,
    quarantine_spool: &QuarantineSpool,
    serial: u64,
    orig_err: &StoreError,
    seals: Vec<(SmolStr, SmolStr, serde_json::Value)>,
    poison: &mut PoisonState,
) {
    // Fail closed on the first confirmed deterministic ledger failure, exactly
    // once (the freeze never auto-lifts, so re-sending it every cycle is noise).
    if !poison.frozen_signaled {
        poison.frozen_signaled = true;
        let _accepted = notify_ledger_breach(
            signals,
            format!("deterministic drop of batch {serial}: {orig_err}"),
            None,
        );
    }

    for (ns, key, value) in seals {
        // A key alone is not a recovery identity: if corrupt/malicious input
        // presents different bytes under an already-handled sealed key, those
        // bytes must be isolated and preserved independently rather than
        // inheriting the earlier payload's resolution.
        let id = (
            ns.clone(),
            key.clone(),
            crate::quarantine::seal_identity_hash(&ns, &key, &value),
        );
        if let Some(kind) = poison.handled.get(&id).copied() {
            // The world retained this exact payload because it has not observed
            // the terminal resolution yet. Republish on every re-emit; this is
            // the lossless retry path for a briefly saturated mailbox.
            let _accepted = publish_resolution(
                signals,
                format!("seal {ns}/{key} resolution republished after retained re-emit"),
                &id,
                kind,
            );
            continue;
        }

        let mut attempts: u32 = 0;
        let mut deterministic_attempts: u32 = 0;
        let mut deterministic_fingerprint: Option<String> = None;
        let mut transient_delay = RETRY_INITIAL;
        loop {
            attempts = attempts.saturating_add(1);
            let single = WriteBatch {
                // `serial` is informational here; an isolated write never
                // advances the commit high-water (that would clear sibling seals
                // sharing this serial). The world clears an isolated seal by its
                // (ns, key), via the breach signal below.
                serial,
                module_ops: vec![ModuleOp {
                    ns: ns.clone(),
                    op: ModuleWrite::JsonSet {
                        key: key.clone(),
                        value: value.clone(),
                    },
                }],
                ..WriteBatch::default()
            };
            match store.write(single).await {
                Ok(receipt) if receipt.serial == serial => {
                    tracing::warn!(
                        %ns, %key, attempts,
                        "ledger seal committed in isolation — collateral, not poison; healed"
                    );
                    let _remembered = remember_resolution(poison, &id, SealResolutionKind::Healed);
                    let _accepted = publish_resolution(
                        signals,
                        format!("seal {ns}/{key} healed in isolation"),
                        &id,
                        SealResolutionKind::Healed,
                    );
                    break;
                }
                Ok(receipt) => {
                    let detail = format!(
                        "isolated seal {ns}/{key} expected receipt serial {serial}, got {}",
                        receipt.serial
                    );
                    tracing::error!(
                        %ns,
                        %key,
                        expected = serial,
                        received = receipt.serial,
                        "isolated seal returned a mismatched receipt; leaving recovery material retained"
                    );
                    notify_deterministic_failure(signals, detail);
                    // A receipt for a different identity proves neither commit
                    // nor poison. Spill the exact bytes as unresolved recovery
                    // material before the outer failed predecessor halts.
                    let terminal = StoreError::Invalid(format!(
                        "isolated seal receipt mismatch: expected {serial}, got {}",
                        receipt.serial
                    ));
                    preserve_terminal_seals(
                        signals,
                        quarantine_spool,
                        serial,
                        &terminal,
                        vec![(ns.clone(), key.clone(), value.clone())],
                        poison,
                    )
                    .await;
                    break;
                }
                Err(e2) if !e2.is_transient() => {
                    let fingerprint = bounded_detail(e2.to_string());
                    match deterministic_fingerprint.as_ref() {
                        None => {
                            deterministic_fingerprint = Some(fingerprint);
                            deterministic_attempts = 1;
                        }
                        Some(previous) if previous == &fingerprint => {
                            deterministic_attempts = deterministic_attempts.saturating_add(1);
                        }
                        Some(previous) => {
                            tracing::error!(
                                %ns,
                                %key,
                                previous,
                                current = %fingerprint,
                                "isolated seal failure fingerprint changed; preserving as unresolved rather than declaring poison"
                            );
                            spool_quarantine_until_preserved(
                                quarantine_spool,
                                &ns,
                                &key,
                                &value,
                                serial,
                                &e2,
                                attempts,
                            )
                            .await;
                            let _remembered =
                                remember_resolution(poison, &id, SealResolutionKind::Quarantined);
                            let _accepted = publish_resolution(
                                signals,
                                format!(
                                    "seal {ns}/{key} preserved after ambiguous isolation failures"
                                ),
                                &id,
                                SealResolutionKind::Quarantined,
                            );
                            break;
                        }
                    }
                    if deterministic_attempts >= POISON_ISOLATION_ATTEMPTS {
                        spool_quarantine_until_preserved(
                            quarantine_spool,
                            &ns,
                            &key,
                            &value,
                            serial,
                            &e2,
                            deterministic_attempts,
                        )
                        .await;
                        tracing::error!(
                            %ns, %key, attempts,
                            "ledger seal is a confirmed deterministic POISON — quarantined to \
                             the spool and dropped from re-emit; the world stays frozen until \
                             recovery verifies a repair"
                        );
                        let _remembered =
                            remember_resolution(poison, &id, SealResolutionKind::Quarantined);
                        let _accepted = publish_resolution(
                            signals,
                            format!("seal {ns}/{key} quarantined as poison"),
                            &id,
                            SealResolutionKind::Quarantined,
                        );
                        break;
                    }
                    // Same-fingerprint isolated retry: pure diagnosis, no backoff
                    // (a deterministic failure repeats identically and instantly).
                }
                Err(_transient) => {
                    tracing::warn!(
                        %ns, %key,
                        retry_ms = u64::try_from(transient_delay.as_millis()).unwrap_or(u64::MAX),
                        "transient store error while isolating a ledger seal; retaining it inside \
                         the fail-stopped writer and retrying after backoff"
                    );
                    // Returning here would make recovery unreachable: the
                    // outer writer latches its integrity halt and rejects every
                    // later ordinary re-emit. Keep custody until either exact
                    // commit or durable spill.
                    deterministic_fingerprint = None;
                    deterministic_attempts = 0;
                    tokio::time::sleep(transient_delay).await;
                    transient_delay = (transient_delay * 2).min(RETRY_MAX);
                }
            }
        }
    }
}

/// Durably preserve a confirmed-poison sealed chunk in the required quarantine
/// spool. Production cannot construct the writer without an opened durable
/// implementation; tests opt into an explicit in-memory implementation.
fn spool_quarantine(
    spool: &QuarantineSpool,
    ns: &str,
    key: &str,
    value: &serde_json::Value,
    serial: u64,
    err: &StoreError,
    attempts: u32,
) -> bool {
    match spool.quarantine(ns, key, value, serial, err, attempts) {
        Ok(true) => {
            tracing::error!(
                ns,
                key,
                "poison ledger seal quarantined to the required spool"
            );
            true
        }
        Ok(false) => false,
        Err(error) => {
            tracing::error!(ns, key, %error, "FAILED to preserve poison ledger seal in quarantine spool");
            false
        }
    }
}

/// Do not release the only known-good bytes of an authoritative seal while its
/// independent recovery sink is unavailable. The store writer stays inside
/// this loop (and has already frozen the world), which bounds queued work and
/// lets an operator repair the spool without restarting the process that still
/// owns the payload.
async fn spool_quarantine_until_preserved(
    spool: &QuarantineSpool,
    ns: &str,
    key: &str,
    value: &serde_json::Value,
    serial: u64,
    err: &StoreError,
    attempts: u32,
) {
    let mut delay = QUARANTINE_RETRY_INITIAL;
    loop {
        if spool_quarantine(spool, ns, key, value, serial, err, attempts) {
            return;
        }
        tracing::error!(
            ns,
            key,
            retry_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            "CRITICAL: poison seal is still RAM-only because quarantine preservation failed; \
             keeping the writer fail-stopped and retrying"
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(QUARANTINE_RETRY_MAX);
    }
}

/// Durably spill every valid sealed-ledger operation from a batch that never
/// entered the writer's custody (retention conflict or closed queue). This is
/// the world side of the same fail-stop contract: returning means either there
/// were no authoritative seals or every exact payload has a verified recovery
/// record that will block ordinary writable boot.
pub(crate) async fn preserve_unqueued_seals(
    spool: &QuarantineSpool,
    ops: &[ModuleOp],
    serial: u64,
    detail: &str,
) {
    let error = StoreError::Invalid(bounded_detail(detail.to_owned()));
    for (ns, key, value) in ops.iter().filter_map(|op| match &op.op {
        ModuleWrite::JsonSet { key, value }
            if !op.ns.contains(':')
                && op.ns.len() <= mush_core::MAX_MODULE_NAMESPACE_BYTES
                && mush_core::is_sealed_chunk(key, value) =>
        {
            Some((op.ns.as_str(), key.as_str(), value))
        }
        _ => None,
    }) {
        spool_quarantine_until_preserved(spool, ns, key, value, serial, &error, 0).await;
    }
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
/// Fails fast when:
/// - any corrupt object document was discovered — silently omitting an
///   otherwise independent object would be authoritative state loss;
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
        Some(snapshot) => hydrate_world_snapshot(snapshot)?,
        None => {
            let mut world = World::new_minimal();
            let dirty = world.take_dirty();
            let seed = batch_from_dirty(&world, dirty, 0, true);
            let receipt = store
                .write(seed)
                .await
                .map_err(|e| format!("writing world seed batch: {e}"))?;
            if receipt.serial != 0 {
                return Err(format!(
                    "writing world seed batch: expected receipt serial 0, got {}",
                    receipt.serial
                ));
            }
            tracing::info!("empty store: seeded minimal world (Limbo #0, God #1)");
            world
        }
    };
    let mut world = world;
    world.set_now(unix_now_s());
    Ok(world)
}

/// Validate and hydrate one non-empty store snapshot. Kept separate from the
/// async read so fail-closed handling of loader-reported corruption is directly
/// regression-testable without exposing raw-store mutation hooks.
fn hydrate_world_snapshot(snapshot: WorldSnapshot) -> Result<World, String> {
    let WorldSnapshot {
        objects,
        next_id,
        corrupt_docs,
        ..
    } = snapshot;
    if corrupt_docs > 0 {
        return Err(format!(
            "persisted world contains {corrupt_docs} corrupt object doc(s); refusing to boot \
             because omitting authoritative objects would lose state (see the store log for keys)"
        ));
    }

    let count = objects.len();
    if (next_id as usize).saturating_sub(count) > MAX_TOMBSTONE_GAP {
        return Err(format!(
            "persisted next_id {next_id} is implausibly far beyond the {count} stored \
             object(s); refusing to allocate — meta:next_id looks corrupt"
        ));
    }
    let world = World::from_objects(objects, next_id)
        .map_err(|e| format!("persisted world is inconsistent: {e}"))?;
    for (id, name) in [(mush_core::LIMBO, "Limbo"), (mush_core::GOD, "God")] {
        if world.get(id).is_none() {
            return Err(format!(
                "persisted world is missing {name} ({id}); refusing to boot an unusable world"
            ));
        }
    }
    tracing::info!(objects = count, next_id, "world loaded from store");
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
        let a = crate::quarantine::seal_identity_hash("scenes", "log:s:0", &v);
        // Stable across calls (content-addressed).
        assert_eq!(
            a,
            crate::quarantine::seal_identity_hash("scenes", "log:s:0", &v)
        );
        // Distinct on ns, key, or payload — no collision from a non-injective
        // sanitizer (log:s:0 vs log_s_0 differ, as do different namespaces).
        assert_ne!(
            a,
            crate::quarantine::seal_identity_hash("scenes", "log_s_0", &v)
        );
        assert_ne!(
            a,
            crate::quarantine::seal_identity_hash("econ", "log:s:0", &v)
        );
        assert_ne!(
            a,
            crate::quarantine::seal_identity_hash(
                "scenes",
                "log:s:0",
                &serde_json::json!({"chunk": 1}),
            )
        );
        assert_eq!(a.len(), 64, "sha-256 hex");
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

    #[test]
    fn boot_refuses_any_loader_reported_corrupt_object() {
        let world = World::new_minimal();
        let snapshot = WorldSnapshot {
            objects: world.iter().cloned().collect(),
            next_id: world.next_id(),
            schema_version: 1,
            corrupt_docs: 1,
        };

        let error = hydrate_world_snapshot(snapshot)
            .expect_err("one corrupt independent object must stop boot");
        assert!(
            error.contains("1 corrupt object doc") && error.contains("refusing to boot"),
            "{error}"
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
