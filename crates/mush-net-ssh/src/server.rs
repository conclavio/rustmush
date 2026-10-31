//! The russh ingress: auth routing, welcome screen, output pump.
//!
//! All russh types are quarantined in this crate (treaty risk 4): the
//! public surface is [`SshConfig`], [`SshServer`]/[`BoundSshServer`],
//! [`SshError`], and the [`SshHooks`] trait.
//!
//! ## Per-connection lifecycle
//!
//! One [`ConnHandler`] per TCP connection (russh calls it for auth and
//! channel events). Username routing per design 02 §5.3 (M1 subset):
//! `play`/`guest`/`new` are accepted with `auth_none` and land on the
//! classic welcome screen; any other username is treated as an account
//! name and authenticated via password or keyboard-interactive against
//! [`SshHooks::auth_account`], or a verified registered public key.
//!
//! Interactive shells require a PTY. Verified public keys may use only the
//! bounded `mind push|pull|check` JSON exec service; subsystems are refused. Once a
//! character connects ([`SshHooks::connect_character`]), a per-connection
//! **output pump** task consumes the bounded event queue, renders each
//! [`OutputEvent`] through `mush-ansi` with the connection's live
//! capabilities, and performs the erase/redraw dance around the line
//! editor so async output never corrupts a half-typed command. When the
//! binary drops the queue's sender (session closed, `@boot`, QUIT), the
//! pump closes the transport; each write is bounded by [`WRITE_TIMEOUT`]
//! so a wedged client (e.g. `^S` flow control) is disconnected rather than
//! parking the pump forever (design 02 §8.4).
//!
//! Abuse gates (doc 29a §9, the sshdoorway reviews applied here): the
//! listener runs its **own accept loop** ([`serve_accepts`]), which survives
//! an accept error (an `EMFILE` burst used to end russh's loop and take the
//! port down until a restart) and asks [`SshHooks::admit_connection`] about
//! every accepted TCP connection **before any SSH byte**: a refused rate,
//! global or /48 cap closes the socket with no handshake, an over-cap
//! address is told in the authentication banner and cut within five
//! seconds, and an admitted one holds its slot until the handler drops
//! (released exactly once, [`SshHooks::release_connection`]). Every accepted
//! stream sits in a [`crate::kill::KillSwitch`]: a connection must
//! authenticate within [`SshConfig::auth_deadline`] of accept and open a
//! channel within another, or it is disconnected and, [`DISCONNECT_GRACE`]
//! later, cut — russh cannot abort its own session task, so a client stalled
//! in the key exchange or ignoring the disconnect would otherwise hold its
//! slot and descriptor forever. Every other server-initiated disconnect (the
//! welcome screen's farewell, the pre-auth watchdog, the pump's close after
//! the game ended the session) arms the same cut. In-game connections are
//! never touched by any of it: there is no inactivity timeout, by design.
//! In-band welcome `connect`/`create` re-check [`SshHooks::is_ip_blocked`]
//! on every attempt (so a tempban armed mid-connection stops further
//! guessing), and a connection that has not reached a character within
//! [`PREAUTH_TIMEOUT`] of its channel open is dropped.
//!
//! A blocked address is **told**. SSH has exactly one place to put words in
//! front of a client it is refusing — the authentication banner, sent before
//! the first method is tried — so that is where [`SshHooks::block_reason`]
//! lands, and every auth callback then answers with [`reject_blocked`]: a
//! `USERAUTH_FAILURE` offering no further methods, which an OpenSSH client
//! reads as "give up" rather than "try a password". Before this, a blocked
//! address met the auth-none front door with a bare `Auth::reject()`, fell
//! through to `password`, and was left typing at a prompt that could never
//! succeed and never said why.
//!
//! Deliberate M1 deferrals from design 02 §5.3 (recorded here so they read
//! as decisions, not oversights): welcome-screen `WHO` is stubbed until
//! pre-auth world queries exist, and in-band `connect`/`create` passwords
//! are echoed in the clear (the classic front door; the wire is encrypted
//! and the line is excluded from editor history) — masked prompts arrive
//! with the M2 account wizard.

use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_ansi::AnsiTheme;
use mush_pixel::caps::{CellSize, Encoder, EncoderChoice};
use mush_proto::{ClientCaps, InputMode, OutputEvent, PanelFrameKind, RichText};
use mush_session::{FloodBucket, FloodVerdict, sanitize_line};
use russh::server::{Auth, Config, Handler, Msg, Response, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::editor::{EchoMode, EditorAction, LineEditor};
use crate::hooks::{
    Admission, AuthResult, BlockReason, CharSummary, NewConn, PixelEvent, SshHooks,
};
use crate::kill::{KillSwitch, Killable};
use crate::tui::{TuiManifest, TuiSession};
use crate::welcome::{WelcomeCmd, parse_welcome};
use crate::{caps, hostkey};

/// Per-connection output queue depth (design 02 §8.4).
const OUTPUT_BUFFER: usize = 256;

/// Pre-auth (welcome screen) flood limits: sustained lines/second and
/// burst. Harsher than the in-game bucket — this funnel is unauthenticated.
const PREAUTH_FLOOD_RATE: f64 = 1.0;
const PREAUTH_FLOOD_BURST: f64 = 5.0;

/// How long a connection may sit without reaching a character before it is
/// dropped (design 02 §8.2: pre-auth phase timeout). A fixed deadline from
/// channel-open, not an idle timer — in-game idlers are never affected.
const PREAUTH_TIMEOUT: Duration = Duration::from_secs(120);

/// The default [`SshConfig::auth_deadline`]: a freshly accepted connection
/// must authenticate within this, and then open a session channel within
/// another, or it is disconnected and cut (doc 29a §7 F1, §9). Before this
/// nothing bounded a connection until it opened a channel: a client stalled
/// in the key exchange, or authenticated and idle, held its task and
/// descriptor forever. The doorway's `auth_deadline_secs` has the same
/// default.
pub const DEFAULT_AUTH_DEADLINE: Duration = Duration::from_secs(30);

/// Grace after a server-initiated `SSH_MSG_DISCONNECT` before the TCP stream
/// is cut through its [`KillSwitch`]. russh half-closes the write side on a
/// disconnect and then reads until the client's EOF — which a half-open
/// client never sends, and `inactivity_timeout` is `None` on purpose — so
/// without the cut a disconnected connection could hold its slot and
/// descriptor for ever (doc 29a §8 F-1). Five seconds is long enough for a
/// slow link to see the farewell (§8 A-5).
pub const DISCONNECT_GRACE: Duration = Duration::from_secs(5);

/// An over-cap connection ([`Admission::OverCap`]) is told in the
/// authentication banner and refused at its first auth method; one that
/// never tries a method is disconnected at this deadline from accept and
/// cut [`OVER_CAP_GRACE`] later — five seconds in all, so over-cap churn
/// cannot hold global capacity (doc 29a §7 "Splice and DoS" 3). Capped by
/// the configured `auth_deadline` when that is shorter.
const OVER_CAP_DEADLINE: Duration = Duration::from_secs(3);
const OVER_CAP_GRACE: Duration = Duration::from_secs(2);

/// After an accept error, how long the loop waits before accepting again.
/// The listener itself is still valid after `EMFILE` and friends; the pause
/// keeps a descriptor-exhausted process from spinning (doc 29a §8 B-2).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// What an over-cap connection is told in the authentication banner.
const OVER_CAP_NOTICE: &str = "Too many connections from your address; try again later.";
/// Pre-connect input is echoed, so a client that stops reading could make the
/// server queue its own keystrokes without bound. Input that arrives while
/// the channel's window is exhausted is dropped instead of echoed, and this
/// many such arrivals in a row close the connection. One coincidence does
/// not: a typist whose window refills within a round trip is left alone
/// (doc 29a §7, round 2).
const PENDING_STRIKES: u8 = 3;

/// Cap on one output-pump write to the transport (design 02 §8.4: "try 5 s
/// then disconnect"). A client that stops reading (terminal `^S`, dead
/// peer behind a live TCP window) stalls `handle.data` on SSH window
/// space; this bounds it so the pump can close the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the TUI's WHO rail re-queries the world while `Mode::Tui` is
/// active (design M15 D5: a read-only 3-5 s refresh that does *not* stamp
/// idle — the pump submits the query, it never types the WHO command).
const WHO_REFRESH: Duration = Duration::from_secs(4);

/// How long an `@history` page in flight is waited for before the request
/// re-arms (interface plan I12). It has to *age* rather than latch: the
/// gateway drops a throttled `narrative.backlog` query in silence — no
/// reply, no error — so a plain in-flight flag would wedge `@history` for
/// the rest of the connection.
const HISTORY_LATCH: Duration = Duration::from_secs(5);
/// Lines `@history` asks for when given no count.
const HISTORY_DEFAULT: u32 = 20;
/// The most `@history` asks for at once — deliberately under the gateway's
/// own 1..=200 clamp, because a page is one `handle.data` write raced
/// against [`WRITE_TIMEOUT`] and is rendered under the connection lock.
const HISTORY_MAX: u32 = 100;
const HISTORY_USAGE: &str = "Usage: @history [<1-100>] [more]";

/// Doc 21 cut G.10: `@sounds`' usage line.
const SOUNDS_USAGE: &str = "Usage: @sounds [on|off]";

/// Doc 21 cut G.2: `@pixel/encoder`'s usage line.
const PIXEL_ENCODER_USAGE: &str = "Usage: @pixel/encoder [auto|kitty|sixel|off]";

const WELCOME_HELP: &str = "\
  connect <account> <password>   connect to an existing account\n\
  create <account> <password>    create a new account and character\n\
  WHO                            (available once connected)\n\
  QUIT                           disconnect";

const BAD_CREDS_MSG: &str = "Either that account does not exist, or the password is wrong.";
const QUIT_CONFIRM_MSG: &str = "Really disconnect? Type y to confirm, anything else to cancel.";

/// Configuration for [`SshServer::run`].
#[derive(Clone, Debug)]
pub struct SshConfig {
    /// Address to listen on, e.g. `0.0.0.0:2222` (port 0 picks an
    /// ephemeral port — see [`BoundSshServer::local_addr`]).
    pub listen: SocketAddr,
    /// Path of the ed25519 host key; generated on first boot and never
    /// regenerated.
    pub host_key_path: PathBuf,
    /// Welcome-screen banner (LF line endings fine; converted to CRLF).
    pub banner: String,
    /// How long a connection has to authenticate after accept, and then to
    /// open a session channel after that, before it is disconnected and cut
    /// ([`DEFAULT_AUTH_DEADLINE`]). Never applied to an in-game connection.
    pub auth_deadline: Duration,
}

/// Errors from binding or running the SSH server.
#[derive(Debug)]
pub enum SshError {
    /// The host key could not be loaded or generated.
    HostKey {
        /// The offending path.
        path: String,
        /// What went wrong.
        detail: String,
    },
    /// Could not bind the listen address.
    Bind {
        /// The address that failed.
        addr: SocketAddr,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The accept loop failed.
    Server(std::io::Error),
    /// Could not bind the doc 29 trusted ingress socket.
    BindTrusted {
        /// The socket path that failed.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshError::HostKey { path, detail } => {
                write!(f, "SSH host key at {path}: {detail}")
            }
            SshError::Bind { addr, source } => write!(f, "binding {addr}: {source}"),
            SshError::Server(e) => write!(f, "SSH server: {e}"),
            SshError::BindTrusted { path, source } => {
                write!(f, "binding the SSH trusted ingress {path}: {source}")
            }
        }
    }
}

impl std::error::Error for SshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SshError::Bind { source, .. }
            | SshError::Server(source)
            | SshError::BindTrusted { source, .. } => Some(source),
            SshError::HostKey { .. } => None,
        }
    }
}

/// The SSH ingress. [`SshServer::run`] is the one-call entry point; use
/// [`SshServer::bind`] when you need the bound address first (tests,
/// ephemeral ports).
pub struct SshServer;

impl SshServer {
    /// Bind, then serve forever (or until the accept loop fails).
    pub async fn run(cfg: SshConfig, hooks: Arc<dyn SshHooks>) -> Result<(), SshError> {
        Self::bind(cfg, hooks).await?.run().await
    }

    /// Bind the doc 29 trusted ingress (a Unix socket only sshdoorway can
    /// reach) without accepting yet. See [`crate::trusted`].
    #[cfg(unix)]
    pub async fn bind_trusted(
        cfg: crate::trusted::TrustedConfig,
        hooks: Arc<dyn SshHooks>,
    ) -> Result<crate::trusted::BoundTrustedServer, SshError> {
        crate::trusted::BoundTrustedServer::bind(cfg, hooks).await
    }

    /// Load/generate the host key and bind the listener without accepting
    /// yet.
    pub async fn bind(
        cfg: SshConfig,
        hooks: Arc<dyn SshHooks>,
    ) -> Result<BoundSshServer, SshError> {
        let config = server_config(&cfg.host_key_path)?;
        let listener = TcpListener::bind(cfg.listen)
            .await
            .map_err(|source| SshError::Bind {
                addr: cfg.listen,
                source,
            })?;
        let addr = listener.local_addr().map_err(SshError::Server)?;
        tracing::info!(%addr, "SSH server listening");
        Ok(BoundSshServer {
            addr,
            listener,
            config,
            ingress: Ingress::new(hooks, &cfg.banner),
            auth_deadline: cfg.auth_deadline,
        })
    }
}

/// The russh configuration every rustmush listener shares (the public TCP
/// port and the doc 29 trusted ingress alike).
pub(crate) fn server_config(host_key_path: &std::path::Path) -> Result<Arc<Config>, SshError> {
    let key = hostkey::load_or_generate(host_key_path)?;
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::None);
    methods.push(MethodKind::PublicKey);
    methods.push(MethodKind::Password);
    methods.push(MethodKind::KeyboardInteractive);
    let config = Arc::new(Config {
        server_id: russh::SshId::Standard(Cow::Borrowed("SSH-2.0-rustmush_0.1")),
        methods,
        keys: vec![key],
        // Idlers are furniture (design 02 §5.6): no inactivity timeout;
        // dead links are caught by keepalives instead.
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        nodelay: true,
        ..Config::default()
    });
    Ok(config)
}

/// A bound-but-not-yet-serving SSH server.
pub struct BoundSshServer {
    addr: SocketAddr,
    listener: TcpListener,
    config: Arc<Config>,
    ingress: Ingress,
    auth_deadline: Duration,
}

impl BoundSshServer {
    /// The actual bound address (resolves ephemeral ports).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serve until the task is dropped. The loop itself never ends: an
    /// accept error is logged and retried (`serve_accepts`), so the
    /// `Result` is kept only for the signature's sake.
    pub async fn run(self) -> Result<(), SshError> {
        let BoundSshServer {
            listener,
            config,
            ingress,
            auth_deadline,
            ..
        } = self;
        serve_accepts(listener, |stream, peer| {
            ingress.handle(stream, peer, Arc::clone(&config), auth_deadline);
        })
        .await;
        Ok(())
    }
}

/// Where accepted connections come from: the bound listener, or a test's
/// script of errors and streams.
trait Acceptor {
    async fn accept(&mut self) -> std::io::Result<(TcpStream, SocketAddr)>;
}

impl Acceptor for TcpListener {
    async fn accept(&mut self) -> std::io::Result<(TcpStream, SocketAddr)> {
        TcpListener::accept(self).await
    }
}

