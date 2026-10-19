//! Closed optional native coverage owner. Explicit composition binds the exact
//! Tacsim actions; requests and decoded receipts alone supply no authority.
//! The paired producer owns actual publication, host proof and retained roots.
use mush_core::DefinitionRef;
use mush_core::participant::ParticipantError;

pub(crate) mod book;
pub(crate) mod bridge;
mod contract;
mod decision;
mod history;
pub(crate) mod install;
mod marker;
pub(crate) mod native;
#[cfg(test)]
mod owner_tests;
mod plans;
mod sparse;
mod subject;
pub use decision::{CoverageDecisionKindV1, CoverageDecisionV1};
pub use history::{CoverageBindingReceiptV1, CoverageHistory, CoverageTerminalReceiptV1};
pub use install::{BoundCoverage, CoveragePatch};
pub use marker::CoverageMarkerV1;
pub(crate) use native::ClosedCoveragePlan;
pub use native::{CoverageBindRequest, CoverageConfig, CoverageOutput, CoverageTerminalRequest};
pub use subject::{CoverageBasisV1, CoverageConsentV1, CoverageMoneyTermsV1, CoverageQuoteTermsV1};

pub const MAX_SUBJECT_BYTES: usize = 8192;
pub const MAX_DECISION_BYTES: usize = 4096;
pub const MAX_CONTRACT_BYTES: usize = 16384;
pub const MAX_TITLE_BYTES: usize = 8192;
pub const MAX_CORRELATION_BYTES: usize = 32768;
pub const MAX_HISTORY_BYTES: usize = 65536;
pub(crate) const NAMESPACE: &str = "tacsim.coverage.v1";
pub const BIND_ACTION: &str = "bind_hull_coverage";
pub const CLAIM_ACTION: &str = "claim_hull_coverage";
pub const RELEASE_ACTION: &str = "release_hull_coverage";

fn action_matches(action: &DefinitionRef, id: &str) -> bool {
    action.key().owner.as_str() == "tacsim"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && action.version() == 1
}

fn refused() -> ParticipantError {
    ParticipantError::refused(
        "The coverage terms or committed evidence are unavailable or have changed.",
    )
}
