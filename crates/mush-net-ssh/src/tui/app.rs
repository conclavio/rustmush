//! [`TuiApp`] — the per-connection modal TUI state (interface plan I2).
//!
//! Holds the screens, the scrollback ring, the panel store, the WHO rail,
//! the room and its exit keys, and the input line; applies each
//! [`OutputEvent`] to that state, decodes raw input into keys and acts on
//! them by mode, and paints one coalesced frame. It is pure state +
//! rendering: it owns no transport and never blocks, so the output pump can
//! drive it synchronously under the connection lock and write the resulting
//! bytes after the lock drops (invariant T2).
//!
//! **Modes** (D-I-17). The app starts in **nav** mode, where bare keys act:
//! `0`–`9` switch screens, `Tab`/`Shift-Tab` move pane focus, arrows and
//! `PgUp`/`PgDn` scroll the focused pane, `g`/`G` jump, `?` opens help,
//! `Esc` closes an overlay, and every other key is the screen's own binding
//! or — on the scene screen — a one-letter exit (D-I-19). `Enter` enters
//! **input** mode (prefilled from the focused pane's highlighted row, or
//! restoring a kept draft); `"` `:` `;` `\` enter it with the trigger typed;
//! `/` enters it with the palette open. In input mode the box is a normal
//! MUSH line with the D-I-6 editing keys, `Enter` submits and stays,
//! `Enter` on an empty box returns to nav, `Esc` returns to nav keeping the
//! draft, and every nav hotkey is `Alt+key`. `^K` (palette), `^P` (sticky
//! mode) and `^]` (detach) work from either mode. A bare key never sends a
//! mutating command (D-I-18): `Run` bindings are held to the read-only list
//! by [`validate_screens`], and the exit walk is the one carve-out.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use mush_ansi::AnsiTheme;
use mush_proto::{
    ClientCaps, ExitRef, InputMode, OutputEvent, PanelFrameKind, RequestKind, RequestOption,
    RespondChoice, RichText, RoomView, WhoEntry,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use serde_json::Value;

use super::keys::{Key, KeyParser};
use super::panels::{
    ARENA_MAP_FEED, COCKPIT_FEED, ChanMsg, NOTICE_SENTENCE_TAG, PanelOutcome, PanelStore, UNIT_FEED,
};
use super::render::{
    Entry, GUTTER_W, PaneRows, RenderCtx, dim_span_owned, legend_line, render_backlog_rows,
    render_channels_rows, render_event, render_rolls_rows, render_sheet_rows, render_vitals_rows,
    render_who_rail, status_line, tab_strip, vitals_line, wrap_spans,
};
use super::screen::{
    BorderRole, COCKPIT_SCREEN, GLOBAL_KEYS, JUMP_KEYS, KeyAction, LocalAction, PaneContent,
    PaneId, PaneSlot, PaneSpec, Screen, ScreenId, builtin_screens, validate_screens,
};
use crate::line::LineBuffer;

/// Scrollback ring depth (entries, pre-wrap).
pub const SCROLLBACK_CAP: usize = 1000;
/// Input history depth (matches the web client).
pub const TUI_HISTORY: usize = 200;
/// Seconds an NPC "thinking" pill lives without an end frame.
pub const THINKING_TTL_S: i64 = 20;
/// Seconds a "composing" pill lives without an `active: false`.
pub const COMPOSING_TTL_S: i64 = 8;
/// Below this many rows the chrome (tab strip, status, gauges, legend) is
/// dropped and the terminal shows scrollback and input only.
pub const MIN_CHROME_ROWS: u16 = 12;
/// Below this many columns the WHO rail is dropped.
pub const WHO_RAIL_MIN_W: u16 = 100;
/// Input-line block height (border + one text row).
const INPUT_H: u16 = 3;
/// Rows a page key moves.
const PAGE: usize = 10;
/// The most palette rows shown.
const MAX_PALETTE_ROWS: usize = 8;
/// Backlog rows requested per `b`.
const BACKLOG_LIMIT: u32 = 50;
/// Narrative-history rows asked for per page (interface plan I12). The
/// gateway clamps `limit` to 1..=200 and cuts a page at 256 KB; an absent or
/// zero `limit` yields a ONE-line page, so it is always sent explicitly.
const NARRATIVE_PAGE: u32 = 100;
/// Seconds after which a narrative-history page in flight is assumed lost
/// and the request re-arms. It has to *age* rather than latch: the gateway
/// drops a throttled `narrative.backlog` query in silence — no reply, no
/// error — so a plain in-flight flag would wedge history for the whole
/// connection (the `RESYNC_LATCH_S` precedent in `panels.rs`).
const BACKLOG_LATCH_S: i64 = 5;
/// The most open server-initiated asks held at once (interface plan I10) —
/// `mush_core::request::MAX_OPEN_REQUESTS_PER_SUBJECT`. Past it the newest
/// is dropped, as the engine refuses to open one.
const MAX_OPEN_REQUESTS_PER_SUBJECT: usize = 32;
/// **Seconds between two asks for the arena's terrain** (16b cut Q.4).
///
/// `arena.map` is PULLED and never pushed (16b cut Q.3), it is never in the
/// session's resume ring, and a reconnected or newly-seated pilot therefore
/// holds none. The cockpit frame says which match the ground belongs to, so
/// holding a frame of match M with no map of M is exactly the condition that
/// asks — and it asks again on the same terms, so a dropped or throttled
/// answer heals on the next pulse instead of leaving a dark map for the
/// session. Two seconds because a frame arrives at most once every five seconds
/// at the quiet tempo, once a second at alert and twice a second at engaged
/// (`[tacsim.arena.tempo]`), and only when it changed; without the latch a slow
/// link would be asked for the whole ground twice a second while a fight is on.
/// The timer heal (16b cut B.2) asks on the pump's four-second tick under the
/// same latch, so a stationary pilot whose answer was dropped is not waiting
/// for the first thing to move.
pub const MAP_HEAL_LATCH_S: i64 = 2;

/// Nav or input (D-I-17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Bare keys act.
    Nav,
    /// The box is a MUSH line; nav keys are Alt+key.
    Input,
}

/// The sticky mode `^P` cycles (D-I-8): what an unprefixed line becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickyMode {
    /// Lines go as typed.
    #[default]
    None,
    /// `say <line>`.
    Say,
    /// `pose <line>`.
    Pose,
    /// `@emit <line>`.
    Emit,
    /// `ooc <line>`.
    Ooc,
}

impl StickyMode {
    fn next(self) -> StickyMode {
        match self {
            StickyMode::None => StickyMode::Say,
            StickyMode::Say => StickyMode::Pose,
            StickyMode::Pose => StickyMode::Emit,
            StickyMode::Emit => StickyMode::Ooc,
            StickyMode::Ooc => StickyMode::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            StickyMode::None => "",
            StickyMode::Say => "say",
            StickyMode::Pose => "pose",
            StickyMode::Emit => "emit",
            StickyMode::Ooc => "ooc",
        }
    }
}

/// What the server told this connection before the TUI opened: the panel
/// manifest from `Hello`, the character from `SessionInfo`, and the last room
/// it was shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuiManifest {
    /// Panel feed ids the server can publish (`Hello.panels`).
    pub panels: Vec<String>,
    /// The attached character's name.
    pub character: Option<String>,
    /// **The last room this connection was shown** (16b cut B.3, `B-66`): a
    /// bare `Room` event or one answering a query (`Reply { Room }`), recorded
    /// by the output pump BEFORE its line/TUI branch, so a walk or a `BOARD`
    /// taken inside the TUI is remembered for the next `@tui` too. A TUI opens
    /// on it ([`TuiApp::set_manifest`]) instead of on `(no room yet)`.
    pub room: Option<RoomView>,
}

/// What a decoded input burst asks the transport to do. Every burst that
/// changed visible state also asks for a repaint.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InputResult {
    /// Repaint the frame.
    pub redraw: bool,
    /// Completed command lines to forward to `hooks.line`, in order. A single
    /// burst can carry more than one (a multi-line paste), so this
    /// accumulates rather than collapsing to the last.
    pub submit: Vec<String>,
    /// Leave the TUI (the detach hotkey or `/notui`).
    pub detach: bool,
    /// Read-only queries the transport should submit via `hooks.query`,
    /// `(name, args)`. Read-only, so they never stamp idle.
    pub queries: Vec<(String, Value)>,
    /// Answers to server-initiated requests the transport should submit via
    /// `hooks.respond`, `(id, choice)` (interface plan I10, D-I-21). Advisory,
    /// never idle stamps.
    pub responds: Vec<(u64, String, RespondChoice)>,
}

/// A server-initiated request being shown as an overlay (interface plan I10).
#[derive(Clone, Debug, PartialEq)]
struct TuiRequest {
    epoch: String,
    id: u64,
    kind: RequestKind,
    prompt: RichText,
    options: Vec<RequestOption>,
    /// When the engine stops accepting an answer, Unix MILLISECONDS. It
    /// expires the ask server-side and tells the subject nothing, so this is
    /// the only thing a queue has to keep a dead ask from blocking the live
    /// ones behind it.
    deadline_ms: Option<u64>,
}

/// One command from the `commands.list` keyframe.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandEntry {
    name: String,
    usage: String,
    summary: String,
}

/// One palette match.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PaletteEntry {
    /// What picking it puts in the box.
    insert: String,
    /// The left column.
    label: String,
    /// The right column.
    detail: String,
}

#[derive(Clone, Debug, Default)]
struct Palette {
    entries: Vec<PaletteEntry>,
    selected: usize,
}

#[derive(Clone, Debug, Default)]
struct PaneUi {
    /// Rows scrolled from the top (text panes).
    scroll: usize,
    /// The highlighted selectable row's index into the pane's rows.
    selected: usize,
    /// **The `row_key` of the row the player picked** (16b cut Q.4), which is
    /// what the index is re-derived from on every render — see
    /// [`TuiApp::anchored`].
    selected_key: Option<String>,
}

/// A client-local command (`/clear`, `/help`, `/history`, `/notui`) the
/// palette lists.
const LOCAL_COMMANDS: [(&str, &str); 4] = [
    ("/clear", "empty the scrollback"),
    ("/help", "the key help"),
    ("/history", "load older scene history"),
    ("/notui", "return to line mode"),
];

/// Per-connection ratatui application state.
pub struct TuiApp {
    caps: ClientCaps,
    theme: AnsiTheme,
    clock: fn() -> i64,
    screens: Vec<Screen>,
    active: ScreenId,
    /// Focused pane index into the active screen's `panes`.
    focus: usize,
    pane_ui: HashMap<(ScreenId, PaneId), PaneUi>,
    mode: Mode,
    line: LineBuffer,
    draft: Option<String>,
    sticky: StickyMode,
    input_mode: InputMode,
    prompt: String,
    parser: KeyParser,
    scrollback: VecDeque<Entry>,
    /// Rows scrolled up from the bottom (0 = following the tail).
    scroll_off: usize,
    /// Front entries `/clear` hid. It is a VIEW floor, not a data clear: the
    /// ring is kept so [`TuiApp::backlog_cursor`] can stay where it is. The
    /// old clear re-armed the cursor, and the next page then came back with
    /// every line that had arrived live since — painted a second time,
    /// because an SSH scrollback entry carries no seq to dedup against.
    cleared: usize,
    /// `before_seq` for the next narrative-history page; `None` asks from
    /// the newest (interface plan I12). It only ever moves DOWN, and that is
    /// the whole dedup: an SSH connection is a `Sink::Ansi` and never sees an
    /// `Envelope`, so a live scrollback entry carries no seq to compare a
    /// page against.
    backlog_cursor: Option<u64>,
    /// The server may hold older narrative below what has been prepended.
    backlog_more: bool,
    /// A page is in flight, and when it was asked for — an aging latch, not
    /// a flag (see [`BACKLOG_LATCH_S`]).
    backlog_pending: Option<i64>,
    /// Whether the "that is all there is" marker has been prepended already.
    backlog_floored: bool,
    /// Rows the last paint could scroll: the scroll-to-top trigger.
    last_max_scroll: usize,
    who: Vec<WhoEntry>,
    room: Option<RoomView>,
    exit_keys: Vec<(char, String)>,
    panels: PanelStore,
    manifest: TuiManifest,
    mail_unread: usize,
    /// Notices (I11) that arrived while the scene tail was not in view; the
    /// status line shows the count. Cleared when the tail is on screen again.
    notice_unseen: usize,
    commands: Vec<CommandEntry>,
    channel_aliases: Vec<String>,
    palette: Option<Palette>,
    help: bool,
    /// Server-initiated asks still open, OLDEST FIRST (I10). A queue, not a
    /// slot: the engine's `respond` answers the lowest open id and the web
    /// store answers `pending[0]`, so an overlay showing the newest would
    /// measure the player's answer against an ask they cannot see.
    requests: VecDeque<TuiRequest>,
    /// The request id whose free-text answer the input box is capturing (I10).
    answering: Option<(u64, String)>,
    thinking: Vec<(String, i64)>,
    composing: Vec<(String, i64)>,
    /// When the pulled `arena.map` was last asked for, Unix seconds (16b cut
    /// Q.4). An aging latch, not a flag — see [`MAP_HEAL_LATCH_S`].
    map_asked_at: Option<i64>,
    /// The match the last such ask was for (16b cut Q.4). The latch re-arms
    /// on a match CHANGE and on nothing else: an answer that keeps coming back
    /// wrong — the unseated one, or ground of a match that has ended — must
    /// not re-arm it, or the ask would go out once per answer instead of once
    /// per [`MAP_HEAL_LATCH_S`], which is the storm the latch exists to stop.
    map_asked_for: Option<i64>,
    /// The match id the cockpit screen has already been switched to for (16b
    /// cut Q.4). It is set once per match, so a pilot who walks away from the
    /// screen is never dragged back to it by the next pulse.
    cockpit_switched: Option<i64>,
    /// **Whether this viewer holds a seat** (16b cut Q.4; 16b cut B.3 made it
    /// end with the seat): set by a match's first seated frame, cleared by the
    /// cleared one — the gate on the tenth tab, described on
    /// [`Self::screen_available`].
    cockpit_seat: bool,
}

fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

impl TuiApp {
    /// A fresh app sized for `caps`, with the built-in screens.
    ///
    /// # Panics
    ///
    /// Never for the built-ins: their table passes [`validate_screens`], and
    /// a test pins it.
    #[must_use]
    pub fn new(caps: ClientCaps) -> TuiApp {
        TuiApp::with_screens(caps, builtin_screens()).expect("built-in screens are valid")
    }

    /// An app with a custom screen table (a ruleset registration).
    ///
    /// # Errors
    ///
    /// The first violation [`validate_screens`] finds.
    pub fn with_screens(caps: ClientCaps, screens: Vec<Screen>) -> Result<TuiApp, String> {
        validate_screens(&screens)?;
        let active = screens.first().map_or(ScreenId(1), |s| s.id);
        Ok(TuiApp {
            caps,
            theme: AnsiTheme::default(),
            clock: system_clock,
            screens,
            active,
            focus: 0,
            pane_ui: HashMap::new(),
            mode: Mode::Nav,
            line: LineBuffer::new(TUI_HISTORY),
            draft: None,
            sticky: StickyMode::None,
            input_mode: InputMode::Normal,
            prompt: String::new(),
            parser: KeyParser::new(),
            scrollback: VecDeque::new(),
            scroll_off: 0,
            cleared: 0,
            backlog_cursor: None,
            backlog_more: true,
            backlog_pending: None,
            backlog_floored: false,
            last_max_scroll: 0,
            who: Vec::new(),
            room: None,
            exit_keys: Vec::new(),
            panels: PanelStore::new(),
            manifest: TuiManifest::default(),
            mail_unread: 0,
            notice_unseen: 0,
            commands: Vec::new(),
            channel_aliases: Vec::new(),
            palette: None,
            help: false,
            requests: VecDeque::new(),
            answering: None,
            thinking: Vec::new(),
            composing: Vec::new(),
            map_asked_at: None,
            map_asked_for: None,
            cockpit_switched: None,
            cockpit_seat: false,
        })
    }

    /// Use a fixed clock (tests, golden frames).
    pub fn set_clock(&mut self, clock: fn() -> i64) {
        self.clock = clock;
    }

    /// Record what the server announced before the TUI opened.
    ///
    /// **16b cut B.3** (`B-66`): the manifest's last room seeds the app's room
    /// AND its exit keys — through the same [`exit_keys`] the `Room` arm of
    /// [`Self::apply_event`] uses — when the app holds no room yet, so `@tui`
    /// opens on the status line of the room the player is standing in and the
    /// scene screen's one-letter exits work before anything else arrives. A
    /// room the app already holds is newer than the manifest's and is kept.
    pub fn set_manifest(&mut self, manifest: TuiManifest) {
        if self.room.is_none()
            && let Some(view) = manifest.room.as_ref()
        {
            self.room = Some(view.clone());
            self.exit_keys = exit_keys(&view.exits);
        }
        self.manifest = manifest;
        self.leave_a_screen_this_server_does_not_have();
    }

    /// **Whether a screen exists for THIS viewer** (16b cut Q.4).
    ///
    /// A screen with no [`Screen::gate`] always does. A gated one needs two
    /// things: the server publishes the feed — an EMPTY manifest is a hello
    /// that has not landed, and over SSH one never does, so it counts as
    /// published — and, for the cockpit, the one gate there is, a frame has
    /// said this viewer is SEATED.
    ///
    /// The second half is the observer rule on the tab strip (doc 16 §1). Both
    /// the push and the query answer `seated: false` to anyone who is not in a
    /// cockpit, so a bystander in `@tui` on an arena server has no cockpit tab
    /// at all, rather than one that would tell them a match is being fought.
    /// The flag ([`Self::cockpit_seat`]) is held from a match's first seated
    /// frame to the cleared one (16c Q21, 16b cut B.3): the tab goes with the
    /// seat, and a pilot standing on it is moved to the first screen — the web
    /// client's tab keeps the same hours.
    fn screen_available(&self, screen: &Screen) -> bool {
        match screen.gate {
            None => true,
            Some(feed) => {
                (self.manifest.panels.is_empty() || manifest_has(&self.manifest, feed))
                    && (feed != COCKPIT_FEED || self.cockpit_seat)
            }
        }
    }

    /// The screens this server has, in table order.
    fn available_screens(&self) -> impl Iterator<Item = &Screen> {
        self.screens.iter().filter(|s| self.screen_available(s))
    }

    /// A hello that takes a gated screen away — or, since 16b cut B.3, the
    /// cleared frame that takes the cockpit's seat — leaves the player standing
    /// on a tab that is no longer drawn: put them back on the first screen,
    /// which is never gated.
    fn leave_a_screen_this_server_does_not_have(&mut self) {
        let gone = self
            .screens
            .iter()
            .find(|s| s.id == self.active)
            .is_some_and(|s| !self.screen_available(s));
        if gone && let Some(first) = self.screens.first().map(|s| s.id) {
            self.active = first;
            self.focus = 0;
        }
    }

    /// Update capabilities (color depth / geometry) on resize.
    pub fn set_caps(&mut self, caps: ClientCaps) {
        self.caps = caps;
    }

    /// The current capabilities, for the backend geometry sync.
    #[must_use]
    pub fn caps(&self) -> ClientCaps {
        self.caps
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The sticky mode.
    #[must_use]
    pub fn sticky(&self) -> StickyMode {
        self.sticky
    }

    /// The active screen's digit.
    #[must_use]
    pub fn active_screen(&self) -> ScreenId {
        self.active
    }

    /// The focused pane's id on the active screen.
    #[must_use]
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.screen().panes.get(self.focus).map(|p| p.id)
    }

    /// The input line as typed.
    #[must_use]
    pub fn input_text(&self) -> String {
        self.line.text()
    }

    /// The cursor within the input line.
    #[must_use]
    pub fn input_cursor(&self) -> usize {
        self.line.cursor()
    }

    /// The draft kept by `Esc`, if any.
    #[must_use]
    pub fn draft(&self) -> Option<&str> {
        self.draft.as_deref()
    }

    /// Whether the palette overlay is open.
    #[must_use]
    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    /// The palette's entries as `insert` strings, highlighted first.
    #[must_use]
    pub fn palette_entries(&self) -> Vec<String> {
        self.palette
            .as_ref()
            .map(|p| p.entries.iter().map(|e| e.insert.clone()).collect())
            .unwrap_or_default()
    }

    /// Whether the help overlay is open.
    #[must_use]
    pub fn help_open(&self) -> bool {
        self.help
    }

    /// Entries held in the scrollback ring, oldest first — including any a
    /// `/clear` floor is hiding (it hides a prefix, it does not drop it).
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Rows scrolled up from the scrollback's tail.
    #[must_use]
    pub fn scroll_off(&self) -> usize {
        self.scroll_off
    }

    /// The one-letter exit keys in force (scene screen).
    #[must_use]
    pub fn exit_keys(&self) -> &[(char, String)] {
        &self.exit_keys
    }

    /// The panel store.
    #[must_use]
    pub fn panels(&self) -> &PanelStore {
        &self.panels
    }

    /// The prompt text the server last set.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// The app's clock, Unix seconds (a fixed one in tests and goldens).
    /// `pub(super)` so [`super::TuiSession::heal_queries`] asks on the same
    /// clock the frame path's latch was armed on.
    pub(super) fn now(&self) -> i64 {
        (self.clock)()
    }

    fn screen(&self) -> &Screen {
        self.screens
            .iter()
            .find(|s| s.id == self.active)
            .or_else(|| self.screens.first())
            .expect("at least one screen")
    }

    fn pane_ui_mut(&mut self, pane: PaneId) -> &mut PaneUi {
        let key = (self.active, pane);
        self.pane_ui.entry(key).or_default()
    }

    fn pane_ui(&self, pane: PaneId) -> PaneUi {
        self.pane_ui
            .get(&(self.active, pane))
            .cloned()
            .unwrap_or_default()
    }

    /// **The index the highlight actually rests on** (16b cut Q.4).
    ///
    /// [`PaneUi::selected`] is an index into rows that are re-rendered from
    /// whatever the feed holds *now*. On a list that changes on a human
    /// timescale an index is fine; the cockpit's contacts arrive as fast as
    /// the arena pulses and grow a scan row under any contact that gains a
    /// close look (16b cut Q.2), so a row inserted above the highlight would
    /// slide it onto a neighbour — and `Enter` would open the box with another
    /// contact's `lock`. The row's KEY is what the player picked, so the index
    /// follows the key wherever the row moved to. A key that is gone (the
    /// contact was lost) leaves the index where it was, and `selected_row`'s
    /// own fallback takes it from there.
    fn anchored(&self, pane: PaneId, rows: &PaneRows) -> usize {
        let ui = self.pane_ui(pane);
        let Some(key) = ui.selected_key.as_deref() else {
            return ui.selected;
        };
        if rows
            .rows
            .get(ui.selected)
            .and_then(|r| r.row_key.as_deref())
            == Some(key)
        {
            return ui.selected;
        }
        rows.rows
            .iter()
            .position(|r| r.selectable && r.row_key.as_deref() == Some(key))
            .unwrap_or(ui.selected)
    }

    /// Move a pane's highlight to `index`, remembering the row's key so the
    /// next render finds it again — see [`Self::anchored`].
    fn remember_selection(&mut self, pane: PaneId, index: usize, rows: &PaneRows) {
        let key = rows.rows.get(index).and_then(|r| r.row_key.clone());
        let ui = self.pane_ui_mut(pane);
        ui.selected = index;
        ui.selected_key = key;
    }

    // ---- events -----------------------------------------------------------

    /// The queries to submit right after entering the TUI.
    ///
    /// Takes `&mut self` because one of them — the first narrative-history
    /// page — arms the backlog latch (interface plan I12); everything else
    /// here is a read of state.
    #[must_use]
    pub fn entry_queries(&mut self) -> Vec<(String, Value)> {
        let empty = || Value::Object(serde_json::Map::new());
        // Interface plan I12: prime the first page of scene history — the
        // player value of the whole feature (reconnect into a scene and read
        // the last few minutes without pressing anything).
        //
        // It goes FIRST, and `enter_tui` awaits these in order. The
        // scrollback is empty when the `TuiApp` is BUILT, not when the page
        // lands: the gateway answers `narrative.backlog` synchronously from
        // the session ring, but every query ahead of it is another awaited
        // bridge send, and a narrative line delivered in that window is
        // push_back'd live AND carried in the page (an SSH entry has no seq
        // to dedup against). Asking first narrows the window to one hook
        // call; it does not close it — a line already queued at the mode flip
        // is still painted twice, and that residue is recorded, not fixed.
        self.backlog_pending = Some(self.now());
        let mut q: Vec<(String, Value)> = vec![
            (
                "narrative.backlog".to_string(),
                serde_json::json!({ "before_seq": self.backlog_cursor, "limit": NARRATIVE_PAGE }),
            ),
            ("who".to_string(), empty()),
            ("commands.list".to_string(), empty()),
        ];
        // `comsys` and `scenes` are modules a world may simply not compose,
        // and their entry queries were unconditional (look-back LB-11) — two
        // "Unknown query" replies into the scene scrollback on every `@tui`
        // against such a server. `who` and `commands.list` are core and are
        // never gated. An empty manifest is a hello that has not landed, not
        // an absent feed, so it still asks.
        for feed in ["comsys", "scenes"] {
            if self.manifest.panels.is_empty() || manifest_has(&self.manifest, feed) {
                q.push((feed_query(feed).to_string(), empty()));
            }
        }
        for feed in &self.manifest.panels {
            if feed.ends_with(".vitals") {
                q.push((feed.clone(), empty()));
            }
        }
        // **16b cut Q.4**: the cockpit feed is asked for once on entry, where
        // the server publishes it. The arena PUSHES a frame only when one
        // changes (`Frame::same_as`), so a pilot sitting still in a powered
        // tank would otherwise wait for the first thing to move before their
        // screen existed at all. Anyone who is not in a cockpit is answered
        // `seated: false`, which is what keeps the tenth tab off a bystander's
        // strip. An empty manifest is a hello that has not landed — and over
        // SSH one never does — so it still asks, exactly as `comsys` and
        // `scenes` above do.
        if self.manifest.panels.is_empty() || manifest_has(&self.manifest, COCKPIT_FEED) {
            q.push((COCKPIT_FEED.to_string(), empty()));
        }
        q.extend(self.screen_queries());
        q
    }

    /// The feed a screen's declaration actually names on THIS server. Screens
    /// 8 and 9 are written against [`UNIT_FEED`], the name the tacsim pack
    /// happens to use; a world that publishes `<pack>.unit` under any other
    /// prefix drives the same two screens (look-back LB-12, mirroring the
    /// web's `endsWith(".unit")` routing). Every other feed is itself.
    fn resolve_feed<'a>(&'a self, feed: &'a str) -> &'a str {
        if feed == UNIT_FEED {
            PanelStore::unit_feed(&self.manifest)
        } else {
            feed
        }
    }