/// Accept for ever. An accept error — `EMFILE` under a descriptor burst, a
/// transient network error — is logged and retried after
/// [`ACCEPT_BACKOFF`]: the listener itself is still valid, and russh's own
/// `run_on_socket` returned on the first such error, which took the whole
/// SSH port down until a restart while the web side kept serving (doc 29a
/// §8 B-2, applied here in §9).
async fn serve_accepts<A: Acceptor>(
    mut listener: A,
    mut on_accept: impl FnMut(TcpStream, SocketAddr),
) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => on_accept(stream, peer),
            Err(error) => {
                tracing::warn!(%error, "SSH accept failed; retrying");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

// ---- per-connection machinery ------------------------------------------

#[derive(Clone)]
pub(crate) struct Ingress {
    pub(crate) hooks: Arc<dyn SshHooks>,
    banner: Arc<str>,
    next_conn: Arc<AtomicU64>,
}

impl Ingress {
    pub(crate) fn new(hooks: Arc<dyn SshHooks>, banner: &str) -> Ingress {
        Ingress::with_base(hooks, banner, 1)
    }

    /// An ingress whose connection ids start at `base`. Every listener in one
    /// process must allocate from a disjoint range: the ids key the session
    /// manager, and two listeners both counting from 1 would hand two players
    /// the same id (doc 29a §8 B-1). The public port starts at 1, the trusted
    /// ingress at [`crate::trusted::TRUSTED_CONN_ID_BASE`]; the web side
    /// starts at 2^32 and Discord at 2^48. The hooks' generation bit
    /// ([`SshHooks::conn_id_bit`], doc 33 §4.3) is ORed into the base, so
    /// two gateways of one server allocate from disjoint ranges too.
    pub(crate) fn with_base(hooks: Arc<dyn SshHooks>, banner: &str, base: u64) -> Ingress {
        let first = base | hooks.conn_id_bit();
        Ingress {
            hooks,
            banner: Arc::from(banner),
            next_conn: Arc::new(AtomicU64::new(first)),
        }
    }

    /// Run one accepted TCP connection: ask the hooks whether to serve it
    /// at all, and if so run SSH on a killable stream under the two
    /// deadlines (authenticate within `auth_deadline`, open a channel
    /// within another) that end in a cut, so no connection can hold its
    /// slot and descriptor without earning them.
    pub(crate) fn handle(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        config: Arc<Config>,
        auth_deadline: Duration,
    ) {
        let addr = peer.ip();
        let admission = self.hooks.admit_connection(addr);
        if admission == Admission::Drop {
            // Closed with no handshake: a flood pays for no key exchange.
            tracing::debug!(%peer, "SSH connection dropped at accept");
            return;
        }
        let _ = stream.set_nodelay(true);
        let handler = self.handler_for_admission(addr, admission, auth_deadline);
        let conn_id = handler.conn_id;
        let switch = Arc::clone(&handler.kill);
        let authed = Arc::clone(&handler.authed);
        let opened = Arc::clone(&handler.opened);
        let (deadline, grace) = if admission == Admission::OverCap {
            (OVER_CAP_DEADLINE.min(auth_deadline), OVER_CAP_GRACE)
        } else {
            (auth_deadline, DISCONNECT_GRACE)
        };
        let stream = Killable::new(stream, Arc::clone(&switch));
        tokio::spawn(async move {
            // `run_stream` exchanges version lines and spawns the session
            // task; a client that never sends its line is dropped here
            // (dropping the future drops the stream and the handler).
            let session = match tokio::time::timeout(
                deadline,
                russh::server::run_stream(config, stream, handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Ok(Err(error)) => {
                    tracing::debug!(conn = conn_id, %error, "SSH start failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(conn = conn_id, "SSH version exchange timed out");
                    return;
                }
            };
            let handle = session.handle();
            // Two deadlines, so nothing holds a slot for ever: authenticate
            // within `deadline`, open a channel within another. The
            // disconnect is polite; the cut after `grace` is not, and it is
            // unconditional: a client stalled in the key exchange never even
            // has the disconnect processed (russh takes no message while a
            // kex is active), and one that ignores it holds the read until
            // EOF (doc 29a §8 F-1, F-3).
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
                tracing::info!(conn = conn_id, "{why}; disconnecting");
                let _ = tokio::time::timeout(
                    grace,
                    handle.disconnect(Disconnect::ByApplication, why.into(), String::new()),
                )
                .await;
                tokio::time::sleep(grace).await;
                switch.kill();
            });
            if let Err(error) = session.await {
                tracing::debug!(conn = conn_id, %error, "SSH session ended with error");
            }
            watchdog.abort();
        });
    }

    /// One connection's handler, for an address whose per-IP slot has
    /// already been asked for with [`SshHooks::connection_opened`]:
    /// `counted` is its answer, and the handler's `Drop` calls
    /// [`SshHooks::connection_closed`] exactly when it is `true`. The doc 29
    /// trusted ingress asks at its preamble's `Hello`, for the address the
    /// doorway names, and builds its handler here; the public TCP port asks
    /// at accept and uses [`Ingress::handler_for_admission`].
    pub(crate) fn handler_for(&self, addr: IpAddr, counted: bool) -> ConnHandler {
        self.handler_with(addr, Slot::Counted(counted), DEFAULT_AUTH_DEADLINE)
    }

    /// One connection's handler for an [`SshHooks::admit_connection`]
    /// answer; its `Drop` releases it with
    /// [`SshHooks::release_connection`]. `admission` is never
    /// [`Admission::Drop`]: that connection got no handler.
    pub(crate) fn handler_for_admission(
        &self,
        addr: IpAddr,
        admission: Admission,
        auth_deadline: Duration,
    ) -> ConnHandler {
        self.handler_with(addr, Slot::Gated(admission), auth_deadline)
    }

    fn handler_with(&self, addr: IpAddr, slot: Slot, auth_deadline: Duration) -> ConnHandler {
        let conn_id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        if slot.refused() {
            tracing::info!(conn = conn_id, %addr, "per-IP connection cap hit; will refuse");
        }
        tracing::debug!(conn = conn_id, %addr, "SSH connection");
        ConnHandler {
            hooks: Arc::clone(&self.hooks),
            banner: Arc::clone(&self.banner),
            conn_id,
            addr,
            shared: Arc::new(Mutex::new(Shared {
                editor: LineEditor::new(),
                base_echo: EchoMode::Normal,
                caps: caps::detect_caps(None, None, None, None, 0, 0),
                term: None,
                colorterm: None,
                lang: None,
                lc_all: None,
                width: 0,
                height: 0,
                cell_size: None,
                mode: Mode::Welcome,
                tui: None,
                manifest: TuiManifest::default(),
                repaint: Arc::new(Notify::new()),
                output_fence: None,
                history_rendered: Arc::new(Notify::new()),
                history: HistoryCursor::default(),
                history_request: None,
                next_history_request: 1 << 63,
                mode_generation: 0,
                pixel_choice: EncoderChoice::Auto,
                sounds_on: true,
            })),
            route: Route::PreAuth,
            auth_method: AuthMethod::None,
            exec_input: None,
            exec_submitted: false,
            channel: None,
            sftp: None,
            sftp_started: false,
            pty_seen: false,
            shell_started: false,
            connected: Arc::new(AtomicBool::new(false)),
            preauth_watchdog: None,
            commands: None,
            command_task: None,
            slot,
            authed: Arc::new(AtomicBool::new(false)),
            opened: Arc::new(AtomicBool::new(false)),
            kill: KillSwitch::new(),
            auth_deadline,
            flood: FloodBucket::new(PREAUTH_FLOOD_RATE, PREAUTH_FLOOD_BURST, Instant::now()),
            pending_strikes: 0,
            attested: None,
        }
    }
}

/// How a connection's per-address accounting was claimed, so `Drop`
/// releases it the same way.
#[derive(Clone, Copy, Debug)]
enum Slot {
    /// [`SshHooks::connection_opened`] answered (`true` = counted; released
    /// with [`SshHooks::connection_closed`]).
    Counted(bool),
    /// [`SshHooks::admit_connection`] answered; released with
    /// [`SshHooks::release_connection`].
    Gated(Admission),
}

impl Slot {
    /// Over the address's cap: told, and refused at the first auth method.
    fn refused(self) -> bool {
        matches!(
            self,
            Slot::Counted(false) | Slot::Gated(Admission::OverCap | Admission::Drop)
        )
    }
}

impl ConnHandler {
    /// This connection's id (tests).
    #[cfg(test)]
    pub(crate) fn conn_id(&self) -> u64 {
        self.conn_id
    }
}

/// Where the connection is in the login state machine.
#[derive(Clone, Debug)]
enum Mode {
    /// Classic welcome screen (`play`/`guest`/`new`).
    Welcome,
    /// Post-auth character selection.
    CharSelect {
        account: String,
        chars: Vec<CharSummary>,
    },
    /// Connected to a character; lines go to [`SshHooks::line`].
    InGame,
    /// Full-screen ratatui TUI (milestone M15), entered from `InGame` with
    /// `@tui` and left with `@notui` / the detach hotkey. Raw input drives
    /// the TUI app (not the line editor) and the pump repaints frames; the
    /// [`TuiSession`] itself lives in [`Shared::tui`].
    Tui,
    /// `^D` quit confirmation; `prev` is restored on cancel.
    QuitConfirm { prev: Box<Mode> },
}

/// Only the authoritative signed-key callback records PublicKey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthMethod {
    None,
    Password,
    PublicKey,
}

/// How SSH-level auth resolved.
#[derive(Clone, Debug)]
enum Route {
    PreAuth,
    /// Front-door username: welcome screen, in-band connect/create.
    Welcome,
    /// `<account>` username authenticated via [`SshHooks::auth_account`]
    /// (password/keyboard-interactive) or [`SshHooks::auth_account_key`]
    /// (public key). `requested_char` carries the `+<char>` part of an
    /// `account+char` username (M19); it can only ever select a character
    /// this authenticated account owns (enforced in `enter_account`).
    Account {
        account: String,
        characters: Vec<CharSummary>,
        requested_char: Option<String>,
    },
}

/// State shared between the russh handler and the output pump. The mutex
/// is `std::sync` and never held across an await.
struct Shared {
    editor: LineEditor,
    /// PTY echo preference, restored after a server input prompt.
    base_echo: EchoMode,
    caps: ClientCaps,
    term: Option<String>,
    colorterm: Option<String>,
    lang: Option<String>,
    lc_all: Option<String>,
    width: u16,
    height: u16,
    /// Cell dimensions in pixels when the SSH client supplied `pix_width` and
    /// `pix_height` on `pty-req` or `window-change`. `CSI 16 t` in `@tui`
    /// remains the fallback when the client sends `0`.
    cell_size: Option<CellSize>,
    mode: Mode,
    /// Active TUI state while [`Mode::Tui`]; `None` in every other mode.
    /// Reached under this same lock by both the input handler and the output
    /// pump — rendering is synchronous, the async write happens after the
    /// lock drops (invariant T2).
    tui: Option<Box<TuiSession>>,
    /// What the server announced in its `Hello` / `SessionInfo`, kept so a
    /// TUI opened later knows the panel manifest and the character
    /// (interface plan I2).
    manifest: TuiManifest,
    /// Signals the output pump to repaint the TUI without a world event
    /// (resize, input edit). Only pulsed while `Mode::Tui`, so line mode is
    /// never woken by it (invariant T1).
    repaint: Arc<Notify>,
    output_fence: Option<tokio::sync::oneshot::Sender<()>>,
    history_rendered: Arc<Notify>,
    /// Where line mode's `@history` has read back to (interface plan I12).
    history: HistoryCursor,
    /// Only this correlated page may consume either mode's paging latch.
    history_request: Option<u64>,
    next_history_request: u64,
    mode_generation: u64,
    /// `@pixel/encoder` for this connection (doc 21 cut G.2; Q-21-2 asks
    /// for it per account, which waits on a per-account store). Kept here,
    /// not in the TUI, so it survives `@notui` and applies at the next `@tui`.
    pixel_choice: EncoderChoice,
    /// `@sounds on|off` for this connection (doc 21 cut G.10), kept here for
    /// the same reason.
    sounds_on: bool,
}

/// Per-connection `@history` paging state. The resume ring is per-session;
/// the cursor is per-connection, so two connections attached to one session
/// page through it independently.
#[derive(Debug)]
struct HistoryCursor {
    /// `before_seq` for the next page; `None` asks from the newest.
    before: Option<u64>,
    /// The server may hold older lines below the last page shown.
    more: bool,
    /// A page is in flight, and when it was asked for — an aging latch (see
    /// [`HISTORY_LATCH`]), never a plain flag.
    pending: Option<Instant>,
}

impl Default for HistoryCursor {
    fn default() -> HistoryCursor {
        // `more: true` — nothing has been read yet, so everything the ring
        // holds is still askable. `#[derive(Default)]` would say `false`.
        HistoryCursor {
            before: None,
            more: true,
            pending: None,
        }
    }
}

impl Shared {
    fn recompute_caps(&mut self) {
        self.caps = caps::detect_caps(
            self.term.as_deref(),
            self.colorterm.as_deref(),
            self.lang.as_deref(),
            self.lc_all.as_deref(),
            self.width,
            self.height,
        );
    }
}

pub(crate) struct ConnHandler {
    auth_method: AuthMethod,
    exec_input: Option<crate::mind_exec::ExecInput>,
    exec_submitted: bool,
    hooks: Arc<dyn SshHooks>,
    banner: Arc<str>,
    conn_id: u64,
    pub(crate) addr: IpAddr,
    shared: Arc<Mutex<Shared>>,
    route: Route,
    channel: Option<ChannelId>,
    pty_seen: bool,
    shell_started: bool,
    /// **The asset pack's channel** (doc 21 cut G.11): a second session
    /// channel, opened by a client in game, held until it asks for the
    /// `sftp` subsystem — the one thing it may ask for.
    sftp: Option<Channel<Msg>>,
    /// The pack's `sftp` session has started on this connection (one a
    /// connection).
    sftp_started: bool,
    /// Set just before `connect_character` is attempted (cleared again on
    /// refusal); gates the `disconnected` hook (fired exactly once, from
    /// `Drop`). Armed *before* the await so a handler future dropped
    /// mid-connect still gets its cleanup — the binary treats a
    /// `disconnected` for a never-attached conn as a no-op.
    connected: Arc<AtomicBool>,
    /// Owned by the transport so a closed connection cannot later log a timeout.
    preauth_watchdog: Option<tokio::task::JoinHandle<()>>,
    /// Bounded typed-command queue; the SSH event loop must remain free to write.
    commands: Option<mpsc::Sender<String>>,
    command_task: Option<tokio::task::JoinHandle<()>>,
    /// What this connection claimed at accept, released exactly once in
    /// `Drop`. A refused slot means over-cap: told in the banner, refused at
    /// the first auth method.
    slot: Slot,
    /// Set by russh once any auth method succeeded; read by the accept-time
    /// watchdog's first deadline.
    authed: Arc<AtomicBool>,
    /// Set when the first session channel opens; read by the watchdog's
    /// second deadline.
    opened: Arc<AtomicBool>,
    /// The switch on this connection's TCP stream (doc 29a §9). Armed after
    /// every server-initiated disconnect, so a client that ignores it cannot
    /// keep the connection half-open; unattached (and harmless) for a
    /// handler that was not made by [`Ingress::handle`].
    kill: Arc<KillSwitch>,
    /// [`SshConfig::auth_deadline`]: also the grace a client gets to answer
    /// the server's channel close with its own before it is disconnected.
    auth_deadline: Duration,
    flood: FloodBucket,
    /// Consecutive pre-connect inputs that arrived while the channel's window
    /// was exhausted (see [`PENDING_STRIKES`]).
    pending_strikes: u8,
    /// Doc 29: the account the trusted ingress's preamble bound to this
    /// connection, after the doorway attested the player holds a key that
    /// [`SshHooks::auth_account_key`] accepted. `auth_none` for exactly
    /// `user` then logs in as a public-key login. Always `None` on the public
    /// TCP port, which never reads a preamble.
    pub(crate) attested: Option<Attested>,
}

/// What a doc 29 `Begin` with an attested key bound (see
/// [`ConnHandler::attested`]).
#[derive(Clone, Debug)]
pub(crate) struct Attested {
    /// The exact upstream username the doorway will authenticate as.
    pub(crate) user: String,
    pub(crate) account: String,
    pub(crate) characters: Vec<CharSummary>,
}

