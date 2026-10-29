//! Closed ordinary locomotion images paired with the actual Sim participant.
//! Normal dispatch does not select this protocol until movement completion is
//! durable. Its receipts are physical evidence, never personnel eligibility.

use std::collections::BTreeSet;

use crate::action::{ActionInvocation, CommitToken};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, OwnerId,
    canonical_json_bytes_with_limit,
};
use crate::participant::{
    OwnerParticipant, OwnerPreparationBinding, OwnerPrepareContext, ParticipantError,
    ParticipantHandle,
};
use crate::{ClockRef, ClockStamp, DefinitionDigest, ModError, ModuleCtx, ObjId, Registrar, World};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::LocomotionAdmission;

pub const ACTION: &str = "sim_locomotion";
pub const PARTICIPANT: &str = "locomotion_transition";
const FAMILY: &str = "locomotion_transitions";
pub const MAX_LOCOMOTION_READ_DEPENDENCIES: usize = 512;
pub const MAX_LOCOMOTION_RECEIPT_SCAN: usize = 128;
const RECORD_BYTES: usize = 16 * 1024;

pub fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(ACTION).map_err(bad)?,
        1,
        json!({"format":1,"protocol":"sim_captured_locomotion_v1",
            "moves":1,"objects":3,"subtree":256,
            "read_dependencies":MAX_LOCOMOTION_READ_DEPENDENCIES,
            "image_bytes":1_048_576,"receipt_bytes":RECORD_BYTES,
            "admission":"actual_captured_command",
            "movement_hooks":"ordinary_exit_aware",
            "capture":{"family":"sim.locomotion_crossings","required":"same_commit",
                "evidence":"uncertified"}}),
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}

fn exact_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    let expected = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").map_err(|_| refused())?,
            action_descriptor().map_err(|_| refused())?,
        )
        .map_err(|_| refused())?;
    if action != &expected {
        return Err(refused());
    }
    Ok(())
}
fn exact_participant(participant: &DefinitionRef) -> Result<(), ParticipantError> {
    if participant.key().owner.as_str() != "sim"
        || participant.key().kind.as_str() != "owner_participant"
        || participant.key().id.as_str() != PARTICIPANT
        || participant.version() != 1
    {
        return Err(refused());
    }
    Ok(())
}

