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

/// What the web bridge exposes to the boundary and periodic refresh loop
/// (EXT-P1-3): cross-generation bearer-token revocation (`ToGateway::
/// RevokeWebToken`) and store-backed live-token reconciliation (catching
/// revocations/expiries across a server restart gap).
pub trait WebTokenRevoker: Send + Sync {
    /// Record `token_hash` as revoked (blocking in-flight attach/resume) and
    /// return every live connection id authenticated by that token hash.
    fn revoke_token_hash(&self, token_hash: &str) -> Vec<u64>;

    /// Snapshot the unique token hashes currently authenticating live web
    /// connections on this gateway.
    fn live_token_hashes(&self) -> Vec<String>;
}

/// The registry the transports consult. Empty until a bridge registers.
#[derive(Default)]
pub struct GatewayControls {
    discord: OnceLock<Arc<dyn DiscordControl>>,
    relay: OnceLock<Arc<dyn RelaySink>>,
    web_tokens: OnceLock<Arc<dyn WebTokenRevoker>>,
}

impl GatewayControls {
    /// Register the web token revoker (the web bridge). A second registration
    /// is ignored and logged.
    pub fn set_web_tokens(&self, revoker: Arc<dyn WebTokenRevoker>) {
        if self.web_tokens.set(revoker).is_err() {
            tracing::warn!("a web token revoker was already registered; keeping the first");
        }
    }

    /// Record `token_hash` as revoked on the web bridge and return every live
    /// web connection id authenticated by it (empty when no web bridge runs).
    #[must_use]
    pub fn revoke_web_token(&self, token_hash: &str) -> Vec<u64> {
        match self.web_tokens.get() {
            Some(revoker) => revoker.revoke_token_hash(token_hash),
            None => Vec::new(),
        }
    }

    /// Snapshot the unique token hashes currently authenticating live web
    /// connections (empty when no web bridge runs).
    #[must_use]
    pub fn live_web_token_hashes(&self) -> Vec<String> {
        match self.web_tokens.get() {
            Some(revoker) => revoker.live_token_hashes(),
            None => Vec::new(),
        }
    }

    /// Whether a web token revoker is registered.
    #[must_use]
    pub fn has_web_tokens(&self) -> bool {
        self.web_tokens.get().is_some()
    }
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
            .field("web_tokens", &self.has_web_tokens())
            .finish()
    }
}
