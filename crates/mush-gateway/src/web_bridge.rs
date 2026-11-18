//! The bridge between the web front end (`mush-net-web`: axum HTTP + WS)
//! and the game: token auth, structured-session attachment, resume replay,
//! input, and the `Req` → [`mush_core::Inbound::Query`] hydration path — the
//! [`mush_net_web::WebHooks`] implementation.
//!
//! ## Division of labor (mirrors [`crate::ssh_bridge`])
//!
//! - The **transport** (`mush-net-web`) owns axum, the WS upgrade, JSON
//!   framing (it serializes the [`mush_proto::Envelope`]s it receives on the conn's
//!   channel), the `hello` frame (features + panels from its `WebConfig` —
//!   `mushd` fills those from [`WebBridge::panels`] at boot), the
//!   `resumed { replayed }` frame it derives from [`WebHooks::resume`]'s
//!   result, app-level ping, and the `/api/auth/*` rate limiter.
//! - The **bridge** owns everything game-side, through the shared
//!   [`GameBridge`] core plus the token machinery here: opaque 256-bit
//!   tokens stored **only** as SHA-256 hashes with a 7-day sliding TTL
//!   ([`ObjectStore`] `token_*` ops), argon2 verification on the blocking
//!   pool (inside the [`AuthBackend`]), and the [`IpGuard`].
//! - Failure counting follows the SSH convention exactly: the bridge never
//!   records auth failures itself — the transport calls
//!   [`WebHooks::auth_failed`] once per failed login/register, and gates on
//!   [`WebHooks::is_ip_blocked`] first.
//!
//! ## Token rules (design 02 §6.2, binding)
//!
//! Raw tokens are 32 bytes from the OS-seeded CSPRNG, hex-encoded (64
//! chars). They exist in memory only on their way to the client; the store
//! holds `sha256(token)` under `token:{hash}` with native TTL. Tokens are
//! **never logged** — no `tracing` call in this module ever receives one —
//! and never reach the world task. Validation slides the expiry to
//! `now + 7 days`. `@newpassword` revokes every token of the target's
//! account ([`ObjectStore::tokens_del_account`]), so stolen or stale web
//! sessions die with the old password.
//!
//! ## Resume semantics (exact contract with `mush-net-web/src/ws.rs`)
//!
//! `resume` replays **before** attaching, so the live queue never carries
//! an envelope the replay already delivered (the SPA renders every frame it
//! receives — duplicates would double-print). The cost is a microsecond
//! window between the replay snapshot and the attach whose events this
//! connection misses (they stay in the ring; they were "lost to the
//! outage" from the client's point of view). On [`ResumeErr::TooOld`] —
//! which this bridge also returns when the session was already reaped —
//! **nothing has been attached**: the transport falls back to a fresh
//! [`WebHooks::connect_character`] with the same conn id, which is the
//! re-hydration path (fresh `Room` render; the client re-requests panels).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::future::BoxFuture;
use mush_core::{DisconnectKind, ObjId};
use mush_ipc::{ConnectionAdmission, InputSend, ToServer, WireInbound};
use mush_net_web::{
    AttachErr, CharSummary, LoginErr, LoginOk, NewWebConn, RegisterErr, ResumeErr, ResumeOk,
    SessionStart, TokenIdentity, TokenVerdict, WebHooks,
};
use mush_proto::{
    ClientCaps, ClientKind, ColorDepth, MindMarketRequest, OutputEvent, Query, RespondChoice,
    RichText, SmolStr,
};
use mush_session::{ConnId, ConnectionHandle, FloodVerdict, ReplayError, SessionManager};
use mush_store::{ObjectStore, TokenRecord};
use sha2::{Digest, Sha256};

use mush_config::ipguard::IpGuard;
use mush_config::unix_now_s;

use crate::accounts::{AuthBackend, CreateErr};
use crate::bridge_common::{AuthResult, ConnectErr, GameBridge};
use crate::delivery::{char_key, send_event_to_conn};
use crate::drain::{CloseWay, GatewayDrain};
use crate::poison::locked;

/// Web session lifetime: 7 days, sliding (each successful validation
/// pushes expiry out to `now + TOKEN_TTL_S`).
pub const TOKEN_TTL_S: i64 = 7 * 24 * 60 * 60;

/// **The largest `PanelSubscribe.params` the bridge forwards** (doc 20 cut SM.9; the brief's
/// 3.8), serialised bytes: a pulled feed's few query words, never a payload.
pub const MAX_PANEL_PARAMS_BYTES: usize = 512;

/// The reply to a `PanelSubscribe` whose `params` is not an object or is past
/// [`MAX_PANEL_PARAMS_BYTES`].
pub const PANEL_PARAMS_REFUSED: &str = "Panel parameters must be a small object.";

/// Why [`WebBridge::authorize_character`] could not hand back a character:
/// the same split [`TokenVerdict`] draws, carried one level up so each
/// caller can map it onto its own transport vocabulary
/// ([`AttachErr`] / [`ResumeErr`]) without re-folding the two together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthzFail {
    /// Checked: the token is dead, or names no such character.
    Refused,
    /// Not checked: the store could not answer.
    Unavailable,
}

/// The production [`WebHooks`] implementation: the shared [`GameBridge`]
/// core plus the token store and the panel manifest.
pub struct WebBridge<A: AuthBackend + 'static> {
    core: GameBridge<A>,
    store: Arc<dyn ObjectStore>,
    /// Panel feed ids for `hello` / `PanelSubscribe` checks
    /// (module-registered feeds plus the engine-side `"who"`), collected at
    /// boot from the registry.
    panels: Vec<String>,
    /// Live web connections by the **hash** of the token that authenticated
    /// them (never the raw token), so `logout` can boot exactly the
    /// connections that token opened — revocation is instant (design 02
    /// §6.2), not "at next reconnect". Entries live from a successful
    /// `connect_character`/`resume` until `disconnected` (or the boot
    /// itself).
    conn_tokens: Mutex<HashMap<u64, String>>,
    /// The composing-hint rate limit, per connection (interface plan I4,
    /// D-I-11).
    composing: Mutex<crate::composing::ComposingLimiter>,
    /// Shared metrics registry — web transport connection counters, and the
    /// registry the `/healthz` + `/metrics` endpoints read (via
    /// [`WebHooks::metrics`]). Defaults to a throwaway registry (tests); the
    /// real server injects the shared one via [`WebBridge::with_metrics`].
    metrics: Arc<mush_net_web::Metrics>,
    /// The bit set on every connection id the web transport allocates (doc 33
    /// §4.3): the gateway generation's, none until
    /// [`WebBridge::with_conn_id_bit`] sets it.
    conn_id_bit: u64,
}

