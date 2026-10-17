//! Complete sealed rule programs and bounded, expiring think-term samples.
//!
//! Binding resolves symbolic weapon names only. Every think term stays in the
//! complete program. A native action from a sampled row is authorized only while
//! its matching validated sample is live, so HOLD vetoes and first-match FIRE
//! ordering use the same gate. No rules helper can strip a think predicate.

use super::conditions::{
    ConditionTable, Rule, RulePredicate, condition_is_lawful, validate_rule_shape,
};
use super::core::{CoreProfile, MAX_CONDITION_SLOTS};
use super::mind::MindView;
use super::perception::MindObjective;
use super::schema::{AuthoredRule, OrderKind, SchemaError, ThinkPredicate};
use super::state::Channel;
use mush_core::{DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

fn invalid(field: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

/// One concrete sealed rule; index is stable even for an absent station.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundRule {
    /// Complete typed action/native AND predicates plus all think gates.
    pub program: AuthoredRule,
    /// An absent station is readable and inert, never an admission failure.
    pub inert: bool,
}

/// Complete immutable program for this card on this hull's carried weapons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundProgram {
    /// Digest of the ordered complete programs and station-inert flags.
    pub digest: DefinitionDigest,
    /// At most the effective core's slots, preserving expanded authored order.
    pub rows: Vec<BoundRule>,
}

impl BoundProgram {
    /// Bind ALL in stable carried-weapon order, and refuse the whole selection
    /// before any ConditionTable write if expansion, symbols or floor fail.
    /// `weapons` is custody's public ordered carried id list, never hidden state.
    /// # Errors
    /// Invalid shapes, missing weapon ids, floor or effective slot capacity.
    pub fn bind(
        rules: &[AuthoredRule],
        weapons: &[SmolStr],
        channels: &[Channel],
        profile: &CoreProfile,
        floor: u32,
    ) -> Result<Self, SchemaError> {
        if rules.len() > profile.slots || profile.slots > MAX_CONDITION_SLOTS {
            return Err(invalid("rules", "rule count exceeds effective class"));
        }
        let mut rows = Vec::new();
        for program in rules {
            program.validate()?;
            validate_rule_shape(&program.rule)
                .map_err(|_| invalid("rule", "invalid native rule"))?;
            if program.rule.predicates().len() + program.think.len() > 4 {
                return Err(invalid("rule", "maximum four AND predicates"));
            }
            let inert = !channels.contains(&program.rule.channel());
            let all = program
                .rule
                .weapon()
                .is_some_and(|w| w.eq_ignore_ascii_case("all"));
            if all && program.rule.channel() != Channel::Fire {
                return Err(invalid("weapon", "ALL is only a fire-station action"));
            }
            // One inert symbolic row preserves authored capacity/readback on an
            // unarmed hull. It cannot enter the live ConditionTable.
            let targets: Vec<_> = if all && !inert {
                weapons.iter().map(Some).collect()
            } else {
                vec![None]
            };
            if targets.is_empty() {
                return Err(invalid("weapon", "no carried weapon for ALL"));
            }
            for target in targets {
                let mut bound = program.clone();
                if let Some(target) = target {
                    set_action_weapon(&mut bound.rule, target);
                }
                if !inert {
                    validate_weapons(&bound.rule, weapons)?;
                    if let Some(fire) = bound.rule.fire() {
                        condition_is_lawful(fire, floor).map_err(|_| {
                            invalid("odds", "fire floor is below the Compact floor")
                        })?;
                    }
                    if bound.rule.predicates().iter().any(|p| matches!(p, RulePredicate::Odds{min_permille,..} if u32::from(*min_permille)<floor)) {
                        return Err(invalid("odds", "predicate is below the Compact floor"));
                    }
                }
                rows.push(BoundRule {
                    program: bound,
                    inert,
                });
                if rows.len() > profile.slots {
                    return Err(invalid("rules", "ALL expansion exceeds effective class"));
                }
            }
        }
        Self::from_rows(rows)
    }

