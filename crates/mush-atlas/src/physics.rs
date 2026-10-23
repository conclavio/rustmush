//! # Engine physics (doc 14 §5.1–§5.2; plan §3.6, S5): the vocabularies and
//! the locomotion table
//!
//! The engine owns the words: the **surface classes** ([`TerrainClass`], each
//! with a default [`Cover`], [`Opacity`] and medium), the **locomotion
//! classes** (the atoms `mush-tactics`'s `MobilityVocabulary::base` declared —
//! `legged`, `wheeled`, `tracked`, `hover`, `rotor`, `fixed_wing`,
//! `displacement`, `thrust` — plus `biped`, the pedestrian default, and the
//! tags `amphibious`, `jump`, `vtol`, `submersible`), and the **media**
//! `ground`, `air`, `water`, `space`. The **locomotion table** is data
//! (`content/locomotion.json`, digest-pinned by a test): `passable`, `cost`
//! and slope limits per class and locomotion. A ruleset may add a locomotion
//! or a class; it may not change a core row ([`LocomotionTable::extend`]
//! refuses one — the X lens).
//!
//! The queries here — [`move_cost`] and [`path`] — are pure over a
//! [`SurfaceComposite`], constant work per cell visited, plus one pass over the
//! grid per search (the search's scratch and the heuristic's floor), and read
//! the same records whether they succeed or fail. The traces are
//! [`crate::trace`].
//!
//! **Cut AP** (doc 20 §8's drive planner, owed by SM.4b): [`path`] resolves its
//! locomotion's rows once per search into a dense [`LocomotionCosts`] row indexed
//! by [`TerrainClass::code`] — one step rule with [`move_cost`], two lookups —
//! and steers by the octile distance priced at the cheapest passable class
//! present on the surface, an admissible and consistent heuristic, so it returns
//! the least cost. For a row whose every passable class costs at least 100
//! (every shipped row but `wheeled`, the drive's `tracked` among them) that is
//! the cost the old `Chebyshev × 100` search returned, and the new heuristic
//! dominates the old there, so the search visits no more cells (up to ties)
//! and usually far fewer. A row pricing a class under 100 (`wheeled`: paved 90, road 80;
//! a ruleset's extension may add one) was over-estimated by the old heuristic,
//! which could return a dearer path; it now gets the least (A2 against a plain
//! Dijkstra), possibly cheaper than before and possibly after visiting more
//! cells, so a tight budget may now answer `OverBudget` where it found one.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::cells::{Cell, Cells};
use crate::terrain::TerrainClass;
use crate::window::SurfaceComposite;

/// The table, as shipped.
pub const LOCOMOTION_JSON: &str = include_str!("../content/locomotion.json");

/// The shipped table's digest (`DefinitionDigest::for_canonical_value` under the
/// domain `atlas.locomotion`): an edit to the table changes it, and the test
/// that pins it fails until the edit is reviewed and the pin moved.
///
/// **16b cut AE.1** moved it: the `wheeled` row prices shallow `water` at 400
/// (a ford is where wheels cross, at five times a road) where it was `null`.
/// Was `13c8bf1b438a00bb7d196d4904c1b1a4a653e8973e86a2fbe735b3a797d234fc`.
pub const LOCOMOTION_DIGEST: &str =
    "9e80c09cf10236713079efcb338723f8ded12850820d2f78c0ebacc85e1d4930";

/// The shipped table's digest as computed now.
///
/// # Errors
///
/// When the table is not JSON.
pub fn locomotion_digest() -> Result<String, PhysicsError> {
    let value: serde_json::Value =
        serde_json::from_str(LOCOMOTION_JSON).map_err(|e| PhysicsError::Table(e.to_string()))?;
    mush_core::DefinitionDigest::for_canonical_value("atlas.locomotion", &value)
        .map(mush_core::DefinitionDigest::to_hex)
        .map_err(|e| PhysicsError::Table(e.to_string()))
}

/// The base media, in `MobilityVocabulary::base`'s order.
pub const BASE_MEDIA: [&str; 4] = ["ground", "air", "water", "space"];

/// The base locomotion classes: the mobility atoms, then `biped`.
pub const BASE_LOCOMOTIONS: [&str; 9] = [
    "legged",
    "wheeled",
    "tracked",
    "hover",
    "rotor",
    "fixed_wing",
    "displacement",
    "thrust",
    "biped",
];

/// The base capability tags.
pub const BASE_TAGS: [&str; 4] = ["amphibious", "jump", "vtol", "submersible"];

/// The pedestrian default.
pub const BIPED: &str = "biped";

/// A cost in hundredths of a cell at nominal speed: `100` is one flat cell.
pub type Cost = u32;

/// The cost of one flat cell on the best ground.
pub const NOMINAL_COST: Cost = 100;

/// The diagonal factor, hundredths (√2).
pub const DIAGONAL_PCT: u32 = 141;

/// A wall's height above its cell, centimetres, for the arcs and the eyes.
pub const WALL_HEIGHT_CM: i32 = 300;

/// The most cells one [`path`] visits by default (plan §7 `path_max_cells`).
pub const DEFAULT_PATH_MAX_CELLS: u32 = 4_096;

/// How much a cell hides what stands on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cover {
    /// Nothing.
    #[default]
    None,
    /// Some: a low wall, rubble, trees.
    Partial,
    /// All: a wall.
    Full,
}

/// How much a cell stops sight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Opacity {
    /// Sight passes.
    #[default]
    Clear,
    /// Sight passes attenuated by this percent per cell; a line whose
    /// attenuation reaches a hundred is blocked.
    Partial(u8),
    /// Sight stops.
    Opaque,
}

/// A class's engine defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceProps {
    /// The cover it gives.
    pub cover: Cover,
    /// The sight it stops.
    pub opacity: Opacity,
    /// The medium above it.
    pub medium: &'static str,
}

/// The engine defaults of a class (doc 14 §5.1).
#[must_use]
pub fn props(class: TerrainClass) -> SurfaceProps {
    let (cover, opacity, medium) = match class {
        TerrainClass::Void => (Cover::Full, Opacity::Opaque, "ground"),
        TerrainClass::Wall => (Cover::Full, Opacity::Opaque, "ground"),
        TerrainClass::Rubble | TerrainClass::Rock => (Cover::Partial, Opacity::Clear, "ground"),
        TerrainClass::WoodsLight => (Cover::Partial, Opacity::Partial(25), "ground"),
        TerrainClass::WoodsHeavy => (Cover::Partial, Opacity::Partial(60), "ground"),
        TerrainClass::Water | TerrainClass::WaterDeep => (Cover::None, Opacity::Clear, "water"),
        TerrainClass::Regolith => (Cover::None, Opacity::Clear, "space"),
        TerrainClass::Paved
        | TerrainClass::Ground
        | TerrainClass::Grass
        | TerrainClass::Floor
        | TerrainClass::Road
        | TerrainClass::Sand
        | TerrainClass::Door
        | TerrainClass::Platform
        | TerrainClass::Ice => (Cover::None, Opacity::Clear, "ground"),
    };
    SurfaceProps {
        cover,
        opacity,
        medium,
    }
}

