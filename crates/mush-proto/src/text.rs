//! Rich text: styled, semantically tagged spans.
//!
//! [`RichText`] is the one representation of player-visible prose on the
//! wire — never raw ANSI, never HTML. Spans carry a semantic [`Span::tag`]
//! (`"exit"`, `"player"`, `"say"`, …) which the ANSI renderer maps to a theme
//! style and the web client maps to a CSS class; an explicit [`Style`] exists
//! for manual color that must survive round-trips; an optional
//! [`CommandLink`] makes a span click-to-run on the web client (SSH ignores
//! it).

use crate::{NarrativeKind, ObjRef, OutputEvent, RoomView, WhoEntry};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt;

/// A single color at any of the three depths a client may support.
///
/// The renderer downgrades as needed for the connection's
/// [`ColorDepth`](crate::ColorDepth) (e.g. `Rgb` → nearest 256-color →
/// nearest 16-color → stripped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Color {
    /// Basic ANSI palette index (0–15).
    Ansi(u8),
    /// xterm 256-color palette index.
    Idx256(u8),
    /// 24-bit true color (red, green, blue).
    Rgb(u8, u8, u8),
}

/// Explicit character styling for a [`Span`].
///
/// Most spans carry no explicit style (the semantic [`Span::tag`] does the
/// theming) — a plain `Style` is skipped entirely on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Style {
    /// Foreground color, if explicitly set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<Color>,
    /// Background color, if explicitly set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
    /// Bold / bright.
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    /// Italic (web + capable terminals).
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    /// Underline.
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Style {
    /// `true` if no styling is set at all — the serde skip predicate for
    /// [`Span::style`].
    pub fn is_plain(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && !self.bold && !self.italic && !self.underline
    }

    /// Returns this style with the foreground color set.
    pub fn with_fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Returns this style with the background color set.
    pub fn with_bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Returns this style with bold set.
    pub fn with_bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Returns this style with italic set.
    pub fn with_italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Returns this style with underline set.
    pub fn with_underline(mut self) -> Self {
        self.underline = true;
        self
    }
}

/// Interactive affordance for the web client: activating the span submits
/// `cmd` exactly as if the player typed it. SSH renderers ignore links.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CommandLink {
    /// The command line to submit, e.g. `"look north"`.
    pub cmd: String,
}

impl CommandLink {
    /// A link that runs `cmd` when activated.
    pub fn new(cmd: impl Into<String>) -> Self {
        Self { cmd: cmd.into() }
    }
}

/// One run of text with uniform styling/semantics inside a [`RichText`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Span {
    /// The literal text. Control characters are never permitted here (the
    /// session layer strips them from inbound lines; emitters must not
    /// introduce them).
    pub text: String,
    /// Explicit style; skipped on the wire when plain.
    #[serde(default, skip_serializing_if = "Style::is_plain")]
    pub style: Style,
    /// Semantic tag, e.g. `"player"`, `"npc"`, `"exit"`, `"channel"`,
    /// `"say"`, `"system"`, `"error"`. Renderers resolve tag → theme color
    /// (ANSI) or tag → CSS class (web).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<SmolStr>,
    /// Click-to-run affordance for the web client; `None` for plain prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<CommandLink>,
}

impl Span {
    /// An unstyled, untagged span.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// A span with a semantic tag.
    pub fn tagged(tag: impl Into<SmolStr>, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tag: Some(tag.into()),
            ..Self::default()
        }
    }

    /// Returns this span with an explicit style.
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Returns this span with a semantic tag.
    pub fn with_tag(mut self, tag: impl Into<SmolStr>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Returns this span with a click-to-run command link.
    pub fn with_link(mut self, cmd: impl Into<String>) -> Self {
        self.link = Some(CommandLink::new(cmd));
        self
    }
}

impl From<&str> for Span {
    fn from(text: &str) -> Self {
        Span::plain(text)
    }
}

impl From<String> for Span {
    fn from(text: String) -> Self {
        Span::plain(text)
    }
}

