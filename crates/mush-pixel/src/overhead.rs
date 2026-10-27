//! **The overhead** (doc 21 §7.4, cut G.12b): the cockpit's second camera,
//! bucket B's scene model (the replay's, G.8) at the cockpit picture's own
//! clock and size. North up, the pilot's own tank at the centre with its
//! heading; around it the held `arena.map`'s ground (its colour map, shaded
//! by G.7b's heights where the feed carries them, flat otherwise), the named
//! spots lettered, the frame's range bands as faint rings, the look as a
//! wedge from the tank, and every contact the frame lists in its team's
//! colour (doc 22 D-22-3, as the first-person picture draws it), the lock
//! ringed.
//!
//! **It draws only what the frame says** (D-21-14): the contacts are the ones
//! the sim detected, placed where the frame's bearing and coarse range put
//! them, and a contact's heading is drawn only where the frame carries one
//! (`heading_deg` on the row; today's frame does not, so none is guessed off
//! the `aspect` word). The tank's own heading is the helm's, as the
//! first-person picture turns it: the frame's carried by the viewer's
//! prediction ([`Look::hull_offset_mdeg`]), and an unreached order ticked.
//!
//! The ground is drawn once a map, size and scale into an image the scene
//! places under the tank, so a tank driving across it costs a blit a frame.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::cockpit::{
    ARENA_MAP_FEED, COCKPIT_FEED, contact_kind, map_from_feed, terrain_from_feed,
};
use crate::frame::Rgb;
use crate::heightfield::Terrain;
use crate::math::Det as _;
use crate::overlay::{BLUE_TEAM, DIM, LOCK, RED_TEAM, TEXT, WRECK, YOU, draw_card, ground_rgb};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::raycast::{Map, SpriteKind};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// The sight assumed when the frame names none (`sight_m`), metres: the
/// stock tank's.
pub const DEFAULT_SIGHT_M: f64 = 600.0;
/// How far past the sight ring the view reaches, so the ring and a contact
/// on it sit inside the frame.
const VIEW_MARGIN: f64 = 1.15;
/// The view's radius is kept inside this band, metres.
const VIEW_BAND_M: (f64, f64) = (150.0, 3_000.0);
/// Half the first-person picture's field of view, degrees (the raycaster's
/// camera plane, 0.66, is about 33°): the look's wedge.
pub const LOOK_HALF_DEG: f64 = 33.0;
/// How many steps of relief shading the ground is drawn in (each a colour a
/// ground kind, so the PNG stays indexed).
const SHADES: i64 = 5;
/// A height step between neighbour cells worth one shade, centimetres.
const SHADE_STEP_CM: i64 = 120;

const RING: Rgb = [74, 80, 94];
const WEDGE: Rgb = [196, 196, 120];
const UNKNOWN_CONTACT: Rgb = [170, 170, 162];
const OFF_MAP: Rgb = [10, 12, 16];

/// The overhead as a [`Picture`]: drawn from the cockpit frame and the arena
/// map on bucket A's clock (the tank moves; the frame hash keeps an idle
/// picture off the wire). Keeps the map it built and the ground drawn at the
/// last scale.
#[derive(Debug, Default)]
pub struct OverheadPicture {
    /// What `map` was built from: the held `arena.map`'s match.
    built_for: Option<Option<i64>>,
    /// The held map's cells, if any.
    map: Option<Map>,
    /// Its heights, when the feed carries them (G.7b).
    terrain: Option<Terrain>,
    /// The ground drawn at a scale, and what it was drawn from.
    ground: Option<(GroundKey, Frame)>,
}

/// What a drawn ground image was drawn from.
#[derive(Clone, Debug, PartialEq)]
struct GroundKey {
    built_for: Option<i64>,
    px_per_m: f64,
}

impl OverheadPicture {
    /// An overhead with no map built yet.
    #[must_use]
    pub fn new() -> OverheadPicture {
        OverheadPicture::default()
    }

