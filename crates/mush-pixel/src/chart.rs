//! **The orbital view** (doc 21 cut G.5b; bucket B's first picture): the
//! `sky.chart` keyframe (doc 19 cut A.3b) drawn as a [`Scene`] — the Sun,
//! the bodies' orbits, the bodies at their places on the chart's own log
//! scale, the passages from the viewer's port as dashed legs (bent by the
//! scale, as they are), light-lag rings about the viewer's body at 3, 6, 12
//! and 24 light-minutes (the owner's defaults of 2026-10-22), the labels, and
//! a title strip.
//!
//! The chart is the viewer's own (built from where they stand, doc 19 §5):
//! the "you" is the vantage body, marked; a ship under way is not on the
//! chart yet (the feed carries none — doc 19's ships along their tickets'
//! legs is a feed change still to come). Moons share their planet's place on
//! a solar-system page and are not drawn apart.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::overlay::{DIM, TEXT, YOU, draw_card};
use crate::picture::{BUCKET_B_INTERVAL, Feeds, Look, Picture};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// The sky chart's feed.
pub const SKY_CHART_FEED: &str = "sky.chart";

/// One light-minute, AU: 299,792,458 m/s for 60 s, over 149,597,870,700 m.
pub const AU_PER_LIGHT_MINUTE: f64 = 0.120_239_3;

/// The light-lag rings, light-minutes: doubling, which spaces evenly on the
/// chart's log scale.
pub const LAG_RING_MINUTES: [u32; 4] = [3, 6, 12, 24];

const SPACE: Rgb = [8, 10, 18];
const ORBIT: Rgb = [38, 46, 68];
const SUN: Rgb = [250, 210, 90];
const BODY: Rgb = [200, 206, 216];
const HERE: Rgb = [110, 220, 255];
const ROUTE: Rgb = [230, 170, 60];
const LAG: Rgb = [70, 120, 150];

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Retained Kitty image ID (`i=180`) for the orbital chart picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_CHART: u32 = 180;
/// Retained Kitty placement ID (`p=80`) for the orbital chart picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_CHART: u32 = 80;
#[cfg(feature = "encode")]
const Z_CHART: i32 = -50;

/// The orbital view as a [`Picture`]: drawn from the held `sky.chart`, on
/// bucket B's clock (on change).
#[derive(Debug, Default)]
pub struct ChartPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
}

impl ChartPicture {
    /// A chart picture.
    #[must_use]
    pub fn new() -> ChartPicture {
        ChartPicture::default()
    }

