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
