//! Generic dice-expression AST, evaluation context, and evaluator.
//!
//! The AST is produced by [`crate::parser::parse`]. Evaluation walks the tree
//! against a caller-supplied RNG and records every die it rolls.

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::RollLimitError;

/// DoS guards applied during evaluation, held on [`RollCtx`].
///
/// [`DiceExpr::check_limits`] enforces these strictly (returning
/// [`RollLimitError`]); [`DiceExpr::eval`] enforces them defensively by capping
/// (setting [`RollOutcome::capped`]) so it can never allocate unbounded work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollLimits {
    /// Maximum total dice rolled across the whole expression.
    pub max_dice: u32,
    /// Maximum sides on any single die.
    pub max_sides: u32,
    /// Maximum number of AST nodes (guards deep `+`/`-`/paren nesting).
    pub max_terms: u32,
}

impl Default for RollLimits {
    fn default() -> Self {
        Self {
            max_dice: 100,
            max_sides: 1000,
            max_terms: 64,
        }
    }
}

/// Everything a roll needs: an injectable RNG plus the DoS limits.
///
/// The RNG is a trait object so the server can seed one `ChaCha` per process
/// while tests inject a fixed seed for reproducible golden results.
pub struct RollCtx<'r> {
    pub rng: &'r mut dyn RngCore,
    pub limits: RollLimits,
}

impl<'r> RollCtx<'r> {
    /// Context with [default limits](RollLimits::default).
    pub fn new(rng: &'r mut dyn RngCore) -> Self {
        Self {
            rng,
            limits: RollLimits::default(),
        }
    }

    /// Context with explicit limits.
    pub fn with_limits(rng: &'r mut dyn RngCore, limits: RollLimits) -> Self {
        Self { rng, limits }
    }

    /// Roll a single die of `sides` faces, yielding `1..=sides` (`0` if
    /// `sides == 0`). Uniform modulo reduction — the tiny high-face bias is
    /// irrelevant for game dice and keeps the byte stream deterministic.
    fn roll_die(&mut self, sides: u32) -> u16 {
        if sides == 0 {
            0
        } else {
            (self.rng.next_u32() % sides) as u16 + 1
        }
    }
}

/// Keep-highest / keep-lowest selector on a dice group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Keep {
    Highest(u32),
    Lowest(u32),
}

/// Additive operator in a `+`/`-` chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddOp {
    Add,
    Sub,
}

/// A parsed dice expression.
///
/// `Dice { count: 0, .. }` (`0d6`) is legal and contributes nothing; a
/// zero-`count` or zero-`sides` group simply rolls no meaningful die.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiceExpr {
    /// An integer literal.
    Const(i64),
    /// `NdS` with an optional keep selector.
    Dice {
        count: u32,
        sides: u32,
        keep: Option<Keep>,
    },
    /// `lhs + rhs` or `lhs - rhs` (left-associative; parentheses regroup).
    Binary {
        op: AddOp,
        lhs: Box<DiceExpr>,
        rhs: Box<DiceExpr>,
    },
}

/// One physical die roll, retained for provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DieRecord {
    pub sides: u16,
    pub value: u16,
    /// Whether this die counted toward the total (false when dropped by keep).
    pub kept: bool,
    /// If this die replaced another (explode/reroll), the index it came from.
    /// Always `None` for the current grammar; reserved for future mechanics.
    pub reroll_of: Option<u16>,
}

/// The result of evaluating a [`DiceExpr`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollOutcome {
    /// Sum of kept dice plus/minus modifiers.
    pub total: i64,
    /// Every die rolled, in evaluation order, kept or not.
    pub dice: Vec<DieRecord>,
    /// True if [`RollLimits`] forced fewer dice than the expression asked for.
    pub capped: bool,
}

impl DiceExpr {
    /// Evaluate against `ctx`, rolling dice and accumulating the total.
    ///
    /// Infallible: limits are enforced defensively by capping the dice rolled
    /// (see [`RollOutcome::capped`]) so this can never do unbounded work. Use
    /// [`check_limits`](Self::check_limits) first if you want to *reject*
    /// oversized expressions instead of silently capping them.
    pub fn eval(&self, ctx: &mut RollCtx) -> RollOutcome {
        let mut dice = Vec::new();
        let mut budget = ctx.limits.max_dice;
        let mut capped = false;
        let total = self.eval_node(ctx, &mut dice, &mut budget, &mut capped);
        RollOutcome {
            total,
            dice,
            capped,
        }
    }

