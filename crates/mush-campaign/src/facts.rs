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
//!
//! **That sentence was true of this header and false of the code until the E7 cut 9
//! independent verify fold (D6).** `settle_root` queued the marker *before* the
//! commit and never inspected its `Option<CanonId>` except to backfill, so a commit
//! that produced nothing still answered this fact as satisfied. The marker is
//! written only after a successful commit and carries its id now, and this provider
//! additionally requires `root.canon` to be set. Two further refusals the fold added:
//! a **covert** operation answers `None` (B12) — matching the `CanonScope::Secret`
//! the same resolution writes, so a contract condition cannot be an existence and
//! timing oracle for one — and an **abandonment** answers `None` (D7), because
//! abandoning is not settling on the consequence path either.

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
                // B12: **a covert operation answers nothing at all.** `scope_for`
                // records a `Staff`-visibility resolution as `CanonScope::Secret`
                // precisely so nobody outside learns it happened — and then this
                // provider answered `satisfied` to any contract condition naming it,
                // so a `fact` condition was an existence and timing oracle whose
                // settlement fired the moment the covert operation resolved.
                if matches!(
                    doc.terms.visibility,
                    mush_economy::EconomicVisibility::Staff
                ) {
                    return None;
                }
                // D7: **an abandonment is not a settlement.** Cut 9 answered
                // `satisfied` for any root marker whatever its outcome, so an
                // operation staff had to abandon past a corrupt surface settled a
                // linked contract exactly as a completed one would.
                if doc.runtime.abandoned {
                    return None;
                }
                // The APPLIED marker, never the status — and since E4(a) the marker
                // exists only once `mush_canon::commit` returned an id, so "the
                // document write AND the commit that pins it" is true of the code as
                // well as of this header.
                let root = doc.runtime.root.as_ref().filter(|r| r.canon.is_some())?;
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
