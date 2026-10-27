//! mushd's integration tests: group `durable_process` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The all-in-one process itself: its boot composition and command line,
//! durability under `kill -9` and the clean shutdown (durability_e2e's
//! CHILD_LOCK; the Redis-gated twins run alone, .config/nextest.toml), the
//! persistence round trips, the relay bridge, the load soak, the world fault
//! firewall, the SIGTERM drain (`sigterm_exit` is Unix-only: its former
//! `#![cfg(unix)]` is the `#[cfg(unix)]` on its `mod` line below) and the
//! panic-exit halves. `world_panic_exit` re-execs this binary for its child
//! half, by the full id `world_panic_exit::the_half_child`.
//!
//! One module: `cargo nextest run -p mushd --test durable_process -E 'test(/^boot_composition::/)'`
//! or `cargo test -p mushd --test durable_process boot_composition::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test durable_process boot_composition::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

mod support;

#[path = "durable_process/boot_composition.rs"]
mod boot_composition;
#[path = "durable_process/cli_args.rs"]
mod cli_args;
#[path = "durable_process/durability_e2e.rs"]
mod durability_e2e;
#[path = "durable_process/load_soak.rs"]
mod load_soak;
#[path = "durable_process/m10c_e2e.rs"]
mod m10c_e2e;
#[path = "durable_process/persistence.rs"]
mod persistence;
#[path = "durable_process/relay_bridge.rs"]
mod relay_bridge;
#[path = "durable_process/server.rs"]
mod server;
#[cfg(unix)]
#[path = "durable_process/sigterm_exit.rs"]
mod sigterm_exit;
#[path = "durable_process/world_fault.rs"]
mod world_fault;
#[path = "durable_process/world_panic_exit.rs"]
mod world_panic_exit;
