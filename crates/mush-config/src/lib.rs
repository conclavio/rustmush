//! # mush-config — shared runtime support for the rustmush binaries
//!
//! The configuration model plus the small pieces of runtime support that both
//! forked halves (`mush-gateway`, `mush-server`) and the all-in-one `mushd`
//! need, kept in one dependency-light crate so neither half drags the other's
//! deps in through them:
//!
//! - [`cli`] — the shared command line (`--config`/`--ipc`/`--repl`) all three
//!   binaries parse, and the one place an explicit config path is stat'ed.
//! - [`config`] — `mushd.toml` parsing/validation (the `[llm]`/`[web]` sections
//!   are captured raw as [`toml::Value`] and deserialized into their typed
//!   forms by the crate that owns them, so `mush-config` needs neither
//!   `mush-llm` nor `mush-net-web`).
//! - [`exit`] — the one process-exit vocabulary both halves speak, so a
//!   supervisor can tell a dead world from a slow one.
//! - [`logging`] — the tracing subscriber + rotating file appender setup.
//! - [`signals`] — the one terminate/interrupt future all three binaries wait
//!   on, including the rule that a supervised child ignores Ctrl-C.
//! - [`ipguard`] — the per-IP auth-failure guard + siteban mirror.
//! - [`accounts`] — transport-agnostic account helpers (validation, argon2id
//!   hashing/verification, the one-time `accounts.json` import).
//! - [`offer_keys`] — durable, bounded, restart-stable HMAC key provisioning
//!   for K5 legal-action offers.
//! - [`resolution_keys`] — an independent durable key family for K4
//!   authoritative resolution RNG.
//! - [`decision_keys`] — durable HMAC keys for K7 delayed-decision prompts.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod accounts;
pub mod cli;
pub mod config;
pub mod decision_keys;
pub mod discord;
mod durable_keys;
pub mod exit;
pub mod ipguard;
pub mod logging;
pub mod metrics;
pub mod offer_keys;
pub mod resolution_keys;
pub mod signals;

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
