//! E7c4 B2 — the `RecipeAuthority` implementation, hosted at the composition
//! root so **`mush-industry` gains no `ruleset-tacsim` edge** (DECIDED: the recipe
//! reaches industry by composition, not by an edge). It reads the ruleset's
//! published packs through `TacsimStore` and answers a `RecipeRef` from the pack's
//! own published digest — no second digest, no second registry, no callback into
//! a ruleset at prepare time (the industry provider consults this only at offer /
//! draft time, where a world is available).

use mush_core::{CanonicalId, DefinitionDigest, World};
use mush_industry::{PinnedRecipe, RecipeAuthority, RecipeLine, RecipeRef};
use ruleset_tacsim::TacsimStore;

/// The one recipe authority, handed to `IndustryModule::with_recipes` at
/// composition. Holds nothing: it reads the per-world `TacsimStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimRecipeAuthority;

fn pin_lines(lines: &[ruleset_tacsim::RecipeLine]) -> Option<Vec<RecipeLine>> {
    lines
        .iter()
        .map(|line| RecipeLine::new(line.commodity.clone(), line.units).ok())
        .collect()
}

impl RecipeAuthority for TacsimRecipeAuthority {
    fn digest_of(&self, world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        // The pack's current published digest — `None` for an unpublished (retired)
        // version, which is exactly "fails closed".
        world
            .module_state::<TacsimStore>()?
            .published_digest(recipe.pack(), recipe.version())
    }

    fn resolve(
        &self,
        world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)> {
        let store = world.module_state::<TacsimStore>()?;
        let (digest, def) = store.recipe(pack, version, id)?;
        let recipe = RecipeRef::new(pack.clone(), version, id.clone(), digest).ok()?;
        let inputs = pin_lines(&def.inputs)?;
        let outputs = pin_lines(&def.outputs)?;
        let pinned =
            PinnedRecipe::new(inputs, outputs, def.requires.clone(), def.work_units).ok()?;
        Some((recipe, pinned))
    }
}
