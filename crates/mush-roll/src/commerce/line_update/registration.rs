use crate::commerce::{
    families::CommerceFamilies,
    participant::RootParticipant,
    rating::{RatingCompatibility, RatingFamilies},
};
use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_economy::registry_filing::InitialFilingParticipant;
use serde_json::json;
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn definitions(manifest: &DefinitionRef) -> Result<[DefinitionDescriptor; 2], ModError> {
    Ok([DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,DefinitionId::new(super::ACTION).map_err(error)?,1,
        json!({"format":1,"protocol":"personal_original_compatible_update_v1","authority":"initial_author_and_current_title","schema":"exact_initial_schema","owner_term":"unchanged","fee":"actual_initial_filing_policy"})).map_err(error)?.with_references(vec![manifest.clone()]),
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(error)?,DefinitionId::new("compatible_mind_update_committed").map_err(error)?,1,json!({"format":1})).map_err(error)?])
}
pub(crate) fn derived_definitions(
    manifest: &DefinitionRef,
) -> Result<[DefinitionDescriptor; 2], ModError> {
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(super::DERIVED_ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"personal_dependent_compatible_update_v1",
                "authority":"current_title_and_own_immediate_parent",
                "schema":"exact_initial_schema","owner_term":"unchanged",
                "source":"immutable_acquisition","fee":"actual_initial_filing_policy"}),
        )
        .map_err(error)?
        .with_references(vec![manifest.clone()]),
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(error)?,
            DefinitionId::new("compatible_derived_mind_update_committed").map_err(error)?,
            1,
            json!({"format":1}),
        )
        .map_err(error)?,
    ])
}

pub(crate) fn register_derived(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: DefinitionRef,
    compatibility: RatingCompatibility,
    definitions: [DefinitionRef; 2],
    parent: mush_core::read_service::OwnerReadHandle<
        crate::commerce::parent_grant::ParentGrantReader,
    >,
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS
        || compatibility.digest() != manifest.digest()
        || action != historical_derived_action(&compatibility)?
    {
        return Err(error("derived update composition differs"));
    }
    let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    reg.action_provider(
        action.clone(),
        super::provider::UpdateProvider {
            action,
            outcome,
            manifest,
            families,
            ratings,
            compatibility,
            economy,
            roots,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The saved revision cannot update this dependent line.",
            )
            .map_err(error)?,
            parent: Some(parent),
        },
    )
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: DefinitionRef,
    compatibility: RatingCompatibility,
    definitions: [DefinitionRef; 2],
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS || compatibility.digest() != manifest.digest() {
        return Err(error("compatible update composition differs"));
    }
    let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let refusal = ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "The saved revision cannot update this original line.",
    )
    .map_err(error)?;
    reg.action_provider(
        action.clone(),
        super::provider::UpdateProvider {
            action,
            outcome,
            manifest,
            families,
            ratings,
            compatibility,
            economy,
            roots,
            refusal,
            parent: None,
        },
    )
}

/// Reconstruct the exact old action from an actual retained certificate's
/// compatibility, rather than trusting its locator or the current manifest.
pub(crate) fn historical_action(
    compatibility: &RatingCompatibility,
) -> Result<DefinitionRef, ModError> {
    historical(compatibility, false)
}
pub(crate) fn historical_derived_action(
    compatibility: &RatingCompatibility,
) -> Result<DefinitionRef, ModError> {
    historical(compatibility, true)
}
fn historical(
    compatibility: &RatingCompatibility,
    derived: bool,
) -> Result<DefinitionRef, ModError> {
    use mush_core::mechanics::{DefinitionKey, DefinitionRegistryBuilder, OwnerId};
    let manifest = DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim").map_err(error)?,
            DefinitionKind::new("registry_manifest").map_err(error)?,
            DefinitionId::new("combat").map_err(error)?,
        ),
        1,
        compatibility.digest(),
    )
    .map_err(error)?;
    let [action, _] = if derived {
        derived_definitions(&manifest)?
    } else {
        definitions(&manifest)?
    };
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, action)
        .map_err(error)
}
