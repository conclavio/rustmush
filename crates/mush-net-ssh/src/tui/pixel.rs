//! **The pixel cockpit view** (spike 2026-10-21; doc 21 cuts G.1, G.2): a
//! first-person picture of the arena streamed over the kitty graphics
//! protocol or as sixel, drawn by `mush-pixel`'s cockpit [`Picture`] from the
//! feeds the TUI already holds.
//!
//! What lives here is the per-connection state the SSH TUI keeps for it; the
//! picture itself (map, camera, sprites, instruments) is the engine's:
//!
//! - **The probe**: `@tui` sends a one-pixel kitty query and `CSI c` right
//!   after the first frame. A graphics OK arriving first means kitty; the
//!   DA1 reply arriving first, or neither within [`PROBE_TIMEOUT`], means
//!   none; the DA1 reply's attributes say whether the terminal draws sixel,
//!   whichever came first, and a `CSI 6 ; h ; w t` reply before it gives the
//!   cell size sixel is drawn in. The result is a [`Graphics`] record; the
//!   view opens only when it, or the player's `@pixel/encoder`, yields an
//!   encoder — kitty, then sixel.
//! - **Two encoders.** Kitty's picture is one image under one id, scaled by
//!   the terminal into the cell box. Sixel's is drawn into the cell grid at
//!   its own pixel size, so the frame is sized to fit the box in the
//!   terminal's cell pixels, sent again whenever the text
//!   frame wrote into its box ([`PixelView::text_touched`]), and wiped by a
//!   full text repaint when it closes or its box changes.
//! - **The camera**: the sim's own `x_cm`/`y_cm`/`heading_mdeg` plus a LOCAL
//!   heading offset the pilot turns with toggle-turn keys (press to start
//!   turning, press again to stop) — the camera on a leash. The sim is not
//!   told; this spike proves the picture and the pacing, not the order.
//! - **The frame clock**: the output pump ticks [`FRAME_INTERVAL`]; each
//!   tick advances the turn, renders, and sends the frame only when its
//!   pixels (or the cell box it is scaled into) changed since the last send.
//!
//! The picture, the PNG, the sixel stream and the kitty framing are
//! `mush-pixel`; the probe replies arrive through the key parser as
//! [`Key::GraphicsOk`], [`Key::GraphicsErr`], [`Key::CellSize`] and
//! [`Key::DeviceAttributes`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use mush_pixel::caps::{CellSize, DeviceAttributes, Encoder, EncoderChoice, Graphics};
use mush_pixel::cockpit::CockpitPicture;
use mush_pixel::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use mush_pixel::{Frame, kitty, png, sixel};
use serde_json::Value;

use super::keys::Key;

/// The rendered frame's size; a kitty terminal scales it into the cell box,
/// and a sixel frame is this or the largest of its aspect that fits.
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
/// The frame clock: 20 frames a second, bucket A's interval. The output
/// pump's idle wait while no view is open.
pub const FRAME_INTERVAL: Duration = BUCKET_A_INTERVAL;
/// How fast a toggled turn swings the camera, millidegrees a second (45°/s).
const TURN_MDEG_PER_S: i64 = 45_000;

/// Where the capability probe stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Not sent yet (line mode, or a session that never entered the TUI).
    NotSent,
    /// Sent at this instant; waiting on an answer.
    Sent(Instant),
    /// Over: answered (kitty OK or error, or the DA1 reply) or timed out;
    /// what it found is [`PixelView::graphics`].
    Done,
}

