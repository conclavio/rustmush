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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;

use mush_atlas::TerrainClass;
use mush_atlas::cells::{base64_decode, base64_encode};
use mush_atlas::def::{HeightfieldDef, RegionDef};
use mush_atlas::physics::{LocomotionTable, StepKind};
use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};

use crate::arena::path::{PathScratch, path_with};
use crate::arena::slope::steepness_pct;
use crate::arena::state::{ArenaTerrain, CostRow, KeySpot as ArenaKeySpot, Team, TerrainCell};
use crate::definitions::arena::{
    ArenaBuildError, ArenaDef, ArenaModeDef, BIOMES, EntryZoneDef, KeySpot, MAX_ARENA_TILES,
    MAX_WAYPOINT_EDGES, MAX_WAYPOINT_NODES, TankDef, WaypointGraph, WaypointNode,
    arena_map_within_bound,
};
use crate::definitions::content;
use crate::match_::cockpit::MAP_MAX_BYTES;

/// Default minimum connected-component size for [`despeckle`] (doc 30 §5).
pub const DEFAULT_SPECKLE_MIN_CELLS: usize = 3;

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
    /// Draft failed §5 grade, passability, or reachability checks.
    #[error("ground draft `{id}` failed check: {}", failures.join("; "))]
    CheckFailed {
        /// Draft slug.
        id: String,
        /// Failure descriptions.
        failures: Vec<String>,
    },
    /// Live staging (`@ground/stage`) refusal.
    #[error("ground stage failed: {0}")]
    Stage(String),
}

/// Output of [`build`]: the post-route/post-repair [`GroundDraft`], the two
/// signed JSON packs, and their manifests' digests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltPacks {
    /// The [`GroundDraft`] the packs were built from (including any synthesized
    /// waypoint graph from [`route`], any repair changes from [`relax_slopes`] /
    /// [`despeckle`], and any generated §5 check report in `note`).
    pub draft: GroundDraft,
    /// Signed atlas region pack JSON (`tacsim.<id>`).
    pub atlas_json: String,
    /// Computed digest of `atlas_json`.
    pub atlas_digest: DefinitionDigest,
    /// Signed campaign-seed arena pack JSON (`tacsim.pack.<id>`).
    pub arena_json: String,
    /// Computed digest of `arena_json`.
    pub arena_digest: DefinitionDigest,
}

/// Outcome of [`stage_in_world`] (`@ground/stage`; doc 30 §8, `Q-30-5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedGround {
    /// Arena short slug (e.g. `"proving_ground"` or `"mars_canyon"`).
    pub id: String,
    /// Atlas pack ID (`tacsim.<id>`).
    pub atlas_pack_id: CanonicalId,
    /// Atlas pack version shelved (and applied when `applied` is `true`).
    pub atlas_version: u32,
    /// Atlas pack SHA-256 digest (hex).
    pub atlas_digest: String,
    /// Campaign-seed arena pack ID (`tacsim.pack.<id>`).
    pub arena_pack_id: CanonicalId,
    /// Campaign-seed arena pack version shelved (and applied when `applied` is `true`).
    pub arena_version: u32,
    /// Campaign-seed arena pack SHA-256 digest (hex).
    pub arena_digest: String,
    /// Grid dimensions `(cols, rows)`.
    pub dims: (u16, u16),
    /// Number of `<= 128 x 128` heightfield quadrant entries.
    pub quadrants: usize,
    /// Whether the two packs were also applied through the content latch (`@ground/stage/apply`).
    pub applied: bool,
    /// `@atlas/pack plan` fingerprint for the shelved atlas pack (when not yet applied).
    pub atlas_plan_fingerprint: Option<String>,
    /// `@atlas/pack plan` fingerprint for the shelved arena pack (when not yet applied).
    pub arena_plan_fingerprint: Option<String>,
}

/// Output of [`check`]: wire size, grid shape, grade, and passability report
/// (doc 30 §5 & §11 WG.1 / WG.3).
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
    /// Percentage of land cells reachable by `tracked` (limit 45 %) from both entry zones.
    pub tracked_passable_pct: u32,
    /// Percentage of land cells reachable by `wheeled` (limit 25 %) from both entry zones.
    pub wheeled_passable_pct: u32,
    /// Percentage of land cells reachable by `legged` (limit 70 %) from both entry zones.
    pub legged_passable_pct: u32,
    /// Whether the two entry zones can reach each other under `tracked`.
    pub zones_reachable_tracked: bool,
    /// Whether the two entry zones can reach each other under `wheeled`.
    pub zones_reachable_wheeled: bool,
    /// Whether the two entry zones can reach each other under `legged`.
    pub zones_reachable_legged: bool,
    /// Whether every key spot is reachable by `wheeled` from at least one entry zone.
    pub key_spots_wheeled_reachable: bool,
    /// Names of any key spots not reachable by `wheeled`.
    pub unreachable_key_spots: Vec<String>,
    /// Whether every waypoint node is reachable by `tracked` from the entry zones.
    pub waypoints_reachable: bool,
    /// IDs of any waypoint nodes not reachable by `tracked`.
    pub unreachable_waypoints: Vec<String>,
    /// Steepest orthogonal passable grade across the grid, in per cent.
    pub max_grade_pct: u32,
    /// Steepest orthogonal grade between adjacent `road` or `paved` cells, in per cent.
    pub max_road_grade_pct: u32,
    /// Maximum elevation step (`cm`) between orthogonal neighbours inside entry zones.
    pub max_entry_zone_step_cm: u32,
    /// Whether all §5 acceptance checks passed.
    pub passed: bool,
    /// Human-readable descriptions of any §5 check failures.
    pub failures: Vec<String>,
}

impl CheckReport {
    /// Format this §5 grade and passability report for inclusion in an arena
    /// pack's `note` field (doc 30 §5).
    #[must_use]
    pub fn summary_note(&self) -> String {
        format!(
            "ground-pack check ({cols}x{rows}, {quads} tiles, map {bytes}/{max} B): \
             passable tracked {t}% / wheeled {w}% / legged {l}%; \
             zones tracked={zt} wheeled={zw} legged={zl}; \
             key_spots_wheeled={ks}; waypoints={wp}; \
             max_grade={mg}% road_grade={rg}% entry_step={es}cm; passed={p}.",
            cols = self.cols,
            rows = self.rows,
            quads = self.quadrants,
            bytes = self.map_bytes,
            max = self.map_max_bytes,
            t = self.tracked_passable_pct,
            w = self.wheeled_passable_pct,
            l = self.legged_passable_pct,
            zt = self.zones_reachable_tracked,
            zw = self.zones_reachable_wheeled,
            zl = self.zones_reachable_legged,
            ks = self.key_spots_wheeled_reachable,
            wp = self.waypoints_reachable,
            mg = self.max_grade_pct,
            rg = self.max_road_grade_pct,
            es = self.max_entry_zone_step_cm,
            p = self.passed,
        )
    }
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
        "temperate",
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
    biome: &str,
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

    let mut region_def = serde_json::json!({
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
    });
    // Omit `"biome": "temperate"` on the wire so the shipped `proving_ground`
    // atlas pack reproduces byte-for-byte; `RegionRec::biome_str()` and
    // `from_packs` both treat an absent `biome` key as `"temperate"` (doc 30 §6).
    if biome != "temperate"
        && let Some(obj) = region_def.as_object_mut()
    {
        obj.insert("biome".to_owned(), serde_json::json!(biome));
    }

