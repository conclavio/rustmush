//! The World-owned **retained-write table** (E0) — the mechanism that closes
//! the dropped-seal loss window in the authoritative ledger without any
//! consumer opting in.
//!
//! A sealed ledger chunk (`log:{id}:{chunk}` with `sealed: true`) is written
//! exactly once by [`mush_core::Ledger::append`], which then advances its
//! in-RAM tail — so if that write is dropped by the persistence layer, ordinary
//! appends never restate it (see the *Durability window* note in
//! [`mush_core::ledger`]). This table sits in the world loop and, recognizing a
//! seal purely by its bytes ([`mush_core::is_sealed_chunk`]), re-emits every
//! unacked seal on each batch build until a durable-commit high-water ack is
//! observed for it. Retention is thus a property of the op **bytes**: no module
//! registers anything, and none can forget to participate.
//!
//! ## Re-stamp semantics (why a global high-water is sound here)
//!
//! On every build, every seal that rides the batch — freshly produced OR
//! re-emitted — is (re)stamped with THAT batch's serial. Because [`reemit_ops`]
//! puts *every* pending seal into the batch, all pending seals carry the current
//! serial after each [`capture`], so the global commit high-water clears exactly
//! the seals whose most-recent carrying batch committed. This is sound even
//! though "batch S committed" does **not** imply "all batches ≤ S committed" (a
//! deterministic drop of S−1 while S commits): a pending seal's stamp always
//! equals the serial of the latest batch that actually carried it, and no higher
//! serial exists while it is still pending, so `high_water ≥ stamp` holds iff
//! that exact batch committed.
//!
//! [`reemit_ops`]: RetainedSeals::reemit_ops
//! [`capture`]: RetainedSeals::capture

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{ModuleOp, ModuleWrite, is_sealed_chunk};
use smol_str::SmolStr;

/// Identity of one retained seal: its module namespace and chunk key (which
/// together map to the persisted `mod:{ns}:{key}` document).
type SealKey = (SmolStr, SmolStr);

/// One unacked sealed-chunk write held for re-emit.
struct Pending {
    /// The whole-doc seal op's value, re-emitted byte-identically each build.
    value: serde_json::Value,
    /// The serial of the most recent batch that carried this seal.
    stamped: u64,
}

/// The world-owned table of sealed-chunk writes awaiting durable commit. See
/// the module docs.
#[derive(Default)]
pub struct RetainedSeals {
    pending: BTreeMap<SealKey, Pending>,
    /// Seals the writer has durably quarantined as poison: dropped from
    /// `pending` and never re-captured, so the world stops re-emitting a seal
    /// that can never commit. The world stays read-only (fail-closed) until
    /// staff repair the quarantined chunk and explicitly release.
    quarantined: BTreeSet<SealKey>,
}

impl RetainedSeals {
    /// A fresh, empty table.
    #[must_use]
    pub fn new() -> RetainedSeals {
        RetainedSeals::default()
    }

