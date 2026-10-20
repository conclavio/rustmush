//! Bounded paired execution. Only complete identical repetitions can produce
//! worker evidence; this evidence still needs actual acknowledged publication.
use super::super::{identity, subject::PreparedSubject};
use super::{
    Case, Role,
    manifest::Policy,
    trial::{ResultRow, Trial},
};
use mush_core::DefinitionDigest;
use mush_roll::commerce::rating::{RatingError, RatingSubject};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAX_CASES: usize = 30;
const RESULT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Key {
    pub(super) case: Case,
    pub(super) seed: u64,
}
impl Key {
    pub(super) fn schedule(role: Role) -> Vec<Self> {
        Case::for_role(role)
            .iter()
            .flat_map(|case| [2350, 2351, 2352, 2353, 2354].map(|seed| Self { case: *case, seed }))
            .collect()
    }
}

/// No decoder or public raw-result constructor. A worker may produce a failed
/// run, but its output cannot be turned into qualifying evidence by a caller.
#[derive(Debug, Serialize)]
pub(super) struct CompletedRun {
    format: u16,
    policy: DefinitionDigest,
    subject: RatingSubject,
    profile: String,
    role: Role,
    keys: Vec<Key>,
    results: Vec<ResultRow>,
    paired_repetitions: u8,
    charged_unit_ms: u64,
    passed: bool,
    digest: DefinitionDigest,
}
impl CompletedRun {
    pub(super) fn measurement(
        &self,
    ) -> Result<mush_roll::commerce::conduct::records::ConductMeasurement, RatingError> {
        mush_roll::commerce::conduct::records::ConductMeasurement::observed(
            self.digest,
            self.policy,
            identity(
                "tacsim.conduct.subject.v1",
                &(&self.subject, &self.profile, self.role),
                4096,
            )?,
            self.charged_unit_ms,
            self.passed,
        )
        .map_err(|_| RatingError::Identity)
    }
    #[cfg(test)]
    pub(super) fn first_failure_summary(&self) -> String {
        self.results
            .iter()
            .find(|row| !row.passed())
            .map(ResultRow::failure_summary)
            .unwrap_or_else(|| "no failed row".into())
    }
    pub(super) fn passed(&self) -> bool {
        self.passed
    }
    pub(super) fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub(super) fn measured_unit_ms(&self) -> u64 {
        self.charged_unit_ms
    }
}

