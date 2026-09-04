//! [`OutputEvent`] -> ratatui [`Line`]s — the TUI's clean-room sibling of
//! `mush_ansi::render` (design M15 D3/main-frame).
//!
//! The same semantic event stream that the line renderer turns into
//! CRLF-terminated ANSI is turned here into owned `Line<'static>` values for
//! the scrollback ring. Word-wrapping is *not* done here: the main-frame
//! `Paragraph` wraps at paint time against the live pane width, so a resize
//! reflows without re-rendering history. Semantic span tags resolve through
//! the shared [`AnsiTheme`] and colors downgrade to the connection's
//! [`ColorDepth`] exactly as they do on the line side.
//!
//! Scrollback-bearing variants (`Room`, `Narrative`, `Sys`, `Channel`,
//! `MailNotice`, `Weather`, `Hello`, `SessionInfo`, `Disconnect`, and a
//! `Reply` unwrapping to one of those) yield lines; `Who` feeds the WHO rail
//! and `Prompt` drives the input line, so both yield nothing here; `Panel`,
//! `Ping`, and `Resumed` render nothing, matching the line renderer.

use mush_ansi::{AnsiTheme, fmt_duration, merge_styles};
use mush_proto::{
    ClientCaps, Color as MushColor, ColorDepth, NarrativeKind, ObjRef, OutputEvent,
    ProjectedActionReceipt, RichText, RoomView, Span as MushSpan, Style as MushStyle, WhoEntry,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

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

/// Accumulates ratatui spans into logical lines, splitting embedded `\n` and
/// stripping other control characters (the same injection defense the line
/// renderer applies).
struct LineAcc<'t> {
    theme: &'t AnsiTheme,
    depth: ColorDepth,
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
}

impl<'t> LineAcc<'t> {
    fn new(theme: &'t AnsiTheme, depth: ColorDepth) -> LineAcc<'t> {
        LineAcc {
            theme,
            depth,
            lines: Vec::new(),
            cur: Vec::new(),
        }
    }

    fn write(&mut self, text: &str, style: Style) {
        let mut first = true;
        for piece in text.split('\n') {
            if !first {
                self.end_line();
            }
            first = false;
            let clean: String = piece.chars().filter(|c| !c.is_control()).collect();
            if clean.is_empty() {
                continue;
            }
            self.cur.push(Span::styled(clean, style));
        }
    }

    fn plain(&mut self, text: &str) {
        self.write(text, Style::default());
    }

    fn tagged(&mut self, tag: &str, text: &str) {
        let style = to_ratatui_style(self.theme.style_for(Some(tag)), self.depth);
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
        self.lines.push(Line::from(std::mem::take(&mut self.cur)));
    }

    /// Flush any trailing partial line and yield the accumulated lines.
    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.cur.is_empty() {
            self.end_line();
        }
        self.lines
    }
}

