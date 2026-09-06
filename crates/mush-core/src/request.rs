//! Server-initiated interaction — the engine lifecycle (interface plan I10
//! part 2, D-I-21).
//!
//! A module opens a request to a subject with
//! [`Ctx::open_request`](crate::cmd::Ctx::open_request); the engine mints an
//! id, delivers an [`OutputEvent::Request`](mush_proto::OutputEvent::Request)
//! to the subject, and arms a deadline. The client answers by id
//! ([`Inbound::Respond`](crate::engine::Inbound::Respond)) or the deadline
//! fires the server's default; either way the engine routes the outcome back
//! to the module's registered [`RequestCompletion`], re-validated against
//! current world state (treaty ruling 3), exactly as
//! [`AsyncCompletion`](crate::module::AsyncCompletion) routes async work.
//!
//! **Choice-only** (D-I-21): the client reports only the choice; the module
//! owns the effect. **Ephemeral / RAM-only** (owner 2026-09-05, D-I-22): a
//! request outstanding at a restart is simply lost — the wire framing is
//! advisory and best-effort, and a request that must carry durable weight is
//! minted as a [`decision`](crate::decision) instead. The open never stamps
//! idle, and neither does the answer.

use mush_proto::{RequestKind, RequestOption, RespondChoice, RichText};
use smol_str::SmolStr;

use crate::ObjId;
use crate::cmd::Ctx;
use crate::sched::TimerId;

/// The most server-initiated requests outstanding for one subject at once —
/// a RAM cap mirroring the `@wait` queue cap, so a forgetful or malicious
/// producer cannot grow the pending map without bound.
pub const MAX_OPEN_REQUESTS_PER_SUBJECT: u32 = 32;

/// How a pending request resolved, handed to the module's
/// [`RequestCompletion`].
pub enum RequestOutcome {
    /// The subject answered.
    Answered(RespondChoice),
    /// The deadline passed with no answer; the module applies its default.
    Expired,
}

/// A module's re-entry point for a server-initiated request it opened
/// (interface plan I10). Mirrors
/// [`AsyncCompletion`](crate::module::AsyncCompletion): the engine calls it
/// with a full [`Ctx`] carrying the module identity, `ctx.actor` set to the
/// subject, re-validated against current world state.
pub trait RequestCompletion: Send + Sync {
    /// Resolve one request. `payload` is this module's own JSON from
    /// `open_request`; `outcome` is the subject's choice or the deadline
    /// default. Persistence goes through [`Ctx::persist`](crate::cmd::Ctx::persist).
    fn on_respond(&self, ctx: &mut Ctx<'_>, payload: &str, outcome: RequestOutcome);
}

/// One outstanding request (engine-internal, RAM only).
pub(crate) struct PendingRequest {
    /// Who was asked (and who alone may answer).
    pub(crate) subject: ObjId,
    /// What was asked — kept so a line-mode `respond` can interpret the
    /// answer against the kind (interface plan I10).
    pub(crate) kind: RequestKind,
    /// The choice options, for mapping a line-mode `respond <n>`.
    pub(crate) options: Vec<RequestOption>,
    /// The module to route the outcome to.
    pub(crate) module: SmolStr,
    /// The module's own JSON continuation.
    pub(crate) payload: String,
    /// The deadline timer, cancelled when an answer arrives first.
    pub(crate) timer: TimerId,
}

/// What a producer hands `Ctx::open_request`, deferred as
/// [`Deferred::OpenRequest`](crate::engine::Deferred::OpenRequest) and
/// registered in the engine's `drain_deferred`.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    /// Who to ask.
    pub subject: ObjId,
    /// What the request asks for.
    pub kind: RequestKind,
    /// The prompt shown to the subject.
    pub prompt: RichText,
    /// The options for a Choice request; empty otherwise.
    pub options: Vec<RequestOption>,
    /// Milliseconds from now until the deadline default fires.
    pub timeout_ms: i64,
    /// The module the outcome routes to (must have registered a
    /// [`RequestCompletion`]).
    pub module: SmolStr,
    /// The module's own JSON continuation.
    pub payload: String,
}
