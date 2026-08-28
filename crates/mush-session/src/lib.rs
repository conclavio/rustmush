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
//!   caps, peer address, output sender, idle bookkeeping, and the
//!   slow-client "dropped output" flag.
//! - **Session**: one authenticated attachment of an *account* to a
//!   *character*, with **1..N connections** (classic multi-connect: all
//!   connections receive all output; input is accepted from any).
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

mod conn;
mod flood;
mod manager;
mod sanitize;

pub use conn::{ConnId, ConnectionHandle};
pub use flood::{FloodBucket, FloodVerdict};
pub use manager::{
    AttachOutcome, CharKey, DetachOutcome, DetachReason, ExpiredSession, SendReport, SendToConn,
    SessId, SessionConfig, SessionManager, SessionMeta,
};
pub use sanitize::sanitize_line;
