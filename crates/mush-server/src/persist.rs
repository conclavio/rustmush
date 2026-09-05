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
//! - [`spawn_store_writer`] — the dedicated writer task. Independently bounded
//!   legacy, authoritative, and control admission lanes are merged by one
//!   monotonic acceptance sequence, then applied **in order**, each awaited to
//!   its ack before the next (write-through per command; no timer batching). On a
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mush_config::metrics::Metrics;
use mush_core::{AuthoritativeBundle, CommitToken, ObjId, World};
use mush_store::{
    AuthoritativeReceipt, AuthoritativeRetentionUsage, AuthoritativeStore, BundleStatus,
    IntentReceipt, MAX_AUTHORITATIVE_DISCOVERY_PAGE, MAX_RETAINED_AUTHORITATIVE_BYTES, ModuleOp,
    ModuleWrite, ObjectStore, StoreError, WorldSnapshot, WriteBatch,
};
use smol_str::SmolStr;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::quarantine::{QuarantineSpool, authoritative_bundle_evidence_hash};
use crate::world_task::WriterSignalSender;
use mush_config::unix_now_s;

/// How many times a sealed chunk is retried **in isolation** (alone, in its own
/// batch) before it is declared a deterministic poison and quarantined (E0). The
/// isolation is what separates a genuine poison seal from a seal merely caught
/// as collateral in a batch some *other* op poisoned — the collateral one
/// commits alone on the first attempt. Small: a deterministic failure repeats
/// identically, so a handful of attempts is diagnosis, not hope.
const POISON_ISOLATION_ATTEMPTS: u32 = 5;

/// Capacity of the legacy writer lane. The world task `await`s its sends, so a
/// full lane back-pressures legacy production instead of dropping batches.
///
/// Deliberately small: every queued batch is a command whose output the
/// player has already seen but whose write has not yet been acked, so this
/// bounds the worst-case `kill -9` loss window while the store is merely
/// *slow* (a stalled BGSAVE, a saturated disk). In the steady state the
/// queue is empty and the loss window is exactly the in-flight command.
const WRITER_LEGACY_QUEUE: usize = 64;

/// Reserved admission for exact-token actions. A legacy seal/retry flood can
/// fill its own lane but cannot consume these slots. Cross-lane execution still
/// follows the global accepted order, so this is isolation without overtaking.
const WRITER_AUTHORITATIVE_QUEUE: usize = 16;

/// Reserved admission for snapshot/recovery control. Boot recovery runs before
/// the runtime writer exists; runtime maintenance uses this lane and cannot be
/// crowded out by gameplay writes.
const WRITER_CONTROL_QUEUE: usize = 8;

/// First retry delay after a failed write; doubles up to [`RETRY_MAX`].
const RETRY_INITIAL: Duration = Duration::from_millis(100);

/// Retry backoff ceiling.
const RETRY_MAX: Duration = Duration::from_secs(5);

/// Maximum cursor pages accepted from one exhaustive authoritative-intent
/// boot scan. At the hard 256-record retained-intent cap a healthy backend
/// normally needs very few pages; this much larger bound permits SCAN to walk
/// a large shared keyspace without allowing a corrupt/cycling backend to keep
/// writable boot pending forever.
const MAX_AUTHORITATIVE_BOOT_SCAN_PAGES: usize = 65_536;

/// A transient store error gets a small bounded recovery attempt budget during
/// boot. Exhausting it fails boot without erasing the retained intent; the
/// service manager can retry later after the store recovers.
const AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS: u32 = 3;

/// Exact replay attempts used to distinguish a deterministic K3 poison from
/// a stale/transient observation before the whole bundle is dead-lettered.
const AUTHORITATIVE_POISON_ATTEMPTS: u32 = 3;

/// Summary of the exact-intent recovery barrier that precedes ordinary world
/// loading and every writable startup mutation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AuthoritativeBootRecoveryReport {
    /// Entries observed in the pending accelerator before reconciliation.
    pub indexed: usize,
    /// Unique immutable intents found by exhaustive cursor discovery.
    pub retained: usize,
    /// Exact canonical bytes charged by all retained full intents.
    pub retained_bytes: usize,
    /// Whether boot rebuilt either non-authoritative capacity counter.
    pub retention_counters_repaired: bool,
    /// Pending intents replayed and finalized in this boot.
    pub reconciled: usize,
    /// Retained intents already carrying an exact finalized witness.
    pub already_finalized: usize,
    /// Finalized full intents replaced by compact checkpoint witnesses.
    pub checkpointed: usize,
    /// Finalized intents deliberately retained because durable outbox work is
    /// not yet terminal.
    pub checkpoint_deferred_outbox: usize,
    /// Cursor pages consumed, including an empty terminal page when Redis
    /// returns one.
    pub scan_pages: usize,
}

