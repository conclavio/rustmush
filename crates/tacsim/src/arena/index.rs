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
//! # The two cost fixes of 16b cut S.2
//!
//! S.1 measured the loaded pulse and found the cull, not the trace, at the top of
//! it: 300 units each culling 300 candidates, with an `i128::isqrt` per pair. Both
//! remedies are here, and neither changes an answer.
//!
//! 1. **The membership test squares instead of rooting.** `range_cm` floors, so
//!    `floor(sqrt(s)) ≤ r` is exactly `s < (r+1)²` — [`within_cm`] is that, and it
//!    is the one test [`SpatialIndex::in_range`] decides membership with (since
//!    16c `T-107a` written there as `square_cm < reach_bound_sq`, the bound
//!    squared once a query; the same test). [`range_cm`] stays for the places a
//!    distance is *stored* or *shown* — a frame's contact rows
//!    (`crate::arena::Arena::frame_contacts`) and a weapon's range check.
//! 2. **The cell window is preferred whenever the pose is on the window.** The old
//!    rule took the cell branch only while the covered area was no larger than the
//!    unit count, which on a 300-unit arena meant any reach past about eight cells
//!    fell back to a scan of every unit. The window is clamped to the arena either
//!    way, so the cell branch is bounded by the arena's own cell count, and the
//!    scan is now only what an **off-window** pose gets — the case where there is
//!    no cell to centre a window on at all.
//!
//! # The index arithmetic of 16c `T-107a`
//!
//! The VPS profile that opened 16c `T-107` put a tenth of the authored step in
//! [`SpatialIndex::in_range`]'s own body — [`square_between`]'s `i128`
//! saturating chain run once per candidate, with the reach bound squared once
//! per candidate beside it — and the rest of a per cent in the `isqrt` of a
//! [`range_cm`] the perception passes only ever compared. Three things moved,
//! and no answer:
//!
//! 1. **The square is taken in `i64` first.** Every pose on a window is a few
//!    hundred thousand centimetres from every other — about 362,000 at the most
//!    across the 2.56 km the game ships, whose square is under `2^37` — so the
//!    three differences and their squares fit `i64` with some twenty-six bits
//!    to spare; [`square_between`] takes them there with checked arithmetic and
//!    widens the exact `i64`, and falls back to the `i128` saturating body it
//!    always had ([`square_between_wide`]) the moment any step would overflow.
//!    An `i64` that fit is the exact integer and `i128::from` is lossless,
//!    while the wide body saturates only at `2^127`, which no integer that fit
//!    `i64` reaches: the two paths are one function, and
//!    `t107a_the_narrow_square_is_the_wide_square` holds them to it over every
//!    mix of fitting and overflowing differences.
//! 2. **The reach bound is squared once per query** ([`reach_bound_sq`]) rather
//!    than once per candidate, and `in_range`'s `keep` compares the square
//!    against it — the same test, [`within_cm`]'s identity, written once. The
//!    bound itself is taken in `u64` first and widened, the `i128` saturating
//!    two lines kept for a reach of `u32::MAX` or past it, whose bound overflows
//!    `u64`: the perception passes take a bound per reach — a target's own, a
//!    pod's own, a link's own — and the narrow form makes each one add and one
//!    multiply rather than the last `i128` multiply on the candidate path.
//! 3. **The four range compares in the perception passes compare squares.**
//!    `range_cm(a, b) > r` is `square_cm(a, b) ≥ (r+1)²`, the negation of the
//!    identity [`within_cm`] rests on, so detection's reach test and its close
//!    look, a pod's own radius and the mesh's link reach no longer root a
//!    square they only compare ([`square_cm`]). [`range_cm`] stays for the
//!    places a distance is *stored* or *shown* — a frame's contact rows, a
//!    weapon's range check — which this cut does not touch.
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
/// The squares are [`square_between`]'s — in `i64` first, and in `i128` only
/// when a difference or a square overflows it (16c `T-107a`), because a pose is
/// `i64` centimetres and two poses far apart in the region frame would overflow
/// `i64` squared. The answer is `u64`: a distance is a magnitude, and the
/// largest one the 2.56 km window can produce is about 362,000.
#[must_use]
pub fn range_cm(a: &Pose, b: &Pose) -> u64 {
    range_between(a.x_cm, a.y_cm, a.z_cm, b.x_cm, b.y_cm, b.z_cm)
}

