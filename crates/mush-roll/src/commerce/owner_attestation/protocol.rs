//! Exact optional owner attestation protocol. Historical descriptors
//! retain their manifest; only the registered current action can issue a row.
use super::super::records::PublicationError;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use serde_json::json;

pub(crate) const ACTION: &str = "attest_owner_mind";
pub(crate) const OUTCOME: &str = "owner_mind_attested";
/// M8c B4: the House owner attestation, its own definition so the personal
/// one's digest never moves: a character acting for the House on the House's
/// scope (`roll.holder.org`) renews the organisation's owner licence under
/// the House terms.
pub(crate) const HOUSE_ACTION: &str = "attest_house_mind";
pub(crate) const HOUSE_OUTCOME: &str = "house_mind_attested";
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub(crate) fn definitions(
    manifest: &DefinitionRef,
    term_seconds: i64,
    grace_seconds: i64,
) -> Result<[DefinitionDescriptor; 2], ModError> {
    super::super::filing::InitialFilingTerms::new(term_seconds, grace_seconds).map_err(error)?;
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "owner attestation requires the combat Registry",
        ));
    }
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"personal_owner_attestation_v1",
                "authority":"personal_author_current_title_and_own_current_parent",
                "source_ancestry":"immutable","term_seconds":term_seconds,"grace_seconds":grace_seconds,"due":"predecessor_term_end","free":true}),
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

pub(crate) fn action(
    manifest: &DefinitionRef,
    term_seconds: i64,
    grace_seconds: i64,
) -> Result<DefinitionRef, PublicationError> {
    let [action, _] = definitions(manifest, term_seconds, grace_seconds)
        .map_err(|_| PublicationError::Invalid)?;
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(|_| PublicationError::Invalid)?,
            action,
        )
        .map_err(|_| PublicationError::Invalid)
}

/// M8c B4: the House attestation's definitions, under the House terms
/// (`CommerceConfig::house_terms`).
pub(crate) fn house_definitions(
    manifest: &DefinitionRef,
    term_seconds: i64,
    grace_seconds: i64,
) -> Result<[DefinitionDescriptor; 2], ModError> {
    super::super::filing::InitialFilingTerms::new(term_seconds, grace_seconds).map_err(error)?;
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "owner attestation requires the combat Registry",
        ));
    }
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(HOUSE_ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"house_owner_attestation_v1",
                "authority":"house_scope_holder_grant_and_current_organisation_title",
                "source_ancestry":"immutable","term_seconds":term_seconds,"grace_seconds":grace_seconds,"due":"predecessor_term_end","free":true}),
        )
        .map_err(error)?
        .with_references(vec![
            manifest.clone(),
            super::super::authored_retirement::retention_reference()?,
        ]),
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(error)?,
            DefinitionId::new(HOUSE_OUTCOME).map_err(error)?,
            1,
            json!({"format":1}),
        )
        .map_err(error)?,
    ])
}
pub(crate) fn house_action(
    manifest: &DefinitionRef,
    term_seconds: i64,
    grace_seconds: i64,
) -> Result<DefinitionRef, PublicationError> {
    let [action, _] = house_definitions(manifest, term_seconds, grace_seconds)
        .map_err(|_| PublicationError::Invalid)?;
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(|_| PublicationError::Invalid)?,
            action,
        )
        .map_err(|_| PublicationError::Invalid)
}
