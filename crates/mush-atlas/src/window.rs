//! Windows (doc 14 §2.4 and §3.4; plan §3.5): a scene's or a staffer's tactical
//! map over pre-authored tiles. A window is a frame — a region, its surfaces, an
//! extent and one cell size — and a **composite**: every covering tile sampled at
//! that cell size, the background where none covers, the features overlaid with
//! their states, and the room index from the footprints. The composite is a pure
//! function of the tile revisions it pinned, the feature states and the frame
//! (P5): it is computed off the world loop from a snapshot taken on it, re-enters
//! through the atlas's async completion, and is stored whole under
//! `mod:atlas:window:<domain>` so a restart reads it back rather than recomputing
//! it under a pack that has moved on.
//!
//! The simulation owns the domain, the placements and the projection
//! ([`mush_sim::window`]); this module owns the frame, the composite, the
//! resolver (`resolve_room`, doc 14 §3.4) and placement on entry.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::{Ctx, Deferred, DefinitionDigest, Effect, ObjId, World};
use mush_sim::{DomainId, OpenWindow, Pose, WindowOpener, WindowPin, WindowRec, WindowSurface};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::background::background;
use crate::cells::{Cell, Cells};
use crate::geom::{PointCm, RectCm, Shape, cm, metres};
use crate::module::{AtlasState, NS, del_doc, put_doc};
use crate::raster::Grid;
use crate::record::{
    Blocks, FeatureKind, FeatureRec, FeatureState, Geometry, RESOLUTIONS_CM, RecordKey, Records,
    RegionRec, RoomGeo, SurfaceRef, TileRec,
};
use crate::terrain::TerrainClass;

/// The document key prefix: `window:<domain>`.
pub const WINDOW_PREFIX: &str = "window:";

/// The street driver's cell size (doc 14 §5.4).
pub const STREET_CELL_CM: u16 = 100;

/// The cell sizes a window may use before a ruleset registers more, finest first;
/// an oversized window coarsens to the next (D-14-19).
pub const DEFAULT_CELL_SIZES: [u16; 4] = [100, 200, 500, 1000];

/// Plan §7 `window_margin_m`.
pub const DEFAULT_WINDOW_MARGIN_M: u32 = 10;

/// Plan §7 `window_max_cells`.
pub const DEFAULT_WINDOW_MAX_CELLS: u32 = 65_536;

/// The most participants one placement pass places.
pub const MAX_PLACED_PER_PASS: usize = 256;

/// The canonical-digest domain of a composite.
const COMPOSITE_DOMAIN: &str = "mush.atlas.window.composite.v1";

/// The most bytes a composite's canonical form may run to before its digest is
/// refused (eight surfaces of the largest window, encoded).
const MAX_COMPOSITE_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------

/// A window's frame: where it is and how fine it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowFrame {
    /// The region.
    pub region: RecordKey,
    /// The surfaces, the ground first.
    pub surfaces: Vec<SurfaceRef>,
    /// The extent, region centimetres.
    pub bbox: RectCm,
    /// The cell size, centimetres.
    pub cell_cm: u16,
}

impl WindowFrame {
    /// The grid every surface of this window shares.
    #[must_use]
    pub fn grid(&self) -> Option<Grid> {
        Grid::over(self.bbox, self.cell_cm, u32::MAX)
    }

    /// The simulation's record of this frame.
    #[must_use]
    pub fn to_rec(
        &self,
        opened_by: WindowOpener,
        revision: u64,
        pins: &[(RecordKey, u64)],
        features_seen: u64,
    ) -> WindowRec {
        WindowRec {
            region: SmolStr::new(self.region.to_string()),
            surfaces: self.surfaces.iter().map(surface_wire).collect(),
            bbox: mush_sim::WindowBox {
                x0: self.bbox.min.x,
                y0: self.bbox.min.y,
                x1: self.bbox.max.x,
                y1: self.bbox.max.y,
            },
            cell_cm: self.cell_cm,
            composited_from: pins
                .iter()
                .map(|(key, revision)| WindowPin {
                    tile: SmolStr::new(key.to_string()),
                    revision: *revision,
                })
                .collect(),
            features_seen,
            opened_by,
            revision,
        }
    }
}

fn surface_wire(surface: &SurfaceRef) -> WindowSurface {
    match surface {
        SurfaceRef::Ground => WindowSurface::Ground,
        SurfaceRef::Floor { structure, index } => WindowSurface::Floor {
            structure: SmolStr::new(structure.to_string()),
            index: *index,
        },
    }
}

fn intersects(a: &RectCm, b: &RectCm) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

fn clip(a: &RectCm, to: &RectCm) -> Option<RectCm> {
    let min = PointCm {
        x: a.min.x.max(to.min.x),
        y: a.min.y.max(to.min.y),
    };
    let max = PointCm {
        x: a.max.x.min(to.max.x),
        y: a.max.y.min(to.max.y),
    };
    (min.x < max.x && min.y < max.y).then_some(RectCm { min, max })
}

/// Grow `bbox` by `margin_cm` on every side.
#[must_use]
pub fn with_margin(bbox: RectCm, margin_cm: i64) -> RectCm {
    RectCm {
        min: PointCm {
            x: bbox.min.x - margin_cm,
            y: bbox.min.y - margin_cm,
        },
        max: PointCm {
            x: bbox.max.x + margin_cm,
            y: bbox.max.y + margin_cm,
        },
    }
}

/// The finest registered cell size at or above `preferred_cm` at which `bbox`
/// fits `max_cells` (D-14-19: coarsen, never refuse, until the sizes run out).
#[must_use]
pub fn choose_cell(
    bbox: RectCm,
    preferred_cm: u16,
    sizes: &BTreeSet<u16>,
    max_cells: u32,
) -> Option<u16> {
    sizes
        .iter()
        .copied()
        .filter(|s| *s >= preferred_cm)
        .find(|s| Grid::over(bbox, *s, max_cells).is_some())
}

// ---------------------------------------------------------------------------
// The composite: a pure function of its input
// ---------------------------------------------------------------------------

/// What a composite is made from — captured on the world loop, computed off it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeInput {
    /// The frame.
    pub frame: WindowFrame,
    /// The region (its background recipe).
    pub region: RegionRec,
    /// The tiles on the frame's surfaces that touch its extent.
    pub tiles: Vec<TileRec>,
    /// The features on those surfaces that touch it.
    pub features: Vec<FeatureRec>,
    /// The rooms on those surfaces that touch it.
    pub rooms: Vec<RoomGeo>,
}

/// Snapshot what `frame` composites from `records`.
///
/// # Errors
///
/// A message when the region is unknown.
pub fn capture(records: &Records, frame: &WindowFrame) -> Result<CompositeInput, String> {
    let region = records
        .regions
        .get(&frame.region)
        .cloned()
        .ok_or_else(|| format!("no region {}", frame.region))?;
    let on_surface = |s: &SurfaceRef| frame.surfaces.contains(s);
    let mut tiles: Vec<TileRec> = records
        .tiles
        .values()
        .filter(|t| t.region == frame.region && on_surface(&t.surface))
        .filter(|t| t.bbox.to_cm().is_ok_and(|b| intersects(&b, &frame.bbox)))
        .cloned()
        .collect();
    tiles.sort_by(|a, b| a.key.cmp(&b.key));
    let mut features: Vec<FeatureRec> = records
        .features
        .values()
        .filter(|f| f.region == frame.region && on_surface(&f.surface))
        .filter(|f| feature_bbox(f).is_some_and(|b| intersects(&b, &frame.bbox)))
        .cloned()
        .collect();
    features.sort_by(|a, b| a.key.cmp(&b.key));
    let mut rooms: Vec<RoomGeo> = records
        .roomgeo
        .values()
        .filter(|g| on_surface(&g.surface))
        .filter(|g| {
            records
                .neighborhoods
                .get(&g.neighborhood)
                .is_some_and(|n| n.region == frame.region)
        })
        .filter(|g| {
            g.footprint
                .shape()
                .is_ok_and(|s| intersects(&s.bbox(), &frame.bbox))
        })
        .cloned()
        .collect();
    rooms.sort_by_key(|g| g.room);
    Ok(CompositeInput {
        frame: frame.clone(),
        region,
        tiles,
        features,
        rooms,
    })
}