/// **The same range, from six coordinates rather than two poses** (16b cut F.3),
/// for the one caller that has no pose to hand: a contact's plot
/// ([`crate::arena::state::SeenPose`]) is four integers and not a pose, because a
/// pose carries the driver's opaque posture blob.
///
/// [`range_cm`] is this function over two poses. One implementation, so a plot's
/// range and a true range cannot be two arithmetics: the root of
/// [`square_between`].
#[must_use]
pub fn range_between(
    ax_cm: i64,
    ay_cm: i64,
    az_cm: i32,
    bx_cm: i64,
    by_cm: i64,
    bz_cm: i32,
) -> u64 {
    let square = square_between(ax_cm, ay_cm, az_cm, bx_cm, by_cm, bz_cm);
    // A sum of squares is never negative, so `unsigned_abs` is the cast and not
    // a policy; `isqrt` floors, which makes the answer a whole centimetre.
    u64::try_from(square.unsigned_abs().isqrt()).unwrap_or(u64::MAX)
}

/// The **squared** separation of two points given as six coordinates, square
/// centimetres (16b cut U.1; 16c `T-24`): the sum [`range_between`] roots.
///
/// **16c `T-107a`: taken in `i64` first.** The three differences and their
/// squares are checked in `i64` — every pair of poses on the 2.56 km window the
/// game ships is far short of the 3,037,000,499 cm a square needs to fit, and a
/// window is `u16` cells of `u16` centimetres, so one *could* be built past it;
/// the fallback is for that one — and the exact `i64` is widened; any `None`
/// falls back to [`square_between_wide`], the
/// `i128` saturating body this function had from cut U.1. The narrow path is
/// exact whenever it answers (`i128::from` is lossless) and the wide one
/// saturates only at `2^127`, which no integer that fit `i64` reaches, so the
/// two are one function and `t107a_the_narrow_square_is_the_wide_square`
/// holds them to one answer over every mix of fitting and overflowing
/// differences.
///
/// Saturating at the region frame's edges for the reason the wide body gives:
/// monotone in the separation, which is what keeps [`within_between`] and
/// [`range_between`] agreeing there.
#[must_use]
pub fn square_between(
    ax_cm: i64,
    ay_cm: i64,
    az_cm: i32,
    bx_cm: i64,
    by_cm: i64,
    bz_cm: i32,
) -> i128 {
    match square_between_narrow(ax_cm, ay_cm, az_cm, bx_cm, by_cm, bz_cm) {
        Some(square) => i128::from(square),
        None => square_between_wide(ax_cm, ay_cm, az_cm, bx_cm, by_cm, bz_cm),
    }
}

/// [`square_between`]'s narrow path (16c `T-107a`): the same sum in `i64`,
/// every step checked, or `None` at the first one that would overflow.
///
/// `dz` is a difference of two `i32`s and cannot overflow `i64`, so it is
/// taken plainly; its square can, and is checked with the rest.
fn square_between_narrow(
    ax_cm: i64,
    ay_cm: i64,
    az_cm: i32,
    bx_cm: i64,
    by_cm: i64,
    bz_cm: i32,
) -> Option<i64> {
    let dx = ax_cm.checked_sub(bx_cm)?;
    let dy = ay_cm.checked_sub(by_cm)?;
    let dz = i64::from(az_cm) - i64::from(bz_cm);
    dx.checked_mul(dx)?
        .checked_add(dy.checked_mul(dy)?)?
        .checked_add(dz.checked_mul(dz)?)
}

