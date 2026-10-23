//! **The replay as a picture** (doc 21 cut G.8; bucket B): a finished
//! match's `arena.replay` document (16b cut I.1) drawn as a [`Scene`] over
//! the ground it was fought on — the whole field, north up, every roster unit
//! in its team's colour with its heading and its trail, the shots as lines
//! from shooter to target for a moment after they were fired, the wrecks, the
//! named spots, the unit in focus ringed — and under it a timeline the whole
//! match long, with the keyframes, every shot and every wreck marked on it
//! and the cursor where the picture stands.
//!
//! **Where the cursor stands is the client's** (16b cut I.2: every step of a
//! replay is the client's own, over the document it holds). The viewer's
//! cursor reaches the picture as a client-local feed, [`REPLAY_CURSOR_FEED`]
//! (`{"match", "t_ms", "focus"}`), which no server sends.
//!
//! **Between keyframes the picture is a drawing, and says so** (16c N9's
//! default, the web's rule): the arena recorded where every unit stood every
//! `summary_ms`; strictly between two keyframes each unit moves in a straight
//! line and turns along the shorter arc, and the title reads `DRAWN BETWEEN
//! KEYFRAMES 2 AND 3`. On a keyframe's own time it is that keyframe; past the
//! last, the last keyframe held (`HELD`).
//!
//! **Both sides are shown** (Q37): a replay is of a finished match and names
//! every tank by the callsign the arena gave it; the document carries no
//! object number, no pilot and no contact id, so none reaches the picture.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::cockpit::{ARENA_MAP_FEED, map_from_feed};
use crate::frame::Rgb;
use crate::overlay::{BLUE_TEAM, DIM, PANEL, RED_TEAM, TEXT, WRECK, YOU, draw_card, ground_rgb};
use crate::picture::{BUCKET_B_INTERVAL, Feeds, Look, Picture};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// The replay document's feed (16b cut I.1's `arena_bridge::REPLAY_FEED`).
pub const REPLAY_FEED: &str = "arena.replay";

/// The viewer's cursor over the held document: `{"match", "t_ms", "focus"}`.
/// Client-local — the TUI writes it from its own state for each draw, and no
/// server sends it.
pub const REPLAY_CURSOR_FEED: &str = "replay.cursor";

/// How long a shot's line stays drawn after it was fired, arena ms.
pub const SHOT_SHOWN_MS: i64 = 1_500;

/// From what age a trail is drawn faded, arena seconds.
pub const TRAIL_FADES_AFTER_S: u32 = 30;

const RED_TRAIL: Rgb = [150, 56, 48];
const BLUE_TRAIL: Rgb = [62, 100, 160];
const RED_OLD: Rgb = [84, 44, 40];
const BLUE_OLD: Rgb = [44, 62, 94];
const HIT: Rgb = [255, 220, 90];
const MISS: Rgb = [150, 150, 150];
const TRACK: Rgb = [48, 52, 62];
const PLAYED: Rgb = [92, 98, 112];
const FIELD_EDGE: Rgb = [96, 104, 120];

/// The replay as a [`Picture`]: drawn from the held document, the ground and
/// the cursor, on bucket B's clock (on change; the TUI runs it faster while
/// playing). Keeps the ground drawn at the size it was last drawn at.
#[derive(Debug, Default)]
pub struct ReplayPicture {
    ground: Option<(GroundKey, Frame)>,
}

/// What a drawn ground was drawn from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GroundKey {
    match_id: i64,
    surface: String,
    cells: (usize, usize),
    px: (i64, i64),
}

impl ReplayPicture {
    /// A replay picture.
    #[must_use]
    pub fn new() -> ReplayPicture {
        ReplayPicture::default()
    }
}

impl Picture for ReplayPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[REPLAY_FEED, ARENA_MAP_FEED, REPLAY_CURSOR_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_B_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, _look: &Look) -> bool {
        let Some(doc) = feeds.feed(REPLAY_FEED).and_then(Replay::of) else {
            draw_card(frame, "WAITING FOR THE REPLAY");
            return true;
        };
        let cursor = feeds.feed(REPLAY_CURSOR_FEED);
        let t_ms = cursor
            .and_then(|c| c.get("t_ms"))
            .and_then(Value::as_i64)
            .unwrap_or_else(|| doc.keyframes.first().map_or(0, |k| k.t_ms));
        let focus = cursor
            .and_then(|c| c.get("focus"))
            .and_then(Value::as_u64)
            .and_then(|f| usize::try_from(f).ok())
            .unwrap_or(0);
        let ground = feeds
            .feed(ARENA_MAP_FEED)
            .filter(|m| m.get("seated").and_then(Value::as_bool) != Some(false))
            .filter(|m| m.get("match").and_then(Value::as_i64) == Some(doc.match_id));
        let scene = replay_scene(frame, &doc, ground, t_ms, focus, &mut self.ground);
        scene.draw(frame);
        true
    }
}