    fn from_rows(rows: Vec<BoundRule>) -> Result<Self, SchemaError> {
        if rows.len() > MAX_CONDITION_SLOTS {
            return Err(invalid("rules", "maximum six bound rows"));
        }
        let digest = DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.bound-rules.v1",
            &serde_json::to_value(&rows).map_err(|_| invalid("rules", "cannot encode rules"))?,
            64 * 1024,
        )
        .map_err(|e| invalid("rules", e.to_string()))?;
        Ok(Self { digest, rows })
    }

    /// Validate a decoded program against a fresh binding of its sealed profile.
    /// # Errors
    /// Any tampered, reordered, omitted or relabelled row.
    pub fn verify(&self, expected: &Self) -> Result<(), SchemaError> {
        if self != expected || Self::from_rows(self.rows.clone())?.digest != self.digest {
            return Err(invalid(
                "rules",
                "bound program does not match sealed catalog and hull",
            ));
        }
        Ok(())
    }

    /// Atomic replacement payload. Sampled rows remain represented; the native
    /// evaluator must additionally ask RuleLease::permits for every live row.
    #[must_use]
    pub fn native_rows(&self) -> Vec<Option<Rule>> {
        self.rows
            .iter()
            .map(|row| (!row.inert).then(|| row.program.rule.clone()))
            .collect()
    }

    /// Install all concrete rows as one transaction, never a sequence visible
    /// between channels. The caller must reset its RuleLease in the same commit.
    /// # Errors
    /// Shape or effective capacity; existing table remains untouched.
    pub fn install(
        &self,
        table: &mut ConditionTable,
        unit: ObjId,
        capacity: usize,
    ) -> Result<(), SchemaError> {
        table
            .replace_profile_rules(unit, &self.native_rows(), capacity)
            .map_err(|_| invalid("rules", "profile rules cannot replace current slots"))
    }

    /// Sample every think term only from the supplied immutable public view.
    /// The caller must charge the declared bounded fee BEFORE invoking this.
    /// `last_hit_ms` is view.own_last_hit_ms, never an inferred notice clock.
    #[must_use]
    pub fn sample(&self, view: &MindView, last_hit_ms: Option<i64>) -> Vec<bool> {
        self.rows
            .iter()
            .map(|row| {
                !row.inert
                    && row.program.think.iter().all(|p| match p {
                        ThinkPredicate::HitWithin { milliseconds } => {
                            last_hit_ms.is_some_and(|at| {
                                at <= view.t_ms
                                    && view.t_ms.saturating_sub(at) <= i64::from(*milliseconds)
                            })
                        }
                        ThinkPredicate::Hostiles { at_least } => {
                            view.contacts
                                .iter()
                                .filter(|c| {
                                    c.class.team().is_some_and(|team| team != view.team)
                                        && c.reads_dead != Some(true)
                                })
                                .count()
                                >= usize::from(*at_least)
                        }
                        ThinkPredicate::Order { is } => order_kind(view) == *is,
                    })
            })
            .collect()
    }

    /// Predictable fee over at most six rows, four terms and 24 visible contacts.
    /// Charge once on the first rules helper call; later calls return its sample.
    #[must_use]
    pub fn sample_fee(&self, view_rows: usize) -> u64 {
        4 + self
            .rows
            .iter()
            .map(|row| {
                2 + row
                    .program
                    .think
                    .iter()
                    .map(|p| match p {
                        ThinkPredicate::Hostiles { .. } => {
                            2 + u64::try_from(view_rows.min(24)).unwrap_or(24)
                        }
                        _ => 2,
                    })
                    .sum::<u64>()
            })
            .sum::<u64>()
    }
}

