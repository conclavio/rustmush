//! Actual owner registration for the metered Registry lane. The returned
//! factory is created only after the real manifest and concrete owner adapters
//! have entered Tacsim's registrar. It still grants no rating or worker lease.

use std::{collections::BTreeSet, sync::Arc};

use mush_core::mechanics::{DefinitionKind, DefinitionValidationContext};
use mush_core::{ModError, Registrar};
use mush_roll::commerce::rating::registration::RegistryDefinitions;

use super::factory::Factory;
use super::manifest::{FrozenPolicy, RegisteredManifest};
use crate::minds::bulk::registry_participant::{ArtifactParticipant, Policy, QuotaParticipant};

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub(super) fn register(
    reg: &mut Registrar<'_>,
    policy: Arc<FrozenPolicy>,
) -> Result<(RegisteredManifest, Arc<Factory>), ModError> {
    register_with_ic(reg, policy, false)
}
pub(super) fn register_with_ic(
    reg: &mut Registrar<'_>,
    policy: Arc<FrozenPolicy>,
    ic: bool,
) -> Result<(RegisteredManifest, Arc<Factory>), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("Registry owner"));
    }
    let manifest = policy.register(reg)?;
    let definitions = RegistryDefinitions::new(manifest.definition().clone())?;
    let mut expected = vec![definitions.quota().clone(), definitions.artifacts().clone()];
    if ic {
        expected.push(crate::minds::ic_sortie::protocol::participant_descriptor()?);
    }
    reg.definition_validator(
        DefinitionKind::new("owner_participant").map_err(error)?,
        move |context: &DefinitionValidationContext<'_>| {
            let actual = context.definition();
            let reference = actual.reference();
            if reference.key().owner.as_str() == crate::NS
                && expected.iter().any(|expected| {
                    reference.key().kind == *expected.kind()
                        && reference.key().id == *expected.id()
                        && reference.version() == expected.version()
                        && actual.payload() == expected.payload()
                        && actual.references() == expected.references()
                        && actual.is_current() == expected.is_current()
                })
            {
                Ok(())
            } else {
                Err("Registry participant differs from the closed owner implementation".into())
            }
        },
    )?;
    let actions = BTreeSet::from(definitions.actions().clone());
    let participant_policy = Policy::new(definitions.actions().clone()).map_err(error)?;
    let quota = reg.definition(definitions.quota().clone())?;
    reg.owner_participant(
        quota,
        actions.clone(),
        QuotaParticipant::new(participant_policy.clone()),
    )?;
    let artifacts = reg.definition(definitions.artifacts().clone())?;
    reg.owner_participant(
        artifacts,
        actions,
        ArtifactParticipant::new(participant_policy),
    )?;
    let factory = Factory::registered(policy, manifest.clone()).map_err(error)?;
    let factory = Arc::new(factory);
    crate::minds::bulk::registry_service::register(
        reg,
        Arc::clone(&factory),
        manifest.definition(),
    )?;
    Ok((manifest, factory))
}

/// Trusted module composition selects the actual two slot adapters; transports
/// cannot choose another pair through this function or pass prepared output.
pub(crate) fn roll_owners(module: mush_roll::RollModule) -> mush_roll::RollModule {
    module.with_registry_owners::<QuotaParticipant, ArtifactParticipant>()
}
