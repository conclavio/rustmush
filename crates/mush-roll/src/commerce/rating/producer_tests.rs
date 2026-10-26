//! Structural transition tests, not worker-conformance evidence.
use super::*;
fn resulting_job(
    old: &RatingJob,
    quota: &QuotaOutput,
    receipt: Option<&RatingReceipt>,
) -> Result<RatingJob, RatingError> {
    super::resulting_job(old, quota, receipt, None)
}
use crate::commerce::rating::{
    RatingClass, RatingSubject, execution::ExecutionObservation, test_support,
};
use mush_core::{ClockRef, ClockStamp, DefinitionDigest};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn complete() -> TestResult<test_support::CompleteRating> {
    let hash = DefinitionDigest::from_bytes([1; 32]);
    let identity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash},"source":hash}))?;
    Ok(test_support::complete_rating(
        RatingSubject::new(identity, hash, hash, hash, hash, RatingClass::I),
        RatingCompatibility::combat_v1(hash),
        hash,
        CommitToken::from_bytes([1; 16]),
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?)
}
fn initial() -> ExecutionObservation {
    ExecutionObservation {
        phase: ExecutionPhase::Queued,
        next_sequence: 1,
        cursor: Default::default(),
        charged_unit_ms: 0,
        flight: None,
        result: None,
    }
}

#[test]
fn phase_mapping_retains_progress_distinguishes_cancel_and_keeps_actual_failure_meaning()
-> TestResult {
    let data = complete()?;
    let old = RatingJob::new(
        data.job.job(),
        data.job.subject().clone(),
        data.job.compatibility().clone(),
        data.job.plan(),
    );
    let after = ExecutionObservation {
        phase: ExecutionPhase::Running,
        next_sequence: 2,
        ..initial()
    };
    let output = |transition, after| {
        QuotaOutput::observed(
            CommitToken::from_bytes([2; 16]),
            old.job(),
            transition,
            TicketId::new(1)?,
            data.execution.admission().clone(),
            Some(initial()),
            after,
            None,
        )
    };
    let running = resulting_job(&old, &output(Transition::Grant, after.clone())?, None)?;
    check_eq!(
        running.state(),
        &RatingJobState::Running { lease_sequence: 1 }
    );
    check_eq!(
        resulting_job(&running, &output(Transition::Settle, after.clone())?, None)?,
        running
    );
    check_eq!(
        resulting_job(
            &running,
            &output(
                Transition::Cancel,
                ExecutionObservation {
                    phase: ExecutionPhase::CancelRequested,
                    ..after.clone()
                }
            )?,
            None
        )?,
        running
    );
    let failed = resulting_job(
        &running,
        &output(
            Transition::Settle,
            ExecutionObservation {
                phase: ExecutionPhase::Failed,
                ..after.clone()
            },
        )?,
        None,
    )?;
    check_eq!(
        failed.state(),
        &RatingJobState::Failed {
            reason: RatingFailure::Execution
        }
    );
    let cancelled = resulting_job(
        &running,
        &output(
            Transition::Settle,
            ExecutionObservation {
                phase: ExecutionPhase::Cancelled,
                ..after.clone()
            },
        )?,
        None,
    )?;
    check_eq!(cancelled.state(), &RatingJobState::Cancelled {});
    check!(
        resulting_job(
            &running,
            &output(
                Transition::Settle,
                ExecutionObservation {
                    phase: ExecutionPhase::Cancelled,
                    ..after.clone()
                }
            )?,
            Some(&data.receipt)
        )
        .is_err()
    );
    check!(resulting_job(&running, &output(Transition::Recover, after)?, None).is_err());
    Ok(())
}

