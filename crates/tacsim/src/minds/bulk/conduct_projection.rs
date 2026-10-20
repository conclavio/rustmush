//! Exact projection of actual quota images, never quota or worker authority.
use super::{
    BulkError, invalid,
    model::{Artifact, Run, Status, WorkKind},
};
use mush_core::{AccountId, CommitToken, DefinitionDigest};
use mush_roll::commerce::{
    conduct::records::ConductAdmission,
    rating::execution::{
        ArtifactIdentity, ExecutionCursor, ExecutionObservation, ExecutionPhase, PaidFlight,
    },
};

fn artifact(value: &Artifact) -> Result<ArtifactIdentity, BulkError> {
    value.validate()?;
    Ok(ArtifactIdentity {
        sha256: DefinitionDigest::from_bytes(value.sha256),
        bytes: value.bytes,
    })
}
pub(super) fn admission(
    account: AccountId,
    job: CommitToken,
    run: &Run,
) -> Result<ConductAdmission, BulkError> {
    run.plan.validate()?;
    if run.plan.kind != WorkKind::Conduct
        || run.conduct_job != Some(job)
        || run.registry_job.is_some()
    {
        return Err(invalid("Conduct quota identity"));
    }
    let budget = run.plan.conduct.as_ref().ok_or(BulkError::Missing)?;
    let binding = DefinitionDigest::for_canonical_value_with_limit(
        "roll.conduct.job-account.v1",
        &serde_json::json!([account, job, run.id.0, run.nonce]),
        4096,
    )
    .map_err(|_| invalid("Conduct account binding"))?;
    ConductAdmission::observed(
        binding,
        run.character,
        run.id.0,
        run.nonce,
        budget.subject,
        budget.policy,
        budget.schedule,
        budget.digest()?,
        artifact(&run.plan.start)?,
        budget.maximum_unit_ms(),
    )
    .map_err(|_| invalid("Conduct admission projection"))
}
pub(super) fn observation(run: &Run) -> Result<ExecutionObservation, BulkError> {
    run.plan.validate()?;
    if run.plan.kind != WorkKind::Conduct || run.registry.is_some() {
        return Err(invalid("Conduct quota identity"));
    }
    let budget = run.plan.conduct.as_ref().ok_or(BulkError::Missing)?;
    let conduct = run.conduct.ok_or(BulkError::Missing)?;
    let case = budget.case(conduct)?;
    let cursor = ExecutionCursor {
        index: u16::from(conduct.index),
        simulated_ms: conduct.simulated_ms,
    };
    let flight = run
        .flight
        .as_ref()
        .map(|f| {
            if f.conduct != run.conduct || f.registry.is_some() {
                return Err(invalid("Conduct flight differs"));
            }
            Ok(PaidFlight {
                sequence: f.sequence,
                utc_day: f.utc_day,
                allowance_unit_ms: f.allowance_unit_ms,
                units: case.units,
                native_ms: case.native_ms,
                cursor,
            })
        })
        .transpose()?;
    let phase = match run.status {
        Status::Queued => ExecutionPhase::Queued,
        Status::Running => ExecutionPhase::Running,
        Status::CancelRequested => ExecutionPhase::CancelRequested,
        Status::Complete => ExecutionPhase::Complete,
        Status::Failed => ExecutionPhase::Failed,
        Status::Cancelled => ExecutionPhase::Cancelled,
        Status::Interrupted => ExecutionPhase::Interrupted,
    };
    let value = ExecutionObservation {
        phase,
        next_sequence: run.next_sequence,
        cursor,
        charged_unit_ms: run.charged_unit_ms,
        flight,
        result: run.result.as_ref().map(artifact).transpose()?,
    };
    value
        .validate(budget.maximum_unit_ms())
        .map_err(|_| invalid("Conduct actual quota projection"))?;
    Ok(value)
}
pub(super) fn require_before(
    account: AccountId,
    job: CommitToken,
    run: &Run,
    expected: &ConductAdmission,
    before: &ExecutionObservation,
) -> Result<(), BulkError> {
    if admission(account, job, run)? != *expected || observation(run)? != *before {
        return Err(invalid("Conduct actual quota changed"));
    }
    Ok(())
}
