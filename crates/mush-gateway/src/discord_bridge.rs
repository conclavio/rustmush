//! The bridge between the Discord transport (`mush-net-discord`) and the
//! game: the [`mush_net_discord::DiscordHooks`] implementation (D1 identity,
//! D2 follow).
//!
//! ## Division of labor (mirrors [`crate::ssh_bridge`] / [`crate::web_bridge`])
//!
//! - The **transport** owns the Gateway shard, REST, the slash-command
//!   surface, the `/link` attempt limiter and the DM writer's render; it hands
//!   this bridge a Discord-asserted user id and normalized text.
//! - The **bridge** owns everything game-side. For the link (D1): the code is
//!   taken once (`discord_code_take`), the ban and both-sides-free checks, the
//!   reverse-index reservation that is the uniqueness lock, then the account
//!   write. For the follow (D2): a **real connection of the character's own
//!   session** — `ConnectionHandle::new_structured` with
//!   [`ClientKind::Discord`], attached through the shared [`GameBridge`] core
//!   exactly as a web tab is (`enter_game`, `line`, `quit`), so perception,
//!   `@ignore`, the turn gate, flood limits and boots apply unchanged. The DM
//!   is only ever a lossy render of what that connection receives.
//!
//! ## One follow per Discord user (D-D-6)
//!
//! `follows` maps a Discord user to at most one attached connection. A second
//! `/follow` for another character quits the first. The writer task removes
//! the entry when the session drops the handle (quit, boot, the linkdead
//! reaper, shutdown), so the map never outlives its connections.
//!
//! ## Refusals are uniform (R0, 03 §3)
//!
//! Every reason a redeem fails is one [`LinkOutcome::Refused`] and one reply
//! text, so the bot is never an oracle on which accounts exist or are linked.
//! The bridge logs the reason class at `info`, never the code.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_config::discord::link_code_hash;
use mush_config::ipguard::IpGuard;
use mush_config::unix_now_s;
use mush_core::ObjId;
use mush_ipc::GatewayLink;
use mush_net_discord::{
    ApiError, DetachOutcome, DiscordApi, DiscordConfig, DiscordHooks, DiscordRest, DmPolicy,
    FollowOutcome, FollowState, Intents, LineOutcome, LinkOutcome, Outgoing, TwilightEvents,
    UnlinkOutcome, WhoAmI, run_dm_writer,
};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, Envelope, OutputEvent, RichText};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::{AccountRec, CharacterRef, DiscordLinkRec, ObjectStore};
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::accounts::AuthBackend;
use crate::bridge_common::{GameBridge, LineOutcome as CoreLine};
use crate::delivery::{char_key, char_key_obj};

/// Discord connection ids start here: disjoint from SSH (allocated from 1)
/// and web (`WEB_CONN_ID_BASE = 1 << 32`), so a follow can never collide with
/// a terminal in the shared session table.
pub const DISCORD_CONN_ID_BASE: u64 = 1 << 48;

/// The structured sink's capacity for one follow. The DM writer drains it
/// eagerly into its own bounded buffer, so it never fills under a burst.
pub const DM_SINK_CAPACITY: usize = 256;

/// How often idle and deaf follows are swept.
pub const IDLE_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// The running bot task, or the absence of one. Both binaries hold one and
/// call [`DiscordTask::stop`] in their shutdown tails.
pub struct DiscordTask {
    stop: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
    sweeper: Option<JoinHandle<()>>,
}

impl DiscordTask {
    /// No bot (disabled, misconfigured, or no token).
    #[must_use]
    pub fn none() -> DiscordTask {
        DiscordTask {
            stop: None,
            join: None,
            sweeper: None,
        }
    }

    /// Whether a bot task was started.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.join.is_some()
    }

    /// Stop the bot gracefully (a no-op when none started): fire its stop
    /// signal and wait a bounded time. Follows are not detached here — the
    /// world task's shutdown notice reaches their sessions like every other
    /// connection's, and the process is ending.
    pub async fn stop(self) {
        if let Some(sweeper) = self.sweeper {
            sweeper.abort();
        }
        if let Some(stop) = self.stop {
            let _ = stop.send(());
        }
        if let Some(join) = self.join
            && tokio::time::timeout(Duration::from_secs(5), join)
                .await
                .is_err()
        {
            tracing::warn!("discord bot did not stop in time; continuing shutdown");
        }
    }
}

/// Start the Discord bot from `[discord]` if it is enabled and usable: the
/// [`DiscordBridge`] over the same accounts, session manager, boundary link,
/// guard and store the SSH and web bridges share, a [`DiscordRest`] client,
/// and one Gateway shard. Every failure to start is **loud but non-fatal**
/// (the web listener's W4 rule): SSH and web players keep playing while the
/// operator fixes the config or the environment.
pub fn spawn_bot<A: AuthBackend + 'static>(
    config: &Config,
    accounts: Arc<A>,
    sessions: Arc<SessionManager>,
    link: Arc<dyn GatewayLink>,
    guard: Arc<IpGuard>,
    store: Arc<dyn ObjectStore>,
) -> DiscordTask {
    let mut dc = match DiscordConfig::deserialize(config.discord.clone()) {
        Ok(dc) => dc,
        Err(e) => {
            tracing::error!(error = %e, "[discord] config invalid; Discord disabled");
            return DiscordTask::none();
        }
    };
    if !dc.enabled {
        return DiscordTask::none();
    }
    // Ids the file leaves at 0 come from the environment, beside the token.
    dc.resolve_ids_from_env();
    if let Err(e) = dc.validate() {
        tracing::error!(error = %e, "[discord] config incomplete; Discord disabled");
        return DiscordTask::none();
    }
    let Some(token) = dc.resolve_token() else {
        tracing::error!(
            env = %dc.token_env,
            "[discord] enabled but the token environment variable is unset; Discord disabled"
        );
        return DiscordTask::none();
    };
    let api: Arc<dyn DiscordApi> = Arc::new(DiscordRest::new(token.clone(), dc.application_id));
    // D2 needs DIRECT_MESSAGES for the DM lines. The content of a DM to the
    // bot is delivered without the privileged Message Content intent (that
    // gates guild messages — D4's relay). Slash commands arrive regardless.
    let events = Box::new(TwilightEvents::new(token, Intents::DIRECT_MESSAGES));
    let bridge = Arc::new(DiscordBridge::with_link(
        accounts,
        sessions,
        link,
        guard,
        store,
        Arc::clone(&api),
        &dc,
    ));
    bridge.install_auto_follow();
    let sweeper = tokio::spawn(idle_sweeper(Arc::clone(&bridge)));
    let hooks: Arc<dyn DiscordHooks> = bridge;
    let (stop, stop_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        if let Err(e) = mush_net_discord::serve(hooks, api, events, dc, async move {
            let _ = stop_rx.await;
        })
        .await
        {
            tracing::error!(error = %e, "discord bot stopped; the game continues without it");
        }
    });
    tracing::info!("discord bot starting");
    DiscordTask {
        stop: Some(stop),
        join: Some(join),
        sweeper: Some(sweeper),
    }
}

