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
//! - The **bridge** owns everything game-side: argon2 checks via the
//!   [`AuthBackend`] (off the async workers with `spawn_blocking`), the
//!   [`SessionManager`] attach/detach lifecycle, flood control, input
//!   sanitation, `QUIT`, linkdead grace reaping, and the [`IpGuard`].
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
//! The game-side flows are **inherent methods** on [`SshBridge`] (typed
//! with [`ConnId`], [`ConnectErr`], [`LineOutcome`] — and directly testable
//! without sockets); the `net` block at the bottom adapts them to the
//! dyn-compatible [`mush_net_ssh::SshHooks`] trait and starts the real
//! listener ([`start_listener`]).

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use mush_core::{DisconnectKind, Inbound, ObjId};
use mush_proto::{ClientCaps, ObjRef, OutputEvent, RichText};
use mush_session::{
    ConnId, ConnectionHandle, DetachOutcome, DetachReason, FloodVerdict, SessionManager,
    sanitize_line,
};
use tokio::sync::mpsc;

use crate::accounts::{AuthBackend, AuthOutcome, CreateErr};
use crate::ipguard::IpGuard;
use crate::world_task::{WorldHandle, char_key, char_key_obj, send_event_to_conn};

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

/// One character offered on the character-select / welcome screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharSummary {
    /// Character key (a widened `ObjId`; opaque to the transport).
    pub id: u64,
    /// Character name.
    pub name: String,
}

/// Result of an account credential check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthResult {
    /// Credentials good.
    Ok {
        /// Account name in registered display case.
        account: String,
        /// The account's characters.
        characters: Vec<CharSummary>,
    },
    /// Unknown account or wrong password (deliberately indistinguishable).
    Denied,
}

/// Why [`SshBridge::connect_character`] refused.
///
/// (The per-IP concurrent-connection cap is enforced by the transport at
/// accept time via the `connection_opened` hook, not here.)
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectErr {
    /// The character does not belong to that account.
    #[error("that is not your character")]
    NotYourCharacter,
    /// The world task is gone (shutdown in progress).
    #[error("the game is shutting down")]
    WorldGone,
}

/// What the transport should do after feeding a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOutcome {
    /// Keep reading.
    Continue,
    /// Flush pending output and close the connection (player quit).
    Close,
}

/// The production hooks implementation. Generic over [`AuthBackend`] so the
/// M2 Redis store slots in without touching this file.
pub struct SshBridge<A: AuthBackend + 'static> {
    accounts: Arc<A>,
    sessions: Arc<SessionManager>,
    world: WorldHandle,
    guard: Arc<IpGuard>,
}

