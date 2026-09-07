//! E7c7 (cut 6 fold, B6 — tracked to this cut): the `RouteCatalog`
//! implementation, hosted at the composition root so **`mush-sim` gains no
//! `ruleset-tacsim` edge**.
//!
//! Cut 6's verify found `@journey/plan|depart <route id>` to be a label only:
//! `build_route` fabricated a fresh `RouteDef` at the `manual` drive class with
//! zero declared propellant and no alignment band, so `RouteDef::propellant()`,
//! `alignment_band()` and `route_duration` had no caller at all. The verify's
//! own fix ("resolve the id against cut 1's pack, which needs no edge") **cannot
//! be built**: the authored pack store is `ruleset_tacsim::TacsimStore` and
//! `ruleset-tacsim` depends on `mush-sim`, so the lookup would be a cycle. It is
//! therefore a composition seam in the `with_accounts` / `with_recipes` shape —
//! the exact same answer cut 4 gave for recipes.
//!
//! Read-only and bounded: one `BTreeMap` lookup against the store's own
//! materialized index, and the `RouteDef` handed back is the store's, which
//! already re-ran every constructor invariant when it was materialized.

use mush_core::{CanonicalId, World};
use mush_sim::{RouteCatalog, RouteDef};
use ruleset_tacsim::TacsimStore;

/// The one route catalog, handed to `SimulationModule::with_route_catalog` at
/// composition. Holds nothing: it reads the per-world `TacsimStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimRouteCatalog;

impl RouteCatalog for TacsimRouteCatalog {
    fn resolve(&self, world: &World, id: &CanonicalId) -> Option<RouteDef> {
        // `TacsimStore::route` answers for the pack version that governs the id
        // (`TacsimStore::resolve`'s rule, the same one every other subject goes
        // through). A route whose places are not all placed was skipped loudly
        // at reindex time and is simply absent here — which is "fall back to the
        // manual route", never "hand out a half-resolved route".
        world.module_state::<TacsimStore>()?.route(id).cloned()
    }
}
