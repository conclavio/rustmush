//! [`OutputEvent`] -> classic MUSH text lines (design 02 §5.7).
//!
//! Every line goes through one pipeline: semantic spans are resolved
//! against the theme (explicit span style wins per-field), colors are
//! downgraded to the connection's [`ColorDepth`](mush_proto::ColorDepth),
//! lines are word-wrapped at `caps.width` (ANSI-aware), and the wire sees
//! **CRLF only** — never a bare `\n` (SSH raw mode).

use mush_proto::{
    ClientCaps, NarrativeKind, NoticeSeverity, ObjRef, OutputEvent, ProjectedActionReceipt,
    RequestKind, RichText, RoomView, Span, Style, WhoEntry,
};

use crate::color::sgr;
use crate::theme::{AnsiTheme, merge_styles};
use crate::wrap::{RESET, wrap_indent};

const CRLF: &[u8] = b"\r\n";

/// The rule under a room heading never exceeds this many columns
/// (interface plan I3).
pub const ROOM_RULE_MAX_W: usize = 72;
/// The label gutter for a room's facts: `Here     `, `Exits    `,
/// `Weather  ` — continuation lines wrap under it.
pub const ROOM_FACT_GUTTER_W: usize = 9;
/// The widest hanging indent a narrative line takes (a speaker's name plus
/// one space); a longer name indents this much and no more.
pub const HANG_MAX_W: usize = 20;

/// Render one event to ANSI bytes appended to `out`.
///
/// Renders every [`OutputEvent`] variant; purely transport-level events
/// (`Ping`) and panel frames with no registered text fallback render
/// nothing. All output lines end in CRLF except [`OutputEvent::Prompt`],
/// which leaves the cursor after the prompt text.
pub fn render(event: &OutputEvent, caps: &ClientCaps, theme: &AnsiTheme, out: &mut Vec<u8>) {
    let mut w = LineWriter::new(caps, theme, out);
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
        // A unified notification (interface plan I11): line mode renders it
        // inline; the tray is a screen thing. The tag colours by severity.
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
            w.tagged(tag, &format!("[{category}] "));
            w.rich(text);
            if let Some(a) = action {
                w.plain(&format!("  ({})", a.prefill));
            }
            w.end_line();
        }
        OutputEvent::Who(entries) => render_who(&mut w, entries),
        // A panel frame renders by its declared shape (doc 06 §10's text
        // fallback, S4): a `grid` prints as a text map; a frame with no shape
        // is still web-only and prints nothing.
        OutputEvent::Panel { kind, data, .. } => {
            if crate::panel::is_grid(data) {
                let columns = if w.caps.width == 0 {
                    crate::panel::GRID_MAX_COLUMNS
                } else {
                    usize::from(w.caps.width)
                        .saturating_sub(2)
                        .min(crate::panel::GRID_MAX_COLUMNS)
                };
                let rows = if w.caps.height == 0 {
                    crate::panel::GRID_MAX_ROWS
                } else {
                    usize::from(w.caps.height)
                        .saturating_sub(4)
                        .min(crate::panel::GRID_MAX_ROWS)
                };
                for line in crate::panel::grid_lines(data, *kind, columns, rows) {
                    w.plain(&line);
                    w.end_line();
                }
            }
        }
        OutputEvent::View(view) => {
            if view.view() == "resolution.outcome"
                && let Ok(receipt) =
                    serde_json::from_value::<ProjectedActionReceipt>(view.data().clone())
                && receipt.view().is_ok_and(|expected| &expected == view)
            {
                let summary = receipt
                    .result()
                    .map_or("Action committed.", |result| result.summary());
                w.tagged(
                    "system",
                    &format!("{summary} Transaction: {}", receipt.status()),
                );
                w.end_line();
            }
        }
        OutputEvent::Weather { text, .. } => {
            w.rich(text);
            w.end_line();
        }
        OutputEvent::Reply { event, .. } => {
            // SSH has no correlation UI: render the payload as if live.
            render(event, caps, theme, out);
        }
        OutputEvent::Prompt(spec) => {
            if let Some(text) = &spec.text {
                w.rich(text);
                w.finish_inline();
            }
        }
        // Private structured authoring replies are consumed by web/TUI or the
        // narrow SSH exec pump. Never dump source into the scrolling stream.
        OutputEvent::InvokeResult { .. } => {}
        // Transport keepalive: nothing to show.
        OutputEvent::Ping { .. } => {}
        // Web-resume acknowledgement: structured-transport only (the WS
        // handshake emits it out-of-band); SSH never sees a resume.
        OutputEvent::Resumed { .. } => {}
        // A presence hint for screens with a status strip; a scrolling
        // stream has nowhere to put "is typing" and nothing to say when it
        // stops.
        OutputEvent::Composing { .. } => {}
        // A server-initiated request (interface plan I10, D-I-21): the modal
        // clients show an overlay; line mode prints the ask and how to answer
        // with `respond`. A client that never answers gets the server's
        // deadline default.
        OutputEvent::Request {
            kind,
            prompt,
            options,
            ..
        } => {
            w.tagged("system", "The game asks:");
            w.end_line();
            w.rich(prompt);
            w.end_line();
            // `respond cancel` is accepted for every kind (`line_respond`)
            // and was printed for none of them (look-back LB-9): a line-mode
            // player who did not want to answer had no documented way out but
            // to wait for the deadline.
            match kind {
                RequestKind::Confirm => {
                    w.plain("  answer:  respond yes   respond no   respond cancel");
                    w.end_line();
                }
                RequestKind::Choice => {
                    // The printed number is the 1-based option index the
                    // engine decodes with `checked_sub(1)`.
                    for (i, o) in options.iter().enumerate() {
                        w.plain(&format!("  {}  {}", i + 1, o.label));
                        w.end_line();
                    }
                    w.plain("  answer:  respond <number>   respond cancel");
                    w.end_line();
                }
                RequestKind::Target | RequestKind::Text => {
                    w.plain("  answer:  respond <your answer>   respond cancel");
                    w.end_line();
                }
            }
        }
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
    }
}

