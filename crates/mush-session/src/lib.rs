//! # mush-session — the session layer
//!
//! Everything between a live transport (an SSH channel today, a WebSocket in
//! M4) and the game world: connection registration, session attachment,
//! multi-connect fan-out, linkdead grace, idle bookkeeping, input flood
//! control, and input sanitation.
//!
//! ## Where this crate sits (treaty ruling 2)
//!
//! The **world task** resolves *game* audiences (room → list of character
//! ids); this crate's [`SessionManager`] resolves *character → live
//! connections* and owns delivery + backpressure. Events are `Arc`-wrapped
//! once by the caller and the `Arc` is cloned per connection — a room-wide
//! emit to 200 players is one allocation.
//!
//! ## Id bridging (no `mush-core` dependency)
//!
//! This crate deliberately does **not** depend on `mush-core`; `mushd` is the
//! bridge between the two vocabularies:
//!
//! - [`ConnId`] here is a newtype over `u64`. `mush-core`'s `ConnId` is a
//!   plain `type ConnId = u64` alias — the same value; `mushd` converts with
//!   `.0` / `ConnId(n)`.
//! - [`CharKey`] is an opaque `u64` character key. `mush-core::ObjId` is a
//!   `u32`; `mushd` widens it (`CharKey(u64::from(obj.0))`) and narrows it
//!   back. This crate never interprets the value.
//!
//! ## Concepts
//!
//! - **Connection** ([`ConnectionHandle`]): one physical transport. Owns
//!   caps, peer address, its output [`Sink`] (ANSI or structured), idle
//!   bookkeeping, and the slow-client "dropped output" flag.
//! - **Session**: one authenticated attachment of an *account* to a
//!   *character*, with **1..N connections** (classic multi-connect: all
//!   connections receive all output; input is accepted from any).
//!
//! ## Two renders, one stamping path (treaty ruling 1, M4)
//!
//! A connection's [`Sink`] types its wire: [`Sink::Ansi`] transports (SSH)
//! receive plain `Arc<OutputEvent>`s and render ANSI; [`Sink::Structured`]
//! transports (web) receive `Arc<Envelope>`s — the same event stamped with
//! a per-session monotonic `seq` and a wall-clock `ts` from the
//! [`EnvelopeClock`] injected at construction. Every session-scoped send
//! funnels through one stamping path
//! ([`SessionManager::send_event_to_char`] and the `Arc`-taking
//! equivalents), which also records the envelope in the session's **resume
//! ring** (capacity [`SessionConfig::resume_ring`], default 512).
//! Recording is always on for every session — memory cost documented on
//! [`SessionConfig::resume_ring`] — so a web client that dropped its socket
//! resumes with [`SessionManager::replay_since`]`(sess, last_seq)`: the
//! missed envelopes in order, or [`ReplayError::TooOld`] when the gap
//! outgrew the ring and the client must re-hydrate. Seqs live on the
//! session, so they stay monotonic across reconnects; connection-scoped
//! sends ([`SessionManager::send_to_conn`]) consume seqs but are never
//! recorded — a prompt for one connection must not replay to another.
//!
//! Authoritative `resolution.outcome` presentation has a second, bounded
//! retention rule. A structured connection acknowledges the exact projected
//! commit token after applying it; until then the session re-emits that receipt
//! on reconnect even if transport sequence resume had already advanced past
//! it. ANSI clients have no ACK and recover through the stable transaction
//! reference plus `action/status` and current state keyframes.
//!
//! ## Linkdead contract (no internal timers)
//!
//! When the last connection of a session detaches for an *unclean* reason
//! ([`DetachReason::Lost`] / [`DetachReason::SlowClient`]), the session
//! enters **linkdead** instead of closing. This crate runs **no tokio timers
//! of its own** — it is fully testable without a runtime. The driver
//! (`mushd`) owns the clock:
//!
//! 1. [`SessionManager::detach`] returns [`DetachOutcome::Linkdead`] with the
//!    configured grace ([`SessionConfig::linkdead_grace`], default 300 s).
//! 2. The driver periodically (or at [`SessionManager::next_linkdead_expiry`])
//!    calls [`SessionManager::take_expired`] with "now"; expired sessions are
//!    removed and returned so the driver can tell the world to sweep the
//!    character.
//! 3. A reconnect ([`SessionManager::attach`] for the same [`CharKey`])
//!    cancels linkdead and reports `Rejoined { from_linkdead: true, .. }`.
//!
//! ## Slow-client policy
//!
//! All sends are non-blocking `try_send`. A full (or closed) per-connection
//! buffer sets that connection's *dropped* flag
//! ([`ConnectionHandle::output_dropped`]) and the connection is reported in
//! the send result; the caller decides whether to disconnect it (typically
//! `detach(conn, DetachReason::SlowClient)`). Nothing here ever awaits.
//!
//! Everything in this crate is `Send + Sync`; interior mutability is DashMap
//! shards plus per-connection atomics/mutexes.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
// WS01-S10B: with the six lock-poison `.expect`s below converted to the
// recovery policy, this crate's production `expect` surface is zero too.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod conn;
mod flood;
mod manager;
mod poison;
mod sanitize;

pub(crate) use poison::{locked, read_locked, write_locked};

pub use conn::{ConnId, ConnectionHandle, Sink};
pub use flood::{FloodBucket, FloodVerdict};
pub use manager::{
    AttachOutcome, CharKey, DetachOutcome, DetachReason, EnvelopeClock, ExpiredSession,
    LinkdeadHook, NarrativeBacklog, ReplayError, SendReport, SendToConn, SessId, SessionConfig,
    SessionManager, SessionMeta, system_unix_ms,
};
pub use sanitize::sanitize_line;
