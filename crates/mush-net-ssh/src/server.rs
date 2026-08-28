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
//! [`SshHooks::auth_account`] (public-key auth arrives with `@sshkey` in
//! M2/M3 and is rejected toward password until then).
//!
//! A PTY is required; `exec`/`subsystem` requests are refused. Once a
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
//! Abuse gates: every accepted connection claims a per-IP slot via
//! [`SshHooks::connection_opened`] **before any auth** (released exactly
//! once on handler drop), in-band welcome `connect`/`create` re-check
//! [`SshHooks::is_ip_blocked`] on every attempt (so a tempban armed
//! mid-connection stops further guessing), and a connection that has not
//! reached a character within [`PREAUTH_TIMEOUT`] is dropped.
//!
//! Deliberate M1 deferrals from design 02 §5.3 (recorded here so they read
//! as decisions, not oversights): welcome-screen `WHO` is stubbed until
//! pre-auth world queries exist, and in-band `connect`/`create` passwords
//! are echoed in the clear (the classic front door; the wire is encrypted
//! and the line is excluded from editor history) — masked prompts arrive
//! with the M2 account wizard.

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_ansi::AnsiTheme;
use mush_proto::{ClientCaps, InputMode, OutputEvent};
use mush_session::{FloodBucket, FloodVerdict, sanitize_line};
use russh::server::{Auth, Config, Handler, Msg, Response, Server as _, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::editor::{EchoMode, EditorAction, LineEditor};
use crate::hooks::{AuthResult, CharSummary, NewConn, SshHooks};
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

/// Cap on one output-pump write to the transport (design 02 §8.4: "try 5 s
/// then disconnect"). A client that stops reading (terminal `^S`, dead
/// peer behind a live TCP window) stalls `handle.data` on SSH window
/// space; this bounds it so the pump can close the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

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
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshError::HostKey { path, detail } => {
                write!(f, "SSH host key at {path}: {detail}")
            }
            SshError::Bind { addr, source } => write!(f, "binding {addr}: {source}"),
            SshError::Server(e) => write!(f, "SSH server: {e}"),
        }
    }
}

impl std::error::Error for SshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SshError::Bind { source, .. } | SshError::Server(source) => Some(source),
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

    /// Load/generate the host key and bind the listener without accepting
    /// yet.
    pub async fn bind(
        cfg: SshConfig,
        hooks: Arc<dyn SshHooks>,
    ) -> Result<BoundSshServer, SshError> {
        let key = hostkey::load_or_generate(&cfg.host_key_path)?;
        let mut methods = MethodSet::empty();
        methods.push(MethodKind::None);
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
            ingress: Ingress {
                hooks,
                banner: Arc::from(cfg.banner.as_str()),
                next_conn: Arc::new(AtomicU64::new(1)),
            },
        })
    }
}

/// A bound-but-not-yet-serving SSH server.
pub struct BoundSshServer {
    addr: SocketAddr,
    listener: TcpListener,
    config: Arc<Config>,
    ingress: Ingress,
}

impl BoundSshServer {
    /// The actual bound address (resolves ephemeral ports).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serve until the accept loop fails.
    pub async fn run(mut self) -> Result<(), SshError> {
        self.ingress
            .run_on_socket(self.config, &self.listener)
            .await
            .map_err(SshError::Server)
    }
}

// ---- per-connection machinery ------------------------------------------

struct Ingress {
    hooks: Arc<dyn SshHooks>,
    banner: Arc<str>,
    next_conn: Arc<AtomicU64>,
}

impl russh::server::Server for Ingress {
    type Handler = ConnHandler;

