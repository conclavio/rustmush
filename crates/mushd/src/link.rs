//! The gateway<->server boundary transport (milestone M16a).
//!
//! `mushd` already has an in-process seam between the **gateway** (connection
//! I/O: the SSH/web bridges + [`mush_session::SessionManager`]) and the
//! **server** (the single-writer world task owning the [`mush_core::Engine`]).
//! M16 splits that seam across a process boundary for zero-downtime restarts;
//! this module is the SAFE FIRST STAGE (M16a): it makes the two halves speak
//! the [`mush_ipc`] protocol **in-process**, so the boundary is one
//! socket-swap away from the fork while the single binary keeps behaving
//! byte-for-byte identically.
//!
//! Two directions, two seam objects:
//!
//! - [`GatewayLink`] — **gateway → server**. The bridges hold an
//!   `Arc<dyn GatewayLink>` instead of a concrete [`WorldHandle`] and send
//!   [`ToServer`] messages / correlated [`ServiceReq`]s through it. The
//!   in-process impl ([`impl GatewayLink for WorldHandle`](WorldHandle))
//!   converts each protocol message back into a [`WorldMsg`](crate::world_task::WorldMsg)
//!   and sends it over the existing mpsc; M16b swaps in a socket client that
//!   serializes [`ToServer`] and reads [`ToGateway::Reply`](mush_ipc::ToGateway)
//!   back off the wire.
//! - [`InProcessTransport`] — **server → gateway**. The world loop's
//!   `resolve_sends` (which stays server-side, reading `World` for the
//!   M11/M14 perception gating) produces `(SendTarget, OutputEvent)` pairs —
//!   each one a [`ToGateway::Deliver`](mush_ipc::ToGateway) — and hands them
//!   here; [`InProcessTransport::transmit`] is the gateway-side consumer that
//!   maps each [`SendTarget`] onto the [`SessionManager`]. M16b replaces this
//!   with the socket writer that serializes [`ToGateway`](mush_ipc::ToGateway)
//!   frames and moves slow-client detection to the gateway side.
//!
//! The in-process delivery batch is carried as `(SendTarget, Arc<OutputEvent>)`
//! rather than owned [`ToGateway::Deliver`](mush_ipc::ToGateway) values on
//! purpose: a `Room` audience fans one `Arc` out to every recipient with no
//! per-recipient clone, so the hot path keeps its zero-copy behavior (invariant
//! S1: no perf regression). That pair *is* the content of a `ToGateway::Deliver`
//! stream, minus the MessagePack framing M16b will add — and the codec that
//! carries the real bytes is already proven by `mush-ipc`'s round-trip tests.

use std::sync::Arc;

use futures::future::BoxFuture;
use mush_core::Inbound;
use mush_ipc::{SendTarget, ServiceReq, ServiceResp, ToServer};
use mush_proto::OutputEvent;
use mush_session::{CharKey, ConnId, SessionManager};

use crate::world_task::WorldHandle;

/// The gateway's protocol-typed handle to the server half.
///
/// Everything the gateway sends across the boundary goes through this trait,
/// so the bridges name only the [`mush_ipc`] vocabulary — never the in-process
/// [`WorldHandle`] / [`WorldMsg`](crate::world_task::WorldMsg) plumbing. The
/// one in-process implementor is [`WorldHandle`]; M16b adds a socket-client
/// implementor that talks to the forked server process.
///
/// Object-safe (returns [`BoxFuture`], the workspace's `ObjectStore` idiom)
/// so the bridges can hold an `Arc<dyn GatewayLink>`.
pub trait GatewayLink: Send + Sync {
    /// Send one one-way [`ToServer`] message — a live-input [`WireInbound`]
    /// arm ([`ToServer::Inbound`]) or operator [`ToServer::Control`]. Resolves
    /// to `false` when the server is gone (shutdown in progress).
    ///
    /// [`WireInbound`]: mush_ipc::WireInbound
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool>;

    /// Issue a correlated service request ([`ServiceReq`]) and await its
    /// [`ServiceResp`]. Resolves to `None` only when the request could not be
    /// delivered at all; per-request failure (e.g. player-creation refusal) is
    /// carried inside the returned [`ServiceResp`].
    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>>;
}

