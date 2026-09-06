//! # mush-gateway — the forked GATEWAY half (milestone M16b)
//!
//! The connection-owning half of the fork: the SSH (and, later, web) listeners,
//! the [`SessionManager`](mush_session::SessionManager), account auth + the IP
//! guard, and the SUPERVISOR that spawns the [`mush-server`] child. It speaks
//! the [`mush_ipc`] boundary protocol over a socket and holds none of the world
//! / object-store / module code.
//!
//! Two seam objects mirror `mushd`'s in-process pair, but over a socket:
//!
//! - [`SocketGatewayLink`] implements `mushd`'s `GatewayLink` (gateway →
//!   server): one-way [`ToServer`] messages go straight onto the writer queue,
//!   and correlated [`ServiceReq`]s allocate an id, park a oneshot, and await
//!   the matching [`ToGateway::Reply`] the reader routes back. The world loop is
//!   never on the awaiting side (D4) — the gateway task awaits.
//! - the **reader task** ([`gateway_reader`]) decodes [`ToGateway`] frames and
//!   runs the same `SendTarget` → session routing `mushd`'s `InProcessTransport`
//!   does, plus the fire-and-forget session-control arms (`SetFloodExempt`,
//!   `CloseSession`, `Detach`) — resolving each locally against *its*
//!   `SessionManager` and feeding the resulting `ConnDown` back to the server
//!   as inbound.
//!
//! [`mush-server`]: https://docs.rs/mush-server
//! [`ToServer`]: mush_ipc::ToServer
//! [`ToGateway`]: mush_ipc::ToGateway
//! [`ServiceReq`]: mush_ipc::ServiceReq

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::DisconnectKind;
use mush_ipc::{
    ActionSend, AttachRec, BoxRead, BoxWrite, ConnectionAdmission, DetachCause, FrameReader,
    GatewayAction, GatewayInput, GatewayLink, GatewayRequest, GatewayRequestDisposition,
    IPC_VERSION, InputSend, IpcEndpoint, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, RequestUid, ServiceReq, ServiceResp, ToGateway, ToServer,
    WireInbound,
};
use mush_net_web::{WebConfig, WebHooks};
use mush_proto::{OutputEvent, RichText};
use mush_session::{CharKey, DetachOutcome, DetachReason, SessionConfig, SessionManager};
use mush_store::ObjectStore;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::accounts::Accounts;
use crate::delivery::{SessionDelivery, char_key, char_key_obj};
use crate::ssh_bridge::{SshBridge, start_listener};
use crate::web_bridge::WebBridge;

pub mod accounts;
pub mod bridge_common;
pub mod composing;
pub mod delivery;
pub mod discord_bridge;
pub mod ssh_bridge;
pub mod web_bridge;

/// Where the server half comes from.
pub enum ServerSource {
    /// Production: spawn + SUPERVISE a `mush-server` child process (D1). The
    /// gateway launches the child, connects to the `[ipc].listen` endpoint it
    /// binds, and on the child's exit (crash or deliberate restart) respawns it
    /// — holding the player connections and re-attaching them (M16c).
    Spawn {
        /// The config path to pass the child (`--config`).
        config_path: PathBuf,
    },
    /// Test / attach: connect to an already-running server at this endpoint
    /// ONCE, spawning nothing and NOT supervising it (the M16b two-process
    /// integration test runs the server as a sibling thread). The gateway does
    /// not restart this server — it serves until Ctrl-C.
    Connect(IpcEndpoint),
    /// Supervised + restartable: the gateway (re)launches the server through a
    /// [`ServerLauncher`] and re-attaches held sessions across each restart
    /// (M16c). Used by the restart-survival test (which launches the server as
    /// a sibling thread on an ephemeral endpoint) and by any embedding that
    /// wants the gateway to own the server lifecycle in-process.
    Restartable(Box<dyn ServerLauncher>),
}

/// Launches a fresh server instance for the supervisor (D1). Each call boots a
/// new server half and returns its live IPC endpoint plus a future that
/// resolves once that instance has fully exited (so the supervisor knows the
/// listen socket is released before it reconnects).
pub trait ServerLauncher: Send + Sync {
    /// Boot one server instance.
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>>;
}

/// A freshly launched server instance the supervisor is watching (D1).
pub struct LaunchedServer {
    /// The IPC endpoint this instance is listening on.
    pub endpoint: IpcEndpoint,
    /// Resolves once this instance has fully exited (child reaped / server
    /// thread returned). Awaited during a restart so the old listen socket is
    /// released before the supervisor reconnects.
    pub exited: BoxFuture<'static, ()>,
}

/// A programmatic restart trigger (D1b): a deliberate operator restart of the
/// supervised server child, driven by a `SIGHUP` handler in production and by
/// the restart-survival test directly. Cloneable; every clone signals the same
/// supervisor. A no-op when the server is not supervised
/// ([`ServerSource::Connect`]).
#[derive(Clone)]
pub struct RestartTrigger {
    tx: mpsc::Sender<()>,
}

impl RestartTrigger {
    /// Request a deliberate server restart. Returns `false` if the supervisor is
    /// gone (gateway shutting down).
    pub async fn restart(&self) -> bool {
        self.tx.send(()).await.is_ok()
    }

    /// Non-blocking restart request (for signal handlers): `false` if the
    /// supervisor is gone or a restart is already queued.
    pub fn try_restart(&self) -> bool {
        self.tx.try_send(()).is_ok()
    }
}

/// Knobs for [`serve`].
pub struct GatewayOptions {
    /// How to obtain the server half.
    pub source: ServerSource,
    /// The gateway's Redis KV handle (accounts/tokens/bans). Tests inject a
    /// seeded in-memory store; production opens Redis from config.
    pub store: Option<Arc<dyn ObjectStore>>,
    /// Signalled with the bound SSH listen address once it is up (the test's
    /// SSH client connects there).
    pub ssh_ready: Option<oneshot::Sender<SocketAddr>>,
    /// Signalled with the bound WEB listen address once the web listener is up
    /// (the test's WS client connects there). `None` for callers that do not
    /// need it (production reads the configured address). A web bind FAILURE
    /// (W4) leaves this unsent — the sender drops and its receiver resolves to
    /// `Err`, exactly as a never-bound listener would.
    pub web_ready: Option<oneshot::Sender<SocketAddr>>,
    /// Signalled with a [`RestartTrigger`] once the supervisor is running (D1b).
    /// The restart-survival test holds the trigger to drive a deliberate
    /// restart; production wires it to a `SIGHUP` handler. `None` for callers
    /// that do not need to trigger restarts (or non-supervised
    /// [`ServerSource::Connect`], where it never fires).
    pub restart_ready: Option<oneshot::Sender<RestartTrigger>>,
}

