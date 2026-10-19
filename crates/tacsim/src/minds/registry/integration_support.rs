//! Test composition of actual Registry resources; no source/evidence substitute.
//! Keeping FrozenPolicy here avoids widening its production visibility merely
//! so the sibling bulk owner tests can compose its real registered adapters.
use std::sync::Arc;

use mush_core::{DefinitionRef, ModError, Registrar};
use mush_roll::commerce::rating::RatingCompatibility;

use super::{factory::Factory, manifest::FrozenPolicy};

#[derive(Clone)]
pub(in crate::minds) struct Resources(Arc<FrozenPolicy>, bool);
impl Resources {
    pub(in crate::minds) fn roll_owners(module: mush_roll::RollModule) -> mush_roll::RollModule {
        super::registration::roll_owners(module)
    }
    pub(in crate::minds) fn capture() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self(Arc::new(super::tests::frozen_policy()?), false))
    }
    pub(in crate::minds) fn reconformance_config(
        cfg: crate::config::TacsimConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        cfg.arena.validate()?;
        cfg.minds.validate()?;
        Ok(Self(
            Arc::new(super::tests::frozen_policy_with_config(cfg)?),
            true,
        ))
    }
    pub(in crate::minds) fn reconformance(&self) -> bool {
        self.1
    }
    pub(in crate::minds) fn reference(&self) -> Result<DefinitionRef, ModError> {
        self.0.definition_ref()
    }
    pub(in crate::minds) fn config(&self) -> crate::config::TacsimConfig {
        self.0.cfg.clone()
    }
    pub(in crate::minds) fn register(
        &self,
        reg: &mut Registrar<'_>,
        active: bool,
    ) -> Result<(DefinitionRef, RatingCompatibility, Option<Arc<Factory>>), ModError> {
        for kind in crate::definitions::KINDS {
            let of_kind = self
                .0
                .catalog
                .descriptors
                .iter()
                .filter(|descriptor| descriptor.kind().as_str() == kind)
                .cloned()
                .collect();
            reg.definition_validator(
                mush_core::mechanics::DefinitionKind::new(kind)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                crate::definitions::exact_validator(of_kind),
            )?;
        }
        for descriptor in &self.0.catalog.descriptors {
            reg.definition(descriptor.clone())?;
        }
        self.register_after_content(reg, active)
    }
    /// The real TacsimRuleset already owns the same actual fielded content.
    /// Register only Registry composition after its production registration.
    pub(in crate::minds) fn register_after_content(
        &self,
        reg: &mut Registrar<'_>,
        active: bool,
    ) -> Result<(DefinitionRef, RatingCompatibility, Option<Arc<Factory>>), ModError> {
        if active {
            let (manifest, factory) = if self.1 {
                super::registration::register_profiles(reg, Arc::clone(&self.0), false, true)?
            } else {
                super::registration::register(reg, Arc::clone(&self.0))?
            };
            Ok((
                manifest.definition().clone(),
                manifest.compatibility().clone(),
                Some(factory),
            ))
        } else {
            let manifest = self.0.register(reg)?;
            Ok((
                manifest.definition().clone(),
                manifest.compatibility().clone(),
                None,
            ))
        }
    }
}
