//! Opt-in composed movement. Only the composition supplies observer adapters;
//! this module never depends on Atlas or invents a completion owner.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::locomotion::composed::{self, ComposedLocomotion, ComposedLocomotionBuilder};
use mush_core::locomotion::transition::{LocomotionDraft, LocomotionIntent};
use mush_core::participant::{
    OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    ModError, ModuleCtx, PrepareCtx, Registrar, ReplayWrite, RulesValue,
};
use serde_json::json;

use crate::data::{RECOVERY_LOG, SpatialState};
use crate::locomotion::{self, LocomotionParticipant};
use crate::locomotion_capture::{ActorMove, LocomotionCrossings};

/// Called during actual Sim registration, before composition freeze. The
/// returned service must retain the supplied primary and actual owner handles.
pub type ComposedMovementFactory = Arc<
    dyn Fn(&mut Registrar<'_>, ComposedLocomotionBuilder) -> Result<ComposedLocomotion, ModError>
        + Send
        + Sync,
>;

const OUTCOME: &str = "composed_locomotion_result";
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn unavailable(message: &'static str) -> ActionError {
    crate::tick::err(ActionErrorCode::Unavailable, message)
}

pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(bad)?, DefinitionId::new(OUTCOME).map_err(bad)?, 1,
        json!({"fields":["actor","source","exit","destination","prepared_at","plan_digest","capture_digest","manifest_digest"],
            "capture":"sim.composed_locomotion_crossings","completion":"exact_registered_subset",
            "personnel_eligibility":false})).map(DefinitionDescriptor::as_current).map_err(bad)
}
pub(crate) fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?, DefinitionId::new(composed::SIM_PARTICIPANT).map_err(bad)?, 1,
        json!({"state":"SpatialState","documents":[],"ledger":RECOVERY_LOG,
            "operations":["captured_exit","captured_home"],"protocol":"sim_composed_locomotion_v1",
            "capture":"sim.composed_locomotion_crossings","authority":"core_captured_locomotion",
            "window_policy":"refuse_any_window","completion":"exact_registered_subset","personnel_eligibility":false})).map_err(bad)
}

struct ComposedLocomotionParticipant;
impl OwnerParticipant for ComposedLocomotionParticipant {
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
struct Handles {
    service: ComposedLocomotion,
    crossings: LocomotionCrossings,
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    factory: &ComposedMovementFactory,
    journey_read: crate::movement_reads::JourneyRead,
) -> Result<(), ModError> {
    let participant = reg.definition(participant_descriptor()?)?;
    let action = reg.definition(composed::action_descriptor()?)?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        ComposedLocomotionParticipant,
    )?;
    let builder = ComposedLocomotionBuilder::new::<ComposedLocomotionParticipant>(
        reg,
        action.clone(),
        participant,
    )?;
    let service = factory(reg, builder)?;
    let crossings = LocomotionCrossings::register_composed(reg)?;
    reg.action_provider(
        action,
        Provider {
            outcome,
            service: service.clone(),
            crossings: crossings.clone(),
            journey_read,
        },
    )?;
    reg.state(Handles { service, crossings });
    Ok(())
}
pub(crate) fn loaded(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let handles = ctx
        .w
        .module_state::<Handles>()
        .ok_or_else(|| bad("Sim composed locomotion handles are missing"))?;
    handles.service.reader().audit(ctx).map_err(bad)?;
    handles
        .crossings
        .audit_composed(ctx, &handles.service.reader())
        .map_err(bad)
}

struct Provider {
    outcome: DefinitionRef,
    service: ComposedLocomotion,
    crossings: LocomotionCrossings,
    journey_read: crate::movement_reads::JourneyRead,
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
            locomotion::capture_composed(state, world, &route, &world.world_ic_now(), true)
                .map_err(|_| unavailable("composed locomotion capture unavailable"))?
        };
        crate::movement_reads::retain_journeys(
            context,
            txn,
            &self.journey_read,
            &captured.facts.read_dependencies,
        )?;
        let receipt = context
            .prepare_composed_locomotion(txn, &self.service)
            .map_err(|_| unavailable("composed locomotion refused"))?;
        if receipt.actor() != route.actor
            || receipt.source() != route.source
            || receipt.exit() != route.exit
            || receipt.destination() != route.destination
            || receipt.prepared_at() != &captured.facts.prepared_at
            || receipt.plan_digest() != &captured.plan_digest
            || receipt.capture_digest() != &captured.facts.capture_digest
        {
            return Err(unavailable(
                "composed locomotion receipt differs from capture",
            ));
        }
        self.crossings
            .append(txn, &receipt, &captured.facts)
            .map_err(|_| unavailable("composed locomotion crossing could not be recorded"))?;
        let outcome = RulesValue::new(self.outcome.clone(), json!({"actor":receipt.actor(),"source":receipt.source(),"exit":receipt.exit(),
            "destination":receipt.destination(),"prepared_at":receipt.prepared_at(),"plan_digest":receipt.plan_digest(),
            "capture_digest":receipt.capture_digest(),"manifest_digest":receipt.manifest_digest()})).map_err(|_| unavailable("composed locomotion outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}
