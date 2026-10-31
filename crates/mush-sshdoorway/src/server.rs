//! The doorway itself: the accept path, the downstream SSH handler (the auth
//! relay of doc 29 §5, the menu of §7) and the session splice of §6.
//!
//! ## The splice and russh (29a §1)
//!
//! - **R-1.** russh's server keeps one `wants_reply` flag per channel and
//!   every incoming request overwrites it, while OpenSSH pipelines `pty-req`,
//!   `env` and `shell`. So every downstream reply is sent **inside** its
//!   handler callback: `pty-req`, `shell`, `exec` and `subsystem` wait there
//!   for the upstream reply; `env`, `window-change` and `signal` are forwarded
//!   and answered locally.
//! - **R-2.** While a callback waits, russh processes nothing else on that
//!   connection — window adjusts included — and both session loops block on
//!   bounded queues. So a callback never waits on the other session's flow
//!   control: downstream → upstream traffic goes through a per-channel
//!   **writer task** fed by a bounded queue ([`UP_QUEUE`]), and the callback
//!   only enqueues. (Awaiting the upstream write in `data` let a slow player
//!   who typed while their own window was full deadlock both loops — found by
//!   the adversarial review.) The only callback waits left are replies to
//!   `pty-req`/`shell`/`exec`/`subsystem` (R-1), bounded by
//!   [`UPSTREAM_TIMEOUT`]; a missing reply closes the connection rather than
//!   leaving the reply queue out of step. Every write the pumps make is
//!   bounded by [`WRITE_TIMEOUT`]; a timeout is a wedge, and closes both
//!   sessions.
//! - **R-3.** Buffering is bounded by russh's channel queues (about 3 MiB a
//!   direction); past that the doorway stops reading upstream and the
//!   instance's own write timeout fires, as for a direct connection.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use russh::keys::PublicKey;
use russh::server::{Auth, Config as ServerConfig, Handle, Handler, Msg, Response, Session};
use russh::{
    Channel, ChannelId, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, MethodKind,
    MethodSet, Pty, Sig, client,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::config::Config;
use crate::drain::{DoorwaySession, Drain, INJECT_QUEUE, LiveGuard, Phase};
use crate::limits::{Admission, Caps, Limits, Slot};
use crate::route::{Route, log_segment, route};
use crate::upstream::{
    BeginOutcome, Connector, DisconnectSlot, HelloOutcome, Preamble, UPSTREAM_TIMEOUT, UpError,
    UpHandler, client_config, start_ssh, timed,
};
use mush_sshdoorway_proto::MAX_KEY_QUERIES;

/// A single relayed write must finish within this, or the connection is
/// treated as wedged and both sessions close (29a R-2).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Grace period after the doorway initiates `SSH_MSG_DISCONNECT` before the
/// underlying TCP stream is forcibly killed via [`crate::kill::KillSwitch`].
/// `russh` half-closes the TCP write side on disconnect and waits for client
/// EOF or `inactivity_timeout`; because `inactivity_timeout` is `None`, a
/// half-open client would otherwise hold its slot and drain guard forever (F-1).
pub const DISCONNECT_GRACE: Duration = Duration::from_secs(5);

/// Downstream → upstream messages queued per channel before the callback
/// itself waits (and, after [`WRITE_TIMEOUT`], gives up on the connection).
/// Each message is at most one SSH packet (32 KiB), so this also bounds the
/// memory a client can park at the doorway per channel: 1 MiB.
pub const UP_QUEUE: usize = 32;

/// At most this many session channels per connection: the game and `sftp`.
pub const MAX_CHANNELS: usize = 2;

/// At most this many session-channel opens over a connection's life, so an
/// open/close loop cannot spin upstream work without end.
pub const MAX_CHANNEL_OPENS: usize = 8;

/// At most this many upstream service dials over one connection's life (F-4).
pub const MAX_DIALS: usize = 4;

/// At most this many password / keyboard-interactive attempts over one
/// connection's life (F-6).
pub const MAX_PASSWORD_TRIES: usize = 3;

/// At most this many menu submissions (empty or invalid) before closing (F-2).
pub const MAX_MENU_TRIES: usize = 16;

/// A `Stage::Notice` session's whole post-auth life: long enough for a slow
/// link to open the channel and read the one line, no longer (F-8).
pub const NOTICE_IDLE: Duration = Duration::from_secs(10);

/// A menu (or notice) session is closed this long after it began, however
/// much it types: the menu holds a slot and leads nowhere by itself.
pub const MENU_HARD_CAP: Duration = Duration::from_secs(15 * 60);

/// `env` limits per connection.
pub const MAX_ENV_VARS: usize = 32;
/// Total `env` bytes per connection.
pub const MAX_ENV_BYTES: usize = 4096;
/// Longest `exec` command relayed.
pub const MAX_EXEC_BYTES: usize = 64 * 1024;
/// Longest menu input line.
pub const MAX_MENU_LINE: usize = 64;

/// The doorway's shared state.
pub struct Doorway {
    cfg: RwLock<Arc<Config>>,
    /// `[limits]` as the gates read it, rebuilt on reload (the exempt
    /// networks are parsed once here, not at every accept).
    caps: RwLock<Caps>,
    server_config: Arc<ServerConfig>,
    client_config: Arc<client::Config>,
    connector: Arc<dyn Connector>,
    /// The doorway's own gates.
    pub limits: Arc<Limits>,
    /// This generation's lifecycle and session registry.
    pub drain: Arc<Drain>,
    next_conn: AtomicU64,
    /// The host key's fingerprint (published for players, Q-29-5).
    pub host_fingerprint: String,
}

impl std::fmt::Debug for Doorway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Doorway")
            .field("host_fingerprint", &self.host_fingerprint)
            .finish_non_exhaustive()
    }
}

impl Doorway {
    /// Build a doorway from a validated config.
    pub fn new(cfg: Config, connector: Arc<dyn Connector>) -> Result<Arc<Doorway>, String> {
        let key = crate::hostkey::load_or_generate(&cfg.listen.host_key)?;
        let host_fingerprint = crate::hostkey::fingerprint(&key);
        let mut methods = MethodSet::empty();
        methods.push(MethodKind::None);
        methods.push(MethodKind::PublicKey);
        methods.push(MethodKind::Password);
        methods.push(MethodKind::KeyboardInteractive);
        let server_config = Arc::new(ServerConfig {
            server_id: russh::SshId::Standard("SSH-2.0-sshdoorway_0.1".into()),
            methods,
            keys: vec![key],
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            // Key-offer rejections at the doorway are fast (50 ms) and allow up
            // to 16 attempts so multi-key `ssh-agent` users can fall through to
            // password without a 5 s stall or lockout (F-6/F-11); password
            // attempts are separately capped at `MAX_PASSWORD_TRIES` (3) and
            // delayed 1 s by the upstream instance.
            auth_rejection_time: Duration::from_millis(50),
            auth_rejection_time_initial: Some(Duration::ZERO),
            max_auth_attempts: 16,
            nodelay: true,
            ..ServerConfig::default()
        });
        Ok(Arc::new(Doorway {
            caps: RwLock::new(Self::caps(&cfg)),
            cfg: RwLock::new(Arc::new(cfg)),
            server_config,
            client_config: client_config(),
            connector,
            limits: Limits::new(),
            drain: Drain::new(),
            next_conn: AtomicU64::new(1),
            host_fingerprint,
        }))
    }

