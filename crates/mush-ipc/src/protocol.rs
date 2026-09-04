//! The directional boundary enums plus the correlated service vocabulary.
//!
//! Everything the gateway and server exchange is one of two tagged unions:
//! [`ToServer`] (gateway → server) and [`ToGateway`] (server → gateway). The
//! former oneshot service calls (`CreatePlayer`/`NameFree`/`InInputContext`/
//! `ConnectionReady`/`Barrier`) become correlated [`ServiceReq`]/[`ServiceResp`]
//! pairs matched by a `u64` request id.

use mush_core::{
    ActionOfferId, CommitToken, DisconnectKind, Inbound, ObjId, PerceivedRef, ProjectedLegalAction,
    RequestUid,
};
use mush_proto::{ActionStatusId, OutputEvent, ProjectedActionReceipt, ProjectedActionStatus};
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
pub const IPC_VERSION: u32 = 4;

/// Maximum replayable player inputs held by either half of one split-process
/// boundary. The gateway enforces the limit before accepting custody and the
/// server independently enforces it before retaining a legacy claim.
pub const MAX_IN_FLIGHT_GATEWAY_INPUTS: usize = 1_024;

/// Maximum replayable player inputs held for one physical connection.
pub const MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION: usize = 32;

/// Maximum logical bytes held by the replayable player-input pool.
pub const MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// Maximum logical bytes held for one physical connection.
pub const MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION: usize = 512 * 1024;

/// Maximum logical size of one replayable input. This rejects a pathological
/// structured query before its allocation can consume the whole custody pool.
pub const MAX_GATEWAY_INPUT_BYTES: usize = 128 * 1024;

/// Maximum number of correlated service requests one split-process boundary
/// may hold in flight at once.
///
/// Service requests ultimately serialize through the single world task, so a
/// larger backlog does not add throughput; it only increases gateway waiter
/// custody and server task count. Both halves enforce this same hard ceiling:
/// the gateway refuses new custody with [`ServiceResp::Busy`], while the
/// server applies the limit independently so a nonconforming peer cannot spawn
/// an unbounded number of handlers.
pub const MAX_IN_FLIGHT_SERVICE_REQUESTS: usize = 256;

/// Maximum number of uncompleted connection-lifecycle obligations the split
/// gateway may own at once.
///
/// An obligation is either one live/reserved physical connection or one
/// link-dead actor whose eventual [`WireInbound::SessionGone`] is still owed.
/// The gateway reserves a slot *before* attaching a transport to its local
/// session manager. Once reserved, that connection's [`WireInbound::ConnUp`]
/// and teardown frames remain must-deliver; overload is refused at admission,
/// rather than by shedding a lifecycle frame after custody was taken.
///
/// Keeping this independent from the configurable input high-water also bounds
/// the restart [`ToServer::ReAnnounce`] snapshot and the number of teardown
/// frames that can arrive after a server stops draining.
pub const MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS: usize = 256;

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

/// Player-originated input that participates in the restart custody protocol.
///
/// Lifecycle messages deliberately cannot inhabit this enum: they retain their
/// existing ordered, must-deliver path, while only floodable player input is
/// admitted to the bounded replay pool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GatewayInput {
    /// One command line from a live connection.
    Cmd {
        /// The connection it arrived on.
        conn: u64,
        /// The actor it runs as.
        actor: ObjId,
        /// The raw line.
        line: String,
    },
    /// A structured, replay-safe panel hydration request.
    Query {
        /// The connection the reply goes to.
        conn: u64,
        /// The requesting actor.
        actor: ObjId,
        /// Client-chosen correlation id, echoed in the reply.
        id: u64,
        /// The registered query-handler name.
        name: SmolStr,
        /// Handler-defined parameters.
        args: serde_json::Value,
    },
    /// A structured action invocation for the authoritative coordinator.
    ///
    /// The connection/actor pair is transport evidence only: for a new request
    /// the server verifies it against the live connection map and constructs
    /// the authenticated principal itself. An exact existing durable binding
    /// may settle a replay after that original connection disappears. The
    /// opaque offer is the only action selector crossing this boundary; exact
    /// mechanics identity remains server-held and is regenerated before use.
    Action(Box<GatewayAction>),
}

/// Structured payload for [`GatewayInput::Action`]. Boxed in the parent enum
/// so adding bounded semantic metadata does not make every ordinary line/query
/// frame carry the action variant's stack footprint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GatewayAction {
    /// The physical connection that submitted the action.
    pub conn: u64,
    /// The actor asserted by the gateway and revalidated by the server.
    pub actor: ObjId,
    /// Opaque offer identity echoed into the semantic invocation.
    pub offered_action: ActionOfferId,
    /// Perceived subject reference, never an unrestricted truth lookup.
    pub subject: PerceivedRef,
    /// Bounded perceived target references.
    pub targets: Vec<PerceivedRef>,
    /// Provider-defined canonical parameters.
    pub parameters: serde_json::Value,
}

