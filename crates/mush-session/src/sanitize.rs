//! Input sanitation.

/// Strip control characters from one input line, keeping tab.
///
/// Removes every Unicode control character — C0 (`0x00..=0x1F`, which
/// includes ESC `0x1B`, the classic ANSI-injection vector, plus stray
/// CR/LF), DEL (`0x7F`), and C1 (`U+0080..=U+009F`, e.g. the 8-bit CSI
/// `U+009B`) — except `\t`. Stripping ESC leaves any escape-sequence tail
/// (`[31m` …) behind as harmless literal text, exactly the classic MUSH
/// defense: a player can never smuggle terminal control codes into other
/// players' screens via say/pose/names.
///
/// Printable text, including all non-control Unicode, passes through
/// unchanged. Call this on every raw input line before parsing.
pub fn sanitize_line(line: &str) -> String {
    line.chars()
        .filter(|&c| c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_table() {
        let cases: &[(&str, &str)] = &[
            // plain text untouched
            ("look here", "look here"),
            ("", ""),
            // tab survives
            ("col1\tcol2", "col1\tcol2"),
            // ESC stripped; sequence tail left as literals
            ("say \x1b[31mred alert\x1b[0m", "say [31mred alert[0m"),
            // bare ESC
            ("\x1b", ""),
            // CR/LF and NUL stripped
            ("line\r\n", "line"),
            ("a\x00b", "ab"),
            // BEL and backspace stripped
            ("\x07ding\x08", "ding"),
            // DEL stripped
            ("de\x7fl", "del"),
            // C1 controls stripped (8-bit CSI)
            ("\u{9b}31mX", "31mX"),
            // non-control Unicode passes through
            ("café 日本語 — ✓", "café 日本語 — ✓"),
            // mixed
            ("po\x1bse \tgrins\x07", "pose \tgrins"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_line(input), *want, "input {input:?}");
        }
    }
}
