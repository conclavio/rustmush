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
//! Entries accumulate in an in-RAM **open chunk**. Each [`Ledger::append`]
//! emits one whole-document [`ModuleWrite::JsonSet`] rewriting that single
//! chunk (`log:{id}:{chunk}`). A whole-doc set is:
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
//! [`Ledger::append`] itself stays fire-and-forget and payload-agnostic — it
//! never advances the open chunk lazily and never blocks. The window is closed
//! *above* the ledger, in the world loop, by a **retained-write table**: every
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
//! [`Ledger::append`] advances in-RAM state **and** returns the persist op that
//! records it; the caller must queue that op in the same command. Because a
//! handler that mutates world state and then returns `Err` has its deferred
//! persist ops truncated but its world/state mutations kept (validate-then-
//! apply is the codebase rule, not a rollback), **append only on the success
//! path** — after the command has committed to succeeding. Appending an event
//! for an action that then fails records something that did not happen. The
//! testkit boundary rig asserts the mutation and its ledger op fold into one
//! write batch for well-behaved handlers.
//!
//! The retained-write table above does **not** relax this. It re-emits ops that
//! *were* built, so a seal op truncated on an `Err` path never enters it (its
//! retention half is moot). But [`append`](Ledger::append) has already advanced
//! the in-RAM tail past the seal, so the *next*, unrelated append would persist
//! a phantom sealed chunk for entries the failed command never truly committed.
//! Append-on-success is the invariant that prevents this; the retained-write
//! table is not a substitute for it.

use std::collections::BTreeMap;

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

/// A hard safety ceiling on a prospective chunk's approximate serialized size,
/// far above [`DEFAULT_CHUNK_TARGET_BYTES`] (normal chunks seal at the target,
/// nowhere near this) and far below any practical store value limit. A chunk
/// that approaches it is a **probable poison** — a single pathologically large
/// entry, or a mis-set chunk target — that the store would deterministically
/// reject. [`Ledger::append`] is infallible and fire-and-forget so it cannot
/// refuse; it surfaces the breach (the writer's quarantine is the real net) so
/// staff see it *before* it reaches the writer. See the *Durability window* note.
pub const MAX_SAFE_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// One appended entry: a monotonic per-log sequence number plus an opaque
/// payload the consumer defines (a scene event, a canon event, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Position in the log, 0-based, strictly increasing, gap-free.
    pub seq: u64,
    /// The consumer's payload.
    pub value: serde_json::Value,
}

