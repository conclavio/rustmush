//! tacsim's integration tests: group `shard_2` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The grouping is line 2 of .github/tacsim-integration-shards.txt, the CI shard
//! that runs it (with `cost_lane`, which stays a target of its own: its timing
//! sections hold an exclusive lock and its C5 test pins scripts/cost-lane.sh's
//! run lines), balanced by test time rather than by theme (six
//! groups since 2026-10-27, on the JUnit per-module times); a rebalance moves
//! a `mod` line between the six shard roots.
//!
//! One module: `cargo nextest run -p tacsim --test shard_2 -E 'test(/^content::/)'`
//! or `cargo test -p tacsim --test shard_2 content::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test shard_2 content::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "shard_2/content.rs"]
mod content;
#[path = "shard_2/e7r1.rs"]
mod e7r1;
#[path = "shard_2/e7r2.rs"]
mod e7r2;
#[path = "shard_2/extensions.rs"]
mod extensions;
#[path = "shard_2/ic_sortie_voyages.rs"]
mod ic_sortie_voyages;
#[path = "shard_2/minds.rs"]
mod minds;
#[path = "shard_2/minds_pool.rs"]
mod minds_pool;
#[path = "shard_2/minds_rhai.rs"]
mod minds_rhai;
#[path = "shard_2/moon_ports.rs"]
mod moon_ports;
#[path = "shard_2/plumbing.rs"]
mod plumbing;
#[path = "shard_2/quarry.rs"]
mod quarry;
#[path = "shard_2/reflex.rs"]
mod reflex;
#[path = "shard_2/sight.rs"]
mod sight;
#[path = "shard_2/twin.rs"]
mod twin;
#[path = "shard_2/voyage.rs"]
mod voyage;
#[path = "shard_2/weapons.rs"]
mod weapons;