    /// The current config.
    #[must_use]
    pub fn config(&self) -> Arc<Config> {
        Arc::clone(
            &self
                .cfg
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// `SIGHUP`: swap in a new routing table and limits. `[listen]` cannot
    /// change without a swap, so a reload that changes it is refused whole.
    pub fn reload(&self, new: Config) -> Result<(), String> {
        new.validate().map_err(|e| e.to_string())?;
        let old = self.config();
        if old.listen != new.listen {
            return Err("[listen] changed; that needs a swap, not a reload".into());
        }
        *self
            .caps
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Self::caps(&new);
        *self
            .cfg
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(new);
        Ok(())
    }

    fn caps(cfg: &Config) -> Caps {
        Caps {
            per_key: cfg.limits.per_ip_connections,
            rate: cfg.limits.per_ip_new_per_minute,
            global: cfg.limits.global_connections,
            per_v6_48: cfg.limits.per_v6_48_connections,
            // Validated at load and on reload, so this cannot fail; an
            // unparsable list would exempt nothing rather than everything.
            exempt: cfg.exempt_nets().unwrap_or_default().into(),
        }
    }

    /// The gates' current numbers.
    fn current_caps(&self) -> Caps {
        self.caps
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// How this doorway reaches services.
    #[must_use]
    pub fn connector(&self) -> Arc<dyn Connector> {
        Arc::clone(&self.connector)
    }

    /// Probe every service's trusted ingress (readiness).
    pub async fn ping_all(&self) -> Result<(), String> {
        let cfg = self.config();
        for s in &cfg.services {
            crate::upstream::ping(&*self.connector, s)
                .await
                .map_err(|e| format!("service {}: {e}", s.name))?;
        }
        Ok(())
    }

    /// Accept on `listener` until the generation starts draining. The
    /// listener is dropped on return, which closes it (with
    /// `tcp_migrate_req` its queue moves to the next generation).
    pub async fn serve(self: &Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        let mut phase = self.drain.watch();
        loop {
            if *phase.borrow() == Phase::Draining {
                return Ok(());
            }
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        let local = stream.local_addr().unwrap_or(peer);
                        self.handle(stream, peer, local);
                    }
                    Err(e) => {
                        // EMFILE and friends: back off rather than spin.
                        tracing::warn!(error = %e, "accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                changed = phase.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Log the refusal counters once a minute (doc 29 §8).
    pub async fn summarize_forever(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.tick().await;
        loop {
            tick.tick().await;
            let counters = self.limits.take_counters();
            if !counters.is_empty() {
                tracing::info!(?counters, "refusals in the last minute");
            }
        }
    }

    /// Run one accepted TCP connection.
    pub fn handle(self: &Arc<Self>, stream: TcpStream, peer: SocketAddr, local: SocketAddr) {
        let cfg = self.config();
        let slot = match self
            .limits
            .admit(peer.ip(), &self.current_caps(), Instant::now())
        {
            Admission::Admit(slot) => slot,
            Admission::Drop(reason) => {
                tracing::debug!(%peer, ?reason, "dropped at accept");
                return;
            }
        };
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        // An over-cap connection is told (banner) and refused; it gets only a
        // few seconds, so over-cap churn cannot hold global capacity.
        let over_cap = !slot.counted();
        let live = self.drain.enter(id);
        let authed = Arc::new(AtomicBool::new(false));
        let opened = Arc::new(AtomicBool::new(false));
        let mut conn = Conn::new(
            Arc::clone(self),
            id,
            peer,
            local,
            slot,
            live,
            Arc::clone(&authed),
        );
        conn.opened = Arc::clone(&opened);
        let config = Arc::clone(&self.server_config);
        let deadline = if over_cap {
            Duration::from_secs(5).min(Duration::from_secs(cfg.limits.auth_deadline_secs))
        } else {
            Duration::from_secs(cfg.limits.auth_deadline_secs)
        };
        let switch = crate::kill::KillSwitch::new();
        self.drain.track(id, Arc::clone(&switch));
        let stream = crate::kill::Killable::new(stream, switch);
        let watchdog_drain = Arc::clone(&self.drain);
        tokio::spawn(async move {
            let session = match tokio::time::timeout(
                deadline,
                russh::server::run_stream(config, stream, conn),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Ok(Err(e)) => {
                    tracing::debug!(conn = id, error = %e, "SSH start failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(conn = id, "SSH start timed out");
                    return;
                }
            };
            let handle = session.handle();
            // Two deadlines, so nothing holds a slot (or a drain) forever:
            // authenticate within `deadline`, and open a channel within
            // another `deadline` after that. If the client is stalled in KEX
            // or half-closed after disconnect, `watchdog_drain.abort(id)`
            // trips the stream's `KillSwitch` unconditionally (F-1, F-3).
            let watchdog = tokio::spawn(async move {
                tokio::time::sleep(deadline).await;
                let why = if !authed.load(Ordering::SeqCst) {
                    "authentication timed out"
                } else {
                    tokio::time::sleep(deadline).await;
                    if opened.load(Ordering::SeqCst) {
                        return;
                    }
                    "no session was opened"
                };
                let _ = tokio::time::timeout(
                    DISCONNECT_GRACE,
                    handle.disconnect(Disconnect::ByApplication, why.into(), String::new()),
                )
                .await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                watchdog_drain.abort(id);
            });
            if let Err(e) = session.await {
                tracing::debug!(conn = id, error = %e, "SSH session ended with error");
            }
            watchdog.abort();
        });
    }
}

/// The upstream side of one connection.
enum Up {
    None,
    Preamble {
        service: String,
        preamble: Preamble,
    },
    Ssh {
        service: String,
        handle: client::Handle<UpHandler>,
        disconnect: DisconnectSlot,
    },
}

impl Up {
    fn service(&self) -> Option<&str> {
        match self {
            Up::None => None,
            Up::Preamble { service, .. } | Up::Ssh { service, .. } => Some(service),
        }
    }
}

/// What dialing a service produced.
enum Dial {
    Opened,
    Tell(String),
    Cap,
}

/// Where the connection is.
#[derive(Debug)]
enum Stage {
    /// Not yet authenticated.
    PreAuth,
    /// Accepted only to tell the player something, then close.
    Notice(String),
    /// The menu.
    Menu,
    /// Spliced to `service`.
    Spliced,
}

#[derive(Clone, Debug)]
struct PtyReq {
    term: String,
    cols: u32,
    rows: u32,
    px: u32,
    py: u32,
    modes: Vec<(Pty, u32)>,
}

/// One relayed channel pair.
struct Pair {
    /// The writer task's queue (downstream → upstream, in order).
    tx: mpsc::Sender<UpCmd>,
    pump: tokio::task::JoinHandle<()>,
    writer: tokio::task::JoinHandle<()>,
    /// Replies the pump still owes the handler.
    pending: Pending,
    /// Set once the client has closed its side of the channel.
    closed: Arc<AtomicBool>,
}

/// A request that wants a reply (29a R-1).
enum Req {
    Pty(PtyReq),
    Shell,
    Exec(Vec<u8>),
    Subsystem(String),
}

/// One downstream → upstream message, applied in order by the writer task.
enum UpCmd {
    Data(Vec<u8>),
    Ext(u32, Vec<u8>),
    Eof,
    Close,
    Env(String, String),
    Window(u32, u32, u32, u32),
    Signal(Sig),
    Request(Req, oneshot::Sender<bool>),
}

type Pending = Arc<Mutex<VecDeque<oneshot::Sender<bool>>>>;

#[derive(Default)]
struct Bytes {
    up: AtomicU64,
    down: AtomicU64,
}

/// One downstream connection.
struct Conn {
    door: Arc<Doorway>,
    id: u64,
    peer: SocketAddr,
    local: SocketAddr,
    slot: Slot,
    _live: LiveGuard,
    authed: Arc<AtomicBool>,
    /// Set at the first accepted channel (the watchdog's second deadline).
    opened: Arc<AtomicBool>,
    channel_opens: usize,
    dials: usize,
    key_queries: usize,
    password_tries: usize,
    notice_offered: bool,
    menu_tries: usize,
    started: Instant,
    user: String,
    service: Option<String>,
    method: &'static str,
    stage: Stage,
    up: Up,
    pending_notice: Option<String>,
    pairs: HashMap<ChannelId, Pair>,
    /// The pre-splice channel (menu, notice): its id and write half. Its read
    /// half is dropped at open — russh pushes every channel message into that
    /// queue before calling the handler, and an unread queue of 100 would
    /// wedge the session loop (adversarial review, critical).
    front: Option<(ChannelId, ChannelWriteHalf<Msg>)>,
    pty: Option<PtyReq>,
    env: Vec<(String, String)>,
    env_bytes: usize,
    menu_line: String,
    menu_esc: u8,
    menu_last: Arc<Mutex<Instant>>,
    menu_watch: Option<tokio::task::JoinHandle<()>>,
    bytes: Arc<Bytes>,
}

/// Schedule a hard stream abort via [`Drain::abort`] after `delay` so a
/// half-closed client cannot hold its slot and drain guard after the server
/// sends `SSH_MSG_DISCONNECT` (F-1).
fn abort_after(drain: Arc<Drain>, conn: u64, delay: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        drain.abort(conn);
    });
}

/// [`abort_after`], unless the connection has authenticated by then: a
/// password chain that ran out of tries must not cut a session that a key
/// (or the front door) let in meanwhile.
fn abort_unless_authed(drain: Arc<Drain>, conn: u64, delay: Duration, authed: Arc<AtomicBool>) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if !authed.load(Ordering::SeqCst) {
            drain.abort(conn);
        }
    });
}

fn methods(kinds: &[MethodKind]) -> MethodSet {
    let mut m = MethodSet::empty();
    for k in kinds {
        m.push(*k);
    }
    m
}

/// "Try a key or a password" — what an account name is told until it logs in.
fn reject_toward_interactive() -> Auth {
    Auth::Reject {
        proceed_with_methods: Some(methods(&[
            MethodKind::PublicKey,
            MethodKind::KeyboardInteractive,
            MethodKind::Password,
        ])),
        partial_success: false,
    }
}

/// "Nothing left to try" — OpenSSH stops without prompting.
fn reject_nothing() -> Auth {
    Auth::Reject {
        proceed_with_methods: Some(MethodSet::empty()),
        partial_success: false,
    }
}

async fn next_inject(rx: &mut Option<mpsc::Receiver<String>>) -> Option<String> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

struct PumpCtx {
    up_read: ChannelReadHalf,
    down: ChannelWriteHalf<Msg>,
    down_id: ChannelId,
    handle: Handle,
    pending: Pending,
    inject: Option<mpsc::Receiver<String>>,
    disconnect: DisconnectSlot,
    bytes: Arc<Bytes>,
    farewell: String,
    drain: Arc<Drain>,
    conn: u64,
    /// Set by `channel_close` when the client closes its side.
    closed: Arc<AtomicBool>,
    /// How long a client may leave a channel the doorway closed unanswered.
    close_grace: Duration,
}

fn crlf_line(line: &str) -> Vec<u8> {
    let mut b = b"\r\n".to_vec();
    b.extend_from_slice(line.as_bytes());
    b.extend_from_slice(b"\r\n");
    b
}

/// Upstream → downstream for one channel pair, plus injected lines.
async fn pump(mut ctx: PumpCtx) {
    enum End {
        Closed,
        Gone,
        Wedged,
    }
    let end = loop {
        tokio::select! {
            msg = ctx.up_read.wait() => match msg {
                None => break End::Gone,
                Some(ChannelMsg::Data { data }) => {
                    ctx.bytes.down.fetch_add(data.len() as u64, Ordering::Relaxed);
                    match tokio::time::timeout(WRITE_TIMEOUT, ctx.down.data_bytes(data)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => break End::Closed,
                        Err(_) => break End::Wedged,
                    }
                }
                Some(ChannelMsg::ExtendedData { data, ext }) => {
                    ctx.bytes.down.fetch_add(data.len() as u64, Ordering::Relaxed);
                    match tokio::time::timeout(WRITE_TIMEOUT, ctx.down.extended_data_bytes(ext, data)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => break End::Closed,
                        Err(_) => break End::Wedged,
                    }
                }
                Some(ChannelMsg::Eof) => {
                    if tokio::time::timeout(WRITE_TIMEOUT, ctx.down.eof()).await.is_err() {
                        break End::Wedged;
                    }
                }
                Some(ChannelMsg::Close) => {
                    let _ = tokio::time::timeout(WRITE_TIMEOUT, ctx.down.close()).await;
                    break End::Closed;
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    if tokio::time::timeout(WRITE_TIMEOUT, ctx.down.exit_status(exit_status)).await.is_err() {
                        break End::Wedged;
                    }
                }
                Some(ChannelMsg::ExitSignal { signal_name, core_dumped, error_message, lang_tag }) => {
                    let send = ctx.handle.exit_signal_request(
                        ctx.down_id,
                        signal_name,
                        core_dumped,
                        error_message,
                        lang_tag,
                    );
                    if tokio::time::timeout(WRITE_TIMEOUT, send).await.is_err() {
                        break End::Wedged;
                    }
                }
                Some(ChannelMsg::Success) => reply(&ctx.pending, true),
                Some(ChannelMsg::Failure) => reply(&ctx.pending, false),
                Some(_) => {}
            },
            line = next_inject(&mut ctx.inject) => {
                let Some(line) = line else {
                    ctx.inject = None;
                    continue;
                };
                if tokio::time::timeout(WRITE_TIMEOUT, ctx.down.data_bytes(crlf_line(&line))).await.is_err() {
                    break End::Wedged;
                }
            }
        }
    };
    // Every reply still awaited is a failure now.
    while let Some(tx) = take_pending(&ctx.pending) {
        let _ = tx.send(false);
    }
    match end {
        End::Closed => {
            // The doorway has closed the channel downstream. A client that
            // never answers with its own close would keep the connection,
            // and its slot, with nothing open; it gets the same grace the
            // last channel's close gets before the no-session watchdog.
            if !ctx.closed.load(Ordering::SeqCst) {
                tokio::time::sleep(ctx.close_grace).await;
                if !ctx.closed.load(Ordering::SeqCst) {
                    tracing::info!(
                        conn = ctx.conn,
                        "a closed channel was never acknowledged; closing"
                    );
                    let _ = tokio::time::timeout(
                        DISCONNECT_GRACE,
                        ctx.handle.disconnect(
                            Disconnect::ByApplication,
                            "the channel was left half-closed".into(),
                            String::new(),
                        ),
                    )
                    .await;
                    abort_after(ctx.drain, ctx.conn, DISCONNECT_GRACE);
                }
            }
        }
        End::Gone => {
            let reason = ctx
                .disconnect
                .lock()
                .ok()
                .and_then(|s| s.clone())
                .filter(|s| !s.is_empty());
            let _ =
                tokio::time::timeout(WRITE_TIMEOUT, ctx.down.data_bytes(crlf_line(&ctx.farewell)))
                    .await;
            let _ = tokio::time::timeout(WRITE_TIMEOUT, ctx.down.close()).await;
            let _ = tokio::time::timeout(
                DISCONNECT_GRACE,
                ctx.handle.disconnect(
                    Disconnect::ByApplication,
                    reason.unwrap_or_else(|| "the game server closed the connection".into()),
                    String::new(),
                ),
            )
            .await;
            abort_after(ctx.drain, ctx.conn, DISCONNECT_GRACE);
        }
        End::Wedged => {
            tracing::info!(conn = ctx.conn, "client stopped reading; closing");
            // A polite disconnect queues behind the data the client is not
            // reading, so it gets two seconds and then the session is ended.
            let _ = tokio::time::timeout(
                DISCONNECT_GRACE,
                ctx.handle.disconnect(
                    Disconnect::ByApplication,
                    "the connection stopped reading".into(),
                    String::new(),
                ),
            )
            .await;
            ctx.drain.abort(ctx.conn);
        }
    }
}

fn take_pending(p: &Pending) -> Option<oneshot::Sender<bool>> {
    p.lock().ok().and_then(|mut q| q.pop_front())
}

fn reply(p: &Pending, ok: bool) {
    if let Some(tx) = take_pending(p) {
        let _ = tx.send(ok);
    }
}

/// Downstream → upstream for one channel pair: applies queued messages in
/// order, each bounded by [`WRITE_TIMEOUT`]. A reply-wanting request is
/// registered in the pump's FIFO immediately before it is sent, so replies
/// line up with requests.
async fn writer(
    up: ChannelWriteHalf<client::Msg>,
    mut rx: mpsc::Receiver<UpCmd>,
    pending: Pending,
    handle: Handle,
    drain: Arc<Drain>,
    conn: u64,
) {
    while let Some(cmd) = rx.recv().await {
        let sent = tokio::time::timeout(WRITE_TIMEOUT, async {
            match cmd {
                UpCmd::Data(d) => up.data_bytes(d).await,
                UpCmd::Ext(code, d) => up.extended_data_bytes(code, d).await,
                UpCmd::Eof => up.eof().await,
                UpCmd::Close => up.close().await,
                UpCmd::Env(k, v) => up.set_env(false, k, v).await,
                UpCmd::Window(c, r, x, y) => up.window_change(c, r, x, y).await,
                UpCmd::Signal(sig) => up.signal(sig).await,
                UpCmd::Request(req, tx) => {
                    if let Ok(mut q) = pending.lock() {
                        q.push_back(tx);
                    }
                    let r = match req {
                        Req::Pty(p) => {
                            up.request_pty(true, &p.term, p.cols, p.rows, p.px, p.py, &p.modes)
                                .await
                        }
                        Req::Shell => up.request_shell(true).await,
                        Req::Exec(cmd) => up.exec(true, cmd).await,
                        Req::Subsystem(name) => up.request_subsystem(true, name).await,
                    };
                    if r.is_err()
                        && let Some(tx) = pending.lock().ok().and_then(|mut q| q.pop_back())
                    {
                        let _ = tx.send(false);
                    }
                    r
                }
            }
        })
        .await;
        match sent {
            Ok(Ok(())) => {}
            // The upstream channel is gone; the pump tells the player.
            Ok(Err(_)) => return,
            Err(_) => {
                tracing::info!(conn, "upstream stopped reading; closing");
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    handle.disconnect(
                        Disconnect::ByApplication,
                        "the game server stopped reading".into(),
                        String::new(),
                    ),
                )
                .await;
                drain.abort(conn);
                return;
            }
        }
    }
}

impl Conn {
    #[allow(clippy::too_many_arguments)]
    fn new(
        door: Arc<Doorway>,
        id: u64,
        peer: SocketAddr,
        local: SocketAddr,
        slot: Slot,
        live: LiveGuard,
        authed: Arc<AtomicBool>,
    ) -> Conn {
        Conn {
            door,
            id,
            peer,
            local,
            slot,
            _live: live,
            authed,
            opened: Arc::default(),
            channel_opens: 0,
            dials: 0,
            key_queries: 0,
            password_tries: 0,
            notice_offered: false,
            menu_tries: 0,
            started: Instant::now(),
            user: String::new(),
            service: None,
            method: "none",
            stage: Stage::PreAuth,
            up: Up::None,
            pending_notice: None,
            pairs: HashMap::new(),
            front: None,
            pty: None,
            env: Vec::new(),
            env_bytes: 0,
            menu_line: String::new(),
            menu_esc: 0,
            menu_last: Arc::new(Mutex::new(Instant::now())),
            menu_watch: None,
            bytes: Arc::new(Bytes::default()),
        }
    }

