//! Closed money/title leg for genuine Industry core integration.
//!
//! This purpose does not reuse Mind purchase consent or commodity work orders.
//! Only the existing Economy owner participant will construct its outputs.
mod authority;
pub(crate) mod bridge;
mod consent;
mod delivery;
pub use bridge::CoreTitleBirth;
pub use delivery::require_core_delivery;
mod fold;
pub(crate) mod install;
pub(crate) mod money;
mod plans;
pub(crate) mod receipt;
mod subject;
pub use authority::CoreIntegrationConfig;
pub use consent::{CoreOfferAttribution, CoreOfferAuthority};
pub use plans::CoreOutputBirth;
pub use receipt::{
    CoreFundingReceipt, CorePaymentHistory, CoreTerminalReceipt, CoreTerminalResult,
};
pub use subject::{
    CoreClass, CoreDeliveryPromise, CoreFundingSubject, CorePaymentPolicy, DieSource,
    IntegrationSite, ScopedCoreAccount, UngovernedIntegration,
};

use mush_core::CommitToken;
use mush_core::participant::ParticipantError;

pub const ACCEPT_ACTION: &str = "accept_core_integration";
pub const COMPLETE_ACTION: &str = "complete_core_integration";
pub const CANCEL_ACTION: &str = "cancel_core_integration";
pub(crate) const NAMESPACE: &str = "industry.core_integration.v1";
pub(crate) const HOLD_PURPOSE: &str = "industry.core_integration";
pub(crate) const MAX_ORDERS: usize = 8192;
pub(crate) const SUBJECT_BYTES: usize = 8192;
pub(crate) const HISTORY_BYTES: usize = 16_384;

/// Data from the trusted Industry provider; certification and Inventory's exact
/// lot remain independently required owner legs in the same acknowledgement.
pub struct CoreFundingRequest {
    pub subject: CoreFundingSubject,
}

/// Completion takes the actual opaque Core output prepared by this same action.
/// No decoded object ID or caller-written object postimage is accepted instead.
pub struct CoreCompletionRequest {
    pub acceptance: CommitToken,
    pub output: mush_core::core_output::ReservedCoreOutput,
}

/// Cleanup names the immutable admission, not a current configuration or price.
pub struct CoreCancellationRequest {
    pub acceptance: CommitToken,
}

pub(crate) fn refused() -> ParticipantError {
    ParticipantError::Refused("core integration money or title disagrees".into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CoreIntegrationError {
    #[error("core integration evidence is malformed")]
    Invalid,
    #[error("core integration evidence disagrees")]
    Integrity,
    #[error("core integration history is incomplete")]
    Incomplete,
    #[error("core integration authority is unavailable")]
    Unavailable,
    #[error("core integration capacity is exhausted")]
    Capacity,
}

pub(crate) use install::{BoundCoreMoney, CoreMoneyPatch};
pub(crate) use money::ClosedCorePlan;
pub use money::{CoreFundingOutput, CoreTerminalOutput};
