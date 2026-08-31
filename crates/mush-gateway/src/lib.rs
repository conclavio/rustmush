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

use std::collections::HashMap;
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
    AttachRec, BoxRead, BoxWrite, DetachCause, FrameReader, GatewayLink, IPC_VERSION, InputSend,
    IpcEndpoint, ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound,
};
use mush_net_web::{WebConfig, WebHooks};
use mush_proto::{OutputEvent, RichText};
use mush_session::{CharKey, SessionConfig, SessionManager};
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
pub mod delivery;
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

/// The shared table of parked service-request waiters, plus a `down` flag the
/// reader raises when the boundary tears down.
///
/// The `down` flag is what makes a mid-request boundary loss fail *fast* instead
/// of hanging forever (F3): the reader ([`gateway_reader`]) sets it and drops
/// every parked sender under the lock when the server EOFs / errors / says Bye,
/// and [`request`](SocketGatewayLink::request) checks it under the *same* lock
/// before parking — so a request can never register into a table that has
/// already been drained and then await a reply that will never come. Both
/// resolve to `None`, which the bridges map to the WorldGone rejection, exactly
/// as the in-process path resolves when the world task drops its oneshot.
struct Pending {
    waiters: HashMap<u64, oneshot::Sender<ServiceResp>>,
    /// Raised once the boundary is gone; no further waiters may park.
    down: bool,
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
}

impl ToServerTx {
    /// Wrap a fresh boundary's `to_server` sender with a zeroed depth gauge.
    fn new(
        tx: mpsc::UnboundedSender<ToServer>,
        depth: Arc<AtomicU64>,
        metrics: Arc<Metrics>,
        cap: usize,
        high_water_pct: u8,
    ) -> ToServerTx {
        ToServerTx {
            tx,
            depth,
            metrics,
            hw_warned: Arc::new(AtomicBool::new(false)),
            cap,
            high_water_pct,
        }
    }

    /// The current queue occupancy (frames enqueued but not yet written).
    fn depth(&self) -> u64 {
        self.depth.load(Ordering::Relaxed)
    }

