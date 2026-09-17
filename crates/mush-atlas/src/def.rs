//! What a pack's entries say (doc 14 §2.5): the authored shapes, one per entry kind,
//! decoded strictly (`deny_unknown_fields`) from `RawEntry::def` at plan and apply
//! time. A def is the **source**; the record it becomes is in `record`.
//!
//! References between entries are strings resolved by [`Resolver`]: a bare id names an
//! entry of this pack, or of a pack this one `requires`; `<pack id>.<entry id>` names
//! one explicitly. The word `limbo` names room `#0`, which belongs to no pack.

use std::collections::BTreeMap;

use mush_content::Pack;
use mush_core::{CanonicalId, LIMBO, ObjId};
use serde::Deserialize;

use crate::geom::{Point2, Point3, Polygon2, Rect2};
use crate::record::{
    Access, BackgroundRecipe, ConnectionKind, ConnectionStatus, Direction, FeatureKind, Footprint,
    RecordKey, Records, RegionKind, Representation, RoomKind, Visibility,
};
use crate::terrain::TerrainClass;

/// The reserved endpoint word for Limbo `#0`.
pub const LIMBO_REF: &str = "limbo";

/// A `region` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionDef {
    /// The name.
    pub name: String,
    /// `surface`, `orbital`, `system` or `interstellar`.
    pub kind: String,
    /// The body, for a surface or an orbit.
    #[serde(default)]
    pub body: Option<String>,
    /// The extent of its map, region metres.
    pub extent: Rect2,
    /// The datum, metres (default 0).
    #[serde(default)]
    pub datum_m: f64,
    /// The background recipe.
    pub background: BackgroundRecipe,
    /// The coarse ground resolution, centimetres (default 1000).
    #[serde(default = "default_ground_resolution")]
    pub ground_resolution_cm: u16,
    /// A theme for this place and what lies within it (S2b).
    #[serde(default)]
    pub theme: Option<crate::theme::Theme>,
}

fn default_ground_resolution() -> u16 {
    1000
}

impl RegionDef {
    /// The record's kind, from the authored word and body.
    ///
    /// # Errors
    ///
    /// An unknown word, or a surface or orbit with no body.
    pub fn region_kind(&self) -> Result<RegionKind, String> {
        let body = || {
            self.body
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("a {} region names its body", self.kind))
        };
        match self.kind.trim() {
            "surface" => Ok(RegionKind::Surface { body: body()? }),
            "orbital" => Ok(RegionKind::Orbital { body: body()? }),
            "system" => Ok(RegionKind::System),
            "interstellar" => Ok(RegionKind::Interstellar),
            other => Err(format!(
                "region kind {other:?} is not surface, orbital, system or interstellar"
            )),
        }
    }
}

/// A `locality` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalityDef {
    /// The region.
    pub region: String,
    /// The name.
    pub name: String,
    /// The kind word.
    pub kind: String,
    /// The footprint.
    pub footprint: Polygon2,
    /// A theme for this place and what lies within it (S2b).
    #[serde(default)]
    pub theme: Option<crate::theme::Theme>,
}

/// A `checkpoint` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointDef {
    /// The region.
    pub region: String,
    /// The name.
    pub name: String,
    /// Where it is.
    pub point: Point3,
}

/// One end of a connection, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessDef {
    /// The neighborhood pack it serves.
    pub neighborhood: String,
    /// Where it is.
    pub point: Point3,
}

/// One leg, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegDefDef {
    /// The checkpoint the leg ends at; the last leg names the connection entry itself
    /// (its far end).
    pub to: String,
    /// Base IC seconds.
    pub base_s: u32,
    /// Hazard band.
    #[serde(default)]
    pub hazard: u8,
    /// Interdiction band.
    #[serde(default)]
    pub interdiction: u8,
}

