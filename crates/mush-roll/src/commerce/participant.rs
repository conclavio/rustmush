//! The registered Roll retention participant. No public root API.

use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::OwnerId;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{CommitToken, ModuleWrite};
use std::sync::Arc;

use super::paid_roots::PaidRoots;
use super::rating_roots::RatingRoots;
use super::roots::{InitialRoots, RootAnchor};
use super::usage::UsageChange;
use crate::{BoundCommercialRoots, NS, PreparedChange, ReferenceKind, ReferenceRecord, RollStore};

/// Only Roll's actual module registration can mint a request handle for this
/// private adapter. The filing builder checks real typed post-images before
/// constructing InitialRoots; no wire decoder or generic external owner lane
/// can manufacture this request or choose arbitrary retained targets.
pub(crate) struct RootParticipant;

/// Only closed Roll builders derive these requests from actual typed images.
/// The Registry variant is not registered for an action until its actual quota
/// and artifact participants are available in the same composed invocation.
pub(crate) enum RootRequest {
    Conduct(Box<super::conduct::publication::Roots>),
    TermLapse(Box<super::facts::publication::Lapse>),
    Ic(Box<super::ic_claims::roots::IcRoots>),
    Coverage(Box<super::coverage::roots::CoverageRoots>),
    InitialFiling(Box<InitialRoots>),
    CompatibleUpdate(Box<super::line_update::publication::UpdateRoots>),
    DependentFiling(Box<super::dependent_filing::DependentRoots>),
    Rating(Box<RatingRoots>),
    Paid(Box<PaidRoots>),
    Entitlement(Box<super::entitlement::roots::EntitlementRoots>),
    House(Box<super::house_purchase::roots::HouseRoots>),
    Renewal(Box<super::renewal::roots::RenewalRoots>),
    HouseRenewal(Box<super::house_renewal::roots::RenewalRoots>),
    AuthoredFork(Box<super::authored_fork::AuthoredFork>),
    Retirement(Box<super::authored_retirement::Retirement>),
    Withdrawal(Box<super::withdrawal::Withdrawal>),
    CharterTransfer(Box<super::charter_transfer::TransferRoots>),
    CharterPending(Box<super::charter_pending::publication::PendingRoots>),
    PaidResale(Box<super::paid_resale::publication::ResaleRoots>),
    RegistryRecall(Box<super::registry_recall::Recall>),
    MeasuredRecall(Box<super::rating::reconformance::recall::publication::Recall>),
    RegistryRevocation(Box<super::registry_revocation::Revocation>),
    OwnerQualification(Box<super::owner_qualification::Qualification>),
    OwnerAttestation(Box<super::owner_attestation::Attestation>),
}

impl RootRequest {
    fn commit(&self) -> CommitToken {
        match self {
            Self::Conduct(request) => request.commit(),
            Self::TermLapse(request) => request.commit(),
            Self::Ic(request) => request.commit(),
            Self::Coverage(request) => request.commit(),
            Self::InitialFiling(request) => request.commit(),
            Self::CompatibleUpdate(request) => request.commit(),
            Self::DependentFiling(request) => request.commit(),
            Self::Rating(request) => request.commit(),
            Self::Paid(request) => request.commit(),
            Self::Entitlement(request) => request.commit(),
            Self::House(request) => request.commit(),
            Self::Renewal(request) => request.commit(),
            Self::HouseRenewal(request) => request.commit(),
            Self::AuthoredFork(request) => request.commit(),
            Self::Retirement(request) => request.commit(),
            Self::Withdrawal(request) => request.commit(),
            Self::CharterTransfer(request) => request.commit(),
            Self::CharterPending(request) => request.commit(),
            Self::PaidResale(request) => request.commit(),
            Self::RegistryRecall(request) => request.commit(),
            Self::MeasuredRecall(request) => request.commit(),
            Self::RegistryRevocation(request) => request.commit(),
            Self::OwnerQualification(request) => request.commit(),
            Self::OwnerAttestation(request) => request.commit(),
        }
    }
}