fn feature_bbox(f: &FeatureRec) -> Option<RectCm> {
    match &f.geometry {
        Geometry::Polygon(poly) => poly.to_cm().ok().map(|p| p.bbox()),
        Geometry::Polyline(points) => {
            let pts: Option<Vec<PointCm>> = points.iter().map(|p| p.to_cm().ok()).collect();
            let pts = pts?;
            let first = *pts.first()?;
            let mut min = first;
            let mut max = first;
            for p in &pts {
                min.x = min.x.min(p.x);
                min.y = min.y.min(p.y);
                max.x = max.x.max(p.x);
                max.y = max.y.max(p.y);
            }
            Some(RectCm {
                min,
                max: PointCm {
                    x: max.x + 1,
                    y: max.y + 1,
                },
            })
        }
        Geometry::Point(p) => {
            let p = p.to_cm().ok()?;
            Some(RectCm {
                min: p,
                max: PointCm {
                    x: p.x + 1,
                    y: p.y + 1,
                },
            })
        }
    }
}

/// A feature as it lies on a composite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureOverlay {
    /// The feature.
    pub key: RecordKey,
    /// What it is.
    pub kind: FeatureKind,
    /// Its state when the composite was made.
    pub state: FeatureState,
    /// What it blocks.
    pub blocks: Blocks,
    /// The cells it covers.
    pub cells: Vec<(u16, u16)>,
    /// Its height band above the surface, centimetres.
    pub z_range_cm: (i32, i32),
}

/// One surface of a composite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceComposite {
    /// The surface.
    pub surface: SurfaceRef,
    /// The grid.
    pub grid: Grid,
    /// The cells: class, elevation, cover, room index.
    pub cells: Cells,
    /// The room table `cell.room - 1` indexes.
    pub rooms: Vec<ObjId>,
    /// The features overlaid.
    pub features: Vec<FeatureOverlay>,
}

impl SurfaceComposite {
    /// The room at a cell.
    #[must_use]
    pub fn room_at(&self, x: u16, y: u16) -> Option<ObjId> {
        let cell = self.cells.get(x, y)?;
        let i = usize::from(cell.room).checked_sub(1)?;
        self.rooms.get(i).copied()
    }

    /// The cell a point falls in.
    #[must_use]
    pub fn cell_of(&self, p: PointCm) -> Option<(u16, u16)> {
        self.grid.cell_of(p)
    }
}

/// A composite: every surface of a frame, sampled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composite {
    /// The frame.
    pub frame: WindowFrame,
    /// The surfaces, in the frame's order.
    pub surfaces: Vec<SurfaceComposite>,
    /// The tile revisions read.
    pub pins: Vec<(RecordKey, u64)>,
    /// The highest feature revision read.
    pub features_seen: u64,
    /// The canonical digest of the surfaces: P5's witness.
    pub digest: String,
}

impl Composite {
    /// The surface composite for `surface`.
    #[must_use]
    pub fn surface(&self, surface: &SurfaceRef) -> Option<&SurfaceComposite> {
        self.surfaces.iter().find(|s| &s.surface == surface)
    }

    /// Every room with at least one cell on any surface.
    #[must_use]
    pub fn rooms(&self) -> BTreeSet<ObjId> {
        self.surfaces
            .iter()
            .flat_map(|s| s.rooms.iter().copied())
            .collect()
    }
}

struct TileView<'a> {
    tile: &'a TileRec,
    grid: Grid,
}

fn tile_views<'a>(tiles: &'a [TileRec], surface: &SurfaceRef) -> Vec<TileView<'a>> {
    let mut views: Vec<TileView<'_>> = tiles
        .iter()
        .filter(|t| &t.surface == surface)
        .filter_map(|t| {
            let bbox = t.bbox.to_cm().ok()?;
            Some(TileView {
                tile: t,
                grid: Grid {
                    origin: bbox.min,
                    resolution_cm: t.resolution_cm,
                    width: t.cells.width(),
                    height: t.cells.height(),
                },
            })
        })
        .collect();
    // Finest first, then by key: the neighborhood's fine ground over the region's
    // coarse ground where both cover a point.
    views.sort_by(|a, b| {
        a.tile
            .resolution_cm
            .cmp(&b.tile.resolution_cm)
            .then_with(|| a.tile.key.cmp(&b.tile.key))
    });
    views
}

/// Sample one window cell from the finest tile covering its centre: a finer tile
/// averages its elevation and majority-classes over the window cell, a coarser
/// one is read at the centre.
fn sample(views: &[TileView<'_>], grid: &Grid, x: u16, y: u16) -> Option<Cell> {
    let centre = grid.centre(x, y);
    let res = i64::from(grid.resolution_cm);
    let square = RectCm {
        min: PointCm {
            x: grid.origin.x + i64::from(x) * res,
            y: grid.origin.y + i64::from(y) * res,
        },
        max: PointCm {
            x: grid.origin.x + (i64::from(x) + 1) * res,
            y: grid.origin.y + (i64::from(y) + 1) * res,
        },
    };
    for view in views {
        let Some((tx, ty)) = view.grid.cell_of(centre) else {
            continue;
        };
        if view.tile.resolution_cm >= grid.resolution_cm {
            return view.tile.cells.get(tx, ty);
        }
        // Finer: every tile cell whose centre lies in the window cell's square.
        let tres = i64::from(view.tile.resolution_cm);
        let first = view.grid.cell_of(PointCm {
            x: square.min.x + tres / 2,
            y: square.min.y + tres / 2,
        });
        let last = view.grid.cell_of(PointCm {
            x: square.max.x - 1 - (square.max.x - 1 - square.min.x) % tres + tres / 2,
            y: square.max.y - 1 - (square.max.y - 1 - square.min.y) % tres + tres / 2,
        });
        let (Some((x0, y0)), Some((x1, y1))) = (first, last) else {
            return view.tile.cells.get(tx, ty);
        };
        let mut classes: BTreeMap<u8, u32> = BTreeMap::new();
        let mut elev: i64 = 0;
        let mut cover: u8 = 0;
        let mut n: i64 = 0;
        for cy in y0.min(y1)..=y0.max(y1) {
            for cx in x0.min(x1)..=x0.max(x1) {
                let Some(c) = view.tile.cells.get(cx, cy) else {
                    continue;
                };
                *classes.entry(c.class.code()).or_insert(0) += 1;
                elev += i64::from(c.elev_cm);
                cover = cover.max(c.cover);
                n += 1;
            }
        }
        if n == 0 {
            return view.tile.cells.get(tx, ty);
        }
        // The majority class; a tie goes to the lower code (void loses to any
        // ground, paved to grass), which is stable across platforms.
        let class = classes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .and_then(|(code, _)| TerrainClass::from_code(*code))
            .unwrap_or_default();
        let mean = elev / n;
        return Some(Cell {
            class,
            elev_cm: i16::try_from(mean).unwrap_or(i16::MAX),
            cover,
            room: 0,
        });
    }
    None
}

fn feature_cells(grid: &Grid, f: &FeatureRec) -> Vec<(u16, u16)> {
    match &f.geometry {
        Geometry::Polygon(poly) => poly
            .to_cm()
            .map(|p| grid.cells_in(&Shape::Polygon(p)))
            .unwrap_or_default(),
        Geometry::Polyline(points) => {
            let pts: Option<Vec<PointCm>> = points.iter().map(|p| p.to_cm().ok()).collect();
            pts.map(|p| grid.cells_along(&p)).unwrap_or_default()
        }
        Geometry::Point(p) => p
            .to_cm()
            .ok()
            .and_then(|p| grid.cell_of(p))
            .into_iter()
            .collect(),
    }
}

/// What a feature's state does to the cells under it: a destroyed wall, rail,
/// counter, tree or platform is rubble; a destroyed door or gate is an open
/// doorway; damage changes nothing about the ground.
fn state_class(kind: FeatureKind, state: FeatureState) -> Option<TerrainClass> {
    if state != FeatureState::Destroyed {
        return None;
    }
    match kind {
        FeatureKind::Wall
        | FeatureKind::Rail
        | FeatureKind::Counter
        | FeatureKind::Tree
        | FeatureKind::Platform => Some(TerrainClass::Rubble),
        FeatureKind::Door | FeatureKind::Gate => Some(TerrainClass::Door),
        FeatureKind::Window | FeatureKind::Crater | FeatureKind::Other => None,
    }
}

/// Composite `input`. Pure: the same input gives the same bytes on every
/// platform and every run (P5).
///
/// # Errors
///
/// A message when the frame makes no grid, or the composite will not encode.
pub fn composite(input: &CompositeInput) -> Result<Composite, String> {
    let grid = input
        .frame
        .grid()
        .ok_or_else(|| "the frame makes no grid".to_string())?;
    let mut surfaces = Vec::with_capacity(input.frame.surfaces.len());
    for surface in &input.frame.surfaces {
        let views = tile_views(&input.tiles, surface);
        let mut cells = Cells::filled(
            grid.width,
            grid.height,
            Cell::default(),
            crate::cells::MAX_TILE_CELLS,
        )
        .map_err(|e| e.to_string())?;
        for y in 0..grid.height {
            for x in 0..grid.width {
                // Off every tile: the ground has its background (P4); a floor
                // ends at its building and is void beyond it.
                let cell = sample(&views, &grid, x, y).unwrap_or_else(|| match surface {
                    SurfaceRef::Ground => {
                        let c = grid.centre(x, y);
                        let ground = background(&input.region, surface, c.x, c.y);
                        Cell {
                            class: ground.class,
                            elev_cm: i16::try_from(ground.elev_cm).unwrap_or(i16::MAX),
                            cover: 0,
                            room: 0,
                        }
                    }
                    SurfaceRef::Floor { .. } => Cell::default(),
                });
                cells.set(x, y, Cell { room: 0, ..cell });
            }
        }
        // Rooms: the footprints, in room order; only rooms with a cell are listed.
        let mut rooms: Vec<ObjId> = Vec::new();
        for geo in input.rooms.iter().filter(|g| &g.surface == surface) {
            let Ok(shape) = geo.footprint.shape() else {
                continue;
            };
            let covered = grid.cells_in(&shape);
            if covered.is_empty() {
                continue;
            }
            rooms.push(geo.room);
            let index = u16::try_from(rooms.len()).map_err(|_| "too many rooms".to_string())?;
            for (x, y) in covered {
                cells.update(x, y, |c| {
                    if c.room == 0 {
                        c.room = index;
                    }
                });
            }
        }
        // Features, with their states.
        let mut features = Vec::new();
        for f in input.features.iter().filter(|f| &f.surface == surface) {
            let covered = feature_cells(&grid, f);
            if covered.is_empty() {
                continue;
            }
            if let Some(class) = state_class(f.kind, f.state) {
                for &(x, y) in &covered {
                    cells.update(x, y, |c| c.class = class);
                }
            }
            let z0 = cm(f.z_range.0)
                .ok()
                .and_then(|z| i32::try_from(z).ok())
                .unwrap_or(0);
            let z1 = cm(f.z_range.1)
                .ok()
                .and_then(|z| i32::try_from(z).ok())
                .unwrap_or(z0);
            features.push(FeatureOverlay {
                key: f.key.clone(),
                kind: f.kind,
                state: f.state,
                blocks: f.blocks,
                cells: covered,
                z_range_cm: (z0, z1),
            });
        }
        surfaces.push(SurfaceComposite {
            surface: surface.clone(),
            grid,
            cells,
            rooms,
            features,
        });
    }
    let mut pins: Vec<(RecordKey, u64)> = input
        .tiles
        .iter()
        .map(|t| (t.key.clone(), t.revision))
        .collect();
    pins.sort();
    pins.dedup();
    let features_seen = input.features.iter().map(|f| f.revision).max().unwrap_or(0);
    let value = serde_json::to_value(&surfaces).map_err(|e| e.to_string())?;
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        COMPOSITE_DOMAIN,
        &value,
        MAX_COMPOSITE_BYTES,
    )
    .map_err(|e| e.to_string())?
    .to_hex();
    Ok(Composite {
        frame: input.frame.clone(),
        surfaces,
        pins,
        features_seen,
        digest,
    })
}

