//! Fuzz-style property tests for worldgen's lenient draft extraction (M9
//! hardening).
//!
//! `@gen` feeds raw *model output* — which is adversarial in the same way user
//! input is (a jailbroken or simply broken model can return anything) — through
//! [`extract_json_object`] and a [`DraftPayload`] decode. The balanced-brace
//! scanner must never panic or mis-slice on arbitrary bytes (including braces
//! inside strings and multi-byte UTF-8), and the typed decode must reject
//! garbage as `Err` rather than crash.

use mush_worldgen::{DraftPayload, extract_json_object};
use proptest::prelude::*;

/// Whatever the scanner returns must be a well-formed, in-bounds `{…}` slice.
fn assert_object_shape(text: &str) {
    if let Some(obj) = extract_json_object(text) {
        assert!(
            obj.starts_with('{'),
            "object did not start with '{{': {obj:?}"
        );
        assert!(obj.ends_with('}'), "object did not end with '}}': {obj:?}");
        // It is a genuine slice of the input (no fabricated/renumbered bytes).
        assert!(
            text.contains(obj),
            "returned slice is not a substring of the input"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Arbitrary bytes/text never crash the scanner or the typed decode.
    #[test]
    fn arbitrary_text_never_panics(s in ".{0,512}") {
        assert_object_shape(&s);
        let _ = serde_json::from_str::<DraftPayload>(&s);
        if let Some(obj) = extract_json_object(&s) {
            // Feeding the extracted object back to the typed decode is exactly
            // what generate.rs does; it must not panic either.
            let _ = serde_json::from_str::<DraftPayload>(obj);
        }
    }

    /// A brace-heavy alphabet (the scanner's string/escape/nesting logic) is
    /// exercised directly.
    #[test]
    fn brace_soup_never_panics(
        s in proptest::collection::vec(
            prop_oneof![
                Just("{"), Just("}"), Just("\""), Just("\\"), Just(":"),
                Just(","), Just("[" ), Just("]"), Just("desc"), Just(" "),
                Just("é"), Just("🎲"), Just("\n"),
            ],
            0..96,
        ).prop_map(|parts| parts.concat()),
    ) {
        assert_object_shape(&s);
        let _ = serde_json::from_str::<DraftPayload>(&s);
    }

    /// A valid draft embedded in surrounding prose is recovered and decodes,
    /// even when its `desc` contains stray braces and quotes.
    #[test]
    fn embedded_valid_draft_is_recovered(
        desc in "[a-zA-Z0-9 {}]{1,40}",
        prefix in "[a-zA-Z ]{0,20}",
        suffix in "[a-zA-Z ]{0,20}",
    ) {
        let payload = serde_json::json!({ "desc": desc }).to_string();
        let text = format!("{prefix} {payload} {suffix}");
        let obj = extract_json_object(&text).expect("an object is present");
        let draft: DraftPayload =
            serde_json::from_str(obj).expect("embedded draft must decode");
        prop_assert_eq!(draft.desc, desc);
    }
}
