//! Five closed action definitions. Cleanup drains retained admissions even
//! after worker policy changes; it never resumes or qualifies old computation.
use super::*;
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    OwnerId,
};
use serde::{Deserialize, Serialize};

use super::records::{ConductAdmission, ConductExecution, ConductMeasurement, ConductSubject};
use crate::commerce::rating::execution::{ExecutionObservation, ExecutionPhase};
use mush_core::{CommitToken, account::PersonalAccountObservation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketId(u64);
impl TicketId {
    pub fn new(id: u64) -> Result<Self, PublicationError> {
        if id == 0 {
            Err(PublicationError::Invalid)
        } else {
            Ok(Self(id))
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}
pub enum QuotaOperation {
    Admit {
        ticket: TicketId,
        subject: ConductSubject,
        account: PersonalAccountObservation,
    },
    Existing {
        ticket: TicketId,
        admission: ConductAdmission,
        before: ConductExecution,
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
        subject: ConductSubject,
        account: PersonalAccountObservation,
    ) -> Self {
        Self {
            operation: QuotaOperation::Admit {
                ticket,
                subject,
                account,
            },
        }
    }
    pub(crate) fn existing(
        ticket: TicketId,
        admission: ConductAdmission,
        before: ConductExecution,
    ) -> Self {
        Self {
            operation: QuotaOperation::Existing {
                ticket,
                admission,
                before,
            },
        }
    }
}

/// A structural projection accepted only from the exact registered quota
/// participant handle. This constructor grants no publication authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaOutput {
    operation: CommitToken,
    job: CommitToken,
    transition: Transition,
    ticket: TicketId,
    admission: ConductAdmission,
    before: Option<ExecutionObservation>,
    after: ExecutionObservation,
    subject: Option<ConductSubject>,
    at_utc: i64,
}
impl QuotaOutput {
    #[allow(clippy::too_many_arguments)]
    pub fn observed(
        operation: CommitToken,
        job: CommitToken,
        transition: Transition,
        ticket: TicketId,
        admission: ConductAdmission,
        before: Option<ExecutionObservation>,
        after: ExecutionObservation,
        subject: Option<ConductSubject>,
        at_utc: i64,
    ) -> Result<Self, PublicationError> {
        admission.validate()?;
        after
            .validate(admission.maximum_unit_ms())
            .map_err(invalid)?;
        if let Some(before) = &before {
            before
                .validate(admission.maximum_unit_ms())
                .map_err(invalid)?;
        }
        match (&before, &subject, transition) {
            (None, Some(subject), Transition::Admit) => {
                subject.validate()?;
                if operation != job
                    || after.phase != ExecutionPhase::Queued
                    || subject.worker_digest()? != admission.subject()
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            (Some(before), None, t) if t != Transition::Admit => {
                if operation == job || before.phase.terminal() {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
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
            at_utc,
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
    pub fn admission(&self) -> &ConductAdmission {
        &self.admission
    }
    pub fn before(&self) -> Option<&ExecutionObservation> {
        self.before.as_ref()
    }
    pub fn after(&self) -> &ExecutionObservation {
        &self.after
    }
    pub fn subject(&self) -> Option<&ConductSubject> {
        self.subject.as_ref()
    }
    pub const fn at_utc(&self) -> i64 {
        self.at_utc
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        digest(
            "roll.conduct.quota-output.v1",
            &(
                self.operation,
                self.job,
                self.transition,
                self.ticket.get(),
                &self.admission,
                &self.before,
                &self.after,
                &self.subject,
                self.at_utc,
            ),
            16 * 1024,
        )
    }
}
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
pub struct ArtifactOutput {
    quota: DefinitionDigest,
    measurement: Option<ConductMeasurement>,
}
impl ArtifactOutput {
    pub fn observed(
        quota: &QuotaOutput,
        measurement: Option<ConductMeasurement>,
    ) -> Result<Self, PublicationError> {
        match (&measurement, quota.after.phase) {
            (Some(m), ExecutionPhase::Complete) => {
                m.validate()?;
                if quota.transition != Transition::Settle
                    || quota.after.result.is_none()
                    || m.charged_unit_ms() != quota.after.charged_unit_ms
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            (None, p) if p != ExecutionPhase::Complete && quota.after.result.is_none() => {}
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(Self {
            quota: quota.digest()?,
            measurement,
        })
    }
    pub(crate) fn matches(&self, quota: &QuotaOutput) -> Result<(), PublicationError> {
        if self.quota == quota.digest()? {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        }
    }
    pub(crate) fn into_measurement(self) -> Option<ConductMeasurement> {
        self.measurement
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Admit,
    Grant,
    Settle,
    Cancel,
    Recover,
}
impl Transition {
    pub const ALL: [Self; 5] = [
        Self::Admit,
        Self::Grant,
        Self::Settle,
        Self::Cancel,
        Self::Recover,
    ];
    pub const fn word(self) -> &'static str {
        match self {
            Self::Admit => "admit",
            Self::Grant => "grant",
            Self::Settle => "settle",
            Self::Cancel => "cancel",
            Self::Recover => "recover",
        }
    }
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Admit => "conduct_trial_admit",
            Self::Grant => "conduct_trial_grant",
            Self::Settle => "conduct_trial_settle",
            Self::Cancel => "conduct_trial_cancel",
            Self::Recover => "conduct_trial_recover",
        }
    }
    pub(crate) fn from_action(action: &DefinitionRef) -> Result<Self, PublicationError> {
        Self::ALL
            .into_iter()
            .find(|t| {
                action.key().owner.as_str() == "roll"
                    && action.key().kind.as_str() == "action"
                    && action.key().id.as_str() == t.action_id()
                    && action.version() == 1
            })
            .ok_or(PublicationError::Mismatch)
    }
}
pub fn action_descriptor(
    manifest: &DefinitionRef,
    transition: Transition,
) -> Result<DefinitionDescriptor, PublicationError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "conduct_manifest"
        || manifest.key().id.as_str() != "safe_conduct"
        || manifest.version() != 1
    {
        return Err(PublicationError::Invalid);
    }
    let cleanup = matches!(transition, Transition::Cancel | Transition::Recover);
    let payload = if cleanup {
        serde_json::json!({"format":1,"protocol":"roll.conduct_trial.v1","transition":transition,"cleanup":true})
    } else {
        serde_json::json!({"format":1,"protocol":"roll.conduct_trial.v1","transition":transition,"manifest":manifest})
    };
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(invalid)?,
        DefinitionId::new(transition.action_id()).map_err(invalid)?,
        1,
        payload,
    )
    .map_err(invalid)?;
    Ok(if cleanup {
        descriptor
    } else {
        descriptor.with_references(vec![manifest.clone()])
    })
}
pub fn action_ref(
    manifest: &DefinitionRef,
    transition: Transition,
) -> Result<DefinitionRef, PublicationError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("roll").map_err(invalid)?,
            action_descriptor(manifest, transition)?,
        )
        .map_err(invalid)
}
pub(crate) fn verify_action(
    action: &DefinitionRef,
    manifest: &DefinitionRef,
    transition: Transition,
) -> Result<(), PublicationError> {
    if *action == action_ref(manifest, transition)? {
        Ok(())
    } else {
        Err(PublicationError::Mismatch)
    }
}

/// Structural typed inputs to the actual native owner's comparison. Constructing
/// this view conveys no worker, quota or qualification authority.
pub struct ConductHistoryRow<'a> {
    pub job: &'a super::records::ConductJob,
    pub execution: &'a super::records::ConductExecution,
    pub result: Option<&'a super::records::ConductResult>,
}
/// Selected only by trusted composition alongside its real participant handle.
/// Neither request parameters nor stored rows select an implementation.
pub trait ConductQuotaParticipant:
    mush_core::participant::OwnerParticipant<Request = QuotaRequest, Output = QuotaOutput>
{
    /// Must compare the entire permanent native inventory, even for zero rows.
    /// Returns no health setter or authority token; Roll owns readiness.
    fn audit_history(
        ctx: &mush_core::ModuleCtx<'_>,
        rows: &[ConductHistoryRow<'_>],
    ) -> Result<(), PublicationError>;
    /// Constant-size runtime join against the actual loaded/applied native row.
    fn check_history(
        world: &mush_core::World,
        row: ConductHistoryRow<'_>,
    ) -> Result<(), PublicationError>;
    fn project_selector(
        engine: &mush_core::Engine,
        query: &mush_core::LegalActionQuery,
        transition: Transition,
    ) -> Result<Option<ConductSelector>, PublicationError>;
}
/// A structural selector from a private ticket. It creates no ticket or output.
pub struct ConductSelector {
    ticket: TicketId,
    subject: SelectorSubject,
}
enum SelectorSubject {
    Admission(super::ConductSubject),
    Existing(CommitToken),
}
impl ConductSelector {
    pub fn admission(ticket: TicketId, subject: super::ConductSubject) -> Self {
        Self {
            ticket,
            subject: SelectorSubject::Admission(subject),
        }
    }
    pub fn existing(ticket: TicketId, operation: CommitToken) -> Self {
        Self {
            ticket,
            subject: SelectorSubject::Existing(operation),
        }
    }
    pub(crate) fn parameters(self) -> serde_json::Value {
        match self.subject {
            SelectorSubject::Admission(subject) => {
                serde_json::json!({"ticket":self.ticket.get(),"filed":subject.filed(),"profile":subject.profile(),"role":subject.role()})
            }
            SelectorSubject::Existing(job) => {
                serde_json::json!({"ticket":self.ticket.get(),"job":job})
            }
        }
    }
}
