//! tacsim's integration tests: group `shard_9` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 9 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (ten
//! groups since 2026-11-03, six from 2026-10-27, on the JUnit per-module
//! times); a rebalance moves a `mod` line between the shard roots, with the
//! file. A module may import only from its own binary: the field helpers it
//! needs are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test shard_9 -E 'test(/^excursions::/)'`
//! or `cargo test -p tacsim --test shard_9 excursions::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_9 excursions::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "support/field/claim_rig.rs"]
mod claim_rig;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "shard_9/excursions.rs"]
mod excursions;
#[path = "shard_9/field_director.rs"]
mod field_director;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "shard_9/field_story.rs"]
mod field_story;
#[path = "shard_9/ic_sortie_surface.rs"]
mod ic_sortie_surface;
#[path = "shard_9/m8_leaks.rs"]
mod m8_leaks;
#[path = "shard_9/m8_world.rs"]
mod m8_world;
#[path = "shard_9/mill.rs"]
mod mill;
#[path = "shard_9/quarry.rs"]
mod quarry;
#[path = "shard_9/sale.rs"]
mod sale;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
