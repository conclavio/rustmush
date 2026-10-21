//! **The pixel cockpit view** (spike, 2026-10-21): a first-person picture of
//! the arena streamed over the kitty graphics protocol, drawn from the
//! `arena.cockpit` frame the TUI already holds.
//!
//! What lives here is the per-connection state the SSH TUI keeps for it:
//!
//! - **The probe**: `@tui` sends a one-pixel kitty query and `CSI c` right
//!   after the first frame. A graphics OK arriving first means support; the
//!   DA1 reply arriving first, or neither within [`PROBE_TIMEOUT`], means
//!   none. The view opens only on support.
//! - **The camera**: the sim's own `x_cm`/`y_cm`/`heading_mdeg` plus a LOCAL
//!   heading offset the pilot turns with toggle-turn keys (press to start
//!   turning, press again to stop) — the camera on a leash. The sim is not
//!   told; this spike proves the picture and the pacing, not the order.
//! - **The frame clock**: the output pump ticks [`FRAME_INTERVAL`]; each
//!   tick advances the turn, renders, and sends the frame only when its
//!   pixels (or the cell box it is scaled into) changed since the last send.
//!
//! The raycaster, the PNG and the kitty framing are `mush-pixel`; the probe
//! replies arrive through the key parser as [`Key::GraphicsOk`],
//! [`Key::GraphicsErr`] and [`Key::DeviceAttributes`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use mush_pixel::cockpit::{contacts_as_sprites, draw_instruments};
use mush_pixel::raycast::{Camera, Ground, Map, render};
use mush_pixel::{Frame, kitty, png};
use serde_json::Value;

use super::keys::Key;

/// The rendered frame's size; the terminal scales it into the cell box.
/// **640x480 at 20 Hz is the target** (the owner's ruling of 2026-10-21,
/// after the load experiment measured it at 21 % of a vCPU and 100 KB/s a
/// viewer): the extra pixels are for the minimap, heat gauges, weapon
/// recycle states and the armour doll a real cut will draw into the frame.
pub const FRAME_W: usize = 640;
/// See [`FRAME_W`].
pub const FRAME_H: usize = 480;
/// The smaller frame `r` switches to: the spike's original 320x220, a
/// quarter of the pixels and the cost, for a slow link or a loaded box.
pub const FRAME_W_LO: usize = 320;
/// See [`FRAME_W_LO`].
pub const FRAME_H_LO: usize = 220;
/// The frame clock's wait when `u` uncaps it: as fast as render, encode
/// and the write allow (tokio's timer floor).
pub const FRAME_INTERVAL_UNCAPPED: Duration = Duration::from_millis(1);
/// The kitty image id the view is placed under (the probe uses 31).
pub const IMAGE_ID: u32 = 1;
/// How long after the probe a silent terminal is taken to lack support.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// The frame clock: 20 frames a second.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(50);
/// How fast a toggled turn swings the camera, millidegrees a second (45°/s).
const TURN_MDEG_PER_S: i64 = 45_000;
/// The map drawn while no `arena.map` is held: open ground, big enough that
/// the eye never sees its edge.
const OPEN_MAP_CELLS: usize = 512;

/// Where the capability probe stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Not sent yet (line mode, or a session that never entered the TUI).
    NotSent,
    /// Sent at this instant; waiting on an answer.
    Sent(Instant),
    /// The terminal answered the graphics query OK.
    Supported,
    /// It answered DA1 first, answered with an error, or stayed silent.
    Unsupported,
}

/// Counters the HUD prints and the notes record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The last frame's render time, microseconds.
    pub render_us: u128,
    /// The last frame's PNG encode time, microseconds.
    pub encode_us: u128,
    /// The last frame's PNG size, bytes.
    pub png_bytes: usize,
    /// The last frame's bytes on the wire (base64, chunk headers, sync).
    pub wire_bytes: usize,
    /// Frames sent.
    pub sent: u64,
    /// Ticks whose frame matched the last sent one and was skipped.
    pub skipped: u64,
}

