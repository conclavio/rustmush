use super::*;
use crate::arena::{
    mind::{CollectedThink, MindRunner},
    motor::MotorCounters,
    state::{Controller, Power, Team},
    test_support::{arena_of, line_tank, pose},
};
use crate::minds::registry::{subject::PreparedSubject, tests::revision};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const OWN: ObjId = ObjId(4);
const SOURCE: &str = "fn think(v,c) { this.calls = 1; }";

fn fixture(maximum: u64) -> Result<(Arena, Arc<MeteredRunner>), Box<dyn std::error::Error>> {
    fixture_source(maximum, SOURCE)
}

fn fixture_source(
    maximum: u64,
    source: &str,
) -> Result<(Arena, Arc<MeteredRunner>), Box<dyn std::error::Error>> {
    let mut hull = line_tank(OWN, Team::Red, pose(1500, 4500));
    hull.controller = Controller::Mind;
    hull.reseal(&crate::match_::SealTerms::fixture())?;
    let mut arena = arena_of(vec![
        hull,
        line_tank(ObjId(9), Team::Blue, pose(6500, 4500)),
    ]);
    arena.unit_mut(OWN).ok_or("hull")?.power = Power::Online;
    let source = PreparedSubject::prepare(
        &revision(source)?,
        source,
        &crate::config::ArenaConfig::default(),
    )?;
    let runtime = arena.profiles.get(&OWN).ok_or("runtime")?;
    let meter = Arc::new(MeteredRunner::new(
        source.runner(),
        arena.mind_profile(OWN),
        BTreeMap::from([(
            OWN,
            vec![Binding::capture(&runtime.card, &runtime.program)?],
        )]),
        maximum,
    )?);
    arena.runners.insert(OWN, meter.clone());
    arena.traces.set_enabled(true);
    meter.enable_physical(&arena)?;
    Ok((arena, meter))
}

fn collect(arena: &mut Arena, at: i64) -> Result<CollectedThink, Box<dyn std::error::Error>> {
    arena.now_ms = at;
    let sequence = arena.minds.get(&OWN).ok_or("mind")?.think_seq;
    arena.collect_think(OWN, sequence, at, &mut MotorCounters::default());
    Ok(arena.collected.pop().ok_or("actual collected think")?)
}

#[test]
fn intact_core_cannot_authenticate_forged_rom_or_lower_class() -> TestResult {
    for rom in [false, true] {
        let (mut arena, meter) = fixture(8)?;
        let think = collect(&mut arena, 1000)?;
        let profile = arena.classes.row(CoreClass::Zero);
        let mut job = think.job();
        job.profile = &profile;
        job.dropped = rom;
        let _actual_unmodified_answer = meter.think(&job);
        check!(meter.reconcile(&arena).is_err());
        check!(
            meter.measurement().is_err(),
            "failed physical proof is sticky"
        );
    }
    Ok(())
}

#[test]
fn invocation_identity_rejects_wrong_binding_profile_and_duplicate_or_gap_sequence() -> TestResult {
    for mode in 0..5 {
        let (mut arena, meter) = fixture(8)?;
        let think = collect(&mut arena, 1000)?;
        let mut profile = *think.job().profile;
        profile.ops += 1;
        let mut job = think.job();
        match mode {
            0 => job.profile_token.unit = ObjId(99),
            1 => job.profile = &profile,
            2 => job.think_seq += 1,
            3 => job.profile_token.generation += 99,
            _ => {
                let _ = meter.think(&job);
            }
        }
        let actual = meter.think(&job);
        let plain = meter.runner.think(&job);
        check_eq!(actual, plain, "rejection never changes the real answer");
        check!(meter.reconcile(&arena).is_err());
    }
    Ok(())
}

#[test]
fn trace_join_is_required_and_pending_evidence_cannot_escape_as_terminal() -> TestResult {
    let (mut arena, meter) = fixture(8)?;
    let think = collect(&mut arena, 1000)?;
    let answer = think.think().result;
    check_eq!(arena.settle_collected(&think, answer).faults, 0);
    meter.reconcile(&arena)?;
    check_eq!(meter.measurement()?.summary.thinks(), 1);
    check!(
        meter.require_complete().is_err(),
        "answer has not landed yet"
    );
    arena.traces.set_enabled(false);
    check!(
        meter.reconcile(&arena).is_err(),
        "missing trace is not healthy empty evidence"
    );
    check!(meter.measurement().is_err());
    Ok(())
}

