//! `mushd.toml` configuration.
//!
//! Reads only the keys each milestone needs; any unknown section/key is
//! tolerated and ignored. Keys used now:
//!
//! | key | default | meaning |
//! |---|---|---|
//! | `[server].name` | `"rustmush"` | server name (banners, logs) |
//! | `[server].data_dir` | `"./data"` | host keys, one-time `accounts.json` import, logs |
//! | `[server].help_dir` | `"./docs/help"` | markdown help topics loaded at boot (`help <topic>`) |
//! | `[store].store` | `"redis"` | store backend: `"redis"` or `"memory"` |
//! | `[store].redis_url` | `"redis://127.0.0.1:6379"` | Redis connection URL |
//! | `[ssh].listen` | `"0.0.0.0:2222"` | SSH listen address |
//! | `[web].listen` | `"127.0.0.1:8080"` | web (HTTP/WS) listen address |
//! | `[web].trusted_proxies` | `[]` | proxies whose `X-Forwarded-For` is honored |
//! | `[game].linkdead_grace_s` | `300` | linkdead grace before disconnect |
//! | `[game].max_characters` | `3` | characters per account cap |
//! | `[game].ic_ratio` | `1` | IC seconds per OOC second (game-clock speed); read at boot |
//! | `[game].ic_epoch` | `"2020-01-01 00:00"` | IC datetime at OOC unix 0 — calendar phase/start; read at boot |
//! | `[rulesets].enabled` | `["wod"]` | ruleset module(s) to load; unknown ids are a hard boot error |
//! | `[llm]` | `enabled = false` | M7 LLM layer (providers, routes, budgets); into [`mush_llm::LlmConfig`] |
//!
//! The `[web]` table deserializes straight into
//! [`mush_net_web::WebConfig`] (every knob it documents — TLS material,
//! rate limits, WS tuning — is available here). Its `features`/`panels`
//! fields are **overwritten at boot** from the module registry, so config
//! files should leave them unset.
//!
//! A missing file yields the defaults; a file that exists but does not parse
//! is a hard error (silently ignoring a typo'd config is worse than not
//! starting).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use mush_core::{CivilDate, GameClock};
use mush_llm::LlmConfig;
use mush_net_web::WebConfig;
use serde::Deserialize;

/// Errors loading `mushd.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The config path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The file exists but is not valid TOML (or has wrongly typed keys).
    #[error("cannot parse {path}: {source}")]
    Parse {
        /// The config path.
        path: PathBuf,
        /// Underlying error (boxed: `toml::de::Error` is large).
        source: Box<toml::de::Error>,
    },
    /// `[ssh].listen` is not a valid socket address.
    #[error("invalid [ssh].listen address {addr:?}: {source}")]
    BadListen {
        /// The offending value.
        addr: String,
        /// Underlying error.
        source: std::net::AddrParseError,
    },
    /// `[store].store` is not a known backend.
    #[error("invalid [store].store {value:?}: expected \"redis\" or \"memory\"")]
    BadStoreBackend {
        /// The offending value.
        value: String,
    },
    /// `[game].ic_epoch` is not a parseable IC datetime.
    #[error(
        "invalid [game].ic_epoch {value:?}: expected \"YYYY-MM-DD\" or \
         \"YYYY-MM-DD HH:MM[:SS]\""
    )]
    BadIcEpoch {
        /// The offending value.
        value: String,
    },
}

/// Which [`mush_store::ObjectStore`] implementation to boot with.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StoreBackend {
    /// The authoritative Redis 8 store (`[store].redis_url`).
    Redis,
    /// The in-memory store — tests and offline runs only; nothing survives
    /// a restart.
    Memory,
}

/// `[server]` section.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ServerCfg {
    /// Server name, shown in banners and logs.
    pub name: String,
    /// Directory for host keys, `accounts.json`, and logs.
    pub data_dir: PathBuf,
    /// Directory of markdown help topics loaded at boot (`help <topic>`).
    /// A missing directory just means zero topics.
    pub help_dir: PathBuf,
}

impl Default for ServerCfg {
    fn default() -> Self {
        ServerCfg {
            name: "rustmush".to_string(),
            data_dir: PathBuf::from("./data"),
            help_dir: PathBuf::from("./docs/help"),
        }
    }
}

/// `[store]` section.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StoreCfg {
    /// Store backend: `"redis"` (default) or `"memory"`. `--repl` never
    /// reaches the store at all.
    pub store: String,
    /// Redis connection URL (`redis://[user:pass@]host:port[/db]`).
    pub redis_url: String,
}