    fn ensure_map(&mut self, map: Option<&Value>) {
        let wanted = map.and_then(|m| m.get("match").and_then(Value::as_i64));
        if self.built_for == Some(wanted) && (map.is_some() || wanted.is_none()) {
            return;
        }
        self.map = map.map(map_from_feed);
        self.terrain = map.and_then(terrain_from_feed);
        self.built_for = Some(wanted);
        self.ground = None;
    }
}

impl Picture for OverheadPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[COCKPIT_FEED, ARENA_MAP_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(cockpit) = feeds.feed(COCKPIT_FEED) else {
            return false;
        };
        if cockpit.get("seated").and_then(Value::as_bool) == Some(false) {
            draw_card(frame, "NOT IN A COCKPIT");
            return true;
        }
        let held = feeds.feed(ARENA_MAP_FEED);
        self.ensure_map(held);
        let scene = overhead_scene(frame, cockpit, held, look, self);
        scene.draw(frame);
        true
    }
}

/// A number off the frame, `0` when absent.
fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// `(east, north)` metres `range_m` from `from` on the absolute bearing
/// `bearing_deg` (clockwise from north), as the frame places a contact.
fn at_bearing(from: (f64, f64), bearing_deg: f64, range_m: f64) -> (f64, f64) {
    let (sin, cos) = bearing_deg.to_radians().det_sin_cos();
    (from.0 + range_m * sin, from.1 + range_m * cos)
}

