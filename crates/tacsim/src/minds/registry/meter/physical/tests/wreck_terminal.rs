use super::*;
use crate::arena::{
    resolution::Resolution,
    state::{DueKind, EndReason},
    weapons::Shot,
};
use smol_str::SmolStr;

type WreckFixture = (Arena, Arc<MeteredRunner>, mush_tactics::TacticalUnit);

/// A fixture-owned lethal round reaches the actual impact/section/M32/wreck
/// pipeline in the same native substep as collection, after that think's event.
/// The original hull/profile, deterministic facet draw, and normal elimination
/// end are retained. No end, trace, pending row, or candidate answer is forged.
fn terminal_wreck(
    source: &str,
    previous_call: bool,
) -> Result<WreckFixture, Box<dyn std::error::Error>> {
    let (mut arena, meter) = fixture_source(8, source)?;
    let original = arena.unit(OWN).ok_or("hull")?.unit.clone();
    let native = arena.substep_len_ms();
    let mut wall = 0_i64;
    arena.clock.reanchor(wall);
    if previous_call {
        for _ in 0..32 {
            wall += native;
            let output = arena.step_settled(wall, &[], &[]);
            check!(output.ended.is_none());
            meter.reconcile(&arena)?;
            if meter.measurement()?.summary.thinks() > 0 {
                break;
            }
        }
        check_eq!(meter.measurement()?.summary.thinks(), 1);
    }
    let sequence = arena.minds.get(&OWN).ok_or("mind")?.think_seq;
    let scheduled = arena
        .due
        .iter()
        .find_map(|event| match &event.kind {
            DueKind::MindThink { unit, think_seq } if *unit == OWN && *think_seq == sequence => {
                Some(event.t_ms)
            }
            _ => None,
        })
        .ok_or("actual scheduled think")?;
    let impact_at = ((scheduled.max(1) + native - 1) / native) * native;
    let target = arena.unit(OWN).ok_or("target")?;
    let shooter = arena.unit(ObjId(9)).ok_or("shooter")?;
    let band =
        crate::arena::resolution::facet_band_from(target, shooter.pose.x_cm, shooter.pose.y_cm);
    let shot_seq = (1..=256_u64)
        .find(|seq| {
            arena
                .choose_section(OWN, band, *seq)
                .is_some_and(|(section, _)| section == "hull")
        })
        .ok_or("bounded actual hull facet draw")?;
    check!(arena.fire_line(ObjId(9), OWN).0.is_clear_or_partial());
    let shot = Shot {
        seq: shot_seq,
        shooter: ObjId(9),
        weapon: SmolStr::new_static("cannon"),
        contact: crate::arena::state::ContactId(1),
        target: OWN,
        rounds: 1,
        fired_at_ms: wall,
        impact_at_ms: impact_at,
        range_cm: 5000,
        aim_x_cm: target.pose.x_cm,
        aim_y_cm: target.pose.y_cm,
        aim_cell: arena.definition.terrain.cell_of(&target.pose),
        from_x_cm: shooter.pose.x_cm,
        from_y_cm: shooter.pose.y_cm,
        from_cell: arena.definition.terrain.cell_of(&shooter.pose),
        from_height_cm: 150,
        damage: 10000,
        resolution: Resolution {
            p_permille: 1000,
            ..Resolution::blind(band, 3000, mush_atlas::Cover::None)
        },
    };
    arena.shots.insert(shot_seq, shot);
    arena.due.schedule(impact_at, DueKind::Impact { shot_seq });
    for _ in 0..32 {
        wall += native;
        let output = arena.step_settled(wall, &[], &[]);
        if let Some(end) = output.ended {
            check!(matches!(end.reason, EndReason::Elimination { .. }));
            check_eq!(end.t_ms, impact_at);
            check!(output.thinks.is_empty());
            check!(arena.unit(OWN).ok_or("wreck")?.limits.wreck);
            check!(crate::arena::damage::is_wreck(
                &arena.unit(OWN).ok_or("wreck")?.unit
            ));
            check!(arena.unit(OWN).ok_or("wreck")?.seal.closed);
            check!(arena.minds.get(&OWN).ok_or("mind")?.pending.is_none());
            check!(!arena.awaiting.contains(&OWN));
            check_eq!(
                arena
                    .thinks_outstanding()
                    .iter()
                    .filter(|think| think.unit == OWN)
                    .count(),
                1
            );
            let row = arena
                .traces
                .recent(OWN, Some(THINKS))
                .pop()
                .ok_or("cancelled trace")?;
            check_eq!(row.think_seq, sequence);
            check!(!row.settled);
            check!(matches!(
                row.outcome,
                Outcome::Cancelled {
                    reason: Cancelled::Wreck
                }
            ));
            return Ok((arena, meter, original));
        }
        meter.reconcile(&arena)?;
    }
    Err("actual lethal impact did not reach its bounded terminal".into())
}

