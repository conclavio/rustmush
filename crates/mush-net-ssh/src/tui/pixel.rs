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
//! **The text cockpit** (2026-10-22, the owner: "the player is going to need
//! to be able to read and send chat messages while navigating"): the
//! picture takes the top two-thirds of the screen ([`picture_area`]) and is
//! drawn at that box's shape ([`frame_size`]), the flight HUD alone over it;
//! the armour doll, weapons and heat, contacts, notices and the chat are
//! terminal text in the bottom third (`super::cockpit_text`). Enter opens
//! the chat line, which owns the keys until Enter sends it or Esc goes back
//! to flying ([`PixelView::chat_opened`] stops a turn first).
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

use super::helm::{Helm, arc};
use super::keys::{Arrow, Key};

/// The rendered frame's width; its height is the picture box's shape
/// ([`frame_size`]). A kitty terminal scales the frame into the cell box,
/// and a sixel frame is this or the largest of its shape that fits.
/// **640 wide at 20 Hz is the target** (the owner's ruling of 2026-10-21,
/// after the load experiment measured 640x480 at 21 % of a vCPU and
/// 100 KB/s a viewer; the text cockpit's wider box is fewer pixels).
pub const FRAME_W: usize = 640;
/// The narrower frame `r` switches to, a quarter of the pixels and the
/// cost, for a slow link or a loaded box.
pub const FRAME_W_LO: usize = 320;
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
/// How fast a toggled look swings the camera, millidegrees a second (45°/s).
const TURN_MDEG_PER_S: i64 = 45_000;
/// Push the kitty keyboard protocol with "disambiguate" and "report event
/// types" (G.4): a held cursor key reports its release, which ends a turn.
/// Pushed when the view opens on a terminal that answered `CSI ? u`.
const KEYBOARD_PUSH: &[u8] = b"\x1b[>3u";
/// Pop it again when the view closes, so the rest of the TUI reads keys as
/// it always has.
const KEYBOARD_POP: &[u8] = b"\x1b[<u";

/// **The view's knobs** (doc 21 cut G.1b), from `[pixel]` in `mushd.toml`
/// through `SshHooks::pixel_config`: the clock, the step-down on a slow link,
/// and the box-wide staff diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelConfig {
    /// The picture's clock (20 Hz).
    pub frame_interval: Duration,
    /// The clock after a step-down (10 Hz).
    pub low_interval: Duration,
    /// Picture writes in a row, each slower than a frame, before stepping
    /// down.
    pub step_down_after: u32,
    /// How long the writes must keep up before stepping back up.
    pub step_up_after: Duration,
    /// The staff diagnostics, box-wide (the owner's ruling of 2026-10-22):
    /// the `u` key and the engineering numbers on the status line.
    pub diagnostics: bool,
}

impl Default for PixelConfig {
    fn default() -> Self {
        PixelConfig {
            frame_interval: FRAME_INTERVAL,
            low_interval: Duration::from_millis(100),
            step_down_after: 3,
            step_up_after: Duration::from_secs(10),
            diagnostics: false,
        }
    }
}

impl PixelConfig {
    /// The knobs from `[pixel]`'s numbers, held to sane bounds: the clock
    /// 10 ms to a second, the low clock never faster than it, at least one
    /// slow write and a second clean.
    #[must_use]
    pub fn from_millis(
        frame_interval_ms: u64,
        low_interval_ms: u64,
        step_down_after: u32,
        step_up_after_s: u64,
        diagnostics: bool,
    ) -> PixelConfig {
        let frame = frame_interval_ms.clamp(10, 1_000);
        PixelConfig {
            frame_interval: Duration::from_millis(frame),
            low_interval: Duration::from_millis(low_interval_ms.clamp(frame, 2_000)),
            step_down_after: step_down_after.max(1),
            step_up_after: Duration::from_secs(step_up_after_s.max(1)),
            diagnostics,
        }
    }
}

/// What a picture write did to the view's step-down (doc 21 cut G.1b), for
/// the box's counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepChange {
    /// Nothing changed.
    None,
    /// The writes fell behind: the next frame is skipped.
    Skip,
    /// They kept falling behind: the view stepped down to the narrow frame
    /// and the low clock (the next frame is skipped too).
    Down,
    /// They kept up long enough: the view stepped back up.
    Up,
}

