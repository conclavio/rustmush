//! Only the paired Roll writer constructs renewal retention images. The
//! operation and next-term roots are real ordinary Roll records before money.
use super::super::{Holder, org_holder};
use super::super::{records::PublicationError, roots::RequiredRoot};
use super::{RenewalOperationV1, protocol::Transition, reader::RenewalFacts};
use mush_core::{CommitToken, DefinitionRef, Flags, ObjKind, participant::OwnerPrepareContext};

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
            Self::ServiceClose => mush_economy::managed_house_renewal::SERVICE_CLOSE_ACTION,
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
    holder: Holder,
    transition: Publication,
    fresh: Option<RenewalFacts>,
    buyer_admission: Option<std::sync::Arc<dyn super::super::house_purchase::HouseBuyerAdmission>>,
}
impl RenewalFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            // M8c B3: the invocation's actor is the holder or acts for its
            // organisation (`org_holder::acts_as`, re-asked at every step).
            || org_holder::acts_as(ctx.world(), self.holder, ctx.invocation().actor()).is_err()
            || ctx.invocation().principal()
                != &(mush_core::action::PrincipalId::Object {
                    id: ctx.invocation().actor(),
                })
            || ctx
                .world()
                .get(ctx.invocation().actor())
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
            || self.fresh.is_some() != self.transition.fresh()
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(facts) = &self.fresh {
            facts.check_fresh(ctx)?;
            if let Some(policy) = &self.buyer_admission {
                policy.check(ctx.world(), facts.sale().issuer(), ctx.invocation().actor())?;
            }
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        action: &DefinitionRef,
        commit: CommitToken,
        transition: Publication,
        before: Option<&RenewalOperationV1>,
        after: &RenewalOperationV1,
        prior_attempt: Option<&RenewalOperationV1>,
        fresh: Option<&RenewalFacts>,
        buyer_admission: Option<
            std::sync::Arc<dyn super::super::house_purchase::HouseBuyerAdmission>,
        >,
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
                    (
                        super::RenewalAttemptV1::Automatic {}
                        | super::RenewalAttemptV1::HolderAutomatic { .. },
                        None,
                    ) => Vec::new(),
                    (
                        super::RenewalAttemptV1::BuyerRetry { previous_operation }
                        | super::RenewalAttemptV1::HolderRetry {
                            previous_operation, ..
                        },
                        Some(prior),
                    ) if prior.operation() == previous_operation
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
                    (super::RenewalAttemptV1::HolderAutomatic { basis }, Some(prior))
                        if prior.state().retryable()
                            && prior.binding().licence() == after.binding().licence()
                            && prior.binding().birth_operation()
                                == after.binding().birth_operation()
                            && prior.binding().next() == after.binding().next()
                            && prior.binding().revision() == after.binding().revision()
                            && basis.ownership.generation
                                > prior
                                    .binding()
                                    .holder_basis()
                                    .map_or(1, |b| b.ownership.generation) =>
                    {
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
                    Holder::Character(mush_core::GOD)
                } else {
                    after.binding.holder
                },
                transition,
                fresh: fresh.cloned(),
                buyer_admission: transition.fresh().then_some(buyer_admission).flatten(),
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
