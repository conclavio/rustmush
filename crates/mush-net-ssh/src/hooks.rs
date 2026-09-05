//! [`SshHooks`] — the crate's only view of the game.
//!
//! `mush-net-ssh` knows how to be an SSH server; it does not know what an
//! account, a character, or a world is. The binary (`mushd`) implements
//! this trait: auth checks go to its account store, connected characters
//! are attached to its `SessionManager`, and every completed input line is
//! forwarded as-is. All async methods return [`BoxFuture`]s so the trait
//! stays dyn-compatible (`Arc<dyn SshHooks>`); implementors write
//! `Box::pin(async move { … })`.
//!
//! Conn ids are plain `u64`s allocated by this crate, unique per process —
//! the same values the binary wraps in `mush_session::ConnId`.

use std::net::IpAddr;
use std::sync::Arc;

use futures::future::BoxFuture;
use mush_proto::{ClientCaps, OutputEvent};
use tokio::sync::mpsc;

/// One character on an account, as shown on the character-select screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharSummary {
    /// Opaque character key (the binary widens its `ObjId` into this).
    pub key: u64,
    /// Display name.
    pub name: String,
}

/// Result of an account/password check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthResult {
    /// Credentials verified.
    Ok {
        /// Canonical account name (case-folded by the store).
        account: String,
        /// The account's characters, in display order.
        characters: Vec<CharSummary>,
    },
    /// Unknown account or wrong password (deliberately indistinguishable).
    BadCreds,
    /// The account is banned; the connection should be refused.
    Banned,
}

/// Why account creation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateErr {
    /// The name is already in use.
    Taken,
    /// The name violates naming rules; the payload explains how.
    BadName(String),
    /// The password fails the strength policy.
    Weak,
}

/// Everything the binary needs to register a freshly connected character's
/// transport: it wraps these fields in a `mush_session::ConnectionHandle`
/// and attaches it. The transport task keeps the receiving end of `tx` and
/// renders each event through `mush-ansi`.
#[derive(Debug)]
pub struct NewConn {
    /// Connection id (unique per process, allocated by this crate).
    pub id: u64,
    /// Peer address.
    pub addr: IpAddr,
    /// Capabilities detected at connect time (resize updates follow via
    /// [`SshHooks::caps_changed`]).
    pub caps: ClientCaps,
    /// Bounded output queue; the SSH output pump consumes the other end.
    pub tx: mpsc::Sender<Arc<OutputEvent>>,
}

/// The game-side callbacks the SSH ingress needs. Implemented by `mushd`;
/// this crate only ever holds it as `Arc<dyn SshHooks>`.
pub trait SshHooks: Send + Sync {
    /// Verify account credentials and list the account's characters.
    fn auth_account<'a>(&'a self, account: &'a str, password: &'a str)
    -> BoxFuture<'a, AuthResult>;

    /// Authenticate an account by an offered SSH public key (M19). The
    /// transport passes the offered key already rendered as an OpenSSH
    /// `"type base64 comment"` line (russh types are quarantined in this
    /// crate — the hook never sees a russh `PublicKey`). The binary matches
    /// it against the account's registered keys by key material (never by
    /// raw string), returning [`AuthResult::Ok`] on a match and
    /// [`AuthResult::BadCreds`] on an unknown account *or* no matching key
    /// (uniform — no account-existence oracle). The default rejects with
    /// `BadCreds`, so a binary that has not wired public-key auth safely
    /// falls back to password.
    fn auth_account_key<'a>(
        &'a self,
        _account: &'a str,
        _openssh_key: &'a str,
    ) -> BoxFuture<'a, AuthResult> {
        Box::pin(async { AuthResult::BadCreds })
    }

    /// Create a new account (this also creates the account's first,
    /// same-named character).
    fn create_account<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<(), CreateErr>>;

    /// Create an additional character on an already-authenticated account —
    /// the character-select screen's `newchar <Name>` (M2). `Err` carries a
    /// player-readable refusal (name rules, per-account cap, ...). The
    /// default refuses, so transports can offer the verb only against
    /// binaries that support it.
    fn create_character<'a>(
        &'a self,
        _account: &'a str,
        _name: &'a str,
    ) -> BoxFuture<'a, Result<CharSummary, String>> {
        Box::pin(async { Err("Character creation is not available.".to_string()) })
    }

    /// Attach `conn` as playing `character` under `account`. On `Ok` the
    /// world owns the character's output until the connection ends; an
    /// `Err` carries a player-readable refusal.
    fn connect_character<'a>(
        &'a self,
        conn: NewConn,
        account: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// One completed input line from a connected (in-game) connection.
    fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()>;

    /// A read-only world query on behalf of a connected connection — the SSH
    /// side of the structured `Inbound::Query` path (milestone M15). The
    /// transport uses it to refresh TUI widgets (e.g. the WHO rail submits
    /// `"who"`) without typing a command, so it must not stamp idle. The
    /// binary forwards `name` and `args` to the engine's query handler of
    /// that name and routes the reply back through the same output queue as
    /// an [`OutputEvent`] (interface plan I1, D-I-3: any registered feed or
    /// query name, with arguments). The default is a no-op, so a binary
    /// without query support simply serves no widget data.
    fn query(&self, _conn: u64, _name: String, _args: serde_json::Value) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// The transport for `conn` is gone (only fired for connections that
    /// passed [`connect_character`](Self::connect_character); the binary
    /// detaches it — `DetachReason::Lost` unless it already detached).
    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()>;

    /// Whether this address is currently banned/throttled (IpGuard lives
    /// binary-side). Checked before every auth attempt — the SSH-level
    /// methods *and* every in-band welcome-screen `connect`/`create` — so a
    /// tempban armed mid-connection also stops an already-open welcome
    /// screen from guessing further.
    fn is_ip_blocked(&self, addr: IpAddr) -> bool;

    /// Count one failed credential check against the address.
    fn auth_failed(&self, addr: IpAddr);

    /// Claim a per-IP connection slot for a **freshly accepted** transport
    /// (before any auth — welcome-screen squatters count too). `false`
    /// means the address is at its concurrency cap and the connection
    /// should be refused. Every `true` is balanced by exactly one
    /// [`connection_closed`](Self::connection_closed) when the connection
    /// handler drops. Default: unlimited.
    fn connection_opened(&self, _addr: IpAddr) -> bool {
        true
    }

    /// Release the slot claimed by
    /// [`connection_opened`](Self::connection_opened). Default: no-op.
    fn connection_closed(&self, _addr: IpAddr) {}

    /// Terminal resize / capability renegotiation for a connected
    /// connection (the binary typically forwards to
    /// `SessionManager::update_caps`). Default: ignored.
    fn caps_changed(&self, _conn: u64, _caps: ClientCaps) {}
}
