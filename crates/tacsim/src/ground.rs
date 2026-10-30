//! **World generator v2: the ground draft, the generator, and the pack writer**
//! (doc 30 §4, D-30-1..3; cut WG.1).
//!
//! Every authoring source — the procedural generator ([`generate`]), the Unity
//! extractor, and the web Terrain & Arena Studio — emits the same intermediate
//! document, [`GroundDraft`] (`ground.draft.v1`), and one writer ([`build`])
//! turns a draft into the two signed packs an arena loads from:
//! - the **atlas region pack** (`tacsim.<id>`), holding `region.<id>`,
//!   `locality.<id>` and the `128 × 128` `field.q{x}_{y}` heightfield quadrants;
//! - the **campaign-seed arena pack** (`tacsim.pack.<id>`), holding
//!   `tacsim.arena.<id>` ([`ArenaDef`]).
//!
//! [`from_packs`] is the inverse (`pack → draft`) so an existing arena pack pair
//! can be opened in the studio or checked by `ground-pack`.

use std::fmt::Write as _;

use mush_atlas::TerrainClass;
use mush_atlas::cells::{base64_decode, base64_encode};
use mush_atlas::def::{HeightfieldDef, RegionDef};
use mush_atlas::physics::LocomotionTable;
use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};

use crate::arena::state::{ArenaTerrain, KeySpot as ArenaKeySpot, Team, TerrainCell};
use crate::definitions::arena::{
    ArenaBuildError, ArenaDef, ArenaModeDef, BIOMES, EntryZoneDef, KeySpot, MAX_ARENA_TILES,
    TankDef, WaypointGraph, WaypointNode, arena_map_within_bound,
};
use crate::definitions::content;
use crate::match_::cockpit::MAP_MAX_BYTES;

/// Schema identifier for [`GroundDraft`] (doc 30 §4, D-30-3).
pub const DRAFT_SCHEMA: &str = "ground.draft.v1";

/// The default window side, in cells (D-16-5).
pub const SIDE: u16 = 256;
/// One cell, centimetres (doc 16 §3.2).
pub const CELL_CM: u16 = 1_000;
/// One heightfield entry's side. Four quadrants tile a 256 × 256 window; up to
/// sixteen tile a 512 × 512 window (`MAX_ENTRY_BYTES` is 64 KiB).
pub const QUAD: u16 = 128;
/// The proving ground region's south-west corner, metres.
pub const ORIGIN_M: i64 = 20_000;
/// Where the river runs, in columns, inclusive.
pub const RIVER: (u16, u16) = (62, 66);
/// Where the ford crosses it, in rows, inclusive.
pub const FORD: (u16, u16) = (92, 102);
/// The east-west road, in rows, inclusive.
pub const ROAD_EW: (u16, u16) = (96, 98);
/// The north-south road, in columns, inclusive.
pub const ROAD_NS: (u16, u16) = (128, 130);

/// The three built compounds: `(x0, x1, y0, y1)`, inclusive.
pub const BUILDINGS: [(u16, u16, u16, u16); 3] = [
    (150, 170, 36, 52),
    (96, 116, 186, 202),
    (200, 216, 140, 156),
];

/// Light woods: `(x0, x1, y0, y1)`.
pub const WOODS_LIGHT: [(u16, u16, u16, u16); 2] = [(20, 58, 150, 200), (160, 210, 40, 80)];
/// Heavy woods, inside the light ones.
pub const WOODS_HEAVY: [(u16, u16, u16, u16); 2] = [(30, 48, 165, 185), (175, 193, 50, 70)];
/// Broken ground.
pub const RUBBLE: [(u16, u16, u16, u16); 1] = [(100, 112, 60, 72)];
/// Bare earth.
pub const BARE: [(u16, u16, u16, u16); 2] = [(210, 255, 0, 36), (0, 40, 0, 24)];

/// **The Bluff's top** (16b cut AE.1; 16c `T-18`): `(x0, x1, y0, y1)`, inclusive.
pub const BLUFF_TOP: (u16, u16, u16, u16) = (141, 147, 105, 118);

/// The top's elevation, centimetres against the region datum — **absolute**.
pub const BLUFF_TOP_CM: i16 = 500;

/// **The ramp down the Bluff's north side**, each box `(x0, x1, y0, y1)` and its
/// absolute elevation.
pub const BLUFF_RAMP: [((u16, u16, u16, u16), i16); 2] =
    [((141, 146, 119, 119), 270), ((141, 146, 120, 120), 40)];

/// **The fought-over ground** (doc 21 cut G.7b, version 3): the rows of the
/// east-west road's corridor, inclusive.
pub const V3_CORRIDOR: (u16, u16) = (86, 108);
/// The cells over which version 3's landscape blends in from the preserved ground.
pub const V3_BLEND: f64 = 14.0;
/// The Bluff and its ramp with a margin: version 2's to the centimetre in version 3.
pub const V3_BLUFF_KEEP: (u16, u16, u16, u16) = (136, 152, 100, 125);
/// **The Bowl** (version 3): an impact crater south of the crossroads.
pub const V3_BOWL: (f64, f64, f64) = (88.0, 36.0, 18.0);
/// **Signal Hill** (version 3): the highlands' summit — column, row.
pub const V3_SIGNAL_HILL: (f64, f64) = (185.0, 215.0);