impl Drop for ConnHandler {
    fn drop(&mut self) {
        if let Some(watchdog) = self.preauth_watchdog.take() {
            watchdog.abort();
        }
        if let Some(task) = self.command_task.take() {
            task.abort();
        }
        // Release the per-IP slot symmetrically with the admission — every
        // path (auth refused, welcome squatter, mid-connect teardown, clean
        // quit, a cut) comes through here exactly once.
        match self.slot {
            Slot::Counted(true) => self.hooks.connection_closed(self.addr),
            Slot::Counted(false) => {}
            Slot::Gated(admission) => self.hooks.release_connection(self.addr, admission),
        }
        if self.connected.swap(false, Ordering::SeqCst) {
            let hooks = Arc::clone(&self.hooks);
            let conn = self.conn_id;
            // The handler is dropped inside the session task (tokio
            // context); outside one (runtime teardown) the hook is moot.
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move { hooks.disconnected(conn).await });
            }
        }
    }
}

/// Case-insensitively strip a leading `verb` from `line`, returning the
/// remainder when the verb is followed by whitespace or the end of line.
fn strip_verb<'a>(line: &'a str, verb: &str) -> Option<&'a str> {
    let head = line.get(..verb.len())?;
    if !head.eq_ignore_ascii_case(verb) {
        return None;
    }
    let rest = &line[verb.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

/// What `@pixel/stats` says outside `@tui`, where no picture is drawn.
const PIXEL_STATS_OUTSIDE_TUI: &str =
    "The picture is drawn inside @tui (Ctrl-G there); its numbers are told there.";

/// `@pixel/stats` (doc 21 cut G.1b), read transport-side: this connection's
/// picture in numbers. Whatever follows the verb is ignored.
fn is_pixel_stats(line: &str) -> bool {
    strip_verb(line.trim(), "@pixel/stats").is_some()
}

/// `@sounds [on|off]`, read transport-side (doc 21 cut G.10): the cockpit's
/// sounds for this connection. `Some(None)` is the bare verb (say what is
/// set), `Some(Some(on))` a setting; anything else after the verb is `Err`.
fn parse_sounds(line: &str) -> Option<Result<Option<bool>, ()>> {
    let rest = strip_verb(line.trim(), "@sounds")?;
    let mut words = rest.split_whitespace();
    Some(match (words.next(), words.next()) {
        (None, _) => Ok(None),
        (Some(word), None) if word.eq_ignore_ascii_case("on") => Ok(Some(true)),
        (Some(word), None) if word.eq_ignore_ascii_case("off") => Ok(Some(false)),
        _ => Err(()),
    })
}

/// `@sounds`' answer: on or off, and — inside `@tui`, where the probe ran —
/// whether the terminal hears them by name (w11term) or as its bell.
fn sounds_reply(on: bool, report: Option<(bool, Option<String>, bool)>) -> String {
    let set = if on {
        "Sounds are on for this connection."
    } else {
        "Sounds are off for this connection."
    };
    let then = match report {
        Some((_, Some(terminal), true)) => {
            format!(" Your terminal ({terminal}) plays them by name.")
        }
        Some((_, Some(terminal), false)) => {
            format!(" Your terminal ({terminal}) hears them as its bell.")
        }
        Some((_, None, _)) => " Your terminal hears them as its bell.".to_string(),
        None => " The cockpit's sounds are played inside @tui.".to_string(),
    };
    format!("{set}{then}")
}

/// `@pixel/encoder [auto|kitty|sixel|off]`, read transport-side (doc 21 cut
/// G.2): the picture's encoder for this connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelEncoderReq {
    /// Bare: say what is set and what this terminal showed.
    Show,
    /// Set it.
    Set(EncoderChoice),
    /// Anything else after the verb.
    Usage,
}

/// Read `@pixel/encoder`; `None` for any other line.
fn parse_pixel_encoder(line: &str) -> Option<PixelEncoderReq> {
    let rest = strip_verb(line.trim(), "@pixel/encoder")?;
    let mut words = rest.split_whitespace();
    Some(match (words.next(), words.next()) {
        (None, _) => PixelEncoderReq::Show,
        (Some(word), None) => {
            EncoderChoice::parse(word).map_or(PixelEncoderReq::Usage, PixelEncoderReq::Set)
        }
        _ => PixelEncoderReq::Usage,
    })
}

/// `@pixel/encoder`'s answer: what is set, and — inside `@tui`, where the
/// probe ran — what the terminal showed and what `^G` draws in.
fn pixel_encoder_reply(
    req: PixelEncoderReq,
    choice: EncoderChoice,
    report: Option<(&str, Option<Encoder>)>,
) -> String {
    let set = match req {
        PixelEncoderReq::Usage => return PIXEL_ENCODER_USAGE.to_string(),
        PixelEncoderReq::Set(_) => {
            format!("Picture encoder for this connection: {}.", choice.name())
        }
        PixelEncoderReq::Show => format!("Picture encoder: {}.", choice.name()),
    };
    let then = match report {
        Some((probe, Some(encoder))) => {
            format!(
                " Your terminal: {probe}. Ctrl-G draws the picture in {}.",
                encoder.name()
            )
        }
        Some((probe, None)) if choice == EncoderChoice::Off => {
            format!(" Your terminal: {probe}. Ctrl-G draws nothing until you set it back.")
        }
        Some((probe, None)) => format!(" Your terminal: {probe}. Ctrl-G draws nothing here."),
        None => " Your terminal is asked what it can show when you enter @tui.".to_string(),
    };
    set + &then
}

/// What an `@history` line asks for (interface plan I12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryReq {
    /// A page of `limit` lines; `more` continues from this connection's
    /// cursor instead of starting again from the newest.
    Page { limit: u32, more: bool },
    /// The verb with a tail that could not be read.
    Usage,
}

/// Parse `@history`, `@history <n>`, `@history more` — with `+history`
/// accepted as an alias, since half the players will reach for the `+` verb
/// family first and this way there is no wrong guess. Case-insensitive, the
/// words in either order. `None` for any other line, which then forwards to
/// the world untouched (invariant T1).
fn parse_history(line: &str) -> Option<HistoryReq> {
    let line = line.trim();
    let rest = strip_verb(line, "@history").or_else(|| strip_verb(line, "+history"))?;
    let mut limit = HISTORY_DEFAULT;
    let mut more = false;
    for word in rest.split_whitespace() {
        if word.eq_ignore_ascii_case("more") {
            more = true;
        } else if let Ok(n) = word.parse::<u32>() {
            if n == 0 || n > HISTORY_MAX {
                return Some(HistoryReq::Usage);
            }
            limit = n;
        } else {
            return Some(HistoryReq::Usage);
        }
    }
    Some(HistoryReq::Page { limit, more })
}

/// Split an account-path SSH username into its account and optional
/// requested-character parts on the **first** `+` (M19 direct-character
/// login: `ssh account+char@game`). Account and character names forbid `+`
/// (see the name validators in `mush-config`), so the split is unambiguous;
/// a trailing/empty character part (`account+`) is treated as no request.
/// The returned account part is what all auth uses; the character part only
/// ever selects a character the authenticated account *owns*.
pub(crate) fn split_account_user(user: &str) -> (&str, Option<&str>) {
    match user.split_once('+') {
        Some((account, character)) if !character.is_empty() => (account, Some(character)),
        Some((account, _)) => (account, None),
        None => (user, None),
    }
}

/// `play` / `guest` / `new` — the auth-none front door (M1: all three
/// alias to the welcome screen).
pub(crate) fn is_front_door(user: &str) -> bool {
    user.eq_ignore_ascii_case("play")
        || user.eq_ignore_ascii_case("guest")
        || user.eq_ignore_ascii_case("new")
}

/// Methods a rejected client can still try next. Includes `PublicKey` so a
/// client that probes with `none` first (as OpenSSH does, to discover the
/// method list) learns public-key auth is available and offers its key —
/// without it, `ssh -i key <account>@host` never even attempts the key and
/// falls straight to password. Keyboard-interactive + password remain the
/// fallback for accounts with no registered key.
fn interactive_methods() -> MethodSet {
    let mut m = MethodSet::empty();
    m.push(MethodKind::PublicKey);
    m.push(MethodKind::KeyboardInteractive);
    m.push(MethodKind::Password);
    m
}

fn reject_toward_interactive() -> Auth {
    Auth::Reject {
        proceed_with_methods: Some(interactive_methods()),
        partial_success: false,
    }
}

/// The refusal for a blocked address: `USERAUTH_FAILURE` with an **empty**
/// method list, which is how the protocol says "there is nothing left for you
/// to try". An OpenSSH client stops there and prints `Permission denied`; it
/// does not prompt.
///
/// A plain `Auth::reject()` leaves the method list alone, so the client walks
/// on to `password` and the operator gets a password prompt at the auth-none
/// front door — a prompt no password can satisfy, saying nothing about why.
/// That is what `ssh -p 6789 play@…` did on 2026-09-20. The *reason* travels
/// separately, in the authentication banner
/// ([`ConnHandler::authentication_banner`]), because a `USERAUTH_FAILURE`
/// carries no text.
fn reject_blocked() -> Auth {
    Auth::Reject {
        proceed_with_methods: Some(MethodSet::empty()),
        partial_success: false,
    }
}

/// What a refused address is told, in one sentence. The tempban names a
/// deadline the player can act on; the siteban deliberately does not (it may
/// be permanent, and staff, not a countdown, lift it).
pub(crate) fn block_notice(reason: BlockReason) -> String {
    match reason {
        BlockReason::Siteban => "Connections from your address are not permitted.".to_owned(),
        BlockReason::TooManyFailures { remaining } => {
            // Rounded UP, and never "0 minutes": a player told to wait a
            // minute and refused again has been lied to.
            let minutes = remaining.as_secs().div_ceil(60).max(1);
            let unit = if minutes == 1 { "minute" } else { "minutes" };
            format!("Too many failed logins from your address. Try again in {minutes} {unit}.")
        }
    }
}

/// The wording for a refusal whose reason the hooks did not report (an
/// embedder that has not implemented [`SshHooks::block_reason`]).
pub(crate) const BLOCKED_FALLBACK: &str =
    "Too many failed logins from your address; try again later.";

