//! The in-process [`GatewayLink`] implementation for [`WorldHandle`].
//!
//! Converts each protocol message back into the world task's mpsc traffic,
//! preserving byte-for-byte behavior. The all-in-one binary hands the bridges an
//! `Arc<dyn GatewayLink>` backed by this impl; the forked gateway uses a socket
//! client instead. The impl lives here (server side) — not in `mush-gateway` —
//! because [`WorldHandle`] is a server type; the [`GatewayLink`] trait lives in
//! `mush-ipc` so neither half depends on the other.

use futures::future::BoxFuture;
use mush_core::Inbound;
use mush_ipc::{ActionSend, GatewayAction, GatewayLink, ServiceReq, ServiceResp, ToServer};

use crate::world_task::WorldHandle;

impl GatewayLink for WorldHandle {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match msg {
                // The cross-boundary input subset: convert back to the shared
                // `Inbound` and queue it exactly as the direct call did.
                ToServer::Inbound(wire) => self.inbound(Inbound::from(wire)).await,
                // Split-mode custody frames are produced only by
                // `SocketGatewayLink`; the all-in-one bridge sends raw inbound
                // directly. Accept a manually supplied request as a direct
                // in-process delivery, while a release has no retained claim.
                // An authoritative action has no `Inbound` form and never
                // reaches the world this way: it is refused, not enqueued.
                ToServer::Input(request) => match Inbound::try_from(request.input) {
                    Ok(inbound) => self.inbound(inbound).await,
                    Err(_) => {
                        debug_assert!(
                            false,
                            "GatewayInput::Action must go through the action coordinator"
                        );
                        false
                    }
                },
                ToServer::GatewayReleased { .. } => true,
                // Operator control (`@shutdown`, `@dump`) crosses as the
                // top-level `ToServer::Control`; in-process it re-enters the
                // world loop as `Inbound::Control`, the same message the console
                // path sends.
                ToServer::Control(ctrl) => self.inbound(Inbound::Control(ctrl)).await,
                // Session bookkeeping (Attach/Detach/ReAnnounce) exists for the
                // standalone gateway to rebuild the server's connection map after
                // a restart. In-process the `SessionManager` is shared, so the
                // server already learns of every attach/detach through
                // `ConnUp`/`ConnDown`; these arms are never sent on the
                // in-process path. Accept and ignore so an early sender is a
                // no-op, not a panic.
                ToServer::Attach { .. } | ToServer::Detach { .. } | ToServer::ReAnnounce(_) => true,
                // A correlated request must go through `request` (it needs a
                // reply); routing one here is a programming error.
                ToServer::Request { .. } => {
                    debug_assert!(
                        false,
                        "ToServer::Request must be sent via GatewayLink::request"
                    );
                    true
                }
            }
        })
    }

    /// The dispositions this link answers with are NOT restated here. Both
    /// deployments call the one table in [`crate::handle_request`], so the
    /// forked server and the all-in-one binary cannot drift on what a dead or
    /// commit-pending world says about a name, a readiness probe or a barrier
    /// — and a new [`ServiceReq`] variant fails to compile in exactly one
    /// place instead of being silently answered on one side only.
    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        Box::pin(async move { Some(crate::handle_request(self, req).await) })
    }

    fn submit_action(&self, action: GatewayAction) -> BoxFuture<'_, ActionSend> {
        Box::pin(async move { self.submit_legal_action(action).await })
    }
}
