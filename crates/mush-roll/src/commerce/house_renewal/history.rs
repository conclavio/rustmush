//! Exact immutable F0/journal provenance. A terminal contract or decoded
//! receipt is never substituted for the Economy owner's completed audit.

use super::super::records::PublicationError;
use super::records::digest;
use super::{RenewalAttemptV1, RenewalOperationV1, RenewalStateV1};
use mush_economy::managed_house_renewal::{RenewalOffer, RenewalOfferView};

/// Shared data relation, also used by the Market pair. The owner producer must
/// additionally prove the actual birth quote, current issuer and account facts.
pub(crate) fn check_offer(
    row: &RenewalOperationV1,
    offer: &RenewalOffer,
) -> Result<(), PublicationError> {
    row.validate()?;
    offer.validate().map_err(|_| PublicationError::Invalid)?;
    let b = &row.binding;
    let basis = &offer.basis;
    let template = b.grant.renewal();
    if basis.licence != b.licence.get()
        || basis.predecessor_generation != b.predecessor.get()
        || basis.next_generation != b.next.get()
        || basis.predecessor_operation != b.predecessor_operation
        || basis.product_digest != b.product_digest
        || basis.template_digest != b.template_digest()?
        || basis.grant_economics_digest != b.grant.digest()?
        || offer.terms.buyer != b.holder
        || offer.terms.organization != b.issuer.organization()
        || offer.terms.scope != b.issuer.scope()
        || offer.terms.charter != mush_core::ObjId(b.charter.get())
        || &offer.terms.instrument != template.instrument()
        || offer.terms.price != template.price()
        || digest("economy.house_mind_renewal_offer.v1", offer)? != row.offer_digest
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// Borrowed evidence can only be obtained from actual EconomyStore replay.
/// No current seller/title/parent or current pointer requirement is imposed.
pub(crate) fn verify(
    row: &RenewalOperationV1,
    actual: &RenewalOfferView<'_>,
) -> Result<(), PublicationError> {
    check_offer(row, actual.offer())?;
    let attempt = RenewalAttemptV1::from_economy(actual.attempt());
    let format = match attempt {
        RenewalAttemptV1::Automatic {} => 1,
        RenewalAttemptV1::BuyerRetry { .. } => 2,
    };
    if actual.format() != format
        || row.attempt != attempt
        || actual.operation() != row.operation
        || actual.action() != &row.action
        || actual.contract() != row.contract
        || actual.consent_hash() != row.consent_hash
        || actual.created_at_ic() != &row.created_at_ic
        || actual.created_at_utc() != row.created_at_utc
    {
        return Err(PublicationError::Mismatch);
    }
    if let Some(service) = actual.service_closed() {
        return match &row.state {
            RenewalStateV1::UnfundedServiceClosed { closure }
                if closure == service
                    && actual.history().is_none()
                    && actual.closure().is_none() =>
            {
                Ok(())
            }
            _ => Err(PublicationError::Mismatch),
        };
    }
    match (&row.state, actual.history(), actual.closure()) {
        (RenewalStateV1::Drafted {}, None, None) => Ok(()),
        (RenewalStateV1::UnfundedClosed { closure }, None, Some(a))
            if a.commit() == closure.commit
                && a.action() == &closure.action
                && a.at_ic() == &closure.at_ic
                && a.at_utc() == closure.posted_at_utc
                && a.reason() == closure.reason =>
        {
            Ok(())
        }
        (RenewalStateV1::Funded { funding }, Some(a), None)
            if a.operation() == row.operation
                && a.funding() == funding
                && a.terminal().is_none() =>
        {
            Ok(())
        }
        (
            RenewalStateV1::Granted { funding, terminal }
            | RenewalStateV1::Refunded { funding, terminal },
            Some(a),
            None,
        ) if a.operation() == row.operation
            && a.funding() == funding
            && a.terminal() == Some(terminal) =>
        {
            Ok(())
        }
        _ => Err(PublicationError::Mismatch),
    }
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
