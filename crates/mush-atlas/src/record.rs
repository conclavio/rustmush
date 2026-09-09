//! The atlas records (doc 14 §2.1–2.3): what the world remembers about places once a
//! pack has been applied. Every record is a bounded whole document with a strict wire
//! (`deny_unknown_fields`) and a `validate` that boot re-runs, keyed by the pack and
//! entry that made it (`RecordKey`) — so a retire can drop exactly what one pack put
//! there and nothing else.
//!
//! Records are **derived**: a pack is the source, the latch is the ledger, and these
//! are what the source became. They never carry live state (who is in a room, what
//! was said there); that stays on the objects and in the modules that own it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::{CanonicalId, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::cells::Cells;
use crate::geom::{Point2, Point3, Polygon2, Rect2};
use crate::terrain::TerrainClass;

/// The most bytes in a place name.
pub const MAX_NAME_BYTES: usize = 128;
/// The most bytes in a description or a hook's text.
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
/// The most bytes in a secret's truth.
pub const MAX_SECRET_BYTES: usize = 4 * 1024;
/// The most bytes in a taxonomy word (`site.office`, `urban`, `bus`).
pub const MAX_WORD_BYTES: usize = 64;
/// The most tiles a neighborhood may own (doc 14 §2.2).
pub const MAX_TILES_PER_NEIGHBORHOOD: usize = 64;
/// The most entrances a site may declare.
pub const MAX_ENTRANCES: usize = 32;
/// The most legs a connection may have (`crates/mush-sim/src/route.rs`'s `MAX_LEGS`,
/// kept — doc 14 §2.3).
pub const MAX_LEGS: usize = 32;
/// The most corridor points a connection may have.
pub const MAX_CORRIDOR_POINTS: usize = 256;
/// The most travel modes a connection may list.
pub const MAX_MODES: usize = 8;
/// The most floors a structure may have.
pub const MAX_FLOORS: usize = 64;
/// The most rooms one tile may index (a `u16` room slot, minus "none").
pub const MAX_ROOMS_PER_TILE: usize = 4096;
/// The most reviewers a neighborhood may name.
pub const MAX_REVIEWERS: usize = 16;
/// The most places a character may know (doc 14 §2.4's `MAX_KNOWN_PLACES`, declared
/// here for S2's known-places record).
pub const MAX_KNOWN_PLACES: usize = 512;

/// Why a record was refused.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum RecordError {
    /// A name or text is empty or over its byte limit.
    #[error("{field} is empty or longer than {max} bytes")]
    Text {
        /// Which field.
        field: &'static str,
        /// The limit.
        max: usize,
    },
    /// A list is over its limit.
    #[error("{field} holds more than {max} entries")]
    Count {
        /// Which field.
        field: &'static str,
        /// The limit.
        max: usize,
    },
    /// A geometric value was refused.
    #[error("{field}: {source}")]
    Geometry {
        /// Which field.
        field: &'static str,
        /// Why.
        #[source]
        source: crate::geom::GeomError,
    },
    /// A value out of its band.
    #[error("{0}")]
    Value(String),
}

/// Refuse empty text and text over `max` bytes.
///
/// # Errors
///
/// [`RecordError::Text`].
pub fn check_text(field: &'static str, value: &str, max: usize) -> Result<(), RecordError> {
    if value.trim().is_empty() || value.len() > max {
        return Err(RecordError::Text { field, max });
    }
    Ok(())
}

/// Refuse text over `max` bytes (empty is fine).
///
/// # Errors
///
/// [`RecordError::Text`].
pub fn check_optional_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), RecordError> {
    if value.len() > max {
        return Err(RecordError::Text { field, max });
    }
    Ok(())
}

fn check_count(field: &'static str, len: usize, max: usize) -> Result<(), RecordError> {
    if len > max {
        return Err(RecordError::Count { field, max });
    }
    Ok(())
}

/// Refuse anything but a short lowercase word (`[a-z0-9_.]`).
///
/// # Errors
///
/// [`RecordError`].
pub fn check_word(field: &'static str, value: &str) -> Result<(), RecordError> {
    check_text(field, value, MAX_WORD_BYTES)?;
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
    {
        return Err(RecordError::Value(format!(
            "{field} {value:?} is not a lowercase word ([a-z0-9_.])"
        )));
    }
    Ok(())
}

/// Which pack entry a record came from.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordKey {
    /// The pack.
    pub pack: CanonicalId,
    /// The entry within it.
    pub entry: CanonicalId,
}

impl RecordKey {
    /// Build one.
    #[must_use]
    pub fn new(pack: CanonicalId, entry: CanonicalId) -> RecordKey {
        RecordKey { pack, entry }
    }

    /// The document key suffix `<pack>:<entry>` (ids cannot contain `:`).
    #[must_use]
    pub fn suffix(&self) -> String {
        format!("{}:{}", self.pack, self.entry)
    }

    /// Parse a `<pack>:<entry>` suffix.
    #[must_use]
    pub fn parse_suffix(suffix: &str) -> Option<RecordKey> {
        let (pack, entry) = suffix.split_once(':')?;
        Some(RecordKey {
            pack: CanonicalId::new(pack).ok()?,
            entry: CanonicalId::new(entry).ok()?,
        })
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.pack, self.entry)
    }
}

// ---------------------------------------------------------------------------
// 2.1 Frames and geography
// ---------------------------------------------------------------------------

