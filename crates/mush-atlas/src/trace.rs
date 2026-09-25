//! # Traces (doc 14 §5.2; plan §3.6, S5): sight, fire, beams and hearing
//! over a surface
//!
//! Every query walks a straight line of cells between two eyes and reads the
//! same records whether it ends in a hit or a miss (the O lens): the walk never
//! stops early — the first blocker is *remembered*, the rest of the line is
//! still visited — so its timing and its reads say nothing about the answer.
//! Every query is pure over the [`SurfaceComposite`] and integer arithmetic,
//! so the same inputs give the same trace across restarts (the D lens).
//!
//! What blocks: a wall or any opaque class; a rise in the ground above the
//! line; a feature that `blocks.sight`, is not destroyed, covers the cell and
//! stands in the line's height band — a window feature (`blocks.sight`
//! false) does not. Light woods attenuate ([`Opacity::Partial`]) and a line
//! whose attenuation reaches a hundred is blocked. The endpoints never block
//! their own line; the target cell's class gives the cover the trace reports.
//!
//! # Two grids, one walk (16b cut S.1)
//!
//! A [`SurfaceComposite`] is a *window's* view of the world: a surface
//! reference, a room table and a feature overlay on top of the cells. The
//! combat arena (doc 16 §3.1, D-16-1/D-16-3) may hold none of those — it is a
//! library with no store, no window and no records — but it does hold a cell
//! grid of its own, and doc 16 §3.6 needs exactly the terrain half of this
//! walk over it. So the walk reads its ground through [`CellGrid`], which is
//! *one cell's class and elevation and nothing else*;
//! [`SurfaceComposite`] implements it, and [`trace_cells`] is the same
//! algorithm with an empty feature overlay: there is no second walk for cells.
//! The arena's early-stopping trace below ([`sight_only`], 16b cut J.1) is a
//! second *walker* over one line, one height and one per-cell rule
//! ([`terrain_sight_rule`]), never a second rule.
//!
//! # The arena's trace (16b cut J.1)
//!
//! There is one line walker that stops early, [`sight_only`], and it is the
//! combat arena's alone (doc 16 §3.2, §3.6). It walks the same cells in the
//! same order ([`Bresenham`] from the [`canonical_ends`]), at the same height
//! at every step ([`line_height_cm`]), under the same per-cell rule
//! ([`terrain_sight_rule`]) as the walk below, allocates nothing, and returns
//! at the first blocker — so it answers the same [`Sight`] and reads fewer
//! cells. That is an exception to the O lens above, taken on purpose: the
//! arena pass's cost already depends on what the hidden field holds before any
//! line is walked — the index cull and the thermal cull decide which lines are
//! traced at all — and no reader of the arena can time one line (a frame
//! carries no timing; the pass's cost is bounded per pulse and never observed
//! per line), so the early stop adds no channel. The walk, and every world
//! trace over it, keeps the rule above; a contract scan
//! (`mush-contract-tests`' `arena_library.rs`) holds [`sight_only`] to the
//! arena.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::physics::{Cover, Opacity, WALL_HEIGHT_CM, obstacle_height_cm, props};
use crate::record::{FeatureState, RecordKey};
use crate::terrain::TerrainClass;
use crate::window::{FeatureOverlay, SurfaceComposite};

/// **A plain cell grid a trace can walk** (16b cut S.1).
///
/// The whole of what the terrain half of a line walk reads: a cell's ground
/// class and its elevation above the datum, in centimetres, or `None` off the
/// grid. Everything else a [`SurfaceComposite`] carries — the surface
/// reference, the room table, the feature overlay — is a *window's*, and a
/// caller that has cells but no window (the combat arena) implements this and
/// gets the same walk.
pub trait CellGrid {
    /// The class and elevation of `(x, y)`, or `None` off the grid.
    fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)>;
}

impl CellGrid for SurfaceComposite {
    fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)> {
        let cell = self.cells.get(x, y)?;
        Some((cell.class, i32::from(cell.elev_cm)))
    }
}

/// What a line walk over a plain [`CellGrid`] found (16b cut S.1).
///
/// [`Sight`] is the answer; [`CellTrace::target_cover`] is the target cell's own
/// class read as cover (16b cut W.2); the other three are what the caller's own
/// budget is measured in — doc 16 §2 W4 bounds *traces and cell probes per
/// pulse*, and a bound nobody can count is not a bound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTrace {
    /// Clear, attenuated or blocked.
    pub sight: Sight,
    /// **The target cell's cover** (16b cut W.2; doc 16 §3.5 step 1: *cover from
    /// `direct_fire_path.target_cover` scales the presented area*).
    ///
    /// The same value [`Trace::target_cover`] carries and computed by the same
    /// walk; it was simply dropped before this cut, because cut S.1 asked a line
    /// walk only whether it could be seen along. The combat arena holds a cell
    /// grid and no [`SurfaceComposite`] (doc 16 §3.1, D-16-3), so this is how it
    /// reads the one field its resolution needs without a window.
    pub target_cover: Cover,
    /// Attenuation gathered along the line, percent.
    pub attenuation: u32,
    /// Cells read, hit or miss.
    pub visited: u32,
    /// The range, in cells (Chebyshev) — never a distance (doc 16 §3.2).
    pub range_cells: u32,
}

/// A standing person's eye above the ground, centimetres.
pub const EYE_HEIGHT_CM: i32 = 150;

/// Hearing: the points one cell of open ground costs.
pub const HEARING_PER_CELL: u32 = 2;

/// Hearing: the points a wall in the line costs.
pub const HEARING_WALL: u32 = 40;

/// Hearing: the points a sight-blocking feature in the line costs.
pub const HEARING_FEATURE: u32 = 20;

/// Hearing: the points a heavy-woods cell costs on top of its distance.
pub const HEARING_WOODS: u32 = 5;

/// An eye: a cell and a height above its ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eye {
    /// The cell.
    pub cell: (u16, u16),
    /// Height above the cell's elevation, centimetres.
    pub height_cm: i32,
}

impl Eye {
    /// A standing person at `cell`.
    #[must_use]
    pub fn standing(cell: (u16, u16)) -> Eye {
        Eye {
            cell,
            height_cm: EYE_HEIGHT_CM,
        }
    }
}

/// What blocked a line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blocker {
    /// An opaque class, or attenuation that reached a hundred.
    Terrain {
        /// The cell.
        at: (u16, u16),
        /// Its class.
        class: TerrainClass,
    },
    /// A feature that blocks sight.
    Feature {
        /// The cell.
        at: (u16, u16),
        /// The feature.
        key: RecordKey,
    },
    /// The ground rose above the line.
    Elevation {
        /// The cell.
        at: (u16, u16),
        /// Its elevation, centimetres.
        elev_cm: i32,
    },
    /// The line left the grid.
    Edge,
}

impl Blocker {
    /// A short account.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Blocker::Terrain { at, class } => format!("{} at ({}, {})", class.label(), at.0, at.1),
            Blocker::Feature { at, key } => format!("{} at ({}, {})", key.entry, at.0, at.1),
            Blocker::Elevation { at, elev_cm } => {
                format!("the ground ({} cm) at ({}, {})", elev_cm, at.0, at.1)
            }
            Blocker::Edge => "the edge of the map".to_string(),
        }
    }
}

/// What a line of sight found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sight {
    /// Nothing in the way.
    Clear,
    /// Attenuated by this percent.
    Partial(u8),
    /// Blocked.
    BlockedBy(Blocker),
}

impl Sight {
    /// Whether anything can be seen at all.
    #[must_use]
    pub fn is_clear_or_partial(&self) -> bool {
        !matches!(self, Sight::BlockedBy(_))
    }
}

/// Something a trace crossed, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Crossed {
    /// A feature (whatever it blocks).
    Feature {
        /// The cell.
        at: (u16, u16),
        /// The feature.
        key: RecordKey,
    },
    /// A unit standing on the line.
    Unit {
        /// The cell.
        at: (u16, u16),
        /// The unit.
        id: ObjId,
    },
}

/// A trace (doc 14 §5.2): every cell crossed, what stood on them, where it
/// stopped, the target's cover and the ground's profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    /// The cells, `from` first, `to` last.
    pub cells: Vec<(u16, u16)>,
    /// Each cell's elevation, centimetres.
    pub profile: Vec<i32>,
    /// The line's height at each cell, centimetres (the datum's).
    pub line_cm: Vec<i32>,
    /// Features and units crossed, in order.
    pub crossed: Vec<Crossed>,
    /// Where it stopped: the index into `cells` and why; `None` reached the target.
    pub stop: Option<(usize, Blocker)>,
    /// The target cell's cover.
    pub target_cover: Cover,
    /// Attenuation gathered, percent (a beam's; a sight's partial).
    pub attenuation: u32,
    /// The range, in cells (Chebyshev).
    pub range_cells: u32,
    /// Cells read, hit or miss.
    pub visited: u32,
}