/// A `connection` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDef {
    /// The region.
    pub region: String,
    /// The name.
    pub name: String,
    /// What kind.
    pub kind: ConnectionKind,
    /// From.
    pub from: AccessDef,
    /// To.
    pub to: AccessDef,
    /// Which way.
    #[serde(default = "default_direction")]
    pub direction: Direction,
    /// Travel modes.
    pub modes: Vec<String>,
    /// The corridor.
    pub corridor: Vec<Point3>,
    /// The legs.
    #[serde(default)]
    pub legs: Vec<LegDefDef>,
    /// The ruleset's cost policy.
    #[serde(default)]
    pub cost: serde_json::Value,
    /// Who can see it.
    #[serde(default)]
    pub visibility: Visibility,
    /// Whether it is in service.
    #[serde(default = "default_status")]
    pub status: ConnectionStatus,
    /// The vehicle that runs it on its own clock (S2).
    #[serde(default)]
    pub service: Option<ServiceDefDef>,
}

/// A pack entry in another pack, as authored: `{"pack": "…", "entry": "…"}`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRefDef {
    /// The pack.
    pub pack: String,
    /// The entry.
    pub entry: String,
}

/// A connection's service, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefDef {
    /// The vehicle room.
    pub vehicle: EntryRefDef,
    /// Real seconds at each stop (default 60).
    #[serde(default = "default_dwell")]
    pub dwell_s: u32,
    /// The driver NPC.
    #[serde(default)]
    pub driver: Option<EntryRefDef>,
}

fn default_dwell() -> u32 {
    60
}

fn default_direction() -> Direction {
    Direction::Both
}

fn default_status() -> ConnectionStatus {
    ConnectionStatus::Active
}

/// A room budget, as authored.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetDef {
    /// Target.
    pub target: u32,
    /// Maximum.
    pub maximum: u32,
}

/// A hydration policy, as authored.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HydrationDef {
    /// Enabled.
    pub enabled: bool,
    /// The most player sites.
    pub max_player_sites: u32,
}

/// A `neighborhood` entry — one per neighborhood pack.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodDef {
    /// The region.
    pub region: String,
    /// The locality.
    pub locality: String,
    /// The name.
    pub name: String,
    /// The setting word.
    pub setting: String,
    /// Authored or implicit.
    #[serde(default = "default_representation")]
    pub representation: Representation,
    /// The footprint.
    pub footprint: Polygon2,
    /// The room budget.
    pub room_budget: BudgetDef,
    /// The hydration policy.
    #[serde(default)]
    pub hydration: HydrationDef,
    /// The fine resolution, centimetres (default 100).
    #[serde(default = "default_fine_resolution")]
    pub resolution_cm: u16,
    /// A theme for this place and what lies within it (S2b).
    #[serde(default)]
    pub theme: Option<crate::theme::Theme>,
}

fn default_representation() -> Representation {
    Representation::Authored
}

fn default_fine_resolution() -> u16 {
    100
}

/// A `site` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteDef {
    /// The name.
    pub name: String,
    /// The kind word.
    pub site_kind: String,
    /// The footprint.
    pub footprint: Polygon2,
    /// The exit entries that are its entrances.
    #[serde(default)]
    pub entrances: Vec<String>,
    /// A theme for this place and what lies within it (S2b).
    #[serde(default)]
    pub theme: Option<crate::theme::Theme>,
}

/// A floor, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorDef {
    /// Index.
    pub index: i16,
    /// Height above the structure's ground, metres.
    #[serde(default)]
    pub z_offset_m: f64,
    /// Floor-to-ceiling, metres (default 3).
    #[serde(default = "default_floor_height")]
    pub height_m: f64,
    /// A name.
    #[serde(default)]
    pub name: Option<String>,
}

fn default_floor_height() -> f64 {
    3.0
}

/// A `structure` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDef {
    /// The site.
    pub site: String,
    /// The footprint.
    pub footprint: Polygon2,
    /// The floors.
    pub floors: Vec<FloorDef>,
}

/// A surface, as authored: `"ground"` or `{ "floor": ["structure.lobby", 0] }`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceDef {
    /// The ground.
    Ground,
    /// A structure floor.
    Floor(String, i16),
}

