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
use mush_proto::{ClientCaps, InputMode, OutputEvent, PanelFrameKind, RichText};
use mush_session::{FloodBucket, FloodVerdict, sanitize_line};
use russh::server::{Auth, Config, Handler, Msg, Response, Server as _, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::editor::{EchoMode, EditorAction, LineEditor};
use crate::hooks::{AuthResult, CharSummary, NewConn, SshHooks};
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
                tui: None,
                manifest: TuiManifest::default(),
                repaint: Arc::new(Notify::new()),
                history: HistoryCursor::default(),
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
    /// Full-screen ratatui TUI (milestone M15), entered from `InGame` with
    /// `@tui` and left with `@notui` / the detach hotkey. Raw input drives
    /// the TUI app (not the line editor) and the pump repaints frames; the
    /// [`TuiSession`] itself lives in [`Shared::tui`].
    Tui,
    /// `^D` quit confirmation; `prev` is restored on cancel.
    QuitConfirm { prev: Box<Mode> },
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
    caps: ClientCaps,
    term: Option<String>,
    colorterm: Option<String>,
    lang: Option<String>,
    lc_all: Option<String>,
    width: u16,
    height: u16,
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
    /// Where line mode's `@history` has read back to (interface plan I12).
    history: HistoryCursor,
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
fn split_account_user(user: &str) -> (&str, Option<&str>) {
    match user.split_once('+') {
        Some((account, character)) if !character.is_empty() => (account, Some(character)),
        Some((account, _)) => (account, None),
        None => (user, None),
    }
}

/// `play` / `guest` / `new` — the auth-none front door (M1: all three
/// alias to the welcome screen).
fn is_front_door(user: &str) -> bool {
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
        // The username may be `<account>` or `<account>+<char>` (M19): auth
        // always uses the account part; the character part rides the route.
        let (account_user, requested_char) = split_account_user(user);
        match self.hooks.auth_account(account_user, password).await {
            AuthResult::Ok {
                account,
                characters,
            } => {
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
                // `@tui` is intercepted transport-side, before the world sees
                // it (design M15 D2); every other line forwards unchanged, so
                // a connection that never types `@tui` behaves exactly as
                // today (invariant T1).
                if line.trim().eq_ignore_ascii_case("@tui") {
                    return self.enter_tui(session).await;
                }
                // Interface plan I12: `@history [n|more]` is answered from
                // the gateway's per-viewer resume ring through the read-only
                // query seam, intercepted here exactly like `@tui`. The
                // world never sees it — which also means, as a consequence
                // and not an oversight, that it does NOT stamp idle
                // (`Core::query` deliberately skips `record_input`;
                // D-I-3, M15 D5). A player reading a scene back therefore
                // reads as idle in WHO, the same as one sitting in the TUI.
                // If staff ever want it otherwise the fix is a
                // `record_input` here, never a change to the query seam.
                if let Some(req) = parse_history(&line) {
                    return self.history_line(session, req).await;
                }
                self.hooks.line(self.conn_id, line).await;
                Ok(())
            }
            // Reached only if a line is submitted through the editor while in
            // TUI mode (the normal TUI input path is `ConnHandler::data`);
            // forward it so the world still sees the command.
            Mode::Tui => {
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

    // ---- TUI (milestone M15) --------------------------------------------

    /// Pulse the repaint signal so the output pump renders a fresh frame
    /// without waiting for a world event (input edits, resize).
    fn fire_repaint(&self) {
        let notify = Arc::clone(&self.lock().repaint);
        notify.notify_one();
    }

    /// `@tui`: build the [`TuiSession`], enter the alternate screen, paint the
    /// first frame, and kick a WHO refresh. From here the output pump owns all
    /// steady-state repaints (a single writer), so no frame races the enter.
    async fn enter_tui(&mut self, session: &mut Session) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let (bytes, queries) = {
            let mut sh = self.lock();
            let caps = sh.caps;
            let mut tui = Box::new(TuiSession::new(caps, sh.manifest.clone()));
            let mut out = TuiSession::enter_bytes().to_vec();
            out.extend_from_slice(&tui.paint());
            let queries = tui.entry_queries();
            sh.tui = Some(tui);
            sh.mode = Mode::Tui;
            (out, queries)
        };
        session.data(channel, bytes)?;
        // Read-only hydration (WHO, the command table, channels, vitals, the
        // screen's own feeds); every reply lands on the pump, which repaints.
        // None of it stamps idle (design M15 D5, interface plan I1 D-I-3).
        for (name, args) in queries {
            self.hooks.query(self.conn_id, name, args).await;
        }
        Ok(())
    }

    /// `@notui` / detach: leave the alternate screen, restore the cursor, drop
    /// the TUI state, and hand the resumed line client its prompt back
    /// (invariant T5 — never leave a client stuck on the alternate screen).
    async fn leave_tui(&mut self, session: &mut Session) -> Result<(), russh::Error> {
        let Some(channel) = self.channel else {
            return Ok(());
        };
        let bytes = {
            let mut sh = self.lock();
            sh.tui = None;
            sh.mode = Mode::InGame;
            let mut out = TuiSession::leave_bytes().to_vec();
            // Repaint the line editor's prompt + in-progress buffer so the
            // resumed line client is not left on a blank line.
            sh.editor.redraw(&mut out);
            out
        };
        session.data(channel, bytes)
    }

    /// Raw input while in [`Mode::Tui`]: decode it into the TUI app, then
    /// repaint / submit / detach. Painting is left to the pump (fired via the
    /// repaint signal) so there is one steady-state frame writer.
    async fn tui_data(&mut self, data: &[u8], session: &mut Session) -> Result<(), russh::Error> {
        let result = {
            let mut sh = self.lock();
            match sh.tui.as_mut() {
                Some(tui) => tui.feed_input(data),
                None => return Ok(()),
            }
        };
        if result.detach {
            return self.leave_tui(session).await;
        }
        if !result.submit.is_empty() {
            // Show the cleared input line immediately; the world's echo
            // repaints again via the pump.
            self.fire_repaint();
            // A burst can carry more than one line (a multi-line paste); each
            // is forwarded in order so none is dropped.
            for line in result.submit {
                // `@notui` typed at the TUI input line also detaches.
                if line.trim().eq_ignore_ascii_case("@notui") {
                    return self.leave_tui(session).await;
                }
                self.hooks.line(self.conn_id, line).await;
            }
            return Ok(());
        }
        // Entering a screen or refreshing a pane submits read-only queries;
        // each reply lands on the pump and repaints. Like the WHO rail, none
        // of it stamps idle (design M15 D5).
        for (name, args) in result.queries {
            self.hooks.query(self.conn_id, name, args).await;
        }
        for (id, epoch, choice) in result.responds {
            self.hooks.respond(self.conn_id, id, epoch, choice).await;
        }
        if result.redraw {
            self.fire_repaint();
        }
        Ok(())
    }

    // ---- line-mode scene history (interface plan I12) --------------------

    /// Write one transport-side system line the way a world `Sys` line
    /// looks (this connection's caps and the shared theme), around the line
    /// editor exactly as the pump does.
    fn write_sys(&self, session: &mut Session, text: &str) -> Result<(), russh::Error> {
        let Some(ch) = self.channel else {
            return Ok(());
        };
        let bytes = {
            let sh = self.lock();
            let mut bytes = Vec::new();
            let interrupted = sh.editor.has_visible_line();
            if interrupted {
                bytes.extend_from_slice(b"\r\x1b[K");
            }
            mush_ansi::render(
                &OutputEvent::Sys(RichText::tag("system", text)),
                &sh.caps,
                &AnsiTheme::default(),
                &mut bytes,
            );
            if interrupted {
                sh.editor.redraw(&mut bytes);
            }
            bytes
        };
        session.data(ch, bytes)
    }

    /// `@history [n|more]`: ask the gateway for a page of this viewer's own
    /// resume ring. The page itself arrives asynchronously as a
    /// `Reply{Panel{narrative, Delta}}` and is rendered by the output pump
    /// (see `render_history_page`); only the refusals are written here.
    async fn history_line(
        &mut self,
        session: &mut Session,
        req: HistoryReq,
    ) -> Result<(), russh::Error> {
        let HistoryReq::Page { limit, more } = req else {
            return self.write_sys(session, HISTORY_USAGE);
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
            Next::Say(msg) => self.write_sys(session, msg),
            Next::Ask(before) => {
                // An explicit, non-zero limit: the gateway does
                // `.unwrap_or(0).clamp(1, 200)`, so an absent or zero limit
                // would silently yield a ONE-line page with `more: true`.
                self.hooks
                    .query(
                        self.conn_id,
                        "narrative.backlog".to_owned(),
                        serde_json::json!({
                            "before_seq": before,
                            "limit": limit.clamp(1, HISTORY_MAX),
                        }),
                    )
                    .await;
                Ok(())
            }
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
                let repaint = {
                    let mut sh = self.lock();
                    sh.mode = Mode::InGame;
                    Arc::clone(&sh.repaint)
                };
                spawn_output_pump(
                    rx,
                    Arc::clone(&self.shared),
                    session.handle(),
                    channel,
                    Arc::clone(&self.hooks),
                    self.conn_id,
                    repaint,
                );
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
        if !self.counted {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
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
        if !self.counted {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
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
        if !self.counted {
            return Err(russh::Error::Disconnect);
        }
        if self.hooks.is_ip_blocked(self.addr) {
            return Ok(Auth::reject());
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
            let (caps, is_tui) = {
                let mut sh = self.lock();
                sh.width = u16::try_from(col_width).unwrap_or(u16::MAX);
                sh.height = u16::try_from(row_height).unwrap_or(u16::MAX);
                sh.recompute_caps();
                let caps = sh.caps;
                // Reflow the TUI to the new geometry; the next repaint clears
                // + redraws at the new size (invariant T4).
                if let Some(tui) = sh.tui.as_mut() {
                    tui.set_caps(caps);
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
        // In TUI mode raw bytes drive the ratatui app (input line + nav
        // hotkeys), not the line editor (design M15 D4).
        if matches!(self.lock().mode, Mode::Tui) {
            return self.tui_data(data, session).await;
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

/// Render one event the classic line way: the exact erase/redraw dance the
/// pump has always used around the line editor. Kept as a function so the
/// TUI branch can sit beside it without perturbing a byte of line output
/// (invariant T1).
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
                InputMode::Normal => EchoMode::Normal,
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
            let interrupted = sh.editor.has_visible_line();
            if interrupted {
                bytes.extend_from_slice(b"\r\x1b[K");
            }
            mush_ansi::render(ev, &sh.caps, theme, bytes);
            if interrupted {
                sh.editor.redraw(bytes);
            }
        }
    }
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
fn spawn_output_pump(
    mut rx: mpsc::Receiver<Arc<OutputEvent>>,
    shared: Arc<Mutex<Shared>>,
    handle: russh::server::Handle,
    channel: ChannelId,
    hooks: Arc<dyn SshHooks>,
    conn_id: u64,
    repaint: Arc<Notify>,
) {
    tokio::spawn(async move {
        let theme = AnsiTheme::default();
        let mut who_timer = tokio::time::interval(WHO_REFRESH);
        who_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first interval tick fires immediately; swallow it so the timer
        // does not kick a query before the connection even enters the TUI.
        who_timer.tick().await;

        enum Wake {
            Event(Option<Arc<OutputEvent>>),
            Repaint,
            Tick,
        }

        loop {
            let wake = tokio::select! {
                biased;
                ev = rx.recv() => Wake::Event(ev),
                () = repaint.notified() => Wake::Repaint,
                _ = who_timer.tick() => Wake::Tick,
            };

            // WHO refresh: submit a read-only query while a TUI is active
            // (design M15 D5 — never types the WHO command, never stamps
            // idle). No bytes here; the reply repaints via the event path.
            if let Wake::Tick = wake {
                let is_tui = matches!(shared.lock().expect("conn state poisoned").mode, Mode::Tui);
                if is_tui {
                    hooks
                        .query(conn_id, "who".to_owned(), serde_json::json!({}))
                        .await;
                }
                continue;
            }

            let mut done = false;
            let mut queries: Vec<(String, serde_json::Value)> = Vec::new();
            let bytes = {
                let mut sh = shared.lock().expect("conn state poisoned");
                match wake {
                    Wake::Repaint => {
                        if matches!(sh.mode, Mode::Tui) {
                            sh.tui.as_mut().map(|t| t.paint()).unwrap_or_default()
                        } else {
                            Vec::new()
                        }
                    }
                    // Sender dropped: the queue is closed, tear down.
                    Wake::Event(None) => {
                        done = true;
                        Vec::new()
                    }
                    Wake::Event(Some(ev)) => {
                        if matches!(&*ev, OutputEvent::Disconnect { .. }) {
                            done = true;
                        }
                        match sh.mode {
                            Mode::Tui => {
                                // Coalesce: apply this event and every other
                                // ready one, then paint a single frame.
                                if let Some(t) = sh.tui.as_mut() {
                                    queries.extend(t.apply_event(&ev));
                                }
                                while let Ok(next) = rx.try_recv() {
                                    if matches!(&*next, OutputEvent::Disconnect { .. }) {
                                        done = true;
                                    }
                                    if let Some(t) = sh.tui.as_mut() {
                                        queries.extend(t.apply_event(&next));
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
                }
            };

            // A cursor mismatch or an unmergeable delta is never patched
            // locally, and a `Hello` names feeds to hydrate: ask the ordinary
            // read-only query seam after the connection lock has dropped; the
            // TUI de-duplicates resyncs until a valid replacement arrives.
            for (name, args) in queries {
                hooks.query(conn_id, name, args).await;
            }

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
            if done {
                break;
            }
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
