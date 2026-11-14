//! **The bay refit's fee** (doc 31 cut UD.4b): the paid leg of a component
//! refit in a facility bay, the sibling of [`crate::core_refit`]. Accept
//! holds the quoted fee on the requester's account; Install captures it to
//! the bay's provider; Cancel releases it. Only the actual Economy
//! participant mints outputs; retained comparison data and catalogue labels
//! confer no authority, and the plan never touches a title or custody.
mod authority;
pub(crate) mod bridge;
mod consent;
mod leg;
mod subject;
#[cfg(test)]
mod tests;
pub use authority::BayRefitConfig;
pub use consent::{BayOfferAttribution, BayOrderAttribution, BayOwnerAuthority};
pub(crate) use install::{BayMoneyPatch, BoundBayMoney};
pub(crate) use money::ClosedBayPlan;
pub use money::{BayFundingOutput, BayTerminalOutput};
use mush_core::CommitToken;
pub use receipt::{BayFundingReceipt, BayPaymentHistory, BayTerminalReceipt, BayTerminalResult};
pub use subject::{BayPaymentPolicy, BaySite, BaySubject};

// The five owner files are `crate::refit_twin`'s, written once for both refits; these are
// their old paths, the bay's instantiation.
pub(crate) mod install {
    pub(crate) use crate::refit_twin::install::prepare;
    pub type BayMoneyPatch = crate::refit_twin::install::MoneyPatch<crate::refit_twin::Bay>;
    pub type BoundBayMoney<'a> = crate::refit_twin::install::BoundMoney<'a, crate::refit_twin::Bay>;
}
pub(crate) mod money {
    pub type Request = crate::refit_twin::money::Request<crate::refit_twin::Bay>;
    pub type Output = crate::refit_twin::money::Output<crate::refit_twin::Bay>;
    pub type ClosedBayPlan = crate::refit_twin::money::ClosedPlan<crate::refit_twin::Bay>;
    pub type BayFundingOutput = crate::refit_twin::money::FundingOutput<crate::refit_twin::Bay>;
    pub type BayTerminalOutput = crate::refit_twin::money::TerminalOutput<crate::refit_twin::Bay>;
}
mod plans {
    #[cfg(test)]
    pub(crate) use crate::refit_twin::plans::{CORRELATION_BYTES, funding, terminal};
    #[cfg(test)]
    pub(crate) fn decode_correlation(
        text: &str,
    ) -> Result<super::leg::Proof, super::BayRefitError> {
        crate::refit_twin::plans::decode_correlation::<crate::refit_twin::Bay>(text)
    }
    pub(crate) type FundingIntent = crate::refit_twin::plans::FundingIntent<crate::refit_twin::Bay>;
}
pub(crate) mod receipt {
    pub use super::leg::BayTerminalResult;
    pub use crate::refit_twin::receipt::FundingReceipt as BayFundingReceipt;
    pub type BayTerminalReceipt =
        crate::refit_twin::receipt::TerminalReceipt<crate::refit_twin::Bay>;
    pub type BayPaymentHistory = crate::refit_twin::receipt::PaymentHistory<crate::refit_twin::Bay>;
    pub(crate) type BayReceiptAudit =
        crate::refit_twin::receipt::ReceiptAudit<crate::refit_twin::Bay>;
}
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
pub type BayFundingRequest = crate::refit_twin::FundingRequest<crate::refit_twin::Bay>;
pub struct BayInstallationRequest {
    pub acceptance: CommitToken,
}
pub struct BayCancellationRequest {
    pub acceptance: CommitToken,
}
/// The owner's refusal (`Leg::REFUSED`), which the CT.0 snapshot pins.
#[cfg(test)]
pub(crate) fn refused() -> mush_core::participant::ParticipantError {
    crate::refit_twin::refused::<crate::refit_twin::Bay>()
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
