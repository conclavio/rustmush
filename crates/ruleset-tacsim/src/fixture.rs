//! **The tiny corridor fixture** (B7; 10b §3.1's *"enough for cuts 2–4 to run and for
//! the owner to see output first"*).
//!
//! Two source-controlled packs, compiled in and **also** loadable from disk through
//! `[tacsim].pack_paths`/`seed_paths` — they are ordinary pack files, with their real
//! declared digests, so the fixture exercises exactly the path an operator's own pack
//! takes.
//!
//! | pack | kind | entries |
//! |---|---|---|
//! | [`CORE_PACK`] | rules | the ten commodity family **ids** of 10b §9 q10, and one recipe `alloy_smelt` |
//! | [`CORRIDOR_PACK`] | seed | three places — a Ceres extraction site, a transfer node, a Phobos yard — and one route joining Ceres to Phobos, `requires tacsim.core@1` |
//!
//! Fifteen entries against a cap of 64 for the fixture as a whole.
//!
//! # There are no quantities of anything
//!
//! 10b §9 q4, verbatim: *there is no interim.* The pack declares commodity **families**
//! and a recipe's declared input and output **lines**; it declares no stock, no pool
//! and no lot, and nothing in cut 1 consumes matter. An object-per-unit stand-in would
//! be a second quantity authority, which is exactly what Q13A exists to prevent, and it
//! would need migrating out one cut later.
//!
//! `metals` and `structural_alloys` are the two families the fixture actually puts to
//! work — they are `alloy_smelt`'s input and output — and the other eight are authored
//! so cut 2 inherits the list from **data** rather than from code.

use crate::config::{PackSource, TacsimConfig};

/// The rules pack's id.
pub const CORE_PACK: &str = "tacsim.core";

/// The campaign-seed pack's id.
pub const CORRIDOR_PACK: &str = "tacsim.corridor";

/// The rules pack, source-controlled and compiled in.
pub const CORE_JSON: &str = include_str!("fixture/tacsim_core.json");

/// The campaign-seed pack, source-controlled and compiled in.
pub const CORRIDOR_JSON: &str = include_str!("fixture/tacsim_corridor.json");

/// The recipe the fixture authors (10b §3.1: `alloy_smelt@1` — entry `alloy_smelt` at
/// pack version 1).
pub const RECIPE: &str = "alloy_smelt";

/// The route the fixture authors, joining Ceres to Phobos.
pub const ROUTE: &str = "ceres_phobos";

/// The three places the seed pack instantiates, in pack-local id order.
pub const PLACES: [&str; 3] = ["ceres_extraction", "phobos_yard", "transfer_node"];

/// S4A's ten commodity families, as 10b §9 q10 transliterates them.
///
/// Authored in the pack, so changing them needs no code — this array is the *test*
/// that the pack still declares them, never the source of truth.
pub const COMMODITY_FAMILIES: [&str; 10] = [
    "carbon_feedstock",
    "composites",
    "electronics",
    "industrial_parts",
    "metals",
    "munitions",
    "propellant",
    "strategic_elements",
    "structural_alloys",
    "volatiles",
];

/// The greatest number of entries the whole fixture may carry (10b §3.1).
pub const MAX_FIXTURE_ENTRIES: usize = 64;

/// The fixture as a **live** `[tacsim]` block, exactly as a host would hand it over.
#[must_use]
pub fn config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        packs: vec![PackSource {
            name: "<built-in tacsim.core>".to_string(),
            bytes: CORE_JSON.to_string(),
        }],
        seeds: vec![PackSource {
            name: "<built-in tacsim.corridor>".to_string(),
            bytes: CORRIDOR_JSON.to_string(),
        }],
        corridor: CORRIDOR_PACK.to_string(),
        active_domain_limit: crate::config::DEFAULT_ACTIVE_DOMAIN_LIMIT,
        ground_pulse_ms: crate::config::DEFAULT_GROUND_PULSE_MS,
        space_pulse_ms: crate::config::DEFAULT_SPACE_PULSE_MS,
        time: crate::config::TacsimTimeConfig::default(),
        // E7r1 R4: the fixture leaves the provider **disabled**. A fixture that
        // granted hulls would be a policy nobody wrote, and doc 10 §11's proving
        // loop is item 11's, not this pass's.
        recovery: crate::config::TacsimRecoveryConfig::default(),
    }
}