    fn accept(&mut self, method: &'static str, stage: Stage) -> Auth {
        self.method = method;
        self.stage = stage;
        self.authed.store(true, Ordering::SeqCst);
        Auth::Accept
    }

    fn tell(&mut self, method: &'static str, text: String) -> Auth {
        // A notice session exists only to print one refusal line and close;
        // bound its entire post-auth life by `NOTICE_IDLE` whether or not the
        // client ever opens a channel or requests a shell (F-1, F-8).
        abort_after(Arc::clone(&self.door.drain), self.id, NOTICE_IDLE);
        self.accept(method, Stage::Notice(text))
    }

    fn drop_up(&mut self) {
        if let Up::Ssh { handle, .. } = std::mem::replace(&mut self.up, Up::None) {
            tokio::spawn(async move {
                let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
            });
        }
    }

    /// Dial `service` (unless this connection already has it).
    async fn dial(&mut self, service: &str) -> Dial {
        if self.up.service() == Some(service) {
            return Dial::Opened;
        }
        self.drop_up();
        if self.dials >= MAX_DIALS {
            self.door.limits.count("dial-cap");
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Dial::Cap;
        }
        self.dials += 1;
        let cfg = self.door.config();
        let Some(svc) = cfg.service(service).cloned() else {
            return Dial::Tell(format!("{service} is not available."));
        };
        match Preamble::hello(&*self.door.connector, &svc, self.peer, self.local, self.id).await {
            Ok(HelloOutcome::Open(preamble)) => {
                self.service = Some(svc.name.clone());
                self.up = Up::Preamble {
                    service: svc.name,
                    preamble,
                };
                Dial::Opened
            }
            Ok(HelloOutcome::Blocked(notice)) => Dial::Tell(notice),
            Ok(HelloOutcome::Refused) => Dial::Tell(format!(
                "Too many connections from your address to {service}; close one and try again."
            )),
            Err(e) => {
                self.door.limits.count("service-unreachable");
                tracing::debug!(conn = self.id, service, error = %e, "service unreachable");
                Dial::Tell(format!(
                    "{service} is not reachable right now; please try again in a minute."
                ))
            }
        }
    }

