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
//! # The rows of 16c `T-107b`
//!
//! The annotate at the hash v2 tree put a quarter of `in_range`'s twelve per
//! cent in the per-candidate search — the branchless select over the unit
//! table's 1,128-byte rows that `keep` made to find a candidate's pose — and
//! `T-107a`'s perf lens noted a second search of the same depth per candidate
//! in the perception passes, where `Arena::unit` found the unit `keep` had just
//! found. Both searches go, and no answer moves:
//!
//! 1. **The index's unit table carries each unit's row.** `IndexRow` is the
//!    id, the cell and the unit's position in the slice [`SpatialIndex::rebuild`]
//!    was handed; `rebuild` writes it from `units.iter().enumerate()`, which costs
//!    nothing it did not already walk.
//! 2. **The cull hands back candidates with their rows.**
//!    [`SpatialIndex::in_range_rows`] answers [`Candidate`]s — the id, the row
//!    and the cell — and [`SpatialIndex::in_range`] is it mapped to the ids, so
//!    [`SpatialIndex::pairs_within`] and every test of the set compile and
//!    answer unchanged.
//! 3. **`keep` reads the row first and searches only on a miss.** The row names
//!    the unit when `units[row].id == id`; a slice that is not the one the index
//!    was rebuilt over — a test's `retain`, an appended unit — misses, and
//!    `keep` then runs the `binary_search_by_key` it always ran. What the
//!    verified read promises is exactly this: it finds **the one element with
//!    that id**, which is the element the search finds whenever the search can
//!    find one — on an id-sorted, duplicate-free slice, always. The two part
//!    only over an unsorted slice, where the search is promised nothing and
//!    the hint may find what it misses (the rotated half of
//!    `t107b_the_table_walk_pushes_in_id_order` is that case, pinned); no such
//!    slice reaches the cull — [`crate::arena::state::Arena::new`] sorts the
//!    table and refuses duplicates, nothing reorders it, and a snapshot decodes
//!    the table it encoded. The candidate's row is the found element's row **in
//!    the slice the query was handed**, whichever path found it.
//! 4. **The perception passes read a candidate's unit by row** —
//!    `crate::arena::state::Arena::unit_row`, the same verified read with
//!    `Arena::unit` as its fallback — where they looked the id up a second time.
//!
//! The grid does not change shape: a slot still names an id, and the cell walk
//! takes a candidate's row off the index's own table by the search
//! [`SpatialIndex::cell_of_unit`] makes, over twelve-byte rows rather than
//! kilobyte ones. The on-window `sort_unstable` is **kept**, and it is two
//! facts and not one: the table walk pushes in the table's order, which on the
//! production slice is id order already, so there the sort is the identity
//! (`t107b_the_table_walk_pushes_in_id_order`); the cell walk pushes in
//! row-major cell order, which is no id order on any slice, so for it the sort
//! is what makes the answer id-ordered — and a pod's jamming scan at a one-cell
//! radius takes that walk. Dropping the sort would move the cell walk's order
//! everywhere and the table walk's over any unsorted slice. The snapshot
//! re-derives the index over the units it decodes, so no row is carried and the
//! format does not move.
//!
//! # The sliced slots of 16c `T-107c`
//!
//! What T-107a's annotate left to this cut: the cell walk's slot reads and the
//! table walk's box check. Two things moved, and no answer:
//!
//! 1. **The cell walk reads each row of the box as one slice.** The box is
//!    `(x0..=x1) × (y0..=y1)`, clamped to the window, and the grid is row-major
//!    with the **width** as its stride, so row `y`'s cells are the slots
//!    `y·width + x0 ..= y·width + x1` in x-ascending order — exactly the slots
//!    [`SpatialIndex::occupant`] reads one bounds check at a time. The walk
//!    takes them as one slice (`SpatialIndex::row_slots`) and feeds the same
//!    ids to the same `keep` in the same order. The slice is **always there by
//!    the clamps** — `y ≤ height − 1` and `x1 ≤ width − 1` put its last slot at
//!    `width·height − 1` at the most, the buffer's last — but the walk does not
//!    rest on that: a row whose slice is not there falls back to the per-cell
//!    `occupant` reads, and is never skipped
//!    (`t107c_a_row_the_buffer_does_not_hold_falls_back_to_the_cells` holds a
//!    truncated buffer to the old walk). A pod's one-cell jamming scan and every
//!    small-reach cull take this branch.
//! 2. **The table walk's box check is two compares, not four.** `x0 ≤ x ≤ x1`
//!    is `x − x0 ≤ x1 − x0` in wrapping `u16` arithmetic (`within_span`),
//!    exact for every `u16` whenever `x0 ≤ x1`, which the clamps give; the two
//!    spans are hoisted out of the loop over the table's rows. The order of the
//!    rows and the set they keep are untouched, and the occupant test after the
//!    box check is the one it was.
//!
//! No RNG, hash, ledger, frame or counter reads the walk's shape, so nothing
//! downstream can move; `t107c_the_cell_walk_on_a_non_square_window_answers_the_brute_force_scan`
//! holds the walk to a brute-force scan on a 12 by 5 and a 5 by 12 window,
//! where a `y·height` stride — the slip no square window would catch — would
//! read the wrong row.
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

/// **Whether `v` lies in `lo..=lo + span`**, as one unsigned compare (16c
/// `T-107c`): the table walk's box check, `(x0..=x1).contains(&x)`, written as
/// `x − x0 ≤ x1 − x0` with the span hoisted out of the loop over the rows.
///
/// Exact for every `u16` `v` whenever `lo + span` does not overflow, which a
/// span taken as `hi − lo` for `lo ≤ hi` guarantees: for `v ≥ lo` the wrapping
/// difference is the true one and `v − lo ≤ hi − lo` is `v ≤ hi`; for `v < lo`
/// it wraps to `v − lo + 65,536 > 65,535 − lo ≥ hi − lo`, and the test is
/// false as `v < lo` wants. `t107c_the_span_test_is_the_inclusive_range_test`
/// holds it to the range's own `contains` across every boundary and at the
/// `u16` extremes.
#[inline]
const fn within_span(v: u16, lo: u16, span: u16) -> bool {
    v.wrapping_sub(lo) <= span
}

/// **One row of the index's unit table** (16c `T-107b`): a unit's id, the
/// cell it stands in, and its **row** — its position in the slice
/// [`SpatialIndex::rebuild`] was handed, which on the production slice
/// ([`crate::arena::state::Arena::units`], laid down in id order and never
/// reordered) is where [`crate::arena::state::Arena::unit`]'s binary search
/// finds it. Twelve bytes, so the table of a 300-unit match is under four
/// kilobytes and stays in cache across a pass.
///
/// `row` is a hint and not a promise: the cull's `keep`
/// (`SpatialIndex::in_range_rows_by`'s, the one body every walk shares)
/// verifies it against the slice it is handed (`units[row].id == id`) and
/// searches on a miss, so a slice that is not the rebuilt one still answers
/// exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexRow {
    id: ObjId,
    cell: (u16, u16),
    row: u32,
}

/// **A unit the cull offers, with where to find it** (16c `T-107b`): the
/// answer of [`SpatialIndex::in_range_rows`], one per unit within reach.
///
/// `row` is the unit's position **in the `units` slice the query was handed**
/// — verified there by `keep`, whichever path found it — so a caller holding
/// that slice reads `units[row]` and checks `id` where it used to search;
/// `crate::arena::state::Arena::unit_row` is that read with `Arena::unit` as
/// its fallback. `cell` is the cell the index records the unit in, the one its
/// slot names it at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The unit.
    pub id: ObjId,
    /// Its row in the slice the query was handed (see the type).
    pub row: u32,
    /// The cell the index holds it in.
    pub cell: (u16, u16),
}

