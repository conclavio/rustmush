//! Closed opt-in locomotion with the actual Scenes transcript in the same ACK.
//! This does not dispatch ordinary movement or complete any other Moved owner.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, OwnerId,
};
use crate::participant::{
    OwnerParticipant, OwnerPreparationBinding, OwnerPrepareContext, ParticipantError,
    ParticipantHandle,
};
use crate::{
    ActionInvocation, ClockRef, ClockStamp, CommitToken, DefinitionDigest, ModError, ModuleCtx,
    ObjId, Registrar, ReplayWrite, World, canonical_json_bytes_with_limit,
};

use super::LocomotionAdmission;
use super::transition::{LocomotionDraft, LocomotionIntent, LocomotionObjectDigest};

pub const ACTION: &str = "sim_locomotion_with_scenes";
pub const SIM_PARTICIPANT: &str = "locomotion_transition_with_scenes";
pub const SCENES_PARTICIPANT: &str = "movement_transcript";
pub const OBSERVER: &str = "scene_movement_transcript";
const FAMILY: &str = "locomotion_scene_transitions";
const RECORD_BYTES: usize = 16 * 1024;
pub const MAX_SCENE_CAPTURE_BYTES: usize = 32 * 1024;
pub const MAX_SCENE_APPENDS: usize = 2;

pub fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(kind("action")?, id(ACTION)?, 1, json!({
        "format":1,"protocol":"sim_captured_locomotion_with_scenes_v1",
        "moves":1,"objects":3,"subtree":256,"read_dependencies":512,
        "image_bytes":1_048_576,"receipt_bytes":RECORD_BYTES,
        "admission":"actual_captured_command","movement_hooks":"ordinary_exit_aware",
        "capture":{"family":"sim.locomotion_scene_crossings","required":"same_commit","evidence":"uncertified"},
        "scenes":{"participant":SCENES_PARTICIPANT,"observer":OBSERVER,"max_appends":MAX_SCENE_APPENDS,
            "capture_bytes":MAX_SCENE_CAPTURE_BYTES,"delivery":"same_commit","empty":"retained_owner_observation"},
        "other_moved_observers_completed":false
    })).map(DefinitionDescriptor::as_current).map_err(bad)
}

pub fn scenes_participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(kind("owner_participant")?, id(SCENES_PARTICIPANT)?, 1, json!({
        "state":"ScenesState","protocol":"captured_scene_movement_v1",
        "max_events":2,"max_witnesses":256,"capture_bytes":MAX_SCENE_CAPTURE_BYTES,
        "resident_log_bytes":1_048_576,"input_depth":120,
        "writes":"scene_ledger_appends","empty":"retained_owner_observation","delivery":"same_commit"
    })).map_err(bad)
}

pub fn observer_descriptor() -> Result<DefinitionDescriptor, ModError> {
    crate::moved_observer::descriptor(OBSERVER, 1)
}

fn kind(value: &str) -> Result<DefinitionKind, ModError> {
    DefinitionKind::new(value).map_err(bad)
}
fn id(value: &str) -> Result<DefinitionId, ModError> {
    DefinitionId::new(value).map_err(bad)
}
fn expected(
    owner: &str,
    descriptor: DefinitionDescriptor,
) -> Result<DefinitionRef, ParticipantError> {
    crate::DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner).map_err(|_| refused())?, descriptor)
        .map_err(|_| refused())
}
pub(crate) fn exact_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    if action != &expected("sim", action_descriptor().map_err(|_| refused())?)? {
        return Err(refused());
    }
    Ok(())
}
fn exact_scenes(
    participant: &DefinitionRef,
    observer: &DefinitionRef,
) -> Result<(), ParticipantError> {
    if participant
        != &expected(
            "scenes",
            scenes_participant_descriptor().map_err(|_| refused())?,
        )?
        || observer != &expected("scenes", observer_descriptor().map_err(|_| refused())?)?
    {
        return Err(refused());
    }
    Ok(())
}
fn exact_sim(participant: &DefinitionRef) -> Result<(), ParticipantError> {
    if participant.key().owner.as_str() != "sim"
        || participant.key().kind.as_str() != "owner_participant"
        || participant.key().id.as_str() != SIM_PARTICIPANT
        || participant.version() != 1
    {
        return Err(refused());
    }
    Ok(())
}

pub fn scene_event_digest(event: &Value) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "scenes.movement.event.v1",
        event,
        MAX_SCENE_CAPTURE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
pub fn scene_capture_digest(capture: &Value) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "scenes.movement.capture.v1",
        capture,
        MAX_SCENE_CAPTURE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