/// `secs` as the classic short WHO duration: `5s`, `2m`, `1h`, `3d`.
#[must_use]
pub fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// The room render (interface plan I3): `Name #id` with the id dim, a rule
/// at `min(width, 72)` (`-` when the terminal has no UTF-8), the
/// description, a blank line, then the facts in a nine-column label gutter
/// — `Here`, `Exits`, and `Weather` from `ambient.weather.line` when the
/// weather module contributed one. Facts wrap under their label.
fn render_room(w: &mut LineWriter<'_>, view: &RoomView) {
    w.tagged("room", &flat(&view.name));
    w.plain(" ");
    w.tagged("dim", &flat(&view.id));
    w.end_line();
    let rule_w = match usize::from(w.caps.width) {
        0 => ROOM_RULE_MAX_W,
        n => n.min(ROOM_RULE_MAX_W),
    };
    let rule: String = if w.caps.utf8 { "\u{2500}" } else { "-" }.repeat(rule_w);
    w.tagged("dim", &rule);
    w.end_line();
    if !view.desc.to_plain_text().is_empty() {
        w.rich(&view.desc);
        w.end_line();
    }
    let weather = ambient_weather_line(view);
    if view.contents.is_empty() && view.exits.is_empty() && weather.is_none() {
        return;
    }
    w.end_line();
    if !view.contents.is_empty() {
        w.plain(&fact_label("Here"));
        for (i, obj) in view.contents.iter().enumerate() {
            if i > 0 {
                w.plain(", ");
            }
            w.plain(&flat(&obj.name));
        }
        w.end_line_indent(ROOM_FACT_GUTTER_W);
    }
    if !view.exits.is_empty() {
        w.plain(&fact_label("Exits"));
        for (i, exit) in view.exits.iter().enumerate() {
            if i > 0 {
                w.plain("  ");
            }
            w.tagged("exit", &flat(&exit.name));
        }
        w.end_line_indent(ROOM_FACT_GUTTER_W);
    }
    if let Some(line) = weather {
        w.plain(&fact_label("Weather"));
        w.tagged("weather", &flat(line));
        w.end_line_indent(ROOM_FACT_GUTTER_W);
    }
}

/// A room field flattened to ONE visual line.
///
/// `LineWriter::write` splits an embedded newline with `end_line()` at indent
/// 0, so a field carrying one would start a fresh flush-left line — exactly
/// where a fact LABEL sits, forging a second `Here` or `Exits` row
/// (look-back I3-C36). Room fields are module- and builder-written and are
/// never sanitised the way player lines are on the way in. Other control
/// bytes the writer already strips.
fn flat(s: &str) -> String {
    s.replace('\n', " ")
}

/// `label` padded to the fact gutter.
fn fact_label(label: &str) -> String {
    format!("{label:<ROOM_FACT_GUTTER_W$}")
}

/// The weather module's prose line from `RoomView.ambient`, when present
/// (`ambient.weather.line`); read defensively, the value is the module's.
fn ambient_weather_line(view: &RoomView) -> Option<&str> {
    view.ambient.get("weather")?.get("line")?.as_str()
}

