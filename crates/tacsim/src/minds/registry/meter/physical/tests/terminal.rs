use super::*;
use crate::arena::state::{DueKind, EndReason, StopReason};

type Fixture = (Arena, Arc<MeteredRunner>);

/// The real duration backstop ends on the first candidate's collecting native
/// substep. No end flag, job, answer, trace, or source measurement is invented.
fn terminal_uncalled() -> Result<Fixture, Box<dyn std::error::Error>> {
    let (mut arena, meter) = fixture(8)?;
    let scheduled = arena
        .due
        .iter()
        .find_map(|event| match &event.kind {
            DueKind::MindThink { unit, think_seq: 0 } if *unit == OWN => Some(event.t_ms),
            _ => None,
        })
        .ok_or("actual scheduled think")?;
    let native = arena.substep_len_ms();
    let deadline = ((scheduled.max(1) + native - 1) / native) * native;
    arena.match_cfg.max_duration_ms = u32::try_from(deadline)?;
    arena.clock.reanchor(0);
    for step in 1..=32_i64 {
        let output = arena.step_settled(step * native, &[], &[]);
        if let Some(end) = output.ended {
            assert_eq!(end.reason, EndReason::Timeout);
            assert_eq!(end.t_ms, deadline);
            assert!(
                output.thinks.is_empty(),
                "actual terminal worker batch is suppressed"
            );
            assert_eq!(
                arena
                    .thinks_outstanding()
                    .iter()
                    .filter(|think| think.unit == OWN)
                    .count(),
                1
            );
            return Ok((arena, meter));
        }
        meter.reconcile(&arena)?;
    }
    Err("real terminal collection did not occur within native bound".into())
}

#[test]
fn real_terminal_uncalled_source_has_no_call_cost_or_coverage_credit() -> TestResult {
    let (arena, meter) = terminal_uncalled()?;
    let before = meter.measurement()?;
    let hash = crate::arena::state_hash(&arena);
    let ledger = arena.ledger.digest();
    meter.reconcile(&arena)?;
    meter.require_complete()?;
    meter.require_complete()?;
    let after = meter.measurement()?;
    assert_eq!(after.summary, before.summary);
    assert_eq!(after.seen, before.seen);
    assert_eq!(after.witness, before.witness);
    assert_eq!(after.summary.thinks(), 0);
    let physical = after.physical.ok_or("physical")?;
    assert_eq!(physical.total_thinks, 0);
    assert_eq!(physical.cancelled_uncalled, 1);
    assert_eq!(physical.cancelled_pending, 0);
    assert_eq!(physical.rom_thinks, 0);
    assert_eq!(physical.degraded_thinks, 0);
    assert_eq!(physical.refusals, 0);
    assert_ne!(
        physical.witness,
        before.physical.ok_or("before physical")?.witness
    );
    assert_eq!(crate::arena::state_hash(&arena), hash);
    assert_eq!(arena.ledger.digest(), ledger);
    assert!(
        !arena
            .traces
            .recent(OWN, Some(THINKS))
            .last()
            .ok_or("trace")?
            .settled
    );
    Ok(())
}

#[test]
fn terminal_suffix_refuses_missing_extra_reordered_or_forged_evidence() -> TestResult {
    for mode in 0..18 {
        let (mut arena, meter) = terminal_uncalled()?;
        let original = arena
            .thinks_outstanding()
            .iter()
            .find(|think| think.unit == OWN)
            .ok_or("actual uncalled job")?
            .clone();
        match mode {
            0 => arena.unsettled.retain(|think| think.unit != OWN),
            1 => arena.unsettled.push(original.clone()),
            2 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .think_seq += 1
            }
            3 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .unit = ObjId(99)
            }
            4 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .profile_token
                    .generation += 1
            }
            5 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .profile
                    .ops += 1
            }
            6 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .dropped = true
            }
            7 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .t_ms += 1
            }
            8 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .lands_ms += 1
            }
            9 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .substep_hash = None
            }
            10 => arena.traces = Default::default(),
            11 => {
                assert!(arena.traces.complete(
                    OWN,
                    original.think_seq,
                    Some(ThinkDiagnostics::measured(1, 1)),
                    &[],
                    &[],
                    None
                ));
            }
            12 => {
                assert!(
                    arena
                        .traces
                        .begin(OWN, original.think_seq + 1, original.t_ms, false)
                );
            }
            13 => arena.minds.get_mut(&OWN).ok_or("mind")?.pending = None,
            14 => {
                arena.awaiting.remove(&OWN);
            }
            15 => arena.minds.get_mut(&OWN).ok_or("mind")?.think_seq += 1,
            16 => {
                let mut extra = original.clone();
                extra.think_seq += 1;
                arena.unsettled.insert(0, extra);
            }
            _ => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|think| think.unit == OWN)
                    .ok_or("job")?
                    .program
                    .digest = DefinitionDigest::from_bytes([0xff; 32]);
            }
        }
        assert!(
            meter.reconcile(&arena).is_err(),
            "forged terminal evidence mode {mode}"
        );
        assert!(meter.measurement().is_err(), "refusal is sticky");
    }
    Ok(())
}