/// [`square_between`]'s wide path (16c `T-107a`): the `i128` saturating body
/// the function had from 16b cut U.1, kept as the fallback for a pair of
/// points whose separation overflows `i64` somewhere — two poses at opposite
/// edges of the region frame — and as the reference the narrow path is held
/// to.
///
/// In `i128` because a pose is `i64` centimetres and two poses far apart in the
/// region frame overflow `i64` squared — the narrow path's `None` is exactly
/// that pair. Saturating, so the answer is monotone in the separation even at
/// the edges of the region frame.
#[must_use]
pub fn square_between_wide(
    ax_cm: i64,
    ay_cm: i64,
    az_cm: i32,
    bx_cm: i64,
    by_cm: i64,
    bz_cm: i32,
) -> i128 {
    let dx = i128::from(ax_cm) - i128::from(bx_cm);
    let dy = i128::from(ay_cm) - i128::from(by_cm);
    let dz = i128::from(az_cm) - i128::from(bz_cm);
    dx.saturating_mul(dx)
        .saturating_add(dy.saturating_mul(dy))
        .saturating_add(dz.saturating_mul(dz))
}

/// **The squared range over two poses** (16c `T-107a`): [`square_between`]
/// over their six coordinates, as [`range_cm`] is [`range_between`] — the form
/// a caller takes when it only *compares* a range, against [`reach_bound_sq`]
/// of its reach, and never stores or shows it.
#[must_use]
pub fn square_cm(a: &Pose, b: &Pose) -> i128 {
    square_between(a.x_cm, a.y_cm, a.z_cm, b.x_cm, b.y_cm, b.z_cm)
}

/// **The square a reach is compared against** (16c `T-107a`): `(reach_cm + 1)²`
/// as an `i128` — the right-hand side of the identity [`within_cm`] rests on,
/// `range ≤ r` iff `square < (r+1)²`, and of its negation, `range > r` iff
/// `square ≥ (r+1)²`.
///
/// It was [`within_between`]'s own two lines, an `i128` saturating add and
/// multiply computed once per candidate; [`SpatialIndex::in_range`] now computes
/// it once per query and each of the perception passes' four compares once per
/// reach. **Taken in `u64` first**: the add and the multiply are checked there
/// and the exact `u64` widened — `i128::from` is lossless, and an integer that
/// fit `u64` is one the saturating form never clipped — with the `i128` two
/// lines kept for a reach of `u32::MAX` or past it, whose bound is `2^64` or
/// more. `t107a_the_reach_bound_is_the_inline_bound_it_replaces` holds both
/// paths to the old two lines either side of that boundary and at `u64::MAX`.
///
/// The one place the identity fails is a bound that saturates, which needs
/// `r ≥ 2^63.5`; every reach the arena compares is a `u16` cells by `u16`
/// cell_cm by per-mille product, a `u32` pod radius or a `u32` share reach, none
/// within thirty bits of it.
#[must_use]
pub fn reach_bound_sq(reach_cm: u64) -> i128 {
    match reach_cm
        .checked_add(1)
        .and_then(|bound| bound.checked_mul(bound))
    {
        Some(square) => i128::from(square),
        None => {
            let bound = i128::from(reach_cm).saturating_add(1);
            bound.saturating_mul(bound)
        }
    }
}

/// **Whether two points given as six coordinates are within `reach_cm` of each
/// other** — [`range_between`]'s comparison without its square root (16b cut
/// U.1; 16c `T-24`), the identity [`within_cm`] documents: `range_between(..) ≤
/// r` is exactly `square_between(..) < (r+1)²`.
///
/// [`within_cm`] is this function over two poses, and
/// [`crate::arena::state::SeenPose::within_cm`] is it over a plot and a pose, so
/// the cull and the two per-pulse tempo signals decide membership with one
/// arithmetic. **16c `T-107a`**: the bound is [`reach_bound_sq`], the two lines
/// this function used to compute for itself, so a caller with one reach and
/// many candidates can square it once.
#[must_use]
pub fn within_between(
    ax_cm: i64,
    ay_cm: i64,
    az_cm: i32,
    bx_cm: i64,
    by_cm: i64,
    bz_cm: i32,
    reach_cm: u64,
) -> bool {
    square_between(ax_cm, ay_cm, az_cm, bx_cm, by_cm, bz_cm) < reach_bound_sq(reach_cm)
}