// ---------------------------------------------------------------------------
// The off-loop job and its outcome
// ---------------------------------------------------------------------------

/// The payload of the atlas's `Effect::ModuleAsync`: composite this.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeJob {
    /// The window's domain.
    pub domain: u64,
    /// The window revision this composite is for; a stale one is dropped.
    pub revision: u64,
    /// The input.
    pub input: CompositeInput,
}

/// What comes back.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeOutcome {
    /// The window's domain.
    pub domain: u64,
    /// The revision it was for.
    pub revision: u64,
    /// The composite, or why not.
    pub result: Result<Composite, String>,
}

/// Run one composite job off the world loop: the payload in, the outcome out,
/// both as the strings the async seam carries. Never panics; a payload that will
/// not decode comes back as a failed outcome for domain zero.
#[must_use]
pub fn run_async(payload: &str) -> String {
    let outcome = match serde_json::from_str::<CompositeJob>(payload) {
        Ok(job) => CompositeOutcome {
            domain: job.domain,
            revision: job.revision,
            result: composite(&job.input),
        },
        Err(error) => CompositeOutcome {
            domain: 0,
            revision: 0,
            result: Err(format!("bad composite job: {error}")),
        },
    };
    serde_json::to_string(&outcome).unwrap_or_else(|error| {
        format!(
            "{{\"domain\":0,\"revision\":0,\"result\":{{\"Err\":\"the outcome would not encode: {error}\"}}}}"
        )
    })
}

// ---------------------------------------------------------------------------
// The open windows
// ---------------------------------------------------------------------------

/// What the composite, once it lands, must do.
#[derive(Clone, Debug, Default)]
pub struct Pending {
    /// The revision requested.
    pub revision: u64,
    /// Who to place when it lands, and through which portal each arrived.
    pub place: Vec<(ObjId, Option<RecordKey>)>,
}

/// One open window: the document plus the RAM-only pending work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowEntry {
    /// The domain.
    pub domain: u64,
    /// The frame.
    pub frame: WindowFrame,
    /// The anchor rooms it was opened over.
    pub anchors: Vec<ObjId>,
    /// The rooms it covers (with the composite: the rooms with a cell; before it:
    /// the rooms whose footprints touch the frame).
    pub rooms: BTreeSet<ObjId>,
    /// Who opened it.
    pub opened_by: WindowOpener,
    /// The composite revision installed (zero before the first).
    pub revision: u64,
    /// The composite, once it landed.
    pub composite: Option<Composite>,
    /// Work waiting on a composite.
    #[serde(skip)]
    pub pending: Option<Pending>,
}

impl WindowEntry {
    /// The document key.
    #[must_use]
    pub fn key(&self) -> SmolStr {
        window_key(self.domain)
    }
}

/// The document key of a window.
#[must_use]
pub fn window_key(domain: u64) -> SmolStr {
    SmolStr::new(format!("{WINDOW_PREFIX}{domain}"))
}

/// Whether `key` is a window document.
#[must_use]
pub fn is_window_key(key: &str) -> bool {
    key.starts_with(WINDOW_PREFIX)
}

/// The windows the atlas keeps.
#[derive(Clone, Debug)]
pub struct Windows {
    /// Open windows by domain.
    pub open: BTreeMap<u64, WindowEntry>,
    /// The surface (an index into the frame's) each viewer looks at, when they
    /// chose one (`+map/floor`); RAM-only.
    pub viewer_floor: BTreeMap<ObjId, usize>,
    /// The registered cell sizes (doc 14 §3.4), finest first.
    pub cell_sizes: BTreeSet<u16>,
    /// The margin around the anchors, centimetres.
    pub margin_cm: i64,
    /// The cell cap.
    pub max_cells: u32,
}

impl Default for Windows {
    fn default() -> Windows {
        Windows {
            open: BTreeMap::new(),
            viewer_floor: BTreeMap::new(),
            cell_sizes: DEFAULT_CELL_SIZES.iter().copied().collect(),
            margin_cm: i64::from(DEFAULT_WINDOW_MARGIN_M) * 100,
            max_cells: DEFAULT_WINDOW_MAX_CELLS,
        }
    }
}

impl Windows {
    /// The window covering `room`, when one does.
    #[must_use]
    pub fn covering(&self, room: ObjId) -> Option<&WindowEntry> {
        self.open.values().find(|e| e.rooms.contains(&room))
    }
}

/// Register a cell size a ruleset's engagements use (doc 14 §5.3).
///
/// # Errors
///
/// A message when the size is not one of [`RESOLUTIONS_CM`] or the atlas is not
/// composed.
pub fn register_cell_size(w: &mut World, cell_cm: u16) -> Result<(), String> {
    if !RESOLUTIONS_CM.contains(&cell_cm) {
        return Err(format!(
            "a cell size is one of {RESOLUTIONS_CM:?} centimetres, not {cell_cm}"
        ));
    }
    let st = w
        .module_state_mut::<AtlasState>()
        .ok_or_else(|| "the atlas module is not composed".to_string())?;
    st.windows.cell_sizes.insert(cell_cm);
    Ok(())
}

