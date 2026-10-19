//! No source or owner readiness is fabricated: this tests only the closed
//! selector relation. Genuine worker/publication coverage lives in Tacsim.
use super::*;
use crate::commerce::rating::reconformance::{ReconformanceAdmission, tests::*};
use crate::commerce::{CharterId, FiledRevisionId, LineId, WorldIcSeconds, filed::FilingSequence};
use mush_core::{ClockRef, ClockStamp, GOD};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn inputs() -> Result<(
    FiledRecord,
    ReconformanceHead,
    crate::commerce::rating::test_support::CompleteRating,
)> {
    let birth = complete(8)?;
    let mut current = complete(9)?;
    let filed = FiledRevisionId::new(1)?;
    let content = serde_json::from_value(json!({"format":1,"filed":filed,
        "position":{"charter":CharterId::new(20)?,"line":LineId::new(1)?,"sequence":FilingSequence::new(1)?},
        "revision":birth.receipt.subject().revision(),"filed_at":WorldIcSeconds::from_stamp(&ClockStamp{clock:ClockRef::WorldIc{generation:1},at:100})?,"parent":null}))?;
    let filed = FiledRecord {
        format: 1,
        content,
        definition: birth.receipt.subject().definition(),
        duplicate: hash(22),
        rating: birth.receipt.reference()?,
        operation: token(2),
    };
    filed.validate()?;
    let mut wire = serde_json::to_value(&current.execution)?;
    let admission = wire
        .get_mut("admission")
        .and_then(|v| v.as_object_mut())
        .ok_or("admission")?;
    admission.insert("character".into(), json!(GOD));
    admission.insert("measured_rejection".into(), json!(true));
    admission.insert(
        "reconformance".into(),
        serde_json::to_value(ReconformanceAdmission::new(
            filed.content.id(),
            filed.rating,
            None,
        )?)?,
    );
    current.execution = serde_json::from_value(wire)?;
    let head = ReconformanceHead {
        format: 1,
        filed: filed.content.id(),
        compatibility: current.receipt.compatibility().digest(),
        job: current.job.job(),
        admitted_at_utc: 100,
    };
    Ok((filed, head, current))
}
#[test]
fn current_selection_requires_exact_completed_profile_source_and_birth() -> Result {
    let (filed, head, current) = inputs()?;
    assert_eq!(
        selected(
            &head,
            &filed,
            current.receipt.compatibility(),
            &current.job,
            &current.execution
        )?,
        current.receipt.reference()?
    );
    let wrong = RatingCompatibility::combat_v1(hash(99));
    assert!(selected(&head, &filed, &wrong, &current.job, &current.execution).is_err());
    let mut crossed = head.clone();
    crossed.filed = FiledRevisionId::new(2)?;
    assert!(
        selected(
            &crossed,
            &filed,
            current.receipt.compatibility(),
            &current.job,
            &current.execution
        )
        .is_err()
    );
    let mut crossed = filed.clone();
    crossed.rating = complete(7)?.receipt.reference()?;
    assert!(
        selected(
            &head,
            &crossed,
            current.receipt.compatibility(),
            &current.job,
            &current.execution
        )
        .is_err()
    );
    let mut crossed = filed;
    crossed.definition = hash(98);
    assert!(
        selected(
            &head,
            &crossed,
            current.receipt.compatibility(),
            &current.job,
            &current.execution
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn pending_or_generic_failure_cannot_become_current_success() -> Result {
    use crate::commerce::rating::{
        RatingFailure,
        execution::{ExecutionObservation, ExecutionPhase, RatingExecution},
    };
    let (filed, head, current) = inputs()?;
    let queued = RatingJob::new(
        current.job.job(),
        current.job.subject().clone(),
        current.job.compatibility().clone(),
        current.job.plan(),
    );
    let before = ExecutionObservation {
        phase: ExecutionPhase::Queued,
        next_sequence: 1,
        cursor: Default::default(),
        charged_unit_ms: 0,
        flight: None,
        result: None,
    };
    let execution = RatingExecution::admit(
        &queued,
        current.execution.admission().clone(),
        before.clone(),
    )?;
    assert!(matches!(
        selected(
            &head,
            &filed,
            current.receipt.compatibility(),
            &queued,
            &execution
        ),
        Err(RatingError::Unavailable)
    ));
    let failed = queued.transition(RatingJobState::Failed {
        reason: RatingFailure::Execution,
    })?;
    let execution = execution.observed(
        &before,
        ExecutionObservation {
            phase: ExecutionPhase::Failed,
            ..before.clone()
        },
        &failed,
        token(33),
    )?;
    assert!(matches!(
        selected(
            &head,
            &filed,
            current.receipt.compatibility(),
            &failed,
            &execution
        ),
        Err(RatingError::Unavailable)
    ));
    assert!(matches!(failed.state(), RatingJobState::Failed { .. }));
    Ok(())
}