/// **Whether two poses are within `reach_cm` of each other** — [`range_cm`]'s
/// comparison without its square root (16b cut S.2).
///
/// `range_cm` floors, so `range_cm(a, b) ≤ r` is *exactly* `square(a, b) <
/// (r+1)²` ([`square_between`]): `floor(sqrt(s)) ≤ r` iff `s < (r+1)²` for whole `r`. That identity is
/// why this is a cost fix and not a behaviour change, and
/// `the_squared_test_agrees_with_the_rooted_one_at_every_boundary` below pins it
/// over every distance either function can be asked about near a boundary.
///
/// This is the test [`SpatialIndex::in_range`] decides membership with — since
/// 16c `T-107a` written in its `keep` as `square_cm < reach_bound_sq`, the bound
/// squared once a query rather than once a candidate — so a 300-unit cull costs
/// three multiplications a pair instead of an `i128::isqrt`.
#[must_use]
pub fn within_cm(a: &Pose, b: &Pose, reach_cm: u64) -> bool {
    within_between(a.x_cm, a.y_cm, a.z_cm, b.x_cm, b.y_cm, b.z_cm, reach_cm)
}

/// **When [`SpatialIndex::in_range`] walks the unit table instead of the
/// cells**: once the clamped box holds more than this many cells per unit the
/// index holds. A cell costs one slot read and a table row a few compares, and
/// a sensor's reach covers the whole 256 by 256 window — 65,536 slots, a
/// megabyte of reads, for at most 300 units — so a long reach walks the table
/// and a weapon's short one still walks the cells. Cost only: both walks
/// produce one set.
const UNIT_WALK_CELLS_PER_UNIT: usize = 4;

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
    /// Two culls, and **16b cut S.2 made the choice between them a fact about the
    /// pose** rather than a comparison of costs: a pose on the window is answered
    /// by walking the cells the range covers, clamped to the window, and a pose
    /// off it — where there is no cell to centre a window on — by walking the
    /// index's own unit table.
    ///
    /// The old rule preferred the cell walk only while the covered area was no
    /// larger than the unit count, which on a 300-unit arena sent every reach past
    /// about eight cells to a scan of all 300 units; the cell walk reads a flat
    /// row-major slot per cell and is bounded by the arena's own cell count either
    /// way, so it is the cheaper branch wherever it is available.
    ///
    /// **The VPS spike of 2026-10-09** found that bound to be the cost: a sensor's
    /// reach clamps to the whole window, so every sensor pass, share and jamming
    /// query read all 65,536 slots — a fifth of the step on the production host.
    /// A pose on the window now walks the unit table when the clamped box holds
    /// more than `UNIT_WALK_CELLS_PER_UNIT` (four) cells per unit, keeping a row only
    /// when it is inside the box **and** holds its cell, which is the cell walk's
    /// own set under any occupancy (the grid names a unit only at the cell its
    /// row records).
    ///
    /// Both produce the same set — the cull never decides membership,
    /// [`within_cm`] does — and they are the same set because occupancy is one unit
    /// per cell, which check-out and motion both enforce (see
    /// [`SpatialIndex::rebuild`]); a grid slot cannot report a second unit standing
    /// in one cell, and no reachable state has one.
    #[must_use]
    pub fn in_range(&self, pose: &Pose, range_cm: u32, units: &[UnitState]) -> Vec<ObjId> {
        self.in_range_by(pose, range_cm, units, None)
    }

    /// [`SpatialIndex::in_range`] with the on-window walk forced — `Some(true)`
    /// the unit table, `Some(false)` the cells — or chosen by cost (`None`), so
    /// the tests can hold the two walks to one set.
    fn in_range_by(
        &self,
        pose: &Pose,
        range_cm: u32,
        units: &[UnitState],
        by_table: Option<bool>,
    ) -> Vec<ObjId> {
        let mut found: Vec<ObjId> = Vec::new();
        let reach = u64::from(range_cm);
        // **16c `T-107a`**: `within_cm`'s bound squared once for the query rather
        // than once a candidate; `keep` is still exactly `within_cm`.
        let bound_sq = reach_bound_sq(reach);
        let keep = |id: ObjId, found: &mut Vec<ObjId>| {
            if let Ok(at) = units.binary_search_by_key(&id, |unit| unit.id)
                && let Some(unit) = units.get(at)
                && square_cm(pose, &unit.pose) < bound_sq
            {
                found.push(id);
            }
        };
        let span = reach / u64::from(self.cell_cm.max(1)) + 1;
        match self.cell_of_pose(pose) {
            Some((cx, cy)) => {
                let span = u16::try_from(span).unwrap_or(u16::MAX);
                let x0 = cx.saturating_sub(span);
                let y0 = cy.saturating_sub(span);
                let x1 = cx.saturating_add(span).min(self.width.saturating_sub(1));
                let y1 = cy.saturating_add(span).min(self.height.saturating_sub(1));
                let box_cells = (usize::from(x1 - x0) + 1).saturating_mul(usize::from(y1 - y0) + 1);
                if by_table.unwrap_or_else(|| {
                    box_cells > self.of_unit.len().saturating_mul(UNIT_WALK_CELLS_PER_UNIT)
                }) {
                    // The unit table, kept to exactly what the cell walk reads: a
                    // unit inside the box that HOLDS its cell. A slot names a unit
                    // only at the cell its `of_unit` row records (`rebuild` and
                    // `move_unit` are the grid's only writers and keep that), so
                    // this is the cell walk's set whatever the occupancy.
                    for &(id, (x, y)) in &self.of_unit {
                        if (x0..=x1).contains(&x)
                            && (y0..=y1).contains(&y)
                            && self.occupant(x, y) == Some(id)
                        {
                            keep(id, &mut found);
                        }
                    }
                } else {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            if let Some(id) = self.occupant(x, y) {
                                keep(id, &mut found);
                            }
                        }
                    }
                }
                found.sort_unstable();
            }
            // The pose is off the window: there is no cell to centre a window on,
            // and the honest answer is still every unit the range function keeps.
            None => {
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
        // 2,000 cm reaches B exactly and not C. **16b cut S.2 moved the branch**:
        // an on-window pose now always takes the cell walk, clamped to the window,
        // so the on-window assertions exercise that branch and the two off-window
        // ones exercise the unit-table scan. The assertion is the *set*, which is
        // the same either way, and
        // `the_cell_window_cull_answers_what_a_brute_force_scan_answers` is what
        // walks it cell by cell against a brute-force scan.
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

    /// **The VPS spike's unit-table walk** (2026-10-09): every pose on the
    /// window and every reach from nothing to past the window answer one set by
    /// the cells and by the table — in a lawful field, with two units sharing a
    /// cell (the lower id holds it, the higher is in no slot), and after the
    /// higher one has moved off into a held cell and on again — so the cost rule
    /// that picks between them cannot move a candidate.
    #[test]
    fn the_unit_table_walk_answers_the_cell_walk_s_set() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let terrain = &arena.definition.terrain;
        let agree = |index: &SpatialIndex, units: &[UnitState], state: &str| {
            for x in (0..8_000).step_by(700) {
                for y in (0..8_000).step_by(900) {
                    let from = pose(x, y);
                    for reach in [0, 999, 1_000, 2_000, 3_500, 7_071, 12_000, 100_000] {
                        let cells = index.in_range_by(&from, reach, units, Some(false));
                        let table = index.in_range_by(&from, reach, units, Some(true));
                        assert_eq!(cells, table, "{state}: from ({x}, {y}) at {reach} cm");
                        assert_eq!(index.in_range(&from, reach, units), cells, "{state}");
                    }
                }
            }
        };
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        agree(&index, &arena.units, "lawful");
        // B into A's cell: (0, 0) held by A, B recorded there and in no slot.
        let mut shared = arena.units.clone();
        if let Some(unit) = shared.iter_mut().find(|unit| unit.id == B) {
            unit.pose.x_cm = 700;
            unit.pose.y_cm = 300;
        }
        index.rebuild(terrain, &shared);
        assert_eq!(index.occupant(0, 0), Some(A));
        assert_eq!(index.cell_of_unit(B), Some((0, 0)));
        agree(&index, &shared, "shared");
        // B moves into C's held cell (taking nothing), then on to a free one.
        index.move_unit(B, (0, 0), (7, 7));
        agree(&index, &shared, "moved into a held cell");
        index.move_unit(B, (7, 7), (3, 4));
        agree(&index, &shared, "moved on");
    }

    /// **16b cut S.2's first index fix**, pinned as the identity it rests on: the
    /// squared test and the rooted one agree at *every* boundary, so swapping one
    /// for the other inside [`SpatialIndex::in_range`] cannot move a candidate.
    #[test]
    fn the_squared_test_agrees_with_the_rooted_one_at_every_boundary() {
        let origin = pose(0, 0);
        // Every separation from 0 to 400 cm, against every reach from 0 to 400: the
        // two answers are the same test written two ways, including at the exact
        // boundary (`range_cm == reach`) and one either side of it.
        for d in 0..=400_u32 {
            let there = pose(i64::from(d), 0);
            let exact = range_cm(&origin, &there);
            for reach in 0..=400_u64 {
                assert_eq!(
                    within_cm(&origin, &there, reach),
                    exact <= reach,
                    "the squared and rooted tests disagree at d={d} reach={reach}"
                );
            }
        }
        // And off the axes, where the root floors: root 2 metres is 141 cm.
        let diagonal = pose(100, 100);
        assert_eq!(range_cm(&origin, &diagonal), 141);
        assert!(within_cm(&origin, &diagonal, 141));
        assert!(!within_cm(&origin, &diagonal, 140));
        // The three-coordinate case, and the region frame's far edges, where both
        // functions saturate: neither claims a unit is in reach.
        let mut high = pose(300, 400);
        high.z_cm = 1_200;
        assert!(within_cm(&origin, &high, 1_300));
        assert!(!within_cm(&origin, &high, 1_299));
        let far = pose(i64::MAX / 2, 0);
        assert!(!within_cm(
            &pose(i64::MIN / 2, 0),
            &far,
            u64::from(u32::MAX)
        ));
        // **16c `T-107a`**: the saturated side, at the reaches the arena can ask
        // about and far past them. Half the frame apart the square still fits
        // `i128` and roots to `2^63 − 1`; the whole frame apart — and the whole
        // `i32` span of elevation besides — it saturates and roots to
        // `floor(2^63.5)`. Neither pair is within any of these reaches by either
        // test, and the two tests agree at every one.
        let mut low = pose(i64::MIN, 0);
        low.z_cm = i32::MIN;
        let mut high = pose(i64::MAX, 0);
        high.z_cm = i32::MAX;
        assert_eq!(square_cm(&low, &high), i128::MAX);
        assert_eq!(range_cm(&low, &high), 13_043_817_825_332_782_212);
        for (a, b) in [(pose(i64::MIN / 2, 0), far), (low, high)] {
            let exact = range_cm(&a, &b);
            for reach in [0_u64, 1, u64::from(u32::MAX), 1 << 48] {
                assert_eq!(
                    within_cm(&a, &b, reach),
                    exact <= reach,
                    "the saturated side disagrees at reach {reach}"
                );
                assert!(!within_cm(&a, &b, reach));
            }
        }
    }

    /// **16c `T-107a`**: the narrow `i64` square is the wide `i128` one. Over
    /// every combination of the planar coordinates at the values where a
    /// difference or a square first overflows `i64` — 3,037,000,499 is the last
    /// whole number whose square fits — and of the `i32` extremes of elevation,
    /// [`square_between`] answers [`square_between_wide`] to the unit whichever
    /// path it took, both paths are taken, and the pin of 16b cut U.1 is still
    /// what [`range_between`] roots.
    #[test]
    fn t107a_the_narrow_square_is_the_wide_square() {
        const PLANAR: [i64; 21] = [
            0,
            1,
            -1,
            999,
            -999,
            1_000,
            -1_000,
            2_147_483_647,
            -2_147_483_647,
            2_147_483_648,
            -2_147_483_648,
            3_037_000_499,
            -3_037_000_499,
            3_037_000_500,
            -3_037_000_500,
            1 << 40,
            -(1 << 40),
            i64::MIN / 2,
            i64::MAX / 2,
            i64::MIN,
            i64::MAX,
        ];
        const HEIGHT: [i32; 5] = [i32::MIN, -1, 0, 1, i32::MAX];
        // The last square that fits `i64`, and the first that does not.
        assert_eq!(
            3_037_000_499_i64.checked_mul(3_037_000_499),
            Some(9_223_372_030_926_249_001)
        );
        assert_eq!(3_037_000_500_i64.checked_mul(3_037_000_500), None);
        let mut narrow = 0_u64;
        let mut wide = 0_u64;
        for ax in PLANAR {
            for bx in PLANAR {
                for ay in PLANAR {
                    for by in PLANAR {
                        for az in HEIGHT {
                            for bz in HEIGHT {
                                let reference = square_between_wide(ax, ay, az, bx, by, bz);
                                assert_eq!(
                                    square_between(ax, ay, az, bx, by, bz),
                                    reference,
                                    "({ax}, {ay}, {az}) to ({bx}, {by}, {bz})"
                                );
                                match square_between_narrow(ax, ay, az, bx, by, bz) {
                                    Some(square) => {
                                        narrow += 1;
                                        assert_eq!(
                                            i128::from(square),
                                            reference,
                                            "the narrow path at ({ax}, {ay}, {az}) to ({bx}, {by}, {bz})"
                                        );
                                    }
                                    None => wide += 1,
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(narrow + wide, 21_u64.pow(4) * 25);
        assert!(
            narrow > 0 && wide > 0,
            "both paths were walked: {narrow} narrow, {wide} wide"
        );
        // The pose form is the six-coordinate form, and the pin of 16b cut U.1
        // holds through the narrow path.
        let mut high = pose(300, 400);
        high.z_cm = 1_200;
        assert_eq!(square_cm(&pose(0, 0), &high), 1_690_000);
        assert_eq!(square_between(0, 0, 0, 300, 400, 1_200), 1_690_000);
        assert_eq!(range_between(0, 0, 0, 300, 400, 1_200), 1_300);
        assert_eq!(
            square_between_narrow(0, 0, 0, 300, 400, 1_200),
            Some(1_690_000)
        );
        // And a pair the narrow path refuses still roots to the separation.
        assert_eq!(
            square_between_narrow(i64::MIN / 2, 0, 0, i64::MAX / 2, 0, 0),
            None
        );
        assert_eq!(
            range_cm(&pose(i64::MIN / 2, 0), &pose(i64::MAX / 2, 0)),
            9_223_372_036_854_775_807
        );
    }

    /// **16c `T-107a`**: [`reach_bound_sq`] is the two lines [`within_between`]
    /// computed for itself before the cut — on its `u64` path and on its `i128`
    /// fallback, either side of the reach whose bound first overflows `u64`, at
    /// every reach the arena can be asked about and at the one that saturates —
    /// and the index's once-per-query bound keeps exactly what the
    /// per-candidate [`within_cm`] kept.
    #[test]
    fn t107a_the_reach_bound_is_the_inline_bound_it_replaces() {
        // `within_between`'s own bound before 16c `T-107a`, kept as the reference.
        let inline = |reach_cm: u64| -> i128 {
            let bound = i128::from(reach_cm).saturating_add(1);
            bound.saturating_mul(bound)
        };
        // The `u64` path's own boundary: `u32::MAX − 1` is the last reach whose
        // bound fits `u64`, `u32::MAX`'s is exactly `2^64`, and `u64::MAX` has
        // no bound in `u64` at all.
        let narrow = |reach_cm: u64| {
            reach_cm
                .checked_add(1)
                .and_then(|bound| bound.checked_mul(bound))
        };
        assert_eq!(narrow(4_294_967_294), Some(18_446_744_065_119_617_025));
        assert_eq!(narrow(u64::from(u32::MAX)), None);
        assert_eq!(narrow(1 << 48), None);
        assert_eq!(narrow(u64::MAX), None);
        for reach in [
            0_u64,
            1,
            3_000,
            362_000,
            4_294_967_294,
            u64::from(u32::MAX),
            1 << 48,
            u64::MAX - 1,
            u64::MAX,
        ] {
            assert_eq!(reach_bound_sq(reach), inline(reach), "at reach {reach}");
        }
        assert_eq!(reach_bound_sq(0), 1);
        assert_eq!(reach_bound_sq(1), 4);
        assert_eq!(reach_bound_sq(3_000), 9_006_001);
        assert_eq!(reach_bound_sq(4_294_967_294), 18_446_744_065_119_617_025);
        assert_eq!(reach_bound_sq(u64::from(u32::MAX)), 1_i128 << 64);
        assert_eq!(reach_bound_sq(1 << 48), (1_i128 << 96) + (1 << 49) + 1);
        assert_eq!(reach_bound_sq(u64::MAX - 1), i128::MAX);
        assert_eq!(reach_bound_sq(u64::MAX), i128::MAX);
        // The bound squared once answers the per-candidate test at every reach,
        // either way round the 3-4-5 triangle and along an axis.
        let origin = pose(0, 0);
        for reach in 0..=600_u64 {
            let bound = reach_bound_sq(reach);
            for there in [pose(300, 400), pose(-300, 400), pose(500, 0), pose(0, -501)] {
                assert_eq!(
                    square_cm(&origin, &there) < bound,
                    within_cm(&origin, &there, reach),
                    "the once-per-query bound disagrees at reach {reach}"
                );
            }
        }
    }

    /// **16b cut U.1** (16c `T-24`): a plot's squared membership test,
    /// [`crate::arena::state::SeenPose::within_cm`], is `range_from() <= r`
    /// exactly — at every separation from 0 to 400 cm against every reach from
    /// 0 to 400, on the axis and off it in three coordinates — and the strict
    /// form the alert-range signal uses, `r >= 1 && within_cm(r − 1)`, is
    /// `range_from() < r`. The two tempo signals changed arithmetic and no
    /// answer.
    #[test]
    fn a_plots_squared_test_agrees_with_its_rooted_range_at_every_boundary() {
        use crate::arena::state::SeenPose;
        let viewer = pose(0, 0);
        for d in 0..=400_i64 {
            for plot in [
                SeenPose {
                    x_cm: d,
                    ..SeenPose::default()
                },
                SeenPose {
                    x_cm: d,
                    y_cm: d / 2,
                    z_cm: i32::try_from(d / 3).unwrap_or(0),
                    ..SeenPose::default()
                },
            ] {
                let exact = plot.range_from(&viewer);
                for reach in 0..=400_u64 {
                    assert_eq!(
                        plot.within_cm(&viewer, reach),
                        exact <= reach,
                        "within_cm and range_from disagree at {plot:?} reach={reach}"
                    );
                    assert_eq!(
                        reach >= 1 && plot.within_cm(&viewer, reach - 1),
                        exact < reach,
                        "the strict form disagrees at {plot:?} reach={reach}"
                    );
                }
            }
        }
        // And `square_between` is what `range_between` roots.
        assert_eq!(square_between(0, 0, 0, 300, 400, 1_200), 1_690_000);
        assert_eq!(range_between(0, 0, 0, 300, 400, 1_200), 1_300);
    }

    #[test]
    fn the_cell_window_cull_answers_what_a_brute_force_scan_answers() {
        // **The branch production takes.** 16b cut S.2: an on-window pose always
        // takes the cell walk, so this is now the branch of *every* production cull
        // and not only of a short one. Nine units on an 8 by 8 window, including the
        // corner, so the `saturating_sub` and the `min` on the window's bounds are
        // both exercised; the answer is checked against a scan over every unit.
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