impl Default for StoreCfg {
    fn default() -> Self {
        StoreCfg {
            store: "redis".to_string(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
        }
    }
}

/// `[ssh]` section.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SshCfg {
    /// Listen address, e.g. `"0.0.0.0:2222"`.
    pub listen: String,
}

impl Default for SshCfg {
    fn default() -> Self {
        SshCfg {
            listen: "0.0.0.0:2222".to_string(),
        }
    }
}

/// `[rulesets]` section — which compiled-in ruleset module(s) load at boot.
///
/// Every ruleset crate is compiled into `mushd`; this list *selects* which
/// ones register (design 00 "Rulesets": selected by config). The only
/// ruleset available in M6a is `"wod"` (oWoD20); listing an unknown name is
/// a hard boot error, resolved by [`crate::modules::ruleset_modules`].
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RulesetsCfg {
    /// Ruleset ids to load, in order. Registered **after** the built-in
    /// modules so their commands/panels/job-queues wire cleanly and their
    /// approval payload handlers see a loaded jobs module.
    pub enabled: Vec<String>,
}

impl Default for RulesetsCfg {
    fn default() -> Self {
        // WoD is the flagship ruleset and the point of M6 — a config-less
        // server is playable out of the box.
        RulesetsCfg {
            enabled: vec!["wod".to_string()],
        }
    }
}

/// `[game]` section.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct GameCfg {
    /// Seconds a session stays linkdead after an unclean drop before the
    /// character is fully disconnected.
    pub linkdead_grace_s: u64,
    /// Maximum characters per account (`newchar` cap).
    pub max_characters: u32,
    /// IC seconds per OOC second — the game-clock speed. `4` makes a full IC
    /// day pass in six real hours; `0` is treated as `1`. **Read once at
    /// boot** (hot-reload lands later); changing it needs a restart.
    pub ic_ratio: u32,
    /// The IC civil datetime the clock reads at OOC unix time 0 — the
    /// calendar's phase and starting year. `"YYYY-MM-DD"` or
    /// `"YYYY-MM-DD HH:MM[:SS]"`. IC time is a pure function of real wall
    /// time (so it survives restarts); it is **not** reset to this value at
    /// each boot. Also read once at boot.
    pub ic_epoch: String,
}

impl Default for GameCfg {
    fn default() -> Self {
        GameCfg {
            linkdead_grace_s: 300,
            max_characters: 3,
            ic_ratio: 1,
            ic_epoch: "2020-01-01 00:00".to_string(),
        }
    }
}

impl GameCfg {
    /// Build the [`GameClock`] from the `[game]` clock knobs. A malformed
    /// `ic_epoch` is a hard boot error (a silently-wrong calendar is worse
    /// than not starting).
    pub fn game_clock(&self) -> Result<GameClock, ConfigError> {
        let epoch = CivilDate::parse(&self.ic_epoch).ok_or_else(|| ConfigError::BadIcEpoch {
            value: self.ic_epoch.clone(),
        })?;
        Ok(GameClock {
            ic_ratio: self.ic_ratio,
            epoch_offset: epoch.to_ic_seconds(),
        })
    }
}

/// The whole configuration (unknown sections/keys ignored).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `[server]`.
    pub server: ServerCfg,
    /// `[store]`.
    pub store: StoreCfg,
    /// `[ssh]`.
    pub ssh: SshCfg,
    /// `[web]` — passed to `mush-net-web` (M4); `features`/`panels` are
    /// filled from the module registry at boot.
    pub web: WebConfig,
    /// `[game]`.
    pub game: GameCfg,
    /// `[rulesets]` — which ruleset module(s) load (M6).
    pub rulesets: RulesetsCfg,
    /// `[llm]` — the M7 LLM layer (providers, feature routes, budgets, kill
    /// switch). Deserializes straight into [`mush_llm::LlmConfig`]; the whole
    /// block defaults to `enabled = false` (a keyless clone still boots). API
    /// keys are named by env var (`api_key_env`), never inlined — see
    /// [`mush_llm::config`].
    pub llm: LlmConfig,
}

impl Config {
    /// Load from `path`. A missing file returns the defaults; a present but
    /// malformed file is an error.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file; using defaults");
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    /// The parsed `[ssh].listen` socket address.
    pub fn ssh_listen(&self) -> Result<SocketAddr, ConfigError> {
        self.ssh
            .listen
            .parse()
            .map_err(|source| ConfigError::BadListen {
                addr: self.ssh.listen.clone(),
                source,
            })
    }

