//! Rendering for the modal TUI (interface plan I2): [`OutputEvent`]s to
//! scrollback [`Entry`]s with a speaker gutter, panel keyframes to
//! [`PaneRows`], the chrome (tab strip, status line, vitals strip, legend),
//! and span wrapping with a hanging indent.
//!
//! The same semantic event stream that the line renderer turns into
//! CRLF-terminated ANSI is turned here into owned ratatui spans. Semantic
//! tags resolve through the shared [`AnsiTheme`] and colors downgrade to
//! the connection's [`ColorDepth`] exactly as they do on the line side;
//! control characters are stripped everywhere (the same injection defense).
//! Panel data is read defensively: a missing or malformed field degrades to
//! a placeholder rather than panicking (invariant T6).

use std::collections::{HashMap, VecDeque};

use mush_ansi::{AnsiTheme, fmt_duration, merge_styles};
use mush_proto::{
    ClientCaps, Color as MushColor, ColorDepth, NarrativeKind, NoticeSeverity, ObjRef, OutputEvent,
    ProjectedActionReceipt, RichText, RoomView, Span as MushSpan, Style as MushStyle, VitalRole,
    VitalState, VitalValue, VitalsFrame, WhoEntry,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::panels::{ARENA_MAP_FEED, ChanMsg, PanelStore, rich_plain};
use super::replay::{self, Picture};
use super::sheet::{Glyphs, SheetRow, sheet_rows};

/// The speaker gutter's width (plan §7).
pub const GUTTER_W: usize = 10;

/// One scrollback entry: a gutter (the speaker, a clock, a `roll` marker)
/// and a body that wraps with a hanging indent under the gutter.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The gutter cell; `None` for a continuation line of the same event.
    pub gutter: Option<Span<'static>>,
    /// The body spans.
    pub body: Vec<Span<'static>>,
    /// When the entry was applied (Unix seconds).
    pub at: i64,
}

/// One rendered pane row.
#[derive(Clone, Debug)]
pub struct PaneRow {
    /// The row's spans.
    pub line: Line<'static>,
    /// The line `Enter` prefills when this row is highlighted.
    pub prefill: Option<String>,
    /// The key a [`KeyAction::PrefillRow`](super::screen::KeyAction)
    /// appends (a mail index, a verb, a channel).
    pub row_key: Option<String>,
    /// Whether the highlight may rest on this row.
    pub selectable: bool,
}

impl PaneRow {
    pub(super) fn plain(line: Line<'static>) -> PaneRow {
        PaneRow {
            line,
            prefill: None,
            row_key: None,
            selectable: false,
        }
    }

    fn selectable(line: Line<'static>, prefill: String, row_key: String) -> PaneRow {
        PaneRow {
            line,
            prefill: Some(prefill),
            row_key: Some(row_key),
            selectable: true,
        }
    }

    /// The row's plain text, spans joined (16b cut Q.4). What a caller that
    /// wants to READ a pane — a golden, the map parity test — asks for, rather
    /// than walking `line.spans` itself.
    #[must_use]
    pub fn text(&self) -> String {
        self.line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}

/// A pane's rendered rows.
#[derive(Clone, Debug, Default)]
pub struct PaneRows {
    /// The rows, top to bottom.
    pub rows: Vec<PaneRow>,
}

impl PaneRows {
    pub(super) fn one(text: &str) -> PaneRows {
        PaneRows {
            rows: vec![PaneRow::plain(Line::from(dim_span_owned(text)))],
        }
    }

    /// The single dim row a declared placeholder pane shows (interface plan
    /// I8): a pane whose producer does not exist yet says so, rather than
    /// waiting forever on a feed that will never arrive.
    #[must_use]
    pub fn placeholder(text: &str) -> PaneRows {
        let mut rows = PaneRows::default();
        for line in text.lines() {
            rows.rows.extend(PaneRows::one(line).rows);
        }
        rows
    }
}

/// What a pane renderer may read.
#[derive(Clone, Copy)]
pub struct RenderCtx<'a> {
    /// The theme.
    pub theme: &'a AnsiTheme,
    /// The connection's color depth.
    pub depth: ColorDepth,
    /// Whether the terminal shows UTF-8.
    pub utf8: bool,
    /// The store, for renderers that combine feeds.
    pub store: &'a PanelStore,
    /// The channel the backlog pane shows.
    pub selected_channel: Option<&'a str>,
    /// **The pane's inner rect, `(width, height)`** (16b cut Q.4), where the
    /// caller knows it: `super::app::TuiApp::draw_pane` passes the rect it is
    /// about to paint into, and the input paths (scrolling, the highlighted
    /// row) pass `None` because they ask what the rows ARE, not how many fit.
    ///
    /// Three renderers read it: [`render_cockpit_map_rows`], because a map is
    /// a pane whose CONTENT is a function of its size — the window it carves
    /// out of the arena — rather than a list the pane scrolls; since 16b cut
    /// B.3, [`render_cockpit_notices_rows`], which answers the TAIL of its ring
    /// that fits, because the pane is not selectable and the newest line is the
    /// one a pilot is waiting for; and, since 16b cut AB.1,
    /// [`render_cockpit_damage_rows`], which packs a section's facet cells into
    /// rows of the pane's width so no cell is split from its numbers.
    pub pane: Option<(u16, u16)>,
    /// **The replay open on screen 0** (16b cut I.2), where one is:
    /// `super::app::TuiApp` fills it from its own state for every pane it
    /// renders. The cockpit's renderers read it when the held `arena.cockpit`
    /// frame is not seated: with a picture here AND the store holding the
    /// replay document of its match, each draws its pane of the replay
    /// ([`super::replay`]); otherwise its existing empty state.
    pub picture: Option<&'a Picture>,
}

impl RenderCtx<'_> {
    fn tag(&self, tag: &str) -> Style {
        to_ratatui_style(self.theme.style_for(Some(tag)), self.depth)
    }
}

/// Map a downgraded `mush_proto` color to a ratatui color. Palette index
/// `0..=15` stays an [`Color::Indexed`] (the [`super::backend::SshBackend`]
/// re-splits `<16` onto the basic/bright SGR forms).
fn mush_to_ratatui_color(c: MushColor) -> Color {
    match c {
        MushColor::Ansi(n) => Color::Indexed(n),
        MushColor::Idx256(i) => Color::Indexed(i),
        MushColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Convert a `mush_proto` [`Style`](MushStyle) to a ratatui [`Style`],
/// downgrading its colors for `depth`. Colorless connections keep the
/// attribute flags off too, matching `mush_ansi`'s `ColorDepth::None` policy.
#[must_use]
pub fn to_ratatui_style(style: MushStyle, depth: ColorDepth) -> Style {
    if matches!(depth, ColorDepth::None) {
        return Style::default();
    }
    let mut out = Style::default();
    if let Some(c) = style.fg.and_then(|c| mush_ansi::downgrade(c, depth)) {
        out = out.fg(mush_to_ratatui_color(c));
    }
    if let Some(c) = style.bg.and_then(|c| mush_ansi::downgrade(c, depth)) {
        out = out.bg(mush_to_ratatui_color(c));
    }
    if style.bold {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.italic {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.underline {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

/// `HH:MM` (UTC) of a Unix timestamp.
#[must_use]
pub fn hhmm(unix: i64) -> String {
    let s = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// `text` with control characters removed. `pub(crate)` since 16b cut B.3,
/// so the cockpit's notices ring cleans a reply exactly as every renderer here
/// cleans server text.
pub(crate) fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// A dim span with owned text (control chars stripped).
pub(crate) fn dim_span_owned(text: &str) -> Span<'static> {
    Span::styled(clean(text), Style::default().add_modifier(Modifier::DIM))
}

/// The first `max` characters of `s`. `pub(crate)` since 16b cut Y.4, so the
/// notices ring cuts the catch-up's callsign the way the renderers here cut
/// every other server string.
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Accumulates ratatui spans into entries, splitting embedded `\n` and
/// stripping other control characters.
struct EntryAcc<'t> {
    theme: &'t AnsiTheme,
    depth: ColorDepth,
    entries: Vec<Entry>,
    cur: Vec<Span<'static>>,
    /// The gutter for the entry being built; `None` for a continuation
    /// line unless a renderer sets another.
    gutter: Option<Span<'static>>,
    at: i64,
}

impl<'t> EntryAcc<'t> {
    fn new(theme: &'t AnsiTheme, depth: ColorDepth, at: i64) -> EntryAcc<'t> {
        EntryAcc {
            theme,
            depth,
            entries: Vec::new(),
            cur: Vec::new(),
            gutter: None,
            at,
        }
    }

    fn style_for(&self, tag: &str) -> Style {
        to_ratatui_style(self.theme.style_for(Some(tag)), self.depth)
    }

    /// Set the gutter of the entry being built.
    fn gutter(&mut self, span: Span<'static>) {
        self.gutter = Some(span);
    }

    /// A clock gutter (`HH:MM`, dim).
    fn clock_gutter(&mut self) {
        self.gutter = Some(Span::styled(
            hhmm(self.at),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }

    fn write(&mut self, text: &str, style: Style) {
        let mut first = true;
        for piece in text.split('\n') {
            if !first {
                self.end_line();
            }
            first = false;
            let cleaned = clean(piece);
            if cleaned.is_empty() {
                continue;
            }
            self.cur.push(Span::styled(cleaned, style));
        }
    }

    fn plain(&mut self, text: &str) {
        self.write(text, Style::default());
    }

    fn tagged(&mut self, tag: &str, text: &str) {
        let style = self.style_for(tag);
        self.write(text, style);
    }

    fn span(&mut self, span: &MushSpan) {
        let theme_style = self.theme.style_for(span.tag.as_deref());
        let merged = merge_styles(span.style, theme_style);
        self.write(&span.text, to_ratatui_style(merged, self.depth));
    }

    fn rich(&mut self, text: &RichText) {
        for span in text.spans() {
            self.span(span);
        }
    }

    fn end_line(&mut self) {
        let body = std::mem::take(&mut self.cur);
        let gutter = self.gutter.take();
        self.entries.push(Entry {
            gutter,
            body,
            at: self.at,
        });
    }

    fn finish(mut self) -> Vec<Entry> {
        if !self.cur.is_empty() {
            self.end_line();
        }
        self.entries
    }
}

/// Render one event to scrollback entries. Returns empty for variants the
/// TUI routes elsewhere (`Who`, `Prompt`, `Panel`, `Composing`, `View`) or
/// drops (`Ping`, `Resumed`). `now` stamps the clock gutter.
#[must_use]
pub fn render_event(
    event: &OutputEvent,
    caps: &ClientCaps,
    theme: &AnsiTheme,
    now: i64,
) -> Vec<Entry> {
    let mut w = EntryAcc::new(theme, caps.color, now);
    match event {
        OutputEvent::Room(view) => render_room(&mut w, view),
        OutputEvent::Narrative { kind, actor, text } => {
            render_narrative(&mut w, *kind, actor.as_ref(), text, caps.utf8);
        }
        OutputEvent::Sys(text) => {
            w.clock_gutter();
            w.rich(text);
            w.end_line();
        }
        OutputEvent::Channel {
            channel,
            actor,
            title,
            kind,
            text,
            ..
        } => render_channel(&mut w, channel, actor, title.as_deref(), *kind, text),
        OutputEvent::MailNotice {
            from,
            subject,
            id,
            unread,
        } => {
            w.clock_gutter();
            w.tagged(
                "system",
                &format!(
                    "MAIL: New message ({id}) from {}: {subject} ({unread} unread)",
                    from.name
                ),
            );
            w.end_line();
        }
        // A unified notification (I11): the terminal's scrollback is its
        // history, so a notice renders inline like line mode does — `[cat]`
        // coloured by severity, then the action's prefill as a hint. The tab
        // status line carries the unseen count (stored in `apply_event`).
        OutputEvent::Notice {
            severity,
            category,
            text,
            action,
            ..
        } => {
            let tag = match severity {
                NoticeSeverity::Urgent => "error",
                NoticeSeverity::Warn | NoticeSeverity::Info => "system",
            };
            w.clock_gutter();
            w.tagged(tag, &format!("[{category}] "));
            w.rich(text);
            if let Some(a) = action {
                w.plain(&format!("  ({})", a.prefill));
            }
            w.end_line();
        }
        OutputEvent::Weather { text, .. } => {
            w.clock_gutter();
            w.rich(text);
            w.end_line();
        }
        OutputEvent::Reply { event, .. } => return render_event(event, caps, theme, now),
        OutputEvent::Hello { proto, server, .. } => {
            w.clock_gutter();
            w.tagged("system", &format!("{server} (proto {proto})"));
            w.end_line();
        }
        OutputEvent::SessionInfo {
            character,
            account,
            connections,
        } => {
            let s = if *connections == 1 { "" } else { "s" };
            w.clock_gutter();
            w.tagged(
                "system",
                &format!(
                    "Connected as {} (account {account}, {connections} connection{s}).",
                    character.name
                ),
            );
            w.end_line();
        }
        OutputEvent::Disconnect { reason } => {
            w.clock_gutter();
            w.rich(reason);
            w.end_line();
        }
        OutputEvent::View(view) if view.view() == "resolution.outcome" => {
            if let Ok(receipt) =
                serde_json::from_value::<ProjectedActionReceipt>(view.data().clone())
                && receipt.view().is_ok_and(|expected| &expected == view)
            {
                w.clock_gutter();
                w.tagged(
                    "system",
                    receipt
                        .result()
                        .map_or("Action committed.", |result| result.summary()),
                );
                w.end_line();
                w.tagged("system", &format!("Transaction: {}", receipt.status()));
                w.end_line();
            }
        }
        // Routed elsewhere or intentionally silent (mirrors mush_ansi): the
        // WHO rail, the input frame, the panel store, the status strip.
        OutputEvent::Who(_)
        | OutputEvent::InvokeResult { .. }
        | OutputEvent::Panel { .. }
        | OutputEvent::Prompt(_)
        | OutputEvent::Ping { .. }
        | OutputEvent::Resumed { .. }
        | OutputEvent::Composing { .. }
        // A server-initiated request (I10): the modal web client answers it;
        // the TUI overlay and line-mode prompt are I10 part 2, until when a
        // TUI user falls back to the server's default (D-I-21).
        | OutputEvent::Request { .. }
        | OutputEvent::View(_) => {}
    }
    w.finish()
}

/// The room render (mirrors line mode's I3 form): `Name #id`, the
/// description, then the facts in a label gutter — `Here`, `Exits`,
/// `Weather` from `ambient.weather.line`.
fn render_room(w: &mut EntryAcc<'_>, view: &RoomView) {
    w.clock_gutter();
    w.tagged("room", &flat(&view.name));
    w.plain(" ");
    w.tagged("dim", &flat(&view.id));
    w.end_line();
    if !view.desc.to_plain_text().is_empty() {
        w.rich(&view.desc);
        w.end_line();
    }
    if !view.contents.is_empty() {
        w.plain("Here     ");
        for (i, obj) in view.contents.iter().enumerate() {
            if i > 0 {
                w.plain(", ");
            }
            w.plain(&flat(&obj.name));
        }
        w.end_line();
    }
    if !view.exits.is_empty() {
        w.plain("Exits    ");
        for (i, exit) in view.exits.iter().enumerate() {
            if i > 0 {
                w.plain("  ");
            }
            w.tagged("exit", &flat(&exit.name));
        }
        w.end_line();
    }
    if let Some(line) = view
        .ambient
        .get("weather")
        .and_then(|v| v.get("line"))
        .and_then(Value::as_str)
    {
        w.plain("Weather  ");
        w.tagged("weather", &flat(line));
        w.end_line();
    }
}

/// A room field flattened to ONE visual line.
///
/// `EntryAcc::write` splits an embedded newline into a fresh ENTRY, and a
/// scrollback entry with no gutter reads as a continuation of the fact above
/// it — a module- or builder-written field could forge a second `Here` or
/// `Exits` row (look-back I3-C36; the same fix as line mode's `flat`). Room
/// fields are never sanitised on the way in the way player lines are; other
/// control bytes `clean` already strips.
fn flat(s: &str) -> String {
    s.replace('\n', " ")
}

/// Whether `text` is a one-line roll narration (its first span is tagged
/// `roll`, interface plan I3).
fn is_roll_narration(text: &RichText) -> bool {
    text.spans()
        .first()
        .is_some_and(|s| s.tag.as_deref() == Some("roll"))
}

fn render_narrative(
    w: &mut EntryAcc<'_>,
    kind: NarrativeKind,
    actor: Option<&ObjRef>,
    text: &RichText,
    utf8: bool,
) {
    let name = actor.map_or("Someone", |a| a.name.as_str());
    let speaker = |w: &EntryAcc<'_>| {
        Span::styled(
            truncate_chars(&clean(name), GUTTER_W),
            w.style_for("player"),
        )
    };
    match kind {
        NarrativeKind::Say => {
            w.gutter(speaker(w));
            w.plain("\"");
            w.rich(text);
            w.plain("\"");
        }
        NarrativeKind::Pose | NarrativeKind::SemiPose => {
            w.gutter(speaker(w));
            w.rich(text);
        }
        NarrativeKind::Emit | NarrativeKind::Spoof => {
            if is_roll_narration(text) {
                let marker = if utf8 { "\u{2684} roll" } else { "roll" };
                let style = w.style_for("roll");
                w.gutter(Span::styled(marker.to_string(), style));
                // Drop the marker and its gap; the rest is the line.
                for span in text.spans().iter().skip(2) {
                    w.span(span);
                }
                if let Some(first) = w.cur.first_mut()
                    && let Some(rest) = first.content.strip_prefix("  ")
                {
                    first.content = rest.to_string().into();
                }
            } else {
                w.clock_gutter();
                w.rich(text);
            }
        }
        NarrativeKind::Page => {
            w.gutter(speaker(w));
            w.plain("pages: ");
            w.rich(text);
        }
        NarrativeKind::Whisper => {
            w.gutter(speaker(w));
            w.plain("whispers, \"");
            w.rich(text);
            w.plain("\"");
        }
        NarrativeKind::Ooc => {
            if actor.is_some() {
                w.gutter(speaker(w));
                w.plain("<OOC> \"");
                w.rich(text);
                w.plain("\"");
            } else {
                w.clock_gutter();
                w.plain("<OOC> ");
                w.rich(text);
            }
        }
    }
    w.end_line();
}

fn render_channel(
    w: &mut EntryAcc<'_>,
    channel: &str,
    actor: &ObjRef,
    title: Option<&str>,
    kind: NarrativeKind,
    text: &RichText,
) {
    let style = w.style_for("channel");
    w.gutter(Span::styled(
        truncate_chars(&format!("[{}]", clean(channel)), GUTTER_W),
        style,
    ));
    let name_prefix = |w: &mut EntryAcc<'_>| {
        if let Some(t) = title {
            w.plain(t);
            w.plain(" ");
        }
        w.tagged("player", &actor.name);
    };
    match kind {
        NarrativeKind::Pose => {
            name_prefix(w);
            w.plain(" ");
            w.rich(text);
        }
        NarrativeKind::SemiPose => {
            name_prefix(w);
            w.rich(text);
        }
        NarrativeKind::Emit | NarrativeKind::Spoof => w.rich(text),
        _ => {
            name_prefix(w);
            w.plain(": ");
            w.rich(text);
        }
    }
    w.end_line();
}

/// Word-wrap spans to `width` columns: breaks at spaces, hard-splits a word
/// longer than the width (a pane has hard edges), keeps each character's
/// style, drops the space a break consumed. `width == 0` yields one row.
#[must_use]
pub fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    if width == 0 {
        return vec![runs(&chars)];
    }
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= width {
            rows.push(runs(&chars[start..]));
            break;
        }
        // A window that ends exactly at a word boundary is a row; else the
        // last space within the window breaks it, else a hard split.
        let window = &chars[start..start + width];
        if chars[start + width].0 == ' ' {
            rows.push(runs(window));
            start += width + 1;
            continue;
        }
        let brk = window
            .iter()
            .rposition(|(c, _)| *c == ' ')
            .filter(|&i| i > 0);
        match brk {
            Some(i) => {
                rows.push(runs(&chars[start..start + i]));
                start += i + 1;
            }
            None => {
                rows.push(runs(window));
                start += width;
            }
        }
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// Merge consecutive same-style characters into spans.
fn runs(chars: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for &(c, style) in chars {
        match out.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(c),
            _ => out.push(Span::styled(c.to_string(), style)),
        }
    }
    out
}

/// Render a WHO listing into rail lines: a bold header, one row per entry
/// (a `here` marker, the name, idle), then a count footer.
#[must_use]
pub fn render_who_rail(
    entries: &[WhoEntry],
    theme: &AnsiTheme,
    depth: ColorDepth,
    utf8: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(entries.len() + 2);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled(
        format!("  {:<13} {:>4}", "Name", "Idle"),
        bold,
    )));
    let player_style = to_ratatui_style(theme.style_for(Some("player")), depth);
    let here_mark = if utf8 { "\u{2022}" } else { "*" };
    for e in entries {
        let name = truncate_chars(&clean(&e.name), 13);
        let idle = fmt_duration(e.idle_s);
        let mark = if e.here { here_mark } else { " " };
        lines.push(Line::from(vec![
            Span::raw(format!("{mark} ")),
            Span::styled(format!("{name:<13}"), player_style),
            Span::raw(" "),
            Span::raw(format!("{idle:>4}")),
        ]));
    }
    let n = entries.len();
    let here = entries.iter().filter(|e| e.here).count();
    lines.push(Line::from(Span::styled(
        format!("{n} online, {here} here"),
        to_ratatui_style(theme.style_for(Some("system")), depth),
    )));
    lines
}

// ---------------------------------------------------------------------------
// Pane renderers: panel keyframes to rows.
// ---------------------------------------------------------------------------

/// The `mail.list` keyframe: a folder header, then one selectable row per
/// message (`index`, an unread marker, sender, subject) prefilling
/// `@mail <index>`.
#[must_use]
pub fn render_mail_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let system = ctx.tag("system");
    let player = ctx.tag("player");
    let mut rows = Vec::new();
    let folder = data
        .get("folder")
        .and_then(Value::as_str)
        .unwrap_or("inbox");
    let entries = data.get("entries").and_then(Value::as_array);
    let count = entries.map_or(0, Vec::len);
    let unread = entries.map_or(0, |e| {
        e.iter()
            .filter(|m| {
                m.get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "read")
            })
            .count()
    });
    rows.push(PaneRow::plain(Line::from(Span::styled(
        clean(&format!(
            "{folder} \u{2014} {count} message{}, {unread} unread",
            if count == 1 { "" } else { "s" }
        )),
        system,
    ))));
    let Some(entries) = entries else {
        return PaneRows { rows };
    };
    if entries.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned("(empty)"))));
        return PaneRows { rows };
    }
    for e in entries {
        let index = e.get("index").and_then(Value::as_u64).unwrap_or(0);
        let status = e.get("status").and_then(Value::as_str).unwrap_or("read");
        let marker = match status {
            "urgent" => "!",
            "unread" => "*",
            _ => " ",
        };
        let from = e
            .get("from")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("(unknown)");
        let subject = e
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or("(no subject)");
        rows.push(PaneRow::selectable(
            Line::from(vec![
                Span::raw(format!("{index:>3}{marker} ")),
                Span::styled(format!("{:<12} ", truncate_chars(&clean(from), 12)), player),
                Span::raw(truncate_chars(&clean(subject), 60)),
            ]),
            format!("@mail {index}"),
            index.to_string(),
        ));
    }
    PaneRows { rows }
}

/// The `intel.list` keyframe: a count header then, per record (newest
/// first), a subject + kind row and a dim detail row.
#[must_use]
pub fn render_intel_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let system = ctx.tag("system");
    let player = ctx.tag("player");
    let channel = ctx.tag("channel");
    let mut rows = Vec::new();
    let records = data.get("records").and_then(Value::as_array);
    let count = records.map_or(0, Vec::len);
    rows.push(PaneRow::plain(Line::from(Span::styled(
        format!(
            "Intel \u{2014} {count} record{}",
            if count == 1 { "" } else { "s" }
        ),
        system,
    ))));
    let Some(records) = records else {
        return PaneRows { rows };
    };
    if records.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned(
            "(nothing filed)",
        ))));
        return PaneRows { rows };
    }
    for r in records {
        let subject = r
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or("(unknown)");
        let kind = r.get("kind").and_then(Value::as_str).unwrap_or("seen");
        let detail = r.get("detail").and_then(Value::as_str).unwrap_or("");
        rows.push(PaneRow::plain(Line::from(vec![
            Span::styled(truncate_chars(&clean(subject), 16), player),
            Span::raw(" "),
            Span::styled(format!("[{}]", clean(kind)), channel),
        ])));
        if !detail.is_empty() {
            rows.push(PaneRow::plain(Line::from(dim_span_owned(&format!(
                "  {}",
                truncate_chars(detail, 60)
            )))));
        }
    }
    PaneRows { rows }
}

/// The shared viewer-safe legal-action DTO array: one selectable row per
/// offer (`+act <verb> — label`) prefilling `+act <verb>`, then a dim
/// help row. Opaque offer tokens and every unrecognized field stay hidden.
#[must_use]
pub fn render_actions_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let system = ctx.tag("system");
    let player = ctx.tag("player");
    let Some(offers) = data.as_array() else {
        return PaneRows::one("(unavailable)");
    };
    if offers.is_empty() {
        return PaneRows::one("(none available)");
    }
    let mut rows = Vec::new();
    for presentation in offers.iter().filter_map(|offer| offer.get("presentation")) {
        let verb = presentation
            .get("verb")
            .and_then(Value::as_str)
            .unwrap_or("action");
        let label = presentation
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(verb);
        let help = presentation
            .get("help")
            .and_then(Value::as_str)
            .unwrap_or("");
        let verb = clean(verb);
        rows.push(PaneRow::selectable(
            Line::from(vec![
                Span::styled(format!("+act {}", truncate_chars(&verb, 18)), player),
                Span::styled(
                    format!(" \u{2014} {}", truncate_chars(&clean(label), 24)),
                    system,
                ),
            ]),
            format!("+act {verb}"),
            verb.clone(),
        ));
        if !help.is_empty() {
            rows.push(PaneRow::plain(Line::from(dim_span_owned(&format!(
                "  {}",
                truncate_chars(help, 60)
            )))));
        }
    }
    PaneRows { rows }
}

/// The `scenes` keyframe: the scene's title, kind and status, its round
/// and turn, and who acts now.
#[must_use]
/// The `atlas.map` pane (doc 14 §6, S4): the window's grid north-up with the
/// units overlaid — the same lines line mode prints, the whole grid so the
/// pane scrolls, the title bold. A frame that is not a grid is nothing yet.
pub fn render_map_rows(data: &Value, _ctx: &RenderCtx<'_>) -> PaneRows {
    if data.is_null() || !mush_ansi::is_grid(data) {
        return PaneRows::one("(no map: a scene's facilitator opens one with +map/open)");
    }
    let lines = mush_ansi::grid_lines(data, mush_proto::PanelFrameKind::Keyframe, 512, 512);
    let mut rows = Vec::with_capacity(lines.len());
    for (i, line) in lines.into_iter().enumerate() {
        let style = if i == 0 {
            Style::default().add_modifier(Modifier::BOLD)
        } else if i == 1 {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default()
        };
        rows.push(PaneRow::plain(Line::from(Span::styled(
            clean(&line),
            style,
        ))));
    }
    PaneRows { rows }
}

pub fn render_scene_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if data.is_null() {
        return PaneRows::one("(no scene)");
    }
    let system = ctx.tag("system");
    let player = ctx.tag("player");
    let s = |k: &str| {
        data.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let n = |k: &str| data.get(k).and_then(Value::as_u64).unwrap_or(0);
    let title = clean(&s("title"));
    let mut rows = vec![PaneRow::plain(Line::from(Span::styled(
        if title.is_empty() {
            "(untitled)".to_string()
        } else {
            title
        },
        Style::default().add_modifier(Modifier::BOLD),
    )))];
    rows.push(PaneRow::plain(Line::from(Span::styled(
        clean(&format!("{} \u{00b7} {}", s("kind"), s("status"))),
        system,
    ))));
    rows.push(PaneRow::plain(Line::from(Span::raw(format!(
        "round {}  turn {}",
        n("round"),
        n("turn")
    )))));
    if let Some(actor) = data
        .get("current_actor")
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
    {
        rows.push(PaneRow::plain(Line::from(vec![
            Span::raw("now: "),
            Span::styled(clean(actor), player),
        ])));
    }
    PaneRows { rows }
}

/// The `comsys.state` keyframe: one selectable row per channel (alias,
/// name, unread badge) prefilling `<alias> `; the row key is the channel
/// name the backlog pane and query use.
#[must_use]
pub fn render_channels_rows(data: &Value, ctx: &RenderCtx<'_>, ic: bool) -> PaneRows {
    let channel_style = ctx.tag("channel");
    let empty = if ic {
        "(no IC channels)"
    } else {
        "(no OOC channels)"
    };
    let Some(channels) = data.get("channels").and_then(Value::as_array) else {
        return PaneRows::one(empty);
    };
    let mut rows = Vec::new();
    for c in channels {
        if c.get("ic").and_then(Value::as_bool).unwrap_or(false) != ic {
            continue;
        }
        let name = clean(c.get("name").and_then(Value::as_str).unwrap_or("?"));
        let alias = clean(c.get("alias").and_then(Value::as_str).unwrap_or(""));
        let unread = c.get("unread").and_then(Value::as_u64).unwrap_or(0);
        let gagged = c.get("gagged").and_then(Value::as_bool).unwrap_or(false);
        let badge = if unread > 0 {
            format!(" {unread}")
        } else {
            String::new()
        };
        let gag = if gagged { " (gag)" } else { "" };
        let prefill = if alias.is_empty() {
            format!("{name} ")
        } else {
            format!("{alias} ")
        };
        rows.push(PaneRow::selectable(
            Line::from(vec![
                Span::styled(format!("{:<6} ", truncate_chars(&alias, 6)), channel_style),
                Span::raw(truncate_chars(&name, 14)),
                Span::styled(badge, Style::default().add_modifier(Modifier::BOLD)),
                dim_span_owned(gag),
            ]),
            prefill,
            name,
        ));
    }
    if rows.is_empty() {
        return PaneRows::one(empty);
    }
    PaneRows { rows }
}

/// The backlog pane: the highlighted channel's kept messages, oldest first.
#[must_use]
pub fn render_backlog_rows(msgs: &[&ChanMsg], channel: Option<&str>) -> PaneRows {
    let Some(channel) = channel else {
        return PaneRows::one("(pick a channel)");
    };
    if msgs.is_empty() {
        return PaneRows::one(&format!("(nothing kept for {channel}; b requests backlog)"));
    }
    PaneRows {
        rows: msgs
            .iter()
            .map(|m| PaneRow::plain(Line::from(Span::raw(clean(&m.line)))))
            .collect(),
    }
}

/// The rolls pane: every `.roll` frame's `summary`, oldest first.
#[must_use]
pub fn render_rolls_rows(rolls: &[Value]) -> PaneRows {
    if rolls.is_empty() {
        return PaneRows::one("(no rolls yet)");
    }
    PaneRows {
        rows: rolls
            .iter()
            .map(|r| {
                let summary = r
                    .get("summary")
                    .and_then(Value::as_str)
                    .map_or_else(|| rich_plain(r), clean);
                PaneRow::plain(Line::from(Span::raw(summary)))
            })
            .collect(),
    }
}

/// The sheet pane: the `.sheet` keyframe walked into label/value rows.
#[must_use]
pub fn render_sheet_rows(payload: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let Some(rows) = sheet_rows(payload, Glyphs::for_utf8(ctx.utf8)) else {
        return PaneRows::one("(no sheet)");
    };
    let header = Style::default().add_modifier(Modifier::BOLD);
    let label_style = ctx.tag("system");
    PaneRows {
        rows: rows
            .into_iter()
            .map(|r| match r {
                SheetRow::Header(h) => PaneRow::plain(Line::from(Span::styled(clean(&h), header))),
                SheetRow::Field { label, value } => PaneRow::plain(Line::from(vec![
                    Span::styled(
                        format!("{:<16} ", truncate_chars(&clean(&label), 16)),
                        label_style,
                    ),
                    Span::raw(clean(&value)),
                ])),
                SheetRow::Blank => PaneRow::plain(Line::from("")),
            })
            .collect(),
    }
}

/// The widest a generic-view list column is padded to (interface plan I9).
const VIEW_COL_MAX_W: usize = 24;
/// The most columns a generic-view list renders.
const VIEW_MAX_COLS: usize = 12;
/// The most rows a generic-view list renders, matching the web's
/// `MAX_VIEW_ROWS` (look-back LB-17: the two sides had no shared number and
/// this side had no row cap at all).
const VIEW_MAX_ROWS: usize = 256;
/// The most fields a generic-view detail or form renders, matching the web's
/// `MAX_VIEW_FIELDS`.
const VIEW_MAX_FIELDS: usize = 32;

