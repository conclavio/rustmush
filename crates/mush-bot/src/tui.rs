//! Local runner UI. The game still uses the same public SSH terminal connection.
use crate::{
    Result,
    config::Config,
    runner::{self, Control, Frontend, Options, Report},
};
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
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Clear, Paragraph, Wrap},
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{IsTerminal, stdout},
    time::Duration,
};
use tokio::sync::mpsc;

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

/// Cursor indexes bytes, always at a UTF-8 boundary; editing never touches game input.
#[derive(Default)]
struct Editor {
    text: String,
    cursor: usize,
}
impl Editor {
    fn new(text: String) -> Self {
        Self { cursor: 0, text }
    }
    fn insert(&mut self, text: &str) {
        if self.text.len() + text.len() <= 128_000 {
            self.text.insert_str(self.cursor, text);
            self.cursor += text.len();
        }
    }
    fn left(&mut self) {
        self.cursor = self.text[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(i, _)| i);
    }
    fn right(&mut self) {
        self.cursor += self.text[self.cursor..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
    }
    fn key(&mut self, key: KeyEvent, multiline: bool) {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Enter if multiline => self.insert("\n"),
            KeyCode::Tab if multiline => self.insert("  "),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => {
                self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
            }
            KeyCode::End => {
                self.cursor += self.text[self.cursor..]
                    .find('\n')
                    .unwrap_or(self.text.len() - self.cursor)
            }
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
            KeyCode::Up | KeyCode::Down if multiline => {
                let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
                let column = self.text[start..self.cursor].chars().count();
                let target = if key.code == KeyCode::Up {
                    if start == 0 {
                        return;
                    }
                    self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1)
                } else {
                    let Some(end) = self.text[self.cursor..].find('\n') else {
                        return;
                    };
                    self.cursor + end + 1
                };
                self.cursor = target
                    + self.text[target..]
                        .split('\n')
                        .next()
                        .unwrap_or("")
                        .char_indices()
                        .nth(column)
                        .map_or_else(
                            || self.text[target..].split('\n').next().unwrap_or("").len(),
                            |(i, _)| i,
                        );
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct View {
    game: String,
    events: VecDeque<String>,
    state: Value,
    input: Editor,
    editor: Option<(String, Editor)>,
    model_choice: Option<usize>,
    panel: usize,
    panel_scroll: u16,
    last_result: String,
    scroll: usize,
    notice: String,
    capture: Value,
}

const PANELS: [&str; 17] = [
    "State",
    "Costs",
    "Memory",
    "Prompts",
    "Context",
    "Debug",
    "Result",
    "Assembly",
    "Groups",
    "Budget",
    "Conversation",
    "Social perspective",
    "Activities",
    "Learning",
    "Map",
    "Mission",
    "Playtest",
];

impl View {
    fn event(&mut self, event: Value) {
        let kind = event["kind"].as_str().unwrap_or("event");
        if kind == "model-input" {
            self.capture = event["data"].clone();
            return;
        }
        if kind == "snapshot" {
            self.state = event["data"].clone();
            return;
        }
        if kind == "observation" {
            self.game.push_str(event["data"].as_str().unwrap_or(""));
            self.game = crate::tail(&self.game, 128_000).into();
        } else if ![
            "model-input",
            "model-reply",
            "usage",
            "cost-receipt",
            "command-result",
        ]
        .contains(&kind)
        {
            let data = event["data"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| event["data"].to_string());
            if [
                "state",
                "cost",
                "budget",
                "prompt",
                "prompt-inspection",
                "group",
                "social",
                "activity",
                "learning",
                "mission",
                "playtest",
                "development",
                "map",
                "character-state",
                "context",
                "debug",
                "knowledge",
                "recall",
                "route",
                "character",
                "attention",
                "conversation",
                "answer",
                "probe-result",
                "probe",
                "help",
            ]
            .contains(&kind)
            {
                self.last_result = crate::terminal::plain(&data);
                self.panel = 6;
                self.panel_scroll = 0;
            }
            self.events.push_back(format!(
                "[{kind}] {}",
                crate::tail(&crate::terminal::plain(&data), 4000)
            ));
            while self.events.len() > 120 {
                self.events.pop_front();
            }
            if ["control-error", "error", "blocked", "breakpoint", "watch"].contains(&kind) {
                self.notice = crate::terminal::plain(&data);
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        if frame.area().width < 30 || frame.area().height < 10 {
            frame.render_widget(
                Paragraph::new(
                    "Resize the terminal to at least 30 × 10. The session is still running.",
                ),
                frame.area(),
            );
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(6),
            Constraint::Length(3),
        ])
        .split(frame.area());
        let model = self.state["model"]["settings"]["name"]
            .as_str()
            .unwrap_or("no model");
        let status = self.state["state"]["status"]
            .as_str()
            .unwrap_or("connecting");
        let spend = &self.state["costs"]["session"]["totals"]["provider_cost_by_currency"];
        frame.render_widget(Paragraph::new(format!("RustMUSH Bot | {status} | {model} | session {spend}\nF2 run/pause F3 model F4 prompts F5 step F6 context F7 debug F8 commit F9 panel F10 budget F11 conversation Ctrl+Q quit")), rows[0]);
        let columns = Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(rows[1]);
        let lines: Vec<_> = self.game.lines().collect();
        let height = columns[0].height.saturating_sub(2) as usize;
        let end = lines.len().saturating_sub(self.scroll.min(lines.len()));
        let text = lines[end.saturating_sub(height)..end].join("\n");
        frame.render_widget(
            Paragraph::new(text).block(Block::bordered().title("Game session | PgUp/PgDn scroll")),
            columns[0],
        );
        let field = [
            "state",
            "costs",
            "memory_store",
            "prompts",
            "context",
            "debug",
            "result",
            "assembly",
            "group_affiliations",
            "budget",
            "conversation",
            "social",
            "activity",
            "learning",
            "map",
            "mission",
            "playtest",
        ][self.panel];
        let info = if self.panel == 6 {
            self.last_result.clone()
        } else if self.panel == 7 {
            serde_json::to_string_pretty(&serde_json::json!({"stale":self.state["capture_stale"],
                "request_revision":self.capture["request_revision"],"composition":self.capture["composition"],
                "messages":self.capture["payload"]["messages"]})).unwrap_or_default()
        } else if self.panel == 10 {
            serde_json::to_string_pretty(&serde_json::json!({"conversation":self.state["conversation"],"provenance":self.state["provenance"],"costs":self.state["costs"]["session"]})).unwrap_or_default()
        } else if self.panel == 2 {
            serde_json::to_string_pretty(&serde_json::json!({"working_notes":self.state["state"]["memory"],"store":self.state["memory_store"],"attention":self.state["attention"]})).unwrap_or_default()
        } else {
            serde_json::to_string_pretty(&self.state[field]).unwrap_or_default()
        };
        frame.render_widget(
            Paragraph::new(info)
                .scroll((self.panel_scroll, 0))
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(format!("{} | Ctrl+PgUp/PgDn", PANELS[self.panel]))),
            columns[1],
        );
        let events = self
            .events
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(
            Paragraph::new(events)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title("Runner events")),
            rows[2],
        );
        frame.render_widget(
            Paragraph::new(self.input.text.clone()).block(Block::bordered().title(
                if self.notice.is_empty() {
                    "Control /help or game command"
                } else {
                    &self.notice
                },
            )),
            rows[3],
        );
        if let Some((name, editor)) = &self.editor {
            let area = inset(frame.area(), 2);
            frame.render_widget(Clear, area);
            let before = &editor.text[..editor.cursor];
            let row = before.bytes().filter(|b| *b == b'\n').count();
            let column = before.rsplit('\n').next().unwrap_or("").chars().count();
            let vertical = row.saturating_sub(area.height.saturating_sub(4) as usize);
            let horizontal = column.saturating_sub(area.width.saturating_sub(4) as usize);
            frame.render_widget(
                Paragraph::new(editor.text.clone())
                    .scroll((
                        vertical.min(u16::MAX as usize) as u16,
                        horizontal.min(u16::MAX as usize) as u16,
                    ))
                    .block(
                        Block::bordered()
                            .title(format!("Edit {name} | Ctrl+S apply & pause | Esc cancel"))
                            .title_bottom(self.notice.as_str()),
                    ),
                area,
            );
            frame.set_cursor_position((
                area.x + 1 + (column - horizontal) as u16,
                area.y + 1 + (row - vertical) as u16,
            ));
        } else if let Some(choice) = self.model_choice {
            let models = self.state["models"].as_array().cloned().unwrap_or_default();
            let text = models
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    format!(
                        "{} {}: {}",
                        if i == choice { ">" } else { " " },
                        m["profile"].as_str().unwrap_or("?"),
                        m["settings"]["name"].as_str().unwrap_or("?")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let area = inset(frame.area(), 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::bordered().title("Select model | arrows + Enter | Esc cancel")),
                area,
            );
        } else if rows[3].width > 2 {
            let column = self.input.text[..self.input.cursor]
                .chars()
                .count()
                .min(rows[3].width.saturating_sub(3) as usize);
            frame.set_cursor_position((rows[3].x + 1 + column as u16, rows[3].y + 1));
        }
    }

    fn key(&mut self, key: KeyEvent) -> Result<Option<Control>> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return Ok(Some(Control::Line("/quit".into())));
        }
        if let Some((name, editor)) = &mut self.editor {
            if key.code == KeyCode::Esc {
                self.editor = None;
            } else if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('s')
            {
                let control = match name.as_str() {
                    "decision_system" | "question_system" => {
                        let mut p: crate::experiment::Prompts =
                            serde_json::from_value(self.state["prompts"].clone())?;
                        if name == "decision_system" {
                            p.decision_system = Some(editor.text.clone());
                        } else {
                            p.question_system = Some(editor.text.clone());
                        }
                        p.validate()?;
                        Control::Prompts(p)
                    }
                    "prompts" => {
                        let p: crate::experiment::Prompts = serde_json::from_str(&editor.text)?;
                        p.validate()?;
                        Control::Prompts(p)
                    }
                    "context" => {
                        let c: crate::experiment::Context = serde_json::from_str(&editor.text)?;
                        c.validate()?;
                        Control::Context(c)
                    }
                    "budget" => {
                        let budget: crate::budget::Settings = serde_json::from_str(&editor.text)?;
                        budget.validate()?;
                        Control::Budget(budget)
                    }
                    "conversation" => {
                        let settings: crate::conversation::Settings =
                            serde_json::from_str(&editor.text)?;
                        settings.validate()?;
                        Control::Conversation(settings)
                    }
                    _ => {
                        let d: crate::debugger::Settings = serde_json::from_str(&editor.text)?;
                        d.validate()?;
                        Control::Debug(d)
                    }
                };
                self.editor = None;
                return Ok(Some(control));
            } else {
                editor.key(key, true);
            }
            return Ok(None);
        }
        if let Some(choice) = &mut self.model_choice {
            let models = self.state["models"].as_array().cloned().unwrap_or_default();
            match key.code {
                KeyCode::Esc => self.model_choice = None,
                KeyCode::Up => *choice = choice.saturating_sub(1),
                KeyCode::Down => *choice = (*choice + 1).min(models.len().saturating_sub(1)),
                KeyCode::Enter => {
                    let name = models
                        .get(*choice)
                        .and_then(|m| m["profile"].as_str())
                        .ok_or("no model profiles available")?;
                    let command = format!("/model use {name}");
                    self.model_choice = None;
                    return Ok(Some(Control::Line(command)));
                }
                _ => {}
            }
            return Ok(None);
        }
        let command = match key.code {
            KeyCode::F(1) => Some("/help".into()),
            KeyCode::F(4)
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
            {
                let prompts: crate::experiment::Prompts =
                    serde_json::from_value(self.state["prompts"].clone())?;
                let question = key.modifiers.contains(KeyModifiers::CONTROL);
                let text = if question {
                    prompts.question_system
                } else {
                    prompts.decision_system
                }
                .unwrap_or_else(|| crate::model::Brain::system(question));
                self.editor = Some((
                    if question {
                        "question_system"
                    } else {
                        "decision_system"
                    }
                    .into(),
                    Editor::new(text),
                ));
                self.notice.clear();
                Some("/pause".into())
            }
            KeyCode::F(2) => Some(
                if self.state["state"]["status"] == "running"
                    || self.state["state"]["status"] == "listening"
                {
                    "/pause"
                } else {
                    "/resume"
                }
                .to_owned(),
            ),
            KeyCode::F(3) => {
                self.model_choice = Some(0);
                None
            }
            KeyCode::F(4) | KeyCode::F(6) | KeyCode::F(7) | KeyCode::F(10) | KeyCode::F(11) => {
                let name = match key.code {
                    KeyCode::F(4) => "prompts",
                    KeyCode::F(6) => "context",
                    KeyCode::F(10) => "budget",
                    KeyCode::F(11) => "conversation",
                    _ => "debug",
                };
                let settings = if name == "budget" {
                    &self.state[name]["settings"]
                } else if name == "conversation" {
                    &self.state[name]["state"]["policy"]
                } else {
                    &self.state[name]
                };
                if settings.is_null() {
                    return Err("wait for runner state before editing".into());
                }
                self.editor = Some((
                    name.into(),
                    Editor::new(serde_json::to_string_pretty(settings)?),
                ));
                self.notice.clear();
                Some("/pause".into())
            }
            KeyCode::F(5) => Some("/step model".into()),
            KeyCode::F(8) => Some("/commit".into()),
            KeyCode::F(9) => {
                self.panel = (self.panel + 1) % PANELS.len();
                self.panel_scroll = 0;
                None
            }
            KeyCode::PageUp if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.panel_scroll = self.panel_scroll.saturating_sub(10);
                None
            }
            KeyCode::PageDown if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.panel_scroll = self.panel_scroll.saturating_add(10);
                None
            }
            KeyCode::PageUp => {
                self.scroll = (self.scroll + 10).min(self.game.lines().count());
                None
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
                None
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input.text);
                self.input.cursor = 0;
                Some(text)
            }
            _ => {
                self.input.key(key, false);
                None
            }
        };
        Ok(command.map(Control::Line))
    }
}

