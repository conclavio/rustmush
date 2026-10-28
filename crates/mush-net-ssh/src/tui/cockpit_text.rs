//! **The text cockpit** (2026-10-22, the owner: "the player is going to need
//! to be able to read and send chat messages while navigating … the bottom
//! third could all be actual terminal text"): the screen the pixel view
//! draws around its picture.
//!
//! - Row 1: the view's status line.
//! - The top two-thirds of the rest ([`super::pixel::picture_area`]): the
//!   picture, the flight HUD alone over it (heading tape, reticle, speed
//!   bar). The text frame clears those cells and never writes into them.
//! - The text panel: the **armour doll** in Unicode blocks coloured by what
//!   is left of each part, the **weapons** (ready or cycling, rounds, the
//!   hit chance) under the **heat** bar, the **contacts** with an arrow for
//!   where each one lies from the camera, and a column of the arena's
//!   **notices** over the **comms** (the scrollback's tail: says, poses,
//!   channels), so chat is read while flying.
//! - The chat line: Enter opens it and it owns the keys (typing never
//!   steers) until Enter sends the line and goes back to flying, or Esc goes
//!   back keeping the draft.
//! - The key legend.
//!
//! Everything here is a pure function of the cockpit frame and what the
//! app hands it; the app (`TuiApp::draw_pixel`) wires in the scrollback,
//! the notices ring and the line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;

use mush_pixel::look;

use super::pixel::picture_area;
use super::render::PaneRows;

/// The armour column's width: the doll and a section a row.
const ARMOUR_W: u16 = 26;
/// The arena notices shown over the comms, at most.
const NOTICE_ROWS: u16 = 3;
/// What the chat line starts with.
pub const CHAT_PROMPT: &str = "\u{203a} ";
/// The chat line's key legend.
pub const CHAT_LEGEND: &str = "Enter: send and fly   Esc: back to flying (keeps the draft)   \u{2191}/\u{2193}: history   ^]: detach";

/// The screen's rows while the view is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Areas {
    /// The status line.
    pub status: Rect,
    /// The picture's rows (the picture is centred across them).
    pub picture: Rect,
    /// The text panel.
    pub panel: Rect,
    /// The chat line.
    pub input: Rect,
    /// The key legend.
    pub legend: Rect,
}

/// Where everything goes in `area`, the whole terminal: the picture's rows
/// are [`picture_area`]'s, so the text never touches the picture.
#[must_use]
pub fn areas(area: Rect) -> Areas {
    let (_, _, _, picture_rows) = picture_area(area.width, area.height);
    let [status, picture, panel, input, legend] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(picture_rows),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    Areas {
        status,
        picture,
        panel,
        input,
        legend,
    }
}

/// What the panel draws from.
pub struct Panel<'a> {
    /// The held `arena.cockpit` frame.
    pub cockpit: Option<&'a Value>,
    /// The camera's heading, millidegrees (the contacts' arrows).
    pub camera_mdeg: i64,
    /// Whether the terminal shows UTF-8.
    pub utf8: bool,
}