/// Existing callers discard usage; fork additionally receives only the actual
/// owner-allocated selector image, not a caller-supplied id or publication token.
pub(crate) struct RootOutput {
    usage: UsageChange,
    selection: Option<super::parent_selector::DraftParentSelection>,
    retirement: Option<super::authored_retirement::RetirementOutput>,
}
impl RootOutput {
    pub(crate) fn retirement(
        self,
    ) -> Result<super::authored_retirement::RetirementOutput, super::records::PublicationError>
    {
        self.retirement
            .ok_or(super::records::PublicationError::Mismatch)
    }
    pub(crate) fn authored_fork(
        self,
    ) -> Result<super::parent_selector::DraftParentSelection, super::records::PublicationError>
    {
        let _usage = self.usage;
        self.selection
            .ok_or(super::records::PublicationError::Mismatch)
    }
}

pub(crate) struct RootPatch {
    conduct: Option<Box<super::conduct::publication::Fence>>,
    term_lapse: Option<Box<super::facts::publication::LapseFence>>,
    commit: CommitToken,
    ic: Option<Box<super::ic_claims::roots::IcFence>>,
    coverage: Option<Box<super::coverage::roots::CoverageFence>>,
    change: PreparedChange,
    fresh_paid: Option<Box<super::sale_reader::OriginalSaleFacts>>,
    entitlement: Option<Box<super::entitlement::roots::EntitlementFence>>,
    house: Option<Box<super::house_purchase::roots::HouseFence>>,
    renewal: Option<Box<super::renewal::roots::RenewalFence>>,
    house_renewal: Option<Box<super::house_renewal::roots::RenewalFence>>,
    dependent: Option<Box<super::dependent_filing::FilingFence>>,
    update: Option<Box<super::line_update::publication::UpdateFence>>,
    marked: Option<Box<super::marked_rating::AdmissionFence>>,
    reconformance: Option<Box<super::rating::reconformance::fence::ServiceFence>>,
    retirement: Option<Box<super::authored_retirement::RetirementFence>>,
    withdrawal: Option<Box<super::withdrawal::WithdrawalFence>>,
    charter_transfer: Option<Box<super::charter_transfer::TransferFence>>,
    charter_pending: Option<Box<super::charter_pending::publication::PendingFence>>,
    paid_resale: Option<Box<super::paid_resale::publication::ResaleFence>>,
    registry_recall: Option<Box<super::registry_recall::RecallFence>>,
    measured_recall: Option<Box<super::rating::reconformance::recall::publication::RecallFence>>,
    registry_revocation: Option<Box<super::registry_revocation::RevocationFence>>,
    qualification: Option<Box<super::owner_qualification::QualificationFence>>,
    attestation: Option<Box<super::owner_attestation::AttestationFence>>,
    fork: Option<
        Box<(
            mush_core::DefinitionRef,
            super::parent_grant::CurrentParentGrant,
        )>,
    >,
}