/// The top of what stands on a cell, centimetres above its elevation: a wall's
/// height, else nothing.
#[must_use]
pub fn obstacle_height_cm(class: TerrainClass) -> i32 {
    match class {
        TerrainClass::Wall | TerrainClass::Void => WALL_HEIGHT_CM,
        TerrainClass::WoodsHeavy => 800,
        TerrainClass::WoodsLight => 600,
        _ => 0,
    }
}

/// Why the table refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PhysicsError {
    /// The shipped table would not parse.
    #[error("the locomotion table would not parse: {0}")]
    Table(String),
    /// An extension row names a core class and a core locomotion.
    #[error(
        "{class}/{locomotion} is a core row; a ruleset adds classes and locomotions, never changes a row"
    )]
    CoreRow {
        /// The class.
        class: SmolStr,
        /// The locomotion.
        locomotion: SmolStr,
    },
    /// An unknown locomotion.
    #[error("no such locomotion: {0}")]
    NoLocomotion(SmolStr),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocomotionRow {
    default: Option<Cost>,
    max_slope_pct: u32,
    #[serde(default)]
    climb_cm: i32,
    #[serde(default)]
    classes: BTreeMap<String, Option<Cost>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TagRow {
    #[serde(default)]
    classes: BTreeMap<String, Option<Cost>>,
    #[serde(default)]
    slope_factor: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableWire {
    version: u32,
    #[serde(default)]
    note: String,
    locomotions: BTreeMap<String, LocomotionRow>,
    #[serde(default)]
    tags: BTreeMap<String, TagRow>,
}

/// One locomotion's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocomotionRows {
    /// The cost of a class the rows do not name; `None` impassable.
    pub default: Option<Cost>,
    /// The slope past which a step is impassable, percent rise over run.
    pub max_slope_pct: u32,
    /// The step this locomotion climbs onto built ground whatever the slope
    /// (a kerb, a doorstep, a dock), centimetres; `0` climbs nothing.
    pub climb_cm: i32,
    /// The named classes.
    pub classes: BTreeMap<SmolStr, Option<Cost>>,
}

/// A tag's amendments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagRows {
    /// Classes the tag opens or re-prices.
    pub classes: BTreeMap<SmolStr, Option<Cost>>,
    /// A multiplier on the slope limit.
    pub slope_factor: Option<u32>,
}

/// The locomotion table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocomotionTable {
    /// The table version.
    pub version: u32,
    /// Rows per locomotion.
    pub locomotions: BTreeMap<SmolStr, LocomotionRows>,
    /// Amendments per tag.
    pub tags: BTreeMap<SmolStr, TagRows>,
    /// The locomotions a ruleset added.
    pub extension_locomotions: Vec<SmolStr>,
    /// The classes a ruleset added.
    pub extension_classes: Vec<SmolStr>,
}

impl LocomotionTable {
    /// Parse a table from its JSON.
    ///
    /// # Errors
    ///
    /// [`PhysicsError::Table`] when the JSON is not a table.
    pub fn parse(json: &str) -> Result<LocomotionTable, PhysicsError> {
        let wire: TableWire =
            serde_json::from_str(json).map_err(|e| PhysicsError::Table(e.to_string()))?;
        let _ = wire.note;
        Ok(LocomotionTable {
            version: wire.version,
            locomotions: wire
                .locomotions
                .into_iter()
                .map(|(k, r)| {
                    (
                        SmolStr::new(k),
                        LocomotionRows {
                            default: r.default,
                            max_slope_pct: r.max_slope_pct,
                            climb_cm: r.climb_cm,
                            classes: r
                                .classes
                                .into_iter()
                                .map(|(c, v)| (SmolStr::new(c), v))
                                .collect(),
                        },
                    )
                })
                .collect(),
            tags: wire
                .tags
                .into_iter()
                .map(|(k, t)| {
                    (
                        SmolStr::new(k),
                        TagRows {
                            classes: t
                                .classes
                                .into_iter()
                                .map(|(c, v)| (SmolStr::new(c), v))
                                .collect(),
                            slope_factor: t.slope_factor,
                        },
                    )
                })
                .collect(),
            extension_locomotions: Vec::new(),
            extension_classes: Vec::new(),
        })
    }

    /// The shipped table.
    #[must_use]
    pub fn shipped() -> &'static LocomotionTable {
        static TABLE: OnceLock<LocomotionTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            LocomotionTable::parse(LOCOMOTION_JSON).unwrap_or_else(|error| {
                tracing::error!(%error, "atlas: the shipped locomotion table is broken");
                LocomotionTable {
                    version: 0,
                    locomotions: BTreeMap::new(),
                    tags: BTreeMap::new(),
                    extension_locomotions: Vec::new(),
                    extension_classes: Vec::new(),
                }
            })
        })
    }

    /// Whether `class` is one of the engine's.
    #[must_use]
    pub fn is_core_class(class: &str) -> bool {
        TerrainClass::parse(class).is_some()
    }

    /// Whether `locomotion` is one of the engine's.
    #[must_use]
    pub fn is_core_locomotion(locomotion: &str) -> bool {
        BASE_LOCOMOTIONS.contains(&locomotion)
    }

    /// Add a ruleset's row: a new class for any locomotion, or a new
    /// locomotion for any class (a new locomotion's first row makes it, with
    /// `default` impassable and the slope limit given).
    ///
    /// # Errors
    ///
    /// [`PhysicsError::CoreRow`] when both the class and the locomotion are the
    /// engine's: a ruleset cannot change a core row (doc 14 §5.1).
    pub fn extend(
        &mut self,
        class: &str,
        locomotion: &str,
        cost: Option<Cost>,
        max_slope_pct: u32,
    ) -> Result<(), PhysicsError> {
        if LocomotionTable::is_core_class(class) && LocomotionTable::is_core_locomotion(locomotion)
        {
            return Err(PhysicsError::CoreRow {
                class: SmolStr::new(class),
                locomotion: SmolStr::new(locomotion),
            });
        }
        let key = SmolStr::new(locomotion);
        let rows = self
            .locomotions
            .entry(key.clone())
            .or_insert_with(|| LocomotionRows {
                default: None,
                max_slope_pct,
                climb_cm: 0,
                classes: BTreeMap::new(),
            });
        rows.classes.insert(SmolStr::new(class), cost);
        if !LocomotionTable::is_core_locomotion(locomotion)
            && !self.extension_locomotions.contains(&key)
        {
            self.extension_locomotions.push(key);
        }
        if !LocomotionTable::is_core_class(class) {
            let c = SmolStr::new(class);
            if !self.extension_classes.contains(&c) {
                self.extension_classes.push(c);
            }
        }
        Ok(())
    }

    /// The flat cost of `class` for `locomotion` with `tags`; `None` impassable.
    #[must_use]
    pub fn cost(&self, class: &str, locomotion: &str, tags: &[&str]) -> Option<Cost> {
        let rows = self.locomotions.get(locomotion)?;
        let base = rows.classes.get(class).copied().unwrap_or(rows.default);
        // A tag opens or re-prices a class: the cheapest word wins.
        let mut best = base;
        for tag in tags {
            if let Some(t) = self.tags.get(*tag)
                && let Some(v) = t.classes.get(class)
            {
                best = match (best, *v) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (None, b) => b,
                    (a, None) => a,
                };
            }
        }
        best
    }

    /// Whether `class` is passable for `locomotion` with `tags`.
    #[must_use]
    pub fn passable(&self, class: &str, locomotion: &str, tags: &[&str]) -> bool {
        self.cost(class, locomotion, tags).is_some()
    }

    /// The step `locomotion` climbs onto built ground, centimetres.
    #[must_use]
    pub fn climb_cm(&self, locomotion: &str) -> i32 {
        self.locomotions.get(locomotion).map_or(0, |r| r.climb_cm)
    }

    /// The slope limit of `locomotion` with `tags`, percent.
    #[must_use]
    pub fn max_slope_pct(&self, locomotion: &str, tags: &[&str]) -> Option<u32> {
        let rows = self.locomotions.get(locomotion)?;
        let mut limit = rows.max_slope_pct;
        for tag in tags {
            if let Some(f) = self.tags.get(*tag).and_then(|t| t.slope_factor) {
                limit = limit.saturating_mul(f);
            }
        }
        Some(limit)
    }
}