/// A room footprint, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FootprintDef {
    /// A rectangle.
    Rect(Rect2),
    /// A polygon.
    Polygon(Polygon2),
    /// A disc.
    Anchor {
        /// Centre.
        point: Point2,
        /// Radius, metres.
        radius_m: f64,
    },
}

impl FootprintDef {
    /// The record form.
    #[must_use]
    pub fn to_footprint(&self) -> Footprint {
        match self {
            FootprintDef::Rect(r) => Footprint::Rect(*r),
            FootprintDef::Polygon(p) => Footprint::Polygon(p.clone()),
            FootprintDef::Anchor { point, radius_m } => Footprint::Anchor {
                point: *point,
                radius_m: *radius_m,
            },
        }
    }
}

/// A `room` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomDef {
    /// The name.
    pub name: String,
    /// The description.
    #[serde(default)]
    pub desc: String,
    /// The site, if one.
    #[serde(default)]
    pub site: Option<String>,
    /// The surface.
    pub surface: SurfaceDef,
    /// The footprint.
    pub footprint: FootprintDef,
    /// What it is (default: outdoor on the ground, indoor on a floor).
    #[serde(default)]
    pub kind: Option<RoomKind>,
    /// Who may enter.
    #[serde(default)]
    pub access: Access,
    /// Named details (`look <name>` targets), name → text.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
    /// Object flags to set: `outdoors`, `sheltered`, `dark`, `safe`.
    #[serde(default)]
    pub flags: Vec<String>,
}

impl RoomDef {
    /// The room kind, defaulted from the surface.
    #[must_use]
    pub fn room_kind(&self) -> RoomKind {
        self.kind.unwrap_or(match self.surface {
            SurfaceDef::Ground => RoomKind::Outdoor,
            SurfaceDef::Floor(..) => RoomKind::Indoor,
        })
    }
}

/// Where a portal is, as authored: a point or a segment.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum PortalAt {
    /// A point.
    Point(Point3),
    /// A segment.
    Segment(Point3, Point3),
}

/// A portal, as authored on an exit.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortalDef {
    /// How it is passed (default `open`).
    #[serde(default = "default_portal_mode")]
    pub mode: crate::record::PortalMode,
    /// Where it is, when the footprints do not say.
    #[serde(default)]
    pub at: Option<PortalAt>,
    /// The climb, metres.
    #[serde(default)]
    pub dz_m: f64,
    /// The walk, metres: required when the footprints do not touch.
    #[serde(default)]
    pub distance_m: Option<u32>,
    /// Who can see it.
    #[serde(default)]
    pub visibility: Visibility,
    /// The travel modes of a **timed** portal (S2): going through it is a one-leg
    /// journey over `distance_m` at the mode's speed. Empty means instant.
    #[serde(default)]
    pub timed: Vec<String>,
}

fn default_portal_mode() -> crate::record::PortalMode {
    crate::record::PortalMode::Open
}

impl Default for PortalDef {
    fn default() -> PortalDef {
        PortalDef {
            mode: crate::record::PortalMode::Open,
            at: None,
            dz_m: 0.0,
            distance_m: None,
            visibility: Visibility::Public,
            timed: Vec::new(),
        }
    }
}

/// An `exit` entry: one row, one or two exit objects, one portal.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitDef {
    /// From (a room entry, or `limbo`).
    pub from: String,
    /// To (a room entry, or `limbo`).
    pub to: String,
    /// The exit names, `"Name <A>;a;alias"` — one for a one-way exit, two for a pair
    /// (the second is the way back).
    pub names: Vec<String>,
    /// The portal.
    #[serde(default)]
    pub portal: PortalDef,
    /// A lock on the way in: the word `staff`, or a lock expression.
    #[serde(default)]
    pub lock: Option<String>,
    /// A description on the exit objects.
    #[serde(default)]
    pub desc: Option<String>,
}