    fn new_client(&mut self, peer: Option<SocketAddr>) -> ConnHandler {
        let conn_id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let addr = peer
            .map(|p| p.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        // Claim the per-IP slot at accept time, before any auth, so
        // welcome-screen squatters count against the cap too. An over-cap
        // connection is dropped at its first auth message.
        let counted = self.hooks.connection_opened(addr);
        if !counted {
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
                caps: caps::detect_caps(None, None, None, None, 0, 0),
                term: None,
                colorterm: None,
                lang: None,
                lc_all: None,
                width: 0,
                height: 0,
                mode: Mode::Welcome,
            })),
            route: Route::PreAuth,
            channel: None,
            pty_seen: false,
            shell_started: false,
            connected: Arc::new(AtomicBool::new(false)),
            counted,
            flood: FloodBucket::new(PREAUTH_FLOOD_RATE, PREAUTH_FLOOD_BURST, Instant::now()),
        }
    }

    fn handle_session_error(&mut self, error: russh::Error) {
        tracing::debug!(%error, "SSH session ended with error");
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
    /// `^D` quit confirmation; `prev` is restored on cancel.
    QuitConfirm { prev: Box<Mode> },
}

/// How SSH-level auth resolved.
#[derive(Clone, Debug)]
enum Route {
    PreAuth,
    /// Front-door username: welcome screen, in-band connect/create.
    Welcome,
    /// `<account>` username authenticated via [`SshHooks::auth_account`].
    Account {
        account: String,
        characters: Vec<CharSummary>,
    },
}

