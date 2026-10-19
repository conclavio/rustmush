//! Explicit organization-issued sealed purchases. Existing personal protocols
//! and immutable histories remain disjoint.
mod codec;
mod protocol;
mod purchase;
mod records;
pub use purchase::{HousePurchaseV1, HouseReceiptV1};
pub use records::{HouseListingV1, HouseProductV1, HouseQuoteV1, HouseSubjectV1};

mod accounts;
mod actions;
mod config;
mod guard;
mod offers;
mod provider;
mod publication;
pub use config::{HouseConfig, HousePolicy};
fn error(e: impl std::fmt::Display) -> mush_core::ModError {
    mush_core::ModError::msg(e.to_string())
}

mod audit;
mod families;
mod registration;
pub(crate) use families::Families;
pub(crate) use guard::HouseGuard;
pub(crate) use registration::{register, register_with_renewal_definitions};

#[cfg(test)]
mod tests;