/// A `feature` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureDef {
    /// What it is.
    pub feature_kind: FeatureKind,
    /// The surface.
    pub surface: SurfaceDef,
    /// A line.
    #[serde(default)]
    pub polyline: Option<Vec<Point2>>,
    /// An area.
    #[serde(default)]
    pub polygon: Option<Polygon2>,
    /// A point.
    #[serde(default)]
    pub point: Option<Point2>,
    /// Above the surface, metres.
    #[serde(default)]
    pub z_range: (f64, f64),
    /// What it blocks.
    #[serde(default)]
    pub blocks: crate::record::Blocks,
    /// How much it raises the ground under it, centimetres.
    #[serde(default)]
    pub raise_cm: i32,
    /// The room it belongs to.
    #[serde(default)]
    pub room: Option<String>,
}

/// An `item` entry: an ordinary object in a room.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDef {
    /// The name.
    pub name: String,
    /// The description.
    #[serde(default)]
    pub desc: String,
    /// The room.
    pub room: String,
    /// Named details.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
    /// Object flags: `enter_ok`, `opaque`, `sticky`, `safe`.
    #[serde(default)]
    pub flags: Vec<String>,
    /// **C0 (doc 16 §3.7)** — the item is an *interior space*: a Thing a
    /// character can be placed inside with `@spatial/place/interior`, which is
    /// what a simulator pod is. The applier attaches `sim.interior` and defines
    /// the unanchored interior-space record; both are idempotent on re-apply.
    #[serde(default)]
    pub interior: bool,
}

/// A `hook` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDef {
    /// The name.
    pub name: String,
    /// The text.
    pub text: String,
    /// The room, if one.
    #[serde(default)]
    pub room: Option<String>,
}

/// A `secret` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretDef {
    /// The truth.
    pub truth: String,
    /// When it may be revealed (default `staff`).
    #[serde(default = "default_reveal")]
    pub reveal: crate::record::Reveal,
    /// What it is about.
    #[serde(default)]
    pub about: Vec<String>,
}

fn default_reveal() -> crate::record::Reveal {
    crate::record::Reveal::Staff
}

// ---------------------------------------------------------------------------
// Reference resolution
// ---------------------------------------------------------------------------

/// What a reference resolved to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resolved {
    /// An entry of a pack.
    Entry(RecordKey),
    /// Limbo.
    Limbo,
}

/// Resolves reference strings for one pack being applied.
pub struct Resolver<'a> {
    pack: &'a Pack,
    placed: &'a BTreeMap<CanonicalId, ObjId>,
    records: &'a Records,
}

