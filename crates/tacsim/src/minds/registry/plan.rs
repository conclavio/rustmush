//! Canonical finite case order and checked per-case reservations. A schedule
//! is made only after production fixture preparation has captured each native
//! cadence. No request can choose a roster or substitute a cheap case cost.

use crate::minds::bulk::registry_budget::{RegistryBudget, RegistryCase};
use mush_core::DefinitionDigest;
use mush_roll::commerce::rating::{
    CaseKey, CaseKind, KernelName, MAX_CASES, ProbeName, RatingClass, RatingError,
};
use serde::Serialize;

use super::subject::{PreparedSubject, rating_class};
use crate::arena::core::{ClassTable, CoreClass};

pub(super) const STARTUP_MS: u64 = 2_000;
pub(super) const TRIAL_LIMIT_MS: u64 = 600_000;
pub(super) const MAX_NATIVE_STEPS: u64 = 8;
pub(super) const PROBES: [ProbeName; 12] = [
    ProbeName::EmptyArmed,
    ProbeName::EmptyUnarmed,
    ProbeName::Hostile450Armed,
    ProbeName::Hostile450Unarmed,
    ProbeName::HurtTwoArmed,
    ProbeName::HurtTwoUnarmed,
    ProbeName::OrderLossArmed,
    ProbeName::OrderLossUnarmed,
    ProbeName::DensityArmed,
    ProbeName::DensityUnarmed,
    ProbeName::FordMobile,
    ProbeName::FordDisabled,
];
pub(super) const KERNELS: [KernelName; 8] = [
    KernelName::MemoryKeysAt,
    KernelName::MemoryKeysOver,
    KernelName::MemoryValueAt,
    KernelName::MemoryValueOver,
    KernelName::RulesSlotsAt,
    KernelName::RulesSlotsOver,
    KernelName::MeterFinalFee,
    KernelName::MeterCallback,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Physics {
    pub(super) units: u16,
    pub(super) native_ms: u32,
}
impl Physics {
    fn validate(self) -> Result<(), RatingError> {
        if self.units == 0 || self.units > 33 || self.native_ms < 50 {
            return Err(RatingError::Bound);
        }
        Ok(())
    }
}

/// Captured by the fixed fixture builder, once per class. The same object is
/// carried into case construction and checked against the resulting Arena;
/// these are not free-standing transport inputs or quota grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ClassPhysics {
    pub(super) class: CoreClass,
    pub(super) trials: [Physics; 3],
    pub(super) probes: [Physics; 12],
    pub(super) cycle: Physics,
    pub(super) kernel: Physics,
    pub(super) loss_expiry_ms: u64,
}

