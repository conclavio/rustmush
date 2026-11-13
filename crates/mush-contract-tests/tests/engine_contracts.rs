//! the workspace's contract gates (doc 43 cut TS.11, test-strategy T.3c): group `engine_contracts` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The compiled production registry's contracts (exit words, help, macro
//! names, the run allowlist, through `production_registry`, the one module
//! three of them share), the state-slot owners (the fourth guard), the
//! typed state lens and module namespaces, the republish guards, the IPC
//! golden frames, the arena library, the pixel picture's fixtures, the NPC
//! ability residual, the vitals projection and the unit looks.
//!
//! One module: `cargo nextest run -p mush-contract-tests --test engine_contracts -E 'test(/^action_state_republish::/)'`
//! or `cargo test -p mush-contract-tests --test engine_contracts action_state_republish::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test engine_contracts action_state_republish::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

mod production_registry;

#[path = "engine_contracts/action_state_republish.rs"]
mod action_state_republish;
#[path = "engine_contracts/arena_library.rs"]
mod arena_library;
#[path = "engine_contracts/client_verbs.rs"]
mod client_verbs;
#[path = "engine_contracts/exit_command_words.rs"]
mod exit_command_words;
#[path = "engine_contracts/help_commands.rs"]
mod help_commands;
#[path = "engine_contracts/hex_vocabulary.rs"]
mod hex_vocabulary;
#[path = "engine_contracts/ipc_golden_frames.rs"]
mod ipc_golden_frames;
#[path = "engine_contracts/j4_narrow_fetch.rs"]
mod j4_narrow_fetch;
#[path = "engine_contracts/macro_names.rs"]
mod macro_names;
#[path = "engine_contracts/module_ns_owners.rs"]
mod module_ns_owners;
#[path = "engine_contracts/npc_ability_residual.rs"]
mod npc_ability_residual;
#[path = "engine_contracts/pixel_hull_down.rs"]
mod pixel_hull_down;
#[path = "engine_contracts/pixel_relief.rs"]
mod pixel_relief;
#[path = "engine_contracts/pixel_sight_heights.rs"]
mod pixel_sight_heights;
#[path = "engine_contracts/player_wait_floors.rs"]
mod player_wait_floors;
#[path = "engine_contracts/sheet_write_republish.rs"]
mod sheet_write_republish;
#[path = "engine_contracts/space_mind_memory.rs"]
mod space_mind_memory;
// Doc 44 unit MW.11: no space think runs on the world thread or makes it wait.
#[path = "engine_contracts/space_thinks_off_thread.rs"]
mod space_thinks_off_thread;
#[path = "engine_contracts/state_slot_owners.rs"]
mod state_slot_owners;
#[path = "engine_contracts/typed_state_lens.rs"]
mod typed_state_lens;
#[path = "engine_contracts/unit_looks.rs"]
mod unit_looks;
#[path = "engine_contracts/vitals_projection_clean.rs"]
mod vitals_projection_clean;
