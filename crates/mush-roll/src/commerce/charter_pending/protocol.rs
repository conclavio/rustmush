//! Separate explicit pending protocol. Immediate offer@1/accept@1 remain exact.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use serde_json::json;
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) const OFFER: &str = "offer_mind_charter_transfer";
pub(crate) const FUND: &str = "fund_mind_charter_transfer";
pub(crate) const SETTLE: &str = "settle_mind_charter_transfer";
pub(crate) const REFUND: &str = "refund_mind_charter_transfer";
pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 8], ModError> {
    let retention = super::super::authored_retirement::retention_reference()?;
    let action =
        |id: &str, version, body: serde_json::Value| -> Result<DefinitionDescriptor, ModError> {
            Ok(DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(bad)?,
                DefinitionId::new(id).map_err(bad)?,
                version,
                body,
            )
            .map_err(bad)?
            .with_references(vec![retention.clone()]))
        };
    let outcome = |id: &str| -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(bad)?,
            DefinitionId::new(id).map_err(bad)?,
            1,
            json!({"format":1}),
        )
        .map_err(bad)
    };
    Ok([
        action(
            OFFER,
            2,
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"current_personal_seller","named_buyer":true,"source_free":true,"settlement":"earlier_of_term_end_and_explicit_delay","no_automatic_acceptance":true}),
        )?,
        action(
            FUND,
            1,
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"named_buyer","hold":"single_real_escrow","title":"unchanged","future_terminal_reserved":true}),
        )?,
        action(
            SETTLE,
            1,
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"registered_private_service_intent","trigger":"last_owned_claim_close_or_same_clock_deadline","old_claims":"retained"}),
        )?,
        action(
            REFUND,
            1,
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"registered_private_service_intent","causes":["registry_revoked","clock_changed","recipient_unavailable"],"source_rights":"not_required"}),
        )?,
        outcome("mind_charter_pending_offered")?,
        outcome("mind_charter_pending_funded")?,
        outcome("mind_charter_pending_settled")?,
        outcome("mind_charter_pending_refunded")?,
    ])
}
fn reference(index: usize) -> Result<DefinitionRef, ModError> {
    let descriptor = definitions()?
        .into_iter()
        .nth(index)
        .ok_or_else(|| bad("pending action absent"))?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor)
        .map_err(bad)
}
pub(crate) fn offer_action() -> Result<DefinitionRef, ModError> {
    reference(0)
}
pub(crate) fn fund_action() -> Result<DefinitionRef, ModError> {
    reference(1)
}
pub(crate) fn settle_action() -> Result<DefinitionRef, ModError> {
    reference(2)
}
pub(crate) fn refund_action() -> Result<DefinitionRef, ModError> {
    reference(3)
}
