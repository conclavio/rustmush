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

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::StreamExt;
use futures::future::BoxFuture;
use mush_core::{DisconnectKind, ObjId};
use mush_ipc::{ConnectionAdmission, InputSend, ToServer, WireInbound};
use mush_net_web::{
    AttachErr, CharSummary, LoginErr, LoginOk, LogoutErr, NewWebConn, RegisterErr, ResumeErr,
    ResumeOk, SessionStart, TokenIdentity, TokenVerdict, WebHooks,
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
use crate::bridge_common::{AuthCheckResult, ConnectErr, GameBridge};
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

/// Bounded capacity of the recent in-process revocation ring used to fence
/// concurrent attach/resume futures against a logout that lands mid-attach
/// (EXT-P1-3). Older revocations are already durable in the token store and
/// caught by the post-attach `token_get` re-verification.
const MAX_RECENT_REVOCATIONS: usize = 4_096;

/// Live web-connection token map plus a monotonic revocation barrier
/// (EXT-P1-3).
///
/// Every `connect_character` and `resume` checks `is_revoked` before and after
/// attaching (atomically while inserting into `conn_tokens`), followed by a
/// durable store re-check. A concurrent `logout` or cross-generation
/// `RevokeWebToken` records the token hash in `recent_revocations` (bounded
/// FIFO with deduplicating monotonic sequence numbers) and drains any
/// already-tracked connections for that hash. Because raw tokens are 256-bit
/// random values that are never re-minted, a revoked hash stays refused locally
/// even if a durable `token_del` failed transiently.
#[derive(Default)]
struct TokenRegistry {
    revocation_seq: u64,
    conn_tokens: HashMap<u64, String>,
    recent_revocations: HashMap<String, u64>,
    revocation_order: VecDeque<(u64, String)>,
}

impl TokenRegistry {
    fn is_revoked(&self, hash: &str) -> bool {
        self.recent_revocations.contains_key(hash)
    }

    fn record_revocation(&mut self, hash: &str) -> Vec<u64> {
        self.revocation_seq = self.revocation_seq.saturating_add(1);
        let seq = self.revocation_seq;
        self.recent_revocations.insert(hash.to_owned(), seq);
        self.revocation_order.push_back((seq, hash.to_owned()));
        while self.recent_revocations.len() > MAX_RECENT_REVOCATIONS {
            let Some((old_seq, old_hash)) = self.revocation_order.pop_front() else {
                break;
            };
            if self.recent_revocations.get(&old_hash).copied() == Some(old_seq) {
                self.recent_revocations.remove(&old_hash);
            }
        }
        while self.revocation_order.len() > MAX_RECENT_REVOCATIONS.saturating_mul(2) {
            let Some((old_seq, old_hash)) = self.revocation_order.pop_front() else {
                break;
            };
            if self.recent_revocations.get(&old_hash).copied() == Some(old_seq) {
                self.recent_revocations.remove(&old_hash);
            }
        }
        // P2-EDGE-1: keep `conn_tokens` entries intact until each connection
        // is actually detached from `SessionManager`, so an interrupted
        // revocation can never blind retries or `reconcile_web_tokens`.
        self.conn_tokens
            .iter()
            .filter(|&(_, h)| h == hash)
            .map(|(&c, _)| c)
            .collect()
    }

    fn try_track_if_unrevoked(&mut self, conn: u64, hash: String) -> bool {
        if self.is_revoked(&hash) {
            return false;
        }
        self.conn_tokens.insert(conn, hash);
        true
    }

    fn is_tracked_and_unrevoked(&self, conn: u64, hash: &str) -> bool {
        !self.is_revoked(hash) && self.conn_tokens.get(&conn).map(String::as_str) == Some(hash)
    }

    fn remove_conn(&mut self, conn: u64) {
        self.conn_tokens.remove(&conn);
    }

    fn live_hashes(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.conn_tokens
            .values()
            .filter(|h| seen.insert((*h).clone()))
            .cloned()
            .collect()
    }
}

/// The production [`WebHooks`] implementation: the shared [`GameBridge`]
/// core plus the token store and the panel manifest.
pub struct WebBridge<A: AuthBackend + 'static> {
    core: Arc<GameBridge<A>>,
    store: Arc<dyn ObjectStore>,
    /// Panel feed ids for `hello` / `PanelSubscribe` checks
    /// (module-registered feeds plus the engine-side `"who"`), collected at
    /// boot from the registry.
    panels: Vec<String>,
    /// Live web connections by the **hash** of the token that authenticated
    /// them (never the raw token) plus the in-process revocation barrier, so
    /// `logout` and cross-generation revocations boot every live connection
    /// for that token and fence any in-flight attach/resume (design 02 §6.2,
    /// EXT-P1-3, P2-EDGE-1).
    tokens: Arc<Mutex<TokenRegistry>>,
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
            core: Arc::new(GameBridge::with_link(accounts, sessions, link, guard)),
            store,
            panels,
            tokens: Arc::new(Mutex::new(TokenRegistry::default())),
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
        if let Some(core) = Arc::get_mut(&mut self.core) {
            core.set_drain(drain);
        }
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
        if locked(&self.tokens, "token registry").is_revoked(&hash) {
            return TokenVerdict::Refused;
        }
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

    /// Verify after local tracking that `hash` was neither revoked locally nor
    /// deleted in the shared token store during attach/resume (EXT-P1-3).
    /// Returns `Ok(())` when the token is still live and tracked; otherwise
    /// boots the connection cleanly and returns whether the failure was a
    /// revocation (`AuthzFail::Refused`) or a transient store error
    /// (`AuthzFail::Unavailable`).
    async fn verify_post_attach(&self, conn_id: ConnId, hash: &str) -> Result<(), AuthzFail> {
        let tracked = locked(&self.tokens, "token registry")
            .try_track_if_unrevoked(conn_id.0, hash.to_owned());
        if !tracked {
            tracing::info!(
                conn = conn_id.0,
                "token revoked during attach/resume; booting connection"
            );
            self.boot_conn(conn_id, "You have logged out.").await;
            return Err(AuthzFail::Refused);
        }
        match self.store.token_get(hash).await {
            Ok(Some(_)) => {
                let still_valid = locked(&self.tokens, "token registry")
                    .is_tracked_and_unrevoked(conn_id.0, hash);
                if !still_valid {
                    tracing::info!(
                        conn = conn_id.0,
                        "token revoked during post-attach verification; booting connection"
                    );
                    self.boot_conn(conn_id, "You have logged out.").await;
                    return Err(AuthzFail::Refused);
                }
                Ok(())
            }
            Ok(None) => {
                tracing::info!(
                    conn = conn_id.0,
                    "token missing from store after attach/resume; booting connection"
                );
                self.boot_conn(conn_id, "You have logged out.").await;
                Err(AuthzFail::Refused)
            }
            Err(e) => {
                tracing::error!(
                    conn = conn_id.0,
                    error = %e,
                    "post-attach token re-verification failed; failing closed (unavailable)"
                );
                self.boot_conn(
                    conn_id,
                    "Session verification unavailable; reconnect shortly.",
                )
                .await;
                Err(AuthzFail::Unavailable)
            }
        }
    }

    /// Boot one attached web connection game-side: world `ConnDown(Boot)`
    /// (announce + engine bookkeeping), a barrier so that output lands
    /// first, a final `Disconnect` event, then the clean detach — dropping
    /// the session's handle closes the envelope channel, which is what
    /// tells the WS pump to close the socket after draining. Mirrors
    /// [`GameBridge::quit`] with `Boot` semantics.
    async fn boot_conn(&self, conn: ConnId, why: &str) {
        boot_conn_parts(&self.core, &self.tokens, conn, why).await;
    }
}