fn inset(area: Rect, margin: u16) -> Rect {
    area.inner(ratatui::layout::Margin::new(
        margin.min(area.width / 3),
        margin.min(area.height / 3),
    ))
}

pub async fn run(config: Config, options: Options) -> Result<Report> {
    if options.headless || options.scenario.is_some() || options.json_events {
        return Err("--tui cannot be combined with headless, script or JSON event output".into());
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(
            "--tui requires an interactive terminal; use the REPL or --headless for pipes".into(),
        );
    }
    let (controls, receiver) = mpsc::channel(32);
    let (events, mut stream) = mpsc::channel(256);
    // Raw mode and alternate screen are owned by one blocking display task.
    let display = tokio::task::spawn_blocking(move || -> Result<()> {
        enable_raw_mode()?;
        let _restore = Restore;
        execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let mut view = View::default();
        loop {
            loop {
                match stream.try_recv() {
                    Ok(event) => view.event(event),
                    Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                }
            }
            terminal.draw(|frame| view.draw(frame))?;
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => match view.key(key) {
                        Ok(Some(control)) => {
                            if controls.try_send(control).is_err() {
                                view.notice = "Runner control queue is busy; retry shortly.".into();
                            }
                        }
                        Ok(None) => {}
                        Err(error) => view.notice = error.to_string(),
                    },
                    Event::Paste(text) => {
                        if let Some((_, editor)) = &mut view.editor {
                            editor.insert(&text.replace('\r', ""));
                        } else if !text.chars().any(char::is_control) {
                            view.input.insert(&text);
                        } else {
                            view.notice =
                                "Multiline paste is supported in the JSON editors.".into();
                        }
                    }
                    _ => {}
                }
            }
        }
    });
    let report = runner::run_controlled(
        config,
        options,
        Some(Frontend {
            controls: receiver,
            events,
        }),
    )
    .await;
    display.await??;
    report
}

