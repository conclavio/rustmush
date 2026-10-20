//! Closed data shapes; none is a worker receipt or publication capability.
use super::*;
use crate::commerce::rating::{
    RatingRef, RatingSubject,
    execution::{ArtifactIdentity, ExecutionObservation, ExecutionPhase},
};
use crate::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity, records::Record};
use mush_core::{CommitToken, DefinitionRef, ObjId};
use serde::{Deserialize, Deserializer, Serialize};

macro_rules! strict {
    ($name:ident,$wire:ident,{$($(#[$attr:meta])* $field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
        pub struct $name { $(pub(super) $field:$ty),* }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct $wire { $($(#[$attr])* $field:$ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let w=$wire::deserialize(d)?; let v=Self{$($field:w.$field),*};
                v.validate().map_err(serde::de::Error::custom)?; Ok(v)
            }
        }
    }
}
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConductRole {
    Escort,
    Transit,
}

strict!(ConductSubject,SubjectWire,{
    charter:CharterId,line:LineId,filed:FiledRevisionId,revision:RevisionIdentity,
    rating:RatingRef,worker:RatingSubject,profile:String,role:ConductRole
});
impl ConductSubject {
    /// Structural tuple only. The actual reader separately proves every row,
    /// current certificate page and source before preparing an admitted job.
    #[allow(clippy::too_many_arguments)]
    pub fn captured(
        charter: CharterId,
        line: LineId,
        filed: FiledRevisionId,
        revision: RevisionIdentity,
        rating: RatingRef,
        worker: RatingSubject,
        profile: String,
        role: ConductRole,
    ) -> Result<Self, PublicationError> {
        let v = Self {
            charter,
            line,
            filed,
            revision,
            rating,
            worker,
            profile,
            role,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.revision != self.worker.revision()
            || self.profile.is_empty()
            || self.profile.len() > 64
            || self.profile.trim() != self.profile
            || self.profile.chars().any(char::is_control)
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, 4096)
    }
    pub fn worker_digest(&self) -> Result<DefinitionDigest, PublicationError> {
        digest(
            "tacsim.conduct.subject.v1",
            &(&self.worker, &self.profile, self.role),
            4096,
        )
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
    pub fn worker(&self) -> &RatingSubject {
        &self.worker
    }
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub const fn role(&self) -> ConductRole {
        self.role
    }
}

strict!(ConductAdmission,AdmissionWire,{
    account_binding:DefinitionDigest,character:ObjId,run:u64,nonce:u64,
    subject:DefinitionDigest,policy:DefinitionDigest,schedule:DefinitionDigest,budget:DefinitionDigest,
    start:ArtifactIdentity,maximum_unit_ms:u64
});
impl ConductAdmission {
    /// The producer accepts this projection only as an actual registered owner
    /// output, never as a decoded client field.
    #[allow(clippy::too_many_arguments)]
    pub fn observed(
        account_binding: DefinitionDigest,
        character: ObjId,
        run: u64,
        nonce: u64,
        subject: DefinitionDigest,
        policy: DefinitionDigest,
        schedule: DefinitionDigest,
        budget: DefinitionDigest,
        start: ArtifactIdentity,
        maximum_unit_ms: u64,
    ) -> Result<Self, PublicationError> {
        let v = Self {
            account_binding,
            character,
            run,
            nonce,
            subject,
            policy,
            schedule,
            budget,
            start,
            maximum_unit_ms,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.start.validate().map_err(invalid)?;
        if self.character != mush_core::GOD
            || self.run == 0
            || self.nonce == 0
            || self.maximum_unit_ms == 0
            || self.maximum_unit_ms > 60 * 4 * 121_000
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, 2048)
    }
    pub const fn character(&self) -> ObjId {
        self.character
    }
    pub const fn run(&self) -> u64 {
        self.run
    }
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }
    pub const fn start(&self) -> &ArtifactIdentity {
        &self.start
    }
    pub const fn maximum_unit_ms(&self) -> u64 {
        self.maximum_unit_ms
    }
    pub const fn account_binding(&self) -> DefinitionDigest {
        self.account_binding
    }
    pub const fn policy(&self) -> DefinitionDigest {
        self.policy
    }
    pub const fn subject(&self) -> DefinitionDigest {
        self.subject
    }
    pub const fn schedule(&self) -> DefinitionDigest {
        self.schedule
    }
    pub const fn budget(&self) -> DefinitionDigest {
        self.budget
    }
}

strict!(ConductJob,JobWire,{
    format:u16,operation:CommitToken,action:DefinitionRef,manifest:DefinitionRef,
    subject:ConductSubject,admission:ConductAdmission,at_utc:i64
});
impl ConductJob {
    pub const FAMILY: &'static str = "roll_conduct_trial_job";
    pub const MAX_BYTES: usize = JOB_BYTES;
    /// Structural reconstruction used by the actual quota/artifact owner and
    /// Roll producer. Authority still requires their registered output union.
    pub fn observed(
        operation: CommitToken,
        action: DefinitionRef,
        manifest: DefinitionRef,
        subject: ConductSubject,
        admission: ConductAdmission,
        at_utc: i64,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: 1,
            operation,
            action,
            manifest,
            subject,
            admission,
            at_utc,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.subject.validate()?;
        self.admission.validate()?;
        if self.format != 1
            || self.admission.subject != self.subject.worker_digest()?
            || self.subject.rating.job() == self.operation
        {
            return Err(PublicationError::Mismatch);
        }
        super::protocol::verify_action(
            &self.action,
            &self.manifest,
            super::protocol::Transition::Admit,
        )?;
        bounded(self, JOB_BYTES - 1024)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.conduct.job.v1", self, JOB_BYTES)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn subject(&self) -> &ConductSubject {
        &self.subject
    }
    pub fn admission(&self) -> &ConductAdmission {
        &self.admission
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
}

strict!(ConductExecution,ExecutionWire,{
    format:u16,job:CommitToken,updated_by:CommitToken,action:DefinitionRef,
    observation:ExecutionObservation
});
impl ConductExecution {
    pub const FAMILY: &'static str = "roll_conduct_trial_execution";
    pub const MAX_BYTES: usize = EXECUTION_BYTES;
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.observation
            .validate(60 * 4 * 121_000)
            .map_err(invalid)?;
        if self.format != 1
            || self.observation.cursor.index >= REPETITIONS
            || self.observation.cursor.simulated_ms > 121_000
            || self
                .observation
                .flight
                .as_ref()
                .is_some_and(|f| !(3..=4).contains(&f.units) || f.native_ms > 1000)
            || (self.observation.phase == ExecutionPhase::Complete
                && (self.observation.cursor.index != 59
                    || self.observation.cursor.simulated_ms < MAXIMUM_SIMULATION_MS))
            || ((self.observation.phase == ExecutionPhase::Queued) != (self.job == self.updated_by))
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, EXECUTION_BYTES - 1024)
    }
    pub fn matches_job(&self, job: &ConductJob) -> Result<(), PublicationError> {
        self.validate()?;
        job.validate()?;
        if self.job != job.operation
            || self.observation.charged_unit_ms > job.admission.maximum_unit_ms
        {
            return Err(PublicationError::Mismatch);
        }
        let transition = super::protocol::Transition::from_action(&self.action)?;
        super::protocol::verify_action(&self.action, &job.manifest, transition)?;
        if (transition == super::protocol::Transition::Admit)
            != (self.observation.phase == ExecutionPhase::Queued)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.conduct.execution.v1", self, EXECUTION_BYTES)
    }
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    pub const fn updated_by(&self) -> CommitToken {
        self.updated_by
    }
    pub fn observation(&self) -> &ExecutionObservation {
        &self.observation
    }
}

strict!(ConductMeasurement,MeasurementWire,{
    digest:DefinitionDigest,policy:DefinitionDigest,subject:DefinitionDigest,
    charged_unit_ms:u64,paired_cases:u16,passed:bool
});
impl ConductMeasurement {
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.paired_cases != 30
            || self.charged_unit_ms == 0
            || self.charged_unit_ms > 60 * 4 * 121_000
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub fn observed(
        digest: DefinitionDigest,
        policy: DefinitionDigest,
        subject: DefinitionDigest,
        charged_unit_ms: u64,
        passed: bool,
    ) -> Result<Self, PublicationError> {
        let v = Self {
            digest,
            policy,
            subject,
            charged_unit_ms,
            paired_cases: 30,
            passed,
        };
        v.validate()?;
        Ok(v)
    }
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub const fn passed(&self) -> bool {
        self.passed
    }
    pub const fn charged_unit_ms(&self) -> u64 {
        self.charged_unit_ms
    }
    pub const fn policy(&self) -> DefinitionDigest {
        self.policy
    }
    pub const fn subject(&self) -> DefinitionDigest {
        self.subject
    }
}

strict!(ConductResult,ResultWire,{
    format:u16,job:CommitToken,operation:CommitToken,action:DefinitionRef,
    job_digest:DefinitionDigest,execution_digest:DefinitionDigest,
    phase:ExecutionPhase,
    #[serde(deserialize_with="required")] artifact:Option<ArtifactIdentity>,
    #[serde(deserialize_with="required")] measurement:Option<ConductMeasurement>
});
impl ConductResult {
    pub const FAMILY: &'static str = "roll_conduct_trial_result";
    pub const MAX_BYTES: usize = RESULT_BYTES;
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || !self.phase.terminal() || self.operation == self.job {
            return Err(PublicationError::Invalid);
        }
        match (&self.artifact, &self.measurement, self.phase) {
            (Some(artifact), Some(measurement), ExecutionPhase::Complete) => {
                artifact.validate().map_err(invalid)?;
                measurement.validate()?;
                if artifact.bytes > MAX_RESULT_ARTIFACT_BYTES {
                    return Err(PublicationError::Capacity);
                }
            }
            (None, None, phase) if phase != ExecutionPhase::Complete => {}
            _ => return Err(PublicationError::Mismatch),
        }
        bounded(self, RESULT_BYTES - 1024)
    }
    pub fn matches(
        &self,
        job: &ConductJob,
        execution: &ConductExecution,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        execution.matches_job(job)?;
        if self.job != job.operation
            || self.operation != execution.updated_by
            || self.action != execution.action
            || self.phase != execution.observation.phase
            || self.artifact != execution.observation.result
            || self.job_digest != job.digest()?
            || self.execution_digest != execution.digest()?
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(m) = &self.measurement {
            if m.policy != job.admission.policy
                || m.subject != job.admission.subject
                || m.charged_unit_ms != execution.observation.charged_unit_ms
            {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.conduct.result.v1", self, RESULT_BYTES)
    }
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn measurement(&self) -> Option<&ConductMeasurement> {
        self.measurement.as_ref()
    }
    pub fn artifact(&self) -> Option<&ArtifactIdentity> {
        self.artifact.as_ref()
    }
    pub fn qualifies(&self) -> bool {
        self.phase == ExecutionPhase::Complete
            && self.measurement.as_ref().is_some_and(|m| m.passed)
    }
}

macro_rules! row {
    ($t:ty,$key:ident) => {
        impl Record for $t {
            const FAMILY: &'static str = <$t>::FAMILY;
            const MAX_BYTES: usize = <$t>::MAX_BYTES;
            fn key(&self) -> String {
                self.$key.to_hex()
            }
            fn validate(&self) -> Result<(), PublicationError> {
                <$t>::validate(self)
            }
        }
    };
}
row!(ConductJob, operation);
row!(ConductExecution, job);
row!(ConductResult, job);
