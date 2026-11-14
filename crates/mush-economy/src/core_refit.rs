//! Closed paid Install/Replace leg. Only the actual Economy participant mints
//! outputs; retained comparison data and catalogue labels confer no authority.
mod authority;
pub(crate) mod bridge;
mod consent;
mod leg;
pub(crate) use install::{BoundRefitMoney, RefitMoneyPatch};
pub(crate) use money::ClosedRefitPlan;
pub use money::{RefitFundingOutput, RefitTerminalOutput};
pub use receipt::{
    RefitFundingReceipt, RefitPaymentHistory, RefitTerminalReceipt, RefitTerminalResult,
};
#[cfg(test)]
mod ct0_snapshot;
mod subject;
#[cfg(test)]
mod tests;
pub use authority::CoreRefitConfig;
pub use consent::{RefitOfferAttribution, RefitOrderAttribution, RefitOwnerAuthority};
use mush_core::CommitToken;
pub use subject::{
    CorelessBase, RefitKind, RefitPaymentPolicy, RefitSite, RefitSubject, UngovernTerms,
    present_ungovern,
};

// The five owner files are `crate::refit_twin`'s, written once for both refits; these are
// their old paths, the core refit's instantiation.
pub(crate) mod install {
    pub(crate) use crate::refit_twin::install::prepare;
    pub type RefitMoneyPatch = crate::refit_twin::install::MoneyPatch<crate::refit_twin::Core>;
    pub type BoundRefitMoney<'a> =
        crate::refit_twin::install::BoundMoney<'a, crate::refit_twin::Core>;
}
pub(crate) mod money {
    pub type Request = crate::refit_twin::money::Request<crate::refit_twin::Core>;
    pub type Output = crate::refit_twin::money::Output<crate::refit_twin::Core>;
    pub type ClosedRefitPlan = crate::refit_twin::money::ClosedPlan<crate::refit_twin::Core>;
    pub type RefitFundingOutput = crate::refit_twin::money::FundingOutput<crate::refit_twin::Core>;
    pub type RefitTerminalOutput =
        crate::refit_twin::money::TerminalOutput<crate::refit_twin::Core>;
}
mod plans {
    #[cfg(test)]
    pub(crate) use crate::refit_twin::plans::{CORRELATION_BYTES, funding, terminal};
    #[cfg(test)]
    pub(crate) fn decode_correlation(text: &str) -> Result<super::leg::Proof, super::RefitError> {
        crate::refit_twin::plans::decode_correlation::<crate::refit_twin::Core>(text)
    }
    pub(crate) type FundingIntent =
        crate::refit_twin::plans::FundingIntent<crate::refit_twin::Core>;
}
pub(crate) mod receipt {
    pub use super::leg::RefitTerminalResult;
    pub use crate::refit_twin::receipt::FundingReceipt as RefitFundingReceipt;
    pub type RefitTerminalReceipt =
        crate::refit_twin::receipt::TerminalReceipt<crate::refit_twin::Core>;
    pub type RefitPaymentHistory =
        crate::refit_twin::receipt::PaymentHistory<crate::refit_twin::Core>;
    pub(crate) type RefitReceiptAudit =
        crate::refit_twin::receipt::ReceiptAudit<crate::refit_twin::Core>;
}
pub const OFFER_ACTION: &str = "offer_core_refit";
pub const ACCEPT_ACTION: &str = "accept_core_refit";
pub const INSTALL_ACTION: &str = "install_core_component";
pub const CANCEL_ACTION: &str = "cancel_core_refit";
pub(crate) const NAMESPACE: &str = "industry.core_refit.v1";
pub(crate) const HOLD_PURPOSE: &str = "industry.core_refit";
pub const FITTING_MINOR: i64 = 10_000;
/// The reservation, accepted to expiry: 28 IC days (7 until the owner's rule of 2026-11-04 that a
/// timer waiting on an absent player's response allows at least 28 IC days), the kernel audit's
/// [`mush_core::player_wait::REFIT_RESERVATION_SECONDS`].
pub const RESERVATION_SECONDS: i64 = mush_core::player_wait::REFIT_RESERVATION_SECONDS;
pub(crate) const MAX_ORDERS: usize = 8192;
pub(crate) const SUBJECT_BYTES: usize = 8192;
pub(crate) const HISTORY_BYTES: usize = 16_384;
pub type RefitFundingRequest = crate::refit_twin::FundingRequest<crate::refit_twin::Core>;
pub struct RefitInstallationRequest {
    pub acceptance: CommitToken,
}
pub struct RefitCancellationRequest {
    pub acceptance: CommitToken,
}
/// The owner's refusal (`Leg::REFUSED`), which the CT.0 snapshot pins.
#[cfg(test)]
pub(crate) fn refused() -> mush_core::participant::ParticipantError {
    crate::refit_twin::refused::<crate::refit_twin::Core>()
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
