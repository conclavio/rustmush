//! **The scene model** (doc 21 §3.3, D-21-8; cut G.5a): bucket B's retained
//! 2D scene and the one rasteriser that draws every bucket-B picture — the
//! orbital view, and, behind it, damage control, fitting, the boards, the
//! replay. A scene builder turns a feed into a [`Scene`]; [`Scene::draw`]
//! turns the scene into pixels.
//!
//! A scene is a [`Viewport`] — a world rectangle (y up) mapped onto a frame
//! rectangle (y down) through a [`Transform`]: linear, or the orbital view's
//! log-radial scale — and [`Layer`]s of [`Item`]s drawn in order. Items
//! placed in the world ([`Item::Rect`], [`Item::Line`], [`Item::Polyline`],
//! [`Item::Trail`], [`Item::Circle`], [`Item::Arc`]) go through the
//! transform, and a curve the transform bends (a circle off the log scale's
//! centre, a straight leg through it) is sampled; items placed at a world
//! point but sized in pixels ([`Item::Marker`], [`Item::Ray`],
//! [`Item::Text`]) keep their size whatever the scale; [`Item::Panel`] and
//! [`Item::Image`] are frame rectangles.
//!
//! **Colours** are the items' own: a builder draws from a small fixed set
//! (the overlay's colours and its own), so a frame stays inside the PNG's
//! 256. There is no blending and no anti-aliasing, on purpose: a line is
//! Bresenham's, a disc is its pixels, text is the 5x7 font.

use crate::Frame;
use crate::frame::Rgb;
use crate::overlay::{fill_rect, line, outline_rect, text};

/// How world points land in the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    /// Straight across: the world rectangle stretched over the frame
    /// rectangle.
    Linear,
    /// **The orbital view's scale** (doc 19 §5): about the world origin, a
    /// point `r` from it is drawn at `ln(1 + r/r0) / ln(1 + r_max/r0)` of the
    /// frame's radius (clamped to the edge), at its own angle — the inner
    /// planets spread, the outer ones kept on the page. The frame radius is
    /// [`LOG_RADIAL_FILL`] of half the frame rectangle's lesser side.
    LogRadial {
        /// Where the log curve turns, world units (0.3 AU for the chart).
        r0: f64,
        /// The edge, world units.
        r_max: f64,
    },
}

/// How much of the half-side a log-radial scale fills (the web chart's
/// 0.94).
pub const LOG_RADIAL_FILL: f64 = 0.94;

/// A world rectangle on a frame rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The world's west, south, east and north edges.
    pub world: (f64, f64, f64, f64),
    /// The frame rectangle: left, top, width, height, pixels.
    pub frame: (i64, i64, i64, i64),
    /// The mapping.
    pub transform: Transform,
}

impl Viewport {
    /// Where a world point lands, in (fractional) frame pixels.
    #[must_use]
    pub fn to_frame(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (w0, s0, e0, n0) = self.world;
        let (fx, fy, fw, fh) = self.frame;
        match self.transform {
            Transform::Linear => {
                let sx = fw as f64 / (e0 - w0).max(f64::MIN_POSITIVE);
                let sy = fh as f64 / (n0 - s0).max(f64::MIN_POSITIVE);
                (fx as f64 + (x - w0) * sx, fy as f64 + (n0 - y) * sy)
            }
            Transform::LogRadial { r0, r_max } => {
                let (cx, cy) = (fx as f64 + fw as f64 / 2.0, fy as f64 + fh as f64 / 2.0);
                let half = (fw.min(fh) as f64 / 2.0) * LOG_RADIAL_FILL;
                let r = x.hypot(y);
                if r <= 0.0 {
                    return (cx, cy);
                }
                let f = ((1.0 + r / r0).ln() / (1.0 + r_max / r0).ln()).min(1.0);
                (cx + f * half * x / r, cy - f * half * y / r)
            }
        }
    }

