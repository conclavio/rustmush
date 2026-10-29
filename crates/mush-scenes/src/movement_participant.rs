//! The actual Scenes owner joins transcript appends to one movement commit.
//! Empty captures retain a checked owner observation and produce no log write.
//! This adapter neither invokes legacy observers nor enables a command route.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::ledger::BoundLedgerBatch;
use mush_core::locomotion::scene_transition::{
    self, SceneLedgerAppendRef, ScenesMovementDraft, ScenesMovementIntent,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ClockStamp, DefinitionKind, DefinitionRegistryBuilder, DefinitionValidationContext, ModError,
    ObjId, OwnerId, PreparedLedgerAppend, Registrar, ReplayWrite,
};

use crate::movement::{self, MovementCapture};
use crate::{NS, ScenesState};

/// The selected log is bounded before any existing tail clone or comparison.
pub const MAX_MOVEMENT_RESIDENT_LOG_BYTES: usize = 1_048_576;

fn refused(message: &'static str) -> ParticipantError {
    ParticipantError::refused(message)
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let descriptor = scene_transition::scenes_participant_descriptor()?;
    reg.definition_validator(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        |ctx: &DefinitionValidationContext<'_>| {
            let descriptor =
                scene_transition::scenes_participant_descriptor().map_err(|e| e.to_string())?;
            let payload = descriptor.payload().clone();
            let expected = DefinitionRegistryBuilder::new()
                .register(OwnerId::new(NS).map_err(|e| e.to_string())?, descriptor)
                .map_err(|e| e.to_string())?;
            if ctx.definition().reference() != &expected
                || ctx.definition().payload() != &payload
                || !ctx.definition().references().is_empty()
            {
                return Err("Scenes movement participant identity differs".into());
            }
            Ok(())
        },
    )?;
    let participant = reg.definition(descriptor.as_current())?;
    let action = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").map_err(bad)?,
            scene_transition::action_descriptor()?,
        )
        .map_err(bad)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        ScenesMovementParticipant,
    )?;
    let observer = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(NS).map_err(bad)?,
            scene_transition::observer_descriptor()?,
        )
        .map_err(bad)?;
    reg.register_moved_completion::<ScenesMovementParticipant>(
        "scene_movement_completion",
        1,
        action,
        participant,
        vec![observer],
    )?;
    Ok(())
}

/// The adapter type is public only so the actual Sim provider can request its
/// registration-time handle. Neither it nor a capture grants movement power.
pub struct ScenesMovementParticipant;

struct EventsPatch {
    capture: MovementCapture,
    appends: Vec<PreparedLedgerAppend>,
    references: Vec<SceneLedgerAppendRef>,
}

impl EventsPatch {
    fn prepare(
        state: &ScenesState,
        actor: ObjId,
        from: Option<ObjId>,
        to: ObjId,
        at: i64,
    ) -> Result<Self, ParticipantError> {
        let capture = movement::capture(state, actor, from, to, at)
            .map_err(|_| refused("Scenes movement capture is unavailable"))?;
        let mut appends = Vec::with_capacity(capture.events().len());
        let mut references = Vec::with_capacity(capture.events().len());
        for planned in capture.events() {
            let value =
                serde_json::to_value(planned.event()).map_err(|_| ParticipantError::Encoding)?;
            let event_digest = scene_transition::scene_event_digest(&value)?;
            let append = state
                .events
                .prepare_append_bounded(
                    &planned.scene().to_string(),
                    value,
                    MAX_MOVEMENT_RESIDENT_LOG_BYTES,
                )
                .map_err(|_| refused("Scenes movement transcript tail is unavailable"))?;
            references.push(SceneLedgerAppendRef {
                scene: planned.scene().0,
                sequence: append.seq(),
                event_digest,
            });
            appends.push(append);
        }
        state
            .events
            .check_prepared_batch(&appends)
            .map_err(|_| refused("Scenes movement transcript tails differ"))?;
        Ok(Self {
            capture,
            appends,
            references,
        })
    }

    fn recheck(&self, state: &ScenesState) -> Result<(), ParticipantError> {
        let current = movement::capture(
            state,
            self.capture.actor(),
            self.capture.from(),
            self.capture.to(),
            self.capture.at(),
        )
        .map_err(|_| refused("Scenes movement capture is unavailable"))?;
        if current != self.capture {
            return Err(refused("Scenes movement capture changed"));
        }
        state
            .events
            .check_prepared_batch(&self.appends)
            .map_err(|_| refused("Scenes movement transcript tails changed"))
    }

    fn writes(&self) -> Result<Vec<ReplayWrite>, ParticipantError> {
        let owner = OwnerId::new(NS).map_err(|_| ParticipantError::Authority)?;
        self.appends
            .iter()
            .map(|append| {
                append
                    .authoritative_replay_write(owner.clone())
                    .map_err(|_| ParticipantError::Encoding)
            })
            .collect()
    }

    fn bind(self, state: &mut ScenesState) -> Result<BoundLedgerBatch<'_>, ParticipantError> {
        self.recheck(state)?;
        state
            .events
            .bind_prepared_batch(self.appends)
            .map_err(|_| refused("Scenes movement transcript tails changed"))
    }
}

pub struct ScenesMovementPatch {
    binding: OwnerPreparationBinding,
    prepared_at: ClockStamp,
    events: EventsPatch,
}

pub struct BoundScenesMovement<'a> {
    events: BoundLedgerBatch<'a>,
}
impl BoundParticipant for BoundScenesMovement<'_> {
    fn apply(self) {
        let _ = self.events.apply();
    }
}

impl OwnerParticipant for ScenesMovementParticipant {
    type State = ScenesState;
    type Request = ScenesMovementIntent;
    type Output = ScenesMovementDraft;
    type Patch = ScenesMovementPatch;
    type Bound<'a> = BoundScenesMovement<'a>;

    fn prepare(
        &self,
        state: &ScenesState,
        ctx: &OwnerPrepareContext<'_>,
        intent: ScenesMovementIntent,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let events = EventsPatch::prepare(
            state,
            intent.actor(),
            intent.source(),
            intent.destination(),
            intent.at_utc(),
        )?;
        let capture_value =
            serde_json::to_value(&events.capture).map_err(|_| ParticipantError::Encoding)?;
        let output = ScenesMovementDraft::new(
            ctx,
            &intent,
            scene_transition::scene_capture_digest(&capture_value)?,
            events.references.clone(),
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
        patch: &ScenesMovementPatch,
    ) -> Result<(), ParticipantError> {
        let now = ctx.world().world_ic_now();
        if !ctx.matches_preparation(&patch.binding)
            || now.clock != patch.prepared_at.clock
            || now.at < patch.prepared_at.at
        {
            return Err(refused("Scenes movement preparation changed"));
        }
        patch.events.recheck(state)
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut ScenesState,
        patch: ScenesMovementPatch,
    ) -> Result<BoundScenesMovement<'a>, ParticipantError> {
        Ok(BoundScenesMovement {
            events: patch.events.bind(state)?,
        })
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write, ReplayWrite::LedgerChunkSet { write }
            if write.namespace().as_str() == NS
                && write.log_id().parse::<u64>().is_ok_and(|id| id.to_string() == write.log_id()))
    }
}

#[cfg(test)]
#[path = "movement_participant_tests.rs"]
mod tests;