/// The distances the frame's range bands end at, metres, nearest first: each
/// weapon's `bands_cm` edges, and the platform's own sight. Distinct, and
/// only the ones a ring of `reach` metres could show.
#[must_use]
pub fn band_rings(cockpit: &Value, reach: f64) -> Vec<f64> {
    let mut rings: Vec<f64> = cockpit
        .get("weapons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| w.get("bands_cm").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_f64)
        .map(|cm| cm / 100.0)
        .collect();
    if let Some(sight) = cockpit.get("sight_m").and_then(Value::as_f64) {
        rings.push(sight);
    }
    rings.retain(|r| *r > 0.0 && *r <= reach * 1.5);
    rings.sort_by(f64::total_cmp);
    rings.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    rings
}

/// The overhead's scene for `frame`: `cockpit` drawn over `held` (the
/// `arena.map` keyframe, with `pic`'s map and heights built from it), the
/// tank turned and the look swung by `look`.
fn overhead_scene(
    frame: &Frame,
    cockpit: &Value,
    held: Option<&Value>,
    look: &Look,
    pic: &mut OverheadPicture,
) -> Scene {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    let you = (
        num(cockpit.get("x_cm")) / 100.0,
        num(cockpit.get("y_cm")) / 100.0,
    );
    let sight = cockpit
        .get("sight_m")
        .and_then(Value::as_f64)
        .filter(|m| *m > 0.0)
        .unwrap_or(DEFAULT_SIGHT_M);
    let reach = (sight * VIEW_MARGIN).clamp(VIEW_BAND_M.0, VIEW_BAND_M.1);
    // The view: `reach` metres from the tank to the frame's nearer edge, the
    // frame's own shape, north up.
    let px_per_m = (w.min(h) as f64 / 2.0) / reach;
    let (half_w, half_h) = (w as f64 / 2.0 / px_per_m, h as f64 / 2.0 / px_per_m);
    let viewport = Viewport {
        world: (
            you.0 - half_w,
            you.1 - half_h,
            you.0 + half_w,
            you.1 + half_h,
        ),
        frame: (0, 0, w, h),
        transform: Transform::Linear,
    };
    let mut scene = Scene::new(viewport);
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: OFF_MAP,
        border: None,
    });

    // The ground: the whole map drawn once at this scale, placed by its
    // north-west corner.
    if let Some(map) = pic.map.as_ref() {
        let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
        let mh = map.height() as f64 * cell_m;
        let key = GroundKey {
            built_for: pic.built_for.flatten(),
            px_per_m,
        };
        if pic.ground.as_ref().is_none_or(|(k, _)| *k != key) {
            let image = ground_image(map, pic.terrain.as_ref(), px_per_m);
            pic.ground = Some((key, image));
        }
        if let Some((_, image)) = pic.ground.as_ref() {
            let at = viewport.to_px((0.0, mh));
            scene.push(Item::Image {
                at,
                frame: image.clone(),
            });
        }
        for spot in held
            .and_then(|m| m.get("spots"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let cell = spot.get("cell").and_then(Value::as_array);
            let n = |i: usize| cell.and_then(|c| c.get(i)).and_then(Value::as_f64);
            let (Some(x), Some(y)) = (n(0), n(1)) else {
                continue;
            };
            let at = ((x + 0.5) * cell_m, (y + 0.5) * cell_m);
            scene.push(Item::Marker {
                at,
                half: 5 * s,
                shape: Shape::Ring,
                colour: DIM,
            });
            scene.push(Item::Text {
                at,
                offset: (0, 0),
                anchor: Anchor::Centre,
                text: spot
                    .get("letter")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                colour: TEXT,
                scale: s as usize,
            });
        }
    }

    // The range bands, faint, each with its metres at its top.
    scene.layer();
    for r in band_rings(cockpit, reach) {
        scene.push(Item::Circle {
            centre: you,
            radius: r,
            colour: RING,
            dash: Some((2, 3)),
        });
        scene.push(Item::Text {
            at: (you.0, you.1 + r),
            offset: (2 * s, -9 * s),
            anchor: Anchor::TopLeft,
            text: format!("{}", r.round() as i64),
            colour: RING,
            scale: s as usize,
        });
    }

    // The look: a wedge from the tank as wide as the first-person picture
    // sees, as long as the sight.
    let heading = num(cockpit.get("heading_mdeg"));
    let hull_deg = (heading + look.hull_offset_mdeg as f64) / 1_000.0;
    let camera_deg = (heading + look.offset_mdeg as f64) / 1_000.0;
    let edge = sight.min(reach);
    for side in [-LOOK_HALF_DEG, LOOK_HALF_DEG] {
        scene.push(Item::Line {
            a: you,
            b: at_bearing(you, camera_deg + side, edge),
            colour: WEDGE,
            dash: Some((4, 4)),
        });
    }
    scene.push(Item::Arc {
        centre: you,
        radius: edge,
        start_deg: camera_deg - LOOK_HALF_DEG,
        sweep_deg: 2.0 * LOOK_HALF_DEG,
        colour: WEDGE,
        filled: false,
    });

    // The contacts, the lock ringed, the designators over them.
    scene.layer();
    let lock = cockpit.get("lock").and_then(Value::as_str);
    let lock_name = cockpit.get("lock_designator").and_then(Value::as_str);
    let contacts: Vec<&Value> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let placed: Vec<((f64, f64), &Value)> = contacts
        .iter()
        .map(|c| {
            (
                at_bearing(you, num(c.get("bearing_deg")), num(c.get("range_m"))),
                *c,
            )
        })
        .collect();
    for (at, c) in &placed {
        let kind = contact_kind(c);
        let colour = match kind {
            SpriteKind::Red => RED_TEAM,
            SpriteKind::Blue => BLUE_TEAM,
            SpriteKind::Unknown => UNKNOWN_CONTACT,
            SpriteKind::Wreck => WRECK,
        };
        if kind == SpriteKind::Wreck {
            scene.push(Item::Marker {
                at: *at,
                half: 2 * s,
                shape: Shape::Square,
                colour,
            });
            continue;
        }
        if let Some(b) = c.get("heading_deg").and_then(Value::as_f64) {
            scene.push(Item::Ray {
                at: *at,
                bearing_deg: b as f32,
                len_px: 8.0 * s as f32,
                colour,
            });
        }
        scene.push(Item::Marker {
            at: *at,
            half: 3 * s,
            shape: Shape::Disc,
            colour,
        });
    }
    for (at, c) in &placed {
        let id = c.get("id").and_then(Value::as_str);
        let name = c.get("designator").and_then(Value::as_str);
        let locked = (lock.is_some() && id == lock) || (lock_name.is_some() && name == lock_name);
        if locked {
            scene.push(Item::Marker {
                at: *at,
                half: 7 * s,
                shape: Shape::Ring,
                colour: LOCK,
            });
        }
        scene.push(Item::Text {
            at: *at,
            offset: (6 * s, -10 * s),
            anchor: Anchor::TopLeft,
            text: name.unwrap_or("?").to_ascii_uppercase(),
            colour: if locked { LOCK } else { TEXT },
            scale: s as usize,
        });
    }

    // The tank itself, over everything: its heading as the helm has it, an
    // unreached order ticked beside it.
    if let Some(ordered) = look.ordered_mdeg {
        scene.push(Item::Ray {
            at: you,
            bearing_deg: (ordered as f64 / 1_000.0) as f32,
            len_px: 10.0 * s as f32,
            colour: LOCK,
        });
    }
    scene.push(Item::Ray {
        at: you,
        bearing_deg: hull_deg.rem_euclid(360.0) as f32,
        len_px: 14.0 * s as f32,
        colour: YOU,
    });
    scene.push(Item::Marker {
        at: you,
        half: 3 * s,
        shape: Shape::Disc,
        colour: YOU,
    });

    // The strip: what this is, which way is north, the view's reach.
    scene.push(Item::Label {
        at: (3 * s, 3 * s),
        text: format!("OVERHEAD  N UP  {} M", reach.round() as i64),
        colour: TEXT,
        scale: s as usize,
    });
    scene
}

