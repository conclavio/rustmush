//! Plane geometry in **region metres** (doc 14 §1.2: one metric frame per surface region),
//! with the integer-centimetre forms the rasterizer and every equality test use.
//!
//! Floats come in from packs and go out to records unchanged; every comparison —
//! overlap, touching, cell membership — happens on whole centimetres, so two packs that
//! author the same coordinates rasterize identically on every platform and a shared
//! edge is exactly shared.

use serde::{Deserialize, Serialize};

/// The largest magnitude a coordinate may have, in metres (a region is at most 2 000 km
/// across; anything larger is a typo, not a place).
pub const MAX_COORD_M: f64 = 1_000_000.0;

/// The most points a footprint polygon may have.
pub const MAX_POLYGON_POINTS: usize = 64;

/// The fewest points a footprint polygon may have.
pub const MIN_POLYGON_POINTS: usize = 3;

/// A point in region metres. Serializes as `[x, y]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point2(pub f64, pub f64);

/// A point in region metres with height above the surface. Serializes as `[x, y, z]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point3(pub f64, pub f64, pub f64);

/// An axis-aligned rectangle in region metres, `[[x0, y0], [x1, y1]]` in any corner
/// order (normalized by [`Rect2::validate`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect2(pub Point2, pub Point2);

/// A simple polygon in region metres, listed once around (not closed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polygon2(pub Vec<Point2>);

/// A point in whole centimetres.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PointCm {
    /// East, centimetres.
    pub x: i64,
    /// North, centimetres.
    pub y: i64,
}

/// A closed integer rectangle in centimetres: `min` inclusive, `max` exclusive on both
/// axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RectCm {
    /// The low corner.
    pub min: PointCm,
    /// The high corner (exclusive).
    pub max: PointCm,
}

/// Why a geometric value was refused.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum GeomError {
    /// A coordinate is NaN, infinite, or past [`MAX_COORD_M`].
    #[error("coordinate {0} is not a finite value within ±{MAX_COORD_M} m")]
    Coordinate(f64),
    /// Too few or too many polygon points.
    #[error("a footprint needs {MIN_POLYGON_POINTS} to {MAX_POLYGON_POINTS} points, not {0}")]
    PolygonSize(usize),
    /// A polygon with no area, or a rectangle with a zero side.
    #[error("the footprint encloses no area")]
    Degenerate,
    /// A radius that is not positive.
    #[error("radius {0} m is not positive")]
    Radius(f64),
}

/// Whole centimetres of a metre value, refusing anything not finite or too large.
///
/// # Errors
///
/// [`GeomError::Coordinate`].
pub fn cm(metres: f64) -> Result<i64, GeomError> {
    if !metres.is_finite() || metres.abs() > MAX_COORD_M {
        return Err(GeomError::Coordinate(metres));
    }
    // Within ±1e8 cm, exactly representable and far inside i64.
    #[allow(clippy::cast_possible_truncation)]
    Ok((metres * 100.0).round() as i64)
}

/// Metres of a centimetre value.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn metres(cm: i64) -> f64 {
    cm as f64 / 100.0
}

impl Point2 {
    /// Validate and convert to centimetres.
    ///
    /// # Errors
    ///
    /// [`GeomError::Coordinate`].
    pub fn to_cm(self) -> Result<PointCm, GeomError> {
        Ok(PointCm {
            x: cm(self.0)?,
            y: cm(self.1)?,
        })
    }
}

impl Point3 {
    /// The plan projection.
    #[must_use]
    pub fn xy(self) -> Point2 {
        Point2(self.0, self.1)
    }

    /// Validate every coordinate.
    ///
    /// # Errors
    ///
    /// [`GeomError::Coordinate`].
    pub fn validate(self) -> Result<(), GeomError> {
        cm(self.0)?;
        cm(self.1)?;
        cm(self.2)?;
        Ok(())
    }
}

