//! The [`AtlasModule`]: the state slot ([`AtlasState`] — limits, the pack shelf and
//! every record), boot hydration through the strict wires, and the persistence
//! helpers every record write goes through.
//!
//! ## The lane
//!
//! Atlas records are bounded whole documents at deterministic keys
//! (`mod:atlas:<kind>:<pack>:<entry>`, `mod:atlas:roomgeo:<obj>`, `mod:atlas:tile:
//! <pack>:<entry>`), written on the compatibility writer lane — the `legacy-post-image`
//! category of `crates/mush-contract-tests/legacy-handlers.toml`, family
//! `atlas.records`. They take part in no authority or prepared mechanic: a room's
//! geometry changes who can *see* a map, never what an action may do. The plan named a
//! `reg.state_family`; that machinery exists for K-owned authoritative families with
//! coordinator hydration, and a tile is not one. When S4's windows put tiles under the
//! K3 bundle limits the records migrate to the family the allowlist entry already
//! names.
//!
//! ## Boot
//!
//! Every document arrives through its wire (`deny_unknown_fields`) and its `validate`,
//! and one that fails is **dropped loudly** with a `Del` fixup so the store matches
//! RAM (`mush-party`'s rule): atlas records are derived from packs, and a pack can be
//! re-applied; refusing to boot for a corrupt tile would be the larger failure. A
//! dropped record is a warning naming the key; the latch that made it is untouched, so
//! `@atlas/pack plan` shows the entry as unchanged and a staffer re-applies the pack
//! after bumping its version to rebuild the record.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_content::{Pack, PackError, load_pack};
use mush_core::module::{
    EventHook, GameModule, HookPoint, ModError, ModuleBootData, ModuleCtx, Registrar,
    WorldEventKind,
};
use mush_core::{
    CanonicalId, CommandCompatibility, Ctx, ModuleOp, ModuleWrite, ObjId, Role, SceneId,
    TickCadence, World,
};
use serde::de::DeserializeOwned;
use smol_str::SmolStr;

use crate::record::{
    CheckpointRec, ConnectionRec, FeatureRec, HookRec, KIND_CHECKPOINT, KIND_CONNECTION,
    KIND_FEATURE, KIND_HOOK, KIND_LOCALITY, KIND_NEIGHBORHOOD, KIND_PORTAL, KIND_REGION,
    KIND_ROOMGEO, KIND_SECRET, KIND_SITE, KIND_STRUCTURE, KIND_TILE, LocalityRec, NeighborhoodRec,
    PortalRec, RecordKey, Records, RegionRec, RoomGeo, SecretRec, SiteRec, StructureRec, TileRec,
    record_key, roomgeo_key,
};

/// The module id and persistence namespace.
pub const NS: &str = "atlas";

/// The attribute namespace this module owns (`atlas:detail:<name>` on rooms and items).
pub const ATTR_NS: &str = "atlas";

/// The most packs `[atlas].pack_paths` may name.
pub const MAX_CONFIGURED_PACKS: usize = 64;

/// One pack file read host-side.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSource {
    /// Where it came from (a path, or `built-in`).
    pub name: String,
    /// The bytes.
    pub bytes: String,
}

/// The `[atlas]` configuration, typed.
#[derive(Clone, Debug)]
pub struct AtlasConfig {
    /// The most cells one tile may hold (plan §7 `tile_max_cells`, default 65 536).
    pub tile_max_cells: u32,
    /// How far, in cells, a point may be from a room and still count as in it (plan §7
    /// `nearest_room_cells`; S4 reads it).
    pub nearest_room_cells: u32,
    /// Operator packs read host-side from `[atlas].pack_paths`, put on the shelf for
    /// `@atlas/pack`.
    pub packs: Vec<PackSource>,
    /// Real seconds a traveler dwells at every checkpoint at least (S2, plan §7
    /// `dwell_floor_s`, default 30), so a stake-out can act.
    pub dwell_floor_s: u32,
    /// Real seconds a whole passage may take at most, per mode (S2, plan §7
    /// `real_cap_s`, default 180 for `foot`, `car`, `transit` and `bus`; a mode
    /// absent here is uncapped).
    pub real_cap_s: BTreeMap<String, u32>,
}

