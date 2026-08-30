//! The directional boundary enums plus the correlated service vocabulary.
//!
//! Everything the gateway and server exchange is one of two tagged unions:
//! [`ToServer`] (gateway → server) and [`ToGateway`] (server → gateway). The
//! former oneshot service calls (`CreatePlayer`/`NameFree`/`InInputContext`/
//! `Barrier`) become correlated [`ServiceReq`]/[`ServiceResp`] pairs matched by
//! a `u64` request id.

use mush_core::{DisconnectKind, Inbound, ObjId};
use mush_proto::OutputEvent;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::target::SendTarget;

/// Operator/driver control, reused verbatim from the engine (`@shutdown`,
/// `@dump`). Distinct from the engine-internal [`Inbound::Control`] path: this
/// is the operator control the **gateway** forwards, carried by
/// [`ToServer::Control`].
pub use mush_core::ControlMsg;

/// The boundary protocol version, announced in [`ToGateway::Hello`]. Bump on a
/// breaking change; additive changes (new enum arms, new struct fields under a
/// self-describing encoding) keep the same version and stay wire-compatible
/// with older peers.
pub const IPC_VERSION: u32 = 1;

/// The cross-boundary subset of [`mush_core::Inbound`].
///
/// Only these five variants originate in the gateway and cross the seam.
/// [`Inbound::Apply`] (typed async completions) and [`Inbound::Control`]
/// (engine-internal control) are server-internal and are structurally excluded
/// here — there is no arm for them, so they cannot be sent over the wire.
///
/// Mirrors the shared [`Inbound`] variants field-for-field; convert with
/// [`From<WireInbound>`](From) and [`TryFrom<Inbound>`](TryFrom).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireInbound {
    /// One input line from a live connection ([`Inbound::Cmd`]).
    Cmd {
        /// The connection it arrived on.
        conn: u64,
        /// The actor it runs as.
        actor: ObjId,
        /// The raw line.
        line: String,
    },
    /// A structured panel-hydration request ([`Inbound::Query`]).
    Query {
        /// The connection the reply goes to.
        conn: u64,
        /// The requesting actor (permissions are evaluated as them).
        actor: ObjId,
        /// Client-chosen correlation id, echoed in the reply.
        id: u64,
        /// The registered query-handler name.
        name: SmolStr,
        /// Handler-defined parameters.
        args: serde_json::Value,
    },
    /// A connection finished auth and attached to an actor ([`Inbound::ConnUp`]).
    ConnUp {
        /// The new connection.
        conn: u64,
        /// The actor it attached to.
        actor: ObjId,
        /// Whether the client understands structured (JSON) events.
        caps_structured: bool,
        /// Whether this attach revived a link-dead session.
        from_linkdead: bool,
    },
    /// A connection dropped ([`Inbound::ConnDown`]).
    ConnDown {
        /// The connection that went away.
        conn: u64,
        /// The actor it was attached to.
        actor: ObjId,
        /// Why it went down.
        reason: DisconnectKind,
    },
    /// The session layer gave up on the actor entirely ([`Inbound::SessionGone`]).
    SessionGone {
        /// The actor whose session ended.
        actor: ObjId,
    },
}

/// [`Inbound`] carried a variant that is not part of the cross-boundary
/// protocol ([`Inbound::Apply`] or [`Inbound::Control`]): both are
/// server-internal and never cross the gateway seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireInboundError {
    /// [`Inbound::Apply`] — a typed async completion; re-enters the world loop
    /// internally, never from the gateway.
    #[error("Inbound::Apply is server-internal and does not cross the boundary")]
    Apply,
    /// [`Inbound::Control`] — engine-internal control; operator control crosses
    /// as [`ToServer::Control`] instead.
    #[error("Inbound::Control is server-internal and does not cross the boundary")]
    Control,
}

impl From<WireInbound> for Inbound {
    fn from(w: WireInbound) -> Inbound {
        match w {
            WireInbound::Cmd { conn, actor, line } => Inbound::Cmd { conn, actor, line },
            WireInbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            } => Inbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            },
            WireInbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            } => Inbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            },
            WireInbound::ConnDown {
                conn,
                actor,
                reason,
            } => Inbound::ConnDown {
                conn,
                actor,
                reason,
            },
            WireInbound::SessionGone { actor } => Inbound::SessionGone { actor },
        }
    }
}

impl TryFrom<Inbound> for WireInbound {
    type Error = WireInboundError;