/// The bounded shared table of parked service-request waiters, plus a `down`
/// flag the reader raises when the boundary tears down.
///
/// The `down` flag is what makes a mid-request boundary loss fail *fast* instead
/// of hanging forever (F3): the reader ([`gateway_reader`]) sets it and drops
/// every parked sender under the lock when the server EOFs / errors / says Bye,
/// and [`request`](SocketGatewayLink::request) checks it under the *same* lock
/// before parking — so a request can never register into a table that has
/// already been drained and then await a reply that will never come. Both
/// resolve to `None`, which the bridges map to the WorldGone rejection, exactly
/// as the in-process path resolves when the world task drops its oneshot.
/// Admission also happens under this lock, so the table can never exceed
/// [`MAX_IN_FLIGHT_SERVICE_REQUESTS`]; a request above that ceiling gets a
/// typed [`ServiceResp::Busy`] before any must-deliver frame is enqueued.
struct Pending {
    waiters: HashMap<u64, oneshot::Sender<ServiceResp>>,
    /// Raised once the boundary is gone; no further waiters may park.
    down: bool,
    /// Edge latch: an abusive caller at the custody ceiling must not turn the
    /// overload warning itself into an unbounded log flood.
    capacity_warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAdmission {
    Registered,
    Down,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecyclePhase {
    /// Capacity has been reserved, but the local session attach and its
    /// `ConnUp` have not completed yet.
    Reserved,
    /// The matching `ConnUp` was accepted into the writer FIFO.
    Live,
}

#[derive(Clone, Copy, Debug)]
struct ConnectionCustody {
    actor: mush_core::ObjId,
    phase: LifecyclePhase,
    /// Reserving a reconnect converts an existing link-dead obligation into a
    /// connection obligation. If admission is cancelled before `ConnUp`, put
    /// that obligation back rather than silently forgetting its cleanup.
    restore_linkdead_on_cancel: bool,
}

/// Restart-surviving, hard-bounded lifecycle custody.
///
/// A slot is held by each reserved/live physical connection. When the last
/// connection for an actor is lost, its slot is converted (not released) into
/// a pending-linkdead obligation until `SessionGone` is posted. This prevents
/// a churner from accumulating arbitrarily many delayed reapers while the IPC
/// writer is healthy and then releasing all of them into a stalled FIFO.
struct LifecycleCustody {
    connections: HashMap<u64, ConnectionCustody>,
    pending_linkdead: HashSet<mush_core::ObjId>,
    admission_warned: bool,
    cap: usize,
}

impl LifecycleCustody {
    fn new(queue_cap: usize) -> LifecycleCustody {
        LifecycleCustody {
            connections: HashMap::new(),
            pending_linkdead: HashSet::new(),
            admission_warned: false,
            cap: queue_cap.clamp(1, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS),
        }
    }

    fn obligations(&self) -> usize {
        self.connections.len() + self.pending_linkdead.len()
    }

    fn cancel_reserved(&mut self, conn: u64) {
        let Some(custody) = self.connections.get(&conn).copied() else {
            return;
        };
        if custody.phase != LifecyclePhase::Reserved {
            return;
        }
        self.connections.remove(&conn);
        if custody.restore_linkdead_on_cancel {
            let actor_is_live = self
                .connections
                .values()
                .any(|other| other.actor == custody.actor && other.phase == LifecyclePhase::Live);
            if !actor_is_live {
                if let Some((_, replacement)) = self.connections.iter_mut().find(|(_, other)| {
                    other.actor == custody.actor && other.phase == LifecyclePhase::Reserved
                }) {
                    // Preserve exactly one restoration responsibility across
                    // concurrent reconnect attempts. If this replacement is
                    // cancelled too, the original link-dead obligation comes
                    // back; if it reaches ConnUp, the obligation is satisfied.
                    replacement.restore_linkdead_on_cancel = true;
                } else {
                    self.pending_linkdead.insert(custody.actor);
                }
            }
        }
        if self.obligations() < self.cap {
            self.admission_warned = false;
        }
    }

    fn observe_conn_down(&mut self, conn: u64, actor: mush_core::ObjId, reason: DisconnectKind) {
        if let Some(custody) = self.connections.remove(&conn) {
            if custody.actor != actor {
                tracing::error!(
                    conn,
                    expected_actor = %custody.actor,
                    actual_actor = %actor,
                    "ConnDown actor disagrees with lifecycle custody"
                );
            }
            let actor_still_live = self
                .connections
                .values()
                .any(|other| other.actor == custody.actor);
            if reason == DisconnectKind::Lost && !actor_still_live {
                // Convert the last lost connection's slot into the delayed
                // SessionGone obligation; do not release capacity yet.
                self.pending_linkdead.insert(custody.actor);
            }
        }
        if self.obligations() < self.cap {
            self.admission_warned = false;
        }
    }

    fn observe_session_gone(&mut self, actor: mush_core::ObjId) -> Vec<u64> {
        self.pending_linkdead.remove(&actor);
        let cancelled: Vec<u64> = self
            .connections
            .iter()
            .filter_map(|(&conn, custody)| {
                (custody.actor == actor && custody.phase == LifecyclePhase::Reserved)
                    .then_some(conn)
            })
            .collect();
        for conn in &cancelled {
            self.connections.remove(conn);
        }
        if self.obligations() < self.cap {
            self.admission_warned = false;
        }
        cancelled
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ConnectionInputCustody {
    count: usize,
    bytes: usize,
}

#[derive(Clone, Debug)]
struct CustodiedInput {
    request: GatewayRequest,
    bytes: usize,
}

/// Gateway-process-owned replay pool for player input.
///
/// Entries survive a server restart and remain until an explicit server
/// disposition transfers/releases ownership. The `BTreeMap` follows the
/// epoch/sequence UID ordering, so a reconnect replays the exact accepted FIFO
/// after the attachment barrier. Both global and per-connection count/byte
/// ceilings are enforced before a frame enters the writer queue.
#[derive(Default)]
struct InputCustody {
    entries: BTreeMap<RequestUid, CustodiedInput>,
    per_connection: HashMap<u64, ConnectionInputCustody>,
    bytes: usize,
    capacity_warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputCustodyAdmission {
    Admitted,
    Full { warn: bool },
    Invalid,
}

impl InputCustody {
    fn admit(&mut self, request: GatewayRequest) -> InputCustodyAdmission {
        if !request.has_valid_uid() || self.entries.contains_key(&request.uid) {
            return InputCustodyAdmission::Invalid;
        }
        let Ok(bytes) = request.input.logical_bytes() else {
            return InputCustodyAdmission::Invalid;
        };
        let conn = request.input.conn();
        let account = self.per_connection.get(&conn).copied().unwrap_or_default();
        let full = self.entries.len() >= MAX_IN_FLIGHT_GATEWAY_INPUTS
            || self.bytes.saturating_add(bytes) > MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES
            || account.count >= MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION
            || account.bytes.saturating_add(bytes)
                > MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION;
        if full {
            let warn = !self.capacity_warned;
            self.capacity_warned = true;
            return InputCustodyAdmission::Full { warn };
        }

        self.entries
            .insert(request.uid, CustodiedInput { request, bytes });
        self.bytes += bytes;
        let account = self.per_connection.entry(conn).or_default();
        account.count += 1;
        account.bytes += bytes;
        self.capacity_warned = false;
        InputCustodyAdmission::Admitted
    }

    fn remove(&mut self, uid: RequestUid) -> Option<GatewayRequest> {
        let held = self.entries.remove(&uid)?;
        let conn = held.request.input.conn();
        self.bytes = self.bytes.saturating_sub(held.bytes);
        if let Some(account) = self.per_connection.get_mut(&conn) {
            account.count = account.count.saturating_sub(1);
            account.bytes = account.bytes.saturating_sub(held.bytes);
            if account.count == 0 {
                self.per_connection.remove(&conn);
            }
        }
        if self.entries.len() < MAX_IN_FLIGHT_GATEWAY_INPUTS
            && self.bytes < MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES
        {
            self.capacity_warned = false;
        }
        Some(held.request)
    }

    fn snapshot(&self) -> Vec<GatewayRequest> {
        self.entries
            .values()
            .map(|held| held.request.clone())
            .collect()
    }
}

/// A `to_server` sender paired with the shared external depth gauge (M18). The
/// channel stays UNBOUNDED (a single FIFO the must-deliver frames can never be
/// blocked on — `ConnUp` must precede a conn's `Cmd`s), but every enqueue bumps
/// `depth` and [`gateway_writer`] decrements it after each successful
/// `write_frame`, so `depth` tracks the queue's TRUE occupancy. The
/// [`SocketGatewayLink`] input-shed reads it to decide whether to drop a
/// `Cmd`/`Query`; every send also records the `to_server` high-water gauge with
/// an edge-gated warn. Cloned into every producer (the link, the delivery
/// transport, the reader's session-control helpers) so they all bump the SAME
/// gauge.
#[derive(Clone)]
pub(crate) struct ToServerTx {
    tx: mpsc::UnboundedSender<ToServer>,
    /// Shared external depth gauge: `+1` per enqueue here, `-1` per successful
    /// write in [`gateway_writer`].
    depth: Arc<AtomicU64>,
    /// The world loop's registry (records the `to_server` high-water).
    metrics: Arc<Metrics>,
    /// Edge latch for the high-water warn (per-crossing, never per-push).
    hw_warned: Arc<AtomicBool>,
    /// `[ipc].to_server_cap` — the high-water threshold base.
    cap: usize,
    /// `[ipc].high_water_warn_pct` — percent of `cap` at which a crossing warns.
    high_water_pct: u8,
    /// Shared across boundary restarts. Every producer (including reverse
    /// session-control handling) observes the same lifecycle reservations.
    lifecycle: Arc<Mutex<LifecycleCustody>>,
}

impl ToServerTx {
    /// Wrap a fresh boundary's `to_server` sender with a zeroed depth gauge.
    fn new(
        tx: mpsc::UnboundedSender<ToServer>,
        depth: Arc<AtomicU64>,
        metrics: Arc<Metrics>,
        cap: usize,
        high_water_pct: u8,
        lifecycle: Arc<Mutex<LifecycleCustody>>,
    ) -> ToServerTx {
        ToServerTx {
            tx,
            depth,
            metrics,
            hw_warned: Arc::new(AtomicBool::new(false)),
            cap,
            high_water_pct,
            lifecycle,
        }
    }

    /// The current queue occupancy (frames enqueued but not yet written).
    fn depth(&self) -> u64 {
        self.depth.load(Ordering::Relaxed)
    }

    /// Reserve lifecycle capacity before a caller attaches the connection to
    /// its local session table. No frame exists yet, so a refusal here sheds no
    /// must-deliver work.
    fn try_reserve_connection(&self, conn: u64, actor: mush_core::ObjId) -> ConnectionAdmission {
        let queue_cap = self.cap.max(1);
        let queue_depth = self.depth();
        let mut lifecycle = self.lifecycle.lock().expect("lifecycle custody poisoned");

        if lifecycle.connections.contains_key(&conn) {
            tracing::error!(conn, %actor, "duplicate lifecycle reservation refused");
            return ConnectionAdmission::Busy;
        }

        let reuses_linkdead = lifecycle.pending_linkdead.contains(&actor);
        let at_custody_cap = !reuses_linkdead && lifecycle.obligations() >= lifecycle.cap;
        let at_queue_cap = queue_depth >= queue_cap as u64;
        if at_custody_cap || at_queue_cap {
            if !lifecycle.admission_warned {
                tracing::warn!(
                    conn,
                    %actor,
                    queue_len = queue_depth,
                    queue_cap,
                    lifecycle_obligations = lifecycle.obligations(),
                    lifecycle_cap = lifecycle.cap,
                    "connection admission refused before lifecycle custody"
                );
                lifecycle.admission_warned = true;
            }
            return ConnectionAdmission::Busy;
        }

        if reuses_linkdead {
            lifecycle.pending_linkdead.remove(&actor);
        }
        lifecycle.connections.insert(
            conn,
            ConnectionCustody {
                actor,
                phase: LifecyclePhase::Reserved,
                restore_linkdead_on_cancel: reuses_linkdead,
            },
        );
        lifecycle.admission_warned = false;
        ConnectionAdmission::Reserved
    }

    /// Cancel a reservation that never reached `ConnUp`.
    fn cancel_connection_reservation(&self, conn: u64) {
        self.lifecycle
            .lock()
            .expect("lifecycle custody poisoned")
            .cancel_reserved(conn);
    }

    /// Enqueue one frame after applying lifecycle bookkeeping. New `ConnUp`
    /// frames require a prior reservation; once reserved they are never shed,
    /// even if other must-deliver traffic fills the queue before the attach is
    /// posted. Teardown converts/releases that bounded custody while preserving
    /// the one FIFO's `ConnUp`-before-input ordering.
    pub(crate) fn send(&self, msg: ToServer) -> bool {
        if let ToServer::Inbound(WireInbound::ConnUp { conn, actor, .. }) = &msg {
            let conn = *conn;
            let actor = *actor;
            let mut lifecycle = self.lifecycle.lock().expect("lifecycle custody poisoned");
            let valid_reservation = matches!(
                lifecycle.connections.get(&conn),
                Some(ConnectionCustody {
                    actor: reserved_actor,
                    phase: LifecyclePhase::Reserved,
                    ..
                }) if *reserved_actor == actor
            );
            if !valid_reservation {
                tracing::error!(conn, %actor, "ConnUp without matching lifecycle reservation refused");
                return false;
            }

            let sent = self.enqueue_raw(msg);
            if sent {
                let custody = lifecycle
                    .connections
                    .get_mut(&conn)
                    .expect("validated lifecycle reservation disappeared");
                custody.phase = LifecyclePhase::Live;
                custody.restore_linkdead_on_cancel = false;
            } else {
                lifecycle.cancel_reserved(conn);
            }
            return sent;
        }

        if let ToServer::Inbound(WireInbound::ConnDown {
            conn,
            actor,
            reason,
        }) = &msg
        {
            let conn = *conn;
            let actor = *actor;
            let reason = *reason;
            // Hold the lifecycle lock through enqueue so a concurrent `ConnUp`
            // for this conn/actor cannot overtake its teardown in the FIFO.
            let mut lifecycle = self.lifecycle.lock().expect("lifecycle custody poisoned");
            lifecycle.observe_conn_down(conn, actor, reason);
            return self.enqueue_raw(msg);
        }

        if let ToServer::Inbound(WireInbound::SessionGone { actor }) = &msg {
            let actor = *actor;
            // Serialize expiry against reservations. If expiry won while a
            // reconnect was merely reserved, invalidate that reservation: its
            // later ConnUp fails and rolls the local attach back, preserving
            // SessionGone-before-any-fresh-ConnUp ordering.
            let mut lifecycle = self.lifecycle.lock().expect("lifecycle custody poisoned");
            let cancelled = lifecycle.observe_session_gone(actor);
            for conn in cancelled {
                tracing::warn!(conn, %actor, "link-dead expiry cancelled an in-progress reconnect reservation");
            }
            return self.enqueue_raw(msg);
        }

        self.enqueue_raw(msg)
    }

    /// Bump the gauge (recording the high-water), then enqueue. Returns `false`
    /// only when the receiver is gone, undoing the gauge bump in that case.
    fn enqueue_raw(&self, msg: ToServer) -> bool {
        let depth = self.depth.fetch_add(1, Ordering::Relaxed) + 1;
        self.observe_depth(depth);
        if self.tx.send(msg).is_ok() {
            true
        } else {
            self.depth.fetch_sub(1, Ordering::Relaxed);
            false
        }
    }

    /// Record a queue-occupancy observation into the high-water gauge and fire an
    /// edge-gated warn on a NEW crossing of `high_water_pct` of `cap`.
    fn observe_depth(&self, depth: u64) {
        self.metrics.observe_ipc_to_server_depth(depth);
        let threshold = self.cap.saturating_mul(self.high_water_pct as usize) / 100;
        if threshold > 0 && depth >= threshold as u64 {
            if !self.hw_warned.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    dir = "to_server",
                    queue_len = depth,
                    cap = self.cap,
                    "ipc queue high-water crossed"
                );
            }
        } else {
            self.hw_warned.store(false, Ordering::Relaxed);
        }
    }
}

/// One live server boundary: the current connection's `ToServer` writer inbox
/// plus its own correlated-request table. Swapped as a unit each (re)connect so
/// a stale reply table is never reused across a restart (M16c). A fresh table
/// per connection means [`request`](SocketGatewayLink::request) and the reader's
/// teardown drain always agree on the SAME table for one server lifetime.
struct BoundaryConn {
    tx: ToServerTx,
    pending: Arc<Mutex<Pending>>,
}

impl BoundaryConn {
    /// Send one correlated service request over THIS exact boundary.
    ///
    /// Keeping registration and send on the connection lets the restart
    /// supervisor prove a fresh server is resynchronized before publishing it
    /// through [`BoundaryHandle`]. The ordinary [`SocketGatewayLink`] path uses
    /// the same helper after allocating a process-unique request id.
    fn request_with_id(
        self: &Arc<Self>,
        id: u64,
        req: ServiceReq,
    ) -> BoxFuture<'static, Option<ServiceResp>> {
        let (otx, orx) = oneshot::channel();
        // Park the waiter BEFORE sending so a fast reply can never race ahead
        // of registration. Admission and boundary teardown share this lock, so
        // a waiter cannot slip into an already-drained table.
        let admission = {
            let mut p = self.pending.lock().expect("pending table poisoned");
            if p.down {
                PendingAdmission::Down
            } else if p.waiters.len() >= MAX_IN_FLIGHT_SERVICE_REQUESTS
                || p.waiters.contains_key(&id)
            {
                if !p.capacity_warned {
                    tracing::warn!(
                        id,
                        pending = p.waiters.len(),
                        cap = MAX_IN_FLIGHT_SERVICE_REQUESTS,
                        "ipc service-request custody full or correlation id already live; refusing request"
                    );
                    p.capacity_warned = true;
                }
                PendingAdmission::Busy
            } else {
                p.waiters.insert(id, otx);
                PendingAdmission::Registered
            }
        };
        match admission {
            PendingAdmission::Down => return Box::pin(async move { None }),
            PendingAdmission::Busy => {
                return Box::pin(async move { Some(ServiceResp::Busy) });
            }
            PendingAdmission::Registered => {}
        }

        // A ServiceReq is must-deliver once its bounded waiter custody has been
        // admitted. A failed enqueue removes that exact waiter immediately.
        let sent = self.tx.send(ToServer::Request { id, req });
        let pending = Arc::clone(&self.pending);
        Box::pin(async move {
            if !sent {
                let mut p = pending.lock().expect("pending table poisoned");
                p.waiters.remove(&id);
                p.capacity_warned = false;
                return None;
            }
            orx.await.ok()
        })
    }
}

/// Input contexts may absorb one additional queue-capacity burst so a normal
/// pasted mail body is less likely to shed, but exemption from the transport
/// rate limiter must never mean unbounded custody in the IPC queue.
const INPUT_CONTEXT_QUEUE_MULTIPLIER: usize = 2;

/// The gateway's swappable pointer to the CURRENT server boundary, or `None`
/// during a restart gap (D3). Cloned into [`SocketGatewayLink`] so every input
/// path targets the live boundary — or rejects, when the server is down and the
/// supervisor is respawning it. Held stable across restarts; only the inner
/// [`BoundaryConn`] swaps, so the bridges never re-wire their `Arc<dyn
/// GatewayLink>`.
#[derive(Clone, Default)]
struct BoundaryHandle {
    inner: Arc<Mutex<Option<Arc<BoundaryConn>>>>,
    /// Raised for the duration of a restart gap (server down, respawn in
    /// progress), so the input path surfaces a clear "restarting" notice rather
    /// than a generic drop.
    restarting: Arc<AtomicBool>,
}

impl BoundaryHandle {
    /// The current live boundary, or `None` during a restart gap.
    fn current(&self) -> Option<Arc<BoundaryConn>> {
        self.inner.lock().expect("boundary handle poisoned").clone()
    }

    /// Install a freshly (re)connected boundary and clear the restart gap.
    fn connect(&self, conn: Arc<BoundaryConn>) {
        *self.inner.lock().expect("boundary handle poisoned") = Some(conn);
        self.restarting.store(false, Ordering::SeqCst);
    }

    /// Enter the restart gap: drop the live boundary and refuse input until the
    /// next [`connect`](BoundaryHandle::connect).
    fn enter_gap(&self) {
        *self.inner.lock().expect("boundary handle poisoned") = None;
        self.restarting.store(true, Ordering::SeqCst);
    }

    /// Whether the boundary is currently in a restart gap.
    fn is_restarting(&self) -> bool {
        self.restarting.load(Ordering::SeqCst)
    }
}

/// The web `hello` manifest the server advertises in its [`ToGateway::Hello`]:
/// the per-module `features` (comsys as `"channels"`) and the panel-feed
/// `panels` (module feeds plus `"who"`). Captured at the handshake and cached
/// so [`serve`] can build the web client's `WebConfig`; a fresh Hello on a
/// server RESTART re-delivers it and updates the cached copy.
#[derive(Clone, Debug, Default)]
struct HelloManifest {
    features: Vec<String>,
    panels: Vec<String>,
}

/// The shared, restart-surviving cache of the latest [`HelloManifest`] the
/// server delivered.
type ManifestCache = Arc<Mutex<HelloManifest>>;

/// The gateway-side [`GatewayLink`]: serializes every gateway → server message
/// onto the CURRENT boundary socket (via the swappable [`BoundaryHandle`]) and
/// correlates [`ServiceReq`]/[`ServiceResp`] pairs by a monotonic id. A restart
/// swaps the boundary underneath it; the bridges hold the same
/// `Arc<dyn GatewayLink>` for the gateway's whole lifetime.
pub struct SocketGatewayLink {
    boundary: BoundaryHandle,
    next_id: AtomicU64,
    /// Random identity for this gateway process. Combined with
    /// `next_input_sequence` to make restart-replay UIDs unique across gateway
    /// process lifetimes without persistence.
    input_epoch: u64,
    next_input_sequence: AtomicU64,
    /// Stable across server restarts; released only by an explicit server
    /// disposition (or rolled back when the initial enqueue fails).
    input_custody: Arc<Mutex<InputCustody>>,
    /// Stable across server restarts so held sessions and delayed link-dead
    /// cleanup retain the same bounded custody ledger as each fresh writer.
    lifecycle: Arc<Mutex<LifecycleCustody>>,
    /// The gateway's session manager — consulted by the input-shed to never drop
    /// a flood-exempt (input-context) connection's `Cmd`/`Query` (M18), mirroring
    /// how such a paste bypasses the flood throttle.
    sessions: Arc<SessionManager>,
    /// The shared registry: the input-shed bumps `ipc_frames_shed_total` and the
    /// `to_server` high-water here.
    metrics: Arc<Metrics>,
    /// `[ipc].to_server_cap`: at/over this depth a `Cmd`/`Query` is shed (M18).
    to_server_cap: usize,
}

impl SocketGatewayLink {
    /// Allocate a correlation id without ever wrapping back into the live id
    /// space. Exhaustion is practically unreachable, but wrapping could pair
    /// a reply with the wrong parked waiter, so it must fail closed.
    fn allocate_request_id(&self) -> Option<u64> {
        self.next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .ok()
    }

    /// A fresh, empty pending-request table for one boundary connection.
    fn pending_table() -> Arc<Mutex<Pending>> {
        Arc::new(Mutex::new(Pending {
            waiters: HashMap::new(),
            down: false,
            capacity_warned: false,
        }))
    }

    /// Whether `msg` is a flood-exempt connection's input (an input-context
    /// paste — mail/chargen). Such input receives a bounded extra burst window;
    /// it never bypasses IPC custody limits entirely. Only `Cmd`/`Query` carry
    /// an actor to check; everything else returns `false`.
    fn is_input_exempt(&self, input: &GatewayInput) -> bool {
        let actor = match input {
            GatewayInput::Cmd { actor, .. } | GatewayInput::Query { actor, .. } => *actor,
            GatewayInput::Action(action) => action.actor,
        };
        self.sessions.is_char_flood_exempt(char_key(actor))
    }

    fn allocate_input_request(&self, input: GatewayInput) -> Option<GatewayRequest> {
        self.next_input_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .ok()
            .map(|sequence| GatewayRequest::new(self.input_epoch, sequence, input))
    }

    /// The shared send + input-shed decision behind both
    /// [`to_server`](GatewayLink::to_server) and
    /// [`to_server_input`](GatewayLink::to_server_input) (M18). `Cmd`,
    /// `Query`, and pre-custody `Action` are sheddable: ordinary input is dropped at
    /// `to_server_cap`; input-context traffic gets one additional bounded cap
    /// of burst room and is then dropped too. Every must-deliver frame
    /// (lifecycle, control, `ReAnnounce`, ...) always enqueues (I1). A missing
    /// boundary (restart gap / shutdown) resolves to [`InputSend::Gone`].
    fn classify_send(&self, msg: ToServer) -> InputSend {
        let Some(conn) = self.boundary.current() else {
            // Session teardown can race a server restart gap. No peer exists to
            // receive these frames, but the next ReAnnounce snapshot is built
            // from the already-updated SessionManager, so update the stable
            // custody ledger locally or gap-time disconnects would leak slots.
            let mut lifecycle = self.lifecycle.lock().expect("lifecycle custody poisoned");
            match &msg {
                ToServer::Inbound(WireInbound::ConnUp { conn, .. }) => {
                    lifecycle.cancel_reserved(*conn);
                }
                ToServer::Inbound(WireInbound::ConnDown {
                    conn,
                    actor,
                    reason,
                }) => lifecycle.observe_conn_down(*conn, *actor, *reason),
                ToServer::Inbound(WireInbound::SessionGone { actor }) => {
                    let cancelled = lifecycle.observe_session_gone(*actor);
                    for conn in cancelled {
                        tracing::warn!(
                            conn,
                            %actor,
                            "gap-time link-dead expiry cancelled a reconnect reservation"
                        );
                    }
                }
                _ => {}
            }
            return InputSend::Gone;
        };
        let input = match msg {
            ToServer::Inbound(wire) => match GatewayInput::try_from(wire) {
                Ok(input) => input,
                Err(lifecycle) => {
                    return if conn.tx.send(ToServer::Inbound(lifecycle)) {
                        InputSend::Sent
                    } else {
                        InputSend::Gone
                    };
                }
            },
            other => {
                return if conn.tx.send(other) {
                    InputSend::Sent
                } else {
                    InputSend::Gone
                };
            }
        };

        self.classify_input_on(&conn, input)
    }

    fn classify_input(&self, input: GatewayInput) -> InputSend {
        let Some(conn) = self.boundary.current() else {
            return InputSend::Gone;
        };
        self.classify_input_on(&conn, input)
    }

    fn classify_input_on(&self, conn: &BoundaryConn, input: GatewayInput) -> InputSend {
        let depth = conn.tx.depth();
        let exempt = self.is_input_exempt(&input);
        let input_cap = if exempt {
            self.to_server_cap
                .saturating_mul(INPUT_CONTEXT_QUEUE_MULTIPLIER)
        } else {
            self.to_server_cap
        };
        let input_kind = match &input {
            GatewayInput::Cmd { .. } => "cmd",
            GatewayInput::Query { .. } => "query",
            GatewayInput::Action(_) => "action",
        };
        if depth >= input_cap as u64 {
            self.metrics.ipc_frame_shed();
            self.metrics.observe_ipc_to_server_depth(depth);
            tracing::warn!(
                dir = "to_server",
                queue_len = depth,
                cap = input_cap,
                input_context = exempt,
                dropped_total = self.metrics.ipc_frames_shed_total(),
                kind = input_kind,
                "ipc queue full; shedding input frame"
            );
            return InputSend::Shed;
        }

        let Some(request) = self.allocate_input_request(input) else {
            tracing::error!("gateway request sequence exhausted; refusing player input");
            return InputSend::Shed;
        };
        let uid = request.uid;
        let admission = self
            .input_custody
            .lock()
            .expect("input custody poisoned")
            .admit(request.clone());
        if admission != InputCustodyAdmission::Admitted {
            self.metrics.ipc_frame_shed();
            if !matches!(admission, InputCustodyAdmission::Full { warn: false }) {
                tracing::warn!(
                    request = %uid,
                    ?admission,
                    conn = request.input.conn(),
                    kind = input_kind,
                    "gateway input custody refused request"
                );
            }
            return InputSend::Shed;
        }

        if conn.tx.send(ToServer::Input(request)) {
            InputSend::Sent
        } else {
            self.input_custody
                .lock()
                .expect("input custody poisoned")
                .remove(uid);
            InputSend::Gone
        }
    }
}

impl GatewayLink for SocketGatewayLink {
    fn try_reserve_connection(&self, conn: u64, actor: mush_core::ObjId) -> ConnectionAdmission {
        let Some(boundary) = self.boundary.current() else {
            return ConnectionAdmission::Gone;
        };
        boundary.tx.try_reserve_connection(conn, actor)
    }

    fn cancel_connection_reservation(&self, conn: u64) {
        if let Some(boundary) = self.boundary.current() {
            boundary.tx.cancel_connection_reservation(conn);
        } else {
            self.lifecycle
                .lock()
                .expect("lifecycle custody poisoned")
                .cancel_reserved(conn);
        }
    }

    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        // A shed `Cmd`/`Query` or a missing boundary (restart gap / shutdown)
        // resolves `false`, which the bridges map to the WorldGone / busy
        // rejection; only [`InputSend::Sent`] counts as delivered.
        let sent = matches!(self.classify_send(msg), InputSend::Sent);
        Box::pin(async move { sent })
    }

    fn to_server_input(&self, msg: ToServer) -> BoxFuture<'_, InputSend> {
        // The input path wants the precise outcome (Sent / Shed / Gone) so the
        // bridge can pick the right player-facing notice.
        let outcome = self.classify_send(msg);
        Box::pin(async move { outcome })
    }