#[test]
fn invocation_bound_includes_rom_and_failed_evidence_never_resets_it() -> TestResult {
    let (mut arena, meter) = fixture(1)?;
    let think = collect(&mut arena, 1000)?;
    let answer = think.think().result;
    arena.settle_collected(&think, answer);
    arena.now_ms = think.lands_ms;
    arena.mind_land(
        OWN,
        think.think_seq,
        think.lands_ms,
        &mut MotorCounters::default(),
    );
    meter.reconcile(&arena)?;
    meter.require_complete()?;
    // A real M32 core loss makes the next ordinary collection a ROM job. The
    // single total cap remains occupied by the earlier source invocation.
    let consequence = crate::arena::resolution::Consequence::DestroyCore;
    let plan = mush_tactics::UnitMutationPlan::new(
        arena.core_ops(OWN, std::slice::from_ref(&consequence)),
        None,
        None,
    )?;
    let before = arena.unit(OWN).ok_or("hull")?.unit.clone();
    let (after, _) = mush_tactics::apply(&before, &plan, 0)?;
    arena.set_unit(OWN, after, Arena::plan_touches_bands(&plan));
    arena.apply_consequence(OWN, &consequence, arena.now_ms, None, &mut Vec::new());
    let next = collect(&mut arena, 3000)?;
    check!(next.job().dropped);
    let actual = next.think().result;
    check_eq!(actual, meter.runner.think(&next.job()));
    check!(meter.reconcile(&arena).is_err());
    let state = meter.state.lock().map_err(|_| "poison")?;
    check_eq!(
        state
            .physical
            .as_ref()
            .ok_or("physical")?
            .summary
            .total_thinks,
        1
    );
    Ok(())
}

#[test]
fn observation_storage_has_fixed_primitive_pending_bound_and_shared_source_bindings() -> TestResult
{
    let (_arena, meter) = fixture(8)?;
    let state = meter.state.lock().map_err(|_| "poison")?;
    let physical = state.physical.as_ref().ok_or("physical")?;
    let owner = state.units.get(&OWN).ok_or("source")?;
    let observed = physical.units.get(&OWN).ok_or("observed")?;
    check!(Arc::ptr_eq(&owner.bindings, &observed.bindings));
    check!(std::mem::size_of::<Invocation>() <= 128);
    check_eq!(MAX_PENDING_PER_UNIT, 64);
    Ok(())
}

#[test]
fn pending_overflow_refuses_without_evicting_the_oldest_proof() -> TestResult {
    let (mut arena, meter) = fixture(100)?;
    for n in 0..=MAX_PENDING_PER_UNIT {
        let think = collect(&mut arena, 1000 + i64::try_from(n)? * 1000)?;
        let answer = think.think().result;
        check!(answer.fault.is_none());
    }
    check!(meter.reconcile(&arena).is_err());
    let state = meter.state.lock().map_err(|_| "poison")?;
    let physical = state.physical.as_ref().ok_or("physical")?;
    let observed = physical.units.get(&OWN).ok_or("unit")?;
    check_eq!(observed.pending.len(), MAX_PENDING_PER_UNIT);
    check!(
        observed.pending.contains_key(&0),
        "no oldest-proof eviction"
    );
    check_eq!(
        physical.summary.total_thinks,
        u64::try_from(MAX_PENDING_PER_UNIT)?
    );
    Ok(())
}

#[test]
fn physical_mode_cannot_start_on_a_preexisting_trace_or_collected_job() -> TestResult {
    for collected in [false, true] {
        let (mut arena, meter) = fixture(8)?;
        if collected {
            let _ = collect(&mut arena, 1000)?;
        } else {
            check!(arena.traces.record(OWN, 7, 0, false, None, &[], &[], None));
        }
        let state = meter.state.lock().map_err(|_| "poison")?;
        check!(Physical::capture(&arena, &state.units, meter.profile).is_err());
    }
    Ok(())
}

mod terminal;

mod wreck_terminal;
