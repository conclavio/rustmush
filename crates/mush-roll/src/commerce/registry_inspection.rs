//! **The Registry's inspection of a hull** (doc 17h cut M8a, A4; doc 17 §13 "`@registry/inspect`";
//! doc 23 §7): one acknowledged read of what a hull carries — its fitted core, the core's
//! provenance, the seal on its open trip and that seal's licence state — written as one
//! [`InspectionRecord`] under the Registry operator who asked.
//!
//! The twin of [`super::registry_revocation`] in shape (a policy of operators under one Org
//! scope, a live authorization observed on every invocation, a bounded record family), with
//! one difference the brief fixes: the facts are the hull's, which only tacsim can read, so
//! **the action and its provider are tacsim's** ([`ACTION_OWNER`]) and the Roll owns the
//! authority definition, the record family and the writer that lands the record
//! ([`InspectionWriter`], the IC claims writer's pattern: the Roll installs it, tacsim's
//! registrar takes it `for_action`, and the record is inserted from tacsim's own transaction
//! through the family delegated to it).
//!
//! An inspection reads; it grants, revokes, recalls, moves or seals nothing, and it is one
//! write (M8a's A8).

mod authority;
pub mod claims;
mod policy;
pub mod principal;
pub(crate) mod records;
#[cfg(test)]
mod tests;
mod writer;

use super::records::{PublicationError, Record};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use authority::Authorization;
pub use claims::{
    CLAIMS_PER_AUTHOR, ClaimRecord, MAX_CLAIM_HULLS, MAX_CLAIM_TEXT, claim_key, claim_slots,
};
pub use policy::RegistryInspectionPolicy;
pub use principal::{PRINCIPAL_FAMILY, RevokedPrincipal, principal_key};
pub use records::{
    HullClass, INSPECTIONS_PER_HULL, InspectionFacts, InspectionRecord, Provenance, SealCheck,
    UngovernedNote, inspection_key, inspection_slots,
};
pub use writer::{InspectionWriter, RegisteredInspectionWriter};

/// The Org capability a Registry operator holds on the configured scope to inspect.
pub const REGISTRY_INSPECTION_CAPABILITY: &str = "roll.registry.inspect";
const CAPABILITY: &str = REGISTRY_INSPECTION_CAPABILITY;
/// **The Org capability a Registry operator holds to fine** (doc 17h cut M8d D3): a capability
/// of its own on the inspection's scope, observed by its own twin
/// (`authority::observe_fine`), never implied by the inspection grant.
pub const REGISTRY_FINE_CAPABILITY: &str = "roll.registry.fine";

/// **The Org capability a Registry operator holds to read the quirk claims** (doc 17h cut
/// M8g G3): its own on the inspection's scope, observed by its own twin
/// (`authority::observe_claims`), never implied by another Registry grant.
pub const REGISTRY_CLAIMS_CAPABILITY: &str = "roll.registry.claims";

/// The quirk claims' record family (doc 17h cut M8g G3), keyed by the author's ring slot.
pub const CLAIM_FAMILY: &str = "roll_mind_claims";

/// The owner of the inspection action and outcome definitions and of the provider that
/// prepares them: tacsim, which reads the hull.
pub const ACTION_OWNER: &str = "tacsim";

/// The record family's name, keyed by the inspection's commit.
pub const INSPECTION_FAMILY: &str = "roll_registry_inspections";

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