// ---- the document ----------------------------------------------------------

/// One roster unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// The callsign the arena gave it.
    pub callsign: String,
    /// Whether it flew blue (anything else reads red, as the server's reader
    /// keeps an unreadable side).
    pub blue: bool,
}

/// Where a unit stood: cell `x`, `y` and a heading in degrees.
pub type Slot = (f64, f64, f64);

/// One keyframe: its arena millisecond and a slot a roster unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// The arena millisecond.
    pub t_ms: i64,
    /// One a roster unit; `None` when it is not in the keyframe.
    pub units: Vec<Option<Slot>>,
}

/// One shot: when, who fired at whom, and whether it hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    /// The arena millisecond.
    pub t_ms: i64,
    /// The shooter's roster index.
    pub shooter: usize,
    /// The target's roster index.
    pub target: usize,
    /// Whether the outcome word was `hit`.
    pub hit: bool,
}

/// **A read of an `arena.replay` document** (16b cut I.1's
/// `Replay::to_document`), total over what the wire might carry: a missing or
/// malformed key reads empty, and a keyframe slot that is not three numbers
/// reads as not in the keyframe.
#[derive(Clone, Debug, PartialEq)]
pub struct Replay {
    /// The match it is a replay of.
    pub match_id: i64,
    /// The arena's size, cells.
    pub cells: (usize, usize),
    /// The roster, in roster order.
    pub roster: Vec<Unit>,
    /// The keyframes, in time order.
    pub keyframes: Vec<Keyframe>,
    /// The shots, in firing order.
    pub shots: Vec<Shot>,
    /// The wrecks: arena millisecond and roster index.
    pub destroyed: Vec<(i64, usize)>,
    /// When the match ended.
    pub end_ms: i64,
    /// The outcome word (`red`, `blue`, `draw`, `timeout`, `abort`, `void`).
    pub outcome: String,
}