impl ConnHandler {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        connection_state(&self.shared)
    }

    /// Write text to the channel, one CRLF per line.
    fn write_text(&self, session: &mut Session, text: &str) -> Result<(), russh::Error> {
        let Some(ch) = self.channel else {
            return Ok(());
        };
        let mut bytes = Vec::with_capacity(text.len() + 16);
        for line in text.lines() {
            bytes.extend_from_slice(line.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        if bytes.is_empty() {
            return Ok(());
        }
        session.data(ch, bytes)
    }

    /// Farewell + close channel + disconnect, and the cut [`DISCONNECT_GRACE`]
    /// later for a client that does not take the disconnect for an answer.
    fn close_polite(&mut self, session: &mut Session, msg: &str) -> Result<(), russh::Error> {
        if !msg.is_empty() {
            self.write_text(session, msg)?;
        }
        if let Some(ch) = self.channel {
            let _ = session.close(ch);
        }
        let _ = session.disconnect(Disconnect::ByApplication, "goodbye", "");
        self.kill.arm(DISCONNECT_GRACE);
        Ok(())
    }

    async fn verify_account(&mut self, user: &str, password: &str) -> Auth {
        // The username may be `<account>` or `<account>+<char>` (M19): auth
        // always uses the account part; the character part rides the route.
        let (account_user, requested_char) = split_account_user(user);
        match self.hooks.auth_account(account_user, password).await {
            AuthResult::Ok {
                account,
                characters,
            } => {
                self.auth_method = AuthMethod::Password;
                self.route = Route::Account {
                    account,
                    characters,
                    requested_char: requested_char.map(str::to_owned),
                };
                Auth::Accept
            }
            AuthResult::BadCreds => {
                self.hooks.auth_failed(self.addr);
                reject_toward_interactive()
            }
            AuthResult::Banned => Auth::reject(),
        }
    }

    fn queue_line(&self, line: String) -> Result<(), russh::Error> {
        let Some(commands) = &self.commands else {
            return Err(russh::Error::Disconnect);
        };
        // Never await capacity in a russh callback: that would also stop writes.
        commands
            .try_send(line)
            .map_err(|_| russh::Error::Disconnect)
    }

    // ---- line dispatch --------------------------------------------------

    async fn handle_line(
        &mut self,
        session: &mut Session,
        raw: String,
    ) -> Result<(), russh::Error> {
        let line = sanitize_line(&raw);
        let mode = self.lock().mode.clone();
        match mode {
            Mode::InGame => self.queue_line(line),
            // Reached only if a line is submitted through the editor while in
            // TUI mode (the normal TUI input path is `ConnHandler::data`);
            // forward it so the world still sees the command.
            Mode::Tui => self.queue_line(line),
            Mode::Welcome => self.welcome_line(session, &line).await,
            Mode::CharSelect { account, chars } => {
                self.charselect_line(session, &account, &chars, &line).await
            }
            Mode::QuitConfirm { prev } => self.quit_confirm_line(session, *prev, &line).await,
        }
    }

    // ---- TUI (milestone M15) --------------------------------------------

    /// Pulse the repaint signal so the output pump renders a fresh frame
    /// without waiting for a world event (input edits, resize).
    fn fire_repaint(&self) {
        let notify = Arc::clone(&self.lock().repaint);
        notify.notify_one();
    }

    /// Raw input while in [`Mode::Tui`]: decode it into the TUI app, then
    /// repaint / submit / detach. Painting is left to the pump (fired via the
    /// repaint signal) so there is one steady-state frame writer.
    async fn tui_data(&mut self, data: &[u8], _session: &mut Session) -> Result<(), russh::Error> {
        let result = {
            let mut sh = self.lock();
            match sh.tui.as_mut() {
                Some(tui) => tui.feed_input(data),
                None => return Ok(()),
            }
        };
        if result.detach {
            return self.queue_line("@notui".to_owned());
        }
        // One burst can carry typed requests AND submitted lines (17g cut
        // C3b's review): a form's Enter, then the screen's `Enter type` and
        // the line's own Enter in the same read — key repeat, a slow link.
        // Everything the burst produced is forwarded; returning after the
        // lines once dropped the typed request, whose owner then waited on a
        // reply that could never come. The typed requests and queries go
        // first: their keys came before the line's own Enter in that case.
        // Author edits use the typed owner path; hydration stays read-only.
        // Replies land on the pump and repaint the current view.
        for (id, request) in result.invokes {
            self.hooks.invoke(self.conn_id, id, request).await;
        }
        for (name, args) in result.queries {
            submit_query(
                self.hooks.as_ref(),
                &self.shared,
                self.conn_id,
                name,
                args,
                None,
            )
            .await;
        }
        for (id, epoch, choice) in result.responds {
            self.hooks.respond(self.conn_id, id, epoch, choice).await;
        }
        if !result.submit.is_empty() {
            // Show the cleared input line immediately; the world's echo
            // repaints again via the pump.
            self.fire_repaint();
            // A burst can carry more than one line (a multi-line paste); each
            // is forwarded in order so none is dropped.
            for line in result.submit {
                self.queue_line(line)?;
            }
            return Ok(());
        }
        if result.redraw {
            self.fire_repaint();
        }
        Ok(())
    }

    async fn welcome_line(
        &mut self,
        session: &mut Session,
        line: &str,
    ) -> Result<(), russh::Error> {
        // Credential-bearing lines must never be recallable from the
        // editor history (↑ would re-show the plaintext password) —
        // dropped before any early return can leave them behind.
        if matches!(
            parse_welcome(line),
            WelcomeCmd::Connect { .. } | WelcomeCmd::Create { .. }
        ) {
            self.lock().editor.forget_last_history();
        }
        if !line.trim().is_empty() && self.flood.check(Instant::now()) == FloodVerdict::Throttled {
            return self.write_text(session, "You're doing that too fast.");
        }
        match parse_welcome(line) {
            WelcomeCmd::Empty => Ok(()),
            WelcomeCmd::Help => self.write_text(session, WELCOME_HELP),
            WelcomeCmd::Malformed { usage } => self.write_text(session, usage),
            WelcomeCmd::Unknown(verb) => {
                self.write_text(session, &format!("Unknown command \"{verb}\"."))?;
                self.write_text(session, WELCOME_HELP)
            }
            WelcomeCmd::Who => {
                self.write_text(session, "WHO is available once you have connected.")
            }
            WelcomeCmd::Quit => self.close_polite(session, "Goodbye."),
            WelcomeCmd::Connect { account, password } => {
                let (account, password) = (account.to_owned(), password.to_owned());
                self.try_connect_account(session, &account, &password).await
            }
            WelcomeCmd::Create { account, password } => {
                let (account, password) = (account.to_owned(), password.to_owned());
                self.create_and_connect(session, &account, &password).await
            }
        }
    }

    /// The sentence a blocked address is refused with, on the welcome
    /// screen and in the authentication banner alike — one wording, one
    /// place. Falls back to [`BLOCKED_FALLBACK`] when the hooks report no
    /// reason.
    fn blocked_notice(&self) -> String {
        self.hooks
            .block_reason(self.addr)
            .map_or_else(|| BLOCKED_FALLBACK.to_owned(), block_notice)
    }

    /// A blocked address is refused before the backend is even asked —
    /// this is what makes an IpGuard tempban bite on an *already open*
    /// welcome screen instead of only on new TCP connections.
    async fn try_connect_account(
        &mut self,
        session: &mut Session,
        account: &str,
        password: &str,
    ) -> Result<(), russh::Error> {
        if self.hooks.is_ip_blocked(self.addr) {
            let notice = self.blocked_notice();
            return self.close_polite(session, &notice);
        }
        match self.hooks.auth_account(account, password).await {
            AuthResult::Banned => self.close_polite(session, "You are banned from this server."),
            AuthResult::BadCreds => {
                self.hooks.auth_failed(self.addr);
                self.write_text(session, BAD_CREDS_MSG)
            }
            AuthResult::Ok {
                account,
                characters,
            } => {
                // The welcome-screen `connect` never carries a `+char`
                // (account names forbid `+`), so no character is requested.
                self.enter_account(session, account, characters, None).await
            }
        }
    }

    async fn create_and_connect(
        &mut self,
        session: &mut Session,
        account: &str,
        password: &str,
    ) -> Result<(), russh::Error> {
        use crate::hooks::CreateErr;
        if self.hooks.is_ip_blocked(self.addr) {
            let notice = self.blocked_notice();
            return self.close_polite(session, &notice);
        }
        match self.hooks.create_account(account, password).await {
            Err(CreateErr::Taken) => {
                self.write_text(session, "That account name is already taken.")
            }
            Err(CreateErr::BadName(why)) => self.write_text(session, &why),
            Err(CreateErr::Weak) => {
                self.write_text(session, "That password is too weak; pick something longer.")
            }
            Ok(()) => {
                self.write_text(session, "Account created.")?;
                self.try_connect_account(session, account, password).await
            }
        }
    }

    /// Route an authenticated account. When the username carried a
    /// `+<char>` request (M19) that names a character this account owns,
    /// connect straight to it (the zero-step login). Otherwise the classic
    /// M2 behavior: one character auto-connects, any other count (including
    /// zero) lands on the character-select screen, where `newchar <Name>` is
    /// also available.
    ///
    /// Security (invariant S2): a `+<char>` can only ever select a character
    /// in `chars` — this account's own list. A non-owned or mistyped request
    /// falls to *this account's* character-select screen; it never targets
    /// or reveals another account's character.
    async fn enter_account(
        &mut self,
        session: &mut Session,
        account: String,
        chars: Vec<CharSummary>,
        requested_char: Option<String>,
    ) -> Result<(), russh::Error> {
        if let Some(want) = requested_char {
            if let Some(c) = chars.iter().find(|c| c.name.eq_ignore_ascii_case(&want)) {
                let c = c.clone();
                return self.do_connect(session, &account, &c).await;
            }
            // Requested a character this account does not own (typo, or a
            // cross-account name): fall to this account's own char-select,
            // never auto-enter and never reveal another account.
            self.lock().mode = Mode::CharSelect {
                account: account.clone(),
                chars: chars.clone(),
            };
            return self.print_char_select(session, &account, &chars);
        }
        if chars.len() == 1 {
            let c = chars[0].clone();
            return self.do_connect(session, &account, &c).await;
        }
        self.lock().mode = Mode::CharSelect {
            account: account.clone(),
            chars: chars.clone(),
        };
        self.print_char_select(session, &account, &chars)
    }

    fn print_char_select(
        &self,
        session: &mut Session,
        account: &str,
        chars: &[CharSummary],
    ) -> Result<(), russh::Error> {
        let mut text = if chars.is_empty() {
            format!("The account {account} has no characters yet.\n")
        } else {
            let mut text = format!("Characters on {account}:\n");
            for (i, c) in chars.iter().enumerate() {
                text.push_str(&format!("  {}) {}\n", i + 1, c.name));
            }
            text
        };
        text.push_str("Enter a number, newchar <Name>, or QUIT:");
        self.write_text(session, &text)
    }

    async fn charselect_line(
        &mut self,
        session: &mut Session,
        account: &str,
        chars: &[CharSummary],
        line: &str,
    ) -> Result<(), russh::Error> {
        let pick = line.trim();
        if pick.is_empty() {
            return self.print_char_select(session, account, chars);
        }
        if pick.eq_ignore_ascii_case("quit") {
            return self.close_polite(session, "Goodbye.");
        }
        if let Some(rest) = strip_verb(pick, "newchar") {
            let name = rest.trim();
            if name.is_empty() {
                self.write_text(session, "Usage: newchar <Name>")?;
                return self.print_char_select(session, account, chars);
            }
            return match self.hooks.create_character(account, name).await {
                Ok(c) => {
                    self.write_text(session, &format!("Character {} created.", c.name))?;
                    let account = account.to_owned();
                    self.do_connect(session, &account, &c).await
                }
                Err(why) => {
                    self.write_text(session, &why)?;
                    self.print_char_select(session, account, chars)
                }
            };
        }
        let choice = pick
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| chars.get(i))
            .or_else(|| chars.iter().find(|c| c.name.eq_ignore_ascii_case(pick)));
        match choice {
            Some(c) => {
                let c = c.clone();
                let account = account.to_owned();
                self.do_connect(session, &account, &c).await
            }
            None => {
                self.write_text(session, "No such character.")?;
                self.print_char_select(session, account, chars)
            }
        }
    }

    /// The seam to the game: hand the connection's output queue to the
    /// binary and start the output pump.
    async fn do_connect(
        &mut self,
        session: &mut Session,
        account: &str,
        character: &CharSummary,
    ) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let (tx, rx) = mpsc::channel::<Arc<OutputEvent>>(OUTPUT_BUFFER);
        let (stamped_tx, stamped_rx) = mpsc::channel(OUTPUT_BUFFER);
        let snapshot = self.lock().caps;
        let conn = NewConn {
            id: self.conn_id,
            addr: self.addr,
            caps: snapshot,
            tx,
            stamped_tx: Some(stamped_tx),
        };
        // Armed *before* the await: if russh drops this future mid-connect
        // (connection torn down), Drop still fires `disconnected` and the
        // binary cleans up whatever the attach managed to do. A refusal
        // clears it again ( `disconnected` for a never-attached conn is a
        // no-op binary-side either way).
        self.connected.store(true, Ordering::SeqCst);
        match self
            .hooks
            .connect_character(conn, account, character.key)
            .await
        {
            Ok(()) => {
                if let Some(watchdog) = self.preauth_watchdog.take() {
                    watchdog.abort();
                }
                tracing::info!(
                    conn = self.conn_id,
                    account,
                    character = character.name,
                    "character connected"
                );
                let repaint = {
                    let mut sh = self.lock();
                    sh.mode = Mode::InGame;
                    Arc::clone(&sh.repaint)
                };
                let (tx, mut commands) = mpsc::channel::<String>(64);
                self.commands = Some(tx);
                let mut worker = GameCommands {
                    hooks: Arc::clone(&self.hooks),
                    shared: Arc::clone(&self.shared),
                    conn_id: self.conn_id,
                    channel: Some(channel),
                    handle: session.handle(),
                };
                self.command_task = Some(tokio::spawn(async move {
                    while let Some(line) = commands.recv().await {
                        match worker.local_line(&line).await {
                            Ok(true) => {}
                            Ok(false) => worker.hooks.line(worker.conn_id, line.clone()).await,
                            Err(_) => {
                                let _ = worker.handle.close(channel).await;
                                break;
                            }
                        }
                        if line.trim().eq_ignore_ascii_case("QUIT") {
                            break;
                        }
                    }
                }));
                spawn_output_pump(
                    rx,
                    stamped_rx,
                    Arc::clone(&self.shared),
                    session.handle(),
                    channel,
                    Arc::clone(&self.hooks),
                    self.conn_id,
                    repaint,
                    Arc::clone(&self.connected),
                    self.cut(),
                );
                Ok(())
            }
            Err(why) => {
                self.connected.store(false, Ordering::SeqCst);
                self.write_text(session, &format!("Connection failed: {why}"))
            }
        }
    }

    fn exec_error(
        &self,
        session: &mut Session,
        channel: ChannelId,
        message: &str,
    ) -> Result<(), russh::Error> {
        let response = mush_proto::InvokeResult::error("refused", message);
        let bytes = exec_json(&response);
        session.data(channel, bytes)?;
        session.exit_status_request(channel, 2)?;
        session.eof(channel)?;
        session.close(channel)
    }

    async fn start_mind_exec(
        &mut self,
        session: &mut Session,
        channel: ChannelId,
        request: mush_proto::InvokeRequest,
    ) -> Result<(), russh::Error> {
        if self.auth_method != AuthMethod::PublicKey {
            return self.exec_error(
                session,
                channel,
                "Verified public-key authentication is required.",
            );
        }
        let Route::Account {
            account,
            characters,
            requested_char,
        } = self.route.clone()
        else {
            return self.exec_error(
                session,
                channel,
                "No owned selected character is available.",
            );
        };
        let Some(character) =
            crate::mind_exec::owned_character(&characters, requested_char.as_deref())
        else {
            return self.exec_error(
                session,
                channel,
                "No owned selected character is available.",
            );
        };
        let (tx, rx) = mpsc::channel::<Arc<OutputEvent>>(OUTPUT_BUFFER);
        let mut caps = self.lock().caps;
        caps.structured = true;
        let conn = NewConn {
            id: self.conn_id,
            addr: self.addr,
            caps,
            tx,
            stamped_tx: None,
        };
        self.connected.store(true, Ordering::SeqCst);
        if self
            .hooks
            .connect_character(conn, &account, character.key)
            .await
            .is_err()
        {
            self.connected.store(false, Ordering::SeqCst);
            return self.exec_error(session, channel, "Character attachment was refused.");
        }
        if let Some(watchdog) = self.preauth_watchdog.take() {
            watchdog.abort();
        }
        spawn_exec_pump(
            rx,
            session.handle(),
            channel,
            Arc::clone(&self.hooks),
            self.conn_id,
            Arc::clone(&self.connected),
            self.cut(),
        );
        self.hooks.invoke(self.conn_id, 1, request).await;
        Ok(())
    }

    /// What a pump needs to cut this connection once it has closed the
    /// session's channel.
    fn cut(&self) -> Cut {
        Cut {
            kill: Arc::clone(&self.kill),
            close_grace: self.auth_deadline,
        }
    }

    // ---- quit confirmation ----------------------------------------------

    async fn begin_quit_confirm(&mut self, session: &mut Session) -> Result<(), russh::Error> {
        let current = self.lock().mode.clone();
        if let Mode::QuitConfirm { prev } = current {
            // Second ^D confirms.
            return self.confirm_quit(session, *prev).await;
        }
        {
            let mut sh = self.lock();
            let prev = std::mem::replace(&mut sh.mode, Mode::Welcome);
            sh.mode = Mode::QuitConfirm {
                prev: Box::new(prev),
            };
        }
        self.write_text(session, QUIT_CONFIRM_MSG)
    }

    async fn quit_confirm_line(
        &mut self,
        session: &mut Session,
        prev: Mode,
        line: &str,
    ) -> Result<(), russh::Error> {
        let answer = line.trim().to_ascii_lowercase();
        if answer == "y" || answer == "yes" {
            self.confirm_quit(session, prev).await
        } else {
            self.lock().mode = prev;
            self.write_text(session, "Cancelled.")
        }
    }

    async fn confirm_quit(
        &mut self,
        session: &mut Session,
        prev: Mode,
    ) -> Result<(), russh::Error> {
        if matches!(prev, Mode::InGame) {
            // In game, ^D is QUIT: route it through the world so the
            // binary detaches cleanly and announces it.
            self.lock().mode = prev;
            self.queue_line("QUIT".to_owned())?;
            Ok(())
        } else {
            self.close_polite(session, "Goodbye.")
        }
    }
}

impl Handler for ConnHandler {
    type Error = russh::Error;