/// Read every window document back; a window the simulation no longer has is
/// dropped, and a simulation window with no composite here is reported so the
/// boot closes it (the process died between its open and its composite).
#[must_use]
pub fn hydrate(
    kv: &[(SmolStr, serde_json::Value)],
    w: &World,
) -> (BTreeMap<u64, WindowEntry>, Vec<SmolStr>, Vec<DomainId>) {
    let mut open = BTreeMap::new();
    let mut dropped = Vec::new();
    for (key, value) in kv.iter().filter(|(k, _)| is_window_key(k)) {
        match serde_json::from_value::<WindowEntry>(value.clone()) {
            Ok(entry)
                if mush_sim::window_of(w, DomainId(entry.domain)).is_some()
                    && entry.composite.is_some() =>
            {
                open.insert(entry.domain, entry);
            }
            Ok(entry) => {
                tracing::warn!(
                    %key,
                    domain = entry.domain,
                    "atlas: a window document without a live simulation window or a composite was dropped at boot"
                );
                dropped.push(key.clone());
            }
            Err(error) => {
                tracing::warn!(%key, %error, "atlas: a window document was dropped at boot");
                dropped.push(key.clone());
            }
        }
    }
    let orphans: Vec<DomainId> = mush_sim::windows(w)
        .map(|(id, _)| id)
        .filter(|id| !open.contains_key(&id.0))
        .collect();
    (open, dropped, orphans)
}

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a AtlasState, String> {
    ctx.state::<AtlasState>()
        .ok_or_else(|| "the atlas module is not composed".to_string())
}

fn state_mut<'a>(ctx: &'a mut Ctx<'_>) -> Result<&'a mut AtlasState, String> {
    ctx.state_mut::<AtlasState>()
        .ok_or_else(|| "the atlas module is not composed".to_string())
}

fn persist_entry(ctx: &mut Ctx<'_>, entry: &WindowEntry) {
    put_doc(ctx, entry.key(), entry);
}

fn name(w: &World, obj: ObjId) -> String {
    mush_core::output::display_name(w, obj)
}

/// What opening a window asks for.
#[derive(Clone, Debug)]
pub struct OpenRequest {
    /// The anchor rooms; the first hosts the domain and is the presentation room.
    pub anchors: Vec<ObjId>,
    /// An explicit extent, region centimetres, instead of the anchors' footprints
    /// plus the margin (staff).
    pub bbox: Option<RectCm>,
    /// Who is opening it.
    pub opened_by: WindowOpener,
    /// The cell size wanted; the street driver's when absent.
    pub cell_cm: Option<u16>,
    /// Who to place once the composite lands.
    pub participants: Vec<ObjId>,
}

/// Open a window (doc 14 §3.4 *Creation*): the extent is the anchors' footprints
/// plus the margin (or the extent given), the surfaces the anchors' plus the
/// ground, the cell size the finest registered one that fits the cap. The
/// composite is deferred off-loop; the participants are placed when it lands.
///
/// # Errors
///
/// A message when an anchor has no geometry, the anchors span regions, a covered
/// room is on another window's map, no cell size fits, or the simulation refuses.
pub fn open(ctx: &mut Ctx<'_>, req: OpenRequest) -> Result<DomainId, String> {
    let (frame, rooms) = {
        let st = state(ctx)?;
        frame_for(st, &req)?
    };
    let host = *req
        .anchors
        .first()
        .ok_or_else(|| "a window needs an anchor room".to_string())?;
    let rec = frame.to_rec(req.opened_by.clone(), 1, &[], 0);
    let domain = mush_sim::open_window(
        ctx,
        OpenWindow {
            host,
            presentation: host,
            rec,
        },
    )?;
    let input = {
        let st = state(ctx)?;
        capture(&st.records, &frame)?
    };
    let entry = WindowEntry {
        domain: domain.0,
        frame,
        anchors: req.anchors.clone(),
        rooms,
        opened_by: req.opened_by,
        revision: 0,
        composite: None,
        pending: Some(Pending {
            revision: 1,
            place: req.participants.iter().map(|o| (*o, None)).collect(),
        }),
    };
    persist_entry(ctx, &entry);
    state_mut(ctx)?.windows.open.insert(domain.0, entry);
    defer_composite(ctx, domain, 1, input);
    Ok(domain)
}

/// The frame and the covered rooms for `req`, against the records as they stand.
fn frame_for(st: &AtlasState, req: &OpenRequest) -> Result<(WindowFrame, BTreeSet<ObjId>), String> {
    let records = &st.records;
    let mut region: Option<RecordKey> = None;
    let mut surfaces: Vec<SurfaceRef> = vec![SurfaceRef::Ground];
    let mut extent: Option<RectCm> = None;
    for &room in &req.anchors {
        let geo = records
            .roomgeo
            .get(&room)
            .ok_or_else(|| format!("{room} has no geometry on the atlas"))?;
        let nb = records
            .neighborhoods
            .get(&geo.neighborhood)
            .ok_or_else(|| format!("{room}'s neighborhood is gone"))?;
        match &region {
            None => region = Some(nb.region.clone()),
            Some(r) if *r != nb.region => {
                return Err("a window lies in one region".to_string());
            }
            Some(_) => {}
        }
        if !surfaces.contains(&geo.surface) {
            surfaces.push(geo.surface.clone());
        }
        let bbox = geo.footprint.shape().map_err(|e| e.to_string())?.bbox();
        extent = Some(match extent {
            None => bbox,
            Some(e) => e.union(&bbox),
        });
    }
    let region_key = region.ok_or_else(|| "a window needs an anchor room".to_string())?;
    let region = records
        .regions
        .get(&region_key)
        .ok_or_else(|| format!("no region {region_key}"))?;
    let region_extent = region.extent.to_cm().map_err(|e| e.to_string())?;
    let wanted = match req.bbox {
        Some(b) => b,
        None => with_margin(
            extent.ok_or_else(|| "a window needs an anchor room".to_string())?,
            st.windows.margin_cm,
        ),
    };
    let bbox = clip(&wanted, &region_extent)
        .ok_or_else(|| "the window lies outside its region".to_string())?;
    let preferred = req.cell_cm.unwrap_or(STREET_CELL_CM);
    let cell_cm = choose_cell(
        bbox,
        preferred,
        &st.windows.cell_sizes,
        st.windows.max_cells,
    )
    .ok_or_else(|| {
        format!(
            "no registered cell size fits {} by {} metres under {} cells",
            metres(bbox.width()),
            metres(bbox.height()),
            st.windows.max_cells
        )
    })?;
    let frame = WindowFrame {
        region: region_key.clone(),
        surfaces,
        bbox,
        cell_cm,
    };
    // Every room on those surfaces whose footprint touches the extent is on this
    // map; a room already on another's is refused.
    let mut rooms = BTreeSet::new();
    for geo in records.roomgeo.values() {
        if !frame.surfaces.contains(&geo.surface) {
            continue;
        }
        if records
            .neighborhoods
            .get(&geo.neighborhood)
            .is_none_or(|n| n.region != region_key)
        {
            continue;
        }
        let Ok(shape) = geo.footprint.shape() else {
            continue;
        };
        if !intersects(&shape.bbox(), &bbox) {
            continue;
        }
        if let Some(other) = st.windows.covering(geo.room) {
            return Err(format!(
                "{} is on window #{}'s map already",
                geo.room, other.domain
            ));
        }
        rooms.insert(geo.room);
    }
    Ok((frame, rooms))
}

fn defer_composite(ctx: &mut Ctx<'_>, domain: DomainId, revision: u64, input: CompositeInput) {
    let job = CompositeJob {
        domain: domain.0,
        revision,
        input,
    };
    let payload = match serde_json::to_string(&job) {
        Ok(p) => p,
        Err(error) => {
            tracing::error!(%domain, %error, "atlas: a composite job would not encode");
            return;
        }
    };
    let correlation = ctx.w.next_correlation_id();
    let actor = ctx.actor;
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor,
        correlation,
        payload,
    }));
}

/// Re-composite an open window against the records as they stand (growth, a
/// feature's state, a pack that moved on); the participants keep their poses.
///
/// # Errors
///
/// A message when the window is not open.
pub fn recomposite(ctx: &mut Ctx<'_>, domain: DomainId) -> Result<u64, String> {
    let (frame, revision) = {
        let st = state_mut(ctx)?;
        let entry = st
            .windows
            .open
            .get_mut(&domain.0)
            .ok_or_else(|| format!("window #{domain} is not open"))?;
        let revision = entry
            .pending
            .as_ref()
            .map_or(entry.revision, |p| p.revision)
            .saturating_add(1);
        let pending = entry.pending.get_or_insert_with(Pending::default);
        pending.revision = revision;
        (entry.frame.clone(), revision)
    };
    let input = {
        let st = state(ctx)?;
        capture(&st.records, &frame)?
    };
    defer_composite(ctx, domain, revision, input);
    Ok(revision)
}

