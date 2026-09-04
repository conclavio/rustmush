//! # mush-proto — the rustmush wire vocabulary
//!
//! The single protocol crate shared by the game core, both renderers, and the
//! web client (treaty ruling 1: **one protocol**). `mush-core` emits
//! [`OutputEvent`] values directly; `mush-ansi` renders them to ANSI bytes for
//! SSH, while the web transport serializes them as JSON — the enum *is* the
//! wire format. Client input arrives as [`ClientCommand`].
//!
//! Design invariants:
//!
//! - **serde only.** No async, no I/O, no game logic. Everything is
//!   `Serialize + Deserialize + Clone + Debug + PartialEq`.
//! - **Self-describing JSON.** Events and commands are adjacently tagged as
//!   `{"t": "<variant>", "d": <payload>}` so clients can route on `t` and
//!   ignore unknown event types.
//! - **Additive evolution.** Unknown fields are tolerated on deserialize
//!   (serde default behavior — no `deny_unknown_fields` anywhere) and
//!   optional fields carry `#[serde(default)]`, so new fields and new event
//!   kinds never break older peers. [`PROTO_VERSION`] bumps only on a
//!   breaking change.
//! - **Semantic text.** All player-visible prose is [`RichText`]: spans with
//!   semantic tags (`"exit"`, `"say"`, `"error"`, …), never raw ANSI, never
//!   HTML. Renderers resolve tags to colors/CSS themselves.
//!
//! The full server→client envelope used on the WebSocket wire is
//! [`Envelope`]; the session layer stamps `seq`/`ts`, and the event fields
//! flatten in beside them: `{"seq": 4212, "ts": ..., "t": "...", "d": ...}`.

mod caps;
mod client;
mod envelope;
mod event;
mod receipt;
mod text;

pub use caps::{ClientCaps, ClientKind, ColorDepth};
pub use client::{ClientCommand, Query};
pub use envelope::Envelope;
pub use event::{
    ExitRef, InputMode, MAX_VIEW_COMMIT_BYTES, MAX_VIEW_DATA_BYTES, MAX_VIEW_ID_BYTES,
    MAX_VIEW_WIRE_INTEGER, NarrativeKind, ObjRef, OutputEvent, PanelFrameKind, PromptSpec,
    RoomView, ViewCursor, ViewEnvelope, ViewEnvelopeError, ViewFrameDisposition, WhoEntry,
};
pub use receipt::{
    ActionStatusId, ActionStatusIdError, MAX_ACTION_STATUS_ID_BYTES,
    MAX_PROJECTED_ACTION_RESULT_BYTES, MAX_PROJECTED_ACTION_SUMMARY_BYTES, ProjectedActionReceipt,
    ProjectedActionReceiptError, ProjectedActionResult, ProjectedActionResultError,
    ProjectedActionStatus,
};
pub use text::{Color, CommandLink, RichText, Span, Style};

// Re-exported so downstream crates can name span tags / feeds without taking
// their own `smol_str` dependency.
pub use smol_str::SmolStr;

/// Wire protocol version, sent in [`OutputEvent::Hello`].
///
/// Bumped **only** on a breaking change to existing shapes. Additive changes
/// (new event variants, new optional fields) do not bump this: clients are
/// required to ignore unknown `t` values and unknown fields.
pub const PROTO_VERSION: u32 = 1;
