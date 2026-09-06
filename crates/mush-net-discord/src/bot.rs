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
use crate::hooks::{
    DetachOutcome, DiscordHooks, FollowOutcome, Interaction, LineOutcome, LinkOutcome,
    UnlinkOutcome,
};

/// The window the `/link` attempt limiter counts over.
pub const LINK_ATTEMPT_WINDOW: Duration = Duration::from_secs(600);

/// How often an unfollowed user typing in the DM is reminded to `/follow`.
pub const HINT_INTERVAL: Duration = Duration::from_secs(600);

/// The slash commands the bot registers (guild-scoped, so they appear at once).
#[must_use]
pub fn commands() -> Vec<SlashCommand> {
    let text = |name: &'static str, description: &'static str| SlashOption {
        name,
        description,
        required: true,
    };
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
        SlashCommand {
            name: "follow",
            description: "Follow one of your characters here: their room, scene and pages, and act as them",
            options: vec![SlashOption {
                name: "character",
                description: "Which character (needed only if your account has several)",
                required: false,
            }],
        },
        SlashCommand {
            name: "detach",
            description: "Stop following; your character leaves the game as if you typed QUIT",
            options: vec![],
        },
        SlashCommand {
            name: "say",
            description: "Say something in your character's room",
            options: vec![text("text", "What to say")],
        },
        SlashCommand {
            name: "pose",
            description: "Pose (emote) as your character",
            options: vec![text("text", "The action, without your name")],
        },
        SlashCommand {
            name: "look",
            description: "Look at your character's room",
            options: vec![],
        },
        SlashCommand {
            name: "scene",
            description: "Show the scene your character is in",
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

/// Per-user state the loop keeps: the link limiter and the "use `/follow`"
/// hint's last time.
struct Loop {
    limiter: LinkLimiter,
    hinted: HashMap<u64, Instant>,
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
    api.register_commands(&commands()).await?;
    tracing::info!(
        guild = config.guild_id,
        "discord bot: slash commands registered (global; guild and bot-DM contexts)"
    );
    let mut state = Loop {
        limiter: LinkLimiter::new(config.link_attempts_per_10m),
        hinted: HashMap::new(),
    };
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
                handle_event(&*hooks, &*api, &mut state, event).await;
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

/// The reminder an unfollowed user gets when they type in the DM.
pub const NOT_FOLLOWING: &str = "You are not following a character. `/follow` (or `/follow <name>`) attaches one; then what you type here runs as them.";

fn follow_reply(outcome: &FollowOutcome) -> String {
    match outcome {
        FollowOutcome::Following {
            character,
            attached: true,
        } => format!(
            "Following **{character}**. What you type here runs as them — say, pose, `+roll`, \
             `respond <n>` to a request, `+scene/pause` and every other command. `/detach` (or \
             `QUIT`) stops; your terminal can join the same session any time."
        ),
        FollowOutcome::Following {
            character,
            attached: false,
        } => format!("Already following **{character}**."),
        FollowOutcome::NotLinked => "This Discord user is not linked to a game account. In the \
                                     game, `@discord/link` mints a code; then `/link <code>` here."
            .to_string(),
        FollowOutcome::Banned => "This Discord user may not follow.".to_string(),
        FollowOutcome::WhichCharacter { available } => format!(
            "Which character? `/follow <name>` — you have: {}.",
            available.join(", ")
        ),
        FollowOutcome::DmsClosed => "I cannot send you direct messages. Allow DMs from members \
                                     of this server (or message me first), then `/follow` again."
            .to_string(),
        FollowOutcome::Failed(why) => format!("Could not attach: {why}."),
    }
}

async fn handle_event(
    hooks: &dyn DiscordHooks,
    api: &dyn DiscordApi,
    state: &mut Loop,
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
        } => {
            let option = |key: &str| -> Option<&str> {
                options
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_str())
            };
            match name.as_str() {
                "link" => {
                    if !state.limiter.admit(user_id, Instant::now()) {
                        reply(
                            api,
                            &handle,
                            "Too many attempts; wait ten minutes and try again.",
                        )
                        .await;
                        return;
                    }
                    let Some(code) = normalize_link_code(option("code").unwrap_or("")) else {
                        reply(api, &handle, LINK_REFUSED).await;
                        return;
                    };
                    match hooks.link_redeem(user_id, &code).await {
                        LinkOutcome::Linked { account } => {
                            state.limiter.forgive(user_id);
                            reply(
                                api,
                                &handle,
                                &format!(
                                    "Linked to the game account **{account}**. `/follow` here \
                                     attaches one of its characters; `@discord` in the game shows \
                                     the link; `/unlink` here or `@discord/unlink` there removes it."
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
                "follow" => {
                    let outcome = hooks.follow(user_id, option("character")).await;
                    reply(api, &handle, &follow_reply(&outcome)).await;
                }
                "detach" => match hooks.detach(user_id).await {
                    DetachOutcome::Detached { character } => {
                        reply(api, &handle, &format!("Detached from **{character}**.")).await;
                    }
                    DetachOutcome::NotFollowing => {
                        reply(api, &handle, "You are not following a character.").await;
                    }
                },
                // Sugar: each of these is one command line, run exactly as
                // typed input would be (D-D-7).
                "say" | "pose" | "look" | "scene" => {
                    let line = match name.as_str() {
                        "say" => format!("say {}", option("text").unwrap_or("")),
                        "pose" => format!("pose {}", option("text").unwrap_or("")),
                        "look" => "look".to_string(),
                        _ => "+scene".to_string(),
                    };
                    match hooks.dm_line(user_id, &line).await {
                        LineOutcome::Accepted => reply(api, &handle, "Sent.").await,
                        LineOutcome::Closed => reply(api, &handle, "Detached.").await,
                        LineOutcome::NotFollowing => reply(api, &handle, NOT_FOLLOWING).await,
                    }
                }
                other => {
                    tracing::debug!(
                        command = other,
                        "discord bot: unknown slash command ignored"
                    );
                    reply(api, &handle, "That command is not available.").await;
                }
            }
        }
        DiscordEvent::Component {
            user_id,
            custom_id,
            values,
            channel_id,
            message_id,
            handle,
        } => {
            tracing::debug!(
                dm = channel_id.is_some(),
                "discord bot: component interaction"
            );
            let reply = hooks
                .interaction(
                    user_id,
                    Interaction {
                        custom_id,
                        values,
                        fields: Vec::new(),
                        channel_id,
                        message_id,
                    },
                )
                .await;
            if let Err(error) = api.respond_component(&handle, reply).await {
                tracing::warn!(%error, "discord bot: component reply failed");
            }
        }
        DiscordEvent::Modal {
            user_id,
            custom_id,
            fields,
            message_id,
            handle,
        } => {
            tracing::debug!(fields = fields.len(), "discord bot: modal submission");
            let reply = hooks
                .interaction(
                    user_id,
                    Interaction {
                        custom_id,
                        values: Vec::new(),
                        fields,
                        channel_id: None,
                        message_id,
                    },
                )
                .await;
            if let Err(error) = api.respond_component(&handle, reply).await {
                tracing::warn!(%error, "discord bot: modal reply failed");
            }
        }
        DiscordEvent::Message {
            user_id,
            channel_id,
            guild_id,
            content,
        } => {
            // Only a DM is a command line (D2); guild messages are the relay's (D4).
            // (The length is logged, never the text.)
            tracing::debug!(
                dm = guild_id.is_none(),
                chars = content.chars().count(),
                "discord bot: message event"
            );
            if guild_id.is_some() || content.trim().is_empty() {
                return;
            }
            match hooks.dm_line(user_id, &content).await {
                LineOutcome::Accepted | LineOutcome::Closed => {}
                LineOutcome::NotFollowing => {
                    let now = Instant::now();
                    let due = state
                        .hinted
                        .get(&user_id)
                        .is_none_or(|t| now.duration_since(*t) >= HINT_INTERVAL);
                    if due {
                        state.hinted.insert(user_id, now);
                        if let Err(error) =
                            api.send(channel_id, Outgoing::plain(NOT_FOLLOWING)).await
                        {
                            tracing::debug!(%error, "discord bot: follow hint failed");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::future::BoxFuture;

    use super::*;
    use crate::api::InteractionReply;
    use crate::api::testing::{Call, FakeApi, FakeEvents, component, modal, slash};
    use crate::hooks::WhoAmI;

    /// A scripted hooks impl that records what it was asked.
    #[derive(Default)]
    struct FakeHooks {
        linked: Mutex<Vec<(u64, String)>>,
        accept: Mutex<Vec<String>>,
        following: Mutex<Vec<u64>>,
        lines: Mutex<Vec<(u64, String)>>,
        interactions: Mutex<Vec<(u64, Interaction)>>,
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

        fn follow<'a>(
            &'a self,
            user_id: u64,
            character: Option<&'a str>,
        ) -> BoxFuture<'a, FollowOutcome> {
            Box::pin(async move {
                if user_id != 1 {
                    return FollowOutcome::NotLinked;
                }
                match character {
                    None => FollowOutcome::WhichCharacter {
                        available: vec!["Nicky".into(), "Alt".into()],
                    },
                    Some(name) => {
                        self.following.lock().unwrap().push(user_id);
                        FollowOutcome::Following {
                            character: name.to_string(),
                            attached: true,
                        }
                    }
                }
            })
        }

        fn detach(&self, user_id: u64) -> BoxFuture<'_, DetachOutcome> {
            Box::pin(async move {
                let mut f = self.following.lock().unwrap();
                if let Some(i) = f.iter().position(|u| *u == user_id) {
                    f.remove(i);
                    DetachOutcome::Detached {
                        character: "Nicky".into(),
                    }
                } else {
                    DetachOutcome::NotFollowing
                }
            })
        }

        fn dm_line<'a>(&'a self, user_id: u64, line: &'a str) -> BoxFuture<'a, LineOutcome> {
            Box::pin(async move {
                if !self.following.lock().unwrap().contains(&user_id) {
                    return LineOutcome::NotFollowing;
                }
                self.lines.lock().unwrap().push((user_id, line.to_string()));
                if line.eq_ignore_ascii_case("quit") {
                    self.following.lock().unwrap().retain(|u| *u != user_id);
                    LineOutcome::Closed
                } else {
                    LineOutcome::Accepted
                }
            })
        }

        fn interaction(
            &self,
            user_id: u64,
            interaction: Interaction,
        ) -> BoxFuture<'_, InteractionReply> {
            Box::pin(async move {
                let reply =
                    InteractionReply::Ephemeral(format!("routed {}", interaction.custom_id));
                self.interactions
                    .lock()
                    .unwrap()
                    .push((user_id, interaction));
                reply
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

    fn dm(user_id: u64, content: &str) -> DiscordEvent {
        DiscordEvent::Message {
            user_id,
            channel_id: 900,
            guild_id: None,
            content: content.to_string(),
        }
    }

    async fn run_events(events: Vec<DiscordEvent>) -> (Arc<FakeApi>, Arc<FakeHooks>) {
        let api = FakeApi::new();
        let hooks = Arc::new(FakeHooks::default());
        hooks.accept.lock().unwrap().push("ABCDEFGH".to_string());
        let (source, tx) = FakeEvents::new();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(
            hooks.clone(),
            api.clone(),
            Box::new(source),
            config(),
            async move {
                let _ = stop_rx.await;
            },
        ));
        for e in events {
            tx.send(e).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = stop_tx.send(());
        task.await.unwrap().unwrap();
        (api, hooks)
    }

    #[tokio::test]
    async fn registers_commands_then_links_with_a_normalized_code() {
        let (api, hooks) = run_events(vec![
            slash(1, "link", &[("code", "abcd-efgh")]),
            slash(1, "whoami", &[]),
            slash(2, "link", &[("code", "not a code")]),
            slash(2, "unlink", &[]),
        ])
        .await;
        let calls = api.calls();
        assert!(matches!(
            &calls[0],
            Call::RegisterCommands { names }
                if names == &["link", "unlink", "whoami", "follow", "detach", "say", "pose", "look", "scene"]
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
        assert!(
            r.iter().all(|t| !t.contains("ABCD")),
            "no reply echoes the code"
        );
    }

    #[tokio::test]
    async fn the_link_limiter_refuses_the_sixth_attempt_without_a_lookup() {
        // A code the hooks refuse: a success would forgive the window.
        let events: Vec<DiscordEvent> = (0..6)
            .map(|_| slash(3, "link", &[("code", "ABCDEFGJ")]))
            .collect();
        let (api, hooks) = run_events(events).await;
        assert_eq!(
            hooks.linked.lock().unwrap().len(),
            5,
            "five lookups, then none"
        );
        let r = replies(&api);
        assert_eq!(r.len(), 6);
        assert!(r[5].contains("Too many attempts"));
    }

    #[tokio::test]
    async fn follow_detach_sugar_and_dm_lines_route_to_the_hooks() {
        let (api, hooks) = run_events(vec![
            slash(1, "follow", &[]),
            slash(1, "follow", &[("character", "Nicky")]),
            slash(1, "say", &[("text", "hello")]),
            slash(1, "pose", &[("text", "waves")]),
            slash(1, "look", &[]),
            slash(1, "scene", &[]),
            dm(1, "+roll 3"),
            DiscordEvent::Message {
                user_id: 1,
                channel_id: 901,
                guild_id: Some(42),
                content: "guild chatter is not a line".into(),
            },
            dm(1, "QUIT"),
            slash(1, "detach", &[]),
            slash(2, "follow", &[("character", "X")]),
        ])
        .await;
        assert_eq!(
            hooks.lines.lock().unwrap().as_slice(),
            &[
                (1, "say hello".to_string()),
                (1, "pose waves".to_string()),
                (1, "look".to_string()),
                (1, "+scene".to_string()),
                (1, "+roll 3".to_string()),
                (1, "QUIT".to_string()),
            ]
        );
        let r = replies(&api);
        assert!(r[0].contains("Which character?") && r[0].contains("Nicky, Alt"));
        assert!(r[1].contains("Following **Nicky**"));
        assert_eq!(&r[2..6], ["Sent.", "Sent.", "Sent.", "Sent."]);
        assert!(
            r[6].contains("not following"),
            "QUIT closed the follow: {r:?}"
        );
        assert!(r[7].contains("not linked"));
    }

    #[tokio::test]
    async fn an_unfollowed_dm_gets_one_hint_per_interval() {
        let (api, hooks) = run_events(vec![dm(5, "look"), dm(5, "look again")]).await;
        assert!(hooks.lines.lock().unwrap().is_empty(), "nothing ran");
        let hints: Vec<Call> = api
            .calls()
            .into_iter()
            .filter(|c| {
                matches!(
                    c,
                    Call::Send {
                        channel_id: 900,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(hints.len(), 1, "{hints:?}");
    }

    #[tokio::test]
    async fn component_clicks_and_modal_submissions_route_to_the_hooks_and_are_answered() {
        let (api, hooks) = run_events(vec![
            component(1, "d:req:9:yes", &[], 500),
            component(1, "d:req:9:s", &["mara"], 500),
            modal(1, "d:actm:3", &[("line", "page Wiz=on my way")], Some(501)),
        ])
        .await;
        let routed = hooks.interactions.lock().unwrap();
        assert_eq!(routed.len(), 3);
        assert_eq!(routed[0].1.custom_id, "d:req:9:yes");
        assert_eq!(routed[0].1.message_id, Some(500));
        assert_eq!(routed[1].1.values, vec!["mara".to_string()]);
        assert_eq!(
            routed[2].1.fields,
            vec![("line".to_string(), "page Wiz=on my way".to_string())]
        );
        let answers: Vec<InteractionReply> = api
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Respond { reply, .. } => Some(reply),
                _ => None,
            })
            .collect();
        assert_eq!(answers.len(), 3, "every interaction is answered once");
        assert!(matches!(&answers[0], InteractionReply::Ephemeral(t) if t == "routed d:req:9:yes"));
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
