//! Closed source-free resale of an existing paid instrument. These structural
//! values never grant holder, claim, service or Registry authority. Actual
//! paired owner installation and complete journal reconstruction are required.

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
mod plans;
mod sparse;
mod subject;
pub use decision::{PaidResaleDecisionKindV1, PaidResaleDecisionV1};
pub use history::{
    PaidResaleAcceptanceReceiptV1, PaidResaleHistoryV1, PaidResaleOutcome,
    PaidResaleTerminalReceiptV1,
};
pub use install::{BoundPaidResale, PaidResalePatch};
pub use marker::PaidResaleMarkerV1;
pub(crate) use native::ClosedPaidResalePlan;
pub use native::{
    PaidResaleAcceptanceRequest, PaidResaleConfig, PaidResaleOutput, PaidResaleTerminalRequest,
};
pub use subject::{
    PaidBirthRefV1, PaidBirthRouteV1, PaidHolderRefV1, PaidResaleBasisV1, PaidResaleClaimsV1,
    PaidResaleConsentV1, PaidResaleModeV1, PaidResaleTermBasisV1, PaidResaleTermsV1,
    PaidResaleWindowV1,
};

pub const MAX_SUBJECT_BYTES: usize = 8192;
pub const MAX_CORRELATION_BYTES: usize = 16384;
pub const MAX_CONTRACT_BYTES: usize = 16384;
pub const MAX_HISTORY_BYTES: usize = 32768;
pub const MAX_HOLDER_GENERATION: u64 = 65537;
pub const MAX_OFFER_SECONDS: i64 = 86400;
pub const SETTLEMENT_IC_SECONDS: i64 = 259200;
pub(crate) const NAMESPACE: &str = "market.paid_resale.v1";
pub(crate) const OFFER: &str = "offer_paid_mind_resale";
pub(crate) const ACCEPT: &str = "accept_paid_mind_resale";
pub(crate) const FUND: &str = "fund_paid_mind_resale";
pub(crate) const SETTLE: &str = "settle_paid_mind_resale";
pub(crate) const REFUND: &str = "refund_paid_mind_resale";

fn refused() -> ParticipantError {
    ParticipantError::refused("The paid licence resale is unavailable or has changed.")
}
fn action_matches(action: &DefinitionRef, id: &str) -> bool {
    action.key().owner.as_str() == "market"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && action.version() == 1
}
fn registry_action(action: &DefinitionRef) -> bool {
    action.key().owner.as_str() == "roll"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == "registry_term_revocation"
        && action.version() == 1
}

#[cfg(test)]
mod owner_tests;
#[cfg(test)]
mod tests;