/// The generic semantic-view pane (interface plan I9): a `{ schema, rows,
/// record }` payload walked into rows by its declared `shape` — a list with a
/// header and one row per record (a row is selectable, prefilling the line
/// named by the schema's `actionKey`, D-I-18), or a detail/form as
/// label/value rows. The same convention the web's `genericview.ts` walks;
/// nothing here names a view or a field, and every value is read defensively.
#[must_use]
pub fn render_view_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let Some(schema) = data.get("schema") else {
        return PaneRows::one("(no view)");
    };
    let shape = schema.get("shape").and_then(Value::as_str).unwrap_or("");
    let label_style = ctx.tag("system");
    match shape {
        "list" => {
            let cols = view_columns(schema);
            if cols.is_empty() {
                return PaneRows::one("(empty view)");
            }
            let action_key = schema.get("actionKey").and_then(Value::as_str);
            let empty = Vec::new();
            let all_rows = data.get("rows").and_then(Value::as_array).unwrap_or(&empty);
            let cut = all_rows.len() > VIEW_MAX_ROWS;
            let rows_data = &all_rows[..all_rows.len().min(VIEW_MAX_ROWS)];
            let mut widths: Vec<usize> = cols.iter().map(|c| c.label.chars().count()).collect();
            for r in rows_data {
                for (i, c) in cols.iter().enumerate() {
                    widths[i] = widths[i].max(view_cell_text(r, &c.key).chars().count());
                }
            }
            for w in &mut widths {
                *w = (*w).min(VIEW_COL_MAX_W);
            }
            let mut rows = Vec::new();
            let header = cols
                .iter()
                .enumerate()
                .map(|(i, c)| view_pad(&c.label, widths[i], c.right))
                .collect::<Vec<_>>()
                .join("  ");
            rows.push(PaneRow::plain(Line::from(Span::styled(
                header,
                label_style,
            ))));
            if rows_data.is_empty() {
                rows.push(PaneRow::plain(Line::from(dim_span_owned("(none)"))));
            }
            for r in rows_data {
                let cells = cols
                    .iter()
                    .enumerate()
                    .map(|(i, c)| view_pad(&view_cell_text(r, &c.key), widths[i], c.right))
                    .collect::<Vec<_>>()
                    .join("  ");
                let line = Line::from(Span::raw(cells));
                let prefill = action_key
                    .and_then(|ak| r.get(ak).and_then(Value::as_str))
                    .filter(|s| !s.is_empty());
                match prefill {
                    Some(p) => rows.push(PaneRow::selectable(line, p.to_string(), p.to_string())),
                    None => rows.push(PaneRow::plain(line)),
                }
            }
            // A view that says its walk was cut says so here, so every
            // generic list inherits the honesty the unit panes have (I8: the
            // contacts table is the first producer to reach the row cap) —
            // and so does a list this renderer cut itself at
            // [`VIEW_MAX_ROWS`] (look-back LB-17).
            if cut || data.get("truncated").and_then(Value::as_bool) == Some(true) {
                rows.push(PaneRow::plain(Line::from(dim_span_owned(
                    "(more rows than shown)",
                ))));
            }
            PaneRows { rows }
        }
        "detail" | "form" => {
            let Some(fields) = schema.get("fields").and_then(Value::as_array) else {
                return PaneRows::one("(empty view)");
            };
            if fields.is_empty() {
                return PaneRows::one("(empty view)");
            }
            let record = data.get("record");
            // Look-back I9-C6: a `record` that is PRESENT but not an object
            // makes every field render blank, which reads exactly like a
            // record that is there and empty. Say which it is. A `null`
            // record is the ordinary "nothing selected" and keeps the blank
            // field rows, which is what the schema promised.
            if record.is_some_and(|r| !r.is_null() && !r.is_object()) {
                return PaneRows::one("(unreadable record)");
            }
            let mut rows = Vec::new();
            for f in fields.iter().take(VIEW_MAX_FIELDS) {
                let Some(key) = f.get("key").and_then(Value::as_str) else {
                    continue;
                };
                let label = f.get("label").and_then(Value::as_str).unwrap_or(key);
                let value = record
                    .and_then(|r| r.get(key))
                    .map(view_scalar_text)
                    .unwrap_or_default();
                rows.push(PaneRow::plain(Line::from(vec![
                    Span::styled(format!("{}: ", clean(label)), label_style),
                    Span::raw(value),
                ])));
            }
            if fields.len() > VIEW_MAX_FIELDS {
                rows.push(PaneRow::plain(Line::from(dim_span_owned(
                    "(more fields than shown)",
                ))));
            }
            PaneRows { rows }
        }
        _ => PaneRows::one("(unknown view shape)"),
    }
}

/// One resolved list column.
struct ViewCol {
    key: String,
    label: String,
    right: bool,
}

fn view_columns(schema: &Value) -> Vec<ViewCol> {
    schema
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let key = c.get("key").and_then(Value::as_str)?;
                    let label = c.get("label").and_then(Value::as_str).unwrap_or(key);
                    let right = c.get("align").and_then(Value::as_str) == Some("right");
                    Some(ViewCol {
                        key: key.to_string(),
                        label: label.to_string(),
                        right,
                    })
                })
                .take(VIEW_MAX_COLS)
                .collect()
        })
        .unwrap_or_default()
}

/// A cell's display text: a string as-is, a number or bool stringified,
/// anything else (or absent) empty. Control chars stripped.
fn view_cell_text(row: &Value, key: &str) -> String {
    row.get(key).map(view_scalar_text).unwrap_or_default()
}

fn view_scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => clean(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Pad (or truncate) `s` to `w` display columns, right-aligned when asked.
fn view_pad(s: &str, w: usize, right: bool) -> String {
    let s = truncate_chars(s, w);
    let gap = w.saturating_sub(s.chars().count());
    let fill = " ".repeat(gap);
    if right {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    }
}

/// The vitals pane (interface plan I6): every `.vitals` frame as widget
/// rows — a subject header, then each vital as `<mark> <label> <widget>`:
/// a track as boxes grouped in threes with the deepest level and the
/// frame's `detail` (a wound penalty, a "next box" warning), a pool as a
/// `▰▱` gauge with digits, a readout as text. The mark is the condition
/// map: `✓` fine, `◐` impaired, `✗` critical or down (`+ ~ ! X` in ASCII).
/// The frame is the whole contract: nothing here names a ruleset, and every
/// number is clamped.
#[must_use]
pub fn render_vitals_rows(frames: &[(mush_proto::SmolStr, VitalsFrame)], utf8: bool) -> PaneRows {
    if frames.is_empty() {
        return PaneRows::one("(no vitals feed)");
    }
    let mut rows = Vec::new();
    for (i, (_, frame)) in frames.iter().enumerate() {
        if i > 0 {
            rows.push(PaneRow::plain(Line::from("")));
        }
        rows.push(PaneRow::plain(Line::from(Span::styled(
            format!("{}  rev {}", clean(&frame.subject.name), frame.revision),
            Style::default().add_modifier(Modifier::BOLD),
        ))));
        let mut vitals: Vec<&mush_proto::Vital> = frame.vitals.iter().collect();
        vitals.sort_by_key(|v| match v.role {
            VitalRole::Dominant => 0,
            VitalRole::Track => 1,
            VitalRole::Pool => 2,
            VitalRole::Readout => 3,
        });
        for v in vitals {
            let (mark, style) = match v.state {
                VitalState::Ok => (if utf8 { '\u{2713}' } else { '+' }, Style::default()),
                VitalState::Warn => (
                    if utf8 { '\u{25d0}' } else { '~' },
                    Style::default().fg(Color::Indexed(3)),
                ),
                VitalState::Crit => (
                    if utf8 { '\u{2717}' } else { '!' },
                    Style::default().fg(Color::Indexed(1)),
                ),
                VitalState::Down => (
                    if utf8 { '\u{2717}' } else { 'X' },
                    Style::default()
                        .fg(Color::Indexed(1))
                        .add_modifier(Modifier::BOLD),
                ),
            };
            let widget = vital_widget(v, utf8);
            let detail = v
                .detail
                .as_deref()
                .map(|d| format!("  {}", clean(d)))
                .unwrap_or_default();
            rows.push(PaneRow::plain(Line::from(vec![
                Span::styled(format!("{mark} "), style),
                dim_span_owned(&format!("{:<12}", truncate_chars(&clean(&v.label), 12))),
                Span::styled(widget, style),
                dim_span_owned(&detail),
            ])));
        }
    }
    PaneRows { rows }
}

/// Gauge cells the vitals pane and strip use.
const VITAL_GAUGE_W: usize = 8;

/// A `▰▱` gauge `cells` wide for `current` within `min..=max` (`#-` without
/// UTF-8), rounded to the nearest cell. Reversed bounds AND extreme
/// magnitudes are tolerated — the arithmetic is done in `i128`, so a frame
/// declaring `i64::MIN..=i64::MAX` draws a bar instead of overflowing the
/// render path — and `current` is clamped, so a malformed frame draws an
/// empty or full bar rather than panicking (invariant T6).
fn gauge_bar(current: i64, min: i64, max: i64, cells: usize, utf8: bool) -> String {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    let span = i128::from(hi) - i128::from(lo);
    let rel = i128::from(current.clamp(lo, hi)) - i128::from(lo);
    let width = i128::try_from(cells).unwrap_or(0);
    let filled = if span <= 0 {
        0
    } else {
        usize::try_from((rel * width + span / 2) / span).unwrap_or(0)
    };
    let (on, off) = if utf8 {
        ('\u{25b0}', '\u{25b1}')
    } else {
        ('#', '-')
    };
    (0..cells)
        .map(|i| if i < filled { on } else { off })
        .collect()
}

/// One vital's widget text: gauge, grouped boxes with the level, or text.
fn vital_widget(v: &mush_proto::Vital, utf8: bool) -> String {
    match &v.value {
        VitalValue::Bounded { current, min, max } => {
            let hi = (*min).max(*max);
            let bar = gauge_bar(*current, *min, *max, VITAL_GAUGE_W, utf8);
            format!("{bar} {current}/{hi}")
        }
        VitalValue::Track { boxes, level } => {
            let empty = if utf8 { '\u{25a1}' } else { '_' };
            let mut s = String::new();
            for (i, b) in boxes.iter().enumerate() {
                if i % 3 == 0 {
                    if i > 0 {
                        s.push(']');
                    }
                    s.push('[');
                }
                s.push(match b.as_deref() {
                    None => empty,
                    Some("bashing") => '/',
                    Some("lethal") => 'X',
                    Some("aggravated") => '*',
                    Some(other) => other.chars().next().map_or('?', |c| c.to_ascii_uppercase()),
                });
            }
            if !boxes.is_empty() {
                s.push(']');
            }
            if let Some(l) = level {
                s.push_str("  ");
                s.push_str(&clean(l));
            }
            s
        }
        VitalValue::Text { text } => clean(text),
    }
}

// ---------------------------------------------------------------------------
// The tactical unit panes (interface plan I8).
//
// Nothing here names a ruleset, a unit kind, a section, a component or a
// trait: every label, id and word is read out of the frame. Colour and glyph
// come ONLY from a `state` word (`ok | warn | crit | down`); the ruleset's
// own `condition` word is printed verbatim and never mapped, which is rule 1
// — rulesets judge, clients render. A `null` frame is the honest no-unit
// state, an unknown `state` word is drawn unjudged, an unknown journal
// `kind` renders as an opaque row, and every missing or malformed field
// degrades rather than panicking (invariant T6).
// ---------------------------------------------------------------------------

/// What a unit pane shows when the held frame is `null`: the viewer holds no
/// station and controls no unit. Defined once; both unit screens use it.
pub const NO_UNIT_LINE: &str =
    "No tactical unit: take a station, or @unit/show #n if you control one.";

/// Gauge cells a unit pane's bar uses. Narrower than the vitals gauge: these
/// stack several to a row in a 24-column rail.
const UNIT_GAUGE_W: usize = 5;

/// The glyph and style a `state` word earns — the only word-to-colour map in
/// the unit panes. Only the four states the contract fixes are judged;
/// anything else (a newer server's word) is drawn dim and unjudged.
fn state_mark(state: Option<&str>, utf8: bool) -> (char, Style) {
    match state {
        Some("ok") => (if utf8 { '\u{2713}' } else { '+' }, Style::default()),
        Some("warn") => (
            if utf8 { '\u{25d0}' } else { '~' },
            Style::default().fg(Color::Indexed(3)),
        ),
        Some("crit") => (
            if utf8 { '\u{2717}' } else { '!' },
            Style::default().fg(Color::Indexed(1)),
        ),
        Some("down") => (
            'X',
            Style::default()
                .fg(Color::Indexed(1))
                .add_modifier(Modifier::BOLD),
        ),
        _ => (
            if utf8 { '\u{00b7}' } else { '.' },
            Style::default().add_modifier(Modifier::DIM),
        ),
    }
}

fn unit_sep(utf8: bool) -> &'static str {
    if utf8 { " \u{00b7} " } else { " | " }
}

fn unit_arrow(utf8: bool) -> &'static str {
    if utf8 { "\u{2192}" } else { "->" }
}

fn unit_bullet(utf8: bool) -> char {
    if utf8 { '\u{00b7}' } else { '-' }
}

/// One cell of a unit frame, read defensively.
struct UnitCell<'a> {
    addr: &'a str,
    label: &'a str,
    current: i64,
    min: i64,
    max: i64,
    state: Option<&'a str>,
}

/// Every cell of a frame that carries an `addr`, in the frame's own order.
fn unit_cells(data: &Value) -> Vec<UnitCell<'_>> {
    data.get("cells")
        .and_then(Value::as_array)
        .map(|cells| {
            cells
                .iter()
                .filter_map(|c| {
                    Some(UnitCell {
                        addr: c.get("addr").and_then(Value::as_str)?,
                        label: c.get("label").and_then(Value::as_str).unwrap_or(""),
                        current: c.get("current").and_then(Value::as_i64).unwrap_or(0),
                        min: c.get("min").and_then(Value::as_i64).unwrap_or(0),
                        max: c.get("max").and_then(Value::as_i64).unwrap_or(0),
                        state: c.get("state").and_then(Value::as_str),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The held unit's `(id, name)`, or `None` when the frame is the no-unit one
/// (`null`) or carries no subject at all.
fn unit_object(data: &Value) -> Option<(&str, &str)> {
    let object = data.get("object")?;
    let id = object.get("id").and_then(Value::as_str)?;
    let name = object.get("name").and_then(Value::as_str).unwrap_or(id);
    Some((id, name))
}

/// The section an `addr` is scoped to and the label to print for it:
/// `section/<s>/<cell>` → `(s, "<cell>")`, `facet/<s>/<f>/<cell>` →
/// `(s, "<f>/<cell>")`. Any other scope belongs to no section.
fn cell_scope(addr: &str) -> Option<(&str, String)> {
    let mut parts = addr.split('/');
    match parts.next()? {
        "section" => {
            let section = parts.next()?;
            let cell = parts.next()?;
            parts.next().is_none().then(|| (section, cell.to_string()))
        }
        "facet" => {
            let section = parts.next()?;
            let facet = parts.next()?;
            let cell = parts.next()?;
            parts
                .next()
                .is_none()
                .then(|| (section, format!("{facet}/{cell}")))
        }
        _ => None,
    }
}

/// The group a component installs into, from its own `install` path.
fn component_scope(component: &Value) -> &str {
    install_scope(
        component
            .get("install")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
}

/// The group an `install` path belongs to: `unit` stays `unit`,
/// `section/<s>` and `slot/<s>/<slot>` group under `<s>`.
fn install_scope(install: &str) -> &str {
    let mut parts = install.split('/');
    match parts.next() {
        Some("section" | "slot") => parts.next().unwrap_or("unit"),
        _ => "unit",
    }
}

/// One cell as `label ▰▰▰▱▱ cur/max`.
fn cell_gauge(label: &str, cell: &UnitCell<'_>, utf8: bool) -> String {
    let hi = cell.min.max(cell.max);
    format!(
        "{} {} {}/{}",
        truncate_chars(&clean(label), 14),
        gauge_bar(cell.current, cell.min, cell.max, UNIT_GAUGE_W, utf8),
        cell.current,
        hi
    )
}

/// The label a cell prints: its own, else the last segment of its address.
fn cell_label<'a>(cell: &'a UnitCell<'a>) -> &'a str {
    if cell.label.is_empty() {
        cell.addr.rsplit('/').next().unwrap_or(cell.addr)
    } else {
        cell.label
    }
}

/// The subject header the two `sections` panes carry: name, id, revision,
/// and — when the frame is a held record rather than a live unit — the
/// server's own words for that, printed verbatim.
fn unit_header(data: &Value) -> Option<Vec<PaneRow>> {
    let (id, name) = unit_object(data)?;
    let mut head = format!("{} {}", truncate_chars(&clean(name), 24), clean(id));
    if let Some(revision) = data.get("revision").and_then(Value::as_u64) {
        head.push_str(&format!("  rev {revision}"));
    }
    if let Some(state) = data.get("state").and_then(Value::as_str)
        && state != "active"
    {
        head.push_str(&format!("  {}", truncate_chars(&clean(state), 16)));
    }
    if let Some(record) = data.get("record").and_then(Value::as_str) {
        head.push_str(&format!("  held {}", truncate_chars(&clean(record), 24)));
    }
    Some(vec![PaneRow::plain(Line::from(Span::styled(
        head,
        Style::default().add_modifier(Modifier::BOLD),
    )))])
}

/// The footer a byte-budgeted view earns: what its walk left out, from the
/// frame's own true `counts`.
fn push_truncation(rows: &mut Vec<PaneRow>, data: &Value) {
    if data.get("truncated").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let total = |key: &str| {
        data.get("counts")
            .and_then(|c| c.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let shown = |key: &str| {
        data.get(key)
            .and_then(Value::as_array)
            .map_or(0, |a| u64::try_from(a.len()).unwrap_or(0))
    };
    rows.push(PaneRow::plain(Line::from(dim_span_owned(&format!(
        "(view truncated: {} cells, {} components, {} capabilities not shown)",
        total("cells").saturating_sub(shown("cells")),
        total("components").saturating_sub(shown("components")),
        total("capabilities").saturating_sub(shown("capabilities")),
    )))));
}

/// The damage screen's `sections` pane: the subject, then one block per
/// section — its id, its `condition` word, a glyph from its `state` — and
/// under it a gauge for every cell scoped to that section, each coloured by
/// the CELL's own state.
#[must_use]
pub fn render_unit_sections_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let Some(mut rows) = unit_header(data) else {
        return PaneRows::one(NO_UNIT_LINE);
    };
    let cells = unit_cells(data);
    let empty = Vec::new();
    let sections = data
        .get("sections")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if sections.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned("(no sections)"))));
    }
    for section in sections {
        let Some(id) = section.get("id").and_then(Value::as_str) else {
            continue;
        };
        rows.push(section_head(id, section, ctx.utf8));
        let mut any = false;
        for cell in &cells {
            let Some((scope, label)) = cell_scope(cell.addr) else {
                continue;
            };
            if scope != id {
                continue;
            }
            any = true;
            let (_, style) = state_mark(cell.state, ctx.utf8);
            rows.push(PaneRow::plain(Line::from(vec![
                Span::raw("    "),
                Span::styled(cell_gauge(&label, cell, ctx.utf8), style),
            ])));
        }
        if !any {
            rows.push(PaneRow::plain(Line::from(dim_span_owned("    (no cells)"))));
        }
    }
    push_truncation(&mut rows, data);
    PaneRows { rows }
}

/// The unit screen's `sections` pane: the same map, compact — glyph, id,
/// `condition` word and the section's FIRST cell on one line, every further
/// cell on a four-column continuation row.
///
/// The rows are narrow by construction: the head is at most 2 + 14 + 12 + 2 +
/// a gauge, a continuation at most 4 + a gauge. This was once the only way a
/// cell survived, when `draw_pane` clipped at the pane's edge; since 16b cut
/// B.3 it wraps every panel row, so a long head would fold rather than be
/// lost, and the one-cell-per-row shape stays because a column of gauges
/// reads better than a gauge broken across two lines.
#[must_use]
pub fn render_unit_sections_compact_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let Some(mut rows) = unit_header(data) else {
        return PaneRows::one(NO_UNIT_LINE);
    };
    let cells = unit_cells(data);
    let empty = Vec::new();
    let sections = data
        .get("sections")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if sections.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned("(no sections)"))));
    }
    for section in sections {
        let Some(id) = section.get("id").and_then(Value::as_str) else {
            continue;
        };
        let (mark, style) = state_mark(section.get("state").and_then(Value::as_str), ctx.utf8);
        let condition = section
            .get("condition")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut head = vec![
            Span::styled(format!("{mark} "), style),
            Span::raw(format!("{:<14}", truncate_chars(&clean(id), 14))),
            dim_span_owned(&truncate_chars(&clean(condition), 12)),
        ];
        let mut more: Vec<PaneRow> = Vec::new();
        for cell in &cells {
            let Some((scope, label)) = cell_scope(cell.addr) else {
                continue;
            };
            if scope != id {
                continue;
            }
            let (_, cell_style) = state_mark(cell.state, ctx.utf8);
            let gauge = cell_gauge(&label, cell, ctx.utf8);
            if head.len() == 3 {
                // The first cell shares the section's line; the rest hang
                // under it, so no cell is ever pushed off the right edge.
                head.push(Span::styled(format!("  {gauge}"), cell_style));
            } else {
                more.push(PaneRow::plain(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(gauge, cell_style),
                ])));
            }
        }
        rows.push(PaneRow::plain(Line::from(head)));
        rows.extend(more);
    }
    push_truncation(&mut rows, data);
    PaneRows { rows }
}

fn section_head(id: &str, section: &Value, utf8: bool) -> PaneRow {
    let (mark, style) = state_mark(section.get("state").and_then(Value::as_str), utf8);
    let condition = section
        .get("condition")
        .and_then(Value::as_str)
        .unwrap_or("");
    PaneRow::plain(Line::from(vec![
        Span::styled(format!("{mark} "), style),
        Span::raw(truncate_chars(&clean(id), 24)),
        dim_span_owned(&format!("  {}", truncate_chars(&clean(condition), 16))),
    ]))
}

/// The unit screen's `plant` pane: every unit-level cell as a gauge, then one
/// mobility line read from the frame's safe mobility subset.
#[must_use]
pub fn render_unit_plant_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if unit_object(data).is_none() {
        return PaneRows::one(NO_UNIT_LINE);
    }
    let cells = unit_cells(data);
    let mut rows = Vec::new();
    for cell in cells.iter().filter(|c| c.addr.starts_with("unit/")) {
        let (mark, style) = state_mark(cell.state, ctx.utf8);
        rows.push(PaneRow::plain(Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(cell_gauge(cell_label(cell), cell, ctx.utf8), style),
        ])));
    }
    if rows.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned(
            "(no unit-level cells)",
        ))));
    }
    rows.push(PaneRow::plain(Line::from(Span::raw(mobility_line(
        data, ctx.utf8,
    )))));
    PaneRows { rows }
}

/// `mobility <mode>`, `mobility <from> → <to> (ready <at>)` while a switch is
/// in flight, or `mobility none` when the unit declares no profile.
fn mobility_line(data: &Value, utf8: bool) -> String {
    let none = "mobility none".to_string();
    let Some(mobility) = data.get("mobility").filter(|m| !m.is_null()) else {
        return none;
    };
    let arrow = unit_arrow(utf8);
    if let Some(switching) = mobility.get("switching").filter(|s| !s.is_null()) {
        let from = switching.get("from").and_then(Value::as_str).unwrap_or("?");
        let to = switching.get("to").and_then(Value::as_str).unwrap_or("?");
        let ready = switching
            .get("ready_at")
            .map(view_scalar_text)
            .unwrap_or_default();
        return clean(&format!(
            "mobility {} {arrow} {} (ready {ready})",
            truncate_chars(from, 20),
            truncate_chars(to, 20)
        ));
    }
    match mobility.get("mode").and_then(Value::as_str) {
        Some(mode) => clean(&format!("mobility {}", truncate_chars(mode, 24))),
        None => none,
    }
}

/// The unit screen's `components` pane: components grouped by the section
/// they install into (the unit's own group first, which also carries the
/// capabilities no component sources), each row a glyph from the component's
/// `state`, its `condition` word verbatim, and `·asset` when it is one; the
/// capabilities a component sources are indented under it, and any whose
/// source is not in this frame at all follow in a trailing `(other)` group.
#[must_use]
pub fn render_unit_components_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if unit_object(data).is_none() {
        return PaneRows::one(NO_UNIT_LINE);
    }
    let bullet = unit_bullet(ctx.utf8);
    let system = ctx.tag("system");
    let cells = unit_cells(data);
    let empty = Vec::new();
    let components = data
        .get("components")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let capabilities = data
        .get("capabilities")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut groups: Vec<&str> = vec!["unit"];
    for c in components {
        let scope = component_scope(c);
        if !groups.contains(&scope) {
            groups.push(scope);
        }
    }
    let mut rows = Vec::new();
    for group in groups {
        let mine: Vec<&Value> = components
            .iter()
            .filter(|c| component_scope(c) == group)
            .collect();
        let unsourced: Vec<&Value> = if group == "unit" {
            capabilities
                .iter()
                .filter(|k| k.get("source").is_none_or(Value::is_null))
                .collect()
        } else {
            Vec::new()
        };
        if mine.is_empty() && unsourced.is_empty() {
            continue;
        }
        rows.push(PaneRow::plain(Line::from(Span::styled(
            truncate_chars(&clean(group), 24),
            system,
        ))));
        for capability in unsourced {
            rows.push(capability_row(capability, bullet, 2));
        }
        for component in mine {
            let id = component.get("id").and_then(Value::as_str).unwrap_or("?");
            let (mark, style) =
                state_mark(component.get("state").and_then(Value::as_str), ctx.utf8);
            let condition = component
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or("");
            let asset = if component.get("asset").and_then(Value::as_bool) == Some(true) {
                format!("  {bullet}asset")
            } else {
                String::new()
            };
            rows.push(PaneRow::plain(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{mark} "), style),
                Span::raw(format!("{:<20}", truncate_chars(&clean(id), 20))),
                dim_span_owned(&format!("{}{asset}", truncate_chars(&clean(condition), 14))),
            ])));
            // The component's own cells, so no scope of the frame renders
            // nowhere: `unit/*` is the plant pane, `section`/`facet` the
            // sections pane, and `component/*` is here.
            let prefix = format!("component/{id}/");
            for cell in cells.iter().filter(|c| c.addr.starts_with(&prefix)) {
                let (_, style) = state_mark(cell.state, ctx.utf8);
                rows.push(PaneRow::plain(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(cell_gauge(cell_label(cell), cell, ctx.utf8), style),
                ])));
            }
            for capability in capabilities
                .iter()
                .filter(|k| k.get("source").and_then(Value::as_str) == Some(id))
            {
                rows.push(capability_row(capability, bullet, 4));
            }
        }
    }
    // A capability whose source names no component IN THIS FRAME — the walk
    // admits components before capabilities, so a byte-budgeted frame can
    // carry one — would otherwise render nowhere at all. It gets a trailing
    // group of its own rather than being silently dropped or hung off the
    // unit, which would claim it is intrinsic.
    let orphans: Vec<&Value> = capabilities
        .iter()
        .filter(|k| {
            k.get("source").and_then(Value::as_str).is_some_and(|s| {
                !components
                    .iter()
                    .any(|c| c.get("id").and_then(Value::as_str) == Some(s))
            })
        })
        .collect();
    if !orphans.is_empty() {
        rows.push(PaneRow::plain(Line::from(Span::styled("(other)", system))));
        for capability in orphans {
            rows.push(capability_row(capability, bullet, 2));
        }
    }
    if rows.is_empty() {
        rows.push(PaneRow::plain(Line::from(dim_span_owned(
            "(no components)",
        ))));
    }
    push_truncation(&mut rows, data);
    PaneRows { rows }
}

fn capability_row(capability: &Value, bullet: char, indent: usize) -> PaneRow {
    let id = capability.get("id").and_then(Value::as_str).unwrap_or("?");
    PaneRow::plain(Line::from(dim_span_owned(&format!(
        "{}{bullet} {}",
        " ".repeat(indent),
        truncate_chars(&clean(id), 28)
    ))))
}

/// `… N more entries` — a delta the ring had to cut, saying so.
fn elided_text(entry: &Value, utf8: bool) -> String {
    let omitted = entry.get("omitted").and_then(Value::as_u64).unwrap_or(0);
    let ellipsis = if utf8 { "\u{2026}" } else { "..." };
    format!("{ellipsis} {omitted} more entries")
}

/// One journal entry as text. An unknown `kind` prints its kind and nothing
/// else: a newer server's entry becomes an opaque row rather than a lie or a
/// dropped line.
fn journal_entry_text(entry: &Value, utf8: bool) -> String {
    let arrow = unit_arrow(utf8);
    let at = |key: &str| entry.get(key).map(view_scalar_text).unwrap_or_default();
    match entry.get("kind").and_then(Value::as_str).unwrap_or("") {
        "cell" => format!("cell {} {}{arrow}{}", at("addr"), at("from"), at("to")),
        "section" => format!(
            "section {} {}{arrow}{}",
            at("section"),
            at("from"),
            at("to")
        ),
        "component" => format!(
            "component {} {}{arrow}{}",
            at("component"),
            at("from"),
            at("to")
        ),
        "asset" => format!(
            "asset {} {}",
            at("component"),
            match entry.get("attached").and_then(Value::as_bool) {
                Some(true) => "attached",
                Some(false) => "detached",
                None => "?",
            }
        ),
        kind @ ("capability_granted" | "capability_revoked") => {
            match entry.get("source").and_then(Value::as_str) {
                Some(source) => format!("{kind} {} ({})", at("id"), clean(source)),
                None => format!("{kind} {}", at("id")),
            }
        }
        "elided" => elided_text(entry, utf8),
        other => clean(other),
    }
}

/// One entry as a before/after table row: a cell by its address, a section or
/// component transition by name; anything else falls back to the journal's
/// own wording.
fn delta_entry_text(entry: &Value, utf8: bool) -> String {
    let arrow = unit_arrow(utf8);
    let at = |key: &str| entry.get(key).map(view_scalar_text).unwrap_or_default();
    match entry.get("kind").and_then(Value::as_str).unwrap_or("") {
        "cell" => format!("{}  {} {arrow} {}", at("addr"), at("from"), at("to")),
        "section" => format!(
            "section {}  {} {arrow} {}",
            at("section"),
            at("from"),
            at("to")
        ),
        "component" => format!(
            "component {}  {} {arrow} {}",
            at("component"),
            at("from"),
            at("to")
        ),
        _ => journal_entry_text(entry, utf8),
    }
}

/// The ring split into its deltas: rows sharing an `(object, rev, at)` are
/// contiguous by construction of `push_unit_journal`, so one pass groups
/// them. Oldest delta first, entries in wire order within each.
fn journal_runs(ring: &VecDeque<Value>) -> Vec<Vec<&Value>> {
    let mut runs: Vec<Vec<&Value>> = Vec::new();
    for row in ring {
        let same = runs.last().and_then(|r| r.last()).is_some_and(|prev| {
            prev.get("object") == row.get("object")
                && prev.get("rev") == row.get("rev")
                && prev.get("at") == row.get("at")
        });
        if same {
            runs.last_mut().expect("a run to extend").push(row);
        } else {
            runs.push(vec![row]);
        }
    }
    runs
}

/// `rev N` for a ring row, or `rev ?` when the delta carried no usable one.
fn journal_rev_text(rev: Option<&Value>) -> String {
    match rev.and_then(Value::as_u64) {
        Some(rev) => format!("rev {rev}"),
        None => "rev ?".to_string(),
    }
}

/// The damage screen's `journal` pane: the after-action ring, newest DELTA
/// first and each delta's entries in the order the server wrote them (so the
/// `elided` marker lands last within its own delta, where the entries it
/// stands for are) — `rev N · cause · <entry>`. A row about another subject
/// than the one held prints that subject first; with NO view held (an
/// unstationed staffer's own `@unit/damage`, answered with the honest `null`
/// keyframe) every row is prefixed, and the ring is still the report. The
/// NO-UNIT line is for the case where there is nothing at all to show.
#[must_use]
pub fn render_unit_journal_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let held = unit_object(data).map(|(id, _)| id);
    let ring = ctx.store.unit_journal();
    if ring.is_empty() {
        return PaneRows::one(if held.is_none() {
            NO_UNIT_LINE
        } else {
            "(no journal entries yet)"
        });
    }
    let sep = unit_sep(ctx.utf8);
    let dash = if ctx.utf8 { "\u{2014}" } else { "-" };
    let null = Value::Null;
    let mut rows = Vec::new();
    for run in journal_runs(ring).iter().rev() {
        for row in run {
            let entry = row.get("entry").unwrap_or(&null);
            let mut text = String::new();
            if let Some(object) = row.get("object").and_then(Value::as_str)
                && held != Some(object)
            {
                text.push_str(&clean(object));
                text.push(' ');
            }
            text.push_str(&journal_rev_text(row.get("rev")));
            text.push_str(sep);
            text.push_str(&clean(
                row.get("cause").and_then(Value::as_str).unwrap_or(dash),
            ));
            text.push_str(sep);
            text.push_str(&journal_entry_text(entry, ctx.utf8));
            let text = truncate_chars(&text, 160);
            // The elision marker is the one row that is not a fact about the
            // unit, so it stays dim — but it keeps its delta's object,
            // revision and cause like any other row, or a mixed ring could
            // not say which volley was cut.
            let span = if entry.get("kind").and_then(Value::as_str) == Some("elided") {
                dim_span_owned(&text)
            } else {
                Span::raw(text)
            };
            rows.push(PaneRow::plain(Line::from(span)));
        }
    }
    PaneRows { rows }
}