impl Replay {
    /// The document in `value`, or `None` for anything else — the pushed
    /// grid, `available: false`, or a payload with no match.
    #[must_use]
    pub fn of(value: &Value) -> Option<Replay> {
        if value.pointer("/schema/shape").and_then(Value::as_str) != Some("replay")
            || value.get("available").and_then(Value::as_bool) == Some(false)
        {
            return None;
        }
        let match_id = value.get("match").and_then(Value::as_i64)?;
        let list = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
        };
        let size = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        let index = |v: Option<&Value>| {
            v.and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
        };
        let roster = list("roster")
            .iter()
            .map(|u| Unit {
                callsign: u
                    .get("callsign")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .chars()
                    .take(8)
                    .collect(),
                blue: u.get("team").and_then(Value::as_str) == Some("blue"),
            })
            .collect();
        let keyframes = list("keyframes")
            .iter()
            .filter_map(|kf| {
                let t_ms = kf.get(0)?.as_i64()?;
                let units = kf
                    .get(1)?
                    .as_array()?
                    .iter()
                    .map(|slot| {
                        let row = slot.as_array()?;
                        let n = |i: usize| row.get(i).and_then(Value::as_f64);
                        Some((n(0)?, n(1)?, n(2)?.rem_euclid(360.0)))
                    })
                    .collect();
                Some(Keyframe { t_ms, units })
            })
            .collect();
        let shots = list("shots")
            .iter()
            .filter_map(|s| {
                Some(Shot {
                    t_ms: s.get(0)?.as_i64()?,
                    shooter: index(s.get(1))?,
                    target: index(s.get(2))?,
                    hit: s.get(3)?.as_str()? == "hit",
                })
            })
            .collect();
        let destroyed = list("destroyed")
            .iter()
            .filter_map(|d| Some((d.get(0)?.as_i64()?, index(d.get(1))?)))
            .collect();
        Some(Replay {
            match_id,
            cells: (size("width"), size("height")),
            roster,
            keyframes,
            shots,
            destroyed,
            end_ms: value.get("end_ms").and_then(Value::as_i64).unwrap_or(0),
            outcome: value
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Whether roster unit `unit` was a wreck by arena millisecond `t_ms`.
    #[must_use]
    pub fn wrecked_by(&self, unit: usize, t_ms: i64) -> bool {
        self.destroyed.iter().any(|(t, u)| *u == unit && *t <= t_ms)
    }

    /// **Every unit at arena millisecond `t_ms`** — the web's `frameAt`: on
    /// a keyframe's own time that keyframe; strictly between two, each unit
    /// on the straight line from one to the other and turned along the
    /// shorter arc (a unit in only one of them drawn where that one has it);
    /// before the first and after the last, that keyframe held.
    #[must_use]
    pub fn frame_at(&self, t_ms: i64) -> At {
        let kfs = &self.keyframes;
        let (Some(first), Some(last)) = (kfs.first(), kfs.last()) else {
            return At {
                units: vec![None; self.roster.len()],
                drawn: Drawn::Keyframe(0),
            };
        };
        let held = |i: usize, kf: &Keyframe, held: bool| At {
            units: kf.units.clone(),
            drawn: if held {
                Drawn::Held(i)
            } else {
                Drawn::Keyframe(i)
            },
        };
        if t_ms <= first.t_ms {
            return held(0, first, t_ms < first.t_ms);
        }
        if t_ms >= last.t_ms {
            return held(kfs.len() - 1, last, t_ms > last.t_ms);
        }
        let after = kfs
            .iter()
            .position(|k| k.t_ms >= t_ms)
            .unwrap_or(kfs.len() - 1);
        let (Some(a), Some(b)) = (kfs.get(after.saturating_sub(1)), kfs.get(after)) else {
            return held(0, first, false);
        };
        if b.t_ms == t_ms {
            return held(after, b, false);
        }
        let f = (t_ms - a.t_ms) as f64 / (b.t_ms - a.t_ms).max(1) as f64;
        let units = (0..self.roster.len())
            .map(|i| {
                let from = a.units.get(i).copied().flatten();
                let to = b.units.get(i).copied().flatten();
                match (from, to) {
                    (Some((x0, y0, h0)), Some((x1, y1, h1))) => {
                        Some((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, turn(h0, h1, f)))
                    }
                    (one, other) => one.or(other),
                }
            })
            .collect();
        At {
            units,
            drawn: Drawn::Between(after - 1, after),
        }
    }

    /// The keyframe at or before `t_ms` (the first when before it).
    #[must_use]
    pub fn keyframe_at(&self, t_ms: i64) -> usize {
        self.keyframes
            .iter()
            .rposition(|k| k.t_ms <= t_ms)
            .unwrap_or(0)
    }
}

/// A heading `a` to `b` by `f` along the shorter arc.
fn turn(a: f64, b: f64, f: f64) -> f64 {
    let delta = ((b - a) % 360.0 + 540.0) % 360.0 - 180.0;
    (a + delta * f).rem_euclid(360.0)
}

/// What [`Replay::frame_at`] drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawn {
    /// A keyframe's own time.
    Keyframe(usize),
    /// Strictly between two keyframes: the renderer's drawing.
    Between(usize, usize),
    /// Before the first or after the last keyframe: that keyframe held.
    Held(usize),
}

/// Every unit at one arena millisecond.
#[derive(Clone, Debug, PartialEq)]
pub struct At {
    /// One a roster unit.
    pub units: Vec<Option<Slot>>,
    /// What was drawn.
    pub drawn: Drawn,
}

/// `m:ss`, as the panes print a match clock.
#[must_use]
pub fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1_000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The outcome word as the title prints it.
fn outcome_words(outcome: &str) -> &'static str {
    match outcome {
        "red" => "RED WON",
        "blue" => "BLUE WON",
        "draw" => "DRAW",
        "timeout" => "TIMED OUT",
        "abort" => "ABORTED",
        "void" => "VOID",
        _ => "",
    }
}

// ---- the scene -------------------------------------------------------------