    /// [`Self::to_frame`], floored to a pixel.
    #[must_use]
    pub fn to_px(&self, p: (f64, f64)) -> (i64, i64) {
        let (x, y) = self.to_frame(p);
        (x.floor() as i64, y.floor() as i64)
    }

    fn bends(&self) -> bool {
        !matches!(self.transform, Transform::Linear)
    }
}

/// How a marker is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A filled square `2h + 1` a side.
    Square,
    /// A filled disc of radius `h`.
    Disc,
    /// A ring of radius `h`.
    Ring,
}

/// Where text sits against its point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The point is the text's top-left.
    TopLeft,
    /// The point is the text's middle.
    Centre,
}

/// One thing a scene draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A frame rectangle, filled, with an optional border: a panel.
    Panel {
        /// Left, top, width, height, pixels.
        rect: (i64, i64, i64, i64),
        /// The fill.
        fill: Rgb,
        /// The border, if any.
        border: Option<Rgb>,
    },
    /// A world rectangle (south-west and north-east corners), filled.
    Rect {
        /// The south-west corner.
        min: (f64, f64),
        /// The north-east corner.
        max: (f64, f64),
        /// The fill.
        fill: Rgb,
    },
    /// A straight line between world points (sampled where the transform
    /// bends it).
    Line {
        /// One end.
        a: (f64, f64),
        /// The other.
        b: (f64, f64),
        /// The colour.
        colour: Rgb,
        /// Dashes, `(on, off)` pixels, or solid.
        dash: Option<(u8, u8)>,
    },
    /// Straight lines through world points in turn.
    Polyline {
        /// The points.
        points: Vec<(f64, f64)>,
        /// The colour.
        colour: Rgb,
        /// Dashes, `(on, off)` pixels, or solid.
        dash: Option<(u8, u8)>,
    },
    /// A polyline whose points have ages (a trail): a segment that starts at
    /// or past `old_after` is drawn in `old`.
    Trail {
        /// The points and their ages, newest last.
        points: Vec<((f64, f64), u32)>,
        /// The colour of the fresh part.
        colour: Rgb,
        /// The colour of the old part.
        old: Rgb,
        /// The age from which a segment is old.
        old_after: u32,
    },
    /// A circle about a world point, of a world radius (sampled; a ring
    /// about the log scale's centre stays a circle).
    Circle {
        /// The centre.
        centre: (f64, f64),
        /// The radius, world units.
        radius: f64,
        /// The colour.
        colour: Rgb,
        /// Dashes, `(on, off)` pixels, or solid.
        dash: Option<(u8, u8)>,
    },
    /// An arc, or a filled sector, about a world point: `start_deg` and
    /// `sweep_deg` clockwise from north, as bearings are.
    Arc {
        /// The centre.
        centre: (f64, f64),
        /// The radius, world units.
        radius: f64,
        /// Where it starts, degrees clockwise from north.
        start_deg: f64,
        /// How far it sweeps, degrees clockwise.
        sweep_deg: f64,
        /// The colour.
        colour: Rgb,
        /// Filled from the centre (a sector) rather than an arc alone.
        filled: bool,
    },
    /// A marker at a world point, sized in pixels.
    Marker {
        /// Where.
        at: (f64, f64),
        /// The half-size (square) or radius (disc, ring), pixels.
        half: i64,
        /// How.
        shape: Shape,
        /// The colour.
        colour: Rgb,
    },
    /// A ray from a world point on a bearing (clockwise from north), sized
    /// in pixels: a heading.
    Ray {
        /// Where it starts.
        at: (f64, f64),
        /// The bearing, degrees clockwise from north.
        bearing_deg: f32,
        /// How long, pixels.
        len_px: f32,
        /// The colour.
        colour: Rgb,
    },
    /// Text at a world point, offset in pixels, in the 5x7 font at `scale`.
    Text {
        /// The point.
        at: (f64, f64),
        /// The offset from it, pixels.
        offset: (i64, i64),
        /// How it sits against the point (after the offset).
        anchor: Anchor,
        /// The words (capitals, digits, a little punctuation).
        text: String,
        /// The colour.
        colour: Rgb,
        /// The font's scale.
        scale: usize,
    },
    /// A frame embedded at a frame point (its top-left): a doll in a plan,
    /// a minimap in a cockpit.
    Image {
        /// Where its top-left goes, pixels.
        at: (i64, i64),
        /// The picture.
        frame: Frame,
    },
}

