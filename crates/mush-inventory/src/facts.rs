//! E7c2 B7 / OPEN 5 — the `inventory.*` fact provider.
//!
//! D-E7-30 gives this crate exactly one namespace and `EconomyModule::with_facts`
//! **panics** on a duplicate, so registering it here is also what proves the rule
//! for cuts 3–9.
//!
//! **Answered from an applied marker, never from a status** (§10's E4c E3
//! finding: `jobs.resolved` read a status a staffer could set by hand). The
//! marker is written *inside the authoritative bundle* by the provider that did
//! the work, so a bundle that did not commit leaves no marker and the fact
//! answers `None` — and `None` never settles money.
//!
//! **Why a mirror.** `FactProvider::resolve` receives `&World`, while typed state
//! lives on the `Engine`, so a provider on this tree cannot read a typed family
//! at all. Cut 2 therefore mirrors the marker family into
//! [`InventoryStore`](crate::store::InventoryStore) at boot, on the coarse sweep
//! tick, and at the top of every `@inv` command. The mirror is **behind** the
//! authoritative truth by at most one of those and never ahead, which is the
//! direction that matters: `facts.rs`'s own contract is *unknown is unsatisfied,
//! everywhere*, so a settlement waits rather than paying early. Widening the seam
//! to carry the typed-state store is recorded as an ask in 10b §3.2's "As built".

use mush_core::World;
use mush_economy::{FactProvider, FactState};

use crate::record::{FACT_LOT_CREATED, FACT_TRANSFERRED, NS};
use crate::store::InventoryStore;

/// The one `inventory.*` provider this crate registers.
#[derive(Clone, Copy, Debug, Default)]
pub struct InventoryFactProvider;

impl FactProvider for InventoryFactProvider {
    fn provider(&self) -> &str {
        NS
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        // A revision is a *stale-reference* guard, not a version of the fact:
        // an inventory marker is written once and never amended, so any named
        // revision other than 1 is a reference to a world this crate never had.
        if revision.is_some_and(|revision| revision != 1) {
            return None;
        }
        if kind != FACT_TRANSFERRED && kind != FACT_LOT_CREATED {
            return None;
        }
        let store = w.module_state::<InventoryStore>()?;
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
