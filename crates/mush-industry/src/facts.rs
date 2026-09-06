//! E7c3 B7 — the `industry.*` fact provider (§10, D-E7-30).
//!
//! D-E7-30 gives this crate exactly **one** namespace and
//! `EconomyModule::with_facts` **panics** on a duplicate, so registering it here
//! is also what proves the rule for cut 4: `industry.workorder.completed` is
//! **cut 4's arm on this same provider**, never a second registration.
//!
//! **Answered from the committed bundle's own applied marker, never from a
//! status** (§10's E4c E3 finding: `jobs.resolved` read a status a staffer could
//! set by hand). A bundle that did not commit leaves no marker and the fact
//! answers `None`, which is unsatisfied — and `None` never settles money.
//!
//! **Why a mirror.** `FactProvider::resolve` receives `&World`, while typed
//! state lives on the `Engine`, so a fact provider on this tree cannot read a
//! typed family at all. Cut 3 mirrors the marker family into
//! [`IndustryStore`](crate::store::IndustryStore) at boot and on the coarse
//! sweep, exactly as cut 2 does, with the same direction guarantee: the mirror
//! is **behind** the authoritative truth by at most one sweep and never ahead,
//! and a mirror that could not see the whole family answers **nothing at all**.
//! Cut 2 recorded widening `FactProvider`/`WorldFacts` to carry the typed-state
//! store as cut 3's ask; cut 3 records it again as cut 4's, because the seam
//! change is a kernel one and this cut's budget went to the cross-owner write.

use mush_core::World;
use mush_economy::{FactProvider, FactState};

use crate::record::{FACT_EXTRACTION_CYCLED, FACT_WORKORDER_COMPLETED, NS};
use crate::store::IndustryStore;

/// The one `industry.*` provider this crate registers.
#[derive(Clone, Copy, Debug, Default)]
pub struct IndustryFactProvider;

impl FactProvider for IndustryFactProvider {
    fn provider(&self) -> &str {
        NS
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        // A revision is a *stale-reference* guard, not a version of the fact: an
        // industry marker is written once and never amended, so any named
        // revision other than 1 references a world this crate never had.
        if revision.is_some_and(|revision| revision != 1) {
            return None;
        }
        match kind {
            FACT_EXTRACTION_CYCLED => {}
            // Cut 4's arm, on this same provider and this same namespace.
            // Until cut 4 writes its marker the honest answer is "unknown",
            // which is unsatisfied.
            FACT_WORKORDER_COMPLETED => return None,
            _ => return None,
        }
        let store = w.module_state::<IndustryStore>()?;
        // A truncated mirror cannot distinguish "no such fact" from "the fact is
        // past the page budget", so it answers neither.
        if !store.is_complete() {
            return None;
        }
        let marker = store.marker(id)?;
        if marker.kind() != kind {
            return None;
        }
        Some(FactState {
            satisfied: true,
            actor: Some(marker.actor()),
            at: marker.at(),
        })
    }
}
