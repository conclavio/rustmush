//! Chunked append-only ledgers — the durable substrate for **authoritative**
//! event logs (scene event streams, the canon event ledger, status-grant
//! histories). The first consumer is the scene event log; this module owns
//! only the generic machinery, never any payload schema.
//!
//! ## Why not [`ModuleWrite::StreamAppend`](crate::module::ModuleWrite)
//!
//! Redis streams (`XADD`) are the wrong primitive for a log that must be
//! authoritative and exactly-once:
//!
//! - **Retries double-append.** The store writer retries a whole batch on an
//!   ambiguous failure (EXEC applied, reply lost); an `XADD *` has a
//!   server-assigned id, so the retry appends the entry *again*. A log that
//!   backs "who witnessed what" cannot tolerate phantom duplicates.
//! - **Deterministic failure drops entries permanently.** A batch that fails
//!   non-transiently is dropped; its stream appends are simply lost, with no
//!   self-heal (unlike a full-object upsert, which the next write restates).
//! - **Migrations skip streams.** The per-namespace migration machinery
//!   rewrites JSON documents only, so a stream's entry schema can never evolve.
//! - **Streams trim.** `StreamAppend` is mandatory-capped; an authoritative
//!   log must not silently discard its own history.
//!
//! ## The chunked-document design
//!
//! Entries accumulate in an in-RAM **open chunk**. Each successful
//! [`Ledger::apply_prepared`] returns one whole-document
//! [`ModuleWrite::JsonSet`] rewriting that single chunk
//! (`log:{id}:{chunk}`). A whole-doc set is:
//!
//! - **idempotent** — a retried batch rewrites the same key to the same bytes,
//!   so a duplicated commit is a no-op, not a phantom entry;
//! - **migration-covered** — a chunk is an ordinary JSON doc;
//! - **non-trimming** — nothing is ever discarded.
//!
//! When the open chunk passes [`Ledger::chunk_target_bytes`] it **seals**: the
//! append that fills it writes the chunk with `sealed: true`, and the next
//! append opens a fresh chunk key. A sealed chunk is immutable history — once
//! its write is confirmed durable it is **never rewritten again** — so the
//! per-append cost is bounded by one open chunk, not the whole log. (Between
//! sealing and that durable ack a seal may be re-emitted a few byte-identical
//! times; see *Durability window*.)
//!
//! ## What lives in RAM
//!
//! Only the **open tail** of each log (the current unsealed chunk plus the
//! running counters). Sealed chunks are on disk; reading old history pages them
//! in through the module store-read seam (a later prerequisite). Until that
//! lands, [`Ledger`] exposes only the open tail — enough to keep appending, and
//! enough for the common case of an *active* scene, whose events are the tail.
//!
//! ## Durability window (closed by ack-gated re-emit)
//!
//! An open-chunk rewrite that is dropped by the persistence layer self-heals:
//! the entries are still in RAM and the log's next append re-persists the whole
//! open chunk. The one case that does *not* self-heal on its own is a dropped
//! **seal-and-roll-over** append — the in-RAM ledger has already advanced to the
//! next chunk, so ordinary appends never restate the sealed chunk again. This
//! can only happen on a *deterministic* store rejection (transient failures
//! retry the same batch until it lands), often as collateral when some other
//! op in the same batch is the poison.
//!
//! The ledger remains payload-agnostic and non-blocking. The sealed-tail window
//! is closed *above* it, in the world loop, by a **retained-write table**: every
//! sealed-chunk `JsonSet` is recognized by its bytes (a `log:` key with
//! `sealed: true` — see [`is_log_key`]) and re-emitted, byte-identical, on every
//! subsequent batch until a durable-commit ack is observed for the batch that
//! carried it. So a dropped seal is simply rewritten next flush; the loss window
//! is gone without any ledger API change. The consequence for this module: a
//! sealed chunk may be written *a few identical times, then never* — the
//! immutability guarantee holds strictly **post-ack**, not per-emit. Retention
//! is a property of the op bytes, so no consumer opts in and no consumer can
//! forget to; this decouples the durability fix from the (still-pending) cold
//! store-read seam, which serves audit reads and is independent of write acks.
//! [`scan_contiguity`] surfaces any hole that predates this machinery at boot.
//!
//! ## Transaction discipline (the one rule consumers must follow)
//!
//! [`Ledger::prepare_append`] performs every fallible size, encoding, and
//! arithmetic check without mutation. The caller validates its whole action,
//! then [`Ledger::apply_prepared`] installs the tail and returns the persist op
//! to queue in the same command. A stale-plan refusal also changes nothing.
//! Applying still belongs on the terminal success path: a handler that later
//! returns `Err` has deferred writes truncated but RAM mutations retained. The
//! testkit boundary rig asserts that the action mutation and ledger op fold
//! into one write batch for well-behaved handlers.
//!
//! The retained-write table above does **not** relax this. It re-emits ops that
//! *were* built, so a seal op truncated on an `Err` path never enters it.
//! Prepare/validate/apply ordering prevents that phantom-tail case; the
//! retained-write table is not a substitute for transaction discipline.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::module::ModuleWrite;

/// The reserved key prefix (within a module namespace) under which ledger
/// chunk documents live: `log:{log_id}:{chunk_seq}`. Consumers must not use
/// this prefix for ordinary documents — a later boot optimization skips it in
/// the namespace scan so cold chunks are not all loaded into RAM.
pub const LOG_PREFIX: &str = "log:";

/// Default open-chunk size target in bytes. A chunk seals once its serialized
/// entries pass this, so each chunk holds roughly this much (plus the one entry
/// that tipped it over). Small enough to bound the per-append whole-doc
/// rewrite; large enough that chunk documents are not tiny.
pub const DEFAULT_CHUNK_TARGET_BYTES: usize = 16 * 1024;

/// A hard safety ceiling on a prospective chunk's exact serialized size,
/// far above [`DEFAULT_CHUNK_TARGET_BYTES`] (normal chunks seal at the target,
/// nowhere near this) and far below any practical store value limit. A chunk
/// that approaches it is a **probable poison** — a single pathologically large
/// entry, or a mis-set chunk target — that the store would deterministically
/// reject. [`Ledger::prepare_append`] refuses it before mutating RAM or
/// producing a persistence write.
pub const MAX_SAFE_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Hard bound on a caller-provided logical log id. Persisted keys add only the
/// fixed prefix, separators, and one decimal `u64` to this value.
pub const MAX_LEDGER_LOG_ID_BYTES: usize = 256;

/// A movement may enter one transcript and leave another. This bound is not
/// a general multi-log transaction facility.
pub const MAX_LEDGER_BATCH_APPENDS: usize = 2;

/// Maximum number of individual integrity findings retained in a checked
/// restore error. The total count is reported separately, so a corrupt store
/// cannot turn diagnostics themselves into an unbounded allocation.
pub const MAX_RESTORE_BREACH_DETAILS: usize = 256;

/// One appended entry: a monotonic per-log sequence number plus an opaque
/// payload the consumer defines (a scene event, a canon event, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    /// Position in the log, 0-based, strictly increasing, gap-free.
    pub seq: u64,
    /// The consumer's payload.
    pub value: serde_json::Value,
}

/// One persisted chunk document (`log:{id}:{chunk}`): a contiguous run of
/// entries and whether the chunk is sealed (full and immutable).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerChunk {
    /// Chunk index within the log, 0-based.
    pub chunk: u64,
    /// A sealed chunk is at capacity and will never be rewritten.
    pub sealed: bool,
    /// The entries in this chunk, in append order.
    pub entries: Vec<LedgerEntry>,
}

/// One persisted ledger chunk document **as the store holds it**: its
/// namespace-relative key (`log:{id}:{chunk}`) and the raw JSON text. Boot hands
/// ledger chunks to modules in this form and never as a parsed
/// `serde_json::Value` (J1, doc 12 F9): a chunk's boot footprint is its byte
/// size, not a `Value` tree several times larger, and the one inspection every
/// consumer shares parses each chunk exactly once.
pub type LedgerDoc = (SmolStr, String);

/// A chunk document whose entry payloads are decoded straight into `T` — the
/// consumer's own record type — from the raw text, with no `Value` in between
/// (J1). Strict like [`LedgerChunk`]: an unknown field anywhere is a decode
/// error, so the shape a chunk is validated by is the shape it is consumed by.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerChunkOf<T> {
    /// Chunk index within the log, 0-based.
    pub chunk: u64,
    /// A sealed chunk is at capacity and will never be rewritten.
    pub sealed: bool,
    /// The entries in this chunk, in append order.
    pub entries: Vec<LedgerEntryOf<T>>,
}

/// One entry of a [`LedgerChunkOf`]: the sequence number and the typed payload.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntryOf<T> {
    /// Position in the log, 0-based, strictly increasing, gap-free.
    pub seq: u64,
    /// The consumer's payload, typed.
    pub value: T,
}

/// Decode one chunk document's entries straight to `(seq, T)` pairs, in
/// append order. The caller has validated the chunk through
/// [`inspect_ledger`] (or [`Ledger::inspect`]) already; this is the typed pass a
/// consumer makes over the chunks the inspection ordered for it
/// ([`LedgerHistory::chunk_docs`]).
///
/// # Errors
/// The document is not a chunk of `T` entries (a payload of another shape, or
/// a field the strict forms do not know).
pub fn typed_entries<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<Vec<(u64, T)>, serde_json::Error> {
    let chunk: LedgerChunkOf<T> = serde_json::from_str(text)?;
    Ok(chunk
        .entries
        .into_iter()
        .map(|entry| (entry.seq, entry.value))
        .collect())
}

/// Decode one chunk document as the opaque [`LedgerChunk`] (payloads as
/// `Value`), from its raw text.
///
/// # Errors
/// The text is not a well-formed chunk document.
pub fn parse_chunk(text: &str) -> Result<LedgerChunk, serde_json::Error> {
    serde_json::from_str(text)
}

/// The in-RAM open tail of one log.
#[derive(Clone, Debug, PartialEq)]
struct LogTail {
    /// The chunk index currently being appended to.
    open_chunk: u64,
    /// Entries buffered in the open chunk (not yet sealed).
    open_entries: Vec<LedgerEntry>,
    /// The next sequence number to hand out for this log.
    next_seq: u64,
    /// Approximate serialized size of `open_entries`, for the seal decision.
    open_bytes: usize,
}

/// A validated append plan. Its fields are deliberately private: only
/// [`Ledger::prepare_append`] can construct a plan that
/// [`Ledger::apply_prepared`] accepts.
///
/// Preparing is non-mutating. Applying consumes the plan and performs only an
/// exact pre-state comparison before installing the already-built tail and
/// returning the already-built persistence write.
#[derive(Debug)]
#[must_use = "a prepared append has no effect until Ledger::apply_prepared consumes it"]
pub struct PreparedLedgerAppend {
    log_id: SmolStr,
    chunk_target_bytes: usize,
    expected_tail: Option<LogTail>,
    next_tail: LogTail,
    seq: u64,
    chunk: LedgerChunk,
    write: ModuleWrite,
    resident_limit: Option<usize>,
}

impl PreparedLedgerAppend {
    /// Sequence number reserved by this plan.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Build the exact replayable Ledger write for an authoritative bundle
    /// without consuming this plan. The caller retains the same plan until
    /// the bundle's exact receipt is acknowledged, then installs it through
    /// [`Ledger::apply_prepared`].
    pub fn authoritative_replay_write(
        &self,
        namespace: crate::mechanics::OwnerId,
    ) -> Result<
        crate::authoritative::ReplayWrite,
        crate::authoritative::AuthoritativeLedgerWriteError,
    > {
        Ok(crate::authoritative::ReplayWrite::LedgerChunkSet {
            write: crate::authoritative::AuthoritativeLedgerWrite::new(
                namespace,
                self.log_id.as_str(),
                self.chunk.clone(),
            )?,
        })
    }
}

// Own the validated plan and keep its exact destination exclusively borrowed
// until application or discard. No authority is conveyed by this private type:
// the ledger's exact tail/policy fence is not an owner or slot identity check.
#[must_use = "binding does not append; apply consumes the validated plan"]
/// Exact prepared append bound to its unchanged tail. Dropping this value does
/// nothing; consuming it performs only the already-prepared insertion/write.
pub struct BoundLedgerAppend<'a> {
    ledger: &'a mut Ledger,
    prepared: PreparedLedgerAppend,
}

impl BoundLedgerAppend<'_> {
    pub fn apply(self) -> (u64, ModuleWrite) {
        let Self { ledger, prepared } = self;
        ledger.logs.insert(prepared.log_id, prepared.next_tail);
        (prepared.seq, prepared.write)
    }
}

/// At most two distinct log tails, all validated before either can change.
/// Dropping the binding leaves every tail and sequence unchanged.
#[must_use = "binding does not append; apply consumes every validated plan"]
pub struct BoundLedgerBatch<'a> {
    ledger: &'a mut Ledger,
    tails: Vec<(SmolStr, LogTail)>,
    outputs: Vec<(u64, ModuleWrite)>,
}

impl BoundLedgerBatch<'_> {
    /// Publish the already validated tails and return the original writes in
    /// caller order. All output allocation completed during binding.
    pub fn apply(self) -> Vec<(u64, ModuleWrite)> {
        for (id, tail) in self.tails {
            self.ledger.logs.insert(id, tail);
        }
        self.outputs
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerBatchError {
    #[error("ledger batch exceeds its two-append bound")]
    TooMany,
    #[error("ledger batch names the same log twice")]
    DuplicateLog,
    #[error("ledger batch prestate exceeds its byte or nesting bound")]
    InputLimit,
    #[error("prepared ledger batch is stale")]
    StalePrestate,
}

/// Why a ledger append could not be prepared. Every variant is detected
/// before the ledger is mutated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerAppendError {
    /// Empty ids cannot round-trip through `log:{id}:{chunk}` keys.
    #[error("ledger log id must not be empty")]
    EmptyLogId,
    /// A log id would create an unreasonably large persistence key.
    #[error("ledger log id is {bytes} bytes, exceeding the {max}-byte limit")]
    LogIdTooLong {
        /// UTF-8 encoded id length.
        bytes: usize,
        /// Configured hard ceiling.
        max: usize,
    },
    /// The entry could not be encoded as JSON.
    #[error("ledger entry could not be encoded")]
    EntryEncoding,
    /// The complete prospective chunk could not be encoded as JSON.
    #[error("ledger chunk could not be encoded")]
    ChunkEncoding,
    /// One entry alone exceeds the hard persistence ceiling.
    #[error("ledger entry is {bytes} bytes, exceeding the {max}-byte limit")]
    EntryTooLarge {
        /// Encoded bytes observed before the limit was crossed. This is at
        /// least `max + 1`, not necessarily the full unbounded payload size.
        bytes: usize,
        /// Configured hard ceiling.
        max: usize,
    },
    /// The complete prospective chunk exceeds the hard persistence ceiling.
    #[error("ledger chunk is {bytes} bytes, exceeding the {max}-byte limit")]
    ChunkTooLarge {
        /// Encoded bytes observed before the limit was crossed.
        bytes: usize,
        /// Configured hard ceiling.
        max: usize,
    },
    /// The next per-log sequence number cannot be represented.
    #[error("ledger sequence number overflow")]
    SequenceOverflow,
    /// Sealing this chunk cannot advance to a representable next chunk.
    #[error("ledger chunk index overflow")]
    ChunkIndexOverflow,
    /// The in-memory encoded-entry byte counter cannot be advanced safely.
    #[error("ledger chunk byte counter overflow")]
    ByteCountOverflow,
    /// The selected resident tail could not be safely inspected before clone.
    #[error("ledger resident tail exceeds its {max}-byte or nesting bound")]
    ResidentLimit { max: usize },
    /// New bounded callers cannot introduce a recursively unsafe value.
    #[error("ledger entry exceeds the bounded input shape")]
    InputLimit,
}

