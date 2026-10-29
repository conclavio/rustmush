//! Versioned receipt data and exact read capability. Historical manifests are
//! checked against retained definitions, not current registration coverage.

use super::super::completion::MovementManifestSnapshot;
use super::super::transition::LocomotionObjectDigest;
use super::owner;
use super::{
    FAMILY, MAX_COMPOSED_OWNERS, MAX_MOVEMENT_PROOF_BYTES, RECORD_BYTES, SIM_PARTICIPANT, bad,
    bounded, exact_action, refused,
};
use crate::participant::ParticipantError;
use crate::{
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ModError,
    ModuleCtx, ObjId, OwnerId, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementEffectReceipt {
    pub(super) group: u16,
    pub(super) binding: DefinitionRef,
    pub(super) participant: DefinitionRef,
    pub(super) intent_digest: DefinitionDigest,
    pub(super) owner_capture_digest: DefinitionDigest,
    pub(super) capture_digest: DefinitionDigest,
    pub(super) proof: Value,
    pub(super) proof_digest: DefinitionDigest,
    pub(super) writes_digest: DefinitionDigest,
}
impl MovementEffectReceipt {
    pub const fn group(&self) -> u16 {
        self.group
    }
    pub const fn binding(&self) -> &DefinitionRef {
        &self.binding
    }
    pub const fn participant(&self) -> &DefinitionRef {
        &self.participant
    }
    pub const fn owner_capture_digest(&self) -> &DefinitionDigest {
        &self.owner_capture_digest
    }
    pub const fn capture_digest(&self) -> &DefinitionDigest {
        &self.capture_digest
    }
    pub const fn proof(&self) -> &Value {
        &self.proof
    }
    pub const fn writes_digest(&self) -> &DefinitionDigest {
        &self.writes_digest
    }
}
pub(super) fn proof_digest(
    binding: &DefinitionRef,
    capture_digest: &DefinitionDigest,
    proof: &Value,
) -> Result<DefinitionDigest, ParticipantError> {
    crate::canonical_json_bytes_with_limit(proof, MAX_MOVEMENT_PROOF_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "core.composed_locomotion.owner_proof.v1",
        &json!({"binding":binding,"capture_digest":capture_digest,"proof":proof}),
        MAX_MOVEMENT_PROOF_BYTES + 4096,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub(super) format: u16,
    pub(super) commit: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) participant: DefinitionRef,
    pub(super) actor: ObjId,
    #[serde(deserialize_with = "required_option")]
    pub(super) source: Option<ObjId>,
    #[serde(deserialize_with = "required_option")]
    pub(super) exit: Option<ObjId>,
    pub(super) destination: ObjId,
    pub(super) prepared_at: ClockStamp,
    pub(super) at_utc: i64,
    pub(super) plan_digest: DefinitionDigest,
    pub(super) capture_digest: DefinitionDigest,
    pub(super) manifest: Value,
    pub(super) manifest_digest: DefinitionDigest,
    pub(super) effects: Vec<MovementEffectReceipt>,
    pub(super) objects: Vec<LocomotionObjectDigest>,
}
impl Record {
    pub(super) fn validate(&self) -> Result<MovementManifestSnapshot, ParticipantError> {
        exact_action(&self.action)?;
        if self.format != 1
            || self.participant.key().owner.as_str() != "sim"
            || self.participant.key().kind.as_str() != "owner_participant"
            || self.participant.key().id.as_str() != SIM_PARTICIPANT
            || self.participant.version() != 1
            || self.actor == self.destination
            || self.source == Some(self.actor)
            || self.exit.is_some_and(|exit| {
                self.source.is_none()
                    || exit == self.actor
                    || Some(exit) == self.source
                    || exit == self.destination
            })
            || !matches!(self.prepared_at.clock, ClockRef::WorldIc { generation } if generation > 0)
            || self.effects.len() >= MAX_COMPOSED_OWNERS
            || self.effects.is_empty()
            || self.objects.len() > 3
            || self
                .objects
                .windows(2)
                .any(|pair| pair[0].object >= pair[1].object)
            || self.objects.iter().any(|row| row.before == row.after)
        {
            return Err(refused());
        }
        let expected: BTreeSet<_> = if self.source == Some(self.destination) {
            if self.exit.is_none() {
                return Err(refused());
            }
            BTreeSet::new()
        } else {
            [Some(self.actor), self.source, Some(self.destination)]
                .into_iter()
                .flatten()
                .collect()
        };
        if self
            .objects
            .iter()
            .map(|row| row.object)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(refused());
        }
        let manifest =
            MovementManifestSnapshot::from_value(&self.manifest).map_err(|_| refused())?;
        if manifest.action() != &self.action
            || manifest.digest().map_err(|_| refused())? != self.manifest_digest
            || manifest.groups().len() != self.effects.len()
        {
            return Err(refused());
        }
        for (index, (effect, group)) in self.effects.iter().zip(manifest.groups()).enumerate() {
            let intent = owner::intent_digest(
                &self.action,
                self.commit,
                self.actor,
                self.source,
                self.exit,
                self.destination,
                &self.prepared_at,
                self.at_utc,
                group.binding(),
            )?;
            if usize::from(effect.group) != index
                || &effect.binding != group.binding()
                || &effect.participant != group.participant()
                || effect.participant == self.participant
                || effect.intent_digest != intent
                || effect.capture_digest
                    != owner::capture_digest(&intent, &effect.owner_capture_digest)?
                || effect.proof_digest
                    != proof_digest(&effect.binding, &effect.capture_digest, &effect.proof)?
            {
                return Err(refused());
            }
        }
        bounded(self, RECORD_BYTES)?;
        Ok(manifest)
    }
}

