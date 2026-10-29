//! Operational unit authority, independent of title, carrier access and rank.
//!
//! Pure resolution reads the complete live grant lineage. The optional runtime
//! owns grants in a typed family and publishes changes only through the shared
//! authoritative coordinator; `OrgStore` contains no copy of those grants.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::mechanics::OwnerId;

mod commands;
mod families;
pub mod reader;
mod record;
mod resolve;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use record::*;
pub use resolve::*;
pub use runtime::UnitAuthorityWorldFacts;

/// Composition-time facts and explicitly authorized owner-read consumers.
/// Missing operational-rights or mission facts fail closed.
#[derive(Clone, Debug)]
pub struct UnitAuthorityConfig {
    pub facts: Arc<dyn UnitAuthorityWorldFacts>,
    pub requesters: BTreeSet<OwnerId>,
    pub dependencies: BTreeSet<OwnerId>,
}
