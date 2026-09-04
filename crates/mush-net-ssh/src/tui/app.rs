//! [`TuiApp`] — the per-connection ratatui application state (design M15
//! D3/D4/D5 WHO-rail slice).
//!
//! Holds the scrollback ring, the WHO rail, and the input line, applies each
//! [`OutputEvent`] to that state, decodes raw input bytes into edits /
//! navigation, and paints one coalesced frame. It is pure state + rendering:
//! it owns no transport and never blocks, so the output pump can drive it
//! synchronously under the connection lock and write the resulting bytes
//! after the lock drops (invariant T2).
//!
//! Layout (POC): a WHO rail on the right (~[`WHO_RAIL_W`] cols), the main
//! frame filling the rest as scrollback + a bordered input line at the
//! bottom. A switchable secondary pane (mail / intel) lands in task 2.

use std::collections::VecDeque;

use mush_ansi::AnsiTheme;
use mush_proto::{ClientCaps, InputMode, OutputEvent, ViewCursor, ViewFrameDisposition, WhoEntry};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::render::{
    render_event, render_intel_panel, render_legal_actions_panel, render_mail_panel,
    render_who_rail,
};

/// Scrollback ring depth (logical lines, pre-wrap).
const SCROLLBACK_CAP: usize = 1000;

/// WHO rail width in columns (design D5: ~20-24).
pub const WHO_RAIL_W: u16 = 22;

/// Secondary-pane (mail / intel) width in columns.
const SECONDARY_W: u16 = 34;

/// Input-line block height (border + one text row).
const INPUT_H: u16 = 3;

/// What a decoded input burst asks the transport to do. Every burst that
/// changed visible state also asks for a repaint.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InputResult {
    /// Repaint the frame.
    pub redraw: bool,
    /// Completed command lines to forward to `hooks.line`, in order. A single
    /// input burst can carry more than one line (a multi-line paste), so this
    /// accumulates rather than collapsing to the last — dropping earlier lines
    /// would silently swallow typed commands.
    pub submit: Vec<String>,
    /// Leave the TUI (the detach hotkey or `@notui`, resolved by the caller).
    pub detach: bool,
    /// A read-only widget query the transport should submit via `hooks.query`
    /// (e.g. `"mail.list"` / `"intel.list"` when a pane is opened). Read-only,
    /// so it never stamps idle.
    pub query: Option<&'static str>,
}

/// Which widget the switchable secondary pane shows (design M15 D5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SecondaryPane {
    /// No secondary pane — the main frame + WHO rail only.
    #[default]
    Hidden,
    /// The `@mail` widget (fed by the `mail.list` query keyframe).
    Mail,
    /// The `+intel` widget (fed by the `intel.list` query keyframe).
    Intel,
    /// Viewer-safe actions (fed by the `legal.actions` query keyframe).
    Actions,
}

/// Parser state for CSI escape sequences (`ESC [ ... final`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum EscState {
    Ground,
    Esc,
    Csi,
}

/// Per-connection ratatui application state.
pub struct TuiApp {
    scrollback: VecDeque<Line<'static>>,
    who: Vec<Line<'static>>,
    /// Raw input bytes (UTF-8, possibly mid-multibyte between feeds).
    input: Vec<u8>,
    input_mode: InputMode,
    prompt: String,
    /// Rows scrolled up from the bottom (0 = following the tail).
    scroll_off: usize,
    esc: EscState,
    /// Whether the previous ground byte was a carriage return, so the `\n` of
    /// a CRLF pair is swallowed rather than submitting a second (empty) line
    /// (mirrors the [`LineEditor`](crate::editor) CR/LF discipline).
    last_was_cr: bool,
    /// Accumulated CSI parameter bytes.
    csi: Vec<u8>,
    theme: AnsiTheme,
    caps: ClientCaps,
    /// Which widget the secondary pane currently shows.
    secondary: SecondaryPane,
    /// Latest `mail.list` keyframe data (`None` until the query replies).
    mail: Option<serde_json::Value>,
    /// Latest `intel.list` keyframe data (`None` until the query replies).
    intel: Option<serde_json::Value>,
    /// Latest `legal.actions` keyframe data (`None` until the query replies).
    actions: Option<serde_json::Value>,
    /// Last safely applied `legal.actions` semantic-view cursor.
    actions_cursor: Option<ViewCursor>,
    /// Suppresses query storms while a replacement keyframe is outstanding.
    actions_resync_pending: bool,
}