#[test]
fn measured_complete_joins_exact_owner_output_and_a_different_job_cannot_take_it() -> TestResult {
    let data = complete()?;
    let measured =
        super::super::execution::MeasuredRating::from_receipt(&data.receipt, &data.pages)?;
    let old = RatingJob::new(
        data.job.job(),
        data.job.subject().clone(),
        data.job.compatibility().clone(),
        data.job.plan(),
    )
    .transition(RatingJobState::Running { lease_sequence: 1 })?;
    let output = QuotaOutput::observed(
        CommitToken::from_bytes([2; 16]),
        old.job(),
        Transition::Settle,
        TicketId::new(1)?,
        data.execution.admission().clone(),
        Some(ExecutionObservation {
            phase: ExecutionPhase::Running,
            next_sequence: 2,
            ..initial()
        }),
        data.execution.observation().clone(),
        None,
    )?;
    let artifacts = ArtifactOutput::observed(&output, Some(measured.clone()))?;
    check!(artifacts.matches(&output)?);
    let completed = resulting_job(&old, &output, Some(&data.receipt))?;
    check_eq!(completed, data.job);
    data.execution
        .verify_completion(&completed, &data.receipt, &data.pages)?;
    let other = RatingJob::new(
        CommitToken::from_bytes([3; 16]),
        old.subject().clone(),
        old.compatibility().clone(),
        old.plan(),
    )
    .transition(RatingJobState::Running { lease_sequence: 1 })?;
    check!(
        RatingRoots::transition(
            &other,
            &completed,
            Some(&data.receipt),
            CommitToken::from_bytes([4; 16])
        )
        .is_err()
    );
    let mut altered = output.after().clone();
    altered.result.as_mut().ok_or("actual artifact")?.bytes += 1;
    let altered = QuotaOutput::observed(
        output.operation(),
        output.job(),
        Transition::Settle,
        output.ticket(),
        output.admission().clone(),
        output.before().cloned(),
        altered,
        None,
    )?;
    check!(ArtifactOutput::observed(&altered, Some(measured)).is_err());
    Ok(())
}

#[test]
fn wire_has_no_account_actor_class_measurement_or_owner_output_fields() -> TestResult {
    let valid = json!({"ticket":1,"mind":1,"revision":1,"revision_digest":DefinitionDigest::from_bytes([1;32])});
    let _: AdmissionWire = serde_json::from_value(valid.clone())?;
    for field in [
        "actor",
        "account",
        "class",
        "measured",
        "receipt",
        "validated",
        "source",
    ] {
        let mut injected = valid.clone();
        injected
            .as_object_mut()
            .ok_or("object")?
            .insert(field.into(), json!(true));
        check!(
            serde_json::from_value::<AdmissionWire>(injected).is_err(),
            "{field}"
        );
    }
    let valid = json!({"ticket":1,"job":CommitToken::from_bytes([1;16])});
    let _: TransitionWire = serde_json::from_value(valid.clone())?;
    let mut injected = valid;
    injected
        .as_object_mut()
        .ok_or("object")?
        .insert("after".into(), json!({"phase":"complete"}));
    check!(serde_json::from_value::<TransitionWire>(injected).is_err());
    Ok(())
}

#[test]
fn recovery_preserves_old_manifest_without_authorizing_old_execution() -> TestResult {
    let data = complete()?;
    let previous = RatingJob::new(
        data.job.job(),
        data.job.subject().clone(),
        data.job.compatibility().clone(),
        data.job.plan(),
    )
    .transition(RatingJobState::Running { lease_sequence: 1 })?;
    let current = RatingCompatibility::combat_v1(DefinitionDigest::from_bytes([99; 32]));
    check_ne!(&current, previous.compatibility());
    for transition in [
        Transition::Admit,
        Transition::Grant,
        Transition::Settle,
        Transition::Cancel,
    ] {
        check!(!accepts_compatibility(
            transition,
            &current,
            previous.compatibility().digest()
        ));
    }
    check!(accepts_compatibility(
        Transition::Recover,
        &current,
        previous.compatibility().digest()
    ));
    let before = ExecutionObservation {
        phase: ExecutionPhase::Running,
        next_sequence: 2,
        ..initial()
    };
    let after = ExecutionObservation {
        phase: ExecutionPhase::Interrupted,
        ..before.clone()
    };
    let output = QuotaOutput::observed(
        CommitToken::from_bytes([2; 16]),
        previous.job(),
        Transition::Recover,
        TicketId::new(1)?,
        data.execution.admission().clone(),
        Some(before),
        after,
        None,
    )?;
    let interrupted = resulting_job(&previous, &output, None)?;
    check_eq!(interrupted.state(), &RatingJobState::Interrupted {});
    check_eq!(interrupted.subject(), previous.subject());
    check_eq!(interrupted.compatibility(), previous.compatibility());
    check_eq!(interrupted.plan(), previous.plan());
    check!(resulting_job(&previous, &output, Some(&data.receipt)).is_err());
    Ok(())
}
