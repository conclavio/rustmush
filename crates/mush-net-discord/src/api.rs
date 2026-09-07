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
use twilight_model::application::interaction::modal::ModalInteractionComponent;
use twilight_model::application::interaction::{
    InteractionContextType, InteractionData, InteractionType,
};
use twilight_model::channel::message::MessageFlags;
use twilight_model::gateway::event::Event;
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseType};
use twilight_model::id::Id;
use twilight_model::id::marker::{
    ApplicationMarker, ChannelMarker, InteractionMarker, MessageMarker, UserMarker,
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

/// Discord's `custom_id` cap, bytes.
pub const MAX_CUSTOM_ID_BYTES: usize = 100;
/// Discord's buttons-per-row cap.
pub const MAX_BUTTONS_PER_ROW: usize = 5;
/// Discord's action-rows-per-message cap.
pub const MAX_ROWS: usize = 5;
/// Discord's options-per-select cap.
pub const MAX_SELECT_OPTIONS: usize = 25;
/// Discord's button-label cap, characters.
pub const MAX_BUTTON_LABEL_CHARS: usize = 80;

/// A button's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Blurple: the main action.
    Primary,
    /// Grey: a neutral action.
    Secondary,
    /// Green: yes / agree.
    Success,
    /// Red: no / decline.
    Danger,
}

/// One clickable button (D3). The click arrives as
/// [`DiscordEvent::Component`] carrying `custom_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    /// The routing key (`d:req:<id>:<opt>`), at most [`MAX_CUSTOM_ID_BYTES`].
    pub custom_id: String,
    /// The text on the button, at most [`MAX_BUTTON_LABEL_CHARS`].
    pub label: String,
    /// Its colour.
    pub style: ButtonStyle,
}

/// One single-choice string select (D3). The pick arrives as
/// [`DiscordEvent::Component`] with the chosen value in `values`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Select {
    /// The routing key.
    pub custom_id: String,
    /// Shown before a pick.
    pub placeholder: String,
    /// `(value, label)` pairs, at most [`MAX_SELECT_OPTIONS`].
    pub options: Vec<(String, String)>,
}

/// One action row of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// Up to [`MAX_BUTTONS_PER_ROW`] buttons.
    Buttons(Vec<Button>),
    /// One select menu.
    Select(Select),
}

/// A modal with one text field (D3), opened in answer to a component click.
/// Its submission arrives as [`DiscordEvent::Modal`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModalSpec {
    /// The routing key of the submission.
    pub custom_id: String,
    /// The modal's title (≤ 45 chars).
    pub title: String,
    /// The one field.
    pub field: TextField,
}

/// The text field of a [`ModalSpec`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextField {
    /// The field's key in the submission.
    pub custom_id: String,
    /// The label above the field.
    pub label: String,
    /// A prefilled value (the `Notice` action's `prefill`).
    pub value: Option<String>,
    /// The longest accepted value, characters (≤ 4 000).
    pub max_len: u16,
    /// Multi-line (paragraph) rather than single-line.
    pub paragraph: bool,
}

/// How the bot answers a component click or a modal submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractionReply {
    /// Acknowledge without changing anything (a deferred update).
    Ack,
    /// A private text reply to the clicker.
    Ephemeral(String),
    /// Replace the message the component was on — content and components
    /// (an empty `components` removes them).
    UpdateMessage(Outgoing),
    /// Open a modal.
    Modal(ModalSpec),
}

/// A message the bot sends: content, whether an interaction reply is
/// visible to the invoking user alone, and any components (D3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outgoing {
    /// The text.
    pub content: String,
    /// For interaction replies: ephemeral (only the invoker sees it).
    pub ephemeral: bool,
    /// Action rows under the text (at most [`MAX_ROWS`]).
    pub components: Vec<Row>,
}

impl Outgoing {
    /// An ephemeral interaction reply.
    #[must_use]
    pub fn ephemeral(content: impl Into<String>) -> Outgoing {
        Outgoing {
            content: content.into(),
            ephemeral: true,
            components: Vec::new(),
        }
    }

    /// A plain message.
    #[must_use]
    pub fn plain(content: impl Into<String>) -> Outgoing {
        Outgoing {
            content: content.into(),
            ephemeral: false,
            components: Vec::new(),
        }
    }

