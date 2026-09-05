//! Semantic-tag themes: tag -> [`Style`] mapping with a classic MUSH
//! default palette, overridable per entry.

use std::collections::HashMap;

use mush_proto::{Color, Style};
use smol_str::SmolStr;

/// Maps semantic span tags (`"player"`, `"exit"`, `"error"`, …) to display
/// styles.
///
/// [`AnsiTheme::default`] is the classic MUSH palette; individual entries
/// can be overridden with [`AnsiTheme::set`] (server config or per-player
/// preferences). Tags without an entry render plain — `say`/`pose` prose is
/// deliberately unthemed.
#[derive(Clone, Debug)]
pub struct AnsiTheme {
    styles: HashMap<SmolStr, Style>,
}

/// The classic MUSH palette ([`AnsiTheme::classic`]).
impl Default for AnsiTheme {
    fn default() -> AnsiTheme {
        AnsiTheme::classic()
    }
}

impl AnsiTheme {
    /// A theme with no entries at all: every tag renders plain.
    #[must_use]
    pub fn empty() -> AnsiTheme {
        AnsiTheme {
            styles: HashMap::new(),
        }
    }

    /// The default MUSH palette:
    ///
    /// | tag | style |
    /// |---|---|
    /// | `player` | cyan |
    /// | `npc` | magenta |
    /// | `exit` | cyan bold |
    /// | `room` | bold |
    /// | `thing` | plain (no entry) |
    /// | `channel` | yellow |
    /// | `system` | green |
    /// | `error` | red |
    /// | `roll` | magenta bold (the marker on a one-line roll, interface plan I3) |
    /// | `dim` | bright black (a dbref beside a name, a rule) |
    /// | `say` / `pose` | plain (no entry) |
    #[must_use]
    pub fn classic() -> AnsiTheme {
        AnsiTheme::empty()
            .with("player", Style::default().with_fg(Color::Ansi(6)))
            .with("npc", Style::default().with_fg(Color::Ansi(5)))
            .with("exit", Style::default().with_fg(Color::Ansi(6)).with_bold())
            .with("room", Style::default().with_bold())
            .with("channel", Style::default().with_fg(Color::Ansi(3)))
            .with("system", Style::default().with_fg(Color::Ansi(2)))
            .with("error", Style::default().with_fg(Color::Ansi(1)))
            .with("roll", Style::default().with_fg(Color::Ansi(5)).with_bold())
            .with("dim", Style::default().with_fg(Color::Ansi(8)))
    }

    /// Set (or override) the style for one tag.
    pub fn set(&mut self, tag: impl Into<SmolStr>, style: Style) {
        self.styles.insert(tag.into(), style);
    }

    /// Remove a tag's entry so it renders plain again.
    pub fn unset(&mut self, tag: &str) {
        self.styles.remove(tag);
    }

    /// Builder form of [`AnsiTheme::set`].
    #[must_use]
    pub fn with(mut self, tag: impl Into<SmolStr>, style: Style) -> AnsiTheme {
        self.set(tag, style);
        self
    }

    /// The style for a tag, if themed.
    #[must_use]
    pub fn get(&self, tag: &str) -> Option<Style> {
        self.styles.get(tag).copied()
    }

    /// The style for an optional tag, plain when absent or unthemed.
    #[must_use]
    pub fn style_for(&self, tag: Option<&str>) -> Style {
        tag.and_then(|t| self.get(t)).unwrap_or_default()
    }
}

/// Merge a span's explicit style over a theme style: every explicitly set
/// field wins, the theme fills the rest (`fg`/`bg` fall back when unset;
/// attribute flags accumulate).
#[must_use]
pub fn merge_styles(explicit: Style, theme: Style) -> Style {
    Style {
        fg: explicit.fg.or(theme.fg),
        bg: explicit.bg.or(theme.bg),
        bold: explicit.bold || theme.bold,
        italic: explicit.italic || theme.italic,
        underline: explicit.underline || theme.underline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_palette_entries() {
        let t = AnsiTheme::classic();
        assert_eq!(t.get("player").unwrap().fg, Some(Color::Ansi(6)));
        assert_eq!(t.get("npc").unwrap().fg, Some(Color::Ansi(5)));
        let exit = t.get("exit").unwrap();
        assert_eq!(exit.fg, Some(Color::Ansi(6)));
        assert!(exit.bold);
        assert!(t.get("room").unwrap().bold);
        assert_eq!(t.get("channel").unwrap().fg, Some(Color::Ansi(3)));
        assert_eq!(t.get("system").unwrap().fg, Some(Color::Ansi(2)));
        assert_eq!(t.get("error").unwrap().fg, Some(Color::Ansi(1)));
        let roll = t.get("roll").unwrap();
        assert_eq!(roll.fg, Some(Color::Ansi(5)));
        assert!(roll.bold);
        assert_eq!(t.get("dim").unwrap().fg, Some(Color::Ansi(8)));
        // say/pose/thing deliberately unthemed.
        assert!(t.get("say").is_none());
        assert!(t.get("pose").is_none());
        assert!(t.get("thing").is_none());
        assert!(t.style_for(Some("say")).is_plain());
        assert!(t.style_for(None).is_plain());
    }

    #[test]
    fn override_and_unset() {
        let mut t = AnsiTheme::classic();
        t.set(
            "error",
            Style::default().with_fg(Color::Ansi(3)).with_bold(),
        );
        let e = t.get("error").unwrap();
        assert_eq!(e.fg, Some(Color::Ansi(3)));
        assert!(e.bold);
        t.unset("error");
        assert!(t.get("error").is_none());
    }

    #[test]
    fn merge_explicit_wins_per_field() {
        let theme = Style::default().with_fg(Color::Ansi(1)).with_bold();
        let explicit = Style::default().with_fg(Color::Ansi(2));
        let m = merge_styles(explicit, theme);
        assert_eq!(m.fg, Some(Color::Ansi(2))); // explicit fg wins
        assert!(m.bold); // theme bold fills in
        let m2 = merge_styles(Style::default(), theme);
        assert_eq!(m2.fg, Some(Color::Ansi(1)));
    }
}
