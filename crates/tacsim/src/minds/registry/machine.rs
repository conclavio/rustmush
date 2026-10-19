//! Real per-case execution inside the existing dedicated bulk worker. This
//! module has no World, source-store or live-pool access. Publication still
//! requires the registered three-owner completion path; an artifact is evidence,
//! never an admission capability.
use super::{
    evidence::{CaseDisposition, Evidence},
    ford, kernels,
    manifest::{FrozenPolicy, RegisteredManifest},
    order_loss,
    plan::{Case, Schedule},
    probes, profiles,
    subject::PreparedSubject,
    trials,
};
use crate::minds::bulk::{
    BulkError,
    executor::{Machine, Turn},
    model::{ArtifactBytes, Status},
    registry_budget::{
        RegistryBudget, RegistryCase, RegistryCursor, RegistryEnding, RegistryProgress,
    },
};
use mush_roll::commerce::rating::{
    CaseBinding, CaseKind, MAX_RESULT_BYTES, RatingCompatibility, RatingError,
};
use std::sync::Arc;

enum Active {
    Trial(Box<trials::Trial>),
    Picture(Box<probes::Single>),
    Ford(Box<ford::Ford>),
    Loss(Box<order_loss::Loss>),
    Profiles(Box<profiles::Cycle>),
    Kernel(Box<kernels::Boundary>),
}
impl Active {
    fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        match case.key.kind() {
            CaseKind::Trial { .. } => trials::Trial::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Trial),
            CaseKind::Probe {
                name:
                    mush_roll::commerce::rating::ProbeName::FordMobile
                    | mush_roll::commerce::rating::ProbeName::FordDisabled,
                ..
            } => ford::Ford::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Ford),
            CaseKind::Probe {
                name:
                    mush_roll::commerce::rating::ProbeName::OrderLossArmed
                    | mush_roll::commerce::rating::ProbeName::OrderLossUnarmed,
                ..
            } => order_loss::Loss::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Loss),
            CaseKind::Probe { .. } => probes::Single::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Picture),
            CaseKind::ProfileCycle {} => profiles::Cycle::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Profiles),
            CaseKind::Kernel { .. } => kernels::Boundary::open(policy, subject, case)
                .map(Box::new)
                .map(Self::Kernel),
        }
    }
    fn binding(&self) -> &CaseBinding {
        match self {
            Self::Trial(v) => v.binding(),
            Self::Picture(v) => v.binding(),
            Self::Ford(v) => v.binding(),
            Self::Loss(v) => v.binding(),
            Self::Profiles(v) => v.binding(),
            Self::Kernel(v) => v.binding(),
        }
    }
    fn advance(&mut self, maximum: u64) -> Result<trials::Slice, RatingError> {
        match self {
            Self::Trial(v) => v.advance(maximum),
            Self::Picture(v) => v.advance(maximum),
            Self::Ford(v) => v.advance(maximum),
            Self::Loss(v) => v.advance(maximum),
            Self::Profiles(v) => v.advance(maximum),
            Self::Kernel(v) => v.advance(maximum),
        }
    }
}

