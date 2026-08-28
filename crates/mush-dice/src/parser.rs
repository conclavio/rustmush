//! `winnow` 0.7 grammar for the generic dice expression layer.
//!
//! ```text
//! expr    := ws additive ws
//! additive:= primary ( ws ('+'|'-') ws primary )*      (left-associative)
//! primary := '(' ws additive ws ')' | dice | integer
//! dice    := uint? 'd' uint keep?                       (leading count defaults to 1)
//! keep    := ('kh'|'kl') uint?                          (count defaults to 1)
//! ```
//!
//! Accepts `3d6+2`, `2d10kh1`, `1d20+5`, `4d6kh3`, `d10`, `10 - (2 - 1)`.

use winnow::ascii::{digit1, multispace0};
use winnow::combinator::{alt, delimited, opt, preceded, repeat};
use winnow::token::one_of;
use winnow::{ModalResult, Parser};

use crate::error::DiceParseError;
use crate::expr::{AddOp, DiceExpr, Keep};

/// Hard cap on source length — a cheap guard against pathological parenthesis
/// nesting (which would otherwise recurse per level). Real dice expressions are
/// a handful of characters; this is enormously generous.
const MAX_EXPR_LEN: usize = 512;

/// Parse a dice expression into a [`DiceExpr`] AST.
///
/// Purely syntactic: it does **not** enforce [`crate::RollLimits`] (call
/// [`DiceExpr::check_limits`](crate::DiceExpr::check_limits) for that). The
/// whole input must be consumed or this errors.
pub fn parse(expr: &str) -> Result<DiceExpr, DiceParseError> {
    if expr.len() > MAX_EXPR_LEN {
        return Err(DiceParseError::new(format!(
            "expression too long ({} chars, max {MAX_EXPR_LEN})",
            expr.len()
        )));
    }
    delimited(multispace0, additive, multispace0)
        .parse(expr)
        .map_err(|e| DiceParseError::new(e.to_string()))
}

fn additive(input: &mut &str) -> ModalResult<DiceExpr> {
    let first = primary(input)?;
    let rest: Vec<(char, DiceExpr)> = repeat(
        0..,
        (
            preceded(multispace0, one_of(['+', '-'])),
            preceded(multispace0, primary),
        ),
    )
    .parse_next(input)?;

    let mut acc = first;
    for (op, rhs) in rest {
        let op = if op == '+' { AddOp::Add } else { AddOp::Sub };
        acc = DiceExpr::Binary {
            op,
            lhs: Box::new(acc),
            rhs: Box::new(rhs),
        };
    }
    Ok(acc)
}

fn primary(input: &mut &str) -> ModalResult<DiceExpr> {
    alt((
        delimited(('(', multispace0), additive, (multispace0, ')')),
        dice,
        number,
    ))
    .parse_next(input)
}

fn dice(input: &mut &str) -> ModalResult<DiceExpr> {
    (opt(uint), 'd', uint, opt(keep))
        .map(|(count, _, sides, keep)| DiceExpr::Dice {
            count: count.unwrap_or(1),
            sides,
            keep,
        })
        .parse_next(input)
}

fn number(input: &mut &str) -> ModalResult<DiceExpr> {
    uint.map(|n| DiceExpr::Const(i64::from(n)))
        .parse_next(input)
}

fn keep(input: &mut &str) -> ModalResult<Keep> {
    (alt(("kh", "kl")), opt(uint))
        .map(|(kind, n)| {
            let n = n.unwrap_or(1);
            if kind == "kh" {
                Keep::Highest(n)
            } else {
                Keep::Lowest(n)
            }
        })
        .parse_next(input)
}

fn uint(input: &mut &str) -> ModalResult<u32> {
    digit1.try_map(|s: &str| s.parse::<u32>()).parse_next(input)
}
