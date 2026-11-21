//! The bridge between the SSH front end (`mush-net-ssh`) and the game:
//! account auth, session attachment, input, and disconnect handling.
//!
//! ## Division of labor
//!
//! - The **transport** (`mush-net-ssh`) owns
//!   russh, the welcome screen, the LineEditor, and rendering
//!   (`mush-ansi`); it calls the hooks with plain data and closes the
//!   socket when the output queue's sender is dropped (session detached)
//!   or an [`OutputEvent::Disconnect`] arrives.
//! - The **game side** lives in the shared [`GameBridge`]
//!   ([`crate::bridge_common`]) — the same core the web bridge
//!   ([`crate::web_bridge`]) delegates to: argon2 checks via the
//!   [`AuthBackend`] (off the async workers with `spawn_blocking`), the
//!   `SessionManager` attach/detach lifecycle, flood control, input
//!   sanitation, `QUIT`, linkdead grace reaping, and the IP guard.
//! - Failure counting: the bridge **never** records auth failures itself;
//!   the transport calls [`SshBridge::auth_failed`] for every failed
//!   attempt (SSH-level and in-band `connect` alike), so each failure is
//!   counted exactly once. Likewise the transport gates on
//!   [`SshBridge::is_ip_blocked`] before every auth attempt — SSH-level
//!   *and* in-band welcome-screen `connect`/`create` — so a tempban armed
//!   mid-connection shuts down further in-band guessing too.
//! - Connection accounting: the transport claims a per-IP slot via the
//!   `connection_opened` hook at **accept** time (before any auth) and
//!   releases it exactly once when the connection handler drops, so
//!   unauthenticated welcome-screen connections count against the cap and
//!   no error path can leak a slot.
//!
//! ## Shape
//!
//! The game-side flows are inherent methods delegating to the shared
//! [`GameBridge`] (typed with [`ConnId`], [`ConnectErr`], [`LineOutcome`] —
//! and directly testable without sockets); the `net` block at the bottom
//! adapts them to the dyn-compatible [`mush_net_ssh::SshHooks`] trait and
//! starts the real listener ([`start_listener`]).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use mush_core::ObjId;
use mush_net_ssh::kill::KillSwitch;
use mush_net_ssh::resume::{Failure, Redeemed, Ticket, ViewPrefs};
use mush_proto::{ClientCaps, OutputEvent, RespondChoice};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use smol_str::SmolStr;
use tokio::sync::mpsc;

use crate::drain::{CloseWay, GatewayDrain};
use crate::poison::locked;

// The shared bridge vocabulary lives in bridge_common; re-exported here so
// existing SSH-side imports (`mushd::ssh_bridge::{AuthResult, ...}`) keep
// working unchanged.
pub use crate::bridge_common::{AuthResult, CharSummary, ConnectErr, LineOutcome};

use mush_config::ipguard::IpGuard;

use crate::accounts::{AuthBackend, AuthCheckOutcome, AuthOutcome, CreateErr};
use crate::bridge_common::GameBridge;

/// A freshly authenticated connection, handed over by the transport.
#[derive(Debug)]
pub struct NewConn {
    /// Connection id (allocated by the transport, unique per process).
    pub id: ConnId,
    /// Peer address.
    pub addr: IpAddr,
    /// Negotiated client capabilities.
    pub caps: ClientCaps,
    /// The transport's output channel (the writer task holds the rx side).
    pub tx: mpsc::Sender<Arc<OutputEvent>>,
    /// Optional SSH envelope stream; legacy adapters may keep using `tx`.
    pub stamped_tx: Option<mpsc::Sender<Arc<mush_proto::Envelope>>>,
}

impl From<mush_net_ssh::NewConn> for NewConn {
    fn from(conn: mush_net_ssh::NewConn) -> Self {
        Self {
            id: ConnId(conn.id),
            addr: conn.addr,
            caps: conn.caps,
            tx: conn.tx,
            stamped_tx: conn.stamped_tx,
        }
    }
}

/// The production hooks implementation. Generic over [`AuthBackend`] so the
/// M2 Redis store slots in without touching this file.
pub struct SshBridge<A: AuthBackend + 'static> {
    core: GameBridge<A>,
    resume: mush_net_ssh::resume::Registry,
    cockpit_transports: Mutex<HashMap<ConnId, Arc<CockpitTransport>>>,
    /// Shared metrics registry — SSH transport connection counters. Defaults
    /// to a throwaway registry (tests); the real server injects the shared one
    /// via [`SshBridge::with_metrics`].
    metrics: Arc<mush_net_web::Metrics>,
    /// The terminal picture's knobs from `[pixel]` (doc 21 cut G.1b); the
    /// built-in defaults until [`SshBridge::with_pixel`] sets them.
    pixel: mush_net_ssh::PixelConfig,
    /// The bit set on every connection id the SSH transports allocate (doc 33
    /// §4.3): the gateway generation's, none until
    /// [`SshBridge::with_conn_id_bit`] sets it.
    conn_id_bit: u64,
}

struct CockpitTransport {
    cut: Arc<KillSwitch>,
    /// Serialize loss, including its world notification, with this transport's replacement.
    disconnect: tokio::sync::Mutex<()>,
}

