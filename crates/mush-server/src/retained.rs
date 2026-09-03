//! The World-owned **retained-write table** (E0) — the mechanism that closes
//! the dropped-seal loss window in the authoritative ledger without any
//! consumer opting in.
//!
//! A sealed ledger chunk (`log:{id}:{chunk}` with `sealed: true`) is emitted
//! exactly once by the checked [`mush_core::Ledger::prepare_append`] /
//! [`mush_core::Ledger::apply_prepared`] transition, which then advances its
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
//! ## Scope: seal durability, NOT bundle atomicity (E0 vs E2)
//!
//! This retains sealed **ledger chunks** — the authoritative event history — and
//! guarantees a dropped seal is not lost. It does **not** make a whole
//! authoritative *write bundle* (journal entry + account projections + holds +
//! idempotency record + objects/spatial changes) atomic: a batch's non-seal ops
//! do not self-heal (a dropped `Del` resurrects; a dropped `StreamAppend` is
//! lost), and Redis `MULTI/EXEC` can partially apply on an execution-time error.
//! Poison isolation may therefore commit a seal while its companion ops are
//! absent. Closing that — retaining/replaying the complete bundle, or treating
//! the ledger as sole authority with a reconciling rebuild of every projection —
//! is **E2** (the economy layer), which the table is shaped to extend (it can
//! hold explicitly-tagged bundle members later without re-architecture). E0's
//! guarantee is narrower and exact: the sealed authoritative history survives a
//! dropped seal, and money-moves ride the durable ack-gate so a confirmation is
//! never issued for a batch that did not commit.
//!
//! [`reemit_ops`]: RetainedSeals::reemit_ops
//! [`capture`]: RetainedSeals::capture

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use mush_core::{ModuleOp, ModuleWrite, is_sealed_chunk};
use smol_str::SmolStr;

/// Maximum exact seal identities kept in process custody (pending plus
/// exceptionally resolved). Normal acknowledgements remove pending entries;
/// poison/heal resolutions move one pending entry into `resolved`, so they do
/// not increase this total. The writer-signal mailbox uses the same ceiling,
/// which makes every accepted retained identity representable there.
pub const MAX_RETAINED_SEALS: usize = 1_024;

/// Maximum encoded bytes held by the retained table. This includes namespace,
/// key, payload hash, and the JSON post-image for pending entries. Re-emission
/// briefly clones at most this much again into one bounded authoritative batch.
pub const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;

/// Identity of one retained seal: its module namespace and chunk key (which
/// together map to the persisted `mod:{ns}:{key}` document).
type SealKey = (SmolStr, SmolStr);

/// Exact identity of one immutable seal payload. A namespace/key alone is not
/// sufficient recovery authority: corrupt input can present different bytes at
/// the same persisted location.
type SealIdentity = (SmolStr, SmolStr, String);

/// A caller attempted to retain two different immutable seal payloads for one
/// persisted namespace/key. Continuing would let one value overwrite the other
/// while an acknowledgement ambiguously cleared both.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "conflicting sealed-ledger payloads for {ns}/{key}: retained {retained_hash}, incoming {incoming_hash}"
)]
pub struct RetainedSealConflict {
    /// Module namespace containing the immutable chunk.
    pub ns: SmolStr,
    /// Module key containing the immutable chunk.
    pub key: SmolStr,
    /// SHA-256 identity already retained or quarantined.
    pub retained_hash: String,
    /// SHA-256 identity of the conflicting incoming payload.
    pub incoming_hash: String,
}

/// A candidate retained-seal set was rejected atomically.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RetainedSealError {
    /// Two payloads claimed one immutable namespace/key.
    #[error(transparent)]
    Conflict(#[from] RetainedSealConflict),
    /// Accepting the candidate would exceed process custody.
    #[error(
        "retained sealed-ledger custody exhausted: {seals}/{max_seals} identities, {bytes}/{max_bytes} bytes"
    )]
    Capacity {
        /// Projected identity count.
        seals: usize,
        /// Hard identity-count ceiling.
        max_seals: usize,
        /// Projected accounted bytes.
        bytes: usize,
        /// Hard byte ceiling.
        max_bytes: usize,
    },
    /// A JSON value could not be measured without allocating an unbounded
    /// encoding. `serde_json::Value` should always encode; treat a failure as
    /// an integrity error rather than accepting unaccounted custody.
    #[error("cannot account retained seal {ns}/{key}: {detail}")]
    Encoding {
        /// Persistence namespace.
        ns: SmolStr,
        /// Persistence key.
        key: SmolStr,
        /// Encoder diagnostic.
        detail: String,
    },
}