    /// Sent once, at the start of authentication, before any method is
    /// tried — and the only channel an SSH server has for telling a client
    /// it is about to refuse *why*. OpenSSH prints it. A blocked address
    /// gets [`block_notice`]; everyone else gets nothing, so the front door
    /// is unchanged.
    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        // Banners are displayed verbatim: one line, CRLF-terminated the way
        // every other line this transport writes is.
        if self.slot.refused() {
            // Over the address's cap: the one thing this connection gets is
            // the reason (doc 29a §9), before its first method is refused.
            return Ok(Some(format!("{OVER_CAP_NOTICE}\r\n")));
        }
        let Some(reason) = self.hooks.block_reason(self.addr) else {
            return Ok(None);
        };
        Ok(Some(format!("{}\r\n", block_notice(reason))))
    }

    /// Any method accepted: the accept-time watchdog's first deadline is
    /// met, and its second (a channel within another `auth_deadline`) starts.
    async fn auth_succeeded(&mut self, _session: &mut Session) -> Result<(), Self::Error> {
        self.authed.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        if self.slot.refused() {
            // Over the per-IP connection cap: shed the connection outright.
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(reject_blocked());
        }
        if let Some(attested) = self.attested.as_ref().filter(|a| a.user == user) {
            // Doc 29: the doorway proved possession of a key the account has
            // registered (the preamble's `Begin`); this is that login.
            let route = Route::Account {
                account: attested.account.clone(),
                characters: attested.characters.clone(),
                requested_char: split_account_user(user).1.map(str::to_owned),
            };
            self.auth_method = AuthMethod::PublicKey;
            self.route = route;
            return Ok(Auth::Accept);
        }
        if is_front_door(user) {
            self.route = Route::Welcome;
            Ok(Auth::Accept)
        } else {
            Ok(reject_toward_interactive())
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if self.slot.refused() {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(reject_blocked());
        }
        if is_front_door(split_account_user(user).0) {
            self.route = Route::Welcome;
            return Ok(Auth::Accept);
        }
        Ok(self.verify_account(user, password).await)
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if self.slot.refused() {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(reject_blocked());
        }
        if is_front_door(split_account_user(user).0) {
            self.route = Route::Welcome;
            return Ok(Auth::Accept);
        }
        match response {
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("rustmush login"),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            Some(mut r) => {
                let password = r
                    .next()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                Ok(self.verify_account(user, &password).await)
            }
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        // The QUERY phase (M19): russh has NOT verified a signature yet, so
        // this callback must never grant access or set the route — it only
        // decides whether to ask the client to sign this key (`Auth::Accept`)
        // or to fall toward password (`reject_toward_interactive`). Access is
        // granted only in `auth_publickey`, after the signature is verified.
        if self.slot.refused() {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(reject_blocked());
        }
        let (account_user, _) = split_account_user(user);
        if is_front_door(account_user) {
            // The front door authenticates with no key; steer to the welcome
            // path's password/none flow.
            return Ok(reject_toward_interactive());
        }
        let Ok(openssh) = key.to_openssh() else {
            return Ok(reject_toward_interactive());
        };
        match self.hooks.auth_account_key(account_user, &openssh).await {
            AuthResult::Ok { .. } => Ok(Auth::Accept),
            // No matching key / unknown account / banned: fall to password —
            // never a hard reject that would kill the fallback. No failure is
            // counted here (the client has not attempted a signed auth yet).
            _ => Ok(reject_toward_interactive()),
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        // The AUTHORITATIVE callback (M19): russh has ALREADY verified the
        // signature, so this is the only place public-key access is granted.
        // Same pre-checks as the password path.
        if self.slot.refused() {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(reject_blocked());
        }
        let (account_user, requested_char) = split_account_user(user);
        if is_front_door(account_user) {
            return Ok(reject_toward_interactive());
        }
        let Ok(openssh) = key.to_openssh() else {
            // A malformed offer is not a ban-worthy failure (see below).
            return Ok(reject_toward_interactive());
        };
        match self.hooks.auth_account_key(account_user, &openssh).await {
            AuthResult::Ok {
                account,
                characters,
            } => {
                self.auth_method = AuthMethod::PublicKey;
                self.route = Route::Account {
                    account,
                    characters,
                    requested_char: requested_char.map(str::to_owned),
                };
                Ok(Auth::Accept)
            }
            // Deliberately NOT counted as an auth failure. russh reached this
            // arm only after cryptographically verifying the client OWNS the
            // offered key, so a miss means "a key I hold is not registered
            // here" — normal client behavior (ssh-agents offer every loaded
            // key), not an attack. Counting it would let a user with a few
            // agent keys trip the 5-failures tempban on a single connect.
            // Password failures remain the ban trigger; per-connection key
            // spray is already bounded by the SSH MaxAuthTries limit.
            AuthResult::BadCreds => Ok(reject_toward_interactive()),
            AuthResult::Banned => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel.is_some() {
            // One session channel per connection — and (doc 21 cut G.11)
            // one more, for the asset pack's `sftp`, once the connection is
            // in game and the pack is served. Dropping the handle rejects
            // the request.
            if self.connected.load(Ordering::SeqCst)
                && self.sftp.is_none()
                && !self.sftp_started
                && crate::assets::AssetPack::shipped().is_some()
            {
                self.sftp = Some(channel);
                reply.accept().await;
            } else {
                drop(reply);
            }
            return Ok(());
        }
        self.channel = Some(channel.id());
        self.opened.store(true, Ordering::SeqCst);
        reply.accept().await;
        // Pre-auth watchdog (design 02 §8.2): a connection that has not
        // reached a character within the timeout is dropped, so idle
        // welcome-screen squatters cannot hold slots forever. In-game
        // idlers are untouched (`connected` is set by then). The cut after
        // the disconnect is for a client that ignores it (doc 29a §9).
        let handle = session.handle();
        let connected = Arc::clone(&self.connected);
        let kill = Arc::clone(&self.kill);
        let channel_id = channel.id();
        let conn_id = self.conn_id;
        self.preauth_watchdog = Some(tokio::spawn(async move {
            tokio::time::sleep(PREAUTH_TIMEOUT).await;
            if !connected.load(Ordering::SeqCst) {
                tracing::info!(conn = conn_id, "pre-auth timeout; dropping connection");
                let _ = handle
                    .data(channel_id, b"\r\nLogin timeout.\r\n".to_vec())
                    .await;
                let _ = handle.close(channel_id).await;
                let _ = handle
                    .disconnect(Disconnect::ByApplication, "login timeout".into(), "".into())
                    .await;
                tokio::time::sleep(DISCONNECT_GRACE).await;
                kill.kill();
            }
        }));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            self.pty_seen = true;
            let mut sh = self.lock();
            sh.base_echo = pty_echo_mode(modes);
            let base = sh.base_echo;
            sh.editor.set_mode(base);
            sh.term = Some(term.to_owned());
            sh.width = u16::try_from(col_width).unwrap_or(u16::MAX);
            sh.height = u16::try_from(row_height).unwrap_or(u16::MAX);
            if let Some(cell) = cell_size_from_pty_px(col_width, row_height, pix_width, pix_height)
            {
                sh.cell_size = Some(cell);
            }
            sh.recompute_caps();
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            let mut sh = self.lock();
            match variable_name.to_ascii_uppercase().as_str() {
                "COLORTERM" => sh.colorterm = Some(variable_value.to_owned()),
                "LANG" => sh.lang = Some(variable_value.to_owned()),
                "LC_ALL" => sh.lc_all = Some(variable_value.to_owned()),
                _ => {} // read-only interest; everything else ignored
            }
            sh.recompute_caps();
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            let (caps, is_tui) = {
                let mut sh = self.lock();
                sh.width = u16::try_from(col_width).unwrap_or(u16::MAX);
                sh.height = u16::try_from(row_height).unwrap_or(u16::MAX);
                if let Some(cell) =
                    cell_size_from_pty_px(col_width, row_height, pix_width, pix_height)
                {
                    sh.cell_size = Some(cell);
                }
                let cell_size = sh.cell_size;
                sh.recompute_caps();
                let caps = sh.caps;
                // Reflow the TUI to the new geometry; the next repaint clears
                // + redraws at the new size (invariant T4).
                if let Some(tui) = sh.tui.as_mut() {
                    tui.set_caps(caps);
                    if let Some(cell) = cell_size {
                        tui.set_cell_size(cell);
                    }
                }
                (caps, matches!(sh.mode, Mode::Tui))
            };
            if self.connected.load(Ordering::SeqCst) {
                self.hooks.caps_changed(self.conn_id, caps);
            }
            // Line mode pushes no bytes on resize (unchanged); TUI mode
            // repaints so the frame reflows without a world event.
            if is_tui {
                self.fire_repaint();
            }
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel != Some(channel) {
            session.channel_failure(channel)?;
            return Ok(());
        }
        session.channel_success(channel)?;
        if self.shell_started {
            return Ok(());
        }
        self.shell_started = true;
        if !self.pty_seen {
            self.write_text(
                session,
                "rustmush needs an interactive terminal; reconnect without forcing no-PTY.",
            )?;
            return self.close_polite(session, "");
        }
        match self.route.clone() {
            Route::Welcome | Route::PreAuth => {
                let banner = self.banner.to_string();
                self.write_text(session, &banner)?;
                self.write_text(session, WELCOME_HELP)
            }
            Route::Account {
                account,
                characters,
                requested_char,
            } => {
                self.enter_account(session, account, characters, requested_char)
                    .await
            }
        }
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = crate::mind_exec::MindExec::parse(data);
        // Any exec attempt consumes this channel's one operation, even refusal.
        // This blocks trailing data from falling back to the line editor.
        self.exec_submitted = true;
        if self.channel != Some(channel)
            || self.shell_started
            || self.pty_seen
            || self.auth_method != AuthMethod::PublicKey
            || command.is_none()
        {
            session.channel_failure(channel)?;
            return self.exec_error(
                session,
                channel,
                "Only verified public-key mind push, mind pull or mind check is available.",
            );
        }
        let Route::Account {
            characters,
            requested_char,
            ..
        } = &self.route
        else {
            session.channel_failure(channel)?;
            return self.exec_error(
                session,
                channel,
                "No owned selected character is available.",
            );
        };
        if crate::mind_exec::owned_character(characters, requested_char.as_deref()).is_none() {
            session.channel_failure(channel)?;
            return self.exec_error(
                session,
                channel,
                "Choose an owned character with account+character.",
            );
        }
        self.shell_started = true;
        if let Some(command) = command {
            self.exec_input = Some(crate::mind_exec::ExecInput::new(command));
        }
        session.channel_success(channel)?;
        Ok(())
    }

    /// **`sftp` on the asset pack's channel** (doc 21 cut G.11): the pack's
    /// read-only tree, served from memory ([`crate::assets::AssetsSftp`]).
    /// Any other subsystem, or `sftp` on the game's own channel, is refused.
    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let ours = self.sftp.as_ref().is_some_and(|c| c.id() == channel);
        let pack = crate::assets::AssetPack::shipped();
        match (name, ours, pack) {
            ("sftp", true, Some(pack)) if self.connected.load(Ordering::SeqCst) => {
                if let Some(chan) = self.sftp.take() {
                    self.sftp_started = true;
                    session.channel_success(channel)?;
                    tracing::info!(conn = self.conn_id, "asset pack sftp session started");
                    russh_sftp::server::run(
                        crate::assets::EndOnError(chan.into_stream()),
                        crate::assets::AssetsSftp::new(pack),
                    )
                    .await;
                    return Ok(());
                }
                session.channel_failure(channel)?;
            }
            _ => session.channel_failure(channel)?,
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel != Some(channel) {
            return Ok(());
        }
        if let Some(input) = &mut self.exec_input {
            if let Err(message) = input.feed(data) {
                self.exec_input = None;
                self.exec_submitted = true;
                return self.exec_error(session, channel, message);
            }
            return Ok(());
        }
        if self.exec_submitted {
            return self.exec_error(
                session,
                channel,
                "Only one request is permitted per exec channel.",
            );
        }
        // In TUI mode raw bytes drive the ratatui app (input line + nav
        // hotkeys), not the line editor (design M15 D4).
        if matches!(self.lock().mode, Mode::Tui) {
            return self.tui_data(data, session).await;
        }
        if !self.connected.load(Ordering::SeqCst) {
            if session.has_pending_data(channel) {
                self.pending_strikes = self.pending_strikes.saturating_add(1);
                if self.pending_strikes >= PENDING_STRIKES {
                    return Err(russh::Error::Disconnect);
                }
                // Not echoed into a window the client is not draining.
                return Ok(());
            }
            self.pending_strikes = 0;
        }
        let (echo, actions) = {
            let mut sh = self.lock();
            let mut out = Vec::new();
            let actions = sh.editor.feed(data, &mut out);
            (out, actions)
        };
        if !echo.is_empty() {
            session.data(channel, echo)?;
        }
        for action in actions {
            match action {
                EditorAction::Submit(line) => self.handle_line(session, line).await?,
                EditorAction::EofQuit => self.begin_quit_confirm(session).await?,
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel != Some(channel) {
            return Ok(());
        }
        if let Some(input) = self.exec_input.take() {
            self.exec_submitted = true;
            let request = match input.finish() {
                Ok(request) => request,
                Err(message) => return self.exec_error(session, channel, message),
            };
            return self.start_mind_exec(session, channel, request).await;
        }
        if self.exec_submitted {
            return Ok(());
        }
        Err(russh::Error::Disconnect)
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            return Err(russh::Error::Disconnect);
        }
        Ok(())
    }
}

// Connected typed commands may wait on persistence, restart, or output fences.
// Their worker owns a Handle, never a borrowed Session, so the SSH event loop
// can flush progress and process window adjustments throughout that wait.
struct GameCommands {
    hooks: Arc<dyn SshHooks>,
    shared: Arc<Mutex<Shared>>,
    conn_id: u64,
    channel: Option<ChannelId>,
    handle: russh::server::Handle,
}

fn connection_state(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().expect("conn state poisoned")
}

/// Derive one terminal cell's pixel size when the SSH `pty-req` or
/// `window-change` message carried non-zero pixel dimensions alongside columns
/// and rows.
fn cell_size_from_pty_px(
    col_width: u32,
    row_height: u32,
    pix_width: u32,
    pix_height: u32,
) -> Option<CellSize> {
    if col_width == 0 || row_height == 0 || pix_width == 0 || pix_height == 0 {
        return None;
    }
    let cw = u16::try_from(pix_width / col_width).ok()?;
    let ch = u16::try_from(pix_height / row_height).ok()?;
    CellSize::from_px(cw, ch)
}

impl GameCommands {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        connection_state(&self.shared)
    }
    /// `@tui`: build the [`TuiSession`], enter the alternate screen, paint the
    /// first frame, and kick a WHO refresh. From here the output pump owns all
    /// steady-state repaints (a single writer), so no frame races the enter.
    async fn enter_tui(&mut self) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let (bytes, queries) = {
            let mut sh = self.lock();
            let caps = sh.caps;
            let mut tui = Box::new(TuiSession::new(caps, sh.manifest.clone()));
            if let Some(cell) = sh.cell_size {
                tui.set_cell_size(cell);
            }
            tui.set_pixel_choice(sh.pixel_choice);
            tui.set_sounds(sh.sounds_on);
            tui.set_pixel_config(self.hooks.pixel_config());
            let mut out = TuiSession::enter_bytes().to_vec();
            out.extend_from_slice(&tui.paint());
            // Pixel cockpit spike (2026-10-21): ask the terminal whether it
            // speaks the kitty graphics protocol; the answer comes back
            // through the input path as marker keys.
            out.extend_from_slice(&tui.probe_bytes());
            let queries = tui.entry_queries();
            sh.tui = Some(tui);
            sh.history_request = None;
            sh.mode_generation = sh.mode_generation.wrapping_add(1);
            sh.mode = Mode::Tui;
            (out, queries)
        };
        self.write_ordered(channel, bytes).await?;
        // Read-only hydration (WHO, the command table, channels, vitals, the
        // screen's own feeds); every reply lands on the pump, which repaints.
        // None of it stamps idle (design M15 D5, interface plan I1 D-I-3).
        for (name, args) in queries {
            submit_query(
                self.hooks.as_ref(),
                &self.shared,
                self.conn_id,
                name,
                args,
                None,
            )
            .await;
        }
        Ok(())
    }

    /// `@notui` / detach: leave the alternate screen, restore the cursor, drop
    /// the TUI state, and hand the resumed line client its prompt back
    /// (invariant T5 — never leave a client stuck on the alternate screen).
    async fn leave_tui(&mut self) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let stop_orders: Vec<String>;
        let bytes = {
            let mut sh = self.lock();
            // Pixel cockpit spike (2026-10-21): free the picture's image
            // before the screen it sits on goes away.
            let mut out = sh
                .tui
                .as_ref()
                .map(|t| t.pixel_close_bytes())
                .unwrap_or_default();
            // Doc 21 cut G.4: a turn in progress stops where it was shown.
            stop_orders = sh
                .tui
                .as_mut()
                .map(|t| t.pixel_leave_orders())
                .unwrap_or_default();
            sh.tui = None;
            sh.history_request = None;
            sh.mode_generation = sh.mode_generation.wrapping_add(1);
            sh.history.pending = None;
            sh.mode = Mode::InGame;
            out.extend_from_slice(TuiSession::leave_bytes());
            // Repaint the line editor's prompt + in-progress buffer so the
            // resumed line client is not left on a blank line.
            sh.editor.redraw(&mut out);
            out
        };
        self.write_ordered(channel, bytes).await?;
        for line in stop_orders {
            self.hooks.control_line(self.conn_id, line).await;
        }
        Ok(())
    }

    // ---- the picture's encoder (doc 21 cut G.2) ---------------------------

    /// `@pixel/encoder`: set or show this connection's choice, applied to an
    /// open TUI at once (a change closes the picture), and the answer's text.
    fn pixel_encoder(&self, req: PixelEncoderReq) -> String {
        let mut sh = self.lock();
        if let PixelEncoderReq::Set(choice) = req {
            sh.pixel_choice = choice;
            if let Some(tui) = sh.tui.as_mut() {
                tui.set_pixel_choice(choice);
            }
        }
        let choice = sh.pixel_choice;
        let report = sh.tui.as_mut().map(|t| t.pixel_report());
        pixel_encoder_reply(req, choice, report)
    }

    // ---- the sounds (doc 21 cut G.10) ---------------------------------------

    /// `@sounds [on|off]`: set or show this connection's sounds, applied to
    /// an open TUI at once, and the answer's text.
    fn sounds(&self, req: Result<Option<bool>, ()>) -> String {
        let Ok(req) = req else {
            return SOUNDS_USAGE.to_string();
        };
        let mut sh = self.lock();
        if let Some(on) = req {
            sh.sounds_on = on;
            if let Some(tui) = sh.tui.as_mut() {
                tui.set_sounds(on);
            }
        }
        let report = sh.tui.as_ref().map(|t| t.sounds_report());
        sounds_reply(sh.sounds_on, report)
    }

    // ---- line-mode scene history (interface plan I12) --------------------

    /// Write one transport-side system line the way a world `Sys` line
    /// looks (this connection's caps and the shared theme), around the line
    /// editor exactly as the pump does.
    async fn write_sys(&self, text: &str) -> Result<(), russh::Error> {
        let Some(ch) = self.channel else {
            return Ok(());
        };
        let event = OutputEvent::Sys(RichText::tag("system", text));
        let bytes = {
            let mut sh = self.lock();
            if let Some(tui) = sh.tui.as_mut() {
                let _ = tui.apply_event(&event);
                tui.paint()
            } else {
                let mut bytes = Vec::new();
                render_line_event(&mut sh, &event, &AnsiTheme::default(), &mut bytes);
                bytes
            }
        };
        self.write_ordered(ch, bytes).await
    }

    async fn write_ordered(&self, channel: ChannelId, bytes: Vec<u8>) -> Result<(), russh::Error> {
        tokio::time::timeout(WRITE_TIMEOUT, self.handle.data(channel, bytes))
            .await
            .map_err(|_| russh::Error::Disconnect)?
            .map_err(|_| russh::Error::Disconnect)
    }

    async fn flush_output(&self) -> Result<(), russh::Error> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut sh = self.lock();
            sh.output_fence = Some(tx);
            sh.repaint.notify_one();
        }
        tokio::time::timeout(WRITE_TIMEOUT, rx)
            .await
            .map_err(|_| russh::Error::Disconnect)?
            .map_err(|_| russh::Error::Disconnect)
    }

    async fn write_marker(&self, tag: &str, text: &str) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let mut bytes = Vec::new();
        mush_ansi::render(
            &OutputEvent::Sys(RichText::tag(tag, text)),
            &self.lock().caps,
            &AnsiTheme::default(),
            &mut bytes,
        );
        self.write_ordered(channel, bytes).await
    }

    /// All transport-local typed commands share one reply boundary and one writer.
    async fn local_line(&mut self, line: &str) -> Result<bool, russh::Error> {
        let in_tui = matches!(self.lock().mode, Mode::Tui);
        let enter = !in_tui && line.trim().eq_ignore_ascii_case("@tui");
        let leave = in_tui && line.trim().eq_ignore_ascii_case("@notui");
        let history = if in_tui { None } else { parse_history(line) };
        let encoder = parse_pixel_encoder(line);
        let sounds = parse_sounds(line);
        let stats = is_pixel_stats(line);
        if !enter && !leave && history.is_none() && encoder.is_none() && sounds.is_none() && !stats
        {
            return Ok(false);
        }
        let reply = self.hooks.begin_local_reply(self.conn_id).await;
        let framed = reply.brackets.prefix.is_some() || reply.brackets.suffix.is_some();
        // A preceding world command may have finished by queueing its body;
        // drain that body before writing a transport-local answer, framed or not.
        self.flush_output().await?;
        if let Some(prefix) = &reply.brackets.prefix {
            self.write_marker(mush_proto::OUTPUT_PREFIX_TAG, prefix)
                .await?;
        }
        if enter {
            self.enter_tui().await?;
        } else if leave {
            self.leave_tui().await?;
        } else if let Some(request) = history {
            self.history_line(request).await?;
            if framed {
                let notify = self.lock().history_rendered.clone();
                let completed = tokio::time::timeout(HISTORY_LATCH, async {
                    loop {
                        let notified = notify.notified();
                        if self.lock().history_request.is_none() {
                            break;
                        }
                        notified.await;
                    }
                })
                .await
                .is_ok();
                if !completed {
                    {
                        let mut sh = self.lock();
                        sh.history_request = None;
                        sh.history.pending = None;
                    }
                    self.write_sys("History is unavailable; please try again.")
                        .await?;
                }
            }
        } else if let Some(request) = encoder {
            let text = self.pixel_encoder(request);
            self.write_sys(&text).await?;
        } else if let Some(request) = sounds {
            let text = self.sounds(request);
            self.write_sys(&text).await?;
        } else if stats {
            let lines = self
                .lock()
                .tui
                .as_mut()
                .map(|t| t.pixel_stats_lines())
                .unwrap_or_else(|| vec![PIXEL_STATS_OUTSIDE_TUI.to_owned()]);
            for text in lines {
                self.write_sys(&text).await?;
            }
        }
        if framed {
            self.flush_output().await?;
        }
        if let Some(suffix) = &reply.brackets.suffix {
            self.write_marker(mush_proto::OUTPUT_SUFFIX_TAG, suffix)
                .await?;
        }
        Ok(true)
    }

    /// `@history [n|more]`: ask the gateway for a page of this viewer's own
    /// resume ring. The page itself arrives asynchronously as a
    /// `Reply{Panel{narrative, Delta}}` and is rendered by the output pump
    /// (see `render_history_page`); only the refusals are written here.
    async fn history_line(&mut self, req: HistoryReq) -> Result<(), russh::Error> {
        let HistoryReq::Page { limit, more } = req else {
            return self.write_sys(HISTORY_USAGE).await;
        };
        /// Either ask, or say why not — decided under the lock, acted on
        /// after it drops (the hooks call is `async`).
        enum Next {
            Ask(Option<u64>),
            Say(&'static str),
        }
        let next = {
            let mut sh = self.lock();
            if sh
                .history
                .pending
                .is_some_and(|at| at.elapsed() < HISTORY_LATCH)
            {
                Next::Say("Your last history request is still on its way.")
            } else if more && !sh.history.more {
                Next::Say("There is no older history.")
            } else {
                if !more {
                    // A bare `@history` starts again from the newest.
                    sh.history.before = None;
                    sh.history.more = true;
                }
                sh.history.pending = Some(Instant::now());
                Next::Ask(sh.history.before)
            }
        };
        match next {
            Next::Say(msg) => self.write_sys(msg).await,
            Next::Ask(before) => {
                // An explicit, non-zero limit: the gateway does
                // `.unwrap_or(0).clamp(1, 200)`, so an absent or zero limit
                // would silently yield a ONE-line page with `more: true`.
                submit_query(
                    self.hooks.as_ref(),
                    &self.shared,
                    self.conn_id,
                    "narrative.backlog".to_owned(),
                    serde_json::json!({
                        "before_seq": before,
                        "limit": limit.clamp(1, HISTORY_MAX),
                    }),
                    None,
                )
                .await;
                Ok(())
            }
        }
    }
}

