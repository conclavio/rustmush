//! Trusted Industry adapters. A parsed pack and its mechanics references are
//! inert until this world's actual Tacsim owner has published the same bytes.
use super::content::{CATALOGUE, CLASSES, PACK, VERSION};
use super::{HardwareCatalogueDef, HardwareProcess, HardwareRecipeDef};
use crate::{
    TacsimStore,
    definitions::DefinitionError,
    pack::{PackEntry, RulesPack},
};
use mush_core::{
    CanonicalId, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRef, DefinitionRegistryBuilder, ModError, OwnerId, Registrar, World,
};
use mush_economy::{core_integration::CoreClass, core_refit::RefitKind};
use mush_industry::{
    RecipeRef,
    core_production::{
        DieClass,
        integration::{CoreRecipeAuthority, PublishedCoreRecipe},
        refit::{PublishedRefitRecipe, RefitRecipeAuthority},
    },
};
use serde_json::json;
use std::any::TypeId;

pub(super) const SOURCE: &str = include_str!("../../content/hardware/governed_hardware.json");
fn bad(error: impl std::fmt::Display) -> DefinitionError {
    DefinitionError::value("governed hardware recipes", error.to_string())
}
fn core(class: DieClass) -> CoreClass {
    CLASSES[class.index()]
}