    /// Whether any seal is awaiting a durable ack. When true, the world loop
    /// arms its re-emit nudge so an idle world still re-flushes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Number of unacked seals — the depth gauge/alarm surface.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.pending.len()
    }

    /// Re-emit ops for every unacked seal — folded into the batch on every build
    /// so a dropped seal is rewritten on the next flush. Byte-identical to the
    /// original op, so a redundant rewrite is an idempotent no-op at the store.
    #[must_use]
    pub fn reemit_ops(&self) -> Vec<ModuleOp> {
        self.pending
            .iter()
            .map(|((ns, key), p)| ModuleOp {
                ns: ns.clone(),
                op: ModuleWrite::JsonSet { key: key.clone(), value: p.value.clone() },
            })
            .collect()
    }

    /// Record (and re-stamp) every sealed-chunk op present in a batch about to be
    /// handed to the writer, to `serial`. Freshly produced seals are inserted;
    /// already-pending (re-emitted) ones are re-stamped to the current serial. A
    /// quarantined seal is never re-captured. Call this on the batch that is then
    /// sent: the caller invokes it just before the send, and the only path where
    /// that send fails is teardown (the writer is gone), where a stale capture is
    /// harmless — so a pending stamp always names a batch that reached, or was in
    /// the act of reaching, the writer.
    pub fn capture(&mut self, ops: &[ModuleOp], serial: u64) {
        for op in ops {
            let ModuleWrite::JsonSet { key, value } = &op.op else {
                continue;
            };
            if !is_sealed_chunk(key, value) {
                continue;
            }
            let id = (op.ns.clone(), key.clone());
            if self.quarantined.contains(&id) {
                continue;
            }
            self.pending
                .entry(id)
                .and_modify(|p| p.stamped = serial)
                .or_insert_with(|| Pending { value: value.clone(), stamped: serial });
        }
    }

    /// Clear every seal whose stamped serial is at or below the durable-commit
    /// high-water — the batch that last carried it committed, so it is durable.
    pub fn ack(&mut self, high_water: u64) {
        self.pending.retain(|_, p| p.stamped > high_water);
    }

    /// Quarantine a poison seal: drop it from `pending` and refuse to re-capture
    /// it. Called when the writer confirms (via isolated retry) that this exact
    /// seal deterministically fails, and has durably preserved it in the
    /// quarantine spool. Idempotent.
    pub fn quarantine(&mut self, ns: SmolStr, key: SmolStr) {
        let id = (ns, key);
        self.pending.remove(&id);
        self.quarantined.insert(id);
    }

    /// Drop a seal the writer confirmed durable **in isolation** — it was
    /// collateral (some other op in its batch was the poison), and the isolated
    /// retry committed it. Unlike [`quarantine`](Self::quarantine) it is not
    /// blocked from future capture (chunks are immutable, so the key never
    /// recurs anyway); this just clears it promptly instead of waiting for the
    /// next re-emit's ack. Idempotent.
    pub fn healed(&mut self, ns: SmolStr, key: SmolStr) {
        self.pending.remove(&(ns, key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{LedgerChunk, LedgerEntry, chunk_key};

    /// A sealed-chunk `ModuleOp` in namespace `ns`.
    fn seal_op(ns: &str, log: &str, chunk: u64, seq0: u64, n: u64) -> ModuleOp {
        let entries = (seq0..seq0 + n)
            .map(|seq| LedgerEntry { seq, value: serde_json::json!(seq) })
            .collect();
        let value = serde_json::to_value(LedgerChunk { chunk, sealed: true, entries }).unwrap();
        ModuleOp { ns: SmolStr::new(ns), op: ModuleWrite::JsonSet { key: chunk_key(log, chunk), value } }
    }

    /// An OPEN (unsealed) chunk op — must never be retained (it self-heals).
    fn open_op(ns: &str, log: &str, chunk: u64) -> ModuleOp {
        let value = serde_json::to_value(LedgerChunk {
            chunk,
            sealed: false,
            entries: vec![LedgerEntry { seq: 0, value: serde_json::json!(0) }],
        })
        .unwrap();
        ModuleOp { ns: SmolStr::new(ns), op: ModuleWrite::JsonSet { key: chunk_key(log, chunk), value } }
    }

    /// A non-ledger module op (comsys write) — never retained.
    fn plain_op(ns: &str, key: &str) -> ModuleOp {
        ModuleOp {
            ns: SmolStr::new(ns),
            op: ModuleWrite::JsonSet { key: SmolStr::new(key), value: serde_json::json!({"x": 1}) },
        }
    }

    #[test]
    fn captures_only_sealed_chunks() {
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "s", 0, 0, 3), open_op("scenes", "s", 1), plain_op("comsys", "chan:1")], 5);
        assert_eq!(r.depth(), 1, "only the sealed chunk is retained");
        assert!(!r.is_empty());
        let re = r.reemit_ops();
        assert_eq!(re.len(), 1);
        assert!(matches!(&re[0].op, ModuleWrite::JsonSet { key, .. } if key == &chunk_key("s", 0)));
    }

    #[test]
    fn ack_clears_at_or_below_the_high_water() {
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "a", 0, 0, 1)], 3);
        r.capture(&[seal_op("scenes", "b", 0, 0, 1)], 5);
        r.ack(3); // a's batch committed; b's has not
        assert_eq!(r.depth(), 1, "the seal stamped 3 clears; the one stamped 5 stays");
        r.ack(5);
        assert!(r.is_empty(), "both cleared once the high-water reaches 5");
    }

    #[test]
    fn re_stamp_defers_clear_to_the_latest_carrying_batch() {
        // A seal captured at serial 4 whose batch DROPPED, then re-emitted at 5.
        // A high-water of 4 (some other batch ≤4 committed) must NOT clear it —
        // its own batch 4 dropped; only reaching 5 (its re-emit's commit) clears.
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "a", 0, 0, 1)], 4);
        // Re-emit next build: the same seal rides batch 5 and re-stamps to 5.
        let re = r.reemit_ops();
        r.capture(&re, 5);
        r.ack(4);
        assert_eq!(r.depth(), 1, "a stale/other-batch high-water of 4 must not clear the re-stamped seal");
        r.ack(5);
        assert!(r.is_empty(), "the re-emit's own commit clears it");
    }

    #[test]
    fn quarantine_drops_and_blocks_recapture() {
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("econ", "ledger", 7, 0, 2)], 9);
        assert_eq!(r.depth(), 1);
        r.quarantine(SmolStr::new("econ"), chunk_key("ledger", 7));
        assert!(r.is_empty(), "quarantine drops the poison seal from pending");
        // A later re-emit-and-capture of the SAME seal must not resurrect it.
        r.capture(&[seal_op("econ", "ledger", 7, 0, 2)], 12);
        assert!(r.is_empty(), "a quarantined seal is never re-captured");
        assert!(r.reemit_ops().is_empty());
    }

    #[test]
    fn same_serial_seals_from_one_batch_clear_together() {
        // Two logs sealing in the same batch share a serial; that serial names
        // exactly one batch, so its commit makes both durable.
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "a", 0, 0, 1), seal_op("scenes", "b", 0, 0, 1)], 8);
        assert_eq!(r.depth(), 2);
        r.ack(8);
        assert!(r.is_empty());
    }
}
