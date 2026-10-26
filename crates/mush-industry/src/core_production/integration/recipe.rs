//! Trusted content adapter for the real published recipe. Its pack reference is
//! retained separately from the mechanics registry's fixed recipe definition.
use super::{IntegrationError as Error, MAX_WORK_UNITS};
use crate::core_production::DieClass;
use crate::workorder::RecipeRef;
use mush_core::{DefinitionDigest, DefinitionRef, World};
use serde::{Deserialize, Serialize};

/// Immutable content returned by the adapter installed by trusted composition.
/// It is not accepted as an action parameter and grants no money/input authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedCoreRecipe {
    pack: RecipeRef,
    definition: DefinitionRef,
    class: DieClass,
    work_units: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeWire {
    pack: RecipeRef,
    definition: DefinitionRef,
    class: DieClass,
    work_units: u64,
}
impl<'de> Deserialize<'de> for PublishedCoreRecipe {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = RecipeWire::deserialize(d)?;
        Self::new(w.pack, w.definition, w.class, w.work_units).map_err(serde::de::Error::custom)
    }
}
impl PublishedCoreRecipe {
    pub fn new(
        pack: RecipeRef,
        definition: DefinitionRef,
        class: DieClass,
        work_units: u64,
    ) -> Result<Self, Error> {
        let r = Self {
            pack,
            definition,
            class,
            work_units,
        };
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.pack.id().as_str() != "core_integrate"
            || self.pack.digest().as_bytes() == &[0; 32]
            || self.definition.key().kind.as_str() != "recipe"
            || self.definition.key().id.as_str() != "core_integrate"
            || self.definition.version() != 1
            || self.definition.digest().as_bytes() == &[0; 32]
            || !(1..=MAX_WORK_UNITS).contains(&self.work_units)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub const fn pack(&self) -> &RecipeRef {
        &self.pack
    }
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }
    pub const fn class(&self) -> DieClass {
        self.class
    }
    pub const fn work_units(&self) -> u64 {
        self.work_units
    }
}

/// No default adapter exists. Implementations resolve actual published content
/// and its exact pack digest; a catalogue label or caller-written recipe is not
/// a substitute. Both lookups are repeated before custody by the owner fence.
pub trait CoreRecipeAuthority: Send + Sync + 'static {
    fn resolve(&self, world: &World, class: DieClass) -> Option<PublishedCoreRecipe>;
    fn published_digest(&self, world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest>;
}
