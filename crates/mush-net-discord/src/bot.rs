//! The bot task: register the DM slash commands, then turn Gateway events
//! into [`DiscordHooks`] calls and ephemeral replies.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_config::discord::normalize_link_code;

use crate::api::{
    DiscordApi, DiscordEvent, EventSource, InteractionHandle, Outgoing, SlashCommand, SlashOption,
};
use crate::config::DiscordConfig;
use crate::hooks::{DiscordHooks, LinkOutcome, UnlinkOutcome};

/// The window the `/link` attempt limiter counts over.
pub const LINK_ATTEMPT_WINDOW: Duration = Duration::from_secs(600);

/// The slash commands D1 registers (guild-scoped, so they appear at once).
#[must_use]
pub fn commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "link",
            description: "Bind this Discord user to your game account with a code from @discord/link",
            options: vec![SlashOption {
                name: "code",
                description: "The code the game showed you (ABCD-EFGH)",
                required: true,
            }],
        },
        SlashCommand {
            name: "unlink",
            description: "Remove the link between this Discord user and your game account",
            options: vec![],
        },
        SlashCommand {
            name: "whoami",
            description: "Which game account this Discord user is linked to",
            options: vec![],
        },
    ]
}

/// Why [`serve`] stopped.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// Registering the slash commands failed; the bot cannot be used.
    #[error("registering slash commands: {0}")]
    Register(#[from] crate::api::ApiError),
    /// The Gateway closed for good (an invalid token, a revoked bot).
    #[error("the Discord gateway closed permanently")]
    GatewayClosed,
}

/// The per-user `/link` attempt limiter: at most `max` failed attempts per
/// [`LINK_ATTEMPT_WINDOW`], counted **before** any lookup, so an attacker
/// cannot probe codes at store speed.
struct LinkLimiter {
    max: u32,
    attempts: HashMap<u64, VecDeque<Instant>>,
}

impl LinkLimiter {
    fn new(max: u32) -> LinkLimiter {
        LinkLimiter {
            max: max.max(1),
            attempts: HashMap::new(),
        }
    }

    /// Whether `user` may attempt now; records the attempt if so.
    fn admit(&mut self, user: u64, now: Instant) -> bool {
        let window = self.attempts.entry(user).or_default();
        while window
            .front()
            .is_some_and(|t| now.duration_since(*t) >= LINK_ATTEMPT_WINDOW)
        {
            window.pop_front();
        }
        if window.len() >= self.max as usize {
            return false;
        }
        window.push_back(now);
        true
    }

    /// A successful link clears the user's window.
    fn forgive(&mut self, user: u64) {
        self.attempts.remove(&user);
    }
}

/// The bot: registers the commands, then serves events until `shutdown`
/// resolves or the source ends. Message content never reaches a log above
/// `trace`; replies never echo a code.
///
/// # Errors
///
/// [`ServeError::Register`] if the commands cannot be registered;
/// [`ServeError::GatewayClosed`] if the event source ends before `shutdown`.
pub async fn serve(
    hooks: Arc<dyn DiscordHooks>,
    api: Arc<dyn DiscordApi>,
    mut events: Box<dyn EventSource>,
    config: DiscordConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    api.register_guild_commands(config.guild_id, &commands())
        .await?;
    tracing::info!(
        guild = config.guild_id,
        "discord bot: slash commands registered"
    );
    let mut limiter = LinkLimiter::new(config.link_attempts_per_10m);
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => {
                tracing::info!("discord bot: shutting down");
                return Ok(());
            }
            event = events.next() => {
                let Some(event) = event else {
                    tracing::error!("discord bot: gateway closed permanently");
                    return Err(ServeError::GatewayClosed);
                };
                handle_event(&*hooks, &*api, &mut limiter, event).await;
            }
        }
    }
}

async fn reply(api: &dyn DiscordApi, handle: &InteractionHandle, text: &str) {
    if let Err(error) = api
        .reply_interaction(handle, Outgoing::ephemeral(text))
        .await
    {
        tracing::warn!(%error, "discord bot: interaction reply failed");
    }
}

