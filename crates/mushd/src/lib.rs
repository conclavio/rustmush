//! # mushd — the rustmush server
//!
//! The M1 binary: a tokio process wrapping the synchronous
//! [`mush_core::Engine`] in the **single-writer world task** (design
//! 01 §4.1), with the session layer ([`mush_session::SessionManager`]) doing
//! character → connection fan-out (treaty ruling 2) and an SSH bridge
//! implementing the `mush-net-ssh` hooks.
//!
//! Layering (nothing here violates the treaty):
//!
//! - [`world_task`] — owns the `Engine`; the only place `World` is touched.
//!   Everything else talks to it over an mpsc of [`world_task::WorldMsg`].
//!   Async completions re-enter as typed
//!   [`WorldCommand`](mush_core::WorldCommand)s (treaty ruling 3).
//! - [`accounts`] — M1-interim account store (argon2id + JSON file),
//!   explicitly replaced by Redis in M2. Creating an account creates the
//!   Player *via a world-task request*, never by touching `World` directly.
//! - [`ssh_bridge`] — implements `mush_net_ssh::SshHooks` against the
//!   account store, the session manager, and the world inbound sender, and
//!   starts the real SSH listener.
//! - [`ipguard`] — per-IP auth-failure tempbans + concurrent-connection cap.
//! - [`config`] — `mushd.toml` (subset of keys used in M1).
//! - [`repl`] — the M0 stdin REPL, preserved behind `--repl`, now driving
//!   the `Engine` synchronously.

pub mod accounts;
pub mod config;
pub mod ipguard;
pub mod repl;
pub mod ssh_bridge;
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