/// Counters the HUD prints and the notes record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The last frame's render time, microseconds.
    pub render_us: u128,
    /// The last frame's encode time (PNG or sixel), microseconds.
    pub encode_us: u128,
    /// The last frame's encoded image, bytes: the PNG, or the sixel stream.
    pub image_bytes: usize,
    /// The last frame's bytes on the wire (base64, chunk headers, cursor
    /// move, sync).
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
    /// What the probe found: kitty from its answer, sixel from the DA1
    /// reply's attributes (recorded even when it follows a kitty OK).
    graphics: Graphics,
    /// The DA1 reply itself, for the HUD and `@pixel/stats` (G.1b).
    da1: Option<DeviceAttributes>,
    /// The cell size the terminal reported, if it did (sixel's box).
    cell: Option<CellSize>,
    /// What `@pixel/encoder` set for this connection.
    choice: EncoderChoice,
    active: bool,
    /// `-1` turning left, `0` still, `1` turning right.
    turn: i8,
    /// The camera's heading offset from the sim's, millidegrees.
    offset_mdeg: i64,
    last_tick: Option<Instant>,
    /// The hash of the last frame sent, with the cell box it went into and
    /// its encoder.
    last_sent: Option<(u64, u16, u16, Encoder)>,
    /// The frame `r` asks for: 640x480, or 320x220.
    want: (usize, usize),
    frame: Frame,
    /// The encoder of the picture now on screen, and the cell box it is in
    /// (`col`, `row` 1-based, `cols`, `rows`): what a close or a resize has
    /// to clean up after.
    shown: Option<(Encoder, (u16, u16, u16, u16))>,
    /// A sixel box moved or shrank: the next frame goes out under a full
    /// text repaint, which wipes the old picture's pixels.
    wipe_pending: bool,
    /// What the view draws: the engine's producer, instruments and clock.
    picture: Box<dyn Picture>,
    /// The view closed: what was on screen, for the caller to free or wipe.
    closed: Option<Encoder>,
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
            graphics: Graphics::default(),
            da1: None,
            cell: None,
            choice: EncoderChoice::Auto,
            active: false,
            turn: 0,
            offset_mdeg: 0,
            last_tick: None,
            last_sent: None,
            want: (FRAME_W, FRAME_H),
            frame: Frame::new(FRAME_W, FRAME_H),
            shown: None,
            wipe_pending: false,
            picture: Box::new(CockpitPicture::new()),
            closed: None,
            uncapped: false,
            sent_at: VecDeque::new(),
            stats: Stats::default(),
        }
    }

    /// The frame clock's wait: the picture's own (20 Hz for the cockpit, in
    /// either encoder), or ~1 ms when uncapped.
    #[must_use]
    pub fn interval(&self) -> Duration {
        if self.uncapped {
            FRAME_INTERVAL_UNCAPPED
        } else {
            self.picture.interval()
        }
    }

    /// Whether the clock is uncapped.
    #[must_use]
    pub fn uncapped(&self) -> bool {
        self.uncapped
    }

    /// The size of the frame last drawn, in pixels (`r` takes effect at the
    /// next frame).
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

    /// The graphics query was answered: the probe is over, kitty or not.
    pub fn graphics_reply(&mut self, ok: bool) {
        if matches!(self.probe, Probe::Sent(_)) {
            self.graphics.kitty = ok;
            self.probe = Probe::Done;
        }
    }

    /// The cell-size report arrived (it precedes the DA1 reply).
    pub fn cell_size(&mut self, cell: CellSize) {
        if self.probe != Probe::NotSent {
            self.cell = Some(cell);
        }
    }

    /// The DA1 reply arrived: if the graphics answer has not, it never will.
    /// Its attributes are recorded either way, so a terminal that answered
    /// kitty OK first still reports whether it draws sixel.
    pub fn device_attributes(&mut self, da1: DeviceAttributes) {
        if self.probe == Probe::NotSent {
            return;
        }
        self.graphics.sixel = da1.sixel();
        self.da1 = Some(da1);
        self.probe = Probe::Done;
    }

    /// Where the probe stands at `now` (a `Sent` older than
    /// [`PROBE_TIMEOUT`] is `Done`, having found nothing, from here on).
    pub fn probe(&mut self, now: Instant) -> Probe {
        if let Probe::Sent(at) = self.probe
            && now.saturating_duration_since(at) >= PROBE_TIMEOUT
        {
            self.probe = Probe::Done;
        }
        self.probe
    }

    /// What the probe found so far.
    #[must_use]
    pub fn graphics(&self) -> Graphics {
        self.graphics
    }

    /// The encoder this connection's pictures go out in: the player's forced
    /// one, or the probe's once it is over; `None` while the probe runs, when
    /// the terminal showed none we have, or with `@pixel/encoder off`.
    pub fn encoder(&mut self, now: Instant) -> Option<Encoder> {
        self.probe(now);
        self.current_encoder()
    }

    /// [`encoder`](Self::encoder) as it stands, without aging the probe.
    fn current_encoder(&self) -> Option<Encoder> {
        self.choice
            .encoder((self.probe == Probe::Done).then_some(self.graphics))
    }

    /// What `@pixel/encoder` set.
    #[must_use]
    pub fn choice(&self) -> EncoderChoice {
        self.choice
    }

    /// `@pixel/encoder <choice>`: a change closes an open view, so the next
    /// `^G` opens in the new encoder and the old picture is cleaned up as
    /// any close is.
    pub fn set_choice(&mut self, choice: EncoderChoice) {
        if choice != self.choice && self.active {
            self.close();
        }
        self.choice = choice;
    }

    /// What the probe found, in the words the HUD and `@pixel/encoder` use.
    pub fn probe_words(&mut self, now: Instant) -> &'static str {
        match (self.probe(now), self.graphics) {
            (Probe::NotSent, _) => "not probed",
            (Probe::Sent(_), _) => "probing",
            (Probe::Done, Graphics { kitty: true, .. }) => "kitty graphics OK",
            (Probe::Done, Graphics { sixel: true, .. }) => "sixel graphics OK",
            (Probe::Done, _) => "no terminal graphics",
        }
    }

    // ---- open / close / keys ---------------------------------------------

    /// Whether the picture is on screen.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `^G`: open the view (only where the probe found an encoder) or close
    /// it. Returns whether it is open afterwards.
    pub fn toggle(&mut self, now: Instant) -> bool {
        if self.active {
            self.close();
        } else if self.encoder(now).is_some() {
            self.active = true;
            self.last_tick = Some(now);
            self.last_sent = None;
        }
        self.active
    }

    fn close(&mut self) {
        self.active = false;
        self.turn = 0;
        self.last_sent = None;
        self.wipe_pending = false;
        if let Some((encoder, _)) = self.shown.take() {
            self.closed = Some(encoder);
        }
    }

    /// The encoder of the picture a close since the last call took off
    /// screen, if one was shown: the caller frees a kitty image, or repaints
    /// the whole text frame over a sixel one.
    pub fn take_close(&mut self) -> Option<Encoder> {
        self.closed.take()
    }

    /// The encoder of the picture on screen, if any.
    #[must_use]
    pub fn shown(&self) -> Option<Encoder> {
        self.shown.map(|(e, _)| e)
    }

    /// The text frame wrote into the picture's cell box ([`sixel_box`](Self::sixel_box))
    /// or cleared the screen. A sixel picture there is gone, so the next
    /// tick sends it again; a kitty image sits above the text and is not
    /// touched.
    pub fn text_touched(&mut self) {
        if self.shown() == Some(Encoder::Sixel) {
            self.last_sent = None;
        }
    }

    /// The cell box of a sixel picture on screen, 0-based (`x`, `y`, `w`,
    /// `h`), for the backend to watch: a kitty picture needs no watching.
    #[must_use]
    pub fn sixel_box(&self) -> Option<(u16, u16, u16, u16)> {
        match self.shown {
            Some((Encoder::Sixel, (col, row, c, r))) => {
                Some((col.saturating_sub(1), row.saturating_sub(1), c, r))
            }
            _ => None,
        }
    }

    /// Whether the next picture must go out under a full text repaint (a
    /// sixel box moved or shrank and left pixels outside the new one).
    pub fn take_wipe(&mut self) -> bool {
        std::mem::take(&mut self.wipe_pending)
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
                self.want = if self.want == (FRAME_W, FRAME_H) {
                    (FRAME_W_LO, FRAME_H_LO)
                } else {
                    (FRAME_W, FRAME_H)
                };
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

    /// Draw the picture from `feeds` (the TUI's held keyframes) and return
    /// the bytes that place it in a terminal `cols` by `rows` — kitty or
    /// sixel — or nothing when the view is closed, has no encoder, the feeds
    /// carry no cockpit frame, or the picture has not changed since the last
    /// send.
    pub fn paint(&mut self, feeds: &dyn Feeds, cols: u16, rows: u16, now: Instant) -> Vec<u8> {
        if !self.active {
            return Vec::new();
        }
        let Some(encoder) = self.encoder(now) else {
            return Vec::new();
        };
        let (size, placed) = match encoder {
            Encoder::Kitty => (self.want, image_box(cols, rows, self.want.0, self.want.1)),
            Encoder::Sixel => sixel_box(
                cols,
                rows,
                self.want,
                self.cell.unwrap_or(CellSize::DEFAULT),
            ),
        };
        if (self.frame.width(), self.frame.height()) != size {
            self.frame = Frame::new(size.0, size.1);
        }
        let look = Look {
            offset_mdeg: self.offset_mdeg,
        };
        let t0 = Instant::now();
        if !self.picture.draw(&mut self.frame, feeds, &look) {
            return Vec::new();
        }
        let render_us = t0.elapsed().as_micros();
        let (col, row, c, r) = placed;

        let key = (self.frame.hash(), c, r, encoder);
        if self.last_sent == Some(key) {
            self.stats.skipped += 1;
            return Vec::new();
        }
        self.stats.render_us = render_us;
        let t1 = Instant::now();
        let (image, wire) = match encoder {
            Encoder::Kitty => {
                let png = png::encode(&self.frame);
                let wire = kitty::frame_bytes(&png, IMAGE_ID, col, row, c, r);
                (png.len(), wire)
            }
            Encoder::Sixel => {
                let stream = sixel::encode(&self.frame);
                let wire = sixel::frame_bytes(&stream, col, row);
                (stream.len(), wire)
            }
        };
        self.stats.encode_us = t1.elapsed().as_micros();
        self.stats.image_bytes = image;
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
        // A sixel picture whose box moved or shrank leaves the old pixels
        // outside the new box; a kitty image is replaced whole.
        if let Some((Encoder::Sixel, old)) = self.shown
            && old != placed
        {
            self.wipe_pending = true;
        }
        self.shown = Some((encoder, placed));
        wire
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
        let probe = self.probe_words(now);
        let drawing = match (self.current_encoder(), self.choice) {
            (Some(e), EncoderChoice::Forced(_)) => format!("{} (forced)", e.name()),
            (Some(e), _) => e.name().to_string(),
            (None, _) => "nothing".to_string(),
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
        let clock = if self.uncapped {
            "uncapped".to_string()
        } else {
            format!("{} Hz", 1_000 / self.interval().as_millis().max(1))
        };
        let (w, h) = self.size();
        let top = format!(
            "PIXEL VIEW  camera {camera:03}° (sim {:03}° {:+}°)  {turning}  |  {probe}, drawing {drawing}  {w}x{h} {clock}  render {:.1} ms  encode {:.1} ms  image {} B  wire {} B  sent {}  fps {fps}",
            heading / 1_000,
            self.offset_mdeg() / 1_000,
            s.render_us as f64 / 1000.0,
            s.encode_us as f64 / 1000.0,
            s.image_bytes,
            s.wire_bytes,
            s.sent,
        );
        let bottom = if cockpit.is_some() {
            "←/a and →/d: toggle a turn   space: hold   c: recentre   r: 640x480/320x220   u: clock/uncapped   Esc/q: close   ^]: detach"
                .to_string()
        } else {
            "No cockpit frame held: take a seat in the arena, then ^G.   Esc/q: close".to_string()
        };
        (top, bottom)
    }
}

/// A sixel picture's frame size and cell box (D-21-10 for sixel): `want`, or
/// the largest frame of its aspect that fits below the top HUD line and above
/// the bottom one in `cell`'s pixels, placed at the box's top left. The box
/// is the frame rounded up to whole cells, so the picture never reaches the
/// key legend and the terminal never scrolls under it. Returns the frame size
/// and `(col, row, cols, rows)`, 1-based.
#[must_use]
pub fn sixel_box(
    cols: u16,
    rows: u16,
    want: (usize, usize),
    cell: CellSize,
) -> ((usize, usize), (u16, u16, u16, u16)) {
    let (cw, ch) = (
        usize::from(cell.width.max(1)),
        usize::from(cell.height.max(1)),
    );
    let avail_cols = usize::from(cols.max(1));
    let avail_rows = usize::from(rows.saturating_sub(2).max(1));
    let (w, h) = sixel::fit(want, (avail_cols * cw, avail_rows * ch));
    let c = w.div_ceil(cw).clamp(1, avail_cols);
    let r = h.div_ceil(ch).clamp(1, avail_rows);
    (
        (w, h),
        (
            1,
            2,
            u16::try_from(c).unwrap_or(1),
            u16::try_from(r).unwrap_or(1),
        ),
    )
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
        (
            avail_cols,
            u16::try_from(tall)
                .unwrap_or(avail_rows)
                .clamp(1, avail_rows),
        )
    };
    (1, 2, c, r)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::panels::{ARENA_MAP_FEED, COCKPIT_FEED};
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

    /// The held feeds, by name, as the TUI's panel store would hold them.
    fn feeds(cockpit: Option<&Value>, map: Option<&Value>) -> HashMap<String, Value> {
        let mut held = HashMap::new();
        if let Some(c) = cockpit {
            held.insert(COCKPIT_FEED.to_string(), c.clone());
        }
        if let Some(m) = map {
            held.insert(ARENA_MAP_FEED.to_string(), m.clone());
        }
        held
    }

    fn da1(params: &str) -> DeviceAttributes {
        DeviceAttributes::parse(params).unwrap_or_default()
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
        let kitty = Graphics {
            kitty: true,
            sixel: false,
        };
        // OK first, then DA1: kitty, and DA1 does not undo it.
        let mut v = PixelView::new();
        assert_eq!(v.probe(t0), Probe::NotSent);
        v.probe_sent(t0);
        v.graphics_reply(true);
        v.device_attributes(da1("?62;22"));
        assert_eq!(v.probe(t0 + Duration::from_secs(5)), Probe::Done);
        assert_eq!(v.graphics(), kitty);
        assert_eq!(v.encoder(t0), Some(Encoder::Kitty));
        // DA1 first: nothing, and a late OK does not revive it.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        v.device_attributes(da1("?62;22"));
        v.graphics_reply(true);
        assert_eq!(v.probe(t0), Probe::Done);
        assert_eq!((v.graphics(), v.encoder(t0)), (Graphics::default(), None));
        // An error reply.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        v.graphics_reply(false);
        assert_eq!((v.probe(t0), v.encoder(t0)), (Probe::Done, None));
        // Silence: still pending inside the window, over after it.
        let mut v = PixelView::new();
        v.probe_sent(t0);
        assert_eq!(v.probe(t0 + Duration::from_millis(400)), Probe::Sent(t0));
        assert_eq!(v.encoder(t0 + Duration::from_millis(400)), None);
        assert_eq!(v.probe(t0 + PROBE_TIMEOUT), Probe::Done);
        assert_eq!(v.graphics(), Graphics::default());
        // A reply before any probe went out is ignored.
        let mut v = PixelView::new();
        v.graphics_reply(true);
        v.device_attributes(da1("?62;4"));
        assert_eq!(
            (v.probe(t0), v.graphics()),
            (Probe::NotSent, Graphics::default())
        );
    }

    /// A view probed as Windows Terminal answers: no kitty, DA1 with 4.
    fn windows_terminal(now: Instant) -> PixelView {
        let mut wt = PixelView::new();
        wt.probe_sent(now);
        wt.device_attributes(da1("?61;4;6;7;14;21;22;23;24;28;32;42"));
        wt
    }

    /// Sixel is read off the DA1 reply whichever answer came first — Windows
    /// Terminal (sixel, no kitty), WezTerm and w11term (both) — and drawn
    /// when kitty is not there (G.2).
    #[test]
    fn the_da1_reply_says_sixel_and_the_view_draws_it() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let both_feeds = feeds(Some(&cockpit), Some(&map));
        let mut wt = windows_terminal(t0);
        assert_eq!(
            wt.graphics(),
            Graphics {
                kitty: false,
                sixel: true
            }
        );
        assert_eq!(wt.encoder(t0), Some(Encoder::Sixel));
        assert!(wt.toggle(t0));
        assert_eq!(wt.interval(), FRAME_INTERVAL, "sixel runs at 20 Hz too");
        let first = String::from_utf8_lossy(&wt.paint(&both_feeds, 120, 40, t0)).into_owned();
        assert!(
            first.starts_with("\x1b[?2026h\x1b[2;1H\x1bP0;1;0q\"1;1;640;480"),
            "{}",
            &first[..60.min(first.len())]
        );
        assert!(first.ends_with("\x1b\\\x1b[?2026l"));
        assert!(
            wt.paint(&both_feeds, 120, 40, t0).is_empty(),
            "unchanged: skipped"
        );
        let (top, _) = wt.hud(Some(&cockpit), t0);
        assert!(top.contains("sixel graphics OK, drawing sixel"), "{top}");
        assert!(top.contains("20 Hz"), "{top}");
        // Both: kitty OK, then DA1 with 4.
        let mut both = supported(t0);
        both.device_attributes(da1("?62;4;22"));
        assert_eq!(
            both.graphics(),
            Graphics {
                kitty: true,
                sixel: true
            }
        );
        assert_eq!(both.encoder(t0), Some(Encoder::Kitty));
    }

    #[test]
    fn the_view_opens_only_on_support_and_closes_with_a_free() {
        let t0 = Instant::now();
        let mut v = PixelView::new();
        assert!(!v.toggle(t0), "no probe, no view");
        v.probe_sent(t0);
        assert!(!v.toggle(t0), "probing, no view");
        v.device_attributes(da1("?62;22"));
        assert!(!v.toggle(t0), "no graphics, no view");
        let mut v = supported(t0);
        assert!(v.toggle(t0));
        assert!(v.is_active());
        assert_eq!(v.take_close(), None);
        // Closed before a picture went out: nothing to free.
        v.key(Key::Esc);
        assert!(!v.is_active());
        assert_eq!(v.take_close(), None);
        // Closed with a picture on screen: its encoder, once.
        let (cockpit, map) = fixtures();
        v.toggle(t0);
        assert!(
            !v.paint(&feeds(Some(&cockpit), Some(&map)), 120, 40, t0)
                .is_empty()
        );
        assert_eq!(v.shown(), Some(Encoder::Kitty));
        v.key(Key::Esc);
        assert_eq!(v.take_close(), Some(Encoder::Kitty));
        assert_eq!(v.take_close(), None, "a close is reported once");
        assert_eq!(v.shown(), None);
    }

    /// The sixel frame is the wanted frame, or the largest of its aspect the
    /// box holds in the terminal's cell pixels, and the box is that frame in
    /// whole cells below the HUD line.
    #[test]
    fn a_sixel_frame_is_sized_by_the_cell_box() {
        let ten_by_twenty = CellSize::DEFAULT;
        // 120x40 at 10x20: 1200 by 760 pixels of room, the 640x480 fits in
        // 64 by 24 cells.
        assert_eq!(
            sixel_box(120, 40, (640, 480), ten_by_twenty),
            ((640, 480), (1, 2, 64, 24))
        );
        // A terminal that reported 8x16 cells: 80 by 30.
        let small = CellSize {
            width: 8,
            height: 16,
        };
        assert_eq!(
            sixel_box(120, 40, (640, 480), small),
            ((640, 480), (1, 2, 80, 30))
        );
        // 80x20: 18 rows of 20 px = 360, so 480x360.
        assert_eq!(
            sixel_box(80, 20, (640, 480), ten_by_twenty),
            ((480, 360), (1, 2, 48, 18))
        );
        // `r`'s frame.
        assert_eq!(
            sixel_box(120, 40, (320, 220), ten_by_twenty),
            ((320, 220), (1, 2, 32, 11))
        );
    }

    /// The view uses the reported cell size, sends a sixel picture again
    /// after the text frame wrote over it (a kitty image is not), and asks
    /// for a wipe when its box changes.
    #[test]
    fn a_sixel_picture_is_resent_after_text_and_wiped_when_its_box_moves() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let both_feeds = feeds(Some(&cockpit), Some(&map));
        let mut wt = windows_terminal(t0);
        wt.cell_size(CellSize {
            width: 8,
            height: 16,
        });
        wt.toggle(t0);
        assert!(!wt.paint(&both_feeds, 120, 40, t0).is_empty());
        assert_eq!(wt.sixel_box(), Some((0, 1, 80, 30)));
        assert!(!wt.take_wipe(), "the first picture moves nothing");
        assert!(wt.paint(&both_feeds, 120, 40, t0).is_empty());
        wt.text_touched();
        assert!(!wt.paint(&both_feeds, 120, 40, t0).is_empty(), "resent");
        // `r`: a smaller frame in a smaller box, under a wipe.
        wt.key(Key::Char('r'));
        assert!(!wt.paint(&both_feeds, 120, 40, t0).is_empty());
        assert_eq!(wt.size(), (320, 220));
        assert!(wt.take_wipe());
        assert!(!wt.take_wipe(), "once");
        wt.key(Key::Esc);
        assert_eq!(wt.take_close(), Some(Encoder::Sixel));
        assert_eq!(wt.sixel_box(), None);
        // Kitty: the text never touches the image.
        let mut kitty = supported(t0);
        kitty.toggle(t0);
        assert!(!kitty.paint(&both_feeds, 120, 40, t0).is_empty());
        assert_eq!(kitty.sixel_box(), None);
        kitty.text_touched();
        assert!(kitty.paint(&both_feeds, 120, 40, t0).is_empty());
    }

    /// `@pixel/encoder`: a forced encoder opens without the probe, `off`
    /// never opens, and a change closes an open view.
    #[test]
    fn a_forced_encoder_opens_without_the_probe_and_off_never_does() {
        let t0 = Instant::now();
        let mut v = PixelView::new();
        v.set_choice(EncoderChoice::Forced(Encoder::Sixel));
        assert_eq!(v.encoder(t0), Some(Encoder::Sixel), "no probe needed");
        assert!(v.toggle(t0));
        assert!(v.hud(None, t0).0.contains("drawing sixel (forced)"));
        v.set_choice(EncoderChoice::Forced(Encoder::Kitty));
        assert!(!v.is_active(), "a change closes the view");
        v.set_choice(EncoderChoice::Off);
        assert!(!v.toggle(t0));
        // Off beats a probe that found kitty.
        let mut k = supported(t0);
        k.set_choice(EncoderChoice::Off);
        assert_eq!(k.encoder(t0), None);
        k.set_choice(EncoderChoice::Auto);
        assert_eq!(k.encoder(t0), Some(Encoder::Kitty));
        assert_eq!(k.choice(), EncoderChoice::Auto);
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
        let both = feeds(Some(&cockpit), Some(&map));
        let map_only = feeds(None, Some(&map));
        let cockpit_only = feeds(Some(&cockpit), None);
        let mut v = supported(t0);
        // Closed: nothing.
        assert!(v.paint(&both, 120, 40, t0).is_empty());
        v.toggle(t0);
        // No frame held: nothing.
        assert!(v.paint(&map_only, 120, 40, t0).is_empty());
        let first = v.paint(&both, 120, 40, t0);
        assert!(!first.is_empty());
        let s = String::from_utf8(first.clone()).expect("ascii");
        assert!(s.starts_with("\x1b[?2026h\x1b[2;1H\x1b_Ga=T,f=100,t=d,i=1,p=1,q=2,C=1,c="));
        assert!(s.ends_with("\x1b[?2026l"));
        assert_eq!(v.stats.sent, 1);
        assert!(
            v.stats.image_bytes > 0 && v.stats.image_bytes < 40_000,
            "{}",
            v.stats.image_bytes
        );
        // The same picture again: skipped.
        assert!(v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!(v.stats.skipped, 1);
        // A resize re-sends into the new box even though the pixels match.
        assert!(!v.paint(&both, 160, 50, t0).is_empty());
        // Turn, tick: a new picture.
        v.key(Key::Right);
        v.tick(t0 + Duration::from_millis(200));
        let turned = v.paint(&both, 160, 50, t0);
        assert!(!turned.is_empty());
        assert_eq!(v.stats.sent, 3);
        assert_eq!(
            v.fps(t0 + Duration::from_millis(200)),
            3,
            "three frames inside a second"
        );
        assert_eq!(v.fps(t0 + Duration::from_secs(2)), 0);
        // Without a map the view still paints (open ground).
        let mut bare = supported(t0);
        bare.toggle(t0);
        assert!(!bare.paint(&cockpit_only, 120, 40, t0).is_empty());
        let (top, bottom) = bare.hud(Some(&cockpit), t0);
        assert!(
            top.contains("camera 045°") && top.contains("kitty graphics OK"),
            "{top}"
        );
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
        let both = feeds(Some(&cockpit), Some(&map));
        let mut v = supported(t0);
        v.toggle(t0);
        assert_eq!(v.size(), (640, 480), "the target frame is the default");
        assert_eq!(v.interval(), FRAME_INTERVAL);
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert!(v.paint(&both, 120, 40, t0).is_empty());
        v.key(Key::Char('r'));
        let small = String::from_utf8_lossy(&v.paint(&both, 120, 40, t0)).into_owned();
        assert_eq!(
            v.size(),
            (320, 220),
            "the next frame drawn is the small one"
        );
        assert!(small.contains("c=110,r=38"), "{}", &small[..80]);
        assert!(v.stats.image_bytes > 0);
        v.key(Key::Char('u'));
        assert!(v.uncapped());
        assert_eq!(v.interval(), FRAME_INTERVAL_UNCAPPED);
        assert!(
            !v.paint(&both, 120, 40, t0).is_empty(),
            "re-sent after the switch"
        );
        let (top, bottom) = v.hud(Some(&cockpit), t0);
        assert!(top.contains("320x220 uncapped"), "{top}");
        assert!(bottom.contains("640x480/320x220") && bottom.contains("uncapped"));
        v.key(Key::Char('r'));
        v.key(Key::Char('u'));
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!((v.size(), v.uncapped()), ((640, 480), false));
    }

    /// The feed names the engine's cockpit picture reads are the ones this
    /// TUI holds its keyframes under.
    #[test]
    fn the_picture_reads_the_feeds_the_tui_holds() {
        assert_eq!(mush_pixel::cockpit::COCKPIT_FEED, COCKPIT_FEED);
        assert_eq!(mush_pixel::cockpit::ARENA_MAP_FEED, ARENA_MAP_FEED);
        assert_eq!(
            CockpitPicture::new().feeds(),
            [COCKPIT_FEED, ARENA_MAP_FEED]
        );
    }
}