    /// The same message with these action rows.
    #[must_use]
    pub fn with_components(mut self, rows: Vec<Row>) -> Outgoing {
        self.components = rows;
        self
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
    /// A button click or a select pick (D3).
    Component {
        /// The clicking Discord user.
        user_id: u64,
        /// The component's routing key.
        custom_id: String,
        /// The picked values (one for a single select; empty for a button).
        values: Vec<String>,
        /// The channel the message is in.
        channel_id: Option<u64>,
        /// The message the component is on.
        message_id: Option<u64>,
        /// How to reply.
        handle: InteractionHandle,
    },
    /// A modal submission (D3).
    Modal {
        /// The submitting Discord user.
        user_id: u64,
        /// The modal's routing key.
        custom_id: String,
        /// `(field custom_id, value)` for every text field.
        fields: Vec<(String, String)>,
        /// The message whose component opened the modal, when known.
        message_id: Option<u64>,
        /// How to reply.
        handle: InteractionHandle,
    },
}

/// The REST calls the bot makes, object-safe so tests substitute a fake.
pub trait DiscordApi: Send + Sync {
    /// Replace the application's slash commands with `commands`. They are
    /// registered **globally** with the guild and bot-DM contexts: a
    /// guild-scoped command is never offered inside the bot's DM, and the DM
    /// is where `/link` and `/follow` live.
    fn register_commands<'a>(
        &'a self,
        commands: &'a [SlashCommand],
    ) -> BoxFuture<'a, Result<(), ApiError>>;

    /// Answer a slash command with a message (within Discord's three-second
    /// window).
    fn reply_interaction<'a>(
        &'a self,
        handle: &'a InteractionHandle,
        reply: Outgoing,
    ) -> BoxFuture<'a, Result<(), ApiError>>;

    /// Answer a component click or a modal submission (D3), within the same
    /// window.
    fn respond_component<'a>(
        &'a self,
        handle: &'a InteractionHandle,
        reply: InteractionReply,
    ) -> BoxFuture<'a, Result<(), ApiError>>;

    /// Post `content` into a channel under `username` through the bot's
    /// webhook for that channel (D4: a relayed line appears as the character,
    /// not as the bot). The implementation finds or creates the webhook and
    /// caches it. Returns the message id.
    fn webhook_send<'a>(
        &'a self,
        channel_id: u64,
        username: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<u64, ApiError>>;

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
    /// The relay webhooks found or created per channel (D4): channel id →
    /// `(webhook id, token)`. A token is a capability; it stays in memory.
    webhooks: Arc<std::sync::Mutex<std::collections::HashMap<u64, (u64, String)>>>,
}

impl fmt::Debug for DiscordRest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordRest")
            .field("application_id", &self.application_id)
            .finish_non_exhaustive()
    }
}

/// The name of the webhook the bot creates in a relayed channel.
pub const WEBHOOK_NAME: &str = "rustmush";