/// The default `real_cap_s` table.
#[must_use]
pub fn default_real_caps() -> BTreeMap<String, u32> {
    crate::travel::CAPPED_MODES
        .iter()
        .map(|m| ((*m).to_string(), crate::travel::DEFAULT_REAL_CAP_S))
        .collect()
}

impl Default for AtlasConfig {
    fn default() -> AtlasConfig {
        AtlasConfig {
            tile_max_cells: crate::cells::MAX_TILE_CELLS,
            nearest_room_cells: 8,
            packs: Vec::new(),
            dwell_floor_s: crate::travel::DEFAULT_DWELL_FLOOR_S,
            real_cap_s: default_real_caps(),
        }
    }
}

/// Parse every configured pack, so a bad file fails the boot naming it.
///
/// # Errors
///
/// A message naming the source and the refusal.
pub fn verify_sources(config: &AtlasConfig) -> Result<Vec<Pack>, String> {
    if config.packs.len() > MAX_CONFIGURED_PACKS {
        return Err(format!(
            "[atlas].pack_paths names {} packs; this server loads at most {MAX_CONFIGURED_PACKS}",
            config.packs.len()
        ));
    }
    if config.tile_max_cells == 0 || config.tile_max_cells > crate::cells::MAX_TILE_CELLS {
        return Err(format!(
            "[atlas].tile_max_cells must be 1 to {}",
            crate::cells::MAX_TILE_CELLS
        ));
    }
    if config.dwell_floor_s > crate::travel::MAX_DWELL_S {
        return Err(format!(
            "[atlas].dwell_floor_s must be at most {}",
            crate::travel::MAX_DWELL_S
        ));
    }
    for (mode, cap) in &config.real_cap_s {
        if mode.is_empty()
            || mode.len() > 32
            || !mode.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(format!("[atlas].real_cap_s: {mode:?} is not a mode word"));
        }
        if *cap == 0 || *cap > crate::travel::MAX_DWELL_S {
            return Err(format!(
                "[atlas].real_cap_s.{mode} must be 1 to {}",
                crate::travel::MAX_DWELL_S
            ));
        }
    }
    let mut packs = Vec::with_capacity(config.packs.len());
    for source in &config.packs {
        let pack = load_pack(&source.bytes).map_err(|e| format!("[atlas] {}: {e}", source.name))?;
        packs.push(pack);
    }
    Ok(packs)
}

/// The limits the rasterizer, the views and the travel system read.
#[derive(Clone, Debug)]
pub struct AtlasLimits {
    /// See [`AtlasConfig::tile_max_cells`].
    pub tile_max_cells: u32,
    /// See [`AtlasConfig::nearest_room_cells`].
    pub nearest_room_cells: u32,
    /// See [`AtlasConfig::dwell_floor_s`].
    pub dwell_floor_s: u32,
    /// See [`AtlasConfig::real_cap_s`].
    pub real_cap_s: BTreeMap<String, u32>,
}

/// The module's state slot.
#[derive(Clone, Debug)]
pub struct AtlasState {
    /// The limits.
    pub limits: AtlasLimits,
    /// The packs `@atlas/pack` can name: the shipped starter packs and the operator's.
    pub shelf: BTreeMap<CanonicalId, Pack>,
    /// Every record.
    pub records: Records,
    /// The last plan each staffer printed, so `apply <fingerprint>` knows the pack.
    pub last_plan: BTreeMap<ObjId, (CanonicalId, String)>,
    /// The travel system (S2): journeys, patches, watches, known places, services.
    pub travel: crate::travel::TravelState,
}

