//! **The arena's state** (16b cut A.1; doc 16 §3.1 "Pulse phases", §3.2 "Space",
//! §3.3 "Units", §3.6 "Perception", §4 "Arena state").
//!
//! One [`Arena`] is one match: the definition it runs on, its units in unit-id
//! order, the per-`(unit, channel)` epoch table intents are authorised against,
//! the per-viewer shadow contact store, the due-event heap, the streaming
//! ledger, the clock, and the bounded per-unit queues of discrete actions. Every
//! stored quantity is an integer (doc 16 §3.1 "Determinism").
//!
//! Nothing here reads a clock, a file or a socket: [`Arena::new`] takes the seam
//! message of doc 16 §3.1 ([`CustodyOut`]) and two configuration blocks, and the
//! only way arena time advances is a caller handing [`Clock::advance`] a wall
//! reading.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use mush_atlas::TerrainClass;
use mush_atlas::trace::Eye;
use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::TacticalUnit;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::clock::Clock;
use crate::arena::horizon::{Block, Horizon, HorizonSight};
use crate::arena::index::{Candidate, SpatialIndex};
use crate::arena::intents::Intent;
use crate::arena::ledger::{EventKind, Ledger};
use crate::arena::perception::{Eccm, FrameNotice, Jammer, SensorProfile, ThermalSignature};
use crate::arena::resolution::UnitProfile;
use crate::arena::step::FrameSlots;
use crate::arena::weapons::{Shot, WeaponBand};
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
use crate::definitions::resolution::ResolutionDef;
use crate::definitions::unit::UnitClass;

/// The digest domain of [`ArenaDefinition::digest`]. Length-prefixed fields
/// follow it, so a definition's digest can never collide with another contract's
/// hash of the same bytes.
pub const DEFINITION_DIGEST_DOMAIN: &[u8] = b"tacsim.arena.definition.v1";

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// One cell of the arena window (doc 16 §3.2 "Terrain").
///
/// `move_cost` is the per-cell movement cost for the `tracked` mode, where
/// **`0` means impassable** and the terrain factor of doc 16 §3.4 step 2 is
/// `100 / move_cost`. Cut T fills it from `mush_atlas`'s passability and cost
/// tables; here it is data, so a test can lay down an impassable band with no
/// atlas at all. `class` is carried for cover and opacity, which cut S and cut W
/// consume.
///
/// **16b cut AE.1** (16c `B-58`): `move_cost` is the **tracked** row's price and
/// nothing else, and it keeps its name for that. A unit reads its own row's
/// price through [`ArenaTerrain::cost`], which answers `move_cost` for the
/// tracked row and the terrain's class table for the other two; `move_cost`
/// and [`TerrainCell::passable`] are read directly only by the loader's
/// entry-zone check, the digest and the tests that build cells — nothing that
/// knows a unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainCell {
    /// The ground class, for cover and opacity.
    pub class: TerrainClass,
    /// The cell's elevation against the region datum, centimetres. A unit's
    /// `z_cm` equals the cell's `elev_cm` (doc 16 §3.2).
    pub elev_cm: i16,
    /// The `tracked` movement cost; `0` is impassable. **The tracked row's
    /// price** (16b cut AE.1): a unit of another row reads its own through
    /// [`ArenaTerrain::cost`].
    pub move_cost: u16,
}

impl TerrainCell {
    /// Whether the `tracked` mode may enter this cell (doc 16 §3.4 step 4).
    ///
    /// **The tracked row's reading** (16b cut AE.1): a reader that knows the
    /// unit asks [`ArenaTerrain::passable_for`] with the unit's [`CostRow`]
    /// instead.
    #[must_use]
    pub const fn passable(&self) -> bool {
        self.move_cost != 0
    }
}

/// **Which row of the shipped locomotion table a unit pays** (16b cut AE.1;
/// 16c `B-58`; doc 16 §3.4 step 2).
///
/// Before this cut every arena cell was priced once, off the table's `tracked`
/// row, and every reader — motion's factor and refusal, the A\* and its
/// heuristic scale, the reflex controller's open-step tests and goal pickers,
/// the shoulder rule, the placement and opening tests — read that one number,
/// so a walker was refused the thick woods its own `legged` row prices at 200
/// and paid a tank's price for rubble. A unit's row is read once, at check-out
/// ([`CostRow::read`], in [`crate::arena::slope`] beside
/// [`crate::arena::slope::SlopeLimit::read`]), and every reader that knows the
/// unit asks [`ArenaTerrain::cost`] with it.
///
/// Three rows and no more: the three ground locomotions the unit definitions
/// author (`crate::definitions::unit::{TRACKED, WHEELED, LEGGED}`). Anything
/// else — a `hover` mode, a unit with no mobility profile, which is every
/// engine fixture — reads [`CostRow::Tracked`], the price every unit paid
/// before.
///
/// **16b cut AF.1** gave it the snapshot's derive: [`UnitState::cost_row`] is
/// carried whole, under the row's own word ([`CostRow::as_str`]).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CostRow {
    /// The `tracked` row: a tank. The row [`TerrainCell::move_cost`] holds.
    #[default]
    Tracked,
    /// The `wheeled` row: a car.
    Wheeled,
    /// The `legged` row: a walker.
    Legged,
}

impl CostRow {
    /// Every row, in [`CostRow::index`] order.
    pub const ALL: [CostRow; 3] = [CostRow::Tracked, CostRow::Wheeled, CostRow::Legged];

    /// The row's locomotion word in the shipped table — the atlas's own
    /// (`crate::definitions::unit::{TRACKED, WHEELED, LEGGED}`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CostRow::Tracked => crate::definitions::unit::TRACKED,
            CostRow::Wheeled => crate::definitions::unit::WHEELED,
            CostRow::Legged => crate::definitions::unit::LEGGED,
        }
    }

    /// The row's column in [`ArenaTerrain`]'s class table and in
    /// [`Arena::min_cost`]: `0` tracked, `1` wheeled, `2` legged.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            CostRow::Tracked => 0,
            CostRow::Wheeled => 1,
            CostRow::Legged => 2,
        }
    }
}

impl ArenaTerrain {
    /// **Whether a unit may open a match on `(x, y)`** (16b cut M.4, 16c
    /// `T-14`): the cell is passable **and has a way out** — at least one of its
    /// four orthogonal neighbours is passable too. A passable cell walled in on
    /// four sides is a cell a tank could stand on and never leave (a diagonal
    /// might still be open, but an opening cell is chosen to be driven out of, and
    /// the orthogonal rule is the one both the loader and the check-out can state
    /// without a slope table).
    ///
    /// Read by `crate::definitions::arena::from_pack` (a zone must hold at least
    /// one such cell) and by the two world-side prechecks of `@arena/place` and
    /// `@arena/pose`.
    ///
    /// **16b cut AE.1**: this is the **tracked** row's reading —
    /// [`ArenaTerrain::has_way_out_for`] with [`CostRow::Tracked`] — kept for
    /// the callers that know no unit. `crate::match_::opening_pose` and the
    /// arena's own placement test know the unit and ask its row.
    #[must_use]
    pub fn has_way_out(&self, x: u16, y: u16) -> bool {
        self.has_way_out_for(CostRow::Tracked, x, y)
    }

    /// **[`ArenaTerrain::has_way_out`] for one row** (16b cut AE.1): the cell is
    /// passable **to that row** and at least one of its four orthogonal
    /// neighbours is too. A car on rubble has no way out, whatever a tank would
    /// have there.
    #[must_use]
    pub fn has_way_out_for(&self, row: CostRow, x: u16, y: u16) -> bool {
        let passable = |x: Option<u16>, y: Option<u16>| match (x, y) {
            (Some(x), Some(y)) => self.passable_for(row, x, y),
            _ => false,
        };
        passable(Some(x), Some(y))
            && (passable(x.checked_sub(1), Some(y))
                || passable(x.checked_add(1), Some(y))
                || passable(Some(x), y.checked_sub(1))
                || passable(Some(x), y.checked_add(1)))
    }
}

/// The arena window's cell grid: row-major, `width × height` cells of `cell_cm`
/// centimetres each (doc 16 §3.2 "The arena window", D-16-5: 256 by 256 at
/// 1,000 cm).
///
/// The fields are private because the length of `cells` is an invariant of the
/// other three: [`ArenaTerrain::new`] is the only way in, and it refuses a grid
/// whose cell count is not `width × height` rather than letting a short vector
/// turn every lookup off the end into a silent `None`.
///
/// **Two ways in since 16b cut AE.1, and one rule about prices** (16c `B-58`).
/// [`ArenaTerrain::new`] (and [`ArenaTerrain::uniform`] over it) prices **every
/// row at the cell's own `move_cost`**: a number a test authors on a cell is
/// its statement about that cell for every unit, so every hand-built window —
/// every fixture in the tree and the CI harness cohort — reads byte for byte
/// what it read before the rows existed, whoever crosses it.
/// [`ArenaTerrain::priced`], which only `crate::definitions::arena::from_pack`
/// calls, lays the shipped locomotion table's three rows beside the cells as one
/// class table, so on the authored ground a walker, a car and a tank each pay
/// their own ([`ArenaTerrain::cost`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaTerrain {
    width: u16,
    height: u16,
    cell_cm: u16,
    cells: Vec<TerrainCell>,
    /// **The horizon index over `cells`** (16b cut J.2; 16c N5 step 2): per
    /// 16- and 4-cell block, the highest and lowest cell and whether any is
    /// opaque or partial, which lets [`ArenaTerrain::sight`] prove a run of a
    /// line clear without reading it and blocked by reading one cell
    /// ([`crate::arena::horizon`]'s head has the rules and why the answer is
    /// `sight_only`'s to the byte).
    ///
    /// **A cache.** It is a pure function of the cells beside it, which
    /// [`ArenaDefinition::digest`] covers, built once with them by
    /// [`ArenaTerrain::new`] and never written, and read by
    /// [`ArenaTerrain::sight`] alone. It lives *here* and not on `Arena`
    /// because the tests swap `arena.definition` and `arena.definition.terrain`
    /// after `Arena::new`, and an index carried by the terrain cannot answer for
    /// any grid but its own. **Not folded into
    /// [`crate::arena::hash::state_hash`]**: folding it would fold the terrain a
    /// second time. And it is not the *cross-observer cache* doc 16 §3.2 rules
    /// out: nothing observed is cached — no line, no answer, no contact — the
    /// index holds terrain, and one observer's pass still depends on nothing
    /// another observer did.
    horizon: Horizon,
    /// **The price of every class for the rows `move_cost` does not hold**
    /// (16b cut AE.1; 16c `B-58`): one `[tracked, wheeled, legged]` row per
    /// `mush_atlas::TerrainClass`, indexed by the class's code (the
    /// discriminants are contiguous, `0..=17`), `0` impassable.
    ///
    /// **`None` is a fixture's window, and it prices every row at the cell's
    /// own `move_cost`.** [`ArenaTerrain::new`] and [`ArenaTerrain::uniform`]
    /// leave it `None`: a number a test authors on a cell is its statement
    /// about that cell **for every unit**, so every hand-built window in the
    /// tree — the CI harness cohort's among them — answers byte for byte what
    /// it answered before the rows existed, whichever unit crosses it. Only
    /// [`ArenaTerrain::priced`], which `crate::definitions::arena::from_pack`
    /// alone calls, fills it, from the shipped locomotion table.
    ///
    /// A class table and not a per-cell layer: the rows are a pure function of
    /// the class, so a layer would be 256 KiB per definition, repeated for every
    /// replay ground cached per digest, to say what fifty-four numbers say. Not
    /// in [`ArenaDefinition::digest`] for the same reason: it is the class and
    /// the shipped table, which is code.
    row_costs: Option<[[u16; 3]; 18]>,
    /// **The cheapest passable price of each row over the window's cells**
    /// (16b cut AE.1), at least `1`, in [`CostRow::index`] order — computed
    /// once at construction, because the grid never changes after it.
    min_cost: [u32; 3],
}

impl ArenaTerrain {
    /// The grid, refusing anything that is not exactly `width × height` cells.
    ///
    /// # Errors
    ///
    /// [`ArenaError::TerrainShape`] for a zero dimension, a zero `cell_cm` (the
    /// divisor of [`ArenaTerrain::cell_of`]) or a cell count that is not the
    /// product.
    pub fn new(
        width: u16,
        height: u16,
        cell_cm: u16,
        cells: Vec<TerrainCell>,
    ) -> Result<ArenaTerrain, ArenaError> {
        let expected = usize::from(width).checked_mul(usize::from(height));
        match expected {
            Some(expected)
                if width != 0 && height != 0 && cell_cm != 0 && cells.len() == expected =>
            {
                // 16b cut J.2: the index is built here, from the cells just
                // checked, so every terrain in the tree carries its own.
                let horizon = Horizon::build(width, height, &cells);
                let mut terrain = ArenaTerrain {
                    width,
                    height,
                    cell_cm,
                    cells,
                    horizon,
                    // 16b cut AE.1: a hand-built window prices every row at the
                    // cell's own number (see the field).
                    row_costs: None,
                    min_cost: [1; 3],
                };
                terrain.min_cost = terrain.cheapest();
                Ok(terrain)
            }
            _ => Err(ArenaError::TerrainShape {
                width,
                height,
                cell_cm,
                cells: cells.len(),
            }),
        }
    }

    /// **The grid priced per row from `table`** (16b cut AE.1; 16c `B-58`) —
    /// [`ArenaTerrain::new`], then the class table filled from
    /// `TerrainClass::ALL × CostRow::ALL` (`table.cost(class.label(), row, &[])`,
    /// `None` → `0`: fifty-four string-keyed lookups per build, never one per
    /// cell) and each row's cheapest price taken again.
    ///
    /// Called by `crate::definitions::arena::from_pack` and nowhere else in the
    /// build, with `LocomotionTable::shipped()`. The tracked column is the one
    /// the loader already wrote onto every cell as `move_cost`, and a debug
    /// build asserts that they agree.
    ///
    /// # Errors
    ///
    /// As [`ArenaTerrain::new`].
    pub fn priced(
        width: u16,
        height: u16,
        cell_cm: u16,
        cells: Vec<TerrainCell>,
        table: &mush_atlas::physics::LocomotionTable,
    ) -> Result<ArenaTerrain, ArenaError> {
        let mut terrain = ArenaTerrain::new(width, height, cell_cm, cells)?;
        let mut rows = [[0_u16; 3]; 18];
        for class in TerrainClass::ALL {
            let Some(slot) = rows.get_mut(usize::from(class.code())) else {
                continue;
            };
            for row in CostRow::ALL {
                let price = table
                    .cost(class.label(), row.as_str(), &[])
                    .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX));
                if let Some(column) = slot.get_mut(row.index()) {
                    *column = price;
                }
            }
        }
        debug_assert!(
            terrain.cells.iter().all(|cell| {
                rows.get(usize::from(cell.class.code()))
                    .and_then(|prices| prices.first())
                    .is_some_and(|tracked| *tracked == cell.move_cost)
            }),
            "the class table's tracked column is the loader's `move_cost`"
        );
        terrain.row_costs = Some(rows);
        terrain.min_cost = terrain.cheapest();
        Ok(terrain)
    }

    /// The immutable locomotion prices captured by a private replay. Unlike
    /// the legacy ground digest, this includes the wheeled and legged prices.
    pub(crate) fn replay_prices(&self) -> Option<[[u16; 3]; 18]> {
        self.row_costs
    }

    /// Rebuild the terrain caches using the recorded prices, never today's
    /// content table. The containing replay verifies its complete payload first.
    pub(crate) fn with_replay_prices(mut self, prices: Option<[[u16; 3]; 18]>) -> Self {
        self.row_costs = prices;
        self.min_cost = self.cheapest();
        self
    }

    /// Each row's cheapest passable price over the cells, at least `1`.
    fn cheapest(&self) -> [u32; 3] {
        let mut out = [1_u32; 3];
        for row in CostRow::ALL {
            let least = self
                .cells
                .iter()
                .map(|cell| self.price(row, cell))
                .filter(|price| *price != 0)
                .min()
                .map_or(1, |price| u32::from(price).max(1));
            if let Some(slot) = out.get_mut(row.index()) {
                *slot = least;
            }
        }
        out
    }

    /// **What `row` pays to be in `cell`** (16b cut AE.1): the cell's own
    /// `move_cost` for the tracked row, and for every row on a window with no
    /// class table; the table's price for the cell's class otherwise. `0` is
    /// impassable.
    ///
    /// The form for a reader that already holds the cell — motion and the A\*
    /// both have it in hand — and [`ArenaTerrain::cost`] is the same answer by
    /// coordinates.
    #[must_use]
    pub fn price(&self, row: CostRow, cell: &TerrainCell) -> u16 {
        match (row, &self.row_costs) {
            (CostRow::Tracked, _) | (_, None) => cell.move_cost,
            (_, Some(table)) => table
                .get(usize::from(cell.class.code()))
                .and_then(|prices| prices.get(row.index()))
                .copied()
                .unwrap_or(0),
        }
    }

    /// **What `row` pays to be in `(x, y)`** (16b cut AE.1):
    /// [`TerrainCell::move_cost`] for [`CostRow::Tracked`]; for the other two,
    /// the class table's price when the terrain was [`ArenaTerrain::priced`],
    /// else the cell's own `move_cost`. `0` off the window or impassable.
    #[must_use]
    pub fn cost(&self, row: CostRow, x: u16, y: u16) -> u16 {
        self.get(x, y).map_or(0, |cell| self.price(row, cell))
    }

    /// Whether `row` may enter `(x, y)` (16b cut AE.1; doc 16 §3.4 step 4): its
    /// [`ArenaTerrain::cost`] there is not `0`.
    #[must_use]
    pub fn passable_for(&self, row: CostRow, x: u16, y: u16) -> bool {
        self.cost(row, x, y) != 0
    }

    /// **The cheapest passable price `row` pays anywhere on the window**, at
    /// least `1` (16b cut AE.1) — the A\* heuristic's scale for a unit of that
    /// row and the divisor of its speed ceiling. Computed at construction.
    #[must_use]
    pub fn min_cost(&self, row: CostRow) -> u32 {
        self.min_cost.get(row.index()).copied().unwrap_or(1)
    }

    /// A grid of one repeated cell — the shape a unit test lays down before it
    /// paints a band or an obstacle into it.
    ///
    /// # Errors
    ///
    /// As [`ArenaTerrain::new`].
    pub fn uniform(
        width: u16,
        height: u16,
        cell_cm: u16,
        cell: TerrainCell,
    ) -> Result<ArenaTerrain, ArenaError> {
        let count = usize::from(width).checked_mul(usize::from(height)).ok_or(
            ArenaError::TerrainShape {
                width,
                height,
                cell_cm,
                cells: 0,
            },
        )?;
        ArenaTerrain::new(width, height, cell_cm, vec![cell; count])
    }

    /// The cell at `(x, y)`, or `None` off the window.
    #[must_use]
    pub fn get(&self, x: u16, y: u16) -> Option<&TerrainCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::from(y).checked_mul(usize::from(self.width))?;
        self.cells.get(index.checked_add(usize::from(x))?)
    }

    /// The cell a pose stands in, or `None` when the pose is off the window.
    ///
    /// Floor division of `x_cm` and `y_cm` by `cell_cm`. A negative coordinate
    /// is off the window by definition — the window's origin is its corner — so
    /// it answers `None` rather than folding toward zero, and a coordinate at or
    /// past `width × cell_cm` (or `height × cell_cm`) answers `None` too. The
    /// boundary belongs to the cell it opens: at `cell_cm = 1000`, `x_cm = 1000`
    /// is column 1, and `x_cm = 999` is column 0.
    #[must_use]
    pub fn cell_of(&self, pose: &Pose) -> Option<(u16, u16)> {
        self.cell_at(pose.x_cm, pose.y_cm)
    }

    /// The same, from two coordinates rather than from a pose (16b cut F.3) —
    /// what a [`SeenPose`] has and a [`Pose`] is not needed for.
    #[must_use]
    pub fn cell_at(&self, x_cm: i64, y_cm: i64) -> Option<(u16, u16)> {
        if x_cm < 0 || y_cm < 0 {
            return None;
        }
        let cell = i64::from(self.cell_cm);
        let x = u16::try_from(x_cm / cell).ok()?;
        let y = u16::try_from(y_cm / cell).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((x, y))
    }

    /// **Whether `cell` is the cell [`ArenaTerrain::cell_at`] answers for
    /// `(x_cm, y_cm)`** (16c `T-107d`), by `cell_at`'s own arithmetic turned
    /// around and without its two divisions: for a non-negative dividend
    /// `x_cm / cell_cm == x` exactly when `x · cell_cm ≤ x_cm < (x + 1) ·
    /// cell_cm`, and the window bound is the same `x < width`, `y < height`
    /// test. A negative coordinate fails the lower bound at every cell (`x ·
    /// cell_cm` is never below zero), so the sign test needs no line of its
    /// own; a coordinate whose quotient is past `u16` is at or past `(x + 1) ·
    /// cell_cm` for every `x` the window holds. One multiply and one add an
    /// axis: the upper bound `(x + 1) · cell_cm` is the lower bound plus the
    /// side, exactly. Both are checked in the pose's `i64` — a `u16` times a
    /// `u16`, and that plus a `u16`, fit it with bits to spare, so neither can
    /// overflow — and a `None` answers `false`, never a wrong `true`. The one
    /// production reader is `Arena::eye_at`, which takes the cull's cell for
    /// a candidate on this test and runs [`ArenaTerrain::cell_of`] when it
    /// fails (`t107d_is_cell_at_is_cell_at_s_own_answer`).
    #[must_use]
    pub fn is_cell_at(&self, cell: (u16, u16), x_cm: i64, y_cm: i64) -> bool {
        let (x, y) = cell;
        if x >= self.width || y >= self.height {
            return false;
        }
        let side = i64::from(self.cell_cm);
        Self::in_span(i64::from(x), side, x_cm) && Self::in_span(i64::from(y), side, y_cm)
    }

    /// `c · side ≤ v < c · side + side` (16c `T-107d`): the product and the
    /// sum checked, the sum being `(c + 1) · side` without the second
    /// multiply.
    fn in_span(c: i64, side: i64, v: i64) -> bool {
        let Some(low) = c.checked_mul(side) else {
            return false;
        };
        let Some(high) = low.checked_add(side) else {
            return false;
        };
        low <= v && v < high
    }

    /// Columns.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Rows.
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// The side of one cell, centimetres.
    #[must_use]
    pub const fn cell_cm(&self) -> u16 {
        self.cell_cm
    }

    /// Every cell, row-major — the read [`ArenaDefinition::digest`] and cut A.2's
    /// motion take.
    #[must_use]
    pub fn cells(&self) -> &[TerrainCell] {
        &self.cells
    }

    /// **The line of sight from `from` to `to` over this terrain, through its
    /// horizon index** (16b cut J.2; doc 16 §3.2 "Line of sight", §3.6 step 2):
    /// the one query, and the one [`Arena::sight`] makes.
    ///
    /// Its `sight` is [`mush_atlas::sight_only`]'s to the byte for every pair;
    /// its `cell_probes` are never more than `sight_only`'s `visited`, and its
    /// `horizon_probes` are the blocks it read ([`Horizon::sight`]).
    #[must_use]
    pub fn sight(&self, from: Eye, to: Eye) -> HorizonSight {
        self.horizon.sight(self, from, to)
    }

    /// The horizon index's size, bytes: its blocks, six bytes each (26,112 on
    /// the shipped 256 by 256 window; reported under 16c `T-36`'s memory bound).
    #[must_use]
    pub fn horizon_bytes(&self) -> usize {
        self.horizon.bytes()
    }

    /// The horizon index's block sides, coarse first: `[16, 4]`, or `[]` for a
    /// terrain made [`ArenaTerrain::without_horizon`].
    #[must_use]
    pub fn horizon_sides(&self) -> Vec<u16> {
        self.horizon.sides()
    }

    /// The block of the index's `level` (an index into
    /// [`crate::arena::horizon::LEVELS`]) that covers cell `(x, y)`, or `None`
    /// for a cell off the window or a level the index does not have — for the
    /// fold check and the tests.
    #[must_use]
    pub fn horizon_block(&self, level: usize, x: u16, y: u16) -> Option<Block> {
        self.get(x, y)?;
        self.horizon.block(level, x, y)
    }

    /// **The same grid with no index** ([`Horizon::walk_only`]): every line is
    /// walked by `sight_only`. For the with-and-without replays; a match never
    /// builds one.
    #[must_use]
    pub fn without_horizon(self) -> ArenaTerrain {
        ArenaTerrain {
            horizon: Horizon::walk_only(),
            ..self
        }
    }
}

/// **The arena's window is a cell grid the atlas trace can walk** (16b cut S.1;
/// doc 16 §3.2 "Line of sight").
///
/// This one impl is the whole of how perception traces without a window. Doc 16
/// §3.2 says line of sight *is* the atlas trace, and doc 16 §3.1 (D-16-1, D-16-3)
/// says the arena holds no store, no records and no window — a
/// `mush_atlas::SurfaceComposite` would drag a `SurfaceRef`, a room table and a
/// `FeatureOverlay` in, all of them record types the contract gate exists to keep
/// out. `mush_atlas::CellGrid` is the trace's *terrain* seam and nothing more: one
/// cell's class and its elevation. So the arena keeps its own grid, the atlas keeps
/// its one per-cell rule, and two walkers read this through it:
/// `mush_atlas::sight_only` for perception (16b cut J.1, [`Arena::sight`]; since
/// 16b cut J.2 through [`ArenaTerrain::sight`], whose horizon index walks a run
/// under the same per-cell rule and hands `sight_only` whatever it does not
/// answer) and `mush_atlas::trace_cells` for the fire path (16b cut W.2).
impl mush_atlas::CellGrid for ArenaTerrain {
    fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)> {
        let cell = self.get(x, y)?;
        Some((cell.class, i32::from(cell.elev_cm)))
    }
}

// ---------------------------------------------------------------------------
// Teams, sides, entry zones and the definition
// ---------------------------------------------------------------------------

/// A side of a match (doc 16 §1: red against blue).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    /// The red side.
    Red,
    /// The blue side.
    Blue,
}

impl Team {
    /// Both sides, in code order.
    pub const ALL: [Team; 2] = [Team::Red, Team::Blue];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Team::Red => "red",
            Team::Blue => "blue",
        }
    }

    /// The other side.
    #[must_use]
    pub const fn other(&self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }

    /// **The letter a side's callsigns carry** (16b cut Q.1; doc 16 §3.6): `R`
    /// for red, `B` for blue, so a classified contact reads `RA`, `BC`.
    ///
    /// A `const fn` over the closed enum and **never the first character of a
    /// display name**: the letter is part of the one name every viewer calls a
    /// unit by, so it has to be a fact about the team and not about whatever a
    /// pack happened to call it. A third side is a compile error here and a
    /// failing test in [`Designator`]'s own module, which is where the rule that
    /// the letters are distinct and none of them is
    /// [`PROVISIONAL_PREFIX`] is written down.
    #[must_use]
    pub const fn letter(&self) -> char {
        match self {
            Team::Red => 'R',
            Team::Blue => 'B',
        }
    }
}

/// A left or right side of a unit — the argument of doc 16 §3.3's
/// `no_turn_toward(left)` consequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The unit's left.
    Left,
    /// The unit's right.
    Right,
}

impl Side {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// A team's deployment rectangle on the window, in cells, inclusive of both
/// corners (doc 16 §3.2 "Terrain": *two team entry zones*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryZone {
    /// The side that deploys here.
    pub team: Team,
    /// The west column.
    pub x0: u16,
    /// The south row.
    pub y0: u16,
    /// The east column, inclusive.
    pub x1: u16,
    /// The north row, inclusive.
    pub y1: u16,
}

/// **A named place on the window** (16b cut N.1; doc 16 §3.2: *three or four key
/// spots*), as the arena carries it.
///
/// Authored by `crate::definitions::arena::KeySpot` and carried across the seam
/// so that a controller and a pilot can call the same ground by the same name.
/// Cut N.1 carries it; the first reader that *routes* on it is cut N.2's order
/// (`objective` names a spot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpot {
    /// What it is called.
    pub name: SmolStr,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// **The marker each key spot is drawn as, by its authored index** (16b cut V.1;
/// doc 16 §3.2 "Terrain": the ground is surveyed, so its named places are
/// known to every pilot at check-out).
///
/// The letters a spot may NOT be, and why each is left out: a spot marker is one
/// byte in the grid, and the three renderers must draw the same byte, so it can
/// never be a byte the ground is already drawn with — `D`, `T` and `W` are
/// [`mush_atlas::TerrainClass::glyph`]'s platform, heavy woods and deep water,
/// and a pilot must never read a field's letter as ground. `X` is
/// [`PROVISIONAL_PREFIX`], the letter an unidentified contact's designator opens
/// with beside the map. Twenty-two letters remain, and
/// [`crate::definitions::arena::MAX_KEY_SPOTS`] is their count, so an arena that
/// names a spot past the alphabet never loads.
///
/// **16b cut O.1 moved it here from `crate::match_::cockpit`**, which re-exports
/// it: a Hold match's [`HoldState::letter`] is the letter of its key spot, and
/// the arena library reaches into no module outside itself for it. One
/// alphabet, one place.
pub const SPOT_LETTERS: [char; 22] = [
    'A', 'B', 'C', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'U',
    'V', 'Y', 'Z',
];

/// **The letter of the spot at `index`** (16b cut V.1), or `None` past the
/// alphabet — which the load-time cap makes unreachable for a loaded arena, and
/// which every caller answers by drawing nothing rather than panicking. (Moved
/// here beside [`SPOT_LETTERS`] by 16b cut O.1.)
#[must_use]
pub fn spot_letter(index: usize) -> Option<char> {
    SPOT_LETTERS.get(index).copied()
}

// ---------------------------------------------------------------------------
// Match modes (16b cut O.1; 16c N11)
// ---------------------------------------------------------------------------

/// **Which rule a match is fought under** (16b cut O.1; 16c N11; doc 16 §3.8).
///
/// Doc 16 §3.8's only positive way to win is to eliminate the other side
/// ([`crate::arena::Arena::settle_elimination`]); a mode is a second reason to
/// fight. It is BOTH a lobby choice (`SIM MODE`, announced like `SIM UNIT`) and
/// an arena property ([`ArenaDefinition::modes`]): the lobby may choose only
/// what the ground declares, and [`Skirmish`](MatchMode::Skirmish) — elimination
/// and nothing else — is what every ground supports and what every match is
/// until somebody chooses otherwise.
///
/// Closed, for [`EndReason`]'s reason: the result record's words are a closed
/// vocabulary (doc 16 §4), and a new mode is a new arm, a new
/// [`code`](MatchMode::code) and a new word. O.3 added `Range`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// **Eliminate the other side** (doc 16 §3.8): the match every ground
    /// supports, and the default.
    #[default]
    Skirmish,
    /// **Keep the ground's key spot uncontested for its time** ([`HoldRule`]).
    /// Elimination still ends it, settled second (16c N11's owner default).
    Hold,
    /// **The range** (16b cut O.3; 16c N13): one side's pilots against a side
    /// of targets — `tacsim.unit.target_mk1`, which neither moves nor shoots —
    /// on the other side's pods. No rule of its own: it ends by the ordinary
    /// elimination rule, when every target is out of action (doc 16 §3.8), or
    /// on `SIM ABORT`; `spent` never ends it, because a target is not armed at
    /// check-out ([`crate::arena::Arena::spent`]). Chosen by `SIM RANGE <n>` and
    /// never by `SIM MODE`, and every ground supports it
    /// ([`ArenaDefinition::supports`]): a target needs no key spot.
    Range,
}

impl MatchMode {
    /// Every mode, in code order — what `SIM MODE` lists a ground's support
    /// against.
    pub const ALL: [MatchMode; 3] = [MatchMode::Skirmish, MatchMode::Hold, MatchMode::Range];

    /// The wire word: the result document's and the replay document's `mode`,
    /// the lobby's word and the ` [hold]` tag.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            MatchMode::Skirmish => "skirmish",
            MatchMode::Hold => "hold",
            MatchMode::Range => "range",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in (item 12).
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            MatchMode::Skirmish => 0,
            MatchMode::Hold => 1,
            MatchMode::Range => 2,
        }
    }

    /// **A typed word** (`SIM MODE hold`): trimmed and case-folded, so `HOLD`
    /// and ` hold ` are the one mode. `None` for anything else.
    #[must_use]
    pub fn parse(word: &str) -> Option<MatchMode> {
        let word = word.trim();
        MatchMode::ALL
            .iter()
            .copied()
            .find(|mode| mode.as_str().eq_ignore_ascii_case(word))
    }
}

/// **A ground's Hold declaration, resolved** (16b cut O.1): which key spot, how
/// many cells round it count as on it, and for how long a side must be alone
/// there to win.
///
/// Authored as `crate::definitions::arena::ArenaModeDef::Hold` by the spot's
/// **name**, checked at load, and resolved by
/// `crate::definitions::arena::from_pack` into the spot's **index** into
/// [`ArenaDefinition::key_spots`] — a comparison per substep has no time for a
/// string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldRule {
    /// The key spot's index into [`ArenaDefinition::key_spots`].
    pub spot: u16,
    /// The half-side of the square round the spot, cells, Chebyshev: a unit is
    /// on the spot when its cell is within this many of the spot's cell on both
    /// axes. `1..=MAX_HOLD_RADIUS_CELLS` (32) by the loader.
    pub radius_cells: u16,
    /// How long a side must be the sole occupant to win, arena milliseconds —
    /// whole seconds, `MIN_HOLD_MS..=MAX_HOLD_MS` by the loader.
    pub hold_ms: u32,
}

/// **One mode a ground declares it supports** (16b cut O.1), beyond
/// [`MatchMode::Skirmish`], which every ground supports and none declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaMode {
    /// A key spot to hold.
    Hold(HoldRule),
}

impl ArenaMode {
    /// The mode this declaration makes available.
    #[must_use]
    pub const fn mode(&self) -> MatchMode {
        match self {
            ArenaMode::Hold(_) => MatchMode::Hold,
        }
    }
}

/// **A Hold match's objective and clock** (16b cut O.1; 16c N11), on
/// [`Arena::hold`].
///
/// The rule is copied off the definition at check-out, with the spot's cell
/// and its map letter resolved, so nothing at a substep reads the definition's
/// lists. `held_by` is the clock: the side that has been the **sole occupant**
/// of the square since the arena millisecond beside it, written only by
/// [`crate::arena::Arena::settle_hold_presence`] at a substep's end and folded
/// into [`crate::arena::hash::state_hash`] (item 12), because it decides an end
/// and moves with no ledger line of its own.
///
/// **It reaches no pilot** (doc 16 §1, §5; plan item 10): the rule is public in
/// selection — the board says it — and whether and since when a side holds the
/// spot is told to nobody. No frame, no notice and no verb reads `held_by`.
///
/// **16b cut AF.1**: carried whole by the snapshot, the clock beside the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldState {
    /// The key spot's cell.
    pub spot: (u16, u16),
    /// The key spot's map letter ([`spot_letter`] of its index), or `None` past
    /// the alphabet, which the load-time cap makes unreachable.
    pub letter: Option<char>,
    /// The half-side of the square, cells.
    pub radius_cells: u16,
    /// How long a sole occupant must stay to win, arena milliseconds.
    pub hold_ms: u32,
    /// **The side alone on the spot, and since when** (arena milliseconds, a
    /// substep boundary), or `None` while the square is empty or contested.
    pub held_by: Option<(Team, i64)>,
}

/// Public objective geometry. Ownership and capture clocks never enter a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldObjective {
    /// The objective's cell.
    pub spot: (u16, u16),
    /// Its public map letter.
    pub letter: Option<char>,
    /// The half-side of the objective square, in cells.
    pub radius_cells: u16,
}

impl HoldObjective {
    /// Whether the cell is inside the public objective square.
    #[must_use]
    pub fn square_contains(&self, cell: (u16, u16)) -> bool {
        cell.0
            .abs_diff(self.spot.0)
            .max(cell.1.abs_diff(self.spot.1))
            <= self.radius_cells
    }
}

impl HoldState {
    /// Copy only the geometry that the pilot and mind may read.
    #[must_use]
    pub const fn objective(&self) -> HoldObjective {
        HoldObjective {
            spot: self.spot,
            letter: self.letter,
            radius_cells: self.radius_cells,
        }
    }

    /// **Whether `cell` is on the spot**: within [`HoldState::radius_cells`] of
    /// [`HoldState::spot`] on both axes (Chebyshev) — one comparison, which is
    /// what makes the presence test one comparison per unit per substep.
    #[must_use]
    pub fn square_contains(&self, cell: (u16, u16)) -> bool {
        cell.0
            .abs_diff(self.spot.0)
            .max(cell.1.abs_diff(self.spot.1))
            <= self.radius_cells
    }
}

/// **One node of the patrol graph** (16b cut N.1; doc 16 §6: *the arena pack's
/// waypoint graph*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waypoint {
    /// The node's authored id, which is what a ledger line or a report names it
    /// by.
    pub id: SmolStr,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// **The patrol graph**, as the arena carries it (16b cut N.1).
///
/// The edges are authored as pairs of node *ids* and stored as pairs of node
/// **indices**, resolved once at construction: the reflex controller walks this
/// graph inside a pulse, and an id lookup per step would be a string comparison
/// per edge per tick. `adjacency` is one sorted, deduplicated row per node, so
/// "the next node from here" is a deterministic walk with no allocation and no
/// map.
///
/// An edge naming a node that is not in `nodes` is **dropped** at construction
/// rather than refused: the loader (`crate::definitions::arena::ArenaDef::check`)
/// is where an unresolvable edge is an authoring error, and a window with one
/// bad edge is still a window a match can be fought on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Waypoints {
    nodes: Vec<Waypoint>,
    adjacency: Vec<Vec<u16>>,
}

impl Waypoints {
    /// The graph, with `edges` naming node ids.
    #[must_use]
    pub fn new(nodes: Vec<Waypoint>, edges: &[(SmolStr, SmolStr)]) -> Waypoints {
        let index_of = |id: &SmolStr| -> Option<u16> {
            nodes
                .iter()
                .position(|node| node.id == *id)
                .and_then(|at| u16::try_from(at).ok())
        };
        let mut adjacency: Vec<Vec<u16>> = vec![Vec::new(); nodes.len()];
        for (from, to) in edges {
            let (Some(from), Some(to)) = (index_of(from), index_of(to)) else {
                continue;
            };
            if from == to {
                continue;
            }
            if let Some(row) = adjacency.get_mut(usize::from(from)) {
                row.push(to);
            }
            if let Some(row) = adjacency.get_mut(usize::from(to)) {
                row.push(from);
            }
        }
        for row in &mut adjacency {
            row.sort_unstable();
            row.dedup();
        }
        Waypoints { nodes, adjacency }
    }

    /// Every node, in authored order.
    #[must_use]
    pub fn nodes(&self) -> &[Waypoint] {
        &self.nodes
    }

    /// One node by index.
    #[must_use]
    pub fn node(&self, at: u16) -> Option<&Waypoint> {
        self.nodes.get(usize::from(at))
    }