/// What kind of place a region is.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegionKind {
    /// A planetary surface patch with a metric frame.
    Surface {
        /// The body (`earth`, `titan`).
        body: String,
    },
    /// An orbital volume around a body.
    Orbital {
        /// The body.
        body: String,
    },
    /// A star system's transfer space.
    System,
    /// Between systems.
    Interstellar,
}

/// The metric frame of a surface region (doc 14 §1.2 P1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    /// Always metres.
    pub units: String,
    /// The origin, in the parent frame or on the body.
    pub origin: Point3,
    /// Always east-north-up.
    pub axes: String,
    /// Optional real-world reference (a lat/long, a survey name).
    pub geo_ref: Option<String>,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            units: "metres".to_string(),
            origin: Point3(0.0, 0.0, 0.0),
            axes: "enu".to_string(),
            geo_ref: None,
        }
    }
}

/// A rigid placement of a region in its parent (declared; S7 reads it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// Offset, metres.
    pub offset: Point3,
    /// Rotation about up, degrees.
    pub rotation_deg: f64,
}

/// The background recipe (doc 14 §3.2, plan P4): what a region's ground looks like
/// where nothing is drawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundRecipe {
    /// The base ground class.
    pub surface: TerrainClass,
    /// Other classes and their weights out of 100; the base takes the rest.
    #[serde(default)]
    pub mix: Vec<(TerrainClass, u8)>,
    /// Relief amplitude, centimetres above and below the datum.
    #[serde(default)]
    pub relief_cm: u16,
    /// The lattice period of the relief and the mix, centimetres — cells within one
    /// lattice square agree, so a tile's runs stay long.
    #[serde(default = "default_relief_period")]
    pub relief_period_cm: u16,
    /// Water level, centimetres against the datum; a cell below it is water.
    #[serde(default)]
    pub water_cm: Option<i32>,
    /// The seed.
    #[serde(default)]
    pub seed: u64,
}

fn default_relief_period() -> u16 {
    800
}

impl BackgroundRecipe {
    /// Validate the weights and the period.
    ///
    /// # Errors
    ///
    /// [`RecordError::Value`].
    pub fn validate(&self) -> Result<(), RecordError> {
        let total: u32 = self.mix.iter().map(|(_, w)| u32::from(*w)).sum();
        if total > 100 {
            return Err(RecordError::Value(format!(
                "background mix weights sum to {total}, over 100"
            )));
        }
        check_count("background.mix", self.mix.len(), 8)?;
        if self.relief_period_cm == 0 {
            return Err(RecordError::Value(
                "background relief_period_cm must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

/// A region (doc 14 §2.1 `RegionRec`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The name.
    pub name: String,
    /// What it is.
    pub kind: RegionKind,
    /// The metric frame (surface regions).
    pub frame: Option<Frame>,
    /// The parent region and the placement in it.
    pub parent: Option<(RecordKey, Transform)>,
    /// The extent of the region's own map, region metres.
    pub extent: Rect2,
    /// The datum, centimetres.
    #[serde(default)]
    pub datum_cm: i32,
    /// The background recipe.
    pub background: BackgroundRecipe,
    /// The coarse ground tiles.
    #[serde(default)]
    pub ground: Vec<RecordKey>,
    /// The coarse ground resolution, centimetres.
    #[serde(default = "default_ground_resolution")]
    pub ground_resolution_cm: u16,
    /// Revision.
    pub revision: u64,
}

fn default_ground_resolution() -> u16 {
    1000
}

impl RegionRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("region.name", &self.name, MAX_NAME_BYTES)?;
        self.extent
            .validate()
            .map_err(|source| RecordError::Geometry {
                field: "region.extent",
                source,
            })?;
        self.background.validate()?;
        check_count(
            "region.ground",
            self.ground.len(),
            MAX_TILES_PER_NEIGHBORHOOD,
        )?;
        if !RESOLUTIONS_CM.contains(&self.ground_resolution_cm) {
            return Err(RecordError::Value(format!(
                "region ground_resolution_cm {} is not one of {RESOLUTIONS_CM:?}",
                self.ground_resolution_cm
            )));
        }
        Ok(())
    }
}

/// A locality (doc 14 §2.1 `LocalityRec`): a named area of a region a neighborhood
/// belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalityRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The name.
    pub name: String,
    /// The kind word (`lot`, `district`, `town`, `station`, …).
    pub kind: String,
    /// The footprint, region metres.
    pub footprint: Polygon2,
    /// Revision.
    pub revision: u64,
}

impl LocalityRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("locality.name", &self.name, MAX_NAME_BYTES)?;
        check_word("locality.kind", &self.kind)?;
        self.footprint
            .to_cm()
            .map_err(|source| RecordError::Geometry {
                field: "locality.footprint",
                source,
            })?;
        Ok(())
    }
}

/// How a neighborhood is represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    /// Rooms exist for it.
    Authored,
    /// Only the background does; a landing yields an arrival room (S4).
    Implicit,
}

/// A room budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// What the brief asked for.
    pub target: u32,
    /// The most rooms the neighborhood may hold.
    pub maximum: u32,
}

/// The hydration policy (doc 14 §4.5) — declared now, read by S3.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HydrationPolicy {
    /// Whether players may request sites here.
    pub enabled: bool,
    /// The most player-built sites.
    pub max_player_sites: u32,
}

