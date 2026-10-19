//! Finite registered test content; every unit is created by the real Tactics owner.
use super::*;
use crate::core_production::refit::{
    self, CoreRefitFactoryConfig, PublishedRefitRecipe, RefitRecipeAuthority,
};
use mush_tactics::{
    CellId, CellSpec, ComponentId, ComponentSpec, ConditionId, InstallPoint, SectionId,
    SectionSpec, UnitSpec, UnitSpecBody,
};
#[derive(Clone)]
pub(super) struct Rules {
    pub specs: Vec<UnitSpec>,
    pub descriptors: Vec<DefinitionDescriptor>,
    pub revision: u32,
}
impl Rules {
    pub fn new() -> Result<Self> {
        let owner = OwnerId::new("tacsim")?;
        let mut b = DefinitionRegistryBuilder::new();
        let schema = DefinitionDescriptor::new(
            DefinitionKind::new("schema")?,
            DefinitionId::new("refit_rules")?,
            1,
            json!({"format":1}),
        )?;
        let schema_ref = b.register(owner.clone(), schema.clone())?;
        let mut descriptors = vec![schema];
        let mut specs = vec![];
        for n in 1..=3 {
            let d = DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new(format!("refit_core_{n}"))?,
                1,
                json!({"class":n}),
            )?;
            let component = b.register(owner.clone(), d.clone())?;
            descriptors.push(d);
            let body = UnitSpecBody {
                sections: vec![SectionSpec {
                    id: SectionId::new("hull")?,
                    condition: ConditionId::new("ok")?,
                    facets: vec![],
                    slots: vec![],
                    cells: vec![],
                }],
                unit_cells: vec![CellSpec {
                    id: CellId::new("heat")?,
                    min: 0,
                    max: 30,
                    init: 0,
                }],
                components: vec![ComponentSpec {
                    id: ComponentId::new("core")?,
                    definition: component,
                    install: InstallPoint::Unit,
                    condition: ConditionId::new("ok")?,
                    cells: vec![CellSpec {
                        id: CellId::new("class")?,
                        min: 0,
                        max: n,
                        init: n,
                    }],
                    asset: None,
                }],
                capabilities: vec![],
                mobility: None,
                rules: RulesValue::new(schema_ref.clone(), json!({}))?,
            };
            let d = body
                .descriptor(
                    DefinitionKind::new("unit")?,
                    DefinitionId::new(format!("refit_unit_{n}"))?,
                    1,
                )?
                .as_current();
            let reference = b.register(owner.clone(), d.clone())?;
            descriptors.push(d);
            specs.push(body.with_definition(reference));
        }
        Ok(Self {
            specs,
            descriptors,
            revision: 1,
        })
    }
    pub fn recipe(&self) -> Result<DefinitionDescriptor> {
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new("recipe")?,
            DefinitionId::new("core_refit")?,
            1,
            json!({"format":1,"fixture_revision":self.revision,"work_units":2}),
        )?)
    }
    fn recipe_ref(&self) -> Result<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("tacsim")?, self.recipe()?)?)
    }
    fn pack(&self) -> Result<crate::workorder::RecipeRef> {
        Ok(crate::workorder::RecipeRef::new(
            id("refit_fixture")?,
            self.revision,
            id("core_refit")?,
            DefinitionDigest::for_canonical_value(
                "refit.fixture.pack",
                &json!({"revision":self.revision}),
            )?,
        )?)
    }
    pub fn register(&self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let mut grouped =
            std::collections::BTreeMap::<DefinitionKind, Vec<DefinitionDescriptor>>::new();
        for d in &self.descriptors {
            grouped.entry(d.kind().clone()).or_default().push(d.clone());
        }
        for (kind, expected) in grouped {
            reg.definition_validator(
                kind,
                move |ctx: &mush_core::DefinitionValidationContext<'_>| {
                    let actual = ctx.definition();
                    if expected.iter().any(|d| {
                        let mut refs = d.references().to_vec();
                        refs.sort();
                        d.id() == &actual.reference().key().id
                            && d.version() == actual.reference().version()
                            && d.payload() == actual.payload()
                            && refs == actual.references()
                            && d.is_current() == actual.is_current()
                    }) {
                        Ok(())
                    } else {
                        Err("refit fixture definition changed".into())
                    }
                },
            )?;
        }
        for d in &self.descriptors {
            reg.definition(d.clone())?;
        }
        Ok(())
    }
    pub fn config(
        &self,
        f: &CoreFactoryConfig,
        mode: CoreOriginMode,
    ) -> Result<CoreRefitFactoryConfig> {
        let fitted_units: [DefinitionRef; 3] = self
            .specs
            .iter()
            .map(|s| s.definition.clone())
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| "three roots")?;
        let payment = mush_economy::core_refit::RefitPaymentPolicy {
            issuer: f.payment.issuer.clone(),
            instrument: f.payment.instrument.clone(),
            provider: f.payment.integrator.clone(),
            factory: f.payment.factory,
            recipe: self.recipe_ref()?,
            fitted_units,
        };
        let action = payment.action_ref(refit::INSTALL)?;
        let component = ComponentId::new("core")?;
        let roots = payment.fitted_units.iter().cloned().collect();
        let descriptor = mush_tactics::component_replacement::participant_descriptor(
            &action, &component, &roots,
        )?;
        let owner =
            DefinitionRegistryBuilder::new().register(OwnerId::new("tactics")?, descriptor)?;
        Ok(CoreRefitFactoryConfig::new(
            mode,
            payment,
            component,
            Arc::new(self.clone()),
            owner,
        )?)
    }
    pub fn tactics(&self, c: &CoreRefitFactoryConfig) -> Result<mush_tactics::TacticsModule> {
        let mut b = mush_tactics::UnitCatalogBuilder::new();
        let vocabulary = mush_tactics::MobilityVocabulary::base().freeze();
        for spec in &self.specs {
            b.add(spec.clone(), &vocabulary)?;
        }
        Ok(
            mush_tactics::TacticsModule::with_catalog(b.freeze()).with_component_replacement(
                c.replacement_policy(),
                c.action(refit::INSTALL)?,
                c.component().clone(),
                self.specs.iter().map(|s| s.definition.clone()).collect(),
            ),
        )
    }
}
impl RefitRecipeAuthority for Rules {
    fn resolve(&self, _: &mush_core::World, class: DieClass) -> Option<PublishedRefitRecipe> {
        PublishedRefitRecipe::new(self.pack().ok()?, self.recipe_ref().ok()?, class, 2).ok()
    }
    fn published_digest(
        &self,
        _: &mush_core::World,
        r: &crate::workorder::RecipeRef,
    ) -> Option<DefinitionDigest> {
        (r == &self.pack().ok()?).then(|| r.digest())
    }
}
