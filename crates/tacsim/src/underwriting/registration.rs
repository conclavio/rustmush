//! Closed explicit composition. Definitions/configuration alone never mint a
//! region host transition, premium authority or a qualified role proof.
use super::{
    CoverageError, CoverageProductV1,
    authority::Readiness,
    families::Families,
    host::{CoverageHost, Installed},
    metadata,
    protocol::Protocol,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    ModError, ObjId, OwnerId, Registrar,
};
use mush_roll::commerce::{
    coverage::{CoverageOpenReader, CoverageProtocol, CoverageWriter},
    module::CommerceConfig,
    purchase::PurchaseParticipants,
};
use std::collections::BTreeSet;
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Clone)]
pub struct CoverageConfiguration {
    protocol: Protocol,
    writer: CoverageWriter,
    service: ObjId,
    products: Vec<CoverageProductV1>,
    conduct: Option<super::qualification::Configuration>,
}
impl CoverageConfiguration {
    pub fn new(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        service: ObjId,
        products: Vec<CoverageProductV1>,
    ) -> Result<Self, ModError> {
        Self::build(manifest, participants, service, products, None)
    }
    /// Qualified entries explicitly select real Conduct operations in approval
    /// order. Construction grants nothing; Publish reads and retains each proof.
    pub fn with_conduct(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        service: ObjId,
        products: Vec<CoverageProductV1>,
        conduct_manifest: DefinitionRef,
        selections: std::collections::BTreeMap<
            mush_core::DefinitionDigest,
            Vec<mush_core::CommitToken>,
        >,
    ) -> Result<Self, ModError> {
        let conduct = super::qualification::Configuration {
            manifest: conduct_manifest,
            selections,
        };
        conduct.validate(&products).map_err(bad)?;
        Self::build(manifest, participants, service, products, Some(conduct))
    }
    fn build(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        service: ObjId,
        products: Vec<CoverageProductV1>,
        conduct: Option<super::qualification::Configuration>,
    ) -> Result<Self, ModError> {
        let protocol = Protocol {
            format: 1,
            manifest,
            participants: participants.ordered(),
        };
        protocol.validate().map_err(bad)?;
        if service.0 == 0 || products.len() > 16 {
            return Err(bad("coverage configuration bound"));
        }
        let mut ids = BTreeSet::new();
        for p in &products {
            if p.adjudicator() != service
                || (!p.approvals().is_empty() && conduct.is_none())
                || !ids.insert(p.digest().map_err(bad)?)
            {
                return Err(bad("coverage exact ordinary product configuration"));
            }
        }
        Ok(Self {
            protocol,
            writer: CoverageWriter::new(),
            service,
            products,
            conduct,
        })
    }
    pub fn configure_roll(&self, config: CommerceConfig) -> Result<CommerceConfig, ModError> {
        config.with_hull_coverage(self.protocol.roll(), self.writer.clone())
    }
    pub fn configure_economy(
        &self,
        module: mush_economy::EconomyModule,
    ) -> Result<mush_economy::EconomyModule, ModError> {
        Ok(module.with_hull_coverage(
            mush_economy::managed_coverage::CoverageConfig::new(
                self.service,
                [
                    self.protocol.action(2).map_err(bad)?,
                    self.protocol.action(3).map_err(bad)?,
                    self.protocol.action(4).map_err(bad)?,
                ],
            )
            .map_err(bad)?,
        ))
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], CoverageError> {
        self.protocol.actions()
    }
    pub(crate) fn layer(&self, layer: &crate::TacsimRuleset) -> Result<Registration, ModError> {
        if !layer.config().enabled
            || layer
                .registry_commerce
                .as_ref()
                .is_none_or(|r| r.reference() != &self.protocol.manifest)
        {
            return Err(bad("coverage requires actual enabled Registry"));
        }
        let ic = layer
            .ic_host()
            .ok_or_else(|| bad("coverage requires actual IC composition"))?;
        Ok(Registration {
            config: self.clone(),
            host: CoverageHost::new(self.protocol.clone(), self.writer.clone(), ic, self.service),
            readiness: Readiness::default(),
        })
    }
}
#[derive(Clone)]
pub(crate) struct Registration {
    config: CoverageConfiguration,
    pub(crate) host: CoverageHost,
    readiness: Readiness,
}
impl Registration {
    pub(crate) fn descriptors(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        (0..5)
            .map(|n| self.config.protocol.descriptor(n).map_err(bad))
            .collect()
    }
    pub(crate) fn outcome(&self) -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(bad)?,DefinitionId::new("hull_coverage_result").map_err(bad)?,1,
            serde_json::json!({"format":1,"protocol":"hull_coverage_result_v1","fields":["operation","kind"],"publication_digest_optional":true})).map_err(bad)
    }
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        families: Families,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS {
            return Err(bad("coverage actual owner"));
        }
        let protocol = &self.config.protocol;
        let actions = protocol.actions().map_err(bad)?;
        let conduct_reader = self
            .config
            .conduct
            .as_ref()
            .map(|c| mush_roll::commerce::conduct::ConductReader::request(reg, &c.manifest))
            .transpose()?;
        let participant = reg.definition(metadata::descriptor()?)?;
        reg.owner_participant(
            participant,
            BTreeSet::from([actions[0].clone()]),
            metadata::Participant {
                protocol: protocol.clone(),
                readiness: self.readiness.clone(),
                conduct_manifest: self.config.conduct.as_ref().map(|c| c.manifest.clone()),
            },
        )?;
        let owner = reg.request_owner_participant::<metadata::Participant>(actions[0].clone())?;
        let publication_writer =
            self.config
                .writer
                .for_publication(reg, &protocol.roll(), &actions[0])?;
        let outcome = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(bad)?, self.outcome()?)
            .map_err(bad)?;
        reg.action_provider(
            actions[0].clone(),
            super::publication::Provider {
                protocol: protocol.clone(),
                action: actions[0].clone(),
                outcome: outcome.clone(),
                families: families.clone(),
                products: self.config.products.clone(),
                owner,
                readiness: self.readiness.clone(),
                refusal: super::publication::refusal()?,
                conduct: self.config.conduct.clone(),
                conduct_reader: conduct_reader.clone(),
                writer: publication_writer,
            },
        )?;
        let reader = CoverageOpenReader::request(reg, &protocol.manifest)?;
        for index in 1..5 {
            let writer = self.config.writer.for_action(
                reg,
                &protocol.roll(),
                &actions[index],
                reader.clone(),
            )?;
            let economy = if index >= 2 {
                Some(reg.request_owner_participant::<mush_economy::registry_filing::InitialFilingParticipant>(actions[index].clone())?)
            } else {
                None
            };
            reg.action_provider(
                actions[index].clone(),
                super::provider::Provider {
                    protocol: protocol.clone(),
                    action: actions[index].clone(),
                    outcome: outcome.clone(),
                    index,
                    families: families.clone(),
                    host: self.host.clone(),
                    writer,
                    economy,
                    refusal: super::publication::refusal()?,
                    conduct_reader: conduct_reader.clone(),
                },
            )?;
        }
        reg.hull_coverage_capacity(
            protocol.manifest.clone(),
            protocol.participants.clone(),
            [actions[2].clone(), actions[3].clone(), actions[4].clone()],
        )?;
        let mut refs = vec![protocol.manifest.clone()];
        refs.extend(protocol.participants.clone());
        refs.extend(actions);
        if let Some(c) = &self.config.conduct {
            refs.push(c.manifest.clone());
        }
        refs.sort();
        let (version, payload) = if self.config.conduct.is_some() {
            (
                2,
                serde_json::json!({"format":2,"protocol":"hull_coverage_owner_union_v2","qualification":"actual_conduct_publication_and_current_read"}),
            )
        } else {
            (
                1,
                serde_json::json!({"format":1,"protocol":"hull_coverage_owner_union_v1","ordinary_only":true}),
            )
        };
        let descriptor = DefinitionDescriptor::new(
            DefinitionKind::new("coverage_composition").map_err(bad)?,
            DefinitionId::new("hull_coverage").map_err(bad)?,
            version,
            payload,
        )
        .map_err(bad)?
        .with_references(refs);
        let shape = descriptor.clone();
        let writer = self.config.writer.clone();
        let expected = protocol.roll();
        reg.definition_validator(
            DefinitionKind::new("coverage_composition").map_err(bad)?,
            move |ctx: &mush_core::DefinitionValidationContext<'_>| {
                crate::definitions::exact_validator(vec![shape.clone()])(ctx)?;
                writer
                    .validate_registration(&expected)
                    .map_err(|e| e.to_string())?;
                for reference in
                    std::iter::once(&expected.manifest).chain(expected.participants.iter())
                {
                    ctx.get_exact(reference).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )?;
        reg.definition(descriptor)?;
        self.host
            .installed
            .set(Installed {
                families,
                readiness: self.readiness.clone(),
            })
            .map_err(|_| bad("coverage installed twice"))
    }
    pub(crate) fn loaded(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        self.readiness.capture(ctx)
    }
    pub(crate) fn service(&self) -> ObjId {
        self.config.service
    }
    pub(crate) fn writer(&self) -> &CoverageWriter {
        &self.config.writer
    }
}

pub(crate) fn register_base(reg: &mut Registrar<'_>) -> Result<Families, ModError> {
    reg.state(metadata::Publications::default());
    Families::register(reg)
}