/// Which way a step goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Along an axis.
    Orthogonal,
    /// Across a corner.
    Diagonal,
}

impl StepKind {
    /// Between two cells; `None` when they are not neighbours.
    #[must_use]
    pub fn between(from: (u16, u16), to: (u16, u16)) -> Option<StepKind> {
        let dx = i32::from(from.0).abs_diff(i32::from(to.0));
        let dy = i32::from(from.1).abs_diff(i32::from(to.1));
        match (dx, dy) {
            (1, 0) | (0, 1) => Some(StepKind::Orthogonal),
            (1, 1) => Some(StepKind::Diagonal),
            _ => None,
        }
    }
}

/// Why a step is impassable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Impassable {
    /// The class refuses the locomotion.
    Class(TerrainClass),
    /// Too steep: the slope, percent, and the limit.
    Slope {
        /// The rise over run, percent.
        slope_pct: u32,
        /// The limit.
        limit_pct: u32,
    },
    /// No such locomotion in the table.
    NoLocomotion,
    /// The cells are not neighbours.
    NotAdjacent,
}

impl std::fmt::Display for Impassable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Impassable::Class(c) => write!(f, "{} is in the way", c.label()),
            Impassable::Slope { slope_pct, .. } => write!(f, "too steep ({slope_pct}%)"),
            Impassable::NoLocomotion => write!(f, "no such locomotion"),
            Impassable::NotAdjacent => write!(f, "not a step"),
        }
    }
}

/// The surcharge of a climb, percent of the destination's cost.
pub const CLIMB_SURCHARGE_PCT: u32 = 50;

/// Whether a class is built ground a step climbs onto: a kerb, a doorstep, a
/// floor, a dock.
#[must_use]
pub fn is_built(class: TerrainClass) -> bool {
    matches!(
        class,
        TerrainClass::Paved
            | TerrainClass::Road
            | TerrainClass::Floor
            | TerrainClass::Door
            | TerrainClass::Platform
    )
}

/// The rise over run of a step, percent.
#[must_use]
pub fn slope_pct(from: Cell, to: Cell, kind: StepKind, cell_cm: u32) -> u32 {
    let rise = i32::from(to.elev_cm).saturating_sub(i32::from(from.elev_cm));
    if rise <= 0 {
        return 0;
    }
    let run = match kind {
        StepKind::Orthogonal => i64::from(cell_cm),
        StepKind::Diagonal => i64::from(cell_cm) * i64::from(DIAGONAL_PCT) / 100,
    }
    .max(1);
    u32::try_from(i64::from(rise) * 100 / run).unwrap_or(u32::MAX)
}

/// The cost of one step from `from` onto `to` for `locomotion` (doc 14 §5.2
/// `move_cost`): the destination class's row, the diagonal factor, and the
/// slope's surcharge up to the limit.
///
/// # Errors
///
/// Why the step is impassable.
pub fn move_cost(
    table: &LocomotionTable,
    from: Cell,
    to: Cell,
    kind: StepKind,
    cell_cm: u32,
    locomotion: &str,
    tags: &[&str],
) -> Result<Cost, Impassable> {
    let limit = table
        .max_slope_pct(locomotion, tags)
        .ok_or(Impassable::NoLocomotion)?;
    let base = table
        .cost(to.class.label(), locomotion, tags)
        .ok_or(Impassable::Class(to.class))?;
    price_step(
        base,
        limit,
        table.climb_cm(locomotion),
        from,
        to,
        kind,
        cell_cm,
    )
}

/// The price of one step once its row is known (cut AP): the climb, the slope and its limit,
/// the diagonal factor, the surcharges — the one rule [`move_cost`] and
/// [`LocomotionCosts::step`] both read, so they cannot drift.
fn price_step(
    base: Cost,
    limit_pct: u32,
    climb_cm: i32,
    from: Cell,
    to: Cell,
    kind: StepKind,
    cell_cm: u32,
) -> Result<Cost, Impassable> {
    let rise = i32::from(to.elev_cm).saturating_sub(i32::from(from.elev_cm));
    // A step up onto built ground within the locomotion's climb is a climb,
    // not a slope: a kerb, a doorstep, a loading dock.
    let climb = rise > 0 && rise <= climb_cm && is_built(to.class);
    let slope = if climb {
        0
    } else {
        slope_pct(from, to, kind, cell_cm)
    };
    if slope > limit_pct {
        return Err(Impassable::Slope {
            slope_pct: slope,
            limit_pct,
        });
    }
    let mut cost = u64::from(base);
    if kind == StepKind::Diagonal {
        cost = cost * u64::from(DIAGONAL_PCT) / 100;
    }
    cost = cost * (100 + u64::from(slope)) / 100;
    if climb {
        cost = cost * (100 + u64::from(CLIMB_SURCHARGE_PCT)) / 100;
    }
    Ok(Cost::try_from(cost).unwrap_or(Cost::MAX))
}

/// How many surface classes the engine has: `TerrainClass::ALL.len()` (18).
pub const CLASS_COUNT: usize = TerrainClass::ALL.len();

/// **One locomotion's rows, resolved once** (cut AP): the flat cost of every [`TerrainClass`]
/// for one locomotion and tag set, indexed by [`TerrainClass::code`], with the locomotion's
/// slope limit and climb — what [`move_cost`] reads through three string-keyed map walks per
/// step, read here by an array index. The table has no per-class slope limit
/// ([`LocomotionRows::max_slope_pct`] is per locomotion, and a tag multiplies it), so the limit
/// and the climb are held once. Extension classes never reach a cell (a [`Cell`] carries a
/// `TerrainClass`), so the 18 engine classes are the whole domain. The step's arithmetic is
/// [`move_cost`]'s own (one private `price_step` both call), so the two lookups price every step
/// alike (`ap_the_dense_row_prices_every_step_as_move_cost_does`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocomotionCosts {
    costs: [Option<Cost>; CLASS_COUNT],
    max_slope_pct: u32,
    climb_cm: i32,
}

