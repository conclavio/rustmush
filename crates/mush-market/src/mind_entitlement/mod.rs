//! Explicit, separately versioned personal entitlement commerce.
//! Default composition installs none of its families, providers or owner grants.
mod accounts;
mod actions;
mod audit;
mod capabilities;
mod codec;
mod config;
mod families;
mod guard;
mod offers;
mod protocol;
mod provider;
mod publication;
mod purchase;
mod records;
mod registration;

pub(crate) use capabilities::register as register_capabilities;
pub use config::{EntitlementConfig, EntitlementPolicy};
pub(crate) use families::Families;
pub(crate) use registration::register;

fn error(e: impl std::fmt::Display) -> mush_core::ModError {
    mush_core::ModError::msg(e.to_string())
}

pub use purchase::{EntitlementPurchaseV1, EntitlementReceiptV1};
pub use records::{
    EntitlementListingV1, EntitlementProductV1, EntitlementQuoteV1, EntitlementSubjectV1,
};

#[cfg(test)]
mod tests;
