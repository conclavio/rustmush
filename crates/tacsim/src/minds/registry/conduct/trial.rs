//! One real private native Arena. The submitted source owns only its selected
//! role. The other convoy hull uses the frozen native ROM and ordinary orders;
//! it is scenario machinery, never another submitted qualification.
use super::super::{identity, meter::MeteredRunner, runtime, subject::PreparedSubject, trials};
use super::{
    Case, Role,
    cargo::{Cargo, CargoReport},
    manifest::Policy,
    observation::{Observations, Observer},
};
use crate::{
    arena::{
        Arena, Channel, Controller, CustodyOut, MatchMode, PartyId,
        core::CoreClass,
        intents::{Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, apply_intent},
        mind::{SealEngine, run_batch_inline},
        profile_seal::SealedCards,
        state::{ArenaDefinition, ArenaTerrain, DueKind, EndReason},
    },
    arena_task::MatchId,
    deliberation::order::{CharacterRef, Constraints, Objective, Order},
    match_::{SealIdentity, SealTerms},
};
use mush_core::{CanonicalId, DefinitionDigest, ObjId};
use mush_roll::commerce::rating::{CaseBinding, MeterSummary, RatingError};
use mush_sim::data::Pose;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

const ESCORT: ObjId = ObjId(100);
const CARRIER: ObjId = ObjId(101);
const THREAT: ObjId = ObjId(200);
const ANCHOR: ObjId = ObjId(201);
const DESTINATION: (u16, u16) = (129, 97);

/// Private result data. Its decoder is used only inside complete owner replay;
/// it cannot construct CompletedRun or an opaque acknowledged qualification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResultRow {
    role: Role,
    case: Case,
    seed: u64,
    policy: DefinitionDigest,
    binding: CaseBinding,
    opening: u64,
    terminal: u64,
    ground: DefinitionDigest,
    ledger: DefinitionDigest,
    elapsed_ms: u64,
    steps: u64,
    observations: Observations,
    meter: MeterSummary,
    meter_witness: DefinitionDigest,
    cargo: CargoReport,
    survived: bool,
    moved_cm: u64,
    shots: u64,
    reached_entry: bool,
    passed: bool,
}
impl ResultRow {
    pub(super) fn passed(&self) -> bool {
        self.passed
    }
    pub(super) fn binding(&self) -> &CaseBinding {
        &self.binding
    }
    pub(super) fn validate(&self, opening: &CaseOpening) -> Result<u64, RatingError> {
        let step_time = self
            .steps
            .checked_mul(u64::from(opening.native_ms))
            .ok_or(RatingError::Bound)?;
        if self.role != opening.role
            || self.case != opening.case
            || self.seed != opening.seed
            || self.policy != opening.policy
            || self.binding != opening.binding
            || self.opening != opening.opening
            || self.ground != opening.ground
            || self.elapsed_ms < u64::from(opening.maximum_ms)
            || self.elapsed_ms > u64::from(opening.maximum_ms) + u64::from(opening.native_ms)
            || self.steps == 0
            || self.steps > u64::from(opening.maximum_ms).div_ceil(u64::from(opening.native_ms)) + 2
            || step_time < self.elapsed_ms
            || step_time - self.elapsed_ms > 2 * u64::from(opening.native_ms)
            || !self.meter.clean()
            || self.meter.thinks() == 0
            || self.observations.thinks == 0
            || self.passed != self.qualifies()
        {
            return Err(RatingError::Identity);
        }
        self.cargo.follows(&opening.cargo, self.elapsed_ms)?;
        self.elapsed_ms
            .checked_mul(u64::from(opening.units))
            .ok_or(RatingError::Bound)
    }
    /// One predicate for both the live measured result and retained replay.
    fn qualifies(&self) -> bool {
        let o = &self.observations;
        let safe = self.survived
            && o.forbidden_fire == 0
            && o.forbidden_move == 0
            && o.retained_move == 0
            && (self.role != Role::Transit || self.shots == 0);
        let behavior = match self.case {
            Case::Convoy => {
                self.cargo.delivered()
                    && self.moved_cm >= 10_000
                    && match self.role {
                        Role::Escort => o.friendly_charge >= 3,
                        Role::Transit => o.advance >= 3 && o.drove_without_threat >= 3,
                    }
            }
            Case::EscortThreat => {
                self.cargo.delivered() && o.friendly_charge >= 3 && o.threat >= 3 && self.shots > 0
            }
            Case::MissingCharge | Case::ForeignCharge => {
                self.shots == 0
                    && self.moved_cm <= 1000
                    && match self.role {
                        Role::Escort => o.missing_charge >= 3,
                        Role::Transit => o.advance == 0,
                    }
            }
            Case::HoldFire => self.shots == 0 && o.threat >= 3,
            Case::ThreatStopResume => {
                self.cargo.delivered() && o.stopped_for_threat >= 3 && o.drove_without_threat >= 3
            }
            Case::BlockedRoute => {
                !self.cargo.delivered() && o.no_route >= 3 && o.retry_attempts >= 2
            }
            Case::Withdraw => o.withdraw >= 3 && self.reached_entry,
        };
        safe && behavior
    }
    /// Bounded public counters only: no source, witness, tape or sealed identity.
    #[cfg(test)]
    pub(super) fn failure_summary(&self) -> String {
        let o = &self.observations;
        format!(
            "role={:?} case={:?} seed={} survived={} delivered={} moved_cm={} shots={} entry={} thinks={} advance={} support={} withdraw={} friendly={} missing={} threat={} stopped={} drove={} no_route={} retries={} forbidden_fire={} forbidden_move={} retained_move={}",
            self.role,
            self.case,
            self.seed,
            self.survived,
            self.cargo.delivered(),
            self.moved_cm,
            self.shots,
            self.reached_entry,
            o.thinks,
            o.advance,
            o.support,
            o.withdraw,
            o.friendly_charge,
            o.missing_charge,
            o.threat,
            o.stopped_for_threat,
            o.drove_without_threat,
            o.no_route,
            o.retry_attempts,
            o.forbidden_fire,
            o.forbidden_move,
            o.retained_move
        )
    }
    #[cfg(test)]
    pub(super) fn observations(&self) -> &Observations {
        &self.observations
    }
}