/// A neighborhood (doc 14 §2.1 `NeighborhoodRec`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The locality.
    pub locality: RecordKey,
    /// The name.
    pub name: String,
    /// The setting word (`urban`, `suburban`, `rural`, `station`, …).
    pub setting: String,
    /// Authored or implicit.
    pub representation: Representation,
    /// The footprint, region metres.
    pub footprint: Polygon2,
    /// The room budget.
    pub room_budget: Budget,
    /// The hydration policy.
    pub hydration: HydrationPolicy,
    /// The zone master object it maintains, if any.
    pub zone: Option<ObjId>,
    /// Who reviews hydrated sites here.
    #[serde(default)]
    pub reviewers: Vec<ObjId>,
    /// The fine tiles over the footprint.
    #[serde(default)]
    pub tiles: Vec<RecordKey>,
    /// The fine resolution, centimetres.
    pub resolution_cm: u16,
    /// Revision.
    pub revision: u64,
}

impl NeighborhoodRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("neighborhood.name", &self.name, MAX_NAME_BYTES)?;
        check_word("neighborhood.setting", &self.setting)?;
        self.footprint
            .to_cm()
            .map_err(|source| RecordError::Geometry {
                field: "neighborhood.footprint",
                source,
            })?;
        if self.room_budget.maximum < self.room_budget.target {
            return Err(RecordError::Value(
                "neighborhood room_budget.maximum is below its target".to_string(),
            ));
        }
        check_count(
            "neighborhood.reviewers",
            self.reviewers.len(),
            MAX_REVIEWERS,
        )?;
        check_count(
            "neighborhood.tiles",
            self.tiles.len(),
            MAX_TILES_PER_NEIGHBORHOOD,
        )?;
        if !RESOLUTIONS_CM.contains(&self.resolution_cm) {
            return Err(RecordError::Value(format!(
                "neighborhood resolution_cm {} is not one of {RESOLUTIONS_CM:?}",
                self.resolution_cm
            )));
        }
        Ok(())
    }
}

/// The tile resolutions the atlas rasterizes at (doc 14 §2.2).
pub const RESOLUTIONS_CM: [u16; 6] = [50, 100, 200, 500, 1000, 2500];

/// Where a site came from.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case", deny_unknown_fields)]
pub enum SiteOrigin {
    /// A staff-authored pack.
    Authored,
    /// A player's hydration request (S3).
    Hydrated {
        /// The request.
        request: String,
    },
}

/// A site's review state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Not subject to review.
    NotApplicable,
    /// Waiting.
    PendingReview,
    /// Reviewed and kept.
    Approved,
    /// Sent back.
    NeedsChanges,
}

/// A site (doc 14 §2.1 `SiteRec`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The neighborhoods it lies in.
    pub neighborhoods: Vec<RecordKey>,
    /// The name.
    pub name: String,
    /// The kind word (`site.office`, `site.shop`, …).
    pub kind: String,
    /// The footprint, region metres.
    pub footprint: Polygon2,
    /// Its entrances (portal records).
    #[serde(default)]
    pub entrances: Vec<RecordKey>,
    /// A titled property, if the housing layer owns it (S2b).
    #[serde(default)]
    pub property: Option<u64>,
    /// Where it came from.
    pub origin: SiteOrigin,
    /// Review state.
    pub review: ReviewState,
    /// Whether every room has a footprint on a tile and every portal has geometry.
    pub tactical_ready: bool,
    /// Revision.
    pub revision: u64,
}

impl SiteRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("site.name", &self.name, MAX_NAME_BYTES)?;
        check_word("site.kind", &self.kind)?;
        if self.neighborhoods.is_empty() {
            return Err(RecordError::Value(
                "a site lies in at least one neighborhood".to_string(),
            ));
        }
        check_count("site.neighborhoods", self.neighborhoods.len(), 4)?;
        self.footprint
            .to_cm()
            .map_err(|source| RecordError::Geometry {
                field: "site.footprint",
                source,
            })?;
        check_count("site.entrances", self.entrances.len(), MAX_ENTRANCES)?;
        Ok(())
    }
}

/// One floor of a structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Floor {
    /// `0` is the structure's ground level; negative is below.
    pub index: i16,
    /// Height of the floor surface above the structure's ground, metres.
    pub z_offset_m: f64,
    /// Floor-to-ceiling, metres.
    pub height_m: f64,
    /// A name, if the floor has one.
    #[serde(default)]
    pub name: Option<String>,
    /// The floor's tile, once rasterized.
    #[serde(default)]
    pub tile: Option<RecordKey>,
}

/// A structure (doc 14 §2.1 `StructureRec`): a building with floors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The site it stands on.
    pub site: RecordKey,
    /// The footprint, region metres.
    pub footprint: Polygon2,
    /// Its floors, index order.
    pub floors: Vec<Floor>,
    /// Revision.
    pub revision: u64,
}

impl StructureRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        self.footprint
            .to_cm()
            .map_err(|source| RecordError::Geometry {
                field: "structure.footprint",
                source,
            })?;
        if self.floors.is_empty() {
            return Err(RecordError::Value(
                "a structure has at least one floor".to_string(),
            ));
        }
        check_count("structure.floors", self.floors.len(), MAX_FLOORS)?;
        let mut seen = BTreeSet::new();
        for floor in &self.floors {
            if !seen.insert(floor.index) {
                return Err(RecordError::Value(format!(
                    "structure floor {} is listed twice",
                    floor.index
                )));
            }
            if !floor.z_offset_m.is_finite()
                || !floor.height_m.is_finite()
                || floor.height_m <= 0.0
                || floor.z_offset_m.abs() > 10_000.0
            {
                return Err(RecordError::Value(format!(
                    "structure floor {} has an impossible height",
                    floor.index
                )));
            }
            if let Some(name) = &floor.name {
                check_text("structure.floor.name", name, MAX_NAME_BYTES)?;
            }
        }
        Ok(())
    }

    /// The floor at `index`.
    #[must_use]
    pub fn floor(&self, index: i16) -> Option<&Floor> {
        self.floors.iter().find(|f| f.index == index)
    }
}

