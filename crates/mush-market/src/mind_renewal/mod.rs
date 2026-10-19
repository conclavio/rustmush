//! Explicit personal Pin renewal composition. Decoded data conveys no owner
//! authority. Providers and paired boot exist only under the optional config;
//! no scheduler, automatic acceptance or dedicated client UI is installed.
mod actions;
mod audit;
mod capacity;
mod config;
mod families;
mod offer;
mod pair;
mod provider;
mod publication;
mod records;
mod registration;
pub(crate) use audit::require_disabled;
pub use config::{RenewalConfig, RenewalPolicy};
pub(crate) use families::Families;
pub use records::{MindRenewalOperationV1, MindRenewalReceiptV1};
pub(crate) use registration::register;

#[cfg(test)]
mod tests;