/// The capabilities a follow connection declares: structured (it receives
/// envelopes), colorless, UTF-8, a nominal 80×24 (nothing on a structured
/// sink pages by height, but a zero would be a surprise to anything that
/// divides by it).
fn discord_caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Discord,
        color: ColorDepth::None,
        width: 80,
        height: 24,
        utf8: true,
        structured: true,
    }
}

/// One attached follow.
struct Follow {
    /// The connection in the shared session table.
    conn: ConnId,
    /// The followed character.
    character: ObjId,
    /// Its name, for replies.
    name: String,
    /// What the writer has learned (scene membership, quiet, a closed DM).
    state: Arc<FollowState>,
    /// The last DM line, for the idle rule.
    last_input: Instant,
}

/// The production [`DiscordHooks`]: the shared [`GameBridge`] core (the
/// follow connections), the store (the link records) and the REST client
/// (DM channels and the writers' sends).
pub struct DiscordBridge<A: AuthBackend + 'static> {
    core: GameBridge<A>,
    store: Arc<dyn ObjectStore>,
    api: Arc<dyn DiscordApi>,
    policy: DmPolicy,
    /// The `auto_follow` a new link starts with (`[discord].auto_follow_default`).
    auto_follow_default: bool,
    /// Detach a follow idle this long and not in a scene; `None` never.
    idle_detach: Option<Duration>,
    /// Discord user → the one follow (D-D-6). Never held across an await.
    follows: Arc<Mutex<HashMap<u64, Follow>>>,
    next_conn: AtomicU64,
}

