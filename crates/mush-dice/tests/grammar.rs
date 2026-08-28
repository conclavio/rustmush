//! Generic dice-expression layer: grammar, evaluation, keep, provenance, DoS.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use mush_dice::{AddOp, DiceExpr, Keep, RollCtx, RollLimitError, RollLimits, RollOutcome, parse};

fn ctx(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

fn eval(expr: &str, seed: u64) -> RollOutcome {
    let ast = parse(expr).expect("parse");
    let mut rng = ctx(seed);
    let mut c = RollCtx::new(&mut rng);
    ast.eval(&mut c)
}

// ---------------------------------------------------------------- parse trees

#[test]
fn parses_simple_dice() {
    assert_eq!(
        parse("3d6").unwrap(),
        DiceExpr::Dice {
            count: 3,
            sides: 6,
            keep: None
        }
    );
}

#[test]
fn parses_leading_count_default_one() {
    assert_eq!(
        parse("d10").unwrap(),
        DiceExpr::Dice {
            count: 1,
            sides: 10,
            keep: None
        }
    );
}

#[test]
fn parses_keep_highest_and_lowest() {
    assert_eq!(
        parse("2d10kh1").unwrap(),
        DiceExpr::Dice {
            count: 2,
            sides: 10,
            keep: Some(Keep::Highest(1))
        }
    );
    assert_eq!(
        parse("4d6kl2").unwrap(),
        DiceExpr::Dice {
            count: 4,
            sides: 6,
            keep: Some(Keep::Lowest(2))
        }
    );
    // keep count defaults to 1
    assert_eq!(
        parse("4d6kh").unwrap(),
        DiceExpr::Dice {
            count: 4,
            sides: 6,
            keep: Some(Keep::Highest(1))
        }
    );
}

#[test]
fn parses_modifier_chain_left_associative() {
    // 10 - 2 - 1 == (10 - 2) - 1
    let ast = parse("10-2-1").unwrap();
    match ast {
        DiceExpr::Binary {
            op: AddOp::Sub,
            lhs,
            rhs,
        } => {
            assert_eq!(*rhs, DiceExpr::Const(1));
            assert!(matches!(*lhs, DiceExpr::Binary { op: AddOp::Sub, .. }));
        }
        other => panic!("unexpected tree: {other:?}"),
    }
}

#[test]
fn whitespace_is_ignored() {
    assert_eq!(parse("  3d6 + 2 ").unwrap(), parse("3d6+2").unwrap());
    assert_eq!(parse("1d20 + 5").unwrap(), parse("1d20+5").unwrap());
}

// -------------------------------------------------------------- parse errors

#[test]
fn rejects_malformed_expressions() {
    for bad in [
        "", "   ", "abc", "d", "3d", "3d6+", "+3", "(3d6", "3d6)", "3dd6", "3d6 x2",
    ] {
        assert!(parse(bad).is_err(), "expected `{bad}` to fail to parse");
    }
}

#[test]
fn rejects_overlong_input() {
    let huge = "1+".repeat(400) + "1"; // > 512 chars
    assert!(parse(&huge).is_err());
}

// ------------------------------------------------------------------ evaluation

#[test]
fn eval_is_deterministic_for_a_seed() {
    let a = eval("5d10+3", 99);
    let b = eval("5d10+3", 99);
    assert_eq!(a, b);
}

#[test]
fn const_only_has_no_dice() {
    let out = eval("5", 1);
    assert_eq!(out.total, 5);
    assert!(out.dice.is_empty());
    assert!(!out.capped);
}

#[test]
fn modifiers_apply_to_total() {
    // 10 - (2 - 1) == 9 ; 10 - 2 - 1 == 7  (parens regroup)
    assert_eq!(eval("10 - (2 - 1)", 1).total, 9);
    assert_eq!(eval("10 - 2 - 1", 1).total, 7);
    assert_eq!(eval("2+3+4", 1).total, 9);
}

#[test]
fn flat_modifier_adds_to_dice_sum() {
    let out = eval("3d6+2", 7);
    let dice_sum: i64 = out.dice.iter().map(|d| i64::from(d.value)).sum();
    assert_eq!(out.total, dice_sum + 2);
    assert_eq!(out.dice.len(), 3);
}

#[test]
fn per_die_provenance_is_recorded() {
    let out = eval("6d6", 123);
    assert_eq!(out.dice.len(), 6);
    for d in &out.dice {
        assert_eq!(d.sides, 6);
        assert!((1..=6).contains(&d.value), "value {} out of range", d.value);
        assert!(d.kept);
        assert_eq!(d.reroll_of, None);
    }
}

#[test]
fn keep_highest_drops_lowest() {
    // seed 7 d6 stream -> [4,1,1,6]; keep highest 3 => total 4+1+6 = 11.
    let out = eval("4d6kh3", 7);
    assert_eq!(out.dice.len(), 4);
    let kept: Vec<u16> = out
        .dice
        .iter()
        .filter(|d| d.kept)
        .map(|d| d.value)
        .collect();
    assert_eq!(kept.len(), 3);
    assert_eq!(out.total, 11);
    // total is exactly the sum of the kept dice
    let kept_sum: i64 = out
        .dice
        .iter()
        .filter(|d| d.kept)
        .map(|d| i64::from(d.value))
        .sum();
    assert_eq!(out.total, kept_sum);
}

#[test]
fn keep_lowest_keeps_smallest() {
    let out = eval("4d6kl1", 7); // stream [4,1,1,6] -> lowest 1
    assert_eq!(out.dice.iter().filter(|d| d.kept).count(), 1);
    assert_eq!(out.total, 1);
}

#[test]
fn keep_more_than_rolled_keeps_all() {
    let out = eval("2d6kh5", 7);
    assert_eq!(out.dice.iter().filter(|d| d.kept).count(), 2);
}

// ------------------------------------------------------------------ DoS guards

#[test]
fn check_limits_rejects_too_many_dice() {
    let ast = parse("200d6").unwrap();
    assert_eq!(
        ast.check_limits(&RollLimits::default()),
        Err(RollLimitError::TooManyDice {
            requested: 200,
            limit: 100
        })
    );
}

#[test]
fn check_limits_sums_dice_across_terms() {
    let ast = parse("60d6 + 60d6").unwrap();
    assert!(matches!(
        ast.check_limits(&RollLimits::default()),
        Err(RollLimitError::TooManyDice { .. })
    ));
    assert_eq!(ast.dice_count(), 120);
}

#[test]
fn check_limits_rejects_too_many_sides() {
    let ast = parse("1d5000").unwrap();
    assert_eq!(
        ast.check_limits(&RollLimits::default()),
        Err(RollLimitError::TooManySides {
            requested: 5000,
            limit: 1000
        })
    );
}

#[test]
fn check_limits_rejects_zero_sides() {
    let ast = parse("3d0").unwrap();
    assert_eq!(
        ast.check_limits(&RollLimits::default()),
        Err(RollLimitError::ZeroSides)
    );
}

#[test]
fn check_limits_rejects_overcomplex() {
    let ast = parse("1+1+1+1+1+1+1+1+1+1").unwrap();
    let tight = RollLimits {
        max_terms: 3,
        ..RollLimits::default()
    };
    assert!(matches!(
        ast.check_limits(&tight),
        Err(RollLimitError::TooComplex { .. })
    ));
}

#[test]
fn eval_caps_defensively_and_flags() {
    let ast = parse("100d6").unwrap();
    let limits = RollLimits {
        max_dice: 5,
        ..RollLimits::default()
    };
    let mut rng = ctx(1);
    let mut c = RollCtx::with_limits(&mut rng, limits);
    let out = ast.eval(&mut c);
    assert_eq!(out.dice.len(), 5);
    assert!(out.capped);
}

#[test]
fn eval_caps_sides_defensively() {
    let ast = parse("1d100000").unwrap();
    let mut rng = ctx(1);
    let mut c = RollCtx::new(&mut rng); // max_sides 1000
    let out = ast.eval(&mut c);
    assert_eq!(out.dice.len(), 1);
    assert!(out.dice[0].value >= 1 && out.dice[0].value <= 1000);
    assert_eq!(out.dice[0].sides, 1000);
}

#[test]
fn zero_count_rolls_nothing() {
    let out = eval("0d6+4", 1);
    assert_eq!(out.total, 4);
    assert!(out.dice.is_empty());
}
