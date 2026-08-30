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
use mush_core::DisconnectKind;
use mush_ipc::{
    AttachRec, BoxRead, BoxWrite, DetachCause, FrameReader, GatewayLink, IPC_VERSION, IpcEndpoint,
    ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound,
};
use mush_proto::{OutputEvent, RichText};
use mush_session::{CharKey, SessionConfig, SessionManager};
use mush_store::ObjectStore;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::accounts::Accounts;
use crate::delivery::{SessionDelivery, char_key_obj};
use crate::ssh_bridge::{SshBridge, start_listener};

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

/// One live server boundary: the current connection's `ToServer` writer inbox
/// plus its own correlated-request table. Swapped as a unit each (re)connect so
/// a stale reply table is never reused across a restart (M16c). A fresh table
/// per connection means [`request`](SocketGatewayLink::request) and the reader's
/// teardown drain always agree on the SAME table for one server lifetime.
struct BoundaryConn {
    tx: mpsc::UnboundedSender<ToServer>,
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

/// The gateway-side [`GatewayLink`]: serializes every gateway → server message
/// onto the CURRENT boundary socket (via the swappable [`BoundaryHandle`]) and
/// correlates [`ServiceReq`]/[`ServiceResp`] pairs by a monotonic id. A restart
/// swaps the boundary underneath it; the bridges hold the same
/// `Arc<dyn GatewayLink>` for the gateway's whole lifetime.
pub struct SocketGatewayLink {
    boundary: BoundaryHandle,
    next_id: AtomicU64,
}

impl SocketGatewayLink {
    /// A fresh, empty pending-request table for one boundary connection.
    fn pending_table() -> Arc<Mutex<Pending>> {
        Arc::new(Mutex::new(Pending {
            waiters: HashMap::new(),
            down: false,
        }))
    }
}

impl GatewayLink for SocketGatewayLink {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        // No live boundary (restart gap) → `false`, which the bridges map to the
        // WorldGone rejection, exactly as at shutdown.
        let ok = self
            .boundary
            .current()
            .is_some_and(|c| c.tx.send(msg).is_ok());
        Box::pin(async move { ok })
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
        let sent = registered && conn.tx.send(ToServer::Request { id, req }).is_ok();
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
        if let Err(e) = mush_ipc::write_frame(&mut w, &msg).await {
            tracing::warn!(error = %e, "ipc write to server failed; boundary down");
            return;
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
    to_server_tx: mpsc::UnboundedSender<ToServer>,
    sessions: Arc<SessionManager>,
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
            ToGateway::Hello { proto, server, .. } => {
                tracing::info!(proto, server, "late Hello (already handshaked)");
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
    to_server_tx: &mpsc::UnboundedSender<ToServer>,
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
    to_server_tx: &mpsc::UnboundedSender<ToServer>,
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
    to_server_tx: &mpsc::UnboundedSender<ToServer>,
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
) -> Result<WiredBoundary, String> {
    let (reader_half, writer_half) = connect_with_retry(endpoint, connect_timeout).await?;

    // Handshake: the server greets with Hello before anything else.
    let mut reader = FrameReader::new(reader_half);
    match reader
        .read::<ToGateway>()
        .await
        .map_err(|e| format!("reading Hello: {e}"))?
    {
        Some(ToGateway::Hello { proto, server, .. }) => {
            if proto != IPC_VERSION {
                return Err(format!(
                    "ipc version mismatch: server speaks {proto}, gateway speaks {IPC_VERSION}"
                ));
            }
            tracing::info!(proto, server, "handshake ok");
        }
        Some(other) => return Err(format!("expected Hello, got {other:?}")),
        None => return Err("server closed the boundary before Hello".to_string()),
    }

    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel::<ToServer>();
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
    tokio::spawn(gateway_writer(writer_half, to_server_rx, close_rx));
    let down = tokio::spawn(gateway_reader(
        reader,
        transport,
        pending,
        to_server_tx,
        Arc::clone(sessions),
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
            match connect_and_wire(&new_instance.endpoint, &sessions, RECONNECT_TIMEOUT).await {
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
            if new_wired.conn.tx.send(ToServer::ReAnnounce(recs)).is_ok() {
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
    let link = Arc::new(SocketGatewayLink {
        boundary: boundary.clone(),
        next_id: AtomicU64::new(1),
    });
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(config.game.linkdead_grace_s),
        ..SessionConfig::default()
    }));

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
    let first_wired =
        connect_and_wire(&first_instance.endpoint, &sessions, INITIAL_CONNECT_TIMEOUT).await?;
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
    let bridge = Arc::new(SshBridge::with_link(
        accounts,
        Arc::clone(&sessions),
        Arc::clone(&link_dyn),
        Arc::clone(&guard),
    ));

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
            ));
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("gateway shutting down");
            listener.stop_accepting();
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
        }
    }
    Ok(())
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

    use mush_ipc::WireInbound;
    use mush_proto::{ClientCaps, ClientKind, ColorDepth, Envelope};
    use mush_session::{ConnId, ConnectionHandle};

    /// Build a link over a single, already-connected boundary (its own reply
    /// table). The restart machinery swaps the boundary in production; these
    /// unit tests exercise one boundary's request/teardown logic directly.
    fn test_link(
        tx: mpsc::UnboundedSender<ToServer>,
        pending: Arc<Mutex<Pending>>,
    ) -> SocketGatewayLink {
        let boundary = BoundaryHandle::default();
        boundary.connect(Arc::new(BoundaryConn { tx, pending }));
        SocketGatewayLink {
            boundary,
            next_id: AtomicU64::new(1),
        }
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

        let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel::<ToServer>();
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
}