/// The view's state for one connection.
#[derive(Debug)]
pub struct PixelView {
    probe: Probe,
    active: bool,
    /// `-1` turning left, `0` still, `1` turning right.
    turn: i8,
    /// The camera's heading offset from the sim's, millidegrees.
    offset_mdeg: i64,
    last_tick: Option<Instant>,
    /// The hash of the last frame sent, with the cell box it was scaled into.
    last_sent: Option<(u64, u16, u16)>,
    frame: Frame,
    /// The arena map built from the held `arena.map`, keyed by its match.
    map: Option<(Option<i64>, Map)>,
    close_pending: bool,
    /// `u`: the frame clock runs as fast as it can instead of 20 Hz.
    uncapped: bool,
    /// When the frames of the last second went out: the HUD's measured
    /// frames-per-second, which is what the wire carries (the `sent` total
    /// says nothing about rate).
    sent_at: VecDeque<Instant>,
    /// Counters.
    pub stats: Stats,
}

impl Default for PixelView {
    fn default() -> Self {
        Self::new()
    }
}

impl PixelView {
    /// A closed view with the probe not sent.
    #[must_use]
    pub fn new() -> PixelView {
        PixelView {
            probe: Probe::NotSent,
            active: false,
            turn: 0,
            offset_mdeg: 0,
            last_tick: None,
            last_sent: None,
            frame: Frame::new(FRAME_W, FRAME_H),
            map: None,
            close_pending: false,
            uncapped: false,
            sent_at: VecDeque::new(),
            stats: Stats::default(),
        }
    }

    /// The frame clock's wait: 20 Hz, or ~1 ms when uncapped.
    #[must_use]
    pub fn interval(&self) -> Duration {
        if self.uncapped {
            FRAME_INTERVAL_UNCAPPED
        } else {
            FRAME_INTERVAL
        }
    }

    /// Whether the clock is uncapped.
    #[must_use]
    pub fn uncapped(&self) -> bool {
        self.uncapped
    }

