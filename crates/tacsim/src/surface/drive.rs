//! **The drive planner** (doc 20 cut SM.4b; §4.1 "the TV's speed and charge", §8 "A* once per
//! command" and "the frozen leg", §11's TV speed and Charge rows; D-20-6, D-20-11, D-20-13; the
//! SM.4b brief's lead rulings 2–4 and its reviewer resolutions R1–R3).
//!
//! Pure integer arithmetic over a site's terrain composite, read by `+tv/drive`, the start's
//! offer, the recover and the map, so they cannot drift:
//!
//! - **the rates** ([`DriveRates`]): the rig unit's tracked ceiling (`UnitSpec.mobility.tracked
//!   .max_speed_cms`, 300 on the TV) and the battery row's `drive_cu_per_nominal_cell` (15 on the
//!   Mk1). With the atlas locomotion table's tracked row (regolith 150, rock 300, ice 200 — a
//!   cost in hundredths of a nominal cell) they give **a regolith cell 5 IC s and 22.5 cu, a
//!   kilometre of regolith exactly 500 IC s and 2,250 cu** (§13's acceptance; lead ruling 3: no
//!   new constant here — [`secs_of_cost`] and [`cu_of_cost`] are the only conversions);
//! - **the targets** ([`Target`], [`parse_target`], [`target_cell`]): a pocket address, a field,
//!   a block letter, a site cell or a compass bearing with metres (§10's `/drive` grammar);
//! - **the leg** ([`drive_leg`]): ONE atlas [`path`] (8-connected, tracked, no tags) over the
//!   composite at [`DRIVE_PATH_BUDGET_CELLS`] — a whole site — then **the freeze** ([`freeze`]):
//!   the exact path simplified by an integer Douglas–Peucker at a display tolerance of 1, 2, 4
//!   then 8 cells ([`LEG_TOLERANCE_CELLS`]; R1), the first that keeps at most
//!   [`MAX_LEG_WAYPOINTS`] points winning. The lane's per-cell terrain makes every least path
//!   weave, so "corner points" would refuse nearly every drive; the freeze keeps the path's
//!   shape within the tolerance, and **time and charge stay the exact path's** — the waypoints
//!   are display only (§8: "positions between are pure interpolation"; the placement happens
//!   once, at the end). A path that needs more than 32 points even at 8 cells (80 m, one
//!   site-view cell) is refused [`LegRefused::Winding`], never cut;
//! - **the way home** ([`HomeField`]): every cell's exact least cost to beside the ramp, built
//!   once per (site terrain, ramp cell) by a multi-source reverse Dijkstra over the same
//!   `move_cost` (the brief's decision P1: the start's offer re-plans every rig's acts on every
//!   projection, so a per-command search home would become a per-projection one). Each cell also
//!   keeps its first step home, so a descent needs no composite (R3: the way home never depends
//!   on the geology key). [`home_cu_to`] reads the charge home from a stand an act would move the
//!   rig to — `None` where there is no way home at all, which a drive refuses (R2) — and
//!   [`home_cu_here`] from the stand a rig already occupies, the conservative bound standing in
//!   for an unreachable one (lead ruling 4: never zero, never optimistic).
//!
//! **Features are not obstacles** (the atlas's own rule, `mush_atlas::physics::path`: "a path is
//! geometry"): the lane's craters author `blocks.movement: false`; slope past the tracked 45 %
//! limit and impassable classes are what a search refuses. `tests/driving.rs` fails the day a
//! lane site authors a movement-blocking feature.
//!
//! Integers only (no `f64`), no RNG, nothing that can panic.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use mush_atlas::physics::{
    DIAGONAL_PCT, LocomotionTable, NOMINAL_COST, PathOutcome, StepKind, move_cost, path,
};
use mush_atlas::window::SurfaceComposite;

use crate::surface::address::{
    self, CELLS_PER_BLOCK, CELLS_PER_FIELD, CELLS_PER_POCKET, Octant, cell_on_site,
};
use crate::surface::excursion::{LegWaypoint, MAX_LEG_WAYPOINTS, cells_apart, site_cell};
use crate::surface::ground::{SITE_CELL_CM, SITE_CELLS};

/// **The A* budget** (doc 20 §8: "a budget of 65,536 cells, a whole site"): `SITE_CELLS²`.
pub const DRIVE_PATH_BUDGET_CELLS: u32 = (SITE_CELLS as u32) * (SITE_CELLS as u32);
/// The TV's locomotion row in the atlas table (its unit's tracked mode).
pub const DRIVE_LOCOMOTION: &str = "tracked";
/// The farthest a bearing target may name, metres (the site's diagonal, 2,560 m × √2).
pub const MAX_BEARING_METRES: u32 = 3_620;
/// **The display tolerances a leg is frozen at**, in 10 m cells, tried in order (R1): the first
/// whose kept points number at most [`MAX_LEG_WAYPOINTS`] wins. 8 cells is one site-view cell
/// (80 m), so the site view never shows the difference.
pub const LEG_TOLERANCE_CELLS: [u32; 4] = [1, 2, 4, 8];
/// The conservative bound's safety factor (lead ruling 4; [`home_cu_here`]).
pub const HOME_BOUND_SAFETY: u64 = 2;

/// The atlas's eight neighbours, in its own order (`mush_atlas::physics::path`'s private table:
/// east, west, north, south, then the diagonals), so the home field relaxes in the order A*
/// expands.
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
/// [`HomeField::next`] at a source cell or an unreachable one.
const NO_STEP: u8 = u8::MAX;

/// **A rig's driving figures** (lead ruling 3): the unit's tracked ceiling and the battery's
/// per-nominal-cell draw — both read from content, never a constant here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriveRates {
    /// The unit's tracked `max_speed_cms` (300 on the TV): one nominal cell (cost 100) takes
    /// `SITE_CELL_CM / max_speed_cms` seconds.
    pub max_speed_cms: u32,
    /// The battery's `drive_cu_per_nominal_cell` (15 on the Mk1).
    pub cu_per_nominal_cell: u32,
}

/// **IC seconds for an atlas path cost** (hundredths of a nominal cell):
/// `⌈cost × SITE_CELL_CM / (NOMINAL_COST × max_speed_cms)⌉`. `None` on a zero speed or an
/// overflow. A regolith cell (150) is 5 s; a kilometre of regolith (15,000) 500 s.
#[must_use]
pub fn secs_of_cost(cost: u64, rates: &DriveRates) -> Option<u32> {
    let per = u64::from(NOMINAL_COST).checked_mul(u64::from(rates.max_speed_cms))?;
    if per == 0 {
        return None;
    }
    let scaled = cost.checked_mul(u64::from(SITE_CELL_CM))?;
    u32::try_from(scaled.div_ceil(per)).ok()
}

/// **Charge for an atlas path cost**: `⌈cost × cu_per_nominal_cell / NOMINAL_COST⌉`. `None` on an
/// overflow. A regolith cell is 22.5 cu (charged with the rest of its leg); a kilometre of
/// regolith 2,250 cu.
#[must_use]
pub fn cu_of_cost(cost: u64, rates: &DriveRates) -> Option<u32> {
    let scaled = cost.checked_mul(u64::from(rates.cu_per_nominal_cell))?;
    u32::try_from(scaled.div_ceil(u64::from(NOMINAL_COST))).ok()
}

// ---- the targets ------------------------------------------------------------------------------

