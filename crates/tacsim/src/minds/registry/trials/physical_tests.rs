//! Actual private trial/runner tests. Core loss uses the same finite core-op
//! builder and M32 writer as damage; it is not a fabricated dropped flag or shot.
use super::*;
use crate::arena::{mind::DropCause, resolution::Consequence};
use mush_tactics::UnitMutationPlan;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SOURCE: &str = "fn think(v,c) { stock::rules(); speed(0); this.calls = if this.contains(\"calls\") { this.calls+1 } else { 1 }; }";

fn open(
    source: &str,
    class: RatingClass,
    cycle: bool,
) -> Result<(Trial, PreparedSubject), Box<dyn std::error::Error>> {
    let revision = super::super::tests::revision(source)?;
    open_revision(source, &revision, class, cycle)
}

fn open_stock(
    source: &str,
    class: RatingClass,
) -> Result<(Trial, PreparedSubject), Box<dyn std::error::Error>> {
    let card = crate::arena::card::line(&crate::config::ReflexConfig::default());
    let definition =
        crate::arena::runtime_card::stock_definition(&[("Line".into(), card)], CoreClass::I)?;
    let profile = definition.profiles.first().ok_or("stock profile")?;
    let envelope = mush_roll::Envelope {
        schema: serde_json::to_value(&definition.schema)?,
        presets: serde_json::json!({}),
        profiles: serde_json::json!({"Line":profile.values}),
        rules: serde_json::json!({"Line":profile.rules}),
        declared_class: "I".into(),
        mind_abi: crate::minds::MIND_ABI.into(),
    };
    let revision = serde_json::from_value(serde_json::json!({"format":1,
        "reference":{"key":{"character":1,"mind":1,"number":1},"digest":envelope.digest(source)?},
        "source":mush_roll::source_digest(source),"envelope":envelope,"notes":""}))?;
    open_revision(source, &revision, class, false)
}

fn open_revision(
    source: &str,
    revision: &mush_roll::Revision,
    class: RatingClass,
    cycle: bool,
) -> Result<(Trial, PreparedSubject), Box<dyn std::error::Error>> {
    let policy = super::super::tests::frozen_policy()?;
    let subject = PreparedSubject::prepare(revision, source, &policy.cfg.arena)?;
    let variant = subject
        .variants()
        .iter()
        .position(|variant| variant.cycle == cycle)
        .ok_or("requested real variant")?;
    let case = Case {
        key: mush_roll::commerce::rating::CaseKey::new(
            class,
            CaseKind::Trial {
                variant: u8::try_from(variant)?,
                layout: TrialLayout::Duel,
                seed: 1,
                swapped: false,
            },
        )?,
        physics: Trial::physics(&policy, TrialLayout::Duel, core(class))?,
        maximum_ms: super::super::plan::TRIAL_LIMIT_MS,
        maximum_thinks: 2402,
    };
    Ok((Trial::open(&policy, &subject, &case)?, subject))
}

fn candidate(trial: &Trial) -> Result<ObjId, Box<dyn std::error::Error>> {
    trial
        .candidates
        .first()
        .copied()
        .ok_or_else(|| "candidate".into())
}

fn advance(trial: &mut Trial) -> Result<Option<CaseResult>, RatingError> {
    match trial.advance(u64::from(trial.case.physics.native_ms)) {
        Ok(slice) => Ok(slice.result),
        Err(error) => {
            eprintln!("M7_PHYSICAL_TEST_ADVANCE error={error:?}");
            trial.failure_diagnostic();
            Err(error)
        }
    }
}

fn until_invoked(trial: &mut Trial) -> TestResult {
    for _ in 0..80 {
        assert!(advance(trial)?.is_none(), "no early fabricated terminal");
        if trial.meter.measurement()?.summary.thinks() > 0 {
            return Ok(());
        }
    }
    Err("actual source invocation did not arrive".into())
}

