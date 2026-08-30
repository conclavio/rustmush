//! # mushd — the rustmush server
//!
//! A tokio process wrapping the synchronous [`mush_core::Engine`] in the
//! **single-writer world task** (design 01 §4.1), with the session layer
//! ([`mush_session::SessionManager`]) doing character → connection fan-out
//! (treaty ruling 2), an SSH bridge implementing the `mush-net-ssh` hooks,
//! and — since M2 — **write-through persistence** on the
//! [`mush_store::ObjectStore`] (Redis authoritative; in-memory for tests):
//! every processed command's world mutations become one atomic store batch,
//! sent immediately, so a `kill -9` loses at most the in-flight command.
//!
//! Layering (nothing here violates the treaty):
//!
//! - [`world_task`] — owns the `Engine`; the only place `World` is touched.
//!   Everything else talks to it over an mpsc of [`world_task::WorldMsg`].
//!   Async completions re-enter as typed
//!   [`WorldCommand`](mush_core::WorldCommand)s (treaty ruling 3). M2: also
//!   the write-through seam — every step's dirty set becomes one store
//!   batch, and the pause-mutations (read-only) policy lives here.
//! - [`persist`] — the store-writer task (ordered, retrying, ack-gating),
//!   dirty-set → [`WriteBatch`](mush_store::WriteBatch) snapshotting, and
//!   boot-time world load/seed.
//! - [`staff`] — `@siteban`/`@dump`: registry-registered handlers that push
//!   typed ops to the world task (which owns the store handles).
//! - [`modules`] — M3: the game-module roster (comsys, mail, jobs), their
//!   boot-data load through the store read API, and per-namespace module
//!   migrations. Module writes ride each command's `WriteBatch` as
//!   `module_ops` (same transaction as the object writes).
//! - [`accounts`] — store-backed accounts (argon2id PHC hashes in
//!   `account:{name}` docs; verification caller-side), plus the one-time
//!   `accounts.json` import. Creating a character creates the Player *via a
//!   world-task request*, never by touching `World` directly, and its
//!   confirmation gates on the store ack.
//! - [`bridge_common`] — the transport-agnostic bridge core
//!   ([`bridge_common::GameBridge`]): account auth/creation, session
//!   attach/enter-game, line handling (sanitize/QUIT/flood), the
//!   disconnect/linkdead flow — shared verbatim by both transports.
//! - [`link`] — M16a: the gateway<->server boundary transport. The bridges
//!   speak the `mush-ipc` protocol through [`link::GatewayLink`] (gateway →
//!   server) and the world loop delivers through [`link::InProcessTransport`]
//!   (server → gateway), so the seam is one socket-swap from the M16b fork
//!   while the single binary behaves identically.
//! - [`ssh_bridge`] — implements `mush_net_ssh::SshHooks` over the shared
//!   core and starts the real SSH listener.
//! - [`web_bridge`] — M4: implements `mush_net_web::WebHooks` over the
//!   same core, plus web-only concerns: opaque session tokens (SHA-256
//!   hashed in the store, 7-day sliding TTL), structured-envelope
//!   connections, resume replay, and the `Req` → `Inbound::Query` panel
//!   hydration mapping.
//! - [`ipguard`] — per-IP auth-failure tempbans + concurrent-connection cap
//!   + the in-memory mirror of persisted sitebans.
//! - [`config`] — `mushd.toml` (subset of keys used through M2).
//! - [`repl`] — the M0 stdin REPL, preserved behind `--repl`, driving the
//!   `Engine` synchronously entirely in memory (no store, no persistence).

pub mod accounts;
pub mod bridge_common;
pub mod config;
pub mod help;
pub mod ipguard;
pub mod link;
pub mod logging;
pub mod modules;
pub mod persist;
pub mod repl;
pub mod ssh_bridge;
pub mod staff;
pub mod web_bridge;
pub mod world_task;

/// The current wall clock as unix seconds (0 if the system clock is broken).
#[must_use]
pub fn unix_now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The current wall clock as unix milliseconds.
#[must_use]
pub fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
