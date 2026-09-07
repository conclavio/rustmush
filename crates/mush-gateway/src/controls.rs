//! Gateway-side control hooks the server drives across the boundary
//! (programme D3): `ToGateway::DiscordControl` asks the Discord bridge to
//! follow, unfollow or re-tune a character's follow. The bridge is optional
//! (`[discord].enabled`), starts after the boundary, and is replaced never —
//! so the delivery transports hold a [`GatewayControls`] registry and the
//! bridge registers itself into it at start-up.

use std::sync::{Arc, OnceLock};

use mush_ipc::DiscordControlOp;
use mush_proto::OutputEvent;

/// What the Discord bridge exposes to the boundary: one fire-and-forget
/// control per character. The bridge answers the character itself.
pub trait DiscordControl: Send + Sync {
    /// Apply `op` for the character `char` (a widened `ObjId`).
    fn control(&self, char: u64, op: DiscordControlOp);
}

/// What the Discord bridge exposes as a relay sink (programme D4): the world
/// publishes a relay-flagged channel's line to `SendTarget::Relay(name)` and
/// the bridge mirrors it to the mapped Discord channel.
pub trait RelaySink: Send + Sync {
    /// Mirror one channel line.
    fn relay(&self, channel: &str, event: Arc<OutputEvent>);
}

/// The registry the transports consult. Empty until a bridge registers.
#[derive(Default)]
pub struct GatewayControls {
    discord: OnceLock<Arc<dyn DiscordControl>>,
    relay: OnceLock<Arc<dyn RelaySink>>,
}

impl GatewayControls {
    /// Register the relay sink (the Discord bridge). A second registration is
    /// ignored and logged.
    pub fn set_relay(&self, sink: Arc<dyn RelaySink>) {
        if self.relay.set(sink).is_err() {
            tracing::warn!("a relay sink was already registered; keeping the first");
        }
    }

    /// Mirror one relayed channel line, or drop it (debug) when no sink runs.
    pub fn relay(&self, channel: &str, event: Arc<OutputEvent>) {
        match self.relay.get() {
            Some(sink) => sink.relay(channel, event),
            None => tracing::debug!(channel, "relay line with no sink; dropped"),
        }
    }

    /// Whether a relay sink is registered.
    #[must_use]
    pub fn has_relay(&self) -> bool {
        self.relay.get().is_some()
    }

    /// Register the Discord bridge. A second registration is ignored (one bot
    /// per process) and logged.
    pub fn set_discord(&self, control: Arc<dyn DiscordControl>) {
        if self.discord.set(control).is_err() {
            tracing::warn!("a Discord control hook was already registered; keeping the first");
        }
    }

    /// Route one control to the bridge, or drop it (logged) when no bridge
    /// runs — the character then simply hears nothing, and `@discord` status
    /// says the bot is not running.
    pub fn discord_control(&self, char: u64, op: DiscordControlOp) {
        match self.discord.get() {
            Some(bridge) => bridge.control(char, op),
            None => tracing::debug!(char, ?op, "discord control with no bridge; dropped"),
        }
    }

    /// Whether a Discord bridge is registered.
    #[must_use]
    pub fn has_discord(&self) -> bool {
        self.discord.get().is_some()
    }
}

impl std::fmt::Debug for GatewayControls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayControls")
            .field("discord", &self.has_discord())
            .field("relay", &self.has_relay())
            .finish()
    }
}