#[derive(Clone, Debug)]
pub struct ComposedLocomotionReceipt {
    pub(super) row: Record,
    pub(super) manifest: MovementManifestSnapshot,
}
impl ComposedLocomotionReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.row.commit
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub const fn participant(&self) -> &DefinitionRef {
        &self.row.participant
    }
    pub const fn actor(&self) -> ObjId {
        self.row.actor
    }
    pub const fn source(&self) -> Option<ObjId> {
        self.row.source
    }
    pub const fn exit(&self) -> Option<ObjId> {
        self.row.exit
    }
    pub const fn destination(&self) -> ObjId {
        self.row.destination
    }
    pub const fn prepared_at(&self) -> &ClockStamp {
        &self.row.prepared_at
    }
    pub const fn at_utc(&self) -> i64 {
        self.row.at_utc
    }
    pub const fn plan_digest(&self) -> &DefinitionDigest {
        &self.row.plan_digest
    }
    pub const fn capture_digest(&self) -> &DefinitionDigest {
        &self.row.capture_digest
    }
    pub const fn manifest(&self) -> &MovementManifestSnapshot {
        &self.manifest
    }
    pub const fn manifest_digest(&self) -> DefinitionDigest {
        self.row.manifest_digest
    }
    pub fn effects(&self) -> &[MovementEffectReceipt] {
        &self.row.effects
    }
}
pub struct ComposedLocomotionReceiptPage {
    entries: Vec<ComposedLocomotionReceipt>,
    next_after: Option<String>,
}
impl ComposedLocomotionReceiptPage {
    pub fn entries(&self) -> &[ComposedLocomotionReceipt] {
        &self.entries
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}

#[derive(Clone)]
pub struct ComposedLocomotionReader {
    pub(super) action: DefinitionRef,
    pub(super) participant: DefinitionRef,
    pub(super) records: StateFamilyHandle<CommitToken, Record>,
}
impl ComposedLocomotionReader {
    fn validate_record(
        &self,
        ctx: &ModuleCtx<'_>,
        key: &str,
        row: &Record,
    ) -> Result<ComposedLocomotionReceipt, ParticipantError> {
        let manifest = row.validate()?;
        let definitions = ctx.reg.definitions().ok_or_else(refused)?;
        if key != row.commit.to_string()
            || row.action != self.action
            || row.participant != self.participant
        {
            return Err(refused());
        }
        definitions
            .inspect_exact(&row.action)
            .map_err(|_| refused())?;
        definitions
            .inspect_exact(&row.participant)
            .map_err(|_| refused())?;
        MovementManifestSnapshot::decode(&manifest.encode().map_err(|_| refused())?, definitions)
            .map_err(|_| refused())?;
        Ok(ComposedLocomotionReceipt {
            row: row.clone(),
            manifest,
        })
    }
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<ComposedLocomotionReceipt>, ParticipantError> {
        if ctx.typed_state.module_owner().map(OwnerId::as_str) != Some("sim") {
            return Err(refused());
        }
        self.read_internal(ctx, commit)
    }
    fn read_internal(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<ComposedLocomotionReceipt>, ParticipantError> {
        let Some(row) = ctx
            .action_state(&self.records, commit)
            .map_err(|_| refused())?
        else {
            return Ok(None);
        };
        self.validate_record(ctx, &commit.to_string(), row)
            .map(Some)
    }
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ComposedLocomotionReceiptPage, ParticipantError> {
        if ctx.typed_state.module_owner().map(OwnerId::as_str) != Some("sim") {
            return Err(refused());
        }
        self.scan_internal(ctx, after, limit)
    }
    fn scan_internal(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ComposedLocomotionReceiptPage, ParticipantError> {
        if limit == 0 || limit > 128 {
            return Err(refused());
        }
        let page = ctx
            .scan_action_state(&self.records, after, limit)
            .map_err(|_| refused())?;
        let mut entries = Vec::with_capacity(page.entries().len());
        for entry in page.entries() {
            entries.push(self.validate_record(ctx, entry.encoded_key(), entry.value())?);
        }
        Ok(ComposedLocomotionReceiptPage {
            entries,
            next_after: page.next_after().map(ToString::to_string),
        })
    }
    pub fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0_usize;
        loop {
            let page = self.scan_receipts(ctx, cursor.as_deref(), 128)?;
            count = count
                .checked_add(page.entries().len())
                .ok_or_else(refused)?;
            if count > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS {
                return Err(refused());
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}
pub(super) fn register_records(
    reg: &mut Registrar<'_>,
    readers: &BTreeSet<OwnerId>,
) -> Result<StateFamilyHandle<CommitToken, Record>, ModError> {
    let identifier = |suffix| CanonicalId::new(format!("sim.{FAMILY}.{suffix}.v1")).map_err(bad);
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(FAMILY).map_err(bad)?,
        identifier("key")?,
        identifier("value")?,
        1,
        identifier("decode")?,
        identifier("validate")?,
        identifier("canonical")?,
    )
    .and_then(|d| d.with_limits(128, RECORD_BYTES))
    .map_err(bad)?;
    let records = reg.state_family(
        descriptor,
        |key: &CommitToken| Ok(key.to_string()),
        decode,
        |row: &Record| row.validate().map(|_| ()).map_err(|e| e.to_string()),
        |row: &Record| bounded(row, RECORD_BYTES).map_err(|e| e.to_string()),
    )?;
    for owner in readers {
        if owner.as_str() != "sim" {
            reg.grant_state_family_read(&records, owner.as_str())?;
        }
    }
    Ok(records)
}
fn decode(value: &Value) -> Result<Record, String> {
    crate::canonical_json_bytes_with_limit(value, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: Record = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    row.validate().map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical composed locomotion receipt".into());
    }
    Ok(row)
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// An observer receives its own effect and the immutable movement header.
/// Other owners' proof payloads are never exposed by this read capability.
pub struct ComposedOwnerReceipt {
    commit: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    manifest_digest: DefinitionDigest,
    effect: MovementEffectReceipt,
}
impl ComposedOwnerReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn source(&self) -> Option<ObjId> {
        self.source
    }
    pub const fn exit(&self) -> Option<ObjId> {
        self.exit
    }
    pub const fn destination(&self) -> ObjId {
        self.destination
    }
    pub const fn prepared_at(&self) -> &ClockStamp {
        &self.prepared_at
    }
    pub const fn at_utc(&self) -> i64 {
        self.at_utc
    }
    pub const fn manifest_digest(&self) -> DefinitionDigest {
        self.manifest_digest
    }
    pub const fn effect(&self) -> &MovementEffectReceipt {
        &self.effect
    }
}
pub struct ComposedOwnerReceiptPage {
    entries: Vec<ComposedOwnerReceipt>,
    next_after: Option<String>,
}
impl ComposedOwnerReceiptPage {
    pub fn entries(&self) -> &[ComposedOwnerReceipt] {
        &self.entries
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}
#[derive(Clone)]
pub struct ComposedOwnerReader {
    pub(super) reader: ComposedLocomotionReader,
    pub(super) participant: DefinitionRef,
    pub(super) known_participants: Vec<DefinitionRef>,
}
impl ComposedOwnerReader {
    fn require_owner(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        if ctx.typed_state.module_owner() != Some(&self.participant.key().owner) {
            return Err(refused());
        }
        Ok(())
    }
    fn project(
        &self,
        receipt: ComposedLocomotionReceipt,
    ) -> Result<Option<ComposedOwnerReceipt>, ParticipantError> {
        if receipt.effects().iter().any(|effect| {
            effect.participant.key().owner == self.participant.key().owner
                && !self.known_participants.contains(&effect.participant)
        }) {
            return Err(refused());
        }
        let Some(effect) = receipt
            .effects()
            .iter()
            .find(|effect| effect.participant.key() == self.participant.key())
        else {
            return Ok(None);
        };
        if effect.participant != self.participant {
            return Err(refused());
        }
        Ok(Some(ComposedOwnerReceipt {
            commit: receipt.commit(),
            action: receipt.action().clone(),
            actor: receipt.actor(),
            source: receipt.source(),
            exit: receipt.exit(),
            destination: receipt.destination(),
            prepared_at: receipt.prepared_at().clone(),
            at_utc: receipt.at_utc(),
            manifest_digest: receipt.manifest_digest(),
            effect: effect.clone(),
        }))
    }
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<ComposedOwnerReceipt>, ParticipantError> {
        self.require_owner(ctx)?;
        let Some(receipt) = self.reader.read_internal(ctx, commit)? else {
            return Ok(None);
        };
        self.project(receipt)
    }
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ComposedOwnerReceiptPage, ParticipantError> {
        self.require_owner(ctx)?;
        let page = self.reader.scan_internal(ctx, after, limit)?;
        let next_after = page.next_after().map(ToString::to_string);
        let mut entries = Vec::new();
        for receipt in page.entries {
            if let Some(receipt) = self.project(receipt)? {
                entries.push(receipt);
            }
        }
        Ok(ComposedOwnerReceiptPage {
            entries,
            next_after,
        })
    }
}
