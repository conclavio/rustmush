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

use mush_ansi::{AnsiTheme, fmt_duration, merge_styles};
use mush_proto::{
    ClientCaps, Color as MushColor, ColorDepth, NarrativeKind, ObjRef, OutputEvent,
    ProjectedActionReceipt, RichText, RoomView, Span as MushSpan, Style as MushStyle, VitalRole,
    VitalState, VitalValue, VitalsFrame, WhoEntry,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::panels::{ChanMsg, PanelStore, rich_plain};
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
    fn plain(line: Line<'static>) -> PaneRow {
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
}

/// A pane's rendered rows.
#[derive(Clone, Debug, Default)]
pub struct PaneRows {
    /// The rows, top to bottom.
    pub rows: Vec<PaneRow>,
}

impl PaneRows {
    fn one(text: &str) -> PaneRows {
        PaneRows {
            rows: vec![PaneRow::plain(Line::from(dim_span_owned(text)))],
        }
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

/// `text` with control characters removed.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// A dim span with owned text (control chars stripped).
pub(crate) fn dim_span_owned(text: &str) -> Span<'static> {
    Span::styled(clean(text), Style::default().add_modifier(Modifier::DIM))
}

fn truncate_chars(s: &str, max: usize) -> String {
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
        | OutputEvent::Panel { .. }
        | OutputEvent::Prompt(_)
        | OutputEvent::Ping { .. }
        | OutputEvent::Resumed { .. }
        | OutputEvent::Composing { .. }
        | OutputEvent::View(_) => {}
    }
    w.finish()
}

/// The room render (mirrors line mode's I3 form): `Name #id`, the
/// description, then the facts in a label gutter — `Here`, `Exits`,
/// `Weather` from `ambient.weather.line`.
fn render_room(w: &mut EntryAcc<'_>, view: &RoomView) {
    w.clock_gutter();
    w.tagged("room", &view.name);
    w.plain(" ");
    w.tagged("dim", &view.id);
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
            w.plain(&obj.name);
        }
        w.end_line();
    }
    if !view.exits.is_empty() {
        w.plain("Exits    ");
        for (i, exit) in view.exits.iter().enumerate() {
            if i > 0 {
                w.plain("  ");
            }
            w.tagged("exit", &exit.name);
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
        w.tagged("weather", line);
        w.end_line();
    }
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
            let rows_data = data.get("rows").and_then(Value::as_array).unwrap_or(&empty);
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
            let mut rows = Vec::new();
            for f in fields {
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

/// One vital's widget text: gauge, grouped boxes with the level, or text.
fn vital_widget(v: &mush_proto::Vital, utf8: bool) -> String {
    match &v.value {
        VitalValue::Bounded { current, min, max } => {
            let (lo, hi) = if min <= max {
                (*min, *max)
            } else {
                (*max, *min)
            };
            let span = hi - lo;
            let cells = 8i64;
            let filled = if span <= 0 {
                0
            } else {
                usize::try_from((((*current).clamp(lo, hi) - lo) * cells + span / 2) / span)
                    .unwrap_or(0)
            };
            let (on, off) = if utf8 {
                ('\u{25b0}', '\u{25b1}')
            } else {
                ('#', '-')
            };
            let bar: String = (0..cells)
                .map(|i| {
                    if usize::try_from(i).unwrap_or(usize::MAX) < filled {
                        on
                    } else {
                        off
                    }
                })
                .collect();
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
// Chrome.
// ---------------------------------------------------------------------------

/// The vitals strip: every `.vitals` frame as `[ label ▰▰▰▱ 6/10 ]` gauges,
/// tracks as boxes with the level named, readouts as text; `Dominant`
/// first. Empty when no frame is held.
#[must_use]
pub fn vitals_line(frames: &[(mush_proto::SmolStr, VitalsFrame)], utf8: bool) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (_, frame) in frames {
        let mut vitals: Vec<&mush_proto::Vital> = frame.vitals.iter().collect();
        vitals.sort_by_key(|v| match v.role {
            VitalRole::Dominant => 0,
            VitalRole::Track => 1,
            VitalRole::Pool => 2,
            VitalRole::Readout => 3,
        });
        for v in vitals {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
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
            spans.push(Span::styled(format!("[ {text} ]"), style));
        }
    }
    Line::from(spans)
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
#[must_use]
pub fn tab_strip(
    screens: &[(u8, &str, usize)],
    active: u8,
    right: &str,
    width: usize,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        "rustmush".to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    let mut used = "rustmush".len();
    for &(digit, name, badge) in screens {
        let label = if badge > 0 {
            format!(" {digit} {name} {badge} ")
        } else {
            format!(" {digit} {name} ")
        };
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

/// The legend: the screen's bindings in nav mode, the editing keys in
/// input mode.
#[must_use]
pub fn legend_line(input_mode: bool, bindings: &[(char, &str)], utf8: bool) -> Line<'static> {
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
    } else {
        for (k, label) in bindings {
            push(&mut spans, &k.to_string(), label);
        }
        push(&mut spans, "Enter", "type");
        push(&mut spans, "?", "help");
    }
    Line::from(spans)
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
        let line = vitals_line(&[("wod.vitals".into(), frame)], false);
        assert_eq!(
            line_text(&line),
            "[ Health X__ Bruised ]  [ WP #####--- 6/10 ]  [ wound -1 ]"
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
        let nav = legend_line(false, &[('l', "look")], false);
        assert_eq!(line_text(&nav), "l look | Enter type | ? help");
        let input = legend_line(true, &[], false);
        assert_eq!(
            line_text(&input),
            "Up history | Tab complete | ^P mode | Esc nav"
        );
        assert_eq!(hhmm(0), "00:00");
        assert_eq!(hhmm(86_399), "23:59");
        assert_eq!(hhmm(-60), "23:59");
    }
}
