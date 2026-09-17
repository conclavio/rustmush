//! **The spatial index** (16b cut A.2; doc 16 §3.2 "Range", "The spatial index"
//! and "Edges").
//!
//! One per arena: a uniform grid of cell buckets over unit positions, rebuilt
//! every substep (doc 16 §3.1 phase 3), used for occupancy, range culls and
//! in-range pair detection. **Occupancy is one unit per cell** at 10 m cells
//! (doc 16 §3.4 step 5), so a "bucket" is one slot rather than a list, and a
//! wreck or a disabled unit occupies its cell exactly as a live one does — doc 16
//! §3.4's *disabled units stay on the map as obstacles*.
//!
//! # One range function
//!
//! Doc 16 §3.2: *Range is one function used everywhere: Euclidean centimetres
//! between two poses.* That function is [`range_cm`], and everything below —
//! [`SpatialIndex::in_range`], [`SpatialIndex::pairs_within`] — is a **cull** in
//! front of it and never a second answer. The atlas's `Trace` supplies cover and
//! blocking only; its `range_cells` is a different metric and is not a range.
//!
//! [`range_cm`] measures all three coordinates. The cull is planar, which is safe
//! in exactly one direction: a three-dimensional range is never *shorter* than
//! the planar one, so every unit inside `range_cm` is inside the planar cell
//! window the cull walks. A cull that missed candidates would be a silent
//! perception bug; a cull that offers too many is a wasted comparison.
//!
//! # Why it is rebuilt rather than maintained
//!
//! A rebuild is `O(units)` — [`SpatialIndex::rebuild`] clears only the cells the
//! previous rebuild touched — so at 300 units it is a few hundred writes over a
//! buffer that is allocated once, and it cannot drift from the truth the way an
//! incrementally maintained index can. The one incremental operation is
//! [`SpatialIndex::move_unit`], which doc 16 §3.4's occupancy rule *requires*:
//! units integrate in unit-id order inside one substep, so the unit that moves
//! second must see where the unit that moved first went.

use mush_core::ObjId;
use mush_sim::data::Pose;

use crate::arena::state::{ArenaTerrain, UnitState};

/// **The one range function** (doc 16 §3.2 "Range"): Euclidean centimetres
/// between two poses, over all three coordinates, rounded down to a whole
/// centimetre.
///
/// The squares are taken in `i128` because a pose is `i64` centimetres and two
/// poses far apart in the region frame would overflow `i64` squared. The answer
/// is `u64`: a distance is a magnitude, and the largest one the 2.56 km window
/// can produce is about 362,000.
#[must_use]
pub fn range_cm(a: &Pose, b: &Pose) -> u64 {
    let dx = i128::from(a.x_cm) - i128::from(b.x_cm);
    let dy = i128::from(a.y_cm) - i128::from(b.y_cm);
    let dz = i128::from(a.z_cm) - i128::from(b.z_cm);
    let square = dx
        .saturating_mul(dx)
        .saturating_add(dy.saturating_mul(dy))
        .saturating_add(dz.saturating_mul(dz));
    // A sum of squares is never negative, so `unsigned_abs` is the cast and not
    // a policy; `isqrt` floors, which makes the answer a whole centimetre.
    u64::try_from(square.unsigned_abs().isqrt()).unwrap_or(u64::MAX)
}

/// **The uniform grid of cell buckets over unit positions** (doc 16 §3.2 "The
/// spatial index").
///
/// Two tables, both rebuilt together:
///
/// - `occupant`, one slot per cell, row-major over the window — the answer to
///   *may a unit enter this cell* (doc 16 §3.4 step 4);
/// - `of_unit`, the cell each unit stands in, **sorted by unit id** because
///   [`SpatialIndex::rebuild`] is handed [`crate::arena::state::Arena::units`],
///   which is kept in that order. A binary search over a flat vector costs less
///   than a map node per unit and, unlike a hash map, its iteration order is the
///   deterministic one every walk in the arena uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialIndex {
    width: u16,
    height: u16,
    cell_cm: u16,
    occupant: Vec<Option<ObjId>>,
    of_unit: Vec<(ObjId, (u16, u16))>,
}

