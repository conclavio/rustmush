//! Only the paired Roll writer constructs renewal retention images. The
//! operation and next-term roots are real ordinary Roll records before money.
use super::super::{Holder, org_holder};
use super::super::{
    records::{PublicationError, Record},
    roots::RequiredRoot,
};
use super::records::{RenewalAttemptV1, RenewalStateV1};
use super::{Binding, Facts, Operation, Rules, protocol::Step as Transition};
use mush_core::{CommitToken, DefinitionRef, Flags, ObjKind, participant::OwnerPrepareContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Publication {
    Ordinary(Transition),
    ServiceClose,
}
impl From<Transition> for Publication {
    fn from(value: Transition) -> Self {
        Self::Ordinary(value)
    }
}
impl Publication {
    fn id<I: Rules>(self) -> &'static str {
        match self {
            Self::Ordinary(t) => t.id::<I>(),
            Self::ServiceClose => <I as mush_economy::renewal_twin::Issuer>::SERVICE_CLOSE_ACTION,
        }
    }
    fn fresh(self) -> bool {
        matches!(self, Self::Ordinary(t) if t.fresh())
    }
}
#[derive(Clone)]
pub(crate) struct RenewalFence<I: super::Issuer> {
    action: DefinitionRef,
    commit: CommitToken,
    holder: Holder,
    /// Doc 17h §3 M8f P2a: the renewal's price for the steps that move
    /// money (Fund, Grant, Refund), a read's zero for the others (the Draft
    /// and the closes: a limited grant renews, the lead's ruling).
    amount: Option<i64>,
    transition: Publication,
    fresh: Option<I::Facts>,
    /// The House's buyer admission at a fresh step (§4.3 5); nothing for the personal issuer.
    admission: I::Admission,
}
impl<I: Rules> RenewalFence<I> {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            // M8c B3: the invocation's actor is the holder or acts for its
            // organisation (`org_holder::acts_as`, re-asked at every step;
            // doc 17h §3 M8f P2a: with the price at a step that moves money).
            || org_holder::acts_as_amount(
                ctx.world(),
                self.holder,
                ctx.invocation().actor(),
                self.amount,
            )
            .is_err()
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
            facts.check_fresh(ctx, self.amount)?;
            I::admit_fresh(&self.admission, ctx, facts)?;
        }
        Ok(())
    }
}
pub(crate) struct RenewalRoots<I: super::Issuer> {
    fence: RenewalFence<I>,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
}
impl<I: Rules> RenewalRoots<I> {
    /// Private writer already consumed the matching actual opaque E4 output
    /// and checked all typed before/post-images. No caller can supply roots.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        action: &DefinitionRef,
        commit: CommitToken,
        transition: Publication,
        before: Option<&I::Operation>,
        after: &I::Operation,
        prior_attempt: Option<&I::Operation>,
        fresh: Option<&I::Facts>,
        admission: I::Admission,
    ) -> Result<Self, PublicationError> {
        after.validate()?;
        if action.key().owner.as_str() != "market"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != transition.id::<I>()
            || action.version() != 1
            || transition.fresh() != fresh.is_some()
            || fresh.is_some_and(|f| f.binding() != after.binding())
        {
            return Err(PublicationError::Mismatch);
        }
        let after_roots = super::audit::required::<I>(after)?.to_vec();
        let before_roots = match transition {
            Publication::Ordinary(Transition::Draft) => {
                if before.is_some()
                    || after.operation() != commit
                    || !matches!(after.state(), RenewalStateV1::Drafted {})
                {
                    return Err(PublicationError::Mismatch);
                }
                match (after.attempt(), prior_attempt) {
                    (
                        RenewalAttemptV1::Automatic {} | RenewalAttemptV1::HolderAutomatic { .. },
                        None,
                    ) => Vec::new(),
                    (
                        RenewalAttemptV1::BuyerRetry { previous_operation }
                        | RenewalAttemptV1::HolderRetry {
                            previous_operation, ..
                        },
                        Some(prior),
                    ) if prior.operation() == previous_operation
                        && prior.binding() == after.binding()
                        && prior.state().retryable() =>
                    {
                        // Keep the closed attempt's permanent root; share only
                        // its exact already allocated next-generation root.
                        super::audit::required::<I>(prior)?
                            .into_iter()
                            .filter(|r| {
                                matches!(r.anchor, super::super::roots::RootAnchor::Term(..))
                            })
                            .collect()
                    }
                    (RenewalAttemptV1::HolderAutomatic { basis }, Some(prior))
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
                        super::audit::required::<I>(prior)?
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
                        RenewalStateV1::Funded { .. }
                    ) | (
                        Publication::Ordinary(Transition::Grant),
                        RenewalStateV1::Granted { .. }
                    ) | (
                        Publication::Ordinary(Transition::Refund),
                        RenewalStateV1::Refunded { .. }
                    ) | (
                        Publication::Ordinary(Transition::Close),
                        RenewalStateV1::UnfundedClosed { .. }
                    ) | (
                        Publication::ServiceClose,
                        RenewalStateV1::UnfundedServiceClosed { .. }
                    )
                );
                if !expected || prior_attempt.is_some() {
                    return Err(PublicationError::Mismatch);
                }
                super::audit::required::<I>(before)?.to_vec()
            }
        };
        Ok(Self {
            fence: RenewalFence {
                action: action.clone(),
                commit,
                holder: if transition == Publication::ServiceClose {
                    Holder::Character(mush_core::GOD)
                } else {
                    after.binding().holder()
                },
                amount: if matches!(
                    transition,
                    Publication::Ordinary(
                        Transition::Fund | Transition::Grant | Transition::Refund
                    )
                ) {
                    Some(after.binding().price_minor())
                } else {
                    org_holder::READ_AMOUNT
                },
                transition,
                fresh: fresh.cloned(),
                admission: if transition.fresh() {
                    admission
                } else {
                    I::Admission::default()
                },
            },
            before: before_roots,
            after: after_roots,
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &RenewalFence<I> {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}