/// The placeholder a generated or hand-authored pack carries until it is signed.
pub const UNSIGNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Which authoring surface produced a [`GroundDraft`] (doc 30 §4, D-30-3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundSource {
    /// Procedural Rust generator (`tacsim::ground::generate`).
    Generator,
    /// Unity heightfield/splatmap extractor (`scripts/ground-extract/`).
    Extractor,
    /// Web Terrain & Arena Studio (`web/src/components/MapEditorStudio.svelte`).
    Studio,
    /// Reconstructed from a pack pair (`tacsim::ground::from_packs`), with the
    /// pack IDs and digests recorded in `provenance.inputs`.
    Pack,
}

/// Provenance metadata carried by a [`GroundDraft`] (doc 30 §4, D-30-3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundProvenance {
    /// Which authoring surface produced this draft.
    pub source: GroundSource,
    /// Generator preset/version, extractor version, or studio version string.
    pub version: String,
    /// Optional deterministic generator seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Named upstream asset packs and versions (required when `source == Extractor`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

fn default_one() -> u32 {
    1
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(v: &u32) -> bool {
    *v == 1
}

/// **`ground.draft.v1`** — the single intermediate document between every
/// authoring surface and the pack writer (doc 30 §4, D-30-3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundDraft {
    /// Always [`DRAFT_SCHEMA`] (`"ground.draft.v1"`).
    pub schema: String,
    /// Canonical slug (`proving_ground` -> `region.<id>`, `tacsim.<id>`, `tacsim.arena.<id>`).
    pub id: String,
    /// Human-facing arena and region name.
    pub label: String,
    /// Regional biome (`temperate`, `mars`, `lunar`, `ice`).
    pub biome: String,
    /// Astronomical body (`luna`, `mars`, etc.), independent of `biome` (Q-30-2).
    pub body: String,
    /// South-west corner of the arena window in region metres, `[east, north]`.
    pub origin_m: [i64; 2],
    /// Cell resolution in centimetres (`1000` = 10 m).
    pub cell_cm: u16,
    /// Grid width in cells.
    pub cols: u16,
    /// Grid height in cells.
    pub rows: u16,
    /// Base64 over `[cols u16 LE][rows u16 LE]` + `[count u16 LE][class u8]`,
    /// southern row first (`HeightfieldDef.classes`).
    pub classes: String,
    /// Base64 over `cols × rows` little-endian `i16` elevation samples (`cm`),
    /// southern row first (`HeightfieldDef.data` with `encoding: "le16"`).
    pub elev: String,
    /// Red and Blue deployment zones.
    pub entry_zones: Vec<EntryZoneDef>,
    /// Named tactical spots (`A`..`V`).
    pub key_spots: Vec<KeySpot>,
    /// AI patrol waypoint graph (may be empty prior to `ground-pack route`).
    pub waypoints: WaypointGraph,
    /// Optional match modes beyond Skirmish (e.g. Hold).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<ArenaModeDef>,
    /// Optional stock tank roster; defaults to the 12-unit proving ground roster
    /// positioned inside `entry_zones` when omitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tanks: Vec<TankDef>,
    /// Atlas pack manifest version (defaults to `1`; `3` for the shipped proving ground).
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    pub atlas_version: u32,
    /// Campaign-seed arena pack manifest version (defaults to `1`; `5` for the shipped proving ground).
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    pub arena_version: u32,
    /// Authoring provenance (required for all drafts; `inputs` required for `Extractor`).
    pub provenance: GroundProvenance,
    /// Authoring rationale and check summary.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Errors returned while validating, generating, building, or checking a [`GroundDraft`].
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GroundError {
    /// JSON parse or serialization error.
    #[error("ground draft JSON error: {0}")]
    Json(String),
    /// Schema or structural validation failure.
    #[error("ground draft `{id}` invalid: {detail}")]
    Invalid {
        /// Draft slug or `"<unknown>"`.
        id: String,
        /// Reason for refusal.
        detail: String,
    },
    /// Arena definition validation failure.
    #[error("ground draft `{id}` arena check failed: {detail}")]
    ArenaCheck {
        /// Draft slug.
        id: String,
        /// Underlying [`ArenaDef::check`] error.
        detail: String,
    },
    /// Built arena or map frame failed an apply-time bound (e.g. `MAP_MAX_BYTES`).
    #[error(transparent)]
    ArenaBuild(#[from] ArenaBuildError),
    /// Pack parsing or digest error.
    #[error("ground pack error: {0}")]
    Pack(String),
}

/// Output of [`build`]: the two signed JSON packs and their manifests' digests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltPacks {
    /// Signed atlas region pack JSON (`tacsim.<id>`).
    pub atlas_json: String,
    /// Computed digest of `atlas_json`.
    pub atlas_digest: DefinitionDigest,
    /// Signed campaign-seed arena pack JSON (`tacsim.pack.<id>`).
    pub arena_json: String,
    /// Computed digest of `arena_json`.
    pub arena_digest: DefinitionDigest,
}

/// Output of [`check`]: wire size and grid shape verification (extended in WG.3
/// with grade and passability metrics).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    /// Draft slug.
    pub id: String,
    /// Grid columns.
    pub cols: u16,
    /// Grid rows.
    pub rows: u16,
    /// Number of 128 × 128 heightfield quadrants.
    pub quadrants: usize,
    /// Encoded byte length of the widest `arena.map` frame.
    pub map_bytes: usize,
    /// Maximum allowed byte length (`MAP_MAX_BYTES`, 262,144).
    pub map_max_bytes: usize,
}

/// Encode a `cols × rows` slice of [`TerrainClass`] into the 3-byte run-length
/// format of `HeightfieldDef.classes` (`[cols u16 LE][rows u16 LE]` + `[count u16 LE][class u8]`).
#[must_use]
pub fn encode_classes(cols: u16, rows: u16, classes: &[TerrainClass]) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&cols.to_le_bytes());
    bytes.extend_from_slice(&rows.to_le_bytes());
    let mut run: Option<(u8, u16)> = None;
    let flush = |out: &mut Vec<u8>, current: Option<(u8, u16)>| {
        if let Some((code, n)) = current {
            out.extend_from_slice(&n.to_le_bytes());
            out.push(code);
        }
    };
    for class in classes {
        let code = class.code();
        match run {
            Some((prev, n)) if prev == code && n < u16::MAX => run = Some((prev, n + 1)),
            other => {
                flush(&mut bytes, other);
                run = Some((code, 1));
            }
        }
    }
    flush(&mut bytes, run);
    base64_encode(&bytes)
}

/// Decode a 3-byte run-length `classes` string (`[cols u16 LE][rows u16 LE]` +
/// `[count u16 LE][class u8]`), refusing `TerrainClass::Void` (`0`) per D-30-3.
///
/// # Errors
///
/// [`GroundError::Invalid`] if the base64, dimensions, class codes, or run lengths
/// are invalid, or if any cell carries code `0` (`Void`).
pub fn decode_classes(
    id: &str,
    expected_cols: u16,
    expected_rows: u16,
    encoded: &str,
) -> Result<Vec<TerrainClass>, GroundError> {
    let err = |detail: String| GroundError::Invalid {
        id: id.to_owned(),
        detail,
    };
    let bytes = base64_decode(encoded.trim())
        .ok_or_else(|| err("classes is not valid base64".to_owned()))?;
    if bytes.len() < 4 || (bytes.len() - 4) % 3 != 0 {
        return Err(err(format!(
            "classes byte length {} is not 4 + 3*k",
            bytes.len()
        )));
    }
    let cols = u16::from_le_bytes([
        bytes.first().copied().unwrap_or(0),
        bytes.get(1).copied().unwrap_or(0),
    ]);
    let rows = u16::from_le_bytes([
        bytes.get(2).copied().unwrap_or(0),
        bytes.get(3).copied().unwrap_or(0),
    ]);
    if cols != expected_cols || rows != expected_rows {
        return Err(err(format!(
            "classes header is {cols}×{rows}, expected {expected_cols}×{expected_rows}"
        )));
    }
    let total = usize::from(cols) * usize::from(rows);
    let mut out = Vec::with_capacity(total);
    let mut offset = 4_usize;
    let mut run_idx = 0_usize;
    while offset < bytes.len() {
        let count = usize::from(u16::from_le_bytes([
            bytes.get(offset).copied().unwrap_or(0),
            bytes.get(offset + 1).copied().unwrap_or(0),
        ]));
        let code = bytes.get(offset + 2).copied().unwrap_or(0);
        let class = TerrainClass::from_code(code).ok_or_else(|| {
            err(format!(
                "classes run {run_idx} has unknown terrain class code {code}"
            ))
        })?;
        if class == TerrainClass::Void {
            return Err(err(format!(
                "classes run {run_idx} uses code 0 (Void); a draft must author explicit ground"
            )));
        }
        if count == 0 || out.len().saturating_add(count) > total {
            return Err(err(format!(
                "classes run {run_idx} of length {count} overflows {cols}×{rows} grid"
            )));
        }
        out.extend(std::iter::repeat_n(class, count));
        offset += 3;
        run_idx += 1;
    }
    if out.len() != total {
        return Err(err(format!(
            "classes covers {} of {total} cells",
            out.len()
        )));
    }
    Ok(out)
}

/// Encode `cols × rows` `i16` elevation samples (centimetres) as `le16` base64.
#[must_use]
pub fn encode_elev(samples: &[i16]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    base64_encode(&bytes)
}

/// Decode `cols × rows` `i16` elevation samples from `le16` base64.
///
/// # Errors
///
/// [`GroundError::Invalid`] if the base64 is malformed or sample count mismatches `cols × rows`.
pub fn decode_elev(
    id: &str,
    expected_cols: u16,
    expected_rows: u16,
    encoded: &str,
) -> Result<Vec<i16>, GroundError> {
    let err = |detail: String| GroundError::Invalid {
        id: id.to_owned(),
        detail,
    };
    let bytes =
        base64_decode(encoded.trim()).ok_or_else(|| err("elev is not valid base64".to_owned()))?;
    if bytes.len() % 2 != 0 {
        return Err(err("elev le16 data has an odd number of bytes".to_owned()));
    }
    let total = usize::from(expected_cols) * usize::from(expected_rows);
    let (pairs, _) = bytes.as_chunks::<2>();
    if pairs.len() != total {
        return Err(err(format!(
            "elev has {} samples, expected {total} for {expected_cols}×{expected_rows}",
            pairs.len()
        )));
    }
    Ok(pairs.iter().map(|p| i16::from_le_bytes(*p)).collect())
}

impl GroundDraft {
    /// Parse a [`GroundDraft`] from JSON text.
    ///
    /// # Errors
    ///
    /// [`GroundError::Json`] on syntax/schema errors.
    pub fn from_json(text: &str) -> Result<Self, GroundError> {
        serde_json::from_str(text).map_err(|e| GroundError::Json(e.to_string()))
    }

