//! # mushd — the all-in-one rustmush server
//!
//! Since the M16b fork, `mushd` no longer *contains* the gateway and server
//! code: it depends on both halves ([`mush_gateway`] + [`mush_server`]) plus the
//! shared [`mush_config`], and provides the **in-process glue** that runs them in
//! one process over a shared [`SessionManager`](mush_session::SessionManager) —
//! byte-for-byte the pre-fork behavior, one socket-swap away from the forked
//! deployment.
//!
//! The glue is:
//!
//! - [`inprocess::InProcessSink`] — the all-in-one implementor of
//!   [`mush_server::gateway_sink::GatewaySink`]: the world loop delivers through
//!   it straight onto the shared session manager (zero-copy `Arc` fan-out) and it
//!   self-polices slow clients back into the loop's own inbound queue, instead of
//!   serializing to a boundary socket.
//! - [`world_task::spawn`] / [`world_task::spawn_persist`] — the
//!   `SessionManager`-based convenience spawns (the ones the all-in-one binary
//!   and the integration suite use), which wire an [`InProcessSink`] over the
//!   shared manager onto `mush-server`'s
//!   [`spawn_persist_with_gateway`](mush_server::world_task::spawn_persist_with_gateway).
//!
//! Everything else is re-exported from the three crates so existing
//! `mushd::{config,accounts,ipguard,world_task,ssh_bridge,web_bridge,…}` paths
//! (used across the integration suite) keep resolving unchanged.
//!
//! [`InProcessSink`]: inprocess::InProcessSink

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod boot;
pub mod inprocess;
pub mod repl;

// --- shared runtime support (mush-config) ---
pub use mush_config::{config, ipguard, logging, unix_now_ms, unix_now_s};

// --- server half (mush-server) ---
pub use mush_server::{discord_dm, help, modules, persist, quarantine, staff};

// --- gateway half (mush-gateway) ---
pub use mush_gateway::{bridge_common, controls, discord_bridge, ssh_bridge, web_bridge};

/// The account layer: the gateway's [`Accounts`](mush_gateway::accounts::Accounts)
/// auth backend merged with the transport-agnostic helpers from
/// [`mush_config::accounts`], so `mushd::accounts::{Accounts, hash_password, …}`
/// all resolve as they did pre-fork.
pub mod accounts {
    pub use mush_config::accounts::{
        burn_dummy_verify, hash_password, import_accounts_json, run_blocking,
        validate_account_name, validate_character_name, validate_password, verify_hash,
    };
    pub use mush_gateway::accounts::{
        Accounts, AuthBackend, AuthOutcome, CharSummary, CreateErr, RESERVED_NAMES,
    };
}

/// The world task: `mush-server`'s implementation re-exported, plus the all-in-one
/// `SessionManager`-based spawns that wire an [`InProcessSink`](inprocess::InProcessSink)
/// over the shared manager (the forked server passes a socket sink instead).
pub mod world_task {
    use std::sync::Arc;

    use mush_core::Engine;
    use mush_session::SessionManager;
    use tokio::task::JoinHandle;

    pub use mush_server::world_task::*;

    // The `ObjId`↔`CharKey` widening helpers and the pre-attach conn sender moved
    // to the gateway's session-side delivery; re-exported here so existing
    // `mushd::world_task::{char_key, char_key_obj, send_event_to_conn}` paths in
    // the integration suite keep resolving.
    pub use mush_gateway::delivery::{char_key, char_key_obj, send_event_to_conn};

    use crate::inprocess::InProcessSink;

    /// Compatibility spawn (tests, tools): the full world task over throwaway
    /// in-memory persistence, delivering in-process over the shared
    /// `SessionManager`.
    #[must_use]
    pub fn spawn(
        engine: Engine,
        sessions: Arc<SessionManager>,
    ) -> (WorldHandle, JoinHandle<WorldOutcome>) {
        let handles = spawn_persist(engine, sessions, ServerDeps::in_memory());
        (handles.handle, handles.world_join)
    }

    /// Spawn the world task and its store-writer with all-in-one in-process
    /// delivery: the server→gateway seam is the shared `SessionManager`.
    #[must_use]
    pub fn spawn_persist(
        engine: Engine,
        sessions: Arc<SessionManager>,
        deps: ServerDeps,
    ) -> PersistHandles {
        spawn_persist_with_decision_workers(
            engine,
            sessions,
            deps,
            mush_server::decision_dispatch::DEFAULT_DECISION_PROVIDER_WORKERS,
            mush_server::world_task::DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
            // No Discord bridge in the convenience spawn: controls are dropped.
            Arc::new(mush_gateway::controls::GatewayControls::default()),
        )
        .unwrap_or_else(|error| panic!("world task spawn: {error}"))
    }

    /// H2a: [`spawn_persist`] with an explicit `[llm].decision_workers` and
    /// `[game].world_clock_checkpoint_s` (the `W-clock` checkpoint cadence). The
    /// all-in-one binary calls this; a composition with decision sources
    /// spawns through the production decision runtime over `deps.llm`.
    ///
    /// # Errors
    /// See [`spawn_persist_with_gateway_production`].
    pub fn spawn_persist_with_decision_workers(
        engine: Engine,
        sessions: Arc<SessionManager>,
        deps: ServerDeps,
        decision_workers: usize,
        world_clock_checkpoint_s: u64,
        controls: Arc<mush_gateway::controls::GatewayControls>,
    ) -> Result<PersistHandles, String> {
        let llm = deps.llm.clone();
        spawn_persist_with_gateway_production(
            engine,
            deps,
            llm,
            decision_workers,
            world_clock_checkpoint_s,
            move |weak_inbound| {
                Arc::new(InProcessSink::new(sessions, weak_inbound).with_controls(controls))
            },
        )
    }
}
