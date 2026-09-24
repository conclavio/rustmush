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
//! algorithm with an empty feature overlay. There is no second line walk.

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
    if b < a {
        let mut out = bresenham(b, a);
        out.reverse();
        return out;
    }
    bresenham(a, b)
}

/// The bare Bresenham walk from `a` to `b`, both included — the canonical
/// direction [`line_cells`] draws every line in.
fn bresenham(a: (u16, u16), b: (u16, u16)) -> Vec<(u16, u16)> {
    let (mut x0, mut y0) = (i32::from(a.0), i32::from(a.1));
    let (x1, y1) = (i32::from(b.0), i32::from(b.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::with_capacity(usize::try_from(dx.max(-dy)).unwrap_or(0) + 1);
    loop {
        if let (Ok(x), Ok(y)) = (u16::try_from(x0), u16::try_from(y0)) {
            out.push((x, y));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    out
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
/// `4·h·i·(n−i)/n²`, was already symmetric.
fn line_height(z_from: i32, z_to: i32, i: usize, n: usize, flight: Flight) -> i32 {
    let n = n.max(1);
    let (i64i, i64n) = (i64::try_from(i).unwrap_or(0), i64::try_from(n).unwrap_or(1));
    let straight = (i64::from(z_from) * (i64n - i64i) + i64::from(z_to) * i64i) / i64n;
    let extra = match flight {
        Flight::Straight => 0,
        // 4·h·t·(1−t), t = i/n, in integers.
        Flight::Arc { apex_cm } => 4 * i64::from(apex_cm) * i64i * (i64n - i64i) / (i64n * i64n),
    };
    i32::try_from(straight + extra).unwrap_or(i32::MAX)
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
        let mut blocker: Option<Blocker> = None;
        let p = props(read.class);
        let top = read.elev_cm + obstacle_height_cm(read.class);
        match (opacity_stops, p.opacity) {
            (true, Opacity::Opaque) => {
                blocker = Some(Blocker::Terrain {
                    at,
                    class: read.class,
                });
            }
            (true, Opacity::Partial(pct)) => {
                attenuation = attenuation.saturating_add(u32::from(pct));
                if attenuation >= 100 {
                    blocker = Some(Blocker::Terrain {
                        at,
                        class: read.class,
                    });
                }
            }
            // An arc: whatever has a top (a wall, a canopy) blocks when the
            // arc passes below it; opacity is nothing to a shell.
            (false, _) if obstacle_height_cm(read.class) > 0 && top > z => {
                blocker = Some(Blocker::Terrain {
                    at,
                    class: read.class,
                });
            }
            _ => {}
        }
        if read.elev_cm > z && blocker.is_none() {
            blocker = Some(Blocker::Elevation {
                at,
                elev_cm: read.elev_cm,
            });
        }
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
/// is only classes and elevations. It is what the combat arena's perception pass
/// traces with, because the arena holds a cell grid and may hold no window and no
/// records at all (doc 16 §3.1, D-16-1/D-16-3).
///
/// The counters come back with the answer: doc 16 §2 W4 bounds *traces and cell
/// probes per pulse*, and `visited` is that probe count for one trace.
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
    /// The combat arena traces with [`trace_cells`] because it may hold no window
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
        let (width, height) = (64_u16, 64_u16);
        let mut state = seed;
        let mut cells = Vec::with_capacity(64 * 64);
        for y in 0..height {
            for x in 0..width {
                // Two slopes and a ridge line, in whole metres.
                let slope = i32::from(x / 8) + i32::from(y / 16);
                let ridge = if x % 13 == 6 { 2 } else { 0 };
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
}
