//! The Discord API seam: [`DiscordApi`] (REST calls the bot makes),
//! [`EventSource`] (what the Gateway delivers), and the two implementations
//! of each — twilight for production ([`DiscordRest`], [`TwilightEvents`])
//! and in-memory fakes for every test ([`testing`]).
//!
//! [`DiscordRest`] is deliberately **REST-only and constructible from the
//! token alone**: it holds no Gateway connection and no world dependency, so
//! either process may hold one — the gateway for the bot, the server for the
//! on-call outbox adapter (03 §7a.2).

use std::fmt;
use std::sync::Arc;

use futures::future::BoxFuture;
use twilight_http::Client;
use twilight_http::error::ErrorType;
use twilight_model::application::command::CommandType;
use twilight_model::application::interaction::application_command::CommandOptionValue;
use twilight_model::application::interaction::{InteractionData, InteractionType};
use twilight_model::channel::message::MessageFlags;
use twilight_model::gateway::event::Event;
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseType};
use twilight_model::id::Id;
use twilight_model::id::marker::{
    ApplicationMarker, ChannelMarker, GuildMarker, InteractionMarker, MessageMarker, UserMarker,
};
use twilight_util::builder::InteractionResponseDataBuilder;
use twilight_util::builder::command::{CommandBuilder, StringBuilder};

/// Why a Discord call failed, classified the way the durable outbox needs
/// (retryable vs. permanent) plus the one permanent case the follow path
/// cares about by name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// Rate limited, a 5xx, a timeout, or a network fault: try again later.
    #[error("retryable: {0}")]
    Retryable(String),
    /// A 4xx that will not change on retry, or a malformed request.
    #[error("permanent: {0}")]
    Permanent(String),
    /// The user does not accept DMs from the bot (Discord error 50007).
    #[error("the user's DMs are closed")]
    CannotDm,
}

impl ApiError {
    /// Classify a twilight HTTP error. Never puts a body in the message
    /// beyond Discord's own error text (which carries no player data).
    fn from_http(error: &twilight_http::Error) -> ApiError {
        match error.kind() {
            ErrorType::Response { status, error, .. } => {
                let code = status.get();
                let text = format!("{status}: {error}");
                if code == 429 || code >= 500 {
                    ApiError::Retryable(text)
                } else if code == 403 && text.contains("50007") {
                    ApiError::CannotDm
                } else {
                    ApiError::Permanent(text)
                }
            }
            ErrorType::RequestTimedOut | ErrorType::RequestError => {
                ApiError::Retryable(error.to_string())
            }
            _ => ApiError::Permanent(error.to_string()),
        }
    }
}

/// A message the bot sends: content, and whether an interaction reply is
/// visible to the invoking user alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outgoing {
    /// The text.
    pub content: String,
    /// For interaction replies: ephemeral (only the invoker sees it).
    pub ephemeral: bool,
}

impl Outgoing {
    /// An ephemeral interaction reply.
    #[must_use]
    pub fn ephemeral(content: impl Into<String>) -> Outgoing {
        Outgoing {
            content: content.into(),
            ephemeral: true,
        }
    }

    /// A plain message.
    #[must_use]
    pub fn plain(content: impl Into<String>) -> Outgoing {
        Outgoing {
            content: content.into(),
            ephemeral: false,
        }
    }
}

/// One slash command the bot registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashCommand {
    /// The command name (`link`).
    pub name: &'static str,
    /// Its description.
    pub description: &'static str,
    /// String options, in order.
    pub options: Vec<SlashOption>,
}

/// One string option of a slash command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashOption {
    /// The option name.
    pub name: &'static str,
    /// Its description.
    pub description: &'static str,
    /// Whether Discord requires it.
    pub required: bool,
}

/// The token an interaction reply needs. `Debug` prints neither the id nor
/// the token (the token authorizes a reply for fifteen minutes).
#[derive(Clone, PartialEq, Eq)]
pub struct InteractionHandle {
    /// The interaction id.
    pub id: u64,
    /// The interaction token.
    pub token: String,
}

impl fmt::Debug for InteractionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InteractionHandle(<redacted>)")
    }
}

/// What the Gateway delivers, reduced to what the bot acts on. Identities
/// are Discord snowflakes; `Debug` prints them because this type never
/// reaches a log above `trace` (the bot task's rule), and the tests need to
/// read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscordEvent {
    /// The shard is connected and identified.
    Ready,
    /// A slash command was invoked.
    SlashCommand {
        /// The invoking Discord user.
        user_id: u64,
        /// The guild it was invoked in, if any (none in a DM).
        guild_id: Option<u64>,
        /// The command name.
        name: String,
        /// String options as `(name, value)`.
        options: Vec<(String, String)>,
        /// How to reply.
        handle: InteractionHandle,
    },
    /// A message from a non-bot author (D2 DMs, D4 relay). Delivered only
    /// when the shard subscribes to message events.
    Message {
        /// The author.
        user_id: u64,
        /// The channel.
        channel_id: u64,
        /// The guild, if any (none in a DM).
        guild_id: Option<u64>,
        /// The text.
        content: String,
    },
}

