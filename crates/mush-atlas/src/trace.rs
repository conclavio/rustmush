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

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::physics::{Cover, Opacity, WALL_HEIGHT_CM, obstacle_height_cm, props};
use crate::record::{FeatureState, RecordKey};
use crate::terrain::TerrainClass;
use crate::window::SurfaceComposite;

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

/// The cells of the straight line from `a` to `b`, both included (Bresenham).
#[must_use]
pub fn line_cells(a: (u16, u16), b: (u16, u16)) -> Vec<(u16, u16)> {
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
fn line_height(z_from: i32, z_to: i32, i: usize, n: usize, flight: Flight) -> i32 {
    let n = n.max(1);
    let (i64i, i64n) = (i64::try_from(i).unwrap_or(0), i64::try_from(n).unwrap_or(1));
    let straight = i64::from(z_from) + (i64::from(z_to) - i64::from(z_from)) * i64i / i64n;
    let extra = match flight {
        Flight::Straight => 0,
        // 4·h·t·(1−t), t = i/n, in integers.
        Flight::Arc { apex_cm } => 4 * i64::from(apex_cm) * i64i * (i64n - i64i) / (i64n * i64n),
    };
    i32::try_from(straight + extra).unwrap_or(i32::MAX)
}

struct Walk<'a> {
    surface: &'a SurfaceComposite,
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
        let cell = self.surface.cells.get(at.0, at.1)?;
        let elev_cm = i32::from(cell.elev_cm);
        let band = line_cm - elev_cm;
        let mut feature_blocking = None;
        let mut features = Vec::new();
        for f in &self.surface.features {
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
            class: cell.class,
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
    surface: &SurfaceComposite,
    units: &[(ObjId, (u16, u16))],
    from: Eye,
    to: Eye,
    flight: Flight,
    opacity_stops: bool,
) -> Trace {
    let cells = line_cells(from.cell, to.cell);
    let n = cells.len().saturating_sub(1);
    let w = Walk { surface, units };
    let z_from = surface
        .cells
        .get(from.cell.0, from.cell.1)
        .map_or(0, |c| i32::from(c.elev_cm))
        + from.height_cm;
    let z_to = surface
        .cells
        .get(to.cell.0, to.cell.1)
        .map_or(0, |c| i32::from(c.elev_cm))
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
    let t = walk(surface, &[], a, b, Flight::Straight, true);
    match t.stop {
        Some((_, b)) => Sight::BlockedBy(b),
        None if t.attenuation > 0 => {
            Sight::Partial(u8::try_from(t.attenuation.min(99)).unwrap_or(99))
        }
        None => Sight::Clear,
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
    walk(surface, units, from, to, Flight::Straight, true)
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
    walk(surface, units, from, level, Flight::Straight, true)
}

/// Hearing from `a` to `b`: distance, walls, sight-blocking features and
/// heavy woods attenuate; a vacuum carries nothing.
#[must_use]
pub fn hearing(surface: &SurfaceComposite, a: Eye, b: Eye) -> Hearing {
    let t = walk(surface, &[], a, b, Flight::Straight, false);
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
}
