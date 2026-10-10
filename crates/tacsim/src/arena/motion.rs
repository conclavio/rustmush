//! **Motion** (16b cut A.2; doc 16 §3.4 "Motion", §3.2 "Edges", §3.1
//! "Determinism").
//!
//! One function, [`integrate`], run once per unit per **substep** in unit-id
//! order, performing doc 16 §3.4's four steps in order:
//!
//! 1. the heading turns toward the ordered one at `turn_rate_mdeg_s`, on the
//!    shortest arc, **blocked toward a destroyed track's side**;
//! 2. the speed moves toward the ordered one at `accel_cms2` (or `brake_cms2`
//!    when the magnitude falls), capped by the terrain factor and the `slow`
//!    consequences, and forced to zero by a wreck, a disabled engine or power
//!    that is not online;
//! 3. the position advances `speed × dt` along the heading, which by the
//!    construction of the substep crosses at most one cell boundary;
//! 4. a crossing tests the cell entered — impassable, occupied, too steep or off
//!    the window — and a refusal stops the unit at the boundary with `speed = 0`
//!    and one `blocked` notice.
//!
//! **16b cut E.3 put one delay into step 4, for traffic alone** (16c `B-46`, §5's
//! Q10). Ground, the window's edge and a slope still stop a unit dead at the
//! centimetre it was refused. A cell held by **another unit** opens a *hold*
//! instead (`hold_at_boundary`): the unit is clamped at the boundary keeping the
//! speed it had, nothing is journalled and nobody is told, and if the way clears
//! it drives on without ever having stopped. Only a hold on one cell that outlasts
//! `[tacsim.arena].occupied_stop_ms` of arena time becomes step 4's literal stop,
//! with its `blocked` line, its pilot notice and its lesson for the machine —
//! exactly as before, two seconds later. The reason is a column at a ford: a
//! follower that integrates before its leader reaches the cell the leader has not
//! vacated yet, and stopping it dead cost it a dozen seconds of rebuilding way and
//! taught its machine to avoid the cell its own leader was about to leave.
//!
//! **16b cut X.6 put the ground's third dimension into steps 2 and 4.** The
//! slope of the edge a unit crossed to reach its cell surcharges that cell's
//! `move_cost` for as long as it stands there, and an edge steeper than the
//! unit's locomotion will take is refused — in **either** direction, so a bluff
//! is never a one-way door. Both are [`crate::arena::slope`]'s rule and not a
//! second copy of it, because the reflex controller's A\* plans over exactly the
//! same one. On level ground the surcharge is the identity and nothing moves.
//!
//! **16b cut AE.1 (16c `B-58`) made the price the unit's own.** Steps 2 and 4
//! read the cell through [`ArenaTerrain::price`] and
//! [`ArenaTerrain::passable_for`] with the unit's
//! [`crate::arena::state::UnitState::cost_row`], so a walker crosses the thick
//! woods a tank is refused and pays its own row's price for rubble. On a
//! hand-built window every row reads the cell's `move_cost`, so every fixture
//! runs the arithmetic it ran before.
//!
//! **The invariant: no unit ever occupies a cell that was not tested on entry.**
//! It holds by construction rather than by inspection: a refused crossing clamps
//! the unit back into the cell it already occupied (which was tested when it
//! entered it), and an accepted one lands in the cell just tested.
//!
//! # Everything stored is an integer
//!
//! Doc 16 §3.1 (D-16-4). The heading is millidegrees, the speed is cm/s, the pose
//! is whole centimetres, and the trigonometry step 3 needs is a **fixed-point
//! table** ([`SIN_FX`]) with integer interpolation — not `f64::sin`, whose last
//! bits are the platform's libm and not the engine's. That is what lets the
//! cross-tempo property test compare byte-identical per-substep hashes across
//! machines as well as across tempos.
//!
//! # The three remainders are carried, not truncated (16b cut Q.6)
//!
//! Each of the three rates above was written `rate × dt / 1000` and the
//! remainder dropped, three times per unit per substep — so the *substep length*
//! was an input to the answer. A crawl is the plainest evidence: a unit ordered
//! to 3 cm/s covered 0.75 cm in a 250 ms substep, which truncated to nothing, so
//! it stood still for as long as it was driven.
//!
//! **X.6's five-against-one `blocked` count was read as the same defect and is
//! not one** (16b cut Q.6 measured it): the carries removed every truncation in
//! motion and that count did not move — 250 ms still printed one and 150 ms still
//! printed five. Its cause was the *two flags* cut F.2 split deliberately: the
//! ledger's line fired on the per-substep `blocked` flag and the pilot's notice
//! on the `held` condition. **16b cut M.3 (16c `T-04`) put both on one edge** —
//! [`integrate`]'s `held` rule, which now knows the refused cell and its cause —
//! and this module's `one_obstacle_is_one_stop_at_every_partition` test, which
//! measured the count, now asserts it: one line at every partition.
//!
//! So each rate keeps what it did not spend, on
//! [`crate::arena::state::MotionCarry`], exactly the way [`crate::arena::heat`]
//! keeps the fraction of a heat unit a substep did not shed — that module's head
//! is the argument, written once. The turn carries millidegrees × 1,000, the
//! acceleration carries cm/s × 1,000, and each axis of the travel carries
//! centimetres × [`DISTANCE_SCALE`], because the distance is `speed × dt × sin`
//! and **neither** division is taken until the carry has been added. A refused
//! substep banks nothing: every stop, every wreck, every dead engine and every
//! cold hull zeroes what it did not travel.
//!
//! **What this makes exact, whatever way arena time is cut into substeps** — 250,
//! 150, 50, or a ragged remainder: a straight run at constant speed, a pure turn,
//! and a speed ramp on uniform ground each end at the same heading, speed and
//! pose.
//!
//! **What it does not.** Position while accelerating, or while turning and moving
//! at once, is first-order in the substep (semi-implicit Euler: the substep
//! travels at the speed and heading it *ends* with). The difference is bounded and
//! it is no longer a truncation that drifts one way. Event **timestamps** are
//! substep boundaries, so two different partitions of the same arena time still do
//! not give one event list or one hash sequence; the cross-tempo property holds
//! because the three cadences share one partition. An absolute substep lattice is
//! the full remedy and is deliberately not taken here.

use mush_atlas::physics::StepKind;
use mush_sim::data::Pose;
use serde::{Deserialize, Serialize};

use crate::arena::index::SpatialIndex;
use crate::arena::slope;
use crate::arena::state::{
    ArenaTerrain, CostRow, OccupiedHold, Power, Side, TerrainCell, UnitState,
};

/// **The scale each axis of [`crate::arena::state::MotionCarry`] is measured in**
/// (16b cut Q.6): the millisecond's thousand times [`SIN_ONE`]'s fixed point.
///
/// The east component of one substep's travel is `speed_cms × dt_ms × sin_fx`,
/// which is centimetres times both of those divisors, so the numerator is carried
/// whole and divided by this once — rather than divided twice with a truncation
/// at each.
pub const DISTANCE_SCALE: i64 = 1_000 * SIN_ONE;

/// The scale [`crate::arena::state::MotionCarry::turn`] and
/// [`crate::arena::state::MotionCarry::speed`] are measured in: milliseconds in a
/// second, because both of those rates are per second and `dt` is milliseconds.
const RATE_SCALE: u64 = 1_000;

/// A full turn, millidegrees.
pub const FULL_TURN_MDEG: u32 = 360_000;

/// Half a turn, millidegrees — the boundary between the two arcs.
pub const HALF_TURN_MDEG: u32 = 180_000;

/// A quarter turn, millidegrees — the table's span, and the offset from sine to
/// cosine.
pub const QUARTER_TURN_MDEG: u32 = 90_000;

/// The fixed-point scale of [`SIN_FX`]: `1 << 20`, so a table value of
/// [`SIN_ONE`] is exactly 1.0.
pub const SIN_ONE: i64 = 1 << 20;

/// Millidegrees between two [`SIN_FX`] samples: half a degree.
pub const SIN_STEP_MDEG: u32 = 500;

/// **The sine table**: `sin(k × 0.5°) × `[`SIN_ONE`], for `k` in `0..=180` — one
/// quadrant, from which the other three follow by symmetry ([`sin_fx`]).
///
/// Half a degree with linear interpolation between samples: the interpolation
/// error of a sine over an interval `h` radians is at most `h² / 8`, which at
/// half a degree is under `1 × 10⁻⁵`, so a unit crossing the whole 2.56 km window
/// is placed within a centimetre of the ideal arc. The samples are literals
/// because `f64::sin` is not a `const fn` and, more to the point, because a table
/// in the source is the same table on every platform.
pub const SIN_FX: [i64; 181] = [
    0, 9150, 18300, 27449, 36595, 45738, 54878, 64014, 73145, 82270, 91389, 100502, 109606, 118702,
    127789, 136867, 145934, 154989, 164033, 173065, 182083, 191088, 200078, 209052, 218011, 226953,
    235878, 244785, 253673, 262542, 271391, 280220, 289027, 297812, 306574, 315313, 324028, 332718,
    341383, 350022, 358634, 367219, 375776, 384304, 392803, 401273, 409711, 418119, 426494, 434837,
    443147, 451424, 459665, 467872, 476044, 484179, 492277, 500337, 508360, 516344, 524288, 532193,
    540057, 547879, 555661, 563399, 571095, 578748, 586356, 593920, 601438, 608911, 616338, 623717,
    631049, 638333, 645568, 652754, 659890, 666976, 674012, 680996, 687928, 694807, 701634, 708408,
    715127, 721792, 728402, 734957, 741455, 747897, 754282, 760610, 766880, 773091, 779244, 785337,
    791370, 797343, 803256, 809107, 814897, 820624, 826289, 831891, 837430, 842905, 848316, 853662,
    858943, 864159, 869309, 874393, 879410, 884360, 889243, 894058, 898805, 903484, 908093, 912634,
    917105, 921507, 925838, 930098, 934288, 938407, 942454, 946429, 950333, 954164, 957922, 961607,
    965219, 968758, 972223, 975614, 978930, 982172, 985339, 988431, 991448, 994389, 997255,
    1000045, 1002758, 1005395, 1007956, 1010440, 1012847, 1015176, 1017429, 1019604, 1021701,
    1023721, 1025662, 1027526, 1029311, 1031017, 1032646, 1034195, 1035666, 1037058, 1038371,
    1039605, 1040760, 1041836, 1042832, 1043749, 1044586, 1045344, 1046022, 1046620, 1047139,
    1047578, 1047937, 1048217, 1048416, 1048536, 1048576,
];

/// The sine of a heading in millidegrees, in [`SIN_ONE`] fixed point.
///
/// The heading is clockwise from north, so this is the **east** component of a
/// unit vector along it: `0` at north, `+`[`SIN_ONE`] at east, `0` at south,
/// `-`[`SIN_ONE`] at west. The quadrant is reduced first and the sign applied
/// after, so the table only ever holds one quadrant and the symmetry is exact
/// (the value at 90° is the value at 90° in every quadrant).
#[must_use]
pub fn sin_fx(heading_mdeg: u32) -> i64 {
    let mdeg = heading_mdeg % FULL_TURN_MDEG;
    let quadrant = mdeg / QUARTER_TURN_MDEG;
    let within = mdeg % QUARTER_TURN_MDEG;
    let magnitude = match quadrant {
        // 0..90 and 180..270 rise from the start of the quadrant; 90..180 and
        // 270..360 fall back toward it.
        0 | 2 => quadrant_sin_fx(within),
        _ => quadrant_sin_fx(QUARTER_TURN_MDEG - within),
    };
    if quadrant >= 2 { -magnitude } else { magnitude }
}

/// The cosine of a heading in millidegrees, in [`SIN_ONE`] fixed point — the
/// **north** component, which is the sine a quarter turn on.
#[must_use]
pub fn cos_fx(heading_mdeg: u32) -> i64 {
    sin_fx((heading_mdeg % FULL_TURN_MDEG) + QUARTER_TURN_MDEG)
}

/// `sin` of `0..=90_000` millidegrees, by linear interpolation between the two
/// nearest [`SIN_FX`] samples. Integer throughout, so the same input is the same
/// output everywhere.
fn quadrant_sin_fx(mdeg: u32) -> i64 {
    let mdeg = mdeg.min(QUARTER_TURN_MDEG);
    let sample = usize::try_from(mdeg / SIN_STEP_MDEG).unwrap_or(0);
    let frac = i64::from(mdeg % SIN_STEP_MDEG);
    let low = SIN_FX.get(sample).copied().unwrap_or(0);
    if frac == 0 {
        return low;
    }
    let high = SIN_FX.get(sample.saturating_add(1)).copied().unwrap_or(low);
    low + (high - low) * frac / i64::from(SIN_STEP_MDEG)
}

/// **Why a unit was stopped at a cell boundary** (16b cut F.2; doc 16 §3.4 step
/// 4's refusals, read as the things a pilot can be told — three of them until
/// 16b cut X.6 gave the slope its own word, which is four).
///
/// The cause and not the cell: doc 16 §3.6's observer rule is what fixes the
/// vocabulary here, because an occupied cell may hold a unit this platform has
/// not detected — so [`BlockedCause::Occupied`] says *something*, and the four
/// words are the whole of what leaves the arena. The fourth costs the rule
/// nothing: a slope is the ground's own fact, map knowledge a pilot could read
/// off the terrain himself, and it tells nobody about another unit.
///
/// **16b cut M.3** (16c `T-04`) put the cause on the ledger's `blocked` line,
/// where it serialises as the same word [`BlockedCause::as_str`] gives — the
/// `snake_case` derive below yields exactly those four, and a unit test holds
/// the two to each other. The ledger is the black box and may name anything; the
/// word is the same one the pilot's notice array already carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedCause {
    /// The ground ahead is impassable for this mode (doc 16 §3.4 step 4).
    ///
    /// The one-cell guard's refusal is read as this too: a jump longer than a
    /// cell is a configuration the substep length forbids, and what the pilot
    /// sees is a tank that will not go forward.
    Impassable,
    /// Another unit holds the cell (doc 16 §3.4 step 5's one unit per cell).
    Occupied,
    /// The cell is off the arena window (doc 16 §3.2 "Edges").
    Edge,
    /// **The edge between the two cells is steeper than this unit's locomotion
    /// will cross** (16b cut X.6; doc 16 §9 item 6).
    ///
    /// A fourth word rather than [`BlockedCause::Impassable`] because it is a
    /// different fact and a pilot can act on the difference: the ground ahead
    /// would take the tank, the *slope* onto it will not, so the answer is to
    /// go round rather than to give the cell up. It is also the one refusal a
    /// unit meets **leaving** a cell as well as entering one, and it names
    /// terrain and nothing else, so doc 16 §3.6's observer rule is untouched —
    /// the ground is map knowledge and tells nobody about another unit.
    TooSteep,
}

impl BlockedCause {
    /// The wire word, for the frame's own notice array.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            BlockedCause::Impassable => "impassable",
            BlockedCause::Occupied => "occupied",
            BlockedCause::Edge => "edge",
            BlockedCause::TooSteep => "too_steep",
        }
    }

    /// **The cause as one byte** (16b cut M.3), for
    /// [`crate::arena::hash::state_hash`]'s fold of [`UnitState::held`]: `0`
    /// impassable, `1` occupied, `2` edge, `3` too steep.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            BlockedCause::Impassable => 0,
            BlockedCause::Occupied => 1,
            BlockedCause::Edge => 2,
            BlockedCause::TooSteep => 3,
        }
    }
}

/// **What a unit is standing against** (16b cut M.3; 16c `T-04`, `B-27`, §5's
/// Q22): the cell it was refused and why — [`UnitState::held`]'s value.
///
/// The cell is [`MotionOutcome::blocked_at`]'s — the refused cell, or for the
/// window's edge the on-window cell nearest to the refused point: the cell the
/// unit is standing in for a push straight off the window, and possibly its
/// neighbour along the edge for a slanted push that also crossed a row or a
/// column — and the cause is
/// [`MotionOutcome::blocked_by`]'s; `stop_at_boundary` writes the two together,
/// so a stop always has both. All integers, like everything the arena stores.
///
/// It knows **what** held the unit because "held clears only on leaving the
/// cell" would get three cases wrong: a second, different obstacle met inside
/// the same cell (a bank, then a wreck after a turn) would be silent; a tank
/// reversing freely off the bank would keep `HELD` lit until it left the cell;
/// and a machine still pushing at a wreck could not be told apart from one that
/// had turned away, which is what its remembered stop is refreshed off
/// (`Arena::refresh_held_avoid`, at a motor step; at the reflex's tick until
/// 17b cut M4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Held {
    /// The refused cell's column.
    pub x: u16,
    /// The refused cell's row.
    pub y: u16,
    /// Why it was refused.
    pub cause: BlockedCause,
}