/// **Where a drive goes** (doc 20 §10: `/drive <addr>|<x,y>|<block>|<bearing> <m>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A pocket address (`C117`): the drive ends on the pocket's address cell `(4px + 2, 4py + 2)`.
    Pocket {
        /// The site pocket, east.
        px: i64,
        /// The site pocket, north.
        py: i64,
    },
    /// A field (`C-NW`): its middle `(32fx + 16, 32fy + 16)`.
    Field {
        /// The site field, east.
        fx: i64,
        /// The site field, north.
        fy: i64,
    },
    /// A block letter (`C`): its middle `(64bx + 32, 64by + 32)`.
    Block {
        /// The block, east (`0..4`).
        bx: i64,
        /// The block, north (`0..4`).
        by: i64,
    },
    /// A site cell (`140,96`), `0..=255` each.
    Cell {
        /// East.
        x: i64,
        /// North.
        y: i64,
    },
    /// A compass bearing and a distance from where the rig stands (`ne 300`).
    Bearing {
        /// The octant.
        octant: Octant,
        /// Metres, `1..=`[`MAX_BEARING_METRES`].
        metres: u32,
    },
}

/// An octant by its abbreviation or its word, case-insensitive.
fn octant_of_word(word: &str) -> Option<Octant> {
    let lower = word.to_ascii_lowercase();
    [
        Octant::N,
        Octant::NE,
        Octant::E,
        Octant::SE,
        Octant::S,
        Octant::SW,
        Octant::W,
        Octant::NW,
    ]
    .into_iter()
    .find(|octant| octant.abbrev().eq_ignore_ascii_case(&lower) || octant.word() == lower)
}

/// A strict decimal: ASCII digits, no sign, no leading zero but `0`.
fn strict_decimal(text: &str) -> Option<i64> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

/// **Parse a drive's target** (pure; the words already split, `go` removed): `C117` (a pocket
/// address), `C-NW` (a field), `C` (a block letter), `140,96` (a site cell, also `140, 96`), or
/// a bearing — `ne 300`, `northeast 300`, `ne 300m`, `ne 300 m`. Case-insensitive; `None` for
/// anything else, an address off the grammar, or a bearing outside `1..=`[`MAX_BEARING_METRES`].
#[must_use]
pub fn parse_target(words: &[&str]) -> Option<Target> {
    match words {
        [one] => {
            if let Some((x, y)) = one.split_once(',') {
                return Some(Target::Cell {
                    x: strict_decimal(x.trim())?,
                    y: strict_decimal(y.trim())?,
                });
            }
            if let Some((px, py)) = address::parse_pocket_address(one) {
                return Some(Target::Pocket { px, py });
            }
            if let Some((fx, fy)) = address::parse_field_name(one) {
                return Some(Target::Field { fx, fy });
            }
            let mut chars = one.chars();
            match (chars.next(), chars.next()) {
                (Some(letter), None) => {
                    address::block_of_letter(letter).map(|(bx, by)| Target::Block { bx, by })
                }
                _ => None,
            }
        }
        [first, second] if first.ends_with(',') || second.starts_with(',') => {
            parse_target(&[format!("{first}{second}").as_str()])
        }
        [direction, rest @ ..] => {
            let octant = octant_of_word(direction)?;
            let metres = match rest {
                [m] => {
                    let lower = m.to_ascii_lowercase();
                    strict_decimal(lower.strip_suffix('m').unwrap_or(&lower))?
                }
                [m, unit] if unit.eq_ignore_ascii_case("m") => strict_decimal(m)?,
                _ => return None,
            };
            let metres = u32::try_from(metres).ok()?;
            if metres == 0 || metres > MAX_BEARING_METRES {
                return None;
            }
            Some(Target::Bearing { octant, metres })
        }
        [] => None,
    }
}

/// An octant's unit step `(east, north)`.
fn octant_step(octant: Octant) -> (i64, i64) {
    match octant {
        Octant::N => (0, 1),
        Octant::NE => (1, 1),
        Octant::E => (1, 0),
        Octant::SE => (1, -1),
        Octant::S => (0, -1),
        Octant::SW => (-1, -1),
        Octant::W => (-1, 0),
        Octant::NW => (-1, 1),
    }
}

/// **The site cell a target names**, from the stand `from` (only a bearing reads it): a pocket's
/// address cell, a field's or a block's middle, the cell itself, or — for a bearing — an
/// orthogonal octant's `⌊(m + 5) / 10⌋` cells on its axis, a diagonal's `⌊(m × 100 + 707) /
/// 1414⌋` cells on each (10 m × √2 a diagonal cell). `None` when the cell is off the site.
#[must_use]
pub fn target_cell(target: &Target, from: (i64, i64)) -> Option<(i64, i64)> {
    let middle = |x: i64, y: i64, side: i64, offset: i64| -> Option<(i64, i64)> {
        Some((
            x.checked_mul(side)?.checked_add(offset)?,
            y.checked_mul(side)?.checked_add(offset)?,
        ))
    };
    let cell = match *target {
        Target::Pocket { px, py } => middle(px, py, CELLS_PER_POCKET, CELLS_PER_POCKET / 2)?,
        Target::Field { fx, fy } => middle(fx, fy, CELLS_PER_FIELD, CELLS_PER_FIELD / 2)?,
        Target::Block { bx, by } => middle(bx, by, CELLS_PER_BLOCK, CELLS_PER_BLOCK / 2)?,
        Target::Cell { x, y } => (x, y),
        Target::Bearing { octant, metres } => {
            let (dx, dy) = octant_step(octant);
            let m = i64::from(metres);
            let k = if dx != 0 && dy != 0 {
                (m.checked_mul(100)?.checked_add(707)?) / 1414
            } else {
                (m.checked_add(5)?) / 10
            };
            (
                from.0.checked_add(dx.checked_mul(k)?)?,
                from.1.checked_add(dy.checked_mul(k)?)?,
            )
        }
    };
    cell_on_site(cell).then_some(cell)
}

/// **A target in words** (the preview, the log and `doing_words`): `C117`, `field C-NW`,
/// `block C`, `cell 140,96`, `300 m NE`. At most 32 bytes.
#[must_use]
pub fn target_words(target: &Target) -> String {
    match *target {
        Target::Pocket { px, py } => {
            address::pocket_address(px, py).unwrap_or_else(|| "a pocket".to_owned())
        }
        Target::Field { fx, fy } => address::field_name(fx, fy)
            .map_or_else(|| "a field".to_owned(), |name| format!("field {name}")),
        Target::Block { bx, by } => address::letter_of_block(bx, by)
            .map_or_else(|| "a block".to_owned(), |letter| format!("block {letter}")),
        Target::Cell { x, y } => format!("cell {x},{y}"),
        Target::Bearing { octant, metres } => format!("{metres} m {}", octant.abbrev()),
    }
}

// ---- the leg ----------------------------------------------------------------------------------

/// **A planned leg** (§0 ruling 2 as refined by R1): the exact A* path and the frozen display leg
/// over it. Never stored whole: an act's params carry the waypoints and the figures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leg {
    /// The exact path, start cell first (kept for tests and the census; never stored).
    pub cells: Vec<(u16, u16)>,
    /// Σ `move_cost` along it — equal to the atlas `Path.cost`.
    pub cost: u64,
    /// The frozen display leg, `2..=`[`MAX_LEG_WAYPOINTS`]: the start pose at 0 s, then each kept
    /// cell's centre at the exact path's cumulative seconds there.
    pub waypoints: Vec<LegWaypoint>,
    /// The [`LEG_TOLERANCE_CELLS`] entry the freeze used.
    pub tolerance_cells: u32,
    /// [`secs_of_cost`] of the cost: the last waypoint's `at_s`.
    pub secs: u32,
    /// [`cu_of_cost`] of the cost.
    pub cu: u32,
    /// The exact path's length: `⌊(orthogonal steps × 1000 + diagonal steps × 1414) / 100⌋` m.
    pub metres: u32,
    /// The exact path's last step's octant × 45,000 (north 0, clockwise).
    pub heading_mdeg: u32,
}