/// The damage screen's `before / after` pane: the newest-ARRIVED delta in the
/// ring for the subject actually held, one row per entry with both sides. The
/// delta carries `from` and `to` itself, so no second keyframe is kept.
///
/// Newest-arrived, never the highest revision: a remove-and-recreate resets a
/// unit to revision 0, and the highest revision in the ring would then be the
/// dead unit's last damage report shown as the current one. With NO view held
/// the newest-arrived row names the subject, and the head says whose it is.
#[must_use]
pub fn render_unit_delta_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    let held = unit_object(data).map(|(id, _)| id);
    let ring = ctx.store.unit_journal();
    let mine = |row: &&Value| match held {
        Some(held) => row.get("object").and_then(Value::as_str) == Some(held),
        None => true,
    };
    let Some(newest) = ring.iter().rev().find(mine) else {
        return PaneRows::one(if held.is_none() && ring.is_empty() {
            NO_UNIT_LINE
        } else {
            "no after-action entries yet"
        });
    };
    let (object, rev) = (newest.get("object"), newest.get("rev"));
    let null = Value::Null;
    let mut rows = Vec::new();
    let mut cause: Option<String> = None;
    for row in ring
        .iter()
        .filter(|row| row.get("object") == object && row.get("rev") == rev)
    {
        if cause.is_none() {
            cause = row.get("cause").and_then(Value::as_str).map(clean);
        }
        let entry = row.get("entry").unwrap_or(&null);
        rows.push(PaneRow::plain(Line::from(Span::raw(truncate_chars(
            &delta_entry_text(entry, ctx.utf8),
            160,
        )))));
    }
    // With a view held the head names only the revision (the pane sits beside
    // the subject's own section map); with none, the subject is the news.
    let mut head = String::new();
    if held.is_none()
        && let Some(object) = object.and_then(Value::as_str)
    {
        head.push_str(&clean(object));
        head.push(' ');
    }
    head.push_str(&journal_rev_text(rev));
    if let Some(cause) = &cause {
        head.push_str("  ");
        head.push_str(&truncate_chars(cause, 24));
    }
    rows.insert(
        0,
        PaneRow::plain(Line::from(Span::styled(
            head,
            Style::default().add_modifier(Modifier::BOLD),
        ))),
    );
    PaneRows { rows }
}

// ---------------------------------------------------------------------------
// The cockpit screen (16b cut Q.4; doc 16 §5).
//
// Six panes over ONE feed, `arena.cockpit`, plus the pulled `arena.map` the map
// pane composes with. Nothing here names a unit, a player or an object number:
// every word on the screen comes out of the frame, which is already filtered by
// the arena's own observer rule (doc 16 §1, §3.6) — a contact is its designator,
// its class word and its plot, and that is all there is to print.
// ---------------------------------------------------------------------------

/// The one line every cockpit pane shows for a frame that says the viewer holds
/// no seat (16b cut Q.3's `seated: false`, the last frame a pilot is sent).
pub const NOT_IN_A_COCKPIT: &str = "Not in a cockpit.";

/// What the map pane says while the PULLED `arena.map` of this match has not
/// arrived (16b cut Q.3: nothing ever pushes that feed).
pub const WAITING_FOR_THE_MAP: &str = "Waiting for the arena map...";

/// The window the map pane carves when nothing told it how big its pane is:
/// seventy-eight by twenty cells, one glyph a cell — the width of the columns a
/// line client prints.
///
/// It is NOT the size `VIEW` sends, nor the size the committed fixtures are
/// composed at: since 16b cut B.4 `VIEW` sends thirty-nine by twenty cells,
/// which a line client prints two columns a cell, and the parity compositions
/// (`tests/tui_golden.rs`' `(39, 22)` pane among them) run at thirty-nine by
/// twenty against `view_grid.txt`, one glyph per cell.
///
/// It is a fallback and not a guess about the terminal: the map pane is not
/// `selectable` and never scrolls (it renders exactly its area), so the only
/// callers that reach it are the input paths, which ask what the rows ARE and
/// never how many of them fit.
pub const COCKPIT_MAP_FALLBACK: (u16, u16) = (78, 20);

/// The rows the map pane spends on its own chrome: the heading above the grid
/// and the marker legend below it. They come out of the pane's height, so the
/// pane renders exactly its area — a pilot standing on the arena's south edge
/// is drawn on the window's LAST row, and a pane that overran by two would clip
/// the one marker the screen exists to show.
pub(super) const MAP_CHROME_ROWS: u16 = 2;

/// Whether a cockpit or map frame is one for a seated pilot.
///
/// `seated` is present ONLY on the cleared frame, so its absence is the normal
/// case and `false` is the one that empties the screen.
pub(super) fn seated(data: &Value) -> bool {
    data.get("seated").and_then(Value::as_bool) != Some(false)
}

/// A `u64`-ish field as a `usize`, `0` when it is missing or malformed.
pub(super) fn cell_num(data: &Value, key: &str) -> usize {
    data.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

/// An `[x, y]` arena cell, `None` for a null or malformed one.
pub(super) fn cell_pair(v: Option<&Value>) -> Option<(usize, usize)> {
    let arr = v?.as_array()?;
    let x = usize::try_from(arr.first()?.as_u64()?).ok()?;
    let y = usize::try_from(arr.get(1)?.as_u64()?).ok()?;
    Some((x, y))
}

/// The glyph a contact is drawn with, by the ONE rule the server already
/// follows (`tacsim::match_::cockpit::ContactRow::glyph`): unknown before side,
/// then friendly or hostile. The cockpit frame carries `class` and `friendly`,
/// so a compositor derives it rather than carrying a second table that could
/// disagree with the server's.
fn contact_glyph(contact: &Value) -> char {
    let class = contact.get("class").and_then(Value::as_str).unwrap_or("");
    if class == "unknown" {
        '?'
    } else if contact
        .get("friendly")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        '+'
    } else {
        'x'
    }
}

/// The degree mark a heading or a bearing carries, or nothing at all on a
/// terminal that told us its charset is not UTF-8 (`LANG=C` and friends, see
/// [`crate::caps::detect_utf8`]).
///
/// It has to degrade like every other non-ASCII glyph on this surface. The TUI
/// paints CELLS: a terminal that renders `\u{b0}`'s two bytes as two glyphs
/// pushes every column after it one to the right, and the pane borders stop
/// lining up for the rest of the frame. A line client can wear that; a
/// full-screen one cannot.
pub(super) fn degree(utf8: bool) -> &'static str {
    if utf8 { "\u{b0}" } else { "" }
}

/// The style one scan band letter is printed in (`g`/`y`/`r`/`d`, 16b cut Q.2).
fn band_style(letter: char, depth: ColorDepth) -> Style {
    let mut style = Style::default();
    if letter == 'd' {
        style = style.add_modifier(Modifier::DIM);
    }
    if matches!(depth, ColorDepth::None) {
        return style;
    }
    match letter {
        'g' => style.fg(Color::Indexed(2)),
        'y' => style.fg(Color::Indexed(3)),
        'r' | 'd' => style.fg(Color::Indexed(1)),
        _ => style,
    }
}

/// The designator a row prints, sanitized: server text on a pane border and in
/// a prefill, so it is cut to what a designator can be.
fn designator_of(contact: &Value) -> String {
    truncate_chars(
        &clean(
            contact
                .get("designator")
                .and_then(Value::as_str)
                .unwrap_or("?"),
        ),
        8,
    )
}

/// **The map pane** (16b cut Q.4; doc 16 §5): the arena's terrain windowed on
/// the pilot, with the frame's own contacts drawn on it, north-up.
///
/// `data` is the `arena.cockpit` keyframe; the terrain comes out of the store's
/// `arena.map`, and the two are composed here rather than server-side, which is
/// what makes the map free to redraw as fast as frames arrive. **No JSON is
/// cloned**: [`mush_ansi::window_rows`] expands only the rows the window shows.
///
/// # The composition, which the web client must match glyph for glyph
///
/// 1. The window is `mush_ansi::viewport(map width, map height, the cockpit
///    frame's own `cell`, grid width, grid height)` — centred on the platform
///    and clamped to the arena, the SAME rule `VIEW` uses and differing only in
///    size. It answers `(x0, y0, w, h)`. The grid's size is the pane's, less
///    `MAP_CHROME_ROWS`; with no pane it is [`COCKPIT_MAP_FALLBACK`]. The
///    committed fixtures are composed at thirty-nine by twenty cells (16b cut
///    B.4), the window `VIEW` sends.
/// 2. The terrain is `window_rows(map rows, map width, map height, x0, y0, w,
///    h)`: row `0` is the window's SOUTH row, each exactly `w` glyphs.
/// 3. The markers go down in the order 16b cut V.1 fixed — the map's named
///    `spots` first, in authored order, each as its one-character `letter`;
///    then the pilot's own `@`; then the contacts in the frame's own order —
///    each at its ABSOLUTE cell less the window origin, and a later marker
///    overwrites an earlier one on the same cell. So a contact or the `@`
///    standing on a spot hides its letter. A spot or a contact whose `cell` is
///    null or outside the window is not drawn (a contact is still in the
///    contacts pane), and a spot with no letter, a longer one or no cell is
///    skipped.
/// 4. A contact's glyph is `contact_glyph`'s: `?` unknown, `+` friendly, `x`
///    hostile.
/// 5. The rows print NORTH-UP: `y` from `h - 1` down to `0`.
///
/// The rows this returns are, in order: the heading line, exactly `h` grid rows,
/// then the marker legend. The parity test reads the `h` in the middle.
///
/// **The legend names what is drawn** (16b cut B.3, `B-62`). It lists, in frame
/// order, each contact that OWNS its window cell — the last marker written
/// there — so two contacts on one cell name the later, whose glyph is the one on
/// screen; the earlier is still a row of the contacts pane. A contact under the
/// pilot's own `@` cannot hide from the legend: the `@` is placed first and a
/// contact overwrites it, so the `@` owns a cell only no contact is on. **Then**
/// (16b cut V.1) it lists, in authored order, each spot that owns its cell, as
/// `A The Ford`; a spot under the `@` or a contact is named nowhere. The owner
/// map is keyed by `MapOwner`, and the `@` is recorded in it, so neither a
/// covered spot nor the `@` is ever listed. Contacts come first because
/// `fit_legend` keeps entries from the front: a pane too narrow for the whole
/// legend drops the name of a field before it drops a contact a pilot aims by.
/// The web's `markerLegend` runs the same rule, and so does `VIEW`'s own legend.
///
/// **The chrome goes before the grid does** (16b cut B.3, `B-65`). With the
/// pane's height `h` known: four rows or more are the heading, `h − 2` grid rows
/// and the legend; three are two grid rows and the legend, the heading shed;
/// two or fewer are `h` grid rows and nothing else, because a map pane that
/// cannot hold the ground AND its words keeps the ground. Every grid row is
/// exactly the window's width in glyphs and the legend is cut to it
/// (`fit_legend`), so no row of the pane ever wraps and the pane is exactly
/// its height. With no pane (the input paths) the fallback window keeps both
/// chrome rows.
#[must_use]
pub fn render_cockpit_map_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        // 16b cut I.2: no seat, and a replay open — its map.
        if let Some(rows) = replay::map_rows(ctx) {
            return rows;
        }
        return PaneRows::one(NOT_IN_A_COCKPIT);
    }
    let Some(map) = ctx
        .store
        .get(ARENA_MAP_FEED)
        .filter(|map| seated(map) && map.get("match") == data.get("match"))
    else {
        return PaneRows::one(WAITING_FOR_THE_MAP);
    };
    let width = cell_num(map, "width");
    let height = cell_num(map, "height");
    let empty = Vec::new();
    let rows = map.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    // (grid width, grid height, heading?, legend?)
    let (grid_w, grid_h, heading, with_legend) = match ctx.pane {
        Some((w, h)) if h > MAP_CHROME_ROWS + 1 => (w, h - MAP_CHROME_ROWS, true, true),
        Some((w, h)) if h == MAP_CHROME_ROWS + 1 => (w, h - 1, false, true),
        Some((w, h)) => (w, h.max(1), false, false),
        None => (COCKPIT_MAP_FALLBACK.0, COCKPIT_MAP_FALLBACK.1, true, true),
    };
    let you = cell_pair(data.get("cell"));
    let (x0, y0, w, h) =
        mush_ansi::viewport(width, height, you, usize::from(grid_w), usize::from(grid_h));
    let mut grid = mush_ansi::window_rows(rows, width, height, x0, y0, w, h);
    // Which marker wrote each window cell LAST (16b cut B.3): only a lookup,
    // never iterated, so the legend's order is the frame's and nothing else.
    // **16b cut V.1** keyed it by `MapOwner`, so a spot's index, the `@` and a
    // contact's index cannot be mistaken for one another.
    let mut owner: HashMap<(usize, usize), MapOwner> = HashMap::new();
    // 16b cut V.1: the map's named spots FIRST, in authored order — the ground
    // under everything — then the pilot's own `@`, then the contacts in frame
    // order: a later marker overwrites an earlier one on the same cell.
    let mut spots_landed: Vec<(usize, char, String, (usize, usize))> = Vec::new();
    let empty_spots = Vec::new();
    let spots = map
        .get("spots")
        .and_then(Value::as_array)
        .unwrap_or(&empty_spots);
    for (index, spot) in spots.iter().enumerate() {
        // An entry with no letter, a letter that is not exactly one
        // character, or no cell is skipped: the map is server text.
        let Some(letter) = spot
            .get("letter")
            .and_then(Value::as_str)
            .and_then(one_char)
        else {
            continue;
        };
        let Some(cell) = cell_pair(spot.get("cell")) else {
            continue;
        };
        if let Some(at) = put_marker(&mut grid, (x0, y0), cell, letter) {
            owner.insert(at, MapOwner::Spot(index));
            let name = clean(spot.get("name").and_then(Value::as_str).unwrap_or(""));
            spots_landed.push((index, letter, name, at));
        }
    }
    // The `@` is recorded too (16b cut V.1): a spot under the pilot's own cell
    // is hidden by it exactly as by a contact. It is never listed.
    if let Some(cell) = you
        && let Some(at) = put_marker(&mut grid, (x0, y0), cell, '@')
    {
        owner.insert(at, MapOwner::You);
    }
    let mut landed: Vec<(usize, char, (usize, usize))> = Vec::new();
    let empty_contacts = Vec::new();
    let contacts = data
        .get("contacts")
        .and_then(Value::as_array)
        .unwrap_or(&empty_contacts);
    for (index, contact) in contacts.iter().enumerate() {
        let Some(cell) = cell_pair(contact.get("cell")) else {
            continue;
        };
        let glyph = contact_glyph(contact);
        if let Some(at) = put_marker(&mut grid, (x0, y0), cell, glyph) {
            owner.insert(at, MapOwner::Contact(index));
            landed.push((index, glyph, at));
        }
    }
    // Named contacts first, then spots — the reverse of the drawing, so that
    // `fit_legend`, which keeps entries from the FRONT, drops a field's name
    // before it drops a contact a pilot aims by.
    let mut legend: Vec<String> = landed
        .iter()
        .filter(|(index, _, at)| owner.get(at) == Some(&MapOwner::Contact(*index)))
        .filter_map(|(index, glyph, _)| {
            contacts
                .get(*index)
                .map(|contact| format!("{glyph} {}", designator_of(contact)))
        })
        .collect();
    legend.extend(
        spots_landed
            .iter()
            .filter(|(index, _, _, at)| owner.get(at) == Some(&MapOwner::Spot(*index)))
            .map(|(_, letter, name, _)| format!("{letter} {name}")),
    );
    let mut out = Vec::with_capacity(h + 2);
    if heading {
        let sep = if ctx.utf8 { "\u{2014}" } else { "-" };
        out.push(PaneRow::plain(Line::from(Span::styled(
            format!(
                "Map {sep} heading {:03}{}",
                cell_num(data, "heading_deg"),
                degree(ctx.utf8)
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ))));
    }
    for y in (0..h).rev() {
        let row: String = grid.get(y).map(|r| r.iter().collect()).unwrap_or_default();
        out.push(PaneRow::plain(Line::from(Span::raw(row))));
    }
    if with_legend {
        let width = ctx.pane.map(|(w, _)| usize::from(w));
        out.push(PaneRow::plain(Line::from(dim_span_owned(&fit_legend(
            &legend, width,
        )))));
    }
    PaneRows { rows: out }
}

/// **The legend on ONE line of the pane** (16b cut B.3, `B-65`): every panel
/// row wraps now, and a legend that wrapped would push the map pane past its
/// height — a second legend line cut off below the frame, and a pane with more
/// lines than rows that Up and PgDn would scroll, heading and ground with it.
/// So with the pane's width known the legend keeps whole entries, in frame
/// order, while they fit, and says how many it left off (`x BA  + RB  3
/// more`); every contact is still a row of the contacts pane. With no pane (the
/// input paths) it is whole.
pub(super) fn fit_legend(entries: &[String], width: Option<usize>) -> String {
    let whole = entries.join("  ");
    let Some(width) = width else {
        return whole;
    };
    if whole.chars().count() <= width {
        return whole;
    }
    for keep in (0..entries.len()).rev() {
        let kept = entries.get(..keep).unwrap_or_default();
        let more = format!("{} more", entries.len() - keep);
        let line = if kept.is_empty() {
            more
        } else {
            format!("{}  {more}", kept.join("  "))
        };
        if line.chars().count() <= width {
            return line;
        }
    }
    String::new()
}

/// Write `glyph` into the window at an ABSOLUTE arena `cell`, answering the
/// WINDOW cell it landed on (16b cut B.3: the legend's owner map is keyed on
/// it). A cell south or west of the window's origin, or past its far corner, is
/// not drawn — it is still listed in the contacts pane.
pub(super) fn put_marker(
    grid: &mut [Vec<char>],
    origin: (usize, usize),
    cell: (usize, usize),
    glyph: char,
) -> Option<(usize, usize)> {
    let (x0, y0) = origin;
    let (x, y) = cell;
    if x < x0 || y < y0 {
        return None;
    }
    let at = (x - x0, y - y0);
    let slot = grid.get_mut(at.1).and_then(|row| row.get_mut(at.0))?;
    *slot = glyph;
    Some(at)
}

/// **Who wrote a map-pane cell last** (16b cut V.1): the key of
/// [`render_cockpit_map_rows`]' owner map. Three kinds of marker go down on the
/// one grid, and an index of one kind must never read as an index of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MapOwner {
    /// A named spot, by its index in the map's `spots`.
    Spot(usize),
    /// The pilot's own `@`.
    You,
    /// A contact, by its index in the frame's `contacts`.
    Contact(usize),
}

/// The one character of a one-character string (a spot's `letter`), or `None`
/// for an empty or a longer one — or a control character, which a grid cell
/// must never be painted with (server text is cleaned on this surface).
pub(super) fn one_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(one), None) if !one.is_control() => Some(one),
        _ => None,
    }
}

/// **The status pane** (16b cut Q.4): the callsign the whole arena calls this
/// tank by, then what the platform is doing and what it is holding.
#[must_use]
pub fn render_cockpit_status_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        // 16b cut I.2: no seat, and a replay open — what it is.
        return replay::status_rows(ctx).unwrap_or_default();
    }
    let system = ctx.tag("system");
    let mut rows = vec![PaneRow::plain(Line::from(Span::styled(
        format!(
            "Callsign {}",
            truncate_chars(
                &clean(data.get("callsign").and_then(Value::as_str).unwrap_or("?")),
                8
            )
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )))];
    let held = if data.get("blocked").and_then(Value::as_bool) == Some(true) {
        "  HELD"
    } else {
        ""
    };
    rows.push(PaneRow::plain(Line::from(Span::raw(format!(
        "power {}{held}",
        clean(data.get("power").and_then(Value::as_str).unwrap_or("?"))
    )))));
    // **16b cut B.1** (`T-86`): the pilot's own two limits (16b cut D.2's
    // `wreck` and `immobile`) on the speed line, the web's words in the web's
    // place — a wreck says only `DESTROYED`, because it is immobile as well.
    let limit = if data.get("wreck").and_then(Value::as_bool) == Some(true) {
        "  DESTROYED"
    } else if data.get("immobile").and_then(Value::as_bool) == Some(true) {
        "  IMMOBILE"
    } else {
        ""
    };
    rows.push(PaneRow::plain(Line::from(Span::raw(format!(
        "{} kph  heading {:03}{}{limit}",
        data.get("speed_kph").and_then(Value::as_i64).unwrap_or(0),
        cell_num(data, "heading_deg"),
        degree(ctx.utf8)
    )))));
    // 16b cut X.1: heat is a line of its own, and only for a platform that
    // carries it — the frame leaves the key null otherwise.
    if let Some(heat) = data.get("heat").filter(|h| !h.is_null()) {
        let over = if heat.get("overheated").and_then(Value::as_bool) == Some(true) {
            "  OVERHEATED"
        } else {
            ""
        };
        rows.push(PaneRow::plain(Line::from(Span::raw(format!(
            "heat {}/{}{over}",
            heat.get("heat").and_then(Value::as_i64).unwrap_or(0),
            heat.get("max").and_then(Value::as_i64).unwrap_or(0),
        )))));
    }
    // **16b cut B.1** (`T-86`): the platform's own receiver (16b cut D.4's
    // `receiver` key), only while it is degraded — the frame leaves the key null
    // on a clean receiver, so the row's presence is what says "degraded". Budget
    // over the unjammed budget, then what is left of the reach:
    // `receiver 15/40  60%`, 19 characters (22 at three digits and 100%), so it
    // never wraps the pane's 32 inner columns.
    // Three integers about the pilot's own machine and nothing about the source.
    if let Some(receiver) = data.get("receiver").filter(|r| r.is_object()) {
        let num = |key: &str| receiver.get(key).and_then(Value::as_i64).unwrap_or(0);
        rows.push(PaneRow::plain(Line::from(Span::raw(format!(
            "receiver {}/{}  {}%",
            num("budget"),
            num("budget_max"),
            num("reach_percent"),
        )))));
    }
    // **16b cut D.2** (16c `T-59`, the panel half): this platform's own scope and
    // not the arena's one global tempo word. Cut D.1 put `scope` on the frame and
    // made `STATUS`'s third line read it; both panels went on printing `tempo`,
    // so a pilot sitting in an empty scope read `engaged` off somebody else's
    // fight. `tempo` stays on the frame and stays decoded, because `T-59` is
    // closed for the printed word alone and a client may yet want the cadence;
    // no shipped client reads it today, here or in `web/`.
    //
    // **16b cut O.2** (16c N12): the pilot's own pod's switch rides this row,
    // `scope hostile  EMCON on` / `  EMCON off`, and nothing is added for a
    // platform with no pod (`ecm` null or absent) — not an eighth row, because
    // cut AB.1 sized the pane at seven (`screen.rs`). The longest,
    // `scope unidentified  EMCON off`, is 29 of the pane's 32 inner columns.
    let pod = match data.get("ecm").and_then(Value::as_str) {
        Some("on") => "  EMCON on",
        Some("off") => "  EMCON off",
        _ => "",
    };
    rows.push(PaneRow::plain(Line::from(Span::styled(
        format!(
            "scope {}{pod}",
            clean(data.get("scope").and_then(Value::as_str).unwrap_or("?"))
        ),
        system,
    ))));
    let lock = match data.get("lock_designator").and_then(Value::as_str) {
        Some(named) => {
            let mut line = format!("Lock {}", truncate_chars(&clean(named), 8));
            if let Some(range) = data.get("lock_range_m").and_then(Value::as_u64) {
                line.push_str(&format!("  {range} m"));
            }
            if let Some(percent) = data.get("lock_percent").and_then(Value::as_u64) {
                line.push_str(&format!("  {percent}%"));
            }
            line
        }
        None => "No lock".to_string(),
    };
    rows.push(PaneRow::plain(Line::from(Span::raw(lock))));
    PaneRows { rows }
}

/// **The damage pane** (16b cut Q.4): each section's condition word and
/// internals, then each facet's armour. A pilot's own tank is outside the
/// observer rule entirely (doc 16 §3.6), so this is the whole of it.
///
/// **16b cut AB.1** reads the pane's size (`ctx.pane`). Per section, one row
/// `{id}  {condition}  {internals}/{internals_max}`, then the facet cells
/// `{id} {armor}/{armor_max}` PACKED into rows of at most the pane's inner
/// width — two spaces of indent, two spaces between cells, a new row when the
/// next cell would not fit — which is the rule
/// `tacsim::match_::cockpit::section_lines` uses at `DAMAGE_COLUMNS`, at the
/// pane's width instead. A facet's name is therefore never split from its
/// numbers (cut B.3 wrapped one long row per section at whatever space fell at
/// the edge). A cell wider than the whole pane sits on a row of its own, where
/// the pane's wrap splits it; no shipped content reaches that. A section with
/// no facets is one row.
///
/// The shipped line tank at the Left column's 32 inner columns is TWELVE rows —
/// the hull four (`hull  intact  200/200`, `  front 400/400  left 250/250`,
/// `  right 250/250  rear 150/150`, `  top 100/100`), the turret three, each
/// track two, the engine one — which is the pane's `min_h`, so from
/// `COCKPIT_ROWS` every section of the stock roster is on screen. The biped
/// (six sections of three to five facets) is nineteen: at 120 by 34 the pane's
/// twelve inner rows show the torso, the head, the left arm and the first two
/// rows of the right arm, with the rest below the fold; at 160 by 48 the
/// pane's eighteen (ratatui gives the Left column's odd row to the status
/// pane above it) show all but the last, the right leg's `  rear 180/180`.
///
/// With no pane size (`ctx.pane == None`, the input paths that ask what the
/// rows ARE) the output is one row per section with its facets appended to it
/// (`hull  intact  60/60  front 55/80`), as before the cut.
#[must_use]
pub fn render_cockpit_damage_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        // 16b cut I.2: no seat, and a replay open — the wrecks so far.
        return replay::damage_rows(ctx).unwrap_or_default();
    }
    let Some(sections) = data.get("sections").and_then(Value::as_array) else {
        return PaneRows::default();
    };
    let width = ctx.pane.map(|(w, _)| usize::from(w));
    let mut rows = Vec::with_capacity(sections.len());
    for section in sections {
        let head = format!(
            "{}  {}  {}/{}",
            clean(section.get("id").and_then(Value::as_str).unwrap_or("?")),
            clean(
                section
                    .get("condition")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
            ),
            section
                .get("internals")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            section
                .get("internals_max")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        );
        let cells: Vec<String> = section
            .get("facets")
            .and_then(Value::as_array)
            .map(|facets| {
                facets
                    .iter()
                    .map(|facet| {
                        format!(
                            "{} {}/{}",
                            clean(facet.get("id").and_then(Value::as_str).unwrap_or("?")),
                            facet.get("armor").and_then(Value::as_i64).unwrap_or(0),
                            facet.get("armor_max").and_then(Value::as_i64).unwrap_or(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(width) = width else {
            // No pane: the section's one row, its facets appended.
            let mut line = head;
            for cell in &cells {
                line.push_str("  ");
                line.push_str(cell);
            }
            rows.push(PaneRow::plain(Line::from(Span::raw(line))));
            continue;
        };
        rows.push(PaneRow::plain(Line::from(Span::raw(head))));
        for line in pack_facet_cells(&cells, width) {
            rows.push(PaneRow::plain(Line::from(Span::raw(line))));
        }
    }
    PaneRows { rows }
}

/// A section's facet cells packed into rows of at most `width` characters (16b
/// cut AB.1): each row starts with two spaces, two spaces join two cells, and
/// a cell that would carry a row past `width` starts the next one. A cell too
/// wide for any row is a row of its own. Characters are counted, not bytes, so
/// a cleaned id with a wide letter in it is measured as the pane measures it.
fn pack_facet_cells(cells: &[String], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut run = String::new();
    let mut used = 0usize;
    for cell in cells {
        let len = cell.chars().count();
        if !run.is_empty() && used + 2 + len > width {
            lines.push(std::mem::take(&mut run));
            used = 0;
        }
        run.push_str("  ");
        run.push_str(cell);
        used += 2 + len;
    }
    if !run.is_empty() {
        lines.push(run);
    }
    lines
}

/// **The weapons pane** (16b cut Q.4): what each weapon is, whether its cycle
/// has finished, what it has left, and its OWN chance against the lock (16b cut
/// F.2's item 4) where a lock gives one.
///
/// **17b cut M1b.3** (doc 17 §5; D-UM-4): under a weapon whose row carries an
/// `armed` object and no canonical rules, one more plain row — two spaces and
/// `arming_line`'s words,
/// the same line line mode's `WEAPONSTATUS` prints under the row and the web's
/// `weaponLines` draws (`  S1: >=60% side/rear keeps 8`). A row past the
/// pane's 46 columns wraps onto a second pane row by the pane's own rule
/// (`draw_pane`'s `wrap_spans`, 16b cut B.3), as a long contact row does. A
/// frame from an older server carries no `armed` key and prints as it always
/// did.
///
/// **17b cut M5.5** (doc 17 §12; the M5 design note's M5.5 item 1): under the
/// weapon rows, the mind's four rows — [`render_cockpit_mind_tail`], the
/// `MIND` block as this pane draws it — and on a hull with no weapons station
/// the same four rows in `(no weapon)`'s place. A frame with no `mind` object
/// (no seal reached the hull, or an older server) prints exactly as before.
/// The tail is the pane's last rows, so where the pane is short (six inner
/// rows at 120 by 30 under the scene's three) it loses its own last row, the
/// channels, from the bottom. Canonical rule summaries precede the weapon rows.
///
/// M5rules: before the weapons, every ordered rule's slot and action in at
/// most two rows. Holds and Sensors rules remain visible on an unarmed hull;
/// `MIND RULES` supplies their full terms without consuming contact rows.
#[must_use]
pub fn render_cockpit_weapons_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        // 16b cut I.2: no seat, and a replay open — the shots of this step.
        // 17b cut M5.5: the replay's and the live picture's rows carry no
        // mind tail; the tail is a seated pilot's alone.
        return replay::weapons_rows(ctx).unwrap_or_default();
    }
    // **17b cut M1a.3** (doc 17 §22's italic line): a hull with no weapons
    // station carries no `weapons` key at all, which is the same statement an
    // empty list made before it.
    let weapons = data
        .get("weapons")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let tail = render_cockpit_mind_tail(data, ctx);
    let mut rows: Vec<_> = rule_summary_lines(data)
        .into_iter()
        .map(|text| PaneRow::plain(Line::from(Span::raw(text))))
        .collect();
    let has_rules = !rows.is_empty();
    if weapons.is_empty() {
        // 17b cut M5.5: the mind's rows take the weapons' place.
        if tail.rows.is_empty() {
            rows.extend(PaneRows::one("(no weapon)").rows);
        } else {
            rows.extend(tail.rows);
        }
        return PaneRows { rows };
    }
    for weapon in weapons {
        let ready = if weapon.get("ready").and_then(Value::as_bool) == Some(true) {
            "ready".to_string()
        } else {
            format!(
                "{} s",
                weapon
                    .get("ready_in_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .saturating_add(999)
                    / 1_000
            )
        };
        let chance = match weapon.get("percent").and_then(Value::as_u64) {
            Some(percent) => format!("  {percent}%"),
            None => String::new(),
        };
        rows.push(PaneRow::plain(Line::from(Span::raw(format!(
            "{}  {ready}  {}/{}{chance}",
            clean(weapon.get("id").and_then(Value::as_str).unwrap_or("?")),
            weapon.get("rounds").and_then(Value::as_i64).unwrap_or(0),
            weapon
                .get("rounds_max")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        )))));
        // Older frames retain the first-fire convenience line. Canonical
        // rules carry predicates that this shortened line cannot express.
        if let Some(words) = weapon
            .get("armed")
            .filter(|_| !has_rules)
            .and_then(arming_line)
        {
            rows.push(PaneRow::plain(Line::from(Span::raw(format!("  {words}")))));
        }
    }
    // 17b cut M5.5: the mind's rows, last.
    rows.extend(tail.rows);
    PaneRows { rows }
}

/// The core's six slots, summarized at the 44-column floor. Match the web's
/// `decodeRules` + `ruleSummaryLines`: accept known actions, one per valid
/// slot, and always draw them in slot order. Arbitrary rule text is never
/// copied into a summary row.
fn rule_summary_lines(data: &Value) -> Vec<String> {
    let Some(rules) = data.get("rules").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut slots = [None; 6];
    for row in rules {
        let Some(slot) = row.get("slot").and_then(Value::as_u64) else {
            continue;
        };
        let Ok(at) = usize::try_from(slot.saturating_sub(1)) else {
            continue;
        };
        if slot == 0 || at >= slots.len() || slots[at].is_some() {
            continue;
        }
        slots[at] = match row
            .get("rule")
            .and_then(|rule| rule.get("action"))
            .and_then(Value::as_str)
        {
            Some("fire") => Some("fire"),
            Some("hold_fire") => Some("hold"),
            Some("lock") => Some("lock"),
            _ => None,
        };
    }
    let words: Vec<_> = slots
        .into_iter()
        .enumerate()
        .filter_map(|(at, action)| action.map(|action| format!("S{} {action}", at + 1)))
        .collect();
    let mut rows: Vec<_> = words
        .chunks(3)
        .enumerate()
        .map(|(at, chunk)| {
            format!(
                "{}{}",
                if at == 0 { "rules: " } else { "" },
                chunk.join(" ")
            )
        })
        .collect();
    if let Some(last) = rows.last_mut() {
        last.push_str("; MIND RULES");
    }
    rows
}

/// **The mind's four rows, the weapons pane's tail** (17b cut M5.5; doc 17
/// §12, OQ-8; the M5 design note's M5.5 item 1): the keyframe's `mind` object
/// (17b cut M5.4, the hull's sealed mind for the person in the seat) as the
/// TUI draws it, in this order —
///
/// ```text
/// mind Standard Doctrine · Class I
/// > Sniper   Line
/// dormant while you fly
/// you: helm sensors fire
/// ```
///
/// The first row names the mind (`mind: your own · Class I` for a seal that is
/// not Standard Doctrine, whose `name` is `null`) and the class it thinks at,
/// read live off the frame; the second, every sealed profile in sealed order,
/// the one flown marked `> ` and the others two spaces in, one space between
/// them; the third, `dormant while you fly`, or `off for the match: on the
/// ROM` where the ROM flies the hull — one dropped to it (`off`), or one
/// whose mind class reads `0` (no core, a core at Class 0, a seal capped
/// there), for which line mode's `MIND` names the ROM too; the fourth, the
/// channels the pilot holds, as the frame lists them. Each row is at most 44 columns, the Right
/// column's inner width at the 120-column floor, at the widest set of four
/// presets and the longest class word — so no row wraps there. The separator
/// is `·` on a UTF-8 terminal and `|` on any other (`unit_sep`, the unit
/// screen's).
///
/// No countdown is drawn: `next_switch_ms` is absolute, and a quiet frame
/// would leave a drawn countdown stale; line mode's `MIND` prints it off the
/// mirror's clock. No row is selectable and no key is bound: `MIND PROFILE
/// <name>` is typed. F10 opens the full sealed card and all profile names;
/// long cockpit labels explicitly truncate to preserve the contact budget.
///
/// Empty when the frame carries no `mind` object — `null` on a hull no seal
/// reached, absent from an older server and from every replay or live
/// picture's payload — or when it is not seated.
#[must_use]
pub fn render_cockpit_mind_tail(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        return PaneRows::default();
    }
    let Some(mind) = data.get("mind").filter(|mind| mind.is_object()) else {
        return PaneRows::default();
    };
    PaneRows {
        rows: mind_tail_lines(mind, ctx.utf8)
            .into_iter()
            .map(|text| {
                PaneRow::plain(Line::from(Span::raw(mind_tail_fit(
                    text,
                    ctx.pane.map(|(w, _)| usize::from(w)),
                    ctx.utf8,
                ))))
            })
            .collect(),
    }
}

fn mind_tail_fit(text: String, width: Option<usize>, utf8: bool) -> String {
    let Some(width) = width else {
        return text;
    };
    if Line::from(text.as_str()).width() <= width {
        return text;
    }
    let suffix = if utf8 { "… F10" } else { "... F10" };
    if Line::from(suffix).width() > width {
        return suffix.chars().take(width).collect();
    }
    let budget = width.saturating_sub(Line::from(suffix).width());
    let mut prefix = String::new();
    for c in text.chars() {
        let mut candidate = prefix.clone();
        candidate.push(c);
        if Line::from(candidate.as_str()).width() > budget {
            break;
        }
        prefix = candidate;
    }
    prefix.push_str(suffix);
    prefix
}

/// The four rows of [`render_cockpit_mind_tail`] for one `mind` object, as
/// text.
fn mind_tail_lines(mind: &Value, utf8: bool) -> [String; 4] {
    let class = clean(mind.get("class").and_then(Value::as_str).unwrap_or("?"));
    let sep = unit_sep(utf8);
    let name = mind
        .get("name")
        .and_then(Value::as_str)
        .map(clean)
        .filter(|name| !name.trim().is_empty());
    let head = match name {
        Some(name) => format!("mind {name}{sep}Class {class}"),
        // A seal that is not Standard Doctrine: `mind: your own`, the colon
        // keeping it from reading as the idiom.
        None => format!("mind: your own{sep}Class {class}"),
    };
    let active = mind
        .get("active")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok());
    let profiles = mind
        .get("profiles")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .enumerate()
                .filter_map(|(at, name)| name.as_str().map(|name| (at, name)))
                .map(|(at, name)| {
                    let mark = if Some(at) == active { "> " } else { "  " };
                    format!("{mark}{}", clean(name))
                })
                .collect::<Vec<String>>()
                .join(" ")
        })
        .unwrap_or_default();
    // 17b cut M5.5's review: the ROM flies a hull dropped to it (`off`) and
    // one whose mind class reads 0 (no core, a core at Class 0, a seal capped
    // there) alike — line mode's `MIND` names the ROM for both (17b cut M5.4's
    // `mind_head_line`) — and a live class only falls during a match, so both
    // are off for the match.
    let on_rom = mind.get("off").and_then(Value::as_bool) == Some(true) || class == "0";
    let state = if on_rom {
        "off for the match: on the ROM"
    } else {
        "dormant while you fly"
    };
    let channels: Vec<String> = mind
        .get("channels")
        .and_then(Value::as_array)
        .map(|words| words.iter().filter_map(Value::as_str).map(clean).collect())
        .unwrap_or_default();
    let you = format!("you: {}", channels.join(" "))
        .trim_end()
        .to_string();
    [head, profiles, state.to_string(), you]
}