impl Trace {
    /// Whether it reached the target.
    #[must_use]
    pub fn reaches(&self) -> bool {
        self.stop.is_none()
    }
}

/// Hearing between two eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hearing {
    /// Attenuation, points: distance, walls, features, woods.
    pub attenuation: u32,
    /// Whether no sound carries at all (a vacuum).
    pub silent: bool,
    /// Cells read.
    pub visited: u32,
}

impl Hearing {
    /// Whether a sound of `loudness` points is heard.
    #[must_use]
    pub fn audible(&self, loudness: u32) -> bool {
        !self.silent && loudness > self.attenuation
    }
}

/// The cells of the straight line from `a` to `b`, both included (Bresenham),
/// in order from `a`.
///
/// **One line, whichever end it is walked from** (16b cut M.4; 16c `B-56`). A
/// bare Bresenham breaks its ties toward the end it starts from, so on a line
/// like two across and one down `(0,0)→(2,1)` walks `(1,1)` where
/// `(2,1)→(0,0)` walks `(1,0)` — two different lines of sight between the same
/// two tanks, and A could see B while B could not see A. So the line is always
/// **drawn from the lesser endpoint in tuple order** (`(x, y)` compared
/// lexicographically): when `b < a` it is walked from `b` to `a` and the result
/// reversed. The two directions then visit one cell set, in opposite order.
#[must_use]
pub fn line_cells(a: (u16, u16), b: (u16, u16)) -> Vec<(u16, u16)> {
    let (lo, hi) = canonical_ends(a, b);
    let mut out: Vec<(u16, u16)> = Bresenham::new(lo, hi).collect();
    if (lo, hi) != (a, b) {
        out.reverse();
    }
    out
}

/// **The canonical direction of a line** (16b cut M.4; 16c `B-56`): the lesser
/// endpoint in tuple order first — `(b, a)` when `b < a`, else `(a, b)`.
///
/// The one place the rule [`line_cells`] states is written, so the walk and
/// [`sight_only`] cannot draw one line two ways.
#[must_use]
pub fn canonical_ends(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if b < a { (b, a) } else { (a, b) }
}

/// **The bare Bresenham walk from `a` to `b`, both included, as an iterator**
/// (16b cut J.1) — the canonical direction [`line_cells`] draws every line in,
/// with nothing allocated: the iterator *is* the line.
///
/// It yields `a` first and `b` last, then `None`, and it is exactly the walk
/// `line_cells` collected before this cut: `err = dx + dy`, `e2 = 2 × err`, a
/// step in `x` when `e2 >= dy` and in `y` when `e2 <= dx`. It does **not**
/// canonicalise its ends — a bare Bresenham breaks its ties toward the end it
/// starts from — so a caller that wants one line from either end starts it
/// from [`canonical_ends`].
#[derive(Clone, Debug)]
pub struct Bresenham {
    /// The cursor, `x`.
    x0: i32,
    /// The cursor, `y`.
    y0: i32,
    /// The far end, `x`.
    x1: i32,
    /// The far end, `y`.
    y1: i32,
    /// `|x1 − x0|` at the start.
    dx: i32,
    /// `−|y1 − y0|` at the start.
    dy: i32,
    /// The step in `x`, ±1.
    sx: i32,
    /// The step in `y`, ±1.
    sy: i32,
    /// The running error term.
    err: i32,
    /// Whether the far end has been yielded.
    done: bool,
}

impl Bresenham {
    /// The line from `a` to `b`, walked from `a`.
    #[must_use]
    pub fn new(a: (u16, u16), b: (u16, u16)) -> Bresenham {
        let (x0, y0) = (i32::from(a.0), i32::from(a.1));
        let (x1, y1) = (i32::from(b.0), i32::from(b.1));
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        Bresenham {
            x0,
            y0,
            x1,
            y1,
            dx,
            dy,
            sx: if x0 < x1 { 1 } else { -1 },
            sy: if y0 < y1 { 1 } else { -1 },
            err: dx + dy,
            done: false,
        }
    }
}

impl Iterator for Bresenham {
    type Item = (u16, u16);

    fn next(&mut self) -> Option<(u16, u16)> {
        // A cell off the `u16` range is skipped, as the collected walk skipped
        // it; between two `u16` ends the cursor never leaves the range, so the
        // guard is the old one kept rather than a case that occurs.
        while !self.done {
            let here = (u16::try_from(self.x0), u16::try_from(self.y0));
            if self.x0 == self.x1 && self.y0 == self.y1 {
                self.done = true;
            } else {
                let e2 = 2 * self.err;
                if e2 >= self.dy {
                    self.err += self.dy;
                    self.x0 += self.sx;
                }
                if e2 <= self.dx {
                    self.err += self.dx;
                    self.y0 += self.sy;
                }
            }
            if let (Ok(x), Ok(y)) = here {
                return Some((x, y));
            }
        }
        None
    }
}

/// How the line's height runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    /// Straight from eye to eye.
    Straight,
    /// An arc: `apex_cm` above the straight line at the middle.
    Arc { apex_cm: i32 },
}

/// The height of the line at step `i` of `n`.
///
/// **The same height from either end** (16b cut M.4; 16c `B-56`). The straight
/// term is `(z_from × (n − i) + z_to × i) / n` in `i64`. Walked from the other
/// end the same cell is step `n − i` with the two heights swapped, and the
/// numerator is the same sum, so the one division gives the one answer. The
/// form it replaces, `z_from + (z_to − z_from) × i / n`, truncated its fraction
/// toward zero, which from the other end is the ceiling instead of the floor:
/// with 150 cm eyes on ground a metre apart in height (eyes at 150 and 250 cm),
/// `n = 59` and `i = 29` came out at 199 cm one way and 200 cm the other, and a
/// 200 cm ridge in that cell was a grazing tie that blocked A and not B. The
/// arc term,
/// `4·h·i·(n−i)/n²`, was already symmetric. The straight term is
/// [`line_height_cm`] (16b cut J.1), which [`sight_only`] reads as well.
fn line_height(z_from: i32, z_to: i32, i: usize, n: usize, flight: Flight) -> i32 {
    let straight = line_height_cm(z_from, z_to, i, n);
    let extra = match flight {
        Flight::Straight => 0,
        // 4·h·t·(1−t), t = i/n, in integers.
        Flight::Arc { apex_cm } => {
            let n = n.max(1);
            let (i64i, i64n) = (i64::try_from(i).unwrap_or(0), i64::try_from(n).unwrap_or(1));
            4 * i64::from(apex_cm) * i64i * (i64n - i64i) / (i64n * i64n)
        }
    };
    i32::try_from(i64::from(straight) + extra).unwrap_or(i32::MAX)
}

/// **A straight line's height at step `i` of `n`, centimetres** (16b cut J.1;
/// the straight arm of the walk's height, 16b cut M.4): `(z_from × (n − i) +
/// z_to × i) / n` in `i64`, `n` floored at one — the one height from either
/// end, for the walk and for [`sight_only`] alike.
#[must_use]
pub fn line_height_cm(z_from: i32, z_to: i32, i: usize, n: usize) -> i32 {
    let n = n.max(1);
    let (i64i, i64n) = (i64::try_from(i).unwrap_or(0), i64::try_from(n).unwrap_or(1));
    let straight = (i64::from(z_from) * (i64n - i64i) + i64::from(z_to) * i64i) / i64n;
    i32::try_from(straight).unwrap_or(i32::MAX)
}

struct Walk<'a> {
    grid: &'a dyn CellGrid,
    features: &'a [FeatureOverlay],
    units: &'a [(ObjId, (u16, u16))],
}

/// What the walk reads at one cell; `None` off the grid.
struct Read {
    class: TerrainClass,
    elev_cm: i32,
    feature_blocking: Option<RecordKey>,
    features: Vec<RecordKey>,
    units: Vec<ObjId>,
}

impl Walk<'_> {
    fn read(&self, at: (u16, u16), line_cm: i32) -> Option<Read> {
        let (class, elev_cm) = self.grid.cell_at(at.0, at.1)?;
        let band = line_cm - elev_cm;
        let mut feature_blocking = None;
        let mut features = Vec::new();
        for f in self.features {
            if !f.cells.contains(&at) {
                continue;
            }
            features.push(f.key.clone());
            if f.blocks.sight
                && f.state != FeatureState::Destroyed
                && band >= f.z_range_cm.0
                && band <= f.z_range_cm.1
                && feature_blocking.is_none()
            {
                feature_blocking = Some(f.key.clone());
            }
        }
        let units = self
            .units
            .iter()
            .filter(|(_, c)| *c == at)
            .map(|(id, _)| *id)
            .collect();
        Some(Read {
            class,
            elev_cm,
            feature_blocking,
            features,
            units,
        })
    }
}