/// Exhaustively reconcile authoritative intents before an object/module
/// snapshot can be loaded or any ordinary boot write can occur.
///
/// The pending set is only an accelerator. This barrier independently walks
/// the complete retained-intent keyspace, requires every indexed token to be
/// present in that scan, rejects conflicting token/hash observations, and
/// reconciles every pending record by its exact hash. A stale finalized index
/// member is also passed through reconciliation so the backend can repair it.
/// The final pending-index read must be empty.
pub async fn reconcile_authoritative_for_writable_boot(
    store: &dyn AuthoritativeStore,
) -> Result<AuthoritativeBootRecoveryReport, String> {
    let retention_generation = store
        .authoritative_retention_generation()
        .await
        .map_err(|error| format!("reading authoritative retention generation: {error}"))?;
    let indexed = store
        .pending_authoritative_intents()
        .await
        .map_err(|error| format!("reading authoritative pending index: {error}"))?;
    let mut report = AuthoritativeBootRecoveryReport {
        indexed: indexed.len(),
        ..AuthoritativeBootRecoveryReport::default()
    };
    let mut indexed_by_token = BTreeMap::new();
    for entry in indexed {
        if let Some(existing) = indexed_by_token.insert(entry.token, entry.bundle_hash)
            && existing != entry.bundle_hash
        {
            return Err(format!(
                "authoritative pending index contains conflicting hashes for {}",
                entry.token
            ));
        }
    }

    let mut retained = BTreeMap::new();
    let mut retained_bytes = 0usize;
    let mut cursor = None;
    loop {
        report.scan_pages = report
            .scan_pages
            .checked_add(1)
            .ok_or_else(|| "authoritative boot scan page counter overflow".to_owned())?;
        if report.scan_pages > MAX_AUTHORITATIVE_BOOT_SCAN_PAGES {
            return Err(format!(
                "authoritative intent scan exceeded {MAX_AUTHORITATIVE_BOOT_SCAN_PAGES} pages; \
                 refusing writable boot"
            ));
        }
        let prior_cursor = cursor.clone();
        let page = store
            .scan_authoritative_intents(cursor, MAX_AUTHORITATIVE_DISCOVERY_PAGE)
            .await
            .map_err(|error| format!("scanning authoritative intents: {error}"))?;
        for bundle in page.intents {
            let token = bundle.token();
            let hash = bundle.bundle_hash();
            let encoded_bytes = serde_json::to_string(&bundle)
                .map_err(|error| {
                    format!("encoding retained authoritative intent {token}: {error}")
                })?
                .len();
            match retained.entry(token) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    retained_bytes =
                        retained_bytes.checked_add(encoded_bytes).ok_or_else(|| {
                            "authoritative retained-intent byte counter overflow".to_owned()
                        })?;
                    entry.insert(hash);
                }
                std::collections::btree_map::Entry::Occupied(entry) if *entry.get() == hash => {}
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(format!(
                        "authoritative token {token} appeared with conflicting bundle hashes during boot"
                    ));
                }
            }
        }
        if retained.len() > mush_store::MAX_RETAINED_AUTHORITATIVE_INTENTS {
            return Err(format!(
                "authoritative boot scan found {} retained intents; maximum is {}",
                retained.len(),
                mush_store::MAX_RETAINED_AUTHORITATIVE_INTENTS
            ));
        }
        if retained_bytes > MAX_RETAINED_AUTHORITATIVE_BYTES {
            return Err(format!(
                "authoritative boot scan found {retained_bytes} retained bytes; maximum is {MAX_RETAINED_AUTHORITATIVE_BYTES}"
            ));
        }
        let Some(next_cursor) = page.next_cursor else {
            break;
        };
        if prior_cursor.as_ref() == Some(&next_cursor) {
            return Err(format!(
                "authoritative intent scan repeated cursor {:?}; refusing writable boot",
                next_cursor.as_str()
            ));
        }
        cursor = Some(next_cursor);
    }
    report.retained = retained.len();
    report.retained_bytes = retained_bytes;

    for (token, indexed_hash) in &indexed_by_token {
        match retained.get(token) {
            Some(scanned_hash) if scanned_hash == indexed_hash => {}
            Some(_) => {
                return Err(format!(
                    "authoritative pending index disagrees with retained intent {token}"
                ));
            }
            None => {
                return Err(format!(
                    "authoritative pending index names {token}, absent from exhaustive intent scan"
                ));
            }
        }
    }

    report.retention_counters_repaired = store
        .repair_authoritative_retention(
            retention_generation,
            AuthoritativeRetentionUsage {
                count: report.retained,
                bytes: report.retained_bytes,
            },
        )
        .await
        .map_err(|error| format!("repairing authoritative retention counters: {error}"))?;

    for (token, hash) in retained {
        let was_indexed = indexed_by_token.contains_key(&token);
        match store
            .authoritative_status(token, hash)
            .await
            .map_err(|error| format!("reading authoritative status for {token}: {error}"))?
        {
            BundleStatus::Retired {
                minimum_accepted_epoch,
            } => {
                return Err(format!(
                    "retained authoritative intent {token} is below minimum epoch {minimum_accepted_epoch}"
                ));
            }
            BundleStatus::PendingExact => {
                reconcile_one_authoritative_boot_intent(store, token, hash).await?;
                report.reconciled += 1;
            }
            BundleStatus::FinalizedExact(receipt) => {
                if receipt.token() != token || receipt.bundle_hash() != hash {
                    return Err(format!(
                        "authoritative finalized witness for {token} is not exact"
                    ));
                }
                report.already_finalized += 1;
                if was_indexed {
                    // Reconciliation of an exact finalized record is the
                    // backend's idempotent stale-index repair path.
                    reconcile_one_authoritative_boot_intent(store, token, hash).await?;
                }
            }
            BundleStatus::QuarantinedExact(_) => {
                return Err(format!(
                    "retained authoritative intent {token} simultaneously has a quarantine tombstone"
                ));
            }
            BundleStatus::Absent => {
                return Err(format!(
                    "authoritative intent {token} disappeared during boot recovery"
                ));
            }
            BundleStatus::Conflict { stored_bundle_hash } => {
                return Err(format!(
                    "authoritative intent {token} changed during boot recovery \
                     (expected {hash}, stored {stored_bundle_hash})"
                ));
            }
        }
        match checkpoint_one_authoritative_boot_intent(store, token, hash).await? {
            BootCheckpoint::Released => report.checkpointed += 1,
            BootCheckpoint::AlreadyCheckpointed => {}
            BootCheckpoint::DeferredOutbox => report.checkpoint_deferred_outbox += 1,
        }
    }

    let remaining = store
        .pending_authoritative_intents()
        .await
        .map_err(|error| format!("verifying authoritative pending index: {error}"))?;
    if !remaining.is_empty() {
        return Err(format!(
            "authoritative recovery left {} pending intent(s); refusing writable boot",
            remaining.len()
        ));
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootCheckpoint {
    Released,
    AlreadyCheckpointed,
    DeferredOutbox,
}

async fn checkpoint_one_authoritative_boot_intent(
    store: &dyn AuthoritativeStore,
    token: mush_core::CommitToken,
    hash: mush_core::CanonicalHash,
) -> Result<BootCheckpoint, String> {
    for attempt in 1..=AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS {
        match store.checkpoint_authoritative(token, hash).await {
            Ok(checkpoint)
                if checkpoint.receipt.token() == token
                    && checkpoint.receipt.bundle_hash() == hash =>
            {
                return Ok(if checkpoint.resumed {
                    BootCheckpoint::AlreadyCheckpointed
                } else {
                    BootCheckpoint::Released
                });
            }
            Ok(_) => {
                return Err(format!(
                    "authoritative checkpoint returned a non-exact witness for {token}"
                ));
            }
            Err(StoreError::AuthoritativeOutboxPending) => {
                return Ok(BootCheckpoint::DeferredOutbox);
            }
            Err(error)
                if error.is_transient() && attempt < AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS =>
            {
                tracing::warn!(
                    %token,
                    attempt,
                    max_attempts = AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS,
                    error = %error,
                    "transient authoritative boot checkpoint failure; retrying exact intent"
                );
                tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
            }
            Err(error) => {
                return Err(format!(
                    "checkpointing authoritative intent {token} on attempt {attempt}: {error}"
                ));
            }
        }
    }
    unreachable!("bounded authoritative checkpoint loop always returns")
}

async fn reconcile_one_authoritative_boot_intent(
    store: &dyn AuthoritativeStore,
    token: mush_core::CommitToken,
    hash: mush_core::CanonicalHash,
) -> Result<(), String> {
    for attempt in 1..=AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS {
        match store.reconcile_authoritative(token, hash).await {
            Ok(receipt) if receipt.token() == token && receipt.bundle_hash() == hash => {
                return Ok(());
            }
            Ok(_) => {
                return Err(format!(
                    "authoritative reconciliation returned a non-exact witness for {token}"
                ));
            }
            Err(error)
                if error.is_transient() && attempt < AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS =>
            {
                tracing::warn!(
                    %token,
                    attempt,
                    max_attempts = AUTHORITATIVE_BOOT_RECONCILE_ATTEMPTS,
                    error = %error,
                    "transient authoritative boot reconciliation failure; retrying exact intent"
                );
                tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
            }
            Err(error) => {
                return Err(format!(
                    "reconciling authoritative intent {token} on attempt {attempt}: {error}"
                ));
            }
        }
    }
    unreachable!("bounded authoritative reconciliation loop always returns")
}

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
    /// Reserve and reconcile one complete K3 exact-token bundle. The writer
    /// keeps FIFO custody across both operations and returns only the exact
    /// immutable receipt or a typed refusal/integrity failure.
    Authoritative {
        bundle: Box<AuthoritativeBundle>,
        /// Resolves at the request-ownership linearization point: only after
        /// the exact immutable intent exists (or an exact finalized result was
        /// rediscovered). This is deliberately separate from `notify`.
        ownership: oneshot::Sender<Result<IntentReceipt, AuthoritativeCommitFailure>>,
        notify: oneshot::Sender<Result<AuthoritativeReceipt, AuthoritativeCommitFailure>>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterLane {
    Legacy,
    Authoritative,
    Control,
}

impl WriterLane {
    const fn for_job(job: &StoreJob) -> Self {
        match job {
            StoreJob::Write { .. } => Self::Legacy,
            StoreJob::Authoritative { .. } => Self::Authoritative,
            StoreJob::Bgsave { .. } => Self::Control,
        }
    }
}

struct SequencedStoreJob {
    order: u64,
    lane: WriterLane,
    job: StoreJob,
    /// Lane capacity is held through merge buffering and store completion,
    /// rather than released merely because the channel receiver copied this
    /// job into its ordering map.
    _capacity: OwnedSemaphorePermit,
}

/// Cloneable admission handle for the single persistence writer.
///
/// Each work class owns a separate bounded channel. A sender reserves capacity
/// in its class before taking the next global order, so a blocked/full legacy
/// lane neither consumes an order nor prevents authoritative/control custody.
/// Once accepted, all classes execute in exact global order.
#[derive(Clone)]
pub struct StoreWriter {
    legacy: mpsc::Sender<SequencedStoreJob>,
    authoritative: mpsc::Sender<SequencedStoreJob>,
    control: mpsc::Sender<SequencedStoreJob>,
    legacy_capacity: Arc<Semaphore>,
    authoritative_capacity: Arc<Semaphore>,
    control_capacity: Arc<Semaphore>,
    next_order: Arc<AtomicU64>,
    metrics: Arc<Metrics>,
}

impl StoreWriter {
    /// Transfer one job into its independently bounded lane.
    pub async fn send(&self, job: StoreJob) -> Result<(), mpsc::error::SendError<StoreJob>> {
        let lane = WriterLane::for_job(&job);
        let sender = match lane {
            WriterLane::Legacy => &self.legacy,
            WriterLane::Authoritative => &self.authoritative,
            WriterLane::Control => &self.control,
        };
        let capacity = match lane {
            WriterLane::Legacy => Arc::clone(&self.legacy_capacity),
            WriterLane::Authoritative => Arc::clone(&self.authoritative_capacity),
            WriterLane::Control => Arc::clone(&self.control_capacity),
        };
        let capacity = match capacity.acquire_owned().await {
            Ok(capacity) => capacity,
            Err(_) => return Err(mpsc::error::SendError(job)),
        };
        let permit = match sender.reserve().await {
            Ok(permit) => permit,
            Err(_) => return Err(mpsc::error::SendError(job)),
        };
        let order =
            match self
                .next_order
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                }) {
                Ok(order) => order,
                Err(_) => {
                    tracing::error!("store writer acceptance sequence exhausted");
                    return Err(mpsc::error::SendError(job));
                }
            };
        self.metrics.add_store_write_queue_depth(1);
        match lane {
            WriterLane::Legacy => self.metrics.add_store_legacy_queue_depth(1),
            WriterLane::Authoritative => {
                self.metrics.add_store_authoritative_queue_depth(1);
            }
            WriterLane::Control => self.metrics.add_store_control_queue_depth(1),
        }
        permit.send(SequencedStoreJob {
            order,
            lane,
            job,
            _capacity: capacity,
        });
        Ok(())
    }
}