/// The async completion: install a composite, place who waited on it.
pub fn on_composited(ctx: &mut Ctx<'_>, outcome: CompositeOutcome) {
    let domain = DomainId(outcome.domain);
    let (opened_by, waiting) = {
        let Ok(st) = state_mut(ctx) else {
            return;
        };
        let Some(entry) = st.windows.open.get_mut(&outcome.domain) else {
            tracing::debug!(%domain, "atlas: a composite landed for a window that closed");
            return;
        };
        let expected = entry.pending.as_ref().map(|p| p.revision);
        if expected != Some(outcome.revision) {
            tracing::debug!(
                %domain,
                revision = outcome.revision,
                expected,
                "atlas: a stale composite was dropped"
            );
            return;
        }
        let pending = entry.pending.take().unwrap_or_default();
        match outcome.result {
            Ok(composite) => {
                entry.rooms = composite.rooms();
                entry.revision = outcome.revision;
                entry.composite = Some(composite);
                (entry.opened_by.clone(), Ok(pending.place))
            }
            Err(error) => (entry.opened_by.clone(), Err(error)),
        }
    };
    match waiting {
        Ok(place) => {
            let (rec, entry) = {
                let Ok(st) = state(ctx) else {
                    return;
                };
                let Some(entry) = st.windows.open.get(&outcome.domain) else {
                    return;
                };
                let Some(composite) = &entry.composite else {
                    return;
                };
                (
                    entry.frame.to_rec(
                        opened_by.clone(),
                        entry.revision,
                        &composite.pins,
                        composite.features_seen,
                    ),
                    entry.clone(),
                )
            };
            if let Err(error) = mush_sim::update_window(ctx, domain, rec) {
                tracing::warn!(%domain, %error, "atlas: the simulation refused the window's frame");
            }
            persist_entry(ctx, &entry);
            let mut placed = 0usize;
            for (obj, portal) in place.into_iter().take(MAX_PLACED_PER_PASS) {
                let Some(room) = ctx.w.get(obj).and_then(|o| o.location) else {
                    continue;
                };
                if !entry.rooms.contains(&room) {
                    continue;
                }
                match place_in_room(ctx, domain, obj, room, portal.as_ref()) {
                    Ok(()) => placed += 1,
                    Err(error) => {
                        if ctx.w.get(obj).is_some() {
                            ctx.out
                                .info(obj, format!("You are not on the map: {error}"));
                        }
                    }
                }
            }
            announce_ready(ctx, &entry, &opened_by, placed);
        }
        Err(error) => {
            tracing::warn!(%domain, %error, "atlas: a composite failed; the window closes");
            if let WindowOpener::Staff { by } = opened_by
                && ctx.w.get(by).is_some()
            {
                ctx.out.error(
                    by,
                    format!("The map of window #{domain} could not be drawn: {error}"),
                );
            }
            if let Err(error) = close(ctx, domain, "its map could not be drawn") {
                tracing::warn!(%domain, %error, "atlas: closing a failed window");
            }
        }
    }
}

fn announce_ready(ctx: &mut Ctx<'_>, entry: &WindowEntry, opened_by: &WindowOpener, placed: usize) {
    let Some(composite) = &entry.composite else {
        return;
    };
    let grid = composite.frame.grid();
    let line = format!(
        "The map is drawn: window #{} — {} by {} cells at {} cm over {} room(s); {placed} placed.",
        entry.domain,
        grid.map_or(0, |g| g.width),
        grid.map_or(0, |g| g.height),
        entry.frame.cell_cm,
        entry.rooms.len()
    );
    let mut told = BTreeSet::new();
    for (obj, _) in mush_sim::participants(ctx.w, DomainId(entry.domain)) {
        if told.insert(obj) {
            ctx.out.info(obj, line.clone());
        }
    }
    if let WindowOpener::Staff { by } = opened_by
        && told.insert(*by)
        && ctx.w.get(*by).is_some()
    {
        ctx.out.info(*by, line);
    }
}

/// Close a window: release everyone where they stand, retire the domain, drop
/// the document.
///
/// # Errors
///
/// A message when the window is not open or the simulation refuses.
pub fn close(ctx: &mut Ctx<'_>, domain: DomainId, reason: &str) -> Result<Vec<ObjId>, String> {
    let entry = {
        let st = state_mut(ctx)?;
        st.windows
            .open
            .remove(&domain.0)
            .ok_or_else(|| format!("window #{domain} is not open"))?
    };
    let released = match mush_sim::close_window(ctx, domain) {
        Ok(released) => released,
        Err(error) => {
            // Put the entry back: the world and the atlas must not disagree.
            if let Ok(st) = state_mut(ctx) {
                st.windows.open.insert(domain.0, entry);
            }
            return Err(error);
        }
    };
    del_doc(ctx, entry.key());
    for &obj in &released {
        if ctx.w.get(obj).is_some() {
            ctx.out.info(
                obj,
                format!("The map closes ({reason}); you are in {}.", {
                    let here = ctx.w.get(obj).and_then(|o| o.location);
                    here.map_or_else(|| "nowhere".to_string(), |r| name(ctx.w, r))
                }),
            );
        }
    }
    Ok(released)
}

// ---------------------------------------------------------------------------
// Placement and the resolver (doc 14 §3.4)
// ---------------------------------------------------------------------------

/// Whether a participant may stand on a cell.
#[must_use]
pub fn standable(cell: Cell) -> bool {
    !matches!(
        cell.class,
        TerrainClass::Void | TerrainClass::Wall | TerrainClass::Water
    )
}

fn occupied(w: &World, domain: DomainId, surface: &SurfaceComposite) -> BTreeSet<(u16, u16)> {
    mush_sim::participants(w, domain)
        .into_iter()
        .filter_map(|(_, pose)| {
            let p = pose_cm(&pose)?;
            surface.cell_of(p)
        })
        .collect()
}

fn pose_cm(pose: &Pose) -> Option<PointCm> {
    Some(PointCm {
        x: cm(pose.x).ok()?,
        y: cm(pose.y).ok()?,
    })
}

/// The pose at the centre of a cell, on the cell's ground (P2).
#[must_use]
pub fn pose_at(surface: &SurfaceComposite, x: u16, y: u16) -> Option<Pose> {
    let cell = surface.cells.get(x, y)?;
    let c = surface.grid.centre(x, y);
    let posture = mush_sim::passive_rules_value(serde_json::json!({})).ok()?;
    Some(Pose::at(
        metres(c.x),
        metres(c.y),
        f64::from(cell.elev_cm) / 100.0,
        posture,
    ))
}

/// The free cell of `room` a participant enters at: adjacent to the portal it
/// arrived through when one is named, else nearest the footprint's centre; ties
/// by row then column.
#[must_use]
pub fn entry_cell(
    w: &World,
    domain: DomainId,
    composite: &Composite,
    records: &Records,
    room: ObjId,
    portal: Option<&RecordKey>,
) -> Option<(usize, u16, u16)> {
    let geo = records.roomgeo.get(&room)?;
    let (si, surface) = composite
        .surfaces
        .iter()
        .enumerate()
        .find(|(_, s)| s.surface == geo.surface)?;
    let index = u16::try_from(surface.rooms.iter().position(|r| *r == room)? + 1).ok()?;
    let taken = occupied(w, domain, surface);
    let free: Vec<(u16, u16)> = surface
        .cells
        .iter()
        .filter(|(x, y, c)| c.room == index && standable(*c) && !taken.contains(&(*x, *y)))
        .map(|(x, y, _)| (x, y))
        .collect();
    if free.is_empty() {
        return None;
    }
    // The doorway's own cell centre is the target, so the entry cell lines up
    // with the door rather than beside it.
    let target: PointCm = portal
        .and_then(|k| records.portals.get(k))
        .and_then(|p| match &p.geometry {
            crate::record::PortalGeometry::Segment(a, b) => {
                let a = a.xy().to_cm().ok()?;
                let b = b.xy().to_cm().ok()?;
                Some(crate::geom::midpoint(a, b))
            }
            crate::record::PortalGeometry::Point(p) => p.xy().to_cm().ok(),
        })
        .map(|p| {
            surface
                .grid
                .cell_of(p)
                .map_or(p, |(x, y)| surface.grid.centre(x, y))
        })
        .unwrap_or_else(|| {
            geo.footprint
                .shape()
                .map_or(surface.grid.centre(0, 0), |s| s.centre())
        });
    let best = free.iter().copied().min_by_key(|&(x, y)| {
        let c = surface.grid.centre(x, y);
        let dx = c.x - target.x;
        let dy = c.y - target.y;
        (dx * dx + dy * dy, y, x)
    })?;
    Some((si, best.0, best.1))
}

