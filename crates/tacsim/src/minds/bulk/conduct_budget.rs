//! Exact per-trial quota accounting for the private Conduct worker. These
//! structural values are not qualification or permission to publish a result.
use super::{BulkError, invalid};
use mush_core::DefinitionDigest;
use serde::{Deserialize, Serialize};

pub(crate) const TRIALS: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConductCase {
    /// Ordered case/seed/repetition index in the captured worker schedule.
    pub index: u8,
    pub units: u16,
    pub native_ms: u32,
    pub maximum_ms: u64,
}
impl ConductCase {
    pub fn limit_ms(&self) -> Result<u64, BulkError> {
        self.maximum_ms
            .checked_add(u64::from(self.native_ms))
            .ok_or_else(|| invalid("Conduct duration overflow"))
    }
    fn validate(&self) -> Result<(), BulkError> {
        if usize::from(self.index) >= TRIALS
            || !(3..=4).contains(&self.units)
            || !(50..=1_000).contains(&self.native_ms)
            || self.maximum_ms != 120_000
        {
            return Err(invalid("Conduct trial dimensions"));
        }
        self.limit_ms()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConductBudget {
    pub subject: DefinitionDigest,
    pub policy: DefinitionDigest,
    pub schedule: DefinitionDigest,
    cases: Vec<ConductCase>,
    maximum_unit_ms: u64,
}
impl ConductBudget {
    pub fn prepare(
        subject: DefinitionDigest,
        policy: DefinitionDigest,
        schedule: DefinitionDigest,
        cases: Vec<ConductCase>,
    ) -> Result<Self, BulkError> {
        let maximum_unit_ms = Self::total(&cases)?;
        let value = Self {
            subject,
            policy,
            schedule,
            cases,
            maximum_unit_ms,
        };
        value.validate()?;
        Ok(value)
    }
    fn total(cases: &[ConductCase]) -> Result<u64, BulkError> {
        cases.iter().try_fold(0u64, |total, case| {
            case.validate()?;
            total
                .checked_add(
                    case.limit_ms()?
                        .checked_mul(u64::from(case.units))
                        .ok_or_else(|| invalid("Conduct trial cost overflow"))?,
                )
                .ok_or_else(|| invalid("Conduct total cost overflow"))
        })
    }
    pub fn validate(&self) -> Result<(), BulkError> {
        if self.cases.len() != TRIALS
            || self
                .cases
                .iter()
                .enumerate()
                .any(|(index, case)| usize::from(case.index) != index)
            || self.cases.as_chunks::<2>().0.iter().any(|pair| match pair {
                [a, b] => {
                    a.units != b.units || a.native_ms != b.native_ms || a.maximum_ms != b.maximum_ms
                }
                _ => true,
            })
            || Self::total(&self.cases)? != self.maximum_unit_ms
        {
            return Err(invalid("Conduct paired schedule or total"));
        }
        Ok(())
    }
    pub fn maximum_unit_ms(&self) -> u64 {
        self.maximum_unit_ms
    }
    pub fn case(&self, cursor: ConductCursor) -> Result<&ConductCase, BulkError> {
        let case = self
            .cases
            .get(usize::from(cursor.index))
            .ok_or_else(|| invalid("Conduct trial cursor"))?;
        if cursor.simulated_ms > case.limit_ms()? {
            return Err(invalid("Conduct trial duration"));
        }
        Ok(case)
    }
    pub fn complete(&self, cursor: ConductCursor) -> Result<bool, BulkError> {
        let case = self.case(cursor)?;
        Ok(usize::from(cursor.index) + 1 == self.cases.len()
            && cursor.simulated_ms >= case.maximum_ms)
    }
    /// A worker cannot skip a trial or charge the next roster on this lease.
    /// A complete prefix is insufficient; only the last actual native timeout
    /// can finish this quota profile. Qualification is independently published.
    pub fn advance(
        &self,
        from: ConductCursor,
        elapsed: u64,
        ending: ConductEnding,
    ) -> Result<ConductCursor, BulkError> {
        let case = self.case(from)?;
        let simulated_ms = from
            .simulated_ms
            .checked_add(elapsed)
            .ok_or_else(|| invalid("Conduct progress overflow"))?;
        if simulated_ms > case.limit_ms()? || !elapsed.is_multiple_of(u64::from(case.native_ms)) {
            return Err(invalid("Conduct native progress"));
        }
        let terminal = usize::from(from.index) + 1 == self.cases.len();
        match ending {
            ConductEnding::Continue => Ok(ConductCursor {
                simulated_ms,
                ..from
            }),
            ConductEnding::CaseComplete if !terminal && simulated_ms >= case.maximum_ms => {
                Ok(ConductCursor {
                    index: from
                        .index
                        .checked_add(1)
                        .ok_or_else(|| invalid("Conduct trial index"))?,
                    simulated_ms: 0,
                })
            }
            ConductEnding::Complete if terminal && simulated_ms >= case.maximum_ms => {
                Ok(ConductCursor {
                    simulated_ms,
                    ..from
                })
            }
            _ => Err(invalid("Conduct incomplete or skipped trial")),
        }
    }
    pub fn digest(&self) -> Result<DefinitionDigest, BulkError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.conduct.budget.v1",
            &serde_json::to_value(self).map_err(|_| invalid("Conduct budget encoding"))?,
            16 * 1024,
        )
        .map_err(|_| invalid("Conduct budget bound"))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConductCursor {
    pub index: u8,
    pub simulated_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConductEnding {
    Continue,
    CaseComplete,
    Complete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConductProgress {
    pub cursor: ConductCursor,
    pub ending: ConductEnding,
}