/// The exact shipped launch catalogue and its three recipes (doc 17h cut M8b
/// added the ungoverning refit's).
/// Constructing this object, registering its definitions or loading its pack
/// does not publish it, create a facility, mint stock or authorize a purchase.
#[derive(Clone, Debug)]
pub struct PublishedHardware {
    pack: RulesPack,
    catalogue: HardwareCatalogueDef,
    recipes: [HardwareRecipeDef; 3],
    descriptors: [DefinitionDescriptor; 3],
    definitions: [DefinitionRef; 3],
}
impl PublishedHardware {
    pub fn shipped() -> Result<Self, DefinitionError> {
        let pack = crate::pack::load_pack(SOURCE).map_err(bad)?;
        if pack.manifest().id().as_str() != PACK
            || pack.manifest().version() != VERSION
            || pack.entries().len() != 4
        {
            return Err(bad("unexpected shipped pack identity or entries"));
        }
        let Some(PackEntry::HardwareCatalogue(catalogue)) =
            pack.entry(&CanonicalId::new(CATALOGUE).map_err(bad)?)
        else {
            return Err(bad("shipped catalogue missing"));
        };
        catalogue.validate()?;
        let catalogue = catalogue.clone();
        let read = |process: HardwareProcess| -> Result<HardwareRecipeDef, DefinitionError> {
            let Some(PackEntry::HardwareRecipe(recipe)) =
                pack.entry(&CanonicalId::new(process.recipe_id()).map_err(bad)?)
            else {
                return Err(bad("shipped certified recipe missing"));
            };
            if recipe.process() != process {
                return Err(bad("shipped recipe process disagrees"));
            }
            recipe.validate()?;
            Ok(recipe.clone())
        };
        let recipes = [
            read(HardwareProcess::Integrate)?,
            read(HardwareProcess::Refit)?,
            read(HardwareProcess::Ungovern)?,
        ];
        let descriptor = |index: usize| -> Result<DefinitionDescriptor, DefinitionError> {
            let recipe = &recipes[index];
            DefinitionDescriptor::new(
                DefinitionKind::new("recipe").map_err(bad)?,
                DefinitionId::new(recipe.process().recipe_id()).map_err(bad)?,
                VERSION,
                json!({"format":1, "pack":pack.manifest().id(),
                    "pack_version":VERSION, "pack_digest":pack.manifest().digest(),
                    "catalogue":catalogue, "recipe":recipe,
                    "facility_capability":recipe.process().facility_capability()}),
            )
            .map_err(bad)
        };
        let descriptors = [descriptor(0)?, descriptor(1)?, descriptor(2)?];
        let mut registry = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new(crate::NS).map_err(bad)?;
        let definitions = [
            registry
                .register(owner.clone(), descriptors[0].clone())
                .map_err(bad)?,
            registry
                .register(owner.clone(), descriptors[1].clone())
                .map_err(bad)?,
            registry
                .register(owner, descriptors[2].clone())
                .map_err(bad)?,
        ];
        Ok(Self {
            pack,
            catalogue,
            recipes,
            descriptors,
            definitions,
        })
    }
    pub fn pack(&self) -> &RulesPack {
        &self.pack
    }
    pub fn catalogue(&self) -> &HardwareCatalogueDef {
        &self.catalogue
    }
    pub fn definition(&self, process: HardwareProcess) -> &DefinitionRef {
        &self.definitions[index(process)]
    }
    /// Registers exact recipe definitions on Tacsim's actual owner registrar.
    /// No actor, account or stock is synthesized here.
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        further: Option<&DefinitionDescriptor>,
    ) -> Result<(), ModError> {
        // Doc 31 UD.4b: one exact recipe validator per owner, so a composed
        // bay's recipe joins the shipped hardware recipes here.
        let mut all = self.descriptors.to_vec();
        all.extend(further.cloned());
        reg.definition_validator(
            DefinitionKind::new("recipe").map_err(|e| ModError::msg(e.to_string()))?,
            crate::definitions::exact_validator(all.clone()),
        )?;
        for descriptor in all {
            reg.definition(descriptor)?;
        }
        Ok(())
    }
    fn current_pack(&self, world: &World) -> Option<DefinitionDigest> {
        if world
            .module_state_owner(TypeId::of::<TacsimStore>())
            .map(|owner| owner.as_str())
            != Some(crate::NS)
        {
            return None;
        }
        let store = world.module_state::<TacsimStore>()?;
        if !store.enabled() {
            return None;
        }
        let manifest = self.pack.manifest();
        let loaded = store.pack(manifest.id(), manifest.version())?;
        let digest = store.published_digest(manifest.id(), manifest.version())?;
        if digest != manifest.digest() || loaded.pack != self.pack {
            return None;
        }
        Some(digest)
    }
    fn pin(&self, world: &World, process: HardwareProcess) -> Option<RecipeRef> {
        RecipeRef::new(
            self.pack.manifest().id().clone(),
            VERSION,
            CanonicalId::new(process.recipe_id()).ok()?,
            self.current_pack(world)?,
        )
        .ok()
    }
    fn current_recipe(
        &self,
        world: &World,
        recipe: &RecipeRef,
        process: HardwareProcess,
    ) -> Option<DefinitionDigest> {
        (self.pin(world, process).as_ref() == Some(recipe)).then_some(recipe.digest())
    }
}
fn index(process: HardwareProcess) -> usize {
    match process {
        HardwareProcess::Integrate => 0,
        HardwareProcess::Refit => 1,
        HardwareProcess::Ungovern => 2,
    }
}
impl CoreRecipeAuthority for PublishedHardware {
    fn resolve(&self, world: &World, class: DieClass) -> Option<PublishedCoreRecipe> {
        let process = HardwareProcess::Integrate;
        PublishedCoreRecipe::new(
            self.pin(world, process)?,
            self.definition(process).clone(),
            class,
            self.recipes[index(process)].work_units(core(class)),
        )
        .ok()
    }
    fn published_digest(&self, world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        self.current_recipe(world, recipe, HardwareProcess::Integrate)
    }
}
impl RefitRecipeAuthority for PublishedHardware {
    fn resolve(&self, world: &World, class: DieClass) -> Option<PublishedRefitRecipe> {
        self.resolve_kind(world, RefitKind::CoreRefit, class)
    }
    /// Doc 17h cut M8b C2: the ungoverning refit resolves its own published
    /// recipe, timed like a fitting.
    fn resolve_kind(
        &self,
        world: &World,
        kind: RefitKind,
        class: DieClass,
    ) -> Option<PublishedRefitRecipe> {
        let process = refit_process(kind);
        PublishedRefitRecipe::new(
            self.pin(world, process)?,
            self.definition(process).clone(),
            class,
            self.recipes[index(process)].work_units(core(class)),
        )
        .ok()
    }
    fn published_digest(&self, world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        self.current_recipe(world, recipe, HardwareProcess::Refit)
            .or_else(|| self.current_recipe(world, recipe, HardwareProcess::Ungovern))
    }
}
const fn refit_process(kind: RefitKind) -> HardwareProcess {
    match kind {
        RefitKind::CoreRefit => HardwareProcess::Refit,
        RefitKind::CoreUngovern => HardwareProcess::Ungovern,
    }
}