    /// Serialize this [`GroundDraft`] to pretty JSON with a trailing newline.
    ///
    /// # Errors
    ///
    /// [`GroundError::Json`] if serialization fails.
    pub fn to_json_pretty(&self) -> Result<String, GroundError> {
        let mut s =
            serde_json::to_string_pretty(self).map_err(|e| GroundError::Json(e.to_string()))?;
        s.push('\n');
        Ok(s)
    }

    /// Validate all structural, provenance, grid, and [`ArenaDef`] invariants,
    /// returning the decoded `(classes, elev)` vectors.
    ///
    /// # Errors
    ///
    /// [`GroundError`] describing the first violated invariant.
    pub fn validate(&self) -> Result<(Vec<TerrainClass>, Vec<i16>), GroundError> {
        let err = |detail: String| GroundError::Invalid {
            id: self.id.clone(),
            detail,
        };
        if self.schema != DRAFT_SCHEMA {
            return Err(err(format!(
                "schema must be `{DRAFT_SCHEMA}`, got `{}`",
                self.schema
            )));
        }
        if self.id.is_empty() {
            return Err(err("id must not be empty".to_owned()));
        }
        // Check that all derived pack and record IDs are valid CanonicalIds.
        for derived in [
            format!("tacsim.{}", self.id),
            format!("tacsim.pack.{}", self.id),
            format!("tacsim.arena.{}", self.id),
            format!("region.{}", self.id),
            format!("locality.{}", self.id),
        ] {
            CanonicalId::new(&derived)
                .map_err(|e| err(format!("derived id `{derived}` invalid: {e}")))?;
        }
        if !BIOMES.contains(&self.biome.as_str()) {
            return Err(err(format!(
                "biome must be one of {BIOMES:?}, got `{}`",
                self.biome
            )));
        }
        if self.body.trim().is_empty() {
            return Err(err("body must not be empty".to_owned()));
        }
        if self.cols == 0 || self.rows == 0 || self.cols > 512 || self.rows > 512 {
            return Err(err(format!(
                "draft dimensions must be in 1..=512; got {}×{}",
                self.cols, self.rows
            )));
        }
        let qx_count = usize::from(self.cols.div_ceil(QUAD));
        let qy_count = usize::from(self.rows.div_ceil(QUAD));
        let quadrants = qx_count.saturating_mul(qy_count);
        if quadrants == 0 || quadrants > MAX_ARENA_TILES {
            return Err(err(format!(
                "draft requires {quadrants} quadrants of {QUAD}×{QUAD}, max is {MAX_ARENA_TILES}"
            )));
        }
        if self.provenance.version.trim().is_empty() {
            return Err(err("provenance.version must not be empty".to_owned()));
        }
        if self.provenance.source == GroundSource::Extractor && self.provenance.inputs.is_empty() {
            return Err(err(
                "an extractor draft must name its upstream pack(s) in provenance.inputs".to_owned(),
            ));
        }
        if self.provenance.source == GroundSource::Pack && self.provenance.inputs.is_empty() {
            return Err(err(
                "a pack-reconstructed draft must name its pack id(s) and digest(s) in provenance.inputs"
                    .to_owned(),
            ));
        }

        let classes = decode_classes(&self.id, self.cols, self.rows, &self.classes)?;
        let elev = decode_elev(&self.id, self.cols, self.rows, &self.elev)?;

        let arena_def = self.to_arena_def()?;
        let arena_entry = format!("tacsim.arena.{}", self.id);
        arena_def
            .check(&arena_entry)
            .map_err(|e| GroundError::ArenaCheck {
                id: self.id.clone(),
                detail: e.to_string(),
            })?;

        Ok((classes, elev))
    }

    /// Build the [`ArenaDef`] corresponding to this draft.
    ///
    /// # Errors
    ///
    /// [`GroundError::Invalid`] if any canonical ID cannot be constructed.
    pub fn to_arena_def(&self) -> Result<ArenaDef, GroundError> {
        let err = |detail: String| GroundError::Invalid {
            id: self.id.clone(),
            detail,
        };
        let pack =
            CanonicalId::new(format!("tacsim.{}", self.id)).map_err(|e| err(e.to_string()))?;
        let region =
            CanonicalId::new(format!("region.{}", self.id)).map_err(|e| err(e.to_string()))?;
        let qx_count = self.cols.div_ceil(QUAD);
        let qy_count = self.rows.div_ceil(QUAD);
        let mut tiles = Vec::with_capacity(usize::from(qx_count) * usize::from(qy_count));
        for qy in 0..qy_count {
            for qx in 0..qx_count {
                let tile_id = CanonicalId::new(format!("field.q{qx}_{qy}"))
                    .map_err(|e| err(e.to_string()))?;
                tiles.push(tile_id);
            }
        }
        let tanks = if self.tanks.is_empty() {
            default_tanks_for_zones(&self.entry_zones, self.rows)?
        } else {
            self.tanks.clone()
        };
        Ok(ArenaDef {
            label: self.label.clone(),
            pack,
            region,
            tiles,
            origin_m: self.origin_m,
            cols: self.cols,
            rows: self.rows,
            cell_cm: self.cell_cm,
            entry_zones: self.entry_zones.clone(),
            key_spots: self.key_spots.clone(),
            waypoints: self.waypoints.clone(),
            modes: self.modes.clone(),
            tanks,
            biome: None,
            procedural_relief: false,
            note: self.note.clone(),
        })
    }
}

fn default_tanks_for_zones(zones: &[EntryZoneDef], rows: u16) -> Result<Vec<TankDef>, GroundError> {
    let unit = CanonicalId::new("tacsim.unit.line_tank").map_err(|e| GroundError::Invalid {
        id: "default_tanks".to_owned(),
        detail: e.to_string(),
    })?;
    let mid_y = (rows / 2).clamp(4, rows.saturating_sub(5));
    let mut red_x = 6_u16;
    let mut blue_x = 249_u16;
    for z in zones {
        match z.team {
            Team::Red => red_x = (z.x0 + z.x1) / 2,
            Team::Blue => blue_x = (z.x0 + z.x1) / 2,
        }
    }
    let offsets: [(i16, i16); 6] = [(0, 0), (0, -2), (0, 2), (0, -4), (0, 4), (-2, -3)];
    let mut out = Vec::with_capacity(12);
    for (idx, (dx, dy)) in offsets.iter().copied().enumerate() {
        let slot = idx + 1;
        let id = CanonicalId::new(format!("red_{slot}")).map_err(|e| GroundError::Invalid {
            id: "default_tanks".to_owned(),
            detail: e.to_string(),
        })?;
        let x = u16::try_from((i32::from(red_x) + i32::from(dx)).max(0)).unwrap_or(red_x);
        let y = u16::try_from((i32::from(mid_y) + i32::from(dy)).max(0)).unwrap_or(mid_y);
        out.push(TankDef {
            id,
            name: format!("Red Tank {slot}"),
            team: Team::Red,
            x,
            y,
            heading_mdeg: 90_000,
            unit: unit.clone(),
        });
    }
    for (idx, (dx, dy)) in offsets.iter().copied().enumerate() {
        let slot = idx + 1;
        let id = CanonicalId::new(format!("blue_{slot}")).map_err(|e| GroundError::Invalid {
            id: "default_tanks".to_owned(),
            detail: e.to_string(),
        })?;
        let x = u16::try_from((i32::from(blue_x) - i32::from(dx)).max(0)).unwrap_or(blue_x);
        let y = u16::try_from((i32::from(mid_y) + i32::from(dy)).max(0)).unwrap_or(mid_y);
        out.push(TankDef {
            id,
            name: format!("Blue Tank {slot}"),
            team: Team::Blue,
            x,
            y,
            heading_mdeg: 270_000,
            unit: unit.clone(),
        });
    }
    Ok(out)
}

/// Whether `(x, y)` is inside `box_`, `(x0, x1, y0, y1)` inclusive.
#[must_use]
pub fn within(x: u16, y: u16, box_: (u16, u16, u16, u16)) -> bool {
    x >= box_.0 && x <= box_.1 && y >= box_.2 && y <= box_.3
}

