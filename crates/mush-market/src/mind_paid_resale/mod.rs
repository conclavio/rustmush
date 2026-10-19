//! Optional paid-property resale. Codecs alone install no service or authority.
mod accounts;
mod actions;
mod audit;
mod compound;
mod config;
mod families;
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