#[derive(Default)]
struct CastActor {
    game: String,
    state: Value,
    capture: Value,
}

#[derive(Default)]
struct CastView {
    floor: Value,
    actors: std::collections::BTreeMap<String, CastActor>,
    selected: usize,
    panel: usize,
    scroll: usize,
    detail_scroll: u16,
    title: String,
    cast_name: String,
    phase: String,
    finished: bool,
    stopping: bool,
    debug: crate::cast_debug::Snapshot,
    notice: String,
    pending_control: Option<(usize, u64)>,
}

const CAST_PANELS: usize = 10;

impl CastView {
    fn sync_debug(&mut self, state: crate::cast_debug::Snapshot) {
        if state.run != self.debug.run && state.run != 0 {
            // The reliable control stream defines the run boundary even if a
            // droppable cast-start event never reached the display.
            for actor in self.actors.values_mut() {
                *actor = CastActor::default();
            }
            self.selected = 0;
            self.scroll = 0;
            self.detail_scroll = 0;
            self.phase.clear();
            self.floor = Value::Null;
            self.title = format!("{} | run {}", self.cast_name, state.run);
        }
        if (state.run, state.revision, state.notice)
            != (self.debug.run, self.debug.revision, self.debug.notice)
        {
            self.pending_control = None;
            self.notice.clear();
        }
        self.debug = state;
    }

