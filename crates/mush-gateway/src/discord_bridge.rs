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

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_config::discord::link_code_hash;
use mush_config::ipguard::IpGuard;
use mush_config::unix_now_s;
use mush_core::output::STAFF_RELAY_SINK;
use mush_core::{AccountId, ObjId};
use mush_ipc::{
    DiscordControlOp, DiscordInteraction, DiscordInteractionError, GatewayLink, RelaySpeak,
    ServiceReq, ServiceResp,
};
use mush_net_discord::dm::escape_md;
use mush_net_discord::{
    ApiError, DetachOutcome, DiscordApi, DiscordConfig, DiscordHooks, DiscordRest, DmPolicy,
    FollowOutcome, FollowState, Intents, Interaction, InteractionReply, LineOutcome, LinkOutcome,
    ModalSpec, Outgoing, RelayLineOutcome, RelayMap, TextField, TwilightEvents, UnlinkOutcome,
    WhoAmI, run_dm_writer,
};
use mush_proto::{
    ClientCaps, ClientKind, ColorDepth, Envelope, NarrativeKind, OutputEvent, RespondChoice,
    RichText,
};
use mush_session::{CharKey, ConnId, ConnectionHandle, SessionManager};
use mush_store::{AccountRec, CharacterRef, DiscordLinkRec, ObjectStore};
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::accounts::AuthBackend;
use crate::bridge_common::{GameBridge, LineOutcome as CoreLine};
use crate::controls::{DiscordControl, GatewayControls, RelaySink};
use crate::delivery::{char_key, char_key_obj};
use crate::poison::locked;

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
    controls: &GatewayControls,
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
    // D4: an inbound relay mapping needs guild message events and their
    // content — the privileged Message Content intent (D-D-13, a portal
    // toggle). Without one, the shard stays DM-only.
    let relay = RelayMapper::new(&dc.relay);
    let mut intents = Intents::DIRECT_MESSAGES;
    if relay.accepts_inbound() {
        intents |= Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;
    }
    let events = Box::new(TwilightEvents::new(token, intents));
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
    // The server's `ToGateway::DiscordControl` (the in-game `@discord/follow`
    // family) reaches the bridge through the process's control registry.
    controls.set_discord(Arc::new(BridgeControl(Arc::downgrade(&bridge))));
    // D4: relay-flagged channel lines the world publishes reach the bridge
    // through the same registry.
    controls.set_relay(Arc::new(BridgeRelay(Arc::downgrade(&bridge))));
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
    core: Arc<GameBridge<A>>,
    // Conservative barrier: any revocation invalidates an in-flight attach.
    revocation: AtomicU64,
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
    /// The `[discord.relay]` map, both ways (D4).
    relay: RelayMapper,
    /// Per-author inbound relay limiter (D4): at most [`RELAY_PER_USER`]
    /// lines per [`RELAY_WINDOW`]; over the limit is silently dropped.
    relay_limiter: Mutex<HashMap<u64, VecDeque<Instant>>>,
    /// The Discord channel staff-facing lines go to (D5, `[discord]
    /// .staff_channel_id`); `0` drops them.
    staff_channel_id: u64,
}

/// The inbound relay limiter's window.
pub const RELAY_WINDOW: Duration = Duration::from_secs(10);
/// Lines one Discord author may relay per [`RELAY_WINDOW`].
pub const RELAY_PER_USER: usize = 5;
/// The longest line the bridge relays into the game, characters (the server
/// guards the same bound).
pub const MAX_RELAY_LINE_CHARS: usize = 1_000;

/// The `[discord.relay]` entries indexed both ways (D4).
#[derive(Debug, Default)]
pub struct RelayMapper {
    by_name: HashMap<String, RelayMap>,
    by_discord: HashMap<u64, RelayMap>,
}

impl RelayMapper {
    /// Index the configured mappings; entries with a blank channel name or a
    /// zero Discord channel id are skipped (logged).
    #[must_use]
    pub fn new(maps: &[RelayMap]) -> RelayMapper {
        let mut m = RelayMapper::default();
        for map in maps {
            if map.channel.trim().is_empty() || map.discord_channel_id == 0 {
                tracing::warn!("[discord.relay] entry with no channel name or id ignored");
                continue;
            }
            m.by_name
                .insert(map.channel.trim().to_ascii_lowercase(), map.clone());
            m.by_discord.insert(map.discord_channel_id, map.clone());
        }
        m
    }

    /// The mapping a game channel's line goes out through, if any.
    #[must_use]
    pub fn outbound(&self, channel: &str) -> Option<&RelayMap> {
        self.by_name
            .get(&channel.trim().to_ascii_lowercase())
            .filter(|m| matches!(m.direction.as_str(), "both" | "to_discord"))
    }

    /// The mapping a Discord channel's line comes in through, if any.
    #[must_use]
    pub fn inbound(&self, discord_channel_id: u64) -> Option<&RelayMap> {
        self.by_discord
            .get(&discord_channel_id)
            .filter(|m| matches!(m.direction.as_str(), "both" | "to_game"))
    }