impl Default for AtlasState {
    fn default() -> AtlasState {
        AtlasState {
            limits: AtlasLimits {
                tile_max_cells: crate::cells::MAX_TILE_CELLS,
                nearest_room_cells: 8,
                dwell_floor_s: crate::travel::DEFAULT_DWELL_FLOOR_S,
                real_cap_s: default_real_caps(),
            },
            shelf: BTreeMap::new(),
            records: Records::default(),
            last_plan: BTreeMap::new(),
            travel: crate::travel::TravelState::default(),
        }
    }
}

/// Staff: `SIM_ADMIN` or `Royalty` — the same word `mush-housing` and `mush-party`
/// use, copied rather than depended on.
#[must_use]
pub fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, Role::Royalty)
}

/// The atlas [`GameModule`].
#[derive(Debug)]
pub struct AtlasModule {
    config: AtlasConfig,
    shelf: Vec<Pack>,
}

impl AtlasModule {
    /// A module with default limits and only the shipped packs on its shelf.
    #[must_use]
    pub fn new() -> AtlasModule {
        AtlasModule {
            config: AtlasConfig::default(),
            shelf: Vec::new(),
        }
    }

    /// A module with `[atlas]` applied. The operator's packs must already have passed
    /// [`verify_sources`] host-side; one that does not parse here is logged and left
    /// off the shelf, never a boot failure (the host had its chance to refuse).
    #[must_use]
    pub fn with_config(config: AtlasConfig) -> AtlasModule {
        let mut shelf = Vec::new();
        for source in &config.packs {
            match load_pack(&source.bytes) {
                Ok(pack) => shelf.push(pack),
                Err(error) => {
                    tracing::error!(source = %source.name, %error, "atlas: a configured pack did not load; left off the shelf");
                }
            }
        }
        AtlasModule { config, shelf }
    }

    /// Put packs on the shelf (the composition root adds the starter packs).
    #[must_use]
    pub fn with_shelf(mut self, packs: Vec<Pack>) -> AtlasModule {
        self.shelf.extend(packs);
        self
    }
}

impl Default for AtlasModule {
    fn default() -> AtlasModule {
        AtlasModule::new()
    }
}

impl GameModule for AtlasModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.attr_namespace(ATTR_NS)?;
        let mut state = AtlasState {
            limits: AtlasLimits {
                tile_max_cells: self
                    .config
                    .tile_max_cells
                    .clamp(1, crate::cells::MAX_TILE_CELLS),
                nearest_room_cells: self.config.nearest_room_cells,
                dwell_floor_s: self.config.dwell_floor_s,
                real_cap_s: self.config.real_cap_s.clone(),
            },
            ..AtlasState::default()
        };
        for pack in &self.shelf {
            state
                .shelf
                .insert(pack.manifest().id().clone(), pack.clone());
        }
        reg.state(state);
        // K8.5: the read surface first, then the writer — each group declared.
        reg.classify_commands(CommandCompatibility::ReadOnly);
        crate::commands::register_views(reg)?;
        reg.classify_commands(CommandCompatibility::LegacyPostImage);
        crate::commands::register_pack(reg)?;
        crate::travel_cmd::register_travel(reg)?;
        // S2: the travel system — one hook under three points (the movement gate and
        // the timed-portal departure queue, the transit look, the transit projection),
        // three observers, and one `Coarse` tick that drains the queue, advances every
        // journey, runs the service vehicles and sweeps idle patches.
        let pending: crate::travel::Pending = Arc::new(Mutex::new(Vec::new()));
        let hook: Arc<dyn EventHook> = Arc::new(crate::travel::TravelHook {
            pending: Arc::clone(&pending),
        });
        reg.hook(HookPoint::MoveAttempt, Arc::clone(&hook));
        reg.hook(HookPoint::LookRendered, Arc::clone(&hook));
        reg.hook(HookPoint::Perceive, hook);
        reg.subscribe(WorldEventKind::Moved, Arc::new(crate::travel::on_moved));
        reg.subscribe(
            WorldEventKind::Disconnected,
            Arc::new(crate::travel::on_disconnected),
        );
        reg.subscribe(
            WorldEventKind::ObjDestroyed,
            Arc::new(crate::travel::on_destroyed),
        );
        reg.tick(
            crate::travel::TICK_TAG,
            TickCadence::Coarse(Duration::from_secs(crate::travel::TICK_SECS)),
            Arc::new(
                move |ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>| {
                    crate::travel::tick(ctx, &pending);
                },
            ),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let Loaded { records, dropped } = parse(&data, ctx.w);
        let (travel, dropped_travel) = crate::travel::hydrate(&data.kv, ctx.w, &records);
        {
            let slot = ctx
                .state_mut::<AtlasState>()
                .ok_or_else(|| ModError::msg("atlas: state slot missing"))?;
            slot.records = records;
            slot.travel = travel;
        }
        for key in dropped.into_iter().chain(dropped_travel) {
            del_doc(ctx, key);
        }
        Ok(())
    }
}