impl ClassPhysics {
    /// The closed real fixture constructors supply every roster/cadence. This
    /// runs only on the preparation worker, never on the world pulse.
    fn capture(
        policy: &super::manifest::FrozenPolicy,
        class: CoreClass,
    ) -> Result<Self, RatingError> {
        use super::{
            ford::Ford, kernels::Boundary, order_loss::Loss, probes::Single, profiles::Cycle,
            trials::Trial,
        };
        use mush_roll::commerce::rating::TrialLayout;
        let mut probes = Vec::with_capacity(PROBES.len());
        for name in PROBES {
            probes.push(match name {
                ProbeName::OrderLossArmed => Loss::physics(policy, class, true)?,
                ProbeName::OrderLossUnarmed => Loss::physics(policy, class, false)?,
                ProbeName::FordMobile | ProbeName::FordDisabled => Ford::physics(policy, class)?,
                _ => Single::physics(policy, name, class)?,
            });
        }
        Ok(Self {
            class,
            trials: [
                Trial::physics(policy, TrialLayout::Duel, class)?,
                Trial::physics(policy, TrialLayout::Muster, class)?,
                Trial::physics(policy, TrialLayout::Hold, class)?,
            ],
            probes: probes.try_into().map_err(|_| RatingError::Incomplete)?,
            cycle: Cycle::physics(policy, class)?,
            kernel: Boundary::physics(policy, class)?,
            loss_expiry_ms: Loss::expiry_budget(policy)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Case {
    pub(super) key: CaseKey,
    pub(super) physics: Physics,
    pub(super) maximum_ms: u64,
    pub(super) maximum_thinks: u64,
}
impl Case {
    pub(super) fn maximum_unit_ms(&self) -> Result<u64, RatingError> {
        self.maximum_ms
            .checked_add(u64::from(self.physics.native_ms))
            .and_then(|ms| ms.checked_mul(u64::from(self.physics.units)))
            .ok_or(RatingError::Bound)
    }
    pub(super) fn quantum_ms(&self) -> u64 {
        u64::from(self.physics.native_ms) * MAX_NATIVE_STEPS
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Schedule {
    cases: Vec<Case>,
    maximum_unit_ms: u64,
}
impl Schedule {
    pub(super) fn capture(
        subject: &PreparedSubject,
        policy: &super::manifest::FrozenPolicy,
    ) -> Result<Self, RatingError> {
        let mut physics = Vec::new();
        for class in [CoreClass::I, CoreClass::II, CoreClass::III] {
            if class.ordinal() >= subject.subject().declared().ordinal() {
                physics.push(ClassPhysics::capture(policy, class)?);
            }
        }
        Self::prepare(subject, &policy.cfg.minds.classes, &physics)
    }
    pub(super) fn prepare(
        subject: &PreparedSubject,
        classes: &ClassTable,
        physics: &[ClassPhysics],
    ) -> Result<Self, RatingError> {
        let start = subject.subject().declared();
        let expected = [CoreClass::I, CoreClass::II, CoreClass::III]
            .into_iter()
            .filter(|class| class.ordinal() >= start.ordinal())
            .collect::<Vec<_>>();
        if physics.len() != expected.len() {
            return Err(RatingError::Incomplete);
        }
        let mut cases = Vec::new();
        for (captured, class) in physics.iter().zip(expected) {
            if captured.class != class {
                return Err(RatingError::Identity);
            }
            let rating = rating_class(class)?;
            let profile = classes.row(class);
            let think_ms = u64::try_from(profile.think_ms).map_err(|_| RatingError::Bound)?;
            if !(250..=5_000).contains(&think_ms) || profile.rows > 32 {
                return Err(RatingError::Bound);
            }
            for index in 0..subject.variants().len() {
                let layouts = [
                    mush_roll::commerce::rating::TrialLayout::Duel,
                    mush_roll::commerce::rating::TrialLayout::Muster,
                    mush_roll::commerce::rating::TrialLayout::Hold,
                ];
                for (layout, physical) in layouts.into_iter().zip(captured.trials) {
                    let units = if layout == mush_roll::commerce::rating::TrialLayout::Duel {
                        2
                    } else {
                        4
                    };
                    if physical.units != units {
                        return Err(RatingError::Identity);
                    }
                    for seed in [1, 2] {
                        for swapped in [false, true] {
                            let maximum_thinks = TRIAL_LIMIT_MS
                                .div_ceil(think_ms)
                                .checked_add(2)
                                .and_then(|n| n.checked_mul(u64::from(units / 2)))
                                .ok_or(RatingError::Bound)?;
                            push(
                                &mut cases,
                                rating,
                                CaseKind::Trial {
                                    variant: u8::try_from(index).map_err(|_| RatingError::Bound)?,
                                    layout,
                                    seed,
                                    swapped,
                                },
                                physical,
                                TRIAL_LIMIT_MS,
                                maximum_thinks,
                            )?;
                        }
                    }
                }
            }
            for index in 0..subject.pairs().len() {
                for (name, physical) in PROBES.into_iter().zip(captured.probes) {
                    let units = match name {
                        ProbeName::EmptyArmed
                        | ProbeName::EmptyUnarmed
                        | ProbeName::FordMobile
                        | ProbeName::FordDisabled => 1,
                        ProbeName::Hostile450Armed | ProbeName::Hostile450Unarmed => 2,
                        ProbeName::HurtTwoArmed
                        | ProbeName::HurtTwoUnarmed
                        | ProbeName::OrderLossArmed
                        | ProbeName::OrderLossUnarmed => 3,
                        ProbeName::DensityArmed | ProbeName::DensityUnarmed => {
                            u16::try_from(profile.rows + 1).map_err(|_| RatingError::Bound)?
                        }
                    };
                    if physical.units != units {
                        return Err(RatingError::Identity);
                    }
                    let thinks = probe_thinks(name);
                    let mut duration = probe_duration(thinks, think_ms, physical.native_ms)?;
                    if matches!(
                        name,
                        ProbeName::OrderLossArmed | ProbeName::OrderLossUnarmed
                    ) {
                        duration = duration
                            .checked_add(captured.loss_expiry_ms)
                            .ok_or(RatingError::Bound)?;
                        // Arena match deadlines are u32; reject admission of a
                        // case that cannot preserve its exact captured TTL.
                        u32::try_from(
                            duration
                                .checked_add(u64::from(physical.native_ms))
                                .ok_or(RatingError::Bound)?,
                        )
                        .map_err(|_| RatingError::Bound)?;
                    }
                    push(
                        &mut cases,
                        rating,
                        CaseKind::Probe {
                            pair: u8::try_from(index).map_err(|_| RatingError::Bound)?,
                            name,
                        },
                        physical,
                        duration,
                        thinks,
                    )?;
                }
            }
            if captured.cycle.units != 1 || captured.kernel.units != 1 {
                return Err(RatingError::Identity);
            }
            push(
                &mut cases,
                rating,
                CaseKind::ProfileCycle {},
                captured.cycle,
                probe_duration(256, think_ms, captured.cycle.native_ms)?,
                256,
            )?;
            for name in KERNELS {
                push(
                    &mut cases,
                    rating,
                    CaseKind::Kernel { name },
                    captured.kernel,
                    probe_duration(1, think_ms, captured.kernel.native_ms)?,
                    1,
                )?;
            }
        }
        if cases.is_empty()
            || cases.len() > MAX_CASES
            || cases
                .windows(2)
                .any(|pair| matches!(pair, [a,b] if a.key >= b.key))
        {
            return Err(RatingError::Bound);
        }
        let maximum_unit_ms = cases.iter().try_fold(0_u64, |sum, case| {
            sum.checked_add(case.maximum_unit_ms()?)
                .ok_or(RatingError::Bound)
        })?;
        Ok(Self {
            cases,
            maximum_unit_ms,
        })
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, RatingError> {
        super::identity("tacsim.registry.schedule.v1", self, 1024 * 1024)
    }
    pub(super) fn budget(
        &self,
        subject: &PreparedSubject,
        compatibility: DefinitionDigest,
    ) -> Result<RegistryBudget, RatingError> {
        RegistryBudget::prepare(
            subject.subject().digest()?,
            compatibility,
            self.digest()?,
            self.cases
                .iter()
                .map(|case| RegistryCase {
                    key: case.key.clone(),
                    units: case.physics.units,
                    native_ms: case.physics.native_ms,
                    maximum_ms: case.maximum_ms,
                })
                .collect(),
        )
        .map_err(|_| RatingError::Invalid)
    }
    pub(super) fn cases(&self) -> &[Case] {
        &self.cases
    }
    pub(super) fn maximum_unit_ms(&self) -> u64 {
        self.maximum_unit_ms
    }
}

fn push(
    cases: &mut Vec<Case>,
    class: RatingClass,
    kind: CaseKind,
    physics: Physics,
    maximum_ms: u64,
    maximum_thinks: u64,
) -> Result<(), RatingError> {
    physics.validate()?;
    if cases.len() == MAX_CASES
        || maximum_ms == 0
        || maximum_thinks == 0
        || maximum_thinks > 1_000_000
    {
        return Err(RatingError::Bound);
    }
    cases.push(Case {
        key: CaseKey::new(class, kind)?,
        physics,
        maximum_ms,
        maximum_thinks,
    });
    Ok(())
}
fn probe_thinks(name: ProbeName) -> u64 {
    match name {
        ProbeName::EmptyArmed
        | ProbeName::EmptyUnarmed
        | ProbeName::Hostile450Armed
        | ProbeName::Hostile450Unarmed
        | ProbeName::HurtTwoArmed
        | ProbeName::HurtTwoUnarmed => 1,
        ProbeName::OrderLossArmed | ProbeName::OrderLossUnarmed => 64,
        ProbeName::DensityArmed | ProbeName::DensityUnarmed => 16,
        ProbeName::FordMobile | ProbeName::FordDisabled => 256,
    }
}
fn probe_duration(thinks: u64, think_ms: u64, native_ms: u32) -> Result<u64, RatingError> {
    thinks
        .checked_add(2)
        .and_then(|n| n.checked_mul(think_ms))
        .and_then(|n| n.checked_add(STARTUP_MS))
        .and_then(|n| n.checked_add(2 * u64::from(native_ms)))
        .ok_or(RatingError::Bound)
}

#[cfg(test)]
mod captured_tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn actual_schedule_reserves_captured_expiry_without_altering_native_time() -> TestResult {
        let mut policy = super::super::tests::frozen_policy()?;
        policy.cfg.sensors.pass_ms = 10_000;
        policy.cfg.sensors.lost_after_passes = 20;
        let source = "fn think(v,c) { stock::rules(); }";
        let subject = PreparedSubject::prepare(
            &super::super::tests::revision(source)?,
            source,
            &policy.cfg.arena,
        )?;
        let schedule = Schedule::capture(&subject, &policy)?;
        let mut losses = 0;
        for case in schedule.cases() {
            if matches!(
                case.key.kind(),
                CaseKind::Probe {
                    name: ProbeName::OrderLossArmed | ProbeName::OrderLossUnarmed,
                    ..
                }
            ) {
                let class = match case.key.class() {
                    RatingClass::I => CoreClass::I,
                    RatingClass::II => CoreClass::II,
                    RatingClass::III => CoreClass::III,
                };
                let think_ms = u64::try_from(policy.cfg.minds.classes.row(class).think_ms)?;
                assert_eq!(case.physics.units, 3);
                assert_eq!(case.maximum_thinks, 64);
                assert_eq!(
                    case.maximum_ms,
                    probe_duration(64, think_ms, case.physics.native_ms)? + 210_000
                );
                losses += 1;
            }
        }
        assert_eq!(losses, 3 * subject.pairs().len() * 2);
        assert_eq!(policy.cfg.sensors.pass_ms, 10_000);
        assert_eq!(policy.cfg.sensors.lost_after_passes, 20);
        assert_eq!(
            schedule.maximum_unit_ms(),
            schedule.cases().iter().try_fold(0_u64, |sum, case| sum
                .checked_add(case.maximum_unit_ms()?)
                .ok_or(RatingError::Bound))?
        );
        Ok(())
    }
}
