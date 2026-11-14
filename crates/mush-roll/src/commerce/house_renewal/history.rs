//! Exact immutable F0/journal provenance. A terminal contract or decoded
//! receipt is never substituted for the Economy owner's completed audit.
//!
//! Doc 46 CT.5: written once in `renewal_twin::history`.
use super::super::records::PublicationError;
use super::super::renewal_twin::House;

/// Shared data relation, also used by the Market pair (`renewal_twin::history`).
#[allow(dead_code)]
pub(crate) fn check_offer(
    row: &super::RenewalOperationV1,
    offer: &<House as mush_economy::renewal_twin::Issuer>::Offer,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::history::check_offer::<House>(row, offer)
}

/// Borrowed evidence can only be obtained from actual EconomyStore replay.
pub(crate) fn verify(
    row: &super::RenewalOperationV1,
    actual: &mush_economy::renewal_twin::RenewalOfferView<'_, House>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::history::verify::<House>(row, actual)
}

/// Historical term proof after immutable purchase birth has been verified.
/// The caller separately validates the complete attempt/head graph at boot.
pub(crate) fn granted_generation(
    ctx: &impl crate::commerce::read::CommerceRead,
    f: &crate::commerce::families::CommerceFamilies,
    licence: &crate::commerce::paid::PersonalPaidLicence,
    term: &crate::commerce::paid::PaidTermRecord,
    birth: &crate::commerce::house_purchase::HouseAllocationV1,
    economy: &mush_economy::EconomyStore,
) -> Result<mush_core::CommitToken, PublicationError> {
    use crate::commerce::{entitlement::history::read, records::Record};
    let row = read(ctx, &f.house_renewal_operations, term.operation().to_hex())?;
    let economics = read(ctx, &f.house_economics, term.key())?;
    let view = economy
        .house_renewal_audit()
        .map_err(|_| PublicationError::Unavailable)?
        .get(row.operation())
        .map_err(|_| PublicationError::Missing)?;
    verify(row, &view)?;
    let (_, expected_term, expected_economics) = row.granted_rows()?;
    let b = row.binding();
    if !b.matches_birth(licence)
        || b.issuer() != birth.issuer()
        || term != &expected_term
        || economics != &expected_economics
        || b.product_digest() != birth.product_digest()
        || b.grant() != birth.economics().grant()
        || view.offer().basis.prior_quote != birth.quote()
        || view.offer().basis.prior_quote_digest != birth.quote_digest()
    {
        return Err(PublicationError::Mismatch);
    }
    let prior = read(
        ctx,
        &f.paid_terms,
        format!("{}/{}", licence.id().key(), b.predecessor().key()),
    )?;
    let prior_economics = read(ctx, &f.house_economics, prior.key())?;
    b.check_predecessor(licence, prior, prior_economics)?;
    row.state()
        .terminal_commit()
        .ok_or(PublicationError::Missing)
}
