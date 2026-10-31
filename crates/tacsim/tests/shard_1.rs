//! tacsim's integration tests: group `shard_1` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 1 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it (with `harness`, which stays a target of its own: its
//! `#![recursion_limit]` is crate-root-only), balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_1 -E 'test(/^driving::/)'`
//! or `cargo test -p tacsim --test shard_1 driving::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_1 driving::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_1/coop.rs"]
mod coop;
#[path = "shard_1/driving.rs"]
mod driving;
#[path = "shard_1/field_carrier.rs"]
mod field_carrier;
#[path = "shard_1/field_host.rs"]
mod field_host;
#[path = "shard_1/hulls.rs"]
mod hulls;
#[path = "shard_1/knowing.rs"]
mod knowing;
#[path = "shard_1/landing.rs"]
mod landing;
#[path = "shard_1/one_picture.rs"]
mod one_picture;
#[path = "shard_1/passage.rs"]
mod passage;
#[path = "shard_1/perception.rs"]
mod perception;
#[path = "shard_1/price.rs"]
mod price;
#[path = "shard_1/replay.rs"]
mod replay;
#[path = "shard_1/s9_corridor.rs"]
mod s9_corridor;
#[path = "shard_1/sale.rs"]
mod sale;
#[path = "shard_1/sheet.rs"]
mod sheet;
#[path = "shard_1/shipped_scale.rs"]
mod shipped_scale;
#[path = "shard_1/site_record.rs"]
mod site_record;
#[path = "shard_1/stations.rs"]
mod stations;
