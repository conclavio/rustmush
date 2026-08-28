//! The per-ruleset pool-mechanic trait.
//!
//! A pool mechanic turns a dice *pool size* plus ruleset options into a
//! serializable, provenance-carrying outcome. Rulesets implement one of these
//! per system: [`crate::owod20::Owod20Pool`] here, `GunneryRoll` in
//! `ruleset-btech` later.

use serde::Serialize;

use crate::expr::RollCtx;

/// A dice-pool resolution mechanic.
///
/// The outcome is [`Serialize`] so it can be handed straight to the event bus /
/// web panel. Implementors are `Send + Sync` because they are stored as trait
/// objects on the ruleset registry.
pub trait PoolMechanic: Send + Sync {
    /// System-specific options (difficulty, specialty, willpower, …).
    type Opts;
    /// The structured, serializable result.
    type Outcome: Serialize;

    /// Resolve `pool` dice under `opts`, drawing randomness from `ctx`.
    fn roll(&self, pool: i32, opts: &Self::Opts, ctx: &mut RollCtx<'_>) -> Self::Outcome;
}