impl GatewayInput {
    /// The physical connection whose bounded custody account owns this input.
    #[must_use]
    pub const fn conn(&self) -> u64 {
        match self {
            Self::Cmd { conn, .. } | Self::Query { conn, .. } => *conn,
            Self::Action(action) => action.conn,
        }
    }

    /// A deterministic accounting size for custody limits.
    ///
    /// This is not the MessagePack frame size. It counts caller-controlled
    /// variable data plus fixed scalar widths, so both peers apply the same
    /// bound without depending on a serializer implementation detail.
    pub fn logical_bytes(&self) -> Result<usize, GatewayInputSizeError> {
        let bytes = match self {
            Self::Cmd { line, .. } => 16_usize.saturating_add(line.len()),
            Self::Query { name, args, .. } => {
                let args = serde_json::to_vec(args)
                    .map_err(|error| GatewayInputSizeError::InvalidJson(error.to_string()))?;
                24_usize
                    .saturating_add(name.len())
                    .saturating_add(args.len())
            }
            Self::Action(action) => {
                let payload = serde_json::to_vec(&serde_json::json!({
                    "offered_action": action.offered_action,
                    "subject": action.subject,
                    "targets": action.targets,
                    "parameters": action.parameters,
                }))
                .map_err(|error| GatewayInputSizeError::InvalidJson(error.to_string()))?;
                16_usize.saturating_add(payload.len())
            }
        };
        if bytes > MAX_GATEWAY_INPUT_BYTES {
            return Err(GatewayInputSizeError::TooLarge {
                actual: bytes,
                maximum: MAX_GATEWAY_INPUT_BYTES,
            });
        }
        Ok(bytes)
    }
}

impl From<GatewayInput> for Inbound {
    fn from(input: GatewayInput) -> Self {
        match input {
            GatewayInput::Cmd { conn, actor, line } => Self::Cmd { conn, actor, line },
            GatewayInput::Query {
                conn,
                actor,
                id,
                name,
                args,
            } => Self::Query {
                conn,
                actor,
                id,
                name,
                args,
            },
            GatewayInput::Action(_) => {
                unreachable!("authoritative actions enter through the coordinator, not Inbound")
            }
        }
    }
}

impl TryFrom<WireInbound> for GatewayInput {
    type Error = WireInbound;

    fn try_from(input: WireInbound) -> Result<Self, Self::Error> {
        match input {
            WireInbound::Cmd { conn, actor, line } => Ok(Self::Cmd { conn, actor, line }),
            WireInbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            } => Ok(Self::Query {
                conn,
                actor,
                id,
                name,
                args,
            }),
            lifecycle => Err(lifecycle),
        }
    }
}

/// Failure to account a gateway input under the custody bounds.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GatewayInputSizeError {
    /// A JSON value unexpectedly failed to serialize.
    #[error("structured input is not serializable: {0}")]
    InvalidJson(String),
    /// The input exceeds the per-request custody limit.
    #[error("gateway input is {actual} logical bytes; maximum is {maximum}")]
    TooLarge {
        /// The measured logical size.
        actual: usize,
        /// The configured hard maximum.
        maximum: usize,
    },
}

/// One replay-stable request held in gateway custody until the server returns
/// an explicit ownership disposition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GatewayRequest {
    /// Stable identity reused verbatim across reconnect/restart replay.
    pub uid: RequestUid,
    /// Random gateway-process epoch embedded in the first half of `uid`.
    pub epoch: u64,
    /// The player input payload.
    pub input: GatewayInput,
}

impl GatewayRequest {
    /// Construct an identity from one process epoch and monotonic sequence.
    #[must_use]
    pub fn new(epoch: u64, sequence: u64, input: GatewayInput) -> Self {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&epoch.to_be_bytes());
        bytes[8..].copy_from_slice(&sequence.to_be_bytes());
        Self {
            uid: RequestUid::from_bytes(bytes),
            epoch,
            input,
        }
    }

    /// Validate that the separately visible epoch agrees with the stable UID.
    #[must_use]
    pub fn has_valid_uid(&self) -> bool {
        self.uid.as_bytes()[..8] == self.epoch.to_be_bytes()
    }
}