/// Draw the text panel into `area`: armour, weapons and heat, contacts, and
/// the notices over the comms. `notices(w, h)` is the arena's notices ring
/// that fits `w` by `h`; `comms(w)` is the scrollback wrapped at `w`.
pub fn draw_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    panel: &Panel<'_>,
    notices: &dyn Fn(u16, u16) -> PaneRows,
    comms: &dyn Fn(usize) -> Vec<Line<'static>>,
) {
    if area.height < 2 {
        return;
    }
    let seated = panel
        .cockpit
        .filter(|c| c.get("seated").and_then(Value::as_bool) != Some(false));
    let detachment = seated
        .and_then(|c| c.get("detachment"))
        .and_then(super::detachment::decode);
    let (armour, weapons, contacts, talk) = if let Some(detachment) = &detachment {
        let [armour, weapons, contacts, roster, talk] = Layout::horizontal([
            Constraint::Length(ARMOUR_W),
            Constraint::Fill(3),
            Constraint::Fill(3),
            Constraint::Fill(4),
            Constraint::Fill(4),
        ])
        .spacing(1)
        .areas(area);
        // Keep the recipient's current order above the roster at short terminal
        // heights. The text cockpit has the complete, wrapped command strings.
        let mut lines = vec![Line::from("p: roster + commands")];
        if let Some(order) = &detachment.my_order {
            lines.push(Line::from(format!("Order: {}", order.objective)));
            lines.push(Line::from(format!("From: {}", order.from)));
            lines.push(Line::from(format!(
                "Reply: {}",
                order
                    .response
                    .as_ref()
                    .map_or_else(|| "none".into(), |r| r.to_uppercase())
            )));
        }
        lines.extend(
            detachment
                .lines()
                .into_iter()
                .take(detachment.slots.len() + 1)
                .map(Line::from),
        );
        column(frame, roster, "DETACHMENT", lines, false);
        (armour, weapons, contacts, talk)
    } else {
        let [armour, weapons, contacts, talk] = Layout::horizontal([
            Constraint::Length(ARMOUR_W),
            Constraint::Fill(3),
            Constraint::Fill(3),
            Constraint::Fill(4),
        ])
        .spacing(1)
        .areas(area);
        (armour, weapons, contacts, talk)
    };
    let (armour_lines, weapon_lines, contact_lines, title) = match seated {
        Some(c) => (
            armour_rows(c, panel.utf8, usize::from(armour.height.saturating_sub(1))),
            weapon_rows(c, panel.utf8),
            contact_rows(c, panel.camera_mdeg, panel.utf8),
            format!(
                "CONTACTS ({})",
                c.get("contacts")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)
            ),
        ),
        None => (
            vec![dim("No cockpit frame held: take a seat in the arena.")],
            Vec::new(),
            Vec::new(),
            "CONTACTS".to_string(),
        ),
    };
    let armour_title = seated
        .and_then(|c| c.get("callsign").and_then(Value::as_str))
        .map_or_else(
            || "ARMOUR".to_string(),
            |cs| {
                let power = seated
                    .and_then(|c| c.get("power").and_then(Value::as_str))
                    .unwrap_or("?");
                format!("ARMOUR {cs} {power}")
            },
        );
    column(frame, armour, &armour_title, armour_lines, false);
    column(frame, weapons, "WEAPONS / HEAT", weapon_lines, false);
    column(frame, contacts, &title, contact_lines, false);

    // The notices over the comms.
    let inner_h = talk.height.saturating_sub(1);
    let notice_h = if inner_h >= 9 { NOTICE_ROWS } else { 1 }.min(inner_h / 2);
    let [notice_area, comms_area] =
        Layout::vertical([Constraint::Length(notice_h + 1), Constraint::Min(0)]).areas(talk);
    let notice_lines: Vec<Line<'static>> = notices(notice_area.width, notice_h)
        .rows
        .into_iter()
        .map(|r| r.line)
        .collect();
    column(frame, notice_area, "NOTICES", notice_lines, true);
    let block = header("COMMS");
    let inner = block.inner(comms_area);
    frame.render_widget(block, comms_area);
    let rows = comms(usize::from(inner.width));
    let skip = rows.len().saturating_sub(usize::from(inner.height));
    frame.render_widget(
        Paragraph::new(rows.into_iter().skip(skip).collect::<Vec<_>>()),
        inner,
    );
}

/// **The chart's text panel** (doc 21 cut G.5b): under the orbital view,
/// the chart's own passages-and-window rows (`table(w, h)`, the chart
/// screen's right pane) beside the comms (`comms(w)`, the scrollback's tail).
pub fn draw_chart_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    table: &dyn Fn(u16, u16) -> PaneRows,
    comms: &dyn Fn(usize) -> Vec<Line<'static>>,
) {
    if area.height < 2 {
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)])
        .spacing(1)
        .areas(area);
    let rows: Vec<Line<'static>> = table(left.width, left.height.saturating_sub(1))
        .rows
        .into_iter()
        .map(|r| r.line)
        .collect();
    column(frame, left, "PASSAGES", rows, false);
    let block = header("COMMS");
    let inner = block.inner(right);
    frame.render_widget(block, right);
    let lines = comms(usize::from(inner.width));
    let skip = lines.len().saturating_sub(usize::from(inner.height));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        inner,
    );
}

/// One titled pane of [`draw_replay_panel`]: its title and its rows at a
/// width and height.
pub type TitledPane<'a> = (&'a str, &'a dyn Fn(u16, u16) -> PaneRows);

