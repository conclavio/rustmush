//! Read-only inspection of saved trace events, without running a bot or a model.
use crate::{
    Result,
    trace::{Query, Trace},
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use std::{
    collections::BTreeSet,
    io::{IsTerminal, Write, stdin, stdout},
    time::Duration,
};

const DETAIL_BYTES: usize = 64_000;
const SEARCH_BYTES: usize = 1_024;

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// The source trace and the authored query are never modified by this viewer.
pub fn run(trace: &Trace, query: &Query) -> Result<()> {
    if !stdin().is_terminal() || !stdout().is_terminal() {
        return Err("trace TUI requires interactive terminal stdin and stdout".into());
    }
    let mut view = View::new(trace, query)?;
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    loop {
        terminal.draw(|frame| view.draw(frame))?;
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if view.key(key) {
                        break;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Search {
    text: String,
    cursor: usize,
}
impl Search {
    fn left(&mut self) {
        self.cursor = self.text[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(index, _)| index);
    }
    fn right(&mut self) {
        self.cursor += self.text[self.cursor..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
    }
    fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !c.is_control()
                    && self.text.len() + c.len_utf8() <= SEARCH_BYTES =>
            {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                let end = self.cursor;
                self.left();
                self.text.drain(self.cursor..end);
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let start = self.cursor;
                self.right();
                self.text.drain(start..self.cursor);
                self.cursor = start;
            }
            _ => {}
        }
    }
}

struct View<'a> {
    trace: &'a Trace,
    base: Vec<usize>,
    base_label: String,
    visible: Vec<usize>,
    selected: usize,
    list_top: usize,
    page_size: usize,
    actors: Vec<Option<String>>,
    kinds: Vec<String>,
    // Zero means all available values; subsequent positions select one value.
    actor_choice: usize,
    kind_choice: usize,
    search: String,
    editing: Option<Search>,
    related_anchor: Option<usize>,
    notice: &'static str,
    detail_event: Option<usize>,
    detail: String,
    detail_width: u16,
    detail_lines: Vec<String>,
    detail_scroll: usize,
    detail_page: usize,
}

impl<'a> View<'a> {
    fn new(trace: &'a Trace, query: &Query) -> Result<Self> {
        let base = trace.matching(query)?;
        let actors = base
            .iter()
            .map(|index| trace.events[*index].actor.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let kinds = base
            .iter()
            .map(|index| trace.events[*index].kind.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let selected = query.offset.min(base.len().saturating_sub(1));
        let base_label = format!(
            "Authored query: actors={} kinds={} text={} time={:?}..{:?} ms; all matches shown",
            query.actors.len(),
            query.kinds.len(),
            if query.contains.is_some() {
                "yes"
            } else {
                "no"
            },
            query.from_ms,
            query.to_ms
        );
        let mut view = Self {
            trace,
            visible: base.clone(),
            base,
            base_label,
            selected,
            list_top: selected,
            page_size: 10,
            actors,
            kinds,
            actor_choice: 0,
            kind_choice: 0,
            search: String::new(),
            editing: None,
            related_anchor: None,
            notice: "UI filters narrow the authored query; its limit applies only to exports.",
            detail_event: None,
            detail: String::new(),
            detail_width: 0,
            detail_lines: Vec::new(),
            detail_scroll: 0,
            detail_page: 10,
        };
        view.update_detail();
        Ok(view)
    }

    fn current(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    fn refilter(&mut self) {
        let current = self.current();
        let related = self.related_anchor.map(|anchor| {
            let mut indices: BTreeSet<_> = self.trace.related(anchor).into_iter().collect();
            indices.insert(anchor);
            indices
        });
        let needle = self.search.to_lowercase();
        self.visible = self
            .base
            .iter()
            .copied()
            .filter(|index| {
                let event = &self.trace.events[*index];
                (self.actor_choice == 0 || event.actor == self.actors[self.actor_choice - 1])
                    && (self.kind_choice == 0 || event.kind == self.kinds[self.kind_choice - 1])
                    && related.as_ref().is_none_or(|set| set.contains(index))
                    && (needle.is_empty() || event.raw.to_string().to_lowercase().contains(&needle))
            })
            .collect();
        self.selected = current
            .and_then(|index| {
                self.visible
                    .iter()
                    .position(|candidate| *candidate == index)
            })
            .unwrap_or(0);
        self.list_top = 0;
        self.update_detail();
    }

    fn update_detail(&mut self) {
        let current = self.current();
        if current == self.detail_event && !self.detail.is_empty() {
            return;
        }
        self.detail_event = current;
        self.detail_scroll = 0;
        self.detail_width = 0;
        self.detail = match current {
            None => {
                "No matching events. F6 clears UI filters; the authored query remains in effect."
                    .to_owned()
            }
            Some(index) => {
                let event = &self.trace.events[index];
                let (raw, truncated) = bounded_json(&event.raw);
                format!(
                    "Source line: {} | actor: {} | kind: {}\nElapsed ms: {:?} | timestamp ms: {:?}\nSequence: {:?} | request ID: {:?} | command sequence: {:?}\nActual recorded event; no reconstructed historical state.\n\n{}{}",
                    event.line,
                    clean(event.actor.as_deref().unwrap_or("(none)"), false),
                    clean(&event.kind, false),
                    event.elapsed_ms,
                    event.timestamp_ms,
                    event.seq,
                    event.request_id,
                    event.command_sequence,
                    clean(&raw, true),
                    if truncated {
                        "\n\n[Display truncated at 64,000 bytes. The complete event remains in the source trace and JSON export.]"
                    } else {
                        ""
                    }
                )
            }
        };
    }

    /// Returns true only when the runner should leave this read-only screen.
    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if let Some(editor) = self.editing.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    self.editing = None;
                    self.notice = "Search canceled; existing filters retained.";
                }
                KeyCode::Enter => {
                    self.search = std::mem::take(&mut editor.text);
                    self.editing = None;
                    self.notice =
                        "Literal case-insensitive JSON search applied within authored query.";
                    self.refilter();
                }
                _ => editor.key(key),
            }
            return false;
        }
        match key.code {
            KeyCode::Char('q' | 'Q') => return true,
            KeyCode::Char('/') => {
                self.editing = Some(Search {
                    text: self.search.clone(),
                    cursor: self.search.len(),
                });
            }
            KeyCode::F(2) => {
                self.actor_choice = (self.actor_choice + 1) % (self.actors.len() + 1);
                self.notice = "Actor filter changed within authored query.";
                self.refilter();
            }
            KeyCode::F(3) => {
                self.kind_choice = (self.kind_choice + 1) % (self.kinds.len() + 1);
                self.notice = "Kind filter changed within authored query.";
                self.refilter();
            }
            KeyCode::F(4) => {
                if self.related_anchor.take().is_some() {
                    self.notice = "ID association filter cleared; other filters retained.";
                } else {
                    self.related_anchor = self.current();
                    self.notice = match self.related_anchor.map(|index| &self.trace.events[index]) {
                        Some(event)
                            if event.request_id.is_none() && event.command_sequence.is_none() =>
                        {
                            "Selected event has no request/command IDs; showing only that event."
                        }
                        Some(_) => {
                            "Same-actor request/command ID association, one hop; not proof of causality."
                        }
                        None => "No selected event to associate. Clear UI filters with F6.",
                    };
                }
                self.refilter();
            }
            KeyCode::F(6) => {
                self.actor_choice = 0;
                self.kind_choice = 0;
                self.search.clear();
                self.related_anchor = None;
                self.notice = "UI filters cleared; authored query remains in effect.";
                self.refilter();
            }
            KeyCode::PageUp if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(self.detail_page);
            }
            KeyCode::PageDown if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.detail_scroll = self
                    .detail_scroll
                    .saturating_add(self.detail_page)
                    .min(self.detail_lines.len().saturating_sub(self.detail_page));
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(self.visible.len().saturating_sub(1));
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(self.page_size),
            KeyCode::PageDown => {
                self.selected = self
                    .selected
                    .saturating_add(self.page_size)
                    .min(self.visible.len().saturating_sub(1));
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.visible.len().saturating_sub(1),
            _ => {}
        }
        self.update_detail();
        false
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 42 || area.height < 14 {
            frame.render_widget(
                Paragraph::new("Offline trace viewer (read only)\nEnlarge terminal to 42 x 14.\nQ / Ctrl+C exit."),
                area,
            );
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(5),
            Constraint::Min(4),
            Constraint::Length(5),
        ])
        .split(area);
        let actor = if self.actor_choice == 0 {
            "all".to_owned()
        } else {
            clean(
                self.actors[self.actor_choice - 1]
                    .as_deref()
                    .unwrap_or("(none)"),
                false,
            )
        };
        let kind = if self.kind_choice == 0 {
            "all".to_owned()
        } else {
            clean(&self.kinds[self.kind_choice - 1], false)
        };
        frame.render_widget(
            Paragraph::new(format!(
                "Offline trace viewer | READ ONLY | {:?}\n{} | revision {}\n{}\nUI actor: {actor} | kind: {kind} | search: {}\n{} / {} authored matches | selected {}{}",
                self.trace.format,
                clean(&self.trace.source.display().to_string(), false),
                clean(&self.trace.revision, false),
                self.base_label,
                clean(&self.search, false),
                self.visible.len(),
                self.base.len(),
                if self.visible.is_empty() { 0 } else { self.selected + 1 },
                self.related_anchor.map_or_else(String::new, |index| format!(
                    " | ID associates: line {} (not causality)", self.trace.events[index].line
                )),
            )),
            rows[0],
        );
        let body = if area.width >= 100 {
            Layout::horizontal([Constraint::Percentage(36), Constraint::Percentage(64)])
                .split(rows[1])
        } else {
            Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(rows[1])
        };
        self.draw_list(frame, body[0]);
        self.draw_detail(frame, body[1]);
        let final_line = self.editing.as_ref().map_or_else(
            || self.notice.to_owned(),
            |editor| {
                // A visible caret also works on terminals where the hardware cursor is hidden.
                format!(
                    "/{}▏{}  [Enter apply | Esc cancel | Ctrl+U clear]",
                    clean(&editor.text[..editor.cursor], false),
                    clean(&editor.text[editor.cursor..], false)
                )
            },
        );
        frame.render_widget(
            Paragraph::new(format!(
                "Up/Down PgUp/PgDn Home/End: events | Ctrl+PgUp/PgDn: detail\nF2 actor | F3 kind | F4 ID associates | / search | F6 clear UI filters\nQ / Ctrl+C exit | All filters retain authored constraints\nSaved events only; this view does not connect, act, or call a model.\n{final_line}"
            )),
            rows[2],
        );
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Events (source order) ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.page_size = usize::from(inner.height).max(1);
        if self.selected < self.list_top {
            self.list_top = self.selected;
        } else if self.selected >= self.list_top.saturating_add(self.page_size) {
            self.list_top = self.selected + 1 - self.page_size;
        }
        self.list_top = self
            .list_top
            .min(self.visible.len().saturating_sub(self.page_size));
        let lines: Vec<_> = self
            .visible
            .iter()
            .enumerate()
            .skip(self.list_top)
            .take(self.page_size)
            .map(|(position, index)| {
                let event = &self.trace.events[*index];
                let text = format!(
                    "{} L{} {} | {}",
                    if position == self.selected { ">" } else { " " },
                    event.line,
                    clean(event.actor.as_deref().unwrap_or("(none)"), false),
                    clean(&event.kind, false),
                );
                if position == self.selected {
                    Line::styled(text, Style::default().fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::raw(text)
                }
            })
            .collect();
        frame.render_widget(
            if lines.is_empty() {
                Paragraph::new("No matching events. F6 clears UI filters.")
            } else {
                Paragraph::new(lines)
            },
            inner,
        );
    }

    fn draw_detail(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Recorded event / JSON (reflowed) ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.detail_page = usize::from(inner.height).max(1);
        if self.detail_width != inner.width || self.detail_lines.is_empty() {
            self.detail_width = inner.width;
            self.detail_lines = reflow(&self.detail, usize::from(inner.width).max(1));
        }
        self.detail_scroll = self
            .detail_scroll
            .min(self.detail_lines.len().saturating_sub(self.detail_page));
        let lines: Vec<_> = self
            .detail_lines
            .iter()
            .skip(self.detail_scroll)
            .take(self.detail_page)
            .map(|line| Line::raw(line.as_str()))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn clean(text: &str, multiline: bool) -> String {
    crate::terminal::plain(text)
        .chars()
        .map(|c| {
            if (c.is_control() && !(multiline && c == '\n'))
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                '�'
            } else {
                c
            }
        })
        .collect()
}

/// Stop pretty serialization at the display limit, including indentation growth.
fn bounded_json(value: &serde_json::Value) -> (String, bool) {
    struct Limited {
        bytes: Vec<u8>,
        truncated: bool,
    }
    impl Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let remaining = DETAIL_BYTES.saturating_sub(self.bytes.len());
            if bytes.len() > remaining {
                self.bytes.extend_from_slice(&bytes[..remaining]);
                self.truncated = true;
                return Err(std::io::Error::other("trace detail display limit reached"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Limited {
        bytes: Vec::new(),
        truncated: false,
    };
    if serde_json::to_writer_pretty(&mut output, value).is_err() && !output.truncated {
        return ("Unable to format this event.".to_owned(), false);
    }
    let prefix = match std::str::from_utf8(&output.bytes) {
        Ok(text) => text,
        Err(error) => {
            std::str::from_utf8(&output.bytes[..error.valid_up_to()]).expect("UTF-8 valid prefix")
        }
    };
    (prefix.to_owned(), output.truncated)
}

/// Pre-wrap the bounded display text so scrolling is capped at its actual height.
fn reflow(text: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for line in text.split('\n') {
        let mut buffer = String::new();
        let mut used = 0;
        for c in line.chars() {
            let cell_width = Span::raw(c.to_string()).width();
            if used > 0 && used + cell_width > width {
                result.push(std::mem::take(&mut buffer));
                used = 0;
            }
            buffer.push(c);
            used += cell_width;
        }
        result.push(buffer);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{Entry, Format};
    use serde_json::json;
    use std::path::PathBuf;

    fn fixture() -> Trace {
        let values = [
            ("scout", "model-input", Some(7), None),
            ("scout", "model-reply", Some(7), Some(8)),
            ("scout", "command", Some(9), Some(8)),
            ("speaker", "model-reply", Some(7), None),
            ("scout", "model-input", Some(10), Some(8)),
            ("scout", "observation", None, None),
        ];
        Trace {
            source: PathBuf::from("fixture/joined.jsonl"),
            revision: "abcdef1234".to_owned(),
            format: Format::Joined,
            events: values
                .into_iter()
                .enumerate()
                .map(|(index, (actor, kind, request_id, command_sequence))| Entry {
                    line: index + 1,
                    actor: Some(actor.to_owned()),
                    kind: kind.to_owned(),
                    elapsed_ms: Some(index as u64 * 100),
                    timestamp_ms: Some(1_000 + index as u64),
                    seq: Some(index as u64),
                    request_id,
                    command_sequence,
                    raw: json!({
                        "actor": actor,
                        "kind": kind,
                        "text": if index == 4 { "Élodie 花 bought a rose" } else { "Marco asked about a rose" },
                        "request_id": request_id,
                        "command_sequence": command_sequence,
                    }),
                })
                .collect(),
        }
    }

    fn key(view: &mut View<'_>, code: KeyCode) -> bool {
        view.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn screen(view: &mut View<'_>, width: u16, height: u16) -> String {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| view.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn authored_scope_survives_filters_and_navigation_ignores_export_limit() {
        let trace = fixture();
        let query = Query {
            actors: vec!["scout".to_owned()],
            kinds: vec!["model-input".to_owned(), "model-reply".to_owned()],
            offset: 1,
            limit: 1,
            ..Query::default()
        };
        let mut view = View::new(&trace, &query).unwrap();
        assert_eq!(view.visible, [0, 1, 4]);
        assert_eq!(view.current(), Some(1));
        key(&mut view, KeyCode::End);
        assert_eq!(view.current(), Some(4));
        key(&mut view, KeyCode::Down);
        assert_eq!(view.current(), Some(4));
        key(&mut view, KeyCode::Home);
        key(&mut view, KeyCode::Up);
        assert_eq!(view.current(), Some(0));
        key(&mut view, KeyCode::F(2));
        key(&mut view, KeyCode::F(3));
        assert_eq!(view.visible, [0, 4]);
        key(&mut view, KeyCode::F(6));
        assert_eq!(view.visible, [0, 1, 4]);
        assert_eq!(query.offset, 1);
        assert_eq!(query.limit, 1);
        let rendered = screen(&mut view, 130, 30);
        assert!(rendered.contains("all matches shown"));
        assert!(rendered.contains("READ ONLY"));
    }

    #[test]
    fn related_view_is_one_hop_same_actor_and_cannot_escape_authored_scope() {
        let trace = fixture();
        let mut view = View::new(&trace, &Query::default()).unwrap();
        key(&mut view, KeyCode::F(4));
        assert_eq!(view.visible, [0, 1]);
        assert!(
            !view.visible.contains(&2),
            "request association cannot follow another command ID"
        );
        assert!(
            !view.visible.contains(&3),
            "IDs do not join different actors"
        );
        key(&mut view, KeyCode::End);
        assert_eq!(view.related_anchor, Some(0));
        key(&mut view, KeyCode::F(4));
        assert_eq!(view.visible.len(), 6);
        assert_eq!(view.current(), Some(1));

        let query = Query {
            kinds: vec!["model-input".to_owned()],
            ..Query::default()
        };
        let mut scoped = View::new(&trace, &query).unwrap();
        key(&mut scoped, KeyCode::F(4));
        assert_eq!(scoped.visible, [0]);
        key(&mut scoped, KeyCode::F(6));
        assert_eq!(scoped.visible, [0, 4]);

        key(&mut view, KeyCode::End);
        key(&mut view, KeyCode::F(4));
        assert_eq!(view.visible, [5]);
        assert!(view.notice.contains("no request/command IDs"));
    }

    #[test]
    fn unicode_search_edits_apply_cancel_and_clear_without_mutating_the_trace() {
        let trace = fixture();
        let original = trace.events[4].raw.clone();
        let mut view = View::new(&trace, &Query::default()).unwrap();
        key(&mut view, KeyCode::Char('/'));
        for c in "éLODiE 花".chars() {
            key(&mut view, KeyCode::Char(c));
        }
        key(&mut view, KeyCode::Left);
        key(&mut view, KeyCode::Delete);
        key(&mut view, KeyCode::Char('花'));
        key(&mut view, KeyCode::Enter);
        assert_eq!(view.visible, [4]);
        key(&mut view, KeyCode::Char('/'));
        assert!(!key(&mut view, KeyCode::Char('q')));
        key(&mut view, KeyCode::Esc);
        assert_eq!(view.search, "éLODiE 花");
        assert_eq!(view.visible, [4]);
        key(&mut view, KeyCode::Char('/'));
        view.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        key(&mut view, KeyCode::Char('🌹'));
        key(&mut view, KeyCode::Backspace);
        key(&mut view, KeyCode::Enter);
        assert_eq!(view.visible.len(), 6);
        assert_eq!(trace.events[4].raw, original);
        assert!(key(&mut view, KeyCode::Char('q')));

        let mut editor = Search::default();
        for _ in 0..SEARCH_BYTES {
            editor.key(KeyEvent::new(KeyCode::Char('🌹'), KeyModifiers::NONE));
        }
        assert_eq!(editor.text.len(), SEARCH_BYTES);
        assert_eq!(editor.cursor, SEARCH_BYTES);
        assert!(editor.text.is_char_boundary(editor.cursor));
    }

    #[test]
    fn no_match_and_small_screens_stay_navigable_and_controls_are_sanitized() {
        let mut trace = fixture();
        trace.events[0].actor = Some("scout\u{1b}]52;clipboard-secret\u{7}".to_owned());
        trace.events[0].kind = "model-input\n\u{202e}".to_owned();
        let mut view = View::new(&trace, &Query::default()).unwrap();
        for (width, height) in [(20, 8), (42, 14), (80, 24), (150, 40)] {
            let rendered = screen(&mut view, width, height);
            assert!(!rendered.contains('\u{1b}'));
            assert!(!rendered.contains('\u{202e}'));
            assert!(!rendered.contains("clipboard-secret"));
        }
        view.search = "missing query value".to_owned();
        view.refilter();
        assert_eq!(view.current(), None);
        for code in [
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::F(4),
        ] {
            assert!(!key(&mut view, code));
        }
        assert!(screen(&mut view, 150, 40).contains("No matching events"));
        key(&mut view, KeyCode::F(6));
        assert_eq!(view.visible.len(), 6);
        assert_eq!(view.current(), Some(0));
        let mut released = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        released.kind = KeyEventKind::Release;
        assert!(!view.key(released));
        key(&mut view, KeyCode::Char('/'));
        assert!(view.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn detail_is_bounded_unicode_safe_and_scroll_resets_on_selection() {
        let (prefix, truncated) = bounded_json(&json!({"x": "🌹".repeat(20_000)}));
        assert!(truncated);
        assert!(
            prefix.len() < DETAIL_BYTES,
            "partial UTF-8 character is removed"
        );
        assert!(prefix.ends_with('🌹'));
        let value = json!({"small": "花"});
        assert_eq!(
            bounded_json(&value),
            (serde_json::to_string_pretty(&value).unwrap(), false)
        );
        let mut trace = fixture();
        trace.events[0].raw = json!({"payload": "🌹".repeat(20_000)});
        let mut view = View::new(&trace, &Query::default()).unwrap();
        assert!(view.detail.contains("Display truncated at 64,000 bytes"));
        assert!(view.detail.len() < DETAIL_BYTES + 1_000);
        assert_eq!(
            trace.events[0].raw["payload"].as_str().unwrap().len(),
            80_000
        );
        screen(&mut view, 130, 30);
        view.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL));
        assert!(view.detail_scroll > 0);
        view.detail_scroll = usize::MAX;
        view.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL));
        assert_eq!(
            view.detail_scroll,
            view.detail_lines.len() - view.detail_page
        );
        assert!(screen(&mut view, 130, 30).contains("complete event remains"));
        key(&mut view, KeyCode::Down);
        assert_eq!(view.detail_scroll, 0);
        assert_eq!(view.detail_event, Some(1));
        assert!(!view.detail.contains("Display truncated"));
        for line in reflow("花🌹abc\né\n", 4) {
            assert!(Line::raw(line).width() <= 4);
        }
    }
}
