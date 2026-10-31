//! **The overhead** (doc 21 §7.4, cut G.12b): the cockpit's second camera,
//! bucket B's scene model (the replay's, G.8) at the cockpit picture's own
//! clock and size. North up, the pilot's own tank at the centre with its
//! heading; around it the held `arena.map`'s ground (its colour map, shaded
//! by G.7b's heights where the feed carries them, flat otherwise), the named
//! spots lettered, the frame's range bands as faint rings, the look as a
//! wedge from the tank, and every contact the frame lists in its relation's
//! colour (doc 32 cut CF.0, Q-CF-12, as the first-person picture draws it),
//! its party letter beside its health pips, the lock ringed.
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
//!
//! **The inset** (cut G.13, §7.4): the same scene drawn small —
//! [`OverheadPicture::draw_inset`] — in a corner of the first-person
//! picture, over the view and under the flight HUD: the picture's own
//! heading at the top ([`Up::Camera`], so what the pilot looks at is up and
//! the hull's ray parts from it as the tape's caret does; [`Up::North`] is
//! the camera's way), the camera's reach rule at the inset's half-side, the
//! ground flat (at under a pixel a cell relief is noise, and the
//! first-person frame's palette has little room), `N` on the rim and the
//! reach in metres in the corner, no strip and no ring metres.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::cockpit::{
    ARENA_MAP_FEED, COCKPIT_FEED, contact_health, contact_kind, contact_party, map_from_feed,
    terrain_from_feed,
};
use crate::frame::Rgb;
use crate::heightfield::Terrain;
use crate::math::Det as _;
use crate::overlay::{
    BORDER, DIM, FRIEND, GONE, HEALTH_PIPS, HOSTILE, LOCK, TEXT, WRECK, YOU, draw_card, ground_rgb,
    outline_rect,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::raycast::{Base, Biome, Ground, Map, SpriteKind};
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

/// A contact's mark colour by the viewer's relation to it (doc 32 cut CF.0,
/// Q-CF-12): friend, hostile, grey for neutral and unclassified, a wreck's.
fn relation_rgb(kind: SpriteKind) -> Rgb {
    match kind {
        SpriteKind::Hostile => HOSTILE,
        SpriteKind::Friend => FRIEND,
        SpriteKind::Unknown => UNKNOWN_CONTACT,
        SpriteKind::Wreck => WRECK,
    }
}
/// The inset's `N` sits this far inside its rim, pixels.
const RIM_INSET_PX: f64 = 6.0;

/// Which way is up in an overhead drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Up {
    /// North at the top: the camera's own way (§7.4).
    #[default]
    North,
    /// The picture's heading at the top (G.13, the HUD's inset): what the
    /// first-person picture looks at is up, the look's wedge straight up,
    /// the hull's ray parting from it where a look has swung the camera, as
    /// the heading tape's caret does.
    Camera,
}

/// How a scene is dressed: the camera's own full frame, or the inset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dress {
    /// The full camera: north up on the world's own coordinates, the ground
    /// blitted from the cached image, the rings' metres and the strip.
    Camera,
    /// The inset (G.13): `up` at the top about the tank, the ground sampled
    /// a pixel, markers at the small scale, `N` on the rim, the reach in the
    /// corner.
    Inset {
        /// Which way is up.
        up: Up,
    },
}

/// How world points land in a scene: as they are (the camera: north up on
/// the world's own coordinates, metres, the viewport placed around the
/// tank), or about the tank, turned so a bearing is at the top and scaled
/// to pixels (the inset: the viewport is then the frame itself, centred on
/// the origin, so the tank lands on its centre pixel exactly).
#[derive(Clone, Copy, Debug)]
struct Placing {
    /// The tank, metres east and north.
    you: (f64, f64),
    /// The bearing at the top, degrees, with its sine and cosine, and the
    /// pixels a metre; `None` for the camera's untouched coordinates.
    turn: Option<(f64, (f64, f64), f64)>,
}