    /// Whether any mapping accepts Discord → game lines (the intents the
    /// shard needs).
    #[must_use]
    pub fn accepts_inbound(&self) -> bool {
        self.by_discord
            .values()
            .any(|m| matches!(m.direction.as_str(), "both" | "to_game"))
    }
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
            core: Arc::new(GameBridge::with_link(accounts, sessions, link, guard)),
            revocation: AtomicU64::new(0),
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
            relay: RelayMapper::new(&config.relay),
            relay_limiter: Mutex::new(HashMap::new()),
            staff_channel_id: config.staff_channel_id,
        }
    }

    /// The character `user_id` is following, if any.
    #[must_use]
    pub fn following(&self, user_id: u64) -> Option<(ObjId, String)> {
        locked(&self.follows, "follows")
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
        let generation = self.revocation.load(Ordering::SeqCst);
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
            let mut follows = locked(&self.follows, "follows");
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
            prev.state.cancelled.cancel();
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
        // Never write an old link snapshot back after awaiting Discord.
        // The channel is idempotently resolved on each follow.
        if !self.authorized(user_id, chosen.id).await {
            return FollowOutcome::NotLinked;
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
            let mut follows = locked(&self.follows, "follows");
            // The same lock protects invalidation and publication.
            if generation == u64::MAX || generation != self.revocation.load(Ordering::SeqCst) {
                return FollowOutcome::Failed("the link changed; try following again".into());
            }
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
        let core = Arc::clone(&self.core);
        tokio::spawn({
            let api = Arc::clone(&self.api);
            let policy = self.policy;
            let state = Arc::clone(&state);
            async move {
                run_dm_writer(api, channel, rx, policy, state.clone()).await;
                state.cancelled.cancel();
                let removed = {
                    let mut follows = locked(&follows, "follows");
                    if follows.get(&user_id).is_some_and(|f| f.conn == conn) {
                        follows.remove(&user_id)
                    } else {
                        None
                    }
                };
                if removed.is_some() {
                    core.quit(conn).await;
                }
            }
        });
        if let Some(text) = intro {
            tokio::select! {
                () = state.cancelled.cancelled() => return FollowOutcome::NotLinked,
                result = self.api.send(channel, Outgoing::plain(text)) => {
                    if let Err(e) = result { tracing::debug!(error = %e, "discord follow: intro not delivered"); }
                }
            }
        }
        match self
            .core
            .enter_game(handle, chosen.id, &account.name, &chosen.name, true)
            .await
        {
            Ok(_) => {
                if state.cancelled.is_cancelled()
                    || generation != self.revocation.load(Ordering::SeqCst)
                    || !self.authorized(user_id, chosen.id).await
                {
                    state.cancelled.cancel();
                    self.core.quit(conn).await;
                    return FollowOutcome::NotLinked;
                }
                tracing::info!(conn = conn.0, actor = %chosen.id, "discord follow attached");
                FollowOutcome::Following {
                    character: chosen.name,
                    attached: true,
                }
            }
            Err(e) => {
                // `enter_game` rolled the attach back and dropped the handle;
                // the writer is ending on its own. Forget the follow now.
                let mut follows = locked(&self.follows, "follows");
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
        if locked(&self.follows, "follows").contains_key(&user_id) {
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
            let follows = locked(&self.follows, "follows");
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
                let mut follows = locked(&self.follows, "follows");
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

/// The longest free-text answer a request modal accepts, characters. The
/// engine's cap is `MAX_REQUEST_TEXT_BYTES` (2 048 bytes); 500 characters
/// cannot exceed it in any script, so a modal answer is never dropped for
/// length.
pub const MAX_MODAL_ANSWER_CHARS: u16 = 500;

/// The longest command line the notice reply modal accepts, characters.
pub const MAX_MODAL_LINE_CHARS: u16 = 1_000;

/// Bound a handler's reply for a Discord ephemeral message.
fn bounded_reply(text: String) -> String {
    const MAX: usize = 1_900;
    if text.chars().count() <= MAX {
        text
    } else {
        text.chars().take(MAX - 1).collect::<String>() + "…"
    }
}

impl<A: AuthBackend + 'static> DiscordBridge<A> {
    /// Store-backed permission check; an unavailable store fails closed.
    async fn authorized(&self, user: u64, character: ObjId) -> bool {
        if !matches!(self.store.discord_ban_get(user).await, Ok(None)) {
            return false;
        }
        matches!(self.store.account_by_discord_user(user).await, Ok(Some(a))
            if a.discord.as_ref().is_some_and(|l| l.user_id == user)
                && a.characters.iter().any(|c| c.id == character))
    }

    fn invalidate_character(&self, actor: ObjId) -> Option<ConnId> {
        let mut follows = locked(&self.follows, "follows");
        // MAX is unreachable in practice; saturating makes exhaustion fail
        // closed below instead of reviving an earlier attachment generation.
        let _ = self
            .revocation
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_add(1))
            });
        let user = follows
            .iter()
            .find(|(_, f)| f.character == actor)
            .map(|(u, _)| *u)?;
        let old = follows.remove(&user)?;
        old.state.cancelled.cancel();
        Some(old.conn)
    }

    async fn validated_follow(&self, user: u64) -> Option<(ConnId, ObjId, Arc<FollowState>)> {
        let (conn, actor, state) = self.follow_of(user)?;
        if state.cancelled.is_cancelled() || !self.authorized(user, actor).await {
            if let Some(conn) = self.invalidate_character(actor) {
                self.core.quit(conn).await;
            }
            return None;
        }
        if state.cancelled.is_cancelled() || !self.follow_of(user).is_some_and(|f| f.0 == conn) {
            return None;
        }
        Some((conn, actor, state))
    }

    /// The follow's connection and writer state, if `user_id` is following.
    fn follow_of(&self, user_id: u64) -> Option<(ConnId, ObjId, Arc<FollowState>)> {
        locked(&self.follows, "follows")
            .get(&user_id)
            .map(|f| (f.conn, f.character, Arc::clone(&f.state)))
    }

    /// The interaction router (D3, 03 §7a.4): by `custom_id` namespace.
    async fn route_interaction(&self, user_id: u64, ix: Interaction) -> InteractionReply {
        if (ix.custom_id.starts_with("d:") || self.follow_of(user_id).is_some())
            && self.validated_follow(user_id).await.is_none()
        {
            return InteractionReply::Ephemeral(
                "You are not following a character here; `/follow` first.".into(),
            );
        }
        if !matches!(self.store.discord_ban_get(user_id).await, Ok(None)) {
            return InteractionReply::Ephemeral("That control is unavailable.".into());
        }
        let id = ix.custom_id.clone();
        if let Some(rest) = id.strip_prefix("d:req:") {
            return self.answer_request(user_id, rest, &ix).await;
        }
        if let Some(rest) = id.strip_prefix("d:reqm:") {
            return self.answer_request_modal(user_id, rest, &ix).await;
        }
        if let Some(nonce) = id.strip_prefix("d:act:") {
            return self.open_action_modal(user_id, nonce);
        }
        if let Some(nonce) = id.strip_prefix("d:actm:") {
            return self.submit_action_modal(user_id, nonce, &ix).await;
        }
        if id.starts_with("d:call:") {
            return InteractionReply::Ephemeral("Calls are not available yet.".to_string());
        }
        self.forward_interaction(user_id, ix).await
    }

    /// Mark the request's message answered (or say so privately when the
    /// message is no longer known here).
    fn answered(
        state: &FollowState,
        request_id: u64,
        message_id: u64,
        custom_id: &str,
        value: Option<&str>,
        fallback_label: &str,
    ) -> InteractionReply {
        match state.take_request_message(request_id, message_id) {
            Some(shown) => {
                let label = shown
                    .label_for(custom_id, value)
                    .unwrap_or_else(|| fallback_label.to_string());
                InteractionReply::UpdateMessage(Outgoing::plain(format!(
                    "{}\n✅ {label}",
                    shown.text
                )))
            }
            None => InteractionReply::Ephemeral(
                "Sent — though that request may already be closed.".to_string(),
            ),
        }
    }

    /// `d:req:<id>:<opt>` — a button or select on a request.
    async fn answer_request(&self, user_id: u64, rest: &str, ix: &Interaction) -> InteractionReply {
        let Some((id, opt)) = rest.split_once(':') else {
            return InteractionReply::Ephemeral("That button is malformed.".to_string());
        };
        let Ok(request_id) = id.parse::<u64>() else {
            return InteractionReply::Ephemeral("That button is malformed.".to_string());
        };
        let Some((conn, _, state)) = self.follow_of(user_id) else {
            return InteractionReply::Ephemeral(
                "You are not following a character here; `/follow` first.".to_string(),
            );
        };
        let Some(shown) = state.request(request_id).filter(|shown| {
            ix.message_id == Some(shown.message_id)
                && ix.channel_id.is_none_or(|c| c == shown.channel_id)
        }) else {
            return InteractionReply::Ephemeral("That request has expired.".into());
        };
        let picked = ix.values.first().map(String::as_str);
        let (choice, fallback) = match opt {
            "yes" => (RespondChoice::Confirm(true), "Yes"),
            "no" => (RespondChoice::Confirm(false), "No"),
            "cancel" => (RespondChoice::Cancel, "Declined"),
            "text" => {
                return InteractionReply::Modal(ModalSpec {
                    custom_id: format!("d:reqm:{request_id}"),
                    title: "Your answer".to_string(),
                    field: TextField {
                        custom_id: "answer".to_string(),
                        label: "Answer".to_string(),
                        value: None,
                        max_len: MAX_MODAL_ANSWER_CHARS,
                        paragraph: true,
                    },
                });
            }
            o if o.starts_with("o:") => (RespondChoice::Choose(o[2..].into()), "Chosen"),
            s if s.starts_with('s') => match picked {
                Some(v) => (RespondChoice::Choose(v.into()), "Chosen"),
                None => return InteractionReply::Ephemeral("Pick an option.".to_string()),
            },
            _ => return InteractionReply::Ephemeral("That button is malformed.".to_string()),
        };
        // The engine applies its own rules: the subject must match, the
        // choice must be one it offered, and a second answer is ignored.
        self.core
            .respond(conn.0, request_id, shown.epoch, choice)
            .await;
        Self::answered(
            &state,
            request_id,
            shown.message_id,
            &ix.custom_id,
            picked,
            fallback,
        )
    }

    /// `d:reqm:<id>` — the free-text answer modal.
    async fn answer_request_modal(
        &self,
        user_id: u64,
        rest: &str,
        ix: &Interaction,
    ) -> InteractionReply {
        let Ok(request_id) = rest.parse::<u64>() else {
            return InteractionReply::Ephemeral("That form is malformed.".to_string());
        };
        let Some((conn, _, state)) = self.follow_of(user_id) else {
            return InteractionReply::Ephemeral(
                "You are not following a character here; `/follow` first.".to_string(),
            );
        };
        let Some(shown) = state.request(request_id).filter(|shown| {
            ix.message_id == Some(shown.message_id)
                && ix.channel_id.is_none_or(|c| c == shown.channel_id)
        }) else {
            return InteractionReply::Ephemeral("That request has expired.".into());
        };
        let answer = ix
            .fields
            .iter()
            .find(|(k, _)| k == "answer")
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        if answer.is_empty() {
            return InteractionReply::Ephemeral("An empty answer was not sent.".to_string());
        }
        self.core
            .respond(
                conn.0,
                request_id,
                shown.epoch,
                RespondChoice::Text(answer.clone()),
            )
            .await;
        match state.take_request_message(request_id, shown.message_id) {
            Some(shown) => InteractionReply::UpdateMessage(Outgoing::plain(format!(
                "{}\n✅ {}",
                shown.text,
                mush_net_discord::dm::escape_md(&answer)
            ))),
            None => InteractionReply::Ephemeral(
                "Sent — though that request may already be closed.".to_string(),
            ),
        }
    }

    /// `d:act:<nonce>` — a notice's reply button opens the prefilled modal.
    fn open_action_modal(&self, user_id: u64, nonce: &str) -> InteractionReply {
        let Ok(nonce) = nonce.parse::<u64>() else {
            return InteractionReply::Ephemeral("That button is malformed.".to_string());
        };
        let Some((_, _, state)) = self.follow_of(user_id) else {
            return InteractionReply::Ephemeral(
                "You are not following a character here; `/follow` first.".to_string(),
            );
        };
        match state.action(nonce) {
            Some((label, prefill)) => InteractionReply::Modal(ModalSpec {
                custom_id: format!("d:actm:{nonce}"),
                title: label,
                field: TextField {
                    custom_id: "line".to_string(),
                    label: "Command".to_string(),
                    value: Some(prefill),
                    max_len: MAX_MODAL_LINE_CHARS,
                    paragraph: false,
                },
            }),
            None => InteractionReply::Ephemeral(
                "That button has expired; type the command in the DM instead.".to_string(),
            ),
        }
    }

    /// `d:actm:<nonce>` — the reply modal's line runs as the character.
    async fn submit_action_modal(
        &self,
        user_id: u64,
        _nonce: &str,
        ix: &Interaction,
    ) -> InteractionReply {
        let line = ix
            .fields
            .iter()
            .find(|(k, _)| k == "line")
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        if line.is_empty() {
            return InteractionReply::Ephemeral("Nothing to send.".to_string());
        }
        match self.dm_line(user_id, &line).await {
            LineOutcome::Accepted => InteractionReply::Ephemeral("Sent.".to_string()),
            LineOutcome::Closed => InteractionReply::Ephemeral("Detached.".to_string()),
            LineOutcome::NotFollowing => InteractionReply::Ephemeral(
                "You are not following a character here; `/follow` first.".to_string(),
            ),
        }
    }

    /// Anything outside the bridge's namespaces crosses to the server, which
    /// dispatches by prefix to the handler a module registered (03 §7a.4).
    async fn forward_interaction(&self, user_id: u64, ix: Interaction) -> InteractionReply {
        let account = match self.store.account_by_discord_user(user_id).await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, "discord interaction: account lookup failed");
                return InteractionReply::Ephemeral(
                    "The account store is unavailable; try again shortly.".to_string(),
                );
            }
        };
        let actor = self
            .follow_of(user_id)
            .map(|(_, character, _)| character)
            .or_else(|| {
                let account = account.as_ref()?;
                account
                    .discord
                    .as_ref()
                    .and_then(|l| l.relay_character)
                    .or_else(|| account.characters.first().map(|c| c.id))
            });
        let values = if ix.values.is_empty() {
            ix.fields.into_iter().map(|(_, v)| v).collect()
        } else {
            ix.values
        };
        let req = DiscordInteraction {
            user_id,
            account: account.map(|a| a.name),
            actor,
            custom_id: ix.custom_id,
            values,
        };
        match self
            .core
            .link
            .request(ServiceReq::DiscordInteraction(req))
            .await
        {
            Some(ServiceResp::DiscordInteraction(Ok(text))) => {
                InteractionReply::Ephemeral(bounded_reply(text))
            }
            Some(ServiceResp::DiscordInteraction(Err(DiscordInteractionError::UnknownPrefix))) => {
                InteractionReply::Ephemeral("That control is not known to the game.".to_string())
            }
            Some(ServiceResp::DiscordInteraction(Err(DiscordInteractionError::Refused(why)))) => {
                InteractionReply::Ephemeral(bounded_reply(why))
            }
            Some(ServiceResp::Busy) => {
                InteractionReply::Ephemeral("The game is busy; try again in a moment.".to_string())
            }
            Some(ServiceResp::Unavailable) | None => InteractionReply::Ephemeral(
                "The game is not reachable right now; try again shortly.".to_string(),
            ),
            Some(other) => {
                tracing::error!(?other, "discord interaction: wrong response type");
                InteractionReply::Ephemeral("The game gave an unexpected answer.".to_string())
            }
        }
    }
}

