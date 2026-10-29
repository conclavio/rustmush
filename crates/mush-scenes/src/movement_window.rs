//! Actual ordered transcript adapter for both closed window Core room legs.
//! Original single-leg receipts remain readable through their retained profile.

use std::{collections::BTreeSet, sync::Arc};

use mush_core::ledger::{BoundLedgerSequence, PreparedLedgerSequence};
use mush_core::locomotion::composed::{self, MovementDraft, MovementIntent};
use mush_core::locomotion::scene_transition;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ClockStamp, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    ModError, OwnerId, Registrar, ReplayWrite,
};
use serde_json::json;

use crate::movement_participant::{MAX_MOVEMENT_RESIDENT_LOG_BYTES, ScenesMovementParticipant};
use crate::movement_window_capture::{self, WindowSceneCapture};
use crate::{NS, ScenesState};

#[path = "movement_window_proof.rs"]
mod proof;
pub(crate) use proof::ordered_claims;

pub const PARTICIPANT: &str = "window_movement_transcript";
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub fn participant_descriptor_v1() -> Result<DefinitionDescriptor, ModError> {
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

pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new(PARTICIPANT).map_err(bad)?,
        2,
        json!({"state":"ScenesState","protocol":"ordered_window_scene_movement_v2",
            "max_events":4,"max_core_legs":2,"max_witnesses":256,"capture_bytes":65536,
            "resident_log_bytes":1_048_576,"input_depth":120,
            "projection":"original_admitted_then_projected_room",
            "writes":"ordered_scene_ledger_appends","empty":"retained_owner_observation",
            "delivery":"same_commit","proof":"original_ordered_ledger_positions_v2"}),
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
    let old = reg.definition(participant_descriptor_v1()?)?;
    reg.definition(mush_core::locomotion::completion::binding_descriptor(
        "window_scene_movement_completion",
        1,
        action.clone(),
        old,
        vec![observer.clone()],
    )?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        WindowScenesMovementParticipant,
    )?;
    reg.register_moved_completion::<WindowScenesMovementParticipant>(
        "window_scene_movement_completion",
        2,
        action,
        participant,
        vec![observer],
    )?;
    Ok(())
}

fn refused() -> ParticipantError {
    ParticipantError::refused("ordered window scene transcript is unavailable or changed")
}

struct WindowEvents {
    capture: WindowSceneCapture,
    sequence: PreparedLedgerSequence,
}
impl WindowEvents {
    fn prepare(state: &ScenesState, capture: WindowSceneCapture) -> Result<Self, ParticipantError> {
        let entries = capture
            .events()
            .map(|event| {
                Ok((
                    event.scene().to_string(),
                    serde_json::to_value(event.event()).map_err(|_| ParticipantError::Encoding)?,
                ))
            })
            .collect::<Result<Vec<_>, ParticipantError>>()?;
        let sequence = state
            .events
            .prepare_sequence_bounded(entries, MAX_MOVEMENT_RESIDENT_LOG_BYTES)
            .map_err(|_| refused())?;
        Ok(Self { capture, sequence })
    }
    fn recheck(&self, state: &ScenesState) -> Result<(), ParticipantError> {
        self.capture.recheck(state).map_err(|_| refused())?;
        state
            .events
            .check_prepared_sequence(&self.sequence)
            .map_err(|_| refused())
    }
    fn bind(self, state: &mut ScenesState) -> Result<BoundLedgerSequence<'_>, ParticipantError> {
        self.recheck(state)?;
        state
            .events
            .bind_prepared_sequence(self.sequence)
            .map_err(|_| refused())
    }
}

pub struct WindowScenesMovementPatch {
    binding: OwnerPreparationBinding,
    prepared_at: ClockStamp,
    events: WindowEvents,
}
pub struct BoundWindowScenesMovement<'a> {
    events: BoundLedgerSequence<'a>,
}
impl BoundParticipant for BoundWindowScenesMovement<'_> {
    fn apply(self) {
        let _ = self.events.apply();
    }
}

/// Request only this actual owner's registration-time handle at composition.
pub struct WindowScenesMovementParticipant;
impl OwnerParticipant for WindowScenesMovementParticipant {
    type State = ScenesState;
    type Request = MovementIntent;
    type Output = MovementDraft;
    type Patch = WindowScenesMovementPatch;
    type Bound<'a> = BoundWindowScenesMovement<'a>;

    fn prepare(
        &self,
        state: &ScenesState,
        ctx: &OwnerPrepareContext<'_>,
        intent: MovementIntent,
    ) -> Result<OwnerPreparation<MovementDraft, WindowScenesMovementPatch>, ParticipantError> {
        let projection = intent
            .window_projection()
            .ok_or(ParticipantError::Authority)?;
        let capture = movement_window_capture::capture(
            state,
            intent.actor(),
            intent.source(),
            intent.destination(),
            projection.projected_room(),
            intent.at_utc(),
        )
        .map_err(|_| refused())?;
        let events = WindowEvents::prepare(state, capture)?;
        let output = MovementDraft::new(
            ctx,
            &intent,
            events.capture.digest().map_err(|_| refused())?,
            proof::make(&events.capture, &events.sequence)?,
        )?;
        let writes = events
            .sequence
            .authoritative_replay_writes(OwnerId::new(NS).map_err(|_| refused())?)
            .map_err(|_| refused())?;
        let patch = WindowScenesMovementPatch {
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
        patch: &WindowScenesMovementPatch,
    ) -> Result<(), ParticipantError> {
        let now = ctx.world().world_ic_now();
        if !ctx.matches_preparation(&patch.binding)
            || now.clock != patch.prepared_at.clock
            || now.at < patch.prepared_at.at
        {
            return Err(refused());
        }
        patch.events.recheck(state)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut ScenesState,
        patch: WindowScenesMovementPatch,
    ) -> Result<BoundWindowScenesMovement<'a>, ParticipantError> {
        Ok(BoundWindowScenesMovement {
            events: patch.events.bind(state)?,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        ScenesMovementParticipant.accepts_write(write)
    }
}

#[cfg(test)]
#[path = "movement_window_native_tests.rs"]
mod tests;