    /// Enqueue one MUST-DELIVER frame: bump the gauge (recording the high-water),
    /// then send. Returns `false` only when the receiver is gone (boundary down),
    /// in which case the gauge bump is undone so it never leaks. NEVER sheds — the
    /// input-shed decision lives in [`SocketGatewayLink`] and only touches
    /// `Cmd`/`Query`.
    pub(crate) fn send(&self, msg: ToServer) -> bool {
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
    /// A fresh, empty pending-request table for one boundary connection.
    fn pending_table() -> Arc<Mutex<Pending>> {
        Arc::new(Mutex::new(Pending {
            waiters: HashMap::new(),
            down: false,
        }))
    }

    /// Whether `msg` is a flood-exempt connection's input (an input-context
    /// paste — mail/chargen). Such input bypasses the input-shed exactly as it
    /// bypasses the flood throttle. Only `Cmd`/`Query` carry an actor to check;
    /// everything else returns `false` (it is not sheddable anyway).
    fn is_input_exempt(&self, msg: &ToServer) -> bool {
        let actor = match msg {
            ToServer::Inbound(WireInbound::Cmd { actor, .. })
            | ToServer::Inbound(WireInbound::Query { actor, .. }) => *actor,
            _ => return false,
        };
        self.sessions.is_char_flood_exempt(char_key(actor))
    }

    /// The shared send + input-shed decision behind both
    /// [`to_server`](GatewayLink::to_server) and
    /// [`to_server_input`](GatewayLink::to_server_input) (M18). ONLY
    /// `Cmd`/`Query` are sheddable: at/over `to_server_cap` (and not flood-exempt)
    /// they are DROPPED — counted, logged, high-water recorded — and never
    /// enqueued. Every must-deliver frame (lifecycle, control, `ReAnnounce`, ...)
    /// always enqueues (I1). A missing boundary (restart gap / shutdown) resolves
    /// to [`InputSend::Gone`].
    fn classify_send(&self, msg: ToServer) -> InputSend {
        let Some(conn) = self.boundary.current() else {
            return InputSend::Gone;
        };
        let sheddable = matches!(
            &msg,
            ToServer::Inbound(WireInbound::Cmd { .. })
                | ToServer::Inbound(WireInbound::Query { .. })
        );
        if sheddable {
            let depth = conn.tx.depth();
            if depth >= self.to_server_cap as u64 && !self.is_input_exempt(&msg) {
                let is_cmd = matches!(&msg, ToServer::Inbound(WireInbound::Cmd { .. }));
                self.metrics.ipc_frame_shed();
                // Record the true occupancy that triggered the shed.
                self.metrics.observe_ipc_to_server_depth(depth);
                tracing::warn!(
                    dir = "to_server",
                    queue_len = depth,
                    cap = self.to_server_cap,
                    dropped_total = self.metrics.ipc_frames_shed_total(),
                    kind = if is_cmd { "cmd" } else { "query" },
                    "ipc queue full; shedding input frame"
                );
                return InputSend::Shed;
            }
        }
        if conn.tx.send(msg) {
            InputSend::Sent
        } else {
            InputSend::Gone
        }
    }
}

impl GatewayLink for SocketGatewayLink {
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

    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        // Resolve the CURRENT boundary once; the whole request (park + send +
        // teardown-drain) is scoped to that one connection's table, so a restart
        // that swaps the boundary can never leave a waiter parked in a table the
        // new connection's reader will never drain.
        let Some(conn) = self.boundary.current() else {
            return Box::pin(async move { None });
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (otx, orx) = oneshot::channel();
        // Park the waiter BEFORE sending so a fast reply can never race ahead
        // of the registration — but only if this boundary is still up. The
        // `down` check and the insert happen under the SAME lock the reader's
        // teardown drain takes, so a request can never slip into a table that
        // was already drained (which would then await a reply that can never
        // arrive). If the boundary is gone we skip the send entirely and
        // resolve to `None` (→ WorldGone), matching the in-process path.
        let registered = {
            let mut p = conn.pending.lock().expect("pending table poisoned");
            if p.down {
                false
            } else {
                p.waiters.insert(id, otx);
                true
            }
        };
        // A `ServiceReq` is MUST-DELIVER (it has a parked oneshot); it always
        // enqueues through `ToServerTx::send` and is never shed (I1).
        let sent = registered && conn.tx.send(ToServer::Request { id, req });
        let pending = Arc::clone(&conn.pending);
        Box::pin(async move {
            if !sent {
                if registered {
                    pending
                        .lock()
                        .expect("pending table poisoned")
                        .waiters
                        .remove(&id);
                }
                return None;
            }
            orx.await.ok()
        })
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
                let waiter = pending
                    .lock()
                    .expect("pending table poisoned")
                    .waiters
                    .remove(&id);
                if let Some(otx) = waiter {
                    let _ = otx.send(resp);
                } else {
                    tracing::warn!(id, "reply for unknown request id");
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
                tracing::info!(proto, server, "late Hello (already handshaked); refreshing manifest");
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
    sessions: &SessionManager,
    to_server_tx: &ToServerTx,
    char: u64,
    cause: DetachCause,
) {
    let actor = char_key_obj(CharKey(char));
    let reason = cause_to_kind(cause);
    let Some(sess) = sessions.session_of_char(CharKey(char)) else {
        return;
    };
    for handle in sessions.close_session(sess) {
        let _ = to_server_tx.send(ToServer::Inbound(WireInbound::ConnDown {
            conn: handle.id.0,
            actor,
            reason,
        }));
        // `handle` drops here → its output sender closes → the transport pump
        // closes the socket once its queue drains.
    }
}

/// Detach one specific connection (`@newpassword`/slow-client in split mode),
/// reporting its `ConnDown` back to the server.
fn detach_one_conn(
    sessions: &SessionManager,
    to_server_tx: &ToServerTx,
    conn: u64,
    cause: DetachCause,
) {
    let conn_id = mush_session::ConnId(conn);
    let actor = sessions
        .session_of_conn(conn_id)
        .and_then(|s| sessions.character_of(s))
        .map(char_key_obj);
    if sessions
        .detach(conn_id, mush_session::DetachReason::Booted)
        .is_none()
    {
        return;
    }
    if let Some(actor) = actor {
        let _ = to_server_tx.send(ToServer::Inbound(WireInbound::ConnDown {
            conn,
            actor,
            reason: cause_to_kind(cause),
        }));
    }
}

/// Account/char-addressed logout-everywhere (`@newpassword` in split mode):
/// close every **structured** (web/token) connection of `chars`, delivering
/// `notice` to each just before it drops, and report each resulting `ConnDown`
/// back to the server as inbound. SSH connections (session-authenticated) are
/// left untouched. Resolves the whole thing locally on the gateway's own
/// `SessionManager`, so the server never needs a reverse session query (D4).
fn close_web_sessions(
    sessions: &SessionManager,
    to_server_tx: &ToServerTx,
    chars: Vec<u64>,
    notice: Option<&mush_proto::OutputEvent>,
    cause: DetachCause,
) {
    let reason = cause_to_kind(cause);
    for char in chars {
        let actor = char_key_obj(CharKey(char));
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
            // Dropping the handle (inside `detach`) closes its transport once
            // the notice drains.
            if sessions.detach(conn, mush_session::DetachReason::Booted).is_some() {
                let _ = to_server_tx.send(ToServer::Inbound(WireInbound::ConnDown {
                    conn: conn.0,
                    actor,
                    reason,
                }));
            }
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
/// endpoint (D1). `kill_on_drop` is armed on the child so a gateway exit — or a
/// dropped [`LaunchedServer::exited`] future — reaps it.
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
            // dies; `kill_on_drop` (set at spawn) reaps it if the future is
            // dropped mid-restart.
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
            *manifest.lock().expect("manifest cache poisoned") =
                HelloManifest { features, panels };
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

/// The resync notice broadcast once the server is back and re-attached (D3),
/// just before the server pushes each session its fresh room render (D2).
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
/// How long to wait for an old instance to fully exit before proceeding anyway
/// (its `kill_on_drop` reaps a truly stuck child).
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

impl CrashGuard {
    fn new() -> CrashGuard {
        CrashGuard {
            recent: Vec::new(),
            consecutive: 0,
            consecutive_failures: 0,
        }
    }

    /// Whether the loop has tripped the guard and must give up. Trips on EITHER
    /// too many fast attempts inside the wall-clock window OR too many
    /// consecutive failed attempts (which may each burn the full connect
    /// deadline, so they slip through the window).
    fn tripped(&self) -> bool {
        self.recent.len() > MAX_RESTARTS_IN_WINDOW
            || self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
    }

    /// Record that a respawn attempt is about to run (drives the window + the
    /// growing backoff). Call once per attempt, before the launch.
    fn record_attempt(&mut self) {
        let now = Instant::now();
        let healthy_gap = self
            .recent
            .last()
            .is_none_or(|&last| now.duration_since(last) > CRASH_WINDOW);
        if healthy_gap {
            // A restart after a healthy run is not part of a loop.
            self.consecutive = 0;
        }
        self.recent.retain(|&t| now.duration_since(t) <= CRASH_WINDOW);
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
        BACKOFF_BASE
            .saturating_mul(1u32 << shift)
            .min(BACKOFF_CAP)
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
        if !crashed {
            let _ = (&mut wired.down).await;
        }
        // Let the old instance fully exit (release its listen socket) before we
        // reconnect — bounded, since `kill_on_drop` backstops a stuck child.
        let _ = tokio::time::timeout(EXIT_WAIT, instance.exited).await;

        // --- RECONNECT: respawn + reconnect, retrying under the crash guard. ---
        let reconnected = loop {
            if guard.tripped() {
                tracing::error!(
                    "server restart loop tripped the crash-loop guard; giving up"
                );
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
            match connect_and_wire(&new_instance.endpoint, &sessions, RECONNECT_TIMEOUT, &env).await {
                Ok(w) => {
                    guard.record_success();
                    break Some((new_instance, w));
                }
                Err(e) => {
                    guard.record_failure();
                    tracing::error!(error = %e, "reconnect after restart failed; retrying");
                    let _ = tokio::time::timeout(EXIT_WAIT, new_instance.exited).await;
                }
            }
        };
        let Some((new_instance, new_wired)) = reconnected else {
            // The guard tripped: leave the boundary in the gap so input stays
            // refused with a clear message, and stop supervising.
            return;
        };

        // --- RESYNC (D2): boundary back up, re-attach every held session. ---
        boundary.connect(Arc::clone(&new_wired.conn));
        broadcast(&sessions, reconnected_notice());
        let recs = build_attach_recs(&sessions);
        if !recs.is_empty() {
            let count = recs.len();
            if new_wired.conn.tx.send(ToServer::ReAnnounce(recs)) {
                tracing::info!(count, "re-announced held sessions to the new server");
            }
        }
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
    let link = Arc::new(SocketGatewayLink {
        boundary: boundary.clone(),
        next_id: AtomicU64::new(1),
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
            ServerSource::Connect(ep) => (None, Some(LaunchedServer {
                endpoint: ep,
                // Nothing to wait on — the sibling server owns its own lifecycle.
                exited: Box::pin(std::future::pending()),
            })),
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
        guard.load_sitebans(bans.into_iter().filter_map(|b| {
            b.ip.parse().ok().map(|ip| (ip, b.until_unix))
        }));
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
        && tokio::time::timeout(Duration::from_secs(5), join).await.is_err()
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
                tracing::warn!("SIGHUP restart dropped (supervisor gone or restart already queued)");
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
    ) -> (ToServerTx, Arc<Metrics>) {
        let metrics = Arc::new(Metrics::new());
        let gauge = Arc::new(AtomicU64::new(depth));
        (
            ToServerTx::new(tx, gauge, Arc::clone(&metrics), cap, 80),
            metrics,
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
        let (to_server, metrics) = test_tx(tx, usize::MAX, 0);
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::new(BoundaryConn {
            tx: to_server,
            pending,
        }));
        SocketGatewayLink {
            boundary,
            next_id: AtomicU64::new(1),
            sessions: Arc::new(SessionManager::new()),
            metrics,
            to_server_cap: usize::MAX,
        }
    }

    /// Build a link whose boundary's `to_server` gauge is FORCED at/over `cap`
    /// (M18), sharing `sessions` (for the flood-exempt bypass) and one registry.
    /// Returns the link, the writer-side receiver, and the registry.
    fn shedding_link(
        sessions: Arc<SessionManager>,
        cap: usize,
    ) -> (
        SocketGatewayLink,
        mpsc::UnboundedReceiver<ToServer>,
        Arc<Metrics>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<ToServer>();
        // Pre-charge the gauge to `cap` so the very next Cmd/Query is at/over it.
        let (to_server, metrics) = test_tx(tx, cap, cap as u64);
        let pending = SocketGatewayLink::pending_table();
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::new(BoundaryConn {
            tx: to_server,
            pending,
        }));
        let link = SocketGatewayLink {
            boundary,
            next_id: AtomicU64::new(1),
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
        let sessions = SessionManager::new();
        let char: u64 = 7;

        // One SSH (ANSI) connection and one web (structured) connection on the
        // same character.
        let (ssh_tx, _ssh_rx) = mpsc::channel::<Arc<OutputEvent>>(8);
        let ssh = ConnectionHandle::new(ConnId(1), addr(1), ansi_caps(false), ssh_tx);
        let (web_tx, mut web_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let web =
            ConnectionHandle::new_structured(ConnId(2), addr(2), ansi_caps(true), web_tx);
        sessions.attach(ssh, CharKey(char), "boss".to_string());
        sessions.attach(web, CharKey(char), "boss".to_string());

        let (to_server_raw, mut to_server_rx) = mpsc::unbounded_channel::<ToServer>();
        let (to_server_tx, _metrics) = test_tx(to_server_raw, usize::MAX, 0);
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

    /// The crash-loop guard (R3): a server that keeps dying fast trips the
    /// guard. Called back-to-back (no healthy gap), the first
    /// `MAX_RESTARTS_IN_WINDOW` restarts are allowed and the next is refused —
    /// the supervisor then gives up and tells players rather than respawning
    /// forever.
    #[test]
    fn crash_guard_trips_after_max_restarts_in_window() {
        let mut guard = CrashGuard::new();
        // Mirror the supervisor: check `tripped()` before each attempt, then
        // record it. The attempt past MAX_RESTARTS_IN_WINDOW must trip.
        for i in 0..=MAX_RESTARTS_IN_WINDOW {
            assert!(
                !guard.tripped(),
                "restart {i} within the window must be allowed"
            );
            guard.record_attempt();
        }
        assert!(
            guard.tripped(),
            "the restart past MAX_RESTARTS_IN_WINDOW must trip the guard"
        );
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
        }
        assert!(
            guard.tripped(),
            "MAX_CONSECUTIVE_FAILURES failed attempts must trip the guard"
        );
        // A success clears the failure streak (and the window still holds only
        // MAX_CONSECUTIVE_FAILURES == MAX_RESTARTS_IN_WINDOW timestamps, not more,
        // so the window bound is not tripped either).
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

        let res = link.request(ServiceReq::NameFree { name: "x".into() }).await;
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

    /// The input-shed (M18): with the `to_server` gauge forced at/over cap, a
    /// `Cmd` is shed (`Shed` + counter++ + nothing enqueued) and a `Query` is
    /// shed too — but every MUST-DELIVER frame (`ConnDown`, `ConnUp`, a
    /// `ServiceReq`) STILL enqueues (I1). `to_server` maps the shed to `false`.
    #[tokio::test]
    async fn input_shed_drops_cmd_query_but_never_must_deliver() {
        let sessions = Arc::new(SessionManager::new());
        let (link, mut rx, metrics) = shedding_link(Arc::clone(&sessions), 8);
        let actor = ObjId(7);

        // A Cmd at/over cap is shed: Shed outcome, counter bump, nothing queued.
        assert_eq!(link.to_server_input(cmd(actor, "say hi")).await, InputSend::Shed);
        assert_eq!(metrics.ipc_frames_shed_total(), 1);
        // `to_server` surfaces the shed as `false` (→ bridge "busy" notice).
        assert!(!link.to_server(cmd(actor, "say hi")).await);
        assert_eq!(metrics.ipc_frames_shed_total(), 2);
        // A Query is shed too (silent at the bridge, but counted here).
        assert_eq!(link.to_server_input(query(actor)).await, InputSend::Shed);
        assert_eq!(metrics.ipc_frames_shed_total(), 3);
        assert!(rx.try_recv().is_err(), "shed frames must never enqueue");

        // MUST-DELIVER lifecycle frames STILL enqueue at/over cap (I1).
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

        // A ServiceReq (CreatePlayer) is must-deliver: it enqueues synchronously
        // even over cap (a dropped ServiceReq would hang its oneshot — I1).
        let fut = link.request(ServiceReq::CreatePlayer { name: "neo".into() });
        assert!(matches!(rx.try_recv(), Ok(ToServer::Request { .. })));
        drop(fut); // never resolves (no reply); we only assert it enqueued.

        // The shed counter never moved for the must-deliver frames.
        assert_eq!(metrics.ipc_frames_shed_total(), 3);
    }

    /// A flood-exempt connection's input (an input-context paste — mail/chargen)
    /// is NEVER shed, even with the gauge over cap — mirroring how it bypasses
    /// the flood throttle.
    #[tokio::test]
    async fn flood_exempt_input_is_never_shed() {
        let sessions = Arc::new(SessionManager::new());
        let actor = ObjId(7);
        sessions.set_flood_exempt(char_key(actor), true);
        let (link, mut rx, metrics) = shedding_link(Arc::clone(&sessions), 8);

        // Over cap, but flood-exempt → enqueued, not shed.
        assert_eq!(link.to_server_input(cmd(actor, "pasted line")).await, InputSend::Sent);
        assert_eq!(metrics.ipc_frames_shed_total(), 0);
        assert!(matches!(
            rx.try_recv(),
            Ok(ToServer::Inbound(WireInbound::Cmd { .. }))
        ));
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

        let ev = ssh_rx.try_recv().expect("a notice must reach the connection");
        let rendered = format!("{ev:?}");
        assert!(
            matches!(&*ev, OutputEvent::Sys(_)) && rendered.contains("busy"),
            "the shed must surface the busy notice, got {rendered}"
        );
    }
}