#[inline(never)]
fn capture_conduct(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::conduct::publication::Fence>>, ParticipantError> {
    let conduct = match request {
        RootRequest::Conduct(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(conduct)
}

#[inline(never)]
fn capture_term_lapse(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::facts::publication::LapseFence>>, ParticipantError> {
    let term_lapse = match request {
        RootRequest::TermLapse(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(term_lapse)
}

#[inline(never)]
fn capture_ic(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::ic_claims::roots::IcFence>>, ParticipantError> {
    let ic = match request {
        RootRequest::Ic(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(ic)
}

#[inline(never)]
fn capture_dependent(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::dependent_filing::FilingFence>>, ParticipantError> {
    let dependent = match request {
        RootRequest::DependentFiling(request) => {
            request
                .fence()
                .check(request.commit(), context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        RootRequest::InitialFiling(_) => {
            if context.action().key().owner.as_str() != NS
                || context.action().key().kind.as_str() != "action"
                || !matches!(
                    context.action().key().id.as_str(),
                    "initial_filing" | "house_initial_filing"
                )
                || context.action().version() != 1
            {
                return Err(ParticipantError::Authority);
            }
            None
        }
        _ => None,
    };
    Ok(dependent)
}

#[inline(never)]
fn capture_fresh_paid(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::sale_reader::OriginalSaleFacts>>, ParticipantError> {
    let fresh_paid = match request {
        RootRequest::Paid(paid) => paid.fresh().cloned().map(Box::new),
        _ => None,
    };
    super::paid_roots::check_fresh(fresh_paid.as_deref(), context)
        .map_err(|_| ParticipantError::Authority)?;
    Ok(fresh_paid)
}

#[inline(never)]
fn capture_reconformance(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::rating::reconformance::fence::ServiceFence>>, ParticipantError> {
    let reconformance = match request {
        RootRequest::Rating(request) => request.reconformance().cloned().map(Box::new),
        _ => None,
    };
    if let Some(fence) = &reconformance {
        fence
            .check(context)
            .map_err(|_| ParticipantError::Authority)?;
    }
    Ok(reconformance)
}

#[inline(never)]
fn capture_marked(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::marked_rating::AdmissionFence>>, ParticipantError> {
    let marked = match request {
        RootRequest::Rating(request) => request.marked().cloned().map(Box::new),
        _ => None,
    };
    if let Some(marked) = &marked {
        marked
            .check(context)
            .map_err(|_| ParticipantError::Authority)?;
    }
    Ok(marked)
}

#[inline(never)]
fn capture_fork(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<
    Option<
        Box<(
            mush_core::DefinitionRef,
            super::parent_grant::CurrentParentGrant,
        )>,
    >,
    ParticipantError,
> {
    let fork = match request {
        RootRequest::AuthoredFork(request) => {
            request
                .check_context(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new((
                request.action().clone(),
                request.grant().clone(),
            )))
        }
        _ => None,
    };
    Ok(fork)
}

#[inline(never)]
fn capture_retirement(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::authored_retirement::RetirementFence>>, ParticipantError> {
    let retirement = match request {
        RootRequest::Retirement(request) => {
            request
                .check_context(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence()))
        }
        _ => None,
    };
    Ok(retirement)
}

#[inline(never)]
fn capture_entitlement(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::entitlement::roots::EntitlementFence>>, ParticipantError> {
    let entitlement = match request {
        RootRequest::Entitlement(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(entitlement)
}

#[inline(never)]
fn capture_coverage(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::coverage::roots::CoverageFence>>, ParticipantError> {
    let coverage = match request {
        RootRequest::Coverage(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(coverage)
}

#[inline(never)]
fn capture_house(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::house_purchase::roots::HouseFence>>, ParticipantError> {
    let house = match request {
        RootRequest::House(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(house)
}

#[inline(never)]
fn capture_renewal(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::renewal::roots::RenewalFence>>, ParticipantError> {
    let renewal = match request {
        RootRequest::Renewal(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(renewal)
}

#[inline(never)]
fn capture_house_renewal(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::house_renewal::roots::RenewalFence>>, ParticipantError> {
    let house_renewal = match request {
        RootRequest::HouseRenewal(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(house_renewal)
}

#[inline(never)]
fn capture_registry_recall(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::registry_recall::RecallFence>>, ParticipantError> {
    let registry_recall = match request {
        RootRequest::RegistryRecall(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(registry_recall)
}

#[inline(never)]
fn capture_measured_recall(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<
    Option<Box<super::rating::reconformance::recall::publication::RecallFence>>,
    ParticipantError,
> {
    let measured_recall = match request {
        RootRequest::MeasuredRecall(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(measured_recall)
}

#[inline(never)]
fn capture_registry_revocation(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::registry_revocation::RevocationFence>>, ParticipantError> {
    let registry_revocation = match request {
        RootRequest::RegistryRevocation(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(registry_revocation)
}

#[inline(never)]
fn capture_paid_resale(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::paid_resale::publication::ResaleFence>>, ParticipantError> {
    let paid_resale = match request {
        RootRequest::RegistryRevocation(request) => request
            .resale()
            .map(|r| {
                r.fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Ok(Box::new(r.fence().clone()))
            })
            .transpose()?,
        RootRequest::PaidResale(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(paid_resale)
}

#[inline(never)]
fn capture_charter_pending(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::charter_pending::publication::PendingFence>>, ParticipantError> {
    let charter_pending = match request {
        RootRequest::RegistryRevocation(request) => request
            .pending()
            .map(|p| {
                p.fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Ok(Box::new(p.fence().clone()))
            })
            .transpose()?,
        RootRequest::CharterPending(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(charter_pending)
}

#[inline(never)]
fn capture_charter_transfer(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::charter_transfer::TransferFence>>, ParticipantError> {
    let charter_transfer = match request {
        RootRequest::CharterTransfer(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(charter_transfer)
}

#[inline(never)]
fn capture_withdrawal(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::withdrawal::WithdrawalFence>>, ParticipantError> {
    let withdrawal = match request {
        RootRequest::Withdrawal(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(withdrawal)
}

#[inline(never)]
fn capture_attestation(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::owner_attestation::AttestationFence>>, ParticipantError> {
    let attestation = match request {
        RootRequest::OwnerAttestation(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(attestation)
}

#[inline(never)]
fn capture_qualification(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::owner_qualification::QualificationFence>>, ParticipantError> {
    let qualification = match request {
        RootRequest::OwnerQualification(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(qualification)
}

#[inline(never)]
fn capture_update(
    request: &RootRequest,
    context: &OwnerPrepareContext<'_>,
) -> Result<Option<Box<super::line_update::publication::UpdateFence>>, ParticipantError> {
    let update = match request {
        RootRequest::CompatibleUpdate(request) => {
            request
                .fence()
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
            Some(Box::new(request.fence().clone()))
        }
        _ => None,
    };
    Ok(update)
}

#[inline(never)]
fn plan_term_lapse(
    state: &RollStore,
    request: &super::facts::publication::Lapse,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_term_lapse_root(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_ic(
    state: &RollStore,
    request: &super::ic_claims::roots::IcRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_ic_claim_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_initial_filing(
    state: &RollStore,
    request: &InitialRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_commercial_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_compatible_update(
    state: &RollStore,
    request: &super::line_update::publication::UpdateRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_line_update_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_dependent_filing(
    state: &RollStore,
    request: &super::dependent_filing::DependentRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_commercial_roots(request.roots())
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_conduct(
    state: &RollStore,
    request: &super::conduct::publication::Roots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = { (state.prepare_conduct_root(request).map_err(refusal)?, None) };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_rating(
    state: &RollStore,
    request: &RatingRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = { (state.prepare_rating_roots(request).map_err(refusal)?, None) };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_paid(
    state: &RollStore,
    request: &PaidRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = { (state.prepare_paid_roots(request).map_err(refusal)?, None) };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_entitlement(
    state: &RollStore,
    request: &super::entitlement::roots::EntitlementRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_entitlement_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_house(
    state: &RollStore,
    request: &super::house_purchase::roots::HouseRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = { (state.prepare_house_roots(request).map_err(refusal)?, None) };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_coverage(
    state: &RollStore,
    request: &super::coverage::roots::CoverageRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_coverage_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_renewal(
    state: &RollStore,
    request: &super::renewal::roots::RenewalRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = { (state.prepare_renewal_roots(request).map_err(refusal)?, None) };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_house_renewal(
    state: &RollStore,
    request: &super::house_renewal::roots::RenewalRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_house_renewal_roots(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_owner_attestation(
    state: &RollStore,
    request: &super::owner_attestation::Attestation,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_owner_attestation_roots(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_owner_qualification(
    state: &RollStore,
    request: &super::owner_qualification::Qualification,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_owner_qualification_root(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_registry_recall(
    state: &RollStore,
    request: &super::registry_recall::Recall,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_registry_recall_root(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_measured_recall(
    state: &RollStore,
    request: &super::rating::reconformance::recall::publication::Recall,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_measured_recall_root(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_registry_revocation(
    state: &RollStore,
    request: &super::registry_revocation::Revocation,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_registry_revocation_root(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_paid_resale(
    state: &RollStore,
    request: &super::paid_resale::publication::ResaleRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_paid_resale_roots(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_charter_pending(
    state: &RollStore,
    request: &super::charter_pending::publication::PendingRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_charter_pending_roots(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_charter_transfer(
    state: &RollStore,
    request: &super::charter_transfer::TransferRoots,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state
            .prepare_charter_transfer_root(request)
            .map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_withdrawal(
    state: &RollStore,
    request: &super::withdrawal::Withdrawal,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = (
        state.prepare_withdrawal_root(request).map_err(refusal)?,
        None,
    );
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_retirement(
    state: &RollStore,
    request: &super::authored_retirement::Retirement,
    retired: &mut Option<super::authored_retirement::RetirementOutput>,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = {
        let (change, output) = state
            .prepare_authored_retirement(request)
            .map_err(refusal)?;
        *retired = Some(output);
        (change, None)
    };
    Ok(Box::new(result))
}

#[inline(never)]
fn plan_authored_fork(
    state: &RollStore,
    request: &super::authored_fork::AuthoredFork,
) -> Result<
    Box<(
        PreparedChange,
        Option<super::parent_selector::DraftParentSelection>,
    )>,
    ParticipantError,
> {
    let result = {
        let (change, selection) = state.prepare_authored_fork(request).map_err(refusal)?;
        (change, Some(selection))
    };
    Ok(Box::new(result))
}

impl OwnerParticipant for RootParticipant {
    type State = RollStore;
    type Request = RootRequest;
    type Output = RootOutput;
    type Patch = RootPatch;
    type Bound<'a> = BoundCommercialRoots<'a>;

    fn prepare(
        &self,
        state: &RollStore,
        context: &OwnerPrepareContext<'_>,
        request: RootRequest,
    ) -> Result<OwnerPreparation<RootOutput, RootPatch>, ParticipantError> {
        if request.commit() != context.commit_token() {
            return Err(ParticipantError::Authority);
        }
        let conduct = capture_conduct(&request, context)?;
        let term_lapse = capture_term_lapse(&request, context)?;
        let ic = capture_ic(&request, context)?;
        let dependent = capture_dependent(&request, context)?;
        let fresh_paid = capture_fresh_paid(&request, context)?;
        let reconformance = capture_reconformance(&request, context)?;
        let marked = capture_marked(&request, context)?;
        let fork = capture_fork(&request, context)?;
        let retirement = capture_retirement(&request, context)?;
        let entitlement = capture_entitlement(&request, context)?;
        let coverage = capture_coverage(&request, context)?;
        let house = capture_house(&request, context)?;
        let renewal = capture_renewal(&request, context)?;
        let house_renewal = capture_house_renewal(&request, context)?;
        let registry_recall = capture_registry_recall(&request, context)?;
        let measured_recall = capture_measured_recall(&request, context)?;
        let registry_revocation = capture_registry_revocation(&request, context)?;
        let paid_resale = capture_paid_resale(&request, context)?;
        let charter_pending = capture_charter_pending(&request, context)?;
        let charter_transfer = capture_charter_transfer(&request, context)?;
        let withdrawal = capture_withdrawal(&request, context)?;
        let attestation = capture_attestation(&request, context)?;
        let qualification = capture_qualification(&request, context)?;
        let update = capture_update(&request, context)?;
        let mut retired = None;
        let (mut change, selection) = *match &request {
            RootRequest::TermLapse(request) => plan_term_lapse(state, request)?,
            RootRequest::Ic(request) => plan_ic(state, request)?,
            RootRequest::InitialFiling(request) => plan_initial_filing(state, request)?,
            RootRequest::CompatibleUpdate(request) => plan_compatible_update(state, request)?,
            RootRequest::DependentFiling(request) => plan_dependent_filing(state, request)?,
            RootRequest::Conduct(request) => plan_conduct(state, request)?,
            RootRequest::Rating(request) => plan_rating(state, request)?,
            RootRequest::Paid(request) => plan_paid(state, request)?,
            RootRequest::Entitlement(request) => plan_entitlement(state, request)?,
            RootRequest::House(request) => plan_house(state, request)?,
            RootRequest::Coverage(request) => plan_coverage(state, request)?,
            RootRequest::Renewal(request) => plan_renewal(state, request)?,
            RootRequest::HouseRenewal(request) => plan_house_renewal(state, request)?,
            RootRequest::OwnerAttestation(request) => plan_owner_attestation(state, request)?,
            RootRequest::OwnerQualification(request) => plan_owner_qualification(state, request)?,
            RootRequest::RegistryRecall(request) => plan_registry_recall(state, request)?,
            RootRequest::MeasuredRecall(request) => plan_measured_recall(state, request)?,
            RootRequest::RegistryRevocation(request) => plan_registry_revocation(state, request)?,
            RootRequest::PaidResale(request) => plan_paid_resale(state, request)?,
            RootRequest::CharterPending(request) => plan_charter_pending(state, request)?,
            RootRequest::CharterTransfer(request) => plan_charter_transfer(state, request)?,
            RootRequest::Withdrawal(request) => plan_withdrawal(state, request)?,
            RootRequest::Retirement(request) => plan_retirement(state, request, &mut retired)?,
            RootRequest::AuthoredFork(request) => plan_authored_fork(state, request)?,
        };
        let follow = match &request {
            RootRequest::InitialFiling(request) => Some(
                request
                    .follow()
                    .cloned()
                    .ok_or(ParticipantError::NotReady)?,
            ),
            RootRequest::DependentFiling(request) => Some(
                request
                    .roots()
                    .follow()
                    .cloned()
                    .ok_or(ParticipantError::NotReady)?,
            ),
            RootRequest::CompatibleUpdate(request) => Some(
                request
                    .follow()
                    .cloned()
                    .ok_or(ParticipantError::NotReady)?,
            ),
            RootRequest::Rating(request) => request.follow().cloned(),
            RootRequest::Withdrawal(request) => Some(
                super::follow::Change::blocked(&request.filed_root())
                    .map_err(|_| ParticipantError::Authority)?,
            ),
            RootRequest::RegistryRecall(request) => Some(
                super::follow::Change::blocked(&request.filed_root())
                    .map_err(|_| ParticipantError::Authority)?,
            ),
            RootRequest::MeasuredRecall(request) => Some(
                super::follow::Change::blocked(&request.filed_root())
                    .map_err(|_| ParticipantError::Authority)?,
            ),
            _ => None,
        };
        if let Some(follow) = follow {
            state
                .prepare_follow(&mut change, &follow)
                .map_err(refusal)?;
        }
        let usage = change.usage().ok_or(ParticipantError::NotReady)?;
        let writes = change
            .operations()
            .iter()
            .map(|op| {
                let namespace = OwnerId::parse_canonical(op.ns.as_str())
                    .map_err(|_| ParticipantError::Encoding)?;
                let write = match &op.op {
                    ModuleWrite::JsonSet { key, value } => ReplayWrite::ModuleJsonSet {
                        namespace,
                        key: AuthoritativeKey::parse_canonical(key)
                            .map_err(|_| ParticipantError::Encoding)?,
                        value: value.clone(),
                    },
                    ModuleWrite::Del { key } => ReplayWrite::ModuleDelete {
                        namespace,
                        key: AuthoritativeKey::parse_canonical(key)
                            .map_err(|_| ParticipantError::Encoding)?,
                    },
                    ModuleWrite::StreamAppend { .. } => return Err(ParticipantError::WriteFamily),
                };
                if !self.accepts_write(&write) {
                    if std::env::var_os("M7_CONDUCT_DIAG").is_some() {
                        // TEMP M7 diag
                        let key = match &write {
                            ReplayWrite::ModuleJsonSet { namespace, key, .. } => format!("set {namespace}/{key}"),
                            ReplayWrite::ModuleDelete { namespace, key } => format!("delete {namespace}/{key}"),
                            _ => "other".to_string(),
                        };
                        eprintln!("M7_CONDUCT_DIAG roll_root_write_family {key}");
                    }
                    return Err(ParticipantError::WriteFamily);
                }
                Ok(write)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let output = RootOutput {
            usage,
            selection,
            retirement: retired,
        };
        Ok(OwnerPreparation::new(
            output,
            RootPatch {
                conduct,
                term_lapse,
                commit: request.commit(),
                ic,
                coverage,
                change,
                fresh_paid,
                entitlement,
                house,
                renewal,
                house_renewal,
                dependent,
                update,
                marked,
                reconformance,
                retirement,
                withdrawal,
                charter_transfer,
                charter_pending,
                paid_resale,
                registry_recall,
                measured_recall,
                registry_revocation,
                qualification,
                attestation,
                fork,
            },
            writes,
        ))
    }

    fn recheck(
        &self,
        state: &RollStore,
        context: &OwnerPrepareContext<'_>,
        patch: &RootPatch,
    ) -> Result<(), ParticipantError> {
        if patch.commit != context.commit_token() {
            return Err(ParticipantError::Authority);
        }
        if let Some(fence) = &patch.conduct {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.term_lapse {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.ic {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.attestation {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.qualification {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.update {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.renewal {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.house_renewal {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.entitlement {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.coverage {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.house {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.dependent {
            fence
                .check(patch.commit, context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        super::paid_roots::check_fresh(patch.fresh_paid.as_deref(), context)
            .map_err(|_| ParticipantError::Authority)?;
        if let Some(fence) = &patch.reconformance {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(marked) = &patch.marked {
            marked
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some((action, grant)) = patch.fork.as_deref() {
            super::authored_fork::check_action(action, patch.commit, grant, context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.registry_recall {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.measured_recall {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.registry_revocation {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.paid_resale {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.charter_pending {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.charter_transfer {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.withdrawal {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        if let Some(fence) = &patch.retirement {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        state
            .check_commercial_prepared(&patch.change)
            .map_err(refusal)
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut RollStore,
        patch: RootPatch,
    ) -> Result<BoundCommercialRoots<'a>, ParticipantError> {
        state.bind_commercial_roots(patch.change).map_err(refusal)
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        if let ReplayWrite::ModuleDelete { namespace, key } = write {
            // Only a pending output root or one exact retired Mind may be
            // deleted. Private request preparation proves its exact cause.
            if namespace.as_str() != NS {
                return false;
            }
            if let Some(rest) = key.as_str().strip_prefix("mind:") {
                let Some((actor, mind)) = rest.split_once(':') else {
                    return false;
                };
                return actor.parse::<u32>().is_ok_and(|a| {
                    mind.parse::<u64>()
                        .is_ok_and(|m| m > 0 && format!("mind:{a}:{m}") == key.as_str())
                });
            }
            let Some(record) = key.as_str().strip_prefix("reference:roll:commercial:") else {
                return false;
            };
            return RootAnchor::parse(record).is_ok_and(|anchor| {
                matches!(anchor, RootAnchor::RatingPending(_)) && anchor.key().key() == key.as_str()
            });
        }
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != NS {
            return false;
        }
        if key.as_str().starts_with("library:") {
            let Ok(row) = serde_json::from_value::<crate::Library>(value.clone()) else {
                return false;
            };
            return row.key() == key.as_str()
                && row.validate().is_ok()
                && row.next_mind >= 2
                && encoded_canonical(&row, value);
        }
        if key.as_str().starts_with("mind:") {
            let Ok(row) = serde_json::from_value::<crate::Mind>(value.clone()) else {
                return false;
            };
            // The only accepted mind image is a fresh attributed first draft.
            // Edits/save cannot enter through this retention extension.
            return row.key() == key.as_str()
                && row.validate().is_ok()
                && row.provenance.is_some()
                && row.edit == 0
                && row.next_revision == 1
                && row.head.is_none()
                && row.unnamed.is_empty()
                && row.named.is_empty()
                && encoded_canonical(&row, value);
        }
        let Ok(row) = serde_json::from_value::<ReferenceRecord>(value.clone()) else {
            return false;
        };
        if row.key.owner == NS && row.key.kind == ReferenceKind::UnnamedHistory {
            let Some((actor, mind)) = row.key.record.split_once('/') else {
                return false;
            };
            return actor.parse::<u32>().is_ok_and(|a| {
                mind.parse::<u64>().is_ok_and(|m| {
                    m > 0
                        && format!("{a}/{m}") == row.key.record
                        && row
                            .revisions
                            .iter()
                            .all(|r| r.key.character == mush_core::ObjId(a) && r.key.mind == m)
                })
            }) && row.key.key() == key.as_str()
                && row.validate().is_ok()
                && encoded_canonical(&row, value);
        }
        // A closed canonical family is necessary but not sufficient: only the
        // actual registered request may prepare these exact retained subjects.
        row.key.owner == NS
            && row.key.kind == ReferenceKind::Commercial
            && row.key.key() == key.as_str()
            && row.validate().is_ok()
            && RootAnchor::parse(&row.key.record).is_ok_and(|anchor| retains(&anchor))
            && serde_json::to_value(&row).is_ok_and(|canonical| canonical == *value)
    }
}

/// Every commercial anchor the root participant may write. Exhaustive with no
/// wildcard: a new `RootAnchor` variant does not compile until someone decides
/// here whether this participant retains it (Conduct jobs, IC claims and term
/// lapses were once added to the enum and missed from the list, and every
/// genuine admission that retained one was refused as a foreign write).
fn retains(anchor: &RootAnchor) -> bool {
    match anchor {
        RootAnchor::parse(&row.key.record)
        | RootAnchor::Filed(_)
        | RootAnchor::Term(_, _)
        | RootAnchor::OwnerParent(_, _)
        | RootAnchor::Operation(_)
        | RootAnchor::UpdateOperation(_)
        | RootAnchor::PaidOperation(_)
        | RootAnchor::PaidResaleFunding(_, _)
        | RootAnchor::PaidResaleTerminal(_, _)
        | RootAnchor::RenewalOperation(_)
        | RootAnchor::HouseRenewalOperation(_)
        | RootAnchor::Coverage(_, _)
        | RootAnchor::CoveragePublication(_, _, _)
        | RootAnchor::DraftRetirement(_, _)
        | RootAnchor::CharterPendingOffer(_, _)
        | RootAnchor::CharterPendingFunding(_, _)
        | RootAnchor::CharterPendingTerminal(_, _)
        | RootAnchor::CharterTransferOffer(_, _)
        | RootAnchor::CharterTransfer(_, _, _, _)
        | RootAnchor::AuthorWithdrawal(_, _)
        | RootAnchor::RegistryRecall(_, _)
        | RootAnchor::RegistryRevocation(_, _)
        | RootAnchor::OwnerQualification(_, _)
        | RootAnchor::OwnerAttestation(_, _)
        | RootAnchor::OwnerAttestationParent(_, _)
        | RootAnchor::ConductJob(_, _)
        | RootAnchor::IcClaim(_, _)
        | RootAnchor::TermLapse(_, _, _, _)
        | RootAnchor::RatingJob(_)
        | RootAnchor::RatingPending(_)
        | RootAnchor::Rating { .. } => true,
    }
}

fn refusal(_: crate::RollError) -> ParticipantError {
    // Diagnostics never expose source-owner keys or another actor's records.
    ParticipantError::refused("commercial retention is unavailable or stale")
}

impl BoundParticipant for BoundCommercialRoots<'_> {
    fn apply(self) {
        BoundCommercialRoots::apply(self);
    }
}

fn encoded_canonical<T: serde::Serialize>(row: &T, value: &serde_json::Value) -> bool {
    serde_json::to_value(row).is_ok_and(|canonical| canonical == *value)
        && super::usage::encoded_len(row).is_some()
}