/// The uniform refusal for `/link` — one text for every reason (03 §3, R0).
pub const LINK_REFUSED: &str = "That code could not be linked. Codes come from `@discord/link` in the game, last ten minutes, and work once; check it and try again.";

async fn handle_event(
    hooks: &dyn DiscordHooks,
    api: &dyn DiscordApi,
    limiter: &mut LinkLimiter,
    event: DiscordEvent,
) {
    match event {
        DiscordEvent::Ready => {
            tracing::info!("discord bot: gateway ready");
        }
        DiscordEvent::SlashCommand {
            user_id,
            name,
            options,
            handle,
            ..
        } => match name.as_str() {
            "link" => {
                if !limiter.admit(user_id, Instant::now()) {
                    reply(
                        api,
                        &handle,
                        "Too many attempts; wait ten minutes and try again.",
                    )
                    .await;
                    return;
                }
                let raw = options
                    .iter()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                let Some(code) = normalize_link_code(raw) else {
                    reply(api, &handle, LINK_REFUSED).await;
                    return;
                };
                match hooks.link_redeem(user_id, &code).await {
                    LinkOutcome::Linked { account } => {
                        limiter.forgive(user_id);
                        reply(
                            api,
                            &handle,
                            &format!(
                                "Linked to the game account **{account}**. `@discord` in the game \
                                 shows the link; `/unlink` here or `@discord/unlink` there removes it."
                            ),
                        )
                        .await;
                    }
                    LinkOutcome::Refused => reply(api, &handle, LINK_REFUSED).await,
                }
            }
            "unlink" => match hooks.unlink(user_id).await {
                UnlinkOutcome::Unlinked { account } => {
                    reply(
                        api,
                        &handle,
                        &format!("Unlinked from the game account **{account}**."),
                    )
                    .await;
                }
                UnlinkOutcome::NotLinked => {
                    reply(
                        api,
                        &handle,
                        "This Discord user is not linked to a game account.",
                    )
                    .await;
                }
            },
            "whoami" => match hooks.whoami(user_id).await {
                Some(w) => {
                    reply(
                        api,
                        &handle,
                        &format!(
                            "Linked to the game account **{}** since unix {}; auto-follow {}.",
                            w.account,
                            w.linked_at,
                            if w.auto_follow { "on" } else { "off" }
                        ),
                    )
                    .await;
                }
                None => {
                    reply(api, &handle, "This Discord user is not linked to a game account. In the game, `@discord/link` mints a code; then `/link <code>` here.").await;
                }
            },
            other => {
                tracing::debug!(
                    command = other,
                    "discord bot: unknown slash command ignored"
                );
                reply(api, &handle, "That command is not available.").await;
            }
        },
        DiscordEvent::Message { .. } => {
            // D2 (DM lines) and D4 (relay) consume these; D1 ignores them.
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::future::BoxFuture;

    use super::*;
    use crate::api::testing::{Call, FakeApi, FakeEvents, slash};
    use crate::hooks::WhoAmI;

    /// A scripted hooks impl that records the normalized codes it was asked
    /// to redeem.
    #[derive(Default)]
    struct FakeHooks {
        linked: Mutex<Vec<(u64, String)>>,
        accept: Mutex<Vec<String>>,
    }

    impl DiscordHooks for FakeHooks {
        fn link_redeem<'a>(&'a self, user_id: u64, code: &'a str) -> BoxFuture<'a, LinkOutcome> {
            Box::pin(async move {
                self.linked
                    .lock()
                    .unwrap()
                    .push((user_id, code.to_string()));
                if self.accept.lock().unwrap().iter().any(|c| c == code) {
                    LinkOutcome::Linked {
                        account: "Nick".to_string(),
                    }
                } else {
                    LinkOutcome::Refused
                }
            })
        }

        fn unlink(&self, user_id: u64) -> BoxFuture<'_, UnlinkOutcome> {
            Box::pin(async move {
                if user_id == 1 {
                    UnlinkOutcome::Unlinked {
                        account: "Nick".to_string(),
                    }
                } else {
                    UnlinkOutcome::NotLinked
                }
            })
        }

        fn whoami(&self, user_id: u64) -> BoxFuture<'_, Option<WhoAmI>> {
            Box::pin(async move {
                (user_id == 1).then(|| WhoAmI {
                    account: "Nick".to_string(),
                    linked_at: 5,
                    auto_follow: true,
                })
            })
        }
    }

    fn config() -> DiscordConfig {
        DiscordConfig {
            enabled: true,
            application_id: 9,
            guild_id: 42,
            ..DiscordConfig::default()
        }
    }

    fn replies(api: &FakeApi) -> Vec<String> {
        api.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Reply { reply, .. } => {
                    assert!(reply.ephemeral, "every bot reply is ephemeral");
                    Some(reply.content)
                }
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn registers_commands_then_links_with_a_normalized_code() {
        let api = FakeApi::new();
        let hooks = Arc::new(FakeHooks::default());
        hooks.accept.lock().unwrap().push("ABCDEFGH".to_string());
        let (events, tx) = FakeEvents::new();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(
            hooks.clone(),
            api.clone(),
            Box::new(events),
            config(),
            async move {
                let _ = stop_rx.await;
            },
        ));
        tx.send(slash(1, "link", &[("code", "abcd-efgh")])).unwrap();
        tx.send(slash(1, "whoami", &[])).unwrap();
        tx.send(slash(2, "link", &[("code", "not a code")]))
            .unwrap();
        tx.send(slash(2, "unlink", &[])).unwrap();
        // Let the loop drain, then stop it.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = stop_tx.send(());
        task.await.unwrap().unwrap();

        let calls = api.calls();
        assert!(matches!(
            &calls[0],
            Call::RegisterCommands { guild_id: 42, names } if names == &["link", "unlink", "whoami"]
        ));
        assert_eq!(
            hooks.linked.lock().unwrap().as_slice(),
            &[(1, "ABCDEFGH".to_string())],
            "the code reaches the hooks normalized; a malformed code never does"
        );
        let r = replies(&api);
        assert!(r[0].contains("Linked to the game account **Nick**"));
        assert!(r[1].contains("since unix 5; auto-follow on"));
        assert_eq!(r[2], LINK_REFUSED);
        assert!(r[3].contains("not linked"));
        // No reply ever echoes the code.
        assert!(r.iter().all(|t| !t.contains("ABCD")));
    }

    #[tokio::test]
    async fn the_link_limiter_refuses_the_sixth_attempt_without_a_lookup() {
        let api = FakeApi::new();
        let hooks = Arc::new(FakeHooks::default());
        let (events, tx) = FakeEvents::new();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(
            hooks.clone(),
            api.clone(),
            Box::new(events),
            config(),
            async move {
                let _ = stop_rx.await;
            },
        ));
        for _ in 0..6 {
            tx.send(slash(3, "link", &[("code", "ABCDEFGH")])).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = stop_tx.send(());
        task.await.unwrap().unwrap();
        assert_eq!(
            hooks.linked.lock().unwrap().len(),
            5,
            "five lookups, then none"
        );
        let r = replies(&api);
        assert_eq!(r.len(), 6);
        assert!(r[5].contains("Too many attempts"));
    }

    #[test]
    fn the_limiter_window_slides_and_a_success_forgives() {
        let mut l = LinkLimiter::new(2);
        let t0 = Instant::now();
        assert!(l.admit(1, t0));
        assert!(l.admit(1, t0));
        assert!(!l.admit(1, t0));
        assert!(l.admit(2, t0), "per user");
        assert!(l.admit(1, t0 + LINK_ATTEMPT_WINDOW), "the window slid");
        l.forgive(1);
        assert!(l.admit(1, t0));
    }

    #[tokio::test]
    async fn a_closed_gateway_ends_serve_with_an_error() {
        let api = FakeApi::new();
        let hooks = Arc::new(FakeHooks::default());
        let (events, tx) = FakeEvents::new();
        drop(tx);
        let result = serve(
            hooks,
            api,
            Box::new(events),
            config(),
            std::future::pending(),
        )
        .await;
        assert!(matches!(result, Err(ServeError::GatewayClosed)));
    }
}
