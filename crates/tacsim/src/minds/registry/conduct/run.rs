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
use serde::Serialize;
use std::sync::Arc;

const MAX_CASES: usize = 30;
const RESULT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Key {
    case: Case,
    seed: u64,
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
