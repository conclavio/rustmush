//! Real pure K arithmetic only; actual owner/action/ACK tests live with providers.
use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 7 },
        at,
    }
}
fn due(p: &CoreProgress, at: i64, cause: ScheduleDriveCause) -> Result<ScheduleAdvance, Error> {
    match p
        .cursor()
        .plan_due(p.cursor().revision(), &stamp(at), cause)
        .map_err(bad)?
    {
        ScheduleEvaluation::Advance(advance) => Ok(*advance),
        _ => Err(Error::Invalid),
    }
}

#[test]
fn every_real_occurrence_replays_inside_the_original_seven_day_hold() -> TestResult {
    let mut p = CoreProgress::new(WorkOrderId(0), stamp(0), MAX_WORK_UNITS)?;
    for n in 1..=MAX_WORK_UNITS {
        let observed = i64::try_from(n)?
            .checked_mul(CADENCE)
            .ok_or("time overflow")?;
        let advance = due(&p, observed, ScheduleDriveCause::Live)?;
        assert_eq!(advance.occurrences().len(), 1);
        p = p.advance(CommitToken::from_bytes([u8::try_from(n)?; 16]), &advance)?;
    }
    assert!(p.production().is_complete());
    assert_eq!(p.production().progress_units(), MAX_WORK_UNITS);
    assert_eq!(p.cursor().next_ordinal(), MAX_WORK_UNITS);
    p.validate()?;
    let bytes = serde_json::to_vec(&p)?;
    assert!(bytes.len() < MAX_PROGRESS_BYTES);
    assert_eq!(serde_json::from_slice::<CoreProgress>(&bytes)?, p);
    let next = due(&p, 7 * 86_400, ScheduleDriveCause::Live)?;
    assert_eq!(
        p.advance(CommitToken::from_bytes([220; 16]), &next),
        Err(Error::Expired)
    );
    Ok(())
}

#[test]
fn real_restart_catchup_uses_occurrences_and_cannot_credit_expired_time() -> TestResult {
    let p = CoreProgress::new(WorkOrderId(u64::MAX), stamp(9), 24)?;
    assert!(matches!(
        p.cursor().plan_due(
            p.cursor().revision(),
            &stamp(9 + CADENCE - 1),
            ScheduleDriveCause::Live
        )?,
        ScheduleEvaluation::Waiting { .. }
    ));
    let at = 9 + 24 * CADENCE;
    let advance = due(&p, at, ScheduleDriveCause::Restart)?;
    assert_eq!(advance.occurrences().len(), 24);
    assert!(advance.skipped().is_none());
    let complete = p.advance(CommitToken::from_bytes([1; 16]), &advance)?;
    complete.validate()?;
    assert!(complete.production().is_complete());
    assert_eq!(complete.advances.len(), 1);
    let expired = due(&p, 9 + 7 * 86_400, ScheduleDriveCause::Restart)?;
    assert_eq!(
        p.advance(CommitToken::from_bytes([2; 16]), &expired),
        Err(Error::Expired)
    );
    assert!(
        p.cursor()
            .plan_due(
                p.cursor().revision(),
                &ClockStamp {
                    clock: ClockRef::WorldIc { generation: 8 },
                    at
                },
                ScheduleDriveCause::Restart
            )
            .is_err()
    );
    assert!(CoreProgress::new(WorkOrderId(1), stamp(0), MAX_WORK_UNITS + 1).is_err());
    Ok(())
}

#[test]
fn cold_replay_rejects_forged_progress_omission_duplicate_and_crossed_cursor() -> TestResult {
    let initial = CoreProgress::new(WorkOrderId(33), stamp(0), 5)?;
    let first = due(&initial, CADENCE, ScheduleDriveCause::Live)?;
    let once = initial.advance(CommitToken::from_bytes([1; 16]), &first)?;
    let second = due(&once, 2 * CADENCE, ScheduleDriveCause::Live)?;
    let twice = once.advance(CommitToken::from_bytes([2; 16]), &second)?;
    let good = serde_json::to_value(&twice)?;
    let mut changed = good.clone();
    changed
        .get_mut("production")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("production")?
        .insert("progress_units".into(), serde_json::json!(5));
    assert!(serde_json::from_value::<CoreProgress>(changed).is_err());
    let mut changed = good.clone();
    changed
        .get_mut("advances")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("advances")?
        .remove(0);
    assert!(serde_json::from_value::<CoreProgress>(changed).is_err());
    let mut changed = twice.clone();
    let first_commit = changed.advances.first().ok_or("first")?.commit;
    changed.advances.last_mut().ok_or("last")?.commit = first_commit;
    assert!(changed.validate().is_err());
    let mut changed = twice.clone();
    changed.cursor = CoreProgress::new(WorkOrderId(34), stamp(0), 5)?.cursor;
    assert!(changed.validate().is_err());
    let mut changed = good;
    changed
        .get_mut("advances")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|a| a.last_mut())
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("last proof")?
        .insert("occurrences".into(), serde_json::json!(2));
    assert!(serde_json::from_value::<CoreProgress>(changed).is_err());
    assert!(
        once.advance(CommitToken::from_bytes([1; 16]), &second)
            .is_err()
    );
    assert!(
        twice
            .advance(CommitToken::from_bytes([3; 16]), &first)
            .is_err()
    );
    Ok(())
}
