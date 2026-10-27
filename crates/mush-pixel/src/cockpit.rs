//! The arena cockpit as a [`Picture`] (bucket A): the `arena.cockpit` and
//! `arena.map` keyframes read into the raycaster's map, camera and sprites
//! and the overlay's inputs. This is the one place that reads those
//! frames' JSON for the picture, shared by every renderer and the fixture
//! example.
//!
//! What is drawn over the view is an [`Overlay`]: the flight HUD alone
//! (heading tape, reticle, speed bar, and since doc 21 cut G.13 the
//! overhead's inset in the top-left corner under them) by default — the
//! text cockpit of 2026-10-22 puts the doll, the weapons, the heat and the
//! contacts in terminal text under the picture, and the web's cockpit tab
//! has its own — or every instrument inside the frame, the 640x480 cockpit
//! of 2026-10-21.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::heightfield::{self, Terrain};
use crate::look::{self, Facing};
use crate::overhead::{OverheadPicture, Up};
use crate::overlay::{
    self, Facet, Heat, Hud, HudLock, LockAt, MapMark, Section, Spot, Status, TapeMark, WeaponRow,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::raycast::{
    Camera, EYE_HEIGHT_CM, Ground, Map, Sprite, SpriteKind, SpritePose, project, render,
};

/// The pilot's own frame: pose, contacts, hull, weapons, heat.
pub const COCKPIT_FEED: &str = "arena.cockpit";
/// The arena's ground: the cell grid, its legend and the named spots.
pub const ARENA_MAP_FEED: &str = "arena.map";
/// The map drawn while no `arena.map` is held: open ground, big enough that
/// the eye never sees its edge.
const OPEN_MAP_CELLS: usize = 512;
/// Which way is up in the HUD's inset (doc 21 cut G.13): the picture's own
/// heading, so what the pilot looks at is up; [`Up::North`] is the one
/// alternative, the camera's way.
pub const INSET_UP: Up = Up::Camera;

/// What a [`CockpitPicture`] draws over the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overlay {
    /// The flight HUD: heading tape, reticle, speed bar
    /// ([`overlay::draw_hud`]), the overhead's inset under them (G.13,
    /// [`OverheadPicture::draw_inset`] at [`overlay::inset_rect`]).
    #[default]
    Hud,
    /// Every instrument in the frame: minimap, heat, weapons, the armour
    /// doll and the status strip ([`draw_instruments`]).
    Instruments,
}

/// The first-person cockpit: the held arena map drawn with relief where it
/// carries heights (the heightfield over its baked colour map, doc 21 cut
/// G.7b) and by the raycaster where it does not, the contacts as billboards,
/// its [`Overlay`] drawn into the frame.
#[derive(Debug)]
pub struct CockpitPicture {
    /// What `arena` was built from: `None` before the first draw, then the
    /// held `arena.map`'s match (itself `None` for open ground).
    built_for: Option<Option<i64>>,
    /// The arena map, open ground until an `arena.map` is held.
    arena: Map,
    /// The same map with its heights, baked, when the feed carries them
    /// (G.7b): what the picture is drawn from then.
    terrain: Option<Terrain>,
    /// What is drawn over the view.
    overlay: Overlay,
    /// Draws so far: the sprites' animation clock (a moving cycle steps every
    /// [`ANIMATION_DRAWS`] draws).
    draws: usize,
    /// The HUD's inset (G.13): the overhead drawn small, with its own map.
    inset: OverheadPicture,
}

/// How many draws a moving cycle's frame is held for: three at 20 Hz is a
/// step every 150 ms.
pub const ANIMATION_DRAWS: usize = 3;

impl Default for CockpitPicture {
    fn default() -> Self {
        Self::new()
    }
}

impl CockpitPicture {
    /// A cockpit picture with no map built yet, the flight HUD over it.
    #[must_use]
    pub fn new() -> CockpitPicture {
        CockpitPicture::with(Overlay::Hud)
    }

    /// A cockpit picture with `overlay` over it.
    #[must_use]
    pub fn with(overlay: Overlay) -> CockpitPicture {
        CockpitPicture {
            built_for: None,
            arena: Map::new(OPEN_MAP_CELLS, OPEN_MAP_CELLS, 1000),
            terrain: None,
            overlay,
            draws: 0,
            inset: OverheadPicture::new(),
        }
    }

    /// Build (or keep) the arena map from the held `arena.map`.
    fn ensure_map(&mut self, map: Option<&Value>) -> &Map {
        let wanted = map.and_then(|m| m.get("match").and_then(Value::as_i64));
        let keep = self.built_for == Some(wanted) && (map.is_some() || wanted.is_none());
        if !keep {
            self.arena = map.map_or_else(
                || Map::new(OPEN_MAP_CELLS, OPEN_MAP_CELLS, 1000),
                map_from_feed,
            );
            // G.7b: relief where the feed carries it, baked once a map.
            self.terrain = map.and_then(terrain_from_feed);
            self.built_for = Some(wanted);
        }
        &self.arena
    }
}

