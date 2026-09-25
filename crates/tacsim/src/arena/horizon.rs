//! **The horizon index** (16b cut J.2; doc 16 §3.2 "Line of sight", §3.6 step
//! 2, §2 W4; 16c N5 step 2, `T-36`).
//!
//! Cut J.1 made the arena's line of sight stop at its first blocker
//! ([`mush_atlas::sight_only`]), but a *clear* line still reads every interior
//! cell, and at the authored sixty-cell sight most lines over open ground are
//! long and clear. This is a two-level pyramid of what each block of the
//! terrain holds — its highest and its lowest cell, whether any cell is opaque,
//! whether any is partial — which lets a query **prove a run of a line clear
//! without reading its cells**, **prove it blocked by reading one**, and hand
//! everything else to the walk.
//!
//! # Where it lives, and why there
//!
//! The index is a private field of [`crate::arena::state::ArenaTerrain`], built
//! by [`crate::arena::state::ArenaTerrain::new`] — the terrain's one
//! constructor — from the very cells it is stored beside, and never written
//! again. The terrain is immutable in production (private fields, one
//! constructor, the definition digest over every cell, one `Arena` per match),
//! but **not in the tests**, which swap `arena.definition` and
//! `arena.definition.terrain` freely; a sibling index on `Arena` would go stale
//! at every such swap and prove a wall's block clear without reading it. An
//! index carried by the terrain cannot answer for any grid but its own, so a
//! swapped definition or terrain carries its own index with it.
//!
//! # A cache of terrain, and nothing observed
//!
//! Doc 16 §3.2 says line of sight *is directional; there is no cross-observer
//! cache*. This is not one: nothing observed is cached — no line, no answer, no
//! contact — and one observer's pass still depends on nothing another observer
//! did. The index holds **terrain**, a pure function of the cells the definition
//! digest already covers, so it is folded into no hash (see
//! [`crate::arena::hash::state_hash`]'s head) for the reason cut U.2's
//! `jam_fold` is not.
//!
//! # The query, and why it is exact
//!
//! [`Horizon::sight`] draws the line from its [`mush_atlas::canonical_ends`],
//! each end at its own height, exactly as `sight_only` does, and materialises
//! its cells into a stack buffer of [`MAX_LINE_CELLS`] (no grid read, no
//! allocation). It then splits the line, coarse level first, into maximal runs
//! of consecutive cells sharing one block, and settles each run in line order by
//! the first rule that applies:
//!
//! * **clear proof** — the block holds no opaque and no partial cell, and no
//!   cell of it stands above the run's lowest line height: the walk would find
//!   no blocker there and gather no attenuation, so the run is skipped unread;
//! * **blocked proof** — every cell of the block stands above the run's highest
//!   line height and the run holds an interior cell: the walk would stop at the
//!   run's first interior cell, so that one cell is read through
//!   [`mush_atlas::terrain_sight_rule`] and names the blocker;
//! * **descend** to the finer level over the same run;
//! * **walk** the run cell by cell at the finest level, as `sight_only` does.
//!
//! The line's height at step `i` is [`mush_atlas::line_height_cm`], a linear
//! numerator under an integer division that truncates toward zero, so it is
//! **monotone in `i`** and a run's lowest and highest line heights are the
//! heights at its two ends. Both proofs rest on that.
//!
//! The runs partition the line's interior cells in line order. A run proved
//! clear contributes what the walk's pass over it contributes: no blocker and
//! no attenuation. A walked run *is* the walk. A blocked proof stops where the
//! walk would stop — at the first interior cell of the first run that could
//! not be proved clear — with the kind the walk would give that cell, because
//! the same rule reads the same cell on the same attenuation. The endpoints are
//! in the runs only for the block fold, which makes a clear proof harder and
//! never easier; they are never read as cells and never tested. An endpoint off
//! the grid is not the index's to answer, and neither is a line longer than the
//! buffer or a terrain built [`Horizon::walk_only`]: those are answered by
//! `sight_only` itself. So **`Horizon::sight(g, a, b).sight ==
//! sight_only(g, a, b).sight` for every pair**, and the cells it reads are never
//! more than `sight_only`'s `visited`. It is conservative in both directions:
//! clear only where the walk would find nothing, blocked only at the walk's own
//! cell, and otherwise the walk.
//!
//! # Its cost
//!
//! Integers only, one `Vec` per level, no map. A [`Block`] is six bytes, so the
//! shipped 256 by 256 window costs `(256 + 4,096) × 6 = 26,112` bytes plus two
//! vector headers ([`Horizon::bytes`], reported on the harness's memory line
//! under `T-36`'s bound). Every block a query reads is counted
//! ([`crate::arena::perception::PerceptionCounters::horizon_probes`]) beside the
//! cells it reads (`cell_probes`).

use mush_atlas::physics::{Opacity, props};
use mush_atlas::trace::{
    Blocker, Bresenham, Eye, Sight, canonical_ends, line_height_cm, sight_only, terrain_sight_rule,
};

