use super::{
    host::IcHost,
    participant::{Config, IcSortieParticipant},
    state::IcSorties,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError, ModuleCtx,
    Registrar,
};
use mush_roll::commerce::ic_claims::{IcAdmissionReader, IcClaimConfig, IcWriter};
use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

#[derive(Clone)]
pub(crate) struct Composition {
    pub(crate) claims: IcClaimConfig,
    pub(crate) writer: IcWriter,
    custody: mush_tactics::ic_custody::IcCustodyConfig,
    exclusion: mush_sim::ic_exclusion::IcExclusionConfig,
    bases: Vec<DefinitionDescriptor>,
}
impl Composition {
    pub(crate) fn new(
        manifest: DefinitionRef,
        bases: Vec<mush_sim::ic_exclusion::ConfiguredIcBase>,
    ) -> Result<Self, ModError> {
        let seal = mush_core::mechanics::DefinitionRegistryBuilder::new()
            .register(
                mush_core::OwnerId::new(crate::NS).map_err(|e| bad(e))?,
                super::protocol::participant_descriptor()?,
            )
            .map_err(|e| bad(e))?;
        let claims = IcClaimConfig::new(manifest, seal)?;
        let custody = mush_tactics::ic_custody::IcCustodyConfig::new(
            claims.manifest().clone(),
            claims.participants(),
        )?;
        let base_descriptors = bases
            .iter()
            .map(|b| b.registration_descriptor())
            .collect::<Result<Vec<_>, _>>()?;
        let exclusion = mush_sim::ic_exclusion::IcExclusionConfig::new(
            claims.manifest().clone(),
            claims.participants(),
            bases,
        )?;
        Ok(Self {
            claims,
            writer: IcWriter::new(),
            custody,
            exclusion,
            bases: base_descriptors,
        })
    }
    pub(crate) fn physical(
        &self,
        tactics: mush_tactics::TacticsModule,
        sim: mush_sim::SimulationModule,
    ) -> (mush_tactics::TacticsModule, mush_sim::SimulationModule) {
        (
            tactics.with_ic_mind_custody(self.custody.clone()),
            sim.with_ic_mind_custody(self.exclusion.clone()),
        )
    }
    pub(crate) fn layer(
        &self,
        cfg: &crate::config::TacsimConfig,
    ) -> Result<Registration, ModError> {
        cfg.arena.validate().map_err(|e| bad(e))?;
        cfg.minds.validate().map_err(|e| bad(e))?;
        let config = Config {
            claims: self.claims.clone(),
            writer: self.writer.clone(),
            arena: cfg.arena.clone(),
            minds: cfg.minds.clone(),
            definitions: Arc::new(OnceLock::new()),
        };
        Ok(Registration {
            host: IcHost::new(self.writer.clone(), self.claims.actions()?),
            config,
            bases: self.bases.clone(),
        })
    }
}
#[derive(Clone)]
pub(crate) struct Registration {
    pub(super) config: Config,
    pub(crate) host: IcHost,
    bases: Vec<DefinitionDescriptor>,
}
impl Registration {
    pub(crate) fn actions(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        self.config.claims.action_descriptors()
    }
    pub(crate) fn outcome(&self) -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(|e| bad(e))?, DefinitionId::new("ic_mind_transition").map_err(|e| bad(e))?, 1,
            serde_json::json!({"format":1,"protocol":"ic_mind_transition_v1","fields":["open","closed"]})).map_err(|e| bad(e))
    }
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        cfg: &crate::config::TacsimConfig,
        manifest: &DefinitionRef,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS
            || self.config.claims.manifest() != manifest
            || self.config.arena != cfg.arena
            || self.config.minds != cfg.minds
            || !cfg.enabled
        {
            return Err(bad("IC composition differs from actual Tacsim"));
        }
        reg.definition_validator(
            DefinitionKind::new("ic_base").map_err(|e| bad(e))?,
            crate::definitions::exact_validator(self.bases.clone()),
        )?;
        for descriptor in &self.bases {
            reg.definition(descriptor.clone())?;
        }
        let claims = &self.config.claims;
        let actions = claims.actions()?;
        let seal = reg.definition(super::protocol::participant_descriptor()?)?;
        if claims.participants().get(1) != Some(&seal) {
            return Err(bad("IC participant differs"));
        }
        reg.state(IcSorties::default());
        reg.owner_participant(
            seal,
            BTreeSet::from(actions.clone()),
            IcSortieParticipant {
                config: self.config.clone(),
            },
        )?;
        let reader = IcAdmissionReader::request(reg, manifest)?;
        let outcome = mush_core::mechanics::DefinitionRegistryBuilder::new()
            .register(
                mush_core::OwnerId::new(crate::NS).map_err(|e| bad(e))?,
                self.outcome()?,
            )
            .map_err(|e| bad(e))?;
        for (index, action) in actions.iter().enumerate() {
            let writer = self.config.writer.for_action::<IcSortieParticipant>(
                reg,
                claims,
                action,
                reader.clone(),
            )?;
            reg.action_provider(
                action.clone(),
                super::provider::Provider::new(
                    action.clone(),
                    outcome.clone(),
                    claims.clone(),
                    index == 1,
                    self.host.clone(),
                    writer,
                )?,
            )?;
        }
        reg.ic_mind_claim_capacity(manifest.clone(), claims.participants(), actions.clone())?;
        // Validate the deferred shared Roll cell at actual definition freeze,
        // preserving the existing Tacsim-before-Roll registration order.
        let expected = claims.clone();
        let writer = self.config.writer.clone();
        let descriptor = DefinitionDescriptor::new(
            DefinitionKind::new("ic_composition").map_err(|e| bad(e))?,
            DefinitionId::new("mind_custody").map_err(|e| bad(e))?,
            1,
            serde_json::json!({"format":1,"protocol":"ic_mind_composition_v1"}),
        )
        .map_err(|e| bad(e))?
        .with_references(
            std::iter::once(manifest.clone())
                .chain(claims.participants())
                .chain(actions)
                .collect(),
        );
        let shape = descriptor.clone();
        reg.definition_validator(
            DefinitionKind::new("ic_composition").map_err(|e| bad(e))?,
            move |ctx: &mush_core::DefinitionValidationContext<'_>| {
                crate::definitions::exact_validator(vec![shape.clone()])(ctx)?;
                writer
                    .validate_registration(&expected)
                    .map_err(|e| e.to_string())?;
                for r in std::iter::once(expected.manifest()).chain(expected.participants().iter())
                {
                    ctx.get_exact(r).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )?;
        reg.definition(descriptor)?;
        Ok(())
    }
    pub(super) fn loaded(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.config
            .definitions
            .set(
                ctx.reg
                    .definitions()
                    .ok_or_else(|| bad("IC definitions unavailable"))?
                    .clone(),
            )
            .map_err(|_| bad("IC owner already loaded"))
    }
}
