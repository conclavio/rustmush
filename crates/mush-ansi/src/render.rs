//! [`OutputEvent`] -> classic MUSH text lines (design 02 §5.7).
//!
//! Every line goes through one pipeline: semantic spans are resolved
//! against the theme (explicit span style wins per-field), colors are
//! downgraded to the connection's [`ColorDepth`](mush_proto::ColorDepth),
//! lines are word-wrapped at `caps.width` (ANSI-aware), and the wire sees
//! **CRLF only** — never a bare `\n` (SSH raw mode).

use mush_proto::{
    ClientCaps, NarrativeKind, ObjRef, OutputEvent, RichText, RoomView, Span, Style, WhoEntry,
};

use crate::color::sgr;
use crate::theme::{AnsiTheme, merge_styles};
use crate::wrap::{RESET, wrap};

const CRLF: &[u8] = b"\r\n";

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
        OutputEvent::Who(entries) => render_who(&mut w, entries),
        // Module text fallbacks register in M3 with the Registrar; until
        // then panel frames are web-only and SSH skips them silently.
        OutputEvent::Panel { .. } => {}
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
        // Transport keepalive: nothing to show.
        OutputEvent::Ping { .. } => {}
        // Web-resume acknowledgement: structured-transport only (the WS
        // handshake emits it out-of-band); SSH never sees a resume.
        OutputEvent::Resumed { .. } => {}
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

fn render_room(w: &mut LineWriter<'_>, view: &RoomView) {
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
    w: &mut LineWriter<'_>,
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
        // Emit and spoofed emits render bare; the actor rides along only
        // for spoof-tracking.
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

/// WHO column widths (name / idle / conn); `Doing` takes the rest.
const WHO_NAME_W: usize = 16;
const WHO_IDLE_W: usize = 5;
const WHO_CONN_W: usize = 8;

fn render_who(w: &mut LineWriter<'_>, entries: &[WhoEntry]) {
    let width = if w.caps.width == 0 {
        usize::MAX
    } else {
        w.caps.width as usize
    };
    let header = format!(
        "{:<WHO_NAME_W$}  {:>WHO_IDLE_W$}  {:>WHO_CONN_W$}  {}",
        "Name", "Idle", "Conn", "Doing"
    );
    w.styled(&fit_row(&header, width), Style::default().with_bold());
    w.end_line();
    for e in entries {
        let row = format!(
            "{:<WHO_NAME_W$}  {:>WHO_IDLE_W$}  {:>WHO_CONN_W$}  {}",
            truncate_chars(&e.name, WHO_NAME_W),
            fmt_duration(e.idle_s),
            fmt_duration(e.conn_s),
            e.doing.as_deref().unwrap_or("")
        );
        w.plain(&fit_row(&row, width));
        w.end_line();
    }
    let n = entries.len();
    let s = if n == 1 { "" } else { "s" };
    w.tagged("system", &format!("{n} player{s} connected."));
    w.end_line();
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
        let logical = std::mem::take(&mut self.cur);
        for line in wrap(&logical, self.caps.width as usize) {
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
    fn fit_row_truncates_and_trims() {
        assert_eq!(fit_row("abc   ", 10), "abc");
        assert_eq!(fit_row("abcdef", 3), "abc");
        assert_eq!(fit_row("ab    cd", 6), "ab");
        assert_eq!(truncate_chars("abcdef", 4), "abcd");
        assert_eq!(truncate_chars("ab", 4), "ab");
    }
}
