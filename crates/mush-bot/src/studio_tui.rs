//! Optional terminal frontend for the standalone authoring studio.
use crate::{Result, config::Config, studio::Session, studio_runtime};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{IsTerminal, Write, stdin, stdout},
    path::Path,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const INPUT_BYTES: usize = 16_000;
const DETAIL_BYTES: usize = 64_000;

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

/// UI failures close the input channel; the engine still finishes pending accounting.
pub async fn run(
    config: Config,
    session: Session,
    output: &Path,
    generate_initial: bool,
) -> Result<Value> {
    if !stdin().is_terminal() || !stdout().is_terminal() {
        return Err("studio TUI requires interactive terminal stdin and stdout".into());
    }
    let initial = serde_json::to_value(&session)?;
    let (input, receiver) = mpsc::unbounded_channel();
    let (events, mut stream) = mpsc::unbounded_channel();
    let (ready, initialized) = oneshot::channel();
    let display = tokio::task::spawn_blocking(move || -> Result<()> {
        enable_raw_mode()?;
        let _restore = Restore;
        execute!(
            stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            crossterm::cursor::Hide
        )?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let mut view = View::new(initial);
        terminal.draw(|frame| view.draw(frame))?;
        if ready.send(()).is_err() {
            return Ok(());
        }
        loop {
            // Bound display work per frame even if the engine emits a burst of records.
            for _ in 0..128 {
                match stream.try_recv() {
                    Ok(studio_runtime::Event::Text { tag, text }) => view.text(&tag, &text),
                    Ok(studio_runtime::Event::State {
                        session,
                        model,
                        budget,
                    }) => {
                        view.state(
                            serde_json::to_value(session)?,
                            format!("{}: {}", model.profile, model.settings.name),
                            budget,
                        );
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
                }
            }
            terminal.draw(|frame| view.draw(frame))?;
            if event::poll(Duration::from_millis(50))? {
                let command = match event::read()? {
                    Event::Key(key) => view.key(key),
                    Event::Paste(text) => {
                        view.paste(&text);
                        None
                    }
                    _ => None,
                };
                if let Some(command) = command
                    && input.send(command).is_err()
                {
                    view.notice = "Studio input is closed; waiting for final accounting.".into();
                    view.stopping = true;
                }
            }
        }
    });
    if initialized.await.is_err() {
        display.await??;
        return Err("studio terminal closed before initialization".into());
    }
    let result = studio_runtime::run_with_io(
        config,
        session,
        output.to_path_buf(),
        false,
        generate_initial,
        receiver,
        events,
    )
    .await;
    let display_result = display.await?;
    // Prefer an engine failure, which may describe persistence/accounting problems.
    let report = result?;
    display_result?;
    Ok(report)
}

#[derive(Default)]
struct Editor {
    text: String,
    cursor: usize,
}
impl Editor {
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
    fn insert(&mut self, text: &str) {
        if self.text.len() + text.len() <= INPUT_BYTES {
            self.text.insert_str(self.cursor, text);
            self.cursor += text.len();
        }
    }
    fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
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

struct View {
    session: Value,
    model: String,
    budget: Value,
    preview: usize,
    list_top: usize,
    logs: VecDeque<String>,
    show_logs: bool,
    detail: String,
    detail_width: usize,
    detail_lines: Vec<String>,
    scroll: usize,
    page: usize,
    input: Editor,
    editing: bool,
    stopping: bool,
    notice: String,
}

impl View {
    fn new(session: Value) -> Self {
        let mut view = Self {
            session,
            model: "initializing".into(),
            budget: Value::Null,
            preview: 0,
            list_top: 0,
            logs: VecDeque::new(),
            show_logs: false,
            detail: String::new(),
            detail_width: 0,
            detail_lines: Vec::new(),
            scroll: 0,
            page: 10,
            input: Editor::default(),
            editing: false,
            stopping: false,
            notice: "Arrow keys preview locally. i edits an instruction; s selects the preview."
                .into(),
        };
        view.refresh();
        view
    }

    fn candidates(&self) -> &[Value] {
        self.session["candidates"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn number(&self) -> Option<u64> {
        self.candidates()
            .get(self.preview)
            .and_then(|candidate| candidate["number"].as_u64())
    }

    fn state(&mut self, session: Value, model: String, budget: Value) {
        let previous = self.number();
        let newly_selected = if session["selected"] != self.session["selected"] {
            session["selected"].as_u64()
        } else {
            None
        };
        self.session = session;
        self.model = clean(&model);
        self.budget = budget;
        self.preview = newly_selected
            .or(previous)
            .and_then(|number| {
                self.candidates()
                    .iter()
                    .position(|candidate| candidate["number"] == number)
            })
            .unwrap_or(0);
        self.refresh();
    }

    fn text(&mut self, tag: &str, text: &str) {
        let text = clean(text);
        let tag = clean(tag);
        self.logs
            .push_back(format!("[studio/{tag}] {}", crate::tail(&text, 4_000)));
        while self.logs.len() > 100 {
            self.logs.pop_front();
        }
        if !self.stopping {
            self.notice = format!(
                "[{tag}] {} | F2 opens runtime log",
                crate::tail(text.lines().next().unwrap_or(""), 300)
            );
        }
        if self.show_logs {
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        self.detail = if self.show_logs {
            self.logs.iter().cloned().collect::<Vec<_>>().join("\n\n")
        } else if let Some(candidate) = self.candidates().get(self.preview) {
            candidate_preview(candidate)
        } else {
            "No candidates yet. Runtime messages are available with F2.\nUse i to enter /help, an authoring request, or /export after selecting a draft.".into()
        };
        self.detail_width = 0;
        self.detail_lines.clear();
        self.scroll = 0;
    }

    fn queue(&mut self, command: String) -> Option<String> {
        if self.stopping || command.trim().is_empty() {
            return None;
        }
        if command.trim() == "/quit" {
            self.stopping = true;
            self.editing = false;
            self.notice = "Quit queued. Waiting for active requests and cost receipts; do not close this terminal.".into();
        } else {
            self.notice =
                "Command queued; the studio processes requests and edits in order.".into();
        }
        Some(command)
    }

    fn key(&mut self, key: KeyEvent) -> Option<String> {
        if key.kind == KeyEventKind::Release || self.stopping {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q' | 'c'))
        {
            return self.queue("/quit".into());
        }
        if self.editing {
            match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    self.input = Editor::default();
                }
                KeyCode::Enter => {
                    self.editing = false;
                    self.input.cursor = 0;
                    let command = std::mem::take(&mut self.input.text);
                    return self.queue(command);
                }
                _ => self.input.key(key),
            }
            return None;
        }
        match key.code {
            KeyCode::Up => {
                self.preview = self.preview.saturating_sub(1);
                self.refresh();
            }
            KeyCode::Down => {
                self.preview = self
                    .preview
                    .saturating_add(1)
                    .min(self.candidates().len().saturating_sub(1));
                self.refresh();
            }
            KeyCode::Home => {
                self.preview = 0;
                self.refresh();
            }
            KeyCode::End => {
                self.preview = self.candidates().len().saturating_sub(1);
                self.refresh();
            }
            KeyCode::F(2) | KeyCode::Tab => {
                self.show_logs = !self.show_logs;
                self.refresh();
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page),
            KeyCode::PageDown => {
                self.scroll = self
                    .scroll
                    .saturating_add(self.page)
                    .min(self.detail_lines.len().saturating_sub(self.page));
            }
            KeyCode::Char('s') | KeyCode::Enter => {
                if let Some(number) = self.number() {
                    return self.queue(number.to_string());
                }
                self.notice = "No candidate is available to select yet.".into();
            }
            KeyCode::Char('i') => self.editing = true,
            KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.editing = true;
                self.input.insert(&c.to_string());
            }
            _ => {}
        }
        None
    }

    fn paste(&mut self, text: &str) {
        if self.stopping {
            return;
        }
        if text.chars().any(char::is_control) {
            self.notice =
                "Paste one instruction line; multiline paste is not sent automatically.".into();
            return;
        }
        if self.input.text.len() + text.len() > INPUT_BYTES {
            self.notice = "Instruction exceeds the 16,000-byte input limit.".into();
            return;
        }
        self.editing = true;
        self.input.insert(text);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 55 || area.height < 16 {
            frame.render_widget(Paragraph::new(format!("Authoring studio: enlarge to 55 x 16.\nCtrl+Q queues quit and preserves receipts.\n{}", self.notice)), area);
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(Paragraph::new(format!(
            "Personality studio | {} | {}\n{} candidates | preview {} | selected {} | budget allowed: {}\nArrow keys preview only; selecting and exporting are separate commands.",
            if self.stopping { "finishing queued work" } else { "local authoring" }, self.model,
            self.candidates().len(), self.number().map_or_else(|| "none".into(), |number| number.to_string()),
            self.session["selected"], self.budget["status"]["allowed"],
        )), rows[0]);
        let columns =
            Layout::horizontal([Constraint::Length(28), Constraint::Min(20)]).split(rows[1]);
        let list_height = usize::from(columns[0].height.saturating_sub(2)).max(1);
        if self.preview < self.list_top {
            self.list_top = self.preview;
        } else if self.preview >= self.list_top.saturating_add(list_height) {
            self.list_top = self.preview + 1 - list_height;
        }
        let candidates: Vec<_> = self
            .candidates()
            .iter()
            .enumerate()
            .skip(self.list_top)
            .take(list_height)
            .map(|(index, candidate)| {
                let text = format!(
                    "{} {}. {}",
                    if index == self.preview { ">" } else { " " },
                    candidate["number"],
                    clean(
                        candidate["draft"]["title"]
                            .as_str()
                            .unwrap_or("Untitled draft")
                    )
                );
                if index == self.preview {
                    Line::styled(text, Style::default().fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::raw(text)
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(candidates).block(Block::bordered().title("Drafts | preview ↑ ↓")),
            columns[0],
        );
        let block = Block::bordered().title(if self.show_logs {
            "Runtime log | F2 profile"
        } else {
            "Full candidate | F2 log"
        });
        let inner = block.inner(columns[1]);
        frame.render_widget(block, columns[1]);
        self.page = usize::from(inner.height).max(1);
        let width = usize::from(inner.width).max(1);
        if width != self.detail_width || self.detail_lines.is_empty() {
            self.detail_width = width;
            self.detail_lines = reflow(&self.detail, width);
        }
        self.scroll = self
            .scroll
            .min(self.detail_lines.len().saturating_sub(self.page));
        let lines: Vec<_> = self
            .detail_lines
            .iter()
            .skip(self.scroll)
            .take(self.page)
            .map(|line| Line::raw(line.as_str()))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        frame.render_widget(Paragraph::new(format!("i instruction | s / Enter select preview | F2 profile/log | PgUp/PgDn detail\nEnter sends typed number or instruction | Esc cancels edit | Ctrl+Q quit\n{}", clean(&self.notice))), rows[2]);
        let before = Span::raw(&self.input.text[..self.input.cursor]).width();
        let horizontal = before.saturating_sub(usize::from(rows[3].width.saturating_sub(3)));
        frame.render_widget(
            Paragraph::new(clean(&self.input.text))
                .scroll((0, horizontal.min(u16::MAX as usize) as u16))
                .block(Block::bordered().title(if self.editing {
                    "Instruction | Enter submit | Ctrl+U clear"
                } else {
                    "Local command: i to edit, /help, /export, /quit"
                })),
            rows[3],
        );
        if self.editing {
            frame
                .set_cursor_position((rows[3].x + 1 + (before - horizontal) as u16, rows[3].y + 1));
        }
    }
}

fn clean(text: &str) -> String {
    crate::terminal::plain(text)
        .chars()
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect()
}

#[derive(Default)]
struct Preview {
    bytes: Vec<u8>,
    truncated: bool,
}
impl Write for Preview {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = DETAIL_BYTES.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            self.bytes.extend_from_slice(&bytes[..remaining]);
            self.truncated = true;
            return Err(std::io::Error::other(
                "studio preview display limit reached",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Preview {
    fn finish(self) -> String {
        let end =
            std::str::from_utf8(&self.bytes).map_or_else(|error| error.valid_up_to(), str::len);
        let mut text = clean(std::str::from_utf8(&self.bytes[..end]).unwrap_or(""));
        if self.truncated {
            text.push_str("\n\n[Preview truncated at 64,000 bytes. Use /show for the full JSON; complete draft remains in the saved session/export.]");
        }
        text
    }
}

fn candidate_preview(candidate: &Value) -> String {
    fn render(output: &mut Preview, candidate: &Value) -> std::io::Result<()> {
        let draft = &candidate["draft"];
        let characters = draft["characters"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let field = |value: &Value| {
            clean(
                value
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .unwrap_or("Unspecified"),
            )
        };
        writeln!(
            output,
            "{}. {}\n",
            candidate["number"],
            field(&draft["title"])
        )?;
        writeln!(output, "{}\n", field(&draft["synopsis"]))?;
        for authored in characters {
            let character = &authored["character"];
            writeln!(
                output,
                "{} ({})",
                field(&character["name"]),
                field(&authored["actor"])
            )?;
            writeln!(output, "Appearance: {}", field(&character["appearance"]))?;
            writeln!(output, "Role: {}", field(&authored["role"]))?;
            writeln!(output, "Goal: {}\n", field(&authored["goal"]))?;
        }
        if !draft["group"].is_null() {
            writeln!(output, "Group context and member briefings (authored)")?;
            serde_json::to_writer_pretty(&mut *output, &draft["group"])
                .map_err(std::io::Error::other)?;
            writeln!(output, "\n")?;
        }
        for authored in characters {
            let character = &authored["character"];
            writeln!(
                output,
                "{} — full character profile (JSON)",
                field(&character["name"])
            )?;
            serde_json::to_writer_pretty(&mut *output, character).map_err(std::io::Error::other)?;
            writeln!(output, "\n")?;
        }
        writeln!(output, "Draft provenance")?;
        writeln!(output, "Based on candidate: {}", candidate["based_on"])?;
        writeln!(
            output,
            "Request revision: {}",
            field(&candidate["request_revision"])
        )?;
        writeln!(output, "Instruction: {}", field(&candidate["instruction"]))?;
        writeln!(output, "\nUse /show for the full candidate JSON.")
    }
    let mut output = Preview::default();
    if render(&mut output, candidate).is_err() && !output.truncated {
        return "Unable to format candidate preview.".into();
    }
    output.finish()
}

fn reflow(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut buffer = String::new();
        let mut used = 0;
        for c in line.chars() {
            let cells = Span::raw(c.to_string()).width();
            if used > 0 && used + cells > width {
                lines.push(std::mem::take(&mut buffer));
                used = 0;
            }
            buffer.push(c);
            used += cells;
        }
        lines.push(buffer);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Value {
        json!({"version":1,"selected":null,"candidates":[
            {"number":3,"draft":{"title":"Three","synopsis":"Élodie keeps a promise.\nShe asks, \"Who needs a guide?\"","characters":[{"actor":"scout","role":"A patient guide","goal":"Find the station","character":{"name":"Élodie","appearance":"A weathered coat with a silver flower pin."}}]}},
            {"number":8,"draft":{"title":"Eight","characters":[{"character":{"name":"Marco"}}]}}
        ]})
    }

    fn key(view: &mut View, code: KeyCode) -> Option<String> {
        view.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn screen(view: &mut View, width: u16, height: u16) -> String {
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
    fn preview_is_local_and_selection_uses_stable_numbers_until_acknowledged() {
        let original = session();
        let mut view = View::new(original.clone());
        assert_eq!(view.number(), Some(3));
        assert!(
            view.detail.starts_with(
                "3. Three\n\nÉlodie keeps a promise.\nShe asks, \"Who needs a guide?\""
            )
        );
        assert!(view.detail.contains("Élodie (scout)\nAppearance: A weathered coat with a silver flower pin.\nRole: A patient guide\nGoal: Find the station"));
        assert!(view.detail.contains("full character profile (JSON)"));
        assert_eq!(key(&mut view, KeyCode::Down), None);
        assert_eq!(view.number(), Some(8));
        assert_eq!(view.session, original);
        assert_eq!(key(&mut view, KeyCode::Char('s')), Some("8".into()));
        assert_eq!(view.session["selected"], Value::Null);
        assert_eq!(key(&mut view, KeyCode::Char('3')), None);
        assert_eq!(key(&mut view, KeyCode::Enter), Some("3".into()));
        assert!(view.input.text.is_empty());
        assert_eq!(view.input.cursor, 0);
        let mut acknowledged = original;
        acknowledged["selected"] = json!(3);
        view.state(
            acknowledged,
            "chosen-model".into(),
            json!({"status":{"allowed":true}}),
        );
        assert_eq!(view.number(), Some(3));
        assert_eq!(view.session["selected"], 3);
        key(&mut view, KeyCode::Char('i'));
        for c in "blend Élodie 🌹".chars() {
            key(&mut view, KeyCode::Char(c));
        }
        key(&mut view, KeyCode::Backspace);
        key(&mut view, KeyCode::Char('花'));
        assert_eq!(
            key(&mut view, KeyCode::Enter),
            Some("blend Élodie 花".into())
        );
        assert_eq!(view.session["selected"], 3);
    }

    #[test]
    fn quit_is_queued_once_and_later_events_preserve_pending_accounting_notice() {
        let mut view = View::new(session());
        key(&mut view, KeyCode::Char('i'));
        let quit = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(view.key(quit), Some("/quit".into()));
        assert!(view.stopping);
        assert_eq!(view.key(quit), None);
        assert_eq!(key(&mut view, KeyCode::Enter), None);
        assert_eq!(key(&mut view, KeyCode::Char('8')), None);
        view.paste("/export");
        assert!(view.input.text.is_empty());
        view.text("cost", "Receipt received");
        assert!(
            view.notice
                .contains("Waiting for active requests and cost receipts")
        );
        assert!(view.logs.back().unwrap().contains("Receipt received"));
        view.state(
            session(),
            "model".into(),
            json!({"status":{"allowed":false}}),
        );
        assert!(view.stopping);
        assert!(screen(&mut view, 140, 32).contains("finishing queued work"));
    }

    #[test]
    fn rendering_is_bounded_and_sanitized_and_empty_sessions_accept_commands() {
        let mut source = session();
        source["candidates"][1]["draft"]["title"] = json!("Eight\u{1b}]52;clipboard\u{7}\u{202e}");
        source["candidates"][1]["draft"]["group"] = json!({"name":"Lanterns","shared_knowledge":["Meet by the square"],"members":[{"actor":"scout","briefing":["Private patron concern"]}]});
        let mut view = View::new(source);
        key(&mut view, KeyCode::End);
        assert!(view.detail.contains("Group context and member briefings"));
        assert!(view.detail.contains("Private patron concern"));
        view.text("notice", "\u{1b}[31mReady\u{1b}[0m");
        for (width, height) in [(30, 10), (55, 16), (100, 30), (160, 48)] {
            let rendered = screen(&mut view, width, height);
            assert!(!rendered.contains('\u{1b}'));
            assert!(!rendered.contains('\u{202e}'));
        }
        key(&mut view, KeyCode::F(2));
        assert!(screen(&mut view, 160, 48).contains("[studio/notice] Ready"));
        assert!(view.show_logs);
        let bounded = candidate_preview(
            &json!({"number":9,"draft":{"title":"Large portrait","synopsis":"🌹".repeat(30_000)}}),
        );
        assert!(bounded.len() < DETAIL_BYTES + 200);
        assert!(bounded.contains("Preview truncated"));
        let mut empty = View::new(json!({"candidates":[]}));
        assert_eq!(key(&mut empty, KeyCode::Enter), None);
        assert!(empty.notice.contains("No candidate"));
        empty.paste("/help\n/export");
        assert!(!empty.editing);
        empty.paste("/help");
        assert_eq!(key(&mut empty, KeyCode::Enter), Some("/help".into()));
    }
}