/// The in-process [`GatewayLink`]: converts each protocol message back into a
/// [`WorldMsg`](crate::world_task::WorldMsg) and sends it over the existing
/// world-task mpsc, preserving byte-for-byte behavior. The correlated service
/// requests reuse [`WorldHandle`]'s oneshot round-trips (id correlation is the
/// transport's job in M16b; in-process the oneshot *is* the correlation).
impl GatewayLink for WorldHandle {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match msg {
                // The cross-boundary input subset: convert back to the shared
                // `Inbound` and queue it exactly as the direct call did.
                ToServer::Inbound(wire) => self.inbound(Inbound::from(wire)).await,
                // Operator control (`@shutdown`, `@dump`) crosses as the
                // top-level `ToServer::Control`; in-process it re-enters the
                // world loop as `Inbound::Control`, the same message the
                // console path sends.
                ToServer::Control(ctrl) => self.inbound(Inbound::Control(ctrl)).await,
                // Session bookkeeping (Attach/Detach/ReAnnounce) exists for the
                // standalone gateway of M16b/M16c to rebuild the server's
                // connection map after a restart. In-process the
                // `SessionManager` is shared, so the server already learns of
                // every attach/detach through `ConnUp`/`ConnDown`; these arms
                // are therefore never sent on the in-process path. Accept and
                // ignore so an early sender is a no-op, not a panic.
                ToServer::Attach { .. } | ToServer::Detach { .. } | ToServer::ReAnnounce(_) => true,
                // A correlated request must go through `request` (it needs a
                // reply); routing one here is a programming error.
                ToServer::Request { .. } => {
                    debug_assert!(false, "ToServer::Request must be sent via GatewayLink::request");
                    true
                }
            }
        })
    }

    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        Box::pin(async move {
            Some(match req {
                ServiceReq::CreatePlayer { name } => {
                    ServiceResp::CreatePlayer(self.create_player(&name).await.map_err(Into::into))
                }
                ServiceReq::NameFree { name } => ServiceResp::NameFree(self.name_free(&name).await),
                ServiceReq::InInputContext { actor } => {
                    ServiceResp::InInputContext(self.in_input_context(actor).await)
                }
                ServiceReq::Barrier => {
                    self.barrier().await;
                    ServiceResp::Barrier
                }
            })
        })
    }
}

/// The in-process server → gateway delivery transport.
///
/// Holds the shared [`SessionManager`]; [`transmit`](InProcessTransport::transmit)
/// is the gateway-side consumer of the [`ToGateway::Deliver`](mush_ipc::ToGateway)
/// payloads the world loop's `resolve_sends` produced. This is the swap point
/// for M16b: the standalone gateway replaces this with a socket reader that
/// decodes `ToGateway` frames and runs the same [`SendTarget`] → session
/// routing on its own `SessionManager`.
#[derive(Clone)]
pub struct InProcessTransport {
    sessions: Arc<SessionManager>,
}

impl InProcessTransport {
    /// Wrap the shared session manager as the in-process gateway.
    #[must_use]
    pub fn new(sessions: Arc<SessionManager>) -> InProcessTransport {
        InProcessTransport { sessions }
    }

    /// Route a resolved delivery batch onto the session manager, returning the
    /// connections whose output buffer overflowed (slow clients) for the
    /// caller to police.
    ///
    /// Each `(SendTarget, event)` pair is one
    /// [`ToGateway::Deliver`](mush_ipc::ToGateway); world-side audience
    /// resolution (room membership, `@ignore`, M11/M14 perception gating)
    /// already ran in `resolve_sends`, so this only maps the [`SendTarget`]
    /// back onto the session layer (character fan-out, single connection, or
    /// global broadcast). The batch carries `Arc<OutputEvent>` so a `Room`
    /// event resolved once is delivered to every recipient without a clone.
    pub fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) -> Vec<ConnId> {
        let mut slow = Vec::new();
        for (target, ev) in sends {
            match target {
                SendTarget::Char(ch) => {
                    let report = self.sessions.send_to_char(CharKey(ch), ev);
                    if report.delivered == 0 && report.dropped.is_empty() {
                        // Link-dead actor (or a race with a disconnect): the
                        // event is lost. Resume buffers are an M2+ concern
                        // (PLAN: mush-session "resume buffers"); make the loss
                        // observable at least.
                        tracing::debug!(
                            char = ch,
                            "output to actor with no live connections dropped"
                        );
                    }
                    slow.extend(report.dropped);
                }
                SendTarget::Conn(conn) => {
                    // Pre-auth / connection-addressed events. An unknown conn is
                    // fine (it may have raced a disconnect).
                    let _ = self.sessions.send_to_conn(ConnId(conn), ev);
                }
                SendTarget::All => {
                    let report = self.sessions.broadcast_all(ev);
                    slow.extend(report.dropped);
                }
            }
        }
        slow.sort_unstable();
        slow.dedup();
        slow
    }

    /// Route one out-of-band delivery through the seam — a single
    /// [`ToGateway::Deliver`](mush_ipc::ToGateway).
    ///
    /// The world loop's audience-resolved output goes through
    /// [`transmit`](InProcessTransport::transmit); this is the path for the
    /// server's *unaddressed* one-off sends (read-only refusals, wizard
    /// store-health broadcasts, staff-op confirmations) that are not driven by
    /// an `Audience` but still cross the boundary as `Deliver`. Slow-client
    /// policing is intentionally skipped for these handful-of-lines sends (the
    /// session layer still counts any drops), matching their prior direct
    /// `send_to_char`/`send_to_conn` behavior.
    pub fn deliver(&self, target: SendTarget, event: Arc<OutputEvent>) {
        let _ = self.transmit(vec![(target, event)]);
    }

    /// Apply a [`ToGateway::SetFloodExempt`](mush_ipc::ToGateway): set or clear
    /// a character's flood-exempt flag on the gateway's throttle.
    ///
    /// The world loop drives this after each `Cmd` so a paste into an input
    /// context bypasses the burst cap. In-process it is the same
    /// [`SessionManager::set_flood_exempt`] call the world loop used to make
    /// directly; M16b serializes the arm across the socket instead.
    pub fn set_flood_exempt(&self, char: u64, exempt: bool) {
        self.sessions.set_flood_exempt(CharKey(char), exempt);
    }
}