/// Render one event to scrollback lines. Returns empty for variants the TUI
/// routes elsewhere (`Who`, `Prompt`) or drops (`Panel`, `Ping`, `Resumed`).
pub fn render_event(
    event: &OutputEvent,
    caps: &ClientCaps,
    theme: &AnsiTheme,
) -> Vec<Line<'static>> {
    let mut w = LineAcc::new(theme, caps.color);
    match event {
        OutputEvent::Room(view) => render_room(&mut w, view),
        OutputEvent::Narrative { kind, actor, text } => {
            render_narrative(&mut w, *kind, actor.as_ref(), text);
        }
        OutputEvent::Sys(text) => {
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
            w.rich(text);
            w.end_line();
        }
        OutputEvent::Reply { event, .. } => return render_event(event, caps, theme),
        OutputEvent::Hello { proto, server, .. } => {
            w.tagged("system", &format!("{server} (proto {proto})"));
            w.end_line();
        }
        OutputEvent::SessionInfo {
            character,
            account,
            connections,
        } => {
            let s = if *connections == 1 { "" } else { "s" };
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
            w.rich(reason);
            w.end_line();
        }
        OutputEvent::View(view) if view.view() == "resolution.outcome" => {
            if let Ok(receipt) =
                serde_json::from_value::<ProjectedActionReceipt>(view.data().clone())
                && receipt.view().is_ok_and(|expected| &expected == view)
            {
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
        // Routed elsewhere or intentionally silent (mirrors mush_ansi).
        OutputEvent::Who(_)
        | OutputEvent::Panel { .. }
        | OutputEvent::Prompt(_)
        | OutputEvent::Ping { .. }
        | OutputEvent::Resumed { .. } => {}
        OutputEvent::View(_) => {}
    }
    w.finish()
}

fn render_room(w: &mut LineAcc<'_>, view: &RoomView) {
    w.tagged("room", &format!("{}({})", view.name, view.id));
    w.end_line();
    w.rich(&view.desc);
    w.end_line();
    if !view.contents.is_empty() {
        w.plain("Contents:");
        w.end_line();
        for obj in &view.contents {
            w.plain(&obj.name);
            w.end_line();
        }
    }
    if !view.exits.is_empty() {
        w.plain("Obvious exits:");
        w.end_line();
        for (i, exit) in view.exits.iter().enumerate() {
            if i > 0 {
                w.plain("  ");
            }
            w.tagged("exit", &exit.name);
        }
        w.end_line();
    }
}

fn render_narrative(
    w: &mut LineAcc<'_>,
    kind: NarrativeKind,
    actor: Option<&ObjRef>,
    text: &RichText,
) {
    let name = actor.map_or("Someone", |a| a.name.as_str());
    match kind {
        NarrativeKind::Say => {
            w.tagged("player", name);
            w.plain(" says, \"");
            w.rich(text);
            w.plain("\"");
        }
        NarrativeKind::Pose => {
            w.tagged("player", name);
            w.plain(" ");
            w.rich(text);
        }
        NarrativeKind::SemiPose => {
            w.tagged("player", name);
            w.rich(text);
        }
        NarrativeKind::Emit | NarrativeKind::Spoof => w.rich(text),
        NarrativeKind::Page => {
            w.tagged("player", name);
            w.plain(" pages: ");
            w.rich(text);
        }
        NarrativeKind::Whisper => {
            w.tagged("player", name);
            w.plain(" whispers, \"");
            w.rich(text);
            w.plain("\"");
        }
        NarrativeKind::Ooc => {
            w.plain("<OOC> ");
            if actor.is_some() {
                w.tagged("player", name);
                w.plain(" says, \"");
                w.rich(text);
                w.plain("\"");
            } else {
                w.rich(text);
            }
        }
    }
    w.end_line();
}

fn render_channel(
    w: &mut LineAcc<'_>,
    channel: &str,
    actor: &ObjRef,
    title: Option<&str>,
    kind: NarrativeKind,
    text: &RichText,
) {
    w.tagged("channel", &format!("[{channel}]"));
    w.plain(" ");
    let name_prefix = |w: &mut LineAcc<'_>| {
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

/// Render a WHO listing into rail lines: a bold header, one row per entry
/// (name + idle, the rail is narrow), then a bold count footer. Rows are
/// left to the rail `Paragraph`/`List` to clip at the rail width.
pub fn render_who_rail(
    entries: &[WhoEntry],
    theme: &AnsiTheme,
    depth: ColorDepth,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(entries.len() + 2);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled(
        format!("{:<14} {:>4}", "Name", "Idle"),
        bold,
    )));
    let player_style = to_ratatui_style(theme.style_for(Some("player")), depth);
    for e in entries {
        let name = truncate_chars(&e.name, 14);
        let idle = fmt_duration(e.idle_s);
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<14}"), player_style),
            Span::raw(" "),
            Span::raw(format!("{idle:>4}")),
        ]));
    }
    let n = entries.len();
    let s = if n == 1 { "" } else { "s" };
    lines.push(Line::from(Span::styled(
        format!("{n} player{s}"),
        to_ratatui_style(theme.style_for(Some("system")), depth),
    )));
    lines
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ---------------------------------------------------------------------------
// Secondary-pane widgets (design M15 D5): the @mail and +intel panels. Both
// render the `OutputEvent::Panel` keyframe JSON their query handlers return
// (`mail.list` / `intel.list`) into owned ratatui lines. Parsing is defensive
// — a missing or malformed field degrades to a placeholder rather than
// panicking (invariant T6: pure + testable).
// ---------------------------------------------------------------------------

