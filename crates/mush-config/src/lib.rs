//! # mush-config — shared runtime support for the rustmush binaries
//!
//! The configuration model plus the small pieces of runtime support that both
//! forked halves (`mush-gateway`, `mush-server`) and the all-in-one `mushd`
//! need, kept in one dependency-light crate so neither half drags the other's
//! deps in through them:
//!
//! - [`config`] — `mushd.toml` parsing/validation (the `[llm]`/`[web]` sections
//!   are captured raw as [`toml::Value`] and deserialized into their typed
//!   forms by the crate that owns them, so `mush-config` needs neither
//!   `mush-llm` nor `mush-net-web`).
//! - [`logging`] — the tracing subscriber + rotating file appender setup.
//! - [`ipguard`] — the per-IP auth-failure guard + siteban mirror.
//! - [`accounts`] — transport-agnostic account helpers (validation, argon2id
//!   hashing/verification, the one-time `accounts.json` import).
//! - [`offer_keys`] — durable, bounded, restart-stable HMAC key provisioning
//!   for K5 legal-action offers.
//! - [`resolution_keys`] — an independent durable key family for K4
//!   authoritative resolution RNG.
//! - [`decision_keys`] — durable HMAC keys for K7 delayed-decision prompts.

pub mod accounts;
pub mod config;
pub mod decision_keys;
pub mod discord;
mod durable_keys;
pub mod ipguard;
pub mod logging;
pub mod metrics;
pub mod offer_keys;
pub mod resolution_keys;

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
