//! Immutable, explicitly uncertified captures of actual personnel crossings.
//!
//! These records prove only the covered physical commit. Legacy movement can
//! still bypass capture, so there are no active intervals, eligibility answers,
//! payable durations, or promotion of old captures into certified history.
use std::collections::BTreeSet;

use mush_core::participant::{OwnerParticipant, ParticipantError};
use mush_core::physical_transition::{
    CorePhysicalTransitions, PhysicalTransitionDraft, PhysicalTransitionReceipt,
};
use mush_core::{
    ActionTxn, CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, ModError,
    ModuleCtx, ObjId, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
    canonical_json_bytes_with_limit,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const MAX_CROSSINGS: usize = 256;
pub(crate) const MAX_CAPTURE_ACTORS: usize = 256;
pub(crate) const MAX_CAPTURE_BYTES: usize = 128 * 1024;
const FAMILY: &str = "personnel_crossings";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CrossingDirection {
    Enter,
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Crossing {
    pub character: ObjId,
    pub carrier: ObjId,
    pub direction: CrossingDirection,
}

/// There is deliberately no certified variant or default. Future certification
/// needs a new coverage epoch and a new actual boarding under complete fences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    UncertifiedLegacyPaths,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossingBatch {
    format: u16,
    commit: CommitToken,
    operator: ObjId,
    root: ObjId,
    prepared_at: ClockStamp,
    plan_digest: DefinitionDigest,
    physical_digest: DefinitionDigest,
    before_ancestry_digest: DefinitionDigest,
    after_ancestry_digest: DefinitionDigest,
    coverage: Coverage,
    crossings: Vec<Crossing>,
}

fn refused() -> ParticipantError {
    ParticipantError::refused("personnel crossing capture is unavailable or inconsistent")
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

fn check_material(stamp: &ClockStamp, crossings: &[Crossing]) -> Result<(), ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
        || crossings.len() > MAX_CROSSINGS
    {
        return Err(refused());
    }
    let mut actors = BTreeSet::new();
    let mut previous = None;
    for row in crossings {
        let key = (row.character, row.carrier);
        if row.character == row.carrier || previous.is_some_and(|before| before >= key) {
            return Err(refused());
        }
        actors.insert(row.character);
        if actors.len() > MAX_CAPTURE_ACTORS {
            return Err(refused());
        }
        previous = Some(key);
    }
    Ok(())
}

/// Hash only material retained in the typed record. Read dependencies are
/// separately fenced by Core and cannot prevent a boot-time integrity check.
pub(crate) fn capture_digest(
    prepared_at: &ClockStamp,
    before_ancestry_digest: &DefinitionDigest,
    after_ancestry_digest: &DefinitionDigest,
    crossings: &[Crossing],
) -> Result<DefinitionDigest, ParticipantError> {
    check_material(prepared_at, crossings)?;
    let value = json!({
        "prepared_at": prepared_at,
        "before_ancestry_digest": before_ancestry_digest,
        "after_ancestry_digest": after_ancestry_digest,
        "crossings": crossings,
    });
    canonical_json_bytes_with_limit(&value, MAX_CAPTURE_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    DefinitionDigest::for_canonical_value("sim.personnel.capture.v1", &value)
        .map_err(|_| ParticipantError::Encoding)
}

pub(crate) fn combined_digest(
    plan_digest: &DefinitionDigest,
    capture_digest: &DefinitionDigest,
) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value(
        "sim.physical.personnel.v2",
        &json!({"plan_digest":plan_digest,"capture_digest":capture_digest}),
    )
    .map_err(|_| ParticipantError::Encoding)
}

impl CrossingBatch {
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1 {
            return Err(refused());
        }
        let capture = capture_digest(
            &self.prepared_at,
            &self.before_ancestry_digest,
            &self.after_ancestry_digest,
            &self.crossings,
        )?;
        if combined_digest(&self.plan_digest, &capture)? != self.physical_digest {
            return Err(refused());
        }
        encoded(self)?;
        Ok(())
    }

    fn matches_receipt(&self, receipt: &PhysicalTransitionReceipt) -> Result<(), ParticipantError> {
        self.validate()?;
        if receipt.action().version() != 2
            || self.commit != receipt.commit()
            || self.operator != receipt.actor()
            || self.root != receipt.root()
            || &self.physical_digest != receipt.structural_digest()
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn encoded(batch: &CrossingBatch) -> Result<Value, ParticipantError> {
    if batch.crossings.len() > MAX_CROSSINGS {
        return Err(ParticipantError::ImageLimit);
    }
    let value = serde_json::to_value(batch).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, MAX_CAPTURE_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}

fn decode(value: &Value) -> Result<CrossingBatch, String> {
    canonical_json_bytes_with_limit(value, MAX_CAPTURE_BYTES).map_err(|e| e.to_string())?;
    let batch: CrossingBatch = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    batch.validate().map_err(|e| e.to_string())?;
    if encoded(&batch).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical personnel crossing capture".into());
    }
    Ok(batch)
}

#[derive(Clone)]
pub(crate) struct PersonnelCrossings {
    records: StateFamilyHandle<CommitToken, CrossingBatch>,
}

impl PersonnelCrossings {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        let id = |suffix| CanonicalId::new(format!("sim.{FAMILY}.{suffix}.v1")).map_err(bad);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(bad)?,
            id("key")?,
            id("value")?,
            1,
            id("decode")?,
            id("validate")?,
            id("canonical")?,
        )
        .and_then(|d| d.with_limits(128, MAX_CAPTURE_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &CommitToken| Ok(key.to_string()),
            decode,
            |batch: &CrossingBatch| batch.validate().map_err(|e| e.to_string()),
            |batch: &CrossingBatch| encoded(batch).map_err(|e| e.to_string()),
        )?;
        Ok(Self { records })
    }

    pub(crate) fn append(
        &self,
        txn: &mut ActionTxn<'_>,
        receipt: &PhysicalTransitionReceipt,
        facts: &crate::personnel_capture::CaptureFacts,
        plan_digest: &DefinitionDigest,
    ) -> Result<(), ParticipantError> {
        let recomputed = capture_digest(
            &facts.prepared_at,
            &facts.before_ancestry_digest,
            &facts.after_ancestry_digest,
            &facts.crossings,
        )?;
        if recomputed != facts.capture_digest
            || txn
                .get(&self.records, &receipt.commit())
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        let batch = CrossingBatch {
            format: 1,
            commit: receipt.commit(),
            operator: receipt.actor(),
            root: receipt.root(),
            prepared_at: facts.prepared_at.clone(),
            plan_digest: *plan_digest,
            physical_digest: *receipt.structural_digest(),
            before_ancestry_digest: facts.before_ancestry_digest,
            after_ancestry_digest: facts.after_ancestry_digest,
            coverage: Coverage::UncertifiedLegacyPaths,
            crossings: facts.crossings.clone(),
        };
        batch.matches_receipt(receipt)?;
        txn.insert(&self.records, &receipt.commit(), batch)
            .map_err(|_| refused())?;
        Ok(())
    }

    /// Audit both directions: no orphan/mismatched capture and no v2 physical
    /// commit without its capture. Historical v1 receipts stay historical.
    pub(crate) fn audit<P: OwnerParticipant<Output = PhysicalTransitionDraft>>(
        &self,
        ctx: &ModuleCtx<'_>,
        physical: &CorePhysicalTransitions<P>,
    ) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(|_| refused())?;
            for entry in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS
                    || entry.encoded_key() != entry.value().commit.to_string()
                {
                    return Err(refused());
                }
                let receipt = physical
                    .read_receipt(ctx, &entry.value().commit)?
                    .ok_or_else(refused)?;
                entry.value().matches_receipt(&receipt)?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                break;
            }
        }
        cursor = None;
        count = 0;
        loop {
            let page = physical.scan_receipts(ctx, cursor.as_deref(), 128)?;
            for receipt in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS {
                    return Err(refused());
                }
                let capture = ctx
                    .action_state(&self.records, &receipt.commit())
                    .map_err(|_| refused())?;
                match (receipt.action().version(), capture) {
                    (1, None) => {}
                    (2, Some(capture)) => capture.matches_receipt(receipt)?,
                    _ => return Err(refused()),
                }
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