/// The sole failure mode after a valid append has been prepared.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerApplyError {
    /// The log (or its chunk policy) changed after preparation. The plan was
    /// not applied and the ledger remains byte-for-byte unchanged.
    #[error("prepared ledger append is stale")]
    StalePrestate,
}

/// Summary of a successful authoritative restore.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LedgerRestoreReport {
    /// Number of independent logs restored.
    pub logs: usize,
    /// Number of authoritative chunk documents validated.
    pub chunks: usize,
    /// Number of entries validated across those chunks.
    pub entries: usize,
}

/// A checked authoritative restore refusal. Details are bounded while
/// [`Self::total_breaches`] records the complete number discovered.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("authoritative ledger restore refused: {total_breaches} integrity breach(es)")]
pub struct LedgerRestoreError {
    total_breaches: usize,
    details_truncated: bool,
    breaches: Vec<ContiguityBreach>,
}

impl LedgerRestoreError {
    /// Total findings, including details omitted by the diagnostic cap.
    #[must_use]
    pub fn total_breaches(&self) -> usize {
        self.total_breaches
    }

    /// Bounded retained findings.
    #[must_use]
    pub fn breaches(&self) -> &[ContiguityBreach] {
        &self.breaches
    }

    /// Whether additional findings were omitted.
    #[must_use]
    pub fn details_truncated(&self) -> bool {
        self.details_truncated
    }
}

impl LogTail {
    fn fresh() -> LogTail {
        LogTail {
            open_chunk: 0,
            open_entries: Vec::new(),
            next_seq: 0,
            open_bytes: 0,
        }
    }
}

/// A set of chunked append-only logs, held in a module's state slot and
/// mirrored write-through by the `log:*` documents in the module's namespace.
/// One [`Ledger`] can carry many logs, keyed by a caller-chosen id (e.g. a
/// scene's durable id); each is independent.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledger {
    logs: BTreeMap<SmolStr, LogTail>,
    chunk_target_bytes: usize,
}

impl Default for Ledger {
    fn default() -> Ledger {
        Ledger::new()
    }
}

impl Ledger {
    /// A ledger with the [`DEFAULT_CHUNK_TARGET_BYTES`] seal threshold.
    #[must_use]
    pub fn new() -> Ledger {
        Ledger {
            logs: BTreeMap::new(),
            chunk_target_bytes: DEFAULT_CHUNK_TARGET_BYTES,
        }
    }

    /// A ledger with a custom seal threshold (tests, or logs with unusually
    /// large or small entries). A threshold of 0 seals every chunk after one
    /// entry (one entry per chunk).
    #[must_use]
    pub fn with_chunk_target(chunk_target_bytes: usize) -> Ledger {
        Ledger {
            logs: BTreeMap::new(),
            chunk_target_bytes,
        }
    }

    /// The seal threshold in bytes.
    #[must_use]
    pub fn chunk_target_bytes(&self) -> usize {
        self.chunk_target_bytes
    }

    /// Bound the complete resident tail set before a caller clones its owning
    /// state. This counts borrowed entries with a streaming encoder; it neither
    /// clones log values nor reads sealed history from storage. The byte bound
    /// covers serialized resident data, while `max_logs` bounds map overhead.
    #[must_use]
    pub fn resident_within_bounds(&self, max_logs: usize, max_bytes: usize) -> bool {
        if self.logs.len() > max_logs {
            return false;
        }
        let Ok(mut bytes) = encoded_len_bounded(&self.chunk_target_bytes, max_bytes) else {
            return false;
        };
        for (id, tail) in &self.logs {
            let Some(remaining) = max_bytes.checked_sub(bytes) else {
                return false;
            };
            let resident = (
                id,
                tail.open_chunk,
                &tail.open_entries,
                tail.next_seq,
                tail.open_bytes,
            );
            let Ok(size) = encoded_len_bounded(&resident, remaining) else {
                return false;
            };
            bytes += size;
        }
        true
    }

    /// Prepare one append after bounding only its selected resident tail,
    /// before the ordinary preparation clones it. The requested limit can
    /// tighten, but never widen, the hard chunk ceiling. Unrelated logs are
    /// neither read nor cloned. The same limit is retained for batch binding.
    pub fn prepare_append_bounded(
        &self,
        log_id: &str,
        value: serde_json::Value,
        max_resident_bytes: usize,
    ) -> Result<PreparedLedgerAppend, LedgerAppendError> {
        if log_id.is_empty() {
            return Err(LedgerAppendError::EmptyLogId);
        }
        if log_id.len() > MAX_LEDGER_LOG_ID_BYTES {
            return Err(LedgerAppendError::LogIdTooLong {
                bytes: log_id.len(),
                max: MAX_LEDGER_LOG_ID_BYTES,
            });
        }
        let max = max_resident_bytes.min(MAX_SAFE_CHUNK_BYTES);
        if !bounded_tail(log_id, self.chunk_target_bytes, self.logs.get(log_id), max) {
            return Err(LedgerAppendError::ResidentLimit { max });
        }
        let mut entry_budget = MAX_SAFE_CHUNK_BYTES;
        if !bounded_value(&value, &mut entry_budget) {
            return Err(LedgerAppendError::InputLimit);
        }
        let mut prepared = self.prepare_append(log_id, value)?;
        prepared.resident_limit = Some(max);
        Ok(prepared)
    }

    /// Validate and fully construct an append without changing the ledger.
    ///
    /// Entry encoding, exact prospective chunk encoding, both hard byte
    /// ceilings, sequence advancement, chunk rollover, and byte accounting are
    /// all checked here. A returned plan therefore contains no fallible work;
    /// [`apply_prepared`](Self::apply_prepared) only verifies that its exact
    /// pre-state is still current and installs it.
    pub fn prepare_append(
        &self,
        log_id: &str,
        value: serde_json::Value,
    ) -> Result<PreparedLedgerAppend, LedgerAppendError> {
        if log_id.is_empty() {
            return Err(LedgerAppendError::EmptyLogId);
        }
        if log_id.len() > MAX_LEDGER_LOG_ID_BYTES {
            return Err(LedgerAppendError::LogIdTooLong {
                bytes: log_id.len(),
                max: MAX_LEDGER_LOG_ID_BYTES,
            });
        }

        let expected_tail = self.logs.get(log_id).cloned();
        let mut next_tail = expected_tail.clone().unwrap_or_else(LogTail::fresh);
        let seq = next_tail.next_seq;
        next_tail.next_seq = seq
            .checked_add(1)
            .ok_or(LedgerAppendError::SequenceOverflow)?;

        let entry = LedgerEntry { seq, value };
        let entry_bytes = match encoded_len_bounded(&entry, MAX_SAFE_CHUNK_BYTES) {
            Ok(bytes) => bytes,
            Err(BoundedEncodeError::Limit { observed }) => {
                return Err(LedgerAppendError::EntryTooLarge {
                    bytes: observed,
                    max: MAX_SAFE_CHUNK_BYTES,
                });
            }
            Err(BoundedEncodeError::Encoding) => {
                return Err(LedgerAppendError::EntryEncoding);
            }
        };
        next_tail.open_bytes = next_tail
            .open_bytes
            .checked_add(entry_bytes)
            .ok_or(LedgerAppendError::ByteCountOverflow)?;
        next_tail.open_entries.push(entry);

        let sealed = next_tail.open_bytes >= self.chunk_target_bytes;
        let chunk = next_tail.open_chunk;
        // Validate rollover before constructing any state that could be
        // installed. Even a seal at u64::MAX is rejected without mutation.
        let rolled_chunk = if sealed {
            Some(
                chunk
                    .checked_add(1)
                    .ok_or(LedgerAppendError::ChunkIndexOverflow)?,
            )
        } else {
            None
        };
        let document = LedgerChunk {
            chunk,
            sealed,
            entries: next_tail.open_entries.clone(),
        };
        match encoded_len_bounded(&document, MAX_SAFE_CHUNK_BYTES) {
            Ok(_) => {}
            Err(BoundedEncodeError::Limit { observed }) => {
                return Err(LedgerAppendError::ChunkTooLarge {
                    bytes: observed,
                    max: MAX_SAFE_CHUNK_BYTES,
                });
            }
            Err(BoundedEncodeError::Encoding) => {
                return Err(LedgerAppendError::ChunkEncoding);
            }
        }
        let value =
            serde_json::to_value(&document).map_err(|_| LedgerAppendError::ChunkEncoding)?;
        let write = ModuleWrite::JsonSet {
            key: chunk_key(log_id, chunk),
            value,
        };

        if let Some(next_chunk) = rolled_chunk {
            next_tail.open_chunk = next_chunk;
            next_tail.open_entries.clear();
            next_tail.open_bytes = 0;
        }

        Ok(PreparedLedgerAppend {
            log_id: SmolStr::new(log_id),
            chunk_target_bytes: self.chunk_target_bytes,
            expected_tail,
            next_tail,
            seq,
            chunk: document,
            write,
            resident_limit: None,
        })
    }

    /// Install a previously validated append plan.
    ///
    /// On stale pre-state this method performs no mutation. Once the guard
    /// passes, installation and returning the prebuilt write are infallible.
    pub fn apply_prepared(
        &mut self,
        prepared: PreparedLedgerAppend,
    ) -> Result<(u64, ModuleWrite), LedgerApplyError> {
        Ok(self.bind_prepared(prepared)?.apply())
    }