    /// `Begin` and, on `Ready`, start upstream SSH.
    async fn begin(&mut self, user: &str, key: Option<String>) -> Result<BeginOutcome, UpError> {
        let outcome = match &mut self.up {
            Up::Preamble { preamble, .. } => match preamble.begin(user, key).await {
                Ok(o) => o,
                Err(e) => {
                    // A dead preamble must not be reused: the next attempt
                    // redials (adversarial review F3).
                    self.drop_up();
                    return Err(e);
                }
            },
            Up::Ssh { .. } if key.is_none() => return Ok(BeginOutcome::Ready),
            _ => return Err(UpError::Limit),
        };
        if outcome != BeginOutcome::Ready {
            return Ok(outcome);
        }
        let Up::Preamble { service, preamble } = std::mem::replace(&mut self.up, Up::None) else {
            return Err(UpError::Limit);
        };
        let (handle, disconnect) = start_ssh(
            Arc::clone(&self.door.client_config),
            preamble.into_stream(),
            &service,
        )
        .await?;
        // (`self.up` is already `None` here, so a failure above redials.)
        self.up = Up::Ssh {
            service,
            handle,
            disconnect,
        };
        Ok(BeginOutcome::Ready)
    }

    async fn up_none(&mut self, user: &str) -> bool {
        let Up::Ssh { handle, .. } = &mut self.up else {
            return false;
        };
        match timed(handle.authenticate_none(user)).await {
            Ok(r) => r.success(),
            Err(_) => {
                self.drop_up();
                false
            }
        }
    }

