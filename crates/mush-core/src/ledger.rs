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
//! append opens a fresh chunk key. A sealed chunk is immutable history —
//! **never rewritten again** — so the per-append cost is bounded by one open
//! chunk, not the whole log.
//!
//! ## What lives in RAM
//!
//! Only the **open tail** of each log (the current unsealed chunk plus the
//! running counters). Sealed chunks are on disk; reading old history pages them
//! in through the module store-read seam (a later prerequisite). Until that
//! lands, [`Ledger`] exposes only the open tail — enough to keep appending, and
//! enough for the common case of an *active* scene, whose events are the tail.
//!
//! ## Durability window (known, narrow)
//!
//! An open-chunk rewrite that is dropped by the persistence layer self-heals:
//! the entries are still in RAM and the log's next append re-persists the whole
//! open chunk. The one non-self-healing case is a dropped **seal-and-roll-over**
//! append — the in-RAM ledger has already advanced to the next chunk, so the
//! sealed chunk is never rewritten. This can only happen on a *deterministic*
//! store rejection (transient failures retry the same batch until it lands), so
//! it is rare; the persistence layer logs it as an authoritative-ledger breach.
//! Closing it entirely needs ack-gated rollover (advance the open chunk only
//! after the sealing batch commits), which lands with the ack-integrated
//! store-read seam.
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
        tail.open_bytes += entry_bytes(&entry);
        tail.open_entries.push(entry);

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
    pub fn restore_from_kv(&mut self, kv: &[(SmolStr, serde_json::Value)]) {
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
}