impl<A: AuthBackend + 'static> DiscordBridge<A> {
    /// Wire the bridge over the shared seams and the transport's REST client.
    #[must_use]
    pub fn with_link(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        link: Arc<dyn GatewayLink>,
        guard: Arc<IpGuard>,
        store: Arc<dyn ObjectStore>,
        api: Arc<dyn DiscordApi>,
        config: &DiscordConfig,
    ) -> DiscordBridge<A> {
        DiscordBridge {
            core: GameBridge::with_link(accounts, sessions, link, guard),
            store,
            api,
            policy: DmPolicy::new(
                config.dm_coalesce_ms,
                config.dm_max_chars,
                config.dm_queue_lines,
            ),
            auto_follow_default: config.auto_follow_default,
            idle_detach: (config.follow_idle_detach_s > 0)
                .then(|| Duration::from_secs(config.follow_idle_detach_s)),
            follows: Arc::new(Mutex::new(HashMap::new())),
            next_conn: AtomicU64::new(DISCORD_CONN_ID_BASE),
        }
    }

    /// The character `user_id` is following, if any.
    #[must_use]
    pub fn following(&self, user_id: u64) -> Option<(ObjId, String)> {
        self.follows
            .lock()
            .expect("follows poisoned")
            .get(&user_id)
            .map(|f| (f.character, f.name.clone()))
    }

    /// Watch the shared session manager for terminals going linkdead and
    /// attach a follow for accounts that asked for it (D-D-5). Holds the
    /// bridge weakly so the manager never keeps it alive.
    pub fn install_auto_follow(self: &Arc<Self>) {
        let weak: Weak<Self> = Arc::downgrade(self);
        self.core
            .sessions
            .set_linkdead_hook(Arc::new(move |character, _grace| {
                let Some(bridge) = weak.upgrade() else {
                    return;
                };
                // The hook runs inside the detaching task; the follow itself is
                // store lookups and a world round trip, so it gets its own task.
                let Ok(rt) = tokio::runtime::Handle::try_current() else {
                    return;
                };
                rt.spawn(async move {
                    bridge.auto_follow(char_key_obj(character)).await;
                });
            }));
    }

    /// Tell every connected character of `account` something about its
    /// Discord link (a `Sys` line tagged `discord`, to their own connections
    /// only). A character with no live session is simply not told.
    fn notify_account(&self, account: &AccountRec, text: &str) {
        for character in &account.characters {
            let key = char_key(character.id);
            if self.core.sessions.session_of_char(key).is_some() {
                let _ = self.core.sessions.send_to_char(
                    key,
                    Arc::new(OutputEvent::Sys(RichText::tag("discord", text))),
                );
            }
        }
    }

    /// The link flow, with the reason class for the log. `Err` is a refusal.
    async fn redeem(&self, user_id: u64, code: &str) -> Result<AccountRec, &'static str> {
        let hash = link_code_hash(code);
        let Some(pending) = self
            .store
            .discord_code_take(&hash)
            .await
            .map_err(|_| "store")?
        else {
            return Err("no such code");
        };
        if self
            .store
            .discord_ban_get(user_id)
            .await
            .map_err(|_| "store")?
            .is_some()
        {
            return Err("banned user");
        }
        let Some(account) = self
            .store
            .get_account(&pending.account)
            .await
            .map_err(|_| "store")?
        else {
            return Err("account gone");
        };
        if account.discord.is_some() {
            return Err("account already linked");
        }
        if self
            .store
            .account_by_discord_user(user_id)
            .await
            .map_err(|_| "store")?
            .is_some()
        {
            return Err("user already linked");
        }
        // The reservation is the lock: exactly one account may hold a user.
        if !self
            .store
            .discord_user_reserve(user_id, &account.name)
            .await
            .map_err(|_| "store")?
        {
            return Err("user reserved elsewhere");
        }
        let link = DiscordLinkRec {
            user_id,
            linked_at: unix_now_s(),
            relay_character: None,
            auto_follow: self.auto_follow_default,
            dm_channel: None,
            quiet: false,
        };
        match self
            .store
            .set_account_discord(&account.name, Some(&link))
            .await
        {
            Ok(true) => {}
            _ => {
                // Undo the reservation so the user is not stuck half-linked.
                let _ = self.store.discord_user_release(user_id).await;
                return Err("account write failed");
            }
        }
        Ok(account)
    }

    /// Which of the account's characters a follow attaches: the named one;
    /// else the only one; else the relay character; else the caller must say.
    fn pick_character<'a>(
        account: &'a AccountRec,
        link: &DiscordLinkRec,
        wanted: Option<&str>,
    ) -> Result<&'a CharacterRef, FollowOutcome> {
        let available = || {
            account
                .characters
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        if account.characters.is_empty() {
            return Err(FollowOutcome::Failed(
                "your account has no characters yet".to_string(),
            ));
        }
        if let Some(name) = wanted.map(str::trim).filter(|n| !n.is_empty()) {
            return account
                .characters
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| FollowOutcome::WhichCharacter {
                    available: available(),
                });
        }
        if let [only] = account.characters.as_slice() {
            return Ok(only);
        }
        if let Some(relay) = link.relay_character
            && let Some(c) = account.characters.iter().find(|c| c.id == relay)
        {
            return Ok(c);
        }
        Err(FollowOutcome::WhichCharacter {
            available: available(),
        })
    }

    /// Attach a follow (the `/follow` and auto-follow paths). `intro`, when
    /// given, is sent to the DM ahead of the connection's own first output.
    async fn follow_impl(
        &self,
        user_id: u64,
        wanted: Option<&str>,
        intro: Option<&str>,
    ) -> FollowOutcome {
        let account = match self.store.account_by_discord_user(user_id).await {
            Ok(Some(a)) => a,
            Ok(None) => return FollowOutcome::NotLinked,
            Err(e) => {
                tracing::warn!(error = %e, "discord follow: account lookup failed");
                return FollowOutcome::Failed("the account store is unavailable".to_string());
            }
        };
        let Some(link) = account.discord.clone() else {
            return FollowOutcome::NotLinked;
        };
        match self.store.discord_ban_get(user_id).await {
            Ok(None) => {}
            Ok(Some(_)) => return FollowOutcome::Banned,
            Err(e) => {
                tracing::warn!(error = %e, "discord follow: ban lookup failed");
                return FollowOutcome::Failed("the account store is unavailable".to_string());
            }
        }
        let chosen = match Self::pick_character(&account, &link, wanted) {
            Ok(c) => c.clone(),
            Err(outcome) => return outcome,
        };
        // One follow per user: the same character is a no-op, another one
        // replaces the first (a clean QUIT of the old connection).
        let previous = {
            let mut follows = self.follows.lock().expect("follows poisoned");
            match follows.get(&user_id) {
                Some(cur) if cur.character == chosen.id => {
                    return FollowOutcome::Following {
                        character: cur.name.clone(),
                        attached: false,
                    };
                }
                Some(_) => follows.remove(&user_id),
                None => None,
            }
        };
        if let Some(prev) = previous {
            self.core.quit(prev.conn).await;
        }
        // The DM channel: `create_dm` is idempotent (Discord returns the
        // existing channel), and it is the one call that tells us the user's
        // DMs are closed before anything is attached.
        let channel = match self.api.create_dm(user_id).await {
            Ok(c) => c,
            Err(ApiError::CannotDm) => return FollowOutcome::DmsClosed,
            Err(e) => {
                tracing::warn!(error = %e, "discord follow: create_dm failed");
                return FollowOutcome::Failed("Discord did not open the conversation".to_string());
            }
        };
        if link.dm_channel != Some(channel) {
            let mut updated = link.clone();
            updated.dm_channel = Some(channel);
            // Informational (the on-call outbox reads it); a failure here
            // does not stop the follow.
            let _ = self
                .store
                .set_account_discord(&account.name, Some(&updated))
                .await;
        }
        let conn = ConnId(self.next_conn.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel::<Arc<Envelope>>(DM_SINK_CAPACITY);
        let handle = ConnectionHandle::new_structured(
            conn,
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            discord_caps(),
            tx,
        );
        let state = FollowState::new(link.quiet, chosen.id.to_string());
        {
            let mut follows = self.follows.lock().expect("follows poisoned");
            if let Some(cur) = follows.get(&user_id) {
                // Raced with an auto-follow for the same user: keep theirs.
                return FollowOutcome::Following {
                    character: cur.name.clone(),
                    attached: false,
                };
            }
            follows.insert(
                user_id,
                Follow {
                    conn,
                    character: chosen.id,
                    name: chosen.name.clone(),
                    state: Arc::clone(&state),
                    last_input: Instant::now(),
                },
            );
        }
        // The writer lives as long as the session holds the handle; when the
        // session drops it (quit, boot, reaper, shutdown) the writer flushes,
        // ends, and forgets the follow if it is still this connection.
        let follows = Arc::clone(&self.follows);
        tokio::spawn({
            let api = Arc::clone(&self.api);
            let policy = self.policy;
            let state = Arc::clone(&state);
            async move {
                run_dm_writer(api, channel, rx, policy, state).await;
                let mut follows = follows.lock().expect("follows poisoned");
                if follows.get(&user_id).is_some_and(|f| f.conn == conn) {
                    follows.remove(&user_id);
                }
            }
        });
        if let Some(text) = intro
            && let Err(e) = self.api.send(channel, Outgoing::plain(text)).await
        {
            tracing::debug!(error = %e, "discord follow: intro not delivered");
        }
        match self
            .core
            .enter_game(handle, chosen.id, &account.name, &chosen.name, true)
            .await
        {
            Ok(_) => {
                tracing::info!(conn = conn.0, actor = %chosen.id, "discord follow attached");
                FollowOutcome::Following {
                    character: chosen.name,
                    attached: true,
                }
            }
            Err(e) => {
                // `enter_game` rolled the attach back and dropped the handle;
                // the writer is ending on its own. Forget the follow now.
                let mut follows = self.follows.lock().expect("follows poisoned");
                if follows.get(&user_id).is_some_and(|f| f.conn == conn) {
                    follows.remove(&user_id);
                }
                tracing::info!(error = %e, "discord follow refused");
                FollowOutcome::Failed(e.to_string())
            }
        }
    }

    /// The auto-follow path (D-D-5): `actor`'s last terminal went linkdead.
    async fn auto_follow(&self, actor: ObjId) {
        // The reverse lookup is a scan, like `@discord`'s; a per-character
        // index is the fix if linkdead events ever get frequent.
        let Ok(accounts) = self.store.list_accounts().await else {
            return;
        };
        let Some(account) = accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == actor))
        else {
            return;
        };
        let Some(link) = account.discord.as_ref() else {
            return;
        };
        if !link.auto_follow {
            return;
        }
        let user_id = link.user_id;
        if self
            .follows
            .lock()
            .expect("follows poisoned")
            .contains_key(&user_id)
        {
            // Already following (this or another character); never yank it.
            return;
        }
        let Some(name) = account
            .characters
            .iter()
            .find(|c| c.id == actor)
            .map(|c| c.name.clone())
        else {
            return;
        };
        match self
            .follow_impl(
                user_id,
                Some(&name),
                Some(
                    "Your terminal dropped; following here so the scene does not lose you. \
                     `/detach` stops this, or just reconnect your terminal.",
                ),
            )
            .await
        {
            FollowOutcome::Following { .. } => {
                tracing::info!(actor = %actor, "discord auto-follow attached");
            }
            FollowOutcome::DmsClosed => {
                tracing::info!(actor = %actor, "discord auto-follow: DMs closed; nothing attached");
            }
            other => {
                tracing::debug!(actor = %actor, ?other, "discord auto-follow did not attach");
            }
        }
    }

    /// One pass of the idle rule: detach follows deaf (closed DMs) or idle
    /// past the threshold and not in a scene. Returns the characters detached.
    pub async fn sweep_idle(&self, now: Instant) -> Vec<String> {
        let due: Vec<(u64, ConnId, String, bool)> = {
            let follows = self.follows.lock().expect("follows poisoned");
            follows
                .iter()
                .filter_map(|(user, f)| {
                    let deaf = f.state.dm_closed.load(Ordering::Relaxed);
                    let idle = self.idle_detach.is_some_and(|limit| {
                        now.saturating_duration_since(f.last_input) >= limit
                            && !f.state.in_scene.load(Ordering::Relaxed)
                    });
                    (deaf || idle).then(|| (*user, f.conn, f.name.clone(), deaf))
                })
                .collect()
        };
        let mut detached = Vec::new();
        for (user, conn, name, deaf) in due {
            let removed = {
                let mut follows = self.follows.lock().expect("follows poisoned");
                follows
                    .get(&user)
                    .is_some_and(|f| f.conn == conn)
                    .then(|| follows.remove(&user))
                    .flatten()
            };
            if removed.is_none() {
                continue;
            }
            if !deaf {
                let _ = self.core.sessions.send_to_conn(
                    conn,
                    Arc::new(OutputEvent::Sys(RichText::tag(
                        "discord",
                        "Detached: no Discord input for a while and no scene to hold you. \
                         `/follow` returns you.",
                    ))),
                );
            }
            tracing::info!(
                conn = conn.0,
                deaf,
                "discord follow detached by the idle sweep"
            );
            self.core.quit(conn).await;
            detached.push(name);
        }
        detached
    }
}