    // Keep the exact pre-state check separate from mutation. A future composed
    // installer must finish every owner's bind before applying any image; this
    // private step is also the existing public path's only admission check.
    /// Bind the same opaque preparation accepted by `apply_prepared`, without
    /// publishing it yet. Used to bind every owner before a composed apply.
    pub fn bind_prepared(
        &mut self,
        prepared: PreparedLedgerAppend,
    ) -> Result<BoundLedgerAppend<'_>, LedgerApplyError> {
        if self.chunk_target_bytes != prepared.chunk_target_bytes
            || self.logs.get(prepared.log_id.as_str()) != prepared.expected_tail.as_ref()
        {
            return Err(LedgerApplyError::StalePrestate);
        }
        Ok(BoundLedgerAppend {
            ledger: self,
            prepared,
        })
    }

    /// Check every selected tail without cloning, allocating postimages, or
    /// mutating. In particular a late malformed value is bounded before Eq.
    pub fn check_prepared_batch(
        &self,
        prepared: &[PreparedLedgerAppend],
    ) -> Result<(), LedgerBatchError> {
        if prepared.len() > MAX_LEDGER_BATCH_APPENDS {
            return Err(LedgerBatchError::TooMany);
        }
        if prepared.len() == 2 && prepared[0].log_id == prepared[1].log_id {
            return Err(LedgerBatchError::DuplicateLog);
        }
        for plan in prepared {
            let limit = plan.resident_limit.unwrap_or(MAX_SAFE_CHUNK_BYTES);
            let live = self.logs.get(plan.log_id.as_str());
            if !bounded_tail(&plan.log_id, self.chunk_target_bytes, live, limit)
                || !bounded_tail(
                    &plan.log_id,
                    plan.chunk_target_bytes,
                    plan.expected_tail.as_ref(),
                    limit,
                )
            {
                return Err(LedgerBatchError::InputLimit);
            }
            if self.chunk_target_bytes != plan.chunk_target_bytes
                || live != plan.expected_tail.as_ref()
            {
                return Err(LedgerBatchError::StalePrestate);
            }
        }
        Ok(())
    }

    /// Bind zero, one, or two distinct log appends. Every input is checked
    /// before a mutable publication capability is returned; empty is inert.
    pub fn bind_prepared_batch(
        &mut self,
        prepared: Vec<PreparedLedgerAppend>,
    ) -> Result<BoundLedgerBatch<'_>, LedgerBatchError> {
        self.check_prepared_batch(&prepared)?;
        let mut tails = Vec::with_capacity(prepared.len());
        let mut outputs = Vec::with_capacity(prepared.len());
        for plan in prepared {
            tails.push((plan.log_id, plan.next_tail));
            outputs.push((plan.seq, plan.write));
        }
        Ok(BoundLedgerBatch {
            ledger: self,
            tails,
            outputs,
        })
    }

    /// Compatibility append for consumers not yet migrated to the
    /// prepare/apply boundary.
    ///
    /// This wrapper panics rather than silently accepting an unsafe append.
    /// Authoritative consumers must handle [`prepare_append`](Self::prepare_append)
    /// errors and apply the plan only when their surrounding action commits.
    #[deprecated(
        since = "0.1.0",
        note = "authoritative callers must use Ledger::prepare_append and Ledger::apply_prepared"
    )]
    pub fn append(&mut self, log_id: &str, value: serde_json::Value) -> (u64, ModuleWrite) {
        let prepared = self
            .prepare_append(log_id, value)
            .expect("deprecated Ledger::append rejected an unsafe append");
        self.apply_prepared(prepared)
            .expect("freshly prepared ledger append cannot be stale")
    }

    /// The entries buffered in a log's open (unsealed) chunk, in order. Sealed
    /// history is on disk, not here — reading it needs the store-read seam.
    /// An unknown log, or one whose open chunk is empty (just rolled over),
    /// yields an empty slice.
    #[must_use]
    pub fn open_entries(&self, log_id: &str) -> &[LedgerEntry] {
        self.logs
            .get(log_id)
            .map_or(&[], |t| t.open_entries.as_slice())
    }

    /// The next sequence number log `log_id` will hand out — i.e. the count of
    /// entries ever appended to it (0 if it has never been touched).
    #[must_use]
    pub fn next_seq(&self, log_id: &str) -> u64 {
        self.logs.get(log_id).map_or(0, |t| t.next_seq)
    }

    /// The open chunk index for `log_id` (0 if untouched).
    #[must_use]
    pub fn open_chunk(&self, log_id: &str) -> u64 {
        self.logs.get(log_id).map_or(0, |t| t.open_chunk)
    }

    /// Inspect `docs` under this ledger's chunk target (the open-tail rule),
    /// so one inspection serves the caller's contiguity decision, the restore
    /// ([`Ledger::restore_from_history`]) and its own typed pass over the chunks
    /// ([`LedgerHistory::chunk_docs`]) — the journal is parsed once at boot (J1).
    #[must_use]
    pub fn inspect(&self, docs: &[LedgerDoc]) -> LedgerHistory {
        inspect_ledger(docs, Some(self.chunk_target_bytes))
    }

    /// [`Ledger::inspect`] over a **narrowed** boot (J4, doc 12 §5.6): for
    /// each log in `floors`, every chunk below the named index may be absent
    /// without being a hole — the loader skipped it on the namespace's boot
    /// manifest. The sequence check starts at the first present chunk of such
    /// a log; the restore is unchanged (it only ever reads the tail).
    #[must_use]
    pub fn inspect_from(&self, docs: &[LedgerDoc], floors: &ChunkFloors) -> LedgerHistory {
        inspect_ledger_from(docs, Some(self.chunk_target_bytes), floors)
    }

    /// Restore the open tail of every log from authoritative boot documents.
    ///
    /// Scans `(key, text)` chunk docs (`log:{id}:{chunk}`), groups by log id,
    /// and rebuilds each log's tail from its **highest** chunk:
    ///
    /// - a sealed tail chunk ⇒ the open chunk is the next index (empty), and
    ///   `next_seq` resumes past the sealed chunk's last entry;
    /// - an unsealed tail chunk ⇒ it *is* the open chunk; its entries and byte
    ///   count are restored so appends continue into it.
    ///
    /// Cold chunks are validated but not retained in RAM. Any malformed key or
    /// document, duplicate/missing chunk, index mismatch, unsealed interior,
    /// sequence discontinuity, unsafe size, or arithmetic overflow refuses the
    /// entire restore. The ledger is unchanged on refusal.
    ///
    /// This is [`Ledger::inspect`] followed by [`Ledger::restore_from_history`];
    /// a caller that also needs the breaches or the chunk order makes the
    /// inspection itself and calls the second half.
    ///
    /// # Errors
    /// Any breach the inspection found; see [`LedgerRestoreError`].
    pub fn restore_from_kv(
        &mut self,
        docs: &[LedgerDoc],
    ) -> Result<LedgerRestoreReport, LedgerRestoreError> {
        let history = self.inspect(docs);
        self.restore_from_history(&history)
    }

    /// Restore every log's open tail from an inspection already made.
    ///
    /// # Errors
    /// The inspection found breaches; the ledger is unchanged.
    pub fn restore_from_history(
        &mut self,
        history: &LedgerHistory,
    ) -> Result<LedgerRestoreReport, LedgerRestoreError> {
        self.restore_logs_from_history(history, |_| true)
    }

    /// [`Ledger::restore_from_history`] for the logs `keep` selects — a scenes
    /// boot validates every scene's history but restores only live scenes'
    /// tails. Breaches anywhere in the inspection still refuse the whole
    /// restore: a corrupt closed log is an integrity breach, not dead RAM.
    ///
    /// # Errors
    /// The inspection found breaches; the ledger is unchanged.
    pub fn restore_logs_from_history(
        &mut self,
        history: &LedgerHistory,
        keep: impl Fn(&str) -> bool,
    ) -> Result<LedgerRestoreReport, LedgerRestoreError> {
        if !history.breaches.is_empty() {
            return Err(bounded_restore_error(&history.breaches));
        }
        // The seal rule for the tails this restore keeps, whichever target the
        // inspection was made under: an unsealed tail already at or over this
        // ledger's target cannot be appended to, so it refuses like any breach.
        let mut over_target = BoundedBreaches::default();
        for (log_id, (_, open_bytes)) in &history.tails {
            if keep(log_id) && *open_bytes >= self.chunk_target_bytes {
                let chunk = history
                    .by_log
                    .get(log_id)
                    .and_then(|chunks| chunks.keys().next_back().copied())
                    .unwrap_or(0);
                over_target.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::UnsealedAtOrOverTarget { chunk },
                });
            }
        }
        if !over_target.is_empty() {
            return Err(bounded_restore_error(&over_target));
        }
        let mut restored = BTreeMap::new();
        let mut chunks_kept = 0usize;
        let mut entries_kept = 0usize;
        for (log_id, chunks) in &history.by_log {
            if !keep(log_id) {
                continue;
            }
            let (&tail_index, tail) = chunks
                .last_key_value()
                .expect("a grouped ledger log has at least one chunk");
            let next_seq = tail
                .seqs
                .last()
                .expect("empty chunks are rejected by inspection")
                .checked_add(1)
                .expect("sequence overflow is rejected by inspection");
            let tail = if tail.sealed {
                LogTail {
                    open_chunk: tail_index
                        .checked_add(1)
                        .expect("chunk overflow is rejected by inspection"),
                    open_entries: Vec::new(),
                    next_seq,
                    open_bytes: 0,
                }
            } else {
                let (parsed, open_bytes) = history
                    .tails
                    .get(log_id)
                    .expect("an unsealed tail is parsed by a clean inspection");
                LogTail {
                    open_chunk: tail_index,
                    open_entries: parsed.entries.clone(),
                    next_seq,
                    open_bytes: *open_bytes,
                }
            };
            chunks_kept = chunks_kept.saturating_add(chunks.len());
            entries_kept =
                entries_kept.saturating_add(chunks.values().map(|c| c.seqs.len()).sum::<usize>());
            restored.insert(log_id.clone(), tail);
        }
        let report = LedgerRestoreReport {
            logs: restored.len(),
            chunks: chunks_kept,
            entries: entries_kept,
        };
        // This is the only mutation point. The supplied authoritative snapshot
        // replaces, rather than merges with, any old RAM state; absent logs
        // must not survive a repeated restore.
        self.logs = restored;
        Ok(report)
    }

    /// Restore whatever tails can be restored, **logging** every breach instead
    /// of refusing — the recovery path for a log whose history is known to be
    /// damaged and whose owner has decided to continue. Never use it on a
    /// money-bearing log.
    pub fn restore_from_kv_lossy(&mut self, docs: &[LedgerDoc]) -> LedgerRestoreReport {
        let history = self.inspect(docs);
        for breach in history.breaches.iter() {
            tracing::error!(
                log_id = %breach.log_id,
                kind = ?breach.kind,
                "ledger: lossy restore ignored authoritative-ledger breach"
            );
        }

        let mut restored_logs = 0usize;
        for (log_id, chunks) in &history.by_log {
            let Some((tail_index, tail)) = chunks.last_key_value() else {
                continue;
            };
            let Some(next_seq) = tail.seqs.last().and_then(|seq| seq.checked_add(1)) else {
                continue;
            };
            let tail = if tail.sealed {
                let Some(open_chunk) = tail_index.checked_add(1) else {
                    continue;
                };
                LogTail {
                    open_chunk,
                    open_entries: Vec::new(),
                    next_seq,
                    open_bytes: 0,
                }
            } else {
                let Some((parsed, open_bytes)) = history.tails.get(log_id) else {
                    continue;
                };
                LogTail {
                    open_chunk: *tail_index,
                    open_entries: parsed.entries.clone(),
                    next_seq,
                    open_bytes: *open_bytes,
                }
            };
            self.logs.insert(log_id.clone(), tail);
            restored_logs = restored_logs.saturating_add(1);
        }
        LedgerRestoreReport {
            logs: restored_logs,
            chunks: history.chunks,
            entries: history.entries,
        }
    }
}

/// The persistence key of one chunk: `log:{log_id}:{chunk}`.
#[must_use]
pub fn chunk_key(log_id: &str, chunk: u64) -> SmolStr {
    SmolStr::new(format!("{LOG_PREFIX}{log_id}:{chunk}"))
}

/// Whether `key` (a module-namespace key, without the `mod:{ns}:` prefix) is a
/// ledger chunk document.
#[must_use]
pub fn is_log_key(key: &str) -> bool {
    parse_chunk_key(key).is_some()
}

/// The entries of the **last `n` sealed chunks** of one log, oldest first
/// (E6b independent verify fold, R3).
///
/// [`Ledger::restore_from_kv`] deliberately keeps only the open tail in RAM, and a
/// *sealed* tail chunk restores as an **empty** open chunk one index further on — so
/// a log whose newest entries happened to fill a chunk just before the process ended
/// comes back with nothing readable at all. For an ordinary append-only log that is
/// the documented memory trade; for one whose newest entries are **evidence a
/// contract settles on**, it is a silent loss with no signal.
///
/// This is the bounded read that closes it: a caller that already holds its boot
/// documents (the module's own `ModuleBootData`) can decode the trailing sealed
/// chunks once, at boot, and keep exactly that much. It validates each candidate the
/// way [`is_sealed_chunk`] does — a malformed or oversized document contributes
/// nothing — and it is `n` chunks, never "all the history you have".
#[must_use]
pub fn recent_sealed(docs: &[LedgerDoc], log_id: &str, n: usize) -> Vec<LedgerEntry> {
    if n == 0 {
        return Vec::new();
    }
    let mut chunks: BTreeMap<u64, Vec<LedgerEntry>> = BTreeMap::new();
    for (key, text) in docs {
        let Some((id, seq)) = parse_chunk_key(key) else {
            continue;
        };
        if id != log_id {
            continue;
        }
        let Some(chunk) = sealed_chunk_of_text(key, text) else {
            continue;
        };
        chunks.insert(seq, chunk.entries);
    }
    let skip = chunks.len().saturating_sub(n);
    chunks.into_values().skip(skip).flatten().collect()
}

/// [`is_sealed_chunk`] over a chunk's raw text, returning the decoded chunk
/// when it is a valid sealed one.
fn sealed_chunk_of_text(key: &str, text: &str) -> Option<LedgerChunk> {
    let (_, key_chunk) = parse_chunk_key(key)?;
    if text.len() > MAX_SAFE_CHUNK_BYTES {
        return None;
    }
    let chunk = parse_chunk(text).ok()?;
    let valid = chunk.sealed
        && chunk.chunk == key_chunk
        && !chunk.entries.is_empty()
        && chunk
            .entries
            .windows(2)
            .all(|pair| pair[0].seq.checked_add(1) == Some(pair[1].seq));
    valid.then_some(chunk)
}

/// Whether a persisted module write `(key, value)` is a **valid sealed** ledger
/// chunk. This is the single-source definition of a retained authoritative
/// write, shared by world retention, isolation, and the quarantine sink.
/// Validation is deliberately complete and bounded: a malformed/oversized
/// document that merely says `sealed:true` must not enter a recovery path whose
/// durable envelope correctly refuses that candidate forever.
#[must_use]
pub fn is_sealed_chunk(key: &str, value: &serde_json::Value) -> bool {
    let Some((_, key_chunk)) = parse_chunk_key(key) else {
        return false;
    };
    if encoded_len_bounded(value, MAX_SAFE_CHUNK_BYTES).is_err() {
        return false;
    }
    let Ok(chunk) = serde_json::from_value::<LedgerChunk>(value.clone()) else {
        return false;
    };
    chunk.sealed
        && chunk.chunk == key_chunk
        && !chunk.entries.is_empty()
        && chunk
            .entries
            .windows(2)
            .all(|pair| pair[0].seq.checked_add(1) == Some(pair[1].seq))
}

/// How many chunks one archive page covers by default (H1, §8 q1: about
/// 64 KiB of entries at the 16 KiB chunk target). Readers page **by chunk
/// index**, newest first — see [`archive_page_range`].
pub const DEFAULT_ARCHIVE_PAGE_CHUNKS: u64 = 4;

/// The chunk range archive page `page` covers over the on-disk chunks
/// `0..=hi_bound`, newest first: page 0 is the newest
/// [`DEFAULT_ARCHIVE_PAGE_CHUNKS`] chunks ending at `hi_bound`, page 1 the
/// ones before it, and so on. `None` once a page would start past the
/// oldest chunk. The last page may be short.
#[must_use]
pub fn archive_page_range(hi_bound: u64, page: u64) -> Option<(u64, u64)> {
    let per = DEFAULT_ARCHIVE_PAGE_CHUNKS;
    let skip = page.checked_mul(per)?;
    if skip > hi_bound {
        return None;
    }
    let hi = hi_bound - skip;
    let lo = hi.saturating_sub(per - 1);
    Some((lo, hi))
}

/// Why a cold page could not be read as a contiguous slice of one log (H1).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerReadError {
    /// No chunk document for `log_id` was in the page at all.
    #[error("no chunk documents for log {log_id:?} in the page")]
    NoChunks {
        /// The requested log.
        log_id: SmolStr,
    },
    /// A returned key was not a chunk of the requested log — the requester
    /// asked for the wrong keys, or the store answered for keys it was not
    /// asked for.
    #[error("key {key:?} is not a chunk document of log {log_id:?}")]
    ForeignKey {
        /// The offending key.
        key: SmolStr,
        /// The requested log.
        log_id: SmolStr,
    },
    /// A chunk document did not decode, or was oversized.
    #[error("chunk {chunk} of log {log_id:?} is malformed")]
    Malformed {
        /// The chunk index from the key.
        chunk: u64,
        /// The requested log.
        log_id: SmolStr,
    },
    /// The key's chunk index and the document's disagree.
    #[error("key names chunk {key_chunk} but the document says chunk {doc_chunk}")]
    IndexMismatch {
        /// The index in the key.
        key_chunk: u64,
        /// The index in the document.
        doc_chunk: u64,
    },
    /// The same chunk index appeared twice in the page.
    #[error("chunk {chunk} appears twice in the page")]
    DuplicateChunk {
        /// The duplicated index.
        chunk: u64,
    },
    /// The page skips a chunk index: a hole in the history the store holds.
    #[error("chunk {expected} is missing from the page (next present is {found})")]
    ChunkGap {
        /// The index that should have followed.
        expected: u64,
        /// The index that did.
        found: u64,
    },
    /// An interior chunk (not the highest in the page) is unsealed — which
    /// the ledger never writes, so this is corruption.
    #[error("interior chunk {chunk} is unsealed")]
    UnsealedInterior {
        /// The unsealed chunk.
        chunk: u64,
    },
    /// A chunk with no entries — the ledger never writes one.
    #[error("chunk {chunk} is empty")]
    EmptyChunk {
        /// The empty chunk.
        chunk: u64,
    },
    /// Entry sequence numbers are not gap-free across the page.
    #[error("entry sequence gap: expected seq {expected}, found {found}")]
    SequenceGap {
        /// The sequence number that should have come next.
        expected: u64,
        /// The one that did.
        found: u64,
    },
}

