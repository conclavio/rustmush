//! The starter area as shipped content (doc 14 §2.6; plan D-P-3): three packs compiled
//! in with `include_str!`, applied at a world's first boot when `[game].starter_area`
//! is on, and pinned by digest in `tests/starter.rs` so an unreviewed edit fails CI.
//!
//! | pack | what |
//! |---|---|
//! | `region.limbo` | the lot: the region, its locality, the tour connection and checkpoints, the shared lore |
//! | `neighborhood.ooc_area` | the OOC hall, three tutorial rooms, the lounge, the bus stop, the bus, Sharon and Aaron |
//! | `neighborhood.staff_lobby` | the forecourt, the lobby building, the shop, the backlot, the coffee shop, the writers' room, and the cast |
//!
//! Fourteen lot rooms, the bus interior, and Limbo. No LLM is involved in making any
//! of it (P9): the packs are static bytes, and applying them is a pure function of the
//! bytes and the world.

use mush_content::{Pack, PackError, load_pack};

/// The region pack's id.
pub const REGION_PACK: &str = "region.limbo";

/// The OOC Area pack's id.
pub const OOC_AREA_PACK: &str = "neighborhood.ooc_area";

/// The Staff Lobby pack's id.
pub const STAFF_LOBBY_PACK: &str = "neighborhood.staff_lobby";

/// The region pack, source-controlled and compiled in.
pub const REGION_JSON: &str = include_str!("../content/region.limbo.json");

/// The OOC Area pack, source-controlled and compiled in.
pub const OOC_AREA_JSON: &str = include_str!("../content/neighborhood.ooc_area.json");

/// The Staff Lobby pack, source-controlled and compiled in.
pub const STAFF_LOBBY_JSON: &str = include_str!("../content/neighborhood.staff_lobby.json");

/// The packs in `requires` order — a pack after every pack it names.
pub const SOURCES: [(&str, &str); 3] = [
    (REGION_PACK, REGION_JSON),
    (OOC_AREA_PACK, OOC_AREA_JSON),
    (STAFF_LOBBY_PACK, STAFF_LOBBY_JSON),
];

/// How many rooms the two neighborhoods hold, not counting the bus or Limbo.
pub const LOT_ROOMS: usize = 14;

/// The starter packs, loaded and digest-verified, in apply order.
///
/// # Errors
///
/// [`PackError`] when a shipped pack does not verify — a build error, since the digests
/// are pinned in this crate's tests.
pub fn packs() -> Result<Vec<Pack>, PackError> {
    SOURCES.iter().map(|(_, json)| load_pack(json)).collect()
}