impl SpatialIndex {
    /// An empty index shaped to `terrain`. The buffer is allocated once here and
    /// reused by every [`SpatialIndex::rebuild`].
    #[must_use]
    pub fn new(terrain: &ArenaTerrain) -> SpatialIndex {
        let cells = usize::from(terrain.width()).saturating_mul(usize::from(terrain.height()));
        SpatialIndex {
            width: terrain.width(),
            height: terrain.height(),
            cell_cm: terrain.cell_cm(),
            occupant: vec![None; cells],
            of_unit: Vec::new(),
        }
    }

    /// **Rebuild from the units' poses**, in unit-id order.
    ///
    /// Only the cells the previous rebuild occupied are cleared, so the cost is
    /// the number of units and not the number of cells: a 256 by 256 window is
    /// 65,536 slots and 300 units, and wiping the whole grid every substep would
    /// be two hundred times the work of the rebuild it precedes.
    ///
    /// A unit whose pose is off the window (which [`crate::arena::state::Arena::new`]
    /// refuses at check-out and motion refuses at every boundary) is recorded
    /// nowhere and occupies nothing.
    ///
    /// **Two units in one cell cannot arise at all**: check-out refuses it
    /// ([`crate::arena::state::ArenaError::CellOccupied`]) and motion refuses it
    /// (doc 16 §3.4 step 4), which is doc 16 §3.4 step 5's one unit per cell. The
    /// rule for it is written down anyway, because a grid with one slot per cell has
    /// to answer *something*: **the lowest id holds the cell**, since the walk is in
    /// id order and the first writer keeps the slot, and the later unit is still
    /// recorded in `of_unit` at the cell it stands in. Nothing in the shipped lane
    /// reaches that state, and [`SpatialIndex::in_range`]'s two culls are equivalent
    /// exactly while it is unreachable.
    pub fn rebuild(&mut self, terrain: &ArenaTerrain, units: &[UnitState]) {
        for (_, (x, y)) in self.of_unit.drain(..) {
            if let Some(slot) = Self::slot_mut(&mut self.occupant, self.width, self.height, x, y) {
                *slot = None;
            }
        }
        self.of_unit.reserve(units.len());
        for unit in units {
            let Some((x, y)) = terrain.cell_of(&unit.pose) else {
                continue;
            };
            self.of_unit.push((unit.id, (x, y)));
            if let Some(slot) = Self::slot_mut(&mut self.occupant, self.width, self.height, x, y)
                && slot.is_none()
            {
                *slot = Some(unit.id);
            }
        }
    }