/// The replay's scene for `frame` at arena millisecond `t_ms`, focused on
/// roster unit `focus`, over `ground` when it is held (the field is drawn
/// dark until it is). `cache` keeps the drawn ground between draws.
fn replay_scene(
    frame: &Frame,
    doc: &Replay,
    ground: Option<&Value>,
    t_ms: i64,
    focus: usize,
    cache: &mut Option<(GroundKey, Frame)>,
) -> Scene {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    let strip = 11 * s;
    let timeline = 23 * s;
    let map = ground.map(map_from_feed);
    let cells = match &map {
        Some(m) if m.width() > 0 && m.height() > 0 => (m.width(), m.height()),
        _ => doc.cells,
    };
    let (cw, ch) = (cells.0.max(1) as f64, cells.1.max(1) as f64);
    // The field, fitted whole between the strips, aspect kept.
    let room = (w - 4 * s, h - strip - timeline - 2 * s);
    let px_per_cell = (room.0 as f64 / cw).min(room.1 as f64 / ch).max(0.1);
    let fw = (cw * px_per_cell).floor() as i64;
    let fh = (ch * px_per_cell).floor() as i64;
    let fx = (w - fw) / 2;
    let fy = strip + (room.1 - fh).max(0) / 2;
    let mut scene = Scene::new(Viewport {
        world: (0.0, 0.0, cw, ch),
        frame: (fx, fy, fw, fh),
        transform: Transform::Linear,
    });
    scene.push(Item::Panel {
        rect: (0, 0, w, h),
        fill: PANEL,
        border: None,
    });
    scene.push(Item::Panel {
        rect: (fx - 1, fy - 1, fw + 2, fh + 2),
        fill: [10, 12, 16],
        border: Some(FIELD_EDGE),
    });
    if let (Some(map), Some(ground)) = (&map, ground) {
        let key = GroundKey {
            match_id: doc.match_id,
            surface: ground
                .get("surface")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            cells,
            px: (fw, fh),
        };
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut image = Frame::new(fw.max(0) as usize, fh.max(0) as usize);
            for py in 0..fh.max(0) {
                let cy = ch as i64 - 1 - (py as f64 / px_per_cell) as i64;
                for px in 0..fw.max(0) {
                    let cx = (px as f64 / px_per_cell) as i64;
                    image.put(px as usize, py as usize, ground_rgb(map.get(cx, cy)));
                }
            }
            *cache = Some((key, image));
        }
        if let Some((_, image)) = cache.as_ref() {
            scene.push(Item::Image {
                at: (fx, fy),
                frame: image.clone(),
            });
        }
        for spot in ground
            .get("spots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let cell = spot.get("cell").and_then(Value::as_array);
            let n = |i: usize| cell.and_then(|c| c.get(i)).and_then(Value::as_f64);
            let (Some(x), Some(y)) = (n(0), n(1)) else {
                continue;
            };
            let letter = spot
                .get("letter")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            scene.push(Item::Marker {
                at: (x + 0.5, y + 0.5),
                half: 4 * s,
                shape: Shape::Ring,
                colour: DIM,
            });
            scene.push(Item::Text {
                at: (x + 0.5, y + 0.5),
                offset: (0, 0),
                anchor: Anchor::Centre,
                text: letter,
                colour: TEXT,
                scale: s as usize,
            });
        }
    }

    let at = doc.frame_at(t_ms);
    let place = |slot: Slot| (slot.0 + 0.5, slot.1 + 0.5);

    // The trails: every keyframe up to the cursor, then where it is drawn;
    // a wreck's ends where it died.
    scene.layer();
    for (i, unit) in doc.roster.iter().enumerate() {
        let died = doc
            .destroyed
            .iter()
            .filter(|(_, u)| *u == i)
            .map(|(t, _)| *t)
            .min();
        let until = died.map_or(t_ms, |d| d.min(t_ms));
        let mut points: Vec<((f64, f64), u32)> = doc
            .keyframes
            .iter()
            .filter(|k| k.t_ms <= until)
            .filter_map(|k| {
                let slot = k.units.get(i).copied().flatten()?;
                let age = u32::try_from((t_ms - k.t_ms).max(0) / 1_000).unwrap_or(u32::MAX);
                Some((place(slot), age))
            })
            .collect();
        if died.is_none_or(|d| d > t_ms)
            && let Some(slot) = at.units.get(i).copied().flatten()
        {
            points.push((place(slot), 0));
        }
        if points.len() > 1 {
            let (colour, old) = if unit.blue {
                (BLUE_TRAIL, BLUE_OLD)
            } else {
                (RED_TRAIL, RED_OLD)
            };
            scene.push(Item::Trail {
                points,
                colour,
                old,
                old_after: TRAIL_FADES_AFTER_S,
            });
        }
    }

    // The shots of the last moment, shooter to target where each stood then.
    for shot in doc
        .shots
        .iter()
        .filter(|s| s.t_ms <= t_ms && t_ms - s.t_ms < SHOT_SHOWN_MS)
    {
        let then = doc.frame_at(shot.t_ms);
        let (Some(from), Some(to)) = (
            then.units.get(shot.shooter).copied().flatten(),
            then.units.get(shot.target).copied().flatten(),
        ) else {
            continue;
        };
        let (colour, dash) = if shot.hit {
            (HIT, None)
        } else {
            (MISS, Some((3, 3)))
        };
        scene.push(Item::Line {
            a: place(from),
            b: place(to),
            colour,
            dash,
        });
        if shot.hit {
            scene.push(Item::Marker {
                at: place(to),
                half: 3 * s,
                shape: Shape::Ring,
                colour: HIT,
            });
        }
    }

    // The units, then the focus ring, then the callsigns over everything.
    scene.layer();
    for (i, unit) in doc.roster.iter().enumerate() {
        let Some(slot) = at.units.get(i).copied().flatten() else {
            continue;
        };
        if doc.wrecked_by(i, t_ms) {
            scene.push(Item::Marker {
                at: place(slot),
                half: 2 * s,
                shape: Shape::Square,
                colour: WRECK,
            });
            continue;
        }
        let colour = if unit.blue { BLUE_TEAM } else { RED_TEAM };
        scene.push(Item::Ray {
            at: place(slot),
            bearing_deg: slot.2 as f32,
            len_px: 7.0 * s as f32,
            colour,
        });
        scene.push(Item::Marker {
            at: place(slot),
            half: 2 * s,
            shape: Shape::Disc,
            colour,
        });
    }
    if let Some(slot) = at.units.get(focus).copied().flatten() {
        scene.push(Item::Marker {
            at: place(slot),
            half: 6 * s,
            shape: Shape::Ring,
            colour: YOU,
        });
    }
    for (i, unit) in doc.roster.iter().enumerate() {
        let Some(slot) = at.units.get(i).copied().flatten() else {
            continue;
        };
        let wreck = doc.wrecked_by(i, t_ms);
        scene.push(Item::Text {
            at: place(slot),
            offset: (5 * s, -9 * s),
            anchor: Anchor::TopLeft,
            text: unit.callsign.to_ascii_uppercase(),
            colour: if wreck { DIM } else { TEXT },
            scale: s as usize,
        });
    }

    // The title strip.
    let drawn = match at.drawn {
        Drawn::Keyframe(i) => format!("KEYFRAME {}/{}", i + 1, doc.keyframes.len()),
        Drawn::Between(a, b) => format!("DRAWN BETWEEN KEYFRAMES {} AND {}", a + 1, b + 1),
        Drawn::Held(i) => format!("KEYFRAME {}/{} HELD", i + 1, doc.keyframes.len()),
    };
    let outcome = outcome_words(&doc.outcome);
    let title = if outcome.is_empty() {
        format!("REPLAY {}", doc.match_id)
    } else {
        format!("REPLAY {} - {outcome}", doc.match_id)
    };
    let clock_words = format!("{} / {}", clock(t_ms), clock(doc.end_ms));
    let cut = |words: String, room: i64| -> String {
        let fits = usize::try_from((room + s) / (6 * s)).unwrap_or(0);
        words.chars().take(fits).collect()
    };
    let left = cut(title, w / 2 - 4 * s);
    scene.push(Item::Label {
        at: (2 * s, 2 * s),
        text: left,
        colour: TEXT,
        scale: s as usize,
    });
    let rw = clock_words.chars().count() as i64 * 6 * s - s;
    scene.push(Item::Label {
        at: (w - 2 * s - rw, 2 * s),
        text: clock_words,
        colour: TEXT,
        scale: s as usize,
    });

    // The timeline: the whole match, the played part lit, the keyframes as
    // ticks under the track, the shots over it in the shooter's colour, the
    // wrecks as tall ticks in the wreck's, the cursor across it all.
    let ty = h - timeline + 6 * s;
    let (tx0, tx1) = (4 * s, w - 4 * s);
    let span = doc
        .end_ms
        .max(doc.keyframes.last().map_or(0, |k| k.t_ms))
        .max(1);
    let x_of = |t: i64| tx0 + (tx1 - tx0) * t.clamp(0, span) / span;
    let rect = |x: i64, y: i64, rw: i64, rh: i64, colour: Rgb| Item::Panel {
        rect: (x, y, rw, rh),
        fill: colour,
        border: None,
    };
    scene.push(rect(tx0, ty, tx1 - tx0, 2 * s, TRACK));
    scene.push(rect(tx0, ty, x_of(t_ms) - tx0, 2 * s, PLAYED));
    for k in &doc.keyframes {
        scene.push(rect(x_of(k.t_ms), ty + 3 * s, s, 2 * s, DIM));
    }
    for shot in &doc.shots {
        let blue = doc.roster.get(shot.shooter).is_some_and(|u| u.blue);
        let colour = if blue { BLUE_TEAM } else { RED_TEAM };
        scene.push(rect(x_of(shot.t_ms), ty - 3 * s, s, 2 * s, colour));
    }
    for (t, unit) in &doc.destroyed {
        let blue = doc.roster.get(*unit).is_some_and(|u| u.blue);
        let colour = if blue { BLUE_TEAM } else { RED_TEAM };
        scene.push(rect(x_of(*t) - s, ty - 6 * s, 2 * s, 11 * s, colour));
    }
    scene.push(rect(x_of(t_ms) - s / 2, ty - 7 * s, s.max(1), 13 * s, YOU));

    // Under it: what the picture is (a keyframe, or the renderer's drawing
    // between two), and the unit in focus.
    let focus_words = doc
        .roster
        .get(focus)
        .map(|u| format!("FOCUS {}", u.callsign.to_ascii_uppercase()))
        .unwrap_or_default();
    let fw_px = focus_words.chars().count() as i64 * 6 * s - s;
    scene.push(Item::Label {
        at: (tx0, ty + 7 * s),
        text: cut(drawn, w - fw_px - 12 * s),
        colour: if matches!(at.drawn, Drawn::Keyframe(_)) {
            TEXT
        } else {
            DIM
        },
        scale: s as usize,
    });
    scene.push(Item::Label {
        at: (tx1 - fw_px, ty + 7 * s),
        text: focus_words,
        colour: TEXT,
        scale: s as usize,
    });
    scene
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "replay" => include_str!("../../../web/src/lib/fixtures/replay/replay.json"),
            _ => include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        };
        let mut v: Value = serde_json::from_str(text).expect("fixture");
        if name == "replay" {
            // The query's answer carries the document under its own shape.
            v["schema"] = serde_json::json!({"id": REPLAY_FEED, "version": 1, "shape": "replay"});
        }
        v
    }

    fn feeds(t_ms: Option<i64>, ground: bool) -> HashMap<String, Value> {
        let mut f: HashMap<String, Value> = HashMap::new();
        f.insert(REPLAY_FEED.into(), fixture("replay"));
        if ground {
            f.insert(ARENA_MAP_FEED.into(), fixture("map"));
        }
        if let Some(t) = t_ms {
            f.insert(
                REPLAY_CURSOR_FEED.into(),
                serde_json::json!({"match": 7, "t_ms": t, "focus": 0}),
            );
        }
        f
    }

    fn draw(f: &HashMap<String, Value>, w: usize, h: usize) -> Frame {
        let mut frame = Frame::new(w, h);
        assert!(ReplayPicture::new().draw(&mut frame, f, &Look::default()));
        frame
    }

    /// The fixture's document reads whole: four units, three keyframes, two
    /// shots, a wreck.
    #[test]
    fn the_document_reads() {
        let doc = Replay::of(&fixture("replay")).unwrap();
        assert_eq!(doc.match_id, 7);
        assert_eq!(doc.cells, (96, 64));
        assert_eq!(doc.roster.len(), 4);
        assert!(doc.roster[2].blue && !doc.roster[0].blue);
        assert_eq!(doc.keyframes.len(), 3);
        assert_eq!(doc.shots.len(), 2);
        assert!(doc.shots[0].hit && !doc.shots[1].hit);
        assert_eq!(doc.destroyed, vec![(7_000, 3)]);
        assert!(doc.wrecked_by(3, 7_000) && !doc.wrecked_by(3, 6_999));
        assert_eq!(doc.end_ms, 10_000);
        assert!(Replay::of(&serde_json::json!({"available": false})).is_none());
    }

    /// The web's `frameAt`: a keyframe on its time, a straight line and the
    /// shorter arc between, held outside.
    #[test]
    fn frame_at_interpolates_between_keyframes_and_holds_outside() {
        let doc = Replay::of(&fixture("replay")).unwrap();
        let k = doc.frame_at(5_000);
        assert_eq!(k.drawn, Drawn::Keyframe(1));
        assert_eq!(k.units[0], Some((6.0, 4.0, 90.0)));
        let mid = doc.frame_at(2_500);
        assert_eq!(mid.drawn, Drawn::Between(0, 1));
        assert_eq!(mid.units[0], Some((5.0, 4.0, 90.0)));
        // RB turns 90 → 0 the short way, through 45.
        assert_eq!(mid.units[1], Some((2.0, 8.0, 45.0)));
        assert_eq!(doc.frame_at(-1).drawn, Drawn::Held(0));
        assert_eq!(doc.frame_at(10_000).drawn, Drawn::Keyframe(2));
        assert_eq!(doc.keyframe_at(7_400), 1);
        assert_eq!(doc.keyframe_at(-5), 0);
        assert!((turn(350.0, 10.0, 0.5) - 0.0).abs() < 1e-9);
    }

    /// The picture: the ground drawn under the units, each team in its
    /// colour, a moving cursor moving the picture.
    #[test]
    fn the_picture_draws_the_field_the_units_and_the_timeline() {
        let f = draw(&feeds(Some(6_100), true), 640, 400);
        let colours: Vec<Rgb> = (0..400)
            .flat_map(|y| (0..640).map(move |x| (x, y)))
            .map(|(x, y)| f.get(x, y))
            .collect();
        for want in [
            RED_TEAM,
            BLUE_TEAM,
            HIT,
            YOU,
            ground_rgb(crate::raycast::Ground::Grass),
        ] {
            assert!(colours.contains(&want), "{want:?} drawn");
        }
        assert!(f.count_colours(256) <= 64, "a small palette");
        let later = draw(&feeds(Some(9_000), true), 640, 400);
        assert_ne!(f.hash(), later.hash(), "the cursor moves the picture");
        let same = draw(&feeds(Some(6_100), true), 640, 400);
        assert_eq!(f.hash(), same.hash(), "and the same cursor draws the same");
    }

    /// The killing shot's line is drawn only for a moment; the wreck is grey
    /// after it.
    #[test]
    fn shots_fade_and_wrecks_turn_grey() {
        let has =
            |f: &Frame, c: Rgb| (0..f.height()).any(|y| (0..f.width()).any(|x| f.get(x, y) == c));
        assert!(has(&draw(&feeds(Some(6_000), true), 640, 400), HIT));
        assert!(!has(
            &draw(&feeds(Some(6_000 + SHOT_SHOWN_MS), true), 640, 400),
            HIT
        ));
        assert!(has(&draw(&feeds(Some(7_500), true), 640, 400), WRECK));
    }

    /// No document is a card; no ground draws the field dark; no cursor
    /// stands at the first keyframe.
    #[test]
    fn missing_feeds_are_drawn_not_refused() {
        let mut frame = Frame::new(320, 220);
        let empty: HashMap<String, Value> = HashMap::new();
        assert!(ReplayPicture::new().draw(&mut frame, &empty, &Look::default()));
        let dark = draw(&feeds(None, false), 320, 220);
        let first = draw(&feeds(Some(0), false), 320, 220);
        assert_eq!(dark.hash(), first.hash());
        // A ground of another match is not this one's.
        let mut other = feeds(Some(0), true);
        other.get_mut(ARENA_MAP_FEED).unwrap()["match"] = 8.into();
        assert_eq!(draw(&other, 320, 220).hash(), first.hash());
    }

    /// The ground is drawn once a size and re-used.
    #[test]
    fn the_ground_is_cached_by_size() {
        let f = feeds(Some(0), true);
        let mut picture = ReplayPicture::new();
        let mut frame = Frame::new(640, 400);
        picture.draw(&mut frame, &f, &Look::default());
        let key = picture.ground.as_ref().map(|(k, _)| k.clone()).unwrap();
        picture.draw(&mut frame, &f, &Look::default());
        assert_eq!(
            picture.ground.as_ref().map(|(k, _)| k.clone()),
            Some(key.clone())
        );
        let mut small = Frame::new(320, 220);
        picture.draw(&mut small, &f, &Look::default());
        assert_ne!(picture.ground.as_ref().map(|(k, _)| k.clone()), Some(key));
    }
}
