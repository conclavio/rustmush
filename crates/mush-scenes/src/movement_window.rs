//! Actual transcript adapter for the closed window profile's single Core leg.
//! A second room projection refuses until ordered transcript capture exists.

use std::{collections::BTreeSet, sync::Arc};

use mush_core::locomotion::composed::{self, MovementDraft, MovementIntent};
use mush_core::locomotion::scene_transition;
use mush_core::participant::{
    OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ModError,
    OwnerId, Registrar, ReplayWrite,
};
use serde_json::json;

use crate::movement_composed::ComposedScenesMovementParticipant;
use crate::movement_participant::{BoundScenesMovement, ScenesMovementPatch};
use crate::{NS, ScenesState};

pub const PARTICIPANT: &str = "window_movement_transcript";
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new(PARTICIPANT).map_err(bad)?,
        1,
        json!({"state":"ScenesState","protocol":"window_scene_movement_v1",
            "max_events":2,"max_witnesses":256,"capture_bytes":32768,
            "resident_log_bytes":1_048_576,"input_depth":120,
            "projection":"admitted_room_only","writes":"scene_ledger_appends",
            "empty":"retained_owner_observation","delivery":"same_commit",
            "proof":"original_ledger_positions_v1"}),
    )
    .map_err(bad)
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let participant = reg.definition(participant_descriptor()?.as_current())?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            OwnerId::new("sim").map_err(bad)?,
            composed::window_action_descriptor()?,
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
        WindowScenesMovementParticipant,
    )?;
    reg.register_moved_completion::<WindowScenesMovementParticipant>(
        "window_scene_movement_completion",
        1,
        action,
        participant,
        vec![observer],
    )?;
    Ok(())
}

/// Request only this actual owner's registration-time handle at composition.
pub struct WindowScenesMovementParticipant;
impl OwnerParticipant for WindowScenesMovementParticipant {
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
    ) -> Result<OwnerPreparation<MovementDraft, ScenesMovementPatch>, ParticipantError> {
        let projection = intent
            .window_projection()
            .ok_or(ParticipantError::Authority)?;
        if projection.projected_room() != intent.destination() {
            return Err(ParticipantError::refused(
                "ordered projected scene transcripts are unavailable",
            ));
        }
        ComposedScenesMovementParticipant.prepare(state, ctx, intent)
    }
    fn recheck(
        &self,
        state: &ScenesState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &ScenesMovementPatch,
    ) -> Result<(), ParticipantError> {
        ComposedScenesMovementParticipant.recheck(state, ctx, patch)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut ScenesState,
        patch: ScenesMovementPatch,
    ) -> Result<BoundScenesMovement<'a>, ParticipantError> {
        Arc::new(ComposedScenesMovementParticipant).bind(state, patch)
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        ComposedScenesMovementParticipant.accepts_write(write)
    }
}
