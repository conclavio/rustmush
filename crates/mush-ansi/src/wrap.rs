//! ANSI-aware width math and word wrapping.
//!
//! Shared publicly because `mush-net-ssh` reuses [`ansi_len`]/[`wrap`] for
//! line-editor redraw math. Escape sequences are treated as zero-width and
//! are never split across lines; wrapped lines are style-self-contained
//! (an SGR style active at a break is closed with a reset and reopened on
//! the continuation line).

/// The SGR reset sequence.
pub const RESET: &str = "\x1b[0m";

/// Whether `c` is a CSI final byte (`0x40..=0x7e`).
fn is_csi_final(c: char) -> bool {
    ('\u{40}'..='\u{7e}').contains(&c)
}

/// The visible width of `s` in terminal columns, counting ANSI escape
/// sequences (CSI and two-byte `ESC x` forms) as zero-width.
///
/// Each non-escape `char` counts as one column: no grapheme clustering or
/// East Asian double-width tables (fine for the classic command surface;
/// revisit if CJK-heavy content lands).
#[must_use]
pub fn ansi_len(s: &str) -> usize {
    let mut n = 0usize;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            // ESC + one char (RIS etc.) is zero-width and fully consumed by
            // the `it.next()`; a CSI (`ESC [`) additionally swallows every
            // byte through its final byte.
            if let Some('[') = it.next() {
                for c2 in it.by_ref() {
                    if is_csi_final(c2) {
                        break;
                    }
                }
            }
        } else {
            n += 1;
        }
    }
    n
}

/// Longest active SGR escape sequence that [`wrap_indent`] will reopen across
/// wrapped lines. Any legitimate SGR emitted by `mush-ansi` (all modifiers +
/// 24-bit fg/bg) is under 50 bytes; capping here prevents quadratic byte
/// amplification from crafted oversized CSI sequences.
const MAX_REOPEN_SGR_BYTES: usize = 64;