impl DiscordRest {
    /// The relay webhook for `channel`: cached, else an existing one of ours
    /// with a token, else a new one.
    async fn webhook_for(&self, channel_id: u64) -> Result<(u64, String), ApiError> {
        if let Some(found) = self
            .webhooks
            .lock()
            .expect("webhook cache poisoned")
            .get(&channel_id)
            .cloned()
        {
            return Ok(found);
        }
        let channel: Id<ChannelMarker> = id(channel_id, "channel")?;
        let existing = self
            .client
            .channel_webhooks(channel)
            .await
            .map_err(|e| ApiError::from_http(&e))?
            .models()
            .await
            .map_err(|e| ApiError::Permanent(format!("decoding webhooks: {e}")))?;
        let ours = existing.into_iter().find_map(|w| {
            (w.name.as_deref() == Some(WEBHOOK_NAME))
                .then_some(())
                .and(w.token.map(|t| (w.id.get(), t)))
        });
        let hook = match ours {
            Some(h) => h,
            None => {
                let created = self
                    .client
                    .create_webhook(channel, WEBHOOK_NAME)
                    .await
                    .map_err(|e| ApiError::from_http(&e))?
                    .model()
                    .await
                    .map_err(|e| ApiError::Permanent(format!("decoding the webhook: {e}")))?;
                let token = created.token.ok_or_else(|| {
                    ApiError::Permanent("the created webhook carried no token".to_string())
                })?;
                (created.id.get(), token)
            }
        };
        self.webhooks
            .lock()
            .expect("webhook cache poisoned")
            .insert(channel_id, hook.clone());
        Ok(hook)
    }
    /// A client for the bot `token` acting as `application_id`.
    #[must_use]
    pub fn new(token: String, application_id: u64) -> DiscordRest {
        DiscordRest {
            client: Arc::new(Client::new(token)),
            application_id,
            webhooks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn app(&self) -> Result<Id<ApplicationMarker>, ApiError> {
        id(self.application_id, "application")
    }
}

impl DiscordApi for DiscordRest {
    fn register_commands<'a>(
        &'a self,
        commands: &'a [SlashCommand],
    ) -> BoxFuture<'a, Result<(), ApiError>> {
        Box::pin(async move {
            let app = self.app()?;
            let built: Vec<_> = commands
                .iter()
                .map(|c| {
                    let mut b = CommandBuilder::new(c.name, c.description, CommandType::ChatInput)
                        // Usable in the guild and in a DM with the bot; never
                        // in group DMs or DMs with other users.
                        .contexts([InteractionContextType::Guild, InteractionContextType::BotDm]);
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
                .set_global_commands(&built)
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

    fn respond_component<'a>(
        &'a self,
        handle: &'a InteractionHandle,
        reply: InteractionReply,
    ) -> BoxFuture<'a, Result<(), ApiError>> {
        Box::pin(async move {
            let app = self.app()?;
            let interaction: Id<InteractionMarker> = id(handle.id, "interaction")?;
            let response = match reply {
                InteractionReply::Ack => InteractionResponse {
                    kind: InteractionResponseType::DeferredUpdateMessage,
                    data: None,
                },
                InteractionReply::Ephemeral(text) => InteractionResponse {
                    kind: InteractionResponseType::ChannelMessageWithSource,
                    data: Some(
                        InteractionResponseDataBuilder::new()
                            .content(text)
                            .flags(MessageFlags::EPHEMERAL)
                            .build(),
                    ),
                },
                InteractionReply::UpdateMessage(msg) => InteractionResponse {
                    kind: InteractionResponseType::UpdateMessage,
                    data: Some(
                        InteractionResponseDataBuilder::new()
                            .content(msg.content)
                            .components(build::rows(&msg.components))
                            .build(),
                    ),
                },
                InteractionReply::Modal(spec) => InteractionResponse {
                    kind: InteractionResponseType::Modal,
                    data: Some(
                        InteractionResponseDataBuilder::new()
                            .custom_id(spec.custom_id)
                            .title(spec.title)
                            .components([build::modal_field(&spec.field)])
                            .build(),
                    ),
                },
            };
            self.client
                .interaction(app)
                .create_response(interaction, &handle.token, &response)
                .await
                .map(|_| ())
                .map_err(|e| ApiError::from_http(&e))
        })
    }

    fn webhook_send<'a>(
        &'a self,
        channel_id: u64,
        username: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<u64, ApiError>> {
        Box::pin(async move {
            let (hook_id, token) = self.webhook_for(channel_id).await?;
            let hook: Id<twilight_model::id::marker::WebhookMarker> = id(hook_id, "webhook")?;
            let none = twilight_model::channel::message::AllowedMentions::default();
            let sent = self
                .client
                .execute_webhook(hook, &token)
                .allowed_mentions(Some(&none))
                .username(username)
                .content(content)
                .wait()
                .await;
            match sent {
                Ok(response) => {
                    let message = response
                        .model()
                        .await
                        .map_err(|e| ApiError::Permanent(format!("decoding the message: {e}")))?;
                    Ok(message.id.get())
                }
                Err(e) => {
                    // A webhook someone deleted is forgotten so the next line
                    // makes a new one.
                    let error = ApiError::from_http(&e);
                    if matches!(error, ApiError::Permanent(_)) {
                        self.webhooks
                            .lock()
                            .expect("webhook cache poisoned")
                            .remove(&channel_id);
                    }
                    Err(error)
                }
            }
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
            // Player text never pings anyone: no mentions are parsed.
            let none = twilight_model::channel::message::AllowedMentions::default();
            let components = build::rows(&msg.components);
            let message = self
                .client
                .create_message(channel)
                .allowed_mentions(Some(&none))
                .components(&components)
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
            // An empty row list clears the components (an answered request's
            // buttons go away).
            let components = build::rows(&msg.components);
            self.client
                .update_message(channel, message)
                .content(Some(&msg.content))
                .components(Some(&components))
                .await
                .map(|_| ())
                .map_err(|e| ApiError::from_http(&e))
        })
    }
}

/// The twilight shapes of the crate's component specs, kept in one place so
/// the library's field lists (and one deprecated field) touch nothing else.
mod build {
    use twilight_model::channel::message::component::{
        ActionRow, Button, ButtonStyle, Component, Label, SelectMenu, SelectMenuOption,
        SelectMenuType, TextInput, TextInputStyle,
    };

    use super::{
        MAX_BUTTON_LABEL_CHARS, MAX_BUTTONS_PER_ROW, MAX_CUSTOM_ID_BYTES, MAX_ROWS,
        MAX_SELECT_OPTIONS, Row, TextField,
    };

    fn clamp_chars(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            s.to_string()
        } else {
            s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
        }
    }

    fn clamp_id(s: &str) -> String {
        // A custom_id over the cap is a producer bug; a truncated id would
        // route wrong, so it is cut *and* the caller's validation should have
        // caught it (the DM writer checks before building).
        let mut out = s.to_string();
        while out.len() > MAX_CUSTOM_ID_BYTES {
            out.pop();
        }
        out
    }

    /// Action rows for a message (bounded to Discord's caps).
    pub(super) fn rows(rows: &[Row]) -> Vec<Component> {
        rows.iter()
            .take(MAX_ROWS)
            .map(|row| {
                Component::ActionRow(ActionRow {
                    id: None,
                    components: match row {
                        Row::Buttons(buttons) => buttons
                            .iter()
                            .take(MAX_BUTTONS_PER_ROW)
                            .map(|b| {
                                Component::Button(Button {
                                    id: None,
                                    custom_id: Some(clamp_id(&b.custom_id)),
                                    disabled: false,
                                    emoji: None,
                                    label: Some(clamp_chars(&b.label, MAX_BUTTON_LABEL_CHARS)),
                                    style: match b.style {
                                        super::ButtonStyle::Primary => ButtonStyle::Primary,
                                        super::ButtonStyle::Secondary => ButtonStyle::Secondary,
                                        super::ButtonStyle::Success => ButtonStyle::Success,
                                        super::ButtonStyle::Danger => ButtonStyle::Danger,
                                    },
                                    url: None,
                                    sku_id: None,
                                })
                            })
                            .collect(),
                        Row::Select(s) => vec![Component::SelectMenu(SelectMenu {
                            id: None,
                            channel_types: None,
                            custom_id: clamp_id(&s.custom_id),
                            default_values: None,
                            disabled: false,
                            kind: SelectMenuType::Text,
                            max_values: Some(1),
                            min_values: Some(1),
                            options: Some(
                                s.options
                                    .iter()
                                    .take(MAX_SELECT_OPTIONS)
                                    .map(|(value, label)| SelectMenuOption {
                                        default: false,
                                        description: None,
                                        emoji: None,
                                        label: clamp_chars(label, 100),
                                        value: clamp_id(value),
                                    })
                                    .collect(),
                            ),
                            placeholder: Some(clamp_chars(&s.placeholder, 150)),
                            required: None,
                        })],
                    },
                })
            })
            .collect()
    }

    /// A modal's one field: a `Label` wrapping a `TextInput` (components V2;
    /// the input's own `label` is the deprecated pre-V2 form).
    #[allow(deprecated)]
    pub(super) fn modal_field(field: &TextField) -> Component {
        Component::Label(Label {
            id: None,
            label: clamp_chars(&field.label, 45),
            description: None,
            component: Box::new(Component::TextInput(TextInput {
                id: None,
                custom_id: clamp_id(&field.custom_id),
                label: None,
                max_length: Some(field.max_len.clamp(1, 4000)),
                min_length: None,
                placeholder: None,
                required: Some(true),
                style: if field.paragraph {
                    TextInputStyle::Paragraph
                } else {
                    TextInputStyle::Short
                },
                value: field
                    .value
                    .as_ref()
                    .map(|v| clamp_chars(v, usize::from(field.max_len.clamp(1, 4000)))),
            })),
        })
    }
}

/// Flatten a modal submission's text fields (through `Label`s and action
/// rows) to `(custom_id, value)`.
fn modal_fields(components: &[ModalInteractionComponent], out: &mut Vec<(String, String)>) {
    for c in components {
        match c {
            ModalInteractionComponent::Label(l) => {
                modal_fields(std::slice::from_ref(&*l.component), out);
            }
            ModalInteractionComponent::ActionRow(r) => modal_fields(&r.components, out),
            ModalInteractionComponent::TextInput(t) => {
                out.push((t.custom_id.clone(), t.value.clone()));
            }
            _ => {}
        }
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
            let user_id = interaction.author_id()?.get();
            let handle = InteractionHandle {
                id: interaction.id.get(),
                token: interaction.token.clone(),
            };
            let message_id = interaction.message.as_ref().map(|m| m.id.get());
            match interaction.kind {
                InteractionType::MessageComponent => {
                    let Some(InteractionData::MessageComponent(data)) = interaction.data else {
                        return None;
                    };
                    let channel_id = interaction
                        .message
                        .as_ref()
                        .map(|m| m.channel_id.get())
                        .or_else(|| interaction.channel.as_ref().map(|c| c.id.get()));
                    return Some(DiscordEvent::Component {
                        user_id,
                        custom_id: data.custom_id,
                        values: data.values,
                        channel_id,
                        message_id,
                        handle,
                    });
                }
                InteractionType::ModalSubmit => {
                    let Some(InteractionData::ModalSubmit(data)) = interaction.data else {
                        return None;
                    };
                    let mut fields = Vec::new();
                    modal_fields(&data.components, &mut fields);
                    return Some(DiscordEvent::Modal {
                        user_id,
                        custom_id: data.custom_id,
                        fields,
                        message_id,
                        handle,
                    });
                }
                InteractionType::ApplicationCommand => {}
                _ => return None,
            }
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
                handle,
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
        ApiError, DiscordApi, DiscordEvent, EventSource, InteractionHandle, InteractionReply,
        Outgoing, SlashCommand,
    };

    /// One recorded REST call.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Call {
        /// `register_commands`.
        RegisterCommands {
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
        /// `respond_component`.
        Respond {
            /// The interaction id.
            interaction_id: u64,
            /// The reply.
            reply: InteractionReply,
        },
        /// `webhook_send`.
        WebhookSend {
            /// The channel.
            channel_id: u64,
            /// The impersonated name.
            username: String,
            /// The text.
            content: String,
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
        fn register_commands<'a>(
            &'a self,
            commands: &'a [SlashCommand],
        ) -> BoxFuture<'a, Result<(), ApiError>> {
            Box::pin(async move {
                self.record(Call::RegisterCommands {
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

        fn respond_component<'a>(
            &'a self,
            handle: &'a InteractionHandle,
            reply: InteractionReply,
        ) -> BoxFuture<'a, Result<(), ApiError>> {
            Box::pin(async move {
                self.record(Call::Respond {
                    interaction_id: handle.id,
                    reply,
                })
            })
        }

        fn webhook_send<'a>(
            &'a self,
            channel_id: u64,
            username: &'a str,
            content: &'a str,
        ) -> BoxFuture<'a, Result<u64, ApiError>> {
            Box::pin(async move {
                self.record(Call::WebhookSend {
                    channel_id,
                    username: username.to_string(),
                    content: content.to_string(),
                })?;
                Ok(self.alloc())
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

    /// A component click (or select pick) for tests.
    #[must_use]
    pub fn component(
        user_id: u64,
        custom_id: &str,
        values: &[&str],
        message_id: u64,
    ) -> DiscordEvent {
        DiscordEvent::Component {
            user_id,
            custom_id: custom_id.to_string(),
            values: values.iter().map(|v| (*v).to_string()).collect(),
            channel_id: Some(900),
            message_id: Some(message_id),
            handle: InteractionHandle {
                id: 78,
                token: "tok".to_string(),
            },
        }
    }

    /// A modal submission for tests.
    #[must_use]
    pub fn modal(
        user_id: u64,
        custom_id: &str,
        fields: &[(&str, &str)],
        message_id: Option<u64>,
    ) -> DiscordEvent {
        DiscordEvent::Modal {
            user_id,
            custom_id: custom_id.to_string(),
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            message_id,
            handle: InteractionHandle {
                id: 79,
                token: "tok".to_string(),
            },
        }
    }
}
