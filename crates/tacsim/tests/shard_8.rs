//! tacsim's integration tests: group `shard_8` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 8 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (ten
//! groups since 2026-11-03, six from 2026-10-27, on the JUnit per-module
//! times); a rebalance moves a `mod` line between the shard roots, with the
//! file. A module may import only from its own binary: the field helpers it
//! needs are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test shard_8 -E 'test(/^escort_lease::/)'`
//! or `cargo test -p tacsim --test shard_8 escort_lease::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_8 escort_lease::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "support/field/claim_rig.rs"]
mod claim_rig;
#[path = "shard_8/crew_custody.rs"]
mod crew_custody;
#[path = "support/field/custody_rig.rs"]
mod custody_rig;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "shard_8/escort_lease.rs"]
mod escort_lease;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "shard_8/hulls.rs"]
mod hulls;
#[path = "shard_8/match_lifecycle.rs"]
mod match_lifecycle;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
#[path = "shard_8/unloading.rs"]
mod unloading;