impl<'a> Resolver<'a> {
    /// A resolver for `pack`, with what it has placed so far and every record.
    #[must_use]
    pub fn new(
        pack: &'a Pack,
        placed: &'a BTreeMap<CanonicalId, ObjId>,
        records: &'a Records,
    ) -> Resolver<'a> {
        Resolver {
            pack,
            placed,
            records,
        }
    }

    /// The pack.
    #[must_use]
    pub fn pack_id(&self) -> &CanonicalId {
        self.pack.manifest().id()
    }

    /// The key of an entry of this pack.
    #[must_use]
    pub fn own(&self, entry: &CanonicalId) -> RecordKey {
        RecordKey::new(self.pack_id().clone(), entry.clone())
    }

    /// Resolve `reference` to an entry of this pack or of a pack it requires.
    ///
    /// # Errors
    ///
    /// A message naming the reference.
    pub fn resolve(&self, reference: &str) -> Result<Resolved, String> {
        let reference = reference.trim();
        if reference == LIMBO_REF {
            return Ok(Resolved::Limbo);
        }
        // This pack first.
        if let Ok(id) = CanonicalId::new(reference)
            && self.pack.entry(&id).is_some()
        {
            return Ok(Resolved::Entry(self.own(&id)));
        }
        let own = self.pack_id().to_string();
        if let Some(rest) = reference.strip_prefix(&format!("{own}."))
            && let Ok(id) = CanonicalId::new(rest)
            && self.pack.entry(&id).is_some()
        {
            return Ok(Resolved::Entry(self.own(&id)));
        }
        // Then the required packs, in order.
        for requirement in self.pack.manifest().requires() {
            let pack = &requirement.id;
            let prefixed = reference
                .strip_prefix(&format!("{pack}."))
                .and_then(|rest| CanonicalId::new(rest).ok());
            let bare = CanonicalId::new(reference).ok();
            for candidate in prefixed.into_iter().chain(bare) {
                let key = RecordKey::new(pack.clone(), candidate);
                if self.has_record(&key) {
                    return Ok(Resolved::Entry(key));
                }
            }
        }
        Err(format!(
            "{reference:?} names no entry of {own} or of a pack it requires"
        ))
    }

    /// Resolve `reference` to a room object: an entry this pack placed, a room record
    /// of a required pack, or Limbo.
    ///
    /// # Errors
    ///
    /// A message naming the reference.
    pub fn room(&self, reference: &str) -> Result<ObjId, String> {
        match self.resolve(reference)? {
            Resolved::Limbo => Ok(LIMBO),
            Resolved::Entry(key) => {
                if &key.pack == self.pack_id() {
                    return self
                        .placed
                        .get(&key.entry)
                        .copied()
                        .filter(|o| self.records.roomgeo.contains_key(o))
                        .ok_or_else(|| {
                            format!("{reference:?} is not a room this pack has made yet")
                        });
                }
                self.records
                    .roomgeo
                    .values()
                    .find(|g| g.key == key)
                    .map(|g| g.room)
                    .ok_or_else(|| format!("{reference:?} is not a room of an applied pack"))
            }
        }
    }

    /// Resolve `reference` to a record key that must exist as `kind` — this pack's
    /// entries count as existing once placed (or, for object-less kinds, once
    /// recorded).
    ///
    /// # Errors
    ///
    /// A message naming the reference.
    pub fn record(&self, reference: &str, kind: &str) -> Result<RecordKey, String> {
        match self.resolve(reference)? {
            Resolved::Limbo => Err(format!("{reference:?} is Limbo, not a {kind}")),
            Resolved::Entry(key) => {
                let present = match kind {
                    "region" => self.records.regions.contains_key(&key),
                    "locality" => self.records.localities.contains_key(&key),
                    "neighborhood" => self.records.neighborhoods.contains_key(&key),
                    "site" => self.records.sites.contains_key(&key),
                    "structure" => self.records.structures.contains_key(&key),
                    "checkpoint" => self.records.checkpoints.contains_key(&key),
                    "exit" => self.records.portals.contains_key(&key),
                    _ => false,
                };
                if present {
                    Ok(key)
                } else {
                    Err(format!("{reference:?} is not an applied {kind}"))
                }
            }
        }
    }

    fn has_record(&self, key: &RecordKey) -> bool {
        self.records.regions.contains_key(key)
            || self.records.localities.contains_key(key)
            || self.records.neighborhoods.contains_key(key)
            || self.records.sites.contains_key(key)
            || self.records.structures.contains_key(key)
            || self.records.checkpoints.contains_key(key)
            || self.records.connections.contains_key(key)
            || self.records.features.contains_key(key)
            || self.records.portals.contains_key(key)
            || self.records.hooks.contains_key(key)
            || self.records.secrets.contains_key(key)
            || self.records.roomgeo.values().any(|g| &g.key == key)
    }
}

// ---------------------------------------------------------------------------
// S7: heightfield import (doc 14 §3.5)
// ---------------------------------------------------------------------------

/// How a heightfield's samples are written.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeightfieldEncoding {
    /// Whitespace-separated integers, `rows` lines of `cols`, the southern row first.
    #[default]
    Text,
    /// Base64 over `cols × rows` little-endian signed 16-bit samples, the same order.
    Le16,
}

/// The largest sample scale: a hundred metres per unit.
pub const MAX_HEIGHTFIELD_SCALE_CM: i32 = 10_000;

