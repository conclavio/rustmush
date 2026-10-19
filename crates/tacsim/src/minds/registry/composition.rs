//! Closed production preparation of the fixed shipped certification ground.
//! This is not a capture of the live world's Atlas or an admission capability.

use std::{collections::HashMap, sync::Arc};

use mush_core::{DefinitionRef, Engine, ModError, Outbox, Registrar, Registry, World};
use mush_roll::commerce::{module::CommerceMode, rating::RatingCompatibility};

use super::manifest::FrozenPolicy;
use crate::{TacsimRuleset, config::TacsimConfig, definitions::Catalog};

#[derive(Clone)]
pub(crate) struct Registration {
    policy: Arc<FrozenPolicy>,
    original_config: TacsimConfig,
    reference: DefinitionRef,
    mode: CommerceMode,
    reconformance: bool,
}

fn error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

impl Registration {
    pub(crate) fn prepare(layer: &TacsimRuleset, mode: CommerceMode) -> Result<Self, ModError> {
        if !layer.config().enabled {
            return Err(ModError::msg(
                "Registry composition requires an enabled Tacsim layer",
            ));
        }
        let cfg = layer.config();
        cfg.arena.validate().map_err(error)?;
        cfg.match_.validate().map_err(error)?;
        cfg.sensors.validate().map_err(error)?;
        cfg.minds.validate().map_err(error)?;
        let catalog =
            Arc::new(Catalog::build_fielded(&layer.loaded_packs()?, crate::NS).map_err(error)?);
        let records = shipped_ground_records()?;
        let policy =
            Arc::new(FrozenPolicy::capture(catalog, &records, cfg.clone()).map_err(error)?);
        let reference = policy.definition_ref()?;
        Ok(Self {
            policy,
            original_config: cfg.clone(),
            reference,
            mode,
            reconformance: false,
        })
    }

    pub(crate) fn with_reconformance(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.reconformance {
            return Err(ModError::msg("reconformance requires one Active Registry"));
        }
        self.reconformance = true;
        Ok(self)
    }
    pub(crate) fn reference(&self) -> &DefinitionRef {
        &self.reference
    }

    pub(crate) fn compatibility(&self) -> RatingCompatibility {
        RatingCompatibility::combat_v1(self.reference.digest())
    }

    pub(crate) fn check(&self, cfg: &TacsimConfig, catalog: &Catalog) -> Result<(), ModError> {
        if cfg != &self.original_config || catalog.references != self.policy.catalog.references {
            return Err(ModError::msg(
                "Registry composition differs from the prepared Tacsim configuration or definitions",
            ));
        }
        Ok(())
    }

    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        cfg: &TacsimConfig,
        catalog: &Catalog,
        ic: bool,
        coverage: bool,
    ) -> Result<(), ModError> {
        self.check(cfg, catalog)?;
        // Every content descriptor has already entered this actual Tacsim
        // registrar. Its manifest validator resolves their exact references.
        let registered = if self.mode == CommerceMode::Active {
            super::registration::register_profiles_with_coverage(
                reg,
                Arc::clone(&self.policy),
                ic,
                self.reconformance,
                coverage,
            )?
            .0
        } else {
            self.policy.register(reg)?
        };
        if registered.definition() != &self.reference
            || registered.compatibility() != &self.compatibility()
        {
            return Err(ModError::msg(
                "registered Registry manifest differs from preparation",
            ));
        }
        Ok(())
    }
}

/// No store, network, worker or account service participates here. The two
/// actual modules apply only the compiled-in Atlas pack through their normal
/// boot path; the resulting temporary world and its writes are discarded.
/// These records define certification policy, independently of mutable live
/// Atlas records and content latches in the server being composed.
fn shipped_ground_records() -> Result<mush_atlas::record::Records, ModError> {
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).map_err(error)?;
    let pack = mush_content::parse_pack(crate::definitions::content::PROVING_GROUND_ATLAS_JSON)
        .map_err(error)?;
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(mush_atlas::AtlasModule::new()),
            Box::new(
                mush_content::ContentModule::new()
                    .with_appliers(appliers)
                    .with_boot_packs(vec![pack]),
            ),
        ],
    )?;
    let mut effects = mush_core::engine::Effects::new();
    engine.modules_loaded(HashMap::new(), &mut Outbox::new(), &mut effects)?;
    if !effects.is_empty() {
        return Err(ModError::msg(
            "Registry ground preparation unexpectedly requested external work",
        ));
    }
    mush_atlas::place::records(&engine.world)
        .cloned()
        .ok_or_else(|| ModError::msg("Registry ground preparation lacks actual Atlas records"))
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
