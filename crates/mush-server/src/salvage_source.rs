//! ---- E7r2: P2 — the `SalvageSource`, hosted at the composition root ----
//!
//! 10b §15.1 item 4. `mush_inventory::SalvageSource` is declared in `mush-inventory`
//! and implemented **here**, for the same reason `sim_routes::TacsimRouteCatalog` and
//! `industry_recipes` are: this is the one crate with edges to both halves. Whether a
//! carrier root is a recoverable wreck is `mush-sim`'s fact and where its cargo sits
//! is `mush-economy`'s, while the crate that must move the lot is `mush-inventory`,
//! whose edge set is exactly `{mush-core, mush-economy}` and is pinned by
//! `mush-inventory/tests/e7c2.rs`. **No `mush-inventory → mush-sim` edge appears**,
//! and that is the whole point of the seam.
//!
//! Read-only and bounded on records **visited** in three places: at most
//! `MAX_SALVAGE_JOURNEY_VISIT` journey records walked for at most `MAX_SALVAGE_WRECKS`
//! wrecks, at most `MAX_SALVAGE_SUBTREE` objects of one wreck's subtree, and at most
//! `MAX_SALVAGE_ENDPOINTS` endpoints reported — and the endpoint lookup is a **point
//! lookup** on the economy's own one-vault-per-backing index, never a walk of every
//! vault in the world.

use mush_core::{ObjId, World};
use mush_economy::{CustodyRef, EconomyStore};
use mush_inventory::{
    EndpointRef, MAX_SALVAGE_ENDPOINTS, MAX_SALVAGE_WRECKS, SalvageSource, WreckReport,
};
use mush_sim::{JourneyBook, JourneyState, SpatialState};

/// The most objects of one wreck's spatial subtree the endpoint search visits.
///
/// Bounded on records **visited** (D-LBK-5). A hull with a larger subtree than this
/// reports the endpoints found inside the budget and no more; the honest consequence
/// is that a deeply-nested hold is not offered, never that a lot is silently lost —
/// the lot stays exactly where it is, and staff still reach it through `@inv/show`.
pub const MAX_SALVAGE_SUBTREE: usize = 256;

/// The most journey records one wreck walk **visits** (E7r2's adversarial verify,
/// vf_a1).
///
/// D-LBK-5 bounds a walk on records **visited**, never on records matched, and
/// `SalvageSource::wrecks` shipped bounding only its result — so a world whose
/// journeys are mostly arrivals paid for all of them on every `@salvage` command.
/// It is deliberately far above `MAX_SALVAGE_WRECKS`, because the common case is a
/// long tail of `Arrived` records with a handful of wrecks at unpredictable keys, and
/// a visit budget as tight as the result budget would hide live wrecks behind old
/// arrivals.
pub const MAX_SALVAGE_JOURNEY_VISIT: usize = 4_096;

/// The one salvage oracle, handed to `InventoryModule::with_salvage_source` at
/// composition. Holds nothing: it reads the per-world `JourneyBook`, `SpatialState`
/// and `EconomyStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimSalvageSource;

/// The terminal loss states cut 6 records, and the only two a wreck can be in.
///
/// `Arrived` is not a loss and `Cruising`/`Promoted`/`Diverted` are still under way,
/// so neither is salvageable — a journey that is merely late must not become an
/// excuse to walk its cargo out.
fn is_wreck(state: &JourneyState) -> bool {
    matches!(state, JourneyState::Failed | JourneyState::Captured { .. })
}

/// Every inventory endpoint that belongs to `root` — its own vault, plus any vault
/// backed by an object inside it.
fn endpoints_of(world: &World, root: ObjId) -> Vec<EndpointRef> {
    let Some(economy) = world.module_state::<EconomyStore>() else {
        return Vec::new();
    };
    let snapshot = economy.snapshot();
    let mut objects = vec![root];
    if let Some(spatial) = world.module_state::<SpatialState>() {
        for child in spatial.descendant_edges(root) {
            if objects.len() >= MAX_SALVAGE_SUBTREE {
                break;
            }
            objects.push(child);
        }
    }
    let mut out = Vec::new();
    for object in objects {
        // A point lookup on the economy's own one-vault-per-backing index, for both
        // backing shapes that name a world object. `CustodyRef::Registered` names
        // none and is therefore not a wreck's endpoint by construction.
        for backing in [
            CustodyRef::Container(object),
            CustodyRef::SpatialInterior(object),
        ] {
            if out.len() >= MAX_SALVAGE_ENDPOINTS {
                return out;
            }
            if let Some(vault) = snapshot.vault_by_backing(&backing) {
                out.push(EndpointRef(vault.id));
            }
        }
    }
    out
}

impl SalvageSource for SimSalvageSource {
    fn wrecks(&self, world: &World, budget: usize) -> Vec<ObjId> {
        let Some(book) = world.module_state::<JourneyBook>() else {
            return Vec::new();
        };
        // `journeys` is a `BTreeMap` keyed by root, so this is object order and the
        // answer is deterministic.
        //
        // ---- vf_a1 (E7r2's adversarial verify): bounded on records **VISITED** ----
        //
        // This used to be a filter-then-take chain, which bounds the **result** and not
        // the walk: a world whose journeys are mostly arrivals visited every one of them
        // before it could take a single wreck, and `sites` is called three times per
        // `@salvage` command (the offer's projection, the command's pre-check and
        // `/status`). D-LBK-5 is *bounded on records visited, never on records matched*,
        // and the difference is the whole point of the rule — a walk that is cheap
        // because the world is currently small is not bounded.
        //
        // The visit budget is the wreck budget, so a world past it reports the first
        // wrecks in object order and no more. That is the honest failure: a wreck that
        // is not *offered* is not a wreck that is *lost* — the lot stays where it is,
        // and the pages fall as the earlier ones are recovered.
        let mut out = Vec::new();
        let ceiling = budget.min(MAX_SALVAGE_WRECKS);
        for (visited, (root, journey)) in book.journeys.iter().enumerate() {
            if visited >= MAX_SALVAGE_JOURNEY_VISIT || out.len() >= ceiling {
                break;
            }
            if is_wreck(journey.state()) {
                out.push(*root);
            }
        }
        out
    }

    fn wreck(&self, world: &World, root: ObjId) -> Option<WreckReport> {
        let book = world.module_state::<JourneyBook>()?;
        let journey = book.get(root)?;
        if !is_wreck(journey.state()) {
            return None;
        }
        let captured_by = match journey.state() {
            JourneyState::Captured { by } => Some(*by),
            _ => None,
        };
        // Cut 6 quarantines a stopped root into `recovery_room`; that is where the
        // hull is and it is what `@salvage/status` renders. It is read from the live
        // `SpatialState` rather than from the default constant, because a world may
        // declare its own.
        let site = world
            .module_state::<SpatialState>()
            .map_or(mush_core::LIMBO, |spatial| spatial.recovery_room);
        WreckReport::new(root, captured_by, site, endpoints_of(world, root))
    }
}