/// Core-minted request to the actual Scenes owner. No public constructor,
/// deserializer or clone can substitute a route, observer, time or invocation.
pub struct ScenesMovementIntent {
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    observer: DefinitionRef,
}
impl ScenesMovementIntent {
    pub(crate) fn new(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
        observer: &DefinitionRef,
    ) -> Result<Self, ParticipantError> {
        let intent =
            LocomotionIntent::new_with_scenes(world, invocation, action, commit, admission)?;
        if observer != &expected("scenes", observer_descriptor().map_err(|_| refused())?)? {
            return Err(refused());
        }
        Ok(Self {
            invocation: invocation.clone(),
            action: action.clone(),
            commit,
            source: intent.source(),
            exit: intent.exit(),
            destination: intent.destination(),
            prepared_at: intent.prepared_at().clone(),
            at_utc: intent.at_utc(),
            observer: observer.clone(),
        })
    }
    pub fn actor(&self) -> ObjId {
        self.invocation.actor()
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
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn observer(&self) -> &DefinitionRef {
        &self.observer
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLedgerAppendRef {
    pub scene: u64,
    pub sequence: u64,
    pub event_digest: DefinitionDigest,
}

pub struct ScenesMovementDraft {
    pub(crate) binding: OwnerPreparationBinding,
    capture_digest: DefinitionDigest,
    appends: Vec<SceneLedgerAppendRef>,
}
impl ScenesMovementDraft {
    pub fn new(
        ctx: &OwnerPrepareContext<'_>,
        intent: &ScenesMovementIntent,
        capture_digest: DefinitionDigest,
        appends: Vec<SceneLedgerAppendRef>,
    ) -> Result<Self, ParticipantError> {
        if ctx.invocation() != &intent.invocation
            || ctx.action() != &intent.action
            || ctx.commit_token() != intent.commit
            || ctx.world().world_ic_now() != intent.prepared_at
            || ctx.world().now() != intent.at_utc
        {
            return Err(refused());
        }
        validate_appends(&appends)?;
        Ok(Self {
            binding: ctx.preparation_binding(),
            capture_digest,
            appends,
        })
    }
}
fn validate_appends(appends: &[SceneLedgerAppendRef]) -> Result<(), ParticipantError> {
    if appends.len() > MAX_SCENE_APPENDS
        || (appends.len() == 2 && appends[0].scene == appends[1].scene)
    {
        return Err(refused());
    }
    Ok(())
}

/// Check the receipt's semantic append claims against the actual registered
/// owner's prepared writes, not caller-provided substitute bytes.
pub(crate) fn verify_scene_writes(
    draft: &ScenesMovementDraft,
    writes: &[ReplayWrite],
) -> Result<DefinitionDigest, ParticipantError> {
    verify_scene_write_claims(&draft.appends, writes)
}
fn verify_scene_write_claims(
    appends: &[SceneLedgerAppendRef],
    writes: &[ReplayWrite],
) -> Result<DefinitionDigest, ParticipantError> {
    validate_appends(appends)?;
    if writes.len() != appends.len() {
        return Err(refused());
    }
    for (append, write) in appends.iter().zip(writes) {
        let ReplayWrite::LedgerChunkSet { write } = write else {
            return Err(refused());
        };
        let Some(entry) = write.chunk().entries.last() else {
            return Err(refused());
        };
        if write.namespace().as_str() != "scenes"
            || write.log_id() != append.scene.to_string()
            || entry.seq != append.sequence
            || scene_event_digest(&entry.value)? != append.event_digest
        {
            return Err(refused());
        }
    }
    DefinitionDigest::for_canonical_value_with_limit(
        "core.scene_movement.writes.v1",
        &serde_json::to_value(writes).map_err(|_| ParticipantError::Encoding)?,
        crate::authoritative::MAX_AUTHORITATIVE_BUNDLE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    participant: DefinitionRef,
    scenes_participant: DefinitionRef,
    observer: DefinitionRef,
    actor: ObjId,
    #[serde(deserialize_with = "required_option")]
    source: Option<ObjId>,
    #[serde(deserialize_with = "required_option")]
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    plan_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    scenes_capture_digest: DefinitionDigest,
    appends: Vec<SceneLedgerAppendRef>,
    scenes_writes_digest: DefinitionDigest,
    objects: Vec<LocomotionObjectDigest>,
}
impl Record {
    fn validate(&self) -> Result<(), ParticipantError> {
        exact_action(&self.action)?;
        exact_sim(&self.participant)?;
        exact_scenes(&self.scenes_participant, &self.observer)?;
        validate_appends(&self.appends)?;
        if self.format != 1
            || self.actor == self.destination
            || self.source == Some(self.actor)
            || (self.source.is_none() && self.appends.len() > 1)
            || self.exit.is_some_and(|exit| {
                self.source.is_none()
                    || exit == self.actor
                    || Some(exit) == self.source
                    || exit == self.destination
            })
            || !matches!(self.prepared_at.clock, ClockRef::WorldIc { generation } if generation != 0)
            || self.objects.len() > 3
            || self.objects.windows(2).any(|w| w[0].object >= w[1].object)
            || self.objects.iter().any(|o| o.before == o.after)
        {
            return Err(refused());
        }
        let expected: BTreeSet<_> = if self.source == Some(self.destination) {
            if self.exit.is_none() || !self.appends.is_empty() {
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
            .map(|o| o.object)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(refused());
        }
        bounded(self)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SceneLocomotionReceipt {
    row: Record,
}
impl SceneLocomotionReceipt {
    pub const fn commit(&self) -> CommitToken {
        self.row.commit
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub const fn participant(&self) -> &DefinitionRef {
        &self.row.participant
    }
    pub const fn scenes_participant(&self) -> &DefinitionRef {
        &self.row.scenes_participant
    }
    pub const fn observer(&self) -> &DefinitionRef {
        &self.row.observer
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
    pub const fn scenes_capture_digest(&self) -> &DefinitionDigest {
        &self.row.scenes_capture_digest
    }
    pub const fn scenes_writes_digest(&self) -> &DefinitionDigest {
        &self.row.scenes_writes_digest
    }
    pub fn appends(&self) -> &[SceneLedgerAppendRef] {
        &self.row.appends
    }
    pub fn object_digests(&self) -> &[LocomotionObjectDigest] {
        &self.row.objects
    }
}
#[derive(Debug)]
pub struct SceneLocomotionReceiptPage {
    entries: Vec<SceneLocomotionReceipt>,
    next_after: Option<String>,
}
impl SceneLocomotionReceiptPage {
    pub fn entries(&self) -> &[SceneLocomotionReceipt] {
        &self.entries
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}

pub struct CoreLocomotionWithScenes<P, S>
where
    P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>,
    S: OwnerParticipant<Request = ScenesMovementIntent, Output = ScenesMovementDraft>,
{
    pub(crate) sim_participant: DefinitionRef,
    pub(crate) scenes_participant: DefinitionRef,
    pub(crate) observer: DefinitionRef,
    pub(crate) sim_handle: ParticipantHandle<P>,
    pub(crate) scenes_handle: ParticipantHandle<S>,
    action: DefinitionRef,
    records: StateFamilyHandle<CommitToken, Record>,
}
impl<P, S> Clone for CoreLocomotionWithScenes<P, S>
where
    P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>,
    S: OwnerParticipant<Request = ScenesMovementIntent, Output = ScenesMovementDraft>,
{
    fn clone(&self) -> Self {
        Self {
            sim_participant: self.sim_participant.clone(),
            scenes_participant: self.scenes_participant.clone(),
            observer: self.observer.clone(),
            sim_handle: self.sim_handle.clone(),
            scenes_handle: self.scenes_handle.clone(),
            action: self.action.clone(),
            records: self.records.clone(),
        }
    }
}
impl<P, S> CoreLocomotionWithScenes<P, S>
where
    P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>,
    S: OwnerParticipant<Request = ScenesMovementIntent, Output = ScenesMovementDraft>,
{
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        sim_participant: DefinitionRef,
        scenes_participant: DefinitionRef,
        observer: DefinitionRef,
    ) -> Result<Self, ModError> {
        exact_action(&action).map_err(bad)?;
        exact_sim(&sim_participant).map_err(bad)?;
        exact_scenes(&scenes_participant, &observer).map_err(bad)?;
        if reg.module() != "sim" {
            return Err(bad("joint locomotion requires Sim ownership"));
        }
        reg.require_current_moved_observer(&observer)?;
        let sim_handle = reg.request_owner_participant::<P>(action.clone())?;
        let scenes_handle = reg.request_owner_participant::<S>(action.clone())?;
        reg.require_moved_completion::<S>(
            action.clone(),
            scenes_participant.clone(),
            vec![observer.clone()],
        )?;
        let records = register_records(reg)?;
        Ok(Self {
            sim_participant,
            scenes_participant,
            observer,
            sim_handle,
            scenes_handle,
            action,
            records,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        admission: &LocomotionAdmission,
        sim: LocomotionDraft,
        scenes: ScenesMovementDraft,
        writes_digest: DefinitionDigest,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(SceneLocomotionReceipt, super::image::LocomotionObjects), ParticipantError> {
        if action != &self.action
            || invocation != &admission.binding.invocation
            || action != &admission.binding.action
            || commit != admission.binding.commit
            || txn
                .get(&self.records, &commit)
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        txn.read_preimage(base, provider).map_err(|_| refused())?;
        let mut image = super::image::LocomotionObjects::prepare(
            world,
            admission,
            &sim.dependencies,
            self.sim_participant.clone(),
            action.clone(),
            commit,
        )?;
        image.scenes_participant = Some(self.scenes_participant.clone());
        let row = Record {
            format: 1,
            commit,
            action: action.clone(),
            participant: self.sim_participant.clone(),
            scenes_participant: self.scenes_participant.clone(),
            observer: self.observer.clone(),
            actor: admission.actor(),
            source: admission.source(),
            exit: admission.exit(),
            destination: admission.destination(),
            prepared_at: sim.prepared_at,
            at_utc: sim.at_utc,
            plan_digest: sim.plan_digest,
            capture_digest: sim.capture_digest,
            scenes_capture_digest: scenes.capture_digest,
            appends: scenes.appends,
            scenes_writes_digest: writes_digest,
            objects: image.digests()?,
        };
        row.validate()?;
        txn.insert(&self.records, &commit, row.clone())
            .map_err(|_| refused())?;
        Ok((SceneLocomotionReceipt { row }, image))
    }
    pub fn reader(&self) -> SceneLocomotionReader {
        SceneLocomotionReader {
            action: self.action.clone(),
            participant: self.sim_participant.clone(),
            scenes_participant: self.scenes_participant.clone(),
            observer: self.observer.clone(),
            records: self.records.clone(),
        }
    }
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<SceneLocomotionReceipt>, ParticipantError> {
        self.reader().read_receipt(ctx, commit)
    }
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<SceneLocomotionReceiptPage, ParticipantError> {
        self.reader().scan_receipts(ctx, after, limit)
    }
    pub fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        self.reader().audit(ctx)
    }
}

/// Exact read-only typed-family capability for Scenes' cold ledger audit.
/// This contains registration handles, never copied authoritative state.
#[derive(Clone)]
pub struct SceneLocomotionReader {
    action: DefinitionRef,
    participant: DefinitionRef,
    scenes_participant: DefinitionRef,
    observer: DefinitionRef,
    records: StateFamilyHandle<CommitToken, Record>,
}
impl SceneLocomotionReader {
    fn validate_registry(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let definitions = ctx.reg.definitions().ok_or_else(refused)?;
        for reference in [
            &self.action,
            &self.participant,
            &self.scenes_participant,
            &self.observer,
        ] {
            definitions
                .inspect_exact(reference)
                .map_err(|_| refused())?;
        }
        Ok(())
    }
    fn validate_record(&self, key: &str, row: &Record) -> Result<(), ParticipantError> {
        row.validate()?;
        if key != row.commit.to_string()
            || row.action != self.action
            || row.participant != self.participant
            || row.scenes_participant != self.scenes_participant
            || row.observer != self.observer
        {
            return Err(refused());
        }
        Ok(())
    }
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<SceneLocomotionReceipt>, ParticipantError> {
        self.validate_registry(ctx)?;
        let Some(row) = ctx
            .action_state(&self.records, commit)
            .map_err(|_| refused())?
        else {
            return Ok(None);
        };
        self.validate_record(&commit.to_string(), row)?;
        Ok(Some(SceneLocomotionReceipt { row: row.clone() }))
    }
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<SceneLocomotionReceiptPage, ParticipantError> {
        if limit == 0 || limit > 128 {
            return Err(refused());
        }
        self.validate_registry(ctx)?;
        let page = ctx
            .scan_action_state(&self.records, after, limit)
            .map_err(|_| refused())?;
        let mut entries = Vec::with_capacity(page.entries().len());
        for entry in page.entries() {
            self.validate_record(entry.encoded_key(), entry.value())?;
            entries.push(SceneLocomotionReceipt {
                row: entry.value().clone(),
            });
        }
        Ok(SceneLocomotionReceiptPage {
            entries,
            next_after: page.next_after().map(ToString::to_string),
        })
    }
    pub fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
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
fn register_records(
    reg: &mut Registrar<'_>,
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
        |row: &Record| row.validate().map_err(|e| e.to_string()),
        |row: &Record| bounded(row).map_err(|e| e.to_string()),
    )?;
    reg.grant_state_family_read(&records, "scenes")?;
    Ok(records)
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn refused() -> ParticipantError {
    ParticipantError::Authority
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn decode(value: &Value) -> Result<Record, String> {
    canonical_json_bytes_with_limit(value, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: Record = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    row.validate().map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical scene locomotion receipt".into());
    }
    Ok(row)
}

#[cfg(test)]
#[path = "scene_transition_tests.rs"]
mod tests;
