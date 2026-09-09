//! The rasterizer (doc 14 §3.2): pure functions from records to cells, portals and
//! tiles. A pack need not draw — a site rectangle is a footprint, a room rectangle is
//! cells on its surface, an exit row is a portal on the shared edge, a structure's
//! ring is a wall on every floor — and the result is legal, coarse and playable.
//!
//! Every function here is pure over its arguments: nothing reads the world, nothing
//! writes, and the same records rasterize to the same bytes on every platform (the
//! centimetre discipline in `geom`).

use std::collections::BTreeMap;

use mush_core::ObjId;

use crate::background::background;
use crate::cells::{Cell, Cells, MAX_TILE_DOC_BYTES, MAX_TILE_SIDE};
use crate::geom::{PointCm, PolygonCm, RectCm, Shape, cm, midpoint};
use crate::record::{
    FeatureKind, FeatureRec, Geometry, PortalMode, PortalRec, RESOLUTIONS_CM, RegionRec, RoomGeo,
    RoomKind, StructureRec, SurfaceRef,
};
use crate::terrain::TerrainClass;

/// A cell grid's placement: where cell `(0, 0)` is and how big a cell is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grid {
    /// The low corner, centimetres.
    pub origin: PointCm,
    /// Cell size, centimetres.
    pub resolution_cm: u16,
    /// Width in cells.
    pub width: u16,
    /// Height in cells.
    pub height: u16,
}

impl Grid {
    /// The grid over `bbox` at `resolution_cm`, or `None` when it would need more than
    /// `max_cells` cells or more than [`MAX_TILE_SIDE`] a side.
    #[must_use]
    pub fn over(bbox: RectCm, resolution_cm: u16, max_cells: u32) -> Option<Grid> {
        let res = i64::from(resolution_cm.max(1));
        let width = ceil_div(bbox.width(), res);
        let height = ceil_div(bbox.height(), res);
        let width = u16::try_from(width).ok()?;
        let height = u16::try_from(height).ok()?;
        if width == 0 || height == 0 || width > MAX_TILE_SIDE || height > MAX_TILE_SIDE {
            return None;
        }
        if u32::from(width) * u32::from(height) > max_cells {
            return None;
        }
        Some(Grid {
            origin: bbox.min,
            resolution_cm,
            width,
            height,
        })
    }

    /// The grid over `bbox` at the finest resolution from `preferred` upward that fits
    /// `max_cells`.
    #[must_use]
    pub fn fitting(bbox: RectCm, preferred_cm: u16, max_cells: u32) -> Option<Grid> {
        RESOLUTIONS_CM
            .iter()
            .copied()
            .filter(|r| *r >= preferred_cm)
            .find_map(|r| Grid::over(bbox, r, max_cells))
    }

    /// The centre of cell `(x, y)`.
    #[must_use]
    pub fn centre(&self, x: u16, y: u16) -> PointCm {
        let res = i64::from(self.resolution_cm);
        PointCm {
            x: self.origin.x + i64::from(x) * res + res / 2,
            y: self.origin.y + i64::from(y) * res + res / 2,
        }
    }

    /// The cell holding point `p`, if inside the grid.
    #[must_use]
    pub fn cell_of(&self, p: PointCm) -> Option<(u16, u16)> {
        let res = i64::from(self.resolution_cm);
        let x = (p.x - self.origin.x).div_euclid(res);
        let y = (p.y - self.origin.y).div_euclid(res);
        let x = u16::try_from(x).ok()?;
        let y = u16::try_from(y).ok()?;
        (x < self.width && y < self.height).then_some((x, y))
    }

    /// The bounding box, centimetres.
    #[must_use]
    pub fn bbox(&self) -> RectCm {
        let res = i64::from(self.resolution_cm);
        RectCm {
            min: self.origin,
            max: PointCm {
                x: self.origin.x + i64::from(self.width) * res,
                y: self.origin.y + i64::from(self.height) * res,
            },
        }
    }

