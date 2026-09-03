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
    write: ModuleWrite,
}

impl PreparedLedgerAppend {
    /// Sequence number reserved by this plan.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }
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
        let value = serde_json::to_value(document).map_err(|_| LedgerAppendError::ChunkEncoding)?;
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
            write,
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
        if self.chunk_target_bytes != prepared.chunk_target_bytes
            || self.logs.get(prepared.log_id.as_str()) != prepared.expected_tail.as_ref()
        {
            return Err(LedgerApplyError::StalePrestate);
        }
        self.logs.insert(prepared.log_id, prepared.next_tail);
        Ok((prepared.seq, prepared.write))
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

    /// Restore the open tail of every log from authoritative boot documents.
    ///
    /// Scans `(key, value)` pairs for `log:{id}:{chunk}` chunk docs, groups by
    /// log id, and rebuilds each log's tail from its **highest** chunk:
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
    pub fn restore_from_kv(
        &mut self,
        kv: &[(SmolStr, serde_json::Value)],
    ) -> Result<LedgerRestoreReport, LedgerRestoreError> {
        let inspection = inspect_history(kv, Some(self.chunk_target_bytes));
        if !inspection.breaches.is_empty() {
            return Err(bounded_restore_error(inspection.breaches));
        }

        let mut restored = BTreeMap::new();
        for (log_id, chunks) in &inspection.by_log {
            let (&tail_index, tail_doc) = chunks
                .last_key_value()
                .expect("a grouped ledger log has at least one chunk");
            let next_seq = tail_doc
                .entries
                .last()
                .expect("empty chunks are rejected by inspection")
                .seq
                .checked_add(1)
                .expect("sequence overflow is rejected by inspection");
            let tail = if tail_doc.sealed {
                LogTail {
                    open_chunk: tail_index
                        .checked_add(1)
                        .expect("chunk overflow is rejected by inspection"),
                    open_entries: Vec::new(),
                    next_seq,
                    open_bytes: 0,
                }
            } else {
                let mut open_bytes = 0usize;
                for entry in &tail_doc.entries {
                    let encoded = encoded_len_bounded(entry, MAX_SAFE_CHUNK_BYTES)
                        .expect("entry encoding and limits are checked by inspection");
                    open_bytes = open_bytes
                        .checked_add(encoded)
                        .expect("byte overflow is rejected by inspection");
                }
                LogTail {
                    open_chunk: tail_index,
                    open_entries: tail_doc.entries.clone(),
                    next_seq,
                    open_bytes,
                }
            };
            restored.insert(log_id.clone(), tail);
        }

        let report = LedgerRestoreReport {
            logs: inspection.by_log.len(),
            chunks: inspection.chunks,
            entries: inspection.entries,
        };
        // This is the only mutation point. The supplied authoritative snapshot
        // replaces, rather than merges with, any old RAM state; absent logs
        // must not survive a repeated restore.
        self.logs = restored;
        Ok(report)
    }

