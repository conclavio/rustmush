use super::{authority, config::CoreFactoryConfig, families::Families, state::OrderView};
use crate::{
    IndustryFamilies,
    core_production::{CoreOriginMode, authority::Definitions, participant::CoreProductionState},
};
use mush_core::{
    GOD, ObjId, ReservationState, World,
    core_output::*,
    read_service::{OwnerReadContext, OwnerReadError},
};
#[derive(Clone)]
pub(crate) struct Policy {
    pub(crate) config: CoreFactoryConfig,
    pub(crate) definitions: Definitions,
    pub(crate) families: Families,
    pub(crate) industry: IndustryFamilies,
}
impl Policy {
    fn facts(
        &self,
        state: &CoreProductionState,
        world: &World,
        actor: ObjId,
        request: &OutputPolicyRequest,
    ) -> Result<OutputPolicyFacts, OwnerReadError> {
        let projection = state
            .integration
            .as_ref()
            .filter(|p| p.ready)
            .ok_or(OwnerReadError::Unavailable)?;
        if !state.ready {
            return Err(OwnerReadError::Unavailable);
        }
        match request.phase() {
            OutputPhase::Initialize => {
                if self.config.mode != CoreOriginMode::Initialize
                    || actor != GOD
                    || projection.index.is_some()
                    || !projection.offers.is_empty()
                    || !projection.orders.is_empty()
                {
                    return Err(OwnerReadError::Unavailable);
                }
                authority::cleanup(world, &self.definitions)
                    .map_err(|_| OwnerReadError::Unavailable)?;
                Ok(OutputPolicyFacts::empty(state.generation))
            }
            OutputPhase::Reserve | OutputPhase::Complete => {
                let id = crate::workorder::WorkOrderId(
                    request.order().ok_or(OwnerReadError::Integrity)?,
                );
                let order = projection
                    .orders
                    .get(&id)
                    .ok_or(OwnerReadError::Unavailable)?;
                if self.config.mode != CoreOriginMode::Active
                    || projection.index.is_none()
                    || order.intent.requester != actor
                    || order.terminal.is_some()
                    || (request.phase() == OutputPhase::Complete
                        && !order.progress.production().is_complete())
                {
                    return Err(OwnerReadError::Unavailable);
                }
                authority::complete(world, &self.definitions, &self.config, &order.intent)
                    .map_err(|_| OwnerReadError::Unavailable)?;
                Ok(OutputPolicyFacts::order(
                    state.generation,
                    order
                        .intent
                        .output_order()
                        .map_err(|_| OwnerReadError::Integrity)?,
                ))
            }
        }
    }
}
impl CoreOutputPolicy for Policy {
    type State = CoreProductionState;
    fn generation(&self, state: &Self::State) -> u64 {
        state.generation
    }
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: &OutputPolicyRequest,
    ) -> Result<OutputPolicyFacts, OwnerReadError> {
        let facts = self.facts(state, ctx.world(), ctx.actor(), request)?;
        if let Some(id) = request.order() {
            let id = crate::workorder::WorkOrderId(id);
            let row = ctx
                .action_state(&self.families.orders, &id)?
                .ok_or(OwnerReadError::Integrity)?;
            row.validate().map_err(|_| OwnerReadError::Integrity)?;
            if state
                .integration
                .as_ref()
                .and_then(|p| p.orders.get(&id))
                .is_none_or(|p| p.as_ref() != &OrderView::capture(row))
            {
                return Err(OwnerReadError::Integrity);
            }
            let key = crate::facility::facility_capacity_key(row.intent.policy.factory)
                .map_err(|_| OwnerReadError::Integrity)?;
            if ctx.action_state(&self.industry.capacity_index, &row.capacity)? != Some(&key) {
                return Err(OwnerReadError::Integrity);
            }
            let book = ctx
                .action_state(&self.industry.facility_books, &key)?
                .ok_or(OwnerReadError::Integrity)?;
            let record = book
                .records()
                .iter()
                .find(|r| r.id() == row.capacity)
                .ok_or(OwnerReadError::Integrity)?;
            if record.state() != ReservationState::Held
                || record.expires() != &row.intent.expires_at
            {
                return Err(OwnerReadError::Unavailable);
            }
        }
        Ok(facts)
    }
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        request: &OutputPolicyRequest,
        facts: &OutputPolicyFacts,
    ) -> Result<(), OwnerReadError> {
        let name = match request.phase() {
            OutputPhase::Initialize => super::INITIALIZE,
            OutputPhase::Reserve => super::RESERVE,
            OutputPhase::Complete => super::COMPLETE,
        };
        if ctx.action()
            != &self
                .config
                .action(name)
                .map_err(|_| OwnerReadError::Integrity)?
            || &self.facts(state, ctx.world(), ctx.actor(), request)? != facts
        {
            return Err(OwnerReadError::Unavailable);
        }
        Ok(())
    }
}
