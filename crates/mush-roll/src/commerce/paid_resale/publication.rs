//! Private same-ACK retention and custody fence. Only the trusted Roll writer
//! constructs these requests after matching actual opaque Economy output.
use super::reader::PaidResaleFacts;
use crate::commerce::records::PublicationError;
use crate::commerce::roots::RequiredRoot;
use mush_core::participant::OwnerPrepareContext;
use mush_core::{ClockStamp, CommitToken, DefinitionRef};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct ResaleFence {
    pub(super) action: DefinitionRef,
    pub(super) commit: CommitToken,
    pub(super) at_ic: ClockStamp,
    pub(super) at_utc: i64,
    pub(super) mode: Mode,
}
#[derive(Clone)]
pub(super) enum Mode {
    Registry {
        ready: super::Readiness,
        fence: Box<crate::commerce::registry_revocation::RevocationFence>,
    },
    /// The buyer's Accept or Fund. 17g C1a: the writer's optional House buyer
    /// admission rides beside the facts, so the fence that holds the Root
    /// custody also repeats the House's acquisition condition.
    Admission(
        Box<PaidResaleFacts>,
        Option<Arc<dyn crate::commerce::house_purchase::HouseBuyerAdmission>>,
    ),
    Service {
        runtime: Box<super::service::Runtime>,
        ready: super::Readiness,
        intent: Box<super::service::Intent>,
    },
}
impl ResaleFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let now = ctx.world().world_ic_now();
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
            || !ctx.invocation().targets().is_empty()
            || now.clock != self.at_ic.clock
            || now.at < self.at_ic.at
            || ctx.world().now() < self.at_utc
        {
            return Err(PublicationError::Mismatch);
        }
        match &self.mode {
            Mode::Registry { ready, fence } => {
                ready.check()?;
                fence.check(ctx)
            }
            Mode::Admission(admission, house) => {
                if ctx.invocation().actor() != admission.buyer()
                    || ![super::protocol::action(1)?, super::protocol::action(2)?]
                        .contains(&self.action)
                {
                    return Err(PublicationError::Mismatch);
                }
                admission.check_fresh(ctx)?;
                // 17g C1a, the conservative reading of the owner's 2026-10-18
                // Coprates ruling (17f §2.1): a resold House Pin's buyer passes
                // the same House buyer admission a fresh purchase does. Personal
                // births carry no issuer and are untouched. A refusal is an
                // ordinary publication error, so the provider answers with its
                // one uniform refusal and no new sentence.
                if let (Some(policy), Some(issuer)) = (house, admission.house_issuer()) {
                    policy.check(ctx.world(), issuer, admission.buyer())?;
                }
                Ok(())
            }
            Mode::Service {
                runtime,
                ready,
                intent,
            } => {
                ready.check()?;
                runtime.require(intent)?;
                if &runtime.intent(ctx.invocation(), ctx.action())? != intent.as_ref() {
                    return Err(PublicationError::Mismatch);
                }
                intent.fresh(ctx.world())
            }
        }
    }
}
pub(crate) struct ResaleRoots {
    pub(super) commit: CommitToken,
    pub(super) old: Vec<RequiredRoot>,
    pub(super) added: Vec<RequiredRoot>,
    pub(super) fence: ResaleFence,
}
impl ResaleRoots {
    pub(crate) const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn old(&self) -> &[RequiredRoot] {
        &self.old
    }
    pub(crate) fn added(&self) -> &[RequiredRoot] {
        &self.added
    }
    pub(crate) fn fence(&self) -> &ResaleFence {
        &self.fence
    }
}