    fn eval_node(
        &self,
        ctx: &mut RollCtx,
        dice: &mut Vec<DieRecord>,
        budget: &mut u32,
        capped: &mut bool,
    ) -> i64 {
        match self {
            DiceExpr::Const(v) => *v,
            DiceExpr::Dice { count, sides, keep } => {
                roll_group(*count, *sides, *keep, ctx, dice, budget, capped)
            }
            DiceExpr::Binary { op, lhs, rhs } => {
                let l = lhs.eval_node(ctx, dice, budget, capped);
                let r = rhs.eval_node(ctx, dice, budget, capped);
                match op {
                    AddOp::Add => l.saturating_add(r),
                    AddOp::Sub => l.saturating_sub(r),
                }
            }
        }
    }

    /// Strictly validate against `limits` without rolling anything.
    ///
    /// Returns the first violation found. This is the DoS gate a command
    /// handler runs before [`eval`](Self::eval) so a hostile `999999d1000` is
    /// rejected with a message rather than capped.
    pub fn check_limits(&self, limits: &RollLimits) -> Result<(), RollLimitError> {
        let mut total_dice: u64 = 0;
        let mut nodes: u32 = 0;
        self.walk(&mut |node| {
            nodes = nodes.saturating_add(1);
            if nodes > limits.max_terms {
                return Err(RollLimitError::TooComplex {
                    nodes,
                    limit: limits.max_terms,
                });
            }
            if let DiceExpr::Dice { count, sides, .. } = node {
                if *sides == 0 {
                    return Err(RollLimitError::ZeroSides);
                }
                if *sides > limits.max_sides {
                    return Err(RollLimitError::TooManySides {
                        requested: *sides,
                        limit: limits.max_sides,
                    });
                }
                total_dice = total_dice.saturating_add(u64::from(*count));
                if total_dice > u64::from(limits.max_dice) {
                    return Err(RollLimitError::TooManyDice {
                        requested: total_dice,
                        limit: limits.max_dice,
                    });
                }
            }
            Ok(())
        })
    }

    /// Total dice this expression would roll if unconstrained.
    pub fn dice_count(&self) -> u64 {
        let mut total = 0u64;
        // Infallible walk; the closure never returns Err.
        let _ = self.walk(&mut |node| {
            if let DiceExpr::Dice { count, .. } = node {
                total = total.saturating_add(u64::from(*count));
            }
            Ok::<(), RollLimitError>(())
        });
        total
    }

    /// Pre-order visit of every node.
    fn walk<E>(&self, f: &mut impl FnMut(&DiceExpr) -> Result<(), E>) -> Result<(), E> {
        f(self)?;
        if let DiceExpr::Binary { lhs, rhs, .. } = self {
            lhs.walk(f)?;
            rhs.walk(f)?;
        }
        Ok(())
    }
}

/// Roll one `NdS` group, apply the keep selector, and return the kept sum.
///
/// `budget` bounds the total dice a whole expression may roll; when a group
/// would exceed it the group is truncated and `capped` is set.
fn roll_group(
    count: u32,
    sides: u32,
    keep: Option<Keep>,
    ctx: &mut RollCtx,
    dice: &mut Vec<DieRecord>,
    budget: &mut u32,
    capped: &mut bool,
) -> i64 {
    let sides = sides.min(ctx.limits.max_sides);
    let mut n = count;
    if n > *budget {
        n = *budget;
        *capped = true;
    }
    *budget -= n;

    let start = dice.len();
    let recorded_sides = sides.min(u32::from(u16::MAX)) as u16;
    for _ in 0..n {
        let value = ctx.roll_die(sides);
        dice.push(DieRecord {
            sides: recorded_sides,
            value,
            kept: true,
            reroll_of: None,
        });
    }
    apply_keep(&mut dice[start..], keep)
}

/// Mark which dice in `group` are kept per `keep`, and return their value sum.
fn apply_keep(group: &mut [DieRecord], keep: Option<Keep>) -> i64 {
    let Some(keep) = keep else {
        return group.iter().map(|d| i64::from(d.value)).sum();
    };
    let (want, highest) = match keep {
        Keep::Highest(n) => (n as usize, true),
        Keep::Lowest(n) => (n as usize, false),
    };

    let mut order: Vec<usize> = (0..group.len()).collect();
    order.sort_by_key(|&i| group[i].value); // ascending
    if highest {
        order.reverse();
    }

    for d in group.iter_mut() {
        d.kept = false;
    }
    let keep_count = want.min(group.len());
    let mut sum = 0i64;
    for &i in order.iter().take(keep_count) {
        group[i].kept = true;
        sum += i64::from(group[i].value);
    }
    sum
}