/// **Why a leg refuses.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegRefused {
    /// The start cell is the target.
    Here,
    /// No way over the ground within the budget (the atlas's `Unreachable` or `OverBudget`).
    NoPath,
    /// More than [`MAX_LEG_WAYPOINTS`] points even at the widest tolerance; `needed` is the count
    /// there.
    Winding {
        /// The kept points at 8 cells.
        needed: usize,
    },
    /// A figure left its type.
    Overflow,
}

/// A cell's centre, centimetres.
fn centre_cm((x, y): (u16, u16)) -> (i64, i64) {
    let cell = i64::from(SITE_CELL_CM);
    let half = cell / 2;
    (
        i64::from(x).saturating_mul(cell).saturating_add(half),
        i64::from(y).saturating_mul(cell).saturating_add(half),
    )
}

/// The step kind between two neighbouring cells.
fn kind_of(from: (u16, u16), to: (u16, u16)) -> Option<StepKind> {
    StepKind::between(from, to)
}

/// One step's atlas cost (tracked, no tags) on `surface`.
fn step_cost(
    surface: &SurfaceComposite,
    table: &LocomotionTable,
    from: (u16, u16),
    to: (u16, u16),
) -> Option<u64> {
    let kind = kind_of(from, to)?;
    let here = surface.cells.get(from.0, from.1)?;
    let next = surface.cells.get(to.0, to.1)?;
    move_cost(
        table,
        here,
        next,
        kind,
        u32::from(surface.grid.resolution_cm),
        DRIVE_LOCOMOTION,
        &[],
    )
    .ok()
    .map(u64::from)
}

/// An octant's index clockwise from north (0..8) for a unit step `(dx, dy)`.
fn heading_of(dx: i32, dy: i32) -> u32 {
    match (dx.signum(), dy.signum()) {
        (0, 1) => 0,
        (1, 1) => 1,
        (1, 0) => 2,
        (1, -1) => 3,
        (0, -1) => 4,
        (-1, -1) => 5,
        (-1, 0) => 6,
        (-1, 1) => 7,
        _ => 0,
    }
}

/// The exact path's length in metres and its last step's heading.
fn length_and_heading(cells: &[(u16, u16)]) -> (u32, u32) {
    let mut orthogonal: u64 = 0;
    let mut diagonal: u64 = 0;
    let mut heading = 0_u32;
    for pair in cells.windows(2) {
        let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        match kind_of(a, b) {
            Some(StepKind::Orthogonal) => orthogonal = orthogonal.saturating_add(1),
            Some(StepKind::Diagonal) => diagonal = diagonal.saturating_add(1),
            None => {}
        }
        let dx = i32::from(b.0) - i32::from(a.0);
        let dy = i32::from(b.1) - i32::from(a.1);
        heading = heading_of(dx, dy).saturating_mul(45_000);
    }
    let metres = orthogonal
        .saturating_mul(1_000)
        .saturating_add(diagonal.saturating_mul(1_414))
        / 100;
    (u32::try_from(metres).unwrap_or(u32::MAX), heading)
}

/// **The leg from the exact points**: the waypoints at `kept` (index 0 the start pose at 0 s,
/// every other kept index its point at the exact cumulative seconds there), and the figures.
fn leg_of(
    cells: Vec<(u16, u16)>,
    points: &[(i64, i64)],
    cum_cost: &[u64],
    kept: &[usize],
    tolerance_cells: u32,
    rates: &DriveRates,
) -> Result<Leg, LegRefused> {
    let cost = cum_cost.last().copied().ok_or(LegRefused::NoPath)?;
    let secs = secs_of_cost(cost, rates).ok_or(LegRefused::Overflow)?;
    let cu = cu_of_cost(cost, rates).ok_or(LegRefused::Overflow)?;
    let mut waypoints = Vec::with_capacity(kept.len());
    for &i in kept {
        let (Some(&(x_cm, y_cm)), Some(&at)) = (points.get(i), cum_cost.get(i)) else {
            return Err(LegRefused::Overflow);
        };
        let at_s = if i == 0 {
            0
        } else {
            secs_of_cost(at, rates).ok_or(LegRefused::Overflow)?
        };
        waypoints.push(LegWaypoint { x_cm, y_cm, at_s });
    }
    let (metres, heading_mdeg) = length_and_heading(&cells);
    Ok(Leg {
        cells,
        cost,
        waypoints,
        tolerance_cells,
        secs,
        cu,
        metres,
        heading_mdeg,
    })
}

/// A path's points (centimetres) and the cumulative cost at each.
type PathPoints = (Vec<(i64, i64)>, Vec<u64>);

/// **The exact path's points and cumulative costs** (the freeze's inputs): index 0 the rig's
/// exact start pose, every later index its cell's centre; the cost re-read with `move_cost` along
/// the cells. `None` when a step does not cost (never, for a path the atlas found).
fn points_of(
    surface: &SurfaceComposite,
    table: &LocomotionTable,
    start_cm: (i64, i64),
    cells: &[(u16, u16)],
) -> Option<PathPoints> {
    let mut points = Vec::with_capacity(cells.len());
    let mut cum = Vec::with_capacity(cells.len());
    let mut total: u64 = 0;
    for (i, cell) in cells.iter().enumerate() {
        if i == 0 {
            points.push(start_cm);
            cum.push(0);
            continue;
        }
        let previous = *cells.get(i.checked_sub(1)?)?;
        total = total.checked_add(step_cost(surface, table, previous, *cell)?)?;
        points.push(centre_cm(*cell));
        cum.push(total);
    }
    Some((points, cum))
}

/// **ONE atlas `path`** (§0 ruling 2) from the start pose's cell to `to` over `surface`, tracked,
/// no tags, at [`DRIVE_PATH_BUDGET_CELLS`]; then [`freeze`]. `Unreachable` and `OverBudget` both
/// refuse [`LegRefused::NoPath`] (the brief's finding F8: through a site composite at a whole
/// site's budget an unreachable target exhausts the reachable cells first).
///
/// # Errors
///
/// [`LegRefused`].
pub fn drive_leg(
    surface: &SurfaceComposite,
    table: &LocomotionTable,
    start_cm: (i64, i64),
    to: (i64, i64),
    rates: &DriveRates,
) -> Result<Leg, LegRefused> {
    let from = site_cell(start_cm.0, start_cm.1);
    if from == to {
        return Err(LegRefused::Here);
    }
    let (Some(from_cell), Some(to_cell)) =
        (address::composite_cell(from), address::composite_cell(to))
    else {
        return Err(LegRefused::NoPath);
    };
    let found = match path(
        surface,
        table,
        from_cell,
        to_cell,
        DRIVE_LOCOMOTION,
        &[],
        DRIVE_PATH_BUDGET_CELLS,
    ) {
        PathOutcome::Found(found) => found,
        PathOutcome::Unreachable { .. } | PathOutcome::OverBudget { .. } => {
            return Err(LegRefused::NoPath);
        }
    };
    let (points, cum) =
        points_of(surface, table, start_cm, &found.cells).ok_or(LegRefused::NoPath)?;
    let (kept, tolerance) =
        freeze(&points, &LEG_TOLERANCE_CELLS).map_err(|needed| LegRefused::Winding { needed })?;
    leg_of(found.cells, &points, &cum, &kept, tolerance, rates)
}