/// **The arming line of one weapon's `armed` object** (17b cut M1b.3) — the
/// TUI's copy of `tacsim::match_::cockpit::arming_words`, written out over the
/// keyframe's keys and pinned equal to it by the fixture scene's golden:
/// `S{slot}:` then, one space apart, `best` for a `best` target (`contact`
/// for a `contact` one; `lock`, the default, prints nothing), `>={percent}%`,
/// the facets as `front`/`side`/`rear` words joined `/` only when the mask is
/// not all four, the range as `{a}-{b}m`, `under {b}m` or `past {a}m` only
/// when bound (a `null` bound is none), and `keeps {k}` only above nought.
/// `None` for `null`, for a missing slot, and for anything not an object, so
/// a malformed object costs a row and never a word.
fn arming_line(armed: &Value) -> Option<String> {
    let slot = armed.get("slot").and_then(Value::as_u64)?;
    let mut terms: Vec<String> = vec![format!("S{slot}:")];
    match armed.get("target").and_then(Value::as_str) {
        Some("best") => terms.push("best".to_string()),
        Some("contact") => terms.push("contact".to_string()),
        _ => {}
    }
    let percent = armed
        .get("percent")
        .and_then(Value::as_u64)
        .or_else(|| {
            armed
                .get("min_permille")
                .and_then(Value::as_u64)
                .map(|permille| permille / 10)
        })
        .unwrap_or(0);
    terms.push(format!(">={percent}%"));
    let facets: Vec<&str> = armed
        .get("aspects")
        .and_then(Value::as_array)
        .map(|words| words.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let has = |word: &str| facets.contains(&word);
    let every = has("front") && has("left") && has("right") && has("rear");
    if !every {
        let words: Vec<&str> = [
            (has("front"), "front"),
            (has("left") || has("right"), "side"),
            (has("rear"), "rear"),
        ]
        .into_iter()
        .filter_map(|(on, word)| on.then_some(word))
        .collect();
        if !words.is_empty() {
            terms.push(words.join("/"));
        }
    }
    let bound = |key: &str| armed.get(key).and_then(Value::as_u64);
    match (
        bound("min_range_m").filter(|m| *m > 0),
        bound("max_range_m"),
    ) {
        (Some(near), Some(far)) => terms.push(format!("{near}-{far}m")),
        (None, Some(far)) => terms.push(format!("under {far}m")),
        (Some(near), None) => terms.push(format!("past {near}m")),
        (None, None) => {}
    }
    if let Some(keeps) = armed
        .get("keeps")
        .and_then(Value::as_u64)
        .filter(|k| *k > 0)
    {
        terms.push(format!("keeps {keeps}"));
    }
    Some(terms.join(" "))
}

/// **The contacts pane** (16b cut Q.4): one selectable row per contact —
/// `Enter` prefills `lock <designator>`, `s` prefills `scan <designator>` — and,
/// under a contact this platform has had a close look at, one row of its scan.
///
/// The scan row reads its section words out of `scan_tables[condition.table]`
/// (16b cut Q.2) and colours each by its band. A word not green also carries its
/// band letter, so a colourless terminal loses nothing: damage is the thing this
/// row exists to show.
#[must_use]
pub fn render_cockpit_contacts_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        // 16b cut I.2: no seat, and a replay open — every roster unit.
        return replay::contacts_rows(ctx).unwrap_or_default();
    }
    let Some(contacts) = data.get("contacts").and_then(Value::as_array) else {
        return PaneRows::default();
    };
    if contacts.is_empty() {
        return PaneRows::one("(no contacts)");
    }
    let empty = Vec::new();
    let tables = data
        .get("scan_tables")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut rows = Vec::new();
    for contact in contacts {
        let named = designator_of(contact);
        rows.push(PaneRow::selectable(
            Line::from(vec![
                Span::styled(
                    format!("{named:<4}"),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "{}  {:03}{}  {}  {} m  {} s{}",
                    clean(contact.get("class").and_then(Value::as_str).unwrap_or("?")),
                    cell_num(contact, "bearing_deg"),
                    degree(ctx.utf8),
                    clean(contact.get("band").and_then(Value::as_str).unwrap_or("?")),
                    contact.get("range_m").and_then(Value::as_u64).unwrap_or(0),
                    contact.get("age_s").and_then(Value::as_i64).unwrap_or(0),
                    contact_tail(contact),
                )),
            ]),
            format!("lock {named}"),
            named,
        ));
        if let Some(scan) = scan_spans(contact, tables, ctx.depth) {
            rows.push(PaneRow::plain(Line::from(scan)));
        }
    }
    PaneRows { rows }
}

/// **The compact tail of one contact row** (17b cut M1a.3; doc 17 §3; the
/// plan's departure (k)): ` {side} {NN}%` for a row whose `odds` array is not
/// empty, ` {side}` for every other — one space before each and no weapon name,
/// because the contacts pane is 46 columns inside at the 120-column floor and
/// line mode's `  side  cannon 62%` does not fit it.
///
/// `side` is the side word of the wire's facet word (`left` and `right` are
/// `side`); the figure is the FIRST entry of the contact's `odds`, which the
/// server orders default-weapon-first, so it is the gun line mode quotes. An
/// unarmed hull's frame carries no `odds` key at all, so every one of its rows
/// ends at the side word. A frame from an older server carries no `aspect`, and
/// its rows print as they always did, with no tail.
///
/// **The floor-width residual** (declared for 16c, not a defect; the brief fixed
/// the tail's shape): a row longer than the pane's 46 columns wraps onto a
/// second pane row, as the pane's scan lines already do. On shipped content
/// every such row is an `effective` one, whose band word is the longest, at a
/// three-digit range — so it is a common case in a fight, not a theoretical
/// one: a `blue` contact seen nose-on with odds (`BA  blue  045°  effective  350
/// m  2 s front 62%`, 47 columns), a `blue` one with odds and an age of ten
/// seconds or more (`… 12 s side 62%`, 47), and an `unknown` one with an age of
/// ten seconds or more (`X3  unknown  315°  effective  200 m  13 s front`, 47).
/// A `red` contact is one column narrower, so it overflows at `effective` only
/// seen nose-on with odds and a two-digit age as well; every row at `close`,
/// `far` or `beyond` fits.
fn contact_tail(contact: &Value) -> String {
    let Some(facet) = contact.get("aspect").and_then(Value::as_str) else {
        return String::new();
    };
    let side = match facet {
        "left" | "right" => "side",
        "rear" => "rear",
        _ => "front",
    };
    let percent = contact
        .get("odds")
        .and_then(Value::as_array)
        .and_then(|odds| odds.first())
        .and_then(|first| {
            first.get("percent").and_then(Value::as_u64).or_else(|| {
                first
                    .get("p_permille")
                    .and_then(Value::as_u64)
                    .map(|permille| permille / 10)
            })
        });
    match percent {
        Some(percent) => format!(" {side} {percent}%"),
        None => format!(" {side}"),
    }
}

/// One contact's scan row, or `None` when this platform holds no reading of it.
fn scan_spans(contact: &Value, tables: &[Value], depth: ColorDepth) -> Option<Vec<Span<'static>>> {
    let condition = contact.get("condition").filter(|c| !c.is_null())?;
    let table = tables.get(cell_num(condition, "table"))?.as_array()?;
    let bands = condition.get("bands").and_then(Value::as_str)?;
    let mut spans = vec![Span::raw("  ".to_string())];
    for (letter, section) in bands.chars().zip(table) {
        let word = clean(section.as_str().unwrap_or("?"));
        let text = if letter == 'g' {
            format!("{word} ")
        } else {
            format!("{word}/{letter} ")
        };
        spans.push(Span::styled(text, band_style(letter, depth)));
    }
    (spans.len() > 1).then_some(spans)
}

/// The words one notice kind is said in (16b cut B.3, `B-64`) — the web's
/// `NOTICE_WORDS` entry for entry. A kind this build predates falls back to its
/// own wire word with the underscores opened out, which can name nothing the
/// payload did not already carry.
const NOTICE_WORDS: &[(&str, &str)] = &[
    ("acquired", "new contact"),
    ("classified", "identified"),
    ("unidentified", "unidentified again"),
    ("lost", "contact lost"),
    ("lock_lost", "lock broken"),
    ("locked", "locked"),
    ("hit", "hit"),
    ("miss", "miss"),
    ("kill", "destroyed"),
    ("struck", "hit taken"),
    ("struck_spent", "hit on a dead section"),
    ("section_lost", "section gone"),
    ("target_section_lost", "section destroyed"),
    ("refused", "refused"),
    ("blocked", "held"),
    ("power_online", "power online"),
    ("destroyed", "your tank is destroyed"),
    ("disabled", "your tank is out of the fight"),
    // 16b cut O.2 (16c N12): the pilot's own pod's switch, short.
    ("emcon_on", "EMCON on"),
    ("emcon_off", "EMCON off: pod silent"),
    // 17b cut M1a.4: the motor's two answers to the pilot's own `DRIVE`, whole —
    // each is already short, and there is nothing in it to compose.
    ("drive_arrived", "DRIVE: arrived."),
    ("drive_no_route", "DRIVE: no route from here; holding."),
    // 17b cut M1b.4: a slot's shot and a slot's held shot. Both lines are
    // composed whole from the notice's fields (`slot_words`); these are the
    // fallbacks a line missing its parts would fall to.
    ("slot_fired", "fires by"),
    ("slot_held", "held"),
];

/// Why an order was refused, in the pilot's words (`Refusal::as_str`; the web's
/// `REFUSAL_WORDS`).
const REFUSAL_WORDS: &[(&str, &str)] = &[
    ("power_not_online", "power is not online"),
    ("queue_full", "too many orders at once"),
    ("wreck", "the tank is destroyed"),
    ("motion_disabled", "the tank cannot move"),
    ("no_contact", "no such contact"),
    ("no_lock", "no lock"),
    ("cycling", "still cycling"),
    ("empty", "empty"),
    ("overheated", "overheated"),
    ("no_weapon", "no such weapon"),
    ("epoch", "the seat changed hands"),
    // 16b cut O.2: `EMCON` on a tank with no pod.
    ("no_jammer", "no jamming pod"),
    // 17b cut M1b.1's three words (said here since 17b cut M1b.3): a shot with
    // no warrant, a slot count full, an arming under the Compact floor.
    ("unwarranted", "no warrant"),
    ("no_slot", "every rule slot occupied"),
    ("unlawful", "that rule is not lawful for this unit"),
    // 17b cut M5.2: a `MIND PROFILE` the arena could not take.
    ("profile", "that profile cannot be switched to now"),
];

/// Why motion stopped, in the pilot's words (`BlockedCause::as_str`; the web's
/// `CAUSE_WORDS`).
const CAUSE_WORDS: &[(&str, &str)] = &[
    ("impassable", "the ground will not take the tank"),
    ("occupied", "something is in the way"),
    ("edge", "the arena's edge"),
    ("too_steep", "too steep: drive round"),
];

/// A table's words for `word`, else the wire word with its underscores opened.
fn words_for(table: &[(&str, &str)], word: &str) -> String {
    table
        .iter()
        .find(|(wire, _)| *wire == word)
        .map_or_else(|| word.replace('_', " "), |(_, said)| (*said).to_string())
}

/// One string field of a notice as the web's decoder reads it (`optStr`):
/// control characters out, trimmed, cut to `max` characters, and `None` for a
/// missing, non-string or empty one. Every string read off a notice passes here.
fn notice_str(notice: &Value, key: &str, max: usize) -> Option<String> {
    let raw = notice.get(key).and_then(Value::as_str)?;
    let text = truncate_chars(clean(raw).trim(), max);
    (!text.is_empty()).then_some(text)
}

/// One integer of a notice's `detail`, `None` for a missing or non-integer one.
/// The wire only ever carries integers there (seconds, heat, the heat limit).
fn notice_int(detail: &Value, key: &str) -> Option<i64> {
    detail.get(key).and_then(Value::as_i64)
}

/// **One line of the cockpit's notices ring** (16b cut B.3, `B-64`, Q20): the
/// web's `noticeLine`, rule for rule, composed from the notice's own FIELDS and
/// never from its `text` — the arena already sent that sentence as a `Sys` line
/// (one sentence, one owner, 16b cut F.2).
///
/// * `refused` → `{ORDER}{ weapon} refused: {why}{, N s | , heat of max}`;
/// * `blocked` → `held: {cause words}`, or `held` alone on a null cause;
/// * `classified` / `unidentified` carrying `was` → `{was} {words}: {designator}`
///   (the old name first, the name the row reads now after the colon);
/// * anything else → `{designator}{ (FRIENDLY)} {words}{: section}`, with
///   ` (already destroyed)` on a spent `hit`.
///
/// Every string read off the notice — the kind, `designator` and `was` (cut to
/// 8), `section`, `weapon`, `refusal` and `cause` (24), `order` (16) — is
/// cleaned and bounded exactly as the web's decoder does it, so a hostile
/// string cannot break the pane. A notice with no string `kind` says nothing
/// (the web drops it at decode). **Pinned against the committed
/// `web/src/lib/fixtures/cockpit/notice_lines.json`**, so the two clients'
/// wording cannot drift.
#[must_use]
pub fn notice_words(notice: &Value) -> String {
    if notice.get("kind").and_then(Value::as_str).is_none() {
        return String::new();
    }
    let kind = notice_str(notice, "kind", 32).unwrap_or_else(|| "?".to_string());
    if kind == "refused" {
        let order = notice_str(notice, "order", 16)
            .unwrap_or_else(|| "order".to_string())
            .to_uppercase();
        let weapon = notice_str(notice, "weapon", 24)
            .map(|w| format!(" {w}"))
            .unwrap_or_default();
        // **17b cut M1b.4**: an arming under the Compact floor carrying its
        // floor reads the line-mode sentence itself (less its full stop), so
        // the ring and the scrollback say the floor in one sentence.
        if notice_str(notice, "refusal", 24).as_deref() == Some("unlawful")
            && let Some(permille) = notice
                .get("detail")
                .and_then(|d| notice_int(d, "floor_permille"))
        {
            return format!(
                "{order}{weapon} refused. The Compact floor is {}%; a condition may not ask for \
                 less",
                permille / 10
            );
        }
        let why = notice_str(notice, "refusal", 24)
            .map_or_else(|| "refused".to_string(), |r| words_for(REFUSAL_WORDS, &r));
        let detail = notice
            .get("detail")
            .filter(|d| d.is_object())
            .map(|d| {
                match (
                    notice_int(d, "secs"),
                    notice_int(d, "heat"),
                    notice_int(d, "max"),
                ) {
                    (Some(secs), _, _) => format!(", {secs} s"),
                    (None, Some(heat), Some(max)) => format!(", {heat} of {max}"),
                    _ => String::new(),
                }
            })
            .unwrap_or_default();
        return format!("{order}{weapon} refused: {why}{detail}");
    }
    if kind == "blocked" {
        return notice_str(notice, "cause", 24).map_or_else(
            || "held".to_string(),
            |cause| format!("held: {}", words_for(CAUSE_WORDS, &cause)),
        );
    }
    let words = words_for(NOTICE_WORDS, &kind);
    let friendly = notice.get("friendly").and_then(Value::as_bool) == Some(true);
    let named = notice_str(notice, "designator", 8)
        .map(|d| format!("{d}{}", if friendly { " (FRIENDLY)" } else { "" }));
    if (kind == "slot_fired" || kind == "slot_held")
        && let Some(line) = slot_words(notice, &kind, named.as_deref())
    {
        return line;
    }
    if (kind == "classified" || kind == "unidentified")
        && let Some(was) = notice_str(notice, "was", 8)
    {
        // The mark goes with the name the row reads now (`X3 identified: RB
        // (FRIENDLY)`): an own-side tank told apart must not read as a hostile.
        return match named {
            Some(named) => format!("{was} {words}: {named}"),
            None => format!("{was} {words}"),
        };
    }
    let head = match named {
        Some(named) => format!("{named} {words}"),
        None => words,
    };
    let line = match notice_str(notice, "section", 24) {
        Some(section) => format!("{head}: {section}"),
        None => head,
    };
    if kind == "hit" && notice.get("spent").and_then(Value::as_bool) == Some(true) {
        format!("{line} (already destroyed)")
    } else {
        line
    }
}

/// A notice's facet word as the pilot reads it (the server's
/// `Aspect::side_word`; the web's `sideWord`, and `contact_tail`'s rule):
/// `left` and `right` are both `side`, `rear` is `rear`, anything else
/// `front`.
fn side_word(facet: &str) -> &'static str {
    match facet {
        "left" | "right" => "side",
        "rear" => "rear",
        _ => "front",
    }
}

/// **A slot notice's line** (17b cut M1b.4; doc 17 §5, D-UM-14) — exactly the
/// line-mode sentence (`notice_line`) less its full stop, from the notice's
/// own fields: `{Weapon} fires at {designator}{ (FRIENDLY)} by S{slot}
/// ({percent}%, {side})` (`{Weapon} fires by S{slot} (…)` with no designator)
/// and `S{slot} held the {weapon}: {why}`, the why off `detail` — `{percent}%
/// under your {floor}%`, `{designator} shows its {side}`, `{designator} at
/// {range_m} m, outside {min} to {max}` (`under {max}` with no nearest, `past
/// {min}` with no furthest), `{rounds} rounds, keeps {keeps}`. `None` for a
/// notice missing a part, which then reads as its kind's words.
fn slot_words(notice: &Value, kind: &str, named: Option<&str>) -> Option<String> {
    let slot = notice_int(notice, "slot")?;
    let weapon = notice_str(notice, "weapon", 24)?;
    if kind == "slot_fired" {
        let percent = notice_int(notice, "percent")?;
        let aspect = notice_str(notice, "aspect", 24)?;
        let mut letters = weapon.chars();
        let gun: String = letters
            .next()
            .map(|first| first.to_uppercase().chain(letters).collect())
            .unwrap_or_default();
        let side = side_word(&aspect);
        return Some(match named {
            Some(named) => format!("{gun} fires at {named} by S{slot} ({percent}%, {side})"),
            None => format!("{gun} fires by S{slot} ({percent}%, {side})"),
        });
    }
    let detail = notice.get("detail").filter(|d| d.is_object())?;
    let designator = notice_str(notice, "designator", 8);
    let why = match notice_str(detail, "why", 24)?.as_str() {
        "odds" => format!(
            "{}% under your {}%",
            notice_int(detail, "percent")?,
            notice_int(detail, "floor")?
        ),
        "aspect" => format!(
            "{} shows its {}",
            designator?,
            side_word(&notice_str(detail, "aspect", 24)?)
        ),
        "range" => {
            let (min, max) = (notice_int(detail, "min_m")?, notice_int(detail, "max_m")?);
            let bounds = if min == 0 {
                format!("under {max}")
            } else if max == i64::from(u32::MAX) {
                format!("past {min}")
            } else {
                format!("{min} to {max}")
            };
            format!(
                "{} at {} m, outside {bounds}",
                designator?,
                notice_int(detail, "range_m")?
            )
        }
        "rounds" => format!(
            "{} rounds, keeps {}",
            notice_int(detail, "rounds")?,
            notice_int(detail, "keeps")?
        ),
        _ => return None,
    };
    Some(format!("S{slot} held the {weapon}: {why}"))
}

/// **The most new or lost contacts a catch-up names, each list** (16b cut
/// Y.4) — the arena's `MAX_CATCHUP_CONTACTS`, which decides which list the one
/// overflow count is said after (the one that is full), or that it is its own
/// clause (both are).
pub const MAX_CATCHUP_CONTACTS: usize = 12;

/// A `[x, y]` cell of a catch-up, `None` for `null` or anything malformed —
/// the web's `cellPair`.
fn catchup_cell(value: &Value, key: &str) -> Option<(i64, i64)> {
    let pair = value.get(key)?.as_array()?;
    let x = pair.first()?.as_i64()?;
    let y = pair.get(1)?.as_i64()?;
    (x >= 0 && y >= 0).then_some((x, y))
}

/// A non-negative count of a catch-up, `0` for a missing or malformed one.
fn catchup_count(value: &Value, key: &str) -> i64 {
    notice_int(value, key).unwrap_or(0).max(0)
}

/// The strings of one list key of a catch-up, each cleaned and cut to `max`,
/// a blank or non-string entry dropped, the list cut to `cap`.
fn catchup_names(value: &Value, key: &str, max: usize, cap: usize) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let text = truncate_chars(clean(entry.as_str()?).trim(), max);
                    (!text.is_empty()).then_some(text)
                })
                .take(cap)
                .collect()
        })
        .unwrap_or_default()
}

/// `none new` / `new BA, X3` / `new BA, X3 and 3 more` — the server's
/// `listed_clause`.
fn catchup_clause(word: &str, names: &[String], more: i64) -> String {
    match (names.is_empty(), more) {
        (true, 0) => format!("none {word}"),
        (true, more) => format!("{word} {more} more"),
        (false, 0) => format!("{word} {}", names.join(", ")),
        (false, more) => format!("{word} {} and {more} more", names.join(", ")),
    }
}

/// `a, b` or `a, b and 2 more`, or `none_word` — the server's `listed_or`.
fn catchup_listed(parts: &[String], separator: &str, more: i64, none_word: &str) -> String {
    match (parts.is_empty(), more) {
        (true, 0) => none_word.to_string(),
        (true, more) => format!("{more} more"),
        (false, 0) => parts.join(separator),
        (false, more) => format!("{} and {more} more", parts.join(separator)),
    }
}

/// **The catch-up report, in the notices ring** (16b cut Y.4; doc 07 §1.3
/// step 5): the web's `catchupLines` and the server's `catchup_lines`, line for
/// line, composed from the frame's `catchup` object's FIELDS — the line client
/// was sent the server's own words as one `Sys` block tagged
/// `NOTICE_SENTENCE_TAG`, which the ring leaves out.
///
/// Every string is cleaned and cut exactly as the web's decoder cuts it: the
/// callsign and each designator to 8, a section id and a weapon id to 24, the
/// machine's state and objective and every condition and power word to 16. A
/// list is cut to what the arena sends at most (12 contacts a list, 6 sections,
/// 6 weapons); a malformed entry is dropped. A `catchup` that is not an object
/// answers nothing. **Pinned against the committed
/// `web/src/lib/fixtures/cockpit/catchup_lines.json`**.
///
/// **17b cut M5.3 (D14)**: the machine's `unknown` — a hull flown by a mind
/// that is not Standard Doctrine — reads `flying on its own mind`, as the
/// server's `machine_state_words` and the web's `MACHINE_STATE_WORDS` say it,
/// rather than the opened word.
#[must_use]
pub fn catchup_words(callsign: &str, catchup: &Value) -> Vec<String> {
    if !catchup.is_object() {
        return Vec::new();
    }
    let word = |value: &Value, key: &str| notice_str(value, key, 16);
    let away_s = catchup_count(catchup, "away_s");
    let mut lines = vec![format!(
        "Back aboard {callsign} after {} away.",
        duration_words(away_s)
    )];
    lines.push(match catchup.get("machine").filter(|m| m.is_object()) {
        Some(machine) => {
            let state = word(machine, "state").unwrap_or_else(|| "?".to_string());
            let state = match state.as_str() {
                "startup" => "starting up".to_string(),
                "patrol" => "patrolling".to_string(),
                "approach" => "closing on a contact".to_string(),
                "engage" => "engaging".to_string(),
                "withdraw" => "withdrawing toward its entry zone".to_string(),
                "disabled" => "disabled".to_string(),
                "unknown" => "flying on its own mind".to_string(),
                other => other.replace('_', " "),
            };
            let order = match word(machine, "objective").as_deref() {
                None => "no order".to_string(),
                Some("hold") => "a hold order".to_string(),
                Some("advance") => "an advance order".to_string(),
                Some("engage") => "an engage order".to_string(),
                Some("withdraw") => "a withdraw order".to_string(),
                Some(other) => format!("a {} order", other.replace('_', " ")),
            };
            format!("  The machine was {state} under {order}.")
        }
        None => "  The machine had no state.".to_string(),
    });
    let cell = |(x, y): (i64, i64)| format!("({x}, {y})");
    lines.push(
        match (
            catchup_cell(catchup, "cell_then"),
            catchup_cell(catchup, "cell_now"),
        ) {
            (_, None) => "  Position: unknown.".to_string(),
            (None, Some(now)) => format!("  Position: cell unknown then, {} now.", cell(now)),
            (Some(then), Some(now)) if then == now => {
                format!("  Position: cell {}, unchanged.", cell(now))
            }
            (Some(then), Some(now)) => {
                format!("  Position: cell {} then, {} now.", cell(then), cell(now))
            }
        },
    );
    let new = catchup_names(catchup, "contacts_new", 8, MAX_CATCHUP_CONTACTS);
    let lost = catchup_names(catchup, "contacts_lost", 8, MAX_CATCHUP_CONTACTS);
    let more = catchup_count(catchup, "contacts_more");
    lines.push(if new.is_empty() && lost.is_empty() && more == 0 {
        "  Contacts: none new, none lost.".to_string()
    } else {
        // The server's `contacts_line`: the one count goes after the one full
        // list (only a full list can have overflowed); with both full it
        // cannot say whose it was and is its own clause.
        let new_full = new.len() >= MAX_CATCHUP_CONTACTS;
        let lost_full = lost.len() >= MAX_CATCHUP_CONTACTS;
        let (new_more, lost_more, unnamed) = match (more, new_full, lost_full) {
            (0, _, _) => (0, 0, 0),
            (more, true, false) => (more, 0, 0),
            (more, false, true) => (0, more, 0),
            (more, _, _) => (0, 0, more),
        };
        let clauses = format!(
            "{}; {}",
            catchup_clause("new", &new, new_more),
            catchup_clause("lost", &lost, lost_more)
        );
        if unnamed == 0 {
            format!("  Contacts: {clauses}.")
        } else {
            format!("  Contacts: {clauses}; {unnamed} more not named.")
        }
    });
    let sections: Vec<String> = catchup
        .get("sections")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|section| {
                    let id = notice_str(section, "id", 24)?;
                    let then = section.get("then").filter(|t| t.is_object())?;
                    let now = section.get("now").filter(|n| n.is_object())?;
                    let (then_word, now_word) = (word(then, "condition"), word(now, "condition"));
                    if then_word != now_word {
                        return Some(format!(
                            "{id} {} then, {} now",
                            then_word.unwrap_or_else(|| "?".to_string()),
                            now_word.unwrap_or_else(|| "?".to_string())
                        ));
                    }
                    let figure = |side: &Value, key: &str| notice_int(side, key).unwrap_or(0);
                    let mut clauses: Vec<String> = Vec::new();
                    let (armour_then, armour_now) = (figure(then, "armour"), figure(now, "armour"));
                    if armour_now < armour_then {
                        clauses.push(format!("armour {armour_then} then, {armour_now} now"));
                    }
                    let (inside_then, inside_now) =
                        (figure(then, "internals"), figure(now, "internals"));
                    if inside_now < inside_then {
                        clauses.push(format!("internals {inside_then} then, {inside_now} now"));
                    }
                    Some(if clauses.is_empty() {
                        id
                    } else {
                        format!("{id} {}", clauses.join(", "))
                    })
                })
                .take(MAX_CATCHUP_SECTIONS)
                .collect()
        })
        .unwrap_or_default();
    lines.push(format!(
        "  Sections: {}.",
        catchup_listed(
            &sections,
            "; ",
            catchup_count(catchup, "sections_more"),
            "unchanged"
        )
    ));
    let rounds: Vec<String> = catchup
        .get("rounds_spent")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|row| {
                    let weapon = notice_str(row, "weapon", 24)?;
                    let spent = notice_int(row, "rounds")?;
                    Some(format!("{weapon} {spent}"))
                })
                .take(MAX_CATCHUP_WEAPONS)
                .collect()
        })
        .unwrap_or_default();
    lines.push(format!(
        "  Rounds spent: {}.",
        catchup_listed(&rounds, ", ", catchup_count(catchup, "rounds_more"), "none")
    ));
    lines.push(format!(
        "  Power {}.",
        word(catchup, "power_now").unwrap_or_else(|| "?".to_string())
    ));
    if catchup.get("wreck").and_then(Value::as_bool) == Some(true) {
        lines.push("  Your tank is a wreck.".to_string());
    }
    lines
}

/// The most changed sections a catch-up names (the arena's
/// `MAX_CATCHUP_SECTIONS`).
pub const MAX_CATCHUP_SECTIONS: usize = 6;
/// The most weapons a catch-up names rounds spent for (the arena's
/// `MAX_CATCHUP_WEAPONS`).
pub const MAX_CATCHUP_WEAPONS: usize = 6;

/// **A length in words** — the server's `duration_words` over whole seconds:
/// `4 m 32 s`, `12 s`, `1 h 2 m 3 s`, `0 s` for nothing or less.
fn duration_words(secs: i64) -> String {
    let secs = secs.max(0);
    let (hours, minutes, seconds) = (secs / 3_600, secs % 3_600 / 60, secs % 60);
    if hours > 0 {
        format!("{hours} h {minutes} m {seconds} s")
    } else if minutes > 0 {
        format!("{minutes} m {seconds} s")
    } else {
        format!("{seconds} s")
    }
}

/// What the notices pane says while seated with nothing rung yet (16b cut B.3).
pub const NOTHING_YET: &str = "Nothing yet.";