/// The whole of `map` drawn at `px_per_m`, north up, its first row the
/// map's northern edge: a cell's ground colour, shaded by `terrain`'s relief
/// where it is held (lit from the north-west, in [`SHADES`] steps) and flat
/// where it is not.
fn ground_image(map: &Map, terrain: Option<&Terrain>, px_per_m: f64) -> Frame {
    let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
    let px_per_cell = (cell_m * px_per_m).max(0.01);
    let fw = (map.width() as f64 * px_per_cell).ceil().max(1.0) as usize;
    let fh = (map.height() as f64 * px_per_cell).ceil().max(1.0) as usize;
    let mut image = Frame::new(fw, fh);
    let rows = map.height() as i64;
    for py in 0..fh {
        let cy = rows - 1 - (py as f64 / px_per_cell) as i64;
        for px in 0..fw {
            let cx = (px as f64 / px_per_cell) as i64;
            let base = ground_rgb(map.get(cx, cy));
            let rgb = match terrain {
                Some(t) => shade(base, relief(t, cx, cy)),
                None => base,
            };
            image.put(px, py, rgb);
        }
    }
    image
}

/// How lit a cell is, `-SHADES/2..=SHADES/2`: its height over its
/// north-west neighbour's, in [`SHADE_STEP_CM`] steps.
fn relief(t: &Terrain, x: i64, y: i64) -> i64 {
    let rise = i64::from(t.elev_cm(x, y)) - i64::from(t.elev_cm(x - 1, y + 1));
    (rise / SHADE_STEP_CM).clamp(-(SHADES / 2), SHADES / 2)
}