/// The hanging indent for a narrative line whose speaker is `name`: the
/// name's width plus one, plus `extra` for any fixed prefix before it,
/// capped at [`HANG_MAX_W`]. Control characters never count — the writer
/// strips them.
fn hanging_indent(name: &str, extra: usize) -> usize {
    (name.chars().filter(|c| !c.is_control()).count() + 1 + extra).min(HANG_MAX_W)
}

fn render_narrative(
    w: &mut LineWriter<'_>,
    kind: NarrativeKind,
    actor: Option<&ObjRef>,
    text: &RichText,
) {
    let name = actor.map_or("Someone", |a| a.name.as_str());
    // A long line stays one block under its speaker (interface plan I3):
    // continuation lines indent to where the body began. Emits have no
    // speaker column and wrap flush.
    let indent = match kind {
        NarrativeKind::Say
        | NarrativeKind::Pose
        | NarrativeKind::SemiPose
        | NarrativeKind::Page
        | NarrativeKind::Whisper => hanging_indent(name, 0),
        NarrativeKind::Ooc if actor.is_some() => hanging_indent(name, "<OOC> ".len()),
        NarrativeKind::Emit | NarrativeKind::Spoof | NarrativeKind::Ooc => 0,
    };
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
        // An emit renders bare; the actor rides along only for tracking. A
        // spoof only reaches a listener entitled to its source (the world
        // task downgrades it to Emit for everyone else), so it is prefixed.
        NarrativeKind::Emit => w.rich(text),
        NarrativeKind::Spoof => {
            w.plain("[");
            w.tagged("player", name);
            w.plain("] ");
            w.rich(text);
        }
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
    w.end_line_indent(indent);
}

fn render_channel(
    w: &mut LineWriter<'_>,
    channel: &str,
    actor: &ObjRef,
    title: Option<&str>,
    kind: NarrativeKind,
    text: &RichText,
) {
    w.tagged("channel", &format!("[{channel}]"));
    w.plain(" ");
    let name_prefix = |w: &mut LineWriter<'_>| {
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
        // Say and everything else: the classic `[Public] Name: text`.
        _ => {
            name_prefix(w);
            w.plain(": ");
            w.rich(text);
        }
    }
    w.end_line();
}

/// WHO column widths (name / idle / conn / where); `Doing` takes the rest.
const WHO_NAME_W: usize = 16;
const WHO_IDLE_W: usize = 5;
const WHO_CONN_W: usize = 8;
/// The `Where` column (interface plan I3, D-I-12): a location the viewer
/// may see — every one for staff, the viewer's own room for a player.
const WHO_WHERE_W: usize = 14;

fn render_who(w: &mut LineWriter<'_>, entries: &[WhoEntry]) {
    let width = if w.caps.width == 0 {
        usize::MAX
    } else {
        usize::from(w.caps.width)
    };
    let header = format!(
        "{:<WHO_NAME_W$}  {:>WHO_IDLE_W$}  {:>WHO_CONN_W$}  {:<WHO_WHERE_W$}  {}",
        "Name", "Idle", "Conn", "Where", "Doing"
    );
    w.styled(&fit_row(&header, width), Style::default().with_bold());
    w.end_line();
    for e in entries {
        // Sanitize before padding so a control character can never shift a
        // column (the writer would strip it after the widths were computed).
        let row = format!(
            "{:<WHO_NAME_W$}  {:>WHO_IDLE_W$}  {:>WHO_CONN_W$}  {:<WHO_WHERE_W$}  {}",
            truncate_chars(&printable(&e.name), WHO_NAME_W),
            fmt_duration(e.idle_s),
            fmt_duration(e.conn_s),
            truncate_chars(&printable(e.location.as_deref().unwrap_or("")), WHO_WHERE_W),
            printable(e.doing.as_deref().unwrap_or(""))
        );
        w.plain(&fit_row(&row, width));
        w.end_line();
    }
    let n = entries.len();
    let here = entries.iter().filter(|e| e.here).count();
    w.tagged("system", &format!("{n} connected, {here} in your room."));
    w.end_line();
}

