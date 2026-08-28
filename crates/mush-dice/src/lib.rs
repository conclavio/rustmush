//! mush-dice — dice engine for rustmush.
//!
//! Two layers, both RNG-injectable so tests are deterministic and every roll
//! carries full per-die provenance for the roll log / web panel / audit trail:
//!
//! 1. A **generic expression evaluator** ([`expr`] + [`parser`]): a small
//!    `winnow` grammar over `NdS` dice with optional keep-highest / keep-lowest,
//!    `+`/`-` modifier chains, and parentheses. Serves generic staff rolls, the
//!    Rhai `mush::dice` binding, and BattleTech's `2d6` later.
//! 2. **Pool mechanics** ([`pool::PoolMechanic`]): a trait each ruleset
//!    implements. [`owod20::Owod20Pool`] is the Classic World of Darkness 20th
//!    Anniversary (V20/M20/W20) Storyteller-System d10 pool: an ST difficulty,
//!    1s subtract successes, botch = zero successes with a 1 showing, specialty
//!    doubles 10s, Willpower adds one automatic success.
//!
//! The core (`mush-core`) knows nothing about any of this; rulesets pull the
//! pieces they need. This crate depends on nothing but `rand` + parser deps, so
//! it unit-tests in isolation.
//!
//! ```
//! use rand::SeedableRng;
//! use mush_dice::{parse, RollCtx};
//! use mush_dice::owod20::{Owod20Opts, Owod20Pool};
//! use mush_dice::pool::PoolMechanic;
//!
//! // Generic expression (seeded RNG -> deterministic).
//! let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
//! let mut ctx = RollCtx::new(&mut rng);
//! let expr = parse("2d10kh1 + 3").unwrap();
//! let outcome = expr.eval(&mut ctx);
//! assert_eq!(outcome.dice.len(), 2);
//!
//! // oWoD20 pool: score a known set of faces (RNG-independent, replayable).
//! let out = Owod20Pool::score(&[7, 8, 2, 1], &Owod20Opts::new(6));
//! assert_eq!(out.net, 1); // two successes (7,8) minus one 1
//! ```

pub mod error;
pub mod expr;
pub mod owod20;
pub mod parser;
pub mod pool;

pub use error::{DiceParseError, RollLimitError};
pub use expr::{AddOp, DiceExpr, DieRecord, Keep, RollCtx, RollLimits, RollOutcome};
pub use parser::parse;
pub use pool::PoolMechanic;
