//! tacsim's integration tests: group `shard_10` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 10 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it, balanced by test time rather than by theme (ten
//! groups since 2026-11-03, six from 2026-10-27, on the JUnit per-module
//! times); a rebalance moves a `mod` line between the shard roots, with the
//! file. A module may import only from its own binary: the field helpers it
//! needs are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test shard_10 -E 'test(/^arena::/)'`
//! or `cargo test -p tacsim --test shard_10 arena::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_10 arena::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_10/arena.rs"]
mod arena;
#[path = "support/field/claim_rig.rs"]
mod claim_rig;
#[path = "shard_10/commerce.rs"]
mod commerce;
#[path = "shard_10/coop.rs"]
mod coop;
#[path = "support/field/custody_rig.rs"]
mod custody_rig;
#[path = "shard_10/deliberation.rs"]
mod deliberation;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "shard_10/field_carrier.rs"]
mod field_carrier;
#[path = "shard_10/field_fr4.rs"]
mod field_fr4;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "shard_10/field_rs1.rs"]
mod field_rs1;
#[path = "shard_10/field_rs2.rs"]
mod field_rs2;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "shard_10/ic_sortie_voyages.rs"]
mod ic_sortie_voyages;
#[path = "shard_10/landing.rs"]
mod landing;
#[path = "shard_10/motor.rs"]
mod motor;
#[path = "shard_10/plumbing.rs"]
mod plumbing;
#[path = "shard_10/ports.rs"]
mod ports;
#[path = "shard_10/price.rs"]
mod price;
#[path = "shard_10/reset.rs"]
mod reset;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
#[path = "shard_10/stations.rs"]
mod stations;
#[path = "shard_10/voyage.rs"]
mod voyage;