/// The straight-line trace, the engine's one line walk. `opacity_stops`:
/// whether opaque classes and attenuation block (sight, fire, beams) or only
/// the ground and walls' tops do (an arc).
fn walk(
    grid: &dyn CellGrid,
    features: &[FeatureOverlay],
    units: &[(ObjId, (u16, u16))],
    from: Eye,
    to: Eye,
    flight: Flight,
    opacity_stops: bool,
) -> Trace {
    let cells = line_cells(from.cell, to.cell);
    let n = cells.len().saturating_sub(1);
    let w = Walk {
        grid,
        features,
        units,
    };
    let z_from = grid
        .cell_at(from.cell.0, from.cell.1)
        .map_or(0, |(_, elev_cm)| elev_cm)
        + from.height_cm;
    let z_to = grid
        .cell_at(to.cell.0, to.cell.1)
        .map_or(0, |(_, elev_cm)| elev_cm)
        + to.height_cm;
    let mut profile = Vec::with_capacity(cells.len());
    let mut line_cm = Vec::with_capacity(cells.len());
    let mut crossed = Vec::new();
    let mut stop: Option<(usize, Blocker)> = None;
    let mut attenuation: u32 = 0;
    let mut visited = 0u32;
    let mut target_cover = Cover::None;
    for (i, &at) in cells.iter().enumerate() {
        let z = line_height(z_from, z_to, i, n, flight);
        line_cm.push(z);
        visited += 1;
        let Some(read) = w.read(at, z) else {
            profile.push(0);
            if stop.is_none() {
                stop = Some((i, Blocker::Edge));
            }
            continue;
        };
        profile.push(read.elev_cm);
        for key in read.features {
            crossed.push(Crossed::Feature { at, key });
        }
        for id in read.units {
            crossed.push(Crossed::Unit { at, id });
        }
        let endpoint = i == 0 || i == n;
        if i == n {
            target_cover = props(read.class).cover;
        }
        if endpoint {
            continue;
        }
        // Every rule is evaluated for every cell; only the FIRST blocker is
        // kept, and nothing here returns early.
        let mut blocker: Option<Blocker> = if opacity_stops {
            // Opacity, attenuation and the ground, in that order: the one
            // per-cell rule `sight_only` reads too (16b cut J.1).
            terrain_sight_rule(at, read.class, read.elev_cm, z, &mut attenuation)
        } else {
            let top = read.elev_cm + obstacle_height_cm(read.class);
            // An arc: whatever has a top (a wall, a canopy) blocks when the
            // arc passes below it; opacity is nothing to a shell.
            if obstacle_height_cm(read.class) > 0 && top > z {
                Some(Blocker::Terrain {
                    at,
                    class: read.class,
                })
            } else if read.elev_cm > z {
                Some(Blocker::Elevation {
                    at,
                    elev_cm: read.elev_cm,
                })
            } else {
                None
            }
        };
        if let Some(key) = read.feature_blocking
            && blocker.is_none()
        {
            blocker = Some(Blocker::Feature { at, key });
        }
        if let Some(b) = blocker
            && stop.is_none()
        {
            stop = Some((i, b));
        }
    }
    let range_cells = u32::from(
        from.cell
            .0
            .abs_diff(to.cell.0)
            .max(from.cell.1.abs_diff(to.cell.1)),
    );
    Trace {
        cells,
        profile,
        line_cm,
        crossed,
        stop,
        target_cover,
        attenuation,
        range_cells,
        visited,
    }
}

/// Line of sight from `a` to `b` (doc 14 §5.2).
#[must_use]
pub fn line_of_sight(surface: &SurfaceComposite, a: Eye, b: Eye) -> Sight {
    let t = walk(
        surface,
        &surface.features,
        &[],
        a,
        b,
        Flight::Straight,
        true,
    );
    sight_of(&t)
}

/// A trace's answer as a [`Sight`]: the first blocker, else the attenuation it
/// gathered, else clear. Written once so [`line_of_sight`] and [`trace_cells`]
/// cannot read one walk two ways.
fn sight_of(t: &Trace) -> Sight {
    match &t.stop {
        Some((_, blocker)) => Sight::BlockedBy(blocker.clone()),
        None if t.attenuation > 0 => {
            Sight::Partial(u8::try_from(t.attenuation.min(99)).unwrap_or(99))
        }
        None => Sight::Clear,
    }
}

/// **Line of sight over a plain cell grid** (16b cut S.1; doc 16 §3.2 "Line of
/// sight", §3.6 step 2).
///
/// [`line_of_sight`] with no feature overlay and no units on the line: the same
/// `walk`, the same opacity, attenuation and elevation rules, over a grid that
/// is only classes and elevations. It is what the combat arena's fire path
/// traces with (16b cut W.2: the shot, the reflex's engage check and the cover
/// readings want [`CellTrace::target_cover`] and the attenuation from the
/// shooter's own end), because the arena holds a cell grid and may hold no
/// window and no records at all (doc 16 §3.1, D-16-1/D-16-3). The arena's
/// perception pass traces with [`sight_only`] since 16b cut J.1.
///
/// The counters come back with the answer. `visited` counts every cell of the
/// line, both endpoints included, and is therefore *not* the arena's
/// `cell_probes` (doc 16 §2 W4), which since 16b cut J.1 is
/// [`CellSight::visited`]: the interior cells up to the first blocker.
#[must_use]
pub fn trace_cells(grid: &dyn CellGrid, a: Eye, b: Eye) -> CellTrace {
    let t = walk(grid, &[], &[], a, b, Flight::Straight, true);
    CellTrace {
        sight: sight_of(&t),
        target_cover: t.target_cover,
        attenuation: t.attenuation,
        visited: t.visited,
        range_cells: t.range_cells,
    }
}

/// **The per-cell terrain rule of a line of sight** (16b cut J.1): what one
/// interior cell of class `class` and ground `elev_cm` does to a line passing it
/// at `line_cm`.
///
/// An [`Opacity::Opaque`] class blocks ([`Blocker::Terrain`]); an
/// [`Opacity::Partial`] one adds its percent to `attenuation` (saturating) and
/// blocks when the sum has reached a hundred; then, with nothing blocking yet,
/// ground above the line blocks ([`Blocker::Elevation`]); else `None`. The order
/// — opacity, then elevation, and the feature rule after both in the walk — is
/// the walk's and does not move: it decides which blocker a cell names. Written
/// once so the walk (whenever opacity stops it) and [`sight_only`] cannot read a
/// cell two ways.
pub fn terrain_sight_rule(
    at: (u16, u16),
    class: TerrainClass,
    elev_cm: i32,
    line_cm: i32,
    attenuation: &mut u32,
) -> Option<Blocker> {
    match props(class).opacity {
        Opacity::Opaque => return Some(Blocker::Terrain { at, class }),
        Opacity::Partial(pct) => {
            *attenuation = attenuation.saturating_add(u32::from(pct));
            if *attenuation >= 100 {
                return Some(Blocker::Terrain { at, class });
            }
        }
        Opacity::Clear => {}
    }
    if elev_cm > line_cm {
        return Some(Blocker::Elevation { at, elev_cm });
    }
    None
}

/// What [`sight_only`] found: the sight, and the interior cells it read to find
/// it (16b cut J.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSight {
    /// Clear, attenuated or blocked — exactly the canonical walk's answer.
    pub sight: Sight,
    /// **Interior cells read, hit or miss, up to and including the blocking one;
    /// never an endpoint**, so a one-cell line reads nothing. It is not
    /// [`CellTrace::visited`], which counts every cell of the line, both
    /// endpoints included.
    pub visited: u32,
}

