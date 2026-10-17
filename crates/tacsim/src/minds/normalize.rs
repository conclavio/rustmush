//! Source identity normalization for authored Rhai 1.26.0 minds.
//!
//! Only CRLF outside literal text becomes LF. Literal bytes, including escaped
//! continuations, raw strings and backtick text, are copied exactly. Interpolated
//! expressions are code again; nested strings/comments/braces are tracked there.
//! Lone CR is retained, so normalization never invents a new source line.
//!
//! Grammar checked against the pinned Rhai tokenizer's `parse_string_literal`,
//! `parse_raw_string_literal`, `scan_block_comment` and token dispatch. This is
//! a lexical boundary scanner, not a second parser: malformed/unfinished syntax
//! remains for the compiler to diagnose. It does not decode any literal escape.

use std::iter::Peekable;
use std::str::Chars;

use mush_roll::{MAX_SOURCE_BYTES, RollError};

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Zero at the root; positive depth in a `${ ... }` expression.
    Code(usize),
    Quoted(char),
    Backtick,
    Raw(usize),
    LineComment,
    BlockComment(usize),
}

fn copy_chars(input: &mut Peekable<Chars<'_>>, output: &mut String, count: usize) {
    for _ in 0..count {
        if let Some(ch) = input.next() {
            output.push(ch);
        }
    }
}