    fn submit_action(&self, action: GatewayAction) -> BoxFuture<'_, ActionSend> {
        let outcome = match self.classify_input(GatewayInput::Action(Box::new(action))) {
            InputSend::Sent => ActionSend::Accepted,
            InputSend::Shed => ActionSend::Busy,
            InputSend::Gone => ActionSend::Gone,
        };
        Box::pin(async move { outcome })
    }

    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        // Resolve the CURRENT boundary once; the whole request (park + send +
        // teardown-drain) is scoped to that one connection's table, so a restart
        // that swaps the boundary can never leave a waiter parked in a table the
        // new connection's reader will never drain.
        let Some(conn) = self.boundary.current() else {
            return Box::pin(async move { None });
        };
        let Some(id) = self.allocate_request_id() else {
            tracing::error!("ipc service-request correlation id space exhausted; refusing request");
            return Box::pin(async move { Some(ServiceResp::Unavailable) });
        };
        conn.request_with_id(id, req)
    }

    fn is_restarting(&self) -> bool {
        self.boundary.is_restarting()
    }
}

/// The socket writer: forward each queued [`ToServer`] to the server. Exits
/// when every sender is dropped (gateway shutting down), a write fails (server
/// gone), or the supervisor fires `close` for a deliberate restart (D1) —
/// dropping `w` on return sends the server EOF, so a still-live server shuts
/// down cleanly and the gateway respawns it.
async fn gateway_writer(
    mut w: BoxWrite,
    mut rx: mpsc::UnboundedReceiver<ToServer>,
    mut close: oneshot::Receiver<()>,
    depth: Arc<AtomicU64>,
    metrics: Arc<Metrics>,
    write_timeout: Duration,
) {
    loop {
        let msg = tokio::select! {
            m = rx.recv() => m,
            _ = &mut close => {
                tracing::info!("boundary writer closing (deliberate restart)");
                return;
            }
        };
        let Some(msg) = msg else { return };
        // Each write is bounded by `write_timeout` (M18): a connected-but-not-
        // draining server is wedged, so we count it, warn, and return — dropping
        // `w` EOFs the boundary into the supervisor's relaunch/re-attach heal.
        // Never skip-a-message-and-continue: the socket buffer is full, so the
        // next write blocks too and skipping could drop a must-deliver frame.
        match tokio::time::timeout(write_timeout, mush_ipc::write_frame(&mut w, &msg)).await {
            Ok(Ok(())) => {
                // The frame left our queue; drop the external depth gauge to
                // track true occupancy for the input-shed / high-water.
                depth.fetch_sub(1, Ordering::Relaxed);
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "ipc write to server failed; boundary down");
                return;
            }
            Err(_) => {
                metrics.ipc_write_timeout();
                tracing::warn!(
                    timeout_ms = write_timeout.as_millis() as u64,
                    "ipc write timed out; peer wedged, tearing down boundary"
                );
                return;
            }
        }
    }
}

/// The socket reader: decode each [`ToGateway`] and route it onto the local
/// session layer. Returns when the server closes the boundary (clean EOF /
/// [`ToGateway::Bye`]) or a protocol fault occurs.
async fn gateway_reader(
    mut reader: FrameReader<BoxRead>,
    transport: SessionDelivery,
    pending: Arc<Mutex<Pending>>,
    to_server_tx: ToServerTx,
    sessions: Arc<SessionManager>,
    manifest: ManifestCache,
    input_custody: Arc<Mutex<InputCustody>>,
) {
    loop {
        let msg = match reader.read::<ToGateway>().await {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::info!("server closed the boundary (EOF)");
                break;
            }
            Err(e) => {
                tracing::warn!(error = %e, "ipc reader from server ended on error");
                break;
            }
        };
        match msg {
            ToGateway::Deliver { target, event } => {
                // Same SendTarget → session routing the in-process gateway runs;
                // the world already resolved audience/perception server-side.
                // The transport carries boundary feedback (D5), so it
                // self-polices any overflowed connection here — detaching the
                // slow client and feeding its `ConnDown` (and, on link-dead, a
                // later `SessionGone`) back to the server as inbound — without
                // the world loop ever touching the session layer.
                transport.transmit(vec![(target, Arc::new(event))]);
            }
            ToGateway::SetFloodExempt { char, exempt } => {
                transport.set_flood_exempt(char, exempt);
            }
            ToGateway::Reply { id, resp } => {
                let waiter = {
                    let mut p = pending.lock().expect("pending table poisoned");
                    let waiter = p.waiters.remove(&id);
                    if p.waiters.len() < MAX_IN_FLIGHT_SERVICE_REQUESTS {
                        p.capacity_warned = false;
                    }
                    waiter
                };
                if let Some(otx) = waiter {
                    let _ = otx.send(resp);
                } else {
                    tracing::warn!(id, "reply for unknown request id");
                }
            }
            ToGateway::RequestDisposition {
                request,
                disposition,
            } => {
                let held = input_custody
                    .lock()
                    .expect("input custody poisoned")
                    .remove(request);
                match (held, disposition) {
                    (Some(_held), GatewayRequestDisposition::LegacyClaimed) => {
                        // Release our exact replay copy BEFORE authorizing
                        // legacy execution. A crash at any point from here may
                        // lose this legacy command, but can never duplicate it.
                        if !to_server_tx.send(ToServer::GatewayReleased { request }) {
                            tracing::warn!(
                                request = %request,
                                "boundary closed after legacy custody release; command may be lost"
                            );
                        }
                    }
                    (Some(held), GatewayRequestDisposition::Rejected { retryable }) => {
                        let text = if retryable {
                            "GAME: The server is busy; please retry that input."
                        } else {
                            "GAME: That input was rejected by the server."
                        };
                        let _ = sessions.send_to_conn(
                            mush_session::ConnId(held.input.conn()),
                            Arc::new(OutputEvent::Sys(RichText::tag("error", text))),
                        );
                    }
                    (Some(held), GatewayRequestDisposition::Busy) => {
                        let _ = sessions.send_to_conn(
                            mush_session::ConnId(held.input.conn()),
                            Arc::new(OutputEvent::Sys(RichText::tag(
                                "error",
                                "GAME: The server is busy; please retry that input.",
                            ))),
                        );
                    }
                    (Some(_), GatewayRequestDisposition::EphemeralFinished) => {}
                    (Some(held), GatewayRequestDisposition::Owned { status, .. }) => {
                        let _ = sessions.send_to_conn(
                            mush_session::ConnId(held.input.conn()),
                            Arc::new(OutputEvent::Sys(RichText::tag(
                                "system",
                                format!("Action accepted. Transaction: {status}"),
                            ))),
                        );
                    }
                    (_, GatewayRequestDisposition::Finished { recipient, receipt }) => {
                        match receipt.view() {
                            Ok(view) => transport.transmit(vec![(
                                mush_ipc::SendTarget::Char(u64::from(recipient.0)),
                                Arc::new(OutputEvent::View(view)),
                            )]),
                            Err(error) => tracing::error!(
                                %recipient,
                                error = %error,
                                "projected action receipt frame is invalid"
                            ),
                        }
                    }
                    (None, disposition) => {
                        // A terminal update may legitimately follow `Owned`,
                        // and a resent disposition may cross a reconnect. Both
                        // are idempotent once gateway custody is already gone.
                        tracing::debug!(
                            request = %request,
                            ?disposition,
                            "request disposition arrived after gateway custody was released"
                        );
                    }
                }
            }
            ToGateway::CloseSession { char, cause } => {
                close_char_sessions(&sessions, &to_server_tx, char, cause);
            }
            ToGateway::Detach { conn, cause } => {
                detach_one_conn(&sessions, &to_server_tx, conn, cause);
            }
            ToGateway::CloseWebSessions {
                chars,
                notice,
                cause,
            } => {
                close_web_sessions(&sessions, &to_server_tx, chars, notice.as_ref(), cause);
            }
            ToGateway::Hello {
                proto,
                server,
                features,
                panels,
            } => {
                // A steady-state Hello (a freshly (re)started server re-greets
                // over its new boundary): refresh the cached manifest so it
                // tracks the current server. The already-running web listener
                // keeps its boot-time WebConfig snapshot; the cache update keeps
                // the gateway's view current for any later read.
                tracing::info!(
                    proto,
                    server,
                    "late Hello (already handshaked); refreshing manifest"
                );
                *manifest.lock().expect("manifest cache poisoned") =
                    HelloManifest { features, panels };
            }
            ToGateway::Bye => {
                tracing::info!("server said Bye; closing boundary");
                break;
            }
        }
    }
    // Boundary gone (EOF / error / Bye): fail every in-flight service request
    // fast instead of leaving its bridge task parked on `orx.await` forever
    // (F3). Raise `down` and drop every parked sender under the lock — dropping
    // a `oneshot::Sender` resolves the awaiting `orx` to `Err` → `None`, which
    // the bridges map to the WorldGone rejection, exactly as the in-process
    // path resolves when the world task drops its reply oneshot. The `down`
    // flag then makes any request that races in after this drain fail fast too
    // (it never parks), so no request can hang across a mid-request teardown.
    let dropped = {
        let mut p = pending.lock().expect("pending table poisoned");
        p.down = true;
        p.capacity_warned = false;
        p.waiters.drain().count()
    };
    if dropped > 0 {
        tracing::warn!(dropped, "boundary down; failed in-flight service requests");
    }
}

/// Map a [`DetachCause`] to the [`DisconnectKind`] the engine announces.
fn cause_to_kind(cause: DetachCause) -> DisconnectKind {
    match cause {
        DetachCause::Booted => DisconnectKind::Boot,
        DetachCause::SlowClient => DisconnectKind::Lost,
    }
}

/// Resolve `char` to its live session and close every connection, reporting
/// each resulting `ConnDown` back to the server as inbound so the world runs
/// the per-connection announce + last-connection disconnect (the server side of
/// `@boot` in split mode). Dropping the returned handles closes the transports.
fn close_char_sessions(
    sessions: &Arc<SessionManager>,
    to_server_tx: &ToServerTx,
    char: u64,
    cause: DetachCause,
) {
    let actor = char_key_obj(CharKey(char));
    let Some(sess) = sessions.session_of_char(CharKey(char)) else {
        return;
    };
    match cause {
        DetachCause::Booted => {
            for handle in sessions.close_session(sess) {
                let _ = to_server_tx.send(ToServer::Inbound(WireInbound::ConnDown {
                    conn: handle.id.0,
                    actor,
                    reason: DisconnectKind::Boot,
                }));
                // `handle` drops here → its output sender closes → the
                // transport pump closes the socket once its queue drains.
            }
        }
        DetachCause::SlowClient => {
            // Slow-client teardown is unclean. Preserve the session and its
            // resume ring after the final connection is removed, then reap it
            // through the normal grace-window path.
            for handle in sessions.conns_of_session(sess) {
                detach_one_conn(sessions, to_server_tx, handle.id.0, cause);
            }
        }
    }
}

/// Detach one specific connection (`@newpassword`/slow-client in split mode),
/// reporting its `ConnDown` back to the server.
fn detach_one_conn(
    sessions: &Arc<SessionManager>,
    to_server_tx: &ToServerTx,
    conn: u64,
    cause: DetachCause,
) {
    let conn_id = mush_session::ConnId(conn);
    let actor = sessions
        .session_of_conn(conn_id)
        .and_then(|s| sessions.character_of(s))
        .map(char_key_obj);
    let detach_reason = match cause {
        DetachCause::Booted => DetachReason::Booted,
        DetachCause::SlowClient => DetachReason::SlowClient,
    };
    let Some(outcome) = sessions.detach(conn_id, detach_reason) else {
        return;
    };
    if let Some(actor) = actor {
        let _ = to_server_tx.send(ToServer::Inbound(WireInbound::ConnDown {
            conn,
            actor,
            reason: cause_to_kind(cause),
        }));
        if let DetachOutcome::Linkdead { grace } = outcome {
            let reaper_sessions = Arc::clone(sessions);
            let reaper_boundary = to_server_tx.clone();
            crate::delivery::spawn_linkdead_reaper(reaper_sessions, grace, move |expired_actor| {
                let boundary = reaper_boundary.clone();
                async move {
                    let _ = boundary.send(ToServer::Inbound(WireInbound::SessionGone {
                        actor: expired_actor,
                    }));
                    std::ops::ControlFlow::Continue(())
                }
            });
        }
    }
}

/// Account/char-addressed logout-everywhere (`@newpassword` in split mode):
/// close every **structured** (web/token) connection of `chars`, delivering
/// `notice` to each just before it drops, and report each resulting `ConnDown`
/// back to the server as inbound. SSH connections (session-authenticated) are
/// left untouched. Resolves the whole thing locally on the gateway's own
/// `SessionManager`, so the server never needs a reverse session query (D4).
fn close_web_sessions(
    sessions: &Arc<SessionManager>,
    to_server_tx: &ToServerTx,
    chars: Vec<u64>,
    notice: Option<&mush_proto::OutputEvent>,
    cause: DetachCause,
) {
    for char in chars {
        let Some(sess) = sessions.session_of_char(CharKey(char)) else {
            continue;
        };
        for handle in sessions.conns_of_session(sess) {
            if !handle.is_structured() {
                continue;
            }
            let conn = handle.id;
            if let Some(ev) = notice {
                let _ = sessions.send_to_conn(conn, Arc::new(ev.clone()));
            }
            // Dropping the handle (inside `detach_one_conn`) closes its
            // transport once the notice drains. Although the only current
            // producer uses `Booted`, keep the wire cause authoritative here.
            detach_one_conn(sessions, to_server_tx, conn.0, cause);
        }
    }
}

/// Connect to the server's IPC endpoint, retrying until it binds or the
/// deadline passes (the child may still be starting up).
async fn connect_with_retry(
    endpoint: &IpcEndpoint,
    timeout: Duration,
) -> Result<(BoxRead, BoxWrite), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match mush_ipc::connect(endpoint).await {
            Ok(pair) => return Ok(pair),
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "cannot connect to server at {}: {e}",
                        endpoint.describe()
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Spawn the `mush-server` child process (the supervisor). Locates the binary
/// as a sibling of the current executable in the same target dir.
fn spawn_server_child(
    config_path: &std::path::Path,
    endpoint: &IpcEndpoint,
) -> Result<tokio::process::Child, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "current_exe has no parent dir".to_string())?;
    let server_bin = dir.join(format!("mush-server{}", std::env::consts::EXE_SUFFIX));
    tracing::info!(bin = %server_bin.display(), "spawning mush-server child");
    tokio::process::Command::new(&server_bin)
        .arg("--config")
        .arg(config_path)
        .arg("--ipc")
        .arg(endpoint.describe())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawning {}: {e}", server_bin.display()))
}

