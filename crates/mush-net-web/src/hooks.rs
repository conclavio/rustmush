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
use mush_proto::{Envelope, ProjectedCommitToken, Query, RespondChoice};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::metrics::Metrics;

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

/// What a token lookup concluded — three outcomes, not two.
///
/// Folding "I checked, and this credential is dead" together with "I could
/// not check" is a defect, not a simplification: the transport's only way to
/// say the first is close 4401, which the SPA answers by **deleting the
/// stored token**. Report the second as `Unavailable` and a store outage
/// costs a retry instead of signing every web player out of a token that was
/// never revoked.
///
/// `Unavailable` is still **fail closed**: the connection is refused. Only
/// the explanation differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenVerdict {
    /// The token is live; here is what it unlocks. The TTL has been slid.
    Valid(TokenIdentity),
    /// Checked and refused: unknown, expired, or revoked. The credential
    /// will never work again, so the client must drop it and sign in.
    Refused,
    /// Not checked — the token store could not answer (down, restarting,
    /// reconnecting). Says **nothing** about the token: the client must keep
    /// it and retry.
    Unavailable,
}

/// Why a fresh attach ([`WebHooks::connect_character`]) was refused.
///
/// Mirrors [`ResumeErr`]'s split, for the same reason: only `Refused` is
/// about the credential, and only `Refused` may reach the client as the
/// fatal close that costs it its token. An overloaded or restarting game is
/// a retry, exactly as it is on the resume path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachErr {
    /// The token/character pair does not name a character this connection
    /// may play, and retrying cannot change that. Carries the
    /// player-readable sentence the close frame quotes.
    Refused(String),
    /// The game cannot accept another connection right now (bounded
    /// lifecycle custody is full). Transient; the client keeps its token.
    Busy,
    /// The game boundary disappeared during setup — a server restart race,
    /// or a store the gateway could not reach. Transient.
    Unavailable,
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
    /// The session is valid and resumable, but the game cannot accept another
    /// lifecycle obligation right now. The transport must ask the client to
    /// retry later; it must not misreport this as an expired credential or
    /// fall back to a fresh attach.
    Busy,
    /// The session is valid, but the game boundary disappeared while the
    /// resume was being attached (normally a server restart race). The
    /// transport should ask the client to reconnect shortly, without treating
    /// the token as expired.
    Unavailable,
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

    /// Resolve a token to its account, refreshing the sliding TTL. The
    /// implementor must distinguish a refused credential from a store that
    /// could not answer — see [`TokenVerdict`]; the transport turns the
    /// first into a close the client answers by discarding its token.
    fn validate_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, TokenVerdict>;

    /// Attach `conn` as playing `character` under the account `token`
    /// resolves to. On `Ok` the world owns the character's output until the
    /// connection ends; an [`AttachErr`] says whether the refusal is about
    /// the credential (fatal) or about the moment (retryable).
    fn connect_character<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<SessionStart, AttachErr>>;

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

    /// Closed module request; results use this physical connection's event queue.
    fn invoke(
        &self,
        _conn: u64,
        _id: u64,
        _request: mush_proto::InvokeRequest,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// A panel-hydration request; the correlated
    /// [`Reply`](mush_proto::OutputEvent::Reply) arrives via the event
    /// stream (`conn`'s `tx`), not as a return value.
    fn request(&self, conn: u64, id: u64, q: Query) -> BoxFuture<'_, ()>;

    /// A structured client accepted and deduplicated one authoritative
    /// result. The game side may retire its bounded presentation copy.
    fn acknowledge(&self, conn: u64, commit: ProjectedCommitToken) -> BoxFuture<'_, ()>;

    /// A composing hint from a connected connection
    /// ([`ClientCommand::Composing`](mush_proto::ClientCommand::Composing),
    /// interface plan I4, D-I-11): `true` when a draft began, `false` when it
    /// was sent or abandoned. The binary rate-limits it and fans it out to
    /// the room; it is never an idle stamp. The default drops it.
    fn composing(&self, _conn: u64, _active: bool) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// A client's answer to a server-initiated request
    /// ([`ClientCommand::Respond`](mush_proto::ClientCommand::Respond),
    /// interface plan I10, D-I-21): the engine correlates it by `id`, applies
    /// the choice, and re-validates. Advisory and best-effort — a lost answer
    /// means the server applies the deadline default; never an idle stamp.
    /// The default drops it.
    fn respond(
        &self,
        _conn: u64,
        _id: u64,
        _epoch: String,
        _choice: RespondChoice,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// The transport for `conn` is gone (only fired for connections that
    /// passed [`connect_character`](Self::connect_character) or
    /// [`resume`](Self::resume); the binary detaches it).
    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()>;

    /// Whether this address is currently banned/throttled (IpGuard lives
    /// binary-side). Checked before the auth routes and before every WS
    /// upgrade.
    fn is_ip_blocked(&self, addr: IpAddr) -> bool;

    /// Count one failed **credential** check against the address — a wrong
    /// password on `/api/auth/login`, and nothing else. This is the counter
    /// that arms the tempban, so only a guess may feed it.
    ///
    /// A `/ws` upgrade whose token fails
    /// [`validate_token`](Self::validate_token) used to call this and no
    /// longer does; it calls [`token_refused`](Self::token_refused) instead.
    fn auth_failed(&self, addr: IpAddr);

    /// Count one **refused bearer token** on a `/ws` upgrade against the
    /// address, on a counter of its own that may throttle
    /// ([`is_token_throttled`](Self::is_token_throttled)) but must never
    /// tempban.
    ///
    /// A 256-bit session token is not guessable, so a refused one is a stale
    /// client — typically a browser tab that outlived the token record (a
    /// flushed store, a revoked account, an expired TTL) and is reconnecting
    /// on its own backoff. Counting that as a credential failure is what
    /// locked an operator out of SSH on 2026-09-20; see
    /// `mush_config::ipguard`. Every embedder must supply this accounting.
    ///
    /// Only [`TokenVerdict::Refused`] feeds this. A
    /// [`TokenVerdict::Unavailable`] counts nothing at all: the store being
    /// down is the server's problem, and charging every reconnecting tab for
    /// it would arm a throttle precisely when recovery needs those
    /// reconnects.
    fn token_refused(&self, addr: IpAddr);

    /// Whether the address is inside a refused-token cool-off. Checked on
    /// every `/ws` upgrade *before* the token is looked up, so a probing
    /// client costs the store nothing. Never a ban: it does not affect
    /// [`is_ip_blocked`](Self::is_ip_blocked), the auth routes, or SSH.
    fn is_token_throttled(&self, addr: IpAddr) -> bool;

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

    /// The shared in-process metrics registry the `/healthz` and `/metrics`
    /// endpoints read. The binary hands the same `Arc<Metrics>` it publishes
    /// into from the world task and the SSH transport, so both endpoints see
    /// the whole server's numbers, not just the web side. Default: a fresh
    /// empty registry (tests and embedders that do not wire metrics still get
    /// well-formed, all-zero output).
    fn metrics(&self) -> Arc<Metrics> {
        Arc::new(Metrics::new())
    }
}
