//! **The unit-mind sandbox** (17b cut M2.1; doc 17 §7 "The sandbox and the
//! budget", §8 "Memory, faults and the drop to doctrine", §13 "Records, the
//! seal and replay", §19 "Fit with the crates"; D-UM-1, D-UM-24, D-UM-25 as
//! amended, D-UM-26, D-UM-27, D-UM-28, D-UM-30 as amended, D-UM-31).
//!
//! A unit mind is a pure function `mind(view, memory, card, keyed draws) ->
//! (≤16 intents, memory')` (D-UM-1). This crate is the interpreter that
//! answers one, and nothing of the game it answers for: it depends on `rhai`
//! alone, names no view, intent, card or map, and is driven by a runner that
//! lives in the game (`crates/tacsim/src/minds`, 17b cut M2.3/M2.4). It is
//! deliberately not `mush-script`: that engine carries `timestamp()` and
//! samples a wall clock in its `on_progress` (`mush-script/src/runtime.rs`),
//! and a mind must never read a clock (doc 17 §7).
//!
//! * **The engine** ([`Sandbox::new`], `engine.rs`): one shared
//!   `Engine::new_raw()` with exactly the seven packages of D-UM-24, the
//!   six keyword-functions disabled, `sleep` shadowed, the limits of
//!   D-UM-25 as amended, `print`/`debug` to the trace and the budget in
//!   `on_progress`; then the game's own API through the caller's closure.
//! * **The budget** ([`budget`], D-UM-26): operations plus kernel fees
//!   against the think's limit, in thread-locals, with no clock anywhere.
//! * **The trace** ([`trace`], doc 17 §14): what a think `print`s, at most
//!   [`trace::TRACE_LINES`] lines of [`trace::TRACE_WIDTH`] characters,
//!   answered beside the result and never journalled.
//! * **The memory** ([`memory`], D-UM-30): the ONE canonical codec — sorted
//!   compact JSON over printable ASCII — the arena stores and hashes, and
//!   the shape check every think's memory passes at its end.
//! * **The cache** ([`Sandbox::compile`], D-UM-27, §13): one AST per
//!   `(source digest, MIND_ABI)`, its top-level statements cleared.
//! * **The run** ([`Sandbox::think`]): one think under `catch_unwind`, its
//!   memory bound as `this`, every way it can end mapped to an [`Outcome`] or
//!   a [`Fault`] with its cause word (D-UM-31).
//! * **The refusal** ([`system_error`]): the one constructor of an error a
//!   script's `try` cannot catch — latched for the think, so not even a
//!   package method's callback, which wraps every error, can swallow it —
//!   through which the game raises every refusal of its station functions
//!   and kernels (D-UM-31).
//!
//! # One think per thread
//!
//! The budget and the trace are thread-locals, set at a think's start and
//! read at its end. That is sound because a think runs on one thread from
//! start to end and a thread runs one think at a time: the sim task calls
//! its runner synchronously (`crates/tacsim/src/arena/mind.rs`,
//! `mind_think`), and 17b cut M4's pool runs one think per worker.

// 17b cut M2.1 (doc 17 §7 "Lints"): all six panic-shaped restriction lints,
// denied at the root — `mush-script`'s list read as a floor plus
// `expect_used`, because `crates/mush-contract-tests/tests/restriction_lints.rs`
// reads a zero in this crate's row as a promise to deny, and this crate has
// nothing to price.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod budget;
mod cache;
mod engine;
pub mod memory;
mod run;
pub mod trace;

pub use cache::{CompileError, Compiled, Digest};
pub use engine::{
    EngineLimits, MAX_ARRAY_SIZE, MAX_MAP_SIZE, MAX_MEMORY_ANY_CLASS, MAX_STRING_BYTES,
    MindRefusal, RHAI_ABI, Sandbox, system_error,
};
pub use run::{Fault, Outcome, Think};

/// **The rhai this crate is built on**, re-exported so the game registers its
/// API on [`Sandbox::new`]'s engine with the very types the sandbox runs
/// (`rhai::Engine`, `Dynamic`, `EvalAltResult`, …) and without a second
/// spelling of the pin (D-UM-24).
pub use rhai;