fn mutate_core(arena: &mut Arena, id: ObjId, consequence: Consequence) -> TestResult {
    let ops = arena.core_ops(id, &[consequence]);
    assert!(!ops.is_empty(), "actual held core must change");
    let plan = UnitMutationPlan::new(ops, None, Some("registry-test-core-consequence".into()))?;
    let unit = &arena.unit(id).ok_or("candidate hull")?.unit;
    let (post, _) = mush_tactics::apply(unit, &plan, 0)?;
    let touched = Arena::plan_touches_bands(&plan);
    arena.set_unit(id, post, touched);
    arena.truncate_slots(id);
    Ok(())
}

fn physical(trial: &Trial) -> Result<super::super::meter::PhysicalSummary, RatingError> {
    trial
        .meter
        .measurement()?
        .physical
        .ok_or(RatingError::Incomplete)
}

#[test]
fn physical_core_loss_keeps_in_flight_source_and_excludes_subsequent_rom() -> TestResult {
    let (mut trial, _) = open(SOURCE, RatingClass::I, false)?;
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    assert!(
        trial.arena.minds.get(&id).ok_or("mind")?.pending.is_some(),
        "first source answer has been measured but its native landing is still pending"
    );
    let before = trial.meter.measurement()?;
    mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
    assert!(trial.arena.core_of(id).is_none());
    assert!(advance(&mut trial)?.is_none());
    let first = trial.arena.minds.get(&id).ok_or("mind")?;
    assert_eq!(first.dropped, Some(DropCause::Core));
    let memory: serde_json::Value = serde_json::from_slice(&first.memory)?;
    assert_eq!(
        memory.get("calls").and_then(serde_json::Value::as_u64),
        Some(1),
        "the source answer collected before damage lands before any ROM memory replacement"
    );
    for _ in 0..80 {
        assert!(advance(&mut trial)?.is_none());
        if physical(&trial)?.rom_thinks >= 2 {
            break;
        }
    }
    let after = trial.meter.measurement()?;
    assert_eq!(
        after.summary, before.summary,
        "ROM gives no source cost or invocation credit"
    );
    assert_eq!(after.seen, before.seen);
    assert!(physical(&trial)?.rom_thinks >= 2);
    assert_eq!(
        physical(&trial)?.total_thinks,
        before.summary.thinks() + physical(&trial)?.rom_thinks
    );
    let mind = trial.arena.minds.get(&id).ok_or("mind")?;
    assert_eq!(mind.dropped, Some(DropCause::Core));
    assert_eq!(mind.strikes, 0);
    Ok(())
}

#[test]
fn physical_nonzero_degradation_and_present_zero_have_separate_actual_measurements() -> TestResult {
    let (mut trial, _) = open_stock(SOURCE, RatingClass::II)?;
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    let source_binding = trial
        .arena
        .profiles
        .get(&id)
        .ok_or("runtime")?
        .program
        .clone();
    let before = trial.meter.measurement()?;
    mutate_core(&mut trial.arena, id, Consequence::DegradeCore)?;
    assert_eq!(
        trial.arena.core_of(id).ok_or("present core")?.class,
        CoreClass::I
    );
    for _ in 0..80 {
        assert!(advance(&mut trial)?.is_none());
        if physical(&trial)?.degraded_thinks >= 2 {
            break;
        }
    }
    let degraded = physical(&trial)?;
    assert!(
        degraded.degraded_thinks >= 2
            && degraded.degraded_max_ops > 0
            && degraded.degraded_max_fees > 0
    );
    assert_eq!(degraded.degraded_ops_faults, 0);
    assert_eq!(degraded.rom_thinks, 0);
    assert_eq!(trial.meter.measurement()?.summary, before.summary);
    assert_eq!(trial.meter.measurement()?.seen, before.seen);
    assert_eq!(
        trial.arena.profiles.get(&id).ok_or("runtime")?.program,
        source_binding
    );
    assert!(trial.arena.minds.get(&id).ok_or("mind")?.dropped.is_none());
    mutate_core(&mut trial.arena, id, Consequence::DegradeCore)?;
    assert_eq!(
        trial.arena.core_of(id).ok_or("present Zero core")?.class,
        CoreClass::Zero
    );
    for _ in 0..80 {
        assert!(advance(&mut trial)?.is_none());
        if physical(&trial)?.rom_thinks >= 2 {
            break;
        }
    }
    assert!(physical(&trial)?.rom_thinks >= 2);
    assert!(
        trial.arena.minds.get(&id).ok_or("mind")?.dropped.is_none(),
        "present class Zero legitimately uses stand-in without a Core drop marker"
    );
    assert!(!trial.arena.profiles.get(&id).ok_or("runtime")?.dropped);
    assert_eq!(trial.meter.measurement()?.summary, before.summary);
    assert_eq!(trial.meter.measurement()?.seen, before.seen);
    Ok(())
}