impl LocomotionCosts {
    /// Resolve `locomotion` with `tags`: each class's [`LocomotionTable::cost`] under its label
    /// (the tag merge reused, never re-derived), the tags' slope limit
    /// ([`LocomotionTable::max_slope_pct`]), the climb ([`LocomotionTable::climb_cm`]).
    ///
    /// # Errors
    ///
    /// [`Impassable::NoLocomotion`] when the table has no such locomotion (what [`move_cost`]
    /// answers for every step).
    pub fn resolve(
        table: &LocomotionTable,
        locomotion: &str,
        tags: &[&str],
    ) -> Result<LocomotionCosts, Impassable> {
        let max_slope_pct = table
            .max_slope_pct(locomotion, tags)
            .ok_or(Impassable::NoLocomotion)?;
        let mut costs = [None; CLASS_COUNT];
        for class in TerrainClass::ALL {
            if let Some(slot) = costs.get_mut(usize::from(class.code())) {
                *slot = table.cost(class.label(), locomotion, tags);
            }
        }
        Ok(LocomotionCosts {
            costs,
            max_slope_pct,
            climb_cm: table.climb_cm(locomotion),
        })
    }

    /// Every class impassable: an unknown locomotion's search, which then answers exactly what
    /// the string-keyed search answered (nothing past the start, `visited` counted the same).
    #[must_use]
    pub fn impassable() -> LocomotionCosts {
        LocomotionCosts {
            costs: [None; CLASS_COUNT],
            max_slope_pct: 0,
            climb_cm: 0,
        }
    }

    /// The flat cost of `class`; `None` impassable.
    #[must_use]
    pub fn cost(&self, class: TerrainClass) -> Option<Cost> {
        self.costs.get(usize::from(class.code())).copied().flatten()
    }

    /// The cheapest passable flat cost among the classes present in `cells`; `None` when no
    /// cell is passable. One pass over the grid.
    #[must_use]
    pub fn cheapest_on(&self, cells: &Cells) -> Option<Cost> {
        let mut present = [false; CLASS_COUNT];
        for cell in cells.as_slice() {
            if let Some(slot) = present.get_mut(usize::from(cell.class.code())) {
                *slot = true;
            }
        }
        TerrainClass::ALL
            .into_iter()
            .filter(|class| {
                present
                    .get(usize::from(class.code()))
                    .copied()
                    .unwrap_or(false)
            })
            .filter_map(|class| self.cost(class))
            .min()
    }

    /// One step's price: exactly [`move_cost`] with the rows this was resolved from.
    ///
    /// # Errors
    ///
    /// [`Impassable::Class`] or [`Impassable::Slope`], as [`move_cost`] would.
    pub fn step(
        &self,
        from: Cell,
        to: Cell,
        kind: StepKind,
        cell_cm: u32,
    ) -> Result<Cost, Impassable> {
        let base = self.cost(to.class).ok_or(Impassable::Class(to.class))?;
        price_step(
            base,
            self.max_slope_pct,
            self.climb_cm,
            from,
            to,
            kind,
            cell_cm,
        )
    }
}

/// A path over a surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    /// The cells, `from` first, `to` last.
    pub cells: Vec<(u16, u16)>,
    /// The cost of walking it.
    pub cost: Cost,
    /// Cells the search visited.
    pub visited: u32,
}

/// What a path search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathOutcome {
    /// A path.
    Found(Path),
    /// No path: the destination is not reachable for this locomotion.
    Unreachable {
        /// Cells the search visited before it ran out.
        visited: u32,
    },
    /// The search hit its visit budget.
    OverBudget {
        /// The budget.
        budget: u32,
    },
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// The octile distance from `from` to `to` priced at `orth` a straight step and
/// `⌊orth × 141 / 100⌋` a diagonal one (cut AP): [`path`]'s heuristic.
fn octile_floor(orth: u64, from: (u16, u16), to: (u16, u16)) -> u64 {
    let diag = orth * u64::from(DIAGONAL_PCT) / 100;
    let dx = u64::from(from.0.abs_diff(to.0));
    let dy = u64::from(from.1.abs_diff(to.1));
    let (lo, hi) = (dx.min(dy), dx.max(dy));
    diag * lo + orth * (hi - lo)
}