// ---------------------------------------------------------------------------
// 2.2 Surfaces, tiles, features
// ---------------------------------------------------------------------------

/// Which surface a thing is on (doc 14 §2.2 `SurfaceRef`).
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "surface", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceRef {
    /// The region's ground.
    Ground,
    /// A floor of a structure.
    Floor {
        /// The structure.
        structure: RecordKey,
        /// The floor index.
        index: i16,
    },
}

impl fmt::Display for SurfaceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurfaceRef::Ground => f.write_str("ground"),
            SurfaceRef::Floor { structure, index } => write!(f, "{structure} floor {index}"),
        }
    }
}

/// Who owns a tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileOwner {
    /// The region's coarse ground.
    Region,
    /// A neighborhood's fine ground.
    Neighborhood,
    /// A structure floor.
    Structure,
    /// A site addition pack (S3).
    Addition,
}

/// A tile (doc 14 §2.2 `TileRec`): a bounded grid of cells over one surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileRec {
    /// Which entry made it (the neighborhood, structure or region entry) — the tile
    /// id is `<that entry>.tile.<n>`.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The surface.
    pub surface: SurfaceRef,
    /// Who owns it.
    pub owner: TileOwner,
    /// The bounding box, region metres.
    pub bbox: Rect2,
    /// Cell size, centimetres.
    pub resolution_cm: u16,
    /// The cells.
    pub cells: Cells,
    /// The room table `cell.room - 1` indexes.
    #[serde(default)]
    pub rooms: Vec<ObjId>,
    /// Revision; a new revision is a new document.
    pub revision: u64,
}

impl TileRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        self.bbox
            .validate()
            .map_err(|source| RecordError::Geometry {
                field: "tile.bbox",
                source,
            })?;
        if !RESOLUTIONS_CM.contains(&self.resolution_cm) {
            return Err(RecordError::Value(format!(
                "tile resolution_cm {} is not one of {RESOLUTIONS_CM:?}",
                self.resolution_cm
            )));
        }
        check_count("tile.rooms", self.rooms.len(), MAX_ROOMS_PER_TILE)?;
        let rooms = self.rooms.len();
        if self
            .cells
            .iter()
            .any(|(_, _, c)| usize::from(c.room) > rooms)
        {
            return Err(RecordError::Value(
                "tile cells index a room slot the tile does not list".to_string(),
            ));
        }
        Ok(())
    }

    /// The room a cell slot names.
    #[must_use]
    pub fn room_of_slot(&self, slot: u16) -> Option<ObjId> {
        let index = usize::from(slot).checked_sub(1)?;
        self.rooms.get(index).copied()
    }
}

/// What a feature is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    /// A wall.
    Wall,
    /// A door.
    Door,
    /// A window.
    Window,
    /// A gate.
    Gate,
    /// A counter or bar.
    Counter,
    /// A tree.
    Tree,
    /// A rail or fence.
    Rail,
    /// A crater or hole.
    Crater,
    /// A raised platform or dock.
    Platform,
    /// Something else the ruleset names.
    Other,
}

impl FeatureKind {
    /// The snake-case name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FeatureKind::Wall => "wall",
            FeatureKind::Door => "door",
            FeatureKind::Window => "window",
            FeatureKind::Gate => "gate",
            FeatureKind::Counter => "counter",
            FeatureKind::Tree => "tree",
            FeatureKind::Rail => "rail",
            FeatureKind::Crater => "crater",
            FeatureKind::Platform => "platform",
            FeatureKind::Other => "other",
        }
    }
}

/// A feature's geometry (doc 14 §2.2 `Geometry3`, plan view plus a z range).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    /// A line.
    Polyline(Vec<Point2>),
    /// An area.
    Polygon(Polygon2),
    /// A point.
    Point(Point2),
}

/// What a feature blocks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Blocks {
    /// Movement.
    pub movement: bool,
    /// Sight.
    pub sight: bool,
}

/// A feature's state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FeatureState {
    /// Whole.
    Intact,
    /// Damaged, 1–255.
    Damaged(u8),
    /// Gone.
    Destroyed,
}

/// A feature (doc 14 §2.2 `FeatureRec`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRec {
    /// Which entry made it (an authored feature) or derived it (`<structure>.wall.<n>`).
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The surface.
    pub surface: SurfaceRef,
    /// What it is.
    pub kind: FeatureKind,
    /// Where it is.
    pub geometry: Geometry,
    /// Above the surface, metres.
    pub z_range: (f64, f64),
    /// What it blocks.
    pub blocks: Blocks,
    /// Its state.
    pub state: FeatureState,
    /// How much it raises the ground under it, centimetres (platforms, docks).
    #[serde(default)]
    pub raise_cm: i32,
    /// The room it belongs to, if one.
    #[serde(default)]
    pub room: Option<ObjId>,
    /// Revision.
    pub revision: u64,
}