    fn event(&mut self, event: Value) {
        let run = event["run"].as_u64().or_else(|| {
            (event["kind"] == "cast-start")
                .then(|| event["data"]["run"].as_u64())
                .flatten()
        });
        if run.is_some_and(|run| self.debug.run != 0 && run != self.debug.run as u64) {
            return;
        }
        if event["kind"] == "cast-start" {
            self.cast_name = event["data"]["name"].as_str().unwrap_or("Cast").into();
            self.title = format!(
                "{} | run {}",
                event["data"]["name"].as_str().unwrap_or("Cast"),
                event["data"]["run"]
            );
            self.actors.clear();
            self.floor = Value::Null;
            self.selected = 0;
            self.scroll = 0;
            self.phase.clear();
            if let Some(names) = event["data"]["actors"].as_array() {
                for name in names.iter().filter_map(Value::as_str) {
                    self.actors.insert(name.into(), Default::default());
                }
            }
        } else if event["kind"] == "cast-finished" {
            self.phase = if event["data"]["success"] == true {
                "Passed. Reports saved.".into()
            } else {
                format!(
                    "Failed: {}",
                    event["data"]["error"].as_str().unwrap_or("see run report")
                )
            };
        } else if event["kind"] == "phase" {
            self.phase = format!(
                "{}: {} ({})",
                event["data"]["id"].as_str().unwrap_or(""),
                event["data"]["name"].as_str().unwrap_or("unnamed"),
                event["data"]["dispatch"].as_str().unwrap_or("parallel")
            );
        } else if event["actor"] == "_harness" && event["event"]["kind"] == "speaking-floor" {
            self.floor = event["event"]["data"].clone();
        } else if let Some(actor) = event["actor"].as_str().filter(|name| *name != "_harness") {
            let view = self.actors.entry(actor.into()).or_default();
            let data = &event["event"]["data"];
            match event["event"]["kind"].as_str().unwrap_or("") {
                "probe-result" => view.state["last_probe"] = data.clone(),
                "snapshot" | "ready" => view.state = data.clone(),
                "result" => {
                    for field in [
                        "state",
                        "memory_store",
                        "model",
                        "costs",
                        "budget",
                        "prompts",
                        "context",
                        "debug",
                        "character_state",
                        "probe_results_omitted",
                        "conversation",
                        "social",
                        "provenance",
                    ] {
                        if let Some(value) = data.get(field) {
                            view.state[field] = value.clone();
                        }
                    }
                    view.state["group_affiliations"] = data["affiliations"].clone();
                    view.state["last_probe"] = data["probe_results"]
                        .as_array()
                        .and_then(|items| items.last())
                        .cloned()
                        .unwrap_or(Value::Null);
                    view.state["pending_probe"] = Value::Null;
                    view.state["pending_command"] = Value::Null;
                    view.state["staged_decision"] = Value::Null;
                    view.state["model_in_flight"] = Value::Bool(false);
                }
                "model-input" => view.capture = data.clone(),
                "observation" => {
                    view.game.push_str(data.as_str().unwrap_or(""));
                    view.game = crate::tail(&view.game, 128_000).into();
                }
                "done" | "blocked" | "error" => {
                    view.state["state"]["status"] = event["event"]["kind"].clone()
                }
                _ => {}
            }
        }
    }