impl<A: AuthBackend + 'static> WebBridge<A> {
    /// Wire the bridge. `panels` is the panel manifest for `hello` /
    /// `PanelSubscribe` (boot wiring: the registry's panel feeds plus
    /// `"who"` — see `main.rs`). Metrics land in a throwaway registry until
    /// [`WebBridge::with_metrics`] injects the server-wide one.
    pub fn new(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        world: impl mush_ipc::GatewayLink + 'static,
        guard: Arc<IpGuard>,
        store: Arc<dyn ObjectStore>,
        panels: Vec<String>,
    ) -> WebBridge<A> {
        Self::with_link(accounts, sessions, Arc::new(world), guard, store, panels)
    }

    /// Wire the bridge with an explicit [`GatewayLink`](mush_ipc::GatewayLink)
    /// (M16b): the forked `mush-gateway` passes a socket-client link instead of
    /// an in-process world handle.
    pub fn with_link(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        link: Arc<dyn mush_ipc::GatewayLink>,
        guard: Arc<IpGuard>,
        store: Arc<dyn ObjectStore>,
        panels: Vec<String>,
    ) -> WebBridge<A> {
        WebBridge {
            core: GameBridge::with_link(accounts, sessions, link, guard),
            store,
            panels,
            conn_tokens: Mutex::new(HashMap::new()),
            composing: Mutex::new(crate::composing::ComposingLimiter::default()),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            conn_id_bit: 0,
        }
    }

    /// The bit every connection id the web transport allocates carries (doc
    /// 33 §4.3): the gateway generation's
    /// (`mush_config::generation::Generation::conn_id_bit`). The split gateway
    /// sets it from `--generation`; `mushd` and the tests leave it at none.
    #[must_use]
    pub fn with_conn_id_bit(mut self, bit: u64) -> WebBridge<A> {
        self.conn_id_bit = bit;
        self
    }

    /// Count this bridge's attached connections into the generation's drain
    /// (doc 33 §4.5). The split gateway sets it; `mushd` and the tests leave
    /// the bridge without one.
    #[must_use]
    pub fn with_drain(mut self, drain: Arc<GatewayDrain>) -> WebBridge<A> {
        self.core.set_drain(drain);
        self
    }

    /// Point web connection counters — and the `/healthz` + `/metrics`
    /// endpoints — at the shared metrics registry. Called by `mushd`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<mush_net_web::Metrics>) -> WebBridge<A> {
        self.metrics = metrics;
        self
    }

    /// The panel feed ids the server can publish — `mushd` copies these
    /// into the transport's `WebConfig::panels` for the `hello` frame.
    #[must_use]
    pub fn panels(&self) -> &[String] {
        &self.panels
    }

    /// Viewport / capability update from the client
    /// (`ClientCommand::Caps`). Not part of the [`WebHooks`] contract (the
    /// transport may handle caps purely client-side); provided so the
    /// integration can forward them into the session layer like SSH resize
    /// does.
    pub fn caps_changed(&self, conn: u64, caps: ClientCaps) {
        self.core.sessions.update_caps(ConnId(conn), caps);
    }

    /// Mint + persist a fresh token for `account`. The raw token goes to
    /// the caller; the store sees only the hash.
    async fn issue_token(&self, account: &str) -> Result<String, String> {
        let token = mint_token();
        let now = unix_now_s();
        self.store
            .token_put(
                token_hash(&token),
                TokenRecord {
                    account: account.to_string(),
                    created: now,
                    expires: now + TOKEN_TTL_S,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(token)
    }

    /// Shared token validation: fetch by hash, slide the TTL, resolve the
    /// account's characters.
    ///
    /// Fails closed on a store error — but as
    /// [`TokenVerdict::Unavailable`], not as a refusal. The two used to
    /// collapse into one `None`, which the transport could only report as
    /// "your session has ended": one stalled instance Redis would then have
    /// deleted the stored token of every open tab at once, each holding a
    /// perfectly valid 7-day credential that was never revoked. The store
    /// not answering is the server's fault and the client's cue to retry.
    async fn validate(&self, token: &str) -> TokenVerdict {
        let hash = token_hash(token);
        let rec = match self.store.token_get(&hash).await {
            Ok(Some(rec)) => rec,
            Ok(None) => return TokenVerdict::Refused,
            Err(e) => {
                tracing::error!(error = %e, "token lookup failed; failing closed (unavailable)");
                return TokenVerdict::Unavailable;
            }
        };
        // Sliding TTL: push expiry out on every successful validation. A
        // `false` means the token lapsed between get and touch — expired.
        match self
            .store
            .token_touch(&hash, unix_now_s() + TOKEN_TTL_S)
            .await
        {
            Ok(true) => {}
            Ok(false) => return TokenVerdict::Refused,
            Err(e) => {
                // The token itself validated; a failed refresh only means
                // the TTL did not slide this time. Not fatal.
                tracing::warn!(error = %e, "token TTL refresh failed (continuing)");
            }
        }
        let characters = self
            .core
            .accounts
            .characters(&rec.account)
            .await
            .into_iter()
            .map(|c| CharSummary {
                key: u64::from(c.id.0),
                name: c.name,
            })
            .collect();
        TokenVerdict::Valid(TokenIdentity {
            account: rec.account,
            characters,
        })
    }

    /// Validate a token AND that `character` is on its account. Returns the
    /// account name + character name, or which kind of failure it was —
    /// both callers must keep "this credential is dead" apart from "the
    /// store could not say", because only the first may cost the client its
    /// token.
    async fn authorize_character(
        &self,
        token: &str,
        character: u64,
    ) -> Result<(String, String), AuthzFail> {
        let identity = match self.validate(token).await {
            TokenVerdict::Valid(identity) => identity,
            TokenVerdict::Refused => return Err(AuthzFail::Refused),
            TokenVerdict::Unavailable => return Err(AuthzFail::Unavailable),
        };
        let name = identity
            .characters
            .iter()
            .find(|c| c.key == character)
            .map(|c| c.name.clone())
            .ok_or(AuthzFail::Refused)?;
        Ok((identity.account, name))
    }

    /// The default capability set for a fresh web connection: structured
    /// JSON wire, true color, UTF-8. Width/height are nominal — the web
    /// client renders its own layout and may update them via
    /// [`WebBridge::caps_changed`].
    fn web_caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Web,
            color: ColorDepth::TrueColor,
            width: 80,
            height: 24,
            utf8: true,
            structured: true,
        }
    }

    /// Send a correlated `Reply` straight to one connection, bypassing the
    /// world task (transport-level refusals and acks). The session layer
    /// stamps the envelope; conn-scoped sends are never ring-recorded.
    fn direct_reply(&self, conn: ConnId, id: u64, event: OutputEvent) {
        send_event_to_conn(
            &self.core.sessions,
            conn,
            OutputEvent::Reply {
                id,
                event: Box::new(event),
            },
        );
    }

    fn direct_error_reply(&self, conn: ConnId, id: u64, msg: impl Into<String>) {
        self.direct_reply(conn, id, OutputEvent::Sys(RichText::tag("error", msg)));
    }

    /// Remember which token (hash) authenticated a now-attached connection.
    fn track_conn(&self, conn: u64, token: &str) {
        locked(&self.conn_tokens, "conn_tokens").insert(conn, token_hash(token));
    }

    /// Boot one attached web connection game-side: world `ConnDown(Boot)`
    /// (announce + engine bookkeeping), a barrier so that output lands
    /// first, a final `Disconnect` event, then the clean detach — dropping
    /// the session's handle closes the envelope channel, which is what
    /// tells the WS pump to close the socket after draining. Mirrors
    /// [`GameBridge::quit`] with `Boot` semantics.
    async fn boot_conn(&self, conn: ConnId, why: &str) {
        if let Some(actor) = self.core.actor_of(conn) {
            self.core
                .link
                .to_server(ToServer::Inbound(WireInbound::ConnDown {
                    conn: conn.0,
                    actor,
                    reason: DisconnectKind::Boot,
                }))
                .await;
            self.core.delivery_barrier(conn, "web boot").await;
        }
        send_event_to_conn(
            &self.core.sessions,
            conn,
            OutputEvent::Disconnect {
                reason: RichText::tag("system", why),
            },
        );
        self.core
            .sessions
            .detach(conn, mush_session::DetachReason::Booted);
        locked(&self.conn_tokens, "conn_tokens").remove(&conn.0);
    }
}