impl Picture for CockpitPicture {
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
        // Out of the seat (the match ended, or the pilot climbed out), the
        // frame's pose is no pose: the picture says so rather than drawing
        // from the map's corner.
        if cockpit.get("seated").and_then(Value::as_bool) == Some(false) {
            overlay::draw_card(frame, "NOT IN A COCKPIT");
            return true;
        }
        let map = feeds.feed(ARENA_MAP_FEED);
        let cam = camera(cockpit, look);
        self.draws = self.draws.wrapping_add(1);
        let sprites = contacts_as_sprites(cockpit, &cam, self.draws / ANIMATION_DRAWS);
        let overlay = self.overlay;
        self.ensure_map(map);
        let arena = &self.arena;
        // G.7b: relief where the feed carries it, seen from the sim's own eye
        // for the unit.
        let terrain = self.terrain.as_ref();
        let eye = eye_cm(cockpit);
        match terrain {
            Some(terrain) => heightfield::render(frame, terrain, &cam, eye, &sprites),
            None => render(frame, arena, &cam, &sprites),
        }
        match overlay {
            // The inset first, the HUD over it (G.13): the tape, the lock's
            // readout and the speed bar are never covered.
            Overlay::Hud => {
                if let Some(rect) = overlay::inset_rect(frame) {
                    self.inset
                        .draw_inset(frame, cockpit, map, look, rect, INSET_UP);
                }
                overlay::draw_hud(frame, &hud(frame, arena, terrain, eye, cockpit, look, &cam))
            }
            // The owner's 640x480 target of 2026-10-21: minimap, heat,
            // weapons, the armour doll and the status strip, all from the
            // frame the text panes already read.
            Overlay::Instruments => draw_instruments(frame, arena, cockpit, map, &cam),
        }
        true
    }
}

/// The eye the relief is seen from (doc 21 cut G.7b): the unit's elevation
/// plus its sensor height (the stock 1.5 m where the frame names none), as
/// the sim traces its sight; `None` for a frame with no `z_cm`, which the
/// heightfield takes as the stock eye over the ground under the camera.
fn eye_cm(cockpit: &Value) -> Option<i64> {
    cockpit.get("z_cm").and_then(Value::as_i64).map(|z| {
        z + cockpit
            .get("sensor_height_cm")
            .and_then(Value::as_i64)
            .unwrap_or(i64::from(EYE_HEIGHT_CM))
    })
}

/// The camera: the sim's own pose from the cockpit frame, turned by the
/// viewer's local look.
#[must_use]
pub fn camera(cockpit: &Value, look: &Look) -> Camera {
    let heading = (num(cockpit.get("heading_mdeg")) + look.offset_mdeg).rem_euclid(360_000);
    Camera {
        x_cm: num(cockpit.get("x_cm")),
        y_cm: num(cockpit.get("y_cm")),
        heading_mdeg: u32::try_from(heading).unwrap_or(0),
    }
}

/// An `arena.map` keyframe (run-length rows `[[glyph, run], …]`, the south
/// row first, a legend of `[glyph, word]`) as a raycaster map. A run with no
/// length is one cell; a run past the width is clipped; a malformed run is
/// skipped; a short row is open ground to its end. These are the rules the
/// text renderers' `mush_ansi::window_rows` reads the same rows by.
#[must_use]
pub fn map_from_feed(map: &Value) -> Map {
    let size = |key: &str| {
        map.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    let (width, height) = (size("width"), size("height"));
    let cell_cm = map
        .get("cell_cm")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1000);
    let legend: Vec<(char, Ground)> = map
        .get("legend")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let g = pair.get(0)?.as_str()?.chars().next()?;
            let name = pair.get(1)?.as_str()?;
            Some((g, Ground::from_legend_name(name)))
        })
        .collect();
    let ground_of = |g: char| {
        legend
            .iter()
            .find(|(c, _)| *c == g)
            .map_or(Ground::Open, |(_, ground)| *ground)
    };
    let mut out = Map::new(width, height, cell_cm);
    let rows = map.get("rows").and_then(Value::as_array);
    for y in 0..height {
        let runs = rows.and_then(|r| r.get(y)).and_then(Value::as_array);
        let mut col = 0usize;
        for run in runs.into_iter().flatten() {
            if col >= width {
                break;
            }
            let Some(pair) = run.as_array() else {
                continue;
            };
            let glyph = pair.first().map_or(' ', |v| {
                v.as_str().and_then(|s| s.chars().next()).unwrap_or('?')
            });
            let n = pair
                .get(1)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(1);
            let end = col.saturating_add(n).min(width);
            let ground = ground_of(glyph);
            for x in col..end {
                out.set(x, y, ground);
            }
            col = end;
        }
        // The rest of a short row, and a missing row, is spaces.
        let rest = ground_of(' ');
        for x in col..width {
            out.set(x, y, rest);
        }
    }
    out
}

