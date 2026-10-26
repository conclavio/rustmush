//! Explicit personal Pin renewal composition. Decoded data conveys no owner
//! authority. Providers and paired boot exist only under the optional config;
//! the separately enabled service installs bounded continuations and controls,
//! never automatic acceptance.
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
mod service;
mod service_provider;
pub(crate) use audit::require_disabled;
pub use config::{RenewalConfig, RenewalPolicy};
pub(crate) use families::Families;
pub use records::{MindRenewalOperationV1, MindRenewalReceiptV1};
pub(crate) use registration::register;
pub(crate) use service::{enable as enable_service, guard_current as service_guard_current};

#[cfg(test)]
mod tests;