impl<A: AuthBackend + 'static> SshBridge<A> {
    /// Wire the bridge. Connection metrics land in a throwaway registry until
    /// [`SshBridge::with_metrics`] injects the server-wide one.
    pub fn new(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        world: impl mush_ipc::GatewayLink + 'static,
        guard: Arc<IpGuard>,
    ) -> SshBridge<A> {
        Self::with_link(accounts, sessions, Arc::new(world), guard)
    }

    /// Wire the bridge with an explicit [`crate::GatewayLink`] (M16b): the forked
    /// `mush-gateway` passes a socket-client link instead of an in-process
    /// world handle.
    pub fn with_link(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        link: Arc<dyn mush_ipc::GatewayLink>,
        guard: Arc<IpGuard>,
    ) -> SshBridge<A> {
        SshBridge {
            core: GameBridge::with_link(accounts, sessions, link, guard),
            resume: Default::default(),
            cockpit_transports: Default::default(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            pixel: mush_net_ssh::PixelConfig::default(),
            conn_id_bit: 0,
        }
    }

    /// The bit every connection id this bridge's transports allocate carries
    /// (doc 33 §4.3): the gateway generation's
    /// (`mush_config::generation::Generation::conn_id_bit`). The split gateway
    /// sets it from `--generation`; `mushd` and the tests leave it at none.
    #[must_use]
    pub fn with_conn_id_bit(mut self, bit: u64) -> SshBridge<A> {
        self.conn_id_bit = bit;
        self
    }

    /// Count this bridge's attached connections (public and trusted) into
    /// the generation's drain (doc 33 §4.5). The split gateway sets it;
    /// `mushd` and the tests leave the bridge without one.
    #[must_use]
    pub fn with_drain(mut self, drain: Arc<GatewayDrain>) -> SshBridge<A> {
        self.core.set_drain(drain);
        self
    }

    /// Point SSH connection counters at the shared metrics registry (the
    /// same one the web ingress serves over `/metrics`). Called by `mushd`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<mush_net_web::Metrics>) -> SshBridge<A> {
        self.metrics = metrics;
        self
    }

    /// The terminal picture's knobs, from `[pixel]` (doc 21 cuts G.1b, G.2b).
    #[must_use]
    pub fn with_pixel(mut self, cfg: &mush_config::config::PixelCfg) -> SshBridge<A> {
        self.pixel = mush_net_ssh::PixelConfig::from_millis(
            cfg.frame_interval_ms,
            cfg.low_interval_ms,
            cfg.step_down_after,
            cfg.step_up_after_s,
            cfg.diagnostics,
        )
        .with_sixel_max_scale(cfg.sixel_max_scale);
        self
    }

    /// Check account credentials (welcome-screen `connect`, SSH password
    /// auth for `<account>@host`). A blocked address is denied outright.
    pub async fn auth_account(&self, account: &str, password: &str, addr: IpAddr) -> AuthResult {
        self.core.auth_account(account, password, addr).await
    }

    /// Create an account plus its first character (`new@host` wizard,
    /// welcome-screen `create`).
    pub async fn create_account(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
        addr: IpAddr,
    ) -> Result<CharSummary, CreateErr> {
        self.core
            .create_account(account, password, char_name, addr)
            .await
    }

    /// Create an additional character on an authenticated account (the
    /// character-select screen's `newchar`). Cap and ack gating live in the
    /// backend.
    pub async fn create_character(
        &self,
        account: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        self.core.create_character(account, char_name).await
    }

    /// Attach an authenticated connection to a character and bring it into
    /// the game (announcements, initial room render).
    pub async fn connect_character(
        &self,
        conn: NewConn,
        account: &str,
        character: u64,
    ) -> Result<(), ConnectErr> {
        self.enter_character(conn, account, character, None)
            .await
            .map(|_| ())
    }

    async fn enter_character(
        &self,
        conn: NewConn,
        account: &str,
        character: u64,
        previous: Option<(mush_session::SessId, ConnId)>,
    ) -> Result<Option<mush_session::ResumeWindow>, ConnectErr> {
        let actor = ObjId(u32::try_from(character).map_err(|_| ConnectErr::NotYourCharacter)?);
        let char_name = self
            .owned_character(account, character)
            .await
            .ok_or(ConnectErr::NotYourCharacter)?;
        let caps_structured = conn.caps.structured;
        let conn_id = conn.id;
        let handle = match conn.stamped_tx {
            Some(tx) => ConnectionHandle::new_with_sink_at(
                conn_id,
                conn.addr,
                conn.caps,
                mush_session::Sink::AnsiStamped(tx),
                std::time::Instant::now(),
            ),
            None => ConnectionHandle::new(conn_id, conn.addr, conn.caps, conn.tx),
        };
        let (_, window) = self
            .core
            .enter_game_with_resume(
                handle,
                actor,
                account,
                &char_name,
                caps_structured,
                previous,
            )
            .await?;
        // Doc 33 §4.6: attached, so one of the generation's live connections,
        // closed at a drain's deadline by the `Disconnect` its pump ends on.
        self.core.watch_connection(conn_id, CloseWay::Disconnect);
        Ok(window)
    }

    async fn owned_character(&self, account: &str, character: u64) -> Option<String> {
        self.core
            .accounts
            .characters(account)
            .await
            .into_iter()
            .find(|entry| u64::from(entry.id.0) == character)
            .map(|entry| entry.name)
    }

    fn cockpit_issue(
        &self,
        conn: ConnId,
        pane: &str,
        account: &str,
        character: u64,
    ) -> Result<(Ticket, u32), Failure> {
        let session = self
            .core
            .sessions
            .exclusive_session(conn, mush_session::CharKey(character), account)
            .ok_or(Failure::Denied)?;
        let grace = self.core.sessions.config().linkdead_grace;
        self.resume
            .issue(conn.0, pane, account, character, session, grace)
    }

    fn cockpit_transport(&self, conn: ConnId, cut: Arc<KillSwitch>) {
        let mut transports = locked(&self.cockpit_transports, "cockpit SSH transports");
        if self.core.sessions.session_of_conn(conn).is_some() {
            transports.entry(conn).or_insert_with(|| {
                Arc::new(CockpitTransport {
                    cut,
                    disconnect: Default::default(),
                })
            });
        }
    }

    fn transport(&self, conn: ConnId) -> Option<Arc<CockpitTransport>> {
        locked(&self.cockpit_transports, "cockpit SSH transports")
            .get(&conn)
            .cloned()
    }

    async fn cockpit_redeem(
        &self,
        conn: NewConn,
        pane: &str,
        account: &str,
        selected: Option<u64>,
        ticket: &Ticket,
    ) -> Result<Redeemed, Failure> {
        let id = conn.id;
        if let Some(owner @ (old, character, session)) = self
            .resume
            .attached_owner(pane, account, selected, ticket)?
            && let Some(transport) = self.transport(ConnId(old))
        {
            let old_conn = ConnId(old);
            let _closing = transport.disconnect.lock().await;
            if self.owned_character(account, character).await.is_none() {
                return Err(Failure::Denied);
            }
            // Revalidate after both awaits. A concurrent takeover, logout, or normal
            // login may have changed the owner since this request arrived.
            if self
                .resume
                .attached_owner(pane, account, selected, ticket)?
                == Some(owner)
            {
                if old_conn == id
                    || self.core.sessions.exclusive_session(
                        old_conn,
                        mush_session::CharKey(character),
                        account,
                    ) != Some(session)
                {
                    return Err(Failure::Busy);
                }
                transport.cut.kill();
                self.disconnect_ssh(old_conn).await;
            }
        }
        let claim = self.resume.reserve(id.0, pane, account, selected, ticket)?;
        let character = claim.character();
        let previous = (claim.session(), ConnId(claim.previous_conn()));
        let view = claim.view();
        if let Some(transport) = self.transport(previous.1) {
            // Natural loss has already parked the lease, but its ConnDown can
            // still be blocked on the world link. It must precede the new ConnUp.
            drop(transport.disconnect.lock().await);
        }
        let window = self
            .enter_character(conn, account, character, Some(previous))
            .await
            .map_err(|error| match error {
                ConnectErr::NotYourCharacter => Failure::Denied,
                ConnectErr::ResumeExpired => Failure::Expired,
                ConnectErr::NotAdmitted => Failure::Unavailable,
                ConnectErr::Busy => Failure::Busy,
                _ => Failure::Unavailable,
            })?;
        let (rotated, grace) = match claim.commit() {
            Ok(grant) => grant,
            Err(failure) => {
                self.core.disconnected(id).await;
                if let Some(window) = window {
                    self.core.restore_resume_window(id, window);
                }
                return Err(failure);
            }
        };
        Ok((character, rotated, grace, view))
    }

    /// Bind an authenticated connection to a character for one exec request
    /// (doc 44 unit MW.24): the same ownership check as
    /// [`connect_character`](Self::connect_character), then a **bare
    /// connection** ([`GameBridge::enter_bare`]) in place of the attach. The
    /// character is not logged in by it: nobody is told they connected, no
    /// session is made or revived, and when it ends nothing goes link-dead.
    pub async fn connect_exec(
        &self,
        conn: NewConn,
        account: &str,
        character: u64,
    ) -> Result<(), ConnectErr> {
        let actor = ObjId(u32::try_from(character).map_err(|_| ConnectErr::NotYourCharacter)?);
        let characters = self.core.accounts.characters(account).await;
        if !characters.iter().any(|c| c.id == actor) {
            return Err(ConnectErr::NotYourCharacter);
        }
        let conn_id = conn.id;
        // An exec's pump reads plain events and nothing else, so its sink is
        // the plain one whatever the transport offered.
        let handle = ConnectionHandle::new(conn_id, conn.addr, conn.caps, conn.tx);
        self.core.enter_bare(handle, actor).await?;
        // Doc 33 §4.6: a drain waits for an exec in flight like any other
        // connection, and closes it at the deadline by the `Disconnect` its
        // pump ends on.
        self.core.watch_connection(conn_id, CloseWay::Disconnect);
        Ok(())
    }

    /// One input line from an attached connection.
    pub async fn line(&self, conn: ConnId, line: &str) -> LineOutcome {
        self.core.line(conn, line).await
    }

    /// The transport lost (or closed) a connection.
    pub async fn disconnected(&self, conn: ConnId) {
        let transport = self.transport(conn);
        let _closing = match &transport {
            Some(transport) => Some(transport.disconnect.lock().await),
            None => None,
        };
        self.disconnect_ssh(conn).await;
    }

    async fn disconnect_ssh(&self, conn: ConnId) {
        self.core
            .disconnect_with(conn, |outcome| {
                if let Some(outcome) = outcome {
                    self.resume.detach(
                        conn.0,
                        matches!(outcome, mush_session::DetachOutcome::Linkdead { .. }),
                    );
                } else {
                    self.resume.close_attached(conn.0);
                }
            })
            .await;
        locked(&self.cockpit_transports, "cockpit SSH transports").remove(&conn);
    }

    /// Terminal resize / capability renegotiation.
    pub fn caps_changed(&self, conn: ConnId, caps: ClientCaps) {
        self.core.sessions.update_caps(conn, caps);
    }

    /// Auth gate: is the address currently tempbanned? (The per-IP
    /// connection cap is separate — `connection_opened` at accept time.)
    #[must_use]
    pub fn is_ip_blocked(&self, addr: IpAddr) -> bool {
        self.core.is_ip_blocked(addr)
    }

    /// Record one failed auth attempt against an IP.
    pub fn auth_failed(&self, addr: IpAddr) {
        self.core.auth_failed(addr);
    }

    /// Why the address is refused, so the transport can put it in the
    /// authentication banner. The siteban is checked first: it is the
    /// stronger fact, and it carries no countdown to promise.
    #[must_use]
    pub fn block_reason(&self, addr: IpAddr) -> Option<mush_net_ssh::BlockReason> {
        if self.core.siteban_active(addr) {
            return Some(mush_net_ssh::BlockReason::Siteban);
        }
        self.core
            .tempban_remaining(addr)
            .map(|remaining| mush_net_ssh::BlockReason::TooManyFailures { remaining })
    }
}

