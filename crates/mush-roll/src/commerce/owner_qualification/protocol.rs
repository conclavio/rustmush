//! Exact optional qualification replacement protocol. Historical descriptors
//! retain their manifest; only the registered current action can issue a row.
use super::super::records::PublicationError;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use serde_json::json;

pub(crate) const ACTION: &str = "replace_owner_parent_qualification";
pub(crate) const OUTCOME: &str = "owner_parent_qualification_replaced";
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub(crate) fn definitions(manifest: &DefinitionRef) -> Result<[DefinitionDescriptor; 2], ModError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "owner qualification requires the combat Registry",
        ));
    }
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"personal_owner_parent_qualification_v1",
                "authority":"current_personal_title_and_own_compatible_parent",
                "source_ancestry":"immutable","child_term":"unchanged"}),
        )
        .map_err(error)?
        .with_references(vec![
            manifest.clone(),
            super::super::authored_retirement::retention_reference()?,
        ]),
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(error)?,
            DefinitionId::new(OUTCOME).map_err(error)?,
            1,
            json!({"format":1}),
        )
        .map_err(error)?,
    ])
}

pub(crate) fn action(manifest: &DefinitionRef) -> Result<DefinitionRef, PublicationError> {
    let [action, _] = definitions(manifest).map_err(|_| PublicationError::Invalid)?;
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(|_| PublicationError::Invalid)?,
            action,
        )
        .map_err(|_| PublicationError::Invalid)
}