    /// The active screen's own hydration queries.
    fn screen_queries(&self) -> Vec<(String, Value)> {
        let screen = self.screen();
        // An `on_enter` that hydrates one of the screen's own PANEL feeds is
        // gated on the manifest exactly as the pane's subscribe is (below):
        // asking a server that does not publish the feed earns an `Unknown
        // query` reply in a scrollback neither tactical screen shows. An
        // EMPTY manifest means the hello has not landed yet, not that the
        // feed is absent, so it still asks. A semantic VIEW (`legal.actions`)
        // is not a panel feed and `Hello.panels` never lists one, so it is
        // never gated.
        //
        // The gate matches the query against each pane's FEED THROUGH
        // `feed_query` (look-back LB-11): `mail.list`, `intel.list` and
        // `comsys.state` are named differently from the `mail`/`intel`/
        // `comsys` feeds they hydrate, so comparing the raw names matched
        // nothing and screens 3/4/6 asked a mail-less, comsys-less server
        // unconditionally.
        //
        // Both sides run through `resolve_feed`, so a screen declaring
        // `tactics.unit` gates and subscribes on the `.unit` feed THIS world
        // actually publishes (look-back LB-12).
        let gated_feed = |name: &str| {
            screen.panes.iter().find_map(|p| {
                pane_feed(p.content)
                    .map(|f| self.resolve_feed(f))
                    .filter(|f| feed_query(f) == name)
            })
        };
        let mut q: Vec<(String, Value)> = screen
            .on_enter
            .iter()
            .map(|(n, a)| (self.resolve_feed(n).to_string(), a.clone()))
            .filter(|(n, _)| match gated_feed(n) {
                Some(feed) => self.manifest.panels.is_empty() || manifest_has(&self.manifest, feed),
                None => true,
            })
            // **16b cut Q.4**: the arena's terrain is the one `on_enter` ask
            // that is not cheap — a whole surface, priced at up to 256 KiB —
            // and the ground does not move. A map of the match the held frame
            // names is already the right one, so entering the cockpit again
            // (or switching back to it) never re-pulls it; when none is held,
            // this IS the self-heal's ask and `follow_the_cockpit` arms the
            // latch on it.
            .filter(|(n, _)| {
                n != ARENA_MAP_FEED
                    || !self
                        .cockpit_match()
                        .is_some_and(|m| self.holds_the_ground(m))
            })
            .collect();
        // Interface plan I8: a pane over a module feed (a dotted name) the
        // manifest carries is subscribed on entry — the contacts table — and
        // an absent feed is never asked for, so a server without its producer
        // prints nothing. A feed `on_enter` already names is not doubled.
        for p in &screen.panes {
            if let PaneContent::Panel { feed, .. } = p.content {
                let feed = self.resolve_feed(feed);
                if feed.contains('.')
                    && manifest_has(&self.manifest, feed)
                    && !q.iter().any(|(n, _)| n == feed)
                {
                    q.push((feed.to_string(), Value::Object(serde_json::Map::new())));
                }
            }
        }
        if screen
            .panes
            .iter()
            .any(|p| matches!(p.content, PaneContent::Sheet))
        {
            for feed in &self.manifest.panels {
                if feed.ends_with(".sheet") {
                    q.push((feed.clone(), Value::Object(serde_json::Map::new())));
                }
            }
        }
        if screen
            .panes
            .iter()
            .any(|p| matches!(p.content, PaneContent::Vitals))
        {
            for feed in &self.manifest.panels {
                if feed.ends_with(".vitals") {
                    q.push((feed.clone(), Value::Object(serde_json::Map::new())));
                }
            }
        }
        q
    }

    /// **What one `arena.cockpit` keyframe asks of the client** (16b cut Q.4;
    /// doc 16 §5): the terrain it is drawn on, and — once per match — the
    /// screen it is drawn on.
    ///
    /// * **The map is pulled.** 16b cut Q.3 registered `arena.map` with its
    ///   feed handle dropped, so nothing ever pushes it and it is never in the
    ///   session's resume ring. A frame of match M with no held map of M is
    ///   the condition that asks, throttled to one ask per
    ///   [`MAP_HEAL_LATCH_S`]; a map of another match is dropped first,
    ///   because stale ground under live markers is worse than none. Only a
    ///   CHANGE of match re-arms that latch ([`Self::map_asked_for`]): an
    ///   answer that keeps coming back wrong would otherwise buy itself a
    ///   fresh ask on every pulse, which is the one thing the latch is for.
    /// * **The screen opens itself**, once per match id. A pilot who then
    ///   presses `1` and stays on the scene is left alone: the next pulse does
    ///   not drag them back. The screen is opened FIRST, because its `on_enter`
    ///   carries an `arena.map` ask of its own and one terrain is enough: the
    ///   heal below takes that ask as its own and arms the latch on it.
    /// * **The notices ring** (16b cut B.3, `B-64`) takes every seated frame's
    ///   notices before anything else here, and a frame of a new match empties
    ///   it first (`PanelStore::push_cockpit_notices`).
    /// * **A cleared frame** (`seated: false`, the last one a pilot gets)
    ///   drops the map, the ring and the tab, and re-arms both latches, so the
    ///   next match starts clean.
    fn follow_the_cockpit(&mut self, frame: &Value, now: i64) -> Vec<(String, Value)> {
        let mut queries = Vec::new();
        let seated = frame.get("seated").and_then(Value::as_bool) != Some(false);
        let Some(match_id) = frame
            .get("match")
            .and_then(Value::as_i64)
            .filter(|_| seated)
        else {
            // The seat is gone, and the tab goes with it (16b cut B.3, `B-63`,
            // 16c Q21): the flag clears, and a pilot standing on screen 0 is
            // moved to the first screen — the scene, where the sentence that
            // said why (`You climb out…`) has landed. Nothing of the seat
            // stays: the ground, the ring and both latches go too.
            self.panels.forget(ARENA_MAP_FEED);
            self.panels.clear_cockpit_notices();
            self.map_asked_at = None;
            self.map_asked_for = None;
            self.cockpit_switched = None;
            self.cockpit_seat = false;
            self.leave_a_screen_this_server_does_not_have();
            return queries;
        };
        // The ring first (16b cut B.3): the frame's own notices, composed from
        // their fields, and a new match's frame empties it before they land.
        self.panels.push_cockpit_notices(frame);
        self.cockpit_seat = true;
        let holds_the_ground = self.holds_the_ground(match_id);
        if !holds_the_ground && self.panels.get(ARENA_MAP_FEED).is_some() {
            // Another match's terrain, or the unseated answer: drop it, because
            // stale ground under live markers is worse than none. The latch is
            // NOT touched here — a wrong answer is the case it guards.
            self.panels.forget(ARENA_MAP_FEED);
        }
        if self.cockpit_switched != Some(match_id) {
            self.cockpit_switched = Some(match_id);
            queries.extend(self.open_the_cockpit_screen());
        }
        if !holds_the_ground {
            let due = self.map_ask_due(match_id, now);
            // The screen that just opened asks for the terrain itself. That
            // ask IS this one: arm the latch on it rather than sending a second.
            let already = queries.iter().any(|(n, _)| n == ARENA_MAP_FEED);
            if due || already {
                self.map_asked_at = Some(now);
                self.map_asked_for = Some(match_id);
                if !already {
                    queries.push((
                        ARENA_MAP_FEED.to_string(),
                        Value::Object(serde_json::Map::new()),
                    ));
                }
            }
        }
        queries
    }

    /// **Whether an `arena.map` ask for `match_id` is due at `now`** (16b cut
    /// Q.4; factored out in cut B.2 so the frame and the timer ask on one
    /// rule). A new match asks at once rather than waiting out a latch armed
    /// for the old one; otherwise the latch AGES, so an answer that never came
    /// (a throttled query is dropped in silence by the bridge) costs one stale
    /// window, not the session. It reads the latch and arms nothing: the
    /// caller that sends the ask arms it.
    fn map_ask_due(&self, match_id: i64, now: i64) -> bool {
        self.map_asked_for != Some(match_id)
            || self
                .map_asked_at
                .is_none_or(|at| !(0..MAP_HEAL_LATCH_S).contains(&now.saturating_sub(at)))
    }

    /// **The timer heal** (16b cut B.2, `B-59`): the `arena.map` ask a held
    /// seated frame is owed when no frame has arrived to ask it.
    ///
    /// The arena pushes a frame only when one CHANGES (`Frame::same_as`), and
    /// `Self::follow_the_cockpit` asks only when a frame arrives — so a
    /// pilot sitting still with no contacts whose map answer was dropped used
    /// to keep `Waiting for the arena map...` for the whole match. The pump
    /// calls this on its four-second tick: while the held frame is seated, the
    /// ground of its match is not held and `Self::map_ask_due` says the
    /// latch has aged, it arms the latch and answers the one ask; otherwise
    /// nothing. The latch is what bounds the rate — the same one the frame
    /// path arms — so the two paths together never ask more than once per
    /// [`MAP_HEAL_LATCH_S`], and a held map is never asked for (the ground
    /// does not move, and a map costs up to 256 KiB and a flood token).
    #[must_use]
    pub fn heal_queries(&mut self, now: i64) -> Vec<(String, Value)> {
        let Some(match_id) = self.cockpit_match() else {
            return Vec::new();
        };
        if self.holds_the_ground(match_id) || !self.map_ask_due(match_id, now) {
            return Vec::new();
        }
        self.map_asked_at = Some(now);
        self.map_asked_for = Some(match_id);
        vec![(
            ARENA_MAP_FEED.to_string(),
            Value::Object(serde_json::Map::new()),
        )]
    }

    /// Arm the map latch when `queries` carries an `arena.map` ask for the held
    /// seated frame's match (16b cut B.2): an ask the screen's own hydration
    /// sent — the `p` key, a switch onto screen 0, a hello — is an ask like
    /// any other, and the frame path and the timer must count it.
    fn note_map_ask(&mut self, queries: &[(String, Value)], now: i64) {
        if let Some(match_id) = self.cockpit_match()
            && queries.iter().any(|(n, _)| n == ARENA_MAP_FEED)
        {
            self.map_asked_at = Some(now);
            self.map_asked_for = Some(match_id);
        }
    }

    /// **Whether the store holds the terrain of `match_id`** (16b cut Q.4).
    ///
    /// One pulled `arena.map` serves every redraw of a match, so this is the
    /// condition both the self-heal above and the screen's own `on_enter` ask
    /// are gated on: a map that is already held is never asked for twice.
    fn holds_the_ground(&self, match_id: i64) -> bool {
        self.panels
            .get(ARENA_MAP_FEED)
            .and_then(|map| map.get("match"))
            .and_then(Value::as_i64)
            == Some(match_id)
    }

    /// The match the held `arena.cockpit` frame belongs to, if there is one
    /// and it is a seated frame (16b cut Q.4).
    fn cockpit_match(&self) -> Option<i64> {
        let frame = self.panels.get(COCKPIT_FEED)?;
        if frame.get("seated").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        frame.get("match").and_then(Value::as_i64)
    }

    /// Switch to the cockpit screen, answering its hydration queries. Nothing
    /// when it is already showing, and nothing on a server whose manifest does
    /// not carry the feed — that screen has no tab either.
    fn open_the_cockpit_screen(&mut self) -> Vec<(String, Value)> {
        if self.active == COCKPIT_SCREEN
            || !self.available_screens().any(|s| s.id == COCKPIT_SCREEN)
        {
            return Vec::new();
        }
        self.active = COCKPIT_SCREEN;
        self.focus = 0;
        self.help = false;
        self.screen_queries()
    }

    /// Apply one event to the app state. Returns read-only queries the app
    /// wants submitted (a resync after an unmergeable delta, a hydration
    /// after a `Hello`).
    pub fn apply_event(&mut self, event: &OutputEvent) -> Vec<(String, Value)> {
        let now = self.now();
        let mut queries = Vec::new();
        // Whether this event answers one of the app's OWN queries (16b cut
        // B.3): the `Sys` arm rings only what the pilot was sent, never a
        // refusal or a throttle line answering a background ask.
        let answered = matches!(event, OutputEvent::Reply { .. });
        let event = match event {
            OutputEvent::Reply { event, .. } => event.as_ref(),
            other => other,
        };
        match event {
            OutputEvent::Who(entries) => {
                self.who.clone_from(entries);
                return queries;
            }
            OutputEvent::Prompt(spec) => {
                self.input_mode = spec.mode;
                self.prompt = spec
                    .text
                    .as_ref()
                    .map(|t| sanitize(&t.to_plain_text()))
                    .unwrap_or_default();
                // A masked or hidden prompt (a password, chargen) wants
                // typing: open a fresh box, leaving any draft aside — a
                // game line has no business in a password field.
                //
                // Look-back I2-C32: this used to fire only from NAV mode, so
                // the one case it exists to prevent was the one it missed —
                // a half-typed line already in the box rode into the masked
                // field and `Enter` sent the game line and the password as
                // one string. The line becomes the draft instead, and comes
                // back the next time input mode opens empty.
                if spec.mode != InputMode::Normal {
                    if !self.line.is_empty() {
                        self.draft = Some(self.line.text());
                    }
                    self.mode = Mode::Input;
                    self.line.clear();
                    self.palette = None;
                }
                return queries;
            }
            OutputEvent::Room(view) => {
                self.room = Some(view.clone());
                self.exit_keys = exit_keys(&view.exits);
            }
            OutputEvent::Panel { feed, kind, data } => {
                // Interface plan I12: a `narrative` Delta is not a panel
                // frame at all — it is a page of this viewer's own resume
                // ring, answering a `narrative.backlog` query. It must never
                // reach the `PanelStore`: an unknown-feed Delta there falls
                // to the resync catch-all and asks the world to re-send a
                // feed no producer publishes, which comes back as an
                // "Unknown query" line in the scrollback and loses the page.
                if feed.as_str() == "narrative" && *kind == PanelFrameKind::Delta {
                    self.apply_narrative_backlog(data);
                    return queries;
                }
                match self.panels.apply_panel(feed, *kind, data, now) {
                    PanelOutcome::NeedsResync(f) => {
                        queries.push((f.to_string(), Value::Object(serde_json::Map::new())));
                    }
                    PanelOutcome::Stored | PanelOutcome::Ignored => {}
                }
                match feed.as_str() {
                    "commands.list" => self.set_commands(data),
                    "comsys" if *kind == PanelFrameKind::Keyframe => self.set_aliases(data),
                    "npc.thinking" => self.set_thinking(data, now),
                    // 16b cut Q.4: the cockpit frame is the one that says which
                    // match this pilot is in, so it is where the map is pulled
                    // from and where the screen opens itself.
                    COCKPIT_FEED if *kind == PanelFrameKind::Keyframe => {
                        let asks = self.follow_the_cockpit(data, now);
                        queries.extend(asks);
                    }
                    // **16b cut B.2**: the world answered a map ask `seated:
                    // false` while this client still holds a seated frame. The
                    // seat is gone and nothing said so — a world restarted
                    // behind the gateway keeps the session but sends no hello
                    // and no cleared frame, and its match died with it — so the
                    // held frame would feed the timer heal an `arena.map` ask
                    // on every tick for the rest of the session. Ask for the
                    // cockpit once: the cleared answer ends the seat, and the
                    // heal with it. The map is pulled, never pushed, so this
                    // asks at most once per map ask, which the latch bounds.
                    ARENA_MAP_FEED
                        if *kind == PanelFrameKind::Keyframe
                            && data.get("seated").and_then(Value::as_bool) == Some(false)
                            && self.cockpit_match().is_some() =>
                    {
                        queries.push((
                            COCKPIT_FEED.to_string(),
                            Value::Object(serde_json::Map::new()),
                        ));
                    }
                    _ => {}
                }
                return queries;
            }
            OutputEvent::View(view) => {
                if let PanelOutcome::NeedsResync(f) = self.panels.apply_view(view, now) {
                    queries.push((f.to_string(), Value::Object(serde_json::Map::new())));
                }
                if view.view() != "resolution.outcome" {
                    return queries;
                }
            }
            OutputEvent::Channel {
                channel,
                seq,
                actor,
                kind,
                text,
                ..
            } => {
                let plain = sanitize(&text.to_plain_text());
                let line = match kind {
                    mush_proto::NarrativeKind::Pose => format!("{} {plain}", actor.name),
                    mush_proto::NarrativeKind::SemiPose => format!("{}{plain}", actor.name),
                    mush_proto::NarrativeKind::Emit => plain,
                    mush_proto::NarrativeKind::Spoof => format!("[{}] {plain}", actor.name),
                    _ => format!("{}: {plain}", actor.name),
                };
                self.panels.push_channel(
                    channel,
                    ChanMsg {
                        seq: *seq,
                        actor: actor.name.clone(),
                        line,
                    },
                );
            }
            OutputEvent::Sys(text) => {
                // **16b cut B.3** (the TUI half of 16b cut B.1's reply ring,
                // Q20): while seated, every `Sys` line ALSO rings on the
                // cockpit's notices pane, because a pilot kept on screen 0 has
                // no scrollback in front of them — `Locking BA.` has to land
                // where `lock BA` was typed. Every line, not a chosen subset,
                // exactly as the web's `pushReply`; the one left out is the
                // bridge's copy of a frame's own notice (its first span tagged
                // `NOTICE_SENTENCE_TAG`), which the ring already composes from
                // the frame. Nor does a `Sys` that ANSWERS one of this app's
                // own queries ring (a `Reply`: a refusal or a throttle line
                // answering the WHO tick, the map heal, a backlog page or the
                // entry hydration, or a subscribe's acknowledgement) — the
                // pilot typed none of those, and the web's `#routeReply` never
                // rings one either. What the pilot types, a bare key's read
                // verb included, is SUBMITTED, so its answer arrives bare. The
                // line still falls through to the scrollback.
                let sentence = text.0.first().and_then(|span| span.tag.as_deref())
                    == Some(NOTICE_SENTENCE_TAG);
                if !sentence && !answered {
                    self.panels.push_cockpit_reply(&text.to_plain_text());
                }
            }
            OutputEvent::MailNotice { unread, .. } => {
                self.mail_unread = usize::try_from(*unread).unwrap_or(0);
            }
            OutputEvent::Notice { .. } => {
                // Count it for the status line, then fall through to render it
                // into the scrollback (the terminal's notice history). The
                // count clears in `draw` once the scene tail is back in view.
                self.notice_unseen = self.notice_unseen.saturating_add(1);
            }
            OutputEvent::Composing { actor, active } => {
                self.composing.retain(|(n, _)| n != &actor.name);
                if *active {
                    self.composing.push((sanitize(&actor.name), now));
                }
                return queries;
            }
            OutputEvent::Request {
                epoch,
                id,
                kind,
                prompt,
                options,
                deadline_ms,
            } => {
                // Interface plan I10: queue the ask, oldest first, and show
                // the front one. A repeat of an id replaces in place (the
                // producer re-sent it — a deadline moved, options changed);
                // past the subject cap the newest is dropped, because the
                // engine would not have opened it either. Expired fronts go
                // first, so an ask that died while the player was away does
                // not spend the cap or the front slot.
                self.requests.retain(|r| r.epoch == *epoch);
                self.evict_expired_requests(now);
                let req = TuiRequest {
                    epoch: epoch.clone(),
                    id: *id,
                    kind: *kind,
                    prompt: prompt.clone(),
                    options: options.clone(),
                    deadline_ms: *deadline_ms,
                };
                match self.requests.iter().position(|q| q.id == req.id) {
                    Some(at) => self.requests[at] = req,
                    None if self.requests.len() < MAX_OPEN_REQUESTS_PER_SUBJECT => {
                        self.requests.push_back(req);
                    }
                    None => {}
                }
                return queries;
            }
            OutputEvent::Hello { panels, .. } => {
                self.manifest.panels.clone_from(panels);
                // 16b cut Q.4: a world restarted without its arena takes the
                // cockpit tab with it, and the player must not be left standing
                // on a screen that is no longer drawn.
                self.leave_a_screen_this_server_does_not_have();
                // Interface plan I12: a world restarted behind the gateway
                // re-sends its hello. Clearing the LATCH here is belt and
                // braces that costs nothing — `narrative.backlog` is answered
                // by the GATEWAY from the session ring, not by the world, so
                // a page in flight across a restart does arrive; if it lands
                // after this re-arm, the cursor filter drops the duplicate.
                // The cursor itself is never re-armed: the session and its
                // resume ring outlive the world, so seqs do not restart.
                // (This is why the web's `beginSegment` is not ported — over
                // SSH nothing remints them, and re-arming here would re-fetch
                // the newest page and paint it twice.)
                self.backlog_pending = None;
                // Interface plan I10: over SSH the hello IS the world-restart
                // seam (`Resumed` is the web pump's, below). The request
                // registry is RAM-only and `next_request_id` remints from 1,
                // so every open ask died with the world: drop the queue
                // rather than let zombies stand in front of the asks the new
                // world opens. A box capturing an answer to one of them goes
                // back to nav with its text kept as a draft, so nothing typed
                // for a dead ask is sent to the new world as a command.
                self.requests.clear();
                if self.answering.take().is_some() {
                    self.leave_input(true);
                }
                let empty = || Value::Object(serde_json::Map::new());
                for feed in panels {
                    if feed.ends_with(".vitals") {
                        queries.push((feed.clone(), empty()));
                    }
                }
                // A server restarted behind the gateway re-sends its hello to
                // a live TUI. Re-fetch what the ACTIVE screen reads too, not
                // just the vitals: otherwise a tactical screen sits on the
                // frame from before the restart until the player presses `r`.
                // The replacement keyframe replaces the held frame and the
                // object-change rule takes care of the ring.
                let screen = self.screen_queries();
                self.note_map_ask(&screen, now);
                queries.extend(screen);
                // **16b cut B.2**: the cockpit is asked again from EVERY screen,
                // not only from screen 0 (whose `on_enter` above already names
                // it), where the server publishes it — the test `entry_queries`
                // makes, and the web's hello pass does on every hello. A pilot
                // who is no longer seated is answered the cleared frame, which
                // ends the seat. This is the PARITY seam with the web, and it
                // runs only when a hello actually arrives: production SSH is
                // never handed one (`entry_queries`), and a session that rides
                // a world restart behind the gateway gets no new hello on
                // either client (the gateway's late `ToGateway::Hello` only
                // refreshes its own manifest cache). What ends a dead seat
                // there is the unseated answer to the timer heal's map ask —
                // the `ARENA_MAP_FEED` arm of `Panel` above re-asks the cockpit.
                let cockpit_published =
                    self.manifest.panels.is_empty() || manifest_has(&self.manifest, COCKPIT_FEED);
                if cockpit_published && self.active != COCKPIT_SCREEN {
                    queries.push((COCKPIT_FEED.to_string(), empty()));
                }
            }
            OutputEvent::SessionInfo { character, .. } => {
                self.manifest.character = Some(sanitize(&character.name));
            }
            OutputEvent::Resumed { .. } => {
                // The mode resets on a resume (D-I-17). A pending request is
                // gone server-side after a full re-attach (I10 is RAM-only),
                // and its id may be reminted, so drop the overlay too.
                //
                // This is the WEB pump's seam, kept here for parity and
                // unreachable over SSH: nothing in the tree sends `Resumed`
                // to an Ansi sink. The SSH restart seam is the `Hello` arm
                // above; the only other eviction is the deadline.
                self.leave_input(true);
                self.requests.clear();
                self.answering = None;
                return queries;
            }
            _ => {}
        }
        let following = self.scroll_off == 0;
        for entry in render_event(event, &self.caps, &self.theme, now) {
            self.push_entry(entry);
        }
        if following {
            self.scroll_off = 0;
        }
        queries
    }

    fn push_entry(&mut self, entry: Entry) {
        if self.scrollback.len() == SCROLLBACK_CAP {
            self.scrollback.pop_front();
            // A `/clear` floor is a count of hidden FRONT entries, and the
            // cap evicts from the front: the floor tracks the eviction down,
            // so it never starts hiding live text.
            self.cleared = self.cleared.saturating_sub(1);
        }
        self.scrollback.push_back(entry);
    }

    // ---- narrative history (interface plan I12) ---------------------------

    /// Ask the gateway for the next older page of this viewer's own scene
    /// history. One page is in flight at a time, and the latch AGES: a
    /// throttled `narrative.backlog` is dropped by the bridge without any
    /// reply, so a permanent in-flight flag would wedge history for the rest
    /// of the connection. Read-only, so it never stamps idle (D-I-3).
    fn request_backlog(&mut self, r: &mut InputResult) {
        if !self.backlog_more {
            return;
        }
        let now = self.now();
        if let Some(armed) = self.backlog_pending
            && (0..BACKLOG_LATCH_S).contains(&now.saturating_sub(armed))
        {
            return;
        }
        self.backlog_pending = Some(now);
        r.queries.push((
            "narrative.backlog".to_string(),
            serde_json::json!({ "before_seq": self.backlog_cursor, "limit": NARRATIVE_PAGE }),
        ));
    }

    /// Splice one page of history in above the live tail, oldest first.
    ///
    /// The page arrives newest-first (the ring is walked in reverse), so
    /// walking it in wire order and pushing each rendered event onto the
    /// FRONT leaves the scrollback reading oldest → newest again. Each line
    /// goes through the same [`render_event`] a live one did, so a recalled
    /// line is byte-identical to how it first appeared.
    fn apply_narrative_backlog(&mut self, data: &Value) {
        self.backlog_pending = None;
        if !self.backlog_more {
            // Nothing is being asked for any more: the server's floor, a full
            // local ring, or a `/clear` view floor. `request_backlog` refuses
            // while `backlog_more` is false and only one page is ever in
            // flight, so the only page that can arrive here is one that was
            // already on the wire when the flag went down — and prepending it
            // over a `/clear` floor (a count of hidden FRONT entries) would
            // push cleared lines back into view. Drop it.
            return;
        }
        let Some(page) = crate::backlog::parse(data) else {
            // Nothing truthful to say about a page we could not read, and no
            // cursor to advance: stop asking rather than loop on it.
            self.backlog_more = false;
            return;
        };
        if page.lines.is_empty() {
            // An empty page is TERMINAL, never an error: a session reaped
            // out from under the query answers this way, and so does a ring
            // with nothing older left in it.
            self.backlog_more = false;
            self.mark_backlog_floor(page.evicted);
            return;
        }
        // The dedup: only lines strictly older than the cursor. An SSH
        // scrollback entry can carry no seq of its own, so this is all there
        // is — and all there needs to be, because entry primes the first
        // page into an EMPTY scrollback.
        let cursor = self.backlog_cursor;
        let kept: Vec<_> = page
            .lines
            .iter()
            .filter(|l| cursor.is_none_or(|c| l.seq < c))
            .collect();
        if kept.is_empty() {
            // A stale page: every line in it is one already held (a second
            // priming page, or one that raced a re-armed latch). Drop it
            // and leave the cursor and `more` alone — the next ask carries
            // the current cursor and gets the right page.
            return;
        }
        // Never evict the live tail. `push_entry` pops the FRONT at the cap,
        // so a prepend that ran past it would have to pop the BACK — live
        // scene text. Keep the NEWEST lines that fit instead and stop asking,
        // or the same page is re-fetched on every scroll to the top forever.
        let mut oldest_pushed: Option<u64> = None;
        let mut truncated = false;
        for line in kept {
            let ev = OutputEvent::Narrative {
                kind: line.kind,
                actor: line.actor.clone(),
                text: line.text.clone(),
            };
            // `Envelope.ts` is Unix MILLISECONDS; `Entry.at` is seconds.
            let entries = render_event(&ev, &self.caps, &self.theme, line.ts / 1000);
            if self.scrollback.len() + entries.len() > SCROLLBACK_CAP {
                truncated = true;
                break;
            }
            // Reversed, so a multi-row event keeps its own row order.
            for entry in entries.into_iter().rev() {
                self.scrollback.push_front(entry);
            }
            oldest_pushed = Some(line.seq);
        }
        self.backlog_cursor = oldest_pushed.or(self.backlog_cursor);
        if truncated {
            // The local ring is the floor here, not the server's buffer, so
            // no marker: it would say something untrue about the world.
            self.backlog_more = false;
            self.backlog_floored = true;
            if oldest_pushed.is_none() {
                // The cut fell before the page's FIRST line, so nothing at
                // all was painted. Say why, as a client line: a gesture that
                // silently does nothing reads as a broken key. `/clear` is
                // not the remedy any more — it hides the ring, it does not
                // empty it — so name the one that is.
                self.push_local_line(
                    "The scrollback is full; nothing older fits. \
                     @notui then @tui starts a fresh one.",
                );
            }
            return;
        }
        self.backlog_more = page.more;
        if !page.more {
            self.mark_backlog_floor(page.evicted);
        }
        // `scroll_off` is measured up from the TAIL, so a prepend does not
        // move the reader and nothing here touches it (no `prependGen`).
    }

    /// One dim line at the very top saying why nothing older will appear.
    fn mark_backlog_floor(&mut self, evicted: bool) {
        if self.backlog_floored || self.scrollback.len() >= SCROLLBACK_CAP {
            return;
        }
        self.backlog_floored = true;
        let dash = if self.caps.utf8 { "\u{2014}" } else { "--" };
        let what = if evicted {
            "older history has rolled off the server's buffer"
        } else {
            "no older history"
        };
        self.scrollback.push_front(Entry {
            gutter: None,
            body: vec![dim_span_owned(&format!("{dash} {what} {dash}"))],
            at: self.now(),
        });
    }

    // ---- server-initiated asks (interface plan I10) -----------------------

    /// Drop asks whose deadline has passed, front first; `true` if any went.
    ///
    /// The engine removes an expired request and emits NOTHING to the
    /// subject, so the clock is all a client has. Without this a dead front
    /// ask blocks every later one for the rest of the connection: the player
    /// answers a request the engine has already dropped (silently discarded
    /// server-side) and only then sees the next. Only the front is popped —
    /// one further back is evicted when it reaches the front, which is the
    /// first moment it could matter.
    fn evict_expired_requests(&mut self, now_s: i64) -> bool {
        let now_ms = u64::try_from(now_s.saturating_mul(1000)).unwrap_or(0);
        let before = self.requests.len();
        while self
            .requests
            .front()
            .is_some_and(|req| req.deadline_ms.is_some_and(|d| d <= now_ms))
        {
            self.requests.pop_front();
        }
        self.requests.len() != before
    }