// ---------------------------------------------------------------------------
// The real transport wiring.
// ---------------------------------------------------------------------------

mod net {
    use futures::future::BoxFuture;
    use mush_config::ipguard;
    use mush_net_ssh as net_ssh;

    use super::*;

    fn map_create_err(err: CreateErr) -> net_ssh::CreateErr {
        match err {
            CreateErr::AccountExists | CreateErr::CharacterNameTaken => net_ssh::CreateErr::Taken,
            CreateErr::BadPassword => net_ssh::CreateErr::Weak,
            other => net_ssh::CreateErr::BadName(other.to_string()),
        }
    }

    /// Adapt the bridge to the transport's dyn-compatible hooks trait.
    ///
    /// `create_account` carries the peer address and delegates to per-IP
    /// [`GameBridge::create_account`] (with `exempt_nets` skipping only the
    /// per-network registration throttle so local bot/load harnesses continue
    /// to work, never sitebans or tempbans); `auth_account` uses
    /// [`AuthBackend::verify_checked`] so Argon2id saturation surfaces as
    /// [`net_ssh::AuthResult::Busy`] without penalizing the peer IP; IP gating
    /// on auth rides [`SshBridge::is_ip_blocked`], checked by the transport
    /// before every SSH-level and in-band welcome attempt, plus the
    /// per-connection pre-auth flood bucket inside `mush-net-ssh`. The per-IP
    /// concurrency cap rides `connection_opened`/`connection_closed` and
    /// `admit_connection`/`release_connection` at accept/drop.
    impl<A: AuthBackend + 'static> net_ssh::SshHooks for SshBridge<A> {
        fn auth_account<'a>(
            &'a self,
            account: &'a str,
            password: &'a str,
        ) -> BoxFuture<'a, net_ssh::AuthResult> {
            Box::pin(async move {
                match self.core.accounts.verify_checked(account, password).await {
                    AuthCheckOutcome::Ok {
                        account,
                        characters,
                    } => net_ssh::AuthResult::Ok {
                        account,
                        characters: characters
                            .into_iter()
                            .map(|c| net_ssh::CharSummary {
                                key: u64::from(c.id.0),
                                name: c.name,
                            })
                            .collect(),
                    },
                    AuthCheckOutcome::BadCredentials => net_ssh::AuthResult::BadCreds,
                    AuthCheckOutcome::Busy => net_ssh::AuthResult::Busy,
                }
            })
        }

        fn auth_account_key<'a>(
            &'a self,
            account: &'a str,
            openssh_key: &'a str,
        ) -> BoxFuture<'a, net_ssh::AuthResult> {
            Box::pin(async move {
                match self.core.accounts.verify_pubkey(account, openssh_key).await {
                    AuthOutcome::Ok {
                        account,
                        characters,
                    } => net_ssh::AuthResult::Ok {
                        account,
                        characters: characters
                            .into_iter()
                            .map(|c| net_ssh::CharSummary {
                                key: u64::from(c.id.0),
                                name: c.name,
                            })
                            .collect(),
                    },
                    AuthOutcome::BadCredentials => net_ssh::AuthResult::BadCreds,
                }
            })
        }

        fn create_account<'a>(
            &'a self,
            account: &'a str,
            password: &'a str,
            addr: IpAddr,
        ) -> BoxFuture<'a, Result<(), net_ssh::CreateErr>> {
            Box::pin(async move {
                // The account's first character shares the account's name
                // (as typed); more arrive via the select screen's newchar.
                if self.core.guard.is_exempt(addr) {
                    if self.core.guard.is_blocked(addr) {
                        return Err(net_ssh::CreateErr::BadName(
                            "your address is blocked".to_string(),
                        ));
                    }
                    return self
                        .core
                        .accounts
                        .create(account, password, account)
                        .await
                        .map(|_| ())
                        .map_err(map_create_err);
                }
                self.core
                    .create_account(account, password, account, addr)
                    .await
                    .map(|_| ())
                    .map_err(map_create_err)
            })
        }

        fn create_character<'a>(
            &'a self,
            account: &'a str,
            name: &'a str,
        ) -> BoxFuture<'a, Result<net_ssh::CharSummary, String>> {
            Box::pin(async move {
                match self.create_character(account, name).await {
                    Ok(c) => Ok(net_ssh::CharSummary {
                        key: c.id,
                        name: c.name,
                    }),
                    Err(e) => Err(format!("{e}.")),
                }
            })
        }

        fn connect_character<'a>(
            &'a self,
            conn: net_ssh::NewConn,
            account: &'a str,
            character: u64,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.connect_character(conn.into(), account, character)
                    .await
                    .map_err(|e| e.to_string())
            })
        }

        fn cockpit_issue<'a>(
            &'a self,
            conn: u64,
            pane: &'a str,
            account: &'a str,
            character: u64,
        ) -> BoxFuture<'a, Result<(net_ssh::resume::Ticket, u32), net_ssh::resume::Failure>>
        {
            Box::pin(async move { self.cockpit_issue(ConnId(conn), pane, account, character) })
        }

        fn cockpit_redeem<'a>(
            &'a self,
            conn: net_ssh::NewConn,
            pane: &'a str,
            account: &'a str,
            selected: Option<u64>,
            ticket: &'a net_ssh::resume::Ticket,
        ) -> BoxFuture<'a, Result<Redeemed, Failure>> {
            Box::pin(async move {
                self.cockpit_redeem(conn.into(), pane, account, selected, ticket)
                    .await
            })
        }

        fn cockpit_transport(&self, conn: u64, cut: Arc<KillSwitch>) {
            self.cockpit_transport(ConnId(conn), cut);
        }

        fn cockpit_view(&self, conn: u64, account: &str, pane: &str, view: ViewPrefs) {
            self.resume.update_view(conn, account, pane, view);
        }

        fn cockpit_close(&self, conn: u64) {
            self.resume.close_attached(conn);
        }

        fn connect_exec<'a>(
            &'a self,
            conn: net_ssh::NewConn,
            account: &'a str,
            character: u64,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.connect_exec(conn.into(), account, character)
                    .await
                    .map_err(|e| e.to_string())
            })
        }

        fn begin_local_reply(&self, conn: u64) -> BoxFuture<'_, mush_net_ssh::LocalReply> {
            Box::pin(async move {
                let Some(handle) = self.core.sessions.conn_handle(ConnId(conn)) else {
                    return mush_net_ssh::LocalReply::default();
                };
                let guard = handle.owned_reply_lock().await;
                mush_net_ssh::LocalReply::with_guard(handle.output_brackets(), guard)
            })
        }

        fn control_line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                self.core.control_line(ConnId(conn), &line).await;
            })
        }

        fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                // On `Close` (QUIT) the detach already dropped the output
                // sender; the transport's pump closes the socket.
                let _ = self.line(ConnId(conn), &line).await;
            })
        }

        /// The SSH TUI's read-only query seam (milestone M15): the transport
        /// asks for a widget's data by name and the reply lands on the same
        /// output queue as an [`OutputEvent::Reply`] — the mirror of the web
        /// bridge's `Req` → `Inbound::Query` path. Any well-formed name
        /// forwards with the caller's args (interface plan I1, D-I-3): the
        /// engine answers with that handler's keyframe, a push-only feed's
        /// ack, or "Unknown query", and never stamps idle for a query, so a
        /// dynamic name is safe; the shape check only keeps garbage off the
        /// world loop. The one legacy convenience is `mail.list` with no
        /// args, which lists the inbox. Preserve the transport correlation id:
        /// history uses it to reject late replies after a retry or mode change.
        fn invoke(
            &self,
            conn: u64,
            id: u64,
            request: mush_proto::InvokeRequest,
        ) -> BoxFuture<'_, ()> {
            Box::pin(async move { self.core.invoke(conn, id, request).await })
        }

        fn query(
            &self,
            conn: u64,
            id: u64,
            name: String,
            args: serde_json::Value,
        ) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                let well_formed = !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.'
                    });
                if !well_formed {
                    tracing::debug!(conn, name, "malformed TUI query name dropped");
                    return;
                }
                let args_given = args.as_object().is_some_and(|o| !o.is_empty());
                let args = if name == "mail.list" && !args_given {
                    serde_json::json!({ "folder": null })
                } else {
                    args
                };
                self.core.query(conn, id, SmolStr::new(name), args).await;
            })
        }

        fn respond(
            &self,
            conn: u64,
            id: u64,
            epoch: String,
            choice: RespondChoice,
        ) -> BoxFuture<'_, ()> {
            Box::pin(async move { self.core.respond(conn, id, epoch, choice).await })
        }

        fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()> {
            Box::pin(async move { self.disconnected(ConnId(conn)).await })
        }

        fn is_ip_blocked(&self, addr: IpAddr) -> bool {
            self.core.is_ip_blocked(addr)
        }

        fn auth_failed(&self, addr: IpAddr) {
            self.core.auth_failed(addr);
        }

        fn block_reason(&self, addr: IpAddr) -> Option<net_ssh::BlockReason> {
            SshBridge::block_reason(self, addr)
        }

        fn connection_opened(&self, addr: IpAddr) -> bool {
            let admitted = self.core.guard.conn_opened(addr);
            // Count only admitted connections so the current-gauge stays
            // balanced with connection_closed (only called for admitted ones).
            if admitted {
                self.metrics.ssh_connection_opened();
            }
            admitted
        }

        fn connection_closed(&self, addr: IpAddr) {
            self.core.guard.conn_closed(addr);
            self.metrics.ssh_connection_closed();
        }

        fn conn_id_bit(&self) -> u64 {
            self.conn_id_bit
        }

        // Doc 29a §9: the public port's accept gates — the guard's rate,
        // global and /48 caps and its exempt networks, answered before any
        // handshake. The gauge counts admitted connections only, as above.
        fn admit_connection(&self, addr: IpAddr) -> net_ssh::Admission {
            match self.core.guard.admit(addr) {
                ipguard::Admission::Admit => {
                    self.metrics.ssh_connection_opened();
                    net_ssh::Admission::Admit
                }
                ipguard::Admission::OverCap => net_ssh::Admission::OverCap,
                ipguard::Admission::Drop(reason) => {
                    tracing::debug!(%addr, ?reason, "SSH connection dropped at accept");
                    net_ssh::Admission::Drop
                }
            }
        }

        fn release_connection(&self, addr: IpAddr, admission: net_ssh::Admission) {
            let held = match admission {
                net_ssh::Admission::Admit => ipguard::Admission::Admit,
                net_ssh::Admission::OverCap => ipguard::Admission::OverCap,
                net_ssh::Admission::Drop => return,
            };
            self.core.guard.release(addr, held);
            if held == ipguard::Admission::Admit {
                self.metrics.ssh_connection_closed();
            }
        }

        fn caps_changed(&self, conn: u64, caps: ClientCaps) {
            self.caps_changed(ConnId(conn), caps);
        }

        fn pixel_config(&self) -> net_ssh::PixelConfig {
            self.pixel
        }

        fn pixel_event(&self, event: net_ssh::PixelEvent) {
            match event {
                net_ssh::PixelEvent::Opened => self.metrics.pixel_view(true),
                net_ssh::PixelEvent::Closed => self.metrics.pixel_view(false),
                net_ssh::PixelEvent::Sent(bytes) => self.metrics.pixel_frame_sent(bytes),
                net_ssh::PixelEvent::SlowSkip => self.metrics.pixel_slow_skip(),
                net_ssh::PixelEvent::SteppedDown => self.metrics.pixel_stepped_down(),
            }
        }
    }
}

