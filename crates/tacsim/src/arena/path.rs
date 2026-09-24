//! **The arena's own path search** (16b cut N.1; doc 16 §6 "AI", 16b §8's cut N
//! row, review item 21).
//!
//! A bounded A\* over [`ArenaTerrain`] and nothing else: no store, no atlas
//! window, no records. The arena holds a *copy* of the ground it fights on
//! (doc 16 §3.1, D-16-1 and D-16-3), so the controller's routes are computed from
//! that copy at pulse time and `mush_atlas::physics::path` — which walks a live
//! window — stays where it is, on the world half.
//!
//! # The bound is the whole design
//!
//! Review item 21: *the reflex controller's path budget is one sixteenth of the
//! arena*. 16b §9 raised it — `[atlas].path_max_cells` for the arena is **65,536**,
//! one node per cell of the 256 by 256 window — so a search that fails has failed
//! on the ground and not on the budget, and "unreachable" is an honest word. Two
//! further bounds keep that affordable and are `crate::arena::reflex`'s, not this
//! module's: one search per unit per `reflex.repath_ms`, and
//! `reflex.max_path_queries` searches per `reflex.decide_ms` window of arena time.
//!
//! # Why the scratch is borrowed and not allocated
//!
//! A 256 by 256 window is 65,536 nodes; a `g` score and a visit stamp for each is
//! half a megabyte. Allocating that per query would dominate the search, so
//! [`PathScratch`] is held for the life of the match on
//! [`crate::arena::reflex::ReflexMemory`] and reused, and a **generation stamp**
//! is what makes reuse free: a node belongs to this search only when its stamp
//! equals the current generation, so nothing is cleared between searches.
//!
//! # Integers only
//!
//! D-16-4. The step cost is the entered cell's `move_cost` for an orthogonal step
//! and `move_cost × 181 / 128` for a diagonal one — 1.4140625, which is
//! `sqrt(2)` to four decimal places and is exact in integers — and then, from 16b
//! cut X.6, the slope surcharge of the edge ([`crate::arena::slope`]). The
//! heuristic is the octile distance in the same units against the window's
//! **cheapest** cell ([`min_move_cost`]), which is admissible by construction: no
//! real step can cost less than the cheapest cell's cost, and slope only ever
//! raises a step's cost.
//!
//! # 16b cut X.6: the ground has a third dimension
//!
//! [`path_with`] takes the caller's slope limit and refuses an edge too steep
//! **either way** for it, exactly as [`crate::arena::motion`] refuses one at the
//! crossing — the same [`crate::arena::slope`] rule, because a route that
//! promised an edge the tank is then refused at is a tank driving at a bluff
//! until the match ends. [`path`] keeps its signature and its behaviour: it is
//! `path_with(.., None)`, and with no limit no edge is refused and no shoulder
//! is read, so every **level** window answers cell for cell and node for node
//! what it answered before. The surcharge itself is not gated on the limit: an
//! unlimited caller on relief pays the climb it plans over, because a search
//! that priced a climb at the flat's cost would promise a route motion charges
//! differently.
//!
//! **The shoulder rule is the safe direction** (16b cut M.4, 16c `T-17`): the
//! search refuses diagonals motion allows — a corner whose passable shoulder is a
//! cliff — and never allows one motion refuses, so every step of every route it
//! finds is a crossing `crate::arena::motion::step_refusal` accepts; a property
//! test over fixed-seed reliefs and limits holds it.
//!
//! The one named residual against `mush_atlas::physics::move_cost` is the
//! diagonal constant — 181/128 here against the atlas's 141/100 — which predates
//! this cut and is left alone; [`crate::arena::slope`]'s own tests pin the
//! difference.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use mush_atlas::physics::StepKind;

use crate::arena::slope;
use crate::arena::state::{ArenaTerrain, TerrainCell};

/// The numerator of the diagonal step's cost multiplier.
///
/// `181 / 128` is 1.4140625. A diagonal step must cost more than an orthogonal
/// one or the search will happily cut corners to nowhere, and it must be a
/// *constant* ratio or the heuristic below stops being admissible.
pub const DIAGONAL_NUM: u32 = 181;

/// The denominator of the diagonal step's cost multiplier.
pub const DIAGONAL_DEN: u32 = 128;