use crate::arena::state::{ArenaTerrain, TerrainCell};

/// Block sides as powers of two, coarse first: 16 and 4 cells.
pub const LEVELS: [u8; 2] = [4, 2];

/// A line longer than this is walked whole; no fielded window is that wide
/// (`crate::definitions::arena::from_pack` yields 256).
pub const MAX_LINE_CELLS: usize = 512;

/// The block holds a cell whose class is [`Opacity::Opaque`] (a wall, the void).
pub const OPAQUE: u8 = 1;

/// The block holds a cell whose class is [`Opacity::Partial`] (light or heavy
/// woods).
pub const PARTIAL: u8 = 2;

/// **What one block of the terrain holds**: the fold of the cells it covers.
///
/// `elev_cm` is an `i16` on [`TerrainCell`], so the fold is lossless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// The highest `elev_cm` of the block's cells.
    pub max_elev_cm: i16,
    /// The lowest `elev_cm` of the block's cells.
    pub min_elev_cm: i16,
    /// [`OPAQUE`] and [`PARTIAL`], OR-ed over the block's cells.
    pub flags: u8,
}

/// One level of the pyramid: blocks of `1 << log2` cells a side, row-major,
/// `cols = ceil(width / side)` by `rows = ceil(height / side)`. A block at the
/// window's right or bottom edge covers only the cells that exist.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Level {
    /// The block side as a power of two.
    log2: u8,
    /// Blocks per row.
    cols: u16,
    /// Rows of blocks.
    rows: u16,
    /// The blocks, row-major.
    blocks: Vec<Block>,
}

impl Level {
    /// Fold `cells` (row-major, `width × height`) into blocks of `1 << log2`.
    fn build(log2: u8, width: u16, height: u16, cells: &[TerrainCell]) -> Level {
        let side = 1_u32 << log2;
        let ceil = |extent: u16| -> u16 {
            u16::try_from((u32::from(extent) + side - 1) >> log2).unwrap_or(u16::MAX)
        };
        let (cols, rows) = (ceil(width), ceil(height));
        // Every block covers at least one cell (the counts are ceilings), so each
        // starting value is overwritten by its first cell.
        let mut blocks = vec![
            Block {
                max_elev_cm: i16::MIN,
                min_elev_cm: i16::MAX,
                flags: 0,
            };
            usize::from(cols) * usize::from(rows)
        ];
        for (index, cell) in cells.iter().enumerate() {
            let (Ok(x), Ok(y)) = (
                u16::try_from(index % usize::from(width.max(1))),
                u16::try_from(index / usize::from(width.max(1))),
            ) else {
                continue;
            };
            let slot = usize::from(y >> log2) * usize::from(cols) + usize::from(x >> log2);
            if let Some(block) = blocks.get_mut(slot) {
                block.max_elev_cm = block.max_elev_cm.max(cell.elev_cm);
                block.min_elev_cm = block.min_elev_cm.min(cell.elev_cm);
                block.flags |= match props(cell.class).opacity {
                    Opacity::Opaque => OPAQUE,
                    Opacity::Partial(_) => PARTIAL,
                    Opacity::Clear => 0,
                };
            }
        }
        Level {
            log2,
            cols,
            rows,
            blocks,
        }
    }

    /// The block id of cell `(x, y)` at this level.
    const fn id(&self, at: (u16, u16)) -> (u16, u16) {
        (at.0 >> self.log2, at.1 >> self.log2)
    }

    /// The block whose span holds `(x, y)`, or `None` past the last block.
    fn block(&self, at: (u16, u16)) -> Option<Block> {
        let (bx, by) = self.id(at);
        if bx >= self.cols || by >= self.rows {
            return None;
        }
        let slot = usize::from(by) * usize::from(self.cols) + usize::from(bx);
        self.blocks.get(slot).copied()
    }
}

/// **The horizon index**: every level of [`LEVELS`] over one terrain's cells.
/// See the module head for what it proves and why it is exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Horizon {
    /// Coarse first; empty for [`Horizon::walk_only`].
    levels: Vec<Level>,
}

/// **What one query cost and found** (16b cut J.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HorizonSight {
    /// Clear, attenuated or blocked — exactly [`mush_atlas::sight_only`]'s
    /// answer, blocker and all.
    pub sight: Sight,
    /// Interior cells read, hit or miss: the cells of every walked run up to
    /// and including a blocker, and the one cell a blocked proof reads. Never
    /// an endpoint, and never more than `sight_only`'s `visited`.
    pub cell_probes: u32,
    /// Blocks read: one per run, whichever rule settled it — proved clear,
    /// proved blocked, descended, or walked at the finest level. Zero when the
    /// line was answered by `sight_only` whole.
    pub horizon_probes: u32,
}

