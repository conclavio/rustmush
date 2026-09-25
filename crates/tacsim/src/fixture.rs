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

/// ---- E7r2: P5 — **the whole Mars–Phobos–Ceres corridor, as `tacsim.corridor@2`**
/// ----
///
/// 10b §15.1 item 9's content half (9b), against doc 10 §11: *"a Mars ground theater,
/// a Phobos shipyard and orbital domain, a Ceres mining/freeport region, one
/// interplanetary route, one refinery, one open market"*. Twelve entries against
/// [`MAX_FIXTURE_ENTRIES`]: v1's six, plus the Ceres freeport and its refinery, the
/// Mars orbital approach, the Tharsis ground theater and its objective, and the
/// `phobos_mars` route that joins them — with E7 residue pass 1's `hazard`,
/// `interdiction` and `controller` authored, because R2 landed those fields and a
/// corridor that cannot be interdicted cannot host §11's *"fight or evade an
/// interception"*.
///
/// **It is a NEW VERSION, and v1 is left byte-identical on purpose** (T1/Q18A:
/// versions coexist and a consumer stays pinned until an explicit migration). A
/// published pack is *"immutable and referenced by digest"*, so re-deriving v1's
/// digest over new content is exactly the in-place edit that immutability forbids —
/// every world seeded from `tacsim.corridor@1`, and every hauling contract's route
/// assumptions with it, would be re-priced, re-timed and re-resolved by a file that
/// changed underneath them. Publishing v2 beside v1 re-prices, re-times and resolves
/// **nothing**, and `tacsim/tests/e7r1.rs`'s digest pin stays green and
/// unmodified as the proof.
///
/// This pack's own digest is **re-derived, never hand-edited** — a hand-edited one
/// fails `PackError::DigestMismatch` naming both, which `e7r2.rs` drives.
pub const CORRIDOR_V2_JSON: &str = include_str!("fixture/tacsim_corridor_v2.json");

/// The recipe the fixture authors (10b §3.1: `alloy_smelt@1` — entry `alloy_smelt` at
/// pack version 1).
pub const RECIPE: &str = "alloy_smelt";

/// The route the fixture authors, joining Ceres to Phobos.
pub const ROUTE: &str = "ceres_phobos";

/// The three places the seed pack instantiates, in pack-local id order.
pub const PLACES: [&str; 3] = ["ceres_extraction", "phobos_yard", "transfer_node"];

/// The route `tacsim.corridor@2` adds, joining Phobos to the Mars ground theater
/// (E7r2 P5).
pub const ROUTE_MARS: &str = "phobos_mars";

/// The seven places `tacsim.corridor@2` instantiates, in pack-local id order.
pub const PLACES_V2: [&str; 7] = [
    "ceres_extraction",
    "ceres_freeport",
    "mars_objective",
    "mars_orbit",
    "mars_theater",
    "phobos_yard",
    "transfer_node",
];

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
        // 16b cut G: the combat arena's five blocks, at their defaults — which
        // are 16b §9's constants table. The literal stays exhaustive rather than
        // taking `..Default::default()`, so a block added later has to be
        // considered here rather than silently appearing in every fixture.
        arena: crate::config::ArenaConfig::default(),
        match_: crate::config::MatchConfig::default(),
        // 16b cut Y.2: the span and the rate, at their defaults.
        command: crate::config::CommandConfig::default(),
        sensors: crate::config::SensorsConfig::default(),
        resolution: crate::config::ResolutionConfig::default(),
        harness: crate::config::HarnessConfig::default(),
    }
}

/// **The fixture with the whole §11 corridor seeded** — `tacsim.core@1` plus
/// `tacsim.corridor@2` (E7r2 P5).
///
/// [`config`] is deliberately left on v1 and is byte-identical to what it has always
/// returned: a default composition seeds exactly what it seeded before, so nothing a
/// running world holds is re-priced, re-timed or re-resolved by this pass. A host that
/// wants the whole corridor asks for it, which is what a version that coexists **means**
/// (T1/Q18A). Doc 10 §11's proving loop drives this one.
#[must_use]
pub fn config_v2() -> TacsimConfig {
    TacsimConfig {
        seeds: vec![PackSource {
            name: "<built-in tacsim.corridor@2>".to_string(),
            bytes: CORRIDOR_V2_JSON.to_string(),
        }],
        ..config()
    }
}
