//! Gateway-side control hooks the server drives across the boundary
//! (programme D3): `ToGateway::DiscordControl` asks the Discord bridge to
//! follow, unfollow or re-tune a character's follow. The bridge is optional
//! (`[discord].enabled`), starts after the boundary, and is replaced never —
//! so the delivery transports hold a [`GatewayControls`] registry and the
//! bridge registers itself into it at start-up.

use std::sync::{Arc, OnceLock};

use mush_ipc::DiscordControlOp;

/// What the Discord bridge exposes to the boundary: one fire-and-forget
/// control per character. The bridge answers the character itself.
pub trait DiscordControl: Send + Sync {
    /// Apply `op` for the character `char` (a widened `ObjId`).
    fn control(&self, char: u64, op: DiscordControlOp);
}

/// The registry the transports consult. Empty until a bridge registers.
#[derive(Default)]
pub struct GatewayControls {
    discord: OnceLock<Arc<dyn DiscordControl>>,
}

impl GatewayControls {
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
            .finish()
    }
}
