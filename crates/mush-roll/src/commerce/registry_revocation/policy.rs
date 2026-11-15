//! Trusted composition identifies a real Registry authority; no actor is created.
//!
//! The policy is one text for both Registry sanctions (`registry_twin::policy`, doc 46 CT.9);
//! this file is the revocation's identity.
use super::super::registry_twin::{RegistryPolicy, TermRevocation, seal::Sealed};
use mush_core::World;
use serde_json::json;
use std::any::TypeId;

/// Exact optional Registry scope and operators for the accepted-term revocation. Decoding this
/// value is data, never a grant; the live Org authorizer remains mandatory on every invocation.
pub type RegistryRevocationPolicy = RegistryPolicy<TermRevocation>;

impl Sealed for TermRevocation {
    const AUTHORITY: &'static str = "mind_revocation";
    const ACTION: &'static str = "registry_term_revocation";
    const OUTCOME: &'static str = "registry_term_revoked";
    const CAPABILITY: &'static str = super::CAPABILITY;
    const UNAVAILABLE: &'static str = "revocation definitions unavailable";
    const CAPTURED: &'static str = "revocation readiness already captured";
    const OWNERS: &'static [&'static str] = &[crate::NS, "org", "economy"];
    fn action_value() -> serde_json::Value {
        json!({
            "format":1,"protocol":"registry_exact_term_revocation_v1","reason":"registry_review",
            "scope":"one_accepted_term_generation","duration":"original_grace_end","later_terms":"fresh_admission","shared_rating":"unchanged"
        })
    }
    /// The revocation reads Economy's store; the recall does not.
    fn owner_missing(world: &World) -> bool {
        world
            .module_state_owner(TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|v| v.as_str() != "economy")
    }
}