/// A size in pixels, `(width, height)`.
pub type Pixels = (usize, usize);
/// A cell box, `(col, row, cols, rows)`, 1-based as `CUP` counts.
pub type CellBox = (u16, u16, u16, u16);
/// What makes two sent pictures the same: the frame hash, the box's
/// `cols` and `rows`, the encoder, the shown size.
type SentKey = (u64, u16, u16, Encoder, Pixels);

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
    /// Ticks skipped because the write before was slower than a frame (G.1b).
    pub slow_skips: u64,
    /// Times the view stepped down to the narrow frame and the low clock.
    pub step_downs: u64,
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
    /// The look (`,` and `.`): `-1` swinging left, `0` still, `1` right.
    look_turn: i8,
    /// The look's offset from the tank's heading, millidegrees: a look, not
    /// a turn (D-21-14).
    look_mdeg: i64,
    /// The tank's helm: the turn keys' orders and the predicted heading.
    helm: Helm,
    /// The terminal answered the kitty keyboard protocol's `CSI ? u`.
    keyboard: bool,
    /// The view pushed the protocol: cursor keys report their release, so
    /// a turn is held, not toggled.
    keyboard_pushed: bool,
    /// Bytes for the terminal ahead of the next text frame: the protocol's
    /// push and pop.
    term_bytes: Vec<u8>,
    last_tick: Option<Instant>,
    /// What the last picture sent was: its frame hash, the cell box it went
    /// into, its encoder and the size it was shown at.
    last_sent: Option<SentKey>,
    /// The size the last picture was shown at: the frame's own for kitty
    /// (the terminal scales it), the box-filling size a sixel frame was
    /// encoded up to.
    shown_size: (usize, usize),
    /// The frame width `r` asks for: 640, or 320.
    want: usize,
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
    /// The knobs from `[pixel]` (G.1b).
    cfg: PixelConfig,
    /// Picture writes in a row slower than a frame.
    slow_streak: u32,
    /// The write before was slow: skip the next frame.
    skip_next: bool,
    /// Stepped down: the narrow frame and the low clock.
    stepped_down: bool,
    /// Since when the writes have kept up, while stepped down.
    clean_since: Option<Instant>,
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
            look_turn: 0,
            look_mdeg: 0,
            helm: Helm::new(),
            keyboard: false,
            keyboard_pushed: false,
            term_bytes: Vec::new(),
            last_tick: None,
            last_sent: None,
            want: FRAME_W,
            shown_size: (FRAME_W, FRAME_W * 3 / 4),
            frame: Frame::new(FRAME_W, FRAME_W * 3 / 4),
            shown: None,
            wipe_pending: false,
            picture: Box::new(CockpitPicture::new()),
            closed: None,
            uncapped: false,
            cfg: PixelConfig::default(),
            slow_streak: 0,
            skip_next: false,
            stepped_down: false,
            clean_since: None,
            sent_at: VecDeque::new(),
            stats: Stats::default(),
        }
    }

    /// The frame clock's wait: `[pixel]`'s clock (20 Hz, in either encoder),
    /// its low clock while stepped down, or ~1 ms when uncapped.
    #[must_use]
    pub fn interval(&self) -> Duration {
        if self.uncapped {
            FRAME_INTERVAL_UNCAPPED
        } else if self.stepped_down {
            self.cfg.low_interval
        } else {
            self.cfg.frame_interval
        }
    }

    /// Take `[pixel]`'s knobs (G.1b). Turning the diagnostics off also
    /// drops an uncapped clock.
    pub fn set_config(&mut self, cfg: PixelConfig) {
        self.cfg = cfg;
        if !cfg.diagnostics {
            self.uncapped = false;
        }
    }

    /// The knobs in force.
    #[must_use]
    pub fn config(&self) -> PixelConfig {
        self.cfg
    }

    /// Whether the view is stepped down.
    #[must_use]
    pub fn stepped_down(&self) -> bool {
        self.stepped_down
    }

    /// **The step-down** (doc 21 §3.4, cut G.1b): a picture write that took
    /// longer than a frame means the link is not keeping up — russh stops
    /// taking a channel's data once the client's window is full, so the write
    /// waits — and the next frame is skipped; [`PixelConfig::step_down_after`]
    /// such writes in a row step the view down to the narrow frame
    /// ([`FRAME_W_LO`]) and the low clock; writes that keep up for
    /// [`PixelConfig::step_up_after`] step it back up. Returns what changed,
    /// for the box's counters.
    pub fn write_took(&mut self, took: Duration, now: Instant) -> StepChange {
        if took > self.interval() {
            self.skip_next = true;
            self.slow_streak = self.slow_streak.saturating_add(1);
            self.clean_since = None;
            if !self.stepped_down && self.slow_streak >= self.cfg.step_down_after {
                self.stepped_down = true;
                self.stats.step_downs += 1;
                self.last_sent = None;
                return StepChange::Down;
            }
            return StepChange::Skip;
        }
        self.slow_streak = 0;
        if self.stepped_down {
            let since = *self.clean_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= self.cfg.step_up_after {
                self.stepped_down = false;
                self.clean_since = None;
                self.last_sent = None;
                return StepChange::Up;
            }
        }
        StepChange::None
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

    /// The kitty keyboard protocol's flags reply arrived: hold-to-turn is
    /// possible on this terminal.
    pub fn keyboard_reply(&mut self) {
        if self.probe != Probe::NotSent {
            self.keyboard = true;
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

    /// The chat line opened over the view: a turn in progress stops where
    /// the picture shows the tank, and the look stops swinging, so nothing
    /// steers while the pilot types.
    pub fn chat_opened(&mut self, now: Instant) {
        self.helm.stop(now);
        self.look_turn = 0;
    }

    /// The camera's heading as the view draws it, millidegrees: the frame's
    /// heading carried by the helm's prediction, plus the look.
    #[must_use]
    pub fn camera_mdeg(&self, cockpit: Option<&Value>, now: Instant) -> i64 {
        let heading = cockpit
            .and_then(|c| c.get("heading_mdeg"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        (self.helm.predicted(now).unwrap_or(heading) + self.look_mdeg).rem_euclid(360_000)
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
            if self.keyboard {
                self.term_bytes.extend_from_slice(KEYBOARD_PUSH);
                self.keyboard_pushed = true;
            }
        }
        self.active
    }

    fn close(&mut self) {
        self.active = false;
        self.look_turn = 0;
        // A turn in progress stops where the picture showed the tank.
        self.helm.stop(Instant::now());
        if std::mem::take(&mut self.keyboard_pushed) {
            self.term_bytes.extend_from_slice(KEYBOARD_POP);
        }
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

    /// Bytes for the terminal ahead of the next text frame: the kitty
    /// keyboard protocol's push when the view opened, its pop when it closed.
    pub fn take_terminal_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.term_bytes)
    }

    /// Whether the view has the kitty keyboard protocol pushed (a leave must
    /// pop it).
    #[must_use]
    pub fn keyboard_pushed(&self) -> bool {
        self.keyboard_pushed
    }

    /// Stop a turn in progress (leaving the TUI with the view open): the
    /// tank stops where the picture showed it.
    pub fn stop_helm(&mut self, now: Instant) {
        self.helm.stop(now);
    }

    /// The helm orders (`HEADING <deg>` lines) the view has made since the
    /// last call, to send to the world as typed lines.
    pub fn take_orders(&mut self) -> Vec<String> {
        self.helm.take_orders()
    }

    /// A key while the view is open (doc 21 cut G.4):
    ///
    /// - **Left / Right turn the tank**: a helm order, the authority the
    ///   `HEADING` verb has. Held where the terminal reports releases (the
    ///   kitty keyboard protocol), toggled (press again to stop) elsewhere.
    /// - `a` / `d` turn it too, always toggled.
    /// - `,` / `.` swing the **look**, the camera's own offset (toggled);
    ///   `c` recentres it on the tank.
    /// - space stops the tank's turn and the look's swing.
    /// - `r` and `u` are the load experiment's switches; Esc and `q` close.
    ///
    /// Every other key is swallowed: the view is modal. Enter, which opens
    /// the chat line, never reaches here (the app routes it).
    pub fn key(&mut self, key: Key, now: Instant) {
        let held = self.keyboard_pushed;
        let steer = |helm: &mut Helm, dir: i8, toggle: bool| {
            if helm.turning() == dir {
                if toggle {
                    helm.stop(now);
                }
            } else {
                helm.start(dir, now);
            }
        };
        match key {
            Key::Left => steer(&mut self.helm, -1, !held),
            Key::Right => steer(&mut self.helm, 1, !held),
            Key::Char('a') => steer(&mut self.helm, -1, true),
            Key::Char('d') => steer(&mut self.helm, 1, true),
            Key::Release(Arrow::Left) if self.helm.turning() == -1 => self.helm.stop(now),
            Key::Release(Arrow::Right) if self.helm.turning() == 1 => self.helm.stop(now),
            Key::Char(',') => self.look_turn = if self.look_turn == -1 { 0 } else { -1 },
            Key::Char('.') => self.look_turn = if self.look_turn == 1 { 0 } else { 1 },
            Key::Char(' ') => {
                self.helm.stop(now);
                self.look_turn = 0;
            }
            Key::Char('c') => {
                self.look_turn = 0;
                self.look_mdeg = 0;
            }
            // The load experiment's two switches (2026-10-21): the frame
            // size and the clock cap. Either resets the last-sent key so the
            // next tick sends whatever the new shape is.
            Key::Char('r') => {
                self.want = if self.want == FRAME_W {
                    FRAME_W_LO
                } else {
                    FRAME_W
                };
                self.last_sent = None;
            }
            // A staff diagnostic, box-wide (G.1b): only where `[pixel]
            // diagnostics` is on.
            Key::Char('u') if self.cfg.diagnostics => {
                self.uncapped = !self.uncapped;
                self.last_sent = None;
            }
            Key::Esc | Key::Char('q') => self.close(),
            _ => {}
        }
    }

    /// The look's offset from the tank's heading, millidegrees in
    /// `-180_000..180_000`.
    #[must_use]
    pub fn look_mdeg(&self) -> i64 {
        (self.look_mdeg + 180_000).rem_euclid(360_000) - 180_000
    }

    /// Which way the look is swinging: `-1`, `0`, `1`.
    #[must_use]
    pub fn look_turn(&self) -> i8 {
        self.look_turn
    }

    /// The tank's helm, as the view drives it.
    #[must_use]
    pub fn helm(&self) -> &Helm {
        &self.helm
    }

    // ---- the frame clock -------------------------------------------------

    /// Advance the look's swing by the time since the last tick, and let a
    /// held turn extend its order.
    pub fn tick(&mut self, now: Instant) {
        let dt = self
            .last_tick
            .map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        self.last_tick = Some(now);
        if self.look_turn != 0 {
            let ms = i64::try_from(dt.as_millis()).unwrap_or(0).min(1_000);
            self.look_mdeg += i64::from(self.look_turn) * TURN_MDEG_PER_S * ms / 1_000;
            self.look_mdeg = self.look_mdeg.rem_euclid(360_000);
        }
        self.helm.tick(now);
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
        // G.1b: the write before this tick's was slow; let the link drain.
        if std::mem::take(&mut self.skip_next) {
            self.stats.slow_skips += 1;
            return Vec::new();
        }
        let cell = self.cell.unwrap_or(CellSize::DEFAULT);
        let area = picture_area(cols, rows);
        let width = if self.stepped_down {
            self.want.min(FRAME_W_LO)
        } else {
            self.want
        };
        let want = frame_size(width, area, cell);
        let (size, shown, placed) = match encoder {
            Encoder::Kitty => (want, want, image_box(area, want, cell)),
            Encoder::Sixel => sixel_box(area, want, cell),
        };
        if (self.frame.width(), self.frame.height()) != size {
            self.frame = Frame::new(size.0, size.1);
        }
        // The camera: the tank's heading as the helm predicts it (the frame's,
        // carried toward the order between pulses), plus the look.
        let cockpit = feeds.feed(mush_pixel::cockpit::COCKPIT_FEED);
        let frame_heading = cockpit
            .and_then(|c| c.get("heading_mdeg"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if let Some(c) = cockpit {
            self.helm.observe(c, now);
        }
        let predicted = self.helm.predicted(now).unwrap_or(frame_heading);
        let hull_offset_mdeg = arc(frame_heading, predicted);
        let look = Look {
            offset_mdeg: hull_offset_mdeg + self.look_mdeg,
            hull_offset_mdeg,
            // An order the picture already shows reached is not marked.
            ordered_mdeg: self
                .helm
                .ordered()
                .filter(|o| arc(predicted, *o).abs() > 1_000),
        };
        let t0 = Instant::now();
        if !self.picture.draw(&mut self.frame, feeds, &look) {
            return Vec::new();
        }
        let render_us = t0.elapsed().as_micros();
        let (col, row, c, r) = placed;

        let key = (self.frame.hash(), c, r, encoder, shown);
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
                let stream = sixel::encode_scaled(&self.frame, shown.0, shown.1);
                let wire = sixel::frame_bytes(&stream, col, row);
                (stream.len(), wire)
            }
        };
        self.stats.encode_us = t1.elapsed().as_micros();
        self.shown_size = shown;
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
        // The picture's bytes move the cursor to its corner: save and restore
        // it around them, so the chat line's cursor stays where the text
        // frame put it.
        let mut out = Vec::with_capacity(wire.len() + 4);
        out.extend_from_slice(b"\x1b7");
        out.extend_from_slice(&wire);
        out.extend_from_slice(b"\x1b8");
        out
    }

    /// The HUD's two lines: the top status and the bottom key legend.
    #[must_use]
    pub fn hud(&mut self, cockpit: Option<&Value>, now: Instant) -> (String, String) {
        let heading = cockpit
            .and_then(|c| c.get("heading_mdeg"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let tank = self.helm.predicted(now).unwrap_or(heading);
        let camera = (tank + self.look_mdeg).rem_euclid(360_000) / 1_000;
        // An order the picture already shows reached is not news, even while
        // the frame that confirms it is a pulse away.
        let ordered = self
            .helm
            .ordered()
            .filter(|o| arc(tank, *o).abs() > 1_000)
            .map(|o| format!(" → {:03}°", o / 1_000))
            .unwrap_or_default();
        let turning = match self.helm.turning() {
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
        let size = if self.shown_size == (w, h) || self.shown() != Some(Encoder::Sixel) {
            format!("{w}x{h}")
        } else {
            format!("{w}x{h} shown {}x{}", self.shown_size.0, self.shown_size.1)
        };
        let slow = if self.stepped_down {
            "  (slow link: stepped down)"
        } else {
            ""
        };
        let mut top = format!(
            "PIXEL VIEW  camera {camera:03}° (tank {:03}°{ordered}, look {:+}°)  {turning}  |  {probe}, drawing {drawing}  {size} {clock}{slow}",
            heading / 1_000,
            self.look_mdeg() / 1_000,
        );
        // G.1b: the engineering numbers are the staff's (`[pixel]
        // diagnostics`); everyone's own are in `@pixel/stats`.
        if self.cfg.diagnostics {
            top.push_str(&format!(
                "  render {:.1} ms  encode {:.1} ms  image {} B  wire {} B  sent {}  fps {fps}",
                s.render_us as f64 / 1000.0,
                s.encode_us as f64 / 1000.0,
                s.image_bytes,
                s.wire_bytes,
                s.sent,
            ));
        }
        let bottom = if cockpit.is_some() {
            let arrows = if self.keyboard_pushed {
                "←/→: turn the tank (hold)"
            } else {
                "←/→ or a/d: turn the tank (again to stop)"
            };
            let clock = if self.cfg.diagnostics {
                "   u: clock"
            } else {
                ""
            };
            format!(
                "{arrows}   ,/.: look   space: stop   c: centre   Enter: chat   r: 640/320 wide{clock}   Esc/q: close   ^]: detach"
            )
        } else {
            "No cockpit frame held: take a seat in the arena, then ^G.   Esc/q: close".to_string()
        };
        (top, bottom)
    }
}

impl PixelView {
    /// **`@pixel/stats`** (doc 21 cut G.1b): this connection's picture in
    /// words — what the terminal showed and what is drawn, the frame and the
    /// clock, the last frame's costs and bytes, and the counts. Everyone's
    /// own numbers; the status line carries them only for staff.
    #[must_use]
    pub fn stats_lines(&mut self, now: Instant) -> Vec<String> {
        let s = self.stats;
        let drawing = self
            .current_encoder()
            .map_or("nothing".to_string(), |e| e.name().to_string());
        let (w, h) = self.size();
        let clock = if self.uncapped {
            "uncapped".to_string()
        } else {
            format!("{} Hz", 1_000 / self.interval().as_millis().max(1))
        };
        let fps = self.sent_at.back().map_or(0, |last| self.fps(*last));
        vec![
            format!(
                "Picture: {}, drawing {drawing}; {}.",
                self.probe_words(now),
                if self.active { "open" } else { "closed" }
            ),
            format!(
                "Frame {w}x{h} at {clock}{}; last frame render {:.1} ms, encode {:.1} ms, image {} B, wire {} B; {fps} frames in the last second.",
                if self.stepped_down {
                    ", stepped down for a slow link"
                } else {
                    ""
                },
                s.render_us as f64 / 1000.0,
                s.encode_us as f64 / 1000.0,
                s.image_bytes,
                s.wire_bytes,
            ),
            format!(
                "Sent {}, unchanged {}, skipped behind a slow write {}, stepped down {} times.",
                s.sent, s.skipped, s.slow_skips, s.step_downs
            ),
        ]
    }
}

/// How far a sixel frame is scaled up to fill its box, at most: three
/// times 640 wide is 1920, and the bytes grow with the rows shown.
pub const SIXEL_MAX_SCALE: usize = 3;

/// **The picture's area** (the text cockpit, 2026-10-22): every column,
/// from under the status line down two-thirds of what is below it; the
/// text panel, the chat line and the key legend take the rest.
/// `(col, row, cols, rows)`, 1-based.
#[must_use]
pub fn picture_area(cols: u16, rows: u16) -> CellBox {
    let below = u32::from(rows.saturating_sub(1));
    let tall = u16::try_from(below * 2 / 3).unwrap_or(u16::MAX).max(1);
    (1, 2, cols.max(1), tall)
}

/// The frame a picture `width` pixels wide is drawn at to fill `area`, of
/// `cell`-sized cells: the area's shape, its height between a quarter and
/// three-quarters of its width (a 4:3 frame at most), and even. The
/// renderers' vertical scale follows the width, so a wide frame shows the
/// world a 4:3 one does, cropped above and below.
#[must_use]
pub fn frame_size(width: usize, area: CellBox, cell: CellSize) -> Pixels {
    let (_, _, cols, rows) = area;
    let aw = usize::from(cols.max(1)) * usize::from(cell.width.max(1));
    let ah = usize::from(rows.max(1)) * usize::from(cell.height.max(1));
    let h = (width * ah / aw).clamp(width / 4, width * 3 / 4) & !1;
    (width, h.max(2))
}

/// A sixel picture's render size, shown size and cell box (D-21-10 for
/// sixel; G.2b fills the box). The picture is shown as large as `area`
/// holds at `want`'s shape, in `cell`'s pixels, up to [`SIXEL_MAX_SCALE`]
/// times `want`: a kitty terminal stretches its image over the box, and a
/// sixel one shows pixels as sent, so the encoder scales the frame up. A
/// frame is rendered at `want`, or at the shown size when that is smaller
/// (the HUD's text then drawn at the size it is seen). The box is the shown
/// size in whole cells, centred across the area at its top, so the picture
/// never reaches the text panel and the terminal never scrolls under it.
/// Returns the render size, the shown size and `(col, row, cols, rows)`,
/// 1-based.
#[must_use]
pub fn sixel_box(area: CellBox, want: Pixels, cell: CellSize) -> (Pixels, Pixels, CellBox) {
    let (col, row, cols, rows) = area;
    let (cw, ch) = (
        usize::from(cell.width.max(1)),
        usize::from(cell.height.max(1)),
    );
    let avail_cols = usize::from(cols.max(1));
    let avail_rows = usize::from(rows.max(1));
    let (w, h) = sixel::fill(want, (avail_cols * cw, avail_rows * ch), SIXEL_MAX_SCALE);
    let render = if w <= want.0 { (w, h) } else { want };
    let c = w.div_ceil(cw).clamp(1, avail_cols);
    let r = h.div_ceil(ch).clamp(1, avail_rows);
    let left = u16::try_from((avail_cols - c) / 2).unwrap_or(0);
    (
        render,
        (w, h),
        (
            col + left,
            row,
            u16::try_from(c).unwrap_or(1),
            u16::try_from(r).unwrap_or(1),
        ),
    )
}

/// The cell box a kitty picture of `size` is scaled into: the largest of its
/// shape `area` holds in `cell`-sized cells, centred across the area at its
/// top. Returns `(col, row, cols, rows)`, 1-based.
#[must_use]
pub fn image_box(area: CellBox, size: Pixels, cell: CellSize) -> CellBox {
    let (col, row, cols, rows) = area;
    let (cw, ch) = (u64::from(cell.width.max(1)), u64::from(cell.height.max(1)));
    let (fw, fh) = (size.0.max(1) as u64, size.1.max(1) as u64);
    let (avail_cols, avail_rows) = (u64::from(cols.max(1)), u64::from(rows.max(1)));
    // As wide as the area's height allows, in cells, or as tall as its width.
    let wide = (avail_rows * ch * fw / fh / cw).max(1);
    let (c, r) = if wide <= avail_cols {
        (wide, avail_rows)
    } else {
        (
            avail_cols,
            (avail_cols * cw * fh / fw / ch).clamp(1, avail_rows),
        )
    };
    let left = u16::try_from((avail_cols - c) / 2).unwrap_or(0);
    (
        col + left,
        row,
        u16::try_from(c).unwrap_or(1),
        u16::try_from(r).unwrap_or(1),
    )
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
            first.starts_with("\x1b7\x1b[?2026h\x1b[2;1H\x1bP0;1;0q\"1;1;1200;517"),
            "{}",
            &first[..60.min(first.len())]
        );
        assert!(first.ends_with("\x1b\\\x1b[?2026l\x1b8"));
        assert!(
            wt.paint(&both_feeds, 120, 40, t0).is_empty(),
            "unchanged: skipped"
        );
        let (top, _) = wt.hud(Some(&cockpit), t0);
        assert!(top.contains("sixel graphics OK, drawing sixel"), "{top}");
        assert!(top.contains("640x276 shown 1200x517 20 Hz"), "{top}");
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
        v.key(Key::Esc, t0);
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
        v.key(Key::Esc, t0);
        assert_eq!(v.take_close(), Some(Encoder::Kitty));
        assert_eq!(v.take_close(), None, "a close is reported once");
        assert_eq!(v.shown(), None);
    }

    /// The text cockpit's geometry: the picture's area is the top
    /// two-thirds under the status line, and the frame is drawn at that
    /// area's shape, 640 (or `r`'s 320) wide, between 4:1 and 4:3.
    #[test]
    fn the_frame_is_shaped_to_the_top_two_thirds() {
        let cell = CellSize::DEFAULT;
        assert_eq!(picture_area(160, 50), (1, 2, 160, 32));
        assert_eq!(picture_area(120, 40), (1, 2, 120, 26));
        assert_eq!(picture_area(1, 1), (1, 2, 1, 1));
        // 160 by 32 cells of 10x20 is 1600 by 640 pixels: 5:2.
        assert_eq!(frame_size(640, picture_area(160, 50), cell), (640, 256));
        assert_eq!(frame_size(320, picture_area(160, 50), cell), (320, 128));
        // 120 by 26: 1200 by 520, kept even.
        assert_eq!(frame_size(640, picture_area(120, 40), cell), (640, 276));
        // Wider than 4:1 is cut to it; taller than 4:3 likewise.
        assert_eq!(frame_size(640, picture_area(120, 20), cell), (640, 160));
        assert_eq!(frame_size(640, picture_area(80, 60), cell), (640, 480));
    }

    /// A sixel picture fills its area at the frame's shape, up to three
    /// times the frame, in the terminal's cell pixels, centred across the
    /// area; the frame is rendered at its own size and scaled up, or
    /// rendered smaller when the area is smaller.
    #[test]
    fn a_sixel_picture_fills_its_cell_box() {
        let ten_by_twenty = CellSize::DEFAULT;
        let at = |cols, rows, width, cell| {
            let area = picture_area(cols, rows);
            sixel_box(area, frame_size(width, area, cell), cell)
        };
        // 160x50 at 10x20: the 640x256 frame shown at 1600x640, every cell.
        assert_eq!(
            at(160, 50, 640, ten_by_twenty),
            ((640, 256), (1600, 640), (1, 2, 160, 32))
        );
        // 120x40: 1200 by 520 of room, the 640x276 frame at 1200x517.
        assert_eq!(
            at(120, 40, 640, ten_by_twenty),
            ((640, 276), (1200, 517), (1, 2, 120, 26))
        );
        // w11term at 240x48 on a high-DPI screen, 15x28 cells: the 4:1
        // frame capped at three times, centred.
        let hidpi = CellSize {
            width: 15,
            height: 28,
        };
        assert_eq!(
            at(240, 48, 640, hidpi),
            ((640, 160), (1920, 480), (57, 2, 128, 18))
        );
        // A short terminal: the area is wider than 4:1, so the picture is
        // height-bound and centred.
        assert_eq!(
            at(120, 20, 640, ten_by_twenty),
            ((640, 160), (960, 240), (13, 2, 96, 12))
        );
        // Room for less than the frame: rendered at the shown size.
        assert_eq!(
            at(60, 20, 640, ten_by_twenty),
            ((600, 240), (600, 240), (1, 2, 60, 12))
        );
        // `r`'s frame, capped at three times its size.
        assert_eq!(
            at(160, 50, 320, ten_by_twenty),
            ((320, 128), (960, 384), (33, 2, 96, 20))
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
        assert_eq!(wt.sixel_box(), Some((0, 1, 120, 26)));
        assert!(!wt.take_wipe(), "the first picture moves nothing");
        assert!(wt.paint(&both_feeds, 120, 40, t0).is_empty());
        wt.text_touched();
        assert!(!wt.paint(&both_feeds, 120, 40, t0).is_empty(), "resent");
        // `r`: a smaller frame scaled up into the same box, no wipe.
        wt.key(Key::Char('r'), t0);
        assert!(!wt.paint(&both_feeds, 120, 40, t0).is_empty());
        assert_eq!(wt.size(), (320, 138));
        assert_eq!(wt.sixel_box(), Some((0, 1, 120, 26)));
        assert!(!wt.take_wipe());
        // A taller terminal moves the box: under a wipe.
        assert!(!wt.paint(&both_feeds, 160, 50, t0).is_empty());
        assert_eq!(wt.sixel_box(), Some((20, 1, 120, 24)));
        assert!(wt.take_wipe());
        assert!(!wt.take_wipe(), "once");
        wt.key(Key::Esc, t0);
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

    /// The look (`,` and `.`): toggled, 45°/s, capped at a second a tick
    /// so a stalled pump does not spin it, recentred by `c`. It is the
    /// camera's own offset and orders nothing.
    #[test]
    fn the_look_swings_the_camera_and_orders_nothing() {
        let t0 = Instant::now();
        let mut v = supported(t0);
        v.toggle(t0);
        v.key(Key::Char(','), t0);
        assert_eq!(v.look_turn(), -1);
        v.tick(t0 + Duration::from_millis(500));
        assert_eq!(v.look_mdeg(), -22_500, "45°/s for half a second");
        v.key(Key::Char(','), t0);
        assert_eq!(v.look_turn(), 0, "the same key stops the look");
        v.tick(t0 + Duration::from_secs(2));
        assert_eq!(v.look_mdeg(), -22_500, "still");
        v.key(Key::Char('.'), t0);
        v.tick(t0 + Duration::from_secs(3));
        assert_eq!(v.look_mdeg(), 22_500);
        v.tick(t0 + Duration::from_secs(60));
        assert_eq!(v.look_mdeg(), 67_500);
        v.key(Key::Char('c'), t0);
        assert_eq!((v.look_turn(), v.look_mdeg()), (0, 0));
        assert!(v.take_orders().is_empty(), "a look is not an order");
        // Other keys are swallowed without effect.
        v.key(Key::Char('1'), t0);
        assert!(v.is_active());
    }

    /// The turn keys steer the tank (doc 21 cut G.4): a `HEADING` order a
    /// pulse ahead, toggled where the terminal reports no releases; held
    /// (press, repeat, release) where the view pushed the kitty keyboard
    /// protocol, which it pops again on close.
    #[test]
    fn the_arrows_steer_the_tank_toggled_or_held() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let both = feeds(Some(&cockpit), Some(&map));
        // Toggled: no keyboard protocol.
        let mut v = supported(t0);
        v.toggle(t0);
        assert!(v.take_terminal_bytes().is_empty(), "nothing pushed");
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        v.key(Key::Right, t0);
        let first = v.take_orders();
        assert_eq!(first.len(), 1);
        assert!(first[0].starts_with("HEADING "), "{first:?}");
        assert_eq!(v.helm().turning(), 1);
        v.key(Key::Right, t0);
        assert_eq!(v.helm().turning(), 0, "pressed again: stopped");
        v.key(Key::Char('a'), t0);
        assert_eq!(v.helm().turning(), -1);
        v.key(Key::Char(' '), t0);
        assert_eq!(v.helm().turning(), 0, "space stops");
        // Held: the terminal answered `CSI ? u`.
        let mut h = supported(t0);
        h.keyboard_reply();
        h.toggle(t0);
        assert_eq!(h.take_terminal_bytes(), b"\x1b[>3u");
        assert!(!h.paint(&both, 120, 40, t0).is_empty());
        h.key(Key::Left, t0);
        assert_eq!(h.helm().turning(), -1);
        h.key(Key::Left, t0);
        h.key(Key::Repeat(Arrow::Left), t0);
        assert_eq!(h.helm().turning(), -1, "a held key keeps turning");
        h.key(Key::Release(Arrow::Right), t0);
        assert_eq!(h.helm().turning(), -1, "another key's release does nothing");
        h.key(Key::Release(Arrow::Left), t0);
        assert_eq!(h.helm().turning(), 0, "released: stopped");
        let (top, bottom) = h.hud(Some(&cockpit), t0);
        assert!(bottom.contains("turn the tank (hold)"), "{bottom}");
        assert!(top.contains("(tank 045°"), "{top}");
        h.key(Key::Esc, t0);
        assert_eq!(h.take_terminal_bytes(), b"\x1b[<u", "popped on close");
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
        assert!(s.starts_with("\x1b7\x1b[?2026h\x1b[2;1H\x1b_Ga=T,f=100,t=d,i=1,p=1,q=2,C=1,c="));
        assert!(
            s.ends_with("\x1b[?2026l\x1b8"),
            "the cursor is restored where the text frame left it"
        );
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
        // Look, tick: a new picture.
        v.key(Key::Char('.'), t0);
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
        assert!(bottom.contains("turn the tank (again to stop)"), "{bottom}");
    }

    /// A kitty picture is scaled into the largest box of its shape the
    /// area holds, centred across it: the whole area when the frame was
    /// shaped to it.
    #[test]
    fn the_image_box_keeps_the_aspect_and_fits() {
        let cell = CellSize::DEFAULT;
        let at = |cols, rows| {
            let area = picture_area(cols, rows);
            image_box(area, frame_size(640, area, cell), cell)
        };
        assert_eq!(at(160, 50), (1, 2, 160, 32));
        assert_eq!(at(120, 40), (1, 2, 120, 26));
        // Wider than 4:1: height-bound, centred.
        assert_eq!(at(120, 20), (13, 2, 96, 12));
        // Taller than 4:3: width-bound.
        assert_eq!(at(80, 60), (1, 2, 80, 30));
        assert_eq!(at(1, 1), (1, 2, 1, 1));
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
        v.key(Key::Char('r'), t0);
        let small = String::from_utf8_lossy(&v.paint(&both, 120, 40, t0)).into_owned();
        assert_eq!(
            v.size(),
            (320, 138),
            "the next frame drawn is the narrow one, the same shape"
        );
        assert!(small.contains("c=120,r=26"), "{}", &small[..80]);
        assert!(v.stats.image_bytes > 0);
        // G.1b: `u` is a staff diagnostic, inert unless `[pixel]
        // diagnostics` is on, and the legend does not offer it.
        v.key(Key::Char('u'), t0);
        assert!(!v.uncapped(), "no diagnostics: u does nothing");
        let (top, bottom) = v.hud(Some(&cockpit), t0);
        assert!(
            !bottom.contains("u: clock") && !top.contains("render"),
            "{top} / {bottom}"
        );
        v.set_config(PixelConfig {
            diagnostics: true,
            ..PixelConfig::default()
        });
        v.key(Key::Char('u'), t0);
        assert!(v.uncapped());
        assert_eq!(v.interval(), FRAME_INTERVAL_UNCAPPED);
        assert!(
            !v.paint(&both, 120, 40, t0).is_empty(),
            "re-sent after the switch"
        );
        let (top, bottom) = v.hud(Some(&cockpit), t0);
        assert!(
            top.contains("320x138 uncapped") && top.contains("render"),
            "{top}"
        );
        assert!(bottom.contains("640/320 wide") && bottom.contains("u: clock"));
        v.key(Key::Char('r'), t0);
        v.key(Key::Char('u'), t0);
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!((v.size(), v.uncapped()), ((640, 276), false));
    }

    /// **Doc 21 cut G.1b, the step-down** (§3.4): a picture write slower than
    /// a frame skips the next frame; three in a row step the view down to
    /// the narrow frame and the low clock; ten seconds of writes that keep
    /// up step it back; `@pixel/stats` tells it.
    #[test]
    fn slow_writes_skip_a_frame_then_step_down_and_clean_ones_step_back_up() {
        let t0 = Instant::now();
        let (cockpit, map) = fixtures();
        let both = feeds(Some(&cockpit), Some(&map));
        let mut v = supported(t0);
        v.toggle(t0);
        assert_eq!(v.config(), PixelConfig::default());
        let slow = Duration::from_millis(80);
        let quick = Duration::from_millis(2);
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!((v.size().0, v.interval()), (640, FRAME_INTERVAL));

        // One slow write: the next tick is skipped, whatever it would draw.
        assert_eq!(v.write_took(slow, t0), StepChange::Skip);
        v.look_turn = 1;
        v.tick(t0 + Duration::from_millis(500));
        assert!(v.paint(&both, 120, 40, t0).is_empty(), "skipped");
        assert_eq!(v.stats.slow_skips, 1);
        assert!(!v.paint(&both, 120, 40, t0).is_empty(), "then drawn again");
        assert_eq!(v.write_took(quick, t0), StepChange::None, "the streak ends");

        // Three in a row: stepped down.
        assert_eq!(v.write_took(slow, t0), StepChange::Skip);
        assert_eq!(v.write_took(slow, t0), StepChange::Skip);
        assert_eq!(v.write_took(slow, t0), StepChange::Down);
        assert!(v.stepped_down());
        assert_eq!(v.interval(), Duration::from_millis(100));
        assert!(v.paint(&both, 120, 40, t0).is_empty(), "the skip first");
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!(v.size().0, FRAME_W_LO, "the narrow frame");
        let (top, _) = v.hud(Some(&cockpit), t0);
        assert!(
            top.contains("10 Hz") && top.contains("stepped down"),
            "{top}"
        );

        // Writes that keep up for ten seconds step it back up — not nine.
        assert_eq!(v.write_took(quick, t0), StepChange::None);
        assert_eq!(
            v.write_took(quick, t0 + Duration::from_secs(9)),
            StepChange::None
        );
        assert_eq!(
            v.write_took(quick, t0 + Duration::from_secs(10)),
            StepChange::Up
        );
        assert!(!v.stepped_down());
        assert_eq!(v.interval(), FRAME_INTERVAL);
        assert!(!v.paint(&both, 120, 40, t0).is_empty());
        assert_eq!(v.size().0, FRAME_W);

        let stats = v.stats_lines(t0).join(" ");
        assert!(
            stats.contains("skipped behind a slow write 2")
                && stats.contains("stepped down 1 times"),
            "{stats}"
        );
    }

    /// `[pixel]`'s numbers are held to sane bounds.
    #[test]
    fn the_config_is_held_to_sane_bounds() {
        let cfg = PixelConfig::from_millis(1, 5, 0, 0, true);
        assert_eq!(cfg.frame_interval, Duration::from_millis(10));
        assert_eq!(
            cfg.low_interval,
            Duration::from_millis(10),
            "never faster than the clock"
        );
        assert_eq!(
            (cfg.step_down_after, cfg.step_up_after),
            (1, Duration::from_secs(1))
        );
        let cfg = PixelConfig::from_millis(50, 100, 3, 10, false);
        assert_eq!(cfg, PixelConfig::default());
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