    let mut entries = serde_json::Map::new();
    entries.insert(
        region_id.clone(),
        serde_json::json!({
            "kind": "region",
            "def": region_def
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
/// By default (doc 30 §5), [`relax_slopes`] and [`despeckle`] run only when
/// `draft.provenance.source == GroundSource::Extractor`, [`route`] runs when
/// `draft.waypoints.nodes.is_empty()`, and the build is refused when [`check`]
/// reports any §5 failure. See [`build_with_options`] for `--repair` / `--force`.
///
/// # Errors
///
/// [`GroundError`] if validation, [`check`], or pack signing fails.
pub fn build(draft: &GroundDraft) -> Result<BuiltPacks, GroundError> {
    build_with_options(draft, false, false)
}

/// **The writer with explicit `--repair` and `--force` flags** (doc 30 §5, WG.3).
///
/// - When `repair` is `true` or `draft.provenance.source == GroundSource::Extractor`,
///   runs [`relax_slopes`] and [`despeckle`] (`DEFAULT_SPECKLE_MIN_CELLS`).
/// - When `draft.waypoints.nodes.is_empty()`, runs [`route`] to synthesize a
///   patrol graph over the tracked cost grid.
/// - Runs [`check`] and refuses the build with [`GroundError::CheckFailed`]
///   when `!report.passed && !force`.
///
/// # Errors
///
/// [`GroundError`] if validation, [`check`] (unless `force`), or pack signing fails.
pub fn build_with_options(
    draft: &GroundDraft,
    repair: bool,
    force: bool,
) -> Result<BuiltPacks, GroundError> {
    let mut working = draft.clone();
    if repair || working.provenance.source == GroundSource::Extractor {
        let _ = despeckle(&mut working, DEFAULT_SPECKLE_MIN_CELLS)?;
        let _ = relax_slopes(&mut working)?;
    }
    if working.waypoints.nodes.is_empty() {
        route(&mut working)?;
    }
    let (classes, elev) = working.validate()?;
    let report = check(&working)?;
    if !report.passed && !force {
        return Err(GroundError::CheckFailed {
            id: working.id,
            failures: report.failures,
        });
    }
    if working.note.is_empty() {
        working.note = report.summary_note();
    }
    let arena_def = working.to_arena_def()?;
    let bg_seed = working.provenance.seed.unwrap_or(2350);

    let (atlas_json, atlas_digest) = build_atlas_pack_json(
        &working.id,
        &working.label,
        &working.biome,
        &working.body,
        working.origin_m,
        working.cell_cm,
        working.cols,
        working.rows,
        working.atlas_version,
        bg_seed,
        &classes,
        &elev,
    )?;
    let (arena_json, arena_digest) =
        format_arena_pack_json(&working.id, working.arena_version, &arena_def)?;

    Ok(BuiltPacks {
        draft: working,
        atlas_json,
        atlas_digest,
        arena_json,
        arena_digest,
    })
}

fn build_priced_terrain(
    draft: &GroundDraft,
    classes: &[TerrainClass],
    elev: &[i16],
) -> Result<ArenaTerrain, GroundError> {
    let table = LocomotionTable::shipped();
    let total = usize::from(draft.cols) * usize::from(draft.rows);
    let mut cells: Vec<TerrainCell> = Vec::with_capacity(total);
    for (&class, &elev_cm) in classes.iter().zip(elev.iter()) {
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
            arena: arena_entry,
            detail: error.to_string(),
        })?
        .with_biome(draft.biome.as_str());
    Ok(terrain)
}

fn zone_seed_cells(
    terrain: &ArenaTerrain,
    zone: &EntryZoneDef,
    tanks: &[TankDef],
    row: CostRow,
) -> Vec<(u16, u16)> {
    let mut seeds: Vec<(u16, u16)> = tanks
        .iter()
        .filter(|t| {
            t.team == zone.team
                && t.x >= zone.x0
                && t.x <= zone.x1
                && t.y >= zone.y0
                && t.y <= zone.y1
                && terrain.passable_for(row, t.x, t.y)
        })
        .map(|t| (t.x, t.y))
        .collect();
    let cx = (zone.x0 + zone.x1) / 2;
    let cy = (zone.y0 + zone.y1) / 2;
    if terrain.passable_for(row, cx, cy) && !seeds.contains(&(cx, cy)) {
        seeds.push((cx, cy));
    }
    if seeds.is_empty() {
        let mut best: Option<(u32, (u16, u16))> = None;
        for y in zone.y0..=zone.y1 {
            for x in zone.x0..=zone.x1 {
                if terrain.passable_for(row, x, y) {
                    let dist = u32::from(x.abs_diff(cx)) + u32::from(y.abs_diff(cy));
                    if best.is_none_or(|(d, _)| dist < d) {
                        best = Some((dist, (x, y)));
                    }
                }
            }
        }
        if let Some((_, cell)) = best {
            seeds.push(cell);
        }
    }
    seeds
}

fn flood_from_seeds(
    terrain: &ArenaTerrain,
    row: CostRow,
    limit: u32,
    seeds: &[(u16, u16)],
) -> BTreeSet<(u16, u16)> {
    let cell_cm = u32::from(terrain.cell_cm());
    let mut seen: BTreeSet<(u16, u16)> = seeds.iter().copied().collect();
    let mut queue: VecDeque<(u16, u16)> = seeds.iter().copied().collect();
    while let Some((x, y)) = queue.pop_front() {
        let Some(here) = terrain.get(x, y) else {
            continue;
        };
        for (dx, dy) in [(1_i32, 0_i32), (-1, 0), (0, 1), (0, -1)] {
            let (Some(nx), Some(ny)) = (
                u16::try_from(i32::from(x) + dx).ok(),
                u16::try_from(i32::from(y) + dy).ok(),
            ) else {
                continue;
            };
            if seen.contains(&(nx, ny)) || !terrain.passable_for(row, nx, ny) {
                continue;
            }
            let Some(there) = terrain.get(nx, ny) else {
                continue;
            };
            if steepness_pct(here, there, StepKind::Orthogonal, cell_cm) > limit {
                continue;
            }
            seen.insert((nx, ny));
            queue.push_back((nx, ny));
        }
    }
    seen
}

/// **The check** (doc 30 §5 & §11 WG.1 / WG.3): validate `draft`, price its cells
/// from the shipped locomotion table, verify that its widest `arena.map` frame
/// fits within [`MAP_MAX_BYTES`] (`256 KiB`), and compute the §5 grade and
/// passability report over [`crate::arena::slope`] and [`crate::arena::path`].
///
/// # Errors
///
/// [`GroundError`] if the draft is structurally invalid or the map frame exceeds
/// [`MAP_MAX_BYTES`].
pub fn check(draft: &GroundDraft) -> Result<CheckReport, GroundError> {
    let (classes, elev) = draft.validate()?;
    let terrain = build_priced_terrain(draft, &classes, &elev)?;
    let arena_entry = format!("tacsim.arena.{}", draft.id);
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

    let table = LocomotionTable::shipped();
    let tracked_limit = table.max_slope_pct("tracked", &[]).unwrap_or(45);
    let wheeled_limit = table.max_slope_pct("wheeled", &[]).unwrap_or(25);
    let legged_limit = table.max_slope_pct("legged", &[]).unwrap_or(70);

    let is_land = |c: TerrainClass| {
        !matches!(
            c,
            TerrainClass::Void | TerrainClass::Water | TerrainClass::WaterDeep | TerrainClass::Wall
        )
    };
    let land_total = classes.iter().copied().filter(|&c| is_land(c)).count();

    let mut tracked_pct_min = 100_u32;
    let mut wheeled_pct_min = 100_u32;
    let mut legged_pct_min = 100_u32;

    let mut tracked_union: BTreeSet<(u16, u16)> = BTreeSet::new();
    let mut wheeled_union: BTreeSet<(u16, u16)> = BTreeSet::new();
    let mut zone_seeds_tracked: Vec<Option<(u16, u16)>> = Vec::new();
    let mut zone_seeds_wheeled: Vec<Option<(u16, u16)>> = Vec::new();
    let mut zone_seeds_legged: Vec<Option<(u16, u16)>> = Vec::new();

    for zone in &draft.entry_zones {
        let t_seeds = zone_seed_cells(&terrain, zone, &draft.tanks, CostRow::Tracked);
        let w_seeds = zone_seed_cells(&terrain, zone, &draft.tanks, CostRow::Wheeled);
        let l_seeds = zone_seed_cells(&terrain, zone, &draft.tanks, CostRow::Legged);

        zone_seeds_tracked.push(t_seeds.first().copied());
        zone_seeds_wheeled.push(w_seeds.first().copied());
        zone_seeds_legged.push(l_seeds.first().copied());

        let t_flood = flood_from_seeds(&terrain, CostRow::Tracked, tracked_limit, &t_seeds);
        let w_flood = flood_from_seeds(&terrain, CostRow::Wheeled, wheeled_limit, &w_seeds);
        let l_flood = flood_from_seeds(&terrain, CostRow::Legged, legged_limit, &l_seeds);

        let count_land = |set: &BTreeSet<(u16, u16)>| -> u32 {
            if land_total == 0 {
                return 100;
            }
            let reached = set
                .iter()
                .filter(|&&(x, y)| terrain.get(x, y).is_some_and(|c| is_land(c.class)))
                .count();
            u32::try_from((reached * 100) / land_total).unwrap_or(100)
        };
        tracked_pct_min = tracked_pct_min.min(count_land(&t_flood));
        wheeled_pct_min = wheeled_pct_min.min(count_land(&w_flood));
        legged_pct_min = legged_pct_min.min(count_land(&l_flood));

        tracked_union.extend(t_flood);
        wheeled_union.extend(w_flood);
    }

    let mut scratch = PathScratch::default();
    let mutual =
        |seeds: &[Option<(u16, u16)>], row: CostRow, limit: u32, scratch: &mut PathScratch| {
            let (Some(Some(a)), Some(Some(b))) = (seeds.first(), seeds.get(1)) else {
                return false;
            };
            path_with(
                &terrain,
                scratch,
                *a,
                *b,
                u32::MAX,
                terrain.min_cost(row),
                Some(limit),
                row,
                &[],
            )
            .is_ok()
        };
    let zones_reachable_tracked = mutual(
        &zone_seeds_tracked,
        CostRow::Tracked,
        tracked_limit,
        &mut scratch,
    );
    let zones_reachable_wheeled = mutual(
        &zone_seeds_wheeled,
        CostRow::Wheeled,
        wheeled_limit,
        &mut scratch,
    );
    let zones_reachable_legged = mutual(
        &zone_seeds_legged,
        CostRow::Legged,
        legged_limit,
        &mut scratch,
    );

    let mut unreachable_key_spots = Vec::new();
    for spot in &draft.key_spots {
        if !wheeled_union.contains(&(spot.x, spot.y)) {
            unreachable_key_spots.push(spot.name.clone());
        }
    }
    let key_spots_wheeled_reachable = unreachable_key_spots.is_empty();

    let mut unreachable_waypoints = Vec::new();
    for node in &draft.waypoints.nodes {
        if !tracked_union.contains(&(node.x, node.y)) {
            unreachable_waypoints.push(node.id.to_string());
        }
    }
    let waypoints_reachable = unreachable_waypoints.is_empty();

    let cell_cm = u32::from(draft.cell_cm);
    let mut max_grade_pct = 0_u32;
    let mut max_road_grade_pct = 0_u32;
    for y in 0..draft.rows {
        for x in 0..draft.cols {
            let Some(here) = terrain.get(x, y) else {
                continue;
            };
            let here_road = matches!(here.class, TerrainClass::Road | TerrainClass::Paved);
            for (nx, ny) in [(x.saturating_add(1), y), (x, y.saturating_add(1))] {
                if nx >= draft.cols || ny >= draft.rows || (nx == x && ny == y) {
                    continue;
                }
                let Some(there) = terrain.get(nx, ny) else {
                    continue;
                };
                let steepness = steepness_pct(here, there, StepKind::Orthogonal, cell_cm);
                if here.passable() && there.passable() {
                    max_grade_pct = max_grade_pct.max(steepness);
                }
                if here_road && matches!(there.class, TerrainClass::Road | TerrainClass::Paved) {
                    max_road_grade_pct = max_road_grade_pct.max(steepness);
                }
            }
        }
    }

    let mut max_entry_zone_step_cm = 0_u32;
    for zone in &draft.entry_zones {
        let seeds = zone_seed_cells(&terrain, zone, &draft.tanks, CostRow::Tracked);
        for (sx, sy) in seeds {
            let Some(here) = terrain.get(sx, sy) else {
                continue;
            };
            for (dx, dy) in [(1_i32, 0_i32), (-1, 0), (0, 1), (0, -1)] {
                let (Some(nx), Some(ny)) = (
                    u16::try_from(i32::from(sx) + dx).ok(),
                    u16::try_from(i32::from(sy) + dy).ok(),
                ) else {
                    continue;
                };
                if nx < zone.x0 || nx > zone.x1 || ny < zone.y0 || ny > zone.y1 {
                    continue;
                }
                if let Some(there) = terrain.get(nx, ny) {
                    let step = u32::from(here.elev_cm.abs_diff(there.elev_cm));
                    max_entry_zone_step_cm = max_entry_zone_step_cm.max(step);
                }
            }
        }
    }

    let mut failures = Vec::new();
    if tracked_pct_min < 90 {
        failures.push(format!(
            "tracked reachability is {tracked_pct_min}% (need >= 90%)"
        ));
    }
    if legged_pct_min < 95 {
        failures.push(format!(
            "legged reachability is {legged_pct_min}% (need >= 95%)"
        ));
    }
    if !zones_reachable_tracked {
        failures.push("entry zones are not mutually reachable by tracked".to_owned());
    }
    if !zones_reachable_legged {
        failures.push("entry zones are not mutually reachable by legged".to_owned());
    }
    if !zones_reachable_wheeled {
        failures.push("entry zones are not mutually reachable by wheeled".to_owned());
    }
    if !key_spots_wheeled_reachable {
        failures.push(format!(
            "key spots not reachable by wheeled: {}",
            unreachable_key_spots.join(", ")
        ));
    }
    if !waypoints_reachable {
        failures.push(format!(
            "waypoints not reachable by tracked: {}",
            unreachable_waypoints.join(", ")
        ));
    }
    if max_road_grade_pct > 20 {
        failures.push(format!(
            "max road grade is {max_road_grade_pct}% (need <= 20%)"
        ));
    }
    let step_limit_cm = 100_u32;
    if max_entry_zone_step_cm > step_limit_cm {
        failures.push(format!(
            "entry zone step is {max_entry_zone_step_cm} cm (need <= {step_limit_cm} cm)"
        ));
    }

    let passed = failures.is_empty();
    Ok(CheckReport {
        id: draft.id.clone(),
        cols: draft.cols,
        rows: draft.rows,
        quadrants,
        map_bytes,
        map_max_bytes: MAP_MAX_BYTES,
        tracked_passable_pct: tracked_pct_min,
        wheeled_passable_pct: wheeled_pct_min,
        legged_passable_pct: legged_pct_min,
        zones_reachable_tracked,
        zones_reachable_wheeled,
        zones_reachable_legged,
        key_spots_wheeled_reachable,
        unreachable_key_spots,
        waypoints_reachable,
        unreachable_waypoints,
        max_grade_pct,
        max_road_grade_pct,
        max_entry_zone_step_cm,
        passed,
        failures,
    })
}

/// **Slope relaxation repair pass** (doc 30 §5): iteratively pull the higher cell
/// of any steep orthogonal edge down toward the drive limit (`20 %` between
/// `road` / `paved` cells, `45 %` on general passable ground), never modifying
/// entry-zone cells. Returns the number of cells whose elevation changed.
///
/// # Errors
///
/// [`GroundError`] if `draft.classes` or `draft.elev` fails to decode.
pub fn relax_slopes(draft: &mut GroundDraft) -> Result<usize, GroundError> {
    let classes = decode_classes(&draft.id, draft.cols, draft.rows, &draft.classes)?;
    let orig_elev = decode_elev(&draft.id, draft.cols, draft.rows, &draft.elev)?;
    let mut elev = orig_elev.clone();

    let w = usize::from(draft.cols);
    let h = usize::from(draft.rows);
    let mut in_zone = vec![false; w * h];
    for z in &draft.entry_zones {
        for y in z.y0..=z.y1.min(draft.rows.saturating_sub(1)) {
            for x in z.x0..=z.x1.min(draft.cols.saturating_sub(1)) {
                if let Some(slot) = in_zone.get_mut(usize::from(y) * w + usize::from(x)) {
                    *slot = true;
                }
            }
        }
    }

    let is_passable = |c: TerrainClass| {
        !matches!(
            c,
            TerrainClass::Void
                | TerrainClass::Wall
                | TerrainClass::WaterDeep
                | TerrainClass::WoodsHeavy
        )
    };
    let is_road = |c: TerrainClass| {
        matches!(
            c,
            TerrainClass::Road
                | TerrainClass::Paved
                | TerrainClass::Water
                | TerrainClass::Floor
                | TerrainClass::Door
        )
    };
    let table = LocomotionTable::shipped();
    let tracked_limit =
        i32::try_from(table.max_slope_pct("tracked", &[]).unwrap_or(45)).unwrap_or(45);
    let wheeled_limit =
        i32::try_from(table.max_slope_pct("wheeled", &[]).unwrap_or(25)).unwrap_or(25);
    let road_limit = wheeled_limit.saturating_sub(5);
    let road_max_rise = (i32::from(draft.cell_cm) * road_limit) / 100;
    let ground_max_rise = (i32::from(draft.cell_cm) * tracked_limit) / 100;

    let relax_edge = |elev: &mut [i16], a: usize, b: usize, max_rise: i32| -> bool {
        let mut ea = i32::from(elev.get(a).copied().unwrap_or(0));
        let mut eb = i32::from(elev.get(b).copied().unwrap_or(0));
        if (ea - eb).abs() <= max_rise {
            return false;
        }
        let za = in_zone.get(a).copied().unwrap_or(false);
        let zb = in_zone.get(b).copied().unwrap_or(false);
        if za && zb {
            return false;
        }
        let mut changed = false;
        if ea > eb {
            if ea > 0 && !za {
                let next_a = (eb + max_rise)
                    .max(0)
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_a != ea {
                    ea = next_a;
                    if let Some(slot) = elev.get_mut(a) {
                        *slot = i16::try_from(ea).unwrap_or(0);
                    }
                    changed = true;
                }
            }
            if ea - eb > max_rise && !zb {
                let next_b = (ea - max_rise).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_b != eb {
                    eb = next_b;
                    if let Some(slot) = elev.get_mut(b) {
                        *slot = i16::try_from(eb).unwrap_or(0);
                    }
                    changed = true;
                }
            }
            if ea - eb > max_rise && !za {
                let next_a = (eb + max_rise).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_a != ea {
                    if let Some(slot) = elev.get_mut(a) {
                        *slot = i16::try_from(next_a).unwrap_or(0);
                    }
                    changed = true;
                }
            }
        } else {
            if eb > 0 && !zb {
                let next_b = (ea + max_rise)
                    .max(0)
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_b != eb {
                    eb = next_b;
                    if let Some(slot) = elev.get_mut(b) {
                        *slot = i16::try_from(eb).unwrap_or(0);
                    }
                    changed = true;
                }
            }
            if eb - ea > max_rise && !za {
                let next_a = (eb - max_rise).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_a != ea {
                    ea = next_a;
                    if let Some(slot) = elev.get_mut(a) {
                        *slot = i16::try_from(ea).unwrap_or(0);
                    }
                    changed = true;
                }
            }
            if eb - ea > max_rise && !zb {
                let next_b = (ea + max_rise).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if next_b != eb {
                    if let Some(slot) = elev.get_mut(b) {
                        *slot = i16::try_from(next_b).unwrap_or(0);
                    }
                    changed = true;
                }
            }
        }
        changed
    };

    for _ in 0..64 {
        let mut changed_iter = false;
        for y in 0..h {
            for x in 0..w {
                let a = y * w + x;
                let ca = classes.get(a).copied().unwrap_or(TerrainClass::Void);
                if !is_passable(ca) {
                    continue;
                }
                for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                    if nx >= w || ny >= h {
                        continue;
                    }
                    let b = ny * w + nx;
                    let cb = classes.get(b).copied().unwrap_or(TerrainClass::Void);
                    if !is_passable(cb) {
                        continue;
                    }
                    let max_rise = if is_road(ca) || is_road(cb) {
                        road_max_rise
                    } else {
                        ground_max_rise
                    };
                    if relax_edge(&mut elev, a, b, max_rise) {
                        changed_iter = true;
                    }
                }
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let a = y * w + x;
                let ca = classes.get(a).copied().unwrap_or(TerrainClass::Void);
                if !is_passable(ca) {
                    continue;
                }
                for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                    if nx >= w || ny >= h {
                        continue;
                    }
                    let b = ny * w + nx;
                    let cb = classes.get(b).copied().unwrap_or(TerrainClass::Void);
                    if !is_passable(cb) {
                        continue;
                    }
                    let max_rise = if is_road(ca) || is_road(cb) {
                        road_max_rise
                    } else {
                        ground_max_rise
                    };
                    if relax_edge(&mut elev, a, b, max_rise) {
                        changed_iter = true;
                    }
                }
            }
        }
        if !changed_iter {
            break;
        }
    }

    let modified = orig_elev
        .iter()
        .zip(elev.iter())
        .filter(|(a, b)| a != b)
        .count();
    if modified > 0 {
        draft.elev = encode_elev(&elev);
    }
    Ok(modified)
}

/// **Speckle filter repair pass** (doc 30 §5): replace connected 4-neighbour
/// class regions smaller than `min_region` (default [`DEFAULT_SPECKLE_MIN_CELLS`],
/// 3) with their majority 4-neighbour class.
///
/// Never removes a built class (`paved`, `road`, `floor`, `door`, `platform`),
/// `wall`, or any cell holding a [`KeySpot`].
/// Returns the number of cells whose class changed.
///
/// # Errors
///
/// [`GroundError`] if `draft.classes` fails to decode.
pub fn despeckle(draft: &mut GroundDraft, min_region: usize) -> Result<usize, GroundError> {
    let orig_classes = decode_classes(&draft.id, draft.cols, draft.rows, &draft.classes)?;
    if min_region <= 1 {
        return Ok(0);
    }
    let mut classes = orig_classes.clone();
    let w = usize::from(draft.cols);
    let h = usize::from(draft.rows);
    let spot_cells: BTreeSet<(usize, usize)> = draft
        .key_spots
        .iter()
        .map(|s| (usize::from(s.x), usize::from(s.y)))
        .collect();
    let is_exempt_class = |c: TerrainClass| {
        matches!(
            c,
            TerrainClass::Wall
                | TerrainClass::Paved
                | TerrainClass::Road
                | TerrainClass::Floor
                | TerrainClass::Door
                | TerrainClass::Platform
        )
    };

    let mut visited = vec![false; w * h];
    let mut component = Vec::new();
    let mut queue = VecDeque::new();

    for _ in 0..8 {
        let mut changed_iter = false;
        visited.fill(false);
        for y in 0..h {
            for x in 0..w {
                let start = y * w + x;
                if visited.get(start).copied().unwrap_or(true) {
                    continue;
                }
                let target_class = classes.get(start).copied().unwrap_or(TerrainClass::Ground);
                if let Some(v) = visited.get_mut(start) {
                    *v = true;
                }
                if is_exempt_class(target_class) {
                    continue;
                }
                component.clear();
                queue.clear();
                component.push((x, y));
                queue.push_back((x, y));
                let mut has_spot = spot_cells.contains(&(x, y));

                while let Some((cx, cy)) = queue.pop_front() {
                    for (dx, dy) in [(1_isize, 0_isize), (-1, 0), (0, 1), (0, -1)] {
                        let (Some(nx), Some(ny)) =
                            (cx.checked_add_signed(dx), cy.checked_add_signed(dy))
                        else {
                            continue;
                        };
                        if nx >= w || ny >= h {
                            continue;
                        }
                        let n_idx = ny * w + nx;
                        if visited.get(n_idx).copied().unwrap_or(true) {
                            continue;
                        }
                        if classes.get(n_idx).copied() == Some(target_class) {
                            if let Some(v) = visited.get_mut(n_idx) {
                                *v = true;
                            }
                            if spot_cells.contains(&(nx, ny)) {
                                has_spot = true;
                            }
                            component.push((nx, ny));
                            queue.push_back((nx, ny));
                        }
                    }
                }

                if has_spot || component.len() >= min_region {
                    continue;
                }

                let comp_set: BTreeSet<(usize, usize)> = component.iter().copied().collect();
                let mut neighbour_counts: BTreeMap<u8, (usize, TerrainClass)> = BTreeMap::new();
                for &(cx, cy) in &component {
                    for (dx, dy) in [(1_isize, 0_isize), (-1, 0), (0, 1), (0, -1)] {
                        let (Some(nx), Some(ny)) =
                            (cx.checked_add_signed(dx), cy.checked_add_signed(dy))
                        else {
                            continue;
                        };
                        if nx >= w || ny >= h || comp_set.contains(&(nx, ny)) {
                            continue;
                        }
                        if let Some(&nc) = classes.get(ny * w + nx)
                            && nc != TerrainClass::Void
                        {
                            let entry = neighbour_counts.entry(nc.code()).or_insert((0, nc));
                            entry.0 += 1;
                        }
                    }
                }
                if let Some((_, &(_, replacement))) =
                    neighbour_counts
                        .iter()
                        .max_by(|(code_a, (cnt_a, _)), (code_b, (cnt_b, _))| {
                            cnt_a.cmp(cnt_b).then_with(|| code_b.cmp(code_a))
                        })
                {
                    for &(cx, cy) in &component {
                        if let Some(slot) = classes.get_mut(cy * w + cx)
                            && *slot != replacement
                        {
                            *slot = replacement;
                            changed_iter = true;
                        }
                    }
                }
            }
        }
        if !changed_iter {
            break;
        }
    }

    let modified = orig_classes
        .iter()
        .zip(classes.iter())
        .filter(|(a, b)| a != b)
        .count();
    if modified > 0 {
        draft.classes = encode_classes(draft.cols, draft.rows, &classes);
    }
    Ok(modified)
}

/// **Auto-route waypoint synthesis** (doc 30 §5): place a waypoint node at each
/// entry zone's muster/centre and at each [`KeySpot`], connect them by A\* over
/// the tracked cost grid under the sim's slope rule with an intermediate node
/// every 20 cells along each path, and write the graph into `draft.waypoints`.
///
/// # Errors
///
/// [`GroundError`] if the draft's grid fails to decode or canonical IDs cannot be formed.
pub fn route(draft: &mut GroundDraft) -> Result<(), GroundError> {
    let classes = decode_classes(&draft.id, draft.cols, draft.rows, &draft.classes)?;
    let elev = decode_elev(&draft.id, draft.cols, draft.rows, &draft.elev)?;
    let terrain = build_priced_terrain(draft, &classes, &elev)?;
    let table = LocomotionTable::shipped();
    let tracked_limit = table.max_slope_pct("tracked", &[]).unwrap_or(45);

    let cid = |s: &str| {
        CanonicalId::new(s).map_err(|e| GroundError::Invalid {
            id: s.to_owned(),
            detail: e.to_string(),
        })
    };

    let mut nodes: Vec<WaypointNode> = Vec::new();
    let mut edges: Vec<[CanonicalId; 2]> = Vec::new();
    let mut edge_set: BTreeSet<(String, String)> = BTreeSet::new();

    let find_or_insert_node = |nodes: &mut Vec<WaypointNode>,
                               preferred_id: &str,
                               x: u16,
                               y: u16|
     -> Result<CanonicalId, GroundError> {
        if let Some(existing) = nodes
            .iter()
            .find(|n| n.x.abs_diff(x) <= 2 && n.y.abs_diff(y) <= 2)
        {
            return Ok(existing.id.clone());
        }
        if nodes.len() >= MAX_WAYPOINT_NODES
            && let Some(nearest) = nodes
                .iter()
                .min_by_key(|n| u32::from(n.x.abs_diff(x)) + u32::from(n.y.abs_diff(y)))
        {
            return Ok(nearest.id.clone());
        }
        let id = cid(preferred_id)?;
        nodes.push(WaypointNode {
            id: id.clone(),
            x,
            y,
        });
        Ok(id)
    };

    let mut zone_anchors: Vec<(u16, u16)> = Vec::new();
    for z in &draft.entry_zones {
        let seeds = zone_seed_cells(&terrain, z, &draft.tanks, CostRow::Tracked);
        let (zx, zy) = seeds
            .first()
            .copied()
            .unwrap_or(((z.x0 + z.x1) / 2, (z.y0 + z.y1) / 2));
        let _ = find_or_insert_node(&mut nodes, &format!("muster_{}", z.team.as_str()), zx, zy)?;
        zone_anchors.push((zx, zy));
    }

    let mut spot_anchors: Vec<(u16, u16)> = Vec::new();
    for (idx, spot) in draft.key_spots.iter().enumerate() {
        let id = cid(&format!("spot_{idx}"))?;
        if !nodes.iter().any(|n| n.x == spot.x && n.y == spot.y) && nodes.len() < MAX_WAYPOINT_NODES
        {
            nodes.push(WaypointNode {
                id,
                x: spot.x,
                y: spot.y,
            });
        }
        spot_anchors.push((spot.x, spot.y));
    }

    let mut pairs: Vec<((u16, u16), (u16, u16))> = Vec::new();
    if spot_anchors.is_empty() {
        if let (Some(&a), Some(&b)) = (zone_anchors.first(), zone_anchors.get(1)) {
            pairs.push((a, b));
        }
    } else {
        for &z_xy in &zone_anchors {
            for &s_xy in &spot_anchors {
                pairs.push((z_xy, s_xy));
            }
        }
        for i in 0..spot_anchors.len() {
            if let (Some(&a), Some(&b)) = (spot_anchors.get(i), spot_anchors.get(i + 1)) {
                pairs.push((a, b));
            }
        }
    }

    let mut scratch = PathScratch::default();
    let mut step_counter = 0_usize;
    for (start, goal) in pairs {
        let Ok(res) = path_with(
            &terrain,
            &mut scratch,
            start,
            goal,
            u32::MAX,
            terrain.min_cost(CostRow::Tracked),
            Some(tracked_limit),
            CostRow::Tracked,
            &[],
        ) else {
            continue;
        };
        let mut waypoints_along: Vec<(u16, u16)> = vec![start];
        for (i, &cell) in res.cells.iter().enumerate() {
            if (i + 1) % 20 == 0 && cell != goal {
                waypoints_along.push(cell);
            }
        }
        waypoints_along.push(goal);

        let mut prev_id: Option<CanonicalId> = None;
        for (wx, wy) in waypoints_along {
            let name = format!("wp_{step_counter:03}");
            step_counter += 1;
            let cur_id = find_or_insert_node(&mut nodes, &name, wx, wy)?;
            if let Some(prev) = prev_id
                && prev != cur_id
                && edges.len() < MAX_WAYPOINT_EDGES
            {
                let a = prev.as_str().to_owned();
                let b = cur_id.as_str().to_owned();
                let key = if a <= b { (a, b) } else { (b, a) };
                if edge_set.insert(key) {
                    edges.push([prev, cur_id.clone()]);
                }
            }
            prev_id = Some(cur_id);
        }
    }

    draft.waypoints = WaypointGraph { nodes, edges };
    Ok(())
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
        biome: region_def.biome.unwrap_or_else(|| "temperate".to_owned()),
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

/// **Viewshed from `(ox, oy)`** (doc 30 §8 & §11 WG.4): classify every cell of
/// `draft` from the sim's 1.5 m eye (`mush_atlas::trace::EYE_HEIGHT_CM`) against
/// a target of height `size_cm` (e.g. `300` cm for a stock tank) using the
/// terrain's real [`crate::arena::horizon::Horizon`] index:
/// - `0` = dead ground (even the turret top at `size_cm` is blocked)
/// - `1` = seen (clear line of sight to both hull at `EYE_HEIGHT_CM` and turret)
/// - `2` = hull-down (turret at `size_cm` is seen or partial, hull at `EYE_HEIGHT_CM` is blocked)
/// - `3` = partial (woods cover the line partially, 25 % / 60 %)
///
/// # Errors
///
/// [`GroundError`] if the draft fails validation or `(ox, oy)` is off the window.
pub fn viewshed(
    draft: &GroundDraft,
    ox: u16,
    oy: u16,
    size_cm: i32,
) -> Result<Vec<u8>, GroundError> {
    if ox >= draft.cols || oy >= draft.rows {
        return Err(GroundError::Invalid {
            id: draft.id.clone(),
            detail: format!(
                "viewshed origin ({ox}, {oy}) is off {}×{} window",
                draft.cols, draft.rows
            ),
        });
    }
    let (classes, elev) = draft.validate()?;
    let terrain = build_priced_terrain(draft, &classes, &elev)?;
    let eye_cm = mush_atlas::trace::EYE_HEIGHT_CM;
    let from = mush_atlas::trace::Eye {
        cell: (ox, oy),
        height_cm: eye_cm,
    };
    let total = usize::from(draft.cols) * usize::from(draft.rows);
    let mut out = Vec::with_capacity(total);
    for y in 0..draft.rows {
        for x in 0..draft.cols {
            if x == ox && y == oy {
                out.push(1);
                continue;
            }
            let turret = terrain
                .sight(
                    from,
                    mush_atlas::trace::Eye {
                        cell: (x, y),
                        height_cm: size_cm,
                    },
                )
                .sight;
            if matches!(turret, mush_atlas::trace::Sight::BlockedBy(_)) {
                out.push(0);
                continue;
            }
            let hull = terrain
                .sight(
                    from,
                    mush_atlas::trace::Eye {
                        cell: (x, y),
                        height_cm: eye_cm,
                    },
                )
                .sight;
            if matches!(hull, mush_atlas::trace::Sight::BlockedBy(_)) {
                out.push(2);
            } else if matches!(turret, mush_atlas::trace::Sight::Partial(_))
                || matches!(hull, mush_atlas::trace::Sight::Partial(_))
            {
                out.push(3);
            } else {
                out.push(1);
            }
        }
    }
    Ok(out)
}

fn shipped_draft(short_id: &str) -> Option<Result<GroundDraft, GroundError>> {
    match short_id {
        "proving_ground" => Some(GroundDraft::from_json(include_str!(
            "../content/ground/proving_ground.draft.json"
        ))),
        "winter_valley" => Some(GroundDraft::from_json(include_str!(
            "../content/ground/winter_valley.draft.json"
        ))),
        "mars_canyon" => Some(GroundDraft::from_json(include_str!(
            "../content/ground/mars_canyon.draft.json"
        ))),
        _ => None,
    }
}

/// **Hot-shelve (and optionally apply) a [`GroundDraft`] into a running world**
/// (doc 30 §8, §13 `Q-30-5`):
///
/// 1. Validates `draft`, auto-bumps `atlas_version` and `arena_version` past any
///    already-published or already-applied version whose digest differs from the
///    candidate build, and builds + signs both packs (`tacsim.<id>` and
///    `tacsim.pack.<id>`).
/// 2. Hot-loads `tacsim.pack.<id>` into [`crate::store::TacsimStore`] via
///    [`crate::store::TacsimStore::stage_pack`] and records its ledger row via
///    `crate::store::put_pack`.
/// 3. Places both packs onto the [`mush_atlas::AtlasState`] shelf via
///    [`mush_atlas::stage_pack`] and computes their [`mush_content::plan`]
///    fingerprints.
/// 4. When `apply` is `true` (`@ground/stage/apply`), checks that no match is
///    running on the simulator centre and applies `tacsim.<id>` followed by
///    `tacsim.pack.<id>` via [`mush_content::apply_pack`].
///
/// # Errors
///
/// [`GroundError`] if validation, pack signing, store binding, or pack
/// application fails.
pub fn stage_in_world(
    ctx: &mut mush_core::Ctx<'_>,
    draft: &GroundDraft,
    apply: bool,
) -> Result<StagedGround, GroundError> {
    if apply && crate::match_::lobby::centre(ctx.w).is_some_and(|centre| centre.state.is_live()) {
        return Err(GroundError::Stage(
            "cannot apply a ground draft while a match is running on the simulator centre"
                .to_owned(),
        ));
    }

    let mut working = draft.clone();
    let mut built = build(&working)?;
    let mut atlas_pack =
        mush_content::load_pack(&built.atlas_json).map_err(|e| GroundError::Pack(e.to_string()))?;
    let mut arena_rules_pack =
        crate::pack::load_pack(&built.arena_json).map_err(|e| GroundError::Pack(e.to_string()))?;
    let atlas_id = atlas_pack.manifest().id().clone();
    let rules_id = arena_rules_pack.pack().manifest().id().clone();

    if let Some(content) = ctx.w.module_state::<mush_content::ContentState>() {
        if let Some(applied) = content.latches.applied(&atlas_id)
            && (applied.version() > working.atlas_version
                || (applied.version() == working.atlas_version
                    && applied.digest() != atlas_pack.manifest().digest()))
        {
            working.atlas_version = applied.version().saturating_add(1);
        }
        if let Some(applied) = content.latches.applied(&rules_id)
            && (applied.version() > working.arena_version
                || (applied.version() == working.arena_version
                    && applied.digest() != arena_rules_pack.pack().manifest().digest()))
        {
            working.arena_version = applied.version().saturating_add(1);
        }
    }

    if let Some(atlas) = ctx.w.module_state::<mush_atlas::AtlasState>() {
        if let Some(shelved) = atlas.shelf.get(&atlas_id) {
            let m = shelved.manifest();
            if m.version() > working.atlas_version
                || (m.version() == working.atlas_version
                    && m.digest() != atlas_pack.manifest().digest())
            {
                working.atlas_version = m.version().saturating_add(1);
            }
        }
        if let Some(shelved) = atlas.shelf.get(&rules_id) {
            let m = shelved.manifest();
            if m.version() > working.arena_version
                || (m.version() == working.arena_version
                    && m.digest() != arena_rules_pack.pack().manifest().digest())
            {
                working.arena_version = m.version().saturating_add(1);
            }
        }
    }

    if let Some(store) = ctx.w.module_state::<crate::store::TacsimStore>() {
        let candidate_digest = arena_rules_pack.pack().manifest().digest();
        let mut max_v = 0_u32;
        let mut collision = false;
        for ((id, v), row) in &store.published {
            if id == &rules_id {
                max_v = max_v.max(*v);
                if *v == working.arena_version && row.digest() != candidate_digest {
                    collision = true;
                }
            }
        }
        for ((id, v), loaded) in &store.loaded {
            if id == &rules_id {
                max_v = max_v.max(*v);
                if *v == working.arena_version
                    && loaded.pack.manifest().digest() != candidate_digest
                {
                    collision = true;
                }
            }
        }
        if max_v > working.arena_version || collision {
            working.arena_version = max_v.max(working.arena_version).saturating_add(1);
        }
    }

    if working.atlas_version != draft.atlas_version || working.arena_version != draft.arena_version
    {
        built = build(&working)?;
        atlas_pack = mush_content::load_pack(&built.atlas_json)
            .map_err(|e| GroundError::Pack(e.to_string()))?;
        arena_rules_pack = crate::pack::load_pack(&built.arena_json)
            .map_err(|e| GroundError::Pack(e.to_string()))?;
    }

    let atlas_digest = atlas_pack.manifest().digest().to_hex();
    let arena_digest = arena_rules_pack.pack().manifest().digest().to_hex();
    let quadrants =
        usize::from(working.cols.div_ceil(QUAD)) * usize::from(working.rows.div_ceil(QUAD));

    // Hot-shelve into TacsimStore and re-bind definitions.
    let loaded = crate::LoadedPack {
        pack: arena_rules_pack.clone(),
        source: format!("staged:{}", working.id),
    };
    let need_ledger_row = {
        let store = ctx
            .w
            .module_state_mut::<crate::store::TacsimStore>()
            .ok_or_else(|| {
                GroundError::Stage("the TacSim store is not composed on this world".to_owned())
            })?;
        store
            .stage_pack(loaded)
            .map_err(|e| GroundError::Stage(e.to_string()))?;
        store
            .staged_drafts
            .insert(working.id.clone(), working.clone());
        let key = (rules_id.clone(), working.arena_version);
        !store.published.contains_key(&key)
    };
    if need_ledger_row {
        let manifest = arena_rules_pack.pack().manifest();
        let entries = u32::try_from(arena_rules_pack.pack().entries().len())
            .map_err(|_| GroundError::Stage("arena pack entry count overflow".to_owned()))?;
        let first_seen = ctx.world_ic_now();
        let record = crate::record::PackRecord::new(
            manifest.id().clone(),
            manifest.version(),
            manifest.kind(),
            manifest.digest(),
            entries,
            first_seen,
        )
        .map_err(|e| GroundError::Stage(e.to_string()))?;
        crate::store::put_pack(ctx, record);
        crate::store::persist_meta(ctx);
    }

    // Place both packs onto the atlas shelf.
    mush_atlas::stage_pack(ctx, atlas_pack.clone()).map_err(GroundError::Stage)?;
    mush_atlas::stage_pack(ctx, arena_rules_pack.pack().clone()).map_err(GroundError::Stage)?;

    let (atlas_plan, rules_plan) = {
        let content = ctx
            .w
            .module_state::<mush_content::ContentState>()
            .ok_or_else(|| {
                GroundError::Stage("the content module is not composed on this world".to_owned())
            })?;
        let ap = mush_content::plan(&atlas_pack, content.latches.applied(&atlas_id))
            .map_err(|e| GroundError::Stage(e.to_string()))?;
        let rp = mush_content::plan(arena_rules_pack.pack(), content.latches.applied(&rules_id))
            .map_err(|e| GroundError::Stage(e.to_string()))?;
        (ap, rp)
    };
    let atlas_fingerprint = atlas_plan.fingerprint();
    let arena_fingerprint = rules_plan.fingerprint();

    if apply {
        if !atlas_plan.is_noop() {
            let (_applied, report) = mush_content::apply_pack(ctx, &atlas_pack)
                .map_err(|e| GroundError::Stage(e.to_string()))?;
            if let Some(anomaly) = report.anomaly {
                return Err(GroundError::Stage(format!(
                    "{atlas_id} apply stopped part way: {anomaly}"
                )));
            }
        }
        if !rules_plan.is_noop() {
            let (_applied, report) = mush_content::apply_pack(ctx, arena_rules_pack.pack())
                .map_err(|e| GroundError::Stage(e.to_string()))?;
            if let Some(anomaly) = report.anomaly {
                return Err(GroundError::Stage(format!(
                    "{rules_id} apply stopped part way: {anomaly}"
                )));
            }
        }
        let arena_entry = smol_str::SmolStr::new(format!("tacsim.arena.{}", working.id));
        let _ = crate::match_::definition_for(ctx.w, &arena_entry).map_err(GroundError::Stage)?;
    }

    Ok(StagedGround {
        id: working.id,
        atlas_pack_id: atlas_id,
        atlas_version: working.atlas_version,
        atlas_digest,
        arena_pack_id: rules_id,
        arena_version: working.arena_version,
        arena_digest,
        dims: (working.cols, working.rows),
        quadrants,
        applied: apply,
        atlas_plan_fingerprint: Some(atlas_fingerprint),
        arena_plan_fingerprint: Some(arena_fingerprint),
    })
}

/// **Serve an arena draft from a running world** (doc 30 §8 & §11 WG.4 / WG.5 / WG.7):
/// reconstructs the [`GroundDraft`] from the world's [`crate::store::TacsimStore`]
/// and [`mush_atlas::AtlasState`] when the arena is loaded there (preserving the
/// staged or checked-in `<id>.draft.json` metadata when the cells and furniture
/// match it), falls back to an operator-staged draft on the shelf before apply,
/// or serves the checked-in `<id>.draft.json` directly when the world has not
/// composed its content store yet.
///
/// # Errors
///
/// [`GroundError`] if the arena `id` is unknown or its tiles are missing.
pub fn draft_from_world(w: &mush_core::World, id: &str) -> Result<GroundDraft, GroundError> {
    let short_id = id.strip_prefix("tacsim.arena.").unwrap_or(id);
    let entry_str = format!("tacsim.arena.{short_id}");
    let staged = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|slot| slot.staged_drafts.get(short_id).cloned());
    let Some(arena_def) = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|slot| {
            CanonicalId::new(&entry_str)
                .ok()
                .and_then(|entry| slot.definitions().arenas.get(&entry).cloned())
        })
    else {
        if let Some(staged) = staged {
            return Ok(staged);
        }
        if let Some(shipped) = shipped_draft(short_id) {
            return shipped;
        }
        return Err(GroundError::Invalid {
            id: short_id.to_owned(),
            detail: format!("arena `{entry_str}` is not loaded on this world"),
        });
    };
    let Some(records) = w
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| &state.records)
    else {
        if let Some(staged) = staged {
            return Ok(staged);
        }
        if let Some(shipped) = shipped_draft(short_id) {
            return shipped;
        }
        return Err(GroundError::Invalid {
            id: short_id.to_owned(),
            detail: "atlas state is not composed on this world".to_owned(),
        });
    };
    match draft_from_records(short_id, &arena_def, records) {
        Ok(rebuilt) => {
            if let Some(staged) = staged
                && staged.classes == rebuilt.classes
                && staged.elev == rebuilt.elev
                && staged.key_spots == rebuilt.key_spots
                && staged.waypoints == rebuilt.waypoints
                && staged.entry_zones == rebuilt.entry_zones
                && staged.biome == rebuilt.biome
            {
                return Ok(staged);
            }
            Ok(rebuilt)
        }
        Err(err) => {
            if let Some(staged) = staged {
                return Ok(staged);
            }
            Err(err)
        }
    }
}

