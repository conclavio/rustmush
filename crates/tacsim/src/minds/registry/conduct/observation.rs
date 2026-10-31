//! Read-only observations at the real collect/answer boundary. No extra view
//! is built, no contact is injected, and every answer is returned unchanged.
use super::{super::identity, Role};
use crate::arena::{
    conditions::{Arming, FireCondition, SlotTarget},
    intents::{IntentKind, MotorGoal},
    mind::{CollectedThink, ThinkAnswer, ViewContact},
    perception::{DriveOutcome, MindObjective},
    schema::CardValue,
    state::Power,
};
use mush_core::{DefinitionDigest, ObjId};
use mush_roll::commerce::rating::RatingError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Observations {
    pub(super) thinks: u64,
    pub(super) advance: u64,
    pub(super) support: u64,
    pub(super) withdraw: u64,
    pub(super) friendly_charge: u64,
    pub(super) missing_charge: u64,
    pub(super) threat: u64,
    pub(super) stopped_for_threat: u64,
    pub(super) drove_without_threat: u64,
    pub(super) no_route: u64,
    pub(super) retry_attempts: u64,
    pub(super) forbidden_fire: u64,
    pub(super) forbidden_move: u64,
    pub(super) retained_move: u64,
    pub(super) witness: DefinitionDigest,
}
pub(super) struct Observer {
    unit: ObjId,
    role: Role,
    next: u64,
    no_route_at: Option<i64>,
    last_retry: Option<i64>,
    brake_cms2: u32,
    accel_cms2: u32,
    native_ms: u32,
    stop: Option<(i64, i64)>,
    value: Observations,
}
impl Observer {
    pub(super) fn new(
        unit: &crate::arena::state::UnitState,
        role: Role,
        native_ms: u32,
    ) -> Result<Self, RatingError> {
        if unit.mobility.brake_cms2 == 0 || native_ms == 0 || native_ms > 1000 {
            return Err(RatingError::Identity);
        }
        Ok(Self {
            unit: unit.id,
            role,
            next: 0,
            no_route_at: None,
            last_retry: None,
            brake_cms2: unit.mobility.brake_cms2,
            accel_cms2: unit.mobility.accel_cms2,
            native_ms,
            stop: None,
            value: Observations {
                thinks: 0,
                advance: 0,
                support: 0,
                withdraw: 0,
                friendly_charge: 0,
                missing_charge: 0,
                threat: 0,
                stopped_for_threat: 0,
                drove_without_threat: 0,
                no_route: 0,
                retry_attempts: 0,
                forbidden_fire: 0,
                forbidden_move: 0,
                retained_move: 0,
                witness: identity(
                    "tacsim.conduct.observations.begin.v2",
                    &(
                        unit.id,
                        role,
                        unit.mobility.brake_cms2,
                        unit.mobility.accel_cms2,
                        native_ms,
                    ),
                    4096,
                )?,
            },
        })
    }
    pub(super) fn record(
        &mut self,
        think: &CollectedThink,
        answer: &ThinkAnswer,
    ) -> Result<(), RatingError> {
        if think.unit != self.unit
            || answer.unit != self.unit
            || answer.think_seq != think.think_seq
            || think.think_seq != self.next
            || think.dropped
            || answer.result.fault.is_some()
        {
            return Err(RatingError::Incomplete);
        }
        self.next = self.next.checked_add(1).ok_or(RatingError::Bound)?;
        if self.next > 4096 {
            return Err(RatingError::Bound);
        }
        let view = &think.view;
        let values = think.card.values().ok_or(RatingError::Incomplete)?;
        let role_name = match self.role {
            Role::Escort => "escort",
            Role::Transit => "transit",
        };
        if !matches!(values.get("role"), Some(CardValue::Word(word)) if word.as_str() == role_name)
        {
            return Err(RatingError::Identity);
        }
        let integer = |key: &str| -> Result<i64, RatingError> {
            match values.get(key) {
                Some(CardValue::Integer(n)) => Ok(*n),
                _ => Err(RatingError::Identity),
            }
        };
        let age = integer("fresh_ms")?;
        let threat_m = u64::try_from(integer("threat_m")?).map_err(|_| RatingError::Bound)?;
        let retry = integer("retry_ms")?;
        let station = u64::try_from(integer("station_m")?).map_err(|_| RatingError::Bound)?;
        let leash = u64::try_from(integer("leash_m")?).map_err(|_| RatingError::Bound)?;
        let floor = u16::try_from(integer("fire_min_permille")?).map_err(|_| RatingError::Bound)?;
        let withdraw =
            u32::try_from(integer("withdraw_permille")?).map_err(|_| RatingError::Bound)?;
        if !(1000..=5000).contains(&age)
            || !(1000..=5000).contains(&retry)
            || !(100..=600).contains(&threat_m)
            || !(10..=50).contains(&station)
            || !(50..=400).contains(&leash)
            || station > leash
            || !(100..=500).contains(&withdraw)
            || !(100..=900).contains(&floor)
        {
            return Err(RatingError::Identity);
        }
        let fresh = |row: &&ViewContact| {
            row.age_ms >= 0
                && row.age_ms <= age
                && row.cell.is_some()
                && row.reads_dead != Some(true)
                && row.class.party().is_some()
        };
        let current = view
            .order
            .as_ref()
            .filter(|order| order.expires_ms > view.t_ms);
        let objective = current.map(|order| order.objective);
        let charge = match objective {
            Some(MindObjective::Support { of }) => of,
            _ => None,
        };
        let held_charge = view
            .contacts
            .iter()
            .filter(fresh)
            .find(|row| Some(row.id) == charge && row.relation.is_friendly());
        let threat = view.contacts.iter().filter(fresh).any(|row| {
            row.relation == crate::arena::party::Relation::Hostile && row.range_m <= threat_m
        });
        let intents = &answer.result.intents;
        let drive = intents.iter().any(|intent| {
            matches!(intent, IntentKind::Goal(MotorGoal::Cell { .. }))
                || matches!(intent, IntentKind::Speed { cms } if *cms != 0)
        });
        let fire = intents.iter().any(|intent| matches!(intent, IntentKind::Fire { .. }
            | IntentKind::Condition(Arming::Arm(_))) || matches!(intent,
                IntentKind::Condition(Arming::Rules { rules, .. }) if rules.iter().any(|rule| rule.fire().is_some())));
        let online = view.power == Power::Online;
        // A Hold answer is a response, not evidence that the native hull has
        // already stopped. Only the next actual views can establish that.
        let held = view.speed_cms == 0 && view.motor.is_none() && !drive;
        self.value.thinks += 1;
        self.value.advance += u64::from(matches!(objective, Some(MindObjective::Advance { .. })));
        self.value.support += u64::from(matches!(objective, Some(MindObjective::Support { .. })));
        self.value.withdraw += u64::from(matches!(objective, Some(MindObjective::Withdraw)));
        self.value.friendly_charge += u64::from(held_charge.is_some());
        self.value.missing_charge += u64::from(
            matches!(objective, Some(MindObjective::Support { .. })) && held_charge.is_none(),
        );
        self.value.threat += u64::from(online && threat);
        self.value.stopped_for_threat += u64::from(online && threat && held);
        self.value.drove_without_threat += u64::from(online && !threat && drive);
        let no_route = view
            .motor
            .is_some_and(|motor| motor.outcome == Some(DriveOutcome::NoRoute));
        if no_route {
            self.value.no_route += 1;
            self.no_route_at.get_or_insert(view.t_ms);
        }
        if drive && let Some(blocked) = self.no_route_at {
            if view.t_ms < blocked.checked_add(retry).ok_or(RatingError::Bound)?
                || self
                    .last_retry
                    .is_some_and(|at| view.t_ms < at.saturating_add(retry))
            {
                self.value.forbidden_move += 1;
            }
            self.value.retry_attempts += 1;
            self.last_retry = Some(view.t_ms);
            self.no_route_at = None;
        }
        let withdrawing = matches!(objective, Some(MindObjective::Withdraw))
            || crate::arena::stock_mind::health_permille(view) < withdraw;
        // Judge every emitted move against the same native, public-view
        // geometry used by the sandbox kernels. Merely ending near the convoy
        // cannot hide an earlier chase or a caller-selected shortcut.
        let expected_goal = if withdrawing {
            view.cell.and_then(|cell| {
                crate::arena::motor::entry_interior_cell(
                    &think.map,
                    view.party,
                    crate::arena::state::CostRow::Tracked,
                    cell,
                )
            })
        } else {
            match self.role {
                Role::Escort => held_charge
                    .filter(|row| row.range_m > station)
                    .and_then(|row| {
                        crate::arena::stock_mind::stand_off_of(
                            &think.map,
                            view,
                            row,
                            station.checked_mul(100)?,
                        )
                    }),
                Role::Transit if !threat => match objective {
                    Some(MindObjective::Advance { to }) => Some(to),
                    _ => None,
                },
                Role::Transit => None,
            }
        };
        self.value.forbidden_move += u64::from(intents.iter().any(|intent| match intent {
            IntentKind::Goal(MotorGoal::Cell { x, y }) => expected_goal != Some((*x, *y)),
            IntentKind::Speed { cms } => *cms != 0,
            IntentKind::Heading { .. } => true,
            _ => false,
        }));
        let fire_forbidden = self.role == Role::Transit
            || current.is_none()
            || current.is_some_and(|order| order.hold_fire)
            || withdrawing
            || !threat
            || self.role == Role::Escort && held_charge.is_none_or(|row| row.range_m > leash);
        self.value.forbidden_fire += u64::from(fire_forbidden && fire);
        // A lawful native condition can still name the wrong public target.
        // Keep the qualification's narrower role rules independent of native
        // slot admission, while native code remains the only shot resolver.
        let hostile = |id| {
            view.contacts.iter().filter(fresh).any(|row| {
                row.id == id
                    && row.relation == crate::arena::party::Relation::Hostile
                    && row.range_m <= threat_m
            })
        };
        let condition_ok = |condition: &FireCondition| {
            let target_ok = match condition.target {
                SlotTarget::Contact { id } => hostile(id),
                SlotTarget::Lock | SlotTarget::BestHostile => view.lock.is_some_and(hostile),
            };
            target_ok
                && condition.min_permille >= floor
                && u64::from(condition.max_range_m) <= threat_m
                && view
                    .weapons
                    .iter()
                    .any(|weapon| weapon.id == condition.weapon)
        };
        self.value.forbidden_fire += u64::from(intents.iter().any(|intent| {
            match intent {
                IntentKind::Condition(Arming::Arm(condition)) => !condition_ok(condition),
                IntentKind::Condition(Arming::Rules { rules, .. }) => rules
                    .iter()
                    .filter_map(|rule| rule.fire())
                    .any(|condition| !condition_ok(condition)),
                IntentKind::Fire { weapon, slot } => !view.rules.iter().any(|row| {
                    Some(row.slot) == *slot
                        && row.rule.fire().is_some_and(|condition| {
                            condition.weapon == *weapon && condition_ok(condition)
                        })
                }),
                IntentKind::Lock { contact } => fire_forbidden || !hostile(*contact),
                _ => false,
            }
        }));
        let move_forbidden = online
            && !withdrawing
            && match self.role {
                Role::Escort => {
                    !matches!(objective, Some(MindObjective::Support { .. }))
                        || held_charge.is_none()
                }
                Role::Transit => {
                    !matches!(objective, Some(MindObjective::Advance { .. })) || threat
                }
            };
        self.value.forbidden_move += u64::from(move_forbidden && drive);
        // Continuous native goals survive a silent answer. Once this role has
        // no lawful goal (including a lost charge or a Transit threat), require
        // a real stop response and observe its effect. Keep the FIRST landing
        // deadline: repeated Hold answers cannot renew the stopping allowance.
        // Native Hold/Speed(0) clears the route on landing. Motion then brakes
        // at the bound hull's rate; two native steps cover landing/integration
        // lattice rounding, not a fresh grace window for every observation.
        if online && !withdrawing && expected_goal.is_none() {
            let first = self.stop.is_none();
            let (landed, stopped) = match self.stop {
                Some(value) => value,
                None => {
                    let latency = u64::try_from(
                        think
                            .lands_ms
                            .checked_sub(view.t_ms)
                            .ok_or(RatingError::Bound)?,
                    )
                    .map_err(|_| RatingError::Identity)?;
                    if latency > u64::from(self.native_ms) {
                        return Err(RatingError::Identity);
                    }
                    let gained = u64::from(self.accel_cms2)
                        .checked_mul(latency)
                        .ok_or(RatingError::Bound)?
                        .div_ceil(1000);
                    let speed = u64::from(view.speed_cms.unsigned_abs())
                        .checked_add(gained)
                        .ok_or(RatingError::Bound)?;
                    let brake_ms = speed
                        .checked_mul(1000)
                        .ok_or(RatingError::Bound)?
                        .div_ceil(u64::from(self.brake_cms2));
                    let delay = brake_ms
                        .checked_add(2 * u64::from(self.native_ms))
                        .ok_or(RatingError::Bound)?;
                    let stopped = think
                        .lands_ms
                        .checked_add(i64::try_from(delay).map_err(|_| RatingError::Bound)?)
                        .ok_or(RatingError::Bound)?;
                    let value = (think.lands_ms, stopped);
                    self.stop = Some(value);
                    value
                }
            };
            let response = intents.iter().any(|intent| {
                matches!(
                    intent,
                    IntentKind::Goal(MotorGoal::Hold) | IntentKind::Speed { cms: 0 }
                )
            });
            let retained = view.motor.is_some();
            let violation = (retained || first && view.speed_cms != 0) && !response
                || view.t_ms > landed && retained
                || view.t_ms >= stopped && view.speed_cms != 0;
            self.value.retained_move += u64::from(violation);
            self.value.forbidden_move += u64::from(violation);
        } else {
            self.stop = None;
        }
        let contacts = view
            .contacts
            .iter()
            .map(|row| {
                (
                    row.id,
                    row.class.code(),
                    row.range_m,
                    row.age_ms,
                    row.cell,
                    row.reads_dead,
                )
            })
            .collect::<Vec<_>>();
        self.value.witness = identity(
            "tacsim.conduct.observations.step.v1",
            &(
                self.value.witness,
                think.unit,
                think.think_seq,
                think.t_ms,
                think.lands_ms,
                &view.order,
                view.cell,
                view.speed_cms,
                view.motor.map(|m| (m.requested_goal, m.outcome)),
                contacts,
                intents,
                identity("tacsim.conduct.memory.v1", &answer.result.memory, 16 * 1024)?,
            ),
            128 * 1024,
        )?;
        Ok(())
    }
    pub(super) fn finish(self) -> Result<Observations, RatingError> {
        if self.value.thinks == 0 {
            return Err(RatingError::Incomplete);
        }
        Ok(self.value)
    }
}