impl PointCm {
    /// Back to metres.
    #[must_use]
    pub fn to_metres(self) -> Point2 {
        Point2(metres(self.x), metres(self.y))
    }
}

impl Rect2 {
    /// Validate and normalize (low corner first).
    ///
    /// # Errors
    ///
    /// [`GeomError::Coordinate`] or [`GeomError::Degenerate`].
    pub fn validate(self) -> Result<Rect2, GeomError> {
        let a = self.0.to_cm()?;
        let b = self.1.to_cm()?;
        if a.x == b.x || a.y == b.y {
            return Err(GeomError::Degenerate);
        }
        Ok(Rect2(
            Point2(self.0.0.min(self.1.0), self.0.1.min(self.1.1)),
            Point2(self.0.0.max(self.1.0), self.0.1.max(self.1.1)),
        ))
    }

    /// The integer rectangle.
    ///
    /// # Errors
    ///
    /// [`GeomError`] as [`Rect2::validate`].
    pub fn to_cm(self) -> Result<RectCm, GeomError> {
        let r = self.validate()?;
        Ok(RectCm {
            min: r.0.to_cm()?,
            max: r.1.to_cm()?,
        })
    }

    /// The rectangle as a four-point polygon, counter-clockwise from the low corner.
    #[must_use]
    pub fn to_polygon(self) -> Polygon2 {
        Polygon2(vec![
            Point2(self.0.0, self.0.1),
            Point2(self.1.0, self.0.1),
            Point2(self.1.0, self.1.1),
            Point2(self.0.0, self.1.1),
        ])
    }
}

impl RectCm {
    /// Width in centimetres.
    #[must_use]
    pub fn width(&self) -> i64 {
        self.max.x.saturating_sub(self.min.x)
    }

    /// Height in centimetres.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.max.y.saturating_sub(self.min.y)
    }

    /// Whether `p` lies inside (min inclusive, max exclusive).
    #[must_use]
    pub fn contains(&self, p: PointCm) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub fn union(&self, other: &RectCm) -> RectCm {
        RectCm {
            min: PointCm {
                x: self.min.x.min(other.min.x),
                y: self.min.y.min(other.min.y),
            },
            max: PointCm {
                x: self.max.x.max(other.max.x),
                y: self.max.y.max(other.max.y),
            },
        }
    }

    /// Back to metres.
    #[must_use]
    pub fn to_metres(&self) -> Rect2 {
        Rect2(self.min.to_metres(), self.max.to_metres())
    }
}

/// A validated polygon in centimetres.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolygonCm {
    points: Vec<PointCm>,
    bbox: RectCm,
}

impl Polygon2 {
    /// Validate: point count, finite coordinates, non-zero area.
    ///
    /// # Errors
    ///
    /// [`GeomError`].
    pub fn to_cm(&self) -> Result<PolygonCm, GeomError> {
        if self.0.len() < MIN_POLYGON_POINTS || self.0.len() > MAX_POLYGON_POINTS {
            return Err(GeomError::PolygonSize(self.0.len()));
        }
        let mut points = Vec::with_capacity(self.0.len());
        for p in &self.0 {
            points.push(p.to_cm()?);
        }
        let polygon = PolygonCm::from_points(points)?;
        Ok(polygon)
    }
}

impl PolygonCm {
    /// Build from centimetre points, refusing a degenerate ring.
    ///
    /// # Errors
    ///
    /// [`GeomError::PolygonSize`] or [`GeomError::Degenerate`].
    pub fn from_points(points: Vec<PointCm>) -> Result<PolygonCm, GeomError> {
        if points.len() < MIN_POLYGON_POINTS || points.len() > MAX_POLYGON_POINTS {
            return Err(GeomError::PolygonSize(points.len()));
        }
        let (Some(first), Some(_)) = (points.first(), points.last()) else {
            return Err(GeomError::PolygonSize(points.len()));
        };
        let mut bbox = RectCm {
            min: *first,
            max: PointCm {
                x: first.x.saturating_add(1),
                y: first.y.saturating_add(1),
            },
        };
        for p in &points {
            bbox = bbox.union(&RectCm {
                min: *p,
                max: PointCm {
                    x: p.x.saturating_add(1),
                    y: p.y.saturating_add(1),
                },
            });
        }
        let polygon = PolygonCm { points, bbox };
        if polygon.twice_area() == 0 {
            return Err(GeomError::Degenerate);
        }
        Ok(polygon)
    }