/// The bridge as the boundary's [`DiscordControl`] (programme D3): held weakly
/// by the process's control registry, it runs each control on its own task.
struct BridgeControl<A: AuthBackend + 'static>(Weak<DiscordBridge<A>>);

impl<A: AuthBackend + 'static> DiscordControl for BridgeControl<A> {
    fn control(&self, char: u64, op: DiscordControlOp) {
        let Some(bridge) = self.0.upgrade() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let actor = char_key_obj(CharKey(char));
        if op == DiscordControlOp::Revoke {
            let conn = bridge.invalidate_character(actor);
            rt.spawn(async move {
                if let Some(conn) = conn {
                    bridge.core.quit(conn).await;
                }
            });
        } else {
            rt.spawn(async move {
                bridge.apply_control(actor, op).await;
            });
        }
    }
}

impl<A: AuthBackend + 'static> DiscordBridge<A> {
    /// Tell `actor` (every connection) something about their Discord follow.
    fn tell(&self, actor: ObjId, text: &str) {
        let _ = self.core.sessions.send_to_char(
            char_key(actor),
            Arc::new(OutputEvent::Sys(RichText::tag("discord", text))),
        );
    }

    /// The follow of `actor`'s character, if any: `(user, conn, state)`.
    fn follow_of_character(&self, actor: ObjId) -> Option<(u64, ConnId, Arc<FollowState>)> {
        locked(&self.follows, "follows")
            .iter()
            .find(|(_, f)| f.character == actor)
            .map(|(user, f)| (*user, f.conn, Arc::clone(&f.state)))
    }

    /// Apply an in-game control (`@discord/follow`, `/unfollow`, a live
    /// `quiet`) for `actor`, answering them with a `discord` system line.
    pub async fn apply_control(&self, actor: ObjId, op: DiscordControlOp) {
        match op {
            DiscordControlOp::Revoke => {
                if let Some(conn) = self.invalidate_character(actor) {
                    self.core.quit(conn).await;
                }
            }
            DiscordControlOp::Follow => {
                if let Some((_, _, _)) = self.follow_of_character(actor) {
                    self.tell(actor, "Discord: already following you.");
                    return;
                }
                let Ok(accounts) = self.store.list_accounts().await else {
                    self.tell(
                        actor,
                        "Discord: the account store is unavailable; try again.",
                    );
                    return;
                };
                let Some(account) = accounts
                    .into_iter()
                    .find(|a| a.characters.iter().any(|c| c.id == actor))
                else {
                    self.tell(
                        actor,
                        "Discord: your character is not linked to an account.",
                    );
                    return;
                };
                let Some(link) = account.discord.as_ref() else {
                    self.tell(
                        actor,
                        "Discord: your account has no Discord link — `@discord/link` mints a code.",
                    );
                    return;
                };
                let user_id = link.user_id;
                if locked(&self.follows, "follows").contains_key(&user_id) {
                    self.tell(
                        actor,
                        "Discord: your Discord user is already following another of your \
                         characters; `/follow <name>` in the DM switches.",
                    );
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
                let outcome = self
                    .follow_impl(
                        user_id,
                        Some(&name),
                        Some("You asked from the game to be followed here; `/detach` stops."),
                    )
                    .await;
                let text = match outcome {
                    FollowOutcome::Following { .. } => {
                        "Discord: following you now — the bot's DM shows what you see."
                    }
                    FollowOutcome::DmsClosed => {
                        "Discord: the bot cannot DM you. Allow DMs from server members (or \
                         message the bot first), then try again."
                    }
                    FollowOutcome::Banned => "Discord: your Discord user may not follow.",
                    FollowOutcome::NotLinked => {
                        "Discord: your account has no Discord link — `@discord/link` mints a code."
                    }
                    FollowOutcome::WhichCharacter { .. } => {
                        "Discord: could not pick the character."
                    }
                    FollowOutcome::Failed(_) => "Discord: the follow could not be attached.",
                };
                self.tell(actor, text);
            }
            DiscordControlOp::Unfollow => match self.follow_of_character(actor) {
                Some((user, conn, _)) => {
                    let removed = {
                        let mut follows = locked(&self.follows, "follows");
                        follows
                            .get(&user)
                            .is_some_and(|f| f.conn == conn)
                            .then(|| follows.remove(&user))
                            .flatten()
                    };
                    if removed.is_some() {
                        self.core.quit(conn).await;
                    }
                    self.tell(actor, "Discord: detached.");
                }
                None => self.tell(actor, "Discord: no follow to detach."),
            },
            DiscordControlOp::Quiet(on) => {
                // No follow: the preference is stored for the next one; the
                // server already said so.
                if let Some((_, _, state)) = self.follow_of_character(actor) {
                    state.quiet.store(on, Ordering::Relaxed);
                    self.tell(
                        actor,
                        if on {
                            "Discord: quiet on for your current follow."
                        } else {
                            "Discord: quiet off for your current follow."
                        },
                    );
                }
            }
        }
    }
}

