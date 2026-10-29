//! Core-minted inputs and exact-context output bindings for prepared observers.

use super::super::LocomotionAdmission;
use super::super::completion::MovementManifestGroup;
use super::{MAX_MOVEMENT_PROOF_BYTES, exact_action, refused};
use crate::participant::{OwnerPreparationBinding, OwnerPrepareContext, ParticipantError};
use crate::{
    ActionInvocation, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World,
};
use serde_json::{Value, json};

/// This cannot be constructed, cloned or deserialized by an owner. The original
/// full invocation, captured route, clocks and actual group stay bound together.
pub struct MovementIntent {
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: ClockStamp,
    at_utc: i64,
    group: MovementManifestGroup,
    intent_digest: DefinitionDigest,
}
impl MovementIntent {
    pub(crate) fn new(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &LocomotionAdmission,
        group: &MovementManifestGroup,
    ) -> Result<Self, ParticipantError> {
        exact_action(action)?;
        if admission.binding.invocation != *invocation
            || admission.binding.action != *action
            || admission.binding.commit != commit
            || admission.actor() != super::super::authenticated_actor(invocation)?
        {
            return Err(refused());
        }
        let prepared_at = world.world_ic_now();
        if !matches!(prepared_at.clock, crate::ClockRef::WorldIc { generation } if generation > 0) {
            return Err(refused());
        }
        let intent_digest = intent_digest(
            action,
            commit,
            admission.actor(),
            admission.source(),
            admission.exit(),
            admission.destination(),
            &prepared_at,
            world.now(),
            group.binding(),
        )?;
        Ok(Self {
            invocation: invocation.clone(),
            action: action.clone(),
            commit,
            source: admission.source(),
            exit: admission.exit(),
            destination: admission.destination(),
            prepared_at,
            at_utc: world.now(),
            group: group.clone(),
            intent_digest,
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
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn group(&self) -> &MovementManifestGroup {
        &self.group
    }
    pub(crate) const fn digest(&self) -> DefinitionDigest {
        self.intent_digest
    }
}

/// The owner supplies bounded observations/proof data. Only the real registered
/// preparation produces writes; Core hashes those actual writes independently.
pub struct MovementDraft {
    pub(crate) binding: OwnerPreparationBinding,
    pub(crate) group_binding: DefinitionRef,
    pub(crate) intent_digest: DefinitionDigest,
    pub(crate) owner_capture_digest: DefinitionDigest,
    pub(crate) capture_digest: DefinitionDigest,
    pub(crate) proof: Value,
}
impl MovementDraft {
    pub fn new(
        ctx: &OwnerPrepareContext<'_>,
        intent: &MovementIntent,
        owner_capture_digest: DefinitionDigest,
        proof: Value,
    ) -> Result<Self, ParticipantError> {
        if ctx.invocation() != &intent.invocation
            || ctx.action() != &intent.action
            || ctx.commit_token() != intent.commit
            || ctx.world().world_ic_now() != intent.prepared_at
            || ctx.world().now() != intent.at_utc
        {
            return Err(refused());
        }
        crate::canonical_json_bytes_with_limit(&proof, MAX_MOVEMENT_PROOF_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        Ok(Self {
            binding: ctx.preparation_binding(),
            group_binding: intent.group.binding().clone(),
            intent_digest: intent.intent_digest,
            owner_capture_digest,
            capture_digest: capture_digest(&intent.intent_digest, &owner_capture_digest)?,
            proof,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn intent_digest(
    action: &DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
    prepared_at: &ClockStamp,
    at_utc: i64,
    binding: &DefinitionRef,
) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "core.composed_locomotion.intent.v1",
        &json!({"action":action,"commit":commit,"actor":actor,
            "source":source,"exit":exit,"destination":destination,"prepared_at":prepared_at,
            "at_utc":at_utc,"binding":binding}),
        16 * 1024,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
pub(crate) fn capture_digest(
    intent: &DefinitionDigest,
    owner_capture: &DefinitionDigest,
) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value(
        "core.composed_locomotion.capture.v1",
        &json!({"intent":intent,"owner_capture":owner_capture}),
    )
    .map_err(|_| ParticipantError::Encoding)
}