/// What one substep of [`integrate`] did to one unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MotionOutcome {
    /// The pose changed: the unit covered at least a whole centimetre.
    pub moved: bool,
    /// The unit was stopped at a cell boundary, naming the cell it could not
    /// enter — doc 16 §3.4 step 4's `blocked` notice.
    ///
    /// For a refusal **off the window** (doc 16 §3.2 "Edges") the cell named is
    /// the nearest on-window cell to the refused one, because a cell off the
    /// window has no coordinates: a unit driven north out of the top row names
    /// the top-row cell it stopped in.
    pub blocked_at: Option<(u16, u16)>,
    /// **Why** it was stopped (16b cut F.2), `None` when it was not.
    pub blocked_by: Option<BlockedCause>,
    /// **This stop is a new edge of [`UnitState::held`]** (16b cut M.3; 16c
    /// `T-04`): the refusal named a cell or a cause the unit was not already held
    /// against. It is the one condition the ledger's `blocked` line, the pilot's
    /// `Blocked` notice and the reflex controller's remembered stop all fire on
    /// (`Arena::substep`); a re-refusal of the same cell for the same cause is
    /// `false` and says nothing to any of them. Only [`integrate`] sets it.
    pub held_edge: bool,
}

/// **One substep of motion for one unit** (doc 16 §3.4, exactly).
///
/// `index` is the occupancy as it stands **now**, inside the substep: rebuilt at
/// the substep's start and updated by the caller through
/// [`SpatialIndex::move_unit`] after each unit that moved. That is the snapshot
/// rule for occupancy and it is what makes "two units cannot both enter one cell
/// in one substep, the lower id wins" true (doc 16 §3.4 step 5). Everything else
/// a unit reads — its own heading, speed and pose, the terrain — is the
/// substep-start snapshot.
///
/// `unit.blocked` is **set here and cleared here**: it is the per-substep flag
/// for *this substep*, read by tests and the harness, and since 16b cut M.3 it
/// journals nothing — its rising edge was a function of how the pulse was cut.
///
/// **The `held` rule** (16b cut F.2's condition; given its cell and cause and
/// written here, in one place, by 16b cut M.3 — 16c `T-04`, `B-27`, §5's Q22).
/// [`UnitState::held`] is `None` or the [`Held`] cell the unit was refused and
/// why:
///
/// - **Set.** A refused crossing sets it to the refused cell and cause
///   ([`MotionOutcome::blocked_at`] and [`MotionOutcome::blocked_by`], which
///   `stop_at_boundary` writes together). It is a **new edge** —
///   [`MotionOutcome::held_edge`] — when `held` was `None`, named a different
///   cause, or named a cell that is not the same obstacle: for another unit a
///   different cell, and for ground, a slope or the window's edge — which run on
///   across cells — a cell not adjacent to the held one (`same_stop` has the
///   measured reason: a slanted push names the diagonal or the orthogonal
///   neighbour depending on how the pulse was cut). A re-refusal of the same
///   obstacle is silent; the condition follows the cell it last named.
/// - **Clear**, three ways: the unit **enters another cell** on an accepted
///   crossing; it is **parked** (`desired_speed_cms == 0`, the E.3 clause); or
///   an accepted move inside its own cell points **strictly away** from the held
///   cell — the travel `(dx, dy)` in centimetres against the direction from the
///   unit's cell to the held one, in whole cells, has a negative dot product.
///   A slide along the bank is exactly zero and leaves it, a creep toward it
///   is positive and leaves it (and straight into it is a refusal, not a move),
///   and a reverse off it clears it
///   inside the cell. For the window's edge, whose refused point has no cell, the
///   direction is the window's outward normal on the axes the pose stands on
///   the window's boundary in (`moved_away` has the argument for both choices,
///   which is measured).
/// - **Otherwise it is left as it was**: a substep that covered no whole
///   centimetre — the case the 2026-09-18 smoke found, a tank against a river
///   bank reading `0 kph` with no `HELD` — and a traffic hold (below).
///
/// The new edge is the **one** condition three readers fire on (`Arena::substep`):
/// the ledger's `blocked` line (which carries the cause), the pilot's `Blocked`
/// notice, and the reflex controller's `Arena::note_blocked`. So one stop is one
/// line, one sentence and one lesson however the pulse is cut, and a second,
/// different obstacle met in the same cell — a bank, then a wreck after a turn —
/// is a second of each. `held` is folded into [`crate::arena::hash::state_hash`]:
/// it decides all three, and a replay that cleared it a substep earlier would
/// journal a stop the original did not.
///
/// **16b cut E.3's two parameters.** `end_ms` is the arena millisecond this
/// substep **ends** at — `now_ms + dt_ms` at the one production call site — and
/// `occupied_stop_ms` is `[tacsim.arena].occupied_stop_ms`. Together they are the
/// whole of the traffic hold: a hold opens and matures at arena timestamps, never
/// after a count of substeps, so nothing about how a pulse was cut decides when a
/// tank stops (doc 16 §3.1, D-16-2). `occupied_stop_ms == 0` is the rule as it
/// stood before the cut: every occupancy refusal is a stop at once.
///
/// A hold leaves [`UnitState::held`] as it was: it is not a stop and it covered
/// no ground, so the condition stays exactly as it was (unless the order has come
/// off, when the parked clause clears it as it would any substep) — which is what
/// keeps a tank pushing at a wreck telling its pilot one line and not one every
/// two seconds.
pub fn integrate(
    unit: &mut UnitState,
    dt_ms: u32,
    terrain: &ArenaTerrain,
    index: &SpatialIndex,
    end_ms: i64,
    occupied_stop_ms: u32,
) -> MotionOutcome {
    let before = (unit.pose.x_cm, unit.pose.y_cm);
    let here = terrain.cell_of(&unit.pose);
    let mut outcome = advance(unit, dt_ms, terrain, index, end_ms, occupied_stop_ms);
    // **The `held` rule** (16b cut M.3), settled once, at the one exit — see
    // this function's head for the whole of it.
    if let (Some((x, y)), Some(cause)) = (outcome.blocked_at, outcome.blocked_by) {
        // Set: the refused cell and why. A re-refusal of the same obstacle —
        // the same cell, or for ground and the window's edge a neighbouring cell
        // of the same cause (`same_stop`) — is no edge; the condition follows
        // the cell the latest refusal named.
        let refused = Held { x, y, cause };
        outcome.held_edge = !unit.held.is_some_and(|held| same_stop(held, refused));
        unit.held = Some(refused);
    } else if unit.desired_speed_cms == 0 {
        // Clear: parked. A tank no longer being ordered anywhere is not being
        // held against anything (the E.3 clause).
        unit.held = None;
    } else if outcome.moved
        && let Some(held) = unit.held
    {
        let after = terrain.cell_of(&unit.pose);
        let travel = (
            unit.pose.x_cm.saturating_sub(before.0),
            unit.pose.y_cm.saturating_sub(before.1),
        );
        // Clear: into another cell on an accepted crossing, or strictly away
        // from the held cell inside this one.
        if after != here || moved_away(held, here, before, travel, terrain) {
            unit.held = None;
        }
    }
    outcome
}

/// **Whether a refusal is the obstacle the unit is already held against**
/// (16b cut M.3; the M.3 review's F1 and R1) — the negation of
/// [`MotionOutcome::held_edge`].
///
/// The cause must match. Then:
///
/// - **Another unit** ([`BlockedCause::Occupied`]) is one cell, so only that cell
///   is the same stop: a second hull beside the first is a second obstacle, and
///   the machine's avoid list has to learn it.
/// - **Ground, a slope and the window's edge** run on across cells, so a refusal
///   of the held cell **or any cell adjacent to it** is the same stop. The
///   refused cell of a slanted push depends on how the pulse was cut: a stop
///   clamps both axes into the unit's cell, so a push into a west bank at 300°
///   slides the tank to its cell's corner, where a full substep's travel names
///   the diagonal neighbour and a pulse tail's — which floors to `-1` on the
///   axis into the bank and `0` on the other — names the orthogonal one. Keyed
///   on the exact cell, that alternation was two `blocked` lines, two notices
///   and two lessons every pulse for as long as the order stood; keyed on the
///   feature it is one (61.8 s of that push at the bank journalled 101 lines).
///   Every cell a unit can be refused from its own cell is one of that cell's
///   eight neighbours, and two of them are adjacent exactly when they share a
///   side or a corner of it: a turn that pushes the same bank round an L's
///   corner is the same stop, and a turn onto ground across the cell is a new
///   one. The condition follows the latest cell, so a clear is measured
///   against the obstacle the unit last pushed at.
fn same_stop(held: Held, refused: Held) -> bool {
    if held.cause != refused.cause {
        return false;
    }
    match refused.cause {
        BlockedCause::Occupied => held.x == refused.x && held.y == refused.y,
        BlockedCause::Impassable | BlockedCause::Edge | BlockedCause::TooSteep => {
            held.x.abs_diff(refused.x) <= 1 && held.y.abs_diff(refused.y) <= 1
        }
    }
}

/// **Whether an accepted move inside the unit's own cell pointed strictly away
/// from what holds it** (16b cut M.3): the travel's dot product with the
/// direction *from the cell it stands in to the held cell*, in whole cells, is
/// negative.
///
/// The direction is cell to cell and not pose to the held cell's centre, and
/// that is measured rather than taste. A stop clamps the pose to the boundary
/// and a pulse's tail moves the tank a centimetre *along* the bank (a fraction
/// of a centimetre south is minus one under the Euclidean division that
/// carries the travel), so the pose soon sits past the held cell's centre line:
/// against the pose, that slide reads as "away", clears the condition, and the
/// next refusal of the same bank is a new edge — thirteen pulses of the B-27
/// driver journalled fourteen lines and told the pilot fourteen times. Against
/// the cell-to-cell direction a slide along the bank is exactly zero and leaves
/// the condition, a creep toward it is positive and leaves it, and a reverse off
/// it is negative and clears it.
///
/// **The window's edge has no refused cell** — [`MotionOutcome::blocked_at`]
/// names the on-window cell nearest the refused point, which is the unit's own
/// cell for a push straight off the window and can be its neighbour along the
/// edge for a slanted one — so its direction is the window's outward normal
/// **on the axes the pose stands on the window's boundary in**: `-1` on an axis
/// whose pose is at its first centimetre, `+1` at its last, `0` otherwise. A
/// stop clamps the axis that left the window to exactly that centimetre, and
/// nothing but a move inward (which is away) takes it off it, so the normal is
/// the axis that refused and not every edge the cell touches. Taken from the
/// cell's row and column instead, the normal in a corner cell was the diagonal:
/// a tank pushed east in the north-east corner cell at 120° slid a centimetre
/// south in every pulse tail, that read as away, and every pulse was a new
/// stop (the M.3 review's R1).
fn moved_away(
    held: Held,
    here: Option<(u16, u16)>,
    before: (i64, i64),
    travel: (i64, i64),
    terrain: &ArenaTerrain,
) -> bool {
    let Some((cx, cy)) = here else {
        return false;
    };
    let (nx, ny) = if held.cause == BlockedCause::Edge {
        let side = i64::from(terrain.cell_cm());
        let outward = |at: i64, cells: u16| -> i64 {
            let last = i64::from(cells).saturating_mul(side).saturating_sub(1);
            if at <= 0 {
                -1
            } else if at >= last {
                1
            } else {
                0
            }
        };
        (
            outward(before.0, terrain.width()),
            outward(before.1, terrain.height()),
        )
    } else {
        (
            i64::from(held.x) - i64::from(cx),
            i64::from(held.y) - i64::from(cy),
        )
    };
    travel
        .0
        .saturating_mul(nx)
        .saturating_add(travel.1.saturating_mul(ny))
        < 0
}

/// Doc 16 §3.4's four steps, in order — everything [`integrate`] does except
/// settle [`UnitState::held`].
fn advance(
    unit: &mut UnitState,
    dt_ms: u32,
    terrain: &ArenaTerrain,
    index: &SpatialIndex,
    end_ms: i64,
    occupied_stop_ms: u32,
) -> MotionOutcome {
    unit.blocked = false;
    if dt_ms == 0 {
        return MotionOutcome::default();
    }
    let dt = i64::from(dt_ms);
    // **16b cut E.3**: a unit that is no longer being driven anywhere is not being
    // held against anything, which is [`UnitState::held`]'s own rule one field
    // over. It is cleared here, before the substep, so a coasting tank that is
    // refused on its way to a stop opens no hold it could never mature.
    if unit.desired_speed_cms == 0 {
        unit.occupied_hold = None;
    }

    // ---- 1. the heading turns toward the ordered one, shortest arc ----
    turn(unit, dt);

    // ---- 2. the speed moves toward the ordered one ----
    let cell = terrain.cell_of(&unit.pose);
    // **16b cut X.6**: the cell the unit is standing in is kept, not merely its
    // cost — step 4 needs it to price the edge it is about to cross, and fetching
    // it twice would be two reads of the same immutable window.
    let from_cell = cell.and_then(|(x, y)| terrain.get(x, y));
    // **16b cut AE.1** (16c `B-58`): the price this unit's own row pays for the
    // cell — the tracked `move_cost` for a tank and every engine fixture, the
    // class table's for a car or a walker on priced ground.
    let move_cost = from_cell.map_or(0, |cell| terrain.price(unit.cost_row, cell));
    // The slope the unit entered this cell by surcharges the cell's own cost: a
    // tank half way up a ramp is held below the flat ceiling. At grade zero the
    // surcharge is the identity, so level ground accelerates exactly as before.
    accelerate(
        unit,
        dt,
        slope::effective_cost(u32::from(move_cost), u32::from(unit.grade_pct)),
    );

    // ---- 3. the position advances along the heading ----
    // **16b cut Q.6**: `speed × dt × sin` is centimetres times a thousand times
    // [`SIN_ONE`], so the numerator is taken whole, the carry is added to it, and
    // the one division answers both the whole centimetres and what is left. The
    // magnitudes stay inside `i64` by a wide margin — the shipped ceiling is
    // 1,500 cm/s over a 250 ms substep, which is under 4 × 10¹¹ — and the
    // multiplications saturate rather than trusting that.
    let (speed_cms, heading_mdeg) = (unit.speed_cms, unit.pose.heading_mdeg);
    let east = axis(speed_cms, dt, sin_fx(heading_mdeg), &mut unit.carry.east);
    let north = axis(speed_cms, dt, cos_fx(heading_mdeg), &mut unit.carry.north);
    // No whole centimetre either way, so no boundary to test — **after** the two
    // carries have been banked, which is what lets a crawl accumulate instead of
    // standing still for ever.
    if east == 0 && north == 0 {
        return MotionOutcome::default();
    }
    let Some(from) = cell else {
        // A unit that is not on the window cannot be integrated onto it: it is
        // stopped where it stands rather than teleported home. `Arena::new`
        // refuses a pose off the window, so this is unreachable through the
        // shipped lane and is written as a refusal rather than an assumption.
        unit.speed_cms = 0;
        // **16b cut Q.6**: a refused substep banks no distance it never travelled.
        unit.carry.speed = 0;
        unit.carry.speed_rate = 0;
        unit.carry.east = 0;
        unit.carry.north = 0;
        unit.blocked = true;
        // 16b cut E.3: a stop of any other cause ends whatever hold there was.
        unit.occupied_hold = None;
        return MotionOutcome {
            moved: false,
            blocked_at: Some((0, 0)),
            blocked_by: Some(BlockedCause::Edge),
            held_edge: false,
        };
    };
    // The move is made **in place** and undone by clamping on a refusal, rather
    // than built as a second `Pose`: a pose carries the opaque posture, and
    // cloning it once per unit per substep would be the most expensive thing in
    // the pulse and would buy nothing — [`stop_at_boundary`] clamps from exactly
    // the moved coordinates.
    unit.pose.x_cm = unit.pose.x_cm.saturating_add(east);
    unit.pose.y_cm = unit.pose.y_cm.saturating_add(north);

    // ---- 4. a crossing tests the cell entered ----
    match terrain.cell_of(&unit.pose) {
        Some(to) if to == from => {
            // 16b cut E.3: a substep that moved and was not refused is a substep
            // the unit is standing against nothing.
            unit.occupied_hold = None;
            MotionOutcome {
                moved: true,
                blocked_at: None,
                blocked_by: None,
                held_edge: false,
            }
        }
        Some(to) => {
            // **16b cut X.6**: the cell entered is fetched **once** here and read
            // three times below — by the refusal, by the elevation and by the
            // grade. Before this cut it was read in `enterable` and again for
            // `z_cm`.
            let crossing = Crossing::between(terrain, from, to, unit.cost_row);
            match refusal(unit, index, &crossing) {
                None => {
                    // Doc 16 §3.2: `z_cm` is the cell's `elev_cm`, carried on entry.
                    if let Some(entered) = crossing.to_cell {
                        unit.pose.z_cm = i32::from(entered.elev_cm);
                    }
                    // **16b cut X.6**: and the slope it came up, which the next
                    // substep's terrain factor is measured against.
                    unit.grade_pct = u16::try_from(crossing.grade_pct()).unwrap_or(u16::MAX);
                    // 16b cut E.3: the crossing was accepted, so whatever it was
                    // standing against it is not standing against it now.
                    unit.occupied_hold = None;
                    MotionOutcome {
                        moved: true,
                        blocked_at: None,
                        blocked_by: None,
                        held_edge: false,
                    }
                }
                // **16b cut E.3**: another unit in the way is *held against* for
                // `occupied_stop_ms` of arena time before it becomes doc 16 §3.4
                // step 4's stop. Everything else — impassable ground, a slope,
                // more than one cell away — stops at the boundary at once, and
                // ends any hold that was open.
                Some(BlockedCause::Occupied) => {
                    let on_this_cell = unit.occupied_hold.is_some_and(|hold| hold.cell == to);
                    // The hold on **this** cell has outlasted the key — or the key
                    // is zero, which is the rule as it stood before this cut.
                    let mature = occupied_stop_ms == 0
                        || unit.occupied_hold.is_some_and(|hold| {
                            hold.cell == to
                                && end_ms.saturating_sub(hold.since_ms)
                                    >= i64::from(occupied_stop_ms)
                        });
                    if mature {
                        unit.occupied_hold = None;
                        return stop_at_boundary(unit, terrain, from, to, BlockedCause::Occupied);
                    }
                    // A unit that is no longer being driven anywhere is not
                    // pushing at anything — it is coasting to a stop of its own,
                    // and a clock it can never run out is a clock that should not
                    // be started. It holds at the boundary and opens nothing.
                    if unit.desired_speed_cms == 0 {
                        unit.occupied_hold = None;
                        return hold_at_boundary(unit, terrain, from);
                    }
                    if !on_this_cell {
                        // A refusal of a *different* cell is a different hold, and
                        // the clock starts again from this substep's end.
                        unit.occupied_hold = Some(OccupiedHold {
                            cell: to,
                            since_ms: end_ms,
                        });
                    }
                    hold_at_boundary(unit, terrain, from)
                }
                Some(cause) => {
                    unit.occupied_hold = None;
                    stop_at_boundary(unit, terrain, from, to, cause)
                }
            }
        }
        // Off the window (doc 16 §3.2 "Edges").
        None => {
            let nearest = nearest_on_window(terrain, &unit.pose);
            unit.occupied_hold = None;
            stop_at_boundary(unit, terrain, from, nearest, BlockedCause::Edge)
        }
    }
}