/// Squared distance of `p` from the segment `a`–`b`, as an exact fraction `(numerator,
/// denominator)` in `i128`.
fn segment_distance2(p: (i64, i64), a: (i64, i64), b: (i64, i64)) -> (i128, i128) {
    let d = (
        i128::from(b.0) - i128::from(a.0),
        i128::from(b.1) - i128::from(a.1),
    );
    let v = (
        i128::from(p.0) - i128::from(a.0),
        i128::from(p.1) - i128::from(a.1),
    );
    let len2 = d.0 * d.0 + d.1 * d.1;
    let t = v.0 * d.0 + v.1 * d.1;
    if len2 == 0 || t <= 0 {
        return (v.0 * v.0 + v.1 * v.1, 1);
    }
    if t >= len2 {
        let w = (
            i128::from(p.0) - i128::from(b.0),
            i128::from(p.1) - i128::from(b.1),
        );
        return (w.0 * w.0 + w.1 * w.1, 1);
    }
    let cross = d.0 * v.1 - d.1 * v.0;
    (cross.saturating_mul(cross), len2)
}

/// `n1 / d1 > n2 / d2` for positive denominators, in checked `i128` (an overflow — never, for
/// site coordinates — reads as greater, which only keeps a point).
fn greater(n1: i128, d1: i128, n2: i128, d2: i128) -> bool {
    match (n1.checked_mul(d2), n2.checked_mul(d1)) {
        (Some(left), Some(right)) => left > right,
        _ => true,
    }
}