// ---- output pump --------------------------------------------------------

/// Query correlation belongs to the physical connection and survives mode flips.
async fn submit_query(
    hooks: &dyn SshHooks,
    shared: &Mutex<Shared>,
    conn: u64,
    name: String,
    args: serde_json::Value,
    generation: Option<u64>,
) {
    let id = if name == "narrative.backlog" {
        let Ok(mut sh) = shared.lock() else {
            return;
        };
        if generation.is_some_and(|generation| generation != sh.mode_generation) {
            return;
        }
        let id = sh.next_history_request;
        let Some(next) = id.checked_add(1) else {
            return;
        };
        sh.next_history_request = next;
        sh.history_request = Some(id);
        id
    } else {
        0
    };
    hooks.query(conn, id, name, args).await;
}

fn reply_wire_boundary(event: &OutputEvent) -> bool {
    match event {
        OutputEvent::Sys(text) => text.0.iter().any(|span| {
            matches!(
                span.tag.as_deref(),
                Some(mush_proto::OUTPUT_PREFIX_TAG | mush_proto::OUTPUT_SUFFIX_TAG)
            )
        }),
        OutputEvent::Disconnect { .. } => true,
        _ => false,
    }
}

fn accept_history_reply(sh: &mut Shared, event: &OutputEvent) -> bool {
    let OutputEvent::Reply { id, event } = event else {
        return true;
    };
    let page = matches!(&**event, OutputEvent::Panel { feed, kind: PanelFrameKind::Delta, .. } if feed == "narrative");
    if *id < (1 << 63) && !page {
        return true;
    }
    if sh.history_request != Some(*id) {
        return false;
    }
    sh.history_request = None;
    true
}

fn pty_echo_mode(modes: &[(russh::Pty, u32)]) -> EchoMode {
    if modes
        .iter()
        .rev()
        .find(|(op, _)| *op == russh::Pty::ECHO)
        .is_some_and(|(_, value)| *value == 0)
    {
        EchoMode::NoEcho
    } else {
        EchoMode::Normal
    }
}

/// Render one event while preserving the line editor and its echo preference.
fn render_line_event(sh: &mut Shared, ev: &OutputEvent, theme: &AnsiTheme, bytes: &mut Vec<u8>) {
    // Remember what a TUI opened later needs to know (interface plan I2).
    match ev {
        OutputEvent::Hello { panels, .. } => sh.manifest.panels.clone_from(panels),
        OutputEvent::SessionInfo { character, .. } => {
            sh.manifest.character = Some(character.name.clone());
        }
        _ => {}
    }
    match ev {
        OutputEvent::Prompt(spec) => {
            sh.editor.set_mode(match spec.mode {
                InputMode::Normal => sh.base_echo,
                InputMode::Masked => EchoMode::Masked,
                InputMode::NoEcho => EchoMode::NoEcho,
            });
            if spec.text.is_some() {
                // Render the prompt once and store it; redraw re-prints it
                // from here on.
                let mut prompt = Vec::new();
                mush_ansi::render(ev, &sh.caps, theme, &mut prompt);
                sh.editor
                    .set_prompt(String::from_utf8_lossy(&prompt).into_owned());
            } else {
                // `text: None` means "no prompt" (the normal in-game mode
                // restored after an input context pops) — clear the stored
                // prompt rather than keeping a stale "Mail>" on screen.
                sh.editor.set_prompt(String::new());
            }
            sh.editor.redraw(bytes);
        }
        // Interface plan I12: a `narrative` Delta reply is a page of this
        // viewer's own resume ring, not a panel frame. `mush-ansi` drops
        // every `Panel` (one arm per `OutputEvent` variant is its contract),
        // so the transport renders the page itself — each line through the
        // SAME `mush_ansi::render` that drew it live.
        OutputEvent::Reply { event, .. }
            if matches!(&**event, OutputEvent::Panel { feed, kind, .. }
                if feed.as_str() == "narrative" && *kind == PanelFrameKind::Delta) =>
        {
            let OutputEvent::Panel { data, .. } = &**event else {
                return;
            };
            // Only a page this connection asked for is printed: an
            // unsolicited narrative frame renders NOTHING.
            if sh.history.pending.take().is_none() {
                return;
            }
            let interrupted = sh.editor.has_visible_line();
            if interrupted {
                bytes.extend_from_slice(b"\r\x1b[K");
            }
            render_history_page(sh, data, theme, bytes);
            if interrupted {
                sh.editor.redraw(bytes);
            }
        }
        _ => {
            // 16b cut B.4 (`B-67`): render into a scratch buffer FIRST. An
            // event that renders to nothing on a line client (a cockpit
            // keyframe with no grid shape, the cleared frame, any other
            // panel) must leave the pilot's half-typed line alone — before
            // this, every such frame erased and redrew the input line, twice
            // a second at `Engaged`.
            let mut rendered = Vec::new();
            mush_ansi::render(ev, &sh.caps, theme, &mut rendered);
            if rendered.is_empty() {
                return;
            }
            let interrupted = sh.editor.has_visible_line();
            if interrupted {
                bytes.extend_from_slice(b"\r\x1b[K");
            }
            bytes.extend_from_slice(&rendered);
            if interrupted {
                sh.editor.redraw(bytes);
            }
        }
    }
}

/// **Remember the room a later `@tui` opens on** (16b cut B.3, `B-66`): a bare
/// `Room` event, or a `Reply` whose inner event is one (16b cut B.4 makes the
/// attach look a `Reply { Room }`), replaces [`TuiManifest::room`].
///
/// The output pump calls this for EVERY event BEFORE its line/TUI branch. In
/// TUI mode the pump hands events to `TuiApp::apply_event` and
/// [`render_line_event`] never sees them, and `leave_tui` copies nothing back,
/// so a room recorded only on the line side went stale the moment a player
/// walked (or `BOARD`ed) inside the TUI — and a second `@tui` opened on the
/// room they had left.
fn remember_the_room(manifest: &mut TuiManifest, ev: &OutputEvent) {
    let inner = match ev {
        OutputEvent::Reply { event, .. } => event.as_ref(),
        other => other,
    };
    if let OutputEvent::Room(view) = inner {
        manifest.room = Some(view.clone());
    }
}

/// **The coalescing drain of the TUI side of the pump**: every event `ready`
/// still has — the burst queued behind the one that woke the pump — recorded by
/// [`remember_the_room`] and then applied to the TUI, the queries each asks for
/// collected into `queries`. Answers whether a `Disconnect` was among them.
///
/// A walk inside the TUI arrives as a burst (the `Sys` line, then the `Room`),
/// so the room is usually NOT the event that woke the pump (that one is
/// recorded before the pump's line/TUI branch) but one drained here (16b cut
/// B.3); the unit test drives this with such a burst.
#[cfg(test)]
fn drain_into_tui(
    manifest: &mut TuiManifest,
    mut tui: Option<&mut TuiSession>,
    mut ready: impl FnMut() -> Option<Arc<OutputEvent>>,
    queries: &mut Vec<(String, serde_json::Value)>,
) -> bool {
    let mut disconnect = false;
    while let Some(next) = ready() {
        if matches!(&*next, OutputEvent::Disconnect { .. }) {
            disconnect = true;
        }
        remember_the_room(manifest, &next);
        if let Some(t) = tui.as_deref_mut() {
            queries.extend(t.apply_event(&next));
        }
    }
    disconnect
}

/// Render one system line the way a world `Sys` line renders.
fn sys_line(sh: &Shared, theme: &AnsiTheme, out: &mut Vec<u8>, text: &str) {
    mush_ansi::render(
        &OutputEvent::Sys(RichText::tag("system", text)),
        &sh.caps,
        theme,
        out,
    );
}

/// Render one `@history` page: a header, the lines OLDEST FIRST (the wire
/// carries them newest first), and a footer naming the next step. Advances
/// the connection's cursor, which is the only state it touches.
fn render_history_page(
    sh: &mut Shared,
    data: &serde_json::Value,
    theme: &AnsiTheme,
    out: &mut Vec<u8>,
) {
    let Some(page) = crate::backlog::parse(data) else {
        // Nothing truthful to say about a page we could not read, and no
        // cursor to advance: stop, rather than loop on it.
        sh.history.more = false;
        sys_line(sh, theme, out, "--- history is unavailable right now ---");
        return;
    };
    if page.lines.is_empty() {
        // An empty page is TERMINAL, never an error: a reaped session gives
        // one, and so does a ring with nothing older left in it.
        sh.history.more = false;
        sys_line(sh, theme, out, "--- no older history ---");
        return;
    }
    sys_line(
        sh,
        theme,
        out,
        &format!(
            "--- older history: {} line(s), oldest first ---",
            page.lines.len()
        ),
    );
    for line in page.lines.iter().rev() {
        mush_ansi::render(
            &OutputEvent::Narrative {
                kind: line.kind,
                actor: line.actor.clone(),
                text: line.text.clone(),
            },
            &sh.caps,
            theme,
            out,
        );
    }
    sh.history.before = page.oldest_seq().or(sh.history.before);
    sh.history.more = page.more;
    let footer = if page.more {
        "--- @history more for older ---"
    } else if page.evicted {
        "--- older history has rolled off the server's buffer ---"
    } else {
        "--- that is the oldest history the server holds ---"
    };
    sys_line(sh, theme, out, footer);
}