/// Core-minted input to the exact registered spatial participant. The provider
/// cannot construct, clone, deserialize, or replace its route or clock stamp.
pub struct LocomotionIntent {
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
}
impl std::fmt::Debug for LocomotionIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocomotionIntent")
            .field("actor", &self.actor())
            .field("source", &self.source)
            .field("exit", &self.exit)
            .field("destination", &self.destination)
            .finish_non_exhaustive()
    }
}
impl LocomotionIntent {
    pub(crate) fn new(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        exact_action(action)?;
        Self::new_bound(world, invocation, action, commit, admission)
    }
    pub(crate) fn new_with_scenes(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        super::scene_transition::exact_action(action)?;
        Self::new_bound(world, invocation, action, commit, admission)
    }
    pub(crate) fn new_composed(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        super::composed::exact_action(action)?;
        Self::new_bound(world, invocation, action, commit, admission)
    }
    fn new_bound(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        if admission.binding.invocation != *invocation
            || admission.binding.action != *action
            || admission.binding.commit != commit
            || admission.actor() != super::authenticated_actor(invocation)?
        {
            return Err(refused());
        }
        let prepared_at = world.world_ic_now();
        if !matches!(prepared_at.clock, ClockRef::WorldIc { generation } if generation != 0) {
            return Err(refused());
        }
        Ok(Self {
            invocation: invocation.clone(),
            action: action.clone(),
            commit,
            source: admission.source(),
            exit: admission.exit(),
            destination: admission.destination(),
            prepared_at,
            at_utc: world.now(),
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
    fn matches_context(&self, ctx: &OwnerPrepareContext<'_>) -> bool {
        ctx.invocation() == &self.invocation
            && ctx.action() == &self.action
            && ctx.commit_token() == self.commit
    }
}

/// The participant returns only its own digests and bounded read dependencies.
/// Core consumes the genuine preparation binding; no replacement move is accepted.
pub struct LocomotionDraft {
    pub(crate) binding: OwnerPreparationBinding,
    pub(crate) plan_digest: DefinitionDigest,
    pub(crate) capture_digest: DefinitionDigest,
    pub(crate) dependencies: BTreeSet<ObjId>,
    pub(crate) prepared_at: ClockStamp,
    pub(crate) at_utc: i64,
}
impl LocomotionDraft {
    pub fn new(
        ctx: &OwnerPrepareContext<'_>,
        intent: &LocomotionIntent,
        plan_digest: DefinitionDigest,
        capture_digest: DefinitionDigest,
        read_dependencies: BTreeSet<ObjId>,
    ) -> Result<Self, ParticipantError> {
        // Only Core can mint an intent, for one of its closed protocols. The
        // exact complete context comparison below preserves that authority.
        if !intent.matches_context(ctx)
            || read_dependencies.len() > MAX_LOCOMOTION_READ_DEPENDENCIES
            || intent.prepared_at != ctx.world().world_ic_now()
            || intent.at_utc != ctx.world().now()
        {
            return Err(refused());
        }
        Ok(Self {
            binding: ctx.preparation_binding(),
            plan_digest,
            capture_digest,
            dependencies: read_dependencies,
            prepared_at: intent.prepared_at.clone(),
            at_utc: intent.at_utc,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocomotionObjectDigest {
    pub object: ObjId,
    pub before: DefinitionDigest,
    pub after: DefinitionDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    participant: DefinitionRef,
    actor: ObjId,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    plan_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    objects: Vec<LocomotionObjectDigest>,
}
impl Record {
    fn validate(&self) -> Result<(), ParticipantError> {
        exact_action(&self.action)?;
        exact_participant(&self.participant)?;
        if self.format != 1
            || self.actor == self.destination
            || self.source == Some(self.actor)
            || self.exit.is_some_and(|exit| {
                self.source.is_none()
                    || exit == self.actor
                    || Some(exit) == self.source
                    || exit == self.destination
            })
            || !matches!(self.prepared_at.clock, ClockRef::WorldIc { generation } if generation != 0)
            || self.objects.len() > 3
            || self.objects.windows(2).any(|w| w[0].object >= w[1].object)
            || self
                .objects
                .iter()
                .any(|object| object.before == object.after)
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
pub struct LocomotionReceipt {
    row: Record,
}
impl LocomotionReceipt {
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
    pub fn object_digests(&self) -> &[LocomotionObjectDigest] {
        &self.row.objects
    }
}
#[derive(Debug)]
pub struct LocomotionReceiptPage {
    entries: Vec<LocomotionReceipt>,
    next_after: Option<String>,
}
impl LocomotionReceiptPage {
    pub fn entries(&self) -> &[LocomotionReceipt] {
        &self.entries
    }
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }
}

pub struct CoreLocomotion<P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>>
{
    pub(crate) participant: DefinitionRef,
    pub(crate) handle: ParticipantHandle<P>,
    action: DefinitionRef,
    records: StateFamilyHandle<CommitToken, Record>,
}
impl<P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>> Clone
    for CoreLocomotion<P>
{
    fn clone(&self) -> Self {
        Self {
            participant: self.participant.clone(),
            handle: self.handle.clone(),
            action: self.action.clone(),
            records: self.records.clone(),
        }
    }
}
impl<P: OwnerParticipant<Request = LocomotionIntent, Output = LocomotionDraft>> CoreLocomotion<P> {
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participant: DefinitionRef,
    ) -> Result<Self, ModError> {
        exact_action(&action).map_err(bad)?;
        exact_participant(&participant).map_err(bad)?;
        if reg.module() != "sim" {
            return Err(bad("ordinary locomotion requires Sim ownership"));
        }
        let handle = reg.request_owner_participant::<P>(action.clone())?;
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
        .and_then(|d| d.with_limits(128, RECORD_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &CommitToken| Ok(key.to_string()),
            decode,
            |row: &Record| row.validate().map_err(|e| e.to_string()),
            |row: &Record| bounded(row).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            participant,
            handle,
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
        draft: LocomotionDraft,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(LocomotionReceipt, super::image::LocomotionObjects), ParticipantError> {
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
        let image = super::image::LocomotionObjects::prepare(
            world,
            admission,
            &draft.dependencies,
            self.participant.clone(),
            action.clone(),
            commit,
        )?;
        let row = Record {
            format: 1,
            commit,
            action: action.clone(),
            participant: self.participant.clone(),
            actor: admission.actor(),
            source: admission.source(),
            exit: admission.exit(),
            destination: admission.destination(),
            prepared_at: draft.prepared_at,
            at_utc: draft.at_utc,
            plan_digest: draft.plan_digest,
            capture_digest: draft.capture_digest,
            objects: image.digests()?,
        };
        row.validate()?;
        txn.insert(&self.records, &commit, row.clone())
            .map_err(|_| refused())?;
        Ok((LocomotionReceipt { row }, image))
    }
    fn validate_registry(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let definitions = ctx.reg.definitions().ok_or_else(refused)?;
        definitions
            .inspect_exact(&self.action)
            .map_err(|_| refused())?;
        definitions
            .inspect_exact(&self.participant)
            .map_err(|_| refused())?;
        Ok(())
    }
    fn validate_record(&self, key: &str, row: &Record) -> Result<(), ParticipantError> {
        row.validate()?;
        if key != row.commit.to_string()
            || row.action != self.action
            || row.participant != self.participant
        {
            return Err(refused());
        }
        Ok(())
    }
    pub fn read_receipt(
        &self,
        ctx: &ModuleCtx<'_>,
        commit: &CommitToken,
    ) -> Result<Option<LocomotionReceipt>, ParticipantError> {
        self.validate_registry(ctx)?;
        let Some(row) = ctx
            .action_state(&self.records, commit)
            .map_err(|_| refused())?
        else {
            return Ok(None);
        };
        self.validate_record(&commit.to_string(), row)?;
        Ok(Some(LocomotionReceipt { row: row.clone() }))
    }
    pub fn scan_receipts(
        &self,
        ctx: &ModuleCtx<'_>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<LocomotionReceiptPage, ParticipantError> {
        if limit == 0 || limit > MAX_LOCOMOTION_RECEIPT_SCAN {
            return Err(refused());
        }
        self.validate_registry(ctx)?;
        let page = ctx
            .scan_action_state(&self.records, after, limit)
            .map_err(|_| refused())?;
        let mut entries = Vec::with_capacity(page.entries().len());
        for entry in page.entries() {
            self.validate_record(entry.encoded_key(), entry.value())?;
            entries.push(LocomotionReceipt {
                row: entry.value().clone(),
            });
        }
        Ok(LocomotionReceiptPage {
            entries,
            next_after: page.next_after().map(ToString::to_string),
        })
    }
    pub fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = self.scan_receipts(ctx, cursor.as_deref(), MAX_LOCOMOTION_RECEIPT_SCAN)?;
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
        return Err("noncanonical locomotion receipt".into());
    }
    Ok(row)
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