pub(super) struct Progress {
    pub(super) charged_unit_ms: u64,
    pub(super) completed: Option<CompletedRun>,
}
pub(super) struct Run {
    policy: Arc<Policy>,
    subject: Arc<PreparedSubject>,
    profile: String,
    role: Role,
    keys: Vec<Key>,
    index: usize,
    first: Option<ResultRow>,
    results: Vec<ResultRow>,
    trial: Option<Trial>,
    charged: u64,
    finished: bool,
}
impl Run {
    pub(super) fn start(
        &self,
        subject: &mush_roll::commerce::conduct::ConductSubject,
    ) -> Result<super::history::Start, RatingError> {
        if self.index != 0
            || self.first.is_some()
            || self.charged != 0
            || self.finished
            || subject.worker() != self.subject.subject()
            || subject.profile() != self.profile
            || serde_json::to_value(subject.role()).map_err(|_| RatingError::Invalid)?
                != serde_json::to_value(self.role).map_err(|_| RatingError::Invalid)?
            || self.keys != Key::schedule(self.role)
        {
            return Err(RatingError::Identity);
        }
        let openings = self
            .keys
            .iter()
            .map(|key| {
                Trial::open(
                    &self.policy,
                    &self.subject,
                    &self.profile,
                    self.role,
                    key.case,
                    key.seed,
                )?
                .opening_record()
            })
            .collect::<Result<Vec<_>, RatingError>>()?;
        super::history::Start::captured(
            self.policy.retained(),
            subject.clone(),
            self.billing_budget()?,
            openings,
        )
    }
    pub(super) fn billing_case(
        &self,
    ) -> Result<crate::minds::bulk::conduct_budget::ConductCase, RatingError> {
        let trial = self.trial.as_ref().ok_or(RatingError::Incomplete)?;
        let index = self
            .index
            .checked_mul(2)
            .and_then(|n| n.checked_add(usize::from(self.first.is_some())))
            .ok_or(RatingError::Bound)?;
        Ok(crate::minds::bulk::conduct_budget::ConductCase {
            index: u8::try_from(index).map_err(|_| RatingError::Bound)?,
            units: u16::try_from(trial.units()).map_err(|_| RatingError::Bound)?,
            native_ms: trial.native_ms(),
            maximum_ms: u64::from(self.policy.maximum_ms()),
        })
    }
    /// Construct the admitted rates from each actual opening Arena. No time is
    /// advanced here, and a scenario label never substitutes for its roster.
    pub(super) fn billing_budget(
        &self,
    ) -> Result<crate::minds::bulk::conduct_budget::ConductBudget, RatingError> {
        use crate::minds::bulk::conduct_budget::{ConductBudget, ConductCase};
        if self.index != 0 || self.first.is_some() || self.charged != 0 || self.finished {
            return Err(RatingError::Identity);
        }
        let mut cases = Vec::new();
        for (index, key) in self.keys.iter().enumerate() {
            let trial = Trial::open(
                &self.policy,
                &self.subject,
                &self.profile,
                self.role,
                key.case,
                key.seed,
            )?;
            for repetition in 0..2usize {
                cases.push(ConductCase {
                    index: u8::try_from(
                        index
                            .checked_mul(2)
                            .and_then(|i| i.checked_add(repetition))
                            .ok_or(RatingError::Bound)?,
                    )
                    .map_err(|_| RatingError::Bound)?,
                    units: u16::try_from(trial.units()).map_err(|_| RatingError::Bound)?,
                    native_ms: trial.native_ms(),
                    maximum_ms: u64::from(self.policy.maximum_ms()),
                });
            }
        }
        ConductBudget::prepare(
            identity(
                "tacsim.conduct.subject.v1",
                &(self.subject.subject(), &self.profile, self.role),
                32 * 1024,
            )?,
            self.policy.digest(),
            identity("tacsim.conduct.schedule.v1", &(&self.keys, 2u8), 16 * 1024)?,
            cases,
        )
        .map_err(|_| RatingError::Bound)
    }
    pub(super) fn new(
        policy: Arc<Policy>,
        subject: Arc<PreparedSubject>,
        profile: String,
        role: Role,
    ) -> Result<Self, RatingError> {
        let keys = Case::for_role(role)
            .iter()
            .flat_map(|case| {
                policy.seeds().iter().map(move |seed| Key {
                    case: *case,
                    seed: *seed,
                })
            })
            .collect::<Vec<_>>();
        if keys.is_empty() || keys.len() > MAX_CASES {
            return Err(RatingError::Bound);
        }
        let first = keys.first().ok_or(RatingError::Incomplete)?;
        let trial = Trial::open(&policy, &subject, &profile, role, first.case, first.seed)?;
        Ok(Self {
            policy,
            subject,
            profile,
            role,
            keys,
            index: 0,
            first: None,
            results: Vec::new(),
            trial: Some(trial),
            charged: 0,
            finished: false,
        })
    }
    /// Actual owner scheduling must obtain this much acknowledged unit-time
    /// before calling advance. A smaller lease makes no simulation progress.
    pub(super) fn next_unit_ms(&self) -> Result<u64, RatingError> {
        if self.finished {
            return Err(RatingError::Incomplete);
        }
        let trial = self.trial.as_ref().ok_or(RatingError::Incomplete)?;
        u64::from(trial.native_ms())
            .checked_mul(u64::try_from(trial.units()).map_err(|_| RatingError::Bound)?)
            .ok_or(RatingError::Bound)
    }
    #[cfg(test)]
    pub(super) fn maximum_steps(&self) -> Result<u64, RatingError> {
        self.trial
            .as_ref()
            .ok_or(RatingError::Incomplete)?
            .maximum_steps()
            .checked_mul(u64::try_from(self.keys.len()).map_err(|_| RatingError::Bound)?)
            .and_then(|n| n.checked_mul(2))
            .ok_or(RatingError::Bound)
    }
    pub(super) fn advance(&mut self, available_unit_ms: u64) -> Result<Progress, RatingError> {
        if self.finished {
            return Err(RatingError::Incomplete);
        }
        let required = self.next_unit_ms()?;
        if available_unit_ms < required {
            return Err(RatingError::Bound);
        }
        let result = self.advance_prepaid(required);
        if result.is_err() {
            // A worker error may follow actual simulation or measurement. No
            // caller can retry the partially advanced machine as a fresh step.
            self.finished = true;
            self.trial = None;
        }
        result
    }
    fn advance_prepaid(&mut self, required: u64) -> Result<Progress, RatingError> {
        let trial = self.trial.as_mut().ok_or(RatingError::Incomplete)?;
        let units = u64::try_from(trial.units()).map_err(|_| RatingError::Bound)?;
        let slice = trial.advance(u64::from(trial.native_ms()))?;
        let charge = slice
            .simulated_ms
            .checked_mul(units)
            .ok_or(RatingError::Bound)?;
        if charge > required {
            return Err(RatingError::Bound);
        }
        self.charged = self.charged.checked_add(charge).ok_or(RatingError::Bound)?;
        let Some(result) = slice.result else {
            return Ok(Progress {
                charged_unit_ms: charge,
                completed: None,
            });
        };
        if let Some(first) = self.first.take() {
            // Compare the whole actual evidence, including all measurements,
            // opening/end states and native ledger, not only a success flag.
            if first != result {
                self.finished = true;
                return Err(RatingError::Identity);
            }
            self.results.push(result);
            self.index = self.index.checked_add(1).ok_or(RatingError::Bound)?;
        } else {
            self.first = Some(result);
        }
        if let Some(key) = self.keys.get(self.index) {
            self.trial = Some(Trial::open(
                &self.policy,
                &self.subject,
                &self.profile,
                self.role,
                key.case,
                key.seed,
            )?);
            return Ok(Progress {
                charged_unit_ms: charge,
                completed: None,
            });
        }
        if self.first.is_some() || self.results.len() != self.keys.len() {
            return Err(RatingError::Incomplete);
        }
        self.finished = true;
        self.trial = None;
        let subject = self.subject.subject().clone();
        if self
            .results
            .iter()
            .any(|row| row.binding().source() != subject.revision().source())
        {
            return Err(RatingError::Identity);
        }
        let digest = identity(
            "tacsim.conduct.completed.v1",
            &(
                1u16,
                self.policy.digest(),
                &subject,
                &self.profile,
                self.role,
                &self.keys,
                &self.results,
                2u8,
                self.charged,
            ),
            RESULT_BYTES,
        )?;
        let passed = self.results.iter().all(ResultRow::passed);
        Ok(Progress {
            charged_unit_ms: charge,
            completed: Some(CompletedRun {
                format: 1,
                policy: self.policy.digest(),
                subject,
                profile: self.profile.clone(),
                role: self.role,
                keys: self.keys.clone(),
                results: std::mem::take(&mut self.results),
                paired_repetitions: 2,
                charged_unit_ms: self.charged,
                passed,
                digest,
            }),
        })
    }
}