struct StoreWriterReceivers {
    legacy: mpsc::Receiver<SequencedStoreJob>,
    authoritative: mpsc::Receiver<SequencedStoreJob>,
    control: mpsc::Receiver<SequencedStoreJob>,
    buffered: BTreeMap<u64, SequencedStoreJob>,
    next_order: u64,
}

impl StoreWriterReceivers {
    fn buffer(&mut self, job: SequencedStoreJob) {
        let prior = self.buffered.insert(job.order, job);
        assert!(prior.is_none(), "duplicate store-writer acceptance order");
    }

    fn drain_ready(&mut self) {
        while let Ok(job) = self.legacy.try_recv() {
            self.buffer(job);
        }
        while let Ok(job) = self.authoritative.try_recv() {
            self.buffer(job);
        }
        while let Ok(job) = self.control.try_recv() {
            self.buffer(job);
        }
    }

    async fn recv(&mut self) -> Option<SequencedStoreJob> {
        loop {
            self.drain_ready();
            if let Some(job) = self.buffered.remove(&self.next_order) {
                self.next_order = self
                    .next_order
                    .checked_add(1)
                    .expect("accepted writer order cannot overflow");
                return Some(job);
            }
            if self.legacy.is_closed() && self.authoritative.is_closed() && self.control.is_closed()
            {
                assert!(
                    self.buffered.is_empty(),
                    "store-writer acceptance sequence closed with a gap"
                );
                return None;
            }
            tokio::select! {
                job = self.legacy.recv(), if !self.legacy.is_closed() => {
                    if let Some(job) = job { self.buffer(job); }
                }
                job = self.authoritative.recv(), if !self.authoritative.is_closed() => {
                    if let Some(job) = job { self.buffer(job); }
                }
                job = self.control.recv(), if !self.control.is_closed() => {
                    if let Some(job) = job { self.buffer(job); }
                }
            }
        }
    }

    #[cfg(test)]
    fn try_recv(&mut self) -> Result<SequencedStoreJob, mpsc::error::TryRecvError> {
        self.drain_ready();
        if let Some(job) = self.buffered.remove(&self.next_order) {
            self.next_order = self
                .next_order
                .checked_add(1)
                .expect("accepted writer order cannot overflow");
            return Ok(job);
        }
        if self.legacy.is_closed() && self.authoritative.is_closed() && self.control.is_closed() {
            Err(mpsc::error::TryRecvError::Disconnected)
        } else {
            Err(mpsc::error::TryRecvError::Empty)
        }
    }
}