    fn push_local_line(&mut self, text: &str) {
        let now = self.now();
        self.push_entry(Entry {
            gutter: Some(dim_span_owned("tui")),
            body: vec![Span::styled(
                sanitize(text),
                Style::default().add_modifier(Modifier::DIM),
            )],
            at: now,
        });
    }

    fn set_commands(&mut self, data: &Value) {
        let Some(list) = data.get("commands").and_then(Value::as_array) else {
            return;
        };
        self.commands = list
            .iter()
            .filter_map(|c| {
                let name = c.get("name").and_then(Value::as_str)?;
                Some(CommandEntry {
                    name: sanitize(name),
                    usage: sanitize(c.get("usage").and_then(Value::as_str).unwrap_or(name)),
                    summary: sanitize(c.get("summary").and_then(Value::as_str).unwrap_or("")),
                })
            })
            .collect();
    }

    fn set_aliases(&mut self, data: &Value) {
        self.channel_aliases = data
            .get("channels")
            .and_then(Value::as_array)
            .map(|chans| {
                chans
                    .iter()
                    .filter_map(|c| c.get("alias").and_then(Value::as_str))
                    .filter(|a| !a.is_empty())
                    .map(sanitize)
                    .collect()
            })
            .unwrap_or_default();
    }

    fn set_thinking(&mut self, data: &Value, now: i64) {
        let name = data
            .get("name")
            .and_then(Value::as_str)
            .map(sanitize)
            .unwrap_or_default();
        let active = data.get("active").and_then(Value::as_bool).unwrap_or(true);
        self.thinking.retain(|(n, _)| n != &name);
        if active && !name.is_empty() {
            self.thinking.push((name, now));
        }
    }

    // ---- keys -------------------------------------------------------------

    /// Decode a burst of raw input bytes and act on every key.
    pub fn feed_input(&mut self, bytes: &[u8]) -> InputResult {
        let mut result = InputResult::default();
        for key in self.parser.feed(bytes) {
            self.handle_key(key, &mut result);
        }
        result
    }

    /// Act on one decoded key (tests drive this directly).
    pub fn handle_key(&mut self, key: Key, r: &mut InputResult) {
        r.redraw = true;
        match key {
            Key::Ctrl(']') => {
                r.detach = true;
                return;
            }
            Key::Ctrl('k') => {
                self.toggle_palette();
                return;
            }
            Key::Ctrl('p') => {
                self.sticky = self.sticky.next();
                return;
            }
            Key::Ctrl('n') => {
                self.next_channel();
                return;
            }
            Key::Ctrl('r') => {
                // I6: the roll builder is the line — `+roll ` ready, Tab
                // completing the sheet's traits inside it.
                self.enter_input(Some("+roll ".to_string()));
                return;
            }
            Key::Ctrl('o') => {
                self.toggle_channel_pane();
                return;
            }
            Key::F(n) if (1..=9).contains(&n) => {
                self.switch_screen(n, r);
                return;
            }
            _ => {}
        }
        // The overlay owns the keyboard while an ask is showing — but not
        // while the box is capturing a free-text answer to one (I10): the
        // next ask waits its turn rather than eating what is being typed
        // (which is also why the eviction below runs only when the overlay
        // has the keys — it must never swallow a typed character).
        if self.answering.is_none() && !self.requests.is_empty() {
            // A dead ask never eats a key and never hides the ones behind it.
            // The keystroke that finds one is SPENT on the eviction: the ask
            // underneath is drawn before it can be answered by a player who
            // has not seen it yet.
            if self.evict_expired_requests(self.now()) {
                return;
            }
            self.request_key(key, r);
            return;
        }
        if self.help {
            if matches!(
                key,
                Key::Esc | Key::Char('?') | Key::Alt('?') | Key::Char('q')
            ) {
                self.help = false;
            }
            return;
        }
        match self.mode {
            Mode::Nav => self.nav_key(key, r),
            Mode::Input => self.input_key(key, r),
        }
    }

    fn nav_key(&mut self, key: Key, r: &mut InputResult) {
        match key {
            Key::Char(c @ '0'..='9') => self.switch_screen(c as u8 - b'0', r),
            Key::Enter => self.enter_input(None),
            Key::Char(c @ ('"' | ':' | ';' | '\\')) => self.enter_input(Some(c.to_string())),
            Key::Char('/') => {
                self.enter_input(Some("/".to_string()));
                self.open_palette();
            }
            Key::Esc => self.palette = None,
            Key::Tab => self.move_focus(1),
            Key::BackTab => self.move_focus(-1),
            Key::Up => self.scroll_focused(-1, r),
            Key::Down => self.scroll_focused(1, r),
            Key::PgUp => self.scroll_focused(-(PAGE as isize), r),
            Key::PgDn => self.scroll_focused(PAGE as isize, r),
            Key::Char('g') => self.jump_focused(false, r),
            Key::Char('G') => self.jump_focused(true, r),
            Key::Char('?') | Key::Alt('?') => self.help = true,
            Key::Alt(c @ '0'..='9') => self.switch_screen(c as u8 - b'0', r),
            Key::Char(c) | Key::Alt(c) => self.screen_key(c, r),
            _ => r.redraw = false,
        }
    }

    /// A screen-level key: an exit on the scene screen, else the screen's
    /// binding; nothing for an unbound key.
    fn screen_key(&mut self, c: char, r: &mut InputResult) {
        if self.screen().exits_are_keys
            && let Some((_, name)) = self.exit_keys.iter().find(|(k, _)| *k == c)
        {
            r.submit.push(name.clone());
            return;
        }
        let Some(binding) = self.screen().binding(c).copied() else {
            r.redraw = false;
            return;
        };
        match binding.action {
            KeyAction::Local(action) => self.local_action(action, r),
            KeyAction::Prefill(p) => self.enter_input(Some(p.to_string())),
            KeyAction::PrefillRow(p) => {
                let key = self.selected_row_key().unwrap_or_default();
                self.enter_input(Some(format!("{p}{key}")));
            }
            KeyAction::Run(cmd) => r.submit.push(cmd.to_string()),
        }
    }

    fn local_action(&mut self, action: LocalAction, r: &mut InputResult) {
        match action {
            LocalAction::Refresh => {
                // On the cockpit screen this is `p`, pull (16b cut B.2): its
                // `on_enter` already carries both arena feeds and already drops
                // the map ask when the ground is held, so the cockpit is asked
                // again always and the map only when it has gone dark — and a
                // map ask that did go out arms the latch, so the next frame and
                // the timer count it rather than asking a second time.
                let queries = self.screen_queries();
                let now = self.now();
                self.note_map_ask(&queries, now);
                r.queries.extend(queries);
            }
            LocalAction::Backlog => {
                if let Some(channel) = self.selected_channel() {
                    r.queries.push((
                        "comsys.backlog".to_string(),
                        serde_json::json!({
                            "channel": channel,
                            "before_seq": Value::Null,
                            "limit": BACKLOG_LIMIT,
                        }),
                    ));
                }
            }
            LocalAction::ClearScrollback => {
                // A VIEW floor, not a data clear (interface plan I12). The
                // entries stay in the ring and the history cursor stays where
                // it is: re-arming it here asked the gateway for the NEWEST
                // page again, and every line that had arrived live since the
                // clear came back in it and was painted a second time (an SSH
                // scrollback entry carries no seq to dedup against). Older
                // history is not loaded into a cleared view either — it is
                // exhausted for THIS view; `@notui` then `@tui` builds a
                // fresh one, which primes from the newest again.
                self.cleared = self.scrollback.len();
                self.scroll_off = 0;
                self.backlog_more = false;
            }
        }
    }

    fn input_key(&mut self, key: Key, r: &mut InputResult) {
        if let Key::Alt(c) = key {
            match c {
                '0'..='9' => self.switch_screen(c as u8 - b'0', r),
                '?' => self.help = true,
                _ => self.screen_key(c, r),
            }
            return;
        }
        if self.palette.is_some() {
            match key {
                Key::Up => {
                    if let Some(p) = &mut self.palette {
                        p.selected = p.selected.saturating_sub(1);
                    }
                    return;
                }
                Key::Down => {
                    if let Some(p) = &mut self.palette {
                        p.selected = (p.selected + 1).min(p.entries.len().saturating_sub(1));
                    }
                    return;
                }
                Key::Tab => {
                    self.pick_palette();
                    return;
                }
                Key::Enter => {
                    // A pick that changes nothing (the line already is the
                    // entry, or nothing matched) falls through to submit, so
                    // `/clear` typed in full runs on one Enter.
                    if self.pick_palette() {
                        return;
                    }
                }
                Key::Esc => {
                    self.palette = None;
                    return;
                }
                _ => {}
            }
        }
        match key {
            Key::Enter => {
                if self.line.is_empty() {
                    // An empty line IS the cancel `submit_line` documents —
                    // it never reaches `submit_line`, so send it here.
                    self.cancel_answer(r);
                    self.leave_input(false);
                    return;
                }
                let line = self.line.submit(self.input_mode == InputMode::Normal);
                self.submit_line(line, r);
            }
            Key::Esc => {
                // The overlay's own printed contract: "Esc cancels (the game
                // decides for you)".
                self.cancel_answer(r);
                self.leave_input(true);
            }
            Key::Char('/') if self.line.is_empty() => {
                self.line.insert('/');
                self.open_palette();
            }
            Key::Char(c) => {
                self.line.insert(c);
                self.refilter_palette();
            }
            Key::Backspace => {
                self.line.backspace();
                if self.line.is_empty() {
                    self.palette = None;
                } else {
                    self.refilter_palette();
                }
            }
            Key::Delete => {
                self.line.delete();
            }
            Key::Left => {
                self.line.left();
            }
            Key::Right => {
                self.line.right();
            }
            Key::Home | Key::Ctrl('a') => {
                self.line.home();
            }
            Key::End | Key::Ctrl('e') => {
                self.line.end();
            }
            Key::Ctrl('u') => {
                self.line.kill_to_start();
            }
            Key::Ctrl('w') => {
                self.line.kill_word();
            }
            Key::Ctrl('c') => self.line.clear(),
            Key::Up => {
                if self.input_mode == InputMode::Normal {
                    self.line.history_up();
                }
            }
            Key::Down => {
                if self.input_mode == InputMode::Normal {
                    self.line.history_down();
                }
            }
            Key::PgUp => {
                // The box-mode page keys scroll the scrollback whatever pane
                // has focus (they always have); paging past its top asks for
                // older history exactly as nav-mode PgUp does (I12).
                self.scroll_off = self.scroll_off.saturating_add(PAGE);
                if self.scroll_off >= self.last_max_scroll {
                    self.request_backlog(r);
                }
            }
            Key::PgDn => self.scroll_off = self.scroll_off.saturating_sub(PAGE),
            Key::Tab => self.complete(),
            _ => r.redraw = false,
        }
    }

    /// A submitted line: a client-local `/command`, else the sticky-mode
    /// form of a game line (D-I-8: an explicit trigger or a channel alias
    /// verb always wins).
    /// Answer the server-initiated request overlay (interface plan I10,
    /// D-I-21): `y`/`n` for a confirm, a digit for a choice, Enter to type a
    /// target/text answer in the box, Esc to cancel (the server then applies
    /// its default). Only the subject sees this, so nothing here idle-stamps.
    /// Answers go to the OLDEST open ask, which is the one the overlay shows
    /// and the one the engine's `respond` resolves.
    fn request_key(&mut self, key: Key, r: &mut InputResult) {
        let (id, epoch, kind) = match self.requests.front() {
            Some(req) => (req.id, req.epoch.clone(), req.kind),
            None => return,
        };
        let choice = match kind {
            RequestKind::Confirm => match key {
                Key::Char('y' | 'Y') => Some(RespondChoice::Confirm(true)),
                Key::Char('n' | 'N') => Some(RespondChoice::Confirm(false)),
                Key::Esc => Some(RespondChoice::Cancel),
                _ => None,
            },
            RequestKind::Choice => match key {
                Key::Char(c @ '1'..='9') => {
                    let idx = (c as usize) - ('1' as usize);
                    self.requests
                        .front()
                        .and_then(|req| req.options.get(idx))
                        .map(|o| RespondChoice::Choose(o.id.clone()))
                }
                Key::Esc => Some(RespondChoice::Cancel),
                _ => None,
            },
            RequestKind::Target | RequestKind::Text => match key {
                Key::Enter => {
                    // Capture the free-text answer in the box. The ask
                    // leaves the queue here (`answering` carries its id), so
                    // the box is not fighting its own overlay for keys.
                    self.answering = Some((id, epoch));
                    self.requests.pop_front();
                    self.enter_input(None);
                    return;
                }
                Key::Esc => Some(RespondChoice::Cancel),
                _ => None,
            },
        };
        match choice {
            Some(c) => {
                r.responds.push((id, epoch, c));
                // Answered or cancelled, the front ask is done — and the
                // next one, if any, is now showing.
                self.requests.pop_front();
            }
            None => r.redraw = false,
        }
    }

    fn submit_line(&mut self, line: String, r: &mut InputResult) {
        if let Some((id, epoch)) = self.answering.take() {
            // The box was capturing a target/text answer (I10): the line is
            // the answer, not a command; an empty line cancels.
            let choice = if line.trim().is_empty() {
                RespondChoice::Cancel
            } else {
                RespondChoice::Text(line)
            };
            r.responds.push((id, epoch, choice));
            return;
        }
        self.scroll_off = 0;
        if let Some(local) = line.strip_prefix('/') {
            match local.trim() {
                "clear" => self.local_action(LocalAction::ClearScrollback, r),
                "help" => self.help = true,
                "history" => {
                    self.request_backlog(r);
                    // `submit_line` pinned the view to the tail above, and
                    // the page prepends at the TOP: without this the command
                    // loads a page the player cannot see and looks like it
                    // did nothing. The paint clamps it, exactly as `g` does.
                    self.scroll_off = usize::MAX / 2;
                }
                "notui" => r.detach = true,
                other => self.push_local_line(&format!("Unknown client command: /{other}")),
            }
            return;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        let explicit = line.starts_with(['"', ':', ';', '\\'])
            || self
                .channel_aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(first));
        let out = match self.sticky {
            StickyMode::None => line,
            _ if explicit => line,
            StickyMode::Say => format!("say {line}"),
            StickyMode::Pose => format!("pose {line}"),
            StickyMode::Emit => format!("@emit {line}"),
            StickyMode::Ooc => format!("ooc {line}"),
        };
        r.submit.push(out);
    }

    /// Cancel the ask the box is capturing a free-text answer for, if any.
    ///
    /// Every exit from that box goes through here. `answering` gates both the
    /// key routing in [`TuiApp::handle_key`] and the overlay in
    /// [`TuiApp::draw`], so one left dangling wedges the whole queue: the next
    /// ask is neither drawn nor keyed, and the player's next command line is
    /// eaten as an answer to an ask that is no longer on screen.
    /// [`TuiApp::leave_input`] cannot do it — `Resumed` calls that too, and
    /// there no answer should ride the wire.
    fn cancel_answer(&mut self, r: &mut InputResult) {
        if let Some((id, epoch)) = self.answering.take() {
            r.responds.push((id, epoch, RespondChoice::Cancel));
        }
    }

    /// Enter input mode: with `prefill`, or restoring a kept draft, or
    /// prefilled from the focused pane's highlighted row, or empty.
    fn enter_input(&mut self, prefill: Option<String>) {
        let text = match prefill {
            Some(p) => {
                self.draft = None;
                Some(p)
            }
            None => self.draft.take().or_else(|| self.selected_prefill()),
        };
        self.mode = Mode::Input;
        match text {
            Some(t) => self.line.set(&t),
            None => self.line.clear(),
        }
    }

    /// Leave input mode, keeping a non-empty line as the draft when asked.
    fn leave_input(&mut self, keep_draft: bool) {
        if keep_draft && !self.line.is_empty() {
            self.draft = Some(self.line.text());
        }
        self.line.clear();
        self.palette = None;
        self.mode = Mode::Nav;
    }

    /// A screen digit. A digit this server has no screen for — an unregistered
    /// one, or a gated one the manifest does not carry (16b cut Q.4) — is inert
    /// rather than an error: the tab is not drawn either.
    fn switch_screen(&mut self, digit: u8, r: &mut InputResult) {
        let id = ScreenId(digit);
        if !self.available_screens().any(|s| s.id == id) {
            r.redraw = false;
            return;
        }
        if self.active != id {
            self.active = id;
            self.focus = 0;
            self.help = false;
            // Entering the cockpit while its ground is dark asks for the map
            // through the screen's own `on_enter` (16b cut B.2): that ask arms
            // the latch like any other, or the pump's next tick — the latch
            // long aged on a pilot who sat on screen 1 — would pull the whole
            // terrain a second time while the first answer is still on the wire.
            let queries = self.screen_queries();
            let now = self.now();
            self.note_map_ask(&queries, now);
            r.queries.extend(queries);
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let n = self.screen().panes.len();
        if n == 0 {
            return;
        }
        let cur = self.focus as isize;
        self.focus = ((cur + delta).rem_euclid(n as isize)) as usize;
    }

    /// Scroll or move the highlight of the focused pane by `delta` rows.
    /// Takes the [`InputResult`] because scrolling up off the top of the
    /// scrollback asks the server for older history (interface plan I12).
    fn scroll_focused(&mut self, delta: isize, r: &mut InputResult) {
        let Some(spec) = self.screen().panes.get(self.focus).copied() else {
            return;
        };
        match spec.content {
            PaneContent::Scrollback => {
                let off = self.scroll_off as isize - delta;
                self.scroll_off = off.max(0) as usize;
                // Already at the top of the local buffer and still going up:
                // that is the ask for the page above it (§3.12).
                if delta < 0 && self.scroll_off >= self.last_max_scroll {
                    self.request_backlog(r);
                }
            }
            _ if spec.selectable => {
                let rows = self.pane_rows(&spec, None);
                let selectable: Vec<usize> = rows
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.selectable)
                    .map(|(i, _)| i)
                    .collect();
                if selectable.is_empty() {
                    return;
                }
                let current = self.anchored(spec.id, &rows);
                let pos = selectable.iter().position(|&i| i == current).unwrap_or(0) as isize;
                let next = (pos + delta).clamp(0, selectable.len() as isize - 1) as usize;
                self.remember_selection(spec.id, selectable[next], &rows);
            }
            _ => {
                let ui = self.pane_ui_mut(spec.id);
                let off = ui.scroll as isize + delta;
                ui.scroll = off.max(0) as usize;
            }
        }
    }