/// **The held map with its relief** (doc 21 cut G.7b): [`map_from_feed`]'s
/// cells and the feed's `elev` — every cell's height, the south row first, as
/// zigzag-LEB128 deltas in base64 (`tacsim::match_::cockpit::elev_field`) —
/// as a [`Terrain`] with its colour map baked, seeded by the map's `surface`
/// so the same ground looks the same every match and in every client. `None`
/// for a feed with no `elev`, or one that does not decode to exactly a height
/// a cell (the raycaster draws that ground flat, as before).
#[must_use]
pub fn terrain_from_feed(map: &Value) -> Option<Terrain> {
    let elev = map.get("elev")?;
    if elev.get("encoding").and_then(Value::as_str) != Some("zigzag-leb128-delta") {
        return None;
    }
    let unit = elev.get("unit_cm").and_then(Value::as_i64).unwrap_or(1);
    let bytes = base64_decode(elev.get("data")?.as_str()?)?;
    let cells = map_from_feed(map);
    let (w, h) = (cells.width(), cells.height());
    let mut heights = Vec::with_capacity(w * h);
    let (mut at, mut last) = (0usize, 0i64);
    while heights.len() < w * h {
        let (mut zig, mut shift) = (0u64, 0u32);
        loop {
            let b = *bytes.get(at)?;
            at += 1;
            zig |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 35 {
                return None;
            }
        }
        let delta = (zig >> 1) as i64 ^ -((zig & 1) as i64);
        last += delta;
        heights.push(last);
    }
    if at != bytes.len() {
        return None;
    }
    let mut terrain = Terrain::new(cells);
    for (i, cm) in heights.into_iter().enumerate() {
        let cm = i32::try_from(cm.saturating_mul(unit)).ok()?;
        terrain.set_elev_cm(i % w.max(1), i / w.max(1), cm);
    }
    let seed = map
        .get("surface")
        .and_then(Value::as_str)
        .unwrap_or("")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    terrain.bake(seed);
    Some(terrain)
}

/// Standard base64 (padding optional) to bytes; `None` for anything else.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let body = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in body.bytes() {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// How a contact is drawn (doc 22 D-22-3): a wreck, or its team's colour —
/// the frame's `class` is the team a classified contact is on (`red`,
/// `blue`) — or unknown. Never by whether it is a friend.
#[must_use]
pub fn contact_kind(c: &Value) -> SpriteKind {
    if c.get("reads_dead").and_then(Value::as_bool) == Some(true) {
        return SpriteKind::Wreck;
    }
    match c.get("class").and_then(Value::as_str) {
        Some("red") => SpriteKind::Red,
        Some("blue") => SpriteKind::Blue,
        _ => SpriteKind::Unknown,
    }
}

/// A contact as the sprite the picture draws it as: placed by bearing and
/// range from `cam`, in its type's look (doc 22 cut U.3) — the contact's
/// `unit_type` (known from classification, D-22-5) names it, an unclassified
/// contact is drawn with the [`look::UNKNOWN`] look; the face it shows is its
/// `aspect`, it plays its moving cycle while `moving`, at animation `step`. A
/// type whose look has no sheet (or that has no look) is the silhouette. The
/// HUD boxes the lock with the same sprite, so its box is the drawn one's —
/// the sheet's size, not the silhouette's (doc 21a `X-03`).
fn contact_sprite(c: &Value, cam: &Camera, step: usize) -> Sprite {
    let kind = contact_kind(c);
    let look = match c.get("unit_type").and_then(Value::as_str) {
        Some(unit) => look::for_unit(unit),
        None if kind == SpriteKind::Unknown => look::unknown(),
        None => None,
    };
    let pose = look
        .and_then(|l| l.sprites.as_ref())
        .map(|sheet| SpritePose {
            sheet,
            facing: Facing::from_aspect(c.get("aspect").and_then(Value::as_str).unwrap_or("front")),
            moving: c.get("moving").and_then(Value::as_bool) == Some(true),
            step,
        });
    Sprite {
        pose,
        ..Sprite::at_bearing(
            cam,
            c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0),
            c.get("range_m").and_then(Value::as_f64).unwrap_or(0.0),
            kind,
        )
    }
}

/// The frame's contacts as sprites, each as `contact_sprite` makes it.
#[must_use]
pub fn contacts_as_sprites(cockpit: &Value, cam: &Camera, step: usize) -> Vec<Sprite> {
    cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| contact_sprite(c, cam, step))
        .collect()
}