impl TuiApp {
    /// A fresh app sized for `caps`.
    #[must_use]
    pub fn new(caps: ClientCaps) -> TuiApp {
        TuiApp {
            scrollback: VecDeque::new(),
            who: Vec::new(),
            input: Vec::new(),
            input_mode: InputMode::Normal,
            prompt: String::new(),
            scroll_off: 0,
            esc: EscState::Ground,
            last_was_cr: false,
            csi: Vec::new(),
            theme: AnsiTheme::default(),
            caps,
            secondary: SecondaryPane::Hidden,
            mail: None,
            intel: None,
            actions: None,
            actions_cursor: None,
            actions_resync_pending: false,
        }
    }

    /// Update capabilities (color depth / geometry) on resize.
    pub fn set_caps(&mut self, caps: ClientCaps) {
        self.caps = caps;
    }

    /// The current color depth, for the backend geometry sync.
    #[must_use]
    pub fn caps(&self) -> ClientCaps {
        self.caps
    }

    /// Apply one event to the app state. `Who` (bare or `Reply`-wrapped)
    /// refreshes the rail, `Prompt` drives the input line, everything else
    /// with a scrollback rendering appends to history.
    pub fn apply_event(&mut self, event: &OutputEvent) -> Option<&'static str> {
        if let Some(entries) = who_entries(event) {
            self.set_who(entries);
            return None;
        }
        if let OutputEvent::Prompt(spec) = unwrap_reply(event) {
            self.input_mode = spec.mode;
            self.prompt = spec
                .text
                .as_ref()
                .map(|t| sanitize_prompt(&t.to_plain_text()))
                .unwrap_or_default();
            return None;
        }
        if let OutputEvent::View(view) = unwrap_reply(event)
            && view.view() == "legal.actions"
        {
            match view.disposition("legal.actions", 1, self.actions_cursor.as_ref()) {
                ViewFrameDisposition::Replace => {
                    self.actions = Some(view.data().clone());
                    self.actions_cursor = Some(view.cursor());
                    self.actions_resync_pending = false;
                    return None;
                }
                ViewFrameDisposition::IgnoreStale => return None,
                // This first feed intentionally emits keyframes only. A valid
                // delta still cannot be guessed at generically: until the
                // feed registers a merge contract, fail closed and rehydrate.
                ViewFrameDisposition::ApplyDelta | ViewFrameDisposition::ResyncRequired => {
                    self.actions = None;
                    if !self.actions_resync_pending {
                        self.actions_resync_pending = true;
                        return Some("legal.actions");
                    }
                    return None;
                }
            }
        }
        // Widget keyframes: route the mail/intel/legal-action panel data to
        // the secondary pane (they never touch scrollback).
        if let OutputEvent::Panel { feed, data, .. } = unwrap_reply(event) {
            match feed.as_str() {
                "mail" => self.mail = Some(data.clone()),
                "intel" => self.intel = Some(data.clone()),
                "legal.actions" => self.actions = Some(data.clone()),
                _ => {}
            }
            return None;
        }
        let following = self.scroll_off == 0;
        let lines = render_event(event, &self.caps, &self.theme);
        for line in lines {
            if self.scrollback.len() == SCROLLBACK_CAP {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(line);
        }
        // Stay pinned to the tail if the viewer had not scrolled up.
        if following {
            self.scroll_off = 0;
        }
        None
    }

    fn set_who(&mut self, entries: &[WhoEntry]) {
        self.who = render_who_rail(entries, &self.theme, self.caps.color);
    }

