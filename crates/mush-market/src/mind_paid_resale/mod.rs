//! Optional paid-property resale. Codecs alone install no service or authority.
mod accounts;
mod actions;
mod audit;
mod compound;
mod config;
mod families;
mod index;
mod offer;
mod provider;
mod records;
mod registration;
mod service_provider;
pub(crate) use audit::require_disabled;
pub use config::PaidResaleConfig;
pub(crate) use families::Families;
// 17g C2g: the per-(buyer, licence) index `mind_commands` point-reads.
pub(crate) use index::ResaleBuyerOffers;
pub use records::{
    MindPaidResaleOffer, MindPaidResaleOperation, MindPaidResaleReceipt, ResaleOutcome,
};
pub(crate) use registration::register;

#[cfg(test)]
mod tests;

/// 17g C2a (C2 plan §2.3): the per-kind verbs (Offer, Accept, Fund), for the
/// crate's split test beside the v2 market's.
#[cfg(test)]
pub(crate) fn split_verbs() -> [&'static str; 3] {
    [
        provider::Kind::Offer,
        provider::Kind::Accept,
        provider::Kind::Fund,
    ]
    .map(|kind| actions::presentation(kind).0)
}

/// 17g C2g (the command read's `ResaleOffers` tests): the seller offer `id`
/// of `licence` naming `buyer` (`buyer_offers::offer`'s binding and seller
/// 7), at `price_minor`, expiring at `expires_at_utc` (offered 1,000 seconds
/// before).
#[cfg(test)]
pub(crate) fn command_offer(
    id: u8,
    buyer: mush_core::ObjId,
    licence: u64,
    price_minor: i64,
    expires_at_utc: i64,
) -> Result<MindPaidResaleOffer, Box<dyn std::error::Error>> {
    let mut row = *tests::buyer_offers::offer(id, buyer, licence, expires_at_utc - 1000)?;
    row.terms.price = mush_economy::Amount::from_minor(price_minor);
    row.validate()?;
    Ok(row)
}

/// 17g C2g: `buyer`'s `(buyer, licence)` offer index row over `offers`.
#[cfg(test)]
pub(crate) fn command_buyer_offers(
    buyer: mush_core::ObjId,
    licence: u64,
    offers: &[mush_core::CommitToken],
) -> Result<ResaleBuyerOffers, Box<dyn std::error::Error>> {
    let row = ResaleBuyerOffers {
        format: 1,
        buyer,
        licence: mush_roll::commerce::LicenceId::new(licence)?,
        offers: offers.iter().copied().collect(),
    };
    row.validate()?;
    Ok(row)
}