/// **The combat arena's line of sight** (16b cut J.1; doc 16 §3.2 "Line of
/// sight", §3.6 step 2): [`trace_cells`]' answer, with no allocation, stopping at
/// the first blocker.
///
/// The line is drawn from its [`canonical_ends`], each end keeping its own
/// `height_cm`, and walked by [`Bresenham`]; the ground under the two ends is
/// read for the line's heights and never counted; the interior cells are read in
/// order under [`terrain_sight_rule`] at [`line_height_cm`], and the first that
/// blocks — or leaves the grid ([`Blocker::Edge`]) — is the answer. An endpoint
/// never blocks its own line and is never read as a cell, but **an endpoint off
/// the grid is the edge**: the lesser one at once (with nothing read), the other
/// only when no interior cell blocked first — which is where the walk sets it.
///
/// **It is exact.** For every `a`, `b` and grid — the endpoints on the grid or
/// not — `sight_only(grid, a, b).sight == trace_cells(grid, lo, hi).sight`, where
/// `(lo, hi)` are the canonical ends carrying their own heights: the same cells
/// in the same order, the same height at every step ([`line_height_cm`]), the
/// same rule per cell ([`terrain_sight_rule`]), the same `Edge` at an off-grid
/// endpoint, the first blocker kept by both — and a walk that continues past its
/// first blocker changes nothing it reports but `visited` and `target_cover`,
/// neither of which this function answers. Therefore, by 16c `B-56` (16b cut
/// M.4: a line is one line from either end), `sees(sight_only(grid, a, b).sight,
/// k) == sees(trace_cells(grid, a, b).sight, k)` for every budget `k`, with equal
/// attenuation. The blocker it names is the first from the *lesser* endpoint; a
/// caller that needs the blocker from its own end uses [`trace_cells`].
///
/// **It is the O lens's one exception.** This is the combat arena's trace (doc
/// 16 §3.2, §3.6) and it stops at the first blocker on purpose. The arena pass's
/// cost already depends on what the hidden field holds before any line is walked
/// — the index cull and the thermal cull decide which lines are traced at all —
/// and no reader of the arena can time one line (a frame carries no timing; the
/// pass's cost is bounded per pulse and never observed per line), so the early
/// stop adds no channel. The walk, and every world trace over it, keeps the
/// module head's rule, and a contract scan holds `sight_only` to the arena.
#[must_use]
pub fn sight_only(grid: &dyn CellGrid, a: Eye, b: Eye) -> CellSight {
    // Each end keeps its own height: swapped, `lo` is `b`'s cell at `b`'s height.
    let (lo, hi) = if canonical_ends(a.cell, b.cell) == (a.cell, b.cell) {
        (a, b)
    } else {
        (b, a)
    };
    // The walk's own `z_from` / `z_to` reads, which it does not count either.
    // The lesser end off the grid is the walk's `Edge` at step 0, set before it
    // skips the endpoint.
    let Some((_, lo_elev_cm)) = grid.cell_at(lo.cell.0, lo.cell.1) else {
        return CellSight {
            sight: Sight::BlockedBy(Blocker::Edge),
            visited: 0,
        };
    };
    let hi_read = grid.cell_at(hi.cell.0, hi.cell.1);
    let z_lo = lo_elev_cm + lo.height_cm;
    let z_hi = hi_read.map_or(0, |(_, elev_cm)| elev_cm) + hi.height_cm;
    let n = usize::from(
        lo.cell
            .0
            .abs_diff(hi.cell.0)
            .max(lo.cell.1.abs_diff(hi.cell.1)),
    );
    let mut attenuation = 0_u32;
    let mut visited = 0_u32;
    for (i, at) in Bresenham::new(lo.cell, hi.cell).enumerate() {
        if i == 0 || i == n {
            continue;
        }
        visited = visited.saturating_add(1);
        let Some((class, elev_cm)) = grid.cell_at(at.0, at.1) else {
            return CellSight {
                sight: Sight::BlockedBy(Blocker::Edge),
                visited,
            };
        };
        let line_cm = line_height_cm(z_lo, z_hi, i, n);
        if let Some(blocker) = terrain_sight_rule(at, class, elev_cm, line_cm, &mut attenuation) {
            return CellSight {
                sight: Sight::BlockedBy(blocker),
                visited,
            };
        }
    }
    let sight = if hi_read.is_none() {
        // The walk's step `n`: reached only when nothing earlier blocked.
        Sight::BlockedBy(Blocker::Edge)
    } else if attenuation > 0 {
        Sight::Partial(u8::try_from(attenuation.min(99)).unwrap_or(99))
    } else {
        Sight::Clear
    };
    CellSight { sight, visited }
}

/// A direct-fire trace: the straight line, stopping at the first blocker,
/// reporting the target's cover and everything crossed.
#[must_use]
pub fn direct_fire_path(
    surface: &SurfaceComposite,
    units: &[(ObjId, (u16, u16))],
    from: Eye,
    to: Eye,
) -> Trace {
    walk(
        surface,
        &surface.features,
        units,
        from,
        to,
        Flight::Straight,
        true,
    )
}

/// An indirect-fire trace: an arc `apex_cm` above the straight line at its
/// middle, clearing whatever stands below it; only the ground and the tops of
/// walls and woods block, never opacity.
#[must_use]
pub fn indirect_fire_path(
    surface: &SurfaceComposite,
    units: &[(ObjId, (u16, u16))],
    from: Eye,
    to: Eye,
    apex_cm: i32,
) -> Trace {
    walk(
        surface,
        &surface.features,
        units,
        from,
        to,
        Flight::Arc {
            apex_cm: apex_cm.max(0),
        },
        false,
    )
}

/// A beam: the straight line with no drop, attenuated by every partial cell
/// (`attenuation`, percent), stopped by an opaque one.
#[must_use]
pub fn beam_path(
    surface: &SurfaceComposite,
    units: &[(ObjId, (u16, u16))],
    from: Eye,
    to: Eye,
) -> Trace {
    let level = Eye {
        cell: to.cell,
        height_cm: to.height_cm,
    };
    walk(
        surface,
        &surface.features,
        units,
        from,
        level,
        Flight::Straight,
        true,
    )
}

/// Hearing from `a` to `b`: distance, walls, sight-blocking features and
/// heavy woods attenuate; a vacuum carries nothing.
#[must_use]
pub fn hearing(surface: &SurfaceComposite, a: Eye, b: Eye) -> Hearing {
    let t = walk(
        surface,
        &surface.features,
        &[],
        a,
        b,
        Flight::Straight,
        false,
    );
    let mut attenuation = t.range_cells.saturating_mul(HEARING_PER_CELL);
    let mut silent = false;
    let mut visited = 0u32;
    for &at in &t.cells {
        visited += 1;
        let Some(cell) = surface.cells.get(at.0, at.1) else {
            continue;
        };
        let p = props(cell.class);
        if p.medium == "space" {
            silent = true;
        }
        match cell.class {
            TerrainClass::Wall => attenuation = attenuation.saturating_add(HEARING_WALL),
            TerrainClass::WoodsHeavy => attenuation = attenuation.saturating_add(HEARING_WOODS),
            _ => {}
        }
        for f in &surface.features {
            if f.blocks.sight && f.state != FeatureState::Destroyed && f.cells.contains(&at) {
                attenuation = attenuation.saturating_add(HEARING_FEATURE);
            }
        }
    }
    let _ = WALL_HEIGHT_CM;
    Hearing {
        attenuation,
        silent,
        visited: visited.max(t.visited),
    }
}