/// Place `obj` in `room` on `domain`'s map (doc 14 §3.4 *Placement*).
///
/// # Errors
///
/// A message when the room is not on the map, is full, or the simulation
/// refuses.
pub fn place_in_room(
    ctx: &mut Ctx<'_>,
    domain: DomainId,
    obj: ObjId,
    room: ObjId,
    portal: Option<&RecordKey>,
) -> Result<(), String> {
    let pose = {
        let st = state(ctx)?;
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .ok_or_else(|| format!("window #{domain} is not open"))?;
        let composite = entry
            .composite
            .as_ref()
            .ok_or_else(|| "the map is not drawn yet".to_string())?;
        let (si, x, y) = entry_cell(ctx.w, domain, composite, &st.records, room, portal)
            .ok_or_else(|| format!("{} has no free cell", name(ctx.w, room)))?;
        let surface = composite
            .surfaces
            .get(si)
            .ok_or_else(|| "no such surface".to_string())?;
        pose_at(surface, x, y).ok_or_else(|| "no such cell".to_string())?
    };
    mush_sim::place(ctx, obj, domain, pose)
}

/// The room a pose is in (doc 14 §3.4 *Leaving*; §5.2 `resolve_room`): the room
/// whose footprint holds it on its surface, else the nearest footprint within
/// `nearest_room_cells` (ties by id), else none — the simulation then uses the
/// domain's presentation room.
#[must_use]
pub fn resolve_room(w: &World, domain: DomainId, pose: &Pose) -> Option<ObjId> {
    let st = w.module_state::<AtlasState>()?;
    let entry = st.windows.open.get(&domain.0)?;
    let composite = entry.composite.as_ref()?;
    let p = pose_cm(pose)?;
    let z_cm = cm(pose.z).ok()?;
    // The surface: the one whose ground at the pose is nearest its height.
    let surface = composite
        .surfaces
        .iter()
        .filter_map(|s| {
            let (x, y) = s.cell_of(p)?;
            let cell = s.cells.get(x, y)?;
            (cell.class != TerrainClass::Void).then_some((
                s,
                x,
                y,
                (i64::from(cell.elev_cm) - z_cm).abs(),
            ))
        })
        .min_by_key(|(_, _, _, dz)| *dz);
    let (surface, x, y, _) = surface?;
    if let Some(room) = surface.room_at(x, y) {
        return Some(room);
    }
    let reach = st.limits.nearest_room_cells;
    let mut best: Option<(u32, ObjId)> = None;
    for (cx, cy, cell) in surface.cells.iter() {
        if cell.room == 0 {
            continue;
        }
        let d = u32::from(cx.abs_diff(x)).max(u32::from(cy.abs_diff(y)));
        if d > reach {
            continue;
        }
        let Some(room) = surface.room_at(cx, cy) else {
            continue;
        };
        if best.is_none_or(|(bd, br)| d < bd || (d == bd && room < br)) {
            best = Some((d, room));
        }
    }
    best.map(|(_, room)| room)
}

/// Register the resolver with the simulation. Once, at the atlas's boot.
///
/// # Errors
///
/// The simulation's refusal.
pub fn register_resolver(w: &mut World) -> Result<(), String> {
    mush_sim::register_room_resolver(w, Arc::new(resolve_room))
}

/// A one-line summary of an open window.
#[must_use]
pub fn summary(w: &World, entry: &WindowEntry) -> String {
    let grid = entry.frame.grid();
    let who = match &entry.opened_by {
        WindowOpener::Scene { system, id } => format!("{system} scene {id}"),
        WindowOpener::Staff { by } => name(w, *by),
    };
    let anchors: Vec<String> = entry.anchors.iter().map(|r| name(w, *r)).collect();
    format!(
        "#{} {}x{} at {} cm, {} room(s), revision {}{}; opened by {who} over {}",
        entry.domain,
        grid.map_or(0, |g| g.width),
        grid.map_or(0, |g| g.height),
        entry.frame.cell_cm,
        entry.rooms.len(),
        entry.revision,
        if entry.composite.is_some() {
            ""
        } else {
            " (drawing)"
        },
        anchors.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Scenes: a window per engagement (plan §3.5), and the exits (doc 14 §3.4)
// ---------------------------------------------------------------------------

/// The window a scene opened, when one is open.
#[must_use]
pub fn window_for_scene(st: &AtlasState, system: &str, id: u64) -> Option<u64> {
    st.windows
        .open
        .values()
        .find(|e| {
            matches!(&e.opened_by, WindowOpener::Scene { system: s, id: i } if s.as_str() == system && *i == id)
        })
        .map(|e| e.domain)
}

/// Open a scene's window over its anchor room, its members placed when the
/// composite lands.
///
/// # Errors
///
/// The open's refusal, or that the scene has a window already.
pub fn open_for_scene(
    ctx: &mut Ctx<'_>,
    system: &str,
    id: u64,
    anchor: ObjId,
    members: Vec<ObjId>,
) -> Result<DomainId, String> {
    if let Some(domain) = window_for_scene(state(ctx)?, system, id) {
        return Err(format!("the scene has window #{domain} already"));
    }
    open(
        ctx,
        OpenRequest {
            anchors: vec![anchor],
            bbox: None,
            opened_by: WindowOpener::Scene {
                system: SmolStr::new(system),
                id,
            },
            cell_cm: Some(STREET_CELL_CM),
            participants: members,
        },
    )
}

/// Close a scene's window, if it has one. `Ok(None)` when it has none.
///
/// # Errors
///
/// The close's refusal.
pub fn close_for_scene(
    ctx: &mut Ctx<'_>,
    system: &str,
    id: u64,
) -> Result<Option<Vec<ObjId>>, String> {
    let Some(domain) = window_for_scene(state(ctx)?, system, id) else {
        return Ok(None);
    };
    close(ctx, DomainId(domain), "the scene ended").map(Some)
}

/// The scenes module's start hook: a timestop opens a window over its room.
pub fn scene_started(ctx: &mut Ctx<'_>, rec: &mush_scenes::SceneRecord) {
    if rec.kind.as_str() != mush_scenes::KIND_TIMESTOP {
        return;
    }
    let members: Vec<ObjId> = rec.members().collect();
    match open_for_scene(ctx, "scenes", rec.id.0, rec.anchor, members) {
        Ok(domain) => {
            if ctx.w.get(rec.facilitator).is_some() {
                ctx.out.info(
                    rec.facilitator,
                    format!(
                        "Window #{domain} opens over the scene; its map is being drawn (+map)."
                    ),
                );
            }
        }
        Err(error) => {
            tracing::debug!(scene = rec.id.0, %error, "atlas: no window for the scene");
        }
    }
}

/// The scenes module's end hook: the window closes with the scene.
pub fn scene_ended(ctx: &mut Ctx<'_>, rec: &mush_scenes::SceneRecord) {
    if let Err(error) = close_for_scene(ctx, "scenes", rec.id.0) {
        tracing::warn!(scene = rec.id.0, %error, "atlas: closing the scene's window");
    }
}

/// Register the scene hooks with the scenes module, when it is composed.
///
/// # Errors
///
/// The scenes module's refusal.
pub fn register_scene_hooks(w: &mut World) -> Result<(), String> {
    mush_scenes::register_scene_hooks(
        w,
        Some(Arc::new(scene_started)),
        Some(Arc::new(scene_ended)),
    )
}

/// The portal between two rooms, when the atlas knows one.
#[must_use]
pub fn portal_between(records: &Records, from: Option<ObjId>, to: ObjId) -> Option<RecordKey> {
    let from = from?;
    records
        .portals
        .values()
        .find(|p| {
            matches!(
                (&p.from, &p.to),
                (crate::record::Endpoint::Room(a), crate::record::Endpoint::Room(b))
                    if (*a == from && *b == to) || (*a == to && *b == from)
            )
        })
        .map(|p| p.key.clone())
}

/// The `Moved` observer (doc 14 §3.4 *Placement*): a character who walks into
/// a room on a map is placed on a free cell by the portal they came through; one
/// who walks off a map is released where they stand; one who walks between two
/// rooms of one map is re-posed.
pub fn on_moved(ctx: &mut Ctx<'_>, ev: &mush_core::WorldEvent) {
    let mush_core::WorldEvent::Moved { who, from, to } = ev else {
        return;
    };
    let (who, from, to) = (*who, *from, *to);
    if !ctx
        .w
        .get(who)
        .is_some_and(|o| matches!(o.kind, mush_core::ObjKind::Player | mush_core::ObjKind::Npc))
    {
        return;
    }
    let placed = mush_sim::pose_of(ctx.w, who).map(|(d, _)| d);
    let covering = state(ctx)
        .ok()
        .and_then(|st| st.windows.covering(to))
        .filter(|e| e.composite.is_some())
        .map(|e| DomainId(e.domain));
    if let Some(d) = placed
        && covering != Some(d)
        && let Err(error) = mush_sim::unplace(ctx, who, "left the map")
    {
        tracing::warn!(%who, %error, "atlas: releasing a character who walked off a map");
        return;
    }
    let Some(domain) = covering else {
        return;
    };
    let portal = state(ctx)
        .ok()
        .and_then(|st| portal_between(&st.records, from, to));
    if let Err(error) = place_in_room(ctx, domain, who, to, portal.as_ref()) {
        ctx.out
            .info(who, format!("You are off the map here: {error}."));
    }
}

// ---------------------------------------------------------------------------
// Steps and growth
// ---------------------------------------------------------------------------

/// A compass step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compass {
    /// Cells east per step.
    pub dx: i32,
    /// Cells north per step.
    pub dy: i32,
}

impl Compass {
    /// Parse `n`, `ne`, `east`, … or a cell `x,y`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Compass> {
        let (dx, dy) = match word.trim().to_ascii_lowercase().as_str() {
            "n" | "north" => (0, 1),
            "s" | "south" => (0, -1),
            "e" | "east" => (1, 0),
            "w" | "west" => (-1, 0),
            "ne" | "northeast" => (1, 1),
            "nw" | "northwest" => (-1, 1),
            "se" | "southeast" => (1, -1),
            "sw" | "southwest" => (-1, -1),
            _ => return None,
        };
        Some(Compass { dx, dy })
    }

    /// The bearing, degrees clockwise from north.
    #[must_use]
    pub fn facing_deg(self) -> u16 {
        match (self.dx.signum(), self.dy.signum()) {
            (0, 1) => 0,
            (1, 1) => 45,
            (1, 0) => 90,
            (1, -1) => 135,
            (0, -1) => 180,
            (-1, -1) => 225,
            (-1, 0) => 270,
            (-1, 1) => 315,
            _ => 0,
        }
    }

    /// The compass word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match (self.dx.signum(), self.dy.signum()) {
            (0, 1) => "north",
            (1, 1) => "northeast",
            (1, 0) => "east",
            (1, -1) => "southeast",
            (0, -1) => "south",
            (-1, -1) => "southwest",
            (-1, 0) => "west",
            (-1, 1) => "northwest",
            _ => "nowhere",
        }
    }
}

