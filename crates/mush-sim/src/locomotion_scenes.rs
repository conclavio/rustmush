//! Opt-in ordinary locomotion with the real Scenes transcript participant.
//!
//! This has no command or offer route. Core supplies the captured admission;
//! the two owners prepare into one acknowledged bundle. Other Moved observers
//! remain legacy, and crossing history remains explicitly uncertified.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use mush_core::locomotion::scene_transition::{self, CoreLocomotionWithScenes};
use mush_core::locomotion::transition::{LocomotionDraft, LocomotionIntent};
use mush_core::participant::{
    OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, ModError, ModuleCtx, OwnerId, PrepareCtx, Registrar, ReplayWrite,
    RulesValue,
};
use mush_scenes::movement_participant::ScenesMovementParticipant;
use serde_json::json;

use crate::data::{RECOVERY_LOG, SpatialState};
use crate::locomotion::{self, LocomotionParticipant};
use crate::locomotion_capture::{ActorMove, LocomotionCrossings};

const OUTCOME: &str = "locomotion_scene_transition_result";
type Service = CoreLocomotionWithScenes<SceneLocomotionParticipant, ScenesMovementParticipant>;

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn unavailable(message: &'static str) -> ActionError {
    crate::tick::err(ActionErrorCode::Unavailable, message)
}

pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(bad)?, DefinitionId::new(OUTCOME).map_err(bad)?, 1,
        json!({
            "fields":["actor","source","exit","destination","prepared_at","plan_digest","capture_digest","scenes_capture_digest"],
            "capture":"sim.locomotion_scene_crossings",
            "scene_transcript":"atomic",
            "other_observer_completion":false,
            "personnel_eligibility":false,
        }),
    ).map(|descriptor| descriptor.as_current()).map_err(bad)
}

pub(crate) fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new(scene_transition::SIM_PARTICIPANT).map_err(bad)?,
        1,
        json!({
            "state":"SpatialState", "documents":[], "ledger":RECOVERY_LOG,
            "operations":["captured_exit","captured_home"],
            "protocol":"sim_locomotion_scene_capture_v1",
            "capture":"sim.locomotion_scene_crossings",
            "authority":"core_captured_locomotion",
            "window_policy":"refuse_any_window",
            "scene_transcript":"atomic",
            "other_observer_completion":false,
            "personnel_eligibility":false,
        }),
    )
    .map_err(bad)
}

/// A distinct actual participant registration, sharing the existing bounded
/// Sim implementation rather than requesting another type's capability.
struct SceneLocomotionParticipant;
impl OwnerParticipant for SceneLocomotionParticipant {
    type State = SpatialState;
    type Request = LocomotionIntent;
    type Output = LocomotionDraft;
    type Patch = locomotion::Patch;
    type Bound<'a> = locomotion::Bound<'a>;

    fn prepare(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        intent: LocomotionIntent,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        LocomotionParticipant.prepare(state, ctx, intent)
    }

    fn recheck(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        LocomotionParticipant.recheck(state, ctx, patch)
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut SpatialState,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        Arc::new(LocomotionParticipant).bind(state, patch)
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        LocomotionParticipant.accepts_write(write)
    }
}

#[derive(Clone)]
struct SceneLocomotionHandles {
    service: Service,
    crossings: LocomotionCrossings,
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    reader: &OnceLock<scene_transition::SceneLocomotionReader>,
    movement_reads: crate::movement_reads::MovementReads,
) -> Result<(), ModError> {
    let participant = reg.definition(participant_descriptor()?)?;
    let action = reg.definition(scene_transition::action_descriptor()?)?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        SceneLocomotionParticipant,
    )?;
    // These exact references describe required foreign registrations; they do
    // not install a substitute Scenes owner. Core resolves the actual adapter.
    let mut definitions = DefinitionRegistryBuilder::new();
    let scenes = OwnerId::new("scenes").map_err(bad)?;
    let scenes_participant = definitions
        .register(
            scenes.clone(),
            scene_transition::scenes_participant_descriptor()?,
        )
        .map_err(bad)?;
    let observer = definitions
        .register(scenes, scene_transition::observer_descriptor()?)
        .map_err(bad)?;
    let service = Service::register(
        reg,
        action.clone(),
        participant,
        scenes_participant,
        observer,
    )?;
    reader
        .set(service.reader())
        .map_err(|_| bad("scene locomotion reader already belongs to a composition"))?;
    let crossings = LocomotionCrossings::register_scenes(reg)?;
    reg.action_provider(
        action,
        Provider {
            outcome,
            service: service.clone(),
            crossings: crossings.clone(),
            movement_reads,
        },
    )?;
    reg.state(SceneLocomotionHandles { service, crossings });
    Ok(())
}

pub(crate) fn loaded(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let handles = ctx
        .w
        .module_state::<SceneLocomotionHandles>()
        .ok_or_else(|| bad("Sim scene locomotion handles are missing"))?;
    handles.service.audit(ctx).map_err(bad)?;
    handles
        .crossings
        .audit_scenes(ctx, &handles.service)
        .map_err(bad)
}

struct Provider {
    outcome: DefinitionRef,
    service: Service,
    crossings: LocomotionCrossings,
    movement_reads: crate::movement_reads::MovementReads,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        locomotion::validate_invocation(invocation)
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let route = {
            let admission = context
                .admit_captured_locomotion(txn)
                .map_err(|_| unavailable("actual captured locomotion is required"))?;
            ActorMove {
                actor: admission.actor(),
                source: admission.source(),
                exit: admission.exit(),
                destination: admission.destination(),
            }
        };
        let captured = {
            let world = context
                .world()
                .map_err(|_| unavailable("locomotion world unavailable"))?;
            let state =
                crate::state(world).ok_or_else(|| unavailable("spatial state unavailable"))?;
            locomotion::capture(state, world, &route, &world.world_ic_now(), true)
                .map_err(|_| unavailable("locomotion capture unavailable"))?
        };
        crate::movement_reads::retain(
            context,
            txn,
            &self.movement_reads,
            &captured.facts.read_dependencies,
        )?;
        let receipt = context
            .prepare_captured_locomotion_with_scenes(txn, &self.service)
            .map_err(|_| unavailable("captured scene locomotion refused"))?;
        if receipt.actor() != route.actor
            || receipt.source() != route.source
            || receipt.exit() != route.exit
            || receipt.destination() != route.destination
            || receipt.prepared_at() != &captured.facts.prepared_at
            || receipt.plan_digest() != &captured.plan_digest
            || receipt.capture_digest() != &captured.facts.capture_digest
        {
            return Err(unavailable("scene locomotion receipt differs from capture"));
        }
        self.crossings
            .append(txn, &receipt, &captured.facts)
            .map_err(|_| unavailable("scene locomotion crossing could not be recorded"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor":receipt.actor(), "source":receipt.source(), "exit":receipt.exit(),
                "destination":receipt.destination(), "prepared_at":receipt.prepared_at(),
                "plan_digest":receipt.plan_digest(), "capture_digest":receipt.capture_digest(),
                "scenes_capture_digest":receipt.scenes_capture_digest(),
            }),
        )
        .map_err(|_| unavailable("scene locomotion outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
#[path = "locomotion_scenes_tests.rs"]
mod tests;