fn store_writer_channels(metrics: Arc<Metrics>) -> (StoreWriter, StoreWriterReceivers) {
    let (legacy, legacy_rx) = mpsc::channel(WRITER_LEGACY_QUEUE);
    let (authoritative, authoritative_rx) = mpsc::channel(WRITER_AUTHORITATIVE_QUEUE);
    let (control, control_rx) = mpsc::channel(WRITER_CONTROL_QUEUE);
    (
        StoreWriter {
            legacy,
            authoritative,
            control,
            legacy_capacity: Arc::new(Semaphore::new(WRITER_LEGACY_QUEUE)),
            authoritative_capacity: Arc::new(Semaphore::new(WRITER_AUTHORITATIVE_QUEUE)),
            control_capacity: Arc::new(Semaphore::new(WRITER_CONTROL_QUEUE)),
            next_order: Arc::new(AtomicU64::new(0)),
            metrics,
        },
        StoreWriterReceivers {
            legacy: legacy_rx,
            authoritative: authoritative_rx,
            control: control_rx,
            buffered: BTreeMap::new(),
            next_order: 0,
        },
    )
}

#[cfg(test)]
pub(crate) struct StoreWriterTestReceiver(StoreWriterReceivers);

#[cfg(test)]
impl StoreWriterTestReceiver {
    pub(crate) async fn recv(&mut self) -> Option<StoreJob> {
        self.0.recv().await.map(|queued| queued.job)
    }

    pub(crate) fn try_recv(&mut self) -> Result<StoreJob, mpsc::error::TryRecvError> {
        self.0.try_recv().map(|queued| queued.job)
    }
}

#[cfg(test)]
pub(crate) fn test_store_writer_channel(
    metrics: Arc<Metrics>,
) -> (StoreWriter, StoreWriterTestReceiver) {
    let (writer, receiver) = store_writer_channels(metrics);
    (writer, StoreWriterTestReceiver(receiver))
}

/// Terminal result from an authoritative writer job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthoritativeCommitFailure {
    /// Reservation definitively created no intent; the typed RAM image may be
    /// aborted without freezing unrelated persistence.
    Refused(String),
    /// An intent may exist, exact identity is contradictory, or store
    /// integrity is damaged. Later writes must not pass this predecessor.
    Integrity(String),
}