/// The bridge as the boundary's [`RelaySink`] (programme D4): held weakly by
/// the process's control registry; each line is mirrored on its own task.
struct BridgeRelay<A: AuthBackend + 'static>(Weak<DiscordBridge<A>>);

impl<A: AuthBackend + 'static> RelaySink for BridgeRelay<A> {
    fn relay(&self, channel: &str, event: Arc<OutputEvent>) {
        let Some(bridge) = self.0.upgrade() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let channel = channel.to_string();
        rt.spawn(async move {
            bridge.relay_out(&channel, &event).await;
        });
    }
}

/// How a relayed game line reads on Discord: the speaker is the webhook's
/// name (or a bold prefix without one), so a say is just its text and a pose
/// is italic.
fn relay_content(kind: NarrativeKind, name: &str, text: &str, impersonated: bool) -> String {
    let text = escape_md(text);
    let name = escape_md(name);
    match (kind, impersonated) {
        (NarrativeKind::Pose, true) => format!("_{name} {text}_"),
        (NarrativeKind::SemiPose, true) => format!("_{name}{text}_"),
        (_, true) => text,
        (NarrativeKind::Pose, false) => format!("_{name} {text}_"),
        (NarrativeKind::SemiPose, false) => format!("_{name}{text}_"),
        (_, false) => format!("**{name}**: {text}"),
    }
}

/// How a staff-sink event reads on the staff channel (D5): a notice with its
/// severity marker and category (and the command its action offers, as text —
/// staff act in the game, not from Discord), a system line as text, a channel
/// line as a bold name and text. Anything else is not a staff line.
fn staff_line(event: &OutputEvent) -> Option<String> {
    match event {
        OutputEvent::Notice {
            severity,
            category,
            text,
            action,
            ..
        } => {
            let marker = match severity {
                mush_proto::NoticeSeverity::Info => "•",
                mush_proto::NoticeSeverity::Warn => "⚠",
                mush_proto::NoticeSeverity::Urgent => "‼",
            };
            let mut s = format!(
                "{marker} [{}] {}",
                escape_md(category),
                escape_md(&text.to_plain_text())
            );
            if let Some(a) = action {
                s.push_str(&format!("  (`{}`)", a.prefill));
            }
            Some(s)
        }
        OutputEvent::Sys(text) => Some(escape_md(&text.to_plain_text())),
        OutputEvent::Channel { actor, text, .. } => Some(format!(
            "**{}**: {}",
            escape_md(&actor.name),
            escape_md(&text.to_plain_text())
        )),
        _ => None,
    }
}

/// The TinyMUX pose forms a Discord line may use: `:waves` → pose,
/// `;grins` → semipose, anything else is said.
fn relay_kind(line: &str) -> (NarrativeKind, &str) {
    if let Some(t) = line.strip_prefix(':') {
        (NarrativeKind::Pose, t)
    } else if let Some(t) = line.strip_prefix(';') {
        (NarrativeKind::SemiPose, t)
    } else {
        (NarrativeKind::Say, line)
    }
}