    /// Decode a burst of raw input bytes into edits / navigation.
    pub fn feed_input(&mut self, bytes: &[u8]) -> InputResult {
        let mut result = InputResult::default();
        for &b in bytes {
            match self.esc {
                EscState::Ground => self.feed_ground(b, &mut result),
                EscState::Esc => {
                    if b == b'[' {
                        self.esc = EscState::Csi;
                        self.csi.clear();
                    } else {
                        // Lone ESC or an unsupported sequence: drop it.
                        self.esc = EscState::Ground;
                    }
                }
                EscState::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.handle_csi(b, &mut result);
                        self.esc = EscState::Ground;
                    } else {
                        self.csi.push(b);
                    }
                }
            }
        }
        result
    }

    fn feed_ground(&mut self, b: u8, result: &mut InputResult) {
        let was_cr = self.last_was_cr;
        self.last_was_cr = false;
        match b {
            b'\r' => {
                self.last_was_cr = true;
                self.submit_line(result);
            }
            b'\n' => {
                // Swallow the LF of a CRLF pair so one Enter yields exactly one
                // submit; a bare LF (some clients) still submits. Without this
                // a CRLF burst would submit the typed line on the CR, then an
                // empty line on the LF, dropping the command downstream.
                if !was_cr {
                    self.submit_line(result);
                }
            }
            0x7f | 0x08 => {
                pop_char(&mut self.input);
                result.redraw = true;
            }
            0x15 => {
                // Ctrl-U: clear the input line.
                self.input.clear();
                result.redraw = true;
            }
            0x1d => {
                // Ctrl-] : detach the TUI (return to line mode).
                result.detach = true;
            }
            0x09 => {
                // Tab: cycle the switchable secondary pane
                // (hidden -> mail -> intel -> hidden). Opening a widget
                // submits its read-only refresh query.
                self.cycle_secondary(result);
            }
            0x1b => self.esc = EscState::Esc,
            // Other C0 control bytes are ignored; printable bytes (including
            // UTF-8 continuation/lead bytes) accumulate.
            b if b >= 0x20 => {
                self.input.push(b);
                result.redraw = true;
            }
            _ => {}
        }
    }

    /// Push the current input buffer as a completed command line and clear it.
    fn submit_line(&mut self, result: &mut InputResult) {
        let line = String::from_utf8_lossy(&self.input).into_owned();
        self.input.clear();
        self.scroll_off = 0;
        result.submit.push(line);
        result.redraw = true;
    }

    /// Advance the secondary pane one step and, when a widget is opened, ask
    /// the transport to submit that widget's read-only refresh query so its
    /// data is current (design M15 D5). Toggling back to `Hidden` submits
    /// nothing.
    fn cycle_secondary(&mut self, result: &mut InputResult) {
        self.secondary = match self.secondary {
            SecondaryPane::Hidden => SecondaryPane::Mail,
            SecondaryPane::Mail => SecondaryPane::Intel,
            SecondaryPane::Intel => SecondaryPane::Actions,
            SecondaryPane::Actions => SecondaryPane::Hidden,
        };
        result.query = match self.secondary {
            SecondaryPane::Mail => Some("mail.list"),
            SecondaryPane::Intel => Some("intel.list"),
            SecondaryPane::Actions => Some("legal.actions"),
            SecondaryPane::Hidden => None,
        };
        result.redraw = true;
    }

    fn handle_csi(&mut self, final_byte: u8, result: &mut InputResult) {
        let params = std::str::from_utf8(&self.csi).unwrap_or("");
        match final_byte {
            // PgUp / PgDn arrive as `ESC [ 5 ~` / `ESC [ 6 ~`.
            b'~' => match params {
                "5" => {
                    self.scroll_off = self.scroll_off.saturating_add(10);
                    result.redraw = true;
                }
                "6" => {
                    self.scroll_off = self.scroll_off.saturating_sub(10);
                    result.redraw = true;
                }
                _ => {}
            },
            // Up / Down arrows scroll one line.
            b'A' => {
                self.scroll_off = self.scroll_off.saturating_add(1);
                result.redraw = true;
            }
            b'B' => {
                self.scroll_off = self.scroll_off.saturating_sub(1);
                result.redraw = true;
            }
            _ => {}
        }
    }

    /// Paint one frame into `frame`. Columns, left to right: the main frame
    /// (scrollback + input), then — as width allows — the switchable secondary
    /// pane (mail / intel) and the WHO rail. Panes drop out on narrow
    /// terminals rather than crowding the main frame (invariant T4).
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        // Right rail when there is room; otherwise the main frame takes all.
        let rail_w = if area.width > WHO_RAIL_W + 10 {
            WHO_RAIL_W
        } else {
            0
        };
        // The secondary pane only claims space when it is open AND there is
        // room for it beside the main frame and rail.
        let sec_w =
            if self.secondary != SecondaryPane::Hidden && area.width > rail_w + SECONDARY_W + 20 {
                SECONDARY_W
            } else {
                0
            };
        let [main, secondary_area, rail] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(sec_w),
            Constraint::Length(rail_w),
        ])
        .areas(area);
        let input_h = INPUT_H.min(area.height);
        let [scroll_area, input_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(input_h)]).areas(main);

        self.draw_scrollback(frame, scroll_area);
        self.draw_input(frame, input_area);
        if sec_w > 0 {
            self.draw_secondary(frame, secondary_area);
        }
        if rail_w > 0 {
            self.draw_rail(frame, rail);
        }
    }

    fn draw_secondary(&self, frame: &mut Frame<'_>, area: Rect) {
        let (title, lines) = match self.secondary {
            SecondaryPane::Mail => (
                "@mail",
                self.mail
                    .as_ref()
                    .map(|d| render_mail_panel(d, &self.theme, self.caps.color)),
            ),
            SecondaryPane::Intel => (
                "+intel",
                self.intel
                    .as_ref()
                    .map(|d| render_intel_panel(d, &self.theme, self.caps.color)),
            ),
            SecondaryPane::Actions => (
                "+actions",
                self.actions
                    .as_ref()
                    .map(|d| render_legal_actions_panel(d, &self.theme, self.caps.color)),
            ),
            // Not drawn when hidden (the caller gates on sec_w > 0).
            SecondaryPane::Hidden => ("", None),
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let lines = lines
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| vec![Line::from(dim_span("(loading...)"))]);
        let para = Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }

    fn draw_scrollback(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title("rustmush");
        let inner_w = block.inner(area).width;
        let inner_h = block.inner(area).height as usize;
        // Estimate wrapped-row count (Wrap{trim:false} breaks on width) to
        // anchor the view at the bottom, honoring any scroll-up offset.
        let total: usize = self
            .scrollback
            .iter()
            .map(|l| wrapped_rows(l.width(), inner_w))
            .sum();
        let max_scroll = total.saturating_sub(inner_h);
        self.scroll_off = self.scroll_off.min(max_scroll);
        let scroll_y = max_scroll.saturating_sub(self.scroll_off);
        let text = Text::from(self.scrollback.iter().cloned().collect::<Vec<_>>());
        let para = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y.min(u16::MAX as usize) as u16, 0));
        frame.render_widget(para, area);
    }

    fn draw_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title("input");
        let inner = block.inner(area);
        let shown = self.shown_input();
        let prefix = format!("{}{}", self.prompt, shown);
        let para = Paragraph::new(Line::from(prefix.clone())).block(block);
        frame.render_widget(para, area);
        if inner.width > 0 && inner.height > 0 {
            let used = display_width(&prefix).min(inner.width.saturating_sub(1) as usize);
            frame.set_cursor_position(Position {
                x: inner.x + used as u16,
                y: inner.y,
            });
        }
    }

    fn draw_rail(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title("WHO");
        let lines = if self.who.is_empty() {
            vec![Line::from(dim_span("(loading...)"))]
        } else {
            self.who.clone()
        };
        let para = Paragraph::new(Text::from(lines)).block(block);
        frame.render_widget(para, area);
    }

    /// The input as it should appear, honoring the current [`InputMode`].
    fn shown_input(&self) -> String {
        let text = String::from_utf8_lossy(&self.input);
        match self.input_mode {
            InputMode::Normal => text.into_owned(),
            InputMode::Masked => "*".repeat(text.chars().count()),
            InputMode::NoEcho => String::new(),
        }
    }
}