    fn try_from(i: Inbound) -> Result<WireInbound, WireInboundError> {
        match i {
            Inbound::Cmd { conn, actor, line } => Ok(WireInbound::Cmd { conn, actor, line }),
            Inbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            } => Ok(WireInbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            }),
            Inbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            } => Ok(WireInbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            }),
            Inbound::ConnDown {
                conn,
                actor,
                reason,
            } => Ok(WireInbound::ConnDown {
                conn,
                actor,
                reason,
            }),
            Inbound::SessionGone { actor } => Ok(WireInbound::SessionGone { actor }),
            Inbound::Apply(_) => Err(WireInboundError::Apply),
            Inbound::Control(_) => Err(WireInboundError::Control),
        }
    }
}

/// An attach record: which actor a live connection is bound to, plus the
/// account/name/caps needed to rebuild session state. Sent one-per-connection
/// inside [`ToServer::ReAnnounce`] so a freshly (re)started server can
/// reconstruct the connection map after a restart (M16c re-attach).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttachRec {
    /// The physical connection id.
    pub conn: u64,
    /// The actor the connection is playing.
    pub actor: ObjId,
    /// The owning account (when known — pre-auth connections have none).
    pub account: Option<String>,
    /// The character name (for announces/logging on the server side).
    pub char_name: String,
    /// Whether the client understands structured (JSON) events.
    pub caps_structured: bool,
}

/// A correlated service request (gateway → server): the former oneshot calls,
/// matched to their [`ServiceResp`] by the `id` on [`ToServer::Request`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServiceReq {
    /// Create a Player object (account/character creation). Answered once the
    /// creation batch is acknowledged by the store.
    CreatePlayer {
        /// The character name.
        name: String,
    },
    /// Is this actor name (case-insensitive) currently unclaimed?
    NameFree {
        /// The name to probe.
        name: String,
    },
    /// Is this actor currently inside an input context (chargen wizard, mail
    /// compose, ...)?
    InInputContext {
        /// The actor to probe.
        actor: ObjId,
    },
    /// Ordering barrier: answered once every message queued before it has been
    /// processed and its output delivered.
    Barrier,
}

/// A correlated service reply (server → gateway), matched to its [`ServiceReq`]
/// by the `id` on [`ToGateway::Reply`]. Each arm answers the like-named
/// request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServiceResp {
    /// Result of a [`ServiceReq::CreatePlayer`]: the new id, or why creation
    /// failed.
    CreatePlayer(Result<ObjId, CreatePlayerError>),
    /// Result of a [`ServiceReq::NameFree`]: `true` = free.
    NameFree(bool),
    /// Result of a [`ServiceReq::InInputContext`]: `true` = inside a context.
    InInputContext(bool),
    /// Acknowledgement of a [`ServiceReq::Barrier`].
    Barrier,
}

/// Player-creation failure, carried in [`ServiceResp::CreatePlayer`]. A
/// serde-friendly mirror of `mushd`'s internal `CreatePlayerErr` so the reply
/// crosses the boundary without pulling the game server into this crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum CreatePlayerError {
    /// An actor with that name (or alias) already exists.
    #[error("that name is already in use")]
    NameTaken,
    /// Any other world error (invariant violation — should not happen).
    #[error("world error: {0}")]
    World(String),
    /// The world task is gone (shutdown).
    #[error("the world is shutting down")]
    WorldGone,
    /// The store is unavailable (read-only mode); creation is refused up front.
    #[error("the database is temporarily unavailable; please try again shortly")]
    StoreUnavailable,
}

/// Why the server is telling the gateway to drop a connection or a whole
/// session — the payload of [`ToGateway::CloseSession`]/[`ToGateway::Detach`].
///
/// A serde mirror of `mush-session`'s `DetachReason` so this crate stays free
/// of the session layer (invariant S4); `mushd` maps between the two. The
/// cause matters to the gateway's disconnect policy: a [`Booted`](DetachCause::Booted)
/// connection is closed hard, while a [`SlowClient`](DetachCause::SlowClient)
/// may enter the link-dead grace window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetachCause {
    /// Booted by staff (`@boot`, `@newpassword` logout-everywhere): hard close.
    Booted,
    /// Output buffer overflowed (slow-client policy): the session may go
    /// link-dead for its grace window rather than closing outright.
    SlowClient,
}

