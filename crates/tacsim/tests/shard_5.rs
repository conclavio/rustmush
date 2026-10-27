//! tacsim's integration tests: group `shard_5` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 5 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_5 -E 'test(/^claims::/)'`
//! or `cargo test -p tacsim --test shard_5 claims::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_5 claims::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;

#[path = "shard_5/claims.rs"]
mod claims;
#[path = "shard_5/mining.rs"]
mod mining;
#[path = "shard_5/sites.rs"]
mod sites;