fn set_action_weapon(rule: &mut Rule, weapon: &SmolStr) {
    match rule {
        Rule::Fire { condition, .. } => condition.weapon = weapon.clone(),
        Rule::HoldFire {
            weapon: current, ..
        } => *current = weapon.clone(),
        Rule::Lock {
            weapon: current, ..
        } => *current = Some(weapon.clone()),
    }
}
fn validate_weapons(rule: &Rule, weapons: &[SmolStr]) -> Result<(), SchemaError> {
    if rule.weapon().is_some_and(|w| !weapons.contains(w))
        || rule.predicates().iter().any(|p| match p {
            RulePredicate::Odds {
                weapon: Some(w), ..
            }
            | RulePredicate::Rounds {
                weapon: Some(w), ..
            } => !weapons.contains(w),
            _ => false,
        })
    {
        return Err(invalid("weapon", "symbol is not carried by this hull"));
    }
    Ok(())
}
fn order_kind(view: &MindView) -> OrderKind {
    match view
        .order
        .as_ref()
        .filter(|order| order.expires_ms > view.t_ms)
        .map(|o| &o.objective)
    {
        None => OrderKind::None,
        Some(MindObjective::Hold) => OrderKind::Hold,
        Some(MindObjective::Advance { .. }) => OrderKind::Advance,
        Some(MindObjective::Engage { .. }) => OrderKind::Engage,
        Some(MindObjective::Withdraw) => OrderKind::Withdraw,
        Some(MindObjective::Screen { .. }) => OrderKind::Screen,
        Some(MindObjective::Support { .. }) => OrderKind::Support,
    }
}

/// Runner-produced sidecar, accepted only at the matching successful landing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSample {
    /// Complete program identity and current profile generation.
    pub program: DefinitionDigest,
    pub generation: u64,
    /// Frozen view time, existing scheduled landing, next scheduled landing.
    pub sampled_ms: i64,
    pub lands_ms: i64,
    pub expires_ms: i64,
    /// Truth of ALL think terms, stable slot order; pure-native bits are ignored.
    pub truth: Vec<bool>,
}

/// Carried active sample. None means every row with think terms is inactive.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleLease(Option<RuleSample>);
impl RuleLease {
    /// Profile, seat/controller, core, wreck or fault invalidation.
    pub fn clear(&mut self) {
        self.0 = None;
    }
    /// Accept one complete sample after caller has checked all captured station
    /// epochs and the PendingThink profile generation. No partial success.
    /// # Errors
    /// Bad identity/count/time/generation. Old sample is cleared on any refusal.
    pub fn land(
        &mut self,
        sample: Option<RuleSample>,
        program: &BoundProgram,
        generation: u64,
        t_ms: i64,
        expected_expiry: i64,
    ) -> Result<(), SchemaError> {
        self.clear();
        let Some(sample) = sample else {
            return Ok(());
        };
        if sample.program != program.digest
            || sample.generation != generation
            || sample.truth.len() != program.rows.len()
            || sample.truth.len() > MAX_CONDITION_SLOTS
            || sample.lands_ms != t_ms
            || sample.sampled_ms < 0
            || sample.sampled_ms > t_ms
            || sample.expires_ms != expected_expiry
            || sample.expires_ms <= t_ms
        {
            return Err(invalid(
                "rule sample",
                "stale or malformed profile rule sample",
            ));
        }
        self.0 = Some(sample);
        Ok(())
    }
    /// Gate every authored FIRE, HOLD and LOCK native evaluator entry. An
    /// ordinary pilot/manual rule not matching the sealed row has its own law;
    /// the integration removes that slot from sealed bindings on manual edit.
    #[must_use]
    pub fn permits(
        &self,
        program: &BoundProgram,
        generation: u64,
        slot: u8,
        rule: &Rule,
        t_ms: i64,
    ) -> bool {
        let Some(index) = usize::from(slot).checked_sub(1) else {
            return false;
        };
        let Some(row) = program.rows.get(index) else {
            return false;
        };
        if row.inert || &row.program.rule != rule {
            return false;
        }
        if row.program.think.is_empty() {
            return true;
        }
        self.0.as_ref().is_some_and(|sample| {
            sample.program == program.digest
                && sample.generation == generation
                && sample.lands_ms <= t_ms
                && t_ms < sample.expires_ms
                && sample.truth.get(index) == Some(&true)
        })
    }
    /// Validate a carried sample before snapshot publication. Expired samples
    /// may be carried but grant no authority; future/stale/malformed ones refuse.
    /// # Errors
    /// Program/generation/count mismatch or impossible arena time order.
    pub fn validate(
        &self,
        program: &BoundProgram,
        generation: u64,
        now_ms: i64,
        sample_period_ms: i64,
    ) -> Result<(), SchemaError> {
        if let Some(sample) = &self.0
            && (sample.program != program.digest
                || sample.generation != generation
                || sample.truth.len() != program.rows.len()
                || sample.truth.len() > MAX_CONDITION_SLOTS
                || sample.sampled_ms < 0
                || sample.sampled_ms > sample.lands_ms
                || sample.lands_ms > now_ms
                || sample_period_ms <= 0
                || sample.lands_ms.checked_add(sample_period_ms) != Some(sample.expires_ms)
                || sample.expires_ms <= sample.lands_ms)
        {
            return Err(invalid(
                "rule sample",
                "invalid carried sample identity or time",
            ));
        }
        Ok(())
    }

