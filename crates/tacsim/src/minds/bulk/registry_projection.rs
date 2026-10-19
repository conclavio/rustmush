//! Exact readback of the real quota owner's image for the registered adapter.
//! No calculation here grants quota, advances a cursor, or certifies a result.
//! The caller obtains Run only from BulkStore or its checked Prepared image.

use mush_core::{AccountId, CommitToken, DefinitionDigest};
use mush_roll::commerce::rating::{
    MAX_PAGE_ROWS,
    execution::{
        ArtifactIdentity, ExecutionAdmission, ExecutionCursor, ExecutionObservation,
        ExecutionPhase, PaidFlight, job_account_binding,
    },
};

use super::{
    BulkError, invalid,
    model::{Artifact, Run, Status, WorkKind},
};

fn artifact(value: &Artifact) -> Result<ArtifactIdentity, BulkError> {
    value.validate()?;
    let value = ArtifactIdentity {
        sha256: DefinitionDigest::from_bytes(value.sha256),
        bytes: value.bytes,
    };
    value
        .validate()
        .map_err(|_| invalid("Registry artifact evidence"))?;
    Ok(value)
}

pub(super) fn admission(
    account: AccountId,
    job: CommitToken,
    run: &Run,
) -> Result<ExecutionAdmission, BulkError> {
    run.plan.validate()?;
    if run.plan.kind != WorkKind::Registry || run.registry_job != Some(job) {
        return Err(invalid("Registry owner evidence requires a Registry run"));
    }
    let budget = run.plan.registry.as_ref().ok_or(BulkError::Missing)?;
    let cases = budget.case_count()?;
    let value = ExecutionAdmission {
        job_account_binding: job_account_binding(account, job, run.id.0, run.nonce)
            .map_err(|_| invalid("Registry account binding"))?,
        character: run.character,
        run: run.id.0,
        nonce: run.nonce,
        subject: budget.subject,
        compatibility: budget.compatibility,
        plan: budget.schedule,
        budget: budget.digest()?,
        start: artifact(&run.plan.start)?,
        maximum_unit_ms: budget.maximum_unit_ms(),
        cases,
        measured_rejection: budget.measured_rejection,
        maximum_pages: u16::try_from(usize::from(cases).div_ceil(MAX_PAGE_ROWS))
            .map_err(|_| invalid("Registry page count"))?,
    };
    value
        .validate()
        .map_err(|_| invalid("Registry immutable owner evidence"))?;
    Ok(value)
}

