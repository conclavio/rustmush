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
        check!(advance(trial)?.is_none(), "no early fabricated terminal");
        if trial.meter.measurement()?.summary.thinks() > 0 {
            return Ok(());
        }
    }
    Err("actual source invocation did not arrive".into())
}

fn mutate_core(arena: &mut Arena, id: ObjId, consequence: Consequence) -> TestResult {
    let ops = arena.core_ops(id, &[consequence]);
    check!(!ops.is_empty(), "actual held core must change");
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
    check!(
        trial.arena.minds.get(&id).ok_or("mind")?.pending.is_some(),
        "first source answer has been measured but its native landing is still pending"
    );
    let before = trial.meter.measurement()?;
    mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
    check!(trial.arena.core_of(id).is_none());
    check!(advance(&mut trial)?.is_none());
    let first = trial.arena.minds.get(&id).ok_or("mind")?;
    check_eq!(first.dropped, Some(DropCause::Core));
    let memory: serde_json::Value = serde_json::from_slice(&first.memory)?;
    check_eq!(
        memory.get("calls").and_then(serde_json::Value::as_u64),
        Some(1),
        "the source answer collected before damage lands before any ROM memory replacement"
    );
    for _ in 0..80 {
        check!(advance(&mut trial)?.is_none());
        if physical(&trial)?.rom_thinks >= 2 {
            break;
        }
    }
    let after = trial.meter.measurement()?;
    check_eq!(
        after.summary,
        before.summary,
        "ROM gives no source cost or invocation credit"
    );
    check_eq!(after.seen, before.seen);
    check!(physical(&trial)?.rom_thinks >= 2);
    check_eq!(
        physical(&trial)?.total_thinks,
        before.summary.thinks() + physical(&trial)?.rom_thinks
    );
    let mind = trial.arena.minds.get(&id).ok_or("mind")?;
    check_eq!(mind.dropped, Some(DropCause::Core));
    check_eq!(mind.strikes, 0);
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
    check_eq!(
        trial.arena.core_of(id).ok_or("present core")?.class,
        CoreClass::I
    );
    for _ in 0..80 {
        check!(advance(&mut trial)?.is_none());
        if physical(&trial)?.degraded_thinks >= 2 {
            break;
        }
    }
    let degraded = physical(&trial)?;
    check!(
        degraded.degraded_thinks >= 2
            && degraded.degraded_max_ops > 0
            && degraded.degraded_max_fees > 0
    );
    check_eq!(degraded.degraded_ops_faults, 0);
    check_eq!(degraded.rom_thinks, 0);
    check_eq!(trial.meter.measurement()?.summary, before.summary);
    check_eq!(trial.meter.measurement()?.seen, before.seen);
    check_eq!(
        trial.arena.profiles.get(&id).ok_or("runtime")?.program,
        source_binding
    );
    check!(trial.arena.minds.get(&id).ok_or("mind")?.dropped.is_none());
    mutate_core(&mut trial.arena, id, Consequence::DegradeCore)?;
    check_eq!(
        trial.arena.core_of(id).ok_or("present Zero core")?.class,
        CoreClass::Zero
    );
    for _ in 0..80 {
        check!(advance(&mut trial)?.is_none());
        if physical(&trial)?.rom_thinks >= 2 {
            break;
        }
    }
    check!(physical(&trial)?.rom_thinks >= 2);
    check!(
        trial.arena.minds.get(&id).ok_or("mind")?.dropped.is_none(),
        "present class Zero legitimately uses stand-in without a Core drop marker"
    );
    check!(!trial.arena.profiles.get(&id).ok_or("runtime")?.dropped);
    check_eq!(trial.meter.measurement()?.summary, before.summary);
    check_eq!(trial.meter.measurement()?.seen, before.seen);
    Ok(())
}

