//! the workspace's contract gates (doc 43 cut TS.11, test-strategy T.3c): group `ci_policy` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The tree's and the workflows' own gates: the gated-test inventory and its
//! tier manifest (test-tiers.toml), the tacsim shard guard, the clippy test
//! allowances, the feature census, the panic budget, the fiat budget (doc
//! 43 cut TS.6) and the restriction
//! lints (three of the four guards run before every push), the toolchain
//! pin, the RNG and serialization policies, the legacy-handler and
//! matcher-call-site registries, the schedule source, the fault gates and
//! the H3 scale harness (its tests #[ignore]d: the nightly `scale` job runs
//! the module by name, and its Redis twins run alone, .config/nextest.toml).
//!
//! One module: `cargo nextest run -p mush-contract-tests --test ci_policy -E 'test(/^ci_gated_test_inventory::/)'`
//! or `cargo test -p mush-contract-tests --test ci_policy ci_gated_test_inventory::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test ci_policy ci_gated_test_inventory::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "ci_policy/ci_gated_test_inventory.rs"]
mod ci_gated_test_inventory;
#[path = "ci_policy/ci_tacsim_shards.rs"]
mod ci_tacsim_shards;
#[path = "ci_policy/clippy_test_allowances.rs"]
mod clippy_test_allowances;
#[path = "ci_policy/feature_matrix_inventory.rs"]
mod feature_matrix_inventory;
#[path = "ci_policy/fiat_budget.rs"]
mod fiat_budget;
#[path = "ci_policy/gate5_fault_matrix.rs"]
mod gate5_fault_matrix;
#[path = "ci_policy/legacy_handler_allowlist.rs"]
mod legacy_handler_allowlist;
#[path = "ci_policy/matcher_call_sites.rs"]
mod matcher_call_sites;
#[path = "ci_policy/panic_budget.rs"]
mod panic_budget;
#[path = "ci_policy/process_fault_gate.rs"]
mod process_fault_gate;
#[path = "ci_policy/provider_rng_policy.rs"]
mod provider_rng_policy;
#[path = "ci_policy/restriction_lints.rs"]
mod restriction_lints;
#[path = "ci_policy/scale_harness.rs"]
mod scale_harness;
#[path = "ci_policy/schedule_source.rs"]
mod schedule_source;
#[path = "ci_policy/toolchain_pin.rs"]
mod toolchain_pin;
#[path = "ci_policy/truth_serialization_policy.rs"]
mod truth_serialization_policy;