/// The authored class of one cell on the proving ground.
#[must_use]
pub fn class_at(x: u16, y: u16) -> TerrainClass {
    for building in BUILDINGS {
        if within(x, y, building) {
            let door_x = (building.0 + building.1) / 2;
            if y == building.2 && x == door_x {
                return TerrainClass::Door;
            }
            let edge = x == building.0 || x == building.1 || y == building.2 || y == building.3;
            return if edge {
                TerrainClass::Wall
            } else {
                TerrainClass::Floor
            };
        }
    }
    let in_river = x >= RIVER.0 && x <= RIVER.1;
    if in_river {
        return if y >= FORD.0 && y <= FORD.1 {
            TerrainClass::Water
        } else {
            TerrainClass::WaterDeep
        };
    }
    if (y >= ROAD_EW.0 && y <= ROAD_EW.1) || (x >= ROAD_NS.0 && x <= ROAD_NS.1) {
        return TerrainClass::Road;
    }
    for patch in WOODS_HEAVY {
        if within(x, y, patch) {
            return TerrainClass::WoodsHeavy;
        }
    }
    for patch in WOODS_LIGHT {
        if within(x, y, patch) {
            return TerrainClass::WoodsLight;
        }
    }
    for patch in RUBBLE {
        if within(x, y, patch) {
            return TerrainClass::Rubble;
        }
    }
    for patch in BARE {
        if within(x, y, patch) {
            return TerrainClass::Ground;
        }
    }
    TerrainClass::Grass
}

