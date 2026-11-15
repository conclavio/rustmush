//! Trusted composition identifies a real Registry authority; no actor is created.
//!
//! The policy is one text for both Registry sanctions (`registry_twin::policy`, doc 46 CT.9);
//! this file is the recall's identity.
use super::super::registry_twin::{ExactRecall, RegistryPolicy, seal::Sealed};
use mush_core::World;
use serde_json::json;

/// Exact optional Registry scope and operators for the filed-revision recall. Decoding this
/// value is data, never a grant; the live Org authorizer remains mandatory on every invocation.
pub type RegistryRecallPolicy = RegistryPolicy<ExactRecall>;

impl Sealed for ExactRecall {
    const AUTHORITY: &'static str = "mind_recall";
    const ACTION: &'static str = "registry_revision_recall";
    const OUTCOME: &'static str = "registry_revision_recalled";
    const CAPABILITY: &'static str = super::CAPABILITY;
    const UNAVAILABLE: &'static str = "recall definitions unavailable";
    const CAPTURED: &'static str = "recall readiness already captured";
    const OWNERS: &'static [&'static str] = &[crate::NS, "org"];
    fn action_value() -> serde_json::Value {
        json!({
            "format":1,"protocol":"registry_exact_revision_recall_v1","reason":"registry_review",
            "scope":"one_filed_revision","shared_rating":"unchanged"
        })
    }
    fn owner_missing(_: &World) -> bool {
        false
    }
}