    /// The nodes `at` is joined to, in index order.
    #[must_use]
    pub fn neighbours(&self, at: u16) -> &[u16] {
        self.adjacency
            .get(usize::from(at))
            .map_or(&[][..], Vec::as_slice)
    }

    /// Whether the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// **The node nearest a cell**, by squared cell distance, ties to the lower
    /// index — the `unreachable` edge's fallback and `Patrol`'s first goal.
    #[must_use]
    pub fn nearest(&self, x: u16, y: u16) -> Option<u16> {
        let mut best: Option<(u64, u16)> = None;
        for (at, node) in self.nodes.iter().enumerate() {
            let dx = i64::from(node.x) - i64::from(x);
            let dy = i64::from(node.y) - i64::from(y);
            let distance = (dx.saturating_mul(dx)).saturating_add(dy.saturating_mul(dy));
            let distance = u64::try_from(distance).unwrap_or(u64::MAX);
            let at = u16::try_from(at).unwrap_or(u16::MAX);
            if best.is_none_or(|(far, _)| distance < far) {
                best = Some((distance, at));
            }
        }
        best.map(|(_, at)| at)
    }
}

/// The arena a match runs on: its id, its window, its entry zones, its named
/// places and its patrol graph (doc 16 §4's `arenas/proving_ground.json`).
///
/// Immutable for the life of the match. Its [`digest`](ArenaDefinition::digest)
/// goes on the result record (doc 16 §4 "The result record"), which is how a
/// replayed ledger can be told to belong to the terrain it was produced on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaDefinition {
    /// The arena's content id.
    pub id: SmolStr,
    /// The window.
    pub terrain: ArenaTerrain,
    /// The deployment rectangles.
    pub entry_zones: Vec<EntryZone>,
    /// **The named places** (16b cut N.1; doc 16 §3.2).
    pub key_spots: Vec<KeySpot>,
    /// **The patrol graph a machine walks** (16b cut N.1; doc 16 §6): the stock
    /// mind's `Patrol` walk (the reflex controller's until 17b cut M4.6).
    pub waypoints: Waypoints,
    /// **The modes this ground declares** beyond [`MatchMode::Skirmish`] (16b cut
    /// O.1; 16c N11), in authored order — `SIM MODE` lists them and
    /// [`ArenaDefinition::supports`] answers them.
    ///
    /// **Deliberately NOT folded into [`ArenaDefinition::digest`]** (plan item
    /// 9): the digest's job is to tie a ledger to the ground it was fought on,
    /// and the mode a match was fought under is on the result record and the
    /// replay (`"mode"`) and in the state hash (item 12). Folding the menu of
    /// modes would move every recorded digest for a ground whose cells, zones,
    /// spots and graph did not change.
    pub modes: Vec<ArenaMode>,
}

impl ArenaDefinition {
    /// **Whether a match on this ground may be fought under `mode`** (16b cut
    /// O.1): [`MatchMode::Skirmish`] always; **[`MatchMode::Range`] always** (16b
    /// cut O.3 — the range is a side of targets on the other side's pods and
    /// needs nothing of the ground, so there is no `ArenaMode` arm to declare
    /// it); any other mode when the ground declares it.
    #[must_use]
    pub fn supports(&self, mode: MatchMode) -> bool {
        matches!(mode, MatchMode::Skirmish | MatchMode::Range)
            || self.modes.iter().any(|declared| declared.mode() == mode)
    }

    /// **The ground's Hold rule**, the first declared (the loader refuses a
    /// second), or `None` for a ground that declares none.
    #[must_use]
    pub fn hold_rule(&self) -> Option<HoldRule> {
        self.modes.first().map(|declared| match declared {
            ArenaMode::Hold(rule) => *rule,
        })
    }

    /// The definition's SHA-256 identity over a canonical byte encoding.
    ///
    /// The encoding, in order, with every multi-byte integer little-endian:
    ///
    /// 1. [`DEFINITION_DIGEST_DOMAIN`];
    /// 2. the id's byte length as `u64`, then its bytes;
    /// 3. `width`, `height`, `cell_cm`, each `u16`;
    /// 4. the cell count as `u64`, then per cell row-major: the class's byte
    ///    code as `u8`, `elev_cm` as `i16`, `move_cost` as `u16`;
    /// 5. the zone count as `u64`, then per zone in declared order: the team's
    ///    byte (`0` red, `1` blue) then `x0`, `y0`, `x1`, `y1` as `u16`;
    /// 6. **(16b cut N.1)** the key-spot count as `u64`, then per spot in declared
    ///    order: the name's byte length as `u64`, its bytes, `x` and `y` as `u16`;
    /// 7. **(16b cut N.1)** the waypoint-node count as `u64`, then per node in
    ///    declared order: the id's byte length as `u64`, its bytes, `x` and `y` as
    ///    `u16`, the neighbour count as `u64` and each neighbour index as `u16`.
    ///
    /// Every variable-length run is length-prefixed, so no two different
    /// definitions share an encoding.
    ///
    /// **Why the graph is in it** (16b cut N.1): the digest's job is that *a
    /// replayed ledger can be told to belong to the terrain it was produced on*,
    /// and from cut N a unit's route is a function of the waypoint graph as much
    /// as of the cells. Two windows with one terrain and two graphs produce two
    /// different matches, so they are two different definitions.
    ///
    /// **Why [`ArenaDefinition::modes`] is NOT in it** (16b cut O.1, plan item
    /// 9): the modes are a menu, not ground. A replayed ledger is told its
    /// ground by the cells, the zones, the spots and the graph; the mode one
    /// match was fought under is the record's and the replay's `"mode"` and the
    /// state hash's item 12. So the proving ground's version 4, which only
    /// declares Hold, digests exactly as version 3 did.
    ///
    /// **Why the terrain's per-row class table is NOT in it** (16b cut AE.1):
    /// the wheeled and legged prices of a cell are a function of its class,
    /// which item 4 folds, and of the shipped locomotion table, which is code
    /// in the way the slope rule is code. So a definition priced per row digests
    /// exactly as it did when every cell carried the tracked price alone.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(DEFINITION_DIGEST_DOMAIN);
        sha.update((self.id.len() as u64).to_le_bytes());
        sha.update(self.id.as_bytes());
        sha.update(self.terrain.width().to_le_bytes());
        sha.update(self.terrain.height().to_le_bytes());
        sha.update(self.terrain.cell_cm().to_le_bytes());
        sha.update((self.terrain.cells().len() as u64).to_le_bytes());
        for cell in self.terrain.cells() {
            sha.update([cell.class as u8]);
            sha.update(cell.elev_cm.to_le_bytes());
            sha.update(cell.move_cost.to_le_bytes());
        }
        sha.update((self.entry_zones.len() as u64).to_le_bytes());
        for zone in &self.entry_zones {
            sha.update([match zone.team {
                Team::Red => 0_u8,
                Team::Blue => 1_u8,
            }]);
            sha.update(zone.x0.to_le_bytes());
            sha.update(zone.y0.to_le_bytes());
            sha.update(zone.x1.to_le_bytes());
            sha.update(zone.y1.to_le_bytes());
        }
        sha.update((self.key_spots.len() as u64).to_le_bytes());
        for spot in &self.key_spots {
            sha.update((spot.name.len() as u64).to_le_bytes());
            sha.update(spot.name.as_bytes());
            sha.update(spot.x.to_le_bytes());
            sha.update(spot.y.to_le_bytes());
        }
        let nodes = self.waypoints.nodes();
        sha.update((nodes.len() as u64).to_le_bytes());
        for (at, node) in nodes.iter().enumerate() {
            sha.update((node.id.len() as u64).to_le_bytes());
            sha.update(node.id.as_bytes());
            sha.update(node.x.to_le_bytes());
            sha.update(node.y.to_le_bytes());
            let neighbours = self
                .waypoints
                .neighbours(u16::try_from(at).unwrap_or(u16::MAX));
            sha.update((neighbours.len() as u64).to_le_bytes());
            for neighbour in neighbours {
                sha.update(neighbour.to_le_bytes());
            }
        }
        sha.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Mobility and condition
// ---------------------------------------------------------------------------

/// The `tracked` mobility-mode rules of doc 16 §3.3 "Mobility mode rules",
/// decoded out of M32's opaque `ModeRules` by the game crate.
///
/// All-zero (the [`Default`]) is a unit that cannot move at all, which is what a
/// wreck and an unmodelled hull get.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedRules {
    /// The forward ceiling, cm/s.
    pub max_speed_cms: u32,
    /// Acceleration, cm/s².
    pub accel_cms2: u32,
    /// Braking, cm/s² — the rate used when the magnitude of the speed falls.
    pub brake_cms2: u32,
    /// Turn rate, millidegrees per second.
    pub turn_rate_mdeg_s: u32,
    /// Reverse ceiling as a percentage of `max_speed_cms`.
    pub reverse_factor_pct: u8,
}

impl TrackedRules {
    /// The reverse ceiling in cm/s (a non-negative magnitude).
    #[must_use]
    pub const fn reverse_ceiling_cms(&self) -> u32 {
        self.max_speed_cms
            .saturating_mul(self.reverse_factor_pct as u32)
            / 100
    }
}

/// What damage has taken away from a unit's motion — doc 16 §3.3's `slow(50)`,
/// `no_turn_toward(left)`, `disable_motion` and `wreck` consequences, read off
/// the M32 unit by the game crate and cached here so a substep of motion needs
/// no section walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionLimits {
    /// The surviving fraction of the speed cap, per cent; `100` is no slow.
    pub slow_pct: u8,
    /// The unit can no longer turn toward its **left** (anticlockwise): a
    /// destroyed left track, doc 16 §3.3's `no_turn_toward(left)`.
    ///
    /// **16b cut M.4 (16c `B-47`)**: the two sides are two flags, not one
    /// `Option<Side>`, because a tank can lose both tracks and the second loss
    /// must not overwrite the first. Damage only ever sets a flag (it is ORed in,
    /// never cleared inside a match). **The pair rule**: with one side closed a
    /// turn whose short arc goes that way takes the long way round; with **both**
    /// closed the unit does not turn at all
    /// (`crate::arena::motion`'s `turn`). The shipped tanks also author
    /// `disable_motion_when_all_of: [left_track, right_track]`, so a tank on no
    /// tracks is `motion_disabled` too; the flags hold the rule for a pack that
    /// does not.
    pub no_turn_left: bool,
    /// The unit can no longer turn toward its **right** (clockwise): a destroyed
    /// right track. The same pair rule as [`MotionLimits::no_turn_left`].
    pub no_turn_right: bool,
    /// Motion is disabled outright (a destroyed engine).
    pub motion_disabled: bool,
    /// The unit is a wreck (doc 16 §3.3): an obstacle and a contact, never a
    /// target, and no longer a subject of any order.
    pub wreck: bool,
}

impl Default for MotionLimits {
    /// Undamaged: full speed, both tracks, motion enabled, not a wreck.
    fn default() -> MotionLimits {
        MotionLimits {
            slow_pct: 100,
            no_turn_left: false,
            no_turn_right: false,
            motion_disabled: false,
            wreck: false,
        }
    }
}

impl MotionLimits {
    /// Close `side` to turning — the `no_turn_toward(side)` consequence, **ORed
    /// in** (16b cut M.4, 16c `B-47`): closing one side never reopens the other,
    /// so a tank that loses its left track and then its right is closed both
    /// ways, whichever order the two losses land in.
    pub fn close_turn(&mut self, side: Side) {
        match side {
            Side::Left => self.no_turn_left = true,
            Side::Right => self.no_turn_right = true,
        }
    }

    /// Whether the unit may no longer turn toward `side`.
    #[must_use]
    pub const fn turn_closed(&self, side: Side) -> bool {
        match side {
            Side::Left => self.no_turn_left,
            Side::Right => self.no_turn_right,
        }
    }
}

/// **The fractions of a turn, of a speed change and of a distance that a substep
/// did not spend** (16b cut Q.6; doc 16 §3.1 "Rate independence", §3.4).
///
/// Motion is three rates over arena time integrated into integer state, and
/// before this cut each of the three was written `rate × dt / 1000` and the
/// remainder thrown away — three truncations per unit per substep. That makes the
/// *substep length* an input to the answer: a tank at 1,500 cm/s crossing a
/// 250 ms substep travels 375 cm, and the same tank across five 50 ms substeps
/// travels five times 75, which agrees only because those numbers happen to be
/// whole. They usually are not, and a crawl is the plainest case: a unit ordered
/// to 3 cm/s covered 0.75 cm in a 250 ms substep, which truncated to nothing, so
/// it stood still for as long as it was driven.
///
/// **X.6's five-against-one `blocked` count was read as the same defect and is
/// not one** (16b cut Q.6 measured it): the carries removed every truncation in
/// motion and that count did not move — 250 ms still prints one and 150 ms still
/// prints five. Its cause is the *two flags* cut F.2 split deliberately, and
/// `crate::arena::motion`'s `one_obstacle_is_one_stop_at_every_partition` test
/// measures it rather than asserting it away.
///
/// So each rate keeps its remainder, exactly the way
/// [`HeatState::carry_milli`](crate::arena::state::HeatState::carry_milli) keeps
/// the fraction of a heat unit that a substep did not shed
/// ([`crate::arena::heat`]'s head is the argument, written once). The total
/// turned, gained or travelled over any interval is then the same however that
/// interval is cut — see [`crate::arena::motion`]'s head for exactly which
/// motions that makes exact and which it does not.
///
/// **Every field is folded into [`crate::arena::hash::state_hash`]**, because a
/// carry is the fractional part of authoritative state: two replays whose carries
/// differ are two replays that are about to disagree about a whole centimetre.
///
/// **A refused substep banks nothing.** Every stop, every wreck, every dead
/// engine and every cold hull zeroes what it refused, which is
/// [`crate::arena::heat`]'s own rule for a unit that has gone cold: a carry
/// belongs to arena time that was actually spent moving.
///
/// **Its serde derive is the snapshot's** (16b cut AF.1): until then it carried
/// none, because [`UnitState`] had none and nothing in the tree encoded one (16b
/// cut Q.6, fixer). The snapshot carries every carry field by field, since each
/// is authoritative state the hash folds; no wire form a pilot or a ledger reads
/// carries one, and those keep their own words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionCarry {
    /// Millidegrees × 1,000 not yet turned, in `0..1000`.
    pub turn: u32,
    /// cm/s × 1,000 not yet gained or shed, in `0..1000`.
    pub speed: u32,
    /// Centimetres × `1000 << 20` not yet travelled eastward, in `0..(1000 << 20)`
    /// — the scale is the millisecond's thousand times
    /// [`SIN_ONE`](crate::arena::motion::SIN_ONE)'s fixed point, because the east
    /// component is `speed × dt × sin` and neither division is taken until the
    /// carry has been added.
    pub east: i64,
    /// The same northward.
    pub north: i64,
    /// **Which way [`MotionCarry::turn`] was banked** (16b cut M.4, 16c `T-16`):
    /// `0` none, `1` left (anticlockwise), `2` right (clockwise). A carry is a
    /// fraction of a rate spent in one direction; a turn that reverses direction
    /// starts from nothing, so `crate::arena::motion`'s `turn` zeroes the carry
    /// whenever this substep turns the other way from the one recorded here, and
    /// records the new side. Reset to `0` with the carry on every return that
    /// zeroes it.
    pub turn_side: u8,
    /// **Which kind of speed change [`MotionCarry::speed`] was banked by** (16b
    /// cut M.4, 16c `T-16`): `0` none, `1` accelerating (`accel_cms2`), `2`
    /// braking (`brake_cms2`). A thousandth of a cm/s banked while speeding up is
    /// not owed to a brake, so `accelerate` zeroes the carry on a flip between
    /// the two and records the new kind. Reset to `0` with the carry.
    pub speed_rate: u8,
}

/// **A unit standing against another unit's cell, and since when** (16b cut E.3;
/// 16c `B-46`, §5's Q10) — the state [`UnitState::occupied_hold`] carries, whose
/// doc is the rule.
///
/// The cell is the one the crossing was refused, not the one the unit is standing
/// in, because the hold is about *what is in the way*: a unit that turns and is
/// refused a different cell has a different hold, and the clock starts again.
/// `since_ms` is the arena millisecond the substep that opened the hold **ended**
/// at, so the maturity test is arena time on both sides and never a count of
/// pulses (doc 16 §3.1, D-16-2).
///
/// **Its serde derive is the snapshot's** (16b cut AF.1), for
/// [`MotionCarry`]'s reason: the hold is hashed state the snapshot carries, and
/// no wire form a pilot or a ledger reads carries one — those keep their own
/// words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupiedHold {
    /// The cell the crossing was refused, `(x, y)`.
    pub cell: (u16, u16),
    /// The arena millisecond the substep that opened the hold ended at.
    pub since_ms: i64,
}

/// A unit's power state (doc 16 §3.3 "Startup").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Power {
    /// Cold. Motion and fire refuse.
    #[default]
    Off,
    /// `STARTUP` has been accepted and `match.startup_ms` has not elapsed.
    Starting,
    /// Running.
    Online,
}

impl Power {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Power::Off => "off",
            Power::Starting => "starting",
            Power::Online => "online",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Power::Off => 0,
            Power::Starting => 1,
            Power::Online => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// The seam message
// ---------------------------------------------------------------------------

/// How high a unit's eye sits when nothing says otherwise, centimetres (doc 16
/// §3.2 "Line of sight": *`sensor_height_cm` (new field; 150 by default)*).
pub const DEFAULT_SENSOR_HEIGHT_CM: u32 = 150;

/// **The rounds one weapon's own `ammo` cell carries** (16b cut F.1), or `None`
/// when the unit declares no such cell.
///
/// The mirror of [`crate::arena::Arena::returned_units`]'s fold: that
/// writes the arena's live count back into `component/<weapon>/ammo` at check-in,
/// and this reads it at check-out, so a match opens on the magazine the last one
/// left. Negative values cannot occur (a cell is bounded) and are floored at zero
/// rather than wrapping.
fn rounds_on_record(unit: &mush_tactics::TacticalUnit, weapon: &SmolStr) -> Option<u32> {
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new(weapon.as_str()).ok()?,
        cell: mush_tactics::CellId::new(crate::definitions::unit::AMMO_CELL).ok()?,
    };
    let held = unit.cell(&addr)?;
    Some(u32::try_from(held.current.max(0)).unwrap_or(u32::MAX))
}

/// **The speed ceiling of a set of units on a window whose cheapest passable
/// cell costs `min_cost[row]` for each row**, cm/s (16b cut M.4, 16c `T-15`;
/// re-derived per row by 16b cut AE.1): the greatest, over the units that are
/// neither a wreck nor disabled, of `max_speed_cms × 100 / min(100,
/// min_cost[unit.cost_row])`. Written once for [`Arena::max_speed_cms`] and for
/// [`Arena::new`]'s substep refusal, which runs before there is an [`Arena`] to
/// ask.
///
/// **Per unit, on its own row** (16b cut AE.1): the terrain factor a unit
/// drives under is its own row's, so the cell it could go fastest in is the
/// cheapest its own row prices. On the shipped ground the tracked and legged
/// rows' cheapest passable price is 100 and the wheeled row's 80 (a road), so a
/// 3,000 cm/s car answers 3,750 and a 1,500 cm/s tank beside it 1,500: the
/// car's cheap road is no margin the tank needs.
fn speed_ceiling_cms(units: &[UnitState], min_cost: &[u32; 3]) -> u32 {
    units
        .iter()
        .filter(|state| !state.limits.wreck && !state.limits.motion_disabled)
        .map(|state| {
            let cheapest = u64::from(
                min_cost
                    .get(state.cost_row.index())
                    .copied()
                    .unwrap_or(1)
                    .clamp(1, 100),
            );
            u32::try_from(u64::from(state.mobility.max_speed_cms).saturating_mul(100) / cheapest)
                .unwrap_or(u32::MAX)
        })
        .max()
        .unwrap_or(0)
}

/// **Which of a unit's sections carry [`crate::arena::resolution::Consequence::DestroyUnit`]**,
/// one bit each (16b cut E.2, `B-45`) — the value of [`UnitState::fatal_sections`].
///
/// The walk is `TacticalUnit::sections()` in its own order, truncated to
/// [`crate::arena::condition::MAX_SCANNED_SECTIONS`], because that is the order
/// and the length [`crate::arena::condition::Condition::of`] packs a reading in:
/// the mask exists to be ANDed with one, so the two have to be counted the same
/// way or bit `i` of one would be a different section from bit `i` of the other.
///
/// A section the profile does not describe contributes nothing, exactly as
/// [`crate::arena::damage::entry_consequences`] skips one.
///
/// **17b cut M1a.3** made it `pub(crate)`: a frame's `reads_dead` is computed
/// over the **target chassis's** definition and profile through this one
/// function (doc 17 D-UM-5; departure (j)) — content, never the live wreck
/// flag — and [`fatal_section_words`] names the bits it sets.
pub(crate) fn fatal_section_mask(
    unit: &mush_tactics::TacticalUnit,
    profile: &crate::arena::resolution::UnitProfile,
) -> u32 {
    let mut mask = 0_u32;
    for (index, (id, _)) in unit
        .sections()
        .enumerate()
        .take(crate::arena::condition::MAX_SCANNED_SECTIONS)
    {
        let fatal = profile.section(id.as_str()).is_some_and(|row| {
            row.consequences.iter().any(|consequence| {
                matches!(
                    consequence,
                    crate::arena::resolution::Consequence::DestroyUnit
                )
            })
        });
        if fatal {
            mask |= 1_u32 << index;
        }
    }
    mask
}

/// **The section words [`fatal_section_mask`] sets a bit for**, in section order
/// (17b cut M1a.3; departure (j)): what a frame publishes beside a held reading
/// as `FrameContact::fatal`, so a pilot reads the rule `reads_dead` applies.
///
/// The same walk over the same chassis, truncated the same way: the answer is
/// the words of exactly the sections whose bits the mask sets, in section order,
/// so the list and the mask cannot disagree. It is **not** indexed by bit — the
/// stock tank's answer is `["hull"]`, word 0, for the mask's bit 1 — so a reader
/// that wants a section's bit tests the mask, and one that wants its word
/// searches the list.
pub(crate) fn fatal_section_words(
    unit: &mush_tactics::TacticalUnit,
    profile: &crate::arena::resolution::UnitProfile,
) -> Vec<SmolStr> {
    let mask = fatal_section_mask(unit, profile);
    unit.sections()
        .enumerate()
        .take(crate::arena::condition::MAX_SCANNED_SECTIONS)
        .filter(|(index, _)| mask & (1_u32 << index) != 0)
        .map(|(_, (id, _))| SmolStr::new(id.as_str()))
        .collect()
}

/// **A unit's chassis-static words, read once** (16c `T-105`, the VPS spike
/// of 2026-10-09 continued on 2026-10-12).
///
/// Every frame a pilot or a mind is given carries, per contact row, three
/// facts of the *target's* chassis that nothing in a match moves: which of
/// its sections are the body (`fatal_section_mask`), those sections' names
/// (`fatal_section_words`) and its section vocabulary in its own order (the
/// scan table [`crate::arena::step::Frame::scan_tables`] is built from).
/// `Arena::frame_scope` used to recompute all three for every row of every
/// frame — a walk of the sections and the profile's consequence rows, then
/// the `SmolStr` allocations — about 74 mind views and 24 pilot frames a wake
/// at 300 units, which the spike measured at about 3 per cent of the authored
/// step on the conclav.io VPS. Here they are read once per unit, from the
/// same two functions over the same two inputs, so every frame's bytes are
/// the ones the per-row computation produced: this is a cache of a pure
/// function of carried state, not a new fact.
///
/// The table lives on [`Arena::chassis`], one row per unit in
/// [`Arena::units`]' order (the unit table is laid down once by [`Arena::new`]
/// and never reordered), and is **re-derived** — by [`Arena::new`] and by the
/// snapshot decode (`crate::arena::snapshot`'s re-derived list) — never
/// carried and never hashed, as [`UnitState::fatal_sections`] is never hashed:
/// a function of the definition and the fit the record carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChassisWords {
    /// `fatal_section_mask` of the unit and its profile.
    pub fatal_mask: u32,
    /// `fatal_section_words` of the unit and its profile.
    pub fatal_words: Vec<SmolStr>,
    /// Every section's name in the unit's own order — the scan table a
    /// frame's `FrameCondition::table` indexes (all sections, not the scanned
    /// prefix: the table names the whole chassis).
    pub section_names: Vec<SmolStr>,
}

impl ChassisWords {
    /// The three words of one unit, off its chassis and its profile.
    #[must_use]
    pub(crate) fn of(
        unit: &mush_tactics::TacticalUnit,
        profile: &crate::arena::resolution::UnitProfile,
    ) -> Self {
        Self {
            fatal_mask: fatal_section_mask(unit, profile),
            fatal_words: fatal_section_words(unit, profile),
            section_names: unit
                .sections()
                .map(|(section, _)| SmolStr::new(section.as_str()))
                .collect(),
        }
    }

    /// One row per unit, in the unit table's order.
    #[must_use]
    pub(crate) fn table(units: &[UnitState]) -> Arc<[ChassisWords]> {
        units
            .iter()
            .map(|state| Self::of(&state.unit, &state.profile))
            .collect()
    }
}

/// **What a unit does with heat** (16b cut X.1; doc 16 §9 item 1: *a `heat` cell
/// and dissipation appear on the unit*).
///
/// Everything about heat that is **not** the number itself. The number is an M32
/// cell — `heat` on the unit's body section — because doc 16 §3.5 makes the M32
/// unit the truth of a unit's condition cells and because the check-in hands the
/// cells back whole; this is the arena's map to it, read once at check-out.
///
/// It is read and not carried across the seam. `CustodyUnit` is a struct six test
/// files build literally, so a new field on it would be a diff in every one of
/// them; the numbers are already on the `TacticalUnit` that crosses beside it, and
/// [`HeatState::read`] finds them there. The same reasoning is why the shot's own
/// cost is a cell on the weapon's component rather than a field on
/// [`WeaponSummary`].
///
/// A unit whose content declares no ceiling has **no** `HeatState` at all, which
/// is what makes the per-substep dissipation an `Option` test for every stock unit
/// in the harness's field of 300.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatState {
    /// The section the `heat` and `heat_sink` cells sit on — found rather than
    /// derived, so the rule for *which* section lives in one place
    /// (`crate::definitions::unit::UnitSpec::heat_section`).
    pub section: SmolStr,
    /// The ceiling, off the `heat` cell's own maximum. `FIRE` refuses at it.
    pub max: u32,
    /// What the unit sheds per second of arena time, off the `heat_sink` cell.
    pub per_s: u32,
    /// What one shot of each weapon adds, by the weapon's component id; a weapon
    /// with no row makes no heat.
    pub per_shot: BTreeMap<SmolStr, u32>,
    /// **The thousandths of a heat unit the substeps have shed and not yet
    /// spent**, `0..1000`.
    ///
    /// Doc 16's model is rate-independent (D-16-2) and integer (D-16-4), and the
    /// two pull against each other here: `tempo.substep_max_ms` is 250, so
    /// `per_s × dt_ms / 1000` truncates to nothing for any rate under four a
    /// second and a tank at `Engaged` would never cool at all. Carrying the
    /// remainder makes the total shed over any interval exactly
    /// `per_s × ms / 1000` **however the interval is chopped**, which is the
    /// property the cross-tempo test asserts of everything else in the model.
    ///
    /// It is arena-side: it is reset with the heat it belongs to and never leaves
    /// the match. **It is folded into [`crate::arena::hash::state_hash`]** since
    /// 16b cut M.3 (16c `T-06`), beside the heat cell whose fractional part it is:
    /// heat gates firing and moves every substep with no ledger line, which is
    /// cut Q.6's argument for the motion carries. (Until then this sentence said
    /// it was not folded, for the opaque `TacticalUnit`'s reason — which was never
    /// this number's: the carry is not in the unit, so no journal held it.)
    pub carry_milli: u32,
}

impl HeatState {
    /// **Read one unit's heat numbers off its cells** (16b cut X.1), or `None`
    /// for a unit that carries no `heat` cell — every stock unit.
    ///
    /// The section is found by probing each section the unit's profile declares
    /// for the cell, so `bind`'s choice of body section is authoritative and this
    /// never re-derives it. A unit with no profile (every hand-made rig) has no
    /// section to probe and no heat.
    #[must_use]
    pub fn read(
        unit: &TacticalUnit,
        profile: &UnitProfile,
        weapons: &[WeaponSummary],
    ) -> Option<HeatState> {
        let (section, cell) = profile.sections.iter().find_map(|section| {
            let addr = heat_addr(section.id.as_str(), crate::definitions::unit::HEAT_CELL)?;
            unit.cell(&addr).map(|cell| (section.id.clone(), *cell))
        })?;
        let per_s = heat_addr(section.as_str(), crate::definitions::unit::HEAT_SINK_CELL)
            .and_then(|addr| unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.current.max(0)).unwrap_or(0));
        let per_shot = weapons
            .iter()
            .filter_map(|weapon| {
                let addr = mush_tactics::CellAddr::Component {
                    component: mush_tactics::ComponentId::new(weapon.id.as_str()).ok()?,
                    cell: mush_tactics::CellId::new(crate::definitions::unit::HEAT_PER_SHOT_CELL)
                        .ok()?,
                };
                let value = u32::try_from(unit.cell(&addr)?.current.max(0)).ok()?;
                (value > 0).then(|| (weapon.id.clone(), value))
            })
            .collect();
        Some(HeatState {
            section,
            max: u32::try_from(cell.max.max(0)).unwrap_or(0),
            per_s,
            per_shot,
            carry_milli: 0,
        })
    }

    /// The address of one of this unit's heat cells.
    #[must_use]
    pub fn addr(&self, cell: &str) -> Option<mush_tactics::CellAddr> {
        heat_addr(self.section.as_str(), cell)
    }

    /// What one shot of `weapon` adds; `0` for a weapon that makes no heat.
    #[must_use]
    pub fn per_shot(&self, weapon: &str) -> u32 {
        self.per_shot.get(weapon).copied().unwrap_or(0)
    }
}

/// One section-cell address, or `None` for an id `mush-tactics` will not take.
fn heat_addr(section: &str, cell: &str) -> Option<mush_tactics::CellAddr> {
    Some(mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new(section).ok()?,
        cell: mush_tactics::CellId::new(cell).ok()?,
    })
}

/// **One weapon, as the check-out hands it over** (16b cut S.1; doc 16 §3.5
/// "Weapon definition").
///
/// Cut S needs exactly one number off it — the longest `max_range_cm` in the
/// match, which is what `tempo.alert_range_cm` is derived from when the key is
/// zero (16b §9) — and cut W needs all of them, so the whole row crosses the seam
/// once at check-out rather than twice. Every field is an integer (D-16-4).
///
/// **16b cut W.1 added [`WeaponSummary::bands`] and
/// [`WeaponSummary::count_per_load`]**, which are the two numbers a shot needs and
/// S.1 had no reader for: the load sizes the unit's `ammo` cell at check-out, and
/// the bands carry the per-band `dispersion_mrad` W.2's error cone is built from.
/// The row is the **definition**, immutable for the life of the match; what a shot
/// changes is the two live cells beside it on [`UnitState`]
/// (`ready_at_ms` and `ammo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponSummary {
    /// The component id the pilot names in `FIRE <weapon>`.
    pub id: SmolStr,
    /// The outermost range band's edge, centimetres. Equal to the largest
    /// [`WeaponBand::max_range_cm`] of [`WeaponSummary::bands`]; carried
    /// separately because it is the one number cut S reads and a check-out with no
    /// authored bands still has a reach.
    pub max_range_cm: u32,
    /// The cycle, milliseconds (floor 1,000, W14 — asserted by the content loader,
    /// `crate::definitions::weapon::CYCLE_FLOOR_MS`, and taken as authored here).
    pub cycle_ms: u32,
    /// The ammunition family it fires.
    pub ammunition: SmolStr,
    /// Rounds per pull.
    pub burst_rounds: u32,
    /// Muzzle speed, cm/s; zero exactly for a beam (doc 16 §3.5: a beam's
    /// `impact_at_ms` is `now`).
    pub muzzle_speed_cms: u32,
    /// **The range bands, nearest first** (16b cut W.1; doc 16 §3.5 step 2), each
    /// with the weapon's own error cone in that band.
    pub bands: Vec<WeaponBand>,
    /// **What one round does** (16b cut W.2; doc 16 §3.5 "Ammunition definition":
    /// *`damage` (one fixed value)*), off the ammunition the weapon names — the
    /// second field of this row that is the ammunition's and not the weapon's.
    /// Zero for a weapon whose ammunition the catalog does not hold, which is a
    /// weapon that arrives and does nothing rather than one that refuses.
    pub damage: u32,
    /// **The section it is mounted in** (16b cut W.2; doc 16 §3.5 "Weapon
    /// definition": *`mount` (section and facet)*), which is what
    /// `remove_weapons(section)` names when a turret is shot off.
    pub mount_section: SmolStr,
    /// **How many rounds one load holds** (16b cut W.1), off the ammunition
    /// definition. It is what [`Arena::new`] sizes the unit's `ammo` cell from and
    /// what `WEAPONSTATUS` prints as the denominator.
    pub count_per_load: u32,
}

/// **Who is flying a unit** (16b cut N.1; doc 16 §6, §3.8 "Disconnects"; doc 07
/// §1.1's fallback controller).
///
/// A unit attribute at custody rather than a property of the match, because the
/// two coexist in one match: a lobby that fields fewer pilots than tanks puts
/// [`Controller::Mind`] on the empty pods (the reflex controller's until 17b
/// cut M4.3), and a seat released past the linkdead grace hands one tank over
/// mid-match while the others keep their pilots.
///
/// **17b cut M4.6** deleted the reflex controller and its variant
/// (`Controller::Reflex`, code `1`, word `reflex`): the two that are left keep
/// their codes — [`Controller::Pilot`] `0` and [`Controller::Mind`] `2` — so
/// every unit of every match folds the byte it always did, and code `1` is
/// never reused. A snapshot that carried the word is format 8, refused by name
/// (`crate::arena::snapshot::SNAPSHOT_FORMAT`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Controller {
    /// A player's verbs are the only source of this unit's orders.
    #[default]
    Pilot,
    /// **A unit mind flies it** (17b cut M1a.5; doc 17 §2, D-UM-2): a
    /// [`crate::arena::mind::MindRunner`] reads the hull's own view at each
    /// [`DueKind::MindThink`] and its intents land one substep later at
    /// [`DueKind::MindLand`] (`crate::arena::mind`). Since 17b cut M4.3 the
    /// fill and every released hull are minds, and since 17b cut M4.6 the
    /// only machine there is.
    Mind,
}

impl Controller {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Controller::Pilot => "pilot",
            Controller::Mind => "mind",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Controller::Pilot => 0,
            // 17b cut M1a.5: a new byte, so every unit of every existing match
            // folds the byte it always did (the M1a plan's head, "the
            // production hash"). 17b cut M4.6 deleted the reflex's `1` and
            // renumbered nothing: `1` is retired, never reused.
            Controller::Mind => 2,
        }
    }

    /// **Whether a machine flies it** (17b cut L.2; D-UM-2, D-UM-53): a unit
    /// mind, never a pilot (the reflex controller or a unit mind until 17b cut
    /// M4.6 deleted the reflex). What both [`crate::deliberation::squads_of`]
    /// callers deal squads by, so a [`Controller::Mind`] hull rides in a squad
    /// under its lance-leader and an order reaches it through the unchanged
    /// Order path (D-UM-15). A pilot's tank still takes no order
    /// ([`crate::arena::intents::Refusal::NotMachine`]).
    #[must_use]
    pub const fn is_machine(&self) -> bool {
        matches!(self, Controller::Mind)
    }
}

/// One unit as it is handed over at check-out (doc 16 §3.1 "Custody").
#[derive(Clone, Debug)]
pub struct CustodyUnit {
    /// The world object the unit is.
    pub id: ObjId,
    /// Its side.
    pub team: Team,
    /// Its pose at check-out, which must be on the window.
    pub pose: Pose,
    /// Its decoded `tracked` rules.
    pub mobility: TrackedRules,
    /// **How high its eye sits**, centimetres (doc 16 §3.2; 16b cut S.1). Zero is
    /// read as [`DEFAULT_SENSOR_HEIGHT_CM`].
    pub sensor_height_cm: u32,
    /// **What it sees with** (16b cut S.1; doc 16 §3.6). `None` runs no sensor
    /// pass at all.
    pub sensor: Option<SensorProfile>,
    /// **What it shoots with** (16b cut S.1 carries it; 16b cut W.1 reads it).
    pub weapons: Vec<WeaponSummary>,
    /// **What it presents and what a hit may strike** (16b cut W.2; doc 16 §3.3
    /// "Facet attribution" and "Silhouette"), decoded off the authored `UnitSpec`
    /// at check-out. An empty profile presents nothing and is struck by nothing,
    /// which is what every fixture from cut A carries.
    pub profile: UnitProfile,
    /// **The gunnery rating resolving its shots** (16b cut W.2; doc 16 §3.5 step
    /// 2's `PilotProfile` seam), 1 to 5. Zero is read as
    /// [`crate::arena::resolution::STOCK_GUNNERY`] — *every stock rating is 1*.
    pub gunnery: u8,
    /// **Who flies it** (16b cut N.1). The check-out sets [`Controller::Mind`]
    /// for a unit with no seated pilot (the reflex controller's until 17b cut
    /// M4.3) and [`Controller::Pilot`] otherwise.
    pub controller: Controller,
    /// **The centre slot this unit was fielded from**, 1-based (16b cut Q.1), or
    /// `0` for a unit no lobby placed — a staff `@arena/start`, a fixture, the
    /// harness's field of three hundred.
    ///
    /// It is what [`Designator::Slot`] renders, so a pilot's `BA` is the pod
    /// `SIM STATUS` prints as `1` on blue. A `0` is not a designator: [`Arena::new`]
    /// gives every such unit the lowest free slot of its team in unit-id order, so
    /// a harness unit and a staff-fielded tank have stable callsigns with no lobby
    /// at all.
    pub slot: u8,
    /// **The seal it is checked out under** (17b cut M3.3; doc 17 §13, D-UM-45,
    /// D-UM-46) — beside `sensor`, `weapons` and `gunnery`, for every unit,
    /// piloted or not, armed or not. Computed LAST, from the finished custody, by
    /// `crate::match_::seal::seal_for` (through `crate::match_::custody_unit`,
    /// `CustodyUnit::reseal` and, for a hand-built literal, `CustodyUnit::sealed`);
    /// [`Arena::new`] keeps its reference on [`UnitState::seal`] and, for a
    /// mind's hull, its digest on the mind's row.
    pub seal: crate::arena::mind::MindSeal,
    /// M32 unit truth, carried opaque: cut A applies no plan to it (D-16-6).
    pub unit: TacticalUnit,
}

