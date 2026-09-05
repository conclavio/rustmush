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
//! `1`–`9` switch screens, `Tab`/`Shift-Tab` move pane focus, arrows and
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
use mush_proto::{ClientCaps, ExitRef, InputMode, OutputEvent, PanelFrameKind, RoomView, WhoEntry};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use serde_json::Value;

use super::keys::{Key, KeyParser};
use super::panels::{ChanMsg, PanelOutcome, PanelStore, rich_plain};
use super::render::{
    Entry, GUTTER_W, PaneRows, RenderCtx, dim_span_owned, legend_line, render_backlog_rows,
    render_channels_rows, render_event, render_rolls_rows, render_sheet_rows, render_vitals_rows,
    render_who_rail, status_line, tab_strip, vitals_line, wrap_spans,
};
use super::screen::{
    BorderRole, GLOBAL_KEYS, JUMP_KEYS, KeyAction, LocalAction, PaneContent, PaneId, PaneSlot,
    PaneSpec, RESERVED_EXIT_KEYS, Screen, ScreenId, builtin_screens, validate_screens,
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
/// manifest from `Hello` and the character from `SessionInfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuiManifest {
    /// Panel feed ids the server can publish (`Hello.panels`).
    pub panels: Vec<String>,
    /// The attached character's name.
    pub character: Option<String>,
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

#[derive(Clone, Copy, Debug, Default)]
struct PaneUi {
    /// Rows scrolled from the top (text panes).
    scroll: usize,
    /// The highlighted selectable row's index into the pane's rows.
    selected: usize,
}

/// A client-local command (`/clear`, `/help`, `/notui`) the palette lists.
const LOCAL_COMMANDS: [(&str, &str); 3] = [
    ("/clear", "empty the scrollback"),
    ("/help", "the key help"),
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
    who: Vec<WhoEntry>,
    room: Option<RoomView>,
    exit_keys: Vec<(char, String)>,
    panels: PanelStore,
    manifest: TuiManifest,
    mail_unread: usize,
    commands: Vec<CommandEntry>,
    channel_aliases: Vec<String>,
    palette: Option<Palette>,
    help: bool,
    thinking: Vec<(String, i64)>,
    composing: Vec<(String, i64)>,
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
            who: Vec::new(),
            room: None,
            exit_keys: Vec::new(),
            panels: PanelStore::new(),
            manifest: TuiManifest::default(),
            mail_unread: 0,
            commands: Vec::new(),
            channel_aliases: Vec::new(),
            palette: None,
            help: false,
            thinking: Vec::new(),
            composing: Vec::new(),
        })
    }

    /// Use a fixed clock (tests, golden frames).
    pub fn set_clock(&mut self, clock: fn() -> i64) {
        self.clock = clock;
    }

    /// Record what the server announced before the TUI opened.
    pub fn set_manifest(&mut self, manifest: TuiManifest) {
        self.manifest = manifest;
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

    /// The scrollback entries, oldest first.
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

    fn now(&self) -> i64 {
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
            .copied()
            .unwrap_or_default()
    }

    // ---- events -----------------------------------------------------------

    /// The queries to submit right after entering the TUI.
    #[must_use]
    pub fn entry_queries(&self) -> Vec<(String, Value)> {
        let empty = || Value::Object(serde_json::Map::new());
        let mut q: Vec<(String, Value)> = vec![
            ("who".to_string(), empty()),
            ("commands.list".to_string(), empty()),
            ("comsys.state".to_string(), empty()),
            ("scenes".to_string(), empty()),
        ];
        for feed in &self.manifest.panels {
            if feed.ends_with(".vitals") {
                q.push((feed.clone(), empty()));
            }
        }
        q.extend(self.screen_queries());
        q
    }

    /// The active screen's own hydration queries.
    fn screen_queries(&self) -> Vec<(String, Value)> {
        let screen = self.screen();
        let mut q: Vec<(String, Value)> = screen
            .on_enter
            .iter()
            .map(|(n, a)| ((*n).to_string(), a.clone()))
            .collect();
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

    /// Apply one event to the app state. Returns read-only queries the app
    /// wants submitted (a resync after an unmergeable delta, a hydration
    /// after a `Hello`).
    pub fn apply_event(&mut self, event: &OutputEvent) -> Vec<(String, Value)> {
        let now = self.now();
        let mut queries = Vec::new();
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
                if spec.mode != InputMode::Normal && self.mode == Mode::Nav {
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
                    mush_proto::NarrativeKind::Emit | mush_proto::NarrativeKind::Spoof => plain,
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
            OutputEvent::MailNotice { unread, .. } => {
                self.mail_unread = usize::try_from(*unread).unwrap_or(0);
            }
            OutputEvent::Composing { actor, active } => {
                self.composing.retain(|(n, _)| n != &actor.name);
                if *active {
                    self.composing.push((sanitize(&actor.name), now));
                }
                return queries;
            }
            OutputEvent::Hello { panels, .. } => {
                self.manifest.panels.clone_from(panels);
                let empty = || Value::Object(serde_json::Map::new());
                for feed in panels {
                    if feed.ends_with(".vitals") {
                        queries.push((feed.clone(), empty()));
                    }
                }
            }
            OutputEvent::SessionInfo { character, .. } => {
                self.manifest.character = Some(sanitize(&character.name));
            }
            OutputEvent::Resumed { .. } => {
                // The mode resets on a resume (D-I-17).
                self.leave_input(true);
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
        }
        self.scrollback.push_back(entry);
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
            Key::Char(c @ '1'..='9') => self.switch_screen(c as u8 - b'0', r),
            Key::Enter => self.enter_input(None),
            Key::Char(c @ ('"' | ':' | ';' | '\\')) => self.enter_input(Some(c.to_string())),
            Key::Char('/') => {
                self.enter_input(Some("/".to_string()));
                self.open_palette();
            }
            Key::Esc => self.palette = None,
            Key::Tab => self.move_focus(1),
            Key::BackTab => self.move_focus(-1),
            Key::Up => self.scroll_focused(-1),
            Key::Down => self.scroll_focused(1),
            Key::PgUp => self.scroll_focused(-(PAGE as isize)),
            Key::PgDn => self.scroll_focused(PAGE as isize),
            Key::Char('g') => self.jump_focused(false),
            Key::Char('G') => self.jump_focused(true),
            Key::Char('?') | Key::Alt('?') => self.help = true,
            Key::Alt(c @ '1'..='9') => self.switch_screen(c as u8 - b'0', r),
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
            LocalAction::Refresh => r.queries.extend(self.screen_queries()),
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
                self.scrollback.clear();
                self.scroll_off = 0;
            }
        }
    }

    fn input_key(&mut self, key: Key, r: &mut InputResult) {
        if let Key::Alt(c) = key {
            match c {
                '1'..='9' => self.switch_screen(c as u8 - b'0', r),
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
                    self.leave_input(false);
                    return;
                }
                let line = self.line.submit(self.input_mode == InputMode::Normal);
                self.submit_line(line, r);
            }
            Key::Esc => self.leave_input(true),
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
            Key::PgUp => self.scroll_off = self.scroll_off.saturating_add(PAGE),
            Key::PgDn => self.scroll_off = self.scroll_off.saturating_sub(PAGE),
            Key::Tab => self.complete(),
            _ => r.redraw = false,
        }
    }

    /// A submitted line: a client-local `/command`, else the sticky-mode
    /// form of a game line (D-I-8: an explicit trigger or a channel alias
    /// verb always wins).
    fn submit_line(&mut self, line: String, r: &mut InputResult) {
        self.scroll_off = 0;
        if let Some(local) = line.strip_prefix('/') {
            match local.trim() {
                "clear" => self.local_action(LocalAction::ClearScrollback, r),
                "help" => self.help = true,
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

    fn switch_screen(&mut self, digit: u8, r: &mut InputResult) {
        let id = ScreenId(digit);
        if !self.screens.iter().any(|s| s.id == id) {
            r.redraw = false;
            return;
        }
        if self.active != id {
            self.active = id;
            self.focus = 0;
            self.help = false;
            r.queries.extend(self.screen_queries());
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
    fn scroll_focused(&mut self, delta: isize) {
        let Some(spec) = self.screen().panes.get(self.focus).copied() else {
            return;
        };
        match spec.content {
            PaneContent::Scrollback => {
                let off = self.scroll_off as isize - delta;
                self.scroll_off = off.max(0) as usize;
            }
            _ if spec.selectable => {
                let rows = self.pane_rows(&spec);
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
                let ui = self.pane_ui(spec.id);
                let pos = selectable
                    .iter()
                    .position(|&i| i == ui.selected)
                    .unwrap_or(0) as isize;
                let next = (pos + delta).clamp(0, selectable.len() as isize - 1) as usize;
                self.pane_ui_mut(spec.id).selected = selectable[next];
            }
            _ => {
                let ui = self.pane_ui_mut(spec.id);
                let off = ui.scroll as isize + delta;
                ui.scroll = off.max(0) as usize;
            }
        }
    }

    fn jump_focused(&mut self, bottom: bool) {
        let Some(spec) = self.screen().panes.get(self.focus).copied() else {
            return;
        };
        match spec.content {
            PaneContent::Scrollback => {
                self.scroll_off = if bottom { 0 } else { usize::MAX / 2 };
            }
            _ if spec.selectable => {
                let rows = self.pane_rows(&spec);
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
                    self.pane_ui_mut(spec.id).selected = i;
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
        let rows = self.pane_rows(&spec);
        let ui = self.pane_ui(spec.id);
        rows.rows
            .get(ui.selected)
            .filter(|r| r.selectable)
            .cloned()
            .or_else(|| rows.rows.into_iter().find(|r| r.selectable))
    }

    fn selected_prefill(&self) -> Option<String> {
        self.selected_row().and_then(|r| r.prefill)
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

    /// The channel highlighted on the active screen: the focused channel
    /// pane's highlight, else the first channel any channel pane lists
    /// (the IC pane is often empty; the OOC pane then answers).
    fn selected_channel(&self) -> Option<String> {
        for i in self.channel_pane_indexes() {
            let spec = self.screen().panes[i];
            let rows = self.pane_rows(&spec);
            let ui = self.pane_ui(spec.id);
            let key = rows
                .rows
                .get(ui.selected)
                .and_then(|r| r.row_key.clone())
                .or_else(|| rows.rows.iter().find_map(|r| r.row_key.clone()));
            if key.is_some() {
                return key;
            }
        }
        None
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
        let rows = self.pane_rows(&spec);
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
        let cur = self.pane_ui(spec.id).selected;
        let pos = selectable.iter().position(|&s| s == cur);
        let next = pos.map_or(0, |p| (p + 1) % selectable.len());
        self.pane_ui_mut(spec.id).selected = selectable[next];
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
        }
    }

    /// The rows a pane shows right now.
    fn pane_rows(&self, spec: &PaneSpec) -> PaneRows {
        let ctx = self.render_ctx();
        match spec.content {
            PaneContent::Scrollback | PaneContent::Who => PaneRows::default(),
            PaneContent::Panel { feed, render } => match self.panels.get(feed) {
                Some(data) => render(data, &ctx),
                None => loading(),
            },
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
                Paragraph::new(vitals_line(&vitals, self.caps.utf8)),
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
            .screens
            .iter()
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
            for (spec, rect) in in_slot.iter().zip(areas.iter()) {
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
        let title = if focused {
            format!(" {} ", spec.title)
        } else {
            spec.title.to_string()
        };
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
                let rows = self.pane_rows(&spec);
                let ui = self.pane_ui(spec.id);
                let total = rows.rows.len();
                let h = usize::from(inner.height);
                let mut scroll = ui.scroll.min(total.saturating_sub(h));
                if spec.selectable && ui.selected < total {
                    if ui.selected < scroll {
                        scroll = ui.selected;
                    } else if ui.selected >= scroll + h {
                        scroll = ui.selected + 1 - h;
                    }
                }
                let lines: Vec<Line<'static>> = rows
                    .rows
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| {
                        if spec.selectable && i == ui.selected && r.selectable {
                            r.line
                                .style(Style::default().add_modifier(Modifier::REVERSED))
                        } else {
                            r.line
                        }
                    })
                    .collect();
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
        for entry in &self.scrollback {
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

    fn draw_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines: Vec<Line<'static>> = vec![
            Line::from(Span::styled(
                format!("{} screen", self.screen().name),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("1-9 screens   Tab focus   Up/Down PgUp/PgDn scroll   g/G top/bottom"),
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

/// The letters a room may always use on the scene screen.
#[must_use]
pub fn reserved_exit_keys() -> &'static [char] {
    &RESERVED_EXIT_KEYS
}

/// The plain text of a serialized `RichText`, for tests and rows.
#[must_use]
pub fn plain_of(v: &Value) -> String {
    rich_plain(v)
}

/// Whether `feed` is one of the manifest's feeds.
#[must_use]
pub fn manifest_has(manifest: &TuiManifest, feed: &str) -> bool {
    manifest.panels.iter().any(|f| f == feed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{
        ClientKind, ColorDepth, NarrativeKind, ObjRef, PromptSpec, RichText, SmolStr,
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
        assert_eq!(app.scrollback_len(), 0);
        let r = app.feed_input(b"/notui\r");
        assert!(r.detach);
        let r = app.feed_input(&[0x1d]);
        assert!(r.detach);
        let r = app.feed_input(b"/bogus\r");
        assert!(r.submit.is_empty());
        assert_eq!(
            app.scrollback_len(),
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
        let text = super::super::frame_text(&mut app, 120, 34);
        assert!(!text.contains("[ Health"));
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
        for screen in 1..=7u8 {
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
        for screen in 1..=7u8 {
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
            panels: vec!["wod.vitals".into(), "wod.sheet".into(), "mail".into()],
            character: Some("Nick".into()),
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
}