#[test]
fn ordinary_running_collected_suffix_still_refuses() -> TestResult {
    let (mut arena, meter) = fixture(8)?;
    let think = collect(&mut arena, 1000)?;
    arena.collected.push(think);
    arena.stamp_collected(crate::arena::state_hash(&arena));
    assert!(arena.ended.is_none());
    assert!(meter.reconcile(&arena).is_err());
    Ok(())
}

#[test]
fn real_stopping_pulse_retains_called_pending_cost_without_landing_credit() -> TestResult {
    let (mut arena, meter) = fixture(8)?;
    let think = collect(&mut arena, 1000)?;
    let answer = think.think().result;
    assert!(answer.fault.is_none());
    assert_eq!(arena.settle_collected(&think, answer).faults, 0);
    meter.reconcile(&arena)?;
    let before = meter.measurement()?;
    assert!(meter.require_complete().is_err());
    arena.stop(StopReason::Timeout);
    let output = arena.step(1000, &[], &[]);
    assert_eq!(
        output.ended.ok_or("actual stopped end")?.reason,
        EndReason::Timeout
    );
    let hash = crate::arena::state_hash(&arena);
    let ledger = arena.ledger.digest();
    meter.reconcile(&arena)?;
    meter.require_complete()?;
    let after = meter.measurement()?;
    assert_eq!(after.summary, before.summary);
    assert_eq!(after.seen, before.seen);
    assert_eq!(after.witness, before.witness);
    let physical = after.physical.ok_or("physical")?;
    assert_eq!(physical.cancelled_pending, 1);
    assert_eq!(physical.cancelled_uncalled, 0);
    assert_eq!(physical.total_thinks, 1);
    assert_eq!(physical.refusals, 0);
    let row = arena
        .traces
        .recent(OWN, Some(THINKS))
        .pop()
        .ok_or("trace")?;
    assert!(row.settled && matches!(row.outcome, Outcome::Pending));
    assert!(row.landed_ms.is_none());
    assert_eq!(crate::arena::state_hash(&arena), hash);
    assert_eq!(arena.ledger.digest(), ledger);
    Ok(())
}

#[test]
fn terminal_does_not_erase_actual_operations_or_interpreter_faults() -> TestResult {
    for (source, ops) in [
        ("fn think(v,c) { while true {} }", true),
        ("fn think(v,c) { throw \"expected fault\"; }", false),
    ] {
        let (mut arena, meter) = fixture_source(8, source)?;
        let think = collect(&mut arena, 1000)?;
        let answer = think.think().result;
        assert!(answer.fault.is_some());
        assert_eq!(arena.settle_collected(&think, answer).faults, 1);
        arena.stop(StopReason::Timeout);
        let output = arena.step(1000, &[], &[]);
        assert!(output.ended.is_some());
        let joined = meter.reconcile(&arena);
        if ops {
            joined?;
            meter.require_complete()?;
            let measured = meter.measurement()?;
            assert!(measured.summary.only_budget_faults());
            assert_eq!(measured.summary.ops_faults(), 1);
            assert_eq!(measured.summary.thinks(), 1);
            assert_eq!(measured.physical.ok_or("physical")?.cancelled_uncalled, 0);
        } else {
            assert!(joined.is_err());
            assert!(meter.measurement().is_err());
        }
    }
    Ok(())
}

#[test]
fn terminal_seal_rejects_reconciliation_or_calls_after_completion() -> TestResult {
    for call in [false, true] {
        let (arena, meter) = terminal_uncalled()?;
        meter.reconcile(&arena)?;
        let before = meter.measurement()?.physical.ok_or("physical")?;
        meter.require_complete()?;
        if call {
            let think = arena
                .thinks_outstanding()
                .iter()
                .find(|think| think.unit == OWN)
                .ok_or("job")?;
            let actual = think.think().result;
            assert_eq!(actual, meter.runner.think(&think.job()));
        } else {
            assert!(
                meter.reconcile(&arena).is_err(),
                "terminal reconciliation is explicitly single-use"
            );
        }
        assert!(meter.measurement().is_err());
        let state = meter.state.lock().map_err(|_| "poison")?;
        assert_eq!(state.physical.as_ref().ok_or("physical")?.summary, before);
    }
    Ok(())
}