impl Horizon {
    /// Every level of [`LEVELS`], every block the fold of the cells it covers.
    /// `cells` is row-major, `width × height` long (the shape
    /// [`ArenaTerrain::new`] has just checked, and the reason this takes the
    /// cells and not the terrain: it runs before the terrain exists).
    #[must_use]
    pub fn build(width: u16, height: u16, cells: &[TerrainCell]) -> Horizon {
        Horizon {
            levels: LEVELS
                .iter()
                .map(|&log2| Level::build(log2, width, height, cells))
                .collect(),
        }
    }

    /// No levels: every line is walked. [`ArenaTerrain::without_horizon`] and
    /// the fallback shape.
    #[must_use]
    pub const fn walk_only() -> Horizon {
        Horizon { levels: Vec::new() }
    }

    /// **The line of sight from `from` to `to` over `grid`, through the index**
    /// — `sight_only`'s answer to the byte, with its cost.
    ///
    /// `grid` must be the terrain this index was built from; the one caller,
    /// [`ArenaTerrain::sight`], passes the terrain that carries it, which is how
    /// that holds by construction.
    #[must_use]
    pub fn sight(&self, grid: &ArenaTerrain, from: Eye, to: Eye) -> HorizonSight {
        let walked = || {
            let seen = sight_only(grid, from, to);
            HorizonSight {
                sight: seen.sight,
                cell_probes: seen.visited,
                horizon_probes: 0,
            }
        };
        // Each end keeps its own height: swapped, `lo` is `to`'s cell at `to`'s
        // height — `sight_only`'s own rule.
        let (lo, hi) = if canonical_ends(from.cell, to.cell) == (from.cell, to.cell) {
            (from, to)
        } else {
            (to, from)
        };
        // The two uncounted endpoint reads. Either end off the grid, a line past
        // the buffer, or no levels: the walk answers, with its own `Edge`. (An
        // off-grid cell's block id can land in a partly filled edge block, so the
        // index does not answer such a line at all.)
        let (Some(lo_read), Some(hi_read)) = (
            grid.get(lo.cell.0, lo.cell.1),
            grid.get(hi.cell.0, hi.cell.1),
        ) else {
            return walked();
        };
        let n = usize::from(
            lo.cell
                .0
                .abs_diff(hi.cell.0)
                .max(lo.cell.1.abs_diff(hi.cell.1)),
        );
        if self.levels.is_empty() || n >= MAX_LINE_CELLS {
            return walked();
        }
        let mut buffer = [(0_u16, 0_u16); MAX_LINE_CELLS];
        for (slot, at) in buffer.iter_mut().zip(Bresenham::new(lo.cell, hi.cell)) {
            *slot = at;
        }
        let Some(line) = buffer.get(..=n) else {
            return walked();
        };
        let mut query = Query {
            horizon: self,
            grid,
            line,
            z_lo: i32::from(lo_read.elev_cm) + lo.height_cm,
            z_hi: i32::from(hi_read.elev_cm) + hi.height_cm,
            n,
            attenuation: 0,
            cell_probes: 0,
            horizon_probes: 0,
        };
        let sight = match query.resolve(0, 0, n) {
            Some(blocked) => blocked,
            None if query.attenuation > 0 => {
                Sight::Partial(u8::try_from(query.attenuation.min(99)).unwrap_or(99))
            }
            None => Sight::Clear,
        };
        HorizonSight {
            sight,
            cell_probes: query.cell_probes,
            horizon_probes: query.horizon_probes,
        }
    }

    /// The levels' vectors: `size_of::<Block>()` per block.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.levels
            .iter()
            .map(|level| level.blocks.len() * size_of::<Block>())
            .sum()
    }

    /// The block sides, coarse first: `[16, 4]` when built, `[]` for
    /// [`Horizon::walk_only`].
    #[must_use]
    pub fn sides(&self) -> Vec<u16> {
        self.levels
            .iter()
            .map(|level| 1_u16 << level.log2)
            .collect()
    }

    /// The block of `level` (an index into [`LEVELS`]) whose span holds cell
    /// `(x, y)`, for tests and the fold check. It does not know the window's
    /// extent: a cell off the grid inside a partly filled edge block answers
    /// that block, which is why [`ArenaTerrain::horizon_block`] asks the grid
    /// first.
    #[must_use]
    pub fn block(&self, level: usize, x: u16, y: u16) -> Option<Block> {
        self.levels.get(level)?.block((x, y))
    }
}

/// One query's working state: the materialised canonical line, its two end
/// heights, the attenuation gathered so far and the two counts.
struct Query<'a> {
    /// The index.
    horizon: &'a Horizon,
    /// The terrain it was built from.
    grid: &'a ArenaTerrain,
    /// The canonical line's cells, `n + 1` of them, the lesser end first.
    line: &'a [(u16, u16)],
    /// The lesser end's line height, centimetres.
    z_lo: i32,
    /// The greater end's line height, centimetres.
    z_hi: i32,
    /// Steps: the line's Chebyshev length.
    n: usize,
    /// What the partial cells read so far have added, as the walk gathers it.
    attenuation: u32,
    /// Cells read.
    cell_probes: u32,
    /// Blocks read.
    horizon_probes: u32,
}

