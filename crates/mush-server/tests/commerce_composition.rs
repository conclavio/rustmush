//! mush-server's integration tests: group `commerce_composition` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The commerce and composition stories against the authoritative server:
//! the E0 ledger's durability, the E4c bridge, E6a's alerts and inbox, the
//! E6b seal, the E7r2 proving loop, the IC buyout-dormancy and slice loops,
//! the market issuer, the minds' commerce composition and the SM story.
//! `ic_buyout_dormancy` and `ic_slice_loop` share tacsim's
//! `tests/support/vertex.rs`: this root declares it once, by its cross-crate
//! `#[path]` (clippy's duplicate_mod refuses two includes of one file), and
//! the two modules `use crate::vertex;`.
//!
//! One module: `cargo nextest run -p mush-server --test commerce_composition -E 'test(/^e0_ledger_durability::/)'`
//! or `cargo test -p mush-server --test commerce_composition e0_ledger_durability::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test commerce_composition e0_ledger_durability::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "../../tacsim/tests/support/vertex.rs"]
mod vertex;

#[path = "commerce_composition/e0_ledger_durability.rs"]
mod e0_ledger_durability;
#[path = "commerce_composition/e4c_bridge.rs"]
mod e4c_bridge;
#[path = "commerce_composition/e6a_alerts.rs"]
mod e6a_alerts;
#[path = "commerce_composition/e6a_inbox.rs"]
mod e6a_inbox;
#[path = "commerce_composition/e6b_seal.rs"]
mod e6b_seal;
#[path = "commerce_composition/e7r2_proving_loop.rs"]
mod e7r2_proving_loop;
#[path = "commerce_composition/ic_buyout_dormancy.rs"]
mod ic_buyout_dormancy;
#[path = "commerce_composition/ic_slice_loop.rs"]
mod ic_slice_loop;
#[path = "commerce_composition/market_issuer.rs"]
mod market_issuer;
#[path = "commerce_composition/mind_commerce_composition.rs"]
mod mind_commerce_composition;
#[path = "commerce_composition/sm_story.rs"]
mod sm_story;