impl FeatureRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        match &self.geometry {
            Geometry::Polyline(points) => {
                if points.len() < 2 || points.len() > crate::geom::MAX_POLYGON_POINTS {
                    return Err(RecordError::Value(format!(
                        "feature polyline has {} points",
                        points.len()
                    )));
                }
                for p in points {
                    p.to_cm().map_err(|source| RecordError::Geometry {
                        field: "feature.polyline",
                        source,
                    })?;
                }
            }
            Geometry::Polygon(poly) => {
                poly.to_cm().map_err(|source| RecordError::Geometry {
                    field: "feature.polygon",
                    source,
                })?;
            }
            Geometry::Point(p) => {
                p.to_cm().map_err(|source| RecordError::Geometry {
                    field: "feature.point",
                    source,
                })?;
            }
        }
        if !self.z_range.0.is_finite()
            || !self.z_range.1.is_finite()
            || self.z_range.1 < self.z_range.0
        {
            return Err(RecordError::Value(
                "feature z_range is not a range".to_string(),
            ));
        }
        if self.raise_cm.abs() > 100_000 {
            return Err(RecordError::Value(
                "feature raise_cm is past a kilometre".to_string(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 2.3 Rooms, portals, connections
// ---------------------------------------------------------------------------

/// A room's footprint (doc 14 §2.3 `Footprint`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Footprint {
    /// A rectangle.
    Rect(Rect2),
    /// A polygon.
    Polygon(Polygon2),
    /// A disc around a point (an implicit landing, a clearing).
    Anchor {
        /// The centre.
        point: Point2,
        /// The radius, metres.
        radius_m: f64,
    },
}

impl Footprint {
    /// The validated shape.
    ///
    /// # Errors
    ///
    /// [`crate::geom::GeomError`].
    pub fn shape(&self) -> Result<crate::geom::Shape, crate::geom::GeomError> {
        Ok(match self {
            Footprint::Rect(rect) => crate::geom::Shape::Polygon(rect.to_polygon().to_cm()?),
            Footprint::Polygon(poly) => crate::geom::Shape::Polygon(poly.to_cm()?),
            Footprint::Anchor { point, radius_m } => {
                crate::geom::Shape::Disc(crate::geom::DiscCm::new(*point, *radius_m)?)
            }
        })
    }
}

/// What a room is, spatially.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomKind {
    /// Under the sky.
    Outdoor,
    /// Under a roof.
    Indoor,
    /// A corridor, stair or street: passage, not a place.
    Circulation,
}

/// Who may enter, for the map (locks decide; this decides what the map draws).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Anyone.
    #[default]
    Public,
    /// Staff, or whoever the lock admits.
    Restricted,
}

/// A room's geometry (doc 14 §2.3 `RoomGeo`), keyed by the room object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomGeo {
    /// The room.
    pub room: ObjId,
    /// Which entry made it.
    pub key: RecordKey,
    /// The neighborhood.
    pub neighborhood: RecordKey,
    /// The site, if one.
    pub site: Option<RecordKey>,
    /// The surface.
    pub surface: SurfaceRef,
    /// The footprint.
    pub footprint: Footprint,
    /// What it is.
    pub kind: RoomKind,
    /// Who may enter.
    #[serde(default)]
    pub access: Access,
    /// Revision.
    pub revision: u64,
}

impl RoomGeo {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        self.footprint
            .shape()
            .map_err(|source| RecordError::Geometry {
                field: "room.footprint",
                source,
            })?;
        Ok(())
    }
}

/// One end of a portal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Endpoint {
    /// A room.
    Room(ObjId),
    /// A point outside any room.
    Outside(Point3),
}

/// A portal's geometry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PortalGeometry {
    /// A threshold segment.
    Segment(Point3, Point3),
    /// A point.
    Point(Point3),
}

/// How a portal is passed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalMode {
    /// A door.
    Door,
    /// A gate.
    Gate,
    /// A stair.
    Stair,
    /// A ramp.
    Ramp,
    /// A ladder.
    Ladder,
    /// A hatch.
    Hatch,
    /// An open threshold.
    Open,
}

impl PortalMode {
    /// The snake-case name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PortalMode::Door => "door",
            PortalMode::Gate => "gate",
            PortalMode::Stair => "stair",
            PortalMode::Ramp => "ramp",
            PortalMode::Ladder => "ladder",
            PortalMode::Hatch => "hatch",
            PortalMode::Open => "open",
        }
    }
}

/// How long a portal takes (doc 14 §3.3; S2 reads it).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "time", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimePolicy {
    /// Instant.
    Instant,
    /// A journey, by the modes named.
    Timed {
        /// Travel modes.
        modes: Vec<String>,
    },
}

/// Who can see a portal or connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// On the map.
    #[default]
    Public,
    /// Known only to those who found it.
    Hidden,
}

/// A portal (doc 14 §2.3 `PortalRec`): what an exit is, on the map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortalRec {
    /// Which entry made it (the exit entry).
    pub key: RecordKey,
    /// The exit objects (one per direction).
    pub exits: Vec<ObjId>,
    /// From.
    pub from: Endpoint,
    /// To.
    pub to: Endpoint,
    /// Where it is.
    pub geometry: PortalGeometry,
    /// How it is passed.
    pub mode: PortalMode,
    /// The walk, metres.
    pub distance_m: u32,
    /// The climb, metres.
    pub dz_m: f64,
    /// How long it takes.
    pub time: TimePolicy,
    /// Who can see it.
    pub visibility: Visibility,
    /// Revision.
    pub revision: u64,
}

