//! The actual private role worker implements the shared prepaid executor. A
//! changed roster yields before another simulation step can use the old lease.
use super::run::Run;
use crate::minds::bulk::{
    BulkError,
    conduct_budget::{ConductBudget, ConductCase, ConductCursor, ConductEnding, ConductProgress},
    executor::{Machine, Turn},
    model::{ArtifactBytes, Status},
};

pub(super) struct Worker {
    run: Run,
    budget: ConductBudget,
    case: ConductCase,
    progress: ConductProgress,
    finished: bool,
    measurement: Option<mush_roll::commerce::conduct::records::ConductMeasurement>,
}
fn invalid(_: impl std::fmt::Display) -> BulkError {
    BulkError::Invalid("Conduct worker evidence is incomplete or differs".into())
}
impl Worker {
    pub(super) fn new(run: Run) -> Result<Box<Self>, BulkError> {
        let budget = run.billing_budget().map_err(invalid)?;
        let case = run.billing_case().map_err(invalid)?;
        if budget.case(ConductCursor::default())? != &case {
            return Err(invalid("opening rate"));
        }
        Ok(Box::new(Self {
            run,
            budget,
            case,
            progress: ConductProgress {
                cursor: ConductCursor::default(),
                ending: ConductEnding::Continue,
            },
            finished: false,
            measurement: None,
        }))
    }
    fn advance(&mut self, maximum: u64) -> Result<Turn, BulkError> {
        if self.finished || maximum < u64::from(self.case.native_ms) {
            return Err(invalid("lease"));
        }
        let current = self.run.billing_case().map_err(invalid)?;
        if current != self.case {
            return Err(invalid("actual roster"));
        }
        let prepaid = u64::from(current.units)
            .checked_mul(u64::from(current.native_ms))
            .ok_or_else(|| invalid("charge"))?;
        let progress = self.run.advance(prepaid).map_err(invalid)?;
        if !progress
            .charged_unit_ms
            .is_multiple_of(u64::from(current.units))
        {
            return Err(invalid("charge"));
        }
        let simulation_ms = progress.charged_unit_ms / u64::from(current.units);
        let (next, ending) = if progress.completed.is_some() {
            (current.clone(), ConductEnding::Complete)
        } else {
            let next = self.run.billing_case().map_err(invalid)?;
            let ending = if next.index == current.index {
                ConductEnding::Continue
            } else {
                ConductEnding::CaseComplete
            };
            (next, ending)
        };
        let cursor = self
            .budget
            .advance(self.progress.cursor, simulation_ms, ending)?;
        if self.budget.case(cursor)? != &next {
            return Err(invalid("post roster"));
        }
        let measurement = progress
            .completed
            .as_ref()
            .map(|row| row.measurement())
            .transpose()
            .map_err(invalid)?;
        let result = progress
            .completed
            .map(|row| ArtifactBytes::encode(&row))
            .transpose()?;
        self.progress = ConductProgress { cursor, ending };
        self.case = next;
        self.finished = ending == ConductEnding::Complete;
        self.measurement = measurement;
        Ok(Turn {
            simulation_ms,
            result,
            terminal: self.finished.then_some(Status::Complete),
        })
    }
}
impl Machine for Worker {
    fn conduct_measurement(
        &self,
    ) -> Option<mush_roll::commerce::conduct::records::ConductMeasurement> {
        self.measurement.clone()
    }
    fn conduct_case(&self) -> Option<ConductCase> {
        Some(self.case.clone())
    }
    fn conduct_progress(&self) -> Option<ConductProgress> {
        Some(self.progress)
    }
    fn begin_quantum(&mut self) {
        if !self.finished {
            self.progress.ending = ConductEnding::Continue;
        }
    }
    fn native_substep_ms(&self) -> u32 {
        self.case.native_ms
    }
    fn next_simulation_ms(&self) -> u64 {
        u64::from(self.case.native_ms)
    }
    fn step(&mut self, maximum: u64) -> Result<Turn, BulkError> {
        let result = self.advance(maximum);
        if result.is_err() {
            self.finished = true;
        }
        result
    }
}
