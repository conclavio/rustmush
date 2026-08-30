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

use crate::{ServiceReq, ServiceResp, ToServer};

/// The gateway's protocol-typed handle to the server half.
///
/// Object-safe (returns [`BoxFuture`], the workspace's `dyn`-async idiom) so the
/// bridges can hold an `Arc<dyn GatewayLink>`.
pub trait GatewayLink: Send + Sync {
    /// Send one one-way [`ToServer`] message — a live-input
    /// [`WireInbound`](crate::WireInbound) arm ([`ToServer::Inbound`]) or
    /// operator [`ToServer::Control`]. Resolves to `false` when the server is
    /// gone (shutdown in progress).
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool>;

    /// Issue a correlated service request ([`ServiceReq`]) and await its
    /// [`ServiceResp`]. Resolves to `None` only when the request could not be
    /// delivered at all; per-request failure (e.g. player-creation refusal) is
    /// carried inside the returned [`ServiceResp`].
    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>>;
}
