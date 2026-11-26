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
//! What unit FL.1 put here:
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
//! Added by unit FL.3, for the stories that run a sim-center match:
//!
//! - [`Figures`]: a game figure (a cycle, a load, a flight, the arena's
//!   pulse) read from the content and the configuration the story's server
//!   boots with, and the limit of a wait derived from it. A story holds no
//!   literal that restates content (rule F).
//! - [`story_seed`] and [`Figures::seed_where`]: the dice of a pinned match
//!   seed, and a seed chosen so that a roll the story leans on is certain
//!   (rule D). [`Odds`] is for the story that must sample chance all the
//!   same: it states its odds, and the kit refuses odds worse than the bar.
//! - [`settled`], [`arena_wake`], [`arena_seconds`] and [`at_one_wake`]: the
//!   barriers. Each waits for the server to say it has got somewhere, over
//!   the story's own [`Session`], in place of a sleep (rule C).
//! - [`wait_for`]: [`wait_until`] for a wait that reads something, whose
//!   failure prints what it last read.
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
//! Added by unit FL.5, for the bot's stories (and any reader after them):
//!
//! - [`BUSY_SENTENCES`]: the one table of the sentences a server refuses a
//!   typed line with for a moment, each held to the product file that writes
//!   it; [`is_busy`] reads a line against it.
//! - [`ask`] and [`Reply`]: a whole reply whose tries count their answers, so
//!   a busy refusal never leaves a sentinel's answer owed to the next try,
//!   and the readers a story parses it with (`row`, `section`,
//!   `number_after`).
//!
//! Added by unit FL.6, for an in-process rig on tokio's paused clock:
//!
//! - [`idle`]: the barrier for "every other task on the runtime has parked",
//!   in place of a sleep or a count of yields that let the world take what was
//!   sent to it. An absence is asserted after it.
//!
//! Added by unit FL.7, for the stories that talk to a server over SSH:
//!
//! - [`Term`]: the one terminal, a line-mode SSH session and its transcript,
//!   with the needle reads, [`Term::say`]'s busy retry, and the [`Session`]
//!   the barriers read through. [`strip_ansi`] is its one copy of the
//!   terminal-to-text rule, and [`AcceptAll`] its client handler. The
//!   `one_reader` gate refuses a second `struct Term`, `ask_whole` or
//!   `strip_ansi` in a crate that has moved onto these.
//! - [`Term::across_restart`]: a held session's side of a restart, bounded
//!   by the restarted half's own heal schedule (rule P).
//!
//! Added by unit FL.8, for mushd's stories:
//!
//! - [`Term::ask_settled`]: a needle read made whole, for a story that parses
//!   what a line printed. [`ask_whole`] now counts its answers as [`ask`]
//!   does.
//! - [`Term::expect_squeezed_after`], [`Term::resize`] and [`Term::sent_at`]:
//!   the rest of mushd's own terminal, for the `@tui` screen and the rig's
//!   flood-bucket room.
//! - [`after_ticks`]: the barrier for "a world tick has fired `n` more
//!   times", on the scheduler's own order.
//!
//! Added by unit FL.9, for the small crates' stories (mush-net-web,
//! mush-net-ssh, mush-sshdoorway):
//!
//! - [`within`] and [`within_for`]: one event (a frame, a close, a task's
//!   end) awaited under the one step limit, in place of a span chosen at the
//!   call. [`at_most`] is for a future whose end is not the point.
//! - [`round_trip`]: the barrier for an SSH server's session loop. A reply to
//!   a request sent after some packets proves the server has handled them,
//!   so an absence is asserted between the send and the reply.
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

mod barrier;
mod budget;
mod busy;
mod figures;
mod odds;
mod paused;
mod phase;
mod readout;
mod reply;
mod scratch;
mod step;
mod term;
mod wait;

pub use barrier::{
    SENTINEL, SENTINEL_REFUSAL, Session, arena_frames, arena_seconds, arena_wake, ask_whole,
    at_one_wake, lift_floor, set_floor, settled, status_clock_seconds, status_frames,
    status_matches, the_match,
};
pub use budget::{NEXTEST_TERMINATE, StoryBudget, TERMINATE_MARGIN};
pub use busy::{BUSY_NEEDLES, BUSY_SENTENCES, BusySentence, busy_needles, holds_busy, is_busy};
pub use figures::{Figures, Mount, story_seed};
pub use odds::{Odds, PULL_REQUEST_BAR};
pub use paused::{IDLE_STEP, idle};
pub use phase::{
    BEFORE_DAY_TURN, BEFORE_MONTH_TURN_DAYS, Calendar, IC_DAY, IC_HOUR, IC_MONTH, PHASE_TESTS,
    PHASE_VAR, Phase, STORY_EPOCH,
};
pub use readout::{FLOODED, TICK_MARK, after_ticks};
pub use reply::{Reply, ask};
pub use scratch::scratch_dir;
pub use step::{at_most, round_trip, within, within_for};
pub use term::{AcceptAll, BUSY_RETRIES, COLS, RECONNECTED, RESTARTING, ROWS, Term, strip_ansi};
pub use wait::{
    LOAD_FACTOR, POLL_INTERVAL, STEP_TIMEOUT, STEP_UNLOADED, loaded, wait_for, wait_until,
};