/// Styled, semantically tagged text — the one prose representation used
/// inside every narrative event.
///
/// Serializes as a bare JSON array of spans. Builder-style helpers keep core
/// code readable:
///
/// ```
/// use mush_proto::RichText;
///
/// let text = RichText::plain("You can go ")
///     .tagged("exit", "north")
///     .text(" from here.");
/// assert_eq!(text.to_plain_text(), "You can go north from here.");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RichText(pub Vec<Span>);

impl RichText {
    /// Empty rich text.
    pub const fn new() -> Self {
        RichText(Vec::new())
    }

    /// A single unstyled span.
    pub fn plain(text: impl Into<String>) -> Self {
        RichText(vec![Span::plain(text)])
    }

    /// A single semantically tagged span (constructor form; see
    /// [`RichText::tagged`] for the chaining form).
    pub fn tag(tag: impl Into<SmolStr>, text: impl Into<String>) -> Self {
        RichText(vec![Span::tagged(tag, text)])
    }

    /// Appends an unstyled span (chaining).
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.0.push(Span::plain(text));
        self
    }

    /// Appends a semantically tagged span (chaining).
    pub fn tagged(mut self, tag: impl Into<SmolStr>, text: impl Into<String>) -> Self {
        self.0.push(Span::tagged(tag, text));
        self
    }

    /// Appends an explicitly styled span (chaining).
    pub fn styled(mut self, style: Style, text: impl Into<String>) -> Self {
        self.0.push(Span::plain(text).with_style(style));
        self
    }

    /// Appends a tagged, click-to-run span (chaining). SSH renderers show
    /// just the text; the web client makes it clickable.
    pub fn linked(
        mut self,
        tag: impl Into<SmolStr>,
        text: impl Into<String>,
        cmd: impl Into<String>,
    ) -> Self {
        self.0.push(Span::tagged(tag, text).with_link(cmd));
        self
    }

    /// Appends an arbitrary pre-built span (chaining).
    pub fn span(mut self, span: Span) -> Self {
        self.0.push(span);
        self
    }

    /// Appends a span in place.
    pub fn push(&mut self, span: Span) {
        self.0.push(span);
    }

    /// Concatenates two texts, consuming both.
    pub fn concat(mut self, other: RichText) -> Self {
        self.0.extend(other.0);
        self
    }

    /// The spans, in order.
    pub fn spans(&self) -> &[Span] {
        &self.0
    }

    /// Number of spans.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` if there are no spans.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The unstyled text content — tags, styles and links dropped. Useful
    /// for logs and tests.
    pub fn to_plain_text(&self) -> String {
        self.0.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Writes the unstyled text content (same as [`RichText::to_plain_text`]).
impl fmt::Display for RichText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for span in &self.0 {
            f.write_str(&span.text)?;
        }
        Ok(())
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> Self {
        RichText::plain(text)
    }
}

impl From<String> for RichText {
    fn from(text: String) -> Self {
        RichText::plain(text)
    }
}

impl From<Span> for RichText {
    fn from(span: Span) -> Self {
        RichText(vec![span])
    }
}

impl FromIterator<Span> for RichText {
    fn from_iter<I: IntoIterator<Item = Span>>(iter: I) -> Self {
        RichText(iter.into_iter().collect())
    }
}

impl Extend<Span> for RichText {
    fn extend<I: IntoIterator<Item = Span>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}