/// Render the `mail.list` panel keyframe (see `mush_mail::query::ListQuery`)
/// into secondary-pane lines: a folder header with the unread count, then one
/// row per message (`index`, an unread marker, sender, subject).
#[must_use]
pub fn render_mail_panel(
    data: &serde_json::Value,
    theme: &AnsiTheme,
    depth: ColorDepth,
) -> Vec<Line<'static>> {
    let system = to_ratatui_style(theme.style_for(Some("system")), depth);
    let player = to_ratatui_style(theme.style_for(Some("player")), depth);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let folder = data
        .get("folder")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("inbox");
    let entries = data.get("entries").and_then(serde_json::Value::as_array);
    let count = entries.map_or(0, Vec::len);
    lines.push(Line::from(Span::styled(
        format!(
            "{folder} — {count} message{}",
            if count == 1 { "" } else { "s" }
        ),
        system,
    )));

    let Some(entries) = entries else {
        return lines;
    };
    if entries.is_empty() {
        lines.push(Line::from(dim_span_owned("(empty)")));
        return lines;
    }
    for e in entries {
        let index = e
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let status = e
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("read");
        let marker = match status {
            "urgent" => "!",
            "unread" => "*",
            _ => " ",
        };
        let from = e
            .get("from")
            .and_then(|f| f.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown)");
        let subject = e
            .get("subject")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(no subject)");
        lines.push(Line::from(vec![
            Span::raw(format!("{index:>2}{marker} ")),
            Span::styled(format!("{}: ", truncate_chars(from, 10)), player),
            Span::raw(truncate_chars(subject, 20)),
        ]));
    }
    lines
}

/// Render the `intel.list` panel keyframe (see
/// `mush_npc::query::IntelListQuery`) into secondary-pane lines: a count
/// header then, per record (newest first), a styled subject + kind line and a
/// dim detail line.
#[must_use]
pub fn render_intel_panel(
    data: &serde_json::Value,
    theme: &AnsiTheme,
    depth: ColorDepth,
) -> Vec<Line<'static>> {
    let system = to_ratatui_style(theme.style_for(Some("system")), depth);
    let player = to_ratatui_style(theme.style_for(Some("player")), depth);
    let channel = to_ratatui_style(theme.style_for(Some("channel")), depth);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let records = data.get("records").and_then(serde_json::Value::as_array);
    let count = records.map_or(0, Vec::len);
    lines.push(Line::from(Span::styled(
        format!(
            "Intel — {count} record{}",
            if count == 1 { "" } else { "s" }
        ),
        system,
    )));

    let Some(records) = records else {
        return lines;
    };
    if records.is_empty() {
        lines.push(Line::from(dim_span_owned("(nothing filed)")));
        return lines;
    }
    for r in records {
        let subject = r
            .get("subject")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown)");
        let kind = r
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("seen");
        let detail = r
            .get("detail")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled(truncate_chars(subject, 16), player),
            Span::raw(" "),
            Span::styled(format!("[{kind}]"), channel),
        ]));
        if !detail.is_empty() {
            lines.push(Line::from(dim_span_owned(&format!(
                "  {}",
                truncate_chars(detail, 28)
            ))));
        }
    }
    lines
}

/// Render the shared viewer-safe legal-action DTO array. Opaque offer tokens
/// and every unrecognized field remain hidden; the TUI needs only the public
/// verb, label, and help text used by the line adapter.
#[must_use]
pub fn render_legal_actions_panel(
    data: &serde_json::Value,
    theme: &AnsiTheme,
    depth: ColorDepth,
) -> Vec<Line<'static>> {
    let system = to_ratatui_style(theme.style_for(Some("system")), depth);
    let player = to_ratatui_style(theme.style_for(Some("player")), depth);
    let Some(offers) = data.as_array() else {
        return vec![Line::from(dim_span_owned("(unavailable)"))];
    };
    if offers.is_empty() {
        return vec![Line::from(dim_span_owned("(none available)"))];
    }
    let mut lines = Vec::new();
    for presentation in offers.iter().filter_map(|offer| offer.get("presentation")) {
        let verb = presentation
            .get("verb")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("action");
        let label = presentation
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(verb);
        let help = presentation
            .get("help")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled(format!("+act {}", truncate_chars(verb, 18)), player),
            Span::styled(format!(" — {}", truncate_chars(label, 18)), system),
        ]));
        if !help.is_empty() {
            lines.push(Line::from(dim_span_owned(&format!(
                "  {}",
                truncate_chars(help, 30)
            ))));
        }
    }
    lines
}