/// Decode one **cold page** of a log — the `log:{log_id}:{n}` documents a
/// [`Ctx::read_store`](crate::Ctx::read_store) brought back — into an ordered,
/// contiguous run of entries (H1, D-H-2).
///
/// The page unit is the **chunk index**: the caller built its keys with
/// [`chunk_key`] for a range of indices and this validates what came back.
/// Every document must be a chunk of `log_id`; indices must be contiguous
/// (a hole is a [`LedgerReadError::ChunkGap`] — history the store no longer
/// holds, or never did); every chunk but the highest returned must be sealed
/// (a page may end at the open tail, which is legitimately unsealed); no
/// chunk may be empty; and entry sequence numbers must be gap-free across the
/// whole page. Entries come back ordered by `seq`, oldest first.
///
/// Keys the store reported absent are the caller's to drop before calling
/// this (a trailing absent chunk past a hint is normal; an interior one is a
/// gap this reports).
pub fn read_cold_range(
    docs: &[(SmolStr, serde_json::Value)],
    log_id: &str,
) -> Result<Vec<LedgerEntry>, LedgerReadError> {
    let log = SmolStr::new(log_id);
    let mut chunks: BTreeMap<u64, LedgerChunk> = BTreeMap::new();
    for (key, value) in docs {
        let Some((id, key_chunk)) = parse_chunk_key(key) else {
            return Err(LedgerReadError::ForeignKey {
                key: key.clone(),
                log_id: log.clone(),
            });
        };
        if id != log_id {
            return Err(LedgerReadError::ForeignKey {
                key: key.clone(),
                log_id: log.clone(),
            });
        }
        if encoded_len_bounded(value, MAX_SAFE_CHUNK_BYTES).is_err() {
            return Err(LedgerReadError::Malformed {
                chunk: key_chunk,
                log_id: log.clone(),
            });
        }
        let chunk = serde_json::from_value::<LedgerChunk>(value.clone()).map_err(|_| {
            LedgerReadError::Malformed {
                chunk: key_chunk,
                log_id: log.clone(),
            }
        })?;
        if chunk.chunk != key_chunk {
            return Err(LedgerReadError::IndexMismatch {
                key_chunk,
                doc_chunk: chunk.chunk,
            });
        }
        if chunk.entries.is_empty() {
            return Err(LedgerReadError::EmptyChunk { chunk: key_chunk });
        }
        if chunks.insert(key_chunk, chunk).is_some() {
            return Err(LedgerReadError::DuplicateChunk { chunk: key_chunk });
        }
    }
    let Some((&highest, _)) = chunks.last_key_value() else {
        return Err(LedgerReadError::NoChunks { log_id: log });
    };
    let mut expected_chunk: Option<u64> = None;
    let mut expected_seq: Option<u64> = None;
    let mut entries = Vec::new();
    for (&index, chunk) in &chunks {
        if let Some(expected) = expected_chunk
            && index != expected
        {
            return Err(LedgerReadError::ChunkGap {
                expected,
                found: index,
            });
        }
        if index != highest && !chunk.sealed {
            return Err(LedgerReadError::UnsealedInterior { chunk: index });
        }
        for entry in &chunk.entries {
            if let Some(expected) = expected_seq
                && entry.seq != expected
            {
                return Err(LedgerReadError::SequenceGap {
                    expected,
                    found: entry.seq,
                });
            }
            expected_seq = entry.seq.checked_add(1);
            entries.push(entry.clone());
        }
        expected_chunk = index.checked_add(1);
    }
    Ok(entries)
}

/// Parse `log:{log_id}:{chunk}` into `(log_id, chunk)`. The chunk is the final
/// `:`-segment; the log id is everything between the prefix and it (so a log id
/// may itself contain `:`). Returns `None` for any non-chunk key.
/// The `(log id, chunk index)` a `log:{id}:{chunk}` key names, or `None` for
/// any other key — including a `log:` key whose chunk is not spelled
/// canonically (`01` is not chunk 1: one durable identity has one spelling).
#[must_use]
pub fn parse_chunk_key(key: &str) -> Option<(SmolStr, u64)> {
    let rest = key.strip_prefix(LOG_PREFIX)?;
    let (log_id, chunk_text) = rest.rsplit_once(':')?;
    if log_id.is_empty() || log_id.len() > MAX_LEDGER_LOG_ID_BYTES {
        return None;
    }
    let chunk = chunk_text.parse::<u64>().ok()?;
    // One durable identity has one spelling. Accepting `01` as chunk 1 would
    // permit two physical keys to alias the same logical history position.
    if chunk.to_string() != chunk_text {
        return None;
    }
    Some((SmolStr::new(log_id), chunk))
}

#[derive(Debug)]
enum BoundedEncodeError {
    Limit { observed: usize },
    Encoding,
}

struct CountingWriter {
    bytes: usize,
    limit: usize,
    exceeded_at: Option<usize>,
}

impl io::Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(next) = self.bytes.checked_add(buf.len()) else {
            self.exceeded_at = Some(usize::MAX);
            return Err(io::Error::other("encoded length overflow"));
        };
        if next > self.limit {
            self.exceeded_at = Some(next);
            return Err(io::Error::other("encoded value exceeds limit"));
        }
        self.bytes = next;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Count encoded JSON bytes while stopping as soon as `limit` is crossed.
/// This avoids allocating a second unbounded byte vector merely to reject an
/// already-oversized JSON value.
fn encoded_len_bounded<T: Serialize>(value: &T, limit: usize) -> Result<usize, BoundedEncodeError> {
    let mut writer = CountingWriter {
        bytes: 0,
        limit,
        exceeded_at: None,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(_) => match writer.exceeded_at {
            Some(observed) => Err(BoundedEncodeError::Limit { observed }),
            None => Err(BoundedEncodeError::Encoding),
        },
    }
}

// Bounded DFS keeps at most one iterator per JSON level. Charging nodes and
// raw text before serialization prevents a huge frontier or deep recursive
// clone/PartialEq from bypassing an eventual encoded-byte refusal.
fn bounded_value(value: &serde_json::Value, remaining: &mut usize) -> bool {
    enum Frame<'a> {
        Value(&'a serde_json::Value, usize),
        Array(std::slice::Iter<'a, serde_json::Value>, usize),
        Object(serde_json::map::Iter<'a>, usize),
    }
    fn charge(remaining: &mut usize, amount: usize) -> bool {
        match remaining.checked_sub(amount) {
            Some(next) => {
                *remaining = next;
                true
            }
            None => false,
        }
    }
    let mut stack = vec![Frame::Value(value, 1)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Value(value, depth) => {
                if !charge(remaining, 1) {
                    return false;
                }
                match value {
                    serde_json::Value::Array(items) => {
                        if depth > crate::MAX_DOC_DEPTH - 4 {
                            return false;
                        }
                        stack.push(Frame::Array(items.iter(), depth));
                    }
                    serde_json::Value::Object(map) => {
                        if depth > crate::MAX_DOC_DEPTH - 4 {
                            return false;
                        }
                        stack.push(Frame::Object(map.iter(), depth));
                    }
                    serde_json::Value::String(text) if !charge(remaining, text.len()) => {
                        return false;
                    }
                    _ => {}
                }
            }
            Frame::Array(mut items, depth) => {
                if let Some(child) = items.next() {
                    stack.push(Frame::Array(items, depth));
                    stack.push(Frame::Value(child, depth + 1));
                }
            }
            Frame::Object(mut items, depth) => {
                if let Some((key, child)) = items.next() {
                    if !charge(remaining, key.len()) {
                        return false;
                    }
                    stack.push(Frame::Object(items, depth));
                    stack.push(Frame::Value(child, depth + 1));
                }
            }
        }
    }
    true
}

fn bounded_tail(log_id: &str, policy: usize, tail: Option<&LogTail>, limit: usize) -> bool {
    let mut remaining = limit;
    if log_id.len() > MAX_LEDGER_LOG_ID_BYTES {
        return false;
    }
    if let Some(tail) = tail {
        for entry in &tail.open_entries {
            if !bounded_value(&entry.value, &mut remaining) {
                return false;
            }
        }
    }
    let resident = (
        log_id,
        policy,
        tail.map(|tail| {
            (
                tail.open_chunk,
                &tail.open_entries,
                tail.next_seq,
                tail.open_bytes,
            )
        }),
    );
    encoded_len_bounded(&resident, limit).is_ok()
}

/// A structural break in a log's persisted chunk history — an
/// **authoritative-ledger breach**. Checked [`Ledger::restore_from_kv`] refuses
/// every such finding; [`scan_contiguity`] exposes the same inspection without
/// mutating or restoring a ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContiguityBreach {
    /// The affected log.
    pub log_id: SmolStr,
    /// What is wrong.
    pub kind: BreachKind,
}

/// The kind of [`ContiguityBreach`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreachKind {
    /// A reserved `log:` key does not have the canonical
    /// `log:{nonempty-id}:{minimal-u64}` form.
    MalformedKey,
    /// The boot input contained the same logical chunk more than once.
    DuplicateChunk {
        /// Duplicated chunk index.
        index: u64,
    },
    /// Chunk `index` is absent though a higher chunk exists (a dropped-seal hole).
    MissingChunk {
        /// The missing chunk index.
        index: u64,
    },
    /// A non-tail chunk (`index` below the highest) is not sealed — an
    /// interior chunk should always be immutable/full.
    UnsealedNonTail {
        /// The offending chunk index.
        index: u64,
    },
    /// A sequence discontinuity: entry `found` appeared where `expected` was due
    /// (a gap or repeat), within chunk `chunk`.
    SeqGap {
        /// The seq the log required next.
        expected: u64,
        /// The seq actually found.
        found: u64,
        /// The chunk it was found in.
        chunk: u64,
    },
    /// A `log:` document at chunk index `at` did not deserialize as a
    /// [`LedgerChunk`] — corrupt authoritative history that must be surfaced, not
    /// silently skipped (else a fail-closed consumer sees a wholly-malformed tail
    /// as "clean").
    MalformedChunk {
        /// The chunk index from the key of the unparseable doc.
        at: u64,
    },
    /// A `log:` document's key says chunk `at` but its embedded `chunk` field is
    /// `embedded` — a mismatched/tampered doc that is otherwise ignored.
    IndexMismatch {
        /// The chunk index from the key.
        at: u64,
        /// The chunk index inside the document.
        embedded: u64,
    },
    /// A persisted chunk is empty. The append protocol never writes an empty
    /// document, and accepting one makes the next sequence ambiguous.
    EmptyChunk {
        /// Empty chunk index.
        index: u64,
    },
    /// An encoded entry exceeds the same hard ceiling used by append.
    EntryTooLarge {
        /// Containing chunk.
        chunk: u64,
        /// Entry offset within the chunk.
        entry: usize,
    },
    /// An encoded chunk exceeds the hard persistence ceiling.
    ChunkTooLarge {
        /// Oversized chunk index.
        chunk: u64,
    },
    /// An unsealed tail is already at or beyond this ledger's seal target and
    /// could not have been produced by its append protocol.
    UnsealedAtOrOverTarget {
        /// Offending tail chunk.
        chunk: u64,
    },
    /// A chunk index or sequence number arithmetic overflowed while validating —
    /// treated as corruption rather than wrapping silently.
    ArithmeticOverflow {
        /// The chunk being validated when the overflow occurred.
        chunk: u64,
    },
}

/// What one inspection of a module's ledger chunk documents found: every
/// log's chunks in index order, the breaches, and — for each log whose tail
/// chunk is unsealed — that tail decoded, ready for the restore. Built once
/// per boot by [`inspect_ledger`] / [`Ledger::inspect`] and shared by the
/// contiguity decision, [`Ledger::restore_from_history`] and the consumer's
/// typed pass ([`LedgerHistory::chunk_docs`]), so the journal is parsed once
/// (J1). The chunk texts themselves stay with the caller; the history keeps
/// each chunk's position in the slice it inspected.
#[derive(Debug)]
pub struct LedgerHistory {
    by_log: BTreeMap<SmolStr, BTreeMap<u64, ChunkFacts>>,
    /// J4: the floors the inspection was made under (log id → chunks below
    /// this index were allowed to be absent); empty for a whole inspection.
    floors: ChunkFloors,
    /// Each log's **unsealed** tail chunk, decoded, with its entries' encoded
    /// byte total (the seal rule's counter).
    tails: BTreeMap<SmolStr, (LedgerChunk, usize)>,
    breaches: BoundedBreaches,
    chunks: usize,
    entries: usize,
}

/// One chunk as the inspection recorded it: where its text is, whether it is
/// sealed, and its entries' sequence numbers.
#[derive(Debug)]
struct ChunkFacts {
    /// Position in the inspected slice.
    doc: usize,
    sealed: bool,
    seqs: Vec<u64>,
}

impl LedgerHistory {
    /// The breaches found, capped at [`MAX_RESTORE_BREACH_DETAILS`] details.
    #[must_use]
    pub fn breaches(&self) -> &[ContiguityBreach] {
        &self.breaches.details
    }

    /// How many breaches were found in total (the details may be capped).
    #[must_use]
    pub const fn total_breaches(&self) -> usize {
        self.breaches.total
    }

    /// No breach at all: every log is a gap-free, correctly sealed history.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.breaches.total == 0
    }

    /// Valid chunks found, over every log.
    #[must_use]
    pub const fn chunks(&self) -> usize {
        self.chunks
    }

    /// Valid entries found, over every log.
    #[must_use]
    pub const fn entries(&self) -> usize {
        self.entries
    }

    /// The log ids seen, in order.
    pub fn logs(&self) -> impl Iterator<Item = &SmolStr> {
        self.by_log.keys()
    }

    /// J4: the chunk index below which `log_id`'s chunks were allowed to be
    /// absent in this inspection — 0 when it was whole.
    #[must_use]
    pub fn floor(&self, log_id: &str) -> u64 {
        self.floors.get(log_id).copied().unwrap_or(0)
    }

    /// `log_id`'s chunks in index order as `(chunk index, position in the
    /// inspected slice)`: the order a consumer's typed pass walks the texts
    /// in. Empty for an unknown log.
    pub fn chunk_docs(&self, log_id: &str) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.by_log
            .get(log_id)
            .into_iter()
            .flat_map(|chunks| chunks.iter().map(|(&index, facts)| (index, facts.doc)))
    }

    /// How many entries `log_id` holds over all its chunks.
    #[must_use]
    pub fn entries_of(&self, log_id: &str) -> usize {
        self.by_log.get(log_id).map_or(0, |chunks| {
            chunks.values().map(|facts| facts.seqs.len()).sum()
        })
    }
}

/// The strict decode a chunk is *inspected* by: the sequence numbers are read,
/// every payload is scanned and discarded (`IgnoredAny` allocates nothing),
/// and an unknown field anywhere is a malformed chunk — the same shape rule as
/// [`LedgerChunk`] at a fraction of the cost.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkFactsDoc {
    chunk: u64,
    sealed: bool,
    entries: Vec<EntryFactsDoc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryFactsDoc {
    seq: u64,
    #[allow(dead_code)]
    value: serde::de::IgnoredAny,
}

#[derive(Debug, Default)]
struct BoundedBreaches {
    details: Vec<ContiguityBreach>,
    total: usize,
}

impl BoundedBreaches {
    fn push(&mut self, breach: ContiguityBreach) {
        self.total = self.total.saturating_add(1);
        if self.details.len() < MAX_RESTORE_BREACH_DETAILS {
            self.details.push(breach);
        }
    }

    fn is_empty(&self) -> bool {
        self.total == 0
    }

    fn iter(&self) -> std::slice::Iter<'_, ContiguityBreach> {
        self.details.iter()
    }
}

fn malformed_key_log_id(key: &str) -> SmolStr {
    key.strip_prefix(LOG_PREFIX)
        .and_then(|rest| rest.rsplit_once(':').map(|(id, _)| id))
        .filter(|id| !id.is_empty())
        .map_or_else(|| SmolStr::new_static("<malformed>"), SmolStr::new)
}