impl CustodyUnit {
    /// **What the squads are dealt from** (16b cut N.2; 17b cut L.2, D-UM-2,
    /// D-UM-53): one `(unit, team, is_machine)` triple per check-out unit, in
    /// check-out order, the bool [`Controller::is_machine`] — a mind's hull (a
    /// reflex hull's too until 17b cut M4.6), never a pilot's. The ONE builder of
    /// [`crate::deliberation::squads_of`]'s input: [`Arena::new`] and the match
    /// record's `match_::start` both call it over the same check-out, so the
    /// record's grouping and the arena's are one function of one data and
    /// cannot drift apart on who is a machine.
    #[must_use]
    pub fn squad_input(units: &[CustodyUnit]) -> Vec<(ObjId, Team, bool)> {
        units
            .iter()
            .map(|unit| (unit.id, unit.team, unit.controller.is_machine()))
            .collect()
    }
}

/// One seated pilot at check-out (doc 16 §3.1's `pilots[{unit, station, epoch}]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pilot {
    /// The tank the station is on (doc 16 §3.3 "Stations are on the tank").
    pub unit: ObjId,
    /// The character in the seat.
    pub pilot: ObjId,
    /// The station's owner-local id.
    ///
    /// **Unknown to the arena after check-out** (16b cut Y.3): every channel's
    /// epoch arrives by its own `Control`, so `station` is read by nothing but
    /// [`Arena::new`] (which raises the epoch table off it); a seat re-added by
    /// [`Arena::seat_pilot`] on a reclaim carries the empty string.
    pub station: SmolStr,
    /// The authority epoch the seat was granted under.
    pub epoch: u64,
}

/// **The check-out message** (doc 16 §3.1 "The seam is seven messages"): the
/// arena definition, the units, the seated pilots and the match seed.
///
/// It is the only way state enters an [`Arena`]. The seed is the key of every
/// draw the match makes ([`crate::arena::rng`]).
#[derive(Clone, Debug)]
pub struct CustodyOut {
    /// The window the match runs on.
    pub definition: ArenaDefinition,
    /// The units, in any order; [`Arena::new`] sorts them.
    pub units: Vec<CustodyUnit>,
    /// The seated pilots.
    pub pilots: Vec<Pilot>,
    /// **The resolution table every shot of this match is resolved through** (16b
    /// cut W.2; doc 16 §3.5 steps 2 and 3), decoded off the content at check-out.
    ///
    /// The one content type the arena carries whole rather than in a copy of its
    /// own, and deliberately: `ResolutionDef::hit_permille` *is* doc 16 §3.5 step
    /// 3's interpolation, written and tested beside the table it walks, and a
    /// second copy of that walk in here would be the one place the cockpit's
    /// percentage and the draw's threshold could come to disagree.
    pub resolution: ResolutionDef,
    /// The match seed.
    pub seed: u64,
    /// **The rule the match is fought under** (16b cut O.1; 16c N11): the
    /// lobby's `SIM MODE`, carried across the seam like the seed. [`Arena::new`]
    /// refuses a mode [`CustodyOut::definition`] does not declare
    /// ([`ArenaError::ModeUnsupported`]).
    pub mode: MatchMode,
}

// ---------------------------------------------------------------------------
// Unit state
// ---------------------------------------------------------------------------

/// One unit's arena state (doc 16 §4 "Arena state").
///
/// **16b cut AF.1: carried whole by its derive.** It has no re-derived and no
/// reset field — the fit constants (`cost_row`, `class`, `slope_limit_pct`,
/// `fatal_sections`, `jammer`, `eccm`), the hashed state, the per-substep
/// `blocked` flag, the band cache `condition` and the opaque `TacticalUnit`
/// alike — so a field added here is carried by the snapshot the day it is
/// added. The one way to drop one is a `#[serde(skip)]`, which is grep-able,
/// and the snapshot's own lists name it then (`crate::arena::snapshot`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitState {
    /// The world object the unit is. The units vector is sorted on it.
    pub id: ObjId,
    /// Its side.
    pub team: Team,
    /// **The name every viewer that has told it apart calls it by** (16b cut
    /// Q.1): always a [`Designator::Slot`], resolved once at check-out from
    /// [`CustodyUnit::slot`] and fixed for the match.
    ///
    /// It is **not** folded into [`crate::arena::hash::state_hash`] on its own: it
    /// is a pure function of the custody message, it never moves, and two replays
    /// of one check-out resolve it identically — the same argument
    /// `slope_limit_pct` makes.
    pub callsign: Designator,
    /// Its pose: whole centimetres and millidegrees (doc 16 §3.2).
    pub pose: Pose,
    /// Its current speed along its heading, cm/s. **Negative is reverse.**
    pub speed_cms: i32,
    /// The helm's ordered heading, millidegrees in `0..360_000`.
    pub desired_heading_mdeg: u32,
    /// The helm's ordered speed, cm/s, negative for reverse.
    pub desired_speed_cms: i32,
    /// Power (doc 16 §3.3 "Startup").
    pub power: Power,
    /// The decoded `tracked` rules.
    pub mobility: TrackedRules,
    /// **The steepest edge this unit's locomotion will cross**, per cent (16b
    /// cut X.6; doc 16 §9 item 6), or `None` for no limit at all.
    ///
    /// Read off the M32 unit at check-out by
    /// [`crate::arena::slope::SlopeLimit::read`], beside [`HeatState::read`] and
    /// the three sight reads, and for the same reason: it is a fact about the
    /// unit's fit that nothing in a match moves, and `CustodyUnit` is a struct a
    /// dozen test files build literally.
    ///
    /// `None` is **no limit**, not a limit of zero — a unit whose content
    /// declares no locomotion the shipped table knows still drives, and still
    /// pays the slope's surcharge. It is not folded into
    /// [`crate::arena::hash::state_hash`], for the reason `jammer` is not: it
    /// never moves, and what it *does* is the refusals the hash already folds
    /// through the poses.
    pub slope_limit_pct: Option<u32>,
    /// **Which row of the shipped locomotion table this unit pays** (16b cut
    /// AE.1; 16c `B-58`): its own driving mode's locomotion
    /// ([`CostRow::read`]), [`CostRow::Tracked`] for a unit that declares none.
    ///
    /// Read off the M32 unit at check-out beside [`UnitState::slope_limit_pct`]
    /// and for the same reasons: a fact about the fit that nothing in a match
    /// moves. Every reader that knows the unit — motion's terrain factor and
    /// refusal, the A\* and its heuristic scale, the reflex controller's
    /// open-step tests and goal pickers, the placement and opening tests —
    /// prices the ground through it ([`ArenaTerrain::cost`]). **Not folded
    /// into [`crate::arena::hash::state_hash`]**, for `slope_limit_pct`'s
    /// reason: it never moves, and what it does is the poses the hash already
    /// folds.
    pub cost_row: CostRow,
    /// **What kind of machine this is, for W2's cap and the end rules** (16b
    /// cut AE.3; doc 16 §2 W2).
    ///
    /// Read off the M32 unit at check-out by [`UnitClass::read`] — the class the
    /// unit's rules blob declares, else [`UnitClass::Walker`] for a legged
    /// driving mode, else [`UnitClass::Armour`] (every engine fixture) — beside
    /// [`UnitState::slope_limit_pct`] and [`UnitState::cost_row`] and for their
    /// reasons: a fact about the fit that nothing in a match moves. `Arena::new`
    /// counts it per side against `[tacsim.arena].max_units_by_class`;
    /// [`Arena::eliminated`] and [`Arena::hold_presence`] read
    /// [`UnitClass::Drone`] as the machine that decides nothing. **Not folded
    /// into [`crate::arena::hash::state_hash`]**, for `slope_limit_pct`'s
    /// reason: it never moves, and what it does — the end of a match, who holds
    /// the spot — the hash already folds through the outcome and the hold.
    pub class: UnitClass,
    /// **Which of this unit's sections kill it when they go**, one bit each (16b
    /// cut E.2, `B-45`).
    ///
    /// Bit `i` is set when section `i` of the unit's **own section order** —
    /// `unit.sections()`, which is the order every
    /// [`crate::arena::condition::Condition`] on this unit is packed in — carries
    /// [`crate::arena::resolution::Consequence::DestroyUnit`] in
    /// [`UnitState::profile`]. At most
    /// [`crate::arena::condition::MAX_SCANNED_SECTIONS`] bits, because that is
    /// how many bands a reading packs and a bit nothing can be read against
    /// would be a bit nothing can use.
    ///
    /// It is what turns a *reading* into the word "dead": a
    /// [`crate::arena::condition::Condition`] is packed bands with no idea which
    /// section is the body, so
    /// [`Arena::contact_reads_dead`] masks a hold's destroyed sections with this
    /// one. A unit whose profile names no fatal section has mask `0` and can
    /// never **read** as dead — only [`ShadowContact::known_dead`] can say so of
    /// it — which is the honest answer for content that authored no killing
    /// blow.
    ///
    /// Laid down once in [`Arena::new`] and never moved, and **not** folded into
    /// [`crate::arena::hash::state_hash`]: it is a fact about the unit's fit,
    /// which is the argument `slope_limit_pct` and `jammer` make.
    pub fatal_sections: u32,
    /// **The uphill slope of the edge this unit entered its cell by**, per cent
    /// (16b cut X.6; doc 16 §3.4 step 2).
    ///
    /// Written by [`crate::arena::motion::integrate`] on every accepted
    /// crossing, beside `pose.z_cm`, and read by the terrain factor of the next
    /// substep: a tank half way up a ramp is held to `max_speed × 100 /
    /// (move_cost × (100 + grade) / 100)`. `0` on flat ground, which makes the
    /// factor exactly what it was before this cut.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], unlike the
    /// limit: it is authoritative state with one writer, it moves without a
    /// ledger event of its own, and a unit can sit in a cell for many substeps
    /// after the crossing that set it.
    pub grade_pct: u16,
    /// **The cell this unit is standing against, and since when** (16b cut E.3;
    /// 16c `B-46`, §5's Q10), or `None` when it is standing against nothing.
    ///
    /// Doc 16 §3.4 step 4 stops a unit dead at every refused crossing, which is
    /// the right rule for ground and the wrong one for **traffic**: a follower
    /// whose id sorts below its leader's integrates first, reaches the cell the
    /// leader has not vacated yet, and is stopped — needing a dozen seconds to
    /// rebuild way, at the ford, every time. So an occupancy refusal opens a
    /// *hold* instead: the unit is clamped at the boundary keeping the speed it
    /// had, nothing is journalled and nobody is told. Only when the hold on one
    /// cell outlasts [`crate::config::ArenaConfig::occupied_stop_ms`] of arena
    /// time does it become step 4's literal stop, with its `blocked` line, its
    /// pilot notice and its lesson for the machine, exactly as before and two
    /// seconds later.
    ///
    /// One writer, [`crate::arena::motion::integrate`], which also clears it: on
    /// an accepted crossing, on a refusal of a different cell or of any other
    /// cause, on a substep that moved the unit without a refusal, on
    /// `desired_speed_cms == 0`, and on the wreck / dead-engine / cold-power stop.
    /// A substep that covered no whole centimetre leaves it alone, which is
    /// [`UnitState::held`]'s own rule.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], for `grade_pct`'s
    /// argument word for word: authoritative state with one writer that decides a
    /// later refusal and moves with no ledger line of its own.
    pub occupied_hold: Option<OccupiedHold>,
    /// **What the last substep of motion did not spend** (16b cut Q.6; doc 16
    /// §3.1 "Rate independence") — the fractional turn, speed change and distance
    /// that would otherwise be truncated away three times a substep.
    ///
    /// Written only by [`crate::arena::motion::integrate`], and folded into
    /// [`crate::arena::hash::state_hash`] field by field: see [`MotionCarry`].
    pub carry: MotionCarry,
    /// What damage has taken from its motion.
    pub limits: MotionLimits,
    /// How high its eye sits, centimetres (doc 16 §3.2 "Line of sight").
    pub sensor_height_cm: u32,
    /// What it sees with (doc 16 §3.6); `None` runs no pass.
    pub sensor: Option<SensorProfile>,
    /// **What its sight makes of a target's heat** (16b cut X.3; doc 16 §9 item
    /// 3), or `None` for a sight with no thermal channel — every stock unit, so
    /// the term costs the pass one `Option` test per candidate.
    ///
    /// Read off the cells at check-out rather than carried across the seam, for
    /// the reason [`HeatState`] gives: `CustodyUnit` and the sensor row are
    /// structs a dozen test files build literally.
    pub thermal: Option<ThermalSignature>,
    /// **The jamming pod it carries** (16b cut X.4; doc 16 §9 item 4), or `None`
    /// for a unit that mounts none — every stock unit, so a stock match's sensor
    /// pass never scans for one.
    ///
    /// Read off the cells at check-out for the reason [`ThermalSignature`] is, and
    /// not folded into [`crate::arena::hash::state_hash`] for the reason the sensor
    /// row is not: it is a fact about the unit's fit that nothing in a match moves,
    /// and what a pod *does* shows up in the contacts the hash already folds.
    pub jammer: Option<Jammer>,
    /// **The pod's switch** (16b cut O.2, 16c N12): `true` while the pod
    /// radiates. `true` at check-out for every unit — the owner's default, and
    /// what every match before cut O.2 did — and read only where `jammer` is
    /// `Some`: [`crate::arena::Arena::jamming_at`] skips a pod whose switch is
    /// off beside the wreck and power tests, so a silent pod neither blinds the
    /// sights inside its bubble nor cuts its own side's mesh.
    ///
    /// **One writer**, [`crate::arena::intents::resolve_emcon`], on a pilot's
    /// `EMCON on|off`. A machine never switches its pod, so a machine that
    /// adopts a released seat keeps the switch its pilot left — off, if they
    /// typed `EMCON off` — for the rest of the match (a recorded residual of
    /// cut O; the handover in `apply_control` does not touch it). Arena state and not a
    /// capability on the M32 record, on `power`'s precedent: it opens `true` at
    /// every [`Arena::new`] and is never written back at check-in, so no
    /// content byte moves. Unlike `jammer` beside it, it **is** folded into
    /// [`crate::arena::hash::state_hash`] (item 1.19): it decides what every
    /// sight in the bubble sees, and it is authoritative state with one writer.
    pub ecm_on: bool,
    /// **The counter-jamming filter it carries** (16b cut X.5; doc 16 §9 item 5), or
    /// `None` for a unit that mounts none — every stock unit, so a stock pass runs
    /// the arithmetic cut X.4 left it.
    ///
    /// Read off the cells at check-out for the reason [`ThermalSignature`] is, and
    /// left out of [`crate::arena::hash::state_hash`] for the reason `jammer` is: it
    /// is a fact about the unit's fit that nothing in a match moves, and what a
    /// filter *does* shows up in the contacts the hash already folds.
    ///
    /// It matters only to **this** unit's own pass: a filter is carried by the
    /// observer and recovers nothing for anybody standing beside it.
    pub eccm: Option<Eccm>,
    /// **What this platform's last sensor pass ran under** (16b cut D.4, `B-22`;
    /// 16c §5 Q17), or `None` for a receiver that is clean — or off. See
    /// [`crate::arena::perception::Receiver`] for the three `None`s and the two declared limits.
    ///
    /// Written by [`Arena::sensor_pass`] alone, at the pass's own arena
    /// timestamp, from the fold `Arena::detections` already made. It is read by
    /// the frame and by the notice and by nothing else: no rule of the arena
    /// decides anything by it.
    ///
    /// Unlike `jammer` and `eccm` beside it, it **is** folded into
    /// [`crate::arena::hash::state_hash`]: those are facts about the unit's fit
    /// that never move, and this is state that persists between passes, decides a
    /// notice, and can move with no ledger line of its own (a pass that stops
    /// running writes `None` silently).
    pub receiver: Option<crate::arena::perception::Receiver>,
    /// What it shoots with — the definition rows, in component order.
    ///
    /// **16b cut W.2 made it mutable in one way and one only**: doc 16 §3.3's
    /// `remove_weapons(section)` consequence empties the rows mounted on a section
    /// that has been shot off, which is what makes a turreted tank stop being able
    /// to fire. Nothing else edits it, and no row's own numbers ever change.
    pub weapons: Vec<WeaponSummary>,
    /// **What it presents and what a hit may strike** (16b cut W.2; doc 16 §3.3).
    pub profile: UnitProfile,
    /// **The gunnery rating resolving its shots** (16b cut W.2), 1 to 5.
    pub gunnery: u8,
    /// **What it does with heat** (16b cut X.1; doc 16 §9 item 1), or `None` for
    /// a unit whose content declares none — every stock unit, so the per-substep
    /// dissipation costs the harness one `Option` test per unit.
    pub heat: Option<HeatState>,
    /// **Who flies it** (16b cut N.1; doc 16 §6). Set at check-out and moved by
    /// exactly one thing afterwards: a [`crate::arena::intents::Control`] that
    /// releases or restores the seat (doc 16 §3.8 "Disconnects").
    pub controller: Controller,
    /// **The seal this hull was checked out under** (17b cut M3.2; doc 17
    /// §13, D-UM-45, D-UM-46): its sortie, the seal's digest, the core class
    /// the seal recorded (what 17b cut M3.4's cause-`core` drop keys on), the
    /// class its mind thinks at, the engine that flies it, the governor bit,
    /// whether the sortie is closed, (17b cut M4.6) whether the mind sealed
    /// is Standard Doctrine ([`crate::arena::mind::SealRef::doctrine`],
    /// derived at the seal, unfolded) and (17b cut M5.2) the profiles it was
    /// sealed with ([`crate::arena::mind::SealRef::profiles`], likewise).
    /// Laid down empty
    /// ([`crate::arena::mind::SealRef::none`]) in 17b cut M3.2; **since 17b cut
    /// M3.3** [`Arena::new`] writes it from the custody's own seal
    /// ([`crate::arena::mind::MindSeal::seal_ref`]), and a snapshot carries it
    /// whole — a resume never re-derives it from the world's record.
    ///
    /// Carried by this struct's derive (`SNAPSHOT_FORMAT` 8; 9 since 17b cut
    /// M4.6, with `doctrine`; 10 since 17b cut M5.2, with `profiles`) and
    /// **NOT
    /// folded into [`crate::arena::hash::state_hash`]**: a per-match
    /// constant, [`Arena::odds_floor_permille`]'s argument — and its one
    /// moving bit, `closed`, moves only with a ledger line (17b cut M3.6's
    /// `sortie_closed`), which the hash already folds through the ledger's
    /// sequence.
    pub seal: crate::arena::mind::SealRef,
    /// **Which of its sealed profiles the hull's mind will fly** (17b cut
    /// M5.2; doc 17 §11, §12, D-UM-44; [`UnitState::profile`] is the hull's
    /// silhouette, so this is `card_profile`): an index into
    /// [`crate::arena::mind::SealRef::profiles`] and the arena millisecond
    /// of the last switch — [`crate::arena::card::ActiveProfile::FIRST`] at
    /// check-out ([`Arena::new`]). Carried by this struct's derive
    /// (`SNAPSHOT_FORMAT` 10), decided carried on purpose: both fields decide
    /// a later card and a refusal. **Folded into
    /// [`crate::arena::hash::state_hash`] as item 1's sub-item 20 only when
    /// it is not the first profile never switched**, so a match that
    /// switches nothing folds the bytes it always did.
    pub card_profile: crate::arena::card::ActiveProfile,
    /// M32 unit truth, carried opaque — and, from 16b cut W.2, written by exactly
    /// one thing: one [`mush_tactics::UnitMutationPlan`] per impact through
    /// `mush_tactics::apply` (doc 16 §3.5 "Damage").
    ///
    /// **16b cut Q.2 made "exactly one thing" true of the assignment too.** There
    /// were two production writers — the impact in `crate::arena::damage` and the
    /// heat plan in `crate::arena::heat` — and a cache beside a field with two
    /// writers is a cache that rots. `Arena::set_unit` is the only assignment of
    /// this field in non-test code, and it is what keeps
    /// [`UnitState::condition`] true of it.
    pub unit: TacticalUnit,
    /// **What a gunner would read off this tank**, cached (16b cut Q.2; doc 16
    /// §3.6), recomputed by `Arena::set_unit` on every write of
    /// [`UnitState::unit`] and by [`Arena::new`] at check-out.
    ///
    /// A cache and not authoritative state: it is a pure function of `unit` and of
    /// the two `[tacsim.sensors]` floors, neither of which moves inside a match
    /// except through the one writer. It is therefore **not** folded into
    /// [`crate::arena::hash::state_hash`] on its own — `unit` is what M32
    /// journals, and hashing a function of it would fold the same truth twice —
    /// while the *readings* copied off it onto contacts are folded, because those
    /// are knowledge and they go stale.
    ///
    /// It exists because a sensor pass reads it once per close classified
    /// detection, and walking five sections and their facets per detection per
    /// pass is exactly the per-pulse cost doc 16 §2 W4 bounds.
    pub condition: crate::arena::condition::Condition,
    /// **Per-weapon ready time in arena milliseconds** (16b cut W.1 fills A.1's
    /// map): the arena millisecond at or after which that weapon may fire again,
    /// set to `now + cycle_ms` by an accepted shot. One row per weapon, laid down
    /// at check-out at zero — a complete table, so a lookup is never the difference
    /// between "ready" and "no such weapon".
    pub ready_at_ms: BTreeMap<SmolStr, i64>,
    /// **Rounds left, per weapon** (16b cut W.1), laid down at check-out from each
    /// weapon's [`WeaponSummary::count_per_load`] and decremented by the burst an
    /// accepted shot fires. One row per weapon, as `ready_at_ms` is.
    pub ammo: BTreeMap<SmolStr, u32>,
    /// The platform's lock (16b cut W.1 gives `LOCK` its effect): the contact
    /// `FIRE` shoots at, which is always one this platform holds — a contact the
    /// platform loses clears it (`crate::arena::step`'s `lost` arm).
    pub lock: Option<ContactId>,
    /// Set by motion when this unit was stopped at a cell boundary during the
    /// substep just integrated, and cleared at the start of the next.
    ///
    /// **A per-substep flag and nothing more** since 16b cut M.3 (16c `T-04`):
    /// it journals nothing. The ledger's `blocked` line used to fire on its
    /// rising edge, which made the line count a function of how a pulse was cut
    /// (a tail substep that covered no whole centimetre lowered it, and the next
    /// re-collision with the same bank was a second line); the line, the pilot's
    /// notice and the reflex controller's lesson now all fire on
    /// [`UnitState::held`]'s edge. Tests and the harness still read the flag.
    pub blocked: bool,
    /// **What it is standing against** (16b cut F.2; given its cell and cause by
    /// 16b cut M.3, 16c `T-04`, `B-27`, §5's Q22): `None` when nothing, else the
    /// refused cell and why ([`crate::arena::motion::Held`]). It is the condition
    /// `STATUS` prints `HELD` for, and its **new edge** — a stop naming a cell or
    /// a cause it was not already set to — is the one condition the ledger's
    /// `blocked` line, the pilot's notice and the reflex controller's remembered
    /// stop fire on.
    ///
    /// Not [`UnitState::blocked`], which is what the *substep just integrated*
    /// did: a tank ordered forward against a river bank covers no whole
    /// centimetre on some substeps, never reaches the boundary to be refused at,
    /// and so read `0 kph` with no `HELD` — the defect the 2026-09-18 smoke found.
    /// **One writer**, [`crate::arena::motion::integrate`], whose head states
    /// the rule: set by a refusal; cleared by entering another cell, by the speed
    /// order coming off, or by an accepted move inside the cell that points
    /// strictly away from the held one; left alone otherwise.
    ///
    /// **It is folded into [`crate::arena::hash::state_hash`]** (16b cut M.3).
    /// Until then it was output only and unhashed, and the flag beside it decided
    /// the ledger line; now it decides a line, a notice and a machine's avoid
    /// list, so a replay that set it a substep earlier must not compare equal.
    ///
    /// **16b cut E.3**: a *hold* against another unit
    /// ([`UnitState::occupied_hold`]) is neither a stop nor a substep that moved,
    /// so it leaves this exactly as it was — a tank already held against a wreck
    /// stays held through the two seconds before the next stop matures, and the
    /// pilot therefore still hears one line and not one per cycle.
    pub held: Option<crate::arena::motion::Held>,
    /// The seat has gone linkdead (doc 07 §1.2): the tank keeps its continuous
    /// orders and accepts no discrete action.
    pub linkdead: bool,
    /// **It has no weapons and cannot move** (16b cut W.2; doc 16 §3.8's *a wreck
    /// or disabled with no weapons*, doc 10 3A's *disabled precedes destroyed*).
    ///
    /// Set once, on the transition, by the consequence that finished it off, and
    /// journalled as `unit_disabled` then. It is **not** folded into the state
    /// hash: every transition into it is a ledger event, so the ledger's own
    /// sequence already moves with it (see [`crate::arena::hash::state_hash`]'s
    /// note on what is deliberately absent).
    pub disabled: bool,
    /// **How many shots it has fired** (16b cut W.2), for the result record's
    /// per-unit row (doc 16 §4).
    pub shots_fired: u32,
    /// **How many of them landed** (16b cut W.2), likewise.
    pub hits: u32,
    /// Arena millisecond of the latest accepted hit on this hull, including a
    /// spent section. A miss or refused mutation never changes this own fact.
    pub own_last_hit_ms: Option<i64>,
    /// **The arena second the seat-order count below belongs to** (16b cut M.1,
    /// `T-82`): `now_ms.div_euclid(1_000)` at the last discrete order a seat gave
    /// this unit, `0` at check-out.
    ///
    /// Written only by [`crate::arena::intents::apply_intent`], for an order from
    /// [`crate::arena::intents::IntentSource::Seat`]. It **is** folded into
    /// [`crate::arena::hash::state_hash`] beside the count: persistent state that
    /// decides a refusal notice is hashed.
    pub orders_window_s: i64,
    /// **How many discrete orders seats have given this unit in that arena
    /// second** (16b cut M.1, `T-82`), against `intents.max_pending_per_unit`:
    /// the discrete orders one seat may give a unit per arena second. An order
    /// that finds it at the bound is refused `queue_full`; a new second resets
    /// it. The machine's own orders are never counted. Hashed, with the window.
    pub orders_in_window: u32,
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

/// **The arena's own contact identity** (doc 16 §3.6, D-16-10).
///
/// A random 64-bit value minted through [`crate::arena::rng::draw_contact_id`]
/// with the `contact` label — never sequential, never reused inside a match, and
/// rendered as sixteen lowercase base-16 digits. It is deliberately a **distinct
/// type** from `mush_core::ContactId`: the durable `tactics/contacts` family is
/// the world's to write and the arena cannot reach it, so the two identity
/// spaces must not be assignable to each other. Contacts die with the match; the
/// ledger's `detected`, `classified` and `lost` events are the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContactId(pub u64);

impl std::fmt::Display for ContactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// **The letter a provisional designator opens with** (16b cut Q.1).
///
/// Pinned distinct from every [`Team::letter`] by a unit test below, so `X2` can
/// never be read as a slot on some later side.
pub const PROVISIONAL_PREFIX: char = 'X';

/// **The short name a pilot reads and types a contact by** (16b cut Q.1; doc 16
/// §3.6 "What a pilot sees").
///
/// The owner's first hand-played match found the defect this type answers:
/// `LOCK 29b81a…` is sixteen base-16 digits to type in the heat of a fight, and
/// nothing shorter existed. So a contact is shown — and resolved — as one of two
/// things:
///
/// * [`Designator::Provisional`] — `X1`, `X2`, …, numbered **per viewer** in the
///   order that platform's own sensors first found things. It is viewer-scoped
///   and says nothing about the target at all, which is exactly what a platform
///   holding an unclassified return knows.
/// * [`Designator::Slot`] — the team's letter and the unit's centre slot in
///   bijective base 26 (`RA`, `BC`, `BAA` past twenty-six a side). It is the
///   **same for every viewer**, which is the point: once a tank has been told
///   apart, two pilots on one side are talking about `BA` and mean one machine.
///
/// **Which one a contact renders as is read off the live contact and never off a
/// flag.** A contact is not durable — [`ShadowContacts::expire`] removes the row
/// and the next detection re-inserts it `Unknown` under the same id — so a sticky
/// "has been classified" bit would name a fresh blip by team and slot. See
/// [`DesignatorBook`] and `Arena::designator_for`.
///
/// **Its serde derive is the snapshot's** (16b cut AF.1): a unit's callsign, a
/// held loss's name and a release row's contact list are carried as
/// `{"provisional": 3}` or `{"slot": {"team": "red", "slot": 1}}`. Every wire
/// form a pilot, a frame or a ledger reads still carries the string
/// [`std::fmt::Display`] produces, and never this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Designator {
    /// A viewer-scoped number, from `1` (`X1`).
    Provisional(u16),
    /// A team letter and a 1-based centre slot (`RA`).
    Slot {
        /// The side.
        team: Team,
        /// The slot, from `1`.
        slot: u16,
    },
}

impl std::fmt::Display for Designator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Designator::Provisional(n) => write!(f, "{PROVISIONAL_PREFIX}{n}"),
            Designator::Slot { team, slot } => {
                write!(f, "{}{}", team.letter(), bijective_base26(*slot))
            }
        }
    }
}

/// **A 1-based slot in bijective base 26** (16b cut Q.1): `1` is `A`, `26` is
/// `Z`, `27` is `AA`.
///
/// Bijective and not ordinary base 26, because there is no zero slot: the sequence
/// has to run `Z`, `AA` rather than `Z`, `BA`, and every slot has exactly one
/// spelling. A slot of `0` — which [`Arena::new`] never produces, since it resolves
/// every unit onto a slot from `1` — renders as the empty string rather than
/// panicking, so a malformed designator is a name nothing resolves and not a
/// crash.
fn bijective_base26(slot: u16) -> String {
    let mut left = u32::from(slot);
    let mut letters: Vec<char> = Vec::new();
    while left > 0 {
        // `(left - 1) % 26` is under 26, so the narrowing is exact and the
        // fallback is unreachable.
        let index = u8::try_from((left.saturating_sub(1)) % 26).unwrap_or(0);
        letters.push(char::from(b'A'.saturating_add(index)));
        left = left.saturating_sub(1) / 26;
    }
    letters.reverse();
    letters.into_iter().collect()
}

/// **One unit's pose, as a check-out refuses it** (16b cut AF.1, extracted from
/// [`Arena::new`] unchanged): a heading that is not a bearing is
/// [`ArenaError::BadHeading`], and a pose off the window is
/// [`ArenaError::PoseOffWindow`], in that order. Answers the cell the pose
/// stands in.
fn check_pose(terrain: &ArenaTerrain, unit: ObjId, pose: &Pose) -> Result<(u16, u16), ArenaError> {
    if pose.heading_mdeg >= 360_000 {
        return Err(ArenaError::BadHeading {
            unit,
            heading_mdeg: pose.heading_mdeg,
        });
    }
    terrain.cell_of(pose).ok_or(ArenaError::PoseOffWindow {
        unit,
        x_cm: pose.x_cm,
        y_cm: pose.y_cm,
    })
}

/// **Doc 16 §3.4 step 5 at a check-out** (16b cut A.2's check, moved here
/// unchanged by 16b cut AF.1): no two units stand in one cell, refused as
/// [`ArenaError::CellOccupied`] naming the higher id as the unit that could not
/// be placed and the lower as the one already there. A unit off the window
/// stands in no cell and is [`check_pose`]'s to refuse.
fn check_occupancy(terrain: &ArenaTerrain, units: &[UnitState]) -> Result<(), ArenaError> {
    let mut cells: Vec<((u16, u16), ObjId)> = units
        .iter()
        .filter_map(|state| terrain.cell_of(&state.pose).map(|cell| (cell, state.id)))
        .collect();
    cells.sort_unstable();
    if let Some(((x, y), unit, other)) = cells
        .windows(2)
        .find(|pair| pair.first().map(|(at, _)| *at) == pair.get(1).map(|(at, _)| *at))
        .and_then(|pair| {
            let (cell, lower) = *pair.first()?;
            let (_, higher) = *pair.get(1)?;
            Some((cell, higher, lower))
        })
    {
        return Err(ArenaError::CellOccupied { unit, other, x, y });
    }
    Ok(())
}

/// **The pose refusals of [`Arena::new`] that a decoded snapshot is held to**
/// (16b cut AF.1): [`check_pose`] for every unit in unit-id order, then
/// [`check_occupancy`] — `BadHeading`, `PoseOffWindow` and `CellOccupied`, and
/// deliberately **not** `CellImpassable`, which `crate::arena::snapshot`'s
/// decode says why of.
///
/// # Errors
///
/// The first of the three refusals a unit meets.
pub(crate) fn check_poses(terrain: &ArenaTerrain, units: &[UnitState]) -> Result<(), ArenaError> {
    for unit in units {
        check_pose(terrain, unit.id, &unit.pose)?;
    }
    check_occupancy(terrain, units)
}

/// **Give every unit its public callsign** (16b cut Q.1), over the sorted unit
/// vector [`Arena::new`] has just built.
///
/// Two passes per side, and the order is the whole of why a replay agrees:
///
/// 1. **Explicit slots first.** A unit the lobby fielded from pod `n` takes slot
///    `n`, whatever else is on the side. Two units of one side in one slot is
///    [`ArenaError::DuplicateSlot`] — they would answer to one name.
/// 2. **Then the rest, in unit-id order**, each taking the lowest slot of its
///    side that pass 1 did not claim and pass 2 has not. That is what gives a
///    staff `@arena/start`, an AI-filled pod with no centre row and the harness's
///    field of three hundred stable callsigns with no lobby at all.
///
/// The vector arrives sorted by id, so pass 2's "in unit-id order" needs no sort
/// of its own.
///
/// # Errors
///
/// [`ArenaError::DuplicateSlot`], naming the side and the slot.
fn resolve_callsigns(states: &mut [UnitState]) -> Result<(), ArenaError> {
    for team in Team::ALL {
        let mut taken: Vec<u16> = Vec::new();
        for state in states.iter().filter(|state| state.team == team) {
            let Designator::Slot { slot, .. } = state.callsign else {
                continue;
            };
            if slot == 0 {
                continue;
            }
            if taken.contains(&slot) {
                return Err(ArenaError::DuplicateSlot {
                    team: team.as_str(),
                    slot: u8::try_from(slot).unwrap_or(u8::MAX),
                });
            }
            taken.push(slot);
        }
        let mut next: u16 = 1;
        for state in states.iter_mut().filter(|state| state.team == team) {
            if state.callsign != (Designator::Slot { team, slot: 0 }) {
                continue;
            }
            while taken.contains(&next) {
                next = next.saturating_add(1);
            }
            state.callsign = Designator::Slot { team, slot: next };
            taken.push(next);
            next = next.saturating_add(1);
        }
    }
    Ok(())
}

/// How much a viewer knows about what it is holding (doc 16 §3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Held, not yet told apart from anything else.
    Unknown,
    /// Told apart: the viewer knows whose it is.
    Classified {
        /// The side the viewer has resolved.
        team: Team,
    },
}

impl Classification {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Classification::Unknown => "unknown",
            Classification::Classified { .. } => "classified",
        }
    }

    /// The side this viewer has resolved, if it has.
    #[must_use]
    pub const fn team(&self) -> Option<Team> {
        match self {
            Classification::Unknown => None,
            Classification::Classified { team } => Some(*team),
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in: `0` unknown, `1`
    /// classified red, `2` classified blue.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Classification::Unknown => 0,
            Classification::Classified { team: Team::Red } => 1,
            Classification::Classified { team: Team::Blue } => 2,
        }
    }
}

/// **Where a target was, and how it was moving, at the sighting that a contact
/// records** (16b cut F.3; doc 16 §3.6 read as the owner's principle that
/// knowledge is earned and goes stale).
///
/// Five integers and not a [`Pose`]: a pose carries the driver's opaque posture
/// blob, and what a plot needs is the three coordinates the one range function
/// takes, the heading the facet band reads, and the speed the error cone's
/// crossing term reads. All five are things a platform learns by *looking*, which
/// is why the plot carries them rather than any reader asking the target. Copying
/// five integers per refresh is the whole cost of the contact store knowing
/// *where it saw something* rather than where that something now is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeenPose {
    /// East, region centimetres.
    pub x_cm: i64,
    /// North, region centimetres.
    pub y_cm: i64,
    /// Height against the region datum, centimetres.
    pub z_cm: i32,
    /// Which way the target was facing, millidegrees clockwise from north.
    pub heading_mdeg: u32,
    /// How fast it was making along that heading, centimetres a second, negative
    /// in reverse — the input to the advertised error cone's crossing term
    /// (`crate::arena::Arena::resolve_seen`).
    pub speed_cms: i32,
}

impl SeenPose {
    /// The plot a unit makes, at this instant.
    #[must_use]
    pub const fn of(unit: &UnitState) -> SeenPose {
        SeenPose {
            x_cm: unit.pose.x_cm,
            y_cm: unit.pose.y_cm,
            z_cm: unit.pose.z_cm,
            heading_mdeg: unit.pose.heading_mdeg,
            speed_cms: unit.speed_cms,
        }
    }

    /// The range from `pose` to this plot, centimetres — the same arithmetic
    /// [`crate::arena::index::range_cm`] does over two poses, so a plot's range
    /// and a true range are one function.
    #[must_use]
    pub fn range_from(&self, pose: &Pose) -> u64 {
        crate::arena::index::range_between(
            pose.x_cm, pose.y_cm, pose.z_cm, self.x_cm, self.y_cm, self.z_cm,
        )
    }

    /// **Whether this plot is within `reach_cm` of `pose`** — exactly
    /// `self.range_from(pose) <= reach_cm`, without the root (16b cut U.1; 16c
    /// `T-24`): [`crate::arena::index::within_between`] over the same six
    /// coordinates, whose identity (`floor(sqrt(s)) ≤ r` iff `s < (r+1)²`) is why
    /// it is a cost fix and not a behaviour change.
    ///
    /// The strict form `range_from(pose) < r` is `r >= 1 && within_cm(pose, r −
    /// 1)`, which is how `Arena::contact_within_alert_range` reads it; the two
    /// per-pulse tempo signals are its callers, one test per held contact per
    /// pulse, where the root was the whole cost.
    #[must_use]
    pub fn within_cm(&self, pose: &Pose, reach_cm: u64) -> bool {
        crate::arena::index::within_between(
            pose.x_cm, pose.y_cm, pose.z_cm, self.x_cm, self.y_cm, self.z_cm, reach_cm,
        )
    }

    /// The bearing from `pose` to this plot, millidegrees clockwise from north.
    #[must_use]
    pub fn bearing_from(&self, pose: &Pose) -> u32 {
        crate::arena::perception::bearing_of(
            self.x_cm.saturating_sub(pose.x_cm),
            self.y_cm.saturating_sub(pose.y_cm),
        )
    }

    /// The window cell this plot is in, or `None` for a plot off the window.
    #[must_use]
    pub fn cell(&self, terrain: &ArenaTerrain) -> Option<(u16, u16)> {
        terrain.cell_at(self.x_cm, self.y_cm)
    }
}

