//! The directional boundary enums plus the correlated service vocabulary.
//!
//! Everything the gateway and server exchange is one of two tagged unions:
//! [`ToServer`] (gateway → server) and [`ToGateway`] (server → gateway). The
//! former oneshot service calls (`CreatePlayer`/`NameFree`/`InInputContext`/
//! `ConnectionReady`/`Barrier`) become correlated [`ServiceReq`]/[`ServiceResp`]
//! pairs matched by a `u64` request id.
//!
//! # A new arm is not a free extension
//!
//! Adding a variant to either enum is wire-compatible in ONE direction only: a
//! new peer reading an old peer's frames. The reverse — an old peer meeting a
//! frame it has no arm for — is an rmp decode error, which [`crate::codec`]
//! maps to an `io::Error` and [`crate::io`] treats as a dead boundary: the
//! connection tears down and every player on it drops. Nor is there a
//! gateway→server handshake to gate emission on ([`ToGateway::Hello`] carries
//! `proto`/`features` server→gateway only; [`ToServer`] has no counterpart), so
//! a sender cannot learn what its peer understands before it speaks.
//!
//! A new arm is therefore a deploy-order hazard, not an extension point, and
//! the standing rule is: **reach for a new frame last.** Both halves already
//! share the object store, so a fact one half needs from the other is usually
//! already written down there, and re-reading it costs one small query and no
//! version skew. The gateway's siteban mirror is the worked example —
//! `@siteban` runs in the server, the guard that admits connections lives in
//! the gateway, and the gap is closed by the gateway re-reading `ban:*` rather
//! than by a `ToGateway::SetSiteban`.

use mush_core::{
    AccountId, ActionOfferId, CommitToken, DisconnectKind, Inbound, ObjId, PerceivedRef,
    ProjectedLegalAction, RequestUid,
};
use mush_proto::{
    ActionStatusId, NarrativeKind, OutputEvent, ProjectedActionReceipt, ProjectedActionStatus,
    RespondChoice,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::target::SendTarget;

/// Operator/driver control, reused verbatim from the engine (`@shutdown`,
/// `@dump`). Distinct from the engine-internal [`Inbound::Control`] path: this
/// is the operator control the **gateway** forwards, carried by
/// [`ToServer::Control`].
pub use mush_core::ControlMsg;

/// The boundary protocol version, announced in [`ToGateway::Hello`]. Bump on a
/// decoding or behavioral incompatibility. Defaulted fields under the
/// self-describing encoding can be additive, provided old peers remain safe.
// Version 5 requires request epochs and account revocation controls.
pub const IPC_VERSION: u32 = 5;

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
    /// A composing hint from a live connection ([`Inbound::Composing`],
    /// interface plan I4): a best-effort presence signal, already
    /// rate-limited by the bridge, so it rides the plain one-way path rather
    /// than the input custody envelope.
    Composing {
        /// The connection it arrived on.
        conn: u64,
        /// The actor typing.
        actor: ObjId,
        /// `true` when a draft began, `false` when it was sent or abandoned.
        active: bool,
    },
    /// A client's answer to a server-initiated request ([`Inbound::Respond`],
    /// interface plan I10): advisory and best-effort, so it rides the plain
    /// one-way path like `Composing`. A lost answer means the server applies
    /// its deadline default.
    Respond {
        /// Opaque lifetime of the engine that opened this request.
        #[serde(default)]
        epoch: String,
        /// The connection it arrived on.
        conn: u64,
        /// The actor answering (must be the request's subject).
        actor: ObjId,
        /// The request id from the originating `Request`.
        id: u64,
        /// The viewer's choice.
        choice: RespondChoice,
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

/// The world's inbound queue accepts only the two replayable player-input
/// kinds. An authoritative action has no `Inbound` form at all: it enters
/// through the coordinator, which owns its durable intent and its single
/// disposition. The conversion therefore *refuses* that arm and hands the
/// input back, so a caller holding custody of it makes a custody decision
/// instead of asserting the process to death on a frame a raw peer chose.
impl TryFrom<GatewayInput> for Inbound {
    /// The refused input, returned unchanged so the caller keeps custody.
    type Error = GatewayInput;

    fn try_from(input: GatewayInput) -> Result<Self, Self::Error> {
        match input {
            GatewayInput::Cmd { conn, actor, line } => Ok(Self::Cmd { conn, actor, line }),
            GatewayInput::Query {
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
            input @ GatewayInput::Action(_) => Err(input),
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
            WireInbound::Composing {
                conn,
                actor,
                active,
            } => Inbound::Composing {
                conn,
                actor,
                active,
            },
            WireInbound::Respond {
                epoch,
                conn,
                actor,
                id,
                choice,
            } => Inbound::Respond {
                epoch,
                conn,
                actor,
                id,
                choice,
            },
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
            Inbound::Composing {
                conn,
                actor,
                active,
            } => Ok(WireInbound::Composing {
                conn,
                actor,
                active,
            }),
            Inbound::Respond {
                epoch,
                conn,
                actor,
                id,
                choice,
            } => Ok(WireInbound::Respond {
                epoch,
                conn,
                actor,
                id,
                choice,
            }),
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
    /// A Discord component click or modal submission the gateway bridge does
    /// not route itself (interface programme D3, 03 §7a.4): the server
    /// dispatches it by `custom_id` prefix to the handler a module registered
    /// at wiring. Additive (same `IPC_VERSION`).
    DiscordInteraction(DiscordInteraction),
    /// A line a linked Discord user typed in a relayed Discord channel
    /// (interface programme D4): spoken on the mapped comsys channel as the
    /// user's character, after the server re-checks the `(account, actor)`
    /// claim against its boot-hydrated oracle and comsys runs membership, gag
    /// and speak-lock. Additive (same `IPC_VERSION`).
    RelaySpeak(RelaySpeak),
}

/// A relayed line forwarded to the server (D4). The bridge authenticated the
/// Discord user (the Gateway asserts `user_id`), resolved the linked account
/// and chose the character; the server trusts none of the pairing until its
/// own oracle agrees.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaySpeak {
    /// The Discord user's snowflake. Never rendered; `Debug` redacts it.
    pub user_id: u64,
    /// The account the user is linked to, as the store's minted id.
    pub account: AccountId,
    /// The character to speak as (followed, else the relay character, else
    /// the account's first).
    pub actor: ObjId,
    /// The comsys channel name the Discord channel maps to.
    pub channel: String,
    /// Say, pose or semipose (the Discord line's leading `:` / `;`).
    pub kind: NarrativeKind,
    /// The line, already trimmed and bounded by the bridge.
    pub text: String,
}

impl std::fmt::Debug for RelaySpeak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelaySpeak")
            .field("user_id", &"<redacted>")
            .field("actor", &self.actor)
            .field("channel", &self.channel)
            .field("kind", &self.kind)
            .field("chars", &self.text.chars().count())
            .finish()
    }
}

/// Why a [`ServiceReq::RelaySpeak`] was not spoken. Every arm is bounded and
/// viewer-safe; the bridge may show it to the Discord author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RelayError {
    /// The server's oracle does not pair the character with the account (a
    /// stale or forged claim, or a character created since boot).
    #[error("that character is not on your account as far as the game knows")]
    AccountMismatch,
    /// The line is over the relay's length cap.
    #[error("that line is too long for the relay")]
    TooLong,
    /// Comsys refused, with its reason.
    #[error("{0}")]
    Refused(String),
}

/// A Discord interaction forwarded to the server (D3). The bridge has already
/// authenticated the Discord user (the Gateway asserts `user_id`) and resolved
/// the account it is linked to; a handler trusts both.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordInteraction {
    /// The Discord user's snowflake. Never rendered; `Debug` redacts it.
    pub user_id: u64,
    /// The account the user is linked to (display name), if any.
    pub account: Option<String>,
    /// The character the bridge chose for the account — the followed one,
    /// else the relay character — if any.
    pub actor: Option<ObjId>,
    /// The component's routing key (`d:oncall:ack:<incident>`).
    pub custom_id: String,
    /// A select's picked values, or a modal's field values in order.
    pub values: Vec<String>,
}