/// One unacked sealed-chunk write held for re-emit.
struct Pending {
    /// The whole-doc seal op's value, re-emitted byte-identically each build.
    value: serde_json::Value,
    /// Exact namespace/key/payload identity used by writer resolution signals.
    payload_hash: String,
    /// The serial of the most recent batch that carried this seal.
    stamped: u64,
    /// Namespace + key + hash + canonical JSON bytes charged to the table.
    accounted_bytes: usize,
}

/// The world-owned table of sealed-chunk writes awaiting durable commit. See
/// the module docs.
#[derive(Default)]
pub struct RetainedSeals {
    pending: BTreeMap<SealKey, Pending>,
    /// Exact seals the writer has resolved durably: either committed in
    /// isolation as collateral or preserved in quarantine as poison. They are
    /// dropped from `pending` and never re-captured. Retaining this small set
    /// also makes duplicate writer-resolution signals idempotent.
    resolved: BTreeSet<SealIdentity>,
    /// Exact bytes charged across `pending` and `resolved`.
    accounted_bytes: usize,
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

    /// Pending plus exceptionally resolved identities held this process life.
    #[must_use]
    pub fn custody_depth(&self) -> usize {
        self.pending.len().saturating_add(self.resolved.len())
    }

    /// Bytes charged to retained process custody.
    #[must_use]
    pub fn custody_bytes(&self) -> usize {
        self.accounted_bytes
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
                op: ModuleWrite::JsonSet {
                    key: key.clone(),
                    value: p.value.clone(),
                },
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
    pub fn capture(&mut self, ops: &[ModuleOp], serial: u64) -> Result<(), RetainedSealError> {
        self.capture_with_limits(ops, serial, MAX_RETAINED_SEALS, MAX_RETAINED_BYTES)
    }

    fn capture_with_limits(
        &mut self,
        ops: &[ModuleOp],
        serial: u64,
        max_seals: usize,
        max_bytes: usize,
    ) -> Result<(), RetainedSealError> {
        // Validate the complete candidate set before changing any stamp. This
        // keeps a rejected batch from partially advancing acknowledgement
        // identities in the retained table.
        let mut candidates: BTreeMap<SealKey, (String, serde_json::Value, usize)> = BTreeMap::new();
        let mut new_count = 0_usize;
        let mut new_bytes = 0_usize;
        for op in ops {
            let ModuleWrite::JsonSet { key, value } = &op.op else {
                continue;
            };
            if op.ns.contains(':')
                || op.ns.len() > mush_core::MAX_MODULE_NAMESPACE_BYTES
                || !is_sealed_chunk(key, value)
            {
                continue;
            }
            let id = (op.ns.clone(), key.clone());
            let payload_hash = crate::quarantine::seal_identity_hash(&op.ns, key, value);
            if let Some((retained_hash, _, _)) = candidates.get(&id)
                && retained_hash != &payload_hash
            {
                return Err(conflict(&id, retained_hash, &payload_hash).into());
            }
            if let Some(pending) = self.pending.get(&id)
                && pending.payload_hash != payload_hash
            {
                return Err(conflict(&id, &pending.payload_hash, &payload_hash).into());
            }
            if let Some((_, _, retained_hash)) = self
                .resolved
                .iter()
                .find(|(ns, candidate_key, _)| ns == &op.ns && candidate_key == key)
                && retained_hash != &payload_hash
            {
                return Err(conflict(&id, retained_hash, &payload_hash).into());
            }
            if candidates.contains_key(&id) {
                continue;
            }
            let exact = (id.0.clone(), id.1.clone(), payload_hash.clone());
            let accounted = if self.resolved.contains(&exact) {
                identity_bytes(&id.0, &id.1, &payload_hash)
            } else if let Some(pending) = self.pending.get(&id) {
                pending.accounted_bytes
            } else {
                match pending_bytes_bounded(&id.0, &id.1, &payload_hash, value, max_bytes).map_err(
                    |detail| RetainedSealError::Encoding {
                        ns: id.0.clone(),
                        key: id.1.clone(),
                        detail,
                    },
                )? {
                    Some(bytes) => bytes,
                    None => {
                        return Err(RetainedSealError::Capacity {
                            seals: self
                                .custody_depth()
                                .saturating_add(new_count)
                                .saturating_add(1),
                            max_seals,
                            bytes: max_bytes.saturating_add(1),
                            max_bytes,
                        });
                    }
                }
            };
            if !self.resolved.contains(&exact) && !self.pending.contains_key(&id) {
                new_count = new_count.saturating_add(1);
                new_bytes = new_bytes.saturating_add(accounted);
                let seals = self.custody_depth().saturating_add(new_count);
                let bytes = self.accounted_bytes.saturating_add(new_bytes);
                if seals > max_seals || bytes > max_bytes {
                    return Err(RetainedSealError::Capacity {
                        seals,
                        max_seals,
                        bytes,
                        max_bytes,
                    });
                }
            }
            candidates.insert(id, (payload_hash, value.clone(), accounted));
        }

        for (id, (payload_hash, value, accounted_bytes)) in candidates {
            let exact = (id.0.clone(), id.1.clone(), payload_hash.clone());
            if self.resolved.contains(&exact) {
                continue;
            }
            if let Some(pending) = self.pending.get_mut(&id) {
                pending.stamped = serial;
            } else {
                self.accounted_bytes = self.accounted_bytes.saturating_add(accounted_bytes);
                self.pending.insert(
                    id,
                    Pending {
                        value,
                        payload_hash,
                        stamped: serial,
                        accounted_bytes,
                    },
                );
            }
        }
        Ok(())
    }

    /// Clear every seal whose stamped serial is at or below the durable-commit
    /// high-water — the batch that last carried it committed, so it is durable.
    pub fn ack(&mut self, high_water: u64) {
        let acked: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(key, pending)| (pending.stamped <= high_water).then_some(key.clone()))
            .collect();
        for key in acked {
            if let Some(pending) = self.pending.remove(&key) {
                self.accounted_bytes = self.accounted_bytes.saturating_sub(pending.accounted_bytes);
            }
        }
    }

