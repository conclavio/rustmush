//! Exact immutable F0/journal provenance. A terminal contract or decoded
//! receipt is never substituted for the Economy owner's completed audit.
//!
//! Doc 46 CT.5: written once in `renewal_twin::history`.
use super::super::records::PublicationError;
use super::super::renewal_twin::Personal;

/// Shared data relation, also used by the Market pair (`renewal_twin::history`).
#[allow(dead_code)]
pub(crate) fn check_offer(
    row: &super::RenewalOperationV1,
    offer: &<Personal as mush_economy::renewal_twin::Issuer>::Offer,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::history::check_offer::<Personal>(row, offer)
}

/// Borrowed evidence can only be obtained from actual EconomyStore replay.
pub(crate) fn verify(
    row: &super::RenewalOperationV1,
    actual: &mush_economy::renewal_twin::RenewalOfferView<'_, Personal>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::history::verify::<Personal>(row, actual)
}
