//! Fuzz-style property tests for the lock-expression parser (M9 hardening).
//!
//! `@lock <expr>` parses attacker-controlled TinyMUX lock syntax. The parser
//! must never panic on arbitrary input, and — the load-bearing invariant the
//! module documents — every AST it produces must satisfy
//! `parse(print(e)) == e`, so a stored lock survives a serialize/reload cycle
//! without silently changing meaning. Both are asserted here over random
//! strings and over a generator of valid lock expressions.

use mush_core::{LockExpr, ObjId, parse_lock};
use proptest::prelude::*;

/// A resolver that accepts any non-empty name (maps them all to `#1`) so more
/// inputs reach the printer/round-trip check. `#dbref` forms bypass it.
fn resolve_any(_name: &str) -> Option<ObjId> {
    Some(ObjId(1))
}

/// Parse, then assert the documented round-trip: printing and reparsing yields
/// the identical AST, and printing is idempotent.
fn assert_round_trip(expr: &LockExpr) {
    let printed = expr.to_string();
    let reparsed = parse_lock(&printed, resolve_any)
        .unwrap_or_else(|e| panic!("reparse of printed {printed:?} failed: {e}"));
    assert_eq!(
        &reparsed, expr,
        "AST changed across print/reparse: {printed:?}"
    );
    assert_eq!(
        reparsed.to_string(),
        printed,
        "print not idempotent for {printed:?}"
    );
}

/// A generator of valid lock-expression strings built from the grammar, so the
/// deeper boolean/precedence structure is exercised (not just single terms).
fn valid_lock_string() -> impl Strategy<Value = String> {
    let ident = "[a-zA-Z][a-zA-Z0-9_]{0,7}";
    let glob = "[a-zA-Z0-9_*?]{0,8}";
    let lock_type = prop_oneof![
        Just("basic"),
        Just("enter"),
        Just("use"),
        Just("page"),
        Just("teleport"),
    ];
    let atom = prop_oneof![
        Just("#TRUE".to_string()),
        Just("#FALSE".to_string()),
        Just("me".to_string()),
        (0u32..9999).prop_map(|n| format!("#{n}")),
        (0u32..9999).prop_map(|n| format!("+#{n}")),
        (0u32..9999).prop_map(|n| format!("$#{n}")),
        ident.prop_map(|s: String| format!("flag^{s}")),
        (ident, glob).prop_map(|(n, g): (String, String)| format!("{n}:{g}")),
        (0u32..9999, lock_type).prop_map(|(n, t)| format!("@#{n}/{t}")),
    ];
    atom.prop_recursive(4, 32, 3, |inner| {
        prop_oneof![
            inner.clone().prop_map(|e| format!("!{e}")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a} & {b}")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a} | {b}")),
            inner.prop_map(|e| format!("({e})")),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Arbitrary strings never panic; any AST that parses round-trips.
    #[test]
    fn parse_arbitrary_never_panics(s in ".{0,160}") {
        if let Ok(expr) = parse_lock(&s, resolve_any) {
            assert_round_trip(&expr);
        }
    }

    /// A hostile alphabet biased toward lock operators and prefixes.
    #[test]
    fn parse_operator_soup_never_panics(
        s in proptest::collection::vec(
            prop_oneof![
                Just("&"), Just("|"), Just("!"), Just("("), Just(")"),
                Just("#"), Just("+"), Just("$"), Just("@"), Just("^"), Just(":"),
                Just("/"), Just(" "), Just("flag"), Just("me"), Just("1"),
                Just("#TRUE"), Just("basic"),
            ],
            0..48,
        ).prop_map(|parts| parts.concat()),
    ) {
        if let Ok(expr) = parse_lock(&s, resolve_any) {
            assert_round_trip(&expr);
        }
    }

    /// Valid lock strings always parse and always round-trip.
    #[test]
    fn valid_locks_parse_and_round_trip(s in valid_lock_string()) {
        let expr = parse_lock(&s, resolve_any)
            .unwrap_or_else(|e| panic!("valid lock {s:?} failed to parse: {e}"));
        assert_round_trip(&expr);
    }
}