    /// The validated `[store].store` backend selection.
    pub fn store_backend(&self) -> Result<StoreBackend, ConfigError> {
        match self.store.store.to_lowercase().as_str() {
            "redis" => Ok(StoreBackend::Redis),
            "memory" => Ok(StoreBackend::Memory),
            other => Err(ConfigError::BadStoreBackend {
                value: other.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let cfg = Config::load(Path::new("Z:/definitely/not/here/mushd.toml")).unwrap();
        assert_eq!(cfg.server.name, "rustmush");
        assert_eq!(cfg.server.data_dir, PathBuf::from("./data"));
        assert_eq!(cfg.server.help_dir, PathBuf::from("./docs/help"));
        assert_eq!(cfg.ssh.listen, "0.0.0.0:2222");
        assert_eq!(cfg.game.linkdead_grace_s, 300);
        assert_eq!(cfg.game.max_characters, 3);
        assert_eq!(cfg.store.store, "redis");
        assert_eq!(cfg.store.redis_url, "redis://127.0.0.1:6379");
        assert_eq!(cfg.store_backend().unwrap(), StoreBackend::Redis);
        assert_eq!(cfg.ssh_listen().unwrap().port(), 2222);
        assert_eq!(cfg.web.listen.to_string(), "127.0.0.1:8080");
        assert!(cfg.web.trusted_proxies.is_empty());
        // Game-clock defaults: 1:1 with OOC time, calendar phased to 2020.
        assert_eq!(cfg.game.ic_ratio, 1);
        assert_eq!(cfg.game.ic_epoch, "2020-01-01 00:00");
        let clock = cfg.game.game_clock().unwrap();
        assert_eq!(clock.ic_ratio, 1);
        assert_eq!(
            clock.epoch_offset,
            mush_core::CivilDate::parse("2020-01-01 00:00")
                .unwrap()
                .to_ic_seconds()
        );
    }

    #[test]
    fn rulesets_default_and_parse() {
        // A config-less server loads WoD out of the box.
        let cfg = Config::default();
        assert_eq!(cfg.rulesets.enabled, ["wod"]);
        // An explicit [rulesets] table overrides it.
        let cfg: Config = toml::from_str("[rulesets]\nenabled = [\"wod\"]").unwrap();
        assert_eq!(cfg.rulesets.enabled, ["wod"]);
        // An empty list disables all rulesets (a bare comsys/mail server).
        let cfg: Config = toml::from_str("[rulesets]\nenabled = []").unwrap();
        assert!(cfg.rulesets.enabled.is_empty());
    }

    #[test]
    fn game_clock_parses_ratio_and_epoch() {
        let cfg: Config =
            toml::from_str("[game]\nic_ratio = 4\nic_epoch = \"2150-06-15 08:30\"").unwrap();
        let clock = cfg.game.game_clock().unwrap();
        assert_eq!(clock.ic_ratio, 4);
        // At OOC time 0 the clock reads exactly the configured epoch.
        assert_eq!(clock.ic_datetime(0), "June 15, 2150, 08:30");
    }

    #[test]
    fn bad_ic_epoch_is_an_error() {
        let cfg: Config = toml::from_str("[game]\nic_epoch = \"not-a-date\"").unwrap();
        assert!(matches!(
            cfg.game.game_clock(),
            Err(ConfigError::BadIcEpoch { .. })
        ));
    }

    #[test]
    fn web_section_parses_into_webconfig() {
        let text = r#"
            [web]
            listen = "0.0.0.0:9090"
            trusted_proxies = ["127.0.0.1"]
        "#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.web.listen.to_string(), "0.0.0.0:9090");
        assert_eq!(
            cfg.web.trusted_proxies,
            ["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]
        );
        // Rate-limit / WS tuning knobs keep their crate defaults.
        assert_eq!(cfg.web.auth_burst, 5);
    }

    #[test]
    fn parses_partial_file_and_ignores_unknown_sections() {
        // The example config carries M2+ sections ([store], [web], [llm]);
        // they must not break the M1 parse, and absent keys keep defaults.
        let text = r#"
            [server]
            name = "testmush"

            [store]
            store = "memory"
            redis_url = "redis://10.0.0.1:6380"

            [game]
            linkdead_grace_s = 10
            guest_pool = 5

            [llm]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.server.name, "testmush");
        assert_eq!(cfg.server.data_dir, PathBuf::from("./data"));
        assert_eq!(cfg.ssh.listen, "0.0.0.0:2222");
        assert_eq!(cfg.game.linkdead_grace_s, 10);
        assert_eq!(cfg.store_backend().unwrap(), StoreBackend::Memory);
        assert_eq!(cfg.store.redis_url, "redis://10.0.0.1:6380");
    }

    #[test]
    fn bad_store_backend_reports() {
        let cfg: Config = toml::from_str("[store]\nstore = \"sqlite\"").unwrap();
        assert!(matches!(
            cfg.store_backend(),
            Err(ConfigError::BadStoreBackend { .. })
        ));
    }

    #[test]
    fn example_config_parses() {
        // Keep mushd.toml.example loadable by the real parser forever.
        let example = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mushd.toml.example");
        let cfg = Config::load(Path::new(example)).unwrap();
        assert_eq!(cfg.server.name, "rustmush dev");
        assert_eq!(cfg.ssh.listen, "0.0.0.0:2222");
        assert_eq!(cfg.game.linkdead_grace_s, 300);
        assert_eq!(cfg.store_backend().unwrap(), StoreBackend::Redis);
        assert_eq!(cfg.game.max_characters, 3);
        assert_eq!(cfg.game.ic_ratio, 1);
        assert_eq!(cfg.game.ic_epoch, "2020-01-01 00:00");
        // The example's clock knobs must build a valid clock.
        assert!(cfg.game.game_clock().is_ok());
        // The example enables the WoD ruleset.
        assert_eq!(cfg.rulesets.enabled, ["wod"]);
        // The example's [llm] block parses into LlmConfig: disabled by default
        // (keyless clone boots), both providers present with env-var-named
        // keys (never inlined), and the starter routes wired.
        assert!(!cfg.llm.enabled);
        assert_eq!(cfg.llm.providers.len(), 2);
        let openrouter = cfg
            .llm
            .providers
            .get("openrouter")
            .expect("openrouter provider");
        assert_eq!(
            openrouter.api_key_env.as_deref(),
            Some("OPENROUTER_API_KEY")
        );
        assert_eq!(openrouter.kind, mush_llm::ProviderKind::Openai);
        let anthropic = cfg
            .llm
            .providers
            .get("anthropic")
            .expect("anthropic provider");
        assert_eq!(anthropic.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(anthropic.kind, mush_llm::ProviderKind::Anthropic);
        let room = cfg
            .llm
            .routes
            .get("worldgen_room")
            .expect("worldgen_room route");
        assert_eq!(room.model, "z-ai/glm-5.3");
        assert!(matches!(
            room.reasoning_config(),
            Some(mush_llm::ReasoningConfig::Effort(mush_llm::Effort::Low))
        ));
        // The M8 NPC dialogue tiers each resolve to a route; the default
        // major route rides OpenRouter (anthropic/claude-opus-5 is reserved as
        // a documented per-NPC override, not the default).
        assert_eq!(
            cfg.llm
                .routes
                .get("npc_dialogue_major")
                .expect("npc_dialogue_major route")
                .model,
            "z-ai/glm-5.3"
        );
        assert_eq!(
            cfg.llm.routes["npc_dialogue_minor"].model,
            "z-ai/glm-5.3-flash"
        );
        assert_eq!(
            cfg.llm.routes["npc_dialogue_extra"].model,
            "deepseek/deepseek-v4-flash-0731"
        );
        assert!(cfg.llm.routes.contains_key("npc_summarize"));
        // Per-NPC daily budget documented in the example.
        assert!(cfg.llm.budgets.per_npc_default.is_some());
    }

    #[test]
    fn llm_section_parses_and_defaults_disabled() {
        // No [llm] table at all: LlmConfig defaults, disabled.
        let cfg = Config::default();
        assert!(!cfg.llm.enabled);
        assert!(cfg.llm.providers.is_empty());
        assert!(cfg.llm.routes.is_empty());
        // A partial [llm] table: enabled + one route, keys named by env var.
        let text = r#"
            [llm]
            enabled = true

            [llm.providers.openrouter]
            kind = "openai"
            base_url = "https://openrouter.ai/api/v1"
            api_key_env = "OPENROUTER_API_KEY"

            [llm.routes.worldgen_room]
            model = "z-ai/glm-5.3"
            max_tokens = 1500
            reasoning = "low"
        "#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert!(cfg.llm.enabled);
        assert_eq!(cfg.llm.routes["worldgen_room"].max_tokens, 1500);
        assert!(cfg.llm.providers["openrouter"].api_key_env.is_some());
    }

    #[test]
    fn malformed_file_is_an_error() {
        // Process-unique dir: concurrent test runs (other checkouts, CI
        // shards) must not race each other on a shared fixed path.
        let dir = std::env::temp_dir().join(format!("mushd-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(&path, "[server\nname=]").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::Parse { .. })
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_listen_reports() {
        let cfg: Config = toml::from_str("[ssh]\nlisten = \"not-an-addr\"").unwrap();
        assert!(matches!(
            cfg.ssh_listen(),
            Err(ConfigError::BadListen { .. })
        ));
    }
}
