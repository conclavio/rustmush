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
//!   connection, and both session loops block on bounded queues — a cycle is
//!   possible when a client stops reading. Every cross-session wait is
//!   therefore timed: [`WRITE_TIMEOUT`] for data, [`UPSTREAM_TIMEOUT`] for
//!   replies and opens. A timeout is a wedge, and closes both sessions.
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
use crate::drain::{Drain, INJECT_QUEUE, LiveGuard, Phase};
use crate::limits::{Admission, Caps, Limits, Slot};
use crate::route::{Route, log_segment, route};
use crate::upstream::{
    BeginOutcome, Connector, DisconnectSlot, HelloOutcome, Preamble, UPSTREAM_TIMEOUT, UpError,
    UpHandler, client_config, start_ssh, timed,
};

/// A single relayed write must finish within this, or the connection is
/// treated as wedged and both sessions close (29a R-2).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// At most this many session channels per connection: the game and `sftp`.
pub const MAX_CHANNELS: usize = 2;

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
            auth_rejection_time: Duration::from_secs(1),
            auth_rejection_time_initial: Some(Duration::ZERO),
            max_auth_attempts: 6,
            nodelay: true,
            ..ServerConfig::default()
        });
        Ok(Arc::new(Doorway {
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
        }
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
            .admit(peer.ip(), Self::caps(&cfg), Instant::now())
        {
            Admission::Admit(slot) => slot,
            Admission::Drop(reason) => {
                tracing::debug!(%peer, ?reason, "dropped at accept");
                return;
            }
        };
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let live = self.drain.enter(id);
        let authed = Arc::new(AtomicBool::new(false));
        let conn = Conn::new(
            Arc::clone(self),
            cfg,
            id,
            peer,
            local,
            slot,
            live,
            Arc::clone(&authed),
        );
        let config = Arc::clone(&self.server_config);
        let deadline = Duration::from_secs(conn.cfg.limits.auth_deadline_secs);
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
            let watchdog = tokio::spawn(async move {
                tokio::time::sleep(deadline).await;
                if !authed.load(Ordering::SeqCst) {
                    let _ = handle
                        .disconnect(
                            Disconnect::ByApplication,
                            "authentication timed out".into(),
                            String::new(),
                        )
                        .await;
                }
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
    up: ChannelWriteHalf<client::Msg>,
    pending: Pending,
    pump: tokio::task::JoinHandle<()>,
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
    cfg: Arc<Config>,
    id: u64,
    peer: SocketAddr,
    local: SocketAddr,
    slot: Slot,
    _live: LiveGuard,
    authed: Arc<AtomicBool>,
    started: Instant,
    user: String,
    service: Option<String>,
    method: &'static str,
    stage: Stage,
    up: Up,
    pending_notice: Option<String>,
    pairs: HashMap<ChannelId, Pair>,
    /// The pre-splice channel (menu, notice).
    front: Option<Channel<Msg>>,
    pty: Option<PtyReq>,
    env: Vec<(String, String)>,
    env_bytes: usize,
    menu_line: String,
    menu_esc: u8,
    menu_last: Arc<Mutex<Instant>>,
    menu_watch: Option<tokio::task::JoinHandle<()>>,
    bytes: Arc<Bytes>,
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
            biased;
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
                    let _ = ctx.down.eof().await;
                }
                Some(ChannelMsg::Close) => {
                    let _ = ctx.down.close().await;
                    break End::Closed;
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    let _ = ctx.down.exit_status(exit_status).await;
                }
                Some(ChannelMsg::ExitSignal { signal_name, core_dumped, error_message, lang_tag }) => {
                    let _ = ctx
                        .handle
                        .exit_signal_request(ctx.down_id, signal_name, core_dumped, error_message, lang_tag)
                        .await;
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
        End::Closed => {}
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
            let _ = ctx.down.close().await;
            let _ = ctx
                .handle
                .disconnect(
                    Disconnect::ByApplication,
                    reason.unwrap_or_else(|| "the game server closed the connection".into()),
                    String::new(),
                )
                .await;
        }
        End::Wedged => {
            tracing::info!("client stopped reading; closing");
            let _ = ctx
                .handle
                .disconnect(
                    Disconnect::ByApplication,
                    "the connection stopped reading".into(),
                    String::new(),
                )
                .await;
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

impl Conn {
    #[allow(clippy::too_many_arguments)]
    fn new(
        door: Arc<Doorway>,
        cfg: Arc<Config>,
        id: u64,
        peer: SocketAddr,
        local: SocketAddr,
        slot: Slot,
        live: LiveGuard,
        authed: Arc<AtomicBool>,
    ) -> Conn {
        Conn {
            door,
            cfg,
            id,
            peer,
            local,
            slot,
            _live: live,
            authed,
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
        let Some(svc) = self.cfg.service(service).cloned() else {
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
                tracing::warn!(conn = self.id, service, error = %e, "service unreachable");
                Dial::Tell(format!(
                    "{service} is not reachable right now; please try again in a minute."
                ))
            }
        }
    }

    /// `Begin` and, on `Ready`, start upstream SSH.
    async fn begin(&mut self, user: &str, key: Option<String>) -> Result<BeginOutcome, UpError> {
        let outcome = match &mut self.up {
            Up::Preamble { preamble, .. } => preamble.begin(user, key).await?,
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
        matches!(timed(handle.authenticate_none(user)).await, Ok(r) if r.success())
    }

    async fn up_password(&mut self, user: &str, password: &str) -> bool {
        let Up::Ssh { handle, .. } = &mut self.up else {
            return false;
        };
        matches!(timed(handle.authenticate_password(user, password)).await, Ok(r) if r.success())
    }

    /// The front door of `service`: `none` upstream as `upstream_user`.
    async fn front_door(&mut self, service: &str, upstream_user: &str) -> Auth {
        match self.dial(service).await {
            Dial::Tell(t) => return self.tell("none", t),
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
        match self.dial(service).await {
            Dial::Tell(t) => return self.tell("password", t),
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
        } else {
            reject_toward_interactive()
        }
    }

    fn farewell(&self) -> String {
        let service = self.service.as_deref().unwrap_or("the game server");
        let user = if self.user.is_empty() {
            service.to_owned()
        } else {
            self.user.clone()
        };
        format!(
            "{service} closed the connection; reconnect with ssh {user}@{}",
            self.cfg.menu.host
        )
    }

    fn pair_up(&mut self, down: Channel<Msg>, up: Channel<client::Msg>, handle: Handle) {
        let down_id = down.id();
        let (down_read, down_write) = down.split();
        // Downstream data reaches the doorway through `Handler::data`; the
        // channel's own queue is not read, and dropping its receiver keeps
        // russh from ever waiting on it.
        drop(down_read);
        let (up_read, up_write) = up.split();
        let pending: Pending = Arc::default();
        let inject = if self.pairs.is_empty() {
            let (tx, rx) = mpsc::channel(INJECT_QUEUE);
            self.door
                .drain
                .register(self.id, handle.clone(), Some(down_id), Some(tx));
            Some(rx)
        } else {
            None
        };
        let disconnect = match &self.up {
            Up::Ssh { disconnect, .. } => Arc::clone(disconnect),
            _ => DisconnectSlot::default(),
        };
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
        }));
        self.pairs.insert(
            down_id,
            Pair {
                up: up_write,
                pending,
                pump,
            },
        );
    }

    /// Send a request that wants a reply upstream, and wait for it.
    async fn relay_reply<F>(&self, ch: ChannelId, send: F) -> bool
    where
        F: AsyncFnOnce(&ChannelWriteHalf<client::Msg>) -> Result<(), russh::Error>,
    {
        let Some(pair) = self.pairs.get(&ch) else {
            return false;
        };
        let (tx, rx) = oneshot::channel();
        if let Ok(mut q) = pair.pending.lock() {
            q.push_back(tx);
        } else {
            return false;
        }
        if send(&pair.up).await.is_err() {
            if let Ok(mut q) = pair.pending.lock() {
                q.pop_back();
            }
            return false;
        }
        matches!(
            tokio::time::timeout(UPSTREAM_TIMEOUT, rx).await,
            Ok(Ok(true))
        )
    }

    fn write_front(&self, session: &mut Session, text: &str) {
        let Some(ch) = self.front.as_ref().map(Channel::id) else {
            return;
        };
        let mut bytes = Vec::with_capacity(text.len() + 8);
        for line in text.lines() {
            bytes.extend_from_slice(line.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        let _ = session.data(ch, bytes);
    }

    fn close_front(&mut self, session: &mut Session, text: &str) {
        self.write_front(session, text);
        if let Some(ch) = self.front.as_ref().map(Channel::id) {
            let _ = session.close(ch);
        }
        let _ = session.disconnect(Disconnect::ByApplication, "goodbye", "");
    }

    fn menu_text(&self) -> String {
        let mut out = format!("{}\n\n", self.cfg.menu.title);
        for (i, s) in self.cfg.services.iter().filter(|s| s.listed).enumerate() {
            out.push_str(&format!("  {}  {:<8} {}\n", i + 1, s.name, s.title));
        }
        out.push('\n');
        out
    }

    fn menu_prompt(&self, session: &mut Session) {
        if let Some(ch) = self.front.as_ref().map(Channel::id) {
            let _ = session.data(ch, b"Type a number or a name (q to quit): ".to_vec());
        }
    }

    fn start_menu_watch(&mut self, handle: Handle) {
        let last = Arc::clone(&self.menu_last);
        let idle = Duration::from_secs(self.cfg.limits.menu_idle_secs);
        let ch = self.front.as_ref().map(Channel::id);
        self.menu_watch = Some(tokio::spawn(async move {
            loop {
                let elapsed = last.lock().map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
                if elapsed >= idle {
                    if let Some(ch) = ch {
                        let _ = handle.data(ch, b"\r\nMenu timed out.\r\n".to_vec()).await;
                    }
                    let _ = handle
                        .disconnect(
                            Disconnect::ByApplication,
                            "menu timed out".into(),
                            String::new(),
                        )
                        .await;
                    return;
                }
                tokio::time::sleep(idle.saturating_sub(elapsed).max(Duration::from_millis(50)))
                    .await;
            }
        }));
    }

    async fn menu_input(&mut self, data: &[u8], session: &mut Session) {
        if let Ok(mut t) = self.menu_last.lock() {
            *t = Instant::now();
        }
        let Some(ch) = self.front.as_ref().map(Channel::id) else {
            return;
        };
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
                    let _ = session.data(ch, std::mem::take(&mut echo));
                    let _ = session.data(ch, b"\r\n".to_vec());
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
                    let _ = session.data(ch, std::mem::take(&mut echo));
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
        let _ = session.data(ch, echo);
    }

    /// Returns `true` when the menu is finished (spliced or closed).
    async fn menu_submit(&mut self, line: &str, session: &mut Session) -> bool {
        if line.is_empty() {
            self.menu_prompt(session);
            return false;
        }
        if line.eq_ignore_ascii_case("q") || line.eq_ignore_ascii_case("quit") {
            self.close_front(session, "Goodbye.");
            return true;
        }
        let listed: Vec<String> = self
            .cfg
            .services
            .iter()
            .filter(|s| s.listed)
            .map(|s| s.name.clone())
            .collect();
        let choice = match line.parse::<usize>() {
            Ok(n) if n >= 1 => listed.get(n - 1).cloned(),
            Ok(_) => None,
            // Unlisted services are reachable by name (Q-29-6).
            Err(_) => self.cfg.service(line).map(|s| s.name.clone()),
        };
        let Some(service) = choice else {
            self.write_front(session, "Unknown choice.");
            self.menu_prompt(session);
            return false;
        };
        self.splice_from_menu(&service, session).await;
        true
    }

    async fn splice_from_menu(&mut self, service: &str, session: &mut Session) {
        self.write_front(session, &format!("Connecting to {service}..."));
        if let Dial::Tell(t) = self.dial(service).await {
            self.close_front(session, &t);
            return;
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
                wait_reply(&mut up, &mut early).await;
            }
            for (k, v) in &self.env {
                up.set_env(false, k.as_str(), v.as_str()).await?;
            }
            up.request_shell(true).await?;
            wait_reply(&mut up, &mut early).await;
            Ok::<(), russh::Error>(())
        };
        if !matches!(
            tokio::time::timeout(UPSTREAM_TIMEOUT, replay).await,
            Ok(Ok(()))
        ) {
            self.close_front(session, &unreachable);
            return;
        }
        self.write_front(
            session,
            &format!(
                "Next time: ssh {service}+<account>@{}\n",
                self.cfg.menu.host
            ),
        );
        if let Some(ch) = self.front.as_ref().map(Channel::id)
            && !early.is_empty()
        {
            let _ = session.data(ch, early);
        }
        if let Some(w) = self.menu_watch.take() {
            w.abort();
        }
        let Some(front) = self.front.take() else {
            return;
        };
        self.user = format!("{service}+play");
        self.stage = Stage::Spliced;
        self.pair_up(front, up, session.handle());
    }
}

/// Wait for one `Success`/`Failure` on a not-yet-pumped channel, keeping any
/// data that arrives first.
async fn wait_reply(up: &mut Channel<client::Msg>, early: &mut Vec<u8>) {
    while let Some(msg) = up.wait().await {
        match msg {
            ChannelMsg::Success | ChannelMsg::Failure => return,
            ChannelMsg::Data { data } => early.extend_from_slice(&data),
            _ => {}
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        for (_, pair) in self.pairs.drain() {
            pair.pump.abort();
        }
        if let Some(w) = self.menu_watch.take() {
            w.abort();
        }
        self.drop_up();
        let segment = log_segment(&self.user);
        if self.authed.load(Ordering::SeqCst) {
            tracing::info!(
                conn = self.id,
                peer = %self.peer,
                user = %segment,
                service = self.service.as_deref().unwrap_or("-"),
                method = self.method,
                stage = ?std::mem::discriminant(&self.stage),
                secs = self.started.elapsed().as_secs(),
                up = self.bytes.up.load(Ordering::Relaxed),
                down = self.bytes.down.load(Ordering::Relaxed),
                "connection ended"
            );
        } else {
            self.door.limits.count("unauthenticated");
        }
    }
}

impl Handler for Conn {
    type Error = russh::Error;

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
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        match route(user, &self.cfg) {
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
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        match route(user, &self.cfg) {
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
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        match route(user, &self.cfg) {
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
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let Route::Service {
            service,
            upstream_user,
            front_door: false,
        } = route(user, &self.cfg)
        else {
            return Ok(match route(user, &self.cfg) {
                Route::Service { .. } => reject_toward_interactive(),
                _ => reject_nothing(),
            });
        };
        let Ok(openssh) = key.to_openssh() else {
            return Ok(reject_toward_interactive());
        };
        match self.dial(&service).await {
            Dial::Tell(t) => {
                // Let the client sign; the signed callback accepts and tells.
                self.pending_notice = Some(t);
                return Ok(Auth::Accept);
            }
            Dial::Opened => {}
        }
        let Up::Preamble { preamble, .. } = &mut self.up else {
            return Ok(reject_toward_interactive());
        };
        Ok(match preamble.key_query(&upstream_user, &openssh).await {
            Ok(true) => Auth::Accept,
            Ok(false) | Err(_) => reject_toward_interactive(),
        })
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if !self.slot.counted() {
            return Ok(reject_nothing());
        }
        self.user = user.to_owned();
        let Route::Service {
            service,
            upstream_user,
            front_door: false,
        } = route(user, &self.cfg)
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
                        self.pair_up(channel, up, session.handle());
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
                self.door
                    .drain
                    .register(self.id, session.handle(), Some(channel.id()), None);
                self.front = Some(channel);
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
        if self.pairs.contains_key(&channel) {
            let ok = self
                .relay_reply(channel, async |up| {
                    up.request_pty(true, term, cols, rows, px, py, modes).await
                })
                .await;
            return if ok {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        if self.front.as_ref().map(Channel::id) == Some(channel) && term.len() <= 64 {
            self.pty = Some(PtyReq {
                term: term.to_owned(),
                cols,
                rows,
                px,
                py,
                modes: modes.iter().take(64).copied().collect(),
            });
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
        if let Some(pair) = self.pairs.get(&channel) {
            let _ =
                tokio::time::timeout(UPSTREAM_TIMEOUT, pair.up.set_env(false, name, value)).await;
        }
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
        if let Some(pair) = self.pairs.get(&channel) {
            let _ =
                tokio::time::timeout(UPSTREAM_TIMEOUT, pair.up.window_change(cols, rows, px, py))
                    .await;
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
            let ok = self
                .relay_reply(channel, async |up| up.request_shell(true).await)
                .await;
            return if ok {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        if self.front.as_ref().map(Channel::id) != Some(channel) {
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
                self.start_menu_watch(session.handle());
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
            let ok = self
                .relay_reply(channel, async |up| up.exec(true, data.to_vec()).await)
                .await;
            return if ok {
                session.channel_success(channel)
            } else {
                session.channel_failure(channel)
            };
        }
        session.channel_failure(channel)?;
        if self.front.as_ref().map(Channel::id) == Some(channel) {
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
            let ok = self
                .relay_reply(channel, async |up| up.request_subsystem(true, name).await)
                .await;
            return if ok {
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
        if let Some(pair) = self.pairs.get(&channel) {
            let _ = tokio::time::timeout(UPSTREAM_TIMEOUT, pair.up.signal(signal)).await;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(pair) = self.pairs.get(&channel) {
            self.bytes
                .up
                .fetch_add(data.len() as u64, Ordering::Relaxed);
            return match tokio::time::timeout(WRITE_TIMEOUT, pair.up.data_bytes(data.to_vec()))
                .await
            {
                Ok(Ok(())) => Ok(()),
                // The upstream channel is gone; the pump tells the player.
                Ok(Err(_)) => Ok(()),
                Err(_) => {
                    tracing::info!(conn = self.id, "upstream stopped reading; closing");
                    Err(russh::Error::Disconnect)
                }
            };
        }
        if matches!(self.stage, Stage::Menu)
            && self.front.as_ref().map(Channel::id) == Some(channel)
        {
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
        if let Some(pair) = self.pairs.get(&channel)
            && tokio::time::timeout(
                WRITE_TIMEOUT,
                pair.up.extended_data_bytes(code, data.to_vec()),
            )
            .await
            .is_err()
        {
            return Err(russh::Error::Disconnect);
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(pair) = self.pairs.get(&channel) {
            let _ = tokio::time::timeout(UPSTREAM_TIMEOUT, pair.up.eof()).await;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(pair) = self.pairs.remove(&channel) {
            let _ = tokio::time::timeout(UPSTREAM_TIMEOUT, pair.up.close()).await;
            // Let the pump deliver the upstream close; it ends on its own.
            let pump = pair.pump;
            tokio::spawn(async move {
                tokio::time::sleep(UPSTREAM_TIMEOUT).await;
                pump.abort();
            });
        }
        if self.front.as_ref().map(Channel::id) == Some(channel) {
            self.front = None;
        }
        Ok(())
    }
}