/// **One Douglas–Peucker pass at `tolerance_cells`** (integers, an explicit stack, ties to the
/// lowest index): the kept indices, ascending, the first and the last always among them.
fn simplify(points: &[(i64, i64)], tolerance_cells: u32) -> Vec<usize> {
    let n = points.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let last = n.saturating_sub(1);
    let tol = i128::from(tolerance_cells).saturating_mul(i128::from(SITE_CELL_CM));
    let tol2 = tol.saturating_mul(tol);
    let mut keep = vec![false; n];
    if let Some(first) = keep.first_mut() {
        *first = true;
    }
    if let Some(end) = keep.last_mut() {
        *end = true;
    }
    let mut stack: Vec<(usize, usize)> = vec![(0, last)];
    while let Some((a, b)) = stack.pop() {
        if b.saturating_sub(a) < 2 {
            continue;
        }
        let (Some(&pa), Some(&pb)) = (points.get(a), points.get(b)) else {
            continue;
        };
        let mut best: Option<(usize, i128, i128)> = None;
        for i in a.saturating_add(1)..b {
            let Some(&p) = points.get(i) else {
                continue;
            };
            let (num, den) = segment_distance2(p, pa, pb);
            let better = match best {
                None => true,
                Some((_, bn, bd)) => greater(num, den, bn, bd),
            };
            if better {
                best = Some((i, num, den));
            }
        }
        if let Some((i, num, den)) = best
            && greater(num, den, tol2, 1)
        {
            if let Some(slot) = keep.get_mut(i) {
                *slot = true;
            }
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    keep.iter()
        .enumerate()
        .filter_map(|(i, kept)| kept.then_some(i))
        .collect()
}

/// **The freeze** (R1): `points` are the exact path's points in centimetres — index 0 the rig's
/// exact start pose, every later index its cell's centre. Tries each tolerance in order; the
/// first whose kept points number at most [`MAX_LEG_WAYPOINTS`] wins: the kept indices
/// (ascending, the first and last always kept) and that tolerance.
///
/// # Errors
///
/// The count kept at the widest tolerance when none fits (`Winding`'s `needed`).
pub fn freeze(points: &[(i64, i64)], tolerances: &[u32]) -> Result<(Vec<usize>, u32), usize> {
    let mut widest = (0..points.len()).collect::<Vec<usize>>();
    for &tolerance in tolerances {
        let kept = simplify(points, tolerance);
        if kept.len() <= MAX_LEG_WAYPOINTS {
            return Ok((kept, tolerance));
        }
        widest = kept;
    }
    Err(widest.len())
}

/// **The freeze, never refused** (a recover's way home, the brief's decision P4): [`freeze`];
/// past the widest tolerance, every k-th of the widest's kept indices, `k` the least that fits,
/// the first and the last kept.
#[must_use]
pub fn freeze_always(points: &[(i64, i64)], tolerances: &[u32]) -> (Vec<usize>, u32) {
    if let Ok(found) = freeze(points, tolerances) {
        return found;
    }
    let widest = tolerances.last().copied().unwrap_or(0);
    let kept = simplify(points, widest);
    let last = kept.last().copied();
    let mut k = 2_usize;
    loop {
        let mut picked: Vec<usize> = kept.iter().copied().step_by(k).collect();
        if let Some(last) = last
            && picked.last() != Some(&last)
        {
            picked.push(last);
        }
        if picked.len() <= MAX_LEG_WAYPOINTS || k >= kept.len() {
            return (picked, widest);
        }
        k = k.saturating_add(1);
    }
}

// ---- the way home -------------------------------------------------------------------------------

/// **Cost-to-home over a whole site** (lead ruling 4; the brief's decision P1, R2, R3): for every
/// cell, the least atlas path cost (tracked, no tags, the same `move_cost`) from that cell to ANY
/// cell within Chebyshev 1 of the ramp's cell, and the first step of that least way. A
/// multi-source Dijkstra from those at most nine cells (cost 0) over reversed edges: pop `v`
/// (heap key `(cost, y, x)`, smallest first); for each of the atlas's eight directions `d` in its
/// order, the neighbour `u = v − d` is relaxed with `cost[v] + move_cost(u → v)`, and on a strict
/// improvement `next[u]` becomes `d`. Deterministic, at most one pop a cell. Self-contained: a
/// descent needs no composite (R3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeField {
    ramp: (i64, i64),
    width: u16,
    height: u16,
    /// Row-major from the south-west; `u32::MAX` unreachable.
    cost: Vec<u32>,
    /// The index into the atlas's eight directions of the first step home; [`NO_STEP`] at a
    /// source or an unreachable cell.
    next: Vec<u8>,
}

impl HomeField {
    /// The index of a cell, `None` off the grid.
    fn index(&self, (x, y): (i64, i64)) -> Option<usize> {
        let (x, y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// **Build the field** over `surface` for the ramp's cell `ramp`: `None` when the surface is
    /// empty or the ramp's cell is off it.
    #[must_use]
    pub fn build(
        surface: &SurfaceComposite,
        table: &LocomotionTable,
        ramp: (i64, i64),
    ) -> Option<HomeField> {
        let width = surface.grid.width;
        let height = surface.grid.height;
        let count = usize::from(width).checked_mul(usize::from(height))?;
        if count == 0 {
            return None;
        }
        let mut field = HomeField {
            ramp,
            width,
            height,
            cost: vec![u32::MAX; count],
            next: vec![NO_STEP; count],
        };
        field.index(ramp)?;
        let mut open: BinaryHeap<Reverse<(u32, u16, u16)>> = BinaryHeap::new();
        for dy in -1_i64..=1 {
            for dx in -1_i64..=1 {
                let cell = (ramp.0.saturating_add(dx), ramp.1.saturating_add(dy));
                let Some(i) = field.index(cell) else {
                    continue;
                };
                if let Some(slot) = field.cost.get_mut(i) {
                    *slot = 0;
                }
                if let (Ok(x), Ok(y)) = (u16::try_from(cell.0), u16::try_from(cell.1)) {
                    open.push(Reverse((0, y, x)));
                }
            }
        }
        let mut closed = vec![false; count];
        while let Some(Reverse((g, y, x))) = open.pop() {
            let Some(i) = field.index((i64::from(x), i64::from(y))) else {
                continue;
            };
            match closed.get_mut(i) {
                Some(true) | None => continue,
                Some(done) => *done = true,
            }
            for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let ux = i32::from(x) - dx;
                let uy = i32::from(y) - dy;
                let (Ok(ux), Ok(uy)) = (u16::try_from(ux), u16::try_from(uy)) else {
                    continue;
                };
                let Some(ui) = field.index((i64::from(ux), i64::from(uy))) else {
                    continue;
                };
                if closed.get(ui).copied().unwrap_or(true) {
                    continue;
                }
                let Some(step) = step_cost(surface, table, (ux, uy), (x, y)) else {
                    continue;
                };
                let Some(cand) = u64::from(g)
                    .checked_add(step)
                    .and_then(|c| u32::try_from(c).ok())
                    .filter(|c| *c < u32::MAX)
                else {
                    continue;
                };
                if let (Some(slot), Some(dir)) = (field.cost.get_mut(ui), field.next.get_mut(ui))
                    && cand < *slot
                {
                    *slot = cand;
                    *dir = u8::try_from(k).unwrap_or(NO_STEP);
                    open.push(Reverse((cand, uy, ux)));
                }
            }
        }
        Some(field)
    }

    /// The ramp's cell the field was built for.
    #[must_use]
    pub fn ramp(&self) -> (i64, i64) {
        self.ramp
    }

    /// **The least cost home from `cell`**; `None` off the grid or where there is no way home.
    #[must_use]
    pub fn cost_at(&self, cell: (i64, i64)) -> Option<u64> {
        let cost = *self.cost.get(self.index(cell)?)?;
        (cost != u32::MAX).then_some(u64::from(cost))
    }

    /// **The way home from `cell`** by each cell's first step, ending on a source (cost 0); its
    /// cumulative cost at each cell is `cost_at(start) − cost_at(cell)`, exactly. `None` when
    /// unreachable; at most one step a cell of the grid.
    #[must_use]
    pub fn descend(&self, cell: (i64, i64)) -> Option<Vec<(u16, u16)>> {
        self.cost_at(cell)?;
        let mut here = (u16::try_from(cell.0).ok()?, u16::try_from(cell.1).ok()?);
        let mut out = vec![here];
        let bound = self.cost.len();
        loop {
            let i = self.index((i64::from(here.0), i64::from(here.1)))?;
            if *self.cost.get(i)? == 0 {
                return Some(out);
            }
            let (dx, dy) = *NEIGHBOURS.get(usize::from(*self.next.get(i)?))?;
            here = (
                u16::try_from(i32::from(here.0) + dx).ok()?,
                u16::try_from(i32::from(here.1) + dy).ok()?,
            );
            out.push(here);
            if out.len() > bound {
                return None;
            }
        }
    }
}

/// Whether `stand` is within one cell of the ramp's (the recover's rule, SM.3b).
fn beside_ramp(field: &HomeField, stand: (i64, i64)) -> bool {
    cells_apart(stand, field.ramp) <= 1
}

/// **The charge home from an END stand an act would move the rig to** (a drive): beside the
/// ramp 0; else [`cu_of_cost`] of [`HomeField::cost_at`]. **`None` when the stand has no way
/// home** — the caller refuses `NO_WAY_HOME` (R2).
#[must_use]
pub fn home_cu_to(field: &HomeField, stand: (i64, i64), rates: &DriveRates) -> Option<u32> {
    if beside_ramp(field, stand) {
        return Some(0);
    }
    cu_of_cost(field.cost_at(stand)?, rates)
}

/// **The dearest cell the bound charges** (lead ruling 4): the tracked row's dearest finite
/// class cost (300 on the shipped table: rock), × [`DIAGONAL_PCT`] / 100, × (100 + the tracked
/// `max_slope_pct`) / 100, × [`HOME_BOUND_SAFETY`]. Never below one nominal cell.
#[must_use]
pub fn home_bound_cost_per_cell(table: &LocomotionTable) -> u64 {
    let (dearest, slope) =
        table
            .locomotions
            .get(DRIVE_LOCOMOTION)
            .map_or((u64::from(NOMINAL_COST), 0), |rows| {
                let dearest = rows
                    .classes
                    .values()
                    .flatten()
                    .chain(rows.default.iter())
                    .copied()
                    .max()
                    .unwrap_or(NOMINAL_COST);
                (u64::from(dearest), u64::from(rows.max_slope_pct))
            });
    (dearest.saturating_mul(u64::from(DIAGONAL_PCT)) / 100)
        .saturating_mul(100_u64.saturating_add(slope))
        .checked_div(100)
        .unwrap_or(0)
        .saturating_mul(HOME_BOUND_SAFETY)
        .max(u64::from(NOMINAL_COST))
}

/// **The charge home from the stand the rig ALREADY occupies** (a scan or a sounding where it
/// stands): as [`home_cu_to`], but a stand with no way home reads the conservative bound —
/// Chebyshev cells to the ramp × [`home_bound_cost_per_cell`], never 0 — because there is nothing
/// left to prevent there (R2). An overflow reads `u32::MAX` (refused by the reserve).
#[must_use]
pub fn home_cu_here(
    field: &HomeField,
    stand: (i64, i64),
    rates: &DriveRates,
    table: &LocomotionTable,
) -> u32 {
    if let Some(cu) = home_cu_to(field, stand, rates) {
        return cu;
    }
    let cells = cells_apart(stand, field.ramp).max(1);
    cells
        .checked_mul(home_bound_cost_per_cell(table))
        .and_then(|cost| cu_of_cost(cost, rates))
        .map_or(u32::MAX, |cu| cu.max(1))
}

/// **A recover's way home** (§3.6; the brief's decision P4): [`HomeField::descend`] from the
/// rig's cell, frozen by [`freeze_always`] — never refused for its waypoints — with `secs` and
/// `cu` from [`HomeField::cost_at`] of the start cell and every kept waypoint's `at_s` the
/// exact cumulative seconds there. `None` when the cell has no way home or already stands
/// beside the ramp.
#[must_use]
pub fn recover_leg(field: &HomeField, start_cm: (i64, i64), rates: &DriveRates) -> Option<Leg> {
    let start = site_cell(start_cm.0, start_cm.1);
    let total = field.cost_at(start)?;
    let cells = field.descend(start)?;
    if cells.len() < 2 {
        return None;
    }
    let mut points = Vec::with_capacity(cells.len());
    let mut cum = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        let here = field.cost_at((i64::from(cell.0), i64::from(cell.1)))?;
        cum.push(total.checked_sub(here)?);
        points.push(if i == 0 { start_cm } else { centre_cm(*cell) });
    }
    let (kept, tolerance) = freeze_always(&points, &LEG_TOLERANCE_CELLS);
    leg_of(cells, &points, &cum, &kept, tolerance, rates).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_atlas::cells::{Cell, Cells, MAX_TILE_CELLS};
    use mush_atlas::raster::Grid;
    use mush_atlas::{PointCm, SurfaceRef, TerrainClass};

    const RATES: DriveRates = DriveRates {
        max_speed_cms: 300,
        cu_per_nominal_cell: 15,
    };

    fn cell(class: TerrainClass, elev_cm: i16) -> Cell {
        Cell {
            class,
            elev_cm,
            cover: 0,
            room: 0,
        }
    }

    /// A flat synthetic 256 × 256 site of one class.
    fn flat(class: TerrainClass) -> SurfaceComposite {
        SurfaceComposite {
            surface: SurfaceRef::Ground,
            grid: Grid {
                origin: PointCm { x: 0, y: 0 },
                resolution_cm: 1_000,
                width: 256,
                height: 256,
            },
            cells: Cells::filled(256, 256, cell(class, 0), MAX_TILE_CELLS).expect("a grid"),
            rooms: Vec::new(),
            features: Vec::new(),
        }
    }

    /// A deterministic mixed, uneven grid (an integer hash of the coordinates, never an RNG):
    /// the three footings and ±100 cm of relief a cell, as the lane's per-cell terrain is.
    fn noisy() -> SurfaceComposite {
        let mut surface = flat(TerrainClass::Regolith);
        for y in 0..256_u16 {
            for x in 0..256_u16 {
                let h = (u32::from(x).wrapping_mul(73_856_093)
                    ^ u32::from(y).wrapping_mul(19_349_663))
                .wrapping_mul(2_654_435_761);
                let class = match (h >> 8) % 100 {
                    0..=52 => TerrainClass::Regolith,
                    53..=87 => TerrainClass::Rock,
                    _ => TerrainClass::Ice,
                };
                let elev = i16::try_from((h >> 16) % 201).unwrap_or(0) - 100;
                surface.cells.set(x, y, cell(class, elev));
            }
        }
        surface
    }

    fn table() -> &'static LocomotionTable {
        LocomotionTable::shipped()
    }

    fn at(x: i64, y: i64) -> (i64, i64) {
        (x * 1_000 + 500, y * 1_000 + 500)
    }

    /// **V2 — a kilometre of regolith is 500 IC s and 2,250 cu** (§13's acceptance; lead ruling
    /// 3): from `(20, 128)` to `(120, 128)` over a flat all-regolith site, two waypoints; rock is
    /// 1,000 s and 4,500 cu; ice 667 s and 3,000 cu (ceilings).
    #[test]
    fn sm4b_a_kilometre_of_regolith_is_500_s_and_2250_cu() {
        for (class, secs, cu) in [
            (TerrainClass::Regolith, 500, 2_250),
            (TerrainClass::Rock, 1_000, 4_500),
            (TerrainClass::Ice, 667, 3_000),
        ] {
            let leg = drive_leg(&flat(class), table(), at(20, 128), (120, 128), &RATES)
                .unwrap_or_else(|e| panic!("{class:?}: {e:?}"));
            assert_eq!((leg.secs, leg.cu), (secs, cu), "{class:?}");
            assert_eq!(leg.metres, 1_000);
            assert_eq!(leg.waypoints.len(), 2, "{class:?}: a straight leg");
            assert_eq!(leg.heading_mdeg, 90_000, "east");
            assert_eq!(leg.cells.len(), 101);
        }
    }

    /// The conversions are ceilings; a zero speed reads none.
    #[test]
    fn sm4b_secs_and_cu_round_up() {
        assert_eq!(secs_of_cost(0, &RATES), Some(0));
        assert_eq!(secs_of_cost(1, &RATES), Some(1));
        assert_eq!(secs_of_cost(150, &RATES), Some(5));
        assert_eq!(secs_of_cost(151, &RATES), Some(6));
        assert_eq!(cu_of_cost(150, &RATES), Some(23), "22.5 rounds up alone");
        assert_eq!(cu_of_cost(300, &RATES), Some(45));
        assert_eq!(cu_of_cost(1, &RATES), Some(1));
        let still = DriveRates {
            max_speed_cms: 0,
            ..RATES
        };
        assert_eq!(secs_of_cost(150, &still), None);
        assert_eq!(cu_of_cost(u64::MAX, &RATES), None);
    }

    /// **Every target kind parses and lands** (§10): case-insensitive; malformed refused; off
    /// the site `None`; a bearing's cells per octant.
    #[test]
    fn sm4b_targets_parse_and_land() {
        let parse = |text: &str| parse_target(&text.split_whitespace().collect::<Vec<_>>());
        let c117 = address::pocket_of_block_index('C', 116).expect("C117");
        assert_eq!(
            parse("C117"),
            Some(Target::Pocket {
                px: c117.0,
                py: c117.1
            })
        );
        assert_eq!(parse("c117"), parse("C117"));
        assert_eq!(
            target_cell(&parse("C117").expect("a target"), (0, 0)),
            Some((c117.0 * 4 + 2, c117.1 * 4 + 2))
        );
        assert_eq!(parse("C-NW"), Some(Target::Field { fx: 4, fy: 7 }));
        assert_eq!(parse("c-nw"), parse("C-NW"));
        assert_eq!(
            target_cell(&Target::Field { fx: 4, fy: 7 }, (0, 0)),
            Some((144, 240))
        );
        assert_eq!(parse("C"), Some(Target::Block { bx: 2, by: 3 }));
        assert_eq!(parse("c"), parse("C"));
        assert_eq!(
            target_cell(&Target::Block { bx: 2, by: 3 }, (0, 0)),
            Some((160, 224))
        );
        assert_eq!(parse("140,96"), Some(Target::Cell { x: 140, y: 96 }));
        assert_eq!(parse("140, 96"), parse("140,96"));
        assert_eq!(
            target_cell(&Target::Cell { x: 140, y: 96 }, (0, 0)),
            Some((140, 96))
        );
        for text in ["ne 300", "NE 300", "northeast 300", "ne 300m", "ne 300 M"] {
            assert_eq!(
                parse(text),
                Some(Target::Bearing {
                    octant: Octant::NE,
                    metres: 300
                }),
                "{text}"
            );
        }
        // A diagonal: ⌊(300 × 100 + 707) / 1414⌋ = 21 cells each way; an axis ⌊(m + 5) / 10⌋.
        let from = (100, 100);
        let bearing = |octant, metres| target_cell(&Target::Bearing { octant, metres }, from);
        assert_eq!(bearing(Octant::NE, 300), Some((121, 121)));
        assert_eq!(bearing(Octant::SW, 300), Some((79, 79)));
        assert_eq!(bearing(Octant::NW, 300), Some((79, 121)));
        assert_eq!(bearing(Octant::SE, 300), Some((121, 79)));
        assert_eq!(bearing(Octant::N, 300), Some((100, 130)));
        assert_eq!(bearing(Octant::E, 304), Some((130, 100)));
        assert_eq!(bearing(Octant::E, 305), Some((131, 100)));
        assert_eq!(bearing(Octant::S, 300), Some((100, 70)));
        assert_eq!(bearing(Octant::W, 300), Some((70, 100)));
        assert_eq!(bearing(Octant::N, 3_000), None, "off the site");
        assert_eq!(target_cell(&Target::Cell { x: 256, y: 0 }, from), None);
        for bad in [
            "",
            "C0",
            "C257",
            "C0117",
            "Q1",
            "C-XX",
            "C-N",
            "Q",
            "AB",
            "140",
            "140,",
            "-1,4",
            "140,256x",
            "ne",
            "ne 0",
            "ne 3621",
            "ne 300 km",
            "up 300",
            "ne 3 0",
            "C117 go",
        ] {
            let parsed = parse(bad);
            let landed = parsed.and_then(|t| target_cell(&t, from));
            assert!(
                parsed.is_none() || landed.is_none(),
                "{bad:?} parsed as {parsed:?}"
            );
        }
        assert_eq!(target_words(&parse("C117").expect("t")), "C117");
        assert_eq!(target_words(&parse("c-nw").expect("t")), "field C-NW");
        assert_eq!(target_words(&parse("c").expect("t")), "block C");
        assert_eq!(target_words(&parse("140,96").expect("t")), "cell 140,96");
        assert_eq!(
            target_words(&parse("northeast 300").expect("t")),
            "300 m NE"
        );
    }

    /// The freeze keeps the ends and the shape: a straight path keeps 2; an L keeps 3; every
    /// dropped point lies within the tolerance of its kept segment; ties go to the lowest index.
    #[test]
    fn sm4b_the_freeze_keeps_the_ends_and_the_shape() {
        let straight: Vec<(i64, i64)> = (0..50).map(|x| at(x, 10)).collect();
        assert_eq!(freeze(&straight, &[1]), Ok((vec![0, 49], 1)));
        let mut l: Vec<(i64, i64)> = (0..20).map(|x| at(x, 10)).collect();
        l.extend((11..30).map(|y| at(19, y)));
        assert_eq!(freeze(&l, &[1]), Ok((vec![0, 19, l.len() - 1], 1)));
        // Every dropped point lies within the tolerance of its kept segment.
        let field = noisy();
        let leg = drive_leg(&field, table(), at(10, 10), (200, 190), &RATES).expect("a leg");
        let points: Vec<(i64, i64)> = leg
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| if i == 0 { at(10, 10) } else { centre_cm(*c) })
            .collect();
        let (kept, tol) = freeze(&points, &LEG_TOLERANCE_CELLS).expect("fits");
        let limit = i128::from(tol) * 1_000;
        for pair in kept.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            for p in &points[a + 1..b] {
                let (num, den) = segment_distance2(*p, points[a], points[b]);
                assert!(num <= limit * limit * den, "within {tol} cells");
            }
        }
        // Ties to the lowest index (the review's conformance/F4, tests/T2): points 1 and 3 stand
        // equally far (3 cells) off the chord 0–4. At 2 cells the lower, 1, is kept; then 3
        // stands √(900/234) ≈ 1.96 cells off the sub-chord 1–4 and is dropped — so the rule
        // shows: ties to the highest index would keep [0, 3, 4] instead. At 1 cell both sides
        // are kept whichever goes first.
        let tie = vec![at(0, 0), at(5, 3), at(10, 0), at(15, 3), at(20, 0)];
        assert_eq!(
            simplify(&tie, 2),
            vec![0, 1, 4],
            "the lowest of the tied points"
        );
        assert_eq!(freeze(&tie, &[2]), Ok((vec![0, 1, 4], 2)));
        let mirrored: Vec<(i64, i64)> = tie.iter().rev().copied().collect();
        assert_eq!(
            simplify(&mirrored, 2),
            vec![0, 1, 4],
            "mirrored, the lowest index is the other point"
        );
        assert_eq!(simplify(&tie, 1), vec![0, 1, 2, 3, 4]);
    }

    /// **A leg sums its steps** (§0 ruling 2): Σ `move_cost` along the cells equals the atlas
    /// `Path.cost`; `at_s` is non-decreasing, the last equals `secs`, each kept `at_s` the exact
    /// cumulative seconds at its cell.
    #[test]
    fn sm4b_a_leg_sums_its_steps() {
        let surface = noisy();
        let leg = drive_leg(&surface, table(), at(30, 40), (220, 200), &RATES).expect("a leg");
        let found = match path(
            &surface,
            table(),
            (30, 40),
            (220, 200),
            DRIVE_LOCOMOTION,
            &[],
            DRIVE_PATH_BUDGET_CELLS,
        ) {
            PathOutcome::Found(found) => found,
            other => panic!("{other:?}"),
        };
        assert_eq!(leg.cost, u64::from(found.cost));
        assert_eq!(leg.cells, found.cells);
        let mut cum = vec![0_u64];
        for pair in leg.cells.windows(2) {
            let step = step_cost(&surface, table(), pair[0], pair[1]).expect("a step");
            cum.push(cum.last().copied().unwrap_or(0) + step);
        }
        assert_eq!(cum.last().copied(), Some(leg.cost));
        assert!(leg.waypoints.windows(2).all(|w| w[0].at_s <= w[1].at_s));
        assert_eq!(leg.waypoints.last().map(|w| w.at_s), Some(leg.secs));
        assert_eq!(leg.waypoints.first().map(|w| w.at_s), Some(0));
        assert_eq!(
            leg.waypoints.first().map(|w| (w.x_cm, w.y_cm)),
            Some(at(30, 40))
        );
        for w in leg.waypoints.iter().skip(1) {
            let i = leg
                .cells
                .iter()
                .position(|c| centre_cm(*c) == (w.x_cm, w.y_cm))
                .expect("a kept cell");
            assert_eq!(Some(w.at_s), secs_of_cost(cum[i], &RATES));
        }
        assert_eq!(leg.cu, cu_of_cost(leg.cost, &RATES).expect("cu"));
    }

    /// A zigzag of `kinks` turns, 10 cells apart east and 20 cells high: every sub-chord the
    /// simplification can draw over it leaves its farthest kink more than 8 cells off (two
    /// steps: 20 cells; three: 11.1), so every point is kept at every tolerance.
    fn zigzag(kinks: usize) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        for k in 0..=kinks {
            let k = i64::try_from(k).expect("small");
            out.push(at(k * 10, if k % 2 == 0 { 0 } else { 20 }));
        }
        out
    }

    /// **More than 32 waypoints is refused, never cut** (R1): a zigzag needing 32 points even at
    /// 8 cells is accepted at 8; one needing 33 is `Err(33)`; the boundary pinned both ways.
    #[test]
    fn sm4b_a_leg_past_32_waypoints_is_refused_never_cut() {
        let fits = zigzag(31);
        assert_eq!(fits.len(), 32);
        let (kept, tol) = freeze(&fits, &LEG_TOLERANCE_CELLS).expect("32 fit");
        assert_eq!((kept.len(), tol), (32, 1));
        let over = zigzag(32);
        assert_eq!(over.len(), 33);
        assert_eq!(freeze(&over, &LEG_TOLERANCE_CELLS), Err(33));
        assert_eq!(MAX_LEG_WAYPOINTS, 32);
        // A recover's freeze never refuses: past 8 cells every k-th, the ends kept.
        let (kept, tol) = freeze_always(&over, &LEG_TOLERANCE_CELLS);
        assert!(kept.len() <= MAX_LEG_WAYPOINTS, "{}", kept.len());
        assert_eq!(kept.first(), Some(&0));
        assert_eq!(kept.last(), Some(&32));
        assert_eq!(tol, 8);
    }

    /// The tolerances are tried in order: a gentle weave fits at 1 cell; a wider weave first fits
    /// at 4.
    #[test]
    fn sm4b_the_freeze_tries_the_tolerances_in_order() {
        // 60 kinks 0.5 cells off the chord: at 1 cell every kink stays (61 points), at 2 none.
        let weave = |off_cm: i64, kinks: i64| -> Vec<(i64, i64)> {
            (0..=kinks)
                .map(|k| (k * 4_000, if k % 2 == 0 { 0 } else { off_cm }))
                .collect()
        };
        let gentle = weave(500, 60);
        assert_eq!(freeze(&gentle, &LEG_TOLERANCE_CELLS), Ok((vec![0, 60], 1)));
        let wide = weave(3_000, 60);
        let (kept, tol) = freeze(&wide, &LEG_TOLERANCE_CELLS).expect("fits at 4");
        assert_eq!((kept, tol), (vec![0, 60], 4));
    }

    /// **No path** (F8): an enclosed target is `NoPath` through `drive_leg`; and the atlas `path`
    /// at a tiny budget returns `OverBudget`, which `drive_leg`'s mapping also turns into
    /// `NoPath`; the start cell is `Here`.
    #[test]
    fn sm4b_no_path() {
        let mut surface = flat(TerrainClass::Regolith);
        for (x, y) in ring((100, 100), 1) {
            surface.cells.set(x, y, cell(TerrainClass::Wall, 0));
        }
        assert_eq!(
            drive_leg(&surface, table(), at(10, 10), (100, 100), &RATES),
            Err(LegRefused::NoPath)
        );
        assert!(matches!(
            path(
                &surface,
                table(),
                (10, 10),
                (100, 100),
                DRIVE_LOCOMOTION,
                &[],
                3
            ),
            PathOutcome::OverBudget { budget: 3 }
        ));
        assert_eq!(
            drive_leg(&surface, table(), at(10, 10), (10, 10), &RATES),
            Err(LegRefused::Here)
        );
    }

    /// The cells of a Chebyshev ring of radius `r` around `c`.
    fn ring(c: (u16, u16), r: u16) -> Vec<(u16, u16)> {
        let mut out = Vec::new();
        for y in c.1 - r..=c.1 + r {
            for x in c.0 - r..=c.0 + r {
                if x.abs_diff(c.0).max(y.abs_diff(c.1)) == r {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// The least A* cost from `c` to any of the nine beside-ramp cells.
    fn astar_home(surface: &SurfaceComposite, c: (u16, u16), ramp: (u16, u16)) -> Option<u64> {
        let mut best: Option<u64> = None;
        for dy in -1_i32..=1 {
            for dx in -1_i32..=1 {
                let t = (
                    u16::try_from(i32::from(ramp.0) + dx).expect("on"),
                    u16::try_from(i32::from(ramp.1) + dy).expect("on"),
                );
                let cost = if t == c {
                    Some(0)
                } else {
                    match path(
                        surface,
                        table(),
                        c,
                        t,
                        DRIVE_LOCOMOTION,
                        &[],
                        DRIVE_PATH_BUDGET_CELLS,
                    ) {
                        PathOutcome::Found(found) => Some(u64::from(found.cost)),
                        _ => None,
                    }
                };
                if let Some(cost) = cost {
                    best = Some(best.map_or(cost, |b| b.min(cost)));
                }
            }
        }
        best
    }

    /// **The home field equals A*** (the brief's decision P1): on the flat grid and on a mixed
    /// noisy one, for 64 sampled cells `cost_at` equals the least `path` cost to the nine
    /// beside-ramp cells — exact, never under — and `descend`'s steps sum to it.
    #[test]
    fn sm4b_the_home_field_equals_astar() {
        let ramp = (128_i64, 124_i64);
        for surface in [flat(TerrainClass::Regolith), noisy()] {
            let field = HomeField::build(&surface, table(), ramp).expect("a field");
            assert_eq!(field.ramp(), ramp);
            for n in 0..64_i64 {
                let c = ((n * 37 + 11) % 256, (n * 91 + 5) % 256);
                let cu16 = (
                    u16::try_from(c.0).expect("x"),
                    u16::try_from(c.1).expect("y"),
                );
                let want = astar_home(&surface, cu16, (128, 124));
                assert_eq!(field.cost_at(c), want, "{c:?}");
                let way = field.descend(c).expect("a way home");
                let mut sum = 0_u64;
                for pair in way.windows(2) {
                    sum += step_cost(&surface, table(), pair[0], pair[1]).expect("a step");
                }
                assert_eq!(Some(sum), field.cost_at(c), "{c:?}: the descent's steps");
                let end = way.last().copied().expect("an end");
                assert!(
                    cells_apart((i64::from(end.0), i64::from(end.1)), ramp) <= 1,
                    "it ends beside the ramp"
                );
            }
        }
    }

    /// **No way home is `None`, and the bound never understates** (R2): a pocket walled by an
    /// impassable ring reads `None` from `home_cu_to` and the bound from `home_cu_here` — above 0
    /// and at least any reachable cell's exact figure at the same distance.
    #[test]
    fn sm4b_no_way_home_is_none_and_the_bound_never_understates() {
        let mut surface = noisy();
        for (x, y) in ring((60, 60), 2) {
            surface.cells.set(x, y, cell(TerrainClass::Wall, 0));
        }
        let field = HomeField::build(&surface, table(), (128, 124)).expect("a field");
        assert_eq!(field.cost_at((60, 60)), None);
        assert_eq!(field.descend((60, 60)), None);
        assert_eq!(home_cu_to(&field, (60, 60), &RATES), None);
        let bound = home_cu_here(&field, (60, 60), &RATES, table());
        assert!(bound > 0);
        assert_eq!(home_bound_cost_per_cell(table()), 1_226);
        // Every reachable cell at the same Chebyshev distance costs no more than the bound.
        let d = cells_apart((60, 60), (128, 124));
        let mut checked = 0;
        for y in 0..256_i64 {
            for x in 0..256_i64 {
                if cells_apart((x, y), (128, 124)) == d
                    && let Some(cu) = home_cu_to(&field, (x, y), &RATES)
                {
                    assert!(cu <= bound, "({x}, {y}): {cu} > {bound}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
        // Beside the ramp the way home is free.
        assert_eq!(home_cu_to(&field, (129, 125), &RATES), Some(0));
        assert_eq!(home_cu_here(&field, (128, 124), &RATES, table()), 0);
    }

    /// **A recover's descent is never refused** (the brief's decision P4): a home field whose
    /// way out weaves past what 8 cells keeps in 32 points still yields at most 32 waypoints, the
    /// exact figures, the start pose first and a beside-ramp cell last.
    #[test]
    fn sm4b_a_recover_descent_is_never_refused() {
        // A serpentine of walls: the way home from the far corner snakes back and forth.
        let mut surface = flat(TerrainClass::Regolith);
        for wall in 0..40_u16 {
            let y = 4 + wall * 3;
            for x in 0..256_u16 {
                let gap = if wall % 2 == 0 { 250 } else { 5 };
                if x.abs_diff(gap) > 1 {
                    surface.cells.set(x, y, cell(TerrainClass::Wall, 0));
                }
            }
        }
        let field = HomeField::build(&surface, table(), (128, 124)).expect("a field");
        let start = at(10, 1);
        let way = field.descend((10, 1)).expect("a way home");
        let points: Vec<(i64, i64)> = way
            .iter()
            .enumerate()
            .map(|(i, c)| if i == 0 { start } else { centre_cm(*c) })
            .collect();
        assert!(
            freeze(&points, &LEG_TOLERANCE_CELLS).is_err(),
            "the way home winds past the drive's cap"
        );
        let leg = recover_leg(&field, start, &RATES).expect("never refused");
        assert!(leg.waypoints.len() <= MAX_LEG_WAYPOINTS);
        assert_eq!(leg.cost, field.cost_at((10, 1)).expect("a cost"));
        assert_eq!(leg.secs, secs_of_cost(leg.cost, &RATES).expect("secs"));
        assert_eq!(leg.cu, cu_of_cost(leg.cost, &RATES).expect("cu"));
        assert_eq!(leg.waypoints.first().map(|w| (w.x_cm, w.y_cm)), Some(start));
        assert_eq!(leg.waypoints.last().map(|w| w.at_s), Some(leg.secs));
        let end = leg.cells.last().copied().expect("an end");
        assert!(cells_apart((i64::from(end.0), i64::from(end.1)), (128, 124)) <= 1);
        assert_eq!(
            recover_leg(&field, at(129, 125), &RATES),
            None,
            "beside the ramp"
        );
    }
}