/// The production [`ServerLauncher`]: spawn (and, on each restart, respawn) a
/// `mush-server` CHILD PROCESS bound to the config's fixed `[ipc].listen`
/// endpoint (D1). `kill_on_drop` is armed as a final runtime-teardown backstop;
/// the supervisor still requires positive exit confirmation before respawn.
struct SpawnLauncher {
    config_path: PathBuf,
    endpoint: IpcEndpoint,
}

impl ServerLauncher for SpawnLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let config_path = self.config_path.clone();
        let endpoint = self.endpoint.clone();
        Box::pin(async move {
            let mut child = spawn_server_child(&config_path, &endpoint)?;
            // Own the child in a task that resolves the `exited` future when it
            // dies. The exit-only API cannot force that task to drop, so the
            // supervisor fails closed if this confirmation misses its deadline.
            let (exit_tx, exit_rx) = oneshot::channel();
            tokio::spawn(async move {
                match child.wait().await {
                    Ok(status) => tracing::warn!(%status, "mush-server child exited"),
                    Err(e) => tracing::warn!(error = %e, "waiting on mush-server child"),
                }
                let _ = exit_tx.send(());
            });
            Ok(LaunchedServer {
                endpoint,
                exited: Box::pin(async move {
                    let _ = exit_rx.await;
                }),
            })
        })
    }
}

/// One live, wired-up server boundary the supervisor is serving over.
struct WiredBoundary {
    /// The swappable unit installed into the [`BoundaryHandle`].
    conn: Arc<BoundaryConn>,
    /// The reader task; resolves when the boundary drops (server EOF / error /
    /// Bye) — the auto-restart trigger (D1a).
    down: JoinHandle<()>,
    /// Fire to close the boundary writer for a DELIBERATE restart (D1b): the
    /// server sees EOF and exits, and the supervisor respawns it.
    close: oneshot::Sender<()>,
}

/// The M18 IPC-bounding knobs + shared metrics threaded into every (re)wired
/// boundary: the input-shed cap, the high-water warn threshold, the per-frame
/// write timeout, and the registry the shed/timeout/high-water counters land in.
/// Cloned per (re)connect; the registry `Arc` is shared with the SSH/web bridges
/// so `/metrics` + `/healthz` + `@stats` all reflect the same numbers.
#[derive(Clone)]
struct BoundaryTuning {
    metrics: Arc<Metrics>,
    to_server_cap: usize,
    high_water_pct: u8,
    write_timeout: Duration,
}

/// The restart-surviving context every (re)connect needs: the shared Hello
/// manifest cache and the [`BoundaryTuning`]. Threaded through the supervisor so
/// each respawn wires its fresh boundary the same way.
#[derive(Clone)]
struct BoundaryEnv {
    manifest: ManifestCache,
    tuning: BoundaryTuning,
    /// One custody ledger for the gateway process, reused by each respawned
    /// server boundary.
    lifecycle: Arc<Mutex<LifecycleCustody>>,
    /// Replay custody is gateway-process state and therefore crosses every
    /// server boundary replacement unchanged.
    input_custody: Arc<Mutex<InputCustody>>,
}

/// Connect to a (freshly launched) server endpoint, run the [`ToGateway::Hello`]
/// handshake + [`IPC_VERSION`] check, and wire up the per-connection boundary:
/// a fresh `ToServer` channel + reply table, a writer task, and a reader task
/// routing `ToGateway` onto the gateway's stable [`SessionManager`]. Returns the
/// swappable unit plus the reader's completion handle (the boundary-down
/// signal) and the deliberate-close trigger.
async fn connect_and_wire(
    endpoint: &IpcEndpoint,
    sessions: &Arc<SessionManager>,
    connect_timeout: Duration,
    env: &BoundaryEnv,
) -> Result<WiredBoundary, String> {
    let manifest = &env.manifest;
    let (reader_half, writer_half) = connect_with_retry(endpoint, connect_timeout).await?;

    // Handshake: the server greets with Hello before anything else. Capture its
    // panel manifest into the shared cache so `serve` can build the web client's
    // WebConfig (and a restart's fresh Hello refreshes it here again).
    let mut reader = FrameReader::new(reader_half);
    match reader
        .read::<ToGateway>()
        .await
        .map_err(|e| format!("reading Hello: {e}"))?
    {
        Some(ToGateway::Hello {
            proto,
            server,
            features,
            panels,
        }) => {
            if proto != IPC_VERSION {
                return Err(format!(
                    "ipc version mismatch: server speaks {proto}, gateway speaks {IPC_VERSION}"
                ));
            }
            *manifest.lock().expect("manifest cache poisoned") = HelloManifest { features, panels };
            tracing::info!(proto, server, "handshake ok");
        }
        Some(other) => return Err(format!("expected Hello, got {other:?}")),
        None => return Err("server closed the boundary before Hello".to_string()),
    }

    let (raw_tx, to_server_rx) = mpsc::unbounded_channel::<ToServer>();
    // The shared external depth gauge for this boundary (M18): every producer
    // bumps it on enqueue; the writer decrements it after each successful write,
    // so it tracks TRUE queue occupancy for the input-shed + high-water gauge.
    let depth = Arc::new(AtomicU64::new(0));
    let to_server_tx = ToServerTx::new(
        raw_tx,
        Arc::clone(&depth),
        Arc::clone(&env.tuning.metrics),
        env.tuning.to_server_cap,
        env.tuning.high_water_pct,
        Arc::clone(&env.lifecycle),
    );
    let pending = SocketGatewayLink::pending_table();
    let conn = Arc::new(BoundaryConn {
        tx: to_server_tx.clone(),
        pending: Arc::clone(&pending),
    });
    // D5: the reader's delivery transport self-polices slow clients and reaps
    // link-dead sessions, feeding the resulting `ConnDown`/`SessionGone` back to
    // THIS server over its own writer inbox.
    let transport = SessionDelivery::new(Arc::clone(sessions), to_server_tx.clone());
    let (close_tx, close_rx) = oneshot::channel();
    tokio::spawn(gateway_writer(
        writer_half,
        to_server_rx,
        close_rx,
        depth,
        Arc::clone(&env.tuning.metrics),
        env.tuning.write_timeout,
    ));
    let down = tokio::spawn(gateway_reader(
        reader,
        transport,
        pending,
        to_server_tx,
        Arc::clone(sessions),
        Arc::clone(manifest),
        Arc::clone(&env.input_custody),
    ));
    Ok(WiredBoundary {
        conn,
        down,
        close: close_tx,
    })
}

/// Build the gateway's full snapshot of held ONLINE attachments to replay to a
/// freshly (re)started server (D2). One [`AttachRec`] per live connection of
/// every non-linkdead session — a multi-connected session contributes one rec
/// per connection so the server rebuilds `live_conns` exactly. Linkdead sessions
/// (no live connection) are skipped: the server has no connection to render to,
/// and the linkdead grace timer is gateway-resident and keeps ticking.
fn build_attach_recs(sessions: &SessionManager) -> Vec<AttachRec> {
    let mut recs = Vec::new();
    for meta in sessions.who() {
        if meta.linkdead || meta.conns == 0 {
            continue;
        }
        let actor = char_key_obj(meta.character);
        for handle in sessions.conns_of_session(meta.sess) {
            recs.push(AttachRec {
                conn: handle.id.0,
                actor,
                account: Some(meta.account.clone()),
                // The gateway's SessionManager keys by CharKey, not name; the
                // server resolves the real name from the reloaded world for its
                // logs. Carry the account for the gateway's own trace only.
                char_name: String::new(),
                caps_structured: handle.caps().structured,
            });
        }
    }
    recs
}

/// The gap-start notice broadcast to every connected player when the server link
/// drops (D3): a clear "restarting, hold on" system line. Input is refused for
/// the duration of the gap (the WorldGone reject path), not dropped.
fn restarting_notice() -> OutputEvent {
    OutputEvent::Sys(RichText::tag(
        "system",
        "GAME: The server is restarting; hold on — you will not be disconnected.",
    ))
}

/// The resync notice broadcast only after the fresh server has accepted the
/// full attachment snapshot and crossed an exact world-loop barrier (D2/D3).
/// A player's fresh room render is produced by that replay and may arrive just
/// before this line; once this line arrives, gameplay input is commit-ready.
fn reconnected_notice() -> OutputEvent {
    OutputEvent::Sys(RichText::tag(
        "system",
        "GAME: Reconnected. Resuming where you left off.",
    ))
}

/// The give-up notice when the crash-loop guard trips (R3): the server keeps
/// dying, so the supervisor stops respawning and tells players.
fn server_down_notice() -> OutputEvent {
    OutputEvent::Sys(RichText::tag(
        "error",
        "GAME: The server is down and could not be restarted. Please try again later.",
    ))
}

/// Broadcast one system line to every connected session.
fn broadcast(sessions: &SessionManager, event: OutputEvent) {
    let _ = sessions.broadcast_all(Arc::new(event));
}

/// The auto-restart crash-loop guard (R3/D1a): allow a bounded number of
/// restarts within a rolling window, with growing backoff between consecutive
/// restarts, and give up if the server keeps dying too fast. A restart that
/// follows a healthy run (longer than the window since the last one) resets the
/// backoff — a one-off crash after weeks of uptime is not a crash loop.
struct CrashGuard {
    /// Timestamps of recent respawn attempts (pruned to the window). Catches a
    /// server that boots and crashes repeatedly FAST: each reconnect succeeds so
    /// the consecutive-failure counter keeps resetting, but the attempts pile up
    /// inside the wall-clock window.
    recent: Vec<Instant>,
    /// Consecutive fast attempts (for the growing backoff).
    consecutive: u32,
    /// Consecutive FAILED relaunch/reconnect attempts, counted independent of
    /// wall-clock time and reset only by a successful reconnect. Catches a server
    /// that never comes back — a pre-bind boot crash, or a relaunch whose connect
    /// burns the full deadline each time, so the wall-clock window alone would
    /// only ever hold a handful of timestamps and never trip.
    consecutive_failures: u32,
}

/// Max respawn attempts allowed within [`CRASH_WINDOW`] before the guard gives
/// up (the fast crash-then-recover loop).
const MAX_RESTARTS_IN_WINDOW: usize = 5;
/// Max CONSECUTIVE failed relaunch/reconnect attempts before the guard gives up,
/// independent of how long each attempt took (the never-comes-back loop).
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// The rolling window the crash-loop guard counts fast attempts over.
const CRASH_WINDOW: Duration = Duration::from_secs(30);
/// Base backoff between consecutive fast restarts (doubles up to the cap).
const BACKOFF_BASE: Duration = Duration::from_millis(200);
/// Cap on the per-restart backoff.
const BACKOFF_CAP: Duration = Duration::from_secs(5);
/// How long to wait for an old instance to fully exit before failing closed.
/// A timeout never permits a replacement launch: this exit-only API cannot yet
/// force-kill and reap a stuck child safely.
const EXIT_WAIT: Duration = Duration::from_secs(5);
/// Connect/handshake budget for the FIRST server boot in [`serve`]. Generous, so
/// a slow first bind under heavy load (e.g. a full `cargo test --workspace` run
/// on Windows) does not spuriously fail the whole gateway before it even binds
/// the SSH listener.
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Connect/handshake budget for a RESPAWN in the supervisor. Shorter than the
/// initial boot: a respawn that cannot connect should fail fast and count toward
/// [`MAX_CONSECUTIVE_FAILURES`] rather than stalling the gap on a dead server.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum time a fresh Hello-speaking server gets to consume the attachment
/// snapshot and cross the world-loop readiness barrier. An alive socket with a
/// wedged world must not strand every held player in the restart gap forever;
/// expiry closes this attempt so boundary teardown drains the parked waiter and
/// the crash guard can retry.
const RESYNC_TIMEOUT: Duration = Duration::from_secs(10);
/// Correlation id reserved for the supervisor's private resync barrier. The
/// ordinary monotonic allocator can advance TO `u64::MAX`, but then refuses its
/// next allocation, so it never returns this value as a live request id.
const RESYNC_BARRIER_REQUEST_ID: u64 = u64::MAX;

