//! Bounded ordered appends, including repeated positions in one selected log.
//! This is ledger machinery, not an owner, movement or installation authority.

use std::collections::BTreeMap;

use smol_str::SmolStr;

use super::{Ledger, LedgerAppendError, LedgerChunk, LogTail, MAX_SAFE_CHUNK_BYTES, bounded_tail};
use crate::module::ModuleWrite;
use crate::{OwnerId, ReplayWrite};

/// Two Core room legs can each join and leave a scene transcript.
pub const MAX_LEDGER_SEQUENCE_APPENDS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerSequenceError {
    #[error("ledger sequence exceeds its four-append bound")]
    TooMany,
    #[error(transparent)]
    Append(#[from] LedgerAppendError),
    #[error("ledger sequence prestate exceeds its byte or nesting bound")]
    InputLimit,
    #[error("prepared ledger sequence is stale")]
    StalePrestate,
}

/// Opaque final tails and original ordered positions. No decoding or caller
/// fields can manufacture a sequence. Preparation changes no resident log.
#[must_use = "a prepared sequence has no effect until bound and applied"]
pub struct PreparedLedgerSequence {
    policy: usize,
    resident_limit: usize,
    expected: BTreeMap<SmolStr, Option<LogTail>>,
    tails: BTreeMap<SmolStr, LogTail>,
    positions: Vec<(SmolStr, u64)>,
    chunks: BTreeMap<(SmolStr, u64), LedgerChunk>,
}

impl PreparedLedgerSequence {
    /// Original event order, including distinct consecutive positions in a
    /// repeated log. These are data until joined to an actual owner receipt.
    pub fn positions(&self) -> impl Iterator<Item = (&str, u64)> {
        self.positions.iter().map(|(id, seq)| (id.as_str(), *seq))
    }

    /// Emit one final write per touched chunk. A later append in the same
    /// chunk replaces its earlier snapshot; sealed rollover chunks remain.
    pub fn authoritative_replay_writes(
        &self,
        namespace: OwnerId,
    ) -> Result<Vec<ReplayWrite>, crate::authoritative::AuthoritativeLedgerWriteError> {
        self.chunks
            .iter()
            .map(|((id, _), chunk)| {
                Ok(ReplayWrite::LedgerChunkSet {
                    write: crate::authoritative::AuthoritativeLedgerWrite::new(
                        namespace.clone(),
                        id,
                        chunk.clone(),
                    )?,
                })
            })
            .collect()
    }
}

/// Every selected original tail is checked before this publication borrow
/// exists. Dropping it is inert; application installs only prepared tails.
#[must_use = "binding does not append; apply consumes the prepared sequence"]
pub struct BoundLedgerSequence<'a> {
    ledger: &'a mut Ledger,
    tails: BTreeMap<SmolStr, LogTail>,
    writes: Vec<ModuleWrite>,
}

impl BoundLedgerSequence<'_> {
    pub fn apply(self) -> Vec<ModuleWrite> {
        for (id, tail) in self.tails {
            self.ledger.logs.insert(id, tail);
        }
        self.writes
    }
}

impl Ledger {
    /// Prepare at most four ordered entries. Only selected resident tails are
    /// bounded and copied; unrelated logs never enter the planning ledger.
    /// Each selected tail retains the requested bound through final binding.
    pub fn prepare_sequence_bounded(
        &self,
        entries: Vec<(String, serde_json::Value)>,
        max_resident_bytes: usize,
    ) -> Result<PreparedLedgerSequence, LedgerSequenceError> {
        if entries.len() > MAX_LEDGER_SEQUENCE_APPENDS {
            return Err(LedgerSequenceError::TooMany);
        }
        let mut selected = Ledger::with_chunk_target(self.chunk_target_bytes);
        let mut expected = BTreeMap::new();
        let mut positions = Vec::with_capacity(entries.len());
        let mut chunks = BTreeMap::new();
        for (id, value) in entries {
            // This validates the id, original tail and opaque value before
            // cloning any selected state. Repeated ids use the prior image.
            let prepared = if expected.contains_key(id.as_str()) {
                selected.prepare_append_bounded(&id, value, max_resident_bytes)?
            } else {
                let prepared = self.prepare_append_bounded(&id, value, max_resident_bytes)?;
                expected.insert(prepared.log_id.clone(), prepared.expected_tail.clone());
                if let Some(tail) = &prepared.expected_tail {
                    selected.logs.insert(prepared.log_id.clone(), tail.clone());
                }
                prepared
            };
            positions.push((prepared.log_id.clone(), prepared.seq));
            chunks.insert(
                (prepared.log_id.clone(), prepared.chunk.chunk),
                prepared.chunk.clone(),
            );
            // Only the bounded temporary ledger changes during preparation.
            selected
                .apply_prepared(prepared)
                .map_err(|_| LedgerSequenceError::StalePrestate)?;
        }
        Ok(PreparedLedgerSequence {
            policy: self.chunk_target_bytes,
            resident_limit: max_resident_bytes.min(MAX_SAFE_CHUNK_BYTES),
            expected,
            tails: selected.logs,
            positions,
            chunks,
        })
    }

    /// Callback-free comparison of every selected original tail. Bound the
    /// live and retained payloads before equality can descend into JSON.
    pub fn check_prepared_sequence(
        &self,
        plan: &PreparedLedgerSequence,
    ) -> Result<(), LedgerSequenceError> {
        for (id, expected) in &plan.expected {
            let live = self.logs.get(id.as_str());
            if !bounded_tail(id, self.chunk_target_bytes, live, plan.resident_limit)
                || !bounded_tail(id, plan.policy, expected.as_ref(), plan.resident_limit)
            {
                return Err(LedgerSequenceError::InputLimit);
            }
            if self.chunk_target_bytes != plan.policy || live != expected.as_ref() {
                return Err(LedgerSequenceError::StalePrestate);
            }
        }
        Ok(())
    }

    /// Bind every original tail before publishing any image. All final chunk
    /// serialization completes before this function returns the borrow.
    pub fn bind_prepared_sequence(
        &mut self,
        plan: PreparedLedgerSequence,
    ) -> Result<BoundLedgerSequence<'_>, LedgerSequenceError> {
        self.check_prepared_sequence(&plan)?;
        let writes = plan
            .chunks
            .iter()
            .map(|((id, chunk), value)| {
                Ok(ModuleWrite::JsonSet {
                    key: super::chunk_key(id, *chunk),
                    value: serde_json::to_value(value)
                        .map_err(|_| LedgerAppendError::ChunkEncoding)?,
                })
            })
            .collect::<Result<Vec<_>, LedgerSequenceError>>()?;
        Ok(BoundLedgerSequence {
            ledger: self,
            tails: plan.tails,
            writes,
        })
    }
}

#[cfg(test)]
#[path = "ledger_sequence_tests.rs"]
mod tests;