    /// Quarantine a poison seal: drop it from `pending` and refuse to re-capture
    /// it. Called when the writer confirms (via isolated retry) that this exact
    /// seal deterministically fails, and has durably preserved it in the
    /// quarantine spool. Idempotent.
    pub fn quarantine(&mut self, ns: SmolStr, key: SmolStr, payload_hash: String) -> bool {
        self.resolve_exact(ns, key, payload_hash)
    }

    /// Drop a seal the writer confirmed durable **in isolation** — it was
    /// collateral (some other op in its batch was the poison), and the isolated
    /// retry committed it. Unlike [`quarantine`](Self::quarantine) it is not
    /// re-emitted again: the exact immutable payload is already durable, so a
    /// later duplicate capture or resolution is an idempotent no-op.
    pub fn healed(&mut self, ns: SmolStr, key: SmolStr, payload_hash: &str) -> bool {
        self.resolve_exact(ns, key, payload_hash.to_owned())
    }

    fn resolve_exact(&mut self, ns: SmolStr, key: SmolStr, payload_hash: String) -> bool {
        let exact = (ns.clone(), key.clone(), payload_hash.clone());
        if self.resolved.contains(&exact) {
            return true;
        }
        let id = (ns, key);
        let matched = self
            .pending
            .get(&id)
            .is_some_and(|pending| pending.payload_hash == payload_hash);
        if matched {
            let pending = self
                .pending
                .remove(&id)
                .expect("matched pending seal must still exist");
            self.accounted_bytes = self
                .accounted_bytes
                .saturating_sub(pending.accounted_bytes)
                .saturating_add(identity_bytes(&exact.0, &exact.1, &exact.2));
            self.resolved.insert(exact);
        }
        matched
    }
}

fn identity_bytes(ns: &str, key: &str, payload_hash: &str) -> usize {
    ns.len()
        .saturating_add(key.len())
        .saturating_add(payload_hash.len())
}

