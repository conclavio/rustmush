//! Closed paid Install/Replace leg. Only the actual Economy participant mints
//! outputs; retained comparison data and catalogue labels confer no authority.
mod authority;
pub(crate) mod bridge;
mod consent;
mod fold;
pub(crate) mod install;
pub(crate) mod money;
mod plans;
pub(crate) mod receipt;
pub(crate) use install::{BoundRefitMoney, RefitMoneyPatch};
pub(crate) use money::ClosedRefitPlan;
pub use money::{RefitFundingOutput, RefitTerminalOutput};
pub use receipt::{
    RefitFundingReceipt, RefitPaymentHistory, RefitTerminalReceipt, RefitTerminalResult,
};
mod subject;
pub use authority::CoreRefitConfig;
pub use consent::{RefitOfferAttribution, RefitOrderAttribution, RefitOwnerAuthority};
use mush_core::CommitToken;
use mush_core::participant::ParticipantError;
pub use subject::{
    CorelessBase, RefitKind, RefitPaymentPolicy, RefitSite, RefitSubject, UngovernTerms,
    present_ungovern,
};
pub const OFFER_ACTION: &str = "offer_core_refit";
pub const ACCEPT_ACTION: &str = "accept_core_refit";
pub const INSTALL_ACTION: &str = "install_core_component";
pub const CANCEL_ACTION: &str = "cancel_core_refit";
pub(crate) const NAMESPACE: &str = "industry.core_refit.v1";
pub(crate) const HOLD_PURPOSE: &str = "industry.core_refit";
pub const FITTING_MINOR: i64 = 10_000;
pub const RESERVATION_SECONDS: i64 = 7 * 86_400;
pub(crate) const MAX_ORDERS: usize = 8192;
pub(crate) const SUBJECT_BYTES: usize = 8192;
pub(crate) const HISTORY_BYTES: usize = 16_384;
pub struct RefitFundingRequest {
    pub subject: RefitSubject,
}
pub struct RefitInstallationRequest {
    pub acceptance: CommitToken,
}
pub struct RefitCancellationRequest {
    pub acceptance: CommitToken,
}
pub(crate) fn refused() -> ParticipantError {
    ParticipantError::Refused("core refit money or accepted authority disagrees".into())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RefitError {
    #[error("core refit evidence is malformed")]
    Invalid,
    #[error("core refit evidence disagrees")]
    Integrity,
    #[error("core refit history is incomplete")]
    Incomplete,
    #[error("core refit authority is unavailable")]
    Unavailable,
    #[error("core refit capacity is exhausted")]
    Capacity,
}