/// The flight HUD's inputs: the camera's heading, the hull's (the sim's
/// turned by the helm's prediction) and an ordered one from `look`, the
/// speed, the contacts for the tape, and the lock placed in the picture as
/// the picture drew it — boxed where it stands, an arrow at the top or
/// bottom edge it lies clear over, or off a side. The picture
/// is the heightfield's over `terrain` from the eye at `eye_cm` where the
/// map carries relief (doc 21 cut G.7b; both as [`heightfield::render`]
/// took them), else the raycaster's over flat `arena`; the lock is projected
/// by whichever drew it, so its box lands on the sprite (doc 21a `X-03`).
#[must_use]
pub fn hud(
    frame: &Frame,
    arena: &Map,
    terrain: Option<&Terrain>,
    eye_cm: Option<i64>,
    cockpit: &Value,
    look: &Look,
    cam: &Camera,
) -> Hud {
    let heading = num(cockpit.get("heading_mdeg"));
    let contacts = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let bearing = |c: &Value| c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0);
    let marks = contacts
        .iter()
        .map(|c| TapeMark {
            bearing_mdeg: (bearing(c) * 1_000.0).round() as i64,
            kind: contact_kind(c),
        })
        .collect();
    let lock_id = cockpit.get("lock").and_then(Value::as_str);
    let lock = contacts
        .iter()
        .find(|c| lock_id.is_some() && c.get("id").and_then(Value::as_str) == lock_id)
        .map(|c| {
            let range_m = c.get("range_m").and_then(Value::as_f64).unwrap_or(0.0);
            // The sprite as the picture draws this contact — its type's
            // sheet gives its size; the step picks the moving cycle's frame,
            // never the size — so the box is the drawn sprite's.
            let sprite = contact_sprite(c, cam, 0);
            let (w, h) = (frame.width(), frame.height());
            let placed = match terrain {
                Some(terrain) => heightfield::project(terrain, cam, eye_cm, w, h, &sprite),
                None => project(arena, cam, w, h, &sprite),
            };
            let at = match placed {
                // In the view's columns: boxed where it stands — or, its
                // whole box over the top edge or under the bottom (close
                // by, up a crest or down a steep slope, where the relief
                // puts it), an arrow at that edge in its column, so the
                // readout is never lost with the box.
                Some(p) if p.screenx >= 0.0 && p.screenx < w as f32 => {
                    let top = p.bottom - p.height;
                    if p.bottom < 0.0 {
                        LockAt::Beyond(p.screenx as i64, -1)
                    } else if top >= h as f32 {
                        LockAt::Beyond(p.screenx as i64, 1)
                    } else {
                        LockAt::Box(
                            (p.screenx - p.width / 2.0) as i64,
                            top as i64,
                            p.width.ceil() as i64,
                            p.height.ceil() as i64,
                        )
                    }
                }
                _ => {
                    let rel = (bearing(c) * 1_000.0).round() as i64 - i64::from(cam.heading_mdeg);
                    LockAt::Off(if (rel + 180_000).rem_euclid(360_000) < 180_000 {
                        -1
                    } else {
                        1
                    })
                }
            };
            HudLock {
                designator: cockpit
                    .get("lock_designator")
                    .and_then(Value::as_str)
                    .or_else(|| c.get("designator").and_then(Value::as_str))
                    .unwrap_or("?")
                    .to_string(),
                range_m: cockpit
                    .get("lock_range_m")
                    .and_then(Value::as_i64)
                    .unwrap_or(range_m.round() as i64),
                percent: cockpit.get("lock_percent").and_then(Value::as_i64),
                at,
            }
        });
    Hud {
        heading_mdeg: i64::from(cam.heading_mdeg),
        hull_mdeg: (heading + look.hull_offset_mdeg).rem_euclid(360_000),
        ordered_mdeg: look.ordered_mdeg,
        speed_kph: num(cockpit.get("speed_kph")),
        marks,
        lock,
    }
}

fn num(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

fn cell_of(v: Option<&Value>) -> Option<(i64, i64)> {
    let pair = v?.as_array()?;
    Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?))
}

/// The frame's `sections` as the doll's and the legend's inputs (doc 21 cut
/// G.6a shares it with the doll picture).
#[must_use]
pub fn sections_from(cockpit: &Value) -> Vec<Section> {
    cockpit
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|sec| Section {
            id: sec
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            condition: sec
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            internals: num(sec.get("internals")),
            internals_max: num(sec.get("internals_max")),
            facets: sec
                .get("facets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|f| Facet {
                    id: f
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                    armor: num(f.get("armor")),
                    armor_max: num(f.get("armor_max")),
                })
                .collect(),
        })
        .collect()
}