/// Items drawn together, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    /// The items.
    pub items: Vec<Item>,
}

/// A retained scene: a viewport and its layers.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Where the world goes.
    pub viewport: Viewport,
    /// The layers, first drawn first.
    pub layers: Vec<Layer>,
}

/// How many straight pieces a full circle is drawn with where the transform
/// bends it.
const CIRCLE_STEPS: usize = 180;
/// How far a straight line may run, pixels, before it is cut into pieces the
/// transform can bend.
const SAMPLE_PX: f64 = 6.0;

impl Scene {
    /// A scene with one empty layer.
    #[must_use]
    pub fn new(viewport: Viewport) -> Scene {
        Scene {
            viewport,
            layers: vec![Layer::default()],
        }
    }

    /// Add an item to the last layer.
    pub fn push(&mut self, item: Item) {
        if self.layers.is_empty() {
            self.layers.push(Layer::default());
        }
        if let Some(layer) = self.layers.last_mut() {
            layer.items.push(item);
        }
    }

    /// Start a new layer.
    pub fn layer(&mut self) {
        self.layers.push(Layer::default());
    }

    /// Draw every layer into `frame`, in order.
    pub fn draw(&self, frame: &mut Frame) {
        for layer in &self.layers {
            for item in &layer.items {
                self.draw_item(frame, item);
            }
        }
    }

    fn draw_item(&self, frame: &mut Frame, item: &Item) {
        let vp = &self.viewport;
        match item {
            Item::Panel { rect, fill, border } => {
                let (x, y, w, h) = *rect;
                fill_rect(frame, x, y, w, h, *fill);
                if let Some(b) = border {
                    outline_rect(frame, x, y, w, h, *b);
                }
            }
            Item::Rect { min, max, fill } => {
                let (x0, y1) = vp.to_px(*min);
                let (x1, y0) = vp.to_px(*max);
                fill_rect(frame, x0, y0, x1 - x0, y1 - y0, *fill);
            }
            Item::Line { a, b, colour, dash } => {
                self.stroke(frame, &self.sampled(&[*a, *b]), *colour, *dash);
            }
            Item::Polyline {
                points,
                colour,
                dash,
            } => {
                self.stroke(frame, &self.sampled(points), *colour, *dash);
            }
            Item::Trail {
                points,
                colour,
                old,
                old_after,
            } => {
                for pair in points.windows(2) {
                    if let [(a, age), (b, _)] = pair {
                        let c = if *age >= *old_after { *old } else { *colour };
                        self.stroke(frame, &self.sampled(&[*a, *b]), c, None);
                    }
                }
            }
            Item::Circle {
                centre,
                radius,
                colour,
                dash,
            } => {
                let pts = arc_points(*centre, *radius, 0.0, 360.0, CIRCLE_STEPS);
                let px: Vec<(i64, i64)> = pts.iter().map(|p| vp.to_px(*p)).collect();
                self.stroke(frame, &px, *colour, *dash);
            }
            Item::Arc {
                centre,
                radius,
                start_deg,
                sweep_deg,
                colour,
                filled,
            } => {
                let steps = ((sweep_deg.abs() / 2.0).ceil() as usize).max(2);
                let pts = arc_points(*centre, *radius, *start_deg, *sweep_deg, steps);
                let rim: Vec<(i64, i64)> = pts.iter().map(|p| vp.to_px(*p)).collect();
                if *filled {
                    let c = vp.to_px(*centre);
                    for p in &rim {
                        line(frame, c.0, c.1, p.0, p.1, *colour);
                    }
                }
                self.stroke(frame, &rim, *colour, None);
            }
            Item::Marker {
                at,
                half,
                shape,
                colour,
            } => {
                let (x, y) = vp.to_px(*at);
                let h = *half;
                match shape {
                    Shape::Square => fill_rect(frame, x - h, y - h, 2 * h + 1, 2 * h + 1, *colour),
                    Shape::Disc | Shape::Ring => {
                        for dy in -h..=h {
                            for dx in -h..=h {
                                let d2 = dx * dx + dy * dy;
                                let inside = d2 <= h * h + h;
                                let rim = d2 >= (h - 1) * (h - 1) + (h - 1).max(0);
                                if inside && (*shape == Shape::Disc || rim) {
                                    put(frame, x + dx, y + dy, *colour);
                                }
                            }
                        }
                    }
                }
            }
            Item::Ray {
                at,
                bearing_deg,
                len_px,
                colour,
            } => {
                let (x, y) = vp.to_px(*at);
                let (dx, dy) = bearing_deg.to_radians().sin_cos();
                line(
                    frame,
                    x,
                    y,
                    x + (dx * len_px) as i64,
                    y - (dy * len_px) as i64,
                    *colour,
                );
            }
            Item::Text {
                at,
                offset,
                anchor,
                text: words,
                colour,
                scale,
            } => {
                let (x, y) = vp.to_px(*at);
                let s = *scale as i64;
                let (mut tx, mut ty) = (x + offset.0, y + offset.1);
                if *anchor == Anchor::Centre {
                    let w = words.chars().count() as i64 * 6 * s - s;
                    tx -= w / 2;
                    ty -= 7 * s / 2;
                }
                text(frame, tx, ty, words, *colour, *scale);
            }
            Item::Image { at, frame: image } => {
                for y in 0..image.height() {
                    for x in 0..image.width() {
                        put(frame, at.0 + x as i64, at.1 + y as i64, image.get(x, y));
                    }
                }
            }
        }
    }

