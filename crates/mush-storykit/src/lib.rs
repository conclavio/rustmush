//! **The story kit** (doc 51 §3.8, Q-51-1): the helpers every story suite
//! shares, defined once.
//!
//! A story is a test that drives a real server: SSH sessions, child
//! processes, a world on its own clock. Before doc 51 each suite kept such
//! helpers in its own `tests/support`, and most stories kept a private copy
//! besides. A fix to one copy never reached the others, which is how one 12-s
//! wait was mended in `sim_range` and missed in `observer_rule`. This crate is
//! the one copy. A suite's `tests/support` keeps what is its own (how it boots
//! its server) and re-exports the kit.
//!
//! What is here after unit FL.1:
//!
//! - [`wait_until`]: the one wait loop. It polls a condition under one limit.
//! - [`STEP_TIMEOUT`]: the one limit a step gets, which already carries
//!   [`LOAD_FACTOR`]. [`loaded`] applies the same factor to any other span.
//! - [`StoryBudget`]: one limit for a whole story, under the test runner's
//!   terminate, that every wait in the story draws from.
//!
//! A story reaches for these instead of a raw `sleep` or `timeout`. The
//! `wait_budget` gate in `mush-contract-tests` counts the raw ones in story
//! files and lets the count only fall.
//!
//! Added by unit FL.4, for a rig that boots an engine on a clock the test
//! sets:
//!
//! - [`STORY_EPOCH`]: the one instant a rig boots at, in place of the wall
//!   clock. [`Phase`] names the instants of the IC calendar a schedule-bound
//!   story also runs at, [`phases!`] makes one test a phase, and [`Calendar`]
//!   finds a phase on the rig's own IC clock. `STORY_PHASE` ([`PHASE_VAR`])
//!   lets the soak draw a phase and a session pin one.
//! - [`scratch_dir`]: a rig's data directory, named by no clock.
//!
//! The same gate counts a story file's reads of the wall clock.
//!
//! **Dev-only.** This crate is a `[dev-dependencies]` entry and nothing else:
//! its helpers panic by design, as a failed assertion does. The panic budget's
//! `no_production_crate_links_an_excluded_crate` test refuses a normal or a
//! build edge to it.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/ci_policy/restriction_lints.rs`
// carries the measurement. `clippy::panic` is the one not named: a wait that
// runs out panics, which is how a test fails.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod budget;
mod phase;
mod scratch;
mod wait;

pub use budget::{NEXTEST_TERMINATE, StoryBudget, TERMINATE_MARGIN};
pub use phase::{
    BEFORE_DAY_TURN, BEFORE_MONTH_TURN_DAYS, Calendar, IC_DAY, IC_HOUR, IC_MONTH, PHASE_TESTS,
    PHASE_VAR, Phase, STORY_EPOCH,
};
pub use scratch::scratch_dir;
pub use wait::{LOAD_FACTOR, POLL_INTERVAL, STEP_TIMEOUT, STEP_UNLOADED, loaded, wait_until};
