//! The gateway<->server boundary protocol (milestone M16).
//!
//! rustmush's single `mushd` binary already has an in-process seam between the
//! **gateway** (connection I/O: the SSH/web bridges + `mush_session`'s
//! `SessionManager`) and the **server** (the single-writer world task owning
//! the [`mush_core::Engine`]). M16 splits that seam across a process boundary
//! for zero-downtime restarts; this crate is the SAFE FIRST STAGE (M16a): it
//! defines the wire vocabulary the two halves speak, so the boundary can be
//! refactored to talk the protocol in-process *before* any fork.
//!
//! ## What crosses the boundary
//!
//! Two directions, each a single tagged enum:
//!
//! - [`ToServer`] — gateway → server: live input ([`WireInbound`], the
//!   cross-boundary subset of [`mush_core::Inbound`]), connection lifecycle
//!   ([`ToServer::Attach`]/[`ToServer::Detach`]/[`ToServer::ReAnnounce`]),
//!   correlated service requests ([`ServiceReq`]), and operator control.
//! - [`ToGateway`] — server → gateway: resolved per-target output
//!   ([`ToGateway::Deliver`] carrying a [`SendTarget`] + [`OutputEvent`]),
//!   correlated service replies ([`ServiceResp`]), and the [`ToGateway::Hello`]
//!   handshake.
//!
//! [`mush_core::Inbound::Apply`] (async effect completions) and
//! [`mush_core::Inbound::Control`] (routed internally by the world loop) are
//! **server-internal** and deliberately excluded from [`WireInbound`]: they
//! never originate in the gateway, so they must not appear in the cross-boundary
//! protocol. (Operator control *does* cross — as the top-level
//! [`ToServer::Control`] arm — but that is a distinct path from the engine's
//! internal `Inbound::Control`.)
//!
//! ## Wire format
//!
//! Messages are framed length-prefixed ([`codec`]) and bodies are encoded with
//! a **self-describing** MessagePack encoding ([`rmp_serde`]) — not `bincode`,
//! because [`mush_core::Inbound::Query`] and [`OutputEvent::Panel`] carry
//! `serde_json::Value` payloads whose shape is only known at runtime. The
//! encoding is additive/version-tolerant like the client protocol: new enum
//! arms and new struct fields extend the wire without breaking old peers, and
//! [`IPC_VERSION`] is announced in [`ToGateway::Hello`].
//!
//! ## What this crate is NOT
//!
//! M16a passes these enums **in-process** (the world-task refactor is a
//! separate task). The [`codec`] is proven by this crate's round-trip tests and
//! carries real bytes only when M16b forks the two halves over a socket. This
//! crate depends on `mush-proto` + `mush-core` **only** — no store, modules,
//! llm, or net — so the future standalone gateway links the vocabulary without
//! pulling the game server.

pub mod codec;
pub mod io;
mod link;
mod protocol;
mod target;

pub use codec::{CodecError, FrameDecoder, MAX_FRAME_LEN, decode_frame, encode_frame};
pub use io::{
    BoxRead, BoxWrite, FrameReader, IpcEndpoint, IpcListener, connect, listen, write_frame,
};
pub use link::{ConnectionAdmission, GatewayLink, InputSend};
pub use protocol::{
    AttachRec, ControlMsg, CreatePlayerError, DetachCause, GatewayAction, GatewayInput,
    GatewayInputSizeError, GatewayRequest, GatewayRequestDisposition, IPC_VERSION,
    MAX_GATEWAY_INPUT_BYTES, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound,
    WireInboundError,
};
pub use target::SendTarget;

// Re-export the borrowed vocabulary so downstream (the future gateway) can name
// the boundary types through `mush_ipc` without also depending on `mush-core`
// or `mush-proto` directly.
pub use mush_core::{CanonicalHash, CommitToken, DisconnectKind, Inbound, ObjId, RequestUid};
pub use mush_proto::OutputEvent;