    /// World points as frame pixels; where the transform bends straight
    /// lines, each segment is cut into pieces no longer than [`SAMPLE_PX`]
    /// on the page first.
    fn sampled(&self, points: &[(f64, f64)]) -> Vec<(i64, i64)> {
        let vp = &self.viewport;
        if !vp.bends() {
            return points.iter().map(|p| vp.to_px(*p)).collect();
        }
        let mut out = Vec::new();
        for pair in points.windows(2) {
            if let [a, b] = pair {
                let (fa, fb) = (vp.to_frame(*a), vp.to_frame(*b));
                let len = (fb.0 - fa.0).hypot(fb.1 - fa.1);
                // A straight line through a log scale can swing far from its
                // ends' chord near the centre; sample by the world length too.
                let steps = ((len / SAMPLE_PX).ceil() as usize).clamp(8, 400);
                for i in 0..steps {
                    let t = i as f64 / steps as f64;
                    out.push(vp.to_px((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)));
                }
            }
        }
        if let Some(last) = points.last() {
            out.push(vp.to_px(*last));
        }
        out
    }

    /// Lines through frame pixels in turn, solid or dashed by pixel count.
    fn stroke(&self, frame: &mut Frame, px: &[(i64, i64)], colour: Rgb, dash: Option<(u8, u8)>) {
        let Some((on, off)) = dash else {
            for pair in px.windows(2) {
                if let [a, b] = pair {
                    line(frame, a.0, a.1, b.0, b.1, colour);
                }
            }
            return;
        };
        let period = u32::from(on) + u32::from(off);
        let mut n: u32 = 0;
        for pair in px.windows(2) {
            if let [a, b] = pair {
                for (x, y) in bresenham(*a, *b) {
                    if period == 0 || n % period < u32::from(on) {
                        put(frame, x, y, colour);
                    }
                    n = n.wrapping_add(1);
                }
            }
        }
    }
}

