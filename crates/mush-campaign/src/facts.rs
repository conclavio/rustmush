//! The `campaign.*` boundary fact (E7 cut 9 B7; D-E7-30, 10b §10).
//!
//! **One namespace, and only one.** `EconomyModule::with_facts` panics on a
//! duplicate, and D-E7-30 gives `mush-campaign` exactly `campaign.*` — the fifth
//! and last crate on that list. Cut 9 registers no second provider, and cut 10
//! registers none at all.
//!
//! **Answered from an applied marker, never a status** (08c §6.2, the E4c verify
//! fold's E3 finding). `OperationRuntime::root` exists only once the terminal batch
//! ran — the document write and the `mush_canon::commit` that pins it — so a
//! contract gated on `campaign.operation.resolved` cannot settle from an operation
//! whose resolution merely *started*. An operation whose status says `Resolving`
//! but whose root marker is absent answers `None`, which is unknown, which is
//! unsatisfied, which never settles money.

use mush_core::World;
use mush_economy::{FactProvider, FactState};

use crate::ids::OperationId;
use crate::settle::KIND_OPERATION_RESOLVED;
use crate::store::{CampaignStore, NS};

/// The one fact provider `mush-campaign` registers.
#[derive(Clone, Copy, Debug, Default)]
pub struct CampaignFactProvider;

impl FactProvider for CampaignFactProvider {
    fn provider(&self) -> &str {
        NS
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        // A revision is a *stale-reference* guard, not a version of the fact: a
        // campaign root marker is written once and never amended, so any named
        // revision other than 1 references a world this crate never had.
        if revision.is_some_and(|revision| revision != 1) {
            return None;
        }
        match kind {
            KIND_OPERATION_RESOLVED => {
                let operation = OperationId::parse(id)?;
                let doc = w.module_state::<CampaignStore>()?.operation(operation)?;
                // The APPLIED marker, never the status. `Resolving` without a root
                // answers unknown.
                let root = doc.runtime.root.as_ref()?;
                Some(FactState {
                    satisfied: true,
                    // The campaign layer cannot say which *object* brought a
                    // resolution about — the participants are units, not actors —
                    // and inventing one would be attribution nobody recorded.
                    actor: None,
                    at: root.at,
                })
            }
            _ => None,
        }
    }
}
