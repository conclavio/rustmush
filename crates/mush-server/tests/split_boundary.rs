//! mush-server's integration tests: group `split_boundary` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The split deploy's boundary: the authoritative boot, action coordinator
//! and durable outbox, boot parity and the boot store, the command line,
//! gateway link parity, the IPC action and teardown paths (ipc_teardown
//! installs a global tracing subscriber: one process per test under
//! nextest), the shipped instances' provisioning smoke, the quarantine boot
//! and the relay registry.
//!
//! One module: `cargo nextest run -p mush-server --test split_boundary -E 'test(/^authoritative_action_coordinator::/)'`
//! or `cargo test -p mush-server --test split_boundary authoritative_action_coordinator::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test split_boundary authoritative_action_coordinator::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "split_boundary/authoritative_action_coordinator.rs"]
mod authoritative_action_coordinator;
#[path = "split_boundary/authoritative_boot.rs"]
mod authoritative_boot;
#[path = "split_boundary/authoritative_durable_outbox.rs"]
mod authoritative_durable_outbox;
#[path = "split_boundary/boot_parity.rs"]
mod boot_parity;
#[path = "split_boundary/boot_store.rs"]
mod boot_store;
#[path = "split_boundary/cli_args.rs"]
mod cli_args;
#[path = "split_boundary/dt_locomotion.rs"]
mod dt_locomotion;
#[path = "split_boundary/dt_movement_completion.rs"]
mod dt_movement_completion;
#[path = "split_boundary/dt_physical_transitions.rs"]
mod dt_physical_transitions;
#[path = "split_boundary/dt_preparation_plans.rs"]
mod dt_preparation_plans;
#[path = "split_boundary/dt_unit_authority.rs"]
mod dt_unit_authority;
#[path = "split_boundary/gateway_link_parity.rs"]
mod gateway_link_parity;
#[path = "split_boundary/ipc_authoritative_action.rs"]
mod ipc_authoritative_action;
#[path = "split_boundary/ipc_teardown.rs"]
mod ipc_teardown;
#[path = "split_boundary/provisioning_smoke.rs"]
mod provisioning_smoke;
#[path = "split_boundary/quarantine_boot.rs"]
mod quarantine_boot;
#[path = "split_boundary/relay_registry.rs"]
mod relay_registry;
