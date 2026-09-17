//! **The game's content, compiled in** (16b cut C.2; doc 16 §4 "Content
//! packs").
//!
//! Nine files under `crates/tacsim/content/`, `include_str!`ed here and pinned
//! by digest in `crates/tacsim/tests/content.rs`, in the shape
//! `mush_atlas::starter` already uses for the lot: **the game's own content
//! loads on every boot without an operator naming a path**.
//!
//! # Two routes, one loader, one place they meet
//!
//! `[tacsim].pack_paths` is the operator's route: the host reads the bytes and
//! [`crate::verify_sources`] parses and verifies them into the
//! [`crate::store::TacsimStore`]'s pack ledger, which `@tacsim/pack` reads. The
//! built-in route is the same bytes from `include_str!` through
//! [`sources`]. **Both go through one [`crate::pack::load_pack`]** — one strict
//! wire, one digest verification, one set of entry bounds — and both land in one
//! [`crate::definitions::Catalog::build_game`], which is what the composition
//! root's unit catalog, the module's registry descriptors and the arena
//! applier's bindings are all three readings of. There is no second loader and
//! no second validation, which is the only way two routes can be made to agree.
//!
//! What the built-in route deliberately does **not** enter is the operator's
//! pack *ledger*: no `tacsim:pack:` row, no `@tacsim/pack list` line, and
//! `[tacsim].enabled = false` stays E7 cut 1's promise that the layer is
//! bit-identical to one built before it existed. A compiled-in pack cannot
//! change under a running world, so the row that exists to catch a re-signed
//! file has nothing to catch; what a *changed* shipped definition does is move
//! its [`mush_core::mechanics::DefinitionRef`] digest, and a persisted unit
//! pointing at the old one then fails M32's own exact-definition check at boot —
//! which is the layer that owns the question. An operator who names one of these
//! files on `pack_paths` anyway gets it in the ledger **and** deduplicated out
//! of the catalog by `(id, version)`, so the definitions are registered once.
//!
//! # What ships, and through which latch
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `ammunition/he_105.json` | `tacsim.pack.he_105` | rules | never — a definition |
//! | `ammunition/ball_762.json` | `tacsim.pack.ball_762` | rules | never |
//! | `weapons/cannon_105.json` | `tacsim.pack.cannon_105` | rules | never |
//! | `weapons/mg_coax.json` | `tacsim.pack.mg_coax` | rules | never |
//! | `sensors/visual_mk1.json` | `tacsim.pack.visual_mk1` | rules | never |
//! | `units/line_tank.json` | `tacsim.pack.line_tank` | rules | never |
//! | `resolution.json` | `tacsim.pack.resolution` | rules | never |
//! | `atlas/proving_ground.json` | `tacsim.proving_ground` | atlas region | the content latch, at a first boot |
//! | `arenas/proving_ground.json` | `tacsim.pack.proving_ground` | seed | the content latch, after it |
//!
//! A *definition* pack is verified and registered and applies nothing: there is
//! no world state in a weapon. The last two **instantiate** — the region's tiles
//! and the arena's room, domain and eight tanks — so they go through
//! `mush_content`'s latch as boot packs, in that order, exactly as the lot's
//! three starter packs do.
//!
//! `resources.json` is not a pack at all; see [`super::resources`].

use crate::config::PackSource;

/// `crates/tacsim/content/ammunition/he_105.json`.
pub const HE_105_JSON: &str = include_str!("../../content/ammunition/he_105.json");
/// `crates/tacsim/content/ammunition/ball_762.json`.
pub const BALL_762_JSON: &str = include_str!("../../content/ammunition/ball_762.json");
/// `crates/tacsim/content/weapons/cannon_105.json`.
pub const CANNON_105_JSON: &str = include_str!("../../content/weapons/cannon_105.json");
/// `crates/tacsim/content/weapons/mg_coax.json`.
pub const MG_COAX_JSON: &str = include_str!("../../content/weapons/mg_coax.json");
/// `crates/tacsim/content/sensors/visual_mk1.json`.
pub const VISUAL_MK1_JSON: &str = include_str!("../../content/sensors/visual_mk1.json");
/// `crates/tacsim/content/units/line_tank.json`.
pub const LINE_TANK_JSON: &str = include_str!("../../content/units/line_tank.json");
/// `crates/tacsim/content/resolution.json`.
pub const RESOLUTION_JSON: &str = include_str!("../../content/resolution.json");
/// `crates/tacsim/content/arenas/proving_ground.json`.
pub const PROVING_GROUND_JSON: &str = include_str!("../../content/arenas/proving_ground.json");
/// `crates/tacsim/content/atlas/proving_ground.json` — the arena's ground, as an
/// ordinary atlas region pack.
pub const PROVING_GROUND_ATLAS_JSON: &str = include_str!("../../content/atlas/proving_ground.json");
/// `crates/tacsim/content/resources.json`.
pub const RESOURCES_JSON: &str = include_str!("../../content/resources.json");

/// The atlas pack's id — the arena entry's `pack` field names it.
pub const PROVING_GROUND_ATLAS_PACK: &str = "tacsim.proving_ground";

/// The arena seed pack's id.
pub const PROVING_GROUND_PACK: &str = "tacsim.pack.proving_ground";

/// The arena definition the shipped content declares.
pub const PROVING_GROUND: &str = "tacsim.arena.proving_ground";

/// The unit schema the shipped content declares.
pub const LINE_TANK: &str = "tacsim.unit.line_tank";

/// The resolution table the shipped content declares.
pub const RESOLUTION: &str = "tacsim.resolution.stock";

/// Every TacSim pack this crate ships, in `requires` order — a pack after every
/// pack it names.
pub const SOURCES: [(&str, &str); 8] = [
    ("tacsim/content/ammunition/he_105.json", HE_105_JSON),
    ("tacsim/content/ammunition/ball_762.json", BALL_762_JSON),
    ("tacsim/content/weapons/cannon_105.json", CANNON_105_JSON),
    ("tacsim/content/weapons/mg_coax.json", MG_COAX_JSON),
    ("tacsim/content/sensors/visual_mk1.json", VISUAL_MK1_JSON),
    ("tacsim/content/units/line_tank.json", LINE_TANK_JSON),
    ("tacsim/content/resolution.json", RESOLUTION_JSON),
    (
        "tacsim/content/arenas/proving_ground.json",
        PROVING_GROUND_JSON,
    ),
];

/// The shipped packs as sources [`crate::verify_sources`] reads exactly as it
/// reads an operator's.
#[must_use]
pub fn sources() -> Vec<PackSource> {
    SOURCES
        .iter()
        .map(|(name, bytes)| PackSource {
            name: format!("<built-in {name}>"),
            bytes: (*bytes).to_string(),
        })
        .collect()
}

/// The two packs the content latch applies at a first boot, in order: the
/// arena's ground, then the arena.
///
/// # Errors
///
/// A message naming the file when a shipped pack does not verify — a build
/// error, since `crates/tacsim/tests/content.rs` pins both digests.
pub fn boot_packs() -> Result<Vec<mush_content::Pack>, String> {
    [
        (
            "tacsim/content/atlas/proving_ground.json",
            PROVING_GROUND_ATLAS_JSON,
        ),
        (
            "tacsim/content/arenas/proving_ground.json",
            PROVING_GROUND_JSON,
        ),
    ]
    .iter()
    .map(|(name, json)| mush_content::load_pack(json).map_err(|error| format!("{name}: {error}")))
    .collect()
}
