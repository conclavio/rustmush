//! Passive window receipt/journal wire data. No decoder here can mint a
//! preview, native Plan, owner patch, installation or personnel eligibility.

use super::*;
use mush_core::locomotion::composed::{self, ComposedLocomotionReceipt, WindowProjection};
use mush_core::{CommitToken, DefinitionRef, DefinitionRegistryBuilder, OwnerId};
use serde::{Deserialize, Deserializer};

const MAX_RECORD_BYTES: usize = 160 * 1024;
const JOURNAL_BYTES: usize = 16 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    UncertifiedLegacyPaths,
}
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalClaim {
    sequence: u64,
    digest: DefinitionDigest,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    coverage: Coverage,
    commit: CommitToken,
    action: DefinitionRef,
    participant: DefinitionRef,
    actor: ObjId,
    #[serde(deserialize_with = "required")]
    source: Option<ObjId>,
    #[serde(deserialize_with = "required")]
    exit: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    plan_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    projection: WindowProjection,
    evidence: Value,
    final_placement_digest: DefinitionDigest,
    journal: JournalClaim,
}

/// Strict passive record data. A supplied journal position is a claim until
/// joined to the actual same-commit native journal and owner receipt.
#[derive(Clone)]
pub struct WindowMovementRecord {
    wire: Box<Wire>,
}
impl WindowMovementRecord {
    pub fn decode(value: &Value) -> Result<Self, ParticipantError> {
        preflight(value)?;
        let wire: Wire =
            serde_json::from_value(value.clone()).map_err(|_| ParticipantError::Encoding)?;
        if encode(&wire)? != *value {
            return Err(refused());
        }
        wire.validate()?;
        Ok(Self {
            wire: Box::new(wire),
        })
    }
    /// Copy only the genuine receipt's original header. This prepares data,
    /// never an append or proof that native publication already occurred.
    pub fn for_receipt(
        receipt: &ComposedLocomotionReceipt,
        capture: &OrderedWindowMovementCapture,
        journal_sequence: u64,
    ) -> Result<Self, ParticipantError> {
        if *receipt.capture_digest() != capture.digest() {
            return Err(refused());
        }
        let projection = receipt.window_projection().ok_or_else(refused)?.clone();
        let evidence: Value =
            serde_json::from_slice(&capture.encode()).map_err(|_| ParticipantError::Encoding)?;
        let decoded = validate_evidence(&evidence, capture.digest())?;
        let mut wire = Wire {
            format: 1,
            coverage: Coverage::UncertifiedLegacyPaths,
            commit: receipt.commit(),
            action: receipt.action().clone(),
            participant: receipt.participant().clone(),
            actor: receipt.actor(),
            source: receipt.source(),
            exit: receipt.exit(),
            admitted_room: receipt.destination(),
            projected_room: projection.projected_room(),
            prepared_at: receipt.prepared_at().clone(),
            at_utc: receipt.at_utc(),
            plan_digest: *receipt.plan_digest(),
            capture_digest: capture.digest(),
            projection,
            evidence,
            final_placement_digest: final_digest(&decoded)?,
            journal: JournalClaim {
                sequence: journal_sequence,
                digest: DefinitionDigest::from_bytes([0; 32]),
            },
        };
        wire.journal.digest = journal_digest(&wire.journal_value())?;
        let record = Self {
            wire: Box::new(wire),
        };
        record.matches_receipt(receipt)?;
        Ok(record)
    }
    pub fn encode(&self) -> Result<Value, ParticipantError> {
        encode(&self.wire)
    }
    pub fn commit(&self) -> CommitToken {
        self.wire.commit
    }
    pub fn actor(&self) -> ObjId {
        self.wire.actor
    }
    pub fn journal_sequence(&self) -> u64 {
        self.wire.journal.sequence
    }
    pub fn journal_digest(&self) -> DefinitionDigest {
        self.wire.journal.digest
    }
    /// The exact expected entry body, still requiring an actual Ledger append.
    pub fn journal_value(&self) -> Value {
        self.wire.journal_value()
    }
    pub fn matches_journal_entry(
        &self,
        sequence: u64,
        value: &Value,
    ) -> Result<(), ParticipantError> {
        preflight(value)?;
        if sequence != self.wire.journal.sequence
            || journal_digest(value)? != self.wire.journal.digest
            || *value != self.wire.journal_value()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub fn evidence(&self) -> Result<WindowMovementEvidence, ParticipantError> {
        self.wire.validate()
    }
    pub fn matches_receipt(
        &self,
        receipt: &ComposedLocomotionReceipt,
    ) -> Result<(), ParticipantError> {
        self.wire.validate()?;
        let w = &self.wire;
        if w.commit != receipt.commit()
            || &w.action != receipt.action()
            || &w.participant != receipt.participant()
            || w.actor != receipt.actor()
            || w.source != receipt.source()
            || w.exit != receipt.exit()
            || w.admitted_room != receipt.destination()
            || &w.prepared_at != receipt.prepared_at()
            || w.at_utc != receipt.at_utc()
            || &w.plan_digest != receipt.plan_digest()
            || &w.capture_digest != receipt.capture_digest()
            || receipt.window_projection() != Some(&w.projection)
        {
            return Err(refused());
        }
        Ok(())
    }
}
fn final_digest(evidence: &WindowMovementEvidence) -> Result<DefinitionDigest, ParticipantError> {
    let value =
        serde_json::to_value(evidence.final_placement()).map_err(|_| ParticipantError::Encoding)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.window_movement.final.v1",
        &value,
        MAX_WINDOW_MOVEMENT_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
fn journal_digest(value: &Value) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.window_movement.journal.v1",
        value,
        JOURNAL_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
fn nonzero(digest: DefinitionDigest) -> bool {
    digest.as_bytes() != &[0; 32]
}
impl Wire {
    fn journal_value(&self) -> Value {
        json!({"format":1,"op":"window_locomotion_v2","coverage":self.coverage,
            "commit":self.commit,"action":self.action,"participant":self.participant,
            "actor":self.actor,"source":self.source,"exit":self.exit,
            "admitted_room":self.admitted_room,"projected_room":self.projected_room,
            "prepared_at":self.prepared_at,"at_utc":self.at_utc,
            "plan_digest":self.plan_digest,"capture_digest":self.capture_digest,
            "projection":self.projection,"final_placement_digest":self.final_placement_digest})
    }
    fn validate(&self) -> Result<WindowMovementEvidence, ParticipantError> {
        let expected = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(|_| ParticipantError::Authority)?,
                composed::window_action_descriptor().map_err(|_| ParticipantError::Encoding)?,
            )
            .map_err(|_| ParticipantError::Encoding)?;
        let evidence = validate_evidence(&self.evidence, self.capture_digest)?;
        let projection =
            serde_json::to_value(&self.projection).map_err(|_| ParticipantError::Encoding)?;
        if self.format != 1
            || self.action != expected
            || self.participant.key().owner.as_str() != crate::NS
            || self.participant.key().kind.as_str() != "owner_participant"
            || self.participant.key().id.as_str() != composed::SIM_PARTICIPANT
            || self.participant.version() != 2
            || self.journal.sequence == u64::MAX
            || self.source == Some(self.actor)
            || (self.exit.is_none() && self.source == Some(self.admitted_room))
            || self.exit.is_some_and(|exit| {
                self.source.is_none()
                    || exit == self.actor
                    || Some(exit) == self.source
                    || exit == self.admitted_room
            })
            || !nonzero(self.plan_digest)
            || !nonzero(self.projection.atlas_capture_digest())
            || projection["format"] != 1
            || evidence.actor() != self.actor
            || evidence.source() != self.source
            || evidence.admitted_room() != self.admitted_room
            || evidence.projected_room() != self.projected_room
            || evidence.prepared_at() != &self.prepared_at
            || evidence.at_utc() != self.at_utc
            || self.projection.projected_room() != self.projected_room
            || self.projection.sim_window_capture_digest() != self.capture_digest
            || serde_json::to_value(evidence.target()).map_err(|_| ParticipantError::Encoding)?
                != projection["target"]
            || final_digest(&evidence)? != self.final_placement_digest
            || journal_digest(&self.journal_value())? != self.journal.digest
        {
            return Err(refused());
        }
        encode(self)?;
        Ok(evidence)
    }
}
fn encode(wire: &Wire) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(wire).map_err(|_| ParticipantError::Encoding)?;
    mush_core::canonical_json_bytes_with_limit(&value, MAX_RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn preflight(value: &Value) -> Result<(), ParticipantError> {
    let mut nodes = 80_000_usize;
    let mut text = MAX_RECORD_BYTES;
    let mut pending = vec![(value, 1_usize)];
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.checked_sub(1).ok_or(ParticipantError::ImageLimit)?;
        if depth > 100 {
            return Err(ParticipantError::ImageLimit);
        }
        match value {
            Value::Array(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                pending.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                for (key, value) in values {
                    text = text
                        .checked_sub(key.len())
                        .ok_or(ParticipantError::ImageLimit)?;
                    pending.push((value, depth + 1));
                }
            }
            Value::String(value) => {
                text = text
                    .checked_sub(value.len())
                    .ok_or(ParticipantError::ImageLimit)?;
            }
            _ => {}
        }
    }
    mush_core::canonical_json_bytes_with_limit(value, MAX_RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(())
}