impl AuthoritativeCommitFailure {
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Refused(detail) | Self::Integrity(detail) => detail,
        }
    }
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
) -> (StoreWriter, JoinHandle<()>) {
    let (tx, rx) = store_writer_channels(Arc::clone(&metrics));
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
    mut rx: StoreWriterReceivers,
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
    while let Some(SequencedStoreJob {
        lane,
        job,
        _capacity: capacity,
        ..
    }) = rx.recv().await
    {
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
                    // A retained seal that was already healed or quarantined
                    // is merely the world's delivery retry. Its exact bytes
                    // already have terminal custody, so do not manufacture a
                    // new serial-dependent legacy-batch record on every flush.
                    // Any collateral operation still requires its own complete
                    // batch record before this rejected job leaves RAM.
                    if !batch_fully_covered_by_handled_seals(&batch, &poison) {
                        spool_legacy_batch_until_preserved(
                            &quarantine_spool,
                            &batch,
                            &StoreError::Invalid(detail.clone()),
                            0,
                        )
                        .await;
                    }
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
            StoreJob::Authoritative {
                bundle,
                ownership,
                notify,
            } => {
                let token = bundle.token();
                let result = if let Some(reason) = &integrity_halt {
                    integrity.latch();
                    let failure = AuthoritativeCommitFailure::Integrity(format!(
                        "persistence integrity halted after an unresolved predecessor: {reason}"
                    ));
                    let _ = ownership.send(Err(failure.clone()));
                    Err(failure)
                } else {
                    let result = commit_authoritative_with_retry(
                        &*store,
                        *bundle,
                        ownership,
                        &mut healthy,
                        &signals,
                        &quarantine_spool,
                    )
                    .await;
                    if let Err(AuthoritativeCommitFailure::Integrity(detail)) = &result {
                        integrity.latch();
                        let reason = bounded_detail(format!(
                            "authoritative bundle {token} was not exactly committed: {detail}"
                        ));
                        tracing::error!(
                            %token,
                            %reason,
                            "store writer latched integrity halt after authoritative bundle"
                        );
                        integrity_halt = Some(reason);
                    }
                    result
                };
                if result.is_ok() {
                    metrics.set_store_last_ack_unix(unix_now_s());
                }
                let _ = notify.send(result);
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
        metrics.add_store_write_queue_depth(-1);
        match lane {
            WriterLane::Legacy => metrics.add_store_legacy_queue_depth(-1),
            WriterLane::Authoritative => metrics.add_store_authoritative_queue_depth(-1),
            WriterLane::Control => metrics.add_store_control_queue_depth(-1),
        }
        drop(capacity);
    }
    tracing::info!("store writer drained and exiting");
}

async fn commit_authoritative_with_retry(
    store: &dyn ObjectStore,
    bundle: AuthoritativeBundle,
    ownership: oneshot::Sender<Result<IntentReceipt, AuthoritativeCommitFailure>>,
    healthy: &mut bool,
    signals: &WriterSignalSender,
    quarantine_spool: &QuarantineSpool,
) -> Result<AuthoritativeReceipt, AuthoritativeCommitFailure> {
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let mut delay = RETRY_INITIAL;
    let Some(authoritative) = store.authoritative_store() else {
        let failure = AuthoritativeCommitFailure::Integrity(
            "configured store has no K3 authoritative capability".to_owned(),
        );
        let _ = ownership.send(Err(failure.clone()));
        return Err(failure);
    };
    let mut deterministic_reconcile_attempts = 0u32;
    let mut ownership = Some(ownership);

    let intent = loop {
        match authoritative
            .reserve_authoritative_intent(bundle.clone())
            .await
        {
            Ok(receipt) if receipt.token == token && receipt.bundle_hash == hash => break receipt,
            Ok(_) => {
                let failure = AuthoritativeCommitFailure::Integrity(format!(
                    "authoritative reservation returned a non-exact witness for {token}"
                ));
                publish_ownership(&mut ownership, Err(failure.clone()));
                return Err(failure);
            }
            Err(error) if error.is_transient() => {
                mark_authoritative_transient(healthy, signals, token, &error);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
            Err(error) => {
                let status = authoritative.authoritative_status(token, hash).await;
                match status {
                    Ok(BundleStatus::Retired {
                        minimum_accepted_epoch,
                    }) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "authoritative reservation for {token} is below minimum epoch {minimum_accepted_epoch}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Ok(BundleStatus::PendingExact) => {
                        break IntentReceipt {
                            token,
                            bundle_hash: hash,
                            resumed: true,
                        };
                    }
                    Ok(BundleStatus::QuarantinedExact(quarantine)) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "authoritative reservation for {token} reached quarantine evidence {}",
                            quarantine.evidence_hash()
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Ok(BundleStatus::FinalizedExact(receipt))
                        if receipt.token() == token && receipt.bundle_hash() == hash =>
                    {
                        publish_ownership(
                            &mut ownership,
                            Ok(IntentReceipt {
                                token,
                                bundle_hash: hash,
                                resumed: true,
                            }),
                        );
                        return checkpoint_finalized_with_retry(
                            authoritative,
                            &bundle,
                            *receipt,
                            healthy,
                            signals,
                            quarantine_spool,
                            &mut delay,
                        )
                        .await;
                    }
                    Ok(BundleStatus::FinalizedExact(_)) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "authoritative status returned a non-exact finalized witness for {token}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Ok(BundleStatus::Absent)
                        if matches!(
                            error,
                            StoreError::AuthoritativeCapacity { .. }
                                | StoreError::AuthoritativeIntentInFlight { .. }
                                | StoreError::AuthoritativeIdempotencyConflict
                                | StoreError::Invalid(_)
                        ) =>
                    {
                        mark_authoritative_recovered(healthy, signals, token);
                        let failure = AuthoritativeCommitFailure::Refused(format!(
                            "authoritative reservation refused before intent: {error}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Ok(BundleStatus::Absent) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "authoritative reservation failed without an intent: {error}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Ok(BundleStatus::Conflict { stored_bundle_hash }) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "authoritative token {token} conflicts with stored bundle {stored_bundle_hash}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                    Err(status_error) if status_error.is_transient() => {
                        mark_authoritative_transient(healthy, signals, token, &status_error);
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(RETRY_MAX);
                    }
                    Err(status_error) => {
                        let failure = AuthoritativeCommitFailure::Integrity(format!(
                            "cannot determine authoritative reservation state for {token}: {status_error}"
                        ));
                        publish_ownership(&mut ownership, Err(failure.clone()));
                        return Err(failure);
                    }
                }
            }
        }
    };
    crate::process_fault::terminate_at("after_intent_before_ownership").await;
    publish_ownership(&mut ownership, Ok(intent));
    crate::process_fault::terminate_at("after_ownership_before_materialization").await;

    loop {
        match authoritative.reconcile_authoritative(token, hash).await {
            Ok(receipt) if receipt.token() == token && receipt.bundle_hash() == hash => {
                crate::process_fault::terminate_at("after_finalize_before_checkpoint").await;
                return checkpoint_finalized_with_retry(
                    authoritative,
                    &bundle,
                    receipt,
                    healthy,
                    signals,
                    quarantine_spool,
                    &mut delay,
                )
                .await;
            }
            Ok(_) => {
                return Err(AuthoritativeCommitFailure::Integrity(format!(
                    "authoritative reconciliation returned a non-exact witness for {token}"
                )));
            }
            Err(error) if error.is_transient() => {
                mark_authoritative_transient(healthy, signals, token, &error);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
            Err(error) => match authoritative.authoritative_status(token, hash).await {
                Ok(BundleStatus::Retired {
                    minimum_accepted_epoch,
                }) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative intent {token} is below minimum epoch {minimum_accepted_epoch}: {error}"
                    )));
                }
                Ok(BundleStatus::FinalizedExact(receipt))
                    if receipt.token() == token && receipt.bundle_hash() == hash =>
                {
                    return checkpoint_finalized_with_retry(
                        authoritative,
                        &bundle,
                        *receipt,
                        healthy,
                        signals,
                        quarantine_spool,
                        &mut delay,
                    )
                    .await;
                }
                Ok(BundleStatus::PendingExact) => {
                    deterministic_reconcile_attempts = deterministic_reconcile_attempts
                        .checked_add(1)
                        .expect("bounded authoritative poison attempt counter");
                    if deterministic_reconcile_attempts < AUTHORITATIVE_POISON_ATTEMPTS {
                        tracing::warn!(
                            %token,
                            attempt = deterministic_reconcile_attempts,
                            max_attempts = AUTHORITATIVE_POISON_ATTEMPTS,
                            error = %error,
                            "deterministic authoritative reconciliation failure; retrying exact bundle before quarantine"
                        );
                        continue;
                    }
                    quarantine_spool
                        .quarantine_authoritative_bundle(
                            &bundle,
                            &error,
                            deterministic_reconcile_attempts,
                        )
                        .map_err(|quarantine_error| {
                            AuthoritativeCommitFailure::Integrity(format!(
                                "authoritative bundle {token} remained pending after {deterministic_reconcile_attempts} deterministic attempts and its dead-letter could not be preserved: {quarantine_error}"
                            ))
                        })?;
                    let evidence_hash = authoritative_bundle_evidence_hash(&bundle);
                    loop {
                        match authoritative
                            .discard_quarantined_authoritative_intent(token, hash, evidence_hash)
                            .await
                        {
                            Ok(discard)
                                if discard.quarantine.token() == token
                                    && discard.quarantine.bundle_hash() == hash
                                    && discard.quarantine.evidence_hash() == evidence_hash =>
                            {
                                break;
                            }
                            Ok(_) => {
                                return Err(AuthoritativeCommitFailure::Integrity(format!(
                                    "authoritative discard returned a non-exact quarantine witness for {token}"
                                )));
                            }
                            Err(discard_error) if discard_error.is_transient() => {
                                mark_authoritative_transient(
                                    healthy,
                                    signals,
                                    token,
                                    &discard_error,
                                );
                                tokio::time::sleep(delay).await;
                                delay = (delay * 2).min(RETRY_MAX);
                            }
                            Err(discard_error) => {
                                return Err(AuthoritativeCommitFailure::Integrity(format!(
                                    "authoritative bundle {token} was spooled but its pending intent could not be replaced by an exact quarantine tombstone: {discard_error}"
                                )));
                            }
                        }
                    }
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative bundle {token} was quarantined and its pending intent released to an exact non-replayable tombstone after {deterministic_reconcile_attempts} deterministic reconciliation failures: {error}"
                    )));
                }
                Ok(BundleStatus::QuarantinedExact(quarantine)) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative intent {token} is quarantined under evidence {}",
                        quarantine.evidence_hash()
                    )));
                }
                Ok(BundleStatus::Absent) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative intent {token} disappeared after reservation: {error}"
                    )));
                }
                Ok(BundleStatus::Conflict { stored_bundle_hash }) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative intent {token} changed to bundle {stored_bundle_hash}: {error}"
                    )));
                }
                Ok(BundleStatus::FinalizedExact(_)) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "authoritative status returned a non-exact finalized witness for {token}"
                    )));
                }
                Err(status_error) if status_error.is_transient() => {
                    mark_authoritative_transient(healthy, signals, token, &status_error);
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RETRY_MAX);
                }
                Err(status_error) => {
                    return Err(AuthoritativeCommitFailure::Integrity(format!(
                        "cannot determine authoritative reconciliation state for {token}: {status_error}"
                    )));
                }
            },
        }
    }
}

