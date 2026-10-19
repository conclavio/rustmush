//! Structural evidence for composed Engine ownership/receipt tests only.
//! This has no production constructor or Registry admission capability and
//! does not claim that a source completed the combat.v1 worker schedule.
use super::*;

pub(crate) struct CompleteRating {
    pub(crate) receipt: RatingReceipt,
    pub(crate) pages: Vec<RatingPage>,
    pub(crate) job: RatingJob,
    pub(crate) completion: RatingCompletion,
    pub(crate) status: RatingStatus,
    pub(crate) execution: execution::RatingExecution,
}
/// Caller supplies the actual immutable revision/schema/definition subject,
/// registered manifest compatibility, admitted plan, transaction token and IC
/// clock stamp. The caller stages these real rows through its registered family
/// handles and derives actual roots through the ordinary private owner path.
pub(crate) fn complete_rating(
    subject: RatingSubject,
    compatibility: RatingCompatibility,
    plan: DefinitionDigest,
    job: CommitToken,
    issued: ClockStamp,
) -> Result<CompleteRating, RatingError> {
    let class = subject.declared();
    let row = CaseResult::new(
        CaseKey::new(
            class,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::EmptyArmed,
            },
        )?,
        CaseBinding::new(
            subject.revision().source(),
            subject.definition(),
            subject.definition(),
            subject.definition(),
        ),
        MeterSummary::new(1, 100, 20, Some(Peak::new(100, 20, 0, 1)?), 0, 0, 0, 0)?,
        Coverage::new(
            Applicability::Armed,
            1,
            1,
            digest(
                "roll.rating.structural_test_witness.v1",
                &(subject.digest()?, plan),
                4096,
            )?,
        )?,
        0,
        CaseOutcome::Conformant {},
        None,
    )?;
    let summary = ClassSummary::from_cases(class, &[&row])?;
    let content = PageContent::new(subject.digest()?, compatibility.digest(), 0, vec![row])?;
    let result = RatingResultRef::from_pages(std::slice::from_ref(&content))?;
    let cert = CertificateContent::new(
        subject.clone(),
        compatibility.clone(),
        plan,
        vec![VariantAlias::new(
            AliasKind::Profile,
            "StructuralFixture".into(),
            0,
        )?],
        vec![summary],
        class,
        result.clone(),
    )?;
    let receipt = RatingReceipt::new(cert, RatingPublication::new(job, issued)?)?;
    let pages = vec![RatingPage::new(
        receipt.publication().job(),
        receipt.id(),
        result.id(),
        content,
    )?];
    receipt.verify_pages(&pages)?;
    let reference = receipt.reference()?;
    let job = RatingJob::new(job, subject, compatibility, plan)
        .transition(RatingJobState::Running { lease_sequence: 1 })?
        .transition(RatingJobState::Completed { receipt: reference })?;
    let execution = completed_execution(&receipt, &pages)?;
    let completion = RatingCompletion::for_execution(&receipt, &execution)?;
    let status = RatingStatus::active(&receipt)?;
    Ok(CompleteRating {
        receipt,
        pages,
        job,
        completion,
        status,
        execution,
    })
}

/// Invented accounting facts are confined to this structural codec fixture.
/// Actual producer tests must obtain all of these from registered owner outputs.
pub(crate) fn completed_execution(
    receipt: &RatingReceipt,
    pages: &[RatingPage],
) -> Result<execution::RatingExecution, RatingError> {
    use execution::{
        ArtifactIdentity, ExecutionAdmission, ExecutionCursor, ExecutionObservation,
        ExecutionPhase, MeasuredRating, RatingExecution,
    };
    let queued = RatingJob::new(
        receipt.publication().job(),
        receipt.subject().clone(),
        receipt.compatibility().clone(),
        receipt.plan(),
    );
    let subject = receipt.subject();
    let cases = receipt.result().cases();
    let admission = ExecutionAdmission {
        job_account_binding: execution::job_account_binding(
            mush_core::AccountId::mint("structural-rating-fixture", 1)
                .map_err(|_| RatingError::Invalid)?,
            queued.job(),
            1,
            1,
        )?,
        character: subject.revision().revision().key.character,
        run: 1,
        nonce: 1,
        subject: subject.digest()?,
        compatibility: receipt.compatibility().digest(),
        plan: receipt.plan(),
        budget: DefinitionDigest::from_bytes([44; 32]),
        start: ArtifactIdentity {
            sha256: DefinitionDigest::from_bytes([45; 32]),
            bytes: 100,
        },
        maximum_unit_ms: 1_000_000,
        cases,
        maximum_pages: u16::try_from(usize::from(cases).div_ceil(MAX_PAGE_ROWS))
            .map_err(|_| RatingError::Bound)?,
    };
    let initial = ExecutionObservation {
        phase: ExecutionPhase::Queued,
        next_sequence: 1,
        cursor: ExecutionCursor::default(),
        charged_unit_ms: 0,
        flight: None,
        result: None,
    };
    let admitted = RatingExecution::admit(&queued, admission, initial.clone())?;
    let running = queued.transition(RatingJobState::Running { lease_sequence: 1 })?;
    let running_observation = ExecutionObservation {
        phase: ExecutionPhase::Running,
        next_sequence: 2,
        ..initial.clone()
    };
    let progress_token = different_token(queued.job(), 201);
    let running_execution = admitted.observed(
        &initial,
        running_observation.clone(),
        &running,
        progress_token,
    )?;
    let result = MeasuredRating::from_receipt(receipt, pages)?.artifact()?;
    let completed = running.transition(RatingJobState::Completed {
        receipt: receipt.reference()?,
    })?;
    let final_observation = ExecutionObservation {
        phase: ExecutionPhase::Complete,
        result: Some(result),
        ..running_observation.clone()
    };
    let finish_token = different_token(queued.job(), 202);
    let row = running_execution.observed(
        &running_observation,
        final_observation,
        &completed,
        finish_token,
    )?;
    row.verify_completion(&completed, receipt, pages)?;
    Ok(row)
}

fn different_token(job: CommitToken, byte: u8) -> CommitToken {
    let token = CommitToken::from_bytes([byte; 16]);
    if token == job {
        CommitToken::from_bytes([byte.wrapping_add(10); 16])
    } else {
        token
    }
}