/// One persisted chunk document (`log:{id}:{chunk}`): a contiguous run of
/// entries and whether the chunk is sealed (full and immutable).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

    /// Append `value` to log `log_id`, returning the entry's sequence number
    /// and the single [`ModuleWrite`] that persists it — a whole-document
    /// rewrite of the open chunk. The caller queues the write via
    /// [`Ctx::persist`](crate::cmd::Ctx::persist) in the **same** command, on
    /// the success path (see the module-level transaction-discipline note).
    ///
    /// The op rewrites `log:{log_id}:{chunk}`. When this append fills the open
    /// chunk, the op carries `sealed: true` and the next append opens a fresh
    /// chunk — so a sealed chunk is written exactly once and never touched
    /// again.
    pub fn append(&mut self, log_id: &str, value: serde_json::Value) -> (u64, ModuleWrite) {
        let target = self.chunk_target_bytes;
        let tail = self
            .logs
            .entry(SmolStr::new(log_id))
            .or_insert_with(LogTail::fresh);

        let seq = tail.next_seq;
        tail.next_seq += 1;
        let entry = LedgerEntry { seq, value };
        let eb = entry_bytes(&entry);
        tail.open_bytes += eb;
        tail.open_entries.push(entry);

        // Preventive size-check (E0): a prospective chunk past the safe ceiling
        // is a probable poison the store will deterministically reject. `append`
        // is infallible + fire-and-forget so it cannot refuse (the retained-write
        // table re-emits and the writer quarantines the offending batch — the
        // real safety net); surface it here so staff see it before the writer.
        if eb > MAX_SAFE_CHUNK_BYTES || tail.open_bytes > MAX_SAFE_CHUNK_BYTES {
            tracing::error!(
                log_id = %log_id,
                seq,
                entry_bytes = eb,
                chunk_bytes = tail.open_bytes,
                max = MAX_SAFE_CHUNK_BYTES,
                "ledger: prospective chunk exceeds the safe size ceiling — probable \
                 poison seal; the store will reject it and the batch will be quarantined"
            );
        }

        let full = tail.open_bytes >= target;
        let chunk = tail.open_chunk;
        let op = ModuleWrite::JsonSet {
            key: chunk_key(log_id, chunk),
            value: serde_json::to_value(LedgerChunk {
                chunk,
                sealed: full,
                entries: tail.open_entries.clone(),
            })
            .expect("ledger chunk serializes"),
        };

        if full {
            // Roll over: the just-written chunk is sealed and immutable; the
            // next append starts a fresh open chunk.
            tail.open_chunk += 1;
            tail.open_entries.clear();
            tail.open_bytes = 0;
        }
        (seq, op)
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

    /// Restore the open tail of every log from a module's boot kv documents.
    ///
    /// Scans `(key, value)` pairs for `log:{id}:{chunk}` chunk docs, groups by
    /// log id, and rebuilds each log's tail from its **highest** chunk:
    ///
    /// - a sealed tail chunk ⇒ the open chunk is the next index (empty), and
    ///   `next_seq` resumes past the sealed chunk's last entry;
    /// - an unsealed tail chunk ⇒ it *is* the open chunk; its entries and byte
    ///   count are restored so appends continue into it.
    ///
    /// Cold (non-tail) chunks are intentionally **not** retained in RAM — only
    /// the tail is needed to keep appending. Malformed chunk docs are skipped
    /// with a warning; a log with no parseable chunk is left untouched.
    /// Non-`log:` keys are ignored, so a module can hand its whole boot kv in.
    ///
    /// Before restoring, the boot kv is run through [`scan_contiguity`] and any
    /// breach (a missing predecessor chunk, an unsealed interior chunk, a seq
    /// gap) is logged as an **authoritative-ledger breach** — restore still
    /// rebuilds the tail from the highest chunk (so the world boots), but the
    /// hole is surfaced instead of silently served. A money-bearing consumer
    /// that must fail closed should call [`scan_contiguity`] itself and refuse
    /// boot on a non-empty result.
    pub fn restore_from_kv(&mut self, kv: &[(SmolStr, serde_json::Value)]) {
        for breach in scan_contiguity(kv) {
            tracing::error!(
                log_id = %breach.log_id,
                kind = ?breach.kind,
                "ledger: authoritative-ledger contiguity breach at boot"
            );
        }

        // Highest chunk seen per log id, and its parsed doc.
        let mut tails: BTreeMap<SmolStr, LedgerChunk> = BTreeMap::new();
        for (key, value) in kv {
            let Some((log_id, chunk_seq)) = parse_chunk_key(key) else {
                continue;
            };
            match serde_json::from_value::<LedgerChunk>(value.clone()) {
                Ok(doc) => {
                    if doc.chunk != chunk_seq {
                        tracing::warn!(%key, "ledger: chunk doc index mismatch, skipped");
                        continue;
                    }
                    let keep = tails
                        .get(&log_id)
                        .is_none_or(|prev| doc.chunk > prev.chunk);
                    if keep {
                        tails.insert(log_id, doc);
                    }
                }
                Err(e) => {
                    tracing::warn!(%key, error = %e, "ledger: unparseable chunk doc skipped");
                }
            }
        }

        for (log_id, tail_doc) in tails {
            let last_seq = tail_doc.entries.last().map(|e| e.seq);
            let next_seq = last_seq.map_or(0, |s| s + 1);
            let restored = if tail_doc.sealed {
                LogTail {
                    open_chunk: tail_doc.chunk + 1,
                    open_entries: Vec::new(),
                    next_seq,
                    open_bytes: 0,
                }
            } else {
                let open_bytes = tail_doc.entries.iter().map(entry_bytes).sum();
                LogTail {
                    open_chunk: tail_doc.chunk,
                    open_entries: tail_doc.entries,
                    next_seq,
                    open_bytes,
                }
            };
            self.logs.insert(log_id, restored);
        }
    }
}

