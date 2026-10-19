//! Only the paired Roll writer constructs renewal retention images. The
//! operation and next-term roots are real ordinary Roll records before money.
use super::super::{records::PublicationError, roots::RequiredRoot};
use super::{RenewalOperationV1, protocol::Transition, reader::RenewalFacts};
use mush_core::{
    CommitToken, DefinitionRef, Flags, ObjId, ObjKind, participant::OwnerPrepareContext,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Ordinary(Transition),
    ServiceClose,
}
impl From<Transition> for Publication {
    fn from(value: Transition) -> Self {
        Self::Ordinary(value)
    }
}
impl Publication {
    fn id(self) -> &'static str {
        match self {
            Self::Ordinary(t) => t.id(),
            Self::ServiceClose => mush_economy::managed_renewal::SERVICE_CLOSE_ACTION,
        }
    }
    fn fresh(self) -> bool {
        matches!(self, Self::Ordinary(t) if t.fresh())
    }
}
#[derive(Clone)]
pub(crate) struct RenewalFence {
    action: DefinitionRef,
    commit: CommitToken,
    holder: ObjId,
    transition: Publication,
    fresh: Option<RenewalFacts>,
}
impl RenewalFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.holder
            || ctx.invocation().principal()
                != &(mush_core::action::PrincipalId::Object { id: self.holder })
            || ctx
                .world()
                .get(self.holder)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
            || self.fresh.is_some() != self.transition.fresh()
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(facts) = &self.fresh {
            facts.check_fresh(ctx)?;
        }
        Ok(())
    }
}
pub(crate) struct RenewalRoots {
    fence: RenewalFence,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
}
impl RenewalRoots {
    /// Private writer already consumed the matching actual opaque E4 output
    /// and checked all typed before/post-images. No caller can supply roots.
    pub(super) fn prepare(
        action: &DefinitionRef,
        commit: CommitToken,
        transition: Publication,
        before: Option<&RenewalOperationV1>,
        after: &RenewalOperationV1,
        prior_attempt: Option<&RenewalOperationV1>,
        fresh: Option<&RenewalFacts>,
    ) -> Result<Self, PublicationError> {
        after.validate()?;
        if action.key().owner.as_str() != "market"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != transition.id()
            || action.version() != 1
            || transition.fresh() != fresh.is_some()
            || fresh.is_some_and(|f| f.binding() != after.binding())
        {
            return Err(PublicationError::Mismatch);
        }
        let after_roots = super::audit::required(after)?.to_vec();
        let before_roots = match transition {
            Publication::Ordinary(Transition::Draft) => {
                if before.is_some()
                    || after.operation() != commit
                    || !matches!(after.state(), super::RenewalStateV1::Drafted {})
                {
                    return Err(PublicationError::Mismatch);
                }
                match (after.attempt(), prior_attempt) {
                    (super::RenewalAttemptV1::Automatic {}, None) => Vec::new(),
                    (super::RenewalAttemptV1::BuyerRetry { previous_operation }, Some(prior))
                        if prior.operation() == previous_operation
                            && prior.binding() == after.binding()
                            && prior.state().retryable() =>
                    {
                        // Keep the closed attempt's permanent root; share only
                        // its exact already allocated next-generation root.
                        super::audit::required(prior)?
                            .into_iter()
                            .filter(|r| {
                                matches!(r.anchor, super::super::roots::RootAnchor::Term(..))
                            })
                            .collect()
                    }
                    _ => return Err(PublicationError::Mismatch),
                }
            }
            _ => {
                let before = before.ok_or(PublicationError::Missing)?;
                after.follows(before)?;
                let actual_commit = match transition {
                    Publication::Ordinary(Transition::Fund) => {
                        after.state().funding().map(|r| r.commit())
                    }
                    Publication::Ordinary(
                        Transition::Grant | Transition::Refund | Transition::Close,
                    )
                    | Publication::ServiceClose => after.state().terminal_commit(),
                    Publication::Ordinary(Transition::Draft) => None,
                };
                if actual_commit != Some(commit) {
                    return Err(PublicationError::Mismatch);
                }
                let expected = matches!(
                    (transition, after.state()),
                    (
                        Publication::Ordinary(Transition::Fund),
                        super::RenewalStateV1::Funded { .. }
                    ) | (
                        Publication::Ordinary(Transition::Grant),
                        super::RenewalStateV1::Granted { .. }
                    ) | (
                        Publication::Ordinary(Transition::Refund),
                        super::RenewalStateV1::Refunded { .. }
                    ) | (
                        Publication::Ordinary(Transition::Close),
                        super::RenewalStateV1::UnfundedClosed { .. }
                    ) | (
                        Publication::ServiceClose,
                        super::RenewalStateV1::UnfundedServiceClosed { .. }
                    )
                );
                if !expected || prior_attempt.is_some() {
                    return Err(PublicationError::Mismatch);
                }
                super::audit::required(before)?.to_vec()
            }
        };
        Ok(Self {
            fence: RenewalFence {
                action: action.clone(),
                commit,
                holder: if transition == Publication::ServiceClose {
                    mush_core::GOD
                } else {
                    after.binding.holder
                },
                transition,
                fresh: fresh.cloned(),
            },
            before: before_roots,
            after: after_roots,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &RenewalFence {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}