    fn key(&mut self, key: KeyEvent) -> Option<crate::cast_debug::Command> {
        use crate::cast_debug::{Action, Command, Mode};
        if key.kind != KeyEventKind::Press {
            return None;
        }
        match key.code {
            KeyCode::Tab => {
                self.selected = (self.selected + 1) % self.actors.len().max(1);
                self.scroll = 0;
                self.detail_scroll = 0;
            }
            KeyCode::F(9) => {
                self.panel = (self.panel + 1) % CAST_PANELS;
                self.detail_scroll = 0;
            }
            KeyCode::PageUp if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(10);
            }
            KeyCode::PageDown if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_add(10);
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::F(2) | KeyCode::F(5) => {
                if self.finished
                    || self.stopping
                    || self.debug.run == 0
                    || matches!(self.debug.mode, Mode::Stopping | Mode::Finished)
                {
                    self.notice =
                        "Phase controls are unavailable while starting, stopping or finished."
                            .into();
                    return None;
                }
                if self.pending_control.is_some() {
                    self.notice =
                        "Waiting for the harness to acknowledge the queued command.".into();
                    return None;
                }
                let action = if key.code == KeyCode::F(5) {
                    if self.debug.mode != Mode::Held {
                        self.notice = "Step is available only when phase dispatch is held.".into();
                        return None;
                    }
                    if self.debug.next_phase.is_none() {
                        self.notice =
                            "No phase remains. F2 continues through the final boundary.".into();
                        return None;
                    }
                    Action::Step
                } else if self.debug.mode == Mode::Running {
                    Action::Hold
                } else {
                    Action::Continue
                };
                return Some(Command {
                    run: self.debug.run,
                    revision: self.debug.revision,
                    action,
                });
            }
            _ => {}
        }
        None
    }

    fn send_control(
        &mut self,
        command: crate::cast_debug::Command,
        controls: &mpsc::Sender<crate::cast_debug::Command>,
    ) {
        let revision = (command.run, command.revision);
        match controls.try_send(command) {
            Ok(()) => {
                self.pending_control = Some(revision);
                self.notice = "Phase command queued; awaiting harness acknowledgment.".into();
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.notice = "Harness control queue is busy; retry after its next update.".into();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.notice =
                    "Harness control channel is closed; phase controls are unavailable.".into();
            }
        }
    }

    fn dispatch_status(&self) -> String {
        use crate::cast_debug::Mode;
        if self.debug.run == 0 {
            return "Phase dispatch: waiting for harness state".into();
        }
        let mode = match self.debug.mode {
            Mode::Running => "running",
            Mode::HoldRequested => "hold requested",
            Mode::Held => "HELD",
            Mode::Stepping => "stepping one phase",
            Mode::Stopping => "stopping",
            Mode::Finished => "finished",
        };
        let next = self.debug.next_phase.map_or_else(
            || "final boundary".into(),
            |n| format!("next {n}/{}", self.debug.phase_count),
        );
        let remaining = if self.debug.mode == Mode::Held {
            format!(
                " | hold limit in {}s",
                self.debug
                    .max_hold_ms
                    .saturating_sub(self.debug.held_ms)
                    .div_ceil(1000)
            )
        } else {
            String::new()
        };
        format!("Phase dispatch: {mode} | {next}{remaining}")
    }

    fn draw(&self, frame: &mut Frame) {
        if frame.area().width < 55 || frame.area().height < 16 {
            frame.render_widget(
                Paragraph::new(format!(
                    "Resize to 55 x 16.\n{}\n{}\nBots and game remain live.",
                    self.dispatch_status(),
                    self.controls_hint()
                )),
                frame.area(),
            );
            return;
        }
        let rows = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(crate::terminal::plain(&format!(
                "{} | {}\n{}\n{}\n{}",
                self.title,
                self.phase,
                self.dispatch_status(),
                self.debug.notice,
                self.notice
            ))),
            rows[0],
        );
        let cols = Layout::horizontal([Constraint::Length(24), Constraint::Min(25)]).split(rows[1]);
        let roster = self
            .actors
            .iter()
            .enumerate()
            .map(|(i, (name, a))| {
                format!(
                    "{} {}\n  {} / {}",
                    if i == self.selected { ">" } else { " " },
                    name,
                    a.state["state"]["status"].as_str().unwrap_or("connecting"),
                    a.state["conversation"]["phase"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(
            Paragraph::new(roster)
                .scroll((
                    (self.selected * 2)
                        .saturating_sub(cols[0].height.saturating_sub(4) as usize)
                        .min(u16::MAX as usize) as u16,
                    0,
                ))
                .block(Block::bordered().title("Actors | Tab")),
            cols[0],
        );
        if let Some((name, actor)) = self.actors.iter().nth(self.selected) {
            let panes = Layout::vertical([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(cols[1]);
            let lines: Vec<_> = actor.game.lines().collect();
            let end = lines.len().saturating_sub(self.scroll.min(lines.len()));
            let start = end.saturating_sub(panes[0].height.saturating_sub(2) as usize);
            frame.render_widget(
                Paragraph::new(lines[start..end].join("\n"))
                    .block(Block::bordered().title(format!("{name} | game session"))),
                panes[0],
            );
            let (title, value) = match self.panel {
                9 => (
                    "Selected actor activities and learning",
                    serde_json::json!({"activity":actor.state["activity"],"learning":actor.state["learning"],"map":actor.state["map"],"mission":actor.state["mission"],"playtest":actor.state["playtest"]}),
                ),
                8 => (
                    "Selected actor social perspective",
                    actor.state["social"].clone(),
                ),
                7 => (
                    "Conversation and floor",
                    serde_json::json!({"conversation":actor.state["conversation"],
                    "floor":self.floor,"provenance":actor.state["provenance"],"costs":actor.state["costs"]["session"]}),
                ),
                1 => ("Prompt assembly", json_value(&actor.capture)),
                2 => (
                    "Session costs and budget",
                    serde_json::json!({"session":actor.state["costs"]["session"],"budget":actor.state["budget"]}),
                ),
                3 => (
                    "Group affiliations",
                    actor.state["group_affiliations"].clone(),
                ),
                4 => (
                    "Selected actor memory",
                    serde_json::json!({"working_notes":actor.state["state"]["memory"],"evidence_store":actor.state["memory_store"],"attention":actor.state["attention"],"context_settings":actor.state["context"]}),
                ),
                5 => (
                    "Selected actor debug",
                    serde_json::json!({"settings":actor.state["debug"],"staged_decision":actor.state["staged_decision"],"pending_command":actor.state["pending_command"],"model_in_flight":actor.state["model_in_flight"],"capture_stale":actor.state["capture_stale"],"max_steps":actor.state["max_steps"],"max_seconds":actor.state["max_seconds"],"goal_elapsed_seconds":actor.state["goal_elapsed_seconds"]}),
                ),
                6 => (
                    "Selected actor probes",
                    serde_json::json!({"pending_probe":actor.state["pending_probe"],"last_probe":actor.state["last_probe"],"earlier_results_omitted":actor.state["probe_results_omitted"]}),
                ),
                _ => (
                    "State and attention",
                    serde_json::json!({"state":actor.state["state"],"attention":actor.state["attention"],"character_state":actor.state["character_state"],"last_probe":actor.state["last_probe"]}),
                ),
            };
            frame.render_widget(
                Paragraph::new(cast_detail(&value))
                    .scroll((self.detail_scroll, 0))
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered().title(format!("{title} | F9"))),
                panes[1],
            );
        }
        let footer = self.controls_hint();
        frame.render_widget(Paragraph::new(format!("{footer}\nPgUp/Dn game | Ctrl+PgUp/Dn details\nCurrent phase completes; bots and game remain live.")), rows[2]);
    }

    fn controls_hint(&self) -> &'static str {
        if self.stopping || self.debug.mode == crate::cast_debug::Mode::Stopping {
            "Stopping owned bots and saving reports..."
        } else if self.finished {
            "Run finished; artifacts saved. Tab actor | F9 details | Q close"
        } else {
            "F2 hold/continue | F5 step phase | Tab actor | F9 details | Q stop"
        }
    }
}

fn cast_detail(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
    if text.len() > 64_000 {
        let mut end = 64_000;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[Display limited to 64000 bytes; full data remains in run artifacts.]");
    }
    text
}

fn json_value(capture: &Value) -> Value {
    serde_json::json!({"request_revision":capture["request_revision"],"composition":capture["composition"],"messages":capture["payload"]["messages"]})
}

/// Multi-actor display with phase controls available, starting with dispatch running.
pub async fn run_cast(
    manifest: crate::harness::Manifest,
    output: &std::path::Path,
) -> Result<Value> {
    run_cast_debugged(
        manifest,
        output,
        crate::cast_debug::Settings {
            start_held: false,
            ..Default::default()
        },
    )
    .await
}

/// Hold/continue/step only harness phase dispatch. Actors retain independent,
/// live sessions, and game commands remain defined by the JSON manifest.
pub async fn run_cast_debugged(
    manifest: crate::harness::Manifest,
    output: &std::path::Path,
    settings: crate::cast_debug::Settings,
) -> Result<Value> {
    settings.validate_phases(manifest.steps.len())?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err("--harness --tui requires an interactive terminal; omit --tui for CI".into());
    }
    let (events, mut stream) = mpsc::channel(256);
    let (cancel, receiver) = tokio::sync::watch::channel(false);
    let (controls, commands) = mpsc::channel(8);
    let (state, mut debug_state) =
        tokio::sync::watch::channel(crate::cast_debug::Snapshot::default());
    let cast_name = manifest.name.clone();
    let actor_names: Vec<_> = manifest
        .actors
        .iter()
        .map(|actor| actor.name.clone())
        .collect();
    let display = tokio::task::spawn_blocking(move || -> Result<()> {
        enable_raw_mode()?;
        let _restore = Restore;
        execute!(stdout(), EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let mut view = CastView {
            cast_name,
            ..Default::default()
        };
        for name in actor_names {
            view.actors.insert(name, Default::default());
        }
        loop {
            view.sync_debug(debug_state.borrow_and_update().clone());
            loop {
                match stream.try_recv() {
                    Ok(event) => {
                        // Never let an older queued display event set the revision
                        // used for commands or populate a later run's actor panes.
                        view.sync_debug(debug_state.borrow_and_update().clone());
                        view.event(event);
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        view.finished = true;
                        break;
                    }
                }
            }
            if view.finished && view.stopping {
                return Ok(());
            }
            terminal.draw(|f| view.draw(f))?;
            if !event::poll(Duration::from_millis(50))? {
                continue;
            }
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('c')
                        if key.code != KeyCode::Char('c')
                            || key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if view.finished {
                            return Ok(());
                        }
                        view.stopping = true;
                        view.pending_control = None;
                        let _ = cancel.send(true);
                    }
                    _ => {
                        view.sync_debug(debug_state.borrow_and_update().clone());
                        if let Some(command) = view.key(key) {
                            view.send_control(command, &controls);
                        }
                    }
                }
            }
        }
    });
    let result = crate::harness::run_debugged(
        manifest,
        output,
        Some(crate::harness::Observer {
            events: events.clone(),
            cancel: receiver,
        }),
        crate::cast_debug::Controller {
            settings,
            commands,
            state,
            bot_program: None,
        },
    )
    .await;
    let summary = match &result {
        Ok(report) => {
            serde_json::json!({"success":report["success"], "error":report["runs"].as_array()
            .and_then(|runs| runs.iter().find(|r| r["result"]["success"] != true)).map(|r| &r["result"]["error"])})
        }
        Err(error) => serde_json::json!({"success":false,"error":error.to_string()}),
    };
    let _ = events
        .send(serde_json::json!({"kind":"cast-finished","data":summary}))
        .await;
    drop(events);
    display.await??;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn social_panel_and_repl_results_display_only_acknowledged_perspective() {
        let mut view = View::default();
        view.event(serde_json::json!({"kind":"snapshot","data":{"state":{"status":"paused"},"social":{"observer":"Lead","assessment":"RESPECT_WITH_RESENTMENT"}}}));
        view.panel = PANELS
            .iter()
            .position(|p| *p == "Social perspective")
            .unwrap();
        for (w, h) in [(160, 48), (30, 10), (1, 1)] {
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| view.draw(f)).unwrap();
            if w == 160 {
                assert!(
                    terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .map(|c| c.symbol())
                        .collect::<String>()
                        .contains("RESPECT_WITH_RESENTMENT")
                );
            }
        }
        view.event(serde_json::json!({"kind":"social","data":"A recalled background fact"}));
        assert_eq!(view.last_result, "A recalled background fact");
    }
    #[test]
    fn conversation_editor_and_panels_use_acknowledged_actor_state() {
        let mut view = View::default();
        let initial = serde_json::json!({"state":{"status":"listening"},"conversation":{"state":{"policy":crate::conversation::Settings::default()},"phase":"waiting"}});
        view.event(serde_json::json!({"kind":"snapshot","data":initial}));
        assert!(
            matches!(view.key(KeyEvent::new(KeyCode::F(11),KeyModifiers::NONE)).unwrap(),Some(Control::Line(s)) if s=="/pause")
        );
        view.editor.as_mut().unwrap().1 = Editor::new("{\"max_social_requests\":0}".into());
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(view.key(save).is_err());
        view.editor.as_mut().unwrap().1 = Editor::new("{\"reply_min_ms\":950}".into());
        assert!(
            matches!(view.key(save).unwrap(),Some(Control::Conversation(s)) if s.reply_min_ms==950)
        );
        assert_eq!(view.state, initial);
        view.panel = PANELS.iter().position(|p| *p == "Conversation").unwrap();
        for (width, height) in [(60, 20), (160, 48)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| view.draw(f)).unwrap();
        }
    }
    #[test]
    fn budget_editor_validates_without_resetting_usage_or_optimistically_changing_state() {
        let mut view = View::default();
        let f10 = KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE);
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(view.key(f10).is_err());
        assert!(view.editor.is_none());
        let settings = serde_json::to_value(crate::budget::Settings::default()).unwrap();
        let snapshot = serde_json::json!({
            "state":{"status":"running"},
            "budget":{"settings":settings,"revision":"original-budget-revision",
                "status":{"allowed":false,"usage":{"requests":7},"remaining_requests":0}},
            "costs":{"session":{"totals":{"requests":7}}}
        });
        view.event(serde_json::json!({"kind":"snapshot","data":snapshot}));
        assert!(matches!(view.key(f10).unwrap(), Some(Control::Line(line)) if line == "/pause"));
        assert_eq!(view.state, snapshot);
        let (name, editor) = view.editor.as_mut().unwrap();
        assert_eq!(name, "budget");
        assert_eq!(
            serde_json::from_str::<Value>(&editor.text).unwrap(),
            settings
        );
        *editor = Editor::new("{not json}".into());
        assert!(view.key(save).is_err());
        assert!(view.editor.is_some());
        assert_eq!(view.state, snapshot);
        view.editor.as_mut().unwrap().1 = Editor::new("{\"version\":0}".into());
        assert!(view.key(save).is_err());
        assert!(view.editor.is_some());
        view.editor.as_mut().unwrap().1 = Editor::new("{\"max_requests\":20}".into());
        match view.key(save).unwrap() {
            Some(Control::Budget(budget)) => assert_eq!(budget.max_requests, Some(20)),
            _ => panic!("budget editor must emit a validated budget control"),
        }
        assert!(view.editor.is_none());
        assert_eq!(
            view.state, snapshot,
            "only runner snapshots update accepted budget and usage"
        );
        view.panel = PANELS.iter().position(|p| *p == "Budget").unwrap();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 48)).unwrap();
        terminal.draw(|frame| view.draw(frame)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Budget | Ctrl+PgUp/PgDn"));
        assert!(screen.contains("original-budget-revision"));
        assert!(screen.contains("F10 budget"));
        view.event(serde_json::json!({"kind":"budget","data":{"status":"budget-result-marker"}}));
        assert_eq!(view.panel, 6);
        assert!(view.last_result.contains("budget-result-marker"));
        assert_eq!(view.state, snapshot);
    }

    #[test]
    fn cast_cost_panel_includes_selected_budget_and_final_report_refreshes_it() {
        let mut view = CastView::default();
        view.event(serde_json::json!({"kind":"cast-start","data":{"name":"Party","run":1,"actors":["scout","speaker"]}}));
        for actor in ["scout", "speaker"] {
            view.event(serde_json::json!({"actor":actor,"event":{"kind":"snapshot","data":{
                "state":{"status":"paused"},"costs":{"session":{"totals":{"requests":3}}},
                "budget":{"revision":format!("{actor}-old-budget"),"status":{"remaining_requests":1}}
            }}}));
        }
        view.event(serde_json::json!({"actor":"scout","event":{"kind":"result","data":{
            "state":{"status":"done"},"costs":{"session":{"totals":{"requests":4}}},
            "budget":{"revision":"scout-final-budget","status":{"allowed":false,"remaining_requests":0}}
        }}}));
        assert_eq!(
            view.actors["scout"].state["budget"]["status"]["remaining_requests"],
            0
        );
        assert_eq!(
            view.actors["speaker"].state["budget"]["status"]["remaining_requests"],
            1
        );
        view.panel = 2;
        let screen = cast_screen(&view, 160, 48);
        assert!(screen.contains("Session costs and budget"));
        assert!(screen.contains("scout-final-budget"));
        assert!(!screen.contains("speaker-old-budget"));
    }

    #[test]
    fn cast_monitor_keeps_actor_sessions_separate_and_renders_all_details() {
        let mut view = CastView::default();
        view.event(serde_json::json!({"kind":"cast-start","data":{"name":"Lanterns","run":1,"actors":["scout","medic"]}}));
        view.event(serde_json::json!({"actor":"scout","event":{"kind":"observation","data":"Scout private session"}}));
        view.event(serde_json::json!({"actor":"medic","event":{"kind":"observation","data":"Medic private session"}}));
        assert!(!view.actors["scout"].game.contains("Medic"));
        view.event(serde_json::json!({"kind":"cast-finished","data":{"success":false,"error":"phase timed out"}}));
        assert_eq!(view.phase, "Failed: phase timed out");
        for (w, h) in [(40, 10), (80, 24), (160, 48)] {
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            for selected in 0..2 {
                for panel in 0..CAST_PANELS {
                    view.selected = selected;
                    view.panel = panel;
                    terminal.draw(|frame| view.draw(frame)).unwrap();
                }
            }
        }
    }

    fn phase_state(
        run: usize,
        revision: u64,
        mode: crate::cast_debug::Mode,
        next_phase: Option<usize>,
    ) -> crate::cast_debug::Snapshot {
        crate::cast_debug::Snapshot {
            run,
            revision,
            mode,
            next_phase,
            phase_count: 3,
            held_ms: 1000,
            max_hold_ms: 9000,
            notice: "Current phase remains live.",
        }
    }

    #[test]
    fn cast_phase_keys_use_acknowledged_revisions_and_never_optimistically_change_mode() {
        use crate::cast_debug::{Action, Mode};
        let mut view = CastView::default();
        let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert!(view.key(f2).is_none());
        view.sync_debug(phase_state(2, 11, Mode::Running, Some(1)));
        assert!(view.key(f5).is_none());
        let command = view.key(f2).unwrap();
        assert_eq!((command.run, command.revision), (2, 11));
        assert!(matches!(command.action, Action::Hold));
        let (sender, mut receiver) = mpsc::channel(1);
        view.send_control(command, &sender);
        assert_eq!(view.debug.mode, Mode::Running);
        assert!(view.notice.contains("awaiting"));
        assert!(view.key(f2).is_none());
        assert!(matches!(receiver.try_recv().unwrap().action, Action::Hold));
        view.sync_debug(phase_state(2, 12, Mode::HoldRequested, Some(2)));
        assert!(view.pending_control.is_none());
        assert!(matches!(view.key(f2).unwrap().action, Action::Continue));
        assert!(view.key(f5).is_none());
        view.sync_debug(phase_state(2, 13, Mode::Held, Some(2)));
        let command = view.key(f5).unwrap();
        assert_eq!(command.revision, 13);
        assert!(matches!(command.action, Action::Step));
        assert!(view.dispatch_status().contains("hold limit in 8s"));
        view.sync_debug(phase_state(2, 14, Mode::Stepping, Some(3)));
        assert!(view.key(f5).is_none());
        assert!(matches!(view.key(f2).unwrap().action, Action::Continue));
        view.sync_debug(phase_state(2, 15, Mode::Held, None));
        assert!(view.key(f5).is_none());
        assert!(view.notice.contains("final boundary"));
        assert!(matches!(view.key(f2).unwrap().action, Action::Continue));
        for stopped in [true, false] {
            view.stopping = stopped;
            view.finished = !stopped;
            assert!(view.key(f2).is_none());
            assert!(view.key(f5).is_none());
        }
        view.finished = false;
        view.sync_debug(phase_state(2, 16, Mode::Stopping, None));
        assert!(view.key(f2).is_none());
        assert!(view.key(f5).is_none());
        assert!(view.dispatch_status().contains("stopping"));
        for (width, height) in [(40, 10), (100, 24)] {
            let screen = cast_screen(&view, width, height);
            assert!(screen.contains("Stopping owned bots"));
            assert!(!screen.contains("F2 hold/continue"));
        }
        view.sync_debug(phase_state(2, 16, Mode::Finished, None));
        assert!(view.key(f2).is_none());
    }

    #[test]
    fn cast_queue_busy_and_stale_rejections_remain_visible_without_replaying_commands() {
        use crate::cast_debug::{Action, Command, Mode};
        let mut view = CastView::default();
        view.sync_debug(phase_state(1, 1, Mode::Running, Some(1)));
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .try_send(Command {
                run: 1,
                revision: 1,
                action: Action::Hold,
            })
            .unwrap();
        let command = view
            .key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
            .unwrap();
        view.send_control(command, &sender);
        assert!(view.notice.contains("queue is busy"));
        assert!(view.pending_control.is_none());
        receiver.try_recv().unwrap();
        let command = view
            .key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
            .unwrap();
        view.send_control(command, &sender);
        let mut rejected = phase_state(1, 2, Mode::Running, Some(1));
        rejected.notice = "Rejected: stale command revision.";
        view.sync_debug(rejected);
        assert!(view.pending_control.is_none());
        assert_eq!(view.debug.notice, "Rejected: stale command revision.");
        let command = view
            .key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(command.revision, 2);
        drop(receiver);
        view.send_control(command, &sender);
        assert!(view.notice.contains("channel is closed"));
        assert!(view.pending_control.is_none());
    }

    fn cast_screen(view: &CastView, width: u16, height: u16) -> String {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| view.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn cast_debug_panes_preserve_actor_and_run_boundaries_even_when_display_events_are_stale() {
        use crate::cast_debug::Mode;
        let mut view = CastView {
            cast_name: "Lanterns".into(),
            ..Default::default()
        };
        view.sync_debug(phase_state(1, 1, Mode::Held, Some(1)));
        view.event(serde_json::json!({"run":1,"kind":"cast-start","data":{"name":"Lanterns","run":1,"actors":["scout","medic"]}}));
        for actor in ["scout", "medic"] {
            view.event(
                serde_json::json!({"run":1,"actor":actor,"event":{"kind":"snapshot","data":{
                    "state":{"memory":format!("{actor}_PRIVATE_MEMORY")},
                    "debug":{"marker":format!("{actor}_PRIVATE_DEBUG")},
                    "last_probe":{"probe_id":format!("{actor}_PRIVATE_PROBE")}
                }}}),
            );
            view.event(serde_json::json!({"run":1,"actor":actor,"event":{"kind":"observation","data":format!("{actor} game room")}}));
        }
        view.selected = view.actors.keys().position(|name| name == "scout").unwrap();
        for (panel, marker) in [(4, "MEMORY"), (5, "DEBUG"), (6, "PROBE")] {
            view.panel = panel;
            let screen = cast_screen(&view, 160, 48);
            assert!(screen.contains(&format!("scout_PRIVATE_{marker}")));
            assert!(!screen.contains(&format!("medic_PRIVATE_{marker}")));
        }
        let small = cast_screen(&view, 45, 10);
        assert!(small.contains("Resize to 55 x 16"));
        assert!(small.contains("Bots and game remain live"));
        view.sync_debug(phase_state(2, 3, Mode::Running, Some(1)));
        assert!(view.actors["scout"].game.is_empty());
        assert!(view.actors["scout"].state.is_null());
        view.event(serde_json::json!({"run":1,"actor":"scout","event":{"kind":"snapshot","data":{"state":{"memory":"STALE_MEMORY"}}}}));
        view.event(serde_json::json!({"run":1,"kind":"phase","data":{"id":"old_phase"}}));
        assert!(view.actors["scout"].state.is_null());
        assert!(view.phase.is_empty());
        view.event(serde_json::json!({"run":2,"actor":"scout","event":{"kind":"observation","data":"New run room"}}));
        assert_eq!(view.actors["scout"].game, "New run room");
        view.event(
            serde_json::json!({"run":2,"actor":"_harness","event":{"kind":"cast-debug","data":{}}}),
        );
        assert!(!view.actors.contains_key("_harness"));
        assert!(cast_detail(&serde_json::json!({"text":"🌹".repeat(20000)})).len() < 64_100);
    }
    #[test]
    fn editor_handles_unicode_and_multiline_without_splitting_characters() {
        let mut editor = Editor::new("a🌹\nrose".into());
        editor.cursor = "a🌹".len();
        editor.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
        assert_eq!(editor.text, "a\nrose");
        editor.insert("é");
        editor.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), true);
        editor.insert("!");
        assert_eq!(editor.text, "aé\nro!se");
    }
    #[test]
    fn terminal_layout_renders_at_small_and_normal_sizes() {
        for (w, h) in [(20, 8), (80, 24), (160, 48)] {
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            let mut view = View {
                game: "Market Square\nObvious exits: east west".into(),
                ..View::default()
            };
            terminal.draw(|f| view.draw(f)).unwrap();
            view.editor = Some((
                "prompts".into(),
                Editor::new("{\n  \"name\": \"rose\"\n}".into()),
            ));
            terminal.draw(|f| view.draw(f)).unwrap();
        }
    }
}