    /// From an integer rectangle.
    #[must_use]
    pub fn from_rect(rect: RectCm) -> PolygonCm {
        let points = vec![
            rect.min,
            PointCm {
                x: rect.max.x,
                y: rect.min.y,
            },
            rect.max,
            PointCm {
                x: rect.min.x,
                y: rect.max.y,
            },
        ];
        PolygonCm { points, bbox: rect }
    }

    /// The ring.
    #[must_use]
    pub fn points(&self) -> &[PointCm] {
        &self.points
    }

    /// The bounding box (max exclusive).
    #[must_use]
    pub fn bbox(&self) -> RectCm {
        self.bbox
    }

    /// Twice the signed area (shoelace), saturating.
    #[must_use]
    pub fn twice_area(&self) -> i128 {
        let mut total: i128 = 0;
        for (a, b) in self.edges() {
            let term = i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y);
            total = total.saturating_add(term);
        }
        total
    }

    /// The edges, each `(from, to)`, closing the ring.
    pub fn edges(&self) -> impl Iterator<Item = (PointCm, PointCm)> + '_ {
        let n = self.points.len();
        self.points.iter().enumerate().filter_map(move |(i, a)| {
            let b = self.points.get((i + 1) % n)?;
            Some((*a, *b))
        })
    }

    /// Even-odd point-in-polygon on the ring; a point exactly on a right-hand or top
    /// edge counts as outside, so two polygons sharing an edge never both claim it.
    #[must_use]
    pub fn contains(&self, p: PointCm) -> bool {
        if !self.bbox.contains(p) {
            return false;
        }
        let mut inside = false;
        for (a, b) in self.edges() {
            if (a.y > p.y) != (b.y > p.y) {
                // x of the edge at p.y, in 128-bit to avoid overflow.
                let dy = i128::from(b.y) - i128::from(a.y);
                let num = (i128::from(p.y) - i128::from(a.y)) * (i128::from(b.x) - i128::from(a.x));
                let lhs = i128::from(p.x) - i128::from(a.x);
                // p.x < a.x + num/dy  <=>  (p.x - a.x) * dy < num, sign-aware.
                let crosses = if dy > 0 {
                    lhs * dy < num
                } else {
                    lhs * dy > num
                };
                if crosses {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// The centroid of the bounding box.
    #[must_use]
    pub fn centre(&self) -> PointCm {
        PointCm {
            x: self.bbox.min.x.saturating_add(self.bbox.width() / 2),
            y: self.bbox.min.y.saturating_add(self.bbox.height() / 2),
        }
    }

    /// The segment two polygons share along collinear, overlapping edges, if any —
    /// the place a portal goes (doc 14 §3.2). A shared single point is not an edge.
    #[must_use]
    pub fn shared_edge(&self, other: &PolygonCm) -> Option<(PointCm, PointCm)> {
        let mut best: Option<(PointCm, PointCm, i128)> = None;
        for (a1, a2) in self.edges() {
            for (b1, b2) in other.edges() {
                if let Some((s, e)) = overlap_of_collinear(a1, a2, b1, b2) {
                    let len = (i128::from(e.x) - i128::from(s.x)).abs()
                        + (i128::from(e.y) - i128::from(s.y)).abs();
                    if len > 0 && best.as_ref().is_none_or(|(_, _, l)| len > *l) {
                        best = Some((s, e, len));
                    }
                }
            }
        }
        best.map(|(s, e, _)| (s, e))
    }
}

/// The overlap of two collinear segments, or `None` when they are not collinear or do
/// not overlap.
fn overlap_of_collinear(
    a1: PointCm,
    a2: PointCm,
    b1: PointCm,
    b2: PointCm,
) -> Option<(PointCm, PointCm)> {
    let cross = |p: PointCm, q: PointCm, r: PointCm| -> i128 {
        (i128::from(q.x) - i128::from(p.x)) * (i128::from(r.y) - i128::from(p.y))
            - (i128::from(q.y) - i128::from(p.y)) * (i128::from(r.x) - i128::from(p.x))
    };
    if cross(a1, a2, b1) != 0 || cross(a1, a2, b2) != 0 {
        return None;
    }
    // Project onto the dominant axis of `a`.
    let horizontal = (a2.x - a1.x).abs() >= (a2.y - a1.y).abs();
    let key = |p: PointCm| if horizontal { p.x } else { p.y };
    let (alo, ahi) = (key(a1).min(key(a2)), key(a1).max(key(a2)));
    let (blo, bhi) = (key(b1).min(key(b2)), key(b1).max(key(b2)));
    let lo = alo.max(blo);
    let hi = ahi.min(bhi);
    if lo >= hi {
        return None;
    }
    let at = |k: i64| -> PointCm {
        // Interpolate along `a`.
        if horizontal {
            let dx = a2.x - a1.x;
            let y = if dx == 0 {
                a1.y
            } else {
                a1.y + ((k - a1.x) as i128 * (a2.y - a1.y) as i128 / dx as i128) as i64
            };
            PointCm { x: k, y }
        } else {
            let dy = a2.y - a1.y;
            let x = if dy == 0 {
                a1.x
            } else {
                a1.x + ((k - a1.y) as i128 * (a2.x - a1.x) as i128 / dy as i128) as i64
            };
            PointCm { x, y: k }
        }
    };
    Some((at(lo), at(hi)))
}

/// The midpoint of a segment.
#[must_use]
pub fn midpoint(a: PointCm, b: PointCm) -> PointCm {
    PointCm {
        x: a.x.saturating_add(b.x) / 2,
        y: a.y.saturating_add(b.y) / 2,
    }
}

/// A disc footprint, validated: centre and a positive radius in centimetres.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscCm {
    /// The centre.
    pub centre: PointCm,
    /// The radius.
    pub radius: i64,
}

impl DiscCm {
    /// Validate from metres.
    ///
    /// # Errors
    ///
    /// [`GeomError`].
    pub fn new(centre: Point2, radius_m: f64) -> Result<DiscCm, GeomError> {
        let centre = centre.to_cm()?;
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return Err(GeomError::Radius(radius_m));
        }
        let radius = cm(radius_m)?;
        if radius <= 0 {
            return Err(GeomError::Radius(radius_m));
        }
        Ok(DiscCm { centre, radius })
    }

    /// The bounding box.
    #[must_use]
    pub fn bbox(&self) -> RectCm {
        RectCm {
            min: PointCm {
                x: self.centre.x.saturating_sub(self.radius),
                y: self.centre.y.saturating_sub(self.radius),
            },
            max: PointCm {
                x: self.centre.x.saturating_add(self.radius),
                y: self.centre.y.saturating_add(self.radius),
            },
        }
    }

    /// Whether `p` lies inside.
    #[must_use]
    pub fn contains(&self, p: PointCm) -> bool {
        let dx = i128::from(p.x) - i128::from(self.centre.x);
        let dy = i128::from(p.y) - i128::from(self.centre.y);
        dx * dx + dy * dy < i128::from(self.radius) * i128::from(self.radius)
    }

    /// The disc as a 16-gon, for edge tests.
    #[must_use]
    pub fn to_polygon(&self) -> PolygonCm {
        let mut points = Vec::with_capacity(16);
        for i in 0..16_u32 {
            let angle = f64::from(i) * std::f64::consts::TAU / 16.0;
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let p = PointCm {
                x: self.centre.x + (self.radius as f64 * angle.cos()).round() as i64,
                y: self.centre.y + (self.radius as f64 * angle.sin()).round() as i64,
            };
            points.push(p);
        }
        // Sixteen distinct points around a positive radius always enclose area.
        PolygonCm::from_points(points).unwrap_or_else(|_| PolygonCm::from_rect(self.bbox()))
    }
}

/// A validated footprint in centimetres: what the rasterizer takes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Shape {
    /// A polygon (rectangles are four-point polygons).
    Polygon(PolygonCm),
    /// A disc.
    Disc(DiscCm),
}