/// A bounded A* over `surface` (doc 14 §5.2 `path`): eight-connected, the
/// table's costs, at most `budget` cells visited, deterministic (ties break
/// by cost then by cell). Units are not obstacles: a path is geometry.
///
/// **Cut AP.** The rows are resolved once per search ([`LocomotionCosts`]; an
/// unknown locomotion resolves to [`LocomotionCosts::impassable`], so the
/// search answers exactly what the string-keyed one did). The heuristic is the
/// octile distance priced at `orth`, the cheapest passable flat cost among the
/// classes **present on the surface** ([`LocomotionCosts::cheapest_on`], one
/// pass per search), and `diag = ⌊orth × 141 / 100⌋`. It is **admissible and
/// consistent**: a step can only enter a cell on the surface, so every straight
/// step costs at least `orth`; every diagonal step costs at least
/// `⌊orth × 141 / 100⌋ = diag`, because the diagonal factor and every
/// surcharge are monotone floors over a base of at least `orth`, and slope and
/// climb only raise; and `orth ≤ diag ≤ 2 × orth`, so the octile distance is
/// the exact least cost on an open grid priced at those minima — a metric —
/// hence `h(n) ≤ c(n, n′) + h(n′)` and `h(goal) = 0`. A consistent heuristic
/// with a closed set returns the least cost. Where every passable class of the
/// row costs at least 100 (every shipped row but `wheeled`; the drive's
/// `tracked`) the old `Chebyshev × 100` was admissible too, so no such answer's
/// cost moves. A row with a class under 100 (`wheeled`'s paved 90 and road 80,
/// or an extension's) was over-estimated by the old heuristic and could get a
/// dearer path; it now gets the least, which may be cheaper than the old answer
/// and may visit more cells (so a tight `budget` may answer `OverBudget`). The
/// floor is the cheapest class present, not the row's cheapest: the tracked
/// row's cheapest is paved and road at 100, but the lane's sites hold only
/// regolith (150), ice (200) and rock (300), and the old `Chebyshev × 100`
/// therefore steered barely at all. The arena's own search does the same
/// against its window's cheapest cell (`tacsim::arena::path`'s
/// `min_move_cost`).
#[must_use]
pub fn path(
    surface: &SurfaceComposite,
    table: &LocomotionTable,
    from: (u16, u16),
    to: (u16, u16),
    locomotion: &str,
    tags: &[&str],
    budget: u32,
) -> PathOutcome {
    let w = usize::from(surface.grid.width);
    let h = usize::from(surface.grid.height);
    let idx = |x: u16, y: u16| usize::from(y) * w + usize::from(x);
    if usize::from(from.0) >= w
        || usize::from(from.1) >= h
        || usize::from(to.0) >= w
        || usize::from(to.1) >= h
    {
        return PathOutcome::Unreachable { visited: 0 };
    }
    let Some(start) = surface.cells.get(from.0, from.1) else {
        return PathOutcome::Unreachable { visited: 0 };
    };
    let cell_cm = u32::from(surface.grid.resolution_cm);
    // Cut AP: the rows once per search, and the heuristic's floor once per search.
    let costs = LocomotionCosts::resolve(table, locomotion, tags)
        .unwrap_or_else(|_| LocomotionCosts::impassable());
    let orth = u64::from(costs.cheapest_on(&surface.cells).unwrap_or(0));
    let heuristic = |x: u16, y: u16| -> u64 { octile_floor(orth, (x, y), to) };
    // The search's tables, one slot per cell; every read and write goes
    // through `get`, so a bad index is a skipped cell, never a panic.
    let mut best: Vec<u64> = vec![u64::MAX; w * h];
    let mut parent: Vec<Option<(u16, u16)>> = vec![None; w * h];
    let mut closed: Vec<bool> = vec![false; w * h];
    let mut open: BinaryHeap<Reverse<(u64, u64, u16, u16)>> = BinaryHeap::new();
    if let Some(b) = best.get_mut(idx(from.0, from.1)) {
        *b = 0;
    }
    open.push(Reverse((heuristic(from.0, from.1), 0, from.1, from.0)));
    let mut visited = 0u32;
    let _ = start;
    while let Some(Reverse((_, g, y, x))) = open.pop() {
        let i = idx(x, y);
        match closed.get_mut(i) {
            Some(true) | None => continue,
            Some(c) => *c = true,
        }
        visited += 1;
        if (x, y) == to {
            let mut cells = vec![(x, y)];
            let mut cur = (x, y);
            while let Some(Some(p)) = parent.get(idx(cur.0, cur.1)).copied() {
                cells.push(p);
                cur = p;
                if cells.len() > w * h {
                    break;
                }
            }
            cells.reverse();
            return PathOutcome::Found(Path {
                cells,
                cost: Cost::try_from(g).unwrap_or(Cost::MAX),
                visited,
            });
        }
        if visited >= budget {
            return PathOutcome::OverBudget { budget };
        }
        let Some(here) = surface.cells.get(x, y) else {
            continue;
        };
        for (dx, dy) in NEIGHBOURS {
            let nx = i32::from(x) + dx;
            let ny = i32::from(y) + dy;
            if nx < 0
                || ny < 0
                || nx >= i32::from(surface.grid.width)
                || ny >= i32::from(surface.grid.height)
            {
                continue;
            }
            let (Ok(nx), Ok(ny)) = (u16::try_from(nx), u16::try_from(ny)) else {
                continue;
            };
            let ni = idx(nx, ny);
            if closed.get(ni).copied().unwrap_or(true) {
                continue;
            }
            let Some(next) = surface.cells.get(nx, ny) else {
                continue;
            };
            let kind = if dx != 0 && dy != 0 {
                StepKind::Diagonal
            } else {
                StepKind::Orthogonal
            };
            let Ok(step) = costs.step(here, next, kind, cell_cm) else {
                continue;
            };
            let ng = g + u64::from(step);
            if let (Some(b), Some(p)) = (best.get_mut(ni), parent.get_mut(ni))
                && ng < *b
            {
                *b = ng;
                *p = Some((x, y));
                open.push(Reverse((ng + heuristic(nx, ny), ng, ny, nx)));
            }
        }
    }
    PathOutcome::Unreachable { visited }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(class: TerrainClass, elev_cm: i16) -> Cell {
        Cell {
            class,
            elev_cm,
            cover: 0,
            room: 0,
        }
    }

    #[test]
    fn the_shipped_table_is_digest_pinned() {
        let actual = locomotion_digest().expect("digest");
        assert_eq!(
            actual, LOCOMOTION_DIGEST,
            "the locomotion table moved; if the edit was reviewed, pin LOCOMOTION_DIGEST to {actual}"
        );
    }

    #[test]
    fn the_shipped_table_parses_and_names_every_base_locomotion() {
        let t = LocomotionTable::shipped();
        assert_eq!(t.version, 1);
        for l in BASE_LOCOMOTIONS {
            assert!(t.locomotions.contains_key(l), "{l}");
        }
        for tag in BASE_TAGS {
            assert!(t.tags.contains_key(tag), "{tag}");
        }
    }

    #[test]
    fn every_locomotion_row_reads_as_doc_14_says() {
        let t = LocomotionTable::shipped();
        // biped on road 1.0
        assert_eq!(t.cost("road", "biped", &[]), Some(100));
        // tracked in deep water impassable; hover over shallow water passable
        assert_eq!(t.cost("water_deep", "tracked", &[]), None);
        assert!(t.passable("water", "hover", &[]));
        assert!(t.passable("water_deep", "hover", &[]));
        // nothing walks a wall but the air
        assert!(!t.passable("wall", "biped", &[]));
        assert!(t.passable("wall", "rotor", &[]));
        // a boat is water-bound
        assert!(t.passable("water_deep", "displacement", &[]));
        assert!(!t.passable("road", "displacement", &[]));
        // amphibious opens deep water to a tracked vehicle; submersible for a boat
        assert_eq!(t.cost("water_deep", "tracked", &["amphibious"]), Some(400));
        assert_eq!(
            t.cost("water_deep", "displacement", &["submersible"]),
            Some(100)
        );
        // wheels on grass, feet on rubble, the default for an unnamed class
        assert_eq!(t.cost("grass", "wheeled", &[]), Some(220));
        assert_eq!(t.cost("rubble", "biped", &[]), Some(250));
        assert_eq!(t.cost("lava", "biped", &[]), Some(150));
        assert_eq!(t.cost("road", "spider", &[]), None);
    }

    #[test]
    fn slope_limits_stop_a_step_and_price_the_climb() {
        let t = LocomotionTable::shipped();
        let flat = cell(TerrainClass::Ground, 0);
        // A 50 cm rise over a metre: 50 %.
        let up = cell(TerrainClass::Ground, 50);
        assert_eq!(slope_pct(flat, up, StepKind::Orthogonal, 100), 50);
        assert_eq!(
            slope_pct(up, flat, StepKind::Orthogonal, 100),
            0,
            "downhill is free"
        );
        assert_eq!(
            move_cost(t, flat, up, StepKind::Orthogonal, 100, "biped", &[]),
            Ok(180),
            "120 × 1.5"
        );
        assert!(matches!(
            move_cost(t, flat, up, StepKind::Orthogonal, 100, "wheeled", &[]),
            Err(Impassable::Slope {
                slope_pct: 50,
                limit_pct: 25
            })
        ));
        assert_eq!(
            move_cost(t, flat, up, StepKind::Orthogonal, 100, "wheeled", &["jump"]),
            Ok(300),
            "jump doubles the limit"
        );
        // A diagonal runs √2 further and costs it.
        assert_eq!(
            move_cost(t, flat, flat, StepKind::Diagonal, 100, "biped", &[]),
            Ok(169)
        );
        assert_eq!(slope_pct(flat, up, StepKind::Diagonal, 100), 35);
        assert_eq!(
            move_cost(
                t,
                flat,
                cell(TerrainClass::Wall, 0),
                StepKind::Orthogonal,
                100,
                "biped",
                &[]
            ),
            Err(Impassable::Class(TerrainClass::Wall))
        );
        assert_eq!(
            move_cost(t, flat, flat, StepKind::Orthogonal, 100, "spider", &[]),
            Err(Impassable::NoLocomotion)
        );
    }

    #[test]
    fn a_dock_is_a_climb_for_legs_and_a_wall_for_wheels() {
        let t = LocomotionTable::shipped();
        let paved = cell(TerrainClass::Paved, 0);
        let dock = cell(TerrainClass::Platform, 120);
        assert_eq!(
            move_cost(t, paved, dock, StepKind::Orthogonal, 100, "biped", &[]),
            Ok(150),
            "a climb: the dock's cost and a half"
        );
        assert_eq!(
            move_cost(t, paved, dock, StepKind::Orthogonal, 100, "legged", &[]),
            Ok(180)
        );
        assert!(matches!(
            move_cost(t, paved, dock, StepKind::Orthogonal, 100, "wheeled", &[]),
            Err(Impassable::Slope { slope_pct: 120, .. })
        ));
        // Too high to climb, or not built: the slope rules.
        let ledge = cell(TerrainClass::Platform, 200);
        assert!(matches!(
            move_cost(t, paved, ledge, StepKind::Orthogonal, 100, "biped", &[]),
            Err(Impassable::Slope { slope_pct: 200, .. })
        ));
        let bank = cell(TerrainClass::Ground, 120);
        assert!(matches!(
            move_cost(t, paved, bank, StepKind::Orthogonal, 100, "biped", &[]),
            Err(Impassable::Slope { slope_pct: 120, .. })
        ));
        // Down off the dock is free of slope.
        assert_eq!(
            move_cost(t, dock, paved, StepKind::Orthogonal, 100, "biped", &[]),
            Ok(100)
        );
    }

    #[test]
    fn a_ruleset_adds_a_class_or_a_locomotion_and_never_changes_a_core_row() {
        let mut t = LocomotionTable::shipped().clone();
        assert_eq!(
            t.extend("road", "biped", Some(50), 60),
            Err(PhysicsError::CoreRow {
                class: "road".into(),
                locomotion: "biped".into()
            })
        );
        assert_eq!(
            t.cost("road", "biped", &[]),
            Some(100),
            "the core row stands"
        );
        t.extend("lava", "biped", None, 60).expect("a new class");
        t.extend("road", "spider", Some(90), 80)
            .expect("a new locomotion");
        t.extend("lava", "spider", Some(100), 80).expect("both new");
        assert_eq!(t.cost("lava", "biped", &[]), None);
        assert_eq!(t.cost("road", "spider", &[]), Some(90));
        assert_eq!(
            t.cost("grass", "spider", &[]),
            None,
            "a new locomotion's default is impassable"
        );
        assert_eq!(t.extension_classes, vec![SmolStr::new("lava")]);
        assert_eq!(t.extension_locomotions, vec![SmolStr::new("spider")]);
        assert_eq!(
            LocomotionTable::shipped().cost("lava", "biped", &[]),
            Some(150),
            "the shipped table is untouched"
        );
    }

    fn open_surface(w: u16, h: u16) -> SurfaceComposite {
        use crate::cells::Cells;
        use crate::geom::PointCm;
        use crate::raster::Grid;
        use crate::record::SurfaceRef;
        SurfaceComposite {
            surface: SurfaceRef::Ground,
            grid: Grid {
                origin: PointCm { x: 0, y: 0 },
                resolution_cm: 100,
                width: w,
                height: h,
            },
            cells: Cells::filled(w, h, cell(TerrainClass::Paved, 0), 65_536).expect("cells"),
            rooms: Vec::new(),
            features: Vec::new(),
        }
    }

    #[test]
    fn a_path_goes_round_a_wall_and_is_the_same_twice() {
        let t = LocomotionTable::shipped();
        let mut s = open_surface(10, 5);
        // A wall across the middle with a gap at the top.
        for y in 0..4 {
            s.cells.set(5, y, cell(TerrainClass::Wall, 0));
        }
        let PathOutcome::Found(p) = path(&s, t, (0, 0), (9, 0), BIPED, &[], 1_000) else {
            panic!("a path");
        };
        assert_eq!(p.cells.first(), Some(&(0, 0)));
        assert_eq!(p.cells.last(), Some(&(9, 0)));
        assert!(p.cells.contains(&(5, 4)), "through the gap: {:?}", p.cells);
        assert!(
            p.cells
                .iter()
                .all(|&(x, y)| s.cells.get(x, y).is_some_and(standable))
        );
        assert!(p.cost > 900, "longer than the straight nine: {}", p.cost);
        let again = path(&s, t, (0, 0), (9, 0), BIPED, &[], 1_000);
        assert_eq!(PathOutcome::Found(p.clone()), again, "deterministic");
        // Wheels take the same gap; a boat has no path at all.
        assert!(matches!(
            path(&s, t, (0, 0), (9, 0), "wheeled", &[], 1_000),
            PathOutcome::Found(_)
        ));
        assert!(matches!(
            path(&s, t, (0, 0), (9, 0), "displacement", &[], 1_000),
            PathOutcome::Unreachable { .. }
        ));
    }

    #[test]
    fn a_path_search_stops_at_its_budget_and_a_sealed_target_is_unreachable() {
        let t = LocomotionTable::shipped();
        let mut s = open_surface(20, 20);
        assert!(matches!(
            path(&s, t, (0, 0), (19, 19), BIPED, &[], 5),
            PathOutcome::OverBudget { budget: 5 }
        ));
        // Seal the target in.
        for (x, y) in [(18, 18), (18, 19), (19, 18)] {
            s.cells.set(x, y, cell(TerrainClass::Wall, 0));
        }
        let PathOutcome::Unreachable { visited } =
            path(&s, t, (0, 0), (19, 19), BIPED, &[], 10_000)
        else {
            panic!("unreachable");
        };
        assert!(
            visited > 300 && visited < 400,
            "the whole open grid was searched once: {visited}"
        );
        // Off the grid is unreachable at once.
        assert_eq!(
            path(&s, t, (0, 0), (40, 40), BIPED, &[], 10_000),
            PathOutcome::Unreachable { visited: 0 }
        );
    }

    fn standable(c: Cell) -> bool {
        LocomotionTable::shipped().passable(c.class.label(), BIPED, &[])
    }

    #[test]
    fn every_class_has_props_and_the_walls_are_opaque() {
        for class in TerrainClass::ALL {
            let p = props(class);
            assert!(BASE_MEDIA.contains(&p.medium), "{class:?}");
        }
        assert_eq!(props(TerrainClass::Wall).opacity, Opacity::Opaque);
        assert_eq!(props(TerrainClass::Wall).cover, Cover::Full);
        assert_eq!(
            props(TerrainClass::WoodsLight).opacity,
            Opacity::Partial(25)
        );
        assert_eq!(props(TerrainClass::Rubble).cover, Cover::Partial);
        assert_eq!(props(TerrainClass::Regolith).medium, "space");
    }

    // ---- cut AP: the dense row and the heuristic ----------------------------------------------

    /// **A1 — the dense row prices every step as `move_cost` does** (cut AP; the brief's R3: one
    /// step rule, two lookups). Exhaustive: every base locomotion, an unknown one (`spider`) and
    /// an extension one (`spider2`, made on a clone), under five tag sets; every class pair, ten
    /// rises from −300 cm to +1,000 cm, both step kinds, at 1 m and 10 m cells. The resolved row's
    /// `step` equals `move_cost`, the `Err` variants included.
    #[test]
    fn ap_the_dense_row_prices_every_step_as_move_cost_does() {
        let mut t = LocomotionTable::shipped().clone();
        t.extend("road", "spider2", Some(90), 80)
            .expect("a new locomotion");
        let mut locomotions: Vec<&str> = BASE_LOCOMOTIONS.to_vec();
        locomotions.extend(["spider", "spider2"]);
        let tag_sets: [&[&str]; 5] = [
            &[],
            &["amphibious"],
            &["jump"],
            &["submersible"],
            &["amphibious", "jump"],
        ];
        let mut checked = 0_u64;
        for locomotion in &locomotions {
            for tags in tag_sets {
                let resolved = LocomotionCosts::resolve(&t, locomotion, tags);
                for from_class in TerrainClass::ALL {
                    for to_class in TerrainClass::ALL {
                        for rise in [-300_i16, -50, 0, 30, 50, 120, 150, 200, 300, 1_000] {
                            let from = cell(from_class, 0);
                            let to = cell(to_class, rise);
                            for kind in [StepKind::Orthogonal, StepKind::Diagonal] {
                                for cell_cm in [100, 1_000] {
                                    let dense = resolved
                                        .clone()
                                        .and_then(|c| c.step(from, to, kind, cell_cm));
                                    let keyed =
                                        move_cost(&t, from, to, kind, cell_cm, locomotion, tags);
                                    assert_eq!(
                                        dense, keyed,
                                        "{locomotion} {tags:?} {from_class:?}→{to_class:?} \
                                         rise {rise} {kind:?} {cell_cm}"
                                    );
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 11 * 5 * 18 * 18 * 10 * 2 * 2);
    }

    /// The fourteen classes A2's grids mix.
    const MIXED: [TerrainClass; 14] = [
        TerrainClass::Paved,
        TerrainClass::Road,
        TerrainClass::Ground,
        TerrainClass::Grass,
        TerrainClass::Water,
        TerrainClass::WaterDeep,
        TerrainClass::Rubble,
        TerrainClass::Sand,
        TerrainClass::Rock,
        TerrainClass::Wall,
        TerrainClass::WoodsLight,
        TerrainClass::WoodsHeavy,
        TerrainClass::Ice,
        TerrainClass::Regolith,
    ];

    /// A deterministic 64 × 64 mixed grid at `cell_cm` (an integer hash of the coordinates, as
    /// `tacsim::surface::drive`'s `noisy`, never an RNG): the fourteen classes and ±150 cm of
    /// relief a cell.
    fn mixed(cell_cm: u16) -> SurfaceComposite {
        let mut s = open_surface(64, 64);
        s.grid.resolution_cm = cell_cm;
        for y in 0..64_u16 {
            for x in 0..64_u16 {
                let h = (u32::from(x).wrapping_mul(73_856_093)
                    ^ u32::from(y).wrapping_mul(19_349_663))
                .wrapping_mul(2_654_435_761);
                let class = MIXED[((h >> 8) % 14) as usize];
                let elev = i16::try_from((h >> 16) % 301).unwrap_or(0) - 150;
                s.cells.set(x, y, cell(class, elev));
            }
        }
        s
    }

    /// The step kind of a neighbour offset.
    fn kind_of(dx: i32, dy: i32) -> StepKind {
        if dx != 0 && dy != 0 {
            StepKind::Diagonal
        } else {
            StepKind::Orthogonal
        }
    }

    /// A plain Dijkstra with no heuristic over `move_cost` (the string rule), in the atlas's
    /// neighbour order: the least cost from `from` to every cell, row-major; `None` unreachable.
    /// `reverse` relaxes reversed edges (`u → v` priced `move_cost(u, v)`), giving the least cost
    /// from every cell TO `from`.
    fn reference_flood(
        surface: &SurfaceComposite,
        table: &LocomotionTable,
        from: (u16, u16),
        locomotion: &str,
        tags: &[&str],
        reverse: bool,
    ) -> Vec<Option<u64>> {
        let w = usize::from(surface.grid.width);
        let h = usize::from(surface.grid.height);
        let cell_cm = u32::from(surface.grid.resolution_cm);
        let mut best: Vec<Option<u64>> = vec![None; w * h];
        let mut done = vec![false; w * h];
        let mut open = BinaryHeap::new();
        best[usize::from(from.1) * w + usize::from(from.0)] = Some(0);
        open.push(Reverse((0_u64, from.1, from.0)));
        while let Some(Reverse((g, y, x))) = open.pop() {
            let i = usize::from(y) * w + usize::from(x);
            if done[i] {
                continue;
            }
            done[i] = true;
            let here = surface.cells.get(x, y).expect("on the grid");
            for (dx, dy) in NEIGHBOURS {
                let (Ok(nx), Ok(ny)) = (
                    u16::try_from(i32::from(x) + dx),
                    u16::try_from(i32::from(y) + dy),
                ) else {
                    continue;
                };
                let Some(next) = surface.cells.get(nx, ny) else {
                    continue;
                };
                let (a, b) = if reverse { (next, here) } else { (here, next) };
                let Ok(step) = move_cost(table, a, b, kind_of(dx, dy), cell_cm, locomotion, tags)
                else {
                    continue;
                };
                let ni = usize::from(ny) * w + usize::from(nx);
                let ng = g + u64::from(step);
                if !done[ni] && best[ni].is_none_or(|b| ng < b) {
                    best[ni] = Some(ng);
                    open.push(Reverse((ng, ny, nx)));
                }
            }
        }
        best
    }

    /// The least cost from `from` to `to` by a plain Dijkstra over `move_cost`; `None`
    /// unreachable.
    fn reference_least(
        surface: &SurfaceComposite,
        table: &LocomotionTable,
        from: (u16, u16),
        to: (u16, u16),
        locomotion: &str,
        tags: &[&str],
    ) -> Option<u64> {
        let w = usize::from(surface.grid.width);
        reference_flood(surface, table, from, locomotion, tags, false)
            [usize::from(to.1) * w + usize::from(to.0)]
    }

    /// A2's and A3's locomotions.
    const SEARCHED: [&str; 6] = [
        "biped",
        "legged",
        "wheeled",
        "tracked",
        "hover",
        "displacement",
    ];
    /// A2's starts.
    const STARTS: [(u16, u16); 4] = [(0, 0), (63, 0), (17, 40), (31, 31)];
    /// A2's goals (with the starts: 16 pairs, `(0, 0)` → `(63, 63)` corner to corner among them).
    const GOALS: [(u16, u16); 4] = [(63, 63), (0, 63), (45, 12), (8, 9)];

    /// **A2 — the search finds the least cost a plain Dijkstra finds** (cut AP; the brief's R6):
    /// on A2's mixed grids at 1 m and 10 m cells, for six locomotions × no tags and amphibious ×
    /// 16 start/goal pairs, at a budget of the whole grid: `Found` exactly where the reference
    /// reaches, its cost the reference's least, its cells from `from` to `to` in neighbour steps
    /// summing (by `move_cost`) to its cost; `Unreachable` exactly where the reference does not.
    #[test]
    fn ap_the_search_finds_the_least_cost_a_plain_dijkstra_finds() {
        let t = LocomotionTable::shipped();
        let (mut found, mut unreachable) = (0, 0);
        for cell_cm in [100_u16, 1_000] {
            let s = mixed(cell_cm);
            let budget = 64 * 64;
            for locomotion in SEARCHED {
                for tags in [&[][..], &["amphibious"][..]] {
                    for from in STARTS {
                        let flood = reference_flood(&s, t, from, locomotion, tags, false);
                        for to in GOALS {
                            let want = flood[usize::from(to.1) * 64 + usize::from(to.0)];
                            let got = path(&s, t, from, to, locomotion, tags, budget);
                            let what = format!("{locomotion} {tags:?} {from:?}→{to:?} {cell_cm}");
                            match (got, want) {
                                (PathOutcome::Found(p), Some(c)) => {
                                    assert_eq!(u64::from(p.cost), c, "{what}");
                                    assert_eq!(p.cells.first(), Some(&from), "{what}");
                                    assert_eq!(p.cells.last(), Some(&to), "{what}");
                                    let mut sum = 0_u64;
                                    for pair in p.cells.windows(2) {
                                        let kind = StepKind::between(pair[0], pair[1])
                                            .expect("neighbours");
                                        let a = s.cells.get(pair[0].0, pair[0].1).expect("a");
                                        let b = s.cells.get(pair[1].0, pair[1].1).expect("b");
                                        let step = move_cost(
                                            t,
                                            a,
                                            b,
                                            kind,
                                            u32::from(cell_cm),
                                            locomotion,
                                            tags,
                                        )
                                        .expect("a step");
                                        sum += u64::from(step);
                                    }
                                    assert_eq!(sum, c, "{what}: the steps sum to the cost");
                                    found += 1;
                                }
                                (PathOutcome::Unreachable { .. }, None) => unreachable += 1,
                                (other, want) => panic!("{what}: {other:?} against {want:?}"),
                            }
                        }
                    }
                }
            }
        }
        println!("A2: found {found}, unreachable {unreachable}");
        assert!(found > 0 && unreachable > 0, "{found} / {unreachable}");
        let s = mixed(1_000);
        assert_eq!(
            reference_least(&s, t, (0, 0), (0, 0), "tracked", &[]),
            Some(0)
        );
    }

    /// **A3 — the heuristic never overestimates and is consistent** (cut AP): on A2's grids, for
    /// each locomotion and each of four goals, with `orth` the cheapest passable class present:
    /// `h(goal) = 0`; for every cell and every passable neighbour, `h(n) ≤ step(n → n′) + h(n′)`;
    /// and for 32 sampled cells, `h(n)` is at most the reference's least cost to the goal (a
    /// reverse flood, itself checked against `reference_least` on two of them).
    #[test]
    fn ap_the_heuristic_never_overestimates_and_is_consistent() {
        let t = LocomotionTable::shipped();
        let mut bounded = 0;
        for cell_cm in [100_u16, 1_000] {
            let s = mixed(cell_cm);
            for locomotion in SEARCHED {
                let costs = LocomotionCosts::resolve(t, locomotion, &[]).expect("a row");
                let orth = u64::from(costs.cheapest_on(&s.cells).unwrap_or(0));
                assert!(orth > 0, "{locomotion}: some class on the grid is passable");
                for goal in GOALS {
                    let h = |c: (u16, u16)| octile_floor(orth, c, goal);
                    assert_eq!(h(goal), 0);
                    for y in 0..64_u16 {
                        for x in 0..64_u16 {
                            let here = s.cells.get(x, y).expect("a cell");
                            for (dx, dy) in NEIGHBOURS {
                                let (Ok(nx), Ok(ny)) = (
                                    u16::try_from(i32::from(x) + dx),
                                    u16::try_from(i32::from(y) + dy),
                                ) else {
                                    continue;
                                };
                                let Some(next) = s.cells.get(nx, ny) else {
                                    continue;
                                };
                                let Ok(step) =
                                    costs.step(here, next, kind_of(dx, dy), u32::from(cell_cm))
                                else {
                                    continue;
                                };
                                assert!(
                                    h((x, y)) <= u64::from(step) + h((nx, ny)),
                                    "{locomotion} {cell_cm}: ({x}, {y})→({nx}, {ny}) toward \
                                     {goal:?}"
                                );
                            }
                        }
                    }
                    let to_goal = reference_flood(&s, t, goal, locomotion, &[], true);
                    for n in 0..32_u16 {
                        let c = ((n * 37 + 11) % 64, (n * 23 + 5) % 64);
                        let least = to_goal[usize::from(c.1) * 64 + usize::from(c.0)];
                        if n < 2 {
                            assert_eq!(
                                least,
                                reference_least(&s, t, c, goal, locomotion, &[]),
                                "the reverse flood is the reference"
                            );
                        }
                        if let Some(least) = least {
                            assert!(
                                h(c) <= least,
                                "{locomotion} {cell_cm}: {c:?} toward {goal:?}"
                            );
                            bounded += 1;
                        }
                    }
                }
            }
        }
        assert!(bounded > 0);
    }

    /// **A4 — a straight diagonal over one footing visits about its own cells** (cut AP: the
    /// heuristic is in force; the old `Chebyshev × 100` visited most of the grid): a 256 × 256
    /// all-regolith surface at 10 m cells, tracked, `(0, 0)` → `(255, 255)`: 256 cells, the cost
    /// `255 × ⌊150 × 141 / 100⌋ = 53,805`, at most 512 cells visited.
    #[test]
    fn ap_a_straight_diagonal_over_one_footing_visits_about_its_own_cells() {
        let t = LocomotionTable::shipped();
        let mut s = open_surface(256, 256);
        s.grid.resolution_cm = 1_000;
        s.cells = crate::cells::Cells::filled(256, 256, cell(TerrainClass::Regolith, 0), 65_536)
            .expect("cells");
        let PathOutcome::Found(p) = path(&s, t, (0, 0), (255, 255), "tracked", &[], 65_536) else {
            panic!("a path");
        };
        println!("A4: visited {}", p.visited);
        assert_eq!(p.cells.len(), 256);
        assert_eq!(p.cost, 255 * (150 * 141 / 100));
        assert_eq!(p.cost, 53_805);
        assert!(p.visited <= 512, "{}", p.visited);
    }

    /// **A5 — an unknown locomotion answers as before** (cut AP; the brief's R4). The old body,
    /// read: the start is pushed and popped whatever the locomotion (`visited` 1); the start is
    /// the goal → `Found` at cost 0; else `visited >= budget` → `OverBudget` at a budget of 1;
    /// else every neighbour's `move_cost` is `NoLocomotion`, nothing is pushed, and the heap
    /// empties → `Unreachable { visited: 1 }`. The dense search resolves the unknown locomotion
    /// to every class impassable and answers the same three.
    #[test]
    fn ap_an_unknown_locomotion_answers_as_before() {
        let t = LocomotionTable::shipped();
        let s = open_surface(10, 10);
        assert_eq!(
            path(&s, t, (3, 3), (3, 3), "spider", &[], 1_000),
            PathOutcome::Found(Path {
                cells: vec![(3, 3)],
                cost: 0,
                visited: 1
            })
        );
        assert_eq!(
            path(&s, t, (3, 3), (5, 5), "spider", &[], 1_000),
            PathOutcome::Unreachable { visited: 1 }
        );
        assert_eq!(
            path(&s, t, (3, 3), (5, 5), "spider", &[], 1),
            PathOutcome::OverBudget { budget: 1 }
        );
        assert_eq!(
            LocomotionCosts::resolve(t, "spider", &[]),
            Err(Impassable::NoLocomotion)
        );
        assert_eq!(LocomotionCosts::impassable().cheapest_on(&s.cells), None);
    }
}
