//! The gateway → server half of the boundary seam: the [`GatewayLink`] trait.
//!
//! The bridges (gateway side) hold an `Arc<dyn GatewayLink>` and speak only the
//! `mush-ipc` vocabulary through it — never the in-process `WorldHandle` /
//! `WorldMsg` plumbing. Two implementors exist:
//!
//! - `WorldHandle` (server side): converts each protocol message back into the
//!   world task's mpsc traffic — the all-in-one, byte-for-byte path.
//! - the socket client (gateway side): serializes [`ToServer`] onto the boundary
//!   socket and correlates [`ServiceReq`]/[`ServiceResp`] pairs off the wire.
//!
//! The trait lives here (not in either half) so `impl GatewayLink for
//! WorldHandle` can live in `mush-server` without `mush-server` and
//! `mush-gateway` depending on each other.

use futures::future::BoxFuture;

use crate::{GatewayAction, ObjId, ServiceReq, ServiceResp, ToServer};

/// The outcome of an input-path (`Cmd`/`Query`) send through [`GatewayLink`]
/// ([`to_server_input`](GatewayLink::to_server_input)). Distinguishes the M18
/// gateway->server input-shed from a boundary-gone failure so the input bridge
/// can surface the right player-facing signal (a "server busy" notice on a shed
/// `Cmd`; silence on a shed `Query`; the "restarting" notice on a gap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSend {
    /// Enqueued to the server.
    Sent,
    /// Dropped by the M18 input-shed: the gateway->server (`to_server`) queue
    /// was at/over its cap (a wedged or slow server). Only live input
    /// (`Cmd`/`Query`) is ever shed. A new connection may separately be refused
    /// *before* lifecycle custody is reserved; after reservation, its
    /// must-deliver frames always enqueue.
    Shed,
    /// The boundary is gone — a restart gap
    /// ([`is_restarting`](GatewayLink::is_restarting)) or a terminal shutdown.
    Gone,
}

/// Admission result for one semantic legal-action submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionSend {
    /// The deployment accepted custody. Split mode reports this when gateway
    /// replay custody is installed; all-in-one reports it after exact intent
    /// ownership. Any later split-mode semantic refusal is delivered through
    /// the ordinary request-disposition error path.
    Accepted,
    /// Bounded input/coordinator custody is full before durable ownership.
    Busy,
    /// The boundary/world or authoritative store is unavailable.
    Gone,
    /// The offer or authenticated actor/connection binding was rejected
    /// before durable ownership.
    Rejected,
}

/// Result of reserving split-gateway lifecycle custody before local session
/// mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionAdmission {
    /// A bounded slot is reserved; the matching `ConnUp` is now must-deliver.
    Reserved,
    /// The boundary is live but its queue or lifecycle ledger is at capacity.
    Busy,
    /// No server boundary exists (restart gap or shutdown).
    Gone,
}

/// The gateway's protocol-typed handle to the server half.
///
/// Object-safe (returns [`BoxFuture`], the workspace's `dyn`-async idiom) so the
/// bridges can hold an `Arc<dyn GatewayLink>`.
pub trait GatewayLink: Send + Sync {
    /// Reserve bounded connection-lifecycle custody before the caller mutates
    /// its local session table.
    ///
    /// The split gateway uses this admission point to cap live connections plus
    /// pending link-dead cleanup and to stop taking new custody when its IPC
    /// writer is backed up. The all-in-one implementation uses the permissive
    /// default because its world channel supplies its own backpressure. A
    /// successful reservation must be followed promptly by a matching
    /// `ConnUp`, or cancelled with [`cancel_connection_reservation`](Self::cancel_connection_reservation).
    fn try_reserve_connection(&self, _conn: u64, _actor: ObjId) -> ConnectionAdmission {
        ConnectionAdmission::Reserved
    }

    /// Cancel a successful lifecycle reservation which never produced a
    /// `ConnUp`. Implementations which do not reserve custody may ignore it.
    fn cancel_connection_reservation(&self, _conn: u64) {}

    /// Send one one-way [`ToServer`] message — a live-input
    /// [`WireInbound`](crate::WireInbound) arm ([`ToServer::Inbound`]) or
    /// operator [`ToServer::Control`]. Resolves to `false` when the server is
    /// gone (shutdown in progress).
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool>;

    /// Send one **input-path** [`ToServer`] (`Cmd`/`Query`), reporting whether it
    /// was enqueued, SHED by the M18 input-shed (a full `to_server` queue), or
    /// dropped because the boundary is GONE. Only the split gateway's socket link
    /// ever sheds; the in-process all-in-one link maps its plain
    /// [`to_server`](GatewayLink::to_server) bool onto [`InputSend::Sent`] /
    /// [`InputSend::Gone`] and never returns [`InputSend::Shed`] (this default).
    fn to_server_input(&self, msg: ToServer) -> BoxFuture<'_, InputSend> {
        Box::pin(async move {
            if self.to_server(msg).await {
                InputSend::Sent
            } else {
                InputSend::Gone
            }
        })
    }

    /// Submit one opaque legal action through the deployment's replay-safe
    /// coordinator ingress. Implementations must allocate the request identity
    /// themselves; public callers never construct [`crate::GatewayRequest`].
    fn submit_action(&self, _action: GatewayAction) -> BoxFuture<'_, ActionSend> {
        Box::pin(async move { ActionSend::Gone })
    }

    /// Issue a correlated service request ([`ServiceReq`]) and await its
    /// [`ServiceResp`]. Resolves to `None` only when the request could not be
    /// delivered at all; per-request failure (e.g. player-creation refusal) is
    /// carried inside the returned [`ServiceResp`]. Bounded split-process
    /// admission returns [`ServiceResp::Busy`] before taking custody when its
    /// correlated-request ceiling is full.
    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>>;

    /// Whether the boundary is currently in a **restart gap** — the server is
    /// down and the gateway is respawning + re-attaching it (M16c). The
    /// in-process all-in-one link is never in this state (the default), so its
    /// behaviour is unchanged; the split gateway's socket link reports `true`
    /// between a server exit and the re-synced reconnect, letting the input path
    /// surface a clear "restarting" notice instead of silently dropping the line
    /// (a `to_server` during the gap resolves `false`, as at shutdown).
    fn is_restarting(&self) -> bool {
        false
    }
}