/// **The replay's panel** (doc 21 cut G.8): under the replay picture, the
/// replay's own panes as screen 0 draws them at the keyframe the picture
/// stands at or after — the match's sentences, the shots, the wrecks — and
/// the comms.
pub fn draw_replay_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    panes: &[TitledPane<'_>],
    comms: &dyn Fn(usize) -> Vec<Line<'static>>,
) {
    if area.height < 2 {
        return;
    }
    let [left, right] = Layout::horizontal([Constraint::Fill(8), Constraint::Fill(3)])
        .spacing(1)
        .areas(area);
    let cols = Layout::horizontal(panes.iter().map(|_| Constraint::Fill(1)))
        .spacing(1)
        .split(left);
    for ((title, rows), col) in panes.iter().zip(cols.iter()) {
        let lines: Vec<Line<'static>> = rows(col.width, col.height.saturating_sub(1))
            .rows
            .into_iter()
            .map(|r| r.line)
            .collect();
        column(frame, *col, title, lines, false);
    }
    let block = header("COMMS");
    let inner = block.inner(right);
    frame.render_widget(block, right);
    let lines = comms(usize::from(inner.width));
    let skip = lines.len().saturating_sub(usize::from(inner.height));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        inner,
    );
}

/// The chat line in `area`: while chatting, the prompt, the line and the
/// cursor (`cursor` characters in); flying, how to open it and any draft
/// Esc kept.
pub fn draw_input(
    frame: &mut Frame<'_>,
    area: Rect,
    chatting: bool,
    text: &str,
    cursor: usize,
    draft: Option<&str>,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    if chatting {
        let line = Line::from(vec![
            Span::styled(CHAT_PROMPT, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(text.to_string()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        let used =
            (CHAT_PROMPT.chars().count() + cursor).min(usize::from(area.width.saturating_sub(1)));
        frame.set_cursor_position(Position {
            x: area.x + u16::try_from(used).unwrap_or(0),
            y: area.y,
        });
    } else {
        let words = match draft {
            Some(d) => format!("Enter: chat (draft: {d})"),
            None => "Enter: chat".to_string(),
        };
        frame.render_widget(Paragraph::new(dim(&words)), area);
    }
}

/// A column: a top rule carrying its title, the lines under it, wrapped or
/// cut at the width.
fn column(frame: &mut Frame<'_>, area: Rect, title: &str, lines: Vec<Line<'static>>, wrap: bool) {
    let block = header(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut para = Paragraph::new(lines);
    if wrap {
        para = para.wrap(Wrap { trim: false });
    }
    frame.render_widget(para, inner);
}

fn header(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().add_modifier(Modifier::DIM))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().add_modifier(Modifier::BOLD),
        ))
}

fn dim(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().add_modifier(Modifier::DIM),
    ))
}

/// The UTF-8 text, or its ASCII stand-in on a terminal without UTF-8.
fn glyphs(s: &str, utf8: bool) -> String {
    if utf8 {
        return s.to_string();
    }
    s.chars()
        .map(|c| match c {
            '\u{2591}' => '.',
            '\u{2191}' => '^',
            '\u{2193}' => 'v',
            '\u{2190}' => '<',
            '\u{2192}' => '>',
            '\u{2197}' | '\u{2199}' => '/',
            '\u{2196}' | '\u{2198}' => '\\',
            c if c.is_ascii() => c,
            _ => '#',
        })
        .collect()
}