/// Wait for one shutdown component to finish inside the old-instance grace
/// window. A timeout is a safety boundary, not permission to launch another
/// server: until the old process is known to be gone, a replacement could
/// overlap it as a second authoritative store writer.
async fn wait_for_exit<F, T>(what: &'static str, future: F) -> bool
where
    F: std::future::Future<Output = T>,
{
    match tokio::time::timeout(EXIT_WAIT, future).await {
        Ok(_) => true,
        Err(_) => {
            tracing::error!(
                what,
                timeout_ms = EXIT_WAIT.as_millis() as u64,
                "server shutdown component did not finish; refusing to respawn while its ownership is uncertain"
            );
            false
        }
    }
}

/// Enter the fail-stop posture for an instance whose exit could not be
/// established. The caller has already put [`BoundaryHandle`] into its gap, so
/// input remains refused. Returning from the supervisor is deliberate: without
/// an owned kill-and-reap handle, retrying could create two live worlds.
fn announce_uncertain_exit(sessions: &SessionManager) {
    broadcast(sessions, server_down_notice());
}

/// Replay the gateway-owned attachment roster onto a fresh server and wait for
/// an exact world-loop barrier before the connection becomes player-visible.
///
/// Both frames use the same FIFO boundary: `ReAnnounce` is therefore accepted
/// by the world before `Barrier`, and a barrier held by CommitPending resolves
/// only after the exact persistence receipt and the coalesced roster have been
/// applied. Until this returns, the public [`BoundaryHandle`] remains in its
/// restart gap, so no gameplay input can race tentative startup state.
async fn resync_boundary(
    conn: &Arc<BoundaryConn>,
    sessions: &SessionManager,
    input_custody: &Arc<Mutex<InputCustody>>,
) -> Result<usize, String> {
    let recs = build_attach_recs(sessions);
    let count = recs.len();
    if count > MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS {
        return Err(format!(
            "held roster has {count} records; protocol cap is {MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS}"
        ));
    }
    // Send even an empty snapshot: this is the explicit full-roster replay for
    // the new server lifetime, not an optional stream of attachment deltas.
    if !conn.tx.send(ToServer::ReAnnounce(recs)) {
        return Err("fresh boundary closed while enqueueing ReAnnounce".to_string());
    }

    match tokio::time::timeout(
        RESYNC_TIMEOUT,
        conn.request_with_id(RESYNC_BARRIER_REQUEST_ID, ServiceReq::Barrier),
    )
    .await
    {
        Ok(Some(ServiceResp::Barrier)) => {
            // The public boundary remains in Gap while this snapshot is
            // enqueued. Existing requests therefore replay after the complete
            // attachment roster, and newly accepted input cannot overtake it.
            let replay = input_custody
                .lock()
                .expect("input custody poisoned")
                .snapshot();
            for request in &replay {
                if !conn.tx.send(ToServer::Input(request.clone())) {
                    return Err(format!(
                        "fresh boundary closed while replaying request {}",
                        request.uid
                    ));
                }
            }
            tracing::info!(requests = replay.len(), "replayed gateway input custody");
            Ok(count)
        }
        Ok(Some(other)) => Err(format!(
            "fresh server refused the post-ReAnnounce barrier with {other:?}"
        )),
        Ok(None) => Err("fresh boundary closed before the post-ReAnnounce barrier".to_string()),
        Err(_) => Err(format!(
            "fresh server did not cross the post-ReAnnounce barrier within {} ms",
            RESYNC_TIMEOUT.as_millis()
        )),
    }
}

impl CrashGuard {
    fn new() -> CrashGuard {
        CrashGuard {
            recent: Vec::new(),
            consecutive: 0,
            consecutive_failures: 0,
        }
    }

    /// Drop attempts that no longer belong to the rolling crash window.
    ///
    /// This must run before *both* the admission check and a new attempt is
    /// recorded. Otherwise a server that recovered after a short crash burst
    /// could be refused hours later because stale timestamps were only pruned
    /// after `tripped` had already made its decision.
    fn prune_window(&mut self, now: Instant) {
        self.recent
            .retain(|&attempt| now.saturating_duration_since(attempt) <= CRASH_WINDOW);
    }

    /// Whether the loop has tripped the guard and must give up. Trips on EITHER
    /// the configured number of fast attempts inside the wall-clock window OR
    /// too many consecutive failed attempts (which may each burn the full
    /// connect deadline, so they slip through the window).
    fn tripped(&mut self) -> bool {
        self.prune_window(Instant::now());
        self.recent.len() >= MAX_RESTARTS_IN_WINDOW
            || self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
    }

    /// Record that a respawn attempt is about to run (drives the window + the
    /// growing backoff). Call once per attempt, before the launch.
    fn record_attempt(&mut self) {
        let now = Instant::now();
        self.prune_window(now);
        let healthy_gap = self.recent.is_empty();
        if healthy_gap {
            // A restart after a healthy run is not part of a loop.
            self.consecutive = 0;
        }
        self.recent.push(now);
        self.consecutive = self.consecutive.saturating_add(1);
    }

    /// Record that the current attempt failed to bring a server back.
    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Record that a respawn succeeded — the server is back and re-attached. Only
    /// the consecutive-failure streak resets; the wall-clock window keeps its
    /// timestamps so a boot-then-crash loop still trips.
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// The backoff to wait before the next respawn (grows with consecutive fast
    /// restarts, capped).
    fn backoff(&self) -> Duration {
        let shift = self.consecutive.saturating_sub(1).min(5);
        BACKOFF_BASE.saturating_mul(1u32 << shift).min(BACKOFF_CAP)
    }
}

/// The gateway supervisor loop (D1/D2/D3): serve over the current boundary until
/// it drops (a server crash, D1a) or a deliberate restart is requested (D1b);
/// then enter the gap (refuse input, tell players), respawn the server, then
/// reconnect and re-attach the held sessions, and resume. Gives up (telling
/// players) if the crash-loop guard trips (R3). Returns when the supervisor can
/// no longer keep a server up.
async fn supervise(
    launcher: Box<dyn ServerLauncher>,
    boundary: BoundaryHandle,
    sessions: Arc<SessionManager>,
    mut restart_rx: mpsc::Receiver<()>,
    first_instance: LaunchedServer,
    first_wired: WiredBoundary,
    env: BoundaryEnv,
) {
    let mut instance = first_instance;
    let mut wired = first_wired;
    let mut guard = CrashGuard::new();
    loop {
        // --- SERVING: wait for a restart reason. ---
        // `crashed` distinguishes an auto-restart (D1a: the boundary reader has
        // already terminated, so `wired.down` is COMPLETE) from a deliberate
        // restart (D1b: the reader is still running until we close the boundary).
        // Re-polling a completed `JoinHandle` panics ("polled after completion"),
        // so the gap below must NOT re-await `wired.down` in the crash case.
        let crashed;
        tokio::select! {
            _ = &mut wired.down => {
                crashed = true;
                tracing::warn!("server boundary down (crash/exit); restarting + re-attaching");
            }
            reason = restart_rx.recv() => {
                if reason.is_none() {
                    // The trigger is gone (gateway shutting down): drop the
                    // supervisor without touching the live server.
                    return;
                }
                crashed = false;
                tracing::info!("deliberate server restart requested");
            }
        }

        // --- GAP (D3): refuse input, tell players, stop the old instance. ---
        boundary.enter_gap();
        broadcast(&sessions, restarting_notice());
        // Close our boundary so a still-live server sees EOF and exits; a
        // crashed server is already gone (this is then a no-op).
        let _ = wired.close.send(());
        // Wait for the reader task to end on the resulting EOF — but ONLY on a
        // deliberate restart. On a crash the reader already terminated (that is
        // what resolved `wired.down` in the select above), so re-awaiting the
        // same completed handle would panic.
        if !crashed && !wait_for_exit("old boundary reader", &mut wired.down).await {
            announce_uncertain_exit(&sessions);
            return;
        }
        // Let the old instance fully exit (release its listen socket) before we
        // reconnect. This exit-only launcher API cannot yet force-kill a stuck
        // child, so an unconfirmed exit is fail-stop: never risk overlapping
        // authoritative worlds/store writers.
        if !wait_for_exit("old server instance", instance.exited).await {
            announce_uncertain_exit(&sessions);
            return;
        }

        // --- RECONNECT: respawn + reconnect, retrying under the crash guard. ---
        let reconnected = loop {
            if guard.tripped() {
                tracing::error!("server restart loop tripped the crash-loop guard; giving up");
                broadcast(&sessions, server_down_notice());
                break None;
            }
            guard.record_attempt();
            tokio::time::sleep(guard.backoff()).await;
            let new_instance = match launcher.launch().await {
                Ok(i) => i,
                Err(e) => {
                    guard.record_failure();
                    tracing::error!(error = %e, "server relaunch failed; retrying");
                    continue;
                }
            };
            match connect_and_wire(&new_instance.endpoint, &sessions, RECONNECT_TIMEOUT, &env).await
            {
                Ok(mut w) => {
                    match resync_boundary(&w.conn, &sessions, &env.input_custody).await {
                        Ok(count) => {
                            tracing::info!(
                                count,
                                "re-announced held sessions; fresh world crossed the readiness barrier"
                            );
                            guard.record_success();
                            break Some((new_instance, w));
                        }
                        Err(e) => {
                            // Hello without this exact barrier is not a successful
                            // reconnect: keep the public boundary in its gap, close
                            // this private attempt, and let the crash guard retry.
                            guard.record_failure();
                            tracing::error!(error = %e, "post-reconnect resync failed; retrying");
                            let _ = w.close.send(());
                            if !wait_for_exit("failed-resync boundary reader", &mut w.down).await {
                                announce_uncertain_exit(&sessions);
                                return;
                            }
                            if !wait_for_exit("failed-resync server instance", new_instance.exited)
                                .await
                            {
                                announce_uncertain_exit(&sessions);
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    guard.record_failure();
                    tracing::error!(error = %e, "reconnect after restart failed; retrying");
                    if !wait_for_exit("failed-connect server instance", new_instance.exited).await {
                        announce_uncertain_exit(&sessions);
                        return;
                    }
                }
            }
        };
        let Some((new_instance, new_wired)) = reconnected else {
            // The guard tripped: leave the boundary in the gap so input stays
            // refused with a clear message, and stop supervising.
            return;
        };

        // --- RESUME (D2): resync + its exact barrier completed above. Only now
        // publish the fresh boundary and tell players input is safe again. ---
        boundary.connect(Arc::clone(&new_wired.conn));
        broadcast(&sessions, reconnected_notice());
        instance = new_instance;
        wired = new_wired;
    }
}

/// The classic welcome banner.
fn banner(name: &str) -> String {
    format!(
        "------------------------------------------------------------------\n\
         \x20  {name}\n\
         \x20  a rustmush server (forked gateway)\n\
         \n\
         \x20  Connect to an existing account, or create a new one:\n\
         ------------------------------------------------------------------"
    )
}

/// Run the gateway: obtain the server half (spawn or connect), handshake, wire
/// the boundary client, bring up the SSH listener, and serve until Ctrl-C.
///
/// For a SUPERVISED source ([`ServerSource::Spawn`]/[`ServerSource::Restartable`])
/// the gateway holds the player connections across a server restart (M16c): on a
/// server exit — a crash (D1a) or a deliberate [`RestartTrigger`] (D1b) — it
/// respawns the server, re-attaches every held online session (D2), and resumes,
/// telling players a "restarting"/"reconnected" line and refusing input for the
/// gap (D3). [`ServerSource::Connect`] is NOT supervised: it wires the boundary
/// once and serves until Ctrl-C, exactly as in M16b.
///
/// # Errors
///
/// Connect/handshake failures, a store open failure, or the SSH bind failing.
pub async fn serve(config: Config, opts: GatewayOptions) -> Result<(), String> {
    // The stable, restart-surviving core: the boundary handle (swapped per
    // connection), the link the bridges hold, and the session manager. None of
    // these are torn down across a server restart.
    let boundary = BoundaryHandle::default();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(config.game.linkdead_grace_s),
        ..SessionConfig::default()
    }));
    // The shared metrics registry (M18): the gateway constructs it (unlike the
    // all-in-one, which passes the world loop's own) so the input-shed, the
    // write-timeout, and the `to_server` high-water land in the SAME registry the
    // web ingress serves over `/metrics` + `/healthz` and the SSH/web bridges
    // count connections into.
    let metrics = Arc::new(Metrics::new());
    let lifecycle = Arc::new(Mutex::new(LifecycleCustody::new(
        config.ipc_to_server_cap(),
    )));
    let input_custody = Arc::new(Mutex::new(InputCustody::default()));
    let input_epoch = rand::random::<u64>();
    let link = Arc::new(SocketGatewayLink {
        boundary: boundary.clone(),
        next_id: AtomicU64::new(1),
        input_epoch,
        next_input_sequence: AtomicU64::new(1),
        input_custody: Arc::clone(&input_custody),
        lifecycle: Arc::clone(&lifecycle),
        sessions: Arc::clone(&sessions),
        metrics: Arc::clone(&metrics),
        to_server_cap: config.ipc_to_server_cap(),
    });
    // The restart-surviving cache of the server's web `hello` manifest,
    // populated at the first handshake below and refreshed on each restart.
    let manifest: ManifestCache = Arc::new(Mutex::new(HelloManifest::default()));
    // The M18 boundary tuning + manifest, threaded into every (re)connect.
    let env = BoundaryEnv {
        manifest: Arc::clone(&manifest),
        lifecycle,
        input_custody,
        tuning: BoundaryTuning {
            metrics: Arc::clone(&metrics),
            to_server_cap: config.ipc_to_server_cap(),
            high_water_pct: config.ipc_high_water_warn_pct(),
            write_timeout: Duration::from_millis(config.ipc_write_timeout_ms()),
        },
    };

    // Obtain + wire the FIRST server boundary. For a supervised source we launch
    // an instance and keep the launcher for respawns; for Connect we wire once.
    let (launcher, first_instance): (Option<Box<dyn ServerLauncher>>, Option<LaunchedServer>) =
        match opts.source {
            ServerSource::Connect(ep) => (
                None,
                Some(LaunchedServer {
                    endpoint: ep,
                    // Nothing to wait on — the sibling server owns its own lifecycle.
                    exited: Box::pin(std::future::pending()),
                }),
            ),
            ServerSource::Spawn { config_path } => {
                let endpoint = config.ipc_endpoint().map_err(|e| e.to_string())?;
                let launcher: Box<dyn ServerLauncher> = Box::new(SpawnLauncher {
                    config_path,
                    endpoint,
                });
                let inst = launcher.launch().await?;
                (Some(launcher), Some(inst))
            }
            ServerSource::Restartable(launcher) => {
                let inst = launcher.launch().await?;
                (Some(launcher), Some(inst))
            }
        };
    let first_instance = first_instance.expect("first instance set above");
    let first_wired = connect_and_wire(
        &first_instance.endpoint,
        &sessions,
        INITIAL_CONNECT_TIMEOUT,
        &env,
    )
    .await?;
    boundary.connect(Arc::clone(&first_wired.conn));

    // The gateway's own KV store (accounts/tokens/bans).
    let store = match opts.store {
        Some(s) => s,
        None => open_store(&config).await?,
    };

    let guard = Arc::new(IpGuard::new());
    if let Ok(bans) = store.get_bans().await {
        guard.load_sitebans(
            bans.into_iter()
                .filter_map(|b| b.ip.parse().ok().map(|ip| (ip, b.until_unix))),
        );
    }

    let link_dyn: Arc<dyn GatewayLink> = link;
    let accounts = Arc::new(Accounts::with_link(
        Arc::clone(&store),
        Arc::clone(&link_dyn),
        config.game.max_characters as usize,
    ));
    let bridge = Arc::new(
        SshBridge::with_link(
            Arc::clone(&accounts),
            Arc::clone(&sessions),
            Arc::clone(&link_dyn),
            Arc::clone(&guard),
        )
        .with_metrics(Arc::clone(&metrics)),
    );

    let data_dir = config.server.data_dir.clone();
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("cannot create data dir {}: {e}", data_dir.display()))?;
    let ssh_listen = config.ssh_listen().map_err(|e| e.to_string())?;
    let listener = start_listener(ssh_listen, &data_dir, banner(&config.server.name), bridge)
        .await
        .map_err(|e| format!("SSH listener on {ssh_listen}: {e}"))?;
    let addr = listener.addr();
    tracing::info!(%addr, "gateway SSH listener up");
    if let Some(tx) = opts.ssh_ready {
        let _ = tx.send(addr);
    }

    // The WEB listener (M16d): the SAME session manager, accounts, guard, store,
    // and boundary link the SSH path uses — so a WS session attaches as a
    // structured ConnectionHandle into the shared SessionManager and re-attaches
    // for free across a server restart (build_attach_recs walks it, the
    // ReAnnounce→ConnUp path re-drives it). The web `hello` manifest comes from
    // the cache captured at the handshake, so web users see the SAME
    // features/panels the all-in-one advertises. A web bind/serve failure is
    // loud but NON-FATAL (W4): SSH players keep playing and the supervisor keeps
    // running while the operator fixes the [web] config.
    let (web_stop, web_join) = {
        let (features, panels) = {
            let m = manifest.lock().expect("manifest cache poisoned");
            (m.features.clone(), m.panels.clone())
        };
        match WebConfig::deserialize(config.web.clone()) {
            Ok(mut web_config) => {
                web_config.server_name = format!(
                    "{} (rustmush {})",
                    config.server.name,
                    env!("CARGO_PKG_VERSION")
                );
                web_config.features = features;
                web_config.panels = panels.clone();
                let web_bridge: Arc<dyn WebHooks> = Arc::new(
                    WebBridge::with_link(
                        Arc::clone(&accounts),
                        Arc::clone(&sessions),
                        Arc::clone(&link_dyn),
                        Arc::clone(&guard),
                        Arc::clone(&store),
                        panels,
                    )
                    .with_metrics(Arc::clone(&metrics)),
                );
                let (web_stop, web_stop_rx) = oneshot::channel::<()>();
                let web_ready = opts.web_ready;
                let join = tokio::spawn(async move {
                    if let Err(e) = mush_net_web::serve_with_shutdown_bound(
                        web_bridge,
                        web_config,
                        web_ready,
                        async move {
                            let _ = web_stop_rx.await;
                        },
                    )
                    .await
                    {
                        // Non-fatal: log and let SSH + the supervisor carry on.
                        tracing::error!(error = %e, "web server failed; SSH continues");
                    }
                });
                (Some(web_stop), Some(join))
            }
            Err(e) => {
                tracing::error!(error = %e, "[web] config invalid; web client disabled (SSH continues)");
                (None, None)
            }
        }
    };

    // The Discord bot (D1/D2): the same accounts, session manager, boundary
    // link, guard and store the SSH and web paths use — a Discord follow is a
    // structured connection of the character's own session, attached through
    // the same `GameBridge` core. Loud but non-fatal, like the web listener.
    let discord = discord_bridge::spawn_bot(
        &config,
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        Arc::clone(&link_dyn),
        Arc::clone(&guard),
        Arc::clone(&store),
    );

    // The restart trigger (D1b): a channel the supervisor drains and a
    // `SIGHUP` handler / the test fills. Handed back through `restart_ready`.
    let (restart_tx, restart_rx) = mpsc::channel::<()>(1);
    let trigger = RestartTrigger { tx: restart_tx };
    if let Some(tx) = opts.restart_ready {
        let _ = tx.send(trigger.clone());
    }
    install_sighup_handler(trigger);

    match launcher {
        Some(launcher) => {
            // Supervised: hold the connections across server restarts (M16c).
            let sup = tokio::spawn(supervise(
                launcher,
                boundary,
                Arc::clone(&sessions),
                restart_rx,
                first_instance,
                first_wired,
                env,
            ));
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("gateway shutting down");
            listener.stop_accepting();
            stop_web(web_stop, web_join).await;
            discord.stop().await;
            sup.abort();
        }
        None => {
            // Non-supervised Connect (M16b): keep the first boundary alive (its
            // `close` sender must not drop, or the writer would close) and serve
            // until Ctrl-C. `first_instance.exited` is `pending`, so nothing
            // respawns; a server-gone EOF surfaces as WorldGone on the next
            // input, exactly as before.
            let _keepalive = first_wired;
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("gateway shutting down");
            listener.stop_accepting();
            stop_web(web_stop, web_join).await;
            discord.stop().await;
        }
    }
    Ok(())
}

/// Stop the web listener gracefully at shutdown: fire its stop signal and wait a
/// bounded window for the serve task to finish (in-flight requests drain). A
/// no-op when the web listener never started (`[web]` config error / bind
/// failure).
async fn stop_web(web_stop: Option<oneshot::Sender<()>>, web_join: Option<JoinHandle<()>>) {
    if let Some(stop) = web_stop {
        let _ = stop.send(());
    }
    if let Some(join) = web_join
        && tokio::time::timeout(Duration::from_secs(5), join)
            .await
            .is_err()
    {
        tracing::warn!("web server did not stop in time; continuing shutdown");
    }
}

/// Install the `SIGHUP` restart handler (D1b/D4): a deploy signals the running
/// gateway to restart its supervised server child (rebuilding from the new
/// binary) WITHOUT dropping player connections. Unix-only — `SIGHUP` does not
/// exist on Windows, where restarts are driven programmatically via
/// [`RestartTrigger`] (the restart-survival test's path) — so the handler is
/// compiled out elsewhere and the trigger simply goes unused.
#[cfg(unix)]
fn install_sighup_handler(trigger: RestartTrigger) {
    tokio::spawn(async move {
        let mut hup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "cannot install SIGHUP handler; deploy-restart disabled");
                return;
            }
        };
        while hup.recv().await.is_some() {
            tracing::info!("SIGHUP received; restarting the server child");
            if !trigger.try_restart() {
                tracing::warn!(
                    "SIGHUP restart dropped (supervisor gone or restart already queued)"
                );
            }
        }
    });
}