/// The running SSH listener: its bound address plus the stop switch the
/// shutdown sequence uses ("stop accepting" comes before the announce in
/// the PLAN order). Dropping the handle leaves the listener running
/// (detached), which is what tests want.
pub struct SshListener {
    addr: std::net::SocketAddr,
    accept_loop: tokio::task::JoinHandle<()>,
    /// Doc 29: the trusted ingress's accept loop, when one is bound.
    trusted_loop: Option<tokio::task::JoinHandle<()>>,
    /// Whether [`SshListener::stop_accepting`] has run (it is asked twice on
    /// a drain: by the lifecycle, then by the shutdown tail).
    stopped: std::sync::atomic::AtomicBool,
}

impl SshListener {
    /// The actual bound address (resolves an ephemeral port).
    #[must_use]
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// Stop accepting new connections. Established sessions are untouched —
    /// they belong to their own per-connection tasks.
    pub fn stop_accepting(&self) {
        if self.stopped.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.accept_loop.abort();
        if let Some(trusted) = &self.trusted_loop {
            trusted.abort();
        }
        tracing::info!("SSH listener stopped accepting");
    }

    /// Doc 29: also serve the **trusted ingress** at `socket_path` — the
    /// Unix socket sshdoorway connects to, whose connections carry the
    /// player's real address and an attested key in a preamble before SSH.
    /// Same host key, banner and hooks as the public port. Unix only.
    pub async fn start_trusted<A: AuthBackend + 'static>(
        &mut self,
        socket_path: std::path::PathBuf,
        data_dir: &std::path::Path,
        banner: String,
        bridge: Arc<SshBridge<A>>,
    ) -> Result<(), String> {
        self.start_trusted_with(socket_path, data_dir, banner, bridge, false)
            .await
    }

    /// [`start_trusted`](SshListener::start_trusted) with the trusted
    /// ingress's `allow_own_group` knob (`TrustedConfig::allow_own_group`):
    /// `true` serves from a directory that is not setgid, which admits every
    /// process of the gateway's own user. **Tests only**; production passes
    /// `false`.
    pub async fn start_trusted_with<A: AuthBackend + 'static>(
        &mut self,
        socket_path: std::path::PathBuf,
        data_dir: &std::path::Path,
        banner: String,
        bridge: Arc<SshBridge<A>>,
        allow_own_group: bool,
    ) -> Result<(), String> {
        #[cfg(unix)]
        {
            let cfg = mush_net_ssh::TrustedConfig {
                socket_path,
                host_key_path: data_dir.join("ssh_host_ed25519"),
                banner,
                allow_own_group,
            };
            let hooks: Arc<dyn mush_net_ssh::SshHooks> = bridge;
            let bound = mush_net_ssh::SshServer::bind_trusted(cfg, hooks)
                .await
                .map_err(|e| e.to_string())?;
            self.trusted_loop = Some(tokio::spawn(async move {
                if let Err(e) = bound.run().await {
                    tracing::error!(error = %e, "SSH trusted ingress accept loop failed");
                }
            }));
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (data_dir, banner, bridge, allow_own_group);
            Err(format!(
                "[ssh] trusted_socket = {} needs a Unix platform (sshdoorway is Linux-only)",
                socket_path.display()
            ))
        }
    }
}

