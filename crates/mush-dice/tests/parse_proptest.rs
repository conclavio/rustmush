//! Fuzz-style property tests for the dice grammar + evaluator (M9 hardening).
//!
//! `+roll <expr>` hands attacker-controlled text straight to [`parse`]; the
//! resulting AST is then evaluated under [`RollLimits`]. Neither step may
//! panic, hang, or allocate unbounded work: the parser has an input-length
//! guard, and `eval` defends by *capping* the dice it rolls. These properties
//! are asserted over random strings and over a generator of syntactically
//! valid expressions.

use mush_dice::{DiceExpr, RollCtx, RollLimits, parse};
use proptest::prelude::*;
use rand::SeedableRng;

fn ctx_eval(expr: &DiceExpr, seed: u64, limits: RollLimits) -> mush_dice::RollOutcome {
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let mut ctx = RollCtx::with_limits(&mut rng, limits);
    expr.eval(&mut ctx)
}

/// Full evaluation invariants for a parsed expression.
fn assert_eval_invariants(expr: &DiceExpr, seed: u64) {
    let limits = RollLimits::default();
    let outcome = ctx_eval(expr, seed, limits);

    // Never roll more dice than the budget — the anti-DoS guarantee.
    assert!(
        outcome.dice.len() <= limits.max_dice as usize,
        "eval rolled {} dice, over the {} cap",
        outcome.dice.len(),
        limits.max_dice
    );
    // Every recorded die is a legal face for its recorded sides.
    for d in &outcome.dice {
        if d.sides == 0 {
            assert_eq!(d.value, 0);
        } else {
            assert!(
                d.value >= 1 && d.value <= d.sides,
                "die out of range: {d:?}"
            );
        }
    }
    // check_limits is the strict gate; when it passes, eval never caps.
    if expr.check_limits(&limits).is_ok() {
        assert!(
            !outcome.capped,
            "check_limits passed but eval still capped: {expr:?}"
        );
        assert_eq!(
            outcome.dice.len() as u64,
            expr.dice_count(),
            "uncapped eval must roll exactly dice_count() dice"
        );
    }
}

/// A generator of syntactically valid dice-expression strings, kept small
/// enough to stay well inside the parser's length and complexity guards.
fn valid_expr_string() -> impl Strategy<Value = String> {
    let dice = (
        0u32..200,
        1u32..2000,
        prop_oneof![
            Just(String::new()),
            (0u32..8).prop_map(|k| format!("kh{k}")),
            (0u32..8).prop_map(|k| format!("kl{k}")),
        ],
    )
        .prop_map(|(count, sides, keep)| format!("{count}d{sides}{keep}"));
    let konst = (0u32..1_000_000).prop_map(|n| n.to_string());
    let leaf = prop_oneof![dice, konst];
    leaf.prop_recursive(4, 24, 3, |inner| {
        (inner.clone(), prop_oneof![Just('+'), Just('-')], inner)
            .prop_map(|(a, op, b)| format!("({a} {op} {b})"))
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Arbitrary strings: parse never panics, and any AST it yields evaluates
    /// within limits without panicking.
    #[test]
    fn parse_arbitrary_never_panics(s in ".{0,600}", seed in any::<u64>()) {
        if let Ok(expr) = parse(&s) {
            assert_eval_invariants(&expr, seed);
        }
    }

    /// A hostile alphabet biased toward grammar tokens (`d`, digits, `kh`/`kl`,
    /// parens, operators, whitespace) hits far more of the parser.
    #[test]
    fn parse_token_soup_never_panics(
        s in proptest::collection::vec(
            prop_oneof![
                Just("d"), Just("kh"), Just("kl"), Just("("), Just(")"),
                Just("+"), Just("-"), Just(" "), Just("0"), Just("9"),
                Just("99"), Just("d6"), Just("2d10"),
            ],
            0..64,
        ).prop_map(|parts| parts.concat()),
        seed in any::<u64>(),
    ) {
        if let Ok(expr) = parse(&s) {
            assert_eval_invariants(&expr, seed);
        }
    }

    /// Valid expressions always parse and always satisfy the eval invariants.
    #[test]
    fn valid_expressions_parse_and_eval(s in valid_expr_string(), seed in any::<u64>()) {
        // The generator targets a bounded size but can overshoot; keep only the
        // ones inside the parser's length guard (the guard itself is covered by
        // `overlong_input_is_rejected`).
        prop_assume!(s.len() <= 500);
        let expr = parse(&s).unwrap_or_else(|e| panic!("valid expr {s:?} failed to parse: {e}"));
        assert_eval_invariants(&expr, seed);
    }

    /// The input-length guard rejects oversized input before doing any real
    /// work (pathological paren nesting can't recurse without bound).
    #[test]
    fn overlong_input_is_rejected(pad in 513usize..4096) {
        let s = "(".repeat(pad);
        prop_assert!(parse(&s).is_err());
    }

    /// A single huge dice group is capped, never rolled in full — even asking
    /// for billions of dice returns a bounded outcome.
    #[test]
    fn huge_pool_is_capped(count in 200u32..1_000_000, sides in 2u32..20, seed in any::<u64>()) {
        let expr = parse(&format!("{count}d{sides}")).expect("parses");
        let limits = RollLimits::default();
        let outcome = ctx_eval(&expr, seed, limits);
        prop_assert!(outcome.dice.len() <= limits.max_dice as usize);
        prop_assert!(outcome.capped);
    }
}
