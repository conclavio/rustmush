//! The bridge between the Discord transport (`mush-net-discord`) and the
//! game: the [`mush_net_discord::DiscordHooks`] implementation (D1).
//!
//! ## Division of labor (mirrors [`crate::ssh_bridge`] / [`crate::web_bridge`])
//!
//! - The **transport** owns the Gateway shard, REST, the slash-command
//!   surface and the `/link` attempt limiter; it hands this bridge a
//!   Discord-asserted user id and a **normalized** code.
//! - The **bridge** owns everything game-side: the link flow against the
//!   store — the code is taken once (`discord_code_take`), the ban and the
//!   both-sides-free checks, the reverse-index reservation that is the
//!   uniqueness lock, then the account write — and the in-game notice to
//!   the account's connected characters through the shared
//!   [`SessionManager`]. D2 adds the follow connection on top of the same
//!   struct.
//!
//! ## Refusals are uniform (R0, 03 §3)
//!
//! Every reason a redeem fails — unknown, expired or used code; a banned
//! user; an account or a user already linked — is one [`LinkOutcome::Refused`]
//! and one reply text, so the bot is never an oracle on which accounts exist
//! or are linked. The bridge logs the reason class at `info`, never the code.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_config::discord::link_code_hash;
use mush_config::unix_now_s;
use mush_net_discord::{
    DiscordApi, DiscordConfig, DiscordHooks, DiscordRest, Intents, LinkOutcome, TwilightEvents,
    UnlinkOutcome, WhoAmI,
};
use mush_proto::{OutputEvent, RichText};
use mush_session::SessionManager;
use mush_store::{AccountRec, DiscordLinkRec, ObjectStore};
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::delivery::char_key;

/// The running bot task, or the absence of one. Both binaries hold one and
/// call [`DiscordTask::stop`] in their shutdown tails.
pub struct DiscordTask {
    stop: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl DiscordTask {
    /// No bot (disabled, misconfigured, or no token).
    #[must_use]
    pub fn none() -> DiscordTask {
        DiscordTask {
            stop: None,
            join: None,
        }
    }

    /// Whether a bot task was started.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.join.is_some()
    }

    /// Stop the bot gracefully (a no-op when none started): fire its stop
    /// signal and wait a bounded time.
    pub async fn stop(self) {
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

/// Start the Discord bot from `[discord]` if it is enabled and usable:
/// the [`DiscordBridge`] over the shared session manager and store, a
/// [`DiscordRest`] client, and one Gateway shard. Every failure to start is
/// **loud but non-fatal** (the web listener's W4 rule): SSH and web players
/// keep playing while the operator fixes the config or the environment.
pub fn spawn_bot(
    config: &Config,
    sessions: Arc<SessionManager>,
    store: Arc<dyn ObjectStore>,
) -> DiscordTask {
    let dc = match DiscordConfig::deserialize(config.discord.clone()) {
        Ok(dc) => dc,
        Err(e) => {
            tracing::error!(error = %e, "[discord] config invalid; Discord disabled");
            return DiscordTask::none();
        }
    };
    if !dc.enabled {
        return DiscordTask::none();
    }
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
    // D1 needs no Gateway intent: slash commands arrive regardless. D2 adds
    // DIRECT_MESSAGES, D4 the guild message intents.
    let events = Box::new(TwilightEvents::new(token, Intents::empty()));
    let hooks: Arc<dyn DiscordHooks> =
        Arc::new(DiscordBridge::new(sessions, store, dc.auto_follow_default));
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
    }
}

/// The production [`DiscordHooks`]: the shared session manager (for in-game
/// notices) and the store (the link records).
pub struct DiscordBridge {
    sessions: Arc<SessionManager>,
    store: Arc<dyn ObjectStore>,
    /// The `auto_follow` a new link starts with (`[discord].auto_follow_default`).
    auto_follow_default: bool,
}

impl DiscordBridge {
    /// Wire the bridge.
    #[must_use]
    pub fn new(
        sessions: Arc<SessionManager>,
        store: Arc<dyn ObjectStore>,
        auto_follow_default: bool,
    ) -> DiscordBridge {
        DiscordBridge {
            sessions,
            store,
            auto_follow_default,
        }
    }

