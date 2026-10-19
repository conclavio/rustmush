//! Fixed refit facts read from the registered Industry owner, never caller
//! supplied object images. The concrete policy owns title/certificate/custody,
//! facility, current installation index and Tactics compatibility checks.
use super::*;
use crate::core_output::OutputRecheckContext;
use crate::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use std::any::Any;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExchangeRequest {
    pub(crate) order: u64,
}
impl ExchangeRequest {
    pub const fn order(&self) -> u64 {
        self.order
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExchangeFacts {
    pub(super) generation: u64,
    pub(super) order: u64,
    pub(super) requester: ObjId,
    pub(super) hull: ObjId,
    pub(super) unit_revision: u64,
    pub(super) incoming: ObjId,
    pub(super) source: ObjId,
    pub(super) outgoing: Option<ObjId>,
    pub(super) return_to: Option<ObjId>,
}
impl ExchangeFacts {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generation: u64,
        order: u64,
        requester: ObjId,
        hull: ObjId,
        unit_revision: u64,
        incoming: ObjId,
        source: ObjId,
        displaced: Option<(ObjId, ObjId)>,
    ) -> Result<Self, ParticipantError> {
        let (outgoing, return_to) = displaced.map_or((None, None), |(o, d)| (Some(o), Some(d)));
        let facts = Self {
            generation,
            order,
            requester,
            hull,
            unit_revision,
            incoming,
            source,
            outgoing,
            return_to,
        };
        facts.validate()?;
        Ok(facts)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.unit_revision == u64::MAX
            || self.requester == self.hull
            || self.requester == self.incoming
            || self.incoming == self.hull
            || self.incoming == self.source
            || self.source == self.hull
            || self.outgoing.is_some() != self.return_to.is_some()
            || self.outgoing.is_some_and(|o| {
                [self.requester, self.hull, self.incoming, self.source].contains(&o)
            })
            || self
                .return_to
                .is_some_and(|d| d == self.hull || d == self.incoming || Some(d) == self.outgoing)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn order(&self) -> u64 {
        self.order
    }
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    pub const fn hull(&self) -> ObjId {
        self.hull
    }
    pub const fn unit_revision(&self) -> u64 {
        self.unit_revision
    }
    pub const fn incoming(&self) -> ObjId {
        self.incoming
    }
    pub const fn source(&self) -> ObjId {
        self.source
    }
    pub const fn displaced(&self) -> Option<(ObjId, ObjId)> {
        match (self.outgoing, self.return_to) {
            (Some(o), Some(d)) => Some((o, d)),
            _ => None,
        }
    }
}
/// Registered trusted code must repeat all live refit checks in `recheck`.
/// `generation` is the local acknowledgement fence and must not invoke services.
/// The paired Tactics policy must select these same incoming/displaced cores;
/// Core additionally requires both actual Economy and Tactics owner images.
pub trait CoreExchangePolicy: Send + Sync + 'static {
    type State: Any + Send;
    fn generation(&self, state: &Self::State) -> u64;
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: &ExchangeRequest,
    ) -> Result<ExchangeFacts, OwnerReadError>;
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        request: &ExchangeRequest,
        facts: &ExchangeFacts,
    ) -> Result<(), OwnerReadError>;
}
pub(crate) struct ExchangeReader<P: CoreExchangePolicy> {
    pub(crate) policy: Arc<P>,
}
impl<P: CoreExchangePolicy> OwnerReadService for ExchangeReader<P> {
    type State = P::State;
    type Request = ExchangeRequest;
    type Output = ExchangeFacts;
    fn read(
        &self,
        state: &P::State,
        ctx: &OwnerReadContext<'_>,
        request: ExchangeRequest,
    ) -> Result<ExchangeFacts, OwnerReadError> {
        if ctx.requester().as_str() != "industry" {
            return Err(OwnerReadError::Authority);
        }
        let facts = self.policy.read(state, ctx, &request)?;
        facts.validate().map_err(|_| OwnerReadError::Integrity)?;
        if facts.generation != self.policy.generation(state)
            || facts.order != request.order
            || facts.requester != ctx.actor()
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(facts)
    }
}
pub(crate) struct PolicyFence<P: CoreExchangePolicy> {
    pub(crate) policy: Arc<P>,
    pub(crate) actor: ObjId,
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    pub(crate) request: ExchangeRequest,
    pub(crate) facts: ExchangeFacts,
}
impl<P: CoreExchangePolicy> PolicyFence<P> {
    fn state<'a>(&self, world: &'a World) -> Result<&'a P::State, ParticipantError> {
        if world
            .module_state_owner(std::any::TypeId::of::<P::State>())
            .is_none_or(|o| o.as_str() != "industry")
        {
            return Err(refused());
        }
        world
            .module_state::<P::State>()
            .ok_or(ParticipantError::Unavailable)
    }
}
impl<P: CoreExchangePolicy> crate::core_delivery::DeliveryFence for PolicyFence<P> {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
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
            .map_err(|_| refused())?;
        self.local(world)
    }
    fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self.policy.generation(self.state(world)?) != self.facts.generation {
            return Err(refused());
        }
        Ok(())
    }
}