/// **The uniform grid of cell buckets over unit positions** (doc 16 §3.2 "The
/// spatial index").
///
/// Two tables, both rebuilt together:
///
/// - `occupant`, one slot per cell, row-major over the window — the answer to
///   *may a unit enter this cell* (doc 16 §3.4 step 4);
/// - `of_unit`, the cell each unit stands in and its row in the slice the
///   rebuild was handed (`IndexRow`; 16c `T-107b`), **sorted by unit id**
///   because [`SpatialIndex::rebuild`] is handed
///   [`crate::arena::state::Arena::units`], which is kept in that order. A
///   binary search over a flat vector costs less than a map node per unit and,
///   unlike a hash map, its iteration order is the deterministic one every walk
///   in the arena uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialIndex {
    width: u16,
    height: u16,
    cell_cm: u16,
    occupant: Vec<Option<ObjId>>,
    of_unit: Vec<IndexRow>,
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
    ///
    /// **16c `T-107b`: each row records the unit's position in `units`.** The
    /// walk is `enumerate()`d and the position written beside the id and the
    /// cell (`IndexRow::row`), so a cull can hand a candidate back with where
    /// to find it and the perception passes can read the unit by row instead of
    /// searching for it. On the production slice the row is the binary search's
    /// answer; on any other it is a hint the cull's `keep`
    /// (`SpatialIndex::in_range_rows_by`'s) verifies before it trusts (see the
    /// module head).
    pub fn rebuild(&mut self, terrain: &ArenaTerrain, units: &[UnitState]) {
        for IndexRow { cell: (x, y), .. } in self.of_unit.drain(..) {
            if let Some(slot) = Self::slot_mut(&mut self.occupant, self.width, self.height, x, y) {
                *slot = None;
            }
        }
        self.of_unit.reserve(units.len());
        for (row, unit) in units.iter().enumerate() {
            let Some((x, y)) = terrain.cell_of(&unit.pose) else {
                continue;
            };
            self.of_unit.push(IndexRow {
                id: unit.id,
                cell: (x, y),
                // A slice past `u32::MAX` rows has no row to record; `keep`'s
                // verified read misses it and searches, as it always did.
                row: u32::try_from(row).unwrap_or(u32::MAX),
            });
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

    /// **The slots of cells `(x0..=x1, y)` as one slice** (16c `T-107c`): the
    /// `x1 − x0 + 1` slots [`SpatialIndex::occupant`] would read one at a time
    /// for those cells, in x-ascending order — the grid is row-major with the
    /// **width** as its stride (`occupant` reads slot `y·width + x`, and so does
    /// this), so row `y`'s cells `x0..=x1` are the contiguous slots
    /// `y·width + x0 ..= y·width + x1`.
    ///
    /// `None` when the row is not wholly on the window (`y ≥ height` or `x1 ≥
    /// width`: a slice across a row's end would hold the next row's cells) or
    /// the buffer does not hold it; the cell walk then reads the row's cells one
    /// by one, which answer what `occupant` always has — `None` for a cell off
    /// the window or past the buffer's end, the slot's id for a cell whose slot
    /// the buffer still holds
    /// (`t107c_a_row_the_buffer_does_not_hold_falls_back_to_the_cells` finds
    /// the neighbour of a slot that is gone through exactly those reads). A
    /// `None` slice is not an empty row. For `x0 > x1` nothing is offered
    /// either way: the slice is empty or `None`, and the per-cell range is
    /// empty.
    fn row_slots(&self, y: u16, x0: u16, x1: u16) -> Option<&[Option<ObjId>]> {
        if y >= self.height || x1 >= self.width {
            return None;
        }
        let base = usize::from(y).checked_mul(usize::from(self.width))?;
        let first = base.checked_add(usize::from(x0))?;
        let last = base.checked_add(usize::from(x1))?;
        self.occupant.get(first..=last)
    }

    /// The cell `id` stands in, or `None` when the index does not hold it.
    #[must_use]
    pub fn cell_of_unit(&self, id: ObjId) -> Option<(u16, u16)> {
        self.row_of(id).map(|row| row.cell)
    }

    /// The unit table's row for `id` (16c `T-107b`), by the binary search over
    /// the table [`SpatialIndex::cell_of_unit`] has always made, or `None` when
    /// the index does not hold it.
    fn row_of(&self, id: ObjId) -> Option<&IndexRow> {
        let at = self.of_unit.binary_search_by_key(&id, |row| row.id).ok()?;
        self.of_unit.get(at)
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
        if let Ok(at) = self.of_unit.binary_search_by_key(&id, |row| row.id)
            && let Some(entry) = self.of_unit.get_mut(at)
        {
            entry.cell = to;
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
    ///
    /// **16c `T-107b`**: the ids of [`SpatialIndex::in_range_rows`], which is
    /// where the walks now live; a caller that goes on to read each unit takes
    /// that one and reads by row. **16c `T-107c`**: the cell walk reads each
    /// row of the box as one slice of the grid (`SpatialIndex::row_slots`,
    /// with the per-cell reads as its fallback) and the table walk's box check
    /// is `within_span` an axis; the module head has both.
    #[must_use]
    pub fn in_range(&self, pose: &Pose, range_cm: u32, units: &[UnitState]) -> Vec<ObjId> {
        self.in_range_by(pose, range_cm, units, None)
    }

    /// **[`SpatialIndex::in_range`] with each unit's row** (16c `T-107b`): the
    /// same set in the same order, each unit as a [`Candidate`] — its id, its
    /// row in `units` and its cell — so the perception passes read the unit
    /// they are about by row (`crate::arena::state::Arena::unit_row`) instead
    /// of searching the unit table a second time for the unit `keep` just found.
    ///
    /// The row is **verified against `units` before it is handed back**: `keep`
    /// reads `units[row]` and checks the id, and on a miss — a slice that is not
    /// the one the index was rebuilt over — runs the binary search it ran before
    /// this cut, so the answer is the search's whichever path found it, and the
    /// row is the found element's position in `units`
    /// (`t107b_a_foreign_slice_takes_the_fallback`). The cell walk takes a row
    /// off the index's own table (`SpatialIndex::row_of`) for the id a slot
    /// names and verifies it the same way.
    #[must_use]
    pub fn in_range_rows(&self, pose: &Pose, range_cm: u32, units: &[UnitState]) -> Vec<Candidate> {
        self.in_range_rows_by(pose, range_cm, units, None)
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
        self.in_range_rows_by(pose, range_cm, units, by_table)
            .into_iter()
            .map(|candidate| candidate.id)
            .collect()
    }

    /// [`SpatialIndex::in_range_rows`] with the on-window walk forced, as
    /// [`SpatialIndex::in_range_by`] forces it: the one body both culls share.
    fn in_range_rows_by(
        &self,
        pose: &Pose,
        range_cm: u32,
        units: &[UnitState],
        by_table: Option<bool>,
    ) -> Vec<Candidate> {
        let reach = u64::from(range_cm);
        // **16c `T-107a`**: `within_cm`'s bound squared once for the query rather
        // than once a candidate; `keep` is still exactly `within_cm`.
        let bound_sq = reach_bound_sq(reach);
        // **16c `T-107b`**: the row first, verified, and the search only on a
        // miss. `row` is the index's hint — the position the unit had in the
        // slice the index was rebuilt over — and it names the unit in *this*
        // slice exactly when `units[row].id == id`; any other slice falls to
        // the `binary_search_by_key` this closure always ran. The candidate's
        // row is wherever the unit was found, so it is true of `units`
        // whichever path answered.
        let keep = |id: ObjId, row: u32, cell: (u16, u16), found: &mut Vec<Candidate>| {
            let at = usize::try_from(row)
                .ok()
                .filter(|&at| units.get(at).is_some_and(|unit| unit.id == id))
                .or_else(|| units.binary_search_by_key(&id, |unit| unit.id).ok());
            if let Some(at) = at
                && let Some(unit) = units.get(at)
                && square_cm(pose, &unit.pose) < bound_sq
            {
                found.push(Candidate {
                    id,
                    row: u32::try_from(at).unwrap_or(u32::MAX),
                    cell,
                });
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
                // 16c `T-107b`: sized once for the most either walk can push,
                // so a 300-unit cull grows no vector on the way.
                let mut found: Vec<Candidate> =
                    Vec::with_capacity(box_cells.min(self.of_unit.len()));
                if by_table.unwrap_or_else(|| {
                    box_cells > self.of_unit.len().saturating_mul(UNIT_WALK_CELLS_PER_UNIT)
                }) {
                    // The unit table, kept to exactly what the cell walk reads: a
                    // unit inside the box that HOLDS its cell. A slot names a unit
                    // only at the cell its `of_unit` row records (`rebuild` and
                    // `move_unit` are the grid's only writers and keep that), so
                    // this is the cell walk's set whatever the occupancy.
                    //
                    // 16c `T-107c`: the box check as one unsigned compare an
                    // axis (`within_span`), its two spans hoisted out of the
                    // loop — the same predicate as the two inclusive ranges,
                    // exact because the clamps give `x0 ≤ x1` and `y0 ≤ y1`.
                    // The rows are walked in the order they were and the
                    // occupant test is the one it was.
                    let span_x = x1 - x0;
                    let span_y = y1 - y0;
                    for &IndexRow { id, cell, row } in &self.of_unit {
                        let (x, y) = cell;
                        if within_span(x, x0, span_x)
                            && within_span(y, y0, span_y)
                            && self.occupant(x, y) == Some(id)
                        {
                            keep(id, row, cell, &mut found);
                        }
                    }
                } else {
                    // 16c `T-107b`: the slot names an id; its row comes off
                    // the index's own table, and `keep` verifies it as it does
                    // the table walk's. A slot naming a unit the table does
                    // not hold has no hint and takes the search — and is
                    // unreachable: `move_unit` is only ever asked about a
                    // unit with a cell, and the index is rebuilt over the
                    // same slice before either caller runs (the motion loop's
                    // rebuild just ahead of its integrate loop, and `place`'s
                    // own on its `None` arm), so a unit with a cell is in the
                    // table.
                    let offer = |id: ObjId, x: u16, y: u16, found: &mut Vec<Candidate>| {
                        let row = self.row_of(id).map_or(u32::MAX, |row| row.row);
                        keep(id, row, (x, y), found);
                    };
                    // 16c `T-107c`: each row of the box read as one slice of
                    // the grid rather than a bounds-checked slot a cell. The
                    // grid is row-major with the WIDTH as its stride — never
                    // the height, which only a non-square window tells apart —
                    // so row `y`'s cells `x0..=x1` are the contiguous slots
                    // `y·width + x0 ..= y·width + x1` (`row_slots`), walked in
                    // x-ascending order: the same ids to the same `keep` in
                    // the same order as the per-cell reads. The slice is there
                    // by the clamps (its last slot is at most the buffer's
                    // last), but a row whose slice is not there is read cell by
                    // cell as before, never skipped.
                    for y in y0..=y1 {
                        match self.row_slots(y, x0, x1) {
                            Some(slots) => {
                                // The slice has exactly `x1 − x0 + 1` slots, so
                                // a count from `x0` beside the slice iterator
                                // names each slot's cell — `x0..=x1`, in order
                                // — with no second iterator's bookkeeping (the
                                // review of this cut: a `RangeInclusive` zipped
                                // with the slice takes `Zip`'s general path,
                                // the range's exhausted flag and two compares a
                                // step on top of the slice's own end test).
                                // The count cannot wrap — `row_slots` gives
                                // `x1 ≤ width − 1 ≤ 65,534`, so the step past
                                // the last slot is at most `65,535` — and the
                                // wrapping form says so without a check; that
                                // one step past is never read.
                                let mut x = x0;
                                for slot in slots {
                                    if let Some(id) = *slot {
                                        offer(id, x, y, &mut found);
                                    }
                                    x = x.wrapping_add(1);
                                }
                            }
                            None => {
                                for x in x0..=x1 {
                                    if let Some(id) = self.occupant(x, y) {
                                        offer(id, x, y, &mut found);
                                    }
                                }
                            }
                        }
                    }
                }
                // Kept (16c `T-107b`), two facts: for the table walk it is the
                // identity on the production slice, whose pushes are already in
                // id order there; for the cell walk, whose pushes are in
                // row-major cell order on every slice, it is what makes the
                // answer id-ordered (see the module head).
                found.sort_unstable_by_key(|candidate| candidate.id);
                found
            }
            // The pose is off the window: there is no cell to centre a window on,
            // and the honest answer is still every unit the range function keeps.
            None => {
                let mut found: Vec<Candidate> = Vec::with_capacity(self.of_unit.len());
                for &IndexRow { id, cell, row } in &self.of_unit {
                    keep(id, row, cell, &mut found);
                }
                found
            }
        }
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

    /// The reaches the sweeps below ask about: nothing; either side of the
    /// 3-4-5 triangle's five metres, which is the one exact boundary the
    /// sweep's grid lands on (from (2,800, 900) to B at (2,500, 500) — 16c
    /// `T-107b`, so a range compare loosened to `<=` fails the sweep itself);
    /// either side of one cell, two, three and a half, the diagonal of five,
    /// past the window's side and past the whole window.
    const SWEEP_REACHES: [u32; 10] = [
        0, 499, 500, 999, 1_000, 2_000, 3_500, 7_071, 12_000, 100_000,
    ];

    /// **`in_range_by` before 16c `T-107b`**, kept as the reference the rows
    /// sweeps hold the new walks to: the walk by the cells or by the table as
    /// `by_table` forces it (`None` the cost rule's choice), exactly as the old
    /// body made it — the clamped box, the occupant test on the table walk,
    /// each id found in `units` by `binary_search_by_key`, kept by
    /// [`range_cm`] (the rooted form [`within_cm`] is pinned to), and sorted
    /// on the window — with each kept unit's position in `units` beside its
    /// id, which is what the new `keep` hands back as the row. A mutation of
    /// the range compare, the occupant test or the lookup fails against this
    /// rather than only in a downstream pin.
    ///
    /// **Also 16c `T-107c`'s reference**, unchanged: its cell branch is the
    /// per-cell [`SpatialIndex::occupant`] walk over `(x0..=x1) × (y0..=y1)`
    /// and its table branch the `contains` box check an axis, which are
    /// exactly the two walk shapes T-107c replaced (the row slices of
    /// `SpatialIndex::row_slots` with the per-cell fallback, and
    /// [`within_span`] with its spans hoisted). The T-107c sweeps hold the
    /// new walks to it id for id and row for row, over the non-square windows
    /// and over a buffer cut short.
    fn keep_before_t107b(
        index: &SpatialIndex,
        from: &Pose,
        reach: u32,
        units: &[UnitState],
        by_table: Option<bool>,
    ) -> Vec<(ObjId, usize)> {
        let mut found: Vec<(ObjId, usize)> = Vec::new();
        let keep = |id: ObjId, found: &mut Vec<(ObjId, usize)>| {
            if let Ok(at) = units.binary_search_by_key(&id, |unit| unit.id)
                && let Some(unit) = units.get(at)
                && range_cm(from, &unit.pose) <= u64::from(reach)
            {
                found.push((id, at));
            }
        };
        let span = u64::from(reach) / u64::from(index.cell_cm.max(1)) + 1;
        match index.cell_of_pose(from) {
            Some((cx, cy)) => {
                let span = u16::try_from(span).unwrap_or(u16::MAX);
                let x0 = cx.saturating_sub(span);
                let y0 = cy.saturating_sub(span);
                let x1 = cx.saturating_add(span).min(index.width.saturating_sub(1));
                let y1 = cy.saturating_add(span).min(index.height.saturating_sub(1));
                let box_cells = (usize::from(x1 - x0) + 1).saturating_mul(usize::from(y1 - y0) + 1);
                if by_table.unwrap_or_else(|| {
                    box_cells > index.of_unit.len().saturating_mul(UNIT_WALK_CELLS_PER_UNIT)
                }) {
                    for row in &index.of_unit {
                        let (x, y) = row.cell;
                        if (x0..=x1).contains(&x)
                            && (y0..=y1).contains(&y)
                            && index.occupant(x, y) == Some(row.id)
                        {
                            keep(row.id, &mut found);
                        }
                    }
                } else {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            if let Some(id) = index.occupant(x, y) {
                                keep(id, &mut found);
                            }
                        }
                    }
                }
                // By id first; an id is pushed at most once a walk, so this is
                // the old `sort_unstable` over the ids.
                found.sort_unstable();
            }
            None => {
                for row in &index.of_unit {
                    keep(row.id, &mut found);
                }
            }
        }
        found
    }

    /// A candidate as the reference states it: the id and the row as a slice
    /// position.
    fn id_and_row(candidate: &Candidate) -> (ObjId, usize) {
        (
            candidate.id,
            usize::try_from(candidate.row).unwrap_or(usize::MAX),
        )
    }

    /// **The four states the unit-table sweep walks**, each handed to `check`
    /// with the index and the slice it was rebuilt over: a lawful field; two
    /// units sharing a cell (the lower id holds it, the higher is in no slot);
    /// the higher one moved off into a held cell; and moved on to a free one.
    /// Factored out of `the_unit_table_walk_answers_the_cell_walk_s_set` by 16c
    /// `T-107b` so the rows sweep exactly the states the set does.
    fn over_the_sweep_states(check: impl Fn(&SpatialIndex, &[UnitState], &str)) {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let terrain = &arena.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        check(&index, &arena.units, "lawful");
        // B into A's cell: (0, 0) held by A, B recorded there and in no slot.
        let mut shared = arena.units.clone();
        if let Some(unit) = shared.iter_mut().find(|unit| unit.id == B) {
            unit.pose.x_cm = 700;
            unit.pose.y_cm = 300;
        }
        index.rebuild(terrain, &shared);
        assert_eq!(index.occupant(0, 0), Some(A));
        assert_eq!(index.cell_of_unit(B), Some((0, 0)));
        check(&index, &shared, "shared");
        // B moves into C's held cell (taking nothing), then on to a free one.
        index.move_unit(B, (0, 0), (7, 7));
        check(&index, &shared, "moved into a held cell");
        index.move_unit(B, (7, 7), (3, 4));
        check(&index, &shared, "moved on");
    }

    /// **The VPS spike's unit-table walk** (2026-10-09): every pose on the
    /// window and every reach from nothing to past the window answer one set by
    /// the cells and by the table — in a lawful field, with two units sharing a
    /// cell (the lower id holds it, the higher is in no slot), and after the
    /// higher one has moved off into a held cell and on again — so the cost rule
    /// that picks between them cannot move a candidate.
    #[test]
    fn the_unit_table_walk_answers_the_cell_walk_s_set() {
        over_the_sweep_states(|index, units, state| {
            for x in (0..8_000).step_by(700) {
                for y in (0..8_000).step_by(900) {
                    let from = pose(x, y);
                    for reach in SWEEP_REACHES {
                        let cells = index.in_range_by(&from, reach, units, Some(false));
                        let table = index.in_range_by(&from, reach, units, Some(true));
                        assert_eq!(cells, table, "{state}: from ({x}, {y}) at {reach} cm");
                        assert_eq!(index.in_range(&from, reach, units), cells, "{state}");
                    }
                }
            }
        });
    }

    /// **16c `T-107b`**: [`SpatialIndex::in_range_rows`] answers what
    /// `in_range_by`'s `keep` answered before the cut — `keep_before_t107b`,
    /// the old body kept as the reference: the same walk, the search, the
    /// rooted range, the sort — id for id and row for row, over the sweep's
    /// four states, by the cells, by the table and by the cost rule, and from
    /// off the window; [`SpatialIndex::in_range`] is the ids of that answer;
    /// and every row and cell handed back is true: `units[row].id == id` in
    /// the slice the query was handed, and the cell is the one
    /// [`SpatialIndex::cell_of_unit`] records.
    #[test]
    fn t107b_in_range_rows_answers_in_range_s_set_with_its_rows() {
        let mut candidates = 0_usize;
        let mut nonempty = 0_usize;
        over_the_sweep_states(|index, units, state| {
            let froms = (0..8_000)
                .step_by(700)
                .flat_map(|x| (0..8_000).step_by(900).map(move |y| pose(x, y)))
                .chain([pose(-1_000, -1_000), pose(9_000, 100), pose(100, 9_000)]);
            for from in froms {
                let (x, y) = (from.x_cm, from.y_cm);
                for reach in SWEEP_REACHES {
                    for by_table in [Some(false), Some(true), None] {
                        let rows = index.in_range_rows_by(&from, reach, units, by_table);
                        let reference = keep_before_t107b(index, &from, reach, units, by_table);
                        assert_eq!(
                            rows.iter().map(id_and_row).collect::<Vec<_>>(),
                            reference,
                            "{state}: from ({x}, {y}) at {reach} cm by {by_table:?}"
                        );
                        // And the id-only cull is the reference's ids: not the
                        // map over `in_range_rows_by` compared with itself, but
                        // the old answer.
                        assert_eq!(
                            index.in_range_by(&from, reach, units, by_table),
                            reference.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                            "{state}: the ids from ({x}, {y}) at {reach} cm by {by_table:?}"
                        );
                        for c in &rows {
                            let at = usize::try_from(c.row).unwrap_or(usize::MAX);
                            assert_eq!(
                                units.get(at).map(|unit| unit.id),
                                Some(c.id),
                                "{state}: row {} of {:?} from ({x}, {y}) at {reach} cm",
                                c.row,
                                c.id
                            );
                            assert_eq!(
                                index.cell_of_unit(c.id),
                                Some(c.cell),
                                "{state}: the cell of {:?}",
                                c.id
                            );
                        }
                    }
                    let chosen = index.in_range_rows(&from, reach, units);
                    assert_eq!(chosen, index.in_range_rows_by(&from, reach, units, None));
                    let by_id: Vec<ObjId> = chosen.iter().map(|c| c.id).collect();
                    assert_eq!(by_id, index.in_range(&from, reach, units), "{state}");
                }
            }
            // And the fast path is the one the production slice takes: every
            // candidate's row is the index's own hint, verified rather than
            // searched for.
            for c in index.in_range_rows(&pose(500, 500), 100_000, units) {
                assert_eq!(
                    index.row_of(c.id).map(|row| row.row),
                    Some(c.row),
                    "{state}"
                );
            }
        });
        // Not vacuous: count what one lawful rebuild hands back across the grid.
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let terrain = &arena.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        for x in (0..8_000).step_by(700) {
            for y in (0..8_000).step_by(900) {
                for reach in SWEEP_REACHES {
                    let rows = index.in_range_rows(&pose(x, y), reach, &arena.units);
                    candidates += rows.len();
                    nonempty += usize::from(!rows.is_empty());
                }
            }
        }
        assert!(
            candidates > 108 && nonempty > 108,
            "{candidates} over {nonempty}"
        );
        assert_eq!(
            index.in_range_rows(&pose(500, 500), 2_000, &arena.units),
            vec![
                Candidate {
                    id: A,
                    row: 0,
                    cell: (0, 0)
                },
                Candidate {
                    id: B,
                    row: 1,
                    cell: (2, 0)
                },
            ]
        );
    }

    /// **16c `T-107b`'s verified fallback**: a `units` slice that is not the
    /// one the index was rebuilt over — B `retain`ed away and a D appended, so
    /// C's row names D and B's names C — answers what `keep`'s old computation
    /// answered (`keep_before_t107b`: the walk, each id found in **this** slice
    /// by `binary_search_by_key`, kept by `range_cm`, sorted on the window),
    /// with each row the found element's position in that slice; the appended
    /// D, which the index does not hold, is never offered; and the rebuilt
    /// slice itself takes the hint unsearched. The fixture is lawful — every
    /// unit the index holds holds its slot — so the reference's occupant test
    /// on the table walk keeps every unit in the box, and the two old walks
    /// agree here as everywhere, which is asserted rather than assumed.
    #[test]
    fn t107b_a_foreign_slice_takes_the_fallback() {
        let arena = arena_of(vec![
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
            tank(C, Team::Blue, pose(7_500, 7_500)),
        ]);
        let terrain = &arena.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        const D: ObjId = ObjId(20);
        let mut foreign = arena.units.clone();
        foreign.retain(|unit| unit.id != B);
        let appended = arena_of(vec![tank(D, Team::Red, pose(3_500, 500))]);
        foreign.extend(appended.units.iter().cloned());
        assert_eq!(
            foreign.iter().map(|unit| unit.id).collect::<Vec<_>>(),
            vec![A, C, D]
        );
        // The index's rows are the rebuilt slice's: A 0, B 1, C 2 — so in the
        // foreign slice A's hint holds, B's names C and C's names D.
        assert_eq!(index.row_of(A).map(|row| row.row), Some(0));
        assert_eq!(index.row_of(B).map(|row| row.row), Some(1));
        assert_eq!(index.row_of(C).map(|row| row.row), Some(2));
        assert!(index.row_of(D).is_none());
        // `keep` before this cut, over the foreign slice, by the walk asked
        // for; on the window the two old walks are first held to one answer.
        let reference = |from: &Pose, reach: u32, by_table: Option<bool>| -> Vec<(ObjId, usize)> {
            if index.cell_of_pose(from).is_some() {
                assert_eq!(
                    keep_before_t107b(&index, from, reach, &foreign, Some(false)),
                    keep_before_t107b(&index, from, reach, &foreign, Some(true)),
                    "the two old walks part over the foreign slice from ({}, {}) at {reach} cm",
                    from.x_cm,
                    from.y_cm
                );
            }
            keep_before_t107b(&index, from, reach, &foreign, by_table)
        };
        let froms = [
            pose(500, 500),
            pose(2_500, 500),
            pose(3_500, 500),
            pose(7_500, 7_500),
            pose(4_000, 4_000),
            pose(-1_000, -1_000),
            pose(9_000, 9_000),
        ];
        let mut fell_back = 0_usize;
        for from in &froms {
            for reach in SWEEP_REACHES {
                for by_table in [Some(false), Some(true), None] {
                    let rows = index.in_range_rows_by(from, reach, &foreign, by_table);
                    assert_eq!(
                        rows.iter().map(id_and_row).collect::<Vec<_>>(),
                        reference(from, reach, by_table),
                        "from ({}, {}) at {reach} cm by {by_table:?}",
                        from.x_cm,
                        from.y_cm
                    );
                    for c in &rows {
                        assert_ne!(c.id, B, "a unit retained away was offered");
                        assert_ne!(c.id, D, "a unit the index never held was offered");
                        if c.id == C {
                            assert_eq!(c.row, 1, "C's row in the foreign slice");
                            fell_back += 1;
                        }
                    }
                }
                // And over the rebuilt slice the rows are the hints themselves.
                for c in index.in_range_rows_by(from, reach, &arena.units, None) {
                    assert_eq!(index.row_of(c.id).map(|row| row.row), Some(c.row));
                }
            }
        }
        assert!(fell_back > 0, "the fallback was never taken");
        // The point cases, by hand: C from its own cell is found at row 1 of
        // the foreign slice (row 2 of the index); B from its old cell is
        // nobody; D from its own cell is nobody.
        assert_eq!(
            index.in_range_rows(&pose(7_500, 7_500), 0, &foreign),
            vec![Candidate {
                id: C,
                row: 1,
                cell: (7, 7)
            }]
        );
        assert_eq!(
            index.in_range_rows(&pose(2_500, 500), 0, &foreign),
            Vec::<Candidate>::new()
        );
        assert_eq!(
            index.in_range_rows(&pose(3_500, 500), 0, &foreign),
            Vec::<Candidate>::new()
        );
        assert_eq!(
            index.in_range(&pose(500, 500), 100_000, &foreign),
            vec![A, C]
        );
    }

    /// **16c `T-107b` keeps the on-window sort**, and this is why: on the
    /// production slice — [`crate::arena::state::Arena::units`], laid down in
    /// id order — the table is in id order, its rows are `0..n`, and the table
    /// walk pushes in id order, so for that walk the sort is the identity;
    /// over a slice in any other order the table walk pushes in *that* order,
    /// each hint still verified, and it is the sort that makes the answer
    /// id-ordered. The rotated half is also **the one place the hint and the
    /// search part**: the search over an unsorted slice is promised nothing,
    /// and the hint finds a unit it misses.
    #[test]
    fn t107b_the_table_walk_pushes_in_id_order() {
        let arena = arena_of(vec![
            tank(C, Team::Blue, pose(7_500, 7_500)),
            tank(A, Team::Red, pose(500, 500)),
            tank(B, Team::Blue, pose(2_500, 500)),
        ]);
        // `Arena::new` sorted the slice whatever order it was handed in.
        assert_eq!(
            arena.units.iter().map(|unit| unit.id).collect::<Vec<_>>(),
            vec![A, B, C]
        );
        let terrain = &arena.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        assert_eq!(
            index
                .of_unit
                .iter()
                .map(|row| (row.id, row.row))
                .collect::<Vec<_>>(),
            vec![(A, 0), (B, 1), (C, 2)]
        );
        for x in (0..8_000).step_by(700) {
            for y in (0..8_000).step_by(900) {
                for reach in SWEEP_REACHES {
                    let table =
                        index.in_range_rows_by(&pose(x, y), reach, &arena.units, Some(true));
                    // What the walk pushed before the sort: the table's order,
                    // filtered — and that is already id order.
                    let pushed: Vec<ObjId> = index
                        .of_unit
                        .iter()
                        .map(|row| row.id)
                        .filter(|id| table.iter().any(|c| c.id == *id))
                        .collect();
                    assert_eq!(
                        table.iter().map(|c| c.id).collect::<Vec<_>>(),
                        pushed,
                        "the sort moved a candidate from ({x}, {y}) at {reach} cm"
                    );
                    assert!(
                        table.windows(2).all(|pair| pair[0].id < pair[1].id),
                        "not strictly ascending from ({x}, {y}) at {reach} cm"
                    );
                    assert!(
                        table.windows(2).all(|pair| pair[0].row < pair[1].row),
                        "rows out of order from ({x}, {y}) at {reach} cm"
                    );
                }
            }
        }
        // A slice in another order — the production slice rotated, C first —
        // rebuilt over: the table and its rows follow the slice, every hint
        // still names its unit, and the table walk's pushes are C, A, B until
        // the kept sort puts them in id order.
        //
        // **This is the one place the hint and the search part.** The hint
        // finds C at row 0, where it is. The old `keep` searched the slice for
        // C, and a binary search over an unsorted slice is promised nothing:
        // over this one, std's halving (today) probes rows 1 and 2 for the
        // largest id and misses, so the old `keep` dropped C and answered A,
        // B — the new answer is a superset, and a different set. On a
        // duplicate-free slice any hit the search does make is the element
        // with that id, which is the hint's row, so the new answer always
        // holds the old one at the same rows; it is more than the old one
        // only over a slice no production cull is handed (`Arena::new` sorts;
        // the module head), and the assertions below are the ones that hold
        // whatever the search does.
        let mut rotated = arena.units.clone();
        rotated.rotate_left(2);
        assert_eq!(
            rotated.iter().map(|unit| unit.id).collect::<Vec<_>>(),
            vec![C, A, B]
        );
        index.rebuild(terrain, &rotated);
        assert_eq!(
            index
                .of_unit
                .iter()
                .map(|row| (row.id, row.row))
                .collect::<Vec<_>>(),
            vec![(C, 0), (A, 1), (B, 2)]
        );
        let whole = index.in_range_rows_by(&pose(500, 500), 100_000, &rotated, Some(true));
        assert_eq!(
            whole,
            vec![
                Candidate {
                    id: A,
                    row: 1,
                    cell: (0, 0)
                },
                Candidate {
                    id: B,
                    row: 2,
                    cell: (2, 0)
                },
                Candidate {
                    id: C,
                    row: 0,
                    cell: (7, 7)
                },
            ]
        );
        for c in &whole {
            assert_eq!(
                rotated
                    .get(usize::try_from(c.row).unwrap_or(usize::MAX))
                    .map(|unit| unit.id),
                Some(c.id)
            );
        }
        // The old `keep` over the rotated slice, by the same walk: whatever
        // the search found, the new answer holds at the same row — and any
        // hit the search makes for C is row 0, the hint's.
        let old = keep_before_t107b(&index, &pose(500, 500), 100_000, &rotated, Some(true));
        assert!(old.len() <= whole.len());
        for (id, at) in &old {
            assert!(
                whole.iter().any(|c| id_and_row(c) == (*id, *at)),
                "the old keep found {id:?} at row {at} and the new answer does not hold it"
            );
        }
        if let Ok(at) = rotated.binary_search_by_key(&C, |unit| unit.id) {
            assert_eq!(at, 0, "a hit for C that is not C's row");
        }
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

    /// **A window of `width` by `height` cells of ground** (16c `T-107c`) with
    /// the stock definition's zones, spots and graph, fielded with `units` —
    /// the shape `the_cell_window_cull_answers_what_a_brute_force_scan_answers`
    /// has on the square test window, on a window whose two strides differ.
    fn arena_on(
        width: u16,
        height: u16,
        units: Vec<crate::arena::state::CustodyUnit>,
    ) -> crate::arena::state::Arena {
        use crate::arena::state::{Arena, ArenaDefinition, ArenaTerrain, CustodyOut, MatchMode};
        use crate::arena::test_support::{definition, ground, resolution};
        use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
        let cells = vec![ground(); usize::from(width) * usize::from(height)];
        Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::new(width, height, 1_000, cells).unwrap_or_else(
                        |error| panic!("the {width} by {height} window: {error:?}"),
                    ),
                    ..definition()
                },
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the {width} by {height} arena: {error:?}"))
    }

    /// **The ten placements of the non-square sweeps** (16c `T-107c`) on the
    /// 12 by 5 window, as `(x_cm, y_cm)`: the four corners, a neighbour across
    /// the first corner's cell boundary, two neighbours mid-window, the far
    /// corner's diagonal neighbour and its neighbour along the last row, and
    /// one more on the last row — deliberately not all cell centres, so a
    /// reach under one cell has to look into the next cell. Transposed
    /// (`(y_cm, x_cm)`) they are the same field on the 5 by 12 window.
    const T107C_PLACEMENTS: [(i64, i64); 10] = [
        (500, 500),      // cell (0, 0): the near corner
        (11_900, 100),   // cell (11, 0)
        (100, 4_900),    // cell (0, 4)
        (11_500, 4_500), // cell (11, 4): the far corner
        (1_100, 500),    // cell (1, 0), 600 cm from the near corner
        (5_500, 2_500),  // cell (5, 2)
        (6_100, 2_900),  // cell (6, 2), 721 cm from (5, 2)
        (10_900, 3_900), // cell (10, 3), 848 cm from the far corner
        (3_500, 4_500),  // cell (3, 4)
        (10_500, 4_500), // cell (10, 4), 1,000 cm from the far corner exactly
    ];

    /// The reaches the non-square sweeps ask about (16c `T-107c`): nothing;
    /// one centimetre; half a cell; either side of one cell, where the span
    /// grows from one to two; just under two cells; and three and a half, where
    /// the clamped box outgrows the cost rule's four cells a unit and the
    /// unforced cull takes the table.
    const T107C_REACHES: [u32; 7] = [0, 1, 500, 999, 1_000, 1_999, 3_500];

    /// [`T107C_PLACEMENTS`] fielded on a `width` by `height` window, transposed
    /// when `transposed` — ten tanks, ids `10..=19` in placement order, so
    /// `Arena::new`'s id order is the placement order and each unit's row is its
    /// placement.
    fn t107c_field(width: u16, height: u16, transposed: bool) -> crate::arena::state::Arena {
        let units: Vec<crate::arena::state::CustodyUnit> = T107C_PLACEMENTS
            .iter()
            .enumerate()
            .map(|(n, (x_cm, y_cm))| {
                let id = ObjId(10 + u32::try_from(n).unwrap_or(0));
                let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
                let at = if transposed {
                    pose(*y_cm, *x_cm)
                } else {
                    pose(*x_cm, *y_cm)
                };
                tank(id, team, at)
            })
            .collect();
        arena_on(width, height, units)
    }

    /// Every pose the non-square sweeps cull from: each unit's own, and the
    /// centres of the window's four corner cells — on the transposed window in
    /// the transposed order, so the `n`th from on either window is the
    /// transpose of the `n`th on the other.
    fn t107c_froms(width: u16, height: u16, transposed: bool, units: &[UnitState]) -> Vec<Pose> {
        let far_x = i64::from(width - 1) * 1_000 + 500;
        let far_y = i64::from(height - 1) * 1_000 + 500;
        let (second, third) = if transposed {
            (pose(500, far_y), pose(far_x, 500))
        } else {
            (pose(far_x, 500), pose(500, far_y))
        };
        units
            .iter()
            .map(|unit| unit.pose.clone())
            .chain([pose(500, 500), second, third, pose(far_x, far_y)])
            .collect()
    }

    /// **16c `T-107c`: the sliced cell walk on a window whose strides differ.**
    /// A 12 by 5 window and its 5 by 12 transpose, ten units at the corners and
    /// inside, seven reaches from every unit and every corner: the cull by the
    /// cells — now a slice a row, `y · width` its stride — answers a brute-force
    /// [`within_cm`] scan of the units; answers, id for id and row for row, the
    /// pre-cut walk kept as `keep_before_t107b` (whose cell branch reads
    /// [`SpatialIndex::occupant`] one cell at a time, which is exactly the walk
    /// this cut replaced); answers the table walk, forced by
    /// `in_range_rows_by`'s switch, and the unforced cull, which the cost rule
    /// — restated here as a tally, since the body exposes no counter — sends
    /// to the cells at the six short reaches and the table at the seventh (the
    /// forcing is what guarantees both walks run at every comparison); and the
    /// transpose answers the same ids from the transposed pose. A `y · height` stride
    /// would read row 4 of the 12 by 5 window at slot 20 rather than 48 — the
    /// slip no square window could tell from the right one — and the far
    /// corner's own cull is pinned by hand so that case is not only implied.
    #[test]
    fn t107c_the_cell_walk_on_a_non_square_window_answers_the_brute_force_scan() {
        let wide = t107c_field(12, 5, false);
        let tall = t107c_field(5, 12, true);
        let mut comparisons = 0_usize;
        let mut by_cells = 0_usize;
        let mut by_table = 0_usize;
        let mut answers: Vec<Vec<Vec<ObjId>>> = Vec::new();
        for (arena, width, height, transposed) in
            [(&wide, 12_u16, 5_u16, false), (&tall, 5, 12, true)]
        {
            let terrain = &arena.definition.terrain;
            assert_eq!((terrain.width(), terrain.height()), (width, height));
            let units = &arena.units;
            assert_eq!(
                units.iter().map(|unit| unit.id).collect::<Vec<_>>(),
                (10..20).map(ObjId).collect::<Vec<_>>()
            );
            let mut index = SpatialIndex::new(terrain);
            index.rebuild(terrain, units);
            assert_eq!(index.len(), 10);
            assert_eq!((index.width(), index.height()), (width, height));
            let brute = |from: &Pose, reach: u32| -> Vec<ObjId> {
                let mut found: Vec<ObjId> = units
                    .iter()
                    .filter(|unit| within_cm(from, &unit.pose, u64::from(reach)))
                    .map(|unit| unit.id)
                    .collect();
                found.sort_unstable();
                found
            };
            let mut here: Vec<Vec<ObjId>> = Vec::new();
            for from in t107c_froms(width, height, transposed, units) {
                let (x, y) = (from.x_cm, from.y_cm);
                for reach in T107C_REACHES {
                    let expected = brute(&from, reach);
                    let cells = index.in_range_rows_by(&from, reach, units, Some(false));
                    let table = index.in_range_rows_by(&from, reach, units, Some(true));
                    let chosen = index.in_range_rows_by(&from, reach, units, None);
                    assert_eq!(
                        cells.iter().map(|c| c.id).collect::<Vec<_>>(),
                        expected,
                        "{width} by {height}: the cell walk and the scan disagree at {reach} cm from ({x}, {y})"
                    );
                    assert_eq!(
                        cells, table,
                        "{width} by {height}: the cell walk and the table walk disagree at {reach} cm from ({x}, {y})"
                    );
                    assert_eq!(
                        chosen, cells,
                        "{width} by {height}: the unforced cull at {reach} cm from ({x}, {y})"
                    );
                    assert_eq!(
                        index.in_range(&from, reach, units),
                        expected,
                        "{width} by {height}: the ids at {reach} cm from ({x}, {y})"
                    );
                    // The pre-cut walk, by the same branch, id for id and row
                    // for row.
                    for forced in [Some(false), Some(true), None] {
                        assert_eq!(
                            index
                                .in_range_rows_by(&from, reach, units, forced)
                                .iter()
                                .map(id_and_row)
                                .collect::<Vec<_>>(),
                            keep_before_t107b(&index, &from, reach, units, forced),
                            "{width} by {height}: the pre-cut walk at {reach} cm from ({x}, {y}) by {forced:?}"
                        );
                    }
                    // Every row and cell handed back is true.
                    for c in &cells {
                        let at = usize::try_from(c.row).unwrap_or(usize::MAX);
                        assert_eq!(units.get(at).map(|unit| unit.id), Some(c.id));
                        assert_eq!(index.cell_of_unit(c.id), Some(c.cell));
                        assert_eq!(terrain.cell_of(&units[at].pose), Some(c.cell));
                    }
                    // Which branch the cost rule sends the unforced cull down,
                    // as this test restates the rule — the box against four
                    // cells a unit, sized as `in_range_rows_by` sizes it. A
                    // tally and not an observation: the production body
                    // exposes no counter, so what guarantees both walks ran
                    // at this comparison is the forcing above (`Some(false)`
                    // and `Some(true)` take each branch whatever the rule
                    // says) with `chosen` held equal to both.
                    let span = u16::try_from(u64::from(reach) / 1_000 + 1).unwrap_or(u16::MAX);
                    let (cx, cy) = index
                        .cell_of_pose(&from)
                        .expect("every from is on the window");
                    let box_cells = (usize::from(
                        cx.saturating_add(span).min(width - 1) - cx.saturating_sub(span),
                    ) + 1)
                        * (usize::from(
                            cy.saturating_add(span).min(height - 1) - cy.saturating_sub(span),
                        ) + 1);
                    if box_cells > 10 * UNIT_WALK_CELLS_PER_UNIT {
                        by_table += 1;
                    } else {
                        by_cells += 1;
                    }
                    comparisons += 1;
                    here.push(expected);
                }
            }
            answers.push(here);
        }
        // Fourteen froms by seven reaches on each of two windows; the cost
        // rule as restated here sends the unforced cull down both branches
        // across the sweep (the forced walks exercised both at every
        // comparison regardless).
        assert_eq!(comparisons, 2 * 14 * 7);
        assert!(
            by_cells > 0 && by_table > 0,
            "{by_cells} by the cells, {by_table} by the table"
        );
        assert_eq!(by_cells + by_table, comparisons);
        // The transpose answers the same ids from the transposed pose: the
        // froms are built in the same order on both windows, and a Euclidean
        // range does not care which axis is which.
        assert_eq!(
            answers[0], answers[1],
            "the 5 by 12 window answers differently"
        );
        // Not vacuous, and the stride pinned by hand: on the 12 by 5 window the
        // far corner's unit at (11, 4) holds slot 4 · 12 + 11 = 59, and a walk
        // striding by the height would read 4 · 5 + 11 = 31 — cell (7, 2),
        // which is empty — and find nobody at reach 0.
        let terrain = &wide.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &wide.units);
        assert_eq!(index.occupant.get(59).copied().flatten(), Some(ObjId(13)));
        assert_eq!(index.occupant.get(31).copied().flatten(), None);
        assert_eq!(
            index.in_range_rows_by(&pose(11_500, 4_500), 0, &wide.units, Some(false)),
            vec![Candidate {
                id: ObjId(13),
                row: 3,
                cell: (11, 4)
            }]
        );
        // From the far corner: 848 cm reaches the diagonal neighbour and not
        // the one along the row at 1,000 exactly, which 1,000 reaches.
        assert_eq!(
            index.in_range(&pose(11_500, 4_500), 999, &wide.units),
            vec![ObjId(13), ObjId(17)]
        );
        assert_eq!(
            index.in_range(&pose(11_500, 4_500), 1_000, &wide.units),
            vec![ObjId(13), ObjId(17), ObjId(19)]
        );
        // From the near corner: 600 cm across the cell boundary at 999 and not
        // at 500.
        assert_eq!(
            index.in_range(&pose(500, 500), 999, &wide.units),
            vec![ObjId(10), ObjId(14)]
        );
        assert_eq!(
            index.in_range(&pose(500, 500), 500, &wide.units),
            vec![ObjId(10)]
        );
        // And on the transpose, the same from the transposed corner.
        let terrain = &tall.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &tall.units);
        assert_eq!(
            index.in_range_rows_by(&pose(4_500, 11_500), 0, &tall.units, Some(false)),
            vec![Candidate {
                id: ObjId(13),
                row: 3,
                cell: (4, 11)
            }]
        );
        assert_eq!(
            index.in_range(&pose(4_500, 11_500), 1_000, &tall.units),
            vec![ObjId(13), ObjId(17), ObjId(19)]
        );
    }

    /// **16c `T-107c`'s fallback**: a row whose slice the buffer does not
    /// hold is read cell by cell, never skipped. The 12 by 5 field's buffer
    /// is cut one slot short — the far corner's own — so the box of a cull
    /// from that corner at one cell's reach, `(9..=11) × (2..=4)`, has two
    /// rows the slice answers and one, the last, it does not; the walk answers
    /// what the pre-cut walk answers over the same cut buffer
    /// (`keep_before_t107b`'s cell branch, one [`SpatialIndex::occupant`] read
    /// a cell, which is `None` for the missing slot and the unit at (10, 4) for
    /// the slot before it), over the whole sweep and by hand: the neighbour at
    /// (10, 4) is found through the fallback, the corner unit is gone with its
    /// slot, and a walk that skipped the row would have lost the neighbour.
    #[test]
    fn t107c_a_row_the_buffer_does_not_hold_falls_back_to_the_cells() {
        let arena = t107c_field(12, 5, false);
        let terrain = &arena.definition.terrain;
        let units = &arena.units;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, units);
        assert_eq!(index.occupant.len(), 60);
        // Whole: every row of the box is a slice.
        assert!(index.row_slots(4, 9, 11).is_some());
        index.occupant.truncate(59);
        // Cut: the last row of the box is not, the two before it are, and the
        // per-cell reads answer what they always did — nothing for the slot
        // that is gone, the unit for the one before it.
        assert!(index.row_slots(4, 9, 11).is_none());
        assert!(index.row_slots(3, 9, 11).is_some());
        assert!(index.row_slots(2, 9, 11).is_some());
        assert_eq!(index.occupant(11, 4), None);
        assert_eq!(index.occupant(10, 4), Some(ObjId(19)));
        assert_eq!(index.occupant(10, 3), Some(ObjId(17)));
        let from = pose(11_500, 4_500);
        let walked = index.in_range_rows_by(&from, 1_000, units, Some(false));
        assert_eq!(
            walked,
            vec![
                Candidate {
                    id: ObjId(17),
                    row: 7,
                    cell: (10, 3)
                },
                Candidate {
                    id: ObjId(19),
                    row: 9,
                    cell: (10, 4)
                },
            ],
            "the neighbour in the cut row was skipped, or the corner unit found without its slot"
        );
        assert_eq!(
            walked.iter().map(id_and_row).collect::<Vec<_>>(),
            keep_before_t107b(&index, &from, 1_000, units, Some(false))
        );
        // And the whole sweep over the cut buffer, by every branch, against the
        // pre-cut walk — the table walk reads the same cut `occupant`, so it
        // and the cell walk still agree.
        let mut whole = SpatialIndex::new(terrain);
        whole.rebuild(terrain, units);
        let mut fell_short = 0_usize;
        for from in t107c_froms(12, 5, false, units) {
            let (x, y) = (from.x_cm, from.y_cm);
            for reach in T107C_REACHES {
                for forced in [Some(false), Some(true), None] {
                    let rows = index.in_range_rows_by(&from, reach, units, forced);
                    assert_eq!(
                        rows.iter().map(id_and_row).collect::<Vec<_>>(),
                        keep_before_t107b(&index, &from, reach, units, forced),
                        "over the cut buffer at {reach} cm from ({x}, {y}) by {forced:?}"
                    );
                    assert!(
                        rows.iter().all(|c| c.id != ObjId(13)),
                        "the unit whose slot is gone was offered at {reach} cm from ({x}, {y}) by {forced:?}"
                    );
                }
                let cells = index.in_range_rows_by(&from, reach, units, Some(false));
                assert_eq!(
                    cells,
                    index.in_range_rows_by(&from, reach, units, Some(true))
                );
                // What the whole buffer answers less the corner unit, which is
                // what a cut buffer can answer at most: the cut loses exactly
                // that unit and nothing else.
                let mut expected = whole.in_range_rows_by(&from, reach, units, Some(false));
                let before = expected.len();
                expected.retain(|c| c.id != ObjId(13));
                fell_short += usize::from(expected.len() < before);
                assert_eq!(cells, expected, "at {reach} cm from ({x}, {y})");
            }
        }
        assert!(fell_short > 0, "the cut slot was never in a box");
    }

    /// **16c `T-107c`**: [`within_span`] is the inclusive range's own
    /// `contains` — for every `lo ≤ hi` over `0..=45` and the three top `u16`s,
    /// and every `v` over the same values, with `span = hi − lo` as the table
    /// walk hoists it — so the box check it replaced cannot keep or drop a row
    /// the range would not.
    #[test]
    fn t107c_the_span_test_is_the_inclusive_range_test() {
        let values: Vec<u16> = (0..=45_u16)
            .chain([u16::MAX - 2, u16::MAX - 1, u16::MAX])
            .collect();
        let mut checked = 0_u64;
        let mut inside = 0_u64;
        for &lo in &values {
            for &hi in &values {
                if lo > hi {
                    continue;
                }
                let span = hi - lo;
                for &v in &values {
                    let expected = (lo..=hi).contains(&v);
                    assert_eq!(within_span(v, lo, span), expected, "v={v} lo={lo} hi={hi}");
                    checked += 1;
                    inside += u64::from(expected);
                }
            }
        }
        assert!(
            checked > 50_000 && inside > 0 && inside < checked,
            "{inside} of {checked}"
        );
        // The boundaries by hand, including the one the wrap matters at: a
        // value just below `lo`, which an unsigned subtraction would otherwise
        // put inside.
        assert!(within_span(3, 3, 0));
        assert!(!within_span(2, 3, 0));
        assert!(!within_span(4, 3, 0));
        assert!(within_span(0, 0, u16::MAX));
        assert!(within_span(u16::MAX, 0, u16::MAX));
        assert!(!within_span(0, 1, u16::MAX - 1));
        assert!(within_span(u16::MAX, 1, u16::MAX - 1));
    }

    /// **16c `T-107c`**: [`SpatialIndex::row_slots`] is the [`SpatialIndex::occupant`]
    /// reads it stands in for — on the 12 by 5 window and its transpose, for
    /// every row and every `x0 ≤ x1` on the window, the slice is the cells'
    /// slots in x-ascending order and exactly `x1 − x0 + 1` of them; off the
    /// window (a row past the height, a span past the width, where a slice
    /// would run into the next row) it is `None`.
    #[test]
    fn t107c_a_row_s_slots_are_the_cells_occupant_reads() {
        let mut sliced = 0_usize;
        let mut refused = 0_usize;
        let mut occupied = 0_usize;
        for (width, height, transposed) in [(12_u16, 5_u16, false), (5, 12, true)] {
            let arena = t107c_field(width, height, transposed);
            let terrain = &arena.definition.terrain;
            let mut index = SpatialIndex::new(terrain);
            index.rebuild(terrain, &arena.units);
            for y in 0..=height {
                for x0 in 0..=width {
                    for x1 in x0..=width {
                        let slots = index.row_slots(y, x0, x1);
                        if y < height && x1 < width {
                            let slots = slots.unwrap_or_else(|| {
                                panic!("{width} by {height}: no slice for row {y}, {x0}..={x1}")
                            });
                            assert_eq!(slots.len(), usize::from(x1 - x0) + 1);
                            let reads: Vec<Option<ObjId>> =
                                (x0..=x1).map(|x| index.occupant(x, y)).collect();
                            assert_eq!(
                                slots,
                                reads.as_slice(),
                                "{width} by {height}: row {y}, {x0}..={x1}"
                            );
                            sliced += 1;
                            occupied += slots.iter().flatten().count();
                        } else {
                            assert!(
                                slots.is_none(),
                                "{width} by {height}: a slice for row {y}, {x0}..={x1}"
                            );
                            refused += 1;
                        }
                    }
                }
            }
        }
        assert!(
            sliced > 0 && refused > 0 && occupied > 0,
            "{sliced} sliced, {refused} refused, {occupied} occupied"
        );
        // The whole last row of the 12 by 5 window, by hand: units at (0, 4),
        // (3, 4), (10, 4) and (11, 4), and nothing else.
        let arena = t107c_field(12, 5, false);
        let terrain = &arena.definition.terrain;
        let mut index = SpatialIndex::new(terrain);
        index.rebuild(terrain, &arena.units);
        let last = index.row_slots(4, 0, 11).expect("the last row");
        assert_eq!(
            last.iter()
                .enumerate()
                .filter_map(|(x, slot)| slot.map(|id| (x, id)))
                .collect::<Vec<_>>(),
            vec![
                (0, ObjId(12)),
                (3, ObjId(18)),
                (10, ObjId(19)),
                (11, ObjId(13))
            ]
        );
        // The slice across a row's end that `row_slots` refuses would have been
        // the next row's first cells: on the 12 by 5 window slots 10..=13 are
        // (10, 0), (11, 0), (0, 1), (1, 1).
        assert!(index.row_slots(0, 10, 13).is_none());
        assert_eq!(
            index.occupant.get(10..=13).map(<[Option<ObjId>]>::len),
            Some(4)
        );
    }
}