/// The REST calls the bot makes, object-safe so tests substitute a fake.
pub trait DiscordApi: Send + Sync {
    /// Replace the guild's slash commands with `commands`.
    fn register_guild_commands<'a>(
        &'a self,
        guild_id: u64,
        commands: &'a [SlashCommand],
    ) -> BoxFuture<'a, Result<(), ApiError>>;

    /// Answer an interaction (within Discord's three-second window).
    fn reply_interaction<'a>(
        &'a self,
        handle: &'a InteractionHandle,
        reply: Outgoing,
    ) -> BoxFuture<'a, Result<(), ApiError>>;

    /// Open (or find) the DM channel with a user. [`ApiError::CannotDm`] when
    /// the user does not accept DMs from the bot.
    fn create_dm(&self, user_id: u64) -> BoxFuture<'_, Result<u64, ApiError>>;

    /// Send a message to a channel; returns the message id.
    fn send(&self, channel_id: u64, msg: Outgoing) -> BoxFuture<'_, Result<u64, ApiError>>;

    /// Replace a message's content.
    fn edit(
        &self,
        channel_id: u64,
        message_id: u64,
        msg: Outgoing,
    ) -> BoxFuture<'_, Result<(), ApiError>>;
}

/// Where Gateway events come from. The bot loop owns exactly one.
pub trait EventSource: Send {
    /// The next event, or `None` once the source is finished for good (a
    /// fatal Gateway close such as an invalid token).
    fn next(&mut self) -> BoxFuture<'_, Option<DiscordEvent>>;
}

fn id<M>(n: u64, what: &str) -> Result<Id<M>, ApiError> {
    Id::new_checked(n).ok_or_else(|| ApiError::Permanent(format!("{what} id is zero")))
}

/// The production REST client over `twilight_http`: token plus application
/// id, nothing else. `Clone` shares the underlying client (and its
/// per-route rate limiter).
#[derive(Clone)]
pub struct DiscordRest {
    client: Arc<Client>,
    application_id: u64,
}

impl fmt::Debug for DiscordRest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordRest")
            .field("application_id", &self.application_id)
            .finish_non_exhaustive()
    }
}

impl DiscordRest {
    /// A client for the bot `token` acting as `application_id`.
    #[must_use]
    pub fn new(token: String, application_id: u64) -> DiscordRest {
        DiscordRest {
            client: Arc::new(Client::new(token)),
            application_id,
        }
    }

    fn app(&self) -> Result<Id<ApplicationMarker>, ApiError> {
        id(self.application_id, "application")
    }
}

impl DiscordApi for DiscordRest {
    fn register_guild_commands<'a>(
        &'a self,
        guild_id: u64,
        commands: &'a [SlashCommand],
    ) -> BoxFuture<'a, Result<(), ApiError>> {
        Box::pin(async move {
            let guild: Id<GuildMarker> = id(guild_id, "guild")?;
            let app = self.app()?;
            let built: Vec<_> = commands
                .iter()
                .map(|c| {
                    let mut b = CommandBuilder::new(c.name, c.description, CommandType::ChatInput);
                    for o in &c.options {
                        b = b.option(
                            StringBuilder::new(o.name, o.description)
                                .required(o.required)
                                .build(),
                        );
                    }
                    b.build()
                })
                .collect();
            self.client
                .interaction(app)
                .set_guild_commands(guild, &built)
                .await
                .map(|_| ())
                .map_err(|e| ApiError::from_http(&e))
        })
    }

    fn reply_interaction<'a>(
        &'a self,
        handle: &'a InteractionHandle,
        reply: Outgoing,
    ) -> BoxFuture<'a, Result<(), ApiError>> {
        Box::pin(async move {
            let app = self.app()?;
            let interaction: Id<InteractionMarker> = id(handle.id, "interaction")?;
            let mut data = InteractionResponseDataBuilder::new().content(reply.content);
            if reply.ephemeral {
                data = data.flags(MessageFlags::EPHEMERAL);
            }
            let response = InteractionResponse {
                kind: InteractionResponseType::ChannelMessageWithSource,
                data: Some(data.build()),
            };
            self.client
                .interaction(app)
                .create_response(interaction, &handle.token, &response)
                .await
                .map(|_| ())
                .map_err(|e| ApiError::from_http(&e))
        })
    }

    fn create_dm(&self, user_id: u64) -> BoxFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let user: Id<UserMarker> = id(user_id, "user")?;
            let channel = self
                .client
                .create_private_channel(user)
                .await
                .map_err(|e| ApiError::from_http(&e))?
                .model()
                .await
                .map_err(|e| ApiError::Permanent(format!("decoding the DM channel: {e}")))?;
            Ok(channel.id.get())
        })
    }

    fn send(&self, channel_id: u64, msg: Outgoing) -> BoxFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let channel: Id<ChannelMarker> = id(channel_id, "channel")?;
            let message = self
                .client
                .create_message(channel)
                .content(&msg.content)
                .await
                .map_err(|e| ApiError::from_http(&e))?
                .model()
                .await
                .map_err(|e| ApiError::Permanent(format!("decoding the message: {e}")))?;
            Ok(message.id.get())
        })
    }

    fn edit(
        &self,
        channel_id: u64,
        message_id: u64,
        msg: Outgoing,
    ) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let channel: Id<ChannelMarker> = id(channel_id, "channel")?;
            let message: Id<MessageMarker> = id(message_id, "message")?;
            self.client
                .update_message(channel, message)
                .content(Some(&msg.content))
                .await
                .map(|_| ())
                .map_err(|e| ApiError::from_http(&e))
        })
    }
}