impl<A: AuthBackend + 'static> DiscordBridge<A> {
    /// The relay map (for the wiring's intents and tests).
    #[must_use]
    pub fn relay_map(&self) -> &RelayMapper {
        &self.relay
    }

    /// Whether `user` may relay another line now; records the line if so.
    fn relay_admit(&self, user: u64) -> bool {
        let now = Instant::now();
        let mut limiter = locked(&self.relay_limiter, "relay limiter");
        let window = limiter.entry(user).or_default();
        while window
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RELAY_WINDOW)
        {
            window.pop_front();
        }
        if window.len() >= RELAY_PER_USER {
            return false;
        }
        window.push_back(now);
        true
    }

    /// Game → Discord (D4): a relay-flagged channel's line, mirrored to its
    /// mapped Discord channel under the character's name.
    pub async fn relay_out(&self, channel: &str, event: &OutputEvent) {
        // D5: the reserved staff sink — `+request` filings and staff alerts —
        // posts to the staff channel as the bot (system lines are never
        // impersonated). No channel configured: dropped.
        if channel == STAFF_RELAY_SINK {
            if self.staff_channel_id == 0 {
                tracing::debug!("staff relay line with no staff channel; dropped");
                return;
            }
            let Some(text) = staff_line(event) else {
                return;
            };
            match self
                .api
                .send(self.staff_channel_id, Outgoing::plain(text))
                .await
            {
                Ok(_) => tracing::debug!("staff relay line posted"),
                Err(e) => tracing::warn!(error = %e, "staff relay line not posted"),
            }
            return;
        }
        let Some(map) = self.relay.outbound(channel) else {
            tracing::debug!(channel, "relay line for an unmapped channel dropped");
            return;
        };
        let OutputEvent::Channel {
            actor, kind, text, ..
        } = event
        else {
            return;
        };
        let content = relay_content(*kind, &actor.name, &text.to_plain_text(), map.webhook);
        let result = if map.webhook {
            self.api
                .webhook_send(map.discord_channel_id, &actor.name, &content)
                .await
        } else {
            self.api
                .send(map.discord_channel_id, Outgoing::plain(content))
                .await
        };
        match result {
            Ok(_) => tracing::debug!(channel, "relay line mirrored"),
            Err(e) => tracing::warn!(channel, error = %e, "relay line not mirrored"),
        }
    }

    /// Discord → game (D4): a line in a mapped Discord channel, spoken on the
    /// game channel as the author's character after the server re-checks the
    /// pairing and comsys its gates.
    pub async fn relay_in(&self, user_id: u64, channel_id: u64, content: &str) -> RelayLineOutcome {
        let Some(map) = self.relay.inbound(channel_id) else {
            return RelayLineOutcome::Ignored;
        };
        let (kind, text) = relay_kind(content.trim());
        let text = text.trim();
        if text.is_empty() {
            return RelayLineOutcome::Ignored;
        }
        if !self.relay_admit(user_id) {
            tracing::debug!("relay line over the per-author limit dropped");
            return RelayLineOutcome::Ignored;
        }
        let account = match self.store.account_by_discord_user(user_id).await {
            Ok(Some(a)) => a,
            Ok(None) => return RelayLineOutcome::NotLinked,
            Err(e) => {
                tracing::warn!(error = %e, "relay: account lookup failed");
                return RelayLineOutcome::Refused("the account store is unavailable".to_string());
            }
        };
        match self.store.discord_ban_get(user_id).await {
            Ok(None) => {}
            // A banned author is ignored silently — no oracle for the ban.
            Ok(Some(_)) => return RelayLineOutcome::Ignored,
            Err(e) => {
                tracing::warn!(error = %e, "relay: ban lookup failed");
                return RelayLineOutcome::Refused("the account store is unavailable".to_string());
            }
        }
        if text.chars().count() > MAX_RELAY_LINE_CHARS {
            return RelayLineOutcome::Refused(format!(
                "that line is over the relay's {MAX_RELAY_LINE_CHARS}-character limit"
            ));
        }
        let Some(account_id) = account
            .id
            .or_else(|| AccountId::mint(&account.name, account.created).ok())
        else {
            return RelayLineOutcome::Refused("your account has no id yet".to_string());
        };
        let actor = self
            .follow_of(user_id)
            .map(|(_, character, _)| character)
            .or_else(|| {
                account
                    .discord
                    .as_ref()
                    .and_then(|l| l.relay_character)
                    .filter(|c| account.characters.iter().any(|ch| ch.id == *c))
            })
            .or_else(|| account.characters.first().map(|c| c.id));
        let Some(actor) = actor else {
            return RelayLineOutcome::Refused("your account has no character yet".to_string());
        };
        let req = RelaySpeak {
            user_id,
            account: account_id,
            actor,
            channel: map.channel.clone(),
            kind,
            text: text.to_string(),
        };
        match self.core.link.request(ServiceReq::RelaySpeak(req)).await {
            Some(ServiceResp::RelaySpeak(Ok(()))) => RelayLineOutcome::Accepted,
            Some(ServiceResp::RelaySpeak(Err(e))) => RelayLineOutcome::Refused(e.to_string()),
            Some(ServiceResp::Busy) => {
                RelayLineOutcome::Refused("the game is busy; try again in a moment".to_string())
            }
            Some(ServiceResp::Unavailable) | None => {
                RelayLineOutcome::Refused("the game is not reachable right now".to_string())
            }
            Some(other) => {
                tracing::error!(?other, "relay speak: wrong response type");
                RelayLineOutcome::Refused("the game gave an unexpected answer".to_string())
            }
        }
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
            for character in &account.characters {
                if let Some(conn) = self.invalidate_character(character.id) {
                    self.core.quit(conn).await;
                }
            }
            let _ = self.store.discord_user_release(user_id).await;
            // An unlinked user has no business following.
            let follow = locked(&self.follows, "follows").remove(&user_id);
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
            let follow = locked(&self.follows, "follows").remove(&user_id);
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
            let Some((validated, _, _)) = self.validated_follow(user_id).await else {
                return LineOutcome::NotFollowing;
            };
            let conn = {
                let mut follows = locked(&self.follows, "follows");
                match follows.get_mut(&user_id) {
                    Some(f) if f.conn == validated && !f.state.cancelled.is_cancelled() => {
                        f.last_input = Instant::now();
                        f.conn
                    }
                    _ => return LineOutcome::NotFollowing,
                }
            };
            tracing::debug!(conn = conn.0, "discord dm line accepted for the follow");
            match self.core.line(conn, line).await {
                CoreLine::Continue => LineOutcome::Accepted,
                CoreLine::Close => {
                    let mut follows = locked(&self.follows, "follows");
                    if follows.get(&user_id).is_some_and(|f| f.conn == conn) {
                        follows.remove(&user_id);
                    }
                    LineOutcome::Closed
                }
            }
        })
    }

    fn interaction(
        &self,
        user_id: u64,
        interaction: Interaction,
    ) -> BoxFuture<'_, InteractionReply> {
        Box::pin(async move { self.route_interaction(user_id, interaction).await })
    }

    fn relay_line<'a>(
        &'a self,
        user_id: u64,
        channel_id: u64,
        content: &'a str,
    ) -> BoxFuture<'a, RelayLineOutcome> {
        Box::pin(async move { self.relay_in(user_id, channel_id, content).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_config::discord::normalize_link_code;
    use mush_ipc::{ServiceReq, ServiceResp, ToServer, WireInbound};
    use mush_net_discord::RequestMessage;
    use mush_net_discord::dm::request_rows;
    use mush_net_discord::testing::{Call, FakeApi};
    use mush_proto::{
        NarrativeKind, NoticeAction, NoticeSeverity, ObjRef, RequestKind, RequestOption, SmolStr,
    };
    use mush_session::SessionConfig;
    use mush_store::{DiscordBanRec, DiscordCodeRecord, InMemoryStore};

    const USER: u64 = 123_456_789_012_345_678;
    const OTHER: u64 = 987_654_321_098_765_432;
    const NICKY: ObjId = ObjId(7);
    const ALT: ObjId = ObjId(8);

    #[tokio::test]
    async fn unlinked_follow_must_stop_accepting_commands() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        assert!(matches!(
            r.bridge.follow(USER, None).await,
            FollowOutcome::Following { .. }
        ));
        // The same two store operations run_discord_unlink performs.
        r.store.set_account_discord("Nick", None).await.unwrap();
        r.store.discord_user_release(USER).await.unwrap();
        assert_eq!(
            r.bridge.dm_line(USER, "say still connected").await,
            LineOutcome::NotFollowing
        );
    }

    #[tokio::test]
    async fn banned_follow_must_stop_accepting_commands() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        assert!(matches!(
            r.bridge.follow(USER, None).await,
            FollowOutcome::Following { .. }
        ));
        r.store
            .discord_ban_put(DiscordBanRec {
                user_id: USER,
                by_account: "Staff".into(),
                reason: "test".into(),
                at: 1,
            })
            .await
            .unwrap();
        assert_eq!(
            r.bridge.dm_line(USER, "say still connected").await,
            LineOutcome::NotFollowing
        );
    }

    /// A world link that admits everything and records what it was sent and
    /// asked; a forwarded Discord interaction is acknowledged by name.
    #[derive(Default)]
    struct AutoLink {
        stall_ready: std::sync::atomic::AtomicBool,
        ready_release: tokio::sync::Notify,
        sent: Mutex<Vec<ToServer>>,
        requests: Mutex<Vec<ServiceReq>>,
    }

    impl AutoLink {
        fn sent(&self) -> Vec<ToServer> {
            self.sent.lock().unwrap().clone()
        }

        fn requests(&self) -> Vec<ServiceReq> {
            self.requests.lock().unwrap().clone()
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
                    ServiceReq::ConnectionReady { .. } => {
                        if self.stall_ready.load(Ordering::Relaxed) {
                            self.ready_release.notified().await;
                        }
                        ServiceResp::ConnectionReady(true)
                    }
                    ServiceReq::InInputContext { .. } => ServiceResp::InInputContext(false),
                    ServiceReq::DiscordInteraction(ix) => {
                        let reply = format!("acked {}", ix.custom_id);
                        self.requests
                            .lock()
                            .unwrap()
                            .push(ServiceReq::DiscordInteraction(ix));
                        ServiceResp::DiscordInteraction(Ok(reply))
                    }
                    ServiceReq::RelaySpeak(req) => {
                        let result = if req.text == "gag me" {
                            Err(mush_ipc::RelayError::Refused("you are gagged".to_string()))
                        } else {
                            Ok(())
                        };
                        self.requests
                            .lock()
                            .unwrap()
                            .push(ServiceReq::RelaySpeak(req));
                        ServiceResp::RelaySpeak(result)
                    }
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
        let mut config = DiscordConfig {
            dm_coalesce_ms: 50,
            ..DiscordConfig::default()
        };
        edit(&mut config);
        rig_with(config).await
    }

    async fn rig_with(config: DiscordConfig) -> Rig {
        let store = store_with("Nick").await;
        let api = FakeApi::new();
        let link = Arc::new(AutoLink::default());
        let sessions = Arc::new(SessionManager::with_config(SessionConfig::default()));
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

    /// The `(conn, actor)` of every attach look `enter_game` asked for: the
    /// core `room` query at `id` 0 (16b cut B.4, `B-14`).
    fn room_queries(link: &AutoLink) -> Vec<(u64, ObjId)> {
        link.sent()
            .into_iter()
            .filter_map(|m| match m {
                ToServer::Inbound(WireInbound::Query {
                    conn,
                    actor,
                    id: 0,
                    name,
                    ..
                }) if name == "room" => Some((conn, actor)),
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
                .is_none(),
            "following must not rewrite the persisted link snapshot"
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
        // Each attach asked the core `room` query for its own connection (the
        // attach look, 16b cut B.4) rather than typing a `look` as the actor.
        let ups = conn_ups(&r.link);
        let looks = room_queries(&r.link);
        assert_eq!(
            looks,
            ups.iter()
                .map(|(conn, actor, ..)| (*conn, *actor))
                .collect::<Vec<_>>(),
            "one room query per attach, to that connection"
        );
        assert!(
            !cmds(&r.link).iter().any(|(_, line)| line == "look"),
            "the attach typed no `look`"
        );
        // A DM line runs as the character.
        assert_eq!(b.dm_line(USER, "say hi").await, LineOutcome::Accepted);
        let lines = cmds(&r.link);
        assert_eq!(lines.last(), Some(&(NICKY, "say hi".to_string())));
        // The query's answer, a `Reply { id: 0, Room }`, reaches the DM as the
        // room card a bare room makes.
        r.sessions.send_to_char(
            char_key(NICKY),
            Arc::new(OutputEvent::Reply {
                id: 0,
                event: Box::new(OutputEvent::Room(mush_proto::RoomView {
                    name: "Limbo".into(),
                    id: "#0".into(),
                    desc: RichText::plain("A grey nowhere."),
                    contents: vec![],
                    exits: vec![mush_proto::ExitRef {
                        name: "east".into(),
                        aliases: vec![],
                    }],
                    zone: None,
                    ambient: Default::default(),
                })),
            }),
        );
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
                .any(|s| s.contains("**Limbo**") && s.contains("Exits: east")),
            "the attach look reaches the DM: {sends:?}"
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

    fn responds(link: &AutoLink) -> Vec<(u64, RespondChoice)> {
        link.sent()
            .into_iter()
            .filter_map(|m| match m {
                ToServer::Inbound(WireInbound::Respond { id, choice, .. }) => Some((id, choice)),
                _ => None,
            })
            .collect()
    }

    fn click(custom_id: &str, values: &[&str]) -> Interaction {
        Interaction {
            custom_id: custom_id.to_string(),
            values: values.iter().map(|v| (*v).to_string()).collect(),
            fields: Vec::new(),
            channel_id: Some(1001),
            message_id: Some(
                500 + custom_id
                    .split(':')
                    .nth(2)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0),
            ),
        }
    }

    fn shown(id: u64, text: &str, kind: RequestKind, options: &[RequestOption]) -> RequestMessage {
        RequestMessage {
            epoch: "test".into(),
            channel_id: 1001,
            message_id: 500 + id,
            text: text.to_string(),
            rows: request_rows(id, kind, options),
        }
    }

    /// The writer's state for `user`'s follow.
    fn state_of<A: AuthBackend + 'static>(b: &DiscordBridge<A>, user: u64) -> Arc<FollowState> {
        Arc::clone(&b.follows.lock().unwrap()[&user].state)
    }

    #[tokio::test]
    async fn request_buttons_answer_through_respond_and_mark_the_message() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        let b = &r.bridge;
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        let state = state_of(b, USER);
        // The writer recorded a confirm request; Yes answers it and the
        // message is rewritten with the answer, buttons gone.
        state.request_shown(
            9,
            shown(9, "❓ Spend a willpower?", RequestKind::Confirm, &[]),
        );
        assert_eq!(
            b.interaction(USER, click("d:req:9:yes", &[])).await,
            InteractionReply::UpdateMessage(Outgoing::plain("❓ Spend a willpower?\n✅ Yes"))
        );
        assert_eq!(responds(&r.link), vec![(9, RespondChoice::Confirm(true))]);
        // A second click: the engine's duplicate rule decides; here the
        // message is already marked, so the clicker is told privately.
        assert!(matches!(
            b.interaction(USER, click("d:req:9:no", &[])).await,
            InteractionReply::Ephemeral(t) if t.contains("expired")
        ));
        // A select pick names the option by its label.
        let many: Vec<RequestOption> = (0..30)
            .map(|i| RequestOption {
                id: mush_proto::SmolStr::new(format!("o{i}")),
                label: format!("Option {i}"),
            })
            .collect();
        state.request_shown(11, shown(11, "❓ Who?", RequestKind::Choice, &many));
        assert_eq!(
            b.interaction(USER, click("d:req:11:s1", &["o27"])).await,
            InteractionReply::UpdateMessage(Outgoing::plain("❓ Who?\n✅ Option 27"))
        );
        assert_eq!(
            responds(&r.link).last(),
            Some(&(11, RespondChoice::Choose("o27".into())))
        );
        // Decline is `respond cancel`.
        state.request_shown(13, shown(13, "❓ Seal?", RequestKind::Choice, &many[..2]));
        assert_eq!(
            b.interaction(USER, click("d:req:13:cancel", &[])).await,
            InteractionReply::UpdateMessage(Outgoing::plain("❓ Seal?\n✅ Decline"))
        );
        assert_eq!(responds(&r.link).last(), Some(&(13, RespondChoice::Cancel)));
        // Text: the button opens a modal; the submission answers.
        state.request_shown(12, shown(12, "❓ Name it.", RequestKind::Text, &[]));
        let InteractionReply::Modal(spec) = b.interaction(USER, click("d:req:12:text", &[])).await
        else {
            panic!("a text request opens a modal");
        };
        assert_eq!(spec.custom_id, "d:reqm:12");
        assert_eq!(
            b.interaction(
                USER,
                Interaction {
                    custom_id: "d:reqm:12".into(),
                    values: vec![],
                    fields: vec![("answer".into(), "the north door".into())],
                    channel_id: None,
                    message_id: Some(512),
                },
            )
            .await,
            InteractionReply::UpdateMessage(Outgoing::plain("❓ Name it.\n✅ the north door"))
        );
        assert_eq!(
            responds(&r.link).last(),
            Some(&(12, RespondChoice::Text("the north door".into())))
        );
        // Not following: refused without touching the game.
        let before = responds(&r.link).len();
        assert!(matches!(
            b.interaction(OTHER, click("d:req:9:yes", &[])).await,
            InteractionReply::Ephemeral(t) if t.contains("not following")
        ));
        assert_eq!(responds(&r.link).len(), before);
    }

    #[tokio::test]
    async fn notice_reply_buttons_open_a_prefilled_modal_whose_line_runs_as_the_character() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        let b = &r.bridge;
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        let state = state_of(b, USER);
        let nonce = state.offer_action("Reply", "page Wiz=");
        let InteractionReply::Modal(spec) = b
            .interaction(USER, click(&format!("d:act:{nonce}"), &[]))
            .await
        else {
            panic!("the reply button opens a modal");
        };
        assert_eq!(spec.title, "Reply");
        assert_eq!(spec.custom_id, format!("d:actm:{nonce}"));
        assert_eq!(spec.field.value.as_deref(), Some("page Wiz="));
        assert_eq!(
            b.interaction(
                USER,
                Interaction {
                    custom_id: format!("d:actm:{nonce}"),
                    values: vec![],
                    fields: vec![("line".into(), "page Wiz=on my way".into())],
                    channel_id: None,
                    message_id: Some(600),
                },
            )
            .await,
            InteractionReply::Ephemeral("Sent.".into())
        );
        assert_eq!(
            cmds(&r.link).last(),
            Some(&(NICKY, "page Wiz=on my way".to_string()))
        );
        // A forgotten (or forged) button.
        assert!(matches!(
            b.interaction(USER, click("d:act:999999", &[])).await,
            InteractionReply::Ephemeral(t) if t.contains("expired")
        ));
    }

    #[tokio::test]
    async fn unknown_prefixes_cross_to_the_server_with_the_account_and_actor() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        let b = &r.bridge;
        assert!(matches!(
            b.follow(USER, None).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        assert_eq!(
            b.interaction(USER, click("oncall:ack:17", &[])).await,
            InteractionReply::Ephemeral("acked oncall:ack:17".into())
        );
        let forwarded = r.link.requests();
        assert_eq!(forwarded.len(), 1);
        let ServiceReq::DiscordInteraction(ix) = &forwarded[0] else {
            panic!("forwarded as a DiscordInteraction");
        };
        assert_eq!(ix.user_id, USER);
        assert_eq!(ix.account.as_deref(), Some("Nick"));
        assert_eq!(ix.actor, Some(NICKY), "the followed character");
        assert!(
            !format!("{ix:?}").contains(&USER.to_string()),
            "Debug redacts the snowflake"
        );
        // An unlinked user is forwarded too, with no account: the handler
        // decides what an anonymous click may do.
        assert!(matches!(
            b.interaction(OTHER, click("oncall:ack:18", &[])).await,
            InteractionReply::Ephemeral(_)
        ));
        let ServiceReq::DiscordInteraction(ix) = &r.link.requests()[1] else {
            panic!();
        };
        assert_eq!((ix.account.as_deref(), ix.actor), (None, None));
        // The bridge keeps `d:call:` for itself.
        assert!(matches!(
            b.interaction(USER, click("d:call:join:1", &[])).await,
            InteractionReply::Ephemeral(t) if t.contains("not available")
        ));
        assert_eq!(r.link.requests().len(), 2, "never forwarded");
    }

    #[tokio::test]
    async fn in_game_controls_follow_unfollow_and_retune_quiet() {
        let r = rig(|_| {}).await;
        let b = &r.bridge;
        // Unlinked: told so, nothing attached.
        b.apply_control(NICKY, DiscordControlOp::Follow).await;
        assert!(conn_ups(&r.link).is_empty());
        linked(&r.store, "Nick", USER, |_| {}).await;
        b.apply_control(NICKY, DiscordControlOp::Follow).await;
        assert_eq!(b.following(USER), Some((NICKY, "Nicky".to_string())));
        assert_eq!(conn_ups(&r.link).len(), 1);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            dm_sends(&r.api)
                .first()
                .is_some_and(|s| s.contains("asked from the game")),
            "{:?}",
            dm_sends(&r.api)
        );
        // Again: no second attach.
        b.apply_control(NICKY, DiscordControlOp::Follow).await;
        assert_eq!(conn_ups(&r.link).len(), 1);
        // Quiet flips the live writer state.
        let state = state_of(b, USER);
        assert!(!state.quiet.load(Ordering::Relaxed));
        b.apply_control(NICKY, DiscordControlOp::Quiet(true)).await;
        assert!(state.quiet.load(Ordering::Relaxed));
        b.apply_control(NICKY, DiscordControlOp::Quiet(false)).await;
        assert!(!state.quiet.load(Ordering::Relaxed));
        // Unfollow quits the follow; a second one is a no-op.
        b.apply_control(NICKY, DiscordControlOp::Unfollow).await;
        assert_eq!(b.following(USER), None);
        assert_eq!(quits(&r.link), vec![NICKY]);
        b.apply_control(NICKY, DiscordControlOp::Unfollow).await;
        assert_eq!(quits(&r.link), vec![NICKY]);
        // The registry hands controls to the bridge (and drops them without one).
        let controls = GatewayControls::default();
        controls.discord_control(u64::from(NICKY.0), DiscordControlOp::Follow);
        assert!(!controls.has_discord());
        controls.set_discord(Arc::new(BridgeControl(Arc::downgrade(&r.bridge))));
        controls.discord_control(u64::from(NICKY.0), DiscordControlOp::Follow);
        let mut waited = 0;
        while r.bridge.following(USER).is_none() && waited < 40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            waited += 1;
        }
        assert_eq!(r.bridge.following(USER), Some((NICKY, "Nicky".to_string())));
    }

    fn relay_config() -> DiscordConfig {
        DiscordConfig {
            dm_coalesce_ms: 50,
            relay: vec![
                RelayMap {
                    channel: "Public".into(),
                    discord_channel_id: 4242,
                    direction: "both".into(),
                    webhook: true,
                },
                RelayMap {
                    channel: "Staff".into(),
                    discord_channel_id: 5151,
                    direction: "to_discord".into(),
                    webhook: false,
                },
                RelayMap {
                    channel: "Intake".into(),
                    discord_channel_id: 6161,
                    direction: "to_game".into(),
                    webhook: true,
                },
            ],
            ..DiscordConfig::default()
        }
    }

    fn channel_line(name: &str, kind: NarrativeKind, text: &str) -> OutputEvent {
        OutputEvent::Channel {
            channel: mush_proto::SmolStr::new(name),
            seq: 7,
            actor: ObjRef::new("#9", "Zoe"),
            title: None,
            kind,
            text: RichText::plain(text),
            ts: 0,
        }
    }

    #[test]
    fn the_relay_map_honours_directions() {
        let m = RelayMapper::new(&relay_config().relay);
        assert_eq!(
            m.outbound("public").map(|r| r.discord_channel_id),
            Some(4242)
        );
        assert_eq!(
            m.outbound("Staff").map(|r| r.discord_channel_id),
            Some(5151)
        );
        assert!(m.outbound("Intake").is_none(), "to_game never goes out");
        assert!(m.inbound(4242).is_some());
        assert!(m.inbound(5151).is_none(), "to_discord never comes in");
        assert!(m.inbound(6161).is_some());
        assert!(m.inbound(9).is_none());
        assert!(m.accepts_inbound());
        assert!(
            !RelayMapper::new(&relay_config().relay[1..2]).accepts_inbound(),
            "an outbound-only map needs no message intents"
        );
        // Blank or zero entries are skipped.
        let none = RelayMapper::new(&[RelayMap {
            channel: String::new(),
            discord_channel_id: 1,
            ..RelayMap::default()
        }]);
        assert!(none.outbound("").is_none());
    }

    #[tokio::test]
    async fn game_lines_go_out_through_the_webhook_or_as_the_bot() {
        let r = rig_with(relay_config()).await;
        let b = &r.bridge;
        b.relay_out(
            "Public",
            &channel_line("Public", NarrativeKind::Say, "hi *all*"),
        )
        .await;
        b.relay_out(
            "Public",
            &channel_line("Public", NarrativeKind::Pose, "waves."),
        )
        .await;
        b.relay_out("Staff", &channel_line("Staff", NarrativeKind::Say, "ping"))
            .await;
        b.relay_out(
            "Intake",
            &channel_line("Intake", NarrativeKind::Say, "nope"),
        )
        .await;
        b.relay_out(
            "Public",
            &OutputEvent::Sys(RichText::tag("channel", "not a channel line")),
        )
        .await;
        let calls = r.api.calls();
        assert_eq!(
            calls,
            vec![
                Call::WebhookSend {
                    channel_id: 4242,
                    username: "Zoe".into(),
                    content: "hi \\*all\\*".into(),
                },
                Call::WebhookSend {
                    channel_id: 4242,
                    username: "Zoe".into(),
                    content: "_Zoe waves._".into(),
                },
                Call::Send {
                    channel_id: 5151,
                    msg: Outgoing::plain("**Zoe**: ping"),
                },
            ],
            "the to_game map and a non-channel event go nowhere"
        );
    }

    #[tokio::test]
    async fn discord_lines_come_in_as_the_linked_character_under_the_gates() {
        let r = rig_with(relay_config()).await;
        r.store.link_character("Nick", ALT, "Alt").await.unwrap();
        let b = &r.bridge;
        // Unmapped channel and outbound-only channel: ignored, no lookup.
        assert_eq!(b.relay_in(USER, 9, "hi").await, RelayLineOutcome::Ignored);
        assert_eq!(
            b.relay_in(USER, 5151, "hi").await,
            RelayLineOutcome::Ignored
        );
        // Unlinked: told (once, by the bot).
        assert_eq!(
            b.relay_in(OTHER, 4242, "hi").await,
            RelayLineOutcome::NotLinked
        );
        assert!(r.link.requests().is_empty());
        // Linked, relay character set: spoken as it, kind from the prefix.
        linked(&r.store, "Nick", USER, |l| l.relay_character = Some(ALT)).await;
        assert_eq!(
            b.relay_in(USER, 4242, "  hello ").await,
            RelayLineOutcome::Accepted
        );
        assert_eq!(
            b.relay_in(USER, 4242, ":waves").await,
            RelayLineOutcome::Accepted
        );
        let reqs = r.link.requests();
        assert_eq!(reqs.len(), 2);
        let ServiceReq::RelaySpeak(first) = &reqs[0] else {
            panic!()
        };
        assert_eq!(first.actor, ALT);
        assert_eq!(first.channel, "Public");
        assert_eq!(first.kind, NarrativeKind::Say);
        assert_eq!(first.text, "hello");
        assert_eq!(first.user_id, USER);
        assert_eq!(
            first.account,
            AccountId::mint("Nick", 1).unwrap(),
            "the account id the server's oracle minted"
        );
        assert!(!format!("{first:?}").contains(&USER.to_string()));
        let ServiceReq::RelaySpeak(second) = &reqs[1] else {
            panic!()
        };
        assert_eq!(
            (second.kind, second.text.as_str()),
            (NarrativeKind::Pose, "waves")
        );
        // A follow wins over the relay character.
        assert!(matches!(
            b.follow(USER, Some("Nicky")).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        assert_eq!(
            b.relay_in(USER, 4242, "as nicky").await,
            RelayLineOutcome::Accepted
        );
        let ServiceReq::RelaySpeak(third) = &r.link.requests()[2] else {
            panic!()
        };
        assert_eq!(third.actor, NICKY);
        // The server's refusal reaches the author.
        assert_eq!(
            b.relay_in(USER, 4242, "gag me").await,
            RelayLineOutcome::Refused("you are gagged".into())
        );
        // Empty and over-long lines never cross; a banned author is silent.
        assert_eq!(
            b.relay_in(USER, 4242, "   ").await,
            RelayLineOutcome::Ignored
        );
        assert!(matches!(
            b.relay_in(USER, 4242, &"x".repeat(MAX_RELAY_LINE_CHARS + 1)).await,
            RelayLineOutcome::Refused(t) if t.contains("limit")
        ));
        let before = r.link.requests().len();
        // The limiter: five in ten seconds, then silence.
        for _ in 0..RELAY_PER_USER {
            let _ = b.relay_in(USER, 4242, "burst").await;
        }
        assert_eq!(
            b.relay_in(USER, 4242, "burst").await,
            RelayLineOutcome::Ignored
        );
        assert!(r.link.requests().len() <= before + RELAY_PER_USER);
        r.store
            .discord_ban_put(DiscordBanRec {
                user_id: OTHER,
                by_account: "wiz".into(),
                reason: "spam".into(),
                at: 1,
            })
            .await
            .unwrap();
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
        linked(&r.store, "Zoe", OTHER, |_| {}).await;
        assert_eq!(
            b.relay_in(OTHER, 4242, "hi").await,
            RelayLineOutcome::Ignored
        );
    }

    #[tokio::test]
    async fn staff_sink_lines_post_to_the_staff_channel_as_the_bot() {
        let notice = OutputEvent::Notice {
            id: 0,
            severity: NoticeSeverity::Urgent,
            category: SmolStr::new("alert"),
            text: RichText::plain("Quarantine *full* on dev1"),
            action: Some(NoticeAction {
                label: "Open".into(),
                prefill: "@econ/alerts #3".into(),
            }),
            ts: 0,
        };
        let r = rig(|c| c.staff_channel_id = 9090).await;
        r.bridge.relay_out(STAFF_RELAY_SINK, &notice).await;
        assert_eq!(
            r.api.calls(),
            vec![Call::Send {
                channel_id: 9090,
                msg: Outgoing::plain(
                    "‼ [alert] Quarantine \\*full\\* on dev1  (`@econ/alerts #3`)"
                ),
            }],
            "posted as the bot, never impersonated"
        );
        // A system line and a channel line render; other events do not.
        r.bridge
            .relay_out(
                STAFF_RELAY_SINK,
                &OutputEvent::Sys(RichText::tag("system", "IPC boundary down")),
            )
            .await;
        r.bridge
            .relay_out(STAFF_RELAY_SINK, &OutputEvent::Ping { nonce: 1 })
            .await;
        let calls = r.api.calls();
        assert_eq!(calls.len(), 2);
        assert!(
            matches!(&calls[1], Call::Send { channel_id: 9090, msg } if msg.content == "IPC boundary down")
        );
        // No staff channel configured: dropped.
        let r0 = rig(|_| {}).await;
        r0.bridge.relay_out(STAFF_RELAY_SINK, &notice).await;
        assert!(r0.api.calls().is_empty());
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

    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn revocation_cancels_an_attached_alt_and_its_buttons() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        r.store.link_character("Nick", ALT, "Alt").await.unwrap();
        assert!(matches!(
            r.bridge.follow(USER, Some("Alt")).await,
            FollowOutcome::Following { attached: true, .. }
        ));
        let state = state_of(&r.bridge, USER);
        state.request_shown(9, shown(9, "Ask", RequestKind::Confirm, &[]));
        let control = BridgeControl(Arc::downgrade(&r.bridge));
        control.control(u64::from(ALT.0), DiscordControlOp::Revoke);
        assert!(
            state.cancelled.is_cancelled(),
            "invalidation is synchronous"
        );
        assert!(r.bridge.follow_of(USER).is_none());
        assert!(matches!(
            r.bridge.interaction(USER, click("d:req:9:yes", &[])).await,
            InteractionReply::Ephemeral(_)
        ));
        assert!(responds(&r.link).is_empty());
        settle().await;
        assert!(r.sessions.session_of_char(char_key(ALT)).is_none());
    }

    #[tokio::test]
    async fn revocation_while_creating_a_dm_cannot_restore_the_link_or_attach() {
        for banned in [false, true] {
            let r = rig(|_| {}).await;
            linked(&r.store, "Nick", USER, |_| {}).await;
            r.api.stall_create.store(true, Ordering::Relaxed);
            let bridge = Arc::clone(&r.bridge);
            let attach = tokio::spawn(async move { bridge.follow(USER, None).await });
            settle().await;
            assert!(
                r.api
                    .calls()
                    .iter()
                    .any(|c| matches!(c, Call::CreateDm { .. }))
            );
            if banned {
                r.store
                    .discord_ban_put(DiscordBanRec {
                        user_id: USER,
                        by_account: "Staff".into(),
                        reason: "test".into(),
                        at: 1,
                    })
                    .await
                    .unwrap();
            } else {
                r.store.set_account_discord("Nick", None).await.unwrap();
            }
            BridgeControl(Arc::downgrade(&r.bridge))
                .control(u64::from(NICKY.0), DiscordControlOp::Revoke);
            r.api.create_release.notify_one();
            assert!(!matches!(
                attach.await.unwrap(),
                FollowOutcome::Following { .. }
            ));
            assert!(r.bridge.follow_of(USER).is_none());
            assert!(r.sessions.session_of_char(char_key(NICKY)).is_none());
            assert!(!r.api.calls().iter().any(|c| matches!(c, Call::Send { .. })));
            if !banned {
                assert!(
                    r.store
                        .get_account("Nick")
                        .await
                        .unwrap()
                        .unwrap()
                        .discord
                        .is_none()
                );
            }
        }
    }

    #[tokio::test]
    async fn old_discord_message_cannot_answer_a_reused_request_id() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        r.bridge.follow(USER, None).await;
        let state = state_of(&r.bridge, USER);
        let mut fresh = shown(9, "Fresh", RequestKind::Confirm, &[]);
        fresh.message_id = 999;
        fresh.epoch = "new-world".into();
        state.request_shown(9, fresh);
        assert!(
            matches!(r.bridge.interaction(USER, click("d:req:9:yes", &[])).await, InteractionReply::Ephemeral(t) if t.contains("expired"))
        );
        assert!(responds(&r.link).is_empty());
        assert!(state.request(9).is_some());
        let mut current = click("d:req:9:yes", &[]);
        current.message_id = Some(999);
        r.bridge.interaction(USER, current).await;
        assert!(r.link.sent().iter().any(|s| matches!(s, ToServer::Inbound(WireInbound::Respond { epoch, .. }) if epoch == "new-world")));
    }

    #[tokio::test]
    async fn revocation_during_world_admission_leaves_no_connection() {
        let r = rig(|_| {}).await;
        linked(&r.store, "Nick", USER, |_| {}).await;
        r.link.stall_ready.store(true, Ordering::Relaxed);
        let bridge = r.bridge.clone();
        let attach = tokio::spawn(async move { bridge.follow(USER, None).await });
        settle().await;
        let (_, _, state) = r
            .bridge
            .follow_of(USER)
            .expect("published before world admission");
        assert!(r.sessions.session_of_char(char_key(NICKY)).is_some());
        r.store.set_account_discord("Nick", None).await.unwrap();
        BridgeControl(Arc::downgrade(&r.bridge))
            .control(u64::from(NICKY.0), DiscordControlOp::Revoke);
        assert!(state.cancelled.is_cancelled());
        r.link.ready_release.notify_one();
        assert!(!matches!(
            attach.await.unwrap(),
            FollowOutcome::Following { .. }
        ));
        settle().await;
        assert!(r.bridge.follow_of(USER).is_none());
        assert!(r.sessions.session_of_char(char_key(NICKY)).is_none());
    }
}
