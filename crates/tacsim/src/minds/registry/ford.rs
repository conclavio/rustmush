//! The shipped ford is exercised through actual pilot custody and motor commands.
//! Candidate policy need not choose to navigate: it observes the opening, retains
//! its exact memory across custody, then observes a genuine motor terminal state.
//! Handover deliberately clears the pilot's motor. A new real request establishes
//! the terminal seen by the candidate; no motor result or memory is transplanted.
use super::{
    identity,
    manifest::FrozenPolicy,
    plan::{Case, Physics},
    probes, runtime,
    subject::PreparedSubject,
    trials::{self, Landings, Slice},
};
use crate::arena::{
    Arena, Channel, Controller, Power,
    author_trace::Outcome,
    core::CoreClass,
    doctrine::MachineState,
    intents::{Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, apply_intent},
    mind::run_batch_inline,
    motor::MotorCounters,
    perception::DriveOutcome,
    resolution::Consequence,
};
use mush_core::{DefinitionDigest, ObjId};
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, Coverage, ProbeName,
    RatingClass, RatingError,
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
const OWN: ObjId = ObjId(1);
const GOAL: (u16, u16) = (68, 97);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Opening,
    Pilot,
    Terminal,
}

pub(super) struct Ford {
    arena: Arena,
    case: Case,
    installed: runtime::Installed,
    wall: i64,
    phase: Phase,
    disabled: bool,
    observations: Landings,
    opening_seen: bool,
    terminal_thinks: BTreeSet<u64>,
    terminal_landed: bool,
    views: Vec<DefinitionDigest>,
    saved_memory: Option<Vec<u8>>,
    saved_seal: [u8; 32],
    saved_profile: Option<crate::arena::runtime_card::ProfileKey>,
    transfers: Vec<(i64, bool, u64, DefinitionDigest)>,
    route: Vec<(i64, (u16, u16))>,
    done: bool,
}
impl Ford {
    pub(super) fn physics(policy: &FrozenPolicy, class: CoreClass) -> Result<Physics, RatingError> {
        let (arena, _) = probes::ford_opening(policy, class)?;
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
        let CaseKind::Probe { pair, name } = case.key.kind() else {
            return Err(RatingError::Identity);
        };
        let disabled = match name {
            ProbeName::FordMobile => false,
            ProbeName::FordDisabled => true,
            _ => return Err(RatingError::Identity),
        };
        let class = match case.key.class() {
            RatingClass::I => CoreClass::I,
            RatingClass::II => CoreClass::II,
            RatingClass::III => CoreClass::III,
        };
        if case.maximum_thinks != 256 || case.physics != Self::physics(policy, class)? {
            return Err(RatingError::Identity);
        }
        let pair = subject
            .pairs()
            .get(usize::from(pair))
            .ok_or(RatingError::Identity)?;
        let variant = subject
            .variants()
            .get(usize::from(pair.variant))
            .ok_or(RatingError::Identity)?;
        let (mut arena, rom) = probes::ford_opening(policy, class)?;
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            pair.profile,
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
            phase: Phase::Opening,
            disabled,
            observations: Landings::new(&[OWN]),
            opening_seen: false,
            terminal_thinks: BTreeSet::new(),
            terminal_landed: false,
            views: Vec::new(),
            saved_memory: None,
            saved_seal: [0; 32],
            saved_profile: None,
            transfers: Vec::new(),
            route: Vec::new(),
            done: false,
        })
    }
    pub(super) fn binding(&self) -> &CaseBinding {
        &self.installed.binding
    }
    fn expected(&self) -> DriveOutcome {
        if self.disabled {
            DriveOutcome::NoRoute
        } else {
            DriveOutcome::Arrived
        }
    }
    fn goal(&mut self) -> Result<(), RatingError> {
        let intent = Intent {
            unit: OWN,
            channel: Channel::Helm,
            epoch: self.arena.epochs.current(OWN, Channel::Helm),
            kind: IntentKind::Goal(MotorGoal::Cell {
                x: GOAL.0,
                y: GOAL.1,
            }),
            seq: 0,
        };
        if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
            != IntentOutcome::AppliedContinuous
        {
            return Err(RatingError::Incomplete);
        }
        Ok(())
    }
    fn remember_transfer(&mut self, pilot: bool) -> Result<(), RatingError> {
        let mind = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        self.transfers
            .push((self.arena.now_ms, pilot, mind.think_seq, {
                let mut digest = Sha256::new();
                digest.update(b"tacsim.registry.ford.memory.v1\0");
                digest.update(&mind.memory);
                DefinitionDigest::from_bytes(digest.finalize().into())
            }));
        if self.transfers.len() > 2 {
            return Err(RatingError::Bound);
        }
        Ok(())
    }
    fn possess(&mut self) -> Result<(), RatingError> {
        let mind = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        self.saved_memory = Some(mind.memory.clone());
        self.saved_seal = self
            .arena
            .unit(OWN)
            .ok_or(RatingError::Incomplete)?
            .seal
            .digest;
        self.saved_profile = Some(
            self.arena
                .profiles
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .card
                .key(),
        );
        self.arena
            .set_controller(OWN, Controller::Pilot, MachineState::Patrol);
        self.check_custody(Controller::Pilot)?;
        self.remember_transfer(true)?;
        self.goal()?;
        self.phase = Phase::Pilot;
        Ok(())
    }
    fn check_custody(&self, controller: Controller) -> Result<(), RatingError> {
        let own = self.arena.unit(OWN).ok_or(RatingError::Incomplete)?;
        let mind = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        let runtime = self
            .arena
            .profiles
            .get(&OWN)
            .ok_or(RatingError::Incomplete)?;
        if own.controller != controller
            || own.seal.digest != self.saved_seal
            || self.saved_memory.as_deref() != Some(mind.memory.as_slice())
            || self.saved_profile != Some(runtime.card.key())
            || mind.strikes != 0
            || mind.dropped.is_some()
            || mind.pending.is_some()
        {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
    fn release(&mut self) -> Result<(), RatingError> {
        self.check_custody(Controller::Pilot)?;
        self.arena
            .set_controller(OWN, Controller::Mind, MachineState::Patrol);
        self.check_custody(Controller::Mind)?;
        if self.arena.motors.row(OWN).is_some() {
            return Err(RatingError::Invalid);
        }
        self.remember_transfer(false)?;
        // The outgoing motor was correctly removed. This is an actual fresh
        // command, at the reached cell for Mobile and the remaining route for
        // Disabled; its result is computed by the real motor below.
        self.goal()?;
        let now = self.arena.now_ms;
        if self.disabled {
            let mut journal = Vec::new();
            self.arena
                .apply_consequence(OWN, &Consequence::DisableMotion, now, None, &mut journal);
            if !journal.is_empty() {
                return Err(RatingError::Invalid);
            }
        }
        self.arena
            .motor_step(OWN, now, &mut MotorCounters::default());
        let frame = self
            .arena
            .frame_for(OWN, self.arena.clock.tempo())
            .ok_or(RatingError::Incomplete)?;
        if frame
            .motor
            .as_ref()
            .is_none_or(|m| m.requested_goal != GOAL || m.outcome != Some(self.expected()))
        {
            return Err(RatingError::Incomplete);
        }
        self.phase = Phase::Terminal;
        Ok(())
    }
    pub(super) fn advance(&mut self, allowance_ms: u64) -> Result<Slice, RatingError> {
        let native = u64::from(self.case.physics.native_ms);
        if self.done || allowance_ms < native {
            return Err(RatingError::Invalid);
        }
        let before = self.arena.now_ms;
        self.wall = self
            .wall
            .checked_add(i64::from(self.case.physics.native_ms))
            .ok_or(RatingError::Bound)?;
        let out = self.arena.step(self.wall, &[], &[]);
        let used = u64::try_from(self.arena.now_ms - before).map_err(|_| RatingError::Bound)?;
        if used > native || self.arena.ended.is_some() {
            return Err(RatingError::Incomplete);
        }
        for think in &out.thinks {
            if think.unit != OWN
                || self.phase == Phase::Pilot
                || self.views.len() >= 256
                || !think.view.weapons.is_empty()
                || think.view.power != Power::Online
                || think.dropped
            {
                return Err(RatingError::Incomplete);
            }
            let view = &think.view;
            let motor = view.motor.as_ref().map(|m| (m.requested_goal, m.outcome));
            self.views.push(identity(
                "tacsim.registry.ford.picture.v1",
                &(think.think_seq, view.t_ms, view.cell, motor, view.immobile),
                1024,
            )?);
            match self.phase {
                Phase::Opening => self.opening_seen = true,
                Phase::Terminal => {
                    let matches = motor == Some((GOAL, Some(self.expected())))
                        && view.immobile == self.disabled;
                    if self.terminal_thinks.is_empty() && !matches {
                        return Err(RatingError::Incomplete);
                    }
                    // Later legal candidate actions can replace the observed
                    // motor while the first terminal-observing answer lands.
                    if matches {
                        self.terminal_thinks.insert(think.think_seq);
                    }
                }
                Phase::Pilot => return Err(RatingError::Invalid),
            }
        }
        let settled = self.arena.deliver_answers(run_batch_inline(&out.thinks));
        if settled.overflows != 0 || settled.dropped != 0 {
            return Err(RatingError::Invalid);
        }
        self.observations.collect(&self.arena.traces)?;
        let applied = self
            .arena
            .traces
            .recent(OWN, None)
            .into_iter()
            .filter(|row| row.settled && matches!(row.outcome, Outcome::Applied))
            .map(|row| row.think_seq)
            .collect::<BTreeSet<_>>();
        self.terminal_landed = self.terminal_thinks.iter().any(|seq| applied.contains(seq));
        let measured = self.installed.meter.measurement()?;
        let budget = measured.summary.only_budget_faults();
        if (!measured.summary.clean() && !budget)
            || self
                .arena
                .minds
                .get(&OWN)
                .is_none_or(|s| s.dropped.is_some() || (!budget && s.strikes != 0))
        {
            return Err(RatingError::Invalid);
        }
        if !budget {
            match self.phase {
                Phase::Opening if self.opening_seen && !applied.is_empty() => self.possess()?,
                Phase::Pilot => {
                    self.check_custody(Controller::Pilot)?;
                    let own = self.arena.unit(OWN).ok_or(RatingError::Incomplete)?;
                    let cell = self
                        .arena
                        .definition
                        .terrain
                        .cell_of(&own.pose)
                        .ok_or(RatingError::Incomplete)?;
                    if self.route.last().is_none_or(|(_, old)| *old != cell) {
                        if self.route.len() >= 256 {
                            return Err(RatingError::Bound);
                        }
                        self.route.push((self.arena.now_ms, cell));
                    }
                    let arrived = self
                        .arena
                        .motors
                        .row(OWN)
                        .is_some_and(|m| m.outcome == Some(DriveOutcome::Arrived));
                    if (self.disabled && cell.0 >= 64) || (!self.disabled && arrived) {
                        self.release()?;
                    }
                }
                _ => {}
            }
        }
        // Include cancellation/refusal rows emitted by the real custody call.
        self.observations.collect(&self.arena.traces)?;
        trials::drain(&mut self.arena, &out.ledger_chunks)?;
        let complete = self.opening_seen
            && self.terminal_landed
            && self.transfers.len() == 2
            && self.route.iter().any(|(_, cell)| cell.0 >= 64);
        if !complete && !budget {
            if measured.summary.thinks() >= self.case.maximum_thinks
                || u64::try_from(self.arena.now_ms).map_err(|_| RatingError::Bound)?
                    > self.case.maximum_ms
            {
                return Err(RatingError::Incomplete);
            }
            return Ok(Slice {
                simulated_ms: used,
                result: None,
            });
        }
        let coverage = Coverage::new(
            Applicability::Unarmed,
            2,
            u32::from(self.opening_seen) + u32::from(self.terminal_landed),
            identity(
                "tacsim.registry.ford.coverage.v1",
                &(
                    &self.views,
                    &self.transfers,
                    &self.route,
                    self.disabled,
                    measured.witness,
                    &measured.seen,
                    crate::arena::state_hash(&self.arena),
                ),
                32 * 1024,
            )?,
        )?;
        let result = CaseResult::new(
            self.case.key.clone(),
            self.installed.binding.clone(),
            measured.summary,
            coverage,
            self.observations.refusals(),
            if budget {
                CaseOutcome::BudgetExceeded {}
            } else {
                CaseOutcome::Conformant {}
            },
            None,
        )?;
        self.done = true;
        Ok(Slice {
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
    fn real_ford_custody_preserves_candidate_memory_and_derives_both_terminal_outcomes()
    -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        // A legal policy that deliberately does not navigate. Its stored count
        // proves actual execution before/after the genuine custody interval.
        let source = "fn think(v,c) { stock::rules(); helm(0); speed(0); if this.contains(\"visits\") { this.visits += 1; } else { this.visits=1; } }";
        let subject = PreparedSubject::prepare(
            &super::super::tests::revision(source)?,
            source,
            &policy.cfg.arena,
        )?;
        for name in [ProbeName::FordMobile, ProbeName::FordDisabled] {
            let case = Case {
                key: mush_roll::commerce::rating::CaseKey::new(
                    RatingClass::I,
                    CaseKind::Probe { pair: 0, name },
                )?,
                physics: Ford::physics(&policy, CoreClass::I)?,
                maximum_ms: 260_000,
                maximum_thinks: 256,
            };
            let mut ford = Ford::open(&policy, &subject, &case)?;
            let mut result = None;
            let mut used = 0;
            for _ in 0..1041 {
                let slice = ford.advance(u64::from(case.physics.native_ms))?;
                used += slice.simulated_ms;
                if slice.result.is_some() {
                    result = slice.result;
                    break;
                }
            }
            let result = result.ok_or("ford completed")?;
            assert!(matches!(result.outcome(), CaseOutcome::Conformant {}));
            assert!(result.coverage().complete());
            assert!(result.meter().thinks() >= 2);
            assert_eq!(used, u64::try_from(ford.arena.now_ms)?);
            let [
                (_, true, first_seq, first_memory),
                (_, false, last_seq, last_memory),
            ] = ford.transfers.as_slice()
            else {
                return Err("two real transfers".into());
            };
            assert_eq!(
                first_seq, last_seq,
                "pilot interval contains no candidate invocation"
            );
            assert_eq!(
                first_memory, last_memory,
                "no memory copy/reset at handback"
            );
            assert!(ford.route.iter().any(|(_, cell)| cell.0 >= 64));
            let before: serde_json::Value =
                serde_json::from_slice(ford.saved_memory.as_deref().ok_or("saved memory")?)?;
            let after: serde_json::Value =
                serde_json::from_slice(&ford.arena.minds.get(&OWN).ok_or("mind")?.memory)?;
            assert!(
                after["visits"].as_u64().ok_or("count")?
                    > before["visits"].as_u64().ok_or("prior count")?
            );
            assert!(ford.terminal_landed);
            assert!(ford.advance(u64::from(case.physics.native_ms)).is_err());
        }
        Ok(())
    }
}