/// **One axis of step 3's travel, with its remainder carried** (16b cut Q.6).
///
/// `num = speed_cms × dt × fixed + carry`, then one Euclidean division by
/// [`DISTANCE_SCALE`]: the quotient is the whole centimetres this substep covers
/// along that axis and the remainder — always in `0..DISTANCE_SCALE`, which is
/// what `rem_euclid` gives for a negative numerator too — is what it did not.
/// Reversing therefore carries as cleanly as driving forward: the quotient floors
/// toward negative infinity and the carry makes up the difference next substep.
fn axis(speed_cms: i32, dt: i64, fixed: i64, carry: &mut i64) -> i64 {
    let numerator = i64::from(speed_cms)
        .saturating_mul(dt)
        .saturating_mul(fixed)
        .saturating_add(*carry);
    *carry = numerator.rem_euclid(DISTANCE_SCALE);
    numerator.div_euclid(DISTANCE_SCALE)
}

/// Step 1: turn toward `desired_heading_mdeg` at `turn_rate_mdeg_s × dt / 1000`
/// on the shortest arc, **blocked toward a closed side**.
///
/// Clockwise (millidegrees increasing) is toward the unit's **right**;
/// anticlockwise is toward its **left**. A destroyed left track is
/// `no_turn_toward(Left)` ([`crate::arena::state::MotionLimits::no_turn_left`])
/// and forbids the anticlockwise direction outright, so a turn whose short arc
/// goes that way **takes the long way round** — which is exactly what a tank on
/// one track does.
///
/// **The pair rule** (16b cut M.4, 16c `B-47`): the two sides are two flags,
/// ORed in by damage, so a tank that has lost **both** tracks has both closed,
/// and then it **does not turn at all** and its turn carry is zeroed. Before
/// M.4 one `Option<Side>` held the side, the second track's loss overwrote the
/// first, and a tank on no tracks still turned one way; the branch below was
/// unreachable then and is the pair rule now. The shipped tanks additionally
/// author `disable_motion_when_all_of: [left_track, right_track]`, so the same
/// tank is `motion_disabled` too and the first return below answers first.
///
/// A wreck and a unit whose motion is disabled do not turn at all: doc 16 §3.3
/// revokes `tactics.mobility` with the wreck, and a destroyed engine is
/// `disable_motion`.
///
/// **16b cut Q.6**: the rate's remainder is carried on
/// [`crate::arena::state::MotionCarry::turn`], so a turn of 30°/s is 30° of arc
/// over a second however that second is cut into substeps. It is zeroed on every
/// substep that does **not** turn — a wreck, a dead engine, a cold hull, an arc
/// already closed, both sides closed — and on the substep that closes the arc,
/// because the fraction left over then belongs to a turn that is finished.
///
/// **16b cut M.4 (16c `T-16`)**: and on the substep that turns the **other
/// way** from the one that banked it
/// ([`crate::arena::state::MotionCarry::turn_side`]): a fraction of a
/// millidegree spent turning left is not owed to a turn to the right.
fn turn(unit: &mut UnitState, dt: i64) {
    if unit.limits.wreck || unit.limits.motion_disabled || unit.power != Power::Online {
        unit.carry.turn = 0;
        unit.carry.turn_side = 0;
        return;
    }
    let current = unit.pose.heading_mdeg % FULL_TURN_MDEG;
    let desired = unit.desired_heading_mdeg % FULL_TURN_MDEG;
    let clockwise_arc = (desired + FULL_TURN_MDEG - current) % FULL_TURN_MDEG;
    if clockwise_arc == 0 {
        unit.carry.turn = 0;
        unit.carry.turn_side = 0;
        return;
    }
    let short_is_clockwise = clockwise_arc <= HALF_TURN_MDEG;
    let mut toward = if short_is_clockwise {
        Side::Right
    } else {
        Side::Left
    };
    if unit.limits.turn_closed(toward) {
        toward = match toward {
            Side::Right => Side::Left,
            Side::Left => Side::Right,
        };
    }
    // The pair rule: both sides closed, so neither way round is open.
    if unit.limits.turn_closed(toward) {
        unit.carry.turn = 0;
        unit.carry.turn_side = 0;
        return;
    }
    // 16c `T-16`: a carry banked turning the other way is not this turn's.
    let side_code = match toward {
        Side::Left => 1,
        Side::Right => 2,
    };
    if unit.carry.turn_side != side_code {
        unit.carry.turn = 0;
        unit.carry.turn_side = side_code;
    }
    let arc = match toward {
        Side::Right => clockwise_arc,
        Side::Left => FULL_TURN_MDEG - clockwise_arc,
    };
    let budget = u64::from(unit.mobility.turn_rate_mdeg_s)
        .saturating_mul(u64::try_from(dt).unwrap_or(0))
        .saturating_add(u64::from(unit.carry.turn));
    let step = u32::try_from(budget / RATE_SCALE)
        .unwrap_or(u32::MAX)
        .min(arc);
    // The arc is closed: what is left of the budget belongs to a turn that is over.
    unit.carry.turn = if step == arc {
        0
    } else {
        u32::try_from(budget % RATE_SCALE).unwrap_or(0)
    };
    unit.pose.heading_mdeg = match toward {
        Side::Right => (current + step) % FULL_TURN_MDEG,
        Side::Left => (current + FULL_TURN_MDEG - step) % FULL_TURN_MDEG,
    };
}

/// Step 2: the speed moves toward the ordered speed at `accel_cms2` — or
/// `brake_cms2` when the **magnitude** of the speed falls — capped by
/// `max_speed_cms × 100 / move_cost × slow_pct / 100` forward and by the reverse
/// ceiling scaled the same way, and **forced to zero** by a wreck, a disabled
/// engine or power that is not online.
///
/// The cap's divisions are taken in the order doc 16 §3.4 step 2 writes them:
/// the terrain factor `100 / move_cost` first, then the `slow` consequence. At
/// the stock tank's 1,500 cm/s that is 1,500 → 600 in heavy woods (cost 250) →
/// 300 with a destroyed track (`slow(50)`).
///
/// **16b cut X.6**: `move_cost` arrives already surcharged for the slope, so the
/// terrain factor is `100 / (move_cost × (100 + grade) / 100)` with `grade` the
/// uphill slope of the edge by which the unit entered the cell it is standing in
/// ([`crate::arena::state::UnitState::grade_pct`]). On the flat that is the
/// identity, so a level window caps exactly where it did before. The parameter
/// widened from `u16` to `u32` with the surcharge; a `move_cost` of zero still
/// caps at zero, because `checked_div` by zero is what answers it.
///
/// An impassable `move_cost` of zero caps the speed at zero: a unit standing on
/// a cell nothing may enter cannot drive out of it. Motion never puts one there
/// (step 4), and since **16b cut M.4** (16c `T-14`) `Arena::new` refuses a
/// check-out that would (`ArenaError::CellImpassable`), so the zero cap is
/// reached only by a fixture that builds an arena by hand.
///
/// **The cap bounds the order, not the speed directly.** A tank that drives off a
/// road into heavy woods, or loses a track mid-charge, does not drop to the new
/// ceiling in one substep: the ceiling lowers the *target* and the fall to it is
/// `brake_cms2` like any other, which is what doc 16 §3.4 step 2 says by putting
/// the rate on the change and the cap on the value it is moving toward. Clamping
/// the current speed instead would be a discontinuity — 900 cm/s in 250 ms at the
/// stock tank, eighteen times its braking rate — and it would arrive exactly when
/// a tank is taking damage or changing terrain, which is when a pilot is watching.
/// The speed can therefore sit **above** the momentary cap while it comes down; it
/// can never sit above `max_speed_cms`, because the order is clamped to that
/// (`apply_intent`) and nothing else raises it, so the substep's one-cell bound
/// (doc 16 §3.1) still holds.
///
/// **16b cut Q.6**: the rate's remainder is carried on
/// [`crate::arena::state::MotionCarry::speed`], so a ramp at 200 cm/s² reaches the
/// same speed however the arena time is cut. It is zeroed when the speed is
/// already at its target, when this substep reaches it, and on the wreck / dead
/// engine / cold power return — which also zeroes the two travel carries, because
/// a tank that has just been stopped dead did not travel the fraction they hold.
///
/// **16b cut M.4 (16c `T-16`)**: and on the substep whose rate is the other one
/// from the rate that banked it ([`crate::arena::state::MotionCarry::speed_rate`]:
/// accelerating against braking) — a thousandth of a cm/s banked at `accel_cms2`
/// is not owed to `brake_cms2`.
fn accelerate(unit: &mut UnitState, dt: i64, move_cost: u32) {
    if unit.limits.wreck || unit.limits.motion_disabled || unit.power != Power::Online {
        unit.speed_cms = 0;
        unit.carry.speed = 0;
        unit.carry.speed_rate = 0;
        unit.carry.east = 0;
        unit.carry.north = 0;
        // **16b cut E.3**: and a tank that has just been stopped dead is not
        // holding against anybody either — the hold is a thing a unit with way on
        // does, and this one has none.
        unit.occupied_hold = None;
        return;
    }
    let scaled = |ceiling: u32| -> i32 {
        let Some(terrain) = u64::from(ceiling)
            .saturating_mul(100)
            .checked_div(u64::from(move_cost))
        else {
            return 0;
        };
        let slowed = terrain.saturating_mul(u64::from(unit.limits.slow_pct)) / 100;
        i32::try_from(slowed).unwrap_or(i32::MAX)
    };
    let forward = scaled(unit.mobility.max_speed_cms);
    let reverse = scaled(unit.mobility.reverse_ceiling_cms()).saturating_neg();
    let target = unit.desired_speed_cms.clamp(reverse, forward);
    let current = unit.speed_cms;
    if target == current {
        unit.carry.speed = 0;
        unit.carry.speed_rate = 0;
        return;
    }
    let braking = target.abs() < current.abs();
    let rate = if braking {
        unit.mobility.brake_cms2
    } else {
        unit.mobility.accel_cms2
    };
    // 16c `T-16`: a carry banked by the other rate is not this change's.
    let rate_code = if braking { 2 } else { 1 };
    if unit.carry.speed_rate != rate_code {
        unit.carry.speed = 0;
        unit.carry.speed_rate = rate_code;
    }
    let budget = u64::from(rate)
        .saturating_mul(u64::try_from(dt).unwrap_or(0))
        .saturating_add(u64::from(unit.carry.speed));
    let delta = i32::try_from(budget / RATE_SCALE).unwrap_or(i32::MAX);
    unit.carry.speed = u32::try_from(budget % RATE_SCALE).unwrap_or(0);
    unit.speed_cms = if target > current {
        current.saturating_add(delta).min(target)
    } else {
        current.saturating_sub(delta).max(target)
    };
    // The target is reached: what is left of the budget belongs to a ramp that is
    // over, exactly as the closed arc's does.
    if unit.speed_cms == target {
        unit.carry.speed = 0;
    }
}

/// **The one boundary a substep crosses**, fetched once (16b cut X.6).
///
/// Doc 16 §3.4 step 4 asks four questions of the same pair of cells — is it one
/// step, is it passable, is it empty, is the edge onto it too steep — and then
/// reads the entered cell twice more for the elevation and the grade. The pair
/// is therefore assembled once, here, and passed down; before cut X.6 the
/// entered cell was fetched in the refusal and again for `z_cm`.
struct Crossing<'a> {
    /// The cell the unit is leaving.
    from: (u16, u16),
    /// The cell it is entering.
    to: (u16, u16),
    /// `from`'s terrain, or `None` when it is off the window.
    from_cell: Option<&'a TerrainCell>,
    /// `to`'s terrain, or `None` when it is off the window.
    to_cell: Option<&'a TerrainCell>,
    /// The step, or `None` when the two are not neighbours.
    kind: Option<StepKind>,
    /// The window's cell size, centimetres — the slope's run.
    cell_cm: u32,
    /// **Whether the crossing unit's own row may enter `to`** (16b cut AE.1):
    /// [`ArenaTerrain::passable_for`] with the unit's row, answered once here
    /// because the class table lives on the terrain and not on the cell.
    to_passable: bool,
}

impl<'a> Crossing<'a> {
    /// **The crossing `from` to `to` over `terrain`**: the two cells fetched once,
    /// the [`StepKind`] and the window's `cell_cm`. One constructor for
    /// [`advance`] and for `step_refusal` (16b cut M.4), so the crossing a test
    /// asks about is the crossing motion builds. **16b cut AE.1**: and whether
    /// `row` — the crossing unit's — may enter `to` at all.
    fn between(
        terrain: &'a ArenaTerrain,
        from: (u16, u16),
        to: (u16, u16),
        row: CostRow,
    ) -> Crossing<'a> {
        Crossing {
            from,
            to,
            from_cell: terrain.get(from.0, from.1),
            to_cell: terrain.get(to.0, to.1),
            kind: slope::step_kind(from, to),
            cell_cm: u32::from(terrain.cell_cm()),
            to_passable: terrain.passable_for(row, to.0, to.1),
        }
    }

    /// The **uphill** slope of this crossing, per cent; `0` for a step that is
    /// not one step, or either side of which is off the window.
    fn grade_pct(&self) -> u32 {
        match (self.from_cell, self.to_cell, self.kind) {
            (Some(from), Some(to), Some(kind)) => slope::grade_pct(from, to, kind, self.cell_cm),
            _ => 0,
        }
    }

    /// The slope of this crossing **whichever way it is taken**, per cent, which
    /// is what a limit is compared against; `0` in the same three cases.
    fn steepness_pct(&self) -> u32 {
        match (self.from_cell, self.to_cell, self.kind) {
            (Some(from), Some(to), Some(kind)) => {
                slope::steepness_pct(from, to, kind, self.cell_cm)
            }
            _ => 0,
        }
    }
}

