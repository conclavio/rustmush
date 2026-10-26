#![cfg(test)]
//! Bounded snapshots, explicitly separate from actual transition timestamps.
use super::*;
use crate::minds::bulk::registry_timing::{self, Scope};
use mush_roll::commerce::rating::{RatingJob, execution::RatingExecution};

pub(super) struct Progress {
    ordinal: u8,
    started: Instant,
    sampled: Instant,
    reported: Instant,
    last_case: Option<u16>,
    reports: u16,
    scope: Option<Scope>,
}
impl Progress {
    pub(super) fn new(ordinal: u8) -> Self {
        let now = Instant::now();
        eprintln!("M7_LAUNCH_REGISTRY_CERT cert={ordinal} point=start elapsed_us=0");
        Self {
            ordinal,
            started: now,
            sampled: now,
            reported: now,
            last_case: None,
            reports: 0,
            scope: None,
        }
    }
    pub(super) fn job(&mut self, job: &RatingJob, revision: &mush_roll::Revision) {
        if self.scope.is_none()
            && job.subject().revision().revision() == revision.reference()
            && job.subject().revision().source() == revision.source()
        {
            self.scope = registry_timing::begin(job.job(), self.ordinal, self.started);
        }
    }
    pub(super) fn sample(&mut self, r: &Rig, force: bool) {
        if self.reports >= 512 || (!force && self.sampled.elapsed() < Duration::from_millis(250)) {
            return;
        }
        self.sampled = Instant::now();
        let job = self.scope.as_ref().map(Scope::job);
        let execution = r.rows("rating_executions").into_iter().find_map(|wire| {
            let row = serde_json::from_value::<RatingExecution>(wire).ok()?;
            (Some(row.job()) == job).then_some(row)
        });
        if let Some(execution) = execution {
            let observed = execution.observation();
            if force
                || self.last_case != Some(observed.cursor.index)
                || self.reported.elapsed() >= Duration::from_secs(30)
            {
                self.last_case = Some(observed.cursor.index);
                self.reported = Instant::now();
                self.reports += 1;
                eprintln!(
                    "M7_LAUNCH_REGISTRY_PROGRESS cert={} elapsed_us={} phase={:?} next={} case={} simulated_ms={} charged_unit_ms={} flight={} result={}",
                    self.ordinal,
                    self.started.elapsed().as_micros(),
                    observed.phase,
                    observed.next_sequence,
                    observed.cursor.index,
                    observed.cursor.simulated_ms,
                    observed.charged_unit_ms,
                    observed.flight.is_some(),
                    observed.result.is_some()
                );
            }
        } else if force || self.reported.elapsed() >= Duration::from_secs(30) {
            self.reported = Instant::now();
            self.reports += 1;
            eprintln!(
                "M7_LAUNCH_REGISTRY_PROGRESS cert={} elapsed_us={} observation=unavailable",
                self.ordinal,
                self.started.elapsed().as_micros()
            );
        }
    }
    pub(super) fn finish(&self, completed: bool) {
        eprintln!(
            "M7_LAUNCH_REGISTRY_CERT cert={} point=end elapsed_us={} completed={completed}",
            self.ordinal,
            self.started.elapsed().as_micros()
        );
    }
}