/// Render one [`OutputEvent`] to plain text — spans flattened, narrative
/// kinds given their classic MUSH shapes. This is the test/REPL renderer;
/// `mush-ansi` owns the real one later.
#[must_use]
pub fn text_of(event: &OutputEvent) -> String {
    match event {
        OutputEvent::Room(rv) => room_text(rv),
        OutputEvent::Narrative { kind, actor, text } => narrative_text(*kind, actor.as_ref(), text),
        OutputEvent::Sys(text) => text.to_plain_text(),
        OutputEvent::Channel {
            channel,
            actor,
            text,
            ..
        } => format!("[{channel}] {}: {}", actor.name, text.to_plain_text()),
        OutputEvent::MailNotice {
            from,
            subject,
            id,
            unread,
        } => format!(
            "MAIL: message {id} from {} -- {subject} ({unread} unread)",
            from.name
        ),
        OutputEvent::Who(entries) => who_text(entries),
        OutputEvent::Reply { id, event } => format!("[reply {id}] {}", text_of(event)),
        // Author replies may carry source; keep it out of generic text logs.
        OutputEvent::InvokeResult { id, .. } => format!("[invoke_result {id}]"),
        OutputEvent::Ping { nonce } => format!("[ping {nonce}]"),
        OutputEvent::Resumed { replayed } => format!("[resumed replayed={replayed}]"),
        OutputEvent::Composing { actor, active } => {
            format!("[composing {} active={active}]", actor.name)
        }
        OutputEvent::Request {
            id, kind, prompt, ..
        } => {
            format!("[request {id} {kind:?} {}]", prompt.to_plain_text())
        }
        OutputEvent::Notice {
            category,
            severity,
            text,
            ..
        } => format!("[notice {category} {severity:?} {}]", text.to_plain_text()),
        OutputEvent::Panel { feed, .. } => format!("[panel {feed}]"),
        OutputEvent::View(view) => format!("[view {}]", view.view()),
        OutputEvent::Weather { text, .. } => text.to_plain_text(),
        OutputEvent::Prompt(p) => p
            .text
            .as_ref()
            .map(RichText::to_plain_text)
            .unwrap_or_default(),
        OutputEvent::Hello { server, proto, .. } => format!("{server} (protocol {proto})"),
        OutputEvent::SessionInfo {
            character, account, ..
        } => format!("Connected as {} ({account})", character.name),
        OutputEvent::Disconnect { reason } => reason.to_plain_text(),
    }
}

/// One classic narrative line: `Alice says, "hi"` / `Alice grins.` / ….
fn narrative_text(kind: NarrativeKind, actor: Option<&ObjRef>, text: &RichText) -> String {
    let name = actor.map_or("Someone", |a| a.name.as_str());
    let t = text.to_plain_text();
    match kind {
        NarrativeKind::Say => format!("{name} says, \"{t}\""),
        NarrativeKind::Pose => format!("{name} {t}"),
        NarrativeKind::SemiPose => format!("{name}{t}"),
        NarrativeKind::Emit | NarrativeKind::Spoof => t,
        NarrativeKind::Page => format!("{name} pages: {t}"),
        NarrativeKind::Whisper => format!("{name} whispers, \"{t}\""),
        NarrativeKind::Ooc => format!("<OOC> {name}: {t}"),
    }
}

/// A multi-line classic room render: name(id), desc, contents, exits, zone.
fn room_text(rv: &RoomView) -> String {
    let mut s = format!("{}({})", rv.name, rv.id);
    s.push('\n');
    s.push_str(&rv.desc.to_plain_text());
    if !rv.contents.is_empty() {
        s.push_str("\nContents:");
        for c in &rv.contents {
            s.push_str("\n  ");
            s.push_str(&c.name);
        }
    }
    if !rv.exits.is_empty() {
        let names: Vec<&str> = rv.exits.iter().map(|e| e.name.as_str()).collect();
        s.push_str("\nObvious exits: ");
        s.push_str(&names.join("  "));
    }
    if let Some(z) = &rv.zone {
        s.push_str("\nZone: ");
        s.push_str(&z.name);
    }
    // The weather module's conditions line rides in `ambient` (interface
    // plan I3); the line renderer prints it as the `Weather` fact.
    if let Some(line) = rv
        .ambient
        .get("weather")
        .and_then(|v| v.get("line"))
        .and_then(|v| v.as_str())
    {
        s.push_str("\nWeather: ");
        s.push_str(line);
    }
    s
}