impl std::fmt::Debug for DiscordInteraction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordInteraction")
            .field("user_id", &"<redacted>")
            .field("account", &self.account)
            .field("actor", &self.actor)
            .field("custom_id", &self.custom_id)
            .field("values", &self.values.len())
            .finish()
    }
}

/// A Discord-side control the server asks the gateway bridge to perform for a
/// character (programme D3): the in-game `@discord/follow` / `/unfollow`
/// verbs and a live `quiet` toggle. Fire-and-forget; the bridge answers the
/// character with a `discord`-tagged system line. Additive (same
/// `IPC_VERSION`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscordControlOp {
    /// Revoke cached and in-flight access after unlink or ban.
    Revoke,
    /// Attach a follow for the character's linked Discord user.
    Follow,
    /// Detach the character's follow, if any.
    Unfollow,
    /// Set the current follow's quiet mode (the persisted preference is the
    /// server's; this is the live copy).
    Quiet(bool),
}

/// Why a [`ServiceReq::DiscordInteraction`] produced no reply text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum DiscordInteractionError {
    /// No module registered a handler for the `custom_id`'s prefix.
    #[error("no handler for this control")]
    UnknownPrefix,
    /// The handler refused, with a bounded, viewer-safe reason.
    #[error("{0}")]
    Refused(String),
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
    /// Result of a [`ServiceReq::DiscordInteraction`]: the handler's bounded
    /// reply text, or why there is none.
    DiscordInteraction(Result<String, DiscordInteractionError>),
    /// Result of a [`ServiceReq::RelaySpeak`]: spoken, or why not.
    RelaySpeak(Result<(), RelayError>),
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
    /// Ask the gateway's Discord bridge to follow, unfollow or re-tune a
    /// character's Discord follow (programme D3). Fire-and-forget: the bridge
    /// tells the character what happened as a `discord`-tagged system line.
    DiscordControl {
        /// The character (widened `ObjId`).
        char: u64,
        /// What to do.
        op: DiscordControlOp,
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