/// Normalize physical line endings only where Rhai is reading code or comments.
/// The 32 KiB input cap bounds output, lookahead and the non-recursive mode stack.
/// Deleting only CR immediately before LF keeps newline count and token columns.
pub(crate) fn normalize_source(source: &str) -> Result<String, RollError> {
    if source.len() > MAX_SOURCE_BYTES || source.contains('\0') {
        return Err(RollError::Invalid {
            field: "source".into(),
            message: "maximum 32 KiB; NUL is forbidden".into(),
        });
    }
    let mut input = source.chars().peekable();
    let mut output = String::with_capacity(source.len());
    let mut mode = Mode::Code(0);
    let mut parents = Vec::new();
    while let Some(ch) = input.next() {
        let outside_literal = matches!(
            mode,
            Mode::Code(_) | Mode::LineComment | Mode::BlockComment(_)
        );
        if !(outside_literal && ch == '\r' && input.peek() == Some(&'\n')) {
            output.push(ch);
        }
        match mode {
            Mode::Code(braces) => match ch {
                '"' | '\'' => {
                    parents.push(mode);
                    mode = Mode::Quoted(ch);
                }
                '`' => {
                    parents.push(mode);
                    mode = Mode::Backtick;
                }
                '#' if matches!(input.peek(), Some('"' | '#')) => {
                    // Consume a hash run once even when its quote is missing;
                    // rescanning every hash would make invalid input quadratic.
                    let more = input.clone().take_while(|ch| *ch == '#').count();
                    copy_chars(&mut input, &mut output, more);
                    if input.peek() == Some(&'"') {
                        copy_chars(&mut input, &mut output, 1);
                        parents.push(mode);
                        mode = Mode::Raw(more + 1);
                    } else {
                        // Rhai has committed to parsing a raw literal once it
                        // sees `##`. Preserve its unexpected next character and
                        // remaining input for the original compiler diagnostic.
                        output.extend(input);
                        break;
                    }
                }
                '/' if input.peek() == Some(&'/') => {
                    copy_chars(&mut input, &mut output, 1);
                    parents.push(mode);
                    mode = Mode::LineComment;
                }
                '/' if input.peek() == Some(&'*') => {
                    copy_chars(&mut input, &mut output, 1);
                    parents.push(mode);
                    mode = Mode::BlockComment(1);
                }
                '{' if braces > 0 => mode = Mode::Code(braces + 1),
                '}' if braces == 1 => mode = parents.pop().unwrap_or(Mode::Code(0)),
                '}' if braces > 1 => mode = Mode::Code(braces - 1),
                _ => {}
            },
            Mode::Quoted(delimiter) => {
                if ch == '\\' {
                    // Keep an escaped character and a CRLF continuation opaque.
                    // Hex escapes need no interpretation: their digit bytes
                    // cannot open a string or interpolation. Invalid sequences
                    // stay unchanged for Rhai's compiler diagnostics.
                    if let Some(escaped) = input.next() {
                        output.push(escaped);
                        if escaped == '\r' && input.peek() == Some(&'\n') {
                            copy_chars(&mut input, &mut output, 1);
                        }
                    }
                } else if ch == delimiter {
                    if input.peek() == Some(&delimiter) {
                        // Rhai accepts doubled wrappers in quoted/backtick text.
                        copy_chars(&mut input, &mut output, 1);
                    } else {
                        mode = parents.pop().unwrap_or(Mode::Code(0));
                    }
                }
            }
            Mode::Backtick => match ch {
                '`' => {
                    if input.peek() == Some(&'`') {
                        copy_chars(&mut input, &mut output, 1);
                    } else {
                        mode = parents.pop().unwrap_or(Mode::Code(0));
                    }
                }
                '\\' if input.peek() == Some(&'$') && input.clone().nth(1) == Some('{') => {
                    // In verbatim backtick text this escape is special; other
                    // backslashes are ordinary literal text, with no odd/even rule.
                    copy_chars(&mut input, &mut output, 2);
                }
                '$' if input.peek() == Some(&'{') => {
                    copy_chars(&mut input, &mut output, 1);
                    parents.push(mode);
                    mode = Mode::Code(1);
                }
                _ => {}
            },
            Mode::Raw(hashes) => {
                if ch == '"'
                    && input
                        .clone()
                        .take(hashes)
                        .take_while(|ch| *ch == '#')
                        .count()
                        == hashes
                {
                    copy_chars(&mut input, &mut output, hashes);
                    mode = parents.pop().unwrap_or(Mode::Code(0));
                }
            }
            Mode::LineComment => {
                if matches!(ch, '\r' | '\n') {
                    mode = parents.pop().unwrap_or(Mode::Code(0));
                }
            }
            Mode::BlockComment(depth) => {
                if ch == '/' && input.peek() == Some(&'*') {
                    copy_chars(&mut input, &mut output, 1);
                    mode = Mode::BlockComment(depth + 1);
                } else if ch == '*' && input.peek() == Some(&'/') {
                    copy_chars(&mut input, &mut output, 1);
                    mode = if depth == 1 {
                        parents.pop().unwrap_or(Mode::Code(0))
                    } else {
                        Mode::BlockComment(depth - 1)
                    };
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_mind::rhai::{Dynamic, Engine};

    fn normal(source: &str) -> String {
        normalize_source(source).expect("bounded source")
    }

    fn equivalent(source: &str) {
        let normalized = normal(source);
        assert_eq!(normal(&normalized), normalized, "idempotent: {source:?}");
        assert_eq!(
            source.matches('\n').count(),
            normalized.matches('\n').count()
        );
        let engine = Engine::new();
        let before = engine.compile(source).expect("original compiles");
        let after = engine.compile(&normalized).expect("normalized compiles");
        let before = engine
            .eval_ast::<Dynamic>(&before)
            .expect("original evaluates");
        let after = engine
            .eval_ast::<Dynamic>(&after)
            .expect("normalized evaluates");
        assert_eq!(format!("{before:?}"), format!("{after:?}"), "{source:?}");
    }

    #[test]
    fn code_and_nested_comments_normalize_without_rewriting_literal_text() {
        let source = "// quote \" raw ##\" backtick `\r\n/* outer \"\r\n /* inner ` */\r\n */\r\nlet word = `雪\r\n🦀`;\r\nword";
        let expected = "// quote \" raw ##\" backtick `\n/* outer \"\n /* inner ` */\n */\nlet word = `雪\r\n🦀`;\nword";
        assert_eq!(normal(source), expected);
        equivalent(source);
    }

    #[test]
    fn every_literal_family_preserves_runtime_values_and_physical_bytes() {
        for source in [
            "let word = \"a\"\"b\";\r\nword",
            "let word = \"first\\\r\n             second\";\r\nword",
            "let word = `first\r\nsecond`;\r\nword",
            "let word = `\r\nfirst``second\r\nthird`;\r\nword",
            "let word = ##\"first\r\nsecond \"# text\"##;\r\nword",
            "let word = #\"\\n ${not_code} /* text */\r\nraw\"#;\r\nword",
            "let c = '\\'';\r\nc",
            "let c = '\\u0041';\r\nc",
        ] {
            equivalent(source);
            // Each case has only one code CRLF, before its final variable.
            let last = source.rfind("\r\n").expect("code newline");
            let mut expected = source.to_owned();
            expected.remove(last);
            assert_eq!(normal(source), expected, "{source:?}");
        }
    }

    #[test]
    fn interpolation_is_code_and_tracks_nested_maps_strings_and_templates() {
        let source = "let x = 2;\r\n`first\r\n${\r\nlet m = #{x: x};\r\n`inner ${\r\nm.x + 1\r\n}`\r\n}\r\nlast`";
        let expected =
            "let x = 2;\n`first\r\n${\nlet m = #{x: x};\n`inner ${\nm.x + 1\n}`\n}\r\nlast`";
        assert_eq!(normal(source), expected);
        equivalent(source);
    }

    #[test]
    fn escaped_interpolation_does_not_open_code_even_after_another_backslash() {
        for source in [
            "`escaped \\${\r\ntext }`;\r\n42",
            "`escaped \\\\${\r\ntext }`;\r\n42",
            "`other \\$x\r\ntext`;\r\n42",
        ] {
            equivalent(source);
            let last = source.rfind("\r\n").expect("code newline");
            let mut expected = source.to_owned();
            expected.remove(last);
            assert_eq!(normal(source), expected);
        }
    }

    #[test]
    fn compiler_diagnostics_keep_the_same_line_column_and_error() {
        for source in [
            "let answer = 1;\r\nlet broken = ;",
            "let value = \"bad\\q\";\r\n42",
            "let value = \"bad\r\nstill bad;",
            "let value = ##\"raw\r\nno end;",
            "let value = `text\r\n${\r\n2 +\r\n}`;",
            "let value = 'xy';\r\n42",
            "##\r\n",
            "let value = ###\r\n",
        ] {
            let normalized = normal(source);
            let engine = Engine::new();
            let before = engine.compile(source).expect_err("invalid original");
            let after = engine.compile(&normalized).expect_err("invalid normalized");
            assert_eq!(before.position(), after.position(), "{source:?}");
            assert_eq!(before.to_string(), after.to_string(), "{source:?}");
        }
    }

    #[test]
    fn lone_cr_is_not_reinterpreted_and_long_invalid_hash_runs_stay_linear() {
        let source = "// comment\rlet answer = 1;\ranswer";
        assert_eq!(normal(source), source);
        equivalent(source);
        let hashes = "#".repeat(MAX_SOURCE_BYTES);
        assert_eq!(normal(&hashes), hashes);
        assert!(normalize_source(&"x".repeat(MAX_SOURCE_BYTES + 1)).is_err());
        assert!(normalize_source("\0").is_err());
    }
}