/// Bind the SSH listener and start serving on a background task, with the
/// default accept-time deadline ([`mush_net_ssh::DEFAULT_AUTH_DEADLINE`]).
/// The gateway proper reads its deadline from `[ssh]` and calls
/// [`start_listener_with`]; the all-in-one and the test fixtures take the
/// default.
pub async fn start_listener<A: AuthBackend + 'static>(
    listen: std::net::SocketAddr,
    data_dir: &std::path::Path,
    banner: String,
    bridge: Arc<SshBridge<A>>,
) -> Result<SshListener, String> {
    start_listener_with(
        listen,
        data_dir,
        banner,
        mush_net_ssh::DEFAULT_AUTH_DEADLINE,
        bridge,
    )
    .await
}

/// [`start_listener`] with an explicit `[ssh].auth_deadline_s` (doc 29a
/// §9): how long an accepted connection has to authenticate, and then to
/// open a channel, before it is cut.
pub async fn start_listener_with<A: AuthBackend + 'static>(
    listen: std::net::SocketAddr,
    data_dir: &std::path::Path,
    banner: String,
    auth_deadline: std::time::Duration,
    bridge: Arc<SshBridge<A>>,
) -> Result<SshListener, String> {
    let listener =
        std::net::TcpListener::bind(listen).map_err(|e| format!("binding {listen}: {e}"))?;
    start_listener_on(listener, data_dir, banner, auth_deadline, bridge).await
}

/// [`start_listener_with`] on a listener the caller bound (doc 33 §4.4): the
/// split gateway binds its public port through `mush_drain::listen::bind_std`
/// (`SO_REUSEPORT`, so the two generations of a swap share it) and hands it
/// here; `mushd` and the fixtures bind plainly through the functions above.
pub async fn start_listener_on<A: AuthBackend + 'static>(
    listener: std::net::TcpListener,
    data_dir: &std::path::Path,
    banner: String,
    auth_deadline: std::time::Duration,
    bridge: Arc<SshBridge<A>>,
) -> Result<SshListener, String> {
    let listen = listener
        .local_addr()
        .map_err(|e| format!("the bound SSH listener has no address: {e}"))?;
    let cfg = mush_net_ssh::SshConfig {
        listen,
        host_key_path: data_dir.join("ssh_host_ed25519"),
        banner,
        auth_deadline,
    };
    let hooks: Arc<dyn mush_net_ssh::SshHooks> = bridge;
    let bound =
        mush_net_ssh::SshServer::bind_listener(cfg, listener, hooks).map_err(|e| e.to_string())?;
    let addr = bound.local_addr();
    // The loop survives accept errors on its own (doc 29a §8 B-2); it only
    // ends with this task.
    let accept_loop = tokio::spawn(async move {
        if let Err(e) = bound.run().await {
            tracing::error!(error = %e, "SSH accept loop failed");
        }
    });
    Ok(SshListener {
        addr,
        accept_loop,
        trusted_loop: None,
        stopped: std::sync::atomic::AtomicBool::new(false),
    })
}

