//! # mush-net-web — axum web server: token auth, WS envelope, embedded SPA
//!
//! The web ingress (design 02 §6): HTTP auth routes minting opaque session
//! tokens, the `/ws` WebSocket carrying JSON [`Envelope`](mush_proto::Envelope)
//! frames (the [`mush_proto::OutputEvent`] enum *is* the wire format —
//! treaty ruling 1), and the embedded Svelte SPA. Like `mush-net-ssh`, this
//! crate knows nothing about the game: everything crosses the [`WebHooks`]
//! trait, implemented by `mushd`.
//!
//! ## Surface
//!
//! | Route | What |
//! |---|---|
//! | `POST /api/auth/login` | `{account, password}` → `{token, account, characters[]}`; uniform 401 |
//! | `POST /api/auth/register` | `{account, password, char_name}` → same as login |
//! | `POST /api/auth/logout` | `Authorization: Bearer` → 204 |
//! | `GET /ws` | token via `Sec-WebSocket-Protocol: rustmush.v1.<token>` only, `?char=<key>`, optional `?resume=<last_seq>` |
//! | `GET /healthz` | `{ok, uptime_s, version, connections, loop_lag_ms, world_loop_unixtime, store_ok}` (lock-free read of the shared [`metrics::Metrics`]) |
//! | `GET /metrics` | Prometheus text exposition; gated on `WebConfig::metrics_enabled` (opt-in, default off); non-sensitive aggregates only |
//! | `/`, `/assets/*`, SPA deep links | embedded `web/dist` (rust-embed; disk in debug builds) |
//!
//! ## Security posture
//!
//! - **Tokens and passwords are never logged.** The trace span carries
//!   method + path only (never the query string — `/ws?token=` would leak),
//!   and no handler puts credential material into an event.
//! - **Same-origin only.** The SPA is served by this same server, so no
//!   CORS headers are emitted anywhere; browsers therefore refuse
//!   cross-origin `fetch`/XHR to `/api/*` by default. (WebSocket upgrades
//!   are not CORS-guarded by browsers — the bearer token requirement is
//!   what actually gates `/ws`.)
//! - **IpGuard first**: [`WebHooks::is_ip_blocked`] is checked before the
//!   auth routes and before every WS upgrade; failed logins **and** invalid
//!   WS tokens count via [`WebHooks::auth_failed`]. Every WS upgrade also
//!   claims a per-IP concurrent-connection slot via
//!   [`WebHooks::connection_opened`] before the handshake completes.
//! - **Rate limits**: tower_governor per-IP GCRA bucket on `/api/auth/*`
//!   (burst 5, 1 per 30 s by default). `X-Forwarded-For` is honored only
//!   when `trusted_proxies` is configured, and only from a peer that is
//!   itself one of `trusted_proxies` (same trust gate `IpGuard` uses).
//! - **WS hygiene**: 64 KiB inbound message cap (oversize, binary frames,
//!   malformed JSON, and mid-connection `resume` are protocol-error
//!   closes), app-level ping every 25 s with a two-missed-pongs cutoff,
//!   bounded outbound queue with a slow-client close.
//! - **Response headers**: every response carries `X-Content-Type-Options:
//!   nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, and
//!   a same-origin `Content-Security-Policy`.
//!
//! ## TLS
//!
//! Recommended prod: reverse proxy in front (Caddy/nginx), plain HTTP here,
//! `trusted_proxies` set. Native TLS exists behind the optional `tls`
//! cargo feature (`axum-server` + rustls/ring) for proxyless setups; SSH is
//! never proxied either way.

mod app;
mod assets;
mod auth;
mod config;
mod hooks;
mod ip;
pub mod metrics;
mod ws;

pub use app::{router, serve, serve_with_shutdown};
pub use config::{TlsConfig, WebConfig};
pub use hooks::{
    CharSummary, LoginErr, LoginOk, NewWebConn, RegisterErr, ResumeErr, ResumeOk, SessionStart,
    TokenIdentity, WEB_CONN_ID_BASE, WebHooks,
};
pub use metrics::{LlmFeatureStat, LlmTodaySpend, Metrics, MetricsSnapshot, WorldSample};
