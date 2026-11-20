//! Input sanitation.

/// Whether `c` is a Unicode bidirectional formatting control or line/paragraph
/// separator that can spoof visual text direction or break single-line framing
/// (SSH-M6):
/// - `U+061C` (Arabic Letter Mark)
/// - `U+200E..=U+200F` (LRM, RLM)
/// - `U+2028..=U+2029` (Line Separator, Paragraph Separator)
/// - `U+202A..=U+202E` (LRE, RLE, PDF, LRO, RLO)
/// - `U+2066..=U+2069` (LRI, RLI, FSI, PDI)
const fn is_forbidden_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200E}'..='\u{200F}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Strip control characters and Unicode BiDi/line-separator controls from one
/// input line, keeping tab.
///
/// Removes every Unicode control character — C0 (`0x00..=0x1F`, which
/// includes ESC `0x1B`, the classic ANSI-injection vector, plus stray
/// CR/LF), DEL (`0x7F`), and C1 (`U+0080..=U+009F`, e.g. the 8-bit CSI
/// `U+009B`) — except `\t`, along with Unicode BiDi override/isolate/mark
/// controls (`U+061C`, `U+200E..=U+200F`, `U+202A..=U+202E`,
/// `U+2066..=U+2069`) and Unicode line/paragraph separators (`U+2028`,
/// `U+2029`). Stripping ESC leaves any escape-sequence tail (`[31m` …)
/// behind as harmless literal text, exactly the classic MUSH defense: a
/// player can never smuggle terminal control codes or BiDi spoofs into other
/// players' screens via say/pose/names.
///
/// Printable text, including all non-control Unicode, passes through
/// unchanged. Call this on every raw input line before parsing.
pub fn sanitize_line(line: &str) -> String {
    line.chars()
        .filter(|&c| (c == '\t' || !c.is_control()) && !is_forbidden_format_char(c))
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
            // SSH-M6: Unicode BiDi controls and line/paragraph separators stripped
            ("a\u{061C}b", "ab"),
            ("a\u{200E}b\u{200F}c", "abc"),
            ("a\u{2028}b\u{2029}c", "abc"),
            ("x\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}y", "xy"),
            ("p\u{2066}\u{2067}\u{2068}\u{2069}q", "pq"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_line(input), *want, "input {input:?}");
        }
    }
}