#[test]
fn present_zero_does_not_hide_an_actual_stand_in_card_fault() -> TestResult {
    let (mut trial, _) = open(SOURCE, RatingClass::I, false)?;
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    assert!(
        trial
            .arena
            .profiles
            .get(&id)
            .ok_or("runtime")?
            .card
            .stock()
            .is_none()
    );
    for _ in 0..3 {
        if trial.arena.core_of(id).ok_or("present core")?.class == CoreClass::Zero {
            break;
        }
        mutate_core(&mut trial.arena, id, Consequence::DegradeCore)?;
    }
    let mut refused = false;
    for _ in 0..80 {
        if advance(&mut trial).is_err() {
            refused = true;
            break;
        }
    }
    assert!(
        refused,
        "real native stand-in profile fault is not excused by physical loss"
    );
    assert_eq!(
        trial.arena.core_of(id).ok_or("present Zero core")?.class,
        CoreClass::Zero
    );
    assert!(trial.arena.minds.get(&id).ok_or("mind")?.strikes > 0);
    assert!(trial.meter.measurement().is_err());
    Ok(())
}

#[test]
fn lower_class_source_operations_fault_is_retained_without_tested_class_promotion() -> TestResult {
    let source =
        "fn think(v,c) { if this.contains(\"landed\") { while true {} } this.landed = true; }";
    let (mut trial, _) = open(source, RatingClass::II, false)?;
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    let before = trial.meter.measurement()?;
    assert!(before.summary.clean());
    mutate_core(&mut trial.arena, id, Consequence::DegradeCore)?;
    let mut refused = false;
    for _ in 0..80 {
        match advance(&mut trial) {
            Ok(None) => {}
            Ok(Some(_)) => return Err("lower-class exhaustion is not a tested-class result".into()),
            Err(RatingError::Invalid) => {
                refused = true;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert!(refused);
    let after = trial.meter.measurement()?;
    assert_eq!(after.summary, before.summary);
    assert_eq!(after.seen, before.seen);
    let physical = after.physical.ok_or("physical measurement")?;
    assert_eq!(physical.degraded_ops_faults, 1);
    assert_eq!(physical.degraded_thinks, 1);
    assert_eq!(trial.arena.minds.get(&id).ok_or("mind")?.strikes, 1);
    assert_eq!(trial.arena.mind_class(id), CoreClass::I);
    Ok(())
}

#[test]
fn physical_cycle_trial_reaches_real_terminal_with_identical_unmetered_hashes() -> TestResult {
    let (mut trial, subject) = open(SOURCE, RatingClass::I, true)?;
    assert!(trial.cycle && trial.profiles > 1);
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
    let before = trial.meter.measurement()?;
    let mut twin = trial.arena.clone();
    for id in &trial.candidates {
        twin.runners.insert(*id, subject.runner());
    }
    let mut terminal = None;
    let limit = trial.case.maximum_ms / u64::from(trial.case.physics.native_ms) + 4;
    for _ in 0..limit {
        terminal = advance(&mut trial)?;
        let out = twin.step_settled(trial.wall, &[], &[]);
        drain(&mut twin, &out.ledger_chunks)?;
        assert_eq!(
            crate::arena::state_hash(&trial.arena),
            crate::arena::state_hash(&twin)
        );
        assert_eq!(trial.arena.ledger.digest(), twin.ledger.digest());
        assert_eq!(trial.arena.now_ms, twin.now_ms);
        if terminal.is_some() {
            break;
        }
    }
    let terminal = terminal.ok_or("ordinary real combat must reach its bounded terminal")?;
    assert!(trial.arena.ended.is_some() && twin.ended.is_some());
    assert!(
        trial.arena.now_ms >= 10_000,
        "crossed the real cycle deadline"
    );
    assert!(
        trial.next_cycle_ms > 10_000,
        "physical ROM did not make cycling fail"
    );
    assert!(matches!(terminal.outcome(), CaseOutcome::Conformant {}));
    assert!(terminal.coverage().complete());
    assert_eq!(terminal.meter(), &before.summary);
    let observed = physical(&trial)?;
    assert!(observed.rom_thinks > 0);
    assert_eq!(observed.cancelled_uncalled, 1, "actual final wreck suffix");
    trial.meter.require_complete()?;
    Ok(())
}

#[test]
fn physical_loss_before_any_tested_invocation_cannot_satisfy_trial_coverage() -> TestResult {
    let (mut trial, _) = open(SOURCE, RatingClass::I, false)?;
    let id = candidate(&trial)?;
    mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
    let limit = trial.case.maximum_ms / u64::from(trial.case.physics.native_ms) + 4;
    let mut refused = false;
    for _ in 0..limit {
        match advance(&mut trial) {
            Ok(None) => {}
            Ok(Some(_)) => return Err("ROM-only work cannot certify the submitted source".into()),
            Err(RatingError::Invalid) if trial.arena.ended.is_some() => {
                refused = true;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert!(
        refused,
        "real terminal retains missing tested-source coverage refusal"
    );
    let measured = trial.meter.measurement()?;
    assert_eq!(measured.summary.thinks(), 0);
    assert!(measured.seen.iter().flatten().all(|count| *count == 0));
    assert!(physical(&trial)?.rom_thinks > 0);
    Ok(())
}

#[test]
fn physical_loss_after_real_source_fault_does_not_erase_the_fault() -> TestResult {
    for (source, ops) in [
        ("fn think(v,c) { while true {} }", true),
        ("fn think(v,c) { throw \"expected runtime fault\"; }", false),
    ] {
        let (mut trial, _) = open(source, RatingClass::I, false)?;
        let id = candidate(&trial)?;
        // Run the same native barrier but delay evidence reconciliation so
        // actual damage follows the measured fault within this test boundary.
        for _ in 0..80 {
            trial.wall += i64::from(trial.case.physics.native_ms);
            let out = trial.arena.step_settled(trial.wall, &[], &[]);
            drain(&mut trial.arena, &out.ledger_chunks)?;
            if trial.arena.minds.get(&id).ok_or("mind")?.strikes != 0 {
                break;
            }
            trial.meter.reconcile(&trial.arena)?;
        }
        assert_eq!(trial.arena.minds.get(&id).ok_or("mind")?.strikes, 1);
        mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
        let joined = trial.meter.reconcile(&trial.arena);
        if ops {
            joined?;
            let measured = trial.meter.measurement()?;
            assert!(measured.summary.only_budget_faults());
            assert_eq!(measured.summary.ops_faults(), 1);
            assert_eq!(measured.summary.thinks(), 1);
        } else {
            assert!(
                joined.is_err(),
                "real interpreter fault remains a refusal after core loss"
            );
            assert!(trial.meter.measurement().is_err());
        }
    }
    Ok(())
}