impl<A: AuthBackend + 'static> SshBridge<A> {
    /// Wire the bridge.
    pub fn new(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        world: WorldHandle,
        guard: Arc<IpGuard>,
    ) -> SshBridge<A> {
        SshBridge {
            accounts,
            sessions,
            world,
            guard,
        }
    }

    /// The actor a connection is playing, if attached.
    fn actor_of(&self, conn: ConnId) -> Option<ObjId> {
        self.sessions
            .session_of_conn(conn)
            .and_then(|s| self.sessions.character_of(s))
            .map(char_key_obj)
    }

    /// Check account credentials (welcome-screen `connect`, SSH password
    /// auth for `<account>@host`). A blocked address is denied outright.
    /// (The backend runs the argon2id verify on the blocking pool.)
    pub async fn auth_account(&self, account: &str, password: &str, addr: IpAddr) -> AuthResult {
        if self.guard.is_blocked(addr) {
            return AuthResult::Denied;
        }
        match self.accounts.verify(account, password).await {
            AuthOutcome::Ok {
                account,
                characters,
            } => AuthResult::Ok {
                account,
                characters: characters
                    .into_iter()
                    .map(|c| CharSummary {
                        id: u64::from(c.id.0),
                        name: c.name,
                    })
                    .collect(),
            },
            AuthOutcome::BadCredentials => AuthResult::Denied,
        }
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
        if self.guard.is_blocked(addr) {
            return Err(CreateErr::Io("your address is blocked".to_string()));
        }
        // Validation + argon2 hash + store reservation + ack-gated world
        // round trip; the backend hops CPU-heavy work to the blocking pool.
        let created = self.accounts.create(account, password, char_name).await?;
        Ok(CharSummary {
            id: u64::from(created.id.0),
            name: created.name,
        })
    }

    /// Create an additional character on an authenticated account (the
    /// character-select screen's `newchar`). Cap and ack gating live in the
    /// backend.
    pub async fn create_character(
        &self,
        account: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        let created = self.accounts.create_character(account, char_name).await?;
        Ok(CharSummary {
            id: u64::from(created.id.0),
            name: created.name,
        })
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
        let characters = self.accounts.characters(account).await;
        let Some(char_name) = characters
            .iter()
            .find(|c| c.id == actor)
            .map(|c| c.name.clone())
        else {
            return Err(ConnectErr::NotYourCharacter);
        };

        let caps_structured = conn.caps.structured;
        let conn_id = conn.id;
        let handle = ConnectionHandle::new(conn.id, conn.addr, conn.caps, conn.tx);
        let (_sess, outcome) = self
            .sessions
            .attach(handle, char_key(actor), account.to_string());
        let (connections, from_linkdead) = match outcome {
            mush_session::AttachOutcome::New => (1, false),
            mush_session::AttachOutcome::Rejoined {
                others,
                from_linkdead,
            } => (others + 1, from_linkdead),
        };

        // World side: connection up (announces connect/reconnect), then an
        // initial look so the player sees where they are.
        if !self
            .world
            .inbound(Inbound::ConnUp {
                conn: conn_id.0,
                actor,
                caps_structured,
                from_linkdead,
            })
            .await
        {
            self.sessions.detach(conn_id, DetachReason::Booted);
            return Err(ConnectErr::WorldGone);
        }
        send_event_to_conn(
            &self.sessions,
            conn_id,
            OutputEvent::SessionInfo {
                character: ObjRef::new(actor.to_string(), char_name),
                account: account.to_string(),
                connections: u8::try_from(connections).unwrap_or(u8::MAX),
            },
        );
        self.world
            .inbound(Inbound::Cmd {
                conn: conn_id.0,
                actor,
                line: "look".to_string(),
            })
            .await;
        tracing::info!(conn = conn_id.0, %actor, account, "character connected");
        Ok(())
    }

    /// One input line from an attached connection.
    pub async fn line(&self, conn: ConnId, line: &str) -> LineOutcome {
        // Idle bookkeeping first — even a throttled keypress is activity.
        self.sessions.record_input(conn);
        let line = sanitize_line(line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return LineOutcome::Continue;
        }
        // QUIT is checked before the flood throttle: a flooding client must
        // always be able to leave.
        if trimmed.eq_ignore_ascii_case("QUIT") {
            self.quit(conn).await;
            return LineOutcome::Close;
        }
        if self.sessions.check_input(conn) == FloodVerdict::Throttled {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::Sys(RichText::tag("error", "You're doing that too fast.")),
            );
            return LineOutcome::Continue;
        }
        let Some(actor) = self.actor_of(conn) else {
            tracing::warn!(conn = conn.0, "line from unattached connection dropped");
            return LineOutcome::Continue;
        };
        self.world
            .inbound(Inbound::Cmd {
                conn: conn.0,
                actor,
                line: trimmed.to_string(),
            })
            .await;
        LineOutcome::Continue
    }

    /// The transport lost (or closed) a connection. (Its IP-guard slot is
    /// released by the transport's own accept/drop accounting, not here.)
    pub async fn disconnected(&self, conn: ConnId) {
        let actor = self.actor_of(conn);
        let outcome = self.sessions.detach(conn, DetachReason::Lost);
        let (Some(actor), Some(outcome)) = (actor, outcome) else {
            // Already detached (QUIT raced the transport teardown) or never
            // attached: nothing game-side.
            return;
        };
        match outcome {
            DetachOutcome::OthersRemain | DetachOutcome::Linkdead { .. } => {
                // The engine announces "has gone link-dead." when this was
                // the actor's last connection.
                self.world
                    .inbound(Inbound::ConnDown {
                        conn: conn.0,
                        actor,
                        reason: DisconnectKind::Lost,
                    })
                    .await;
            }
            DetachOutcome::Closed => {
                // Session vanished underneath us (e.g. concurrent close):
                // clean scoped cleanup for this connection, then a full
                // disconnect (which the engine ignores if a fresh session's
                // connections are live — treaty ruling 3 re-validation).
                self.world
                    .inbound(Inbound::ConnDown {
                        conn: conn.0,
                        actor,
                        reason: DisconnectKind::Boot,
                    })
                    .await;
                self.world.inbound(Inbound::SessionGone { actor }).await;
            }
        }
        if let DetachOutcome::Linkdead { grace } = outcome {
            tracing::info!(conn = conn.0, %actor, grace_s = grace.as_secs(), "linkdead");
            self.arm_linkdead_reaper(grace);
        }
    }

    /// Terminal resize / capability renegotiation.
    pub fn caps_changed(&self, conn: ConnId, caps: ClientCaps) {
        self.sessions.update_caps(conn, caps);
    }

    /// Auth gate: is the address currently tempbanned? (The per-IP
    /// connection cap is separate — `connection_opened` at accept time.)
    #[must_use]
    pub fn is_ip_blocked(&self, addr: IpAddr) -> bool {
        self.guard.is_blocked(addr)
    }

    /// Record one failed auth attempt against an IP.
    pub fn auth_failed(&self, addr: IpAddr) {
        self.guard.record_failure(addr);
    }

    /// The `QUIT` flow, serialized through the world task so a quitter's
    /// in-flight output is never dropped:
    ///
    /// 1. `ConnDown(Quit)` is queued **behind** every line the connection
    ///    already sent; the engine processes those first (their output
    ///    reaches this connection's queue), then announces the clean
    ///    disconnect ("has disconnected." on the last connection) and
    ///    forgets the conn.
    /// 2. A world barrier guarantees all of that has been delivered.
    /// 3. Only then the farewell + `Disconnect` event go out and the session
    ///    detaches. Detaching drops the session's `ConnectionHandle` (and
    ///    with it the output sender), which is what tells the transport's
    ///    pump to close the socket after draining.
    async fn quit(&self, conn: ConnId) {
        if let Some(actor) = self.actor_of(conn) {
            self.world
                .inbound(Inbound::ConnDown {
                    conn: conn.0,
                    actor,
                    reason: DisconnectKind::Quit,
                })
                .await;
            self.world.barrier().await;
        }
        send_event_to_conn(
            &self.sessions,
            conn,
            OutputEvent::Disconnect {
                reason: RichText::tag("system", "Goodbye."),
            },
        );
        self.sessions.detach(conn, DetachReason::Quit);
    }

    /// Arm one reaper for a session that just went linkdead: after the
    /// grace period, every expired linkdead session is closed and its
    /// character fully disconnected. Reattachment in the meantime revives
    /// the session and [`SessionManager::take_expired`] skips it.
    fn arm_linkdead_reaper(&self, grace: std::time::Duration) {
        let sessions = Arc::clone(&self.sessions);
        let world = self.world.clone();
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            for expired in sessions.take_expired(Instant::now()) {
                let actor = char_key_obj(expired.character);
                tracing::info!(%actor, account = %expired.account, "linkdead grace expired");
                world.inbound(Inbound::SessionGone { actor }).await;
            }
        });
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
                match self.accounts.verify(account, password).await {
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
                match self.accounts.create(account, password, account).await {
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

        fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()> {
            Box::pin(async move { self.disconnected(ConnId(conn)).await })
        }

        fn is_ip_blocked(&self, addr: IpAddr) -> bool {
            self.guard.is_blocked(addr)
        }

        fn auth_failed(&self, addr: IpAddr) {
            self.guard.record_failure(addr);
        }

        fn connection_opened(&self, addr: IpAddr) -> bool {
            self.guard.conn_opened(addr)
        }

        fn connection_closed(&self, addr: IpAddr) {
            self.guard.conn_closed(addr);
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
