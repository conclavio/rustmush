//! Exact filed-revision Registry recall. Shared rating and captured seals stay immutable.
pub(crate) mod audit;
mod authority;
pub(crate) mod notice;
mod policy;
mod provider;
pub(crate) mod records;
use super::{
    FiledRevisionId,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ModError, ObjId};
pub use policy::RegistryRecallPolicy;
use serde::{Deserialize, Serialize};
use serde_json::json;
/// Capability checked by the actual Org authorizer; declaring its vocabulary
/// does not grant it to any actor.
pub const REGISTRY_RECALL_CAPABILITY: &str = "roll.registry.recall";
const CAPABILITY: &str = REGISTRY_RECALL_CAPABILITY;
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    filed: String,
    reason: records::Reason,
}
impl Request {
    fn parse(
        inv: &ActionInvocation,
        action: &DefinitionRef,
        policy: &RegistryRecallPolicy,
    ) -> Result<FiledRevisionId, PublicationError> {
        if action != &policy.action().map_err(bad)?
            || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let request: Self = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        let value = request.filed.parse::<u64>().map_err(bad)?;
        if value.to_string() != request.filed || request.reason != records::Reason::RegistryReview {
            return Err(PublicationError::Invalid);
        }
        FiledRevisionId::new(value).map_err(bad)
    }
}
pub(crate) struct Recall {
    operation: records::RecallOperation,
    fence: RecallFence,
}
impl Recall {
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn fence(&self) -> &RecallFence {
        &self.fence
    }
    pub(crate) fn filed_root(&self) -> RequiredRoot {
        RequiredRoot {
            anchor: RootAnchor::Filed(self.operation.filed),
            identity: self.operation.revision,
        }
    }
    pub(crate) fn cause_root(&self) -> Result<RequiredRoot, PublicationError> {
        self.operation.root()
    }
}
#[derive(Clone)]
pub(crate) struct RecallFence {
    operation: records::RecallOperation,
    readiness: authority::Readiness,
}
impl RecallFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let op = &self.operation;
        if ctx.action() != &op.action
            || ctx.commit_token() != op.operation
            || ctx.invocation().actor() != op.actor
            || Request::parse(ctx.invocation(), ctx.action(), &op.authority)? != op.filed
        {
            return Err(PublicationError::Mismatch);
        }
        authority::recheck(
            ctx.world(),
            &self.readiness,
            &op.authority,
            op.actor,
            &op.authorization,
            &op.at_ic,
        )
    }
}
pub(crate) use authority::Readiness;
pub(crate) use provider::register;

#[cfg(test)]
mod tests;