/// Track the SGR style left active after scanning `tok`.
///
/// `None` means reset/plain (or an oversized SGR that must not be repeated
/// across wrapped lines). Only whole `ESC [ ... m` sequences update the
/// state; the renderer always emits complete self-describing sequences, so
/// the last non-reset SGR *is* the active style.
fn update_active_sgr(active: &mut Option<String>, tok: &str) {
    let bytes = tok.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let start = i;
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            if i < bytes.len() {
                let fin = bytes[i];
                i += 1;
                if fin == b'm' {
                    let seq = &tok[start..i];
                    if seq == "\x1b[m" || seq == RESET || seq.len() > MAX_REOPEN_SGR_BYTES {
                        *active = None;
                    } else {
                        *active = Some(seq.to_string());
                    }
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Word-wrap `s` (one logical line, no newlines) to `width` visible
/// columns, ANSI-aware.
///
/// - Escape sequences are zero-width and never split.
/// - Break points are spaces only; a word longer than `width` is **not**
///   split — it overflows on its own line (the terminal soft-wraps it).
/// - Runs of spaces are preserved when they stay on one line; a break
///   consumes exactly one space.
/// - A style active at a break is closed with [`RESET`] at the end of the
///   line and reopened at the start of the next, so every returned line is
///   style-self-contained.
/// - `width == 0` means "no wrapping": the input comes back as one line.
///
/// Returned lines carry no line terminators.
#[must_use]
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    wrap_indent(s, width, 0)
}

/// The fewest body columns a hanging indent must leave; below that the
/// indent is dropped rather than squeezing every continuation line into a
/// sliver.
pub const MIN_HANG_BODY: usize = 8;

/// [`wrap`] with a hanging indent: the first line fills `width`, and every
/// continuation line starts with `indent` plain spaces and fills the rest
/// (interface plan I3: a long pose stays one block under its speaker, a
/// labelled room fact wraps under its label).
///
/// The indent is dropped (treated as `0`) when it would leave fewer than
/// [`MIN_HANG_BODY`] columns or exceed half of `width`, and never applies at
/// `width == 0`. The pad is inserted *before* a reopened style, so the indent
/// itself is never coloured. Styles are closed and reopened across breaks only
/// when `width >= MIN_HANG_BODY` and the active SGR is bounded relative to
/// `width`, keeping worst-case output linear even at `width == 1`.
#[must_use]
pub fn wrap_indent(s: &str, width: usize, indent: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let indent = if indent.saturating_add(MIN_HANG_BODY) > width || indent.saturating_mul(2) > width
    {
        0
    } else {
        indent
    };
    let pad = " ".repeat(indent);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_vis = 0usize;
    let mut first_in_line = true;
    // Style open at the current end of `cur`.
    let mut active: Option<String> = None;

    for tok in s.split(' ') {
        let tok_vis = ansi_len(tok);
        if !first_in_line && cur_vis.saturating_add(1).saturating_add(tok_vis) > width {
            let reopen = active.as_deref().filter(|sgr| {
                width >= MIN_HANG_BODY
                    && sgr.len() <= MAX_REOPEN_SGR_BYTES
                    && sgr.len() <= width.saturating_mul(3)
            });
            if reopen.is_some() {
                cur.push_str(RESET);
            }
            lines.push(std::mem::take(&mut cur));
            cur.push_str(&pad);
            if let Some(sgr) = reopen {
                cur.push_str(sgr);
            }
            cur_vis = indent;
            first_in_line = true;
        }
        if !first_in_line {
            cur.push(' ');
            cur_vis += 1;
        }
        cur.push_str(tok);
        cur_vis += tok_vis;
        first_in_line = false;
        update_active_sgr(&mut active, tok);
    }
    lines.push(cur);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_len_counts_visible_only() {
        assert_eq!(ansi_len(""), 0);
        assert_eq!(ansi_len("hello"), 5);
        assert_eq!(ansi_len("\x1b[1;36mhi\x1b[0m"), 2);
        assert_eq!(ansi_len("\x1b[38;2;1;2;3mx\x1b[m"), 1);
        // Unicode chars count one column each.
        assert_eq!(ansi_len("héllo"), 5);
        // Dangling ESC at end is zero-width, not a panic.
        assert_eq!(ansi_len("a\x1b"), 1);
        assert_eq!(ansi_len("a\x1b["), 1);
    }

    #[test]
    fn wrap_plain_words() {
        assert_eq!(
            wrap("the quick brown fox", 10),
            vec!["the quick", "brown fox"]
        );
        assert_eq!(wrap("abc", 10), vec!["abc"]);
        assert_eq!(wrap("", 10), vec![""]);
    }

    #[test]
    fn wrap_zero_width_means_no_wrap() {
        assert_eq!(wrap("a b c d e f", 0), vec!["a b c d e f"]);
    }

    #[test]
    fn wrap_width_one_puts_each_word_on_its_own_line() {
        assert_eq!(wrap("ab cd", 1), vec!["ab", "cd"]);
    }

    #[test]
    fn wrap_never_splits_long_words() {
        let long = "supercalifragilistic";
        assert_eq!(wrap(long, 10), vec![long]);
        assert_eq!(wrap(&format!("aa {long} bb"), 10), vec!["aa", long, "bb"]);
    }

    #[test]
    fn wrap_preserves_space_runs_on_one_line() {
        assert_eq!(wrap("a  b", 10), vec!["a  b"]);
        assert_eq!(wrap("  indent", 10), vec!["  indent"]);
    }

    #[test]
    fn wrap_carries_style_across_breaks() {
        let styled = "\x1b[31mred words here\x1b[0m tail";
        let lines = wrap(styled, 9);
        assert_eq!(
            lines,
            vec!["\x1b[31mred words\x1b[0m", "\x1b[31mhere\x1b[0m tail",]
        );
        // Visible widths respect the limit.
        for l in &lines {
            assert!(ansi_len(l) <= 9, "line too wide: {l:?}");
        }
    }

    #[test]
    fn wrap_indent_pads_continuation_lines_only() {
        let lines = wrap_indent("Nick says, \"the quick brown fox jumps over\"", 20, 5);
        assert_eq!(
            lines,
            vec![
                "Nick says, \"the",
                "     quick brown fox",
                "     jumps over\""
            ]
        );
        for l in &lines {
            assert!(ansi_len(l) <= 20, "line too wide: {l:?}");
        }
        // The indent goes before a reopened style, so it is never coloured.
        let styled = "\x1b[36mNick\x1b[0m \x1b[31mred words here and more\x1b[0m";
        let lines = wrap_indent(styled, 14, 5);
        assert_eq!(lines[0], "\x1b[36mNick\x1b[0m \x1b[31mred words\x1b[0m");
        assert!(lines[1].starts_with("     \x1b[31m"), "{:?}", lines[1]);
        // No indent at width 0, and none when it would starve the body.
        assert_eq!(wrap_indent("a b c", 0, 5), vec!["a b c"]);
        assert_eq!(
            wrap_indent("aaaa bbbb cccc", 9, 4),
            wrap("aaaa bbbb cccc", 9)
        );
        // Indent 0 is exactly `wrap`.
        assert_eq!(
            wrap_indent("the quick brown fox", 10, 0),
            wrap("the quick brown fox", 10)
        );
    }

    #[test]
    fn wrap_does_not_reopen_after_reset() {
        let s = "\x1b[31mred\x1b[0m plain plain plain";
        let lines = wrap(s, 9);
        assert_eq!(lines[0], "\x1b[31mred\x1b[0m plain");
        assert_eq!(lines[1], "plain");
    }

    #[test]
    fn wrap_indent_stays_linear_at_width_one_extreme_indent_and_huge_sgr() {
        // Width 1 with full TrueColor fg+bg+attr SGR previously expanded ~25x.
        let tc = "\x1b[1;2;3;4;7;38;2;255;255;255;48;2;255;255;255m";
        let body = "a ".repeat(1_000);
        let styled = format!("{tc}{body}{RESET}");
        let lines = wrap_indent(&styled, 1, 0);
        let out_bytes: usize = lines.iter().map(String::len).sum();
        assert!(
            out_bytes <= styled.len(),
            "width=1 styled wrap must stay linear, got {out_bytes} bytes from {} bytes",
            styled.len()
        );

        // Extreme indent near width - MIN_HANG_BODY previously expanded ~49x.
        let plain = "word ".repeat(200);
        let wide_indent = wrap_indent(&plain, 400, 392);
        let indent_bytes: usize = wide_indent.iter().map(String::len).sum();
        assert!(
            indent_bytes <= plain.len() * 2,
            "extreme indent must be dropped, got {indent_bytes} bytes from {} bytes",
            plain.len()
        );

        // usize::MAX indent must not overflow.
        assert_eq!(
            wrap_indent("alpha beta gamma", 10, usize::MAX),
            wrap("alpha beta gamma", 10)
        );

        // Crafted multi-kilobyte SGR must not cause quadratic amplification.
        let giant_sgr = format!("\x1b[{}m", "1;".repeat(2_000));
        let hostile = format!("{giant_sgr}{}", "word ".repeat(400));
        let wrapped = wrap_indent(&hostile, 80, 0);
        let hostile_out: usize = wrapped.iter().map(String::len).sum();
        assert!(
            hostile_out <= hostile.len() * 2,
            "oversized SGR must not repeat per line: {hostile_out} vs {}",
            hostile.len()
        );
    }
}