    /// Best-effort compatibility restore for explicitly non-authoritative
    /// logs. Integrity breaches are logged and malformed pieces skipped.
    ///
    /// Do not use this for scene, canon, economy, ownership, or any other log
    /// whose contents influence authoritative state.
    pub fn restore_from_kv_lossy(
        &mut self,
        kv: &[(SmolStr, serde_json::Value)],
    ) -> LedgerRestoreReport {
        let inspection = inspect_history(kv, Some(self.chunk_target_bytes));
        for breach in inspection.breaches.iter() {
            tracing::error!(
                log_id = %breach.log_id,
                kind = ?breach.kind,
                "ledger: lossy restore ignored authoritative-ledger breach"
            );
        }

        let mut restored_logs = 0usize;
        for (log_id, chunks) in inspection.by_log {
            let Some((tail_index, tail_doc)) = chunks.last_key_value() else {
                continue;
            };
            let Some(next_seq) = tail_doc
                .entries
                .last()
                .and_then(|entry| entry.seq.checked_add(1))
            else {
                continue;
            };
            let tail = if tail_doc.sealed {
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
                let mut open_bytes = 0usize;
                let mut valid = true;
                for entry in &tail_doc.entries {
                    let Ok(encoded) = encoded_len_bounded(entry, MAX_SAFE_CHUNK_BYTES) else {
                        valid = false;
                        break;
                    };
                    let Some(total) = open_bytes.checked_add(encoded) else {
                        valid = false;
                        break;
                    };
                    open_bytes = total;
                }
                if !valid {
                    continue;
                }
                LogTail {
                    open_chunk: *tail_index,
                    open_entries: tail_doc.entries.clone(),
                    next_seq,
                    open_bytes,
                }
            };
            self.logs.insert(log_id, tail);
            restored_logs = restored_logs.saturating_add(1);
        }
        LedgerRestoreReport {
            logs: restored_logs,
            chunks: inspection.chunks,
            entries: inspection.entries,
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

/// Parse `log:{log_id}:{chunk}` into `(log_id, chunk)`. The chunk is the final
/// `:`-segment; the log id is everything between the prefix and it (so a log id
/// may itself contain `:`). Returns `None` for any non-chunk key.
fn parse_chunk_key(key: &str) -> Option<(SmolStr, u64)> {
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

struct HistoryInspection {
    by_log: BTreeMap<SmolStr, BTreeMap<u64, LedgerChunk>>,
    breaches: BoundedBreaches,
    chunks: usize,
    entries: usize,
}

#[derive(Default)]
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

fn inspect_history(
    kv: &[(SmolStr, serde_json::Value)],
    chunk_target: Option<usize>,
) -> HistoryInspection {
    let mut by_log: BTreeMap<SmolStr, BTreeMap<u64, LedgerChunk>> = BTreeMap::new();
    let mut breaches = BoundedBreaches::default();
    let mut valid_chunks = 0usize;
    let mut valid_entries = 0usize;

    for (key, value) in kv {
        let Some((log_id, chunk_seq)) = parse_chunk_key(key) else {
            if key.starts_with(LOG_PREFIX) {
                breaches.push(ContiguityBreach {
                    log_id: malformed_key_log_id(key),
                    kind: BreachKind::MalformedKey,
                });
            }
            continue;
        };

        match encoded_len_bounded(value, MAX_SAFE_CHUNK_BYTES) {
            Ok(_) => {}
            Err(BoundedEncodeError::Limit { .. }) => {
                breaches.push(ContiguityBreach {
                    log_id,
                    kind: BreachKind::ChunkTooLarge { chunk: chunk_seq },
                });
                continue;
            }
            Err(BoundedEncodeError::Encoding) => {
                breaches.push(ContiguityBreach {
                    log_id,
                    kind: BreachKind::MalformedChunk { at: chunk_seq },
                });
                continue;
            }
        }

        match serde_json::from_value::<LedgerChunk>(value.clone()) {
            Ok(doc) if doc.chunk == chunk_seq => {
                if doc.entries.is_empty() {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::EmptyChunk { index: chunk_seq },
                    });
                }
                for (entry_index, entry) in doc.entries.iter().enumerate() {
                    if encoded_len_bounded(entry, MAX_SAFE_CHUNK_BYTES).is_err() {
                        breaches.push(ContiguityBreach {
                            log_id: log_id.clone(),
                            kind: BreachKind::EntryTooLarge {
                                chunk: chunk_seq,
                                entry: entry_index,
                            },
                        });
                    }
                }
                let chunks = by_log.entry(log_id.clone()).or_default();
                if chunks.insert(chunk_seq, doc).is_some() {
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

    for (log_id, chunks) in &by_log {
        let Some(&max) = chunks.keys().next_back() else {
            continue;
        };
        let mut prev: Option<u64> = None;
        for &idx in chunks.keys() {
            match prev {
                None if idx != 0 => breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::MissingChunk { index: 0 },
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

        let mut expected_seq = 0u64;
        for (&idx, doc) in chunks {
            if idx < max && !doc.sealed {
                breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::UnsealedNonTail { index: idx },
                });
            }
            for entry in &doc.entries {
                valid_entries = valid_entries.saturating_add(1);
                if entry.seq != expected_seq {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::SeqGap {
                            expected: expected_seq,
                            found: entry.seq,
                            chunk: idx,
                        },
                    });
                    expected_seq = entry.seq;
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

        if chunks.get(&max).is_some_and(|tail| tail.sealed) && max == u64::MAX {
            breaches.push(ContiguityBreach {
                log_id: log_id.clone(),
                kind: BreachKind::ArithmeticOverflow { chunk: max },
            });
        }
        if let (Some(target), Some(tail)) = (chunk_target, chunks.get(&max))
            && !tail.sealed
        {
            let mut open_bytes = 0usize;
            for entry in &tail.entries {
                let Ok(bytes) = encoded_len_bounded(entry, MAX_SAFE_CHUNK_BYTES) else {
                    continue;
                };
                let Some(total) = open_bytes.checked_add(bytes) else {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::ArithmeticOverflow { chunk: max },
                    });
                    break;
                };
                open_bytes = total;
            }
            if open_bytes >= target {
                breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::UnsealedAtOrOverTarget { chunk: max },
                });
            }
        }
    }

    HistoryInspection {
        by_log,
        breaches,
        chunks: valid_chunks,
        entries: valid_entries,
    }
}

fn bounded_restore_error(breaches: BoundedBreaches) -> LedgerRestoreError {
    let total_breaches = breaches.total;
    let details_truncated = total_breaches > breaches.details.len();
    LedgerRestoreError {
        total_breaches,
        details_truncated,
        breaches: breaches.details,
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
/// onward), which is true for the current boot path. If a future cold-chunk
/// boot-skip lands (see [`LOG_PREFIX`]), a tail-only kv would look like a missing
/// prefix here and this check must be revisited (e.g. keyed off a persisted
/// min-present marker).
///
/// Returned details are capped at [`MAX_RESTORE_BREACH_DETAILS`]. Checked
/// restore additionally reports the total and whether details were truncated.
#[must_use]
pub fn scan_contiguity(kv: &[(SmolStr, serde_json::Value)]) -> Vec<ContiguityBreach> {
    inspect_history(kv, None).breaches.details
}

#[cfg(test)]
#[allow(deprecated)] // legacy wrapper behavior remains covered during migration
mod tests {
    use super::*;

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
            .restore_from_kv(&[(key, serde_json::to_value(&doc).unwrap())])
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
            .restore_from_kv(&[
                (k0, serde_json::to_value(&doc0).unwrap()),
                (k1, serde_json::to_value(&doc1).unwrap()),
            ])
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
        led.restore_from_kv(&kv).unwrap();
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

    #[test]
    fn scan_contiguity_is_clean_for_a_well_formed_log() {
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, true, 3, 2),
            kv_chunk("s", 2, false, 5, 1), // open tail
            (SmolStr::new("meta"), serde_json::json!({"unrelated": true})),
        ];
        assert!(
            scan_contiguity(&kv).is_empty(),
            "a gap-free, seq-continuous log has no breach"
        );
    }

    #[test]
    fn scan_contiguity_flags_a_dropped_predecessor_chunk() {
        // Chunk 1 never persisted (a dropped seal), but chunk 2 exists — the
        // exact loss the byte-scan closes and this scan surfaces at boot.
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 2, false, 5, 1)];
        let breaches = scan_contiguity(&kv);
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
        let breaches = scan_contiguity(&kv);
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
        let breaches = scan_contiguity(&kv);
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
        let breaches = scan_contiguity(&kv);
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
        let breaches = scan_contiguity(&kv);
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
        assert!(!scan_contiguity(&kv).is_empty());
        let mut led = Ledger::new();
        let plan = led
            .prepare_append("existing", serde_json::json!(1))
            .unwrap();
        led.apply_prepared(plan).unwrap();
        let before = led.clone();
        let error = led.restore_from_kv(&kv).unwrap_err();
        assert!(error.total_breaches() > 0);
        assert_eq!(led, before);
    }

    #[test]
    fn explicitly_lossy_restore_can_rebuild_a_holed_non_authoritative_tail() {
        let kv = vec![kv_chunk("s", 0, true, 0, 3), kv_chunk("s", 2, false, 5, 1)];
        let mut led = Ledger::new();
        let report = led.restore_from_kv_lossy(&kv);
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
            let error = led.restore_from_kv(&kv).unwrap_err();
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
            .restore_from_kv(&[valid, duplicate, noncanonical, unknown_field])
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
        let error = led.restore_from_kv(&kv).unwrap_err();
        assert_eq!(error.total_breaches(), MAX_RESTORE_BREACH_DETAILS + 20);
        assert_eq!(error.breaches().len(), MAX_RESTORE_BREACH_DETAILS);
        assert!(error.details_truncated());
        assert_eq!(led, Ledger::new());
    }
}
