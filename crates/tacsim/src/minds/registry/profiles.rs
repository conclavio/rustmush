//! Complete original-profile traversal through real commands and real delayed
//! answers. This private arena retains evolving source memory. Every switch
//! deliberately crosses a collected answer, then checks exact cancellation.
use super::{
    identity,
    manifest::FrozenPolicy,
    plan::{Case, Physics},
    probes, runtime,
    subject::PreparedSubject,
};
use crate::arena::{
    Arena, Channel,
    author_trace::{Cancelled, Outcome},
    core::CoreClass,
    intents::{Intent, IntentKind, IntentOutcome, IntentSource, apply_intent},
    mind::run_batch_inline,
    profile_rules::RuleLease,
};
use mush_core::ObjId;
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, Coverage, RatingClass,
    RatingError,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
const OWN: ObjId = ObjId(1);
pub(super) struct Cycle {
    arena: Arena,
    case: Case,
    installed: runtime::Installed,
    wall: i64,
    targets: VecDeque<u8>,
    next_switch: i64,
    pending: BTreeMap<u64, u8>,
    applied: BTreeSet<u8>,
    last_applied: Option<u64>,
    changes: Vec<(u8, i64, u64, bool)>,
    refusals: u32,
    profile_count: u8,
    done: bool,
}
impl Cycle {
    pub(super) fn physics(policy: &FrozenPolicy, class: CoreClass) -> Result<Physics, RatingError> {
        super::kernels::Boundary::physics(policy, class)
    }
    pub(super) fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        if !matches!(case.key.kind(), CaseKind::ProfileCycle {}) || case.maximum_thinks != 256 {
            return Err(RatingError::Identity);
        }
        let class = match case.key.class() {
            RatingClass::I => CoreClass::I,
            RatingClass::II => CoreClass::II,
            RatingClass::III => CoreClass::III,
        };
        let (mut arena, rom) = probes::kernel_opening(policy, class)?;
        if Self::physics(policy, class)? != case.physics {
            return Err(RatingError::Identity);
        }
        let variant = subject.variants().first().ok_or(RatingError::Incomplete)?;
        if variant.cycle
            || variant.initial != 0
            || variant
                .definition
                .validate()
                .map_err(|_| RatingError::Invalid)?
                != subject.subject().definition()
        {
            return Err(RatingError::Identity);
        }
        let profile_count =
            u8::try_from(variant.definition.profiles.len()).map_err(|_| RatingError::Bound)?;
        let targets = (1..profile_count)
            .chain((0..profile_count.saturating_sub(1)).rev())
            .collect();
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            0,
            class,
            &BTreeMap::from([(OWN, rom)]),
            case.maximum_thinks,
        )?;
        arena.traces.set_enabled(true);
        Ok(Self {
            arena,
            case: case.clone(),
            installed,
            wall: 0,
            targets,
            next_switch: 10_000,
            pending: BTreeMap::new(),
            applied: BTreeSet::new(),
            last_applied: None,
            changes: Vec::new(),
            refusals: 0,
            profile_count,
            done: false,
        })
    }
    pub(super) fn binding(&self) -> &CaseBinding {
        &self.installed.binding
    }
    pub(super) fn advance(
        &mut self,
        allowance_ms: u64,
    ) -> Result<super::trials::Slice, RatingError> {
        let cadence = u64::from(self.case.physics.native_ms);
        if self.done || allowance_ms < cadence {
            return Err(RatingError::Invalid);
        }
        let before = self.arena.now_ms;
        self.wall = self
            .wall
            .checked_add(i64::from(self.case.physics.native_ms))
            .ok_or(RatingError::Bound)?;
        let out = self.arena.step(self.wall, &[], &[]);
        if self.arena.ended.is_some() {
            return Err(RatingError::Incomplete);
        }
        let used = u64::try_from(self.arena.now_ms - before).map_err(|_| RatingError::Bound)?;
        if used > cadence {
            return Err(RatingError::Bound);
        }
        let mut newest = None;
        for think in &out.thinks {
            if think.unit != OWN || self.pending.len() >= 256 {
                return Err(RatingError::Identity);
            }
            let index = self
                .arena
                .profiles
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .profiles
                .names()
                .enumerate()
                .find_map(|(index, _)| {
                    let index = u8::try_from(index).ok()?;
                    (self.arena.profiles.get(&OWN)?.profiles.key(index) == Some(think.card.key()))
                        .then_some(index)
                })
                .ok_or(RatingError::Identity)?;
            if self.pending.insert(think.think_seq, index).is_some() {
                return Err(RatingError::Identity);
            }
            newest = Some((think.think_seq, think.profile_token));
        }
        let answers = run_batch_inline(&out.thinks);
        let mut switched = false;
        if let (Some(target), Some((seq, token))) = (self.targets.front().copied(), newest)
            && self.arena.now_ms >= self.next_switch
        {
            let old_memory = self
                .arena
                .minds
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .memory
                .clone();
            let had_sample = answers.iter().any(|a| a.result.rule_sample.is_some());
            let intent = Intent {
                unit: OWN,
                channel: Channel::Helm,
                epoch: self.arena.epochs.current(OWN, Channel::Helm),
                kind: IntentKind::Profile { index: target },
                seq,
            };
            if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
                != IntentOutcome::AppliedContinuous
            {
                return Err(RatingError::Incomplete);
            }
            let runtime = self
                .arena
                .profiles
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?;
            let state = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
            if runtime.accepts(token)
                || runtime.lease != RuleLease::default()
                || state.pending.is_some()
                || state.memory != old_memory
            {
                return Err(RatingError::Invalid);
            }
            // The ordinary cooldown must reject the immediately reversed
            // command without changing the just-selected immutable identity.
            let accepted = runtime.token();
            let back = out.thinks.last().ok_or(RatingError::Incomplete)?.card.key();
            let back = self
                .arena
                .profiles
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .profiles
                .names()
                .enumerate()
                .find_map(|(index, _)| {
                    let index = u8::try_from(index).ok()?;
                    (self.arena.profiles.get(&OWN)?.profiles.key(index) == Some(back))
                        .then_some(index)
                })
                .ok_or(RatingError::Identity)?;
            let mut too_soon = intent;
            too_soon.kind = IntentKind::Profile { index: back };
            if apply_intent(&mut self.arena, &too_soon, IntentSource::Seat)
                != IntentOutcome::Refused(crate::arena::intents::Refusal::Profile)
                || self
                    .arena
                    .profiles
                    .get(&OWN)
                    .ok_or(RatingError::Incomplete)?
                    .token()
                    != accepted
            {
                return Err(RatingError::Invalid);
            }
            self.targets.pop_front();
            self.next_switch = self
                .arena
                .now_ms
                .checked_add(10_000)
                .ok_or(RatingError::Bound)?;
            for think in &out.thinks {
                self.changes
                    .push((target, self.arena.now_ms, think.think_seq, had_sample));
            }
            switched = true;
        }
        let answered = u32::try_from(answers.len()).map_err(|_| RatingError::Bound)?;
        let settled = self.arena.deliver_answers(answers);
        if settled.overflows != 0
            || (switched && (settled.dropped != answered || settled.written != 0))
            || (!switched && settled.dropped != 0)
        {
            return Err(RatingError::Invalid);
        }
        for row in self.arena.traces.recent(OWN, None) {
            let Some(index) = self.pending.get(&row.think_seq).copied() else {
                continue;
            };
            if !row.settled || matches!(row.outcome, Outcome::Pending) {
                continue;
            }
            if row.rom || row.rom_drop.is_some() || row.diagnostics.omitted_intents() != 0 {
                return Err(RatingError::Invalid);
            }
            match row.outcome {
                Outcome::Applied => {
                    self.applied.insert(index);
                    self.last_applied = Some(
                        self.last_applied
                            .map_or(row.think_seq, |old| old.max(row.think_seq)),
                    );
                }
                Outcome::Cancelled {
                    reason: Cancelled::Superseded,
                } if self
                    .changes
                    .iter()
                    .any(|(_, _, seq, _)| *seq == row.think_seq) => {}
                Outcome::Faulted {
                    fault: crate::arena::author_trace::Fault::Ops,
                } => {}
                _ => return Err(RatingError::Invalid),
            }
            self.refusals = self
                .refusals
                .checked_add(
                    u32::try_from(
                        row.intents
                            .iter()
                            .filter(|i| matches!(i.outcome, Outcome::Refused { .. }))
                            .count(),
                    )
                    .map_err(|_| RatingError::Bound)?,
                )
                .ok_or(RatingError::Bound)?;
            self.pending.remove(&row.think_seq);
        }
        super::trials::drain(&mut self.arena, &out.ledger_chunks)?;
        let measured = self.installed.meter.measurement()?;
        let budget = measured.summary.only_budget_faults();
        if !measured.summary.clean() && !budget {
            return Err(RatingError::Invalid);
        }
        let state = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        if state.dropped.is_some() || (!budget && state.strikes != 0) {
            return Err(RatingError::Invalid);
        }
        let complete = self.targets.is_empty()
            && self.applied.len() == usize::from(self.profile_count)
            // A previously visited profile cannot complete the return switch.
            // Require an actual new landing after the final cancelled answer.
            && self.changes.last().is_none_or(|(_, _, cancelled, _)| {
                self.last_applied.is_some_and(|landed| landed > *cancelled)
            })
            && measured.summary.thinks() >= 2;
        if !complete && !budget {
            if measured.summary.thinks() >= self.case.maximum_thinks
                || u64::try_from(self.arena.now_ms).map_err(|_| RatingError::Bound)?
                    > self.case.maximum_ms
            {
                return Err(RatingError::Incomplete);
            }
            return Ok(super::trials::Slice {
                simulated_ms: used,
                result: None,
            });
        }
        let required = u32::from(self.profile_count);
        let evaluated = u32::try_from(self.applied.len()).map_err(|_| RatingError::Bound)?;
        let coverage = Coverage::new(
            Applicability::Armed,
            required,
            evaluated,
            identity(
                "tacsim.registry.profiles.coverage.v1",
                &(
                    &self.changes,
                    &self.applied,
                    measured.witness,
                    &measured.seen,
                    self.refusals,
                    crate::arena::state_hash(&self.arena),
                ),
                16 * 1024,
            )?,
        )?;
        let result = CaseResult::new(
            self.case.key.clone(),
            self.installed.binding.clone(),
            measured.summary,
            coverage,
            self.refusals,
            if budget {
                CaseOutcome::BudgetExceeded {}
            } else {
                CaseOutcome::Conformant {}
            },
            None,
        )?;
        self.done = true;
        Ok(super::trials::Slice {
            simulated_ms: used,
            result: Some(result),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    #[test]
    fn every_profile_runs_with_evolving_memory_across_real_cancelled_answers() -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { stock::rules(); if this.contains(\"count\") { this.count += 1; } else { this.count=1; } this.bias=c.get(\"bias\"); }";
        let revision = super::super::tests::revision(source)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        let case = Case {
            key: mush_roll::commerce::rating::CaseKey::new(
                RatingClass::I,
                CaseKind::ProfileCycle {},
            )?,
            physics: Cycle::physics(&policy, CoreClass::I)?,
            maximum_ms: 256_250,
            maximum_thinks: 256,
        };
        let mut cycle = Cycle::open(&policy, &subject, &case)?;
        let mut result = None;
        for _ in 0..1025 {
            let slice = cycle.advance(u64::from(case.physics.native_ms))?;
            assert!(slice.simulated_ms <= u64::from(case.physics.native_ms));
            if slice.result.is_some() {
                result = slice.result;
                break;
            }
        }
        let result = result.ok_or("complete traversal")?;
        assert!(matches!(result.outcome(), CaseOutcome::Conformant {}));
        assert!(result.coverage().complete());
        assert_eq!(cycle.applied, BTreeSet::from([0, 1]));
        assert_eq!(
            cycle
                .changes
                .iter()
                .map(|(index, _, _, _)| *index)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert!(cycle.changes.iter().all(|(_, _, _, sample)| *sample));
        let cancelled = cycle.changes.last().ok_or("last switch")?.2;
        assert!(cycle.last_applied.is_some_and(|landed| landed > cancelled));
        assert!(
            matches!(cycle.changes.as_slice(), [(_,first,_,_),(_,second,_,_)] if *first>=10_000 && *second-*first>=10_000)
        );
        let memory: serde_json::Value =
            serde_json::from_slice(&cycle.arena.minds.get(&OWN).ok_or("mind")?.memory)?;
        assert!(
            memory
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|n| n > 2)
        );
        assert_eq!(
            memory.get("bias").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(cycle.advance(u64::from(case.physics.native_ms)).is_err());
        Ok(())
    }
}