    /// Read exact carried sample for the snapshot hash and safe profile frame.
    #[must_use]
    pub fn sample(&self) -> Option<&RuleSample> {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::super::conditions::{ASPECTS_ANY, FireCondition, SlotTarget};
    use super::super::core::{ClassTable, CoreClass};
    use super::*;
    fn fire(weapon: &str, think: Vec<ThinkPredicate>) -> AuthoredRule {
        AuthoredRule {
            rule: Rule::fire_rule(
                FireCondition {
                    weapon: weapon.into(),
                    target: SlotTarget::Lock,
                    min_permille: 100,
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                },
                vec![],
            ),
            think,
        }
    }
    fn bind(rules: &[AuthoredRule], class: CoreClass) -> BoundProgram {
        BoundProgram::bind(
            rules,
            &["cannon".into(), "mg".into()],
            &Channel::ALL,
            &ClassTable::STOCK.row(class),
            100,
        )
        .unwrap()
    }
    fn sample(program: &BoundProgram, truth: Vec<bool>) -> RuleSample {
        RuleSample {
            program: program.digest,
            generation: 7,
            sampled_ms: 1000,
            lands_ms: 1020,
            expires_ms: 2020,
            truth,
        }
    }
    #[test]
    fn m6_all_capacity_refuses_before_table_changes_and_binds_order() {
        let rules = vec![fire("ALL", vec![]), fire("cannon", vec![])];
        assert!(
            BoundProgram::bind(
                &rules,
                &["cannon".into(), "mg".into()],
                &Channel::ALL,
                &ClassTable::STOCK.row(CoreClass::I),
                100
            )
            .is_err()
        );
        let bound = bind(&rules, CoreClass::II);
        assert_eq!(
            bound
                .rows
                .iter()
                .map(|row| row.program.rule.weapon().unwrap().as_str())
                .collect::<Vec<_>>(),
            ["cannon", "mg", "cannon"]
        );
        let unit = ObjId(1);
        let mut table = ConditionTable::default();
        bind(&[fire("mg", vec![])], CoreClass::I)
            .install(&mut table, unit, 2)
            .unwrap();
        let before = table.clone();
        assert!(bound.install(&mut table, unit, 2).is_err());
        assert_eq!(table, before);
    }
    #[test]
    fn m6_unarmed_fire_is_retained_inert_and_never_installed() {
        let bound = BoundProgram::bind(
            &[fire("cannon", vec![])],
            &[],
            &[Channel::Helm, Channel::Sensors],
            &ClassTable::STOCK.row(CoreClass::I),
            500,
        )
        .unwrap();
        assert_eq!(bound.rows.len(), 1);
        assert!(bound.rows[0].inert);
        let mut table = ConditionTable::default();
        bound.install(&mut table, ObjId(1), 2).unwrap();
        assert!(table.is_empty_for(ObjId(1)));
    }
    #[test]
    fn m6_think_gate_is_inactive_until_matching_landing_and_expires_exactly() {
        let bound = bind(
            &[fire(
                "cannon",
                vec![ThinkPredicate::HitWithin { milliseconds: 2000 }],
            )],
            CoreClass::I,
        );
        let rule = &bound.rows[0].program.rule;
        let mut lease = RuleLease::default();
        assert!(!lease.permits(&bound, 7, 1, rule, 1020));
        lease
            .land(Some(sample(&bound, vec![true])), &bound, 7, 1020, 2020)
            .unwrap();
        assert!(!lease.permits(&bound, 7, 1, rule, 1019));
        assert!(lease.permits(&bound, 7, 1, rule, 1020));
        assert!(lease.permits(&bound, 7, 1, rule, 2019));
        assert!(!lease.permits(&bound, 7, 1, rule, 2020));
        assert!(!lease.permits(&bound, 8, 1, rule, 1500));
    }
    #[test]
    fn m6_bad_or_absent_sample_clears_old_authority_atomically() {
        let bound = bind(
            &[fire(
                "cannon",
                vec![ThinkPredicate::Order {
                    is: OrderKind::Hold,
                }],
            )],
            CoreClass::I,
        );
        let mut lease = RuleLease::default();
        let good = sample(&bound, vec![true]);
        lease
            .land(Some(good.clone()), &bound, 7, 1020, 2020)
            .unwrap();
        let mut bad = good.clone();
        bad.truth.push(true);
        assert!(lease.land(Some(bad), &bound, 7, 1020, 2020).is_err());
        assert!(lease.sample().is_none());
        lease
            .land(Some(good.clone()), &bound, 7, 1020, 2020)
            .unwrap();
        assert!(lease.land(Some(good), &bound, 8, 1020, 2020).is_err());
        assert!(lease.sample().is_none());
        lease.land(None, &bound, 8, 2020, 3020).unwrap();
        assert!(lease.sample().is_none());
    }
    #[test]
    fn m6_later_sampled_hold_vetoes_earlier_fire_and_later_fire_keeps_order() {
        let hold = AuthoredRule {
            rule: Rule::hold_rule("cannon".into(), vec![]),
            think: vec![ThinkPredicate::Hostiles { at_least: 2 }],
        };
        let bound = bind(
            &[fire("cannon", vec![]), hold, fire("mg", vec![])],
            CoreClass::II,
        );
        let mut lease = RuleLease::default();
        let selected = |lease: &RuleLease, t_ms| {
            let eligible = bound
                .rows
                .iter()
                .enumerate()
                .filter(|(i, row)| {
                    lease.permits(
                        &bound,
                        7,
                        u8::try_from(i + 1).unwrap(),
                        &row.program.rule,
                        t_ms,
                    )
                })
                .collect::<Vec<_>>();
            eligible.iter().find_map(|(i,row)|{
                let gun=row.program.rule.fire()?;
                (!eligible.iter().any(|(_,other)|matches!(&other.program.rule,Rule::HoldFire{weapon,..} if weapon==&gun.weapon))).then_some(i+1)
            })
        };
        assert_eq!(selected(&lease, 1020), Some(1));
        lease
            .land(
                Some(sample(&bound, vec![true, true, true])),
                &bound,
                7,
                1020,
                2020,
            )
            .unwrap();
        assert_eq!(selected(&lease, 1020), Some(3));
        assert_eq!(selected(&lease, 2020), Some(1));
        let bytes = serde_json::to_vec(&lease).unwrap();
        let resumed: RuleLease = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(selected(&resumed, 1500), Some(3));
    }
    #[test]
    fn m6_unknown_weapon_or_tampered_bound_program_is_refused() {
        assert!(
            BoundProgram::bind(
                &[fire("absent", vec![])],
                &["cannon".into()],
                &Channel::ALL,
                &ClassTable::STOCK.row(CoreClass::I),
                100
            )
            .is_err()
        );
        let bound = bind(&[fire("cannon", vec![])], CoreClass::I);
        let mut bad = bound.clone();
        bad.rows[0].inert = true;
        assert!(bad.verify(&bound).is_err());
    }
    #[test]
    fn m6_carried_sample_cannot_extend_its_captured_schedule() {
        let program = bind(
            &[fire(
                "cannon",
                vec![ThinkPredicate::Hostiles { at_least: 1 }],
            )],
            CoreClass::I,
        );
        let mut lease = RuleLease::default();
        lease
            .land(Some(sample(&program, vec![true])), &program, 7, 1020, 2020)
            .unwrap();
        lease.validate(&program, 7, 1500, 1000).unwrap();
        let mut value = serde_json::to_value(&lease).unwrap();
        value["expires_ms"] = serde_json::json!(9_000_000);
        let forged: RuleLease = serde_json::from_value(value).unwrap();
        assert!(forged.validate(&program, 7, 1500, 1000).is_err());
    }
}
