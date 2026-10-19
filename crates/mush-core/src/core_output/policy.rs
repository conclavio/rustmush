//! Trusted Industry policy for the closed output-reservation service. Facts are
//! read from the actual registered owner; serialized facts are never a capability.
use crate::mechanics::{DefinitionDigest, DefinitionRef};
use crate::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use crate::{CommitToken, ObjId, World};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputPhase {
    Initialize,
    Reserve,
    Complete,
}

/// Core constructs this request from the actual action and closed selector.
/// There is no requester, destination, policy, purpose or postimage parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputPolicyRequest {
    pub(crate) phase: OutputPhase,
    pub(crate) order: Option<u64>,
}
impl OutputPolicyRequest {
    pub const fn phase(&self) -> OutputPhase {
        self.phase
    }
    pub const fn order(&self) -> Option<u64> {
        self.order
    }
}

/// Immutable accepted order identity; mutable work-order state is not a birth proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreOutputOrder {
    id: u64,
    intent: DefinitionDigest,
    requester: ObjId,
    recipe: DefinitionRef,
    facility: ObjId,
    container: ObjId,
}
impl CoreOutputOrder {
    /// For the trusted installed Industry policy only. This value grants no
    /// authority until returned by the actual owner service in this preparation.
    pub fn new(
        id: u64,
        intent: DefinitionDigest,
        requester: ObjId,
        recipe: DefinitionRef,
        facility: ObjId,
        container: ObjId,
    ) -> Result<Self, OwnerReadError> {
        let result = Self {
            id,
            intent,
            requester,
            recipe,
            facility,
            container,
        };
        result.validate()?;
        Ok(result)
    }
    pub const fn id(&self) -> u64 {
        self.id
    }
    pub const fn intent(&self) -> DefinitionDigest {
        self.intent
    }
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    pub const fn recipe(&self) -> &DefinitionRef {
        &self.recipe
    }
    pub const fn facility(&self) -> ObjId {
        self.facility
    }
    pub const fn container(&self) -> ObjId {
        self.container
    }
    pub(crate) fn validate(&self) -> Result<(), OwnerReadError> {
        if self.intent.as_bytes() == &[0; 32]
            || self.requester.0 == 0
            || self.facility.0 == 0
            || self.container.0 == 0
            || self.recipe.key().kind.as_str() != "recipe"
            || self.recipe.key().id.as_str() != "core_integrate"
            || self.recipe.version() != 1
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(())
    }
}

/// Returned only through the real Industry read registration. The exact owner
/// generation and order facts are retained for the policy's final recheck.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputPolicyFacts {
    generation: u64,
    order: Option<CoreOutputOrder>,
}
impl OutputPolicyFacts {
    pub fn empty(generation: u64) -> Self {
        Self {
            generation,
            order: None,
        }
    }
    pub fn order(generation: u64, order: CoreOutputOrder) -> Self {
        Self {
            generation,
            order: Some(order),
        }
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn order_facts(&self) -> Option<&CoreOutputOrder> {
        self.order.as_ref()
    }
}

/// The actual preparation's identity, frozen by Core rather than supplied in a DTO.
pub struct OutputRecheckContext<'a> {
    pub(crate) world: &'a World,
    pub(crate) actor: ObjId,
    pub(crate) action: &'a DefinitionRef,
    pub(crate) commit: CommitToken,
}
impl OutputRecheckContext<'_> {
    pub const fn world(&self) -> &World {
        self.world
    }
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn action(&self) -> &DefinitionRef {
        self.action
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
}

/// One implementation is selected at trusted module construction. It must prove
/// actual order/facility/capacity and requester authority, including exact empty
/// production history for Initialize. It must repeat all mutable World/owner
/// facts at final recheck; an OwnerRead observation alone checks only readiness
/// and slot identity. No method receives mutable World or supplies an Obj image.
pub trait CoreOutputPolicy: Send + Sync + 'static {
    type State: Any + Send;
    /// Read only the owner-local monotonic generation. Used at local binding
    /// after acknowledgement; no World, external service or authority callback.
    fn generation(&self, state: &Self::State) -> u64;
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: &OutputPolicyRequest,
    ) -> Result<OutputPolicyFacts, OwnerReadError>;
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        request: &OutputPolicyRequest,
        facts: &OutputPolicyFacts,
    ) -> Result<(), OwnerReadError>;
}

pub(crate) struct PolicyReader<P: CoreOutputPolicy> {
    pub(crate) policy: Arc<P>,
}
impl<P: CoreOutputPolicy> OwnerReadService for PolicyReader<P> {
    type State = P::State;
    type Request = OutputPolicyRequest;
    type Output = OutputPolicyFacts;
    fn read(
        &self,
        state: &P::State,
        ctx: &OwnerReadContext<'_>,
        request: OutputPolicyRequest,
    ) -> Result<OutputPolicyFacts, OwnerReadError> {
        if ctx.requester().as_str() != "industry" {
            return Err(OwnerReadError::Authority);
        }
        let facts = self.policy.read(state, ctx, &request)?;
        if facts.generation != self.policy.generation(state) {
            return Err(OwnerReadError::Integrity);
        }
        match (&request.phase, &facts.order) {
            (OutputPhase::Initialize, None) if request.order.is_none() => {}
            (OutputPhase::Reserve | OutputPhase::Complete, Some(order))
                if request.order == Some(order.id) && order.requester == ctx.actor() =>
            {
                order.validate()?
            }
            _ => return Err(OwnerReadError::Integrity),
        }
        Ok(facts)
    }
}