fn publish_ownership(
    slot: &mut Option<oneshot::Sender<Result<IntentReceipt, AuthoritativeCommitFailure>>>,
    result: Result<IntentReceipt, AuthoritativeCommitFailure>,
) {
    if let Some(sender) = slot.take() {
        let _ = sender.send(result);
    }
}

#[allow(clippy::too_many_arguments)]
async fn checkpoint_finalized_with_retry(
    store: &dyn AuthoritativeStore,
    bundle: &AuthoritativeBundle,
    receipt: AuthoritativeReceipt,
    healthy: &mut bool,
    signals: &WriterSignalSender,
    quarantine_spool: &QuarantineSpool,
    delay: &mut Duration,
) -> Result<AuthoritativeReceipt, AuthoritativeCommitFailure> {
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let mut deterministic_attempts = 0u32;
    loop {
        match store.checkpoint_authoritative(token, hash).await {
            Ok(checkpoint)
                if checkpoint.receipt.token() == token
                    && checkpoint.receipt.bundle_hash() == hash =>
            {
                mark_authoritative_recovered(healthy, signals, token);
                crate::process_fault::terminate_at("after_checkpoint_before_final_ack").await;
                return Ok(checkpoint.receipt);
            }
            Ok(_) => {
                return Err(AuthoritativeCommitFailure::Integrity(format!(
                    "authoritative checkpoint returned a non-exact witness for {token}"
                )));
            }
            Err(StoreError::AuthoritativeOutboxPending) => {
                // Durable outbox dispatch owns the full replay record until
                // every item is terminal. Finalization is nevertheless exact.
                mark_authoritative_recovered(healthy, signals, token);
                return Ok(receipt);
            }
            Err(error) if error.is_transient() => {
                mark_authoritative_transient(healthy, signals, token, &error);
                tokio::time::sleep(*delay).await;
                *delay = (*delay * 2).min(RETRY_MAX);
            }
            Err(error) => {
                deterministic_attempts = deterministic_attempts
                    .checked_add(1)
                    .expect("bounded checkpoint poison attempt counter");
                if deterministic_attempts < AUTHORITATIVE_POISON_ATTEMPTS {
                    tracing::warn!(
                        %token,
                        attempt = deterministic_attempts,
                        max_attempts = AUTHORITATIVE_POISON_ATTEMPTS,
                        error = %error,
                        "deterministic authoritative checkpoint failure; retrying before quarantine"
                    );
                    continue;
                }
                quarantine_spool
                    .quarantine_authoritative_bundle(bundle, &error, deterministic_attempts)
                    .map_err(|quarantine_error| {
                        AuthoritativeCommitFailure::Integrity(format!(
                            "finalized authoritative bundle {token} could not be checkpointed and its dead-letter could not be preserved: {quarantine_error}"
                        ))
                    })?;
                return Err(AuthoritativeCommitFailure::Integrity(format!(
                    "finalized authoritative bundle {token} could not be checkpointed and was quarantined after {deterministic_attempts} deterministic attempts: {error}"
                )));
            }
        }
    }
}

fn mark_authoritative_transient(
    healthy: &mut bool,
    signals: &WriterSignalSender,
    token: CommitToken,
    error: &StoreError,
) {
    tracing::error!(%token, error = %error, "authoritative store operation failed; retrying exact bundle");
    if *healthy {
        *healthy = false;
        notify_status(signals, false, error.to_string());
    }
}