/// A dim span with owned text (control chars stripped), for widget placeholders
/// and detail lines.
fn dim_span_owned(text: &str) -> Span<'static> {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    Span::styled(clean, Style::default().add_modifier(Modifier::DIM))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ClientKind, ObjRef};

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

    fn text_of(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn say_renders_one_line_with_player_tag() {
        let theme = AnsiTheme::classic();
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#10", "Nick")),
            text: RichText::plain("hi there"),
        };
        let lines = render_event(&ev, &caps(ColorDepth::Ansi16), &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(text_of(&lines[0]), "Nick says, \"hi there\"");
        // The actor's name span carries the themed player color (cyan).
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Indexed(6)));
    }

    #[test]
    fn sys_multiline_splits_into_lines() {
        let theme = AnsiTheme::classic();
        let ev = OutputEvent::Sys(RichText::plain("one\ntwo\nthree"));
        let lines = render_event(&ev, &caps(ColorDepth::TrueColor), &theme);
        assert_eq!(lines.len(), 3);
        assert_eq!(text_of(&lines[0]), "one");
        assert_eq!(text_of(&lines[2]), "three");
    }

    #[test]
    fn channel_renders_bracketed_prefix() {
        let theme = AnsiTheme::classic();
        let ev = OutputEvent::Channel {
            channel: "pub".into(),
            seq: 1,
            actor: ObjRef::new("#5", "Zoe"),
            title: None,
            kind: NarrativeKind::Say,
            text: RichText::plain("hello"),
            ts: 0,
        };
        let lines = render_event(&ev, &caps(ColorDepth::Ansi16), &theme);
        assert_eq!(text_of(&lines[0]), "[pub] Zoe: hello");
    }

    #[test]
    fn reply_unwraps_payload() {
        let theme = AnsiTheme::classic();
        let inner = OutputEvent::Sys(RichText::plain("wrapped"));
        let ev = OutputEvent::Reply {
            id: 7,
            event: Box::new(inner),
        };
        let lines = render_event(&ev, &caps(ColorDepth::None), &theme);
        assert_eq!(text_of(&lines[0]), "wrapped");
    }

    #[test]
    fn who_and_prompt_yield_no_scrollback() {
        let theme = AnsiTheme::classic();
        assert!(
            render_event(&OutputEvent::Who(vec![]), &caps(ColorDepth::Ansi16), &theme).is_empty()
        );
        let prompt = OutputEvent::Prompt(mush_proto::PromptSpec {
            mode: mush_proto::InputMode::Masked,
            text: Some(RichText::plain("pw>")),
        });
        assert!(render_event(&prompt, &caps(ColorDepth::Ansi16), &theme).is_empty());
    }

    #[test]
    fn action_receipt_renders_summary_and_recovery_reference() {
        let status = mush_proto::ActionStatusId::new(
            "k5:toy:brace:0000000000000007:01010101010101010101010101010101",
        )
        .unwrap();
        let result = mush_proto::ProjectedActionResult::new(
            "brace",
            "Brace committed.",
            serde_json::json!({"event": "braced", "focus": 2}),
        )
        .unwrap();
        let event = OutputEvent::View(
            mush_proto::ProjectedActionReceipt::new(
                status,
                "02020202020202020202020202020202",
                Some(result),
            )
            .unwrap()
            .view()
            .unwrap(),
        );
        let lines = render_event(&event, &caps(ColorDepth::None), &AnsiTheme::classic());
        assert_eq!(text_of(&lines[0]), "Brace committed.");
        assert_eq!(
            text_of(&lines[1]),
            "Transaction: k5:toy:brace:0000000000000007:01010101010101010101010101010101"
        );
        let OutputEvent::View(view) = event else {
            unreachable!()
        };
        let forged = OutputEvent::View(
            view.with_commit("03030303030303030303030303030303")
                .unwrap(),
        );
        assert!(render_event(&forged, &caps(ColorDepth::None), &AnsiTheme::classic()).is_empty());
    }

    #[test]
    fn color_depth_none_drops_styling() {
        let theme = AnsiTheme::classic();
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Nick")),
            text: RichText::plain("x"),
        };
        let lines = render_event(&ev, &caps(ColorDepth::None), &theme);
        assert_eq!(lines[0].spans[0].style.fg, None);
    }

    #[test]
    fn mail_panel_header_and_entry_rows() {
        let theme = AnsiTheme::classic();
        let data = serde_json::json!({
            "folder": "inbox",
            "entries": [
                { "index": 1, "status": "unread", "from": { "name": "Zoe" }, "subject": "Hi" },
                { "index": 2, "status": "read", "from": { "name": "Al" }, "subject": "Later" },
            ],
        });
        let lines = render_mail_panel(&data, &theme, ColorDepth::Ansi16);
        assert_eq!(lines.len(), 3); // header + 2 entries
        assert!(text_of(&lines[0]).contains("inbox"));
        assert!(text_of(&lines[0]).contains("2 messages"));
        assert!(text_of(&lines[1]).contains("Zoe"));
        assert!(text_of(&lines[1]).contains("Hi"));
        // The unread marker distinguishes row 1 from the read row 2.
        assert!(text_of(&lines[1]).contains('*'));
    }

    #[test]
    fn mail_panel_empty_folder_shows_placeholder() {
        let theme = AnsiTheme::classic();
        let data = serde_json::json!({ "folder": "inbox", "entries": [] });
        let lines = render_mail_panel(&data, &theme, ColorDepth::Ansi16);
        assert_eq!(lines.len(), 2);
        assert!(text_of(&lines[1]).contains("empty"));
    }

    #[test]
    fn mail_panel_tolerates_missing_fields() {
        let theme = AnsiTheme::classic();
        // No entries key at all: just the header, no panic.
        let lines = render_mail_panel(&serde_json::json!({}), &theme, ColorDepth::None);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn intel_panel_header_and_records() {
        let theme = AnsiTheme::classic();
        let data = serde_json::json!({
            "count": 2,
            "records": [
                { "subject": "Mallory", "kind": "spoke", "detail": "said hello" },
                { "subject": "Trent", "kind": "arrived", "detail": "" },
            ],
        });
        let lines = render_intel_panel(&data, &theme, ColorDepth::Ansi16);
        // header + (subject line + detail line) for #1 + (subject line only) for #2
        assert_eq!(lines.len(), 4);
        assert!(text_of(&lines[0]).contains("2 records"));
        assert!(text_of(&lines[1]).contains("Mallory"));
        assert!(text_of(&lines[1]).contains("[spoke]"));
        assert!(text_of(&lines[2]).contains("said hello"));
        assert!(text_of(&lines[3]).contains("Trent"));
    }

    #[test]
    fn intel_panel_empty_shows_placeholder() {
        let theme = AnsiTheme::classic();
        let data = serde_json::json!({ "count": 0, "records": [] });
        let lines = render_intel_panel(&data, &theme, ColorDepth::Ansi16);
        assert_eq!(lines.len(), 2);
        assert!(text_of(&lines[1]).contains("nothing"));
    }

    #[test]
    fn legal_action_panel_renders_only_public_presentation() {
        let theme = AnsiTheme::classic();
        let data = serde_json::json!([{
            "offer": "secret-opaque-token",
            "truth_subject": { "clock": "scene_turn" },
            "private": { "definition": "private-definition-digest" },
            "presentation": {
                "verb": "brace",
                "label": "Brace",
                "help": "Adopt a braced tactical stance."
            }
        }]);
        let lines = render_legal_actions_panel(&data, &theme, ColorDepth::Ansi16);
        let rendered = lines.iter().map(text_of).collect::<Vec<_>>().join("\n");
        assert_eq!(lines.len(), 2);
        assert!(text_of(&lines[0]).contains("+act brace"));
        assert!(text_of(&lines[0]).contains("Brace"));
        assert!(text_of(&lines[1]).contains("braced tactical stance"));
        assert!(
            lines.iter().all(|line| !text_of(line).contains("secret")),
            "the TUI must not display opaque execution tokens"
        );
        mush_contract_tests::assert_projection_text_clean(
            "TUI legal-action panel",
            &rendered,
            &[
                "secret-opaque-token",
                "scene_turn",
                "private-definition-digest",
            ],
        );
    }

    #[test]
    fn who_rail_has_header_rows_and_footer() {
        let theme = AnsiTheme::classic();
        let entries = vec![
            WhoEntry {
                name: "Alice".into(),
                alias: None,
                idle_s: 5,
                conn_s: 100,
                location: None,
                doing: None,
            },
            WhoEntry {
                name: "Bob".into(),
                alias: None,
                idle_s: 65,
                conn_s: 200,
                location: None,
                doing: None,
            },
        ];
        let lines = render_who_rail(&entries, &theme, ColorDepth::Ansi16);
        assert_eq!(lines.len(), 4); // header + 2 rows + footer
        assert!(text_of(&lines[0]).starts_with("Name"));
        assert!(text_of(&lines[1]).contains("Alice"));
        assert!(text_of(&lines[2]).contains("1m")); // 65s -> 1m
        assert_eq!(text_of(&lines[3]), "2 players");
    }
}
