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
    /// The checkpoint the leg ends at.
    pub to: String,
    /// Base seconds.
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