impl PortalRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_count("portal.exits", self.exits.len(), 2)?;
        match self.geometry {
            PortalGeometry::Segment(a, b) => {
                a.validate().map_err(|source| RecordError::Geometry {
                    field: "portal.geometry",
                    source,
                })?;
                b.validate().map_err(|source| RecordError::Geometry {
                    field: "portal.geometry",
                    source,
                })?;
            }
            PortalGeometry::Point(p) => p.validate().map_err(|source| RecordError::Geometry {
                field: "portal.geometry",
                source,
            })?,
        }
        if !self.dz_m.is_finite() || self.dz_m.abs() > 10_000.0 {
            return Err(RecordError::Value("portal dz_m is impossible".to_string()));
        }
        if let TimePolicy::Timed { modes } = &self.time {
            check_count("portal.time.modes", modes.len(), MAX_MODES)?;
            for mode in modes {
                check_word("portal.time.modes", mode)?;
            }
        }
        Ok(())
    }

    /// The plan-view point of the portal.
    #[must_use]
    pub fn point(&self) -> Point2 {
        match self.geometry {
            PortalGeometry::Segment(a, b) => Point2((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
            PortalGeometry::Point(p) => p.xy(),
        }
    }
}

/// A checkpoint on a corridor (doc 14 §2.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The name.
    pub name: String,
    /// Where it is.
    pub point: Point3,
    /// Revision.
    pub revision: u64,
}

impl CheckpointRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("checkpoint.name", &self.name, MAX_NAME_BYTES)?;
        self.point
            .validate()
            .map_err(|source| RecordError::Geometry {
                field: "checkpoint.point",
                source,
            })
    }
}

/// An access point: where a connection meets a neighborhood (doc 14 §2.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessPoint {
    /// The neighborhood **pack** it serves — a forward reference, resolved when the
    /// neighborhood is applied and travel is built (S2).
    pub neighborhood: CanonicalId,
    /// The room, once the neighborhood exists and names one.
    #[serde(default)]
    pub room: Option<ObjId>,
    /// Where it is.
    pub point: Point3,
}

/// One leg of a corridor (doc 14 §2.3 `LegDef`, kept from `route.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegDef {
    /// The checkpoint the leg ends at; the **last** leg names the connection itself,
    /// meaning its far end.
    pub to: RecordKey,
    /// The base time, seconds.
    pub base_s: u32,
    /// Hazard band.
    pub hazard: u8,
    /// Interdiction band.
    pub interdiction: u8,
}

/// What kind of connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// Two neighborhoods that touch: a street continues.
    Boundary,
    /// A journey with legs.
    Journey,
}

/// Which way a connection runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Both ways.
    Both,
    /// From the first endpoint to the second only.
    OneWay,
}

/// Whether a connection is in service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// In service.
    Active,
    /// Not in service.
    Inactive,
}

/// A vehicle that runs a connection on its own clock (S2): the tour bus. The
/// references are pack entries resolved through the content latch at run time, since
/// the vehicle's pack may apply after the region's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRec {
    /// The vehicle room.
    pub vehicle: RecordKey,
    /// Real seconds it stands at each stop before setting out again.
    pub dwell_s: u32,
    /// The driver NPC, for its announcements.
    #[serde(default)]
    pub driver: Option<RecordKey>,
}

/// A connection (doc 14 §2.3 `ConnectionRec`): a corridor between two neighborhoods,
/// its checkpoints as legs, and (S2) the vehicle that may serve it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The name.
    pub name: String,
    /// What kind.
    pub kind: ConnectionKind,
    /// Its two ends.
    pub endpoints: (AccessPoint, AccessPoint),
    /// Which way it runs.
    pub direction: Direction,
    /// Travel modes.
    pub modes: Vec<String>,
    /// The corridor, region metres.
    pub corridor: Vec<Point3>,
    /// The legs.
    #[serde(default)]
    pub legs: Vec<LegDef>,
    /// The ruleset's cost policy, opaque here.
    #[serde(default)]
    pub cost: serde_json::Value,
    /// Who can see it.
    pub visibility: Visibility,
    /// Whether it is in service.
    pub status: ConnectionStatus,
    /// The vehicle that runs it, if one does (S2).
    #[serde(default)]
    pub service: Option<ServiceRec>,
    /// Revision.
    pub revision: u64,
}

impl ConnectionRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("connection.name", &self.name, MAX_NAME_BYTES)?;
        if self.modes.is_empty() {
            return Err(RecordError::Value(
                "a connection names at least one travel mode".to_string(),
            ));
        }
        check_count("connection.modes", self.modes.len(), MAX_MODES)?;
        for mode in &self.modes {
            check_word("connection.modes", mode)?;
        }
        if self.corridor.len() < 2 {
            return Err(RecordError::Value(
                "a corridor has at least two points".to_string(),
            ));
        }
        check_count(
            "connection.corridor",
            self.corridor.len(),
            MAX_CORRIDOR_POINTS,
        )?;
        for p in &self.corridor {
            p.validate().map_err(|source| RecordError::Geometry {
                field: "connection.corridor",
                source,
            })?;
        }
        check_count("connection.legs", self.legs.len(), MAX_LEGS)?;
        if let Some(last) = self.legs.last() {
            if last.to != self.key {
                return Err(RecordError::Value(
                    "a connection's last leg names the connection itself (its far end)".to_string(),
                ));
            }
            if self
                .legs
                .iter()
                .take(self.legs.len() - 1)
                .any(|l| l.to == self.key)
            {
                return Err(RecordError::Value(
                    "only a connection's last leg names the connection itself".to_string(),
                ));
            }
        }
        if let Some(service) = &self.service {
            if service.dwell_s == 0 || service.dwell_s > crate::travel::MAX_DWELL_S {
                return Err(RecordError::Value(format!(
                    "a service dwells 1 to {} seconds",
                    crate::travel::MAX_DWELL_S
                )));
            }
            if self.direction != Direction::Both {
                return Err(RecordError::Value(
                    "a service runs a connection that goes both ways".to_string(),
                ));
            }
        }
        for ap in [&self.endpoints.0, &self.endpoints.1] {
            ap.point
                .validate()
                .map_err(|source| RecordError::Geometry {
                    field: "connection.endpoints",
                    source,
                })?;
        }
        if serde_json::to_vec(&self.cost).map_or(true, |b| b.len() > 4096) {
            return Err(RecordError::Value(
                "connection cost is over 4 KiB".to_string(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Content that is not geometry: hooks and secrets (doc 14 §2.6)
// ---------------------------------------------------------------------------

/// A hook: an invitation, a place strangers meet — prose with a room, no mechanic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The name.
    pub name: String,
    /// The text.
    pub text: String,
    /// The room, if one.
    #[serde(default)]
    pub room: Option<ObjId>,
    /// Revision.
    pub revision: u64,
}

impl HookRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("hook.name", &self.name, MAX_NAME_BYTES)?;
        check_text("hook.text", &self.text, MAX_TEXT_BYTES)
    }
}

/// When a secret may be revealed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reveal {
    /// Staff may read it and reveal it by hand.
    Staff,
    /// Nobody, ever, through the game.
    Never,
}