/// Where a step wants to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepTarget {
    /// `n` cells along a heading.
    Along(Compass, u16),
    /// Toward a cell, at most `MAX_STEP_CELLS` steps.
    Toward(u16, u16),
}

/// The most cells one `+move` walks.
pub const MAX_STEP_CELLS: u16 = 64;

/// What a walk did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReport {
    /// Cells walked.
    pub walked: u16,
    /// Where the walk ended.
    pub at: (u16, u16),
    /// The room there, when the cell is on a footprint.
    pub room: Option<ObjId>,
    /// Why it stopped short, if it did.
    pub stopped: Option<String>,
}

/// The surface, cell and elevation gap a pose stands at.
fn locate(composite: &Composite, pose: &Pose) -> Option<(usize, u16, u16)> {
    let p = pose_cm(pose)?;
    let z_cm = cm(pose.z).ok()?;
    composite
        .surfaces
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let (x, y) = s.cell_of(p)?;
            let cell = s.cells.get(x, y)?;
            (cell.class != TerrainClass::Void).then_some((
                i,
                x,
                y,
                (i64::from(cell.elev_cm) - z_cm).abs(),
            ))
        })
        .min_by_key(|(_, _, _, dz)| *dz)
        .map(|(i, x, y, _)| (i, x, y))
}

/// Whether a feature on the surface blocks movement into a cell. A doorway cut
/// through a wall is the wall's own gap: the wall's feature does not block it.
fn blocked_by_feature(
    surface: &SurfaceComposite,
    x: u16,
    y: u16,
    class: TerrainClass,
) -> Option<&RecordKey> {
    surface
        .features
        .iter()
        .find(|f| {
            f.blocks.movement
                && f.state != FeatureState::Destroyed
                && f.cells.contains(&(x, y))
                && !(matches!(
                    f.kind,
                    FeatureKind::Wall | FeatureKind::Rail | FeatureKind::Gate
                ) && class == TerrainClass::Door)
        })
        .map(|f| &f.key)
}

/// Walk `actor` across their window (plan §3.5 `+move`, cost one per cell until
/// S5's table): each cell must be ground someone can stand on, free, not behind
/// a feature that blocks movement, and — crossing into another room's footprint
/// — a room the walker may enter (its access; staff anywhere). A step off the
/// map's edge onto a room's footprint grows the map (doc 14 §3.4 *Growth*) and
/// stops the walk until the new composite lands.
///
/// # Errors
///
/// A message when the actor is not on a map, or the simulation refuses the pose.
pub fn step(ctx: &mut Ctx<'_>, actor: ObjId, target: StepTarget) -> Result<StepReport, String> {
    let (domain, pose) =
        mush_sim::pose_of(ctx.w, actor).ok_or_else(|| "you are not on a map".to_string())?;
    let staff = crate::module::is_staff(ctx.w, actor);
    let (walk, heading, end_surface, grow_at) = {
        let st = state(ctx)?;
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .ok_or_else(|| "your map is closed".to_string())?;
        let composite = entry
            .composite
            .as_ref()
            .ok_or_else(|| "the map is not drawn yet".to_string())?;
        let (mut si, mut x, mut y) =
            locate(composite, &pose).ok_or_else(|| "you are off the map".to_string())?;
        // Everyone else's cells, on every surface.
        let taken: BTreeSet<(usize, u16, u16)> = mush_sim::participants(ctx.w, domain)
            .into_iter()
            .filter(|(o, _)| *o != actor)
            .filter_map(|(_, p)| locate(composite, &p))
            .collect();
        let surface_at = |i: usize| composite.surfaces.get(i);
        let mut current_room = surface_at(si).and_then(|s| s.room_at(x, y));
        let (heading, budget) = match target {
            StepTarget::Along(h, n) => (h, n.clamp(1, MAX_STEP_CELLS)),
            StepTarget::Toward(tx, ty) => (
                Compass {
                    dx: i32::from(tx) - i32::from(x),
                    dy: i32::from(ty) - i32::from(y),
                },
                MAX_STEP_CELLS,
            ),
        };
        let mut walked = 0u16;
        let mut stopped = None;
        let mut grow_at: Option<PointCm> = None;
        for _ in 0..budget {
            let Some(surface) = surface_at(si) else {
                stopped = Some("no such surface".to_string());
                break;
            };
            let (dx, dy) = match target {
                StepTarget::Along(h, _) => (h.dx.signum(), h.dy.signum()),
                StepTarget::Toward(tx, ty) => {
                    let (dx, dy) = (
                        (i32::from(tx) - i32::from(x)).signum(),
                        (i32::from(ty) - i32::from(y)).signum(),
                    );
                    if dx == 0 && dy == 0 {
                        break;
                    }
                    (dx, dy)
                }
            };
            let nx = i32::from(x) + dx;
            let ny = i32::from(y) + dy;
            let inside = nx >= 0
                && ny >= 0
                && nx < i32::from(surface.grid.width)
                && ny < i32::from(surface.grid.height);
            if !inside {
                let res = i64::from(surface.grid.resolution_cm);
                grow_at = Some(PointCm {
                    x: surface.grid.origin.x + i64::from(nx) * res + res / 2,
                    y: surface.grid.origin.y + i64::from(ny) * res + res / 2,
                });
                stopped = Some("the map ends here".to_string());
                break;
            }
            let (Ok(cx), Ok(cy)) = (u16::try_from(nx), u16::try_from(ny)) else {
                stopped = Some("the map ends here".to_string());
                break;
            };
            // The cell ahead on this surface; when it is not ground to stand
            // on, another surface's cell there at the same height is (a
            // doorway out of a building onto the lot, a floor onto its
            // stair's landing): the walk crosses onto it.
            let here_elev = surface.cells.get(x, y).map_or(0, |c| i32::from(c.elev_cm));
            let mut next = surface.cells.get(cx, cy).map(|c| (si, c));
            if !next.is_some_and(|(_, c)| standable(c)) {
                next = composite
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != si)
                    .filter_map(|(j, s)| s.cells.get(cx, cy).map(|c| (j, c)))
                    .find(|(_, c)| standable(*c) && (i32::from(c.elev_cm) - here_elev).abs() <= 150)
                    .or(next);
            }
            let Some((ni, cell)) = next else {
                stopped = Some("the map ends here".to_string());
                break;
            };
            if !standable(cell) {
                stopped = Some(format!("{} is in the way", cell.class.label()));
                break;
            }
            if taken.contains(&(ni, cx, cy)) {
                stopped = Some("someone is standing there".to_string());
                break;
            }
            let Some(next_surface) = surface_at(ni) else {
                stopped = Some("no such surface".to_string());
                break;
            };
            if let Some(key) = blocked_by_feature(next_surface, cx, cy, cell.class) {
                stopped = Some(format!("{} is in the way", key.entry));
                break;
            }
            let there = next_surface.room_at(cx, cy);
            if there != current_room
                && let Some(room) = there
                && !staff
                && st
                    .records
                    .roomgeo
                    .get(&room)
                    .is_some_and(|g| g.access == crate::record::Access::Restricted)
            {
                stopped = Some(format!("{} is not open to you", name(ctx.w, room)));
                break;
            }
            si = ni;
            x = cx;
            y = cy;
            current_room = there;
            walked += 1;
        }
        (
            StepReport {
                walked,
                at: (x, y),
                room: current_room,
                stopped,
            },
            heading,
            si,
            grow_at,
        )
    };
    if walk.walked > 0 {
        let pose = {
            let st = state(ctx)?;
            let composite = st
                .windows
                .open
                .get(&domain.0)
                .and_then(|e| e.composite.as_ref())
                .ok_or_else(|| "your map closed".to_string())?;
            let surface = composite
                .surfaces
                .get(end_surface)
                .ok_or_else(|| "no such surface".to_string())?;
            let mut new_pose =
                pose_at(surface, walk.at.0, walk.at.1).ok_or_else(|| "no such cell".to_string())?;
            new_pose.facing_deg = heading.facing_deg();
            new_pose
        };
        mush_sim::place(ctx, actor, domain, pose)?;
    }
    if let Some(beyond) = grow_at {
        let surface_ref = {
            let st = state(ctx)?;
            st.windows
                .open
                .get(&domain.0)
                .and_then(|e| e.frame.surfaces.get(end_surface).cloned())
        };
        if let Some(surface_ref) = surface_ref {
            let mut report = walk;
            match grow(ctx, domain, beyond, &surface_ref) {
                Ok(Some(room)) => {
                    report.stopped = Some(format!(
                        "the map grows to take in {}; step again when it is drawn",
                        name(ctx.w, room)
                    ));
                }
                Ok(None) => {}
                Err(error) => report.stopped = Some(error),
            }
            return Ok(report);
        }
    }
    Ok(walk)
}