/// **The notices pane** (16b cut B.3, `B-64`, Q20): the store's ring —
/// [`PanelStore::cockpit_notices`], each seated frame's notices through
/// [`notice_words`] and, while seated, the pilot's own replies — oldest first.
///
/// `data` is only asked whether the viewer is seated (the cleared frame says
/// [`NOT_IN_A_COCKPIT`]); the lines are the ring's. With the pane's size known
/// it answers only the TAIL that fits once wrapped at the pane's width: the
/// pane is not selectable and does not follow a scroll, and the newest line —
/// the answer to what the pilot just typed — is the one they are waiting for,
/// so it is the oldest lines that go. With no size (the input paths) it answers
/// the whole ring.
#[must_use]
pub fn render_cockpit_notices_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if !seated(data) {
        return PaneRows::one(NOT_IN_A_COCKPIT);
    }
    let ring = ctx.store.cockpit_notices();
    if ring.is_empty() {
        return PaneRows::one(NOTHING_YET);
    }
    let mut kept: Vec<PaneRow> = Vec::new();
    let mut used = 0usize;
    for line in ring.iter().rev() {
        let row = Line::from(Span::raw(line.clone()));
        if let Some((w, h)) = ctx.pane {
            let cost = wrap_spans(&row.spans, usize::from(w)).len();
            if !kept.is_empty() && used + cost > usize::from(h) {
                break;
            }
            used += cost;
        }
        kept.push(PaneRow::plain(row));
    }
    kept.reverse();
    PaneRows { rows: kept }
}

// ---------------------------------------------------------------------------
// Chrome.
// ---------------------------------------------------------------------------

/// The vitals strip: every `.vitals` frame as `[ label ▰▰▰▱ 6/10 ]` gauges,
/// tracks as boxes with the level named, readouts as text. Empty when no
/// frame is held.
///
/// Ordering (look-back LB-8). Across FEEDS: the viewer's own — the frame whose
/// subject is `own`, the character the hello named — then any feed carrying a
/// `Dominant` vital, then the order given. Within a feed: `Dominant`, `Track`,
/// `Pool`, `Readout`. A tactics unit publishes up to sixteen pool cells (~400
/// columns) and sorts before `wod.vitals` alphabetically, so without this the
/// pilot's own health sat off the right edge of a strip that is one row and
/// does not wrap.
///
/// The strip is CUT to `width` columns and says so: the cells that fit, then a
/// dim `+N more`. `width` 0 means unbounded (a caller that measures itself).
#[must_use]
pub fn vitals_line(
    frames: &[(mush_proto::SmolStr, VitalsFrame)],
    utf8: bool,
    width: usize,
    own: Option<&str>,
) -> Line<'static> {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.sort_by_key(|&i| {
        let frame = &frames[i].1;
        let is_own = own.is_some_and(|c| frame.subject.name == c);
        let has_dominant = frame
            .vitals
            .iter()
            .any(|v| matches!(v.role, VitalRole::Dominant));
        (!is_own, !has_dominant, i)
    });
    let mut cells: Vec<Span<'static>> = Vec::new();
    for i in order {
        let frame = &frames[i].1;
        let mut vitals: Vec<&mush_proto::Vital> = frame.vitals.iter().collect();
        vitals.sort_by_key(|v| match v.role {
            VitalRole::Dominant => 0,
            VitalRole::Track => 1,
            VitalRole::Pool => 2,
            VitalRole::Readout => 3,
        });
        for v in vitals {
            let style = match v.state {
                VitalState::Ok => Style::default(),
                VitalState::Warn => Style::default().fg(Color::Indexed(3)),
                VitalState::Crit => Style::default().fg(Color::Indexed(1)),
                VitalState::Down => Style::default()
                    .fg(Color::Indexed(1))
                    .add_modifier(Modifier::BOLD),
            };
            let label = clean(&v.label);
            let text = match &v.value {
                VitalValue::Bounded { current, min, max } => {
                    let span = (max - min).max(1);
                    let cells = 8usize;
                    let filled = usize::try_from(
                        ((current - min).clamp(0, span) * cells as i64 + span / 2) / span,
                    )
                    .unwrap_or(0);
                    let (on, off) = if utf8 {
                        ('\u{25b0}', '\u{25b1}')
                    } else {
                        ('#', '-')
                    };
                    let bar: String = (0..cells)
                        .map(|i| if i < filled { on } else { off })
                        .collect();
                    format!("{label} {bar} {current}/{max}")
                }
                VitalValue::Track { boxes, level } => {
                    let empty = if utf8 { '\u{25a1}' } else { '_' };
                    let cells: String = boxes
                        .iter()
                        .map(|b| match b.as_deref() {
                            None => empty,
                            Some("bashing") => '/',
                            Some("lethal") => 'X',
                            Some("aggravated") => '*',
                            Some(other) => {
                                other.chars().next().map_or('?', |c| c.to_ascii_uppercase())
                            }
                        })
                        .collect();
                    match level {
                        Some(l) => format!("{label} {cells} {}", clean(l)),
                        None => format!("{label} {cells}"),
                    }
                }
                VitalValue::Text { text } => format!("{label} {}", clean(text)),
            };
            cells.push(Span::styled(format!("[ {text} ]"), style));
        }
    }
    Line::from(fit_cells(cells, width))
}

/// Lays `cells` out separated by two spaces, cut to `width` columns with a
/// dim `+N more` for what did not fit. `width` 0 is unbounded.
fn fit_cells(cells: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let join = |k: usize| -> Vec<Span<'static>> {
        let mut out: Vec<Span<'static>> = Vec::new();
        for c in cells.iter().take(k) {
            if !out.is_empty() {
                out.push(Span::raw("  "));
            }
            out.push(c.clone());
        }
        out
    };
    let used = |k: usize| -> usize {
        if k == 0 {
            0
        } else {
            cells[..k].iter().map(Span::width).sum::<usize>() + 2 * (k - 1)
        }
    };
    if cells.is_empty() || width == 0 || used(cells.len()) <= width {
        return join(cells.len());
    }
    // The marker's own width counts against the budget, and it grows with the
    // number elided, so the largest prefix that fits is searched from the top.
    let mut k = cells.len();
    let marker = |m: usize| format!("+{m} more");
    while k > 0 {
        // `+ 2` is the separator before the marker; inside this loop there is
        // always at least one cell to separate it from.
        if used(k) + 2 + marker(cells.len() - k).chars().count() <= width {
            break;
        }
        k -= 1;
    }
    let mut out = join(k);
    if k > 0 {
        out.push(Span::raw("  "));
    }
    out.push(dim_span_owned(&truncate_chars(
        &marker(cells.len() - k),
        width,
    )));
    out
}

/// The status line: room name and id, zone, weather state, the IC clock,
/// and the exits with their nav keys (`n·north`).
#[must_use]
pub fn status_line(
    room: Option<&RoomView>,
    exit_keys: &[(char, String)],
    utf8: bool,
    theme: &AnsiTheme,
    depth: ColorDepth,
) -> Line<'static> {
    let Some(room) = room else {
        return Line::from(dim_span_owned("(no room yet)"));
    };
    let sep = if utf8 { " \u{00b7} " } else { " | " };
    let dot = if utf8 { "\u{00b7}" } else { ":" };
    let room_style = to_ratatui_style(theme.style_for(Some("room")), depth);
    let exit_style = to_ratatui_style(theme.style_for(Some("exit")), depth);
    let mut spans = vec![
        Span::styled(clean(&room.name), room_style),
        Span::raw(" "),
        dim_span_owned(&room.id),
    ];
    if let Some(zone) = &room.zone {
        spans.push(Span::raw(sep.to_string()));
        spans.push(Span::raw(clean(&zone.name)));
    }
    if let Some(state) = room
        .ambient
        .get("weather")
        .and_then(|w| w.get("state"))
        .and_then(Value::as_str)
    {
        spans.push(Span::raw(sep.to_string()));
        spans.push(Span::raw(clean(state)));
    }
    if let Some(ic) = room.ambient.get("ic_time") {
        let hour = ic.get("hour").and_then(Value::as_u64);
        let minute = ic.get("minute").and_then(Value::as_u64);
        if let (Some(h), Some(m)) = (hour, minute) {
            spans.push(Span::raw(sep.to_string()));
            spans.push(Span::raw(format!("IC {h:02}:{m:02}")));
        }
    }
    if !room.exits.is_empty() {
        spans.push(Span::raw(sep.to_string()));
        for (i, exit) in room.exits.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            let name = clean(&exit.name);
            match exit_keys
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(&exit.name))
            {
                Some((k, _)) => {
                    spans.push(Span::styled(k.to_string(), exit_style));
                    spans.push(dim_span_owned(dot));
                    spans.push(Span::raw(name));
                }
                None => spans.push(Span::styled(name, exit_style)),
            }
        }
    }
    Line::from(spans)
}

/// The tab strip: `rustmush  1 scene  2 sheet  3 mail 2 …` with the active
/// screen reversed and unread badges, then `right` flush right in `width`.
///
/// Ten named tabs need 110 columns and nine 98, so at the 120-column floor
/// (16b cut AB.1) every tab is named; the right-hand block is dropped beside
/// ten tabs until 128 columns. When unread badges push the tabs alone past the
/// width the strip elides: every INACTIVE tab drops its name and
/// keeps its digit (and its unread badge, which is the only place unread is
/// shown at all), and the active tab keeps its name, since answering "where
/// am I" is what the strip is for. The right-hand block's own drop rule is
/// unchanged, so a strip that fits looks exactly as it did.
#[must_use]
pub fn tab_strip(
    screens: &[(u8, &str, usize)],
    active: u8,
    right: &str,
    width: usize,
) -> Line<'static> {
    // Doc 19 cut A.3b: a lettered screen's id is its letter's byte, and the
    // strip prints the letter (`Alt+c`); a digit prints as itself.
    let hotkey = |digit: u8| -> String {
        if digit <= 9 {
            digit.to_string()
        } else {
            char::from(digit).to_string()
        }
    };
    let label_of = |digit: u8, name: &str, badge: usize, named: bool| {
        let key = hotkey(digit);
        match (named, badge > 0) {
            (true, true) => format!(" {key} {name} {badge} "),
            (true, false) => format!(" {key} {name} "),
            (false, true) => format!(" {key} {badge} "),
            (false, false) => format!(" {key} "),
        }
    };
    let full: usize = "rustmush".len()
        + screens
            .iter()
            .map(|&(d, n, b)| label_of(d, n, b, true).chars().count() + 1)
            .sum::<usize>();
    let named = full <= width;
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        "rustmush".to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    let mut used = "rustmush".len();
    for &(digit, name, badge) in screens {
        let label = label_of(digit, name, badge, named || digit == active);
        used += label.chars().count() + 1;
        spans.push(Span::raw(" "));
        let style = if digit == active {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        spans.push(Span::styled(label, style));
    }
    let right = clean(right);
    let rw = right.chars().count();
    if rw > 0 && used + 1 + rw <= width {
        spans.push(Span::raw(" ".repeat(width - used - rw)));
        spans.push(dim_span_owned(&right));
    }
    Line::from(spans)
}

/// **The sentence painted below the floor** (16b cut AB.1, 16c `Q-AB1`):
/// the floor ([`FLOOR_COLS`](super::screen::FLOOR_COLS)) and the terminal's
/// own width, `cols`, and the two ways back to a terminal that works —
/// Ctrl-], which detaches from either mode, and `@notui`, which nav mode opens
/// the box for below the floor. For a client that reported no size, `cols` is
/// the 80 the session substitutes.
#[must_use]
pub fn floor_line(cols: u16) -> String {
    format!(
        "The TUI needs at least {} columns and this terminal is {cols} wide: \
         make it larger, or press Ctrl-] or type @notui for line mode.",
        super::screen::FLOOR_COLS
    )
}

/// The legend: the screen's bindings in nav mode, the editing keys in
/// input mode.
///
/// **16b cut AB.1**: `width` is the row it is painted on (`0` is unbounded).
/// In nav mode, when the whole legend — the bindings in order, `Enter type`,
/// `? help` — would run past it, whole entries are kept in order while they
/// fit and `? help` is ALWAYS the last: the help overlay names every key the
/// cut left off, and the input box's own title already says `Enter to type`.
/// So the cockpit's twelve bindings (148 characters whole) read ten and
/// `? help` at the 120-column floor instead of stopping mid-word. Input mode
/// is four short entries and is never cut.
#[must_use]
pub fn legend_line(
    input_mode: bool,
    bindings: &[(char, &str)],
    utf8: bool,
    width: usize,
) -> Line<'static> {
    let sep = if utf8 { " \u{00b7} " } else { " | " };
    let key_style = Style::default().add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let push = |spans: &mut Vec<Span<'static>>, key: &str, label: &str| {
        if !spans.is_empty() {
            spans.push(dim_span_owned(sep));
        }
        spans.push(Span::styled(key.to_string(), key_style));
        spans.push(Span::raw(format!(" {label}")));
    };
    if input_mode {
        let (up, tab) = if utf8 {
            ("\u{2191}", "\u{21e5}")
        } else {
            ("Up", "Tab")
        };
        push(&mut spans, up, "history");
        push(&mut spans, tab, "complete");
        push(&mut spans, "^P", "mode");
        push(&mut spans, "Esc", "nav");
        return Line::from(spans);
    }
    let mut entries: Vec<(String, &str)> = bindings
        .iter()
        .map(|(k, label)| (k.to_string(), *label))
        .collect();
    entries.push(("Enter".to_string(), "type"));
    let help = ("?".to_string(), "help");
    // An entry is its key, a space and its label; every entry after the first
    // costs the separator too.
    let sep_w = sep.chars().count();
    let cost = |(key, label): &(String, &str)| key.chars().count() + 1 + label.chars().count();
    let whole = entries.iter().map(|e| cost(e) + sep_w).sum::<usize>() + cost(&help);
    if width > 0 && whole > width {
        // Keep whole entries while they and the `? help` after them fit.
        let mut used = cost(&help);
        let mut kept = 0;
        for entry in &entries {
            let next = used + cost(entry) + sep_w;
            if next > width {
                break;
            }
            used = next;
            kept += 1;
        }
        entries.truncate(kept);
    }
    for (key, label) in &entries {
        push(&mut spans, key, label);
    }
    push(&mut spans, &help.0, help.1);
    Line::from(spans)
}

// ---- doc 19 cut A.3b: the sky chart screen ---------------------------------------

/// The `sky.chart` feed (`tacsim::port::chart_panel::SKY_CHART_FEED`).
pub const CHART_FEED: &str = "sky.chart";