fn num(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

/// Green, yellow, red or grey by what is left of a thing, as the picture's
/// instruments ramp it.
fn ramp(left: i64, max: i64) -> Color {
    if max <= 0 || left <= 0 {
        Color::DarkGray
    } else if left * 3 >= max * 2 {
        Color::Green
    } else if left * 3 >= max {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// A bar `cells` wide, filled by `left` of `max` (a sliver shows while any
/// is left).
fn bar(left: i64, max: i64, cells: usize) -> String {
    let filled = if max <= 0 || left <= 0 {
        0
    } else {
        usize::try_from((left.min(max) * cells as i64 + max - 1) / max).unwrap_or(cells)
    };
    format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(cells - filled.min(cells))
    )
}

// ---- the armour doll -------------------------------------------------------

/// A section's internals left and their most; a destroyed section has none.
fn internals(sec: &Value) -> (i64, i64) {
    let max = num(sec.get("internals_max"));
    if sec.get("condition").and_then(Value::as_str) == Some("destroyed") {
        (0, max)
    } else {
        (num(sec.get("internals")), max)
    }
}

/// **The doll** (doc 22 D-22-2): the grid of the pilot's own unit's look,
/// found by the frame's `unit_type`, drawn in half-blocks — two grid rows to
/// a text row, the upper cell the glyph's colour and the lower its
/// background — each cell coloured by what is left of its part (a facet's
/// armour, or its section's internals) — touching parts in the same state
/// told apart by a lighter tone ([`look::Doll::tones`]) — centred in
/// `width`. Nothing for a
/// unit type with no look, or a look with no doll.
fn doll_rows(cockpit: &Value, width: usize, utf8: bool) -> Vec<Line<'static>> {
    let Some(doll) = cockpit
        .get("unit_type")
        .and_then(Value::as_str)
        .and_then(look::for_unit)
        .and_then(|l| l.doll.as_ref())
    else {
        return Vec::new();
    };
    let condition = doll.condition(cockpit.get("sections").unwrap_or(&Value::Null));
    let tones = doll.tones();
    let colour = |x: usize, y: usize| -> Option<Color> {
        if y >= doll.height {
            return None;
        }
        let part = (*doll.cells.get(y * doll.width + x)?)?;
        let base = match condition.get(part).copied().flatten() {
            Some((left, max)) => ramp(left, max),
            None => Color::DarkGray,
        };
        // Touching parts in the same state are told apart by tone.
        Some(match (tones.get(part), base) {
            (Some(1), Color::Green) => Color::LightGreen,
            (Some(1), Color::Yellow) => Color::LightYellow,
            (Some(1), Color::Red) => Color::LightRed,
            (Some(1), Color::DarkGray) => Color::Gray,
            (_, c) => c,
        })
    };
    let pad = " ".repeat(width.saturating_sub(doll.width) / 2);
    (0..doll.height)
        .step_by(2)
        .map(|y| {
            let mut spans = vec![Span::raw(pad.clone())];
            spans.extend((0..doll.width).map(|x| {
                let (glyph, style) = match (colour(x, y), colour(x, y + 1)) {
                    (None, None) => (" ", Style::default()),
                    (Some(top), None) => ("\u{2580}", Style::default().fg(top)),
                    (None, Some(low)) => ("\u{2584}", Style::default().fg(low)),
                    (Some(top), Some(low)) => ("\u{2580}", Style::default().fg(top).bg(low)),
                };
                Span::styled(glyphs(glyph, utf8), style)
            }));
            Line::from(spans)
        })
        .collect()
}

/// A section's short name: the tracks as `L track` and `R track`, others
/// with spaces for underscores, cut to ten.
fn short(id: &str) -> String {
    let name = match id {
        "left_track" => "L track".to_string(),
        "right_track" => "R track".to_string(),
        other => other.replace('_', " "),
    };
    name.chars().take(10).collect()
}

/// The armour column: the doll from the unit's look ([`doll_rows`]) and a
/// row a section — its name, a bar of its internals, and the numbers — or,
/// where those would not fit in `rows`, two sections a row with shorter
/// bars and no numbers (the dev2 smoke: a biped's eight doll rows and six
/// sections left its torso off the bottom).
#[must_use]
pub fn armour_rows(cockpit: &Value, utf8: bool, rows: usize) -> Vec<Line<'static>> {
    let mut lines = doll_rows(cockpit, usize::from(ARMOUR_W), utf8);
    let sections: Vec<&Value> = cockpit
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    if lines.len() + sections.len() > rows {
        for pair in sections.chunks(2) {
            let mut spans = Vec::new();
            for sec in pair {
                let (left, max) = internals(sec);
                let id = sec.get("id").and_then(Value::as_str).unwrap_or("?");
                let name: String = short(id).chars().take(8).collect();
                spans.push(Span::raw(format!("{name:<8} ")));
                spans.push(Span::styled(
                    glyphs(&bar(left, max, 3), utf8),
                    Style::default().fg(ramp(left, max)),
                ));
                spans.push(Span::raw(" "));
            }
            lines.push(Line::from(spans));
        }
        return lines;
    }
    for sec in sections {
        let (left, max) = internals(sec);
        let id = sec.get("id").and_then(Value::as_str).unwrap_or("?");
        lines.push(Line::from(vec![
            Span::raw(format!("{:<10} ", short(id))),
            Span::styled(
                glyphs(&bar(left, max, 5), utf8),
                Style::default().fg(ramp(left, max)),
            ),
            Span::raw(format!(" {left}/{max}")),
        ]));
    }
    lines
}