/// `rgb` a step of relief lighter or darker (a tenth a step).
fn shade(rgb: Rgb, step: i64) -> Rgb {
    let f = 10 + step;
    rgb.map(|c| u8::try_from((i64::from(c) * f / 10).clamp(0, 255)).unwrap_or(u8::MAX))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "cockpit" => include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
            _ => include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        };
        serde_json::from_str(text).expect("fixture")
    }

    fn feeds(map: bool) -> HashMap<String, Value> {
        let mut f = HashMap::new();
        f.insert(COCKPIT_FEED.to_string(), fixture("cockpit"));
        if map {
            f.insert(ARENA_MAP_FEED.to_string(), fixture("map"));
        }
        f
    }

    fn draw(f: &HashMap<String, Value>, look: &Look, w: usize, h: usize) -> Frame {
        let mut frame = Frame::new(w, h);
        assert!(OverheadPicture::new().draw(&mut frame, f, look));
        frame
    }

    fn has(f: &Frame, c: Rgb) -> bool {
        (0..f.height()).any(|y| (0..f.width()).any(|x| f.get(x, y) == c))
    }

    /// The fixture: the tank white at the centre, the blue and red contacts
    /// and the unknown ones in their colours, the lock ringed, the ground
    /// under them, a small palette.
    #[test]
    fn the_overhead_draws_the_tank_the_contacts_and_the_ground() {
        let f = draw(&feeds(true), &Look::default(), 640, 276);
        assert_eq!(f.get(320, 138), YOU, "the tank at the centre");
        for want in [
            BLUE_TEAM,
            RED_TEAM,
            UNKNOWN_CONTACT,
            LOCK,
            WEDGE,
            RING,
            ground_rgb(crate::raycast::Ground::Grass),
            ground_rgb(crate::raycast::Ground::Water),
        ] {
            assert!(has(&f, want), "{want:?} drawn");
        }
        assert!(f.count_colours(256) <= 64, "a small palette");
        // The look swings the wedge and nothing else orders anything.
        let swung = draw(
            &feeds(true),
            &Look {
                offset_mdeg: 90_000,
                ..Look::default()
            },
            640,
            276,
        );
        assert_ne!(f.hash(), swung.hash());
        // The same feeds draw the same.
        assert_eq!(
            f.hash(),
            draw(&feeds(true), &Look::default(), 640, 276).hash()
        );
    }

    /// No map: the field is dark, the tank and its contacts still drawn; no
    /// frame: nothing; out of the seat: a card.
    #[test]
    fn missing_feeds_are_drawn_or_refused_as_the_cockpit_is() {
        let bare = draw(&feeds(false), &Look::default(), 320, 220);
        assert!(!has(&bare, ground_rgb(crate::raycast::Ground::Grass)));
        assert!(has(&bare, BLUE_TEAM));
        let mut frame = Frame::new(320, 220);
        let empty: HashMap<String, Value> = HashMap::new();
        assert!(!OverheadPicture::new().draw(&mut frame, &empty, &Look::default()));
        let mut out = feeds(true);
        if let Some(c) = out.get_mut(COCKPIT_FEED) {
            c["seated"] = false.into();
        }
        assert!(OverheadPicture::new().draw(&mut frame, &out, &Look::default()));
        assert!(!has(&frame, BLUE_TEAM), "a card, no contacts");
    }

    /// The rings are the weapons' band edges and the sight, distinct, out to
    /// what the view could show.
    #[test]
    fn the_band_rings_are_the_frames_own() {
        let cockpit = fixture("cockpit");
        assert_eq!(band_rings(&cockpit, 690.0), vec![400.0, 600.0, 1_000.0]);
        assert_eq!(band_rings(&serde_json::json!({}), 690.0), Vec::<f64>::new());
    }

    /// Relief shades the ground in steps, flat ground unshaded.
    #[test]
    fn relief_shades_in_steps() {
        assert_eq!(shade([100, 100, 100], 0), [100, 100, 100]);
        assert_eq!(shade([100, 100, 100], 2), [120, 120, 120]);
        assert_eq!(shade([100, 100, 100], -2), [80, 80, 80]);
        let mut t = Terrain::new(Map::new(3, 3, 1000));
        t.set_elev_cm(1, 1, 600);
        assert_eq!(relief(&t, 1, 1), SHADES / 2, "a rise is lit");
        assert_eq!(relief(&t, 2, 0), -(SHADES / 2), "below it, shadow");
        assert_eq!(relief(&t, 0, 0), 0, "flat");
    }
}