/// Approximate serialized size of one entry, for the seal decision.
fn entry_bytes(e: &LedgerEntry) -> usize {
    serde_json::to_vec(e).map_or(0, |v| v.len())
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

/// Whether a persisted module write `(key, value)` is a **sealed** ledger chunk —
/// a `log:{id}:{chunk}` document whose `sealed` field is `true`. This is the
/// single-source definition of a *retained write*: the world loop's byte-scan
/// recognizes exactly these ops and re-emits them until a durable ack, so the
/// dropped-seal loss window closes without any consumer opting in. A cheap
/// field probe, not a full deserialize — an open (unsealed) chunk self-heals on
/// the log's next append and is deliberately excluded.
#[must_use]
pub fn is_sealed_chunk(key: &str, value: &serde_json::Value) -> bool {
    is_log_key(key) && value.get("sealed").and_then(serde_json::Value::as_bool) == Some(true)
}

/// Parse `log:{log_id}:{chunk}` into `(log_id, chunk)`. The chunk is the final
/// `:`-segment; the log id is everything between the prefix and it (so a log id
/// may itself contain `:`). Returns `None` for any non-chunk key.
fn parse_chunk_key(key: &str) -> Option<(SmolStr, u64)> {
    let rest = key.strip_prefix(LOG_PREFIX)?;
    let (log_id, chunk) = rest.rsplit_once(':')?;
    if log_id.is_empty() {
        return None;
    }
    let chunk = chunk.parse::<u64>().ok()?;
    Some((SmolStr::new(log_id), chunk))
}

/// A structural break in a log's persisted chunk history — an
/// **authoritative-ledger breach**. `restore_from_kv` only ever selects the
/// highest chunk, so on its own it can silently serve a log with a missing
/// predecessor; [`scan_contiguity`] surfaces the hole so a boot can log it (and,
/// for money-bearing logs, refuse to serve a silently-holed ledger).
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
    /// A chunk index or sequence number arithmetic overflowed while validating —
    /// treated as corruption rather than wrapping silently.
    ArithmeticOverflow {
        /// The chunk being validated when the overflow occurred.
        chunk: u64,
    },
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
#[must_use]
pub fn scan_contiguity(kv: &[(SmolStr, serde_json::Value)]) -> Vec<ContiguityBreach> {
    let mut by_log: BTreeMap<SmolStr, BTreeMap<u64, LedgerChunk>> = BTreeMap::new();
    let mut breaches = Vec::new();
    for (key, value) in kv {
        let Some((log_id, chunk_seq)) = parse_chunk_key(key) else {
            continue;
        };
        // A malformed or index-mismatched authoritative doc is a breach — surface
        // it, never silently drop it (else a wholly-corrupt tail reads as clean).
        match serde_json::from_value::<LedgerChunk>(value.clone()) {
            Ok(doc) if doc.chunk == chunk_seq => {
                by_log.entry(log_id).or_default().insert(chunk_seq, doc);
            }
            Ok(doc) => breaches.push(ContiguityBreach {
                log_id,
                kind: BreachKind::IndexMismatch { at: chunk_seq, embedded: doc.chunk },
            }),
            Err(_) => breaches.push(ContiguityBreach {
                log_id,
                kind: BreachKind::MalformedChunk { at: chunk_seq },
            }),
        }
    }

    for (log_id, chunks) in &by_log {
        let Some(&max) = chunks.keys().next_back() else { continue };
        // Detect missing chunks by walking the PRESENT indices in order (bounded
        // by the number of persisted chunks), NOT by iterating `0..=max` — a
        // corrupted/bit-flipped chunk index (e.g. 4e9) must never turn boot
        // validation into a multi-billion-entry allocation. Report the first
        // missing index of each gap, plus a missing chunk-0 prefix; that is
        // enough to fail closed on a hole. Arithmetic is checked (a wrap would be
        // corruption, not a valid successor).
        let mut prev: Option<u64> = None;
        for &idx in chunks.keys() {
            match prev {
                None if idx != 0 => breaches.push(ContiguityBreach {
                    log_id: log_id.clone(),
                    kind: BreachKind::MissingChunk { index: 0 },
                }),
                Some(p) => match p.checked_add(1) {
                    Some(next) if idx != next => breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::MissingChunk { index: next },
                    }),
                    None => breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::ArithmeticOverflow { chunk: p },
                    }),
                    _ => {}
                },
                _ => {}
            }
            prev = Some(idx);
        }
        let mut expected_seq: u64 = 0;
        for (&idx, doc) in chunks {
            if idx < max && !doc.sealed {
                breaches.push(ContiguityBreach { log_id: log_id.clone(), kind: BreachKind::UnsealedNonTail { index: idx } });
            }
            for e in &doc.entries {
                if e.seq != expected_seq {
                    breaches.push(ContiguityBreach {
                        log_id: log_id.clone(),
                        kind: BreachKind::SeqGap { expected: expected_seq, found: e.seq, chunk: idx },
                    });
                    expected_seq = e.seq; // resync so one gap does not cascade
                }
                match expected_seq.checked_add(1) {
                    Some(n) => expected_seq = n,
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
    }
    breaches
}

#[cfg(test)]
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
        assert_eq!(k1, chunk_key("s", 1), "next append writes a fresh chunk key");
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
        rebooted.restore_from_kv(&[(key, serde_json::to_value(&doc).unwrap())]);
        assert_eq!(rebooted.next_seq("s"), 2, "resumes past the loaded entries");
        assert_eq!(rebooted.open_chunk("s"), 0, "the unsealed chunk is still open");
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
        led.append("s", serde_json::json!(0)); // chunk 0, sealed
        let (_, op1) = led.append("s", serde_json::json!(1)); // chunk 1, sealed
        let (k1, doc1) = chunk_of(&op1);
        let mut rebooted = Ledger::new();
        rebooted.restore_from_kv(&[(k1, serde_json::to_value(&doc1).unwrap())]);
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
            entries: vec![LedgerEntry { seq: 0, value: serde_json::json!("a") }],
        };
        let c1 = LedgerChunk {
            chunk: 1,
            sealed: true,
            entries: vec![LedgerEntry { seq: 1, value: serde_json::json!("b") }],
        };
        let c2 = LedgerChunk {
            chunk: 2,
            sealed: false,
            entries: vec![LedgerEntry { seq: 2, value: serde_json::json!("c") }],
        };
        let kv = vec![
            (chunk_key("s", 0), serde_json::to_value(&c0).unwrap()),
            (chunk_key("s", 1), serde_json::to_value(&c1).unwrap()),
            (chunk_key("s", 2), serde_json::to_value(&c2).unwrap()),
            (SmolStr::new("meta"), serde_json::json!({"unrelated": true})),
        ];
        let mut led = Ledger::new();
        led.restore_from_kv(&kv);
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
        assert_eq!(parse_chunk_key("log:scene:42:7"), Some((SmolStr::new("scene:42"), 7)));
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
        assert_eq!(round, serde_json::to_value(LedgerChunk {
            chunk: 0,
            sealed: false,
            entries: vec![
                LedgerEntry { seq: 0, value: serde_json::json!("a") },
                LedgerEntry { seq: 1, value: serde_json::json!("b") },
            ],
        }).unwrap());
    }

    /// Build a `log:s:{chunk}` kv doc with a contiguous seq run `[first..first+n)`.
    fn kv_chunk(log: &str, chunk: u64, sealed: bool, first: u64, n: u64) -> (SmolStr, serde_json::Value) {
        let entries = (first..first + n)
            .map(|seq| LedgerEntry { seq, value: serde_json::json!(seq) })
            .collect();
        (chunk_key(log, chunk), serde_json::to_value(LedgerChunk { chunk, sealed, entries }).unwrap())
    }

    #[test]
    fn is_sealed_chunk_recognizes_only_sealed_log_docs() {
        let (sealed_key, sealed_val) = kv_chunk("s", 0, true, 0, 2);
        let (open_key, open_val) = kv_chunk("s", 1, false, 2, 1);
        assert!(is_sealed_chunk(&sealed_key, &sealed_val), "a sealed chunk op is retained");
        assert!(!is_sealed_chunk(&open_key, &open_val), "an open chunk self-heals; not retained");
        // A non-ledger doc that happens to carry `sealed:true` is not a chunk.
        assert!(!is_sealed_chunk("comsys:chan:1", &serde_json::json!({"sealed": true})));
    }

    #[test]
    fn scan_contiguity_is_clean_for_a_well_formed_log() {
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, true, 3, 2),
            kv_chunk("s", 2, false, 5, 1), // open tail
            (SmolStr::new("meta"), serde_json::json!({"unrelated": true})),
        ];
        assert!(scan_contiguity(&kv).is_empty(), "a gap-free, seq-continuous log has no breach");
    }

    #[test]
    fn scan_contiguity_flags_a_dropped_predecessor_chunk() {
        // Chunk 1 never persisted (a dropped seal), but chunk 2 exists — the
        // exact loss the byte-scan closes and this scan surfaces at boot.
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 2, false, 5, 1),
        ];
        let breaches = scan_contiguity(&kv);
        assert!(
            breaches.iter().any(|b| b.log_id == "s" && b.kind == BreachKind::MissingChunk { index: 1 }),
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
            (chunk_key("s", huge), serde_json::to_value(LedgerChunk {
                chunk: huge,
                sealed: false,
                entries: vec![LedgerEntry { seq: 1, value: serde_json::json!(1) }],
            }).unwrap()),
        ];
        let breaches = scan_contiguity(&kv);
        // Exactly one missing-chunk breach (the first gap index), not ~4e9.
        let missing: Vec<_> = breaches
            .iter()
            .filter(|b| matches!(b.kind, BreachKind::MissingChunk { .. }))
            .collect();
        assert_eq!(missing.len(), 1, "one bounded gap breach, not a per-index flood");
        assert!(matches!(missing[0].kind, BreachKind::MissingChunk { index: 1 }));
    }

    #[test]
    fn scan_contiguity_surfaces_malformed_and_mismatched_chunks() {
        // A malformed authoritative doc and an index-mismatched one must be
        // reported, not silently skipped — else a fail-closed consumer reads a
        // wholly-corrupt tail as "clean".
        let mismatched = serde_json::to_value(LedgerChunk {
            chunk: 9, // embedded index disagrees with the key's chunk 1
            sealed: true,
            entries: vec![LedgerEntry { seq: 0, value: serde_json::json!(0) }],
        })
        .unwrap();
        let kv = vec![
            (chunk_key("s", 0), serde_json::json!({"not": "a chunk"})),
            (chunk_key("s", 1), mismatched),
        ];
        let breaches = scan_contiguity(&kv);
        assert!(
            breaches.iter().any(|b| matches!(b.kind, BreachKind::MalformedChunk { at: 0 })),
            "unparseable chunk surfaced: {breaches:?}"
        );
        assert!(
            breaches.iter().any(|b| matches!(b.kind, BreachKind::IndexMismatch { at: 1, embedded: 9 })),
            "index-mismatched chunk surfaced: {breaches:?}"
        );
        assert!(!breaches.is_empty(), "a corrupt tail is never reported clean");
    }

    #[test]
    fn scan_contiguity_flags_an_unsealed_interior_chunk() {
        let kv = vec![
            kv_chunk("s", 0, false, 0, 3), // interior but NOT sealed
            kv_chunk("s", 1, false, 3, 1),
        ];
        let breaches = scan_contiguity(&kv);
        assert!(
            breaches.iter().any(|b| b.kind == BreachKind::UnsealedNonTail { index: 0 }),
            "the unsealed interior chunk 0 is reported: {breaches:?}"
        );
    }

    #[test]
    fn scan_contiguity_flags_a_sequence_gap() {
        // Chunk 1 starts at seq 9, not 3 — a discontinuity within a contiguous
        // chunk run (e.g. a corrupted/edited chunk).
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 1, false, 9, 1),
        ];
        let breaches = scan_contiguity(&kv);
        assert!(
            breaches.iter().any(|b| matches!(b.kind, BreachKind::SeqGap { expected: 3, found: 9, chunk: 1 })),
            "the seq gap at chunk 1 is reported: {breaches:?}"
        );
    }

    #[test]
    fn restore_still_rebuilds_the_tail_despite_a_contiguity_breach() {
        // A holed log (chunk 1 dropped) must still boot from the highest chunk —
        // restore logs the breach but does not refuse; a fail-closed consumer
        // calls scan_contiguity itself.
        let kv = vec![
            kv_chunk("s", 0, true, 0, 3),
            kv_chunk("s", 2, false, 5, 1),
        ];
        assert!(!scan_contiguity(&kv).is_empty());
        let mut led = Ledger::new();
        led.restore_from_kv(&kv);
        assert_eq!(led.open_chunk("s"), 2, "tail rebuilt from the highest chunk");
        assert_eq!(led.next_seq("s"), 6);
        assert_eq!(led.open_entries("s").len(), 1);
    }
}
