#![cfg(test)]
//! 17g C4 item 5 (the Registry load measurement, `registry_load`): the store's
//! own clock on every Registry lease. Each `rating_grant` and `rating_settle`
//! bundle is named at its reservation (its rating job and lease sequence, read
//! from the `rating_executions` row the bundle writes) and stamped when the
//! store commits it. A test-store observation only: it changes nothing the
//! store does, and it is bounded.
use mush_core::{AuthoritativeBundle, CanonicalHash, CommitToken, ReplayWrite};
use mush_roll::commerce::rating::execution::RatingExecution;
use std::{sync::Mutex, time::Instant};

/// At most this many named bundles waiting for their commit, and this many
/// commits kept (two ratings of about 1,070 leases are about 4,300).
const BOUND: usize = 16_384;

/// Which half of a lease a commit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::world_task::submission_disposition::tests) enum Leg {
    Grant,
    Settle,
}

/// One committed lease bundle: its job, its lease sequence, its half and the
/// moment the store committed it.
#[derive(Clone, Copy, Debug)]
pub(in crate::world_task::submission_disposition::tests) struct LeaseCommit {
    pub job: CommitToken,
    pub sequence: u64,
    pub leg: Leg,
    pub at: Instant,
}

struct Named {
    token: CommitToken,
    hash: CanonicalHash,
    job: CommitToken,
    sequence: u64,
    leg: Leg,
}

#[derive(Default)]
struct State {
    named: Vec<Named>,
    committed: Vec<LeaseCommit>,
}

pub(in crate::world_task::submission_disposition::tests) struct LeaseClock(Mutex<State>);

impl LeaseClock {
    pub(super) fn new() -> Self {
        Self(Mutex::new(State::default()))
    }

    /// Names a Registry lease bundle as the writer reserves it.
    pub(super) fn reserve_enter(&self, bundle: &AuthoritativeBundle) {
        let key = bundle.action().key();
        if key.owner.as_str() != "roll" || key.kind.as_str() != "action" {
            return;
        }
        let leg = match key.id.as_str() {
            "rating_grant" => Leg::Grant,
            "rating_settle" => Leg::Settle,
            _ => return,
        };
        let row = bundle.writes().iter().find_map(|write| {
            let ReplayWrite::ModuleJsonSet {
                namespace, value, ..
            } = write
            else {
                return None;
            };
            (namespace.as_str() == "roll"
                && value.get("family")?.get("id")?.as_str()? == "rating_executions")
                .then(|| value.get("value"))
                .flatten()
        });
        let Some(row) = row else { return };
        let Ok(execution) = serde_json::from_value::<RatingExecution>(row.clone()) else {
            return;
        };
        // The leased sequence: a grant advances `next_sequence` past it and
        // its settle leaves it there (`registry_timing`'s reading).
        let Some(sequence) = execution.observation().next_sequence.checked_sub(1) else {
            return;
        };
        let Ok(mut state) = self.0.lock() else { return };
        if state.named.len() >= BOUND
            || state
                .named
                .iter()
                .any(|n| n.token == bundle.token() && n.hash == bundle.bundle_hash())
        {
            return;
        }
        state.named.push(Named {
            token: bundle.token(),
            hash: bundle.bundle_hash(),
            job: execution.job(),
            sequence,
            leg,
        });
    }

    /// Stamps a named bundle the store has just committed.
    pub(super) fn committed(&self, token: CommitToken, hash: CanonicalHash) {
        let at = Instant::now();
        let Ok(mut state) = self.0.lock() else { return };
        let Some(index) = state
            .named
            .iter()
            .position(|n| n.token == token && n.hash == hash)
        else {
            return;
        };
        let named = state.named.swap_remove(index);
        if state.committed.len() < BOUND {
            state.committed.push(LeaseCommit {
                job: named.job,
                sequence: named.sequence,
                leg: named.leg,
                at,
            });
        }
    }

    /// Every lease commit so far, in commit order.
    pub(in crate::world_task::submission_disposition::tests) fn commits(&self) -> Vec<LeaseCommit> {
        self.0
            .lock()
            .map(|state| state.committed.clone())
            .unwrap_or_default()
    }
}