/// The eight neighbours of a cell, in a fixed order: the four orthogonals first,
/// then the four diagonals, each pair in ascending `(dy, dx)`.
///
/// The order is part of the answer — two cells at the same cost are separated by
/// which was pushed first — so it is written out rather than generated.
const NEIGHBOURS: [(i32, i32); 8] = [
    (0, -1),
    (-1, 0),
    (1, 0),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// **The cheapest passable cell on the window**, for the heuristic's scale.
///
/// `1` when the window has no passable cell at all, which makes the heuristic
/// weaker and never inadmissible — the one direction it is safe to be wrong in.
#[must_use]
pub fn min_move_cost(terrain: &ArenaTerrain) -> u32 {
    terrain
        .cells()
        .iter()
        .filter(|cell| cell.passable())
        .map(|cell| u32::from(cell.move_cost))
        .min()
        .unwrap_or(1)
        .max(1)
}

/// **Why a search produced no path.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFailure {
    /// The start or the goal is off the window, or one of them is impassable.
    OffGround,
    /// Every reachable cell was expanded and the goal was not among them.
    NoRoute,
    /// The node budget ran out first.
    Budget,
}

/// **What one search cost and what it found.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathResult {
    /// The route from the cell after the start to the goal, inclusive. Empty when
    /// the start **is** the goal.
    pub cells: Vec<(u16, u16)>,
    /// How many nodes were expanded — what the budget counts.
    pub expanded: u32,
}

/// The reusable buffers one match's searches share.
///
/// `g` and `stamp` are one entry per cell; `heap` is the open set. `generation`
/// is what makes reuse free (see the module head).
#[derive(Clone, Debug, Default)]
pub struct PathScratch {
    width: u16,
    height: u16,
    g: Vec<u32>,
    from: Vec<u32>,
    stamp: Vec<u32>,
    generation: u32,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
    /// Edges the last search refused by the slope rule (16b cut U.1): see
    /// [`PathScratch::slope_refusals`].
    slope_refused: u32,
}

impl PartialEq for PathScratch {
    /// **Scratch is not state.** Two arenas that differ only in what their last
    /// path search left in these buffers are the same arena, and comparing half a
    /// megabyte of stale `g` scores would make every `PartialEq` on an
    /// [`crate::arena::state::Arena`] a memcmp of the window.
    fn eq(&self, other: &PathScratch) -> bool {
        self.width == other.width && self.height == other.height
    }
}

impl Eq for PathScratch {}

