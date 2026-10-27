//! mush-server's integration tests: group `world_task` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The world task itself: the arena bridge, the dead-world reader, H2b's
//! scene turns, H2c's ability coordinator, the preflight retry bounds, the
//! production action migrations, the world's exit status and its panic
//! firewall.
//!
//! One module: `cargo nextest run -p mush-server --test world_task -E 'test(/^arena_bridge::/)'`
//! or `cargo test -p mush-server --test world_task arena_bridge::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test world_task arena_bridge::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

#[path = "world_task/arena_bridge.rs"]
mod arena_bridge;
#[path = "world_task/dead_world_reader.rs"]
mod dead_world_reader;
#[path = "world_task/h2b_scene_turns.rs"]
mod h2b_scene_turns;
#[path = "world_task/h2c_ability_coordinator.rs"]
mod h2c_ability_coordinator;
#[path = "world_task/preflight_retry_bounds.rs"]
mod preflight_retry_bounds;
#[path = "world_task/production_action_migrations.rs"]
mod production_action_migrations;
#[path = "world_task/world_exit_status.rs"]
mod world_exit_status;
#[path = "world_task/world_panic_firewall.rs"]
mod world_panic_firewall;
