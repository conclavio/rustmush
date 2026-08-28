//! [`WebHooks`] — the crate's only view of the game.
//!
//! `mush-net-web` knows how to be an HTTP/WebSocket server; it does not know
//! what an account, a character, a token store, or a world is. The binary
//! (`mushd`) implements this trait: logins verify argon2 hashes on
//! `spawn_blocking`, tokens live hashed in `mush-store` with a sliding TTL,
//! connected characters are attached to its `SessionManager`, and every
//! inbound [`ClientCommand::Line`](mush_proto::ClientCommand::Line) is
//! forwarded as-is. All async methods return [`BoxFuture`]s so the trait
//! stays dyn-compatible (`Arc<dyn WebHooks>`); implementors write
//! `Box::pin(async move { … })` (same pattern as `mush_net_ssh::SshHooks`).
//!
//! Conn ids are plain `u64`s allocated by this crate, unique per process.
//! They start at [`WEB_CONN_ID_BASE`] so they can never collide with the ids
//! `mush-net-ssh` allocates (which count up from 1) inside the binary's
//! single `mush_session::ConnId` space.
//!
//! Raw tokens cross this boundary (`login` mints one, `validate_token` /
//! `resume` / `logout` present one) but are **never logged** on either side;
//! the store only ever sees their SHA-256.

use std::net::IpAddr;
use std::sync::Arc;

use futures::future::BoxFuture;
use mush_proto::{Envelope, Query};
use serde::Serialize;
use tokio::sync::mpsc;

/// First conn id this crate hands out. `mush-net-ssh` allocates from 1
/// upward; starting the web side at `2^32` keeps the two ranges disjoint in
/// the binary's shared `ConnId` space.
pub const WEB_CONN_ID_BASE: u64 = 1 << 32;

/// One character on an account, as listed by [`WebHooks::login`] and the
/// character-select UI. Serializes into the `/api/auth/login` response as
/// `{"key": 7, "name": "Nick"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CharSummary {
    /// Opaque character key (the binary widens its `ObjId` into this).
    pub key: u64,
    /// Display name.
    pub name: String,
}

/// A successful login or registration: the freshly minted session token and
/// what it unlocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginOk {
    /// Opaque 256-bit session token (the raw token — the store keeps only
    /// its hash). NEVER logged.
    pub token: String,
    /// Canonical account name (case-folded by the store).
    pub account: String,
    /// The account's characters, in display order.
    pub characters: Vec<CharSummary>,
}

/// Why a login was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginErr {
    /// Unknown account or wrong password (deliberately indistinguishable;
    /// the HTTP layer answers a uniform 401).
    BadCreds,
    /// The account (or address) is banned.
    Banned,
    /// The binary-side limiter refused the attempt (e.g. argon2 verify
    /// queue depth); maps to 429.
    RateLimited,
}

/// Why a registration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterErr {
    /// The account name is already in use.
    Taken,
    /// The name violates naming rules; the payload explains how.
    BadName(String),
    /// The password fails the strength policy.
    Weak,
    /// The address is banned from creating accounts.
    Banned,
}

/// What a valid token resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenIdentity {
    /// Canonical account name.
    pub account: String,
    /// The account's characters, in display order.
    pub characters: Vec<CharSummary>,
}

/// Everything the binary needs to register a freshly upgraded WebSocket's
/// transport: it wraps these fields in a structured-sink
/// `mush_session::ConnectionHandle` and attaches it. The transport task
/// keeps the receiving end of `tx` and forwards each envelope as one JSON
/// text frame.
#[derive(Clone, Debug)]
pub struct NewWebConn {
    /// Connection id (unique per process, allocated by this crate; starts
    /// at [`WEB_CONN_ID_BASE`]).
    pub id: u64,
    /// Client address (the real client, not the reverse proxy, when
    /// `trusted_proxies` is configured).
    pub addr: IpAddr,
    /// Bounded output queue of session-stamped envelopes; the WS output
    /// pump consumes the other end.
    pub tx: mpsc::Sender<Arc<Envelope>>,
}

/// Result of a successful fresh attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionStart {
    /// Highest envelope seq already delivered on this session (multi-connect:
    /// another web tab may have consumed output already); `0` if fresh.
    pub resume_from: u64,
}

