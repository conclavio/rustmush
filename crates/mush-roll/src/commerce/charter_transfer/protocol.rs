use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use serde_json::json;
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) const OFFER: &str = "offer_mind_charter_transfer";
pub(crate) const ACCEPT: &str = "accept_mind_charter_transfer";
pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 4], ModError> {
    let retention = super::super::authored_retirement::retention_reference()?;
    let action = |id: &str, body: serde_json::Value| -> Result<DefinitionDescriptor, ModError> {
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(bad)?,
            DefinitionId::new(id).map_err(bad)?,
            1,
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
            json!({"format":1,"protocol":"personal_charter_transfer_v1","authority":"current_personal_seller","named_buyer":true,"source_free":true}),
        )?,
        action(
            ACCEPT,
            json!({"format":1,"protocol":"personal_charter_transfer_v1","authority":"named_buyer","settlement":"native_money_and_title_same_union","open_owned_claims":"refuse"}),
        )?,
        outcome("mind_charter_transfer_offered")?,
        outcome("mind_charter_transferred")?,
    ])
}
fn reference(index: usize) -> Result<DefinitionRef, ModError> {
    let descriptor = definitions()?
        .into_iter()
        .nth(index)
        .ok_or_else(|| bad("charter transfer action absent"))?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor)
        .map_err(bad)
}
pub(crate) fn offer_action() -> Result<DefinitionRef, ModError> {
    reference(0)
}
pub(crate) fn accept_action() -> Result<DefinitionRef, ModError> {
    reference(1)
}
