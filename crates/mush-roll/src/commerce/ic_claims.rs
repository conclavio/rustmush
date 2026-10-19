//! Fixed IC claim history and accounting. These data do not authorize a seal.
//!
//! Only the closed composed owner path may install them. OOC selection never
//! reaches this module. The actual Tacsim/Tactics/Sim custody proof and final
//! reverse audit are prerequisites for registering the writer.

pub(crate) mod audit;
mod config;
mod inventory;
mod owner_protocol;
mod protocol;
pub(crate) mod reader;
mod records;
pub(crate) mod registration;
pub(crate) mod roots;
mod writer;

pub use audit::IcClaimHistory;
pub use config::IcClaimConfig;
pub(crate) use inventory::ClaimInventory;
pub use owner_protocol::{IcCloseKind, IcSealOutput, IcSealParticipant};
pub(crate) use protocol::{IcParticipants, IcProtocol, Transition};
pub use reader::{IcAdmissionFacts, IcAdmissionReader, IcAdmissionRequest};
pub(crate) use records::{ClaimRecord, CloseCause, CloseKind, OpenClaim, SortieNumber};
pub use writer::{IcClaimReceipt, IcWriter, RegisteredIcWriter};

/// Fixed storage limits, not paid seat entitlements or caller configuration.
pub const MAX_OPEN_IC_CLAIMS: usize = 1024;
pub const MAX_IC_CLAIM_HISTORY: usize = 8192;

#[cfg(test)]
mod tests;