    /// Every cell whose centre `shape` contains.
    #[must_use]
    pub fn cells_in(&self, shape: &Shape) -> Vec<(u16, u16)> {
        let bbox = shape.bbox();
        let mut out = Vec::new();
        let (x0, y0) = self.clamp_cell(bbox.min);
        let (x1, y1) = self.clamp_cell(PointCm {
            x: bbox.max.x.saturating_add(i64::from(self.resolution_cm)),
            y: bbox.max.y.saturating_add(i64::from(self.resolution_cm)),
        });
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x < self.width && y < self.height && shape.contains(self.centre(x, y)) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    fn clamp_cell(&self, p: PointCm) -> (u16, u16) {
        let res = i64::from(self.resolution_cm);
        let x = (p.x - self.origin.x)
            .div_euclid(res)
            .clamp(0, i64::from(self.width.saturating_sub(1)));
        let y = (p.y - self.origin.y)
            .div_euclid(res)
            .clamp(0, i64::from(self.height.saturating_sub(1)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (x as u16, y as u16)
    }

    /// The cells a polyline passes through (a conservative walk along each segment at
    /// half-cell steps).
    #[must_use]
    pub fn cells_along(&self, points: &[PointCm]) -> Vec<(u16, u16)> {
        let mut out: Vec<(u16, u16)> = Vec::new();
        let step = i64::from(self.resolution_cm).max(2) / 2;
        for pair in points.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = dx.abs().max(dy.abs());
            let steps = (len / step).max(1);
            for i in 0..=steps {
                let p = PointCm {
                    x: a.x + dx * i / steps,
                    y: a.y + dy * i / steps,
                };
                if let Some(c) = self.cell_of(p)
                    && out.last() != Some(&c)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
        }
        out
    }
}

/// Ceiling division for non-negative numerators.
fn ceil_div(n: i64, d: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    (n + d - 1) / d
}

/// Whether two shapes overlap in any cell centre at `resolution_cm` (doc 14 P3: room
/// footprints partition their surface; 11a M9: site footprints do not overlap).
#[must_use]
pub fn overlap(a: &Shape, b: &Shape, resolution_cm: u16) -> bool {
    let bbox = a.bbox().union(&b.bbox());
    let Some(grid) = Grid::over(
        bbox,
        resolution_cm,
        u32::from(MAX_TILE_SIDE) * u32::from(MAX_TILE_SIDE),
    ) else {
        // Too large to test at this resolution: test coarser rather than answer
        // "no overlap" for two shapes we did not look at.
        return RESOLUTIONS_CM
            .iter()
            .copied()
            .filter(|r| *r > resolution_cm)
            .find_map(|r| Grid::over(bbox, r, u32::from(MAX_TILE_SIDE) * u32::from(MAX_TILE_SIDE)))
            .is_none_or(|g| g.cells_in(a).iter().any(|c| b.contains(g.centre(c.0, c.1))));
    };
    grid.cells_in(a)
        .iter()
        .any(|c| b.contains(grid.centre(c.0, c.1)))
}

/// Where a portal between two footprints goes: the midpoint of their shared edge
/// (doc 14 §3.2), or `None` when they do not touch.
#[must_use]
pub fn shared_threshold(a: &Shape, b: &Shape) -> Option<(PointCm, PointCm)> {
    a.ring().shared_edge(&b.ring())
}

/// The point a portal is drawn at, from its threshold.
#[must_use]
pub fn threshold_point(threshold: (PointCm, PointCm)) -> PointCm {
    midpoint(threshold.0, threshold.1)
}

/// What one surface's rasterization needs to know.
pub struct SurfaceInput<'a> {
    /// The region (for the background).
    pub region: &'a RegionRec,
    /// The surface.
    pub surface: &'a SurfaceRef,
    /// The area to cover, centimetres.
    pub bbox: RectCm,
    /// The preferred resolution.
    pub resolution_cm: u16,
    /// The rooms on this surface.
    pub rooms: Vec<&'a RoomGeo>,
    /// The structures whose rings are walls on this surface (ground: every structure
    /// in the area; a floor: its own structure).
    pub structures: Vec<&'a StructureRec>,
    /// The features on this surface.
    pub features: Vec<&'a FeatureRec>,
    /// The portals on this surface (doors cut the walls).
    pub portals: Vec<&'a PortalRec>,
    /// The floor's height above the structure's ground, centimetres (floors only).
    pub floor_z_cm: i32,
    /// The cell cap.
    pub max_cells: u32,
}

/// One rasterized surface.
#[derive(Clone, Debug)]
pub struct Rasterized {
    /// The grid.
    pub grid: Grid,
    /// The cells.
    pub cells: Cells,
    /// The room table `cell.room - 1` indexes.
    pub rooms: Vec<ObjId>,
}

/// Rasterize one surface, coarsening until the grid fits `max_cells` and the encoded
/// cells fit [`MAX_TILE_DOC_BYTES`].
///
/// # Errors
///
/// A message when no resolution fits, or a footprint will not validate.
pub fn rasterize(input: &SurfaceInput<'_>) -> Result<Rasterized, String> {
    let mut last_error = String::from("no resolution fits");
    for res in RESOLUTIONS_CM
        .iter()
        .copied()
        .filter(|r| *r >= input.resolution_cm)
    {
        let Some(grid) = Grid::over(input.bbox, res, input.max_cells) else {
            continue;
        };
        let out = rasterize_at(input, grid)?;
        if out.cells.encode().len() <= MAX_TILE_DOC_BYTES {
            return Ok(out);
        }
        last_error = format!("the tile at {res} cm encodes past {MAX_TILE_DOC_BYTES} bytes");
    }
    Err(last_error)
}

fn rasterize_at(input: &SurfaceInput<'_>, grid: Grid) -> Result<Rasterized, String> {
    let floor = matches!(input.surface, SurfaceRef::Floor { .. });
    let structure_shapes: Vec<PolygonCm> = input
        .structures
        .iter()
        .map(|s| {
            s.footprint
                .to_cm()
                .map_err(|e| format!("structure {}: {e}", s.key))
        })
        .collect::<Result<_, _>>()?;
    // The base: background, or the floor slab.
    let mut cells = Cells::filled(grid.width, grid.height, Cell::default(), input.max_cells)
        .map_err(|e| e.to_string())?;
    let anchor_elev = |shape: &PolygonCm| -> i32 {
        let c = shape.centre();
        background(input.region, &SurfaceRef::Ground, c.x, c.y).elev_cm
    };
    for y in 0..grid.height {
        for x in 0..grid.width {
            let p = grid.centre(x, y);
            let cell = if floor {
                let Some(slab) = structure_shapes.iter().find(|s| s.contains(p)) else {
                    continue;
                };
                Cell {
                    class: TerrainClass::Floor,
                    elev_cm: clamp_elev(anchor_elev(slab).saturating_add(input.floor_z_cm)),
                    cover: 0,
                    room: 0,
                }
            } else {
                let g = background(input.region, input.surface, p.x, p.y);
                // Inside a building, the ground surface is not a surface at all.
                let inside = structure_shapes.iter().any(|s| s.contains(p));
                Cell {
                    class: if inside { TerrainClass::Void } else { g.class },
                    elev_cm: clamp_elev(g.elev_cm),
                    cover: 0,
                    room: 0,
                }
            };
            cells.set(x, y, cell);
        }
    }
    // Raised features (platforms, docks) lift the ground under them.
    for feature in &input.features {
        if feature.raise_cm == 0 {
            continue;
        }
        let Geometry::Polygon(poly) = &feature.geometry else {
            continue;
        };
        let shape = Shape::Polygon(
            poly.to_cm()
                .map_err(|e| format!("feature {}: {e}", feature.key))?,
        );
        for (x, y) in grid.cells_in(&shape) {
            cells.update(x, y, |c| {
                c.elev_cm = clamp_elev(i32::from(c.elev_cm).saturating_add(feature.raise_cm));
                if feature.kind == FeatureKind::Platform {
                    c.class = TerrainClass::Platform;
                }
            });
        }
    }
    // Rooms: cells and class.
    let mut rooms: Vec<ObjId> = Vec::new();
    let mut sorted: Vec<&RoomGeo> = input.rooms.clone();
    sorted.sort_by_key(|g| g.room);
    for geo in sorted {
        let shape = geo
            .footprint
            .shape()
            .map_err(|e| format!("room {}: {e}", geo.key))?;
        let slot = u16::try_from(rooms.len().saturating_add(1))
            .map_err(|_| "more rooms than a tile can index".to_string())?;
        rooms.push(geo.room);
        for (x, y) in grid.cells_in(&shape) {
            cells.update(x, y, |c| {
                c.room = slot;
                match geo.kind {
                    RoomKind::Indoor => c.class = TerrainClass::Floor,
                    RoomKind::Circulation => {
                        if c.class == TerrainClass::Void {
                            c.class = TerrainClass::Paved;
                        }
                    }
                    RoomKind::Outdoor => {
                        if c.class == TerrainClass::Void {
                            c.class = TerrainClass::Ground;
                        }
                    }
                }
            });
        }
    }
    // Walls: every structure ring on this surface.
    for shape in &structure_shapes {
        let mut ring: Vec<PointCm> = shape.points().to_vec();
        if let Some(first) = ring.first().copied() {
            ring.push(first);
        }
        for (x, y) in grid.cells_along(&ring) {
            cells.update(x, y, |c| c.class = TerrainClass::Wall);
        }
    }
    // Authored walls and rails.
    for feature in &input.features {
        if !matches!(
            feature.kind,
            FeatureKind::Wall | FeatureKind::Rail | FeatureKind::Counter
        ) {
            continue;
        }
        let points: Vec<PointCm> = match &feature.geometry {
            Geometry::Polyline(points) => points
                .iter()
                .map(|p| p.to_cm().map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?,
            Geometry::Polygon(poly) => {
                let p = poly.to_cm().map_err(|e| e.to_string())?;
                let mut ring = p.points().to_vec();
                if let Some(first) = ring.first().copied() {
                    ring.push(first);
                }
                ring
            }
            Geometry::Point(_) => continue,
        };
        if feature.kind == FeatureKind::Wall && feature.blocks.movement {
            for (x, y) in grid.cells_along(&points) {
                cells.update(x, y, |c| c.class = TerrainClass::Wall);
            }
        }
    }
    // Doors cut the walls.
    for portal in &input.portals {
        if !matches!(
            portal.mode,
            PortalMode::Door | PortalMode::Gate | PortalMode::Open
        ) {
            continue;
        }
        let p = portal.point();
        let (Ok(x), Ok(y)) = (cm(p.0), cm(p.1)) else {
            continue;
        };
        if let Some((cx, cy)) = grid.cell_of(PointCm { x, y }) {
            cells.update(cx, cy, |c| {
                if c.class == TerrainClass::Wall {
                    c.class = TerrainClass::Door;
                }
            });
        }
    }
    Ok(Rasterized { grid, cells, rooms })
}

fn clamp_elev(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

/// Derive the wall features of a structure: one closed polyline per floor (doc 14
/// §3.2, "a structure's footprint edges → wall features on every floor").
#[must_use]
pub fn wall_features(
    structure: &StructureRec,
    region: &crate::record::RecordKey,
) -> Vec<FeatureRec> {
    let mut out = Vec::with_capacity(structure.floors.len());
    for floor in &structure.floors {
        let mut ring = structure.footprint.0.clone();
        if let Some(first) = ring.first().copied() {
            ring.push(first);
        }
        let entry = format!(
            "{}.wall.{}",
            structure.key.entry,
            floor.index.unsigned_abs()
        );
        let Ok(entry) = mush_core::CanonicalId::new(&entry) else {
            continue;
        };
        out.push(FeatureRec {
            key: crate::record::RecordKey::new(structure.key.pack.clone(), entry),
            region: region.clone(),
            surface: SurfaceRef::Floor {
                structure: structure.key.clone(),
                index: floor.index,
            },
            kind: FeatureKind::Wall,
            geometry: Geometry::Polyline(ring),
            z_range: (0.0, floor.height_m),
            blocks: crate::record::Blocks {
                movement: true,
                sight: true,
            },
            state: crate::record::FeatureState::Intact,
            raise_cm: 0,
            room: None,
            revision: structure.revision,
        });
    }
    out
}

/// The features grouped by surface, for lookups.
#[must_use]
pub fn by_surface<'a>(
    features: impl Iterator<Item = &'a FeatureRec>,
) -> BTreeMap<SurfaceRef, Vec<&'a FeatureRec>> {
    let mut out: BTreeMap<SurfaceRef, Vec<&FeatureRec>> = BTreeMap::new();
    for f in features {
        out.entry(f.surface.clone()).or_default().push(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point2, Polygon2, Rect2};
    use crate::record::{Access, BackgroundRecipe, Footprint, RecordKey, RegionKind};
    use mush_core::CanonicalId;

    fn key(e: &str) -> RecordKey {
        RecordKey::new(
            CanonicalId::new("p").expect("id"),
            CanonicalId::new(e).expect("id"),
        )
    }

    fn region() -> RegionRec {
        RegionRec {
            key: key("region"),
            name: "R".to_string(),
            kind: RegionKind::Surface {
                body: "earth".to_string(),
            },
            frame: None,
            parent: None,
            extent: Rect2(Point2(0.0, 0.0), Point2(100.0, 100.0)),
            datum_cm: 0,
            background: BackgroundRecipe {
                surface: TerrainClass::Paved,
                mix: Vec::new(),
                relief_cm: 0,
                relief_period_cm: 800,
                water_cm: None,
                seed: 1,
            },
            ground: Vec::new(),
            ground_resolution_cm: 1000,
            revision: 1,
        }
    }

    fn room(id: u32, e: &str, rect: Rect2, kind: RoomKind, surface: SurfaceRef) -> RoomGeo {
        RoomGeo {
            room: ObjId(id),
            key: key(e),
            neighborhood: key("nb"),
            site: None,
            surface,
            footprint: Footprint::Rect(rect),
            kind,
            access: Access::Public,
            revision: 1,
        }
    }

    #[test]
    fn a_grid_fits_by_coarsening() {
        let bbox = RectCm {
            min: PointCm { x: 0, y: 0 },
            max: PointCm {
                x: 40_000,
                y: 40_000,
            },
        };
        let g = Grid::fitting(bbox, 100, 65_536).expect("fits");
        assert_eq!(
            g.resolution_cm, 200,
            "400 m at 1 m is 160 000 cells; 2 m fits"
        );
        assert_eq!((g.width, g.height), (200, 200));
        assert!(Grid::over(bbox, 100, 65_536).is_none());
    }

    #[test]
    fn rooms_fill_their_cells_and_rings_become_walls_with_a_door() {
        let r = region();
        let structure = StructureRec {
            key: key("s"),
            site: key("site"),
            footprint: Rect2(Point2(10.0, 10.0), Point2(20.0, 20.0)).to_polygon(),
            floors: vec![crate::record::Floor {
                index: 0,
                z_offset_m: 0.0,
                height_m: 3.0,
                name: None,
                tile: None,
            }],
            revision: 1,
        };
        let floor = SurfaceRef::Floor {
            structure: key("s"),
            index: 0,
        };
        let lobby = room(
            5,
            "lobby",
            Rect2(Point2(11.0, 11.0), Point2(19.0, 19.0)),
            RoomKind::Indoor,
            floor.clone(),
        );
        let portal = PortalRec {
            key: key("door"),
            exits: vec![ObjId(7)],
            from: Endpoint::Room(ObjId(5)),
            to: Endpoint::Room(ObjId(6)),
            geometry: crate::record::PortalGeometry::Point(crate::geom::Point3(15.0, 10.0, 0.0)),
            mode: PortalMode::Door,
            distance_m: 0,
            dz_m: 0.0,
            time: crate::record::TimePolicy::Instant,
            visibility: crate::record::Visibility::Public,
            revision: 1,
        };
        use crate::record::Endpoint;
        let input = SurfaceInput {
            region: &r,
            surface: &floor,
            bbox: RectCm {
                min: PointCm { x: 1000, y: 1000 },
                max: PointCm { x: 2000, y: 2000 },
            },
            resolution_cm: 100,
            rooms: vec![&lobby],
            structures: vec![&structure],
            features: Vec::new(),
            portals: vec![&portal],
            floor_z_cm: 0,
            max_cells: 65_536,
        };
        let out = rasterize(&input).expect("rasterizes");
        assert_eq!((out.grid.width, out.grid.height), (10, 10));
        assert_eq!(out.rooms, vec![ObjId(5)]);
        let inner = out.cells.get(5, 5).expect("cell");
        assert_eq!(inner.room, 1);
        assert_eq!(inner.class, TerrainClass::Floor);
        let corner = out.cells.get(0, 0).expect("cell");
        assert_eq!(corner.class, TerrainClass::Wall, "the ring is a wall");
        let door = out.cells.get(5, 0).expect("cell");
        assert_eq!(door.class, TerrainClass::Door, "the door cuts the wall");
        // The ground surface of the same area reads Void inside the building.
        let ground_input = SurfaceInput {
            surface: &SurfaceRef::Ground,
            rooms: Vec::new(),
            portals: Vec::new(),
            ..input
        };
        let ground = rasterize(&ground_input).expect("ground");
        assert_eq!(
            ground.cells.get(5, 5).map(|c| c.class),
            Some(TerrainClass::Void)
        );
        assert_eq!(
            ground.cells.get(0, 0).map(|c| c.class),
            Some(TerrainClass::Wall)
        );
    }

    #[test]
    fn overlap_and_touching_are_told_apart() {
        let a = Shape::Polygon(
            Rect2(Point2(0.0, 0.0), Point2(10.0, 10.0))
                .to_polygon()
                .to_cm()
                .expect("a"),
        );
        let b = Shape::Polygon(
            Rect2(Point2(10.0, 0.0), Point2(20.0, 10.0))
                .to_polygon()
                .to_cm()
                .expect("b"),
        );
        let c = Shape::Polygon(
            Rect2(Point2(5.0, 5.0), Point2(15.0, 15.0))
                .to_polygon()
                .to_cm()
                .expect("c"),
        );
        assert!(!overlap(&a, &b, 100));
        assert!(overlap(&a, &c, 100));
        let t = shared_threshold(&a, &b).expect("touch");
        assert_eq!(threshold_point(t), PointCm { x: 1000, y: 500 });
        assert!(
            shared_threshold(&a, &c).is_none(),
            "overlapping is not touching"
        );
        let tri = Shape::Polygon(
            Polygon2(vec![
                Point2(20.0, 0.0),
                Point2(30.0, 0.0),
                Point2(20.0, 10.0),
            ])
            .to_cm()
            .expect("tri"),
        );
        assert!(!overlap(&b, &tri, 100));
        assert!(
            shared_threshold(&b, &tri).is_some(),
            "b's right edge meets the triangle's left"
        );
    }
}
