#![cfg(test)]
//! Diagnostic-only timing of actual transitions for explicitly selected jobs.
use mush_core::{CommitToken, World};
use mush_roll::commerce::rating::{execution::ExecutionObservation, protocol::Transition};
use std::{sync::Mutex, time::Instant};

use super::{owner::BulkStore, registry_tickets::Binding};

#[derive(Clone, Copy)]
pub(super) enum Point {
    GrantSubmit,
    GrantAcknowledged,
    WorkerStart,
    WorkerEnd,
    SettleSubmit,
    SettleAcknowledged,
}
impl Point {
    fn label(self) -> &'static str {
        match self {
            Self::GrantSubmit => "grant_submit_enter",
            Self::GrantAcknowledged => "grant_ack_consumed",
            Self::WorkerStart => "worker_start",
            Self::WorkerEnd => "worker_end",
            Self::SettleSubmit => "settle_submit_enter",
            Self::SettleAcknowledged => "settle_ack_consumed",
        }
    }
}
struct Trace {
    job: CommitToken,
    ordinal: u8,
    started: Instant,
    seen: [[bool; 6]; 32],
}
static TRACES: Mutex<Vec<Trace>> = Mutex::new(Vec::new());

pub(crate) struct Scope {
    job: CommitToken,
}
impl Scope {
    pub(crate) fn job(&self) -> CommitToken {
        self.job
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if let Ok(mut traces) = TRACES.lock() {
            traces.retain(|trace| trace.job != self.job);
        }
    }
}
pub(crate) fn begin(job: CommitToken, ordinal: u8, started: Instant) -> Option<Scope> {
    let Ok(mut traces) = TRACES.lock() else {
        return None;
    };
    if !(1..=3).contains(&ordinal) || traces.len() >= 3 || traces.iter().any(|t| t.job == job) {
        return None;
    }
    traces.push(Trace {
        job,
        ordinal,
        started,
        seen: [[false; 6]; 32],
    });
    Some(Scope { job })
}
pub(super) fn record(job: CommitToken, sequence: u64, point: Point) {
    let observed = Instant::now();
    let Some(index) = sequence
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
    else {
        return;
    };
    if index >= 32 {
        return;
    }
    let event = {
        let Ok(mut traces) = TRACES.lock() else {
            return;
        };
        let Some(trace) = traces.iter_mut().find(|t| t.job == job) else {
            return;
        };
        let Some(seen) = trace
            .seen
            .get_mut(index)
            .and_then(|row| row.get_mut(point as usize))
        else {
            return;
        };
        if *seen {
            return;
        }
        *seen = true;
        (
            trace.ordinal,
            observed
                .saturating_duration_since(trace.started)
                .as_micros(),
        )
    };
    eprintln!(
        "M7_LAUNCH_REGISTRY_TIMING cert={} lease={} point={} elapsed_us={}",
        event.0,
        sequence,
        point.label(),
        event.1
    );
}
pub(super) fn submit(world: &World, binding: Option<Binding>, transition: Transition) {
    let Some(binding) = binding else { return };
    let Some(store) = world.module_state::<BulkStore>() else {
        return;
    };
    let Ok(run) = store.account_run(binding.account, binding.run) else {
        return;
    };
    let pair = match transition {
        Transition::Grant => Some((run.next_sequence, Point::GrantSubmit)),
        Transition::Settle => run
            .flight
            .as_ref()
            .map(|flight| (flight.sequence, Point::SettleSubmit)),
        _ => None,
    };
    if let Some((sequence, point)) = pair {
        record(binding.job, sequence, point);
    }
}
pub(super) fn acknowledged(job: CommitToken, transition: Transition, after: &ExecutionObservation) {
    let point = match transition {
        Transition::Grant => Point::GrantAcknowledged,
        Transition::Settle => Point::SettleAcknowledged,
        _ => return,
    };
    if let Some(sequence) = after.next_sequence.checked_sub(1) {
        record(job, sequence, point);
    }
}
