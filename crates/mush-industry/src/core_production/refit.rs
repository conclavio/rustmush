//! Optional paid refit producer. Every physical image remains Core/Tactics owned.
mod authority;
mod capacity;
mod config;
mod families;
mod policies;
mod progress;
mod recipe;
pub(crate) mod records;
pub(crate) mod state;
pub(crate) use super::integration::{IntegrationError as Error, bad};
pub use config::CoreRefitFactoryConfig;
pub use policies::RefitReplacementPolicy;
pub use recipe::{PublishedRefitRecipe, RefitRecipeAuthority};
pub(crate) const INITIALIZE: &str = "initialize_core_refit";
pub(crate) const OFFER: &str = "offer_core_refit";
pub(crate) const ACCEPT: &str = "accept_core_refit";
pub(crate) const ADVANCE: &str = "advance_core_refit";
pub(crate) const INSTALL: &str = "install_core_component";
pub(crate) const CANCEL: &str = "cancel_core_refit";
pub(crate) const ACTIONS: [&str; 6] = [INITIALIZE, OFFER, ACCEPT, ADVANCE, INSTALL, CANCEL];
pub(crate) const CADENCE: i64 = crate::workorder::PRODUCTION_CADENCE_SECS;
pub(crate) const MAX_WORK_UNITS: u64 = 167;
pub(crate) const MAX_PROGRESS_BYTES: usize = 32768;

pub(crate) mod owner;

pub(crate) mod runtime;

pub(crate) mod audit;
pub(crate) mod provider;
pub(crate) mod schedule;