    async fn up_password(&mut self, user: &str, password: &str) -> bool {
        let Up::Ssh { handle, .. } = &mut self.up else {
            return false;
        };
        match timed(handle.authenticate_password(user, password)).await {
            Ok(r) => r.success(),
            Err(_) => {
                self.drop_up();
                false
            }
        }
    }

    /// The front door of `service`: `none` upstream as `upstream_user`.
    async fn front_door(&mut self, service: &str, upstream_user: &str) -> Auth {
        match self.dial(service).await {
            Dial::Tell(t) => return self.tell("none", t),
            Dial::Cap => return reject_nothing(),
            Dial::Opened => {}
        }
        match self.begin(upstream_user, None).await {
            Ok(BeginOutcome::Ready) => {}
            Ok(_) | Err(_) => {
                return self.tell(
                    "none",
                    format!("{service} is not reachable right now; please try again in a minute."),
                );
            }
        }
        if self.up_none(upstream_user).await {
            self.accept("none", Stage::Spliced)
        } else {
            reject_nothing()
        }
    }

    /// A password (or keyboard-interactive answer) for an account name.
    async fn password(&mut self, service: &str, upstream_user: &str, password: &str) -> Auth {
        if self.password_tries >= MAX_PASSWORD_TRIES {
            self.door.limits.count("password-tries");
            abort_unless_authed(
                Arc::clone(&self.door.drain),
                self.id,
                DISCONNECT_GRACE,
                Arc::clone(&self.authed),
            );
            return reject_nothing();
        }
        self.password_tries += 1;
        match self.dial(service).await {
            Dial::Tell(t) => return self.tell("password", t),
            Dial::Cap => return reject_nothing(),
            Dial::Opened => {}
        }
        if !matches!(
            self.begin(upstream_user, None).await,
            Ok(BeginOutcome::Ready)
        ) {
            return reject_toward_interactive();
        }
        if self.up_password(upstream_user, password).await {
            self.accept("password", Stage::Spliced)
        } else if self.password_tries >= MAX_PASSWORD_TRIES {
            abort_unless_authed(
                Arc::clone(&self.door.drain),
                self.id,
                DISCONNECT_GRACE,
                Arc::clone(&self.authed),
            );
            reject_nothing()
        } else {
            reject_toward_interactive()
        }
    }

    fn farewell(&self) -> String {
        let cfg = self.door.config();
        let service = self.service.as_deref().unwrap_or("the game server");
        let user = if self.user.is_empty() {
            service.to_owned()
        } else {
            self.user.clone()
        };
        format!(
            "{service} closed the connection; reconnect with ssh {user}@{}",
            cfg.menu.host
        )
    }

    fn pair_up(
        &mut self,
        down_id: ChannelId,
        down_write: ChannelWriteHalf<Msg>,
        up: Channel<client::Msg>,
        handle: Handle,
    ) {
        let (up_read, up_write) = up.split();
        let pending: Pending = Arc::default();
        let inject = if self.pairs.is_empty() {
            let (tx, rx) = mpsc::channel(INJECT_QUEUE);
            self.door.drain.register(
                self.id,
                DoorwaySession::new(handle.clone(), Some(down_id), Some(tx)),
            );
            Some(rx)
        } else {
            None
        };
        let disconnect = match &self.up {
            Up::Ssh { disconnect, .. } => Arc::clone(disconnect),
            _ => DisconnectSlot::default(),
        };
        let (tx, rx) = mpsc::channel(UP_QUEUE);
        let closed = Arc::new(AtomicBool::new(false));
        let writer = tokio::spawn(writer(
            up_write,
            rx,
            Arc::clone(&pending),
            handle.clone(),
            Arc::clone(&self.door.drain),
            self.id,
        ));
        let pump = tokio::spawn(pump(PumpCtx {
            up_read,
            down: down_write,
            down_id,
            handle,
            pending: Arc::clone(&pending),
            inject,
            disconnect,
            bytes: Arc::clone(&self.bytes),
            farewell: self.farewell(),
            drain: Arc::clone(&self.door.drain),
            conn: self.id,
            closed: Arc::clone(&closed),
            close_grace: Duration::from_secs(self.door.config().limits.auth_deadline_secs),
        }));
        self.pairs.insert(
            down_id,
            Pair {
                tx,
                pump,
                writer,
                pending,
                closed,
            },
        );
    }

