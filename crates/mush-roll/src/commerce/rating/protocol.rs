//! Closed in-process requests between the Roll producer and the two actual
//! Tacsim owner participants. None of these requests is a wire payload.
//!
//! Only this crate's registered producer constructs a request. Public readback
//! values are structural data, not certification: the producer accepts them
//! only as the return of PrepareCtx::prepare_owner on its exact typed handle.

use mush_core::{CommitToken, DefinitionDigest, account::PersonalAccountObservation};

use super::execution::{ExecutionAdmission, ExecutionObservation, MeasuredRating, RatingExecution};
use super::nonconformance::MeasuredRegistryResult;
use super::{RatingCompatibility, RatingError, RatingSubject};
use crate::commerce::RevisionIdentity;

/// A local bounded service ticket. The private owner must still resolve its
/// actual generation/actor/job; knowledge of this number grants nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketId(u64);
impl TicketId {
    pub fn new(value: u64) -> Result<Self, RatingError> {
        if value == 0 {
            return Err(RatingError::Invalid);
        }
        Ok(Self(value))
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The participant consumes this view. Constructing the enum outside Roll does
/// not construct a QuotaRequest or a registered participant handle.
pub enum QuotaOperation {
    Admit {
        ticket: TicketId,
        revision: RevisionIdentity,
        account: PersonalAccountObservation,
    },
    Grant {
        ticket: TicketId,
        before: RatingExecution,
    },
    Settle {
        ticket: TicketId,
        before: RatingExecution,
    },
    Cancel {
        ticket: TicketId,
        before: RatingExecution,
        account: PersonalAccountObservation,
    },
    Recover {
        ticket: TicketId,
        before: RatingExecution,
    },
}

pub struct QuotaRequest {
    operation: QuotaOperation,
}
impl QuotaRequest {
    pub fn into_operation(self) -> QuotaOperation {
        self.operation
    }
    pub(crate) fn admit(
        ticket: TicketId,
        revision: RevisionIdentity,
        account: PersonalAccountObservation,
    ) -> Self {
        Self {
            operation: QuotaOperation::Admit {
                ticket,
                revision,
                account,
            },
        }
    }
    pub(crate) fn grant(ticket: TicketId, before: RatingExecution) -> Self {
        Self {
            operation: QuotaOperation::Grant { ticket, before },
        }
    }
    pub(crate) fn settle(ticket: TicketId, before: RatingExecution) -> Self {
        Self {
            operation: QuotaOperation::Settle { ticket, before },
        }
    }
    pub(crate) fn cancel(
        ticket: TicketId,
        before: RatingExecution,
        account: PersonalAccountObservation,
    ) -> Self {
        Self {
            operation: QuotaOperation::Cancel {
                ticket,
                before,
                account,
            },
        }
    }
    pub(crate) fn recover(ticket: TicketId, before: RatingExecution) -> Self {
        Self {
            operation: QuotaOperation::Recover { ticket, before },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Admit,
    Grant,
    Settle,
    Cancel,
    Recover,
}
impl Transition {
    pub const fn reconformance_action_id(self) -> &'static str {
        match self {
            Self::Admit => "reconformance_admit",
            Self::Grant => "reconformance_grant",
            Self::Settle => "reconformance_settle",
            Self::Cancel => "reconformance_cancel",
            Self::Recover => "reconformance_recover",
        }
    }
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Admit => "rating_admit",
            Self::Grant => "rating_grant",
            Self::Settle => "rating_settle",
            Self::Cancel => "rating_cancel",
            Self::Recover => "rating_recover",
        }
    }
}

/// The real quota participant reports its exact checked before/after images.
/// It never returns a raw account identifier, a lease or a mutable owner handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaOutput {
    operation: CommitToken,
    job: CommitToken,
    transition: Transition,
    ticket: TicketId,
    admission: ExecutionAdmission,
    before: Option<ExecutionObservation>,
    after: ExecutionObservation,
    subject: Option<(RatingSubject, RatingCompatibility)>,
}
impl QuotaOutput {
    #[allow(clippy::too_many_arguments)]
    pub fn observed(
        operation: CommitToken,
        job: CommitToken,
        transition: Transition,
        ticket: TicketId,
        admission: ExecutionAdmission,
        before: Option<ExecutionObservation>,
        after: ExecutionObservation,
        subject: Option<(RatingSubject, RatingCompatibility)>,
    ) -> Result<Self, RatingError> {
        admission.validate()?;
        after.validate(admission.maximum_unit_ms)?;
        if let Some(before) = &before {
            before.validate(admission.maximum_unit_ms)?;
        }
        match (&subject, &before, transition) {
            (Some((subject, compatibility)), None, Transition::Admit) => {
                if operation != job
                    || subject.digest()? != admission.subject
                    || compatibility.digest() != admission.compatibility
                {
                    return Err(RatingError::Identity);
                }
            }
            (None, Some(_), transition) if transition != Transition::Admit => {
                if operation == job {
                    return Err(RatingError::Identity);
                }
            }
            _ => return Err(RatingError::Identity),
        }
        Ok(Self {
            operation,
            job,
            transition,
            ticket,
            admission,
            before,
            after,
            subject,
        })
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    pub const fn transition(&self) -> Transition {
        self.transition
    }
    pub const fn ticket(&self) -> TicketId {
        self.ticket
    }
    pub const fn admission(&self) -> &ExecutionAdmission {
        &self.admission
    }
    pub const fn before(&self) -> Option<&ExecutionObservation> {
        self.before.as_ref()
    }
    pub const fn after(&self) -> &ExecutionObservation {
        &self.after
    }
    pub fn subject(&self) -> Option<(&RatingSubject, &RatingCompatibility)> {
        self.subject.as_ref().map(|(s, c)| (s, c))
    }
    pub fn digest(&self) -> Result<DefinitionDigest, RatingError> {
        // Full actual structural observation, not a new certification identity.
        let value = serde_json::json!({
            "operation":self.operation,"job":self.job,"transition":self.transition.action_id(),
            "ticket":self.ticket.get(),"admission":self.admission,"before":self.before,
            "after":self.after,"subject":self.subject,
        });
        DefinitionDigest::for_canonical_value_with_limit(
            "roll.registry.quota-output.v1",
            &value,
            16 * 1024,
        )
        .map_err(|_| RatingError::Bound)
    }
}

/// Its sole input is the actual earlier quota participant output. This binds
/// private ticket consumption and artifact installation to the same operation.
pub struct ArtifactRequest {
    quota: QuotaOutput,
}
impl ArtifactRequest {
    pub(crate) fn from_quota(quota: &QuotaOutput) -> Self {
        Self {
            quota: quota.clone(),
        }
    }
    pub fn into_quota(self) -> QuotaOutput {
        self.quota
    }
}

/// Returned only through the exact Artifacts participant. Complete includes
/// the actual measured worker data whose raw artifact identity is rechecked.
pub struct ArtifactOutput {
    quota_digest: DefinitionDigest,
    measured: Option<MeasuredRegistryResult>,
}
impl ArtifactOutput {
    pub fn observed(
        quota: &QuotaOutput,
        measured: Option<MeasuredRating>,
    ) -> Result<Self, RatingError> {
        Self::observed_result(quota, measured.map(MeasuredRegistryResult::Certified))
    }
    pub fn observed_result(
        quota: &QuotaOutput,
        measured: Option<MeasuredRegistryResult>,
    ) -> Result<Self, RatingError> {
        if matches!(&measured, Some(MeasuredRegistryResult::Nonconforming(_)))
            && !quota.admission().measured_rejection
        {
            return Err(RatingError::Identity);
        }
        match (&measured, quota.after().phase) {
            (Some(result), super::execution::ExecutionPhase::Complete) => {
                if quota.transition() != Transition::Settle
                    || quota.after().result.as_ref() != Some(&result.artifact()?)
                {
                    return Err(RatingError::Identity);
                }
            }
            (None, phase)
                if phase != super::execution::ExecutionPhase::Complete
                    && quota.after().result.is_none() => {}
            _ => return Err(RatingError::Identity),
        }
        Ok(Self {
            quota_digest: quota.digest()?,
            measured,
        })
    }
    pub(crate) fn matches(&self, quota: &QuotaOutput) -> Result<bool, RatingError> {
        Ok(self.quota_digest == quota.digest()?)
    }
    pub(crate) fn into_measured(self) -> Option<MeasuredRegistryResult> {
        self.measured
    }
}

/// Absolute shared worker/queue ceiling (8 workers plus 128 queued jobs).
/// This bounds retained boot projection memory, not all loaded account visits.
pub const MAX_STOPPED_REGISTRY_JOBS: usize = 136;

/// Projection support implemented by the concrete quota owner selected during
/// trusted composition. No transport accepts this trait or an implementation.
pub trait RegistryQuotaParticipant:
    mush_core::participant::OwnerParticipant<Request = QuotaRequest, Output = QuotaOutput>
{
    /// Read-only final boot join over the actual private quota owner. The only
    /// readiness caller is Roll's private type-selected installer, which reads
    /// the actual typed rows itself. This returns no authority or health token.
    /// Implementations must inspect their inventory even when `executions` is
    /// empty and reject unavailable/quarantined or unmatched nonterminal work.
    fn audit_stopped_jobs(
        ctx: &mush_core::ModuleCtx<'_>,
        executions: &[super::execution::RatingExecution],
    ) -> Result<(), RatingError>;

    fn project_reconformance_selector(
        _engine: &mush_core::Engine,
        _query: &mush_core::LegalActionQuery,
        _transition: Transition,
    ) -> Result<Option<RegistrySelector>, RatingError> {
        Ok(None)
    }

    fn project_selector(
        engine: &mush_core::Engine,
        query: &mush_core::LegalActionQuery,
        transition: Transition,
    ) -> Result<Option<RegistrySelector>, RatingError>;
}
/// A structural selector, never a ticket constructor or participant output.
/// The actual quota owner derives it from a retained private ticket; preparation
/// still checks that exact ticket and every current owner fact.
pub struct RegistrySelector {
    ticket: TicketId,
    subject: SelectorSubject,
}
enum SelectorSubject {
    Reconform(crate::commerce::FiledRevisionId),
    Admit(crate::RevisionRef),
    Existing(CommitToken),
}
impl RegistrySelector {
    pub fn admission(ticket: TicketId, revision: crate::RevisionRef) -> Result<Self, RatingError> {
        if revision.key.mind == 0 || revision.key.number == 0 {
            return Err(RatingError::Invalid);
        }
        Ok(Self {
            ticket,
            subject: SelectorSubject::Admit(revision),
        })
    }
    pub const fn reconformance(ticket: TicketId, filed: crate::commerce::FiledRevisionId) -> Self {
        Self {
            ticket,
            subject: SelectorSubject::Reconform(filed),
        }
    }
    pub const fn existing(ticket: TicketId, job: CommitToken) -> Self {
        Self {
            ticket,
            subject: SelectorSubject::Existing(job),
        }
    }
    pub(crate) fn parameters(self) -> serde_json::Value {
        match self.subject {
            SelectorSubject::Reconform(filed) => {
                serde_json::json!({"ticket":self.ticket.get(),"filed":filed})
            }
            SelectorSubject::Admit(revision) => {
                serde_json::json!({"ticket":self.ticket.get(),"mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest})
            }
            SelectorSubject::Existing(job) => {
                serde_json::json!({"ticket":self.ticket.get(),"job":job})
            }
        }
    }
}
