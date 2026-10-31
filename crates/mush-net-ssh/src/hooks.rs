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
use std::time::Duration;

use futures::future::BoxFuture;
use mush_proto::{ClientCaps, OutputEvent, RespondChoice};

use crate::tui::pixel::PixelConfig;
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

/// What the embedder's guard says about a freshly accepted TCP connection
/// ([`SshHooks::admit_connection`]), before any SSH byte is exchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Serve it. Holds a per-address slot until
    /// [`SshHooks::release_connection`] with `Admit`.
    Admit,
    /// Over the address's concurrency cap: SSH runs only far enough to tell
    /// the client why (the authentication banner), every auth method is
    /// refused, and the connection is cut within five seconds of accept.
    /// Released with `OverCap`.
    OverCap,
    /// Over the rate, the global cap or the IPv6 /48 cap: the TCP connection
    /// is closed without a handshake. Nothing is held, nothing is released.
    Drop,
}

/// Why an address is being refused, so the transport can say so in words
/// instead of leaving an OpenSSH client to guess at a password prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// A staff-set, persisted siteban. No countdown is promised: it may be
    /// permanent, and the expiry is not the player's business.
    Siteban,
    /// The auth-failure tempban, with what is left of it.
    TooManyFailures {
        /// Time remaining on the ban.
        remaining: Duration,
    },
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
    /// Optional SSH envelope stream; legacy adapters may keep using `tx`.
    pub stamped_tx: Option<mpsc::Sender<Arc<mush_proto::Envelope>>>,
}

/// A transport-local typed reply shares the gateway's physical-connection lock.
#[derive(Default)]
pub struct LocalReply {
    /// Delimiters retained by the gateway.
    pub brackets: mush_proto::OutputBrackets,
    _guard: Option<Box<dyn Send>>,
}
impl LocalReply {
    /// Retain a connection's reply guard until the transport finishes writing.
    pub fn with_guard(brackets: mush_proto::OutputBrackets, guard: impl Send + 'static) -> Self {
        Self {
            brackets,
            _guard: Some(Box::new(guard)),
        }
    }
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

    /// Serialize an SSH-local typed command with world and gateway replies.
    fn begin_local_reply(&self, _conn: u64) -> BoxFuture<'_, LocalReply> {
        Box::pin(async { LocalReply::default() })
    }

    /// A continuous cockpit control, not a typed reply. Its notices remain
    /// ordinary asynchronous game output and do not open nested delimiters.
    fn control_line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
        self.line(conn, line)
    }

    /// Closed module request; results use this physical connection's event queue.
    fn invoke(
        &self,
        _conn: u64,
        _id: u64,
        _request: mush_proto::InvokeRequest,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// A read-only world query on behalf of a connected connection — the SSH
    /// side of the structured `Inbound::Query` path (milestone M15). The
    /// transport uses it to refresh TUI widgets (e.g. the WHO rail submits
    /// `"who"`) without typing a command, so it must not stamp idle. The
    /// binary forwards `name` and `args` to the engine's query handler of
    /// that name and routes the reply back through the same output queue as
    /// an [`OutputEvent`] (interface plan I1, D-I-3: any registered feed or
    /// query name, with arguments). The default is a no-op, so a binary
    /// without query support simply serves no widget data.
    fn query(
        &self,
        _conn: u64,
        _id: u64,
        _name: String,
        _args: serde_json::Value,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// A client's answer to a server-initiated request (interface plan I10,
    /// D-I-21): the TUI's request overlay and a line-mode `respond` both route
    /// here. Advisory and best-effort — a lost answer means the server applies
    /// the deadline default; never an idle stamp. The default drops it.
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
    /// passed [`connect_character`](Self::connect_character); the binary
    /// detaches it — `DetachReason::Lost` unless it already detached).
    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()>;

    /// Whether this address is currently banned/throttled (IpGuard lives
    /// binary-side). Checked before every auth attempt — the SSH-level
    /// methods *and* every in-band welcome-screen `connect`/`create` — so a
    /// tempban armed mid-connection also stops an already-open welcome
    /// screen from guessing further.
    fn is_ip_blocked(&self, addr: IpAddr) -> bool;

    /// Why the address is blocked, when it is — the refusal
    /// [`is_ip_blocked`](Self::is_ip_blocked) reports, in enough detail to
    /// tell the client about it. The transport sends it as an SSH
    /// authentication banner (the one thing an OpenSSH client displays
    /// before it gives up) and reuses it on the welcome screen.
    ///
    /// Answering `None` for a blocked address is safe: the refusal is
    /// unaffected, the client is simply told nothing. Every embedder must
    /// explicitly choose how to explain the refusal.
    fn block_reason(&self, addr: IpAddr) -> Option<BlockReason>;

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

    /// Gate a **freshly accepted** TCP connection on the public port, before
    /// any SSH byte is exchanged (doc 29a §9): the per-address concurrency
    /// cap, the per-address rate, the global cap and the IPv6 /48 cap all
    /// answer here, where a refusal costs no key exchange. Every
    /// [`Admission::Admit`] and [`Admission::OverCap`] is balanced by exactly
    /// one [`release_connection`](Self::release_connection) with the same
    /// value when the handler drops; an [`Admission::Drop`] gets no handler
    /// and is never released.
    ///
    /// Default: [`connection_opened`](Self::connection_opened)'s answer,
    /// mapped (`true` is `Admit`, `false` is `OverCap`), so an embedder with
    /// only the per-address cap keeps exactly what it had.
    fn admit_connection(&self, addr: IpAddr) -> Admission {
        if self.connection_opened(addr) {
            Admission::Admit
        } else {
            Admission::OverCap
        }
    }

    /// Release what [`admit_connection`](Self::admit_connection) claimed.
    /// Default: [`connection_closed`](Self::connection_closed) for an
    /// `Admit`, nothing otherwise (the default `admit_connection` holds
    /// nothing for an `OverCap`).
    fn release_connection(&self, addr: IpAddr, admission: Admission) {
        if admission == Admission::Admit {
            self.connection_closed(addr);
        }
    }

    /// Terminal resize / capability renegotiation for a connected
    /// connection (the binary typically forwards to
    /// `SessionManager::update_caps`). Default: ignored.
    fn caps_changed(&self, _conn: u64, _caps: ClientCaps) {}

    /// The terminal picture's knobs, from `[pixel]` (doc 21 cut G.1b): read
    /// when a connection enters `@tui`. Default: the built-in defaults.
    fn pixel_config(&self) -> PixelConfig {
        PixelConfig::default()
    }

    /// One thing the terminal picture did, for the box's counters (doc 21
    /// cut G.1b). Default: ignored.
    fn pixel_event(&self, _event: PixelEvent) {}
}

/// What the terminal picture reports to the box's counters (doc 21 cut
/// G.1b; [`SshHooks::pixel_event`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelEvent {
    /// A view opened.
    Opened,
    /// A view closed (or its connection went).
    Closed,
    /// A picture went out, this many bytes on the wire.
    Sent(usize),
    /// A tick was skipped because the write before it was slow.
    SlowSkip,
    /// A view stepped down to the narrow frame and the low clock.
    SteppedDown,
}
