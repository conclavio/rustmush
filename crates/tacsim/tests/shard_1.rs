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
//! `#![recursion_limit]` is crate-root-only), balanced by test time rather than by theme (ten
//! groups since 2026-11-03, six from 2026-10-27, on the JUnit per-module
//! times); a rebalance moves a `mod` line between the shard roots, with the
//! file. A module may import only from its own binary: the field helpers it
//! needs are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test shard_1 -E 'test(/^bay_36a::/)'`
//! or `cargo test -p tacsim --test shard_1 bay_36a::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_1 bay_36a::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_1/bay_36a.rs"]
mod bay_36a;
#[path = "shard_1/bay_gc8.rs"]
mod bay_gc8;
#[path = "shard_1/crew_exit.rs"]
mod crew_exit;
#[path = "shard_1/crew_treatment.rs"]
mod crew_treatment;
#[path = "shard_1/driving.rs"]
mod driving;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "shard_1/field_crash.rs"]
mod field_crash;
#[path = "shard_1/field_gc10.rs"]
mod field_gc10;
#[path = "shard_1/field_gc2.rs"]
mod field_gc2;
#[path = "shard_1/field_gc3.rs"]
mod field_gc3;
#[path = "shard_1/field_gc4.rs"]
mod field_gc4;
#[path = "shard_1/field_gc5.rs"]
mod field_gc5;
#[path = "shard_1/field_gc6.rs"]
mod field_gc6;
#[path = "shard_1/field_host.rs"]
mod field_host;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "shard_1/field_raiders.rs"]
mod field_raiders;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "shard_1/field_scale.rs"]
mod field_scale;
#[path = "shard_1/one_picture.rs"]
mod one_picture;
#[path = "shard_1/passage.rs"]
mod passage;
#[path = "shard_1/perception.rs"]
mod perception;
#[path = "shard_1/replay.rs"]
mod replay;
#[path = "shard_1/review_36a.rs"]
mod review_36a;
#[path = "shard_1/s9_corridor.rs"]
mod s9_corridor;
#[path = "shard_1/sheet.rs"]
mod sheet;
#[path = "shard_1/shipped_scale.rs"]
mod shipped_scale;
#[path = "shard_1/site_record.rs"]
mod site_record;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