impl<A: AuthBackend + 'static> WebHooks for WebBridge<A> {
    fn login<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
        ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, LoginErr>> {
        Box::pin(async move {
            // Sitebans read as Banned; the failure-window tempban as
            // RateLimited. `is_blocked` folds both, so consult the siteban
            // mirror first.
            if self.core.guard.siteban_active_at(ip, unix_now_s()) {
                return Err(LoginErr::Banned);
            }
            if self.core.guard.is_blocked(ip) {
                return Err(LoginErr::RateLimited);
            }
            match self.core.auth_account(account, password, ip).await {
                AuthResult::Ok {
                    account,
                    characters,
                } => {
                    let token = self.issue_token(&account).await.map_err(|e| {
                        // The credentials were GOOD; only the session mint
                        // failed (store hiccup). Surface as try-again —
                        // never BadCreds, which the transport counts
                        // against the IP.
                        tracing::error!(error = %e, "token mint failed after successful login");
                        LoginErr::RateLimited
                    })?;
                    Ok(LoginOk {
                        token,
                        account,
                        characters: characters
                            .into_iter()
                            .map(|c| CharSummary {
                                key: c.id,
                                name: c.name,
                            })
                            .collect(),
                    })
                }
                AuthResult::Denied => Err(LoginErr::BadCreds),
            }
        })
    }

    fn register<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
        char_name: &'a str,
        ip: IpAddr,
    ) -> BoxFuture<'a, Result<LoginOk, RegisterErr>> {
        Box::pin(async move {
            if self.core.is_ip_blocked(ip) {
                return Err(RegisterErr::Banned);
            }
            // The shared creation flow: validation, argon2 on the blocking
            // pool, atomic store reservation, ack-gated Player creation.
            let created = match self
                .core
                .create_account(account, password, char_name, ip)
                .await
            {
                Ok(c) => c,
                Err(CreateErr::AccountExists | CreateErr::CharacterNameTaken) => {
                    return Err(RegisterErr::Taken);
                }
                Err(CreateErr::BadPassword) => return Err(RegisterErr::Weak),
                Err(other) => return Err(RegisterErr::BadName(format!("{other}."))),
            };
            let token = self.issue_token(account).await.map_err(|e| {
                // Account + character exist; only the auto-login token
                // failed. The player can log in normally.
                tracing::error!(error = %e, "token mint failed after registration");
                RegisterErr::BadName(
                    "Your account was created, but signing you in failed; \
                     please log in."
                        .to_string(),
                )
            })?;
            Ok(LoginOk {
                token,
                account: account.to_string(),
                characters: vec![CharSummary {
                    key: created.id,
                    name: created.name,
                }],
            })
        })
    }

    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let hash = token_hash(token);
            if let Err(e) = self.store.token_del(&hash).await {
                tracing::error!(error = %e, "token delete failed on logout");
            }
            // Revocation is instant: any live connection this token opened
            // is booted now, not left playing until its transport drops.
            let doomed: Vec<u64> = {
                let map = locked(&self.conn_tokens, "conn_tokens");
                map.iter()
                    .filter(|&(_, h)| *h == hash)
                    .map(|(&c, _)| c)
                    .collect()
            };
            for conn in doomed {
                tracing::info!(conn, "logout boots live web connection");
                self.boot_conn(ConnId(conn), "You have logged out.").await;
            }
        })
    }

    fn validate_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, TokenVerdict> {
        Box::pin(self.validate(token))
    }

    fn connect_character<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<SessionStart, AttachErr>> {
        Box::pin(async move {
            let (account, char_name) = match self.authorize_character(token, character).await {
                Ok(pair) => pair,
                Err(AuthzFail::Refused) => {
                    return Err(AttachErr::Refused(
                        "Invalid session; please log in again.".to_string(),
                    ));
                }
                // The store could not say whether the token is good. Not the
                // player's credential's fault, so not the fatal close.
                Err(AuthzFail::Unavailable) => return Err(AttachErr::Unavailable),
            };
            let Ok(actor) = u32::try_from(character).map(ObjId) else {
                return Err(AttachErr::Refused(
                    "That is not your character.".to_string(),
                ));
            };
            let closer = conn.closer.clone();
            let handle = ConnectionHandle::new_structured(
                ConnId(conn.id),
                conn.addr,
                Self::web_caps(),
                conn.tx,
            );
            let conn_id = conn.id;
            // Track BEFORE attach (LOW finding: track_conn revocation
            // race) — a concurrent logout sweep must never be able to run
            // between the session attaching and this entry existing, so it
            // is inserted before `enter_game` even starts, not after it
            // succeeds; removed again if the attach itself fails.
            self.track_conn(conn_id, token);
            let entered = match self
                .core
                .enter_game(handle, actor, &account, &char_name, true)
                .await
            {
                Ok(entered) => entered,
                Err(e) => {
                    locked(&self.conn_tokens, "conn_tokens").remove(&conn_id);
                    // `enter_game` refuses for transient reasons — full
                    // lifecycle custody, the server half being gone
                    // mid-setup (the ordinary split `reload`), and a live
                    // server that did not bind the connection (doc 33 GG.2,
                    // a duplicate id across gateways). None says anything
                    // about the token, so none may reach the client as the
                    // close that deletes it; the resume path has always
                    // mapped the same to 1013/1012.
                    return Err(match e {
                        ConnectErr::Busy => AttachErr::Busy,
                        ConnectErr::WorldGone
                        | ConnectErr::Restarting
                        | ConnectErr::ServerDown
                        | ConnectErr::NotAdmitted => AttachErr::Unavailable,
                        ConnectErr::NotYourCharacter => AttachErr::Refused(format!("{e}.")),
                    });
                }
            };
            // Doc 33 §4.6: attached, so one of the generation's live
            // connections, closed at a drain's deadline with a 1012 frame.
            self.core
                .watch_connection(ConnId(conn_id), CloseWay::Web(closer));
            let _ = self.core.sessions.redeliver_unacknowledged(ConnId(conn_id));
            Ok(SessionStart {
                resume_from: entered.resume_from,
            })
        })
    }

    fn resume<'a>(
        &'a self,
        conn: NewWebConn,
        token: &'a str,
        character: u64,
        last_seq: u64,
    ) -> BoxFuture<'a, Result<ResumeOk, ResumeErr>> {
        Box::pin(async move {
            let account = match self.authorize_character(token, character).await {
                Ok((account, _char_name)) => account,
                Err(AuthzFail::Refused) => return Err(ResumeErr::BadSession),
                // Same rule as the fresh attach: a store that could not
                // answer is a reconnect, not an expired session.
                Err(AuthzFail::Unavailable) => return Err(ResumeErr::Unavailable),
            };
            let Ok(actor) = u32::try_from(character).map(ObjId) else {
                return Err(ResumeErr::BadSession);
            };
            // Resume targets an EXISTING session (live or linkdead). A
            // reaped session cannot replay — report TooOld, whose
            // transport-side fallback (fresh connect_character) IS the
            // re-hydration path. Nothing is attached on any Err.
            let Some(sess) = self.core.sessions.session_of_char(char_key(actor)) else {
                tracing::info!(conn = conn.id, %actor, "resume with no session; re-hydrating");
                return Err(ResumeErr::TooOld);
            };
            // Replay BEFORE attaching (see the module docs): the live queue
            // must never duplicate a replayed envelope.
            let replayed = match self.core.sessions.replay_since(sess, last_seq) {
                Ok(envs) => envs,
                Err(ReplayError::TooOld | ReplayError::UnknownSession) => {
                    tracing::info!(conn = conn.id, %actor, "resume gap too old; re-hydrating");
                    return Err(ResumeErr::TooOld);
                }
            };
            let conn_id = ConnId(conn.id);
            let closer = conn.closer.clone();
            let handle =
                ConnectionHandle::new_structured(conn_id, conn.addr, Self::web_caps(), conn.tx);
            match self.core.link.try_reserve_connection(conn_id.0, actor) {
                ConnectionAdmission::Reserved => {}
                ConnectionAdmission::Busy => {
                    tracing::warn!(conn = conn_id.0, %actor, "web resume refused: lifecycle custody busy");
                    return Err(ResumeErr::Busy);
                }
                ConnectionAdmission::Gone => return Err(ResumeErr::Unavailable),
            }
            // Track BEFORE attach (LOW finding: track_conn revocation
            // race) — see the matching comment in `connect_character`;
            // removed again if the world's ConnUp confirmation fails.
            self.track_conn(conn_id.0, token);
            let (_sess, outcome) = self.core.sessions.attach(handle, char_key(actor), account);
            let from_linkdead = matches!(
                outcome,
                mush_session::AttachOutcome::Rejoined {
                    from_linkdead: true,
                    ..
                }
            );
            // World side: the reconnect announce (delivered live, after the
            // replay snapshot — never duplicated into it).
            if !self
                .core
                .link
                .to_server(ToServer::Inbound(WireInbound::ConnUp {
                    conn: conn_id.0,
                    actor,
                    caps_structured: true,
                    from_linkdead,
                }))
                .await
            {
                self.core.link.cancel_connection_reservation(conn_id.0);
                self.core.rollback_local_attach(conn_id, from_linkdead);
                locked(&self.conn_tokens, "conn_tokens").remove(&conn_id.0);
                return Err(ResumeErr::Unavailable);
            }
            let live_from = replayed.last().map_or(last_seq, |env| env.seq) + 1;
            tracing::info!(
                conn = conn_id.0,
                %actor,
                replayed = replayed.len(),
                live_from,
                "web session resumed"
            );
            // Doc 33 §4.6: a resumed connection is as live as a fresh one.
            self.core.watch_connection(conn_id, CloseWay::Web(closer));
            let _ = self.core.sessions.redeliver_unacknowledged(conn_id);
            Ok(ResumeOk {
                replayed,
                live_from,
            })
        })
    }

    fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // On `Close` (QUIT) the detach already dropped the envelope
            // sender; the transport's pump closes the socket.
            let _ = self.core.line(ConnId(conn), &line).await;
        })
    }

    fn invoke(&self, conn: u64, id: u64, request: mush_proto::InvokeRequest) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.core.invoke(conn, id, request).await })
    }

    fn request(&self, conn: u64, id: u64, q: Query) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let conn_id = ConnId(conn);
            // Interface plan I12: the narrative backlog is answered LOCALLY from
            // the caller's resume ring (the world cannot read the gateway ring),
            // gated by a STRICT flood check that ignores flood-exempt — a
            // locally-answered query has no world-side backpressure. Dispatched
            // before the generic, exempt-honouring `check_input`. On a failed
            // match `q` is untouched and flows to the world-routed match below.
            if let Query::NarrativeBacklog { before_seq, limit } = q {
                if self.core.sessions.check_query_flood(conn_id) == FloodVerdict::Throttled {
                    self.direct_error_reply(conn_id, id, "You're doing that too fast.");
                    return;
                }
                self.core
                    .narrative_backlog_reply(conn_id, id, before_seq, limit);
                return;
            }
            // Requests share the per-connection input flood bucket with
            // lines (a panel cannot spam the world task past the throttle)
            // but do NOT stamp idle time — a background refresh is not the
            // player typing.
            if self.core.sessions.check_input(conn_id) == FloodVerdict::Throttled {
                self.direct_error_reply(conn_id, id, "You're doing that too fast.");
                return;
            }
            let Some(actor) = self.core.actor_of(conn_id) else {
                tracing::warn!(conn, "request from unattached connection dropped");
                return;
            };
            // Map the wire Query onto a registered query handler
            // (engine-side dispatch via Inbound::Query), or answer directly
            // where no world state is needed.
            let (name, args) = match q {
                Query::Who => (SmolStr::new_static("who"), serde_json::json!({})),
                Query::MailList { folder } => (
                    SmolStr::new_static("mail.list"),
                    serde_json::json!({ "folder": folder }),
                ),
                Query::ChannelBacklog {
                    channel,
                    before_seq,
                    limit,
                } => (
                    SmolStr::new_static("comsys.backlog"),
                    serde_json::json!({
                        "channel": channel,
                        "before_seq": before_seq,
                        "limit": limit,
                    }),
                ),
                Query::Sheet => {
                    // Route to the active ruleset's sheet query handler,
                    // registered under its `<ruleset>.sheet` feed name (e.g.
                    // "wod.sheet"). Ruleset-agnostic: we pick the sole feed
                    // whose id ends in ".sheet" rather than hardcoding "wod",
                    // so a future ruleset works with no bridge change. When no
                    // ruleset is loaded there is no such feed, so we keep the
                    // polite "unavailable" reply the web panel degrades to.
                    match self.panels.iter().find(|p| p.ends_with(".sheet")) {
                        Some(feed) => (SmolStr::new(feed), serde_json::json!({})),
                        None => {
                            self.direct_reply(
                                conn_id,
                                id,
                                OutputEvent::Sys(RichText::tag(
                                    "system",
                                    "Character sheets are not available here.",
                                )),
                            );
                            return;
                        }
                    }
                }
                Query::Commands => (SmolStr::new_static("commands.list"), serde_json::json!({})),
                Query::MindBulk { run, page } => (
                    SmolStr::new_static("mind.bulk"),
                    serde_json::json!({ "run": run, "page": page }),
                ),
                Query::MarketOriginalMinds(request) => (
                    SmolStr::new_static("market.original_minds"),
                    serde_json::json!(request),
                ),
                // 17g C3a (the C3 plan §2.3): the player market's two panel
                // reads, each forwarded to its same-named feed query with
                // the closed request's own fields (the code already decoded
                // as a `ListingCode`, the cursor as a `u64`).
                Query::MindMarket(MindMarketRequest::Listings { after }) => (
                    SmolStr::new_static("mind.market"),
                    serde_json::json!({ "after": after }),
                ),
                Query::MindMarket(MindMarketRequest::Holdings { section, after }) => (
                    SmolStr::new_static("mind.holdings"),
                    serde_json::json!({ "section": section, "after": after }),
                ),
                Query::PanelSubscribe { feed, params } => {
                    if !self.panels.iter().any(|p| p.as_str() == feed.as_str()) {
                        self.direct_error_reply(
                            conn_id,
                            id,
                            format!("No such panel feed {feed:?}."),
                        );
                        return;
                    }
                    // The subscription ack IS the panel's keyframe where a
                    // state query exists; deltas then stream live. (M4 has
                    // no per-feed mute — every attached connection receives
                    // all panel frames.) The feeds whose query is named
                    // differently from the feed — `comsys`, `mail`, `intel`,
                    // and `legal.actions`, answered locally — are mapped
                    // here; every other feed forwards under its own name, and
                    // the engine answers with the same-named handler's
                    // keyframe or, for a push-only feed, a plain "subscribed"
                    // ack (interface plan I1, D-I-3) — so `llm.dashboard`,
                    // `wod.vitals`, `scenes` and the next feed need no bridge
                    // arm. A feed whose keyframe query is named differently
                    // DOES need one (look-back LB-10 found `intel` missing:
                    // the subscribe forwarded `intel`, which no handler
                    // answers), until `Registrar::panel_feed` declares it.
                    match feed.as_str() {
                        "comsys" => (SmolStr::new_static("comsys.state"), serde_json::json!({})),
                        "who" => (SmolStr::new_static("who"), serde_json::json!({})),
                        "intel" => (SmolStr::new_static("intel.list"), serde_json::json!({})),
                        "legal.actions" => {
                            self.core.legal_actions_panel(conn_id, id, actor).await;
                            return;
                        }
                        "mail" => (
                            SmolStr::new_static("mail.list"),
                            serde_json::json!({ "folder": null }),
                        ),
                        // Doc 20 cut SM.9 (the brief's 3.8): a pulled feed's parameters ride as
                        // its query's arguments — an object, at most `MAX_PANEL_PARAMS_BYTES`
                        // serialised, else refused before anything is sent.
                        _ => match params {
                            None => (feed.clone(), serde_json::json!({})),
                            Some(params)
                                if params.is_object()
                                    && serde_json::to_vec(&params).is_ok_and(|bytes| {
                                        bytes.len() <= MAX_PANEL_PARAMS_BYTES
                                    }) =>
                            {
                                (feed.clone(), params)
                            }
                            Some(_) => {
                                self.direct_error_reply(
                                    conn_id,
                                    id,
                                    PANEL_PARAMS_REFUSED.to_owned(),
                                );
                                return;
                            }
                        },
                    }
                }
                // Answered locally above (before the flood check), so it never
                // reaches this world-routing match.
                Query::NarrativeBacklog { .. } => {
                    unreachable!("narrative backlog is answered locally")
                }
            };
            match self
                .core
                .link
                .to_server_input(ToServer::Inbound(WireInbound::Query {
                    conn,
                    actor,
                    id,
                    name,
                    args,
                }))
                .await
            {
                // A shed Query stays SILENT — the widget keeps its last data
                // (matching the flood-drop path); only the shed counter + warn
                // fire, inside the link.
                InputSend::Sent | InputSend::Shed => {}
                InputSend::Gone => {
                    self.direct_error_reply(conn_id, id, "The game is shutting down.");
                }
            }
        })
    }

    fn acknowledge(
        &self,
        conn: u64,
        commit: mush_proto::ProjectedCommitToken,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self
                .core
                .sessions
                .acknowledge_receipt(ConnId(conn), &commit);
        })
    }

    fn composing(&self, conn: u64, active: bool) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // A hint, not input: no flood bucket and no idle stamp. The
            // limiter keeps a burst of keystrokes to one hint per interval;
            // an `active: false` always passes (interface plan I4, D-I-11).
            let allowed =
                locked(&self.composing, "composing limiter").allow(conn, active, Instant::now());
            if !allowed {
                return;
            }
            let Some(actor) = self.core.actor_of(ConnId(conn)) else {
                return;
            };
            self.core
                .link
                .to_server(ToServer::Inbound(WireInbound::Composing {
                    conn,
                    actor,
                    active,
                }))
                .await;
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
        Box::pin(async move {
            locked(&self.composing, "composing limiter").forget(conn);
            locked(&self.conn_tokens, "conn_tokens").remove(&conn);
            self.core.disconnected(ConnId(conn)).await;
        })
    }

    fn is_ip_blocked(&self, addr: IpAddr) -> bool {
        self.core.is_ip_blocked(addr)
    }

    fn auth_failed(&self, addr: IpAddr) {
        self.core.auth_failed(addr);
    }

    // A refused `/ws` token rides the guard's OWN counter, never the one
    // that tempbans: a 256-bit token record is not guessable, so a refused
    // one is a tab that outlived the record, and counting it as a credential
    // failure is what locked an operator out of SSH on 2026-09-20.
    fn token_refused(&self, addr: IpAddr) {
        self.core.token_refused(addr);
    }

    fn is_token_throttled(&self, addr: IpAddr) -> bool {
        self.core.is_token_throttled(addr)
    }

    // Per-IP concurrent-connection cap (HIGH finding): the same `IpGuard`
    // counter the SSH bridge already claims/releases via
    // `connection_opened`/`connection_closed` — reused, not duplicated, so
    // one address's SSH and web connections share one cap.
    fn connection_opened(&self, addr: IpAddr) -> bool {
        let admitted = self.core.guard.conn_opened(addr);
        // Count only admitted connections so the current-gauge balances with
        // connection_closed (called once per admitted WS task; the crate
        // contract guarantees the pairing).
        if admitted {
            self.metrics.web_connection_opened();
        }
        admitted
    }

    fn connection_closed(&self, addr: IpAddr) {
        self.core.guard.conn_closed(addr);
        self.metrics.web_connection_closed();
    }

    fn metrics(&self) -> Arc<mush_net_web::Metrics> {
        Arc::clone(&self.metrics)
    }

    fn conn_id_bit(&self) -> u64 {
        self.conn_id_bit
    }
}