/// One contact a viewer platform holds (doc 16 §3.6's contact lifecycle as
/// vocabulary, in RAM).
///
/// **16b cut F.3 gave it its provenance and its plot.** Cut S.1 stored when the
/// contact was last refreshed and nothing about *who* refreshed it or *where the
/// target was*, and the two omissions cost the same thing: a contact that never
/// expired (two meshed teammates refreshing each other's copy for ever) and a
/// contact reported at the target's live pose (a tank tracked through a wall for
/// the whole time-to-live). Both are answered here, in the store, rather than at
/// every reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowContact {
    /// What is being held.
    pub target: ObjId,
    /// How well.
    pub class: Classification,
    /// Arena milliseconds at first detection.
    pub first_seen_ms: i64,
    /// Arena milliseconds at the last refresh — the input to expiry. Either this
    /// platform's own sensors or a teammate's share set it, and a share never
    /// raises it above the sharer's own sighting (16b cut F.3).
    pub last_seen_ms: i64,
    /// **Where the target was at the sighting that set [`ShadowContact::last_seen_ms`]**
    /// (16b cut F.3) — this platform's own, or the sharer's own as the share
    /// carried it.
    ///
    /// Every reader of a contact's position reads this and never the target's
    /// live pose: it is a plot, not a tether. While a target is in continuous view
    /// the plot is at most one `sensors.pass_ms` old.
    pub seen_at: SeenPose,
    /// **This platform's OWN last sighting** (16b cut F.3): when its own sensors
    /// saw the target and where the target was — or `None` for a copy that only
    /// ever arrived over the mesh.
    ///
    /// It is what a share is allowed to be built from (doc 16 §2 W6: shared
    /// contacts fan out only within comms range *of the platform that sees*), so a
    /// mesh copy is never relayed onward and two teammates cannot keep one
    /// another's copy alive after the last real sighting.
    ///
    /// **The stamp and the plot are one field** rather than two beside
    /// [`ShadowContact::seen_at`], because a share hands over both and they have
    /// to be the same sighting. Held apart, a platform whose own sighting is older
    /// than a teammate's share would relay its own *time* against the teammate's
    /// *place* — a second hop for the position, which is exactly what rule 1
    /// exists to stop.
    pub own_seen: Option<OwnSighting>,
    /// **The per-section bands this platform holds on the target**, with the
    /// arena timestamp of the look that took them (16b cut Q.2; doc 16 §3.6) —
    /// this platform's own, or a teammate's as a share carried it.
    ///
    /// `None` until the platform has had a **close look** at a contact that is
    /// **classified** (`crate::arena::perception::Detection`'s `close`), which is
    /// what makes a reading earned rather than a free consequence of detection.
    /// It is what `SCAN` prints, and it dies with the contact row: the store's
    /// `expire` removes the row, so a re-acquired blip is `Unknown` again and
    /// reads nothing until it has been told apart and looked at a second time.
    ///
    /// It is **not** [`OwnSighting::condition`]: this one may be a teammate's,
    /// and only this platform's own may be put back on the mesh.
    pub seen_condition: Option<crate::arena::condition::Reading>,
    /// **This hold was opened under a provisional number** (16b cut D.4, `B-21`).
    ///
    /// `true` at exactly one site — the `None` arm of [`Arena::sensor_pass`], with
    /// `[tacsim.sensors].designate_on_detection` off, where
    /// `Designators::assign` issues or re-uses the number — and never written
    /// again for the life of the row. `false` for a mesh arrival
    /// ([`Arena::share_pass`]'s insert) and in [`ShadowContact::sighted`]'s own
    /// default.
    ///
    /// It exists because the designator **book** is permanent and a hold is not.
    /// `Classified`'s `was` used to read the book, so a target this platform
    /// numbered in an earlier hold, lost, and then received over the mesh printed
    /// `New contact BA: …` and at once `Contact X2 identified: BA, blue.` — a
    /// rename of a row the pilot is not looking at. Read off the hold, a mesh
    /// arrival says nothing after its acquisition and a row this platform's own
    /// sensors opened still says the rename.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], because it moves
    /// with no distinguishable ledger event: a mesh `detected` and a sensor
    /// `detected` are byte for byte the same line, which is the argument cut Q.1
    /// made for the book itself.
    pub numbered: bool,
    /// **This platform watched the target die** (16b cut E.2, `B-45`; 16c §5's
    /// Q11).
    ///
    /// `false` at every insert site, including
    /// [`ShadowContact::sighted`]'s own default. **One writer**: the
    /// `EventKind::UnitDestroyed { unit, by: Some(shooter) }` arm of
    /// `crate::arena::step`'s notice scan, beside the `Kill` line — the same
    /// fact the shooter's pilot is sent, written at the same place and off the
    /// same `by` cut D.4 (`T-08`) reads there.
    ///
    /// **A share never carries it.** [`Arena::share_pass`] builds a mesh copy
    /// from [`ShadowContact::own_seen`], and a kill this platform watched is not
    /// a *sighting*: it is one fact a gunner read through his own sight, exactly
    /// as the destroyed band `Arena::observe_section_lost` writes is. A mesh
    /// arrival therefore opens `false` and a teammate keeps its track until the
    /// track expires, which is doc 16 §1's observer rule applied to the
    /// machines.
    ///
    /// It dies with the row: `ShadowContacts::expire` removes the hold, so a
    /// re-acquired blip on the same object knows nothing again.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], beside
    /// `numbered` and for the same reason: it moves with no ledger line of its
    /// own that names the **holder** — a `destroyed` line says who died and who
    /// killed it and nothing about whose contact learned it — so a replay that
    /// credited the kill to a different shooter would otherwise compare equal.
    pub known_dead: bool,
}

/// **One platform's own sighting of a target** (16b cut F.3): the moment, and
/// where the target was at it. See [`ShadowContact::own_seen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnSighting {
    /// Arena milliseconds at the sighting.
    pub at_ms: i64,
    /// Where the target stood, as this platform's own sensors took it.
    pub plot: SeenPose,
    /// **The bands this platform's OWN sensors last read off the target** (16b
    /// cut Q.2), with the timestamp of *that look* and not of this sighting.
    ///
    /// The two stamps differ on purpose. A sighting is every pass the platform
    /// holds the target; a **look** is only the passes it held the target
    /// classified and inside the close-look reach. So this field **carries
    /// forward** across refreshes that did not earn a new reading, keeping its own
    /// older stamp, and a share hands the receiver a reading that is honestly as
    /// old as the look behind it.
    ///
    /// It is what may travel on the mesh, which is why it rides here and not
    /// beside [`ShadowContact::seen_condition`]: a copy of somebody else's reading
    /// must never be relayed a second hop (doc 16 §2 W6).
    pub condition: Option<crate::arena::condition::Reading>,
}

impl ShadowContact {
    /// **A contact this platform's own sensors have just taken** at `t_ms`, with
    /// the target where it stood at that sighting (16b cut F.3).
    ///
    /// **16b cut Q.2**: and with no reading of any kind. A contact this
    /// constructor makes is `Unknown` at every production call site, and an
    /// unidentified return has no section vocabulary to read bands into; the
    /// sensor pass writes one later, on a pass that finds the contact classified
    /// and close.
    #[must_use]
    pub const fn sighted(
        target: ObjId,
        class: Classification,
        t_ms: i64,
        seen_at: SeenPose,
    ) -> ShadowContact {
        ShadowContact {
            target,
            class,
            first_seen_ms: t_ms,
            last_seen_ms: t_ms,
            seen_at,
            own_seen: Some(OwnSighting {
                at_ms: t_ms,
                plot: seen_at,
                condition: None,
            }),
            seen_condition: None,
            // **16b cut D.4**: the constructor says nothing about the book, and
            // the one site that issues a number sets it. A fixture that builds a
            // contact by hand is therefore a hold with no number behind it, which
            // is the honest default.
            numbered: false,
            // **16b cut E.2** (`B-45`): a fresh sighting has watched nothing die.
            // The one writer is the kill line's own arm.
            known_dead: false,
        }
    }

    /// **What this platform may put on the mesh**, or `None` when it has nothing
    /// to say about this contact (16b cut F.3).
    ///
    /// Both of the cut's share rules, in the one place they are written, so
    /// [`Arena::sensor_pass`]'s *is there anything to say* and
    /// [`Arena::share_pass`]'s *what is there to say* cannot answer differently:
    /// the contact is **classified** (an `unknown` smudge means nothing to
    /// anybody else), and this platform's **own** sensors saw it inside the time
    /// to live.
    #[must_use]
    pub fn shareable(&self, cutoff_ms: i64) -> Option<(Team, OwnSighting)> {
        let team = self.class.team()?;
        let own = self.own_seen.filter(|own| own.at_ms >= cutoff_ms)?;
        Some((team, own))
    }
}

/// **The one byte-identical refusal for a contact id a platform does not hold**
/// (doc 16 §3.6: *unresolvable, unowned or expired* are one answer, and it is the
/// same answer a nonexistent unit gets).
///
/// It lives here, beside [`ShadowContacts::resolve`] — the only lookup a pilot's
/// id ever goes through — so that the rule and its sentence cannot be separated.
/// `crate::match_::cockpit` re-exports it under K.2's name, so the line a pilot
/// reads exists exactly once in the tree.
pub const NO_SUCH_CONTACT: &str = "No such contact.";

/// **How often one spent section may be said**, arena milliseconds (16b cut D.2,
/// `B-15`).
///
/// The window [`Arena::spent_said`] is read against, for the victim and for the
/// gunner who holds the target classified. It is arena time and not wall time and
/// not a per-pulse count, which is what makes the decision the same at every
/// cadence.
pub const SPENT_NOTICE_MS: i64 = 5_000;

/// **The per-viewer shadow contact store** (doc 16 §3.6): viewer platform to its
/// contacts, contact id to what it holds.
///
/// Cut A defined it and cut S.1's sensor pass fills it. It is a `BTreeMap` of
/// `BTreeMap` rather than a hash map so that iteration order is the state hash's
/// order and a replay cannot diverge on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowContacts {
    by_viewer: BTreeMap<ObjId, BTreeMap<ContactId, ShadowContact>>,
}

impl ShadowContacts {
    /// An empty store.
    #[must_use]
    pub fn new() -> ShadowContacts {
        ShadowContacts::default()
    }

    /// Record or refresh a contact for `viewer`.
    pub fn insert(&mut self, viewer: ObjId, id: ContactId, contact: ShadowContact) {
        self.by_viewer
            .entry(viewer)
            .or_default()
            .insert(id, contact);
    }

    /// What `viewer` holds under `id`, if anything.
    ///
    /// An unresolvable, unowned or expired id is indistinguishable from a
    /// nonexistent one — doc 16 §3.6's byte-identical refusal is exactly this
    /// `None`.
    #[must_use]
    pub fn get(&self, viewer: ObjId, id: ContactId) -> Option<&ShadowContact> {
        self.by_viewer.get(&viewer)?.get(&id)
    }

    /// The same, for refreshing in place.
    pub fn get_mut(&mut self, viewer: ObjId, id: ContactId) -> Option<&mut ShadowContact> {
        self.by_viewer.get_mut(&viewer)?.get_mut(&id)
    }

    /// **The one lookup a pilot's named id goes through** (16b cut S.1; doc 16
    /// §3.6's existence-hiding rule).
    ///
    /// Byte for byte the same as [`ShadowContacts::get`] and deliberately a
    /// different name: every surface that resolves an id a *pilot typed* calls
    /// this, and the rule that goes with it is that its `None` becomes
    /// [`NO_SUCH_CONTACT`] and never a sentence about which of the three reasons
    /// applied. An id that was never minted, one minted for another platform and
    /// one this platform has let expire are one answer, so an unauthorised viewer
    /// cannot learn that a unit exists by naming it.
    #[must_use]
    pub fn resolve(&self, viewer: ObjId, id: ContactId) -> Option<&ShadowContact> {
        self.get(viewer, id)
    }

    /// The id `viewer` already holds `target` under, if any — so a refresh does
    /// not mint a second id for one target.
    #[must_use]
    pub fn id_of(&self, viewer: ObjId, target: ObjId) -> Option<ContactId> {
        self.by_viewer
            .get(&viewer)?
            .iter()
            .find(|(_, contact)| contact.target == target)
            .map(|(id, _)| *id)
    }

    /// Everything `viewer` holds, in contact-id order.
    pub fn of(&self, viewer: ObjId) -> impl Iterator<Item = (&ContactId, &ShadowContact)> {
        self.by_viewer.get(&viewer).into_iter().flatten()
    }

    /// **Every contact every viewer holds**, viewer object number ascending then
    /// contact id ascending — the two `BTreeMap`s' own order, which is why
    /// [`crate::arena::hash::state_hash`] can fold the store without a sort.
    pub fn all(&self) -> impl Iterator<Item = (ObjId, ContactId, &ShadowContact)> {
        self.by_viewer.iter().flat_map(|(viewer, held)| {
            held.iter()
                .map(move |(id, contact)| (*viewer, *id, contact))
        })
    }

    /// Drop one contact, answering what was dropped.
    pub fn remove(&mut self, viewer: ObjId, id: ContactId) -> Option<ShadowContact> {
        let held = self.by_viewer.get_mut(&viewer)?.remove(&id)?;
        if self.by_viewer.get(&viewer).is_some_and(BTreeMap::is_empty) {
            self.by_viewer.remove(&viewer);
        }
        Some(held)
    }

    /// Drop everything `viewer` has not refreshed since `before_ms`, answering
    /// the losses in contact-id order so the caller can journal one `lost` event
    /// each.
    pub fn expire(&mut self, viewer: ObjId, before_ms: i64) -> Vec<(ContactId, ShadowContact)> {
        let mut lost: Vec<(ContactId, ShadowContact)> = Vec::new();
        if let Some(held) = self.by_viewer.get_mut(&viewer) {
            let stale: Vec<ContactId> = held
                .iter()
                .filter(|(_, contact)| contact.last_seen_ms < before_ms)
                .map(|(id, _)| *id)
                .collect();
            for id in stale {
                if let Some(contact) = held.remove(&id) {
                    lost.push((id, contact));
                }
            }
            if held.is_empty() {
                self.by_viewer.remove(&viewer);
            }
        }
        lost
    }

    /// Forget every contact a viewer held (its platform is a wreck, or has left).
    pub fn clear_viewer(&mut self, viewer: ObjId) {
        self.by_viewer.remove(&viewer);
    }

    /// How many viewers hold anything.
    #[must_use]
    pub fn viewers(&self) -> usize {
        self.by_viewer.len()
    }

    /// How many contacts are held in total.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_viewer.values().map(BTreeMap::len).sum()
    }

    /// Whether anything at all is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// **The provisional numbers one match has issued** (16b cut Q.1), per viewer.
///
/// It holds numbers and **nothing else** — no class, no "has been classified"
/// flag, no team. That is the whole of its design, and it is the correction the
/// plan's first draft needed: a contact is not a durable object
/// ([`ShadowContacts::expire`] removes the row and the next detection re-inserts
/// it [`Classification::Unknown`] under the same id), so any bit that outlived the
/// row would name a freshly re-acquired blip by team and slot on every surface
/// that reads it. What a viewer *now* knows is read off the live contact
/// (`Arena::designator_for`); this only answers *which number this platform's own
/// sensors gave that target the first time they found it*.
///
/// **One writer**: `Arena::sensor_pass`'s `None` arm — the insert of an `Unknown`
/// contact by the platform's own sensors — and only while
/// `[tacsim.sensors].designate_on_detection` is off. A mesh arrival writes
/// nothing, so a number always means *something my own sensors found*, and the
/// sequence stays contiguous: a mesh-first target that later expires and is found
/// by this platform's own sight takes the next number then.
///
/// A number is **never reused** and a re-detected target **keeps** its number —
/// the same continuity the sixteen-digit id always had, since
/// `crate::arena::rng::draw_contact_id` mints one id per `(viewer, target)` pair
/// for the match.
///
/// **It is folded into [`crate::arena::hash::state_hash`]** (that function's own
/// note says why): it moves without a distinguishable ledger event, because a
/// mesh `detected` and a sensor `detected` are the same ledger line. It is not
/// journalled: the ledger names units by object id, which is the canon's
/// vocabulary, and a replay in a pilot's own vocabulary is not this cut's.
///
/// **16b cut M.3** (16c `T-27`) gave it a running [`DesignatorBook::digest`],
/// which is what the hash folds now in place of walking both maps every
/// substep.
///
/// **16b cut AF.1**: carried whole by the snapshot — `of` as pairs, its key
/// being a tuple no JSON object can name (`crate::arena::snapshot::pairs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignatorBook {
    /// The next number each viewer will issue, from `1`.
    next: BTreeMap<ObjId, u16>,
    /// `(viewer, target)` to the number that viewer gave that target.
    #[serde(with = "crate::arena::snapshot::pairs")]
    of: BTreeMap<(ObjId, ObjId), u16>,
    /// **FNV-1a 64 over every mint, in the order it was made** (16b cut M.3):
    /// seeded at [`crate::arena::hash::FNV_OFFSET_BASIS`] and folded by
    /// [`DesignatorBook::assign`] with the viewer's `u32`, the target's `u32` and
    /// the number's `u16`, little-endian, only when it mints.
    digest: u64,
}

/// Hand-written rather than derived (16b cut M.3): an empty book's digest is
/// FNV-1a's offset basis, not zero, so that a book is the fold of its mints from
/// the same start [`crate::arena::hash::state_hash`] uses.
impl Default for DesignatorBook {
    fn default() -> DesignatorBook {
        DesignatorBook {
            next: BTreeMap::new(),
            of: BTreeMap::new(),
            digest: crate::arena::hash::FNV_OFFSET_BASIS,
        }
    }
}

impl DesignatorBook {
    /// The number `viewer` holds `target` under, minting the viewer's next one if
    /// it holds none.
    ///
    /// Saturating at [`u16::MAX`], which a match cannot reach: a viewer would have
    /// to detect 65,535 distinct targets, and the arena's own unit cap is far
    /// under that.
    ///
    /// **16b cut M.3**: a mint folds into [`DesignatorBook::digest`]; a pair the
    /// book already holds returns before it — `crate::arena::perception` asks
    /// again on every re-opened hold — and folds nothing, so the digest moves once
    /// per number issued and never per question.
    pub fn assign(&mut self, viewer: ObjId, target: ObjId) -> u16 {
        if let Some(held) = self.of.get(&(viewer, target)) {
            return *held;
        }
        let next = self.next.entry(viewer).or_insert(1);
        let issued = *next;
        *next = next.saturating_add(1);
        self.of.insert((viewer, target), issued);
        let digest = crate::arena::hash::fnv1a64_update(self.digest, &viewer.0.to_le_bytes());
        let digest = crate::arena::hash::fnv1a64_update(digest, &target.0.to_le_bytes());
        self.digest = crate::arena::hash::fnv1a64_update(digest, &issued.to_le_bytes());
        issued
    }

    /// **The running digest of every number this book has minted, in order**
    /// (16b cut M.3, 16c `T-27`) — what [`crate::arena::hash::state_hash`] folds
    /// for the book.
    #[must_use]
    pub const fn digest(&self) -> u64 {
        self.digest
    }

    /// The number `viewer` holds `target` under, if it holds one.
    #[must_use]
    pub fn provisional(&self, viewer: ObjId, target: ObjId) -> Option<u16> {
        self.of.get(&(viewer, target)).copied()
    }

    /// The `next` table, viewer ascending. [`crate::arena::hash::state_hash`]
    /// walked it until 16b cut M.3 folded [`DesignatorBook::digest`] instead; it
    /// stays for readers.
    pub fn counters(&self) -> impl Iterator<Item = (ObjId, u16)> {
        self.next.iter().map(|(viewer, next)| (*viewer, *next))
    }

    /// How many viewers have issued a number, read off the map rather than by
    /// counting the iterator.
    #[must_use]
    pub fn counters_len(&self) -> usize {
        self.next.len()
    }

    /// How many numbers have been issued in all — folded by
    /// [`crate::arena::hash::state_hash`] beside the digest, read off the map
    /// because the hash runs once a substep.
    #[must_use]
    pub fn issued_len(&self) -> usize {
        self.of.len()
    }

    /// Every issued number as `(viewer, target, n)`, viewer then target
    /// ascending — the `BTreeMap`'s own order. The hash walked it until 16b cut
    /// M.3; it stays for readers.
    pub fn issued(&self) -> impl Iterator<Item = (ObjId, ObjId, u16)> {
        self.of
            .iter()
            .map(|((viewer, target), n)| (*viewer, *target, *n))
    }
}

// ---------------------------------------------------------------------------
// Channels and the epoch table
// ---------------------------------------------------------------------------

/// **The cockpit's channel words, in [`Channel::ALL`]'s order** (17b cut M1a.5;
/// doc 17 §4, D-UM-47): the one table where the arena's closed [`Channel`] and
/// `mush-sim`'s open `ChannelId` meet — *by wire word, written down in M1a*.
///
/// The cockpit station's grant (`crate::match_::cockpit::cockpit_channels`) is
/// built from it, and a test holds it to [`Channel::as_str`] and
/// [`Channel::parse`] both ways, so a fourth channel added to one side and not
/// the other fails the build's tests rather than a seat at run time.
pub const COCKPIT_CHANNEL_IDS: [&str; 3] = ["helm", "sensors", "fire"];

/// A control channel over a tank (doc 16 §3.3 "Stations are on the tank",
/// doc 09 C8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Heading, speed and startup.
    Helm,
    /// Contacts and locks.
    Sensors,
    /// Weapons.
    Fire,
}

impl Channel {
    /// Every channel, in code order — the set a cockpit station grants.
    pub const ALL: [Channel; 3] = [Channel::Helm, Channel::Sensors, Channel::Fire];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Channel::Helm => "helm",
            Channel::Sensors => "sensors",
            Channel::Fire => "fire",
        }
    }

    /// **The `mush-sim` channel id this channel is granted as** (17b cut
    /// M1a.5; D-UM-47): its wire word, the entry of [`COCKPIT_CHANNEL_IDS`] at
    /// its own place in [`Channel::ALL`].
    #[must_use]
    pub const fn channel_id(&self) -> &'static str {
        self.as_str()
    }

    /// The channel a word names, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Channel> {
        match word {
            "helm" => Some(Channel::Helm),
            "sensors" => Some(Channel::Sensors),
            "fire" => Some(Channel::Fire),
            _ => None,
        }
    }
}

/// **The arena's authority epoch table** (doc 16 §3.1, doc 09 C8): the epoch
/// each `(unit, channel)` is currently at.
///
/// An [`Intent`] is validated at the world when it is enqueued and **authorised
/// at apply** here, against this table. The world raises the table through a
/// [`crate::arena::intents::Control`] on every seat activation, deactivation,
/// lease revocation, linkdead onset and revival, `EXIT` and `SIM ABORT`; the
/// arena then purges every queued intent below the new epoch, which is why a
/// pilot who types `FIRE` then `EXIT` in one burst never fires.
///
/// The table **never lowers**. A `Control` that arrives out of order, or twice,
/// cannot reopen an epoch a later one closed.
///
/// **16b cut AF.1**: carried by the snapshot as pairs, its key being a tuple no
/// JSON object can name (`crate::arena::snapshot::pairs`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochTable {
    #[serde(with = "crate::arena::snapshot::pairs")]
    epochs: BTreeMap<(ObjId, Channel), u64>,
}

impl EpochTable {
    /// An empty table: every `(unit, channel)` is at epoch `0`.
    #[must_use]
    pub fn new() -> EpochTable {
        EpochTable::default()
    }

    /// Raise `(unit, channel)` to `epoch`, answering whether it moved.
    ///
    /// Never lowers: a lower or equal `epoch` is a no-op answering `false`.
    pub fn raise(&mut self, unit: ObjId, channel: Channel, epoch: u64) -> bool {
        let slot = self.epochs.entry((unit, channel)).or_insert(0);
        if epoch > *slot {
            *slot = epoch;
            true
        } else {
            false
        }
    }

    /// The current epoch of `(unit, channel)`; `0` when the table has never held
    /// it.
    #[must_use]
    pub fn current(&self, unit: ObjId, channel: Channel) -> u64 {
        self.epochs.get(&(unit, channel)).copied().unwrap_or(0)
    }

    /// Whether `epoch` is the **exact** current epoch. A stale epoch is refused
    /// and so is one from the future: only the epoch the world last published is
    /// authority.
    #[must_use]
    pub fn authorises(&self, unit: ObjId, channel: Channel, epoch: u64) -> bool {
        self.current(unit, channel) == epoch
    }

    /// How many `(unit, channel)` pairs the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.epochs.len()
    }

    /// **Every row, in `(unit, channel)` order** (16b cut AF.3): what a resumed
    /// match's world mirror is seeded from, read off the decoded arena before it
    /// moves into its task, so the world stamps the epochs the arena authorises.
    pub fn rows(&self) -> impl Iterator<Item = ((ObjId, Channel), u64)> + '_ {
        self.epochs.iter().map(|(key, epoch)| (*key, *epoch))
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.epochs.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Due events
// ---------------------------------------------------------------------------

/// What a scheduled event does when its arena timestamp comes round (doc 16
/// §3.1 "Rate independence").
///
/// Cut A schedules [`DueKind::StartupComplete`] and [`DueKind::PositionSummary`]
/// and nothing else; the remaining kinds exist so that the heap, the ordering
/// and the state hash are built once and the later cuts add bodies rather than
/// shapes. **16b cut M.2** added the last two: the duration bound and the end
/// of a held loss's hysteresis, the two things that still happened at a pulse
/// boundary — doc 16 §3.1's rate independence says everything in the arena
/// happens at an arena timestamp.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DueKind {
    /// Expiration of one exact sampled program lease; stale events are inert.
    RuleLeaseExpired {
        /// The hull whose sampled gates expire.
        unit: ObjId,
        /// Runtime generation captured by the lease.
        generation: u64,
        /// The sample's successful landing timestamp.
        lands_ms: i64,
    },
    /// `match.startup_ms` after `STARTUP` was accepted: power goes online.
    StartupComplete {
        /// The unit starting up.
        unit: ObjId,
    },
    /// **A weapon's cycle has finished** (16b cut W.1). It changes no state —
    /// readiness is a comparison against `now_ms` — and exists so that
    /// `next_due_ms` wakes the pulse when the breech comes free.
    WeaponReady {
        /// The unit.
        unit: ObjId,
        /// The weapon's component id.
        weapon: SmolStr,
    },
    /// **A shot's flight time has elapsed** (16b cut W.1): the arena resolves it
    /// through `Arena::resolve_impact`.
    Impact {
        /// The shot's sequence number, which is also its draw label's argument.
        shot_seq: u64,
    },
    /// One unit's sensor pass (cut S).
    SensorPass {
        /// The observing unit.
        unit: ObjId,
        /// The pass's sequence number, which is also its draw label's argument.
        pass_seq: u64,
    },
    /// One unit's contact share (cut S).
    Share {
        /// The sharing unit.
        unit: ObjId,
    },
    /// Every `ledger.summary_ms` of arena time, in every tempo (doc 16 §4).
    PositionSummary,
    /// **The duration bound** (16b cut M.2, 16c `T-01`): doc 16 §3.8's `Timeout`,
    /// scheduled once, by [`Arena::new`], at `match.max_duration_ms`.
    ///
    /// It is a due event for the reason every other end is settled inside the
    /// substep: phase 5 compared `now_ms` against the bound at the pulse's end,
    /// so the `match_ended { timeout }` line carried the millisecond the cadence
    /// happened to end a pulse on. `Arena::substep`'s arm stops the due loop
    /// after the deadline's own millisecond, so nothing due after it is applied
    /// or journalled; the substep then settles the timeout unless an elimination
    /// or a `Spent` in the same substep won.
    MatchDeadline,
    /// **A Hold match's clock has run out for one sole occupant** (16b cut O.1;
    /// 16c N11), scheduled by [`crate::arena::Arena::settle_hold_presence`] at
    /// `since_ms + hold_ms` when `team` becomes the square's only side, exactly
    /// as [`DueKind::MatchDeadline`] is scheduled at the bound.
    ///
    /// `Arena::substep`'s arm awards the win only when the clock it was
    /// scheduled for is still the one running (`held_by == Some((team,
    /// since_ms))`) AND a fresh presence reading at the pop still finds `team`
    /// alone; otherwise it fires into nothing, as a stale
    /// [`DueKind::LossSettles`] does. It is never cancelled with
    /// [`DueHeap::retain`]: see `settle_hold_presence`.
    HoldSettles {
        /// The side that was alone on the spot.
        team: Team,
        /// The substep boundary it was first seen alone at.
        since_ms: i64,
    },
    /// **A held loss's hysteresis has run out** (16b cut M.2, 16c `T-83`): doc 16
    /// §3.6 "What a pilot sees", one event per held loss, scheduled by the `lost`
    /// arm of `Arena::stage_pilot_notice` at the very deadline it writes into
    /// [`Arena::held_losses`].
    ///
    /// Its arm says the loss when the entry is still there with that deadline,
    /// and is a no-op otherwise: a loss cancelled by a re-detection, or one lost
    /// again with a later deadline, leaves a stale event that fires into nothing,
    /// and the due count folds it deterministically.
    ///
    /// The one held loss that gets **no** event is one whose window closes at or
    /// before the end of the substep that staged it — only a
    /// `[tacsim.sensors].pass_ms` shorter than the substep makes one — because
    /// an event for it would be scheduled into the past. The staging scan that
    /// staged it settles it itself, at its place in `(t_ms, seq)` order.
    LossSettles {
        /// The platform whose pilots are owed the line.
        viewer: ObjId,
        /// The contact it lost.
        contact: ContactId,
    },
    /// **The motor's cadence** (17b cut M1a.4; doc 17 §4 D-UM-3) — a pilot's
    /// `DRIVE` since that cut, a mind's `goto` since 17b cut M1a.5 — due every
    /// `arena.reflex.decide_ms` of arena time on the stagger grid
    /// `Arena::grid_at` answers (the reflex controller's tick ran on it until
    /// 17b cut M4.6 deleted that), while the hull's motor row holds a goal.
    ///
    /// It is a due event and not a phase-4 pass for the reason every other
    /// cadence in this engine is one: a due event is applied at **its own arena
    /// timestamp**, inside the substep whose boundary that timestamp fell in, so
    /// the same route is followed at the same arena milliseconds at 5,000,
    /// 1,000 and 500 ms. A drive run once per *pulse* would run ten times as
    /// often at the fast cadence as at the slow one, which is doc 16 §3.1's
    /// D-16-2 exactly backwards — the tempo would be deciding outcomes.
    /// Scheduled once per engagement by `Arena::engage_motor` (a goal landing
    /// finds the heap holding one or not), applied by `Arena::motor_step`,
    /// rescheduled from its own `t_ms` while a goal stands, and taken off the
    /// heap with the rest of a wreck's work.
    MotorTick {
        /// The hull being driven.
        unit: ObjId,
    },
    /// **One unit mind's think** (17b cut M1a.5; doc 17 §6, D-UM-20), due
    /// every `crate::arena::mind::STOCK_THINK_MS` of arena time on the
    /// stagger grid [`DueKind::MotorTick`] runs on, from the lattice
    /// millisecond `Arena::grid_at` answers.
    ///
    /// **Its place in this enum orders nothing**: [`DueEvent`]'s derived order
    /// is `(t_ms, seq, kind)` and `seq` is unique, so `kind` never breaks a
    /// tie. D-UM-20's *after that millisecond's other due items* and D-UM-21's
    /// *in unit-id order* are delivered by `Arena::substep`'s due loop, which
    /// holds a millisecond's mind kinds back until every other kind due at it
    /// has run and then runs them landings first, by unit, by `think_seq`.
    MindThink {
        /// The hull thinking.
        unit: ObjId,
        /// The think's sequence number for that hull, from `0` — what a stale
        /// event is told apart by (`crate::arena::mind::MindState::think_seq`).
        think_seq: u64,
    },
    /// **One think's landing** (17b cut M1a.5; doc 17 §6, D-UM-21), due one
    /// substep after the think it lands (`Arena::substep_len_ms`), at which
    /// the think's intents are applied at the epochs it recorded. Ordered as
    /// [`DueKind::MindThink`] is, and for the same reason not by its place
    /// here.
    MindLand {
        /// The hull whose think lands.
        unit: ObjId,
        /// The think it lands.
        think_seq: u64,
    },
}

impl DueKind {
    /// **Whether this is a mind's kind** (17b cut M1a.5) — the two the due
    /// loop holds back to the end of their millisecond (D-UM-20).
    #[must_use]
    pub const fn is_mind(&self) -> bool {
        matches!(self, DueKind::MindThink { .. } | DueKind::MindLand { .. })
    }
}

/// One scheduled event: its arena timestamp, its insertion sequence, and what it
/// does.
///
/// The derived order is `(t_ms, seq, kind)`, and `seq` is unique per heap, so it
/// is exactly doc 16 §3.1 phase 3(c)'s *timestamp order then event-sequence
/// order* — total, and independent of how many pulses have run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DueEvent {
    /// When, in arena milliseconds.
    pub t_ms: i64,
    /// The monotonic insertion sequence, unique within the heap.
    pub seq: u64,
    /// What.
    pub kind: DueKind,
}

/// **The due-event heap**: a min-heap by `(t_ms, seq)` (doc 16 §4 "Arena state").
///
/// Implemented over a `BTreeSet` rather than a `BinaryHeap` so that
/// [`DueHeap::peek_due`] can hand out a reference to the minimum and
/// [`DueHeap::iter`] can walk the schedule in order — both of which cut A.2's
/// `step` and the frame builder want, and neither of which a binary heap gives.
///
/// **16b cut AF.1**: carried whole — the events in `(t_ms, seq)` order and
/// `next_seq` — so a resumed match numbers its next event where this one would
/// have.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DueHeap {
    events: BTreeSet<DueEvent>,
    next_seq: u64,
}

impl DueHeap {
    /// An empty heap. The first scheduled event gets sequence `1`, so `0` is
    /// available as "no event".
    #[must_use]
    pub fn new() -> DueHeap {
        DueHeap::default()
    }

    /// Schedule `kind` at `t_ms`, answering its sequence number.
    pub fn schedule(&mut self, t_ms: i64, kind: DueKind) -> u64 {
        let seq = self.next_seq.saturating_add(1);
        self.next_seq = seq;
        self.events.insert(DueEvent { t_ms, seq, kind });
        seq
    }

    /// The earliest event, when it is due at or before `until_ms`.
    #[must_use]
    pub fn peek_due(&self, until_ms: i64) -> Option<&DueEvent> {
        self.events.first().filter(|event| event.t_ms <= until_ms)
    }

    /// Take the earliest event, when it is due at or before `until_ms`.
    pub fn pop_due(&mut self, until_ms: i64) -> Option<DueEvent> {
        if self.peek_due(until_ms).is_some() {
            self.events.pop_first()
        } else {
            None
        }
    }

    /// When the next event is due — half of `next_due_ms` (doc 16 §3.1 phase 5).
    #[must_use]
    pub fn next_t_ms(&self) -> Option<i64> {
        self.events.first().map(|event| event.t_ms)
    }

    /// The whole schedule, in `(t_ms, seq)` order.
    pub fn iter(&self) -> impl Iterator<Item = &DueEvent> {
        self.events.iter()
    }

    /// Drop every event whose kind `predicate` accepts, answering how many went
    /// — how a unit's scheduled work is cancelled when it becomes a wreck.
    pub fn retain(&mut self, mut predicate: impl FnMut(&DueKind) -> bool) -> usize {
        let before = self.events.len();
        let kept: BTreeSet<DueEvent> = std::mem::take(&mut self.events)
            .into_iter()
            .filter(|event| predicate(&event.kind))
            .collect();
        self.events = kept;
        before.saturating_sub(self.events.len())
    }

    /// How many events are scheduled — folded into the state hash.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing is scheduled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Stopping and ending
// ---------------------------------------------------------------------------

/// Why the world asked the arena to stop (doc 16 §3.1's `Stop` message).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// `SIM ABORT`, or an empty side.
    Abort,
    /// `match.max_duration_ms`.
    Timeout,
    /// The match is void: nothing it derived may be committed.
    Void,
    /// **A side gave up** (16b cut D.3, `B-33`): `SIM CONCEDE`, by that side's
    /// lead.
    ///
    /// It carries the side that **conceded**, not the side that won, because that
    /// is the fact the world has: the arena turns it into
    /// [`EndReason::Conceded`], whose `winner` is the other one.
    Concede {
        /// The side that gave up.
        by: Team,
    },
    /// **Every pilot has left the match** (16b cut D.3, `B-33`).
    ///
    /// `[tacsim.match].unattended_ms` of world time after the last un-`left` slot
    /// went `left` during `Running`, the lobby's 1 s tick sends this. Doc 16 §11's
    /// N.1 amendment makes a released seat one-way, so a match every human has
    /// walked out of otherwise runs to `match.max_duration_ms` — a quarter-hour
    /// of machines driving at each other in a centre nobody can claim a slot in.
    Unattended,
}

/// How a match ended (doc 16 §4 "The result record").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// A side was eliminated; `None` is a draw (both sides gone at once).
    Elimination {
        /// The surviving side, or `None` for a draw.
        winner: Option<Team>,
    },
    /// **Nobody left on either side can fire a round** (16b cut Q.7), and the
    /// last one in flight has landed: a **draw**, naming no winner.
    ///
    /// It is deliberately not a redefinition of doc 16 §3.8's elimination, which
    /// stays *a wreck, or disabled with no weapons*: counting an empty magazine
    /// as out of action would make a solo player at full armour lose the instant
    /// their last round left the breech. This is the symmetric reading instead —
    /// a side that is dry while the other is not plays on, and only when neither
    /// can shoot is there nothing left to decide. Before it, such a match ran to
    /// `match.max_duration_ms`, which was an hour, and is fifteen minutes since
    /// AP.2, of two tanks driving at each other.
    Spent,
    /// **A side kept the key spot uncontested for the declared time** (16b cut
    /// O.1; 16c N11): a Hold match's own end, settled at its own arena
    /// millisecond by [`DueKind::HoldSettles`] and **before** elimination and
    /// `Spent`, which are settled second (the owner's default: *elimination
    /// still ends it, settled second*).
    ///
    /// An end with a winner, like [`EndReason::Conceded`]: the result's outcome
    /// is the winner's own word and the reason word is `hold` — doc 16 §4's six
    /// outcome words stay six.
    Hold {
        /// The side that held.
        winner: Team,
    },
    /// **A side conceded** (16b cut D.3, `B-33`): `SIM CONCEDE`, by that side's
    /// lead, and the other side wins.
    ///
    /// It is an end and not an abort: the match was fought, so its cells are
    /// committed and the tanks stand as evidence exactly as after an elimination
    /// ([`crate::match_::MatchState::Ended`], and the winner's own outcome word).
    Conceded {
        /// The side that did **not** concede.
        winner: Team,
    },
    /// **Every pilot had left it** (16b cut D.3, `B-33`): `[tacsim.match]`'s
    /// `unattended_ms` elapsed with every slot `left`.
    ///
    /// A `timeout` in the result record's vocabulary (doc 16 §4 has six words and
    /// this is not a seventh), and not a void: the match really ran and what its
    /// machines did to each other is evidence like any other.
    Unattended,
    /// `match.max_duration_ms`, or `ledger.max_bytes` reached (doc 16 §4).
    Timeout,
    /// `SIM ABORT`.
    Abort,
    /// A panic or a watchdog: no arena-derived cell may be committed.
    ///
    /// **16b cut AA.3 (16c `B-79`)**: a stall on the pulse that ended the match
    /// is voided by the arena task without a stopping pulse, so the ledger keeps
    /// the arena's own end wherever it journalled one (`match_ended`, or `aborted`
    /// for an abort; a ledger-full timeout journals none) and no `void` event,
    /// while the result says void.
    Void,
    /// The bounded egress backed up past `ledger.max_unflushed_chunks`.
    LedgerOverflow,
}

