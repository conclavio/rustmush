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

use std::net::IpAddr;
use std::sync::Arc;

use mush_core::ObjId;
use mush_proto::{ClientCaps, OutputEvent, RespondChoice};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use smol_str::SmolStr;
use tokio::sync::mpsc;

// The shared bridge vocabulary lives in bridge_common; re-exported here so
// existing SSH-side imports (`mushd::ssh_bridge::{AuthResult, ...}`) keep
// working unchanged.
pub use crate::bridge_common::{AuthResult, CharSummary, ConnectErr, LineOutcome};

use mush_config::ipguard::IpGuard;

use crate::accounts::{AuthBackend, AuthOutcome, CreateErr};
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
}

/// The production hooks implementation. Generic over [`AuthBackend`] so the
/// M2 Redis store slots in without touching this file.
pub struct SshBridge<A: AuthBackend + 'static> {
    core: GameBridge<A>,
    /// Shared metrics registry — SSH transport connection counters. Defaults
    /// to a throwaway registry (tests); the real server injects the shared one
    /// via [`SshBridge::with_metrics`].
    metrics: Arc<mush_net_web::Metrics>,
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

    /// Wire the bridge with an explicit [`GatewayLink`] (M16b): the forked
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
            metrics: Arc::new(mush_net_web::Metrics::new()),
        }
    }

    /// Point SSH connection counters at the shared metrics registry (the
    /// same one the web ingress serves over `/metrics`). Called by `mushd`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<mush_net_web::Metrics>) -> SshBridge<A> {
        self.metrics = metrics;
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
        let actor = ObjId(u32::try_from(character).map_err(|_| ConnectErr::NotYourCharacter)?);
        let characters = self.core.accounts.characters(account).await;
        let Some(char_name) = characters
            .iter()
            .find(|c| c.id == actor)
            .map(|c| c.name.clone())
        else {
            return Err(ConnectErr::NotYourCharacter);
        };
        let caps_structured = conn.caps.structured;
        let handle = ConnectionHandle::new(conn.id, conn.addr, conn.caps, conn.tx);
        self.core
            .enter_game(handle, actor, account, &char_name, caps_structured)
            .await?;
        Ok(())
    }

    /// One input line from an attached connection.
    pub async fn line(&self, conn: ConnId, line: &str) -> LineOutcome {
        self.core.line(conn, line).await
    }

    /// The transport lost (or closed) a connection.
    pub async fn disconnected(&self, conn: ConnId) {
        self.core.disconnected(conn).await;
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
}

// ---------------------------------------------------------------------------
// The real transport wiring.
// ---------------------------------------------------------------------------

mod net {
    use futures::future::BoxFuture;
    use mush_net_ssh as net_ssh;

    use super::*;

    /// Adapt the bridge to the transport's dyn-compatible hooks trait.
    ///
    /// The trait carries no peer address on `auth_account`/`create_account`;
    /// IP gating on the real path rides [`SshBridge::is_ip_blocked`], which
    /// the transport checks before every auth attempt — SSH-level *and*
    /// in-band welcome-screen `connect`/`create` — plus the per-connection
    /// pre-auth flood bucket inside `mush-net-ssh`. The per-IP concurrency
    /// cap rides `connection_opened`/`connection_closed`, called by the
    /// transport at accept/drop.
    impl<A: AuthBackend + 'static> net_ssh::SshHooks for SshBridge<A> {
        fn auth_account<'a>(
            &'a self,
            account: &'a str,
            password: &'a str,
        ) -> BoxFuture<'a, net_ssh::AuthResult> {
            Box::pin(async move {
                match self.core.accounts.verify(account, password).await {
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
        ) -> BoxFuture<'a, Result<(), net_ssh::CreateErr>> {
            Box::pin(async move {
                // The account's first character shares the account's name
                // (as typed); more arrive via the select screen's newchar.
                match self.core.accounts.create(account, password, account).await {
                    Ok(_) => Ok(()),
                    Err(CreateErr::AccountExists | CreateErr::CharacterNameTaken) => {
                        Err(net_ssh::CreateErr::Taken)
                    }
                    Err(CreateErr::BadPassword) => Err(net_ssh::CreateErr::Weak),
                    Err(other) => Err(net_ssh::CreateErr::BadName(other.to_string())),
                }
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
            let conn = NewConn {
                id: ConnId(conn.id),
                addr: conn.addr,
                caps: conn.caps,
                tx: conn.tx,
            };
            Box::pin(async move {
                self.connect_character(conn, account, character)
                    .await
                    .map_err(|e| e.to_string())
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
        /// args, which lists the inbox. The correlation id is unused by the
        /// TUI — it routes replies by the inner event's type — so a fixed id
        /// is fine.
        fn query(&self, conn: u64, name: String, args: serde_json::Value) -> BoxFuture<'_, ()> {
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
                self.core.query(conn, 0, SmolStr::new(name), args).await;
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

        fn caps_changed(&self, conn: u64, caps: ClientCaps) {
            self.caps_changed(ConnId(conn), caps);
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
        self.accept_loop.abort();
        tracing::info!("SSH listener stopped accepting");
    }
}

/// Bind the SSH listener and start serving on a background task.
pub async fn start_listener<A: AuthBackend + 'static>(
    listen: std::net::SocketAddr,
    data_dir: &std::path::Path,
    banner: String,
    bridge: Arc<SshBridge<A>>,
) -> Result<SshListener, String> {
    let cfg = mush_net_ssh::SshConfig {
        listen,
        host_key_path: data_dir.join("ssh_host_ed25519"),
        banner,
    };
    let hooks: Arc<dyn mush_net_ssh::SshHooks> = bridge;
    let bound = mush_net_ssh::SshServer::bind(cfg, hooks)
        .await
        .map_err(|e| e.to_string())?;
    let addr = bound.local_addr();
    let accept_loop = tokio::spawn(async move {
        if let Err(e) = bound.run().await {
            tracing::error!(error = %e, "SSH accept loop failed");
        }
    });
    Ok(SshListener { addr, accept_loop })
}