/// Gateway → server. Everything the gateway half sends across the boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    /// Live engine input — the cross-boundary subset of [`Inbound`].
    Inbound(WireInbound),
    /// A connection attached to an actor (session bookkeeping for re-attach).
    Attach {
        /// The physical connection id.
        conn: u64,
        /// The actor it is playing.
        actor: ObjId,
        /// The owning account (when known).
        account: Option<String>,
        /// The character name.
        char_name: String,
        /// Whether the client understands structured (JSON) events.
        caps_structured: bool,
    },
    /// A connection detached from its actor.
    Detach {
        /// The physical connection id.
        conn: u64,
        /// The actor it was playing.
        actor: ObjId,
        /// Why it detached.
        reason: DisconnectKind,
    },
    /// The gateway's full snapshot of live attachments, replayed to a freshly
    /// (re)started server so it can rebuild its connection map (M16c).
    ReAnnounce(Vec<AttachRec>),
    /// A correlated service request; the reply comes back as
    /// [`ToGateway::Reply`] with the same `id`.
    Request {
        /// Correlation id, echoed in the reply.
        id: u64,
        /// The request payload.
        req: ServiceReq,
    },
    /// Operator/driver control (`@shutdown`, `@dump`).
    Control(ControlMsg),
}

/// Server → gateway. Everything the server half sends across the boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToGateway {
    /// One resolved output: `event` goes to `target`. The world loop's
    /// `resolve_sends` already applied audience/ignore/perception filtering, so
    /// the gateway just routes.
    Deliver {
        /// Who receives it.
        target: SendTarget,
        /// The event.
        event: OutputEvent,
    },
    /// A correlated service reply, matched to its request by `id`.
    Reply {
        /// The `id` from the originating [`ToServer::Request`].
        id: u64,
        /// The reply payload.
        resp: ServiceResp,
    },
    /// Set (or clear) a character's flood-exempt flag. The world loop re-syncs
    /// this after every `Cmd` — a paste into an input context (mail body,
    /// chargen) must bypass the transport's burst cap. The gateway owns the
    /// throttle, so the server drives it across the seam rather than mutating
    /// the session layer directly.
    SetFloodExempt {
        /// The character whose flag to set (widened `ObjId`, as [`SendTarget::Char`]).
        char: u64,
        /// `true` while the actor sits in an input context.
        exempt: bool,
    },
    /// Close a whole session: every live connection of `char` is dropped
    /// (`@boot`). The gateway drops the transports and reports each resulting
    /// connection back to the server as a
    /// [`ToServer::Inbound`]`(`[`ConnDown`](WireInbound::ConnDown)`)` so the
    /// world can run the per-connection announce + last-connection disconnect.
    CloseSession {
        /// The character whose session to close (widened `ObjId`).
        char: u64,
        /// Why (drives the gateway's close-vs-linkdead policy).
        cause: DetachCause,
    },
    /// Detach one specific connection (slow-client policing). Like
    /// [`CloseSession`](ToGateway::CloseSession) but connection-scoped; the
    /// gateway reports the resulting [`ConnDown`](WireInbound::ConnDown) back to
    /// the server.
    Detach {
        /// The physical connection to detach.
        conn: u64,
        /// Why (drives the gateway's close-vs-linkdead policy).
        cause: DetachCause,
    },
    /// Account-addressed logout-everywhere (`@newpassword`): close every
    /// **structured** (web/token-authenticated) connection of the listed
    /// characters. SSH sessions are session-authenticated and survive. This is
    /// the split-mode replacement for the forked server's in-process walk of a
    /// `SessionManager` it does not hold (D4): the server names only the
    /// characters, and the gateway resolves each to its live connections on its
    /// own manager, delivers `notice` (if any) to each structured connection,
    /// drops it, and reports the resulting
    /// [`ConnDown`](WireInbound::ConnDown) back as inbound. Fire-and-forget —
    /// the world loop never awaits a reply.
    CloseWebSessions {
        /// The characters whose structured connections to close (widened
        /// `ObjId`s, as [`SendTarget::Char`](crate::SendTarget::Char)).
        chars: Vec<u64>,
        /// A farewell event delivered to each structured connection just before
        /// it is dropped (the `@newpassword` "log in again" notice). `None`
        /// closes without a message.
        notice: Option<OutputEvent>,
        /// Why (drives the gateway's close-vs-linkdead policy — `Booted` here).
        cause: DetachCause,
    },
    /// The server's opening handshake: its protocol version and advertised
    /// features. Sent once when a gateway connects.
    Hello {
        /// The server's [`IPC_VERSION`].
        proto: u32,
        /// A human-readable server identifier (build/version string).
        server: String,
        /// Advertised optional features (additive capability negotiation).
        features: Vec<String>,
    },
    /// The server is shutting the boundary down cleanly.
    Bye,
}
