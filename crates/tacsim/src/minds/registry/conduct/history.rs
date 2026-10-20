//! Complete retained worker data validation. This module cannot create a live
//! worker result or qualification capability. Native index, quota, typed rows
//! and permanent source roots must independently agree before any owner read.
use super::{
    super::identity,
    Role,
    manifest::RetainedPolicy,
    run::Key,
    trial::{CaseOpening, ResultRow},
};
use crate::minds::bulk::{
    conduct_budget::{ConductBudget, ConductCursor},
    model::{ArtifactBytes, Plan, WorkKind},
};
use mush_core::DefinitionDigest;
use mush_roll::commerce::{
    conduct::{
        ConductSubject,
        records::{ConductJob, ConductMeasurement},
    },
    rating::{RatingError, RatingSubject},
};
use serde::{Deserialize, Serialize};

const START_BYTES: usize = 256 * 1024;
const RESULT_BYTES: usize = 256 * 1024;

fn role(subject: &ConductSubject) -> Role {
    match subject.role() {
        mush_roll::commerce::conduct::ConductRole::Escort => Role::Escort,
        mush_roll::commerce::conduct::ConductRole::Transit => Role::Transit,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    format: u16,
    policy: RetainedPolicy,
    subject: ConductSubject,
    budget: ConductBudget,
    openings: Vec<CaseOpening>,
}
impl Start {
    pub(super) fn captured(
        policy: RetainedPolicy,
        subject: ConductSubject,
        budget: ConductBudget,
        openings: Vec<CaseOpening>,
    ) -> Result<Self, RatingError> {
        let value = Self {
            format: 1,
            policy,
            subject,
            budget,
            openings,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.subject.validate().map_err(|_| RatingError::Identity)?;
        self.budget.validate().map_err(|_| RatingError::Identity)?;
        self.policy.validate(self.budget.policy)?;
        let keys = Key::schedule(role(&self.subject));
        if self.format != 1
            || self.openings.len() != keys.len()
            || self.openings.len() != 30
            || self.budget.subject
                != self
                    .subject
                    .worker_digest()
                    .map_err(|_| RatingError::Identity)?
            || self.budget.schedule
                != identity("tacsim.conduct.schedule.v1", &(&keys, 2u8), 16 * 1024)?
        {
            return Err(RatingError::Identity);
        }
        for (index, (opening, key)) in self.openings.iter().zip(&keys).enumerate() {
            opening.validate(
                self.budget.policy,
                &self.policy,
                self.subject.worker(),
                role(&self.subject),
                key.case,
                key.seed,
            )?;
            for repetition in 0..2usize {
                let index = u8::try_from(
                    index
                        .checked_mul(2)
                        .and_then(|n| n.checked_add(repetition))
                        .ok_or(RatingError::Bound)?,
                )
                .map_err(|_| RatingError::Bound)?;
                if self
                    .budget
                    .case(ConductCursor {
                        index,
                        simulated_ms: 0,
                    })
                    .map_err(|_| RatingError::Identity)?
                    != &opening.rate(index)
                {
                    return Err(RatingError::Identity);
                }
            }
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(|_| RatingError::Invalid)?,
            START_BYTES,
        )
        .map_err(|_| RatingError::Bound)?;
        Ok(())
    }
    pub(super) fn prepare(&self) -> Result<PreparedStart, RatingError> {
        let bytes = self.bytes()?;
        Ok(PreparedStart {
            bytes,
            subject: self.subject.clone(),
            manifest: self.policy.definition_ref()?,
            budget: self.budget.clone(),
        })
    }
    pub(super) fn bytes(&self) -> Result<ArtifactBytes, RatingError> {
        self.validate()?;
        ArtifactBytes::encode(self).map_err(|_| RatingError::Bound)
    }
    pub(super) fn matches(&self, job: &ConductJob, plan: &Plan) -> Result<(), RatingError> {
        self.validate()?;
        plan.validate().map_err(|_| RatingError::Identity)?;
        let a = job.admission();
        if &self.subject != job.subject()
            || &self.policy.definition_ref()? != job.manifest()
            || plan.kind != WorkKind::Conduct
            || plan.conduct.as_ref() != Some(&self.budget)
            || plan.start.sha256 != *a.start().sha256.as_bytes()
            || plan.start.bytes != a.start().bytes
            || a.policy() != self.budget.policy
            || a.subject() != self.budget.subject
            || a.schedule() != self.budget.schedule
            || a.budget() != self.budget.digest().map_err(|_| RatingError::Identity)?
            || a.maximum_unit_ms() != self.budget.maximum_unit_ms()
            || plan.result_reservation_bytes != RESULT_BYTES as u64
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    pub(crate) fn decode(
        bytes: &ArtifactBytes,
        job: &ConductJob,
        plan: &Plan,
    ) -> Result<Self, RatingError> {
        if bytes.descriptor().bytes > START_BYTES as u64 || bytes.descriptor() != &plan.start {
            return Err(RatingError::Identity);
        }
        let value: Self =
            serde_json::from_slice(bytes.bytes()).map_err(|_| RatingError::Invalid)?;
        value.matches(job, plan)?;
        Ok(value)
    }
}

/// Unlike CompletedRun, this is merely a strict disk projection. It is never
/// accepted by the live completion participant and has no authority constructor.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedWire {
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
pub(crate) fn result(
    bytes: &ArtifactBytes,
    start: &Start,
) -> Result<ConductMeasurement, RatingError> {
    if bytes.descriptor().bytes > RESULT_BYTES as u64 {
        return Err(RatingError::Bound);
    }
    start.validate()?;
    let value: CompletedWire =
        serde_json::from_slice(bytes.bytes()).map_err(|_| RatingError::Invalid)?;
    let keys = Key::schedule(role(&start.subject));
    if value.format != 1
        || value.policy != start.budget.policy
        || &value.subject != start.subject.worker()
        || value.profile != start.subject.profile()
        || value.role != role(&start.subject)
        || value.keys != keys
        || value.results.len() != keys.len()
        || value.paired_repetitions != 2
        || value.charged_unit_ms > start.budget.maximum_unit_ms()
        || value.passed != value.results.iter().all(ResultRow::passed)
    {
        return Err(RatingError::Identity);
    }
    let mut charge = 0u64;
    for (row, opening) in value.results.iter().zip(&start.openings) {
        charge = charge
            .checked_add(
                row.validate(opening)?
                    .checked_mul(2)
                    .ok_or(RatingError::Bound)?,
            )
            .ok_or(RatingError::Bound)?;
    }
    if charge != value.charged_unit_ms
        || value.digest
            != identity(
                "tacsim.conduct.completed.v1",
                &(
                    value.format,
                    value.policy,
                    &value.subject,
                    &value.profile,
                    value.role,
                    &value.keys,
                    &value.results,
                    value.paired_repetitions,
                    value.charged_unit_ms,
                ),
                RESULT_BYTES,
            )?
    {
        return Err(RatingError::Identity);
    }
    ConductMeasurement::observed(
        value.digest,
        value.policy,
        start.budget.subject,
        charge,
        value.passed,
    )
    .map_err(|_| RatingError::Identity)
}

/// Minted only by the actual captured native trial on its worker. This compact
/// affinity value avoids decoding the complete retained artifact at custody.
/// It is neither serializable nor constructible by a service request.
pub(crate) struct PreparedStart {
    bytes: ArtifactBytes,
    subject: ConductSubject,
    manifest: mush_core::DefinitionRef,
    budget: ConductBudget,
}
impl PreparedStart {
    pub(crate) fn bytes(&self) -> &ArtifactBytes {
        &self.bytes
    }
    pub(crate) fn matches(&self, job: &ConductJob, plan: &Plan) -> Result<(), RatingError> {
        let a = job.admission();
        if job.subject() != &self.subject
            || job.manifest() != &self.manifest
            || plan.kind != WorkKind::Conduct
            || plan.conduct.as_ref() != Some(&self.budget)
            || &plan.start != self.bytes.descriptor()
            || a.start().sha256.as_bytes() != &plan.start.sha256
            || a.start().bytes != plan.start.bytes
            || a.policy() != self.budget.policy
            || a.subject() != self.budget.subject
            || a.schedule() != self.budget.schedule
            || a.budget() != self.budget.digest().map_err(|_| RatingError::Identity)?
            || a.maximum_unit_ms() != self.budget.maximum_unit_ms()
            || plan.result_reservation_bytes != RESULT_BYTES as u64
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
}
