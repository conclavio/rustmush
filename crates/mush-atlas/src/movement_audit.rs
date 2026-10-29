//! Original knowledge-position audit. Later legacy visits are real journal
//! mutations; a historical receipt is never compared to today's room geometry.

use std::collections::BTreeMap;

use mush_core::action_state::{MAX_ACTION_STATE_BOOT_BYTES, MAX_ACTION_STATE_BOOT_RECORDS};
use mush_core::locomotion::composed::{ComposedOwnerReader, ComposedOwnerReceipt};
use mush_core::{
    ClockStamp, CommitToken, DefinitionDigest, DefinitionRegistryBuilder, LedgerHistory, ModError,
    ModuleBootData, ModuleCtx, ObjId, OwnerId,
};

use crate::movement::{Discovery, MovementCapture, SlotPresence, WindowEffect};
use crate::movement_history::{self as history, Mutation, Source, known_digest, normalized};
use crate::movement_participant::{self, EventClaim, Proof};
use crate::travel::TravelState;

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

struct Expected {
    actor: ObjId,
    before: Option<DefinitionDigest>,
    after: Option<DefinitionDigest>,
    event: Option<EventClaim>,
    commit: CommitToken,
    stamp: ClockStamp,
    at_utc: i64,
    capture: DefinitionDigest,
}
struct Audit {
    pending: BTreeMap<u64, Vec<Expected>>,
    known: BTreeMap<ObjId, DefinitionDigest>,
    remaining: usize,
    receipts: usize,
}
impl Audit {
    fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            known: BTreeMap::new(),
            remaining: MAX_ACTION_STATE_BOOT_BYTES,
            receipts: 0,
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<(), ModError> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| bad("Atlas movement audit exceeds its memory/scan bound"))?;
        Ok(())
    }
    fn add(&mut self, receipt: &ComposedOwnerReceipt) -> Result<(), ModError> {
        self.receipts = self
            .receipts
            .checked_add(1)
            .ok_or_else(|| bad("Atlas receipt count overflow"))?;
        if self.receipts > MAX_ACTION_STATE_BOOT_RECORDS {
            return Err(bad("Atlas receipt count limit exceeded"));
        }
        self.charge(1024)?;
        let expected = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(bad)?,
                movement_participant::participant_descriptor()?,
            )
            .map_err(bad)?;
        if receipt.effect().participant() != &expected {
            return Err(bad("unknown Atlas movement proof profile"));
        }
        let value = receipt.effect().proof();
        mush_core::canonical_json_bytes_with_limit(
            value,
            mush_core::locomotion::composed::MAX_MOVEMENT_PROOF_BYTES,
        )
        .map_err(bad)?;
        let proof: Proof = serde_json::from_value(value.clone()).map_err(bad)?;
        if proof.format != 1 || serde_json::to_value(&proof).map_err(bad)? != *value {
            return Err(bad("invalid Atlas movement proof"));
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(
            &proof.capture,
            crate::movement::MAX_MOVEMENT_CAPTURE_BYTES,
        )
        .map_err(bad)?;
        let capture = MovementCapture::decode(&bytes).map_err(bad)?;
        let digest = movement_participant::capture_digest(&capture).map_err(bad)?;
        if capture.actor() != receipt.actor()
            || capture.from() != receipt.source()
            || capture.to() != receipt.destination()
            || capture.at() != receipt.at_utc()
            || capture.atlas_slot() != SlotPresence::Present
            || capture.window_effect() != WindowEffect::NoSelectedEffect
            || &digest != receipt.effect().owner_capture_digest()
        {
            return Err(bad(
                "Atlas movement proof differs from its original movement",
            ));
        }
        let before = if let Discovery::Mapped { known_before, .. } = capture.discovery() {
            if let Some(before) = known_before {
                history::canonical_known(before).map_err(bad)?;
            }
            Some(known_digest(known_before.as_deref()).map_err(bad)?)
        } else {
            None
        };
        let after = capture
            .planned_known()
            .map(|doc| known_digest(Some(&doc.places)).map_err(bad))
            .transpose()?;
        if proof.event.is_some() != after.is_some()
            || proof
                .event
                .as_ref()
                .is_some_and(|event| event.sequence != proof.history_next)
        {
            return Err(bad(
                "Atlas movement proof has inconsistent knowledge effect claims",
            ));
        }
        self.pending
            .entry(proof.history_next)
            .or_default()
            .push(Expected {
                actor: receipt.actor(),
                before,
                after,
                event: proof.event,
                commit: receipt.commit(),
                stamp: receipt.prepared_at().clone(),
                at_utc: receipt.at_utc(),
                capture: digest,
            });
        Ok(())
    }
    fn observe(&mut self, actor: ObjId, digest: DefinitionDigest) -> Result<(), ModError> {
        if let Some(previous) = self.known.get(&actor) {
            if previous != &digest {
                return Err(bad(
                    "Atlas knowledge observation contradicts original history",
                ));
            }
        } else {
            self.charge(256)?;
            self.known.insert(actor, digest);
        }
        Ok(())
    }
    fn boundary(&mut self, sequence: u64, mutation: Option<&Mutation>) -> Result<(), ModError> {
        let mut claimed = false;
        if let Some(expected) = self.pending.remove(&sequence) {
            for proof in expected {
                if let Some(before) = proof.before {
                    self.observe(proof.actor, before)?;
                }
                if let Some(event) = proof.event {
                    if claimed {
                        return Err(bad("Atlas receipts claim the same knowledge position"));
                    }
                    claimed = true;
                    let mutation =
                        mutation.ok_or_else(|| bad("Atlas prepared knowledge event is missing"))?;
                    let Source::Prepared {
                        commit,
                        prepared_at,
                        capture_digest,
                    } = &mutation.source
                    else {
                        return Err(bad(
                            "Atlas prepared event was replaced by a legacy mutation",
                        ));
                    };
                    if mutation.actor != proof.actor
                        || mutation.at_utc != proof.at_utc
                        || *commit != proof.commit
                        || prepared_at != &proof.stamp
                        || capture_digest != &proof.capture
                        || mutation.digest().map_err(bad)? != event.digest
                        || Some(known_digest(Some(&mutation.known.places)).map_err(bad)?)
                            != proof.after
                    {
                        return Err(bad(
                            "Atlas prepared knowledge event differs from its receipt",
                        ));
                    }
                }
            }
        }
        if let Some(mutation) = mutation {
            if matches!(mutation.source, Source::Prepared { .. }) && !claimed {
                return Err(bad(
                    "Atlas prepared knowledge event has no original owner receipt",
                ));
            }
            if !self.known.contains_key(&mutation.actor) {
                self.charge(256)?;
            }
            let places = normalized(&mutation.known);
            self.known
                .insert(mutation.actor, known_digest(Some(&places)).map_err(bad)?);
        }
        Ok(())
    }
    fn read_history(
        &mut self,
        data: &ModuleBootData,
        history: &LedgerHistory,
    ) -> Result<(), ModError> {
        if !history.is_clean() || history.logs().any(|id| id.as_str() != history::LOG) {
            return Err(bad(
                "Atlas knowledge history is invalid or has an unknown log",
            ));
        }
        let mut next = 0_u64;
        for (_, position) in history.chunk_docs(history::LOG) {
            let (_, text) = data
                .ledger
                .get(position)
                .ok_or_else(|| bad("Atlas knowledge history index is inconsistent"))?;
            self.charge(text.len())?;
            let chunk = mush_core::ledger::parse_chunk(text).map_err(bad)?;
            for entry in chunk.entries {
                if entry.seq != next || next >= MAX_ACTION_STATE_BOOT_RECORDS as u64 {
                    return Err(bad("Atlas knowledge history is incomplete or excessive"));
                }
                let mutation = Mutation::decode(&entry.value).map_err(bad)?;
                self.boundary(next, Some(&mutation))?;
                next = next
                    .checked_add(1)
                    .ok_or_else(|| bad("Atlas knowledge sequence overflow"))?;
            }
        }
        self.boundary(next, None)?;
        if !self.pending.is_empty() {
            return Err(bad("Atlas receipt observes unavailable knowledge history"));
        }
        Ok(())
    }
    fn finish(self, travel: &TravelState) -> Result<(), ModError> {
        for (actor, digest) in self.known {
            if known_digest(travel.known.get(&actor).map(Vec::as_slice)).map_err(bad)? != digest {
                return Err(bad(
                    "Atlas current knowledge projection differs from its original history",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn audit_boot(
    ctx: &ModuleCtx<'_>,
    data: &ModuleBootData,
    history: &LedgerHistory,
    travel: &TravelState,
    reader: &ComposedOwnerReader,
) -> Result<(), ModError> {
    let mut audit = Audit::new();
    let mut after = None;
    loop {
        let page = reader
            .scan_receipts(ctx, after.as_deref(), 128)
            .map_err(bad)?;
        for receipt in page.entries() {
            audit.add(receipt)?;
        }
        after = page.next_after().map(ToString::to_string);
        if after.is_none() {
            break;
        }
    }
    audit.read_history(data, history)?;
    audit.finish(travel)
}

#[cfg(test)]
#[path = "movement_audit_tests.rs"]
mod tests;