/// The end of a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchEnd {
    /// Why.
    pub reason: EndReason,
    /// When, in arena milliseconds.
    pub t_ms: i64,
}

/// What can refuse an [`Arena`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArenaError {
    /// Recovery state cannot be admitted under the captured byte ceiling.
    #[error("match recovery capacity refused: {reason}")]
    SnapshotCapacity { reason: String },
    /// Custody's immutable mind payload cannot be admitted for this hull.
    #[error("unit {unit} mind cannot be admitted: {reason}")]
    Mind {
        /// The hull whose whole admission was refused.
        unit: ObjId,
        /// Bounded schema/admission reason.
        reason: String,
    },
    /// The terrain's dimensions and its cell vector disagree, or a dimension or
    /// the cell size is zero.
    #[error(
        "arena terrain is {width} by {height} cells of {cell_cm} cm with {cells} cells: a window \
         is a non-zero width and height of non-zero cells, and holds exactly width x height of them"
    )]
    TerrainShape {
        /// Columns.
        width: u16,
        /// Rows.
        height: u16,
        /// The declared cell size.
        cell_cm: u16,
        /// How many cells were supplied.
        cells: usize,
    },
    /// Two custody units carry one object id.
    #[error("unit {unit} was handed over twice: one object is one unit")]
    DuplicateUnit {
        /// The repeated id.
        unit: ObjId,
    },
    /// A unit's pose is not on the window.
    #[error(
        "unit {unit} was handed over at ({x_cm}, {y_cm}) cm, which is off the arena window: every \
         unit starts on a cell of the definition it is checked out into"
    )]
    PoseOffWindow {
        /// The unit.
        unit: ObjId,
        /// Its east coordinate.
        x_cm: i64,
        /// Its north coordinate.
        y_cm: i64,
    },
    /// A unit's heading is not a bearing.
    #[error(
        "unit {unit} was handed over facing {heading_mdeg} millidegrees: a heading is 0..360000"
    )]
    BadHeading {
        /// The unit.
        unit: ObjId,
        /// The offered heading.
        heading_mdeg: u32,
    },
    /// Two units were handed over standing in one cell.
    #[error(
        "units {unit} and {other} were both handed over in cell ({x}, {y}): occupancy is one unit \
         per cell, so a check-out places them apart"
    )]
    CellOccupied {
        /// The unit that could not be placed.
        unit: ObjId,
        /// The unit already in the cell.
        other: ObjId,
        /// The column.
        x: u16,
        /// The row.
        y: u16,
    },
    /// A pilot names a unit that was not handed over.
    #[error(
        "pilot {pilot} is seated at station {station:?} on unit {unit}, which was not handed over: a \
         station's subject is a unit in custody"
    )]
    UnknownPilotUnit {
        /// The unit named.
        unit: ObjId,
        /// The pilot.
        pilot: ObjId,
        /// The station.
        station: SmolStr,
    },
    /// No units were handed over.
    #[error("no units were handed over: an arena with no units has no match to run")]
    NoUnits,
    /// **16b cut Q.1** — two units of one side were fielded from one centre slot,
    /// so they would answer to one callsign.
    #[error(
        "two units on the {team} side were handed over in slot {slot}: a slot is one unit, and a \
         callsign is one unit's name"
    )]
    DuplicateSlot {
        /// The side.
        team: &'static str,
        /// The repeated slot.
        slot: u8,
    },
    /// **16b cut M.4** (16c `B-57`, `T-15`) — the fastest unit would cross more
    /// than one cell in the shortest substep the clock will cut.
    ///
    /// The substep is `min(substep_max_ms, (cell_cm − 1) × 1000 / ceiling)`
    /// floored at [`crate::arena::clock::MIN_SUBSTEP_MS`], and the floor wins
    /// above 19,980 cm/s on 1,000 cm cells: past it a substep's travel is more
    /// than a cell, which is the one bound step 4's single-crossing test rests on.
    /// Nothing refused that before M.4; `crate::arena::motion`'s one-cell guard
    /// stopped the unit instead, every substep, for the whole match. The ceiling
    /// is [`Arena::max_speed_cms`]'s, the true one.
    ///
    /// The sentence names no unit and no cell: `lobby::run` announces
    /// `Match {id} could not start: {error}.` to the whole centre, so it is read
    /// by bystanders.
    #[error(
        "the fastest unit would cross more than one cell in a {min_substep_ms} ms substep on \
         {cell_cm} cm cells; the match cannot be timed",
        min_substep_ms = crate::arena::clock::MIN_SUBSTEP_MS
    )]
    SubstepBound {
        /// The true speed ceiling, cm/s ([`Arena::max_speed_cms`]).
        ceiling_cms: u32,
        /// The window's cell size, centimetres.
        cell_cm: u16,
    },
    /// **16b cut M.4** (16c `T-14`) — a unit was handed over standing on a cell
    /// nothing may enter.
    ///
    /// Motion never puts a unit there (step 4 refuses the crossing), so a unit
    /// that opens on one could never drive out of it: the terrain factor caps its
    /// speed at zero. The world side's `opening_pose` never chooses such a cell;
    /// this is the arena refusing to trust that, as it refuses an off-window pose.
    ///
    /// The sentence names no object and no cell, for [`ArenaError::SubstepBound`]'s
    /// reason; the fields ride a `tracing::warn!` at the refusal site.
    #[error("a unit's opening cell is not passable")]
    CellImpassable {
        /// The unit.
        unit: ObjId,
        /// The column.
        x: u16,
        /// The row.
        y: u16,
    },
    /// **16b cut O.1** — the check-out asked for a mode its ground does not
    /// declare ([`ArenaDefinition::supports`]). `match_::start` refuses the same
    /// thing before it writes anything, so this is the arena refusing to trust
    /// that, as it refuses an off-window pose. The sentence names the ground's
    /// id and the mode's word, both public.
    #[error("arena {arena} declares no mode {mode}")]
    ModeUnsupported {
        /// The mode's wire word.
        mode: &'static str,
        /// The definition's id.
        arena: SmolStr,
    },
    /// **16b cut AE.3** (doc 16 §2 W2: *no drone swarms; class counts on the
    /// map obey the caps*) — one side was handed more units of a class than
    /// `[tacsim.arena].max_units_by_class` lets a side field.
    ///
    /// The lobby refuses the second drone at `SIM UNIT` already; this is the
    /// arena refusing to trust that, and the staff `@arena/start` path's one
    /// guard. The sentence names a side, a class word and two numbers, all
    /// public, and no object, for [`ArenaError::SubstepBound`]'s reason.
    #[error("the {team} side fields {fielded} of class {class}; a side fields at most {cap}")]
    ClassCap {
        /// The side's word.
        team: &'static str,
        /// The class word ([`UnitClass::as_str`]).
        class: &'static str,
        /// How many of the class the side was handed.
        fielded: u32,
        /// The configured cap.
        cap: u32,
    },
    /// **Doc 18 cut IC2** (doc 18 §4.2 schema addition (1), D-18-29) — a side
    /// was handed a unit whose class no arena fields (a `hauler`).
    ///
    /// The lobby never offers one (`fieldable_ids`); the custody seam is the last
    /// door (D-16 custody), so a staff `@arena/start` or a fixture that checks one
    /// out is refused here before anything is built. The sentence names a side
    /// and a class word, both public, and no object.
    #[error("the {team} side fields a {class}, which no arena fields")]
    UnfieldableClass {
        /// The side's word.
        team: &'static str,
        /// The class word ([`UnitClass::as_str`]).
        class: &'static str,
    },
}

// ---------------------------------------------------------------------------
// The arena
// ---------------------------------------------------------------------------

/// **Where the one hash a run of discrete actions shares stands** (16b cut
/// M.3, 16c `T-23`): [`Arena::action_hash`]'s three states.
///
/// A lazy slot, filled by the first [`Arena::event_hash`] that needs it and
/// never before, so a phase or a substep with no discrete action to journal
/// takes no hash at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ActionHash {
    /// Not inside phase 2's intent loop or phase 4 (or, until 17b cut M4.6, a
    /// substep's reflex ticks):
    /// every [`Arena::event_hash`] takes a fresh
    /// [`crate::arena::hash::state_hash`] and stores nothing. What a direct
    /// resolver call in a test sees.
    #[default]
    Outside,
    /// Inside, and nothing has journalled yet: the first
    /// [`Arena::event_hash`] takes the hash, stores it as
    /// [`ActionHash::Taken`], and answers it. Set by phase 2 before its intent
    /// loop (16b cut Y.1: the `ordered` lines of one pulse) and by phase 4
    /// before its loop (and by a substep's first reflex tick until 17b cut
    /// M4.6 deleted the reflex controller).
    Pending,
    /// Inside, and taken: every later line of the run carries this one hash.
    /// Phase 2's and phase 4's ends and the substep's end set
    /// [`ActionHash::Outside`] again.
    Taken(u64),
}

/// **One match** (doc 16 §4 "Arena state").
///
/// Every field is public because cut A.2's `motion`, `index` and `step` are
/// siblings in this module tree and the discipline that matters is the one the
/// contract test enforces — no I/O — not field privacy inside the library.
/// (One exception since 16b cut M.3: `action_hash`, crate-visible with its
/// type, because nothing outside the arena may set a hash the ledger records.)
///
/// The two configuration blocks are **cloned in**, so `step` needs no
/// configuration argument and cannot be handed a different tempo table halfway
/// through a match.
#[derive(Clone, Debug)]
pub struct Arena {
    /// Bounded diagnostics only; never hashed, snapshotted or used by execution.
    pub(crate) traces: super::author_trace::TraceStore,
    /// The window and its entry zones.
    ///
    /// **17b cut M2.2** (17b's settled question (ii)): behind an `Arc`, wrapped
    /// once by [`Arena::new`] and once by the snapshot's decode, so a think
    /// hands a scripted runner the map (`ThinkJob::map`) without copying the
    /// terrain and `Arena: Clone` clones the `Arc`, not the map. Every reader
    /// works through the deref; a test that rewrites the map writes through
    /// `Arc::make_mut`. The snapshot records the definition's digest, never the
    /// definition, so no format moves.
    pub definition: Arc<ArenaDefinition>,
    /// The units, **sorted by id** — the order every deterministic walk uses.
    pub units: Vec<UnitState>,
    /// The authority epochs intents are authorised against.
    pub epochs: EpochTable,
    /// The per-viewer shadow contacts (cut S fills it).
    pub contacts: ShadowContacts,
    /// **The provisional numbers this match has issued** (16b cut Q.1). See
    /// [`DesignatorBook`]: numbers and nothing else, written at one site, folded
    /// into the state hash.
    pub designators: DesignatorBook,
    /// **The designator a contact carried at the moment it expired** (16b cut
    /// Q.1), `(viewer, contact)` to what the row read as.
    ///
    /// A `lost` notice is staged *after* the row is gone, so the class that
    /// decides between `BA` and `X2` is no longer readable anywhere. It is
    /// captured in `Arena::sensor_pass` from the record
    /// [`ShadowContacts::expire`] hands back and taken out again by the
    /// `EventKind::Lost` arm of the staging scan — so it is filled and drained
    /// **inside one substep's event list** and never survives the substep it was
    /// filled in.
    ///
    /// It is deliberately **not** empty at every hash point: the substep takes its
    /// record before the staging scan runs, so a substep in which a contact
    /// expired is hashed with that row's name still in here. That costs nothing,
    /// because the table is not folded into [`crate::arena::hash::state_hash`] at
    /// all — its contents are a function of state the hash already folds (the
    /// expiry is the contact store's own change; the name is read off the book and
    /// the callsign), so folding it would add no information and would make the
    /// hash a function of where the drain sits.
    pub lost_designators: BTreeMap<(ObjId, ContactId), Designator>,
    /// **The shots of this substep that found a section already destroyed** (16b
    /// cut D.2, `B-15`), by shot sequence.
    ///
    /// Filled by `Arena::apply_hit` at the one branch
    /// where the plan has no ops — a round that arrived on a section already at
    /// zero armour and zero internals — and drained by the `hit` arm of the
    /// staging scan in the **same substep**, which is what tells the three
    /// readers' sentences apart.
    ///
    /// **It is deliberately not folded into
    /// [`crate::arena::hash::state_hash`]**, on `lost_designators`' own argument,
    /// which really does apply here: its contents are a function of state the
    /// hash already folds (the section's cells, and the `hit` line the ledger
    /// sequence counts), and it never survives the substep it was filled in. It
    /// is **not** empty at every hash point — the substep takes its record before
    /// the staging scan runs — and that costs nothing for the same reason.
    pub spent_hits: BTreeSet<u64>,
    /// **When each (platform told, target, section) last read a spent-section
    /// line** (16b cut D.2, `B-15`), in arena milliseconds.
    ///
    /// `B-15`: one duel put six `armour gone, 0 of 0 inside.` lines in front of
    /// one pilot in ten seconds. The round stays absorbed and journalled exactly
    /// as it was; what this bounds is the **telling**, to one line per section per
    /// [`SPENT_NOTICE_MS`] of arena time, for the two readers who can tell a spent
    /// section from a live one: the victim, whose own tank is outside the observer
    /// rule, and the gunner who **holds the target classified**.
    ///
    /// **The unidentified gunner has no entry here and is never suppressed**, and
    /// that is an observer-rule decision rather than an oversight: a line
    /// withheld after a round he watched land would tell him the round hit
    /// something already dead, and a per-section key would let him count the
    /// target's destroyed sections by counting lines per five seconds. Both are
    /// condition facts the close look is the price of — the argument
    /// [`crate::arena::step::Frame::scan_tables`] already makes for the same class
    /// of fact.
    ///
    /// **It IS folded into [`crate::arena::hash::state_hash`]**, and that is where
    /// it parts from `spent_hits` beside it: it persists across substeps and
    /// pulses and it decides what a later substep says, so `lost_designators`'
    /// argument is false of it. Two rules keep the fold rate-independent — the
    /// decision and the write happen whether or not anybody is seated and whether
    /// or not a frame is ever sent, because the scan runs per substep at the
    /// event's own timestamp; and entries are pruned **only on touch**, relative
    /// to the event being staged, never by a per-pulse sweep.
    pub spent_said: BTreeMap<(ObjId, ObjId, SmolStr), i64>,
    /// The scheduled events.
    pub due: DueHeap,
    /// The streaming match ledger.
    pub ledger: Ledger,
    /// **The uniform grid over unit positions** (doc 16 §3.2 "The spatial
    /// index"), rebuilt twice per substep by [`Arena::step`].
    ///
    /// It lives on the arena rather than inside the pulse so that its buffer —
    /// one slot per cell, 65,536 of them on the shipped window — is allocated
    /// once for the match instead of once per substep, and so that cut S's sensor
    /// pass reads the same index motion tested occupancy against.
    pub index: SpatialIndex,
    /// **Each unit's chassis-static words** ([`ChassisWords`]; 16c `T-105`),
    /// one row per unit in [`Arena::units`]' order, read by `frame_scope` for
    /// every contact row instead of recomputed there. Re-derived by
    /// [`Arena::new`] and by the snapshot decode from the units' own chassis
    /// and profiles; never carried, never hashed. Shared, so a clone of the
    /// arena (the twin, the tests) shares the table.
    pub chassis: Arc<[ChassisWords]>,
    /// The adaptive tempo and the lag clamp.
    pub clock: Clock,
    pub tape_cursor: Option<super::input_tape::TapeCursor>,
    pub tape_witnesses: super::input_tape::WitnessBuffer,
    /// The match seed: the key of every draw.
    pub seed: u64,
    /// **The resolution table** (16b cut W.2), cloned in at check-out for the same
    /// reason the configuration blocks are: `step` needs no content argument and
    /// cannot be handed a different curve halfway through a match.
    pub resolution: ResolutionDef,
    /// The seated pilots, as handed over.
    pub pilots: Vec<Pilot>,
    /// **The motor table** (17b cut M1a.1; doc 17 §4 "The motor", D-UM-3): the
    /// one table every controller's hull drives from — a
    /// [`crate::arena::motor::MotorRow`] per hull something is steering (its
    /// goal, its route, its searches and the cells it was stopped at), the path
    /// budget every row shares and the A\* scratch. A row exists for a unit
    /// something is steering; a seated pilot flying by `HEADING` and `SPEED` has
    /// none.
    ///
    /// Folded into the state hash as its own section, every row in unit-id
    /// order (`crate::arena::hash`, item 3b; until 17b cut M4.6 a reflex hull's
    /// row was folded inside its reflex row, item 3); carried whole by a
    /// snapshot but its scratch.
    pub motors: crate::arena::motor::MotorTable,
    /// **What each unit mind holds** (17b cut M1a.5; doc 17 §3, §6, §8,
    /// D-UM-8, D-UM-23): one [`crate::arena::mind::MindState`] per
    /// [`Controller::Mind`] hull — its think counter, the think in flight with
    /// its answer (17b cut M3.2), its memory's bytes, its notice buffer, its
    /// range memory, its standing order, its strikes, its drop and its seal's
    /// digest (17b cut M3.2) — in unit-id order.
    ///
    /// Folded into the state hash per row with no count prefix
    /// (`crate::arena::hash`, item 3c), so an arena with no mind — every match
    /// that exists — folds no byte for it; carried whole by a snapshot.
    pub minds: BTreeMap<ObjId, crate::arena::mind::MindState>,
    /// Record-only latest accepted attribution, outside combat state_hash.
    pub(crate) order_watches: super::order_watch::OrderWatches,
    /// Immutable complete card definitions, admitted once and shared by key.
    pub cards: crate::arena::runtime_card::CardCatalog,
    /// Per-hull active program, profile generation and sampled rule authority.
    pub profiles: BTreeMap<ObjId, crate::arena::profile_runtime::ProfileRuntime>,
    /// **Every hull's condition slots** (17b cut M1b.1; doc 17 §5, D-UM-16):
    /// the principal's shot decisions, armed on a hull of any controller
    /// (`crate::arena::conditions`' head says why here and not on a mind's row).
    /// A hull holds a key only while a slot is armed. Folded per row with no
    /// count prefix (`crate::arena::hash`, item 3d), so an arena with none —
    /// every production match — folds no byte; carried by a snapshot.
    pub conditions: crate::arena::conditions::ConditionTable,
    /// **When each slot last said why it held, per contact** (17b cut M1b.1;
    /// doc 17 §5: *a held shot said at most once per contact per 10 s*):
    /// `(unit, slot, contact)` to the arena millisecond it was said. 17b cut
    /// M1b.4 writes it; this cut carries it (through `pairs`), folds it (item
    /// 3e, per row with no count prefix) and drops a slot's rows when the slot
    /// is disarmed, re-armed or cleared.
    pub held_said: BTreeMap<(ObjId, u8, ContactId), i64>,
    /// **The match's Compact floor**, per mille (17b cut M1b.1; doc 17 §5,
    /// D-UM-17 as amended 2026-10-04): the least `min_permille` a lawful
    /// condition may carry, **captured at check-out** —
    /// `crate::arena_task::spawn_arena` writes `[tacsim.minds].odds_floor_permille`
    /// here before the arena moves into its task, and [`Arena::new`] defaults it
    /// to `MindsConfig::default()`'s 100 for every other caller. A per-match
    /// constant: **NEVER folded into `state_hash`** (a folded constant would move
    /// the production hash for a value no production match reads before 17b cut
    /// M4); **carried by the snapshot**, so a config reload between a check-out
    /// and a resume moves nothing — a resumed arena keeps its own.
    pub odds_floor_permille: u32,
    /// **The class table this match thinks under** (17b cut M3.1; doc 17 §9,
    /// D-UM-35): every figure of every class — the think period, the op
    /// ceiling, the view's rows and odds rows, the slots, the memory keys,
    /// `line_to` and the profiles — that [`Arena::mind_profile`] and
    /// [`Arena::slot_capacity`] read. [`Arena::new`] sets
    /// [`crate::arena::core::ClassTable::STOCK`]; `crate::arena_task::spawn_arena`
    /// writes `[tacsim.minds].classes` here right after the floor, so it is
    /// **captured at check-out** as the floor is.
    ///
    /// **Not folded into `state_hash`** (a per-match constant, the floor's
    /// argument) and, **since 17b cut M3.2, carried** by the snapshot
    /// (`SNAPSHOT_FORMAT` 8): a resumed match keeps the table it was checked
    /// out under, so a config reload between a check-out and a resume moves
    /// nothing — the gap 17b cut M3.1 declared (an operator's table re-derived
    /// stock at the decode) is closed.
    pub classes: crate::arena::core::ClassTable,
    /// **The hulls whose think awaits its write** (17b cut M4.1; doc 17 §6
    /// "Execution policy", D-UM-22, D-UM-23): named by `Arena::collect_think`
    /// when it writes the think's facts on the row with an empty answer, and
    /// taken off by `Arena::settle_collected` when the answer (or the fault)
    /// is written — or by whatever ends the row's wait first (a wreck, a
    /// possess, a release), which drops the answer. The "answered" fact
    /// beside the row, because an EMPTY memory is a legal clean answer from
    /// every test runner and `NoMind`, so [`crate::arena::mind::PendingThink`]
    /// cannot say it itself. Scratch: **never hashed** (the facts it stands
    /// beside are, item 3c), **never carried** and **re-derived empty** by the
    /// decode — `Snapshot::encode` REFUSES an arena that holds any, by name
    /// (the sim task awaits every outstanding batch before it encodes).
    pub awaiting: BTreeSet<ObjId>,
    /// **The thinks collected in the substep being run** (17b cut M4.1), in
    /// collection order, each with no substep hash yet: moved to
    /// [`Arena::unsettled`] with the substep's end hash stamped on them at
    /// the substep's record (`Arena::stamp_collected`). Scratch, empty
    /// between two pulses and never carried.
    pub(crate) collected: Vec<crate::arena::mind::CollectedThink>,
    /// **The thinks collected and not yet written** (17b cut M4.1), stamped,
    /// in collection order: what `crate::arena::step::StepOutput::thinks`
    /// clones out to the sim task for its pool, what
    /// `Arena::deliver_answers` writes from the pool's answers, and what the
    /// next substep's head (or the next `Arena::step`'s) settles inline when
    /// nothing delivered it. Scratch: never hashed, never carried, re-derived
    /// empty by the decode (an encode refuses while any think awaits).
    pub(crate) unsettled: Vec<crate::arena::mind::CollectedThink>,
    /// **The card changes waiting for the next pulse** (17b cut M1b.5; doc 17
    /// §11, §22's M1b): per hull, in arrival order, what staff's `@mind/live`
    /// offered — handed in by the task BEFORE the step
    /// ([`Arena::stage_card`]) and applied to the arena's own rows at phase
    /// 2's head (`Arena::apply_mind_cards`), which empties it.
    ///
    /// (The thinks waiting to land, `mind_landings`, were scratch beside it
    /// until 17b cut M3.2 moved each think's answer onto its row's
    /// [`crate::arena::mind::MindState::pending`], D-UM-23 as amended.)
    ///
    /// Scratch: not hashed (what it changes is,
    /// the row's card, item 3c) and **reset** by the snapshot — empty between
    /// two pulses, and a change the task held when a match resumed was never
    /// applied, which the read-back says (`not read since the resume`).
    pub pending_cards: BTreeMap<ObjId, Vec<crate::arena::card::CardChange>>,
    /// **What runs a think** (17b cut M1a.5; doc 17 §7, D-UM-1): a trait the
    /// sim task implements outside this library — the arena only calls it,
    /// synchronously, inside the think's due event. Set by
    /// `crate::arena::mind::default_mind_runner` in [`Arena::new`] and in the
    /// snapshot's decode alike; neither hashed nor carried.
    ///
    /// **Since 17b cut M3.5** it is the arena-wide FALLBACK: a hull with a
    /// runner of its own on [`Arena::runners`] thinks on that one
    /// ([`Arena::runner_for`]), and every other hull on this.
    pub mind_runner: std::sync::Arc<dyn crate::arena::mind::MindRunner>,
    /// **Each hull's own runner, by its seal** (17b cut M3.5; doc 17 §13,
    /// D-UM-1): what `crate::minds::install_sealed_runners` installed for a
    /// hull whose seal names a scripted mind — the script the seal's source
    /// digest names, compiled once per digest and shared. Empty for a hull
    /// sealed `native` or `reflex` (every production hull before 17b cut M4),
    /// which thinks on [`Arena::mind_runner`]. Compiled code like the
    /// fallback: neither hashed nor carried — **re-derived empty** by the
    /// snapshot's decode, and refilled by the installer from the decoded
    /// seals (`crate::arena_task::resume_arena`), so a resumed hull thinks on
    /// the runner its seal names, never on whatever the decode defaulted to.
    pub runners: BTreeMap<ObjId, std::sync::Arc<dyn crate::arena::mind::MindRunner>>,
    /// **The cheapest passable cell on the window for each row**, in
    /// [`CostRow::index`] order ([`ArenaTerrain::min_cost`]; each at least
    /// `1`), read once at check-out because the terrain is immutable for the
    /// life of a match.
    ///
    /// Two readers, one copy (16b cut M.4, 16c `T-15`): the motor's A\*
    /// heuristic scale (16b cut N.1, which kept it on the reflex controller's
    /// memory until M.4) and
    /// [`Arena::max_speed_cms`]'s true ceiling. **16b cut AE.1** made it one
    /// number per row (it was `min_move_cost`, the tracked row's): a search for
    /// a car scaled on the tracked 100 over an 80 road would be inadmissible,
    /// and a car's ceiling is its own row's. Not folded into the state hash: it
    /// is a function of the definition and never moves.
    pub min_cost: [u32; 3],
    /// Queued discrete actions per unit, with **no bound of their own** (16b
    /// cut M.1, `T-82`): a seat's `intents.max_pending_per_unit` orders a unit
    /// per arena second (counted in `apply_intent`) and the arena task's
    /// per-unit pre-pulse bound (that value plus one) limit what reaches them;
    /// a machine's own orders are not counted, and phase 4 empties the queue
    /// every pulse.
    pub pending: BTreeMap<ObjId, VecDeque<Intent>>,
    /// **The shots in flight**, by shot sequence (16b cut W.1; doc 16 §4 "Arena
    /// state": *scheduled impacts*).
    ///
    /// One row per accepted shot, from the moment `FIRE` is accepted until its
    /// [`DueKind::Impact`] is applied, which is what removes it. The
    /// [`DueKind::Impact`] event carries the sequence and nothing else, so this is
    /// where the shot's shooter, weapon, target, predicted point and round count
    /// live — everything doc 16 §3.5's re-validation at impact reads.
    pub shots: BTreeMap<u64, Shot>,
    /// Optional proving observations, excluded from execution state.
    pub(crate) acceptance: Option<super::acceptance::Acceptance>,
    /// The sequence the next accepted shot takes; the first shot of a match is `1`,
    /// so `0` is available as "no shot". It is the argument of the `hit:`, `facet:`
    /// and `burst:` draw labels (doc 16 §3.5), so it never repeats within a match
    /// and never restarts.
    pub next_shot_seq: u64,
    /// **The hash pulse phase 4's events are recorded against** (16b cut W.1), and
    /// a substep's mind landings' (a substep's reflex ticks' until 17b cut M4.6)
    /// — [`ActionHash`] says when it is taken.
    ///
    /// Doc 16 §4 asks every ledger line to carry the substep state hash, and
    /// [`crate::arena::step`]'s substep already takes **one** hash for all of its
    /// events — *the state at the substep's end, before any of that substep's events
    /// was appended* — because that is what makes the value a function of the state
    /// rather than of the order the events happened to be written in. Phase 4 is the
    /// same case read once more: it runs at one arena timestamp with no time
    /// advancing inside it, so every action it resolves happened at one moment and
    /// belongs to one hash.
    ///
    /// It is also the difference between O(1) and O(actions) hashes a pulse, and the
    /// hash is a walk of every unit and every contact: a pulse in which 150 units
    /// lock and fire measured 9.5 ms against 4.2 ms with this slot, all of it the
    /// hash (16b cut W.1's harness run). A caller outside the pulse —
    /// [`Arena::event_hash`] under [`ActionHash::Outside`] — takes a fresh hash,
    /// so a direct `resolve_fire` in a test is unchanged.
    ///
    /// **16b cut M.3** (16c `T-23`) made it lazy inside phase 4 as well: the phase
    /// used to take a whole `state_hash` every pulse, before its `ended` guard and
    /// with nothing queued, and a pulse with no discrete action and no machine's
    /// landing (no reflex tick, until 17b cut M4.6) now takes none.
    ///
    /// It is a **view** and is deliberately not folded into the state hash: it is
    /// scratch for one phase, and folding it would make the hash a function of
    /// where in the pulse it was taken. (`held_losses` was named beside it until
    /// 16b cut M.2 folded that table.) Crate-visible since 16b cut M.3, with its
    /// type: nothing outside the arena reads or writes it.
    pub(crate) action_hash: ActionHash,
    /// A `Stop` the world sent, honoured by phase 1 of the next step.
    pub stop: Option<StopReason>,
    /// Arena time: milliseconds since match start.
    pub now_ms: i64,
    /// **The last frame each pilot was sent** (doc 16 §3.1's *latest-wins slot per
    /// pilot*), so a pulse that changed nothing a pilot can see sends nothing.
    pub last_frames: FrameSlots,
    /// **What each platform held and was when its pilot lost sight of it** (16b
    /// cut Y.4; doc 07 §1.3 step 5), by **unit** — the baseline of the catch-up
    /// report ([`crate::arena::catchup`]).
    ///
    /// Written by `apply_control`'s `Linkdead` and `Released` arms for a control
    /// that names a pilot, the first one winning; removed by the `Live` that
    /// follows (consumed into [`Arena::pending_catchup`] when it re-seats a
    /// dropped pilot, dropped when it finds the pilot never left). A **view**,
    /// on `last_frames`' argument: it is a fact about what a pilot will be told
    /// and not about the match, so [`crate::arena::hash::state_hash`] does not
    /// fold it — folding it would make the hash a function of who was
    /// connected — and nothing that steers reads it. At most one row a unit.
    pub at_release: BTreeMap<ObjId, crate::arena::catchup::ReleaseSnapshot>,
    /// **The catch-up report each re-seated pilot is owed** (16b cut Y.4), by
    /// **pilot**, built at the `Live` that re-seated them and taken by
    /// `Arena::frames` onto that pilot's next frame
    /// ([`crate::arena::step::Frame::catchup`]) — so it is said once. A view,
    /// for the reason [`Arena::at_release`] is.
    pub pending_catchup: BTreeMap<ObjId, crate::arena::catchup::Catchup>,
    /// **Whether any staffer is watching this match** (16b cut I.4; 16c N10),
    /// set and cleared by the task on an `ArenaMsg::Watch` and read once by
    /// phase 5 of [`Arena::step`]: while it is set the pulse builds the staff
    /// picture ([`Arena::observer`]) at most once per pulse and at most once an
    /// arena second.
    ///
    /// A **view**, on `last_frames`' argument: a fact about who is watching and
    /// not about the match, so [`crate::arena::hash::state_hash`] does not fold
    /// it, nothing that steers reads it, and no equality compares it. `false` at
    /// [`Arena::new`].
    pub watched: bool,
    /// **The arena second the last staff picture was built in** (16b cut I.4),
    /// or `None` before the first — the whole of the once-an-arena-second gate.
    /// Cleared by every `ArenaMsg::Watch { on: true }`, so a new watcher's first
    /// pulse builds one. A view, for the reason [`Arena::watched`] is; `None`
    /// at [`Arena::new`].
    pub last_observer_s: Option<i64>,
    /// **The notices this pulse owes each platform's pilots** (16b cut S.2; doc 16
    /// §3.6: *notices fire once per transition*), keyed by the **platform** and not
    /// by the pilot, because doc 16 §3.7 puts the station on the tank and two
    /// pilots in one tank hear one scope.
    ///
    /// Staged as the pass journals its `detected`, `classified` and `lost` events —
    /// so a pilot's lines and the black box can never say different things — and
    /// cleared wholesale by `Arena::frames` at the end of the
    /// pulse, which bounds it at one pulse's transitions.
    pub pending_notices: BTreeMap<ObjId, Vec<FrameNotice>>,
    /// **Losses whose notice is being held** (16b cut S.2), `(platform, contact)`
    /// to the arena millisecond the notice becomes due.
    ///
    /// This is [`FrameNotice`]'s hysteresis, and the whole of it: a loss goes in
    /// here instead of into `pending_notices`, a re-acquisition inside the window
    /// takes it back out and says nothing, and a [`DueKind::LossSettles`] at the
    /// deadline says what is left.
    ///
    /// **16b cut M.2** (16c `T-83`) made the expiry a due event and folded the
    /// table into [`crate::arena::hash::state_hash`]. Until then a sweep at the
    /// end of every pulse said the expired entries, so the end of the hysteresis
    /// was a pulse boundary — a re-detection past the deadline was cancelled at
    /// `Quiet` and said at `Engaged`, two tempos and two stories — and the table
    /// was kept out of the hash because folding a thing a pulse boundary moved
    /// would have made the hash a function of the tempo. Now nothing but an
    /// arena timestamp moves it, and it decides what a later substep says, so it
    /// is folded under `spent_said`'s rule: it is written in the staging scan,
    /// after the substep's hash, so a write at substep *k* is first folded at
    /// *k + 1*.
    ///
    /// **16b cut Q.1** put the designator in the value beside the deadline: the
    /// row is already gone when the notice is finally said, so the name it read
    /// as has to be carried from the expiry that took it.
    pub held_losses: BTreeMap<(ObjId, ContactId), (i64, Designator)>,
    /// **The state hash of every substep of the pulse just run**, in order, and
    /// cleared at the start of each pulse (doc 16 §3.1 "Determinism": *each substep
    /// produces a state hash recorded with its pulse*).
    ///
    /// The ledger records a hash on each event, but a substep that journalled
    /// nothing would leave the sequence with a hole, and the property the hash
    /// exists for — one scripted log replayed at three tempos produces one hash
    /// sequence — is about every substep. This is where a test, the harness and cut
    /// T's watchdog read it.
    pub substep_hashes: Vec<u64>,
    /// Set once, when the match ends.
    pub ended: Option<MatchEnd>,
    /// The frozen `[tacsim.arena]` block.
    pub arena_cfg: ArenaConfig,
    /// The frozen `[tacsim.match]` block.
    pub match_cfg: MatchConfig,
    /// The frozen `[tacsim.sensors]` block (16b cut S.1).
    pub sensors_cfg: SensorsConfig,
    /// **The held-contact range that raises `Alert`**, centimetres, *resolved*
    /// (doc 16 §3.1's tempo table; 16b §9's `tempo.alert_range_cm` row, whose
    /// *2 × max weapon range* 16b cut E.1 capped — see below).
    ///
    /// A configured zero means derived, and [`Arena::new`] derives it once at
    /// check-out from the units it was handed: **the lesser of twice the longest
    /// weapon `max_range_cm` in the match ([`UnitState::weapon_reach_cm`]) and
    /// the window's own diagonal**
    /// (`isqrt(width² + height²) × cell_cm`). 16b cut E.1 added the cap, because
    /// twice a 3 km gun is 6 km on a window whose furthest two points are 3.6 km
    /// apart — a threshold no pair could be outside, which made the `Alert`
    /// trigger read "a contact is held at all" (16c `T-32`).
    ///
    /// It is resolved here rather than read from the configuration on every pulse
    /// so that a match's `Alert` threshold cannot change under it, exactly as the
    /// two configuration blocks are cloned in for the same reason.
    pub alert_range_cm: u32,
    /// **The widest jamming bubble in this match**, centimetres, *resolved* (16b cut
    /// X.4; doc 16 §9 item 4) — and **zero for every match in which nothing carries
    /// a pod**, which is every match the game ships.
    ///
    /// It is the one radius [`crate::arena::Arena::jamming_at`] queries
    /// the spatial index at, and the zero is what lets that function return before
    /// touching the index at all: the ECM term costs a stock match one integer
    /// comparison per sensor pass and nothing else.
    ///
    /// Resolved at check-out from the units it was handed, exactly as
    /// `alert_range_cm` is, and for the same reason: a match's reach cannot change
    /// under it. A pod that is wrecked or shut down mid-match stops radiating — the
    /// scan reads both at scan time — but the *query radius* stays where check-out
    /// put it, which costs one index query of a bubble nobody is filling and is the
    /// honest price of not recomputing a maximum per pass.
    pub jam_radius_cm: u32,
    /// **The jamming each unit is standing in, folded at most once between two
    /// clears** (16b cut U.2, 16c `T-29`; doc 16 §9 item 4): unit to its raw
    /// [`crate::arena::perception::Jamming`] off
    /// [`crate::arena::Arena::jamming_at`] — `None` for no live pod in reach —
    /// with no filter applied (each caller recovers its own). Read and filled
    /// only through [`crate::arena::Arena::jam_fold_of`].
    ///
    /// **It is exact, not approximate.** The fold reads five things: the unit's
    /// pose, and for every pod in the bubble its pose, its `power`, its
    /// `limits.wreck` and its switch `ecm_on` (a pod's `jammer` row is fixed at
    /// check-out). The memo is cleared at **both** `SpatialIndex::rebuild` sites
    /// in `Arena::step`'s substep — the poses are fixed from each rebuild to the
    /// next — after **every `StartupComplete` and every `Impact`** the due loop
    /// applies, which are the only two things inside a substep that move a pod's
    /// power or wreck (a startup brings a pod online; an impact can wreck its
    /// hull, which turns the power off), and by
    /// [`crate::arena::intents::resolve_emcon`] on every flip of a pod's switch
    /// (16b cut O.2). Between two clears nothing the fold reads can move, so a
    /// memoised answer is the answer `jamming_at` would give.
    ///
    /// A `BTreeMap`, two integers a row, and empty for every match with no pod:
    /// [`crate::arena::Arena::jam_fold_of`] returns before touching it when
    /// `jam_radius_cm` is zero. **Not folded into
    /// [`crate::arena::hash::state_hash`]**: it is a cache of state the hash
    /// already folds (every pose, power byte and wreck byte it is a function
    /// of), so folding it would add no information and would make the hash a
    /// function of which folds had been asked for by the time it was taken.
    pub jam_fold: BTreeMap<ObjId, Option<crate::arena::perception::Jamming>>,
    /// **The squads this match's machine units are commanded in** (16b cut N.2;
    /// doc 16 §6 "Deliberation": *per squad or commander rather than per unit*;
    /// reflex or, since 17b cut L.2, mind; a mind alone since 17b cut M4.6),
    /// derived at check-out by
    /// [`crate::deliberation::squads_of`] — the same function, over the same
    /// [`CustodyUnit::squad_input`] triples and the same
    /// `[tacsim.arena].squad_size` (16b cut Z.1), that files them on the match
    /// record, so the two groupings cannot drift.
    ///
    /// Empty for a match with no AI at all: an arena with no squads reports no
    /// signals and accepts no order, which is exactly what doc 16 §1's two-tank
    /// story wants.
    pub squads: Vec<crate::deliberation::squad::SquadRoster>,
    /// **What the world was last told about each squad** (16b cut N.2), so the
    /// trigger can be *event-driven* rather than a poll.
    ///
    /// A **view**, like `action_hash` and `last_frames` (and unlike `held_losses`,
    /// which 16b cut M.2 folded): it is a fact about what
    /// has been reported and not about the match, so
    /// [`crate::arena::hash::state_hash`] does not fold it. Folding it would make
    /// the hash a function of the request cadence, which is the opposite of what
    /// the cross-tempo property asks for.
    pub squad_watch: crate::arena::command::SquadWatchTable,
    /// Squads whose standing order ran out since the last signal was taken (16b
    /// cut N.2) — the `order_expired` trigger, drained by
    /// `Arena::take_deliberation_signals`. A view, for the same reason.
    pub squads_expired: crate::arena::command::ExpiredSquads,
    /// **The rule this match is fought under** (16b cut O.1; 16c N11), off
    /// [`CustodyOut::mode`] and never moved. Folded into the state hash (item
    /// 12) by its code byte.
    pub mode: MatchMode,
    /// **The Hold objective and its clock** (16b cut O.1), `Some` exactly when
    /// [`Arena::mode`] is [`MatchMode::Hold`] — see [`HoldState`]. `None` is what
    /// makes the presence test cost a `Skirmish` match one `Option` test per
    /// substep and nothing more.
    pub hold: Option<HoldState>,
}