impl Shape {
    /// The bounding box.
    #[must_use]
    pub fn bbox(&self) -> RectCm {
        match self {
            Shape::Polygon(p) => p.bbox(),
            Shape::Disc(d) => d.bbox(),
        }
    }

    /// Whether `p` lies inside.
    #[must_use]
    pub fn contains(&self, p: PointCm) -> bool {
        match self {
            Shape::Polygon(poly) => poly.contains(p),
            Shape::Disc(d) => d.contains(p),
        }
    }

    /// The centre.
    #[must_use]
    pub fn centre(&self) -> PointCm {
        match self {
            Shape::Polygon(p) => p.centre(),
            Shape::Disc(d) => d.centre,
        }
    }

    /// The ring, for edge tests (a disc as a 16-gon).
    #[must_use]
    pub fn ring(&self) -> PolygonCm {
        match self {
            Shape::Polygon(p) => p.clone(),
            Shape::Disc(d) => d.to_polygon(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PolygonCm {
        Rect2(Point2(x0, y0), Point2(x1, y1))
            .to_polygon()
            .to_cm()
            .expect("rect")
    }

    #[test]
    fn a_shared_edge_is_found_and_a_shared_corner_is_not() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(10.0, 2.0, 20.0, 8.0);
        let (s, e) = a.shared_edge(&b).expect("they touch along x = 10");
        assert_eq!((s.x, e.x), (1000, 1000));
        assert_eq!((s.y.min(e.y), s.y.max(e.y)), (200, 800));
        let c = rect(10.0, 10.0, 20.0, 20.0);
        assert!(a.shared_edge(&c).is_none(), "a corner is not an edge");
        let d = rect(30.0, 0.0, 40.0, 10.0);
        assert!(a.shared_edge(&d).is_none());
    }

    #[test]
    fn containment_is_half_open_so_neighbours_never_both_claim_a_point() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(10.0, 0.0, 20.0, 10.0);
        let on_edge = PointCm { x: 1000, y: 500 };
        assert!(!a.contains(on_edge));
        assert!(b.contains(on_edge));
        assert!(a.contains(PointCm { x: 50, y: 50 }));
        assert!(!a.contains(PointCm { x: -1, y: 50 }));
    }

    #[test]
    fn degenerate_and_oversized_shapes_are_refused() {
        assert_eq!(
            Rect2(Point2(1.0, 1.0), Point2(1.0, 5.0)).validate(),
            Err(GeomError::Degenerate)
        );
        assert_eq!(
            Polygon2(vec![Point2(0.0, 0.0), Point2(1.0, 1.0)]).to_cm(),
            Err(GeomError::PolygonSize(2))
        );
        assert!(matches!(
            Polygon2(vec![
                Point2(0.0, 0.0),
                Point2(f64::NAN, 1.0),
                Point2(1.0, 1.0)
            ])
            .to_cm(),
            Err(GeomError::Coordinate(_))
        ));
        assert!(DiscCm::new(Point2(0.0, 0.0), 0.0).is_err());
    }

    #[test]
    fn a_disc_contains_its_centre_and_not_its_corner() {
        let d = DiscCm::new(Point2(10.0, 10.0), 5.0).expect("disc");
        assert!(d.contains(PointCm { x: 1000, y: 1000 }));
        assert!(!d.contains(PointCm { x: 1500, y: 1500 }));
        assert!(d.to_polygon().twice_area() > 0);
    }
}
