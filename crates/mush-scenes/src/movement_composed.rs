//! Real transcript owner for Core's composition-bound movement protocol.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::locomotion::composed::{self, ComposedOwnerReceipt, MovementDraft, MovementIntent};
use mush_core::locomotion::scene_transition::{self, SceneLedgerAppendRef};
use mush_core::participant::{
    OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ModError,
    OwnerId, Registrar, ReplayWrite,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::movement_participant::{
    BoundScenesMovement, EventsPatch, ScenesMovementParticipant, ScenesMovementPatch,
};
use crate::{NS, ScenesState};

pub const PARTICIPANT: &str = "composed_movement_transcript";

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new(PARTICIPANT).map_err(bad)?,
        1,
        json!({"state":"ScenesState","protocol":"composed_scene_movement_v1",
            "max_events":2,"max_witnesses":256,"capture_bytes":32768,
            "resident_log_bytes":1_048_576,"input_depth":120,
            "writes":"scene_ledger_appends","empty":"retained_owner_observation",
            "delivery":"same_commit","proof":"original_ledger_positions_v1"}),
    )
    .map_err(bad)
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let participant = reg.definition(participant_descriptor()?.as_current())?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            OwnerId::new("sim").map_err(bad)?,
            composed::action_descriptor()?,
        )
        .map_err(bad)?;
    let observer = definitions
        .register(
            OwnerId::new(NS).map_err(bad)?,
            scene_transition::observer_descriptor()?,
        )
        .map_err(bad)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        ComposedScenesMovementParticipant,
    )?;
    reg.register_moved_completion::<ComposedScenesMovementParticipant>(
        "composed_scene_movement_completion",
        1,
        action,
        participant,
        vec![observer],
    )?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    format: u16,
    appends: Vec<SceneLedgerAppendRef>,
}

/// Decode only this owner's exact proof profile. The Core reader already
/// authenticates the participant binding and original movement header.
pub(crate) fn appends(
    receipt: &ComposedOwnerReceipt,
) -> Result<Vec<SceneLedgerAppendRef>, ModError> {
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(NS).map_err(bad)?, participant_descriptor()?)
        .map_err(bad)?;
    if receipt.effect().participant() != &expected {
        return Err(bad("unknown composed Scenes proof profile"));
    }
    let value = receipt.effect().proof();
    mush_core::canonical_json_bytes_with_limit(value, 4096).map_err(bad)?;
    let proof: Proof = serde_json::from_value(value.clone()).map_err(bad)?;
    if proof.format != 1
        || proof.appends.len() > 2
        || (proof.appends.len() == 2 && proof.appends[0].scene == proof.appends[1].scene)
        || (receipt.source() == Some(receipt.destination()) && !proof.appends.is_empty())
        || serde_json::to_value(&proof).map_err(bad)? != *value
    {
        return Err(bad("invalid composed Scenes proof"));
    }
    Ok(proof.appends)
}

/// Public solely for requesting the actual typed adapter at composition.
pub struct ComposedScenesMovementParticipant;
impl OwnerParticipant for ComposedScenesMovementParticipant {
    type State = ScenesState;
    type Request = MovementIntent;
    type Output = MovementDraft;
    type Patch = ScenesMovementPatch;
    type Bound<'a> = BoundScenesMovement<'a>;

    fn prepare(
        &self,
        state: &ScenesState,
        ctx: &OwnerPrepareContext<'_>,
        intent: MovementIntent,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let events = EventsPatch::prepare(
            state,
            intent.actor(),
            intent.source(),
            intent.destination(),
            intent.at_utc(),
        )?;
        let value =
            serde_json::to_value(&events.capture).map_err(|_| ParticipantError::Encoding)?;
        let proof = serde_json::to_value(Proof {
            format: 1,
            appends: events.references.clone(),
        })
        .map_err(|_| ParticipantError::Encoding)?;
        let output = MovementDraft::new(
            ctx,
            &intent,
            scene_transition::scene_capture_digest(&value)?,
            proof,
        )?;
        let writes = events.writes()?;
        let patch = ScenesMovementPatch {
            binding: ctx.preparation_binding(),
            prepared_at: intent.prepared_at().clone(),
            events,
        };
        Ok(OwnerPreparation::new(output, patch, writes))
    }
    fn recheck(
        &self,
        state: &ScenesState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        ScenesMovementParticipant.recheck(state, ctx, patch)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut ScenesState,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        Arc::new(ScenesMovementParticipant).bind(state, patch)
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        ScenesMovementParticipant.accepts_write(write)
    }
}