/// **S7 (doc 14 §3.5)** — an imported ground relief: a grid of elevations over a
/// rectangle of a region, rasterized to one ground tile at the entry's own
/// resolution. The tile's class is the region's background at each cell unless
/// the entry authors one in [`classes`](HeightfieldDef::classes); only the
/// elevation and the class are imported. A field wider than a tile is several
/// entries.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeightfieldDef {
    /// The region.
    pub region: String,
    /// The low (south-west) corner, region metres.
    pub origin: Point2,
    /// The sample spacing, one of the tile resolutions.
    pub resolution_cm: u16,
    /// Samples east.
    pub cols: u16,
    /// Samples north.
    pub rows: u16,
    /// How `data` is written.
    #[serde(default)]
    pub encoding: HeightfieldEncoding,
    /// The samples.
    pub data: String,
    /// Centimetres per sample unit.
    #[serde(default = "default_scale_cm")]
    pub scale_cm: i32,
    /// Added to every sample: the field's own datum against the region's, centimetres.
    #[serde(default)]
    pub datum_cm: i32,
    /// **C.1 (16b §8 cut C; doc 16 §3.2 "Terrain")** — the authored per-cell
    /// terrain classes over the same `cols × rows` grid as `data`, run-length
    /// coded exactly the way a tile's cells are on disk
    /// ([`Cells::encode`](crate::cells::Cells::encode)) with a class code as the
    /// only payload: base64 over `[cols u16][rows u16]` and then `[count
    /// u16][class u8]` per run, all little-endian, the southern row first.
    ///
    /// Code `0` ([`TerrainClass::Void`](crate::terrain::TerrainClass::Void)) is
    /// the **not authored** cell: the region's background recipe keeps that one,
    /// so a field that paints a road over open ground is two or three runs a
    /// row and a field that authors nothing is one run. `None` — no `classes`
    /// key at all — leaves the whole rectangle to the recipe, which is what
    /// every field written before this cut does.
    #[serde(default)]
    pub classes: Option<String>,
}

fn default_scale_cm() -> i32 {
    1
}

impl HeightfieldDef {
    /// The shape's own bounds: a tile's side and resolutions, a sane scale.
    ///
    /// # Errors
    ///
    /// A message naming the field.
    pub fn check_shape(&self) -> Result<(), String> {
        let side = crate::cells::MAX_TILE_SIDE;
        if self.cols == 0 || self.rows == 0 || self.cols > side || self.rows > side {
            return Err(format!(
                "a heightfield is 1 to {side} samples a side; {}×{} is not",
                self.cols, self.rows
            ));
        }
        if !crate::record::RESOLUTIONS_CM.contains(&self.resolution_cm) {
            return Err(format!(
                "heightfield resolution_cm {} is not one of {:?}",
                self.resolution_cm,
                crate::record::RESOLUTIONS_CM
            ));
        }
        if self.scale_cm < 1 || self.scale_cm > MAX_HEIGHTFIELD_SCALE_CM {
            return Err(format!(
                "heightfield scale_cm is 1 to {MAX_HEIGHTFIELD_SCALE_CM}"
            ));
        }
        Ok(())
    }