struct Loaded {
    records: Records,
    dropped: Vec<SmolStr>,
}

fn decode<T: DeserializeOwned>(
    key: &SmolStr,
    value: &serde_json::Value,
    validate: impl FnOnce(&T) -> Result<(), crate::record::RecordError>,
) -> Result<T, String> {
    let record: T = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    validate(&record).map_err(|e| format!("{key}: {e}"))?;
    Ok(record)
}

fn keyed<T: DeserializeOwned>(
    kind: &str,
    key: &SmolStr,
    value: &serde_json::Value,
    validate: impl FnOnce(&T) -> Result<(), crate::record::RecordError>,
    key_of: impl FnOnce(&T) -> RecordKey,
) -> Result<(RecordKey, T), String> {
    let record: T = decode(key, value, validate)?;
    let record_key_value = key_of(&record);
    if record_key(kind, &record_key_value) != *key {
        return Err(format!("{key}: the document's key disagrees with its id"));
    }
    Ok((record_key_value, record))
}

/// Parse every persisted document into a fresh store, dropping loudly what will not
/// load.
fn parse(data: &ModuleBootData, w: &World) -> Loaded {
    let mut records = Records::default();
    let mut dropped: Vec<SmolStr> = Vec::new();
    for (key, value) in &data.kv {
        if crate::travel::is_travel_key(key) {
            continue;
        }
        let Some((kind, _)) = key.split_once(':') else {
            tracing::warn!(%key, "atlas: unrecognised persisted key; dropping");
            dropped.push(key.clone());
            continue;
        };
        let outcome: Result<(), String> = match kind {
            KIND_REGION => keyed(kind, key, value, RegionRec::validate, |r: &RegionRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.regions.insert(k, r);
            }),
            KIND_LOCALITY => keyed(
                kind,
                key,
                value,
                LocalityRec::validate,
                |r: &LocalityRec| r.key.clone(),
            )
            .map(|(k, r)| {
                records.localities.insert(k, r);
            }),
            KIND_NEIGHBORHOOD => keyed(
                kind,
                key,
                value,
                NeighborhoodRec::validate,
                |r: &NeighborhoodRec| r.key.clone(),
            )
            .map(|(k, r)| {
                records.neighborhoods.insert(k, r);
            }),
            KIND_SITE => keyed(kind, key, value, SiteRec::validate, |r: &SiteRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.sites.insert(k, r);
            }),
            KIND_STRUCTURE => keyed(
                kind,
                key,
                value,
                StructureRec::validate,
                |r: &StructureRec| r.key.clone(),
            )
            .map(|(k, r)| {
                records.structures.insert(k, r);
            }),
            KIND_CHECKPOINT => keyed(
                kind,
                key,
                value,
                CheckpointRec::validate,
                |r: &CheckpointRec| r.key.clone(),
            )
            .map(|(k, r)| {
                records.checkpoints.insert(k, r);
            }),
            KIND_CONNECTION => keyed(
                kind,
                key,
                value,
                ConnectionRec::validate,
                |r: &ConnectionRec| r.key.clone(),
            )
            .map(|(k, r)| {
                records.connections.insert(k, r);
            }),
            KIND_FEATURE => keyed(kind, key, value, FeatureRec::validate, |r: &FeatureRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.features.insert(k, r);
            }),
            KIND_PORTAL => keyed(kind, key, value, PortalRec::validate, |r: &PortalRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.portals.insert(k, r);
            }),
            KIND_TILE => keyed(kind, key, value, TileRec::validate, |r: &TileRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.tiles.insert(k, r);
            }),
            KIND_HOOK => keyed(kind, key, value, HookRec::validate, |r: &HookRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.hooks.insert(k, r);
            }),
            KIND_SECRET => keyed(kind, key, value, SecretRec::validate, |r: &SecretRec| {
                r.key.clone()
            })
            .map(|(k, r)| {
                records.secrets.insert(k, r);
            }),
            KIND_ROOMGEO => decode(key, value, RoomGeo::validate).and_then(|geo: RoomGeo| {
                if roomgeo_key(geo.room) != *key {
                    return Err(format!("{key}: the document's key disagrees with its room"));
                }
                // DL-07 in spirit: geometry for a room that is not a room is dropped —
                // the object was destroyed outside the atlas (an `@destroy` the veto
                // let through, or a swept husk).
                if !w
                    .get(geo.room)
                    .is_some_and(|o| o.kind == mush_core::ObjKind::Room)
                {
                    return Err(format!("{key}: room {} no longer exists", geo.room));
                }
                records.roomgeo.insert(geo.room, geo);
                Ok(())
            }),
            _ => Err(format!("{key}: unknown record kind {kind:?}")),
        };
        if let Err(reason) = outcome {
            tracing::warn!(%key, %reason, "atlas: a record failed its wire; dropping");
            dropped.push(key.clone());
        }
    }
    Loaded { records, dropped }
}

