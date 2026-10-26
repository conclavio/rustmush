//! Test-store observations, deliberately not called worker or service ACK times.
use mush_core::{AuthoritativeBundle, CanonicalHash, CommitToken, ReplayWrite};
use mush_roll::commerce::rating::execution::RatingExecution;
use std::{sync::Mutex, time::Instant};

struct Record {
    token: CommitToken,
    hash: CanonicalHash,
    sequence: u64,
    action: &'static str,
    seen: [bool; 4],
}
struct State {
    started: Instant,
    job: Option<CommitToken>,
    records: Vec<Record>,
}
pub(super) struct Observer(Mutex<State>);
impl Observer {
    pub(super) fn new() -> Self {
        Self(Mutex::new(State {
            started: Instant::now(),
            job: None,
            records: Vec::new(),
        }))
    }
    pub(super) fn reserve_enter(&self, bundle: &AuthoritativeBundle) {
        let key = bundle.action().key();
        if key.owner.as_str() != "roll" || key.kind.as_str() != "action" {
            return;
        }
        let action = match key.id.as_str() {
            "rating_grant" => "grant",
            "rating_settle" => "settle",
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
        // Avoid decoding later leases; the actual strict typed record is still
        // required below before any selected diagnostic record is emitted.
        let Some(next) = row
            .get("observation")
            .and_then(|x| x.get("next_sequence"))
            .and_then(|x| x.as_u64())
        else {
            return;
        };
        if !(2..=33).contains(&next) {
            return;
        }
        let Ok(execution) = serde_json::from_value::<RatingExecution>(row.clone()) else {
            return;
        };
        let Some(sequence) = execution.observation().next_sequence.checked_sub(1) else {
            return;
        };
        {
            let Ok(mut state) = self.0.lock() else { return };
            if state.job.is_some_and(|job| job != execution.job()) {
                return;
            }
            if !state
                .records
                .iter()
                .any(|r| r.token == bundle.token() && r.hash == bundle.bundle_hash())
            {
                if state.records.len() >= 64 {
                    return;
                }
                state.job = Some(execution.job());
                state.records.push(Record {
                    token: bundle.token(),
                    hash: bundle.bundle_hash(),
                    sequence,
                    action,
                    seen: [false; 4],
                });
            }
        }
        self.point(bundle.token(), bundle.bundle_hash(), 0);
    }
    pub(super) fn reserved(&self, token: CommitToken, hash: CanonicalHash) {
        self.point(token, hash, 1);
    }
    pub(super) fn reconcile_enter(&self, token: CommitToken, hash: CanonicalHash) {
        self.point(token, hash, 2);
    }
    pub(super) fn committed(&self, token: CommitToken, hash: CanonicalHash) {
        self.point(token, hash, 3);
    }
    fn point(&self, token: CommitToken, hash: CanonicalHash, point: usize) {
        let observed = Instant::now();
        let event = {
            let Ok(mut state) = self.0.lock() else { return };
            let elapsed = observed
                .saturating_duration_since(state.started)
                .as_micros();
            let Some(row) = state
                .records
                .iter_mut()
                .find(|r| r.token == token && r.hash == hash)
            else {
                return;
            };
            let Some(seen) = row.seen.get_mut(point) else {
                return;
            };
            if *seen {
                return;
            }
            *seen = true;
            (row.sequence, row.action, elapsed)
        };
        let Some(label) = [
            "store_reserve_enter",
            "store_reserved",
            "store_reconcile_enter",
            "store_committed",
        ]
        .get(point) else {
            return;
        };
        eprintln!(
            "M7_SERVER_REGISTRY_TIMING cert=1 lease={} action={} point={} elapsed_us={}",
            event.0, event.1, label, event.2
        );
    }
}