impl PathScratch {
    /// Size the buffers to a window, if they are not already.
    fn fit(&mut self, terrain: &ArenaTerrain) {
        let width = terrain.width();
        let height = terrain.height();
        let cells = usize::from(width).saturating_mul(usize::from(height));
        if self.width != width || self.height != height || self.g.len() != cells {
            self.width = width;
            self.height = height;
            self.g = vec![0; cells];
            self.from = vec![0; cells];
            self.stamp = vec![0; cells];
            self.generation = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        // Generation `0` is "never visited", so a wrap has to skip it — and the
        // stamps have to be cleared once, because a stale `1` from four billion
        // searches ago would otherwise read as fresh.
        if self.generation == 0 {
            self.stamp.iter_mut().for_each(|slot| *slot = 0);
            self.generation = 1;
        }
        self.heap.clear();
        self.slope_refused = 0;
    }

    /// **How many edges the last search refused by the slope rule** (16b cut
    /// U.1; 16c `T-35`): an edge too steep either way for the caller's limit, or
    /// a diagonal the shoulder rule refused. Counted once per refusal as the
    /// search meets it, so an edge met from two expansions is two; reset at the
    /// start of every search, and nought for a search with no limit, which reads
    /// no slope. It is a cost figure for the harness's report and the reflex
    /// controller's [`crate::arena::reflex::ReflexCounters::slope_refusals`], not
    /// state: [`PathScratch`]'s equality ignores it with the rest of the buffers.
    #[must_use]
    pub fn slope_refusals(&self) -> u32 {
        self.slope_refused
    }

    fn at(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        usize::from(y)
            .checked_mul(usize::from(self.width))?
            .checked_add(usize::from(x))
    }

    fn seen(&self, at: usize) -> bool {
        self.stamp.get(at).copied() == Some(self.generation)
    }
}

/// **The bounded A\*** (doc 16 §6): the cheapest route from `start` to `goal` over
/// `terrain`, expanding at most `budget` nodes.
///
/// The step cost is the **entered** cell's `move_cost`, scaled for a diagonal and
/// then surcharged for the slope of the edge (16b cut X.6), so a road is cheap, a
/// climb is dear and rubble is dear exactly as motion's terrain factor makes them
/// (doc 16 §3.4 step 2). An impassable cell (`move_cost == 0`) is never
/// entered and never expanded, which is the same rule motion's step 4 applies.
///
/// This is [`path_with`] with **no limit**, which is the whole of the difference:
/// no edge is refused for being too steep, and no shoulder is read. On level
/// ground the surcharge is the identity, so every flat window answers exactly
/// what it answered before cut X.6; on relief the unlimited search pays the
/// climb it plans over, as it must, or it would promise a route motion prices
/// differently.
/// **Occupancy is not consulted**: a unit standing in a cell is a thing to drive
/// around at the moment of the step, not a permanent hole in the map, and a route
/// recomputed every time somebody parked would be a route recomputed every tick.
///
/// `scale` is the window's cheapest cell ([`min_move_cost`]), the unit the
/// heuristic is measured in. It is an **argument** rather than a scan of its own
/// because the terrain is immutable for the life of a match and the scan is the
/// whole window: `crate::arena::reflex::ReflexMemory` takes it once at check-out
/// and hands it to every search. A `scale` larger than the window's true minimum
/// would make the heuristic inadmissible and the route merely good rather than
/// cheapest; a smaller one only makes the search slower, so `1` is always safe.
///
/// # Errors
///
/// [`PathFailure`], which is the `unreachable` edge's input: `OffGround` for a
/// start or goal that is not passable ground on this window, `NoRoute` when the
/// search exhausted everything it could reach, and `Budget` when it ran out of
/// nodes first. The controller treats all three the same way — fall back to the
/// nearest waypoint — and the distinction is for the report and the tests.
pub fn path(
    terrain: &ArenaTerrain,
    scratch: &mut PathScratch,
    start: (u16, u16),
    goal: (u16, u16),
    budget: u32,
    scale: u32,
) -> Result<PathResult, PathFailure> {
    path_with(terrain, scratch, start, goal, budget, scale, None, &[])
}

/// **The bounded A\* with a slope limit** (16b cut X.6; doc 16 §9 item 6) —
/// [`path`] with the caller's `slope_limit_pct`, which is the whole of the
/// difference.
///
/// The step cost is the entered cell's `move_cost`, then the diagonal factor,
/// then the slope surcharge of the edge ([`crate::arena::slope::effective_cost`]
/// over [`crate::arena::slope::grade_pct`]); and **an edge too steep either way
/// for `slope_limit` is not an edge**, so the search walks round a bluff rather
/// than promising a route over it. That is the same rule
/// [`crate::arena::motion`]'s step 4 applies at the crossing, read out of the
/// same module, because a route the tank is then refused at is worse than no
/// route at all.
///
/// **The shoulder rule.** A diagonal step cuts a corner and motion never
/// occupies either edge-adjacent cell while it does — but a corner both of whose
/// shoulders are a cliff is a corner nothing drives through, and a route that
/// took one would send a tank along the ridge line beside it. So with a limit in
/// force a diagonal is refused when a shoulder that exists and is passable is
/// unreachable from here, or cannot reach the neighbour. A missing or impassable
/// shoulder is **skipped**: the class gap there predates this cut, and closing it
/// would change every route on every window with a wall in it.
///
/// With `slope_limit` `None` neither test runs, and the surcharge is the
/// identity on level ground, so [`path`] is exactly what it was.
///
/// **The `avoid` list** (16b cut Q.7): cells this caller has been *stopped at*
/// and will not try again — [`crate::arena::reflex::ReflexUnit::avoid`], fed by
/// the `Blocked` ledger event's own `Occupied` cause (true since 16b cut M.3,
/// which put the cause on the event and made the event, the pilot's notice and
/// this list one edge of `held`; the tick keeps a cell the unit is still held
/// against fresh). They are impassable to the
/// search exactly as a wall is, and for the same reason the shoulder rule
/// exists: a route that walks the caller back into the cell a parked tank or a
/// wreck is standing in is a route that ends in the same stop. It is **not**
/// occupancy: the search never reads the index, the list is a memory of this
/// unit's own refusals, and the occupant may be a unit this platform has not
/// detected (doc 16 §3.6's observer rule). A goal that is itself on the list is
/// an ordinary [`PathFailure::NoRoute`] — nothing enters it, so nothing closes
/// it — and the `unreachable` edge takes it from there. `&[]` is exactly what
/// the search was before.
///
/// # Errors
///
/// [`PathFailure`], exactly as [`path`].
///
/// The argument list is long because every one of them is a **caller's** fact —
/// the ground, the reusable scratch, the two ends, the two bounds, the unit's
/// slope limit and the cells it has been stopped at — and a struct to carry them
/// would be a struct built and thrown away once per search.
#[allow(clippy::too_many_arguments)]
pub fn path_with(
    terrain: &ArenaTerrain,
    scratch: &mut PathScratch,
    start: (u16, u16),
    goal: (u16, u16),
    budget: u32,
    scale: u32,
    slope_limit: Option<u32>,
    avoid: &[(u16, u16)],
) -> Result<PathResult, PathFailure> {
    // The refusal count is this search's, whichever way it ends: the two early
    // answers below return before `fit`, and a caller that reads
    // `slope_refusals()` after every search (the reflex follower does) would
    // otherwise count the previous search's refusals a second time.
    scratch.slope_refused = 0;
    let passable = |x: u16, y: u16| terrain.get(x, y).is_some_and(|cell| cell.passable());
    let cell_cm = u32::from(terrain.cell_cm());
    if !passable(start.0, start.1) || !passable(goal.0, goal.1) {
        return Err(PathFailure::OffGround);
    }
    if start == goal {
        return Ok(PathResult {
            cells: Vec::new(),
            expanded: 0,
        });
    }
    scratch.fit(terrain);
    let scale = scale.max(1);
    let heuristic = |x: u16, y: u16| -> u32 {
        let dx = u32::from(x.abs_diff(goal.0));
        let dy = u32::from(y.abs_diff(goal.1));
        let (long, short) = if dx >= dy { (dx, dy) } else { (dy, dx) };
        let straight = long.saturating_sub(short).saturating_mul(scale);
        let diagonal = short.saturating_mul(scale).saturating_mul(DIAGONAL_NUM) / DIAGONAL_DEN;
        straight.saturating_add(diagonal)
    };
    let Some(start_at) = scratch.at(start.0, start.1) else {
        return Err(PathFailure::OffGround);
    };
    let Some(goal_at) = scratch.at(goal.0, goal.1) else {
        return Err(PathFailure::OffGround);
    };
    let generation = scratch.generation;
    if let (Some(g), Some(stamp), Some(from)) = (
        scratch.g.get_mut(start_at),
        scratch.stamp.get_mut(start_at),
        scratch.from.get_mut(start_at),
    ) {
        *g = 0;
        *stamp = generation;
        *from = u32::MAX;
    }
    scratch.heap.push(Reverse((
        heuristic(start.0, start.1),
        u32::try_from(start_at).unwrap_or(u32::MAX),
    )));
    let mut expanded = 0_u32;
    let width = scratch.width;
    let mut closed_goal = false;
    while let Some(Reverse((estimate, node))) = scratch.heap.pop() {
        let at = usize::try_from(node).unwrap_or(usize::MAX);
        let Some(here) = scratch.g.get(at).copied() else {
            continue;
        };
        // A stale heap entry: the node was reached again more cheaply after this
        // one was pushed. `g + h` is what was pushed, so a mismatch is the test.
        let (x, y) = (
            u16::try_from(at % usize::from(width.max(1))).unwrap_or(0),
            u16::try_from(at / usize::from(width.max(1))).unwrap_or(0),
        );
        if here.saturating_add(heuristic(x, y)) != estimate {
            continue;
        }
        if at == goal_at {
            closed_goal = true;
            break;
        }
        expanded = expanded.saturating_add(1);
        if expanded > budget {
            return Err(PathFailure::Budget);
        }
        // 16b cut X.6: the expanding node's own cell, read once per expansion and
        // not once per neighbour — it is the `from` of all eight edges.
        let here_cell = terrain.get(x, y);
        for (dx, dy) in NEIGHBOURS {
            let (Some(nx), Some(ny)) = (
                u16::try_from(i32::from(x).saturating_add(dx)).ok(),
                u16::try_from(i32::from(y).saturating_add(dy)).ok(),
            ) else {
                continue;
            };
            let Some(cell) = terrain.get(nx, ny) else {
                continue;
            };
            if !cell.passable() {
                continue;
            }
            // **16b cut Q.7**: a cell this caller was stopped at is not a cell it
            // may plan through. The test is a scan of at most
            // `crate::arena::reflex::REFLEX_AVOID_MAX` pairs, taken after the
            // class test so an ordinary window pays nothing for an empty list.
            if avoid.contains(&(nx, ny)) {
                continue;
            }
            let kind = if dx != 0 && dy != 0 {
                StepKind::Diagonal
            } else {
                StepKind::Orthogonal
            };
            // **16b cut X.6's edge rule**, after the class test and before the
            // cost: an edge too steep either way is not an edge, and a diagonal
            // whose shoulders are a cliff is not a corner anything drives through.
            //
            // Both tests hang off `slope_limit` being `Some`, and deliberately so:
            // `too_steep(None, ..)` refuses nothing, so an unlimited caller that
            // computed the steepness would be computing two slopes an edge at a
            // time to throw them both away. [`path`] is the search the harness
            // times, and it does no slope work here at all.
            let mut grade = 0;
            if let Some(here_cell) = here_cell {
                if let Some(limit) = slope_limit {
                    // 16b cut U.1: each refusal counted, for the report.
                    if slope::too_steep(
                        Some(limit),
                        slope::steepness_pct(here_cell, cell, kind, cell_cm),
                    ) {
                        scratch.slope_refused = scratch.slope_refused.saturating_add(1);
                        continue;
                    }
                    if kind == StepKind::Diagonal
                        && shoulder_refuses(terrain, Some(limit), here_cell, cell, (x, y), (nx, ny))
                    {
                        scratch.slope_refused = scratch.slope_refused.saturating_add(1);
                        continue;
                    }
                }
                grade = slope::grade_pct(here_cell, cell, kind, cell_cm);
            }
            let cost = u32::from(cell.move_cost);
            let step = if dx != 0 && dy != 0 {
                cost.saturating_mul(DIAGONAL_NUM) / DIAGONAL_DEN
            } else {
                cost
            };
            // The surcharge goes on last, in the atlas's own order: the class's
            // cost, the diagonal factor, then the slope.
            let step = slope::effective_cost(step, grade);
            let next = here.saturating_add(step.max(1));
            let Some(neighbour) = scratch.at(nx, ny) else {
                continue;
            };
            let known = scratch.seen(neighbour);
            if known
                && scratch
                    .g
                    .get(neighbour)
                    .copied()
                    .is_some_and(|had| had <= next)
            {
                continue;
            }
            if let (Some(g), Some(stamp), Some(from)) = (
                scratch.g.get_mut(neighbour),
                scratch.stamp.get_mut(neighbour),
                scratch.from.get_mut(neighbour),
            ) {
                *g = next;
                *stamp = generation;
                *from = node;
            }
            scratch.heap.push(Reverse((
                next.saturating_add(heuristic(nx, ny)),
                u32::try_from(neighbour).unwrap_or(u32::MAX),
            )));
        }
    }
    if !closed_goal {
        return Err(PathFailure::NoRoute);
    }
    // Walk the parents back, then reverse: the route the controller follows runs
    // forward, and the start is not on it (the unit is already there).
    let mut cells: Vec<(u16, u16)> = Vec::new();
    let mut cursor = goal_at;
    while cursor != start_at {
        let Some(at) = u16::try_from(cursor % usize::from(width.max(1)))
            .ok()
            .zip(u16::try_from(cursor / usize::from(width.max(1))).ok())
        else {
            break;
        };
        cells.push(at);
        let Some(parent) = scratch.from.get(cursor).copied() else {
            break;
        };
        if parent == u32::MAX {
            break;
        }
        cursor = usize::try_from(parent).unwrap_or(start_at);
        if cells.len() > usize::from(width).saturating_mul(usize::from(scratch.height)) {
            // A parent chain longer than the window is a corrupted scratch, not a
            // route; refuse rather than loop.
            return Err(PathFailure::NoRoute);
        }
    }
    cells.reverse();
    Ok(PathResult { cells, expanded })
}

/// **The shoulder rule** (16b cut X.6): whether either shoulder of a diagonal
/// step refuses it.
///
/// The two shoulders of the corner `here → to` are `(to.0, here.1)` and
/// `(here.0, to.1)`. A shoulder that is off the window or impassable is skipped
/// — motion cuts that corner today and this cut does not close the gap — and one
/// that is there and passable must be reachable from `here` and able to reach
/// `to`, both as **orthogonal** steps, or the corner is a cliff edge and the
/// route goes another way.
///
/// **16b cut Q.7** made it `pub(crate)`: the follower's own one-cell test
/// ([`crate::arena::reflex`]) has to ask the edge question *exactly* as the
/// search asks it, and a second implementation of the shoulder rule beside this
/// one would be two rules with one name.
pub(crate) fn shoulder_refuses(
    terrain: &ArenaTerrain,
    limit: Option<u32>,
    here_cell: &TerrainCell,
    to_cell: &TerrainCell,
    here: (u16, u16),
    to: (u16, u16),
) -> bool {
    let cell_cm = u32::from(terrain.cell_cm());
    for (sx, sy) in [(to.0, here.1), (here.0, to.1)] {
        let Some(shoulder) = terrain.get(sx, sy) else {
            continue;
        };
        if !shoulder.passable() {
            continue;
        }
        if slope::too_steep(
            limit,
            slope::steepness_pct(here_cell, shoulder, StepKind::Orthogonal, cell_cm),
        ) || slope::too_steep(
            limit,
            slope::steepness_pct(shoulder, to_cell, StepKind::Orthogonal, cell_cm),
        ) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::TerrainCell;
    use crate::arena::test_support::ground;

    fn window(width: u16, height: u16, impassable: &[(u16, u16)]) -> ArenaTerrain {
        let mut cells = vec![ground(); usize::from(width) * usize::from(height)];
        for (x, y) in impassable {
            if let Some(slot) =
                cells.get_mut(usize::from(*y) * usize::from(width) + usize::from(*x))
            {
                slot.move_cost = 0;
            }
        }
        ArenaTerrain::new(width, height, 1_000, cells).expect("a test window")
    }

    #[test]
    fn a_clear_window_is_a_straight_diagonal_run() {
        let terrain = window(8, 8, &[]);
        let mut scratch = PathScratch::default();
        let found = path(
            &terrain,
            &mut scratch,
            (0, 0),
            (5, 5),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route");
        assert_eq!(
            found.cells,
            vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)],
            "the cheapest route over uniform ground is the diagonal"
        );
        // The start is never on the route: the unit is already standing there.
        assert!(!found.cells.contains(&(0, 0)));
        assert!(found.expanded > 0);
    }

    #[test]
    fn the_start_cell_is_an_empty_route_and_costs_nothing() {
        let terrain = window(8, 8, &[]);
        let mut scratch = PathScratch::default();
        let found = path(
            &terrain,
            &mut scratch,
            (3, 3),
            (3, 3),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route");
        assert!(found.cells.is_empty());
        assert_eq!(found.expanded, 0);
    }

    #[test]
    fn a_wall_is_walked_round_and_a_sealed_goal_has_no_route() {
        // A wall across the window with one gap at row 0.
        let wall: Vec<(u16, u16)> = (1..8).map(|y| (4_u16, y)).collect();
        let terrain = window(8, 8, &wall);
        let mut scratch = PathScratch::default();
        let found = path(
            &terrain,
            &mut scratch,
            (0, 4),
            (7, 4),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route round it");
        assert!(
            found.cells.contains(&(4, 0)),
            "the only gap in the wall is at row 0: {:?}",
            found.cells
        );
        assert!(found.cells.iter().all(|cell| !wall.contains(cell)));

        // Seal the gap and the goal is unreachable.
        let mut sealed = wall.clone();
        sealed.push((4, 0));
        let terrain = window(8, 8, &sealed);
        assert_eq!(
            path(
                &terrain,
                &mut scratch,
                (0, 4),
                (7, 4),
                4_096,
                min_move_cost(&terrain)
            ),
            Err(PathFailure::NoRoute)
        );
    }

    #[test]
    fn an_impassable_end_is_off_ground_and_a_short_budget_is_a_budget_failure() {
        let terrain = window(8, 8, &[(7, 7)]);
        let mut scratch = PathScratch::default();
        assert_eq!(
            path(
                &terrain,
                &mut scratch,
                (0, 0),
                (7, 7),
                4_096,
                min_move_cost(&terrain)
            ),
            Err(PathFailure::OffGround)
        );
        assert_eq!(
            path(
                &terrain,
                &mut scratch,
                (9, 9),
                (0, 0),
                4_096,
                min_move_cost(&terrain)
            ),
            Err(PathFailure::OffGround)
        );
        // One node of budget cannot cross eight cells.
        let terrain = window(32, 32, &[]);
        assert_eq!(
            path(
                &terrain,
                &mut scratch,
                (0, 0),
                (31, 31),
                1,
                min_move_cost(&terrain)
            ),
            Err(PathFailure::Budget)
        );
    }

    #[test]
    fn a_cheaper_lane_is_preferred_over_the_straight_line() {
        // A window of dear ground with a cheap lane along row 0.
        let mut cells = vec![
            TerrainCell {
                class: mush_atlas::TerrainClass::WoodsHeavy,
                elev_cm: 0,
                move_cost: 400,
            };
            8 * 3
        ];
        for x in 0..8_usize {
            if let Some(slot) = cells.get_mut(x) {
                slot.move_cost = 50;
                slot.class = mush_atlas::TerrainClass::Road;
            }
        }
        let terrain = ArenaTerrain::new(8, 3, 1_000, cells).expect("a lane");
        assert_eq!(min_move_cost(&terrain), 50);
        let mut scratch = PathScratch::default();
        let found = path(
            &terrain,
            &mut scratch,
            (0, 2),
            (7, 2),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route");
        assert!(
            found.cells.iter().filter(|(_, y)| *y == 0).count() >= 5,
            "the road should carry most of the run: {:?}",
            found.cells
        );
    }

    /// **16b cut X.6**: on a level window a limit changes nothing at all — the
    /// surcharge is the identity at grade zero and no edge is ever too steep — so
    /// the limited search must answer cell for cell and node for node what the
    /// unlimited one answers. That is what lets every flat fixture in this crate
    /// stand unedited.
    #[test]
    fn a_limit_changes_no_route_on_level_ground() {
        let wall: Vec<(u16, u16)> = (1..8).map(|y| (4_u16, y)).collect();
        for terrain in [
            window(8, 8, &[]),
            window(8, 8, &wall),
            window(16, 16, &[(8, 8), (8, 9), (8, 10)]),
            window(32, 32, &[]),
        ] {
            let scale = min_move_cost(&terrain);
            let last = (terrain.width() - 1, terrain.height() - 1);
            for (start, goal) in [((0_u16, 0_u16), last), ((0, 4), (7, 4)), ((2, 1), (5, 6))] {
                let mut plain = PathScratch::default();
                let mut limited = PathScratch::default();
                assert_eq!(
                    path(&terrain, &mut plain, start, goal, 65_536, scale),
                    path_with(
                        &terrain,
                        &mut limited,
                        start,
                        goal,
                        65_536,
                        scale,
                        Some(45),
                        &[]
                    ),
                    "{start:?} -> {goal:?} moved under a limit on level ground"
                );
            }
        }
    }

    /// **16b cut Q.7**: an avoided cell is a wall to the search, and an avoided
    /// goal is an ordinary `NoRoute` rather than a special case.
    #[test]
    fn an_avoided_cell_is_walked_round_and_an_avoided_goal_has_no_route() {
        let terrain = window(8, 3, &[]);
        let scale = min_move_cost(&terrain);
        let mut scratch = PathScratch::default();
        // Column 4 of the three rows, all three avoided: the run has to go round
        // the window rather than through it, and on a three-row window there is
        // nowhere to go.
        let sealed = [(4, 0), (4, 1), (4, 2)];
        assert_eq!(
            path_with(
                &terrain,
                &mut scratch,
                (0, 1),
                (7, 1),
                4_096,
                scale,
                None,
                &sealed
            ),
            Err(PathFailure::NoRoute)
        );
        // One avoided cell on the straight line is stepped round, and the route
        // never enters it.
        let found = path_with(
            &terrain,
            &mut scratch,
            (0, 1),
            (7, 1),
            4_096,
            scale,
            None,
            &[(4, 1)],
        )
        .expect("a route round one cell");
        assert!(!found.cells.contains(&(4, 1)));
        assert_eq!(found.cells.last().copied(), Some((7, 1)));
        // And an avoided **goal** is the ordinary failure: nothing enters it, so
        // nothing closes it.
        assert_eq!(
            path_with(
                &terrain,
                &mut scratch,
                (0, 1),
                (7, 1),
                4_096,
                scale,
                None,
                &[(7, 1)]
            ),
            Err(PathFailure::NoRoute)
        );
        // An empty list is the search exactly as it was.
        assert_eq!(
            path_with(
                &terrain,
                &mut scratch,
                (0, 1),
                (7, 1),
                4_096,
                scale,
                None,
                &[]
            ),
            path(
                &terrain,
                &mut PathScratch::default(),
                (0, 1),
                (7, 1),
                4_096,
                scale
            )
        );
    }

    #[test]
    fn the_scratch_is_reused_and_two_searches_agree_with_two_fresh_ones() {
        let terrain = window(16, 16, &[(8, 8), (8, 9), (8, 10)]);
        let mut shared = PathScratch::default();
        let first = path(
            &terrain,
            &mut shared,
            (0, 0),
            (15, 15),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route");
        let second = path(
            &terrain,
            &mut shared,
            (2, 14),
            (14, 2),
            4_096,
            min_move_cost(&terrain),
        )
        .expect("a route");
        let mut fresh = PathScratch::default();
        assert_eq!(
            path(
                &terrain,
                &mut fresh,
                (0, 0),
                (15, 15),
                4_096,
                min_move_cost(&terrain)
            )
            .expect("a route"),
            first
        );
        let mut fresh = PathScratch::default();
        assert_eq!(
            path(
                &terrain,
                &mut fresh,
                (2, 14),
                (14, 2),
                4_096,
                min_move_cost(&terrain)
            )
            .expect("a route"),
            second
        );
    }

    /// **16b cut U.1**: [`PathScratch::slope_refusals`] is the **last** search's
    /// count. A limited search round an 8 m bluff (80 per cent on every
    /// orthogonal rim edge, past a 45 per cent limit) counts refusals; the same
    /// search with no limit counts none; and a search that ends before it looks
    /// at an edge — an impassable end, or a start that is the goal — answers
    /// nought rather than the previous search's figure, which the reflex
    /// follower would otherwise add a second time.
    #[test]
    fn u1_the_slope_refusal_count_is_the_last_searchs_and_an_early_answer_is_nought() {
        let mut cells = vec![ground(); 8 * 8];
        for y in 2..=5_usize {
            if let Some(slot) = cells.get_mut(y * 8 + 4) {
                slot.elev_cm = 800;
            }
        }
        if let Some(slot) = cells.get_mut(7 * 8 + 7) {
            slot.move_cost = 0;
        }
        let terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("a bluff window");
        let scale = min_move_cost(&terrain);
        let mut scratch = PathScratch::default();
        let limited = path_with(
            &terrain,
            &mut scratch,
            (0, 4),
            (7, 4),
            4_096,
            scale,
            Some(45),
            &[],
        )
        .expect("a route round the bluff");
        assert!(
            limited
                .cells
                .iter()
                .all(|&(x, y)| x != 4 || !(2..=5).contains(&y)),
            "the limited route climbed the bluff: {:?}",
            limited.cells
        );
        let refused = scratch.slope_refusals();
        assert!(refused > 0, "the bluff refused no edge");

        // An impassable goal: `OffGround` before the search looks at an edge.
        assert_eq!(
            path_with(
                &terrain,
                &mut scratch,
                (0, 4),
                (7, 7),
                4_096,
                scale,
                Some(45),
                &[]
            ),
            Err(PathFailure::OffGround)
        );
        assert_eq!(
            scratch.slope_refusals(),
            0,
            "an OffGround search kept {refused}"
        );

        // The start is the goal: an empty route, and nothing refused.
        path_with(
            &terrain,
            &mut scratch,
            (0, 4),
            (7, 4),
            4_096,
            scale,
            Some(45),
            &[],
        )
        .expect("the bluff again");
        assert_eq!(scratch.slope_refusals(), refused, "one search, one count");
        path_with(
            &terrain,
            &mut scratch,
            (0, 4),
            (0, 4),
            4_096,
            scale,
            Some(45),
            &[],
        )
        .expect("the start cell");
        assert_eq!(
            scratch.slope_refusals(),
            0,
            "a start-is-goal search kept a count"
        );

        // No limit reads no slope, and refuses nothing.
        path(&terrain, &mut scratch, (0, 4), (7, 4), 4_096, scale).expect("over the bluff");
        assert_eq!(scratch.slope_refusals(), 0);
    }

    /// **16b cut M.4 (16c `T-17`): the shoulder rule is the safe direction.**
    ///
    /// Over several fixed-seed reliefs — whole-metre bumps, so the edges run from
    /// level to past every limit below, with a sprinkling of impassable cells —
    /// and three slope limits, every route [`path_with`] finds is walked step by
    /// step through `crate::arena::motion::step_refusal`, the crossing test motion
    /// itself runs, from a unit standing at the step's start with that limit. Not
    /// one step is refused: the search refuses diagonals motion allows (the
    /// shoulders), never one motion would refuse. Orthogonal steps are held too,
    /// since they are the same edge rule without the shoulders.
    #[test]
    fn m4_every_step_of_a_found_route_is_one_motion_accepts() {
        use crate::arena::index::SpatialIndex;
        use crate::arena::motion::step_refusal;
        use crate::arena::state::Team;
        use crate::arena::test_support::{arena_of, pose, tank};

        /// splitmix64, so the reliefs and the pairs are the same on every run.
        fn next(state: &mut u64) -> u64 {
            *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        const SIDE: u16 = 24;
        let base = arena_of(vec![tank(
            mush_core::ObjId(4),
            Team::Red,
            pose(1_500, 1_500),
        )]);
        let template = base
            .units
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("the fixture tank"));
        let mut routes = 0_u32;
        let mut diagonals = 0_u32;
        let mut state = 0x4d17_u64;
        for _relief in 0..6 {
            let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
            for cell in &mut cells {
                let roll = next(&mut state) % 100;
                if roll < 6 {
                    cell.move_cost = 0;
                }
                cell.elev_cm = i16::try_from(next(&mut state) % 6).unwrap_or(0) * 100;
            }
            let terrain = ArenaTerrain::new(SIDE, SIDE, 1_000, cells).expect("a relief window");
            let index = SpatialIndex::new(&terrain);
            for limit in [20_u32, 35, 60] {
                let mut unit = template.clone();
                unit.slope_limit_pct = Some(limit);
                for _pair in 0..30 {
                    let coord =
                        |state: &mut u64| u16::try_from(next(state) % u64::from(SIDE)).unwrap_or(0);
                    let start = (coord(&mut state), coord(&mut state));
                    let goal = (coord(&mut state), coord(&mut state));
                    let Ok(found) = path_with(
                        &terrain,
                        &mut PathScratch::default(),
                        start,
                        goal,
                        4_096,
                        min_move_cost(&terrain),
                        Some(limit),
                        &[],
                    ) else {
                        continue;
                    };
                    routes += 1;
                    let mut here = start;
                    for &step in &found.cells {
                        if here.0 != step.0 && here.1 != step.1 {
                            diagonals += 1;
                        }
                        assert_eq!(
                            step_refusal(&unit, &index, &terrain, here, step),
                            None,
                            "limit {limit}%: the search took {here:?} -> {step:?} and motion \
                             refuses it"
                        );
                        here = step;
                    }
                }
            }
        }
        // The property is worth asserting: routes were found, and they cut
        // corners.
        assert!(routes > 100, "{routes} routes");
        assert!(diagonals > 100, "{diagonals} diagonal steps");
    }
}
