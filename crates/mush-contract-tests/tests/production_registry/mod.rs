//! The production command registry, composed through the ONE root both
//! binaries boot — `mush_server::boot::game_modules` — shared by the contract
//! gates that ask "what does this server actually register?".
//!
//! It is the root and not a transcription of it on purpose, for the reason
//! `module_ns_owners.rs` states at its own head: a gate asserting against a
//! hand-copied roster asserts against the copy. A module added to
//! `boot::game_modules` and not to a copy would leave
//! `exit_command_words.rs` — whose whole job is to make a new verb/exit
//! collision a decision a person makes — printing `1 passed` while the verb it
//! never registered is swallowed at the Bus Stop.
//!
//! This crate's `src/` is an ordinary dependency and cannot reach
//! `mush-server`, which is a dev-dependency, so the roster cannot live there
//! without making the production crate a production edge of the contract crate.
//! A `tests/` module is the one place it can be shared between test binaries.
//!
//! `legacy_handler_allowlist.rs` keeps its own copy: it also projects the
//! compatibility class each verb publishes, and its roster is the thing its
//! pinned baseline counts. 16b cut D.5 did not move it.

use std::sync::Arc;

use mush_core::{Engine, GameModule, ModuleComposition};
use mush_server::boot;
use mush_server::staff::{StaffQueue, register_server_commands};
use mush_server::world_task::base_registry;

/// The production module roster with default configuration, straight from the
/// composition root.
///
/// Default configuration means `[tacsim].enabled = false` and no LLM provider,
/// and neither changes which verbs register: a switched-off layer still answers
/// "not enabled on this server", which is exactly what keeps this surface
/// independent of an operator's switches.
fn production_modules() -> Vec<Box<dyn GameModule>> {
    let config = mush_config::config::Config::default();
    // No provider is configured and the service is disabled; which modules
    // compose does not depend on it.
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .global_enabled(false)
            .build(),
    );
    boot::game_modules(&config, &llm).expect("the default config composes a roster")
}

/// Compose and freeze every registry before any world record exists — the verbs
/// are fixed here, and no world and no boot are needed.
pub fn production_composition() -> ModuleComposition {
    let staff = Arc::new(StaffQueue::new());
    let mut registry = base_registry();
    register_server_commands(&mut registry, &staff)
        .expect("server command registration must not collide");
    Engine::compose_modules(registry, production_modules())
        .expect("the production composition registers without collision")
}