impl Arena {
    /// **Open a match** from the check-out message.
    ///
    /// In order: the units are sorted by id and checked for duplicates; each
    /// pose is checked to be a bearing on the window; each seated pilot's
    /// station raises the epoch table; the spatial index is built over the
    /// opening poses; the first `position_summary` is scheduled at
    /// `ledger.summary_ms` and the duration bound at `match.max_duration_ms`
    /// (16b cut M.2); and `match_started` is the ledger's first event,
    /// carrying the opening state hash.
    ///
    /// # Stations and epochs
    ///
    /// A station is an owner-local id that grants a *set* of channels
    /// (`mush_core::ChannelGrant`), so this constructor reads
    /// [`Channel::parse`]: a station literally named `helm`, `sensors` or `fire`
    /// raises that one channel, and any other station name raises all three —
    /// the cockpit seat of doc 16 §3.7. Raising never lowers, so cut T is free
    /// to publish narrower per-channel epochs with an explicit `Control`
    /// immediately afterwards.
    ///
    /// # Errors
    ///
    /// [`ArenaError`] naming the unit, the pilot or the terrain. Nothing is
    /// partially built: a refusal leaves no arena.
    pub fn new(
        custody: CustodyOut,
        arena_cfg: &ArenaConfig,
        match_cfg: &MatchConfig,
        sensors_cfg: &SensorsConfig,
    ) -> Result<Arena, ArenaError> {
        Self::new_with_minds(
            custody,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            &crate::config::MindsConfig::default(),
        )
    }

    /// Admit custody with the exact frozen mind limits before binding any card,
    /// native program, slot or initial think schedule.
    pub fn new_with_minds(
        custody: CustodyOut,
        arena_cfg: &ArenaConfig,
        match_cfg: &MatchConfig,
        sensors_cfg: &SensorsConfig,
        minds_cfg: &crate::config::MindsConfig,
    ) -> Result<Arena, ArenaError> {
        let CustodyOut {
            definition,
            units,
            pilots,
            resolution,
            seed,
            mode,
        } = custody;
        if units.is_empty() {
            return Err(ArenaError::NoUnits);
        }
        // **16b cut O.1** (16c N11): the mode, first of the definition's own
        // refusals — a ground that does not declare it cannot be fought under it.
        // The spot's cell and its letter are resolved here, once, so the presence
        // test at every substep reads four integers and no list.
        let hold = match mode {
            // 16b cut O.3: a range carries no objective; its targets end it by
            // the elimination rule.
            MatchMode::Skirmish | MatchMode::Range => None,
            MatchMode::Hold => {
                let rule = definition
                    .hold_rule()
                    .ok_or_else(|| ArenaError::ModeUnsupported {
                        mode: mode.as_str(),
                        arena: definition.id.clone(),
                    })?;
                let spot = definition
                    .key_spots
                    .get(usize::from(rule.spot))
                    .ok_or_else(|| ArenaError::ModeUnsupported {
                        mode: mode.as_str(),
                        arena: definition.id.clone(),
                    })?;
                Some(HoldState {
                    spot: (spot.x, spot.y),
                    letter: spot_letter(usize::from(rule.spot)),
                    radius_cells: rule.radius_cells,
                    hold_ms: rule.hold_ms,
                    held_by: None,
                })
            }
        };
        // 16b cut N.2: who is commanded, and by whom (see `squads` below), read
        // off the check-out before it is consumed. 17b cut L.2 (D-UM-53): a
        // machine of either controller — a mind's hull is dealt as a reflex one
        // was (the only machine since 17b cut M4.6), so a lance-leader's order
        // reaches it — through the one builder
        // `match_::start` calls too ([`CustodyUnit::squad_input`]); each state
        // below carries its unit's controller and id unchanged, in this order.
        let squad_input = CustodyUnit::squad_input(&units);
        let mut states: Vec<UnitState> = Vec::with_capacity(units.len());
        // **17b cut M3.6** (doc 17 §13): each unit's `mind_sealed` line, off
        // the custody's own seal before the custody is consumed; journalled
        // below, in unit-id order, right after `match_started`.
        let mut sealed: Vec<(ObjId, EventKind)> = Vec::with_capacity(units.len());
        let mut cards = crate::arena::runtime_card::CardCatalog::default();
        let mut profiles = BTreeMap::new();
        let mut conditions = crate::arena::conditions::ConditionTable::default();
        for mut unit in units {
            sealed.push((unit.id, unit.seal.sealed_line()));
            // **16b cut AF.1**: the heading and the window, in one function the
            // snapshot's decode calls too (`check_poses`), in the order they
            // always ran.
            let opening_cell = check_pose(&definition.terrain, unit.id, &unit.pose)?;
            // **16b cut AE.1**: the row this unit pays, read beside its slope
            // limit below and first because the opening test needs it.
            let cost_row = CostRow::read(&unit.unit);
            // **16b cut AE.3**: and its class, off the same record, for the cap
            // counted below once every unit is in hand.
            let class = UnitClass::read(&unit.unit);
            // **16b cut M.4** (16c `T-14`): and on a cell a unit may stand on —
            // **its own row** since 16b cut AE.1, so a car handed over on rubble
            // is refused here as a tank in deep water is. The announcement names
            // nobody, so the unit and the cell go to the log.
            if !definition
                .terrain
                .passable_for(cost_row, opening_cell.0, opening_cell.1)
            {
                tracing::warn!(
                    unit = %unit.id,
                    x = opening_cell.0,
                    y = opening_cell.1,
                    "arena refused: a unit's opening cell is not passable"
                );
                return Err(ArenaError::CellImpassable {
                    unit: unit.id,
                    x: opening_cell.0,
                    y: opening_cell.1,
                });
            }
            // **16b cut X.6** (doc 16 §3.2 "Terrain"): *a unit's `z_cm` equals the
            // cell's `elev_cm`*, and that is true from the opening hash and not
            // merely from the first crossing. [`CustodyOut`] is the only door into
            // an arena; the arena is the authority for its own grid, so it does not
            // trust an inbound z — a stale one, or the flat zero every world-side
            // placement writes, is corrected here rather than carried for however
            // many substeps it takes the tank to cross a boundary.
            if let Some(cell) = definition.terrain.get(opening_cell.0, opening_cell.1) {
                unit.pose.z_cm = i32::from(cell.elev_cm);
            }
            // **16b cut X.1**: the heat numbers, read off the cells the
            // `TacticalUnit` already carries rather than across a widened seam
            // (see [`HeatState`]). `None` for every unit whose content declares
            // no ceiling.
            let heat = HeatState::read(&unit.unit, &unit.profile, &unit.weapons);
            // **16b cut X.3**: the sight's thermal channel, read the same way and
            // `None` for every unit whose mount authors none.
            let thermal = ThermalSignature::read(&unit.unit);
            // **16b cut X.4**: the jamming pod, likewise — and the window's own
            // cell size applied here, once, so the pass compares centimetres.
            let jammer = Jammer::read(&unit.unit, definition.terrain.cell_cm());
            // **16b cut X.5**: the counter-jamming filter, likewise — and `None`
            // when its component has been shot off, which is the one thing that
            // stops a filter working.
            let eccm = Eccm::read(&unit.unit);
            // **16b cut X.6**: the steepest edge this unit's locomotion will
            // cross, read off the same record and `None` for a unit that declares
            // no locomotion the shipped table knows — every engine fixture, which
            // therefore still drives and still pays the surcharge.
            let slope_limit_pct = crate::arena::slope::SlopeLimit::read(&unit.unit);
            // **16b cut F.1** (doc 16 §3.1 "Custody"): the match opens on the unit
            // the world hands over, and that unit's own record may already say it
            // is a wreck, has lost a track or has had its turret shot off. Until
            // this cut every match opened at `MotionLimits::default()`, so a hulk
            // drove and shot and could never be killed again; this reads the
            // consequences off the record and puts them in force before pulse one.
            let entry = crate::arena::damage::entry_consequences(&unit.unit, &unit.profile);
            // **16b cut E.2** (`B-45`): which sections of this unit are the body,
            // as one mask in the unit's own section order — the order every
            // `Condition` is packed in, so a reading and this mask can be ANDed.
            // Read once here for the reason the slope limit and the heat cells
            // are: it is a fact about the fit that nothing in a match moves.
            let fatal_sections = fatal_section_mask(&unit.unit, &unit.profile);
            let operands = unit
                .weapons
                .iter()
                .map(|weapon| crate::arena::rules::RuleWeapon {
                    id: weapon.id.clone(),
                    rounds_max: weapon.count_per_load,
                })
                .collect::<Vec<_>>();
            let admission_error = |error: crate::arena::schema::SchemaError| ArenaError::Mind {
                unit: unit.id,
                reason: error.to_string(),
            };
            unit.seal
                .verify(&minds_cfg.classes, &operands)
                .map_err(admission_error)?;
            if unit.seal.odds_floor_permille != minds_cfg.odds_floor_permille {
                return Err(ArenaError::Mind {
                    unit: unit.id,
                    reason: "seal floor differs from frozen match floor".into(),
                });
            }
            let runtime = crate::arena::runtime::prepare_profile_runtime(
                &mut cards,
                unit.id,
                &unit.seal.cards,
                &unit.seal.rom,
                &minds_cfg.classes,
                unit.seal.class,
                &operands,
                &unit.seal.channels,
                minds_cfg.odds_floor_permille,
            )
            .map_err(admission_error)?;
            runtime
                .program
                .install(
                    &mut conditions,
                    unit.id,
                    minds_cfg.classes.row(unit.seal.class).slots,
                )
                .map_err(admission_error)?;
            profiles.insert(unit.id, runtime);
            let weapons: Vec<WeaponSummary> = unit
                .weapons
                .into_iter()
                .filter(|weapon| entry.carries(&weapon.mount_section))
                .collect();
            let weapons_left = weapons.len();
            // **16b cut Q.2**: the first value of the band cache, off the record
            // the world handed over — so a tank that checks in with a shot-off
            // turret reads `destroyed` there from the opening frame, exactly as
            // cut F.1's entry consequences already put its lost track in force.
            let condition = crate::arena::condition::Condition::of(
                &unit.unit,
                sensors_cfg.condition_green_permille.min(1_000),
                sensors_cfg.condition_yellow_permille.min(1_000),
            );
            states.push(UnitState {
                id: unit.id,
                team: unit.team,
                // **16b cut Q.1**: the slot as it was handed over, carried here
                // unresolved — `resolve_callsigns` below turns a `0` into the
                // lowest free slot of the team once every unit is in hand and the
                // vector is in unit-id order.
                callsign: Designator::Slot {
                    team: unit.team,
                    slot: u16::from(unit.slot),
                },
                desired_heading_mdeg: unit.pose.heading_mdeg,
                pose: unit.pose,
                speed_cms: 0,
                desired_speed_cms: 0,
                power: Power::Off,
                mobility: unit.mobility,
                slope_limit_pct,
                cost_row,
                class,
                fatal_sections,
                // A unit opens the match standing still, so it entered its cell by
                // no edge at all: grade zero, which is the flat factor.
                grade_pct: 0,
                // **16b cut E.3**: and it is not standing against anybody.
                occupied_hold: None,
                // **16b cut Q.6**: and it has spent no arena time moving, so there
                // is no fraction of a turn, a speed change or a centimetre to
                // carry into its first substep.
                carry: MotionCarry::default(),
                limits: entry.limits,
                sensor_height_cm: if unit.sensor_height_cm == 0 {
                    DEFAULT_SENSOR_HEIGHT_CM
                } else {
                    unit.sensor_height_cm
                },
                // A wreck sees nothing (`Arena::destroy_unit`'s own rule), so one
                // that arrives as a wreck sees nothing either.
                sensor: if entry.limits.wreck {
                    None
                } else {
                    unit.sensor
                },
                thermal,
                jammer,
                // **16b cut O.2**: every pod checks out radiating (the owner's
                // default, 16c N12); a unit with no pod carries the switch too and
                // nothing reads it.
                ecm_on: true,
                eccm,
                // **16b cut D.4**: a unit checks out with a clean receiver and
                // learns otherwise at its first pass. There is no notice for it:
                // the transition is what is said, and there has been none.
                receiver: None,
                // 16b cut W.1: the two live cells, one row per weapon, laid down
                // here so that every later lookup is a read of a complete table.
                // A weapon id declared twice on one unit keeps one row, which is
                // the same thing the component table itself is.
                ready_at_ms: weapons
                    .iter()
                    .map(|weapon| (weapon.id.clone(), 0_i64))
                    .collect(),
                // **16b cut F.1**: the rounds the unit's own `ammo` cell carries,
                // not the catalog's full load. A tank that checked in with 31
                // rounds checks out with 31. The authored load is the fallback for
                // a unit whose record declares no such cell at all — every fixture
                // arena in this crate, and any content whose weapon keeps its
                // ammunition somewhere else.
                ammo: weapons
                    .iter()
                    .map(|weapon| {
                        (
                            weapon.id.clone(),
                            rounds_on_record(&unit.unit, &weapon.id)
                                .unwrap_or(weapon.count_per_load),
                        )
                    })
                    .collect(),
                weapons,
                profile: unit.profile,
                // Doc 16 §3.5 step 2: *every stock rating is 1 through the
                // `PilotProfile` seam*, which is also the sheet's own floor, so a
                // check-out that named none resolves a green gunner rather than
                // dividing by zero.
                gunnery: if unit.gunnery == 0 {
                    crate::arena::resolution::STOCK_GUNNERY
                } else {
                    unit.gunnery
                },
                heat,
                controller: unit.controller,
                // **17b cut M3.3** (doc 17 §13): the seal's reference, off the
                // custody's own seal — what the check-out sealed, never
                // re-derived here (a resume decodes it whole).
                seal: unit.seal.seal_ref(),
                // **17b cut M5.2**: every hull checks out on its first
                // sealed profile, never switched.
                card_profile: crate::arena::card::ActiveProfile::FIRST,
                unit: unit.unit,
                condition,
                lock: None,
                blocked: false,
                held: None,
                linkdead: false,
                // **16b cut F.1**: doc 16 §3.8's *disabled — no weapons and no
                // motion* is a fact about the unit, so a unit that arrives that
                // way arrives disabled. A wreck is past disabled and is never
                // journalled both (`Arena::settle_disabled`'s own rule).
                disabled: !entry.limits.wreck && weapons_left == 0 && entry.limits.motion_disabled,
                shots_fired: 0,
                hits: 0,
                own_last_hit_ms: None,
                // **16b cut M.1** (`T-82`): no seat has ordered anything yet.
                orders_window_s: 0,
                orders_in_window: 0,
            });
        }
        states.sort_by_key(|state| state.id);
        if let Some(duplicate) = states
            .windows(2)
            .find(|pair| pair.first().map(|a| a.id) == pair.get(1).map(|b| b.id))
            .and_then(|pair| pair.first())
            .map(|state| state.id)
        {
            return Err(ArenaError::DuplicateUnit { unit: duplicate });
        }
        // **16b cut AE.3** (doc 16 §2 W2: *class counts on the map obey the
        // caps*): every unit counted per side and class, in the map's order, and
        // the first side and class over `[tacsim.arena].max_units_by_class`
        // refused before anything else is built — the invariant the staff
        // `@arena/start` path meets here and the lobby's `SIM UNIT` meets at the
        // choice. A class the map does not name is uncapped.
        //
        // **Doc 18 cut IC2** (doc 18 §4.2, D-18-29): first, a class no arena
        // fields at all — the first such unit in id order — is refused outright;
        // a hauler is never capped, it is never fielded.
        if let Some(state) = states.iter().find(|state| !state.class.fieldable()) {
            return Err(ArenaError::UnfieldableClass {
                team: state.team.as_str(),
                class: state.class.as_str(),
            });
        }
        let mut fielded: BTreeMap<(Team, UnitClass), u32> = BTreeMap::new();
        for state in &states {
            let count = fielded.entry((state.team, state.class)).or_insert(0);
            *count = count.saturating_add(1);
        }
        if let Some(((team, class), count, cap)) = fielded.iter().find_map(|(key, count)| {
            arena_cfg
                .max_units_by_class
                .get(&key.1)
                .filter(|cap| *count > **cap)
                .map(|cap| (*key, *count, *cap))
        }) {
            return Err(ArenaError::ClassCap {
                team: team.as_str(),
                class: class.as_str(),
                fielded: count,
                cap,
            });
        }
        // **16b cut Q.1**: every unit's public callsign, resolved once, here —
        // after the sort, so the fallback order is unit-id order and a replay of
        // one check-out resolves the same names.
        resolve_callsigns(&mut states)?;
        let mut epochs = EpochTable::new();
        for pilot in &pilots {
            if states
                .binary_search_by_key(&pilot.unit, |state| state.id)
                .is_err()
            {
                return Err(ArenaError::UnknownPilotUnit {
                    unit: pilot.unit,
                    pilot: pilot.pilot,
                    station: pilot.station.clone(),
                });
            }
            match Channel::parse(&pilot.station) {
                Some(channel) => {
                    epochs.raise(pilot.unit, channel, pilot.epoch);
                }
                None => {
                    for channel in Channel::ALL {
                        epochs.raise(pilot.unit, channel, pilot.epoch);
                    }
                }
            }
        }
        let mut due = DueHeap::new();
        due.schedule(
            i64::from(arena_cfg.ledger.summary_ms),
            DueKind::PositionSummary,
        );
        // **16b cut M.2** (16c `T-01`): the duration bound, as a due event, so the
        // timeout is settled at its own millisecond (see `DueKind::MatchDeadline`).
        // Scheduled second, after the summary and before the passes and the
        // thinks (and the reflex ticks, until 17b cut M4.6): the order of
        // scheduling is what hands out the sequence numbers, so
        // it is fixed here once and every arena opened from one check-out numbers
        // its schedule identically — and the opening hash folds the due count,
        // which this event is now one of.
        due.schedule(i64::from(match_cfg.max_duration_ms), DueKind::MatchDeadline);
        // Doc 16 §3.4 step 5: occupancy is one unit per cell. Motion is what keeps
        // it true once the match is running (step 4 refuses an occupied cell), and
        // this is what makes it true to begin with — the alternative is a cell whose
        // occupancy has two answers, which every later reader of the index would
        // have to carry a rule for. **16b cut A.2 added this check**; cut T's
        // check-out is its caller and places every tank it checks out on a cell of its own.
        // **16b cut AF.1** moved it into `check_occupancy`, which the snapshot's
        // decode calls too, unchanged.
        check_occupancy(&definition.terrain, &states)?;
        let mut index = SpatialIndex::new(&definition.terrain);
        index.rebuild(&definition.terrain, &states);
        // 16b cut N.1: the cheapest cell on the window, read once here so the A*
        // heuristic is admissible without a scan per query
        // (`crate::arena::path::min_move_cost`). **16b cut M.4** (`T-15`): and the
        // speed ceiling reads it too, so it is held on the arena, once. **16b cut
        // AE.1**: one per row, copied from the terrain, which took them when it
        // was built.
        let min_cost = CostRow::ALL.map(|row| definition.terrain.min_cost(row));
        // **16b cut M.4** (16c `B-57`): the substep floor may not beat the
        // one-cell bound. The ceiling is the terrain factor's (`speed_ceiling_cms`),
        // so a window with a cell cheaper than 100 is judged at the speed its
        // terrain factor allows — a conservative margin, since the order clamp
        // (`apply_intent`) keeps every unit at or under its authored speed. Each
        // unit on its own row's cheapest cell (16b cut AE.1).
        let ceiling_cms = speed_ceiling_cms(&states, &min_cost);
        let cell_cm = definition.terrain.cell_cm();
        if u64::from(crate::arena::clock::MIN_SUBSTEP_MS).saturating_mul(u64::from(ceiling_cms))
            > u64::from(cell_cm.saturating_sub(1)).saturating_mul(1_000)
        {
            return Err(ArenaError::SubstepBound {
                ceiling_cms,
                cell_cm,
            });
        }
        // 16b §9's `tempo.alert_range_cm`: **the lesser of twice the longest
        // weapon range in the match and the window's own diagonal**, and the
        // configured `0` is what asks for that. Derived once, here, from the units
        // this match was handed; a match whose units carry no weapon derives `0`
        // and never raises `Alert` on a held contact, which is the honest answer.
        // The reach half is `UnitState::weapon_reach_cm`, which exists to be read
        // here and is read nowhere else in the crate.
        //
        // **16b cut E.1 (16c `T-32`) added the diagonal cap.**
        // Twice the stock cannon's 3 km is 6 km on a window 2.56 km on a side,
        // whose furthest two points are 3.6 km apart — so the threshold was a
        // number no pair of units could ever be outside, and `Alert`'s "a contact
        // is held inside `alert_range_cm`" meant "a contact is held". Capped, it
        // is honest on any window, and on the shipped one it is still wider than
        // anything a sight plus a mesh hop can hold (900 m), so nothing about a
        // shipped match's tempo moves.
        let alert_range_cm = if arena_cfg.tempo.alert_range_cm == 0 {
            let twice = u64::from(
                states
                    .iter()
                    .map(UnitState::weapon_reach_cm)
                    .max()
                    .unwrap_or(0),
            )
            .saturating_mul(2);
            let width = u64::from(definition.terrain.width());
            let height = u64::from(definition.terrain.height());
            let diagonal = width
                .saturating_mul(width)
                .saturating_add(height.saturating_mul(height))
                .isqrt()
                .saturating_mul(u64::from(definition.terrain.cell_cm()));
            u32::try_from(twice.min(diagonal)).unwrap_or(u32::MAX)
        } else {
            arena_cfg.tempo.alert_range_cm
        };
        // **16b cut X.4**: the widest pod bubble on the field, and `0` when there is
        // no pod — see `jam_radius_cm`.
        let jam_radius_cm = states
            .iter()
            .filter_map(|state| state.jammer.map(|jammer| jammer.radius_cm))
            .max()
            .unwrap_or(0);
        // 16c T-105: each unit's chassis-static words, once, in the unit
        // table's order.
        let chassis = ChassisWords::table(&states);
        let mut arena = Arena {
            // 17b cut M2.2: the map behind its one `Arc`.
            definition: Arc::new(definition),
            units: states,
            chassis,
            epochs,
            contacts: ShadowContacts::new(),
            designators: DesignatorBook::default(),
            lost_designators: BTreeMap::new(),
            // 16b cut D.2: the spent-round scratch and the spent-round window.
            spent_hits: BTreeSet::new(),
            spent_said: BTreeMap::new(),
            due,
            ledger: Ledger::new(&arena_cfg.ledger),
            index,
            clock: Clock::new(),
            tape_cursor: None,
            tape_witnesses: super::input_tape::WitnessBuffer::default(),
            seed,
            resolution,
            pilots,
            // 17b cut M1a.1: the motor table, empty until the check-out's
            // adoptions below lay a row down for each machine.
            motors: crate::arena::motor::MotorTable::new(),
            // 17b cut M1a.5: no mind until the adoptions below; the runner is
            // the one `snapshot.rs`' decode sets too.
            minds: BTreeMap::new(),
            order_watches: super::order_watch::OrderWatches::default(),
            cards,
            profiles,
            // 17b cut M1b.1: no slot, nothing said, and the default floor —
            // `spawn_arena` writes the configured one before the move.
            conditions: crate::arena::conditions::ConditionTable::default(),
            held_said: BTreeMap::new(),
            odds_floor_permille: minds_cfg.odds_floor_permille,
            // 17b cut M3.1: the stock table — `spawn_arena` writes the
            // configured table before the move. (Standard Doctrine's rating
            // is each hull's seal's since 17b cut M4.1, `UnitState::seal`.)
            classes: minds_cfg.classes,
            // 17b cut M4.1: no think awaits, none is collected.
            awaiting: BTreeSet::new(),
            traces: super::author_trace::TraceStore::default(),
            collected: Vec::new(),
            unsettled: Vec::new(),
            // 17b cut M1b.5: no card change offered yet.
            pending_cards: BTreeMap::new(),
            mind_runner: crate::arena::mind::default_mind_runner(arena_cfg),
            // 17b cut M3.5: no hull's own runner until the sealed installer
            // runs over the check-out (`crate::arena_task::spawn_arena`).
            runners: BTreeMap::new(),
            min_cost,
            pending: BTreeMap::new(),
            shots: BTreeMap::new(),
            acceptance: None,
            next_shot_seq: 0,
            action_hash: ActionHash::Outside,
            stop: None,
            now_ms: 0,
            last_frames: FrameSlots::new(),
            // 16b cut Y.4: nobody has lost sight of anything yet.
            at_release: BTreeMap::new(),
            pending_catchup: BTreeMap::new(),
            // 16b cut I.4: nobody is watching yet.
            watched: false,
            last_observer_s: None,
            pending_notices: BTreeMap::new(),
            held_losses: BTreeMap::new(),
            substep_hashes: Vec::new(),
            ended: None,
            arena_cfg: arena_cfg.clone(),
            match_cfg: match_cfg.clone(),
            sensors_cfg: sensors_cfg.clone(),
            alert_range_cm,
            jam_radius_cm,
            // 16b cut U.2: filled by the first fold of the first substep.
            jam_fold: BTreeMap::new(),
            // **16b cut N.2**: the squads are *derived* from the check-out, by the
            // same `squads_of` the match record files them with — a side's machine
            // units (reflex or, since 17b cut L.2, mind) dealt into squads of at
            // most `[tacsim.arena].squad_size` (16b cut Z.1), lowest id
            // commanding. Derived rather than handed over
            // because two groupings of one match would be two answers to "which
            // squad is that order for", and because it means the harness and every
            // fixture get the same grouping the production lobby does: the size
            // is read off the same `ArenaConfig` the match record's call reads, so
            // the two are one function of one data.
            squads: crate::deliberation::squad::squads_of(&squad_input, arena_cfg.squad_size),
            squad_watch: crate::arena::command::SquadWatchTable::new(),
            squads_expired: crate::arena::command::ExpiredSquads::new(),
            // 16b cut O.1: nobody holds anything at check-out; the first
            // substep's end is the first reading.
            mode,
            hold,
        };
        let ids = arena.units.iter().map(|unit| unit.id).collect::<Vec<_>>();
        for unit in ids {
            arena.constrain_profile_hardware(unit);
        }
        // Doc 16 §3.6: one pass per unit per `sensors.pass_ms`, staggered by unit
        // id. Scheduled after the arena is built because the stagger reads the
        // sorted unit vector, and before the opening hash because the hash folds
        // the schedule's length.
        arena.schedule_first_passes();
        // 17b cut M1a.5 (doc 17 §6): one think per mind, on the same shape —
        // staggered by unit id on the motor's grid (16b cut N.1's reflex ticks
        // were scheduled here, before the thinks, until 17b cut M4.6 deleted
        // the reflex controller; no production hull had one since 17b cut
        // M4.3, so the opening hash's due count never held one).
        arena.schedule_first_mind_thinks();
        let opening = crate::arena::hash::state_hash(&arena);
        // A ledger that refuses its own first event is a black box that will not
        // record anything; the match still opens, and cut A.2's step reads
        // `ledger.overflowed()` on its own terms.
        let _ = arena
            .ledger
            .append(0, EventKind::MatchStarted, Vec::new(), opening);
        // **17b cut M3.6** (doc 17 §13 "The seal", D-UM-45, D-UM-46): every
        // sortie opens on the tape — one `mind_sealed` per unit, in unit-id
        // order, at `t_ms` 0 under the opening hash, from the record the
        // check-out sealed (never from a view of the arena). These lines shift
        // the ledger's sequence, which `state_hash` folds, so every match's
        // hash sequence moved here once (17b cut M3.6's declared re-bless).
        sealed.sort_by_key(|(id, _)| *id);
        for (_, line) in sealed {
            let _ = arena.ledger.append(0, line, Vec::new(), opening);
        }
        super::snapshot_capacity::admit(&arena).map_err(|error| ArenaError::SnapshotCapacity {
            reason: error.to_string(),
        })?;
        Ok(arena)
    }

    /// One unit by id, by binary search over the sorted vector.
    #[must_use]
    pub fn unit(&self, id: ObjId) -> Option<&UnitState> {
        self.units.get(self.unit_at(id)?)
    }

    /// **The unit's row in [`Arena::units`]** (and so in every table kept in
    /// the unit table's order, [`Arena::chassis`] among them), by the binary
    /// search [`Arena::unit`] has always made: the table is laid down in
    /// unit-id order by [`Arena::new`] and never reordered.
    #[must_use]
    pub(crate) fn unit_at(&self, id: ObjId) -> Option<usize> {
        self.units.binary_search_by_key(&id, |state| state.id).ok()
    }

    /// **A cull's candidate, read by its row** (16c `T-107b`):
    /// [`Arena::unit_by_row`] over the candidate's row and id — `units[row]`
    /// when that element's id is the candidate's, which it is for every
    /// candidate [`crate::arena::index::SpatialIndex::in_range_rows`] answers
    /// over [`Arena::units`], the slice the index is rebuilt over and the one
    /// the passes hand it — and otherwise [`Arena::unit`], the binary search
    /// the perception passes made for every candidate before this cut.
    #[must_use]
    pub(crate) fn unit_row(&self, candidate: &Candidate) -> Option<&UnitState> {
        self.unit_by_row(
            usize::try_from(candidate.row).unwrap_or(usize::MAX),
            candidate.id,
        )
    }

    /// **The verified read by row** (16c `T-107b`): `units[row]` when that
    /// element's id is `id`, and otherwise [`Arena::unit`]'s binary search.
    /// The read and the search find the one element of an id-sorted,
    /// duplicate-free table, so the answer is `unit(id)` either way; a row
    /// past the table (`usize::MAX` for a candidate with no hint) or one
    /// naming another unit falls to the search
    /// (`t107b_unit_row_reads_the_row_and_falls_back_to_the_search`).
    /// [`Arena::unit_row`] is it over a candidate; `share_pass` takes it over
    /// the row [`Arena::unit_at`] found for the sharer at its head, which
    /// holds across the folds between because nothing reorders the table.
    #[must_use]
    pub(crate) fn unit_by_row(&self, row: usize, id: ObjId) -> Option<&UnitState> {
        self.units
            .get(row)
            .filter(|unit| unit.id == id)
            .or_else(|| self.unit(id))
    }

    /// The same, mutably.
    pub fn unit_mut(&mut self, id: ObjId) -> Option<&mut UnitState> {
        let index = self
            .units
            .binary_search_by_key(&id, |state| state.id)
            .ok()?;
        self.units.get_mut(index)
    }

    /// **Every unit's callsign, in unit-id order** (16b cut V.2): the one place
    /// the slot rule of 16b cut Q.1 was applied, read back for the result record.
    ///
    /// A pure read of [`UnitState::callsign`] over the already id-sorted
    /// [`Arena::units`]. The world side reads it off the arena
    /// `crate::arena_task::spawn_arena` has just built, before the arena moves
    /// into its task, rather than re-deriving the rule from the roster: the
    /// roster's order is not the slot order (explicit slots resolve first, then
    /// the slot-0 units in unit-id order), and one source of truth cannot
    /// disagree with itself.
    #[must_use]
    pub fn callsigns(&self) -> Vec<(ObjId, Designator)> {
        self.units
            .iter()
            .map(|state| (state.id, state.callsign))
            .collect()
    }

    /// **The one assignment of [`UnitState::unit`] in non-test code** (16b cut
    /// Q.2; doc 16 §3.5 "Damage", D-16-6): take the unit `mush_tactics::apply`
    /// handed back, and recompute the band cache beside it.
    ///
    /// Cut W.2's rule was already that a unit's cells move only through one
    /// [`mush_tactics::UnitMutationPlan`] per change — but the *assignment* had
    /// two production sites, the impact in [`crate::arena::damage`] and the heat
    /// plan in [`crate::arena::heat`], and cut Q.2 added a cache
    /// ([`UnitState::condition`]) that both of them would have had to remember.
    /// A cache with two writers is a cache that rots, and a rotted band is a
    /// `SCAN` that tells a gunner a spent turret is green. So there is one
    /// function, both call it, and a third site added later is a diff somebody
    /// reads rather than a silent divergence.
    ///
    /// A unit this arena does not hold is a no-op: the caller has already
    /// resolved it, and `apply` produced a unit for an id that has left the match
    /// only if the match let go of it inside the same substep. **16b cut M.3**
    /// (16c `T-28`): it returns before computing anything, where it used to
    /// compute the whole band cache for a unit it then threw away.
    ///
    /// **`touched_bands`** (16b cut M.3, 16c `T-28`): whether the plan that made
    /// `post` wrote anything the bands are a function of — a section or facet
    /// cell, or a section's condition — which `plan_touches_bands` decides from
    /// the plan at each call site. `false` leaves [`UnitState::condition`] exactly
    /// as it was: a heat plan writes one section cell that is not armour, and
    /// recomputing every band on every substep's cooling for every hot unit was
    /// the cost this flag removes. It is the caller's claim and not a guess:
    /// `Condition::of` reads the `armor` and `internals` cells and the section
    /// conditions and nothing else, and the heat cell is neither.
    pub(crate) fn set_unit(&mut self, id: ObjId, post: TacticalUnit, touched_bands: bool) {
        // Read the floors before the borrow, and read them `.min(1000)` the way
        // every consumer of these keys does (their validation is Q.7's).
        let green = self.sensors_cfg.condition_green_permille.min(1_000);
        let yellow = self.sensors_cfg.condition_yellow_permille.min(1_000);
        let Some(state) = self.unit_mut(id) else {
            return;
        };
        if touched_bands {
            state.condition = crate::arena::condition::Condition::of(&post, green, yellow);
        }
        state.unit = post;
    }

    /// **Whether `plan` writes anything the band cache is a function of** (16b
    /// cut M.3, 16c `T-28`) — [`Arena::set_unit`]'s `touched_bands`, decided the
    /// same way at both of its call sites.
    ///
    /// `Condition::of` reads three things per section and nothing else: the
    /// section's `internals` cell, every facet's `armor` cell, and the section's
    /// condition. So a plan touches the bands when it sets or adjusts a
    /// **facet** cell, sets or adjusts a **section** cell named `internals`, or
    /// sets a section's condition. A section cell of any other name — the `heat`
    /// cell, which is the only other section cell the arena writes — does not,
    /// and neither does anything at unit or component level. The impact's plan
    /// always answers `true` (a hit spends armour or internals or it has no ops
    /// and is never applied); the heat plan always answers `false`.
    pub(crate) fn plan_touches_bands(plan: &mush_tactics::UnitMutationPlan) -> bool {
        use mush_tactics::{CellAddr, UnitMutationOp};
        plan.ops().iter().any(|op| match op {
            UnitMutationOp::SetCell { addr, .. } | UnitMutationOp::AdjustCell { addr, .. } => {
                match addr {
                    CellAddr::Facet { .. } => true,
                    CellAddr::Section { cell, .. } => {
                        cell.as_str() == crate::definitions::unit::INTERNALS_CELL
                    }
                    CellAddr::Unit { .. } | CellAddr::Component { .. } => false,
                }
            }
            UnitMutationOp::SetSectionCondition { .. } => true,
            _ => false,
        })
    }

    /// **Take one pilot off the frame roster** (16b cut F.1; doc 16 §3.6).
    ///
    /// The roster is what `Arena::frames` walks, so a pilot dropped from it is
    /// built no further frame and told no further notice. Keyed on the **pair**:
    /// a pilot who left `unit` is not a co-pilot still sitting in it, and a tank
    /// with two seats loses only the seat that went.
    ///
    /// The remembered frame goes with the row, so a pilot the world later seats
    /// again is sent a whole frame rather than compared against one from a life
    /// they no longer have. Nothing else in the arena reads the roster — it is not
    /// folded into [`crate::arena::hash::state_hash`], which is what keeps this
    /// invisible to the cross-tempo property — so this is the whole of the drop.
    ///
    /// Idempotent. Answers whether a row went.
    pub fn drop_pilot(&mut self, pilot: ObjId, unit: ObjId) -> bool {
        let before = self.pilots.len();
        self.pilots
            .retain(|seat| !(seat.pilot == pilot && seat.unit == unit));
        let dropped = self.pilots.len() != before;
        if dropped && !self.pilots.iter().any(|seat| seat.pilot == pilot) {
            self.last_frames.remove(&pilot);
            // 16b cut Y.4: and a catch-up report no frame carried yet — a
            // re-seat and a drop in one pulse — is not owed to anybody.
            self.pending_catchup.remove(&pilot);
        }
        dropped
    }

    /// **Put one pilot back on the frame roster** (16b cut Y.3; 16c `N4`; doc 07
    /// §1.3 "Reclaiming control") — the undo of [`Arena::drop_pilot`], called by
    /// `apply_control` on a `Live` that names a pilot, before the tank is handed
    /// back to `Controller::Pilot`.
    ///
    /// Pushes `Pilot { unit, pilot, station: "", epoch }` when no `(pilot,
    /// unit)` row exists: the station is unknown to the arena after check-out
    /// (see [`Pilot::station`]). `drop_pilot` forgot the pilot's last frame with
    /// the row, so the next `Arena::frames` sends a whole keyframe, which is what
    /// a pilot back in the pod needs. Like `drop_pilot`, the roster is not folded
    /// into [`crate::arena::hash::state_hash`], so this is invisible to the
    /// cross-tempo property.
    ///
    /// Idempotent. Answers whether a row was added — `true` exactly when this
    /// `Live` re-seated a pilot the arena had dropped (a reclaim past the grace),
    /// and `false` for a revival inside it or a pilot who never left.
    pub fn seat_pilot(&mut self, pilot: ObjId, unit: ObjId, epoch: u64) -> bool {
        if self
            .pilots
            .iter()
            .any(|seat| seat.pilot == pilot && seat.unit == unit)
        {
            return false;
        }
        self.pilots.push(Pilot {
            unit,
            pilot,
            station: SmolStr::new_static(""),
            epoch,
        });
        true
    }