    /// Tell every connected character of `account` something about its
    /// Discord link (a `Sys` line tagged `discord`, to their own connections
    /// only). A character with no live session is simply not told.
    fn notify_account(&self, account: &AccountRec, text: &str) {
        for character in &account.characters {
            let key = char_key(character.id);
            if self.sessions.session_of_char(key).is_some() {
                let _ = self.sessions.send_to_char(
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
}

impl DiscordHooks for DiscordBridge {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_config::discord::normalize_link_code;
    use mush_core::ObjId;
    use mush_session::SessionConfig;
    use mush_store::{DiscordBanRec, DiscordCodeRecord, InMemoryStore};

    const USER: u64 = 123_456_789_012_345_678;
    const OTHER: u64 = 987_654_321_098_765_432;

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
        store.link_character(name, ObjId(7), "Nicky").await.unwrap();
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

    fn bridge(store: Arc<InMemoryStore>) -> DiscordBridge {
        DiscordBridge::new(
            Arc::new(SessionManager::with_config(SessionConfig::default())),
            store,
            true,
        )
    }

    #[tokio::test]
    async fn a_minted_code_links_once_and_whoami_and_unlink_follow() {
        let store = store_with("Nick").await;
        let code = mint(&store, "Nick", "abcd-efgh", unix_now_s() + 600).await;
        let b = bridge(store.clone());
        assert_eq!(
            b.link_redeem(USER, &code).await,
            LinkOutcome::Linked {
                account: "Nick".into()
            }
        );
        let rec = store.get_account("nick").await.unwrap().unwrap();
        let link = rec.discord.as_ref().expect("linked");
        assert_eq!(link.user_id, USER);
        assert!(link.auto_follow, "the configured default applies");
        assert_eq!(
            store
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
        assert!(who.auto_follow);
        assert!(b.whoami(OTHER).await.is_none());
        assert_eq!(
            b.unlink(USER).await,
            UnlinkOutcome::Unlinked {
                account: "Nick".into()
            }
        );
        assert_eq!(b.unlink(USER).await, UnlinkOutcome::NotLinked);
        assert!(
            store
                .get_account("nick")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
        assert!(store.account_by_discord_user(USER).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn refusals_are_uniform_expired_banned_already_linked_either_side() {
        let store = store_with("Nick").await;
        let b = bridge(store.clone());
        // Unknown and expired codes.
        assert_eq!(b.link_redeem(USER, "ABCDEFGH").await, LinkOutcome::Refused);
        let dead = mint(&store, "Nick", "ABCD-EFGJ", unix_now_s() - 1).await;
        assert_eq!(b.link_redeem(USER, &dead).await, LinkOutcome::Refused);
        // A banned user is refused even with a good code (and the code is spent).
        store
            .discord_ban_put(DiscordBanRec {
                user_id: USER,
                by_account: "wiz".into(),
                reason: "spam".into(),
                at: 1,
            })
            .await
            .unwrap();
        let good = mint(&store, "Nick", "ABCD-EFGK", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(USER, &good).await, LinkOutcome::Refused);
        assert!(
            store
                .get_account("nick")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
        store.discord_ban_del(USER).await.unwrap();
        // Link, then: the account is taken for OTHER, and USER is taken for a
        // second account.
        let good = mint(&store, "Nick", "ABCD-EFGL", unix_now_s() + 600).await;
        assert!(matches!(
            b.link_redeem(USER, &good).await,
            LinkOutcome::Linked { .. }
        ));
        let again = mint(&store, "Nick", "ABCD-EFGM", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(OTHER, &again).await, LinkOutcome::Refused);
        store
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
        let zoe = mint(&store, "Zoe", "ABCD-EFGN", unix_now_s() + 600).await;
        assert_eq!(b.link_redeem(USER, &zoe).await, LinkOutcome::Refused);
        assert!(
            store
                .get_account("zoe")
                .await
                .unwrap()
                .unwrap()
                .discord
                .is_none()
        );
    }
}
