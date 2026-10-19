//! Actual participant request and retained stage/take fence for one operation.
use super::*;
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef};

pub(crate) struct Recall {
    pub(crate) operation: records::MeasuredRecall,
    fence: RecallFence,
}
impl Recall {
    pub(super) fn new(
        operation: records::MeasuredRecall,
        intent: service::Intent,
        runtime: service::Runtime,
        ready: Readiness,
    ) -> Result<Self, PublicationError> {
        operation.validate()?;
        if operation.filed != intent.proof.filed
            || operation.rejection != intent.proof.rejection
            || operation.revision != intent.proof.revision
            || operation.birth_rating != intent.proof.birth_rating
            || operation.plan != intent.proof.plan
            || operation.action != intent.action
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            fence: RecallFence {
                action: operation.action.clone(),
                commit: operation.operation,
                intent,
                runtime,
                ready,
            },
            operation,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn cause_root(
        &self,
    ) -> Result<crate::commerce::roots::RequiredRoot, PublicationError> {
        self.operation.root()
    }
    pub(crate) fn filed_root(&self) -> crate::commerce::roots::RequiredRoot {
        crate::commerce::roots::RequiredRoot {
            anchor: crate::commerce::roots::RootAnchor::Filed(self.operation.filed),
            identity: self.operation.revision,
        }
    }
    pub(crate) fn fence(&self) -> &RecallFence {
        &self.fence
    }
}
#[derive(Clone)]
pub(crate) struct RecallFence {
    action: DefinitionRef,
    commit: CommitToken,
    intent: service::Intent,
    runtime: service::Runtime,
    ready: Readiness,
}
impl RecallFence {
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