    /// The unit holding `(x, y)`, or `None` for an empty or off-window cell.
    #[must_use]
    pub fn occupant(&self, x: u16, y: u16) -> Option<ObjId> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::from(y)
            .checked_mul(usize::from(self.width))?
            .checked_add(usize::from(x))?;
        self.occupant.get(index).copied().flatten()
    }

    /// The cell `id` stands in, or `None` when the index does not hold it.
    #[must_use]
    pub fn cell_of_unit(&self, id: ObjId) -> Option<(u16, u16)> {
        let at = self
            .of_unit
            .binary_search_by_key(&id, |(unit, _)| *unit)
            .ok()?;
        self.of_unit.get(at).map(|(_, cell)| *cell)
    }

    /// **Record a move made inside the substep just being integrated** (doc 16
    /// §3.4 step 4's snapshot rule).
    ///
    /// Units integrate in unit-id order from the substep-start snapshot, and
    /// occupancy is the one thing they must read *after* the earlier movers: two
    /// units cannot both enter one cell in one substep, so the lower id wins and
    /// the higher is blocked. That only works if the index the higher id tests
    /// already holds the lower id's new cell — which is what this call does.
    ///
    /// `from` is released only when this unit actually holds it, so a unit that
    /// was standing in a cell another unit holds (see
    /// [`SpatialIndex::rebuild`]) cannot evict that holder by moving away.
    pub fn move_unit(&mut self, id: ObjId, from: (u16, u16), to: (u16, u16)) {
        if self.occupant(from.0, from.1) == Some(id)
            && let Some(slot) =
                Self::slot_mut(&mut self.occupant, self.width, self.height, from.0, from.1)
        {
            *slot = None;
        }
        if let Some(slot) = Self::slot_mut(&mut self.occupant, self.width, self.height, to.0, to.1)
            && slot.is_none()
        {
            *slot = Some(id);
        }
        if let Ok(at) = self.of_unit.binary_search_by_key(&id, |(unit, _)| *unit)
            && let Some(entry) = self.of_unit.get_mut(at)
        {
            entry.1 = to;
        }
    }

    /// **Every unit within `range_cm` of `pose`**, in unit-id order, culled by the
    /// cell buckets the range covers.
    ///
    /// The answer includes a unit standing at `pose` itself: the index does not
    /// know whose pose it was handed, so a caller asking about its own
    /// surroundings filters itself out ([`SpatialIndex::pairs_within`] does).
    ///
    /// Two culls, and the cheaper one is chosen: walking the covered cells costs
    /// the covered **area**, and walking the index's own unit table costs the
    /// **unit count**, so a range that covers more cells than the arena holds
    /// units is answered by the second. Both produce the same set — the cull never
    /// decides membership, [`range_cm`] does — and they are the same set because
    /// occupancy is one unit per cell, which check-out and motion both enforce (see
    /// [`SpatialIndex::rebuild`]); a grid slot cannot report a second unit standing
    /// in one cell, and no reachable state has one.
    #[must_use]
    pub fn in_range(&self, pose: &Pose, range_cm: u32, units: &[UnitState]) -> Vec<ObjId> {
        let mut found: Vec<ObjId> = Vec::new();
        let reach = u64::from(range_cm);
        let keep = |id: ObjId, found: &mut Vec<ObjId>| {
            if let Ok(at) = units.binary_search_by_key(&id, |unit| unit.id)
                && let Some(unit) = units.get(at)
                && self::range_cm(pose, &unit.pose) <= reach
            {
                found.push(id);
            }
        };
        let cell = self.cell_of_pose(pose);
        let span = reach / u64::from(self.cell_cm.max(1)) + 1;
        let covered = span
            .saturating_mul(2)
            .saturating_add(1)
            .saturating_mul(span.saturating_mul(2).saturating_add(1));
        let held = u64::try_from(self.of_unit.len()).unwrap_or(u64::MAX);
        match cell {
            Some((cx, cy)) if covered <= held => {
                let span = u16::try_from(span).unwrap_or(u16::MAX);
                let x0 = cx.saturating_sub(span);
                let y0 = cy.saturating_sub(span);
                let x1 = cx.saturating_add(span).min(self.width.saturating_sub(1));
                let y1 = cy.saturating_add(span).min(self.height.saturating_sub(1));
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if let Some(id) = self.occupant(x, y) {
                            keep(id, &mut found);
                        }
                    }
                }
                found.sort_unstable();
            }
            // Either the pose is off the window — there is no cell to centre a
            // window on — or the range covers more cells than there are units.
            _ => {
                for (id, _) in &self.of_unit {
                    keep(*id, &mut found);
                }
            }
        }
        found
    }

    /// **Every pair of distinct units within `range_cm` of each other**, each pair
    /// once as `(lower id, higher id)` and the whole list in that order.
    ///
    /// The shape doc 16 §3.1's tempo table wants for *hostile units are within
    /// weapons range of each other*; cut W is its first caller. The cost is one
    /// [`SpatialIndex::in_range`] per unit, so it is the caller's business to keep
    /// `range_cm` a weapon's reach rather than the window's diagonal.
    #[must_use]
    pub fn pairs_within(&self, range_cm: u32, units: &[UnitState]) -> Vec<(ObjId, ObjId)> {
        let mut pairs: Vec<(ObjId, ObjId)> = Vec::new();
        for unit in units {
            for other in self.in_range(&unit.pose, range_cm, units) {
                if other > unit.id {
                    pairs.push((unit.id, other));
                }
            }
        }
        pairs
    }

    /// How many units the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.of_unit.len()
    }

    /// Whether the index holds no unit at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.of_unit.is_empty()
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

    /// The cell a pose stands in, by the index's own copy of the window's shape —
    /// the same floor division [`ArenaTerrain::cell_of`] makes, so the index and
    /// the terrain never disagree about where a unit is.
    fn cell_of_pose(&self, pose: &Pose) -> Option<(u16, u16)> {
        if pose.x_cm < 0 || pose.y_cm < 0 || self.cell_cm == 0 {
            return None;
        }
        let side = i64::from(self.cell_cm);
        let x = u16::try_from(pose.x_cm / side).ok()?;
        let y = u16::try_from(pose.y_cm / side).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((x, y))
    }

    /// The occupancy slot of `(x, y)`, or `None` off the window. An associated
    /// function rather than a method so a caller already holding `&mut self.of_unit`
    /// can reach the grid.
    fn slot_mut(
        occupant: &mut [Option<ObjId>],
        width: u16,
        height: u16,
        x: u16,
        y: u16,
    ) -> Option<&mut Option<ObjId>> {
        if x >= width || y >= height {
            return None;
        }
        let index = usize::from(y)
            .checked_mul(usize::from(width))?
            .checked_add(usize::from(x))?;
        occupant.get_mut(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, pose, tank};

    const A: ObjId = ObjId(4);
    const B: ObjId = ObjId(9);
    const C: ObjId = ObjId(11);

    #[test]
    fn range_is_euclidean_centimetres_over_three_coordinates() {
        // The 3-4-5 triangle, in centimetres and then in metres.
        assert_eq!(range_cm(&pose(0, 0), &pose(300, 400)), 500);
        assert_eq!(range_cm(&pose(0, 0), &pose(-300, -400)), 500);
        assert_eq!(range_cm(&pose(1_000, 1_000), &pose(1_000, 1_000)), 0);
        // It floors rather than rounds: root 2 metres is 141 cm, not 142.
        assert_eq!(range_cm(&pose(0, 0), &pose(100, 100)), 141);
        // Elevation counts (doc 16 §3.2: between two *poses*).
        let mut high = pose(300, 400);
        high.z_cm = 1_200;
        assert_eq!(range_cm(&pose(0, 0), &high), 1_300);
        // Symmetric, and the whole `i64` span of the region frame apart without
        // overflowing: the squares are taken in `i128`, so the answer is the
        // separation itself and not a saturated one.
        assert_eq!(range_cm(&high, &pose(0, 0)), 1_300);
        assert_eq!(
            range_cm(&pose(i64::MIN / 2, 0), &pose(i64::MAX / 2, 0)),
            9_223_372_036_854_775_807
        );
    }

    #[test]
    fn a_rebuild_places_every_unit_and_clears_the_last_one() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(1_500, 1_500)),
            tank(B, Team::Blue, pose(6_500, 2_500)),
        ]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        assert!(index.is_empty());
        index.rebuild(&arena.definition.terrain, &arena.units);
        assert_eq!(index.len(), 2);
        assert_eq!(index.occupant(1, 1), Some(A));
        assert_eq!(index.occupant(6, 2), Some(B));
        assert_eq!(index.cell_of_unit(A), Some((1, 1)));
        assert_eq!(index.cell_of_unit(B), Some((6, 2)));
        assert!(index.cell_of_unit(C).is_none());
        // Empty cells, and cells off the window.
        assert!(index.occupant(0, 0).is_none());
        assert!(index.occupant(8, 0).is_none());
        assert!(index.occupant(0, 8).is_none());

        // A second rebuild from moved poses leaves no ghost behind.
        let moved = arena_of(vec![
            tank(A, Team::Red, pose(3_500, 3_500)),
            tank(B, Team::Blue, pose(6_500, 2_500)),
        ]);
        index.rebuild(&moved.definition.terrain, &moved.units);
        assert_eq!(index.len(), 2);
        assert!(
            index.occupant(1, 1).is_none(),
            "the old cell was not cleared"
        );
        assert_eq!(index.occupant(3, 3), Some(A));
    }

    #[test]
    fn the_lowest_id_holds_a_cell_two_units_somehow_share() {
        // A state the shipped lane cannot reach — check-out refuses it
        // (`ArenaError::CellOccupied`) and motion refuses it — so it is built by
        // moving a pose after the fact, which is the only way in. The rule is
        // asserted because a grid with one slot per cell has to answer something.
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(2_100, 2_900)),
            tank(B, Team::Blue, pose(6_500, 6_500)),
        ]);
        let mut units = arena.units.clone();
        if let Some(unit) = units.iter_mut().find(|unit| unit.id == B) {
            unit.pose.x_cm = 2_500;
            unit.pose.y_cm = 2_500;
        }
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &units);
        // Both stand in cell (2, 2); the lower id holds it and both are located.
        assert_eq!(index.occupant(2, 2), Some(A));
        assert_eq!(index.cell_of_unit(A), Some((2, 2)));
        assert_eq!(index.cell_of_unit(B), Some((2, 2)));
        assert_eq!(index.len(), 2);
        // And the higher id moving away cannot evict the holder.
        index.move_unit(B, (2, 2), (3, 3));
        assert_eq!(index.occupant(2, 2), Some(A));
        assert_eq!(index.occupant(3, 3), Some(B));
    }

    #[test]
    fn a_unit_off_the_window_occupies_nothing() {
        let arena = arena_of(vec![tank(A, Team::Red, pose(1_500, 1_500))]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        let mut units = arena.units.clone();
        if let Some(unit) = units.first_mut() {
            unit.pose.x_cm = -5_000;
        }
        index.rebuild(&arena.definition.terrain, &units);
        assert!(index.is_empty());
        assert!(index.cell_of_unit(A).is_none());
    }

    #[test]
    fn move_unit_releases_the_old_cell_and_takes_the_new_one() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(1_500, 1_500)),
            tank(B, Team::Blue, pose(6_500, 2_500)),
        ]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        index.move_unit(A, (1, 1), (2, 1));
        assert!(index.occupant(1, 1).is_none());
        assert_eq!(index.occupant(2, 1), Some(A));
        assert_eq!(index.cell_of_unit(A), Some((2, 1)));
        // A move into an occupied cell does not steal it; the caller is expected
        // to have refused the move (doc 16 §3.4 step 4).
        index.move_unit(A, (2, 1), (6, 2));
        assert_eq!(index.occupant(6, 2), Some(B));
        // A move off the window releases the old cell and takes nothing.
        index.move_unit(B, (6, 2), (99, 99));
        assert!(index.occupant(6, 2).is_none());
    }

    #[test]
    fn in_range_answers_the_same_set_by_either_cull() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        let from = pose(500, 500);
        // 2,000 cm reaches B exactly and not C. A reach of 2,000 over 1,000 cm cells
        // is a span of 3, so the cell cull would walk a 7 by 7 window — more cells
        // than this arena has units — and the unit-table branch is taken instead.
        // The assertion is the *set*, and
        // `the_cell_window_cull_answers_what_a_brute_force_scan_answers` is what
        // covers the other branch.
        assert_eq!(index.in_range(&from, 2_000, &arena.units), vec![A, B]);
        assert_eq!(index.in_range(&from, 1_999, &arena.units), vec![A]);
        // Off the window there is no cell to centre the cull on, and the answer is
        // still the honest set: the range function decides, not the grid.
        assert_eq!(
            index.in_range(&pose(-1_000, -1_000), 3_000, &arena.units),
            vec![A]
        );
        assert_eq!(
            index.in_range(&pose(-1_000, -1_000), 2_000, &arena.units),
            Vec::<ObjId>::new()
        );
        // A range that covers the whole window finds everything, in id order.
        assert_eq!(index.in_range(&from, 100_000, &arena.units), vec![A, B, C]);
    }

    #[test]
    fn the_cell_window_cull_answers_what_a_brute_force_scan_answers() {
        // **The branch production takes.** The cell window is walked only when it
        // covers no more cells than the arena holds units — `(2·span+1)² ≤ units` —
        // so a reach inside one cell (span 1, nine cells) needs nine units, and the
        // 300-unit arena of the harness takes this branch for every sensor range
        // under three cells. Nine units on an 8 by 8 window, including the corner,
        // so the `saturating_sub` and the `min` on the window's bounds are both
        // exercised; the answer is checked against a scan over every unit.
        //
        // The poses are deliberately **not** all cell centres: two neighbours sit
        // close to the shared boundary, so a reach under one cell really does have
        // to look into the next cell and the cull is doing work.
        let placements: [(i64, i64); 9] = [
            (500, 500),     // cell (0, 0)
            (1_100, 500),   // cell (1, 0), 600 cm away
            (500, 1_400),   // cell (0, 1), 900 cm away
            (1_600, 1_600), // cell (1, 1), 1,556 cm away
            (2_500, 1_500), // cell (2, 1)
            (1_500, 2_500), // cell (1, 2)
            (4_500, 4_500), // cell (4, 4)
            (7_900, 7_900), // cell (7, 7): the far corner
            (7_100, 100),   // cell (7, 0)
        ];
        let units: Vec<crate::arena::state::CustodyUnit> = placements
            .iter()
            .enumerate()
            .map(|(n, (x_cm, y_cm))| {
                let id = ObjId(10 + u32::try_from(n).unwrap_or(0));
                let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
                tank(id, team, pose(*x_cm, *y_cm))
            })
            .collect();
        let arena = arena_of(units);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        assert_eq!(index.len(), 9);

        let brute = |from: &Pose, reach: u32| -> Vec<ObjId> {
            let mut found: Vec<ObjId> = arena
                .units
                .iter()
                .filter(|unit| range_cm(from, &unit.pose) <= u64::from(reach))
                .map(|unit| unit.id)
                .collect();
            found.sort_unstable();
            found
        };
        // Every unit's own surroundings and the window's four corners, at five
        // reaches inside one cell — `span = reach / 1,000 + 1 = 1`, so nine cells
        // against nine units, and the cell branch is the one taken.
        let mut comparisons = 0_usize;
        for reach in [0_u32, 1, 500, 900, 999] {
            for from in arena.units.iter().map(|unit| unit.pose.clone()).chain([
                pose(500, 500),
                pose(7_500, 500),
                pose(500, 7_500),
                pose(7_500, 7_500),
            ]) {
                assert_eq!(
                    index.in_range(&from, reach, &arena.units),
                    brute(&from, reach),
                    "the cell cull and a full scan disagree at {reach} cm from ({}, {})",
                    from.x_cm,
                    from.y_cm
                );
                comparisons += 1;
            }
        }
        assert_eq!(comparisons, 65);
        // And the answers are not vacuous: from the first unit, 900 cm reaches the
        // two neighbours across a cell boundary and not the diagonal one at 1,556.
        assert_eq!(
            index.in_range(&pose(500, 500), 900, &arena.units),
            vec![ObjId(10), ObjId(11), ObjId(12)],
            "the cell window answered the wrong neighbours"
        );
        assert_eq!(
            index.in_range(&pose(500, 500), 599, &arena.units),
            vec![ObjId(10)]
        );
    }

    #[test]
    fn in_range_is_culled_but_never_decided_by_the_grid() {
        // Two units three cells apart on the diagonal: the cell window covers
        // them, and only `range_cm` refuses the corner.
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(3_500, 3_500)),
        ]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        let exact = range_cm(&pose(500, 500), &pose(3_500, 3_500));
        assert_eq!(exact, 4_242);
        assert_eq!(
            index.in_range(&pose(500, 500), 4_242, &arena.units),
            vec![A, B]
        );
        assert_eq!(
            index.in_range(&pose(500, 500), 4_241, &arena.units),
            vec![A],
            "the grid offered the candidate and the range function refused it"
        );
    }

    #[test]
    fn pairs_within_answers_each_pair_once_in_id_order() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(1_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        assert_eq!(index.pairs_within(1_500, &arena.units), vec![(A, B)]);
        assert_eq!(index.pairs_within(100, &arena.units), Vec::new());
        assert_eq!(
            index.pairs_within(100_000, &arena.units),
            vec![(A, B), (A, C), (B, C)]
        );
    }
}