impl Placing {
    /// A world point as the scene places it.
    fn place(&self, p: (f64, f64)) -> (f64, f64) {
        match self.turn {
            None => p,
            Some((_, (sin, cos), px_per_m)) => {
                let (dx, dy) = (p.0 - self.you.0, p.1 - self.you.1);
                (
                    (dx * cos - dy * sin) * px_per_m,
                    (dx * sin + dy * cos) * px_per_m,
                )
            }
        }
    }

    /// A length in metres as the scene measures it.
    fn len(&self, m: f64) -> f64 {
        match self.turn {
            None => m,
            Some((_, _, px_per_m)) => m * px_per_m,
        }
    }

    /// A world bearing as the scene draws it.
    fn bearing(&self, deg: f64) -> f64 {
        match self.turn {
            None => deg,
            Some((top, _, _)) => deg - top,
        }
    }
}

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

    /// **The inset** (G.13): this overhead drawn `side` pixels square with
    /// `up` at the top, into `frame` with its top-left at `(x, y)` and a
    /// border round it (`rect` as [`crate::overlay::inset_rect`] gives it);
    /// nothing outside that square and its border is touched. The caller
    /// (the cockpit picture) draws it before the flight HUD, so the tape,
    /// the lock's readout and the speed bar always draw over it.
    pub fn draw_inset(
        &mut self,
        frame: &mut Frame,
        cockpit: &Value,
        held: Option<&Value>,
        look: &Look,
        rect: (i64, i64, usize),
        up: Up,
    ) {
        let (x0, y0, side) = rect;
        let at = (x0, y0);
        if side == 0 {
            return;
        }
        self.ensure_map(held);
        let mut sub = Frame::new(side, side);
        overhead_scene(&sub, cockpit, held, look, self, Dress::Inset { up }).draw(&mut sub);
        for y in 0..side {
            for x in 0..side {
                let (fx, fy) = (at.0 + x as i64, at.1 + y as i64);
                if let (Ok(fx), Ok(fy)) = (usize::try_from(fx), usize::try_from(fy)) {
                    frame.put(fx, fy, sub.get(x, y));
                }
            }
        }
        let side = side as i64;
        outline_rect(frame, at.0 - 1, at.1 - 1, side + 2, side + 2, BORDER);
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
        let scene = overhead_scene(frame, cockpit, held, look, self, Dress::Camera);
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
/// tank turned and the look swung by `look`, dressed as `dress` says.
fn overhead_scene(
    frame: &Frame,
    cockpit: &Value,
    held: Option<&Value>,
    look: &Look,
    pic: &mut OverheadPicture,
    dress: Dress,
) -> Scene {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = match dress {
        Dress::Camera if w.min(h) >= 360 => 2,
        _ => 1,
    };
    let base_you = (
        num(cockpit.get("x_cm")) / 100.0,
        num(cockpit.get("y_cm")) / 100.0,
    );
    let you = (
        base_you.0 + look.dx_cm as f64 / 100.0,
        base_you.1 + look.dy_cm as f64 / 100.0,
    );
    let heading = num(cockpit.get("heading_mdeg"));
    let hull_deg = (heading + look.hull_offset_mdeg as f64) / 1_000.0;
    let camera_deg = (heading + look.offset_mdeg as f64) / 1_000.0;
    let sight = cockpit
        .get("sight_m")
        .and_then(Value::as_f64)
        .filter(|m| *m > 0.0)
        .unwrap_or(DEFAULT_SIGHT_M);
    let reach = (sight * VIEW_MARGIN).clamp(VIEW_BAND_M.0, VIEW_BAND_M.1);
    // The view: `reach` metres from the tank to the frame's nearer edge, the
    // frame's own shape.
    let px_per_m = (w.min(h) as f64 / 2.0) / reach;
    // How the world lands in the frame: the camera's untouched coordinates,
    // or the inset's, about the tank with `up` at the top, in pixels.
    let pl = Placing {
        you,
        turn: match dress {
            Dress::Camera => None,
            Dress::Inset { up } => {
                let top = match up {
                    Up::North => 0.0,
                    Up::Camera => camera_deg,
                };
                Some((top, top.to_radians().det_sin_cos(), px_per_m))
            }
        },
    };
    // The scene's units and where the tank is in them: the camera's are
    // metres about the tank, north up; the inset's pixels about the origin.
    let (half_w, half_h, centre) = match dress {
        Dress::Camera => (w as f64 / 2.0 / px_per_m, h as f64 / 2.0 / px_per_m, you),
        Dress::Inset { .. } => (w as f64 / 2.0, h as f64 / 2.0, (0.0, 0.0)),
    };
    let viewport = Viewport {
        world: (
            centre.0 - half_w,
            centre.1 - half_h,
            centre.0 + half_w,
            centre.1 + half_h,
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

    // The ground: the camera's is the whole map drawn once at this scale,
    // placed by its north-west corner; the inset's is sampled a pixel,
    // turned about the tank.
    if let Some(map) = pic.map.as_ref() {
        let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
        let mh = map.height() as f64 * cell_m;
        match pl.turn {
            None => {
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
            }
            Some((_, sin_cos, _)) => scene.push(Item::Image {
                at: (0, 0),
                frame: sampled_ground(map, you, sin_cos, px_per_m, (w, h)),
            }),
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
            let at = pl.place(((x + 0.5) * cell_m, (y + 0.5) * cell_m));
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

    // The range bands, faint, each with its metres at its top (the camera
    // only: the inset has no room for them).
    scene.layer();
    for r in band_rings(cockpit, reach) {
        scene.push(Item::Circle {
            centre,
            radius: pl.len(r),
            colour: RING,
            dash: Some((2, 3)),
        });
        if dress == Dress::Camera {
            scene.push(Item::Text {
                at: (you.0, you.1 + r),
                offset: (2 * s, -9 * s),
                anchor: Anchor::TopLeft,
                text: format!("{}", r.round() as i64),
                colour: RING,
                scale: s as usize,
            });
        }
    }

    // The look: a wedge from the tank as wide as the first-person picture
    // sees, as long as the sight.
    let edge = sight.min(reach);
    for side in [-LOOK_HALF_DEG, LOOK_HALF_DEG] {
        scene.push(Item::Line {
            a: centre,
            b: pl.place(at_bearing(you, camera_deg + side, edge)),
            colour: WEDGE,
            dash: Some((4, 4)),
        });
    }
    scene.push(Item::Arc {
        centre,
        radius: pl.len(edge),
        start_deg: pl.bearing(camera_deg) - LOOK_HALF_DEG,
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
                pl.place(at_bearing(
                    base_you,
                    num(c.get("bearing_deg")),
                    num(c.get("range_m")),
                )),
                *c,
            )
        })
        .collect();
    for (at, c) in &placed {
        let kind = contact_kind(c);
        let colour = relation_rgb(kind);
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
                bearing_deg: pl.bearing(b) as f32,
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
        // The designators: the camera's only. At the inset's scale they
        // crowd the tank (a contact 200 m out is 15 pixels off), and they
        // are on the tape and the lock's readout already. The health pips
        // (G.14) under the designator, the camera's only for the same
        // reason — the inset's contacts carry theirs over their sprites in
        // the picture itself. The party letter (doc 32 cut CF.0, Q-CF-12)
        // right of the pips, centred on them, in the relation's colour, so
        // parties stay distinct inside one relation colour.
        if dress == Dress::Camera {
            scene.push(Item::Text {
                at: *at,
                offset: (6 * s, -10 * s),
                anchor: Anchor::TopLeft,
                text: name.unwrap_or("?").to_ascii_uppercase(),
                colour: if locked { LOCK } else { TEXT },
                scale: s as usize,
            });
            if let Some(health) = contact_health(c) {
                scene.push(Item::Pips {
                    at: *at,
                    offset: (6 * s, -s),
                    lit: health.lit(),
                    of: HEALTH_PIPS,
                    size: 2 * s,
                    colour: health.rgb(),
                    dim: GONE,
                });
                if let Some(letter) = contact_party(c) {
                    scene.push(Item::Text {
                        at: *at,
                        offset: (16 * s, -3 * s),
                        anchor: Anchor::TopLeft,
                        text: letter.to_string(),
                        colour: relation_rgb(contact_kind(c)),
                        scale: s as usize,
                    });
                }
            }
        }
    }

    // The tank itself, over everything: its heading as the helm has it, an
    // unreached order ticked beside it.
    if let Some(ordered) = look.ordered_mdeg {
        scene.push(Item::Ray {
            at: centre,
            bearing_deg: pl.bearing(ordered as f64 / 1_000.0) as f32,
            len_px: 10.0 * s as f32,
            colour: LOCK,
        });
    }
    scene.push(Item::Ray {
        at: centre,
        bearing_deg: pl.bearing(hull_deg).rem_euclid(360.0) as f32,
        len_px: 14.0 * s as f32,
        colour: YOU,
    });
    scene.push(Item::Marker {
        at: centre,
        half: 3 * s,
        shape: Shape::Disc,
        colour: YOU,
    });

    match dress {
        // The strip: what this is, which way is north, the view's reach.
        Dress::Camera => scene.push(Item::Label {
            at: (3 * s, 3 * s),
            text: format!("OVERHEAD  N UP  {} M", reach.round() as i64),
            colour: TEXT,
            scale: s as usize,
        }),
        // `N` on the rim where north lies, the reach in the corner.
        Dress::Inset { .. } => {
            let rim_m = (w.min(h) as f64 / 2.0 - RIM_INSET_PX) / px_per_m;
            scene.push(Item::Text {
                at: pl.place((you.0, you.1 + rim_m)),
                offset: (0, 0),
                anchor: Anchor::Centre,
                text: "N".to_string(),
                colour: TEXT,
                scale: 1,
            });
            let reach_m = format!("{}M", reach.round() as i64);
            scene.push(Item::Label {
                at: (w - reach_m.len() as i64 * 6 - 1, h - 9),
                text: reach_m,
                colour: TEXT,
                scale: 1,
            });
        }
    }
    scene
}

/// The inset's ground: a `(w, h)` image, every pixel the colour of the cell
/// under it with `(sin, cos)`'s bearing at the top about `you` at the
/// image's centre, [`OFF_MAP`] off the map. Flat: at under a pixel a cell
/// relief shading is noise, and the first-person frame's palette has little
/// room beside the raycaster's closed set.
pub(crate) fn biome_ground_rgb(biome: Biome, g: Ground) -> Rgb {
    if biome == Biome::Temperate {
        return ground_rgb(g);
    }
    match g {
        Ground::Wall => Base::WallLit.rgb_for_biome(biome),
        Ground::Woods => Base::Woods.rgb_for_biome(biome),
        Ground::WoodsHeavy => Base::WoodsHeavy.rgb_for_biome(biome),
        _ => Base::floor(g, false).rgb_for_biome(biome),
    }
}

fn sampled_ground(
    map: &Map,
    you: (f64, f64),
    (sin, cos): (f64, f64),
    px_per_m: f64,
    (w, h): (i64, i64),
) -> Frame {
    let biome = map.biome();
    let cell_m = f64::from(map.cell_cm().max(1)) / 100.0;
    let (cols, rows) = (map.width() as i64, map.height() as i64);
    let mut image = Frame::new(w.max(0) as usize, h.max(0) as usize);
    for py in 0..h {
        for px in 0..w {
            // The pixel's centre, metres right of and up from the tank in
            // the turned frame, then back to east and north.
            let dx = (px as f64 + 0.5 - w as f64 / 2.0) / px_per_m;
            let dy = (h as f64 / 2.0 - py as f64 - 0.5) / px_per_m;
            let east = you.0 + dx * cos + dy * sin;
            let north = you.1 - dx * sin + dy * cos;
            let (cx, cy) = (
                (east / cell_m).floor() as i64,
                (north / cell_m).floor() as i64,
            );
            let rgb = if (0..cols).contains(&cx) && (0..rows).contains(&cy) {
                biome_ground_rgb(biome, map.get(cx, cy))
            } else {
                OFF_MAP
            };
            image.put(px as usize, py as usize, rgb);
        }
    }
    image
}

/// The whole of `map` drawn at `px_per_m`, north up, its first row the
/// map's northern edge: a cell's ground colour, shaded by `terrain`'s relief
/// where it is held (lit from the north-west, in [`SHADES`] steps) and flat
/// where it is not.
fn ground_image(map: &Map, terrain: Option<&Terrain>, px_per_m: f64) -> Frame {
    let biome = map.biome();
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
            let base = biome_ground_rgb(biome, map.get(cx, cy));
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
pub(crate) fn relief(t: &Terrain, x: i64, y: i64) -> i64 {
    let rise = i64::from(t.elev_cm(x, y)) - i64::from(t.elev_cm(x - 1, y + 1));
    (rise / SHADE_STEP_CM).clamp(-(SHADES / 2), SHADES / 2)
}

/// `rgb` a step of relief lighter or darker (a tenth a step).
pub(crate) fn shade(rgb: Rgb, step: i64) -> Rgb {
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
            FRIEND,
            HOSTILE,
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
        assert!(has(&bare, FRIEND));
        let mut frame = Frame::new(320, 220);
        let empty: HashMap<String, Value> = HashMap::new();
        assert!(!OverheadPicture::new().draw(&mut frame, &empty, &Look::default()));
        let mut out = feeds(true);
        if let Some(c) = out.get_mut(COCKPIT_FEED) {
            c["seated"] = false.into();
        }
        assert!(OverheadPicture::new().draw(&mut frame, &out, &Look::default()));
        assert!(!has(&frame, FRIEND), "a card, no contacts");
    }

    /// The inset (G.13): the overhead drawn small into a corner, nothing
    /// outside its square and border touched; the tank at the centre with
    /// its ray straight up when the picture's heading is up (the fixture
    /// faces 045), north-east-ish when north is; the contacts and the ground
    /// in it; a small palette; and the same feeds draw the same.
    #[test]
    fn the_inset_is_the_overhead_small_and_turned() {
        let f = feeds(true);
        let draw = |up: Up, look: &Look| {
            let mut frame = Frame::new(640, 256);
            OverheadPicture::new().draw_inset(
                &mut frame,
                f.get(COCKPIT_FEED).expect("cockpit"),
                f.get(ARENA_MAP_FEED),
                look,
                (8, 8, 104),
                up,
            );
            frame
        };
        let camera = draw(Up::Camera, &Look::default());
        let (cx, cy) = (8 + 52, 8 + 52);
        assert_eq!(camera.get(cx, cy), YOU, "the tank at the centre");
        for d in 4..12 {
            assert_eq!(camera.get(cx, cy - d), YOU, "its ray straight up, {d}");
        }
        assert_ne!(camera.get(cx + 6, cy - 6), YOU);
        let north = draw(Up::North, &Look::default());
        assert_eq!(north.get(cx, cy), YOU);
        assert_eq!(north.get(cx + 6, cy - 6), YOU, "045: up and right");
        assert_ne!(north.get(cx, cy - 10), YOU);
        for frame in [&camera, &north] {
            let inside = |c: Rgb| (8..112).any(|y| (8..112).any(|x| frame.get(x, y) == c));
            for want in [
                FRIEND,
                HOSTILE,
                UNKNOWN_CONTACT,
                LOCK,
                WEDGE,
                TEXT,
                ground_rgb(crate::raycast::Ground::Grass),
            ] {
                assert!(inside(want), "{want:?} drawn");
            }
            assert_eq!(frame.get(7, 7), BORDER, "the border round it");
            // Outside the square and its border the frame is as it was.
            for (x, y) in [(6, 6), (113, 113), (6, 60), (60, 113), (300, 100)] {
                assert_eq!(frame.get(x, y), [0, 0, 0], "({x}, {y}) untouched");
            }
            assert!(frame.count_colours(64) <= 40, "a small palette");
        }
        // A look swings the camera-up inset and leaves the tank's ray where
        // the hull is; north-up keeps the ground and swings the wedge.
        let look = Look {
            offset_mdeg: 90_000,
            ..Look::default()
        };
        let swung = draw(Up::Camera, &look);
        assert_ne!(swung.hash(), camera.hash());
        assert_eq!(
            swung.get(cx - 8, cy),
            YOU,
            "the hull's ray, now to the left"
        );
        assert_eq!(
            draw(Up::Camera, &look).hash(),
            swung.hash(),
            "deterministic"
        );
    }

    /// Doc 21 cut G.14: the camera marks an identified contact's health
    /// beside its mark, under the designator — the fixture's BA one red pip
    /// and two grey; RB (no word), X3 and X4 (unidentified) nothing — and
    /// draws none once the word is gone; the inset draws none, as it draws
    /// no designators.
    #[test]
    fn the_overhead_marks_a_contact_s_health_beside_it() {
        use crate::overlay::{BAD, GONE};
        let count = |f: &Frame, c: Rgb| {
            (0..f.height())
                .flat_map(|y| (0..f.width()).map(move |x| (x, y)))
                .filter(|&(x, y)| f.get(x, y) == c)
                .collect::<Vec<_>>()
        };
        let f = feeds(true);
        let full = draw(&f, &Look::default(), 640, 480);
        let red = count(&full, BAD);
        let grey = count(&full, GONE);
        assert_eq!(red.len(), 16, "one 4x4 pip at 2x: {red:?}");
        assert_eq!(grey.len(), 32, "two unlit: {grey:?}");
        let (x, y) = red[0];
        assert!(grey.contains(&(x + 6, y)) && grey.contains(&(x + 12, y)));
        // Beside BA's mark: right of it, and under its designator. BA is
        // hostile to the fixture's red pilot (doc 32 cut CF.0, Q-CF-12): its
        // mark is the hostile colour, and so is its party letter `B`, right
        // of the pips — the mark is the hostile pixels left of them.
        let hostile = count(&full, HOSTILE);
        let mark: Vec<_> = hostile.iter().copied().filter(|&(hx, _)| hx < x).collect();
        assert!(!mark.is_empty(), "BA's mark");
        let (bx, by) = mark
            .iter()
            .fold((0, 0), |(sx, sy), &(x, y)| (sx + x, sy + y));
        let (bx, by) = (bx / mark.len(), by / mark.len());
        let letter: Vec<_> = hostile
            .iter()
            .copied()
            .filter(|&(hx, hy)| hx >= x + 16 && hx < x + 30 && hy + 8 >= y && hy < y + 12)
            .collect();
        assert!(
            letter.len() > 20,
            "BA's party letter right of its pips: {letter:?}"
        );
        let (lx0, ly0) = letter
            .iter()
            .fold((usize::MAX, usize::MAX), |(a, b), &(x, y)| {
                (a.min(x), b.min(y))
            });
        assert_eq!(
            (lx0, ly0),
            (x + 20, y - 4),
            "the B's top-left, 2s past the pips"
        );
        assert!(
            x > bx && x < bx + 24,
            "right of BA ({bx}, {by}): ({x}, {y})"
        );
        assert!(
            y + 2 >= by && y <= by + 4,
            "level with BA ({bx}, {by}): ({x}, {y})"
        );

        let mut quiet = feeds(true);
        if let Some(c) = quiet.get_mut(COCKPIT_FEED) {
            c["contacts"][0]["health"] = Value::Null;
        }
        let none = draw(&quiet, &Look::default(), 640, 480);
        assert!(!has(&none, BAD) && !has(&none, GONE), "no word, no pips");

        let mut inset = Frame::new(640, 256);
        OverheadPicture::new().draw_inset(
            &mut inset,
            f.get(COCKPIT_FEED).expect("cockpit"),
            f.get(ARENA_MAP_FEED),
            &Look::default(),
            (8, 8, 104),
            Up::Camera,
        );
        assert!(!has(&inset, BAD), "the inset carries no pips");
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