    /// Queue one message for the channel's writer. Waits only when the queue
    /// is full, and then at most [`WRITE_TIMEOUT`].
    async fn send_up(&self, ch: ChannelId, cmd: UpCmd) -> Result<(), russh::Error> {
        let Some(pair) = self.pairs.get(&ch) else {
            return Ok(());
        };
        match pair.tx.try_send(cmd) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Full(cmd)) => {
                match tokio::time::timeout(WRITE_TIMEOUT, pair.tx.send(cmd)).await {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::info!(conn = self.id, "upstream queue stayed full; closing");
                        Err(russh::Error::Disconnect)
                    }
                }
            }
        }
    }

    /// Send a request that wants a reply upstream, and wait for it. A reply
    /// that never comes closes the connection: the FIFO of pending replies
    /// would otherwise be out of step for the rest of the session.
    async fn relay_reply(&self, ch: ChannelId, req: Req) -> Result<bool, russh::Error> {
        if !self.pairs.contains_key(&ch) {
            return Ok(false);
        }
        let (tx, rx) = oneshot::channel();
        self.send_up(ch, UpCmd::Request(req, tx)).await?;
        match tokio::time::timeout(UPSTREAM_TIMEOUT + WRITE_TIMEOUT, rx).await {
            Ok(Ok(ok)) => Ok(ok),
            Ok(Err(_)) => Ok(false),
            Err(_) => {
                tracing::info!(conn = self.id, "upstream never replied; closing");
                Err(russh::Error::Disconnect)
            }
        }
    }

    fn front_id(&self) -> Option<ChannelId> {
        self.front.as_ref().map(|(id, _)| *id)
    }

    fn write_front(&self, session: &mut Session, text: &str) {
        let Some(ch) = self.front_id() else {
            return;
        };
        if session.has_pending_data(ch) {
            return;
        }
        let mut bytes = Vec::with_capacity(text.len() + 8);
        for line in text.lines() {
            bytes.extend_from_slice(line.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        let _ = session.data(ch, bytes);
    }

    fn close_front(&mut self, session: &mut Session, text: &str) {
        self.write_front(session, text);
        if let Some(ch) = self.front_id() {
            let _ = session.close(ch);
        }
        let _ = session.disconnect(Disconnect::ByApplication, "goodbye", "");
        abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
    }

    fn menu_text(&self) -> String {
        let cfg = self.door.config();
        let mut out = format!("{}\n\n", cfg.menu.title);
        for (i, s) in cfg.services.iter().filter(|s| s.listed).enumerate() {
            out.push_str(&format!("  {}  {:<8} {}\n", i + 1, s.name, s.title));
        }
        out.push('\n');
        out
    }

    fn menu_prompt(&self, session: &mut Session) {
        if let Some(ch) = self.front_id()
            && !session.has_pending_data(ch)
        {
            let _ = session.data(ch, b"Type a number or a name (q to quit): ".to_vec());
        }
    }

    fn start_menu_watch(&mut self, handle: Handle) {
        if let Some(w) = self.menu_watch.take() {
            w.abort();
        }
        let last = Arc::clone(&self.menu_last);
        let idle = if matches!(self.stage, Stage::Notice(_)) {
            NOTICE_IDLE
        } else {
            Duration::from_secs(self.door.config().limits.menu_idle_secs)
        };
        let ch = self.front_id();
        let drain = Arc::clone(&self.door.drain);
        let conn = self.id;
        let began = Instant::now();
        self.menu_watch = Some(tokio::spawn(async move {
            loop {
                let elapsed = last.lock().map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
                if elapsed >= idle || began.elapsed() >= MENU_HARD_CAP {
                    if let Some(ch) = ch {
                        let _ = tokio::time::timeout(
                            DISCONNECT_GRACE,
                            handle.data(ch, b"\r\nMenu timed out.\r\n".to_vec()),
                        )
                        .await;
                    }
                    let _ = tokio::time::timeout(
                        DISCONNECT_GRACE,
                        handle.disconnect(
                            Disconnect::ByApplication,
                            "menu timed out".into(),
                            String::new(),
                        ),
                    )
                    .await;
                    abort_after(drain, conn, DISCONNECT_GRACE);
                    return;
                }
                let left = idle
                    .saturating_sub(elapsed)
                    .min(MENU_HARD_CAP.saturating_sub(began.elapsed()));
                tokio::time::sleep(left.max(Duration::from_millis(50))).await;
            }
        }));
    }

    async fn menu_input(&mut self, data: &[u8], session: &mut Session) {
        if let Ok(mut t) = self.menu_last.lock() {
            *t = Instant::now();
        }
        let Some(ch) = self.front_id() else {
            return;
        };
        if session.has_pending_data(ch) {
            self.door.limits.count("menu-window-wedged");
            let _ = session.close(ch);
            let _ = session.disconnect(
                Disconnect::ByApplication,
                "the connection stopped reading",
                "",
            );
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return;
        }
        let mut echo = Vec::new();
        for &b in data {
            // A tiny escape-sequence skipper, so arrow keys are not typed.
            match self.menu_esc {
                1 => {
                    self.menu_esc = if b == b'[' || b == b'O' { 2 } else { 0 };
                    continue;
                }
                2 => {
                    if (0x40..=0x7e).contains(&b) {
                        self.menu_esc = 0;
                    }
                    continue;
                }
                _ => {}
            }
            match b {
                0x1b => self.menu_esc = 1,
                b'\r' | b'\n' => {
                    if !session.has_pending_data(ch) {
                        let _ = session.data(ch, std::mem::take(&mut echo));
                        let _ = session.data(ch, b"\r\n".to_vec());
                    } else {
                        echo.clear();
                    }
                    let line = std::mem::take(&mut self.menu_line);
                    if self.menu_submit(line.trim(), session).await {
                        return;
                    }
                }
                0x7f | 0x08 => {
                    if self.menu_line.pop().is_some() {
                        echo.extend_from_slice(b"\x08 \x08");
                    }
                }
                0x03 | 0x04 => {
                    if !session.has_pending_data(ch) {
                        let _ = session.data(ch, std::mem::take(&mut echo));
                    }
                    self.close_front(session, "\r\nGoodbye.");
                    return;
                }
                0x20..=0x7e if self.menu_line.len() < MAX_MENU_LINE => {
                    self.menu_line.push(char::from(b));
                    echo.push(b);
                }
                _ => {}
            }
        }
        if !echo.is_empty() && !session.has_pending_data(ch) {
            let _ = session.data(ch, echo);
        }
    }

    /// Returns `true` when the menu is finished (spliced or closed).
    async fn menu_submit(&mut self, line: &str, session: &mut Session) -> bool {
        if line.is_empty() {
            self.menu_tries += 1;
            if self.menu_tries >= MAX_MENU_TRIES {
                self.door.limits.count("menu-tries");
                self.close_front(session, "Goodbye.");
                return true;
            }
            self.menu_prompt(session);
            return false;
        }
        if line.eq_ignore_ascii_case("q") || line.eq_ignore_ascii_case("quit") {
            self.close_front(session, "Goodbye.");
            return true;
        }
        let cfg = self.door.config();
        let listed: Vec<String> = cfg
            .services
            .iter()
            .filter(|s| s.listed)
            .map(|s| s.name.clone())
            .collect();
        let choice = match line.parse::<usize>() {
            Ok(n) if n >= 1 => listed.get(n - 1).cloned(),
            Ok(_) => None,
            // Unlisted services are reachable by name (Q-29-6).
            Err(_) => cfg.service(line).map(|s| s.name.clone()),
        };
        let Some(service) = choice else {
            self.menu_tries += 1;
            if self.menu_tries >= MAX_MENU_TRIES {
                self.door.limits.count("menu-tries");
                self.close_front(session, "Too many invalid choices.");
                return true;
            }
            self.write_front(session, "Unknown choice.");
            self.menu_prompt(session);
            return false;
        };
        self.splice_from_menu(&service, session).await;
        true
    }

    async fn splice_from_menu(&mut self, service: &str, session: &mut Session) {
        // Stop the menu idle timer before awaiting the upstream handshake so it
        // cannot fire mid-splice and disconnect the newly spliced session (F-9).
        if let Some(w) = self.menu_watch.take() {
            w.abort();
        }
        self.write_front(session, &format!("Connecting to {service}..."));
        match self.dial(service).await {
            Dial::Tell(t) => {
                self.close_front(session, &t);
                return;
            }
            Dial::Cap => {
                self.close_front(session, "Too many authentication attempts.");
                return;
            }
            Dial::Opened => {}
        }
        let unreachable =
            format!("{service} is not reachable right now; please try again in a minute.");
        if !matches!(self.begin("play", None).await, Ok(BeginOutcome::Ready))
            || !self.up_none("play").await
        {
            self.close_front(session, &unreachable);
            return;
        }
        let Up::Ssh { handle, .. } = &self.up else {
            self.close_front(session, &unreachable);
            return;
        };
        let Ok(mut up) = timed(handle.channel_open_session()).await else {
            self.close_front(session, &unreachable);
            return;
        };
        let mut early: Vec<u8> = Vec::new();
        let replay = async {
            if let Some(p) = &self.pty {
                up.request_pty(true, &p.term, p.cols, p.rows, p.px, p.py, &p.modes)
                    .await?;
                if !wait_reply(&mut up, &mut early).await {
                    return Err(russh::Error::Disconnect);
                }
            }
            for (k, v) in &self.env {
                up.set_env(false, k.as_str(), v.as_str()).await?;
            }
            up.request_shell(true).await?;
            if !wait_reply(&mut up, &mut early).await {
                return Err(russh::Error::Disconnect);
            }
            Ok::<(), russh::Error>(())
        };
        if !matches!(
            tokio::time::timeout(UPSTREAM_TIMEOUT, replay).await,
            Ok(Ok(()))
        ) {
            self.close_front(session, &unreachable);
            return;
        }
        let host = self.door.config().menu.host.clone();
        self.write_front(
            session,
            &format!("Next time: ssh {service}+<account>@{host}\n"),
        );
        if let Some(ch) = self.front_id()
            && !early.is_empty()
        {
            let _ = session.data(ch, early);
        }
        let Some((front_id, front_write)) = self.front.take() else {
            return;
        };
        self.user = format!("{service}+play");
        self.stage = Stage::Spliced;
        self.pair_up(front_id, front_write, up, session.handle());
    }
}

const MAX_EARLY_BYTES: usize = 1024 * 1024;

/// Wait for one `Success`/`Failure` on a not-yet-pumped channel, keeping any
/// data that arrives first. `false` when the channel closed or ended first
/// (the shell went away before answering) or the request failed.
async fn wait_reply(up: &mut Channel<client::Msg>, early: &mut Vec<u8>) -> bool {
    while let Some(msg) = up.wait().await {
        match msg {
            ChannelMsg::Success => return true,
            ChannelMsg::Failure | ChannelMsg::Close | ChannelMsg::Eof => return false,
            ChannelMsg::Data { data } if early.len() < MAX_EARLY_BYTES => {
                let room = MAX_EARLY_BYTES - early.len();
                early.extend_from_slice(data.get(..room).unwrap_or(&data));
            }
            _ => {}
        }
    }
    false
}

impl Drop for Conn {
    fn drop(&mut self) {
        for (_, pair) in self.pairs.drain() {
            pair.pump.abort();
            pair.writer.abort();
        }
        if let Some(w) = self.menu_watch.take() {
            w.abort();
        }
        self.drop_up();
        let segment = log_segment(&self.user);
        if !self.authed.load(Ordering::SeqCst) {
            self.door.limits.count("unauthenticated");
        } else if matches!(self.stage, Stage::Spliced) {
            // One line per player session. Menu and notice sessions are
            // cheap to make and would let an address write the log at will
            // (the file sink is capped, but at the cost of the lines that
            // matter), so they are counted instead and logged at debug.
            tracing::info!(
                conn = self.id,
                peer = %self.peer,
                user = %segment,
                service = self.service.as_deref().unwrap_or("-"),
                method = self.method,
                secs = self.started.elapsed().as_secs(),
                up = self.bytes.up.load(Ordering::Relaxed),
                down = self.bytes.down.load(Ordering::Relaxed),
                "connection ended"
            );
        } else {
            self.door.limits.count(match self.stage {
                Stage::Menu => "menu-ended",
                _ => "notice-ended",
            });
            tracing::debug!(
                conn = self.id,
                peer = %self.peer,
                user = %segment,
                stage = ?std::mem::discriminant(&self.stage),
                secs = self.started.elapsed().as_secs(),
                "connection ended"
            );
        }
    }
}

impl Handler for Conn {
    type Error = russh::Error;

    async fn auth_succeeded(&mut self, session: &mut Session) -> Result<(), Self::Error> {
        // Reachable by the drain's deadline from the moment it is in, before
        // any channel exists.
        self.door
            .drain
            .register(self.id, DoorwaySession::new(session.handle(), None, None));
        Ok(())
    }

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        if self.slot.counted() {
            return Ok(None);
        }
        Ok(Some(
            "Too many connections from your address; close one and try again.\r\n".to_owned(),
        ))
    }

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let cfg = self.door.config();
        match route(user, &cfg) {
            Route::Invalid => {
                self.door.limits.count("invalid-user");
                Ok(reject_nothing())
            }
            Route::Menu => Ok(self.accept("none", Stage::Menu)),
            Route::Service {
                service,
                upstream_user,
                front_door: true,
            } => Ok(self.front_door(&service, &upstream_user).await),
            Route::Service { .. } => Ok(reject_toward_interactive()),
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let cfg = self.door.config();
        match route(user, &cfg) {
            Route::Invalid | Route::Menu => {
                self.door.limits.count("password-for-non-service");
                Ok(reject_nothing())
            }
            Route::Service {
                service,
                upstream_user,
                front_door: true,
            } => Ok(self.front_door(&service, &upstream_user).await),
            Route::Service {
                service,
                upstream_user,
                ..
            } => Ok(self.password(&service, &upstream_user, password).await),
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let cfg = self.door.config();
        match route(user, &cfg) {
            Route::Invalid | Route::Menu => {
                self.door.limits.count("password-for-non-service");
                Ok(reject_nothing())
            }
            Route::Service {
                service,
                upstream_user,
                front_door: true,
            } => Ok(self.front_door(&service, &upstream_user).await),
            Route::Service {
                service,
                upstream_user,
                ..
            } => match response {
                None => Ok(Auth::Partial {
                    name: "rustmush login".into(),
                    instructions: "".into(),
                    prompts: vec![("Password: ".into(), false)].into(),
                }),
                Some(mut r) => {
                    let password = r
                        .next()
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    Ok(self.password(&service, &upstream_user, &password).await)
                }
            },
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Ok(reject_nothing());
        }
        if self.notice_offered || self.key_queries >= MAX_KEY_QUERIES {
            return Ok(reject_toward_interactive());
        }
        self.key_queries += 1;
        self.user = user.to_owned();
        let cfg = self.door.config();
        let Route::Service {
            service,
            upstream_user,
            front_door: false,
        } = route(user, &cfg)
        else {
            return Ok(match route(user, &cfg) {
                Route::Service { .. } => reject_toward_interactive(),
                _ => reject_nothing(),
            });
        };
        let Ok(openssh) = key.to_openssh() else {
            return Ok(reject_toward_interactive());
        };
        match self.dial(&service).await {
            Dial::Tell(t) => {
                // Let the client sign at most once; the signed callback accepts and tells (F-4).
                self.pending_notice = Some(t);
                self.notice_offered = true;
                return Ok(Auth::Accept);
            }
            Dial::Cap => return Ok(reject_nothing()),
            Dial::Opened => {}
        }
        let Up::Preamble { preamble, .. } = &mut self.up else {
            return Ok(reject_toward_interactive());
        };
        match preamble.key_query(&upstream_user, &openssh).await {
            Ok(true) => Ok(Auth::Accept),
            Ok(false) => Ok(reject_toward_interactive()),
            Err(_) => {
                self.drop_up();
                Ok(reject_toward_interactive())
            }
        }
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            abort_after(Arc::clone(&self.door.drain), self.id, DISCONNECT_GRACE);
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let cfg = self.door.config();
        let Route::Service {
            service,
            upstream_user,
            front_door: false,
        } = route(user, &cfg)
        else {
            return Ok(reject_nothing());
        };
        if let Some(t) = self.pending_notice.take() {
            return Ok(self.tell("publickey", t));
        }
        let Ok(openssh) = key.to_openssh() else {
            return Ok(reject_toward_interactive());
        };
        match self.dial(&service).await {
            Dial::Tell(t) => return Ok(self.tell("publickey", t)),
            Dial::Cap => return Ok(reject_nothing()),
            Dial::Opened => {}
        }
        match self.begin(&upstream_user, Some(openssh)).await {
            Ok(BeginOutcome::Ready) => {}
            Ok(BeginOutcome::Banned) => return Ok(Auth::reject()),
            Ok(BeginOutcome::BadCreds) | Err(_) => return Ok(reject_toward_interactive()),
        }
        if self.up_none(&upstream_user).await {
            Ok(self.accept("publickey", Stage::Spliced))
        } else {
            Ok(reject_toward_interactive())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channel_opens += 1;
        if self.channel_opens > MAX_CHANNEL_OPENS {
            drop(reply);
            return Ok(());
        }
        let id = channel.id();
        // Downstream messages reach the doorway through the Handler
        // callbacks. The channel's own queue is never read, so its receiver
        // is dropped here and russh's push into it fails at once instead of
        // blocking the session loop (adversarial review, critical).
        let (read, write) = channel.split();
        drop(read);
        match self.stage {
            Stage::Spliced => {
                if self.pairs.len() >= MAX_CHANNELS {
                    drop(reply);
                    return Ok(());
                }
                let Up::Ssh { handle, .. } = &self.up else {
                    drop(reply);
                    return Ok(());
                };
                match timed(handle.channel_open_session()).await {
                    Ok(up) => {
                        reply.accept().await;
                        self.opened.store(true, Ordering::SeqCst);
                        self.pair_up(id, write, up, session.handle());
                    }
                    Err(_) => drop(reply),
                }
            }
            Stage::Menu | Stage::Notice(_) => {
                if self.front.is_some() {
                    drop(reply);
                    return Ok(());
                }
                reply.accept().await;
                self.opened.store(true, Ordering::SeqCst);
                self.door.drain.register(
                    self.id,
                    DoorwaySession::new(session.handle(), Some(id), None),
                );
                self.front = Some((id, write));
                // The idle clock starts at the channel, not the shell request:
                // a channel that never asks for a shell still times out.
                if let Ok(mut t) = self.menu_last.lock() {
                    *t = Instant::now();
                }
                self.start_menu_watch(session.handle());
            }
            Stage::PreAuth => drop(reply),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        px: u32,
        py: u32,
        modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if term.len() > 64 {
            return session.channel_failure(channel);
        }
        let req = PtyReq {
            term: term.to_owned(),
            cols,
            rows,
            px,
            py,
            modes: modes.iter().take(64).copied().collect(),
        };
        if self.pairs.contains_key(&channel) {
            return if self.relay_reply(channel, Req::Pty(req)).await? {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        if self.front_id() == Some(channel) {
            self.pty = Some(req);
            return session.channel_success(channel);
        }
        session.channel_failure(channel)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let size = name.len() + value.len();
        if self.env.len() >= MAX_ENV_VARS || self.env_bytes + size > MAX_ENV_BYTES {
            return session.channel_failure(channel);
        }
        self.env_bytes += size;
        self.send_up(channel, UpCmd::Env(name.to_owned(), value.to_owned()))
            .await?;
        self.env.push((name.to_owned(), value.to_owned()));
        session.channel_success(channel)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        px: u32,
        py: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.pairs.contains_key(&channel) {
            self.send_up(channel, UpCmd::Window(cols, rows, px, py))
                .await?;
        } else if let Some(p) = self.pty.as_mut() {
            (p.cols, p.rows, p.px, p.py) = (cols, rows, px, py);
        }
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.pairs.contains_key(&channel) {
            return if self.relay_reply(channel, Req::Shell).await? {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        if self.front_id() != Some(channel) {
            return session.channel_failure(channel);
        }
        session.channel_success(channel)?;
        match &self.stage {
            Stage::Notice(text) => {
                let text = text.clone();
                self.close_front(session, &text);
            }
            Stage::Menu if self.pty.is_none() => {
                self.close_front(
                    session,
                    "sshdoorway needs an interactive terminal; reconnect without forcing no-PTY.",
                );
            }
            Stage::Menu => {
                let text = self.menu_text();
                self.write_front(session, &text);
                self.menu_prompt(session);
            }
            _ => {}
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.pairs.contains_key(&channel) && data.len() <= MAX_EXEC_BYTES {
            return if self.relay_reply(channel, Req::Exec(data.to_vec())).await? {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        session.channel_failure(channel)?;
        if self.front_id() == Some(channel) {
            self.door.limits.count("exec-refused");
            let text = match &self.stage {
                Stage::Notice(t) => t.clone(),
                _ => "sshdoorway needs an interactive terminal; commands are not available here."
                    .to_owned(),
            };
            self.close_front(session, &text);
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.pairs.contains_key(&channel) && name.len() <= 64 {
            return if self
                .relay_reply(channel, Req::Subsystem(name.to_owned()))
                .await?
            {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        session.channel_failure(channel)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single: bool,
        _protocol: &str,
        _cookie: &str,
        _screen: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)
    }

    async fn agent_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let _ = (channel, session);
        Ok(false)
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_up(channel, UpCmd::Signal(signal)).await
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.pairs.contains_key(&channel) {
            self.bytes
                .up
                .fetch_add(data.len() as u64, Ordering::Relaxed);
            return self.send_up(channel, UpCmd::Data(data.to_vec())).await;
        }
        if matches!(self.stage, Stage::Menu) && self.front_id() == Some(channel) {
            self.menu_input(data, session).await;
        }
        Ok(())
    }

    async fn extended_data(
        &mut self,
        channel: ChannelId,
        code: u32,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_up(channel, UpCmd::Ext(code, data.to_vec())).await
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_up(channel, UpCmd::Eof).await
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(pair) = self.pairs.remove(&channel) {
            pair.closed.store(true, Ordering::SeqCst);
            // The client has closed its side: nothing more goes down this
            // channel, so the pump stops now. Left running, a pump blocked on
            // this channel's exhausted window would wait out `WRITE_TIMEOUT`
            // and then end the whole connection as wedged, other channels
            // included. Every reply it still owed is a failure.
            pair.pump.abort();
            while let Some(tx) = take_pending(&pair.pending) {
                let _ = tx.send(false);
            }
            // The close must reach the instance even through a full queue
            // (dropped, it would leave the instance's channel open until its
            // own timeout); the writer then gets a grace to deliver it.
            // (Opens are capped per connection, so these stay few.)
            let (tx, writer) = (pair.tx, pair.writer);
            tokio::spawn(async move {
                let _ = tokio::time::timeout(WRITE_TIMEOUT, tx.send(UpCmd::Close)).await;
                tokio::time::sleep(UPSTREAM_TIMEOUT).await;
                writer.abort();
            });
        }
        if self.front_id() == Some(channel) {
            self.front = None;
        }
        if self.pairs.is_empty() && self.front.is_none() {
            self.opened.store(false, Ordering::SeqCst);
            let opened = Arc::clone(&self.opened);
            let drain = Arc::clone(&self.door.drain);
            let conn = self.id;
            let handle = session.handle();
            let grace = Duration::from_secs(self.door.config().limits.auth_deadline_secs);
            tokio::spawn(async move {
                tokio::time::sleep(grace).await;
                if !opened.load(Ordering::SeqCst) {
                    let _ = tokio::time::timeout(
                        DISCONNECT_GRACE,
                        handle.disconnect(
                            Disconnect::ByApplication,
                            "no session is open".into(),
                            String::new(),
                        ),
                    )
                    .await;
                    abort_after(drain, conn, DISCONNECT_GRACE);
                }
            });
        }
        Ok(())
    }
}