/// Version 1 (`bluff = false`) and Version 2 (`bluff = true`) relief of the
/// proving ground, centimetres against the region datum.
#[must_use]
pub fn elev_at(bluff: bool, x: u16, y: u16) -> i16 {
    let fx = f64::from(x);
    let fy = f64::from(y);
    let raw =
        200.0 * (fx / 40.0).sin() + 150.0 * (fy / 55.0).cos() + 100.0 * ((fx + fy) / 70.0).sin();
    let mut cm = (raw / 100.0).round() * 100.0;
    if matches!(
        class_at(x, y),
        TerrainClass::Water | TerrainClass::WaterDeep
    ) {
        cm -= 200.0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let cm = cm as i32;
    let cm = i16::try_from(cm).unwrap_or(0);
    if bluff {
        if within(x, y, BLUFF_TOP) {
            return BLUFF_TOP_CM;
        }
        for (box_, ramp_cm) in BLUFF_RAMP {
            if within(x, y, box_) {
                return ramp_cm;
            }
        }
    }
    cm
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sq(v: f64) -> f64 {
    v * v
}

fn gauss(dx: f64, dy: f64, sigma: f64) -> f64 {
    libm::exp(-(dx * dx + dy * dy) / (2.0 * sigma * sigma))
}

fn box_distance(x: u16, y: u16, box_: (u16, u16, u16, u16)) -> f64 {
    let dx = f64::from(box_.0.saturating_sub(x).max(x.saturating_sub(box_.1)));
    let dy = f64::from(box_.2.saturating_sub(y).max(y.saturating_sub(box_.3)));
    dx.max(dy)
}

fn v3_features(fx: f64, fy: f64) -> f64 {
    let highlands = 3_500.0
        * smoothstep((fy - 118.0) / 45.0)
        * (0.65 + 0.35 * libm::sin(fx / 19.0) * libm::cos(fy / 15.0));
    let signal = 8_000.0 * gauss(fx - V3_SIGNAL_HILL.0, fy - V3_SIGNAL_HILL.1, 20.0);
    let knoll = 4_500.0 * gauss(fx - 40.0, fy - 225.0, 16.0);
    let hills = 3_000.0 * gauss(fx - 205.0, fy - 128.0, 12.0)
        + 3_500.0 * gauss(fx - 28.0, fy - 58.0, 14.0)
        + 4_000.0 * gauss(fx - 228.0, fy - 188.0, 15.0);
    let (bx, by, rim) = V3_BOWL;
    let r = libm::hypot(fx - bx, fy - by);
    let bowl = if r < rim {
        -1_200.0 + 3_200.0 * (r / rim) * (r / rim)
    } else {
        2_000.0 * libm::exp(-sq((r - rim) / 9.0))
    };
    let valley = -1_200.0 * libm::exp(-sq((fx - 64.0) / 16.0));
    let spine = 2_000.0
        * libm::exp(-sq((fy - 66.0) / 6.0))
        * smoothstep((fx - 180.0) / 12.0)
        * smoothstep((240.0 - fx) / 12.0);
    let shelf = 2_500.0
        * smoothstep((fx - 206.0) / 12.0)
        * smoothstep((254.0 - fx) / 12.0)
        * smoothstep((fy + 2.0) / 12.0)
        * smoothstep((36.0 - fy) / 12.0);
    highlands + signal + knoll + hills + bowl + valley + spine + shelf
}

/// **Version 3's relief** (doc 21 cut G.7b): version 2's relief ([`elev_at`]
/// with the bluff), unchanged on the fought-over ground, and `v3_features`
/// blending in over [`V3_BLEND`] cells away from them.
#[must_use]
pub fn elev_v3(x: u16, y: u16) -> i16 {
    let base = f64::from(elev_at(true, x, y));
    let corridor = f64::from(
        V3_CORRIDOR
            .0
            .saturating_sub(y)
            .max(y.saturating_sub(V3_CORRIDOR.1)),
    );
    let road = f64::from(x.abs_diff(ROAD_NS.0 + 1)) - 1.0;
    let keep = corridor
        .min(road.max(0.0))
        .min(box_distance(x, y, V3_BLUFF_KEEP));
    let w = smoothstep(keep / V3_BLEND);
    let (fx, fy) = (f64::from(x), f64::from(y));
    let mut feature = v3_features(fx, fy);
    for b in BUILDINGS {
        let near = box_distance(
            x,
            y,
            (
                b.0.saturating_sub(2),
                b.1 + 2,
                b.2.saturating_sub(2),
                b.3 + 2,
            ),
        );
        let pad = v3_features(f64::from(b.0 + b.1) / 2.0, f64::from(b.2 + b.3) / 2.0);
        let t = smoothstep(near / 6.0);
        feature = pad + (feature - pad) * t;
    }
    let cm = ((base + w * feature) / 10.0).round() * 10.0;
    #[allow(clippy::cast_possible_truncation)]
    let cm = cm.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    cm
}

/// Which relief version of the proving ground a pack is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relief {
    /// Version 1: the seeded relief alone.
    V1,
    /// Version 2: and The Bluff.
    V2,
    /// Version 3: and the showcase landscape (doc 21 cut G.7b).
    V3,
}

impl Relief {
    fn elev(self, x: u16, y: u16) -> i16 {
        match self {
            Relief::V1 => elev_at(false, x, y),
            Relief::V2 => elev_at(true, x, y),
            Relief::V3 => elev_v3(x, y),
        }
    }

    fn version(self) -> u32 {
        match self {
            Relief::V1 => 1,
            Relief::V2 => 2,
            Relief::V3 => 3,
        }
    }
}

fn quadrant_le16_from_grid(
    cols: u16,
    elev: &[i16],
    qx: u16,
    qy: u16,
    q_cols: u16,
    q_rows: u16,
) -> String {
    let mut bytes = Vec::with_capacity(usize::from(q_cols) * usize::from(q_rows) * 2);
    for y in 0..q_rows {
        let wy = usize::from(qy * QUAD + y);
        for x in 0..q_cols {
            let wx = usize::from(qx * QUAD + x);
            let idx = wy * usize::from(cols) + wx;
            let sample = elev.get(idx).copied().unwrap_or(0);
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    base64_encode(&bytes)
}

fn quadrant_classes_from_grid(
    cols: u16,
    classes: &[TerrainClass],
    qx: u16,
    qy: u16,
    q_cols: u16,
    q_rows: u16,
) -> String {
    let mut sub = Vec::with_capacity(usize::from(q_cols) * usize::from(q_rows));
    for y in 0..q_rows {
        let wy = usize::from(qy * QUAD + y);
        for x in 0..q_cols {
            let wx = usize::from(qx * QUAD + x);
            let idx = wy * usize::from(cols) + wx;
            sub.push(classes.get(idx).copied().unwrap_or(TerrainClass::Grass));
        }
    }
    encode_classes(q_cols, q_rows, &sub)
}

/// Compute a pack's own digest and stamp it into its manifest, or return the
/// input unchanged if it cannot be parsed.
#[must_use]
pub fn sign(json: &str) -> String {
    let Ok(parsed) = mush_content::parse_pack(json) else {
        return json.to_owned();
    };
    let Ok(digest) = parsed.computed_digest() else {
        return json.to_owned();
    };
    json.replace(UNSIGNED, &digest.to_hex())
}

/// Compute a pack's digest and stamp it into its manifest, returning both the
/// signed string and the [`DefinitionDigest`].
///
/// # Errors
///
/// [`GroundError::Pack`] if `mush_content::parse_pack` or `computed_digest` fails.
pub fn sign_checked(json: &str) -> Result<(String, DefinitionDigest), GroundError> {
    let parsed = mush_content::parse_pack(json).map_err(|e| GroundError::Pack(e.to_string()))?;
    let digest = parsed
        .computed_digest()
        .map_err(|e| GroundError::Pack(e.to_string()))?;
    Ok((json.replace(UNSIGNED, &digest.to_hex()), digest))
}

/// **The proving ground's atlas pack** at version 1 (`bluff = false`) or
/// version 2 (`bluff = true`).
#[must_use]
pub fn atlas_pack_at(bluff: bool) -> String {
    atlas_pack_with(if bluff { Relief::V2 } else { Relief::V1 })
}

/// **Version 3, the shipped proving ground atlas pack** (doc 21 cut G.7b).
#[must_use]
pub fn atlas_pack_v3() -> String {
    atlas_pack_with(Relief::V3)
}

/// Build the proving ground's atlas pack with `relief`.
#[must_use]
pub fn atlas_pack_with(relief: Relief) -> String {
    let mut classes = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    let mut elev = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    for y in 0..SIDE {
        for x in 0..SIDE {
            classes.push(class_at(x, y));
            elev.push(relief.elev(x, y));
        }
    }
    build_atlas_pack_json(
        "proving_ground",
        "The Proving Ground",
        "luna",
        [ORIGIN_M, ORIGIN_M],
        CELL_CM,
        SIDE,
        SIDE,
        relief.version(),
        2350,
        &classes,
        &elev,
    )
    .map_or_else(|_| String::new(), |(signed, _)| signed)
}

#[allow(clippy::too_many_arguments)]
fn build_atlas_pack_json(
    id: &str,
    label: &str,
    body: &str,
    origin_m: [i64; 2],
    cell_cm: u16,
    cols: u16,
    rows: u16,
    version: u32,
    bg_seed: u64,
    classes: &[TerrainClass],
    elev: &[i16],
) -> Result<(String, DefinitionDigest), GroundError> {
    let lo_x = origin_m[0];
    let lo_y = origin_m[1];
    let hi_x = lo_x + i64::from(cols) * i64::from(cell_cm) / 100;
    let hi_y = lo_y + i64::from(rows) * i64::from(cell_cm) / 100;
    let region_id = format!("region.{id}");
    let locality_id = format!("locality.{id}");
    let pack_id = if id == "proving_ground" {
        content::PROVING_GROUND_ATLAS_PACK.to_owned()
    } else {
        format!("tacsim.{id}")
    };

    let mut entries = serde_json::Map::new();
    entries.insert(
        region_id.clone(),
        serde_json::json!({
            "kind": "region",
            "def": {
                "name": label,
                "kind": "surface",
                "body": body,
                "extent": [[lo_x, lo_y], [hi_x, hi_y]],
                "datum_m": 0.0,
                "background": {
                    "surface": "ground",
                    "mix": [],
                    "relief_cm": 0,
                    "relief_period_cm": 1000,
                    "water_cm": null,
                    "seed": bg_seed
                },
                "ground_resolution_cm": cell_cm
            }
        }),
    );
    entries.insert(
        locality_id,
        serde_json::json!({
            "kind": "locality",
            "def": {
                "region": region_id,
                "name": label,
                "kind": "lot",
                "footprint": [[lo_x, lo_y], [hi_x, lo_y], [hi_x, hi_y], [lo_x, hi_y]]
            }
        }),
    );

    let qx_count = cols.div_ceil(QUAD);
    let qy_count = rows.div_ceil(QUAD);
    for qy in 0..qy_count {
        for qx in 0..qx_count {
            let q_cols = QUAD.min(cols.saturating_sub(qx * QUAD));
            let q_rows = QUAD.min(rows.saturating_sub(qy * QUAD));
            let ox = lo_x + i64::from(qx) * i64::from(QUAD) * i64::from(cell_cm) / 100;
            let oy = lo_y + i64::from(qy) * i64::from(QUAD) * i64::from(cell_cm) / 100;
            entries.insert(
                format!("field.q{qx}_{qy}"),
                serde_json::json!({
                    "kind": "heightfield",
                    "def": {
                        "region": region_id,
                        "origin": [ox, oy],
                        "resolution_cm": cell_cm,
                        "cols": q_cols,
                        "rows": q_rows,
                        "encoding": "le16",
                        "data": quadrant_le16_from_grid(cols, elev, qx, qy, q_cols, q_rows),
                        "scale_cm": 1,
                        "datum_cm": 0,
                        "classes": quadrant_classes_from_grid(cols, classes, qx, qy, q_cols, q_rows)
                    }
                }),
            );
        }
    }

    let doc = serde_json::json!({
        "manifest": {
            "id": pack_id,
            "version": version,
            "kind": "region",
            "digest": UNSIGNED,
            "requires": []
        },
        "entries": entries
    });
    let unsigned =
        serde_json::to_string_pretty(&doc).map_err(|e| GroundError::Json(e.to_string()))?;
    sign_checked(&format!("{unsigned}\n"))
}

fn format_arena_pack_json(
    id: &str,
    version: u32,
    def: &ArenaDef,
) -> Result<(String, DefinitionDigest), GroundError> {
    let pack_id = format!("tacsim.pack.{id}");
    let entry_id = format!("tacsim.arena.{id}");
    let json_str = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned());

    let mut out = String::with_capacity(4096);
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"manifest\": {{");
    let _ = writeln!(out, "    \"id\": {},", json_str(&pack_id));
    let _ = writeln!(out, "    \"version\": {version},");
    let _ = writeln!(out, "    \"kind\": \"campaign_seed\",");
    let _ = writeln!(out, "    \"digest\": \"{UNSIGNED}\",");
    let _ = writeln!(out, "    \"requires\": []");
    let _ = writeln!(out, "  }},");
    let _ = writeln!(out, "  \"entries\": {{");
    let _ = writeln!(out, "    {}: {{", json_str(&entry_id));
    let _ = writeln!(out, "      \"kind\": \"arena\",");
    let _ = writeln!(out, "      \"def\": {{");
    let _ = writeln!(out, "        \"label\": {},", json_str(&def.label));
    let _ = writeln!(out, "        \"pack\": {},", json_str(def.pack.as_str()));
    let _ = writeln!(
        out,
        "        \"region\": {},",
        json_str(def.region.as_str())
    );

    let tiles_joined = def
        .tiles
        .iter()
        .map(|t| json_str(t.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "        \"tiles\": [{tiles_joined}],");
    let _ = writeln!(
        out,
        "        \"origin_m\": [{}, {}],",
        def.origin_m[0], def.origin_m[1]
    );
    let _ = writeln!(out, "        \"cols\": {},", def.cols);
    let _ = writeln!(out, "        \"rows\": {},", def.rows);
    let _ = writeln!(out, "        \"cell_cm\": {},", def.cell_cm);

    let _ = writeln!(out, "        \"entry_zones\": [");
    for (i, z) in def.entry_zones.iter().enumerate() {
        let comma = if i + 1 < def.entry_zones.len() {
            ","
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "          {{ \"team\": {}, \"x0\": {}, \"y0\": {}, \"x1\": {}, \"y1\": {} }}{comma}",
            json_str(z.team.as_str()),
            z.x0,
            z.y0,
            z.x1,
            z.y1
        );
    }
    let _ = writeln!(out, "        ],");

    let _ = writeln!(out, "        \"key_spots\": [");
    for (i, s) in def.key_spots.iter().enumerate() {
        let comma = if i + 1 < def.key_spots.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "          {{ \"name\": {}, \"x\": {}, \"y\": {} }}{comma}",
            json_str(&s.name),
            s.x,
            s.y
        );
    }
    let _ = writeln!(out, "        ],");

    let _ = writeln!(out, "        \"waypoints\": {{");
    let _ = writeln!(out, "          \"nodes\": [");
    for (i, n) in def.waypoints.nodes.iter().enumerate() {
        let comma = if i + 1 < def.waypoints.nodes.len() {
            ","
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "            {{ \"id\": {}, \"x\": {}, \"y\": {} }}{comma}",
            json_str(n.id.as_str()),
            n.x,
            n.y
        );
    }
    let _ = writeln!(out, "          ],");
    let _ = writeln!(out, "          \"edges\": [");
    for (i, [a, b]) in def.waypoints.edges.iter().enumerate() {
        let comma = if i + 1 < def.waypoints.edges.len() {
            ","
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "            [{}, {}]{comma}",
            json_str(a.as_str()),
            json_str(b.as_str())
        );
    }
    let _ = writeln!(out, "          ]");
    let _ = writeln!(out, "        }},");

    if !def.modes.is_empty() {
        let _ = writeln!(out, "        \"modes\": [");
        for (i, mode) in def.modes.iter().enumerate() {
            let comma = if i + 1 < def.modes.len() { "," } else { "" };
            match mode {
                ArenaModeDef::Hold {
                    spot,
                    radius_cells,
                    hold_ms,
                } => {
                    let _ = writeln!(
                        out,
                        "          {{ \"mode\": \"hold\", \"spot\": {}, \"radius_cells\": {radius_cells}, \"hold_ms\": {hold_ms} }}{comma}",
                        json_str(spot)
                    );
                }
            }
        }
        let _ = writeln!(out, "        ],");
    }

    let has_note = !def.note.is_empty();
    let _ = writeln!(out, "        \"tanks\": [");
    for (i, t) in def.tanks.iter().enumerate() {
        let comma = if i + 1 < def.tanks.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "          {{ \"id\": {}, \"name\": {}, \"team\": {}, \"x\": {}, \"y\": {}, \"heading_mdeg\": {}, \"unit\": {} }}{comma}",
            json_str(t.id.as_str()),
            json_str(&t.name),
            json_str(t.team.as_str()),
            t.x,
            t.y,
            t.heading_mdeg,
            json_str(t.unit.as_str())
        );
    }
    if has_note {
        let _ = writeln!(out, "        ],");
        let _ = writeln!(out, "        \"note\": {}", json_str(&def.note));
    } else {
        let _ = writeln!(out, "        ]");
    }
    let _ = writeln!(out, "      }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");

    sign_checked(&out)
}