/// Consume the binary's event queue for one connection. In line mode each
/// event renders through `mush-ansi` and interleaves with the line editor,
/// exactly as before. In [`Mode::Tui`] the pump instead applies every ready
/// event to the [`TuiSession`] and repaints one coalesced ratatui frame; it
/// also wakes on the repaint signal (resize / input) and on a WHO-refresh
/// timer. Rendering is synchronous under the lock; the async write happens
/// after the lock drops (invariant T2). Ends when the sender is dropped
/// (session closed) or after a [`OutputEvent::Disconnect`], restoring the
/// terminal first if a TUI was active (invariant T5).
/// The cut a pump arms after it has closed the session's channel (doc 29a
/// §8 A-3, applied in §9): a client that never answers the server's channel
/// close with its own would keep the connection, and its slot, with nothing
/// open. It gets `close_grace` to close, then a disconnect, then — a
/// [`DISCONNECT_GRACE`] later — the stream is cut. Harmless when the
/// connection has already ended: the disconnect fails to send and the
/// switch cuts a stream that is gone.
struct Cut {
    kill: Arc<KillSwitch>,
    close_grace: Duration,
}

impl Cut {
    fn after_close(self, handle: russh::server::Handle) {
        tokio::spawn(async move {
            tokio::time::sleep(self.close_grace).await;
            if self.kill.is_killed() {
                return;
            }
            let _ = tokio::time::timeout(
                WRITE_TIMEOUT,
                handle.disconnect(
                    Disconnect::ByApplication,
                    "session ended".into(),
                    String::new(),
                ),
            )
            .await;
            tokio::time::sleep(DISCONNECT_GRACE).await;
            self.kill.kill();
        });
    }
}

#[allow(clippy::too_many_arguments)] // Both legacy and stamped adapters share one output pump.
fn spawn_output_pump(
    mut rx: mpsc::Receiver<Arc<OutputEvent>>,
    mut stamped_rx: mpsc::Receiver<Arc<mush_proto::Envelope>>,
    shared: Arc<Mutex<Shared>>,
    handle: russh::server::Handle,
    channel: ChannelId,
    hooks: Arc<dyn SshHooks>,
    conn_id: u64,
    repaint: Arc<Notify>,
    connected: Arc<AtomicBool>,
    cut: Cut,
) {
    tokio::spawn(async move {
        let theme = AnsiTheme::default();
        let mut who_timer = tokio::time::interval(WHO_REFRESH);
        who_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first interval tick fires immediately; swallow it so the timer
        // does not kick a query before the connection even enters the TUI.
        who_timer.tick().await;
        // Pixel cockpit spike (2026-10-21): the frame clock. It ticks for
        // every TUI connection and costs a lock and a bool when the view is
        // closed; a spike's price, noted in its NOTES. The wait is read from
        // the view each time round (20 Hz, or ~1 ms while `u` uncaps it for
        // the load experiment); a deadline, not an interval, so the view can
        // change it, and a deadline rather than a plain sleep so the render
        // and the write do not stretch the period (an 8 ms frame on a 50 ms
        // sleep measured 17 Hz on dev2). A tick that falls behind resyncs to
        // now instead of bursting.
        let mut next_frame = tokio::time::Instant::now() + crate::tui::pixel::FRAME_INTERVAL;
        // Doc 21 cut G.1b: whether this connection's picture view is open, as
        // the box's viewer count last heard it.
        let mut viewing = false;

        enum Wake {
            Event(Option<Arc<OutputEvent>>),
            Stamped(Option<Arc<mush_proto::Envelope>>),
            Repaint,
            Tick,
            Frame,
        }

        let mut pending = std::collections::VecDeque::new();
        let mut legacy_open = true;
        let mut stamped_open = true;
        let mut exit_status = 0;
        loop {
            // A poisoned lock here only picks the idle wait; the next wake's
            // own lock reports it.
            let frame_wait = shared
                .lock()
                .map_or(crate::tui::pixel::FRAME_INTERVAL, |sh| {
                    match (&sh.mode, sh.tui.as_ref()) {
                        (Mode::Tui, Some(t)) if t.pixel_active() => t.pixel_interval(),
                        _ => crate::tui::pixel::FRAME_INTERVAL,
                    }
                });
            let wake = if let Some(wake) = pending.pop_front() {
                wake
            } else {
                tokio::select! {
                    biased;
                    ev = rx.recv(), if legacy_open => Wake::Event(ev),
                    ev = stamped_rx.recv(), if stamped_open => Wake::Stamped(ev),
                    () = repaint.notified() => Wake::Repaint,
                    _ = who_timer.tick() => Wake::Tick,
                    () = tokio::time::sleep_until(next_frame) => Wake::Frame,
                }
            };
            // Real adapters keep only one sender. A closed unused stream is not logout.
            if matches!(wake, Wake::Event(None)) {
                legacy_open = false;
            }
            if matches!(wake, Wake::Stamped(None)) {
                stamped_open = false;
            }
            if !legacy_open && !stamped_open {
                break;
            }
            if matches!(wake, Wake::Event(None) | Wake::Stamped(None)) {
                continue;
            }
            let (wake, stamp) = match wake {
                Wake::Stamped(Some(env)) => (
                    Wake::Event(Some(Arc::new(env.event.clone()))),
                    Some(env.seq),
                ),
                other => (other, None),
            };
            if matches!(wake, Wake::Frame) {
                next_frame = std::cmp::max(next_frame + frame_wait, tokio::time::Instant::now());
            }

            // WHO refresh: submit a read-only query while a TUI is active
            // (design M15 D5 — never types the WHO command, never stamps
            // idle). No bytes here; the reply repaints via the event path.
            //
            // 16b cut B.2 (`B-59`): the same tick is the cockpit's timer heal.
            // The arena pushes a frame only when one changes, so a pilot
            // sitting still whose `arena.map` answer was dropped would wait for
            // the first thing to move; `heal_queries` answers at most one map
            // ask, under the latch the frame path arms. Collected under the
            // lock, submitted after it drops, exactly as the WHO query is.
            if let Wake::Tick = wake {
                let heal = {
                    let mut sh = shared.lock().expect("conn state poisoned");
                    if matches!(sh.mode, Mode::Tui) {
                        Some((
                            sh.mode_generation,
                            sh.tui
                                .as_mut()
                                .map(|t| t.heal_queries())
                                .unwrap_or_default(),
                        ))
                    } else {
                        None
                    }
                };
                if let Some((generation, heal)) = heal {
                    hooks
                        .query(conn_id, 0, "who".to_owned(), serde_json::json!({}))
                        .await;
                    for (name, args) in heal {
                        submit_query(
                            hooks.as_ref(),
                            &shared,
                            conn_id,
                            name,
                            args,
                            Some(generation),
                        )
                        .await;
                    }
                }
                continue;
            }

            let fence_wake = matches!(wake, Wake::Repaint);
            let mut done = false;
            // Doc 21 cut G.1b: this wake's bytes carry a picture, whose write
            // is timed for the step-down.
            let mut picture = false;
            let mut queries: Vec<(String, serde_json::Value)> = Vec::new();
            // Doc 21 cut G.4: the pixel view's helm orders, sent as typed lines.
            let mut lines: Vec<String> = Vec::new();
            let (bytes, invokes, now_viewing, generation) = {
                let mut sh = shared.lock().expect("conn state poisoned");
                let bytes = match wake {
                    Wake::Repaint => {
                        if matches!(sh.mode, Mode::Tui) {
                            sh.tui.as_mut().map(|t| t.paint()).unwrap_or_default()
                        } else {
                            Vec::new()
                        }
                    }
                    // Sender dropped: the queue is closed, tear down.
                    Wake::Event(None) | Wake::Stamped(_) => {
                        done = true;
                        Vec::new()
                    }
                    Wake::Event(Some(ev)) => {
                        if !accept_history_reply(&mut sh, &ev) {
                            continue;
                        }
                        if matches!(&*ev, OutputEvent::Disconnect { .. }) {
                            done = true;
                        }
                        // 16b cut B.3 (`B-66`): the room is remembered in
                        // EITHER mode, before the branch, so the next `@tui`
                        // opens on the room this connection was last shown.
                        remember_the_room(&mut sh.manifest, &ev);
                        match sh.mode {
                            Mode::Tui if reply_wire_boundary(&ev) => {
                                let mut bytes = Vec::new();
                                if done {
                                    bytes.extend_from_slice(TuiSession::leave_bytes());
                                    sh.tui = None;
                                    sh.mode = Mode::InGame;
                                }
                                mush_ansi::render(&ev, &sh.caps, &theme, &mut bytes);
                                bytes
                            }
                            Mode::Tui => {
                                // Coalesce: apply this event and every other
                                // ready one, then paint a single frame.
                                if let Some(t) = sh.tui.as_mut() {
                                    queries.extend(t.apply_stamped_event(&ev, stamp));
                                }
                                while let Ok(next) = rx.try_recv() {
                                    if reply_wire_boundary(&next) {
                                        pending.push_back(Wake::Event(Some(next)));
                                        break;
                                    }
                                    if !accept_history_reply(&mut sh, &next) {
                                        continue;
                                    }
                                    if matches!(&*next, OutputEvent::Disconnect { .. }) {
                                        done = true;
                                    }
                                    remember_the_room(&mut sh.manifest, &next);
                                    if let Some(t) = sh.tui.as_mut() {
                                        queries.extend(t.apply_event(&next));
                                    }
                                }
                                while pending.is_empty() {
                                    let Ok(next) = stamped_rx.try_recv() else {
                                        break;
                                    };
                                    if reply_wire_boundary(&next.event) {
                                        pending.push_back(Wake::Stamped(Some(next)));
                                        break;
                                    }
                                    if !accept_history_reply(&mut sh, &next.event) {
                                        continue;
                                    }
                                    if matches!(&next.event, OutputEvent::Disconnect { .. }) {
                                        done = true;
                                    }
                                    remember_the_room(&mut sh.manifest, &next.event);
                                    if let Some(t) = sh.tui.as_mut() {
                                        queries.extend(
                                            t.apply_stamped_event(&next.event, Some(next.seq)),
                                        );
                                    }
                                }
                                sh.tui.as_mut().map(|t| t.paint()).unwrap_or_default()
                            }
                            _ => {
                                let mut bytes = Vec::new();
                                render_line_event(&mut sh, &ev, &theme, &mut bytes);
                                bytes
                            }
                        }
                    }
                    Wake::Tick => Vec::new(), // handled above
                    // The frame clock: advance the camera and take the tick's
                    // bytes, which are NOTHING unless the picture changed.
                    Wake::Frame => {
                        if matches!(sh.mode, Mode::Tui) {
                            match sh.tui.as_mut() {
                                Some(t) if t.pixel_active() => {
                                    t.pixel_tick();
                                    let skipped = t.pixel_stats().slow_skips;
                                    let bytes = t.pixel_frame();
                                    if t.pixel_stats().slow_skips > skipped {
                                        hooks.pixel_event(PixelEvent::SlowSkip);
                                    }
                                    picture = !bytes.is_empty();
                                    lines.extend(t.pixel_take_orders());
                                    bytes
                                }
                                Some(t) => {
                                    // A view that just closed may still owe
                                    // its stop order.
                                    lines.extend(t.pixel_take_orders());
                                    Vec::new()
                                }
                                None => Vec::new(),
                            }
                        } else {
                            Vec::new()
                        }
                    }
                };
                let invokes = sh
                    .tui
                    .as_mut()
                    .map(|t| t.take_invokes())
                    .unwrap_or_default();
                let now_viewing = matches!(sh.mode, Mode::Tui)
                    && sh.tui.as_ref().is_some_and(|t| t.pixel_active());
                (bytes, invokes, now_viewing, sh.mode_generation)
            };
            if now_viewing != viewing {
                viewing = now_viewing;
                hooks.pixel_event(if viewing {
                    PixelEvent::Opened
                } else {
                    PixelEvent::Closed
                });
            }

            // A cursor mismatch or an unmergeable delta is never patched
            // locally, and a `Hello` names feeds to hydrate: ask the ordinary
            // read-only query seam after the connection lock has dropped; the
            // TUI de-duplicates resyncs until a valid replacement arrives.
            for (id, request) in invokes {
                hooks.invoke(conn_id, id, request).await;
            }
            for (name, args) in queries {
                submit_query(
                    hooks.as_ref(),
                    &shared,
                    conn_id,
                    name,
                    args,
                    Some(generation),
                )
                .await;
            }
            for line in lines {
                hooks.control_line(conn_id, line).await;
            }

            if !bytes.is_empty() {
                let sent = bytes.len();
                let started = std::time::Instant::now();
                let wrote = tokio::time::timeout(WRITE_TIMEOUT, handle.data(channel, bytes)).await;
                // Doc 21 cut G.1b (§3.4): a picture write that waited means the
                // client's window is full — russh stops taking a channel's data
                // while it cannot send it — so the view skips a frame, and
                // steps down if it keeps happening.
                if picture && matches!(wrote, Ok(Ok(()))) {
                    hooks.pixel_event(PixelEvent::Sent(sent));
                    let change = shared.lock().ok().and_then(|mut sh| {
                        sh.tui
                            .as_mut()
                            .map(|t| t.pixel_write_took(started.elapsed()))
                    });
                    if change == Some(crate::tui::pixel::StepChange::Down) {
                        tracing::info!(
                            conn = conn_id,
                            "picture stepped down: the link is not keeping up"
                        );
                        hooks.pixel_event(PixelEvent::SteppedDown);
                    }
                }
                match wrote {
                    Ok(Ok(())) => {}
                    // Transport gone: nothing left to close.
                    Ok(Err(_)) => return,
                    // Write wedged (client stopped reading — `^S`, dead
                    // peer): give up and close the transport below.
                    Err(_) => {
                        tracing::info!("output write timed out; closing slow connection");
                        exit_status = 1;
                        break;
                    }
                }
            }
            {
                let Ok(mut sh) = shared.lock() else {
                    return;
                };
                sh.history_rendered.notify_waiters();
                if fence_wake && let Some(fence) = sh.output_fence.take() {
                    let _ = fence.send(());
                }
            }
            if done {
                break;
            }
        }

        if viewing {
            hooks.pixel_event(PixelEvent::Closed);
        }
        // If the connection tore down while still on the alternate screen,
        // restore it so the client is never stuck there (invariant T5).
        let was_tui = matches!(shared.lock().expect("conn state poisoned").mode, Mode::Tui);
        if was_tui {
            let _ = tokio::time::timeout(
                WRITE_TIMEOUT,
                handle.data(channel, TuiSession::leave_bytes().to_vec()),
            )
            .await;
        }

        // Close in channel order, after the last data. A connection-level
        // disconnect can discard channel data still buffered by the SSH engine.
        // This is the same drain sequence as the structured exec pump.
        let _ = tokio::time::timeout(
            WRITE_TIMEOUT,
            handle.exit_status_request(channel, exit_status),
        )
        .await;
        let _ = tokio::time::timeout(WRITE_TIMEOUT, handle.eof(channel)).await;
        let _ = tokio::time::timeout(WRITE_TIMEOUT, handle.close(channel)).await;
        if connected.swap(false, Ordering::SeqCst) {
            hooks.disconnected(conn_id).await;
        }
        cut.after_close(handle);
    });
}