fn put(frame: &mut Frame, x: i64, y: i64, px: Rgb) {
    if x >= 0 && y >= 0 {
        frame.put(x as usize, y as usize, px);
    }
}

/// The pixels of a line, Bresenham, both ends included except the last (so
/// segments joined end to end do not count their joint twice).
fn bresenham(a: (i64, i64), b: (i64, i64)) -> Vec<(i64, i64)> {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = (
        if a.0 < b.0 { 1 } else { -1 },
        if a.1 < b.1 { 1 } else { -1 },
    );
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut out = Vec::new();
    for _ in 0..8192 {
        if (x, y) == b {
            break;
        }
        out.push((x, y));
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// Points on a world circle about `c`, from `start_deg` sweeping
/// `sweep_deg` clockwise from north, `steps` pieces.
fn arc_points(
    c: (f64, f64),
    r: f64,
    start_deg: f64,
    sweep_deg: f64,
    steps: usize,
) -> Vec<(f64, f64)> {
    (0..=steps)
        .map(|i| {
            let a = (start_deg + sweep_deg * i as f64 / steps as f64).to_radians();
            (c.0 + r * a.sin(), c.1 + r * a.cos())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = [200, 40, 40];
    const BLUE: Rgb = [40, 40, 200];

    fn count(f: &Frame, px: Rgb) -> usize {
        (0..f.width())
            .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| f.get(x, y) == px)
            .count()
    }

    #[test]
    fn a_linear_viewport_puts_north_up() {
        let vp = Viewport {
            world: (0.0, 0.0, 10.0, 10.0),
            frame: (5, 5, 100, 100),
            transform: Transform::Linear,
        };
        assert_eq!(
            vp.to_px((0.0, 0.0)),
            (5, 105),
            "south-west at the bottom-left"
        );
        assert_eq!(
            vp.to_px((10.0, 10.0)),
            (105, 5),
            "north-east at the top-right"
        );
        assert_eq!(vp.to_px((5.0, 5.0)), (55, 55));
    }

    /// The orbital view's scale: the centre in the middle, an angle kept, a
    /// radius on the log curve, the edge clamped.
    #[test]
    fn the_log_radial_scale_spreads_the_inside_and_keeps_the_angle() {
        let vp = Viewport {
            world: (-10.0, -10.0, 10.0, 10.0),
            frame: (0, 0, 200, 200),
            transform: Transform::LogRadial {
                r0: 0.3,
                r_max: 10.0,
            },
        };
        assert_eq!(vp.to_px((0.0, 0.0)), (100, 100));
        let half = 100.0 * LOG_RADIAL_FILL;
        let (ex, ey) = vp.to_frame((10.0, 0.0));
        assert!(
            (ex - (100.0 + half)).abs() < 1e-9 && (ey - 100.0).abs() < 1e-9,
            "the edge, east"
        );
        let (_, ny) = vp.to_frame((0.0, 50.0));
        assert!(
            (ny - (100.0 - half)).abs() < 1e-9,
            "past the edge: clamped, north"
        );
        let (mx, _) = vp.to_frame((1.0, 0.0));
        let linear = 100.0 + half * 0.1;
        assert!(
            mx > linear + 20.0,
            "1 AU of 10 drawn far more than a tenth out"
        );
    }

    /// The items draw where they say: a world rect, a dashed line half its
    /// pixels, a marker, a ray, a circle closed, text, an embedded image.
    #[test]
    fn the_items_draw_where_they_say() {
        let vp = Viewport {
            world: (0.0, 0.0, 100.0, 100.0),
            frame: (0, 0, 100, 100),
            transform: Transform::Linear,
        };
        let mut scene = Scene::new(vp);
        scene.push(Item::Rect {
            min: (10.0, 10.0),
            max: (20.0, 20.0),
            fill: RED,
        });
        let mut f = Frame::new(100, 100);
        scene.draw(&mut f);
        assert_eq!(count(&f, RED), 100);
        assert_eq!(f.get(10, 80), RED, "south-west corner, near the bottom");

        let mut f = Frame::new(100, 100);
        let mut s = Scene::new(vp);
        s.push(Item::Line {
            a: (0.0, 50.0),
            b: (40.0, 50.0),
            colour: BLUE,
            dash: Some((2, 2)),
        });
        s.draw(&mut f);
        assert_eq!(count(&f, BLUE), 20, "40 pixels, every other pair");

        let mut f = Frame::new(100, 100);
        let mut s = Scene::new(vp);
        s.push(Item::Marker {
            at: (50.0, 50.0),
            half: 2,
            shape: Shape::Square,
            colour: RED,
        });
        s.push(Item::Ray {
            at: (50.0, 50.0),
            bearing_deg: 90.0,
            len_px: 10.0,
            colour: BLUE,
        });
        s.push(Item::Circle {
            centre: (50.0, 50.0),
            radius: 30.0,
            colour: BLUE,
            dash: None,
        });
        s.push(Item::Text {
            at: (10.0, 90.0),
            offset: (0, 0),
            anchor: Anchor::TopLeft,
            text: "AB".to_string(),
            colour: RED,
            scale: 1,
        });
        let mut img = Frame::new(3, 3);
        for y in 0..3 {
            for x in 0..3 {
                img.put(x, y, [9, 9, 9]);
            }
        }
        s.push(Item::Image {
            at: (90, 90),
            frame: img,
        });
        s.draw(&mut f);
        assert_eq!(
            f.get(50, 52),
            RED,
            "the marker (its centre is under the ray)"
        );
        assert_eq!(f.get(58, 50), BLUE, "the ray points east");
        assert_eq!(f.get(80, 50), BLUE, "the circle's east point");
        assert_eq!(f.get(20, 50), BLUE, "and its west");
        assert!(count(&f, RED) > 25, "the marker and the text");
        assert_eq!(count(&f, [9, 9, 9]), 9);
    }

    /// Under the log scale a circle about the centre stays round, and one
    /// off it, and a straight line through it, are sampled into curves that
    /// stay on the page.
    #[test]
    fn curves_are_sampled_through_the_log_scale() {
        let vp = Viewport {
            world: (-5.0, -5.0, 5.0, 5.0),
            frame: (0, 0, 200, 200),
            transform: Transform::LogRadial {
                r0: 0.3,
                r_max: 5.0,
            },
        };
        let mut s = Scene::new(vp);
        s.push(Item::Circle {
            centre: (0.0, 0.0),
            radius: 1.0,
            colour: RED,
            dash: None,
        });
        let mut f = Frame::new(200, 200);
        s.draw(&mut f);
        let r = vp.to_frame((1.0, 0.0)).0 - 100.0;
        for deg in [0.0f64, 90.0, 180.0, 270.0] {
            let (x, y) = (
                100.0 + r * deg.to_radians().sin(),
                100.0 - r * deg.to_radians().cos(),
            );
            let near = (-1..=1).any(|d: i64| {
                (-1..=1)
                    .any(|e: i64| f.get((x as i64 + d) as usize, (y as i64 + e) as usize) == RED)
            });
            assert!(near, "round at {deg}°");
        }
        let mut s = Scene::new(vp);
        s.push(Item::Line {
            a: (-4.0, 0.5),
            b: (4.0, 0.5),
            colour: BLUE,
            dash: None,
        });
        let mut f = Frame::new(200, 200);
        s.draw(&mut f);
        assert!(count(&f, BLUE) > 100, "drawn the whole way");
        // The line passes 0.5 from the centre: on the log scale it bulges out
        // toward the edge at its ends and dips near the centre.
        let mid = vp.to_px((0.0, 0.5));
        let near = (-1..=1).any(|d: i64| {
            (-1..=1).any(|e: i64| f.get((mid.0 + d) as usize, (mid.1 + e) as usize) == BLUE)
        });
        assert!(near, "through the point it passes, to the pixel");
    }
}