fn pending_bytes_bounded(
    ns: &str,
    key: &str,
    payload_hash: &str,
    value: &serde_json::Value,
    max_bytes: usize,
) -> Result<Option<usize>, String> {
    let identity = identity_bytes(ns, key, payload_hash);
    let Some(value_limit) = max_bytes.checked_sub(identity) else {
        return Ok(None);
    };
    let mut counter = BoundedByteCounter::new(value_limit);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(Some(identity.saturating_add(counter.written))),
        Err(_) if counter.exceeded => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

struct BoundedByteCounter {
    written: usize,
    limit: usize,
    exceeded: bool,
}

impl BoundedByteCounter {
    const fn new(limit: usize) -> Self {
        Self {
            written: 0,
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.written) {
            self.exceeded = true;
            return Err(io::Error::other("retained seal exceeds byte custody"));
        }
        self.written = self.written.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn conflict(id: &SealKey, retained_hash: &str, incoming_hash: &str) -> RetainedSealConflict {
    RetainedSealConflict {
        ns: id.0.clone(),
        key: id.1.clone(),
        retained_hash: retained_hash.to_owned(),
        incoming_hash: incoming_hash.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{LedgerChunk, LedgerEntry, chunk_key};

    /// A sealed-chunk `ModuleOp` in namespace `ns`.
    fn seal_op(ns: &str, log: &str, chunk: u64, seq0: u64, n: u64) -> ModuleOp {
        let entries = (seq0..seq0 + n)
            .map(|seq| LedgerEntry {
                seq,
                value: serde_json::json!(seq),
            })
            .collect();
        let value = serde_json::to_value(LedgerChunk {
            chunk,
            sealed: true,
            entries,
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

    /// An OPEN (unsealed) chunk op — must never be retained (it self-heals).
    fn open_op(ns: &str, log: &str, chunk: u64) -> ModuleOp {
        let value = serde_json::to_value(LedgerChunk {
            chunk,
            sealed: false,
            entries: vec![LedgerEntry {
                seq: 0,
                value: serde_json::json!(0),
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

    /// A non-ledger module op (comsys write) — never retained.
    fn plain_op(ns: &str, key: &str) -> ModuleOp {
        ModuleOp {
            ns: SmolStr::new(ns),
            op: ModuleWrite::JsonSet {
                key: SmolStr::new(key),
                value: serde_json::json!({"x": 1}),
            },
        }
    }

    fn identity(op: &ModuleOp) -> String {
        let ModuleWrite::JsonSet { key, value } = &op.op else {
            panic!("test seal must be JsonSet")
        };
        crate::quarantine::seal_identity_hash(&op.ns, key, value)
    }

    #[test]
    fn captures_only_sealed_chunks() {
        let mut r = RetainedSeals::new();
        r.capture(
            &[
                seal_op("scenes", "s", 0, 0, 3),
                open_op("scenes", "s", 1),
                plain_op("comsys", "chan:1"),
            ],
            5,
        )
        .unwrap();
        assert_eq!(r.depth(), 1, "only the sealed chunk is retained");
        assert!(!r.is_empty());
        let re = r.reemit_ops();
        assert_eq!(re.len(), 1);
        assert!(matches!(&re[0].op, ModuleWrite::JsonSet { key, .. } if key == &chunk_key("s", 0)));
    }

    #[test]
    fn ack_clears_at_or_below_the_high_water() {
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "a", 0, 0, 1)], 3).unwrap();
        r.capture(&[seal_op("scenes", "b", 0, 0, 1)], 5).unwrap();
        r.ack(3); // a's batch committed; b's has not
        assert_eq!(
            r.depth(),
            1,
            "the seal stamped 3 clears; the one stamped 5 stays"
        );
        r.ack(5);
        assert!(r.is_empty(), "both cleared once the high-water reaches 5");
    }

    #[test]
    fn re_stamp_defers_clear_to_the_latest_carrying_batch() {
        // A seal captured at serial 4 whose batch DROPPED, then re-emitted at 5.
        // A high-water of 4 (some other batch ≤4 committed) must NOT clear it —
        // its own batch 4 dropped; only reaching 5 (its re-emit's commit) clears.
        let mut r = RetainedSeals::new();
        r.capture(&[seal_op("scenes", "a", 0, 0, 1)], 4).unwrap();
        // Re-emit next build: the same seal rides batch 5 and re-stamps to 5.
        let re = r.reemit_ops();
        r.capture(&re, 5).unwrap();
        r.ack(4);
        assert_eq!(
            r.depth(),
            1,
            "a stale/other-batch high-water of 4 must not clear the re-stamped seal"
        );
        r.ack(5);
        assert!(r.is_empty(), "the re-emit's own commit clears it");
    }

    #[test]
    fn quarantine_drops_and_blocks_recapture() {
        let mut r = RetainedSeals::new();
        let seal = seal_op("econ", "ledger", 7, 0, 2);
        let payload_hash = identity(&seal);
        r.capture(&[seal], 9).unwrap();
        assert_eq!(r.depth(), 1);
        assert!(r.quarantine(
            SmolStr::new("econ"),
            chunk_key("ledger", 7),
            payload_hash.clone(),
        ));
        assert!(
            r.quarantine(SmolStr::new("econ"), chunk_key("ledger", 7), payload_hash,),
            "duplicate exact resolution is idempotent"
        );
        assert!(
            r.is_empty(),
            "quarantine drops the poison seal from pending"
        );
        // A later re-emit-and-capture of the SAME seal must not resurrect it.
        r.capture(&[seal_op("econ", "ledger", 7, 0, 2)], 12)
            .unwrap();
        assert!(r.is_empty(), "a quarantined seal is never re-captured");
        assert!(r.reemit_ops().is_empty());
    }

    #[test]
    fn resolution_must_match_the_exact_payload() {
        let mut r = RetainedSeals::new();
        let seal = seal_op("econ", "ledger", 7, 0, 2);
        let payload_hash = identity(&seal);
        r.capture(&[seal], 9).unwrap();

        assert!(!r.healed(
            SmolStr::new("econ"),
            chunk_key("ledger", 7),
            "0".repeat(64).as_str(),
        ));
        assert_eq!(
            r.depth(),
            1,
            "a mismatched resolution cannot clear the seal"
        );
        assert!(r.healed(SmolStr::new("econ"), chunk_key("ledger", 7), &payload_hash,));
        assert!(r.is_empty());
    }

    #[test]
    fn conflicting_payload_for_one_immutable_key_is_rejected_atomically() {
        let mut r = RetainedSeals::new();
        let original = seal_op("econ", "ledger", 7, 0, 2);
        let conflict = seal_op("econ", "ledger", 7, 10, 2);
        r.capture(&[original], 9).unwrap();

        let RetainedSealError::Conflict(err) = r.capture(&[conflict], 10).unwrap_err() else {
            panic!("conflicting payload must report the conflicting exact identities")
        };
        assert_eq!(err.ns, "econ");
        assert_eq!(err.key, chunk_key("ledger", 7));
        assert_ne!(err.retained_hash, err.incoming_hash);
        assert_eq!(r.depth(), 1);
        r.ack(9);
        assert!(
            r.is_empty(),
            "the rejected capture did not re-stamp the original"
        );
    }

    #[test]
    fn same_serial_seals_from_one_batch_clear_together() {
        // Two logs sealing in the same batch share a serial; that serial names
        // exactly one batch, so its commit makes both durable.
        let mut r = RetainedSeals::new();
        r.capture(
            &[
                seal_op("scenes", "a", 0, 0, 1),
                seal_op("scenes", "b", 0, 0, 1),
            ],
            8,
        )
        .unwrap();
        assert_eq!(r.depth(), 2);
        r.ack(8);
        assert!(r.is_empty());
    }

    #[test]
    fn count_limit_rejects_the_whole_candidate_without_restamping() {
        let mut r = RetainedSeals::new();
        let first = seal_op("scenes", "a", 0, 0, 1);
        let second = seal_op("scenes", "b", 0, 0, 1);
        r.capture_with_limits(&[first.clone(), second], 7, 2, usize::MAX)
            .unwrap();

        let err = r
            .capture_with_limits(&[first, seal_op("scenes", "c", 0, 0, 1)], 9, 2, usize::MAX)
            .unwrap_err();
        assert!(matches!(
            err,
            RetainedSealError::Capacity {
                seals: 3,
                max_seals: 2,
                ..
            }
        ));
        assert_eq!(r.depth(), 2);
        r.ack(7);
        assert!(
            r.is_empty(),
            "rejected capture must not re-stamp even its existing candidate"
        );
        assert_eq!(r.custody_bytes(), 0);
    }

    #[test]
    fn byte_limit_rejects_the_whole_candidate_without_restamping() {
        let mut r = RetainedSeals::new();
        let first = seal_op("scenes", "a", 0, 0, 1);
        r.capture(std::slice::from_ref(&first), 11).unwrap();
        let before = r.custody_bytes();
        let extra = seal_op("scenes", "b", 0, 0, 1);
        let ModuleWrite::JsonSet { key, value } = &extra.op else {
            unreachable!()
        };
        let hash = crate::quarantine::seal_identity_hash(&extra.ns, key, value);
        let extra_bytes = pending_bytes_bounded(&extra.ns, key, &hash, value, usize::MAX)
            .unwrap()
            .expect("small test seal fits byte counter");

        let err = r
            .capture_with_limits(
                &[first, extra],
                12,
                usize::MAX,
                before.saturating_add(extra_bytes).saturating_sub(1),
            )
            .unwrap_err();
        assert!(matches!(err, RetainedSealError::Capacity { .. }));
        assert_eq!(r.depth(), 1);
        assert_eq!(r.custody_bytes(), before);
        r.ack(11);
        assert!(
            r.is_empty(),
            "byte-bound refusal must leave the prior acknowledgement stamp intact"
        );
    }
}