/// Grow `domain`'s window to take in the room whose footprint holds `beyond` on
/// `surface` (doc 14 §3.4 *Growth*): the frame widens by that footprint plus
/// the margin, the cell size is chosen again under the cap, and a re-composite
/// is deferred. `Ok(None)` when no footprint lies there.
///
/// # Errors
///
/// A message when the room is on another window's map, or no cell size fits.
pub fn grow(
    ctx: &mut Ctx<'_>,
    domain: DomainId,
    beyond: PointCm,
    surface: &SurfaceRef,
) -> Result<Option<ObjId>, String> {
    let (room, new_frame, rooms) = {
        let st = state(ctx)?;
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .ok_or_else(|| format!("window #{domain} is not open"))?;
        let region = st
            .records
            .regions
            .get(&entry.frame.region)
            .ok_or_else(|| format!("no region {}", entry.frame.region))?;
        let Some(geo) = st
            .records
            .roomgeo
            .values()
            .filter(|g| &g.surface == surface)
            .filter(|g| {
                st.records
                    .neighborhoods
                    .get(&g.neighborhood)
                    .is_some_and(|n| n.region == entry.frame.region)
            })
            .find(|g| g.footprint.shape().is_ok_and(|s| s.contains(beyond)))
        else {
            return Ok(None);
        };
        if let Some(other) = st.windows.covering(geo.room)
            && other.domain != entry.domain
        {
            return Err(format!(
                "the map cannot grow into {}: it is on window #{}'s map",
                name(ctx.w, geo.room),
                other.domain
            ));
        }
        let shape = geo.footprint.shape().map_err(|e| e.to_string())?;
        let wanted = entry
            .frame
            .bbox
            .union(&with_margin(shape.bbox(), st.windows.margin_cm));
        let region_extent = region.extent.to_cm().map_err(|e| e.to_string())?;
        let bbox = clip(&wanted, &region_extent)
            .ok_or_else(|| "the window lies outside its region".to_string())?;
        let cell_cm = choose_cell(
            bbox,
            entry.frame.cell_cm.min(STREET_CELL_CM),
            &st.windows.cell_sizes,
            st.windows.max_cells,
        )
        .ok_or_else(|| "the map cannot grow: no registered cell size fits".to_string())?;
        let frame = WindowFrame {
            region: entry.frame.region.clone(),
            surfaces: entry.frame.surfaces.clone(),
            bbox,
            cell_cm,
        };
        let mut rooms = entry.rooms.clone();
        for g in st.records.roomgeo.values() {
            if !frame.surfaces.contains(&g.surface) {
                continue;
            }
            if st
                .records
                .neighborhoods
                .get(&g.neighborhood)
                .is_none_or(|n| n.region != frame.region)
            {
                continue;
            }
            let Ok(s) = g.footprint.shape() else {
                continue;
            };
            if !intersects(&s.bbox(), &bbox) {
                continue;
            }
            if let Some(other) = st.windows.covering(g.room)
                && other.domain != entry.domain
            {
                return Err(format!(
                    "the map cannot grow into {}: it is on window #{}'s map",
                    name(ctx.w, g.room),
                    other.domain
                ));
            }
            rooms.insert(g.room);
        }
        (geo.room, frame, rooms)
    };
    {
        let st = state_mut(ctx)?;
        if let Some(entry) = st.windows.open.get_mut(&domain.0) {
            entry.frame = new_frame;
            entry.rooms = rooms;
        }
    }
    let entry = state(ctx)?
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .ok_or_else(|| format!("window #{domain} is not open"))?;
    persist_entry(ctx, &entry);
    recomposite(ctx, domain)?;
    Ok(Some(room))
}

// ---------------------------------------------------------------------------
// Write-back (doc 14 §3.4)
// ---------------------------------------------------------------------------

/// Set a feature's state — the wreckage a fight leaves — and re-composite every
/// window over it. Returns those windows.
///
/// # Errors
///
/// A message when the feature is unknown.
pub fn set_feature_state(
    ctx: &mut Ctx<'_>,
    key: &RecordKey,
    new_state: FeatureState,
) -> Result<Vec<DomainId>, String> {
    let (rec, windows) = {
        let st = state_mut(ctx)?;
        let f = st
            .records
            .features
            .get_mut(key)
            .ok_or_else(|| format!("no feature {key}"))?;
        f.state = new_state;
        f.revision = f.revision.saturating_add(1);
        let rec = f.clone();
        let bbox = feature_bbox(&rec);
        let windows: Vec<DomainId> = st
            .windows
            .open
            .values()
            .filter(|e| {
                e.frame.region == rec.region
                    && e.frame.surfaces.contains(&rec.surface)
                    && bbox.is_some_and(|b| intersects(&b, &e.frame.bbox))
            })
            .map(|e| DomainId(e.domain))
            .collect();
        (rec, windows)
    };
    put_doc(
        ctx,
        crate::record::record_key(crate::record::KIND_FEATURE, key),
        &rec,
    );
    for &domain in &windows {
        recomposite(ctx, domain)?;
    }
    Ok(windows)
}

/// Parse `intact`, `damaged[:<n>]` or `destroyed`.
#[must_use]
pub fn parse_feature_state(text: &str) -> Option<FeatureState> {
    let t = text.trim().to_ascii_lowercase();
    match t.as_str() {
        "intact" | "whole" => Some(FeatureState::Intact),
        "destroyed" | "gone" => Some(FeatureState::Destroyed),
        "damaged" => Some(FeatureState::Damaged(128)),
        _ => t
            .strip_prefix("damaged:")
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| *n > 0)
            .map(FeatureState::Damaged),
    }
}
