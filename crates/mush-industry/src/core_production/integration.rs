//! Actual certified-input manufacturing, separate from pooled commodity orders.
//! Optional composition will reuse the existing Industry owner participant.
pub(crate) mod audit;
mod authority;
mod config;
pub(crate) mod delivery;
pub(crate) mod families;
mod offer;
pub(super) mod output_policy;
pub(crate) mod owner;
pub(crate) mod records;
pub(crate) mod runtime;
pub(crate) mod state;
pub use config::CoreFactoryConfig;
mod progress;
mod recipe;
pub use recipe::{CoreRecipeAuthority, PublishedCoreRecipe};

pub(crate) const INITIALIZE: &str = "initialize_core_production";
pub(crate) const OFFER: &str = "offer_core_integration";
pub(crate) const ACTIONS: [&str; 7] = [
    INITIALIZE, OFFER, ACCEPT, RESERVE, ADVANCE, COMPLETE, CANCEL,
];
pub(crate) const ACCEPT: &str = "accept_core_integration";
pub(crate) const RESERVE: &str = "reserve_core_output";
pub(crate) const ADVANCE: &str = "advance_core_integration";
pub(crate) const COMPLETE: &str = "complete_core_integration";
pub(crate) const CANCEL: &str = "cancel_core_integration";
pub(crate) const CADENCE: i64 = crate::workorder::PRODUCTION_CADENCE_SECS;
// The final occurrence must be strictly before Inventory's existing expiry.
// This leaves one real cadence for completion; it does not extend either hold.
pub(crate) const MAX_WORK_UNITS: u64 = 167;
pub(crate) const MAX_PROGRESS_BYTES: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IntegrationError {
    #[error("core integration request is invalid")]
    Invalid,
    #[error("core integration history disagrees")]
    Integrity,
    #[error("core integration capacity is exhausted")]
    Capacity,
    #[error("core integration authority is unavailable")]
    Unavailable,
    #[error("core integration input or capacity reservation has expired")]
    Expired,
}
pub(crate) fn bad<T>(_: T) -> IntegrationError {
    IntegrationError::Integrity
}

pub(crate) mod provider;
pub(crate) mod schedule;
