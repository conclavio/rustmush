use super::super::families::CommerceFamilies;
use super::super::records::Record;
use super::*;
use crate::RollStore;
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
#[derive(Serialize)]
pub(super) struct MetadataRequest {
    pub(super) name: String,
    pub(super) mind: Option<u64>,
    pub(super) base: EditBase,
}
pub(super) struct MetadataReader {
    pub(super) families: CommerceFamilies,
}
impl OwnerReadService for MetadataReader {
    type State = RollStore;
    type Request = MetadataRequest;
    type Output = RetirementSubject;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: MetadataRequest,
    ) -> Result<RetirementSubject, OwnerReadError> {
        if ctx.requester().as_str() != crate::NS
            || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
            || ctx.world().get(ctx.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Err(OwnerReadError::Authority);
        }
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        let mind = roll
            .mind(ctx.actor(), &request.name)
            .map_err(|_| OwnerReadError::Refused)?;
        if mind.name() != request.name
            || mind.base() != request.base
            || request.mind.is_some_and(|id| id != mind.id())
        {
            return Err(OwnerReadError::Refused);
        }
        let key = DraftParentSelection::key_for(ctx.actor(), mind.id());
        let selector = ctx
            .action_state(&self.families.draft_parents, &key)
            .map_err(|_| OwnerReadError::Unavailable)?
            .ok_or(OwnerReadError::Refused)?;
        selector
            .check_mind(mind)
            .map_err(|_| OwnerReadError::Integrity)?;
        selector.validate().map_err(|_| OwnerReadError::Integrity)?;
        if ctx
            .action_state(&self.families.retired_draft_parents, &key)
            .map_err(|_| OwnerReadError::Unavailable)?
            .is_some()
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(RetirementSubject {
            actor: ctx.actor(),
            mind: mind.id(),
            name: request.name,
            base: request.base,
            selector: selector.clone(),
        })
    }
}
