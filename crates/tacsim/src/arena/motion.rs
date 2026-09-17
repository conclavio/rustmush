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
//! 4. a crossing tests the cell entered — impassable, occupied or off the window
//!    — and a refusal stops the unit at the boundary with `speed = 0` and one
//!    `blocked` notice.
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
//! The one rounding that is visible: the distance along each axis is rounded to
//! the nearest whole centimetre **per substep**, so a unit on a diagonal
//! accumulates up to half a centimetre of rounding per substep rather than
//! carrying a remainder (a remainder would be state, and
//! [`crate::arena::state::UnitState`] deliberately holds none). At 250 ms
//! substeps that is under 2 cm a second against cells 10 m wide.

use mush_sim::data::Pose;

use crate::arena::index::SpatialIndex;
use crate::arena::state::{ArenaTerrain, Power, Side, UnitState};

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

/// `value × fixed / `[`SIN_ONE`], rounded to the nearest whole unit with halves
/// away from zero. The product is taken in `i128`: a distance in centimetres
/// times a fixed-point unit vector is comfortably inside `i64`, but only once the
/// division has happened.
fn scale_round(value: i64, fixed: i64) -> i64 {
    let product = i128::from(value).saturating_mul(i128::from(fixed));
    let half = i128::from(SIN_ONE) / 2;
    let rounded = if product >= 0 {
        product.saturating_add(half) / i128::from(SIN_ONE)
    } else {
        product.saturating_sub(half) / i128::from(SIN_ONE)
    };
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
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
/// `unit.blocked` is **set here and cleared here**: it is the condition "stopped
/// against something", not an event, and the caller turns its rising edge into the
/// one ledger notice per stop.
pub fn integrate(
    unit: &mut UnitState,
    dt_ms: u32,
    terrain: &ArenaTerrain,
    index: &SpatialIndex,
) -> MotionOutcome {
    unit.blocked = false;
    if dt_ms == 0 {
        return MotionOutcome::default();
    }
    let dt = i64::from(dt_ms);

    // ---- 1. the heading turns toward the ordered one, shortest arc ----
    turn(unit, dt);

    // ---- 2. the speed moves toward the ordered one ----
    let cell = terrain.cell_of(&unit.pose);
    let move_cost = cell
        .and_then(|(x, y)| terrain.get(x, y))
        .map_or(0, |cell| cell.move_cost);
    accelerate(unit, dt, move_cost);

    // ---- 3. the position advances along the heading ----
    let travel = i64::from(unit.speed_cms).saturating_mul(dt) / 1_000;
    if travel == 0 {
        return MotionOutcome::default();
    }
    let east = scale_round(travel, sin_fx(unit.pose.heading_mdeg));
    let north = scale_round(travel, cos_fx(unit.pose.heading_mdeg));
    if east == 0 && north == 0 {
        return MotionOutcome::default();
    }
    let Some(from) = cell else {
        // A unit that is not on the window cannot be integrated onto it: it is
        // stopped where it stands rather than teleported home. `Arena::new`
        // refuses a pose off the window, so this is unreachable through the
        // shipped lane and is written as a refusal rather than an assumption.
        unit.speed_cms = 0;
        unit.blocked = true;
        return MotionOutcome {
            moved: false,
            blocked_at: Some((0, 0)),
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
        Some(to) if to == from => MotionOutcome {
            moved: true,
            blocked_at: None,
        },
        Some(to) if enterable(unit, terrain, index, from, to) => {
            // Doc 16 §3.2: `z_cm` is the cell's `elev_cm`, carried on entry.
            if let Some(entered) = terrain.get(to.0, to.1) {
                unit.pose.z_cm = i32::from(entered.elev_cm);
            }
            MotionOutcome {
                moved: true,
                blocked_at: None,
            }
        }
        // Impassable, occupied or more than one cell away: stop at the boundary.
        Some(to) => stop_at_boundary(unit, terrain, from, to),
        // Off the window (doc 16 §3.2 "Edges").
        None => {
            let nearest = nearest_on_window(terrain, &unit.pose);
            stop_at_boundary(unit, terrain, from, nearest)
        }
    }
}

/// Step 1: turn toward `desired_heading_mdeg` at `turn_rate_mdeg_s × dt / 1000`
/// on the shortest arc, **blocked toward a `no_turn_toward` side**.
///
/// Clockwise (millidegrees increasing) is toward the unit's **right**;
/// anticlockwise is toward its **left**. A destroyed left track is
/// `no_turn_toward(Left)` and forbids the anticlockwise direction outright, so a
/// turn whose short arc goes that way **takes the long way round** — which is
/// exactly what a tank on one track does. `no_turn_toward` names at most one
/// side, so the long way is always open; the "both ways are blocked, so do not
/// turn" branch is written anyway, because a rule with an unhandled case is not a
/// rule.
///
/// A wreck and a unit whose motion is disabled do not turn at all: doc 16 §3.3
/// revokes `tactics.mobility` with the wreck, and a destroyed engine is
/// `disable_motion`.
fn turn(unit: &mut UnitState, dt: i64) {
    if unit.limits.wreck || unit.limits.motion_disabled || unit.power != Power::Online {
        return;
    }
    let current = unit.pose.heading_mdeg % FULL_TURN_MDEG;
    let desired = unit.desired_heading_mdeg % FULL_TURN_MDEG;
    let clockwise_arc = (desired + FULL_TURN_MDEG - current) % FULL_TURN_MDEG;
    if clockwise_arc == 0 {
        return;
    }
    let short_is_clockwise = clockwise_arc <= HALF_TURN_MDEG;
    let mut toward = if short_is_clockwise {
        Side::Right
    } else {
        Side::Left
    };
    if unit.limits.no_turn_toward == Some(toward) {
        toward = match toward {
            Side::Right => Side::Left,
            Side::Left => Side::Right,
        };
    }
    if unit.limits.no_turn_toward == Some(toward) {
        return;
    }
    let arc = match toward {
        Side::Right => clockwise_arc,
        Side::Left => FULL_TURN_MDEG - clockwise_arc,
    };
    let rate = u64::from(unit.mobility.turn_rate_mdeg_s)
        .saturating_mul(u64::try_from(dt).unwrap_or(0))
        / 1_000;
    let step = u32::try_from(rate).unwrap_or(u32::MAX).min(arc);
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
/// An impassable `move_cost` of zero caps the speed at zero: a unit standing on
/// a cell nothing may enter cannot drive out of it. Motion never puts one there
/// (step 4), and a check-out that does is held rather than allowed to drive.
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
fn accelerate(unit: &mut UnitState, dt: i64, move_cost: u16) {
    if unit.limits.wreck || unit.limits.motion_disabled || unit.power != Power::Online {
        unit.speed_cms = 0;
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
        return;
    }
    let rate = if target.abs() < current.abs() {
        unit.mobility.brake_cms2
    } else {
        unit.mobility.accel_cms2
    };
    let delta = i64::from(rate).saturating_mul(dt) / 1_000;
    let delta = i32::try_from(delta).unwrap_or(i32::MAX);
    unit.speed_cms = if target > current {
        current.saturating_add(delta).min(target)
    } else {
        current.saturating_sub(delta).max(target)
    };
}

/// Whether `to` may be entered from `from`: it is at most one cell away on each
/// axis, it is passable, and it is either empty or already held by this unit.
///
/// **The one-cell guard.** The substep length is `min(substep_max_ms, cell_cm ×
/// 1000 / max_speed_cms)` (doc 16 §3.1), so a unit covers at most one cell in a
/// substep and step 4 has at most one boundary to test. A configuration whose
/// floor beat that bound could offer a longer jump, and the cells in between
/// would never be tested — so a jump of more than one cell is **refused** rather
/// than trusted, which keeps the invariant true of the code and not merely of the
/// shipped constants.
///
/// A **diagonal** crossing tests the diagonal neighbour and nothing else: the
/// unit cuts the corner and never occupies either edge-adjacent cell, so neither
/// is entered and neither needs testing.
fn enterable(
    unit: &UnitState,
    terrain: &ArenaTerrain,
    index: &SpatialIndex,
    from: (u16, u16),
    to: (u16, u16),
) -> bool {
    let one_step = from.0.abs_diff(to.0) <= 1 && from.1.abs_diff(to.1) <= 1;
    let passable = terrain
        .get(to.0, to.1)
        .is_some_and(crate::arena::state::TerrainCell::passable);
    let free = match index.occupant(to.0, to.1) {
        None => true,
        Some(holder) => holder == unit.id,
    };
    one_step && passable && free
}

/// Stop the unit at the boundary it could not cross: the pose is clamped into the
/// cell it already occupies — **both** axes, so a refused diagonal cannot deposit
/// it in an untested edge-adjacent cell — the speed goes to zero, and the stop is
/// reported against `refused`.
///
/// "At the boundary" is the last whole centimetre of its own cell, because
/// [`ArenaTerrain::cell_of`] gives a boundary to the cell it opens: at 1,000 cm
/// cells, column 3 is `3_000..=3_999`.
fn stop_at_boundary(
    unit: &mut UnitState,
    terrain: &ArenaTerrain,
    from: (u16, u16),
    refused: (u16, u16),
) -> MotionOutcome {
    let side = i64::from(terrain.cell_cm());
    let low_x = i64::from(from.0).saturating_mul(side);
    let low_y = i64::from(from.1).saturating_mul(side);
    let moved_x = unit.pose.x_cm;
    let moved_y = unit.pose.y_cm;
    unit.pose.x_cm = moved_x.clamp(low_x, low_x.saturating_add(side).saturating_sub(1));
    unit.pose.y_cm = moved_y.clamp(low_y, low_y.saturating_add(side).saturating_sub(1));
    unit.speed_cms = 0;
    unit.blocked = true;
    MotionOutcome {
        moved: false,
        blocked_at: Some(refused),
    }
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

    /// One substep for one unit, with the index rebuilt from the current poses.
    fn step_one(arena: &mut Arena, id: ObjId, dt_ms: u32) -> MotionOutcome {
        let index = index_of(arena);
        let terrain = arena.definition.terrain.clone();
        let Some(unit) = arena.unit_mut(id) else {
            return MotionOutcome::default();
        };
        integrate(unit, dt_ms, &terrain, &index)
    }

    /// `count` substeps of `dt_ms` each — the shape a real pulse has, and the
    /// only shape in which a unit may cross a cell (a single long substep would
    /// jump several and be refused by design).
    fn run(arena: &mut Arena, id: ObjId, count: u32, dt_ms: u32) {
        for _ in 0..count {
            step_one(arena, id, dt_ms);
        }
    }

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
            unit.limits.no_turn_toward = Some(Side::Left);
            // The short arc to 350 degrees is anticlockwise — to the left, which
            // the destroyed track forbids — so it goes the long way, clockwise.
            unit.desired_heading_mdeg = 350_000;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(30_000));
        // The mirror: a destroyed right track sends a starboard order to port. The
        // long way round is **always** open, because `no_turn_toward` names one
        // side, which is why `turn`'s "neither way" guard is unreachable through
        // this surface and no test claims to reach it.
        if let Some(unit) = arena.unit_mut(A) {
            unit.pose.heading_mdeg = 0;
            unit.desired_heading_mdeg = 90_000;
            unit.limits.no_turn_toward = Some(Side::Right);
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
            unit.limits.no_turn_toward = None;
            unit.mobility.turn_rate_mdeg_s = 0;
        }
        step_one(&mut arena, A, 1_000);
        assert_eq!(arena.unit(A).map(|u| u.pose.heading_mdeg), Some(0));
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
        arena.definition.terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the woods window");
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
        arena.definition.terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the woods window");
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
        arena.definition.terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the band window");
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

    #[test]
    fn an_occupied_cell_stops_the_unit_and_the_lower_id_wins() {
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
        // A is in cell 3 and B holds cell 4: A is refused, B moves on.
        let outcome = step_one(&mut arena, A, 250);
        assert_eq!(outcome.blocked_at, Some((4, 3)));
        assert_eq!(arena.unit(A).map(|u| u.pose.x_cm), Some(3_999));
        let onward = step_one(&mut arena, B, 250);
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
        arena.definition.terrain = ArenaTerrain::new(8, 8, 1_000, cells).expect("the hill window");
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

    #[test]
    fn no_unit_ever_occupies_a_cell_that_was_not_tested_on_entry() {
        // A random walk over a window with an impassable band and two units,
        // asserting the invariant of doc 16 §3.4 step 4 after every substep.
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
        ]);
        arena.definition.terrain = terrain.clone();
        for unit in &mut arena.units {
            unit.desired_speed_cms = 1_000;
            unit.speed_cms = 1_000;
        }
        // A deterministic pseudo-random heading per unit per substep, from the
        // arena's own draw primitive rather than a generator with state.
        let mut seed = 2_350_u64;
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
    }
}
