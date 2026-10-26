//! Real standing orders, accepted shots and sensor expiry. Each public fact is
//! credited only after the answer that read it actually lands. Custody changes
//! isolate fixture phases without replacing source memory or its sealed program.
use super::{
    identity,
    manifest::FrozenPolicy,
    plan::{Case, Physics},
    probes, runtime,
    subject::PreparedSubject,
    trials::{self, Landings, Slice},
};
use crate::{
    arena::{
        Arena, Channel, ContactId, Controller, Power,
        author_trace::Outcome,
        core::CoreClass,
        doctrine::MachineState,
        intents::{Intent, IntentKind, IntentOutcome, IntentSource, apply_intent},
        mind::{CollectedThink, MindView, run_batch_inline},
        motor::MotorCounters,
        perception::{FrameNotice, MindObjective},
        weapons::FiredBy,
    },
    deliberation::{
        SquadId,
        order::{CharacterRef, Constraints, Objective, Order},
    },
};
use mush_core::{DefinitionDigest, ObjId};
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, Coverage, ProbeName,
    RatingClass, RatingError,
};
use std::collections::BTreeMap;

const OWN: ObjId = ObjId(1);
const CONTACT: ContactId = ContactId(1);
const MOVE: u16 = 1;
const HOLD: u16 = 2;
const WITHDRAW: u16 = 4;
const LOCKED: u16 = 8;
const LOST: u16 = 16;
const HURT: u16 = 32;
const MISS: u16 = 64;
const DRY: u16 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
enum Phase {
    Move,
    Hold,
    Withdraw,
    WaitingLoss,
    Lost,
}
const OBSERVED_PHASES: [Phase; 4] = [Phase::Move, Phase::Hold, Phase::Withdraw, Phase::Lost];