    /// Jump the focused pane to its top or bottom. `g` on the scrollback is
    /// also the plan's explicit "load older history" gesture (§3.12) — it is
    /// a JUMP key, so no room exit can ever shadow it.
    fn jump_focused(&mut self, bottom: bool, r: &mut InputResult) {
        let Some(spec) = self.screen().panes.get(self.focus).copied() else {
            return;
        };
        match spec.content {
            PaneContent::Scrollback => {
                self.scroll_off = if bottom { 0 } else { usize::MAX / 2 };
                if !bottom {
                    self.request_backlog(r);
                }
            }
            _ if spec.selectable => {
                let rows = self.pane_rows(&spec, None);
                let selectable: Vec<usize> = rows
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.selectable)
                    .map(|(i, _)| i)
                    .collect();
                if let Some(&i) = if bottom {
                    selectable.last()
                } else {
                    selectable.first()
                } {
                    self.remember_selection(spec.id, i, &rows);
                }
            }
            _ => {
                let ui = self.pane_ui_mut(spec.id);
                ui.scroll = if bottom { usize::MAX / 2 } else { 0 };
            }
        }
    }

    /// The highlighted row of the focused pane, if it is a row pane.
    fn selected_row(&self) -> Option<super::render::PaneRow> {
        let spec = self.screen().panes.get(self.focus).copied()?;
        if !spec.selectable {
            return None;
        }
        let rows = self.pane_rows(&spec, None);
        let at = self.anchored(spec.id, &rows);
        rows.rows
            .get(at)
            .filter(|r| r.selectable)
            .cloned()
            .or_else(|| rows.rows.into_iter().find(|r| r.selectable))
    }

    /// The line `Enter` prefills: the focused pane's highlighted row, else —
    /// on a screen with channel panes — the channel [`Self::selected_channel`]
    /// already answers with (I7-C12). On a stock server every channel is OOC,
    /// so the channel screen opens with focus on an EMPTY IC pane whose only
    /// row is non-selectable; without this fallback screen 4's advertised
    /// `Enter type` opened a blank line.
    fn selected_prefill(&self) -> Option<String> {
        self.selected_row()
            .and_then(|r| r.prefill)
            .or_else(|| self.selected_channel_row().and_then(|r| r.prefill))
    }

    fn selected_row_key(&self) -> Option<String> {
        self.selected_row().and_then(|r| r.row_key)
    }

    /// The indexes of the active screen's channel panes, the focused one
    /// first.
    fn channel_pane_indexes(&self) -> Vec<usize> {
        let panes = &self.screen().panes;
        let mut idx: Vec<usize> = (0..panes.len())
            .filter(|&i| matches!(panes[i].content, PaneContent::Channels { .. }))
            .collect();
        if let Some(pos) = idx.iter().position(|&i| i == self.focus) {
            idx.rotate_left(pos);
        }
        idx
    }

    /// The channel ROW highlighted on the active screen: the focused channel
    /// pane's highlight, else the first channel any channel pane lists
    /// (the IC pane is often empty; the OOC pane then answers). The whole row
    /// rather than its key, so `Enter` can reach the row's prefill too.
    fn selected_channel_row(&self) -> Option<super::render::PaneRow> {
        for i in self.channel_pane_indexes() {
            let spec = self.screen().panes[i];
            let rows = self.pane_rows(&spec, None);
            let at = self.anchored(spec.id, &rows);
            let row = rows
                .rows
                .get(at)
                .filter(|r| r.row_key.is_some())
                .or_else(|| rows.rows.iter().find(|r| r.row_key.is_some()))
                .cloned();
            if row.is_some() {
                return row;
            }
        }
        None
    }

    /// The name of the channel [`Self::selected_channel_row`] names: the key
    /// `b` asks backlog for and the backlog pane's title.
    fn selected_channel(&self) -> Option<String> {
        self.selected_channel_row().and_then(|r| r.row_key)
    }

    /// `^N` (interface plan I7): highlight the next channel of the focused
    /// channel pane, wrapping; focus moves to a channel pane first when it
    /// is elsewhere. Nothing on a screen without channel panes.
    fn next_channel(&mut self) {
        let Some(&i) = self.channel_pane_indexes().first() else {
            return;
        };
        self.focus = i;
        let spec = self.screen().panes[i];
        let rows = self.pane_rows(&spec, None);
        let selectable: Vec<usize> = rows
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable)
            .map(|(i, _)| i)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let cur = self.anchored(spec.id, &rows);
        let pos = selectable.iter().position(|&s| s == cur);
        let next = pos.map_or(0, |p| (p + 1) % selectable.len());
        self.remember_selection(spec.id, selectable[next], &rows);
    }

    /// `^O` (interface plan I7): focus the other channel pane (IC to OOC
    /// and back). Nothing on a screen without two.
    fn toggle_channel_pane(&mut self) {
        let idx = self.channel_pane_indexes();
        if idx.len() < 2 {
            return;
        }
        self.focus = if idx[0] == self.focus { idx[1] } else { idx[0] };
    }

    // ---- palette and completion ------------------------------------------

    fn toggle_palette(&mut self) {
        if self.palette.is_some() {
            self.palette = None;
            return;
        }
        if self.mode == Mode::Nav {
            self.enter_input(Some("/".to_string()));
        } else if self.line.is_empty() {
            self.line.insert('/');
        }
        self.open_palette();
    }

    fn open_palette(&mut self) {
        self.palette = Some(Palette::default());
        self.refilter_palette();
    }

    /// The text after the slash, or the whole line when it has none.
    fn palette_query(&self) -> String {
        let text = self.line.text();
        text.strip_prefix('/').unwrap_or(&text).to_lowercase()
    }

    fn refilter_palette(&mut self) {
        if self.palette.is_none() {
            return;
        }
        let q = self.palette_query();
        let mut prefix: Vec<PaletteEntry> = Vec::new();
        let mut rest: Vec<PaletteEntry> = Vec::new();
        let mut consider = |entry: PaletteEntry, key: &str| {
            let k = key.to_lowercase();
            if q.is_empty() || k.starts_with(&q) {
                prefix.push(entry);
            } else if k.contains(&q) {
                rest.push(entry);
            }
        };
        for (cmd, desc) in LOCAL_COMMANDS {
            consider(
                PaletteEntry {
                    insert: cmd.to_string(),
                    label: cmd.to_string(),
                    detail: desc.to_string(),
                },
                cmd,
            );
        }
        for alias in &self.channel_aliases {
            consider(
                PaletteEntry {
                    insert: format!("{alias} "),
                    label: alias.clone(),
                    detail: "channel".to_string(),
                },
                alias,
            );
        }
        if let Some(room) = &self.room {
            for exit in &room.exits {
                consider(
                    PaletteEntry {
                        insert: exit.name.clone(),
                        label: sanitize(&exit.name),
                        detail: "exit".to_string(),
                    },
                    &exit.name,
                );
            }
        }
        for c in &self.commands {
            consider(
                PaletteEntry {
                    insert: c.usage.clone(),
                    label: c.name.clone(),
                    detail: c.summary.clone(),
                },
                &c.name,
            );
        }
        let mut entries = prefix;
        entries.extend(rest);
        entries.truncate(MAX_PALETTE_ROWS * 4);
        if let Some(p) = &mut self.palette {
            p.entries = entries;
            p.selected = 0;
        }
    }

    /// Replace the `/query` with the highlighted entry (a client-local
    /// command keeps its slash) and close the palette. `true` when the line
    /// changed.
    fn pick_palette(&mut self) -> bool {
        let Some(p) = self.palette.take() else {
            return false;
        };
        match p.entries.get(p.selected) {
            Some(e) if e.insert != self.line.text() => {
                self.line.set(&e.insert);
                true
            }
            _ => false,
        }
    }

    /// Tab completion over `commands.list`: complete the first word to the
    /// first command it prefixes.
    fn complete(&mut self) {
        let text = self.line.text();
        if text.is_empty() {
            return;
        }
        if let Some(rest) = text.strip_prefix("+roll ") {
            // I6: a trait inside a roll line — the token after the last
            // space or `+`, against the sheet's categorised dotted fields.
            let start = rest.rfind([' ', '+']).map_or(0, |i| i + 1);
            let token = rest[start..].to_lowercase();
            if token.is_empty() {
                return;
            }
            let mut traits: Vec<String> = self
                .sheet_traits()
                .into_iter()
                .filter(|t| t.starts_with(&token))
                .collect();
            traits.sort();
            if let Some(t) = traits.first() {
                let head = &rest[..start];
                self.line.set(&format!("+roll {head}{t}"));
            }
            return;
        }
        if text.contains(' ') {
            return;
        }
        let lower = text.to_lowercase();
        let mut matches = self
            .commands
            .iter()
            .filter(|c| c.name.to_lowercase().starts_with(&lower))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>();
        matches.sort();
        if let Some(m) = matches.first() {
            self.line.set(&format!("{m} "));
        }
    }

    /// The sheet's rollable traits (I6): every dotted field with a
    /// `category`, lowercased, from the first `.sheet` keyframe held. The
    /// schema is the only source; nothing here names a trait.
    fn sheet_traits(&self) -> Vec<String> {
        let Some(sheet) = self.panels.sheet() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let sections = sheet
            .get("schema")
            .and_then(|s| s.get("sections"))
            .and_then(Value::as_array);
        for section in sections.into_iter().flatten() {
            let Some(fields) = section.get("fields").and_then(Value::as_array) else {
                continue;
            };
            for f in fields {
                let dots = f
                    .get("ty")
                    .and_then(|t| t.get("kind"))
                    .and_then(Value::as_str)
                    == Some("dots");
                let categorised = f.get("category").and_then(Value::as_str).is_some();
                if dots
                    && categorised
                    && let Some(id) = f.get("id").and_then(Value::as_str)
                {
                    out.push(id.to_lowercase());
                }
            }
        }
        out
    }

    // ---- rows and layout --------------------------------------------------

    fn render_ctx(&self) -> RenderCtx<'_> {
        RenderCtx {
            theme: &self.theme,
            depth: self.caps.color,
            utf8: self.caps.utf8,
            store: &self.panels,
            selected_channel: None,
            pane: None,
        }
    }

    /// The rows a pane shows right now.
    ///
    /// **16b cut Q.4** added `area`, the pane's inner `(width, height)` where
    /// the caller has one: [`Self::draw_pane`] passes the rect it is about to
    /// paint into, and the input paths — scrolling, jumping, the highlighted
    /// row — pass `None`, because they ask what the rows ARE and not how many
    /// of them fit. Two renderers read it (`RenderCtx::pane`): the cockpit
    /// map, which sizes its window and sheds its chrome by it, and — since 16b
    /// cut B.3 — the cockpit's notices pane, which answers the tail of its
    /// ring that fits. Neither is selectable, so no input path ever needs it.
    fn pane_rows(&self, spec: &PaneSpec, area: Option<(u16, u16)>) -> PaneRows {
        let ctx = RenderCtx {
            pane: area,
            ..self.render_ctx()
        };
        match spec.content {
            PaneContent::Scrollback | PaneContent::Who => PaneRows::default(),
            PaneContent::Panel { feed, render } => {
                let feed = self.resolve_feed(feed);
                match self.panels.get(feed) {
                    Some(data) => render(data, &ctx),
                    // A feed whose first keyframe is still in flight is
                    // honestly loading — except a `.unit` feed (interface plan
                    // I8), whose frame may never come: a refusal lands in the
                    // scrollback, which neither tactical screen shows, and a
                    // composition without the tactical module answers nothing
                    // at all. Its renderers have an honest empty state (the
                    // no-unit line, or the after-action ring when one has
                    // arrived), so hand them the empty frame rather than a
                    // spinner that never stops.
                    None if feed.ends_with(".unit") => render(&Value::Null, &ctx),
                    // A module feed this server does not publish at all (the
                    // contacts table on a server without the propagation
                    // layer) is declared-empty, not loading: nothing will ever
                    // arrive.
                    None if feed.contains('.')
                        && !self.manifest.panels.is_empty()
                        && !manifest_has(&self.manifest, feed) =>
                    {
                        PaneRows::placeholder("(not published by this server)")
                    }
                    None => loading(),
                }
            }
            PaneContent::View { view, render } => match self.panels.get(view) {
                Some(data) => render(data, &ctx),
                None => loading(),
            },
            PaneContent::Sheet => match self.panels.sheet() {
                Some(data) => render_sheet_rows(data, &ctx),
                None => loading(),
            },
            PaneContent::Channels { ic } => match self.panels.get("comsys") {
                Some(data) => render_channels_rows(data, &ctx, ic),
                None => loading(),
            },
            PaneContent::Backlog => {
                let channel = self.selected_channel();
                let msgs = channel
                    .as_deref()
                    .map(|c| self.panels.backlog(c))
                    .unwrap_or_default();
                render_backlog_rows(&msgs, channel.as_deref())
            }
            PaneContent::Rolls => {
                let rolls: Vec<Value> = self.panels.rolls().iter().cloned().collect();
                render_rolls_rows(&rolls)
            }
            PaneContent::Vitals => render_vitals_rows(&self.panels.vitals(), self.caps.utf8),
            PaneContent::Placeholder(text) => PaneRows::placeholder(text),
        }
    }

    /// The panes of the active screen that fit `area`, honouring the drop
    /// rules: the WHO rail below [`WHO_RAIL_MIN_W`] columns, then the
    /// screen's `degrade` order while the main pane would be starved.
    fn visible_panes(&self, area: Rect) -> Vec<PaneSpec> {
        let screen = self.screen();
        let mut panes: Vec<PaneSpec> = screen.panes.clone();
        if area.width < WHO_RAIL_MIN_W {
            panes.retain(|p| !matches!(p.content, PaneContent::Who));
        }
        let fits = |panes: &[PaneSpec]| {
            let col_w = |slot: PaneSlot| {
                panes
                    .iter()
                    .filter(|p| p.slot == slot)
                    .map(|p| p.min_w)
                    .max()
                    .unwrap_or(0)
            };
            let main_min = panes
                .iter()
                .filter(|p| p.slot == PaneSlot::Main)
                .map(|p| p.min_w)
                .max()
                .unwrap_or(0);
            let width_ok = col_w(PaneSlot::Left) + col_w(PaneSlot::Right) + main_min <= area.width;
            let height_ok = |slot: PaneSlot| {
                let need: u16 = panes
                    .iter()
                    .filter(|p| p.slot == slot)
                    .map(|p| p.min_h + 2)
                    .sum();
                need <= area.height
            };
            width_ok
                && height_ok(PaneSlot::Left)
                && height_ok(PaneSlot::Right)
                && height_ok(PaneSlot::Main)
        };
        for drop in &screen.degrade {
            if fits(&panes) {
                break;
            }
            panes.retain(|p| p.id != *drop);
        }
        if !fits(&panes) {
            // Keep only the main column as a last resort.
            panes.retain(|p| p.slot == PaneSlot::Main);
        }
        panes
    }

    // ---- drawing ------------------------------------------------------------

    /// Paint one frame into `frame`.
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let now = self.now();
        self.thinking.retain(|(_, at)| now - at < THINKING_TTL_S);
        self.composing.retain(|(_, at)| now - at < COMPOSING_TTL_S);
        // The scene tail is the terminal's notice surface: while it is in view
        // (following the tail on a screen that shows the scrollback), notices
        // are seen as they land, so the unseen count clears (I11).
        if self.scroll_off == 0
            && self
                .screen()
                .panes
                .iter()
                .any(|p| matches!(p.content, PaneContent::Scrollback))
        {
            self.notice_unseen = 0;
        }
        let chrome = area.height >= MIN_CHROME_ROWS;
        let vitals = self.panels.vitals();
        let gauge_h = u16::from(chrome && !vitals.is_empty());
        let chrome_h = u16::from(chrome);
        let input_h = INPUT_H.min(area.height);
        let [
            tabs_area,
            status_area,
            body_area,
            gauge_area,
            input_area,
            legend_area,
        ] = Layout::vertical([
            Constraint::Length(chrome_h),
            Constraint::Length(chrome_h),
            Constraint::Min(0),
            Constraint::Length(gauge_h),
            Constraint::Length(input_h),
            Constraint::Length(chrome_h),
        ])
        .areas(area);
        if chrome {
            self.draw_tabs(frame, tabs_area, now);
            self.draw_status(frame, status_area);
            self.draw_legend(frame, legend_area);
        }
        if gauge_h > 0 {
            frame.render_widget(
                Paragraph::new(vitals_line(
                    &vitals,
                    self.caps.utf8,
                    gauge_area.width as usize,
                    self.manifest.character.as_deref(),
                )),
                gauge_area,
            );
        }
        self.draw_body(frame, body_area);
        self.draw_input(frame, input_area);
        if self.palette.is_some() {
            self.draw_palette(frame, body_area);
        }
        if self.help {
            self.draw_help(frame, area);
        }
        // An ask the engine has already expired is never painted: it would
        // stand in front of every live one behind it (I10).
        self.evict_expired_requests(now);
        if self.answering.is_none() && !self.requests.is_empty() {
            self.draw_request(frame, area);
        }
    }

    fn draw_tabs(&self, frame: &mut Frame<'_>, area: Rect, now: i64) {
        let comsys_unread: u64 = self
            .panels
            .get("comsys")
            .and_then(|d| d.get("channels"))
            .and_then(Value::as_array)
            .map(|chans| {
                chans
                    .iter()
                    .filter_map(|c| c.get("unread").and_then(Value::as_u64))
                    .sum()
            })
            .unwrap_or(0);
        let tabs: Vec<(u8, &str, usize)> = self
            .available_screens()
            .map(|s| {
                let badge = match s.name {
                    "mail" => self.mail_unread,
                    "chan" => usize::try_from(comsys_unread).unwrap_or(0),
                    _ => 0,
                };
                (s.id.0, s.name, badge)
            })
            .collect();
        let dot = if self.caps.utf8 { "\u{25cf}" } else { "*" };
        let mut right = String::new();
        if let Some(name) = &self.manifest.character {
            right.push_str(name);
            right.push(' ');
        }
        right.push_str(&format!(
            "{dot} {} on {}",
            self.who.len(),
            super::render::hhmm(now)
        ));
        frame.render_widget(
            Paragraph::new(tab_strip(
                &tabs,
                self.active.0,
                &right,
                usize::from(area.width),
            )),
            area,
        );
    }

    fn draw_status(&self, frame: &mut Frame<'_>, area: Rect) {
        let exit_keys = if self.screen().exits_are_keys {
            self.exit_keys.clone()
        } else {
            Vec::new()
        };
        let mut line = status_line(
            self.room.as_ref(),
            &exit_keys,
            self.caps.utf8,
            &self.theme,
            self.caps.color,
        );
        let sep = if self.caps.utf8 { " \u{00b7} " } else { " | " };
        for (name, _) in &self.thinking {
            line.spans.push(dim_span_owned(sep));
            line.spans
                .push(dim_span_owned(&format!("{name} is thinking")));
        }
        for (name, _) in &self.composing {
            line.spans.push(dim_span_owned(sep));
            line.spans
                .push(dim_span_owned(&format!("{name} is typing")));
        }
        if self.notice_unseen > 0 {
            let bell = if self.caps.utf8 { "\u{1f514}" } else { "!" };
            line.spans.push(dim_span_owned(sep));
            line.spans.push(Span::styled(
                format!("{bell} {} new", self.notice_unseen),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(Paragraph::new(line), area);
    }

    fn draw_legend(&self, frame: &mut Frame<'_>, area: Rect) {
        let bindings: Vec<(char, &str)> = self
            .screen()
            .keys
            .iter()
            .map(|b| (b.key, b.label))
            .collect();
        frame.render_widget(
            Paragraph::new(legend_line(
                self.mode == Mode::Input,
                &bindings,
                self.caps.utf8,
            )),
            area,
        );
    }

    fn draw_body(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if area.height == 0 {
            return;
        }
        let panes = self.visible_panes(area);
        let col_w = |slot: PaneSlot| {
            panes
                .iter()
                .filter(|p| p.slot == slot)
                .map(|p| p.min_w)
                .max()
                .unwrap_or(0)
        };
        let left_w = col_w(PaneSlot::Left);
        let right_w = col_w(PaneSlot::Right);
        // The Right column shares the surplus with Main rather than staying
        // at `max(min_w)` at every width (interface plan I8): the tactical
        // screens' right column carries whole table rows — a before/after
        // line is 36 columns for the reference frame — and a fixed 34 once
        // clipped the pane at 160 columns just as at 80. Since 16b cut B.3
        // `draw_pane` wraps every panel row at its pane's edge, so the share
        // no longer decides whether a row is read at all: it keeps a row on
        // ONE line where the terminal has the width. `visible_panes` is unchanged, so no
        // survivor set moves; only screens that DECLARE the split take it,
        // which keeps the rails (the WHO rail, the channel list) at the width
        // they were designed for.
        let right_w = if right_w > 0 && self.screen().right_shares_surplus {
            let main_min = panes
                .iter()
                .filter(|p| p.slot == PaneSlot::Main)
                .map(|p| p.min_w)
                .max()
                .unwrap_or(0);
            let surplus = area.width.saturating_sub(left_w + main_min + right_w);
            right_w + surplus / 2
        } else {
            right_w
        };
        let [left, main, right] = Layout::horizontal([
            Constraint::Length(left_w),
            Constraint::Min(1),
            Constraint::Length(right_w),
        ])
        .areas(area);
        for (slot, column) in [
            (PaneSlot::Left, left),
            (PaneSlot::Main, main),
            (PaneSlot::Right, right),
        ] {
            let in_slot: Vec<PaneSpec> = panes.iter().filter(|p| p.slot == slot).copied().collect();
            if in_slot.is_empty() || column.width == 0 {
                continue;
            }
            let shares: Vec<Constraint> = in_slot
                .iter()
                .map(|_| Constraint::Ratio(1, in_slot.len() as u32))
                .collect();
            let areas = Layout::vertical(shares).split(column);
            let mins: Vec<u16> = in_slot.iter().map(|p| p.min_h + 2).collect();
            let rects = honour_min_heights(&areas, &mins);
            for (spec, rect) in in_slot.iter().zip(rects.iter()) {
                self.draw_pane(frame, *spec, *rect);
            }
        }
    }

    fn pane_block(&self, spec: PaneSpec, focused: bool) -> Block<'static> {
        let color = match spec.role {
            BorderRole::Neutral => None,
            BorderRole::Unit => Some(Color::Indexed(6)),
            BorderRole::Ic => Some(Color::Indexed(2)),
            BorderRole::Ooc => Some(Color::Indexed(3)),
            BorderRole::Danger => Some(Color::Indexed(1)),
        };
        let mut style = Style::default();
        if let Some(c) = color
            && !matches!(self.caps.color, mush_proto::ColorDepth::None)
        {
            style = style.fg(c);
        }
        if focused {
            style = style.add_modifier(Modifier::BOLD);
        } else {
            style = style.add_modifier(Modifier::DIM);
        }
        // The backlog pane frames ONE channel's messages, and its title was
        // the literal word "channel" (look-back I7-C16) while the plan said
        // it named the channel. The name is server-supplied, so it is cut to
        // printable ASCII and bounded before it touches a border.
        let name = match spec.content {
            PaneContent::Backlog => self
                .selected_channel()
                .map(|c| ascii_title(&c))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| spec.title.to_string()),
            _ => spec.title.to_string(),
        };
        let title = if focused { format!(" {name} ") } else { name };
        Block::default()
            .borders(Borders::ALL)
            .border_style(style)
            .title(title)
    }

    fn draw_pane(&mut self, frame: &mut Frame<'_>, spec: PaneSpec, area: Rect) {
        let focused = self
            .screen()
            .panes
            .get(self.focus)
            .is_some_and(|p| p.id == spec.id);
        let block = self.pane_block(spec, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        match spec.content {
            PaneContent::Scrollback => self.draw_scrollback(frame, inner),
            PaneContent::Who => {
                let lines = if self.who.is_empty() {
                    vec![Line::from(dim_span_owned("(loading...)"))]
                } else {
                    render_who_rail(&self.who, &self.theme, self.caps.color, self.caps.utf8)
                };
                frame.render_widget(Paragraph::new(Text::from(lines)), inner);
            }
            _ => {
                // 16b cut Q.4: the one place a pane is told how big it is.
                let rows = self.pane_rows(&spec, Some((inner.width, inner.height)));
                let ui = self.pane_ui(spec.id);
                // 16b cut Q.4: the highlight follows its row's KEY, not the
                // index it was last at — the rows under it change as fast as
                // the arena pulses.
                let selected = self.anchored(spec.id, &rows);
                // **16b cut B.3** (`B-10`): every row wraps at the pane's edge
                // into as many lines as it takes — the rule `scrollback_rows`
                // has always used, through the same `wrap_spans` — so a narrow
                // pane costs rows and never words. One rule for every panel
                // pane on every screen. The map's rows are exactly the window's
                // width and never wrap. Each wrapped line remembers its SOURCE
                // row: the highlight lies on every line of the selected row, and
                // `anchored`, `selected_row` and the keys stay on source rows.
                let width = usize::from(inner.width);
                let mut lines: Vec<(usize, Line<'static>)> = Vec::new();
                // The first wrapped line of each source row (a prefix sum).
                let mut first_line: Vec<usize> = Vec::with_capacity(rows.rows.len());
                for (i, r) in rows.rows.into_iter().enumerate() {
                    first_line.push(lines.len());
                    let highlight = spec.selectable && i == selected && r.selectable;
                    let style = if highlight {
                        r.line.style.add_modifier(Modifier::REVERSED)
                    } else {
                        r.line.style
                    };
                    for spans in wrap_spans(&r.line.spans, width) {
                        lines.push((i, Line::from(spans).style(style)));
                    }
                }
                let total = lines.len();
                let h = usize::from(inner.height);
                let mut scroll = ui.scroll.min(total.saturating_sub(h));
                // The scroll follows the highlight over WRAPPED lines, anchored
                // on the selected row's first line: it is always in view, and
                // the rest of the row is brought in below it where it fits.
                if spec.selectable
                    && let Some(&anchor) = first_line.get(selected)
                {
                    let last = first_line
                        .get(selected + 1)
                        .map_or(total, |next| *next)
                        .saturating_sub(1)
                        .max(anchor);
                    if anchor < scroll {
                        scroll = anchor;
                    } else if last >= scroll + h {
                        scroll = (last + 1 - h).min(anchor);
                    }
                }
                let lines: Vec<Line<'static>> = lines.into_iter().map(|(_, line)| line).collect();
                frame.render_widget(
                    Paragraph::new(Text::from(lines))
                        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
                    inner,
                );
            }
        }
    }

    /// The scrollback rows for a pane `width` wide: each entry's body
    /// wrapped with a hanging indent under the gutter; the gutter is dropped
    /// on a pane too narrow to hold it.
    fn scrollback_rows(&self, width: usize) -> Vec<Line<'static>> {
        let gutter = width > GUTTER_W + 12;
        let body_w = if gutter { width - GUTTER_W - 1 } else { width };
        let mut rows = Vec::new();
        // `/clear` hides a prefix rather than dropping it (see `cleared`).
        for entry in self.scrollback.iter().skip(self.cleared) {
            let wrapped = wrap_spans(&entry.body, body_w);
            for (i, body) in wrapped.into_iter().enumerate() {
                let mut spans: Vec<Span<'static>> = Vec::new();
                if gutter {
                    let cell = match (&entry.gutter, i) {
                        (Some(g), 0) => {
                            let text: String = g.content.chars().take(GUTTER_W).collect();
                            Span::styled(format!("{text:>GUTTER_W$}"), g.style)
                        }
                        _ => Span::raw(" ".repeat(GUTTER_W)),
                    };
                    spans.push(cell);
                    spans.push(Span::raw(" "));
                }
                spans.extend(body);
                rows.push(Line::from(spans));
            }
        }
        rows
    }

    fn draw_scrollback(&mut self, frame: &mut Frame<'_>, inner: Rect) {
        let rows = self.scrollback_rows(usize::from(inner.width));
        let h = usize::from(inner.height);
        let max_scroll = rows.len().saturating_sub(h);
        // Remembered for the scroll-to-top trigger: a key handler cannot
        // measure the buffer (it does not know the pane's height), and the
        // draw path can never issue a query (it returns nothing).
        self.last_max_scroll = max_scroll;
        self.scroll_off = self.scroll_off.min(max_scroll);
        let start = max_scroll - self.scroll_off;
        let shown: Vec<Line<'static>> = rows.into_iter().skip(start).take(h).collect();
        frame.render_widget(Paragraph::new(Text::from(shown)), inner);
    }

    fn draw_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let input = self.mode == Mode::Input;
        let mut title = String::from("input");
        if self.sticky != StickyMode::None {
            title.push_str(&format!(" \u{2039}{}\u{203a}", self.sticky.label()));
        }
        if !input {
            title.push_str(" \u{2039}nav\u{203a} Enter to type");
        }
        if !self.caps.utf8 {
            title = title.replace('\u{2039}', "<").replace('\u{203a}', ">");
        }
        let border = if input {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(title);
        let inner = block.inner(area);
        let shown = self.shown_input();
        let prefix = format!("{}{}", self.prompt, shown);
        let text_style = if input {
            Style::default()
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        let para =
            Paragraph::new(Line::from(Span::styled(prefix.clone(), text_style))).block(block);
        frame.render_widget(para, area);
        if input && inner.width > 0 && inner.height > 0 {
            let shown_cursor = match self.input_mode {
                InputMode::NoEcho => 0,
                _ => self.line.cursor(),
            };
            let used = (self.prompt.chars().count() + shown_cursor)
                .min(usize::from(inner.width.saturating_sub(1)));
            frame.set_cursor_position(Position {
                x: inner.x + u16::try_from(used).unwrap_or(0),
                y: inner.y,
            });
        }
    }

    fn draw_palette(&self, frame: &mut Frame<'_>, body: Rect) {
        let Some(p) = &self.palette else {
            return;
        };
        let rows = p.entries.len().clamp(1, MAX_PALETTE_ROWS);
        let h = u16::try_from(rows + 2).unwrap_or(u16::MAX).min(body.height);
        if h < 3 || body.width < 10 {
            return;
        }
        let w = body.width.min(60);
        let area = Rect {
            x: body.x,
            y: body.y + body.height - h,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, area);
        let first = p.selected.saturating_sub(rows - 1);
        let lines: Vec<Line<'static>> = if p.entries.is_empty() {
            vec![Line::from(dim_span_owned("(no matches)"))]
        } else {
            p.entries
                .iter()
                .enumerate()
                .skip(first)
                .take(rows)
                .map(|(i, e)| {
                    let text = format!("{:<20} {}", sanitize(&e.label), sanitize(&e.detail));
                    let line = Line::from(Span::raw(text));
                    if i == p.selected {
                        line.style(Style::default().add_modifier(Modifier::REVERSED))
                    } else {
                        line
                    }
                })
                .collect()
        };
        let block = Block::default().borders(Borders::ALL).title("palette");
        frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    /// The server-initiated request overlay (interface plan I10): the prompt,
    /// the options (a confirm as `y`/`n`, a choice numbered), and the keys.
    fn draw_request(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(req) = self.requests.front() else {
            return;
        };
        let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
            sanitize(&req.prompt.to_plain_text()),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        lines.push(Line::from(""));
        match req.kind {
            RequestKind::Confirm => {
                lines.push(Line::from("y  yes      n  no"));
            }
            RequestKind::Choice => {
                for (i, o) in req.options.iter().enumerate().take(9) {
                    lines.push(Line::from(format!("{}  {}", i + 1, sanitize(&o.label))));
                }
            }
            RequestKind::Target | RequestKind::Text => {
                lines.push(Line::from(dim_span_owned("Enter to type your answer")));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(dim_span_owned(
            "Esc cancels (the game decides for you)",
        )));
        let h = u16::try_from(lines.len() + 2)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let w = area.width.min(64);
        if h < 3 || w < 10 {
            return;
        }
        let rect = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, rect);
        // With more than one ask open, say so: the answer goes to THIS one,
        // and the next takes its place when it does.
        let title = match self.requests.len() {
            0 | 1 => "the game asks".to_string(),
            n => format!("the game asks (1 of {n})"),
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        frame.render_widget(Paragraph::new(Text::from(lines)).block(block), rect);
    }

    fn draw_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines: Vec<Line<'static>> = vec![
            Line::from(Span::styled(
                format!("{} screen", self.screen().name),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("0-9 screens   Tab focus   Up/Down PgUp/PgDn scroll   g/G top/bottom"),
            Line::from("g (or /history) at the top loads older scene history from the server"),
            Line::from("/clear hides what is on screen; @notui then @tui starts a fresh view"),
            Line::from(
                "Enter type   \" : ; \\ speak   / palette   ^K palette   ^P mode   ^] leave",
            ),
            Line::from("In the box: Alt+key = the nav key   Esc keeps the draft   Enter stays"),
            Line::from(""),
        ];
        for b in &self.screen().keys {
            lines.push(Line::from(format!("{}  {}", b.key, b.label)));
        }
        if self.screen().exits_are_keys && !self.exit_keys.is_empty() {
            let exits: Vec<String> = self
                .exit_keys
                .iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect();
            lines.push(Line::from(format!("exits: {}", exits.join("  "))));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(dim_span_owned("Esc closes")));
        let h = u16::try_from(lines.len() + 2)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let w = area.width.min(76);
        if h < 3 || w < 10 {
            return;
        }
        let rect = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, rect);
        let block = Block::default().borders(Borders::ALL).title("help");
        frame.render_widget(Paragraph::new(Text::from(lines)).block(block), rect);
    }

    /// The input as it should appear, honoring the current [`InputMode`].
    fn shown_input(&self) -> String {
        let text = self.line.text();
        match self.input_mode {
            InputMode::Normal => text,
            InputMode::Masked => "*".repeat(text.chars().count()),
            InputMode::NoEcho => String::new(),
        }
    }
}

/// **A column's pane rects with every pane at least its own `min_h`** (16b
/// cut B.3), the equal shares `draw_body` splits a column into, repaired.
///
/// `visible_panes` keeps a pane only when its column has the rows for every
/// survivor's `min_h` plus its border, so those rows exist; an equal split
/// could still hand one pane less than it declared while a neighbour had rows
/// to spare. The cockpit's status pane is the case: ten rows, the LOCK the
/// last of them, and at 80 by 24 an even split with the notices pane gave it
/// seven, so the one row a gunner reads before `FIRE` was the one cut. Where
/// every pane already has its minimum the equal split stands unchanged (every
/// other screen's golden is byte-identical); where one is short, rows move to it
/// one at a time from the pane with the most to spare (the later of a tie),
/// and the rects are re-stacked top to bottom in the column.
fn honour_min_heights(areas: &[Rect], mins: &[u16]) -> Vec<Rect> {
    let mut heights: Vec<u16> = areas.iter().map(|r| r.height).collect();
    let short = |heights: &[u16]| {
        heights
            .iter()
            .zip(mins)
            .position(|(have, want)| have < want)
    };
    if short(&heights).is_none() {
        return areas.to_vec();
    }
    while let Some(needy) = short(&heights) {
        let donor = heights
            .iter()
            .zip(mins)
            .enumerate()
            .filter(|(_, (have, want))| have > want)
            .max_by_key(|(i, (have, want))| (**have - **want, *i))
            .map(|(i, _)| i);
        let Some(donor) = donor else {
            // No pane has a row to spare: `visible_panes` should have made
            // this impossible, and the equal split is the honest fallback.
            return areas.to_vec();
        };
        if let Some(h) = heights.get_mut(donor) {
            *h -= 1;
        }
        if let Some(h) = heights.get_mut(needy) {
            *h += 1;
        }
    }
    let mut y = areas.first().map_or(0, |r| r.y);
    areas
        .iter()
        .zip(heights)
        .map(|(r, height)| {
            let rect = Rect { y, height, ..*r };
            y = y.saturating_add(height);
            rect
        })
        .collect()
}

fn loading() -> PaneRows {
    PaneRows {
        rows: vec![super::render::PaneRow {
            line: Line::from(dim_span_owned("(loading...)")),
            prefill: None,
            row_key: None,
            selectable: false,
        }],
    }
}

/// Text collapsed to a single line (control chars / newlines out).
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// The most characters a server-supplied pane title may spend on a border.
const MAX_PANE_TITLE: usize = 16;

/// A server-supplied string safe to draw INTO a pane border: printable ASCII
/// only (a border is box-drawing, and a wide or combining char there
/// mis-measures the frame) and bounded by [`MAX_PANE_TITLE`].
fn ascii_title(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(MAX_PANE_TITLE)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The one-letter exit keys a room offers (D-I-19): every exit whose name
/// or an alias is exactly one ASCII letter binds that letter, unless the
/// letter is a global or jump key; first exit wins a contested letter.
#[must_use]
pub fn exit_keys(exits: &[ExitRef]) -> Vec<(char, String)> {
    let mut keys: Vec<(char, String)> = Vec::new();
    for exit in exits {
        let candidates =
            std::iter::once(exit.name.as_str()).chain(exit.aliases.iter().map(String::as_str));
        for cand in candidates {
            let mut chars = cand.trim().chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                continue;
            };
            if !c.is_ascii_alphabetic() {
                continue;
            }
            let c = c.to_ascii_lowercase();
            if GLOBAL_KEYS.contains(&c) || JUMP_KEYS.contains(&c) {
                continue;
            }
            if keys.iter().any(|(k, _)| *k == c) {
                continue;
            }
            keys.push((c, exit.name.clone()));
            break;
        }
    }
    keys
}

// Look-back LB-24: `reserved_exit_keys()` and `plain_of()` were `pub` with
// no caller anywhere — `pub mod tui` hides that from `dead_code`, so nothing
// could see it. `RESERVED_EXIT_KEYS` is already `pub` in `screen`, and
// `rich_plain` is used directly by the renderers.

/// Whether `feed` is one of the manifest's feeds.
#[must_use]
pub fn manifest_has(manifest: &TuiManifest, feed: &str) -> bool {
    manifest.panels.iter().any(|f| f == feed)
}

/// The read-only query that hydrates a panel FEED, mirroring the web bridge's
/// subscribe map (`mush-gateway/src/web_bridge.rs`): a feed is normally
/// answered by a handler of the same name, and these are the ones that are
/// not (look-back LB-10/LB-11 — three copies of this map exist; the long fix
/// is `Registrar::panel_feed` declaring its own keyframe query).
#[must_use]
pub fn feed_query(feed: &str) -> &str {
    match feed {
        "comsys" => "comsys.state",
        "intel" => "intel.list",
        "mail" => "mail.list",
        other => other,
    }
}

/// The panel feed a pane reads, where it has one. The two channel panes and
/// the backlog beside them all read `comsys` (`pane_rows`), which is what the
/// manifest names — `comsys.state` is only the query.
#[must_use]
fn pane_feed(content: PaneContent) -> Option<&'static str> {
    match content {
        PaneContent::Panel { feed, .. } => Some(feed),
        PaneContent::Channels { .. } | PaneContent::Backlog => Some("comsys"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use mush_proto::{
        ClientKind, ColorDepth, NarrativeKind, NoticeAction, NoticeSeverity, ObjRef, PromptSpec,
        RequestKind, RequestOption, RespondChoice, RichText, SmolStr,
    };

    fn caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi16,
            width: 120,
            height: 34,
            utf8: true,
            structured: false,
        }
    }

    fn app() -> TuiApp {
        let mut app = TuiApp::new(caps());
        app.set_clock(|| 3600 * 14 + 60 * 30);
        app
    }

    fn room_with_exits() -> OutputEvent {
        OutputEvent::Room(RoomView {
            name: "Hall".into(),
            id: "#2".into(),
            desc: RichText::plain("A hall."),
            contents: vec![],
            exits: vec![
                ExitRef {
                    name: "North".into(),
                    aliases: vec!["n".into(), "north".into()],
                },
                ExitRef {
                    name: "quai".into(),
                    aliases: vec![],
                },
                ExitRef {
                    name: "?".into(),
                    aliases: vec!["3".into()],
                },
            ],
            zone: None,
            ambient: Default::default(),
        })
    }

    #[test]
    fn digits_switch_screens_in_nav_and_alt_digits_in_both_modes() {
        let mut app = app();
        let r = app.feed_input(b"3");
        assert_eq!(app.active_screen(), ScreenId(3));
        assert!(r.queries.iter().any(|(n, _)| n == "mail.list"));
        app.feed_input(b"\r");
        assert_eq!(app.mode(), Mode::Input);
        app.feed_input(b"6");
        assert_eq!(
            app.active_screen(),
            ScreenId(3),
            "a digit in the box inserts"
        );
        assert_eq!(app.input_text(), "6");
        app.feed_input(b"\x1b1");
        assert_eq!(
            app.active_screen(),
            ScreenId(1),
            "Alt+1 switches from the box"
        );
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "6", "Alt+1 inserted nothing");
        app.feed_input(b"\x1bOQ");
        assert_eq!(
            app.active_screen(),
            ScreenId(2),
            "F2 switches in input mode"
        );
        app.feed_input(b"\x1b[15~");
        assert_eq!(app.active_screen(), ScreenId(5));
        app.feed_input(b"9");
        assert_eq!(app.active_screen(), ScreenId(5), "no screen 9: unchanged");
    }

    #[test]
    fn a_split_alt_burst_leaves_input_mode_keeps_the_draft_then_switches() {
        let mut app = app();
        app.feed_input(b"\rlook at ");
        assert_eq!(app.mode(), Mode::Input);
        app.feed_input(b"\x1b");
        assert_eq!(app.mode(), Mode::Nav);
        assert_eq!(app.draft(), Some("look at "));
        app.feed_input(b"3");
        assert_eq!(app.active_screen(), ScreenId(3));
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "look at ", "the draft came back");
    }

    #[test]
    fn alt_letter_in_input_mode_performs_the_nav_binding_and_inserts_nothing() {
        let mut app = app();
        app.feed_input(b"3\r");
        assert_eq!(app.mode(), Mode::Input);
        let r = app.feed_input(b"\x1bs");
        assert_eq!(app.input_text(), "@mail/send ", "Alt+s prefilled send");
        assert!(r.submit.is_empty());
        app.feed_input(b"\x1b");
        // Enter restores the kept draft; ^U clears it for a fresh line.
        app.feed_input(b"1\r\x15");
        app.feed_input(b"hi");
        let r = app.feed_input(b"\x1bl");
        assert_eq!(r.submit, vec!["look".to_string()], "Alt+l ran look");
        assert_eq!(app.input_text(), "hi", "nothing inserted");
    }

    #[test]
    fn shift_tab_moves_focus_back_in_nav_and_is_inert_in_input() {
        let mut app = app();
        assert_eq!(app.focused_pane(), Some(PaneId("scrollback")));
        app.feed_input(b"\t");
        assert_eq!(app.focused_pane(), Some(PaneId("who")));
        app.feed_input(b"\x1b[Z");
        assert_eq!(app.focused_pane(), Some(PaneId("scrollback")));
        app.feed_input(b"\x1b[Z");
        assert_eq!(app.focused_pane(), Some(PaneId("actions")), "wraps");
        app.feed_input(b"\r");
        app.feed_input(b"\x1b[Z");
        assert_eq!(app.focused_pane(), Some(PaneId("actions")));
        assert_eq!(app.input_text(), "", "never mistaken for Alt+[");
        assert_eq!(app.mode(), Mode::Input);
    }

    #[test]
    fn enter_enters_input_empty_or_prefilled_from_a_focused_row() {
        let mut app = app();
        app.feed_input(b"\r");
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "");
        app.feed_input(b"\r");
        assert_eq!(
            app.mode(),
            Mode::Nav,
            "Enter on an empty box returns to nav"
        );
        app.feed_input(b"3");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("mail"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "folder": "inbox", "entries": [
                { "index": 1, "status": "unread", "from": { "name": "Zoe" }, "subject": "Hi" },
                { "index": 2, "status": "read", "from": { "name": "Al" }, "subject": "Later" },
            ]}),
        });
        app.feed_input(b"\x1b[B");
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "@mail 2");
        app.feed_input(b"\x1b");
        let r = app.feed_input(b"d");
        assert_eq!(app.input_text(), "@mail/delete 2");
        assert!(r.submit.is_empty(), "a prefill sends nothing");
    }

    #[test]
    fn enter_on_a_typed_line_submits_and_stays_in_input_mode() {
        let mut app = app();
        app.feed_input(b"\r");
        let r = app.feed_input(b"say hi\r");
        assert_eq!(r.submit, vec!["say hi".to_string()]);
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "");
        let r = app.feed_input(b"one\r\ntwo\r\n");
        assert_eq!(r.submit, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn esc_keeps_the_draft_and_the_next_enter_restores_it() {
        let mut app = app();
        app.feed_input(b"\rpose waves\x1b");
        assert_eq!(app.mode(), Mode::Nav);
        assert_eq!(app.draft(), Some("pose waves"));
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "pose waves");
        assert_eq!(app.draft(), None);
    }

    #[test]
    fn triggers_enter_input_with_the_prefix_and_slash_opens_the_palette() {
        let mut app = app();
        app.feed_input(b"\"");
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "\"");
        app.feed_input(b"\x1b");
        app.feed_input(b":");
        assert_eq!(app.input_text(), ":");
        app.feed_input(b"\x1b");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("commands.list"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "commands": [
                { "name": "look", "usage": "look [<thing>]", "summary": "See." },
                { "name": "@mail", "usage": "@mail [<n>]", "summary": "Mail." },
            ]}),
        });
        app.feed_input(b"/");
        assert!(app.palette_open());
        assert_eq!(app.input_text(), "/");
        app.feed_input(b"lo");
        let entries = app.palette_entries();
        assert_eq!(entries.first().map(String::as_str), Some("look [<thing>]"));
        app.feed_input(b"\r");
        assert!(!app.palette_open());
        assert_eq!(
            app.input_text(),
            "look [<thing>]",
            "a game command loses the slash"
        );
        app.feed_input(b"\x15/cle\r");
        assert_eq!(
            app.input_text(),
            "/clear",
            "a client-local command keeps it"
        );
        app.feed_input(b"\x15/xyz");
        app.feed_input(b"\x1b");
        assert!(!app.palette_open());
        assert_eq!(
            app.input_text(),
            "/xyz",
            "Esc closes the palette and keeps the text"
        );
        assert_eq!(app.mode(), Mode::Input);
    }

    #[test]
    fn unbound_keys_do_nothing_and_bound_letters_insert_in_input_mode() {
        let mut app = app();
        let before = super::super::frame_text(&mut app, 80, 24);
        let r = app.feed_input(b"z");
        assert!(!r.redraw);
        assert_eq!(super::super::frame_text(&mut app, 80, 24), before);
        app.feed_input(b"\r");
        let r = app.feed_input(b"l");
        assert!(r.submit.is_empty());
        assert_eq!(app.input_text(), "l");
    }

    #[test]
    fn exits_are_keys_on_the_scene_screen_only() {
        let mut app = app();
        let r = app.feed_input(b"n");
        assert!(r.submit.is_empty(), "no room, no exit");
        app.apply_event(&room_with_exits());
        assert_eq!(app.exit_keys(), &[('n', "North".to_string())]);
        let r = app.feed_input(b"n");
        assert_eq!(r.submit, vec!["North".to_string()]);
        let r = app.feed_input(b"3");
        assert_eq!(
            app.active_screen(),
            ScreenId(3),
            "an exit aliased 3 never steals the digit"
        );
        assert!(r.submit.is_empty());
        let r = app.feed_input(b"n");
        assert!(r.submit.is_empty(), "inert on the mail screen");
        app.feed_input(b"1");
        let r = app.feed_input(b"n");
        assert_eq!(
            r.submit,
            vec!["North".to_string()],
            "back on the scene screen"
        );
        app.feed_input(b"\r");
        let r = app.feed_input(b"\x1bn");
        assert_eq!(
            r.submit,
            vec!["North".to_string()],
            "Alt+n walks from the box"
        );
        // A room without a one-letter exit binds nothing.
        app.apply_event(&OutputEvent::Room(RoomView {
            name: "Bar".into(),
            id: "#3".into(),
            desc: RichText::plain(""),
            contents: vec![],
            exits: vec![ExitRef {
                name: "tabac".into(),
                aliases: vec![],
            }],
            zone: None,
            ambient: Default::default(),
        }));
        assert!(app.exit_keys().is_empty());
        let r = app.feed_input(b"\x1bt");
        assert!(r.submit.is_empty());
    }

    #[test]
    fn mode_resets_on_resume_and_masked_prompts_open_the_box() {
        let mut app = app();
        app.feed_input(b"\rdraft");
        app.apply_event(&OutputEvent::Resumed { replayed: true });
        assert_eq!(app.mode(), Mode::Nav);
        assert_eq!(app.draft(), Some("draft"));
        app.apply_event(&OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Masked,
            text: Some(RichText::plain("pw>")),
        }));
        assert_eq!(app.mode(), Mode::Input);
        app.feed_input(b"secret");
        assert_eq!(app.shown_input(), "******");
        assert_eq!(app.prompt(), "pw>");
        let r = app.feed_input(b"\r");
        assert_eq!(r.submit, vec!["secret".to_string()]);
    }

    #[test]
    fn sticky_mode_prefixes_and_explicit_prefixes_win() {
        let mut app = app();
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": [ { "name": "Public", "alias": "pub", "unread": 0 } ] }),
        });
        app.feed_input(&[0x10]);
        assert_eq!(app.sticky(), StickyMode::Say);
        app.feed_input(b"\r");
        let r = app.feed_input(b"hello\r:waves\rpub hi\r\"quoted\r");
        assert_eq!(
            r.submit,
            vec![
                "say hello".to_string(),
                ":waves".to_string(),
                "pub hi".to_string(),
                "\"quoted".to_string()
            ]
        );
        app.feed_input(&[0x10]);
        app.feed_input(&[0x10]);
        assert_eq!(app.sticky(), StickyMode::Emit);
        let r = app.feed_input(b"the rain\r");
        assert_eq!(r.submit, vec!["@emit the rain".to_string()]);
    }

    #[test]
    fn local_commands_and_detach() {
        let mut app = app();
        app.apply_event(&OutputEvent::Sys(RichText::plain("x")));
        app.feed_input(b"\r");
        let r = app.feed_input(b"/clear\r");
        assert!(r.submit.is_empty());
        // A view floor, not a data clear (I12): the entry stays in the ring
        // so the history cursor can stay where it is.
        assert!(rows_of(&app).is_empty());
        assert_eq!(app.scrollback_len(), 1);
        let r = app.feed_input(b"/notui\r");
        assert!(r.detach);
        let r = app.feed_input(&[0x1d]);
        assert!(r.detach);
        let r = app.feed_input(b"/bogus\r");
        assert!(r.submit.is_empty());
        assert_eq!(
            rows_of(&app).len(),
            1,
            "an unknown local command explains itself"
        );
    }

    #[test]
    fn help_overlay_toggles_and_legend_follows_the_mode() {
        let mut app = app();
        app.feed_input(b"?");
        assert!(app.help_open());
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("scene screen"));
        app.feed_input(b"\x1b");
        assert!(!app.help_open());
        let nav = super::super::frame_text(&mut app, 120, 34);
        assert!(nav.contains("Enter type"));
        assert!(nav.contains("\u{2039}nav\u{203a}"));
        app.feed_input(b"\r");
        let input = super::super::frame_text(&mut app, 120, 34);
        assert!(input.contains("Esc nav"));
        assert!(!input.contains("Enter type"));
        app.feed_input(b"\x1b?");
        assert!(app.help_open(), "Alt+? from the box");
    }

    #[test]
    fn history_and_cursor_edits_match_the_line_editor() {
        use crate::{EditorAction, LineEditor};
        let sequences: [&[u8]; 4] = [
            b"helo\x1b[D\x1b[Dl\x01>\x05<",
            b"say hello there\x17\x15kept",
            b"first\rsecond\r\x1b[A\x1b[A\x1b[B",
            b"abc\x1b[H\x1b[3~\x1bOD\x7f",
        ];
        for seq in sequences {
            let mut ed = LineEditor::new();
            let mut out = Vec::new();
            let mut ed_submits = Vec::new();
            for a in ed.feed(seq, &mut out) {
                if let EditorAction::Submit(s) = a {
                    ed_submits.push(s);
                }
            }
            let mut app = app();
            app.feed_input(b"\r");
            let r = app.feed_input(seq);
            assert_eq!(r.submit, ed_submits, "{seq:?}");
            assert_eq!(app.input_text(), ed.line(), "{seq:?}");
            assert_eq!(app.input_cursor(), ed.cursor(), "{seq:?}");
        }
    }

    #[test]
    fn gutter_lines_wrap_with_a_hanging_indent_and_never_bleed_left() {
        let mut app = app();
        app.apply_event(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Mireille")),
            text: RichText::plain(
                "the quick brown fox jumps over the lazy dog and keeps running well past the edge of any pane",
            ),
        });
        let rows = app.scrollback_rows(40);
        assert!(rows.len() >= 3);
        let texts: Vec<String> = rows
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(texts[0].starts_with("  Mireille "), "{:?}", texts[0]);
        for t in &texts[1..] {
            assert!(t.starts_with(&" ".repeat(GUTTER_W + 1)), "{t:?}");
            assert!(t.chars().count() <= 40);
        }
        // A narrow pane drops the gutter rather than starving the body.
        let rows = app.scrollback_rows(15);
        let first: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first.starts_with("\"the"), "{first:?}");
    }

    #[test]
    fn vitals_strip_shows_and_prompt_shows_without_it() {
        let mut app = app();
        // I2-C40: the textual prompt is the fallback for a server with no
        // vitals feed at all, and nothing asserted it showed. It shows with
        // the strip and without it.
        app.apply_event(&OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain("HP:20>")),
        }));
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains("[ Health"));
        assert!(
            text.contains("HP:20>"),
            "no vitals, so the prompt is it:\n{text}"
        );
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("wod.vitals"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "subject": { "id": "#5", "name": "Nick" }, "revision": 1,
                "vitals": [
                    { "id": "wod.willpower", "label": "WP", "role": "pool",
                      "value": { "kind": "bounded", "current": 6, "min": 0, "max": 10 }, "state": "ok" },
                    { "id": "wod.health", "label": "Health", "role": "dominant",
                      "value": { "kind": "track", "boxes": [null, null, null], "level": null }, "state": "ok" },
                    { "id": "wod.wound", "label": "wound", "role": "readout",
                      "value": { "kind": "text", "text": "0" }, "state": "ok" }
                ]
            }),
        });
        let text = super::super::frame_text(&mut app, 120, 34);
        let strip = text
            .lines()
            .find(|l| l.contains("[ Health"))
            .expect("a strip");
        assert!(
            strip.find("[ Health").unwrap() < strip.find("[ WP").unwrap(),
            "dominant first"
        );
        assert!(strip.contains("[ wound 0 ]"));
        assert!(
            text.contains("HP:20>"),
            "the prompt keeps its place beside the strip:\n{text}"
        );
    }

    /// Look-back I2-C32: a masked prompt (a password, a chargen question)
    /// opens a FRESH box from either mode. It used to fire only from nav, so
    /// a half-typed game line already in the box rode into the password field
    /// and one Enter sent both. The line is kept as the draft and comes back
    /// when input mode next opens empty.
    #[test]
    fn a_masked_prompt_never_inherits_a_half_typed_game_line() {
        let mut app = app();
        app.feed_input(b"\rattack the guard");
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "attack the guard");
        app.apply_event(&OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Masked,
            text: Some(RichText::plain("Password:")),
        }));
        assert_eq!(app.mode(), Mode::Input, "the box stays open for typing");
        assert_eq!(app.input_text(), "", "the password field starts empty");
        assert_eq!(app.draft(), Some("attack the guard"));
        let r = app.feed_input(b"secret\r");
        assert_eq!(r.submit, vec!["secret".to_string()], "the password alone");
        // Back in normal mode the line the player was writing is offered again.
        app.apply_event(&OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Normal,
            text: None,
        }));
        app.feed_input(&[0x1b]);
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "attack the guard");
    }

    #[test]
    fn deltas_resync_once_and_unknown_feeds_never_paint() {
        let mut app = app();
        let q = app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("intel"),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({ "records": [] }),
        });
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].0, "intel");
        let q = app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("intel"),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({ "records": [] }),
        });
        assert!(q.is_empty(), "exactly one resync");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("mystery.feed"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "secret": "never-shown-\u{7}text" }),
        });
        assert!(app.panels().get("mystery.feed").is_some());
        // 16b cut Q.4 made `0` a screen: every one of the ten, not nine.
        for screen in 0..=9u8 {
            app.feed_input(&[b'0' + screen]);
            let text = super::super::frame_text(&mut app, 120, 34);
            assert!(!text.contains("never-shown"), "screen {screen}");
        }
    }

    #[test]
    fn control_characters_in_panel_data_are_stripped() {
        let mut app = app();
        app.feed_input(b"3");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("mail"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "folder": "in\u{1b}[2Jbox", "entries": [
                { "index": 1, "status": "unread", "from": { "name": "Z\u{7}oe" }, "subject": "H\ni" },
            ]}),
        });
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'));
        assert!(text.contains("Zoe"));
        // The ESC is gone; the printable remainder of its sequence is text.
        assert!(text.contains("in[2Jbox"), "{text}");
    }

    #[test]
    fn tab_badges_follow_mail_notices_and_channel_unread() {
        let mut app = app();
        app.apply_event(&OutputEvent::MailNotice {
            from: ObjRef::new("#9", "Zoe"),
            subject: "hi".into(),
            id: 4,
            unread: 3,
        });
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": [
                { "name": "Public", "alias": "pub", "unread": 2 },
                { "name": "Staff", "alias": "st", "unread": 5 },
            ] }),
        });
        let text = super::super::frame_text(&mut app, 120, 34);
        let tabs = text.lines().next().unwrap();
        assert!(tabs.contains("3 mail 3"), "{tabs:?}");
        assert!(tabs.contains("4 chan 7"), "{tabs:?}");
    }

    fn page_notice() -> OutputEvent {
        OutputEvent::Notice {
            id: 7,
            severity: NoticeSeverity::Urgent,
            category: SmolStr::new_static("page"),
            text: RichText::plain("Razor: meet me at the docks"),
            action: Some(NoticeAction {
                label: "Reply".into(),
                prefill: "page Razor=".into(),
            }),
            ts: 0,
        }
    }

    #[test]
    fn a_notice_lands_in_the_scrollback_with_its_category_and_action_hint() {
        let mut app = app();
        app.apply_event(&page_notice());
        let rows = app.scrollback_rows(70);
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("[page]"), "{joined:?}");
        assert!(joined.contains("meet me at the docks"), "{joined:?}");
        assert!(joined.contains("(page Razor=)"), "{joined:?}");
    }

    #[test]
    fn unseen_notices_show_on_the_status_line_and_clear_when_the_tail_returns() {
        let mut app = app();
        app.apply_event(&page_notice());
        assert_eq!(app.notice_unseen, 1);
        // On a screen without the scrollback pane (sheet, digit 2) the count
        // stands, and the status line names it.
        app.feed_input(b"2");
        let away = super::super::frame_text(&mut app, 120, 34);
        assert!(away.contains("1 new"), "{away}");
        assert_eq!(app.notice_unseen, 1);
        // Back on the scene tail, drawing marks them seen.
        app.feed_input(b"1");
        let home = super::super::frame_text(&mut app, 120, 34);
        assert!(!home.contains("1 new"), "{home}");
        assert_eq!(app.notice_unseen, 0);
    }

    #[test]
    fn every_screen_paints_at_every_size_without_panic_and_degrades_in_order() {
        let mut app = app();
        app.apply_event(&room_with_exits());
        app.apply_event(&OutputEvent::Who(vec![WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 1,
            conn_s: 1,
            location: None,
            here: true,
            doing: None,
        }]));
        // 16b cut Q.4 made `0` a screen: every one of the ten, not nine.
        for screen in 0..=9u8 {
            app.feed_input(&[b'0' + screen]);
            for (w, h) in [
                (1u16, 1u16),
                (2, 1),
                (80, 1),
                (1, 24),
                (10, 3),
                (80, 24),
                (120, 34),
                (160, 44),
            ] {
                let _ = super::super::frame_text(&mut app, w, h);
            }
        }
        app.feed_input(b"1");
        let wide = super::super::frame_text(&mut app, 160, 44);
        assert!(wide.contains(" WHO ") || wide.contains("WHO"));
        assert!(wide.contains("actions"));
        let budget = super::super::frame_text(&mut app, 120, 34);
        assert!(budget.contains("WHO"));
        let narrow = super::super::frame_text(&mut app, 80, 24);
        assert!(!narrow.contains("WHO"), "the rail drops below 100 columns");
        assert!(narrow.contains("input"));
        let short = super::super::frame_text(&mut app, 80, 8);
        assert!(!short.contains("Enter type"), "no chrome below 12 rows");
        assert!(short.contains("input"));
    }

    /// I8: the unit screen's four declared placeholders say what they wait
    /// on rather than sitting on `(loading...)` forever, and they are the
    /// first four panes to go when the terminal narrows — a live pane never
    /// loses its place to one that has no producer yet.
    #[test]
    fn placeholders_render_their_declared_text_and_degrade_before_every_live_pane() {
        let mut app = app();
        app.feed_input(b"9");
        assert_eq!(app.active_screen(), ScreenId(9));

        // The body area of a 120x34 terminal (tabs, status, input, legend).
        let survivors = |app: &TuiApp, w: u16, h: u16| -> Vec<&'static str> {
            app.visible_panes(Rect::new(0, 0, w, h))
                .iter()
                .map(|p| p.id.0)
                .collect()
        };
        assert_eq!(
            survivors(&app, 120, 28),
            vec![
                "sections",
                "components",
                "plant",
                "grid",
                "contacts",
                "weapons",
                "comms"
            ],
            "all seven at 120x34"
        );
        assert_eq!(
            survivors(&app, 80, 18),
            vec!["sections", "components", "plant"],
            "at 80x24 the placeholders and contacts go, the live panes stay"
        );
        // Narrowing further keeps dropping in the declared order.
        assert_eq!(survivors(&app, 60, 18), vec!["sections", "components"]);
        assert_eq!(
            survivors(&app, 30, 18),
            vec!["sections"],
            "narrower than the main pane's minimum: the first pane alone"
        );

        // The damage screen keeps all three panes down to eighty columns.
        app.feed_input(b"8");
        assert_eq!(
            survivors(&app, 80, 18),
            vec!["sections", "delta", "journal"]
        );
        assert_eq!(survivors(&app, 60, 18), vec!["sections"]);

        // The declared text paints, and no unit pane ever says "loading":
        // a placeholder shows its two-row declaration, the contacts pane on
        // a server whose manifest lacks the feed says so, and the unit panes
        // read the no-unit line.
        app.set_manifest(TuiManifest {
            panels: vec!["tactics.unit".into()],
            character: None,
            room: None,
        });
        app.feed_input(b"9");
        let text = super::super::frame_text(&mut app, 120, 34);
        // (S4 produced the grid pane: the `atlas.map` frame, which on a
        // server whose manifest lacks the feed reads its own no-map line.)
        assert_eq!(
            text.matches("projection (plan").count(),
            2,
            "one declaration per placeholder:\n{text}"
        );
        assert!(text.contains("grid"), "the grid pane stands:\n{text}");
        for row in ["awaiting a weapons-cycle", "awaiting a station comms"] {
            assert!(text.contains(row), "{row} missing:\n{text}");
        }
        for title in ["grid", "weapons", "comms", "contacts"] {
            assert!(text.contains(title), "{title} missing:\n{text}");
        }
        assert!(
            text.contains("(not published by this server)"),
            "a feed the manifest lacks is declared empty, never loading:\n{text}"
        );
        assert!(
            text.contains("No tactical unit"),
            "no keyframe held is the no-unit line, never a spinner:\n{text}"
        );
        assert!(!text.contains("(loading...)"), "{text}");
    }

    /// I8 (D-I8-1): the contacts pane's feed is asked for on entry only when
    /// the manifest carries it (a server without the propagation layer never
    /// prints an unknown-query line), `r` replays it, the damage screen never
    /// asks for it, and the table renders through the generic-view renderer
    /// with the row's `+contact` prefill ready on Enter.
    #[test]
    fn the_unit_screen_subscribes_the_contacts_feed_only_when_the_manifest_carries_it() {
        let names =
            |r: &InputResult| -> Vec<String> { r.queries.iter().map(|(n, _)| n.clone()).collect() };
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec!["tactics.unit".into()],
            character: None,
            room: None,
        });
        app.feed_input(b"1");
        assert_eq!(names(&app.feed_input(b"9")), vec!["tactics.unit"]);
        app.set_manifest(TuiManifest {
            panels: vec!["tactics.unit".into(), "tactics.contacts".into()],
            character: None,
            room: None,
        });
        app.feed_input(b"1");
        let r = app.feed_input(b"9");
        assert_eq!(names(&r), vec!["tactics.unit", "tactics.contacts"]);
        let refresh = app.feed_input(b"r");
        assert_eq!(refresh.queries, r.queries, "r replays both");
        assert_eq!(names(&app.feed_input(b"8")), vec!["tactics.unit"]);
        app.apply_event(&OutputEvent::Panel {
            feed: "tactics.contacts".into(),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "schema": { "id": "tactics.contacts", "version": 1, "shape": "list",
                    "columns": [ { "key": "id", "label": "contact", "align": "left" },
                                 { "key": "state", "label": "state", "align": "left" } ],
                    "fields": [], "actionKey": "prefill" },
                "rows": [ { "id": "aabbccddeeff", "state": "held", "prefill": "+contact aabbccddeeff" } ],
                "truncated": false
            }),
        });
        app.feed_input(b"9");
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("aabbccddeeff"), "{text}");
        assert!(!text.contains("(loading...)"), "{text}");
        // Tab to the contacts pane (the fifth), Enter prefills the row's line.
        for _ in 0..4 {
            app.feed_input(b"\t");
        }
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "+contact aabbccddeeff");
    }

    /// I8: both tactical screens hydrate the one feed they read, with the
    /// empty params that mean "the viewer's own subject"; `r` replays it.
    #[test]
    fn the_tactical_screens_ask_for_the_unit_feed_on_entry_and_on_refresh() {
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec!["tactics.unit".into()],
            character: None,
            room: None,
        });
        for digit in *b"89" {
            app.feed_input(b"1");
            let r = app.feed_input(&[digit]);
            assert_eq!(
                r.queries,
                vec![(
                    "tactics.unit".to_string(),
                    Value::Object(serde_json::Map::new())
                )]
            );
            let refresh = app.feed_input(b"r");
            assert_eq!(refresh.queries, r.queries, "r re-requests the keyframe");
        }
        // An EMPTY manifest is "the hello has not landed", not "the feed is
        // absent", so the screens still ask.
        let mut unheard = TuiApp::new(caps());
        unheard.feed_input(b"1");
        assert_eq!(unheard.feed_input(b"8").queries.len(), 1);
    }

    /// I8 (cross PARTIAL low): a screen's `on_enter` is gated on the manifest
    /// exactly as its pane feeds are. On a composition without the tactical
    /// module the query does not exist, and its `Unknown query` reply would
    /// land in a scrollback neither tactical screen shows.
    #[test]
    fn the_tactical_screens_ask_nothing_on_a_manifest_without_the_unit_feed() {
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec!["wod.vitals".into()],
            character: None,
            room: None,
        });
        for digit in *b"89" {
            app.feed_input(b"1");
            assert!(
                app.feed_input(&[digit]).queries.is_empty(),
                "screen {} asked for a feed the server does not publish",
                char::from(digit)
            );
            assert!(app.feed_input(b"r").queries.is_empty(), "nor does `r`");
        }
        // The screens are still there, and still honest.
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("No tactical unit"), "{text}");
        assert!(!text.contains("(loading...)"), "{text}");
    }

    /// Look-back LB-12: `tactics.unit` is the name one pack uses, and screens
    /// 8/9 promised to follow any `<pack>.unit` feed "without a client
    /// change". They now resolve it by SUFFIX from the manifest, as the web
    /// always did and as `.sheet`/`.vitals`/`.roll` already do here: the
    /// entry query, the pane lookup and the after-action journal all follow.
    #[test]
    fn a_differently_named_unit_feed_drives_the_tactical_screens() {
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec!["mek.unit".into()],
            character: None,
            room: None,
        });
        let names: Vec<String> = app
            .feed_input(b"9")
            .queries
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["mek.unit"], "the world's own feed is asked for");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("mek.unit"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "object": { "id": "#7", "name": "Atlas" },
                "revision": 3, "state": "active", "record": null,
                "sections": [ { "id": "center_torso", "condition": "ok", "state": "ok",
                                "facets": [], "slots": [] } ],
                "cells": [ { "addr": "unit/heat", "label": "heat",
                             "current": 20, "min": 0, "max": 30, "state": "ok" } ],
                "components": [], "capabilities": [],
                "counts": { "sections": 1, "cells": 1, "components": 0, "capabilities": 0 },
                "truncated": false
            }),
        });
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("Atlas"), "the unit paints:\n{text}");
        assert!(!text.contains("No tactical unit"), "{text}");
        // The delta rings in the journal the after-action panes read.
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("mek.unit"),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({
                "object": { "id": "#7", "name": "Atlas" },
                "from_revision": 3, "to_revision": 4, "now": 1_757_000_000, "cause": "gauss",
                "entries": [ { "kind": "cell", "addr": "unit/heat", "from": 20, "to": 28 } ]
            }),
        });
        app.feed_input(b"8");
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(
            text.contains("unit/heat") && text.contains("gauss"),
            "the before/after and journal panes read the delta:\n{text}"
        );
    }

    /// I8 (hole 11): a server restarted behind the gateway re-sends its hello
    /// to a live TUI. The vitals feeds were always re-fetched; the active
    /// screen's own feeds are now too, so a tactical screen does not sit on
    /// the frame from before the restart until someone presses `r`.
    #[test]
    fn a_hello_rehydrates_the_active_screen() {
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec!["tactics.unit".into(), "tactics.contacts".into()],
            character: None,
            room: None,
        });
        app.feed_input(b"9");
        let names: Vec<String> = app
            .apply_event(&OutputEvent::Hello {
                proto: mush_proto::PROTO_VERSION,
                server: "rustmush 0.1".into(),
                features: vec![],
                panels: vec![
                    "wod.vitals".to_string(),
                    "tactics.unit".to_string(),
                    "tactics.contacts".to_string(),
                ],
            })
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        for expected in ["wod.vitals", "tactics.unit", "tactics.contacts"] {
            assert!(names.contains(&expected.to_string()), "{names:?}");
        }
        // A hello whose manifest drops the feed asks for nothing tactical.
        let names: Vec<String> = app
            .apply_event(&OutputEvent::Hello {
                proto: mush_proto::PROTO_VERSION,
                server: "rustmush 0.1".into(),
                features: vec![],
                panels: vec!["wod.vitals".to_string()],
            })
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(
            !names.iter().any(|n| n.starts_with("tactics.")),
            "{names:?}"
        );
    }

    #[test]
    fn scrollback_ring_is_capped_and_pgup_scrolls() {
        let mut app = app();
        for i in 0..(SCROLLBACK_CAP + 50) {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("line {i}"))));
        }
        assert_eq!(app.scrollback_len(), SCROLLBACK_CAP);
        let r = app.feed_input(b"\x1b[5~");
        assert!(r.redraw);
        assert_eq!(app.scroll_off(), PAGE);
        app.feed_input(b"\x1b[6~");
        assert_eq!(app.scroll_off(), 0);
        app.feed_input(b"\x1b[A");
        assert_eq!(app.scroll_off(), 1);
        app.feed_input(b"G");
        assert_eq!(app.scroll_off(), 0);
    }

    #[test]
    fn entry_queries_cover_the_manifest_and_the_screen() {
        let mut app = app();
        app.set_manifest(TuiManifest {
            panels: vec![
                "wod.vitals".into(),
                "wod.sheet".into(),
                "mail".into(),
                "comsys".into(),
                "scenes".into(),
            ],
            character: Some("Nick".into()),
            room: None,
        });
        let q: Vec<String> = app.entry_queries().into_iter().map(|(n, _)| n).collect();
        for expected in [
            "who",
            "commands.list",
            "comsys.state",
            "scenes",
            "wod.vitals",
            "legal.actions",
        ] {
            assert!(q.contains(&expected.to_string()), "{expected} in {q:?}");
        }
        assert!(!q.contains(&"wod.sheet".to_string()));
        let r = app.feed_input(b"2");
        assert!(
            r.queries.iter().any(|(n, _)| n == "wod.sheet"),
            "{:?}",
            r.queries
        );
        app.feed_input(b"4");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": [ { "name": "Public", "alias": "pub", "unread": 0 } ] }),
        });
        let r = app.feed_input(b"b");
        assert_eq!(r.queries.len(), 1);
        assert_eq!(r.queries[0].0, "comsys.backlog");
        assert_eq!(r.queries[0].1["channel"], "Public");
    }

    /// Look-back LB-11: every entry and `on_enter` query that hydrates a
    /// module feed is gated on that FEED being in the manifest, not on the
    /// query's own name — `mail.list`, `intel.list` and `comsys.state` are
    /// named differently from `mail`, `intel` and `comsys`, so the old
    /// name-match gated nothing and screens 3/4/6 earned "Unknown query" on a
    /// server without the module.
    #[test]
    fn screen_and_entry_queries_are_gated_on_the_pane_feed() {
        let names =
            |r: &InputResult| -> Vec<String> { r.queries.iter().map(|(n, _)| n.clone()).collect() };
        let mut bare = app();
        bare.set_manifest(TuiManifest {
            panels: vec!["wod.vitals".into()],
            character: Some("Nick".into()),
            room: None,
        });
        let q: Vec<String> = bare.entry_queries().into_iter().map(|(n, _)| n).collect();
        for absent in ["comsys.state", "scenes"] {
            assert!(!q.contains(&absent.to_string()), "{absent} not in {q:?}");
        }
        assert!(q.contains(&"who".to_string()), "core queries still asked");
        for (digit, absent) in [
            (b'3', "mail.list"),
            (b'4', "comsys.state"),
            (b'6', "intel.list"),
        ] {
            let r = bare.feed_input(&[digit]);
            assert!(
                !names(&r).contains(&absent.to_string()),
                "{absent} not asked: {:?}",
                names(&r)
            );
        }
        // The same screens on a server that publishes the feeds.
        let mut full = app();
        full.set_manifest(TuiManifest {
            panels: vec![
                "mail".into(),
                "comsys".into(),
                "scenes".into(),
                "intel".into(),
            ],
            character: Some("Nick".into()),
            room: None,
        });
        let q: Vec<String> = full.entry_queries().into_iter().map(|(n, _)| n).collect();
        for expected in ["comsys.state", "scenes"] {
            assert!(q.contains(&expected.to_string()), "{expected} in {q:?}");
        }
        for (digit, expected) in [
            (b'3', "mail.list"),
            (b'4', "comsys.state"),
            (b'6', "intel.list"),
        ] {
            let r = full.feed_input(&[digit]);
            assert!(
                names(&r).contains(&expected.to_string()),
                "{expected} asked: {:?}",
                names(&r)
            );
        }
    }

    /// I7: the channel screen lists IC and OOC channels apart; `^N` walks
    /// the focused pane's channels and wraps, `^O` swaps panes, and the
    /// highlight is what `b` asks backlog for. With no IC channel the OOC
    /// pane answers even while the (empty) IC pane has focus.
    #[test]
    fn channel_screen_splits_ic_and_ooc_with_ctrl_n_and_ctrl_o() {
        let mut app = app();
        app.feed_input(b"4");
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": [
                { "name": "Public", "alias": "pub", "unread": 0 },
                { "name": "Staff", "alias": "st", "unread": 0 },
            ] }),
        });
        assert_eq!(app.focused_pane(), Some(PaneId("ic")));
        let r = app.feed_input(b"b");
        assert_eq!(r.queries[0].1["channel"], "Public", "the OOC pane answers");
        // ^N from the empty IC pane: nothing to walk, focus stays.
        app.feed_input(&[0x0e]);
        assert_eq!(app.focused_pane(), Some(PaneId("ic")));
        // ^O to the OOC pane; ^N walks and wraps.
        app.feed_input(&[0x0f]);
        assert_eq!(app.focused_pane(), Some(PaneId("ooc")));
        app.feed_input(&[0x0e]);
        assert_eq!(app.feed_input(b"b").queries[0].1["channel"], "Staff");
        app.feed_input(&[0x0e]);
        assert_eq!(
            app.feed_input(b"b").queries[0].1["channel"],
            "Public",
            "wraps"
        );
        // An IC channel lands in the IC pane and ^O reaches it.
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("comsys"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({ "channels": [
                { "name": "Public", "alias": "pub", "unread": 0 },
                { "name": "Radio", "alias": "rad", "unread": 2, "ic": true },
            ] }),
        });
        app.feed_input(&[0x0f]);
        assert_eq!(app.focused_pane(), Some(PaneId("ic")));
        assert_eq!(app.feed_input(b"b").queries[0].1["channel"], "Radio");
        // Ctrl+N and Ctrl+O are inert on a screen without channel panes.
        app.feed_input(b"1");
        let before = app.focused_pane();
        app.feed_input(&[0x0e, 0x0f]);
        assert_eq!(app.focused_pane(), before);
    }

    /// I7-C12: `Enter` on the channel screen prefills the channel the screen
    /// is already acting on. On a stock server every channel is OOC and focus
    /// opens on the empty IC pane, whose one row is not selectable — so the
    /// prefill has to fall back to the channel `b` would ask backlog for. With
    /// an IC channel present the focused pane answers for itself.
    #[test]
    fn channel_enter_prefills_the_fallback_channel() {
        let on_channel_screen = |channels: Value| {
            let mut a = app();
            a.feed_input(b"4");
            a.apply_event(&OutputEvent::Panel {
                feed: SmolStr::new_static("comsys"),
                kind: PanelFrameKind::Keyframe,
                data: serde_json::json!({ "channels": channels }),
            });
            a
        };
        let mut ooc_only = on_channel_screen(serde_json::json!([
            { "name": "Public", "alias": "pub", "unread": 0 },
        ]));
        assert_eq!(
            ooc_only.focused_pane(),
            Some(PaneId("ic")),
            "the empty IC pane"
        );
        ooc_only.feed_input(b"\r");
        assert_eq!(ooc_only.mode(), Mode::Input);
        assert_eq!(
            ooc_only.input_text(),
            "pub ",
            "the OOC pane's channel answers"
        );
        // With an IC channel the focused pane has its own selectable row.
        let mut with_ic = on_channel_screen(serde_json::json!([
            { "name": "Public", "alias": "pub", "unread": 0 },
            { "name": "Radio", "alias": "rad", "unread": 0, "ic": true },
        ]));
        with_ic.feed_input(b"\r");
        assert_eq!(with_ic.mode(), Mode::Input);
        assert_eq!(with_ic.input_text(), "rad ");
    }

    /// I10: the server-initiated request overlay — a confirm answers with
    /// `y`/`n`, a choice with a digit, a target/text opens the box so the next
    /// line is the answer, and a key that answers nothing leaves it up. The
    /// answers ride `InputResult.responds`.
    #[test]
    fn request_overlay_answers_across_kinds() {
        let mut app = app();
        let req = |id, kind, options: Vec<RequestOption>| OutputEvent::Request {
            epoch: "test".into(),
            id,
            kind,
            prompt: RichText::plain("?"),
            options,
            deadline_ms: None,
        };

        app.apply_event(&req(1, RequestKind::Confirm, vec![]));
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(1, "test".into(), RespondChoice::Confirm(true))]
        );

        app.apply_event(&req(2, RequestKind::Confirm, vec![]));
        assert_eq!(
            app.feed_input(b"n").responds,
            vec![(2, "test".into(), RespondChoice::Confirm(false))]
        );

        app.apply_event(&req(
            3,
            RequestKind::Choice,
            vec![
                RequestOption {
                    id: "razor".into(),
                    label: "Razor".into(),
                },
                RequestOption {
                    id: "mara".into(),
                    label: "Mara".into(),
                },
            ],
        ));
        assert_eq!(
            app.feed_input(b"2").responds,
            vec![(3, "test".into(), RespondChoice::Choose("mara".into()))]
        );

        // Target/Text: Enter opens the box, the next line is the answer.
        app.apply_event(&req(4, RequestKind::Text, vec![]));
        assert!(
            app.feed_input(b"\r").responds.is_empty(),
            "Enter opens the box"
        );
        assert_eq!(
            app.feed_input(b"Fenris\r").responds,
            vec![(4, "test".into(), RespondChoice::Text("Fenris".into()))]
        );

        // A key that answers nothing leaves the overlay up.
        app.apply_event(&req(5, RequestKind::Confirm, vec![]));
        assert!(app.feed_input(b"z").responds.is_empty());
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(5, "test".into(), RespondChoice::Confirm(true))]
        );
    }

    /// I6: `^R` opens the box with `+roll ` ready and Tab completes the
    /// sheet's categorised dotted traits inside the line (after a space or
    /// a `+`); an uncategorised or non-dotted field is never offered. The
    /// line goes out exactly as built.
    #[test]
    fn ctrl_r_prefills_a_roll_and_tab_completes_sheet_traits() {
        let mut app = app();
        app.apply_event(&OutputEvent::Panel {
            feed: SmolStr::new_static("wod.sheet"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "target": "#1",
                "schema": { "ruleset": "wod", "version": 1, "sections": [
                    { "id": "a", "label": "A", "layout": "list", "fields": [
                        { "id": "strength", "label": "Strength", "ty": { "kind": "dots", "min": 1, "max": 5 }, "category": "physical", "default": 1 },
                        { "id": "stamina", "label": "Stamina", "ty": { "kind": "dots", "min": 1, "max": 5 }, "category": "physical", "default": 1 },
                        { "id": "brawl", "label": "Brawl", "ty": { "kind": "dots", "min": 0, "max": 5 }, "category": "talents", "default": 0 },
                        { "id": "willpower", "label": "Willpower", "ty": { "kind": "number", "min": 1, "max": 10 }, "default": 1 },
                        { "id": "luck", "label": "Luck", "ty": { "kind": "dots", "min": 0, "max": 5 }, "default": 0 }
                    ] }
                ] },
                "approved": { "state": "approved", "approved": true },
                "data": { "strength": 3, "stamina": 2, "brawl": 2, "willpower": 5, "luck": 1 }
            }),
        });
        app.feed_input(&[0x12]);
        assert_eq!(app.mode(), Mode::Input);
        assert_eq!(app.input_text(), "+roll ");
        app.feed_input(b"st\t");
        assert_eq!(app.input_text(), "+roll stamina", "the first match, sorted");
        app.feed_input(&[0x15]);
        app.feed_input(b"+roll str\t");
        assert_eq!(app.input_text(), "+roll strength");
        app.feed_input(b"+br\t");
        assert_eq!(app.input_text(), "+roll strength+brawl");
        app.feed_input(b"+lu\t");
        assert_eq!(
            app.input_text(),
            "+roll strength+brawl+lu",
            "uncategorised: not a trait"
        );
        app.feed_input(&[0x15]);
        app.feed_input(b"+roll wil\t");
        assert_eq!(
            app.input_text(),
            "+roll wil",
            "a number field is not a trait"
        );
        app.feed_input(&[0x15]);
        app.feed_input(b"+roll strength+brawl vs 7");
        let r = app.feed_input(b"\r");
        assert_eq!(r.submit, vec!["+roll strength+brawl vs 7".to_string()]);
    }

    // ---- I12: the narrative-history backlog -------------------------------

    /// One `narrative.backlog` reply exactly as the gateway sends it
    /// (`bridge_common.rs`, `narrative_backlog_reply`): a `Reply` with the
    /// hard-coded id 0 wrapping a `narrative` Delta panel frame.
    fn backlog_reply(lines: Value, more: bool, evicted: bool) -> OutputEvent {
        OutputEvent::Reply {
            id: 0,
            event: Box::new(OutputEvent::Panel {
                feed: SmolStr::new_static("narrative"),
                kind: PanelFrameKind::Delta,
                data: serde_json::json!({ "backlog": {
                    "before_seq": Value::Null, "more": more,
                    "evicted": evicted, "lines": lines } }),
            }),
        }
    }

    /// One line of a page. `ts` is Unix MILLISECONDS, as `Envelope.ts` is.
    fn hline(seq: u64, name: &str, text: &str) -> Value {
        serde_json::json!({
            "seq": seq, "ts": 1_700_000_000_000i64, "kind": "say",
            "actor": { "id": "#5", "name": name },
            "text": RichText::plain(text),
        })
    }

    fn rows_of(app: &TuiApp) -> Vec<String> {
        app.scrollback_rows(70)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// A clock ten seconds past the fixture's, for the aging latch.
    fn clock_ten_s_later() -> i64 {
        3600 * 14 + 60 * 30 + 10
    }

    /// I12, the regression that matters most: a narrative-backlog page is
    /// NOT a panel frame. Before the interception it fell through to the
    /// `PanelStore`'s unknown-feed Delta catch-all, which asked the world to
    /// resync a feed no producer publishes — earning an "Unknown query" line
    /// and losing the page. It must reach neither the store nor the wire.
    #[test]
    fn a_narrative_backlog_reply_never_reaches_the_panel_store() {
        let mut app = app();
        let q = app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "Mireille", "the rain again")]),
            true,
            false,
        ));
        assert!(q.is_empty(), "no resync may be asked for: {q:?}");
        assert!(app.panels().get("narrative").is_none());
        assert_eq!(app.scrollback_len(), 1, "the page was painted, not dropped");
    }

    /// The wire carries a page newest-first; the scrollback must read
    /// oldest-first above whatever is already there, and never below it.
    #[test]
    fn a_backlog_page_prepends_oldest_first_above_the_live_tail() {
        let mut app = app();
        app.apply_event(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Nick")),
            text: RichText::plain("live one"),
        });
        app.apply_event(&backlog_reply(
            serde_json::json!([
                hline(9, "Mireille", "seq nine"),
                hline(8, "Mireille", "seq eight"),
                hline(7, "Mireille", "seq seven"),
            ]),
            true,
            false,
        ));
        let rows = rows_of(&app);
        assert_eq!(rows.len(), 4, "{rows:?}");
        for (row, want) in rows
            .iter()
            .zip(["seq seven", "seq eight", "seq nine", "live one"])
        {
            assert!(row.contains(want), "{rows:?}");
        }
    }

    /// The next page is asked for `before` the OLDEST seq of the last one,
    /// and the limit is always explicit (the gateway reads an absent or zero
    /// limit as one line).
    #[test]
    fn paging_uses_the_oldest_seq_as_the_next_cursor() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "a"), hline(7, "M", "b")]),
            true,
            false,
        ));
        let r = app.feed_input(b"g");
        assert_eq!(r.queries.len(), 1, "{:?}", r.queries);
        assert_eq!(r.queries[0].0, "narrative.backlog");
        assert_eq!(
            r.queries[0].1,
            serde_json::json!({ "before_seq": 7, "limit": 100 })
        );
    }

    /// One page in flight at a time — and the latch AGES, because the
    /// gateway drops a throttled `narrative.backlog` in silence and a plain
    /// flag would wedge history for the rest of the connection.
    #[test]
    fn one_page_in_flight_and_the_latch_ages() {
        let mut app = app();
        assert_eq!(app.feed_input(b"g").queries.len(), 1);
        assert!(
            app.feed_input(b"g").queries.is_empty(),
            "a second ask while one is in flight"
        );
        app.set_clock(clock_ten_s_later);
        assert_eq!(
            app.feed_input(b"g").queries.len(),
            1,
            "past the latch it asks again"
        );
    }

    /// The cursor is the dedup (an SSH scrollback entry can carry no seq):
    /// a page that races a re-armed latch, or a second priming page, is
    /// dropped rather than painted twice.
    #[test]
    fn a_late_page_that_is_not_older_than_the_cursor_is_dropped() {
        let mut app = app();
        let page = || {
            backlog_reply(
                serde_json::json!([hline(9, "M", "a"), hline(7, "M", "b")]),
                true,
                false,
            )
        };
        app.apply_event(&page());
        let after_first = app.scrollback_len();
        assert_eq!(after_first, 2);
        app.apply_event(&page());
        assert_eq!(app.scrollback_len(), after_first, "the same page again");
        // …and a stale page does not end the backlog either: the next ask
        // still carries the cursor.
        let r = app.feed_input(b"g");
        assert_eq!(r.queries[0].1["before_seq"], 7, "{:?}", r.queries);
    }

    /// `more: false` ends the backlog: nothing more is asked for, and one
    /// dim line at the very top says why — worded from `evicted`.
    #[test]
    fn more_false_stops_asking_and_marks_the_floor() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "the oldest line")]),
            false,
            true,
        ));
        let rows = rows_of(&app);
        assert!(rows[0].contains("rolled off"), "{rows:?}");
        assert!(rows[1].contains("the oldest line"), "{rows:?}");
        assert!(
            app.feed_input(b"g").queries.is_empty(),
            "an exhausted backlog asks nothing"
        );
        // The marker is emitted once, not on every later page.
        app.apply_event(&backlog_reply(serde_json::json!([]), false, true));
        assert_eq!(rows_of(&app).len(), 2);
    }

    /// With nothing evicted the floor is worded the other way: the scene
    /// simply has no more history, which is not the same as losing it.
    #[test]
    fn an_unevicted_floor_says_there_is_simply_no_more() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "the first line ever")]),
            false,
            false,
        ));
        let rows = rows_of(&app);
        assert!(rows[0].contains("no older history"), "{rows:?}");
        assert!(!rows[0].contains("rolled off"), "{rows:?}");
    }

    /// A prepend must never evict the live tail — `push_entry` pops the
    /// FRONT at the cap, so running past it would delete live scene text.
    /// The page is cut instead, and a cut page ends the backlog (otherwise
    /// the same one is re-fetched on every scroll, forever).
    #[test]
    fn a_page_never_evicts_the_live_tail_and_a_truncated_page_ends_the_backlog() {
        let mut app = app();
        for i in 0..(SCROLLBACK_CAP - 2) {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("live {i}"))));
        }
        assert_eq!(app.scrollback_len(), SCROLLBACK_CAP - 2);
        let lines: Vec<Value> = (0..10)
            .map(|i| hline(20 - i, "M", &format!("old {i}")))
            .collect();
        app.apply_event(&backlog_reply(Value::Array(lines), true, false));
        assert_eq!(app.scrollback_len(), SCROLLBACK_CAP);
        let rows = rows_of(&app);
        assert!(
            rows[rows.len() - 1].contains(&format!("live {}", SCROLLBACK_CAP - 3)),
            "the live tail survived: {:?}",
            rows[rows.len() - 1]
        );
        assert!(
            app.feed_input(b"g").queries.is_empty(),
            "a cut page ends the backlog"
        );
    }

    /// `scroll_off` counts up from the TAIL, so prepending preserves the
    /// reader's place for free — nothing in the apply path may touch it.
    #[test]
    fn prepending_does_not_move_the_reader() {
        let mut app = app();
        for i in 0..60 {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("line {i}"))));
        }
        let _ = super::super::frame_text(&mut app, 120, 34);
        app.feed_input(b"\x1b[5~");
        let _ = super::super::frame_text(&mut app, 120, 34);
        let before = app.scroll_off();
        assert_eq!(before, PAGE, "the paint kept the reader where they were");
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "a"), hline(8, "M", "b")]),
            true,
            false,
        ));
        assert_eq!(app.scroll_off(), before);
    }

    /// Entering the TUI primes the first page. That moment — a fresh
    /// `TuiApp`, an EMPTY scrollback — is what makes the whole design
    /// dedup-free, and it is the player value besides.
    #[test]
    fn entry_queries_prime_the_first_narrative_page() {
        let mut app = app();
        let q = app.entry_queries();
        let asked: Vec<&Value> = q
            .iter()
            .filter(|(n, _)| n == "narrative.backlog")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(asked.len(), 1, "{q:?}");
        assert_eq!(
            *asked[0],
            serde_json::json!({ "before_seq": Value::Null, "limit": 100 })
        );
        // FIRST, because `enter_tui` awaits these in order and every query
        // ahead of it widens the window in which a live line is painted both
        // by the pump and by the page (C3: narrowed here, not closed).
        assert_eq!(q[0].0, "narrative.backlog", "{q:?}");
        // The priming request arms the latch, so an immediate `g` does not
        // ask for the same page twice.
        assert!(app.feed_input(b"g").queries.is_empty());
    }

    /// `/clear` is a VIEW floor, not a data clear: the entries stay in the
    /// ring (so the history cursor can stay honest), the draw hides them, and
    /// live text lands below the floor and shows at once.
    #[test]
    fn clear_hides_the_view_without_dropping_the_ring() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "recalled")]),
            true,
            false,
        ));
        app.apply_event(&OutputEvent::Sys(RichText::plain("live one")));
        let held = app.scrollback_len();
        assert_eq!(held, 2);
        app.feed_input(b"\r");
        app.feed_input(b"/clear\r");
        app.feed_input(b"\x1b"); // back to nav, where `g` is a key again
        assert!(rows_of(&app).is_empty(), "the view is empty");
        assert_eq!(app.scrollback_len(), held, "the ring is not");
        app.apply_event(&OutputEvent::Sys(RichText::plain("after the clear")));
        let rows = rows_of(&app);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].contains("after the clear"), "{rows:?}");
    }

    /// The C2 regression. `/clear` used to re-arm the cursor, so the next
    /// history gesture asked for the NEWEST page — which came back holding
    /// every line that had arrived live since the clear, and painted each of
    /// them a second time (an SSH entry carries no seq to dedup against).
    /// A cleared view now asks for nothing at all, and a page that was
    /// already in flight when the clear happened cannot uncover it either.
    #[test]
    fn a_clear_never_re_fetches_the_lines_that_arrived_after_it() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "primed")]),
            true,
            false,
        ));
        app.feed_input(b"\r");
        app.feed_input(b"/clear\r");
        app.feed_input(b"\x1b");
        for i in 0..3 {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("live {i}"))));
        }
        assert!(
            app.feed_input(b"g").queries.is_empty(),
            "a cleared view asks for nothing"
        );
        // The page the old re-arm would have fetched, delivered anyway (a
        // page in flight at clear time): the kept cursor drops every line.
        app.apply_event(&backlog_reply(
            serde_json::json!([
                hline(12, "M", "live 2"),
                hline(11, "M", "live 1"),
                hline(10, "M", "live 0"),
            ]),
            true,
            false,
        ));
        // …and so is a page of genuinely OLDER lines: prepending one over a
        // floor counted from the front would push cleared lines back up.
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(8, "M", "older still")]),
            true,
            false,
        ));
        let rows = rows_of(&app);
        assert_eq!(rows.len(), 3, "each live line exactly once: {rows:?}");
        for (row, want) in rows.iter().zip(["live 0", "live 1", "live 2"]) {
            assert!(row.contains(want), "{rows:?}");
        }
        assert!(
            !rows.iter().any(|r| r.contains("primed")),
            "the cleared line stays cleared: {rows:?}"
        );
    }

    /// A page cut before its FIRST line paints nothing at all, and a gesture
    /// that silently does nothing reads as a broken key: say why, as a client
    /// line, and name the remedy that still works (`/clear` hides the ring
    /// now, it does not empty it).
    #[test]
    fn a_full_scrollback_says_so_instead_of_swallowing_the_gesture() {
        let mut app = app();
        for i in 0..SCROLLBACK_CAP {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("live {i}"))));
        }
        assert_eq!(app.scrollback_len(), SCROLLBACK_CAP);
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "never fits")]),
            true,
            false,
        ));
        let rows = rows_of(&app);
        assert!(
            rows.iter().any(|r| r.contains("The scrollback is full")),
            "{:?}",
            &rows[rows.len().saturating_sub(3)..]
        );
        assert!(
            !rows.iter().any(|r| r.contains("never fits")),
            "nothing was prepended"
        );
        assert!(
            app.feed_input(b"g").queries.is_empty(),
            "and it stays quiet afterwards"
        );
    }

    /// `/history` is the discoverable form of the same gesture — and the page
    /// it loads lands where the player can see it.
    #[test]
    fn the_history_local_command_asks_for_a_page() {
        let mut app = app();
        // Enough live text that the top of the buffer is off screen.
        for i in 0..60 {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("live {i}"))));
        }
        app.feed_input(b"\r");
        let r = app.feed_input(b"/history\r");
        assert!(r.submit.is_empty(), "it never reaches the world");
        assert_eq!(r.queries.len(), 1);
        assert_eq!(r.queries[0].0, "narrative.backlog");
        assert!(
            LOCAL_COMMANDS.iter().any(|(c, _)| *c == "/history"),
            "the palette lists it"
        );
        // `submit_line` pins the view to the tail before the local command
        // runs; without the jump the page prepends off-screen and nothing
        // visible happens.
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "the recalled line")]),
            true,
            false,
        ));
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(frame.contains("the recalled line"), "{frame}");
        // …and the `?` crib names the gesture, whole and unclipped.
        app.feed_input(b"\x1b?");
        let help = super::super::frame_text(&mut app, 120, 34);
        assert!(help.contains("loads older scene history"), "{help}");
        assert!(help.contains("/clear hides what is on screen"), "{help}");
    }

    /// A page we cannot read stops the backlog rather than panicking or
    /// looping on a cursor that never advanced.
    #[test]
    fn a_malformed_page_stops_the_backlog_without_panicking() {
        let mut app = app();
        let q = app.apply_event(&OutputEvent::Reply {
            id: 0,
            event: Box::new(OutputEvent::Panel {
                feed: SmolStr::new_static("narrative"),
                kind: PanelFrameKind::Delta,
                data: serde_json::json!({ "backlog": { "lines": [ { "seq": "x" } ] } }),
            }),
        });
        assert!(q.is_empty());
        assert_eq!(app.scrollback_len(), 0);
        assert!(app.feed_input(b"g").queries.is_empty());
    }

    /// A world restarted behind the gateway re-sends its hello. Clearing the
    /// LATCH there is belt and braces that costs nothing — the page is
    /// answered by the gateway from the session ring, so it arrives whatever
    /// the world does, and the cursor filter drops it as a duplicate. The
    /// CURSOR must not re-arm: the session and its ring outlive the world,
    /// seqs do not restart, and re-arming would re-fetch and duplicate the
    /// newest page. (This is why the web's `beginSegment` is not ported.)
    #[test]
    fn hello_keeps_the_cursor_but_clears_the_latch() {
        let mut app = app();
        app.apply_event(&backlog_reply(
            serde_json::json!([hline(9, "M", "a"), hline(7, "M", "b")]),
            true,
            false,
        ));
        let first = app.feed_input(b"g");
        assert_eq!(first.queries[0].1["before_seq"], 7);
        assert!(app.feed_input(b"g").queries.is_empty(), "latched");
        app.apply_event(&OutputEvent::Hello {
            proto: 1,
            server: "rustmush".into(),
            features: vec![],
            panels: vec![],
        });
        let after = app.feed_input(b"g");
        assert_eq!(after.queries.len(), 1, "the latch cleared");
        assert_eq!(
            after.queries[0].1["before_seq"], 7,
            "the cursor did NOT re-arm"
        );
    }

    /// I10 parity: asks QUEUE oldest-first. The engine's `respond` resolves
    /// the lowest open id and the web store answers `pending[0]`; a TUI that
    /// replaced the showing ask with the newest measured the player's answer
    /// against one they could not see, and lost the older one silently.
    #[test]
    fn request_overlay_queues_and_answers_oldest_first() {
        let mut app = app();
        let choice = OutputEvent::Request {
            epoch: "test".into(),
            id: 7,
            kind: RequestKind::Choice,
            prompt: RichText::plain("which door?"),
            options: vec![
                RequestOption {
                    id: "left".into(),
                    label: "Left".into(),
                },
                RequestOption {
                    id: "right".into(),
                    label: "Right".into(),
                },
            ],
            deadline_ms: None,
        };
        let confirm = OutputEvent::Request {
            epoch: "test".into(),
            id: 9,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("end the scene?"),
            options: vec![],
            deadline_ms: None,
        };
        app.apply_event(&choice);
        app.apply_event(&confirm);
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(
            frame.contains("which door?"),
            "the OLDER ask shows: {frame}"
        );
        assert!(!frame.contains("end the scene?"));
        assert!(frame.contains("the game asks (1 of 2)"), "{frame}");

        assert_eq!(
            app.feed_input(b"1").responds,
            vec![(7, "test".into(), RespondChoice::Choose("left".into()))]
        );
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(frame.contains("end the scene?"), "the next ask: {frame}");
        assert!(frame.contains("the game asks"), "{frame}");
        assert!(!frame.contains("1 of"), "one left: {frame}");
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(9, "test".into(), RespondChoice::Confirm(true))]
        );
        assert!(!super::super::frame_text(&mut app, 120, 34).contains("end the scene?"));

        // Esc on the front one cancels it and reveals the next.
        app.apply_event(&choice);
        app.apply_event(&confirm);
        assert_eq!(
            app.feed_input(b"\x1b").responds,
            vec![(7, "test".into(), RespondChoice::Cancel)]
        );
        assert!(super::super::frame_text(&mut app, 120, 34).contains("end the scene?"));

        // A repeat of an open id replaces it in place — it does not queue
        // twice and does not jump the line.
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 9,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("end the scene now?"),
            options: vec![],
            deadline_ms: None,
        });
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(frame.contains("end the scene now?"), "{frame}");
        assert!(!frame.contains("1 of"), "still one ask: {frame}");

        // The position rule, where it is observable: a re-send of the FRONT
        // id while another waits behind it keeps the front slot. (The web
        // store demoted it to the back and showed the other one; in-place is
        // the rule that behaves on a remint collision, and the web is moving
        // to match — D-I-29.)
        app.apply_event(&choice);
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(
            frame.contains("end the scene now?"),
            "still the front: {frame}"
        );
        assert!(frame.contains("the game asks (1 of 2)"), "{frame}");
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 9,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("end it here?"),
            options: vec![],
            deadline_ms: None,
        });
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(
            frame.contains("end it here?"),
            "the front was replaced in place: {frame}"
        );
        assert!(
            !frame.contains("which door?"),
            "it did not go to the back: {frame}"
        );
        assert!(
            frame.contains("the game asks (1 of 2)"),
            "still two: {frame}"
        );
    }

    /// One `Confirm` ask, with a deadline.
    fn timed_ask(id: u64, prompt: &str, deadline_ms: Option<u64>) -> OutputEvent {
        OutputEvent::Request {
            epoch: "test".into(),
            id,
            kind: RequestKind::Confirm,
            prompt: RichText::plain(prompt),
            options: vec![],
            deadline_ms,
        }
    }

    /// C5: the engine expires a request and emits NOTHING to the subject, so
    /// the deadline is all a client has. A dead front ask must not be drawn,
    /// must not eat the key that finds it, and must not hide the live ask
    /// queued behind it. (HEAD's single slot self-healed — the newest ask
    /// replaced the zombie; a queue makes every zombie a blocker.)
    #[test]
    fn an_expired_front_ask_is_evicted_before_it_can_eat_a_key() {
        let mut app = app();
        // The fixture clock is 52 200 s; `clock_ten_s_later` is 52 210 s.
        app.apply_event(&timed_ask(1, "the stale ask", Some(52_205_000)));
        app.apply_event(&timed_ask(2, "the live ask", Some(52_260_000)));
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(frame.contains("the stale ask"), "{frame}");
        assert!(frame.contains("the game asks (1 of 2)"), "{frame}");

        app.set_clock(clock_ten_s_later);
        let r = app.feed_input(b"y");
        assert!(
            r.responds.is_empty(),
            "no answer rides for an ask the engine already dropped: {:?}",
            r.responds
        );
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(
            frame.contains("the live ask"),
            "the next one shows: {frame}"
        );
        assert!(!frame.contains("the stale ask"), "{frame}");
        assert!(!frame.contains("1 of"), "one left: {frame}");
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(2, "test".into(), RespondChoice::Confirm(true))],
            "and it answers normally"
        );
    }

    /// The draw path evicts too, so a dead ask cannot be painted by a repaint
    /// the player never caused.
    #[test]
    fn an_expired_ask_is_never_painted() {
        let mut app = app();
        app.apply_event(&timed_ask(1, "the stale ask", Some(52_205_000)));
        app.set_clock(clock_ten_s_later);
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(!frame.contains("the game asks"), "{frame}");
        assert!(app.requests.is_empty(), "and it left the queue");
    }

    /// Over SSH the `Hello` IS the world-restart seam (`Resumed` reaches no
    /// Ansi sink). The request registry is RAM-only and ids remint from 1, so
    /// every open ask died with the world: the queue goes, and a box that was
    /// capturing an answer to one of them hands its text back as a draft
    /// rather than sending it to the new world as a command.
    #[test]
    fn a_hello_drops_every_open_ask() {
        let mut app = app();
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 1,
            kind: RequestKind::Text,
            prompt: RichText::plain("name them"),
            options: vec![],
            deadline_ms: None,
        });
        app.apply_event(&timed_ask(2, "sure?", None));
        assert!(app.feed_input(b"\r").responds.is_empty(), "the box opens");
        app.feed_input(b"Fenr");
        app.apply_event(&OutputEvent::Hello {
            proto: 1,
            server: "rustmush".into(),
            features: vec![],
            panels: vec![],
        });
        assert!(app.requests.is_empty(), "the queue is empty");
        assert!(app.answering.is_none(), "and nothing is being answered");
        assert_eq!(app.mode(), Mode::Nav);
        assert_eq!(app.draft(), Some("Fenr"), "the typing is kept as a draft");
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(!frame.contains("the game asks"), "{frame}");
        assert!(
            app.feed_input(b"y").responds.is_empty(),
            "no answer can ride for a dead ask"
        );
    }

    /// C6: Esc out of the box that is capturing a free-text answer cancels
    /// that ask — the overlay's own printed contract. A dangling `answering`
    /// wedges BOTH gates (key routing and the overlay), so the next ask would
    /// be invisible and unkeyed until the player happened to submit a line —
    /// which would then be eaten as an answer to an ask no longer on screen.
    #[test]
    fn esc_out_of_the_answer_box_cancels_the_ask_and_frees_the_queue() {
        let mut app = app();
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 1,
            kind: RequestKind::Text,
            prompt: RichText::plain("name them"),
            options: vec![],
            deadline_ms: None,
        });
        app.apply_event(&timed_ask(2, "sure?", None));
        assert!(app.feed_input(b"\r").responds.is_empty(), "the box opens");
        assert_eq!(
            app.feed_input(b"\x1b").responds,
            vec![(1, "test".into(), RespondChoice::Cancel)]
        );
        assert!(app.answering.is_none());
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(frame.contains("sure?"), "ask 2 shows at once: {frame}");
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(2, "test".into(), RespondChoice::Confirm(true))]
        );
    }

    /// The same for the other exit: an empty line in the box IS the cancel
    /// `submit_line` documents, and it never reaches `submit_line`.
    #[test]
    fn an_empty_line_in_the_answer_box_cancels_the_ask() {
        let mut app = app();
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 1,
            kind: RequestKind::Text,
            prompt: RichText::plain("name them"),
            options: vec![],
            deadline_ms: None,
        });
        assert!(app.feed_input(b"\r").responds.is_empty(), "the box opens");
        assert_eq!(
            app.feed_input(b"\r").responds,
            vec![(1, "test".into(), RespondChoice::Cancel)]
        );
        assert!(app.answering.is_none());
        assert_eq!(app.mode(), Mode::Nav);
    }

    /// Past the engine's own per-subject cap the newest ask is dropped, and
    /// the ones already open still answer oldest-first.
    #[test]
    fn the_request_queue_is_capped_at_the_engine_s_own_limit() {
        let mut app = app();
        for id in 0..(MAX_OPEN_REQUESTS_PER_SUBJECT as u64 + 5) {
            app.apply_event(&OutputEvent::Request {
                epoch: "test".into(),
                id,
                kind: RequestKind::Confirm,
                prompt: RichText::plain(format!("ask {id}")),
                options: vec![],
                deadline_ms: None,
            });
        }
        assert_eq!(app.requests.len(), MAX_OPEN_REQUESTS_PER_SUBJECT);
        assert_eq!(
            app.feed_input(b"y").responds,
            vec![(0, "test".into(), RespondChoice::Confirm(true))],
            "the oldest is still the one answered"
        );
    }

    /// A free-text ask hands the keyboard to the box: the ask behind it must
    /// not steal the keys, and must be showing again once the line is sent.
    #[test]
    fn a_text_ask_keeps_the_box_while_the_next_ask_waits() {
        let mut app = app();
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 1,
            kind: RequestKind::Text,
            prompt: RichText::plain("name them"),
            options: vec![],
            deadline_ms: None,
        });
        app.apply_event(&OutputEvent::Request {
            epoch: "test".into(),
            id: 2,
            kind: RequestKind::Confirm,
            prompt: RichText::plain("sure?"),
            options: vec![],
            deadline_ms: None,
        });
        assert!(
            app.feed_input(b"\r").responds.is_empty(),
            "Enter opens the box"
        );
        let frame = super::super::frame_text(&mut app, 120, 34);
        assert!(!frame.contains("sure?"), "the next ask waits: {frame}");
        assert_eq!(
            app.feed_input(b"Fenris\r").responds,
            vec![(1, "test".into(), RespondChoice::Text("Fenris".into()))]
        );
        assert!(super::super::frame_text(&mut app, 120, 34).contains("sure?"));
    }

    // -----------------------------------------------------------------------
    // The cockpit screen (16b cut Q.4; doc 16 §5)
    // -----------------------------------------------------------------------

    /// The canonical scene 16b cut Q.3 committed, which all three renderers
    /// read. Rust `include_str!`s it exactly as `screen.rs` does the run
    /// allowlist; vitest reads it natively.
    const COCKPIT_FIXTURE: &str =
        include_str!("../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");
    const ARENA_MAP_FIXTURE: &str =
        include_str!("../../../../web/src/lib/fixtures/cockpit/arena_map.json");

    fn arena_json(text: &str) -> Value {
        serde_json::from_str(text).expect("a committed fixture is JSON")
    }

    fn arena_keyframe(feed: &'static str, data: Value) -> OutputEvent {
        OutputEvent::Panel {
            feed: SmolStr::new_static(feed),
            kind: PanelFrameKind::Keyframe,
            data,
        }
    }

    /// The cleared cockpit 16b cut Q.3 sends when the seat goes: two keys.
    fn cockpit_cleared() -> OutputEvent {
        arena_keyframe(
            COCKPIT_FEED,
            serde_json::json!({
                "schema": { "id": COCKPIT_FEED, "version": 1, "retain": "latest" },
                "seated": false
            }),
        )
    }

    /// A server that publishes both arena feeds.
    fn arena_manifest() -> TuiManifest {
        TuiManifest {
            panels: vec![COCKPIT_FEED.to_string(), ARENA_MAP_FEED.to_string()],
            character: None,
            room: None,
        }
    }

    /// An app on such a server, at the shared fixed clock.
    fn arena_app() -> TuiApp {
        let mut app = app();
        app.set_manifest(arena_manifest());
        app
    }

    /// The clock the self-heal test drives. It is an atomic because
    /// [`TuiApp::set_clock`] takes a plain `fn` pointer, and exactly ONE test
    /// touches it, so the shared cell cannot race another.
    static HEAL_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn heal_clock() -> i64 {
        HEAL_CLOCK.load(Ordering::Relaxed)
    }

    /// 16b cut Q.4: `@tui` against a world with an arena but no seat of your
    /// own opens on the SCENE and shows no cockpit tab at all — the tab is a
    /// held `arena.cockpit` frame, and the arena pushes that to seated pilots
    /// and to nobody else. With a frame, `0` and `Alt+0` reach it.
    #[test]
    fn zero_reaches_the_cockpit_only_for_a_viewer_the_arena_sends_frames_to() {
        // A bystander on an arena server: the feed is published, no frame has
        // ever arrived, so there is no tenth tab and the digit is inert.
        let mut bystander = arena_app();
        // The arena pushes a frame only when one CHANGES, so entering the TUI
        // asks for one; a viewer with no seat is answered `seated: false`.
        assert!(
            bystander
                .entry_queries()
                .iter()
                .any(|(n, _)| n == COCKPIT_FEED),
            "the cockpit feed is hydrated on entry"
        );
        bystander.feed_input(b"0");
        assert_eq!(
            bystander.active_screen(),
            ScreenId(1),
            "no frame, no screen: @tui opens on the scene"
        );
        let text = super::super::frame_text(&mut bystander, 120, 34);
        assert!(!text.contains("cockpit"), "no tenth tab:\n{text}");

        let mut seated = arena_app();
        seated.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        seated.feed_input(b"1");
        assert_eq!(seated.active_screen(), ScreenId(1));
        seated.feed_input(b"0");
        assert_eq!(seated.active_screen(), COCKPIT_SCREEN);
        seated.feed_input(b"1");
        // Alt+0 is the same hotkey from the input box.
        seated.feed_input(b"\x1b0");
        assert_eq!(seated.active_screen(), COCKPIT_SCREEN);
        let text = super::super::frame_text(&mut seated, 120, 34);
        assert!(text.contains("0 cockpit"), "the tenth tab:\n{text}");

        // A world with no arena: the frame could never arrive either.
        let mut bare = app();
        bare.set_manifest(TuiManifest {
            panels: vec!["mail".into()],
            character: None,
            room: None,
        });
        bare.feed_input(b"0");
        assert_eq!(
            bare.active_screen(),
            ScreenId(1),
            "a digit with no screen is inert"
        );
        let text = super::super::frame_text(&mut bare, 120, 34);
        assert!(!text.contains("cockpit"), "no tenth tab:\n{text}");

        // A hello that takes the feed away while the player is standing on the
        // screen puts them back on the scene rather than on a tab that is gone.
        let mut left = arena_app();
        left.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(left.active_screen(), COCKPIT_SCREEN);
        left.apply_event(&OutputEvent::Hello {
            proto: 1,
            server: "rustmush test".into(),
            features: vec![],
            panels: vec!["mail".into()],
        });
        assert_eq!(left.active_screen(), ScreenId(1));
    }

    /// 16b cut Q.4: the FIRST cockpit frame of a match opens the screen, and
    /// only the first — a pilot who walks back to the scene stays there.
    #[test]
    fn the_first_frame_of_a_match_opens_the_cockpit_and_the_next_one_does_not() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        app.feed_input(b"1");
        assert_eq!(app.active_screen(), ScreenId(1));
        for _ in 0..3 {
            app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        }
        assert_eq!(
            app.active_screen(),
            ScreenId(1),
            "the pilot left the screen; the next pulse must not drag them back"
        );
        // A NEW match opens it again, once.
        let mut next = arena_json(COCKPIT_FIXTURE);
        next["match"] = serde_json::json!(8);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, next.clone()));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        app.feed_input(b"1");
        app.apply_event(&arena_keyframe(COCKPIT_FEED, next.clone()));
        assert_eq!(app.active_screen(), ScreenId(1));

        // A cleared frame never opens anything — asked of the pilot who WAS
        // seated and walked back to the scene, because that is the only app
        // the cockpit screen is available on at all: on one that has never
        // been sent a frame the screen is gated away and the guard could not
        // fail if it tried.
        app.apply_event(&cockpit_cleared());
        assert_eq!(
            app.active_screen(),
            ScreenId(1),
            "the seat went; nothing drags the player onto the empty cockpit"
        );
        // 16b cut B.3 (`B-63`, 16c Q21): the tab goes with the seat. (It used
        // to stand for the rest of the session.)
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(
            !text.contains("0 cockpit"),
            "the tab went with the seat:\n{text}"
        );
        // And a seat taken again IS a new seating: the screen opens for it.
        app.apply_event(&arena_keyframe(COCKPIT_FEED, next));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
    }

    /// 16b cut Q.4: `arena.map` is PULLED. Holding a cockpit frame of match M
    /// and no map of M is the condition that asks, at most once every
    /// [`MAP_HEAL_LATCH_S`]; the ask stops once the map is held, and a new
    /// match drops the old ground and asks again at once.
    #[test]
    fn the_map_is_pulled_once_every_two_seconds_until_it_arrives() {
        let asks = |queries: &[(String, Value)]| -> usize {
            queries.iter().filter(|(n, _)| n == ARENA_MAP_FEED).count()
        };
        HEAL_CLOCK.store(100, Ordering::Relaxed);
        let mut app = TuiApp::new(caps());
        app.set_clock(heal_clock);
        app.set_manifest(arena_manifest());
        let frame = arena_json(COCKPIT_FIXTURE);

        // The first frame also opens the screen, whose `on_enter` asks for the
        // terrain too — and ONE terrain is enough: the self-heal takes that ask
        // as its own rather than pulling the whole surface twice.
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        assert_eq!(asks(&q), 1, "the first such frame asks exactly once: {q:?}");
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        assert_eq!(asks(&q), 0, "the same second does not ask again");
        HEAL_CLOCK.store(100 + MAP_HEAL_LATCH_S - 1, Ordering::Relaxed);
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        assert_eq!(asks(&q), 0, "inside the latch");
        HEAL_CLOCK.store(100 + MAP_HEAL_LATCH_S, Ordering::Relaxed);
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        assert_eq!(asks(&q), 1, "the answer never came: ask again");

        // **An answer that comes back WRONG does not buy a fresh ask.** The
        // bridge answers `arena.map` with `map_unseated()` whenever it cannot
        // reach the ground, and that payload carries no `match`: it is dropped
        // on sight, but the latch is the thing standing between a bridge that
        // is already struggling and one ask per pulse, so it is not re-armed.
        let unseated = serde_json::json!({
            "schema": { "id": ARENA_MAP_FEED, "version": 1, "shape": "terrain",
                        "retain": "latest" },
            "seated": false
        });
        for pulse in 0..3 {
            app.apply_event(&arena_keyframe(ARENA_MAP_FEED, unseated.clone()));
            let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
            assert_eq!(asks(&q), 0, "pulse {pulse} inside the latch: {q:?}");
            assert!(
                app.panels().get(ARENA_MAP_FEED).is_none(),
                "ground that is not this match's is dropped on sight"
            );
        }
        HEAL_CLOCK.store(100 + 2 * MAP_HEAL_LATCH_S, Ordering::Relaxed);
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        assert_eq!(asks(&q), 1, "and the latch still ages out: {q:?}");

        // The map arrives: the asking stops.
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        for _ in 0..3 {
            let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
            assert_eq!(asks(&q), 0, "the ground is held");
        }
        // Nor does walking off the screen and back: `on_enter` asks for the
        // frame, never for ground that is already held.
        let mut r = app.feed_input(b"1");
        assert_eq!(asks(&r.queries), 0, "{:?}", r.queries);
        r = app.feed_input(b"0");
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        assert_eq!(
            asks(&r.queries),
            0,
            "the ground does not move; re-entering never re-pulls it: {:?}",
            r.queries
        );

        // A new match is new ground: the old map goes and the ask re-arms on
        // that very frame, latch or no latch.
        let mut next = frame.clone();
        next["match"] = serde_json::json!(8);
        let q = app.apply_event(&arena_keyframe(COCKPIT_FEED, next));
        assert_eq!(asks(&q), 1, "a new match asks at once: {q:?}");
        assert!(
            app.panels().get(ARENA_MAP_FEED).is_none(),
            "the old ground was dropped"
        );

        // The cleared frame (`seated: false`) drops the map with the seat.
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        app.apply_event(&cockpit_cleared());
        assert!(app.panels().get(ARENA_MAP_FEED).is_none());
    }

    /// 16b cut B.2 (`B-59`): **the timer heal**. The arena pushes a frame only
    /// when one changes, so a pilot sitting still whose map answer was dropped
    /// has no frame to ask on; the pump's tick calls `heal_queries`, which asks
    /// under the same latch the frame path arms — once it has aged, once only,
    /// never for ground that is held and never once the seat is gone.
    #[test]
    fn the_heal_timer_asks_for_a_lost_map_without_a_frame() {
        let asks = |queries: &[(String, Value)]| -> usize {
            queries.iter().filter(|(n, _)| n == ARENA_MAP_FEED).count()
        };
        // A bystander holds no seated frame: the timer never asks.
        let mut bystander = arena_app();
        assert!(bystander.heal_queries(1_000_000).is_empty());

        let mut app = arena_app();
        let armed = app.now();
        // The first frame of match 7 asks (its screen's `on_enter`) and arms
        // the latch at `armed`; the answer lands, and is then lost.
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        app.panels.forget(ARENA_MAP_FEED);
        assert!(
            app.heal_queries(armed + MAP_HEAL_LATCH_S - 1).is_empty(),
            "inside the latch the frame path armed"
        );
        let q = app.heal_queries(armed + MAP_HEAL_LATCH_S + 1);
        assert_eq!(asks(&q), 1, "no frame came, the timer asks: {q:?}");
        assert_eq!(q.len(), 1, "and asks for nothing else: {q:?}");
        assert!(
            app.heal_queries(armed + MAP_HEAL_LATCH_S + 1).is_empty(),
            "the ask armed the latch: at once again, nothing"
        );
        // The ground arrives: held ground is never asked for, however late.
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        assert!(app.heal_queries(armed + 100 * MAP_HEAL_LATCH_S).is_empty());
        // The cleared frame ends the seat, and the timer with it.
        app.panels.forget(ARENA_MAP_FEED);
        app.apply_event(&cockpit_cleared());
        assert!(app.heal_queries(armed + 1000 * MAP_HEAL_LATCH_S).is_empty());
    }

    /// The clock [`the_pull_key_re_asks_the_cockpit_and_a_missing_map`]
    /// drives: its own cell, because [`HEAL_CLOCK`] belongs to one test.
    static PULL_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn pull_clock() -> i64 {
        PULL_CLOCK.load(Ordering::Relaxed)
    }

    /// 16b cut B.2 (`B-59`): `p`, pull, on the cockpit screen asks for the
    /// cockpit again always and for the map only when the ground is not held,
    /// sends nothing, and re-arms the latch on the map ask it made.
    #[test]
    fn the_pull_key_re_asks_the_cockpit_and_a_missing_map() {
        let names = |queries: &[(String, Value)]| -> Vec<String> {
            queries.iter().map(|(n, _)| n.clone()).collect()
        };
        PULL_CLOCK.store(100, Ordering::Relaxed);
        let mut app = TuiApp::new(caps());
        app.set_clock(pull_clock);
        app.set_manifest(arena_manifest());
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);

        // The answer never came; past the latch the pilot pulls.
        let at = 100 + MAP_HEAL_LATCH_S + 5;
        PULL_CLOCK.store(at, Ordering::Relaxed);
        let r = app.feed_input(b"p");
        assert!(r.submit.is_empty(), "a bare key sends nothing: {r:?}");
        let asked = names(&r.queries);
        assert!(asked.contains(&COCKPIT_FEED.to_string()), "{asked:?}");
        assert!(asked.contains(&ARENA_MAP_FEED.to_string()), "{asked:?}");
        // The pull's map ask armed the latch: the timer does not double it, and
        // asks again only once that latch has aged in turn.
        assert!(app.heal_queries(at).is_empty(), "re-armed by the pull");
        assert!(app.heal_queries(at + MAP_HEAL_LATCH_S - 1).is_empty());
        assert_eq!(app.heal_queries(at + MAP_HEAL_LATCH_S).len(), 1);

        // With the ground held the pull asks for the frame alone.
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        let r = app.feed_input(b"p");
        assert_eq!(names(&r.queries), vec![COCKPIT_FEED.to_string()]);
    }

    /// 16b cut B.2: a hello asks for the cockpit again even from screen 1, as
    /// the web's hello pass does — so a pilot who is no longer seated is
    /// answered the cleared frame, which ends the seat and the timer with it.
    /// This is the parity seam and it runs only where a hello arrives: a live
    /// session riding a world restart is handed none, and that case is
    /// [`a_dead_seat_is_ended_by_the_unseated_map_answer`].
    #[test]
    fn a_restarted_world_is_asked_for_the_cockpit_off_screen_zero() {
        let hello = |panels: Vec<String>| OutputEvent::Hello {
            proto: 1,
            server: "rustmush test".into(),
            features: vec![],
            panels,
        };
        let cockpit_asks = |queries: &[(String, Value)]| -> usize {
            queries.iter().filter(|(n, _)| n == COCKPIT_FEED).count()
        };
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        app.feed_input(b"1");
        assert_eq!(app.active_screen(), ScreenId(1));
        let q = app.apply_event(&hello(arena_manifest().panels));
        assert_eq!(cockpit_asks(&q), 1, "asked off screen 0: {q:?}");
        // The answer is the cleared frame: the seat ends, and nothing is owed.
        app.apply_event(&cockpit_cleared());
        assert!(app.heal_queries(app.now() + 1000).is_empty());

        // On screen 0 the screen's own `on_enter` already asks: once, not twice.
        let mut on_zero = arena_app();
        on_zero.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(on_zero.active_screen(), COCKPIT_SCREEN);
        let q = on_zero.apply_event(&hello(arena_manifest().panels));
        assert_eq!(cockpit_asks(&q), 1, "{q:?}");
        // A world that no longer publishes the feed is never asked for it.
        let mut gone = arena_app();
        gone.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        gone.feed_input(b"1");
        let q = gone.apply_event(&hello(vec!["mail".into()]));
        assert_eq!(cockpit_asks(&q), 0, "{q:?}");
    }

    /// 16b cut B.2: a live session that rides a world restart is handed no
    /// hello and no cleared frame, and its match died with the world — so the
    /// held seated frame would feed the timer heal a map ask on every tick for
    /// the rest of the session. The world's `seated: false` answer to that ask
    /// is the proof the seat is gone: it asks for the cockpit once, and the
    /// cleared answer ends the seat and the heal with it.
    #[test]
    fn a_dead_seat_is_ended_by_the_unseated_map_answer() {
        let unseated_map = || {
            arena_keyframe(
                ARENA_MAP_FEED,
                serde_json::json!({
                    "schema": { "id": ARENA_MAP_FEED, "version": 1, "retain": "latest" },
                    "seated": false
                }),
            )
        };
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        app.feed_input(b"1");
        // The timer's ask, answered by a world with no arena for this pilot:
        // one cockpit ask and nothing else — asked through the query seam
        // exactly as the pump submits every other answer's re-ask.
        let answer = OutputEvent::Reply {
            id: 0,
            event: Box::new(unseated_map()),
        };
        let q = app.apply_event(&answer);
        assert_eq!(
            q,
            vec![(
                COCKPIT_FEED.to_string(),
                Value::Object(serde_json::Map::new())
            )]
        );
        // The cleared answer ends the seat; the heal owes nothing any more, and
        // a late unseated map asks for nothing either.
        app.apply_event(&cockpit_cleared());
        assert!(app.heal_queries(app.now() + 1000).is_empty());
        assert!(app.apply_event(&unseated_map()).is_empty());

        // A bystander's hello-pass answer is `seated: false` too: no frame is
        // held, so it asks for nothing.
        let mut bystander = arena_app();
        assert!(bystander.apply_event(&unseated_map()).is_empty());
        // Nor does the seated pilot's own ground.
        let mut seated = arena_app();
        seated.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        let q = seated.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        assert!(q.is_empty(), "{q:?}");
    }

    /// The clock [`switching_onto_the_cockpit_arms_the_latch_on_its_map_ask`]
    /// drives: its own cell, as [`PULL_CLOCK`] is.
    static SWITCH_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn switch_clock() -> i64 {
        SWITCH_CLOCK.load(Ordering::Relaxed)
    }

    /// 16b cut B.2: pressing `0` while the ground is dark asks for the map
    /// through the screen's `on_enter`, and that ask arms the latch — or the
    /// pump's next tick, finding the latch long aged on a pilot who sat on
    /// screen 1, would pull the whole terrain a second time at once.
    #[test]
    fn switching_onto_the_cockpit_arms_the_latch_on_its_map_ask() {
        SWITCH_CLOCK.store(100, Ordering::Relaxed);
        let mut app = TuiApp::new(caps());
        app.set_clock(switch_clock);
        app.set_manifest(arena_manifest());
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        app.feed_input(b"1");
        assert_eq!(app.active_screen(), ScreenId(1));

        // Long after the frame's ask, the answer never came; back to screen 0.
        let at = 100 + 10 * MAP_HEAL_LATCH_S;
        SWITCH_CLOCK.store(at, Ordering::Relaxed);
        let r = app.feed_input(b"0");
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        assert_eq!(
            r.queries
                .iter()
                .filter(|(n, _)| n == ARENA_MAP_FEED)
                .count(),
            1,
            "{:?}",
            r.queries
        );
        assert!(app.heal_queries(at).is_empty(), "armed by the switch");
        assert!(app.heal_queries(at + MAP_HEAL_LATCH_S - 1).is_empty());
        assert_eq!(app.heal_queries(at + MAP_HEAL_LATCH_S).len(), 1);
    }

    /// 16b cut Q.4: the contacts pane is the one selectable pane on the screen
    /// — `Enter` prefills the highlighted contact's `LOCK`, `s` its `SCAN` —
    /// and neither sends anything until the player presses Enter.
    #[test]
    fn the_contacts_pane_prefills_lock_and_scan_and_sends_neither() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        let contacts = app
            .screen()
            .panes
            .iter()
            .position(|p| p.id == PaneId("contacts"))
            .expect("a contacts pane");
        for _ in 0..contacts {
            app.feed_input(b"\t");
        }
        assert_eq!(app.focused_pane(), Some(PaneId("contacts")));
        let r = app.feed_input(b"\r");
        assert!(r.submit.is_empty(), "nothing is sent: {r:?}");
        assert_eq!(app.input_text(), "lock BA");

        // Esc back to nav, then `s`: the same row, the other verb.
        app.feed_input(&[0x1b]);
        let r = app.feed_input(b"s");
        assert!(r.submit.is_empty(), "nothing is sent: {r:?}");
        assert_eq!(app.input_text(), "scan BA");
    }

    /// 16b cut Q.4: **the highlight stays on the contact the player picked**
    /// while the frame under it changes.
    ///
    /// The contacts pane is re-rendered on every arena pulse, and a contact
    /// that gains a close look (16b cut Q.2) grows a scan row UNDER it, pushing
    /// everything below down one. A highlight that was only an index would
    /// slide onto a neighbour — or onto a scan row, which is not selectable at
    /// all, and `Enter` would then fall back to the first contact and open the
    /// box with the wrong `lock`.
    #[test]
    fn the_highlighted_contact_survives_a_row_appearing_above_it() {
        let mut app = arena_app();
        // The scene as it stands before anyone has had a close look: nobody
        // carries a reading, so the rows are one per contact.
        let mut frame = arena_json(COCKPIT_FIXTURE);
        for contact in frame["contacts"].as_array_mut().expect("contacts") {
            contact["condition"] = Value::Null;
        }
        app.apply_event(&arena_keyframe(COCKPIT_FEED, frame.clone()));
        let contacts = app
            .screen()
            .panes
            .iter()
            .position(|p| p.id == PaneId("contacts"))
            .expect("a contacts pane");
        for _ in 0..contacts {
            app.feed_input(b"\t");
        }
        // Down twice: BA, RB, X3 — the third row.
        app.feed_input(b"\x1b[B");
        app.feed_input(b"\x1b[B");
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "lock X3");
        app.feed_input(&[0x1b]);

        // The next pulse: BA is told apart and grows a scan row, so the row at
        // index 2 is now BA's scan continuation rather than X3.
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        let r = app.feed_input(b"\r");
        assert!(r.submit.is_empty(), "nothing is sent: {r:?}");
        assert_eq!(
            app.input_text(),
            "lock X3",
            "the highlight follows the contact, not the index it sat at"
        );
        assert!(
            super::super::frame_text(&mut app, 120, 34).contains("X3"),
            "the row is still on the screen"
        );
    }

    /// 16b cut Q.4: `seated: false` — the last frame a pilot is sent — empties
    /// the cockpit. **16b cut B.3** (`B-63`): it takes the screen with it, so a
    /// pilot standing on it lands on the scene, and nothing of the seat is left
    /// to paint; the panes' own empty state is still the one line, for any
    /// caller that asks them.
    #[test]
    fn a_cleared_frame_empties_the_cockpit() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("Callsign RA"), "{text}");
        app.apply_event(&cockpit_cleared());
        assert_eq!(app.active_screen(), ScreenId(1), "off the screen that went");
        let text = super::super::frame_text(&mut app, 120, 34);
        for gone in [
            "Callsign RA",
            "Lock BA",
            "cannon",
            "BA  blue",
            "0 cockpit",
            super::super::render::NOT_IN_A_COCKPIT,
        ] {
            assert!(!text.contains(gone), "{gone:?} survived the clear:\n{text}");
        }
        let cockpit = app
            .screens
            .iter()
            .find(|s| s.id == COCKPIT_SCREEN)
            .cloned()
            .expect("the cockpit screen");
        for pane in &cockpit.panes {
            let rows = app.pane_rows(pane, Some((40, 10)));
            let text: Vec<String> = rows
                .rows
                .iter()
                .map(super::super::render::PaneRow::text)
                .collect();
            assert!(
                text.is_empty() || text == [super::super::render::NOT_IN_A_COCKPIT],
                "{}: {text:?}",
                pane.id.0
            );
        }
    }

    /// 16b cut Q.4: the drop order. Damage first, then the weapons, then the
    /// contacts; the map, the status and the notices ring survive every
    /// terminal the TUI paints at.
    ///
    /// **16b cut B.3** (`B-10`): at 80 by 24 only `damage` goes, and for HEIGHT
    /// — `22 + 36 + 22` is eighty columns, so the contacts pane, the one `Enter`
    /// prefills a `lock` from, stays. Asserted on the PAINTED frame, because
    /// `visible_panes` is asked with the body rect and a whole-terminal rect is
    /// not one.
    #[test]
    fn the_cockpit_degrades_damage_then_weapons_then_contacts() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        // It paints at every size the TUI is asked to paint at.
        for (w, h) in [(1u16, 1u16), (2, 1), (80, 1), (1, 24), (10, 3), (80, 24)] {
            let _ = super::super::frame_text(&mut app, w, h);
        }
        let survivors = |app: &TuiApp, w: u16, h: u16| -> Vec<&'static str> {
            app.visible_panes(Rect::new(0, 0, w, h))
                .iter()
                .map(|p| p.id.0)
                .collect()
        };
        assert_eq!(
            survivors(&app, 120, 28),
            vec!["map", "status", "damage", "notices", "weapons", "contacts"]
        );
        // Eighty by twenty-four, painted: every pane's title but damage's.
        let text = super::super::frame_text(&mut app, 80, 24);
        for title in ["status", "notices", "weapons", "contacts"] {
            assert!(
                text.contains(&format!("\u{250c}{title}")),
                "{title} is painted at 80x24:\n{text}"
            );
        }
        assert!(text.contains("\u{250c} map "), "the map, focused:\n{text}");
        assert!(
            !text.contains("\u{250c}damage"),
            "damage goes first, for height:\n{text}"
        );
        assert_eq!(survivors(&app, 30, 20), vec!["map"]);
    }

    /// **16b cut B.3**: a column's equal split stands where every pane has its
    /// `min_h`; where one is short, rows move to it from the pane with the most
    /// to spare, and the rects stay stacked and inside the column.
    #[test]
    fn a_column_split_gives_every_pane_its_own_minimum() {
        let col = Rect::new(0, 2, 22, 17);
        let split = |n: u16| -> Vec<Rect> {
            Layout::vertical((0..n).map(|_| Constraint::Ratio(1, u32::from(n))))
                .split(col)
                .to_vec()
        };
        // Fair already: untouched.
        let fair = split(2);
        assert_eq!(honour_min_heights(&fair, &[5, 5]), fair);
        // The cockpit at 80x24: status wants twelve (ten rows and a border),
        // notices five — the whole seventeen — and an even split gave the
        // status nine.
        let even = split(2);
        let fixed = honour_min_heights(&even, &[12, 5]);
        assert_eq!(fixed.iter().map(|r| r.height).collect::<Vec<_>>(), [12, 5]);
        assert_eq!(fixed.first().map(|r| r.y), Some(2));
        assert_eq!(fixed.get(1).map(|r| r.y), Some(14));
        assert!(fixed.iter().all(|r| r.x == 0 && r.width == 22));
        // Three panes, one short: the one with the most to spare gives.
        let three = split(3);
        let fixed = honour_min_heights(&three, &[10, 4, 1]);
        let heights: Vec<u16> = fixed.iter().map(|r| r.height).collect();
        assert_eq!(heights.iter().sum::<u16>(), 17);
        assert_eq!(heights.first(), Some(&10), "{heights:?}");
        assert!(
            heights.iter().zip([10, 4, 1]).all(|(h, m)| *h >= m),
            "{heights:?}"
        );
        // Not enough rows for everyone: the equal split is kept.
        assert_eq!(honour_min_heights(&even, &[12, 12]), even);
    }

    /// Paint `app` at `w` by `h` and hand back the buffer, styles and all.
    fn painted(app: &mut TuiApp, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut caps = app.caps();
        caps.width = w;
        caps.height = h;
        app.set_caps(caps);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).expect("backend");
        terminal.draw(|frame| app.draw(frame)).expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The row of `buf` that holds `needle` starting at or after column `from`,
    /// and the column it starts at.
    fn find_in(buf: &ratatui::buffer::Buffer, needle: &str, from: u16) -> Option<(u16, u16)> {
        let area = buf.area;
        for y in 0..area.height {
            let row: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
            let chars: Vec<char> = row.chars().collect();
            let want: Vec<char> = needle.chars().collect();
            for x in usize::from(from)..chars.len() {
                if chars.get(x..x + want.len()) == Some(&want[..]) {
                    return Some((u16::try_from(x).unwrap_or(0), y));
                }
            }
        }
        None
    }

    /// **16b cut B.3** (`B-10`): a panel row wider than its pane wraps into as
    /// many lines as it takes — nothing is cut at the pane's edge — and the
    /// highlight lies on EVERY line of the selected row.
    #[test]
    fn a_panel_row_wider_than_its_pane_wraps_and_keeps_its_highlight() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        // 80 columns: the Right column is 22 wide, twenty inside its borders,
        // and BA's row — `BA  blue  045°  far  450 m  1 s`, 31 characters —
        // is two lines there. The highlight rests on it (the first row).
        let buf = painted(&mut app, 80, 24);
        let (x, y) = find_in(&buf, "BA  blue  045\u{b0}  far", 58).expect("BA's first line");
        let (x2, y2) = find_in(&buf, "450 m  1 s", 58).expect("BA's second line");
        assert_eq!(y2, y + 1, "the wrap continues on the next line");
        assert_eq!(x2, x, "inside the same pane");
        for (cx, cy) in [(x, y), (x2, y2)] {
            assert!(
                buf[(cx, cy)].modifier.contains(Modifier::REVERSED),
                "the highlight lies on every line of the row ({cx}, {cy})"
            );
        }
        // The next contact's row is not highlighted.
        let (x3, y3) = find_in(&buf, "RB  red", 58).expect("RB's row");
        assert!(!buf[(x3, y3)].modifier.contains(Modifier::REVERSED));
        // And the scan row under BA, 43 characters, wraps rather than clips:
        // every one of its five words is painted.
        let text = super::super::frame_text(&mut app, 80, 24);
        for word in [
            "engine",
            "hull",
            "left_track/y",
            "right_track/r",
            "turret/d",
        ] {
            assert!(text.contains(word), "{word} was clipped:\n{text}");
        }
        // Moving the highlight keeps it on source rows: Down lands on RB, the
        // next SELECTABLE row, not on BA's wrapped line or its scan row.
        let contacts = app
            .screen()
            .panes
            .iter()
            .position(|p| p.id == PaneId("contacts"))
            .expect("a contacts pane");
        for _ in 0..contacts {
            app.feed_input(b"\t");
        }
        app.feed_input(b"\x1b[B");
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "lock RB");
    }

    /// **16b cut B.3** (`B-10`): the scroll follows the highlight over WRAPPED
    /// lines. At 80 by 24 the contacts pane holds seven lines and its rows take
    /// thirteen, so moving the highlight to the last contact scrolls: X4's
    /// FIRST line and its last are both in view and both reversed, and BA's
    /// row, at the top, has gone off the pane.
    #[test]
    fn the_scroll_follows_a_wrapped_highlight_down_the_pane() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        let contacts = app
            .screen()
            .panes
            .iter()
            .position(|p| p.id == PaneId("contacts"))
            .expect("a contacts pane");
        for _ in 0..contacts {
            app.feed_input(b"\t");
        }
        // BA → RB → X3 → X4, source rows all (BA's scan row is not a stop).
        for _ in 0..3 {
            app.feed_input(b"\x1b[B");
        }
        let buf = painted(&mut app, 80, 24);
        let (x, y) = find_in(&buf, "X4  unknown", 58).expect("X4's first line is in view");
        let (x2, y2) = find_in(&buf, "600 m", 58).expect("X4's last line is in view");
        assert_eq!(y2, y + 1, "the row's two lines, one under the other");
        assert!(x2 >= x);
        // `x` is the pane's first inner column: both lines start there.
        for (cx, cy) in [(x, y), (x, y2), (x2, y2)] {
            assert!(
                buf[(cx, cy)].modifier.contains(Modifier::REVERSED),
                "the highlight lies on every line of X4's row ({cx}, {cy})"
            );
        }
        assert!(
            find_in(&buf, "BA  blue", 58).is_none(),
            "the pane scrolled: BA's row is off the top"
        );
        // The row above X4 is X3's, and it is not highlighted.
        let (x3, y3) = find_in(&buf, "X3  unknown", 58).expect("X3's row above");
        assert!(y3 < y);
        assert!(!buf[(x3, y3)].modifier.contains(Modifier::REVERSED));
        app.feed_input(b"\r");
        assert_eq!(app.input_text(), "lock X4");
    }

    /// **16b cut B.3**: the map pane's rows are exactly the window's width in
    /// glyphs, so the wrap every panel pane now takes never folds the ground.
    #[test]
    fn the_map_pane_rows_never_wrap() {
        let mut app = arena_app();
        app.apply_event(&arena_keyframe(
            ARENA_MAP_FEED,
            arena_json(ARENA_MAP_FIXTURE),
        ));
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        let map = app
            .screen()
            .pane(PaneId("map"))
            .copied()
            .expect("a map pane");
        // (pane, heading rows, grid rows, legend rows) — the chrome rule.
        for (w, h, head, ground, legend) in [
            (34u16, 15u16, 1usize, 13usize, 1usize),
            (44, 20, 1, 18, 1),
            (54, 24, 1, 22, 1),
            (20, 3, 0, 2, 1),
            (20, 2, 0, 2, 0),
        ] {
            let rows = app.pane_rows(&map, Some((w, h)));
            assert_eq!(rows.rows.len(), head + ground + legend, "{w}x{h}");
            assert_eq!(rows.rows.len(), usize::from(h), "{w}x{h}: exactly the pane");
            for row in rows.rows.iter().skip(head).take(ground) {
                assert_eq!(
                    row.text().chars().count(),
                    usize::from(w),
                    "{w}x{h}: a grid row is the window's width"
                );
                assert_eq!(
                    wrap_spans(&row.line.spans, usize::from(w)).len(),
                    1,
                    "{w}x{h}: a map row wrapped: {:?}",
                    row.text()
                );
            }
        }
        // A crowd: twelve contacts around the pilot make a legend far wider
        // than a 34-column pane. It is cut to one line, so no row of the pane
        // wraps and the pane holds exactly its height — nothing to scroll.
        let mut crowded = arena_json(COCKPIT_FIXTURE);
        let (cx, cy) = (
            crowded["cell"][0].as_i64().unwrap_or(0),
            crowded["cell"][1].as_i64().unwrap_or(0),
        );
        let template = crowded["contacts"][0].clone();
        crowded["contacts"] = serde_json::Value::Array(
            (0..12)
                .map(|i| {
                    let mut c = template.clone();
                    c["designator"] = serde_json::json!(format!("X{}", i + 10));
                    c["cell"] = serde_json::json!([cx + i + 1, cy]);
                    c
                })
                .collect(),
        );
        app.apply_event(&arena_keyframe(COCKPIT_FEED, crowded));
        let rows = app.pane_rows(&map, Some((34, 15)));
        assert_eq!(rows.rows.len(), 15);
        for row in &rows.rows {
            assert_eq!(
                wrap_spans(&row.line.spans, 34).len(),
                1,
                "a row of the crowded map wrapped: {:?}",
                row.text()
            );
        }
        let legend = rows.rows.last().map(super::super::render::PaneRow::text);
        assert!(
            legend.as_deref().is_some_and(|l| l.ends_with(" more")),
            "the legend counts what it left off: {legend:?}"
        );
    }

    /// **16b cut B.3** (`B-63`, `T-63`, 16c Q21): the tab exists from a match's
    /// first seated frame to the cleared one. A pilot standing on it when the
    /// seat goes lands on the first screen; a pilot on another screen stays
    /// there; `0` is inert afterwards; and a new seat brings it back.
    #[test]
    fn the_tab_goes_with_the_seat() {
        let mut app = arena_app();
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains("0 cockpit"), "no seat, no tab:\n{text}");
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(text.contains("0 cockpit"), "{text}");
        // Standing on it when the seat goes: moved to the first screen.
        app.apply_event(&cockpit_cleared());
        assert_eq!(app.active_screen(), ScreenId(1));
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains("cockpit"), "the tab is gone:\n{text}");
        let r = app.feed_input(b"0");
        assert!(!r.redraw, "the digit is inert: {r:?}");
        assert_eq!(app.active_screen(), ScreenId(1));
        // A new seat brings it back (and opens it); a pilot who is on another
        // screen when that seat goes is left where they are.
        let mut next = arena_json(COCKPIT_FIXTURE);
        next["match"] = serde_json::json!(8);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, next));
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        app.feed_input(b"3");
        assert_eq!(app.active_screen(), ScreenId(3));
        app.apply_event(&cockpit_cleared());
        assert_eq!(app.active_screen(), ScreenId(3), "left where they were");
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains("0 cockpit"), "{text}");
    }

    /// The notices pane's rows, as text, off a painted-size render.
    fn notice_rows(app: &TuiApp, pane: (u16, u16)) -> Vec<String> {
        let spec = app
            .screens
            .iter()
            .find(|s| s.id == COCKPIT_SCREEN)
            .and_then(|s| s.pane(PaneId("notices")))
            .copied()
            .expect("a notices pane");
        app.pane_rows(&spec, Some(pane))
            .rows
            .iter()
            .map(super::super::render::PaneRow::text)
            .collect()
    }

    /// **16b cut B.3** (`B-64`, `B-80`'s reset on the TUI): the notices pane
    /// is the frame's notices composed from their fields — never the
    /// scrollback — and it starts each match clean and ends with the seat.
    #[test]
    fn the_notices_ring_resets_on_a_new_match_and_on_the_cleared_frame() {
        let mut app = arena_app();
        // Chatter before the seat is the scrollback's, never the pane's.
        app.apply_event(&OutputEvent::Sys(RichText::plain("Bob has connected.")));
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        assert_eq!(
            notice_rows(&app, (20, 10)),
            ["X4 new contact", "BA section destroyed: turret"]
        );
        // The next pulse of the same match adds its own and keeps the rest.
        let mut pulse = arena_json(COCKPIT_FIXTURE);
        pulse["notices"] = serde_json::json!([
            { "kind": "locked", "designator": "RB", "friendly": true }
        ]);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, pulse));
        assert_eq!(
            notice_rows(&app, (40, 10)).last().map(String::as_str),
            Some("RB (FRIENDLY) locked")
        );
        assert_eq!(notice_rows(&app, (40, 10)).len(), 3);
        // A read verb's re-emitted frame carries `notices: []`: nothing rings.
        let mut mirror = arena_json(COCKPIT_FIXTURE);
        mirror["notices"] = serde_json::json!([]);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, mirror));
        assert_eq!(notice_rows(&app, (40, 10)).len(), 3);
        // Match 8's first frame empties the ring before its own lines land.
        let mut next = arena_json(COCKPIT_FIXTURE);
        next["match"] = serde_json::json!(8);
        next["notices"] = serde_json::json!([
            { "kind": "acquired", "designator": "X1", "friendly": false }
        ]);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, next));
        assert_eq!(notice_rows(&app, (40, 10)), ["X1 new contact"]);
        // The cleared frame ends it.
        app.apply_event(&cockpit_cleared());
        assert!(app.panels().cockpit_notices().is_empty());
        // Seated again with nothing rung: the pane says so.
        let mut quiet = arena_json(COCKPIT_FIXTURE);
        quiet["notices"] = serde_json::json!([]);
        app.apply_event(&arena_keyframe(COCKPIT_FEED, quiet));
        assert_eq!(
            notice_rows(&app, (40, 10)),
            [super::super::render::NOTHING_YET]
        );
    }

    /// **16b cut B.3** (the TUI half of B.1's reply ring, Q20): a `Sys` line
    /// that arrives while seated is the ring's last line — `Locking BA.` lands
    /// where `lock BA` was typed — and one that arrives before any frame is not.
    /// The bridge's copy of a frame's own notice (tagged `arena-notice`) is left
    /// out: the ring composed that notice from the frame already. So is a `Sys`
    /// wrapped in a `Reply`, the answer to one of the app's own queries.
    #[test]
    fn a_reply_lands_on_the_notices_pane_while_seated() {
        let mut app = arena_app();
        app.apply_event(&OutputEvent::Sys(RichText::plain("Before the seat.")));
        assert!(app.panels().cockpit_notices().is_empty());
        app.apply_event(&arena_keyframe(COCKPIT_FEED, arena_json(COCKPIT_FIXTURE)));
        app.apply_event(&OutputEvent::Sys(RichText::tag("arena", "Locking BA.")));
        assert_eq!(
            notice_rows(&app, (40, 10)).last().map(String::as_str),
            Some("Locking BA.")
        );
        // A `Sys` answering one of the app's OWN queries is not the pilot's
        // reply and does not ring (the web's `#routeReply` never rings one):
        // the refusal of a background ask, a subscribe's acknowledgement.
        let before = app.panels().cockpit_notices().len();
        app.apply_event(&OutputEvent::Reply {
            id: 0,
            event: Box::new(OutputEvent::Sys(RichText::tag(
                "error",
                "Slow down: too many queries.",
            ))),
        });
        assert_eq!(
            app.panels().cockpit_notices().len(),
            before,
            "an answer to a query the pilot never typed is not rung"
        );
        // What a bare key's read verb answers arrives bare, and rings.
        app.apply_event(&OutputEvent::Sys(RichText::tag("arena", "No lock held.")));
        assert_eq!(
            notice_rows(&app, (40, 10)).last().map(String::as_str),
            Some("No lock held.")
        );
        let before = app.panels().cockpit_notices().len();
        app.apply_event(&OutputEvent::Sys(RichText::tag(
            NOTICE_SENTENCE_TAG,
            "New contact X4: bearing 030\u{b0}, far.",
        )));
        assert_eq!(
            app.panels().cockpit_notices().len(),
            before,
            "the notice's own sentence is not rung twice"
        );
        // Both still reach the scrollback on screen 1.
        app.feed_input(b"1");
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(
            text.contains("Locking BA.") && text.contains("New contact X4"),
            "{text}"
        );
        // The pane shows the TAIL that fits: at three rows, the last three.
        let tail = notice_rows(&app, (40, 3));
        assert_eq!(tail.len(), 3);
        assert_eq!(tail.last().map(String::as_str), Some("No lock held."));
        // After the cleared frame nothing rings.
        app.apply_event(&cockpit_cleared());
        app.apply_event(&OutputEvent::Sys(RichText::plain("You climb out.")));
        assert!(app.panels().cockpit_notices().is_empty());
    }
}