/// **The writer** (doc 30 §4, D-30-2): validate `draft`, quadrant its grid into
/// `<= 64 KiB` heightfield entries, and produce the signed atlas region pack
/// and campaign-seed arena pack.
///
/// # Errors
///
/// [`GroundError`] if validation, [`check`], or pack signing fails.
pub fn build(draft: &GroundDraft) -> Result<BuiltPacks, GroundError> {
    let (classes, elev) = draft.validate()?;
    let _ = check(draft)?;
    let arena_def = draft.to_arena_def()?;
    let bg_seed = draft.provenance.seed.unwrap_or(2350);

    let (atlas_json, atlas_digest) = build_atlas_pack_json(
        &draft.id,
        &draft.label,
        &draft.body,
        draft.origin_m,
        draft.cell_cm,
        draft.cols,
        draft.rows,
        draft.atlas_version,
        bg_seed,
        &classes,
        &elev,
    )?;
    let (arena_json, arena_digest) =
        format_arena_pack_json(&draft.id, draft.arena_version, &arena_def)?;

    Ok(BuiltPacks {
        atlas_json,
        atlas_digest,
        arena_json,
        arena_digest,
    })
}

/// **The check** (doc 30 §4 & §11 WG.1): validate `draft`, price its cells from
/// the shipped locomotion table, and verify that its widest `arena.map` frame
/// fits within [`MAP_MAX_BYTES`] (`256 KiB`).
///
/// # Errors
///
/// [`GroundError`] if the draft is invalid, an entry zone is unusable, or the
/// map frame exceeds [`MAP_MAX_BYTES`].
pub fn check(draft: &GroundDraft) -> Result<CheckReport, GroundError> {
    let (classes, elev) = draft.validate()?;
    let table = LocomotionTable::shipped();
    let total = usize::from(draft.cols) * usize::from(draft.rows);
    let mut cells: Vec<TerrainCell> = Vec::with_capacity(total);
    for (class, elev_cm) in classes.into_iter().zip(elev) {
        let move_cost = match table.cost(class.label(), "tracked", &[]) {
            Some(cost) => u16::try_from(cost).unwrap_or(u16::MAX),
            None => 0,
        };
        cells.push(TerrainCell {
            class,
            elev_cm,
            move_cost,
        });
    }
    let arena_entry = format!("tacsim.arena.{}", draft.id);
    let terrain = ArenaTerrain::priced(draft.cols, draft.rows, draft.cell_cm, cells, table)
        .map_err(|error| ArenaBuildError::Shape {
            arena: arena_entry.clone(),
            detail: error.to_string(),
        })?;
    let spots: Vec<ArenaKeySpot> = draft
        .key_spots
        .iter()
        .map(|s| ArenaKeySpot {
            name: smol_str::SmolStr::new(&s.name),
            x: s.x,
            y: s.y,
        })
        .collect();
    let map_bytes = arena_map_within_bound(&arena_entry, &terrain, &spots)?;
    let quadrants = usize::from(draft.cols.div_ceil(QUAD)) * usize::from(draft.rows.div_ceil(QUAD));
    Ok(CheckReport {
        id: draft.id.clone(),
        cols: draft.cols,
        rows: draft.rows,
        quadrants,
        map_bytes,
        map_max_bytes: MAP_MAX_BYTES,
    })
}

