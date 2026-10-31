//! mush-gateway's integration tests: group `split_halves` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The gateway half of the split deploy, driven against a real mush-server
//! it dials (the `harness` module's `ServerUnit` stands in for the server's
//! systemd unit, doc 33 GG.1): the arena custody and story lenses, the
//! authoritative process-kill proofs (PROCESS_LOCK: they run alone,
//! .config/nextest.toml), the command line, the front door, restart survival
//! over SSH and over the web, the server unit's gap and give-up, the TUI,
//! the two-process e2e and LLM journeys, and two gateways on one server
//! (doc 33 GG.2).
//! `authoritative_process_kill` and `server_panic_relaunch` re-exec this
//! binary for their workers, by the full `<module>::<fn>` ids. One allowed
//! exception to the suffix rule below: `restart_survival` is a suffix of
//! `web_restart_survival` (the manifest's allow_suffix), because only
//! nextest's anchored `test(/^restart_survival::/)` ever selects either;
//! under `cargo test` the filter `restart_survival::` selects both modules.
//!
//! One module: `cargo nextest run -p mush-gateway --test split_halves -E 'test(/^arena_custody_split::/)'`
//! or `cargo test -p mush-gateway --test split_halves arena_custody_split::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test split_halves arena_custody_split::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "split_halves/arena_custody_split.rs"]
mod arena_custody_split;
#[path = "split_halves/arena_story_split.rs"]
mod arena_story_split;
#[path = "split_halves/authoritative_process_kill.rs"]
mod authoritative_process_kill;
#[path = "split_halves/cli_args.rs"]
mod cli_args;
#[path = "split_halves/front_door_split.rs"]
mod front_door_split;
#[path = "split_halves/harness.rs"]
mod harness;
#[path = "split_halves/restart_survival.rs"]
mod restart_survival;
#[path = "split_halves/server_panic_relaunch.rs"]
mod server_panic_relaunch;
#[path = "split_halves/server_unit.rs"]
mod server_unit;
#[path = "split_halves/tui_split.rs"]
mod tui_split;
#[path = "split_halves/two_gateways.rs"]
mod two_gateways;
#[path = "split_halves/two_process_e2e.rs"]
mod two_process_e2e;
#[path = "split_halves/two_process_llm.rs"]
mod two_process_llm;
#[path = "split_halves/web_restart_survival.rs"]
mod web_restart_survival;
#[path = "split_halves/web_split_e2e.rs"]
mod web_split_e2e;
