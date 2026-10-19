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
    TermLapse(super::facts::publication::Lapse),
    Ic(super::ic_claims::roots::IcRoots),
    InitialFiling(InitialRoots),
    CompatibleUpdate(super::line_update::publication::UpdateRoots),
    DependentFiling(super::dependent_filing::DependentRoots),
    Rating(RatingRoots),
    Paid(PaidRoots),
    Entitlement(super::entitlement::roots::EntitlementRoots),
    House(super::house_purchase::roots::HouseRoots),
    Renewal(super::renewal::roots::RenewalRoots),
    HouseRenewal(super::house_renewal::roots::RenewalRoots),
    AuthoredFork(super::authored_fork::AuthoredFork),
    Retirement(super::authored_retirement::Retirement),
    Withdrawal(super::withdrawal::Withdrawal),
    CharterTransfer(super::charter_transfer::TransferRoots),
    CharterPending(super::charter_pending::publication::PendingRoots),
    RegistryRecall(super::registry_recall::Recall),
    MeasuredRecall(super::rating::reconformance::recall::publication::Recall),
    RegistryRevocation(super::registry_revocation::Revocation),
    OwnerQualification(super::owner_qualification::Qualification),
    OwnerAttestation(super::owner_attestation::Attestation),
}

impl RootRequest {
    fn commit(&self) -> CommitToken {
        match self {
            Self::TermLapse(request) => request.commit(),
            Self::Ic(request) => request.commit(),
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
    term_lapse: Option<super::facts::publication::LapseFence>,
    commit: CommitToken,
    ic: Option<super::ic_claims::roots::IcFence>,
    change: PreparedChange,
    fresh_paid: Option<super::sale_reader::OriginalSaleFacts>,
    entitlement: Option<super::entitlement::roots::EntitlementFence>,
    house: Option<super::house_purchase::roots::HouseFence>,
    renewal: Option<super::renewal::roots::RenewalFence>,
    house_renewal: Option<super::house_renewal::roots::RenewalFence>,
    dependent: Option<super::dependent_filing::FilingFence>,
    update: Option<super::line_update::publication::UpdateFence>,
    marked: Option<super::marked_rating::AdmissionFence>,
    reconformance: Option<super::rating::reconformance::fence::ServiceFence>,
    retirement: Option<super::authored_retirement::RetirementFence>,
    withdrawal: Option<super::withdrawal::WithdrawalFence>,
    charter_transfer: Option<super::charter_transfer::TransferFence>,
    charter_pending: Option<super::charter_pending::publication::PendingFence>,
    registry_recall: Option<super::registry_recall::RecallFence>,
    measured_recall: Option<super::rating::reconformance::recall::publication::RecallFence>,
    registry_revocation: Option<super::registry_revocation::RevocationFence>,
    qualification: Option<super::owner_qualification::QualificationFence>,
    attestation: Option<super::owner_attestation::AttestationFence>,
    fork: Option<(
        mush_core::DefinitionRef,
        super::parent_grant::CurrentParentGrant,
    )>,
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
        let term_lapse = match &request {
            RootRequest::TermLapse(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let ic = match &request {
            RootRequest::Ic(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let dependent = match &request {
            RootRequest::DependentFiling(request) => {
                request
                    .fence()
                    .check(request.commit(), context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
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
        let fresh_paid = match &request {
            RootRequest::Paid(paid) => paid.fresh().cloned(),
            _ => None,
        };
        super::paid_roots::check_fresh(fresh_paid.as_ref(), context)
            .map_err(|_| ParticipantError::Authority)?;
        let reconformance = match &request {
            RootRequest::Rating(request) => request.reconformance().cloned(),
            _ => None,
        };
        if let Some(fence) = &reconformance {
            fence
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        let marked = match &request {
            RootRequest::Rating(request) => request.marked().cloned(),
            _ => None,
        };
        if let Some(marked) = &marked {
            marked
                .check(context)
                .map_err(|_| ParticipantError::Authority)?;
        }
        let fork = match &request {
            RootRequest::AuthoredFork(request) => {
                request
                    .check_context(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some((request.action().clone(), request.grant().clone()))
            }
            _ => None,
        };
        let retirement = match &request {
            RootRequest::Retirement(request) => {
                request
                    .check_context(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence())
            }
            _ => None,
        };
        let entitlement = match &request {
            RootRequest::Entitlement(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let house = match &request {
            RootRequest::House(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let renewal = match &request {
            RootRequest::Renewal(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let house_renewal = match &request {
            RootRequest::HouseRenewal(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let registry_recall = match &request {
            RootRequest::RegistryRecall(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let measured_recall = match &request {
            RootRequest::MeasuredRecall(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let registry_revocation = match &request {
            RootRequest::RegistryRevocation(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let charter_pending = match &request {
            RootRequest::RegistryRevocation(request) => request
                .pending()
                .map(|p| {
                    p.fence()
                        .check(context)
                        .map_err(|_| ParticipantError::Authority)?;
                    Ok(p.fence().clone())
                })
                .transpose()?,
            RootRequest::CharterPending(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let charter_transfer = match &request {
            RootRequest::CharterTransfer(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let withdrawal = match &request {
            RootRequest::Withdrawal(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let attestation = match &request {
            RootRequest::OwnerAttestation(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let qualification = match &request {
            RootRequest::OwnerQualification(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let update = match &request {
            RootRequest::CompatibleUpdate(request) => {
                request
                    .fence()
                    .check(context)
                    .map_err(|_| ParticipantError::Authority)?;
                Some(request.fence().clone())
            }
            _ => None,
        };
        let mut retired = None;
        let (change, selection) = match &request {
            RootRequest::TermLapse(request) => (
                state.prepare_term_lapse_root(request).map_err(refusal)?,
                None,
            ),
            RootRequest::Ic(request) => (
                state.prepare_ic_claim_roots(request).map_err(refusal)?,
                None,
            ),
            RootRequest::InitialFiling(request) => (
                state.prepare_commercial_roots(request).map_err(refusal)?,
                None,
            ),
            RootRequest::CompatibleUpdate(request) => (
                state.prepare_line_update_roots(request).map_err(refusal)?,
                None,
            ),
            RootRequest::DependentFiling(request) => (
                state
                    .prepare_commercial_roots(request.roots())
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::Rating(request) => {
                (state.prepare_rating_roots(request).map_err(refusal)?, None)
            }
            RootRequest::Paid(request) => {
                (state.prepare_paid_roots(request).map_err(refusal)?, None)
            }
            RootRequest::Entitlement(request) => (
                state.prepare_entitlement_roots(request).map_err(refusal)?,
                None,
            ),
            RootRequest::House(request) => {
                (state.prepare_house_roots(request).map_err(refusal)?, None)
            }
            RootRequest::Renewal(request) => {
                (state.prepare_renewal_roots(request).map_err(refusal)?, None)
            }
            RootRequest::HouseRenewal(request) => (
                state
                    .prepare_house_renewal_roots(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::OwnerAttestation(request) => (
                state
                    .prepare_owner_attestation_roots(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::OwnerQualification(request) => (
                state
                    .prepare_owner_qualification_root(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::RegistryRecall(request) => (
                state
                    .prepare_registry_recall_root(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::MeasuredRecall(request) => (
                state
                    .prepare_measured_recall_root(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::RegistryRevocation(request) => (
                state
                    .prepare_registry_revocation_root(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::CharterPending(request) => (
                state
                    .prepare_charter_pending_roots(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::CharterTransfer(request) => (
                state
                    .prepare_charter_transfer_root(request)
                    .map_err(refusal)?,
                None,
            ),
            RootRequest::Withdrawal(request) => (
                state.prepare_withdrawal_root(request).map_err(refusal)?,
                None,
            ),
            RootRequest::Retirement(request) => {
                let (change, output) = state
                    .prepare_authored_retirement(request)
                    .map_err(refusal)?;
                retired = Some(output);
                (change, None)
            }
            RootRequest::AuthoredFork(request) => {
                let (change, selection) = state.prepare_authored_fork(request).map_err(refusal)?;
                (change, Some(selection))
            }
        };
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
                term_lapse,
                commit: request.commit(),
                ic,
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
        super::paid_roots::check_fresh(patch.fresh_paid.as_ref(), context)
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
        if let Some((action, grant)) = &patch.fork {
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
            && matches!(
                RootAnchor::parse(&row.key.record),
                Ok(RootAnchor::Filed(_)
                    | RootAnchor::Term(_, _)
                    | RootAnchor::OwnerParent(_, _)
                    | RootAnchor::Operation(_)
                    | RootAnchor::UpdateOperation(_)
                    | RootAnchor::PaidOperation(_)
                    | RootAnchor::RenewalOperation(_)
                    | RootAnchor::HouseRenewalOperation(_)
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
                    | RootAnchor::RatingJob(_)
                    | RootAnchor::RatingPending(_)
                    | RootAnchor::Rating { .. })
            )
            && serde_json::to_value(&row).is_ok_and(|canonical| canonical == *value)
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