/// Constructed only by the closed frozen-job preparer after every recipe and
/// retained input has been admitted. No command receives this constructor.
/// The factory is still inactive while registered admission/completion
/// participants are under implementation and the real recipes await gates.
pub(super) struct RegistryMachine {
    policy: Arc<FrozenPolicy>,
    subject: Arc<PreparedSubject>,
    compatibility: RatingCompatibility,
    schedule: Schedule,
    budget: RegistryBudget,
    current_case: RegistryCase,
    progress: RegistryProgress,
    active: Option<Active>,
    evidence: Option<Evidence>,
    done: bool,
    // Test diagnostics retain only a fixed stage label, never source or tape data.
    #[cfg(test)]
    diagnostic_stage: &'static str,
}
impl RegistryMachine {
    pub(super) fn new(
        policy: Arc<FrozenPolicy>,
        subject: Arc<PreparedSubject>,
        registered: &RegisteredManifest,
        schedule: Schedule,
    ) -> Result<Self, RatingError> {
        if !policy.matches_registration(registered)? {
            return Err(RatingError::Identity);
        }
        let compatibility = registered.compatibility().clone();
        let budget = schedule.budget(&subject, compatibility.digest())?;
        let current_case = budget
            .case(RegistryCursor::default())
            .map_err(|_| RatingError::Incomplete)?
            .clone();
        Ok(Self {
            policy,
            subject,
            compatibility,
            schedule,
            budget,
            current_case,
            progress: RegistryProgress {
                cursor: RegistryCursor::default(),
                ending: RegistryEnding::Continue,
            },
            active: None,
            evidence: Some(Evidence::new()),
            done: false,
            #[cfg(test)]
            diagnostic_stage: "new",
        })
    }
    fn case(&self) -> Result<&Case, RatingError> {
        self.schedule
            .cases()
            .get(usize::from(self.progress.cursor.index))
            .ok_or(RatingError::Incomplete)
    }
    fn advance(&mut self, maximum: u64) -> Result<Turn, RatingError> {
        #[cfg(test)]
        {
            self.diagnostic_stage = "entry";
        }
        if self.done || self.progress.ending != RegistryEnding::Continue {
            return Err(RatingError::Invalid);
        }
        if self.active.is_none() {
            #[cfg(test)]
            {
                self.diagnostic_stage = "open";
            }
            self.active = Some(Active::open(&self.policy, &self.subject, self.case()?)?);
            // Opening/compilation consumes one bounded non-time worker phase.
            // No simulated time or source invocation has happened yet.
            return Ok(Turn {
                simulation_ms: 0,
                result: None,
                terminal: None,
            });
        }
        let active = self.active.as_mut().ok_or(RatingError::Incomplete)?;
        #[cfg(test)]
        {
            self.diagnostic_stage = "advance";
        }
        let slice = active.advance(maximum)?;
        if slice.simulated_ms > maximum {
            return Err(RatingError::Bound);
        }
        let mut ending = RegistryEnding::Continue;
        let mut result = None;
        let mut terminal = None;
        if let Some(row) = slice.result {
            #[cfg(test)]
            {
                self.diagnostic_stage = "evidence-push";
            }
            let disposition = self
                .evidence
                .as_mut()
                .ok_or(RatingError::Incomplete)?
                .push(&self.schedule, row, active.binding())?;
            ending = match disposition {
                CaseDisposition::Next => RegistryEnding::CaseComplete,
                CaseDisposition::Promote => RegistryEnding::ClassBudget,
                CaseDisposition::Certified => RegistryEnding::Certified,
            };
            if matches!(disposition, CaseDisposition::Certified) {
                #[cfg(test)]
                {
                    self.diagnostic_stage = "certificate-finish";
                }
                let evidence = self.evidence.take().ok_or(RatingError::Incomplete)?;
                let certificate =
                    evidence.finish(&self.subject, &self.compatibility, &self.schedule)?;
                // Each previously validated row/page has its independent small
                // bound; the aggregate has a stricter Registry artifact ceiling.
                #[cfg(test)]
                {
                    self.diagnostic_stage = "certificate-encode";
                }
                let bytes = serde_json::to_vec(&certificate).map_err(|_| RatingError::Invalid)?;
                if bytes.len() > MAX_RESULT_BYTES {
                    return Err(RatingError::Bound);
                }
                result = Some(ArtifactBytes::new(bytes).map_err(|_| RatingError::Bound)?);
                terminal = Some(Status::Complete);
                self.done = true;
            }
            self.active = None;
        }
        #[cfg(test)]
        {
            self.diagnostic_stage = "budget-advance";
        }
        let cursor = self
            .budget
            .advance(self.progress.cursor, slice.simulated_ms, ending)
            .map_err(|_| RatingError::Invalid)?;
        #[cfg(test)]
        {
            self.diagnostic_stage = "next-case";
        }
        self.current_case = self
            .budget
            .case(cursor)
            .map_err(|_| RatingError::Incomplete)?
            .clone();
        self.progress = RegistryProgress { cursor, ending };
        Ok(Turn {
            simulation_ms: slice.simulated_ms,
            result,
            terminal,
        })
    }
}
impl Machine for RegistryMachine {
    fn registry_case(&self) -> Option<RegistryCase> {
        Some(self.current_case.clone())
    }
    fn registry_progress(&self) -> Option<RegistryProgress> {
        Some(self.progress)
    }
    fn begin_quantum(&mut self) {
        if !self.done {
            self.progress.ending = RegistryEnding::Continue;
        }
    }
    fn native_substep_ms(&self) -> u32 {
        self.current_case.native_ms
    }
    fn next_simulation_ms(&self) -> u64 {
        if self.active.is_some() {
            u64::from(self.native_substep_ms())
        } else {
            0
        }
    }
    fn step(&mut self, maximum_sim_ms: u64) -> Result<Turn, BulkError> {
        let result = self.advance(maximum_sim_ms);
        #[cfg(test)]
        if let Err(error) = &result {
            eprintln!(
                "M7_REGISTRY_CASE_FAILURE stage={} error={error:?} case={:?} progress={:?} maximum_sim_ms={maximum_sim_ms}",
                self.diagnostic_stage, self.current_case, self.progress,
            );
            if let Some(Active::Trial(trial)) = &self.active {
                trial.failure_diagnostic();
            }
        }
        result.map_err(|error| BulkError::Invalid(format!("Registry case: {error}")))
    }
}
