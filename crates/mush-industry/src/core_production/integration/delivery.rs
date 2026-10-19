//! Closed delivery history and actual owner policy. The Core adapter owns every
//! physical postimage and captures the real carry hooks independently.
pub(crate) mod audit;
use super::{
    IntegrationError as Error, authority, bad,
    config::CoreFactoryConfig,
    families::Families,
    records::{self, CoreIntegrationOrder, MAX_ORDERS},
    state::OrderView,
};
use crate::{
    core_production::{CoreOriginMode, authority::Definitions, participant::CoreProductionState},
    workorder::WorkOrderId,
};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, ObjId, World,
    core_delivery::{CoreDeliveryPolicy, DeliveredCoreOutput, DeliveryFacts, DeliveryRequest},
    core_output::OutputRecheckContext,
    read_service::{OwnerReadContext, OwnerReadError},
};
use mush_economy::core_integration::{CoreDeliveryPromise, CoreTerminalResult};
use serde::{Deserialize, Serialize};

pub(crate) const ACTION: &str = "deliver_core_output";
pub(crate) const FAMILY: &str = "industry_core_deliveries";
pub(crate) const BYTES: usize = 4096;
pub(crate) const INDEX_KEY: &str = "core_delivery_registry";
pub(crate) const INDEX_BYTES: usize = 4096;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) order: WorkOrderId,
    pub(crate) accepted: CommitToken,
    pub(crate) completion: CommitToken,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) object: ObjId,
    pub(crate) reservation: CommitToken,
    pub(crate) at_utc: i64,
    pub(crate) core_receipt: DefinitionDigest,
}
impl Record {
    pub(crate) fn capture(
        sequence: u64,
        order: &CoreIntegrationOrder,
        output: &DeliveredCoreOutput,
    ) -> Result<Self, Error> {
        let r = Self {
            format: 1,
            sequence,
            order: order.intent.order,
            accepted: order.accepted,
            completion: output.completion(),
            operation: output.commit(),
            action: output.action().clone(),
            object: output.object(),
            reservation: output.reservation(),
            at_utc: output.delivered_at(),
            core_receipt: output.digest().map_err(bad)?,
        };
        r.matches_order(order)?;
        r.matches_core(output, order)?;
        Ok(r)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || !super::super::fixed(&self.action, "action", ACTION)
            || self.core_receipt.as_bytes() == &[0; 32]
            || self.operation == self.accepted
            || self.operation == self.completion
            || self.operation == self.reservation
        {
            return Err(Error::Integrity);
        }
        records::encode(self, BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn matches_order(&self, order: &CoreIntegrationOrder) -> Result<(), Error> {
        self.validate()?;
        order.validate()?;
        let terminal = order.terminal.as_ref().ok_or(Error::Integrity)?;
        let CoreTerminalResult::Completed { output, .. } = terminal.result() else {
            return Err(Error::Integrity);
        };
        if order.intent.policy.delivery != Some(CoreDeliveryPromise::PersonalCustodyV1)
            || self.order != order.intent.order
            || self.accepted != order.accepted
            || self.completion != terminal.commit()
            || self.action != order.intent.policy.action_ref(ACTION).map_err(bad)?
            || self.object != output.object
            || self.reservation != output.reservation
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn matches_core(
        &self,
        output: &DeliveredCoreOutput,
        order: &CoreIntegrationOrder,
    ) -> Result<(), Error> {
        self.matches_order(order)?;
        if output.order() != &order.intent.output_order()?
            || output.object() != self.object
            || output.reservation() != self.reservation
            || output.completion() != self.completion
            || output.commit() != self.operation
            || output.action() != &self.action
            || output.delivered_at() != self.at_utc
            || output.digest().map_err(bad)? != self.core_receipt
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
/// This independent owner document is initialized before any promised payment.
/// Its fixed bounded hash-chain detects loss of both typed delivery halves.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Index {
    format: u16,
    initialized: CommitToken,
    pub(crate) delivered: u64,
    chain: DefinitionDigest,
}
impl Index {
    pub(crate) fn new(initialized: CommitToken) -> Result<Self, Error> {
        Ok(Self {
            format: 1,
            initialized,
            delivered: 0,
            chain: records::digest(
                "industry.core_delivery_genesis.v1",
                &initialized,
                INDEX_BYTES,
            )?,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1 || self.delivered > MAX_ORDERS || self.chain.as_bytes() == &[0; 32] {
            return Err(Error::Integrity);
        }
        records::encode(self, INDEX_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn append(&self, row: &Record) -> Result<Self, Error> {
        self.validate()?;
        row.validate()?;
        if self.delivered.checked_add(1) != Some(row.sequence) {
            return Err(Error::Integrity);
        }
        let next = Self {
            format: 1,
            initialized: self.initialized,
            delivered: row.sequence,
            chain: records::digest(
                "industry.core_delivery_chain.v1",
                &serde_json::json!({"prior":self.chain,"receipt":row}),
                INDEX_BYTES,
            )?,
        };
        next.validate()?;
        Ok(next)
    }
    pub(crate) fn genesis(&self) -> Result<Self, Error> {
        Self::new(self.initialized)
    }
    pub(crate) fn initialized(&self) -> CommitToken {
        self.initialized
    }
}

#[derive(Clone)]
pub(crate) struct Policy {
    pub(crate) config: CoreFactoryConfig,
    pub(crate) definitions: Definitions,
    pub(crate) families: Families,
}
impl Policy {
    fn facts(
        &self,
        state: &CoreProductionState,
        world: &World,
        actor: ObjId,
        order: WorkOrderId,
    ) -> Result<DeliveryFacts, OwnerReadError> {
        let p = state
            .integration
            .as_ref()
            .filter(|p| p.ready)
            .ok_or(OwnerReadError::Unavailable)?;
        if !state.ready
            || self.config.mode != CoreOriginMode::Active
            || self.config.payment.delivery.is_none()
            || p.index.is_none()
            || p.delivery_index.is_none()
            || p.deliveries.contains_key(&order)
        {
            return Err(OwnerReadError::Unavailable);
        }
        authority::cleanup(world, &self.definitions).map_err(|_| OwnerReadError::Unavailable)?;
        let row = p.orders.get(&order).ok_or(OwnerReadError::Unavailable)?;
        let h = mush_economy::core_integration::require_core_delivery(world, row.accepted)
            .map_err(|_| OwnerReadError::Unavailable)?;
        let t = h.terminal().ok_or(OwnerReadError::Integrity)?;
        let CoreTerminalResult::Completed { output, .. } = t.result() else {
            return Err(OwnerReadError::Unavailable);
        };
        if row.intent.requester != actor
            || h.subject()
                != &row
                    .intent
                    .subject()
                    .map_err(|_| OwnerReadError::Integrity)?
            || h.policy() != &row.intent.policy
            || row.terminal.as_ref() != Some(t)
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(DeliveryFacts::new(
            state.generation,
            row.intent
                .output_order()
                .map_err(|_| OwnerReadError::Integrity)?,
            output.object,
            output.reservation,
            t.commit(),
        ))
    }
}
impl CoreDeliveryPolicy for Policy {
    type State = CoreProductionState;
    fn generation(&self, state: &Self::State) -> u64 {
        state.generation
    }
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: &DeliveryRequest,
    ) -> Result<DeliveryFacts, OwnerReadError> {
        let id = WorkOrderId(request.order());
        let facts = self.facts(state, ctx.world(), ctx.actor(), id)?;
        let row = ctx
            .action_state(&self.families.orders, &id)?
            .ok_or(OwnerReadError::Integrity)?;
        row.validate().map_err(|_| OwnerReadError::Integrity)?;
        if state
            .integration
            .as_ref()
            .and_then(|p| p.orders.get(&id))
            .is_none_or(|v| v.as_ref() != &OrderView::capture(row))
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(facts)
    }
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        request: &DeliveryRequest,
        facts: &DeliveryFacts,
    ) -> Result<(), OwnerReadError> {
        if ctx.action()
            != &self
                .config
                .action(ACTION)
                .map_err(|_| OwnerReadError::Integrity)?
            || &self.facts(
                state,
                ctx.world(),
                ctx.actor(),
                WorkOrderId(request.order()),
            )? != facts
        {
            return Err(OwnerReadError::Unavailable);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) action: DefinitionRef,
    pub(crate) records: mush_core::StateFamilyHandle<WorkOrderId, Record>,
    pub(crate) outputs:
        mush_core::core_delivery::CoreOutputDeliveries<super::output_policy::Policy, Policy>,
}
impl Runtime {
    pub(crate) fn register(
        reg: &mut mush_core::Registrar<'_>,
        config: &CoreFactoryConfig,
        definitions: &Definitions,
        families: &Families,
        outputs: &mush_core::core_output::CoreOutputReservations<super::output_policy::Policy>,
    ) -> Result<Self, mush_core::ModError> {
        let err = |e: Error| mush_core::ModError::msg(e.to_string());
        let action = reg.definition(
            config
                .payment
                .action_descriptor(ACTION)
                .map_err(bad)
                .map_err(err)?,
        )?;
        let records = reg.state_family(
            super::families::descriptor(FAMILY, BYTES)?
                .with_limits(16, BYTES)
                .map_err(|e| mush_core::ModError::msg(e.to_string()))?,
            |key: &WorkOrderId| Ok(format!("{:016x}", key.0)),
            |value: &serde_json::Value| records::decode::<Record>(value, BYTES),
            |row: &Record| row.validate().map_err(|e| e.to_string()),
            |row: &Record| records::encode(row, BYTES),
        )?;
        let dependencies = ["industry", "economy", "inventory"]
            .into_iter()
            .map(|s| {
                mush_core::OwnerId::new(s).map_err(|e| mush_core::ModError::msg(e.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let outputs = mush_core::core_delivery::CoreOutputDeliveries::register(
            reg,
            outputs.clone(),
            action.clone(),
            Policy {
                config: config.clone(),
                definitions: definitions.clone(),
                families: families.clone(),
            },
            dependencies,
        )?;
        Ok(Self {
            action,
            records,
            outputs,
        })
    }
}