// ---------------------------------------------------------------------------
// Persist helpers
// ---------------------------------------------------------------------------

/// Queue one namespaced write.
pub(crate) fn push_op(ctx: &mut Ctx<'_>, op: ModuleWrite) {
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    });
}

/// Queue a whole-document write of a record; a record that will not serialize is
/// logged and skipped, never a panic (this runs inside a module boot and inside
/// commands on the world task).
pub(crate) fn put_doc<T: serde::Serialize>(ctx: &mut Ctx<'_>, key: SmolStr, record: &T) {
    match serde_json::to_value(record) {
        Ok(value) => push_op(ctx, ModuleWrite::JsonSet { key, value }),
        Err(error) => tracing::error!(%key, %error, "atlas: a record would not serialize"),
    }
}

/// Queue the removal of one document.
pub(crate) fn del_doc(ctx: &mut Ctx<'_>, key: SmolStr) {
    push_op(ctx, ModuleWrite::Del { key });
}

/// The persisted kinds and their key prefixes, for `@atlas/pack show`.
pub const RECORD_KINDS: [&str; 13] = [
    KIND_REGION,
    KIND_LOCALITY,
    KIND_NEIGHBORHOOD,
    KIND_SITE,
    KIND_STRUCTURE,
    KIND_CHECKPOINT,
    KIND_CONNECTION,
    KIND_FEATURE,
    KIND_PORTAL,
    KIND_TILE,
    KIND_HOOK,
    KIND_SECRET,
    KIND_ROOMGEO,
];

/// Load a pack from text for tests and tools, naming the source on failure.
///
/// # Errors
///
/// [`PackError`].
pub fn load_named(name: &str, bytes: &str) -> Result<Pack, PackError> {
    load_pack(bytes).inspect_err(|error| {
        tracing::debug!(source = name, %error, "atlas: pack did not load");
    })
}
