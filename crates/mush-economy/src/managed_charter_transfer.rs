//! Closed optional pending charter native owner. Actual buyer funding, retained
//! native journal history and terminal installation use the existing Economy
//! slot. Paired Roll claim/sanction custody and Core future capacity are required
//! before a production producer can expose this branch.

use mush_core::DefinitionRef;
use mush_core::participant::ParticipantError;

pub(crate) mod bridge;
mod contract;
mod decision;
pub(crate) mod history;
pub(crate) mod install;
mod marker;
pub(crate) mod native;
mod plans;
mod sparse;
mod subject;
pub use decision::{CharterPendingDecisionKindV1, CharterPendingDecisionV1};
pub use history::{
    CharterPendingFundingReceiptV1, CharterPendingHistory, CharterPendingTerminalReceiptV1,
};
pub use install::{BoundCharterPending, CharterPendingPatch};
pub use marker::CharterPendingMarkerV1;
pub(crate) use native::ClosedCharterPendingPlan;
pub use native::{
    CharterPendingConfig, CharterPendingFundingRequest, CharterPendingOutput,
    CharterPendingTerminalRequest,
};
pub use subject::{
    CharterOwnershipBasisV1, CharterPendingConsentV1, CharterPendingTermsV1, CharterTermBasisV1,
};

pub const MAX_CHARTER_PENDING_SUBJECT_BYTES: usize = 4096;
pub const MAX_CHARTER_PENDING_OFFER_SECONDS: i64 = 86400;
pub const MAX_CHARTER_SETTLEMENT_IC_SECONDS: i64 = 259200;
pub const MAX_CHARTER_OWNERSHIP_GENERATION: u64 = 65537;
pub const MAX_PENDING_CONTRACT_BYTES: usize = 8192;
pub const MAX_PENDING_CORRELATION_BYTES: usize = 8192;
pub const MAX_PENDING_HISTORY_BYTES: usize = 32768;
pub(crate) const NAMESPACE: &str = "roll.charter_pending.v1";
pub(crate) const OFFER_ACTION: &str = "offer_mind_charter_transfer";
pub(crate) const FUND_ACTION: &str = "fund_mind_charter_transfer";
pub(crate) const SETTLE_ACTION: &str = "settle_mind_charter_transfer";
pub(crate) const REFUND_ACTION: &str = "refund_mind_charter_transfer";

fn refused() -> ParticipantError {
    ParticipantError::refused("The pending charter transfer is unavailable or has changed.")
}
fn action_matches(action: &DefinitionRef, id: &str, version: u32) -> bool {
    action.key().owner.as_str() == "roll"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && action.version() == version
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod owner_tests;