/// A simple WHO table: one line per entry, then a count line.
fn who_text(entries: &[WhoEntry]) -> String {
    let mut s = String::new();
    for e in entries {
        s.push_str(&e.name);
        if let Some(loc) = &e.location {
            s.push_str(&format!(" [{loc}]"));
        }
        if let Some(d) = &e.doing {
            s.push_str(&format!(" -- {d}"));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "{} player{} connected.",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExitRef;

    #[test]
    fn style_is_plain() {
        assert!(Style::default().is_plain());
        assert!(!Style::default().with_bold().is_plain());
        assert!(!Style::default().with_fg(Color::Ansi(3)).is_plain());
    }

    #[test]
    fn builder_reads_cleanly() {
        let t = RichText::plain("Obvious exits: ")
            .linked("exit", "north", "north")
            .text(" and ")
            .tagged("exit", "south");
        assert_eq!(t.len(), 4);
        assert_eq!(t.to_plain_text(), "Obvious exits: north and south");
        assert_eq!(t.spans()[1].link.as_ref().unwrap().cmd, "north");
        assert_eq!(t.spans()[3].tag.as_deref(), Some("exit"));
    }

    #[test]
    fn concat_and_collect() {
        let a = RichText::plain("a").concat(RichText::tag("say", "b"));
        assert_eq!(a.to_plain_text(), "ab");
        let c: RichText = [Span::plain("x"), Span::tagged("npc", "y")]
            .into_iter()
            .collect();
        assert_eq!(c.to_plain_text(), "xy");
        assert_eq!(c.to_string(), "xy");
    }

    #[test]
    fn plain_span_serializes_minimal() {
        let json = serde_json::to_string(&Span::plain("hi")).unwrap();
        assert_eq!(json, r#"{"text":"hi"}"#);
    }

    #[test]
    fn span_tolerates_unknown_and_missing_fields() {
        let span: Span = serde_json::from_str(r#"{"text":"hi","future_field":42}"#).unwrap();
        assert_eq!(span, Span::plain("hi"));
    }

    #[test]
    fn color_wire_shapes() {
        assert_eq!(
            serde_json::to_string(&Color::Ansi(5)).unwrap(),
            r#"{"ansi":5}"#
        );
        assert_eq!(
            serde_json::to_string(&Color::Idx256(208)).unwrap(),
            r#"{"idx256":208}"#
        );
        assert_eq!(
            serde_json::to_string(&Color::Rgb(18, 52, 86)).unwrap(),
            r#"{"rgb":[18,52,86]}"#
        );
    }

    #[test]
    fn narrative_shapes() {
        let cases = [
            (NarrativeKind::Say, "Alice says, \"hi\""),
            (NarrativeKind::Pose, "Alice hi"),
            (NarrativeKind::SemiPose, "Alicehi"),
            (NarrativeKind::Emit, "hi"),
            (NarrativeKind::Spoof, "hi"),
            (NarrativeKind::Page, "Alice pages: hi"),
            (NarrativeKind::Whisper, "Alice whispers, \"hi\""),
            (NarrativeKind::Ooc, "<OOC> Alice: hi"),
        ];
        let alice = ObjRef::new("#9", "Alice");
        for (kind, want) in cases {
            assert_eq!(
                narrative_text(kind, Some(&alice), &RichText::plain("hi")),
                want
            );
        }
        // Unattributed narratives fall back to "Someone".
        assert_eq!(
            narrative_text(NarrativeKind::Say, None, &RichText::plain("hi")),
            "Someone says, \"hi\""
        );
    }

    #[test]
    fn text_of_flattens_spans() {
        let ev = OutputEvent::Sys(
            RichText::tag("error", "Huh?")
                .text("  ")
                .span(Span::tagged("system", "(hint)")),
        );
        assert_eq!(text_of(&ev), "Huh?  (hint)");
    }

    #[test]
    fn room_text_renders_all_sections() {
        let rv = RoomView {
            name: "Hall".into(),
            id: "#5".into(),
            desc: RichText::plain("A big hall."),
            contents: vec![ObjRef::new("#7", "ball")],
            exits: vec![ExitRef {
                name: "north".into(),
                aliases: vec!["n".into()],
            }],
            zone: Some(ObjRef::new("#3", "Downtown")),
            ambient: Default::default(),
        };
        let t = room_text(&rv);
        assert_eq!(
            t,
            "Hall(#5)\nA big hall.\nContents:\n  ball\nObvious exits: north\nZone: Downtown"
        );
    }

    #[test]
    fn who_text_counts() {
        let e = WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 0,
            conn_s: 0,
            location: Some("Limbo".into()),
            here: false,
            doing: Some("testing".into()),
        };
        assert_eq!(
            who_text(&[e]),
            "Alice [Limbo] -- testing\n1 player connected."
        );
        assert_eq!(who_text(&[]), "0 players connected.");
    }
}
