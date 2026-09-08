//! One nesting-depth primitive for stored documents.
//!
//! Every document this server persists is JSON, and every decoder that reads
//! one back walks it *recursively* — `serde_json`'s own parser included. A
//! deep-enough document is therefore a stack hazard on the single world task,
//! which may never panic and may never blow its guard page. `serde_json`
//! caps its recursion at 128 levels, which keeps the decoder itself honest,
//! but that bound is the library's, not ours: it is not a number this crate
//! can state, refuse against, or report on.
//!
//! So depth becomes one constant ([`MAX_DOC_DEPTH`]) in one crate, plus three
//! measurements of it:
//!
//! - [`depth_of_json_text`] — a byte scan over document *text*, for the store
//!   seams and the ops diagnostics, which must be able to measure a document
//!   that is *too deep to decode* (the case that matters most) and must not
//!   themselves recurse to do it;
//! - [`depth_of_value`] — the same number for an already-decoded
//!   [`serde_json::Value`];
//! - [`depth_of_lock`] — the structural depth of a [`LockExpr`] tree, the one
//!   hand-built tree shape that reaches RAM without ever having been text;
//!   this one delegates to [`LockExpr::depth`], which already exists and
//!   already walks by worklist, because a second walker over the same tree
//!   would only drift.
//!
//! **None of the three recurses.** Each walks an explicit worklist or a flat
//! byte scan, so measuring a hostile document costs heap, not stack: the whole
//! point of a depth guard is defeated if the guard overflows first.
//!
//! Depth is counted in *containers*: a scalar document is depth 0, `{}` and
//! `[1,2]` are depth 1, `{"a":{"b":1}}` is depth 2. Nothing here refuses
//! anything — a bound with no policy attached is deliberate: callers
//! (mush-store on the write path, the ops preflight on the read path) decide
//! what a violation means to them.

use crate::lock::LockExpr;

/// The deepest document this server will accept as sound.
///
/// Chosen to sit clearly **below** `serde_json`'s 128-level recursion limit:
/// a document at or under this depth decodes on every path that will ever see
/// it, while the headroom between 100 and 128 means a refusal here is a
/// deliberate, reportable answer rather than a decoder error surfacing from
/// somewhere further in. It is not tuned to any live store — no store has
/// run yet — it is picked to be comfortably above every document this server
/// constructs (the deepest is a lock tree, itself bounded at 48 by
/// [`crate::lock::MAX_LOCK_EXPR_DEPTH`]) and comfortably below the depth at
/// which the decoders give out.
pub const MAX_DOC_DEPTH: usize = 100;