/// A dim placeholder span (owned).
fn dim_span(text: &str) -> ratatui::text::Span<'static> {
    ratatui::text::Span::styled(
        text.to_string(),
        Style::default().add_modifier(Modifier::DIM),
    )
}

/// Rows a logical line of display width `w` occupies in a `width`-wide pane
/// under `Wrap{trim:false}` (a width-boundary break estimate).
fn wrapped_rows(w: usize, width: u16) -> usize {
    if width == 0 {
        return 1;
    }
    w.div_ceil(width as usize).max(1)
}

/// Unicode display width of a string (control chars already excluded).
fn display_width(s: &str) -> usize {
    // POC: char count is a good enough caret estimate; the renderer strips
    // control chars, and wide-glyph caret drift is cosmetic here.
    s.chars().count()
}

/// Prompt text collapsed to a single line (control chars / newlines out).
fn sanitize_prompt(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Pop one whole UTF-8 char off the end of a byte buffer.
fn pop_char(buf: &mut Vec<u8>) {
    while let Some(&b) = buf.last() {
        buf.pop();
        // Stop once we removed a non-continuation (lead/ASCII) byte.
        if b & 0xc0 != 0x80 {
            break;
        }
    }
}

/// Unwrap a `Reply` to its inner event (one level).
fn unwrap_reply(event: &OutputEvent) -> &OutputEvent {
    match event {
        OutputEvent::Reply { event, .. } => event,
        other => other,
    }
}

/// The WHO entries an event carries, unwrapping a `Reply`; `None` otherwise.
fn who_entries(event: &OutputEvent) -> Option<&[WhoEntry]> {
    match unwrap_reply(event) {
        OutputEvent::Who(entries) => Some(entries),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ClientKind, ObjRef, PromptSpec, RichText};

    fn caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: mush_proto::ColorDepth::Ansi16,
            width: 80,
            height: 24,
            utf8: true,
            structured: false,
        }
    }

    #[test]
    fn enter_submits_and_clears_input() {
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"say hi");
        assert!(r.redraw);
        assert!(r.submit.is_empty());
        let r = app.feed_input(b"\r");
        assert_eq!(r.submit, vec!["say hi".to_string()]);
        assert!(app.input.is_empty());
    }

    #[test]
    fn crlf_enter_submits_one_line_not_an_empty_second() {
        // A client that sends CRLF for Enter in one burst must submit the
        // typed line exactly once — never a phantom empty line that would
        // overwrite / follow the real command downstream.
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"say hi\r\n");
        assert_eq!(r.submit, vec!["say hi".to_string()]);
        assert!(app.input.is_empty());
    }

    #[test]
    fn crlf_split_across_bursts_still_swallows_the_lf() {
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"pose waves\r");
        assert_eq!(r.submit, vec!["pose waves".to_string()]);
        // The trailing LF arrives in the next packet; it must be swallowed.
        let r = app.feed_input(b"\n");
        assert!(r.submit.is_empty());
    }

    #[test]
    fn bare_lf_still_submits() {
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"look\n");
        assert_eq!(r.submit, vec!["look".to_string()]);
    }

    #[test]
    fn multiline_paste_yields_every_line() {
        // A two-line paste (CRLF-separated) must dispatch both lines in order.
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"line one\r\nline two\r\n");
        assert_eq!(
            r.submit,
            vec!["line one".to_string(), "line two".to_string()]
        );
        assert!(app.input.is_empty());
    }

    #[test]
    fn backspace_removes_one_char() {
        let mut app = TuiApp::new(caps());
        app.feed_input("abc".as_bytes());
        app.feed_input(&[0x7f]);
        assert_eq!(String::from_utf8_lossy(&app.input), "ab");
    }

    #[test]
    fn backspace_pops_whole_multibyte_char() {
        let mut app = TuiApp::new(caps());
        app.feed_input("é".as_bytes()); // 2 bytes
        assert_eq!(app.input.len(), 2);
        app.feed_input(&[0x7f]);
        assert!(app.input.is_empty());
    }

    #[test]
    fn ctrl_rbracket_detaches() {
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(&[0x1d]);
        assert!(r.detach);
    }

    #[test]
    fn pgup_pgdn_adjust_scroll() {
        let mut app = TuiApp::new(caps());
        let r = app.feed_input(b"\x1b[5~");
        assert!(r.redraw);
        assert_eq!(app.scroll_off, 10);
        app.feed_input(b"\x1b[6~");
        assert_eq!(app.scroll_off, 0);
    }

    #[test]
    fn masked_prompt_hides_input() {
        let mut app = TuiApp::new(caps());
        app.apply_event(&OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Masked,
            text: Some(RichText::plain("pw>")),
        }));
        app.feed_input(b"secret");
        assert_eq!(app.shown_input(), "******");
        assert_eq!(app.prompt, "pw>");
    }

    #[test]
    fn who_reply_populates_rail() {
        let mut app = TuiApp::new(caps());
        let who = OutputEvent::Who(vec![WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 3,
            conn_s: 9,
            location: None,
            doing: None,
        }]);
        app.apply_event(&OutputEvent::Reply {
            id: 1,
            event: Box::new(who),
        });
        assert!(!app.who.is_empty());
    }

    #[test]
    fn narrative_appends_scrollback() {
        let mut app = TuiApp::new(caps());
        app.apply_event(&OutputEvent::Narrative {
            kind: mush_proto::NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Nick")),
            text: RichText::plain("hello"),
        });
        assert_eq!(app.scrollback.len(), 1);
    }

    #[test]
    fn scrollback_ring_is_capped() {
        let mut app = TuiApp::new(caps());
        for i in 0..(SCROLLBACK_CAP + 50) {
            app.apply_event(&OutputEvent::Sys(RichText::plain(format!("line {i}"))));
        }
        assert_eq!(app.scrollback.len(), SCROLLBACK_CAP);
    }

    #[test]
    fn tab_cycles_secondary_pane_and_requests_queries() {
        let mut app = TuiApp::new(caps());
        assert_eq!(app.secondary, SecondaryPane::Hidden);
        // Hidden -> Mail: submits the mail.list refresh.
        let r = app.feed_input(&[0x09]);
        assert_eq!(app.secondary, SecondaryPane::Mail);
        assert_eq!(r.query, Some("mail.list"));
        assert!(r.redraw);
        // Mail -> Intel: submits the intel.list refresh.
        let r = app.feed_input(&[0x09]);
        assert_eq!(app.secondary, SecondaryPane::Intel);
        assert_eq!(r.query, Some("intel.list"));
        // Intel -> Actions: submits the legal.actions refresh.
        let r = app.feed_input(&[0x09]);
        assert_eq!(app.secondary, SecondaryPane::Actions);
        assert_eq!(r.query, Some("legal.actions"));
        // Actions -> Hidden: nothing to submit.
        let r = app.feed_input(&[0x09]);
        assert_eq!(app.secondary, SecondaryPane::Hidden);
        assert_eq!(r.query, None);
    }

    #[test]
    fn mail_panel_keyframe_is_stored() {
        let mut app = TuiApp::new(caps());
        let data = serde_json::json!({ "folder": "inbox", "entries": [] });
        app.apply_event(&OutputEvent::Panel {
            feed: mush_proto::SmolStr::new_static("mail"),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: data.clone(),
        });
        assert_eq!(app.mail.as_ref(), Some(&data));
        assert!(app.intel.is_none());
        // A Panel keyframe never lands in scrollback.
        assert!(app.scrollback.is_empty());
    }

    #[test]
    fn intel_panel_keyframe_via_reply_is_stored() {
        let mut app = TuiApp::new(caps());
        let data = serde_json::json!({ "count": 0, "records": [] });
        let panel = OutputEvent::Panel {
            feed: mush_proto::SmolStr::new_static("intel"),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: data.clone(),
        };
        app.apply_event(&OutputEvent::Reply {
            id: 3,
            event: Box::new(panel),
        });
        assert_eq!(app.intel.as_ref(), Some(&data));
        assert!(app.scrollback.is_empty());
    }

    #[test]
    fn legal_actions_view_accepts_keyframe_and_requests_resync_on_gap() {
        let mut app = TuiApp::new(caps());
        let data = serde_json::json!([{"presentation": {"verb": "brace"}}]);
        let keyframe =
            mush_proto::ViewEnvelope::keyframe("legal.actions", 1, 7, 3, data.clone()).unwrap();
        assert_eq!(app.apply_event(&OutputEvent::View(keyframe)), None);
        assert_eq!(app.actions.as_ref(), Some(&data));

        let gap = mush_proto::ViewEnvelope::delta(
            "legal.actions",
            1,
            7,
            5,
            4,
            serde_json::json!({"remove": 0}),
        )
        .unwrap();
        assert_eq!(
            app.apply_event(&OutputEvent::View(gap.clone())),
            Some("legal.actions")
        );
        assert!(app.actions.is_none(), "stale action buttons must disappear");
        assert_eq!(
            app.apply_event(&OutputEvent::View(gap)),
            None,
            "one missing keyframe must not create a resync query storm"
        );

        let replacement =
            mush_proto::ViewEnvelope::keyframe("legal.actions", 1, 7, 6, data.clone()).unwrap();
        assert_eq!(app.apply_event(&OutputEvent::View(replacement)), None);
        assert_eq!(app.actions.as_ref(), Some(&data));
        assert!(!app.actions_resync_pending);
    }

    #[test]
    fn secondary_pane_renders_widget_content() {
        let mut app = TuiApp::new(caps());
        app.feed_input(&[0x09]); // open mail
        app.apply_event(&OutputEvent::Panel {
            feed: mush_proto::SmolStr::new_static("mail"),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "folder": "inbox",
                "entries": [
                    { "index": 1, "status": "unread", "from": { "name": "Zoe" }, "subject": "Hi" }
                ],
            }),
        });
        let backend =
            super::super::backend::SshBackend::new(120, 24, mush_proto::ColorDepth::Ansi16);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let bytes = term.backend_mut().take_output();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("mail"), "mail pane title missing: {text:?}");
        assert!(text.contains("Zoe"), "mail entry sender missing");
    }

    #[test]
    fn draw_on_tiny_terminal_does_not_panic() {
        let mut app = TuiApp::new(caps());
        app.apply_event(&OutputEvent::Sys(RichText::plain("hi")));
        for (w, h) in [(1u16, 1u16), (2, 1), (80, 1), (1, 24), (10, 3)] {
            let backend =
                super::super::backend::SshBackend::new(w, h, mush_proto::ColorDepth::Ansi16);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| app.draw(f)).unwrap();
        }
    }

    #[test]
    fn draw_with_secondary_open_on_tiny_terminal_does_not_panic() {
        let mut app = TuiApp::new(caps());
        app.feed_input(&[0x09]); // open the mail pane
        app.apply_event(&OutputEvent::Panel {
            feed: mush_proto::SmolStr::new_static("mail"),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: serde_json::json!({ "folder": "inbox", "entries": [] }),
        });
        // On narrow widths the pane simply drops out; must not panic.
        for (w, h) in [(1u16, 1u16), (10, 3), (40, 5), (120, 24)] {
            let backend =
                super::super::backend::SshBackend::new(w, h, mush_proto::ColorDepth::Ansi16);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| app.draw(f)).unwrap();
        }
    }
}