/// Draw the instruments over a rendered frame from the cockpit keyframe and
/// the held `arena.map` (for its named spots): minimap, heat, weapons, the
/// armour doll and the status strip.
pub fn draw_instruments(
    frame: &mut Frame,
    arena: &Map,
    cockpit: &Value,
    map: Option<&Value>,
    cam: &Camera,
) {
    // The minimap.
    let marks: Vec<MapMark> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(MapMark {
                cell: cell_of(c.get("cell"))?,
                kind: contact_kind(c),
            })
        })
        .collect();
    let spots: Vec<Spot> = map
        .and_then(|m| m.get("spots"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|spot| {
            Some(Spot {
                cell: cell_of(spot.get("cell"))?,
                letter: spot.get("letter")?.as_str()?.chars().next()?,
            })
        })
        .collect();
    let cell = i64::from(arena.cell_cm().max(1));
    let you = cell_of(cockpit.get("cell")).unwrap_or((cam.x_cm / cell, cam.y_cm / cell));
    overlay::draw_minimap(frame, arena, you, cam.heading_mdeg, &marks, &spots);

    // Heat: the stock tanks on dev2 carry no heat model (`heat` is null), so
    // the gauge is drawn empty with a dash rather than left out.
    let heat = cockpit.get("heat").filter(|h| !h.is_null());
    overlay::draw_heat(
        frame,
        &Heat {
            heat: num(heat.and_then(|h| h.get("heat"))),
            max: num(heat.and_then(|h| h.get("max"))),
            overheated: heat
                .and_then(|h| h.get("overheated"))
                .and_then(Value::as_bool)
                == Some(true),
        },
    );

    // Weapons.
    let rows: Vec<WeaponRow> = cockpit
        .get("weapons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| WeaponRow {
            name: w
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            ready: w.get("ready").and_then(Value::as_bool) == Some(true),
            ready_in_ms: num(w.get("ready_in_ms")),
            rounds: num(w.get("rounds")),
            rounds_max: num(w.get("rounds_max")),
            percent: w.get("percent").and_then(Value::as_u64),
        })
        .collect();
    overlay::draw_weapons(frame, &rows);

    // The doll.
    let sections = sections_from(cockpit);
    let doll = cockpit
        .get("unit_type")
        .and_then(Value::as_str)
        .and_then(crate::look::for_unit)
        .and_then(|look| look.doll.as_ref());
    let condition = doll
        .map(|d| d.condition(cockpit.get("sections").unwrap_or(&Value::Null)))
        .unwrap_or_default();
    overlay::draw_doll(frame, doll.map(|d| (d, condition.as_slice())), &sections);

    // The status strip.
    let lock = cockpit
        .get("lock_designator")
        .and_then(Value::as_str)
        .map(|who| (who.to_string(), num(cockpit.get("lock_range_m"))));
    overlay::draw_status(
        frame,
        &Status {
            callsign: cockpit
                .get("callsign")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            speed_kph: num(cockpit.get("speed_kph")),
            heading_deg: i64::from(cam.heading_mdeg / 1_000),
            lock,
            power: cockpit
                .get("power")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const COCKPIT: &str = include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");
    const MAP: &str = include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json");

    fn parsed(text: &str) -> Value {
        let parsed = serde_json::from_str::<Value>(text);
        assert!(parsed.is_ok(), "fixture: {parsed:?}");
        parsed.unwrap_or_default()
    }

    #[test]
    fn the_fixture_map_decodes_its_legend() {
        let m = map_from_feed(&parsed(MAP));
        assert_eq!((m.width(), m.height(), m.cell_cm()), (96, 64, 1000));
        // Row 2 has water at x 20..30 (the fixture's pond), grass elsewhere.
        assert_eq!(m.get(25, 2), Ground::Water);
        assert_eq!(m.get(0, 0), Ground::Grass);
    }

    /// The run-length rules `mush_ansi::window_rows` reads rows by: a run
    /// without a length is one cell, a run past the width is clipped, a
    /// malformed run is skipped, a short row pads, rows past the height are
    /// not read.
    #[test]
    fn the_map_decode_keeps_the_text_renderers_run_rules() {
        let map = serde_json::json!({
            "width": 4, "height": 2, "cell_cm": 500,
            "legend": [["#", "wall"], ["~", "water"]],
            "rows": [
                [["#"], "junk", ["~", 9]],
                [["~", 1]],
                [["#", 4]]
            ]
        });
        let m = map_from_feed(&map);
        assert_eq!(m.cell_cm(), 500);
        let row = |y| (0..4).map(|x| m.get(x, y)).collect::<Vec<_>>();
        assert_eq!(
            row(0),
            [Ground::Wall, Ground::Water, Ground::Water, Ground::Water]
        );
        assert_eq!(
            row(1),
            [Ground::Water, Ground::Open, Ground::Open, Ground::Open]
        );
        assert_eq!(
            m.get(0, 2),
            Ground::Wall,
            "outside the map, not row 2's runs"
        );
    }

    /// Through the seam: no cockpit frame, nothing drawn and the frame
    /// untouched; with one, the picture is drawn and turns with the look.
    #[test]
    fn the_cockpit_picture_draws_from_its_feeds_and_turns_with_the_look() {
        let mut feeds = std::collections::HashMap::new();
        let mut picture = CockpitPicture::new();
        assert_eq!(picture.interval(), BUCKET_A_INTERVAL);
        assert_eq!(picture.feeds(), [COCKPIT_FEED, ARENA_MAP_FEED]);
        let mut frame = Frame::new(320, 220);
        assert!(!picture.draw(&mut frame, &feeds, &Look::default()));
        assert!(frame == Frame::new(320, 220), "untouched");
        feeds.insert(COCKPIT_FEED.to_string(), parsed(COCKPIT));
        feeds.insert(ARENA_MAP_FEED.to_string(), parsed(MAP));
        assert!(picture.draw(&mut frame, &feeds, &Look::default()));
        let ahead = frame.hash();
        let look = Look {
            offset_mdeg: 30_000,
            ..Look::default()
        };
        assert!(picture.draw(&mut frame, &feeds, &look));
        assert_ne!(frame.hash(), ahead, "the look turns the camera");
        assert!(picture.draw(&mut frame, &feeds, &Look::default()));
        assert_eq!(frame.hash(), ahead, "and back");
        let cam = camera(&parsed(COCKPIT), &look);
        assert_eq!(
            cam.heading_mdeg, 75_000,
            "the fixture faces 045, the look adds 30"
        );
    }

    #[test]
    fn the_fixture_frame_draws_every_instrument_inside_the_palette() {
        let cockpit = parsed(COCKPIT);
        let mut arena = Map::new(96, 64, 1000);
        for y in 0..64 {
            for x in 0..96 {
                arena.set(x, y, Ground::Grass);
            }
        }
        let cam = Camera {
            x_cm: 6_500,
            y_cm: 4_500,
            heading_mdeg: 45_000,
        };
        let sprites = contacts_as_sprites(&cockpit, &cam, 0);
        assert_eq!(sprites.len(), 4);
        // Doc 22 D-22-3: by team, not by friend — RB (the pilot's own red
        // team) is red and BA blue.
        assert_eq!(
            sprites.iter().filter(|s| s.kind == SpriteKind::Red).count(),
            1,
            "RB, red"
        );
        assert_eq!(
            sprites
                .iter()
                .filter(|s| s.kind == SpriteKind::Blue)
                .count(),
            1,
            "BA, blue"
        );
        assert_eq!(
            sprites
                .iter()
                .filter(|s| s.kind == SpriteKind::Unknown)
                .count(),
            2,
            "X3 and X4"
        );
        let mut frame = Frame::new(640, 480);
        render(&mut frame, &arena, &cam, &sprites);
        draw_instruments(&mut frame, &arena, &cockpit, None, &cam);
        assert!(frame.count_colours(256) <= 256);
    }

    /// Doc 21 cut G.13: the flight HUD carries the overhead's inset in the
    /// top-left corner — the tank at its centre, the contacts in their team
    /// colours around it, the ground under them — at the text cockpit's
    /// 640x256 and the 640x480 target, on the flat yard and on relief, and
    /// the frame stays inside the palette; a look turns the inset (the
    /// picture's heading is up) and the instruments overlay has no inset
    /// (its own minimap is bottom-left).
    #[test]
    fn the_hud_carries_the_overhead_inset() {
        use crate::overlay::{BLUE_TEAM, RED_TEAM, YOU};
        let mut feeds = std::collections::HashMap::new();
        feeds.insert(COCKPIT_FEED.to_string(), parsed(COCKPIT));
        feeds.insert(ARENA_MAP_FEED.to_string(), parsed(MAP));
        let has_in = |f: &Frame, (x, y, side): (i64, i64, usize), c: [u8; 3]| {
            (y..y + side as i64)
                .any(|yy| (x..x + side as i64).any(|xx| f.get(xx as usize, yy as usize) == c))
        };
        for (w, h) in [(640, 256), (640, 480)] {
            let mut frame = Frame::new(w, h);
            assert!(CockpitPicture::new().draw(&mut frame, &feeds, &Look::default()));
            let rect = overlay::inset_rect(&frame).expect("an inset at {w}x{h}");
            let (x, y, side) = rect;
            let c = (x + side as i64 / 2) as usize;
            assert_eq!(
                frame.get(c, (y + side as i64 / 2) as usize),
                YOU,
                "{w}x{h}: the tank"
            );
            assert_eq!(
                frame.get(c, (y + side as i64 / 2 - 5) as usize),
                YOU,
                "{w}x{h}: its ray up"
            );
            assert!(has_in(&frame, rect, BLUE_TEAM) && has_in(&frame, rect, RED_TEAM));
            assert!(
                has_in(&frame, rect, overlay::ground_rgb(Ground::Grass)),
                "{w}x{h}: the ground"
            );
            assert!(frame.count_colours(256) <= 256, "{w}x{h}: in the palette");
            // A look swings the picture and the inset turns with it: a
            // colour of the ground (the pond) moves within the square.
            let mut swung = Frame::new(w, h);
            let look = Look {
                offset_mdeg: 90_000,
                ..Look::default()
            };
            assert!(CockpitPicture::new().draw(&mut swung, &feeds, &look));
            let square = |f: &Frame| -> Vec<[u8; 3]> {
                (y..y + side as i64)
                    .flat_map(|yy| (x..x + side as i64).map(move |xx| (xx as usize, yy as usize)))
                    .map(|(xx, yy)| f.get(xx, yy))
                    .collect()
            };
            assert_ne!(square(&frame), square(&swung), "{w}x{h}: the inset turned");
        }
        let mut instruments = Frame::new(640, 480);
        assert!(CockpitPicture::with(Overlay::Instruments).draw(
            &mut instruments,
            &feeds,
            &Look::default()
        ));
        assert_ne!(
            instruments.get(64, 64),
            YOU,
            "no inset under the instruments"
        );
    }

    /// The dev2 smoke of 2026-10-22: out of the seat the picture is a plain
    /// card, not a view from the map's corner, and it does replace the last
    /// picture sent.
    #[test]
    fn an_unseated_frame_draws_a_card() {
        let mut feeds = std::collections::HashMap::new();
        feeds.insert(
            COCKPIT_FEED.to_string(),
            serde_json::json!({ "seated": false }),
        );
        let mut frame = Frame::new(320, 128);
        assert!(CockpitPicture::new().draw(&mut frame, &feeds, &Look::default()));
        assert!(frame.count_colours(8) <= 3, "the card and its words");
    }

    /// The flight HUD (the text cockpit, 2026-10-22): the lock ahead is
    /// boxed where it stands; swung away by a look it is an arrow at the
    /// side it lies off, and the hull is marked where the look parted the
    /// camera from it; an ordered heading rides along; the HUD keeps the
    /// frame inside the palette; and the cockpit picture draws the HUD by
    /// default, the instruments on request.
    #[test]
    fn the_hud_boxes_the_lock_or_points_to_it() {
        let cockpit = parsed(COCKPIT);
        let mut arena = Map::new(96, 64, 1000);
        for y in 0..64 {
            for x in 0..96 {
                arena.set(x, y, Ground::Grass);
            }
        }
        let mut frame = Frame::new(640, 256);
        let ahead = Look::default();
        let cam = camera(&cockpit, &ahead);
        let h = hud(&frame, &arena, None, None, &cockpit, &ahead, &cam);
        assert_eq!(
            (h.heading_mdeg, h.hull_mdeg, h.speed_kph),
            (45_000, 45_000, 43)
        );
        assert_eq!(h.marks.len(), 4);
        let lock = h.lock.clone().expect("BA is locked");
        assert_eq!(
            (lock.designator.as_str(), lock.range_m, lock.percent),
            ("BA", 440, Some(62))
        );
        let LockAt::Box(x, y, w, bh) = lock.at else {
            panic!("BA at 045 is dead ahead: {:?}", lock.at);
        };
        assert!(x <= 320 && x + w >= 320, "boxed at the centre column");
        assert!(y < 128 + 8 && y + bh > 128, "on the horizon, 450 m out");

        // A look 90° right: BA lies off to the left; the hull, still at
        // 045, is marked; an order rides along.
        let look = Look {
            offset_mdeg: 90_000,
            hull_offset_mdeg: 0,
            ordered_mdeg: Some(60_000),
        };
        let cam = camera(&cockpit, &look);
        let h = hud(&frame, &arena, None, None, &cockpit, &look, &cam);
        assert_eq!((h.heading_mdeg, h.hull_mdeg), (135_000, 45_000));
        assert_eq!(h.ordered_mdeg, Some(60_000));
        assert_eq!(h.lock.as_ref().map(|l| l.at), Some(LockAt::Off(-1)));
        render(&mut frame, &arena, &cam, &[]);
        overlay::draw_hud(&mut frame, &h);
        assert!(frame.count_colours(256) <= 256);

        let mut feeds = std::collections::HashMap::new();
        feeds.insert(COCKPIT_FEED.to_string(), cockpit);
        feeds.insert(ARENA_MAP_FEED.to_string(), parsed(MAP));
        let mut with_hud = Frame::new(640, 480);
        let mut with_instruments = Frame::new(640, 480);
        assert!(CockpitPicture::new().draw(&mut with_hud, &feeds, &ahead));
        assert!(CockpitPicture::with(Overlay::Instruments).draw(
            &mut with_instruments,
            &feeds,
            &ahead
        ));
        assert_ne!(with_hud.hash(), with_instruments.hash());
    }

    /// Doc 21a `X-03`: on relief the lock box stands where the heightfield
    /// drew the sprite — on the ground at its own point, seen from the
    /// unit's own eye — not where flat ground from the stock eye would put
    /// it. The fixture's yard falling away ahead of the tank (a metre a cell
    /// each way), the tank on its own ground: BA, 440 m out, stands some
    /// 60 m below it, and its box sits well under the row the flat
    /// projection boxed it on; on level ground the two agree.
    #[test]
    fn the_lock_box_stands_on_the_relief_where_the_sprite_does() {
        let mut cockpit = parsed(COCKPIT);
        let arena = map_from_feed(&parsed(MAP));
        let mut terrain = Terrain::new(arena.clone());
        for y in 0..arena.height() {
            for x in 0..arena.width() {
                terrain.set_elev_cm(x, y, -((x + y) as i32) * 100);
            }
        }
        // The tank on its own ground (the frame's `z_cm`), its eye the
        // stock 1.5 m over it.
        cockpit["z_cm"] = serde_json::json!(terrain.elev_cm(6, 4));
        let eye = eye_cm(&cockpit);
        assert_eq!(eye, Some(-1_000 + 150));
        let frame = Frame::new(640, 480);
        let look = Look::default();
        let cam = camera(&cockpit, &look);
        let boxed = |h: &Hud| match h.lock.as_ref().expect("BA is locked").at {
            LockAt::Box(x, y, w, h) => (x, y, w, h),
            at => panic!("BA dead ahead is boxed: {at:?}"),
        };
        let flat = boxed(&hud(&frame, &arena, None, None, &cockpit, &look, &cam));
        let relief = boxed(&hud(
            &frame,
            &arena,
            Some(&terrain),
            eye,
            &cockpit,
            &look,
            &cam,
        ));
        assert_eq!(
            (flat.0, flat.2),
            (relief.0, relief.2),
            "dead ahead at the same range: the same column and width"
        );
        assert!(
            relief.1 + relief.3 >= flat.1 + flat.3 + 30,
            "down the slope the box drops: flat bottom {}, relief bottom {}",
            flat.1 + flat.3,
            relief.1 + relief.3
        );
        // Exactly where the heightfield draws the sprite.
        let c = cockpit["contacts"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["id"] == cockpit["lock"]))
            .expect("the locked contact");
        let sprite = contact_sprite(c, &cam, 0);
        let p = heightfield::project(&terrain, &cam, eye, 640, 480, &sprite).expect("in view");
        assert_eq!(
            relief,
            (
                (p.screenx - p.width / 2.0) as i64,
                (p.bottom - p.height) as i64,
                p.width.ceil() as i64,
                p.height.ceil() as i64
            )
        );
        // Level ground from the same eye: the heightfield and the raycaster
        // box it alike (the fixture yard's all-zero heights draw the same
        // picture either way).
        let level = Terrain::new(arena.clone());
        cockpit["z_cm"] = serde_json::json!(0);
        let eye = eye_cm(&cockpit);
        assert_eq!(
            boxed(&hud(
                &frame,
                &arena,
                Some(&level),
                eye,
                &cockpit,
                &look,
                &cam
            )),
            flat
        );
    }

    /// Doc 21a `X-03`, review: the lock box is the drawn sprite's box. BA is
    /// a line tank, drawn from the tracked tank's sheet 7.2 m wide and 3 m
    /// tall, and its box has that shape — not the 5 m by 3.5 m silhouette's
    /// the HUD built its own sprite with, a third narrower and taller than
    /// the tank under it. BA brought in to 100 m, so the two boxes differ in
    /// whole pixels.
    #[test]
    fn the_lock_box_is_the_drawn_sprite_s() {
        let mut cockpit = parsed(COCKPIT);
        cockpit["contacts"][0]["range_m"] = serde_json::json!(100);
        let arena = map_from_feed(&parsed(MAP));
        let frame = Frame::new(640, 480);
        let look = Look::default();
        let cam = camera(&cockpit, &look);
        let flight = hud(&frame, &arena, None, None, &cockpit, &look, &cam);
        let LockAt::Box(_, _, w, bh) = flight.lock.expect("BA is locked").at else {
            panic!("BA dead ahead is boxed");
        };
        let c = &cockpit["contacts"][0];
        assert_eq!(c["designator"], "BA");
        let drawn = project(&arena, &cam, 640, 480, &contact_sprite(c, &cam, 0)).expect("in view");
        assert!(drawn.pose.is_some(), "drawn from its sheet");
        assert_eq!(
            (w, bh),
            (drawn.width.ceil() as i64, drawn.height.ceil() as i64)
        );
        let silhouette = Sprite::at_bearing(&cam, 45.0, 100.0, contact_kind(c));
        let s = project(&arena, &cam, 640, 480, &silhouette).expect("in view");
        assert!(
            w > s.width.ceil() as i64 && bh < s.height.ceil() as i64,
            "the sheet's box {w}x{bh} is wider and lower than the silhouette's {}x{}",
            s.width.ceil(),
            s.height.ceil()
        );
    }

    /// Doc 21a `X-03`, review: a lock in the view's columns but clear over
    /// its top edge or under its bottom — close by, up a crest or down a
    /// steep slope, where the relief projection now puts it — keeps its
    /// readout: an arrow at that edge in its column with the designator,
    /// range and hit chance beside it, where the box (and the label under
    /// it) would have been drawn clear of the view and nothing of the lock
    /// shown. And a box running off the bottom edge keeps its readout in the
    /// view, over the box. The text cockpit's wide frame (640 by 256), BA
    /// 60 m out, the yard at one height under the whole map.
    #[test]
    fn the_lock_over_an_edge_keeps_its_readout() {
        let mut cockpit = parsed(COCKPIT);
        cockpit["contacts"][0]["range_m"] = serde_json::json!(60);
        let arena = map_from_feed(&parsed(MAP));
        let look = Look::default();
        let cam = camera(&cockpit, &look);
        let eye = eye_cm(&cockpit);
        assert_eq!(
            eye,
            Some(150),
            "the stock eye over the tank's own ground, z 0"
        );
        let (w, h) = (640usize, 256usize);
        let on = |elev_cm: i32| {
            let mut terrain = Terrain::new(arena.clone());
            for y in 0..arena.height() {
                for x in 0..arena.width() {
                    terrain.set_elev_cm(x, y, elev_cm);
                }
            }
            let frame = Frame::new(w, h);
            let flight = hud(&frame, &arena, Some(&terrain), eye, &cockpit, &look, &cam);
            (flight.lock.clone().expect("BA is locked").at, flight)
        };
        // The rows the lock's colour is drawn on, the HUD alone on the frame.
        let lock_rows = |flight: &Hud| -> Vec<usize> {
            let mut frame = Frame::new(w, h);
            overlay::draw_hud(&mut frame, flight);
            (0..h)
                .filter(|&y| (0..w).any(|x| frame.get(x, y) == overlay::LOCK))
                .collect()
        };

        // 25 m down: the whole box under the bottom edge.
        let (at, flight) = on(-2_500);
        assert_eq!(
            at,
            LockAt::Beyond(320, 1),
            "under the bottom edge, dead ahead"
        );
        let rows = lock_rows(&flight);
        assert!(!rows.is_empty(), "the lock is drawn");
        assert!(
            rows.iter().all(|&y| y >= h - 32),
            "at the bottom edge: {rows:?}"
        );
        // 25 m up: clear over the top edge.
        let (at, flight) = on(2_500);
        assert_eq!(at, LockAt::Beyond(320, -1), "over the top edge, dead ahead");
        let rows = lock_rows(&flight);
        assert!(!rows.is_empty(), "the lock is drawn");
        assert!(rows.iter().all(|&y| y < 32), "at the top edge: {rows:?}");
        // 14 m down: the box runs off the bottom edge, in view; its label,
        // off the view under it, goes over it.
        let (at, flight) = on(-1_400);
        let LockAt::Box(_, y, _, bh) = at else {
            panic!("partly in view is boxed: {at:?}");
        };
        assert!(
            y < h as i64 && y + bh > h as i64 - 20,
            "the box's bottom by the edge: top {y}, height {bh}"
        );
        let rows = lock_rows(&flight);
        assert!(
            rows.iter().any(|&r| (r as i64) < y - 2),
            "the label over the box: rows {rows:?}, box top {y}"
        );
    }
}
