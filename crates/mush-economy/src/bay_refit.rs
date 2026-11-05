//! **The bay refit's fee** (doc 31 cut UD.4b): the paid leg of a component
//! refit in a facility bay, the sibling of [`crate::core_refit`]. Accept
//! holds the quoted fee on the requester's account; Install captures it to
//! the bay's provider; Cancel releases it. Only the actual Economy
//! participant mints outputs; retained comparison data and catalogue labels
//! confer no authority, and the plan never touches a title or custody.
mod authority;
pub(crate) mod bridge;
mod consent;
mod fold;
pub(crate) mod install;
pub(crate) mod money;
mod plans;
pub(crate) mod receipt;
mod subject;
#[cfg(test)]
mod tests;
pub use authority::BayRefitConfig;
pub use consent::{BayOfferAttribution, BayOrderAttribution, BayOwnerAuthority};
pub(crate) use install::{BayMoneyPatch, BoundBayMoney};
pub(crate) use money::ClosedBayPlan;
pub use money::{BayFundingOutput, BayTerminalOutput};
use mush_core::CommitToken;
use mush_core::participant::ParticipantError;
pub use receipt::{BayFundingReceipt, BayPaymentHistory, BayTerminalReceipt, BayTerminalResult};
pub use subject::{BayPaymentPolicy, BaySite, BaySubject};
pub const OFFER_ACTION: &str = "offer_bay_refit";
pub const ACCEPT_ACTION: &str = "accept_bay_refit";
pub const INSTALL_ACTION: &str = "install_bay_refit";
pub const CANCEL_ACTION: &str = "cancel_bay_refit";
pub(crate) const NAMESPACE: &str = "industry.bay_refit.v1";
pub(crate) const HOLD_PURPOSE: &str = "industry.bay_refit";
/// The Org capability an offering operator needs on the bay's scope.
pub const OPERATOR_CAPABILITY: &str = "industry.bay.refit";
/// The reservation, accepted to expiry: 28 IC days (7 until the owner's rule of 2026-11-04 that a
/// timer waiting on an absent player's response allows at least 28 IC days), the kernel audit's
/// [`mush_core::player_wait::REFIT_RESERVATION_SECONDS`].
pub const RESERVATION_SECONDS: i64 = mush_core::player_wait::REFIT_RESERVATION_SECONDS;
pub(crate) const MAX_ORDERS: usize = 8192;
pub(crate) const SUBJECT_BYTES: usize = 8192;
pub(crate) const HISTORY_BYTES: usize = 16_384;
pub struct BayFundingRequest {
    pub subject: BaySubject,
}
pub struct BayInstallationRequest {
    pub acceptance: CommitToken,
}
pub struct BayCancellationRequest {
    pub acceptance: CommitToken,
}
pub(crate) fn refused() -> ParticipantError {
    ParticipantError::Refused("bay refit money or accepted authority disagrees".into())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BayRefitError {
    #[error("bay refit evidence is malformed")]
    Invalid,
    #[error("bay refit evidence disagrees")]
    Integrity,
    #[error("bay refit history is incomplete")]
    Incomplete,
    #[error("bay refit authority is unavailable")]
    Unavailable,
    #[error("bay refit capacity is exhausted")]
    Capacity,
}
