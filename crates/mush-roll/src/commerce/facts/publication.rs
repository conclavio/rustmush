//! Actual participant request and retained stage/take fence for one occurrence.
use super::*;
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef};

pub(crate) struct Lapse {
    pub(crate) occurrence: records::LapseOccurrence,
    fence: LapseFence,
}
impl Lapse {
    pub(super) fn new(
        occurrence: records::LapseOccurrence,
        intent: service::Intent,
        runtime: service::Runtime,
        ready: Readiness,
    ) -> Result<Self, PublicationError> {
        occurrence.validate()?;
        if occurrence.term != intent.accepted || occurrence.action != intent.action {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            fence: LapseFence {
                action: occurrence.action.clone(),
                commit: occurrence.operation,
                intent,
                runtime,
                ready,
            },
            occurrence,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.occurrence.operation
    }
    pub(crate) fn root(&self) -> Result<super::super::roots::RequiredRoot, PublicationError> {
        self.occurrence.root()
    }
    pub(crate) fn fence(&self) -> &LapseFence {
        &self.fence
    }
}
#[derive(Clone)]
pub(crate) struct LapseFence {
    action: DefinitionRef,
    commit: CommitToken,
    intent: service::Intent,
    runtime: service::Runtime,
    ready: Readiness,
}
impl LapseFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        self.ready.check(ctx.world())?;
        if !service::live(ctx.world()) {
            return Err(PublicationError::Unavailable);
        }
        self.runtime.require(&self.intent)?;
        self.intent.fresh(ctx.world())?;
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || self.runtime.intent(ctx.invocation(), ctx.action())? != self.intent
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
