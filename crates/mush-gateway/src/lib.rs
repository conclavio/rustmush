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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::DisconnectKind;
use mush_ipc::{
    BoxRead, BoxWrite, DetachCause, FrameReader, IPC_VERSION, IpcEndpoint, ServiceReq, ServiceResp,
    ToGateway, ToServer, WireInbound,
};
use mush_session::{CharKey, SessionConfig, SessionManager};
use mush_store::ObjectStore;
use tokio::sync::{mpsc, oneshot};

use mushd::accounts::Accounts;
use mushd::config::Config;
use mushd::ipguard::IpGuard;
use mushd::link::{GatewayLink, InProcessTransport};
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::world_task::char_key_obj;

/// Where the server half comes from.
pub enum ServerSource {
    /// Production: spawn a `mush-server` child process (the supervisor) and
    /// connect to the `[ipc].listen` endpoint it binds.
    Spawn {
        /// The config path to pass the child (`--config`).
        config_path: PathBuf,
    },
    /// Test / attach: connect to an already-running server at this endpoint,
    /// spawning nothing (the two-process integration test runs the server as a
    /// sibling tokio task).
    Connect(IpcEndpoint),
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

/// The gateway-side [`GatewayLink`]: serializes every gateway → server message
/// onto the boundary socket and correlates [`ServiceReq`]/[`ServiceResp`] pairs
/// by a monotonic id.
pub struct SocketGatewayLink {
    tx: mpsc::UnboundedSender<ToServer>,
    pending: Arc<Mutex<Pending>>,
    next_id: AtomicU64,
}

impl SocketGatewayLink {
    /// The shared pending-request table the reader task resolves replies into.
    fn pending_table() -> Arc<Mutex<Pending>> {
        Arc::new(Mutex::new(Pending {
            waiters: HashMap::new(),
            down: false,
        }))
    }
}

impl GatewayLink for SocketGatewayLink {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        let ok = self.tx.send(msg).is_ok();
        Box::pin(async move { ok })
    }

    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (otx, orx) = oneshot::channel();
        // Park the waiter BEFORE sending so a fast reply can never race ahead
        // of the registration — but only if the boundary is still up. The
        // `down` check and the insert happen under the SAME lock the reader's
        // teardown drain takes, so a request can never slip into a table that
        // was already drained (which would then await a reply that can never
        // arrive). If the boundary is gone we skip the send entirely and
        // resolve to `None` (→ WorldGone), matching the in-process path.
        let registered = {
            let mut p = self.pending.lock().expect("pending table poisoned");
            if p.down {
                false
            } else {
                p.waiters.insert(id, otx);
                true
            }
        };
        let sent = registered && self.tx.send(ToServer::Request { id, req }).is_ok();
        let pending = Arc::clone(&self.pending);
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
}

/// The socket writer: forward each queued [`ToServer`] to the server. Exits
/// when every sender is dropped (gateway shutting down).
async fn gateway_writer(mut w: BoxWrite, mut rx: mpsc::UnboundedReceiver<ToServer>) {
    while let Some(msg) = rx.recv().await {
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
    transport: InProcessTransport,
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

/// Wait on the `mush-server` child and log its exit (D3). Owning the child
/// handle here keeps `kill_on_drop` armed for the gateway's whole lifetime; a
/// child exit is logged with its status. M16b does NOT restart it — the
/// WorldGone reject path already handles input in flight when the boundary
/// drops — but the supervisor is structured so M16c can add restart + a
/// `ReAnnounce` of the gateway's live attachments here.
async fn supervise_child(mut child: tokio::process::Child) {
    match child.wait().await {
        Ok(status) => {
            tracing::warn!(
                %status,
                "mush-server child exited; the boundary is down and in-flight \
                 input is rejected (WorldGone). Restart + re-attach is M16c."
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed waiting on the mush-server child");
        }
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
/// the boundary client, then serve SSH connections until Ctrl-C or the server
/// closes the boundary.
///
/// # Errors
///
/// Connect/handshake failures, a store open failure, or the SSH bind failing.
pub async fn serve(config: Config, opts: GatewayOptions) -> Result<(), String> {
    let endpoint = match &opts.source {
        ServerSource::Connect(ep) => ep.clone(),
        ServerSource::Spawn { .. } => config.ipc_endpoint().map_err(|e| e.to_string())?,
    };

    // Supervisor: spawn the server child first, so it is binding while we retry
    // the connect, then hand it to a task that waits on its exit (D3). The
    // supervisor task owns the child handle (kill_on_drop), so it stays alive
    // for the gateway's lifetime and the child is killed if the gateway exits.
    // M16b only logs a child exit; the in-flight input is already rejected via
    // the WorldGone path (the boundary writer fails once the child is gone, so
    // `SocketGatewayLink::to_server` returns false). Restart + re-attach is M16c.
    if let ServerSource::Spawn { config_path } = &opts.source {
        let child = spawn_server_child(config_path, &endpoint)?;
        tokio::spawn(supervise_child(child));
    }

    let (reader_half, writer_half) =
        connect_with_retry(&endpoint, Duration::from_secs(10)).await?;

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

    // The boundary client: writer drains ToServer, reader routes ToGateway.
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel::<ToServer>();
    let pending = SocketGatewayLink::pending_table();
    let link = Arc::new(SocketGatewayLink {
        tx: to_server_tx.clone(),
        pending: Arc::clone(&pending),
        next_id: AtomicU64::new(1),
    });

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(config.game.linkdead_grace_s),
        ..SessionConfig::default()
    }));
    // D5: the gateway owns the connection buffers, so its delivery transport
    // self-polices slow clients and reaps link-dead sessions, feeding the
    // resulting `ConnDown`/`SessionGone` back to the server as inbound over the
    // boundary writer.
    let mut transport = InProcessTransport::new(Arc::clone(&sessions));
    transport.with_boundary_feedback(to_server_tx.clone());

    tokio::spawn(gateway_writer(writer_half, to_server_rx));
    tokio::spawn(gateway_reader(
        reader,
        transport,
        pending,
        to_server_tx.clone(),
        Arc::clone(&sessions),
    ));

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

    // Serve until Ctrl-C (SIGTERM handling + web ingress are follow-ups). The
    // reader task independently logs a server-gone EOF; a full restart+re-attach
    // supervisor loop is M16c.
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("gateway shutting down");
    listener.stop_accepting();
    Ok(())
}

/// Open the gateway's KV store from config (Redis, or in-memory for throwaway
/// runs). Mirrors the server's opener but the gateway only ever touches
/// account/token/ban keys.
async fn open_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    use mushd::config::StoreBackend;
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
    use mush_proto::{ClientCaps, ClientKind, ColorDepth, Envelope, OutputEvent, RichText};
    use mush_session::{ConnId, ConnectionHandle};

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
        let link = SocketGatewayLink {
            tx,
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
        };

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

    /// A request issued *after* the boundary has already gone down must fail
    /// fast (never park), closing the race where the reader drains the table
    /// between a request's `down` check and its insert.
    #[tokio::test]
    async fn request_after_boundary_down_fails_fast() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToServer>();
        let pending = SocketGatewayLink::pending_table();
        pending.lock().unwrap().down = true;
        let link = SocketGatewayLink {
            tx,
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
        };

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
