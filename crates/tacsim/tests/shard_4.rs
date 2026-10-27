//! tacsim's integration tests: group `shard_4` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 4 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_4 -E 'test(/^arena::/)'`
//! or `cargo test -p tacsim --test shard_4 arena::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_4 arena::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_4/arena.rs"]
mod arena;
#[path = "shard_4/arena_task.rs"]
mod arena_task;
#[path = "shard_4/chart.rs"]
mod chart;
#[path = "shard_4/commerce.rs"]
mod commerce;
#[path = "shard_4/conditions.rs"]
mod conditions;
#[path = "shard_4/core_replacement.rs"]
mod core_replacement;
#[path = "shard_4/economy.rs"]
mod economy;
#[path = "shard_4/excursions.rs"]
mod excursions;
#[path = "shard_4/geology.rs"]
mod geology;
#[path = "shard_4/lane.rs"]
mod lane;
#[path = "shard_4/match_lifecycle.rs"]
mod match_lifecycle;
#[path = "shard_4/resolution.rs"]
mod resolution;
#[path = "shard_4/sky.rs"]
mod sky;
#[path = "shard_4/vehicles.rs"]
mod vehicles;