/// Reassemble a [`GroundDraft`] from an [`ArenaDef`] and loaded atlas [`mush_atlas::record::Records`].
///
/// # Errors
///
/// - [`GroundError::ArenaBuild`] if the region or any quadrant tile is missing or uncovered.
/// - [`GroundError::Pack`] if tile bounding-box coordinates cannot be converted to grid offsets.
pub fn draft_from_records(
    short_id: &str,
    arena_def: &ArenaDef,
    records: &mush_atlas::record::Records,
) -> Result<GroundDraft, GroundError> {
    let short_id = short_id.strip_prefix("tacsim.arena.").unwrap_or(short_id);
    let entry_str = format!("tacsim.arena.{short_id}");
    let region_key = arena_def.region_key();
    let region_rec = records.regions.get(&region_key).ok_or_else(|| {
        GroundError::ArenaBuild(ArenaBuildError::NoRegion {
            arena: entry_str.clone(),
            region: region_key.to_string(),
        })
    })?;
    let total = usize::from(arena_def.cols) * usize::from(arena_def.rows);
    let mut classes = vec![TerrainClass::Grass; total];
    let mut elev = vec![0_i16; total];
    let mut covered = vec![false; total];
    let cell_cm = i64::from(arena_def.cell_cm).max(1);
    let origin_cm = (
        arena_def.origin_m[0].saturating_mul(100),
        arena_def.origin_m[1].saturating_mul(100),
    );
    let keys = arena_def
        .tile_keys(&entry_str)
        .map_err(|e| GroundError::ArenaCheck {
            id: short_id.to_owned(),
            detail: e.to_string(),
        })?;
    for key in keys {
        let tile = records.tiles.get(&key).ok_or_else(|| {
            GroundError::ArenaBuild(ArenaBuildError::NoTile {
                arena: entry_str.clone(),
                tile: key.to_string(),
            })
        })?;
        let tx_cm = mush_atlas::geom::cm(tile.bbox.0.0)
            .map_err(|e| GroundError::Pack(format!("{key} bbox east: {e}")))?;
        let ty_cm = mush_atlas::geom::cm(tile.bbox.0.1)
            .map_err(|e| GroundError::Pack(format!("{key} bbox north: {e}")))?;
        let ox = u16::try_from((tx_cm - origin_cm.0) / cell_cm)
            .map_err(|_| GroundError::Pack(format!("{key} origin east offset out of range")))?;
        let oy = u16::try_from((ty_cm - origin_cm.1) / cell_cm)
            .map_err(|_| GroundError::Pack(format!("{key} origin north offset out of range")))?;
        for y in 0..tile.cells.height() {
            for x in 0..tile.cells.width() {
                let Some(cell) = tile.cells.get(x, y) else {
                    continue;
                };
                let wx = ox.saturating_add(x);
                let wy = oy.saturating_add(y);
                if wx >= arena_def.cols || wy >= arena_def.rows {
                    continue;
                }
                let dst_idx = usize::from(wy) * usize::from(arena_def.cols) + usize::from(wx);
                if let Some(slot) = classes.get_mut(dst_idx) {
                    *slot = cell.class;
                }
                if let Some(slot) = elev.get_mut(dst_idx) {
                    *slot = cell.elev_cm;
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
            arena: entry_str,
            missing,
            total,
        }));
    }
    let classes_b64 = encode_classes(arena_def.cols, arena_def.rows, &classes);
    let elev_b64 = encode_elev(&elev);
    if let Some(Ok(shipped)) = shipped_draft(short_id)
        && shipped.classes == classes_b64
        && shipped.elev == elev_b64
        && shipped.key_spots == arena_def.key_spots
        && shipped.waypoints == arena_def.waypoints
        && shipped.entry_zones == arena_def.entry_zones
        && shipped.biome == region_rec.biome_str()
    {
        return Ok(shipped);
    }
    let body = match &region_rec.kind {
        mush_atlas::record::RegionKind::Surface { body }
        | mush_atlas::record::RegionKind::Orbital { body } => body.clone(),
        _ => "luna".to_owned(),
    };
    Ok(GroundDraft {
        schema: DRAFT_SCHEMA.to_owned(),
        id: short_id.to_owned(),
        label: arena_def.label.clone(),
        biome: region_rec.biome_str().to_owned(),
        body,
        origin_m: arena_def.origin_m,
        cell_cm: arena_def.cell_cm,
        cols: arena_def.cols,
        rows: arena_def.rows,
        classes: classes_b64,
        elev: elev_b64,
        entry_zones: arena_def.entry_zones.clone(),
        key_spots: arena_def.key_spots.clone(),
        waypoints: arena_def.waypoints.clone(),
        modes: arena_def.modes.clone(),
        tanks: arena_def.tanks.clone(),
        atlas_version: u32::try_from(region_rec.revision).unwrap_or(1),
        arena_version: 1,
        provenance: GroundProvenance {
            source: GroundSource::Pack,
            version: format!("v{}", region_rec.revision),
            seed: Some(region_rec.background.seed),
            inputs: vec![
                format!("{}:{}", region_key.pack, region_key.entry),
                format!("{}:{entry_str}", arena_def.pack),
            ],
        },
        note: arena_def.note.clone(),
    })
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

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn generate_biome_preset(
    preset: &str,
    default_biome: &str,
    body: &str,
    label: &str,
    seed: u64,
    biome_override: Option<&str>,
) -> Result<GroundDraft, GroundError> {
    if let Some(b) = biome_override
        && b != default_biome
    {
        return Err(GroundError::Invalid {
            id: preset.to_owned(),
            detail: format!(
                "preset `{preset}` requires biome `{default_biome}`; got `--biome {b}`"
            ),
        });
    }
    let phase = (seed % 10_000) as f64 * 0.017_453_292_519_943_295;
    let shift_x = 6.0 * libm::sin(phase);
    let shift_y = 6.0 * libm::cos(phase * 1.3);
    let road_class = if default_biome == "lunar" {
        TerrainClass::Paved
    } else {
        TerrainClass::Road
    };

    let mut classes = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    let mut elev = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));

    for y in 0..SIDE {
        for x in 0..SIDE {
            let fx = f64::from(x);
            let fy = f64::from(y);

            // 1. Author per-biome surface class (doc 30 §6: no grass or woods on mars/lunar/ice).
            let class = if within(x, y, BLUFF_TOP) {
                match default_biome {
                    "mars" => TerrainClass::Sand,
                    "lunar" => TerrainClass::Regolith,
                    _ => TerrainClass::Ice,
                }
            } else if BLUFF_RAMP.iter().any(|(b, _)| within(x, y, *b)) {
                TerrainClass::Ground
            } else if let Some(b) = BUILDINGS.iter().copied().find(|b| within(x, y, *b)) {
                let door_x = (b.0 + b.1) / 2;
                if y == b.2 && x == door_x {
                    TerrainClass::Door
                } else if x == b.0 || x == b.1 || y == b.2 || y == b.3 {
                    TerrainClass::Wall
                } else {
                    TerrainClass::Floor
                }
            } else if (y >= ROAD_EW.0 && y <= ROAD_EW.1)
                || (x >= ROAD_NS.0 && x <= ROAD_NS.1)
                || ((106..=128).contains(&x) && (184..=185).contains(&y))
                || ((207..=209).contains(&x) && (99..=139).contains(&y))
            {
                road_class
            } else if x >= RIVER.0 && x <= RIVER.1 {
                if y >= FORD.0 && y <= FORD.1 {
                    if default_biome == "ice" {
                        TerrainClass::Ice
                    } else {
                        TerrainClass::Ground
                    }
                } else if default_biome == "ice" {
                    TerrainClass::WaterDeep
                } else {
                    TerrainClass::Ground
                }
            } else {
                let n = libm::sin((fx + shift_x) * 0.09 + phase)
                    * libm::cos((fy + shift_y) * 0.08 - phase * 0.5)
                    + 0.5 * libm::sin((fx - fy) * 0.05 + phase);
                let dist_road = (fx - 129.0).abs().min((fy - 97.0).abs());
                let in_entry = x <= 18 || x >= 237;
                if !in_entry && dist_road >= 8.0 && within(x, y, RUBBLE[0]) {
                    TerrainClass::Rubble
                } else if !in_entry
                    && dist_road >= 10.0
                    && (within(x, y, (24, 52, 156, 192)) || within(x, y, (168, 202, 46, 74)))
                {
                    if n > 0.35 {
                        TerrainClass::Rubble
                    } else {
                        TerrainClass::Rock
                    }
                } else {
                    match default_biome {
                        "mars" => {
                            if n > 0.45 {
                                TerrainClass::Regolith
                            } else if n < -0.35 {
                                TerrainClass::Ground
                            } else {
                                TerrainClass::Sand
                            }
                        }
                        "lunar" => {
                            if n > 0.30 {
                                TerrainClass::Ground
                            } else {
                                TerrainClass::Regolith
                            }
                        }
                        _ => {
                            if n > 0.35 {
                                TerrainClass::Ground
                            } else {
                                TerrainClass::Ice
                            }
                        }
                    }
                }
            };
            classes.push(class);

            // 2. Author per-biome relief at §5 amplitudes, keeping entry zones flat at 0 cm
            //    and keeping roads smooth (<= 20% grade).
            if within(x, y, BLUFF_TOP) {
                elev.push(BLUFF_TOP_CM);
                continue;
            }
            if let Some((_, ramp_cm)) = BLUFF_RAMP.iter().copied().find(|(b, _)| within(x, y, *b)) {
                elev.push(ramp_cm);
                continue;
            }

            let edge_dist = f64::from(x.saturating_sub(15).min(240_u16.saturating_sub(x)));
            let edge_w = smoothstep(edge_dist / 16.0);
            let corridor = f64::from(
                V3_CORRIDOR
                    .0
                    .saturating_sub(y)
                    .max(y.saturating_sub(V3_CORRIDOR.1)),
            );
            let road_ns = f64::from(x.abs_diff(ROAD_NS.0 + 1)) - 1.0;
            let spur_barn = box_distance(x, y, (104, 130, 182, 204));
            let spur_comp = box_distance(x, y, (198, 218, 96, 158));
            let keep = corridor
                .min(road_ns.max(0.0))
                .min(spur_barn)
                .min(spur_comp)
                .min(box_distance(x, y, V3_BLUFF_KEEP));
            let w = smoothstep(keep / V3_BLEND) * edge_w;

            let base_wave = edge_w
                * (160.0 * libm::sin((fx + shift_x) / 42.0)
                    + 120.0 * libm::cos((fy + shift_y) / 52.0));

            let raw_feature = match default_biome {
                "mars" => {
                    // Canyon wadi trough along x = 64, stepped shelves north/south, and Olympus Spur summit.
                    let wadi = -450.0 * libm::exp(-sq((fx - 64.0) / 11.0));
                    let north_shelf = 3_200.0 * smoothstep((fy - 122.0) / 36.0);
                    let south_shelf = 2_200.0 * smoothstep((72.0 - fy) / 28.0);
                    let spur_peak = 6_200.0
                        * gauss(
                            fx - (184.0 + shift_x * 0.5),
                            fy - (214.0 + shift_y * 0.5),
                            22.0,
                        );
                    let crater_r = libm::hypot(fx - 88.0, fy - 38.0);
                    let crater = if crater_r < 16.0 {
                        -900.0 + 2_200.0 * sq(crater_r / 16.0)
                    } else {
                        1_300.0 * libm::exp(-sq((crater_r - 16.0) / 8.0))
                    };
                    wadi + north_shelf + south_shelf + spur_peak + crater
                }
                "lunar" => {
                    // Two ringed impact craters with central uplift and northern highland massif.
                    let r1 = libm::hypot(fx - (86.0 + shift_x * 0.4), fy - (40.0 + shift_y * 0.4));
                    let c1 = if r1 < 18.0 {
                        -1_100.0 + 2_900.0 * sq(r1 / 18.0)
                    } else {
                        1_800.0 * libm::exp(-sq((r1 - 18.0) / 8.5))
                    };
                    let r2 =
                        libm::hypot(fx - (182.0 - shift_x * 0.4), fy - (210.0 - shift_y * 0.4));
                    let c2 = if r2 < 22.0 {
                        -1_200.0
                            + 3_600.0 * sq(r2 / 22.0)
                            + 1_400.0 * gauss(fx - 182.0, fy - 210.0, 5.0)
                    } else {
                        2_400.0 * libm::exp(-sq((r2 - 22.0) / 9.5))
                    };
                    let highlands = 2_800.0 * smoothstep((fy - 135.0) / 42.0);
                    c1 + c2 + highlands
                }
                _ => {
                    // Glacial shelf, pressure ridges, and ice dome.
                    let shelf = 2_900.0 * smoothstep((fy - 125.0) / 40.0);
                    let lead = -380.0 * libm::exp(-sq((fx - 64.0) / 10.0));
                    let ridge_a = 2_400.0
                        * libm::exp(-sq((fy - 58.0 - 5.0 * libm::sin(fx / 24.0)) / 9.0))
                        * smoothstep((fx - 30.0) / 14.0)
                        * smoothstep((225.0 - fx) / 14.0);
                    let dome = 5_200.0
                        * gauss(
                            fx - (186.0 + shift_x * 0.4),
                            fy - (214.0 + shift_y * 0.4),
                            21.0,
                        );
                    shelf + lead + ridge_a + dome
                }
            };

            let mut feature = raw_feature;
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
                let t = smoothstep(near / 6.0);
                feature *= t;
            }
            let cm = if x <= 15 || x >= 240 {
                0_i16
            } else {
                let v = ((base_wave + w * feature) / 10.0).round() * 10.0;
                v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
            };
            elev.push(cm);
        }
    }

    let preset_origin_m = match default_biome {
        "mars" => 40_000_i64,
        "lunar" => 50_000_i64,
        "ice" => 60_000_i64,
        _ => ORIGIN_M,
    };
    let base = proving_ground_draft()?;
    let mut draft = GroundDraft {
        schema: DRAFT_SCHEMA.to_owned(),
        id: preset.to_owned(),
        label: label.to_owned(),
        biome: default_biome.to_owned(),
        body: body.to_owned(),
        origin_m: [preset_origin_m, preset_origin_m],
        cell_cm: CELL_CM,
        cols: SIDE,
        rows: SIDE,
        classes: encode_classes(SIDE, SIDE, &classes),
        elev: encode_elev(&elev),
        entry_zones: base.entry_zones,
        key_spots: base.key_spots,
        waypoints: base.waypoints,
        modes: base.modes,
        tanks: base.tanks,
        atlas_version: 1,
        arena_version: 1,
        provenance: GroundProvenance {
            source: GroundSource::Generator,
            version: "v1".to_owned(),
            seed: Some(seed),
            inputs: Vec::new(),
        },
        note: String::new(),
    };
    let _ = despeckle(&mut draft, DEFAULT_SPECKLE_MIN_CELLS)?;
    let report = check(&draft)?;
    draft.note = report.summary_note();
    Ok(draft)
}

