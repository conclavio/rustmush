//! [`SendTarget`]: one fully resolved output destination.
//!
//! Moved here from `mushd`'s world task. The world loop's `resolve_sends`
//! expands every [`mush_core::Audience`] against **current** world state (room
//! membership, `@ignore`, M11/M14 perception gating) into a flat list of
//! `(SendTarget, OutputEvent)` pairs — the server half of the delivery split.
//! Each pair becomes a [`ToGateway::Deliver`](crate::ToGateway::Deliver); the
//! gateway's `transmit` maps the `SendTarget` back onto its `SessionManager`
//! (character fan-out, single connection, or global broadcast).
//!
//! The payloads are raw `u64` routing keys, not `mush-session` newtypes: this
//! crate must not depend on the session layer (invariant S4). `mushd` widens a
//! `mush_core::ObjId` into the `Char` key exactly as it always has
//! (`u64::from(id.0)`) and wraps the `Conn` key back into its `ConnId(u64)` on
//! the gateway side.

use serde::{Deserialize, Serialize};

/// One fully resolved send target: world-side audience resolution (room
/// membership, ignore/perception filtering) has already been applied, so the
/// gateway only has to route.
///
/// The `u64` payloads are opaque routing keys. `Char` is a widened
/// `mush_core::ObjId` (the session layer's character fan-out key); `Conn` is a
/// single physical connection id (`mush_session::ConnId`'s inner value).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SendTarget {
    /// Every live connection of one character (widened `ObjId`).
    Char(u64),
    /// One specific connection.
    Conn(u64),
    /// Every connection on the server.
    All,
    /// A named relay sink outside the world (interface programme D4): the
    /// gateway's Discord bridge mirrors it to the mapped Discord channel. The
    /// name is the comsys channel's. Additive (same `IPC_VERSION`).
    Relay(smol_str::SmolStr),
}