/// The periodic idle sweep ([`IDLE_SWEEP_INTERVAL`]).
async fn idle_sweeper<A: AuthBackend + 'static>(bridge: Arc<DiscordBridge<A>>) {
    let mut tick = tokio::time::interval(IDLE_SWEEP_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        bridge.sweep_idle(Instant::now()).await;
    }
}

impl<A: AuthBackend + 'static> DiscordHooks for DiscordBridge<A> {
    fn link_redeem<'a>(&'a self, user_id: u64, code: &'a str) -> BoxFuture<'a, LinkOutcome> {
        Box::pin(async move {
            match self.redeem(user_id, code).await {
                Ok(account) => {
                    tracing::info!(account = %account.name, "discord: account linked");
                    self.notify_account(
                        &account,
                        "Discord: your account is now linked to a Discord user (`@discord` shows \
                         the link; `@discord/unlink` removes it).",
                    );
                    LinkOutcome::Linked {
                        account: account.name,
                    }
                }
                Err(reason) => {
                    // The reason class is operational; the code and the user
                    // are not logged.
                    tracing::info!(reason, "discord: link refused");
                    LinkOutcome::Refused
                }
            }
        })
    }

    fn unlink(&self, user_id: u64) -> BoxFuture<'_, UnlinkOutcome> {
        Box::pin(async move {
            let Ok(Some(account)) = self.store.account_by_discord_user(user_id).await else {
                return UnlinkOutcome::NotLinked;
            };
            if !matches!(
                self.store.set_account_discord(&account.name, None).await,
                Ok(true)
            ) {
                return UnlinkOutcome::NotLinked;
            }
            let _ = self.store.discord_user_release(user_id).await;
            // An unlinked user has no business following.
            let follow = self
                .follows
                .lock()
                .expect("follows poisoned")
                .remove(&user_id);
            if let Some(f) = follow {
                self.core.quit(f.conn).await;
            }
            tracing::info!(account = %account.name, "discord: account unlinked (from Discord)");
            self.notify_account(
                &account,
                "Discord: your account's Discord link was removed (from Discord).",
            );
            UnlinkOutcome::Unlinked {
                account: account.name,
            }
        })
    }

    fn whoami(&self, user_id: u64) -> BoxFuture<'_, Option<WhoAmI>> {
        Box::pin(async move {
            let account = self.store.account_by_discord_user(user_id).await.ok()??;
            let link = account.discord.as_ref()?;
            Some(WhoAmI {
                account: account.name.clone(),
                linked_at: link.linked_at,
                auto_follow: link.auto_follow,
            })
        })
    }

    fn follow<'a>(
        &'a self,
        user_id: u64,
        character: Option<&'a str>,
    ) -> BoxFuture<'a, FollowOutcome> {
        Box::pin(async move { self.follow_impl(user_id, character, None).await })
    }

    fn detach(&self, user_id: u64) -> BoxFuture<'_, DetachOutcome> {
        Box::pin(async move {
            let follow = self
                .follows
                .lock()
                .expect("follows poisoned")
                .remove(&user_id);
            match follow {
                None => DetachOutcome::NotFollowing,
                Some(f) => {
                    self.core.quit(f.conn).await;
                    DetachOutcome::Detached { character: f.name }
                }
            }
        })
    }

    fn dm_line<'a>(&'a self, user_id: u64, line: &'a str) -> BoxFuture<'a, LineOutcome> {
        Box::pin(async move {
            let conn = {
                let mut follows = self.follows.lock().expect("follows poisoned");
                match follows.get_mut(&user_id) {
                    Some(f) => {
                        f.last_input = Instant::now();
                        f.conn
                    }
                    None => return LineOutcome::NotFollowing,
                }
            };
            tracing::debug!(conn = conn.0, "discord dm line accepted for the follow");
            match self.core.line(conn, line).await {
                CoreLine::Continue => LineOutcome::Accepted,
                CoreLine::Close => {
                    let mut follows = self.follows.lock().expect("follows poisoned");
                    if follows.get(&user_id).is_some_and(|f| f.conn == conn) {
                        follows.remove(&user_id);
                    }
                    LineOutcome::Closed
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_config::discord::normalize_link_code;
    use mush_ipc::{ServiceReq, ServiceResp, ToServer, WireInbound};
    use mush_net_discord::testing::{Call, FakeApi};
    use mush_proto::{NarrativeKind, ObjRef};
    use mush_session::SessionConfig;
    use mush_store::{DiscordBanRec, DiscordCodeRecord, InMemoryStore};

    const USER: u64 = 123_456_789_012_345_678;
    const OTHER: u64 = 987_654_321_098_765_432;
    const NICKY: ObjId = ObjId(7);
    const ALT: ObjId = ObjId(8);

    /// A world link that admits everything and records what it was sent.
    #[derive(Default)]
    struct AutoLink {
        sent: Mutex<Vec<ToServer>>,
    }

    impl AutoLink {
        fn sent(&self) -> Vec<ToServer> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl GatewayLink for AutoLink {
        fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
            self.sent.lock().unwrap().push(msg);
            Box::pin(async { true })
        }
        fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
            Box::pin(async move {
                Some(match req {
                    ServiceReq::Barrier => ServiceResp::Barrier,
                    ServiceReq::ConnectionReady { .. } => ServiceResp::ConnectionReady(true),
                    ServiceReq::InInputContext { .. } => ServiceResp::InInputContext(false),
                    _ => ServiceResp::Unavailable,
                })
            })
        }
    }

    /// The bridge never touches the auth backend; every method is a stub.
    struct FakeAuth;
    impl AuthBackend for FakeAuth {
        async fn verify(&self, _a: &str, _p: &str) -> crate::accounts::AuthOutcome {
            crate::accounts::AuthOutcome::BadCredentials
        }
        async fn characters(&self, _a: &str) -> Vec<crate::accounts::CharSummary> {
            Vec::new()
        }
        async fn create(
            &self,
            _a: &str,
            _p: &str,
            _c: &str,
        ) -> Result<crate::accounts::CharSummary, crate::accounts::CreateErr> {
            Err(crate::accounts::CreateErr::Io("test".into()))
        }
        async fn create_character(
            &self,
            _a: &str,
            _c: &str,
        ) -> Result<crate::accounts::CharSummary, crate::accounts::CreateErr> {
            Err(crate::accounts::CreateErr::Io("test".into()))
        }
    }

    async fn store_with(name: &str) -> Arc<InMemoryStore> {
        let store = Arc::new(InMemoryStore::new());
        store
            .create_account(AccountRec {
                id: None,
                name: name.to_string(),
                argon2_hash: "$argon2id$x".to_string(),
                created: 1,
                characters: vec![],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .unwrap();
        store.link_character(name, NICKY, "Nicky").await.unwrap();
        store
    }

    async fn mint(store: &InMemoryStore, account: &str, code: &str, expires: i64) -> String {
        let normalized = normalize_link_code(code).unwrap();
        store
            .discord_code_put(
                link_code_hash(&normalized),
                DiscordCodeRecord {
                    account: account.to_string(),
                    expires,
                },
            )
            .await
            .unwrap();
        normalized
    }

    async fn linked(
        store: &InMemoryStore,
        account: &str,
        user: u64,
        edit: impl FnOnce(&mut DiscordLinkRec),
    ) {
        let mut link = DiscordLinkRec {
            user_id: user,
            linked_at: 5,
            relay_character: None,
            auto_follow: false,
            dm_channel: None,
            quiet: false,
        };
        edit(&mut link);
        assert!(store.discord_user_reserve(user, account).await.unwrap());
        assert!(
            store
                .set_account_discord(account, Some(&link))
                .await
                .unwrap()
        );
    }

    struct Rig {
        bridge: Arc<DiscordBridge<FakeAuth>>,
        store: Arc<InMemoryStore>,
        api: Arc<FakeApi>,
        link: Arc<AutoLink>,
        sessions: Arc<SessionManager>,
    }

    async fn rig(edit: impl FnOnce(&mut DiscordConfig)) -> Rig {
        let store = store_with("Nick").await;
        let api = FakeApi::new();
        let link = Arc::new(AutoLink::default());
        let sessions = Arc::new(SessionManager::with_config(SessionConfig::default()));
        let mut config = DiscordConfig {
            dm_coalesce_ms: 50,
            ..DiscordConfig::default()
        };
        edit(&mut config);
        let bridge = Arc::new(DiscordBridge::with_link(
            Arc::new(FakeAuth),
            Arc::clone(&sessions),
            Arc::clone(&link) as Arc<dyn GatewayLink>,
            Arc::new(IpGuard::new()),
            Arc::clone(&store) as Arc<dyn ObjectStore>,
            Arc::clone(&api) as Arc<dyn DiscordApi>,
            &config,
        ));
        Rig {
            bridge,
            store,
            api,
            link,
            sessions,
        }
    }

    fn dm_sends(api: &FakeApi) -> Vec<String> {
        api.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send { msg, .. } => Some(msg.content),
                _ => None,
            })
            .collect()
    }

    fn conn_ups(link: &AutoLink) -> Vec<(u64, ObjId, bool, bool)> {
        link.sent()
            .into_iter()
            .filter_map(|m| match m {
                ToServer::Inbound(WireInbound::ConnUp {
                    conn,
                    actor,
                    caps_structured,
                    from_linkdead,
                }) => Some((conn, actor, caps_structured, from_linkdead)),
                _ => None,
            })
            .collect()
    }

    fn cmds(link: &AutoLink) -> Vec<(ObjId, String)> {
        link.sent()
            .into_iter()
            .filter_map(|m| match m {
                ToServer::Inbound(WireInbound::Cmd { actor, line, .. }) => Some((actor, line)),
                _ => None,
            })
            .collect()
    }

    fn quits(link: &AutoLink) -> Vec<ObjId> {
        link.sent()
            .into_iter()
            .filter_map(|m| match m {
                ToServer::Inbound(WireInbound::ConnDown {
                    actor,
                    reason: mush_ipc::DisconnectKind::Quit,
                    ..
                }) => Some(actor),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_minted_code_links_once_and_whoami_and_unlink_follow() {
        let r = rig(|_| {}).await;
        let code = mint(&r.store, "Nick", "abcd-efgh", unix_now_s() + 600).await;
        let b = &r.bridge;
        assert_eq!(
            b.link_redeem(USER, &code).await,
            LinkOutcome::Linked {
                account: "Nick".into()
            }
        );
        let rec = r.store.get_account("nick").await.unwrap().unwrap();
        let link = rec.discord.as_ref().expect("linked");
        assert_eq!(link.user_id, USER);
        assert!(!link.auto_follow, "the configured default applies");
        assert_eq!(
            r.store
                .account_by_discord_user(USER)
                .await
                .unwrap()
                .unwrap()
                .name,
            "Nick"
        );
        // The code is consumed: a second redeem is refused.
        assert_eq!(b.link_redeem(OTHER, &code).await, LinkOutcome::Refused);
        let who = b.whoami(USER).await.expect("linked");
        assert_eq!(who.account, "Nick");
        assert!(b.whoami(OTHER).await.is_none());
        assert_eq!(
            b.unlink(USER).await,
            UnlinkOutcome::Unlinked {
                account: "Nick".into()
            }
        );
        assert_eq!(b.unlink(USER).await, UnlinkOutcome::NotLinked);
        assert!(
            r.store
                .get_account("nick")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
        assert!(
            r.store
                .account_by_discord_user(USER)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn refusals_are_uniform_expired_banned_already_linked_either_side() {
        let r = rig(|_| {}).await;
        let b = &r.bridge;
        // Unknown and expired codes.
        assert_eq!(b.link_redeem(USER, "ABCDEFGH").await, LinkOutcome::Refused);
        let dead = mint(&r.store, "Nick", "ABCD-EFGJ", unix_now_s() - 1).await;
        assert_eq!(b.link_redeem(USER, &dead).await, LinkOutcome::Refused);
        // A banned user is refused even with a good code (and the code is spent).
        r.store
            .discord_ban_put(DiscordBanRec {
                user_id: USER,
                by_account: "wiz".into(),
                reason: "spam".into(),
                at: 1,
            })
            .await
            .unwrap();
        let good = mint(&r.store, "Nick", "ABCD-EFGK", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(USER, &good).await, LinkOutcome::Refused);
        assert!(
            r.store
                .get_account("nick")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
        r.store.discord_ban_del(USER).await.unwrap();
        // Link, then: the account is taken for OTHER, and USER is taken for a
        // second account.
        let good = mint(&r.store, "Nick", "ABCD-EFGL", unix_now_s() + 600).await;
        assert!(matches!(
            b.link_redeem(USER, &good).await,
            LinkOutcome::Linked { .. }
        ));
        let again = mint(&r.store, "Nick", "ABCD-EFGM", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(OTHER, &again).await, LinkOutcome::Refused);
        r.store
            .create_account(AccountRec {
                id: None,
                name: "Zoe".to_string(),
                argon2_hash: "$argon2id$x".to_string(),
                created: 1,
                characters: vec![],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .unwrap();
        let zoe = mint(&r.store, "Zoe", "ABCD-EFGN", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(USER, &zoe).await, LinkOutcome::Refused);
        assert!(
            r.store
                .get_account("zoe")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
    }

    #[tokio::test]
    async fn follow_attaches_a_structured_connection_of_the_characters_session() {
        let r = rig(|_| {}).await;
        r.store.link_character("Nick", ALT, "Alt").await.unwrap();
        linked(&r.store, "Nick", USER, |_| {}).await;
        let b = &r.bridge;
        // Two characters, none named: the caller must choose.
        assert_eq!(
            b.follow(USER, None).await,
            FollowOutcome::WhichCharacter {
                available: vec!["Nicky".into(), "Alt".into()]
            }
        );
        assert_eq!(
            b.follow(USER, Some("Nobody")).await,
            FollowOutcome::WhichCharacter {
                available: vec!["Nicky".into(), "Alt".into()]
            }
        );
        assert!(conn_ups(&r.link).is_empty(), "nothing attached yet");
        // The relay character breaks the tie.
        linked(&r.store, "Nick", OTHER, |_| {}).await; // (a second account is not needed; reset below)
        r.store.set_account_discord("Nick", None).await.unwrap();
        r.store.discord_user_release(OTHER).await.unwrap();
        linked(&r.store, "Nick", USER, |l| l.relay_character = Some(ALT)).await;
        assert_eq!(
            b.follow(USER, None).await,
            FollowOutcome::Following {
                character: "Alt".into(),
                attached: true
            }
        );
        let ups = conn_ups(&r.link);
        assert_eq!(ups.len(), 1);
        assert_eq!(ups[0].1, ALT);
        assert!(ups[0].2, "a follow is a structured connection");
        assert!(!ups[0].3);
        assert!(ups[0].0 >= DISCORD_CONN_ID_BASE, "its own id space");
        assert!(r.sessions.session_of_char(char_key(ALT)).is_some());
        assert!(
            r.api
                .calls()
                .iter()
                .any(|c| matches!(c, Call::CreateDm { user_id: USER }))
        );
        assert!(
            r.store
                .get_account("nick")
                .await
                .unwrap()
                .unwrap()
                .discord
                .unwrap()
                .dm_channel
                .is_some(),
            "the DM channel is remembered"
        );
        // The same character again is a no-op; another replaces it cleanly.
        assert_eq!(
            b.follow(USER, Some("alt")).await,
            FollowOutcome::Following {
                character: "Alt".into(),
                attached: false
            }
        );
        assert_eq!(
            b.follow(USER, Some("Nicky")).await,
            FollowOutcome::Following {
                character: "Nicky".into(),
                attached: true
            }
        );
        assert_eq!(quits(&r.link), vec![ALT], "the first follow quit cleanly");
        assert!(r.sessions.session_of_char(char_key(ALT)).is_none());
        assert_eq!(b.following(USER), Some((NICKY, "Nicky".to_string())));
        // A DM line runs as the character; the enter_game `look` came first.
        assert_eq!(b.dm_line(USER, "say hi").await, LineOutcome::Accepted);
        let lines = cmds(&r.link);
        assert_eq!(lines.last(), Some(&(NICKY, "say hi".to_string())));
        // What the session delivers to the character reaches the DM, rendered.
        r.sessions.send_to_char(
            char_key(NICKY),
            Arc::new(OutputEvent::Narrative {
                kind: NarrativeKind::Say,
                actor: Some(ObjRef::new("#9", "Zoe")),
                text: RichText::plain("welcome back"),
            }),
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
        let sends = dm_sends(&r.api);
        assert!(
            sends.iter().any(|s| s.contains("Following **Nicky**")),
            "{sends:?}"
        );
        assert!(
            sends
                .iter()
                .any(|s| s.contains("Zoe says, \"welcome back\"")),
            "{sends:?}"
        );
        // Detach quits the connection; the writer says goodbye and forgets.
        assert_eq!(
            b.detach(USER).await,
            DetachOutcome::Detached {
                character: "Nicky".into()
            }
        );
        assert_eq!(quits(&r.link), vec![ALT, NICKY]);
        assert!(r.sessions.session_of_char(char_key(NICKY)).is_none());
        assert_eq!(b.following(USER), None);
        assert_eq!(b.detach(USER).await, DetachOutcome::NotFollowing);
        assert_eq!(b.dm_line(USER, "look").await, LineOutcome::NotFollowing);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            dm_sends(&r.api)
                .iter()
                .any(|s| s.contains("Disconnected: Goodbye.")),
            "{:?}",
            dm_sends(&r.api)
        );
    }

    #[tokio::test]
    async fn follow_refusals_not_linked_banned_closed_dms_and_quit_in_the_dm() {
        let r = rig(|_| {}).await;
        let b = &r.bridge;
        assert_eq!(b.follow(OTHER, None).await, FollowOutcome::NotLinked);
        assert_eq!(b.dm_line(OTHER, "look").await, LineOutcome::NotFollowing);
        linked(&r.store, "Nick", USER, |_| {}).await;
        r.store
            .discord_ban_put(DiscordBanRec {
                user_id: USER,
                by_account: "wiz".into(),
                reason: "spam".into(),
                at: 1,
            })
            .await
            .unwrap();
        assert_eq!(b.follow(USER, None).await, FollowOutcome::Banned);
        r.store.discord_ban_del(USER).await.unwrap();
        // Closed DMs: nothing is attached, no ConnUp crosses.
        r.api.close_dms(USER);
        assert_eq!(b.follow(USER, None).await, FollowOutcome::DmsClosed);
        assert!(conn_ups(&r.link).is_empty());
        assert!(r.sessions.session_of_char(char_key(NICKY)).is_none());
        // (A fresh fake with open DMs.)
        let r = rig(|_| {}).await;
        let b = &r.bridge;
        linked(&r.store, "Nick", USER, |_| {}).await;
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        assert_eq!(b.dm_line(USER, "QUIT").await, LineOutcome::Closed);
        assert_eq!(b.following(USER), None);
        assert_eq!(quits(&r.link), vec![NICKY]);
        // Unlinking while following detaches.
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        assert!(matches!(
            b.unlink(USER).await,
            UnlinkOutcome::Unlinked { .. }
        ));
        assert_eq!(b.following(USER), None);
        assert_eq!(quits(&r.link), vec![NICKY, NICKY]);
    }

    #[tokio::test]
    async fn auto_follow_attaches_when_the_last_terminal_goes_linkdead() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |l| l.auto_follow = true).await;
        r.bridge.install_auto_follow();
        // A "terminal" connection of Nicky, through the same core.
        let (tx, _rx) = mpsc::channel::<Arc<Envelope>>(8);
        let terminal = ConnectionHandle::new_structured(
            ConnId(1),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            ClientCaps {
                kind: ClientKind::Web,
                ..discord_caps()
            },
            tx,
        );
        r.bridge
            .core
            .enter_game(terminal, NICKY, "Nick", "Nicky", true)
            .await
            .unwrap();
        assert_eq!(r.bridge.following(USER), None);
        // The terminal drops: linkdead → the hook → a follow, before the
        // grace expires, as a rejoin (one announce).
        r.bridge.core.disconnected(ConnId(1)).await;
        let mut waited = 0;
        while r.bridge.following(USER).is_none() && waited < 40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            waited += 1;
        }
        assert_eq!(r.bridge.following(USER), Some((NICKY, "Nicky".to_string())));
        let ups = conn_ups(&r.link);
        assert_eq!(ups.len(), 2, "{ups:?}");
        assert!(ups[1].3, "the follow rejoined the linkdead session");
        assert!(ups[1].0 >= DISCORD_CONN_ID_BASE);
        tokio::time::sleep(Duration::from_millis(150)).await;
        let sends = dm_sends(&r.api);
        assert!(
            sends
                .first()
                .is_some_and(|s| s.contains("Your terminal dropped")),
            "the intro comes first: {sends:?}"
        );
        // With auto-follow off nothing happens.
        r.bridge.detach(USER).await;
        linked(&r.store, "Nick", USER, |l| l.auto_follow = false).await;
        let (tx, _rx) = mpsc::channel::<Arc<Envelope>>(8);
        let terminal = ConnectionHandle::new_structured(
            ConnId(2),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            discord_caps(),
            tx,
        );
        r.bridge
            .core
            .enter_game(terminal, NICKY, "Nick", "Nicky", true)
            .await
            .unwrap();
        r.bridge.core.disconnected(ConnId(2)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(r.bridge.following(USER), None);
    }

    #[tokio::test]
    async fn the_idle_sweep_detaches_idle_and_deaf_follows_but_not_scene_members() {
        let r = rig(|c| c.follow_idle_detach_s = 3600).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        let b = &r.bridge;
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        let now = Instant::now();
        assert!(b.sweep_idle(now).await.is_empty(), "fresh input");
        let later = now + Duration::from_secs(3_601);
        // In a scene: held.
        {
            let follows = b.follows.lock().unwrap();
            follows[&USER].state.in_scene.store(true, Ordering::Relaxed);
        }
        assert!(
            b.sweep_idle(later).await.is_empty(),
            "a scene member is never idle-detached"
        );
        {
            let follows = b.follows.lock().unwrap();
            follows[&USER]
                .state
                .in_scene
                .store(false, Ordering::Relaxed);
        }
        assert_eq!(b.sweep_idle(later).await, vec!["Nicky".to_string()]);
        assert_eq!(b.following(USER), None);
        assert_eq!(quits(&r.link), vec![NICKY]);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            dm_sends(&r.api)
                .iter()
                .any(|s| s.contains("Detached: no Discord input")),
            "{:?}",
            dm_sends(&r.api)
        );
        // A deaf follow (closed DMs mid-follow) goes at the next sweep.
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        {
            let follows = b.follows.lock().unwrap();
            follows[&USER]
                .state
                .dm_closed
                .store(true, Ordering::Relaxed);
        }
        assert_eq!(
            b.sweep_idle(Instant::now()).await,
            vec!["Nicky".to_string()]
        );
        assert_eq!(b.following(USER), None);
    }

    #[tokio::test]
    async fn idle_detach_zero_means_never() {
        let r = rig(|c| c.follow_idle_detach_s = 0).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        assert!(matches!(
            r.bridge.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        let far = Instant::now() + Duration::from_secs(10 * 24 * 3600);
        assert!(r.bridge.sweep_idle(far).await.is_empty());
        assert_eq!(r.bridge.following(USER), Some((NICKY, "Nicky".to_string())));
    }
}