pub(super) fn observation(run: &Run) -> Result<ExecutionObservation, BulkError> {
    run.plan.validate()?;
    if run.plan.kind != WorkKind::Registry {
        return Err(invalid("Registry observation requires a Registry run"));
    }
    let budget = run.plan.registry.as_ref().ok_or(BulkError::Missing)?;
    let cursor = run.registry.ok_or(BulkError::Missing)?;
    let case = budget.case(cursor)?;
    let cursor = ExecutionCursor {
        index: cursor.index,
        simulated_ms: cursor.simulated_ms,
    };
    let flight = run
        .flight
        .as_ref()
        .map(|flight| {
            if flight.registry != run.registry {
                return Err(invalid("Registry flight and run cursors differ"));
            }
            Ok(PaidFlight {
                sequence: flight.sequence,
                utc_day: flight.utc_day,
                allowance_unit_ms: flight.allowance_unit_ms,
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
        .map_err(|_| invalid("Registry observed owner image"))?;
    Ok(value)
}

/// A request must name the exact persisted execution's immutable account/run
/// binding and before-image. The actual participant calls this before preparing
/// a change and rechecks its captured owner generation before installation.
pub(super) fn require_before(
    account: AccountId,
    job: CommitToken,
    run: &Run,
    expected: &ExecutionAdmission,
    before: &ExecutionObservation,
) -> Result<(), BulkError> {
    if admission(account, job, run)? != *expected || observation(run)? != *before {
        return Err(invalid("Registry owner observation changed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minds::bulk::{
        model::{BulkConfig, Opponent, Plan, RunId, Scenario},
        owner::{BulkStore, Lease, Prepared, Receipt},
        registry_budget::{
            RegistryBudget, RegistryCase, RegistryCursor, RegistryEnding, RegistryProgress,
        },
    };
    use mush_core::ObjId;
    use mush_roll::commerce::rating::{CaseKey, CaseKind, ProbeName, RatingClass};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn plan() -> TestResult<Plan> {
        let hash = DefinitionDigest::from_bytes([1; 32]);
        let cases = [2, 33]
            .into_iter()
            .enumerate()
            .map(|(pair, units)| {
                Ok(RegistryCase {
                    key: CaseKey::new(
                        RatingClass::I,
                        CaseKind::Probe {
                            pair: u8::try_from(pair)?,
                            name: ProbeName::DensityArmed,
                        },
                    )?,
                    units,
                    native_ms: 250,
                    maximum_ms: 10_000,
                })
            })
            .collect::<TestResult<Vec<_>>>()?;
        Ok(Plan {
            registry: Some(RegistryBudget::prepare(hash, hash, hash, cases)?),
            kind: WorkKind::Registry,
            scenario: Scenario::Duel,
            opponent: Opponent::Doctrine,
            per_side: 0,
            unit_count: 0,
            games: 0,
            seeds: Vec::new(),
            limit_ms: 0,
            native_substep_ms: 0,
            start: Artifact {
                sha256: [4; 32],
                bytes: 100,
            },
            result_reservation_bytes: 8192,
        })
    }

    fn apply(store: &mut BulkStore, change: Prepared) -> Result<Option<Lease>, BulkError> {
        store.check(&change)?;
        Ok(store.apply_checked(change))
    }

    #[test]
    fn exact_readback_follows_actual_paid_roster_change_and_refund() -> TestResult {
        let account = AccountId::mint("registry-readback", 1)?;
        let actor = ObjId(7);
        let job = CommitToken::from_bytes([5; 16]);
        let mut store = BulkStore::new(BulkConfig::default())?;
        let (run, change) = store.prepare_registry_admit(account, actor, 1, plan()?, job, 0)?;
        let initial = change.resulting_run(run).ok_or("prepared run")?;
        let admitted = admission(account, job, initial)?;
        let queued = observation(initial)?;
        assert_eq!((admitted.cases, admitted.maximum_pages), (2, 1));
        assert_eq!(
            (queued.phase, queued.charged_unit_ms),
            (ExecutionPhase::Queued, 0)
        );
        apply(&mut store, change)?;
        require_before(
            account,
            job,
            store.run(account, actor, run)?,
            &admitted,
            &queued,
        )?;
        assert!(
            require_before(
                account,
                CommitToken::from_bytes([6; 16]),
                store.run(account, actor, run)?,
                &admitted,
                &queued
            )
            .is_err()
        );
        let change = store.prepare_grant(account, run, 0, 2000)?;
        let granted = observation(change.resulting_run(run).ok_or("grant")?)?;
        let paid = granted.flight.as_ref().ok_or("paid flight")?;
        assert_eq!(
            (paid.units, paid.allowance_unit_ms, granted.charged_unit_ms),
            (2, 4000, 4000)
        );
        let lease = apply(&mut store, change)?.ok_or("lease")?;
        let change = store.prepare_finish(
            Receipt {
                registry: Some(RegistryProgress {
                    cursor: RegistryCursor {
                        index: 1,
                        simulated_ms: 0,
                    },
                    ending: RegistryEnding::CaseComplete,
                }),
                lease,
                actual_unit_ms: 500,
                result: None,
                terminal: None,
            },
            1,
        )?;
        let settled = observation(change.resulting_run(run).ok_or("settled run")?)?;
        assert_eq!((settled.cursor.index, settled.charged_unit_ms), (1, 500));
        assert!(settled.flight.is_none());
        assert_eq!(
            admission(account, job, change.resulting_run(run).ok_or("run")?)?,
            admitted
        );
        apply(&mut store, change)?;
        let change = store.prepare_grant(account, run, 1, 2000)?;
        let second = observation(change.resulting_run(run).ok_or("second grant")?)?;
        let paid = second.flight.as_ref().ok_or("second flight")?;
        assert_eq!(
            (paid.units, paid.allowance_unit_ms, second.charged_unit_ms),
            (33, 66_000, 66_500)
        );
        Ok(())
    }

    #[test]
    fn no_unknown_failed_work_refund_is_invented_by_readback() -> TestResult {
        let account = AccountId::mint("registry-failed-readback", 1)?;
        let actor = ObjId(7);
        let job = CommitToken::from_bytes([7; 16]);
        let mut store = BulkStore::new(BulkConfig::default())?;
        let (run, change) = store.prepare_registry_admit(account, actor, 1, plan()?, job, 0)?;
        apply(&mut store, change)?;
        let change = store.prepare_grant(account, run, 0, 2000)?;
        let lease = apply(&mut store, change)?.ok_or("lease")?;
        let before = observation(store.run(account, actor, run)?)?;
        let mut failed = Receipt {
            registry: None,
            lease,
            actual_unit_ms: 0,
            result: None,
            terminal: Some(Status::Failed),
        };
        assert!(store.prepare_finish(failed.clone(), 1).is_err());
        assert_eq!(observation(store.run(account, actor, run)?)?, before);
        failed.actual_unit_ms = 4000;
        let change = store.prepare_finish(failed, 1)?;
        let after = observation(change.resulting_run(run).ok_or("failed run")?)?;
        assert_eq!(
            (after.phase, after.charged_unit_ms),
            (ExecutionPhase::Failed, 4000)
        );
        assert!(after.flight.is_none());
        assert!(after.result.is_none());
        apply(&mut store, change)?;
        assert_eq!(
            observation(store.run(account, actor, RunId(run.0))?)?,
            after
        );
        Ok(())
    }
}