    /// The rendered frame's size in pixels.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.frame.width(), self.frame.height())
    }

    /// Frames sent in the second before `now`.
    #[must_use]
    pub fn fps(&self, now: Instant) -> usize {
        self.sent_at
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < Duration::from_secs(1))
            .count()
    }

    // ---- the probe -------------------------------------------------------

    /// The probe bytes went out at `now`.
    pub fn probe_sent(&mut self, now: Instant) {
        if self.probe == Probe::NotSent {
            self.probe = Probe::Sent(now);
        }
    }

    /// The graphics query was answered.
    pub fn graphics_reply(&mut self, ok: bool) {
        if matches!(self.probe, Probe::Sent(_)) {
            self.probe = if ok {
                Probe::Supported
            } else {
                Probe::Unsupported
            };
        }
    }

    /// The DA1 reply arrived: if the graphics answer has not, it never will.
    pub fn device_attributes(&mut self) {
        if matches!(self.probe, Probe::Sent(_)) {
            self.probe = Probe::Unsupported;
        }
    }

    /// Where the probe stands at `now` (a `Sent` older than
    /// [`PROBE_TIMEOUT`] becomes `Unsupported` here).
    pub fn probe(&mut self, now: Instant) -> Probe {
        if let Probe::Sent(at) = self.probe
            && now.saturating_duration_since(at) >= PROBE_TIMEOUT
        {
            self.probe = Probe::Unsupported;
        }
        self.probe
    }

    // ---- open / close / keys ---------------------------------------------

    /// Whether the picture is on screen.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `^G`: open the view (only on a supported terminal) or close it.
    /// Returns whether it is open afterwards.
    pub fn toggle(&mut self, now: Instant) -> bool {
        if self.active {
            self.close();
        } else if self.probe(now) == Probe::Supported {
            self.active = true;
            self.last_tick = Some(now);
            self.last_sent = None;
        }
        self.active
    }

    fn close(&mut self) {
        self.active = false;
        self.turn = 0;
        self.close_pending = true;
    }

    /// Whether a close happened since the last call: the caller frees the
    /// image.
    pub fn take_close(&mut self) -> bool {
        std::mem::take(&mut self.close_pending)
    }

    /// A key while the view is open. Left/`a` and Right/`d` toggle a turn,
    /// space stops it, `c` recentres the camera on the sim's heading, Esc
    /// and `q` close. Every other key is swallowed: the view is modal.
    pub fn key(&mut self, key: Key) {
        match key {
            Key::Left | Key::Char('a') => self.turn = if self.turn == -1 { 0 } else { -1 },
            Key::Right | Key::Char('d') => self.turn = if self.turn == 1 { 0 } else { 1 },
            Key::Char(' ') => self.turn = 0,
            Key::Char('c') => {
                self.turn = 0;
                self.offset_mdeg = 0;
            }
            // The load experiment's two switches (2026-10-21): the frame
            // size and the clock cap. Either resets the last-sent key so the
            // next tick sends whatever the new shape is.
            Key::Char('r') => {
                let (w, h) = if self.frame.width() == FRAME_W {
                    (FRAME_W_LO, FRAME_H_LO)
                } else {
                    (FRAME_W, FRAME_H)
                };
                self.frame = Frame::new(w, h);
                self.last_sent = None;
            }
            Key::Char('u') => {
                self.uncapped = !self.uncapped;
                self.last_sent = None;
            }
            Key::Esc | Key::Char('q') => self.close(),
            _ => {}
        }
    }

    /// The camera's offset from the sim heading, millidegrees in
    /// `-180_000..180_000`.
    #[must_use]
    pub fn offset_mdeg(&self) -> i64 {
        (self.offset_mdeg + 180_000).rem_euclid(360_000) - 180_000
    }

    /// Which way the camera is turning: `-1`, `0`, `1`.
    #[must_use]
    pub fn turn(&self) -> i8 {
        self.turn
    }

    // ---- the frame clock -------------------------------------------------

    /// Advance the turn by the time since the last tick.
    pub fn tick(&mut self, now: Instant) {
        let dt = self
            .last_tick
            .map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        self.last_tick = Some(now);
        if self.turn != 0 {
            let ms = i64::try_from(dt.as_millis()).unwrap_or(0).min(1_000);
            self.offset_mdeg += i64::from(self.turn) * TURN_MDEG_PER_S * ms / 1_000;
            self.offset_mdeg = self.offset_mdeg.rem_euclid(360_000);
        }
    }

    /// Render the picture from `cockpit` (the held `arena.cockpit` frame)
    /// and `map` (the held `arena.map`, if any) and return the kitty bytes
    /// that place it in a terminal `cols` by `rows` — or nothing when the
    /// view is closed, unsupported, has no frame, or the picture has not
    /// changed since the last send.
    pub fn paint(
        &mut self,
        cockpit: Option<&Value>,
        map: Option<&Value>,
        cols: u16,
        rows: u16,
        now: Instant,
    ) -> Vec<u8> {
        if !self.active || self.probe(now) != Probe::Supported {
            return Vec::new();
        }
        let Some(cockpit) = cockpit else {
            return Vec::new();
        };
        self.ensure_map(map);
        let heading = (cockpit
            .get("heading_mdeg")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + self.offset_mdeg)
            .rem_euclid(360_000);
        let cam = Camera {
            x_cm: cockpit.get("x_cm").and_then(Value::as_i64).unwrap_or(0),
            y_cm: cockpit.get("y_cm").and_then(Value::as_i64).unwrap_or(0),
            heading_mdeg: u32::try_from(heading).unwrap_or(0),
        };
        let sprites = contacts_as_sprites(cockpit, &cam);
        let (col, row, c, r) = image_box(cols, rows, self.frame.width(), self.frame.height());

        let t0 = Instant::now();
        if let Some((_, arena)) = &self.map {
            render(&mut self.frame, arena, &cam, &sprites);
            // The instruments, over the picture (the owner's 640x480 target
            // of 2026-10-21): minimap, heat, weapons, the armour doll and the
            // status strip, all from the frame the TUI's panes already read.
            draw_instruments(&mut self.frame, arena, cockpit, map, &cam);
        }
        let render_us = t0.elapsed().as_micros();

        let key = (self.frame.hash(), c, r);
        if self.last_sent == Some(key) {
            self.stats.skipped += 1;
            return Vec::new();
        }
        self.stats.render_us = render_us;
        let t1 = Instant::now();
        let bytes = png::encode(&self.frame);
        self.stats.encode_us = t1.elapsed().as_micros();
        self.stats.png_bytes = bytes.len();
        let wire = kitty::frame_bytes(&bytes, IMAGE_ID, col, row, c, r);
        self.stats.wire_bytes = wire.len();
        self.stats.sent += 1;
        self.sent_at.push_back(now);
        while self
            .sent_at
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= Duration::from_secs(1))
        {
            self.sent_at.pop_front();
        }
        self.last_sent = Some(key);
        wire
    }

    /// Build (or keep) the arena map from the held `arena.map`.
    fn ensure_map(&mut self, map: Option<&Value>) {
        let wanted = map.and_then(|m| m.get("match").and_then(Value::as_i64));
        if let Some((have, _)) = &self.map
            && *have == wanted
            && (map.is_some() || wanted.is_none())
        {
            return;
        }
        let built = map.map_or_else(
            || Map::new(OPEN_MAP_CELLS, OPEN_MAP_CELLS, 1000),
            map_from_feed,
        );
        self.map = Some((wanted, built));
    }

    /// The HUD's two lines: the top status and the bottom key legend.
    #[must_use]
    pub fn hud(&mut self, cockpit: Option<&Value>, now: Instant) -> (String, String) {
        let heading = cockpit
            .and_then(|c| c.get("heading_mdeg"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let camera = (heading + self.offset_mdeg).rem_euclid(360_000) / 1_000;
        let turning = match self.turn {
            -1 => "turning left",
            1 => "turning right",
            _ => "steady",
        };
        let probe = match self.probe(now) {
            Probe::NotSent => "not probed",
            Probe::Sent(_) => "probing",
            Probe::Supported => "kitty graphics OK",
            Probe::Unsupported => "no kitty graphics",
        };
        let s = self.stats;
        // The skipped-tick counter is NOT printed: a line that changes every
        // 50 ms would make the text frame repaint on every tick the picture
        // did not, which is twenty small writes a second for nothing (and a
        // slow client's SSH window fills up on them). Everything printed here
        // changes only when a frame is sent or a key is pressed.
        // `fps` is the frames of the last second, read at the last send (the
        // HUD is drawn only under a sent picture, so this never ticks a
        // repaint of its own).
        let fps = self.sent_at.back().map_or(0, |last| self.fps(*last));
        let clock = if self.uncapped { "uncapped" } else { "20 Hz" };
        let (w, h) = self.size();
        let top = format!(
            "PIXEL VIEW  camera {camera:03}° (sim {:03}° {:+}°)  {turning}  |  {probe}  {w}x{h} {clock}  render {:.1} ms  encode {:.1} ms  png {} B  wire {} B  sent {}  fps {fps}",
            heading / 1_000,
            self.offset_mdeg() / 1_000,
            s.render_us as f64 / 1000.0,
            s.encode_us as f64 / 1000.0,
            s.png_bytes,
            s.wire_bytes,
            s.sent,
        );
        let bottom = if cockpit.is_some() {
            "←/a and →/d: toggle a turn   space: hold   c: recentre   r: 640x480/320x220   u: 20 Hz/uncapped   Esc/q: close   ^]: detach"
                .to_string()
        } else {
            "No cockpit frame held: take a seat in the arena, then ^G.   Esc/q: close".to_string()
        };
        (top, bottom)
    }
}

/// The cell box a `w` by `h` picture is scaled into: below the top HUD
/// line, above the bottom one, as wide as keeps roughly the frame's aspect
/// at the usual 1:2 cell. Returns `(col, row, cols, rows)`, 1-based.
#[must_use]
pub fn image_box(cols: u16, rows: u16, w: usize, h: usize) -> (u16, u16, u16, u16) {
    let avail_rows = rows.saturating_sub(2).max(1);
    let avail_cols = cols.max(1);
    let (fw, fh) = (w.max(1) as u32, h.max(1) as u32);
    let wide = u32::from(avail_rows) * 2 * fw / fh;
    let (c, r) = if wide <= u32::from(avail_cols) {
        (u16::try_from(wide).unwrap_or(avail_cols).max(1), avail_rows)
    } else {
        let tall = u32::from(avail_cols) * fh / (2 * fw);
        (avail_cols, u16::try_from(tall).unwrap_or(avail_rows).clamp(1, avail_rows))
    };
    (1, 2, c, r)
}

/// An `arena.map` keyframe (run-length rows, south row first, a legend of
/// `[glyph, word]`) as a raycaster map.
fn map_from_feed(map: &Value) -> Map {
    let width = map.get("width").and_then(Value::as_u64).unwrap_or(0) as usize;
    let height = map.get("height").and_then(Value::as_u64).unwrap_or(0) as usize;
    let cell_cm = map
        .get("cell_cm")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1000);
    let empty = Vec::new();
    let rows = map.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    let grid = mush_ansi::window_rows(rows, width, height, 0, 0, width, height);
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
    Map::from_glyph_rows(
        &grid,
        &|g| {
            legend
                .iter()
                .find(|(c, _)| *c == g)
                .map_or(Ground::Open, |(_, ground)| *ground)
        },
        cell_cm,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const COCKPIT: &str =
        include_str!("../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");
    const MAP: &str = include_str!("../../../../web/src/lib/fixtures/cockpit/arena_map.json");

    fn fixtures() -> (Value, Value) {
        (
            serde_json::from_str(COCKPIT).expect("cockpit fixture"),
            serde_json::from_str(MAP).expect("map fixture"),
        )
    }

    fn supported(now: Instant) -> PixelView {
        let mut v = PixelView::new();
        v.probe_sent(now);
        v.graphics_reply(true);
        v
    }

    #[test]
    fn the_probe_is_ok_before_da1_or_nothing() {
        let t0 = Instant::now();
        // OK first, then DA1: supported, and DA1 does not undo it.
        let mut v = PixelView::new();
        assert_eq!(v.probe(t0), Probe::NotSent);
        v.probe_sent(t0);
        v.graphics_reply(true);
        v.device_attributes();
        assert_eq!(v.probe(t0 + Duration::from_secs(5)), Probe::Supported);
        // DA1 first: unsupported, and a late OK does not revive it.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        v.device_attributes();
        v.graphics_reply(true);
        assert_eq!(v.probe(t0), Probe::Unsupported);
        // An error reply.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        v.graphics_reply(false);
        assert_eq!(v.probe(t0), Probe::Unsupported);
        // Silence: still pending inside the window, unsupported after it.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        assert_eq!(v.probe(t0 + Duration::from_millis(400)), Probe::Sent(t0));
        assert_eq!(v.probe(t0 + PROBE_TIMEOUT), Probe::Unsupported);
        // A reply before any probe went out is ignored.
        let mut v = PixelView::new();
        v.graphics_reply(true);
        assert_eq!(v.probe(t0), Probe::NotSent);
    }

    #[test]
    fn the_view_opens_only_on_support_and_closes_with_a_free() {
        let t0 = Instant::now();
        let mut v = PixelView::new();
        assert!(!v.toggle(t0), "no probe, no view");
        v.probe_sent(t0);
        v.device_attributes();
        assert!(!v.toggle(t0), "unsupported, no view");
        let mut v = supported(t0);
        assert!(v.toggle(t0));
        assert!(v.is_active());
        assert!(!v.take_close());
        v.key(Key::Esc);
        assert!(!v.is_active());
        assert!(v.take_close());
        assert!(!v.take_close(), "a close is reported once");
    }

    #[test]
    fn toggle_turn_swings_the_camera_until_toggled_again() {
        let t0 = Instant::now();
        let mut v = supported(t0);
        v.toggle(t0);
        v.key(Key::Left);
        assert_eq!(v.turn(), -1);
        v.tick(t0 + Duration::from_millis(500));
        assert_eq!(v.offset_mdeg(), -22_500, "45°/s for half a second");
        v.key(Key::Left);
        assert_eq!(v.turn(), 0, "the same key stops the turn");
        v.tick(t0 + Duration::from_secs(2));
        assert_eq!(v.offset_mdeg(), -22_500, "still");
        v.key(Key::Char('d'));
        v.tick(t0 + Duration::from_secs(3));
        assert_eq!(v.offset_mdeg(), 22_500);
        // A long gap is capped at a second so a stalled pump does not spin
        // the camera.
        v.tick(t0 + Duration::from_secs(60));
        assert_eq!(v.offset_mdeg(), 67_500);
        v.key(Key::Char('c'));
        assert_eq!((v.turn(), v.offset_mdeg()), (0, 0));
        // Other keys are swallowed without effect.
        v.key(Key::Char('1'));
        assert!(v.is_active());
    }

    #[test]
    fn the_fixture_scene_paints_once_then_skips_until_the_camera_moves() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let mut v = supported(t0);
        // Closed: nothing.
        assert!(v.paint(Some(&cockpit), Some(&map), 120, 40, t0).is_empty());
        v.toggle(t0);
        // No frame held: nothing.
        assert!(v.paint(None, Some(&map), 120, 40, t0).is_empty());
        let first = v.paint(Some(&cockpit), Some(&map), 120, 40, t0);
        assert!(!first.is_empty());
        let s = String::from_utf8(first.clone()).expect("ascii");
        assert!(s.starts_with("\x1b[?2026h\x1b[2;1H\x1b_Ga=T,f=100,t=d,i=1,p=1,q=2,C=1,c="));
        assert!(s.ends_with("\x1b[?2026l"));
        assert_eq!(v.stats.sent, 1);
        assert!(v.stats.png_bytes > 0 && v.stats.png_bytes < 40_000, "{}", v.stats.png_bytes);
        // The same picture again: skipped.
        assert!(v.paint(Some(&cockpit), Some(&map), 120, 40, t0).is_empty());
        assert_eq!(v.stats.skipped, 1);
        // A resize re-sends into the new box even though the pixels match.
        assert!(!v.paint(Some(&cockpit), Some(&map), 160, 50, t0).is_empty());
        // Turn, tick: a new picture.
        v.key(Key::Right);
        v.tick(t0 + Duration::from_millis(200));
        let turned = v.paint(Some(&cockpit), Some(&map), 160, 50, t0);
        assert!(!turned.is_empty());
        assert_eq!(v.stats.sent, 3);
        assert_eq!(v.fps(t0 + Duration::from_millis(200)), 3, "three frames inside a second");
        assert_eq!(v.fps(t0 + Duration::from_secs(2)), 0);
        // Without a map the view still paints (open ground).
        let mut bare = supported(t0);
        bare.toggle(t0);
        assert!(!bare.paint(Some(&cockpit), None, 120, 40, t0).is_empty());
        let (top, bottom) = bare.hud(Some(&cockpit), t0);
        assert!(top.contains("camera 045°") && top.contains("kitty graphics OK"), "{top}");
        assert!(bottom.contains("toggle a turn"));
    }

    #[test]
    fn the_image_box_keeps_the_aspect_and_fits() {
        // 40 rows: 38 for the picture, 110 cells wide at 1:2 cells.
        assert_eq!(image_box(120, 40, 320, 220), (1, 2, 110, 38));
        // A narrow terminal: width-bound.
        let (_, _, c, r) = image_box(80, 60, 320, 220);
        assert_eq!(c, 80);
        assert!((1..=58).contains(&r));
        assert_eq!(image_box(1, 1, 320, 220), (1, 2, 1, 1));
        // The 4:3 frame is squarer: 38 rows take 101 columns.
        assert_eq!(image_box(120, 40, 640, 480), (1, 2, 101, 38));
    }

    /// The load experiment's switches (2026-10-21): `r` swaps the frame
    /// size and `u` uncaps the clock; either re-sends on the next tick.
    #[test]
    fn r_swaps_the_size_and_u_uncaps_the_clock() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let mut v = supported(t0);
        v.toggle(t0);
        assert_eq!(v.size(), (640, 480), "the target frame is the default");
        assert_eq!(v.interval(), FRAME_INTERVAL);
        assert!(!v.paint(Some(&cockpit), Some(&map), 120, 40, t0).is_empty());
        assert!(v.paint(Some(&cockpit), Some(&map), 120, 40, t0).is_empty());
        v.key(Key::Char('r'));
        assert_eq!(v.size(), (320, 220));
        let small = String::from_utf8_lossy(&v.paint(Some(&cockpit), Some(&map), 120, 40, t0)).into_owned();
        assert!(small.contains("c=110,r=38"), "{}", &small[..80]);
        assert!(v.stats.png_bytes > 0);
        v.key(Key::Char('u'));
        assert!(v.uncapped());
        assert_eq!(v.interval(), FRAME_INTERVAL_UNCAPPED);
        assert!(!v.paint(Some(&cockpit), Some(&map), 120, 40, t0).is_empty(), "re-sent after the switch");
        let (top, bottom) = v.hud(Some(&cockpit), t0);
        assert!(top.contains("320x220 uncapped"), "{top}");
        assert!(bottom.contains("640x480/320x220") && bottom.contains("uncapped"));
        v.key(Key::Char('r'));
        v.key(Key::Char('u'));
        assert_eq!((v.size(), v.uncapped()), ((640, 480), false));
    }

    #[test]
    fn the_fixture_map_decodes_its_legend() {
        let (_, map) = fixtures();
        let m = map_from_feed(&map);
        assert_eq!((m.width(), m.height(), m.cell_cm()), (96, 64, 1000));
        // Row 2 has water at x 20..30 (the fixture's pond), grass elsewhere.
        assert_eq!(m.get(25, 2), Ground::Water);
        assert_eq!(m.get(0, 0), Ground::Grass);
    }
}
