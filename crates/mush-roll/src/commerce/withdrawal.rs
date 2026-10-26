//! Personal author's permanent metadata withdrawal. No source or recall power.
pub(crate) mod audit;
mod provider;
pub(crate) mod records;
#[cfg(test)]
mod tests;

use super::records::{PublicationError, Record};
use super::roots::{RequiredRoot, RootAnchor};
use super::{CharterId, FiledRevisionId};
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKey, DefinitionKind, DefinitionLocator,
    DefinitionRegistry, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, Flags, ModError, ObjId, ObjKind, World};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::any::TypeId;
use std::sync::{Arc, OnceLock};

/// Only actual final composition can fill this private process-local cell.
/// The registry clone retains the real shared mechanical-readiness latch;
/// it is not a cached health flag or a title/source permission.
#[derive(Clone, Default)]
pub(crate) struct EconomyReadiness(Arc<OnceLock<(DefinitionRegistry, DefinitionRef)>>);
impl EconomyReadiness {
    pub(crate) fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        let definitions = ctx
            .reg
            .definitions()
            .ok_or_else(|| error("withdrawal definitions unavailable"))?;
        let locator = DefinitionLocator::new(
            DefinitionKey {
                owner: OwnerId::new("economy").map_err(error)?,
                kind: DefinitionKind::new("owner_participant").map_err(error)?,
                id: DefinitionId::new("initial_filing").map_err(error)?,
            },
            1,
        )
        .map_err(error)?;
        let exact = definitions.resolve_locator(&locator).map_err(error)?;
        definitions.get_exact(&exact).map_err(error)?;
        self.0
            .set((definitions.clone(), exact))
            .map_err(|_| error("withdrawal runtime already captured"))
    }
    pub(crate) fn check(&self) -> Result<(), PublicationError> {
        let (definitions, exact) = self.0.get().ok_or(PublicationError::Unavailable)?;
        definitions
            .get_exact(exact)
            .map_err(|_| PublicationError::Unavailable)?;
        Ok(())
    }
}

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 2], ModError> {
    let retention = super::authored_retirement::retention_reference()?;
    Ok([
        DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?, DefinitionId::new("withdraw_authored_mind").map_err(error)?, 1,
            json!({"format":1,"protocol":"personal_author_withdrawal_v1","authority":"initial_author_and_current_personal_title","recall":false}))
            .map_err(error)?.with_references(vec![retention.clone()]),
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(error)?, DefinitionId::new("authored_mind_withdrawn").map_err(error)?, 1, json!({"format":1})).map_err(error)?,
    ])
}
pub(crate) fn action_reference() -> Result<DefinitionRef, ModError> {
    let [action, _] = definitions()?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, action)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    filed: String,
}
impl Request {
    fn parse(
        inv: &ActionInvocation,
        action: &DefinitionRef,
    ) -> Result<FiledRevisionId, PublicationError> {
        if action != &action_reference().map_err(bad)?
            || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let request: Self = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        let value = request.filed.parse::<u64>().map_err(bad)?;
        if value.to_string() != request.filed {
            return Err(PublicationError::Invalid);
        }
        FiledRevisionId::new(value).map_err(bad)
    }
}

/// Checks current title, never entitlement/source eligibility. This producer
/// intentionally excludes houses and a new charter holder who is not author.
fn current_title(world: &World, actor: ObjId, charter: CharterId) -> Result<u64, PublicationError> {
    if world
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world
            .get(charter.object())
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        || world
            .module_state_owner(TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let title = economy
        .snapshot()
        .asset(charter.object())
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen()
        || title.object != charter.object()
        || title.revision == 0
        || title.title_owner != mush_org::PrincipalId::Object(actor)
        || title.scope.is_some()
        || title.frozen
        || title.stolen.is_some()
        || title.lifecycle != mush_economy::AssetLifecycle::Active
    {
        return Err(PublicationError::Unavailable);
    }
    Ok(title.revision)
}

#[derive(Debug, Serialize)]
pub(super) struct Subject {
    actor: ObjId,
    title_revision: u64,
    initial: super::records::InitialFilingOperation,
    filed: super::records::FiledRecord,
    status: super::records::RevisionStatus,
}

/// Minted only after actual owner preimage read and exact typed before-images.
pub(crate) struct Withdrawal {
    pub(super) operation: records::WithdrawalOperation,
    fence: WithdrawalFence,
}
impl Withdrawal {
    fn new(
        action: &DefinitionRef,
        commit: CommitToken,
        subject: &Subject,
        readiness: EconomyReadiness,
    ) -> Result<Self, PublicationError> {
        let operation = records::WithdrawalOperation {
            format: 1,
            operation: commit,
            action: action.clone(),
            author: subject.actor,
            initial_filing: subject.initial.commit,
            filed: subject.filed.content.id(),
            charter: subject.initial.charter,
            line: subject.initial.line,
            revision: subject.filed.content.revision(),
            title_revision: subject.title_revision,
        };
        operation.matches_initial(&subject.initial, &subject.filed)?;
        if subject.status.filed != operation.filed
            || subject.status.withdrawn_by.is_some()
            || subject.status.recalled_by.is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            fence: WithdrawalFence {
                action: action.clone(),
                commit,
                actor: subject.actor,
                filed: operation.filed,
                charter: operation.charter,
                title_revision: subject.title_revision,
                readiness,
            },
            operation,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn fence(&self) -> &WithdrawalFence {
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
pub(crate) struct WithdrawalFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    filed: FiledRevisionId,
    charter: CharterId,
    title_revision: u64,
    readiness: EconomyReadiness,
}
impl WithdrawalFence {
    pub(crate) fn check(&self, context: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        self.readiness.check()?;
        if self.action != *context.action()
            || self.commit != context.commit_token()
            || self.actor != context.invocation().actor()
            || Request::parse(context.invocation(), context.action())? != self.filed
            || current_title(context.world(), self.actor, self.charter)? != self.title_revision
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
pub(crate) use provider::register;