/// Server ownership result for a replayable gateway request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GatewayRequestDisposition {
    /// Refused before execution or durable ownership was taken.
    Rejected {
        /// Whether a fresh player submission may reasonably succeed later.
        retryable: bool,
    },
    /// Refused before ownership because a bounded admission pool is full.
    /// The same request may be submitted again by the player later.
    Busy,
    /// A replay-safe query has been accepted; no durable ownership is needed.
    EphemeralFinished,
    /// The server has classified a legacy command but will not execute until
    /// the gateway proves it released its replay copy.
    LegacyClaimed,
    /// The complete authoritative mutation is durably owned by K3.
    Owned {
        /// Stable token for exact-token retry and status lookup.
        token: CommitToken,
        /// Public recovery reference. No private bundle hash crosses the
        /// presentation boundary.
        status: ActionStatusId,
    },
    /// The authoritative request reached terminal durable completion.
    Finished {
        /// Authenticated character receiving this result even when the
        /// original physical connection has disappeared.
        recipient: ObjId,
        /// Bounded redacted semantic result; the compact store receipt stays
        /// server-private.
        receipt: ProjectedActionReceipt,
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
    /// Does the engine currently track this exact transport connection as
    /// attached to this actor? Used after the connect ordering barrier so the
    /// gateway never publishes a local session whose `ConnUp` was refused by
    /// a hard world mode or ignored for an invalid actor.
    ConnectionReady {
        /// The physical connection id.
        conn: u64,
        /// The actor the gateway attached locally.
        actor: ObjId,
    },
    /// Enumerate viewer-safe legal actions for one perceived subject. The
    /// connection/actor pair is revalidated by the world task before any
    /// provider can project an offer.
    LegalActions {
        /// The physical connection making the request.
        conn: u64,
        /// The actor currently attached to `conn`.
        actor: ObjId,
        /// A previously projected subject reference. `KnownObject` is
        /// accepted only when its revision matches the actor's one
        /// unambiguous current station-grant epoch; contact scopes require
        /// their K6 private lifecycle provider.
        subject: PerceivedRef,
    },
    /// Recover one previously accepted authoritative action by its public
    /// transaction reference. The connection/actor pair is authenticated
    /// before the server reconstructs the principal-scoped store key.
    ActionStatus {
        conn: u64,
        actor: ObjId,
        status: ActionStatusId,
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
    /// Result of a [`ServiceReq::ConnectionReady`]: `true` = the exact
    /// connection-to-actor association exists in the engine.
    ConnectionReady(bool),
    /// Result of a [`ServiceReq::LegalActions`] projection. Busy and world
    /// disappearance retain the request-level [`Self::Busy`] and
    /// [`Self::Unavailable`] arms; this result carries semantic refusals.
    LegalActions(Result<Vec<ProjectedLegalAction>, LegalActionsError>),
    /// Authorized, persistence-internal-free action status/result.
    ActionStatus(ProjectedActionStatus),
    /// Acknowledgement of a [`ServiceReq::Barrier`].
    Barrier,
    /// The boundary has reached its fixed correlated-request custody limit.
    ///
    /// This is a refusal *before* the request enters world-task custody. A
    /// caller may retry later; it must not interpret this as a negative answer
    /// to the request itself (for example, `NameFree(false)`).
    Busy,
    /// The request was admitted by the boundary, but its server-side service
    /// became unavailable before it could acknowledge completion.
    ///
    /// In particular, a failed ordering barrier must use this arm: failure may
    /// never be synthesized into [`ServiceResp::Barrier`].
    Unavailable,
}

/// Semantic refusal for a legal-action projection that reached the world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum LegalActionsError {
    /// The perceived subject kind is not yet backed by a safe authoritative
    /// projection/revision path.
    #[error("that perceived subject cannot be used for legal-action discovery")]
    InvalidSubject,
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
    /// A player command/query under bounded, replay-stable gateway custody.
    Input(GatewayRequest),
    /// The gateway has discarded its replay copy of a legacy request. The
    /// server may execute it at most once after receiving this release.
    GatewayReleased {
        /// The request whose gateway custody was released.
        request: RequestUid,
    },
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
    /// Ownership/completion classification for a replayable player input.
    RequestDisposition {
        /// Stable request identity from [`GatewayRequest::uid`].
        request: RequestUid,
        /// The server's current terminal or handoff state.
        disposition: GatewayRequestDisposition,
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
    /// The server's opening handshake: its protocol version, advertised
    /// features, and the web `hello` panel manifest. Sent once when a gateway
    /// connects (and re-sent by a freshly (re)started server so the gateway can
    /// refresh its cached manifest across a restart). Additive: `panels` was
    /// added after `features` under the self-describing rmp codec, so both wire
    /// halves rebuild together and [`IPC_VERSION`] stays the same.
    Hello {
        /// The server's [`IPC_VERSION`].
        proto: u32,
        /// A human-readable server identifier (build/version string).
        server: String,
        /// Advertised optional features (additive capability negotiation) —
        /// one id per loaded module (comsys advertises as `"channels"`). The
        /// gateway copies these into the web `hello` frame's `features`.
        features: Vec<String>,
        /// The web `hello` panel-feed manifest: every module-registered panel
        /// feed plus the engine-side `"who"`. The gateway copies these into the
        /// web `hello` frame's `panels` and the [`WebHooks`] panel routing.
        ///
        /// [`WebHooks`]: https://docs.rs/mush-net-web
        panels: Vec<String>,
    },
    /// The server is shutting the boundary down cleanly.
    Bye,
}
