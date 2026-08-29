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
//! | `[log].level` | `""` (→ `RUST_LOG` or `"info"`) | tracing env-filter directive |
//! | `[log].format` | `"pretty"` | log format: `"pretty"` or `"json"` (prod shipping) |
//! | `[log].file` | `false` | also write daily-rotating logs to `<data_dir>/logs/` |
//! | `[web].metrics_enabled` | `false` | serve `GET /metrics` (Prometheus text; opt-in) |
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

/// `[onboarding]` section — the `chargen_required` gate and its start rooms.
///
/// Off by default (`chargen_required = false`): a fresh character plays
/// immediately, exactly as before. When on, a newly created character with an
/// unapproved sheet is placed in `start_room_unapproved`, held to the chargen
/// command set (chargen/look/say/help; channel speech and movement are gated),
/// and dropped into the guided wizard on connect; staff approving its
/// `wod-chargen` job lifts the gate and moves it to `start_room`. Both rooms
/// default to `#0` (Limbo, the seeded start
/// room), so turning the flag on with no rooms configured still works — Limbo
/// doubles as the chargen area, and the restriction is enforced by the gate.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct OnboardingCfg {
    /// Gate unapproved characters behind chargen + staff approval.
    pub chargen_required: bool,
    /// Object id (`#N`) of the live start room approved characters land in.
    pub start_room: u32,
    /// Object id (`#N`) of the restricted chargen area unapproved characters
    /// are placed in (defaults to `start_room` when left at the sentinel).
    pub start_room_unapproved: u32,
}

impl Default for OnboardingCfg {
    fn default() -> Self {
        // 0 = Limbo (the seeded start room). u32::MAX is the "unset" sentinel
        // for start_room_unapproved so it can fall back to start_room.
        OnboardingCfg {
            chargen_required: false,
            start_room: mush_core::LIMBO.0,
            start_room_unapproved: u32::MAX,
        }
    }
}

impl OnboardingCfg {
    /// The ruleset-facing onboarding config: resolve the room ids and fold the
    /// `start_room_unapproved` sentinel back to `start_room`.
    #[must_use]
    pub fn to_ruleset(&self) -> ruleset_wod::OnboardingConfig {
        let start_room = mush_core::ObjId(self.start_room);
        let start_room_unapproved = if self.start_room_unapproved == u32::MAX {
            start_room
        } else {
            mush_core::ObjId(self.start_room_unapproved)
        };
        ruleset_wod::OnboardingConfig {
            chargen_required: self.chargen_required,
            start_room,
            start_room_unapproved,
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

/// Log output format.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable console output (dev default).
    Pretty,
    /// One JSON object per line — for prod log shipping/aggregation.
    Json,
}

/// `[log]` section — structured logging + optional file rotation
/// (PLAN "Observability"). Every field defaults, so the block is optional.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct LogCfg {
    /// `tracing` env-filter directive (e.g. `"info"`,
    /// `"info,mush_llm=debug"`). Empty (default) means: honor `RUST_LOG` if
    /// set, else `"info"`.
    pub level: String,
    /// Output format: `"pretty"` (default) or `"json"`.
    pub format: String,
    /// When `true`, also write daily-rotating log files to
    /// `<data_dir>/logs/` (in addition to stdout). Default `false` (stdout
    /// only). The appender is non-blocking; its worker guard lives for the
    /// process lifetime.
    pub file: bool,
}

impl Default for LogCfg {
    fn default() -> Self {
        LogCfg {
            level: String::new(),
            format: "pretty".to_string(),
            file: false,
        }
    }
}

impl LogCfg {
    /// The validated output format. An unknown value falls back to `Pretty`
    /// with a warning rather than refusing to boot (logging config should
    /// never be the thing that stops the server).
    #[must_use]
    pub fn format(&self) -> LogFormat {
        match self.format.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            "pretty" | "" => LogFormat::Pretty,
            other => {
                // No tracing yet at parse time in the normal path; the init
                // path re-checks and warns through the installed subscriber.
                eprintln!("mushd: unknown [log].format {other:?}; using \"pretty\"");
                LogFormat::Pretty
            }
        }
    }

    /// The effective env-filter directive string: `level` when set, else
    /// `RUST_LOG`, else `"info"`.
    #[must_use]
    pub fn directive(&self) -> String {
        if !self.level.trim().is_empty() {
            return self.level.clone();
        }
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())
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
    /// `[log]` — structured logging level/format + optional file rotation.
    pub log: LogCfg,
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
    /// `[onboarding]` — the `chargen_required` gate + start rooms (M9). Off by
    /// default; consumed by the WoD ruleset via [`OnboardingCfg::to_ruleset`].
    pub onboarding: OnboardingCfg,
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
    fn log_defaults_and_parse() {
        // No [log] table: pretty, stdout only, level falls back to info
        // (assuming RUST_LOG is unset in the test env).
        let cfg = Config::default();
        assert_eq!(cfg.log.format(), LogFormat::Pretty);
        assert!(!cfg.log.file);
        // directive() honors an explicit level over everything.
        let cfg: Config =
            toml::from_str("[log]\nlevel = \"debug\"\nformat = \"json\"\nfile = true").unwrap();
        assert_eq!(cfg.log.directive(), "debug");
        assert_eq!(cfg.log.format(), LogFormat::Json);
        assert!(cfg.log.file);
        // Unknown format degrades to pretty (never a boot failure).
        let cfg: Config = toml::from_str("[log]\nformat = \"yaml\"").unwrap();
        assert_eq!(cfg.log.format(), LogFormat::Pretty);
    }

    #[test]
    fn web_metrics_enabled_defaults_off_and_parses() {
        // Opt-in (M9): the endpoint binds the possibly-public web listener with
        // no auth, so a redeploy must not begin serving it without an explicit
        // opt-in.
        let cfg = Config::default();
        assert!(!cfg.web.metrics_enabled);
        let cfg: Config = toml::from_str("[web]\nmetrics_enabled = true").unwrap();
        assert!(cfg.web.metrics_enabled);
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
    fn onboarding_defaults_off_and_resolves_rooms() {
        // Default: off, both rooms fold to Limbo (#0).
        let cfg = Config::default();
        assert!(!cfg.onboarding.chargen_required);
        let r = cfg.onboarding.to_ruleset();
        assert!(!r.chargen_required);
        assert_eq!(r.start_room, mush_core::LIMBO);
        assert_eq!(r.start_room_unapproved, mush_core::LIMBO);

        // Explicit: gate on, a distinct start room, unapproved falls back to it
        // when unset.
        let cfg: Config =
            toml::from_str("[onboarding]\nchargen_required = true\nstart_room = 5").unwrap();
        let r = cfg.onboarding.to_ruleset();
        assert!(r.chargen_required);
        assert_eq!(r.start_room, mush_core::ObjId(5));
        assert_eq!(r.start_room_unapproved, mush_core::ObjId(5));

        // Both rooms explicit.
        let cfg: Config = toml::from_str(
            "[onboarding]\nchargen_required = true\nstart_room = 5\nstart_room_unapproved = 9",
        )
        .unwrap();
        let r = cfg.onboarding.to_ruleset();
        assert_eq!(r.start_room, mush_core::ObjId(5));
        assert_eq!(r.start_room_unapproved, mush_core::ObjId(9));
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