/// The production [`EventSource`]: one Gateway shard, reconnecting on its
/// own; `next` ends only on a fatal close.
pub struct TwilightEvents {
    shard: twilight_gateway::Shard,
}

impl TwilightEvents {
    /// One shard for `token` with `intents`. D1 needs no privileged intent:
    /// slash commands arrive regardless, and DM content is exempt.
    #[must_use]
    pub fn new(token: String, intents: twilight_gateway::Intents) -> TwilightEvents {
        TwilightEvents {
            shard: twilight_gateway::Shard::new(twilight_gateway::ShardId::ONE, token, intents),
        }
    }
}

/// Reduce a Gateway event to what the bot acts on; `None` for everything else.
fn reduce(event: Event) -> Option<DiscordEvent> {
    match event {
        Event::Ready(_) => Some(DiscordEvent::Ready),
        Event::InteractionCreate(ic) => {
            let interaction = ic.0;
            if interaction.kind != InteractionType::ApplicationCommand {
                return None;
            }
            let user_id = interaction.author_id()?.get();
            let Some(InteractionData::ApplicationCommand(cmd)) = interaction.data else {
                return None;
            };
            let options = cmd
                .options
                .iter()
                .filter_map(|o| match &o.value {
                    CommandOptionValue::String(s) => Some((o.name.clone(), s.clone())),
                    _ => None,
                })
                .collect();
            Some(DiscordEvent::SlashCommand {
                user_id,
                guild_id: interaction.guild_id.map(Id::get),
                name: cmd.name,
                options,
                handle: InteractionHandle {
                    id: interaction.id.get(),
                    token: interaction.token,
                },
            })
        }
        Event::MessageCreate(mc) => {
            let message = mc.0;
            if message.author.bot {
                return None;
            }
            Some(DiscordEvent::Message {
                user_id: message.author.id.get(),
                channel_id: message.channel_id.get(),
                guild_id: message.guild_id.map(Id::get),
                content: message.content,
            })
        }
        _ => None,
    }
}

impl EventSource for TwilightEvents {
    fn next(&mut self) -> BoxFuture<'_, Option<DiscordEvent>> {
        use twilight_gateway::{EventTypeFlags, StreamExt};
        Box::pin(async move {
            let wanted = EventTypeFlags::READY
                | EventTypeFlags::INTERACTION_CREATE
                | EventTypeFlags::MESSAGE_CREATE;
            loop {
                match self.shard.next_event(wanted).await {
                    Some(Ok(event)) => {
                        if let Some(reduced) = reduce(event) {
                            return Some(reduced);
                        }
                    }
                    Some(Err(source)) => {
                        // Non-fatal: the shard reconnects on its own.
                        tracing::warn!(error = %source, "discord gateway: receive error");
                    }
                    None => return None,
                }
            }
        })
    }
}

/// In-memory fakes for tests: a scripted event source and a recording API.
pub mod testing {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use futures::future::BoxFuture;
    use tokio::sync::mpsc;

    use super::{
        ApiError, DiscordApi, DiscordEvent, EventSource, InteractionHandle, Outgoing, SlashCommand,
    };