// Exec output is structured only: room announcements, prompts and unrelated
// events are never rendered into the source/diagnostic stream.
fn exec_json(result: &mush_proto::InvokeResult) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&result.clone().bounded())
        .unwrap_or_else(|_| br#"{"service":"mind","result":{"status":"error","code":"encoding","message":"Result unavailable."}}"#.to_vec());
    bytes.push(b'\n');
    bytes
}
fn spawn_exec_pump(
    mut rx: mpsc::Receiver<Arc<OutputEvent>>,
    handle: russh::server::Handle,
    channel: ChannelId,
    hooks: Arc<dyn SshHooks>,
    conn: u64,
    connected: Arc<AtomicBool>,
    cut: Cut,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(125);
        let mut status = 2;
        loop {
            let event = match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(event)) => event,
                _ => {
                    let error = mush_proto::InvokeResult::error(
                        "expired",
                        "Authoring request expired or disconnected.",
                    );
                    let _ = tokio::time::timeout(
                        WRITE_TIMEOUT,
                        handle.data(channel, exec_json(&error)),
                    )
                    .await;
                    break;
                }
            };
            match event.as_ref() {
                OutputEvent::InvokeResult { id: 1, result } => {
                    let result = result.clone().bounded();
                    let mush_proto::InvokeResult::Mind(response) = &result else {
                        // This channel was admitted only by the closed Mind exec grammar.
                        break;
                    };
                    if !matches!(
                        tokio::time::timeout(
                            WRITE_TIMEOUT,
                            handle.data(channel, exec_json(&result))
                        )
                        .await,
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                    if response.is_final() {
                        status = u32::from(!response.succeeded());
                        break;
                    }
                }
                OutputEvent::Disconnect { .. } => break,
                _ => {}
            }
        }
        let _ =
            tokio::time::timeout(WRITE_TIMEOUT, handle.exit_status_request(channel, status)).await;
        let _ = tokio::time::timeout(WRITE_TIMEOUT, handle.eof(channel)).await;
        let _ = tokio::time::timeout(WRITE_TIMEOUT, handle.close(channel)).await;
        if connected.swap(false, Ordering::SeqCst) {
            hooks.disconnected(conn).await;
        }
        cut.after_close(handle);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ExitRef, RoomView};

    fn room(name: &str) -> RoomView {
        RoomView {
            name: name.into(),
            id: "#2".into(),
            desc: RichText::plain("A room."),
            contents: vec![],
            exits: vec![ExitRef {
                name: "North".into(),
                aliases: vec!["n".into()],
            }],
            zone: None,
            ambient: Default::default(),
        }
    }

    /// 16b cut B.3 (`B-66`): the pump's record of the last room is fed by a
    /// bare `Room` and by a `Room` answering a query (the attach look B.4
    /// builds), each replacing the last; nothing else touches it.
    #[test]
    fn the_last_room_shown_is_remembered_bare_or_as_a_reply() {
        let mut manifest = TuiManifest::default();
        remember_the_room(&mut manifest, &OutputEvent::Room(room("Hall")));
        assert_eq!(
            manifest.room.as_ref().map(|r| r.name.as_str()),
            Some("Hall")
        );
        remember_the_room(
            &mut manifest,
            &OutputEvent::Reply {
                id: 0,
                event: Box::new(OutputEvent::Room(room("Pod"))),
            },
        );
        assert_eq!(manifest.room.as_ref().map(|r| r.name.as_str()), Some("Pod"));
        remember_the_room(&mut manifest, &OutputEvent::Sys(RichText::plain("x")));
        remember_the_room(
            &mut manifest,
            &OutputEvent::Reply {
                id: 0,
                event: Box::new(OutputEvent::Sys(RichText::plain("x"))),
            },
        );
        assert_eq!(
            manifest.room.as_ref().map(|r| r.name.as_str()),
            Some("Pod"),
            "only a room replaces the room"
        );
    }

    /// 16b cut B.3 (`B-66`): a walk inside the TUI is a burst — the `Sys` line
    /// wakes the pump and the `Room` is queued behind it — so the room is
    /// recorded by the coalescing drain, not by the pre-branch call. The drain
    /// records every event it applies, and reports a `Disconnect` among them.
    #[test]
    fn the_coalescing_drain_remembers_a_room_queued_behind_the_wake() {
        let mut manifest = TuiManifest {
            room: Some(room("Hall")),
            ..TuiManifest::default()
        };
        let mut burst = vec![
            Arc::new(OutputEvent::Sys(RichText::plain("You go north."))),
            Arc::new(OutputEvent::Reply {
                id: 3,
                event: Box::new(OutputEvent::Room(room("Pantry"))),
            }),
            Arc::new(OutputEvent::Sys(RichText::plain("Bob has arrived."))),
        ]
        .into_iter();
        let mut queries = Vec::new();
        let gone = drain_into_tui(&mut manifest, None, || burst.next(), &mut queries);
        assert!(!gone);
        assert_eq!(
            manifest.room.as_ref().map(|r| r.name.as_str()),
            Some("Pantry"),
            "the room queued behind the wake is the one remembered"
        );
        assert!(burst.next().is_none(), "the drain takes the whole burst");
        let mut last = vec![Arc::new(OutputEvent::Disconnect {
            reason: RichText::plain("bye"),
        })]
        .into_iter();
        assert!(drain_into_tui(
            &mut manifest,
            None,
            || last.next(),
            &mut queries
        ));
    }

    /// Doc 21 cut G.1b: `@pixel/stats` is read transport-side, case-folded,
    /// whatever follows it ignored, and nothing else is it.
    #[test]
    fn pixel_stats_is_read_transport_side() {
        assert!(is_pixel_stats("@pixel/stats"));
        assert!(is_pixel_stats("  @PIXEL/Stats now "));
        assert!(!is_pixel_stats("@pixel/statsx"));
        assert!(!is_pixel_stats("say @pixel/stats"));
        assert!(!is_pixel_stats("@pixel/encoder"));
    }

    /// Doc 21 cut G.2: `@pixel/encoder` is read transport-side, case-folded,
    /// and answered from the connection's own state.
    /// Doc 21 cut G.10: `@sounds` is read transport-side, case-folded, and
    /// answered with what is set and how the terminal hears them.
    #[test]
    fn sounds_reads_and_answers() {
        assert_eq!(parse_sounds("@sounds"), Some(Ok(None)));
        assert_eq!(parse_sounds("  @SOUNDS Off "), Some(Ok(Some(false))));
        assert_eq!(parse_sounds("@sounds on"), Some(Ok(Some(true))));
        assert_eq!(parse_sounds("@sounds loud"), Some(Err(())));
        assert_eq!(parse_sounds("@sounds on now"), Some(Err(())));
        assert_eq!(parse_sounds("@soundsx"), None);
        assert_eq!(parse_sounds("say @sounds"), None);
        assert_eq!(
            sounds_reply(true, None),
            "Sounds are on for this connection. The cockpit's sounds are played inside @tui."
        );
        assert_eq!(
            sounds_reply(true, Some((true, Some("w11term 1".into()), true))),
            "Sounds are on for this connection. Your terminal (w11term 1) plays them by name."
        );
        assert_eq!(
            sounds_reply(false, Some((false, None, false))),
            "Sounds are off for this connection. Your terminal hears them as its bell."
        );
    }

    #[test]
    fn pixel_encoder_reads_and_answers() {
        use PixelEncoderReq::{Set, Show, Usage};
        assert_eq!(parse_pixel_encoder("@pixel/encoder"), Some(Show));
        assert_eq!(
            parse_pixel_encoder("  @PIXEL/ENCODER Sixel "),
            Some(Set(EncoderChoice::Forced(Encoder::Sixel)))
        );
        assert_eq!(
            parse_pixel_encoder("@pixel/encoder off"),
            Some(Set(EncoderChoice::Off))
        );
        assert_eq!(parse_pixel_encoder("@pixel/encoder png"), Some(Usage));
        assert_eq!(parse_pixel_encoder("@pixel/encoder auto now"), Some(Usage));
        assert_eq!(parse_pixel_encoder("@pixel/encoders"), None);
        assert_eq!(parse_pixel_encoder("say @pixel/encoder"), None);
        assert_eq!(
            pixel_encoder_reply(Show, EncoderChoice::Auto, None),
            "Picture encoder: auto. Your terminal is asked what it can show when you enter @tui."
        );
        assert_eq!(
            pixel_encoder_reply(
                Set(EncoderChoice::Auto),
                EncoderChoice::Auto,
                Some(("sixel graphics OK", Some(Encoder::Sixel)))
            ),
            "Picture encoder for this connection: auto. Your terminal: sixel graphics OK. Ctrl-G draws the picture in sixel."
        );
        assert!(
            pixel_encoder_reply(Show, EncoderChoice::Off, Some(("kitty graphics OK", None)))
                .ends_with("Ctrl-G draws nothing until you set it back.")
        );
        assert_eq!(
            pixel_encoder_reply(Usage, EncoderChoice::Auto, None),
            PIXEL_ENCODER_USAGE
        );
    }

    /// A connection's shared state as a fresh line-mode session holds it.
    fn line_mode_shared() -> Shared {
        Shared {
            editor: LineEditor::new(),
            base_echo: EchoMode::Normal,
            caps: caps::detect_caps(None, None, None, None, 0, 0),
            term: None,
            colorterm: None,
            lang: None,
            lc_all: None,
            width: 0,
            height: 0,
            cell_size: None,
            mode: Mode::InGame,
            tui: None,
            manifest: TuiManifest::default(),
            repaint: Arc::new(Notify::new()),
            output_fence: None,
            history_rendered: Arc::new(Notify::new()),
            history: HistoryCursor::default(),
            history_request: None,
            next_history_request: 1 << 63,
            mode_generation: 0,
            pixel_choice: EncoderChoice::Auto,
            sounds_on: true,
        }
    }

    #[test]
    fn late_history_reply_cannot_consume_newer_request() {
        let mut sh = line_mode_shared();
        let old = 1 << 63;
        let new = old + 1;
        sh.history_request = Some(new);
        sh.history.pending = Some(Instant::now());
        let page = |id| OutputEvent::Reply {
            id,
            event: Box::new(OutputEvent::Panel {
                feed: "narrative".into(),
                kind: PanelFrameKind::Delta,
                data: serde_json::json!({"backlog":{"lines":[],"more":false,"evicted":false}}),
            }),
        };
        assert!(!accept_history_reply(&mut sh, &page(old)));
        assert_eq!(sh.history_request, Some(new));
        assert!(sh.history.pending.is_some());
        assert!(accept_history_reply(&mut sh, &page(new)));
        assert_eq!(sh.history_request, None);
        assert!(!accept_history_reply(&mut sh, &page(new)));
        assert!(accept_history_reply(
            &mut sh,
            &OutputEvent::Reply {
                id: 7,
                event: Box::new(OutputEvent::Sys(RichText::plain("unrelated query"))),
            }
        ));
    }

    #[test]
    fn pty_echo_preference_survives_prompts_and_async_redraws() {
        assert_eq!(pty_echo_mode(&[]), EchoMode::Normal);
        assert_eq!(pty_echo_mode(&[(russh::Pty::ECHO, 1)]), EchoMode::Normal);
        let mut sh = line_mode_shared();
        sh.base_echo = pty_echo_mode(&[(russh::Pty::ECHO, 0)]);
        sh.editor.set_mode(sh.base_echo);
        let theme = AnsiTheme::default();
        let mut bytes = Vec::new();
        let _ = sh.editor.feed(b"private draft", &mut bytes);
        assert!(bytes.is_empty());
        render_line_event(
            &mut sh,
            &OutputEvent::Sys(RichText::plain("Notice.")),
            &theme,
            &mut bytes,
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("private draft"));
        for (mode, expected) in [
            (InputMode::Masked, EchoMode::Masked),
            (InputMode::NoEcho, EchoMode::NoEcho),
            (InputMode::Normal, EchoMode::NoEcho),
        ] {
            render_line_event(
                &mut sh,
                &OutputEvent::Prompt(mush_proto::PromptSpec { text: None, mode }),
                &theme,
                &mut bytes,
            );
            assert_eq!(sh.editor.mode(), expected);
        }
    }

    /// **16b cut B.4** (`B-67`): an event that renders to nothing on a line
    /// client leaves the pilot's half-typed line alone — no erase, no redraw,
    /// zero bytes — while a line that does render still erases the input line,
    /// prints, and redraws it exactly as before.
    ///
    /// The two nothing-events are the cockpit's own: the cleared frame a panel
    /// is sent when its pilot climbs out, and a per-pulse keyframe, which
    /// declares no shape (a shaped `grid` frame is `VIEW`'s alone).
    #[test]
    fn an_event_that_renders_nothing_leaves_the_input_line_alone() {
        let theme = AnsiTheme::default();
        let mut sh = line_mode_shared();
        sh.editor.set_prompt("> ");
        let mut typed = Vec::new();
        let _ = sh.editor.feed(b"fire", &mut typed);
        assert!(sh.editor.has_visible_line(), "a visible editor line");
        assert_eq!(sh.editor.line(), "fire");

        let cleared = OutputEvent::Panel {
            feed: "arena.cockpit".into(),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
                "seated": false,
            }),
        };
        let pulse = OutputEvent::Panel {
            feed: "arena.cockpit".into(),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
                "match": 7, "callsign": "RA", "power": "online", "contacts": [],
                "notices": [], "weapons": [], "sections": [],
            }),
        };
        for (what, ev) in [("the cleared frame", &cleared), ("a pulse frame", &pulse)] {
            let mut bytes = Vec::new();
            render_line_event(&mut sh, ev, &theme, &mut bytes);
            assert!(
                bytes.is_empty(),
                "{what} wrote {:?} around the pilot's input line",
                String::from_utf8_lossy(&bytes)
            );
        }
        assert_eq!(sh.editor.line(), "fire", "the draft is untouched");

        // A line that renders still interrupts: erase, the line, the redraw.
        let mut bytes = Vec::new();
        render_line_event(
            &mut sh,
            &OutputEvent::Sys(RichText::plain("Locking BA.")),
            &theme,
            &mut bytes,
        );
        let written = String::from_utf8_lossy(&bytes).into_owned();
        assert!(written.starts_with("\r\x1b[K"), "erased first: {written:?}");
        assert!(written.contains("Locking BA."), "{written:?}");
        assert!(
            written.ends_with("> fire"),
            "and the prompt and the draft redrawn: {written:?}"
        );
    }

    /// A scripted [`Acceptor`]: a run of errors, then real accepted
    /// streams, then pending for ever (a quiet listener).
    struct Scripted {
        errors: u32,
        listener: TcpListener,
        accepted: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl Acceptor for Scripted {
        async fn accept(&mut self) -> std::io::Result<(TcpStream, SocketAddr)> {
            if self.errors > 0 {
                self.errors -= 1;
                return Err(std::io::Error::other("Too many open files (os error 24)"));
            }
            let accepted = TcpListener::accept(&self.listener).await?;
            self.accepted.fetch_add(1, Ordering::SeqCst);
            Ok(accepted)
        }
    }

    /// Doc 29a §8 B-2 applied to the public port: an accept error (an
    /// `EMFILE` burst) is logged and retried, and the connections that
    /// follow it are still served. russh's own loop returned on the first
    /// error, which took the SSH port down until a restart.
    #[tokio::test]
    async fn the_accept_loop_survives_accept_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let served = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let served_by_loop = std::sync::Arc::clone(&served);
        let scripted = Scripted {
            errors: 3,
            listener,
            accepted: std::sync::Arc::clone(&accepted),
        };
        let serving = tokio::spawn(serve_accepts(scripted, move |_stream, _peer| {
            served_by_loop.fetch_add(1, Ordering::SeqCst);
        }));
        // The errors cost three backoffs; a client that connects meanwhile
        // waits in the backlog and is served once the loop gets past them.
        let _client = TcpStream::connect(addr).await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while served.load(Ordering::SeqCst) == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the loop never served the connection after the accept errors"
            );
            assert!(!serving.is_finished(), "the accept loop ended on an error");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
        let _second = TcpStream::connect(addr).await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while served.load(Ordering::SeqCst) < 2 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "second connection unserved"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!serving.is_finished());
        serving.abort();
    }
}
