//! Closed source-free retirement of an actual owned marked working draft.
//! Expired paid rights do not confer authority here and are not a prerequisite.
pub(crate) mod audit;
pub(super) mod entry;
mod provider;
mod reader;
pub(crate) mod records;
#[cfg(test)]
mod tests;

use super::parent_selector::DraftParentSelection;
use super::records::PublicationError;
use crate::EditBase;
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, ModError, ObjId};
use records::{RetiredDraftParent, RetirementOperation};
use serde::{Deserialize, Serialize};
use serde_json::json;
const ACTION: &str = "retire_authored_draft";
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
pub(crate) fn retention_reference() -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            super::rating::registration::commercial_roots_descriptor()?,
        )
        .map_err(error)
}
pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 3], ModError> {
    let retention = retention_reference()?;
    Ok([
        DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,DefinitionId::new(ACTION).map_err(error)?,1,
            json!({"format":1,"protocol":"authored_retirement_v1","operation":"retire","source":false})).map_err(error)?
            .with_references(vec![retention.clone()]),
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(error)?,DefinitionId::new("authored_draft_retired").map_err(error)?,1,
            json!({"format":1})).map_err(error)?,
        DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("authored_retirement_metadata").map_err(error)?,1,
            json!({"format":1,"protocol":"authored_retirement_v1","subject":"actual_actor","source":false,"current_paid_rights":false})).map_err(error)?
            .with_references(vec![retention]),
    ])
}
pub(crate) fn action_reference() -> Result<DefinitionRef, ModError> {
    let [action, _, _] = definitions()?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, action)
        .map_err(error)
}
pub(crate) fn captured_definition() -> Result<DefinitionDescriptor, ModError> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(error)?,
        DefinitionId::new(ACTION).map_err(error)?,
        2,
        json!({"format":2,"protocol":"authored_retirement_v2","operation":"retire","source":false,"selection":"mind_name_edit_base"}),
    ).map_err(error)?.with_references(vec![retention_reference()?]))
}
pub(crate) fn captured_action_reference() -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            captured_definition()?,
        )
        .map_err(error)
}
pub(crate) fn known_action(action: &DefinitionRef) -> Result<bool, ModError> {
    Ok(action == &action_reference()? || action == &captured_action_reference()?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRequest {
    name: String,
    edit: String,
    revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedRequest {
    mind: String,
    name: String,
    edit: String,
    revision: String,
}
struct Request {
    mind: Option<u64>,
    name: String,
    edit: String,
    revision: String,
}
impl Request {
    fn base(&self) -> Result<EditBase, PublicationError> {
        fn number(text: &str) -> Result<u64, PublicationError> {
            let value = text.parse::<u64>().map_err(bad)?;
            if value.to_string() != text {
                return Err(PublicationError::Invalid);
            }
            Ok(value)
        }
        let edit = number(&self.edit)?;
        let revision = if self.revision == "none" {
            None
        } else {
            Some(number(&self.revision)?)
        };
        if revision == Some(0) || revision.is_some_and(|n| n > edit) {
            return Err(PublicationError::Invalid);
        }
        Ok(EditBase { edit, revision })
    }
    fn parse(inv: &ActionInvocation, action: &DefinitionRef) -> Result<Self, PublicationError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let row = if action == &action_reference().map_err(bad)? {
            let LegacyRequest {
                name,
                edit,
                revision,
            } = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            Self {
                mind: None,
                name,
                edit,
                revision,
            }
        } else if action == &captured_action_reference().map_err(bad)? {
            let CapturedRequest {
                mind,
                name,
                edit,
                revision,
            } = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            let id = mind.parse::<u64>().map_err(bad)?;
            if id == 0 || id.to_string() != mind {
                return Err(PublicationError::Invalid);
            }
            Self {
                mind: Some(id),
                name,
                edit,
                revision,
            }
        } else {
            return Err(PublicationError::Invalid);
        };
        if crate::canonical_name(&row.name).map_err(bad)? != row.name {
            return Err(PublicationError::Invalid);
        }
        row.base()?;
        Ok(row)
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct RetirementSubject {
    pub(crate) actor: ObjId,
    pub(crate) mind: u64,
    pub(crate) name: String,
    pub(crate) base: EditBase,
    pub(crate) selector: DraftParentSelection,
}
pub(crate) struct Retirement {
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    pub(crate) subject: RetirementSubject,
}
impl Retirement {
    pub(crate) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn fence(&self) -> RetirementFence {
        RetirementFence {
            action: self.action.clone(),
            commit: self.commit,
            actor: self.subject.actor,
            mind: self.subject.mind,
            name: self.subject.name.clone(),
            base: self.subject.base,
        }
    }
    pub(crate) fn check_context(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        self.fence().check(ctx)
    }
}
#[derive(Clone)]
pub(crate) struct RetirementFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    mind: u64,
    name: String,
    base: EditBase,
}
impl RetirementFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let request = Request::parse(ctx.invocation(), ctx.action())?;
        if self.action != *ctx.action()
            || !known_action(&self.action).map_err(bad)?
            || self.commit != ctx.commit_token()
            || self.actor != ctx.invocation().actor()
            || request.name != self.name
            || request.mind.is_some_and(|id| id != self.mind)
            || request.base() != Ok(self.base)
            || ctx.world().get(self.actor).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
pub(crate) struct RetirementOutput {
    pub(crate) operation: RetirementOperation,
    pub(crate) tombstone: RetiredDraftParent,
}
pub(crate) use provider::register;