    /// One recorded REST call.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Call {
        /// `register_guild_commands`.
        RegisterCommands {
            /// The guild.
            guild_id: u64,
            /// The command names.
            names: Vec<String>,
        },
        /// `reply_interaction`.
        Reply {
            /// The interaction id.
            interaction_id: u64,
            /// The reply.
            reply: Outgoing,
        },
        /// `create_dm`.
        CreateDm {
            /// The user.
            user_id: u64,
        },
        /// `send`.
        Send {
            /// The channel.
            channel_id: u64,
            /// The message.
            msg: Outgoing,
        },
        /// `edit`.
        Edit {
            /// The channel.
            channel_id: u64,
            /// The message.
            message_id: u64,
            /// The new content.
            msg: Outgoing,
        },
    }

    /// A recording [`DiscordApi`] with scriptable failures.
    #[derive(Default)]
    pub struct FakeApi {
        calls: Mutex<Vec<Call>>,
        /// Errors returned by the next calls, in order; `None` means success.
        failures: Mutex<VecDeque<Option<ApiError>>>,
        next_id: Mutex<u64>,
        /// Users whose DMs are closed.
        closed_dms: Mutex<Vec<u64>>,
    }

    impl FakeApi {
        /// A fresh fake.
        #[must_use]
        pub fn new() -> Arc<FakeApi> {
            Arc::new(FakeApi::default())
        }

        /// Every call so far, in order.
        #[must_use]
        pub fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("fake api poisoned").clone()
        }

        /// Script the outcome of the next N calls (`None` = success).
        pub fn script(&self, outcomes: impl IntoIterator<Item = Option<ApiError>>) {
            self.failures
                .lock()
                .expect("fake api poisoned")
                .extend(outcomes);
        }

        /// Make `create_dm` fail with [`ApiError::CannotDm`] for `user_id`.
        pub fn close_dms(&self, user_id: u64) {
            self.closed_dms
                .lock()
                .expect("fake api poisoned")
                .push(user_id);
        }

        fn record(&self, call: Call) -> Result<(), ApiError> {
            self.calls.lock().expect("fake api poisoned").push(call);
            match self.failures.lock().expect("fake api poisoned").pop_front() {
                Some(Some(err)) => Err(err),
                _ => Ok(()),
            }
        }

        fn alloc(&self) -> u64 {
            let mut n = self.next_id.lock().expect("fake api poisoned");
            *n += 1;
            *n + 1_000
        }
    }

    impl DiscordApi for FakeApi {
        fn register_guild_commands<'a>(
            &'a self,
            guild_id: u64,
            commands: &'a [SlashCommand],
        ) -> BoxFuture<'a, Result<(), ApiError>> {
            Box::pin(async move {
                self.record(Call::RegisterCommands {
                    guild_id,
                    names: commands.iter().map(|c| c.name.to_string()).collect(),
                })
            })
        }

        fn reply_interaction<'a>(
            &'a self,
            handle: &'a InteractionHandle,
            reply: Outgoing,
        ) -> BoxFuture<'a, Result<(), ApiError>> {
            Box::pin(async move {
                self.record(Call::Reply {
                    interaction_id: handle.id,
                    reply,
                })
            })
        }

        fn create_dm(&self, user_id: u64) -> BoxFuture<'_, Result<u64, ApiError>> {
            Box::pin(async move {
                self.record(Call::CreateDm { user_id })?;
                if self
                    .closed_dms
                    .lock()
                    .expect("fake api poisoned")
                    .contains(&user_id)
                {
                    return Err(ApiError::CannotDm);
                }
                Ok(self.alloc())
            })
        }

        fn send(&self, channel_id: u64, msg: Outgoing) -> BoxFuture<'_, Result<u64, ApiError>> {
            Box::pin(async move {
                self.record(Call::Send { channel_id, msg })?;
                Ok(self.alloc())
            })
        }

        fn edit(
            &self,
            channel_id: u64,
            message_id: u64,
            msg: Outgoing,
        ) -> BoxFuture<'_, Result<(), ApiError>> {
            Box::pin(async move {
                self.record(Call::Edit {
                    channel_id,
                    message_id,
                    msg,
                })
            })
        }
    }

    /// A scripted [`EventSource`]: events pushed through a channel; the
    /// source ends when the sender is dropped.
    pub struct FakeEvents {
        rx: mpsc::UnboundedReceiver<DiscordEvent>,
    }

    impl FakeEvents {
        /// A source and the handle that feeds it.
        #[must_use]
        pub fn new() -> (FakeEvents, mpsc::UnboundedSender<DiscordEvent>) {
            let (tx, rx) = mpsc::unbounded_channel();
            (FakeEvents { rx }, tx)
        }
    }

    impl EventSource for FakeEvents {
        fn next(&mut self) -> BoxFuture<'_, Option<DiscordEvent>> {
            Box::pin(async move { self.rx.recv().await })
        }
    }

    /// A slash-command event for tests.
    #[must_use]
    pub fn slash(user_id: u64, name: &str, options: &[(&str, &str)]) -> DiscordEvent {
        DiscordEvent::SlashCommand {
            user_id,
            guild_id: None,
            name: name.to_string(),
            options: options
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            handle: InteractionHandle {
                id: 77,
                token: "tok".to_string(),
            },
        }
    }
}
