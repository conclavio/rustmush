//! # mush-net-discord — the Discord transport (D1, D2)
//!
//! The fourth interface's transport half (`docs/interface/04-discord-implementation-plan.md`
//! §3.1–3.2). Like `mush-net-ssh` and `mush-net-web` it owns the wire — here the
//! Discord Gateway websocket and REST, through [`twilight`] — and calls
//! **hooks** the gateway's bridge implements for everything game-side.
//!
//! Three seams, each with a production and an in-memory implementation:
//!
//! - [`DiscordApi`] — the REST calls the bot makes. [`DiscordRest`] is
//!   REST-only and built from the token alone, so either process may hold one
//!   (the on-call outbox adapter in the server half, 03 §7a.2);
//!   [`testing::FakeApi`] records calls and scripts failures.
//! - [`EventSource`] — Gateway events reduced to [`DiscordEvent`].
//!   [`TwilightEvents`] is one reconnecting shard; [`testing::FakeEvents`] is
//!   a channel.
//! - [`DiscordHooks`] — what the bot asks the game side: link, unlink, whoami
//!   (D1); follow, detach, a DM line (D2).
//!
//! [`serve`] is the bot task; [`dm::run_dm_writer`] is one follow's lossy
//! render (the allow-list in [`dm::render`], coalescing, the drop marker). No
//! test in this workspace ever reaches Discord.
//!
//! [`twilight`]: https://twilight.rs

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod api;
pub mod bot;
pub mod config;
pub mod dm;
pub mod hooks;

pub use api::{
    ApiError, Button, ButtonStyle, DiscordApi, DiscordEvent, DiscordRest, EventSource,
    InteractionHandle, InteractionReply, MAX_BUTTON_LABEL_CHARS, MAX_BUTTONS_PER_ROW,
    MAX_CUSTOM_ID_BYTES, MAX_ROWS, MAX_SELECT_OPTIONS, ModalSpec, Outgoing, Row, Select,
    SlashCommand, SlashOption, TextField, TwilightEvents, WEBHOOK_NAME, testing,
};
pub use bot::{LINK_REFUSED, NOT_FOLLOWING, RELAY_NOT_LINKED, ServeError, commands, serve};
pub use config::{DiscordConfig, RelayMap};
pub use dm::{
    DmPolicy, FollowState, Interactive, MAX_NOTICE_ACTIONS, Rendered, RequestMessage, render,
    run_dm_writer,
};
pub use hooks::{
    DetachOutcome, DiscordHooks, FollowOutcome, Interaction, LineOutcome, LinkOutcome,
    RelayLineOutcome, UnlinkOutcome, WhoAmI,
};
pub use twilight_gateway::Intents;
