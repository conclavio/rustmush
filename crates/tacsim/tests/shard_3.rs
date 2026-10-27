//! tacsim's integration tests: group `shard_3` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 3 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_3 -E 'test(/^capacity::/)'`
//! or `cargo test -p tacsim --test shard_3 capacity::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_3 capacity::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_3/capacity.rs"]
mod capacity;
#[path = "shard_3/composition_edges.rs"]
mod composition_edges;
#[path = "shard_3/definitions.rs"]
mod definitions;
#[path = "shard_3/deliberation.rs"]
mod deliberation;
#[path = "shard_3/deposits.rs"]
mod deposits;
#[path = "shard_3/drones.rs"]
mod drones;
#[path = "shard_3/e7c1.rs"]
mod e7c1;
#[path = "shard_3/ephemeris.rs"]
mod ephemeris;
#[path = "shard_3/mill.rs"]
mod mill;
#[path = "shard_3/motor.rs"]
mod motor;
#[path = "shard_3/orders.rs"]
mod orders;
#[path = "shard_3/port_tick.rs"]
mod port_tick;
#[path = "shard_3/ports.rs"]
mod ports;
#[path = "shard_3/report.rs"]
mod report;
#[path = "shard_3/reset.rs"]
mod reset;
