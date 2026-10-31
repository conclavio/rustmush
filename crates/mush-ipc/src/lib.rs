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
//!   handshake, answered by the gateway's [`ToServer::Hello`].
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
//! struct fields extend the wire without breaking old peers (a new enum arm
//! does not, and is a version bump), and each half announces the versions it
//! serves, `[min_proto, proto]`, in its Hello. The two halves may run one
//! version apart in one direction (doc 33 §4.1): the server at N serves
//! gateways at N-1 and N through the per-link [`EmissionGate`]; see
//! [`protocol`]'s module docs for the rules a change follows.
//!
//! ## What this crate is NOT
//!
//! M16a passes these enums **in-process** (the world-task refactor is a
//! separate task). The [`codec`] is proven by this crate's round-trip tests and
//! carries real bytes only when M16b forks the two halves over a socket. This
//! crate depends on `mush-proto` + `mush-core` **only** — no store, modules,
//! llm, or net — so the future standalone gateway links the vocabulary without
//! pulling the game server.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod codec;
pub mod io;
mod link;
pub mod protocol;
mod target;
mod window;

pub use codec::{CodecError, FrameDecoder, MAX_FRAME_LEN, decode_frame, encode_frame};
pub use io::{
    BoxRead, BoxWrite, FrameReader, IpcEndpoint, IpcListener, connect, listen, write_frame,
};
pub use link::{ActionSend, ConnectionAdmission, GatewayLink, InputSend};
pub use protocol::{
    AttachRec, ControlMsg, CreatePlayerError, DetachCause, DiscordControlOp, DiscordInteraction,
    DiscordInteractionError, GatewayAction, GatewayInput, GatewayInputSizeError, GatewayRequest,
    GatewayRequestDisposition, IPC_MIN_PROTO, IPC_PRE_HELLO_VERSION, IPC_VERSION,
    LegalActionsError, MAX_GATEWAY_INPUT_BYTES, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, RelayError, RelaySpeak, ServiceReq, ServiceResp, ToGateway,
    ToServer, WireInbound, WireInboundError,
};
pub use target::SendTarget;
pub use window::{EmissionGate, IPC_VERSION_SKEW_MARKER, LinkVersion, PRE_WINDOW, ProtoWindow};

// Re-export the borrowed vocabulary so downstream (the future gateway) can name
// the boundary types through `mush_ipc` without also depending on `mush-core`
// or `mush-proto` directly.
pub use mush_core::{CanonicalHash, CommitToken, DisconnectKind, Inbound, ObjId, RequestUid};
pub use mush_proto::OutputEvent;
