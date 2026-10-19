//! Durable observation of the real registered quota/artifact owners.
//!
//! These strict values are not a second quota owner or a certification token.
//! Only the Roll producer may stage them, after obtaining matching outputs from
//! the actual registered BulkStore and Artifacts participants. They preserve
//! completed evidence when the bounded bulk history later evicts its run.

use mush_core::{AccountId, CommitToken, ObjId, mechanics::DefinitionDigest};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest as _, Sha256};

use super::{MAX_CASES, MAX_PAGES, RatingError, RatingJob, RatingJobState};

pub const EXECUTION_BYTES: usize = 4 * 1024;
pub const START_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    pub sha256: DefinitionDigest,
    pub bytes: u64,
}
impl ArtifactIdentity {
    pub fn validate(&self) -> Result<(), RatingError> {
        if self.bytes == 0 || self.bytes > START_BYTES {
            return Err(RatingError::Bound);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCursor {
    pub index: u16,
    pub simulated_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Queued,
    Running,
    CancelRequested,
    Complete,
    Failed,
    Cancelled,
    Interrupted,
}
impl ExecutionPhase {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidFlight {
    pub sequence: u64,
    pub utc_day: i64,
    pub allowance_unit_ms: u64,
    pub units: u16,
    pub native_ms: u32,
    pub cursor: ExecutionCursor,
}
impl PaidFlight {
    fn validate(&self) -> Result<(), RatingError> {
        let maximum = u64::from(self.units)
            .checked_mul(u64::from(self.native_ms))
            .and_then(|n| n.checked_mul(8))
            .ok_or(RatingError::Bound)?;
        if self.sequence == 0
            || !(1..=33).contains(&self.units)
            || self.native_ms < 50
            || self.allowance_unit_ms == 0
            || self.allowance_unit_ms > maximum
            || !self.allowance_unit_ms.is_multiple_of(u64::from(self.units))
            || usize::from(self.cursor.index) >= MAX_CASES
        {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
}

/// The owner-produced before/after run projection. No wire endpoint accepts it.
/// Account quota arithmetic remains entirely in BulkStore::prepare_*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionObservation {
    pub phase: ExecutionPhase,
    pub next_sequence: u64,
    pub cursor: ExecutionCursor,
    pub charged_unit_ms: u64,
    #[serde(deserialize_with = "super::required_option")]
    pub flight: Option<PaidFlight>,
    #[serde(deserialize_with = "super::required_option")]
    pub result: Option<ArtifactIdentity>,
}
impl ExecutionObservation {
    pub fn validate(&self, admitted_maximum: u64) -> Result<(), RatingError> {
        if self.next_sequence == 0
            || self.charged_unit_ms > admitted_maximum
            || usize::from(self.cursor.index) >= MAX_CASES
            || (self.phase.terminal() && self.flight.is_some())
            || (self.phase == ExecutionPhase::Complete && self.result.is_none())
            || (self.phase != ExecutionPhase::Complete && self.result.is_some())
        {
            return Err(RatingError::Invalid);
        }
        if let Some(result) = &self.result {
            result.validate()?;
            if result.bytes > super::MAX_RESULT_BYTES as u64 {
                return Err(RatingError::Bound);
            }
        }
        if let Some(flight) = &self.flight {
            flight.validate()?;
            if flight.sequence.checked_add(1) != Some(self.next_sequence)
                || flight.cursor != self.cursor
                || flight.allowance_unit_ms > self.charged_unit_ms
                || !matches!(
                    self.phase,
                    ExecutionPhase::Running | ExecutionPhase::CancelRequested
                )
            {
                return Err(RatingError::Identity);
            }
        }
        if self.phase == ExecutionPhase::Queued
            && (self.next_sequence != 1
                || self.cursor != ExecutionCursor::default()
                || self.charged_unit_ms != 0
                || self.flight.is_some()
                || self.result.is_some())
        {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
}

/// Domain-separated, job-scoped account binding computed from the actual Bulk
/// participant's private account identity. This pure calculation grants no
/// authority; no request may substitute its own digest for the real output.
/// Raw AccountId remains in the existing private quota/artifact persistence.
pub fn job_account_binding(
    account: AccountId,
    job: CommitToken,
    run: u64,
    nonce: u64,
) -> Result<DefinitionDigest, RatingError> {
    if run == 0 || nonce == 0 {
        return Err(RatingError::Invalid);
    }
    super::digest(
        "roll.registry.job-account.v1",
        &(account, job, run, nonce),
        4096,
    )
}

/// Exact immutable admission inputs derived from the actual two participant
/// outputs and current owned immutable revision. Account evidence is scoped to
/// this exact job; a global account identifier is not copied into business rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAdmission {
    pub job_account_binding: DefinitionDigest,
    pub character: ObjId,
    pub run: u64,
    pub nonce: u64,
    pub subject: DefinitionDigest,
    pub compatibility: DefinitionDigest,
    pub plan: DefinitionDigest,
    pub budget: DefinitionDigest,
    pub start: ArtifactIdentity,
    pub maximum_unit_ms: u64,
    pub cases: u16,
    pub maximum_pages: u16,
    // Absent for historical admissions: their exact future-key promise is unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub measured_rejection: bool,
    #[serde(
        default,
        deserialize_with = "present_reconformance",
        skip_serializing_if = "Option::is_none"
    )]
    pub reconformance: Option<super::reconformance::ReconformanceAdmission>,
}
fn present_reconformance<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<super::reconformance::ReconformanceAdmission>, D::Error> {
    super::reconformance::ReconformanceAdmission::deserialize(d).map(Some)
}
fn is_false(value: &bool) -> bool {
    !value
}
impl ExecutionAdmission {
    pub fn validate(&self) -> Result<(), RatingError> {
        self.start.validate()?;
        if let Some(binding) = &self.reconformance {
            binding.validate()?;
            if self.character != mush_core::GOD || !self.measured_rejection {
                return Err(RatingError::Identity);
            }
        }
        if self.character.0 == 0
            || self.run == 0
            || self.nonce == 0
            || self.maximum_unit_ms == 0
            || self.cases == 0
            || usize::from(self.cases) > MAX_CASES
            || self.maximum_pages == 0
            || usize::from(self.maximum_pages) > MAX_PAGES
            || usize::from(self.maximum_pages)
                != usize::from(self.cases).div_ceil(super::MAX_PAGE_ROWS)
        {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
    fn matches_job(&self, job: &RatingJob) -> Result<(), RatingError> {
        let correct_actor = match &self.reconformance {
            None => self.character == job.subject().revision().revision().key.character,
            Some(binding) => {
                self.character == mush_core::GOD
                    && binding.birth_rating().job() != job.job()
                    && binding.previous_attempt() != Some(job.job())
            }
        };
        if !correct_actor
            || self.subject != job.subject().digest()?
            || self.compatibility != job.compatibility().digest()
            || self.plan != job.plan()
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RatingExecution {
    format: u16,
    job: CommitToken,
    admission: ExecutionAdmission,
    observation: ExecutionObservation,
    updated_by: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionWire {
    format: u16,
    job: CommitToken,
    admission: ExecutionAdmission,
    observation: ExecutionObservation,
    updated_by: CommitToken,
}
impl<'de> Deserialize<'de> for RatingExecution {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = ExecutionWire::deserialize(decoder)?;
        let row = Self {
            format: wire.format,
            job: wire.job,
            admission: wire.admission,
            observation: wire.observation,
            updated_by: wire.updated_by,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl RatingExecution {
    pub(crate) fn admit(
        job: &RatingJob,
        facts: ExecutionAdmission,
        observed: ExecutionObservation,
    ) -> Result<Self, RatingError> {
        if !matches!(job.state(), RatingJobState::Queued {})
            || observed.phase != ExecutionPhase::Queued
        {
            return Err(RatingError::Invalid);
        }
        let row = Self {
            format: super::FORMAT,
            job: job.job(),
            admission: facts,
            observation: observed,
            updated_by: job.job(),
        };
        row.validate()?;
        row.matches_job(job)?;
        Ok(row)
    }
    /// Caller has reconciled an actual owner output, including exact receipt,
    /// before-state and charged progress. This does not perform quota arithmetic.
    pub(crate) fn observed(
        &self,
        before: &ExecutionObservation,
        after: ExecutionObservation,
        job: &RatingJob,
        commit: CommitToken,
    ) -> Result<Self, RatingError> {
        if before != &self.observation
            || before.phase.terminal()
            || commit == self.updated_by
            || after.next_sequence < before.next_sequence
            || after.next_sequence
                > before
                    .next_sequence
                    .checked_add(1)
                    .ok_or(RatingError::Bound)?
        {
            return Err(RatingError::Identity);
        }
        let row = Self {
            observation: after,
            updated_by: commit,
            ..self.clone()
        };
        row.validate()?;
        row.matches_job(job)?;
        Ok(row)
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        if self.format != super::FORMAT {
            return Err(RatingError::Invalid);
        }
        self.admission.validate()?;
        self.observation.validate(self.admission.maximum_unit_ms)?;
        if self.observation.cursor.index >= self.admission.cases
            || ((self.observation.phase == ExecutionPhase::Queued) != (self.updated_by == self.job))
        {
            return Err(RatingError::Identity);
        }
        super::encoded(self, EXECUTION_BYTES - super::TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    pub(crate) fn matches_job(&self, job: &RatingJob) -> Result<(), RatingError> {
        self.admission.matches_job(job)?;
        if self.job != job.job() {
            return Err(RatingError::Identity);
        }
        let agrees = match (job.state(), self.observation.phase) {
            (RatingJobState::Queued {}, ExecutionPhase::Queued) => true,
            (
                RatingJobState::Running { lease_sequence },
                ExecutionPhase::Running | ExecutionPhase::CancelRequested,
            ) => lease_sequence.checked_add(1) == Some(self.observation.next_sequence),
            (RatingJobState::Completed { receipt }, ExecutionPhase::Complete) => {
                receipt.job() == self.job
            }
            (RatingJobState::Nonconforming { receipt }, ExecutionPhase::Complete) => {
                self.admission.measured_rejection && receipt.job() == self.job
            }
            (RatingJobState::Failed { .. }, ExecutionPhase::Failed)
            | (RatingJobState::Cancelled {}, ExecutionPhase::Cancelled)
            | (RatingJobState::Interrupted {}, ExecutionPhase::Interrupted) => true,
            _ => false,
        };
        if !agrees {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    pub const fn admission(&self) -> &ExecutionAdmission {
        &self.admission
    }
    pub const fn observation(&self) -> &ExecutionObservation {
        &self.observation
    }
    pub const fn updated_by(&self) -> CommitToken {
        self.updated_by
    }
    pub fn digest(&self) -> Result<DefinitionDigest, RatingError> {
        self.validate()?;
        super::digest("roll.registry.execution.v1", self, EXECUTION_BYTES)
    }

    pub(crate) fn verify_nonconformance(
        &self,
        job: &RatingJob,
        receipt: &super::nonconformance::RatingNonconformance,
        pages: &[super::RatingPage],
    ) -> Result<(), RatingError> {
        self.validate()?;
        self.matches_job(job)?;
        if !self.admission.measured_rejection
            || self.observation.phase != ExecutionPhase::Complete
            || !job.matches_nonconformance(receipt)?
            || receipt.result().cases() > self.admission.cases
            || pages.len() > usize::from(self.admission.maximum_pages)
            || self.observation.result.as_ref()
                != Some(
                    &super::nonconformance::MeasuredNonconformance::from_receipt(receipt, pages)?
                        .artifact()?,
                )
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }

    pub(crate) fn verify_completion(
        &self,
        job: &RatingJob,
        receipt: &super::RatingReceipt,
        pages: &[super::RatingPage],
    ) -> Result<(), RatingError> {
        self.validate()?;
        self.matches_job(job)?;
        if self.observation.phase != ExecutionPhase::Complete
            || !job.matches_receipt(receipt)?
            || receipt.result().cases() > self.admission.cases
            || pages.len() > usize::from(self.admission.maximum_pages)
            || self.observation.result.as_ref()
                != Some(&MeasuredRating::from_receipt(receipt, pages)?.artifact()?)
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commerce::rating::{RatingClass, RatingCompatibility, RatingSubject, test_support};
    use mush_core::{ClockRef, ClockStamp};
    use serde_json::json;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn complete() -> TestResult<test_support::CompleteRating> {
        let digest = DefinitionDigest::from_bytes([1; 32]);
        let revision = serde_json::from_value(json!({"format":1,"revision":{
            "key":{"character":7,"mind":1,"number":1},"digest":digest},"source":digest}))?;
        let subject = RatingSubject::new(revision, digest, digest, digest, digest, RatingClass::I);
        Ok(test_support::complete_rating(
            subject,
            RatingCompatibility::combat_v1(digest),
            digest,
            CommitToken::from_bytes([3; 16]),
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 100,
            },
        )?)
    }

    #[test]
    fn account_evidence_is_job_scoped_and_never_serializes_the_global_identity() -> TestResult {
        let account = AccountId::mint("structural-rating-fixture", 1)?;
        let job = CommitToken::from_bytes([3; 16]);
        let binding = job_account_binding(account, job, 1, 1)?;
        assert_ne!(
            binding,
            job_account_binding(account, CommitToken::from_bytes([4; 16]), 1, 1)?
        );
        assert_ne!(binding, job_account_binding(account, job, 2, 1)?);
        assert_ne!(binding, job_account_binding(account, job, 1, 2)?);
        assert_ne!(
            binding,
            job_account_binding(AccountId::mint("other", 1)?, job, 1, 1)?
        );
        let execution = complete()?.execution;
        assert_eq!(execution.admission.job_account_binding, binding);
        let raw = serde_json::to_string(&account)?;
        let wire = serde_json::to_string(&execution)?;
        assert!(!wire.contains(&raw));
        assert!(wire.contains("job_account_binding"));
        assert!(job_account_binding(account, job, 0, 1).is_err());
        Ok(())
    }

    #[test]
    fn observed_result_binds_the_actual_worker_encoding_and_receipt_pages() -> TestResult {
        let evidence = complete()?;
        let image = MeasuredRating::from_receipt(&evidence.receipt, &evidence.pages)?;
        #[derive(Serialize)]
        struct PreviousWorkerWire<'a> {
            content: &'a super::super::CertificateContent,
            pages: Vec<&'a super::super::PageContent>,
        }
        let old = PreviousWorkerWire {
            content: &evidence.receipt.content,
            pages: evidence.pages.iter().map(|page| page.content()).collect(),
        };
        assert_eq!(serde_json::to_vec(&old)?, serde_json::to_vec(&image)?);
        assert_eq!(
            evidence.execution.observation.result,
            Some(image.artifact()?)
        );
        evidence
            .execution
            .verify_completion(&evidence.job, &evidence.receipt, &evidence.pages)?;
        let mut crossed = evidence.execution.clone();
        crossed
            .observation
            .result
            .as_mut()
            .ok_or(RatingError::Incomplete)?
            .sha256 = DefinitionDigest::from_bytes([99; 32]);
        assert!(
            crossed
                .verify_completion(&evidence.job, &evidence.receipt, &evidence.pages)
                .is_err()
        );
        assert!(!evidence.completion.matches(&evidence.receipt, &crossed)?);
        let mut different_run = evidence.execution.clone();
        different_run.admission.run += 1;
        assert!(
            !evidence
                .completion
                .matches(&evidence.receipt, &different_run)?
        );
        Ok(())
    }

    #[test]
    fn completion_cannot_expand_the_admitted_case_count_within_one_page() -> TestResult {
        use super::super::{
            CaseKey, CaseKind, ClassSummary, PageContent, ProbeName, RatingPage, RatingReceipt,
            RatingResultRef, Rows,
        };
        let original = complete()?;
        let first = original
            .pages
            .first()
            .ok_or(RatingError::Incomplete)?
            .content()
            .rows()
            .first()
            .ok_or(RatingError::Incomplete)?
            .clone();
        let mut second = first.clone();
        second.key = CaseKey::new(
            RatingClass::I,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::Hostile450Armed,
            },
        )?;
        let rows = vec![first, second];
        let summary = ClassSummary::from_cases(RatingClass::I, &rows.iter().collect::<Vec<_>>())?;
        let page = PageContent::new(
            original.receipt.subject().digest()?,
            original.receipt.compatibility().digest(),
            0,
            rows,
        )?;
        let mut content = original.receipt.content.clone();
        content.result = RatingResultRef::from_pages(std::slice::from_ref(&page))?;
        content.classes = Rows::new(vec![summary])?;
        let receipt = RatingReceipt::new(content, original.receipt.publication.clone())?;
        let pages = vec![RatingPage::new(
            receipt.publication().job(),
            receipt.id(),
            receipt.result().id(),
            page,
        )?];
        receipt.verify_pages(&pages)?;
        let job = RatingJob::new(
            receipt.publication().job(),
            receipt.subject().clone(),
            receipt.compatibility().clone(),
            receipt.plan(),
        )
        .transition(RatingJobState::Running { lease_sequence: 1 })?
        .transition(RatingJobState::Completed {
            receipt: receipt.reference()?,
        })?;
        let mut execution = test_support::completed_execution(&receipt, &pages)?;
        assert_eq!(execution.admission.maximum_pages, 1);
        execution.verify_completion(&job, &receipt, &pages)?;
        execution.admission.cases = 1;
        assert!(execution.verify_completion(&job, &receipt, &pages).is_err());
        execution.admission.cases = 3;
        execution.verify_completion(&job, &receipt, &pages)?;
        Ok(())
    }

    #[test]
    fn terminal_capacity_is_exactly_job_keyed_and_includes_both_existing_images() -> TestResult {
        let mut row = complete()?.execution;
        row.admission.cases = u16::try_from(MAX_CASES)?;
        row.admission.maximum_pages = u16::try_from(MAX_PAGES)?;
        let targets = row.terminal_targets()?;
        assert_eq!(targets.len(), 122);
        assert_eq!(targets.iter().filter(|target| target.existing).count(), 2);
        assert_eq!(
            targets
                .iter()
                .filter(|target| target.family == "rating_pages")
                .count(),
            117
        );
        assert!(
            targets
                .windows(2)
                .all(|pair| matches!(pair, [a,b] if (&a.family,&a.key) < (&b.family,&b.key)))
        );
        assert!(
            targets
                .iter()
                .all(|target| target.key.starts_with(&row.job.to_string()))
        );
        assert_eq!(
            targets.iter().map(|target| target.ceiling).sum::<usize>(),
            3_915_776
        );
        assert!(
            serde_json::to_vec(&row)?.len() + super::super::TYPED_ENVELOPE_RESERVE
                <= EXECUTION_BYTES
        );
        Ok(())
    }

    #[test]
    fn unsuccessful_terminal_execution_cannot_claim_a_result_artifact() -> TestResult {
        let evidence = complete()?;
        for phase in [
            ExecutionPhase::Failed,
            ExecutionPhase::Cancelled,
            ExecutionPhase::Interrupted,
        ] {
            let mut execution = evidence.execution.clone();
            execution.observation.phase = phase;
            assert!(
                execution
                    .observation
                    .validate(execution.admission.maximum_unit_ms)
                    .is_err()
            );
            assert!(
                serde_json::from_value::<RatingExecution>(serde_json::to_value(&execution)?)
                    .is_err()
            );
            execution.observation.result = None;
            execution
                .observation
                .validate(execution.admission.maximum_unit_ms)?;
            assert_eq!(
                serde_json::from_value::<RatingExecution>(serde_json::to_value(&execution)?)?,
                execution
            );
        }
        Ok(())
    }

    #[test]
    fn execution_decode_refuses_missing_payment_fields_and_incoherent_phases() -> TestResult {
        let evidence = complete()?;
        let wire = serde_json::to_value(&evidence.execution)?;
        assert_eq!(
            serde_json::from_value::<RatingExecution>(wire.clone())?,
            evidence.execution
        );
        for key in ["flight", "result"] {
            let mut bad = wire.clone();
            bad.get_mut("observation")
                .ok_or(RatingError::Incomplete)?
                .as_object_mut()
                .ok_or(RatingError::Incomplete)?
                .remove(key);
            assert!(serde_json::from_value::<RatingExecution>(bad).is_err());
        }
        let mut bad = wire.clone();
        bad.as_object_mut()
            .ok_or(RatingError::Incomplete)?
            .insert("authority".into(), json!(true));
        assert!(serde_json::from_value::<RatingExecution>(bad).is_err());
        let mut bad = wire;
        bad.as_object_mut()
            .ok_or(RatingError::Incomplete)?
            .insert("updated_by".into(), json!(evidence.execution.job()));
        assert!(serde_json::from_value::<RatingExecution>(bad).is_err());
        assert!(
            evidence
                .execution
                .observed(
                    evidence.execution.observation(),
                    evidence.execution.observation().clone(),
                    &evidence.job,
                    CommitToken::from_bytes([8; 16])
                )
                .is_err(),
            "terminal observations are immutable"
        );
        Ok(())
    }
}

/// The actual worker's bounded output encoding. It is shared with the reader so
/// a completed execution's raw artifact SHA binds precisely these receipt pages,
/// not only a separately supplied subject digest. It is still structural data;
/// constructors do not authorize publication or certify that any case ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MeasuredRating {
    content: super::CertificateContent,
    pages: super::Rows<super::PageContent, MAX_PAGES>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasuredWire {
    content: super::CertificateContent,
    pages: super::Rows<super::PageContent, MAX_PAGES>,
}
impl<'de> Deserialize<'de> for MeasuredRating {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = MeasuredWire::deserialize(decoder)?;
        let row = Self {
            content: wire.content,
            pages: wire.pages,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl MeasuredRating {
    pub fn new(
        content: super::CertificateContent,
        pages: Vec<super::PageContent>,
    ) -> Result<Self, RatingError> {
        let row = Self {
            content,
            pages: super::Rows::new(pages)?,
        };
        row.validate()?;
        Ok(row)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.content.validate()?;
        if self.content.result != super::RatingResultRef::from_pages(&self.pages.0)? {
            return Err(RatingError::Identity);
        }
        super::encoded(self, super::MAX_RESULT_BYTES)?;
        Ok(())
    }
    pub fn artifact(&self) -> Result<ArtifactIdentity, RatingError> {
        self.validate()?;
        let bytes = super::encoded(self, super::MAX_RESULT_BYTES)?;
        let sha256 = DefinitionDigest::from_bytes(Sha256::digest(&bytes).into());
        Ok(ArtifactIdentity {
            sha256,
            bytes: bytes.len() as u64,
        })
    }
    pub fn publish_parts(
        self,
        publication: super::RatingPublication,
    ) -> Result<(super::RatingReceipt, Vec<super::RatingPage>), RatingError> {
        self.validate()?;
        let job = publication.job();
        let receipt = super::RatingReceipt::new(self.content, publication)?;
        let pages = self
            .pages
            .0
            .into_iter()
            .map(|content| {
                super::RatingPage::new(job, receipt.id(), receipt.result().id(), content)
            })
            .collect::<Result<Vec<_>, _>>()?;
        receipt.verify_pages(&pages)?;
        Ok((receipt, pages))
    }
    pub(super) fn from_receipt(
        receipt: &super::RatingReceipt,
        pages: &[super::RatingPage],
    ) -> Result<Self, RatingError> {
        receipt.verify_pages(pages)?;
        Self::new(
            receipt.content.clone(),
            pages.iter().map(|p| p.content().clone()).collect(),
        )
    }
}

/// Data projection consumed only by the closed core capacity adapter. These
/// rows do not reserve anything or grant access to a typed family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalTarget {
    pub family: &'static str,
    pub key: String,
    pub schema: String,
    pub ceiling: usize,
    pub existing: bool,
}
impl RatingExecution {
    pub(crate) fn terminal_targets(&self) -> Result<Vec<TerminalTarget>, RatingError> {
        self.validate()?;
        let mut targets = Vec::with_capacity(usize::from(self.admission.maximum_pages) + 5);
        let mut target = |family: &'static str, key: String, ceiling, existing| {
            targets.push(TerminalTarget {
                family,
                key,
                schema: format!("roll.{family}.value.v1"),
                ceiling,
                existing,
            });
        };
        let job = self.job.to_string();
        target("rating_jobs", job.clone(), 4096, true);
        target("rating_executions", job.clone(), EXECUTION_BYTES, true);
        target(
            "rating_receipts",
            job.clone(),
            super::MAX_RECEIPT_BYTES,
            false,
        );
        target("rating_completions", job.clone(), 4096, false);
        target("rating_status", job.clone(), 4096, false);
        if self.admission.measured_rejection {
            target(
                "rating_nonconformance",
                job,
                super::MAX_RECEIPT_BYTES,
                false,
            );
        }
        for ordinal in 0..self.admission.maximum_pages {
            target(
                "rating_pages",
                super::RatingPageKey::new(self.job, ordinal)?.encoded(),
                super::MAX_PAGE_BYTES,
                false,
            );
        }
        targets.sort_by(|a, b| (&a.family, &a.key).cmp(&(&b.family, &b.key)));
        if targets.len() > 128 {
            return Err(RatingError::Bound);
        }
        Ok(targets)
    }
}
