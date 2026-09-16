//! mush-dice — dice engine for rustmush.
//!
//! Two layers, both RNG-injectable so tests are deterministic and every roll
//! carries full per-die provenance for the roll log / web panel / audit trail:
//!
//! 1. A **generic expression evaluator** ([`expr`] + [`parser`]): a small
//!    `winnow` grammar over `NdS` dice with optional keep-highest / keep-lowest,
//!    `+`/`-` modifier chains, and parentheses. Serves generic staff rolls, the
//!    Rhai `mush::dice` binding, and TacSim's `2d6` later.
//! 2. **Pool mechanics** ([`pool::PoolMechanic`]): a trait each ruleset
//!    implements. [`owod20::Owod20Pool`] is the Classic World of Darkness 20th
//!    Anniversary (V20/M20/W20) Storyteller-System d10 pool: an ST difficulty,
//!    1s subtract successes, botch = zero successes with a 1 showing, specialty
//!    doubles 10s, Willpower adds one automatic success. [`sr5::Sr5Pool`] is the
//!    Shadowrun 5th Edition d6 hit pool — a genuinely different mechanic: 5s and
//!    6s are hits, more-than-half 1s glitch, a per-test Limit caps hits, and Edge
//!    can push the limit (waive it + explode 6s via the Rule of Six).
//!
//! Rulesets pull the pieces they need. Ordinary tests may inject any `rand`
//! source; authoritative actions use [`with_resolution_stream`] to bridge the
//! kernel's bounded, recorded RNG into the same evaluators.
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

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod error;
pub mod expr;
pub mod owod20;
pub mod parser;
pub mod pool;
pub mod resolution;

pub use error::{DiceParseError, RollLimitError};
pub use expr::{AddOp, DiceExpr, DieRecord, Keep, RollCtx, RollLimits, RollOutcome};
pub use parser::parse;
pub use pool::PoolMechanic;
pub use resolution::with_resolution_stream;
