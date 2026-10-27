//! tacsim's integration tests: group `shard_6` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 6 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_6 -E 'test(/^plans::/)'`
//! or `cargo test -p tacsim --test shard_6 plans::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_6 plans::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;

#[path = "shard_6/cockpit.rs"]
mod cockpit;
#[path = "shard_6/plans.rs"]
mod plans;
