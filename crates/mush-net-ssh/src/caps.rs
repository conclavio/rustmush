//! Terminal capability detection (design 02 §5.6).
//!
//! SSH gives us `TERM` (from the pty request) and, when the client's config
//! forwards them, `COLORTERM` / `LANG` / `LC_ALL` (from env requests). The
//! heuristics here turn those into a [`ClientCaps`]:
//!
//! - `COLORTERM=truecolor|24bit` → [`ColorDepth::TrueColor`]
//! - `TERM=dumb` → [`ColorDepth::None`] (wins even over `COLORTERM`)
//! - `TERM` containing `256color` → [`ColorDepth::Ansi256`]
//! - `TERM` containing `direct` (terminfo direct-color convention) →
//!   [`ColorDepth::TrueColor`]
//! - otherwise → [`ColorDepth::Ansi16`]
//!
//! UTF-8 is assumed (it is 2026); locale variables are sniffed only to
//! *disable* it: an explicit `LC_ALL`/`LANG` that names a non-UTF-8
//! charset (e.g. `C`, `POSIX`, `en_US.ISO8859-1`) turns it off. `LC_ALL`
//! overrides `LANG` per POSIX.

use mush_proto::{ClientCaps, ClientKind, ColorDepth};

/// Fallback terminal size when the client reports zero columns/rows.
pub const DEFAULT_WIDTH: u16 = 80;
/// Fallback terminal height.
pub const DEFAULT_HEIGHT: u16 = 24;

/// Color support implied by `TERM` and `COLORTERM`.
pub fn detect_color_depth(term: Option<&str>, colorterm: Option<&str>) -> ColorDepth {
    let term_lc = term.map(str::to_ascii_lowercase);
    if term_lc.as_deref() == Some("dumb") {
        return ColorDepth::None;
    }
    if let Some(ct) = colorterm {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return ColorDepth::TrueColor;
        }
    }
    match term_lc.as_deref() {
        Some(t) if t.contains("256color") => ColorDepth::Ansi256,
        Some(t) if t.contains("direct") => ColorDepth::TrueColor,
        _ => ColorDepth::Ansi16,
    }
}

/// Whether the client accepts UTF-8 output. `None`/empty variables mean
/// "no opinion" and default to yes.
pub fn detect_utf8(lang: Option<&str>, lc_all: Option<&str>) -> bool {
    let effective = lc_all
        .filter(|s| !s.is_empty())
        .or_else(|| lang.filter(|s| !s.is_empty()));
    match effective {
        None => true,
        Some(v) => {
            let v = v.to_ascii_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        }
    }
}

/// Full capability snapshot for an SSH connection.
pub fn detect_caps(
    term: Option<&str>,
    colorterm: Option<&str>,
    lang: Option<&str>,
    lc_all: Option<&str>,
    width: u16,
    height: u16,
) -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: detect_color_depth(term, colorterm),
        width: if width == 0 { DEFAULT_WIDTH } else { width },
        height: if height == 0 { DEFAULT_HEIGHT } else { height },
        utf8: detect_utf8(lang, lc_all),
        structured: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_depth_table() {
        let cases: &[(Option<&str>, Option<&str>, ColorDepth)] = &[
            (Some("xterm"), None, ColorDepth::Ansi16),
            (Some("xterm-256color"), None, ColorDepth::Ansi256),
            (Some("screen-256color"), None, ColorDepth::Ansi256),
            (Some("tmux-256color"), None, ColorDepth::Ansi256),
            (Some("xterm-direct"), None, ColorDepth::TrueColor),
            (Some("dumb"), None, ColorDepth::None),
            // COLORTERM upgrades…
            (
                Some("xterm-256color"),
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            (Some("xterm"), Some("24bit"), ColorDepth::TrueColor),
            (Some("xterm"), Some("TRUECOLOR"), ColorDepth::TrueColor),
            // …but a dumb terminal stays dumb.
            (Some("dumb"), Some("truecolor"), ColorDepth::None),
            // Unknown COLORTERM values are ignored.
            (Some("xterm"), Some("yes"), ColorDepth::Ansi16),
            // No TERM at all: conservative 16-color.
            (None, None, ColorDepth::Ansi16),
            (None, Some("truecolor"), ColorDepth::TrueColor),
            // Case-insensitive TERM.
            (Some("XTERM-256COLOR"), None, ColorDepth::Ansi256),
        ];
        for (term, colorterm, want) in cases {
            assert_eq!(
                detect_color_depth(*term, *colorterm),
                *want,
                "TERM={term:?} COLORTERM={colorterm:?}"
            );
        }
    }

    #[test]
    fn utf8_table() {
        let cases: &[(Option<&str>, Option<&str>, bool)] = &[
            (None, None, true),
            (Some(""), None, true),
            (Some("en_US.UTF-8"), None, true),
            (Some("C.utf8"), None, true),
            (Some("C"), None, false),
            (Some("POSIX"), None, false),
            (Some("en_US.ISO8859-1"), None, false),
            // LC_ALL overrides LANG.
            (Some("en_US.UTF-8"), Some("C"), false),
            (Some("C"), Some("de_DE.UTF-8"), true),
            (Some("C"), Some(""), false),
        ];
        for (lang, lc_all, want) in cases {
            assert_eq!(
                detect_utf8(*lang, *lc_all),
                *want,
                "LANG={lang:?} LC_ALL={lc_all:?}"
            );
        }
    }

    #[test]
    fn caps_defaults_zero_dims() {
        let caps = detect_caps(Some("xterm"), None, None, None, 0, 0);
        assert_eq!(caps.width, DEFAULT_WIDTH);
        assert_eq!(caps.height, DEFAULT_HEIGHT);
        assert_eq!(caps.kind, ClientKind::Ssh);
        assert!(!caps.structured);
        let caps = detect_caps(Some("xterm"), None, None, None, 120, 40);
        assert_eq!((caps.width, caps.height), (120, 40));
    }
}
