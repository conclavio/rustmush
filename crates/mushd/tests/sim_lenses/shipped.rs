//! **The shipped mounts' numbers, read off the content** (doc 49 cut CC.0, Q-49-16).
//!
//! A lens that asserts a magazine reads it here, not as a literal, so a retune of a
//! mount or round pack moves only the content pins (the digests, the harness
//! baseline and the corpus pins), never these stories. What each story asserts
//! about the rows is unchanged: a fresh unit reads a full load, a shot spends what
//! its burst spends, and so on.
//!
//! Since doc 51 unit FL.3 the reader is the story kit's [`Figures`], which reads
//! [`tacsim::builtin_definitions`], the catalog the production composition binds
//! `SIM UNIT` and `@unit/create` through, so the story's server and these helpers
//! read one content. This module is only the short spelling the lenses already
//! use. A lens that waits on a cycle takes its limit from the same place
//! ([`Figures::after_one_round`]).

use crate::support::{Figures, Mount};

/// The mount `component` on the shipped unit `unit` (`line_tank`, `scout_mk1`,
/// `heavy_mk1`, ...): its burst, its round's load, its cycle and its muzzle speed.
///
/// # Panics
///
/// When the unit is not fielded, or carries no such mount: a story's premise.
pub fn mount(unit: &str, component: &str) -> Mount {
    Figures::of(&tacsim::TacsimConfig::default()).mount(unit, component)
}