/// No-op on non-Unix (Windows dev/prod): `SIGHUP` is Linux-only; restarts there
/// are driven programmatically through [`RestartTrigger`].
#[cfg(not(unix))]
fn install_sighup_handler(_trigger: RestartTrigger) {}

/// Open the gateway's KV store from config (Redis, or in-memory for throwaway
/// runs). Mirrors the server's opener but the gateway only ever touches
/// account/token/ban keys.
async fn open_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    use mush_config::config::StoreBackend;
    match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            let store = mush_store::RedisStore::connect(url)
                .await
                .map_err(|e| format!("cannot reach Redis at {url}: {e}"))?;
            Ok(Arc::new(store))
        }
        StoreBackend::Memory => Ok(Arc::new(mush_store::InMemoryStore::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::atomic::AtomicUsize;

    use mush_core::ObjId;
    use mush_ipc::WireInbound;
    use mush_proto::{ClientCaps, ClientKind, ColorDepth, Envelope};
    use mush_session::{ConnId, ConnectionHandle};

    /// A test [`ToServerTx`] over `tx` with a throwaway registry, the given
    /// input-shed `cap`, and `depth` frames pre-charged onto the gauge (to force
    /// or avoid the input-shed). Returns the wrapper and its registry.
    fn test_tx(
        tx: mpsc::UnboundedSender<ToServer>,
        cap: usize,
        depth: u64,
    ) -> (ToServerTx, Arc<Metrics>, Arc<Mutex<LifecycleCustody>>) {
        let metrics = Arc::new(Metrics::new());
        let gauge = Arc::new(AtomicU64::new(depth));
        let lifecycle = Arc::new(Mutex::new(LifecycleCustody::new(cap)));
        (
            ToServerTx::new(
                tx,
                gauge,
                Arc::clone(&metrics),
                cap,
                80,
                Arc::clone(&lifecycle),
            ),
            metrics,
            lifecycle,
        )
    }

    /// Build a link over a single, already-connected boundary (its own reply
    /// table). The restart machinery swaps the boundary in production; these
    /// unit tests exercise one boundary's request/teardown logic directly. The
    /// input-shed cap is `usize::MAX` (nothing sheds), so the request/teardown
    /// tests are unaffected by M18.
    fn test_link(
        tx: mpsc::UnboundedSender<ToServer>,
        pending: Arc<Mutex<Pending>>,
    ) -> SocketGatewayLink {
        let (to_server, metrics, lifecycle) = test_tx(tx, usize::MAX, 0);
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::new(BoundaryConn {
            tx: to_server,
            pending,
        }));
        SocketGatewayLink {
            boundary,
            next_id: AtomicU64::new(1),
            input_epoch: 7,
            next_input_sequence: AtomicU64::new(1),
            input_custody: Arc::new(Mutex::new(InputCustody::default())),
            lifecycle,
            sessions: Arc::new(SessionManager::new()),
            metrics,
            to_server_cap: usize::MAX,
        }
    }

    /// Build a link whose boundary's `to_server` gauge is FORCED at/over `cap`
    /// (M18), sharing `sessions` (for the bounded input-context burst) and one registry.
    /// Returns the link, the writer-side receiver, and the registry.
    fn shedding_link(
        sessions: Arc<SessionManager>,
        cap: usize,
    ) -> (
        SocketGatewayLink,
        mpsc::UnboundedReceiver<ToServer>,
        Arc<Metrics>,
    ) {
        shedding_link_at_depth(sessions, cap, cap as u64)
    }

    fn shedding_link_at_depth(
        sessions: Arc<SessionManager>,
        cap: usize,
        depth: u64,
    ) -> (
        SocketGatewayLink,
        mpsc::UnboundedReceiver<ToServer>,
        Arc<Metrics>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<ToServer>();
        let (to_server, metrics, lifecycle) = test_tx(tx, cap, depth);
        let pending = SocketGatewayLink::pending_table();
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::new(BoundaryConn {
            tx: to_server,
            pending,
        }));
        let link = SocketGatewayLink {
            boundary,
            next_id: AtomicU64::new(1),
            input_epoch: 7,
            next_input_sequence: AtomicU64::new(1),
            input_custody: Arc::new(Mutex::new(InputCustody::default())),
            lifecycle,
            sessions,
            metrics: Arc::clone(&metrics),
            to_server_cap: cap,
        };
        (link, rx, metrics)
    }

    fn ansi_caps(structured: bool) -> ClientCaps {
        ClientCaps {
            kind: if structured {
                ClientKind::Web
            } else {
                ClientKind::Ssh
            },
            color: ColorDepth::Ansi256,
            width: 80,
            height: 24,
            utf8: true,
            structured,
        }
    }

    fn addr(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
    }

    /// `CloseWebSessions` resolution (D4): only the **structured** (web)
    /// connection of a multi-connected character is closed, it receives the
    /// notice, and its `ConnDown` is reported back as inbound; the SSH
    /// connection is untouched.
    #[tokio::test]
    async fn close_web_sessions_drops_only_structured_conns() {
        let sessions = Arc::new(SessionManager::new());
        let char: u64 = 7;

        // One SSH (ANSI) connection and one web (structured) connection on the
        // same character.
        let (ssh_tx, _ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let ssh = ConnectionHandle::new(ConnId(1), addr(1), ansi_caps(false), ssh_tx);
        let (web_tx, mut web_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let web = ConnectionHandle::new_structured(ConnId(2), addr(2), ansi_caps(true), web_tx);
        sessions.attach(ssh, CharKey(char), "boss".to_string());
        sessions.attach(web, CharKey(char), "boss".to_string());

        let (to_server_raw, mut to_server_rx) = mpsc::unbounded_channel::<ToServer>();
        let (to_server_tx, _metrics, _lifecycle) = test_tx(to_server_raw, usize::MAX, 0);
        let notice = OutputEvent::Disconnect {
            reason: RichText::tag("system", "log in again"),
        };
        close_web_sessions(
            &sessions,
            &to_server_tx,
            vec![char],
            Some(&notice),
            DetachCause::Booted,
        );

        // The web connection is gone; the SSH connection remains.
        assert!(
            sessions.session_of_conn(ConnId(2)).is_none(),
            "web connection must be detached"
        );
        assert!(
            sessions.session_of_conn(ConnId(1)).is_some(),
            "SSH connection must survive @newpassword logout-everywhere"
        );

        // The notice reached the web connection before it closed.
        let env = web_rx.try_recv().expect("web conn got the notice");
        assert!(matches!(env.event, OutputEvent::Disconnect { .. }));

        // Exactly one ConnDown was reported to the server — for the web conn.
        let msg = to_server_rx.try_recv().expect("a ConnDown was reported");
        match msg {
            ToServer::Inbound(WireInbound::ConnDown { conn, reason, .. }) => {
                assert_eq!(conn, 2);
                assert_eq!(reason, DisconnectKind::Boot);
            }
            other => panic!("expected ConnDown for the web conn, got {other:?}"),
        }
        assert!(
            to_server_rx.try_recv().is_err(),
            "only the structured conn should be reported"
        );
    }

    /// Reverse slow-client control is an unclean detach: the last connection
    /// must become link-dead (retaining its resume ring) and report `Lost`, not
    /// be locally destroyed as `Booted`. Its eventual `SessionGone` also
    /// releases the bounded lifecycle obligation.
    #[tokio::test]
    async fn reverse_slow_client_detach_is_linkdead_then_session_gone() {
        let sessions = Arc::new(SessionManager::with_config(SessionConfig {
            linkdead_grace: Duration::ZERO,
            ..SessionConfig::default()
        }));
        let actor = ObjId(7);
        let (web_tx, _web_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let web = ConnectionHandle::new_structured(ConnId(1), addr(1), ansi_caps(true), web_tx);
        let (sess, _) = sessions.attach(web, char_key(actor), "boss".to_string());

        let (to_server_raw, mut to_server_rx) = mpsc::unbounded_channel::<ToServer>();
        let (to_server_tx, _metrics, lifecycle) = test_tx(to_server_raw, 1, 0);
        lifecycle.lock().unwrap().connections.insert(
            1,
            ConnectionCustody {
                actor,
                phase: LifecyclePhase::Live,
                restore_linkdead_on_cancel: false,
            },
        );

        detach_one_conn(&sessions, &to_server_tx, 1, DetachCause::SlowClient);

        assert!(sessions.session_of_conn(ConnId(1)).is_none());
        assert_eq!(
            sessions.session_of_char(char_key(actor)),
            Some(sess),
            "the resumable session must survive its last slow connection"
        );
        assert!(matches!(
            to_server_rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnDown {
                conn: 1,
                actor: reported_actor,
                reason: DisconnectKind::Lost,
            })) if reported_actor == actor
        ));
        assert_eq!(
            lifecycle.lock().unwrap().obligations(),
            1,
            "ConnDown(Lost) converts connection custody into expiry custody"
        );

        let expiry = tokio::time::timeout(Duration::from_secs(1), to_server_rx.recv())
            .await
            .expect("zero-grace reaper must run")
            .expect("reaper boundary remains live");
        assert!(matches!(
            expiry,
            ToServer::Inbound(WireInbound::SessionGone {
                actor: reported_actor,
            }) if reported_actor == actor
        ));
        assert!(sessions.session_of_char(char_key(actor)).is_none());
        assert_eq!(
            lifecycle.lock().unwrap().obligations(),
            0,
            "SessionGone releases the delayed lifecycle obligation"
        );
    }

    /// F3 (no hang on mid-request teardown): a service request parked in the
    /// pending table must resolve to `None` — not block forever — when the
    /// boundary tears down (the reader raises `down` and drops every waiter)
    /// before the server ever replies. This is the `@newpassword` Barrier /
    /// CreatePlayer / `InInputContext`-probe-during-a-server-crash case.
    #[tokio::test(start_paused = true)]
    async fn boundary_teardown_fails_inflight_request() {
        // A live receiver so the request's `to_server` send succeeds and the
        // waiter actually parks (a dead receiver would fail fast without ever
        // parking, testing nothing).
        let (tx, _rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = test_link(tx, Arc::clone(&pending));

        // Park a request; nobody will ever send its Reply.
        let fut = link.request(ServiceReq::Barrier);
        assert_eq!(
            pending.lock().unwrap().waiters.len(),
            1,
            "the request must have parked a waiter"
        );

        // The reader's teardown drain (EOF / error / Bye): raise `down` and drop
        // every parked sender.
        {
            let mut p = pending.lock().unwrap();
            p.down = true;
            p.waiters.clear();
        }

        // The parked request resolves to None instead of hanging.
        let res = tokio::time::timeout(Duration::from_secs(5), fut)
            .await
            .expect("the request must not hang past the boundary teardown");
        assert!(res.is_none(), "a torn-down request must resolve to None");
    }

    /// A reconnect is not player-ready at Hello. The supervisor must first
    /// enqueue the full held roster and then cross an exact barrier on that
    /// SAME FIFO boundary; only that result permits publishing "Reconnected".
    #[tokio::test]
    async fn resync_orders_full_roster_before_readiness_barrier() {
        let sessions = Arc::new(SessionManager::new());
        let (output_tx, _output_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(77), addr(7), ansi_caps(false), output_tx);
        sessions.attach(handle, CharKey(42), "boss".to_string());

        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<ToServer>();
        let (tx, _metrics, _lifecycle) = test_tx(raw_tx, usize::MAX, 0);
        let pending = SocketGatewayLink::pending_table();
        let conn = Arc::new(BoundaryConn {
            tx,
            pending: Arc::clone(&pending),
        });

        let input_custody = Arc::new(Mutex::new(InputCustody::default()));
        let later = GatewayRequest::new(
            91,
            2,
            GatewayInput::Cmd {
                conn: 77,
                actor: ObjId(42),
                line: "second".to_string(),
            },
        );
        let earlier = GatewayRequest::new(
            91,
            1,
            GatewayInput::Cmd {
                conn: 77,
                actor: ObjId(42),
                line: "first".to_string(),
            },
        );
        {
            let mut custody = input_custody.lock().unwrap();
            assert_eq!(
                custody.admit(later.clone()),
                InputCustodyAdmission::Admitted
            );
            assert_eq!(
                custody.admit(earlier.clone()),
                InputCustodyAdmission::Admitted
            );
        }

        let resync = tokio::spawn({
            let conn = Arc::clone(&conn);
            let sessions = Arc::clone(&sessions);
            let input_custody = Arc::clone(&input_custody);
            async move { resync_boundary(&conn, &sessions, &input_custody).await }
        });

        match raw_rx.recv().await.expect("resync emitted its roster") {
            ToServer::ReAnnounce(recs) => {
                assert_eq!(recs.len(), 1);
                assert_eq!(recs[0].conn, 77);
                assert_eq!(recs[0].actor, ObjId(42));
            }
            other => panic!("expected ReAnnounce first, got {other:?}"),
        }
        let barrier_id = match raw_rx.recv().await.expect("resync emitted its barrier") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected Barrier second, got {other:?}"),
        };
        assert_eq!(barrier_id, RESYNC_BARRIER_REQUEST_ID);
        assert!(
            !resync.is_finished(),
            "Hello + roster enqueue must not be mistaken for world readiness"
        );

        let waiter = pending
            .lock()
            .unwrap()
            .waiters
            .remove(&barrier_id)
            .expect("resync barrier parked one exact waiter");
        waiter.send(ServiceResp::Barrier).unwrap();
        assert_eq!(
            raw_rx.recv().await,
            Some(ToServer::Input(earlier)),
            "oldest accepted request replays immediately after readiness"
        );
        assert_eq!(
            raw_rx.recv().await,
            Some(ToServer::Input(later)),
            "restart replay preserves the gateway FIFO"
        );
        assert_eq!(resync.await.unwrap(), Ok(1));
    }

    /// An alive IPC socket with a wedged world must not strand the supervisor
    /// gap forever. The bounded wait fails the resync attempt, after which the
    /// supervisor closes that boundary and its ordinary teardown drain releases
    /// the one parked correlation waiter.
    #[tokio::test(start_paused = true)]
    async fn resync_readiness_barrier_times_out_and_teardown_drains_waiter() {
        let sessions = Arc::new(SessionManager::new());
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<ToServer>();
        let (tx, _metrics, _lifecycle) = test_tx(raw_tx, usize::MAX, 0);
        let pending = SocketGatewayLink::pending_table();
        let conn = Arc::new(BoundaryConn {
            tx,
            pending: Arc::clone(&pending),
        });

        let resync = tokio::spawn({
            let conn = Arc::clone(&conn);
            let sessions = Arc::clone(&sessions);
            let input_custody = Arc::new(Mutex::new(InputCustody::default()));
            async move { resync_boundary(&conn, &sessions, &input_custody).await }
        });
        assert!(matches!(
            raw_rx.recv().await,
            Some(ToServer::ReAnnounce(recs)) if recs.is_empty()
        ));
        assert!(matches!(
            raw_rx.recv().await,
            Some(ToServer::Request {
                id: RESYNC_BARRIER_REQUEST_ID,
                req: ServiceReq::Barrier,
            })
        ));

        tokio::time::advance(RESYNC_TIMEOUT).await;
        let err = resync
            .await
            .unwrap()
            .expect_err("a wedged readiness barrier must time out");
        assert!(err.contains("did not cross"));
        assert_eq!(
            pending.lock().unwrap().waiters.len(),
            1,
            "the exact waiter remains owned by this boundary until teardown"
        );

        // Mirror gateway_reader teardown after the supervisor fires `close`.
        let dropped = {
            let mut p = pending.lock().unwrap();
            p.down = true;
            p.capacity_warned = false;
            p.waiters.drain().count()
        };
        assert_eq!(dropped, 1);
        assert!(pending.lock().unwrap().waiters.is_empty());
    }

    struct CountingLauncher {
        launches: Arc<AtomicUsize>,
    }

    impl ServerLauncher for CountingLauncher {
        fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("a replacement must not be launched".to_string()) })
        }
    }

    /// An EOF is not proof that the old server process has exited. If its exit
    /// future remains pending past `EXIT_WAIT`, the supervisor must stay in the
    /// gap and stop WITHOUT calling the launcher: otherwise a stuck old world
    /// and its replacement could write the authoritative store concurrently.
    #[tokio::test(start_paused = true)]
    async fn uncertain_old_instance_exit_fail_stops_without_respawn() {
        let sessions = Arc::new(SessionManager::new());
        let metrics = Arc::new(Metrics::new());
        let lifecycle = Arc::new(Mutex::new(LifecycleCustody::new(8)));
        let (raw_tx, _raw_rx) = mpsc::unbounded_channel();
        let conn = Arc::new(BoundaryConn {
            tx: ToServerTx::new(
                raw_tx,
                Arc::new(AtomicU64::new(0)),
                Arc::clone(&metrics),
                8,
                80,
                Arc::clone(&lifecycle),
            ),
            pending: SocketGatewayLink::pending_table(),
        });
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::clone(&conn));

        // A completed reader models the crash/EOF trigger, while the instance
        // itself deliberately never reports exit.
        let down = tokio::spawn(async {});
        let (close, _close_rx) = oneshot::channel();
        let first_wired = WiredBoundary { conn, down, close };
        let first_instance = LaunchedServer {
            endpoint: IpcEndpoint::parse("127.0.0.1:1").unwrap(),
            exited: Box::pin(std::future::pending()),
        };
        let launches = Arc::new(AtomicUsize::new(0));
        let launcher: Box<dyn ServerLauncher> = Box::new(CountingLauncher {
            launches: Arc::clone(&launches),
        });
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let env = BoundaryEnv {
            manifest: Arc::new(Mutex::new(HelloManifest::default())),
            tuning: BoundaryTuning {
                metrics,
                to_server_cap: 8,
                high_water_pct: 80,
                write_timeout: Duration::from_secs(1),
            },
            lifecycle,
            input_custody: Arc::new(Mutex::new(InputCustody::default())),
        };

        let supervisor = tokio::spawn(supervise(
            launcher,
            boundary.clone(),
            sessions,
            restart_rx,
            first_instance,
            first_wired,
            env,
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(EXIT_WAIT + Duration::from_millis(1)).await;
        supervisor
            .await
            .expect("fail-stop supervisor must return after the exit deadline");

        assert_eq!(
            launches.load(Ordering::SeqCst),
            0,
            "an unconfirmed old instance must prevent every replacement launch"
        );
        assert!(boundary.current().is_none(), "boundary must remain in Gap");
        assert!(boundary.is_restarting(), "Gap must continue refusing input");
    }

    /// The crash-loop guard (R3): a server that keeps dying fast trips the
    /// guard. Called back-to-back (no healthy gap), the first
    /// `MAX_RESTARTS_IN_WINDOW` restarts are allowed; once that many attempts
    /// occupy the window, another is refused —
    /// the supervisor then gives up and tells players rather than respawning
    /// forever.
    #[test]
    fn crash_guard_trips_after_max_restarts_in_window() {
        let mut guard = CrashGuard::new();
        // Mirror the supervisor: check `tripped()` before each attempt, then
        // record it. Exactly MAX_RESTARTS_IN_WINDOW attempts are admitted.
        for i in 0..MAX_RESTARTS_IN_WINDOW {
            assert!(
                !guard.tripped(),
                "restart {i} within the window must be allowed"
            );
            guard.record_attempt();
        }
        assert!(
            guard.tripped(),
            "an attempt after MAX_RESTARTS_IN_WINDOW must trip the guard"
        );
    }

    /// Window timestamps are rolling state, not a lifetime failure count. A
    /// healthy interval must age an old burst out before the next admission
    /// decision, rather than tripping on stale entries and never reaching the
    /// pruning in `record_attempt`.
    #[test]
    fn crash_guard_prunes_stale_window_before_admission() {
        let mut guard = CrashGuard::new();
        let stale = Instant::now()
            .checked_sub(CRASH_WINDOW + Duration::from_millis(1))
            .expect("test instant supports a short subtraction");
        guard.recent = vec![stale; MAX_RESTARTS_IN_WINDOW];
        guard.consecutive = u32::try_from(MAX_RESTARTS_IN_WINDOW).unwrap();

        assert!(
            !guard.tripped(),
            "stale attempts must be pruned before the guard decides"
        );
        assert!(guard.recent.is_empty());
        guard.record_attempt();
        assert_eq!(guard.consecutive, 1, "a healthy gap resets backoff");
    }

    /// A server that never comes back — every relaunch/reconnect fails, each
    /// possibly burning its full connect deadline so few timestamps land in the
    /// wall-clock window — must still trip the guard on the consecutive-failure
    /// count alone, and a successful reconnect resets that streak.
    #[test]
    fn crash_guard_trips_after_consecutive_failures() {
        let mut guard = CrashGuard::new();
        for i in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(!guard.tripped(), "failed attempt {i} must be allowed");
            guard.record_attempt();
            guard.record_failure();
            // Model attempts whose connect deadlines put them outside the
            // rolling window. The independent consecutive-failure ceiling must
            // still catch them.
            let stale = Instant::now()
                .checked_sub(CRASH_WINDOW + Duration::from_millis(1))
                .expect("test instant supports a short subtraction");
            guard.recent.fill(stale);
        }
        assert!(
            guard.tripped(),
            "MAX_CONSECUTIVE_FAILURES failed attempts must trip the guard"
        );
        // A success clears the failure streak; the deliberately stale window
        // entries are independently pruned by the admission check.
        guard.record_success();
        assert!(
            !guard.tripped(),
            "a successful reconnect must reset the consecutive-failure streak"
        );
    }

    /// The guard's backoff grows with consecutive fast restarts and never
    /// exceeds the cap — so a crash loop backs off instead of hot-spinning,
    /// while a first restart is near-immediate.
    #[test]
    fn crash_guard_backoff_grows_then_caps() {
        let mut guard = CrashGuard::new();
        guard.record_attempt();
        let first = guard.backoff();
        assert_eq!(first, BACKOFF_BASE, "the first backoff is the base");
        let mut prev = first;
        // Drive several more consecutive restarts; each backoff is >= the last
        // and clamped at the cap.
        for _ in 0..10 {
            guard.record_attempt();
            let b = guard.backoff();
            assert!(b >= prev, "backoff must not shrink across fast restarts");
            assert!(b <= BACKOFF_CAP, "backoff must never exceed the cap");
            prev = b;
        }
        assert_eq!(prev, BACKOFF_CAP, "sustained fast restarts reach the cap");
    }

    /// A request issued *after* the boundary has already gone down must fail
    /// fast (never park), closing the race where the reader drains the table
    /// between a request's `down` check and its insert.
    #[tokio::test]
    async fn request_after_boundary_down_fails_fast() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        pending.lock().unwrap().down = true;
        let link = test_link(tx, Arc::clone(&pending));

        let res = link
            .request(ServiceReq::NameFree { name: "x".into() })
            .await;
        assert!(res.is_none(), "a request past a down boundary must be None");
        assert!(
            pending.lock().unwrap().waiters.is_empty(),
            "no waiter should have parked on a down boundary"
        );
        assert!(
            rx.try_recv().is_err(),
            "nothing should have been queued to the writer for a down boundary"
        );
    }

    /// Correlation ids are never allowed to wrap and alias a still-live
    /// waiter. Once the process-local id space is exhausted, requests fail
    /// closed before reserving custody or emitting a wire frame.
    #[tokio::test]
    async fn service_request_id_exhaustion_fails_before_enqueue() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = test_link(tx, Arc::clone(&pending));
        link.next_id.store(u64::MAX, Ordering::Relaxed);

        assert_eq!(
            link.request(ServiceReq::Barrier).await,
            Some(ServiceResp::Unavailable)
        );
        assert!(pending.lock().unwrap().waiters.is_empty());
        assert!(rx.try_recv().is_err());
    }

    /// Correlated requests have their own hard custody ceiling. Reaching it
    /// refuses the next request with a typed Busy response *before* a waiter or
    /// must-deliver wire frame is created; releasing one slot admits the next
    /// request normally and its exact reply still resolves the parked waiter.
    #[tokio::test]
    async fn service_request_waiters_are_bounded_and_refused_before_enqueue() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = test_link(tx, Arc::clone(&pending));
        let mut held_receivers = Vec::with_capacity(MAX_IN_FLIGHT_SERVICE_REQUESTS);
        {
            let mut p = pending.lock().unwrap();
            for id in 0..MAX_IN_FLIGHT_SERVICE_REQUESTS as u64 {
                let (reply, receiver) = oneshot::channel();
                p.waiters.insert(id, reply);
                held_receivers.push(receiver);
            }
        }

        assert_eq!(
            link.request(ServiceReq::Barrier).await,
            Some(ServiceResp::Busy),
            "the request above the ceiling must be refused explicitly"
        );
        assert_eq!(
            pending.lock().unwrap().waiters.len(),
            MAX_IN_FLIGHT_SERVICE_REQUESTS,
            "Busy refusal must not grow waiter custody"
        );
        assert!(
            rx.try_recv().is_err(),
            "a request refused before custody must not create a wire frame"
        );

        // The Busy attempt consumed correlation id 1 before custody refused it,
        // so release the allocator's CURRENT next id. Freeing some other slot
        // would correctly exercise the duplicate-id refusal instead.
        let next_id = link.next_id.load(Ordering::Relaxed);
        pending.lock().unwrap().waiters.remove(&next_id);
        let admitted = link.request(ServiceReq::Barrier);
        let id = match rx.try_recv().expect("admitted request was enqueued") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected a Barrier request, got {other:?}"),
        };
        let waiter = pending
            .lock()
            .unwrap()
            .waiters
            .remove(&id)
            .expect("the admitted request parked a waiter");
        waiter.send(ServiceResp::Barrier).unwrap();
        assert_eq!(admitted.await, Some(ServiceResp::Barrier));
        assert!(
            pending.lock().unwrap().waiters.len() < MAX_IN_FLIGHT_SERVICE_REQUESTS,
            "resolved requests release custody"
        );
    }

    // ---- M18 gateway->server input-shed --------------------------------------

    fn cmd(actor: ObjId, line: &str) -> ToServer {
        ToServer::Inbound(WireInbound::Cmd {
            conn: 1,
            actor,
            line: line.to_string(),
        })
    }

    fn query(actor: ObjId) -> ToServer {
        ToServer::Inbound(WireInbound::Query {
            conn: 1,
            actor,
            id: 9,
            name: "who".into(),
            args: serde_json::json!({}),
        })
    }

    /// The input-shed (M18): with the `to_server` gauge at/over cap, `Cmd` and
    /// `Query` shed. A ConnUp reserved *before* the crossing and all teardown /
    /// admitted-service frames remain must-deliver, while a brand-new ConnUp is
    /// refused at the pre-custody admission point.
    #[tokio::test]
    async fn input_shed_drops_cmd_query_but_never_must_deliver() {
        let sessions = Arc::new(SessionManager::new());
        let (link, mut rx, metrics) = shedding_link_at_depth(Arc::clone(&sessions), 8, 7);
        let actor = ObjId(7);

        // Reserve while there is room, then let unrelated must-deliver work
        // take the writer gauge to the cap. The reservation is the admission
        // boundary: its eventual ConnUp may no longer be shed.
        assert_eq!(
            link.try_reserve_connection(1, actor),
            ConnectionAdmission::Reserved
        );
        assert!(
            link.to_server(ToServer::Control(mush_core::ControlMsg::Dump))
                .await
        );

        // A Cmd at/over cap is shed: Shed outcome, counter bump, nothing queued.
        assert_eq!(
            link.to_server_input(cmd(actor, "say hi")).await,
            InputSend::Shed
        );
        assert_eq!(metrics.ipc_frames_shed_total(), 1);
        // `to_server` surfaces the shed as `false` (→ bridge "busy" notice).
        assert!(!link.to_server(cmd(actor, "say hi")).await);
        assert_eq!(metrics.ipc_frames_shed_total(), 2);
        // A Query is shed too (silent at the bridge, but counted here).
        assert_eq!(link.to_server_input(query(actor)).await, InputSend::Shed);
        assert_eq!(metrics.ipc_frames_shed_total(), 3);
        assert!(matches!(rx.try_recv(), Ok(ToServer::Control(_))));
        assert!(rx.try_recv().is_err(), "shed frames must never enqueue");

        // The already-reserved ConnUp and its teardown STILL enqueue at/over
        // cap. ConnUp remains before ConnDown in the single FIFO.
        assert!(
            link.to_server(ToServer::Inbound(WireInbound::ConnUp {
                conn: 1,
                actor,
                caps_structured: false,
                from_linkdead: false,
            }))
            .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnUp { .. }))
        ));
        assert!(
            link.to_server(ToServer::Inbound(WireInbound::ConnDown {
                conn: 1,
                actor,
                reason: DisconnectKind::Lost,
            }))
            .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnDown { .. }))
        ));

        // New lifecycle custody is refused before local session mutation once
        // the queue is full; no must-deliver frame has been created or dropped.
        assert_eq!(
            link.try_reserve_connection(2, ObjId(8)),
            ConnectionAdmission::Busy
        );

        // A ServiceReq (CreatePlayer) is must-deliver: it enqueues synchronously
        // even over cap (a dropped ServiceReq would hang its oneshot — I1).
        let fut = link.request(ServiceReq::CreatePlayer { name: "neo".into() });
        assert!(matches!(rx.try_recv(), Ok(ToServer::Request { .. })));
        drop(fut); // never resolves (no reply); we only assert it enqueued.

        // The shed counter never moved for the must-deliver frames.
        assert_eq!(metrics.ipc_frames_shed_total(), 3);
    }

    /// Live connections plus delayed link-dead cleanup share one hard custody
    /// budget. Reservations allocate no wire frame, and the first request over
    /// the cap is refused before a caller can mutate its SessionManager.
    #[test]
    fn lifecycle_obligations_are_bounded_before_session_custody() {
        let sessions = Arc::new(SessionManager::new());
        let (link, mut rx, _metrics) = shedding_link_at_depth(sessions, 4, 0);

        for conn in 1..=4 {
            assert_eq!(
                link.try_reserve_connection(conn, ObjId(conn as u32)),
                ConnectionAdmission::Reserved
            );
        }
        assert_eq!(
            link.try_reserve_connection(5, ObjId(5)),
            ConnectionAdmission::Busy
        );
        assert!(
            rx.try_recv().is_err(),
            "reservations must not create lifecycle frames"
        );

        link.cancel_connection_reservation(1);
        assert_eq!(
            link.try_reserve_connection(5, ObjId(5)),
            ConnectionAdmission::Reserved,
            "cancelling unused custody must make exactly one slot available"
        );
    }

    /// Losing the last connection converts its slot into a link-dead cleanup
    /// obligation. It is not reusable by an unrelated actor until SessionGone,
    /// while the same actor may reconnect by converting that exact slot back.
    #[tokio::test]
    async fn linkdead_cleanup_holds_lifecycle_capacity_until_session_gone() {
        let sessions = Arc::new(SessionManager::new());
        let (link, mut rx, _metrics) = shedding_link_at_depth(sessions, 1, 0);
        let actor = ObjId(7);

        assert_eq!(
            link.try_reserve_connection(1, actor),
            ConnectionAdmission::Reserved
        );
        assert!(
            link.to_server(ToServer::Inbound(WireInbound::ConnUp {
                conn: 1,
                actor,
                caps_structured: false,
                from_linkdead: false,
            }))
            .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnUp { .. }))
        ));
        link.boundary
            .current()
            .expect("test boundary")
            .tx
            .depth
            .fetch_sub(1, Ordering::Relaxed);

        assert!(
            link.to_server(ToServer::Inbound(WireInbound::ConnDown {
                conn: 1,
                actor,
                reason: DisconnectKind::Lost,
            }))
            .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnDown { .. }))
        ));
        link.boundary
            .current()
            .expect("test boundary")
            .tx
            .depth
            .fetch_sub(1, Ordering::Relaxed);

        assert_eq!(
            link.try_reserve_connection(2, ObjId(8)),
            ConnectionAdmission::Busy,
            "an unrelated actor cannot consume pending cleanup custody"
        );
        assert_eq!(
            link.try_reserve_connection(2, actor),
            ConnectionAdmission::Reserved,
            "the link-dead actor may convert its own obligation into reconnect custody"
        );
        link.cancel_connection_reservation(2);

        assert!(
            link.to_server(ToServer::Inbound(WireInbound::SessionGone { actor }))
                .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::SessionGone { .. }))
        ));
        link.boundary
            .current()
            .expect("test boundary")
            .tx
            .depth
            .fetch_sub(1, Ordering::Relaxed);
        assert_eq!(
            link.try_reserve_connection(3, ObjId(8)),
            ConnectionAdmission::Reserved,
            "SessionGone releases the cleanup slot"
        );
    }

    /// A transport can disappear while the supervisor has removed the old
    /// boundary. ReAnnounce will omit it, so no wire frame is needed, but the
    /// restart-surviving custody ledger must still observe both loss and expiry.
    #[tokio::test]
    async fn lifecycle_teardown_during_restart_gap_does_not_leak_custody() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = test_link(tx, pending);
        let actor = ObjId(7);

        assert_eq!(
            link.try_reserve_connection(1, actor),
            ConnectionAdmission::Reserved
        );
        assert!(
            link.to_server(ToServer::Inbound(WireInbound::ConnUp {
                conn: 1,
                actor,
                caps_structured: false,
                from_linkdead: false,
            }))
            .await
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::ConnUp { .. }))
        ));

        link.boundary.enter_gap();
        assert!(
            !link
                .to_server(ToServer::Inbound(WireInbound::ConnDown {
                    conn: 1,
                    actor,
                    reason: DisconnectKind::Lost,
                }))
                .await
        );
        assert_eq!(link.lifecycle.lock().unwrap().obligations(), 1);

        assert!(
            !link
                .to_server(ToServer::Inbound(WireInbound::SessionGone { actor }))
                .await
        );
        assert_eq!(link.lifecycle.lock().unwrap().obligations(), 0);
        assert!(
            rx.try_recv().is_err(),
            "gap-time teardown has no stale writer"
        );
    }

    /// A flood-exempt connection's input gets one bounded extra burst window,
    /// but cannot grow the physically unbounded must-deliver FIFO forever.
    #[tokio::test]
    async fn flood_exempt_input_has_a_bounded_burst_window() {
        let sessions = Arc::new(SessionManager::new());
        let actor = ObjId(7);
        sessions.set_flood_exempt(char_key(actor), true);
        let (link, mut rx, metrics) = shedding_link(Arc::clone(&sessions), 8);

        // Over cap, but flood-exempt → enqueued, not shed.
        assert_eq!(
            link.to_server_input(cmd(actor, "pasted line")).await,
            InputSend::Sent
        );
        assert_eq!(metrics.ipc_frames_shed_total(), 0);
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Input(GatewayRequest {
                input: GatewayInput::Cmd { .. },
                ..
            }))
        ));

        let (bounded, mut bounded_rx, metrics) = shedding_link_at_depth(
            Arc::clone(&sessions),
            8,
            (8 * INPUT_CONTEXT_QUEUE_MULTIPLIER) as u64,
        );
        assert_eq!(
            bounded.to_server_input(cmd(actor, "too much paste")).await,
            InputSend::Shed
        );
        assert_eq!(metrics.ipc_frames_shed_total(), 1);
        assert!(bounded_rx.try_recv().is_err());
    }

    #[test]
    fn replay_custody_is_per_connection_bounded_and_exactly_released() {
        let mut custody = InputCustody::default();
        let actor = ObjId(7);
        for sequence in 0..MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION as u64 {
            let request = GatewayRequest::new(
                88,
                sequence,
                GatewayInput::Cmd {
                    conn: 4,
                    actor,
                    line: format!("say {sequence}"),
                },
            );
            assert_eq!(custody.admit(request), InputCustodyAdmission::Admitted);
        }
        assert_eq!(
            custody.admit(GatewayRequest::new(
                88,
                10_000,
                GatewayInput::Cmd {
                    conn: 4,
                    actor,
                    line: "over cap".to_string(),
                },
            )),
            InputCustodyAdmission::Full { warn: true }
        );
        assert_eq!(
            custody.admit(GatewayRequest::new(
                88,
                10_001,
                GatewayInput::Cmd {
                    conn: 4,
                    actor,
                    line: "still over cap".to_string(),
                },
            )),
            InputCustodyAdmission::Full { warn: false }
        );

        let first_uid = GatewayRequest::new(
            88,
            0,
            GatewayInput::Cmd {
                conn: 4,
                actor,
                line: String::new(),
            },
        )
        .uid;
        assert!(custody.remove(first_uid).is_some());
        assert_eq!(
            custody.entries.len(),
            MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION - 1
        );
        assert_eq!(custody.per_connection[&4].count, custody.entries.len());
    }

    #[tokio::test]
    async fn legacy_claim_drops_replay_copy_before_sending_release() {
        let sessions = Arc::new(SessionManager::new());
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<ToServer>();
        let (to_server, _metrics, _lifecycle) = test_tx(raw_tx, 64, 0);
        let transport = SessionDelivery::new(Arc::clone(&sessions), to_server.clone());
        let custody = Arc::new(Mutex::new(InputCustody::default()));
        let request = GatewayRequest::new(
            44,
            1,
            GatewayInput::Cmd {
                conn: 5,
                actor: ObjId(7),
                line: "look".to_string(),
            },
        );
        assert_eq!(
            custody.lock().unwrap().admit(request.clone()),
            InputCustodyAdmission::Admitted
        );

        let (gateway_io, mut server_io) = tokio::io::duplex(64 * 1024);
        let (read_half, _write_half) = tokio::io::split(gateway_io);
        let read_half: BoxRead = Box::new(read_half);
        let reader = FrameReader::new(read_half);
        let reader_task = tokio::spawn(gateway_reader(
            reader,
            transport,
            SocketGatewayLink::pending_table(),
            to_server,
            sessions,
            Arc::new(Mutex::new(HelloManifest::default())),
            Arc::clone(&custody),
        ));

        mush_ipc::write_frame(
            &mut server_io,
            &ToGateway::RequestDisposition {
                request: request.uid,
                disposition: GatewayRequestDisposition::LegacyClaimed,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            raw_rx.recv().await,
            Some(ToServer::GatewayReleased {
                request: request.uid
            })
        );
        assert!(
            custody.lock().unwrap().entries.is_empty(),
            "the replay copy must be gone before legacy execution is authorized"
        );

        mush_ipc::write_frame(&mut server_io, &ToGateway::Bye)
            .await
            .unwrap();
        reader_task.await.unwrap();
    }

    /// A minimal [`AuthBackend`] for the bridge-level notice test.
    /// [`GameBridge::line`] never touches the backend, so every method is a stub.
    struct FakeAuth;
    impl crate::accounts::AuthBackend for FakeAuth {
        async fn verify(&self, _a: &str, _p: &str) -> crate::accounts::AuthOutcome {
            crate::accounts::AuthOutcome::BadCredentials
        }
        async fn characters(&self, _a: &str) -> Vec<crate::accounts::CharSummary> {
            Vec::new()
        }
        async fn create(
            &self,
            _a: &str,
            _p: &str,
            _c: &str,
        ) -> Result<crate::accounts::CharSummary, crate::accounts::CreateErr> {
            Err(crate::accounts::CreateErr::Io("test".into()))
        }
        async fn create_character(
            &self,
            _a: &str,
            _c: &str,
        ) -> Result<crate::accounts::CharSummary, crate::accounts::CreateErr> {
            Err(crate::accounts::CreateErr::Io("test".into()))
        }
    }

    fn answer_request(pending: &Arc<Mutex<Pending>>, id: u64, response: ServiceResp) {
        pending
            .lock()
            .expect("pending table poisoned")
            .waiters
            .remove(&id)
            .expect("request waiter exists")
            .send(response)
            .expect("request receiver remains live");
    }

    /// Connect publication is fenced behind both the exact post-ConnUp
    /// barrier and an engine-side association check. SessionInfo and the
    /// initial world read/`look` may not overtake either proof.
    #[tokio::test]
    async fn enter_game_waits_for_durable_and_engine_connection_readiness() {
        use crate::bridge_common::GameBridge;
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        let (tx, mut wire_rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = Arc::new(test_link(tx, Arc::clone(&pending)));
        let link_dyn: Arc<dyn GatewayLink> = link;
        let bridge = Arc::new(GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        ));
        let (ssh_tx, mut ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(77), addr(7), ansi_caps(false), ssh_tx);
        let entered = tokio::spawn({
            let bridge = Arc::clone(&bridge);
            async move {
                bridge
                    .enter_game(handle, ObjId(7), "boss", "Boss", false)
                    .await
            }
        });

        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnUp {
                conn: 77,
                actor: ObjId(7),
                ..
            }))
        ));
        assert!(
            ssh_rx.try_recv().is_err(),
            "SessionInfo overtook ConnUp fence"
        );

        let barrier_id = match wire_rx.recv().await.expect("barrier request") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected Barrier after ConnUp, got {other:?}"),
        };
        assert!(ssh_rx.try_recv().is_err(), "SessionInfo overtook Barrier");
        answer_request(&pending, barrier_id, ServiceResp::Barrier);

        let ready_id = match wire_rx.recv().await.expect("readiness request") {
            ToServer::Request {
                id,
                req:
                    ServiceReq::ConnectionReady {
                        conn: 77,
                        actor: ObjId(7),
                    },
            } => id,
            other => panic!("expected ConnectionReady after Barrier, got {other:?}"),
        };
        assert!(
            ssh_rx.try_recv().is_err(),
            "SessionInfo overtook engine readiness proof"
        );
        answer_request(&pending, ready_id, ServiceResp::ConnectionReady(true));

        let context_id = match wire_rx.recv().await.expect("context request") {
            ToServer::Request {
                id,
                req: ServiceReq::InInputContext { actor: ObjId(7) },
            } => id,
            other => panic!("expected InInputContext after readiness, got {other:?}"),
        };
        assert!(matches!(
            ssh_rx.recv().await.as_deref(),
            Some(OutputEvent::SessionInfo { .. })
        ));
        answer_request(&pending, context_id, ServiceResp::InInputContext(false));
        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Input(GatewayRequest {
                input: GatewayInput::Cmd {
                    conn: 77,
                    actor: ObjId(7),
                    ref line,
                },
                ..
            })) if line == "look"
        ));
        assert!(entered.await.expect("enter task panicked").is_ok());
    }

    /// A failed exact fence occurs after ConnUp entered lifecycle custody, so
    /// it must produce a compensating clean ConnDown and erase the local
    /// session without ever publishing SessionInfo.
    #[tokio::test]
    async fn enter_game_barrier_failure_compensates_and_removes_fresh_session() {
        use crate::bridge_common::{ConnectErr, GameBridge};
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        let (tx, mut wire_rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = Arc::new(test_link(tx, Arc::clone(&pending)));
        let lifecycle = Arc::clone(&link.lifecycle);
        let link_dyn: Arc<dyn GatewayLink> = link;
        let bridge = Arc::new(GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        ));
        let (ssh_tx, mut ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(78), addr(8), ansi_caps(false), ssh_tx);
        let entered = tokio::spawn({
            let bridge = Arc::clone(&bridge);
            async move {
                bridge
                    .enter_game(handle, ObjId(7), "boss", "Boss", false)
                    .await
            }
        });

        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnUp { conn: 78, .. }))
        ));
        let barrier_id = match wire_rx.recv().await.expect("barrier request") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected Barrier, got {other:?}"),
        };
        answer_request(&pending, barrier_id, ServiceResp::Unavailable);
        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnDown {
                conn: 78,
                actor: ObjId(7),
                reason: DisconnectKind::Boot,
            }))
        ));
        assert_eq!(
            entered.await.expect("enter task panicked"),
            Err(ConnectErr::WorldGone)
        );
        assert_eq!(sessions.connection_count(), 0);
        assert_eq!(sessions.session_count(), 0);
        assert_eq!(lifecycle.lock().unwrap().obligations(), 0);
        assert!(
            ssh_rx.try_recv().is_err(),
            "failed connect published output"
        );
    }

    /// A successful ordering fence is not enough when the world refused or
    /// ignored ConnUp. The explicit negative engine association must unwind
    /// exactly like a failed fence, before local readiness is published.
    #[tokio::test]
    async fn enter_game_untracked_connection_compensates_without_session_info() {
        use crate::bridge_common::{ConnectErr, GameBridge};
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        let (tx, mut wire_rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = Arc::new(test_link(tx, Arc::clone(&pending)));
        let link_dyn: Arc<dyn GatewayLink> = link;
        let bridge = Arc::new(GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        ));
        let (ssh_tx, mut ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(80), addr(10), ansi_caps(false), ssh_tx);
        let entered = tokio::spawn({
            let bridge = Arc::clone(&bridge);
            async move {
                bridge
                    .enter_game(handle, ObjId(7), "boss", "Boss", false)
                    .await
            }
        });

        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnUp { conn: 80, .. }))
        ));
        let barrier_id = match wire_rx.recv().await.expect("barrier request") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected Barrier, got {other:?}"),
        };
        answer_request(&pending, barrier_id, ServiceResp::Barrier);
        let ready_id = match wire_rx.recv().await.expect("readiness request") {
            ToServer::Request {
                id,
                req: ServiceReq::ConnectionReady { conn: 80, .. },
            } => id,
            other => panic!("expected ConnectionReady, got {other:?}"),
        };
        answer_request(&pending, ready_id, ServiceResp::ConnectionReady(false));
        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnDown {
                conn: 80,
                reason: DisconnectKind::Boot,
                ..
            }))
        ));
        assert_eq!(
            entered.await.expect("enter task panicked"),
            Err(ConnectErr::WorldGone)
        );
        assert_eq!(sessions.connection_count(), 0);
        assert_eq!(sessions.session_count(), 0);
        assert!(
            ssh_rx.try_recv().is_err(),
            "failed connect published output"
        );
    }

    /// A failed reconnect fence restores the pre-existing link-dead session
    /// and uses `Lost` for the compensating edge; treating it as a clean boot
    /// would destroy the resume ring/session the reconnect meant to revive.
    #[tokio::test]
    async fn enter_game_barrier_failure_restores_revived_linkdead_session() {
        use crate::bridge_common::{ConnectErr, GameBridge};
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        let (old_tx, _old_rx) = mpsc::channel::<Arc<OutputEvent>>(1);
        let old = ConnectionHandle::new(ConnId(70), addr(7), ansi_caps(false), old_tx);
        let (sess, _) = sessions.attach(old, CharKey(7), "boss".to_string());
        assert!(matches!(
            sessions.detach(ConnId(70), mush_session::DetachReason::Lost),
            Some(mush_session::DetachOutcome::Linkdead { .. })
        ));

        let (tx, mut wire_rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        let link = Arc::new(test_link(tx, Arc::clone(&pending)));
        let link_dyn: Arc<dyn GatewayLink> = link;
        let bridge = Arc::new(GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        ));
        let (ssh_tx, mut ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(79), addr(9), ansi_caps(false), ssh_tx);
        let entered = tokio::spawn({
            let bridge = Arc::clone(&bridge);
            async move {
                bridge
                    .enter_game(handle, ObjId(7), "boss", "Boss", false)
                    .await
            }
        });

        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnUp {
                conn: 79,
                from_linkdead: true,
                ..
            }))
        ));
        let barrier_id = match wire_rx.recv().await.expect("barrier request") {
            ToServer::Request {
                id,
                req: ServiceReq::Barrier,
            } => id,
            other => panic!("expected Barrier, got {other:?}"),
        };
        answer_request(&pending, barrier_id, ServiceResp::Unavailable);
        assert!(matches!(
            wire_rx.recv().await,
            Some(ToServer::Inbound(WireInbound::ConnDown {
                conn: 79,
                actor: ObjId(7),
                reason: DisconnectKind::Lost,
            }))
        ));
        assert_eq!(
            entered.await.expect("enter task panicked"),
            Err(ConnectErr::WorldGone)
        );
        assert_eq!(sessions.connection_count(), 0);
        assert_eq!(sessions.session_count(), 1);
        assert_eq!(sessions.session_of_char(CharKey(7)), Some(sess));
        assert!(sessions.is_linkdead(sess));
        assert!(
            ssh_rx.try_recv().is_err(),
            "failed reconnect published output"
        );
    }

    /// Lifecycle overload is decided before SessionManager mutation. This is
    /// especially important for a link-dead reconnect: attaching and then
    /// rolling back as a clean boot would destroy the session/ring it meant to
    /// revive.
    #[tokio::test]
    async fn lifecycle_busy_refuses_before_local_session_attach() {
        use crate::bridge_common::{ConnectErr, GameBridge};
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        let (link, _rx, _metrics) = shedding_link(Arc::clone(&sessions), 1);
        let link_dyn: Arc<dyn GatewayLink> = Arc::new(link);
        let bridge = GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        );
        let (ssh_tx, _ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let handle = ConnectionHandle::new(ConnId(77), addr(7), ansi_caps(false), ssh_tx);

        assert_eq!(
            bridge
                .enter_game(handle, ObjId(7), "boss", "Boss", false)
                .await,
            Err(ConnectErr::Busy)
        );
        assert_eq!(sessions.connection_count(), 0);
        assert_eq!(sessions.session_count(), 0);
    }

    /// End-to-end (M18): a shed `Cmd` through the shared [`GameBridge::line`]
    /// surfaces the "The server is busy; try again." Sys notice to the player,
    /// and the connection is NOT closed (`LineOutcome::Continue`).
    #[tokio::test]
    async fn line_shed_emits_busy_notice() {
        use crate::bridge_common::{GameBridge, LineOutcome};
        use mush_config::ipguard::IpGuard;

        let sessions = Arc::new(SessionManager::new());
        // Attach an ANSI connection so `actor_of` resolves and the notice has a
        // live connection to reach.
        let (ssh_tx, mut ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let ssh = ConnectionHandle::new(ConnId(1), addr(1), ansi_caps(false), ssh_tx);
        sessions.attach(ssh, CharKey(7), "boss".to_string());

        let (link, _rx, _metrics) = shedding_link(Arc::clone(&sessions), 8);
        let link_dyn: Arc<dyn GatewayLink> = Arc::new(link);
        let bridge = GameBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            link_dyn,
            Arc::new(IpGuard::new()),
        );

        assert_eq!(bridge.line(ConnId(1), "hello").await, LineOutcome::Continue);

        let ev = ssh_rx
            .try_recv()
            .expect("a notice must reach the connection");
        let rendered = format!("{ev:?}");
        assert!(
            matches!(&*ev, OutputEvent::Sys(_)) && rendered.contains("busy"),
            "the shed must surface the busy notice, got {rendered}"
        );
    }
}
