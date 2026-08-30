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
use mush_ipc::{GatewayLink, ServiceReq, ServiceResp, ToServer};

use crate::world_task::WorldHandle;

impl GatewayLink for WorldHandle {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match msg {
                // The cross-boundary input subset: convert back to the shared
                // `Inbound` and queue it exactly as the direct call did.
                ToServer::Inbound(wire) => self.inbound(Inbound::from(wire)).await,
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