fn mark_authoritative_recovered(
    healthy: &mut bool,
    signals: &WriterSignalSender,
    token: CommitToken,
) {
    if !*healthy {
        *healthy = true;
        tracing::info!(%token, "authoritative store writes recovered");
        notify_status(signals, true, String::new());
    }
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
                    spool_legacy_batch_until_preserved(quarantine_spool, &batch, &terminal, 0)
                        .await;
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
                spool_legacy_batch_until_preserved(
                    quarantine_spool,
                    &batch,
                    &StoreError::Closed,
                    0,
                )
                .await;
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
                // Preserve the complete causal write set before isolating any
                // seal. Object post-images, deletes, open Ledger chunks, and
                // collateral module operations must not disappear merely
                // because only sealed chunks have an older recovery format.
                spool_legacy_batch_until_preserved(quarantine_spool, &batch, &e, 1).await;
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
                        "dropped batch carried module ops; the batch is spooled to the \
                         quarantine, which replays it at recovery only when every op is an \
                         idempotent absolute post-image (a delete or a stream append is not, \
                         and stays a tombstone for a person to reconcile)"
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

/// Whether every operation in a rejected batch is an exact seal generation
/// whose terminal outcome is already held by [`PoisonState`]. This is narrowly
/// a retained-delivery de-duplication rule: object writes, deletes, allocator
/// changes, non-seal module ops, and mixed batches always need complete batch
/// custody even if one member seal was seen before.
fn batch_fully_covered_by_handled_seals(batch: &WriteBatch, poison: &PoisonState) -> bool {
    if !batch.upserts.is_empty()
        || !batch.deletes.is_empty()
        || batch.next_id.is_some()
        || batch.module_ops.is_empty()
    {
        return false;
    }
    let seals = sealed_chunks_in(batch);
    seals.len() == batch.module_ops.len()
        && seals.iter().all(|(namespace, key, value)| {
            let id = (
                namespace.clone(),
                key.clone(),
                crate::quarantine::seal_identity_hash(namespace, key, value),
            );
            poison.handled.contains_key(&id)
        })
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

fn spool_legacy_batch(
    spool: &QuarantineSpool,
    batch: &WriteBatch,
    err: &StoreError,
    attempts: u32,
) -> bool {
    match spool.quarantine_legacy_batch(batch, err, attempts) {
        Ok(true) => {
            tracing::error!(
                serial = batch.serial,
                operations = batch.upserts.len()
                    + batch.deletes.len()
                    + batch.module_ops.len()
                    + usize::from(batch.next_id.is_some()),
                "complete failed legacy batch preserved in quarantine"
            );
            true
        }
        Ok(false) => false,
        Err(error) => {
            tracing::error!(
                serial = batch.serial,
                %error,
                "FAILED to preserve complete legacy batch in quarantine"
            );
            false
        }
    }
}

/// Keep custody in the fail-stopped writer until the independent spool has a
/// verified complete batch record. This is deliberately the same unbounded
/// operational retry posture as authoritative bundle/seal preservation: disk
/// or capacity repair is safer than allowing the only complete recovery bytes
/// to leave RAM.
async fn spool_legacy_batch_until_preserved(
    spool: &QuarantineSpool,
    batch: &WriteBatch,
    err: &StoreError,
    attempts: u32,
) {
    if batch.is_empty() {
        return;
    }
    let mut delay = QUARANTINE_RETRY_INITIAL;
    loop {
        if spool_legacy_batch(spool, batch, err, attempts) {
            return;
        }
        tracing::error!(
            serial = batch.serial,
            retry_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            "CRITICAL: complete failed legacy batch is still RAM-only; keeping writer fail-stopped and retrying"
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(QUARANTINE_RETRY_MAX);
    }
}

/// Spill a complete legacy batch that could not enter writer custody, then
/// preserve its sealed members under the older per-seal recovery protocol as
/// supplemental evidence. The complete record is what protects unrelated
/// object/module/delete/allocator state.
pub(crate) async fn preserve_unqueued_batch(
    spool: &QuarantineSpool,
    batch: &WriteBatch,
    detail: &str,
) {
    let error = StoreError::Invalid(bounded_detail(detail.to_owned()));
    spool_legacy_batch_until_preserved(spool, batch, &error, 0).await;
    preserve_unqueued_seals(spool, &batch.module_ops, batch.serial, detail).await;
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
    use mush_core::{
        AuthoritativeBundle, AuthoritativeBundleDraft, CanonicalHash, CanonicalId, CommitToken,
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, GOD,
        IdempotencyKey, IdempotencyScope, LIMBO, Obj, ObjKind, OwnerId, PreparedCausalRef,
        PrincipalId, ReplayWrite, RequestUid, RulesValue, ScopedIdempotencyKey,
    };
    use mush_store::{AuthoritativeStore, BundleStatus, InMemoryStore, RedisStore};

    fn boot_recovery_bundle() -> AuthoritativeBundle {
        let owner = OwnerId::new("boot_recovery_test").unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("create_marker").unwrap(),
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
                    DefinitionId::new("marker_created").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([71; 16]);
        let principal = PrincipalId::Object { id: GOD };
        AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([72; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: owner.clone(),
                    namespace: CanonicalId::new("create_marker").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([73; 32]),
            request_hash: CanonicalHash::from_bytes([74; 32]),
            plan_hash: CanonicalHash::from_bytes([75; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome.clone()],
            writes: vec![
                ReplayWrite::ObjectSet {
                    object: Obj::new(ObjId(2), ObjKind::Thing, "recovered marker", GOD, 0),
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
    async fn writer_lanes_reserve_capacity_and_merge_by_acceptance_order() {
        let metrics = Arc::new(Metrics::new());
        let (writer, mut receivers) = store_writer_channels(Arc::clone(&metrics));
        for serial in 0..WRITER_LEGACY_QUEUE {
            writer
                .send(StoreJob::Write {
                    batch: WriteBatch {
                        serial: u64::try_from(serial).unwrap(),
                        ..WriteBatch::default()
                    },
                    notify: None,
                })
                .await
                .unwrap();
        }

        // This producer is blocked on legacy capacity and therefore owns no
        // global acceptance order yet.
        let blocked_writer = writer.clone();
        let blocked_legacy = tokio::spawn(async move {
            blocked_writer
                .send(StoreJob::Write {
                    batch: WriteBatch {
                        serial: 10_000,
                        ..WriteBatch::default()
                    },
                    notify: None,
                })
                .await
        });
        tokio::task::yield_now().await;
        assert!(!blocked_legacy.is_finished());
        receivers.drain_ready();
        tokio::task::yield_now().await;
        assert!(
            !blocked_legacy.is_finished(),
            "moving jobs into the order buffer must not release lane capacity"
        );

        let (authoritative_ownership, _authoritative_owned) = oneshot::channel();
        let (authoritative_notify, _authoritative_receipt) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_secs(1),
            writer.send(StoreJob::Authoritative {
                bundle: Box::new(boot_recovery_bundle()),
                ownership: authoritative_ownership,
                notify: authoritative_notify,
            }),
        )
        .await
        .expect("authoritative lane must admit while legacy is full")
        .unwrap();
        let (control_done, _control_receipt) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_secs(1),
            writer.send(StoreJob::Bgsave { done: control_done }),
        )
        .await
        .expect("control lane must admit while legacy is full")
        .unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot.store_legacy_queue_depth,
            i64::try_from(WRITER_LEGACY_QUEUE).unwrap()
        );
        assert_eq!(snapshot.store_authoritative_queue_depth, 1);
        assert_eq!(snapshot.store_control_queue_depth, 1);

        // Independent admission does not weaken persistence ordering: the two
        // reserved-lane jobs follow all previously accepted legacy work, and
        // precede the legacy sender that was waiting for capacity.
        for expected in 0..WRITER_LEGACY_QUEUE {
            let StoreJob::Write { batch, .. } = receivers.recv().await.unwrap().job else {
                panic!("previously accepted legacy work must remain first");
            };
            assert_eq!(batch.serial, u64::try_from(expected).unwrap());
        }
        assert!(matches!(
            receivers.recv().await.unwrap().job,
            StoreJob::Authoritative { .. }
        ));
        assert!(matches!(
            receivers.recv().await.unwrap().job,
            StoreJob::Bgsave { .. }
        ));
        blocked_legacy.await.unwrap().unwrap();
        let StoreJob::Write { batch, .. } = receivers.recv().await.unwrap().job else {
            panic!("capacity-blocked legacy sender must follow accepted reserved-lane work");
        };
        assert_eq!(batch.serial, 10_000);
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

    #[tokio::test]
    async fn authoritative_recovery_barrier_finishes_before_world_reload() {
        let store = InMemoryStore::new();
        load_or_seed_world(&store).await.unwrap();
        let bundle = boot_recovery_bundle();
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();

        let before = store.load_world().await.unwrap().unwrap();
        assert!(before.objects.iter().all(|object| object.id != ObjId(2)));
        assert!(matches!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::PendingExact
        ));

        let report = reconcile_authoritative_for_writable_boot(&store)
            .await
            .unwrap();
        assert_eq!(
            report,
            AuthoritativeBootRecoveryReport {
                indexed: 1,
                retained: 1,
                retained_bytes: serde_json::to_string(&bundle).unwrap().len(),
                retention_counters_repaired: false,
                reconciled: 1,
                already_finalized: 0,
                checkpointed: 1,
                checkpoint_deferred_outbox: 0,
                scan_pages: 1,
            }
        );
        let world = load_or_seed_world(&store).await.unwrap();
        assert_eq!(world.next_id(), 3);
        assert_eq!(world.get(ObjId(2)).unwrap().name, "recovered marker");

        // Rebooting is an exact no-op: the compact checkpoint is not part of
        // the full-intent recovery scan.
        let second = reconcile_authoritative_for_writable_boot(&store)
            .await
            .unwrap();
        assert_eq!(second.indexed, 0);
        assert_eq!(second.retained, 0);
        assert_eq!(second.reconciled, 0);
        assert_eq!(second.already_finalized, 0);
        assert_eq!(second.checkpointed, 0);
    }

    #[tokio::test]
    async fn authoritative_writer_reserves_reconciles_and_returns_exact_receipt() {
        let store = InMemoryStore::new();
        load_or_seed_world(&store).await.unwrap();
        let bundle = boot_recovery_bundle();
        let (signals, _signal_rx) = crate::world_task::writer_signal_channel();
        let mut healthy = true;

        let spool = QuarantineSpool::in_memory();
        let (ownership, owned) = oneshot::channel();
        let receipt = commit_authoritative_with_retry(
            &store,
            bundle.clone(),
            ownership,
            &mut healthy,
            &signals,
            &spool,
        )
        .await
        .unwrap();

        let owned = owned.await.unwrap().unwrap();
        assert_eq!(owned.token, bundle.token());
        assert_eq!(owned.bundle_hash, bundle.bundle_hash());
        assert!(receipt.matches_bundle(&bundle));
        assert!(healthy);
        assert_eq!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::FinalizedExact(Box::new(receipt))
        );
        assert!(
            store
                .pending_authoritative_intents()
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
            "the writer checkpoints and releases finalized replay material"
        );
    }

    #[tokio::test]
    async fn authoritative_writer_quarantines_whole_bundle_after_bounded_poison() {
        let store = InMemoryStore::new();
        load_or_seed_world(&store).await.unwrap();
        let bundle = boot_recovery_bundle();

        // Model a crash/interleaving that left this exact immutable intent
        // pending while durable allocator state advanced beyond its post-image.
        // Exact replay can never be valid, so each reconciliation fails
        // deterministically while status remains PendingExact.
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();
        store
            .write(mush_store::WriteBatch {
                serial: 2,
                next_id: Some(4),
                durable: true,
                ..mush_store::WriteBatch::default()
            })
            .await
            .unwrap();

        let (signals, _signal_rx) = crate::world_task::writer_signal_channel();
        let mut healthy = true;
        let mut spool = QuarantineSpool::in_memory();
        let (ownership, owned) = oneshot::channel();
        let error = commit_authoritative_with_retry(
            &store,
            bundle.clone(),
            ownership,
            &mut healthy,
            &signals,
            &spool,
        )
        .await
        .unwrap_err();
        assert_eq!(owned.await.unwrap().unwrap().token, bundle.token());

        assert!(matches!(
            error,
            AuthoritativeCommitFailure::Integrity(ref detail)
                if detail.contains("quarantined")
                    && detail.contains("after 3 deterministic reconciliation failures")
        ));
        assert!(matches!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::QuarantinedExact(receipt)
                if receipt.matches_bundle(&bundle)
                    && receipt.evidence_hash() == authoritative_bundle_evidence_hash(&bundle)
        ));
        assert!(
            store
                .scan_authoritative_intents(None, 1)
                .await
                .unwrap()
                .intents
                .is_empty(),
            "the independent spool now permits bounded full-intent release"
        );

        // The independently retained whole-bundle record participates in the
        // same boot gate. Its exact reconcile is still poison, so it remains
        // active rather than being archived on mere token/hash presence.
        let report = spool.reconcile_exact_matches(&store).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.resolved, 0);
        assert_eq!(report.remaining.unresolved_records, 1);
        assert!(spool.require_resolved_for_boot().is_err());
    }

    #[tokio::test]
    #[ignore = "requires live Redis; CI runs this proof explicitly"]
    async fn live_redis_wrong_type_bundle_quarantines_repairs_and_recovers() {
        use fred::interfaces::KeysInterface as _;

        let url = std::env::var("REDIS_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .expect("REDIS_URL must be set when explicitly running the live Redis proof");
        let store = RedisStore::connect(&url)
            .await
            .expect("connect to disposable live Redis");
        store
            .wipe_namespaces()
            .await
            .expect("wipe disposable Redis namespaces");
        load_or_seed_world(&store).await.unwrap();
        let bundle = boot_recovery_bundle();
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .expect("reserve exact intent before fault injection");

        // Generic SET gives obj:2 the Redis string type. The authoritative
        // JSON.SET replay is then deterministic WRONGTYPE poison while the
        // complete exact intent remains independently retained in Redis.
        let _: () = store
            .client()
            .set("obj:2", "wrong type", None, None, false)
            .await
            .expect("install deterministic wrong-type target");

        let root = std::env::temp_dir().join(format!(
            "rustmush-k3-quarantine-live-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let spool_path = root.join("quarantine");
        let mut spool = QuarantineSpool::open(&spool_path).unwrap();
        let (signals, _signal_rx) = crate::world_task::writer_signal_channel();
        let mut healthy = true;
        let (ownership, owned) = oneshot::channel();
        let error = commit_authoritative_with_retry(
            &store,
            bundle.clone(),
            ownership,
            &mut healthy,
            &signals,
            &spool,
        )
        .await
        .unwrap_err();
        assert_eq!(owned.await.unwrap().unwrap().token, bundle.token());
        assert!(matches!(error, AuthoritativeCommitFailure::Integrity(_)));
        assert!(matches!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::QuarantinedExact(receipt)
                if receipt.matches_bundle(&bundle)
                    && receipt.evidence_hash() == authoritative_bundle_evidence_hash(&bundle)
        ));

        let files: Vec<_> = std::fs::read_dir(&spool_path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        assert_eq!(files.len(), 1);
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
        assert_eq!(
            record["record_version"],
            crate::quarantine::QUARANTINE_RECORD_VERSION
        );
        assert_eq!(record["content"]["kind"], "authoritative_bundle");
        assert_eq!(
            record["content"]["bundle"],
            serde_json::to_value(&bundle).unwrap(),
            "the dead-letter retains the whole immutable bundle"
        );

        let deleted: i64 = store.client().del("obj:2").await.unwrap();
        assert_eq!(deleted, 1);
        let report = spool.reconcile_exact_matches(&store).await.unwrap();
        assert_eq!(report.resolved, 1);
        assert_eq!(report.remaining.unresolved_records, 0);
        assert!(matches!(
            store
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::FinalizedExact(receipt)
                if receipt.token() == bundle.token()
                    && receipt.bundle_hash() == bundle.bundle_hash()
        ));

        drop(spool);
        store.wipe_namespaces().await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
