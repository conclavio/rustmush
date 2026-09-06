//! [`DiscordConfig`] — everything the Discord transport needs from
//! `mushd.toml`'s `[discord]` table.

use serde::{Deserialize, Serialize};

/// One relayed comsys channel ↔ Discord channel mapping (`[[discord.relay]]`,
/// D4). Parsed now so a configured deployment does not break when the relay
/// lands; unused until then.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayMap {
    /// The comsys channel name (`Public`).
    pub channel: String,
    /// The Discord channel id it mirrors to.
    pub discord_channel_id: u64,
    /// `both` | `to_discord` | `to_game`.
    pub direction: String,
    /// Post game lines through a webhook under the character's name.
    pub webhook: bool,
}

impl Default for RelayMap {
    fn default() -> Self {
        RelayMap {
            channel: String::new(),
            discord_channel_id: 0,
            direction: "both".to_string(),
            webhook: true,
        }
    }
}

/// Configuration for the Discord bot. Every field has a serde default, so a
/// `[discord]` TOML table only needs the values it wants to override — and an
/// absent table means `enabled = false`.
///
/// The bot **token is never in this struct**: `token_env` names the
/// environment variable that holds it ([`DiscordConfig::resolve_token`]),
/// the same rule as LLM API keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordConfig {
    /// Start the bot at all. Off by default.
    pub enabled: bool,
    /// The environment variable holding the bot token.
    pub token_env: String,
    /// The Discord application id (the bot's).
    pub application_id: u64,
    /// The one guild the bot serves (slash commands are registered per guild
    /// so they appear immediately).
    pub guild_id: u64,
    /// How many failed `/link` attempts one Discord user may make per ten
    /// minutes before further attempts are refused without a lookup.
    pub link_attempts_per_10m: u32,
    /// DM coalescing window, milliseconds (D2).
    pub dm_coalesce_ms: u64,
    /// Longest DM message the writer emits, characters (D2; Discord's cap is
    /// 2 000).
    pub dm_max_chars: usize,
    /// Lines the DM coalescer buffers before dropping the oldest narrative (D2).
    pub dm_queue_lines: usize,
    /// Detach a follow after this long with no Discord input and no scene
    /// membership, seconds (D2).
    pub follow_idle_detach_s: u64,
    /// The `auto_follow` value a new link starts with.
    pub auto_follow_default: bool,
    /// Relayed channels (D4).
    pub relay: Vec<RelayMap>,
    /// The channel staff requests and alerts mirror to (D5); `0` disables.
    pub staff_channel_id: u64,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        DiscordConfig {
            enabled: false,
            token_env: "DISCORD_BOT_TOKEN".to_string(),
            application_id: 0,
            guild_id: 0,
            link_attempts_per_10m: 5,
            dm_coalesce_ms: 1_500,
            dm_max_chars: 1_900,
            dm_queue_lines: 2_000,
            follow_idle_detach_s: 21_600,
            auto_follow_default: false,
            relay: Vec::new(),
            staff_channel_id: 0,
        }
    }
}

impl DiscordConfig {
    /// The bot token from the environment variable `token_env` names —
    /// `None` (never an error, never a log line with the value) when the
    /// variable is unset or empty.
    #[must_use]
    pub fn resolve_token(&self) -> Option<String> {
        match std::env::var(&self.token_env) {
            Ok(v) if !v.trim().is_empty() => Some(v),
            _ => None,
        }
    }

    /// Refuse a configuration that says `enabled` but cannot run: a zero
    /// application or guild id. (A missing token is reported separately, at
    /// boot, because it is an environment fact rather than a file fact.)
    ///
    /// # Errors
    ///
    /// A message naming the first missing value.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.application_id == 0 {
            return Err("[discord].application_id is required when enabled".to_string());
        }
        if self.guild_id == 0 {
            return Err("[discord].guild_id is required when enabled".to_string());
        }
        if self.token_env.trim().is_empty() {
            return Err("[discord].token_env must name an environment variable".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_table_is_disabled_and_validates() {
        let cfg: DiscordConfig = toml::from_str("").unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.token_env, "DISCORD_BOT_TOKEN");
        assert_eq!(cfg.link_attempts_per_10m, 5);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn enabled_requires_the_ids() {
        let cfg: DiscordConfig = toml::from_str("enabled = true").unwrap();
        assert!(cfg.validate().unwrap_err().contains("application_id"));
        let cfg: DiscordConfig = toml::from_str("enabled = true\napplication_id = 5").unwrap();
        assert!(cfg.validate().unwrap_err().contains("guild_id"));
        let cfg: DiscordConfig =
            toml::from_str("enabled = true\napplication_id = 5\nguild_id = 7").unwrap();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn relay_entries_parse_with_defaults() {
        let cfg: DiscordConfig =
            toml::from_str("[[relay]]\nchannel = \"Public\"\ndiscord_channel_id = 42\n").unwrap();
        assert_eq!(cfg.relay.len(), 1);
        assert_eq!(cfg.relay[0].direction, "both");
        assert!(cfg.relay[0].webhook);
    }

    #[test]
    fn debug_carries_no_token() {
        let cfg = DiscordConfig::default();
        let debug = format!("{cfg:?}");
        assert!(debug.contains("token_env"));
        assert!(!debug.contains("token:"), "the config never holds a token");
    }
}