/// Inspect a module's ledger chunk documents once: group them by log, decode
/// each chunk's facts (index, seal, sequence numbers) from its raw text, and
/// walk every log for gaps, seals and sequence contiguity. With
/// `chunk_target`, an unsealed tail whose entries already reach the seal
/// threshold is a breach too (the restore's rule). Every breach kind
/// [`BreachKind`] lists is produced here, except `EntryTooLarge`: a chunk
/// under [`MAX_SAFE_CHUNK_BYTES`] bounds every entry it holds, so that check
/// (the append path's) has nothing left to find on the read side.
///
/// The work is bounded by the number of persisted chunks and entries — never
/// by the raw chunk *index* — and the per-entry cost is a scan, not a `Value`
/// tree: each payload is skipped, not built.
#[must_use]
pub fn inspect_ledger(docs: &[LedgerDoc], chunk_target: Option<usize>) -> LedgerHistory {
    inspect_ledger_from(docs, chunk_target, &ChunkFloors::new())
}

/// Per ledger log id, the chunk index below which chunks may be absent from a
/// boot without being a hole (J4, doc 12 §5.6): the loader skipped them on the
/// namespace's boot manifest, and the module's checkpoint covers them.
pub type ChunkFloors = BTreeMap<SmolStr, u64>;

/// [`inspect_ledger`] over a **narrowed** boot: for each log in `floors`, a
/// chunk below the floor may be absent, the run must be gap-free from the
/// first present chunk up, and the sequence check starts at that chunk's first
/// entry. A log absent from `floors` is inspected whole (chunk 0 onward). The
/// floor's own chunk must be present when any higher one is — a manifest
/// naming a chunk the store no longer has is a `MissingChunk` at the floor.
#[must_use]
pub fn inspect_ledger_from(
    docs: &[LedgerDoc],
    chunk_target: Option<usize>,
    floors: &ChunkFloors,
) -> LedgerHistory {
    let mut by_log: BTreeMap<SmolStr, BTreeMap<u64, ChunkFacts>> = BTreeMap::new();
    let mut breaches = BoundedBreaches::default();
    let mut valid_chunks = 0usize;
    let mut valid_entries = 0usize;

    for (position, (key, text)) in docs.iter().enumerate() {
        let Some((log_id, chunk_seq)) = parse_chunk_key(key) else {
            if key.starts_with(LOG_PREFIX) {
                breaches.push(ContiguityBreach {
                    log_id: malformed_key_log_id(key),
                    kind: BreachKind::MalformedKey,
                });
            }
            continue;
        };

        if text.len() > MAX_SAFE_CHUNK_BYTES {
            breaches.push(ContiguityBreach {
                log_id,
                kind: BreachKind::ChunkTooLarge { chunk: chunk_seq },
            });
            continue;
        }

        match serde_json::from_str::<ChunkFactsDoc>(text) {
            Ok(doc) if doc.chunk == chunk_seq => {
                if doc.entries.is_empty() {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::EmptyChunk { index: chunk_seq },
                    });
                }
                let facts = ChunkFacts {
                    doc: position,
                    sealed: doc.sealed,
                    seqs: doc.entries.iter().map(|entry| entry.seq).collect(),
                };
                let chunks = by_log.entry(log_id.clone()).or_default();
                if chunks.insert(chunk_seq, facts).is_some() {
                    breaches.push(ContiguityBreach {
                        log_id,
                        kind: BreachKind::DuplicateChunk { index: chunk_seq },
                    });
                } else {
                    valid_chunks = valid_chunks.saturating_add(1);
                }
            }
            Ok(doc) => breaches.push(ContiguityBreach {
                log_id,
                kind: BreachKind::IndexMismatch {
                    at: chunk_seq,
                    embedded: doc.chunk,
                },
            }),
            Err(_) => breaches.push(ContiguityBreach {
                log_id,
                kind: BreachKind::MalformedChunk { at: chunk_seq },
            }),
        }
    }

    let mut tails: BTreeMap<SmolStr, (LedgerChunk, usize)> = BTreeMap::new();
    for (log_id, chunks) in &by_log {
        let Some(&max) = chunks.keys().next_back() else {
            continue;
        };
        // J4: below a floor, absence is not a hole; the first present chunk
        // may be anything at or below the floor, and must be the floor itself
        // when it is above it.
        let floor = floors.get(log_id).copied().unwrap_or(0);
        let mut prev: Option<u64> = None;
        for &idx in chunks.keys() {
            match prev {
                None if idx > floor => breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::MissingChunk { index: floor },
                }),
                Some(previous) => match previous.checked_add(1) {
                    Some(next) if idx != next => breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::MissingChunk { index: next },
                    }),
                    None => breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::ArithmeticOverflow { chunk: previous },
                    }),
                    _ => {}
                },
                _ => {}
            }
            prev = Some(idx);
        }

        // J4: a narrowed log's sequence run starts where its first present
        // chunk starts; a whole log's at 0.
        let mut expected_seq = if floor > 0 {
            chunks
                .values()
                .next()
                .and_then(|facts| facts.seqs.first().copied())
                .unwrap_or(0)
        } else {
            0
        };
        for (&idx, facts) in chunks {
            if idx < max && !facts.sealed {
                breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::UnsealedNonTail { index: idx },
                });
            }
            for &seq in &facts.seqs {
                valid_entries = valid_entries.saturating_add(1);
                if seq != expected_seq {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::SeqGap {
                            expected: expected_seq,
                            found: seq,
                            chunk: idx,
                        },
                    });
                    expected_seq = seq;
                }
                match expected_seq.checked_add(1) {
                    Some(next) => expected_seq = next,
                    None => {
                        breaches.push(ContiguityBreach {
                            log_id: log_id.clone(),
                            kind: BreachKind::ArithmeticOverflow { chunk: idx },
                        });
                        break;
                    }
                }
            }
        }

        let tail = chunks.get(&max).expect("max is a key of chunks");
        if tail.sealed && max == u64::MAX {
            breaches.push(ContiguityBreach {
                log_id: log_id.clone(),
                kind: BreachKind::ArithmeticOverflow { chunk: max },
            });
        }
        if !tail.sealed {
            // The one chunk the restore keeps in RAM: decode it fully, once,
            // and count its entries' encoded bytes for the seal rule.
            let text = &docs[tail.doc].1;
            let Ok(parsed) = parse_chunk(text) else {
                breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::MalformedChunk { at: max },
                });
                continue;
            };
            let mut open_bytes = 0usize;
            let mut overflowed = false;
            for entry in &parsed.entries {
                let Ok(bytes) = encoded_len_bounded(entry, MAX_SAFE_CHUNK_BYTES) else {
                    continue;
                };
                let Some(total) = open_bytes.checked_add(bytes) else {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::ArithmeticOverflow { chunk: max },
                    });
                    overflowed = true;
                    break;
                };
                open_bytes = total;
            }
            if overflowed {
                continue;
            }
            if let Some(target) = chunk_target
                && open_bytes >= target
            {
                breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::UnsealedAtOrOverTarget { chunk: max },
                });
            }
            tails.insert(log_id.clone(), (parsed, open_bytes));
        }
    }

    LedgerHistory {
        by_log,
        floors: floors.clone(),
        tails,
        breaches,
        chunks: valid_chunks,
        entries: valid_entries,
    }
}

fn bounded_restore_error(breaches: &BoundedBreaches) -> LedgerRestoreError {
    let total_breaches = breaches.total;
    let details_truncated = total_breaches > breaches.details.len();
    LedgerRestoreError {
        total_breaches,
        details_truncated,
        breaches: breaches.details.clone(),
    }
}

/// Scan a module's boot kv for authoritative-ledger contiguity breaches WITHOUT
/// mutating anything: for each log, the persisted chunks must be a gap-free run
/// `0..=max`, every non-tail chunk sealed, and the entry sequence numbers
/// contiguous and gap-free across the whole log. A pure pre-restore check a boot
/// can act on (log, or — for a money-bearing log — refuse boot).
///
/// The work is bounded by the number of persisted chunks and entries — never by
/// the raw chunk *index* — so a corrupted large index cannot make it allocate or
/// loop unboundedly. A gap reports the first missing index of the run (enough to
/// fail closed), not every absent index.
///
/// This assumes the log's **complete** chunk history is present in `kv` (chunk 0
/// onward). A boot narrowed by a boot manifest (J4, doc 12 §5.6) hands the
/// omitted prefix as floors to [`scan_contiguity_from`] instead — the
/// min-present marker this comment once foresaw.
///
/// Returned details are capped at [`MAX_RESTORE_BREACH_DETAILS`]. Checked
/// restore additionally reports the total and whether details were truncated.
#[must_use]
pub fn scan_contiguity(docs: &[LedgerDoc]) -> Vec<ContiguityBreach> {
    inspect_ledger(docs, None).breaches.details
}

/// [`scan_contiguity`] under chunk floors (J4): see [`inspect_ledger_from`].
#[must_use]
pub fn scan_contiguity_from(docs: &[LedgerDoc], floors: &ChunkFloors) -> Vec<ContiguityBreach> {
    inspect_ledger_from(docs, None, floors).breaches.details
}

#[cfg(test)]
#[allow(deprecated)] // legacy wrapper behavior remains covered during migration
mod tests {
    /// The persisted `(key, value)` docs a test builds, in the raw-text form
    /// boot hands the ledger (J1).
    fn as_docs(kv: &[(SmolStr, serde_json::Value)]) -> Vec<LedgerDoc> {
        kv.iter()
            .map(|(k, v)| (k.clone(), serde_json::to_string(v).unwrap()))
            .collect()
    }
    use super::*;

    #[test]
    fn resident_clone_budget_counts_every_log_and_exact_open_tail_bytes() {
        let empty = Ledger::new();
        assert!(empty.resident_within_bounds(0, 64));
        assert!(!empty.resident_within_bounds(0, 0));
        let mut ledger = Ledger::with_chunk_target(1024);
        for id in ["first", "second"] {
            let prepared = ledger
                .prepare_append(id, serde_json::json!({"data":"payload"}))
                .unwrap();
            ledger.apply_prepared(prepared).unwrap();
        }
        let before = ledger.clone();
        let exact = encoded_len_bounded(&ledger.chunk_target_bytes, usize::MAX).unwrap()
            + ledger
                .logs
                .iter()
                .map(|(id, tail)| {
                    encoded_len_bounded(
                        &(
                            id,
                            tail.open_chunk,
                            &tail.open_entries,
                            tail.next_seq,
                            tail.open_bytes,
                        ),
                        usize::MAX,
                    )
                    .unwrap()
                })
                .sum::<usize>();
        assert!(ledger.resident_within_bounds(2, exact));
        assert!(!ledger.resident_within_bounds(1, exact));
        assert!(!ledger.resident_within_bounds(2, exact - 1));
        assert_eq!(
            ledger, before,
            "the clone admission check cannot mutate tails"
        );
        let mut sealed = Ledger::with_chunk_target(0);
        let prepared = sealed
            .prepare_append("sealed", serde_json::json!(1))
            .unwrap();
        sealed.apply_prepared(prepared).unwrap();
        assert!(
            !sealed.resident_within_bounds(0, 4096),
            "sealed logs retain resident counters"
        );
        assert!(sealed.resident_within_bounds(1, 4096));
    }

    /// Decode a chunk op's persisted doc for assertions.
    fn chunk_of(op: &ModuleWrite) -> (SmolStr, LedgerChunk) {
        match op {
            ModuleWrite::JsonSet { key, value } => (
                key.clone(),
                serde_json::from_value(value.clone()).expect("chunk doc"),
            ),
            other => panic!("expected JsonSet, got {other:?}"),
        }
    }

    #[test]
    fn append_hands_out_gapless_sequence_and_rewrites_open_chunk() {
        let mut led = Ledger::new();
        let (s0, op0) = led.append("scene1", serde_json::json!({"e": "a"}));
        let (s1, op1) = led.append("scene1", serde_json::json!({"e": "b"}));
        assert_eq!((s0, s1), (0, 1));
        assert_eq!(led.next_seq("scene1"), 2);
        // Both appends rewrite the SAME open chunk key (chunk 0), each with the
        // full running contents — the second op supersedes the first.
        let (k0, c0) = chunk_of(&op0);
        let (k1, c1) = chunk_of(&op1);
        assert_eq!(k0, chunk_key("scene1", 0));
        assert_eq!(k1, chunk_key("scene1", 0));
        assert_eq!(c0.entries.len(), 1);
        assert_eq!(c1.entries.len(), 2);
        assert!(!c1.sealed, "an under-target chunk is not sealed");
        assert_eq!(led.open_entries("scene1").len(), 2);
    }

    #[test]
    fn logs_are_independent() {
        let mut led = Ledger::new();
        led.append("a", serde_json::json!(1));
        led.append("b", serde_json::json!(1));
        led.append("a", serde_json::json!(2));
        assert_eq!(led.next_seq("a"), 2);
        assert_eq!(led.next_seq("b"), 1);
        assert_eq!(led.next_seq("unknown"), 0);
    }

    #[test]
    fn chunks_seal_and_roll_over_at_the_target() {
        // Target 0 seals after every entry: one entry per chunk.
        let mut led = Ledger::with_chunk_target(0);
        let (_, op0) = led.append("s", serde_json::json!({"n": 0}));
        let (k0, c0) = chunk_of(&op0);
        assert_eq!(k0, chunk_key("s", 0));
        assert!(c0.sealed, "over-target chunk seals on the filling append");
        assert_eq!(c0.entries.len(), 1);
        // Rolled over: the open chunk is now 1, empty.
        assert_eq!(led.open_chunk("s"), 1);
        assert!(led.open_entries("s").is_empty());
        let (_, op1) = led.append("s", serde_json::json!({"n": 1}));
        let (k1, c1) = chunk_of(&op1);
        assert_eq!(
            k1,
            chunk_key("s", 1),
            "next append writes a fresh chunk key"
        );
        assert_eq!(c1.entries[0].seq, 1, "sequence is continuous across chunks");
    }

    // ---- H1: the cold-range reader ----

    /// A log with three sealed chunks (target 0: one entry per chunk) plus an
    /// open fourth, as the persisted `(key, value)` docs a store read returns.
    fn cold_docs() -> Vec<(SmolStr, serde_json::Value)> {
        let mut led = Ledger::with_chunk_target(0);
        let mut docs: BTreeMap<SmolStr, serde_json::Value> = BTreeMap::new();
        for n in 0..3 {
            let (_, op) = led.append("s", serde_json::json!({ "n": n }));
            let (k, c) = chunk_of(&op);
            docs.insert(k, serde_json::to_value(c).unwrap());
        }
        // An unsealed tail chunk: a ledger with a large target holds it open.
        let mut open = Ledger::with_chunk_target(1 << 20);
        // Seed it to the same seq by restoring the sealed history first.
        let sealed: Vec<(SmolStr, serde_json::Value)> =
            docs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        open.restore_from_kv(&as_docs(&sealed)).unwrap();
        let (_, op) = open.append("s", serde_json::json!({ "n": 3 }));
        let (k, c) = chunk_of(&op);
        assert!(!c.sealed);
        docs.insert(k, serde_json::to_value(c).unwrap());
        docs.into_iter().collect()
    }