/// Detached-task-friendly implementation of [`WebBridge::boot_conn`]: detaches
/// the connection and removes it from `conn_tokens` only AFTER `sessions.detach`
/// completes (P2-EDGE-1).
async fn boot_conn_parts<A: AuthBackend + 'static>(
    core: &GameBridge<A>,
    tokens: &Mutex<TokenRegistry>,
    conn: ConnId,
    why: &str,
) {
    if let Some(actor) = core.actor_of(conn) {
        core.link
            .to_server(ToServer::Inbound(WireInbound::ConnDown {
                conn: conn.0,
                actor,
                reason: DisconnectKind::Boot,
            }))
            .await;
        core.delivery_barrier(conn, "web boot").await;
    }
    send_event_to_conn(
        &core.sessions,
        conn,
        OutputEvent::Disconnect {
            reason: RichText::tag("system", why),
        },
    );
    core.sessions
        .detach(conn, mush_session::DetachReason::Booted);
    locked(tokens, "token registry").remove_conn(conn.0);
}

/// Maximum simultaneous local `boot_conn` barriers run by one `WebBridge::logout`
/// (P2-EDGE-1).
const MAX_CONCURRENT_LOGOUT_BOOTS: usize = 8;

impl<A: AuthBackend + 'static> crate::controls::WebTokenRevoker for WebBridge<A> {
    fn revoke_token_hash(&self, token_hash: &str) -> Vec<u64> {
        locked(&self.tokens, "token registry").record_revocation(token_hash)
    }

    fn forget_conn(&self, conn: u64) {
        locked(&self.tokens, "token registry").remove_conn(conn);
    }

    fn live_token_hashes(&self) -> Vec<String> {
        locked(&self.tokens, "token registry").live_hashes()
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
            match self.core.auth_account_checked(account, password, ip).await {
                AuthCheckResult::Ok {
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
                AuthCheckResult::Denied => Err(LoginErr::BadCreds),
                AuthCheckResult::Busy => Err(LoginErr::RateLimited),
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
                Err(CreateErr::RateLimited) => return Err(RegisterErr::RateLimited),
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

    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<(), LogoutErr>> {
        let hash = token_hash(token);
        let store = Arc::clone(&self.store);
        let core = Arc::clone(&self.core);
        let tokens = Arc::clone(&self.tokens);
        Box::pin(async move {
            // Detach revocation, peer-generation broadcast, and local boots into
            // a non-cancellable task (P2-EDGE-1): client abort or HTTP
            // `request_timeout` dropping this future mid-barrier can never
            // strand live WebSockets or suppress `ToServer::RevokeWebToken`.
            let task = tokio::spawn(async move {
                let del_result = store.token_del(&hash).await;
                if let Err(ref e) = del_result {
                    tracing::error!(error = %e, "token delete failed on logout");
                }
                // Record the revocation in-process immediately (blocking any
                // concurrent attach/resume) WITHOUT evicting `conn_tokens`
                // until each connection is actually detached below.
                let doomed = locked(&tokens, "token registry").record_revocation(&hash);
                // Propagate revocation to peer gateway generations BEFORE
                // awaiting local delivery barriers so slow local barriers
                // cannot delay peer-generation ejection (EXT-P1-3, P2-EDGE-1).
                let _ = core
                    .link
                    .to_server(ToServer::RevokeWebToken { token_hash: hash })
                    .await;
                // Boot local connections concurrently (bounded), so multiple
                // live tabs on a busy world do not stack 5s barriers serially.
                futures::stream::iter(doomed)
                    .for_each_concurrent(MAX_CONCURRENT_LOGOUT_BOOTS, |conn| {
                        let core = Arc::clone(&core);
                        let tokens = Arc::clone(&tokens);
                        async move {
                            tracing::info!(conn, "logout boots live web connection");
                            boot_conn_parts(&core, &tokens, ConnId(conn), "You have logged out.")
                                .await;
                        }
                    })
                    .await;
                del_result.map(|_| ()).map_err(|_| LogoutErr::Unavailable)
            });
            match task.await {
                Ok(res) => res,
                Err(_) => Err(LogoutErr::Unavailable),
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
            let hash = token_hash(token);
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
            if locked(&self.tokens, "token registry").is_revoked(&hash) {
                return Err(AttachErr::Refused(
                    "Invalid session; please log in again.".to_string(),
                ));
            }
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
            let entered = match self
                .core
                .enter_game(handle, actor, &account, &char_name, true)
                .await
            {
                Ok(entered) => entered,
                Err(e) => {
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
                        | ConnectErr::ResumeExpired
                        | ConnectErr::NotAdmitted => AttachErr::Unavailable,
                        ConnectErr::NotYourCharacter => AttachErr::Refused(format!("{e}.")),
                    });
                }
            };
            // EXT-P1-3: atomically check the in-process revocation barrier
            // while inserting into `conn_tokens`, then re-verify durable token
            // presence in the store. Closes both single-process attach/logout
            // TOCTOU and concurrent remote-generation logout / `@newpassword`.
            if let Err(fail) = self.verify_post_attach(ConnId(conn_id), &hash).await {
                return Err(match fail {
                    AuthzFail::Refused => {
                        AttachErr::Refused("Invalid session; please log in again.".to_string())
                    }
                    AuthzFail::Unavailable => AttachErr::Unavailable,
                });
            }
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
            let hash = token_hash(token);
            let account = match self.authorize_character(token, character).await {
                Ok((account, _char_name)) => account,
                Err(AuthzFail::Refused) => return Err(ResumeErr::BadSession),
                // Same rule as the fresh attach: a store that could not
                // answer is a reconnect, not an expired session.
                Err(AuthzFail::Unavailable) => return Err(ResumeErr::Unavailable),
            };
            if locked(&self.tokens, "token registry").is_revoked(&hash) {
                return Err(ResumeErr::BadSession);
            }
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
            let (_sess, outcome) = self.core.sessions.attach(handle, char_key(actor), account);
            if matches!(outcome, mush_session::AttachOutcome::Duplicate) {
                self.core.link.cancel_connection_reservation(conn_id.0);
                return Err(ResumeErr::BadSession);
            }
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
                return Err(ResumeErr::Unavailable);
            }
            // EXT-P1-3: check local revocation barrier + track atomically, then
            // re-verify durable token presence in the store.
            if let Err(fail) = self.verify_post_attach(conn_id, &hash).await {
                return Err(match fail {
                    AuthzFail::Refused => ResumeErr::BadSession,
                    AuthzFail::Unavailable => ResumeErr::Unavailable,
                });
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
            // Every structured query — local (`NarrativeBacklog`) or
            // world-routed — charges the strict query flood bucket so
            // `flood_exempt` (mail compose, chargen) can never be abused to
            // bypass flood control (HIGH-2). Queries do NOT stamp idle time.
            if self.core.sessions.check_query_flood(conn_id) == FloodVerdict::Throttled {
                self.direct_error_reply(conn_id, id, "You're doing that too fast.");
                return;
            }
            if let Query::NarrativeBacklog { before_seq, limit } = q {
                self.core
                    .narrative_backlog_reply(conn_id, id, before_seq, limit);
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
            // A hint, not input: no idle stamp. The per-conn state machine
            // enforces cooldown on both `true` and `false` transitions and
            // ignores redundant hints, and the frame rides the bounded
            // sheddable `to_server_input` lane (HIGH-1).
            let allowed =
                locked(&self.composing, "composing limiter").allow(conn, active, Instant::now());
            if !allowed {
                return;
            }
            let Some(actor) = self.core.actor_of(ConnId(conn)) else {
                return;
            };
            let _ = self
                .core
                .link
                .to_server_input(ToServer::Inbound(WireInbound::Composing {
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
            locked(&self.tokens, "token registry").remove_conn(conn);
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
                bridge.tokens.lock().unwrap().conn_tokens.is_empty(),
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
        assert!(
            !bridge
                .tokens
                .lock()
                .unwrap()
                .conn_tokens
                .contains_key(&new_conn.0)
        );
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

    #[tokio::test]
    async fn web_bridge_throttles_flood_exempt_queries_routes_composing_to_input_and_maps_busy() {
        use mush_session::SessionConfig;

        struct ProbeAuth {
            busy_login: bool,
            rate_limited_create: bool,
        }

        impl AuthBackend for ProbeAuth {
            async fn verify(&self, _: &str, _: &str) -> crate::accounts::AuthOutcome {
                crate::accounts::AuthOutcome::BadCredentials
            }
            async fn verify_checked(&self, _: &str, _: &str) -> crate::accounts::AuthCheckOutcome {
                if self.busy_login {
                    crate::accounts::AuthCheckOutcome::Busy
                } else {
                    crate::accounts::AuthCheckOutcome::BadCredentials
                }
            }
            async fn characters(&self, _: &str) -> Vec<crate::accounts::CharSummary> {
                Vec::new()
            }
            async fn create(
                &self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<crate::accounts::CharSummary, CreateErr> {
                if self.rate_limited_create {
                    Err(CreateErr::RateLimited)
                } else {
                    Err(CreateErr::AccountExists)
                }
            }
            async fn create_character(
                &self,
                _: &str,
                _: &str,
            ) -> Result<crate::accounts::CharSummary, CreateErr> {
                Err(CreateErr::NoSuchAccount)
            }
        }

        struct ProbeLink {
            input_msgs: Mutex<Vec<ToServer>>,
            raw_msgs: Mutex<Vec<ToServer>>,
        }

        impl mush_ipc::GatewayLink for ProbeLink {
            fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
                self.raw_msgs.lock().unwrap().push(msg);
                Box::pin(async { true })
            }
            fn to_server_input(&self, msg: ToServer) -> BoxFuture<'_, InputSend> {
                self.input_msgs.lock().unwrap().push(msg);
                Box::pin(async { InputSend::Sent })
            }
            fn request(
                &self,
                _: mush_ipc::ServiceReq,
            ) -> BoxFuture<'_, Option<mush_ipc::ServiceResp>> {
                Box::pin(async { None })
            }
        }

        let sessions = Arc::new(SessionManager::with_config(SessionConfig {
            flood_rate: 0.0001,
            flood_burst: 1.0,
            ..SessionConfig::default()
        }));
        let link = Arc::new(ProbeLink {
            input_msgs: Mutex::new(Vec::new()),
            raw_msgs: Mutex::new(Vec::new()),
        });
        let store: Arc<dyn ObjectStore> = Arc::new(mush_store::InMemoryStore::new());
        let bridge = WebBridge::with_link(
            Arc::new(ProbeAuth {
                busy_login: true,
                rate_limited_create: true,
            }),
            Arc::clone(&sessions),
            link.clone(),
            Arc::new(IpGuard::new()),
            store,
            Vec::new(),
        );

        let ip: IpAddr = "203.0.113.10".parse().unwrap();
        // Busy Argon2 semaphore maps to LoginErr::RateLimited (429).
        assert_eq!(
            bridge.login("nick", "pw", ip).await,
            Err(LoginErr::RateLimited)
        );
        // RateLimited creation maps to RegisterErr::RateLimited (429).
        assert_eq!(
            bridge.register("nick", "secret123", "Nick", ip).await,
            Err(RegisterErr::RateLimited)
        );

        let (tx, mut rx) = mpsc::channel::<Arc<Envelope>>(8);
        let conn = ConnId(77);
        let actor = ObjId(9);
        let _ = sessions.attach(
            ConnectionHandle::new_structured(
                conn,
                ip,
                ClientCaps {
                    kind: ClientKind::Web,
                    color: ColorDepth::TrueColor,
                    width: 0,
                    height: 0,
                    utf8: true,
                    structured: true,
                },
                tx,
            ),
            char_key(actor),
            "nick".to_string(),
        );
        sessions.set_flood_exempt(char_key(actor), true);

        // First query uses the 1-token burst; second MUST be throttled despite flood_exempt.
        bridge.request(conn.0, 1, Query::Who).await;
        bridge.request(conn.0, 2, Query::Who).await;
        assert_eq!(link.input_msgs.lock().unwrap().len(), 1);
        let err_reply = rx.try_recv().expect("throttled query receives error reply");
        assert!(matches!(err_reply.event, OutputEvent::Reply { id: 2, .. }));

        // Composing rides `to_server_input` (NOT `to_server`), and redundant/fast toggle is dropped.
        bridge.composing(conn.0, false).await;
        bridge.composing(conn.0, true).await;
        bridge.composing(conn.0, false).await;
        bridge.composing(conn.0, true).await;
        assert!(
            link.raw_msgs.lock().unwrap().is_empty(),
            "composing must never bypass to_server_input"
        );
        let composing_sent = link
            .input_msgs
            .lock()
            .unwrap()
            .iter()
            .filter(|m| matches!(m, ToServer::Inbound(WireInbound::Composing { .. })))
            .count();
        assert_eq!(composing_sent, 1);
    }

    /// A link that pauses on the first `ConnUp` so tests can deterministically
    /// race `logout` between `authorize_character` and post-attach tracking.
    struct PausingAttachLink {
        conn_up_seen: Arc<Notify>,
        release_conn_up: Arc<Notify>,
        pause_conn_up: std::sync::atomic::AtomicBool,
        sent: Mutex<Vec<ToServer>>,
    }

    impl PausingAttachLink {
        fn new(pause: bool) -> Arc<Self> {
            Arc::new(Self {
                conn_up_seen: Arc::new(Notify::new()),
                release_conn_up: Arc::new(Notify::new()),
                pause_conn_up: std::sync::atomic::AtomicBool::new(pause),
                sent: Mutex::new(Vec::new()),
            })
        }
    }

    impl GatewayLink for PausingAttachLink {
        fn try_reserve_connection(&self, _conn: u64, _actor: ObjId) -> ConnectionAdmission {
            ConnectionAdmission::Reserved
        }

        fn cancel_connection_reservation(&self, _conn: u64) {}

        fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
            let should_pause = matches!(&msg, ToServer::Inbound(WireInbound::ConnUp { .. }))
                && self
                    .pause_conn_up
                    .swap(false, std::sync::atomic::Ordering::AcqRel);
            self.sent.lock().unwrap().push(msg);
            let seen = Arc::clone(&self.conn_up_seen);
            let release = Arc::clone(&self.release_conn_up);
            Box::pin(async move {
                if should_pause {
                    seen.notify_one();
                    release.notified().await;
                }
                true
            })
        }

        fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            let resp = match req {
                ServiceReq::ReplyBarrier { .. } | ServiceReq::Barrier => Some(ServiceResp::Barrier),
                ServiceReq::ConnectionReady { .. } => Some(ServiceResp::ConnectionReady(true)),
                ServiceReq::InInputContext { .. } => Some(ServiceResp::InInputContext(false)),
                _ => None,
            };
            Box::pin(async move { resp })
        }
    }

    /// EXT-P1-3(a): if `logout` lands after `authorize_character` passes and
    /// while `enter_game` is still in flight, post-attach verification must
    /// immediately boot and untrack the newly attached connection instead of
    /// leaving an active untracked game session behind.
    #[tokio::test]
    async fn logout_during_connect_character_toctou_boots_and_refuses() {
        let actor = ObjId(7);
        let token = "toctou-connect-token";
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
        let sessions = Arc::new(SessionManager::default());
        let link = PausingAttachLink::new(true);
        let bridge = Arc::new(WebBridge::with_link(
            Arc::new(ResumeAuth { actor }),
            Arc::clone(&sessions),
            Arc::clone(&link) as Arc<dyn GatewayLink>,
            Arc::new(IpGuard::new()),
            Arc::clone(&store),
            Vec::new(),
        ));

        let (tx, mut rx) = mpsc::channel::<Arc<Envelope>>(16);
        let attach_bridge = Arc::clone(&bridge);
        let attach_task = tokio::spawn(async move {
            attach_bridge
                .connect_character(
                    NewWebConn {
                        id: 41,
                        addr: "127.0.0.1".parse().unwrap(),
                        tx,
                        closer: mush_net_web::ConnCloser::detached(),
                    },
                    token,
                    u64::from(actor.0),
                )
                .await
        });

        // Wait until `connect_character` passed `authorize_character` and is
        // paused mid-`enter_game`, then run `logout` to completion.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            link.conn_up_seen.notified(),
        )
        .await
        .expect("connect_character reached ConnUp");
        assert_eq!(bridge.logout(token).await, Ok(()));

        // Let `enter_game` finish; post-attach verification must boot + refuse.
        link.release_conn_up.notify_one();
        let attach_res = tokio::time::timeout(std::time::Duration::from_secs(5), attach_task)
            .await
            .expect("attach finishes")
            .unwrap();
        assert!(
            matches!(attach_res, Err(AttachErr::Refused(_))),
            "connect_character must refuse a token logged out mid-attach, got {attach_res:?}"
        );
        assert_eq!(
            sessions.connection_count(),
            0,
            "booted connection must not remain attached in SessionManager"
        );
        assert!(
            bridge.tokens.lock().unwrap().conn_tokens.is_empty(),
            "refused connection must not remain tracked in TokenRegistry"
        );
        let sent = link.sent.lock().unwrap();
        assert!(
            sent.iter().any(|m| matches!(
                m,
                ToServer::Inbound(WireInbound::ConnDown {
                    conn: 41,
                    reason: DisconnectKind::Boot,
                    ..
                })
            )),
            "post-attach revocation must send ConnDown(Boot), saw {sent:?}"
        );
        let mut saw_disconnect = false;
        while let Ok(env) = rx.try_recv() {
            if matches!(env.event, OutputEvent::Disconnect { .. }) {
                saw_disconnect = true;
            }
        }
        assert!(saw_disconnect, "client must receive Disconnect notice");
    }

    /// EXT-P1-3(a): the same single-process attach/logout TOCTOU on `resume`
    /// must boot the resumed connection and return `ResumeErr::BadSession`.
    #[tokio::test]
    async fn logout_during_resume_toctou_boots_and_refuses() {
        let actor = ObjId(7);
        let sessions = Arc::new(SessionManager::default());
        let (old_tx, _old_rx) = mpsc::channel::<Arc<Envelope>>(8);
        let old = ConnectionHandle::new_structured(
            ConnId(50),
            "127.0.0.1".parse().unwrap(),
            WebBridge::<ResumeAuth>::web_caps(),
            old_tx,
        );
        let _ = sessions.attach(old, char_key(actor), "acct".to_string());
        assert!(
            sessions
                .detach(ConnId(50), mush_session::DetachReason::Lost)
                .is_some()
        );

        let token = "toctou-resume-token";
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
        let link = PausingAttachLink::new(true);
        let bridge = Arc::new(WebBridge::with_link(
            Arc::new(ResumeAuth { actor }),
            Arc::clone(&sessions),
            Arc::clone(&link) as Arc<dyn GatewayLink>,
            Arc::new(IpGuard::new()),
            Arc::clone(&store),
            Vec::new(),
        ));

        let (tx, _rx) = mpsc::channel::<Arc<Envelope>>(8);
        let resume_bridge = Arc::clone(&bridge);
        let resume_task = tokio::spawn(async move {
            resume_bridge
                .resume(
                    NewWebConn {
                        id: 51,
                        addr: "127.0.0.2".parse().unwrap(),
                        tx,
                        closer: mush_net_web::ConnCloser::detached(),
                    },
                    token,
                    u64::from(actor.0),
                    0,
                )
                .await
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            link.conn_up_seen.notified(),
        )
        .await
        .expect("resume reached ConnUp");
        assert_eq!(bridge.logout(token).await, Ok(()));

        link.release_conn_up.notify_one();
        let resume_res = tokio::time::timeout(std::time::Duration::from_secs(5), resume_task)
            .await
            .expect("resume finishes")
            .unwrap();
        assert!(
            matches!(resume_res, Err(ResumeErr::BadSession)),
            "resume must return BadSession when logged out mid-resume, got {resume_res:?}"
        );
        assert_eq!(sessions.connection_count(), 0);
        assert!(bridge.tokens.lock().unwrap().conn_tokens.is_empty());
        let sent = link.sent.lock().unwrap();
        assert!(
            sent.iter().any(|m| matches!(
                m,
                ToServer::Inbound(WireInbound::ConnDown {
                    conn: 51,
                    reason: DisconnectKind::Boot,
                    ..
                })
            )),
            "post-resume revocation must send ConnDown(Boot), saw {sent:?}"
        );
    }

    /// EXT-P2-4 + EXT-P1-3(b): when durable `store.token_del` fails during
    /// `logout`, `WebHooks::logout` returns `Err(LogoutErr::Unavailable)` (so
    /// HTTP responds `503` instead of a misleading `204`), yet STILL boots
    /// local live connections, records local revocation, and broadcasts
    /// `ToServer::RevokeWebToken` to peer generations.
    #[tokio::test]
    async fn logout_returns_unavailable_when_store_delete_fails_but_still_boots_and_broadcasts() {
        let actor = ObjId(7);
        let token = "durable-delete-fail-token";
        let expected_hash = token_hash(token);
        let now = unix_now_s();
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        store
            .token_put(
                expected_hash.clone(),
                TokenRecord {
                    account: "acct".to_string(),
                    created: now,
                    expires: now + TOKEN_TTL_S,
                },
            )
            .await
            .unwrap();
        let sessions = Arc::new(SessionManager::default());
        let link = PausingAttachLink::new(false);
        let bridge = WebBridge::with_link(
            Arc::new(ResumeAuth { actor }),
            Arc::clone(&sessions),
            Arc::clone(&link) as Arc<dyn GatewayLink>,
            Arc::new(IpGuard::new()),
            Arc::clone(&store),
            Vec::new(),
        );

        let (tx, mut rx) = mpsc::channel::<Arc<Envelope>>(16);
        bridge
            .connect_character(
                NewWebConn {
                    id: 61,
                    addr: "127.0.0.1".parse().unwrap(),
                    tx,
                    closer: mush_net_web::ConnCloser::detached(),
                },
                token,
                u64::from(actor.0),
            )
            .await
            .expect("initial attach succeeds");
        assert_eq!(sessions.connection_count(), 1);

        // Close the store so `token_del` fails with a durable-delete error.
        store.close().await.unwrap();
        assert_eq!(
            bridge.logout(token).await,
            Err(LogoutErr::Unavailable),
            "failed durable token_del must surface LogoutErr::Unavailable"
        );
        assert_eq!(
            sessions.connection_count(),
            0,
            "local live connection must still be booted even when token_del fails"
        );
        assert_eq!(
            bridge.validate_token(token).await,
            TokenVerdict::Refused,
            "locally recorded revocation must refuse the token even without durable delete"
        );
        let sent = link.sent.lock().unwrap();
        assert!(
            sent.iter().any(|m| matches!(
                m,
                ToServer::RevokeWebToken { token_hash } if token_hash == &expected_hash
            )),
            "logout must still broadcast RevokeWebToken to peer generations, saw {sent:?}"
        );
        let mut saw_disconnect = false;
        while let Ok(env) = rx.try_recv() {
            if matches!(env.event, OutputEvent::Disconnect { .. }) {
                saw_disconnect = true;
            }
        }
        assert!(saw_disconnect, "booted connection must receive Disconnect");
    }

    /// P2-EDGE-1: dropping/cancelling `WebBridge::logout` mid-barrier (client
    /// disconnect or HTTP `request_timeout`) must NOT strand live WebSockets,
    /// blind `live_token_hashes()` prior to detach, or suppress peer-generation
    /// `ToServer::RevokeWebToken`, and local boots must run concurrently.
    #[tokio::test]
    async fn p2_edge_1_cancelled_logout_completes_concurrent_boots_and_broadcast() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::Notify;

        struct BlockingBarrierLink {
            block_barriers: std::sync::atomic::AtomicBool,
            barrier_hits: AtomicUsize,
            barrier_entered: Notify,
            release_barriers: Notify,
            sent: Mutex<Vec<ToServer>>,
        }

        impl GatewayLink for BlockingBarrierLink {
            fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
                self.sent.lock().unwrap().push(msg);
                Box::pin(async { true })
            }
            fn request(
                &self,
                req: mush_ipc::ServiceReq,
            ) -> BoxFuture<'_, Option<mush_ipc::ServiceResp>> {
                Box::pin(async move {
                    match req {
                        mush_ipc::ServiceReq::ConnectionReady { .. } => {
                            Some(mush_ipc::ServiceResp::ConnectionReady(true))
                        }
                        mush_ipc::ServiceReq::Barrier => {
                            if self.block_barriers.load(Ordering::SeqCst) {
                                self.barrier_hits.fetch_add(1, Ordering::SeqCst);
                                self.barrier_entered.notify_waiters();
                                self.release_barriers.notified().await;
                            }
                            Some(mush_ipc::ServiceResp::Barrier)
                        }
                        _ => None,
                    }
                })
            }
        }

        let actor = ObjId(9);
        let token = "cancel-safe-logout-token";
        let expected_hash = token_hash(token);
        let now = unix_now_s();
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        store
            .token_put(
                expected_hash.clone(),
                TokenRecord {
                    account: "acct".to_string(),
                    created: now,
                    expires: now + TOKEN_TTL_S,
                },
            )
            .await
            .unwrap();
        let sessions = Arc::new(SessionManager::default());
        let link = Arc::new(BlockingBarrierLink {
            block_barriers: std::sync::atomic::AtomicBool::new(false),
            barrier_hits: AtomicUsize::new(0),
            barrier_entered: Notify::new(),
            release_barriers: Notify::new(),
            sent: Mutex::new(Vec::new()),
        });
        let bridge = WebBridge::with_link(
            Arc::new(ResumeAuth { actor }),
            Arc::clone(&sessions),
            Arc::clone(&link) as Arc<dyn GatewayLink>,
            Arc::new(IpGuard::new()),
            Arc::clone(&store),
            Vec::new(),
        );

        let mut rxs = Vec::new();
        for conn_id in [71u64, 72, 73] {
            let (tx, rx) = mpsc::channel::<Arc<Envelope>>(16);
            bridge
                .connect_character(
                    NewWebConn {
                        id: conn_id,
                        addr: "127.0.0.1".parse().unwrap(),
                        tx,
                        closer: mush_net_web::ConnCloser::detached(),
                    },
                    token,
                    u64::from(actor.0),
                )
                .await
                .expect("attach succeeds");
            rxs.push(rx);
        }
        assert_eq!(sessions.connection_count(), 3);
        link.block_barriers.store(true, Ordering::SeqCst);

        // Start logout, wait until all 3 boot_conn barriers are concurrently
        // in flight, then DROP the logout future (simulating client abort / 408).
        {
            let logout_fut = bridge.logout(token);
            tokio::pin!(logout_fut);
            let wait_all_barriers = async {
                loop {
                    let notified = link.barrier_entered.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if link.barrier_hits.load(Ordering::SeqCst) == 3 {
                        return;
                    }
                    notified.await;
                }
            };
            tokio::select! {
                _ = &mut logout_fut => panic!("logout cannot complete while barriers are blocked"),
                _ = tokio::time::timeout(std::time::Duration::from_secs(2), wait_all_barriers) => {}
            }
            // All 3 boots entered concurrently, RevokeWebToken was ALREADY sent,
            // and conn_tokens is NOT blinded while sessions are still live!
            assert_eq!(link.barrier_hits.load(Ordering::SeqCst), 3);
            assert!(
                link.sent.lock().unwrap().iter().any(|m| matches!(
                    m,
                    ToServer::RevokeWebToken { token_hash } if token_hash == &expected_hash
                )),
                "RevokeWebToken must be sent before awaiting local barriers"
            );
            assert!(
                crate::controls::WebTokenRevoker::live_token_hashes(&bridge)
                    .contains(&expected_hash),
                "conn_tokens must remain visible until sessions actually detach"
            );
            // Drop `logout_fut` right here (simulating HTTP abort / request_timeout)!
        }

        // Release barriers: the detached revocation task must still finish
        // booting every live connection and cleaning up `conn_tokens`.
        link.release_barriers.notify_waiters();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while (sessions.connection_count() > 0
            || !crate::controls::WebTokenRevoker::live_token_hashes(&bridge).is_empty())
            && Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(sessions.connection_count(), 0, "all 3 sessions must detach");
        assert!(
            crate::controls::WebTokenRevoker::live_token_hashes(&bridge).is_empty(),
            "conn_tokens cleaned up after detach"
        );
        assert!(store.token_get(&expected_hash).await.unwrap().is_none());
        for rx in &mut rxs {
            let mut saw_disconnect = false;
            while let Ok(env) = rx.try_recv() {
                if matches!(env.event, OutputEvent::Disconnect { .. }) {
                    saw_disconnect = true;
                }
            }
            assert!(
                saw_disconnect,
                "each booted session must receive Disconnect"
            );
        }
    }
}