fn chart_num(data: &Value, key: &str) -> usize {
    data.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

fn chart_cell(v: Option<&Value>) -> Option<(usize, usize)> {
    let arr = v?.as_array()?;
    Some((
        usize::try_from(arr.first()?.as_u64()?).ok()?,
        usize::try_from(arr.get(1)?.as_u64()?).ok()?,
    ))
}

/// **The sky chart pane** (doc 19 cut A.3b; doc 19 §5): the `sky.chart` frame's
/// flat render — the server's own grid, the one `+chart/map` prints — windowed
/// to the pane around the viewer's port (the whole 60-by-28 grid fits a
/// 120-column terminal's Main pane), its title above and its legend below. No
/// frame yet: a sentence; a frame with no grid (no lane here): the sentence the
/// server sent as its title.
pub fn render_chart_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if data.is_null() {
        return PaneRows::one("(no chart yet: r asks the server for the sky)");
    }
    let title = data
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("The sky");
    let width = chart_num(data, "width");
    let height = chart_num(data, "height");
    if width == 0 || height == 0 {
        return PaneRows::one(&clean(title));
    }
    let empty = Vec::new();
    let runs = data.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    let (pane_w, pane_h) = match ctx.pane {
        Some((w, h)) => (usize::from(w).max(8), usize::from(h).max(4)),
        None => (width, height + 2),
    };
    // The title is one clipped row; the legend wraps at entry boundaries over
    // as many rows as it needs; the grid takes what is left, windowed around
    // the viewer's port.
    let clip = |text: &str| -> String { clean(text).chars().take(pane_w).collect() };
    let entries: Vec<String> = data
        .get("legend")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|pair| {
                    Some(format!(
                        "{} {}",
                        pair.get(0)?.as_str()?,
                        pair.get(1)?.as_str()?
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut legend_rows: Vec<String> = Vec::new();
    for entry in entries {
        let entry = clean(&entry);
        match legend_rows.last_mut() {
            Some(last) if last.chars().count() + 2 + entry.chars().count() <= pane_w => {
                last.push_str("  ");
                last.push_str(&entry);
            }
            _ => legend_rows.push(clip(&entry)),
        }
    }
    let grid_h = pane_h.saturating_sub(1 + legend_rows.len()).max(1);
    let you = chart_cell(data.get("you"));
    let (x0, y0, w, h) = mush_ansi::viewport(width, height, you, pane_w, grid_h);
    let grid = mush_ansi::window_rows(runs, width, height, x0, y0, w, h);
    let mut rows = vec![PaneRow::plain(Line::from(Span::styled(
        clip(title),
        Style::default().add_modifier(Modifier::BOLD),
    )))];
    // North up: the grid's row 0 is the south edge.
    for row in grid.iter().rev() {
        rows.push(PaneRow::plain(Line::from(row.iter().collect::<String>())));
    }
    for line in legend_rows {
        rows.push(PaneRow::plain(Line::from(dim_span_owned(&line))));
    }
    PaneRows { rows }
}

/// **The chart's tables pane** (doc 19 cut A.3b): the lines `+chart` and
/// `+chart <port>` print — the sky table and the passages now, then each
/// destination's window — as the server sent them in `chart.lines`, so this
/// client composes no figure of its own.
pub fn render_chart_table_rows(data: &Value, ctx: &RenderCtx<'_>) -> PaneRows {
    if data.is_null() {
        return PaneRows::one("(no chart yet)");
    }
    let Some(lines) = data.pointer("/chart/lines") else {
        return PaneRows::one("(the server sent no tables)");
    };
    let system = ctx.tag("system");
    let mut rows = Vec::new();
    // The window first — the trader's question — then the sky table, which the
    // picture beside it already draws.
    for (key, head) in [
        ("window", "the window"),
        ("sky", "the sky and the passages now"),
    ] {
        let Some(list) = lines.get(key).and_then(Value::as_array) else {
            continue;
        };
        if list.is_empty() {
            continue;
        }
        rows.push(PaneRow::plain(Line::from(Span::styled(
            head.to_owned(),
            system,
        ))));
        for line in list.iter().filter_map(Value::as_str) {
            rows.push(PaneRow::plain(Line::from(clean(line))));
        }
    }
    if rows.is_empty() {
        return PaneRows::one("(the server sent no tables)");
    }
    PaneRows { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ClientKind, ExitRef, ObjRef};

    fn caps(depth: ColorDepth) -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: depth,
            width: 80,
            height: 24,
            utf8: true,
            structured: false,
        }
    }

    fn body_text(e: &Entry) -> String {
        e.body.iter().map(|s| s.content.as_ref()).collect()
    }

    fn gutter_text(e: &Entry) -> String {
        e.gutter
            .as_ref()
            .map(|s| s.content.to_string())
            .unwrap_or_default()
    }

    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn say_puts_the_speaker_in_the_gutter() {
        let theme = AnsiTheme::classic();
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#10", "Nick")),
            text: RichText::plain("hi there"),
        };
        let entries = render_event(&ev, &caps(ColorDepth::Ansi16), &theme, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(gutter_text(&entries[0]), "Nick");
        assert_eq!(body_text(&entries[0]), "\"hi there\"");
        assert_eq!(
            entries[0].gutter.as_ref().unwrap().style.fg,
            Some(Color::Indexed(6))
        );
    }

    #[test]
    fn roll_narration_takes_the_roll_gutter_and_sys_takes_the_clock() {
        let theme = AnsiTheme::classic();
        let text = RichText(vec![
            MushSpan::tagged("roll", "roll"),
            MushSpan::plain("  "),
            MushSpan::tagged("player", "Nick"),
            MushSpan::plain("  strength+brawl  8 6 4 = 1 success  diff 6"),
        ]);
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: Some(ObjRef::new("#10", "Nick")),
            text,
        };
        let entries = render_event(&ev, &caps(ColorDepth::Ansi16), &theme, 3661);
        assert_eq!(gutter_text(&entries[0]), "\u{2684} roll");
        assert_eq!(
            body_text(&entries[0]),
            "Nick  strength+brawl  8 6 4 = 1 success  diff 6"
        );
        let mut ascii = caps(ColorDepth::None);
        ascii.utf8 = false;
        let entries = render_event(&ev, &ascii, &theme, 3661);
        assert_eq!(gutter_text(&entries[0]), "roll");
        let sys = render_event(
            &OutputEvent::Sys(RichText::plain("one\ntwo")),
            &caps(ColorDepth::None),
            &theme,
            3661,
        );
        assert_eq!(sys.len(), 2);
        assert_eq!(gutter_text(&sys[0]), "01:01");
        assert!(sys[1].gutter.is_none(), "a continuation line has no gutter");
    }

    #[test]
    fn room_renders_the_facts_and_channel_the_bracket() {
        let theme = AnsiTheme::classic();
        let mut view = RoomView {
            name: "Hall".into(),
            id: "#2".into(),
            desc: RichText::plain("A hall."),
            contents: vec![ObjRef::new("#7", "ball")],
            exits: vec![ExitRef {
                name: "north".into(),
                aliases: vec!["n".into()],
            }],
            zone: None,
            ambient: Default::default(),
        };
        view.ambient.insert(
            "weather".into(),
            serde_json::json!({ "state": "rain", "line": "Overhead: rain." }),
        );
        let entries = render_event(&OutputEvent::Room(view), &caps(ColorDepth::None), &theme, 0);
        let texts: Vec<String> = entries.iter().map(body_text).collect();
        assert_eq!(texts[0], "Hall #2");
        assert_eq!(texts[2], "Here     ball");
        assert_eq!(texts[3], "Exits    north");
        assert_eq!(texts[4], "Weather  Overhead: rain.");
        let ch = OutputEvent::Channel {
            channel: "pub".into(),
            seq: 1,
            actor: ObjRef::new("#5", "Zoe"),
            title: None,
            kind: NarrativeKind::Say,
            text: RichText::plain("hello"),
            ts: 0,
        };
        let entries = render_event(&ch, &caps(ColorDepth::Ansi16), &theme, 0);
        assert_eq!(gutter_text(&entries[0]), "[pub]");
        assert_eq!(body_text(&entries[0]), "Zoe: hello");
    }

    /// Look-back I3-C36 (the TUI half): `EntryAcc::write` splits an embedded
    /// newline into a fresh ENTRY, and a scrollback entry with no gutter reads
    /// as a continuation of the fact above it — so a builder-named object
    /// could forge a second `Here` or `Exits` row. Room fields are flattened
    /// to one line each, exactly as line mode flattens them.
    #[test]
    fn control_bytes_in_room_fields_cannot_break_the_fact_gutter() {
        let theme = AnsiTheme::classic();
        let mut view = RoomView {
            name: "Hall\nExits    forged".into(),
            id: "#2".into(),
            desc: RichText::plain("A hall."),
            contents: vec![ObjRef::new("#7", "ball\nHere     forged")],
            exits: vec![ExitRef {
                name: "north\nWeather  forged".into(),
                aliases: vec!["n".into()],
            }],
            zone: None,
            ambient: Default::default(),
        };
        view.ambient.insert(
            "weather".into(),
            serde_json::json!({ "state": "rain", "line": "Rain.\nHere     forged again" }),
        );
        let entries = render_event(&OutputEvent::Room(view), &caps(ColorDepth::None), &theme, 0);
        let texts: Vec<String> = entries.iter().map(body_text).collect();
        for label in ["Here", "Exits", "Weather"] {
            assert_eq!(
                texts.iter().filter(|t| t.starts_with(label)).count(),
                1,
                "exactly one {label} row: {texts:?}"
            );
        }
        assert_eq!(texts[0], "Hall Exits    forged #2", "one heading entry");
        assert_eq!(texts[2], "Here     ball Here     forged");
        assert_eq!(texts[3], "Exits    north Weather  forged");
        assert_eq!(texts[4], "Weather  Rain. Here     forged again");
    }

    #[test]
    fn wrap_spans_breaks_at_spaces_keeps_styles_and_hard_splits_long_words() {
        let red = Style::default().fg(Color::Indexed(1));
        let spans = vec![
            Span::styled("red words", red),
            Span::raw(" then plain text here"),
        ];
        let rows = wrap_spans(&spans, 10);
        let texts: Vec<String> = rows
            .iter()
            .map(|r| r.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(texts, vec!["red words", "then plain", "text here"]);
        assert_eq!(rows[0][0].style, red);
        let long = vec![Span::raw("supercalifragilistic")];
        let rows = wrap_spans(&long, 8);
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .all(|r| r.iter().map(|s| s.content.chars().count()).sum::<usize>() <= 8)
        );
        assert_eq!(wrap_spans(&[], 5).len(), 1);
        assert_eq!(wrap_spans(&long, 0).len(), 1);
    }

    #[test]
    fn who_rail_marks_here_and_counts() {
        let theme = AnsiTheme::classic();
        let entries = vec![
            WhoEntry {
                name: "Alice".into(),
                alias: None,
                idle_s: 5,
                conn_s: 100,
                location: Some("Hall".into()),
                here: true,
                doing: None,
            },
            WhoEntry {
                name: "Bob".into(),
                alias: None,
                idle_s: 65,
                conn_s: 200,
                location: None,
                here: false,
                doing: None,
            },
        ];
        let lines = render_who_rail(&entries, &theme, ColorDepth::Ansi16, true);
        let text: Vec<String> = lines.iter().map(line_text).collect();
        assert!(text[1].starts_with("\u{2022} Alice"));
        assert!(text[2].starts_with("  Bob"));
        assert_eq!(text[3], "2 online, 1 here");
    }

    #[test]
    fn mail_rows_prefill_and_channels_rows_key_by_name() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let data = serde_json::json!({
            "folder": "inbox",
            "entries": [
                { "index": 1, "status": "unread", "from": { "name": "Zoe" }, "subject": "Hi" },
                { "index": 2, "status": "read", "from": { "name": "Al" }, "subject": "Later" },
            ],
        });
        let rows = render_mail_rows(&data, &ctx);
        assert_eq!(rows.rows.len(), 3);
        assert!(!rows.rows[0].selectable);
        assert_eq!(rows.rows[1].prefill.as_deref(), Some("@mail 1"));
        assert_eq!(rows.rows[1].row_key.as_deref(), Some("1"));
        let chans = serde_json::json!({ "channels": [
            { "name": "Public", "alias": "pub", "unread": 3, "gagged": false },
            { "name": "Staff", "alias": "", "unread": 0, "gagged": true },
            { "name": "Radio", "alias": "rad", "unread": 0, "gagged": false, "ic": true },
        ]});
        let rows = render_channels_rows(&chans, &ctx, false);
        assert_eq!(rows.rows.len(), 2, "the OOC pane never lists an IC channel");
        assert_eq!(rows.rows[0].prefill.as_deref(), Some("pub "));
        assert_eq!(rows.rows[0].row_key.as_deref(), Some("Public"));
        assert_eq!(rows.rows[1].prefill.as_deref(), Some("Staff "));
        let ic_rows = render_channels_rows(&chans, &ctx, true);
        assert_eq!(ic_rows.rows.len(), 1, "the IC pane lists only IC channels");
        assert_eq!(ic_rows.rows[0].row_key.as_deref(), Some("Radio"));
        let none = render_channels_rows(&serde_json::json!({ "channels": [] }), &ctx, true);
        assert_eq!(line_text(&none.rows[0].line), "(no IC channels)");
        let actions = serde_json::json!([{ "offer": "secret-token", "presentation": { "verb": "brace", "label": "Brace", "help": "Hold." } }]);
        let rows = render_actions_rows(&actions, &ctx);
        assert_eq!(rows.rows[0].prefill.as_deref(), Some("+act brace"));
        let all: String = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        assert!(!all.contains("secret"));
        mush_contract_tests::assert_projection_text_clean(
            "TUI actions pane",
            &all,
            &["secret-token", "scene_turn", "private-definition-digest"],
        );
    }

    /// I9: the generic view renderer walks a list schema into a header plus
    /// one row per record (selectable, prefilling the schema's `actionKey`
    /// line), and a detail schema into label/value rows; a malformed or
    /// unknown shape degrades, never panics.
    #[test]
    fn view_rows_render_list_and_detail_shapes() {
        let theme = AnsiTheme::default();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::TrueColor,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let list = serde_json::json!({
            "schema": { "id": "jobs", "version": 1, "shape": "list",
                "columns": [ { "key": "id", "label": "#", "align": "right" },
                             { "key": "title", "label": "Title" } ],
                "actionKey": "_open" },
            "rows": [ { "id": 3, "title": "Docks", "_open": "@job 3" },
                      { "id": 4, "title": "Lamp" } ]
        });
        let rows = render_view_rows(&list, &ctx);
        let t: Vec<String> = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        assert_eq!(t[0], "#  Title");
        assert_eq!(t[1], "3  Docks");
        assert_eq!(t[2], "4  Lamp ");
        assert_eq!(rows.rows[1].prefill.as_deref(), Some("@job 3"));
        assert!(rows.rows[1].selectable);
        assert!(
            !rows.rows[2].selectable,
            "a row with no action line is plain"
        );

        let detail = serde_json::json!({
            "schema": { "id": "job", "version": 1, "shape": "detail",
                "fields": [ { "key": "title", "label": "Title" },
                            { "key": "n", "label": "Count", "kind": "number" } ] },
            "record": { "title": "Docks", "n": 2 }
        });
        let rows = render_view_rows(&detail, &ctx);
        let t: Vec<String> = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        assert_eq!(t, vec!["Title: Docks".to_string(), "Count: 2".to_string()]);

        // I9-C38: a `form` is the detail shape by another name — same rows.
        let form = serde_json::json!({
            "schema": { "id": "job", "version": 1, "shape": "form",
                "fields": [ { "key": "title", "label": "Title" },
                            { "key": "n", "label": "Count", "kind": "number" } ] },
            "record": { "title": "Docks", "n": 2 }
        });
        let form_rows: Vec<String> = render_view_rows(&form, &ctx)
            .rows
            .iter()
            .map(|r| line_text(&r.line))
            .collect();
        assert_eq!(form_rows, t, "a form renders as its detail twin");

        // I9-C6: a `record` that is present but unusable is named, not shown
        // as a screen of blank fields indistinguishable from an empty one.
        let junk = serde_json::json!({
            "schema": { "shape": "detail", "fields": [ { "key": "title" } ] },
            "record": "junk"
        });
        assert_eq!(
            line_text(&render_view_rows(&junk, &ctx).rows[0].line),
            "(unreadable record)"
        );
        let no_record = serde_json::json!({
            "schema": { "shape": "detail", "fields": [ { "key": "title", "label": "Title" } ] },
            "record": serde_json::Value::Null
        });
        assert_eq!(
            line_text(&render_view_rows(&no_record, &ctx).rows[0].line),
            "Title: ",
            "a null record is the ordinary nothing-selected, not a fault"
        );

        // I9-C38: the three `(empty view)` return sites, none of them asserted
        // before — a detail with no `fields` key, one with an empty `fields`,
        // and a list whose schema names no columns.
        for empty in [
            serde_json::json!({ "schema": { "shape": "detail" } }),
            serde_json::json!({ "schema": { "shape": "detail", "fields": [] } }),
            serde_json::json!({ "schema": { "shape": "list", "columns": [] } }),
        ] {
            assert_eq!(
                line_text(&render_view_rows(&empty, &ctx).rows[0].line),
                "(empty view)",
                "{empty}"
            );
        }

        assert_eq!(
            line_text(&render_view_rows(&serde_json::json!({}), &ctx).rows[0].line),
            "(no view)"
        );
        assert_eq!(
            line_text(
                &render_view_rows(&serde_json::json!({ "schema": { "shape": "grid" } }), &ctx).rows
                    [0]
                .line
            ),
            "(unknown view shape)"
        );
    }

    /// Look-back LB-17: the generic-view caps diverged — the web bounded rows
    /// at 256 and fields at 32, this side bounded neither, so from the 257th
    /// row the "same columns, same rows" promise failed in silence. Both
    /// sides carry the same pair now, and a list or detail this renderer cut
    /// itself says so exactly as a server-truncated one does.
    #[test]
    fn a_generic_view_cut_by_the_row_or_field_cap_says_so() {
        let theme = AnsiTheme::default();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::None,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let rows: Vec<Value> = (0..300)
            .map(|i| serde_json::json!({ "id": i, "title": format!("row {i}") }))
            .collect();
        let list = serde_json::json!({
            "schema": { "shape": "list",
                "columns": [ { "key": "id", "label": "#" }, { "key": "title", "label": "Title" } ] },
            "rows": rows
        });
        let painted = render_view_rows(&list, &ctx);
        let text: Vec<String> = painted.rows.iter().map(|r| line_text(&r.line)).collect();
        // Header + VIEW_MAX_ROWS rows + the marker.
        assert_eq!(painted.rows.len(), VIEW_MAX_ROWS + 2);
        assert_eq!(
            text.last().map(String::as_str),
            Some("(more rows than shown)")
        );
        assert!(
            text.iter().any(|l| l.starts_with("255")),
            "{:?}",
            &text[..3]
        );
        assert!(!text.iter().any(|l| l.starts_with("256")), "cut at the cap");

        let fields: Vec<Value> = (0..40)
            .map(|i| serde_json::json!({ "key": format!("k{i}"), "label": format!("F{i}") }))
            .collect();
        let detail = serde_json::json!({
            "schema": { "shape": "detail", "fields": fields },
            "record": { "k0": "a" }
        });
        let painted = render_view_rows(&detail, &ctx);
        assert_eq!(painted.rows.len(), VIEW_MAX_FIELDS + 1);
        assert_eq!(
            line_text(&painted.rows[VIEW_MAX_FIELDS].line),
            "(more fields than shown)"
        );
    }

    /// I9 (verify C, P): the generic renderer reproduces the information a
    /// legal-actions list carries — the label shown and the `+act <verb>`
    /// line each row runs — so an actions-shaped feed needs no bespoke
    /// renderer; and a row field the schema does not name (an opaque offer
    /// token) never reaches the render.
    #[test]
    fn view_rows_reproduce_actions_and_never_leak_undeclared_fields() {
        let theme = AnsiTheme::default();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::TrueColor,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let view = serde_json::json!({
            "schema": { "id": "actions", "version": 1, "shape": "list",
                "columns": [ { "key": "label", "label": "Action" } ], "actionKey": "act" },
            "rows": [ { "label": "Brace", "act": "+act brace", "_offer": "secret-token" },
                      { "label": "Aim", "act": "+act aim", "_offer": "another-secret" } ]
        });
        let rows = render_view_rows(&view, &ctx);
        assert_eq!(rows.rows.len(), 3, "a header plus two rows");
        assert_eq!(rows.rows[1].prefill.as_deref(), Some("+act brace"));
        assert!(line_text(&rows.rows[1].line).contains("Brace"));
        assert_eq!(rows.rows[2].prefill.as_deref(), Some("+act aim"));
        let all: String = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        mush_contract_tests::assert_projection_text_clean(
            "I9 generic view",
            &all,
            &["secret-token", "another-secret", "_offer"],
        );
    }

    /// I6: the vitals pane's widgets — a gauge with digits, boxes grouped
    /// in threes with the level and the detail, a readout, the condition
    /// mark per state — in ASCII and UTF-8; swapped bounds and empty boxes
    /// never panic.
    #[test]
    fn vitals_rows_render_each_widget_and_clamp() {
        let frame = VitalsFrame::from_panel_data(&serde_json::json!({
            "subject": { "id": "#5", "name": "Nick" },
            "revision": 4,
            "vitals": [
                { "id": "t.pool", "label": "Pool", "role": "pool",
                  "value": { "kind": "bounded", "current": 6, "min": 0, "max": 10 }, "state": "warn" },
                { "id": "t.track", "label": "Track", "role": "dominant",
                  "value": { "kind": "track", "boxes": ["bashing", "lethal", null, null, null, null, null], "level": "Hurt" },
                  "state": "crit", "detail": "-1 dice" },
                { "id": "t.read", "label": "Readout", "role": "readout",
                  "value": { "kind": "text", "text": "P5 M4 S3" }, "state": "ok" },
                { "id": "t.swap", "label": "Swapped", "role": "pool",
                  "value": { "kind": "bounded", "current": 50, "min": 10, "max": 0 }, "state": "down" },
                { "id": "t.empty", "label": "Empty", "role": "track",
                  "value": { "kind": "track", "boxes": [], "level": null }, "state": "ok" }
            ]
        }))
        .unwrap();
        let frames = vec![(mush_proto::SmolStr::new_static("t.vitals"), frame)];
        let rows = render_vitals_rows(&frames, false);
        let t: Vec<String> = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        assert_eq!(t[0], "Nick  rev 4");
        assert_eq!(t[1], "! Track       [/X_][___][_]  Hurt  -1 dice");
        assert_eq!(t[2], "+ Empty       ");
        assert_eq!(t[3], "~ Pool        #####--- 6/10");
        assert_eq!(t[4], "X Swapped     ######## 50/10");
        assert_eq!(t[5], "+ Readout     P5 M4 S3");
        let rows = render_vitals_rows(&frames, true);
        let t: Vec<String> = rows.rows.iter().map(|r| line_text(&r.line)).collect();
        assert!(
            t[1].starts_with("\u{2717} Track       [/X\u{25a1}]"),
            "{}",
            t[1]
        );
        assert!(
            t[3].contains("\u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b1}\u{25b1}\u{25b1} 6/10"),
            "{}",
            t[3]
        );
        assert_eq!(
            line_text(&render_vitals_rows(&[], true).rows[0].line),
            "(no vitals feed)"
        );
    }

    #[test]
    fn vitals_status_tabs_and_legend_lines() {
        let frame = VitalsFrame::new(
            ObjRef::new("#5", "Nick"),
            1,
            vec![
                mush_proto::Vital {
                    id: "wod.willpower".into(),
                    label: "WP".into(),
                    role: VitalRole::Pool,
                    value: VitalValue::Bounded {
                        current: 6,
                        min: 0,
                        max: 10,
                    },
                    state: VitalState::Ok,
                    detail: None,
                },
                mush_proto::Vital {
                    id: "wod.health".into(),
                    label: "Health".into(),
                    role: VitalRole::Dominant,
                    value: VitalValue::Track {
                        boxes: vec![Some("lethal".into()), None, None],
                        level: Some("Bruised".into()),
                    },
                    state: VitalState::Warn,
                    detail: None,
                },
                mush_proto::Vital {
                    id: "wod.wound".into(),
                    label: "wound".into(),
                    role: VitalRole::Readout,
                    value: VitalValue::Text { text: "-1".into() },
                    state: VitalState::Ok,
                    detail: None,
                },
            ],
        )
        .unwrap();
        let held = [("wod.vitals".into(), frame)];
        let line = vitals_line(&held, false, 0, None);
        assert_eq!(
            line_text(&line),
            "[ Health X__ Bruised ]  [ WP #####--- 6/10 ]  [ wound -1 ]"
        );
        // Look-back LB-8: the strip is one row and does not wrap, so a width
        // that cannot hold every cell drops cells from the right and says how
        // many went.
        assert_eq!(
            line_text(&vitals_line(&held, false, 46, None)),
            "[ Health X__ Bruised ]  +2 more",
            "the marker's own width counts against the budget"
        );
        assert_eq!(
            line_text(&vitals_line(&held, false, 8, None)),
            "+3 more",
            "not even one cell fits"
        );
        assert!(
            line_text(&vitals_line(&held, false, 200, None)).ends_with("[ wound -1 ]"),
            "room for everything means no marker"
        );
        let theme = AnsiTheme::classic();
        let mut view = RoomView {
            name: "Hall".into(),
            id: "#2".into(),
            desc: RichText::plain(""),
            contents: vec![],
            exits: vec![
                ExitRef {
                    name: "north".into(),
                    aliases: vec!["n".into()],
                },
                ExitRef {
                    name: "quai".into(),
                    aliases: vec![],
                },
            ],
            zone: Some(ObjRef::new("#9", "Downtown")),
            ambient: Default::default(),
        };
        view.ambient
            .insert("weather".into(), serde_json::json!({ "state": "rain" }));
        view.ambient.insert(
            "ic_time".into(),
            serde_json::json!({ "hour": 14, "minute": 5 }),
        );
        let status = status_line(
            Some(&view),
            &[('n', "north".to_string())],
            false,
            &theme,
            ColorDepth::None,
        );
        assert_eq!(
            line_text(&status),
            "Hall #2 | Downtown | rain | IC 14:05 | n:north  quai"
        );
        let tabs = tab_strip(&[(1, "scene", 0), (3, "mail", 2)], 1, "Nick 12:00", 60);
        let text = line_text(&tabs);
        assert!(
            text.starts_with("rustmush  1 scene   3 mail 2 "),
            "{text:?}"
        );
        assert!(text.ends_with("Nick 12:00"));
        assert_eq!(text.chars().count(), 60);
        let nav = legend_line(false, &[('l', "look")], false, 0);
        assert_eq!(line_text(&nav), "l look | Enter type | ? help");
        let input = legend_line(true, &[], false, 0);
        assert_eq!(
            line_text(&input),
            "Up history | Tab complete | ^P mode | Esc nav"
        );
        assert_eq!(hhmm(0), "00:00");
        assert_eq!(hhmm(86_399), "23:59");
        assert_eq!(hhmm(-60), "23:59");
    }

    // ---- I8: the tactical unit panes -------------------------------------

    /// The contract's F1 keyframe, verbatim.
    fn f1() -> Value {
        serde_json::json!({
            "object": { "id": "#7", "name": "Atlas" },
            "schema": "tacsim:unit:atlas@1#0000000000000000000000000000000000000000000000000000000000000000",
            "revision": 3, "state": "active", "record": null,
            "mobility": { "mode": "ground.biped", "switching": null, "modes": [
                { "id": "ground.biped", "locomotion": "legged", "mediums": ["ground"], "tags": [],
                  "nominal_speed_cms": 640, "transition_s": 0 } ] },
            "sections": [ { "id": "center_torso", "condition": "ok", "state": "crit",
                "facets": [ { "id": "rear", "cells": ["armor"] } ], "slots": ["torso_hardpoint"] } ],
            "cells": [
                { "addr": "unit/heat",                      "label": "heat",      "current": 20, "min": 0, "max": 30, "state": "ok"   },
                { "addr": "section/center_torso/structure", "label": "structure", "current": 8,  "min": 0, "max": 8,  "state": "ok"   },
                { "addr": "facet/center_torso/rear/armor",  "label": "armor",     "current": 1,  "min": 0, "max": 6,  "state": "crit" },
                { "addr": "component/gauss/ammo",           "label": "ammo",      "current": 9,  "min": 0, "max": 12, "state": "ok"   }
            ],
            "components": [ { "id": "gauss",
                "definition": "tacsim:component:gauss@1#0000000000000000000000000000000000000000000000000000000000000000",
                "install": "slot/center_torso/torso_hardpoint", "condition": "ok", "state": "ok", "asset": false } ],
            "capabilities": [ { "id": "tactics.mobility", "source": null } ],
            "counts": { "sections": 1, "cells": 4, "components": 1, "capabilities": 1 },
            "truncated": false
        })
    }

    /// The contract's F3 delta, verbatim, for `object`.
    fn f3(object: &str) -> Value {
        serde_json::json!({
            "object": { "id": object, "name": "Atlas" },
            "from_revision": 3, "to_revision": 4, "now": 1_757_000_000, "cause": "gauss",
            "entries": [
                { "kind": "cell",               "addr": "facet/center_torso/rear/armor", "from": 6, "to": 1 },
                { "kind": "section",            "section": "center_torso", "from": "ok", "to": "damaged" },
                { "kind": "component",          "component": "gauss", "from": "ok", "to": "destroyed" },
                { "kind": "asset",              "component": "gauss", "attached": false },
                { "kind": "capability_granted", "id": "tactics.mobility", "source": "gauss", "replaced": false },
                { "kind": "capability_revoked", "id": "tactics.mobility", "source": "gauss", "was_present": true }
            ]
        })
    }

    fn ringed(deltas: &[Value]) -> PanelStore {
        let mut store = PanelStore::new();
        for d in deltas {
            store.apply_panel(
                &mush_proto::SmolStr::new("tactics.unit"),
                mush_proto::PanelFrameKind::Delta,
                d,
                0,
            );
        }
        store
    }

    fn texts(rows: &PaneRows) -> Vec<String> {
        rows.rows.iter().map(|r| line_text(&r.line)).collect()
    }

    /// I8 / rule 1: every unit pane renders labels, ids and words straight out
    /// of the frame — the client names no section, component, trait or unit
    /// kind — and the only word it ever maps to a colour is a `state`. A
    /// `condition` word is printed and never judged.
    #[test]
    fn unit_panes_render_the_frame_and_colour_only_from_state() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let data = f1();

        let sections = texts(&render_unit_sections_rows(&data, &ctx));
        assert_eq!(sections[0], "Atlas #7  rev 3");
        assert_eq!(sections[1], "\u{2717} center_torso  ok", "{sections:?}");
        assert_eq!(
            sections[2],
            "    structure \u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b0} 8/8"
        );
        assert_eq!(
            sections[3], "    rear/armor \u{25b0}\u{25b1}\u{25b1}\u{25b1}\u{25b1} 1/6",
            "a facet cell keeps its facet in the label"
        );
        // The section is `condition: ok` but `state: crit`: the glyph and the
        // colour follow the STATE, and the word is printed as the ruleset
        // wrote it. Mapping `ok` to green here would be the client judging.
        let head = &render_unit_sections_rows(&data, &ctx).rows[1];
        assert_eq!(head.line.spans[0].style.fg, Some(Color::Indexed(1)));
        assert!(line_text(&head.line).contains("ok"));

        let compact = texts(&render_unit_sections_compact_rows(&data, &ctx));
        assert_eq!(
            compact,
            vec![
                "Atlas #7  rev 3".to_string(),
                "\u{2717} center_torso  ok  structure \u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b0} 8/8"
                    .to_string(),
                "    rear/armor \u{25b0}\u{25b1}\u{25b1}\u{25b1}\u{25b1} 1/6".to_string(),
            ],
            "the first cell shares the section's line, the rest hang under it"
        );

        let plant = texts(&render_unit_plant_rows(&data, &ctx));
        assert_eq!(
            plant[0], "\u{2713} heat \u{25b0}\u{25b0}\u{25b0}\u{25b1}\u{25b1} 20/30",
            "only unit-scoped cells"
        );
        assert_eq!(plant[1], "mobility ground.biped");
        assert_eq!(plant.len(), 2);
        // A frame carries at most ONE of `mode` / `switching` — the server
        // reports a switch only while it is in progress, and a due-but-
        // unsettled one reports its new mode — so the pane and the server's
        // own `@unit/show` text agree by construction, whichever the pane
        // reads first.
        let mut mid = data.clone();
        mid["mobility"] = serde_json::json!({ "mode": null, "modes": [],
            "switching": { "from": "ground.biped", "to": "ground.quad", "ready_at": 110 } });
        assert_eq!(
            texts(&render_unit_plant_rows(&mid, &ctx))[1],
            "mobility ground.biped \u{2192} ground.quad (ready 110)"
        );
        let mut settled = data.clone();
        settled["mobility"] =
            serde_json::json!({ "mode": "ground.quad", "switching": null, "modes": [] });
        assert_eq!(
            texts(&render_unit_plant_rows(&settled, &ctx))[1],
            "mobility ground.quad"
        );
        let mut none = data.clone();
        none["mobility"] = Value::Null;
        assert_eq!(
            texts(&render_unit_plant_rows(&none, &ctx))[1],
            "mobility none"
        );

        let components = texts(&render_unit_components_rows(&data, &ctx));
        assert_eq!(components[0], "unit", "the unit's own group leads");
        assert_eq!(
            components[1], "  \u{b7} tactics.mobility",
            "a capability no component sources hangs off the unit"
        );
        assert_eq!(components[2], "center_torso", "grouped by install section");
        assert!(components[3].starts_with("  \u{2713} gauss"));
        assert!(components[3].contains("ok"));
        assert!(
            !components[3].contains("asset"),
            "asset: false says nothing: {:?}",
            components[3]
        );
        assert_eq!(
            components[4], "      ammo \u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b1} 9/12",
            "a component's own cells render under it"
        );

        // ASCII terminals get the fallback glyphs, never a UTF-8 byte.
        let ascii = RenderCtx { utf8: false, ..ctx };
        let all = texts(&render_unit_sections_rows(&data, &ascii)).join("\n")
            + &texts(&render_unit_plant_rows(&data, &ascii)).join("\n")
            + &texts(&render_unit_components_rows(&data, &ascii)).join("\n");
        assert!(all.is_ascii(), "{all:?}");
        assert!(all.contains("! center_torso") && all.contains("+ heat #"));
    }

    /// I8: a `null` frame is the honest no-unit state on every pane, and a
    /// garbage payload degrades to the same line rather than panicking or
    /// claiming to be loading (invariant T6). The journal and before/after
    /// panes are judged against an EMPTY store: with nothing held AND nothing
    /// ringed there is genuinely nothing to say, which is what the NO-UNIT
    /// line says. (With a ring they render it — the test below.)
    #[test]
    fn a_null_or_garbage_unit_frame_is_the_no_unit_line_on_every_pane() {
        let theme = AnsiTheme::classic();
        let store = ringed(&[f3("#7")]);
        let empty = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let unringed = RenderCtx {
            store: &empty,
            ..ctx
        };
        let renderers: [(&str, super::super::screen::PanelRenderer, bool); 6] = [
            ("sections", render_unit_sections_rows, false),
            ("compact", render_unit_sections_compact_rows, false),
            ("plant", render_unit_plant_rows, false),
            ("components", render_unit_components_rows, false),
            ("journal", render_unit_journal_rows, true),
            ("delta", render_unit_delta_rows, true),
        ];
        for bad in [
            Value::Null,
            serde_json::json!({}),
            serde_json::json!([1, 2]),
            serde_json::json!("nope"),
            serde_json::json!({ "object": { "name": "no id" } }),
        ] {
            for (name, render, reads_ring) in renderers {
                let rows = render(&bad, if reads_ring { &unringed } else { &ctx });
                assert_eq!(rows.rows.len(), 1, "{name} on {bad}");
                assert_eq!(line_text(&rows.rows[0].line), NO_UNIT_LINE, "{name}");
            }
        }
        // A frame with a subject but nothing else still renders, honestly.
        let bare = serde_json::json!({ "object": { "id": "#7", "name": "Atlas" } });
        for (name, render, _) in renderers {
            let rows = render(&bare, &ctx);
            assert!(!rows.rows.is_empty(), "{name}");
            for r in &rows.rows {
                assert_ne!(line_text(&r.line), "(loading...)", "{name}");
            }
        }
    }

    /// I8 (cross must_fix 3): the actor who PRODUCES an after-action report
    /// is the one least likely to hold a station on its subject — `@unit/damage`
    /// is staff-gated, the fan-out puts the staffer first, and their own
    /// resync can only ask for their own default subject, which answers
    /// `null`. So with a `null` frame held and a non-empty ring the journal
    /// and before/after panes render the ring, naming the object of every
    /// row; the panes that describe a unit's own state keep the NO-UNIT line,
    /// because there is no unit to describe.
    #[test]
    fn a_null_keyframe_still_shows_the_after_action_ring() {
        let theme = AnsiTheme::classic();
        let store = ringed(&[f3("#7")]);
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let journal = texts(&render_unit_journal_rows(&Value::Null, &ctx));
        assert_eq!(journal.len(), 6, "{journal:?}");
        for row in &journal {
            assert!(
                row.starts_with("#7 rev 4 \u{b7} gauss \u{b7} "),
                "every row names its own subject: {row:?}"
            );
        }
        let delta = texts(&render_unit_delta_rows(&Value::Null, &ctx));
        assert_eq!(
            delta[0], "#7 rev 4  gauss",
            "the head says whose report it is"
        );
        assert_eq!(delta[1], "facet/center_torso/rear/armor  6 \u{2192} 1");
        assert_eq!(delta.len(), 7);
        for (name, render) in [
            (
                "sections",
                render_unit_sections_rows as super::super::screen::PanelRenderer,
            ),
            ("compact", render_unit_sections_compact_rows),
            ("plant", render_unit_plant_rows),
            ("components", render_unit_components_rows),
        ] {
            let rows = texts(&render(&Value::Null, &ctx));
            assert_eq!(rows, vec![NO_UNIT_LINE.to_string()], "{name}");
        }
    }

    /// I8 (the plan's promised fan-out-order test): the delta arrives BEFORE
    /// any keyframe — D-I8-2 sends it first and the keyframe only answers the
    /// resync — so the before/after table must read the same rows either way,
    /// losing only the object prefix once the subject is known.
    #[test]
    fn a_delta_before_any_keyframe_then_the_keyframe_leaves_the_before_after_table_consistent() {
        let theme = AnsiTheme::classic();
        let mut store = PanelStore::new();
        let feed = mush_proto::SmolStr::new("tactics.unit");
        store.apply_panel(&feed, mush_proto::PanelFrameKind::Delta, &f3("#7"), 0);
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let before = texts(&render_unit_delta_rows(&Value::Null, &ctx));
        assert_eq!(before[0], "#7 rev 4  gauss");
        assert_eq!(before[1], "facet/center_torso/rear/armor  6 \u{2192} 1");
        let journal_before = texts(&render_unit_journal_rows(&Value::Null, &ctx));
        assert!(journal_before[0].starts_with("#7 rev 4"));

        store.apply_panel(&feed, mush_proto::PanelFrameKind::Keyframe, &f1(), 1);
        assert_eq!(
            store.unit_journal().len(),
            6,
            "the first keyframe never throws the rows away"
        );
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let after = texts(&render_unit_delta_rows(&f1(), &ctx));
        assert_eq!(after[0], "rev 4  gauss", "the subject is held: no prefix");
        assert_eq!(after[1..], before[1..], "the same table, either way");
        let journal_after = texts(&render_unit_journal_rows(&f1(), &ctx));
        assert!(journal_after[0].starts_with("rev 4 \u{b7} gauss \u{b7} cell "));
        assert_eq!(journal_after.len(), 6);
    }

    /// I8: the journal reads the ring newest first and renders an unknown
    /// entry kind as an opaque row; the before/after table takes the newest
    /// revision of the subject actually HELD, and a row about another subject
    /// (a staffer's `@unit/damage #n`, which the client's empty-params resync
    /// cannot follow) is kept out of the table and named in the journal.
    #[test]
    fn the_before_after_pane_ignores_a_ring_row_for_another_object_and_prints_its_object_in_the_journal()
     {
        let theme = AnsiTheme::classic();
        let mut store = ringed(&[f3("#7")]);
        // A foreign delta, and a newer revision for it, arriving after ours.
        let mut foreign = f3("#8");
        foreign["to_revision"] = serde_json::json!(9);
        foreign["entries"] = serde_json::json!([
            { "kind": "cell", "addr": "unit/heat", "from": 1, "to": 2 },
            { "kind": "meteor_strike", "note": "from a newer server" }
        ]);
        store.apply_panel(
            &mush_proto::SmolStr::new("tactics.unit"),
            mush_proto::PanelFrameKind::Delta,
            &foreign,
            0,
        );
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let data = f1();

        // Newest DELTA first, entries within a delta in the order the server
        // wrote them (cell → section → component → asset → capability), so
        // this pane and the before/after table beside it read the same way.
        let journal = texts(&render_unit_journal_rows(&data, &ctx));
        assert_eq!(journal.len(), 8, "{journal:?}");
        assert!(
            journal[0].starts_with("#8 rev 9 \u{b7} gauss \u{b7} cell unit/heat 1"),
            "the newer delta leads, and the foreign subject is named: {journal:?}"
        );
        assert_eq!(
            journal[1], "#8 rev 9 \u{b7} gauss \u{b7} meteor_strike",
            "an unknown kind is an opaque row"
        );
        assert!(
            journal[2].contains("cell facet/center_torso/rear/armor 6\u{2192}1"),
            "{journal:?}"
        );
        assert_eq!(
            journal[7], "rev 4 \u{b7} gauss \u{b7} capability_revoked tactics.mobility (gauss)",
            "the held subject is not repeated on every row"
        );

        let delta = texts(&render_unit_delta_rows(&data, &ctx));
        assert_eq!(delta[0], "rev 4  gauss", "rev 9 belongs to #8, not to us");
        assert_eq!(delta[1], "facet/center_torso/rear/armor  6 \u{2192} 1");
        assert_eq!(delta[2], "section center_torso  ok \u{2192} damaged");
        assert_eq!(delta[3], "component gauss  ok \u{2192} destroyed");
        assert_eq!(delta[4], "asset gauss detached");
        assert_eq!(delta.len(), 7);
        assert!(
            !delta.iter().any(|r| r.contains("unit/heat")),
            "no foreign row in the table: {delta:?}"
        );

        // With nothing of ours in the ring the table says so, not "loading".
        let mine_only = ringed(&[foreign.clone()]);
        let ctx = RenderCtx {
            store: &mine_only,
            ..ctx
        };
        assert_eq!(
            texts(&render_unit_delta_rows(&data, &ctx)),
            vec!["no after-action entries yet".to_string()]
        );
        let empty = PanelStore::new();
        let ctx = RenderCtx {
            store: &empty,
            ..ctx
        };
        assert_eq!(
            texts(&render_unit_journal_rows(&data, &ctx)),
            vec!["(no journal entries yet)".to_string()]
        );

        // The table takes the newest-ARRIVED delta for the held object, never
        // the highest revision: a remove-and-recreate resets a unit to
        // revision 0, and the dead unit's rev 7 must not be reported as the
        // current damage.
        let mut old = f3("#7");
        old["to_revision"] = serde_json::json!(7);
        old["cause"] = serde_json::json!("laser");
        let mut fresh = f3("#7");
        fresh["from_revision"] = serde_json::json!(0);
        fresh["to_revision"] = serde_json::json!(1);
        let recreated = ringed(&[old, fresh]);
        let ctx = RenderCtx {
            store: &recreated,
            ..ctx
        };
        assert_eq!(
            texts(&render_unit_delta_rows(&data, &ctx))[0],
            "rev 1  gauss",
            "the newest arrival wins, not the highest revision"
        );
    }

    /// I8 (hole 4): the compact map has no width to spend, so every cell after
    /// the first gets its own indented row instead of being appended to one
    /// long line (which clipped silently before 16b cut B.3, and would fold a
    /// gauge across two lines since `draw_pane` wraps).
    #[test]
    fn the_compact_sections_pane_never_drops_a_cell() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        assert_eq!(
            texts(&render_unit_sections_compact_rows(&f1(), &ctx)).len(),
            3,
            "the header, the section, and its second cell"
        );

        let mut wide = f1();
        wide["cells"] = Value::Array(
            (0..8)
                .map(|i| {
                    serde_json::json!({ "addr": format!("section/center_torso/cell_{i}"),
                        "label": format!("cell_{i}"), "current": 3, "min": 0, "max": 9,
                        "state": "warn" })
                })
                .collect(),
        );
        let rows = texts(&render_unit_sections_compact_rows(&wide, &ctx));
        let all = rows.join("\n");
        for i in 0..8 {
            assert!(
                all.contains(&format!("cell_{i}")),
                "cell_{i} dropped: {rows:?}"
            );
        }
        for row in &rows {
            assert!(
                row.chars().count() <= 60,
                "a row wider than the narrowest pane it must fit: {row:?}"
            );
        }
    }

    /// I8 (hole 6): a frame may declare any bounds an `i64` can hold. The
    /// gauge computes in `i128`, so `i64::MIN..=i64::MAX` draws a bar rather
    /// than overflowing inside the render path (a debug build panics on
    /// overflow, and `cargo test` is a debug build).
    #[test]
    fn gauge_bar_tolerates_extreme_bounds() {
        assert_eq!(
            gauge_bar(i64::MAX, i64::MIN, i64::MAX, 5, true),
            "\u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b0}"
        );
        assert_eq!(
            gauge_bar(0, i64::MIN, i64::MAX, 5, true),
            "\u{25b0}\u{25b0}\u{25b0}\u{25b1}\u{25b1}"
        );
        assert_eq!(gauge_bar(i64::MAX, 0, i64::MAX, 5, false), "#####");
        assert_eq!(
            gauge_bar(i64::MIN, i64::MIN, i64::MAX, 5, true),
            "\u{25b1}\u{25b1}\u{25b1}\u{25b1}\u{25b1}"
        );
        // Reversed bounds and a span of nothing still draw, as before.
        assert_eq!(gauge_bar(3, 10, 0, 5, false), "##---");
        assert_eq!(gauge_bar(5, 4, 4, 5, false), "-----");
    }

    /// I9 / I8 (hole 7): a generic list whose producer capped its rows says
    /// so, the way the unit panes say what their byte budget cut. The
    /// contacts table is the first producer to reach the cap.
    #[test]
    fn a_truncated_generic_list_says_so() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let view = |truncated: Option<bool>| {
            let mut v = serde_json::json!({
                "schema": { "id": "tactics.contacts", "version": 1, "shape": "list",
                    "columns": [ { "key": "id", "label": "contact", "align": "left" } ],
                    "fields": [], "actionKey": "prefill" },
                "rows": [ { "id": "aabbccddeeff", "prefill": "+contact aabbccddeeff" } ]
            });
            if let Some(t) = truncated {
                v["truncated"] = serde_json::json!(t);
            }
            v
        };
        let cut = texts(&render_view_rows(&view(Some(true)), &ctx));
        assert_eq!(cut.last().unwrap(), "(more rows than shown)");
        assert_eq!(cut.len(), 3, "header, row, footer: {cut:?}");
        for whole in [view(Some(false)), view(None)] {
            let rows = texts(&render_view_rows(&whole, &ctx));
            assert_eq!(rows.len(), 2, "{rows:?}");
            assert!(!rows.iter().any(|r| r.contains("more rows")));
        }
    }

    /// I8 (the components-pane gap): the byte-budget walk admits components
    /// before capabilities, so a truncated frame can carry a capability whose
    /// source is not in the frame. It renders under a trailing `(other)`
    /// group rather than nowhere, and the footer counts capabilities too.
    #[test]
    fn a_capability_whose_source_is_not_in_the_frame_still_renders() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let mut cut = f1();
        cut["capabilities"] = serde_json::json!([
            { "id": "tactics.mobility", "source": null },
            { "id": "tactics.targeting", "source": "gauss" },
            { "id": "tactics.jump", "source": "missing_component" }
        ]);
        cut["truncated"] = serde_json::json!(true);
        cut["counts"] =
            serde_json::json!({ "sections": 1, "cells": 4, "components": 3, "capabilities": 5 });
        let rows = texts(&render_unit_components_rows(&cut, &ctx));
        let other = rows
            .iter()
            .position(|r| r == "(other)")
            .unwrap_or_else(|| panic!("an (other) group: {rows:?}"));
        assert_eq!(rows[other + 1], "  \u{b7} tactics.jump");
        assert!(
            rows.iter().any(|r| r == "    \u{b7} tactics.targeting"),
            "a sourced capability still hangs off its component: {rows:?}"
        );
        assert_eq!(
            rows.last().unwrap(),
            "(view truncated: 0 cells, 2 components, 2 capabilities not shown)"
        );
        // A whole frame grows no (other) group.
        let rows = texts(&render_unit_components_rows(&f1(), &ctx));
        assert!(!rows.iter().any(|r| r == "(other)"), "{rows:?}");
    }

    /// I8: a delta the ring had to cut says so rather than lying by omission,
    /// and a truncated view names what the byte budget left out.
    #[test]
    fn an_elided_delta_and_a_truncated_view_both_say_what_is_missing() {
        let theme = AnsiTheme::classic();
        let mut huge = f3("#7");
        huge["entries"] = Value::Array(
            (0..80)
                .map(|i| serde_json::json!({ "kind": "cell", "addr": format!("unit/c{i}"), "from": 2, "to": 1 }))
                .collect(),
        );
        let store = ringed(&[huge]);
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        // The elision marker is an ordinary ring row: it keeps its delta's
        // object, revision and cause, and it sits LAST within its own delta,
        // where the entries it stands for are.
        let journal = texts(&render_unit_journal_rows(&f1(), &ctx));
        assert_eq!(journal.len(), 50);
        assert_eq!(
            journal[49],
            "rev 4 \u{b7} gauss \u{b7} \u{2026} 31 more entries"
        );
        assert!(journal[0].starts_with("rev 4 \u{b7} gauss \u{b7} cell unit/c0"));

        // In a mixed ring the marker still says which volley was cut.
        let mut later = f3("#8");
        later["to_revision"] = serde_json::json!(9);
        later["cause"] = serde_json::json!("laser");
        later["entries"] = Value::Array(
            (0..60)
                .map(|i| serde_json::json!({ "kind": "cell", "addr": format!("unit/d{i}"), "from": 2, "to": 1 }))
                .collect(),
        );
        let mut mixed = ringed(&[f3("#7")]);
        mixed.apply_panel(
            &mush_proto::SmolStr::new("tactics.unit"),
            mush_proto::PanelFrameKind::Delta,
            &later,
            0,
        );
        let ctx2 = RenderCtx {
            store: &mixed,
            ..ctx
        };
        let journal = texts(&render_unit_journal_rows(&f1(), &ctx2));
        assert_eq!(
            journal[49],
            "#8 rev 9 \u{b7} laser \u{b7} \u{2026} 11 more entries",
            "{:?}",
            &journal[..3]
        );
        assert!(journal[50].starts_with("rev 4 \u{b7} gauss \u{b7} cell "));

        let mut cut = f1();
        cut["truncated"] = serde_json::json!(true);
        cut["counts"] = serde_json::json!({ "sections": 1, "cells": 900, "components": 40,
            "capabilities": 4 });
        let rows = texts(&render_unit_sections_rows(&cut, &ctx));
        assert_eq!(
            rows.last().unwrap(),
            "(view truncated: 896 cells, 39 components, 3 capabilities not shown)"
        );
    }

    /// I8, re-derived for the floor (16b cut AB.1): ten named tabs take 110
    /// columns and nine 98, so at 120 every tab is named — beside ten tabs the
    /// right-hand block (`Nick ● 3 on 14:30`, 17) does not fit and is dropped,
    /// beside nine it does. The strip elides only when unread badges carry the
    /// named tabs past the width: inactive tabs keep their digit and their
    /// badge, the active tab keeps its name. At 160 there is room for all.
    #[test]
    fn the_tab_strip_names_every_tab_at_one_twenty_and_elides_only_for_badges() {
        let right = "Nick \u{25cf} 3 on 14:30";
        let ten: Vec<(u8, &str, usize)> = vec![
            (1, "scene", 0),
            (2, "sheet", 0),
            (3, "mail", 0),
            (4, "chan", 0),
            (5, "rolls", 0),
            (6, "intel", 0),
            (7, "actions", 0),
            (8, "damage", 0),
            (9, "unit", 0),
            (0, "cockpit", 0),
        ];
        let names = [
            " 1 scene ",
            " 2 sheet ",
            " 3 mail ",
            " 4 chan ",
            " 5 rolls ",
            " 6 intel ",
            " 7 actions ",
            " 8 damage ",
            " 9 unit ",
            " 0 cockpit ",
        ];
        // Ten tabs at the floor: all named, and no room for the block.
        let floor = line_text(&tab_strip(&ten, 0, right, 120));
        for name in names {
            assert!(floor.contains(name), "{name:?} at 120: {floor:?}");
        }
        assert!(!floor.contains("Nick"), "the block is dropped: {floor:?}");
        assert_eq!(floor.chars().count(), 110);

        // Nine (a viewer with no seat): all named, and the block fits.
        let nine = line_text(&tab_strip(&ten[..9], 1, right, 120));
        for name in names.iter().take(9) {
            assert!(nine.contains(name), "{name:?} at 120: {nine:?}");
        }
        assert!(nine.ends_with(right), "{nine:?}");
        assert_eq!(nine.chars().count(), 120);

        // Unread badges past the width: the elision, badges kept. The rule is
        // the strip's for any badge, not only the mail and channel ones the
        // app counts today.
        let badged: Vec<(u8, &str, usize)> = ten
            .iter()
            .map(|&(d, n, _)| {
                let badge = match d {
                    2 => 1,
                    3 => 3,
                    4 => 7,
                    6 => 9,
                    8 => 2,
                    9 => 4,
                    _ => 0,
                };
                (d, n, badge)
            })
            .collect();
        let elided = line_text(&tab_strip(&badged, 0, right, 120));
        assert!(
            elided.contains(" 0 cockpit "),
            "the active tab keeps its name: {elided:?}"
        );
        assert!(
            !elided.contains("scene") && !elided.contains("actions"),
            "{elided:?}"
        );
        assert!(
            elided.contains(" 3 3 ") && elided.contains(" 4 7 ") && elided.contains(" 9 4 "),
            "unread survives the elision: {elided:?}"
        );
        assert!(elided.ends_with(right), "{elided:?}");
        assert_eq!(elided.chars().count(), 120);
        // Exactly: every inactive tab keeps its digit, with or without a
        // badge, at the same spacing, and the block is right-aligned after
        // the padding (68 columns of tabs, 35 of padding, 17 of block).
        assert_eq!(
            elided,
            format!(
                "rustmush  1   2 1   3 3   4 7   5   6 9   7   8 2   9 4   0 cockpit {}{right}",
                " ".repeat(35)
            ),
            "the elided strip"
        );

        // At 160 the ten named tabs and the block.
        let wide = line_text(&tab_strip(&ten, 0, right, 160));
        for name in names {
            assert!(wide.contains(name), "{name:?} at 160: {wide:?}");
        }
        assert!(wide.ends_with(right), "{wide:?}");
        assert_eq!(wide.chars().count(), 160);
    }

    /// **16b cut AB.1**: the floor sentence names the floor and the terminal's
    /// own width, word for word.
    #[test]
    fn floor_line_names_the_floor_and_the_terminal() {
        assert_eq!(
            floor_line(80),
            "The TUI needs at least 120 columns and this terminal is 80 wide: make it \
             larger, or press Ctrl-] or type @notui for line mode."
        );
        assert_eq!(
            floor_line(119),
            "The TUI needs at least 120 columns and this terminal is 119 wide: make it \
             larger, or press Ctrl-] or type @notui for line mode."
        );
    }

    /// The shipped line tank's five sections as the frame carries them
    /// (`crates/tacsim/content/units/line_tank.json`, undamaged): the hull's
    /// five facets and the turret's four at three digits, one facet on each
    /// track, none on the engine.
    fn line_tank_sections() -> Value {
        serde_json::json!([
            { "id": "hull", "condition": "intact", "internals": 200, "internals_max": 200,
              "facets": [
                { "id": "front", "armor": 400, "armor_max": 400 },
                { "id": "left", "armor": 250, "armor_max": 250 },
                { "id": "right", "armor": 250, "armor_max": 250 },
                { "id": "rear", "armor": 150, "armor_max": 150 },
                { "id": "top", "armor": 100, "armor_max": 100 } ] },
            { "id": "turret", "condition": "intact", "internals": 150, "internals_max": 150,
              "facets": [
                { "id": "front", "armor": 350, "armor_max": 350 },
                { "id": "left", "armor": 220, "armor_max": 220 },
                { "id": "right", "armor": 220, "armor_max": 220 },
                { "id": "rear", "armor": 140, "armor_max": 140 } ] },
            { "id": "left_track", "condition": "intact", "internals": 100, "internals_max": 100,
              "facets": [ { "id": "left", "armor": 120, "armor_max": 120 } ] },
            { "id": "right_track", "condition": "destroyed", "internals": 100, "internals_max": 100,
              "facets": [ { "id": "right", "armor": 120, "armor_max": 120 } ] },
            { "id": "engine", "condition": "intact", "internals": 120, "internals_max": 120,
              "facets": [] }
        ])
    }

    /// **16b cut AB.1**: the damage pane packs a section's facet cells into
    /// rows of the pane's width and never splits one. The shipped line tank at
    /// the Left column's 32 inner columns is exactly twelve rows — the pane's
    /// `min_h` — and with no pane size (the input paths) it is one row per
    /// section, as it was.
    #[test]
    fn the_damage_pane_packs_facet_cells_whole() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = |pane: Option<(u16, u16)>| RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane,
            picture: None,
        };
        let mut frame = cockpit(serde_json::json!([5, 5]), serde_json::json!([]));
        frame["sections"] = line_tank_sections();
        let rows: Vec<String> = render_cockpit_damage_rows(&frame, &ctx(Some((32, 12))))
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(
            rows,
            [
                "hull  intact  200/200",
                "  front 400/400  left 250/250",
                "  right 250/250  rear 150/150",
                "  top 100/100",
                "turret  intact  150/150",
                "  front 350/350  left 220/220",
                "  right 220/220  rear 140/140",
                "left_track  intact  100/100",
                "  left 120/120",
                // The widest condition row a shipped section prints, 31.
                "right_track  destroyed  100/100",
                "  right 120/120",
                "engine  intact  120/120",
            ]
        );
        for row in &rows {
            assert!(row.chars().count() <= 32, "{row:?} wraps the pane");
        }
        // Every facet cell is whole on one row, and a facet row is indented.
        for cell in [
            "front 400/400",
            "left 250/250",
            "right 250/250",
            "rear 150/150",
            "top 100/100",
            "front 350/350",
            "left 220/220",
            "right 220/220",
            "rear 140/140",
            "left 120/120",
            "right 120/120",
        ] {
            let row = rows
                .iter()
                .find(|row| row.contains(cell))
                .unwrap_or_else(|| panic!("{cell:?} is not whole on any row: {rows:?}"));
            assert!(row.starts_with("  "), "{row:?}");
        }
        // The input paths: one row per section, its facets appended.
        let flat: Vec<String> = render_cockpit_damage_rows(&frame, &ctx(None))
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(flat.len(), 5, "{flat:?}");
        assert_eq!(
            flat.first().map(String::as_str),
            Some(
                "hull  intact  200/200  front 400/400  left 250/250  right 250/250  \
                 rear 150/150  top 100/100"
            )
        );
        assert_eq!(
            flat.last().map(String::as_str),
            Some("engine  intact  120/120")
        );
    }

    /// **16b cut AB.1**: the cockpit's legend at the 120-column floor keeps
    /// whole entries in order and ends on `? help`, instead of stopping
    /// mid-word; from 148 columns it is whole.
    #[test]
    fn the_legend_keeps_whole_entries_and_help_at_the_floor() {
        let screens = super::super::screen::builtin_screens();
        let cockpit = screens.last().expect("the cockpit screen");
        let bindings: Vec<(char, &str)> = cockpit.keys.iter().map(|b| (b.key, b.label)).collect();
        assert_eq!(bindings.len(), 12);
        let whole = "k lock \u{b7} f fire \u{b7} h heading \u{b7} m speed \u{b7} p pull \u{b7} \
                     c contacts \u{b7} t status \u{b7} w weapons \u{b7} r range \u{b7} \
                     b bearing \u{b7} n scan \u{b7} s scan row \u{b7} Enter type \u{b7} ? help";
        assert_eq!(whole.chars().count(), 148);
        let floor = line_text(&legend_line(false, &bindings, true, 120));
        assert_eq!(
            floor,
            "k lock \u{b7} f fire \u{b7} h heading \u{b7} m speed \u{b7} p pull \u{b7} \
             c contacts \u{b7} t status \u{b7} w weapons \u{b7} r range \u{b7} b bearing \
             \u{b7} ? help"
        );
        assert_eq!(floor.chars().count(), 113);
        assert_eq!(line_text(&legend_line(false, &bindings, true, 160)), whole);
        assert_eq!(line_text(&legend_line(false, &bindings, true, 148)), whole);
        assert_eq!(line_text(&legend_line(false, &bindings, true, 0)), whole);
        // Input mode is never cut.
        assert_eq!(
            line_text(&legend_line(true, &bindings, true, 120)),
            line_text(&legend_line(true, &[], true, 0))
        );
    }

    // -----------------------------------------------------------------------
    // The cockpit panes (16b cut Q.4; doc 16 §5)
    // -----------------------------------------------------------------------

    /// A 40-by-30 arena of open grass with a wall down column 10.
    fn yard() -> Value {
        let rows: Vec<Value> = (0..30)
            .map(|_| serde_json::json!([[".", 10], ["#", 1], [".", 29]]))
            .collect();
        serde_json::json!({
            "schema": { "id": "arena.map", "version": 1, "shape": "terrain", "retain": "latest" },
            "match": 3, "surface": "yard", "cell_cm": 1000,
            "width": 40, "height": 30, "origin_cell": [0, 0],
            "rows": rows, "legend": [[".", "grass"], ["#", "wall"]]
        })
    }

    fn cockpit(cell: Value, contacts: Value) -> Value {
        serde_json::json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "match": 3, "callsign": "RA", "cell": cell, "heading_deg": 90,
            "power": "online", "blocked": false, "speed_kph": 20, "tempo": "alert",
            "scope": "hostile",
            "heat": null, "lock_designator": null, "contacts": contacts,
            "scan_tables": [["engine", "hull", "turret"]],
            "sections": [], "weapons": []
        })
    }

    /// A contact as the wire carries one — the sixteen-digit arena id
    /// included, because the row that must never print it is the one assertion
    /// worth making here (16b cut Q.1's `no_sixteen_digit_word`, on this
    /// surface).
    fn contact(designator: &str, class: &str, friendly: bool, cell: Value) -> Value {
        serde_json::json!({
            "id": "2222222222222222",
            "designator": designator, "class": class, "friendly": friendly, "cell": cell,
            "bearing_deg": 90, "band": "close", "range_m": 50, "age_s": 0, "condition": null
        })
    }

    fn map_rows(frame: &Value, map: Option<&Value>, pane: (u16, u16)) -> Vec<String> {
        let theme = AnsiTheme::classic();
        let mut store = PanelStore::new();
        if let Some(map) = map {
            store.apply_panel(
                &mush_proto::SmolStr::new_static("arena.map"),
                mush_proto::PanelFrameKind::Keyframe,
                map,
                0,
            );
        }
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: Some(pane),
            picture: None,
        };
        render_cockpit_map_rows(frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect()
    }

    /// 16b cut Q.4: the window follows the platform and CLAMPS at the arena's
    /// edge — the same rule `VIEW` uses — so a tank in a corner is drawn at its
    /// own cell rather than at the middle of a window that ran off the map.
    #[test]
    fn the_cockpit_window_follows_the_pilot_and_clamps_at_the_edge() {
        let map = yard();
        // A ten-by-ten window (twelve rows of pane, two of chrome) in the
        // middle: the platform is drawn at the window's centre.
        let rows = map_rows(
            &cockpit(serde_json::json!([20, 15]), serde_json::json!([])),
            Some(&map),
            (10, 12),
        );
        assert_eq!(rows.len(), 12, "a head, ten rows, a legend");
        // x0 = 20 - 5 = 15, y0 = 15 - 5 = 10; the marker is at (5, 5) of the
        // window. The rows print north-up under one heading row, so window row
        // `y` is at index `h - y`: 10 - 5 = 5.
        assert_eq!(rows.get(5).and_then(|r| r.chars().nth(5)), Some('@'));

        // The south-west corner: both axes clamp to zero, and the platform is
        // drawn at its OWN cell — including on the window's very last row,
        // which is the row a pane that overran its area would clip.
        let rows = map_rows(
            &cockpit(serde_json::json!([1, 0]), serde_json::json!([])),
            Some(&map),
            (10, 12),
        );
        assert_eq!(
            rows.get(10).and_then(|r| r.chars().nth(1)),
            Some('@'),
            "{rows:?}"
        );
        assert_eq!(
            rows.get(1).map(String::len),
            Some(10),
            "ten columns, north-up: {rows:?}"
        );

        // The north-east corner: x0 = 40 - 10 = 30, y0 = 30 - 10 = 20.
        let rows = map_rows(
            &cockpit(serde_json::json!([39, 29]), serde_json::json!([])),
            Some(&map),
            (10, 12),
        );
        assert_eq!(rows.get(1).and_then(|r| r.chars().nth(9)), Some('@'));

        // A window wider than the arena takes the whole arena, wall included.
        let rows = map_rows(
            &cockpit(serde_json::json!([5, 5]), serde_json::json!([])),
            Some(&map),
            (200, 200),
        );
        assert_eq!(rows.len(), 32, "a head, thirty rows, a legend");
        assert_eq!(rows.get(1).and_then(|r| r.chars().nth(10)), Some('#'));
    }

    /// 16b cut Q.4: the marker order — the pilot first, then the contacts in
    /// the frame's own order, a later marker overwriting an earlier one — and a
    /// contact off the window is not drawn and not in the legend (it is still
    /// in the contacts pane, which is where the pilot reads it).
    #[test]
    fn markers_go_down_in_frame_order_and_a_contact_off_the_window_is_not_drawn() {
        let map = yard();
        let frame = cockpit(
            serde_json::json!([20, 15]),
            serde_json::json!([
                contact("X2", "unknown", false, serde_json::json!([18, 15])),
                contact("BA", "blue", false, serde_json::json!([22, 16])),
                contact("RB", "red", true, serde_json::json!([18, 15])),
                contact("X9", "unknown", false, serde_json::json!([39, 29])),
                contact("X8", "unknown", false, Value::Null),
            ]),
        );
        let rows = map_rows(&frame, Some(&map), (10, 12));
        // x0 = 15, y0 = 10.
        assert_eq!(
            rows.get(5).and_then(|r| r.chars().nth(3)),
            Some('+'),
            "RB came after X2 on the same cell and overwrote it: {rows:?}"
        );
        assert_eq!(rows.get(4).and_then(|r| r.chars().nth(7)), Some('x'));
        assert_eq!(rows.get(5).and_then(|r| r.chars().nth(5)), Some('@'));
        // The legend is the markers that OWN their cell, in frame order (16b
        // cut B.3, `B-62`): RB overwrote X2, so the legend names the `+` that is
        // on screen and not the `?` under it. (It used to name both.)
        assert_eq!(rows.last().map(String::as_str), Some("x BA  + RB"));
        // A contact in a far corner and one with no plot at all are neither
        // drawn nor named on the map.
        for off in ["X9", "X8"] {
            assert!(
                !rows.last().is_some_and(|l| l.contains(off)),
                "{off} is off the window: {rows:?}"
            );
        }
    }

    /// 16b cut Q.4: with no map of this match the pane says it is waiting, and
    /// a cleared frame is the one-line empty state on the map and nothing at
    /// all on the others.
    #[test]
    fn the_cockpit_panes_wait_for_the_ground_and_empty_on_a_cleared_frame() {
        let frame = cockpit(serde_json::json!([5, 5]), serde_json::json!([]));
        assert_eq!(
            map_rows(&frame, None, (20, 12)),
            vec![WAITING_FOR_THE_MAP.to_string()]
        );
        // A map of ANOTHER match is not this ground.
        let mut elsewhere = yard();
        elsewhere["match"] = serde_json::json!(4);
        assert_eq!(
            map_rows(&frame, Some(&elsewhere), (20, 12)),
            vec![WAITING_FOR_THE_MAP.to_string()]
        );

        let cleared = serde_json::json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "seated": false
        });
        assert_eq!(
            map_rows(&cleared, Some(&yard()), (20, 12)),
            vec![NOT_IN_A_COCKPIT.to_string()]
        );
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        for render in [
            render_cockpit_status_rows,
            render_cockpit_damage_rows,
            render_cockpit_weapons_rows,
            render_cockpit_contacts_rows,
        ] {
            assert!(
                render(&cleared, &ctx).rows.is_empty(),
                "a cleared frame empties every other pane"
            );
        }
    }

    /// 16b cut Q.4: the status, damage, weapons and contacts panes read the
    /// frame and nothing else — and a contact this platform has had a close
    /// look at carries its scan words, read through `scan_tables`.
    #[test]
    fn the_cockpit_panes_read_the_frame_and_the_scan_table() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let mut frame = cockpit(
            serde_json::json!([5, 5]),
            serde_json::json!([
                { "id": "1111111111111111",
                  "designator": "BA", "class": "blue", "friendly": false, "cell": [7, 7],
                  "bearing_deg": 45, "band": "effective", "range_m": 450, "age_s": 1,
                  "condition": { "table": 0, "bands": "gyr", "age_ms": 2000 } },
                contact("X2", "unknown", false, Value::Null)
            ]),
        );
        frame["blocked"] = serde_json::json!(true);
        frame["heat"] = serde_json::json!({ "heat": 90, "max": 100, "overheated": true });
        frame["lock_designator"] = serde_json::json!("BA");
        frame["lock_range_m"] = serde_json::json!(430);
        frame["lock_percent"] = serde_json::json!(62);
        frame["sections"] = serde_json::json!([
            { "id": "hull", "condition": "intact", "internals": 60, "internals_max": 60,
              "facets": [ { "id": "front", "armor": 55, "armor_max": 80 } ] },
            { "id": "turret", "condition": "destroyed", "internals": 0, "internals_max": 30,
              "facets": [] }
        ]);
        frame["weapons"] = serde_json::json!([
            { "id": "cannon", "ready": true, "ready_in_ms": 0, "rounds": 14, "rounds_max": 20,
              "percent": 62 },
            { "id": "machine_gun", "ready": false, "ready_in_ms": 900, "rounds": 380,
              "rounds_max": 500, "percent": null }
        ]);

        let status: Vec<String> = render_cockpit_status_rows(&frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(
            status,
            vec![
                "Callsign RA".to_string(),
                "power online  HELD".to_string(),
                "20 kph  heading 090\u{b0}".to_string(),
                "heat 90/100  OVERHEATED".to_string(),
                "scope hostile".to_string(),
                "Lock BA  430 m  62%".to_string(),
            ]
        );
        // A platform with no lock says so, and one with no heat has no row.
        let mut cold = frame.clone();
        cold["lock_designator"] = Value::Null;
        cold["heat"] = Value::Null;
        let status: Vec<String> = render_cockpit_status_rows(&cold, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(status.len(), 5);
        assert_eq!(status.last().map(String::as_str), Some("No lock"));

        let damage: Vec<String> = render_cockpit_damage_rows(&frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(
            damage,
            vec![
                "hull  intact  60/60  front 55/80".to_string(),
                "turret  destroyed  0/30".to_string(),
            ]
        );

        let weapons: Vec<String> = render_cockpit_weapons_rows(&frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(
            weapons,
            vec![
                "cannon  ready  14/20  62%".to_string(),
                "machine_gun  1 s  380/500".to_string(),
            ]
        );

        let contacts = render_cockpit_contacts_rows(&frame, &ctx);
        let rows: Vec<String> = contacts.rows.iter().map(PaneRow::text).collect();
        assert_eq!(
            rows,
            vec![
                "BA  blue  045\u{b0}  effective  450 m  1 s".to_string(),
                // The scan words, read through `scan_tables[0]`: green needs no
                // mark, and everything else carries its band letter so a
                // terminal with no colour loses nothing.
                "  engine hull/y turret/r ".to_string(),
                "X2  unknown  090\u{b0}  close  50 m  0 s".to_string(),
            ]
        );
        assert_eq!(
            contacts.rows.first().and_then(|r| r.prefill.clone()),
            Some("lock BA".to_string())
        );
        assert_eq!(
            contacts.rows.first().and_then(|r| r.row_key.clone()),
            Some("BA".to_string())
        );
        assert!(
            contacts.rows.get(1).is_some_and(|r| !r.selectable),
            "the scan row is a continuation, not a target"
        );
        // Nothing in any pane prints the contact's arena id — and both of this
        // frame's contacts carry one, so the needle has something behind it.
        for row in &rows {
            for id in ["1111111111111111", "2222222222222222"] {
                assert!(!row.contains(id), "the contacts pane leaked {id}: {row:?}");
            }
        }
    }

    /// **16b cut B.1** (`T-86`): the status pane prints the degraded receiver
    /// after the heat row and the pilot's own limit on the speed row, in the
    /// web's words and the web's places — and the committed fixture frame
    /// (`receiver: null`, neither flag set) prints neither, so the cockpit
    /// goldens do not move.
    #[test]
    fn the_status_pane_prints_the_receiver_and_the_tanks_own_limit() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let status = |frame: &Value| -> Vec<String> {
            render_cockpit_status_rows(frame, &ctx)
                .rows
                .iter()
                .map(PaneRow::text)
                .collect()
        };
        let mut frame = cockpit(serde_json::json!([5, 5]), serde_json::json!([]));
        frame["heat"] = serde_json::json!({ "heat": 40, "max": 100, "overheated": false });
        frame["receiver"] =
            serde_json::json!({ "budget": 15, "budget_max": 40, "reach_percent": 60 });
        let rows = status(&frame);
        assert_eq!(
            rows,
            vec![
                "Callsign RA".to_string(),
                "power online".to_string(),
                "20 kph  heading 090\u{b0}".to_string(),
                "heat 40/100".to_string(),
                "receiver 15/40  60%".to_string(),
                "scope hostile".to_string(),
                "No lock".to_string(),
            ]
        );
        assert!(
            rows.iter().all(|row| row.chars().count() <= 22),
            "a row would wrap the pane's 32 inner columns: {rows:?}"
        );

        // A wreck says DESTROYED (and not IMMOBILE, though it is that too); an
        // immobile tank says IMMOBILE.
        let mut wreck = frame.clone();
        wreck["wreck"] = serde_json::json!(true);
        wreck["immobile"] = serde_json::json!(true);
        assert_eq!(
            status(&wreck).get(2).map(String::as_str),
            Some("20 kph  heading 090\u{b0}  DESTROYED")
        );
        let mut stuck = frame.clone();
        stuck["immobile"] = serde_json::json!(true);
        assert_eq!(
            status(&stuck).get(2).map(String::as_str),
            Some("20 kph  heading 090\u{b0}  IMMOBILE")
        );

        // The committed fixture frame: neither row.
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .unwrap_or(Value::Null);
        let rows = status(&fixture);
        assert!(!rows.is_empty(), "the fixture frame is seated");
        for row in &rows {
            for word in ["receiver", "DESTROYED", "IMMOBILE"] {
                assert!(!row.contains(word), "the fixture printed {word}: {rows:?}");
            }
        }
    }

    /// **16b cut O.2** (16c N12): the pod's switch rides the scope row —
    /// `scope hostile  EMCON on` / `  EMCON off` — and a platform with no pod
    /// (`ecm` null, or a frame from before the key existed) prints the row
    /// unchanged. No eighth row, and every row inside the pane's 32 columns at
    /// the longest scope word.
    #[test]
    fn o2_the_scope_row_carries_the_pods_switch() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let status = |frame: &Value| -> Vec<String> {
            render_cockpit_status_rows(frame, &ctx)
                .rows
                .iter()
                .map(PaneRow::text)
                .collect()
        };
        let mut frame = cockpit(serde_json::json!([5, 5]), serde_json::json!([]));
        let plain = status(&frame);
        assert!(plain.contains(&"scope hostile".to_string()), "{plain:?}");
        for (ecm, row) in [
            (serde_json::json!("on"), "scope hostile  EMCON on"),
            (serde_json::json!("off"), "scope hostile  EMCON off"),
            (Value::Null, "scope hostile"),
        ] {
            frame["ecm"] = ecm.clone();
            let rows = status(&frame);
            assert_eq!(rows.len(), plain.len(), "{ecm}: no row added");
            assert!(rows.contains(&row.to_string()), "{ecm}: {rows:?}");
        }
        // An absent key is null.
        if let Some(object) = frame.as_object_mut() {
            object.remove("ecm");
        }
        assert_eq!(status(&frame), plain);
        // The longest scope word with the longer suffix still fits.
        frame["ecm"] = serde_json::json!("off");
        frame["scope"] = serde_json::json!("unidentified");
        let rows = status(&frame);
        assert!(
            rows.contains(&"scope unidentified  EMCON off".to_string()),
            "{rows:?}"
        );
        assert!(
            rows.iter().all(|row| row.chars().count() <= 32),
            "a row would wrap the pane's 32 inner columns: {rows:?}"
        );
        // A word the client does not know adds nothing.
        frame["ecm"] = serde_json::json!("sideways");
        assert!(status(&frame).contains(&"scope unidentified".to_string()));
    }

    /// 16b cut Q.4: the degree mark degrades with every other non-ASCII glyph
    /// on this surface.
    ///
    /// The TUI paints cells. A terminal that told us its charset is not UTF-8
    /// (`LANG=C`) would render `\u{b0}`'s two bytes as two glyphs and every
    /// column after it — the pane's own right border included — would sit one
    /// to the right for the rest of the frame.
    #[test]
    fn nothing_on_the_cockpit_is_non_ascii_without_utf8() {
        let theme = AnsiTheme::classic();
        let map = yard();
        let mut store = PanelStore::new();
        store.apply_panel(
            &mush_proto::SmolStr::new_static("arena.map"),
            mush_proto::PanelFrameKind::Keyframe,
            &map,
            0,
        );
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::None,
            utf8: false,
            store: &store,
            selected_channel: None,
            pane: Some((20, 12)),
            picture: None,
        };
        let frame = cockpit(
            serde_json::json!([20, 15]),
            serde_json::json!([contact("X2", "unknown", false, serde_json::json!([18, 15]))]),
        );
        let mut rows: Vec<String> = render_cockpit_map_rows(&frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect();
        assert_eq!(
            rows.first().map(String::as_str),
            Some("Map - heading 090"),
            "the em dash AND the degree mark degrade: {rows:?}"
        );
        for render in [
            render_cockpit_status_rows,
            render_cockpit_damage_rows,
            render_cockpit_weapons_rows,
            render_cockpit_contacts_rows,
        ] {
            rows.extend(render(&frame, &ctx).rows.iter().map(PaneRow::text));
        }
        assert!(
            rows.iter().any(|r| r.contains("heading 090")),
            "the status row painted: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.contains("X2  unknown  090  close")),
            "the contacts row painted: {rows:?}"
        );
        for row in &rows {
            assert!(
                row.is_ascii(),
                "a terminal with no UTF-8 is sent ASCII only: {row:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 16b cut B.3: the legend, the chrome and the notices ring
    // -----------------------------------------------------------------------

    /// **16b cut B.3** (`B-62`): two contacts on one cell — the legend names
    /// the LATER, whose glyph is the one drawn, and keeps frame order for the
    /// rest; a contact on the pilot's own cell owns it (the `@` went first).
    #[test]
    fn the_legend_names_the_contact_that_owns_its_cell() {
        let map = yard();
        let frame = cockpit(
            serde_json::json!([20, 15]),
            serde_json::json!([
                contact("BA", "blue", false, serde_json::json!([18, 15])),
                contact("X3", "unknown", false, serde_json::json!([21, 16])),
                contact("RB", "red", true, serde_json::json!([18, 15])),
                contact("X5", "unknown", false, serde_json::json!([20, 15])),
            ]),
        );
        let rows = map_rows(&frame, Some(&map), (10, 12));
        assert_eq!(rows.get(5).and_then(|r| r.chars().nth(3)), Some('+'));
        assert_eq!(
            rows.get(5).and_then(|r| r.chars().nth(5)),
            Some('?'),
            "a contact on the pilot's cell overwrites the `@`"
        );
        // The legend, 16 columns, read off a pane wide enough to carry it
        // whole (a narrower one cuts it to the pane: the next test).
        let rows = map_rows(&frame, Some(&map), (20, 12));
        assert_eq!(
            rows.last().map(String::as_str),
            Some("? X3  + RB  ? X5"),
            "BA's cell is RB's; X5 owns the `@`'s: {rows:?}"
        );
    }

    /// **16b cut B.3** (`B-65`): the map pane sheds its heading, then its
    /// legend, before it gives up a row of ground — and renders exactly its
    /// height every time.
    #[test]
    fn the_map_sheds_its_chrome_at_four_three_and_two_rows() {
        let map = yard();
        let frame = cockpit(
            serde_json::json!([20, 15]),
            serde_json::json!([contact("BA", "blue", false, serde_json::json!([21, 15]))]),
        );
        // Four rows: the heading, two rows of ground, the legend.
        let rows = map_rows(&frame, Some(&map), (10, 4));
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(
            rows.first().is_some_and(|r| r.starts_with("Map ")),
            "{rows:?}"
        );
        assert_eq!(rows.last().map(String::as_str), Some("x BA"));
        // Three: the heading goes, two rows of ground and the legend stay.
        let rows = map_rows(&frame, Some(&map), (10, 3));
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert!(!rows.iter().any(|r| r.starts_with("Map ")), "{rows:?}");
        assert_eq!(rows.last().map(String::as_str), Some("x BA"));
        assert!(rows.iter().take(2).all(|r| r.chars().count() == 10));
        // Two: ground only.
        let rows = map_rows(&frame, Some(&map), (10, 2));
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().all(|r| r.chars().count() == 10), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains('@')), "{rows:?}");
        // One: one row of ground, the pilot's own.
        let rows = map_rows(&frame, Some(&map), (10, 1));
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows.first().is_some_and(|r| r.contains('@')), "{rows:?}");
    }

    /// **16b cut B.3** (`B-65`): a legend wider than the pane is cut to it —
    /// whole entries in frame order, then how many were left off — so it is
    /// one line, never wrapped below the frame, and the map pane never has a
    /// line more than its height to scroll.
    #[test]
    fn a_crowded_legend_is_cut_to_the_pane_and_counts_the_rest() {
        let map = yard();
        let crowd: Vec<Value> = (0..7)
            .map(|i| {
                contact(
                    &format!("X{}", i + 1),
                    "unknown",
                    false,
                    serde_json::json!([16 + i, 17]),
                )
            })
            .collect();
        let frame = cockpit(serde_json::json!([20, 15]), Value::Array(crowd));
        // Seven entries are 4 + 6 × 6 = 40 columns; the pane is 20.
        let rows = map_rows(&frame, Some(&map), (20, 8));
        assert_eq!(rows.len(), 8, "{rows:?}");
        let legend = rows.last().cloned().unwrap_or_default();
        assert_eq!(legend, "? X1  ? X2  5 more");
        assert!(legend.chars().count() <= 20);
        // Wide enough, it is whole.
        let rows = map_rows(&frame, Some(&map), (40, 8));
        assert_eq!(
            rows.last().map(String::as_str),
            Some("? X1  ? X2  ? X3  ? X4  ? X5  ? X6  ? X7")
        );
        // Too narrow even for a count: nothing, rather than a wrapped word.
        assert_eq!(fit_legend(&["? X1".to_string()], Some(3)), "");
        assert_eq!(fit_legend(&["? X1".to_string()], None), "? X1");
    }

    /// **16b cut B.3** (`B-64`): `notice_words` IS the web's `noticeLine`, pair
    /// for pair, against the fixture B.1 committed for both clients — so the two
    /// panes' wording cannot drift.
    #[test]
    fn notice_words_matches_the_committed_pairs() {
        let pairs: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/notice_lines.json"
        ))
        .expect("notice_lines.json is a JSON array");
        assert!(pairs.len() >= 18, "the fixture has every shape B.1 pinned");
        // 17b cut M1b.4: and its five slot cases — a shot with a designator and
        // one without, an odds and an aspect held shot, and an unlawful refusal
        // with its floor.
        for kind in ["slot_fired", "slot_held"] {
            assert!(
                pairs.iter().any(|pair| pair["notice"]["kind"] == kind),
                "the fixture carries a {kind}"
            );
        }
        assert!(
            pairs
                .iter()
                .any(|pair| pair["notice"]["refusal"] == "unlawful"),
            "the fixture carries an unlawful refusal"
        );
        for pair in &pairs {
            let notice = pair.get("notice").expect("a notice");
            let line = pair.get("line").and_then(Value::as_str).expect("a line");
            assert_eq!(notice_words(notice), line, "{notice}");
        }
    }

    /// **17b cut M1b.4**: a slot notice's ring line is the line-mode sentence
    /// less its full stop, for every why the committed fixture does not carry
    /// — the range's three shapes and the rounds — and the friendly mark on a
    /// shot; a slot notice missing a part falls to its kind's words, and an
    /// `unlawful` refusal with no floor reads the generic refusal.
    #[test]
    fn m1b4_slot_words_say_every_why() {
        let held = |detail: Value| {
            serde_json::json!({
                "kind": "slot_held", "slot": 1, "weapon": "cannon", "designator": "BA",
                "friendly": false, "detail": detail
            })
        };
        let range = |min: i64, max: i64| {
            held(serde_json::json!({
                "why": "range", "percent": null, "floor": null, "aspect": null,
                "range_m": 650, "min_m": min, "max_m": max, "rounds": null, "keeps": null
            }))
        };
        assert_eq!(
            notice_words(&range(200, 600)),
            "S1 held the cannon: BA at 650 m, outside 200 to 600"
        );
        assert_eq!(
            notice_words(&range(0, 600)),
            "S1 held the cannon: BA at 650 m, outside under 600"
        );
        assert_eq!(
            notice_words(&range(200, i64::from(u32::MAX))),
            "S1 held the cannon: BA at 650 m, outside past 200"
        );
        assert_eq!(
            notice_words(&held(serde_json::json!({
                "why": "rounds", "percent": null, "floor": null, "aspect": null,
                "range_m": null, "min_m": null, "max_m": null, "rounds": 7, "keeps": 8
            }))),
            "S1 held the cannon: 7 rounds, keeps 8"
        );
        let friendly = serde_json::json!({
            "kind": "slot_fired", "slot": 2, "weapon": "machine_gun", "designator": "RB",
            "friendly": true, "percent": 95, "aspect": "right", "detail": null
        });
        assert_eq!(
            notice_words(&friendly),
            "Machine_gun fires at RB (FRIENDLY) by S2 (95%, side)"
        );
        // A part missing: the kind's own words.
        let bare = serde_json::json!({ "kind": "slot_fired", "weapon": "cannon" });
        assert_eq!(notice_words(&bare), "fires by");
        let why_less = serde_json::json!({ "kind": "slot_held", "slot": 1, "weapon": "cannon" });
        assert_eq!(notice_words(&why_less), "held");
        let floorless = serde_json::json!({
            "kind": "refused", "order": "fire", "weapon": "cannon", "refusal": "unlawful",
            "detail": null
        });
        assert_eq!(
            notice_words(&floorless),
            "FIRE cannon refused: that rule is not lawful for this unit"
        );
        let mut under_floor = floorless;
        under_floor["detail"] = serde_json::json!({ "floor_permille": 100 });
        assert_eq!(
            notice_words(&under_floor),
            "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less"
        );
    }

    /// **16b cut Y.4**: `catchup_words` IS the server's `catchup_lines` and the
    /// web's `catchupLines`, case for case, against the one hand-written
    /// fixture all three read — so a reclaimed pilot reads one report on every
    /// surface.
    #[test]
    fn catchup_words_match_the_committed_cases() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
        ))
        .expect("catchup_lines.json is a JSON array");
        // Was 9 until 17b cut M5.3, which added the tenth (D14).
        assert_eq!(cases.len(), 10, "the ten shapes the fixture pins");
        for case in &cases {
            let callsign = case
                .get("callsign")
                .and_then(Value::as_str)
                .expect("a callsign");
            let catchup = case.get("catchup").expect("a catchup");
            let lines: Vec<&str> = case
                .get("lines")
                .and_then(Value::as_array)
                .expect("lines")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert_eq!(catchup_words(callsign, catchup), lines, "{catchup}");
        }
    }

    /// **17b cut M5.3 (D14)**: the machine's `unknown` is said as the mind's
    /// own — `  The machine was flying on its own mind under no order.` — the
    /// fixture's tenth case, in parity with the server's in-source test and
    /// vitest of the same name; under every order it is the same phrase, and
    /// the six doctrine states keep their words.
    #[test]
    fn m5_d14_the_catch_up_says_its_own_mind_in_three_renderers() {
        const D14: &str = "  The machine was flying on its own mind under no order.";
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
        ))
        .expect("catchup_lines.json is a JSON array");
        let said: Vec<&Value> = cases
            .iter()
            .filter(|case| case.pointer("/catchup/machine/state") == Some(&Value::from("unknown")))
            .collect();
        assert_eq!(said.len(), 1, "one case of the fixture is D14's");
        let catchup = said[0].get("catchup").expect("a catchup");
        let lines = catchup_words("RA", catchup);
        assert_eq!(lines.get(1).map(String::as_str), Some(D14), "{lines:?}");
        assert!(!lines.iter().any(|line| line.contains("unknown under")));
        for objective in ["hold", "advance", "engage", "withdraw", "screen", "support"] {
            let mut ordered = catchup.clone();
            ordered["machine"]["objective"] = Value::from(objective);
            let line = catchup_words("RA", &ordered).swap_remove(1);
            assert!(
                line.starts_with("  The machine was flying on its own mind under a"),
                "{line}"
            );
            assert!(line.chars().count() < 120, "{line}");
        }
        for state in [
            "startup", "patrol", "approach", "engage", "withdraw", "disabled",
        ] {
            let mut doctrine = catchup.clone();
            doctrine["machine"]["state"] = Value::from(state);
            let line = catchup_words("RA", &doctrine).swap_remove(1);
            assert!(!line.contains("mind"), "{state}: {line}");
        }
    }

    /// **16b cut Y.4**: a `catchup` that is not an object says nothing, a
    /// malformed entry is dropped, and every string is cleaned and cut.
    #[test]
    fn catchup_words_refuse_a_malformed_report_and_bound_every_field() {
        for bad in [
            serde_json::json!("Back aboard"),
            serde_json::json!(7),
            Value::Null,
            serde_json::json!([]),
        ] {
            assert!(catchup_words("RA", &bad).is_empty(), "{bad}");
        }
        let hostile = serde_json::json!({
            "away_s": 61, "cell_then": [1, -2], "cell_now": ["x", 3],
            "power_now": "on\u{1b}[31mline", "wreck": "yes",
            "contacts_new": ["ABCDEFGHIJ", 3, "  "], "contacts_lost": [],
            "contacts_more": 0,
            "sections": [{ "id": "s".repeat(30), "then": { "condition": "intact", "armour": 9 },
                           "now": { "condition": "intact", "armour": 4 } }, "junk"],
            "rounds_spent": [{ "weapon": "w\u{7}", "rounds": 2 }, { "weapon": "x" }],
            "machine": { "state": "a_new_state_word_x", "objective": null }
        });
        assert_eq!(
            catchup_words("RA", &hostile),
            vec![
                "Back aboard RA after 1 m 1 s away.".to_string(),
                "  The machine was a new state word under no order.".to_string(),
                "  Position: unknown.".to_string(),
                "  Contacts: new ABCDEFGH; none lost.".to_string(),
                format!("  Sections: {} armour 9 then, 4 now.", "s".repeat(24)),
                "  Rounds spent: w 2.".to_string(),
                "  Power on[31mline.".to_string(),
            ]
        );
    }

    /// **16b cut B.3**: the three word tables are the web's, entry for entry
    /// and in order — read out of `cockpit.ts` itself, because the parity
    /// fixture exercises only some of the words, and a word reworded on one
    /// client alone would otherwise pass every test.
    #[test]
    fn the_notice_word_tables_are_the_web_s() {
        let web = include_str!("../../../../web/src/lib/cockpit.ts");
        let table = |name: &str| -> Vec<(String, String)> {
            let head = format!("const {name}: ReadonlyMap<string, string> = new Map([");
            let start = web
                .find(&head)
                .unwrap_or_else(|| panic!("{name} in cockpit.ts"));
            let body = web.get(start + head.len()..).unwrap_or_default();
            let body = body
                .get(..body.find("]);").unwrap_or(0))
                .unwrap_or_default();
            body.lines()
                .map(str::trim)
                .filter(|l| l.starts_with("[\""))
                .map(|l| {
                    let inner = l.trim_start_matches("[\"").trim_end_matches("\"],");
                    let (wire, said) = inner.split_once("\", \"").expect("a pair");
                    (wire.to_string(), said.to_string())
                })
                .collect()
        };
        for (name, tui) in [
            ("NOTICE_WORDS", NOTICE_WORDS),
            ("REFUSAL_WORDS", REFUSAL_WORDS),
            ("CAUSE_WORDS", CAUSE_WORDS),
        ] {
            let ours: Vec<(String, String)> = tui
                .iter()
                .map(|(w, s)| ((*w).to_string(), (*s).to_string()))
                .collect();
            assert_eq!(ours, table(name), "{name} has drifted from the web's");
        }
    }

    /// **16b cut B.3**: every string read off a notice is cleaned and bounded,
    /// exactly as the web's decoder does — a control byte never reaches the
    /// pane, a designator is at most eight characters and a section 24, and a
    /// notice with no string kind says nothing.
    #[test]
    fn notice_words_cleans_and_bounds_every_field() {
        let hit = serde_json::json!({
            "kind": "hit", "designator": "B\u{1b}A\u{7}", "friendly": false,
            "section": "s".repeat(40), "spent": false
        });
        assert_eq!(notice_words(&hit), format!("BA hit: {}", "s".repeat(24)));
        let long = serde_json::json!({ "kind": "acquired", "designator": "ABCDEFGHIJKL" });
        assert_eq!(notice_words(&long), "ABCDEFGH new contact");
        let refused = serde_json::json!({
            "kind": "refused", "order": "fire\n", "weapon": "w".repeat(30),
            "refusal": "a_new_reason", "detail": { "heat": 7, "max": 9 }
        });
        assert_eq!(
            notice_words(&refused),
            format!("FIRE {} refused: a new reason, 7 of 9", "w".repeat(24))
        );
        let was = serde_json::json!({
            "kind": "classified", "designator": "BA", "was": "X2\u{1b}[2J",
            "friendly": false
        });
        assert_eq!(notice_words(&was), "X2[2J identified: BA");
        // An empty designator reads as none; a missing kind is no line at all.
        let bare = serde_json::json!({ "kind": "lost", "designator": "  " });
        assert_eq!(notice_words(&bare), "contact lost");
        assert_eq!(notice_words(&serde_json::json!({ "kind": 3 })), "");
        assert_eq!(notice_words(&Value::Null), "");
    }

    /// **16b cut B.3**: the notices pane is the ring, oldest first; the TAIL
    /// that fits once wrapped when the pane's size is known; `Nothing yet.`
    /// while seated with nothing rung; and the one empty line on a cleared
    /// frame.
    #[test]
    fn the_notices_pane_answers_the_tail_of_the_ring() {
        let theme = AnsiTheme::classic();
        let mut store = PanelStore::new();
        let feed = mush_proto::SmolStr::new_static("arena.cockpit");
        let frame = cockpit(serde_json::json!([5, 5]), serde_json::json!([]));
        store.apply_panel(&feed, mush_proto::PanelFrameKind::Keyframe, &frame, 0);
        let rows = |store: &PanelStore, pane: Option<(u16, u16)>, data: &Value| -> Vec<String> {
            let ctx = RenderCtx {
                theme: &theme,
                depth: ColorDepth::Ansi16,
                utf8: true,
                store,
                selected_channel: None,
                pane,
                picture: None,
            };
            render_cockpit_notices_rows(data, &ctx)
                .rows
                .iter()
                .map(PaneRow::text)
                .collect()
        };
        assert_eq!(rows(&store, Some((20, 5)), &frame), [NOTHING_YET]);
        for line in [
            "one",
            "two",
            "a reply long enough to wrap twice here",
            "four",
        ] {
            store.push_cockpit_reply(line);
        }
        assert_eq!(
            rows(&store, None, &frame),
            [
                "one",
                "two",
                "a reply long enough to wrap twice here",
                "four"
            ]
        );
        // Twenty columns: the long reply is two lines, so four rows hold it
        // and the two after it — `one` is the line that goes.
        assert_eq!(
            rows(&store, Some((20, 4)), &frame),
            ["two", "a reply long enough to wrap twice here", "four"]
        );
        assert_eq!(rows(&store, Some((20, 1)), &frame), ["four"]);
        let cleared = serde_json::json!({ "seated": false });
        assert_eq!(rows(&store, Some((20, 4)), &cleared), [NOT_IN_A_COCKPIT]);
    }

    /// **17b cut M1a.3** (doc 17 §3; departure (k)): the contacts pane's compact
    /// tail in its two shapes — ` {side} {NN}%` off the first `odds` entry when
    /// the array is not empty, ` {side}` when it is or is absent — with `left`
    /// and `right` printed as `side`; and a contact from an older server, with
    /// no `aspect`, prints the row it always printed.
    #[test]
    fn m1a3_a_contact_row_ends_in_the_compact_aspect_and_odds_tail() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let contact = |designator: &str, aspect: Option<&str>, odds: Option<Value>| {
            let mut row = serde_json::json!({
                "designator": designator, "class": "blue", "bearing_deg": 45,
                "band": "far", "range_m": 450, "age_s": 1, "condition": null,
            });
            if let Some(aspect) = aspect {
                row["aspect"] = serde_json::json!(aspect);
            }
            if let Some(odds) = odds {
                row["odds"] = odds;
            }
            row
        };
        let data = serde_json::json!({
            "contacts": [
                contact("BA", Some("left"), Some(serde_json::json!([
                    { "weapon": "cannon", "p_permille": 624, "percent": 62 },
                    { "weapon": "machine_gun", "p_permille": 210, "percent": 21 },
                ]))),
                contact("BB", Some("rear"), Some(serde_json::json!([]))),
                contact("BC", Some("right"), None),
                contact("BD", None, None),
            ],
            "scan_tables": [],
        });
        let rows = render_cockpit_contacts_rows(&data, &ctx);
        let text: Vec<String> = rows.rows.iter().map(|row| line_text(&row.line)).collect();
        assert_eq!(
            text,
            vec![
                "BA  blue  045°  far  450 m  1 s side 62%".to_string(),
                "BB  blue  045°  far  450 m  1 s rear".to_string(),
                "BC  blue  045°  far  450 m  1 s side".to_string(),
                "BD  blue  045°  far  450 m  1 s".to_string(),
            ]
        );
    }

    /// **17b cut M1a.3** (doc 17 §22's italic line): an unarmed hull's frame
    /// carries no `weapons` key and no contact carries `odds`; the weapons pane
    /// says `(no weapon)` as it did for an empty list, and every contacts row
    /// ends at the side word.
    #[test]
    fn m1a3_a_frame_without_weapons_renders() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let data = serde_json::json!({
            "contacts": [{
                "designator": "X3", "class": "unknown", "bearing_deg": 315,
                "band": "beyond", "range_m": 200, "age_s": 3, "condition": null,
                "aspect": "front", "fatal": null,
            }],
            "scan_tables": [],
        });
        let weapons = render_cockpit_weapons_rows(&data, &ctx);
        let text: Vec<String> = weapons
            .rows
            .iter()
            .map(|row| line_text(&row.line))
            .collect();
        assert_eq!(text, vec!["(no weapon)".to_string()]);
        let contacts = render_cockpit_contacts_rows(&data, &ctx);
        let text: Vec<String> = contacts
            .rows
            .iter()
            .map(|row| line_text(&row.line))
            .collect();
        assert_eq!(
            text,
            vec!["X3  unknown  315°  beyond  200 m  3 s front".to_string()]
        );
    }

    /// **17b cut M1b.3** (doc 17 §5; D-UM-4): under an armed weapon the pane
    /// prints ONE more row, two spaces and the arming line — the TUI's copy of
    /// `arming_words`, over every target, facet, range and reserve shape; a
    /// weapon with `armed: null`, and a frame from an older server with no
    /// `armed` key, print as they always did; a malformed object costs a row
    /// and never a word, and a key the server never sends (a designator, a
    /// contact id) never reaches the pane; a row past the pane's 46 columns
    /// wraps by the pane's own rule (`wrap_spans`, as `draw_pane` runs it).
    #[test]
    fn m1b3_an_armed_weapon_prints_its_arming_line_under_its_row() {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        let every = serde_json::json!(["front", "left", "right", "rear"]);
        let armed =
            |target: &str, percent: u64, aspects: Value, keeps: u64, near: Value, far: Value| {
                serde_json::json!({
                    "slot": 1, "target": target, "min_permille": percent * 10,
                    "percent": percent, "aspects": aspects, "keeps": keeps,
                    "min_range_m": near, "max_range_m": far,
                })
            };
        let line = |armed: &Value| arming_line(armed).unwrap_or_default();
        // The fixture scene's, the bounded one, and T1's.
        assert_eq!(
            line(&armed(
                "lock",
                60,
                serde_json::json!(["left", "right", "rear"]),
                8,
                Value::Null,
                Value::Null
            )),
            "S1: >=60% side/rear keeps 8"
        );
        assert_eq!(
            line(&armed(
                "lock",
                60,
                serde_json::json!(["left", "right", "rear"]),
                8,
                serde_json::json!(200),
                serde_json::json!(600)
            )),
            "S1: >=60% side/rear 200-600m keeps 8"
        );
        assert_eq!(
            line(&armed(
                "best",
                10,
                every.clone(),
                0,
                Value::Null,
                serde_json::json!(420)
            )),
            "S1: best >=10% under 420m"
        );
        // Every other shape of each term.
        assert_eq!(
            line(&armed(
                "contact",
                60,
                every.clone(),
                0,
                Value::Null,
                Value::Null
            )),
            "S1: contact >=60%"
        );
        assert_eq!(
            line(&armed(
                "lock",
                60,
                every.clone(),
                0,
                serde_json::json!(300),
                Value::Null
            )),
            "S1: >=60% past 300m"
        );
        assert_eq!(
            line(&armed(
                "lock",
                60,
                every.clone(),
                0,
                serde_json::json!(0),
                serde_json::json!(500)
            )),
            "S1: >=60% under 500m",
            "a zero lower bound is no bound"
        );
        for (aspects, words) in [
            (serde_json::json!(["front"]), "S1: >=60% front"),
            (serde_json::json!(["right"]), "S1: >=60% side"),
            (serde_json::json!(["rear"]), "S1: >=60% rear"),
            (
                serde_json::json!(["front", "left", "rear"]),
                "S1: >=60% front/side/rear",
            ),
            (serde_json::json!([]), "S1: >=60%"),
            (every.clone(), "S1: >=60%"),
        ] {
            assert_eq!(
                line(&armed("lock", 60, aspects, 0, Value::Null, Value::Null)),
                words
            );
        }
        // Not an object, no slot: no row.
        assert_eq!(arming_line(&Value::Null), None);
        assert_eq!(arming_line(&serde_json::json!("S1")), None);
        assert_eq!(arming_line(&serde_json::json!({ "target": "best" })), None);
        // A key the server never sends does not reach the line.
        let stray = serde_json::json!({
            "slot": 2, "target": "lock", "percent": 45, "aspects": ["front"],
            "keeps": 0, "min_range_m": null, "max_range_m": null,
            "designator": "BA", "contact": "00000000000000ab",
        });
        assert_eq!(line(&stray), "S2: >=45% front");

        // The pane: the row, then the arming row; a null arming and a frame
        // from an older server print the one row.
        let gun = |id: &str, armed: Option<Value>| {
            let mut row = serde_json::json!({
                "id": id, "ready": true, "ready_in_ms": 0, "rounds": 14,
                "rounds_max": 20, "percent": 62,
            });
            if let (Some(object), Some(armed)) = (row.as_object_mut(), armed) {
                object.insert("armed".to_string(), armed);
            }
            row
        };
        let frame = serde_json::json!({
            "weapons": [
                gun("cannon", Some(armed("lock", 60, serde_json::json!(["left", "right", "rear"]), 8, Value::Null, Value::Null))),
                gun("machine_gun", Some(Value::Null)),
                gun("older", None),
            ],
        });
        let rows = render_cockpit_weapons_rows(&frame, &ctx);
        let text: Vec<String> = rows.rows.iter().map(PaneRow::text).collect();
        assert_eq!(
            text,
            vec![
                "cannon  ready  14/20  62%".to_string(),
                "  S1: >=60% side/rear keeps 8".to_string(),
                "machine_gun  ready  14/20  62%".to_string(),
                "older  ready  14/20  62%".to_string(),
            ]
        );
        // The longest arming a slot can carry is past 46 columns: the pane's
        // own rule wraps it onto a second line and loses no word.
        let long = armed(
            "contact",
            100,
            serde_json::json!(["front", "left", "rear"]),
            4_294_967_295,
            serde_json::json!(4_294_967_294_u64),
            serde_json::json!(4_294_967_294_u64),
        );
        let frame = serde_json::json!({ "weapons": [gun("cannon", Some(long))] });
        let rows = render_cockpit_weapons_rows(&frame, &ctx);
        let arming = rows.rows.get(1).expect("the arming row");
        let printed = PaneRow::text(arming);
        assert!(printed.chars().count() > 46, "{printed}");
        let wrapped: Vec<String> = wrap_spans(&arming.line.spans, 46)
            .into_iter()
            .map(|spans| spans.iter().map(|span| span.content.to_string()).collect())
            .collect();
        assert_eq!(wrapped.len(), 2, "{wrapped:?}");
        assert!(
            wrapped.iter().all(|row| row.chars().count() <= 46),
            "{wrapped:?}"
        );
        assert_eq!(
            wrapped.join(" ").split_whitespace().collect::<Vec<_>>(),
            printed.split_whitespace().collect::<Vec<_>>(),
            "no word lost to the wrap"
        );
    }

    /// The `mind` object of the canonical scene (17b cut M5.4's
    /// `cockpit_frame.json`): Standard Doctrine sealed on Sniper and Line,
    /// Sniper flown, thinking at Class I on a Class II core, an armed hull.
    fn scene_mind() -> Value {
        serde_json::json!({
            "name": "Standard Doctrine", "profiles": ["Sniper", "Line"], "active": 0,
            "class": "I", "core": "II", "channels": ["helm", "sensors", "fire"],
            "next_switch_ms": null, "off": false,
        })
    }

    /// The text of the weapons pane's rows for `frame`.
    fn weapons_text(frame: &Value, utf8: bool) -> Vec<String> {
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8,
            store: &store,
            selected_channel: None,
            pane: Some((44, 7)),
            picture: None,
        };
        render_cockpit_weapons_rows(frame, &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect()
    }

    #[test]
    fn ordered_rules_show_every_slot_at_the_pane_floor_even_on_an_unarmed_hull() {
        let row = |slot, action| serde_json::json!({ "slot": slot, "rule": { "action": action } });
        let frame = serde_json::json!({
            "rules": [row(6, "fire"), row(1, "fire"), row(2, "hold_fire"),
                row(3, "lock"), row(4, "fire"), row(5, "hold_fire")],
        });
        let summary = [
            "rules: S1 fire S2 hold S3 lock",
            "S4 fire S5 hold S6 fire; MIND RULES",
        ];
        assert_eq!(rule_summary_lines(&frame), summary);
        assert!(
            rule_summary_lines(&frame)
                .iter()
                .all(|row| row.chars().count() <= 44)
        );
        for utf8 in [true, false] {
            assert_eq!(
                weapons_text(&frame, utf8),
                [summary[0], summary[1], "(no weapon)"]
            );
        }
        let sparse = serde_json::json!({
            "rules": [row(1, "fire"), row(3, "hold_fire"), row(6, "lock")],
            "mind": scene_mind(),
        });
        assert_eq!(
            weapons_text(&sparse, true)[0],
            "rules: S1 fire S3 hold S6 lock; MIND RULES"
        );
        assert_eq!(weapons_text(&sparse, true).len(), 5);
        assert!(weapons_text(&sparse, true)[0].chars().count() <= 44);
        let mut cleared = frame;
        cleared["seated"] = Value::Bool(false);
        assert!(weapons_text(&cleared, true).is_empty());
    }

    #[test]
    fn ordered_rule_summaries_drop_invalid_slots_and_duplicate_or_unknown_actions() {
        let row = |slot, action| serde_json::json!({ "slot": slot, "rule": { "action": action } });
        let frame = serde_json::json!({
            "rules": [Value::Null, {}, row(0, "fire"), row(7, "fire"),
                { "slot": 1.5, "rule": { "action": "fire" } },
                row(1, "unknown"), row(2, "hold"), row(1, "fire"), row(1, "lock"),
                { "slot": 3, "rule": { "action": "lock", "contact": "ffffffffffffffff" },
                    "text": "\u{1b}hidden" }],
        });
        assert_eq!(
            rule_summary_lines(&frame),
            ["rules: S1 fire S3 lock; MIND RULES"]
        );
        for rules in [Value::Null, serde_json::json!({}), serde_json::json!([])] {
            assert!(rule_summary_lines(&serde_json::json!({ "rules": rules })).is_empty());
        }
    }

    #[test]
    fn canonical_rules_replace_the_incomplete_first_fire_convenience_line() {
        let weapon = serde_json::json!({
            "id": "cannon", "ready": true, "rounds": 14, "rounds_max": 20,
            "armed": { "slot": 1, "target": "lock", "min_permille": 100,
                "percent": 10, "aspects": [], "keeps": 0 },
        });
        let current = serde_json::json!({
            "weapons": [weapon.clone()],
            "rules": [{ "slot": 1, "rule": { "action": "fire" } }],
        });
        assert_eq!(
            weapons_text(&current, true),
            ["rules: S1 fire; MIND RULES", "cannon  ready  14/20"]
        );
        let older = serde_json::json!({ "weapons": [weapon] });
        assert_eq!(weapons_text(&older, true).len(), 2);
        assert!(weapons_text(&older, true)[1].contains("S1:"));
    }

    /// **17b cut M5.5** (doc 17 §12; the M5 design note's M5.5 item 1): the
    /// mind's four rows under the weapon rows, in their order and words; the
    /// readings each row has; on an unarmed hull the four rows in `(no
    /// weapon)`'s place; and nothing where the frame carries no `mind`
    /// object — `null`, absent, malformed, or a cleared frame. No row is
    /// selectable, and none says a countdown: a switched profile's
    /// `next_switch_ms` draws nothing.
    #[test]
    fn m5_the_mind_tail_draws_four_rows_under_the_weapons() {
        let gun = serde_json::json!({
            "id": "cannon", "ready": true, "ready_in_ms": 0, "rounds": 14,
            "rounds_max": 20, "percent": 62, "armed": null,
        });
        let armed = serde_json::json!({ "weapons": [gun], "mind": scene_mind() });
        assert_eq!(
            weapons_text(&armed, true),
            vec![
                "cannon  ready  14/20  62%".to_string(),
                "mind Standard Doctrine \u{b7} Class I".to_string(),
                "> Sniper   Line".to_string(),
                "dormant while you fly".to_string(),
                "you: helm sensors fire".to_string(),
            ]
        );
        // A terminal that is not UTF-8 gets the ASCII separator.
        assert_eq!(
            weapons_text(&armed, false).get(1).map(String::as_str),
            Some("mind Standard Doctrine | Class I")
        );
        // Every reading: the other profile flown, inside its switch window
        // (no countdown drawn), a seal that is not Standard Doctrine, a
        // degraded core's class read off the frame, a hull dropped to the ROM.
        let mut other = armed.clone();
        other["mind"]["active"] = serde_json::json!(1);
        other["mind"]["next_switch_ms"] = serde_json::json!(40_000);
        other["mind"]["name"] = Value::Null;
        other["mind"]["class"] = serde_json::json!("0");
        other["mind"]["off"] = serde_json::json!(true);
        let rows = weapons_text(&other, true);
        assert_eq!(
            rows[1..],
            [
                "mind: your own \u{b7} Class 0".to_string(),
                "  Sniper > Line".to_string(),
                "off for the match: on the ROM".to_string(),
                "you: helm sensors fire".to_string(),
            ]
        );
        let mut unswitched = other.clone();
        unswitched["mind"]["next_switch_ms"] = Value::Null;
        assert_eq!(
            weapons_text(&unswitched, true),
            rows,
            "the switch window draws nothing: no countdown"
        );
        // A live class of 0 that is not `off` — a coreless hull such as
        // `target_mk1`, or a seal capped at Class 0 on a real core — is the
        // ROM's as well (line mode's `MIND` head names the ROM for both).
        for core in ["0", "II"] {
            let mut rom = armed.clone();
            rom["mind"]["class"] = serde_json::json!("0");
            rom["mind"]["core"] = serde_json::json!(core);
            rom["mind"]["off"] = serde_json::json!(false);
            assert_eq!(
                weapons_text(&rom, true)[1..],
                [
                    "mind Standard Doctrine \u{b7} Class 0".to_string(),
                    "> Sniper   Line".to_string(),
                    "off for the match: on the ROM".to_string(),
                    "you: helm sensors fire".to_string(),
                ],
                "core {core}"
            );
        }
        // A set of one, and every profile of a four-set in sealed order.
        let mut one = armed.clone();
        one["mind"]["profiles"] = serde_json::json!(["Line"]);
        assert_eq!(weapons_text(&one, true)[2], "> Line");
        let mut four = armed.clone();
        four["mind"]["profiles"] =
            serde_json::json!(["Sentry", "Sniper", "Skirmisher", "Rearguard"]);
        four["mind"]["active"] = serde_json::json!(2);
        assert_eq!(
            weapons_text(&four, true)[2],
            "  Sentry   Sniper > Skirmisher   Rearguard"
        );

        // An unarmed hull: no `weapons` key, and the four rows in `(no
        // weapon)`'s place, its channels without `fire`.
        let mut unarmed = serde_json::json!({ "mind": scene_mind() });
        unarmed["mind"]["channels"] = serde_json::json!(["helm", "sensors"]);
        assert_eq!(
            weapons_text(&unarmed, true),
            vec![
                "mind Standard Doctrine \u{b7} Class I".to_string(),
                "> Sniper   Line".to_string(),
                "dormant while you fly".to_string(),
                "you: helm sensors".to_string(),
            ]
        );
        // No `mind` object: the pane as it was before the cut, armed or not.
        for mind in [None, Some(Value::Null), Some(serde_json::json!("Sniper"))] {
            let mut bare = serde_json::json!({});
            let mut gunned = serde_json::json!({ "weapons": [armed["weapons"][0].clone()] });
            if let Some(mind) = mind {
                bare["mind"] = mind.clone();
                gunned["mind"] = mind;
            }
            assert_eq!(weapons_text(&bare, true), vec!["(no weapon)".to_string()]);
            assert_eq!(
                weapons_text(&gunned, true),
                vec!["cannon  ready  14/20  62%".to_string()]
            );
        }
        // A cleared frame draws no tail, whatever it carries.
        let cleared = serde_json::json!({ "seated": false, "mind": scene_mind() });
        assert!(weapons_text(&cleared, true).is_empty());

        // No tail row is selectable: the pane binds no key.
        let theme = AnsiTheme::classic();
        let store = PanelStore::new();
        let ctx = RenderCtx {
            theme: &theme,
            depth: ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane: None,
            picture: None,
        };
        // The tail's own guard, not only its caller's early return.
        assert!(render_cockpit_mind_tail(&cleared, &ctx).rows.is_empty());
        let tail = render_cockpit_mind_tail(&armed, &ctx);
        assert_eq!(tail.rows.len(), 4);
        assert!(
            tail.rows
                .iter()
                .all(|row| !row.selectable && row.prefill.is_none() && row.row_key.is_none())
        );
    }

    /// **17b cut M5.5**: every tail row is at most 44 columns — the Right
    /// column's inner width at the 120-column floor, so no row wraps there —
    /// over every ordered set of one to four of Standard Doctrine's six
    /// presets (`tacsim::arena::card::PRESETS`, the words a sealed set holds)
    /// with each flown, either mind name, every class word, either state,
    /// either channel list and either separator.
    #[test]
    fn m5_every_mind_tail_row_is_at_most_44_columns() {
        // A COPY of `tacsim::arena::card::PRESETS` (this crate does not depend
        // on tacsim): nothing ties it to the source, so a renamed or longer
        // preset would pass here unseen — tying both walks to one committed
        // fixture is carried to the cut that next touches the presets (M6).
        const PRESETS: [&str; 6] = [
            "Line",
            "Sentry",
            "Sniper",
            "Skirmisher",
            "Hunter",
            "Rearguard",
        ];
        let mut sets: Vec<Vec<&str>> = vec![Vec::new()];
        let mut ordered: Vec<Vec<&str>> = Vec::new();
        for _ in 0..4 {
            let mut longer = Vec::new();
            for set in &sets {
                for preset in PRESETS {
                    if !set.contains(&preset) {
                        let mut next = set.clone();
                        next.push(preset);
                        longer.push(next);
                    }
                }
            }
            ordered.extend(longer.iter().cloned());
            sets = longer;
        }
        assert_eq!(ordered.len(), 6 + 30 + 120 + 360);
        let mut widest = 0usize;
        let mut checked = 0usize;
        for set in &ordered {
            for active in 0..set.len() {
                for name in [serde_json::json!("Standard Doctrine"), Value::Null] {
                    for class in ["0", "I", "II", "III"] {
                        for off in [false, true] {
                            for channels in
                                [vec!["helm", "sensors", "fire"], vec!["helm", "sensors"]]
                            {
                                let mind = serde_json::json!({
                                    "name": name, "profiles": set, "active": active,
                                    "class": class, "core": "III", "channels": channels,
                                    "next_switch_ms": 1_000_000_000_i64, "off": off,
                                });
                                for utf8 in [true, false] {
                                    for row in mind_tail_lines(&mind, utf8) {
                                        let width = row.chars().count();
                                        assert!(width <= 44, "{width}: {row:?}");
                                        widest = widest.max(width);
                                    }
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            checked,
            (6 + 2 * 30 + 3 * 120 + 4 * 360) * 2 * 4 * 2 * 2 * 2
        );
        assert_eq!(
            widest, 42,
            "the widest row: a four-set holding Skirmisher and Rearguard"
        );
    }

    /// **17b cut M1b.3**: the three words M1b.1's refusals are said in, in the
    /// pane's ring.
    #[test]
    fn m1b3_the_three_new_refusals_are_said() {
        for (word, said) in [
            ("unwarranted", "FIRE cannon refused: no warrant"),
            ("no_slot", "FIRE cannon refused: every rule slot occupied"),
            (
                "unlawful",
                "FIRE cannon refused: that rule is not lawful for this unit",
            ),
        ] {
            let refused = serde_json::json!({
                "kind": "refused", "order": "fire", "weapon": "cannon", "refusal": word,
            });
            assert_eq!(notice_words(&refused), said);
        }
    }
}