/// **The inverse (`pack → draft`)** (doc 30 §8 & §11 WG.1): reconstruct a
/// [`GroundDraft`] from a signed or unsigned atlas region pack and campaign-seed
/// arena pack.
///
/// # Errors
///
/// [`GroundError`] if the packs cannot be parsed or their tiles do not cover the
/// arena window.
pub fn from_packs(atlas_json: &str, arena_json: &str) -> Result<GroundDraft, GroundError> {
    let atlas_pack =
        mush_content::parse_pack(atlas_json).map_err(|e| GroundError::Pack(e.to_string()))?;
    let arena_pack =
        mush_content::parse_pack(arena_json).map_err(|e| GroundError::Pack(e.to_string()))?;

    let (arena_entry_id, arena_raw) = arena_pack
        .entries()
        .iter()
        .find(|(_, e)| e.kind().as_str() == "arena")
        .ok_or_else(|| GroundError::Pack("arena pack contains no `arena` entry".to_owned()))?;
    let arena_def: ArenaDef =
        serde_json::from_value(serde_json::Value::Object(arena_raw.def().clone()))
            .map_err(|e| GroundError::Json(format!("{arena_entry_id}: {e}")))?;

    let id = arena_entry_id
        .as_str()
        .strip_prefix("tacsim.arena.")
        .unwrap_or_else(|| arena_entry_id.as_str())
        .to_owned();

    let region_raw = atlas_pack.entries().get(&arena_def.region).ok_or_else(|| {
        GroundError::Pack(format!(
            "atlas pack has no region entry `{}`",
            arena_def.region
        ))
    })?;
    let region_def: RegionDef =
        serde_json::from_value(serde_json::Value::Object(region_raw.def().clone()))
            .map_err(|e| GroundError::Json(format!("{}: {e}", arena_def.region)))?;

    let total = usize::from(arena_def.cols) * usize::from(arena_def.rows);
    let mut classes = vec![TerrainClass::Grass; total];
    let mut elev = vec![0_i16; total];
    let mut covered = vec![false; total];
    let cell_m = i64::from(arena_def.cell_cm) / 100;

    for tile_id in &arena_def.tiles {
        let tile_raw = atlas_pack.entries().get(tile_id).ok_or_else(|| {
            GroundError::Pack(format!("atlas pack has no heightfield entry `{tile_id}`"))
        })?;
        let hf: HeightfieldDef =
            serde_json::from_value(serde_json::Value::Object(tile_raw.def().clone()))
                .map_err(|e| GroundError::Json(format!("{tile_id}: {e}")))?;
        let samples = hf.samples().map_err(GroundError::Pack)?;
        let class_grid = hf
            .class_grid()
            .map_err(GroundError::Pack)?
            .ok_or_else(|| GroundError::Pack(format!("{tile_id} has no classes grid")))?;

        let ox_m =
            mush_atlas::geom::cm(hf.origin.0).map_err(|e| GroundError::Pack(e.to_string()))? / 100;
        let oy_m =
            mush_atlas::geom::cm(hf.origin.1).map_err(|e| GroundError::Pack(e.to_string()))? / 100;
        let ox = u16::try_from((ox_m - arena_def.origin_m[0]) / cell_m.max(1))
            .map_err(|_| GroundError::Pack(format!("{tile_id} origin east offset out of range")))?;
        let oy = u16::try_from((oy_m - arena_def.origin_m[1]) / cell_m.max(1)).map_err(|_| {
            GroundError::Pack(format!("{tile_id} origin north offset out of range"))
        })?;

        for y in 0..hf.rows {
            for x in 0..hf.cols {
                let wx = ox + x;
                let wy = oy + y;
                if wx >= arena_def.cols || wy >= arena_def.rows {
                    continue;
                }
                let src_idx = usize::from(y) * usize::from(hf.cols) + usize::from(x);
                let dst_idx = usize::from(wy) * usize::from(arena_def.cols) + usize::from(wx);
                if let Some(slot) = elev.get_mut(dst_idx) {
                    *slot = samples.get(src_idx).copied().unwrap_or(0);
                }
                if let Some(slot) = classes.get_mut(dst_idx) {
                    *slot = class_grid
                        .get(src_idx)
                        .copied()
                        .flatten()
                        .unwrap_or(TerrainClass::Ground);
                }
                if let Some(slot) = covered.get_mut(dst_idx) {
                    *slot = true;
                }
            }
        }
    }

    let missing = covered.iter().filter(|&&c| !c).count();
    if missing > 0 {
        return Err(GroundError::ArenaBuild(ArenaBuildError::NotCovered {
            arena: arena_entry_id.to_string(),
            missing,
            total,
        }));
    }

    let seed = Some(region_def.background.seed);
    let draft = GroundDraft {
        schema: DRAFT_SCHEMA.to_owned(),
        id,
        label: arena_def.label,
        biome: arena_def.biome.unwrap_or_else(|| "temperate".to_owned()),
        body: region_def.body.unwrap_or_else(|| "luna".to_owned()),
        origin_m: arena_def.origin_m,
        cell_cm: arena_def.cell_cm,
        cols: arena_def.cols,
        rows: arena_def.rows,
        classes: encode_classes(arena_def.cols, arena_def.rows, &classes),
        elev: encode_elev(&elev),
        entry_zones: arena_def.entry_zones,
        key_spots: arena_def.key_spots,
        waypoints: arena_def.waypoints,
        modes: arena_def.modes,
        tanks: arena_def.tanks,
        atlas_version: atlas_pack.manifest().version(),
        arena_version: arena_pack.manifest().version(),
        provenance: GroundProvenance {
            source: GroundSource::Pack,
            version: format!("v{}", atlas_pack.manifest().version()),
            seed,
            inputs: vec![
                format!(
                    "{}@v{}:{}",
                    atlas_pack.manifest().id(),
                    atlas_pack.manifest().version(),
                    atlas_pack.manifest().digest().to_hex()
                ),
                format!(
                    "{}@v{}:{}",
                    arena_pack.manifest().id(),
                    arena_pack.manifest().version(),
                    arena_pack.manifest().digest().to_hex()
                ),
            ],
        },
        note: arena_def.note,
    };
    let _ = draft.validate()?;
    Ok(draft)
}

