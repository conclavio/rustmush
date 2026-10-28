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
mod index;
mod offers;
mod protocol;
mod provider;
mod publication;
mod purchase;
mod records;
mod registration;

pub(crate) use capabilities::register_with_house as register_capabilities_with_house;
pub use config::{EntitlementConfig, EntitlementPolicy};
pub(crate) use families::Families;
// 17g C2c: the command read's (`crate::mind_commands`) view of this owner.
pub(crate) use guard::EntitlementGuard;
// 17g C2g: the per-buyer index the own-purchase reads point-read.
pub(crate) use index::{EntitlementActorIndex, MAX_BUYER_PURCHASES, MAX_BUYER_QUOTES};
pub(crate) use offers::current_policy;
pub(crate) use registration::register_with_house_definitions;

fn error(e: impl std::fmt::Display) -> mush_core::ModError {
    mush_core::ModError::msg(e.to_string())
}

pub use purchase::{EntitlementPurchaseV1, EntitlementReceiptV1};
pub use records::{
    EntitlementListingV1, EntitlementProductV1, EntitlementQuoteV1, EntitlementSubjectV1,
};

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
pub(crate) mod tests;
