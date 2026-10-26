//! Private service lifecycle witness retained through actual owner custody.
use crate::commerce::rating::{RatingError, registration::ReconformanceDefinitions};
use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, Flags, GOD, ObjKind};

#[derive(Clone)]
pub(crate) struct ServiceFence {
    action: DefinitionRef,
    commit: CommitToken,
}
impl ServiceFence {
    pub(crate) fn new(
        action: DefinitionRef,
        manifest: DefinitionRef,
        commit: CommitToken,
    ) -> Result<Self, RatingError> {
        let definitions =
            ReconformanceDefinitions::new(manifest).map_err(|_| RatingError::Identity)?;
        if !definitions.actions().contains(&action) {
            return Err(RatingError::Identity);
        }
        Ok(Self { action, commit })
    }
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), RatingError> {
        let inv = ctx.invocation();
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || inv.actor() != GOD
            || inv.principal() != &(PrincipalId::Object { id: GOD })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || ctx
                .world()
                .get(GOD)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
}