    /// **The hash one discrete action's ledger line carries** (16b cut W.1):
    /// the one `Arena::action_hash` holds while the pulse is inside phase 2's
    /// intent loop (16b cut Y.1, the `ordered` lines) or phase 4 (or, until
    /// 17b cut M4.6, a substep's reflex ticks), and a fresh
    /// [`crate::arena::hash::state_hash`] outside them.
    ///
    /// Every resolver of a discrete action goes through this rather than hashing for
    /// itself, so the phase's lines share one hash exactly as a substep's do, and a
    /// caller outside the pulse still gets an honest one.
    ///
    /// **16b cut M.3** (16c `T-23`): `&mut self`, because it is where the lazy
    /// slot is filled — `ActionHash::Taken` answers the stored hash,
    /// `ActionHash::Pending` takes the hash now, stores it and answers it, and
    /// `ActionHash::Outside` takes a fresh one and stores nothing. The hash of a
    /// run is therefore the state at its **first** journalled line — every
    /// resolver asks before it writes, so it is the state before that line's own
    /// action, after any refused ones before it (which write nothing hashed) —
    /// and every later line
    /// of the run shares it: one moment, one hash, the rule the substep follows,
    /// and a function of the actions resolved and not of the cadence.
    pub fn event_hash(&mut self) -> u64 {
        match self.action_hash {
            ActionHash::Taken(hash) => hash,
            ActionHash::Pending => {
                let hash = crate::arena::hash::state_hash(self);
                self.action_hash = ActionHash::Taken(hash);
                hash
            }
            ActionHash::Outside => crate::arena::hash::state_hash(self),
        }
    }

    /// Ask the arena to stop. **Phase 1 of the next step honours it**; nothing
    /// here ends the match, so a `Stop` never lands in the middle of a substep.
    ///
    /// The first reason wins: a `Void` that follows an `Abort` does not rewrite
    /// history, and an `Abort` that follows a `Void` cannot make a voided match
    /// committable.
    pub fn stop(&mut self, reason: StopReason) {
        if self.stop.is_none() {
            self.stop = Some(reason);
        }
    }

    /// **The speed ceiling of the match**, cm/s — the divisor of the substep
    /// length (doc 16 §3.1 "Rate independence";
    /// [`crate::arena::clock::substep_ms`]). Zero when nothing can move.
    ///
    /// A wreck and a unit whose motion is disabled are excluded: a substep sized
    /// for a speed no unit can reach costs every pulse a needless subdivision.
    ///
    /// **The terrain factor's ceiling since 16b cut M.4** (16c `T-15`):
    /// `fastest × 100 / min(100, min_move_cost)`. Motion's cap is
    /// `max_speed_cms × 100 / move_cost`, which on a cell cheaper than 100 sits
    /// above the authored speed, and the one-cell bound used to rest on every
    /// cost being at least 100. **No unit reaches that cap today**: every order
    /// is clamped to the authored `max_speed_cms` (`apply_intent`, seat and
    /// machine alike) and `accelerate` drives toward the order, so the authored
    /// speed is the real top speed. The scale is a **conservative margin**: it
    /// keeps the one-cell bound true of the terrain factor itself, should a later
    /// order path ever let the factor lift a unit past its authored figure.
    /// [`Arena::new`] refuses a match whose ceiling beats the substep floor
    /// ([`ArenaError::SubstepBound`]).
    ///
    /// **16b cut AE.1** (16c `B-58`) made it per unit on the unit's own row:
    /// the largest `max_speed_cms × 100 / min(100, min_cost[row])` over the
    /// units ([`Arena::min_cost`]). On the shipped ground the tracked and
    /// legged rows' cheapest passable price is 100, so a tank or a walker
    /// answers its authored speed; the `wheeled` row prices a road at 80, so a
    /// 3,000 cm/s car answers 3,750 on the proving ground, and the substep,
    /// `min(250, 999,000 / 3,750)`, is still 250 ms.
    #[must_use]
    pub fn max_speed_cms(&self) -> u32 {
        speed_ceiling_cms(&self.units, &self.min_cost)
    }

    /// The side of one cell, centimetres.
    #[must_use]
    pub fn cell_cm(&self) -> u16 {
        self.definition.terrain.cell_cm()
    }

    /// A `position_summary` payload over every unit, in unit-id order (doc 16
    /// §4): `(unit, x_cm, y_cm, z_cm, heading_mdeg)`.
    #[must_use]
    pub fn position_summary(&self) -> Vec<(ObjId, i64, i64, i32, u32)> {
        self.units
            .iter()
            .map(|state| {
                (
                    state.id,
                    state.pose.x_cm,
                    state.pose.y_cm,
                    state.pose.z_cm,
                    state.pose.heading_mdeg,
                )
            })
            .collect()
    }

    /// **Any unit holds a classified hostile inside its own weapons reach** — doc
    /// 16 §3.1's third `Engaged` trigger, which cut W.1 left `false` because
    /// nobody had bounded a field-wide sweep.
    ///
    /// There is no sweep: it reads the **contacts a platform already holds**, which
    /// the sensor pass minted and bounded, so the cost is one range test per held
    /// classified contact and never a pass over pairs of units. A platform with no
    /// weapon has no reach and answers `false` for every contact it holds, which is
    /// the honest reading of *within weapons range*.
    ///
    /// **16b cut F.3**: the range is to the contact's **plot** — where the
    /// platform last saw the target — for the same reason
    /// [`Arena::contact_within_alert_range`]'s is. How often a pilot is sent a
    /// frame is something a pilot can count, so a tempo read off a position
    /// nobody holds would be a leak of hidden truth.
    ///
    /// **16b cut E.1 (16c `T-32`)**: the reach is
    /// [`UnitState::fight_inner_cm`] — the band the platform would actually
    /// fight in — and not the longest gun it carries. Against the 3 km cannon
    /// this answered `true` for *any* classified hostile a sight or a mesh hop
    /// could hold, so `Engaged` meant "somebody holds a classified hostile" and
    /// the arena ran at 500 ms from the first classification to the end of the
    /// match. It now means what doc 16 §3.1's table says: hostiles are within
    /// weapons range of each other.
    ///
    /// **16b cut E.2 (16c `B-45`)**: and *hostile* is read off the contact,
    /// through [`Arena::contact_reads_dead`], rather than off the target's live
    /// wreck flag. The tempo now falls when the platforms holding the wreck
    /// **learn** it — at the kill for whoever fired the round, at the track's
    /// expiry for everybody else — and not at the arena millisecond the hull
    /// burned. A contact whose unit has left the match still counts for nothing.
    ///
    /// **17b cut M4.6a**: here, out of the reflex controller's module — it
    /// reads the contacts and the units, no reflex state.
    #[must_use]
    pub fn hostile_in_weapons_range(&self) -> bool {
        let cell_cm = self.definition.terrain.cell_cm();
        self.units.iter().any(|viewer| {
            let reach = viewer.fight_inner_cm(cell_cm);
            reach != 0
                && self.contacts.of(viewer.id).any(|(_, held)| {
                    held.class.team().is_some_and(|team| team != viewer.team)
                        && self.unit(held.target).is_some()
                        && !self.contact_reads_dead(held)
                        // 16b cut U.1 (16c `T-24`): the squared test, no root.
                        && held.seen_at.within_cm(&viewer.pose, reach)
                })
        })
    }

