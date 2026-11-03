//! tacsim's integration tests: group `shard_4` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 4 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (ten
//! groups since 2026-11-03, six from 2026-10-27, on the JUnit per-module
//! times); a rebalance moves a `mod` line between the shard roots, with the
//! file. A module may import only from its own binary: the field helpers it
//! needs are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test shard_4 -E 'test(/^arena_task::/)'`
//! or `cargo test -p tacsim --test shard_4 arena_task::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_4 arena_task::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_4/arena_task.rs"]
mod arena_task;
#[path = "shard_4/chart.rs"]
mod chart;
#[path = "shard_4/conditions.rs"]
mod conditions;
#[path = "shard_4/core_replacement.rs"]
mod core_replacement;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "shard_4/economy.rs"]
mod economy;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "shard_4/geology.rs"]
mod geology;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "shard_4/knowing.rs"]
mod knowing;
#[path = "shard_4/lane.rs"]
mod lane;
#[path = "shard_4/parties.rs"]
mod parties;
#[path = "shard_4/resolution.rs"]
mod resolution;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
#[path = "shard_4/sky.rs"]
mod sky;