/// **The generator** (doc 30 §4 & §9, D-30-9): generate a [`GroundDraft`] for a
/// named preset (`"proving_ground"`, `"mars_canyon"`, `"lunar_crater"`, `"ice_shelf"`).
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
        "mars_canyon" => {
            generate_biome_preset("mars_canyon", "mars", "mars", "Mars Canyon", seed, biome)
        }
        "lunar_crater" => {
            generate_biome_preset("lunar_crater", "lunar", "luna", "Lunar Crater", seed, biome)
        }
        "ice_shelf" => {
            generate_biome_preset("ice_shelf", "ice", "europa", "Ice Shelf", seed, biome)
        }
        other => Err(GroundError::Invalid {
            id: other.to_owned(),
            detail: format!(
                "unknown generator preset `{other}` (expected `proving_ground`, `mars_canyon`, `lunar_crater`, or `ice_shelf`)"
            ),
        }),
    }
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn write_png_chunk(out: &mut Vec<u8>, tag: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(0);
    out.extend_from_slice(&len.to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(&tag);
    out.extend_from_slice(data);
    let crc = png_crc32(out.get(crc_start..).unwrap_or(&[]));
    out.extend_from_slice(&crc.to_be_bytes());
}

fn biome_class_rgb(biome: &str, class: TerrainClass) -> [u8; 3] {
    match biome {
        "mars" => match class {
            TerrainClass::Sand => [194, 108, 68],
            TerrainClass::Regolith => [156, 82, 52],
            TerrainClass::Ground => [138, 70, 44],
            TerrainClass::Road | TerrainClass::Paved => [168, 124, 96],
            TerrainClass::Rock => [112, 58, 42],
            TerrainClass::Rubble => [128, 66, 48],
            TerrainClass::Wall => [212, 180, 152],
            TerrainClass::Floor => [118, 92, 78],
            TerrainClass::Door => [230, 176, 80],
            _ => [164, 90, 56],
        },
        "lunar" => match class {
            TerrainClass::Regolith => [142, 144, 150],
            TerrainClass::Ground => [118, 120, 126],
            TerrainClass::Paved | TerrainClass::Road => [176, 178, 184],
            TerrainClass::Rock => [94, 96, 104],
            TerrainClass::Rubble => [108, 110, 118],
            TerrainClass::Wall => [216, 218, 224],
            TerrainClass::Floor => [124, 126, 132],
            TerrainClass::Door => [224, 188, 88],
            _ => [134, 136, 142],
        },
        "ice" => match class {
            TerrainClass::Ice => [206, 228, 244],
            TerrainClass::Ground => [132, 148, 166],
            TerrainClass::Road | TerrainClass::Paved => [162, 178, 194],
            TerrainClass::Rock => [96, 110, 128],
            TerrainClass::Rubble => [116, 132, 150],
            TerrainClass::WaterDeep | TerrainClass::Water => [28, 64, 112],
            TerrainClass::Wall => [232, 240, 248],
            TerrainClass::Floor => [138, 150, 164],
            TerrainClass::Door => [232, 190, 86],
            _ => [190, 214, 232],
        },
        _ => match class {
            TerrainClass::Grass => [78, 124, 68],
            TerrainClass::Ground => [134, 110, 82],
            TerrainClass::Road | TerrainClass::Paved => [164, 152, 132],
            TerrainClass::WoodsLight => [52, 94, 48],
            TerrainClass::WoodsHeavy => [32, 66, 32],
            TerrainClass::Rubble => [118, 112, 104],
            TerrainClass::Rock => [104, 100, 96],
            TerrainClass::Water => [58, 124, 168],
            TerrainClass::WaterDeep => [28, 76, 122],
            TerrainClass::Wall => [208, 200, 188],
            TerrainClass::Floor => [128, 122, 114],
            TerrainClass::Door => [228, 182, 78],
            TerrainClass::Sand => [194, 174, 128],
            TerrainClass::Ice => [206, 228, 244],
            TerrainClass::Regolith => [142, 144, 150],
            _ => [96, 96, 96],
        },
    }
}

/// **Render an overhead PNG of a [`GroundDraft`]** (doc 30 §4 `D-30-2` & §8):
/// draws a `2 × 2`-pixel-per-cell 24-bit RGB PNG with biome-keyed surface colors,
/// NW hillshading from `elev_cm`, 10 m contour lines, entry-zone borders,
/// waypoint edges/nodes, key-spot markers, and an optional [`viewshed`] overlay.
///
/// # Errors
///
/// [`GroundError`] if `draft` fails validation or `viewshed_origin` is out of bounds.
pub fn render_png(
    draft: &GroundDraft,
    viewshed_origin: Option<(u16, u16)>,
    size_cm: i32,
) -> Result<Vec<u8>, GroundError> {
    use std::io::Write as _;

    let (classes, elev) = draft.validate()?;
    let vs = if let Some((ox, oy)) = viewshed_origin {
        Some(viewshed(draft, ox, oy, size_cm)?)
    } else {
        None
    };

    let scale = 2_usize;
    let w = usize::from(draft.cols);
    let h = usize::from(draft.rows);
    let img_w = w * scale;
    let img_h = h * scale;
    let mut rgb = vec![0_u8; img_w * img_h * 3];

    let elev_at_cell =
        |cx: usize, cy: usize| -> i32 { i32::from(elev.get(cy * w + cx).copied().unwrap_or(0)) };

    for cy in 0..h {
        let py_base = (h - 1 - cy) * scale;
        for cx in 0..w {
            let idx = cy * w + cx;
            let class = classes.get(idx).copied().unwrap_or(TerrainClass::Ground);
            let base = biome_class_rgb(&draft.biome, class);

            let e_here = elev_at_cell(cx, cy);
            let e_w = elev_at_cell(cx.saturating_sub(1), cy);
            let e_e = elev_at_cell((cx + 1).min(w - 1), cy);
            let e_s = elev_at_cell(cx, cy.saturating_sub(1));
            let e_n = elev_at_cell(cx, (cy + 1).min(h - 1));
            let dx = e_e - e_w;
            let dy = e_n - e_s;
            let shade = ((-dx + dy) / 22).clamp(-42, 42);
            let contour = (e_here / 1_000 != e_e / 1_000) || (e_here / 1_000 != e_n / 1_000);
            let contour_bias = if contour { -16 } else { 0 };

            let mut r = (i32::from(base[0]) + shade + contour_bias).clamp(0, 255);
            let mut g = (i32::from(base[1]) + shade + contour_bias).clamp(0, 255);
            let mut b = (i32::from(base[2]) + shade + contour_bias).clamp(0, 255);

            if let Some(vs_grid) = &vs {
                match vs_grid.get(idx).copied().unwrap_or(0) {
                    1 => {
                        r = (r * 3 + 70) / 4;
                        g = (g * 3 + 220) / 4;
                        b = (b * 3 + 90) / 4;
                    }
                    2 => {
                        r = (r * 3 + 235) / 4;
                        g = (g * 3 + 180) / 4;
                        b = (b * 3 + 50) / 4;
                    }
                    3 => {
                        r = (r * 3 + 70) / 4;
                        g = (g * 3 + 190) / 4;
                        b = (b * 3 + 225) / 4;
                    }
                    _ => {
                        r = (r * 2) / 3;
                        g = (g * 2) / 3;
                        b = (b * 2) / 3;
                    }
                }
            }

            let pixel = [
                u8::try_from(r).unwrap_or(0),
                u8::try_from(g).unwrap_or(0),
                u8::try_from(b).unwrap_or(0),
            ];
            for sy in 0..scale {
                for sx in 0..scale {
                    let px = cx * scale + sx;
                    let py = py_base + sy;
                    let p_idx = (py * img_w + px) * 3;
                    if let Some(dst) = rgb.get_mut(p_idx..p_idx + 3) {
                        dst.copy_from_slice(&pixel);
                    }
                }
            }
        }
    }

    let put_px = |buf: &mut [u8], px: usize, py: usize, c: [u8; 3]| {
        if px < img_w && py < img_h {
            let p_idx = (py * img_w + px) * 3;
            if let Some(dst) = buf.get_mut(p_idx..p_idx + 3) {
                dst.copy_from_slice(&c);
            }
        }
    };

    // Entry zone outlines
    for z in &draft.entry_zones {
        let color = match z.team {
            Team::Red => [225, 72, 64],
            Team::Blue => [68, 142, 235],
        };
        let x0 = usize::from(z.x0) * scale;
        let x1 = ((usize::from(z.x1) + 1) * scale)
            .saturating_sub(1)
            .min(img_w - 1);
        let y0 = (h.saturating_sub(1 + usize::from(z.y1))) * scale;
        let y1 = ((h - usize::from(z.y0)) * scale)
            .saturating_sub(1)
            .min(img_h - 1);
        for px in x0..=x1 {
            put_px(&mut rgb, px, y0, color);
            put_px(&mut rgb, px, y1, color);
        }
        for py in y0..=y1 {
            put_px(&mut rgb, x0, py, color);
            put_px(&mut rgb, x1, py, color);
        }
    }

    // Waypoint nodes and key spots
    for n in &draft.waypoints.nodes {
        let px = usize::from(n.x) * scale;
        let py = (h.saturating_sub(1 + usize::from(n.y))) * scale;
        put_px(&mut rgb, px, py, [80, 220, 245]);
    }
    for spot in &draft.key_spots {
        let cx = usize::from(spot.x) * scale;
        let cy = (h.saturating_sub(1 + usize::from(spot.y))) * scale;
        for dy in 0..=2_usize {
            for dx in 0..=2_usize {
                put_px(
                    &mut rgb,
                    cx.saturating_add(dx).saturating_sub(1),
                    cy.saturating_add(dy).saturating_sub(1),
                    [255, 225, 64],
                );
            }
        }
    }

    let mut raw_scanlines = Vec::with_capacity((img_w * 3 + 1) * img_h);
    for py in 0..img_h {
        raw_scanlines.push(0_u8);
        let row_start = py * img_w * 3;
        if let Some(row_bytes) = rgb.get(row_start..row_start + img_w * 3) {
            raw_scanlines.extend_from_slice(row_bytes);
        }
    }

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&raw_scanlines)
        .map_err(|e| GroundError::Pack(format!("png zlib encode failed: {e}")))?;
    let idat = encoder
        .finish()
        .map_err(|e| GroundError::Pack(format!("png zlib finish failed: {e}")))?;

    let mut png = Vec::with_capacity(idat.len() + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0_u8; 13];
    ihdr[0..4].copy_from_slice(&u32::try_from(img_w).unwrap_or(0).to_be_bytes());
    ihdr[4..8].copy_from_slice(&u32::try_from(img_h).unwrap_or(0).to_be_bytes());
    ihdr[8] = 8; // 8 bits per channel
    ihdr[9] = 2; // RGB truecolor
    write_png_chunk(&mut png, *b"IHDR", &ihdr);
    write_png_chunk(&mut png, *b"IDAT", &idat);
    write_png_chunk(&mut png, *b"IEND", &[]);
    Ok(png)
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

    #[test]
    fn wg3_proving_ground_check_report_meets_all_section_5_targets() -> Result<(), Box<dyn Error>> {
        let draft = proving_ground_draft()?;
        let report = check(&draft)?;
        check!(report.passed);
        check!(report.failures.is_empty());
        check!(report.tracked_passable_pct >= 90);
        check!(report.legged_passable_pct >= 95);
        check!(report.zones_reachable_tracked);
        check!(report.zones_reachable_wheeled);
        check!(report.zones_reachable_legged);
        check!(report.key_spots_wheeled_reachable);
        check!(report.unreachable_key_spots.is_empty());
        check!(report.waypoints_reachable);
        check!(report.unreachable_waypoints.is_empty());
        check!(report.max_road_grade_pct <= 20);
        Ok(())
    }

    #[test]
    fn wg3_ridge_at_45_pct_passes_check_and_50_pct_is_reported() -> Result<(), Box<dyn Error>> {
        let make_ridge = |rise_cm: i16, cut_road: bool| -> Result<GroundDraft, Box<dyn Error>> {
            let cols = 64_u16;
            let rows = 64_u16;
            let w = usize::from(cols);
            let mut classes = vec![TerrainClass::Ground; w * usize::from(rows)];
            let mut elev = vec![0_i16; w * usize::from(rows)];
            for x in 0..w {
                if let Some(slot) = classes.get_mut(32 * w + x) {
                    *slot = TerrainClass::Road;
                }
            }
            for y in 0..usize::from(rows) {
                for x in 32..w {
                    if let Some(slot) = elev.get_mut(y * w + x) {
                        *slot = rise_cm;
                    }
                }
            }
            if cut_road {
                // Cut a <= 20% road ramp through the ridge on row 32 (0 -> 150 -> 300 -> 450).
                if let Some(slot) = elev.get_mut(32 * w + 32) {
                    *slot = 150;
                }
                if let Some(slot) = elev.get_mut(32 * w + 33) {
                    *slot = 300;
                }
            }
            Ok(GroundDraft {
                schema: DRAFT_SCHEMA.to_owned(),
                id: "ridge_test".to_owned(),
                label: "Ridge Test".to_owned(),
                biome: "mars".to_owned(),
                body: "mars".to_owned(),
                origin_m: [0, 0],
                cell_cm: 1_000,
                cols,
                rows,
                classes: encode_classes(cols, rows, &classes),
                elev: encode_elev(&elev),
                entry_zones: vec![
                    EntryZoneDef {
                        team: Team::Red,
                        x0: 0,
                        y0: 0,
                        x1: 7,
                        y1: 63,
                    },
                    EntryZoneDef {
                        team: Team::Blue,
                        x0: 56,
                        y0: 0,
                        x1: 63,
                        y1: 63,
                    },
                ],
                key_spots: Vec::new(),
                waypoints: WaypointGraph {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                },
                modes: Vec::new(),
                tanks: Vec::new(),
                atlas_version: 1,
                arena_version: 1,
                provenance: GroundProvenance {
                    source: GroundSource::Studio,
                    version: "1.0".to_owned(),
                    seed: None,
                    inputs: Vec::new(),
                },
                note: String::new(),
            })
        };

        let ridge_45 = make_ridge(450, true)?;
        let report_45 = check(&ridge_45)?;
        check!(report_45.passed);
        check!(report_45.zones_reachable_tracked);
        check!(report_45.zones_reachable_wheeled);
        check!(report_45.max_road_grade_pct <= 20);
        check_eq!(report_45.tracked_passable_pct, 100);

        let ridge_50 = make_ridge(500, false)?;
        let report_50 = check(&ridge_50)?;
        check!(!report_50.passed);
        check!(!report_50.zones_reachable_tracked);
        check!(!report_50.zones_reachable_wheeled);
        check!(report_50.tracked_passable_pct < 90);
        check!(build(&ridge_50).is_err());

        // Extractor draft with a 50% ridge is automatically relaxed on `build`,
        // its routed waypoint graph and §5 report note are written into `built.draft`,
        // and the built packs round-trip to match `built.draft`.
        let mut extractor_ridge = ridge_50;
        extractor_ridge.provenance.source = GroundSource::Extractor;
        extractor_ridge.provenance.inputs = vec!["Mars Pack v1".to_owned()];
        let built = build(&extractor_ridge)?;
        check!(!built.draft.waypoints.nodes.is_empty());
        check!(built.draft.note.contains("ground-pack check"));
        let round_trip = from_packs(&built.atlas_json, &built.arena_json)?;
        check_eq!(round_trip.biome, "mars");
        check_eq!(round_trip.waypoints, built.draft.waypoints);
        check_eq!(round_trip.note, built.draft.note);
        let after = check(&round_trip)?;
        check!(after.passed);
        check!(after.zones_reachable_tracked);
        check!(after.zones_reachable_wheeled);
        Ok(())
    }

    #[test]
    fn wg3_route_over_proving_ground_with_empty_graph_reaches_every_key_spot()
    -> Result<(), Box<dyn Error>> {
        let mut draft = proving_ground_draft()?;
        draft.waypoints.nodes.clear();
        draft.waypoints.edges.clear();
        route(&mut draft)?;
        check!(!draft.waypoints.nodes.is_empty());
        check!(!draft.waypoints.edges.is_empty());
        for spot in &draft.key_spots {
            check!(
                draft
                    .waypoints
                    .nodes
                    .iter()
                    .any(|n| n.x.abs_diff(spot.x) <= 2 && n.y.abs_diff(spot.y) <= 2)
            );
        }
        let report = check(&draft)?;
        check!(report.passed);
        check!(report.waypoints_reachable);
        Ok(())
    }

    #[test]
    fn wg3_one_cell_wall_and_studio_70_pct_cliff_survive_build() -> Result<(), Box<dyn Error>> {
        let mut draft = proving_ground_draft()?;
        draft.id = "studio_cliff".to_owned();
        draft.provenance.source = GroundSource::Studio;
        draft.modes.clear();

        let mut classes = decode_classes(&draft.id, draft.cols, draft.rows, &draft.classes)?;
        // Place a 1-cell `wall` cover pillar at (50, 50) and a 1-cell `woods_light` speckle at (55, 50).
        let w = usize::from(draft.cols);
        if let Some(slot) = classes.get_mut(50 * w + 50) {
            *slot = TerrainClass::Wall;
        }
        if let Some(slot) = classes.get_mut(50 * w + 55) {
            *slot = TerrainClass::WoodsLight;
        }
        draft.classes = encode_classes(draft.cols, draft.rows, &classes);

        // Studio build does NOT run repair passes by default: both the 70% cliff at The Bluff
        // and the 1-cell wall survive `build`.
        let built = build(&draft)?;
        let rebuilt = from_packs(&built.atlas_json, &built.arena_json)?;
        let rebuilt_classes =
            decode_classes(&rebuilt.id, rebuilt.cols, rebuilt.rows, &rebuilt.classes)?;
        let rebuilt_elev = decode_elev(&rebuilt.id, rebuilt.cols, rebuilt.rows, &rebuilt.elev)?;
        check_eq!(
            rebuilt_classes.get(50 * w + 50).copied(),
            Some(TerrainClass::Wall)
        );
        // The Bluff's 70% west face (140, 111) -> (141, 111) is 700 cm rise.
        let e_foot = rebuilt_elev.get(111 * w + 140).copied().unwrap_or(0);
        let e_top = rebuilt_elev.get(111 * w + 141).copied().unwrap_or(0);
        check_eq!((i32::from(e_top) - i32::from(e_foot)).abs(), 700);

        // Even when `despeckle` runs explicitly, the 1-cell `wall` and built classes survive,
        // while the 1-cell `woods_light` speckle is replaced by its grass neighbours.
        let mut despeckled = draft;
        let changed = despeckle(&mut despeckled, DEFAULT_SPECKLE_MIN_CELLS)?;
        check!(changed >= 1);
        let after_classes = decode_classes(
            &despeckled.id,
            despeckled.cols,
            despeckled.rows,
            &despeckled.classes,
        )?;
        check_eq!(
            after_classes.get(50 * w + 50).copied(),
            Some(TerrainClass::Wall)
        );
        check_eq!(
            after_classes.get(50 * w + 55).copied(),
            Some(TerrainClass::Grass)
        );
        Ok(())
    }

    #[test]
    fn wg4_open_draft_move_spot_build_load_and_range_plus_viewshed() -> Result<(), Box<dyn Error>> {
        // 1. Open the shipped proving ground draft through `draft_from_world`.
        let world = mush_core::World::default();
        let mut draft = draft_from_world(&world, "proving_ground")?;
        check_eq!(draft.id, "proving_ground");
        check_eq!(
            draft.key_spots.first().map(|s| (s.name.as_str(), s.x, s.y)),
            Some(("The Ford", 64, 97))
        );

        // 2. Move "The Ford" to (80, 97) (on the east road), save to JSON, re-open, and `build`.
        if let Some(ford) = draft.key_spots.first_mut() {
            ford.x = 80;
            ford.y = 97;
        }
        let saved_json = draft.to_json_pretty()?;
        let reopened = GroundDraft::from_json(&saved_json)?;
        let built = build(&reopened)?;

        // 3. Load the rebuilt atlas and arena packs into `Records` and `ArenaDefinition`
        //    and run `RANGE` (`resolve_spot` + `spot_range_cm` + `spot_range_line`) from (30, 97).
        let atlas_pack = mush_content::parse_pack(&built.atlas_json)?;
        let arena_pack = mush_content::parse_pack(&built.arena_json)?;
        let mut records = mush_atlas::record::Records::default();
        let pack_id = CanonicalId::new("tacsim.proving_ground")?;
        let region_id = CanonicalId::new("region.proving_ground")?;
        let region_raw = atlas_pack
            .entries()
            .get(&region_id)
            .ok_or("missing region")?;
        let region_def: RegionDef =
            serde_json::from_value(serde_json::Value::Object(region_raw.def().clone()))?;
        let region_key = mush_atlas::RecordKey::new(pack_id.clone(), region_id);
        let region_rec = mush_atlas::RegionRec {
            key: region_key.clone(),
            name: region_def.name.clone(),
            kind: region_def.region_kind()?,
            frame: None,
            parent: None,
            extent: region_def.extent,
            datum_cm: 0,
            background: region_def.background,
            ground: Vec::new(),
            ground_resolution_cm: region_def.ground_resolution_cm,
            theme: None,
            biome: region_def.biome,
            revision: 1,
        };
        records.regions.insert(region_key.clone(), region_rec);
        for (entry_id, raw) in atlas_pack.entries() {
            if raw.kind().as_str() == "heightfield" {
                let hf: HeightfieldDef =
                    serde_json::from_value(serde_json::Value::Object(raw.def().clone()))?;
                let samples = hf.samples()?;
                let authored = hf.class_grid()?;
                let mut cells = mush_atlas::Cells::filled(
                    hf.cols,
                    hf.rows,
                    mush_atlas::Cell::default(),
                    mush_atlas::MAX_TILE_CELLS,
                )?;
                for y in 0..hf.rows {
                    for x in 0..hf.cols {
                        let at = usize::from(y) * usize::from(hf.cols) + usize::from(x);
                        let elev_cm = samples.get(at).copied().unwrap_or(0);
                        let class = authored
                            .as_ref()
                            .and_then(|g| g.get(at).copied().flatten())
                            .unwrap_or(TerrainClass::Ground);
                        cells.set(
                            x,
                            y,
                            mush_atlas::Cell {
                                class,
                                elev_cm,
                                cover: 0,
                                room: 0,
                            },
                        );
                    }
                }
                let tile_id = CanonicalId::new(format!("{entry_id}.tile.0"))?;
                let tile_key = mush_atlas::RecordKey::new(pack_id.clone(), tile_id);
                let step_m = f64::from(hf.resolution_cm) / 100.0;
                let bbox = mush_atlas::Rect2(
                    mush_atlas::Point2(hf.origin.0, hf.origin.1),
                    mush_atlas::Point2(
                        hf.origin.0 + f64::from(hf.cols) * step_m,
                        hf.origin.1 + f64::from(hf.rows) * step_m,
                    ),
                );
                records.tiles.insert(
                    tile_key.clone(),
                    mush_atlas::TileRec {
                        key: tile_key,
                        region: region_key.clone(),
                        surface: mush_atlas::SurfaceRef::Ground,
                        owner: mush_atlas::record::TileOwner::Heightfield,
                        bbox,
                        resolution_cm: hf.resolution_cm,
                        cells,
                        rooms: Vec::new(),
                        revision: 1,
                    },
                );
            }
        }
        let arena_id = CanonicalId::new("tacsim.arena.proving_ground")?;
        let arena_raw = arena_pack.entries().get(&arena_id).ok_or("missing arena")?;
        let arena_def: ArenaDef =
            serde_json::from_value(serde_json::Value::Object(arena_raw.def().clone()))?;
        let loaded = crate::definitions::arena::from_pack(
            "tacsim.arena.proving_ground",
            &arena_def,
            &records,
        )?;
        let reassembled = draft_from_records("proving_ground", &arena_def, &records)?;
        check_eq!(reassembled.provenance.source, GroundSource::Pack);
        check_eq!(
            reassembled.provenance.inputs,
            vec![
                "tacsim.proving_ground:region.proving_ground".to_owned(),
                "tacsim.proving_ground:tacsim.arena.proving_ground".to_owned()
            ]
        );
        let reassembled_ford = reassembled
            .key_spots
            .iter()
            .find(|s| s.name == "The Ford")
            .ok_or("missing The Ford in reassembled draft")?;
        check_eq!((reassembled_ford.x, reassembled_ford.y), (80, 97));

        let (idx, ford) = crate::match_::cockpit::resolve_spot(&loaded.key_spots, "the ford")
            .ok_or("The Ford not resolved")?;
        check_eq!((ford.x, ford.y), (80, 97));
        let pilot_pose = mush_sim::data::Pose {
            x_cm: 30 * 1_000 + 500,
            y_cm: 97 * 1_000 + 500,
            z_cm: 0,
            heading_mdeg: 90_000,
            posture: mush_sim::data::passive_rules_value(serde_json::Value::Null)?,
        };
        let dist_cm =
            crate::match_::cockpit::spot_range_cm(&pilot_pose, ford, loaded.terrain.cell_cm());
        let metres = (dist_cm + crate::match_::cockpit::SPOT_RANGE_STEP_M * 100 / 2)
            / (crate::match_::cockpit::SPOT_RANGE_STEP_M * 100)
            * crate::match_::cockpit::SPOT_RANGE_STEP_M;
        let letter = crate::match_::cockpit::spot_letter(idx).ok_or("missing spot letter")?;
        let line = crate::match_::cockpit::spot_range_line(ford, letter, metres);
        // From (30, 97) to (80, 97) is 50 cells = 500 m (was 340 m at (64, 97)).
        check_eq!(line, "Range to The Ford (A): 500 m.");

        // 4. Viewshed from The Bluff (144, 111) over the shipped proving ground.
        let pg = proving_ground_draft()?;
        let vs = viewshed(&pg, 144, 111, 300)?;
        check_eq!(vs.len(), 256 * 256);
        check_eq!(vs.get(111 * 256 + 144).copied(), Some(1));
        let seen_count = vs.iter().filter(|&&v| v == 1).count();
        let hull_down_count = vs.iter().filter(|&&v| v == 2).count();
        let partial_count = vs.iter().filter(|&&v| v == 3).count();
        let dead_count = vs.iter().filter(|&&v| v == 0).count();
        check!(seen_count > 1000);
        check!(hull_down_count > 100);
        check!(partial_count > 100);
        check!(dead_count > 1000);

        let vs_classes: Vec<TerrainClass> = vs
            .iter()
            .map(|&v| TerrainClass::from_code(v).unwrap_or(TerrainClass::Void))
            .collect();
        let vs_rle_b64 = encode_classes(256, 256, &vs_classes);
        let mut pg_routed = proving_ground_draft()?;
        pg_routed.waypoints = WaypointGraph::default();
        route(&mut pg_routed)?;
        let parity_doc = serde_json::json!({
            "bluff_viewshed_144_111_300_rle_b64": vs_rle_b64,
            "routed_waypoints": pg_routed.waypoints,
        });
        let parity_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/src/lib/fixtures/proving_ground_wg4_parity.json");
        if !parity_path.exists() {
            std::fs::write(&parity_path, serde_json::to_string_pretty(&parity_doc)?)?;
        }
        let disk_parity: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&parity_path)?)?;
        check_eq!(disk_parity, parity_doc);
        Ok(())
    }

    #[test]
    fn wg5_winter_valley_and_biome_presets_and_render_png_meet_acceptance()
    -> Result<(), Box<dyn Error>> {
        // 1. Checked-in winter_valley draft builds the shipped packs byte for byte,
        //    carries extractor provenance naming the Unity packs and versions, and passes §5 check.
        let world = mush_core::World::default();
        let wv = draft_from_world(&world, "winter_valley")?;
        check_eq!(wv.id, "winter_valley");
        check_eq!(wv.provenance.source, GroundSource::Extractor);
        check_eq!(wv.provenance.inputs.len(), 3);
        check!(
            wv.provenance
                .inputs
                .iter()
                .any(|s| s.contains("Winter Pack v1.0"))
        );
        check!(
            wv.provenance
                .inputs
                .iter()
                .any(|s| s.contains("Arcane Forest Environment v1.0"))
        );
        check!(
            wv.provenance
                .inputs
                .iter()
                .any(|s| s.contains("Wasteland Environment v1.0"))
        );
        let wv_report = check(&wv)?;
        check!(wv_report.passed);
        check!(wv_report.tracked_passable_pct >= 90);
        check!(wv_report.legged_passable_pct >= 95);
        check!(wv_report.zones_reachable_tracked);
        check!(wv_report.zones_reachable_wheeled);
        check!(wv_report.zones_reachable_legged);
        check!(wv_report.key_spots_wheeled_reachable);
        check!(wv_report.waypoints_reachable);
        check!(wv_report.max_road_grade_pct <= 20);
        check_eq!(wv_report.max_entry_zone_step_cm, 0);

        let built_wv = build(&wv)?;
        check_eq!(built_wv.draft, wv);
        check_eq!(
            built_wv.atlas_json.replace("\r\n", "\n"),
            content::WINTER_VALLEY_ATLAS_JSON.replace("\r\n", "\n")
        );
        check_eq!(
            built_wv.arena_json.replace("\r\n", "\n"),
            content::WINTER_VALLEY_JSON.replace("\r\n", "\n")
        );

        // 2. winter_valley boots from the tree's packs into Records and ArenaDefinition.
        let packs = content::winter_valley_packs()?;
        check_eq!(packs.len(), 2);
        let atlas_pack = packs.first().ok_or("missing winter_valley atlas pack")?;
        let arena_pack = packs.get(1).ok_or("missing winter_valley arena pack")?;
        let mut records = mush_atlas::record::Records::default();
        let pack_id = CanonicalId::new(content::WINTER_VALLEY_ATLAS_PACK)?;
        let region_id = CanonicalId::new("region.winter_valley")?;
        let region_raw = atlas_pack
            .entries()
            .get(&region_id)
            .ok_or("missing region.winter_valley")?;
        let region_def: RegionDef =
            serde_json::from_value(serde_json::Value::Object(region_raw.def().clone()))?;
        let region_key = mush_atlas::RecordKey::new(pack_id.clone(), region_id);
        records.regions.insert(
            region_key.clone(),
            mush_atlas::RegionRec {
                key: region_key.clone(),
                name: region_def.name.clone(),
                kind: region_def.region_kind()?,
                frame: None,
                parent: None,
                extent: region_def.extent,
                datum_cm: 0,
                background: region_def.background,
                ground: Vec::new(),
                ground_resolution_cm: region_def.ground_resolution_cm,
                theme: None,
                biome: region_def.biome,
                revision: 1,
            },
        );
        for (entry_id, raw) in atlas_pack.entries() {
            if raw.kind().as_str() == "heightfield" {
                let hf: HeightfieldDef =
                    serde_json::from_value(serde_json::Value::Object(raw.def().clone()))?;
                let samples = hf.samples()?;
                let authored = hf.class_grid()?;
                let mut cells = mush_atlas::Cells::filled(
                    hf.cols,
                    hf.rows,
                    mush_atlas::Cell::default(),
                    mush_atlas::MAX_TILE_CELLS,
                )?;
                for y in 0..hf.rows {
                    for x in 0..hf.cols {
                        let at = usize::from(y) * usize::from(hf.cols) + usize::from(x);
                        let elev_cm = samples.get(at).copied().unwrap_or(0);
                        let class = authored
                            .as_ref()
                            .and_then(|g| g.get(at).copied().flatten())
                            .unwrap_or(TerrainClass::Ground);
                        cells.set(
                            x,
                            y,
                            mush_atlas::Cell {
                                class,
                                elev_cm,
                                cover: 0,
                                room: 0,
                            },
                        );
                    }
                }
                let tile_id = CanonicalId::new(format!("{entry_id}.tile.0"))?;
                let tile_key = mush_atlas::RecordKey::new(pack_id.clone(), tile_id);
                let step_m = f64::from(hf.resolution_cm) / 100.0;
                let bbox = mush_atlas::Rect2(
                    mush_atlas::Point2(hf.origin.0, hf.origin.1),
                    mush_atlas::Point2(
                        hf.origin.0 + f64::from(hf.cols) * step_m,
                        hf.origin.1 + f64::from(hf.rows) * step_m,
                    ),
                );
                records.tiles.insert(
                    tile_key.clone(),
                    mush_atlas::TileRec {
                        key: tile_key,
                        region: region_key.clone(),
                        surface: mush_atlas::SurfaceRef::Ground,
                        owner: mush_atlas::record::TileOwner::Heightfield,
                        bbox,
                        resolution_cm: hf.resolution_cm,
                        cells,
                        rooms: Vec::new(),
                        revision: 1,
                    },
                );
            }
        }
        let arena_id = CanonicalId::new(content::WINTER_VALLEY)?;
        let arena_raw = arena_pack
            .entries()
            .get(&arena_id)
            .ok_or("missing tacsim.arena.winter_valley")?;
        let arena_def: ArenaDef =
            serde_json::from_value(serde_json::Value::Object(arena_raw.def().clone()))?;
        let loaded_wv =
            crate::definitions::arena::from_pack(content::WINTER_VALLEY, &arena_def, &records)?;
        check_eq!(loaded_wv.terrain.width(), 256);
        check_eq!(loaded_wv.terrain.height(), 256);
        check_eq!(loaded_wv.key_spots.len(), 5);
        let reassembled_wv = draft_from_records("winter_valley", &arena_def, &records)?;
        check_eq!(reassembled_wv, wv);

        // 3. Procedural biome presets (`mars_canyon`, `lunar_crater`, `ice_shelf`)
        //    meet §5 check, carry disjoint `origin_m`, and author only their §6 per-biome class vocabularies.
        let mc = draft_from_world(&world, "mars_canyon")?;
        check_eq!(mc.id, "mars_canyon");
        check_eq!(mc.biome, "mars");
        check_eq!(mc.body, "mars");
        check_eq!(mc.origin_m, [40_000, 40_000]);
        check_eq!(mc, generate("mars_canyon", 2350, None)?);
        let built_mc = build(&mc)?;
        check_eq!(built_mc.draft, mc);
        check_eq!(
            built_mc.atlas_json.replace("\r\n", "\n"),
            content::MARS_CANYON_ATLAS_JSON.replace("\r\n", "\n")
        );
        check_eq!(
            built_mc.arena_json.replace("\r\n", "\n"),
            content::MARS_CANYON_JSON.replace("\r\n", "\n")
        );
        check_eq!(content::mars_canyon_packs()?.len(), 2);

        for (preset, expected_biome, expected_body, expected_origin) in [
            ("mars_canyon", "mars", "mars", 40_000_i64),
            ("lunar_crater", "lunar", "luna", 50_000_i64),
            ("ice_shelf", "ice", "europa", 60_000_i64),
        ] {
            let d = generate(preset, 42, None)?;
            check_eq!(d.biome, expected_biome);
            check_eq!(d.body, expected_body);
            check_eq!(d.origin_m, [expected_origin, expected_origin]);
            check_eq!(d.provenance.source, GroundSource::Generator);
            check_eq!(d.provenance.seed, Some(42));
            let rep = check(&d)?;
            check!(rep.passed);
            let cls = decode_classes(&d.id, d.cols, d.rows, &d.classes)?;
            // None of the non-temperate presets may author grass or woods (§6).
            check!(!cls.iter().any(|c| matches!(
                c,
                TerrainClass::Grass | TerrainClass::WoodsLight | TerrainClass::WoodsHeavy
            )));
            if expected_biome != "ice" {
                check!(
                    !cls.iter()
                        .any(|c| matches!(c, TerrainClass::Water | TerrainClass::WaterDeep))
                );
            } else {
                check!(cls.contains(&TerrainClass::Ice));
                check!(cls.contains(&TerrainClass::WaterDeep));
            }
        }

        // 4. `render_png` emits a valid PNG both plain and with a viewshed overlay.
        let png_plain = render_png(&wv, None, 300)?;
        check!(png_plain.starts_with(b"\x89PNG\r\n\x1a\n"));
        check!(png_plain.len() > 1_000);
        let png_vs = render_png(&wv, Some((144, 111)), 300)?;
        check!(png_vs.starts_with(b"\x89PNG\r\n\x1a\n"));
        check_ne!(png_plain, png_vs);
        Ok(())
    }

    #[test]
    fn wg6_512_quadrant_draft_and_steep_256_draft_fit_within_map_max_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::arena_task::MatchId;
        use crate::definitions::arena::{ArenaDef, arena_map_within_bound};
        use crate::match_::cockpit::{
            MAP_MAX_BYTES, map_frame, map_frame_for_focus, map_frame_window_bounded,
        };
        use mush_core::CanonicalId;

        // 1. Build a 512×512 draft (sixteen 128×128 quadrants) by tiling four 256×256
        //    proving_ground quadrants and scaling coordinates 2×.
        let base = proving_ground_draft()?;
        let base_elev = decode_elev(&base.id, base.cols, base.rows, &base.elev)?;
        let base_cls = decode_classes(&base.id, base.cols, base.rows, &base.classes)?;
        let (cols, rows) = (512_u16, 512_u16);
        let n = usize::from(cols) * usize::from(rows);
        let mut elev_512 = vec![0_i16; n];
        let mut cls_512 = vec![TerrainClass::Ground; n];
        for y in 0..usize::from(rows) {
            let sy = y / 2;
            for x in 0..usize::from(cols) {
                let sx = x / 2;
                let si = sy * 256 + sx;
                elev_512[y * 512 + x] = base_elev.get(si).copied().unwrap_or(0);
                cls_512[y * 512 + x] = base_cls.get(si).copied().unwrap_or(TerrainClass::Ground);
            }
        }
        let mut draft_512 = base.clone();
        draft_512.id = "mega_valley".to_owned();
        draft_512.label = "Mega Valley".to_owned();
        draft_512.cols = cols;
        draft_512.rows = rows;
        draft_512.elev = encode_elev(&elev_512);
        draft_512.classes = encode_classes(cols, rows, &cls_512);
        for z in &mut draft_512.entry_zones {
            z.x0 *= 2;
            z.y0 *= 2;
            z.x1 = z.x1.saturating_mul(2).min(511);
            z.y1 = z.y1.saturating_mul(2).min(511);
        }
        for s in &mut draft_512.key_spots {
            s.x *= 2;
            s.y *= 2;
        }
        for w in &mut draft_512.waypoints.nodes {
            w.x *= 2;
            w.y *= 2;
        }
        for t in &mut draft_512.tanks {
            t.x *= 2;
            t.y *= 2;
        }

        let built_512 = build(&draft_512)?;
        let atlas_512 = mush_content::parse_pack(&built_512.atlas_json)?;
        let arena_512 = mush_content::parse_pack(&built_512.arena_json)?;
        // 1 geographic_frame + 1 region + 16 heightfield quadrants = 18 entries, each < 64 KiB.
        check_eq!(atlas_512.entries().len(), 18);
        for (id, entry) in atlas_512.entries() {
            let entry_bytes = serde_json::to_vec(entry)?;
            check!(
                entry_bytes.len() <= mush_content::MAX_ENTRY_BYTES,
                "{id} fits in 64 KiB: {} B",
                entry_bytes.len()
            );
        }

        let mut records = mush_atlas::record::Records::default();
        let pack_id = atlas_512.manifest().id().clone();
        let region_id = CanonicalId::new("region.mega_valley")?;
        let region_raw = atlas_512
            .entries()
            .get(&region_id)
            .ok_or("missing region.mega_valley")?;
        let region_def: RegionDef =
            serde_json::from_value(serde_json::Value::Object(region_raw.def().clone()))?;
        let region_key = mush_atlas::RecordKey::new(pack_id.clone(), region_id);
        records.regions.insert(
            region_key.clone(),
            mush_atlas::RegionRec {
                key: region_key.clone(),
                name: region_def.name.clone(),
                kind: region_def.region_kind()?,
                frame: None,
                parent: None,
                extent: region_def.extent,
                datum_cm: 0,
                background: region_def.background,
                ground: Vec::new(),
                ground_resolution_cm: region_def.ground_resolution_cm,
                theme: None,
                biome: region_def.biome,
                revision: 1,
            },
        );
        for (entry_id, raw) in atlas_512.entries() {
            if raw.kind().as_str() == "heightfield" {
                let hf: HeightfieldDef =
                    serde_json::from_value(serde_json::Value::Object(raw.def().clone()))?;
                let samples = hf.samples()?;
                let authored = hf.class_grid()?;
                let mut cells = mush_atlas::Cells::filled(
                    hf.cols,
                    hf.rows,
                    mush_atlas::Cell::default(),
                    mush_atlas::MAX_TILE_CELLS,
                )?;
                for y in 0..hf.rows {
                    for x in 0..hf.cols {
                        let at = usize::from(y) * usize::from(hf.cols) + usize::from(x);
                        let elev_cm = samples.get(at).copied().unwrap_or(0);
                        let class = authored
                            .as_ref()
                            .and_then(|g| g.get(at).copied().flatten())
                            .unwrap_or(TerrainClass::Ground);
                        cells.set(
                            x,
                            y,
                            mush_atlas::Cell {
                                class,
                                elev_cm,
                                cover: 0,
                                room: 0,
                            },
                        );
                    }
                }
                let tile_id = CanonicalId::new(format!("{entry_id}.tile.0"))?;
                let tile_key = mush_atlas::RecordKey::new(pack_id.clone(), tile_id);
                let step_m = f64::from(hf.resolution_cm) / 100.0;
                let bbox = mush_atlas::Rect2(
                    mush_atlas::Point2(hf.origin.0, hf.origin.1),
                    mush_atlas::Point2(
                        hf.origin.0 + f64::from(hf.cols) * step_m,
                        hf.origin.1 + f64::from(hf.rows) * step_m,
                    ),
                );
                records.tiles.insert(
                    tile_key.clone(),
                    mush_atlas::TileRec {
                        key: tile_key,
                        region: region_key.clone(),
                        surface: mush_atlas::SurfaceRef::Ground,
                        owner: mush_atlas::record::TileOwner::Heightfield,
                        bbox,
                        resolution_cm: hf.resolution_cm,
                        cells,
                        rooms: Vec::new(),
                        revision: 1,
                    },
                );
            }
        }
        let arena_id = CanonicalId::new("tacsim.arena.mega_valley")?;
        let arena_raw = arena_512
            .entries()
            .get(&arena_id)
            .ok_or("missing tacsim.arena.mega_valley")?;
        let arena_def: ArenaDef =
            serde_json::from_value(serde_json::Value::Object(arena_raw.def().clone()))?;
        check_eq!(arena_def.tiles.len(), 16);
        let loaded_512 =
            crate::definitions::arena::from_pack("tacsim.arena.mega_valley", &arena_def, &records)?;
        check_eq!(loaded_512.terrain.width(), 512);
        check_eq!(loaded_512.terrain.height(), 512);

        // `arena_map_within_bound` succeeds and stays <= MAP_MAX_BYTES (256 KiB).
        let bound_bytes = arena_map_within_bound(
            "tacsim.arena.mega_valley",
            &loaded_512.terrain,
            &loaded_512.key_spots,
        )?;
        check!(bound_bytes <= MAP_MAX_BYTES);

        // Explicit `window` slice `[128, 128, 256, 256]` carries `origin_cell: [128, 128]`
        // and `extent: [512, 512]` and stays <= MAP_MAX_BYTES.
        let win_frame = map_frame_window_bounded(
            MatchId(1),
            "tacsim.arena.mega_valley",
            &loaded_512.terrain,
            &loaded_512.key_spots,
            (128, 128, 256, 256),
        );
        check_eq!(win_frame["origin_cell"], serde_json::json!([128, 128]));
        check_eq!(win_frame["width"], serde_json::json!(256));
        check_eq!(win_frame["height"], serde_json::json!(256));
        check_eq!(win_frame["extent"], serde_json::json!([512, 512]));
        check!(serde_json::to_vec(&win_frame)?.len() <= MAP_MAX_BYTES);

        // And `draft_from_records` round-trips all 16 quadrants' cells, elev, and furniture.
        let round_trip_512 = draft_from_records("mega_valley", &arena_def, &records)?;
        check_eq!(round_trip_512.cols, 512);
        check_eq!(round_trip_512.rows, 512);
        check_eq!(round_trip_512.classes, built_512.draft.classes);
        check_eq!(round_trip_512.elev, built_512.draft.elev);
        check_eq!(round_trip_512.entry_zones, built_512.draft.entry_zones);
        check_eq!(round_trip_512.key_spots, built_512.draft.key_spots);
        check_eq!(round_trip_512.waypoints, built_512.draft.waypoints);
        check_eq!(round_trip_512.tanks, built_512.draft.tanks);

        // 2. A steep 256×256 ground whose unwindowed `map_frame` exceeds `MAP_MAX_BYTES`
        //    automatically windows via `map_frame_for_focus` to fit within `MAP_MAX_BYTES`.
        let mut steep_cells = loaded_512.terrain.cells()[0..(256 * 256)].to_vec();
        for (i, c) in steep_cells.iter_mut().enumerate() {
            let x = (i % 256) as i16;
            let y = (i / 256) as i16;
            // Steep oscillating relief so every delta takes 3 LEB128 bytes (> 256 KiB unwindowed).
            c.elev_cm = if (x + y) % 2 == 0 { 14_000 } else { -14_000 };
        }
        let steep_terrain = crate::arena::state::ArenaTerrain::priced(
            256,
            256,
            1_000,
            steep_cells,
            mush_atlas::physics::LocomotionTable::shipped(),
        )?;
        let unwindowed = map_frame(MatchId(1), "tacsim.arena.steep", &steep_terrain, &[]);
        check!(
            serde_json::to_vec(&unwindowed)?.len() > MAP_MAX_BYTES,
            "unwindowed steep 256x256 exceeds 256 KiB"
        );
        let windowed_steep =
            map_frame_for_focus(MatchId(1), "tacsim.arena.steep", &steep_terrain, &[], None);
        let steep_bytes = serde_json::to_vec(&windowed_steep)?.len();
        check!(
            steep_bytes <= MAP_MAX_BYTES,
            "windowed steep 256x256 fits in 256 KiB: {steep_bytes} B"
        );
        check_eq!(windowed_steep["extent"], serde_json::json!([256, 256]));
        check!(
            arena_map_within_bound("tacsim.arena.steep", &steep_terrain, &[]).is_ok(),
            "arena_map_within_bound accepts the windowed steep 256x256 ground"
        );
        Ok(())
    }

    fn booted_test_world() -> Result<mush_testkit::TestWorld, Box<dyn std::error::Error>> {
        use crate::composition as comp;
        let cfg = crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        };
        let onboarding = mush_core::OnboardingConfig::default();
        let game = crate::GameEconomyConfig::default();
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.extend(comp::sheet_modules(&onboarding, &game));
        mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(false, true)?);
        Ok(mush_testkit::TestWorld::with_modules(mods))
    }

    #[test]
    fn wg7_live_staging_hot_shelves_and_applies_ground_drafts()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tw = booted_test_world()?;
        let god = mush_core::GOD;

        // 1. `@ground/check proving_ground` and `@ground/draft proving_ground`.
        let ev = tw.run(god, "@ground/check proving_ground");
        let check_out = tw.seen_by(&ev, god).join("\n");
        check!(
            check_out.contains("[PASS] ground `proving_ground`"),
            "unexpected @ground/check output: {check_out}"
        );

        let ev = tw.run(god, "@ground/draft proving_ground");
        let draft_out = tw.seen_by(&ev, god).join("\n");
        check!(
            draft_out.contains("GroundDraft `proving_ground`"),
            "unexpected @ground/draft output: {draft_out}"
        );

        // 2. `@ground/stage mars_canyon` hot-shelves onto `TacsimStore` and `AtlasState.shelf`
        //    without applying yet, and prints the `@atlas/pack apply` fingerprints.
        let ev = tw.run(god, "@ground/stage mars_canyon");
        let stage_out = tw.seen_by(&ev, god).join("\n");
        check!(
            stage_out.contains("Staged ground `mars_canyon` on shelf"),
            "unexpected @ground/stage output: {stage_out}"
        );

        // Even before `@atlas/pack apply`, `draft_from_world` serves the staged draft.
        let shelved_draft = draft_from_world(tw.world(), "mars_canyon")?;
        check_eq!(shelved_draft.id, "mars_canyon");
        check_eq!(shelved_draft.biome, "mars");

        // Now apply `mars_canyon` via `@ground/stage/apply mars_canyon` and verify
        // `definition_for` and its 12 roster tanks are live.
        let ev = tw.run(god, "@ground/stage/apply mars_canyon");
        let apply_out = tw.seen_by(&ev, god).join("\n");
        check!(
            apply_out.contains("Staged and applied ground `mars_canyon`"),
            "unexpected @ground/stage/apply output: {apply_out}"
        );
        let mars_id = smol_str::SmolStr::new("tacsim.arena.mars_canyon");
        let mars_loaded = crate::match_::definition_for(tw.world(), &mars_id)?
            .ok_or("missing tacsim.arena.mars_canyon")?;
        check_eq!(mars_loaded.terrain.width(), 256);
        check_eq!(mars_loaded.terrain.height(), 256);
        let mars_entry = CanonicalId::new("tacsim.arena.mars_canyon")?;
        let mars_roster_len = tw
            .world()
            .module_state::<crate::store::TacsimStore>()
            .and_then(|s| s.arena_roster(&mars_entry))
            .map_or(0, |r| r.units.len());
        check_eq!(mars_roster_len, 12);

        // 3. Multi-chunk streaming (`@ground/stage/begin` .. `/chunk` .. `/end/apply`) on an
        //    edited `proving_ground` draft auto-bumps `atlas_version` (3 -> 4) and
        //    `arena_version` (5 -> 6) and updates the live world in place.
        let mut edited = proving_ground_draft()?;
        let mut elev = decode_elev(&edited.id, edited.cols, edited.rows, &edited.elev)?;
        elev[120 * usize::from(edited.cols) + 129] += 50;
        edited.elev = encode_elev(&elev);
        edited.key_spots.push(KeySpot {
            name: "Studio Ridge".to_owned(),
            x: 129,
            y: 120,
        });
        let json = serde_json::to_string(&edited)?;
        let _ = tw.run(god, "@ground/stage/begin");
        for chunk in json.as_bytes().chunks(4000) {
            let s = std::str::from_utf8(chunk)?;
            let _ = tw.run(god, &format!("@ground/stage/chunk {s}"));
        }
        let ev = tw.run(god, "@ground/stage/end/apply");
        let end_out = tw.seen_by(&ev, god).join("\n");
        check!(
            end_out.contains("Staged and applied ground `proving_ground`: tacsim.proving_ground@4")
                && end_out.contains("tacsim.pack.proving_ground@6"),
            "expected version auto-bump to @4 / @6, got: {end_out}"
        );

        let pg_id = smol_str::SmolStr::new("tacsim.arena.proving_ground");
        let updated_loaded = crate::match_::definition_for(tw.world(), &pg_id)?
            .ok_or("missing tacsim.arena.proving_ground")?;
        check!(
            updated_loaded
                .key_spots
                .iter()
                .any(|s| s.name == "Studio Ridge" && s.x == 129 && s.y == 120),
            "live proving_ground should include newly staged key spot Studio Ridge"
        );
        let round_trip = draft_from_world(tw.world(), "proving_ground")?;
        check_eq!(round_trip.atlas_version, 4);
        check_eq!(round_trip.arena_version, 6);
        check!(
            round_trip
                .key_spots
                .iter()
                .any(|s| s.name == "Studio Ridge")
        );
        Ok(())
    }
}