    /// **Armour plus internals, per mille of what the unit was authored with**
    /// (reconciliation (c) of the cut brief), read off the M32 cells on
    /// `UnitState::unit`.
    ///
    /// Every `armor` and `internals` cell on the unit, current against maximum,
    /// each measured from its own floor: a facet whose armour is at `min` counts
    /// zero and a section at full counts its whole span. A unit with no such cell
    /// at all — cut A's fixtures — reads `1000`, a unit in perfect health, which is
    /// the only answer that does not make an unmodelled hull withdraw at once.
    ///
    /// **17b cut M4.6a**: the reflex's `reflex_health_permille`, renamed and
    /// moved here; the squad signal reads it, and the stock mind's
    /// [`crate::arena::stock_mind::health_permille`] is the same reading over
    /// the view.
    #[must_use]
    pub fn health_permille(&self, id: ObjId) -> u32 {
        let Some(unit) = self.unit(id) else {
            return 0;
        };
        if unit.limits.wreck {
            return 0;
        }
        let mut have: i64 = 0;
        let mut full: i64 = 0;
        for (addr, cell) in unit.unit.cells() {
            let name = match addr {
                mush_tactics::CellAddr::Facet { cell, .. }
                | mush_tactics::CellAddr::Section { cell, .. } => cell.as_str(),
                mush_tactics::CellAddr::Unit { .. } | mush_tactics::CellAddr::Component { .. } => {
                    continue;
                }
            };
            if name != crate::definitions::unit::ARMOR_CELL
                && name != crate::definitions::unit::INTERNALS_CELL
            {
                continue;
            }
            have = have.saturating_add(cell.current.saturating_sub(cell.min).max(0));
            full = full.saturating_add(cell.max.saturating_sub(cell.min).max(0));
        }
        if full <= 0 {
            return 1_000;
        }
        u32::try_from(have.saturating_mul(1_000) / full)
            .unwrap_or(1_000)
            .min(1_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::test_support::{arena_of, custody, ground, pose, tank};

    /// **17b cut L.2**: a machine is a unit mind, and never a pilot — what
    /// both `squads_of` callers deal squads by. (`l2_is_machine_is_reflex_or_mind`
    /// until 17b cut M4.6 deleted the reflex controller and its line here.)
    #[test]
    fn l2_is_machine_is_a_mind_and_never_a_pilot() {
        assert!(!Controller::Pilot.is_machine());
        assert!(Controller::Mind.is_machine());
    }

    /// **16c `T-107b`**: [`Arena::unit_row`] is the verified read with the
    /// search as its fallback, and [`Arena::unit_by_row`] is its one body — a
    /// true row answers the element at that row, which is [`Arena::unit`]'s;
    /// a row past the table, the no-hint `u32::MAX`, or one naming another
    /// unit answers [`Arena::unit`]; and a candidate for a unit the table does
    /// not hold is `None` whatever its row. Dropping the id check or the
    /// fallback fails here.
    #[test]
    fn t107b_unit_row_reads_the_row_and_falls_back_to_the_search() {
        use crate::arena::index::Candidate;
        let arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(500, 500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 500)),
            tank(ObjId(11), Team::Blue, pose(7_500, 7_500)),
        ]);
        let candidate = |id: ObjId, row: u32| Candidate {
            id,
            row,
            cell: (0, 0),
        };
        let is = |read: Option<&UnitState>, unit: &UnitState| {
            read.is_some_and(|read| std::ptr::eq(read, unit))
        };
        let rows = u32::try_from(arena.units.len()).unwrap_or(u32::MAX);
        for (n, unit) in arena.units.iter().enumerate() {
            let row = u32::try_from(n).unwrap_or(u32::MAX);
            // The true row: the element at that row, and `unit`'s.
            let read = arena.unit_row(&candidate(unit.id, row));
            assert!(is(read, unit), "the true row of {:?}", unit.id);
            assert!(
                read.zip(arena.unit(unit.id))
                    .is_some_and(|(read, searched)| std::ptr::eq(read, searched))
            );
            // No hint at all, one past the table, and far past it: the search.
            for past in [u32::MAX, rows, rows.saturating_add(1_000)] {
                assert!(
                    is(arena.unit_row(&candidate(unit.id, past)), unit),
                    "row {past} for {:?} did not fall back to the search",
                    unit.id
                );
            }
            // Every other unit's row: the id check refuses it and the search
            // answers.
            for (wrong, other) in arena.units.iter().enumerate() {
                if wrong == n {
                    continue;
                }
                let read = arena.unit_row(&candidate(unit.id, u32::try_from(wrong).unwrap_or(0)));
                assert!(
                    is(read, unit),
                    "row {wrong} of {:?} read as {:?}",
                    other.id,
                    unit.id
                );
                assert!(!is(read, other));
            }
        }
        // A unit the table does not hold is `None` at every row.
        for row in [0, 1, 2, 3, u32::MAX] {
            assert!(arena.unit_row(&candidate(ObjId(20), row)).is_none());
        }
        // And the one body, over a slice position: the row, the wrong row,
        // and no row.
        assert!(
            arena
                .unit_by_row(1, ObjId(9))
                .is_some_and(|unit| unit.id == ObjId(9))
        );
        assert!(
            arena
                .unit_by_row(0, ObjId(9))
                .is_some_and(|unit| unit.id == ObjId(9))
        );
        assert!(
            arena
                .unit_by_row(usize::MAX, ObjId(9))
                .is_some_and(|unit| unit.id == ObjId(9))
        );
        assert!(arena.unit_by_row(1, ObjId(20)).is_none());
        // The production candidate is the true row: what the index hands the
        // passes over `Arena::units`.
        for c in arena
            .index
            .in_range_rows(&pose(500, 500), 100_000, &arena.units)
        {
            assert_eq!(arena.unit_at(c.id), usize::try_from(c.row).ok());
            assert!(arena.unit_row(&c).is_some_and(|unit| unit.id == c.id));
        }
    }

    /// **16c `T-107d`**: [`ArenaTerrain::is_cell_at`] is `cell_at`'s own
    /// answer restated — `is_cell_at(cell, x, y)` is `cell_at(x, y) ==
    /// Some(cell)` for every cell and every pair of coordinates — on the unit
    /// tests' 8 by 8 window of 1,000 cm cells and on a 12 by 5 window of 700
    /// cm cells (a width/height slip, or a side baked in, fails on the
    /// second). The coordinates straddle every boundary the cut's brief names
    /// and 700's, the far edge of each window, the sign, the `u16` quotient
    /// bound (65,535,000 and 65,536,000) and both ends of `i64`; the cells are
    /// every cell of a 13 by 13 box over the windows (both windows' cells, and
    /// a column and a row past each), `(99, 99)` and the three `u16::MAX`
    /// corners. 183,034 comparisons, and the agreements counted so the
    /// on-window pairs are known to have found their one cell.
    #[test]
    fn t107d_is_cell_at_is_cell_at_s_own_answer() {
        let windows = [
            ArenaTerrain::uniform(8, 8, 1_000, ground()).expect("the 8 by 8 window"),
            ArenaTerrain::uniform(12, 5, 700, ground()).expect("a 12 by 5 window"),
        ];
        const COORDINATES: [i64; 23] = [
            i64::MIN,
            -65_536_000,
            -1,
            0,
            1,
            699,
            700,
            701,
            999,
            1_000,
            1_001,
            3_499,
            3_500,
            3_501,
            7_999,
            8_000,
            8_001,
            8_399,
            8_400,
            8_401,
            65_535_000,
            65_536_000,
            i64::MAX,
        ];
        let mut cells: Vec<(u16, u16)> = (0..13_u16)
            .flat_map(|x| (0..13_u16).map(move |y| (x, y)))
            .collect();
        cells.extend([(99, 99), (u16::MAX, 0), (0, u16::MAX), (u16::MAX, u16::MAX)]);
        let mut compared = 0_u32;
        let mut agreed = 0_u32;
        for terrain in &windows {
            let (width, height) = (terrain.width(), terrain.height());
            for x in COORDINATES {
                for y in COORDINATES {
                    let answer = terrain.cell_at(x, y);
                    for &cell in &cells {
                        let expected = answer == Some(cell);
                        assert_eq!(
                            terrain.is_cell_at(cell, x, y),
                            expected,
                            "{width} by {height}: is {cell:?} the cell at ({x}, {y})? cell_at says {answer:?}"
                        );
                        compared += 1;
                        if expected {
                            agreed += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(compared, 2 * 23 * 23 * 173);
        // The on-window pairs, each finding its one cell (every window cell is
        // in the box): on the 8 by 8 window the on-window coordinates are {0,
        // 1, 699, 700, 701, 999, 1,000, 1,001, 3,499, 3,500, 3,501, 7,999} (12
        // an axis, 144 pairs); on the 12 by 5 the x axis has those twelve and
        // 8,000, 8,001 and 8,399 (15; the last four in column 11), the y axis
        // those up to 3,499 (9): 135 pairs.
        assert_eq!(agreed, 144 + 135);
        // By hand: the boundary belongs to the cell it opens, on both windows,
        // and the sign and the far edge are off.
        let [eight, twelve] = &windows;
        assert!(eight.is_cell_at((0, 0), 999, 999));
        assert!(eight.is_cell_at((1, 0), 1_000, 999));
        assert!(!eight.is_cell_at((0, 0), 1_000, 0));
        assert!(eight.is_cell_at((7, 7), 7_999, 7_999));
        assert!(!eight.is_cell_at((8, 7), 8_000, 7_999));
        assert!(!eight.is_cell_at((7, 7), 8_000, 7_999));
        assert!(!eight.is_cell_at((0, 0), -1, 0));
        assert!(!eight.is_cell_at((0, 0), 0, -1));
        assert!(twelve.is_cell_at((11, 4), 8_399, 3_499));
        assert!(!twelve.is_cell_at((11, 4), 8_400, 3_499));
        assert!(!twelve.is_cell_at((11, 4), 8_399, 3_500));
        assert!(!twelve.is_cell_at((4, 11), 3_499, 8_399));
    }

    /// 17b cut M4.6a: moved here from `crate::arena::reflex`'s tests with
    /// [`Arena::health_permille`] (the reflex's `reflex_health_permille`).
    #[test]
    fn an_unmodelled_hull_reads_as_whole_and_a_wreck_reads_as_gone() {
        let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        // Cut A's fixture carries one armour cell and one internals cell, both full.
        assert_eq!(arena.health_permille(ObjId(4)), 1_000);
        assert_eq!(arena.health_permille(ObjId(9)), 0, "no such unit");
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.limits.wreck = true;
        }
        assert_eq!(arena.health_permille(ObjId(4)), 0);
    }

    /// **16b cut Q.1**: the letters are the whole of what keeps a provisional
    /// designator and a slot designator apart, so they have to be pairwise
    /// distinct and none of them may be [`PROVISIONAL_PREFIX`]. A third side is a
    /// failing test here with the fix beside it.
    #[test]
    fn every_team_letter_is_distinct_and_none_of_them_is_the_provisional_prefix() {
        let letters: Vec<char> = Team::ALL.iter().map(|team| team.letter()).collect();
        for (at, letter) in letters.iter().enumerate() {
            assert_ne!(
                *letter,
                PROVISIONAL_PREFIX,
                "{:?} spells its callsigns with the provisional prefix",
                Team::ALL.get(at)
            );
            assert!(
                letters.iter().skip(at + 1).all(|other| other != letter),
                "two sides share the letter {letter}"
            );
        }
    }

    /// The spelling of a designator, at each of the three shapes that matter: the
    /// provisional number, a slot inside the alphabet, and the first slot past it.
    #[test]
    fn a_designator_spells_a_number_a_letter_and_the_roll_past_z() {
        assert_eq!(Designator::Provisional(1).to_string(), "X1");
        assert_eq!(Designator::Provisional(12).to_string(), "X12");
        for (slot, spelling) in [
            (1_u16, "RA"),
            (2, "RB"),
            (12, "RL"),
            (26, "RZ"),
            (27, "RAA"),
            (28, "RAB"),
            (52, "RAZ"),
            (53, "RBA"),
        ] {
            assert_eq!(
                Designator::Slot {
                    team: Team::Red,
                    slot,
                }
                .to_string(),
                spelling
            );
        }
        assert_eq!(
            Designator::Slot {
                team: Team::Blue,
                slot: 3,
            }
            .to_string(),
            "BC"
        );
        // A slot of zero is a name nothing resolves, not a panic: `Arena::new`
        // never produces one.
        assert_eq!(
            Designator::Slot {
                team: Team::Blue,
                slot: 0,
            }
            .to_string(),
            "B"
        );
    }

    /// **16b cut Q.1**: a number is the viewer's own, issued in first-seen order,
    /// never reused, and kept for the target it was issued for.
    #[test]
    fn the_book_numbers_per_viewer_in_first_seen_order_and_never_reuses_one() {
        let mut book = DesignatorBook::default();
        assert_eq!(book.provisional(ObjId(1), ObjId(9)), None);
        assert_eq!(book.assign(ObjId(1), ObjId(9)), 1);
        assert_eq!(book.assign(ObjId(1), ObjId(4)), 2);
        // The same pair answers the same number for ever.
        assert_eq!(book.assign(ObjId(1), ObjId(9)), 1);
        assert_eq!(book.provisional(ObjId(1), ObjId(9)), Some(1));
        // A second viewer counts from one of its own.
        assert_eq!(book.assign(ObjId(2), ObjId(4)), 1);
        assert_eq!(book.provisional(ObjId(2), ObjId(9)), None);
        // The walks the hash folds, in map order.
        assert_eq!(
            book.counters().collect::<Vec<(ObjId, u16)>>(),
            vec![(ObjId(1), 3), (ObjId(2), 2)]
        );
        assert_eq!(
            book.issued().collect::<Vec<(ObjId, ObjId, u16)>>(),
            vec![
                (ObjId(1), ObjId(4), 2),
                (ObjId(1), ObjId(9), 1),
                (ObjId(2), ObjId(4), 1),
            ]
        );
    }

    #[test]
    fn terrain_refuses_a_grid_that_is_not_its_dimensions() {
        let short = ArenaTerrain::new(4, 4, 1_000, vec![ground(); 15]);
        assert!(matches!(
            short,
            Err(ArenaError::TerrainShape { cells: 15, .. })
        ));
        let zero_cell = ArenaTerrain::uniform(4, 4, 0, ground());
        assert!(matches!(
            zero_cell,
            Err(ArenaError::TerrainShape { cell_cm: 0, .. })
        ));
        let empty = ArenaTerrain::uniform(0, 4, 1_000, ground());
        assert!(matches!(
            empty,
            Err(ArenaError::TerrainShape { width: 0, .. })
        ));
        let ok = ArenaTerrain::uniform(4, 4, 1_000, ground()).expect("a 4x4 window");
        assert_eq!(ok.cells().len(), 16);
    }

    #[test]
    fn cell_of_floors_and_answers_none_off_the_window() {
        let terrain = ArenaTerrain::uniform(4, 3, 1_000, ground()).expect("a 4x3 window");
        // The origin, and the last centimetre of the first cell.
        assert_eq!(terrain.cell_of(&pose(0, 0)), Some((0, 0)));
        assert_eq!(terrain.cell_of(&pose(999, 999)), Some((0, 0)));
        // The boundary belongs to the cell it opens.
        assert_eq!(terrain.cell_of(&pose(1_000, 0)), Some((1, 0)));
        assert_eq!(terrain.cell_of(&pose(0, 1_000)), Some((0, 1)));
        // The far corner, and one centimetre past it on each axis.
        assert_eq!(terrain.cell_of(&pose(3_999, 2_999)), Some((3, 2)));
        assert_eq!(terrain.cell_of(&pose(4_000, 2_999)), None);
        assert_eq!(terrain.cell_of(&pose(3_999, 3_000)), None);
        // Negative is off the window, never folded toward zero.
        assert_eq!(terrain.cell_of(&pose(-1, 0)), None);
        assert_eq!(terrain.cell_of(&pose(0, -1)), None);
        assert_eq!(terrain.cell_of(&pose(-1_000, -1_000)), None);
        // And `get` agrees with the window's bounds.
        assert!(terrain.get(3, 2).is_some());
        assert!(terrain.get(4, 2).is_none());
        assert!(terrain.get(3, 3).is_none());
    }

    #[test]
    fn the_epoch_table_never_lowers_and_authorises_only_the_current_epoch() {
        let unit = ObjId(7);
        let mut epochs = EpochTable::new();
        assert_eq!(epochs.current(unit, Channel::Helm), 0);
        assert!(epochs.authorises(unit, Channel::Helm, 0));
        assert!(epochs.raise(unit, Channel::Helm, 3));
        assert_eq!(epochs.current(unit, Channel::Helm), 3);
        // A lower or equal epoch is a no-op.
        assert!(!epochs.raise(unit, Channel::Helm, 2));
        assert!(!epochs.raise(unit, Channel::Helm, 3));
        assert_eq!(epochs.current(unit, Channel::Helm), 3);
        // Only the exact current epoch authorises: stale and future both refuse.
        assert!(epochs.authorises(unit, Channel::Helm, 3));
        assert!(!epochs.authorises(unit, Channel::Helm, 2));
        assert!(!epochs.authorises(unit, Channel::Helm, 4));
        // Channels are independent, and so are units.
        assert_eq!(epochs.current(unit, Channel::Fire), 0);
        assert_eq!(epochs.current(ObjId(8), Channel::Helm), 0);
    }

    #[test]
    fn the_due_heap_is_a_min_heap_by_timestamp_then_sequence() {
        let mut due = DueHeap::new();
        let late = due.schedule(500, DueKind::PositionSummary);
        let early = due.schedule(100, DueKind::StartupComplete { unit: ObjId(1) });
        let tie = due.schedule(100, DueKind::StartupComplete { unit: ObjId(2) });
        assert_eq!((late, early, tie), (1, 2, 3));
        assert_eq!(due.len(), 3);
        assert_eq!(due.next_t_ms(), Some(100));
        // Nothing before its time.
        assert!(due.peek_due(99).is_none());
        assert!(due.pop_due(99).is_none());
        // Timestamp order first, then insertion sequence.
        assert_eq!(due.pop_due(100).map(|event| event.seq), Some(early));
        assert_eq!(due.pop_due(100).map(|event| event.seq), Some(tie));
        assert!(due.pop_due(100).is_none());
        assert_eq!(due.pop_due(i64::MAX).map(|event| event.seq), Some(late));
        assert!(due.is_empty());
    }

    #[test]
    fn opening_a_match_sorts_units_schedules_the_summary_and_journals_the_start() {
        let arena = arena_of(vec![
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
        ]);
        assert_eq!(
            arena.units.iter().map(|state| state.id).collect::<Vec<_>>(),
            vec![ObjId(4), ObjId(9)]
        );
        assert_eq!(arena.now_ms, 0);
        assert!(arena.ended.is_none());
        assert!(arena.stop.is_none());
        // Every unit starts cold, stationary, and ordered to hold its heading.
        for state in &arena.units {
            assert_eq!(state.power, Power::Off);
            assert_eq!(state.speed_cms, 0);
            assert_eq!(state.desired_speed_cms, 0);
            assert_eq!(state.desired_heading_mdeg, state.pose.heading_mdeg);
            assert_eq!(state.limits, MotionLimits::default());
            assert!(!state.linkdead);
            assert!(state.ready_at_ms.is_empty());
            assert!(state.lock.is_none());
        }
        // Three scheduled events: the first position summary at
        // `ledger.summary_ms`, and — **16b cut F.3** — one sensor pass per unit,
        // staggered at `stagger_ms(seed, id, pass_ms)` since 16b cut U.1 (it was
        // `id % pass_ms`). The pass is where a platform's contacts expire, so
        // every platform has one whether or not it carries a sight.
        // **16b cut M.2** (16c `T-01`) adds the fourth: the duration bound, at
        // `match.max_duration_ms`, scheduled second.
        assert_eq!(arena.due.len(), 4);
        let first = [ObjId(4), ObjId(9)]
            .iter()
            .map(|id| crate::arena::rng::stagger_ms(arena.seed, *id, 1_000))
            .min();
        assert_eq!(
            arena.due.next_t_ms(),
            first,
            "the nearer unit's own stagger"
        );
        assert_eq!(
            arena
                .due
                .iter()
                .filter(|event| event.kind == DueKind::MatchDeadline)
                .map(|event| (event.t_ms, event.seq))
                .collect::<Vec<_>>(),
            vec![(i64::from(MatchConfig::default().max_duration_ms), 2)],
            "one deadline, at the bound, second in the schedule"
        );
        // And the opening lines: `match_started`, sequence 1, and since 17b
        // cut M3.6 one `mind_sealed` per unit after it (was `seq() == 1`).
        assert_eq!(
            arena.ledger.seq(),
            1 + u64::try_from(arena.units.len()).unwrap_or(0),
            "`match_started` and one `mind_sealed` per unit"
        );
        assert!(arena.ledger.bytes_total() > 0);
    }

    #[test]
    fn opening_a_match_refuses_a_pose_off_the_window_a_duplicate_and_a_stray_pilot() {
        let off = Arena::new(
            custody(
                vec![tank(ObjId(4), Team::Red, pose(99_000, 1_500))],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            off,
            Err(ArenaError::PoseOffWindow { unit: ObjId(4), .. })
        ));

        let twice = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                    tank(ObjId(4), Team::Blue, pose(2_500, 2_500)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            twice,
            Err(ArenaError::DuplicateUnit { unit: ObjId(4) })
        ));

        let stray = Arena::new(
            custody(
                vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))],
                vec![Pilot {
                    unit: ObjId(5),
                    pilot: ObjId(100),
                    station: SmolStr::new("cockpit"),
                    epoch: 1,
                }],
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            stray,
            Err(ArenaError::UnknownPilotUnit { unit: ObjId(5), .. })
        ));

        let nothing = Arena::new(
            custody(Vec::new(), Vec::new()),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(nothing, Err(ArenaError::NoUnits)));
    }

    #[test]
    fn opening_a_match_refuses_two_units_in_one_cell() {
        // Doc 16 §3.4 step 5: occupancy is one unit per cell. Two poses in cell
        // (2, 2) — different centimetres, one cell — and the refusal names the
        // cell and the unit already in it (the lower id, because it is the one a
        // check-out would have placed first).
        let shared = Arena::new(
            custody(
                vec![
                    tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
                    tank(ObjId(4), Team::Red, pose(2_100, 2_900)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(
            matches!(
                shared,
                Err(ArenaError::CellOccupied {
                    unit: ObjId(9),
                    other: ObjId(4),
                    x: 2,
                    y: 2
                })
            ),
            "{shared:?}"
        );
        // Adjacent cells are fine, and so is the cell boundary: 2,999 is column 2
        // and 3,000 is column 3.
        let apart = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(2_999, 2_500)),
                    tank(ObjId(9), Team::Blue, pose(3_000, 2_500)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(apart.is_ok(), "{apart:?}");
    }

    #[test]
    fn a_seat_raises_its_channels_and_a_named_station_raises_only_its_own() {
        let arena = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                    tank(ObjId(5), Team::Blue, pose(2_500, 2_500)),
                ],
                vec![
                    Pilot {
                        unit: ObjId(4),
                        pilot: ObjId(100),
                        station: SmolStr::new("cockpit"),
                        epoch: 4,
                    },
                    Pilot {
                        unit: ObjId(5),
                        pilot: ObjId(101),
                        station: SmolStr::new("fire"),
                        epoch: 7,
                    },
                ],
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("two tanks and two seats");
        for channel in Channel::ALL {
            assert_eq!(arena.epochs.current(ObjId(4), channel), 4);
        }
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Fire), 7);
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Helm), 0);
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Sensors), 0);
    }

    #[test]
    fn unit_lookup_is_by_id_and_stop_keeps_the_first_reason() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
        ]);
        assert_eq!(
            arena.unit(ObjId(9)).map(|state| state.team),
            Some(Team::Blue)
        );
        assert!(arena.unit(ObjId(5)).is_none());
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.speed_cms = 42;
        }
        assert_eq!(arena.unit(ObjId(4)).map(|state| state.speed_cms), Some(42));
        arena.stop(StopReason::Abort);
        arena.stop(StopReason::Void);
        assert_eq!(arena.stop, Some(StopReason::Abort));
    }

    #[test]
    fn the_definition_digest_is_stable_and_separates_every_field() {
        let base = ArenaDefinition {
            id: SmolStr::new("tacsim.arena.proving_ground"),
            terrain: ArenaTerrain::uniform(4, 4, 1_000, ground()).expect("a 4x4 window"),
            entry_zones: vec![EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 1,
            }],
            key_spots: vec![KeySpot {
                name: SmolStr::new("The Ford"),
                x: 1,
                y: 2,
            }],
            waypoints: Waypoints::new(
                vec![
                    Waypoint {
                        id: SmolStr::new("west"),
                        x: 0,
                        y: 2,
                    },
                    Waypoint {
                        id: SmolStr::new("east"),
                        x: 3,
                        y: 2,
                    },
                ],
                &[(SmolStr::new("west"), SmolStr::new("east"))],
            ),
            modes: Vec::new(),
        };
        assert_eq!(base.digest(), base.clone().digest());

        let mut renamed = base.clone();
        renamed.id = SmolStr::new("tacsim.arena.proving_grounds");
        assert_ne!(base.digest(), renamed.digest());

        let mut repainted = base.clone();
        let mut cells = repainted.terrain.cells().to_vec();
        if let Some(cell) = cells.get_mut(5) {
            cell.move_cost = 0;
        }
        repainted.terrain =
            ArenaTerrain::new(4, 4, 1_000, cells).expect("the same window, one cell impassable");
        assert_ne!(base.digest(), repainted.digest());

        let mut moved_zone = base.clone();
        moved_zone.entry_zones = vec![EntryZone {
            team: Team::Blue,
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        }];
        assert_ne!(base.digest(), moved_zone.digest());

        let mut resized = base.clone();
        resized.terrain = ArenaTerrain::uniform(2, 8, 1_000, ground()).expect("a 2x8 window");
        assert_ne!(base.digest(), resized.digest());

        // 16b cut N.1: the furniture is part of the definition's identity — two
        // windows with one terrain and two graphs are two arenas, because the
        // routes a replay reproduces are a function of the graph.
        let mut renamed_spot = base.clone();
        renamed_spot.key_spots = vec![KeySpot {
            name: SmolStr::new("The Crossing"),
            x: 1,
            y: 2,
        }];
        assert_ne!(base.digest(), renamed_spot.digest());

        let mut moved_spot = base.clone();
        moved_spot.key_spots = vec![KeySpot {
            name: SmolStr::new("The Ford"),
            x: 2,
            y: 2,
        }];
        assert_ne!(base.digest(), moved_spot.digest());

        let mut cut_edge = base.clone();
        cut_edge.waypoints = Waypoints::new(
            vec![
                Waypoint {
                    id: SmolStr::new("west"),
                    x: 0,
                    y: 2,
                },
                Waypoint {
                    id: SmolStr::new("east"),
                    x: 3,
                    y: 2,
                },
            ],
            &[],
        );
        assert_ne!(
            base.digest(),
            cut_edge.digest(),
            "the same nodes with no edge between them is a different graph"
        );
    }

    #[test]
    fn the_waypoint_graph_resolves_its_edges_and_drops_the_unresolvable() {
        let nodes = vec![
            Waypoint {
                id: SmolStr::new("a"),
                x: 0,
                y: 0,
            },
            Waypoint {
                id: SmolStr::new("b"),
                x: 4,
                y: 0,
            },
            Waypoint {
                id: SmolStr::new("c"),
                x: 4,
                y: 4,
            },
        ];
        let graph = Waypoints::new(
            nodes,
            &[
                (SmolStr::new("a"), SmolStr::new("b")),
                (SmolStr::new("b"), SmolStr::new("c")),
                // Declared twice, one of them backwards: one edge.
                (SmolStr::new("c"), SmolStr::new("b")),
                // A node nothing declares, and a self-edge: both dropped.
                (SmolStr::new("b"), SmolStr::new("ghost")),
                (SmolStr::new("a"), SmolStr::new("a")),
            ],
        );
        assert_eq!(graph.nodes().len(), 3);
        assert!(!graph.is_empty());
        assert_eq!(graph.neighbours(0), &[1]);
        assert_eq!(graph.neighbours(1), &[0, 2]);
        assert_eq!(graph.neighbours(2), &[1]);
        assert!(graph.neighbours(9).is_empty());
        assert_eq!(graph.node(2).map(|node| (node.x, node.y)), Some((4, 4)));
        // The nearest node, ties to the lower index.
        assert_eq!(graph.nearest(0, 1), Some(0));
        assert_eq!(graph.nearest(4, 3), Some(2));
        assert_eq!(graph.nearest(2, 0), Some(0));
        assert!(Waypoints::default().nearest(0, 0).is_none());
        assert!(Waypoints::default().is_empty());
    }

    /// A plot at `(x_cm, y_cm)`, facing north and standing still (16b cut F.3).
    fn plot(x_cm: i64, y_cm: i64) -> SeenPose {
        SeenPose {
            x_cm,
            y_cm,
            z_cm: 0,
            heading_mdeg: 0,
            speed_cms: 0,
        }
    }

    #[test]
    fn the_shadow_contact_store_holds_refreshes_and_expires() {
        let viewer = ObjId(4);
        let mut contacts = ShadowContacts::new();
        assert!(contacts.is_empty());
        let first = ContactId(0xdead_beef);
        let second = ContactId(0x0bad_f00d);
        contacts.insert(
            viewer,
            first,
            ShadowContact::sighted(ObjId(9), Classification::Unknown, 1_000, plot(0, 0)),
        );
        contacts.insert(
            viewer,
            second,
            ShadowContact {
                last_seen_ms: 4_000,
                ..ShadowContact::sighted(
                    ObjId(10),
                    Classification::Classified { team: Team::Blue },
                    1_000,
                    plot(0, 0),
                )
            },
        );
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts.viewers(), 1);
        assert_eq!(
            contacts.get(viewer, first).map(|held| held.target),
            Some(ObjId(9))
        );
        assert_eq!(contacts.id_of(viewer, ObjId(10)), Some(second));
        assert!(contacts.id_of(viewer, ObjId(11)).is_none());
        // Another viewer's id is nobody else's.
        assert!(contacts.get(ObjId(5), first).is_none());
        if let Some(held) = contacts.get_mut(viewer, first) {
            held.last_seen_ms = 2_000;
        }
        // Expiry answers the losses and keeps the refreshed one.
        let lost = contacts.expire(viewer, 3_000);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost.first().map(|(id, _)| *id), Some(first));
        assert_eq!(contacts.len(), 1);
        contacts.clear_viewer(viewer);
        assert!(contacts.is_empty());
        assert_eq!(contacts.viewers(), 0);
    }

    #[test]
    fn channels_round_trip_their_words() {
        for channel in Channel::ALL {
            assert_eq!(Channel::parse(channel.as_str()), Some(channel));
        }
        assert!(Channel::parse("helms").is_none());
        assert!(Channel::parse("HELM").is_none());
        assert_eq!(Team::Red.other(), Team::Blue);
        assert_eq!(Side::Left.as_str(), "left");
        assert_eq!(Power::Online.code(), 2);
        assert_eq!(ContactId(0x1234).to_string(), "0000000000001234");
    }

    #[test]
    fn the_reverse_ceiling_is_a_percentage_of_the_forward_one() {
        let rules = TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 200,
            brake_cms2: 400,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        };
        assert_eq!(rules.reverse_ceiling_cms(), 400);
        assert_eq!(TrackedRules::default().reverse_ceiling_cms(), 0);
    }

    #[test]
    fn the_fastest_unit_ignores_wrecks_and_disabled_motion() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
        ]);
        assert_eq!(arena.max_speed_cms(), 1_000);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.limits.wreck = true;
        }
        if let Some(state) = arena.unit_mut(ObjId(9)) {
            state.mobility.max_speed_cms = 300;
        }
        assert_eq!(arena.max_speed_cms(), 300);
        if let Some(state) = arena.unit_mut(ObjId(9)) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(arena.max_speed_cms(), 0);
    }

    #[test]
    fn the_position_summary_is_every_unit_in_id_order() {
        let arena = arena_of(vec![
            tank(ObjId(9), Team::Blue, pose(2_500, 2_600)),
            tank(ObjId(4), Team::Red, pose(1_500, 1_600)),
        ]);
        assert_eq!(
            arena.position_summary(),
            vec![
                (ObjId(4), 1_500, 1_600, 0, 0),
                (ObjId(9), 2_500, 2_600, 0, 0)
            ]
        );
    }

    /// **16b cut M.3** (16c `T-27`): the book's digest starts at FNV-1a's offset
    /// basis, moves on a mint, stands still on a re-assignment of a pair the book
    /// already holds, and is a function of the mints in order.
    #[test]
    fn m3_the_book_digest_moves_on_a_mint_only_and_follows_the_order() {
        let mut book = DesignatorBook::default();
        assert_eq!(book.digest(), crate::arena::hash::FNV_OFFSET_BASIS);
        assert_eq!(book.assign(ObjId(4), ObjId(9)), 1);
        let minted = book.digest();
        assert_ne!(
            minted,
            crate::arena::hash::FNV_OFFSET_BASIS,
            "a mint moves it"
        );
        // A held pair: the same number, and nothing folded.
        assert_eq!(book.assign(ObjId(4), ObjId(9)), 1);
        assert_eq!(book.digest(), minted, "a re-assignment moved the digest");
        // Two books with the same mints in the same order agree.
        let book_of = |mints: &[(u32, u32)]| {
            let mut book = DesignatorBook::default();
            for (viewer, target) in mints {
                let _ = book.assign(ObjId(*viewer), ObjId(*target));
            }
            book
        };
        let one = book_of(&[(4, 9), (4, 10), (5, 9)]);
        let two = book_of(&[(4, 9), (4, 10), (5, 9)]);
        assert_eq!(one.digest(), two.digest());
        assert_eq!(one, two);
        // A different order disagrees — within one viewer, where the numbers
        // themselves differ, and across two viewers, where the final table is
        // the same and only the order of the mints tells them apart.
        let swapped = book_of(&[(4, 10), (4, 9), (5, 9)]);
        assert_ne!(swapped.digest(), one.digest());
        let interleaved = book_of(&[(5, 9), (4, 9), (4, 10)]);
        assert_eq!(
            interleaved.issued().collect::<Vec<_>>(),
            one.issued().collect::<Vec<_>>(),
            "the same numbers for the same pairs"
        );
        assert_ne!(interleaved.digest(), one.digest());
    }

    /// **16b cut M.3** (16c `T-28`): an impact's plan recomputes the band cache,
    /// a unit the arena does not hold is a no-op before anything is computed,
    /// and `plan_touches_bands` reads the ops the way `Condition::of` reads the
    /// unit.
    #[test]
    fn m3_set_unit_recomputes_the_bands_only_when_told_and_ignores_a_stranger() {
        use crate::arena::condition::{Condition, ConditionBand};
        use mush_tactics::{
            CellAddr, CellId, FacetId, SectionId, UnitMutationOp, UnitMutationPlan,
        };

        let mut arena = arena_of(vec![crate::arena::test_support::line_tank(
            ObjId(9),
            Team::Blue,
            pose(1_500, 1_500),
        )]);
        let honest = arena.unit(ObjId(9)).map(|unit| unit.condition);
        // A band cache no intact tank has, so a recompute is visible.
        let marked = Condition::default().with_band(0, ConditionBand::Destroyed);
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.condition = marked;
        }
        let same = arena
            .unit(ObjId(9))
            .map(|unit| unit.unit.clone())
            .unwrap_or_else(|| panic!("the tank"));
        // Told nothing was touched: left exactly as it was.
        arena.set_unit(ObjId(9), same.clone(), false);
        assert_eq!(
            arena.unit(ObjId(9)).map(|unit| unit.condition),
            Some(marked)
        );
        // Told the bands were touched: recomputed from the unit.
        arena.set_unit(ObjId(9), same.clone(), true);
        assert_eq!(arena.unit(ObjId(9)).map(|unit| unit.condition), honest);
        // A stranger: nothing moves.
        let before = crate::arena::hash::state_hash(&arena);
        arena.set_unit(ObjId(77), same, true);
        assert_eq!(crate::arena::hash::state_hash(&arena), before);
        assert_eq!(arena.units.len(), 1);

        let plan = |op: UnitMutationOp| {
            UnitMutationPlan::new(vec![op], None, None).unwrap_or_else(|_| panic!("a plan"))
        };
        let section = |cell: &str| CellAddr::Section {
            section: SectionId::new("hull").expect("a section"),
            cell: CellId::new(cell).expect("a cell"),
        };
        let armour = CellAddr::Facet {
            section: SectionId::new("hull").expect("a section"),
            facet: FacetId::new("front").expect("a facet"),
            cell: CellId::new("armor").expect("a cell"),
        };
        for (op, touches) in [
            (
                UnitMutationOp::SetCell {
                    addr: armour.clone(),
                    value: 1,
                },
                true,
            ),
            (
                UnitMutationOp::AdjustCell {
                    addr: armour,
                    delta: -1,
                },
                true,
            ),
            (
                UnitMutationOp::SetCell {
                    addr: section("internals"),
                    value: 1,
                },
                true,
            ),
            (
                UnitMutationOp::SetSectionCondition {
                    section: SectionId::new("hull").expect("a section"),
                    condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
                },
                true,
            ),
            // The heat cell is a section cell the bands never read.
            (
                UnitMutationOp::SetCell {
                    addr: section(crate::definitions::unit::HEAT_CELL),
                    value: 1,
                },
                false,
            ),
        ] {
            assert_eq!(
                Arena::plan_touches_bands(&plan(op.clone())),
                touches,
                "{op:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 16b cut M.4: the substep bound, the true ceiling and the opening cell
    // -----------------------------------------------------------------------

    /// The test window with `paint`ed cells and `units` handed over, opened or
    /// refused.
    fn open_on(
        units: Vec<CustodyUnit>,
        paint: &[((u16, u16), TerrainCell)],
    ) -> Result<Arena, ArenaError> {
        let mut out = custody(units, Vec::new());
        let (width, height) = (
            out.definition.terrain.width(),
            out.definition.terrain.height(),
        );
        let mut cells = out.definition.terrain.cells().to_vec();
        for ((x, y), cell) in paint {
            if let Some(slot) =
                cells.get_mut(usize::from(*y) * usize::from(width) + usize::from(*x))
            {
                *slot = *cell;
            }
        }
        out.definition.terrain =
            ArenaTerrain::new(width, height, 1_000, cells).expect("the painted window");
        Arena::new(
            out,
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
    }

    /// A stock fixture tank at `speed_cms`.
    fn fast(id: ObjId, team: Team, at: Pose, speed_cms: u32) -> CustodyUnit {
        let mut unit = tank(id, team, at);
        unit.mobility.max_speed_cms = speed_cms;
        unit
    }

    /// **16b cut M.4 (16c `T-15`): the substep is sized on the terrain factor's
    /// ceiling.** One passable cell at cost 80 raises motion's terrain cap a
    /// quarter above the authored speed (a cap the order clamp keeps any unit
    /// from reaching; the scale is a conservative margin), so `max_speed_cms`
    /// answers `fastest × 100 / 80` and the substep shortens with it; on uniform
    /// ground (every cost 100) nothing moves.
    #[test]
    fn m4_a_cell_cheaper_than_a_hundred_raises_the_ceiling_and_shortens_the_substep() {
        let red = || fast(ObjId(4), Team::Red, pose(1_500, 1_500), 5_000);
        let flat = open_on(vec![red()], &[]).expect("the flat window opens");
        assert_eq!(flat.min_cost[0], 100);
        assert_eq!(
            flat.max_speed_cms(),
            5_000,
            "the authored speed on cost 100"
        );
        let cheap = TerrainCell {
            move_cost: 80,
            ..ground()
        };
        let fast_ground = open_on(vec![red()], &[((5, 5), cheap)]).expect("it opens");
        assert_eq!(fast_ground.min_cost[0], 80);
        assert_eq!(fast_ground.max_speed_cms(), 6_250, "5,000 × 100 / 80");
        let tempo = crate::config::TempoConfig::default();
        let before = crate::arena::clock::substep_ms(&tempo, 1_000, flat.max_speed_cms());
        let after = crate::arena::clock::substep_ms(&tempo, 1_000, fast_ground.max_speed_cms());
        assert_eq!((before, after), (199, 159), "999,000 / 5,000 and / 6,250");
        // A cell dearer than 100 never lengthens it: the ceiling is capped at
        // the authored speed from below.
        let dear = TerrainCell {
            move_cost: 250,
            ..ground()
        };
        let mut all_dear = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                all_dear.push(((x, y), dear));
            }
        }
        let slow = open_on(vec![red()], &all_dear).expect("it opens");
        assert_eq!(slow.max_speed_cms(), 5_000);
    }

    /// **16b cut AE.1: the ceiling is each unit's own row's** (16c `T-15`
    /// re-derived). On a priced window of grass with one road cell — the tracked
    /// row prices the road at 100, the wheeled row at 80 — a 3,000 cm/s car
    /// answers `3,000 × 100 / 80` = 3,750 and the match opens (the substep is
    /// `min(250, 999,000 / 3,750)` = 250 ms, the shipped ceiling's); a
    /// 16,000 cm/s car over that one road cell answers 20,000 and is refused
    /// `SubstepBound`; and a 3,000 cm/s **tracked** unit on the same window
    /// answers 3,000, because its own row's cheapest cell is the road at 100.
    #[test]
    fn ae1_the_ceiling_is_each_units_own_rows() {
        use crate::arena::test_support::{priced_terrain, tactical_unit_on};
        use mush_atlas::TerrainClass;
        let open = |speed_cms: u32, locomotion: Option<&str>| {
            let mut unit = fast(ObjId(4), Team::Red, pose(1_500, 1_500), speed_cms);
            if let Some(locomotion) = locomotion {
                unit.unit = tactical_unit_on(ObjId(4), locomotion);
                unit.reseal(&crate::match_::SealTerms::fixture())
                    .expect("the fixture terms seal");
            }
            let mut out = custody(vec![unit], Vec::new());
            out.definition.terrain =
                priced_terrain(8, 8, TerrainClass::Grass, &[((5, 5), TerrainClass::Road)]);
            Arena::new(
                out,
                &ArenaConfig::default(),
                &MatchConfig::default(),
                &SensorsConfig::default(),
            )
        };
        let car = open(3_000, Some(crate::definitions::unit::WHEELED)).expect("the car opens");
        assert_eq!(car.min_cost, [100, 80, 100], "the road, on every row");
        assert_eq!(
            car.unit(ObjId(4)).map(|unit| unit.cost_row),
            Some(CostRow::Wheeled)
        );
        assert_eq!(car.max_speed_cms(), 3_750, "3,000 × 100 / 80");
        let tempo = crate::config::TempoConfig::default();
        assert_eq!(
            crate::arena::clock::substep_ms(&tempo, 1_000, car.max_speed_cms()),
            250,
            "999,000 / 3,750 is 266, so the cap answers"
        );
        assert!(matches!(
            open(16_000, Some(crate::definitions::unit::WHEELED)),
            Err(ArenaError::SubstepBound {
                ceiling_cms: 20_000,
                ..
            })
        ));
        let tank = open(3_000, None).expect("the tank opens");
        assert_eq!(
            tank.unit(ObjId(4)).map(|unit| unit.cost_row),
            Some(CostRow::Tracked)
        );
        assert_eq!(tank.max_speed_cms(), 3_000, "its own row's cheapest is 100");
    }

    /// **16b cut AE.1: an opening cell is judged on the unit's own row.**
    /// `Arena::new`'s opening test asks `passable_for(cost_row, ..)`, so on a
    /// priced window a car handed over on rubble is refused `CellImpassable`
    /// where a tank on the same cell opens, and a walker handed over in the
    /// thick woods opens where a tank is refused — the arena trusting no row's
    /// word but the unit's own, as it trusts no world pose.
    #[test]
    fn ae1_the_opening_cell_is_judged_on_the_units_own_row() {
        use crate::arena::test_support::{priced_terrain, tactical_unit_on};
        use crate::definitions::unit::{LEGGED, WHEELED};
        use mush_atlas::TerrainClass;
        let open = |locomotion: Option<&str>, class: TerrainClass| {
            let mut unit = tank(ObjId(4), Team::Red, pose(1_500, 1_500));
            if let Some(locomotion) = locomotion {
                unit.unit = tactical_unit_on(ObjId(4), locomotion);
                unit.reseal(&crate::match_::SealTerms::fixture())
                    .expect("the fixture terms seal");
            }
            let mut out = custody(vec![unit], Vec::new());
            out.definition.terrain = priced_terrain(8, 8, TerrainClass::Grass, &[((1, 1), class)]);
            Arena::new(
                out,
                &ArenaConfig::default(),
                &MatchConfig::default(),
                &SensorsConfig::default(),
            )
        };
        let refused = ArenaError::CellImpassable {
            unit: ObjId(4),
            x: 1,
            y: 1,
        };
        assert_eq!(
            open(Some(WHEELED), TerrainClass::Rubble).err(),
            Some(refused.clone()),
            "a car opened on rubble"
        );
        assert!(
            open(None, TerrainClass::Rubble).is_ok(),
            "a tank opens on rubble"
        );
        assert!(
            open(Some(LEGGED), TerrainClass::WoodsHeavy).is_ok(),
            "a walker opens in the thick woods"
        );
        assert_eq!(
            open(None, TerrainClass::WoodsHeavy).err(),
            Some(refused),
            "a tank opened in the thick woods"
        );
    }

    /// **16b cut M.4 (16c `B-57`): a match the substep floor cannot time is
    /// refused by name.** 50 ms at 20,000 cm/s is a whole 1,000 cm cell; 19,980
    /// is the last speed the floor still bounds. The ceiling is the true one, so
    /// 16,000 cm/s over one 80-cost cell is 20,000 and refused as well. The
    /// sentence names no unit and no cell.
    #[test]
    fn m4_a_unit_the_substep_floor_cannot_bound_is_refused_by_name() {
        let refused = open_on(
            vec![fast(ObjId(4), Team::Red, pose(1_500, 1_500), 20_000)],
            &[],
        );
        let Err(error) = refused else {
            panic!("20,000 cm/s on 1,000 cm cells opened");
        };
        assert_eq!(
            error,
            ArenaError::SubstepBound {
                ceiling_cms: 20_000,
                cell_cm: 1_000,
            }
        );
        assert_eq!(
            error.to_string(),
            "the fastest unit would cross more than one cell in a 50 ms substep on 1000 cm \
             cells; the match cannot be timed"
        );
        assert!(
            open_on(
                vec![fast(ObjId(4), Team::Red, pose(1_500, 1_500), 19_980)],
                &[],
            )
            .is_ok(),
            "19,980 cm/s is 999 cm a substep, inside one cell"
        );
        let cheap = TerrainCell {
            move_cost: 80,
            ..ground()
        };
        assert!(matches!(
            open_on(
                vec![fast(ObjId(4), Team::Red, pose(1_500, 1_500), 16_000)],
                &[((6, 6), cheap)],
            ),
            Err(ArenaError::SubstepBound {
                ceiling_cms: 20_000,
                ..
            })
        ));
    }

    /// **16b cut M.4 (16c `T-14`): an impassable opening cell is refused by
    /// name**, with the unit and the cell on the error's fields (and in the
    /// log) and neither in the sentence the centre is read.
    #[test]
    fn m4_an_impassable_opening_cell_is_refused_and_the_sentence_names_nobody() {
        let water = TerrainCell {
            class: mush_atlas::TerrainClass::Water,
            move_cost: 0,
            ..ground()
        };
        let refused = open_on(
            vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))],
            &[((1, 1), water)],
        );
        let Err(error) = refused else {
            panic!("a tank opened standing in water");
        };
        assert_eq!(
            error,
            ArenaError::CellImpassable {
                unit: ObjId(4),
                x: 1,
                y: 1,
            }
        );
        let said = error.to_string();
        assert_eq!(said, "a unit's opening cell is not passable");
        assert!(!said.contains('#') && !said.chars().any(|c| c.is_ascii_digit()));
        // The neighbouring cell is fine.
        assert!(
            open_on(
                vec![tank(ObjId(4), Team::Red, pose(2_500, 1_500))],
                &[((1, 1), water)],
            )
            .is_ok()
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut O.1 — the modes and the Hold objective
    // -----------------------------------------------------------------------

    /// The test window with its one key spot declared for Hold: `The Middle`
    /// at (4, 4), one cell round it, ten seconds.
    fn hold_definition() -> ArenaDefinition {
        ArenaDefinition {
            modes: vec![ArenaMode::Hold(HoldRule {
                spot: 0,
                radius_cells: 1,
                hold_ms: 10_000,
            })],
            ..crate::arena::test_support::definition()
        }
    }

    /// An arena of `units` on [`hold_definition`] at `MatchMode::Hold`.
    fn hold_arena(units: Vec<CustodyUnit>) -> Arena {
        Arena::new(
            CustodyOut {
                definition: hold_definition(),
                mode: MatchMode::Hold,
                ..custody(units, Vec::new())
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the hold arena")
    }

    /// **16b cut O.1**: the mode's words, codes and parse, and its serde word.
    #[test]
    fn o1_match_mode_round_trips_its_word_and_code() {
        assert_eq!(
            MatchMode::ALL,
            [MatchMode::Skirmish, MatchMode::Hold, MatchMode::Range]
        );
        assert_eq!(MatchMode::default(), MatchMode::Skirmish);
        for (mode, word, code) in [
            (MatchMode::Skirmish, "skirmish", 0),
            (MatchMode::Hold, "hold", 1),
            (MatchMode::Range, "range", 2),
        ] {
            assert_eq!(mode.as_str(), word);
            assert_eq!(mode.code(), code);
            assert_eq!(MatchMode::parse(word), Some(mode));
            assert_eq!(
                serde_json::to_value(mode).expect("encodes"),
                serde_json::json!(word)
            );
            assert_eq!(
                serde_json::from_value::<MatchMode>(serde_json::json!(word)).expect("decodes"),
                mode
            );
        }
        assert_eq!(
            MatchMode::parse(" HOLD "),
            Some(MatchMode::Hold),
            "trimmed and case-folded"
        );
        assert_eq!(MatchMode::parse("Skirmish"), Some(MatchMode::Skirmish));
        // 16b cut O.3: the word parses; `lobby::mode` is what refuses it.
        assert_eq!(MatchMode::parse("range"), Some(MatchMode::Range));
        assert_eq!(MatchMode::parse(""), None);
        assert_eq!(
            ArenaMode::Hold(HoldRule {
                spot: 0,
                radius_cells: 1,
                hold_ms: 10_000
            })
            .mode(),
            MatchMode::Hold
        );
        // The support rule: skirmish everywhere, hold where declared.
        let bare = crate::arena::test_support::definition();
        assert!(bare.supports(MatchMode::Skirmish));
        assert!(!bare.supports(MatchMode::Hold));
        // 16b cut O.3: every ground supports a range.
        assert!(bare.supports(MatchMode::Range));
        assert_eq!(bare.hold_rule(), None);
        let held = hold_definition();
        assert!(held.supports(MatchMode::Hold));
        assert_eq!(held.hold_rule().map(|rule| rule.spot), Some(0));
        // And the modes are not in the definition's digest (plan item 9).
        assert_eq!(held.digest(), bare.digest());
    }

    /// **16b cut O.1**: a Hold check-out on a ground that declares none is
    /// refused by name, on the stand-in window as on any other; the same units
    /// at a skirmish open.
    #[test]
    fn o1_a_hold_on_a_ground_that_declares_none_is_refused() {
        let stock = crate::match_::stock_definition(&SmolStr::new(crate::match_::STAND_IN))
            .expect("the stand-in window");
        let custody_on = |mode: MatchMode| CustodyOut {
            definition: stock.clone(),
            mode,
            ..custody(
                vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))],
                Vec::new(),
            )
        };
        let refused = Arena::new(
            custody_on(MatchMode::Hold),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        let Err(error) = refused else {
            panic!("a hold opened on a ground with no key spot to hold");
        };
        assert_eq!(
            error,
            ArenaError::ModeUnsupported {
                mode: "hold",
                arena: SmolStr::new(crate::match_::STAND_IN),
            }
        );
        assert_eq!(
            error.to_string(),
            "arena tacsim.arena.stand_in declares no mode hold"
        );
        let opened = Arena::new(
            custody_on(MatchMode::Skirmish),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("a skirmish opens");
        assert_eq!(opened.mode, MatchMode::Skirmish);
        assert_eq!(opened.hold, None);
        // A hold on a ground that declares one opens with its state resolved.
        let held = hold_arena(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        assert_eq!(held.mode, MatchMode::Hold);
        assert_eq!(
            held.hold,
            Some(HoldState {
                spot: (4, 4),
                letter: Some('A'),
                radius_cells: 1,
                hold_ms: 10_000,
                held_by: None,
            })
        );
    }

    /// **16b cut O.1**: the square is Chebyshev — both axes within the radius.
    #[test]
    fn o1_the_hold_square_is_chebyshev() {
        let hold = HoldState {
            spot: (10, 10),
            letter: Some('B'),
            radius_cells: 2,
            hold_ms: 10_000,
            held_by: None,
        };
        for cell in [(10, 10), (8, 8), (12, 12), (8, 12), (12, 8), (10, 12)] {
            assert!(hold.square_contains(cell), "{cell:?} is on the spot");
        }
        for cell in [(7, 10), (13, 10), (10, 7), (10, 13), (13, 13)] {
            assert!(!hold.square_contains(cell), "{cell:?} is off the spot");
        }
        let edge = HoldState {
            spot: (0, 0),
            ..hold
        };
        assert!(edge.square_contains((0, 0)) && edge.square_contains((2, 2)));
        assert!(!edge.square_contains((3, 0)));
    }

    /// **16b cut O.1**: `hold_presence` over the fixtures — nobody, one side,
    /// both sides, a wreck or a disabled unit inside counting for nothing, a
    /// unit off the window counting for nothing, and a skirmish answering
    /// `None` whoever stands there.
    #[test]
    fn o1_hold_presence_reads_the_sides_in_action_on_the_square() {
        // The spot is (4, 4); cell centres are `n × 1000 + 500`.
        let at = |x: i64, y: i64| pose(x * 1_000 + 500, y * 1_000 + 500);
        let mut arena = hold_arena(vec![
            tank(ObjId(4), Team::Red, at(0, 0)),
            tank(ObjId(5), Team::Red, at(1, 0)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        assert_eq!(arena.hold_presence(), None, "nobody on the spot");
        let put = |arena: &mut Arena, id: u32, x: i64, y: i64| {
            if let Some(unit) = arena.unit_mut(ObjId(id)) {
                unit.pose = at(x, y);
            }
        };
        put(&mut arena, 4, 4, 5);
        assert_eq!(arena.hold_presence(), Some(Team::Red), "one side");
        put(&mut arena, 5, 3, 3);
        assert_eq!(arena.hold_presence(), Some(Team::Red), "two of one side");
        put(&mut arena, 9, 5, 5);
        assert_eq!(arena.hold_presence(), None, "both sides: contested");
        // A wreck holds nothing.
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.limits.wreck = true;
        }
        assert_eq!(
            arena.hold_presence(),
            Some(Team::Red),
            "the blue wreck counts for nothing"
        );
        // Nor does a disabled hulk.
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.limits.wreck = false;
            unit.disabled = true;
        }
        assert_eq!(
            arena.hold_presence(),
            Some(Team::Red),
            "a disabled unit counts for nothing"
        );
        // Red's two gone the same ways: nobody.
        for (id, wreck) in [(4, true), (5, false)] {
            if let Some(unit) = arena.unit_mut(ObjId(id)) {
                unit.limits.wreck = wreck;
                unit.disabled = !wreck;
            }
        }
        assert_eq!(arena.hold_presence(), None);
        // A unit off the window is on no square — tested on a spot at the
        // window's east edge, (7, 4), whose square reaches column 8: a pose at
        // x 8,500 cm has no cell, where a clamped cell (7, 4) or a naive one
        // (8, 4) would both stand inside it.
        let mut off = hold_arena(vec![
            tank(ObjId(4), Team::Red, at(0, 0)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        off.hold = off.hold.map(|hold| HoldState {
            spot: (7, 4),
            ..hold
        });
        put(&mut off, 4, 7, 4);
        assert_eq!(
            off.hold_presence(),
            Some(Team::Red),
            "the edge cell itself is on the square"
        );
        if let Some(unit) = off.unit_mut(ObjId(4)) {
            unit.pose = pose(8_500, 4_500);
        }
        assert_eq!(off.definition.terrain.cell_of(&pose(8_500, 4_500)), None);
        assert!(off.hold.is_some_and(|hold| hold.square_contains((8, 4))));
        assert_eq!(
            off.hold_presence(),
            None,
            "a unit off the window holds nothing"
        );
        // A skirmish has no square to stand on.
        let mut skirmish = arena_of(vec![
            tank(ObjId(4), Team::Red, at(4, 4)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        assert_eq!(skirmish.hold_presence(), None);
        skirmish.settle_hold_presence();
        assert!(
            !skirmish
                .due
                .iter()
                .any(|event| matches!(event.kind, DueKind::HoldSettles { .. })),
            "a skirmish schedules no hold"
        );
    }

    /// **16b cut O.1**: `settle_hold_presence` opens the clock with one event,
    /// keeps it while the same side stays, drops it when the square is
    /// contested or empty, and re-opens with a new `since` and a new event.
    #[test]
    fn o1_settling_presence_opens_keeps_drops_and_reopens_the_clock() {
        let at = |x: i64, y: i64| pose(x * 1_000 + 500, y * 1_000 + 500);
        let mut arena = hold_arena(vec![
            tank(ObjId(4), Team::Red, at(4, 4)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        let holds = |arena: &Arena| -> Vec<(i64, Team, i64)> {
            arena
                .due
                .iter()
                .filter_map(|event| match event.kind {
                    DueKind::HoldSettles { team, since_ms } => Some((event.t_ms, team, since_ms)),
                    _ => None,
                })
                .collect()
        };
        arena.now_ms = 250;
        arena.settle_hold_presence();
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Red, 250))
        );
        assert_eq!(holds(&arena), vec![(10_250, Team::Red, 250)]);
        // Still alone at the next boundary: unchanged, and no second event.
        arena.now_ms = 500;
        arena.settle_hold_presence();
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Red, 250))
        );
        assert_eq!(holds(&arena).len(), 1);
        // Contested: dropped; the event is left to fire into nothing.
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.pose = at(5, 5);
        }
        arena.now_ms = 750;
        arena.settle_hold_presence();
        assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
        assert_eq!(holds(&arena).len(), 1, "retain is not used");
        // Blue alone: a new clock, a new event.
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.pose = at(0, 0);
        }
        arena.now_ms = 1_000;
        arena.settle_hold_presence();
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((Team::Blue, 1_000))
        );
        assert_eq!(
            holds(&arena),
            vec![(10_250, Team::Red, 250), (11_000, Team::Blue, 1_000)]
        );
        // Empty: dropped again.
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.pose = at(7, 7);
        }
        arena.now_ms = 1_250;
        arena.settle_hold_presence();
        assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
    }

    /// **16b cut O.1**: a wreck cancels nothing of the hold's — `destroy_unit`'s
    /// retain keeps a scheduled `HoldSettles`, and the arm's fresh read is what
    /// decides.
    #[test]
    fn o1_destroy_unit_keeps_a_scheduled_hold() {
        let at = |x: i64, y: i64| pose(x * 1_000 + 500, y * 1_000 + 500);
        let mut arena = hold_arena(vec![
            tank(ObjId(4), Team::Red, at(4, 4)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        arena.now_ms = 250;
        arena.settle_hold_presence();
        let mut journal = Vec::new();
        arena.destroy_unit(ObjId(4), 300, None, &mut journal);
        assert!(
            arena.due.iter().any(|event| event.kind
                == DueKind::HoldSettles {
                    team: Team::Red,
                    since_ms: 250
                }),
            "the hold's event survives its holder's wreck"
        );
        assert_eq!(
            arena.hold_presence(),
            None,
            "and the fresh read finds nobody"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut AE.3 — W2's cap, and a drone on the hold square
    // -----------------------------------------------------------------------

    /// An arena of `units` under `caps`, or its refusal.
    fn capped(units: Vec<CustodyUnit>, caps: &[(UnitClass, u32)]) -> Result<Arena, ArenaError> {
        let arena_cfg = ArenaConfig {
            max_units_by_class: caps.iter().copied().collect(),
            ..ArenaConfig::default()
        };
        Arena::new(
            custody(units, Vec::new()),
            &arena_cfg,
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
    }

    /// **16b cut AE.3** (doc 16 §2 W2): `Arena::new` counts every unit per side
    /// and class and refuses a side over its cap by name — two drones on red at
    /// the default one — with nothing built; one a side is two drones on the
    /// map and accepted (the cap is per side); `Drone: 2` admits red's two;
    /// `Drone: 0` refuses even one; and a blob-less fixture reads `Armour` and
    /// is never counted against the drone's cap, whatever it is.
    #[test]
    fn ae3_arena_new_refuses_a_side_over_its_class_cap() {
        use crate::arena::test_support::drone;
        let default_caps: Vec<(UnitClass, u32)> = ArenaConfig::default()
            .max_units_by_class
            .into_iter()
            .collect();
        assert_eq!(default_caps, vec![(UnitClass::Drone, 1)]);
        let two_red = || {
            vec![
                drone(ObjId(4), Team::Red, pose(500, 500)),
                drone(ObjId(5), Team::Red, pose(1_500, 500)),
                tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
            ]
        };
        let Err(error) = capped(two_red(), &default_caps) else {
            panic!("two drones on one side opened under a cap of one");
        };
        assert_eq!(
            error,
            ArenaError::ClassCap {
                team: "red",
                class: "drone",
                fielded: 2,
                cap: 1,
            }
        );
        assert_eq!(
            error.to_string(),
            "the red side fields 2 of class drone; a side fields at most 1"
        );
        // One a side: per side, not per map.
        let one_each = capped(
            vec![
                drone(ObjId(4), Team::Red, pose(500, 500)),
                drone(ObjId(9), Team::Blue, pose(7_500, 7_500)),
            ],
            &default_caps,
        )
        .expect("one drone a side is legal");
        assert!(
            one_each
                .units
                .iter()
                .all(|unit| unit.class == UnitClass::Drone)
        );
        // Raised, the two open.
        capped(two_red(), &[(UnitClass::Drone, 2)]).expect("a cap of two admits two");
        // Zero forbids the class outright.
        assert_eq!(
            capped(
                vec![
                    drone(ObjId(4), Team::Red, pose(500, 500)),
                    tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
                ],
                &[(UnitClass::Drone, 0)],
            )
            .err()
            .map(|error| error.to_string()),
            Some("the red side fields 1 of class drone; a side fields at most 0".to_owned())
        );
        // The blob-less fixture is armour and never a drone.
        let tanks = capped(
            vec![
                tank(ObjId(4), Team::Red, pose(500, 500)),
                tank(ObjId(5), Team::Red, pose(1_500, 500)),
                tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
            ],
            &[(UnitClass::Drone, 0)],
        )
        .expect("a drone cap of nought touches no tank");
        assert!(
            tanks
                .units
                .iter()
                .all(|unit| unit.class == UnitClass::Armour)
        );
        // An armour cap binds the same way, on the tanks.
        assert_eq!(
            capped(
                vec![
                    tank(ObjId(4), Team::Red, pose(500, 500)),
                    tank(ObjId(5), Team::Red, pose(1_500, 500)),
                    tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
                ],
                &[(UnitClass::Armour, 1)],
            )
            .err()
            .map(|error| error.to_string()),
            Some("the red side fields 2 of class armour; a side fields at most 1".to_owned())
        );
    }

    /// **Doc 18 cut IC2** (doc 18 §4.2, D-18-29): a hauler at the custody seam
    /// is refused before anything is built — a hauler on red beside a blue tank,
    /// under the default caps (which name no hauler), answers
    /// `UnfieldableClass` with its sentence; the class is refused outright, not
    /// counted against a cap.
    #[test]
    fn ic2_arena_new_refuses_a_hauler() {
        use crate::arena::test_support::hauler;
        let default_caps: Vec<(UnitClass, u32)> = ArenaConfig::default()
            .max_units_by_class
            .into_iter()
            .collect();
        let Err(error) = capped(
            vec![
                hauler(ObjId(4), Team::Red, pose(500, 500)),
                tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
            ],
            &default_caps,
        ) else {
            panic!("a hauler opened an arena");
        };
        assert_eq!(
            error,
            ArenaError::UnfieldableClass {
                team: "red",
                class: "hauler",
            }
        );
        assert_eq!(
            error.to_string(),
            "the red side fields a hauler, which no arena fields"
        );
    }

    /// **Doc 20 cut SM.3a** (D-20-6; reviewer F7): the hauler test's twin — a
    /// terrain vehicle at the custody seam is refused before anything is
    /// built. The sentence reads the class word (`UnitClass::as_str`), not the
    /// noun, so it prints `a vehicle`.
    #[test]
    fn sm3a_arena_new_refuses_a_vehicle() {
        use crate::arena::test_support::vehicle;
        let default_caps: Vec<(UnitClass, u32)> = ArenaConfig::default()
            .max_units_by_class
            .into_iter()
            .collect();
        let Err(error) = capped(
            vec![
                vehicle(ObjId(4), Team::Red, pose(500, 500)),
                tank(ObjId(9), Team::Blue, pose(7_500, 7_500)),
            ],
            &default_caps,
        ) else {
            panic!("a terrain vehicle opened an arena");
        };
        assert_eq!(
            error,
            ArenaError::UnfieldableClass {
                team: "red",
                class: "vehicle",
            }
        );
        assert_eq!(
            error.to_string(),
            "the red side fields a vehicle, which no arena fields"
        );
    }

    /// **16b cut AE.3**: `ArenaConfig::validate` bounds every cap `0..=24` by
    /// name — 25 is refused with its sentence, and 0 (the class forbidden) and
    /// 24 (every unit an arena places) are accepted.
    #[test]
    fn ae3_validate_bounds_the_class_caps() {
        let with = |cap: u32| ArenaConfig {
            max_units_by_class: BTreeMap::from([(UnitClass::Drone, cap)]),
            ..ArenaConfig::default()
        };
        assert_eq!(
            with(25).validate(),
            Err("[tacsim.arena.max_units_by_class].drone must be 0 to 24 (it is 25)".to_owned())
        );
        assert_eq!(with(0).validate(), Ok(()));
        assert_eq!(with(24).validate(), Ok(()));
        assert_eq!(ArenaConfig::default().validate(), Ok(()));
    }

    /// **16b cut AE.3** (doc 16 §2 W2): a drone holds nothing and contests
    /// nothing. Alone on the square it is nobody; beside its own tank the side
    /// holds through the tank; and on the square with an enemy tank it does not
    /// contest it. The O.1 tests above, whose holders are the unarmed `Armour`
    /// fixtures, are untouched.
    #[test]
    fn ae3_a_drone_holds_nothing_and_contests_nothing() {
        use crate::arena::test_support::drone;
        let at = |x: i64, y: i64| pose(x * 1_000 + 500, y * 1_000 + 500);
        let mut arena = hold_arena(vec![
            drone(ObjId(4), Team::Red, at(4, 4)),
            tank(ObjId(5), Team::Red, at(0, 0)),
            tank(ObjId(9), Team::Blue, at(7, 7)),
        ]);
        assert_eq!(
            arena.unit(ObjId(4)).map(|unit| unit.class),
            Some(UnitClass::Drone)
        );
        assert_eq!(arena.hold_presence(), None, "a drone alone holds nothing");
        let put = |arena: &mut Arena, id: u32, x: i64, y: i64| {
            if let Some(unit) = arena.unit_mut(ObjId(id)) {
                unit.pose = at(x, y);
            }
        };
        put(&mut arena, 5, 3, 4);
        assert_eq!(
            arena.hold_presence(),
            Some(Team::Red),
            "beside its own tank, the side holds through the tank"
        );
        // The drone standing on the square does not contest a blue tank alone.
        put(&mut arena, 5, 0, 0);
        put(&mut arena, 9, 5, 5);
        assert_eq!(
            arena.hold_presence(),
            Some(Team::Blue),
            "a drone contests nothing"
        );
        arena.settle_hold_presence();
        assert_eq!(
            arena
                .hold
                .as_ref()
                .and_then(|hold| hold.held_by)
                .map(|(team, _)| team),
            Some(Team::Blue)
        );
    }

    /// **17b cut M1a.5: the channel table is one table** (doc 17 §4,
    /// D-UM-47): [`COCKPIT_CHANNEL_IDS`] is [`Channel::ALL`]'s wire words in
    /// order, [`Channel::channel_id`] is each one's entry, and
    /// [`Channel::parse`] reads each entry back to its channel.
    #[test]
    fn m1a5_the_channel_ids_are_the_channels_words_both_ways() {
        assert_eq!(
            COCKPIT_CHANNEL_IDS,
            Channel::ALL.map(|channel| channel.as_str())
        );
        for (channel, id) in Channel::ALL.into_iter().zip(COCKPIT_CHANNEL_IDS) {
            assert_eq!(channel.channel_id(), id);
            assert_eq!(Channel::parse(id), Some(channel), "{id} reads back");
        }
    }

    /// **17b cut M1a.5: a mind is its own controller byte** (D-UM-2) — `2`,
    /// so the two existing bytes, and every hash that folds them, are what
    /// they were. **17b cut M4.6** deleted the reflex's `1` and renumbered
    /// nothing: the pilot is still `0` and the mind still `2`, which every
    /// hash of every match folds. The non-mind due kind below is the motor's
    /// tick, where it was the reflex's until that cut.
    #[test]
    fn m1a5_a_mind_is_code_two_and_the_word_mind() {
        assert_eq!(
            [Controller::Pilot, Controller::Mind].map(|c| (c.as_str(), c.code())),
            [("pilot", 0), ("mind", 2)]
        );
        assert!(
            DueKind::MindThink {
                unit: ObjId(1),
                think_seq: 0
            }
            .is_mind()
        );
        assert!(
            DueKind::MindLand {
                unit: ObjId(1),
                think_seq: 0
            }
            .is_mind()
        );
        assert!(!DueKind::MotorTick { unit: ObjId(1) }.is_mind());
    }
}