    /// The samples as tile elevations, centimetres against the region datum,
    /// `rows × cols` of them, the southern row first.
    ///
    /// # Errors
    ///
    /// A message naming the sample: an unreadable one, a count that disagrees with
    /// the shape, or a relief past the tile's range.
    pub fn samples(&self) -> Result<Vec<i16>, String> {
        self.check_shape()?;
        let raw: Vec<i32> = match self.encoding {
            HeightfieldEncoding::Text => {
                let mut out = Vec::new();
                for (n, token) in self.data.split_whitespace().enumerate() {
                    let value: i32 = token.parse().map_err(|_| {
                        format!("heightfield sample {n} is not an integer: {token:?}")
                    })?;
                    out.push(value);
                }
                out
            }
            HeightfieldEncoding::Le16 => {
                let bytes = crate::cells::base64_decode(self.data.trim())
                    .ok_or_else(|| "heightfield data is not base64".to_string())?;
                if bytes.len() % 2 != 0 {
                    return Err("heightfield le16 data has an odd byte".to_string());
                }
                let (pairs, _) = bytes.as_chunks::<2>();
                pairs
                    .iter()
                    .map(|pair| i32::from(i16::from_le_bytes(*pair)))
                    .collect()
            }
        };
        let expected = usize::from(self.cols) * usize::from(self.rows);
        if raw.len() != expected {
            return Err(format!(
                "a heightfield of {}×{} has {expected} samples; {} were given",
                self.cols,
                self.rows,
                raw.len()
            ));
        }
        let mut out = Vec::with_capacity(expected);
        for (n, value) in raw.into_iter().enumerate() {
            let cm = value
                .checked_mul(self.scale_cm)
                .and_then(|v| v.checked_add(self.datum_cm))
                .ok_or_else(|| format!("heightfield sample {n} overflows"))?;
            let elev = i16::try_from(cm).map_err(|_| {
                format!(
                    "heightfield sample {n} at {cm} cm is past the tile's range of ±{} cm \
                     about the region datum",
                    i16::MAX
                )
            })?;
            out.push(elev);
        }
        Ok(out)
    }

    /// The authored classes as one entry per cell, `rows × cols` of them, the
    /// southern row first: `Some(class)` where the grid authors one and `None`
    /// where it leaves the cell to the region's background recipe. `Ok(None)`
    /// when the entry authors no grid at all.
    ///
    /// # Errors
    ///
    /// A message naming what is wrong: data that is not base64, a shape that
    /// disagrees with `cols × rows`, an unknown class code, or runs that do not
    /// cover the grid exactly.
    pub fn class_grid(&self) -> Result<Option<Vec<Option<TerrainClass>>>, String> {
        let Some(text) = self.classes.as_deref() else {
            return Ok(None);
        };
        self.check_shape()?;
        let bytes = crate::cells::base64_decode(text.trim())
            .ok_or_else(|| "heightfield classes is not base64".to_string())?;
        let mut cursor = bytes.as_slice();
        let width = take_u16(&mut cursor)?;
        let height = take_u16(&mut cursor)?;
        if width != self.cols || height != self.rows {
            return Err(format!(
                "the classes grid is {width}×{height}; the field's data is {}×{}",
                self.cols, self.rows
            ));
        }
        let expected = usize::from(self.cols) * usize::from(self.rows);
        let mut out: Vec<Option<TerrainClass>> = Vec::with_capacity(expected);
        let mut run = 0_usize;
        while !cursor.is_empty() {
            let count = usize::from(take_u16(&mut cursor)?);
            let code = take_u8(&mut cursor)?;
            let class = TerrainClass::from_code(code).ok_or_else(|| {
                format!("heightfield classes run {run} names unknown terrain class code {code}")
            })?;
            if count == 0 || out.len().saturating_add(count) > expected {
                return Err(format!(
                    "heightfield classes run {run} runs past the {}×{} grid",
                    self.cols, self.rows
                ));
            }
            let cell = (class != TerrainClass::Void).then_some(class);
            out.extend(std::iter::repeat_n(cell, count));
            run = run.saturating_add(1);
        }
        if out.len() != expected {
            return Err(format!(
                "the classes grid covers {} of {expected} cells",
                out.len()
            ));
        }
        Ok(Some(out))
    }
}

fn take_u8(cursor: &mut &[u8]) -> Result<u8, String> {
    let (first, rest) = cursor
        .split_first()
        .ok_or_else(|| "the classes grid is truncated".to_string())?;
    *cursor = rest;
    Ok(*first)
}

fn take_u16(cursor: &mut &[u8]) -> Result<u16, String> {
    let lo = take_u8(cursor)?;
    let hi = take_u8(cursor)?;
    Ok(u16::from_le_bytes([lo, hi]))
}