    /// Return Kitty Graphics escape bytes that delete the retained chart image
    /// (`IMAGE_ID_CHART`), and reset cached placement state.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_CHART);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Invalidate the retained Kitty chart image if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_CHART {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force the retained Kitty chart placement to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render the orbital chart as a retained Kitty Graphics stream (`a=t` +
    /// `a=p`), scaling up to `1280x` when `cell` is known and `!step_down`
    /// (`640x` fallback otherwise).
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<crate::caps::CellSize>,
        step_down: bool,
    ) -> Option<(crate::iso25d::Viewport25D, Vec<u8>)> {
        let vp = crate::cockpit_kitty::viewport_fp(area.2, area.3, cell, step_down);
        let (vw, vh) = (vp.width_px, vp.height_px);
        let mut frame = Frame::new(vw, vh);
        if !self.draw(&mut frame, feeds, look) {
            return None;
        }
        let placed_area = (
            area.0.saturating_add(vp.col_offset),
            area.1.saturating_add(vp.row_offset),
            vp.cols,
            vp.rows,
        );
        let h = frame.hash();
        let mut ops = Vec::new();
        if self.kitty_hash != Some(h) {
            let png = crate::png::encode(&frame);
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_CHART);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_CHART,
                PLACEMENT_CHART,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_CHART,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for ChartPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[SKY_CHART_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_B_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, _look: &Look) -> bool {
        let Some(chart) = feeds.feed(SKY_CHART_FEED) else {
            return false;
        };
        match chart_scene(frame, chart) {
            Some(scene) => scene.draw(frame),
            // A refusal (no lane, nowhere to stand) is a frame with no chart.
            None => draw_card(frame, "NO CHART FROM HERE"),
        }
        true
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// One body as the scene places it.
struct Body<'a> {
    label: &'a str,
    at: (f64, f64),
    r_au: f64,
    here: bool,
}

/// The chart's scene for `frame`, or `None` for a frame with no chart block
/// (a refusal).
#[must_use]
pub fn chart_scene(frame: &Frame, sky: &Value) -> Option<Scene> {
    let chart = sky.get("chart")?;
    let scale = chart.get("scale")?;
    let r_max = num(scale.get("au_max")).filter(|r| *r > 0.0)?;
    let r0 = num(scale.get("r0_au")).filter(|r| *r > 0.0).unwrap_or(0.3);
    let bodies_json = chart.get("bodies")?.as_array()?;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    // The text's scale from the page's lesser side: the circle is drawn in
    // it, and a wide, short picture (the text cockpit's top two-thirds) has
    // a small circle that the doubled font would crowd.
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    let strip = 11 * s;

    let mut scene = Scene::new(Viewport {
        world: (-r_max, -r_max, r_max, r_max),
        frame: (0, strip, w, h - 2 * strip),
        transform: Transform::LogRadial { r0, r_max },
    });
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: SPACE,
        border: None,
    });

    // The planets' and asteroids' bodies; a moon shares its parent's place.
    let bodies: Vec<Body<'_>> = bodies_json
        .iter()
        .filter(|b| b.get("parent").is_none_or(Value::is_null))
        .filter_map(|b| {
            Some(Body {
                label: b.get("label").and_then(Value::as_str).unwrap_or("?"),
                at: (num(b.get("x_au"))?, num(b.get("y_au"))?),
                r_au: num(b.get("r_au"))?,
                here: b.get("here").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect();
    let place = |id: &str| -> Option<(f64, f64)> {
        bodies_json
            .iter()
            .find(|b| b.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|b| Some((num(b.get("x_au"))?, num(b.get("y_au"))?)))
    };

    // The orbits, faint, under everything.
    for b in &bodies {
        scene.push(Item::Circle {
            centre: (0.0, 0.0),
            radius: b.r_au,
            colour: ORBIT,
            dash: None,
        });
    }

    // The light-lag rings about where the viewer stands.
    scene.layer();
    let here = bodies.iter().find(|b| b.here).map(|b| b.at);
    if let Some(at) = here {
        for minutes in LAG_RING_MINUTES {
            let radius = f64::from(minutes) * AU_PER_LIGHT_MINUTE;
            scene.push(Item::Circle {
                centre: at,
                radius,
                colour: LAG,
                dash: Some((2, 3)),
            });
        }
    }

    // The passages from the viewer's port: each leg between two bodies,
    // dashed (a leg on one body — a hop across Ceres — has no length here).
    scene.layer();
    for route in chart
        .get("routes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        for leg in route
            .get("legs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let from = leg.get("from").and_then(Value::as_str).and_then(place);
            let to = leg.get("to").and_then(Value::as_str).and_then(place);
            if let (Some(a), Some(b)) = (from, to)
                && a != b
            {
                scene.push(Item::Line {
                    a,
                    b,
                    colour: ROUTE,
                    dash: Some((4, 3)),
                });
            }
        }
    }

    // The Sun, the bodies and the viewer's.
    scene.layer();
    scene.push(Item::Marker {
        at: (0.0, 0.0),
        half: 3 * s,
        shape: Shape::Disc,
        colour: SUN,
    });
    for b in &bodies {
        scene.push(Item::Marker {
            at: b.at,
            half: s + 1,
            shape: Shape::Disc,
            colour: if b.here { HERE } else { BODY },
        });
        if b.here {
            scene.push(Item::Marker {
                at: b.at,
                half: 4 * s,
                shape: Shape::Ring,
                colour: HERE,
            });
        }
    }

    // The labels (the live smoke of 2026-10-23 found MARS's dot under
    // EARTH's word and the ring labels on the bodies'): every body's dot, the
    // Sun and the viewer's ring are obstacles from the start; a body's label
    // tries above-right, below-right, above-left and below-left of its dot,
    // inside the chart, the viewer's first and then the rest outward, and is
    // left out where none is free (an asteroid beside the viewer's own reads
    // as the viewer's); the light-lag rings' labels last, at each ring's
    // north or south point, where they fit.
    let vp = scene.viewport;
    let mut placed: Vec<(i64, i64, i64, i64)> = Vec::new();
    let dot = |at: (f64, f64), half: i64| {
        let (x, y) = vp.to_px(at);
        (x - half - 1, y - half - 1, 2 * half + 3, 2 * half + 3)
    };
    placed.push(dot((0.0, 0.0), 3 * s));
    for b in &bodies {
        placed.push(dot(b.at, if b.here { 4 * s } else { s + 1 }));
    }
    let clear = |rect: (i64, i64, i64, i64), placed: &[(i64, i64, i64, i64)]| {
        rect.0 >= 0
            && rect.1 >= strip
            && rect.0 + rect.2 <= w
            && rect.1 + rect.3 <= h - strip
            && !placed.iter().any(|p| {
                rect.0 < p.0 + p.2
                    && p.0 < rect.0 + rect.2
                    && rect.1 < p.1 + p.3
                    && p.1 < rect.1 + rect.3
            })
    };
    let mut order: Vec<&Body<'_>> = bodies.iter().collect();
    order.sort_by(|a, b| b.here.cmp(&a.here).then(a.r_au.total_cmp(&b.r_au)));
    for b in order {
        let words = if b.here {
            format!("{} - YOU", b.label)
        } else {
            b.label.to_string()
        };
        let (x, y) = vp.to_px(b.at);
        let (tw, th) = (words.chars().count() as i64 * 6 * s - s, 7 * s);
        let gap = if b.here { 5 * s } else { 3 * s };
        let spots = [
            (gap, -gap - th),
            (gap, gap),
            (-gap - tw, -gap - th),
            (-gap - tw, gap),
        ];
        let Some(offset) = spots
            .into_iter()
            .find(|(dx, dy)| clear((x + dx, y + dy, tw, th), &placed))
        else {
            continue;
        };
        placed.push((x + offset.0, y + offset.1, tw, th));
        scene.push(Item::Text {
            at: b.at,
            offset,
            anchor: Anchor::TopLeft,
            text: words,
            colour: if b.here { HERE } else { TEXT },
            scale: s as usize,
        });
    }
    if let Some(at) = here {
        for minutes in LAG_RING_MINUTES {
            let radius = f64::from(minutes) * AU_PER_LIGHT_MINUTE;
            let words = format!("{minutes}M");
            let (tw, th) = (words.chars().count() as i64 * 6 * s - s, 7 * s);
            let spot = [(at.0, at.1 + radius), (at.0, at.1 - radius)]
                .into_iter()
                .map(|p| {
                    let (x, y) = vp.to_px(p);
                    (p, (x - tw / 2, y - th / 2, tw, th))
                })
                .find(|(_, rect)| clear(*rect, &placed));
            if let Some((p, rect)) = spot {
                placed.push(rect);
                scene.push(Item::Text {
                    at: p,
                    offset: (0, 0),
                    anchor: Anchor::Centre,
                    text: words,
                    colour: LAG,
                    scale: s as usize,
                });
            }
        }
    }

    // The title strip above and the scale's words below, each cut to the
    // width.
    scene.layer();
    let fits = usize::try_from((w - 4 * s) / (6 * s)).unwrap_or(0);
    let cut = |words: &str| -> String { words.chars().take(fits).collect() };
    let title = sky
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("The sky");
    for (y, words, colour) in [
        (0, cut(title), YOU),
        (
            h - strip,
            cut(&format!(
                "log scale past {r0:.1} AU, edge {r_max:.1} AU; rings in light-minutes"
            )),
            DIM,
        ),
    ] {
        scene.push(Item::Panel {
            rect: (0, y, w, strip),
            fill: [16, 18, 24],
            border: None,
        });
        scene.push(Item::Label {
            at: (2 * s, y + 2 * s),
            text: words,
            colour,
            scale: s as usize,
        });
    }
    Some(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKY: &str = include_str!("../../../web/src/lib/fixtures/chart/sky_chart.json");

    fn sky() -> Value {
        serde_json::from_str(SKY).expect("the chart fixture")
    }

    fn count(f: &Frame, px: Rgb) -> usize {
        (0..f.width())
            .flat_map(|x| (0..f.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| f.get(x, y) == px)
            .count()
    }

    /// **No label covers a dot or another label** (the dev2 smoke, 2026-10-23:
    /// EARTH's word over MARS's dot): with Mars moved beside Earth, every
    /// text drawn keeps clear of every body's dot and of every other text,
    /// and both labels are still drawn.
    #[test]
    fn labels_keep_clear_of_dots_and_each_other() {
        let mut sky = sky();
        let bodies = sky["chart"]["bodies"].as_array_mut().unwrap();
        let earth = bodies
            .iter()
            .find(|b| b["label"] == "earth")
            .cloned()
            .unwrap();
        for b in bodies.iter_mut().filter(|b| b["label"] == "mars") {
            b["x_au"] = (earth["x_au"].as_f64().unwrap() + 0.05).into();
            b["y_au"] = (earth["y_au"].as_f64().unwrap() + 0.08).into();
        }
        for (w, h) in [(640, 230), (640, 480), (320, 220)] {
            let frame = Frame::new(w, h);
            let scene = chart_scene(&frame, &sky).unwrap();
            let vp = scene.viewport;
            let s = if w.min(h) >= 360 { 2 } else { 1 };
            let mut texts = Vec::new();
            let mut dots = Vec::new();
            for item in scene.layers.iter().flat_map(|l| &l.items) {
                match item {
                    Item::Text {
                        at,
                        offset,
                        anchor,
                        text,
                        ..
                    } => {
                        let (x, y) = vp.to_px(*at);
                        let (tw, th) = (text.chars().count() as i64 * 6 * s - s, 7 * s);
                        let (mut tx, mut ty) = (x + offset.0, y + offset.1);
                        if *anchor == Anchor::Centre {
                            tx -= tw / 2;
                            ty -= th / 2;
                        }
                        texts.push((text.clone(), (tx, ty, tw, th)));
                    }
                    Item::Marker { at, half, .. } => {
                        let (x, y) = vp.to_px(*at);
                        dots.push((x - half, y - half, 2 * half + 1, 2 * half + 1));
                    }
                    _ => {}
                }
            }
            let hit = |a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)| {
                a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
            };
            for (i, (word, r)) in texts.iter().enumerate() {
                for d in &dots {
                    assert!(!hit(*r, *d), "{w}x{h}: {word} covers a dot");
                }
                for (other, o) in texts.iter().skip(i + 1) {
                    assert!(!hit(*r, *o), "{w}x{h}: {word} covers {other}");
                }
            }
            let words: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
            assert!(words.contains(&"earth"), "{w}x{h}: {words:?}");
            assert!(words.contains(&"mars"), "{w}x{h}: {words:?}");
        }
    }

    /// The fixture chart drawn: the Sun at the centre, orbits, the viewer's
    /// body ringed, Ceres' passages to Luna, Vesta and Phobos as legs, the
    /// light-lag rings, the title — inside the PNG's palette.
    #[test]
    fn the_fixture_chart_draws_every_part_inside_the_palette() {
        let feeds: std::collections::HashMap<String, Value> =
            [(SKY_CHART_FEED.to_string(), sky())].into_iter().collect();
        let mut picture = ChartPicture::new();
        assert_eq!(picture.interval(), BUCKET_B_INTERVAL);
        let mut f = Frame::new(640, 480);
        assert!(picture.draw(&mut f, &feeds, &Look::default()));
        let scene = chart_scene(&f, &sky()).expect("a chart");
        let centre = scene.viewport.to_px((0.0, 0.0));
        assert_eq!(f.get(centre.0 as usize, centre.1 as usize), SUN);
        for (what, px, least) in [
            ("orbits", ORBIT, 400),
            ("the viewer's ring", HERE, 20),
            ("the passages", ROUTE, 60),
            ("the light-lag rings", LAG, 200),
            ("the title and labels", TEXT, 100),
        ] {
            assert!(count(&f, px) >= least, "{what}: {}", count(&f, px));
        }
        assert!(f.count_colours(256) <= 256);
        let ceres = scene
            .viewport
            .to_px((2.849_389_460_896_806_4, -0.675_418_616_084_449_5));
        assert_eq!(
            f.get(ceres.0 as usize, ceres.1 as usize),
            HERE,
            "Ceres, the viewer's"
        );
    }

    /// A refusal (no chart block) draws a card; no feed draws nothing.
    #[test]
    fn a_refusal_is_a_card_and_no_feed_is_nothing() {
        let mut picture = ChartPicture::new();
        let mut f = Frame::new(320, 128);
        let none: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        assert!(!picture.draw(&mut f, &none, &Look::default()));
        let refusal: std::collections::HashMap<String, Value> = [(
            SKY_CHART_FEED.to_string(),
            serde_json::json!({ "title": "No chart here.", "width": 0, "height": 0 }),
        )]
        .into_iter()
        .collect();
        assert!(picture.draw(&mut f, &refusal, &Look::default()));
        assert!(f.count_colours(8) <= 3);
    }

    #[cfg(feature = "encode")]
    #[test]
    fn retained_kitty_chart_scales_to_1280_and_skips_unchanged_ticks() {
        use crate::caps::CellSize;
        let feeds: std::collections::HashMap<String, Value> =
            [(SKY_CHART_FEED.to_string(), sky())].into_iter().collect();
        let mut pic = ChartPicture::new();
        let area = (1, 1, 128, 36);
        let cell = Some(CellSize {
            width: 10,
            height: 20,
        });
        let (vp, b1) = pic
            .draw_kitty(&feeds, &Look::default(), area, cell, false)
            .expect("drawn");
        assert_eq!(vp.width_px, 1280);
        let s1 = String::from_utf8_lossy(&b1);
        assert!(s1.contains("a=t,f=100,t=d,i=180"));
        assert!(s1.contains("a=p,i=180,p=80"));

        let (_, b2) = pic
            .draw_kitty(&feeds, &Look::default(), area, cell, false)
            .expect("drawn");
        assert!(b2.is_empty(), "unchanged chart emits 0 bytes");

        let (vp_fb, _) = pic
            .draw_kitty(&feeds, &Look::default(), area, None, false)
            .expect("fallback");
        assert_eq!(vp_fb.width_px, 640);
    }
}
