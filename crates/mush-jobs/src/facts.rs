//! The jobs [`FactProvider`] (E4c, 08c §6.1 item 1) — the first implementor of the
//! economy's external-fact seam.
//!
//! The seam lives in `mush-economy`; the *answer* lives here, with the data, which
//! is the E1c direction (a faction standing predicate, a custody provider, a scope-use
//! binder — all the same shape). The dependency runs `mush-jobs → mush-economy` and
//! nothing runs back: neither the economy nor org depends on jobs, directly or in dev.
//!
//! ## One fact, and why only one
//!
//! `jobs.resolved` answers *"was job N approved?"* — and **only for a
//! payload-bearing job**. A payload job is one-shot by construction: `+job/approve`
//! runs its handler and `+job/status` refuses to reopen it afterwards
//! (`commands.rs`, the double-XP-spend guard), so `Resolved` is a fact about the
//! past that cannot be taken back. A **payload-less** job has no such latch: staff
//! may set it back to `Open` at any time, so its resolution is a *state*, not a
//! terminal fact, and a contract that settled on it could be settling on something
//! that stops being true a minute later. This provider therefore answers `None`
//! (unknown → unsatisfied) for a payload-less job rather than `Some(false)`: the
//! distinction is "this is not a fact I can vouch for", not "it did not happen".

use mush_core::World;
use mush_economy::{FactProvider, FactState};

use crate::data::{JobStatus, JobsState};

/// The jobs fact provider. Wire it into
/// [`EconomyModule::with_facts`](mush_economy::EconomyModule::with_facts).
#[derive(Debug, Default, Clone, Copy)]
pub struct JobsFactProvider;

impl FactProvider for JobsFactProvider {
    fn provider(&self) -> &str {
        "jobs"
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        if kind != "resolved" {
            return None;
        }
        // A job carries no version counter, so a reference that *claims* one is
        // asking about a version this provider cannot identify. Doc 10 §15 wants
        // `revision` honoured rather than ignored — a provider may refuse a stale
        // reference — and the honest answer here is "unknown", not a fact about
        // whatever the record happens to say now.
        if revision.is_some() {
            return None;
        }
        let job_id: u64 = id.parse().ok()?;
        let job = w.module_state::<JobsState>()?.jobs.get(&job_id)?;
        // The payload rule, above.
        job.payload.as_ref()?;
        Some(FactState {
            satisfied: job.status == JobStatus::Resolved,
            actor: job.assignee,
            at: job.updated_at,
        })
    }
}
