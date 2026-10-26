//! Trusted Industry delivery facts; no caller supplies World postimages.
use crate::core_output::{CoreOutputOrder, OutputRecheckContext};
use crate::mechanics::DefinitionRef;
use crate::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use crate::{CommitToken, ObjId};
use serde::Serialize;
use std::any::Any;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeliveryRequest {
    pub(crate) order: u64,
}
impl DeliveryRequest {
    pub const fn order(&self) -> u64 {
        self.order
    }
}

/// Produced only through the configured actual Industry owner service. These
/// ordinary facts have no decoder and confer no authority outside that call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeliveryFacts {
    generation: u64,
    order: CoreOutputOrder,
    object: ObjId,
    reservation: CommitToken,
    completion: CommitToken,
}
impl DeliveryFacts {
    pub fn new(
        generation: u64,
        order: CoreOutputOrder,
        object: ObjId,
        reservation: CommitToken,
        completion: CommitToken,
    ) -> Self {
        Self {
            generation,
            order,
            object,
            reservation,
            completion,
        }
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub fn order(&self) -> &CoreOutputOrder {
        &self.order
    }
    pub const fn object(&self) -> ObjId {
        self.object
    }
    pub const fn reservation(&self) -> CommitToken {
        self.reservation
    }
    pub const fn completion(&self) -> CommitToken {
        self.completion
    }
}

/// A concrete trusted owner implementation must prove the actual completed
/// Industry order and native paid/title history, actual ordinary Container
/// custody, current requester/title/receiving eligibility and dependencies.
/// Recheck repeats every mutable authority fact before custody. Generation is
/// the local post-ack fence only; it must not call an external service.
pub trait CoreDeliveryPolicy: Send + Sync + 'static {
    type State: Any + Send;
    fn generation(&self, state: &Self::State) -> u64;
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: &DeliveryRequest,
    ) -> Result<DeliveryFacts, OwnerReadError>;
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        request: &DeliveryRequest,
        facts: &DeliveryFacts,
    ) -> Result<(), OwnerReadError>;
}
pub(crate) struct DeliveryReader<D: CoreDeliveryPolicy> {
    pub(crate) policy: Arc<D>,
}
impl<D: CoreDeliveryPolicy> OwnerReadService for DeliveryReader<D> {
    type State = D::State;
    type Request = DeliveryRequest;
    type Output = DeliveryFacts;
    fn read(
        &self,
        state: &D::State,
        ctx: &OwnerReadContext<'_>,
        request: DeliveryRequest,
    ) -> Result<DeliveryFacts, OwnerReadError> {
        if ctx.requester().as_str() != "industry" {
            return Err(OwnerReadError::Authority);
        }
        let facts = self.policy.read(state, ctx, &request)?;
        facts.order.validate()?;
        if facts.generation != self.policy.generation(state)
            || facts.order.id() != request.order
            || facts.order.requester() != ctx.actor()
            || facts.reservation == facts.completion
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(facts)
    }
}

pub(crate) struct PolicyFence<D: CoreDeliveryPolicy> {
    pub(crate) policy: Arc<D>,
    pub(crate) actor: ObjId,
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    pub(crate) request: DeliveryRequest,
    pub(crate) facts: DeliveryFacts,
}
impl<D: CoreDeliveryPolicy> PolicyFence<D> {
    fn state<'a>(
        &self,
        world: &'a crate::World,
    ) -> Result<&'a D::State, crate::participant::ParticipantError> {
        use crate::participant::ParticipantError;
        if world
            .module_state_owner(std::any::TypeId::of::<D::State>())
            .is_none_or(|o| o.as_str() != "industry")
        {
            return Err(ParticipantError::Authority);
        }
        world
            .module_state::<D::State>()
            .ok_or(ParticipantError::Unavailable)
    }
}
impl<D: CoreDeliveryPolicy> super::DeliveryFence for PolicyFence<D> {
    fn recheck(&self, world: &crate::World) -> Result<(), crate::participant::ParticipantError> {
        use crate::participant::ParticipantError;
        self.local(world)?;
        self.policy
            .recheck(
                self.state(world)?,
                &OutputRecheckContext {
                    world,
                    actor: self.actor,
                    action: &self.action,
                    commit: self.commit,
                },
                &self.request,
                &self.facts,
            )
            .map_err(|_| ParticipantError::Authority)?;
        self.local(world)
    }
    fn local(&self, world: &crate::World) -> Result<(), crate::participant::ParticipantError> {
        if self.policy.generation(self.state(world)?) != self.facts.generation() {
            return Err(crate::participant::ParticipantError::Authority);
        }
        Ok(())
    }
}