#[test]
fn present_zero_does_not_hide_an_actual_stand_in_card_fault() -> TestResult {
    let (mut trial, _) = open(SOURCE, RatingClass::I, false)?;
    until_invoked(&mut trial)?;
    let id = candidate(&trial)?;
    check!(
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
    check!(
        refused,
        "real native stand-in profile fault is not excused by physical loss"
    );
    check_eq!(
        trial.arena.core_of(id).ok_or("present Zero core")?.class,
        CoreClass::Zero
    );
    check!(trial.arena.minds.get(&id).ok_or("mind")?.strikes > 0);
    check!(trial.meter.measurement().is_err());
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
    check!(before.summary.clean());
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
    check!(refused);
    let after = trial.meter.measurement()?;
    check_eq!(after.summary, before.summary);
    check_eq!(after.seen, before.seen);
    let physical = after.physical.ok_or("physical measurement")?;
    check_eq!(physical.degraded_ops_faults, 1);
    check_eq!(physical.degraded_thinks, 1);
    check_eq!(trial.arena.minds.get(&id).ok_or("mind")?.strikes, 1);
    check_eq!(trial.arena.mind_class(id), CoreClass::I);
    Ok(())
}

#[test]
fn physical_cycle_trial_reaches_real_terminal_with_identical_unmetered_hashes() -> TestResult {
    let (mut trial, subject) = open(SOURCE, RatingClass::I, true)?;
    check!(trial.cycle && trial.profiles > 1);
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
        check_eq!(
            crate::arena::state_hash(&trial.arena),
            crate::arena::state_hash(&twin)
        );
        check_eq!(trial.arena.ledger.digest(), twin.ledger.digest());
        check_eq!(trial.arena.now_ms, twin.now_ms);
        if terminal.is_some() {
            break;
        }
    }
    let terminal = terminal.ok_or("ordinary real combat must reach its bounded terminal")?;
    check!(trial.arena.ended.is_some() && twin.ended.is_some());
    check!(
        trial.arena.now_ms >= 10_000,
        "crossed the real cycle deadline"
    );
    check!(
        trial.next_cycle_ms > 10_000,
        "physical ROM did not make cycling fail"
    );
    check!(matches!(terminal.outcome(), CaseOutcome::Conformant {}));
    check!(terminal.coverage().complete());
    check_eq!(terminal.meter(), &before.summary);
    let observed = physical(&trial)?;
    check!(observed.rom_thinks > 0);
    check_eq!(observed.cancelled_uncalled, 1, "actual final wreck suffix");
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
    check!(
        refused,
        "real terminal retains missing tested-source coverage refusal"
    );
    let measured = trial.meter.measurement()?;
    check_eq!(measured.summary.thinks(), 0);
    check!(measured.seen.iter().flatten().all(|count| *count == 0));
    check!(physical(&trial)?.rom_thinks > 0);
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
        check_eq!(trial.arena.minds.get(&id).ok_or("mind")?.strikes, 1);
        mutate_core(&mut trial.arena, id, Consequence::DestroyCore)?;
        let joined = trial.meter.reconcile(&trial.arena);
        if ops {
            joined?;
            let measured = trial.meter.measurement()?;
            check!(measured.summary.only_budget_faults());
            check_eq!(measured.summary.ops_faults(), 1);
            check_eq!(measured.summary.thinks(), 1);
        } else {
            check!(
                joined.is_err(),
                "real interpreter fault remains a refusal after core loss"
            );
            check!(trial.meter.measurement().is_err());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Doc 44 unit MW.5 (doc 17g C4, item 4): a Registry trial ends the same on every host
// ---------------------------------------------------------------------------

/// A script that drives at whatever it has told apart, with the stock fire rules: movement,
/// sensing, the dice and the damage pipeline all take part in how its trial ends.
const CLOSES: &str = "fn think(v,c) { stock::rules(); for k in v.contacts { if !k.friendly && k.cell != () { goto(k.cell); return; } } }";

/// **The pinned ends** ([`c4_a_registry_trial_ends_the_same_on_every_host`]): for each case,
/// the millisecond its arena stopped at, the arena's final state hash, the black box's digest
/// and the SHA-256 of the case result the Registry would file (its JSON, the record a rating's
/// page carries: the key, the binding, the meter's summary, the coverage, the outcome and the
/// trial's own end).
///
/// Blessed on Linux (conclav.dev, x86-64) and run on Windows (x86-64) at the same commit; the
/// cut note (`docs/design/44-cut-notes.md`, MW.5) records both runs. A change that moves a row
/// on purpose (a new mind ABI, a rule's numbers, a trial's layout) re-blesses it from the
/// `C4_TRIAL_PIN` lines the test prints, and runs it on the other host before it merges.
const C4_TRIAL_PINS: [&str; 4] = [
    "stands/I/fixed at_ms=58500 state=2ac0bb5770d49505 ledger=9adf3f2351084758a774f3ee83c8ef3d8974373801a187cd6d5213e7d4649f89 result=e7f18842334322008f5aa8e0549777d46b4d0301d43928773ae558c5286f838c",
    "stands/I/cycle at_ms=58500 state=366ad86d479ee5d5 ledger=42f3888715bbf465e74e682e9e4face459e60ff14c45bcad2d5aa71ec56781f5 result=bc08d3501e32a8b64b79f4173c027189978159391e33bcf893c8f95675de93b7",
    "closes/I/fixed at_ms=58500 state=f8d70307b1c6f298 ledger=1a4f042b377cf8d81033bacbbcd671bb023d70a9a501d5f472e4adeefd734b32 result=18020786323446b38988cbe48da6c3d078f0be3405b63b856b1d6abdf4ee9706",
    "stock/II at_ms=58500 state=a1804cd413eba194 ledger=462bc14e1b6d950896415726d2c14d5aea91e819ed05a2119f7f13c85f88cadb result=b497f3b07b2699e436ab645e481bd57328941707ffb1d8c2429d0ec1f5d52311",
];

/// Lower-case base-16 of `bytes`.
fn base16(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Run `trial` to its terminal and print what a pin holds.
fn ended(label: &str, mut trial: Trial) -> Result<String, Box<dyn std::error::Error>> {
    use sha2::{Digest as _, Sha256};
    let limit = trial.case.maximum_ms / u64::from(trial.case.physics.native_ms) + 4;
    let mut terminal = None;
    for _ in 0..limit {
        terminal = advance(&mut trial)?;
        if terminal.is_some() {
            break;
        }
    }
    let result = terminal.ok_or("the trial reaches its bounded terminal")?;
    let wire = serde_json::to_string(&result)?;
    Ok(format!(
        "{label} at_ms={} state={:016x} ledger={} result={}",
        trial.arena.now_ms,
        crate::arena::state_hash(&trial.arena),
        base16(&trial.arena.ledger.digest()),
        base16(&Sha256::digest(wire.as_bytes())),
    ))
}

/// **Doc 17g C4, item 4: the Registry's trial of a source ends the same on every host.**
/// D-UM-28's corpus pins what a mind does in a match; nothing pinned what the Registry makes
/// of a mind, and a rating is filed, priced and bought on it. Four cases of the Duel trial at
/// seed 1 are run to their real terminals: a script that stands and fires by the stock rules
/// (its fixed and its cycling variant), a script that closes on what it sees, and the stock
/// profile at Class II. Each case's end is compared with its pin, byte for byte.
#[test]
fn c4_a_registry_trial_ends_the_same_on_every_host() -> TestResult {
    let lines = [
        ended("stands/I/fixed", open(SOURCE, RatingClass::I, false)?.0)?,
        ended("stands/I/cycle", open(SOURCE, RatingClass::I, true)?.0)?,
        ended("closes/I/fixed", open(CLOSES, RatingClass::I, false)?.0)?,
        ended("stock/II", open_stock(SOURCE, RatingClass::II)?.0)?,
    ];
    for line in &lines {
        eprintln!("C4_TRIAL_PIN {line}");
    }
    check_eq!(lines.as_slice(), C4_TRIAL_PINS.as_slice());
    Ok(())
}