/// `s` with every control character removed.
fn printable(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Truncate a plain (escape-free) row to the terminal width and drop
/// trailing padding, so WHO rows never trigger the word-wrapper.
fn fit_row(row: &str, width: usize) -> String {
    let fitted: String = row.chars().take(width).collect();
    fitted.trim_end().to_string()
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Accumulates one logical line of styled text, then flushes it through
/// the word-wrapper as CRLF-terminated output bytes.
struct LineWriter<'a> {
    caps: &'a ClientCaps,
    theme: &'a AnsiTheme,
    out: &'a mut Vec<u8>,
    cur: String,
}

impl<'a> LineWriter<'a> {
    fn new(caps: &'a ClientCaps, theme: &'a AnsiTheme, out: &'a mut Vec<u8>) -> LineWriter<'a> {
        LineWriter {
            caps,
            theme,
            out,
            cur: String::new(),
        }
    }

    /// Append every span of `text`.
    fn rich(&mut self, text: &RichText) {
        for span in text.spans() {
            self.span(span);
        }
    }

    /// Append one wire span: explicit style merged over the theme style for
    /// its tag (explicit fields win).
    fn span(&mut self, span: &Span) {
        let theme_style = self.theme.style_for(span.tag.as_deref());
        let style = merge_styles(span.style, theme_style);
        self.write(&span.text, style);
    }

    /// Append renderer-generated text under a semantic tag.
    fn tagged(&mut self, tag: &str, text: &str) {
        let style = self.theme.style_for(Some(tag));
        self.write(text, style);
    }

    /// Append unstyled text.
    fn plain(&mut self, text: &str) {
        self.write(text, Style::default());
    }

    /// Append text with an explicit style (no tag lookup).
    fn styled(&mut self, text: &str, style: Style) {
        self.write(text, style);
    }

    /// Append `text` in `style`, splitting embedded `\n` into line breaks
    /// and stripping all other control characters (ANSI-injection defense:
    /// spans must never smuggle raw escapes to the wire).
    fn write(&mut self, text: &str, style: Style) {
        let mut first = true;
        for piece in text.split('\n') {
            if !first {
                self.end_line();
            }
            first = false;
            if piece.is_empty() {
                continue;
            }
            let clean: String = piece.chars().filter(|c| !c.is_control()).collect();
            if clean.is_empty() {
                continue;
            }
            let seq = sgr(style, self.caps.color);
            if seq.is_empty() {
                self.cur.push_str(&clean);
            } else {
                self.cur.push_str(&seq);
                self.cur.push_str(&clean);
                self.cur.push_str(RESET);
            }
        }
    }

    /// Wrap the accumulated logical line at the terminal width and emit it
    /// as CRLF-terminated bytes (style reset before every line end).
    fn end_line(&mut self) {
        self.end_line_indent(0);
    }

    /// [`LineWriter::end_line`] with a hanging indent: continuation lines
    /// start `indent` columns in (see [`crate::wrap_indent`]).
    fn end_line_indent(&mut self, indent: usize) {
        let logical = std::mem::take(&mut self.cur);
        for line in wrap_indent(&logical, usize::from(self.caps.width), indent) {
            self.out.extend_from_slice(line.as_bytes());
            self.out.extend_from_slice(CRLF);
        }
    }

    /// Emit the accumulated text without a line terminator (prompts leave
    /// the cursor on the line).
    fn finish_inline(&mut self) {
        let logical = std::mem::take(&mut self.cur);
        self.out.extend_from_slice(logical.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_duration_short_forms() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(5), "5s");
        assert_eq!(fmt_duration(59), "59s");
        assert_eq!(fmt_duration(60), "1m");
        assert_eq!(fmt_duration(120), "2m");
        assert_eq!(fmt_duration(3599), "59m");
        assert_eq!(fmt_duration(3600), "1h");
        assert_eq!(fmt_duration(86_399), "23h");
        assert_eq!(fmt_duration(86_400), "1d");
        assert_eq!(fmt_duration(3 * 86_400 + 7), "3d");
    }

    #[test]
    fn hanging_indent_is_name_plus_one_capped_and_control_free() {
        assert_eq!(hanging_indent("Nick", 0), 5);
        assert_eq!(hanging_indent("Nick", 6), 11);
        assert_eq!(hanging_indent("Ni\x07ck", 0), 5);
        assert_eq!(
            hanging_indent("An-Extraordinarily-Long-Name", 0),
            HANG_MAX_W
        );
    }

    #[test]
    fn fit_row_truncates_and_trims() {
        assert_eq!(fit_row("abc   ", 10), "abc");
        assert_eq!(fit_row("abcdef", 3), "abc");
        assert_eq!(fit_row("ab    cd", 6), "ab");
        assert_eq!(truncate_chars("abcdef", 4), "abcd");
        assert_eq!(truncate_chars("ab", 4), "ab");
    }
}
