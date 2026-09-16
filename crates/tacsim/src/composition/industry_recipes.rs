//! E7c4 B2 — the `RecipeAuthority` implementation, hosted at the composition
//! root so **`mush-industry` gains no `tacsim` edge** (DECIDED: the recipe
//! reaches industry by composition, not by an edge). It reads the ruleset's
//! published packs through `TacsimStore` and answers a `RecipeRef` from the pack's
//! own published digest — no second digest, no second registry, no callback into
//! a ruleset at prepare time (the industry provider consults this only at offer /
//! draft time, where a world is available).

use mush_core::{CanonicalId, DefinitionDigest, World};
use mush_industry::{PinnedRecipe, RecipeAuthority, RecipeLine, RecipeRef};
use crate::TacsimStore;

/// The one recipe authority, handed to `IndustryModule::with_recipes` at
/// composition. Holds nothing: it reads the per-world `TacsimStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimRecipeAuthority;

fn pin_lines(lines: &[crate::RecipeLine]) -> Option<Vec<RecipeLine>> {
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

    fn catalog_revision(&self, world: &World) -> u64 {
        // A fold over exactly what `catalog` filters on: the published, still
        // loaded `(pack, version)` rows and the digest each was published under.
        // A pack's digest covers its entries, so this moves when — and only when —
        // the catalog's content would move. Cheap: the published ledger is a
        // handful of rows, where the catalog walk is every entry of every pack.
        let Some(store) = world.module_state::<TacsimStore>() else {
            return 0;
        };
        let mut acc = FNV_OFFSET;
        for record in store.published() {
            let (pack, version) = (record.id(), record.version());
            if store.pack(pack, version).is_none() {
                continue;
            }
            acc = fold(acc, pack.as_str().as_bytes());
            acc = fold(acc, &version.to_be_bytes());
            acc = fold(acc, record.digest().as_bytes());
        }
        acc
    }

    fn catalog(&self, world: &World) -> Vec<(RecipeRef, PinnedRecipe)> {
        // `formation_kinds`' shape, for the same seam and the same reason: the
        // published ledger is a `BTreeMap` and so is a pack's entry table, so two
        // walks of one unchanged world answer the identical sequence. A pinned
        // line the industry crate's own constructor refuses is skipped rather
        // than guessed at — an unpinnable recipe is simply not offered.
        let Some(store) = world.module_state::<TacsimStore>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for record in store.published() {
            let (pack, version) = (record.id(), record.version());
            let Some(loaded) = store.pack(pack, version) else {
                continue;
            };
            for (id, entry) in loaded.pack.entries() {
                let crate::PackEntry::Recipe(def) = entry else {
                    continue;
                };
                let Ok(reference) =
                    RecipeRef::new(pack.clone(), version, id.clone(), record.digest())
                else {
                    continue;
                };
                let (Some(inputs), Some(outputs)) =
                    (pin_lines(&def.inputs), pin_lines(&def.outputs))
                else {
                    continue;
                };
                let Ok(pinned) =
                    PinnedRecipe::new(inputs, outputs, def.requires.clone(), def.work_units)
                else {
                    continue;
                };
                out.push((reference, pinned));
            }
        }
        out
    }
}

/// FNV-1a's 64-bit offset basis and prime — a fold, not a digest: the value only
/// has to move when the published set does, and it never leaves this process.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fold(mut acc: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        acc ^= u64::from(*byte);
        acc = acc.wrapping_mul(FNV_PRIME);
    }
    acc
}