/// A secret (doc 14 §2.6): a truth that is never in any prompt and never on any map;
/// `@atlas/pack show` prints it to staff and nothing else does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRec {
    /// Which entry made it.
    pub key: RecordKey,
    /// The truth.
    pub truth: String,
    /// When it may be revealed.
    pub reveal: Reveal,
    /// What it is about (pack entry ids).
    #[serde(default)]
    pub about: Vec<CanonicalId>,
    /// Revision.
    pub revision: u64,
}

impl SecretRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// [`RecordError`].
    pub fn validate(&self) -> Result<(), RecordError> {
        check_text("secret.truth", &self.truth, MAX_SECRET_BYTES)?;
        check_count("secret.about", self.about.len(), 16)
    }
}

// ---------------------------------------------------------------------------
// The record store
// ---------------------------------------------------------------------------

/// Every atlas record in RAM, mirrored by the `mod:atlas:*` documents.
#[derive(Clone, Debug, Default)]
pub struct Records {
    /// Regions.
    pub regions: BTreeMap<RecordKey, RegionRec>,
    /// Localities.
    pub localities: BTreeMap<RecordKey, LocalityRec>,
    /// Neighborhoods.
    pub neighborhoods: BTreeMap<RecordKey, NeighborhoodRec>,
    /// Sites.
    pub sites: BTreeMap<RecordKey, SiteRec>,
    /// Structures.
    pub structures: BTreeMap<RecordKey, StructureRec>,
    /// Checkpoints.
    pub checkpoints: BTreeMap<RecordKey, CheckpointRec>,
    /// Connections.
    pub connections: BTreeMap<RecordKey, ConnectionRec>,
    /// Features.
    pub features: BTreeMap<RecordKey, FeatureRec>,
    /// Portals.
    pub portals: BTreeMap<RecordKey, PortalRec>,
    /// Tiles.
    pub tiles: BTreeMap<RecordKey, TileRec>,
    /// Hooks.
    pub hooks: BTreeMap<RecordKey, HookRec>,
    /// Secrets.
    pub secrets: BTreeMap<RecordKey, SecretRec>,
    /// Room geometry, by room.
    pub roomgeo: BTreeMap<ObjId, RoomGeo>,
}

/// The document kinds, as key prefixes.
pub const KIND_REGION: &str = "region";
/// See [`KIND_REGION`].
pub const KIND_LOCALITY: &str = "locality";
/// See [`KIND_REGION`].
pub const KIND_NEIGHBORHOOD: &str = "neighborhood";
/// See [`KIND_REGION`].
pub const KIND_SITE: &str = "site";
/// See [`KIND_REGION`].
pub const KIND_STRUCTURE: &str = "structure";
/// See [`KIND_REGION`].
pub const KIND_CHECKPOINT: &str = "checkpoint";
/// See [`KIND_REGION`].
pub const KIND_CONNECTION: &str = "connection";
/// See [`KIND_REGION`].
pub const KIND_FEATURE: &str = "feature";
/// See [`KIND_REGION`].
pub const KIND_PORTAL: &str = "portal";
/// See [`KIND_REGION`].
pub const KIND_TILE: &str = "tile";
/// See [`KIND_REGION`].
pub const KIND_HOOK: &str = "hook";
/// See [`KIND_REGION`].
pub const KIND_SECRET: &str = "secret";
/// See [`KIND_REGION`].
pub const KIND_ROOMGEO: &str = "roomgeo";

/// The document key of a keyed record: `<kind>:<pack>:<entry>`.
#[must_use]
pub fn record_key(kind: &str, key: &RecordKey) -> SmolStr {
    SmolStr::new(format!("{kind}:{}", key.suffix()))
}

/// The document key of a room's geometry: `roomgeo:<obj>`.
#[must_use]
pub fn roomgeo_key(room: ObjId) -> SmolStr {
    SmolStr::new(format!("{KIND_ROOMGEO}:{}", room.0))
}

impl Records {
    /// The room geometry of every room in `neighborhood`, in room order.
    pub fn rooms_in(&self, neighborhood: &RecordKey) -> impl Iterator<Item = &RoomGeo> {
        self.roomgeo
            .values()
            .filter(move |g| &g.neighborhood == neighborhood)
    }

