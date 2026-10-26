//! Closed per-case billing descriptors for the Registry worker. This is quota
//! bookkeeping, not certification authority. The frozen Registry producer must
//! independently bind the exact case manifest/source/compatibility artifact.

use mush_core::DefinitionDigest;
use mush_roll::commerce::rating::{CaseKey, CaseKind, MAX_CASES};
use serde::{Deserialize, Serialize};

use super::{BulkError, invalid};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryCase {
    pub key: CaseKey,
    pub units: u16,
    pub native_ms: u32,
    pub maximum_ms: u64,
}
impl RegistryCase {
    pub fn limit_ms(&self) -> Result<u64, BulkError> {
        self.maximum_ms
            .checked_add(u64::from(self.native_ms))
            .ok_or_else(|| invalid("Registry duration overflow"))
    }
    pub fn maximum_unit_ms(&self) -> Result<u64, BulkError> {
        self.limit_ms()?
            .checked_mul(u64::from(self.units))
            .ok_or_else(|| invalid("Registry cost overflow"))
    }
    fn validate(&self) -> Result<(), BulkError> {
        // Limits derive from the captured real class periods/native cadence.
        // A lawful larger cadence is not silently clamped to250ms.
        if !(1..=33).contains(&self.units) || self.native_ms < 50 || self.maximum_ms == 0 {
            return Err(invalid("Registry case dimensions"));
        }
        self.maximum_unit_ms()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryBudget {
    pub subject: DefinitionDigest,
    pub compatibility: DefinitionDigest,
    pub schedule: DefinitionDigest,
    cases: Vec<RegistryCase>,
    maximum_unit_ms: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub measured_rejection: bool,
    #[serde(
        default,
        deserialize_with = "present_reconformance",
        skip_serializing_if = "Option::is_none"
    )]
    pub reconformance: Option<mush_roll::commerce::rating::reconformance::ReconformanceAdmission>,
}
fn present_reconformance<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<mush_roll::commerce::rating::reconformance::ReconformanceAdmission>, D::Error> {
    mush_roll::commerce::rating::reconformance::ReconformanceAdmission::deserialize(d).map(Some)
}
fn is_false(value: &bool) -> bool {
    !value
}
impl RegistryBudget {
    pub fn prepare(
        subject: DefinitionDigest,
        compatibility: DefinitionDigest,
        schedule: DefinitionDigest,
        cases: Vec<RegistryCase>,
    ) -> Result<Self, BulkError> {
        let maximum_unit_ms = cases.iter().try_fold(0_u64, |sum, case| {
            sum.checked_add(case.maximum_unit_ms()?)
                .ok_or_else(|| invalid("Registry total cost overflow"))
        })?;
        let value = Self {
            subject,
            compatibility,
            schedule,
            cases,
            maximum_unit_ms,
            measured_rejection: true,
            reconformance: None,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), BulkError> {
        if self.reconformance.is_some() && !self.measured_rejection {
            return Err(invalid(
                "reconformance requires measured rejection reservation",
            ));
        }
        if self.cases.is_empty()
            || self.cases.len() > MAX_CASES
            || self
                .cases
                .windows(2)
                .any(|pair| matches!(pair,[a,b] if a.key >= b.key))
        {
            return Err(invalid("Registry case order/count"));
        }
        let mut total = 0_u64;
        for case in &self.cases {
            case.validate()?;
            total = total
                .checked_add(case.maximum_unit_ms()?)
                .ok_or_else(|| invalid("Registry total cost overflow"))?;
        }
        if total != self.maximum_unit_ms {
            return Err(invalid("Registry total reservation mismatch"));
        }
        Ok(())
    }
    pub(super) fn case_count(&self) -> Result<u16, BulkError> {
        self.validate()?;
        u16::try_from(self.cases.len()).map_err(|_| invalid("Registry case count"))
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, BulkError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| invalid("Registry budget encoding"))?;
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.registry.budget.v1",
            &value,
            super::model::MAX_ACCOUNT_DOCUMENT,
        )
        .map_err(|_| invalid("Registry budget encoding"))
    }
    pub fn maximum_unit_ms(&self) -> u64 {
        self.maximum_unit_ms
    }
    pub fn case(&self, cursor: RegistryCursor) -> Result<&RegistryCase, BulkError> {
        let case = self
            .cases
            .get(usize::from(cursor.index))
            .ok_or_else(|| invalid("Registry case cursor"))?;
        if cursor.simulated_ms > case.limit_ms()? {
            return Err(invalid("Registry case duration"));
        }
        Ok(case)
    }
    pub fn can_certify(&self, cursor: RegistryCursor) -> Result<bool, BulkError> {
        let case = self.case(cursor)?;
        Ok(self
            .cases
            .get(usize::from(cursor.index) + 1)
            .is_none_or(|next| next.key.class() != case.key.class()))
    }
    /// Quota completion may retain a measured maximum-class rejection before
    /// the rest of that class runs. This is not permission to issue a certificate:
    /// the registered artifact and Roll owners verify the actual rejection.
    pub fn can_reject(&self, cursor: RegistryCursor) -> Result<bool, BulkError> {
        let case = self.case(cursor)?;
        Ok(self.measured_rejection
            && case.key.class() == mush_roll::commerce::rating::RatingClass::III
            && !matches!(case.key.kind(), CaseKind::Kernel { .. })
            && !self
                .cases
                .iter()
                .skip(usize::from(cursor.index) + 1)
                .any(|next| next.key.class() > case.key.class()))
    }
    /// Computes the sole allowed post-cursor, before refund or owner mutation.
    /// A promotion skips only the current class's remaining cases; measured
    /// budget-fault evidence remains required by the closed result producer.
    pub fn advance(
        &self,
        from: RegistryCursor,
        actual_sim_ms: u64,
        ending: RegistryEnding,
    ) -> Result<RegistryCursor, BulkError> {
        let case = self.case(from)?;
        let simulated_ms = from
            .simulated_ms
            .checked_add(actual_sim_ms)
            .ok_or_else(|| invalid("Registry progress overflow"))?;
        if simulated_ms > case.limit_ms()? {
            return Err(invalid("Registry case exceeded admitted duration"));
        }
        let next_index = usize::from(from.index)
            .checked_add(1)
            .ok_or_else(|| invalid("Registry case index overflow"))?;
        let next = self.cases.get(next_index);
        match ending {
            RegistryEnding::Continue => Ok(RegistryCursor {
                simulated_ms,
                ..from
            }),
            RegistryEnding::CaseComplete => {
                if next.is_none_or(|next| next.key.class() != case.key.class()) {
                    return Err(invalid(
                        "Registry class completion requires certification or measured promotion",
                    ));
                }
                Ok(RegistryCursor {
                    index: u16::try_from(next_index)
                        .map_err(|_| invalid("Registry cursor overflow"))?,
                    simulated_ms: 0,
                })
            }
            RegistryEnding::ClassBudget => {
                if matches!(case.key.kind(), CaseKind::Kernel { .. }) {
                    return Err(invalid(
                        "Registry boundary probe cannot promote a submitted source",
                    ));
                }
                let index = self
                    .cases
                    .iter()
                    .enumerate()
                    .skip(next_index)
                    .find(|(_, next)| next.key.class() > case.key.class())
                    .map(|(i, _)| i)
                    .ok_or_else(|| invalid("Registry has no higher admitted class"))?;
                Ok(RegistryCursor {
                    index: u16::try_from(index).map_err(|_| invalid("Registry cursor overflow"))?,
                    simulated_ms: 0,
                })
            }
            RegistryEnding::Rejected => {
                if !self.can_reject(from)? {
                    return Err(invalid("Registry has no admitted maximum-class rejection"));
                }
                Ok(RegistryCursor {
                    simulated_ms,
                    ..from
                })
            }
            RegistryEnding::Certified => {
                if next.is_some_and(|next| next.key.class() == case.key.class()) {
                    return Err(invalid("Registry class coverage is not complete"));
                }
                Ok(RegistryCursor {
                    simulated_ms,
                    ..from
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryCursor {
    pub index: u16,
    pub simulated_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegistryEnding {
    Continue,
    CaseComplete,
    ClassBudget,
    Certified,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegistryProgress {
    pub cursor: RegistryCursor,
    pub ending: RegistryEnding,
}