/// Maximum container nesting in `text`, by a flat byte scan.
///
/// Braces and brackets inside string literals do not count, and a `\`-escaped
/// character inside a string is never a closing quote — those two rules are
/// the whole of JSON's lexical structure that depth needs.
///
/// This is deliberately *not* a validator: it does not care whether `text` is
/// well-formed JSON, and unbalanced closers saturate at 0 rather than
/// underflowing. Its job is to answer "how deep is this?" for text that may
/// be exactly the text no decoder will accept.
#[must_use]
pub fn depth_of_json_text(text: &str) -> usize {
    let mut depth: usize = 0;
    let mut max: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    for byte in text.as_bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Maximum container nesting in an already-decoded value, by an explicit
/// worklist. Agrees with [`depth_of_json_text`] on the same document.
#[must_use]
pub fn depth_of_value(value: &serde_json::Value) -> usize {
    let mut max: usize = 0;
    // (node, depth this node sits at if it is a container).
    let mut work: Vec<(&serde_json::Value, usize)> = vec![(value, 1)];
    while let Some((node, depth)) = work.pop() {
        match node {
            serde_json::Value::Object(map) => {
                max = max.max(depth);
                work.extend(map.values().map(|child| (child, depth.saturating_add(1))));
            }
            serde_json::Value::Array(items) => {
                max = max.max(depth);
                work.extend(items.iter().map(|child| (child, depth.saturating_add(1))));
            }
            _ => {}
        }
    }
    max
}

/// Structural depth of a lock tree, in the same units the rest of this
/// module counts: how many [`LockExpr`] nodes deep its longest branch runs,
/// a leaf counting 1.
///
/// A thin adapter over [`LockExpr::depth`], which already walks the tree by
/// explicit worklist for exactly this module's reason — a tree decoded from
/// the store or built by a module never went through
/// [`crate::lock::parse_lock`], so measuring it must not be the thing that
/// overflows. It lives here so a caller weighing a document against
/// [`MAX_DOC_DEPTH`] finds all three shapes behind one door, and so that the
/// one walk stays in `lock.rs`, the only file allowed to take a lock tree
/// apart (see `tests/lock_construction_ratchet.rs`).
///
/// The number counts *expression* nodes, not the JSON levels the tree
/// serializes into — a leaf that carries fields (`AttrMatch`, `Indirect`) is
/// one more JSON level than it is a lock level — so it is a lower bound on
/// the stored document's depth, which is the safe direction for a guard.
#[must_use]
pub fn depth_of_lock(expr: &LockExpr) -> usize {
    usize::try_from(expr.depth()).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::parse_lock;
    use crate::obj::ObjId;

    /// `{`/`[` inside a string literal is text, not structure — and a `\"`
    /// does not end the literal it sits in.
    #[test]
    fn braces_inside_strings_do_not_count() {
        assert_eq!(depth_of_json_text(r#"{"a":"{{{[[["}"#), 1);
        assert_eq!(depth_of_json_text(r#"{"a":"he said \"{[\" once"}"#), 1);
        // An escaped backslash ends the escape: the quote after it closes.
        assert_eq!(depth_of_json_text(r#"{"a":"back\\","b":[1]}"#), 2);
    }

    /// The text scan and the value walk are two measurements of one number.
    #[test]
    fn text_scan_agrees_with_value_walk() {
        for doc in [
            "null",
            "42",
            r#""{[{[""#,
            "{}",
            "[]",
            r#"{"a":1}"#,
            r#"{"a":{"b":[1,2,{"c":[]}]},"d":[[[]]]}"#,
            r#"[{"a":[{"b":{}}]},{}]"#,
        ] {
            let value: serde_json::Value = serde_json::from_str(doc).expect("valid fixture json");
            assert_eq!(
                depth_of_json_text(doc),
                depth_of_value(&value),
                "disagreement on {doc}"
            );
        }
    }

    fn nested_object_text(depth: usize) -> String {
        let mut text = String::with_capacity(depth * 8);
        for _ in 0..depth {
            text.push_str(r#"{"a":"#);
        }
        text.push('1');
        for _ in 0..depth {
            text.push('}');
        }
        text
    }

    /// The boundary is a fact about the constant, pinned in both directions:
    /// a document at `MAX_DOC_DEPTH` measures exactly that, one level deeper
    /// measures over it.
    #[test]
    fn max_doc_depth_boundary_is_pinned() {
        assert_eq!(MAX_DOC_DEPTH, 100);
        let at = nested_object_text(MAX_DOC_DEPTH);
        let over = nested_object_text(MAX_DOC_DEPTH + 1);
        assert_eq!(depth_of_json_text(&at), 100);
        assert!(depth_of_json_text(&at) <= MAX_DOC_DEPTH);
        assert_eq!(depth_of_json_text(&over), 101);
        assert!(depth_of_json_text(&over) > MAX_DOC_DEPTH);
        // And the constant stays under serde_json's own recursion limit, so
        // a document at the bound is still decodable.
        serde_json::from_str::<serde_json::Value>(&at).expect("a doc at the bound decodes");
    }

    /// The whole point of the byte scan: a document far too deep for any
    /// decoder is still measurable, and measuring it does not recurse.
    #[test]
    fn two_hundred_thousand_levels_do_not_overflow_the_stack() {
        let text: String = "[".repeat(200_000) + &"]".repeat(200_000);
        assert_eq!(depth_of_json_text(&text), 200_000);
        // Unbalanced closers saturate instead of underflowing.
        assert_eq!(depth_of_json_text(&"]".repeat(200_000)), 0);
    }

    /// `depth_of_lock` is `LockExpr::depth` in this module's units, leaf
    /// included. The tall-tree proof that the walk does not recurse lives
    /// with the walk, in `tests/lock_depth_*.rs`.
    #[test]
    fn lock_depth_counts_expression_levels() {
        for (src, expect) in [("#1", 1), ("!#1", 2), ("#1 & !(#2 | #3)", 4)] {
            let expr = parse_lock(src, |_| Some(ObjId(1))).expect("fixture lock parses");
            assert_eq!(depth_of_lock(&expr), expect, "on {src}");
            assert_eq!(
                depth_of_lock(&expr),
                usize::try_from(expr.depth()).expect("depth fits"),
            );
        }
    }
}
