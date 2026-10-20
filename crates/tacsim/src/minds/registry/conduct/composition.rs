//! Explicit real-policy registration. Ordinary Registry worlds never enable
//! this service or acquire permission from a retained Conduct record.
use super::{factory::Factory, manifest::Policy};
use crate::minds::bulk::conduct_participant::{ArtifactParticipant, QuotaParticipant};
use crate::{TacsimRuleset, config::TacsimConfig, definitions::Catalog};
use mush_core::{
    DefinitionRef, ModError, Registrar,
    mechanics::{DefinitionKind, DefinitionValidationContext},
};
use mush_roll::commerce::{
    conduct::{ConductConfig, registration},
    module::CommerceMode,
};
use std::{collections::BTreeSet, sync::Arc};
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

#[derive(Clone)]
pub(crate) struct Registration {
    policy: Arc<Policy>,
    // Registry physics normalizes its trial clock. Registration must compare
    // the actual host configuration captured before that normalization.
    original_config: TacsimConfig,
    reference: DefinitionRef,
}
impl Registration {
    pub(crate) fn prepare(
        layer: &TacsimRuleset,
        mode: CommerceMode,
        registry: &DefinitionRef,
    ) -> Result<Self, ModError> {
        if mode != CommerceMode::Active || !layer.config().enabled {
            return Err(error("Conduct requires Active Registry composition"));
        }
        let catalog = Arc::new(layer.definition_catalog()?);
        let records = super::super::composition::shipped_ground_records()?;
        let policy =
            Arc::new(Policy::capture(catalog, &records, layer.config().clone()).map_err(error)?);
        if &policy.physics.definition_ref()? != registry {
            return Err(error("Conduct and Registry captured policies disagree"));
        }
        let reference = policy.definition_ref().map_err(error)?;
        Ok(Self {
            policy,
            original_config: layer.config().clone(),
            reference,
        })
    }
    pub(crate) fn reference(&self) -> &DefinitionRef {
        &self.reference
    }
    pub(crate) fn roll_config(&self) -> Result<ConductConfig, ModError> {
        ConductConfig::new::<QuotaParticipant, ArtifactParticipant>(self.reference.clone())
    }
    pub(crate) fn check(&self, cfg: &TacsimConfig, catalog: &Catalog) -> Result<(), ModError> {
        if cfg != &self.original_config
            || catalog.references != self.policy.physics.catalog.references
        {
            return Err(error("Conduct configured resources changed"));
        }
        Ok(())
    }
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        cfg: &TacsimConfig,
        catalog: &Catalog,
    ) -> Result<(), ModError> {
        self.check(cfg, catalog)?;
        if reg.module() != crate::NS {
            return Err(error("Conduct actual owner"));
        }
        let expected = self.policy.descriptor().map_err(error)?;
        let captured = expected.clone();
        reg.definition_validator(
            DefinitionKind::new("conduct_manifest").map_err(error)?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let actual = ctx.definition();
                if actual.reference().key().owner.as_str() != crate::NS
                    || actual.reference().key().id != *captured.id()
                    || actual.reference().version() != captured.version()
                    || actual.payload() != captured.payload()
                    || actual.references() != captured.references()
                    || actual.is_current() != captured.is_current()
                {
                    return Err("Conduct captured resources differ".into());
                }
                for reference in actual.references() {
                    ctx.get_exact(reference).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )?;
        let manifest = reg.definition(expected)?;
        if manifest != self.reference {
            return Err(error("Conduct registered manifest differs"));
        }
        let config = self.roll_config()?;
        let actions = config.actions()?;
        let policy = crate::minds::bulk::conduct_participant::Policy::new(
            manifest.clone(),
            self.policy.digest(),
            actions.clone(),
        )
        .map_err(error)?;
        let quota = reg.definition(registration::quota_descriptor()?)?;
        reg.owner_participant(
            quota,
            BTreeSet::from(actions.clone()),
            QuotaParticipant::new(policy.clone()),
        )?;
        let artifacts = reg.definition(registration::artifacts_descriptor()?)?;
        reg.owner_participant(
            artifacts,
            BTreeSet::from(actions.clone()),
            ArtifactParticipant::new(policy),
        )?;
        let factory = Arc::new(
            Factory::registered(Arc::clone(&self.policy), manifest.clone()).map_err(error)?,
        );
        crate::minds::bulk::conduct_service::register(reg, factory, &manifest, actions)?;
        Ok(())
    }
}
pub(crate) fn carrier_pack() -> Result<crate::config::LoadedPack, ModError> {
    Policy::carrier_pack().map_err(error)
}