#[test]
fn actual_lethal_impact_cancels_uncalled_source_without_credit() -> TestResult {
    let (arena, meter, _) = terminal_wreck(SOURCE, false)?;
    let before = meter.measurement()?;
    let state_hash = crate::arena::state_hash(&arena);
    let ledger = arena.ledger.digest();
    meter.reconcile(&arena)?;
    meter.require_complete()?;
    let after = meter.measurement()?;
    check_eq!(after.summary, before.summary);
    check_eq!(after.seen, before.seen);
    check_eq!(after.witness, before.witness);
    let physical = after.physical.ok_or("physical")?;
    check_eq!(physical.total_thinks, 0);
    check_eq!(physical.cancelled_uncalled, 1);
    check_eq!(physical.cancelled_pending, 0);
    check_eq!(physical.refusals, 0);
    check_eq!(crate::arena::state_hash(&arena), state_hash);
    check_eq!(arena.ledger.digest(), ledger);
    Ok(())
}

#[test]
fn terminal_wreck_suffix_requires_exact_physical_trace_and_job_evidence() -> TestResult {
    for mode in 0..15 {
        let (mut arena, meter, original_unit) = terminal_wreck(SOURCE, false)?;
        let think = arena
            .thinks_outstanding()
            .iter()
            .find(|think| think.unit == OWN)
            .ok_or("job")?
            .clone();
        match mode {
            0 => arena.ended = None,
            1 => arena.unit_mut(OWN).ok_or("wreck")?.limits.wreck = false,
            2 => arena.unit_mut(OWN).ok_or("wreck")?.unit = original_unit,
            3 => arena.unit_mut(OWN).ok_or("wreck")?.seal.closed = false,
            4 => arena.unit_mut(OWN).ok_or("wreck")?.seal.sortie.0 ^= 1,
            5 => {
                arena.awaiting.insert(OWN);
            }
            6 => {
                arena.minds.get_mut(&OWN).ok_or("mind")?.pending =
                    Some(crate::arena::mind::PendingThink {
                        think_seq: think.think_seq,
                        t_ms: think.t_ms,
                        epochs: think.epochs,
                        profile_token: think.profile_token,
                        lands_ms: think.lands_ms,
                        period_ms: think.profile.think_ms,
                        profile_class: think.profile.class,
                        rule_sample: None,
                        intents: Vec::new(),
                        memory: Vec::new(),
                    })
            }
            7..=9 => {
                arena.traces = Default::default();
                arena.traces.set_enabled(true);
                check!(
                    arena
                        .traces
                        .begin(OWN, think.think_seq, think.t_ms, think.dropped)
                );
                let reason = if mode == 7 {
                    Cancelled::Ended
                } else if mode == 8 {
                    Cancelled::Controller
                } else {
                    Cancelled::Wreck
                };
                let at = if mode == 9 {
                    arena.now_ms - 1
                } else {
                    arena.now_ms
                };
                check!(arena.traces.cancel(OWN, think.think_seq, at, reason));
            }
            10 => {
                arena
                    .unsettled
                    .iter_mut()
                    .find(|job| job.unit == OWN)
                    .ok_or("job")?
                    .profile_token
                    .generation += 1
            }
            11 => arena.unsettled.retain(|job| job.unit != OWN),
            12 => arena.unsettled.push(think.clone()),
            13 => {
                check!(arena.traces.complete(
                    OWN,
                    think.think_seq,
                    Some(ThinkDiagnostics::measured(1, 1)),
                    &[],
                    &[],
                    None
                ));
            }
            _ => {
                check!(
                    arena
                        .traces
                        .complete(OWN, think.think_seq, None, &[], &[], Some("ops"))
                );
            }
        }
        check!(
            meter.reconcile(&arena).is_err(),
            "forged wreck suffix mode {mode}"
        );
        check!(meter.measurement().is_err(), "refusal remains sticky");
    }
    Ok(())
}

#[test]
fn actual_budget_fault_before_terminal_wreck_is_retained() -> TestResult {
    let (arena, meter, _) = terminal_wreck("fn think(v,c) { while true {} }", true)?;
    let before = meter.measurement()?;
    check!(before.summary.only_budget_faults());
    check_eq!(before.summary.thinks(), 1);
    check_eq!(before.summary.ops_faults(), 1);
    meter.reconcile(&arena)?;
    meter.require_complete()?;
    let after = meter.measurement()?;
    check_eq!(after.summary, before.summary);
    check_eq!(after.seen, before.seen);
    check_eq!(after.witness, before.witness);
    check_eq!(after.physical.ok_or("physical")?.cancelled_uncalled, 1);
    Ok(())
}