/// Result of a successful resume: the missed envelopes and where the live
/// stream picks up.
#[derive(Clone, Debug)]
pub struct ResumeOk {
    /// The gap — every recorded envelope with `seq > last_seq`, in order.
    pub replayed: Vec<Arc<Envelope>>,
    /// First seq that will arrive via the live `tx` stream.
    pub live_from: u64,
}

/// Why a resume was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeErr {
    /// The gap exceeded the session's resume ring: the client must
    /// re-hydrate. The transport falls back to a fresh
    /// [`connect_character`](WebHooks::connect_character) and tells the
    /// client `resumed { replayed: false }`.
    TooOld,
    /// The token/character pair no longer names a resumable session.
    BadSession,
}

/// The game-side callbacks the web ingress needs. Implemented by `mushd`
/// (`src/web_bridge.rs`); this crate only ever holds it as
/// `Arc<dyn WebHooks>`.
pub trait WebHooks: Send + Sync {
    /// Verify account credentials, mint a session token (stored hashed,
    /// 7-day sliding TTL), and list the account's characters. Argon2
    /// verification runs on `spawn_blocking` binary-side.
    fn login<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
        ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, LoginErr>>;

    /// Create an account plus its first character, then log it in (same
    /// return as [`login`](Self::login)).
    fn register<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
        char_name: &'a str,
        ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, RegisterErr>>;

    /// Revoke a token (deletes the stored hash). Idempotent; unknown tokens
    /// are a no-op.
    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, ()>;

    /// Resolve a token to its account, refreshing the sliding TTL. `None`
    /// means unknown or expired.
    fn validate_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Option<TokenIdentity>>;

    /// Attach `conn` as playing `character` under the account `token`
    /// resolves to. On `Ok` the world owns the character's output until the
    /// connection ends; an `Err` carries a player-readable refusal.
    fn connect_character<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<SessionStart, String>>;

    /// Re-attach `conn` to an existing session and replay everything after
    /// `last_seq` from its resume ring.
    fn resume<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
        last_seq: u64,
    ) -> BoxFuture<'a, Result<ResumeOk, ResumeErr>>;

    /// One command line from a connected (in-game) connection.
    fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()>;

    /// A panel-hydration request; the correlated
    /// [`Reply`](mush_proto::OutputEvent::Reply) arrives via the event
    /// stream (`conn`'s `tx`), not as a return value.
    fn request(&self, conn: u64, id: u64, q: Query) -> BoxFuture<'_, ()>;

    /// The transport for `conn` is gone (only fired for connections that
    /// passed [`connect_character`](Self::connect_character) or
    /// [`resume`](Self::resume); the binary detaches it).
    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()>;

    /// Whether this address is currently banned/throttled (IpGuard lives
    /// binary-side). Checked before the auth routes and before every WS
    /// upgrade.
    fn is_ip_blocked(&self, addr: IpAddr) -> bool;

    /// Count one failed credential check against the address. Also called
    /// for a WS upgrade whose token fails [`validate_token`](Self::validate_token)
    /// — token probing over `/ws` counts against the address exactly like a
    /// failed `/api/auth/login` (HIGH finding: `/ws` previously had no
    /// failure accounting of its own).
    fn auth_failed(&self, addr: IpAddr);

    /// Claim a per-IP concurrent WS connection slot for a **freshly
    /// requested** upgrade, before it completes and before its token is
    /// even validated — probing counts too. `false` means the address is
    /// at its concurrency cap and the upgrade should be refused. Every
    /// `true` is balanced by exactly one
    /// [`connection_closed`](Self::connection_closed) when the socket task
    /// ends (mirrors `mush_net_ssh::SshHooks::connection_opened`, which the
    /// SSH transport already uses against the same per-IP counter).
    /// Default: unlimited (existing implementors, and `MockWebHooks` in
    /// tests, are unaffected).
    fn connection_opened(&self, _addr: IpAddr) -> bool {
        true
    }

    /// Release the slot claimed by
    /// [`connection_opened`](Self::connection_opened). Default: no-op.
    fn connection_closed(&self, _addr: IpAddr) {}
}
