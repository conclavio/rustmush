//! Fuzz-style property tests for NPC action parsing + output hygiene (M9
//! hardening).
//!
//! An NPC's completion is untrusted: the typed-action allowlist is the whole
//! security boundary (design 03 §7). [`parse_actions`] must never panic on
//! arbitrary model output, must return a *bounded* list (`<= MAX_ACTIONS`), and
//! every player-visible field it yields must already be run through
//! [`sanitize_line`] — no control chars, no ANSI escapes, no Unicode
//! bidi/format tricks, and length-capped. Asserted here over random text,
//! random tool-call JSON, and random sanitize inputs.

use mush_llm::{CompletionResponse, ContentBlock, StopReason, Usage};
use mush_npc::{MAX_ACTIONS, NpcAction, SAY_CAP, parse_actions, sanitize_line};
use proptest::prelude::*;

fn resp_text(s: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::text(s)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "mock".into(),
    }
}

fn resp_tools(calls: Vec<(String, serde_json::Value)>) -> CompletionResponse {
    let content = calls
        .into_iter()
        .enumerate()
        .map(|(i, (name, input))| ContentBlock::ToolUse {
            id: i.to_string(),
            name,
            input,
        })
        .collect();
    CompletionResponse {
        content,
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "mock".into(),
    }
}

/// No player-visible string may carry a control char or a Unicode
/// bidi/zero-width format control.
fn assert_clean(s: &str, cap: usize) {
    assert!(
        !s.chars().any(char::is_control),
        "sanitized text kept a control char: {s:?}"
    );
    for c in s.chars() {
        let bad = matches!(c,
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{061C}'
            | '\u{FEFF}'
        );
        assert!(
            !bad,
            "sanitized text kept a bidi/format control U+{:04X}",
            c as u32
        );
    }
    assert!(
        s.chars().count() <= cap + 1,
        "sanitized text over cap+1 ({} > {})",
        s.chars().count(),
        cap + 1
    );
}

/// Validate one cleaned action from `parse_actions`.
fn assert_action_clean(a: &NpcAction) {
    match a {
        NpcAction::Say { text } | NpcAction::Pose { text } => {
            assert!(!text.is_empty());
            assert_clean(text, SAY_CAP);
        }
        NpcAction::Move { exit } => {
            assert!(!exit.is_empty());
            assert_clean(exit, 64);
        }
        NpcAction::LookAt { target } => {
            assert!(!target.is_empty());
            assert_clean(target, 64);
        }
        NpcAction::Remember { subject, note, .. } => {
            assert!(!subject.is_empty() && !note.is_empty());
            assert_clean(subject, 64);
            assert_clean(note, SAY_CAP);
        }
        NpcAction::UseAbility { name, .. } => {
            assert!(!name.is_empty());
            assert_clean(name, 64);
        }
        NpcAction::DoNothing { reason } => assert_clean(reason, 200),
    }
}

fn assert_parse_ok(resp: &CompletionResponse) {
    let (actions, _needs_repair) = parse_actions(resp);
    assert!(
        actions.len() <= MAX_ACTIONS,
        "parse_actions returned {} actions, over MAX_ACTIONS",
        actions.len()
    );
    for a in &actions {
        assert_action_clean(a);
    }
}

/// A small bounded arbitrary JSON value for tool inputs.
fn arb_json() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(|n| serde_json::json!(n)),
        ".{0,32}".prop_map(serde_json::Value::String),
    ];
    leaf.prop_recursive(3, 12, 5, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(serde_json::Value::Array),
            proptest::collection::hash_map("[a-z_]{1,10}", inner, 0..5)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// `sanitize_line` never panics and always produces clean, capped output.
    #[test]
    fn sanitize_is_always_clean(s in ".{0,800}", cap in 1usize..700) {
        let out = sanitize_line(&s, cap);
        assert_clean(&out, cap);
    }

    /// Inputs deliberately packed with ANSI/control/bidi payloads are scrubbed.
    #[test]
    fn sanitize_scrubs_injection_payloads(
        s in proptest::collection::vec(
            prop_oneof![
                Just("\u{1b}[31m"), Just("\u{1b}]0;title\u{7}"), Just("\u{202e}"),
                Just("\u{200b}"), Just("\u{feff}"), Just("\n"), Just("\r"), Just("\t"),
                Just("\u{0}"), Just("hello"), Just("é"), Just(" "),
            ],
            0..64,
        ).prop_map(|p| p.concat()),
        cap in 1usize..700,
    ) {
        let out = sanitize_line(&s, cap);
        assert_clean(&out, cap);
    }

    /// Arbitrary free-text completions (the JSON-fallback path) parse safely.
    #[test]
    fn parse_arbitrary_text_is_safe(s in ".{0,600}") {
        assert_parse_ok(&resp_text(&s));
    }

    /// Text biased toward JSON action shapes exercises the fallback parser far
    /// more, and remains bounded + sanitized.
    #[test]
    fn parse_actionish_text_is_safe(
        s in proptest::collection::vec(
            prop_oneof![
                Just("{"), Just("}"), Just("["), Just("]"), Just("\""),
                Just("action"), Just("say"), Just("pose"), Just("move"),
                Just("text"), Just("exit"), Just(":"), Just(","), Just(" "),
                Just("\u{1b}[0m"), Just("\u{202e}"),
            ],
            0..96,
        ).prop_map(|p| p.concat()),
    ) {
        assert_parse_ok(&resp_text(&s));
    }

    /// Arbitrary tool calls (valid, invented, or the `act` batch tool) parse to
    /// a bounded, sanitized, allowlisted action list — an invented tool can
    /// never escape to a command; it lands on the inert `UseAbility` path.
    #[test]
    fn parse_arbitrary_tool_calls_is_safe(
        calls in proptest::collection::vec(
            (
                prop_oneof![
                    Just("say".to_string()), Just("pose".to_string()),
                    Just("move".to_string()), Just("look_at".to_string()),
                    Just("remember".to_string()), Just("do_nothing".to_string()),
                    Just("act".to_string()), "[a-z_]{1,16}",
                ],
                arb_json(),
            ),
            0..6,
        ),
    ) {
        assert_parse_ok(&resp_tools(calls));
    }
}