// ---- weapons and heat ------------------------------------------------------

/// The weapons column: the heat bar, then a row a weapon — its name,
/// `READY` or the seconds to its next shot (`EMPTY` with no rounds), its
/// rounds and the hit chance on the lock.
#[must_use]
pub fn weapon_rows(cockpit: &Value, utf8: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match cockpit.get("heat").filter(|h| !h.is_null()) {
        Some(h) => {
            let (heat, max) = (num(h.get("heat")), num(h.get("max")));
            let colour = if h.get("overheated").and_then(Value::as_bool) == Some(true) {
                Color::Red
            } else {
                ramp(max - heat, max)
            };
            let mut spans = vec![
                Span::raw("HEAT "),
                Span::styled(
                    glyphs(&bar(heat, max, 10), utf8),
                    Style::default().fg(colour),
                ),
                Span::raw(format!(" {heat}/{max}")),
            ];
            if h.get("overheated").and_then(Value::as_bool) == Some(true) {
                spans.push(Span::styled(
                    " OVERHEAT",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::from(spans));
        }
        None => lines.push(dim("HEAT - (no heat model)")),
    }
    let weapons: Vec<&Value> = cockpit
        .get("weapons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let name_w = weapons
        .iter()
        .filter_map(|w| w.get("id").and_then(Value::as_str))
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(0)
        .min(12);
    for w in weapons {
        let name: String = w
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .chars()
            .take(12)
            .collect();
        let (rounds, rounds_max) = (num(w.get("rounds")), num(w.get("rounds_max")));
        let (state, colour) = if rounds_max > 0 && rounds <= 0 {
            ("EMPTY".to_string(), Color::Red)
        } else if w.get("ready").and_then(Value::as_bool) == Some(true) {
            ("READY".to_string(), Color::Green)
        } else {
            let ms = num(w.get("ready_in_ms"));
            (
                format!("{}.{}s", ms / 1_000, ms % 1_000 / 100),
                Color::Yellow,
            )
        };
        let mut spans = vec![
            Span::raw(format!("{name:<name_w$} ")),
            Span::styled(format!("{state:<6}"), Style::default().fg(colour)),
            Span::styled(
                format!("{rounds:>4}/{rounds_max:<4}"),
                Style::default().fg(ramp(rounds, rounds_max)),
            ),
        ];
        if let Some(p) = w.get("percent").and_then(Value::as_i64) {
            spans.push(Span::raw(format!(" {p:>3}%")));
        }
        lines.push(Line::from(spans));
    }
    lines
}

// ---- contacts --------------------------------------------------------------

/// Where a bearing lies from the camera, as an arrow (up is ahead).
fn arrow(bearing_deg: f64, camera_mdeg: i64) -> &'static str {
    let rel = (bearing_deg * 1_000.0).round() as i64 - camera_mdeg;
    let octant = (rel + 22_500).rem_euclid(360_000) / 45_000;
    [
        "\u{2191}", "\u{2197}", "\u{2192}", "\u{2198}", "\u{2193}", "\u{2199}", "\u{2190}",
        "\u{2196}",
    ]
    .get(usize::try_from(octant).unwrap_or(0))
    .copied()
    .unwrap_or("\u{2191}")
}

/// The contacts column, nearest first: an arrow for where each lies from
/// the camera, its designator and class, range and bearing, the best hit
/// chance on it, `LOCK` on the lock and `friend` on one of the pilot's own
/// side; coloured by **team** (doc 22 D-22-3: red, blue, unknown grey, a wreck
/// dark), so the colour never says friend or foe — the word does.
#[must_use]
pub fn contact_rows(cockpit: &Value, camera_mdeg: i64, utf8: bool) -> Vec<Line<'static>> {
    let lock = cockpit.get("lock").and_then(Value::as_str);
    let mut contacts: Vec<&Value> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let range = |c: &Value| c.get("range_m").and_then(Value::as_f64).unwrap_or(0.0);
    contacts.sort_by(|a, b| range(a).total_cmp(&range(b)));
    if contacts.is_empty() {
        return vec![dim("No contacts.")];
    }
    contacts
        .into_iter()
        .map(|c| {
            let bearing = c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0);
            let designator = c.get("designator").and_then(Value::as_str).unwrap_or("?");
            let class: String = c
                .get("class")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .chars()
                .take(8)
                .collect();
            let colour = match mush_pixel::cockpit::contact_kind(c) {
                mush_pixel::raycast::SpriteKind::Wreck => Color::DarkGray,
                mush_pixel::raycast::SpriteKind::Blue => Color::LightBlue,
                mush_pixel::raycast::SpriteKind::Red => Color::LightRed,
                mush_pixel::raycast::SpriteKind::Unknown => Color::Gray,
            };
            let best = c
                .get("odds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| o.get("percent").and_then(Value::as_i64))
                .max();
            let mut text = format!(
                "{} {designator:<3} {class:<8} {:>5.0}m {:03.0}\u{b0}",
                arrow(bearing, camera_mdeg),
                range(c),
                bearing.rem_euclid(360.0),
            );
            if let Some(p) = best {
                text.push_str(&format!(" {p:>2}%"));
            }
            let mut spans = vec![Span::styled(
                glyphs(&text, utf8),
                Style::default().fg(colour),
            )];
            if c.get("friendly").and_then(Value::as_bool) == Some(true) {
                spans.push(Span::styled(
                    " friend",
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            if lock.is_some() && c.get("id").and_then(Value::as_str) == lock {
                spans.push(Span::styled(
                    " LOCK",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture")
    }

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// The picture takes the top two-thirds under the status line; the
    /// panel, the chat line and the legend share the rest, and nothing
    /// overlaps.
    #[test]
    fn the_picture_takes_the_top_two_thirds() {
        let a = areas(Rect::new(0, 0, 160, 50));
        assert_eq!(a.status, Rect::new(0, 0, 160, 1));
        assert_eq!(a.picture, Rect::new(0, 1, 160, 32));
        assert_eq!(a.panel, Rect::new(0, 33, 160, 15));
        assert_eq!(a.input, Rect::new(0, 48, 160, 1));
        assert_eq!(a.legend, Rect::new(0, 49, 160, 1));
        let (col, row, cols, rows) = picture_area(160, 50);
        assert_eq!(
            (col - 1, row - 1, cols, rows),
            (a.picture.x, a.picture.y, a.picture.width, a.picture.height),
            "the picture's cells are the text frame's picture rows"
        );
    }

    /// The colour each doll cell of `row` (a text row) is drawn in: the
    /// glyph's and its background, for the cell `x` cells into the doll.
    fn cell_colours(line: &Line<'_>, x: usize) -> (Option<Color>, Option<Color>) {
        // The first span is the centring pad.
        let span = &line.spans[1 + x];
        (span.style.fg, span.style.bg)
    }

    /// Doc 22 D-22-2: the tank's doll is its look's 24x16 grid in eight
    /// half-block rows, centred, each cell coloured by what is left of its
    /// part — the fixture's worn left track (6 of 24) red, its destroyed
    /// right track grey, its front facet (55 of 80) green until worn past a
    /// third — then a row a section.
    #[test]
    fn the_doll_is_the_looks_grid_coloured_by_what_is_left() {
        let mut c = fixture();
        c["unit_type"] = "tacsim.unit.line_tank".into();
        let lines = armour_rows(&c, true, 99);
        let rows = texts(&lines);
        assert_eq!(rows.len(), 8 + 5, "the doll and a row a section");
        let top = "\u{2580}";
        assert_eq!(
            rows[0],
            format!(" {t}{t}{t} {f} {t}{t}{t}", t = top, f = top.repeat(16))
        );
        assert_eq!(
            cell_colours(&lines[0], 0),
            (Some(Color::Red), Some(Color::Red))
        );
        assert_eq!(
            cell_colours(&lines[0], 22),
            (Some(Color::DarkGray), Some(Color::DarkGray))
        );
        assert_eq!(
            cell_colours(&lines[0], 5),
            (Some(Color::Green), Some(Color::Green))
        );
        let mut worn = c.clone();
        worn["sections"][1]["facets"][0]["armor"] = 40.into();
        let worn = armour_rows(&worn, true, 99);
        assert_eq!(cell_colours(&worn[0], 5).0, Some(Color::Yellow), "40 of 80");
        assert_eq!(
            rows[8],
            "engine     \u{2588}\u{2588}\u{2588}\u{2588}\u{2588} 40/40"
        );
        assert_eq!(
            rows[10],
            "L track    \u{2588}\u{2588}\u{2591}\u{2591}\u{2591} 6/24"
        );
        let ascii = texts(&armour_rows(&c, false, 99));
        assert!(ascii.iter().all(|r| r.is_ascii()), "{ascii:?}");
    }

    /// The biped gets its own, upright doll (the hard-coded tank doll gave it
    /// none); a unit type with no look gets the section rows alone.
    #[test]
    fn a_biped_gets_its_own_doll_and_a_lookless_unit_none() {
        let section = |id: &str, left: i64| serde_json::json!({ "id": id, "condition": "intact", "internals": left, "internals_max": 100, "facets": [] });
        let biped = serde_json::json!({
            "unit_type": "tacsim.unit.biped_mk1",
            "sections": [
                section("torso", 100), section("head", 100), section("left_arm", 20),
                section("right_arm", 100), section("left_leg", 100), section("right_leg", 100)
            ]
        });
        let lines = armour_rows(&biped, true, 99);
        assert_eq!(lines.len(), 8 + 6);
        // Row 1 holds grid rows 2 and 3: the left arm at cells 3..7, red.
        // The arm is red at either tone; the whole torso beside it is green,
        // in the tone that tells it from the arms and the head.
        assert!(matches!(
            cell_colours(&lines[1], 3).0,
            Some(Color::Red | Color::LightRed)
        ));
        let torso = cell_colours(&lines[1], 10);
        assert!(
            matches!(torso.0, Some(Color::Green | Color::LightGreen)) && torso.0 == torso.1,
            "{torso:?}"
        );
        assert_ne!(
            cell_colours(&lines[1], 10).0,
            cell_colours(&lines[0], 10).0,
            "the head and the torso touch, so their tones differ"
        );
        // In the smoke's 13 rows the six sections pair up, so the torso
        // shows.
        let fitted = texts(&armour_rows(&biped, true, 13));
        assert_eq!(fitted.len(), 8 + 3);
        assert!(fitted.iter().any(|r| r.contains("torso")), "{fitted:?}");
        let mut hauler = biped.clone();
        hauler["unit_type"] = "tacsim.unit.dray".into();
        assert_eq!(
            armour_rows(&hauler, true, 99).len(),
            6,
            "no look: sections only"
        );
    }

    #[test]
    fn the_weapons_show_heat_readiness_rounds_and_odds() {
        let c = fixture();
        assert_eq!(
            texts(&weapon_rows(&c, true)),
            [
                "HEAT \u{2588}\u{2588}\u{2588}\u{2588}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591} 40/100",
                "cannon      READY   14/20    62%",
                "machine_gun 0.9s   380/500   21%",
            ]
        );
        let mut cold = c.clone();
        cold["heat"] = Value::Null;
        cold["weapons"][0]["rounds"] = 0.into();
        let rows = texts(&weapon_rows(&cold, true));
        assert_eq!(rows[0], "HEAT - (no heat model)");
        assert!(rows[1].starts_with("cannon      EMPTY"), "{rows:?}");
    }

    /// Nearest first, an arrow for where each lies from the camera (the
    /// fixture faces 045: BA dead ahead, RB at 270 behind-left), the lock
    /// marked, a teammate marked `friend` (doc 22: its colour is its team's,
    /// red, as the pilot's own is).
    #[test]
    fn the_contacts_point_from_the_camera_nearest_first() {
        let c = fixture();
        assert_eq!(
            texts(&contact_rows(&c, 45_000, true)),
            [
                "\u{2199} RB  red        100m 270\u{b0} friend",
                "\u{2190} X3  unknown    200m 315\u{b0}",
                "\u{2191} BA  blue       450m 045\u{b0} 62% LOCK",
                "\u{2191} X4  unknown    600m 030\u{b0}",
            ]
        );
        // A look of 90 to the right: BA now lies to the left.
        let swung = texts(&contact_rows(&c, 135_000, true));
        assert!(swung[2].starts_with('\u{2190}'), "{swung:?}");
    }
}