/// Actual pre-step Arena identity retained in the admitted start artifact.
/// Decoding it is data validation only; only Trial::opening_record captures it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaseOpening {
    role: Role,
    case: Case,
    seed: u64,
    policy: DefinitionDigest,
    binding: CaseBinding,
    opening: u64,
    ground: DefinitionDigest,
    units: u16,
    native_ms: u32,
    maximum_ms: u32,
    cargo: CargoReport,
}
impl CaseOpening {
    pub(super) fn validate(
        &self,
        policy: DefinitionDigest,
        retained: &super::manifest::RetainedPolicy,
        subject: &mush_roll::commerce::rating::RatingSubject,
        role: Role,
        case: Case,
        seed: u64,
    ) -> Result<(), RatingError> {
        if self.role != role
            || self.case != case
            || self.seed != seed
            || self.policy != policy
            || self.binding.source() != subject.revision().source()
            || self.units != if case == Case::EscortThreat { 4 } else { 3 }
            || !(50..=1000).contains(&self.native_ms)
            || self.maximum_ms != 120_000
            || self.cargo != retained.cargo(policy, seed)?
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    pub(super) fn rate(&self, index: u8) -> crate::minds::bulk::conduct_budget::ConductCase {
        crate::minds::bulk::conduct_budget::ConductCase {
            index,
            units: self.units,
            native_ms: self.native_ms,
            maximum_ms: u64::from(self.maximum_ms),
        }
    }
}
pub(super) struct Slice {
    pub(super) simulated_ms: u64,
    pub(super) result: Option<ResultRow>,
}
pub(super) struct Trial {
    arena: Arena,
    cargo: Cargo,
    observer: Option<Observer>,
    meter: Arc<MeteredRunner>,
    binding: CaseBinding,
    role: Role,
    case: Case,
    seed: u64,
    policy: DefinitionDigest,
    ground: DefinitionDigest,
    candidate: ObjId,
    opening: u64,
    opening_position: (i64, i64),
    initial_ammo: u64,
    native_ms: u32,
    maximum_ms: u32,
    wall: i64,
    steps: u64,
    order_changed: bool,
    support_reissued: bool,
    threat_departed: bool,
    finished: bool,
}
impl Trial {
    pub(super) fn opening_record(&self) -> Result<CaseOpening, RatingError> {
        if self.steps != 0 || self.wall != 0 || self.finished || self.arena.now_ms != 0 {
            return Err(RatingError::Identity);
        }
        Ok(CaseOpening {
            role: self.role,
            case: self.case,
            seed: self.seed,
            policy: self.policy,
            binding: self.binding.clone(),
            opening: self.opening,
            ground: self.ground,
            units: u16::try_from(self.units()).map_err(|_| RatingError::Bound)?,
            native_ms: self.native_ms,
            maximum_ms: self.maximum_ms,
            cargo: self.cargo.report()?,
        })
    }
    pub(super) fn open(
        policy: &Policy,
        subject: &PreparedSubject,
        profile: &str,
        role: Role,
        case: Case,
        seed: u64,
    ) -> Result<Self, RatingError> {
        if !policy.seeds().contains(&seed)
            || profile.is_empty()
            || profile.len() > 64
            || !Case::for_role(role).contains(&case)
        {
            return Err(RatingError::Identity);
        }
        // The exact named full-card expansion is retained; no profile is
        // rotated, truncated, rewritten, or inferred from a source comment.
        let variant = subject
            .variants()
            .iter()
            .find(|variant| {
                !variant.cycle
                    && variant
                        .definition
                        .profiles
                        .get(usize::from(variant.initial))
                        .is_some_and(|row| row.name == profile)
            })
            .ok_or(RatingError::Identity)?;
        let candidate = match role {
            Role::Escort => ESCORT,
            Role::Transit => CARRIER,
        };
        let (mut arena, rom) = opening(policy, role, case, seed)?;
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            variant.initial,
            CoreClass::II,
            &BTreeMap::from([(candidate, rom)]),
            4096,
        )?;
        arena.traces.set_enabled(true);
        installed.meter.enable_physical(&arena)?;
        let own = arena.unit(candidate).ok_or(RatingError::Incomplete)?;
        let opening_position = (own.pose.x_cm, own.pose.y_cm);
        let initial_ammo = ammo(own)?;
        let native_ms = u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?;
        if native_ms == 0 || native_ms > 1000 {
            return Err(RatingError::Bound);
        }
        let observer = Observer::new(own, role, native_ms)?;
        let ground = identity(
            "tacsim.conduct.case.ground.v1",
            &crate::arena::replay_ground::ReplayGround::capture(&arena.definition)
                .map_err(|_| RatingError::Invalid)?,
            16 * 1024 * 1024,
        )?;
        let cargo = Cargo::load(
            policy.cargo(),
            policy.compartment(),
            &policy.goods,
            CARRIER,
            DESTINATION,
            policy.digest(),
            seed,
        )?;
        let mut value = Self {
            arena,
            cargo,
            observer: Some(observer),
            meter: installed.meter,
            binding: installed.binding,
            role,
            case,
            seed,
            policy: policy.digest(),
            ground,
            candidate,
            opening: 0,
            opening_position,
            initial_ammo,
            native_ms,
            maximum_ms: policy.maximum_ms(),
            wall: 0,
            steps: 0,
            order_changed: false,
            support_reissued: false,
            threat_departed: false,
            finished: false,
        };
        value.order(
            CARRIER,
            if case == Case::MissingCharge && role == Role::Transit {
                Objective::Hold
            } else {
                Objective::Advance { to: DESTINATION }
            },
            true,
        )?;
        value.order(
            ESCORT,
            Objective::Support {
                of: if matches!(case, Case::MissingCharge | Case::ForeignCharge) {
                    THREAT
                } else {
                    CARRIER
                },
            },
            role == Role::Transit || case == Case::HoldFire,
        )?;
        value.opening = crate::arena::state_hash(&value.arena);
        Ok(value)
    }
    pub(super) fn native_ms(&self) -> u32 {
        self.native_ms
    }
    pub(super) fn units(&self) -> usize {
        self.arena.units.len()
    }
    pub(super) fn maximum_steps(&self) -> u64 {
        u64::from(self.maximum_ms).div_ceil(u64::from(self.native_ms)) + 2
    }
    /// Test-only actual commander transition after an actual running route.
    /// It does not change the motor, pose, source answer or observed view.
    #[cfg(test)]
    pub(super) fn hold_after_real_motion(&mut self) -> Result<(), RatingError> {
        for _ in 0..self.maximum_steps() {
            if self.support_reissued
                && self
                    .arena
                    .unit(self.candidate)
                    .is_some_and(|u| u.speed_cms != 0)
                && self
                    .arena
                    .motors
                    .row(self.candidate)
                    .is_some_and(|m| m.goal.is_some())
            {
                return self.order(self.candidate, Objective::Hold, true);
            }
            if self.advance(u64::from(self.native_ms))?.result.is_some() {
                break;
            }
        }
        Err(RatingError::Incomplete)
    }
    fn order(
        &mut self,
        unit: ObjId,
        objective: Objective,
        hold_fire: bool,
    ) -> Result<(), RatingError> {
        let squad = self.arena.squad_of(unit).ok_or(RatingError::Incomplete)?.id;
        let order = Order {
            squad,
            objective,
            target: None,
            horizon_ms: self.maximum_ms,
            constraints: Constraints {
                hold_fire,
                max_range_band: None,
            },
            ttl_ms: self.maximum_ms,
            issued_ms: self.arena.now_ms,
            issuer: Some(unit),
            by: CharacterRef::Persona { squad },
            spot: None,
        };
        order
            .validate(Some((
                self.arena.definition.terrain.width(),
                self.arena.definition.terrain.height(),
            )))
            .map_err(|_| RatingError::Invalid)?;
        let intent = Intent {
            unit,
            channel: Channel::Helm,
            epoch: self.arena.epochs.current(unit, Channel::Helm),
            kind: IntentKind::Order(Box::new(order)),
            seq: self.steps,
        };
        if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
            != IntentOutcome::AppliedContinuous
        {
            return Err(RatingError::Incomplete);
        }
        Ok(())
    }
    pub(super) fn advance(&mut self, allowance_ms: u64) -> Result<Slice, RatingError> {
        if self.finished || allowance_ms < u64::from(self.native_ms) {
            return Err(RatingError::Invalid);
        }
        let result = self.advance_native();
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn advance_native(&mut self) -> Result<Slice, RatingError> {
        if !self.support_reissued && self.arena.now_ms >= 10_000 {
            // Support is re-keyed only when a real order lands. The opening
            // order deliberately has no earned plot. A fixed later commander
            // order uses whatever the sensors actually learned by this time.
            if self.case != Case::MissingCharge {
                self.order(
                    ESCORT,
                    Objective::Support {
                        of: if self.case == Case::ForeignCharge {
                            THREAT
                        } else {
                            CARRIER
                        },
                    },
                    self.role == Role::Transit || self.case == Case::HoldFire,
                )?;
            }
            self.support_reissued = true;
        }
        if self.case == Case::Withdraw && !self.order_changed && self.arena.now_ms >= 20_000 {
            self.order(self.candidate, Objective::Withdraw, true)?;
            self.order_changed = true;
        }
        if self.case == Case::ThreatStopResume
            && !self.threat_departed
            && self.arena.now_ms >= 20_000
        {
            // The threat leaves by the same native seat motor command as a
            // pilot. No pose, contact, visibility, health or memory is edited.
            let intent = Intent {
                unit: THREAT,
                channel: Channel::Helm,
                epoch: self.arena.epochs.current(THREAT, Channel::Helm),
                kind: IntentKind::Goal(MotorGoal::Cell { x: 224, y: 97 }),
                seq: self.steps,
            };
            if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
                != IntentOutcome::AppliedContinuous
            {
                return Err(RatingError::Incomplete);
            }
            self.threat_departed = true;
        }
        let before = self.arena.now_ms;
        self.wall = self
            .wall
            .checked_add(i64::from(self.native_ms))
            .ok_or(RatingError::Bound)?;
        // Exactly the production step_settled barrier, expanded only to read
        // its already-collected immutable views and actual unchanged answers.
        let mut out = self.arena.step(self.wall, &[], &[]);
        let thinks = std::mem::take(&mut out.thinks);
        let answers = run_batch_inline(&thinks);
        if thinks.len() != answers.len() {
            return Err(RatingError::Incomplete);
        }
        for (think, answer) in thinks.iter().zip(&answers) {
            if think.unit == self.candidate {
                self.observer
                    .as_mut()
                    .ok_or(RatingError::Incomplete)?
                    .record(think, answer)?;
            }
        }
        let settled = self.arena.deliver_answers(answers);
        if settled.faults != 0 || settled.overflows != 0 || settled.dropped != 0 {
            return Err(RatingError::Incomplete);
        }
        trials::drain(&mut self.arena, &out.ledger_chunks)?;
        self.meter.reconcile(&self.arena)?;
        self.cargo.observe(&self.arena)?;
        let simulated_ms =
            u64::try_from(self.arena.now_ms - before).map_err(|_| RatingError::Invalid)?;
        if simulated_ms > u64::from(self.native_ms) {
            return Err(RatingError::Bound);
        }
        self.steps = self.steps.checked_add(1).ok_or(RatingError::Bound)?;
        if self.steps > self.maximum_steps() {
            return Err(RatingError::Bound);
        }
        let measured = self.meter.measurement()?;
        if !measured.summary.clean() {
            return Err(RatingError::Incomplete);
        }
        let Some(ended) = self.arena.ended else {
            return Ok(Slice {
                simulated_ms,
                result: None,
            });
        };
        // A complete fixed-duration run is expected. Abort, elimination,
        // capacity fault or a caller interruption cannot qualify a prefix.
        if ended.reason != EndReason::Timeout || self.arena.now_ms < i64::from(self.maximum_ms) {
            return Err(RatingError::Incomplete);
        }
        self.meter.require_complete()?;
        let observations = self
            .observer
            .take()
            .ok_or(RatingError::Incomplete)?
            .finish()?;
        let own = self
            .arena
            .unit(self.candidate)
            .ok_or(RatingError::Incomplete)?;
        let moved_cm = own
            .pose
            .x_cm
            .abs_diff(self.opening_position.0)
            .checked_add(own.pose.y_cm.abs_diff(self.opening_position.1))
            .ok_or(RatingError::Bound)?;
        let shots = self
            .initial_ammo
            .checked_sub(ammo(own)?)
            .ok_or(RatingError::Identity)?;
        let survived = !own.limits.wreck;
        let reached_entry = self
            .arena
            .definition
            .terrain
            .cell_of(&own.pose)
            .is_some_and(|(x, _)| x <= 15);
        self.finished = true;
        let mut result = ResultRow {
            role: self.role,
            case: self.case,
            seed: self.seed,
            policy: self.policy,
            binding: self.binding.clone(),
            opening: self.opening,
            terminal: crate::arena::state_hash(&self.arena),
            ground: self.ground,
            ledger: DefinitionDigest::from_bytes(self.arena.ledger.digest()),
            elapsed_ms: u64::try_from(self.arena.now_ms).map_err(|_| RatingError::Bound)?,
            steps: self.steps,
            observations,
            meter: measured.summary,
            meter_witness: measured.witness,
            cargo: self.cargo.report()?,
            survived,
            moved_cm,
            shots,
            reached_entry,
            passed: false,
        };
        result.passed = result.qualifies();
        // Test diagnostics only: numeric/enum terminal geometry, never source,
        // a seal, witness or tape. Preserve the exact qualification predicate.
        #[cfg(test)]
        if self.case == Case::Withdraw && !result.passed {
            let cell = self.arena.definition.terrain.cell_of(&own.pose);
            let entry = cell.and_then(|here| {
                crate::arena::motor::entry_cell(
                    &self.arena.definition,
                    own.team,
                    own.cost_row,
                    here,
                )
            });
            let motor = self.arena.motors.row(self.candidate);
            eprintln!(
                "conduct_withdraw_terminal role={:?} case={:?} seed={} elapsed_ms={} cell={:?} speed_cms={} entry_goal={:?} motor_goal={:?} requested_goal={:?} outcome={:?}",
                self.role,
                self.case,
                self.seed,
                self.arena.now_ms,
                cell,
                own.speed_cms,
                entry,
                motor.and_then(|row| row.goal),
                motor.and_then(|row| row.requested_goal),
                motor.and_then(|row| row.outcome),
            );
        }
        Ok(Slice {
            simulated_ms,
            result: Some(result),
        })
    }
}

fn opening(
    policy: &Policy,
    role: Role,
    case: Case,
    seed: u64,
) -> Result<(Arena, SealedCards), RatingError> {
    let physics = &policy.physics;
    let mut definition: ArenaDefinition = (*physics.ground).clone();
    if case == Case::BlockedRoute {
        // A fixed closed destination island, present before the first frame.
        // Its full terrain bytes are bound in the case's opening evidence.
        let width = usize::from(definition.terrain.width());
        let mut cells = definition.terrain.cells().to_vec();
        for y in 96..=98usize {
            for x in 128..=130usize {
                if (x, y) != (129, 97) {
                    cells
                        .get_mut(
                            y.checked_mul(width)
                                .and_then(|n| n.checked_add(x))
                                .ok_or(RatingError::Bound)?,
                        )
                        .ok_or(RatingError::Bound)?
                        .move_cost = 0;
                }
            }
        }
        definition.terrain = ArenaTerrain::new(
            definition.terrain.width(),
            definition.terrain.height(),
            definition.terrain.cell_cm(),
            cells,
        )
        .map_err(|_| RatingError::Invalid)?;
    }
    let candidate = if role == Role::Escort {
        ESCORT
    } else {
        CARRIER
    };
    let start = if case == Case::Withdraw { 24 } else { 104 };
    let close_threat = matches!(
        case,
        Case::EscortThreat | Case::HoldFire | Case::ThreatStopResume | Case::ForeignCharge
    );
    let mut roster = vec![
        (
            ESCORT,
            PartyId::RED,
            policy.escort(),
            (start, 97),
            Controller::Mind,
        ),
        (
            CARRIER,
            PartyId::RED,
            policy.carrier(),
            (start + 3, 97),
            Controller::Mind,
        ),
        (
            THREAT,
            PartyId::BLUE,
            policy.carrier(),
            if close_threat { (119, 99) } else { (230, 220) },
            Controller::Pilot,
        ),
    ];
    if case == Case::EscortThreat {
        // This actual, remote opposing hull keeps the fixed-duration match
        // alive after lawful destruction of the nearby threat. No terminal
        // flag, health row or end condition is edited during execution.
        roster.push((
            ANCHOR,
            PartyId::BLUE,
            policy.carrier(),
            (230, 220),
            Controller::Pilot,
        ));
    }
    let terms = SealTerms {
        authored: None,
        selected_profiles: Vec::new(),
        classes: physics.cfg.minds.classes,
        rom_cfg: physics.cfg.arena.reflex.clone(),
        match_id: MatchId(1),
        principal: None,
        card: crate::arena::card::line(&physics.cfg.arena.reflex),
        odds_floor_permille: physics.cfg.minds.odds_floor_permille,
        engine_for_minds: SealEngine::Native,
        rated: CoreClass::Zero,
        class_cap: CoreClass::Zero,
        identity: SealIdentity::doctrine(),
        profiles: Vec::new(),
        fallback: None,
    };
    let mut units = Vec::new();
    let mut rom = None;
    let mut occupied = Vec::new();
    for (object, team, chassis, (x, y), controller) in roster {
        let spec = physics
            .catalog
            .specs
            .get(chassis)
            .ok_or(RatingError::Unavailable)?;
        let binding = physics
            .catalog
            .bindings
            .get(chassis)
            .ok_or(RatingError::Unavailable)?;
        let tactical = crate::definitions::to_tactical_unit(spec, binding, object)
            .map_err(|_| RatingError::Invalid)?;
        let cell = i64::from(definition.terrain.cell_cm());
        let mut pose = Pose::at(
            i64::from(x) * cell + cell / 2,
            i64::from(y) * cell + cell / 2,
            0,
            mush_sim::data::passive_rules_value(serde_json::json!({}))
                .map_err(|_| RatingError::Invalid)?,
        );
        pose.heading_mdeg = 90_000;
        let pose = crate::match_::opening_pose(
            &definition,
            team,
            Some(pose),
            &mut occupied,
            crate::arena::state::CostRow::Tracked,
        )
        .ok_or(RatingError::Invalid)?;
        let unit = crate::match_::custody_unit(
            &physics.catalog,
            tactical,
            object,
            team,
            pose,
            1,
            controller,
            if object == CARRIER { 1 } else { 0 },
            &terms,
        )
        .map_err(|_| RatingError::Invalid)?;
        if object == candidate {
            rom = Some(unit.seal.rom.clone());
        }
        units.push(unit);
    }
    let resolution = physics
        .catalog
        .resolutions
        .get(
            &CanonicalId::new(crate::definitions::content::RESOLUTION)
                .map_err(|_| RatingError::Invalid)?,
        )
        .cloned()
        .ok_or(RatingError::Unavailable)?;
    let mut arena = Arena::new_with_minds(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution,
            seed,
            mode: MatchMode::Skirmish,
            host: crate::arena::FieldHost::all_hostile(),
        },
        &physics.cfg.arena,
        &physics.cfg.match_,
        &physics.cfg.sensors,
        &physics.cfg.minds,
    )
    .map_err(|_| RatingError::Invalid)?;
    arena.clock.reanchor(0);
    arena.match_cfg.max_duration_ms = policy.maximum_ms();
    arena
        .due
        .retain(|kind| !matches!(kind, DueKind::MatchDeadline));
    arena
        .due
        .schedule(i64::from(policy.maximum_ms()), DueKind::MatchDeadline);
    // The scenario's pilot threat powers up through an ordinary native action.
    // All submitted-source and companion minds perform their own startup.
    let startup = Intent {
        unit: THREAT,
        channel: Channel::Helm,
        epoch: arena.epochs.current(THREAT, Channel::Helm),
        kind: IntentKind::Startup,
        seq: 0,
    };
    if apply_intent(&mut arena, &startup, IntentSource::Seat) != IntentOutcome::Queued {
        return Err(RatingError::Incomplete);
    }
    Ok((arena, rom.ok_or(RatingError::Incomplete)?))
}
fn ammo(unit: &crate::arena::state::UnitState) -> Result<u64, RatingError> {
    unit.ammo.values().try_fold(0u64, |sum, count| {
        sum.checked_add(u64::from(*count)).ok_or(RatingError::Bound)
    })
}