/// Whether `to` may be entered from `from`: it is at most one cell away on each
/// axis, it is passable, the edge onto it is within this unit's slope limit, and
/// it is either empty or already held by this unit.
///
/// **The one-cell guard.** The substep length is
/// `min(substep_max_ms, (cell_cm - 1) × 1000 / max_speed_cms)` (doc 16 §3.1,
/// tightened by 16b cut Q.6), so one substep's travel on either axis is at most
/// `cell_cm - 1` — the carry included, because [`axis`] adds it *inside* the one
/// division — and a unit covers at most one cell in a substep, so step 4 has at
/// most one boundary to test. A configuration whose floor beat that bound could
/// offer a longer jump, and the cells in between would never be tested — so a jump
/// of more than one cell is **refused** rather than trusted, which keeps the
/// invariant true of the code and not merely of the shipped constants.
///
/// **Since 16b cut M.4 the front door is the refusal** (16c `B-57`, `T-15`):
/// `Arena::new` refuses a match whose true speed ceiling
/// (`Arena::max_speed_cms`, which scales for a cell cheaper than 100 as a
/// conservative margin over the order clamp) would beat
/// the substep floor, with `ArenaError::SubstepBound`, so no shipped or
/// configurable match reaches this guard. It stays, belt and braces, for a
/// fixture that builds an arena by hand and drives `integrate` at a substep of
/// its own choosing.
///
/// A **diagonal** crossing tests the diagonal neighbour and nothing else: the
/// unit cuts the corner and never occupies either edge-adjacent cell, so neither
/// is entered and neither needs testing.
/// **16b cut F.2**: the same questions, answering *which* of them said no so
/// that doc 16 §3.4 step 4's notice can name a cause. `None` is "enterable",
/// which is the predicate this replaced.
///
/// The order is the order a pilot would work it out in — the ground, then the
/// slope, then the traffic — and the one-cell guard is folded into the ground's
/// answer for the reason [`BlockedCause::Impassable`] gives. A crossing of more
/// than one cell has no [`StepKind`], and the slope test therefore adds no
/// second reason for refusing it: it has already been refused.
///
/// **16b cut X.6's conjunct, and why the limit is symmetric.** The edge is
/// refused when [`crate::arena::slope::steepness_pct`] — the slope of the edge
/// *whichever way it is crossed* — passes the unit's limit, so a descent too
/// steep to climb is refused too. `mush_atlas::physics::move_cost` refuses only
/// the climb, which is right for a world where a step is asked for one at a
/// time and wrong for an arena: an uphill-only limit makes any bluff a one-way
/// door into a cell the unit can never drive out of, and a reflex controller
/// would re-plan at the wall of that cell every `repath_ms` until the match
/// ended. The *surcharge* stays uphill-only, because a descent is not slower
/// than the flat.
///
/// A unit with no limit at all (`slope_limit_pct: None` — one whose content
/// declares no locomotion the shipped table knows) is refused by no slope and
/// still pays the surcharge.
fn refusal(
    unit: &UnitState,
    index: &SpatialIndex,
    crossing: &Crossing<'_>,
) -> Option<BlockedCause> {
    let (from, to) = (crossing.from, crossing.to);
    let one_step = from.0.abs_diff(to.0) <= 1 && from.1.abs_diff(to.1) <= 1;
    // **16b cut AE.1** (16c `B-58`): passable **to this unit's row**. A cell the
    // row prices impassable is the same word, the same ledger line and the same
    // pilot's sentence as a tank at deep water: the ground will not take it.
    let passable = crossing.to_cell.is_some() && crossing.to_passable;
    if !one_step || !passable {
        return Some(BlockedCause::Impassable);
    }
    if slope::too_steep(unit.slope_limit_pct, crossing.steepness_pct()) {
        return Some(BlockedCause::TooSteep);
    }
    match index.occupant(to.0, to.1) {
        Some(holder) if holder != unit.id => Some(BlockedCause::Occupied),
        _ => None,
    }
}

/// **The crossing refusal for one step, as motion would answer it** (16b cut
/// M.4, 16c `T-17`): `from` to `to` for `unit` over `terrain` and `index`, with
/// the [`Crossing`] built exactly as [`advance`] builds it — the two cells, the
/// [`StepKind`] and the window's `cell_cm` — and then [`refusal`]. `None` is
/// "motion would accept it".
///
/// It exists so that the one statement about the search and motion that needs
/// both can be held by a test: `crate::arena::path`'s **shoulder rule** is the
/// safe direction, so every step a route the search finds takes is one this
/// accepts (the search refuses diagonals motion allows, never the reverse).
/// [`refusal`] takes the private [`Crossing`], so making it crate-visible alone
/// would give the search's tests nothing they could call.
///
/// Compiled for tests only: its one caller is that property, and a
/// crate-visible function nothing in the build calls is dead code the lints
/// refuse. The crossing it builds is `Crossing::between`, the constructor
/// [`advance`] itself calls.
#[cfg(test)]
pub(crate) fn step_refusal(
    unit: &UnitState,
    index: &SpatialIndex,
    terrain: &ArenaTerrain,
    from: (u16, u16),
    to: (u16, u16),
) -> Option<BlockedCause> {
    refusal(
        unit,
        index,
        &Crossing::between(terrain, from, to, unit.cost_row),
    )
}

/// Stop the unit at the boundary it could not cross: the pose is clamped into the
/// cell it already occupies ([`clamp_into_cell`]), the speed goes to zero, and the
/// stop is reported against `refused`.
///
/// **16b cut Q.6**: and the three motion carries go with the speed. The rule is
/// [`crate::arena::heat`]'s — a carry belongs to arena time that was actually
/// spent — and a refused substep spent none: banking the fraction of a centimetre
/// the unit was clamped back out of would push it over the boundary on the next
/// substep without a second test.
fn stop_at_boundary(
    unit: &mut UnitState,
    terrain: &ArenaTerrain,
    from: (u16, u16),
    refused: (u16, u16),
    cause: BlockedCause,
) -> MotionOutcome {
    clamp_into_cell(unit, terrain, from);
    unit.speed_cms = 0;
    unit.carry.speed = 0;
    unit.carry.speed_rate = 0;
    unit.carry.east = 0;
    unit.carry.north = 0;
    unit.blocked = true;
    MotionOutcome {
        moved: false,
        blocked_at: Some(refused),
        blocked_by: Some(cause),
        held_edge: false,
    }
}

/// **Hold the unit at the boundary it could not cross, keeping its way** (16b cut
/// E.3; 16c `B-46`, §5's Q10) — the traffic half of doc 16 §3.4 step 4.
///
/// The pose is clamped into the cell the unit already occupies exactly as
/// [`stop_at_boundary`] clamps it, **both** axes, so the invariant this module's
/// head states is as true of a hold as of a stop: no unit ever occupies a cell
/// that was not tested on entry. The two travel carries go with it, for
/// [`stop_at_boundary`]'s reason and this one more: a clamped fraction of a
/// centimetre would cross the boundary on the next substep without being tested
/// again.
///
/// **What it keeps is `speed_cms` and `carry.speed`**, which is the whole point —
/// a follower in a column that is held for a substep and let through on the next
/// drives on at the speed it had, rather than rebuilding fifteen metres a second
/// from nothing at every cell of the ford. [`UnitState::blocked`] stays `false`,
/// nothing is reported against the outcome, so the substep journals no `blocked`
/// line, stages no notice and teaches no machine: to everything downstream this
/// is a substep in which a unit did not move.
///
/// **What a pilot's frame reads during a hold, decided** (16b cut E.3).
/// `Frame::speed_cms` reads [`UnitState::speed_cms`], which a hold keeps, and
/// `Frame::held` (16b cut M.3's name for the field) reads [`UnitState::held`],
/// which a hold does not set: so for
/// up to `occupied_stop_ms` the pilot carries the speed they had, a pose that is
/// not advancing, and no `HELD`. That is deliberate and it is not a new channel.
/// The pilot of a tank stopped dead by another unit is sent `Held: something is
/// in the way.` — a sentence only [`BlockedCause::Occupied`] produces — and at
/// HEAD that arrived on the *first* substep, so "a unit and not ground" was
/// already disclosed, sooner than any reading of a frozen pose could disclose it;
/// the hold tells nobody anything earlier or more than before, and the sentence
/// stays `something`. The alternative, reporting `0` while holding, would make
/// the speed of every tank following another in column flicker to zero for a
/// substep at a time.
///
/// **What keeping the heading and the way costs, recorded** (16b cut E.3). A hold
/// clamps the pose but leaves `heading_mdeg` alone, so a unit refused a cell while
/// it is on a **diagonal** heading is parked at the corner of its own cell with
/// full way on, and the first substep whose other axis also crosses enters the
/// *diagonal* neighbour instead — tested on entry like every other crossing, so
/// the invariant this module's head states is untouched, but the unit has gone
/// round the obstacle rather than waited behind it. That is not new in kind: at
/// HEAD a tank under a standing speed order stopped, rebuilt its way and crept
/// the same way round a second or two later. The hold makes it a substep or two
/// instead. Where a machine is steering a route the route is what decides and the
/// drift is a cell of it; where a pilot is driving, the tank goes where it is
/// pointed. Terrain refusals are unchanged, so 16c §6's "sliding along an obstacle
/// after a terrain refusal" is still not built: this is traffic, and it is the
/// reason `a_stop_a_wreck_and_a_cold_hull_all_zero_the_carries` runs its
/// [`BlockedCause::Occupied`] row at `occupied_stop_ms = 0`.
fn hold_at_boundary(
    unit: &mut UnitState,
    terrain: &ArenaTerrain,
    from: (u16, u16),
) -> MotionOutcome {
    clamp_into_cell(unit, terrain, from);
    unit.carry.east = 0;
    unit.carry.north = 0;
    MotionOutcome {
        moved: false,
        blocked_at: None,
        blocked_by: None,
        held_edge: false,
    }
}

/// Clamp a pose that has just left `from` back into it — **both** axes, so a
/// refused diagonal cannot deposit the unit in an untested edge-adjacent cell.
///
/// "At the boundary" is the last whole centimetre of its own cell, because
/// [`ArenaTerrain::cell_of`] gives a boundary to the cell it opens: at 1,000 cm
/// cells, column 3 is `3_000..=3_999`. It is the one piece [`stop_at_boundary`]
/// and [`hold_at_boundary`] share, and sharing it is what makes "a hold leaves the
/// unit exactly where a stop would have" true by construction rather than by two
/// copies agreeing.
fn clamp_into_cell(unit: &mut UnitState, terrain: &ArenaTerrain, from: (u16, u16)) {
    let side = i64::from(terrain.cell_cm());
    let low_x = i64::from(from.0).saturating_mul(side);
    let low_y = i64::from(from.1).saturating_mul(side);
    let moved_x = unit.pose.x_cm;
    let moved_y = unit.pose.y_cm;
    unit.pose.x_cm = moved_x.clamp(low_x, low_x.saturating_add(side).saturating_sub(1));
    unit.pose.y_cm = moved_y.clamp(low_y, low_y.saturating_add(side).saturating_sub(1));
}