// ---------------------------------------------------------------------------
// Token primitives
// ---------------------------------------------------------------------------

/// 32 bytes from the OS-seeded CSPRNG (`rand`'s thread-local generator),
/// hex-encoded: the raw session token.
fn mint_token() -> String {
    let bytes: [u8; 32] = rand::random();
    hex(&bytes)
}

/// The store key material for a token: SHA-256 of the raw token, hex.
/// Crate-visible so tests can locate records without ever persisting a raw
/// token.
pub(crate) fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use mush_ipc::{GatewayLink, ServiceReq, ServiceResp};
    use mush_proto::Envelope;
    use mush_store::InMemoryStore;
    use tokio::sync::{Notify, mpsc};

    struct ResumeAuth {
        actor: ObjId,
    }

    impl AuthBackend for ResumeAuth {
        async fn verify(&self, _account: &str, _password: &str) -> crate::accounts::AuthOutcome {
            crate::accounts::AuthOutcome::BadCredentials
        }

        async fn characters(&self, _account: &str) -> Vec<crate::accounts::CharSummary> {
            vec![crate::accounts::CharSummary {
                id: self.actor,
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

    #[derive(Default)]
    struct FailedConnUpState {
        pending_linkdead: bool,
        reserved: bool,
        cancellations: usize,
        attempted_conn_up: usize,
        delivered: Vec<ToServer>,
    }

    /// Models the exact restart race: reservation succeeds, then the boundary
    /// disappears before the matching ConnUp can take wire custody.
    struct FailConnUpLink {
        state: Arc<Mutex<FailedConnUpState>>,
        session_gone: Arc<Notify>,
    }

    impl GatewayLink for FailConnUpLink {
        fn try_reserve_connection(&self, _conn: u64, _actor: ObjId) -> ConnectionAdmission {
            let mut state = self.state.lock().unwrap();
            assert!(
                state.pending_linkdead,
                "the reconnect reuses expiry custody"
            );
            assert!(!state.reserved, "only one reservation is expected");
            state.pending_linkdead = false;
            state.reserved = true;
            ConnectionAdmission::Reserved
        }

        fn cancel_connection_reservation(&self, _conn: u64) {
            let mut state = self.state.lock().unwrap();
            if state.reserved {
                state.reserved = false;
                state.pending_linkdead = true;
                state.cancellations += 1;
            }
        }

        fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
            let delivered = {
                let mut state = self.state.lock().unwrap();
                if matches!(&msg, ToServer::Inbound(WireInbound::ConnUp { .. })) {
                    state.attempted_conn_up += 1;
                    false
                } else {
                    if matches!(&msg, ToServer::Inbound(WireInbound::SessionGone { .. })) {
                        state.pending_linkdead = false;
                        self.session_gone.notify_one();
                    }
                    state.delivered.push(msg);
                    true
                }
            };
            Box::pin(async move { delivered })
        }

        fn request(&self, _req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            Box::pin(async { None })
        }
    }

    /// A link that answers admission however the test says and never gets
    /// as far as carrying a message: the refusals under test all happen
    /// before or at admission.
    struct StubLink {
        admission: ConnectionAdmission,
    }

    impl GatewayLink for StubLink {
        fn try_reserve_connection(&self, _conn: u64, _actor: ObjId) -> ConnectionAdmission {
            self.admission
        }

        fn cancel_connection_reservation(&self, _conn: u64) {}

        fn to_server(&self, _msg: ToServer) -> BoxFuture<'_, bool> {
            Box::pin(async { false })
        }

        fn request(&self, _req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            Box::pin(async { None })
        }
    }

    async fn stub_bridge(
        store: Arc<dyn ObjectStore>,
        admission: ConnectionAdmission,
    ) -> WebBridge<ResumeAuth> {
        WebBridge::with_link(
            Arc::new(ResumeAuth { actor: ObjId(7) }),
            Arc::new(SessionManager::default()),
            Arc::new(StubLink { admission }),
            Arc::new(IpGuard::new()),
            store,
            Vec::new(),
        )
    }

    /// The store being unable to answer is not evidence against the token.
    /// The transport's only way to say "refused" is the close that makes the
    /// SPA DELETE its stored token, so folding a store outage into a refusal
    /// would have signed every open tab out — each holding a valid,
    /// unrevoked 7-day credential — the moment this instance's Redis
    /// stalled during a deploy. Fail closed, yes; fail *fatal*, no.
    #[tokio::test]
    async fn a_store_that_cannot_answer_is_unavailable_not_refused() {
        let raw = InMemoryStore::new();
        let token = "a-perfectly-good-token";
        let now = unix_now_s();
        raw.token_put(
            token_hash(token),
            TokenRecord {
                account: "acct".to_string(),
                created: now,
                expires: now + TOKEN_TTL_S,
            },
        )
        .await
        .unwrap();
        let store: Arc<dyn ObjectStore> = Arc::new(raw);
        let bridge = stub_bridge(Arc::clone(&store), ConnectionAdmission::Reserved).await;

        // Baseline: while the store answers, the same token is Valid and a
        // token it has never heard of is Refused.
        assert!(matches!(
            bridge.validate_token(token).await,
            TokenVerdict::Valid(_)
        ));
        assert_eq!(
            bridge.validate_token("never-issued").await,
            TokenVerdict::Refused
        );

        // Now the store cannot answer at all (Redis restarting, pool
        // reconnecting). The SAME good token must not be called dead.
        store.close().await.unwrap();
        assert_eq!(
            bridge.validate_token(token).await,
            TokenVerdict::Unavailable,
            "a store outage says nothing about the credential"
        );
        let (tx, _rx) = mpsc::channel::<Arc<Envelope>>(4);
        assert_eq!(
            bridge
                .connect_character(
                    NewWebConn {
                        id: 1,
                        addr: "127.0.0.1".parse().unwrap(),
                        tx,
                        closer: mush_net_web::ConnCloser::detached(),
                    },
                    token,
                    7,
                )
                .await,
            Err(AttachErr::Unavailable),
            "and the attach refusal must be the retryable one"
        );
    }

    /// The other half of the same rule: a game half that is full or gone is
    /// a retry, not a dead session. An ordinary split `reload` restarts
    /// `mush-server` under a surviving gateway — a web tab reconnecting
    /// inside that window must not be logged out by it.
    #[tokio::test]
    async fn a_busy_or_absent_world_is_never_a_credential_refusal() {
        for (admission, expected) in [
            (ConnectionAdmission::Busy, AttachErr::Busy),
            (ConnectionAdmission::Gone, AttachErr::Unavailable),
        ] {
            let raw = InMemoryStore::new();
            let token = "live-token";
            let now = unix_now_s();
            raw.token_put(
                token_hash(token),
                TokenRecord {
                    account: "acct".to_string(),
                    created: now,
                    expires: now + TOKEN_TTL_S,
                },
            )
            .await
            .unwrap();
            let bridge = stub_bridge(Arc::new(raw), admission).await;
            let (tx, _rx) = mpsc::channel::<Arc<Envelope>>(4);
            let err = bridge
                .connect_character(
                    NewWebConn {
                        id: 2,
                        addr: "127.0.0.1".parse().unwrap(),
                        tx,
                        closer: mush_net_web::ConnCloser::detached(),
                    },
                    token,
                    7,
                )
                .await
                .expect_err("admission refuses");
            assert_eq!(err, expected, "{admission:?} must stay retryable");
            assert!(
                !matches!(err, AttachErr::Refused(_)),
                "only a checked-and-dead credential may take the fatal close"
            );
            assert!(
                bridge.conn_tokens.lock().unwrap().is_empty(),
                "a refused attach leaves no tracked connection behind"
            );
        }
    }

    #[test]
    fn tokens_are_64_hex_chars_and_unique() {
        let a = mint_token();
        let b = mint_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two mints must differ");
    }

    #[test]
    fn token_hash_is_sha256_hex() {
        // sha256("abc") — the classic test vector.
        assert_eq!(
            token_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(token_hash("abc").len(), 64);
        assert_ne!(token_hash("abc"), token_hash("abd"));
    }

    #[tokio::test]
    async fn failed_resume_conn_up_restores_linkdead_without_conn_down() {
        let actor = ObjId(7);
        let sessions = Arc::new(SessionManager::with_config(mush_session::SessionConfig {
            linkdead_grace: std::time::Duration::ZERO,
            ..mush_session::SessionConfig::default()
        }));

        // Establish a real resumable session with one ring event, then make it
        // link-dead without arming an old timer. The failed resume below owns
        // responsibility for restoring and re-arming it.
        let (old_tx, _old_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let old_conn = ConnId(10);
        let old = ConnectionHandle::new_structured(
            old_conn,
            "127.0.0.1".parse().unwrap(),
            WebBridge::<ResumeAuth>::web_caps(),
            old_tx,
        );
        let (sess, _) = sessions.attach(old, char_key(actor), "acct".to_string());
        sessions.send_to_char(
            char_key(actor),
            Arc::new(OutputEvent::Sys(RichText::tag("system", "retained event"))),
        );
        let ring_seq = sessions.last_seq(sess).expect("session sequence");
        assert!(matches!(
            sessions.detach(old_conn, mush_session::DetachReason::Lost),
            Some(mush_session::DetachOutcome::Linkdead { .. })
        ));

        let token = "restart-race-token";
        let now = unix_now_s();
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        store
            .token_put(
                token_hash(token),
                TokenRecord {
                    account: "acct".to_string(),
                    created: now,
                    expires: now + TOKEN_TTL_S,
                },
            )
            .await
            .unwrap();

        let state = Arc::new(Mutex::new(FailedConnUpState {
            pending_linkdead: true,
            ..FailedConnUpState::default()
        }));
        let session_gone = Arc::new(Notify::new());
        let link = Arc::new(FailConnUpLink {
            state: Arc::clone(&state),
            session_gone: Arc::clone(&session_gone),
        });
        let bridge = WebBridge::with_link(
            Arc::new(ResumeAuth { actor }),
            Arc::clone(&sessions),
            link,
            Arc::new(IpGuard::new()),
            store,
            Vec::new(),
        );
        let (new_tx, _new_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let new_conn = ConnId(11);

        let result = bridge
            .resume(
                NewWebConn {
                    id: new_conn.0,
                    addr: "127.0.0.2".parse().unwrap(),
                    tx: new_tx,
                    closer: mush_net_web::ConnCloser::detached(),
                },
                token,
                u64::from(actor.0),
                0,
            )
            .await;
        assert!(matches!(result, Err(ResumeErr::Unavailable)));

        // Before the zero-grace reaper gets CPU, the exact old session and its
        // ring are back in link-dead state. There was no world-visible ConnUp,
        // therefore rollback must not fabricate a ConnDown.
        assert_eq!(sessions.session_of_char(char_key(actor)), Some(sess));
        assert!(sessions.session_of_conn(new_conn).is_none());
        let replayed = sessions.replay_since(sess, 0).expect("ring retained");
        assert_eq!(replayed.last().map(|env| env.seq), Some(ring_seq));
        assert_eq!(bridge.core.sessions.connection_count(), 0);
        assert!(!bridge.conn_tokens.lock().unwrap().contains_key(&new_conn.0));
        {
            let state = state.lock().unwrap();
            assert_eq!(state.attempted_conn_up, 1);
            assert_eq!(state.cancellations, 1);
            assert!(!state.reserved);
            assert!(state.pending_linkdead, "expiry custody was restored");
            assert!(
                state
                    .delivered
                    .iter()
                    .all(|msg| !matches!(msg, ToServer::Inbound(WireInbound::ConnDown { .. })))
            );
        }

        tokio::time::timeout(std::time::Duration::from_secs(1), session_gone.notified())
            .await
            .expect("restored linkdead session must be reaped");
        assert!(sessions.session_of_char(char_key(actor)).is_none());
        let state = state.lock().unwrap();
        assert!(
            !state.pending_linkdead,
            "SessionGone released expiry custody"
        );
        assert!(matches!(
            state.delivered.as_slice(),
            [ToServer::Inbound(WireInbound::SessionGone { actor: gone })] if *gone == actor
        ));
    }
}