/// Doc 44 unit MW.24: the SSH `mind` exec's connection as the gateway makes
/// it, against a link that records every frame the bridge sends the server.
#[cfg(test)]
mod exec_tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use futures::future::BoxFuture;
    use mush_ipc::{GatewayLink, ServiceReq, ServiceResp, ToServer, WireInbound};
    use mush_proto::{ClientKind, ColorDepth};
    use mush_session::{CharKey, DetachOutcome, DetachReason};

    use super::*;
    use crate::bridge_common::ConnectErr;

    const PILOT: ObjId = ObjId(7);

    struct OnePilot {
        owns_character: AtomicBool,
        busy: AtomicBool,
    }

    impl AuthBackend for OnePilot {
        async fn verify(&self, _account: &str, _password: &str) -> AuthOutcome {
            AuthOutcome::BadCredentials
        }

        async fn verify_checked(&self, _account: &str, _password: &str) -> AuthCheckOutcome {
            if self.busy.load(Ordering::SeqCst) {
                AuthCheckOutcome::Busy
            } else {
                AuthCheckOutcome::BadCredentials
            }
        }

        async fn characters(&self, _account: &str) -> Vec<crate::accounts::CharSummary> {
            if !self.owns_character.load(Ordering::SeqCst) {
                return Vec::new();
            }
            vec![crate::accounts::CharSummary {
                id: PILOT,
                name: "Pilot".to_string(),
            }]
        }

        async fn create(
            &self,
            _account: &str,
            _password: &str,
            _char_name: &str,
        ) -> Result<crate::accounts::CharSummary, CreateErr> {
            Err(CreateErr::Io("unused test backend".to_string()))
        }

        async fn create_character(
            &self,
            _account: &str,
            _char_name: &str,
        ) -> Result<crate::accounts::CharSummary, CreateErr> {
            Err(CreateErr::Io("unused test backend".to_string()))
        }
    }

    /// What the server was sent, and whether it says the connection is bound.
    struct Seen {
        frames: Mutex<Vec<ToServer>>,
        binds: AtomicBool,
        stall_next_down: Mutex<
            Option<(
                tokio::sync::oneshot::Sender<()>,
                tokio::sync::oneshot::Receiver<()>,
            )>,
        >,
    }

    struct Recording(Arc<Seen>);

    impl GatewayLink for Recording {
        fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                if matches!(msg, ToServer::Inbound(WireInbound::ConnDown { .. })) {
                    let stall = self.0.stall_next_down.lock().unwrap().take();
                    if let Some((entered, release)) = stall {
                        entered.send(()).unwrap();
                        release.await.unwrap();
                    }
                }
                self.0.frames.lock().unwrap().push(msg);
                true
            })
        }

        fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            let resp = match req {
                ServiceReq::Barrier => Some(ServiceResp::Barrier),
                ServiceReq::ConnectionReady { .. } => Some(ServiceResp::ConnectionReady(
                    self.0.binds.load(Ordering::SeqCst),
                )),
                _ => None,
            };
            Box::pin(async move { resp })
        }
    }

    type Rig = (SshBridge<OnePilot>, Arc<SessionManager>, Arc<Seen>);

    fn rig(binds: bool) -> Rig {
        let sessions = Arc::new(SessionManager::new());
        let seen = Arc::new(Seen {
            frames: Mutex::new(Vec::new()),
            binds: AtomicBool::new(binds),
            stall_next_down: Default::default(),
        });
        let bridge = SshBridge::new(
            Arc::new(OnePilot {
                owns_character: AtomicBool::new(true),
                busy: AtomicBool::new(false),
            }),
            Arc::clone(&sessions),
            Recording(Arc::clone(&seen)),
            Arc::new(IpGuard::new()),
        );
        (bridge, sessions, seen)
    }

    fn exec_caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi256,
            width: 120,
            height: 40,
            utf8: true,
            structured: true,
        }
    }

    #[tokio::test]
    async fn cockpit_ticket_rebinds_only_the_dropped_session_after_world_admission() {
        use mush_net_ssh::resume::Failure;
        const PANE: &str = "00112233445566778899aabbccddeeff";
        let (bridge, sessions, seen) = rig(true);
        let character = u64::from(PILOT.0);
        let (origin, _original_output) = exec_conn(45);
        bridge
            .connect_character(origin, "pilot", character)
            .await
            .unwrap();
        let session = sessions.session_of_conn(ConnId(45)).unwrap();
        sessions.send_event_to_session(
            session,
            OutputEvent::Sys(mush_proto::RichText::plain("old one-shot")),
        );
        let (ticket, _) = bridge
            .cockpit_issue(ConnId(45), PANE, "pilot", character)
            .unwrap();
        let redeem = |id, account, selected| {
            let (conn, output) = exec_conn(id);
            (
                bridge.cockpit_redeem(conn, PANE, account, selected, &ticket),
                output,
            )
        };
        assert_eq!(
            redeem(46, "pilot", Some(character)).0.await.unwrap_err(),
            Failure::Busy
        );
        bridge.disconnected(ConnId(45)).await;
        assert_eq!(
            redeem(46, "other", Some(character)).0.await.unwrap_err(),
            Failure::Denied
        );
        assert_eq!(
            redeem(46, "pilot", Some(8)).0.await.unwrap_err(),
            Failure::Denied
        );
        // Auth still names the same account and character. Removing that
        // character from the live roster after issue must revoke admission.
        bridge
            .core
            .accounts
            .owns_character
            .store(false, Ordering::SeqCst);
        assert_eq!(
            redeem(46, "pilot", Some(character)).0.await.unwrap_err(),
            Failure::Denied
        );
        assert!(sessions.is_linkdead(session));
        bridge
            .core
            .accounts
            .owns_character
            .store(true, Ordering::SeqCst);
        seen.binds.store(false, Ordering::SeqCst);
        assert_eq!(
            redeem(46, "pilot", None).0.await.unwrap_err(),
            Failure::Unavailable
        );
        assert!(
            sessions.is_linkdead(session),
            "rejected world binding restores the old drop"
        );
        seen.binds.store(true, Ordering::SeqCst);
        let (result, mut output) = redeem(47, "pilot", None);
        assert_eq!(result.await.unwrap().0, character);
        assert_eq!(sessions.session_of_conn(ConnId(47)), Some(session));
        assert!(matches!(
            &*output.try_recv().unwrap(),
            OutputEvent::SessionInfo { .. }
        ));
        assert!(
            output.try_recv().is_err(),
            "previous session effects are not replayed"
        );
        assert!(seen.frames.lock().unwrap().iter().any(|frame| matches!(frame, ToServer::Inbound(WireInbound::ConnUp {conn:47,actor,from_linkdead:true,..}) if *actor == PILOT)));
        assert!(
            redeem(48, "pilot", None).0.await.is_err(),
            "old ticket cannot acquire a second transport"
        );
    }

    #[tokio::test]
    async fn cockpit_ticket_fences_only_its_authorized_live_transport() {
        let pane = "00112233445566778899aabbccddeeff";
        let (bridge, sessions, seen) = rig(true);
        let character = u64::from(PILOT.0);
        let (original, _rx) = exec_conn(49);
        bridge
            .connect_character(original, "pilot", character)
            .await
            .unwrap();
        let original_session = sessions.session_of_conn(ConnId(49)).unwrap();
        let (ticket, grace) = bridge
            .cockpit_issue(ConnId(49), pane, "pilot", character)
            .unwrap();
        let cut = KillSwitch::new();
        bridge.cockpit_transport(ConnId(49), cut.clone());
        let (wrong_account, _rx2) = exec_conn(50);
        assert_eq!(
            bridge
                .cockpit_redeem(wrong_account, pane, "other", None, &ticket)
                .await
                .unwrap_err(),
            Failure::Denied
        );
        bridge
            .core
            .accounts
            .owns_character
            .store(false, Ordering::SeqCst);
        let (revoked, _rx2) = exec_conn(50);
        assert_eq!(
            bridge
                .cockpit_redeem(revoked, pane, "pilot", None, &ticket)
                .await
                .unwrap_err(),
            Failure::Denied
        );
        assert!(!cut.is_killed());
        assert_eq!(sessions.session_of_conn(ConnId(49)), Some(original_session));
        bridge
            .core
            .accounts
            .owns_character
            .store(true, Ordering::SeqCst);

        let (second, _another_rx) = exec_conn(52);
        bridge
            .connect_character(second, "pilot", character)
            .await
            .unwrap();
        let (ambiguous, _rx2) = exec_conn(50);
        assert_eq!(
            bridge
                .cockpit_redeem(ambiguous, pane, "pilot", None, &ticket)
                .await
                .unwrap_err(),
            Failure::Busy
        );
        assert!(!cut.is_killed());
        assert_eq!(sessions.session_of_conn(ConnId(52)), Some(original_session));
        bridge.disconnected(ConnId(52)).await;

        let (replacement, _rx2) = exec_conn(50);
        let (resumed, _, rotated_grace, _) = bridge
            .cockpit_redeem(replacement, pane, "pilot", Some(character), &ticket)
            .await
            .unwrap();
        assert_eq!(resumed, character);
        assert_eq!(grace, rotated_grace);
        assert!(cut.is_killed());
        assert!(sessions.conn_handle(ConnId(49)).is_none());
        assert_eq!(sessions.session_of_conn(ConnId(50)), Some(original_session));
        let before = frames(&seen);
        let loss = before
            .iter()
            .position(|f| matches!(f, ToServer::Inbound(WireInbound::ConnDown { conn: 49, .. })))
            .unwrap();
        let attach = before
            .iter()
            .position(|f| {
                matches!(
                    f,
                    ToServer::Inbound(WireInbound::ConnUp {
                        conn: 50,
                        from_linkdead: true,
                        ..
                    })
                )
            })
            .unwrap();
        assert!(loss < attach);
        bridge.line(ConnId(49), "look").await;
        bridge.disconnected(ConnId(49)).await;
        assert_eq!(frames(&seen), before);

        // The rotated reply could be lost. Its predecessor can recover only
        // after a real drop, and cannot fence the healthy new owner.
        let next_cut = KillSwitch::new();
        bridge.cockpit_transport(ConnId(50), next_cut.clone());
        let (stale, _rx3) = exec_conn(51);
        assert!(
            bridge
                .cockpit_redeem(stale, pane, "pilot", None, &ticket)
                .await
                .is_err()
        );
        assert!(!next_cut.is_killed());
        bridge.disconnected(ConnId(50)).await;
        bridge.disconnected(ConnId(50)).await;
        let (retry, _rx3) = exec_conn(51);
        assert!(
            bridge
                .cockpit_redeem(retry, pane, "pilot", None, &ticket)
                .await
                .is_ok()
        );
        assert_eq!(sessions.session_of_conn(ConnId(51)), Some(original_session));
    }

    #[tokio::test(start_paused = true)]
    async fn cockpit_ticket_parks_before_a_slow_world_loss_and_orders_the_new_attachment() {
        let (bridge, sessions, seen) = rig(true);
        let pane = "00112233445566778899aabbccddeeff";
        let character = u64::from(PILOT.0);
        let (original, _rx) = exec_conn(49);
        bridge
            .connect_character(original, "pilot", character)
            .await
            .unwrap();
        let (ticket, _) = bridge
            .cockpit_issue(ConnId(49), pane, "pilot", character)
            .unwrap();
        bridge.cockpit_transport(ConnId(49), KillSwitch::new());
        let session = sessions.session_of_conn(ConnId(49)).unwrap();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, stalled) = tokio::sync::oneshot::channel();
        *seen.stall_next_down.lock().unwrap() = Some((started, stalled));
        let (replacement, _rx2) = exec_conn(50);
        let resume = async {
            observed.await.unwrap();
            assert!(sessions.is_linkdead(session));
            assert!(
                bridge
                    .resume
                    .attached_owner(pane, "pilot", None, &ticket)
                    .unwrap()
                    .is_none()
            );
            let redeem = bridge.cockpit_redeem(replacement, pane, "pilot", None, &ticket);
            tokio::pin!(redeem);
            // Outwait the former 250ms busy-poll budget with ConnDown blocked.
            assert!(
                tokio::time::timeout(Duration::from_millis(300), &mut redeem)
                    .await
                    .is_err()
            );
            assert!(sessions.is_linkdead(session));
            release.send(()).unwrap();
            redeem.await.unwrap();
        };
        tokio::join!(bridge.disconnected(ConnId(49)), resume);
        assert_eq!(sessions.session_of_conn(ConnId(50)), Some(session));
        let lifecycle: Vec<_> = frames(&seen)
            .into_iter()
            .filter_map(|f| match f {
                ToServer::Inbound(WireInbound::ConnDown { conn: 49, .. }) => Some("lost"),
                ToServer::Inbound(WireInbound::ConnUp {
                    conn: 50,
                    from_linkdead: true,
                    ..
                }) => Some("restored"),
                _ => None,
            })
            .collect();
        assert_eq!(lifecycle, ["lost", "restored"]);
    }

    fn exec_conn(id: u64) -> (NewConn, mpsc::Receiver<Arc<OutputEvent>>) {
        let (tx, rx) = mpsc::channel(8);
        let conn = NewConn {
            id: ConnId(id),
            addr: "127.0.0.1".parse().unwrap(),
            caps: exec_caps(),
            tx,
            stamped_tx: None,
        };
        (conn, rx)
    }

    fn frames(seen: &Seen) -> Vec<ToServer> {
        seen.frames.lock().unwrap().clone()
    }

    fn bare_up(conn: u64) -> ToServer {
        ToServer::Inbound(WireInbound::BareUp { conn, actor: PILOT })
    }

    fn bare_down(conn: u64) -> ToServer {
        ToServer::Inbound(WireInbound::BareDown { conn, actor: PILOT })
    }

    /// The exec of a player who is offline: the server is sent a `BareUp`,
    /// the invoke and a `BareDown`, never a `ConnUp`, a `ConnDown` or a
    /// `SessionGone`; the gateway makes no session for it and leaves none
    /// link-dead behind it.
    #[tokio::test]
    async fn an_offline_exec_is_bound_and_never_logged_in() {
        let (bridge, sessions, seen) = rig(true);
        let (conn, _rx) = exec_conn(5);
        bridge
            .connect_exec(conn, "acct", u64::from(PILOT.0))
            .await
            .expect("the exec is bound");
        assert_eq!(frames(&seen), vec![bare_up(5)]);
        assert_eq!(bridge.core.actor_of(ConnId(5)), Some(PILOT));
        assert_eq!(sessions.session_count(), 0, "no session was made");
        assert!(sessions.who().is_empty(), "WHO has no row for the exec");

        // The one request runs as the character, over that connection.
        let request = mush_proto::InvokeRequest::Mind(mush_proto::MindRequest::List {});
        bridge.core.invoke(5, 1, request.clone()).await;
        assert_eq!(
            frames(&seen).last(),
            Some(&ToServer::Inbound(WireInbound::Invoke {
                conn: 5,
                actor: PILOT,
                id: 1,
                request,
            }))
        );

        bridge.disconnected(ConnId(5)).await;
        assert_eq!(frames(&seen).last(), Some(&bare_down(5)));
        assert_eq!(frames(&seen).len(), 3, "{:?}", frames(&seen));
        assert_eq!(sessions.session_count(), 0, "nobody is left link-dead");
        assert_eq!(sessions.next_linkdead_expiry(), None);
        assert_eq!(bridge.core.actor_of(ConnId(5)), None);
        // A second teardown (the pump and the dropped handler both fire) is
        // nothing at all.
        bridge.disconnected(ConnId(5)).await;
        assert_eq!(frames(&seen).len(), 3);
    }

    /// An exec typed four minutes into a link loss: the session stays
    /// link-dead and its window keeps its start through the whole life of the
    /// exec, and the server is told of no connection and no loss.
    #[tokio::test]
    async fn an_exec_during_a_link_loss_leaves_the_window_alone() {
        let (bridge, sessions, seen) = rig(true);
        let lost_at = Instant::now() - Duration::from_secs(240);
        let (tx, _rx) = mpsc::channel(8);
        let live = ConnectionHandle::new(ConnId(1), "127.0.0.1".parse().unwrap(), exec_caps(), tx);
        let (sess, _) = sessions.attach(live, CharKey(u64::from(PILOT.0)), "acct".to_string());
        assert!(matches!(
            sessions.detach_at(ConnId(1), DetachReason::Lost, lost_at),
            Some(DetachOutcome::Linkdead { .. })
        ));
        let expiry = sessions.next_linkdead_expiry();
        assert_eq!(expiry, Some(lost_at + Duration::from_secs(300)));

        let (conn, _rx) = exec_conn(5);
        bridge
            .connect_exec(conn, "acct", u64::from(PILOT.0))
            .await
            .expect("the exec is bound");
        assert!(sessions.is_linkdead(sess), "the exec revives nobody");
        assert_eq!(sessions.next_linkdead_expiry(), expiry);
        bridge.disconnected(ConnId(5)).await;
        assert!(sessions.is_linkdead(sess));
        assert_eq!(
            sessions.next_linkdead_expiry(),
            expiry,
            "and its end does not start the five minutes again"
        );
        assert_eq!(frames(&seen), vec![bare_up(5), bare_down(5)]);
    }

    /// A server that does not bind the connection (a world mode that takes
    /// none, an id another gateway holds) refuses the exec, and the gateway
    /// undoes what it did.
    #[tokio::test]
    async fn an_exec_the_server_does_not_bind_is_refused_and_undone() {
        let (bridge, sessions, seen) = rig(false);
        let (conn, _rx) = exec_conn(5);
        assert_eq!(
            bridge.connect_exec(conn, "acct", u64::from(PILOT.0)).await,
            Err(ConnectErr::NotAdmitted)
        );
        assert_eq!(frames(&seen), vec![bare_up(5), bare_down(5)]);
        assert!(sessions.conn_handle(ConnId(5)).is_none());
        assert!(sessions.bare_conns().is_empty());
    }

    /// The ownership check is the one `connect_character` makes: a character
    /// the account does not own is refused before anything is bound or sent.
    #[tokio::test]
    async fn an_exec_as_a_character_the_account_does_not_own_is_refused() {
        let (bridge, sessions, seen) = rig(true);
        let (conn, _rx) = exec_conn(5);
        assert_eq!(
            bridge.connect_exec(conn, "acct", 99).await,
            Err(ConnectErr::NotYourCharacter)
        );
        assert!(frames(&seen).is_empty());
        assert!(sessions.bare_conns().is_empty());
    }

    #[tokio::test]
    async fn ssh_hooks_create_account_rate_limits_per_ip_without_cross_user_poisoning() {
        use mush_net_ssh::SshHooks;

        let (bridge, _sessions, _seen) = rig(true);
        let peer_a: IpAddr = "203.0.113.10".parse().unwrap();
        let peer_b: IpAddr = "203.0.113.11".parse().unwrap();
        let v6_net_a1: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let v6_net_a2: IpAddr = "2001:db8:1:2::99".parse().unwrap();
        let v6_net_b: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();

        // First MAX_REGISTERS_PER_WINDOW calls from peer_a reach the backend;
        // the next call from peer_a is throttled before the backend.
        for i in 0..mush_config::ipguard::MAX_REGISTERS_PER_WINDOW {
            let err = SshHooks::create_account(&bridge, &format!("acct{i}"), "secret123", peer_a)
                .await
                .unwrap_err();
            assert!(
                !format!("{err:?}").contains("too many requests"),
                "call {i} from peer_a should not be rate-limited yet: {err:?}"
            );
        }
        let limited_a = SshHooks::create_account(&bridge, "acct_over", "secret123", peer_a)
            .await
            .unwrap_err();
        assert!(
            matches!(limited_a, mush_net_ssh::CreateErr::BadName(ref msg) if msg.contains("too many requests")),
            "expected rate-limit error for peer_a, got {limited_a:?}"
        );

        // Exhausting peer_a's quota MUST NOT throttle an unrelated public peer_b (EXT-P2-1).
        let peer_b_err = SshHooks::create_account(&bridge, "other_user", "secret123", peer_b)
            .await
            .unwrap_err();
        assert!(
            !format!("{peer_b_err:?}").contains("too many requests"),
            "peer_b must have its own registration bucket: {peer_b_err:?}"
        );

        // IPv6 peers in the same /64 share one bucket while another /64 is isolated.
        for i in 0..mush_config::ipguard::MAX_REGISTERS_PER_WINDOW {
            let err = SshHooks::create_account(&bridge, &format!("v6_{i}"), "secret123", v6_net_a1)
                .await
                .unwrap_err();
            assert!(
                !format!("{err:?}").contains("too many requests"),
                "v6 call {i} should not be rate-limited yet: {err:?}"
            );
        }
        let limited_v6 = SshHooks::create_account(&bridge, "v6_over", "secret123", v6_net_a2)
            .await
            .unwrap_err();
        assert!(
            matches!(limited_v6, mush_net_ssh::CreateErr::BadName(ref msg) if msg.contains("too many requests")),
            "same IPv6 /64 must share registration quota: {limited_v6:?}"
        );
        let isolated_v6 = SshHooks::create_account(&bridge, "v6_other", "secret123", v6_net_b)
            .await
            .unwrap_err();
        assert!(
            !format!("{isolated_v6:?}").contains("too many requests"),
            "distinct IPv6 /64 must not be throttled: {isolated_v6:?}"
        );

        // Exempt loopback peers skip the per-IP registration rate throttle
        // (preserving local bot/load harnesses), unless explicitly blocked.
        for i in 0..(mush_config::ipguard::MAX_REGISTERS_PER_WINDOW + 2) {
            let err = SshHooks::create_account(&bridge, &format!("lo_{i}"), "secret123", loopback)
                .await
                .unwrap_err();
            assert!(
                !format!("{err:?}").contains("too many requests"),
                "exempt loopback call {i} should not be rate-limited: {err:?}"
            );
        }
        for _ in 0..mush_config::ipguard::IpGuardConfig::default().max_failures {
            bridge.auth_failed(loopback);
        }
        let blocked_lo = SshHooks::create_account(&bridge, "lo_blocked", "secret123", loopback)
            .await
            .unwrap_err();
        assert!(
            matches!(blocked_lo, mush_net_ssh::CreateErr::BadName(ref msg) if msg.contains("blocked")),
            "blocked exempt IP must still be refused: {blocked_lo:?}"
        );
    }

    #[tokio::test]
    async fn ssh_hooks_auth_account_maps_argon2_saturation_to_busy() {
        use mush_net_ssh::SshHooks;

        let (bridge, _sessions, _seen) = rig(true);
        assert_eq!(
            SshHooks::auth_account(&bridge, "pilot", "wrong").await,
            mush_net_ssh::AuthResult::BadCreds
        );
        bridge.core.accounts.busy.store(true, Ordering::SeqCst);
        assert_eq!(
            SshHooks::auth_account(&bridge, "pilot", "wrong").await,
            mush_net_ssh::AuthResult::Busy
        );
    }
}
