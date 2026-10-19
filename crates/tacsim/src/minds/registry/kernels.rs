//! Trusted, separately identified engine-boundary invocations. They run real
//! compiled source through the production Rhai runner, collection and landing.
//! Their one private fee injector exercises the sandbox's post-return check;
//! it is never registered on a submitted source's sandbox.
use super::{
    identity,
    manifest::FrozenPolicy,
    meter::{Binding, MeteredRunner},
    plan::{Case, Physics},
    probes, runtime,
    subject::PreparedSubject,
};
use crate::{
    arena::{
        Arena,
        core::{CoreClass, CoreProfile},
        motor::MotorCounters,
    },
    minds::RhaiRunner,
};
use mush_core::ObjId;
use mush_mind::{Digest, EngineLimits, Sandbox};
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, Coverage, KernelName,
    KernelOutcome, RatingClass, RatingError,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

const OWN: ObjId = ObjId(1);
/// The generated source and the exact expected boundary are part of the frozen
/// manifest. Class dimensions are captured from the real operator table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipe {
    source: String,
    expected: KernelOutcome,
    injected_fee: Option<u64>,
}
pub(super) fn recipe(name: KernelName, profile: CoreProfile) -> Result<Recipe, RatingError> {
    let (source,expected,injected_fee)=match name {
        KernelName::MemoryKeysAt | KernelName::MemoryKeysOver=>{
            let keys=profile.memory_keys.checked_add(usize::from(name==KernelName::MemoryKeysOver)).ok_or(RatingError::Bound)?;
            if keys>129 {return Err(RatingError::Bound);}
            let rows=(0..keys).map(|n|format!("k{n:03}:0")).collect::<Vec<_>>().join(",");
            (format!("fn think(v,c) {{ this = #{{{rows}}}; }}"),if name==KernelName::MemoryKeysAt {KernelOutcome::Passed}else{KernelOutcome::Memory},None)
        }
        KernelName::MemoryValueAt | KernelName::MemoryValueOver=>{
            let n=crate::arena::mind::STOCK_MEMORY_LIMITS.string_bytes+usize::from(name==KernelName::MemoryValueOver);
            if n>256 {return Err(RatingError::Bound);}
            (format!("fn think(v,c) {{ this.value = \"{}\"; }}","x".repeat(n)),if name==KernelName::MemoryValueAt {KernelOutcome::Passed}else{KernelOutcome::Memory},None)
        }
        KernelName::RulesSlotsAt=>("fn think(v,c) { fire(v.weapons[0].id, slots()); }".into(),KernelOutcome::Passed,None),
        KernelName::RulesSlotsOver=>("fn think(v,c) { fire(v.weapons[0].id, slots()+1); }".into(),KernelOutcome::Unknown,None),
        KernelName::MeterFinalFee=>("fn think(v,c) { registry_fee_last(); }".into(),KernelOutcome::Ops,Some(profile.ops)),
        KernelName::MeterCallback=>("fn spin(x) { let n=0; loop { n+=1; } } fn think(v,c) { try { [1].map(spin); } catch { } }".into(),KernelOutcome::Ops,None),
    };
    if source.len() > 8 * 1024 {
        return Err(RatingError::Bound);
    }
    Ok(Recipe {
        source,
        expected,
        injected_fee,
    })
}
pub(super) struct Boundary {
    arena: Arena,
    case: Case,
    meter: Arc<MeteredRunner>,
    binding: CaseBinding,
    recipe: Recipe,
    done: bool,
}
impl Boundary {
    pub(super) fn physics(policy: &FrozenPolicy, class: CoreClass) -> Result<Physics, RatingError> {
        let (arena, _) = probes::kernel_opening(policy, class)?;
        Ok(Physics {
            units: 1,
            native_ms: u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?,
        })
    }
    pub(super) fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        let CaseKind::Kernel { name } = case.key.kind() else {
            return Err(RatingError::Identity);
        };
        let class = match case.key.class() {
            RatingClass::I => CoreClass::I,
            RatingClass::II => CoreClass::II,
            RatingClass::III => CoreClass::III,
        };
        let profile = policy.cfg.minds.classes.row(class);
        let recipe = recipe(name, profile)?;
        let (mut arena, rom) = probes::kernel_opening(policy, class)?;
        if case.physics != Self::physics(policy, class)? || case.maximum_thinks != 1 {
            return Err(RatingError::Identity);
        }
        let variant = subject.variants().first().ok_or(RatingError::Incomplete)?;
        // This only hydrates the actual schema/card/program. The submitted
        // runner is not called by an engine probe, nor counted as coverage.
        let _installed = runtime::install(
            &mut arena,
            subject,
            variant,
            0,
            class,
            &BTreeMap::from([(OWN, rom)]),
            1,
        )?;
        let current = arena.profiles.get(&OWN).ok_or(RatingError::Incomplete)?;
        let binding = Binding::capture(&current.card, &current.program)?;
        let binding = BTreeMap::from([(OWN, vec![binding])]);
        let fee = recipe.injected_fee;
        let sandbox = Arc::new(Sandbox::new(&EngineLimits::default(), move |engine| {
            crate::minds::register_api(engine);
            if let Some(fee) = fee {
                engine.register_fn("registry_fee_last", move || {
                    mush_mind::budget::charge(fee);
                });
            }
        }));
        let source = mush_roll::source_digest(&recipe.source);
        let runner = Arc::new(
            RhaiRunner::on(
                sandbox,
                &policy.cfg.arena,
                &recipe.source,
                Digest(*source.as_bytes()),
            )
            .map_err(|_| RatingError::Invalid)?,
        );
        let case_binding = CaseBinding::new(
            source,
            variant
                .definition
                .validate()
                .map_err(|_| RatingError::Invalid)?,
            identity("tacsim.registry.kernel.cards.v1", &binding, 512 * 1024)?,
            identity(
                "tacsim.registry.kernel.program.v1",
                &current.program,
                64 * 1024,
            )?,
        );
        let meter = Arc::new(MeteredRunner::new(runner, profile, binding, 1)?);
        arena.runners.insert(OWN, meter.clone());
        arena.traces.set_enabled(true);
        Ok(Self {
            arena,
            case: case.clone(),
            meter,
            binding: case_binding,
            recipe,
            done: false,
        })
    }
    pub(super) fn binding(&self) -> &CaseBinding {
        &self.binding
    }
    pub(super) fn advance(
        &mut self,
        allowance_ms: u64,
    ) -> Result<super::trials::Slice, RatingError> {
        if self.done || allowance_ms < u64::from(self.case.physics.native_ms) {
            return Err(RatingError::Invalid);
        }
        let mut counters = MotorCounters::default();
        self.arena.collect_think(OWN, 0, 0, &mut counters);
        let think = self.arena.collected.pop().ok_or(RatingError::Incomplete)?;
        if !self.arena.collected.is_empty()
            || think.unit != OWN
            || think.dropped
            || !think.memory.is_empty() && think.memory != b"{}"
        {
            return Err(RatingError::Identity);
        }
        let answer = think.think().result;
        let actual = match answer.fault.as_ref().map(|f| f.cause.as_str()) {
            None => KernelOutcome::Passed,
            Some("ops") => KernelOutcome::Ops,
            Some("memory") => KernelOutcome::Memory,
            Some("unknown") => KernelOutcome::Unknown,
            Some("limit") => KernelOutcome::Limit,
            Some("type") => KernelOutcome::Type,
            _ => return Err(RatingError::Invalid),
        };
        if actual != self.recipe.expected {
            return Err(RatingError::Invalid);
        }
        let measured = self.meter.measurement()?;
        let settled = self.arena.settle_collected(&think, answer);
        self.arena.now_ms = think.lands_ms;
        self.arena
            .mind_land(OWN, think.think_seq, think.lands_ms, &mut counters);
        if settled.overflows != 0
            || settled.dropped != 0
            || settled.faults != u32::from(actual != KernelOutcome::Passed)
        {
            return Err(RatingError::Invalid);
        }
        let state = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        if state.dropped.is_some() || state.strikes != u8::from(actual != KernelOutcome::Passed) {
            return Err(RatingError::Invalid);
        }
        let traces = self.arena.traces.recent(OWN, None);
        let trace = traces
            .iter()
            .find(|row| row.think_seq == think.think_seq)
            .ok_or(RatingError::Incomplete)?;
        if !trace.settled || trace.diagnostics.omitted_intents() != 0 {
            return Err(RatingError::Incomplete);
        }
        let refusals = u32::try_from(
            trace
                .intents
                .iter()
                .filter(|intent| {
                    matches!(
                        intent.outcome,
                        crate::arena::author_trace::Outcome::Refused { .. }
                    )
                })
                .count(),
        )
        .map_err(|_| RatingError::Bound)?;
        let witness = identity(
            "tacsim.registry.kernel.coverage.v1",
            &(
                &self.recipe,
                actual,
                measured.witness,
                crate::arena::state_hash(&self.arena),
            ),
            16 * 1024,
        )?;
        let result = CaseResult::new(
            self.case.key.clone(),
            self.binding.clone(),
            measured.summary,
            Coverage::new(Applicability::EngineBoundary, 1, 1, witness)?,
            refusals,
            CaseOutcome::Kernel {
                expected: self.recipe.expected,
                actual,
            },
            None,
        )?;
        self.done = true;
        Ok(super::trials::Slice {
            simulated_ms: u64::from(self.case.physics.native_ms),
            result: Some(result),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_boundaries_use_real_runner_and_exact_expected_faults()
    -> Result<(), Box<dyn std::error::Error>> {
        let policy = super::super::tests::frozen_policy()?;
        // A submitted source which would fault must not be executed or credited
        // by these independently named engine boundary invocations.
        let source = "fn think(v,c) { throw \"candidate never runs here\"; }";
        let revision = super::super::tests::revision(source)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        for class in [RatingClass::I, RatingClass::II, RatingClass::III] {
            let core = match class {
                RatingClass::I => CoreClass::I,
                RatingClass::II => CoreClass::II,
                RatingClass::III => CoreClass::III,
            };
            for name in [
                KernelName::MemoryKeysAt,
                KernelName::MemoryKeysOver,
                KernelName::MemoryValueAt,
                KernelName::MemoryValueOver,
                KernelName::RulesSlotsAt,
                KernelName::RulesSlotsOver,
                KernelName::MeterFinalFee,
                KernelName::MeterCallback,
            ] {
                let case = Case {
                    key: mush_roll::commerce::rating::CaseKey::new(
                        class,
                        CaseKind::Kernel { name },
                    )?,
                    physics: Boundary::physics(&policy, core)?,
                    maximum_ms: 10_000,
                    maximum_thinks: 1,
                };
                let mut probe = Boundary::open(&policy, &subject, &case)?;
                assert_ne!(probe.binding().source(), revision.source());
                let row = probe
                    .advance(u64::from(case.physics.native_ms))?
                    .result
                    .ok_or("boundary")?;
                assert_eq!(row.meter().thinks(), 1);
                assert!(
                    row.meter().max_ops() > 0
                        && row.meter().max_fees() >= crate::minds::fees::FEE_THINK
                );
                assert!(
                    matches!(row.outcome(),CaseOutcome::Kernel{expected,actual} if expected==actual)
                );
                if name == KernelName::MeterFinalFee {
                    assert!(row.meter().max_fees() > policy.cfg.minds.classes.row(core).ops);
                }
                assert!(probe.advance(u64::from(case.physics.native_ms)).is_err());
            }
        }
        Ok(())
    }
}