    /// Every record made by `pack`, as `(kind, key)` document keys.
    #[must_use]
    pub fn keys_of_pack(&self, pack: &CanonicalId) -> Vec<SmolStr> {
        let mut keys: Vec<SmolStr> = Vec::new();
        let mut push = |kind: &str, key: &RecordKey| {
            if &key.pack == pack {
                keys.push(record_key(kind, key));
            }
        };
        self.regions.keys().for_each(|k| push(KIND_REGION, k));
        self.localities.keys().for_each(|k| push(KIND_LOCALITY, k));
        self.neighborhoods
            .keys()
            .for_each(|k| push(KIND_NEIGHBORHOOD, k));
        self.sites.keys().for_each(|k| push(KIND_SITE, k));
        self.structures.keys().for_each(|k| push(KIND_STRUCTURE, k));
        self.checkpoints
            .keys()
            .for_each(|k| push(KIND_CHECKPOINT, k));
        self.connections
            .keys()
            .for_each(|k| push(KIND_CONNECTION, k));
        self.features.keys().for_each(|k| push(KIND_FEATURE, k));
        self.portals.keys().for_each(|k| push(KIND_PORTAL, k));
        self.tiles.keys().for_each(|k| push(KIND_TILE, k));
        self.hooks.keys().for_each(|k| push(KIND_HOOK, k));
        self.secrets.keys().for_each(|k| push(KIND_SECRET, k));
        for geo in self.roomgeo.values() {
            if &geo.key.pack == pack {
                keys.push(roomgeo_key(geo.room));
            }
        }
        keys
    }

    /// The neighborhood a room is in.
    #[must_use]
    pub fn neighborhood_of_room(&self, room: ObjId) -> Option<&NeighborhoodRec> {
        let geo = self.roomgeo.get(&room)?;
        self.neighborhoods.get(&geo.neighborhood)
    }

    /// The region a neighborhood is in.
    #[must_use]
    pub fn region_of(&self, neighborhood: &NeighborhoodRec) -> Option<&RegionRec> {
        self.regions.get(&neighborhood.region)
    }

    /// The portals touching `room`.
    pub fn portals_of_room(&self, room: ObjId) -> impl Iterator<Item = &PortalRec> {
        self.portals.values().filter(move |p| {
            matches!(p.from, Endpoint::Room(r) if r == room)
                || matches!(p.to, Endpoint::Room(r) if r == room)
        })
    }

    /// The structures standing on `site`.
    pub fn structures_of_site(&self, site: &RecordKey) -> impl Iterator<Item = &StructureRec> {
        self.structures.values().filter(move |s| &s.site == site)
    }

    /// The sites in `neighborhood`.
    pub fn sites_in(&self, neighborhood: &RecordKey) -> impl Iterator<Item = &SiteRec> {
        self.sites
            .values()
            .filter(move |s| s.neighborhoods.contains(neighborhood))
    }

    /// The neighborhoods of `region`.
    pub fn neighborhoods_in(&self, region: &RecordKey) -> impl Iterator<Item = &NeighborhoodRec> {
        self.neighborhoods
            .values()
            .filter(move |n| &n.region == region)
    }

    /// The connections of `region`.
    pub fn connections_in(&self, region: &RecordKey) -> impl Iterator<Item = &ConnectionRec> {
        self.connections
            .values()
            .filter(move |c| &c.region == region)
    }

    /// How many records there are, all kinds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.regions.len()
            + self.localities.len()
            + self.neighborhoods.len()
            + self.sites.len()
            + self.structures.len()
            + self.checkpoints.len()
            + self.connections.len()
            + self.features.len()
            + self.portals.len()
            + self.tiles.len()
            + self.hooks.len()
            + self.secrets.len()
            + self.roomgeo.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(p: &str, e: &str) -> RecordKey {
        RecordKey::new(
            CanonicalId::new(p).expect("pack"),
            CanonicalId::new(e).expect("entry"),
        )
    }

    #[test]
    fn keys_print_and_parse() {
        let k = key("region.limbo", "locality.lot");
        assert_eq!(k.suffix(), "region.limbo:locality.lot");
        assert_eq!(RecordKey::parse_suffix(&k.suffix()), Some(k.clone()));
        assert_eq!(
            record_key(KIND_LOCALITY, &k),
            "locality:region.limbo:locality.lot"
        );
        assert_eq!(RecordKey::parse_suffix("no-colon"), None);
        assert_eq!(roomgeo_key(ObjId(9)), "roomgeo:9");
    }

    #[test]
    fn a_wire_refuses_unknown_fields_and_validate_refuses_bad_bounds() {
        let json = serde_json::json!({
            "key": {"pack": "p", "entry": "e"},
            "region": {"pack": "p", "entry": "r"},
            "name": "The Lot",
            "kind": "lot",
            "footprint": [[0,0],[10,0],[10,10]],
            "revision": 1,
            "surprise": true
        });
        assert!(serde_json::from_value::<LocalityRec>(json).is_err());
        let bad = LocalityRec {
            key: key("p", "e"),
            region: key("p", "r"),
            name: "The Lot".to_string(),
            kind: "Lot".to_string(),
            footprint: Polygon2(vec![
                Point2(0.0, 0.0),
                Point2(10.0, 0.0),
                Point2(10.0, 10.0),
            ]),
            revision: 1,
        };
        assert!(bad.validate().is_err(), "kind must be a lowercase word");
    }

    #[test]
    fn a_background_mix_over_100_is_refused() {
        let recipe = BackgroundRecipe {
            surface: TerrainClass::Paved,
            mix: vec![(TerrainClass::Grass, 60), (TerrainClass::Water, 50)],
            relief_cm: 0,
            relief_period_cm: 800,
            water_cm: None,
            seed: 1,
        };
        assert!(recipe.validate().is_err());
    }
}
