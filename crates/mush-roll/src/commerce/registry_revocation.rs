//! Registry sanction of one accepted licence term. Old evidence and captured
//! sorties remain immutable; a new term needs normal fresh admission.
pub(crate) mod audit;
mod authority;
pub(crate) mod current;
mod history;
pub(crate) mod notice;
mod policy;
mod provider;
pub(crate) mod records;
use super::{
    LicenceId,
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
pub use policy::RegistryRevocationPolicy;
use serde::{Deserialize, Serialize};
use serde_json::json;
pub const REGISTRY_REVOCATION_CAPABILITY: &str = "roll.registry.revoke";
const CAPABILITY: &str = REGISTRY_REVOCATION_CAPABILITY;
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    licence: String,
    reason: records::Reason,
}
impl Request {
    fn parse(
        inv: &ActionInvocation,
        action: &DefinitionRef,
        policy: &RegistryRevocationPolicy,
    ) -> Result<LicenceId, PublicationError> {
        if action != &policy.action().map_err(bad)?
            || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let r: Self = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        let id = r.licence.parse::<u64>().map_err(bad)?;
        if id.to_string() != r.licence || r.reason != records::Reason::RegistryReview {
            return Err(PublicationError::Invalid);
        }
        LicenceId::new(id).map_err(bad)
    }
}
pub(crate) struct Revocation {
    operation: records::RevocationOperation,
    fence: RevocationFence,
    pending: Option<super::charter_pending::publication::PendingRoots>,
}
impl Revocation {
    pub(crate) fn pending(&self) -> Option<&super::charter_pending::publication::PendingRoots> {
        self.pending.as_ref()
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn fence(&self) -> &RevocationFence {
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
pub(crate) struct RevocationFence {
    operation: records::RevocationOperation,
    readiness: authority::Readiness,
}
impl RevocationFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let op = &self.operation;
        if ctx.action() != &op.action
            || ctx.commit_token() != op.operation
            || ctx.invocation().actor() != op.actor
            || Request::parse(ctx.invocation(), ctx.action(), &op.authority)? != op.term.licence()
            || !matches!(
                op.term.window().phase_at(
                    super::WorldIcSeconds::from_stamp(&ctx.world().world_ic_now()).map_err(bad)?
                ),
                super::TermPhase::Active | super::TermPhase::Grace
            )
        {
            return Err(PublicationError::Stale);
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