/// The room a cell is in (doc 14 §3.4's resolver, on one surface).
#[must_use]
pub fn resolve_room(surface: &SurfaceComposite, at: (u16, u16)) -> Option<ObjId> {
    surface.room_at(at.0, at.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::{Cell, Cells};
    use crate::geom::PointCm;
    use crate::raster::Grid;
    use crate::record::{Blocks, FeatureKind, SurfaceRef};
    use crate::window::FeatureOverlay;

    fn surface(w: u16, h: u16, class: TerrainClass) -> SurfaceComposite {
        SurfaceComposite {
            surface: SurfaceRef::Ground,
            grid: Grid {
                origin: PointCm { x: 0, y: 0 },
                resolution_cm: 100,
                width: w,
                height: h,
            },
            cells: Cells::filled(
                w,
                h,
                Cell {
                    class,
                    elev_cm: 0,
                    cover: 0,
                    room: 0,
                },
                65_536,
            )
            .expect("cells"),
            rooms: Vec::new(),
            features: Vec::new(),
        }
    }

    fn set(s: &mut SurfaceComposite, x: u16, y: u16, class: TerrainClass, elev_cm: i16) {
        s.cells.set(
            x,
            y,
            Cell {
                class,
                elev_cm,
                cover: 0,
                room: 0,
            },
        );
    }

    fn feature(key: &str, cells: Vec<(u16, u16)>, sight: bool, z: (i32, i32)) -> FeatureOverlay {
        FeatureOverlay {
            key: RecordKey::parse_suffix(key).expect("key"),
            kind: FeatureKind::Counter,
            state: FeatureState::Intact,
            blocks: Blocks {
                movement: true,
                sight,
            },
            cells,
            z_range_cm: z,
        }
    }

    #[test]
    fn a_line_walks_every_cell_between_its_ends() {
        assert_eq!(
            line_cells((0, 0), (3, 0)),
            vec![(0, 0), (1, 0), (2, 0), (3, 0)]
        );
        assert_eq!(line_cells((0, 0), (2, 2)), vec![(0, 0), (1, 1), (2, 2)]);
        assert_eq!(line_cells((3, 1), (0, 1)).len(), 4);
        assert_eq!(line_cells((5, 5), (5, 5)), vec![(5, 5)]);
    }

    #[test]
    fn sight_is_blocked_by_a_wall_and_not_by_a_window_feature() {
        let mut s = surface(10, 3, TerrainClass::Paved);
        let a = Eye::standing((0, 1));
        let b = Eye::standing((9, 1));
        assert_eq!(line_of_sight(&s, a, b), Sight::Clear);
        set(&mut s, 5, 1, TerrainClass::Wall, 0);
        assert_eq!(
            line_of_sight(&s, a, b),
            Sight::BlockedBy(Blocker::Terrain {
                at: (5, 1),
                class: TerrainClass::Wall
            })
        );
        // A window (blocks movement, not sight) in a doorway cell is see-through.
        set(&mut s, 5, 1, TerrainClass::Door, 0);
        s.features
            .push(feature("lot:window", vec![(5, 1)], false, (0, 300)));
        assert_eq!(line_of_sight(&s, a, b), Sight::Clear);
        // A shutter that blocks sight in the eye's band is not.
        s.features
            .push(feature("lot:shutter", vec![(5, 1)], true, (0, 300)));
        assert!(matches!(
            line_of_sight(&s, a, b),
            Sight::BlockedBy(Blocker::Feature { at: (5, 1), .. })
        ));
        // A destroyed shutter is gone.
        s.features[1].state = FeatureState::Destroyed;
        assert_eq!(line_of_sight(&s, a, b), Sight::Clear);
        // A low counter (below the eyes) does not block sight either.
        s.features[1].state = FeatureState::Intact;
        s.features[1].z_range_cm = (0, 100);
        assert_eq!(line_of_sight(&s, a, b), Sight::Clear);
    }

    #[test]
    fn sight_is_blocked_by_a_rise_and_partial_through_light_woods() {
        let mut s = surface(10, 1, TerrainClass::Ground);
        let a = Eye::standing((0, 0));
        let b = Eye::standing((9, 0));
        // A two-metre bank in the middle stands above two standing eyes.
        set(&mut s, 5, 0, TerrainClass::Ground, 200);
        assert_eq!(
            line_of_sight(&s, a, b),
            Sight::BlockedBy(Blocker::Elevation {
                at: (5, 0),
                elev_cm: 200
            })
        );
        // From a rooftop the same bank is under the line.
        let high = Eye {
            cell: (0, 0),
            height_cm: 800,
        };
        assert_eq!(line_of_sight(&s, high, b), Sight::Clear);
        // Two cells of light woods: partial; four: blocked.
        let mut s = surface(10, 1, TerrainClass::Ground);
        set(&mut s, 3, 0, TerrainClass::WoodsLight, 0);
        set(&mut s, 4, 0, TerrainClass::WoodsLight, 0);
        assert_eq!(line_of_sight(&s, a, b), Sight::Partial(50));
        set(&mut s, 5, 0, TerrainClass::WoodsLight, 0);
        set(&mut s, 6, 0, TerrainClass::WoodsLight, 0);
        assert!(matches!(
            line_of_sight(&s, a, b),
            Sight::BlockedBy(Blocker::Terrain {
                at: (6, 0),
                class: TerrainClass::WoodsLight
            })
        ));
    }

    #[test]
    fn a_direct_trace_stops_at_the_first_blocker_and_reports_the_targets_cover() {
        let mut s = surface(10, 1, TerrainClass::Paved);
        set(&mut s, 4, 0, TerrainClass::Wall, 0);
        set(&mut s, 7, 0, TerrainClass::Wall, 0);
        set(&mut s, 9, 0, TerrainClass::Rubble, 0);
        let units = [(ObjId(7), (2u16, 0u16)), (ObjId(8), (9, 0))];
        let t = direct_fire_path(&s, &units, Eye::standing((0, 0)), Eye::standing((9, 0)));
        assert_eq!(
            t.stop,
            Some((
                4,
                Blocker::Terrain {
                    at: (4, 0),
                    class: TerrainClass::Wall
                }
            ))
        );
        assert_eq!(t.target_cover, Cover::Partial);
        assert_eq!(t.cells.len(), 10);
        assert_eq!(t.visited, 10, "the walk reads every cell, hit or miss");
        assert_eq!(
            t.crossed,
            vec![
                Crossed::Unit {
                    at: (2, 0),
                    id: ObjId(7)
                },
                Crossed::Unit {
                    at: (9, 0),
                    id: ObjId(8)
                }
            ]
        );
        // A miss reads the same cells.
        let open = surface(10, 1, TerrainClass::Paved);
        let m = direct_fire_path(&open, &[], Eye::standing((0, 0)), Eye::standing((9, 0)));
        assert!(m.reaches());
        assert_eq!(m.visited, t.visited);
        assert_eq!(m.cells, t.cells);
    }

    #[test]
    fn an_indirect_trace_clears_a_wall_below_its_apex_and_a_beam_attenuates() {
        let mut s = surface(11, 1, TerrainClass::Paved);
        set(&mut s, 5, 0, TerrainClass::Wall, 0);
        let a = Eye::standing((0, 0));
        let b = Eye::standing((10, 0));
        // A three-metre wall: a flat arc hits it, a five-metre apex clears it.
        assert!(!indirect_fire_path(&s, &[], a, b, 0).reaches());
        let high = indirect_fire_path(&s, &[], a, b, 500);
        assert!(high.reaches(), "{:?}", high.stop);
        assert!(high.line_cm[5] > WALL_HEIGHT_CM);
        // The arc is not stopped by woods (opacity), only by tops.
        set(&mut s, 5, 0, TerrainClass::WoodsHeavy, 0);
        assert!(indirect_fire_path(&s, &[], a, b, 900).reaches());
        assert!(
            !indirect_fire_path(&s, &[], a, b, 100).reaches(),
            "the canopy's top is eight metres"
        );
        // A beam through three cells of light woods reaches, attenuated 75.
        let mut s = surface(11, 1, TerrainClass::Paved);
        for x in 3..6 {
            set(&mut s, x, 0, TerrainClass::WoodsLight, 0);
        }
        let beam = beam_path(&s, &[], a, b);
        assert!(beam.reaches());
        assert_eq!(beam.attenuation, 75);
        set(&mut s, 6, 0, TerrainClass::WoodsLight, 0);
        assert!(!beam_path(&s, &[], a, b).reaches(), "a hundred stops it");
    }

    #[test]
    fn hearing_attenuates_by_distance_walls_and_features_and_not_in_a_vacuum() {
        let mut s = surface(21, 1, TerrainClass::Paved);
        let a = Eye::standing((0, 0));
        let b = Eye::standing((20, 0));
        let open = hearing(&s, a, b);
        assert_eq!(open.attenuation, 40, "twenty cells at two");
        assert!(open.audible(41));
        assert!(!open.audible(40));
        set(&mut s, 10, 0, TerrainClass::Wall, 0);
        let walled = hearing(&s, a, b);
        assert_eq!(walled.attenuation, 80);
        s.features
            .push(feature("lot:shutter", vec![(12, 0)], true, (0, 300)));
        assert_eq!(hearing(&s, a, b).attenuation, 100);
        assert_eq!(hearing(&s, a, b).visited, walled.visited);
        let mut moon = surface(21, 1, TerrainClass::Regolith);
        set(&mut moon, 3, 0, TerrainClass::Regolith, 0);
        assert!(hearing(&moon, a, b).silent);
        assert!(!hearing(&moon, a, b).audible(u32::MAX));
    }

    #[test]
    fn the_same_inputs_give_the_same_trace() {
        let mut s = surface(12, 12, TerrainClass::Ground);
        set(&mut s, 6, 6, TerrainClass::Wall, 0);
        set(&mut s, 3, 8, TerrainClass::Ground, 120);
        let a = Eye::standing((0, 11));
        let b = Eye::standing((11, 0));
        let first = direct_fire_path(&s, &[], a, b);
        let again = direct_fire_path(&s.clone(), &[], a, b);
        assert_eq!(first, again);
        let json = serde_json::to_string(&first).expect("json");
        let back: Trace = serde_json::from_str(&json).expect("trace");
        assert_eq!(back, first);
    }

    /// A grid that is nothing but classes and elevations — what the combat arena
    /// has, and the shape [`CellGrid`] exists for.
    struct PlainGrid {
        width: u16,
        height: u16,
        cells: Vec<(TerrainClass, i32)>,
    }

    impl PlainGrid {
        /// The same ground a [`SurfaceComposite`] holds, with its surface
        /// reference, room table and feature overlay dropped.
        fn of(s: &SurfaceComposite) -> PlainGrid {
            let (width, height) = (s.cells.width(), s.cells.height());
            let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
            for y in 0..height {
                for x in 0..width {
                    cells.push(s.cell_at(x, y).unwrap_or((TerrainClass::Void, 0)));
                }
            }
            PlainGrid {
                width,
                height,
                cells,
            }
        }
    }

    impl CellGrid for PlainGrid {
        fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)> {
            if x >= self.width || y >= self.height {
                return None;
            }
            self.cells
                .get(usize::from(y) * usize::from(self.width) + usize::from(x))
                .copied()
        }
    }

    /// **16b cut S.1: `trace_cells` is `line_of_sight`, over cells alone.**
    ///
    /// The combat arena's fire path traces with [`trace_cells`] (and its
    /// perception with [`sight_only`], which answers the same [`Sight`] as the
    /// canonical walk; the J.1 tests below) because it may hold no window
    /// and no records (doc 16 §3.1, D-16-1/D-16-3). That is only safe while the two
    /// are the *same* walk, so every terrain rule the walk has — an opaque class, a
    /// rise above the line, attenuation that reaches a hundred, the edge, the
    /// endpoints never blocking their own line — is asserted to give one answer
    /// through both doors, on a feature-free surface and on the plain grid taken
    /// off it.
    #[test]
    fn trace_cells_answers_what_line_of_sight_answers_over_the_same_ground() {
        let mut s = surface(12, 12, TerrainClass::Ground);
        set(&mut s, 6, 6, TerrainClass::Wall, 0);
        set(&mut s, 3, 8, TerrainClass::Ground, 120);
        set(&mut s, 4, 4, TerrainClass::WoodsLight, 0);
        set(&mut s, 5, 5, TerrainClass::WoodsHeavy, 0);
        set(&mut s, 8, 2, TerrainClass::WoodsLight, 0);
        set(&mut s, 9, 1, TerrainClass::WoodsLight, 0);
        let grid = PlainGrid::of(&s);
        let mut compared = 0_usize;
        let mut clear = 0_usize;
        let mut partial = 0_usize;
        let mut blocked = 0_usize;
        for height_cm in [EYE_HEIGHT_CM, 400] {
            for ax in 0..12_u16 {
                for by in 0..12_u16 {
                    let a = Eye {
                        cell: (ax, 11),
                        height_cm,
                    };
                    let b = Eye {
                        cell: (11, by),
                        height_cm: EYE_HEIGHT_CM,
                    };
                    let through_window = line_of_sight(&s, a, b);
                    let through_cells = trace_cells(&grid, a, b);
                    assert_eq!(
                        through_cells.sight, through_window,
                        "the two doors disagree from ({ax}, 11) at {height_cm} cm to (11, {by})"
                    );
                    match through_window {
                        Sight::Clear => clear += 1,
                        Sight::Partial(_) => partial += 1,
                        Sight::BlockedBy(_) => blocked += 1,
                    }
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, 288);
        assert!(clear > 0 && blocked > 0, "{clear}/{partial}/{blocked}");

        // The sweep's geometry does not guarantee every answer, so each of the
        // three is also produced deliberately, on one row, through both doors.
        let a = Eye::standing((0, 0));
        let b = Eye::standing((9, 0));
        for (paint, want) in [
            (Vec::new(), Sight::Clear),
            (vec![(4_u16, TerrainClass::WoodsLight)], Sight::Partial(25)),
            (
                vec![(3, TerrainClass::WoodsLight), (4, TerrainClass::WoodsHeavy)],
                Sight::Partial(85),
            ),
            (
                vec![(5, TerrainClass::Wall)],
                Sight::BlockedBy(Blocker::Terrain {
                    at: (5, 0),
                    class: TerrainClass::Wall,
                }),
            ),
        ] {
            let mut row = surface(10, 1, TerrainClass::Ground);
            for (x, class) in paint {
                set(&mut row, x, 0, class, 0);
            }
            let plain = PlainGrid::of(&row);
            assert_eq!(line_of_sight(&row, a, b), want);
            assert_eq!(trace_cells(&plain, a, b).sight, want);
        }
        // The counters are the trace's own, and a line off the grid is the edge.
        let across = trace_cells(&grid, Eye::standing((0, 0)), Eye::standing((11, 0)));
        assert_eq!(across.visited, 12);
        assert_eq!(across.range_cells, 11);
        assert_eq!(
            trace_cells(&grid, Eye::standing((0, 0)), Eye::standing((30, 0))).sight,
            Sight::BlockedBy(Blocker::Edge)
        );
    }

    /// **16b cut W.2: `trace_cells` reports the target cell's cover.**
    ///
    /// [`Trace::target_cover`] has always been the walk's answer and
    /// [`CellTrace`] simply dropped it, because cut S.1 asked a line only whether it
    /// could be seen along. The combat arena's resolution (doc 16 §3.5 step 1)
    /// scales the presented silhouette by it and holds no
    /// [`SurfaceComposite`], so the two readings have to agree over the same ground.
    #[test]
    fn a_cell_trace_reports_the_same_target_cover_a_full_trace_does() {
        for (class, want) in [
            (TerrainClass::Ground, Cover::None),
            (TerrainClass::Road, Cover::None),
            (TerrainClass::Rubble, Cover::Partial),
            (TerrainClass::WoodsLight, Cover::Partial),
            (TerrainClass::Wall, Cover::Full),
        ] {
            let mut row = surface(6, 1, TerrainClass::Ground);
            set(&mut row, 5, 0, class, 0);
            let a = Eye::standing((0, 0));
            let b = Eye::standing((5, 0));
            let plain = PlainGrid::of(&row);
            assert_eq!(
                trace_cells(&plain, a, b).target_cover,
                want,
                "the {class:?} cell's cover"
            );
            assert_eq!(
                direct_fire_path(&row, &[], a, b).target_cover,
                want,
                "and the full trace agrees"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 16b cut M.4 (16c `B-56`): one line, whichever end it is walked from
    // -----------------------------------------------------------------------

    /// A fixed-seed integer generator for the properties below: splitmix64, so
    /// the pairs are the same on every run and every platform, and no `rand`.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// One coordinate on a 64-cell side, off the generator.
    fn coord(state: &mut u64) -> u16 {
        u16::try_from(next(state) % 64).unwrap_or(0)
    }

    /// A sight with the blocker's identity dropped: which blocker a walk meets
    /// FIRST depends on the end it starts from, and the property is about
    /// whether the line is seen along, and how attenuated, not about which
    /// obstacle is named.
    fn seen(sight: &Sight) -> (u8, u8) {
        match sight {
            Sight::Clear => (0, 0),
            Sight::Partial(pct) => (1, *pct),
            Sight::BlockedBy(_) => (2, 0),
        }
    }

    /// **A 64 by 64 relief with walls, woods and slopes**, every height a whole
    /// metre — the relief the tie needs, since whole-metre ground under
    /// 150 cm eyes is where the old truncation split a line by a centimetre.
    fn relief(seed: u64) -> PlainGrid {
        relief_sized(seed, 64)
    }

    /// **The same relief on a `side` by `side` grid** (16b cut J.1): the slopes'
    /// steps and the ridge's period scaled to the side — at 64 they are the M.4
    /// relief's 8, 16 and 13-with-phase-6 exactly, so `relief` is unchanged —
    /// and the walls, woods and bumps drawn per cell off the same generator.
    fn relief_sized(seed: u64, side: u16) -> PlainGrid {
        let (width, height) = (side, side);
        let x_step = (side / 8).max(1);
        let y_step = (side / 4).max(1);
        let period = u16::try_from((u32::from(side) * 13 / 64).max(2)).unwrap_or(13);
        let phase = u16::try_from(u32::from(side) * 6 / 64).unwrap_or(6) % period;
        let mut state = seed;
        let mut cells = Vec::with_capacity(usize::from(side) * usize::from(side));
        for y in 0..height {
            for x in 0..width {
                // Two slopes and a ridge line, in whole metres.
                let slope = i32::from(x / x_step) + i32::from(y / y_step);
                let ridge = if x % period == phase { 2 } else { 0 };
                let class = match next(&mut state) % 200 {
                    0 => TerrainClass::Wall,
                    1..=4 => TerrainClass::WoodsLight,
                    5 => TerrainClass::WoodsHeavy,
                    _ => TerrainClass::Ground,
                };
                let bump = i32::from(next(&mut state).is_multiple_of(10));
                cells.push((class, (slope + ridge + bump) * 100));
            }
        }
        PlainGrid {
            width,
            height,
            cells,
        }
    }

    /// **`line_cells` is one cell set from either end** (16c `B-56`): for any
    /// two cells, the walk from `b` is the walk from `a` reversed — including
    /// the shallow lines a bare Bresenham breaks its ties on, `(0,0)`–`(2,1)`
    /// among them.
    #[test]
    fn m4_a_line_is_one_cell_set_from_either_end() {
        let mut back = line_cells((2, 1), (0, 0));
        back.reverse();
        assert_eq!(line_cells((0, 0), (2, 1)), back);
        assert_eq!(line_cells((0, 0), (2, 1)), vec![(0, 0), (1, 1), (2, 1)]);
        let mut state = 0x4d34_u64;
        for _ in 0..10_000 {
            let a = (coord(&mut state), coord(&mut state));
            let b = (coord(&mut state), coord(&mut state));
            let forward = line_cells(a, b);
            let mut reverse = line_cells(b, a);
            reverse.reverse();
            assert_eq!(forward, reverse, "{a:?} to {b:?}");
            assert_eq!(forward.first(), Some(&a));
            assert_eq!(forward.last(), Some(&b));
        }
    }

    /// **The line's height is one height from either end** (16c `B-56`): the
    /// symmetric numerator gives step `i` from one end exactly what step
    /// `n − i` gives from the other, for both flights.
    #[test]
    fn m4_a_lines_height_is_one_height_from_either_end() {
        let mut state = 0x4d35_u64;
        for _ in 0..10_000 {
            let n = usize::try_from(next(&mut state) % 90).unwrap_or(0) + 1;
            let i = usize::try_from(next(&mut state) % 91).unwrap_or(0) % (n + 1);
            let z_from = i32::try_from(next(&mut state) % 2_000).unwrap_or(0) - 500;
            let z_to = i32::try_from(next(&mut state) % 2_000).unwrap_or(0) - 500;
            let apex_cm = i32::try_from(next(&mut state) % 3_000).unwrap_or(0);
            for flight in [Flight::Straight, Flight::Arc { apex_cm }] {
                assert_eq!(
                    line_height(z_from, z_to, i, n, flight),
                    line_height(z_to, z_from, n - i, n, flight),
                    "{z_from} to {z_to}, step {i} of {n}, {flight:?}"
                );
            }
        }
    }

    /// **A sees B exactly when B sees A** (16c `B-56`), over a synthetic relief
    /// with walls, woods and whole-metre slopes, for ten thousand fixed-seed
    /// pairs at equal eye heights.
    #[test]
    fn m4_sight_over_a_relief_is_the_same_from_either_end() {
        let grid = relief(0x4d36);
        let mut state = 0x4d37_u64;
        let mut blocked = 0_u32;
        let mut open = 0_u32;
        for _ in 0..10_000 {
            let a = Eye::standing((coord(&mut state), coord(&mut state)));
            let b = Eye::standing((coord(&mut state), coord(&mut state)));
            let there = trace_cells(&grid, a, b);
            let back = trace_cells(&grid, b, a);
            assert_eq!(
                seen(&there.sight),
                seen(&back.sight),
                "{:?} and {:?} disagree: {:?} against {:?}",
                a.cell,
                b.cell,
                there.sight,
                back.sight
            );
            assert_eq!(there.attenuation, back.attenuation);
            assert_eq!(there.visited, back.visited);
            if seen(&there.sight).0 == 2 {
                blocked += 1;
            } else {
                open += 1;
            }
        }
        // The relief is worth testing over: both answers occur, often.
        assert!(
            blocked > 1_000 && open > 1_000,
            "{blocked} blocked, {open} not"
        );
    }

    /// **The hand-built tie** (16c `B-56`): eyes at 150 cm over ground a metre
    /// apart in height, 60 cells end to end, and a ridge at 200 cm in the cell
    /// 29 steps from the low end. Before the cut the line crossed that cell at
    /// 199 cm walked from the low end and at 200 cm walked from the high one,
    /// so the low tank was blocked and the high tank saw over; now it is 199 cm
    /// both ways and the two agree.
    #[test]
    fn m4_the_one_centimetre_tie_is_one_answer_both_ways() {
        // The heights first, as the old and the new arithmetic give them.
        assert_eq!(line_height(150, 250, 29, 59, Flight::Straight), 199);
        assert_eq!(line_height(250, 150, 30, 59, Flight::Straight), 199);
        let old = |z_from: i64, z_to: i64, i: i64, n: i64| z_from + (z_to - z_from) * i / n;
        assert_eq!(
            (old(150, 250, 29, 59), old(250, 150, 30, 59)),
            (199, 200),
            "the truncation the cut removed split this cell by a centimetre"
        );
        let mut row = surface(60, 1, TerrainClass::Ground);
        set(&mut row, 59, 0, TerrainClass::Ground, 100);
        set(&mut row, 29, 0, TerrainClass::Ground, 200);
        let grid = PlainGrid::of(&row);
        let low = Eye {
            cell: (0, 0),
            height_cm: 150,
        };
        let high = Eye {
            cell: (59, 0),
            height_cm: 150,
        };
        let there = trace_cells(&grid, low, high).sight;
        let back = trace_cells(&grid, high, low).sight;
        assert!(
            matches!(
                there,
                Sight::BlockedBy(Blocker::Elevation { at: (29, 0), .. })
            ),
            "{there:?}"
        );
        assert_eq!(there, back, "A sees B exactly when B sees A");
        // And a centimetre lower the ridge is under the line both ways.
        set(&mut row, 29, 0, TerrainClass::Ground, 199);
        let grid = PlainGrid::of(&row);
        assert_eq!(trace_cells(&grid, low, high).sight, Sight::Clear);
        assert_eq!(trace_cells(&grid, high, low).sight, Sight::Clear);
    }

    // -----------------------------------------------------------------------
    // 16b cut J.1: `sight_only` is the canonical walk, stopped at its blocker
    // -----------------------------------------------------------------------

    /// **The three J.1 assertions for one ordered pair**, answering the sight:
    ///
    /// 1. `sight_only` is *exactly* the walk from the canonical ends, each end at
    ///    its own height — blocker and all;
    /// 2. it is seen along exactly as the walk asked from `a` is (16c `B-56`),
    ///    and when either is partial the two attenuations are one;
    /// 3. it reads at least the walk's two endpoints fewer cells (a one-cell
    ///    line: none against the walk's one).
    fn j1_agrees(grid: &dyn CellGrid, a: Eye, b: Eye) -> Sight {
        let (lo, hi) = if canonical_ends(a.cell, b.cell) == (a.cell, b.cell) {
            (a, b)
        } else {
            (b, a)
        };
        let fast = sight_only(grid, a, b);
        let walk = trace_cells(grid, lo, hi);
        assert_eq!(
            fast.sight, walk.sight,
            "sight_only and the canonical walk disagree from {a:?} to {b:?}"
        );
        let asked = trace_cells(grid, a, b);
        assert_eq!(
            seen(&fast.sight),
            seen(&asked.sight),
            "sight_only and the walk asked from {a:?} disagree to {b:?}: {:?} against {:?}",
            fast.sight,
            asked.sight
        );
        if matches!(fast.sight, Sight::Partial(_)) || matches!(asked.sight, Sight::Partial(_)) {
            assert_eq!(walk.attenuation, asked.attenuation, "{a:?} to {b:?}");
            if let Sight::Partial(pct) = fast.sight {
                assert_eq!(u32::from(pct), asked.attenuation, "{a:?} to {b:?}");
            }
        }
        if a.cell == b.cell {
            assert_eq!((fast.visited, walk.visited), (0, 1), "{a:?} to itself");
        } else {
            assert!(
                fast.visited + 2 <= walk.visited,
                "{a:?} to {b:?}: read {} against the walk's {}",
                fast.visited,
                walk.visited
            );
        }
        fast.sight
    }

    /// Clear, partial and blocked, counted.
    fn tally(counts: &mut [u32; 3], sight: &Sight) {
        let slot = usize::from(seen(sight).0);
        if let Some(count) = counts.get_mut(slot) {
            *count += 1;
        }
    }

    /// **`sight_only` is the canonical walk over every pair of a relief** (16b
    /// cut J.1): every ordered pair of a 16 by 16 relief's 256 cells, a cell with
    /// itself included, at 150 cm eyes and again with `a` at 900 cm — 131,072
    /// comparisons — gives the walk's exact sight, is seen along as the walk
    /// asked from `a` is, and reads two cells fewer at least.
    ///
    /// Every arm of [`terrain_sight_rule`] is live over it, and asserted to be:
    /// seed `0x4a39` draws four walls, seven light and three heavy woods cells
    /// on this side, so an opaque class, woods that reach a hundred and a rise
    /// above the line each name the blocker of some pair.
    #[test]
    fn j1_sight_only_is_the_canonical_walk_over_every_pair_of_a_relief() {
        let side = 16_u16;
        let grid = relief_sized(0x4a39, side);
        let opaque_cells = grid
            .cells
            .iter()
            .filter(|(class, _)| props(*class).opacity == Opacity::Opaque)
            .count();
        assert!(opaque_cells > 0, "the relief holds no opaque cell");
        let mut counts = [0_u32; 3];
        // Blocked by an opaque class, by woods that reached a hundred, by a rise.
        let mut arms = [0_u32; 3];
        let mut compared = 0_u32;
        for a_height in [EYE_HEIGHT_CM, 900] {
            for ay in 0..side {
                for ax in 0..side {
                    for by in 0..side {
                        for bx in 0..side {
                            let a = Eye {
                                cell: (ax, ay),
                                height_cm: a_height,
                            };
                            let b = Eye::standing((bx, by));
                            let sight = j1_agrees(&grid, a, b);
                            tally(&mut counts, &sight);
                            let arm = match sight {
                                Sight::BlockedBy(Blocker::Terrain { class, .. })
                                    if props(class).opacity == Opacity::Opaque =>
                                {
                                    Some(0)
                                }
                                Sight::BlockedBy(Blocker::Terrain { .. }) => Some(1),
                                Sight::BlockedBy(Blocker::Elevation { .. }) => Some(2),
                                _ => None,
                            };
                            if let Some(count) = arm.and_then(|slot| arms.get_mut(slot)) {
                                *count += 1;
                            }
                            compared += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(compared, 131_072);
        // The relief is worth testing over: all three answers, often.
        let [clear, partial, blocked] = counts;
        assert!(
            clear > 2_000 && partial > 2_000 && blocked > 2_000,
            "{clear} clear, {partial} partial, {blocked} blocked"
        );
        // And every arm of the per-cell rule named a blocker. Seed `0x4a39`
        // gives 65,210 clear, 16,270 partial and 49,592 blocked, the blocked
        // split 14,971 opaque, 98 saturated woods and 34,523 elevation: woods
        // reach a hundred only across two heavy cells, a heavy and two light, or
        // four light ones on one line (60 and 25 per cent), which is rare on a
        // 16-cell side, so the bar is 50.
        let [opaque, saturated, elevation] = arms;
        assert!(
            opaque > 50 && saturated > 50 && elevation > 50,
            "{opaque} opaque, {saturated} saturated woods, {elevation} elevation"
        );
    }

    /// **And over the M.4 relief from either end** (16b cut J.1): twenty thousand
    /// fixed-seed pairs on the 64 by 64 relief, each asked both ways — `a` at
    /// 900 cm on every fourth pair, so the ends' own heights travel with the
    /// swap — every one of them the canonical walk's answer.
    #[test]
    fn j1_sight_only_over_the_relief_from_either_end() {
        let grid = relief(0x4d36);
        let mut state = 0x4a32_u64;
        let mut blocked = 0_u32;
        let mut open = 0_u32;
        for pair in 0..20_000_u32 {
            let a = Eye {
                cell: (coord(&mut state), coord(&mut state)),
                height_cm: if pair % 4 == 0 { 900 } else { EYE_HEIGHT_CM },
            };
            let b = Eye::standing((coord(&mut state), coord(&mut state)));
            for sight in [j1_agrees(&grid, a, b), j1_agrees(&grid, b, a)] {
                if seen(&sight).0 == 2 {
                    blocked += 1;
                } else {
                    open += 1;
                }
            }
        }
        assert!(
            blocked > 1_000 && open > 1_000,
            "{blocked} blocked, {open} not"
        );
    }

    /// **`sight_only` stops at the first blocker and counts what it read** (16b
    /// cut J.1), on the S.1 rows and the M.4 tie. Every count is the interior
    /// cells read up to and including the blocking one (or every interior cell
    /// when nothing blocks); the endpoints are never read as cells.
    #[test]
    fn j1_sight_only_stops_at_the_first_blocker_and_counts_what_it_read() {
        let a = Eye::standing((0, 0));
        let b = Eye::standing((9, 0));
        // (a) A wall at 5: cells 1 to 5 read, the wall the fifth.
        let mut row = surface(10, 1, TerrainClass::Paved);
        set(&mut row, 5, 0, TerrainClass::Wall, 0);
        let got = sight_only(&PlainGrid::of(&row), a, b);
        assert_eq!(
            got,
            CellSight {
                sight: Sight::BlockedBy(Blocker::Terrain {
                    at: (5, 0),
                    class: TerrainClass::Wall
                }),
                visited: 5,
            }
        );
        // (b) Light woods at 3 and 4: nothing blocks, all eight interior cells read.
        let mut row = surface(10, 1, TerrainClass::Paved);
        set(&mut row, 3, 0, TerrainClass::WoodsLight, 0);
        set(&mut row, 4, 0, TerrainClass::WoodsLight, 0);
        let got = sight_only(&PlainGrid::of(&row), a, b);
        assert_eq!(
            got,
            CellSight {
                sight: Sight::Partial(50),
                visited: 8,
            }
        );
        // (c) Light woods at 3 to 6: the fourth belt reaches a hundred at cell 6,
        // cells 1 to 6 read.
        set(&mut row, 5, 0, TerrainClass::WoodsLight, 0);
        set(&mut row, 6, 0, TerrainClass::WoodsLight, 0);
        let got = sight_only(&PlainGrid::of(&row), a, b);
        assert_eq!(
            got,
            CellSight {
                sight: Sight::BlockedBy(Blocker::Terrain {
                    at: (6, 0),
                    class: TerrainClass::WoodsLight
                }),
                visited: 6,
            }
        );
        // (d) The M.4 tie: drawn from (0,0) whichever end asks, so the ridge at
        // 29 is the 29th interior cell read both ways; a centimetre lower it is
        // under the line and all 58 interior cells are read.
        let mut row = surface(60, 1, TerrainClass::Ground);
        set(&mut row, 59, 0, TerrainClass::Ground, 100);
        set(&mut row, 29, 0, TerrainClass::Ground, 200);
        let grid = PlainGrid::of(&row);
        let low = Eye::standing((0, 0));
        let high = Eye::standing((59, 0));
        let ridge = CellSight {
            sight: Sight::BlockedBy(Blocker::Elevation {
                at: (29, 0),
                elev_cm: 200,
            }),
            visited: 29,
        };
        assert_eq!(sight_only(&grid, low, high), ridge);
        assert_eq!(sight_only(&grid, high, low), ridge);
        set(&mut row, 29, 0, TerrainClass::Ground, 199);
        let grid = PlainGrid::of(&row);
        let open = CellSight {
            sight: Sight::Clear,
            visited: 58,
        };
        assert_eq!(sight_only(&grid, low, high), open);
        assert_eq!(sight_only(&grid, high, low), open);
        // (e) Off the 12-wide S.1 grid: cells 1 to 11 read and clear, cell 12 off
        // the grid read and counted — twelve.
        let mut s = surface(12, 12, TerrainClass::Ground);
        set(&mut s, 6, 6, TerrainClass::Wall, 0);
        set(&mut s, 3, 8, TerrainClass::Ground, 120);
        set(&mut s, 4, 4, TerrainClass::WoodsLight, 0);
        set(&mut s, 5, 5, TerrainClass::WoodsHeavy, 0);
        set(&mut s, 8, 2, TerrainClass::WoodsLight, 0);
        set(&mut s, 9, 1, TerrainClass::WoodsLight, 0);
        let grid = PlainGrid::of(&s);
        assert_eq!(
            sight_only(&grid, Eye::standing((0, 0)), Eye::standing((30, 0))),
            CellSight {
                sight: Sight::BlockedBy(Blocker::Edge),
                visited: 12,
            }
        );
        // (f) An endpoint off the grid: (12, 0) is one cell past the edge. Every
        // interior cell, 1 to 11, is read and clear, the far endpoint is never
        // read as a cell, and the edge is the answer after the loop — both ways,
        // because both ways draw from (0, 0). And the walk agrees: from (0, 0) it
        // sets the edge at its last step, from (12, 0) at its first.
        let near = Eye::standing((0, 0));
        let off = Eye::standing((12, 0));
        let edge = CellSight {
            sight: Sight::BlockedBy(Blocker::Edge),
            visited: 11,
        };
        assert_eq!(sight_only(&grid, near, off), edge);
        assert_eq!(sight_only(&grid, off, near), edge);
        assert_eq!(trace_cells(&grid, near, off).sight, edge.sight);
        assert_eq!(trace_cells(&grid, off, near).sight, edge.sight);
        // And the lesser end off the grid is the edge at once, nothing read.
        assert_eq!(
            sight_only(&grid, Eye::standing((12, 3)), Eye::standing((20, 3))),
            CellSight {
                sight: Sight::BlockedBy(Blocker::Edge),
                visited: 0,
            }
        );
    }

    /// **The lazy line is the collected one** (16b cut J.1): `line_cells` is now
    /// `Bresenham` collected from the canonical ends, and the iterator stops for
    /// good after its far end.
    #[test]
    fn j1_the_bresenham_iterator_is_the_line_and_ends() {
        let mut line = Bresenham::new((0, 0), (2, 1));
        assert_eq!(line.next(), Some((0, 0)));
        assert_eq!(line.next(), Some((1, 1)));
        assert_eq!(line.next(), Some((2, 1)));
        assert_eq!(line.next(), None);
        assert_eq!(line.next(), None);
        assert_eq!(
            Bresenham::new((5, 5), (5, 5)).collect::<Vec<_>>(),
            vec![(5, 5)]
        );
        assert_eq!(canonical_ends((3, 1), (0, 1)), ((0, 1), (3, 1)));
        assert_eq!(canonical_ends((0, 1), (3, 1)), ((0, 1), (3, 1)));
        let mut state = 0x4a33_u64;
        for _ in 0..2_000 {
            let a = (coord(&mut state), coord(&mut state));
            let b = (coord(&mut state), coord(&mut state));
            let (lo, hi) = canonical_ends(a, b);
            let drawn: Vec<(u16, u16)> = Bresenham::new(lo, hi).collect();
            let n = usize::from(lo.0.abs_diff(hi.0).max(lo.1.abs_diff(hi.1)));
            assert_eq!(drawn.len(), n + 1, "{a:?} to {b:?}");
            if lo == a {
                assert_eq!(line_cells(a, b), drawn);
            } else {
                let mut back = drawn;
                back.reverse();
                assert_eq!(line_cells(a, b), back);
            }
        }
    }
}