/// Build the canonical [`GroundDraft`] for the shipped `proving_ground` arena
/// (version 3 atlas relief, version 5 arena entry).
///
/// # Errors
///
/// [`GroundError::Invalid`] if any static canonical ID literal fails validation.
pub fn proving_ground_draft() -> Result<GroundDraft, GroundError> {
    let mut classes = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    let mut elev = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    for y in 0..SIDE {
        for x in 0..SIDE {
            classes.push(class_at(x, y));
            elev.push(elev_v3(x, y));
        }
    }
    let cid = |s: &str| {
        CanonicalId::new(s).map_err(|e| GroundError::Invalid {
            id: s.to_owned(),
            detail: e.to_string(),
        })
    };
    let node = |id: &str, x: u16, y: u16| -> Result<WaypointNode, GroundError> {
        Ok(WaypointNode { id: cid(id)?, x, y })
    };
    let edge =
        |a: &str, b: &str| -> Result<[CanonicalId; 2], GroundError> { Ok([cid(a)?, cid(b)?]) };
    let line_tank = cid("tacsim.unit.line_tank")?;
    let tank = |id: &str,
                name: &str,
                team: Team,
                x: u16,
                y: u16,
                heading_mdeg: u32|
     -> Result<TankDef, GroundError> {
        Ok(TankDef {
            id: cid(id)?,
            name: name.to_owned(),
            team,
            x,
            y,
            heading_mdeg,
            unit: line_tank.clone(),
        })
    };

    Ok(GroundDraft {
        schema: DRAFT_SCHEMA.to_owned(),
        id: "proving_ground".to_owned(),
        label: "The Proving Ground".to_owned(),
        biome: "temperate".to_owned(),
        body: "luna".to_owned(),
        origin_m: [ORIGIN_M, ORIGIN_M],
        cell_cm: CELL_CM,
        cols: SIDE,
        rows: SIDE,
        classes: encode_classes(SIDE, SIDE, &classes),
        elev: encode_elev(&elev),
        entry_zones: vec![
            EntryZoneDef {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 15,
                y1: 255,
            },
            EntryZoneDef {
                team: Team::Blue,
                x0: 240,
                y0: 0,
                x1: 255,
                y1: 255,
            },
        ],
        key_spots: vec![
            KeySpot {
                name: "The Ford".to_owned(),
                x: 64,
                y: 97,
            },
            KeySpot {
                name: "The Crossroads".to_owned(),
                x: 129,
                y: 97,
            },
            KeySpot {
                name: "North Barn".to_owned(),
                x: 106,
                y: 194,
            },
            KeySpot {
                name: "East Compound".to_owned(),
                x: 208,
                y: 148,
            },
            KeySpot {
                name: "The Bluff".to_owned(),
                x: 144,
                y: 111,
            },
        ],
        waypoints: WaypointGraph {
            nodes: vec![
                node("ew_008", 8, 97)?,
                node("ew_032", 32, 97)?,
                node("ew_056", 56, 97)?,
                node("ew_064", 64, 97)?,
                node("ew_080", 80, 97)?,
                node("ew_104", 104, 97)?,
                node("xroads", 129, 97)?,
                node("ew_152", 152, 97)?,
                node("ew_176", 176, 97)?,
                node("ew_200", 200, 97)?,
                node("ew_224", 224, 97)?,
                node("ew_248", 248, 97)?,
                node("ns_008", 129, 8)?,
                node("ns_032", 129, 32)?,
                node("ns_056", 129, 56)?,
                node("ns_080", 129, 80)?,
                node("ns_120", 129, 120)?,
                node("ns_144", 129, 144)?,
                node("ns_168", 129, 168)?,
                node("ns_192", 129, 192)?,
                node("ns_216", 129, 216)?,
                node("ns_248", 129, 248)?,
                node("barn", 106, 194)?,
                node("compound", 208, 148)?,
                node("muster_red", 6, 105)?,
                node("muster_blue", 249, 105)?,
            ],
            edges: vec![
                edge("ew_008", "ew_032")?,
                edge("ew_032", "ew_056")?,
                edge("ew_056", "ew_064")?,
                edge("ew_064", "ew_080")?,
                edge("ew_080", "ew_104")?,
                edge("ew_104", "xroads")?,
                edge("xroads", "ew_152")?,
                edge("ew_152", "ew_176")?,
                edge("ew_176", "ew_200")?,
                edge("ew_200", "ew_224")?,
                edge("ew_224", "ew_248")?,
                edge("ns_008", "ns_032")?,
                edge("ns_032", "ns_056")?,
                edge("ns_056", "ns_080")?,
                edge("ns_080", "xroads")?,
                edge("xroads", "ns_120")?,
                edge("ns_120", "ns_144")?,
                edge("ns_144", "ns_168")?,
                edge("ns_168", "ns_192")?,
                edge("ns_192", "ns_216")?,
                edge("ns_216", "ns_248")?,
                edge("ns_192", "barn")?,
                edge("ew_200", "compound")?,
                edge("ew_008", "muster_red")?,
                edge("ew_248", "muster_blue")?,
            ],
        },
        modes: vec![ArenaModeDef::Hold {
            spot: "The Crossroads".to_owned(),
            radius_cells: 6,
            hold_ms: 120_000,
        }],
        tanks: vec![
            tank("red_1", "Red Tank 1", Team::Red, 6, 97, 90_000)?,
            tank("red_2", "Red Tank 2", Team::Red, 6, 95, 90_000)?,
            tank("red_3", "Red Tank 3", Team::Red, 6, 99, 90_000)?,
            tank("red_4", "Red Tank 4", Team::Red, 6, 93, 90_000)?,
            tank("blue_1", "Blue Tank 1", Team::Blue, 249, 97, 270_000)?,
            tank("blue_2", "Blue Tank 2", Team::Blue, 249, 95, 270_000)?,
            tank("blue_3", "Blue Tank 3", Team::Blue, 249, 99, 270_000)?,
            tank("blue_4", "Blue Tank 4", Team::Blue, 249, 93, 270_000)?,
            tank("red_5", "Red Tank 5", Team::Red, 6, 101, 90_000)?,
            tank("red_6", "Red Tank 6", Team::Red, 4, 94, 90_000)?,
            tank("blue_5", "Blue Tank 5", Team::Blue, 249, 101, 270_000)?,
            tank("blue_6", "Blue Tank 6", Team::Blue, 251, 94, 270_000)?,
        ],
        atlas_version: 3,
        arena_version: 5,
        provenance: GroundProvenance {
            source: GroundSource::Generator,
            version: "v3".to_owned(),
            seed: Some(2350),
            inputs: Vec::new(),
        },
        note: "D-16-5 fixes the window: 256 by 256 cells of 1,000 cm at 20,000 m, clear of the lot and the harness. Entry zones are sixteen cells deep. The key spots are the ford, the crossroads, the two compounds and the bluff. The waypoint graph is two roads and four spurs, so Patrol's route is not a straight line. VERSION 2 (16b cut F.4) moves the muster onto the ford: the river (columns 62 to 66) stopped a tank at deep water. The ford is rows 92 to 102; slot 1 stands on the east-west road at row 97 and slots 2 to 4 at 95, 99 and 93, so every tank drives straight across. The muster nodes moved to row 105. VERSION 3 (16b cut Z.3) adds slots 5 and 6 a side: row 101 in the muster column and row 94 one behind it, inside the ford. VERSION 4 (16b cut O.1) declares the modes the ground supports: hold at The Crossroads, six cells, 120 s. VERSION 5 (16b cut AE.1): The Bluff, a grass mesa 7 m up at 141..147 by 105..118, ramp on its north side; its faces refuse tracks and wheels, not legs; the fifth spot, 144,111.".to_owned(),
    })
}

/// **The generator** (doc 30 §4 & §9, D-30-9): generate a [`GroundDraft`] for a
/// named preset (`"proving_ground"` in WG.1; biome presets join in WG.3/WG.5).
///
/// # Errors
///
/// [`GroundError::Invalid`] if `preset` or `biome` is unknown.
pub fn generate(preset: &str, seed: u64, biome: Option<&str>) -> Result<GroundDraft, GroundError> {
    if let Some(b) = biome
        && !BIOMES.contains(&b)
    {
        return Err(GroundError::Invalid {
            id: preset.to_owned(),
            detail: format!("biome must be one of {BIOMES:?}, got `{b}`"),
        });
    }
    match preset {
        "proving_ground" => {
            if seed != 2350 {
                return Err(GroundError::Invalid {
                    id: preset.to_owned(),
                    detail: format!(
                        "`proving_ground` is the fixed V3 landscape (seed 2350); got `--seed {seed}`"
                    ),
                });
            }
            if let Some(b) = biome
                && b != "temperate"
            {
                return Err(GroundError::Invalid {
                    id: preset.to_owned(),
                    detail: format!(
                        "`proving_ground` is the fixed temperate V3 landscape; got `--biome {b}`"
                    ),
                });
            }
            proving_ground_draft()
        }
        other => Err(GroundError::Invalid {
            id: other.to_owned(),
            detail: format!("unknown generator preset `{other}` (expected `proving_ground`)"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn proving_ground_draft_builds_shipped_packs_byte_for_byte() -> Result<(), Box<dyn Error>> {
        let draft = generate("proving_ground", 2350, None)?;
        let built = build(&draft)?;
        check_eq!(
            built.atlas_json.replace("\r\n", "\n"),
            content::PROVING_GROUND_ATLAS_JSON.replace("\r\n", "\n")
        );
        check_eq!(
            built.arena_json.replace("\r\n", "\n"),
            content::PROVING_GROUND_JSON.replace("\r\n", "\n")
        );
        Ok(())
    }

    #[test]
    fn inverse_from_packs_round_trips_proving_ground() -> Result<(), Box<dyn Error>> {
        let draft = from_packs(
            content::PROVING_GROUND_ATLAS_JSON,
            content::PROVING_GROUND_JSON,
        )?;
        check_eq!(draft.provenance.source, GroundSource::Pack);
        check_eq!(draft.provenance.inputs.len(), 2);
        let expected = proving_ground_draft()?;
        let mut normalized = draft.clone();
        normalized.provenance = expected.provenance.clone();
        check_eq!(normalized, expected);
        let built = build(&draft)?;
        check_eq!(
            built.atlas_json.replace("\r\n", "\n"),
            content::PROVING_GROUND_ATLAS_JSON.replace("\r\n", "\n")
        );
        check_eq!(
            built.arena_json.replace("\r\n", "\n"),
            content::PROVING_GROUND_JSON.replace("\r\n", "\n")
        );
        Ok(())
    }

    #[test]
    fn draft_validation_refuses_void_cells_and_missing_extractor_inputs()
    -> Result<(), Box<dyn Error>> {
        let mut draft = proving_ground_draft()?;
        draft.provenance.source = GroundSource::Extractor;
        draft.provenance.inputs.clear();
        check!(draft.validate().is_err());

        draft.provenance.source = GroundSource::Pack;
        check!(draft.validate().is_err());

        check!(generate("proving_ground", 7, None).is_err());
        check!(generate("proving_ground", 2350, Some("mars")).is_err());

        let mut bad_void = proving_ground_draft()?;
        let void_grid = vec![TerrainClass::Void; usize::from(SIDE) * usize::from(SIDE)];
        bad_void.classes = encode_classes(SIDE, SIDE, &void_grid);
        check!(bad_void.validate().is_err());
        Ok(())
    }
}