/// The on-window cell nearest to a pose that is off it — each axis clamped into
/// the window. It is what [`MotionOutcome::blocked_at`] names for an edge refusal,
/// where the refused cell itself has no coordinates.
fn nearest_on_window(terrain: &ArenaTerrain, pose: &Pose) -> (u16, u16) {
    let side = i64::from(terrain.cell_cm().max(1));
    let clamp = |value: i64, limit: u16| -> u16 {
        let cell = value.div_euclid(side);
        let high = i64::from(limit).saturating_sub(1).max(0);
        u16::try_from(cell.clamp(0, high)).unwrap_or(0)
    };
    (
        clamp(pose.x_cm, terrain.width()),
        clamp(pose.y_cm, terrain.height()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{Arena, ArenaTerrain, MotionLimits, Team, TerrainCell};
    use crate::arena::test_support::{arena_of, ground, pose, tank};
    use mush_core::ObjId;

    const A: ObjId = ObjId(4);
    const B: ObjId = ObjId(9);
    /// A third hull, parked (16b cut E.3).
    const C: ObjId = ObjId(11);

    /// An arena whose units are online and ordered nowhere.
    fn running(units: Vec<crate::arena::state::CustodyUnit>) -> Arena {
        let mut arena = arena_of(units);
        for unit in &mut arena.units {
            unit.power = Power::Online;
        }
        arena
    }

    fn index_of(arena: &Arena) -> SpatialIndex {
        let mut index = SpatialIndex::new(&arena.definition.terrain);
        index.rebuild(&arena.definition.terrain, &arena.units);
        index
    }

    /// **The shipped `[tacsim.arena].occupied_stop_ms`** (16b cut E.3), which
    /// [`step_one`] drives under.
    const OCCUPIED_STOP_MS: u32 = 2_000;

    /// One substep for one unit, with the index rebuilt from the current poses,
    /// under the shipped traffic hold.
    ///
    /// **16b cut E.3**: the arena's own `now_ms` is the clock — this advances it
    /// by the substep it just ran, so `end_ms` is arena time and a hold against
    /// another unit matures after `occupied_stop_ms` of it. A fixture that drives
    /// two units through one conceptual substep therefore advances the clock
    /// twice; none of those measures arena time, and every fixture that measures
    /// a hold drives one unit.
    fn step_one(arena: &mut Arena, id: ObjId, dt_ms: u32) -> MotionOutcome {
        step_under(arena, id, dt_ms, OCCUPIED_STOP_MS)
    }

    /// [`step_one`] under a named `[tacsim.arena].occupied_stop_ms` — `0` is the
    /// rule as it stood before 16b cut E.3.
    fn step_under(
        arena: &mut Arena,
        id: ObjId,
        dt_ms: u32,
        occupied_stop_ms: u32,
    ) -> MotionOutcome {
        let index = index_of(arena);
        let terrain = arena.definition.terrain.clone();
        let end_ms = arena.now_ms.saturating_add(i64::from(dt_ms));
        arena.now_ms = end_ms;
        let Some(unit) = arena.unit_mut(id) else {
            return MotionOutcome::default();
        };
        integrate(unit, dt_ms, &terrain, &index, end_ms, occupied_stop_ms)
    }

    /// `count` substeps of `dt_ms` each — the shape a real pulse has, and the
    /// only shape in which a unit may cross a cell (a single long substep would
    /// jump several and be refused by design).
    fn run(arena: &mut Arena, id: ObjId, count: u32, dt_ms: u32) {
        for _ in 0..count {
            step_one(arena, id, dt_ms);
        }
    }

    /// **Drive `total_ms` of arena time through one partition of it** (16b cut
    /// Q.6): pulses of `pulse_ms`, each cut into substeps of at most
    /// `substep_max_ms` by [`crate::arena::clock::substeps`] — the real partition
    /// machinery, so a ragged pulse produces the ragged tail a live one does.
    ///
    /// Answers how many **stops** the drive produced, counted the way
    /// [`crate::arena::step`] journals its `blocked` line: on the new edge of
    /// `UnitState::held` (16b cut M.3), once per stop rather than once per
    /// substep a standing order holds the unit against the obstacle.
    fn drive(
        arena: &mut Arena,
        id: ObjId,
        total_ms: u32,
        pulse_ms: u32,
        substep_max_ms: u32,
    ) -> (usize, usize) {
        drive_under(
            arena,
            id,
            total_ms,
            pulse_ms,
            substep_max_ms,
            OCCUPIED_STOP_MS,
        )
    }

    /// [`drive`] under a named `[tacsim.arena].occupied_stop_ms`.
    ///
    /// **16b cut M.3** (16c `T-04`): the first count is now what
    /// `crate::arena::step` journals — [`MotionOutcome::held_edge`], the one
    /// edge the `blocked` line, the pilot's notice and the machine's lesson share
    /// — and the second is an independent witness, the rising edge of `held`'s
    /// presence, which is what the pilot's notice fired on before the cut. For a
    /// single obstacle the two must agree.
    fn drive_under(
        arena: &mut Arena,
        id: ObjId,
        total_ms: u32,
        pulse_ms: u32,
        substep_max_ms: u32,
        occupied_stop_ms: u32,
    ) -> (usize, usize) {
        let mut spent = 0_u32;
        let mut journalled = 0_usize;
        let mut told = 0_usize;
        let mut was_held = false;
        while spent < total_ms {
            let pulse = pulse_ms.min(total_ms - spent);
            for dt in crate::arena::clock::substeps(pulse, substep_max_ms) {
                let outcome = step_under(arena, id, dt, occupied_stop_ms);
                if outcome.held_edge {
                    journalled += 1;
                }
                if outcome.blocked_by.is_some() && !was_held {
                    told += 1;
                }
                was_held = arena.unit(id).is_some_and(|unit| unit.held.is_some());
                spent = spent.saturating_add(dt);
            }
        }
        (journalled, told)
    }

    /// The four partitions every Q.6 property is asserted over: three uniform
    /// substep ceilings driven by a 5,000 ms pulse, and a **ragged** one — pulses
    /// of 370 ms, which is not a multiple of any substep ceiling, so every pulse
    /// ends in a short tail. That tail is exactly what X.6's measurement caught.
    const PARTITIONS: [(u32, u32); 4] = [(5_000, 250), (5_000, 150), (5_000, 50), (370, 250)];

    #[test]
    fn the_sine_table_is_pinned_at_the_angles_everyone_knows() {
        assert_eq!(SIN_FX.len(), 181);
        assert_eq!(sin_fx(0), 0);
        assert_eq!(sin_fx(30_000), SIN_ONE / 2, "sin 30 is a half, exactly");
        assert_eq!(sin_fx(45_000), 741_455);
        assert_eq!(sin_fx(60_000), 908_093);
        assert_eq!(sin_fx(90_000), SIN_ONE);
        assert_eq!(sin_fx(150_000), SIN_ONE / 2);
        assert_eq!(sin_fx(180_000), 0);
        assert_eq!(sin_fx(210_000), -SIN_ONE / 2);
        assert_eq!(sin_fx(270_000), -SIN_ONE);
        assert_eq!(sin_fx(330_000), -SIN_ONE / 2);
        // A full turn round, and past it.
        assert_eq!(sin_fx(360_000), 0);
        assert_eq!(sin_fx(360_000 + 90_000), SIN_ONE);
        // Cosine is the quarter turn on: north, east, south, west.
        assert_eq!(cos_fx(0), SIN_ONE);
        assert_eq!(cos_fx(90_000), 0);
        assert_eq!(cos_fx(180_000), -SIN_ONE);
        assert_eq!(cos_fx(270_000), 0);
        // Interpolated between samples, and monotone across the first quadrant.
        assert_eq!(sin_fx(250), (SIN_FX[1] - SIN_FX[0]) / 2);
        let mut previous = -1_i64;
        for mdeg in (0..=90_000).step_by(137) {
            let value = sin_fx(mdeg);
            assert!(value >= previous, "sin fell at {mdeg} millidegrees");
            previous = value;
        }
        // And the unit circle holds to a thousandth: sin^2 + cos^2 = 1.
        for mdeg in (0..360_000).step_by(1_111) {
            let s = sin_fx(mdeg);
            let c = cos_fx(mdeg);
            let sum = s * s + c * c;
            let one = SIN_ONE * SIN_ONE;
            assert!(
                (sum - one).abs() < one / 1_000,
                "sin^2 + cos^2 is {sum} against {one} at {mdeg}"
            );
        }
    }

    #[test]
    fn a_turn_takes_the_shortest_arc_at_the_ordered_rate() {
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        // 30 degrees a second: a 250 ms substep turns 7.5 degrees.
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_heading_mdeg = 90_000;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(7_500));
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(15_000));
        // The short arc west is anticlockwise, through zero.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 10_000;
            unit.desired_heading_mdeg = 350_000;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(2_500));
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(355_000));
        // It never overshoots the order.
        step_one(&mut arena, A, 10_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(350_000));
        step_one(&mut arena, A, 10_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(350_000));
    }

    #[test]
    fn a_destroyed_track_turns_the_long_way_round() {
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.limits.no_turn_left = true;
            // The short arc to 350 degrees is anticlockwise — to the left, which
            // the destroyed track forbids — so it goes the long way, clockwise.
            unit.desired_heading_mdeg = 350_000;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(30_000));
        // The mirror: a destroyed right track sends a starboard order to port.
        // (16b cut M.4: the two sides are two flags now, so the mirror opens the
        // left again by hand — damage never would; both closed is the pair rule,
        // `both_tracks_gone_is_no_turn_at_all_and_no_carry`.)
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 0;
            unit.desired_heading_mdeg = 90_000;
            unit.limits.no_turn_left = false;
            unit.limits.no_turn_right = true;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(
            arena.unit(A).map(|u| u.pose.heading_mdeg),
            Some(330_000),
            "the long way round to starboard is to port"
        );
        // A unit that cannot turn at all holds its heading: the rate, not the
        // rule, is what stops it.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 0;
            unit.desired_heading_mdeg = 90_000;
            unit.limits.no_turn_right = false;
            unit.mobility.turn_rate_mdeg_s = 0;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(0));
    }

    /// **16b cut M.4 (16c `B-47`): the pair rule.** With both sides closed the
    /// tank does not turn at all, whichever way the order points and however long
    /// it is driven, and the turn carry it had banked is zeroed — the branch in
    /// `turn` that was unreachable while one `Option<Side>` held the side.
    #[test]
    fn both_tracks_gone_is_no_turn_at_all_and_no_carry() {
        for desired in [90_000, 270_000, 180_000] {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.limits.close_turn(Side::Left);
                unit.limits.close_turn(Side::Right);
                unit.desired_heading_mdeg = desired;
                unit.carry.turn = 700;
                unit.carry.turn_side = 2;
            }
            for _ in 0..8 {
                step_one(&mut arena, A, 250);
            }
            let state = arena.unit(A).unwrap_or_else(|| panic!("A"));
            assert_eq!(state.pose.heading_mdeg, 0, "it turned toward {desired}");
            assert_eq!(
                (state.carry.turn, state.carry.turn_side),
                (0, 0),
                "and kept a carry it can never spend"
            );
        }
        // One side alone is still the long way round (the test above), and
        // closing a side twice is closing it once.
        let mut limits = MotionLimits::default();
        limits.close_turn(Side::Left);
        limits.close_turn(Side::Left);
        assert!(limits.turn_closed(Side::Left) && !limits.turn_closed(Side::Right));
    }

    /// **16b cut M.4 (16c `T-16`): a turn carry dies on a reversal.** A carry
    /// banked turning left is zeroed on the substep that turns right, and the
    /// side is recorded; a carry banked the same way is spent as before. The rate
    /// times the substep is whole (30,000 mdeg/s × 250 ms), so whatever carry is
    /// left after the substep is the one it started with, or nothing.
    #[test]
    fn a_turn_carry_dies_when_the_turn_reverses() {
        for (banked_side, kept) in [(1_u8, false), (2_u8, true)] {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                // Ordered to starboard: the short arc is clockwise, side 2.
                unit.desired_heading_mdeg = 90_000;
                unit.carry.turn = 700;
                unit.carry.turn_side = banked_side;
            }
            step_one(&mut arena, A, 250);
            let state = arena.unit(A).unwrap_or_else(|| panic!("A"));
            assert_eq!(state.pose.heading_mdeg, 7_500);
            assert_eq!(
                state.carry.turn,
                if kept { 700 } else { 0 },
                "banked on side {banked_side}"
            );
            assert_eq!(state.carry.turn_side, 2, "the new side is recorded");
        }
    }

    /// **16b cut M.4 (16c `T-16`): a speed carry dies on a flip between the two
    /// rates.** A carry banked accelerating is zeroed on the substep that brakes;
    /// one banked braking is kept. 400 cm/s² × 250 ms is whole, so the carry
    /// after the substep is the one it started with, or nothing.
    #[test]
    fn a_speed_carry_dies_when_accelerating_turns_to_braking() {
        for (banked_rate, kept) in [(1_u8, false), (2_u8, true)] {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.speed_cms = 500;
                unit.desired_speed_cms = 0;
                unit.carry.speed = 800;
                unit.carry.speed_rate = banked_rate;
            }
            step_one(&mut arena, A, 250);
            let state = arena.unit(A).unwrap_or_else(|| panic!("A"));
            assert_eq!(state.speed_cms, 400, "one substep of braking");
            assert_eq!(
                state.carry.speed,
                if kept { 800 } else { 0 },
                "banked at rate {banked_rate}"
            );
            assert_eq!(state.carry.speed_rate, 2, "the new rate is recorded");
        }
    }

    #[test]
    fn speed_accelerates_at_the_ordered_rate_and_brakes_faster() {
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        // The stock tank: 1,000 cm/s, 200 cm/s^2 accelerating, 400 braking.
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 1_000;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(200));
        step_one(&mut arena, A, 500);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(300));
        // Braking to a stop uses the brake rate.
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 0;
        }
        step_one(&mut arena, A, 500);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(100));
        step_one(&mut arena, A, 500);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
        // Never past the order: one substep reaches it and the next holds it.
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 150;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(150));
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(150));
    }

    #[test]
    fn the_terrain_factor_and_the_slow_consequence_cap_the_speed() {
        let mut cells = vec![ground(); 64];
        // A band of heavy woods at cost 250 down column 4.
        for y in 0..8_u16 {
            if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 4) {
                cell.move_cost = 250;
            }
        }
        let mut arena = running(vec![tank(A, Team::Red, pose(4_500, 3_500))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the woods window");
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 1_000;
        }
        // 1,000 x 100 / 250 = 400 cm/s in heavy woods, reached and held. The unit
        // drives north up the wooded column, so the factor never changes.
        run(&mut arena, A, 20, 250);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(400));
        assert_eq!(
            arena
                .definition
                .terrain
                .cell_of(&arena.unit(A).map_or(pose(0, 0), |u| u.pose.clone()))
                .map(|(x, _)| x),
            Some(4),
            "the unit left the wooded column and the cap changed under the test"
        );
        // A destroyed track halves it again — and **braking is what takes it
        // there**, one substep at a time, rather than the cap clamping the speed
        // down in one jump. At 400 cm/s² and 250 ms that is 100 cm/s a substep, so
        // 400 → 300 → 200, and the assertion is the intermediate value as much as
        // the endpoint: a clamp would answer 200 on the first substep.
        if let Some(unit) = arena.unit_mut(A) {
            unit.limits.slow_pct = 50;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(300));
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(200));
        // And it holds there rather than overshooting the new ceiling.
        run(&mut arena, A, 4, 250);
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(200));
    }

    #[test]
    fn a_ceiling_that_drops_under_way_is_reached_at_the_braking_rate() {
        // The other half: not a consequence but the ground. A tank at its ceiling on
        // open road crossing into heavy woods (cost 250, cap 400) brakes down to the
        // new cap; it does not teleport its speed there, and it is allowed to be
        // above the momentary cap while it comes down.
        let mut cells = vec![ground(); 64];
        for y in 0..8_u16 {
            for x in 4..8_u16 {
                if let Some(cell) = cells.get_mut(usize::from(y) * 8 + usize::from(x)) {
                    cell.move_cost = 250;
                }
            }
        }
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the woods window");
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        // One substep takes it into the woods at its full 1,000 cm/s — the cap of
        // the cell it was **in** is what governed that substep.
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_750));
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(1_000));
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(4_000));
        // Now in the woods: braking at 400 cm/s² is 100 cm/s a substep.
        for expected in [900, 800, 700, 600, 500, 400, 400] {
            step_one(&mut arena, A, 250);
            assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(expected));
        }
    }

    #[test]
    fn a_wreck_a_dead_engine_and_cold_power_all_stop_dead() {
        for limits in [
            MotionLimits {
                wreck: true,
                ..MotionLimits::default()
            },
            MotionLimits {
                motion_disabled: true,
                ..MotionLimits::default()
            },
        ] {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.speed_cms = 900;
                unit.desired_speed_cms = 900;
                unit.desired_heading_mdeg = 90_000;
                unit.limits = limits;
            }
            let outcome = step_one(&mut arena, A, 250);
            assert_eq!(outcome, MotionOutcome::default());
            assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
            assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_500));
            assert_eq!(
                arena.unit(A).map(|u| u.pose.heading_mdeg),
                Some(0),
                "a unit that cannot move cannot turn either"
            );
        }
        // Power off: the same, and `Starting` is not online.
        for power in [Power::Off, Power::Starting] {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.power = power;
                unit.speed_cms = 900;
                unit.desired_speed_cms = 900;
            }
            step_one(&mut arena, A, 250);
            assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
            assert_eq!(arena.unit(A).map(|u| u.pose.y_cm), Some(3_500));
        }
    }

    #[test]
    fn the_position_advances_along_the_heading() {
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        // North at 1,000 cm/s for 250 ms is 250 cm of northing and no easting.
        let outcome = step_one(&mut arena, A, 250);
        assert!(outcome.moved);
        assert_eq!(
            arena.unit(A).map(|u| (u.pose.x_cm, u.pose.y_cm)),
            Some((3_500, 3_750))
        );
        // East.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(
            arena.unit(A).map(|u| (u.pose.x_cm, u.pose.y_cm)),
            Some((3_750, 3_750))
        );
        // South-west, on the diagonal: equal and opposite, at 250 / root 2.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 225_000;
            unit.desired_heading_mdeg = 225_000;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(
            arena.unit(A).map(|u| (u.pose.x_cm, u.pose.y_cm)),
            Some((3_573, 3_573))
        );
        // Reverse runs backwards along the heading, not along its opposite.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 0;
            unit.desired_heading_mdeg = 0;
            unit.speed_cms = -400;
            unit.desired_speed_cms = -400;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.y_cm), Some(3_473));
    }

    #[test]
    fn an_impassable_cell_stops_the_unit_at_the_boundary_with_one_notice() {
        let mut cells = vec![ground(); 64];
        // Column 4 is impassable.
        for y in 0..8_u16 {
            if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 4) {
                cell.move_cost = 0;
            }
        }
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the band window");
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        // Two substeps of 250 cm take it to 4,000 cm, which is the band.
        let first = step_one(&mut arena, A, 250);
        assert_eq!(first.blocked_at, None);
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_750));
        let second = step_one(&mut arena, A, 250);
        assert_eq!(second.blocked_at, Some((4, 3)));
        assert!(!second.moved);
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_999));
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
        assert_eq!(arena.unit(A).map(|u| u.blocked), Some(true));
        // The cell it stopped in was tested when it entered it.
        assert_eq!(
            arena.definition.terrain.cell_of(&pose(3_999, 3_500)),
            Some((3, 3))
        );
    }

    /// Two tanks nose to tail, one cell apart, under the shipped hold.
    fn nose_to_tail() -> Arena {
        let mut arena = running(vec![
            tank(A, Team::Red, pose(3_900, 3_500)),
            tank(B, Team::Blue, pose(4_500, 3_500)),
        ]);
        for unit in &mut arena.units {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        arena
    }

    /// **16b cut E.3 turned the first half of this into a hold.** A is in cell 3
    /// and B holds cell 4: A is refused, but a refusal by another *unit* keeps
    /// A's way and says nothing for `occupied_stop_ms` — so the outcome names no
    /// cell and no cause, `blocked` stays down, and the only thing the refusal
    /// costs A is the ground it did not cover. The **lower-id-wins** half is
    /// unchanged: A is still refused the cell B holds, and B still moves on.
    #[test]
    fn an_occupied_cell_holds_the_unit_and_the_lower_id_wins() {
        let mut arena = nose_to_tail();
        let outcome = step_one(&mut arena, A, 250);
        assert_eq!(outcome.blocked_at, None, "a hold journals nothing");
        assert_eq!(outcome.blocked_by, None, "and tells nobody");
        assert!(!outcome.moved);
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_999));
        assert_eq!(
            arena.unit(A).map(|u| u.speed_cms),
            Some(1_000),
            "a held unit keeps its way, which is the whole of the cut"
        );
        assert_eq!(arena.unit(A).map(|u| u.blocked), Some(false));
        assert_eq!(
            arena.unit(A).and_then(|u| u.occupied_hold),
            Some(OccupiedHold {
                cell: (4, 3),
                since_ms: 250
            })
        );
        let onward = step_one(&mut arena, B, 250);
        assert_eq!(onward.blocked_at, None);
        assert_eq!(arena.unit(B).map(|u| u.pose.x_cm), Some(4_750));
    }

    /// **`occupied_stop_ms = 0` is the rule as it stood before 16b cut E.3**, byte
    /// for byte: the old body of the test above, run under the key.
    #[test]
    fn at_a_zero_horizon_an_occupied_cell_stops_the_unit_at_once() {
        let mut arena = nose_to_tail();
        let outcome = step_under(&mut arena, A, 250, 0);
        assert_eq!(outcome.blocked_at, Some((4, 3)));
        assert_eq!(outcome.blocked_by, Some(BlockedCause::Occupied));
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_999));
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
        assert_eq!(arena.unit(A).map(|u| u.blocked), Some(true));
        assert_eq!(
            arena.unit(A).and_then(|u| u.held),
            Some(Held {
                x: 4,
                y: 3,
                cause: BlockedCause::Occupied
            })
        );
        assert_eq!(
            arena.unit(A).and_then(|u| u.occupied_hold),
            None,
            "a stop leaves no hold behind"
        );
        let onward = step_under(&mut arena, B, 250, 0);
        assert_eq!(onward.blocked_at, None);
        assert_eq!(arena.unit(B).map(|u| u.pose.x_cm), Some(4_750));
    }

    #[test]
    fn the_window_edge_stops_the_unit() {
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 7_500))]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        // Two substeps take it to 8,000 cm, which is off an 8-cell window.
        step_one(&mut arena, A, 250);
        let outcome = step_one(&mut arena, A, 250);
        assert_eq!(
            outcome.blocked_at,
            Some((3, 7)),
            "an edge refusal names the nearest on-window cell"
        );
        assert_eq!(arena.unit(A).map(|u| u.pose.y_cm), Some(7_999));
        assert_eq!(arena.unit(A).map(|u| u.speed_cms), Some(0));
    }

    #[test]
    fn entering_a_cell_carries_its_elevation() {
        let mut cells = vec![ground(); 64];
        if let Some(cell) = cells.get_mut(4 * 8 + 3) {
            *cell = TerrainCell {
                elev_cm: 1_200,
                ..ground()
            };
        }
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_900))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the hill window");
        if let Some(unit) = arena.unit_mut(A) {
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        assert_eq!(arena.unit(A).map(|u| u.pose.z_cm), Some(0));
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).map(|u| u.pose.y_cm), Some(4_150));
        assert_eq!(arena.unit(A).map(|u| u.pose.z_cm), Some(1_200));
    }

    #[test]
    fn a_jump_of_more_than_one_cell_is_refused_rather_than_trusted() {
        // A configuration no shipped substep can produce: 5,000 cm of travel in
        // one substep over 1,000 cm cells. The cells in between were never
        // tested, so the move is refused.
        let mut arena = running(vec![tank(A, Team::Red, pose(500, 3_500))]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
            unit.mobility.max_speed_cms = 20_000;
            unit.speed_cms = 20_000;
            unit.desired_speed_cms = 20_000;
        }
        let outcome = step_one(&mut arena, A, 250);
        assert_eq!(outcome.blocked_at, Some((5, 3)));
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(999));
    }

    /// **The invariant, walked** (doc 16 §3.4 step 4): a random walk over a window
    /// with an impassable band and two units, asserted after every substep.
    ///
    /// **16b cut E.3 put a third unit in the way and made the walk assert the
    /// hold too.** A hold clamps the pose exactly as a stop does, and this is the
    /// property that says so: a unit that is being held against another unit's
    /// cell for two seconds is a unit that has *not* crossed into it, at any
    /// substep of those two seconds. `C` is parked in the middle of the walk so
    /// that traffic refusals are certain rather than incidental, and the run
    /// asserts that at least one hold was really opened — a walk in which nothing
    /// ever held would pass this invariant and prove nothing about the cut.
    #[test]
    fn no_unit_ever_occupies_a_cell_that_was_not_tested_on_entry() {
        let mut cells = vec![ground(); 64];
        for y in 0..8_u16 {
            if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 5) {
                cell.move_cost = 0;
            }
        }
        let terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the walk window");
        let mut arena = running(vec![
            tank(A, Team::Red, pose(1_500, 1_500)),
            tank(B, Team::Blue, pose(2_500, 2_500)),
            // Parked: it is never stepped, so it is a hole in the map with a hull.
            tank(C, Team::Blue, pose(3_500, 2_500)),
        ]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain = terrain.clone();
        for unit in &mut arena.units {
            unit.desired_speed_cms = 1_000;
            unit.speed_cms = 1_000;
        }
        if let Some(parked) = arena.unit_mut(C) {
            parked.speed_cms = 0;
            parked.desired_speed_cms = 0;
        }
        // A deterministic pseudo-random heading per unit per substep, from the
        // arena's own draw primitive rather than a generator with state.
        let mut seed = 2_350_u64;
        let mut ever_held = false;
        for substep in 0..400_u32 {
            seed = crate::arena::rng::splitmix64(seed);
            let ids = [A, B];
            for (n, id) in ids.iter().enumerate() {
                if substep % 8 == 0 {
                    let spin = crate::arena::rng::splitmix64(seed ^ u64::try_from(n).unwrap_or(0));
                    if let Some(unit) = arena.unit_mut(*id) {
                        unit.desired_heading_mdeg =
                            u32::try_from(spin % u64::from(FULL_TURN_MDEG)).unwrap_or(0);
                    }
                }
                step_one(&mut arena, *id, 250);
                ever_held |= arena
                    .unit(*id)
                    .is_some_and(|unit| unit.occupied_hold.is_some());
                // The invariant, after every single integrate: on the window, on
                // a passable cell, and alone in it.
                let mut seen: Vec<(u16, u16)> = Vec::new();
                for unit in &arena.units {
                    let cell = terrain
                        .cell_of(&unit.pose)
                        .expect("a unit never leaves the window");
                    let tile = terrain.get(cell.0, cell.1).expect("an on-window cell");
                    assert!(
                        tile.passable(),
                        "unit {} stands on an impassable cell {cell:?}",
                        unit.id
                    );
                    assert!(
                        !seen.contains(&cell),
                        "two units share cell {cell:?} at substep {substep}"
                    );
                    seen.push(cell);
                }
            }
        }
        assert!(
            ever_held,
            "no unit was ever held against another's cell, so the walk said nothing about 16b \
             cut E.3's clamp"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut Q.6: the carries
    // -----------------------------------------------------------------------

    /// **A straight run, a pure turn and a speed ramp do not depend on how arena
    /// time is cut into substeps** (16b cut Q.6; doc 16 §3.1 "Rate independence").
    ///
    /// Four partitions of the same 100 seconds — 250, 150 and 50 ms ceilings under
    /// a 5,000 ms pulse, and a ragged 370 ms pulse whose every tail is short — and
    /// the three motions this module's head calls **exact** end identically in all
    /// four. The numbers are chosen so that each one would have failed before the
    /// carries: a crawl at 3 cm/s covers no whole centimetre in any of these
    /// substeps, a turn at 33.333°/s leaves a remainder in every one of them, and
    /// a ramp at 333 cm/s² likewise.
    ///
    /// The ramp's **pose** is deliberately not compared: a substep travels at the
    /// speed it ends with, so position under acceleration is first-order in the
    /// substep, which is the one thing the module head says the carries do *not*
    /// make exact.
    #[test]
    fn the_carries_make_a_run_a_turn_and_a_ramp_independent_of_the_substep() {
        // (a) the crawl: 3 cm/s for 100 s is 300 cm, and before the carries it was
        // nothing at all — `3 × 250 / 1000` is zero, and so is `3 × 50 / 1000`.
        let mut crawled: Vec<(i64, i64)> = Vec::new();
        for (pulse_ms, substep_max_ms) in PARTITIONS {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 90_000;
                unit.desired_heading_mdeg = 90_000;
                unit.speed_cms = 3;
                unit.desired_speed_cms = 3;
            }
            assert_eq!(
                drive(&mut arena, A, 100_000, pulse_ms, substep_max_ms),
                (0, 0)
            );
            crawled.push(
                arena
                    .unit(A)
                    .map_or((0, 0), |unit| (unit.pose.x_cm, unit.pose.y_cm)),
            );
        }
        assert!(
            crawled.iter().all(|pose| *pose == (3_800, 3_500)),
            "a crawl of 3 cm/s for 100 s is 300 cm at every partition: {crawled:?}"
        );

        // (b) the full-speed run: 1,000 cm/s due east for 4 s, across four cell
        // boundaries of open ground.
        let mut ran: Vec<(i64, i64)> = Vec::new();
        for (pulse_ms, substep_max_ms) in PARTITIONS {
            let mut arena = running(vec![tank(A, Team::Red, pose(500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 90_000;
                unit.desired_heading_mdeg = 90_000;
                unit.speed_cms = 1_000;
                unit.desired_speed_cms = 1_000;
            }
            assert_eq!(
                drive(&mut arena, A, 4_000, pulse_ms, substep_max_ms),
                (0, 0)
            );
            ran.push(
                arena
                    .unit(A)
                    .map_or((0, 0), |unit| (unit.pose.x_cm, unit.pose.y_cm)),
            );
        }
        assert!(
            ran.iter().all(|pose| *pose == (4_500, 3_500)),
            "a road run at 1,000 cm/s for 4 s is 4,000 cm at every partition: {ran:?}"
        );

        // (c) the turn: 33.333 degrees a second for 4 s is 133.332 degrees of a
        // half turn that is never closed, so what is compared is the arc itself
        // and not the order it is heading for.
        let mut turned: Vec<u32> = Vec::new();
        for (pulse_ms, substep_max_ms) in PARTITIONS {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.mobility.turn_rate_mdeg_s = 33_333;
                unit.desired_heading_mdeg = 180_000;
            }
            assert_eq!(
                drive(&mut arena, A, 4_000, pulse_ms, substep_max_ms),
                (0, 0)
            );
            turned.push(arena.unit(A).map_or(0, |unit| unit.pose.heading_mdeg));
        }
        assert!(
            turned.iter().all(|heading| *heading == 133_332),
            "33,333 mdeg/s for 4,000 ms is 133,332 mdeg at every partition: {turned:?}"
        );

        // (d) the ramp: 333 cm/s² for 2 s is 666 cm/s, mid-ramp, so the target is
        // never reached and the rate itself is what is compared.
        let mut ramped: Vec<i32> = Vec::new();
        for (pulse_ms, substep_max_ms) in PARTITIONS {
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.mobility.accel_cms2 = 333;
                unit.desired_speed_cms = 1_000;
            }
            drive(&mut arena, A, 2_000, pulse_ms, substep_max_ms);
            ramped.push(arena.unit(A).map_or(0, |unit| unit.speed_cms));
        }
        assert!(
            ramped.iter().all(|speed| *speed == 666),
            "333 cm/s² for 2,000 ms is 666 cm/s at every partition: {ramped:?}"
        );
    }

    /// **One obstacle is one stop, and the tank is held at the same centimetre,
    /// at every partition** (16b cut Q.6) — and the one thing that is still not
    /// partition-independent, measured rather than asserted away.
    ///
    /// A tank ordered east into an impassable band is stopped at the boundary and
    /// held there however the pulse is cut, and **the pilot is told once** at
    /// every partition, because the notice fires on the rising edge of the `held`
    /// condition (16b cut F.2) and that condition survives a substep in which
    /// nothing moved.
    ///
    /// **The ledger's `blocked` line did not, until 16b cut M.3** (16c `T-04`).
    /// It fired on the rising edge of the per-substep `blocked` **flag**, which
    /// `advance` clears at the top of every substep and a substep that covers no
    /// whole centimetre never sets again — so on a short tail substep a held tank
    /// re-accelerated from the zero a stop left it at, travelled under a
    /// centimetre, and made the next refusal a second rising edge: `[1, 6, 278,
    /// 1]` lines over the four partitions, which this test printed rather than
    /// asserted. Cut M.3 moved the line onto `held`'s new edge, so it is now
    /// asserted: **one line at every partition**, equal to the pilot's one line.
    #[test]
    fn one_obstacle_is_one_stop_at_every_partition() {
        let mut counts: Vec<(usize, usize)> = Vec::new();
        let mut poses: Vec<(i64, bool)> = Vec::new();
        for (pulse_ms, substep_max_ms) in PARTITIONS {
            let mut cells = vec![ground(); 64];
            for y in 0..8_u16 {
                if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 4) {
                    cell.move_cost = 0;
                }
            }
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            std::sync::Arc::make_mut(&mut arena.definition).terrain =
                ArenaTerrain::new(8, 8, 1_000, cells).expect("the band window");
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 90_000;
                unit.desired_heading_mdeg = 90_000;
                unit.desired_speed_cms = 1_000;
            }
            counts.push(drive(&mut arena, A, 30_000, pulse_ms, substep_max_ms));
            poses.push(
                arena
                    .unit(A)
                    .map_or((0, false), |unit| (unit.pose.x_cm, unit.held.is_some())),
            );
        }
        assert!(
            poses.iter().all(|end| *end == (3_999, true)),
            "the tank is held at the same centimetre at every partition: {poses:?}"
        );
        assert!(
            counts.iter().all(|count| count.1 == 1),
            "the pilot is told once at every partition: {counts:?}"
        );
        // 16b cut M.3: and the black box says it once too, at every partition.
        assert!(
            counts.iter().all(|count| count.0 == 1),
            "one `blocked` line at every partition: {counts:?}"
        );
    }

    /// An 8 by 8 window with an impassable band down column 4, and `A` in cell
    /// `(3, 3)`'s centre facing east — the band fixture of the partition test.
    fn m3_band(units: Vec<crate::arena::state::CustodyUnit>) -> Arena {
        let mut cells = vec![ground(); 64];
        for y in 0..8_u16 {
            if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 4) {
                cell.move_cost = 0;
            }
        }
        let mut arena = running(units);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the band window");
        arena
    }

    /// Point `A` at `heading_mdeg` from a standstill under a speed order.
    fn m3_order(arena: &mut Arena, heading_mdeg: u32, desired_speed_cms: i32) {
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = heading_mdeg;
            unit.desired_heading_mdeg = heading_mdeg;
            unit.speed_cms = 0;
            unit.carry = crate::arena::state::MotionCarry::default();
            unit.desired_speed_cms = desired_speed_cms;
        }
    }

    /// Step `A` until a substep reports `until`, answering that outcome.
    fn m3_until(arena: &mut Arena, until: fn(&MotionOutcome) -> bool) -> MotionOutcome {
        for _ in 0..40 {
            let outcome = step_under(arena, A, 250, 0);
            if until(&outcome) {
                return outcome;
            }
        }
        panic!("forty substeps and the awaited outcome never came");
    }

    fn m3_held(arena: &Arena) -> Option<Held> {
        arena.unit(A).and_then(|unit| unit.held)
    }

    /// **16b cut M.3's `held` rule, one clause at a time** (16c `T-04`, `B-27`,
    /// §5's Q22).
    ///
    /// **Set**: a refusal sets the refused cell and cause, and it is a new edge;
    /// a re-refusal of the same cell for the same cause is not. **Clear** three
    /// ways: an accepted move inside the cell strictly away from the held one
    /// (reversing off the bank); the speed order coming off; and entering another
    /// cell. A move **along** the bank leaves it. A second, different obstacle
    /// met in the same cell is a new edge. And the window's edge, which names the
    /// unit's own cell, is measured against the window's outward normal.
    #[test]
    fn m3_the_held_rule_sets_on_a_new_cell_or_cause_and_clears_three_ways() {
        let band = Held {
            x: 4,
            y: 3,
            cause: BlockedCause::Impassable,
        };
        // A parked tank north of A's cell, for the second obstacle.
        let mut arena = m3_band(vec![
            tank(A, Team::Red, pose(3_500, 3_500)),
            tank(B, Team::Blue, pose(3_500, 4_500)),
        ]);
        m3_order(&mut arena, 90_000, 1_000);
        let stop = m3_until(&mut arena, |out| out.blocked_by.is_some());
        assert!(stop.held_edge, "the first stop is an edge");
        assert_eq!(m3_held(&arena), Some(band));
        // Pushing on: every substep is refused the same cell for the same cause.
        for _ in 0..8 {
            let again = step_under(&mut arena, A, 250, 0);
            assert!(!again.held_edge, "a re-refusal is no edge: {again:?}");
            assert_eq!(m3_held(&arena), Some(band));
        }
        // Along the bank (due north, inside the cell): the travel's dot product
        // with (1, 0) is zero, and the condition stands.
        m3_order(&mut arena, 0, 1_000);
        let along = m3_until(&mut arena, |out| out.moved);
        assert!(!along.held_edge);
        assert_eq!(
            m3_held(&arena),
            Some(band),
            "a slide along the bank clears nothing"
        );
        // Reversing off it: strictly away, and cleared inside the cell.
        m3_order(&mut arena, 90_000, -400);
        m3_until(&mut arena, |out| out.moved);
        assert_eq!(m3_held(&arena), None, "a reverse off the bank clears it");
        assert_eq!(
            arena.definition.terrain.cell_of(&arena.units[0].pose),
            Some((3, 3)),
            "and the unit is still in its cell"
        );
        // Back at it: a new edge, because the condition was clear.
        m3_order(&mut arena, 90_000, 1_000);
        let back = m3_until(&mut arena, |out| out.blocked_by.is_some());
        assert!(back.held_edge);
        // A second obstacle in the same cell: north into the parked tank, which
        // is a different cell and a different cause — a new edge.
        m3_order(&mut arena, 0, 1_000);
        let other = m3_until(&mut arena, |out| out.blocked_by.is_some());
        assert!(other.held_edge, "a different obstacle is a new stop");
        assert_eq!(
            m3_held(&arena),
            Some(Held {
                x: 3,
                y: 4,
                cause: BlockedCause::Occupied
            })
        );
        // The order coming off clears it without a centimetre of travel.
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 0;
        }
        let parked = step_under(&mut arena, A, 250, 0);
        assert!(!parked.moved);
        assert_eq!(m3_held(&arena), None, "parked is not held");

        // Entering another cell: stopped at the band, then driven south until it
        // crosses into row 2.
        let mut arena = m3_band(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        m3_order(&mut arena, 90_000, 1_000);
        m3_until(&mut arena, |out| out.blocked_by.is_some());
        assert_eq!(m3_held(&arena), Some(band));
        m3_order(&mut arena, 180_000, 1_000);
        let mut crossed = false;
        for _ in 0..40 {
            step_under(&mut arena, A, 250, 0);
            let row = arena
                .unit(A)
                .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
                .map(|(_, y)| y);
            if row == Some(2) {
                crossed = true;
                break;
            }
            // Still in row 3, sliding south along the band: held.
            assert_eq!(m3_held(&arena), Some(band));
        }
        assert!(crossed, "the tank never left its row");
        assert_eq!(
            m3_held(&arena),
            None,
            "a crossing into another cell clears it"
        );

        // The window's edge: north out of the top row, then a slide east along
        // the edge (held), then a reverse south off it (cleared).
        let mut arena = m3_band(vec![tank(A, Team::Red, pose(1_500, 7_500))]);
        m3_order(&mut arena, 0, 1_000);
        let edge = m3_until(&mut arena, |out| out.blocked_by.is_some());
        assert!(edge.held_edge);
        let at_edge = Held {
            x: 1,
            y: 7,
            cause: BlockedCause::Edge,
        };
        assert_eq!(m3_held(&arena), Some(at_edge));
        m3_order(&mut arena, 90_000, 1_000);
        m3_until(&mut arena, |out| out.moved);
        assert_eq!(m3_held(&arena), Some(at_edge), "a slide along the edge");
        m3_order(&mut arena, 0, -400);
        m3_until(&mut arena, |out| out.moved);
        assert_eq!(m3_held(&arena), None, "a reverse off the edge");
    }

    /// A window of `ground()` with the columns in `impassable` refused, and `A`
    /// at `at` ordered at `heading_mdeg` and 1,000 cm/s from a standstill.
    fn m3_slanted(impassable: &[u16], at: (i64, i64), heading_mdeg: u32) -> Arena {
        let mut cells = vec![ground(); 64];
        for &x in impassable {
            for y in 0..8_u16 {
                if let Some(cell) = cells.get_mut(usize::from(y) * 8 + usize::from(x)) {
                    cell.move_cost = 0;
                }
            }
        }
        let mut arena = running(vec![tank(A, Team::Red, pose(at.0, at.1))]);
        std::sync::Arc::make_mut(&mut arena.definition).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the window");
        m3_order(&mut arena, heading_mdeg, 1_000);
        arena
    }

    /// One case of the slanted-push test: its name, the impassable columns, the
    /// start, the heading, how long it is driven and the cell it must end in.
    type SlantedCase = (
        &'static str,
        &'static [u16],
        (i64, i64),
        u32,
        u32,
        (u16, u16),
    );

    /// **A slanted push is one stop for as long as it lasts, at a bank, at the
    /// window's edge and in a corner cell** (16b cut M.3; the M.3 review's F1 and
    /// R1). Pulses of 1,030 ms — the production shape, four 250 ms substeps and a
    /// 30 ms tail — for a minute, long enough for the clamp to slide the tank to
    /// its cell's corner (twenty pulses in the corner cell).
    ///
    /// - A **west bank** at 300°: at the corner a full substep names the diagonal
    ///   neighbour and a tail the orthogonal one. Keyed on the exact cell that was
    ///   two lines a pulse (`same_stop`).
    /// - The **west window edge** at 300°: `nearest_on_window` varies the same
    ///   way.
    /// - The **north-east corner cell**, pushed east at 120°: every tail slides a
    ///   centimetre south. Against a normal taken from the corner cell's row and
    ///   column — the diagonal — that read as away and every pulse was a new stop
    ///   (`moved_away`).
    ///
    /// In each the tank ends in the cell it started in, still held, told once.
    #[test]
    fn m3_a_slanted_push_is_one_stop_at_a_bank_an_edge_and_a_corner_cell() {
        // The corner case runs twenty pulses, not sixty: at 120° the tank slides
        // south about thirty centimetres a pulse, and leaving the row would be a
        // legitimate second stop.
        let cases: [SlantedCase; 3] = [
            ("west bank", &[2], (3_500, 3_500), 300_000, 61_800, (3, 3)),
            ("west edge", &[], (500, 3_500), 300_000, 61_800, (0, 3)),
            (
                "north-east corner",
                &[],
                (7_950, 7_990),
                120_000,
                20_600,
                (7, 7),
            ),
        ];
        for (name, impassable, at, heading_mdeg, total_ms, cell) in cases {
            let mut arena = m3_slanted(impassable, at, heading_mdeg);
            let counts = drive_under(&mut arena, A, total_ms, 1_030, 250, 0);
            let unit = arena.unit(A).expect("the tank");
            assert_eq!(
                arena.definition.terrain.cell_of(&unit.pose),
                Some(cell),
                "{name}: the tank stays in its cell: {:?}",
                unit.pose
            );
            assert!(unit.held.is_some(), "{name}: still held");
            assert_eq!(counts, (1, 1), "{name}: one line and one notice");
        }
        // The bank's tank really did reach the corner, where the alternation
        // lives: the clamp holds it at the bank's centimetre and the top of its
        // row.
        let mut arena = m3_slanted(&[2], (3_500, 3_500), 300_000);
        drive_under(&mut arena, A, 61_800, 1_030, 250, 0);
        let unit = arena.unit(A).expect("the tank");
        assert_eq!((unit.pose.x_cm, unit.pose.y_cm), (3_000, 3_999));
    }

    /// **16b cut M.3**: a traffic hold (16b cut E.3) is neither a stop nor a
    /// move, so it leaves `held` exactly as it was — whatever it was — and it is
    /// no edge.
    #[test]
    fn m3_a_traffic_hold_leaves_held_as_it_was_and_is_no_edge() {
        let before = Held {
            x: 6,
            y: 6,
            cause: BlockedCause::Impassable,
        };
        for held in [None, Some(before)] {
            let mut arena = nose_to_tail();
            if let Some(unit) = arena.unit_mut(A) {
                unit.held = held;
            }
            let outcome = step_one(&mut arena, A, 250);
            assert!(
                arena
                    .unit(A)
                    .is_some_and(|unit| unit.occupied_hold.is_some()),
                "the fixture's first substep is a hold"
            );
            assert!(!outcome.held_edge);
            assert_eq!(outcome.blocked_at, None, "a hold journals nothing");
            assert_eq!(arena.unit(A).and_then(|unit| unit.held), held);
        }
    }

    /// **The substep bound leaves a centimetre for the carry, and a unit driven
    /// flat out from a cell edge is never refused one** (16b cut Q.6; the
    /// adversarial review's X-6).
    ///
    /// Two things, and it is worth being exact about which is which.
    ///
    /// The **constant** is the property: `substep_ms` is `min(substep_max_ms,
    /// (cell_cm - 1) × 1000 / max_speed_cms)`, so at 5,000 cm/s on 1,000 cm cells
    /// it is **199** and not 200, and one substep's travel is therefore strictly
    /// inside a cell rather than exactly a cell. It is a margin and not a repair:
    /// [`crate::arena::clock::Clock::substep_ms`]'s own comment re-derives why the
    /// old quotient could not produce a two-cell step either, and this test does
    /// **not** claim that it could.
    ///
    /// The **drive** is a regression pin on the arithmetic that margin protects:
    /// from one centimetre short of a cell edge, a minute at full speed crosses
    /// three hundred boundaries and is refused none of them, and the distance it
    /// covers is exact. Part (a) is the brief's own fixture, whose 5,000 cm/s
    /// divides `cell_cm × 1000` exactly and so never rolls a carry over at all;
    /// part (b) is the same run at 4,999, where every substep banks 801 thousandths
    /// of a centimetre and the rollover happens inside the cell crossings — which
    /// is the case (a) cannot reach.
    #[test]
    fn a_unit_at_the_edge_of_its_cell_at_full_speed_is_never_refused() {
        use crate::arena::clock::Clock;
        use crate::config::TempoConfig;

        let cfg = TempoConfig::default();
        assert_eq!(
            Clock::substep_ms(&cfg, 1_000, 5_000),
            199,
            "the bound is one centimetre short of a cell"
        );
        assert_eq!(
            Clock::substep_ms(&cfg, 1_000, 4_999),
            199,
            "and the same for the speed that does not divide it"
        );

        // A long, flat corridor: 320 cells of open ground, which 5,000 cm/s covers
        // 300 of in the minute below.
        let corridor = |speed: i32| -> Arena {
            let cells = vec![ground(); 320 * 4];
            let mut arena = running(vec![tank(A, Team::Red, pose(999, 1_500))]);
            std::sync::Arc::make_mut(&mut arena.definition).terrain =
                ArenaTerrain::new(320, 4, 1_000, cells).expect("the corridor window");
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 90_000;
                unit.desired_heading_mdeg = 90_000;
                unit.mobility.max_speed_cms = u32::try_from(speed).unwrap_or(0);
                unit.speed_cms = speed;
                unit.desired_speed_cms = speed;
            }
            arena
        };

        // (a) the brief's fixture: 5,000 cm/s, whose travel is whole centimetres.
        let mut exact = corridor(5_000);
        assert_eq!(
            drive(&mut exact, A, 60_000, 1_000, 199),
            (0, 0),
            "a tank on open ground was refused a cell"
        );
        assert_eq!(
            exact.unit(A).map(|unit| unit.pose.x_cm),
            Some(999 + 300_000),
            "60 s at 5,000 cm/s is 300,000 cm"
        );

        // (b) 4,999 cm/s, which rolls the carry over: one 199 ms substep is
        // 994.801 cm, so the substep covers 994 and banks 801 thousandths, and the
        // rollover lands inside the crossings the drive below makes.
        let mut probe = corridor(4_999);
        step_one(&mut probe, A, 199);
        assert_eq!(
            probe
                .unit(A)
                .map(|unit| (unit.pose.x_cm, unit.carry.east != 0)),
            Some((999 + 994, true)),
            "4,999 cm/s over 199 ms is 994 cm and a carry"
        );
        let mut rolling = corridor(4_999);
        assert_eq!(
            drive(&mut rolling, A, 60_000, 1_000, 199),
            (0, 0),
            "the rolling carry pushed the tank into a cell it was refused"
        );
        assert_eq!(
            rolling.unit(A).map(|unit| unit.pose.x_cm),
            Some(999 + 299_940),
            "60 s at 4,999 cm/s is 299,940 cm, carried and not truncated"
        );
    }

    /// **A refused substep banks no distance it never travelled** (16b cut Q.6):
    /// every stop, every wreck, every dead engine and every cold hull zeroes the
    /// carries — [`crate::arena::heat`]'s own rule for a unit that has gone cold.
    ///
    /// **16b cut E.3**: the occupancy row runs at `occupied_stop_ms = 0`, and it
    /// is honest to say why. This test's claim is about the *carries a refusal
    /// banks*, not about the wait a traffic hold puts in front of the stop; at the
    /// shipped two seconds the same fixture's tank slips diagonally past the
    /// parked one while it is being held, which is the cut working and would turn
    /// this row into a different test. What a **hold** does to the two travel
    /// carries — it zeroes them, exactly as the stop does — is pinned by
    /// `a_hold_zeroes_the_travel_carries_and_keeps_the_speed_and_its_carry`.
    #[test]
    fn a_stop_a_wreck_and_a_cold_hull_all_zero_the_carries() {
        // (a) the four refusals. Each fixture drives a tank on a heading whose
        // carries are non-zero on **both** axes — 30 degrees, where neither
        // `sin_fx` nor `cos_fx` is zero and neither divides the travel — into the
        // thing that refuses it. Due east would leave `carry.north` zero
        // throughout, and its zeroing would then be asserted against a field that
        // was never set.
        let banded = |cost: u16| -> Vec<TerrainCell> {
            let mut cells = vec![ground(); 64];
            for y in 0..8_u16 {
                if let Some(cell) = cells.get_mut(usize::from(y) * 8 + 4) {
                    cell.move_cost = cost;
                }
            }
            cells
        };
        // Impassable ground, a cell another tank holds, and the window's own edge.
        // The slope needs relief rather than a band, so it builds its own window.
        let mut cliff = vec![ground(); 64];
        for y in 0..8_u16 {
            if let Some(cell) = cliff.get_mut(usize::from(y) * 8 + 4) {
                cell.elev_cm = 900;
            }
        }
        for (cause, terrain, units, stop_ms) in [
            (
                BlockedCause::Impassable,
                ArenaTerrain::new(8, 8, 1_000, banded(0)).expect("the band window"),
                vec![tank(A, Team::Red, pose(3_800, 3_500))],
                OCCUPIED_STOP_MS,
            ),
            (
                BlockedCause::Occupied,
                ArenaTerrain::new(8, 8, 1_000, banded(100)).expect("the open window"),
                vec![
                    tank(A, Team::Red, pose(3_800, 3_500)),
                    tank(B, Team::Blue, pose(4_500, 3_500)),
                ],
                // 16b cut E.3: see this test's head.
                0,
            ),
            (
                BlockedCause::TooSteep,
                ArenaTerrain::new(8, 8, 1_000, cliff).expect("the cliff window"),
                vec![tank(A, Team::Red, pose(3_800, 3_500))],
                OCCUPIED_STOP_MS,
            ),
            (
                BlockedCause::Edge,
                ArenaTerrain::new(8, 8, 1_000, banded(100)).expect("the open window"),
                vec![tank(A, Team::Red, pose(7_800, 3_500))],
                OCCUPIED_STOP_MS,
            ),
        ] {
            let mut arena = running(units);
            std::sync::Arc::make_mut(&mut arena.definition).terrain = terrain;
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 30_000;
                unit.desired_heading_mdeg = 30_000;
                unit.speed_cms = 999;
                unit.desired_speed_cms = 999;
                unit.slope_limit_pct = Some(60);
            }
            // 999 cm/s over 250 ms on a 30 degree heading is 124.875 cm east and
            // 216.28 cm north, so the first substep stays inside its own cell and
            // banks a fraction of a centimetre on each axis. That is what makes
            // the assertion after the refusal mean something on both of them.
            let first = step_under(&mut arena, A, 250, stop_ms);
            assert_eq!(first.blocked_by, None, "{cause:?}: refused too early");
            assert!(
                arena
                    .unit(A)
                    .is_some_and(|unit| unit.carry.east != 0 && unit.carry.north != 0),
                "{cause:?}: nothing was carried into the refusal"
            );
            let outcome = step_under(&mut arena, A, 250, stop_ms);
            assert_eq!(outcome.blocked_by, Some(cause));
            assert_eq!(
                arena.unit(A).map(|unit| unit.carry),
                Some(crate::arena::state::MotionCarry::default()),
                "{cause:?}: a refused substep banked what it never travelled"
            );
        }

        // (b) a wreck, a dead engine and a cold hull, each with every carry
        // already loaded: the substep that stops the tank dead empties them.
        for limits in [
            MotionLimits {
                wreck: true,
                ..MotionLimits::default()
            },
            MotionLimits {
                motion_disabled: true,
                ..MotionLimits::default()
            },
            MotionLimits::default(),
        ] {
            let cold = limits == MotionLimits::default();
            let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
            if let Some(unit) = arena.unit_mut(A) {
                unit.carry = crate::arena::state::MotionCarry {
                    turn: 700,
                    speed: 800,
                    east: 900,
                    north: 1_000,
                    // 16b cut M.4 (`T-16`): the two sign bytes go with them.
                    turn_side: 2,
                    speed_rate: 1,
                };
                unit.speed_cms = 500;
                unit.desired_speed_cms = 500;
                unit.desired_heading_mdeg = 90_000;
                unit.limits = limits;
                if cold {
                    unit.power = Power::Off;
                }
            }
            step_one(&mut arena, A, 250);
            assert_eq!(
                arena.unit(A).map(|unit| unit.carry),
                Some(crate::arena::state::MotionCarry::default()),
                "a stopped unit kept a carry (cold {cold})"
            );
            assert_eq!(arena.unit(A).map(|unit| unit.speed_cms), Some(0));
        }

        // (c) and the two carries that belong to a finished order: an arc that
        // closes and a ramp that reaches its target both end at zero, so the next
        // order starts from a known place rather than from whatever the last one
        // left over.
        let mut arena = running(vec![tank(A, Team::Red, pose(3_500, 3_500))]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.mobility.turn_rate_mdeg_s = 33_333;
            unit.desired_heading_mdeg = 30_000;
            unit.desired_speed_cms = 333;
            unit.mobility.accel_cms2 = 333;
        }
        // 33,333 mdeg/s and 333 cm/s² both leave a quarter of a unit over on a
        // 250 ms substep, so both carries are alive before either order finishes.
        step_one(&mut arena, A, 250);
        assert_eq!(
            arena
                .unit(A)
                .map(|unit| (unit.carry.turn, unit.carry.speed)),
            Some((250, 250)),
            "the first substep closed neither"
        );
        run(&mut arena, A, 3, 250);
        assert_eq!(
            arena
                .unit(A)
                .map(|unit| (unit.pose.heading_mdeg, unit.speed_cms)),
            Some((30_000, 333)),
            "the arc and the ramp both finished"
        );
        assert_eq!(
            arena
                .unit(A)
                .map(|unit| (unit.carry.turn, unit.carry.speed)),
            Some((0, 0)),
            "a finished order left a carry behind"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut E.3: the traffic hold (16c `B-46`, §5's Q10)
    // -----------------------------------------------------------------------

    /// **A follower behind a leader that moves off never stops at all** (16b cut
    /// E.3), which is the whole reason the hold exists.
    ///
    /// `A` sorts below `B`, so it integrates first and reaches the cell `B` has
    /// not vacated yet — the column at the ford, in two tanks. Before this cut it
    /// was stopped dead and needed eleven seconds at 1 m/s² to rebuild way; now it
    /// is held, keeps its fifteen metres a second, and drives into the cell on the
    /// substep after the leader leaves it.
    #[test]
    fn a_follower_is_held_behind_its_leader_and_never_loses_its_way() {
        let mut arena = nose_to_tail();
        let mut outcomes: Vec<MotionOutcome> = Vec::new();
        let mut speeds: Vec<i32> = Vec::new();
        let mut cells: Vec<Option<(u16, u16)>> = Vec::new();
        for _ in 0..3 {
            outcomes.push(step_one(&mut arena, A, 250));
            speeds.push(arena.unit(A).map_or(0, |unit| unit.speed_cms));
            cells.push(
                arena
                    .unit(A)
                    .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose)),
            );
            step_one(&mut arena, B, 250);
        }
        assert!(
            outcomes.iter().all(|out| out.blocked_by.is_none()),
            "the follower was stopped: {outcomes:?}"
        );
        assert!(
            outcomes.iter().all(|out| out.blocked_at.is_none()),
            "the follower journalled a stop: {outcomes:?}"
        );
        assert_eq!(
            speeds,
            vec![1_000, 1_000, 1_000],
            "the follower's speed fell while it was being held"
        );
        assert_eq!(
            cells,
            vec![Some((3, 3)), Some((3, 3)), Some((4, 3))],
            "the follower did not enter the cell on the substep after the leader left it"
        );
        assert_eq!(
            arena.unit(A).and_then(|unit| unit.occupied_hold),
            None,
            "an accepted crossing clears the hold"
        );
        assert_eq!(arena.unit(A).and_then(|unit| unit.held), None);
    }

    /// **A hold against a parked unit matures into doc 16 §3.4 step 4's stop
    /// after exactly `occupied_stop_ms` of arena time** (16b cut E.3), with the
    /// event, the cause and the zeroed speed the stop always had.
    #[test]
    fn a_hold_against_a_parked_unit_matures_into_the_stop() {
        let mut arena = nose_to_tail();
        // The leader is never stepped, so it is a parked obstacle.
        let mut refusals: Vec<(i64, MotionOutcome)> = Vec::new();
        for _ in 0..12 {
            let outcome = step_one(&mut arena, A, 250);
            refusals.push((arena.now_ms, outcome));
            if outcome.blocked_by.is_some() {
                break;
            }
        }
        let (stopped_at, stop) = *refusals.last().expect("twelve substeps");
        let holds = &refusals[..refusals.len() - 1];
        assert!(
            holds
                .iter()
                .all(|(_, out)| !out.moved && out.blocked_by.is_none() && out.blocked_at.is_none()),
            "a hold moved or reported something: {holds:?}"
        );
        assert_eq!(
            stopped_at, 2_250,
            "the hold opened at the end of the substep that met the obstacle (250 ms) and the \
             stop is the first substep at or past 250 + 2,000 ms"
        );
        assert_eq!(stop.blocked_by, Some(BlockedCause::Occupied));
        assert_eq!(stop.blocked_at, Some((4, 3)));
        assert_eq!(arena.unit(A).map(|unit| unit.speed_cms), Some(0));
        assert_eq!(arena.unit(A).map(|unit| unit.pose.x_cm), Some(3_999));
        assert_eq!(arena.unit(A).map(|unit| unit.blocked), Some(true));
        assert_eq!(
            arena.unit(A).and_then(|unit| unit.held),
            Some(Held {
                x: 4,
                y: 3,
                cause: BlockedCause::Occupied
            })
        );
        assert_eq!(
            arena.unit(A).and_then(|unit| unit.occupied_hold),
            None,
            "the stop consumed the hold"
        );

        // And a tank with a standing speed order cycles: it re-accelerates, opens
        // a second hold and matures it two seconds later, so the ledger takes one
        // `blocked` line per cycle and not one per substep.
        let mut cycles = 0_usize;
        let from = arena.now_ms;
        while arena.now_ms < from + 5_000 {
            if step_one(&mut arena, A, 250).blocked_by.is_some() {
                cycles += 1;
            }
        }
        assert_eq!(
            cycles, 2,
            "a standing order against a parked obstacle is one stop per {OCCUPIED_STOP_MS} ms \
             and not one per substep"
        );
    }

    /// **A hold zeroes the two travel carries and keeps the speed and its own
    /// carry** (16b cut E.3) — the half of 16b cut Q.6's rule that a hold shares
    /// with a stop, and the half it does not.
    #[test]
    fn a_hold_zeroes_the_travel_carries_and_keeps_the_speed_and_its_carry() {
        let mut arena = running(vec![
            tank(A, Team::Red, pose(3_800, 3_500)),
            tank(B, Team::Blue, pose(4_500, 3_500)),
        ]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
            // Mid-ramp, so `carry.speed` is alive and has to survive the hold.
            unit.speed_cms = 111;
            unit.desired_speed_cms = 999;
            unit.mobility.accel_cms2 = 333;
        }
        // Due east at 111 cm/s: 27.75 cm a substep, so it banks a carry and
        // creeps at the boundary. Push until the refusal.
        let mut outcome = step_one(&mut arena, A, 250);
        let mut substeps = 1;
        while arena
            .definition
            .terrain
            .cell_of(&arena.unit(A).map_or(pose(0, 0), |unit| unit.pose.clone()))
            == Some((3, 3))
            && arena.unit(A).and_then(|unit| unit.occupied_hold).is_none()
        {
            outcome = step_one(&mut arena, A, 250);
            substeps += 1;
            assert!(substeps < 40, "it never reached the parked tank");
        }
        assert_eq!(outcome, MotionOutcome::default(), "a hold reports nothing");
        let held = arena.unit(A).expect("the follower");
        assert_eq!(
            (held.carry.east, held.carry.north),
            (0, 0),
            "a hold banked a fraction of a centimetre it was clamped back out of"
        );
        assert!(
            held.speed_cms > 111,
            "the hold shed the way the substep built: {} cm/s",
            held.speed_cms
        );
        assert_ne!(held.carry.speed, 0, "the ramp's own carry was thrown away");
    }

    /// **The five things that clear a hold** (16b cut E.3), one fixture each.
    #[test]
    fn a_hold_is_cleared_by_a_turn_by_an_order_of_zero_and_by_a_cold_hull() {
        // (a) turning away inside the cell and moving.
        let mut arena = nose_to_tail();
        step_one(&mut arena, A, 250);
        assert!(arena.unit(A).is_some_and(|u| u.occupied_hold.is_some()));
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 270_000;
            unit.desired_heading_mdeg = 270_000;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(
            arena.unit(A).and_then(|u| u.occupied_hold),
            None,
            "a substep that moved and was not refused leaves nothing to hold against"
        );

        // (b) the speed order coming off.
        let mut arena = nose_to_tail();
        step_one(&mut arena, A, 250);
        if let Some(unit) = arena.unit_mut(A) {
            unit.desired_speed_cms = 0;
        }
        step_one(&mut arena, A, 250);
        assert_eq!(arena.unit(A).and_then(|u| u.occupied_hold), None);

        // (c) the wreck, the dead engine and the cold hull.
        for limits in [
            Some(MotionLimits {
                wreck: true,
                ..MotionLimits::default()
            }),
            Some(MotionLimits {
                motion_disabled: true,
                ..MotionLimits::default()
            }),
            None,
        ] {
            let mut arena = nose_to_tail();
            step_one(&mut arena, A, 250);
            assert!(arena.unit(A).is_some_and(|u| u.occupied_hold.is_some()));
            if let Some(unit) = arena.unit_mut(A) {
                match limits {
                    Some(limits) => unit.limits = limits,
                    None => unit.power = Power::Off,
                }
            }
            step_one(&mut arena, A, 250);
            assert_eq!(
                arena.unit(A).and_then(|u| u.occupied_hold),
                None,
                "a tank stopped dead is holding against nobody: {limits:?}"
            );
        }
    }

    /// **The fifth clear: a refusal of a *different* cell is a different hold, and
    /// the clock starts again** (16b cut E.3) — the one the other fixture does not
    /// cover, and the one with a consequence worth writing down.
    ///
    /// A unit wedged into the corner between two parked hulls, its heading put
    /// east and north turn about, is refused `(4, 3)`, then `(3, 4)`, then
    /// `(4, 3)`: each refusal names a cell the open hold does not, so each opens a
    /// new hold stamped at this substep's end and `end_ms − since_ms` never
    /// reaches `occupied_stop_ms`. **It therefore holds for as long as its heading
    /// keeps changing**: no `stop_at_boundary`, so no `blocked` line, no
    /// `Held: something is in the way.` and no lesson for its machine. That is the
    /// decided rule (each cell is judged on its own two seconds, so a tank edged
    /// round one obstacle into another is not stopped on the first one's clock),
    /// and the reason it is bounded in the shipped lane is that nothing changes a
    /// heading every substep: `Arena::follow` orders one at its decide tick, a
    /// pilot orders one when they type, and [`turn`] sweeps between them on the
    /// shortest arc — monotonically, so each cell along the sweep is refused once
    /// and the last one's clock runs out. The second half of this test is that
    /// last part: hold the heading still and the stop arrives on schedule.
    #[test]
    fn a_refusal_of_a_different_cell_restarts_the_clock() {
        let mut arena = running(vec![
            tank(A, Team::Red, pose(3_900, 3_900)),
            tank(B, Team::Blue, pose(4_500, 3_500)),
            tank(C, Team::Blue, pose(3_500, 4_500)),
        ]);
        if let Some(unit) = arena.unit_mut(A) {
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
        // Ten seconds of arena time — five times the horizon — with the heading
        // put east and north turn about.
        let mut cells: Vec<(u16, u16)> = Vec::new();
        for substep in 0..40_u32 {
            let heading = if substep % 2 == 0 { 90_000 } else { 0 };
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = heading;
                unit.desired_heading_mdeg = heading;
            }
            let outcome = step_one(&mut arena, A, 250);
            assert_eq!(
                outcome,
                MotionOutcome::default(),
                "a stop matured at substep {substep} although the refused cell kept changing"
            );
            let hold = arena
                .unit(A)
                .and_then(|unit| unit.occupied_hold)
                .expect("a hold against the hull it was refused");
            assert_eq!(
                hold.since_ms, arena.now_ms,
                "the clock did not start again at substep {substep}, so a hold on one cell was \
                 being aged on another cell's stamp"
            );
            cells.push(hold.cell);
        }
        assert!(
            cells.contains(&(4, 3)) && cells.contains(&(3, 4)),
            "the fixture never alternated, so nothing was measured: {cells:?}"
        );
        assert_eq!(
            arena.unit(A).map(|unit| unit.speed_cms),
            Some(1_000),
            "and it kept its way for the whole of it"
        );

        // Now hold the heading still. The next refusal of `(4, 3)` opens the last
        // hold, and two seconds of arena time after that the stop arrives.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 90_000;
            unit.desired_heading_mdeg = 90_000;
        }
        let mut stopped_at: Option<i64> = None;
        let mut opened_at: Option<i64> = None;
        for _ in 0..12_u32 {
            let outcome = step_one(&mut arena, A, 250);
            if opened_at.is_none() {
                opened_at = arena
                    .unit(A)
                    .and_then(|unit| unit.occupied_hold)
                    .map(|hold| hold.since_ms);
            }
            if outcome.blocked_by.is_some() {
                stopped_at = Some(arena.now_ms);
                assert_eq!(outcome.blocked_at, Some((4, 3)));
                break;
            }
        }
        let (opened_at, stopped_at) = (
            opened_at.expect("a last hold"),
            stopped_at.expect("the stop never matured with the heading held still"),
        );
        assert_eq!(
            stopped_at - opened_at,
            i64::from(OCCUPIED_STOP_MS),
            "the stop arrived {} ms after the hold opened",
            stopped_at - opened_at
        );
    }

    /// **Ground, the window's edge and a slope still stop a unit dead** (16b cut
    /// E.3): no hold is opened for any of the three, at the shipped horizon.
    #[test]
    fn impassable_ground_an_edge_and_a_slope_stop_at_once_and_open_no_hold() {
        let mut band = vec![ground(); 64];
        for y in 0..8_u16 {
            if let Some(cell) = band.get_mut(usize::from(y) * 8 + 4) {
                cell.move_cost = 0;
            }
        }
        let mut cliff = vec![ground(); 64];
        for y in 0..8_u16 {
            if let Some(cell) = cliff.get_mut(usize::from(y) * 8 + 4) {
                cell.elev_cm = 900;
            }
        }
        for (cause, terrain, at, refused) in [
            (
                BlockedCause::Impassable,
                ArenaTerrain::new(8, 8, 1_000, band).expect("the band window"),
                pose(3_900, 3_500),
                (4_u16, 3_u16),
            ),
            (
                BlockedCause::TooSteep,
                ArenaTerrain::new(8, 8, 1_000, cliff).expect("the cliff window"),
                pose(3_900, 3_500),
                (4, 3),
            ),
            (
                BlockedCause::Edge,
                ArenaTerrain::new(8, 8, 1_000, vec![ground(); 64]).expect("the open window"),
                pose(7_900, 3_500),
                (7, 3),
            ),
        ] {
            let mut arena = running(vec![tank(A, Team::Red, at)]);
            std::sync::Arc::make_mut(&mut arena.definition).terrain = terrain;
            if let Some(unit) = arena.unit_mut(A) {
                unit.pose.heading_mdeg = 90_000;
                unit.desired_heading_mdeg = 90_000;
                unit.speed_cms = 1_000;
                unit.desired_speed_cms = 1_000;
                unit.slope_limit_pct = Some(60);
            }
            let outcome = step_one(&mut arena, A, 250);
            assert_eq!(outcome.blocked_by, Some(cause));
            assert_eq!(outcome.blocked_at, Some(refused));
            assert_eq!(arena.unit(A).map(|unit| unit.speed_cms), Some(0));
            assert_eq!(
                arena.unit(A).and_then(|unit| unit.occupied_hold),
                None,
                "{cause:?} opened a traffic hold"
            );
        }
    }
}