/// State shared between the russh handler and the output pump. The mutex
/// is `std::sync` and never held across an await.
struct Shared {
    editor: LineEditor,
    caps: ClientCaps,
    term: Option<String>,
    colorterm: Option<String>,
    lang: Option<String>,
    lc_all: Option<String>,
    width: u16,
    height: u16,
    mode: Mode,
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

struct ConnHandler {
    hooks: Arc<dyn SshHooks>,
    banner: Arc<str>,
    conn_id: u64,
    addr: IpAddr,
    shared: Arc<Mutex<Shared>>,
    route: Route,
    channel: Option<ChannelId>,
    pty_seen: bool,
    shell_started: bool,
    /// Set just before `connect_character` is attempted (cleared again on
    /// refusal); gates the `disconnected` hook (fired exactly once, from
    /// `Drop`). Armed *before* the await so a handler future dropped
    /// mid-connect still gets its cleanup — the binary treats a
    /// `disconnected` for a never-attached conn as a no-op.
    connected: Arc<AtomicBool>,
    /// Whether this connection claimed a per-IP slot at accept time
    /// (`SshHooks::connection_opened`); released exactly once in `Drop`.
    /// `false` means over-cap: refused at the first auth message.
    counted: bool,
    flood: FloodBucket,
}

impl Drop for ConnHandler {
    fn drop(&mut self) {
        // Release the per-IP slot symmetrically with new_client — every
        // path (auth refused, welcome squatter, mid-connect teardown, clean
        // quit) comes through here exactly once.
        if self.counted {
            self.hooks.connection_closed(self.addr);
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

/// `play` / `guest` / `new` — the auth-none front door (M1: all three
/// alias to the welcome screen).
fn is_front_door(user: &str) -> bool {
    user.eq_ignore_ascii_case("play")
        || user.eq_ignore_ascii_case("guest")
        || user.eq_ignore_ascii_case("new")
}

/// Methods a rejected client should try next.
fn interactive_methods() -> MethodSet {
    let mut m = MethodSet::empty();
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

impl ConnHandler {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().expect("conn state poisoned")
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

    /// Farewell + close channel + disconnect.
    fn close_polite(&mut self, session: &mut Session, msg: &str) -> Result<(), russh::Error> {
        if !msg.is_empty() {
            self.write_text(session, msg)?;
        }
        if let Some(ch) = self.channel {
            let _ = session.close(ch);
        }
        let _ = session.disconnect(Disconnect::ByApplication, "goodbye", "");
        Ok(())
    }

    async fn verify_account(&mut self, user: &str, password: &str) -> Auth {
        match self.hooks.auth_account(user, password).await {
            AuthResult::Ok {
                account,
                characters,
            } => {
                self.route = Route::Account {
                    account,
                    characters,
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

    // ---- line dispatch --------------------------------------------------

    async fn handle_line(
        &mut self,
        session: &mut Session,
        raw: String,
    ) -> Result<(), russh::Error> {
        let line = sanitize_line(&raw);
        let mode = self.lock().mode.clone();
        match mode {
            Mode::InGame => {
                self.hooks.line(self.conn_id, line).await;
                Ok(())
            }
            Mode::Welcome => self.welcome_line(session, &line).await,
            Mode::CharSelect { account, chars } => {
                self.charselect_line(session, &account, &chars, &line).await
            }
            Mode::QuitConfirm { prev } => self.quit_confirm_line(session, *prev, &line).await,
        }
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
            return self.close_polite(
                session,
                "Too many failed logins from your address; try again later.",
            );
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
            } => self.enter_account(session, account, characters).await,
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
            return self.close_polite(
                session,
                "Too many failed logins from your address; try again later.",
            );
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

    async fn enter_account(
        &mut self,
        session: &mut Session,
        account: String,
        chars: Vec<CharSummary>,
    ) -> Result<(), russh::Error> {
        match chars.len() {
            0 => self.write_text(
                session,
                "That account has no characters yet; ask staff for help.",
            ),
            1 => {
                let c = chars[0].clone();
                self.do_connect(session, &account, &c).await
            }
            _ => {
                self.lock().mode = Mode::CharSelect {
                    account: account.clone(),
                    chars: chars.clone(),
                };
                self.print_char_select(session, &account, &chars)
            }
        }
    }

    fn print_char_select(
        &self,
        session: &mut Session,
        account: &str,
        chars: &[CharSummary],
    ) -> Result<(), russh::Error> {
        let mut text = format!("Characters on {account}:\n");
        for (i, c) in chars.iter().enumerate() {
            text.push_str(&format!("  {}) {}\n", i + 1, c.name));
        }
        text.push_str("Enter a number (or QUIT):");
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
        let snapshot = self.lock().caps;
        let conn = NewConn {
            id: self.conn_id,
            addr: self.addr,
            caps: snapshot,
            tx,
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
                tracing::info!(
                    conn = self.conn_id,
                    account,
                    character = character.name,
                    "character connected"
                );
                self.lock().mode = Mode::InGame;
                spawn_output_pump(rx, Arc::clone(&self.shared), session.handle(), channel);
                Ok(())
            }
            Err(why) => {
                self.connected.store(false, Ordering::SeqCst);
                self.write_text(session, &format!("Connection failed: {why}"))
            }
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
            self.hooks.line(self.conn_id, "QUIT".to_owned()).await;
            Ok(())
        } else {
            self.close_polite(session, "Goodbye.")
        }
    }
}

impl Handler for ConnHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        if !self.counted {
            // Over the per-IP connection cap: shed the connection outright.
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
        }
        if is_front_door(user) {
            self.route = Route::Welcome;
            Ok(Auth::Accept)
        } else {
            Ok(reject_toward_interactive())
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if !self.counted {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
        }
        if is_front_door(user) {
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
        if !self.counted {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
        }
        if is_front_door(user) {
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
        _user: &str,
        _key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        // Public-key auth lands with in-game @sshkey registration (M2/M3);
        // steer clients to password/keyboard-interactive without burning
        // signature verification.
        Ok(reject_toward_interactive())
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(reject_toward_interactive())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel.is_some() {
            // One session channel per connection; dropping the handle
            // rejects the request.
            drop(reply);
            return Ok(());
        }
        self.channel = Some(channel.id());
        reply.accept().await;
        // Pre-auth watchdog (design 02 §8.2): a connection that has not
        // reached a character within the timeout is dropped, so idle
        // welcome-screen squatters cannot hold slots forever. In-game
        // idlers are untouched (`connected` is set by then).
        let handle = session.handle();
        let connected = Arc::clone(&self.connected);
        let channel_id = channel.id();
        let conn_id = self.conn_id;
        tokio::spawn(async move {
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
            }
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            self.pty_seen = true;
            let mut sh = self.lock();
            sh.term = Some(term.to_owned());
            sh.width = u16::try_from(col_width).unwrap_or(u16::MAX);
            sh.height = u16::try_from(row_height).unwrap_or(u16::MAX);
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
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            let caps = {
                let mut sh = self.lock();
                sh.width = u16::try_from(col_width).unwrap_or(u16::MAX);
                sh.height = u16::try_from(row_height).unwrap_or(u16::MAX);
                sh.recompute_caps();
                sh.caps
            };
            if self.connected.load(Ordering::SeqCst) {
                self.hooks.caps_changed(self.conn_id, caps);
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
            } => self.enter_account(session, account, characters).await,
        }
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Interactive shell only (design 02 §5.6): no exec in v1.
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
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
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channel == Some(channel) {
            return Err(russh::Error::Disconnect);
        }
        Ok(())
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

// ---- output pump --------------------------------------------------------

/// Consume the binary's event queue for one connection: render through
/// `mush-ansi` with the live caps and interleave with the line editor
/// (erase the in-progress line, print the event, redraw prompt+buffer).
/// Ends when the sender is dropped (session closed) or after a
/// [`OutputEvent::Disconnect`], closing the transport either way.
fn spawn_output_pump(
    mut rx: mpsc::Receiver<Arc<OutputEvent>>,
    shared: Arc<Mutex<Shared>>,
    handle: russh::server::Handle,
    channel: ChannelId,
) {
    tokio::spawn(async move {
        let theme = AnsiTheme::default();
        while let Some(ev) = rx.recv().await {
            let is_disconnect = matches!(&*ev, OutputEvent::Disconnect { .. });
            let bytes = {
                let mut sh = shared.lock().expect("conn state poisoned");
                let mut bytes = Vec::new();
                match &*ev {
                    OutputEvent::Prompt(spec) => {
                        sh.editor.set_mode(match spec.mode {
                            InputMode::Normal => EchoMode::Normal,
                            InputMode::Masked => EchoMode::Masked,
                            InputMode::NoEcho => EchoMode::NoEcho,
                        });
                        if spec.text.is_some() {
                            // Render the prompt once and store it; redraw
                            // re-prints it from here on.
                            let mut prompt = Vec::new();
                            mush_ansi::render(&ev, &sh.caps, &theme, &mut prompt);
                            sh.editor
                                .set_prompt(String::from_utf8_lossy(&prompt).into_owned());
                        }
                        sh.editor.redraw(&mut bytes);
                    }
                    _ => {
                        let interrupted = sh.editor.has_visible_line();
                        if interrupted {
                            bytes.extend_from_slice(b"\r\x1b[K");
                        }
                        mush_ansi::render(&ev, &sh.caps, &theme, &mut bytes);
                        if interrupted {
                            sh.editor.redraw(&mut bytes);
                        }
                    }
                }
                bytes
            };
            if !bytes.is_empty() {
                match tokio::time::timeout(WRITE_TIMEOUT, handle.data(channel, bytes)).await {
                    Ok(Ok(())) => {}
                    // Transport gone: nothing left to close.
                    Ok(Err(_)) => return,
                    // Write wedged (client stopped reading — `^S`, dead
                    // peer): give up and close the transport below.
                    Err(_) => {
                        tracing::info!("output write timed out; closing slow connection");
                        break;
                    }
                }
            }
            if is_disconnect {
                break;
            }
        }
        // Session closed (QUIT, @boot, linkdead sweep) or explicit
        // Disconnect event: close the transport.
        let _ = handle.close(channel).await;
        let _ = handle
            .disconnect(
                Disconnect::ByApplication,
                "session closed".into(),
                "".into(),
            )
            .await;
    });
}