pub(super) struct Loss {
    arena: Arena,
    case: Case,
    installed: runtime::Installed,
    phase: Phase,
    wall: i64,
    armed: bool,
    observations: Landings,
    pending: BTreeMap<u64, (Phase, u16)>,
    applied: u16,
    // One actual landed answer per required phase, separate from the number
    // of facts in its picture or the total number of metered invocations.
    phase_landings: BTreeMap<Phase, u64>,
    pictures: Vec<DefinitionDigest>,
    transitions: Vec<(i64, u8, DefinitionDigest)>,
    waiting_memory: Option<Vec<u8>>,
    waiting_seal: [u8; 32],
    waiting_profile: Option<crate::arena::runtime_card::ProfileKey>,
    initial: Option<CollectedThink>,
    done: bool,
}
impl Loss {
    pub(super) fn physics(
        policy: &FrozenPolicy,
        class: CoreClass,
        armed: bool,
    ) -> Result<Physics, RatingError> {
        let (arena, _) = probes::loss_opening(policy, class, armed)?;
        Ok(Physics {
            units: u16::try_from(arena.units.len()).map_err(|_| RatingError::Bound)?,
            native_ms: u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?,
        })
    }
    /// The captured TTL plus one whole pass covers its stagger. The native
    /// tail is already separately admitted by the schedule. No config clamp.
    pub(super) fn expiry_budget(policy: &FrozenPolicy) -> Result<u64, RatingError> {
        u64::from(policy.cfg.sensors.pass_ms)
            .checked_mul(u64::from(policy.cfg.sensors.lost_after_passes))
            .and_then(|ttl| ttl.checked_add(u64::from(policy.cfg.sensors.pass_ms)))
            .filter(|ms| *ms <= i64::MAX as u64)
            .ok_or(RatingError::Bound)
    }
    pub(super) fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        let CaseKind::Probe { pair, name } = case.key.kind() else {
            return Err(RatingError::Identity);
        };
        let armed = match name {
            ProbeName::OrderLossArmed => true,
            ProbeName::OrderLossUnarmed => false,
            _ => return Err(RatingError::Identity),
        };
        let class = match case.key.class() {
            RatingClass::I => CoreClass::I,
            RatingClass::II => CoreClass::II,
            RatingClass::III => CoreClass::III,
        };
        if case.maximum_thinks != 64 || case.physics != Self::physics(policy, class, armed)? {
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
        let (mut arena, rom) = probes::loss_opening(policy, class, armed)?;
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            pair.profile,
            class,
            &BTreeMap::from([(OWN, rom)]),
            case.maximum_thinks,
        )?;
        let deadline = case
            .maximum_ms
            .checked_add(u64::from(case.physics.native_ms))
            .ok_or(RatingError::Bound)?;
        if case.maximum_ms < Self::expiry_budget(policy)? {
            return Err(RatingError::Bound);
        }
        arena.match_cfg.max_duration_ms =
            u32::try_from(deadline).map_err(|_| RatingError::Bound)?;
        arena
            .due
            .retain(|kind| !matches!(kind, crate::arena::state::DueKind::MatchDeadline));
        arena.due.schedule(
            i64::from(arena.match_cfg.max_duration_ms),
            crate::arena::state::DueKind::MatchDeadline,
        );
        arena.traces.set_enabled(true);
        let mut value = Self {
            arena,
            case: case.clone(),
            installed,
            phase: Phase::Move,
            wall: 0,
            armed,
            observations: Landings::new(&[OWN]),
            pending: BTreeMap::new(),
            applied: 0,
            phase_landings: BTreeMap::new(),
            pictures: Vec::new(),
            transitions: Vec::new(),
            waiting_memory: None,
            waiting_seal: [0; 32],
            waiting_profile: None,
            initial: None,
            done: false,
        };
        value
            .arena
            .resolve_lock(OWN, CONTACT, 0)
            .map_err(|_| RatingError::Incomplete)?;
        value.order(Objective::Advance { to: (4, 8) })?;
        // Like the six single pictures, the first bounded invocation is taken
        // at the exact opening. All later calls use the real class due queue.
        let sequence = value
            .arena
            .minds
            .get(&OWN)
            .ok_or(RatingError::Incomplete)?
            .think_seq;
        value
            .arena
            .collect_think(OWN, sequence, 0, &mut MotorCounters::default());
        value.initial = Some(value.arena.collected.pop().ok_or(RatingError::Incomplete)?);
        if !value.arena.collected.is_empty() {
            return Err(RatingError::Incomplete);
        }
        Ok(value)
    }
    pub(super) fn binding(&self) -> &CaseBinding {
        &self.installed.binding
    }
    fn order(&mut self, objective: Objective) -> Result<(), RatingError> {
        let now = self.arena.now_ms;
        let order = Order {
            squad: SquadId::parse("red-1").ok_or(RatingError::Identity)?,
            objective,
            target: None,
            horizon_ms: 600_000,
            constraints: Constraints::default(),
            ttl_ms: 600_000,
            issued_ms: now,
            issuer: Some(OWN),
            by: CharacterRef::Character { id: OWN },
            spot: None,
        };
        order
            .validate(Some((64, 64)))
            .map_err(|_| RatingError::Invalid)?;
        let intent = Intent {
            unit: OWN,
            channel: Channel::Helm,
            epoch: self.arena.epochs.current(OWN, Channel::Helm),
            kind: IntentKind::Order(Box::new(order)),
            seq: 0,
        };
        if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
            != IntentOutcome::AppliedContinuous
        {
            return Err(RatingError::Incomplete);
        }
        Ok(())
    }
    fn facts(&self, view: &MindView) -> u16 {
        let mut facts = 0;
        match view.order.as_ref().map(|o| o.objective) {
            Some(MindObjective::Advance { .. }) => facts |= MOVE,
            Some(MindObjective::Hold) => facts |= HOLD,
            Some(MindObjective::Withdraw) => facts |= WITHDRAW,
            _ => {}
        }
        if view.lock == Some(CONTACT) {
            facts |= LOCKED;
        }
        if view.lock.is_none()
            && !view.contacts.iter().any(|c| c.id == CONTACT)
            && self.arena.contacts.get(OWN, CONTACT).is_none()
            && view.t_ms >= self.arena.contact_ttl_ms()
        {
            facts |= LOST;
        }
        if view
            .sections
            .iter()
            .any(|s| s.internals.is_some_and(|(now, full)| now > 0 && now < full))
        {
            facts |= HURT;
        }
        if self.armed && !view.weapons.is_empty() && view.weapons.iter().all(|w| w.rounds == 0) {
            facts |= DRY;
        }
        if view.notices.iter().any(|n|matches!(n,FrameNotice::Miss {reason,launch_cell:Some((5,5)),..} if reason=="blocked" || reason=="cover")) {facts|=MISS;}
        facts
    }
    fn observe(&mut self, think: &CollectedThink) -> Result<(), RatingError> {
        if think.unit != OWN
            || think.dropped
            || self.pictures.len() >= 64
            || think.view.weapons.is_empty() == self.armed
            || think.view.power != Power::Online
        {
            return Err(RatingError::Incomplete);
        }
        let facts = self.facts(&think.view);
        if self
            .pending
            .insert(think.think_seq, (self.phase, facts))
            .is_some()
        {
            return Err(RatingError::Identity);
        }
        self.pictures.push(identity(
            "tacsim.registry.order_loss.picture.v1",
            &(
                think.think_seq,
                think.view.t_ms,
                facts,
                &think.view.order,
                think.view.lock,
                &think.view.notices,
            ),
            16 * 1024,
        )?);
        Ok(())
    }
    fn custody(&mut self, controller: Controller) -> Result<(), RatingError> {
        let memory = self
            .arena
            .minds
            .get(&OWN)
            .ok_or(RatingError::Incomplete)?
            .memory
            .clone();
        let seal = self
            .arena
            .unit(OWN)
            .ok_or(RatingError::Incomplete)?
            .seal
            .digest;
        let profile = self
            .arena
            .profiles
            .get(&OWN)
            .ok_or(RatingError::Incomplete)?
            .card
            .key();
        self.arena
            .set_controller(OWN, controller, MachineState::Patrol);
        if self
            .arena
            .minds
            .get(&OWN)
            .is_none_or(|m| m.memory != memory || m.pending.is_some() || m.dropped.is_some())
            || self
                .arena
                .unit(OWN)
                .is_none_or(|u| u.seal.digest != seal || u.controller != controller)
            || self
                .arena
                .profiles
                .get(&OWN)
                .is_none_or(|r| r.card.key() != profile)
        {
            return Err(RatingError::Invalid);
        }
        if self.transitions.len() >= 8 {
            return Err(RatingError::Bound);
        }
        // Memory bytes never leave the worker. The witness is a bounded hash.
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"tacsim.registry.order_loss.memory.v1\0");
        hash.update(&memory);
        self.transitions.push((
            self.arena.now_ms,
            u8::from(controller == Controller::Mind),
            DefinitionDigest::from_bytes(hash.finalize().into()),
        ));
        Ok(())
    }
    fn next_order(&mut self, phase: Phase, objective: Objective) -> Result<(), RatingError> {
        self.custody(Controller::Pilot)?;
        self.custody(Controller::Mind)?;
        self.order(objective)?;
        self.phase = phase;
        Ok(())
    }
    fn begin_loss_wait(&mut self) -> Result<(), RatingError> {
        self.custody(Controller::Pilot)?;
        // Restore an actual held lock if a legal candidate chose to unlock it.
        // Normal sensor expiry, never a synthetic Lost event, removes it.
        if self.arena.contacts.get(OWN, CONTACT).is_some() {
            let now = self.arena.now_ms;
            self.arena
                .resolve_lock(OWN, CONTACT, now)
                .map_err(|_| RatingError::Incomplete)?;
        }
        let intent = Intent {
            unit: OWN,
            channel: Channel::Helm,
            epoch: self.arena.epochs.current(OWN, Channel::Helm),
            kind: IntentKind::Speed { cms: 0 },
            seq: 0,
        };
        if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
            != IntentOutcome::AppliedContinuous
        {
            return Err(RatingError::Incomplete);
        }
        self.waiting_memory = Some(
            self.arena
                .minds
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .memory
                .clone(),
        );
        self.waiting_seal = self
            .arena
            .unit(OWN)
            .ok_or(RatingError::Incomplete)?
            .seal
            .digest;
        self.waiting_profile = Some(
            self.arena
                .profiles
                .get(&OWN)
                .ok_or(RatingError::Incomplete)?
                .card
                .key(),
        );
        self.phase = Phase::WaitingLoss;
        Ok(())
    }
    fn resume_after_loss(&mut self) -> Result<(), RatingError> {
        let mind = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        if self.waiting_memory.as_deref() != Some(mind.memory.as_slice())
            || self
                .arena
                .unit(OWN)
                .is_none_or(|u| u.seal.digest != self.waiting_seal)
            || self
                .arena
                .profiles
                .get(&OWN)
                .is_none_or(|r| Some(r.card.key()) != self.waiting_profile)
        {
            return Err(RatingError::Invalid);
        }
        self.custody(Controller::Mind)?;
        self.phase = Phase::Lost;
        Ok(())
    }
    pub(super) fn advance(&mut self, allowance_ms: u64) -> Result<Slice, RatingError> {
        let native = u64::from(self.case.physics.native_ms);
        if self.done || allowance_ms < native {
            return Err(RatingError::Invalid);
        }
        if let Some(think) = self.initial.take() {
            self.observe(&think)?;
            let result = think.think().result;
            let settled = self.arena.settle_collected(&think, result);
            if settled.overflows != 0 || settled.dropped != 0 {
                return Err(RatingError::Invalid);
            }
            if self.armed {
                let weapons = self
                    .arena
                    .unit(OWN)
                    .ok_or(RatingError::Incomplete)?
                    .weapons
                    .iter()
                    .map(|w| w.id.clone())
                    .collect::<Vec<_>>();
                for weapon in weapons {
                    self.arena
                        .resolve_fire_by(OWN, &weapon, 0, FiredBy::Seat)
                        .map_err(|_| RatingError::Incomplete)?;
                }
            }
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
            self.observe(think)?;
        }
        let settled = self.arena.deliver_answers(run_batch_inline(&out.thinks));
        if settled.overflows != 0 || settled.dropped != 0 {
            return Err(RatingError::Invalid);
        }
        for row in self.arena.traces.recent(OWN, None) {
            // Worker completion sets settled before the scheduled landing.
            // Preserve the captured facts until that landing has an outcome.
            if !row.settled || matches!(row.outcome, Outcome::Pending) {
                continue;
            }
            let Some((phase, facts)) = self.pending.remove(&row.think_seq) else {
                continue;
            };
            if matches!(row.outcome, Outcome::Applied) {
                self.applied |= facts;
                let required_fact = match phase {
                    Phase::Move => MOVE,
                    Phase::Hold => HOLD,
                    Phase::Withdraw => WITHDRAW,
                    Phase::Lost => LOST,
                    Phase::WaitingLoss => 0,
                };
                if required_fact != 0 && facts & required_fact != 0 {
                    self.phase_landings.entry(phase).or_insert(row.think_seq);
                }
            }
        }
        self.observations.collect(&self.arena.traces)?;
        let measured = self.installed.meter.measurement()?;
        let budget = measured.summary.only_budget_faults();
        if (!measured.summary.clean() && !budget)
            || self
                .arena
                .minds
                .get(&OWN)
                .is_none_or(|m| m.dropped.is_some() || (!budget && m.strikes != 0))
        {
            return Err(RatingError::Invalid);
        }
        let first = MOVE | LOCKED | HURT | if self.armed { MISS | DRY } else { 0 };
        if !budget {
            match self.phase {
                Phase::Move if self.applied & first == first => {
                    self.next_order(Phase::Hold, Objective::Hold)?
                }
                Phase::Hold if self.applied & HOLD != 0 => {
                    self.next_order(Phase::Withdraw, Objective::Withdraw)?
                }
                Phase::Withdraw if self.applied & WITHDRAW != 0 => self.begin_loss_wait()?,
                Phase::WaitingLoss
                    if self.arena.contacts.get(OWN, CONTACT).is_none()
                        && self.arena.unit(OWN).is_some_and(|u| u.lock.is_none()) =>
                {
                    self.resume_after_loss()?
                }
                _ => {}
            }
        }
        self.observations.collect(&self.arena.traces)?;
        trials::drain(&mut self.arena, &out.ledger_chunks)?;
        let required = first | HOLD | WITHDRAW | LOST;
        let complete = self.phase == Phase::Lost
            && self.applied & required == required
            && OBSERVED_PHASES
                .iter()
                .all(|phase| self.phase_landings.contains_key(phase));
        if !complete && !budget {
            if measured.summary.thinks() >= 64
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
            if self.armed {
                Applicability::Armed
            } else {
                Applicability::Unarmed
            },
            u32::try_from(OBSERVED_PHASES.len()).map_err(|_| RatingError::Bound)?,
            u32::try_from(self.phase_landings.len()).map_err(|_| RatingError::Bound)?,
            identity(
                "tacsim.registry.order_loss.coverage.v1",
                &(
                    &self.pictures,
                    &self.transitions,
                    &self.phase_landings,
                    required,
                    self.applied,
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
    fn real_orders_shot_dry_and_expiry_are_observed_without_requiring_a_policy_to_fight()
    -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { stock::rules(); speed(0); if this.contains(\"visits\") { this.visits+=1; } else { this.visits=1; } }";
        let subject = PreparedSubject::prepare(
            &super::super::tests::revision(source)?,
            source,
            &policy.cfg.arena,
        )?;
        for armed in [false, true] {
            let name = if armed {
                ProbeName::OrderLossArmed
            } else {
                ProbeName::OrderLossUnarmed
            };
            let case = Case {
                key: mush_roll::commerce::rating::CaseKey::new(
                    RatingClass::I,
                    CaseKind::Probe { pair: 0, name },
                )?,
                physics: Loss::physics(&policy, CoreClass::I, armed)?,
                maximum_ms: 70_000 + Loss::expiry_budget(&policy)?,
                maximum_thinks: 64,
            };
            let mut loss = Loss::open(&policy, &subject, &case)?;
            let first = loss.initial.as_ref().ok_or("real opening think")?;
            check_eq!(
                loss.facts(&first.view) & (MOVE | LOCKED | HURT),
                MOVE | LOCKED | HURT,
                "one captured picture supplies three distinct facts"
            );
            let mut finished = None;
            let mut billed = 0;
            for _ in 0..400 {
                let slice = loss.advance(u64::from(case.physics.native_ms)).map_err(|error| {
                    format!(
                        "order/loss {error:?}: armed={armed}, phase={:?}, wall={}, now={}, facts={:#x}, phase_landings={:?}, measured={:?}, traces={:?}",
                        loss.phase, loss.wall, loss.arena.now_ms, loss.applied, loss.phase_landings,
                        loss.installed.meter.measurement(),
                        loss.arena.traces.recent(OWN, None).into_iter().map(|row| (
                            row.think_seq, row.settled, row.outcome, row.fault, row.rom_drop
                        )).collect::<Vec<_>>()
                    )
                })?;
                billed += slice.simulated_ms;
                if slice.result.is_some() {
                    finished = slice.result;
                    break;
                }
            }
            let result = finished.ok_or("all real loss witnesses")?;
            check!(matches!(result.outcome(), CaseOutcome::Conformant {}));
            check!(result.coverage().complete());
            check_eq!(result.coverage().required_thinks(), 4);
            check_eq!(result.coverage().evaluated_thinks(), 4);
            check!(result.meter().thinks() >= 4);
            check_eq!(loss.phase_landings.len(), OBSERVED_PHASES.len());
            let mut sequences = loss.phase_landings.values().copied().collect::<Vec<_>>();
            sequences.sort_unstable();
            sequences.dedup();
            check_eq!(
                sequences.len(),
                OBSERVED_PHASES.len(),
                "four distinct actual answers"
            );
            // The author-facing default returns only three rows; this final
            // check needs every retained phase witness (at most 64 thinks).
            let traces = loss
                .arena
                .traces
                .recent(OWN, Some(crate::arena::author_trace::THINKS));
            for phase in OBSERVED_PHASES {
                let sequence = loss.phase_landings.get(&phase).ok_or("landed phase")?;
                check!(traces.iter().any(|row| row.think_seq == *sequence
                    && row.settled
                    && matches!(row.outcome, Outcome::Applied)));
            }
            if !armed {
                check!(
                    result.meter().thinks() < 6,
                    "the six required facts must not be mislabeled as six metered thinks"
                );
            }
            check_eq!(
                loss.applied & (MOVE | HOLD | WITHDRAW | LOCKED | LOST | HURT),
                MOVE | HOLD | WITHDRAW | LOCKED | LOST | HURT
            );
            check_eq!(
                loss.applied & (MISS | DRY),
                if armed { MISS | DRY } else { 0 }
            );
            check_eq!(billed, u64::try_from(loss.arena.now_ms)?);
            check_eq!(
                loss.transitions.len(),
                6,
                "three real possess/release pairs"
            );
            for pair in loss.transitions.as_chunks::<2>().0 {
                let [(_, pilot, before), (_, mind, after)] = pair;
                check_eq!((*pilot, *mind), (0, 1));
                check_eq!(before, after, "custody preserved actual memory");
            }
            check!(loss.arena.contacts.get(OWN, CONTACT).is_none());
            check!(loss.arena.now_ms >= loss.arena.contact_ttl_ms());
            let memory: serde_json::Value =
                serde_json::from_slice(&loss.arena.minds.get(&OWN).ok_or("mind")?.memory)?;
            check!(
                memory
                    .get("visits")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|v| v >= 4)
            );
            check!(loss.advance(u64::from(case.physics.native_ms)).is_err());
        }
        Ok(())
    }
    #[test]
    fn captured_expiry_is_reserved_without_shortening_the_sensor_policy() -> TestResult {
        let mut policy = super::super::tests::frozen_policy()?;
        policy.cfg.sensors.pass_ms = 10_000;
        policy.cfg.sensors.lost_after_passes = 20;
        check_eq!(Loss::expiry_budget(&policy)?, 210_000);
        let (arena, _) = probes::loss_opening(&policy, CoreClass::I, false)?;
        check_eq!(arena.contact_ttl_ms(), 200_000);
        policy.cfg.sensors.pass_ms = u32::MAX;
        policy.cfg.sensors.lost_after_passes = u32::MAX;
        check!(
            Loss::expiry_budget(&policy).is_err(),
            "unrepresentable real time refuses admission"
        );
        Ok(())
    }
}
