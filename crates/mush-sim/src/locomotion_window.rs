//! Closed window producer and actual Sim-owned history handles. No live route
//! or offer is registered; incomplete observer coverage stays explicit.

use crate::window_movement::{
    WINDOW_MOVEMENT_LOG, WindowMovementHistory, WindowMovementParticipant, WindowMovementRecord,
    WindowSelector, capture_ordered,
};
use mush_core::locomotion::composed::{self, ComposedLocomotion, ComposedLocomotionBuilder};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    ModError, ModuleBootData, ModuleCtx, PrepareCtx, Registrar, RulesValue,
};
use serde_json::json;
use std::collections::BTreeSet;

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn unavailable() -> ActionError {
    crate::tick::err(
        ActionErrorCode::Unavailable,
        "closed window locomotion is unavailable or changed",
    )
}

pub(crate) fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    // Current is an authoring default, independent of exact definition identity.
    // Keep v1 current while this separately selected v2 profile is incomplete.
    let descriptor = composed::window_action_descriptor()?;
    DefinitionDescriptor::new(
        descriptor.kind().clone(),
        descriptor.id().clone(),
        descriptor.version(),
        descriptor.payload().clone(),
    )
    .map(|d| d.with_references(descriptor.references().to_vec()))
    .map_err(bad)
}
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(bad)?,
        DefinitionId::new("window_locomotion_result").map_err(bad)?,
        1,
        json!({"fields":["actor","source","exit","admitted_room","projected_room","prepared_at",
            "plan_digest","capture_digest","manifest_digest"],
            "capture":"sim.window_locomotion_crossings","completion":"exact_registered_subset",
            "history_commit_limit":crate::window_movement::MAX_WINDOW_MOVEMENT_COMMITS,
            "personnel_eligibility":false}),
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(bad)
}

#[derive(Clone)]
struct Handles {
    service: ComposedLocomotion,
    history: WindowMovementHistory,
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    selector: WindowSelector,
    factory: &crate::ComposedMovementFactory,
    movement_reads: crate::movement_reads::MovementReads,
) -> Result<(), ModError> {
    let action = reg.definition(action_descriptor()?)?;
    let participant = reg.definition(WindowMovementParticipant::descriptor()?)?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        WindowMovementParticipant::new(selector.clone()),
    )?;
    let builder = ComposedLocomotionBuilder::new_window::<WindowMovementParticipant>(
        reg,
        action.clone(),
        participant,
    )?;
    let service = factory(reg, builder)?;
    let history = WindowMovementHistory::register(reg)?;
    reg.action_provider(
        action,
        Provider {
            selector,
            service: service.clone(),
            history: history.clone(),
            outcome,
            movement_reads,
        },
    )?;
    reg.state(Handles { service, history });
    Ok(())
}
fn handles<'a>(ctx: &'a ModuleCtx<'_>) -> Result<&'a Handles, ModError> {
    ctx.w
        .module_state::<Handles>()
        .ok_or_else(|| bad("window movement handles are missing"))
}
pub(crate) fn audit_typed(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let handles = handles(ctx)?;
    handles
        .history
        .audit_typed(ctx, &handles.service.reader())
        .map_err(bad)
}
pub(crate) fn audit_native(ctx: &ModuleCtx<'_>, data: &ModuleBootData) -> Result<(), ModError> {
    let handles = handles(ctx)?;
    handles
        .history
        .audit_native(ctx, data, &handles.service.reader())
}

struct Provider {
    selector: WindowSelector,
    service: ComposedLocomotion,
    history: WindowMovementHistory,
    outcome: DefinitionRef,
    movement_reads: crate::movement_reads::MovementReads,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        crate::locomotion::validate_invocation(invocation)
    }
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let (actor, source, destination) = {
            let admission = ctx
                .admit_captured_locomotion(txn)
                .map_err(|_| unavailable())?;
            (
                admission.actor(),
                admission.source(),
                admission.destination(),
            )
        };
        let (capture, selection) = {
            let world = ctx.world().map_err(|_| unavailable())?;
            let selection = (self.selector)(world, actor, source, destination, world.now())
                .map_err(|_| unavailable())?;
            let preview = ctx
                .preview_captured_locomotion_projection(&BTreeSet::new(), selection.room())
                .map_err(|_| unavailable())?;
            let capture = capture_ordered(world, preview, selection.native_target())
                .map_err(|_| unavailable())?;
            (capture, selection)
        };
        crate::movement_reads::retain(ctx, txn, &self.movement_reads, capture.read_dependencies())?;
        let receipt = ctx
            .prepare_composed_locomotion(txn, &self.service)
            .map_err(|_| unavailable())?;
        if receipt.window_projection() != Some(&selection.projection(capture.digest()))
            || receipt.plan_digest()
                != &selection
                    .plan_digest(capture.digest())
                    .map_err(|_| unavailable())?
        {
            return Err(unavailable());
        }
        let record = {
            let state = ctx
                .world()
                .map_err(|_| unavailable())?
                .module_state::<crate::SpatialState>()
                .ok_or_else(unavailable)?;
            WindowMovementRecord::for_receipt(
                &receipt,
                &capture,
                state.recovery.next_seq(WINDOW_MOVEMENT_LOG),
            )
            .map_err(|_| unavailable())?
        };
        let append = {
            let state = ctx
                .world()
                .map_err(|_| unavailable())?
                .module_state::<crate::SpatialState>()
                .ok_or_else(unavailable)?;
            WindowMovementParticipant::prepare_journal(state, &record).map_err(|_| unavailable())?
        };
        self.history
            .append(txn, &receipt, &capture, &append)
            .map_err(|_| unavailable())?;
        let projection = receipt.window_projection().ok_or_else(unavailable)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor":receipt.actor(),
            "source":receipt.source(),"exit":receipt.exit(),"admitted_room":receipt.destination(),
            "projected_room":projection.projected_room(),"prepared_at":receipt.prepared_at(),
            "plan_digest":receipt.plan_digest(),"capture_digest":receipt.capture_digest(),
            "manifest_digest":receipt.manifest_digest()}),
        )
        .map_err(|_| unavailable())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}