    #[test]
    fn h1_read_cold_range_orders_a_contiguous_page_and_accepts_an_open_tail() {
        let docs = cold_docs();
        let entries = read_cold_range(&docs, "s").unwrap();
        assert_eq!(
            entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        // Reversed input order does not matter: the page orders by chunk.
        let mut rev = docs.clone();
        rev.reverse();
        assert_eq!(read_cold_range(&rev, "s").unwrap(), entries);
        // A sealed-only sub-page is fine too.
        let sealed_only: Vec<_> = docs.iter().take(3).cloned().collect();
        assert_eq!(read_cold_range(&sealed_only, "s").unwrap().len(), 3);
    }

    #[test]
    fn h1_read_cold_range_refuses_gaps_interior_unsealed_foreign_and_duplicate() {
        let docs = cold_docs();
        // A gap: chunks 0 and 2 without 1.
        let gapped: Vec<_> = docs
            .iter()
            .filter(|(k, _)| k != &chunk_key("s", 1))
            .cloned()
            .collect();
        assert!(matches!(
            read_cold_range(&gapped, "s"),
            Err(LedgerReadError::ChunkGap {
                expected: 1,
                found: 2
            })
        ));
        // An unsealed interior chunk: flip chunk 1's seal off.
        let mut unsealed = docs.clone();
        for (k, v) in &mut unsealed {
            if k == &chunk_key("s", 1) {
                v["sealed"] = serde_json::json!(false);
            }
        }
        assert!(matches!(
            read_cold_range(&unsealed, "s"),
            Err(LedgerReadError::UnsealedInterior { chunk: 1 })
        ));
        // A foreign log's chunk in the page.
        let mut foreign = docs.clone();
        foreign.push((chunk_key("other", 0), docs[0].1.clone()));
        assert!(matches!(
            read_cold_range(&foreign, "s"),
            Err(LedgerReadError::ForeignKey { .. })
        ));
        // A non-chunk key.
        let mut junk = docs.clone();
        junk.push((SmolStr::new("scene:1"), serde_json::json!({})));
        assert!(matches!(
            read_cold_range(&junk, "s"),
            Err(LedgerReadError::ForeignKey { .. })
        ));
        // A duplicate chunk.
        let mut dup = docs.clone();
        dup.push(docs[0].clone());
        assert!(matches!(
            read_cold_range(&dup, "s"),
            Err(LedgerReadError::DuplicateChunk { chunk: 0 })
        ));
        // Nothing for the log at all.
        assert!(matches!(
            read_cold_range(&[], "s"),
            Err(LedgerReadError::NoChunks { .. })
        ));
    }

    #[test]
    fn h1_archive_pages_walk_newest_first_and_end_at_chunk_zero() {
        // Ten chunks on disk (0..=9), four per page.
        assert_eq!(archive_page_range(9, 0), Some((6, 9)));
        assert_eq!(archive_page_range(9, 1), Some((2, 5)));
        assert_eq!(
            archive_page_range(9, 2),
            Some((0, 1)),
            "the last page is short"
        );
        assert_eq!(archive_page_range(9, 3), None, "past the oldest chunk");
        // A single chunk is one page.
        assert_eq!(archive_page_range(0, 0), Some((0, 0)));
        assert_eq!(archive_page_range(0, 1), None);
        // No overflow on absurd pages.
        assert_eq!(archive_page_range(9, u64::MAX), None);
    }

    #[test]
    fn h1_read_cold_range_refuses_sequence_gaps_index_mismatch_and_empty_chunks() {
        let docs = cold_docs();
        // A sequence gap: renumber chunk 2's entry.
        let mut seq_gap = docs.clone();
        for (k, v) in &mut seq_gap {
            if k == &chunk_key("s", 2) {
                v["entries"][0]["seq"] = serde_json::json!(7);
            }
        }
        assert!(matches!(
            read_cold_range(&seq_gap, "s"),
            Err(LedgerReadError::SequenceGap {
                expected: 2,
                found: 7
            })
        ));
        // The document's own index disagrees with its key.
        let mut mismatch = docs.clone();
        for (k, v) in &mut mismatch {
            if k == &chunk_key("s", 1) {
                v["chunk"] = serde_json::json!(5);
            }
        }
        assert!(matches!(
            read_cold_range(&mismatch, "s"),
            Err(LedgerReadError::IndexMismatch {
                key_chunk: 1,
                doc_chunk: 5
            })
        ));
        // An empty chunk.
        let mut empty = docs.clone();
        for (k, v) in &mut empty {
            if k == &chunk_key("s", 1) {
                v["entries"] = serde_json::json!([]);
            }
        }
        assert!(matches!(
            read_cold_range(&empty, "s"),
            Err(LedgerReadError::EmptyChunk { chunk: 1 })
        ));
        // A document that is not a chunk at all.
        let malformed = vec![(chunk_key("s", 0), serde_json::json!({ "nope": 1 }))];
        assert!(matches!(
            read_cold_range(&malformed, "s"),
            Err(LedgerReadError::Malformed { chunk: 0, .. })
        ));
    }

    #[test]
    fn a_sealed_chunk_is_written_exactly_once() {
        // With target 0, chunk 0 seals on append #0 and must never be rewritten.
        let mut led = Ledger::with_chunk_target(0);
        let (_, op0) = led.append("s", serde_json::json!(0));
        let (k0, _) = chunk_of(&op0);
        // Every later append targets a DIFFERENT chunk key.
        for n in 1..5 {
            let (_, op) = led.append("s", serde_json::json!(n));
            let (k, _) = chunk_of(&op);
            assert_ne!(k, k0, "append {n} must not rewrite the sealed chunk 0");
        }
    }

    #[test]
    fn restore_from_kv_resumes_an_unsealed_tail() {
        let mut led = Ledger::new();
        led.append("s", serde_json::json!({"x": 1}));
        let (_, op) = led.append("s", serde_json::json!({"x": 2}));
        let (key, doc) = chunk_of(&op);
        // Simulate a reboot: hand the persisted tail chunk to a fresh ledger.
        let mut rebooted = Ledger::new();
        rebooted
            .restore_from_kv(&as_docs(&[(key, serde_json::to_value(&doc).unwrap())]))
            .unwrap();
        assert_eq!(rebooted.next_seq("s"), 2, "resumes past the loaded entries");
        assert_eq!(
            rebooted.open_chunk("s"),
            0,
            "the unsealed chunk is still open"
        );
        assert_eq!(rebooted.open_entries("s").len(), 2);
        // Appending continues the sequence and rewrites the same open chunk.
        let (seq, op) = rebooted.append("s", serde_json::json!({"x": 3}));
        assert_eq!(seq, 2);
        let (_, c) = chunk_of(&op);
        assert_eq!(c.entries.len(), 3);
    }

    #[test]
    fn restore_from_kv_opens_the_next_chunk_after_a_sealed_tail() {
        let mut led = Ledger::with_chunk_target(0); // one entry per chunk
        let (_, op0) = led.append("s", serde_json::json!(0)); // chunk 0, sealed
        let (_, op1) = led.append("s", serde_json::json!(1)); // chunk 1, sealed
        let (k0, doc0) = chunk_of(&op0);
        let (k1, doc1) = chunk_of(&op1);
        let mut rebooted = Ledger::new();
        rebooted
            .restore_from_kv(&as_docs(&[
                (k0, serde_json::to_value(&doc0).unwrap()),
                (k1, serde_json::to_value(&doc1).unwrap()),
            ]))
            .unwrap();
        // Tail chunk 1 is sealed ⇒ open chunk is 2, empty, seq resumes at 2.
        assert_eq!(rebooted.open_chunk("s"), 2);
        assert_eq!(rebooted.next_seq("s"), 2);
        assert!(rebooted.open_entries("s").is_empty());
        let (seq, op) = rebooted.append("s", serde_json::json!(2));
        assert_eq!(seq, 2);
        let (k, _) = chunk_of(&op);
        assert_eq!(k, chunk_key("s", 2));
    }

    #[test]
    fn restore_picks_the_highest_chunk_and_ignores_cold_and_foreign_keys() {
        // Two sealed cold chunks + one open tail, plus an unrelated doc.
        let c0 = LedgerChunk {
            chunk: 0,
            sealed: true,
            entries: vec![LedgerEntry {
                seq: 0,
                value: serde_json::json!("a"),
            }],
        };
        let c1 = LedgerChunk {
            chunk: 1,
            sealed: true,
            entries: vec![LedgerEntry {
                seq: 1,
                value: serde_json::json!("b"),
            }],
        };
        let c2 = LedgerChunk {
            chunk: 2,
            sealed: false,
            entries: vec![LedgerEntry {
                seq: 2,
                value: serde_json::json!("c"),
            }],
        };
        let kv = vec![
            (chunk_key("s", 0), serde_json::to_value(&c0).unwrap()),
            (chunk_key("s", 1), serde_json::to_value(&c1).unwrap()),
            (chunk_key("s", 2), serde_json::to_value(&c2).unwrap()),
            (SmolStr::new("meta"), serde_json::json!({"unrelated": true})),
        ];
        let mut led = Ledger::new();
        led.restore_from_kv(&as_docs(&kv)).unwrap();
        // Tail is the unsealed chunk 2; only its one entry is in RAM.
        assert_eq!(led.open_chunk("s"), 2);
        assert_eq!(led.next_seq("s"), 3);
        assert_eq!(led.open_entries("s").len(), 1);
        assert_eq!(led.open_entries("s")[0].seq, 2);
    }

    #[test]
    fn key_helpers_round_trip_including_ids_with_colons() {
        assert_eq!(chunk_key("5", 3).as_str(), "log:5:3");
        assert!(is_log_key("log:5:3"));
        assert!(!is_log_key("scene:5:state"));
        assert!(!is_log_key("log:5")); // no chunk segment
        assert!(!is_log_key("meta"));
        // A log id containing ':' parses (chunk is the final segment).
        assert_eq!(
            parse_chunk_key("log:scene:42:7"),
            Some((SmolStr::new("scene:42"), 7))
        );
        assert_eq!(parse_chunk_key("log::7"), None); // empty id
    }

    #[test]
    fn re_persisting_the_open_chunk_is_byte_identical_idempotent() {
        // Two appends produce two ops for chunk 0; the second is the whole
        // chunk. Re-emitting the SAME op (a retried batch) is a no-op rewrite —
        // we prove the op is a pure function of state by rebuilding it.
        let mut led = Ledger::new();
        led.append("s", serde_json::json!("a"));
        let (_, op) = led.append("s", serde_json::json!("b"));
        let (key, doc) = chunk_of(&op);
        // The persisted doc is exactly the open chunk's contents.
        assert_eq!(key, chunk_key("s", 0));
        assert_eq!(doc.entries.len(), 2);
        assert!(!doc.sealed);
        let round = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            round,
            serde_json::to_value(LedgerChunk {
                chunk: 0,
                sealed: false,
                entries: vec![
                    LedgerEntry {
                        seq: 0,
                        value: serde_json::json!("a")
                    },
                    LedgerEntry {
                        seq: 1,
                        value: serde_json::json!("b")
                    },
                ],
            })
            .unwrap()
        );
    }

    /// Build a `log:s:{chunk}` kv doc with a contiguous seq run `[first..first+n)`.
    fn kv_chunk(
        log: &str,
        chunk: u64,
        sealed: bool,
        first: u64,
        n: u64,
    ) -> (SmolStr, serde_json::Value) {
        let entries = (first..first + n)
            .map(|seq| LedgerEntry {
                seq,
                value: serde_json::json!(seq),
            })
            .collect();
        (
            chunk_key(log, chunk),
            serde_json::to_value(LedgerChunk {
                chunk,
                sealed,
                entries,
            })
            .unwrap(),
        )
    }

    #[test]
    fn is_sealed_chunk_recognizes_only_sealed_log_docs() {
        let (sealed_key, sealed_val) = kv_chunk("s", 0, true, 0, 2);
        let (open_key, open_val) = kv_chunk("s", 1, false, 2, 1);
        assert!(
            is_sealed_chunk(&sealed_key, &sealed_val),
            "a sealed chunk op is retained"
        );
        assert!(
            !is_sealed_chunk(&open_key, &open_val),
            "an open chunk self-heals; not retained"
        );
        // A non-ledger doc that happens to carry `sealed:true` is not a chunk.
        assert!(!is_sealed_chunk(
            "comsys:chan:1",
            &serde_json::json!({"sealed": true})
        ));
        assert!(!is_sealed_chunk(
            &sealed_key,
            &serde_json::json!({"chunk": 0, "sealed": true, "entries": []})
        ));
        assert!(!is_sealed_chunk(
            &sealed_key,
            &serde_json::json!({
                "chunk": 1,
                "sealed": true,
                "entries": [{"seq": 0, "value": null}]
            })
        ));
        assert!(!is_sealed_chunk(
            &sealed_key,
            &serde_json::json!({
                "chunk": 0,
                "sealed": true,
                "entries": [
                    {"seq": 0, "value": null},
                    {"seq": 2, "value": null}
                ]
            })
        ));
    }