impl Query<'_> {
    /// The line's height at step `i` — monotone in `i` (see the module head),
    /// so a run's range is its two ends.
    fn z(&self, i: usize) -> i32 {
        line_height_cm(self.z_lo, self.z_hi, i, self.n)
    }

    /// **The first blocker in `[i0, i1]`, or `None` for none there**, settling
    /// the range run by run at `level` (the module head's four rules, first that
    /// applies). Every block read counts, whichever rule settles its run.
    fn resolve(&mut self, level: usize, i0: usize, i1: usize) -> Option<Sight> {
        let horizon = self.horizon;
        let Some(this) = horizon.levels.get(level) else {
            return self.walk(i0, i1);
        };
        let mut start = i0;
        while start <= i1 {
            let Some(&first) = self.line.get(start) else {
                break;
            };
            let id = this.id(first);
            // The run: every following cell of the range in the same block. A
            // line is monotone in x and in y, so it never re-enters a block.
            let mut end = start;
            while end < i1
                && self
                    .line
                    .get(end + 1)
                    .is_some_and(|&next| this.id(next) == id)
            {
                end += 1;
            }
            if let Some(blocked) = self.settle(level, this, first, start, end) {
                return Some(blocked);
            }
            start = end + 1;
        }
        None
    }

    /// Settle one run `[start, end]` whose first cell is `first`.
    fn settle(
        &mut self,
        level: usize,
        this: &Level,
        first: (u16, u16),
        start: usize,
        end: usize,
    ) -> Option<Sight> {
        let Some(block) = this.block(first) else {
            // Never: both ends are on the grid, so every cell of the line is. The
            // walk is the safe answer.
            return self.walk(start, end);
        };
        self.horizon_probes = self.horizon_probes.saturating_add(1);
        let (z_a, z_b) = (self.z(start), self.z(end));
        let (z_min, z_max) = (z_a.min(z_b), z_a.max(z_b));
        // Clear proof: nothing opaque or partial, nothing above the line's lowest
        // point — the walk finds no blocker here and gathers no attenuation.
        if block.flags == 0 && i32::from(block.max_elev_cm) <= z_min {
            return None;
        }
        // Blocked proof: every cell stands above the line's highest point, so the
        // walk stops at the run's first interior cell. Read that one, on a copy
        // of the attenuation.
        let j = start.max(1);
        if i32::from(block.min_elev_cm) > z_max && j <= end && j < self.n {
            if let Some(&at) = self.line.get(j)
                && let Some(cell) = self.grid.get(at.0, at.1)
            {
                self.cell_probes = self.cell_probes.saturating_add(1);
                let mut probe = self.attenuation;
                if let Some(blocker) = terrain_sight_rule(
                    at,
                    cell.class,
                    i32::from(cell.elev_cm),
                    self.z(j),
                    &mut probe,
                ) {
                    self.attenuation = probe;
                    return Some(Sight::BlockedBy(blocker));
                }
            }
            // Never, by construction (an opaque cell is `Terrain`, a partial one
            // reaching a hundred is `Terrain`, anything else stands above the
            // line). The attenuation is untouched, and the walk of the run reads
            // the cell again on it: never a panic, never a partial cell counted
            // twice.
            return self.walk(start, end);
        }
        if level + 1 < self.horizon.levels.len() {
            return self.resolve(level + 1, start, end);
        }
        self.walk(start, end)
    }

    /// **The walk of `[i0, i1]`**: `sight_only`'s loop over the range's interior
    /// cells, each counted.
    fn walk(&mut self, i0: usize, i1: usize) -> Option<Sight> {
        for i in i0.max(1)..=i1.min(self.n.saturating_sub(1)) {
            let Some(&at) = self.line.get(i) else {
                return Some(Sight::BlockedBy(Blocker::Edge));
            };
            self.cell_probes = self.cell_probes.saturating_add(1);
            let Some(cell) = self.grid.get(at.0, at.1) else {
                return Some(Sight::BlockedBy(Blocker::Edge));
            };
            if let Some(blocker) = terrain_sight_rule(
                at,
                cell.class,
                i32::from(cell.elev_cm),
                self.z(i),
                &mut self.attenuation,
            ) {
                return Some(Sight::BlockedBy(blocker));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_atlas::TerrainClass;
    use mush_atlas::trace::{EYE_HEIGHT_CM, sight_only};

    /// A fixed-seed integer generator: splitmix64, the atlas M.4 tests' and
    /// `tests/perception.rs`' own, so the reliefs and the pairs are the same on
    /// every run and every platform, and no `rand`.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A cell of `class` at `elev_cm`; `move_cost` is not read by anything here.
    fn cell(class: TerrainClass, elev_cm: i16) -> TerrainCell {
        TerrainCell {
            class,
            elev_cm,
            move_cost: 100,
        }
    }

    /// **A `width` by `height` relief with walls, both woods, whole-metre slopes
    /// and a 200 cm ridge** — the atlas M.4 relief's shape, with a sunken metre
    /// here and there so the fold's minimum is exercised below the datum too.
    fn relief(seed: u64, width: u16, height: u16) -> ArenaTerrain {
        let x_step = (width / 8).max(1);
        let y_step = (height / 4).max(1);
        let period = (width * 13 / 64).max(2);
        let phase = (width * 6 / 64) % period;
        let mut state = seed;
        let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
        for y in 0..height {
            for x in 0..width {
                let slope =
                    i16::try_from(x / x_step).unwrap_or(0) + i16::try_from(y / y_step).unwrap_or(0);
                let ridge = if x % period == phase { 2 } else { 0 };
                let class = match next(&mut state) % 200 {
                    0 => TerrainClass::Wall,
                    1..=4 => TerrainClass::WoodsLight,
                    5 => TerrainClass::WoodsHeavy,
                    _ => TerrainClass::Ground,
                };
                let bump = match next(&mut state) % 20 {
                    0 | 1 => 1,
                    2 => -1,
                    _ => 0,
                };
                cells.push(cell(class, (slope + ridge + bump) * 100));
            }
        }
        ArenaTerrain::new(width, height, 1_000, cells).expect("the relief")
    }

    /// One of the two shipped eye heights, off the generator: 150 cm, or the
    /// biped's 900 cm one time in eight.
    fn eye(state: &mut u64, width: u16, height: u16) -> Eye {
        let cell = (
            u16::try_from(next(state) % u64::from(width)).unwrap_or(0),
            u16::try_from(next(state) % u64::from(height)).unwrap_or(0),
        );
        Eye {
            cell,
            height_cm: if next(state).is_multiple_of(8) {
                900
            } else {
                EYE_HEIGHT_CM
            },
        }
    }

    /// The block at `(bx, by)` of `level`, by its block id — the fold check's
    /// own lookup, beside [`Horizon::block`]'s by cell.
    fn level_block(horizon: &Horizon, level: usize, bx: u16, by: u16) -> Option<Block> {
        let this = horizon.levels.get(level)?;
        if bx >= this.cols || by >= this.rows {
            return None;
        }
        this.blocks
            .get(usize::from(by) * usize::from(this.cols) + usize::from(bx))
            .copied()
    }

    /// **Every block is the fold of its cells** (16b cut J.2): over a 64 by 64
    /// relief and a 37 by 23 grid (whose edge blocks are partly filled at both
    /// levels), for every level and every block, the two elevations are the max
    /// and min of the cells it covers and the flags the OR of their opacities;
    /// every cell of the window is covered by exactly one block per level, which
    /// is the block `horizon_block` answers for it; and each level holds
    /// `ceil(w / s) × ceil(h / s)` blocks.
    #[test]
    fn j2_every_block_is_the_fold_of_its_cells() {
        for (seed, width, height) in [(0x4a32_u64, 64_u16, 64_u16), (0x4a33, 37, 23)] {
            let terrain = relief(seed, width, height);
            let built = Horizon::build(width, height, terrain.cells());
            assert_eq!(built.sides(), vec![16, 4]);
            let mut total = 0_usize;
            for (index, level) in built.levels.iter().enumerate() {
                let side = 1_u16 << level.log2;
                let (cols, rows) = (width.div_ceil(side), height.div_ceil(side));
                assert_eq!((level.cols, level.rows), (cols, rows), "{width}x{height}");
                assert_eq!(level.blocks.len(), usize::from(cols) * usize::from(rows));
                total += level.blocks.len();
                let mut covered = 0_usize;
                for by in 0..rows {
                    for bx in 0..cols {
                        let (mut max, mut min, mut flags) = (i16::MIN, i16::MAX, 0_u8);
                        let xs = bx * side..((bx + 1) * side).min(width);
                        let ys = by * side..((by + 1) * side).min(height);
                        for y in ys.clone() {
                            for x in xs.clone() {
                                let here = terrain.get(x, y).expect("a cell of the window");
                                max = max.max(here.elev_cm);
                                min = min.min(here.elev_cm);
                                flags |= match props(here.class).opacity {
                                    Opacity::Opaque => OPAQUE,
                                    Opacity::Partial(_) => PARTIAL,
                                    Opacity::Clear => 0,
                                };
                                covered += 1;
                                // The one block the terrain answers for the cell
                                // is this block.
                                assert_eq!(
                                    terrain.horizon_block(index, x, y),
                                    level_block(&built, index, bx, by),
                                    "({x}, {y}) at level {index}"
                                );
                            }
                        }
                        assert_eq!(
                            level_block(&built, index, bx, by),
                            Some(Block {
                                max_elev_cm: max,
                                min_elev_cm: min,
                                flags,
                            }),
                            "block ({bx}, {by}) of level {index}, {width}x{height}"
                        );
                    }
                }
                // The spans are disjoint by construction and cover the window
                // once: every cell, exactly one block per level.
                assert_eq!(covered, usize::from(width) * usize::from(height));
                // Off the window, the terrain answers no block.
                assert_eq!(terrain.horizon_block(index, width, 0), None);
                assert_eq!(terrain.horizon_block(index, 0, height), None);
            }
            assert_eq!(terrain.horizon_block(2, 0, 0), None, "two levels only");
            assert_eq!(terrain.horizon_bytes(), total * 6);
            assert_eq!(size_of::<Block>(), 6);
            // The relief carries every flag, so the fold was tested on both.
            let every = built
                .levels
                .iter()
                .flat_map(|level| level.blocks.iter())
                .fold(0_u8, |acc, block| acc | block.flags);
            assert_eq!(every, OPAQUE | PARTIAL, "{width}x{height}");
        }
    }

    /// **The wall line** (the `tests/perception.rs` fixture: flat ground, eyes
    /// at 150 cm, the line (10,10) to (40,10), a wall at (25,10)): **two cells
    /// and five blocks**. The 16-block over x 0..15 is proved clear; the one over
    /// x 16..31 holds the wall and is descended; its 4-blocks over 16..19 and
    /// 20..23 are proved clear; the one over 24..27 holds the wall and is walked,
    /// x 24 read clear and x 25 read as the wall — the blocker `sight_only`
    /// names, which read fifteen cells to find it.
    #[test]
    fn j2_the_wall_line_reads_two_cells_and_five_blocks() {
        let mut cells = vec![cell(TerrainClass::Ground, 0); 80 * 80];
        if let Some(wall) = cells.get_mut(10 * 80 + 25) {
            *wall = cell(TerrainClass::Wall, 0);
        }
        let terrain = ArenaTerrain::new(80, 80, 1_000, cells).expect("the window");
        let (from, to) = (
            Eye {
                cell: (10, 10),
                height_cm: 150,
            },
            Eye {
                cell: (40, 10),
                height_cm: 150,
            },
        );
        for (a, b) in [(from, to), (to, from)] {
            let seen = terrain.sight(a, b);
            assert_eq!(
                seen.sight,
                Sight::BlockedBy(Blocker::Terrain {
                    at: (25, 10),
                    class: TerrainClass::Wall
                })
            );
            assert_eq!(seen.sight, sight_only(&terrain, a, b).sight);
            assert_eq!((seen.cell_probes, seen.horizon_probes), (2, 5));
            assert_eq!(sight_only(&terrain, a, b).visited, 15);
        }
    }

    /// **The M.4 tie through the index** (16c `B-56`): 60 by 1, ground at 0 but
    /// 100 cm at cell 59, a ridge in cell 29, eyes at 150 cm both ends, so the
    /// line runs from 150 to 250 cm and crosses cell 28 at 197 cm and cell 29
    /// at 199 cm.
    ///
    /// At 200 cm the ridge blocks: 0..15 is proved clear; 16..31 is descended
    /// (its highest cell, 200, stands above the line's 177 cm at cell 16);
    /// 16..19, 20..23 and 24..27 are proved clear; 28..31 is walked and blocked
    /// at 29 — **two cells and six blocks**, `Elevation` at (29, 0) from both
    /// ends. At 199 cm it does not: 28..31 is walked whole and clear, 32..47 is
    /// proved clear, and 48..59 is proved clear (cell 59's 100 cm under the
    /// line's 231 cm at cell 48) — **four cells and eight blocks**, `Clear`.
    #[test]
    fn j2_the_tie_is_one_answer_both_ways_through_the_index() {
        let row = |ridge_cm: i16| {
            let mut cells = vec![cell(TerrainClass::Ground, 0); 60];
            if let Some(high) = cells.get_mut(59) {
                *high = cell(TerrainClass::Ground, 100);
            }
            if let Some(ridge) = cells.get_mut(29) {
                *ridge = cell(TerrainClass::Ground, ridge_cm);
            }
            ArenaTerrain::new(60, 1, 1_000, cells).expect("the row")
        };
        assert_eq!(
            [16, 28, 29, 48].map(|i| line_height_cm(150, 250, i, 59)),
            [177, 197, 199, 231]
        );
        let low = Eye {
            cell: (0, 0),
            height_cm: 150,
        };
        let high = Eye {
            cell: (59, 0),
            height_cm: 150,
        };
        let blocked = row(200);
        let under = row(199);
        for (a, b) in [(low, high), (high, low)] {
            let seen = blocked.sight(a, b);
            assert_eq!(
                seen.sight,
                Sight::BlockedBy(Blocker::Elevation {
                    at: (29, 0),
                    elev_cm: 200
                })
            );
            assert_eq!(seen.sight, sight_only(&blocked, a, b).sight);
            assert_eq!((seen.cell_probes, seen.horizon_probes), (2, 6));
            let seen = under.sight(a, b);
            assert_eq!(seen.sight, Sight::Clear);
            assert_eq!(seen.sight, sight_only(&under, a, b).sight);
            assert_eq!((seen.cell_probes, seen.horizon_probes), (4, 8));
        }
    }

    /// **A blocked proof reads one cell** (16b cut J.2): a line across a flat
    /// plain into a 16-block that stands a hundred metres above it is stopped at
    /// that block's first interior cell by one read — the cell and the kind
    /// `sight_only` gives it, after `sight_only` read every cell before it.
    #[test]
    fn j2_a_block_above_the_line_is_proved_blocked_at_one_cell() {
        let mut cells = vec![cell(TerrainClass::Ground, 0); 64];
        for x in 32..48 {
            if let Some(hill) = cells.get_mut(x) {
                *hill = cell(TerrainClass::Ground, 10_000);
            }
        }
        let terrain = ArenaTerrain::new(64, 1, 1_000, cells).expect("the row");
        let (a, b) = (Eye::standing((0, 0)), Eye::standing((60, 0)));
        let seen = terrain.sight(a, b);
        assert_eq!(
            seen.sight,
            Sight::BlockedBy(Blocker::Elevation {
                at: (32, 0),
                elev_cm: 10_000
            })
        );
        assert_eq!(seen.sight, sight_only(&terrain, a, b).sight);
        // 0..15 and 16..31 proved clear, 32..47 proved blocked at 32.
        assert_eq!((seen.cell_probes, seen.horizon_probes), (1, 3));
        assert_eq!(sight_only(&terrain, a, b).visited, 32);
    }

    /// **A blocked proof reads its cell on the attenuation the walk carried**
    /// (16b cut J.2): the row of the pin above, with heavy woods at x 5 and at
    /// x 32, the hill's first cell. Heavy woods add sixty; the walk gathers
    /// sixty at x 5 and reaches a hundred and twenty at x 32, so its blocker is
    /// the woods (`Terrain`), not the hill (`Elevation`). The blocked proof must
    /// answer the same, which it does only because it starts its one read from
    /// the attenuation the walked runs before it gathered, not from zero.
    #[test]
    fn j2_a_blocked_proof_carries_the_walked_attenuation() {
        let mut cells = vec![cell(TerrainClass::Ground, 0); 64];
        for x in 32..48 {
            if let Some(hill) = cells.get_mut(x) {
                *hill = cell(TerrainClass::Ground, 10_000);
            }
        }
        if let Some(woods) = cells.get_mut(5) {
            *woods = cell(TerrainClass::WoodsHeavy, 0);
        }
        if let Some(woods) = cells.get_mut(32) {
            *woods = cell(TerrainClass::WoodsHeavy, 10_000);
        }
        let terrain = ArenaTerrain::new(64, 1, 1_000, cells).expect("the row");
        let woods = Sight::BlockedBy(Blocker::Terrain {
            at: (32, 0),
            class: TerrainClass::WoodsHeavy,
        });
        for (a, b) in [((0, 0), (60, 0)), ((60, 0), (0, 0))] {
            let (a, b) = (Eye::standing(a), Eye::standing(b));
            let seen = terrain.sight(a, b);
            assert_eq!(seen.sight, woods, "{a:?} to {b:?}");
            assert_eq!(seen.sight, sight_only(&terrain, a, b).sight);
            // The 16-block over 0..15 holds the woods: descended. Its 4-blocks
            // over 0..3, 8..11 and 12..15 are proved clear; the one over 4..7
            // holds the woods and is walked (x 4 to 7, sixty gathered at x 5).
            // 16..31 is proved clear; 32..47 stands above the line and is proved
            // blocked at x 32, one cell read on the carried sixty: five cells,
            // seven blocks.
            assert_eq!(
                (seen.cell_probes, seen.horizon_probes),
                (5, 7),
                "{a:?} to {b:?}"
            );
            assert_eq!(sight_only(&terrain, a, b).visited, 32);
        }
    }

    /// **The index is `sight_only` over a relief** (16b cut J.2): twenty
    /// thousand seeded pairs on the 64 by 64 relief and as many on the 37 by 23
    /// grid, at mixed eye heights, answer `sight_only`'s exact sight — blocker
    /// and all — and never read more cells than it.
    #[test]
    fn j2_the_index_is_sight_only_over_a_relief() {
        for (seed, width, height) in [(0x4a34_u64, 64_u16, 64_u16), (0x4a35, 37, 23)] {
            let terrain = relief(seed, width, height);
            let mut state = seed ^ 0xff;
            let (mut fewer, mut kinds) = (0_u32, [0_u32; 3]);
            for _ in 0..20_000 {
                let (a, b) = (
                    eye(&mut state, width, height),
                    eye(&mut state, width, height),
                );
                let fast = terrain.sight(a, b);
                let walk = sight_only(&terrain, a, b);
                assert_eq!(fast.sight, walk.sight, "{a:?} to {b:?}");
                assert!(
                    fast.cell_probes <= walk.visited,
                    "{a:?} to {b:?}: {} against {}",
                    fast.cell_probes,
                    walk.visited
                );
                fewer += u32::from(fast.cell_probes < walk.visited);
                let kind = match fast.sight {
                    Sight::Clear => 0,
                    Sight::Partial(_) => 1,
                    Sight::BlockedBy(_) => 2,
                };
                if let Some(count) = kinds.get_mut(kind) {
                    *count += 1;
                }
            }
            let [clear, partial, blocked] = kinds;
            assert!(
                clear > 500 && partial > 500 && blocked > 500 && fewer > 5_000,
                "{width}x{height}: {clear} clear, {partial} partial, {blocked} blocked, \
                 {fewer} cheaper"
            );
        }
    }

    /// **Without the index, the walk** (16b cut J.2): `without_horizon()` answers
    /// `sight_only` to the byte, with `horizon_probes == 0` and `cell_probes ==
    /// visited`, over two thousand seeded pairs of the relief; and it is the same
    /// grid, levels aside.
    #[test]
    fn j2_walk_only_is_sight_only() {
        let terrain = relief(0x4a36, 64, 64).without_horizon();
        assert!(terrain.horizon_sides().is_empty());
        assert_eq!(terrain.horizon_bytes(), 0);
        assert_eq!(terrain.horizon_block(0, 0, 0), None);
        assert_eq!(terrain.cells(), relief(0x4a36, 64, 64).cells());
        let mut state = 0x4a37_u64;
        for _ in 0..2_000 {
            let (a, b) = (eye(&mut state, 64, 64), eye(&mut state, 64, 64));
            let walk = sight_only(&terrain, a, b);
            assert_eq!(
                terrain.sight(a, b),
                HorizonSight {
                    sight: walk.sight,
                    cell_probes: walk.visited,
                    horizon_probes: 0,
                }
            );
        }
    }

    /// **A line past the buffer is walked** (16b cut J.2): on a 600 by 1 grid
    /// the line from end to end has 600 cells, more than [`MAX_LINE_CELLS`], so
    /// `sight_only` answers it whole; a line of exactly 512 cells is the index's.
    #[test]
    fn j2_a_line_past_the_buffer_is_walked() {
        let mut cells = vec![cell(TerrainClass::Ground, 0); 600];
        if let Some(woods) = cells.get_mut(300) {
            *woods = cell(TerrainClass::WoodsLight, 0);
        }
        let terrain = ArenaTerrain::new(600, 1, 1_000, cells).expect("the row");
        let (a, b) = (Eye::standing((0, 0)), Eye::standing((599, 0)));
        let walk = sight_only(&terrain, a, b);
        assert_eq!(walk.sight, Sight::Partial(25));
        assert_eq!(
            terrain.sight(a, b),
            HorizonSight {
                sight: walk.sight,
                cell_probes: walk.visited,
                horizon_probes: 0,
            }
        );
        // 512 cells, the buffer's size, is the index's again: a woods cell in
        // one 4-block, everything else proved clear.
        let (a, b) = (Eye::standing((0, 0)), Eye::standing((511, 0)));
        let fast = terrain.sight(a, b);
        assert_eq!(fast.sight, sight_only(&terrain, a, b).sight);
        assert_eq!(fast.sight, Sight::Partial(25));
        assert!(fast.horizon_probes > 0);
        assert_eq!(fast.cell_probes, 4, "the woods' 4-block, walked");
    }

    /// **An endpoint off the grid is the walk's answer** (16b cut J.2): `(0,0)`
    /// to `(40,0)` and back on a 37 by 23 grid — the far end inside a partly
    /// filled edge block's span but off the window — is `sight_only`'s `Edge`,
    /// with no block read.
    #[test]
    fn j2_an_endpoint_off_the_grid_is_the_walks_answer() {
        let terrain =
            ArenaTerrain::uniform(37, 23, 1_000, cell(TerrainClass::Ground, 0)).expect("the grid");
        let (a, b) = (Eye::standing((0, 0)), Eye::standing((40, 0)));
        for (a, b) in [(a, b), (b, a)] {
            let fast = terrain.sight(a, b);
            let walk = sight_only(&terrain, a, b);
            assert_eq!(walk.sight, Sight::BlockedBy(Blocker::Edge));
            assert_eq!(fast.sight, walk.sight);
            assert_eq!(fast.cell_probes, walk.visited);
            assert_eq!(fast.horizon_probes, 0);
        }
    }

    /// **The shipped index is small** (16b cut J.2; 16c `T-36`): on a 256 by 256
    /// window it is `(256 + 4,096) × 6 = 26,112` bytes of blocks, under 64 KiB,
    /// at 16- and 4-cell sides.
    #[test]
    fn j2_the_shipped_index_is_small() {
        let terrain = ArenaTerrain::uniform(256, 256, 1_000, cell(TerrainClass::Ground, 0))
            .expect("the shipped window's shape");
        assert_eq!(terrain.horizon_sides(), vec![16, 4]);
        assert_eq!(terrain.horizon_bytes(), 26_112);
        assert!(terrain.horizon_bytes() < 65_536);
    }
}