    /// J4: below a floor, an absent chunk is not a hole; the sequence run
    /// starts at the first present chunk; the restore resumes past the tail.
    #[test]
    fn j4_inspect_from_accepts_an_omitted_prefix_and_restores_the_tail() {
        let whole = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, true, 3, 2),
            kv_chunk("s", 2, true, 5, 2),
            kv_chunk("s", 3, false, 7, 1),
        ];
        let narrowed: Vec<_> = whole[2..].to_vec();
        let floors: ChunkFloors = [(SmolStr::new("s"), 2u64)].into_iter().collect();
        assert!(
            !scan_contiguity(&as_docs(&narrowed)).is_empty(),
            "whole: the prefix is a hole"
        );
        let history = inspect_ledger_from(&as_docs(&narrowed), None, &floors);
        assert!(history.is_clean(), "{:?}", history.breaches());
        assert_eq!((history.chunks(), history.entries()), (2, 3));
        assert_eq!(history.floor("s"), 2);
        assert_eq!(history.floor("other"), 0);
        let mut led = Ledger::new();
        assert!(led.inspect_from(&as_docs(&narrowed), &floors).is_clean());
        led.restore_from_history(&history).unwrap();
        assert_eq!(led.next_seq("s"), 8, "resumes past the present tail");
        // Chunks below the floor may still be present: a floor permits, never
        // demands, their absence.
        let history = inspect_ledger_from(&as_docs(&whole), None, &floors);
        assert!(history.is_clean());
        assert_eq!(history.entries(), 8);
        // A log the floors do not name is inspected whole.
        let mut mixed = narrowed.clone();
        mixed.push(kv_chunk("t", 1, false, 4, 1));
        let history = inspect_ledger_from(&as_docs(&mixed), None, &floors);
        assert!(
            history
                .breaches()
                .iter()
                .any(|b| b.log_id == "t" && b.kind == BreachKind::MissingChunk { index: 0 }),
            "{:?}",
            history.breaches()
        );
    }

    /// J4: a floor names a chunk the store no longer has — the floor's own
    /// chunk is missing though a higher one exists — and a gap above the floor
    /// is still a gap.
    #[test]
    fn j4_inspect_from_flags_the_floor_chunk_missing_and_gaps_above_it() {
        let floors: ChunkFloors = [(SmolStr::new("s"), 2u64)].into_iter().collect();
        let above = vec![kv_chunk("s", 3, false, 7, 1)];
        let history = inspect_ledger_from(&as_docs(&above), None, &floors);
        assert_eq!(
            history.breaches().first().map(|b| b.kind),
            Some(BreachKind::MissingChunk { index: 2 })
        );
        let gapped = vec![kv_chunk("s", 2, true, 5, 2), kv_chunk("s", 4, false, 9, 1)];
        let history = inspect_ledger_from(&as_docs(&gapped), None, &floors);
        assert!(
            history
                .breaches()
                .iter()
                .any(|b| b.kind == BreachKind::MissingChunk { index: 3 }),
            "{:?}",
            history.breaches()
        );
        let seq_gap = vec![kv_chunk("s", 2, true, 5, 2), kv_chunk("s", 3, false, 9, 1)];
        let history = inspect_ledger_from(&as_docs(&seq_gap), None, &floors);
        assert!(
            history.breaches().iter().any(|b| matches!(
                b.kind,
                BreachKind::SeqGap {
                    expected: 7,
                    found: 9,
                    ..
                }
            )),
            "{:?}",
            history.breaches()
        );
    }

    #[test]
    fn scan_contiguity_is_clean_for_a_well_formed_log() {
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, true, 3, 2),
            kv_chunk("s", 2, false, 5, 1), // open tail
            (SmolStr::new("meta"), serde_json::json!({"unrelated": true})),
        ];
        assert!(
            scan_contiguity(&as_docs(&kv)).is_empty(),
            "a gap-free, seq-continuous log has no breach"
        );
    }

    #[test]
    fn scan_contiguity_flags_a_dropped_predecessor_chunk() {
        // Chunk 1 never persisted (a dropped seal), but chunk 2 exists — the
        // exact loss the byte-scan closes and this scan surfaces at boot.
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 2, false, 5, 1)];
        let breaches = scan_contiguity(&as_docs(&kv));
        assert!(
            breaches
                .iter()
                .any(|b| b.log_id == "s" && b.kind == BreachKind::MissingChunk { index: 1 }),
            "the missing chunk 1 is reported: {breaches:?}"
        );
    }

    #[test]
    fn scan_contiguity_is_bounded_by_chunk_count_not_index() {
        // A corrupted/bit-flipped chunk index must NOT make the validator iterate
        // billions of times — it walks the two present chunks and reports one
        // bounded MissingChunk, returning immediately.
        let huge = 4_000_000_000u64;
        let kv = vec![
            kv_chunk("s", 0, true, 0, 1),
            // chunk doc whose index matches its (corrupted) key
            (
                chunk_key("s", huge),
                serde_json::to_value(LedgerChunk {
                    chunk: huge,
                    sealed: false,
                    entries: vec![LedgerEntry {
                        seq: 1,
                        value: serde_json::json!(1),
                    }],
                })
                .unwrap(),
            ),
        ];
        let breaches = scan_contiguity(&as_docs(&kv));
        // Exactly one missing-chunk breach (the first gap index), not ~4e9.
        let missing: Vec<_> = breaches
            .iter()
            .filter(|b| matches!(b.kind, BreachKind::MissingChunk { .. }))
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "one bounded gap breach, not a per-index flood"
        );
        assert!(matches!(
            missing[0].kind,
            BreachKind::MissingChunk { index: 1 }
        ));
    }

    #[test]
    fn scan_contiguity_surfaces_malformed_and_mismatched_chunks() {
        // A malformed authoritative doc and an index-mismatched one must be
        // reported, not silently skipped — else a fail-closed consumer reads a
        // wholly-corrupt tail as "clean".
        let mismatched = serde_json::to_value(LedgerChunk {
            chunk: 9, // embedded index disagrees with the key's chunk 1
            sealed: true,
            entries: vec![LedgerEntry {
                seq: 0,
                value: serde_json::json!(0),
            }],
        })
        .unwrap();
        let kv = vec![
            (chunk_key("s", 0), serde_json::json!({"not": "a chunk"})),
            (chunk_key("s", 1), mismatched),
        ];
        let breaches = scan_contiguity(&as_docs(&kv));
        assert!(
            breaches
                .iter()
                .any(|b| matches!(b.kind, BreachKind::MalformedChunk { at: 0 })),
            "unparseable chunk surfaced: {breaches:?}"
        );
        assert!(
            breaches
                .iter()
                .any(|b| matches!(b.kind, BreachKind::IndexMismatch { at: 1, embedded: 9 })),
            "index-mismatched chunk surfaced: {breaches:?}"
        );
        assert!(
            !breaches.is_empty(),
            "a corrupt tail is never reported clean"
        );
    }

    #[test]
    fn scan_contiguity_flags_an_unsealed_interior_chunk() {
        let kv = vec![
            kv_chunk("s", 0, false, 0, 3), // interior but NOT sealed
            kv_chunk("s", 1, false, 3, 1),
        ];
        let breaches = scan_contiguity(&as_docs(&kv));
        assert!(
            breaches
                .iter()
                .any(|b| b.kind == BreachKind::UnsealedNonTail { index: 0 }),
            "the unsealed interior chunk 0 is reported: {breaches:?}"
        );
    }

    #[test]
    fn scan_contiguity_flags_a_sequence_gap() {
        // Chunk 1 starts at seq 9, not 3 — a discontinuity within a contiguous
        // chunk run (e.g. a corrupted/edited chunk).
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 1, false, 9, 1)];
        let breaches = scan_contiguity(&as_docs(&kv));
        assert!(
            breaches.iter().any(|b| matches!(
                b.kind,
                BreachKind::SeqGap {
                    expected: 3,
                    found: 9,
                    chunk: 1
                }
            )),
            "the seq gap at chunk 1 is reported: {breaches:?}"
        );
    }

    #[test]
    fn authoritative_restore_refuses_a_contiguity_breach_without_mutation() {
        // A holed authoritative log must fail closed and leave even a
        // pre-populated destination byte-identical.
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 2, false, 5, 1)];
        assert!(!scan_contiguity(&as_docs(&kv)).is_empty());
        let mut led = Ledger::new();
        let plan = led
            .prepare_append("existing", serde_json::json!(1))
            .unwrap();
        led.apply_prepared(plan).unwrap();
        let before = led.clone();
        let error = led.restore_from_kv(&as_docs(&kv)).unwrap_err();
        assert!(error.total_breaches() > 0);
        assert_eq!(led, before);
    }

    #[test]
    fn explicitly_lossy_restore_can_rebuild_a_holed_non_authoritative_tail() {
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 2, false, 5, 1)];
        let mut led = Ledger::new();
        let report = led.restore_from_kv_lossy(&as_docs(&kv));
        assert_eq!(report.logs, 1);
        assert_eq!(led.open_chunk("s"), 2);
        assert_eq!(led.next_seq("s"), 6);
        assert_eq!(led.open_entries("s").len(), 1);
    }

    #[test]
    fn prepare_is_non_mutating_and_apply_installs_exact_prebuilt_write() {
        let mut led = Ledger::new();
        let before = led.clone();
        let prepared = led
            .prepare_append("s", serde_json::json!({"event": "ready"}))
            .unwrap();
        assert_eq!(prepared.seq(), 0);
        assert_eq!(led, before, "preparation must not alter ledger state");

        let (seq, write) = led.apply_prepared(prepared).unwrap();
        assert_eq!(seq, 0);
        let (key, document) = chunk_of(&write);
        assert_eq!(key, chunk_key("s", 0));
        assert_eq!(document.entries[0].seq, 0);
        assert_eq!(
            document.entries[0].value,
            serde_json::json!({"event": "ready"})
        );
        assert_eq!(led.next_seq("s"), 1);
    }

    #[test]
    fn bound_append_installs_the_exact_open_or_sealed_tail_and_write() {
        for target in [usize::MAX, 0] {
            let mut ledger = Ledger::with_chunk_target(target);
            let prepared = ledger
                .prepare_append("s", serde_json::json!({"event": "ready"}))
                .unwrap();
            let expected_tail = prepared.next_tail.clone();
            let expected_write = prepared.write.clone();
            let expected_seq = prepared.seq();
            let before = ledger.clone();

            let bound = ledger.bind_prepared(prepared).unwrap();
            assert_eq!(*bound.ledger, before, "binding installs nothing");
            let (seq, write) = bound.apply();
            assert_eq!(seq, expected_seq);
            assert_eq!(write, expected_write);
            assert_eq!(ledger.logs.get("s"), Some(&expected_tail));
            let (_, document) = chunk_of(&write);
            assert_eq!(document.sealed, target == 0);
            assert_eq!(ledger.next_seq("s"), 1);
            assert_eq!(ledger.open_chunk("s"), u64::from(target == 0));
        }
    }

    #[test]
    fn dropping_a_bound_append_leaves_existing_and_absent_tails_unchanged() {
        let mut ledger = Ledger::new();
        let first = ledger
            .prepare_append("existing", serde_json::json!(1))
            .unwrap();
        ledger.apply_prepared(first).unwrap();
        for log in ["existing", "absent"] {
            let before = ledger.clone();
            let prepared = ledger.prepare_append(log, serde_json::json!(2)).unwrap();
            let seq = prepared.seq();
            let bound = ledger.bind_prepared(prepared).unwrap();
            assert_eq!(*bound.ledger, before);
            drop(bound);
            assert_eq!(ledger, before);
            let fresh = ledger.prepare_append(log, serde_json::json!(2)).unwrap();
            assert_eq!(fresh.seq(), seq, "discarding never consumes a sequence");
            ledger.apply_prepared(fresh).unwrap();
        }
    }

    #[test]
    fn last_ledger_bind_refusal_does_not_install_an_earlier_bound_tail() {
        for changed_policy in [false, true] {
            let mut first = Ledger::new();
            let mut last = Ledger::new();
            let first_plan = first.prepare_append("s", serde_json::json!(1)).unwrap();
            let last_plan = last.prepare_append("s", serde_json::json!(2)).unwrap();
            if changed_policy {
                last.chunk_target_bytes = 0;
            } else {
                let competing = last.prepare_append("s", serde_json::json!(3)).unwrap();
                last.apply_prepared(competing).unwrap();
            }
            let first_before = first.clone();
            let last_before = last.clone();
            let first_bound = first.bind_prepared(first_plan).unwrap();
            assert!(matches!(
                last.bind_prepared(last_plan),
                Err(LedgerApplyError::StalePrestate)
            ));
            drop(first_bound);
            assert_eq!(first, first_before);
            assert_eq!(last, last_before);
        }
    }

    #[test]
    fn bounded_batch_preserves_original_order_and_drop_is_inert() {
        let mut ledger = Ledger::new();
        let prior = ledger
            .prepare_append("source", serde_json::json!("prior"))
            .unwrap();
        ledger.apply_prepared(prior).unwrap();
        let before = ledger.clone();
        let prepare = |ledger: &Ledger| {
            vec![
                ledger
                    .prepare_append_bounded("destination", serde_json::json!("joined"), 4096)
                    .unwrap(),
                ledger
                    .prepare_append_bounded("source", serde_json::json!("left"), 4096)
                    .unwrap(),
            ]
        };
        let plans = prepare(&ledger);
        assert_eq!(
            plans
                .iter()
                .map(PreparedLedgerAppend::seq)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        let bound = ledger.bind_prepared_batch(plans).unwrap();
        drop(bound);
        assert_eq!(ledger, before);
        let plans = prepare(&ledger);
        let expected = plans
            .iter()
            .map(|plan| (plan.seq, plan.write.clone()))
            .collect::<Vec<_>>();
        assert_eq!(ledger.bind_prepared_batch(plans).unwrap().apply(), expected);
        assert_eq!(ledger.logs["destination"].next_seq, 1);
        assert_eq!(ledger.logs["source"].next_seq, 2);
    }

    #[test]
    fn stale_second_tail_or_policy_refuses_entire_batch() {
        for changed_policy in [false, true] {
            let mut ledger = Ledger::new();
            let first = ledger
                .prepare_append_bounded("first", serde_json::json!(1), 4096)
                .unwrap();
            let second = ledger
                .prepare_append_bounded("second", serde_json::json!(2), 4096)
                .unwrap();
            if changed_policy {
                ledger.chunk_target_bytes = 0;
            } else {
                let competing = ledger
                    .prepare_append("second", serde_json::json!(3))
                    .unwrap();
                ledger.apply_prepared(competing).unwrap();
            }
            let before = ledger.clone();
            assert!(matches!(
                ledger.bind_prepared_batch(vec![first, second]),
                Err(LedgerBatchError::StalePrestate)
            ));
            assert_eq!(ledger, before);
            assert!(!ledger.logs.contains_key("first"));
        }
    }

    #[test]
    fn batch_rejects_duplicate_and_excess_logs_but_accepts_empty() {
        let mut ledger = Ledger::new();
        let prepare = |ledger: &Ledger, log: &str| {
            ledger
                .prepare_append_bounded(log, serde_json::json!(1), 4096)
                .unwrap()
        };
        let duplicate = vec![prepare(&ledger, "same"), prepare(&ledger, "same")];
        assert!(matches!(
            ledger.bind_prepared_batch(duplicate),
            Err(LedgerBatchError::DuplicateLog)
        ));
        let too_many = vec![
            prepare(&ledger, "a"),
            prepare(&ledger, "b"),
            prepare(&ledger, "c"),
        ];
        assert!(matches!(
            ledger.bind_prepared_batch(too_many),
            Err(LedgerBatchError::TooMany)
        ));
        assert!(
            ledger
                .bind_prepared_batch(Vec::new())
                .unwrap()
                .apply()
                .is_empty()
        );
        assert_eq!(ledger, Ledger::new());
    }

    #[test]
    fn bounded_prepare_only_reads_the_selected_tail() {
        let mut ledger = Ledger::with_chunk_target(usize::MAX);
        let large = ledger
            .prepare_append("large", serde_json::json!("x".repeat(8192)))
            .unwrap();
        ledger.apply_prepared(large).unwrap();
        assert!(
            ledger
                .prepare_append_bounded("small", serde_json::json!(1), 128)
                .is_ok()
        );
        assert!(matches!(
            ledger.prepare_append_bounded("large", serde_json::json!(1), 128),
            Err(LedgerAppendError::ResidentLimit { max: 128 })
        ));
        assert!(
            ledger
                .prepare_append_bounded("large", serde_json::json!(1), 16384)
                .is_ok()
        );
    }

    #[test]
    fn bounded_batch_checks_late_live_shape_before_recursive_equality() {
        for deep in [false, true] {
            let mut ledger = Ledger::new();
            let initial = ledger
                .prepare_append("selected", serde_json::json!(1))
                .unwrap();
            ledger.apply_prepared(initial).unwrap();
            let plan = ledger
                .prepare_append_bounded("selected", serde_json::json!(2), 512)
                .unwrap();
            let mut value = serde_json::json!("x".repeat(1024));
            if deep {
                value = serde_json::Value::Null;
                for _ in 0..crate::MAX_DOC_DEPTH {
                    value = serde_json::json!([value]);
                }
            }
            let tail = ledger.logs.get_mut("selected").unwrap();
            tail.open_entries[0].value = value;
            let next_seq = tail.next_seq;
            assert!(matches!(
                ledger.bind_prepared_batch(vec![plan]),
                Err(LedgerBatchError::InputLimit)
            ));
            assert_eq!(ledger.logs["selected"].next_seq, next_seq);
            assert_eq!(ledger.logs["selected"].open_entries.len(), 1);
        }
    }

    #[test]
    fn bounded_prepare_refuses_deep_entry_and_resident_before_clone() {
        let mut ledger = Ledger::new();
        let mut value = serde_json::Value::Null;
        for _ in 0..crate::MAX_DOC_DEPTH {
            value = serde_json::json!([value]);
        }
        assert!(matches!(
            ledger.prepare_append_bounded("s", value.clone(), 4096),
            Err(LedgerAppendError::InputLimit)
        ));
        ledger.logs.insert(
            "s".into(),
            LogTail {
                open_chunk: 0,
                open_entries: vec![LedgerEntry { seq: 0, value }],
                next_seq: 1,
                open_bytes: 1,
            },
        );
        assert!(matches!(
            ledger.prepare_append_bounded("s", serde_json::json!(1), 4096),
            Err(LedgerAppendError::ResidentLimit { .. })
        ));
        assert_eq!(ledger.logs["s"].next_seq, 1);
    }

    #[test]
    fn oversized_entry_is_rejected_without_mutation() {
        let led = Ledger::new();
        let before = led.clone();
        let value = serde_json::Value::String("x".repeat(MAX_SAFE_CHUNK_BYTES + 1));
        let error = led.prepare_append("s", value).unwrap_err();
        assert!(matches!(error, LedgerAppendError::EntryTooLarge { .. }));
        assert_eq!(led, before);
    }

    #[test]
    fn oversized_prospective_chunk_is_rejected_without_mutation() {
        // Each entry is below the ceiling; together (plus document framing)
        // they cross it. A huge target keeps the first entry unsealed.
        let mut led = Ledger::with_chunk_target(usize::MAX);
        let half = MAX_SAFE_CHUNK_BYTES / 2;
        let first = led
            .prepare_append("s", serde_json::Value::String("a".repeat(half)))
            .unwrap();
        led.apply_prepared(first).unwrap();
        let before = led.clone();
        let error = led
            .prepare_append("s", serde_json::Value::String("b".repeat(half)))
            .unwrap_err();
        assert!(matches!(error, LedgerAppendError::ChunkTooLarge { .. }));
        assert_eq!(led, before);
    }

    #[test]
    fn sequence_byte_and_chunk_overflows_are_non_mutating() {
        let cases = [
            (
                LogTail {
                    open_chunk: 0,
                    open_entries: Vec::new(),
                    next_seq: u64::MAX,
                    open_bytes: 0,
                },
                usize::MAX,
                LedgerAppendError::SequenceOverflow,
            ),
            (
                LogTail {
                    open_chunk: 0,
                    open_entries: Vec::new(),
                    next_seq: 0,
                    open_bytes: usize::MAX,
                },
                usize::MAX,
                LedgerAppendError::ByteCountOverflow,
            ),
            (
                LogTail {
                    open_chunk: u64::MAX,
                    open_entries: Vec::new(),
                    next_seq: 0,
                    open_bytes: 0,
                },
                0,
                LedgerAppendError::ChunkIndexOverflow,
            ),
        ];
        for (tail, target, expected) in cases {
            let mut led = Ledger::with_chunk_target(target);
            led.logs.insert(SmolStr::new_static("s"), tail);
            let before = led.clone();
            assert_eq!(
                led.prepare_append("s", serde_json::json!(1)).unwrap_err(),
                expected
            );
            assert_eq!(led, before);
        }
    }

    #[test]
    fn stale_prepared_plan_is_rejected_without_mutation() {
        let mut led = Ledger::new();
        let first = led.prepare_append("s", serde_json::json!(1)).unwrap();
        let stale = led.prepare_append("s", serde_json::json!(2)).unwrap();
        led.apply_prepared(first).unwrap();
        let before = led.clone();
        assert_eq!(
            led.apply_prepared(stale),
            Err(LedgerApplyError::StalePrestate)
        );
        assert_eq!(led, before);
    }

    #[test]
    fn restore_refuses_sequence_and_chunk_overflow_without_mutation() {
        let overflowing_seq = LedgerChunk {
            chunk: 0,
            sealed: false,
            entries: vec![LedgerEntry {
                seq: u64::MAX,
                value: serde_json::json!("bad"),
            }],
        };
        let overflowing_chunk = LedgerChunk {
            chunk: u64::MAX,
            sealed: true,
            entries: vec![LedgerEntry {
                seq: 0,
                value: serde_json::json!("bad"),
            }],
        };
        for kv in [
            vec![(
                chunk_key("s", 0),
                serde_json::to_value(overflowing_seq.clone()).unwrap(),
            )],
            vec![(
                chunk_key("s", u64::MAX),
                serde_json::to_value(overflowing_chunk.clone()).unwrap(),
            )],
        ] {
            let mut led = Ledger::new();
            let prepared = led
                .prepare_append("existing", serde_json::json!(1))
                .unwrap();
            led.apply_prepared(prepared).unwrap();
            let before = led.clone();
            let error = led.restore_from_kv(&as_docs(&kv)).unwrap_err();
            assert!(
                error
                    .breaches()
                    .iter()
                    .any(|breach| matches!(breach.kind, BreachKind::ArithmeticOverflow { .. }))
            );
            assert_eq!(led, before);
        }
    }

    #[test]
    fn checked_restore_rejects_noncanonical_duplicate_and_unknown_field_docs() {
        let valid = kv_chunk("s", 0, false, 0, 1);
        let duplicate = valid.clone();
        let noncanonical = (
            SmolStr::new_static("log:s:01"),
            serde_json::to_value(LedgerChunk {
                chunk: 1,
                sealed: false,
                entries: vec![LedgerEntry {
                    seq: 1,
                    value: serde_json::json!(1),
                }],
            })
            .unwrap(),
        );
        let unknown_field = (
            chunk_key("other", 0),
            serde_json::json!({
                "chunk": 0,
                "sealed": false,
                "entries": [{"seq": 0, "value": 1, "surprise": true}]
            }),
        );
        let mut led = Ledger::new();
        let error = led
            .restore_from_kv(&as_docs(&[valid, duplicate, noncanonical, unknown_field]))
            .unwrap_err();
        assert!(
            error
                .breaches()
                .iter()
                .any(|breach| matches!(breach.kind, BreachKind::DuplicateChunk { .. }))
        );
        assert!(
            error
                .breaches()
                .iter()
                .any(|breach| breach.kind == BreachKind::MalformedKey)
        );
        assert!(
            error
                .breaches()
                .iter()
                .any(|breach| matches!(breach.kind, BreachKind::MalformedChunk { .. }))
        );
        assert_eq!(led, Ledger::new());
    }

    #[test]
    fn checked_restore_diagnostics_are_bounded() {
        let kv: Vec<_> = (0..(MAX_RESTORE_BREACH_DETAILS + 20))
            .map(|index| {
                (
                    SmolStr::new(format!("log:s:bad-{index}")),
                    serde_json::json!(null),
                )
            })
            .collect();
        let mut led = Ledger::new();
        let error = led.restore_from_kv(&as_docs(&kv)).unwrap_err();
        assert_eq!(error.total_breaches(), MAX_RESTORE_BREACH_DETAILS + 20);
        assert_eq!(error.breaches().len(), MAX_RESTORE_BREACH_DETAILS);
        assert!(error.details_truncated());
        assert_eq!(led, Ledger::new());
    }

    // ---- J1: one inspection, one typed pass (doc 12 §7 J1; lenses E and R) ----

    /// One inspection serves the contiguity decision, the restore and the
    /// consumer's typed pass, and the chunk order it hands out is the log's
    /// order, whatever order the documents arrived in.
    #[test]
    fn j1_one_inspection_serves_scan_restore_and_the_typed_pass() {
        let kv = vec![
            kv_chunk("s", 2, false, 6, 2),
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, true, 3, 3),
        ];
        let docs = as_docs(&kv);
        let led = Ledger::new();
        let history = led.inspect(&docs);
        assert!(history.is_clean(), "{:?}", history.breaches());
        assert_eq!((history.chunks(), history.entries()), (3, 8));
        assert_eq!(history.entries_of("s"), 8);
        assert_eq!(history.entries_of("nope"), 0);
        assert_eq!(
            history.logs().cloned().collect::<Vec<_>>(),
            vec![SmolStr::new("s")]
        );
        let order: Vec<(u64, SmolStr)> = history
            .chunk_docs("s")
            .map(|(chunk, position)| (chunk, docs[position].0.clone()))
            .collect();
        assert_eq!(
            order,
            vec![
                (0, chunk_key("s", 0)),
                (1, chunk_key("s", 1)),
                (2, chunk_key("s", 2))
            ]
        );
        // The typed pass decodes straight from the text, in that order.
        let mut typed: Vec<(u64, u64)> = Vec::new();
        for (_, position) in history.chunk_docs("s") {
            typed.extend(typed_entries::<u64>(&docs[position].1).unwrap());
        }
        assert_eq!(
            typed.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        assert!(
            typed.iter().all(|(seq, value)| seq == value),
            "kv_chunk payloads are their own seqs"
        );
        // The restore from the shared inspection is the one-shot restore.
        let mut from_history = Ledger::new();
        let report = from_history.restore_from_history(&history).unwrap();
        let mut one_shot = Ledger::new();
        let report2 = one_shot.restore_from_kv(&docs).unwrap();
        assert_eq!(from_history, one_shot);
        assert_eq!((report.logs, report.chunks, report.entries), (1, 3, 8));
        assert_eq!((report2.logs, report2.chunks, report2.entries), (1, 3, 8));
        assert_eq!(from_history.next_seq("s"), 8);
        assert_eq!(from_history.open_chunk("s"), 2);
        assert_eq!(from_history.open_entries("s").len(), 2);
    }

    /// The typed pass is as strict as the opaque form: an unknown field
    /// anywhere, or a payload of another shape, is a decode error.
    #[test]
    fn j1_typed_entries_are_strict() {
        let (_, doc) = kv_chunk("s", 0, true, 0, 2);
        let text = serde_json::to_string(&doc).unwrap();
        assert_eq!(typed_entries::<u64>(&text).unwrap(), vec![(0, 0), (1, 1)]);
        assert!(
            typed_entries::<String>(&text).is_err(),
            "a number payload is not a string"
        );
        let mut loose = doc.clone();
        loose["entries"][0]["extra"] = serde_json::json!(true);
        assert!(typed_entries::<u64>(&serde_json::to_string(&loose).unwrap()).is_err());
        let mut loose = doc;
        loose["surprise"] = serde_json::json!(1);
        assert!(typed_entries::<u64>(&serde_json::to_string(&loose).unwrap()).is_err());
        assert!(parse_chunk("{not-json").is_err());
    }

    /// The seal rule follows the ledger that restores, not the inspection: an
    /// inspection made without a target (a pure contiguity scan) still refuses
    /// to restore an open tail at or over the restoring ledger's target, and
    /// refusal leaves the ledger untouched.
    #[test]
    fn j1_restore_applies_the_seal_rule_of_the_restoring_ledger() {
        let kv = vec![kv_chunk("s", 0, true, 0, 2), kv_chunk("s", 1, false, 2, 3)];
        let docs = as_docs(&kv);
        let history = inspect_ledger(&docs, None);
        assert!(history.is_clean(), "{:?}", history.breaches());
        let mut tiny = Ledger::with_chunk_target(1);
        let error = tiny.restore_from_history(&history).unwrap_err();
        assert!(
            error
                .breaches()
                .iter()
                .any(|b| matches!(b.kind, BreachKind::UnsealedAtOrOverTarget { chunk: 1 })),
            "{error:?}"
        );
        assert_eq!(tiny, Ledger::with_chunk_target(1), "refused: unchanged");
        let mut roomy = Ledger::with_chunk_target(1 << 20);
        roomy.restore_from_history(&history).unwrap();
        assert_eq!(roomy.open_entries("s").len(), 3);
        // And an inspection made under the target reports it as a breach itself.
        let under_target = inspect_ledger(&docs, Some(1));
        assert!(!under_target.is_clean());
        assert_eq!(under_target.total_breaches(), 1);
    }

    /// A filtered restore validates every log but keeps only the selected
    /// tails, reports what it kept, and a breach in a log it does NOT keep
    /// still refuses the whole restore.
    #[test]
    fn j1_filtered_restore_keeps_only_the_selected_logs() {
        let kv = vec![
            kv_chunk("live", 0, false, 0, 2),
            kv_chunk("closed", 0, true, 0, 4),
            kv_chunk("closed", 1, false, 4, 1),
        ];
        let history = inspect_ledger(&as_docs(&kv), None);
        assert!(history.is_clean());
        let mut led = Ledger::new();
        let report = led
            .restore_logs_from_history(&history, |log| log == "live")
            .unwrap();
        assert_eq!((report.logs, report.chunks, report.entries), (1, 1, 2));
        assert_eq!(led.next_seq("live"), 2);
        assert_eq!(led.next_seq("closed"), 0, "not restored");
        let mut broken = kv;
        broken.push(kv_chunk("closed", 3, false, 9, 1)); // chunk 2 never persisted
        let history = inspect_ledger(&as_docs(&broken), None);
        assert!(!history.is_clean());
        let mut led2 = Ledger::new();
        assert!(
            led2.restore_logs_from_history(&history, |log| log == "live")
                .is_err()
        );
        assert_eq!(led2, Ledger::new());
    }

    /// `EntryTooLarge` is the append path's: on the read side a chunk under the
    /// cap bounds every entry it holds, and the chunk cap is judged on the
    /// stored text, with no serialization.
    #[test]
    fn j1_the_chunk_cap_is_judged_on_the_stored_text() {
        let big = "x".repeat(MAX_SAFE_CHUNK_BYTES);
        let doc = serde_json::json!({
            "chunk": 0, "sealed": true, "entries": [{ "seq": 0, "value": big }]
        });
        let breaches = scan_contiguity(&as_docs(&[(chunk_key("s", 0), doc)]));
        assert!(
            breaches
                .iter()
                .any(|b| matches!(b.kind, BreachKind::ChunkTooLarge { chunk: 0 })),
            "{breaches:?}"
        );
    }

    /// `recent_sealed` over text: the last `n` sealed chunks, oldest first,
    /// skipping the open tail and any document that is not a valid sealed chunk.
    #[test]
    fn j1_recent_sealed_reads_text() {
        let kv = vec![
            kv_chunk("s", 0, true, 0, 2),
            kv_chunk("s", 1, true, 2, 2),
            kv_chunk("s", 2, true, 4, 2),
            kv_chunk("s", 3, false, 6, 1),
            (chunk_key("s", 9), serde_json::json!("not a chunk")),
        ];
        let docs = as_docs(&kv);
        let seqs: Vec<u64> = recent_sealed(&docs, "s", 2).iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 3, 4, 5]);
        assert!(recent_sealed(&docs, "s", 0).is_empty());
        assert!(recent_sealed(&docs, "other", 2).is_empty());
    }
}
