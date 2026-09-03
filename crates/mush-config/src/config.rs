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
//! | `[weather].flavor_variants` | `10` | M10 `@weather/generate` variants requested per (climate,state) bucket |
//! | `[worldgen].batch_max_rooms` | `40` | M10 `@gen/building` interior-walk cap (rooms per batch) |
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
use serde::Deserialize;

/// An empty inline TOML table, the default for the raw `[llm]`/`[web]` sections
/// (deserialized into their typed forms by the crate that consumes them).
fn empty_toml_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

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
    /// `[ipc].listen` is not a valid boundary endpoint (M16b).
    #[error("invalid [ipc].listen {value:?}: {detail}")]
    BadIpcListen {
        /// The offending value.
        value: String,
        /// The parse detail.
        detail: String,
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
    /// The resolved live start room (`#N`).
    #[must_use]
    pub fn start_room_id(&self) -> mush_core::ObjId {
        mush_core::ObjId(self.start_room)
    }

    /// The resolved restricted chargen start room, folding the
    /// `start_room_unapproved` sentinel (`u32::MAX`) back to [`start_room`].
    ///
    /// [`start_room`]: OnboardingCfg::start_room
    #[must_use]
    pub fn start_room_unapproved_id(&self) -> mush_core::ObjId {
        if self.start_room_unapproved == u32::MAX {
            self.start_room_id()
        } else {
            mush_core::ObjId(self.start_room_unapproved)
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

/// `[weather]` section — the M10 weather LLM flavor top-up knobs.
///
/// The deterministic Markov machine (states/transitions/temperature) is
/// unaffected by these; only the *generated flavor text* pool sizing is
/// tuned here. `tick` / ambient probability keep their crate defaults (they
/// are not surfaced as config knobs yet).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WeatherCfg {
    /// How many flavor variants `@weather/generate` requests per
    /// `(climate, state)` bucket (transition + ambient). Defaults to the
    /// `mush-weather` crate default (10); the typed conversion into
    /// `mush_weather::WeatherConfig` lives in `mush-server`.
    pub flavor_variants: usize,
}

impl Default for WeatherCfg {
    fn default() -> Self {
        // Mirrors `mush_weather::WeatherConfig::default().flavor_variants`
        // (kept as a literal so `mush-config` needs no `mush-weather` dep).
        WeatherCfg {
            flavor_variants: 10,
        }
    }
}

/// `[worldgen]` section — the M10 batch `@gen/building` knobs.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorldgenCfg {
    /// The `@gen/building` interior-walk cap: the most rooms one batch will
    /// describe. Defaults to the `mush-worldgen` crate default (40); the typed
    /// conversion into `mush_worldgen::WorldgenConfig` lives in `mush-server`.
    pub batch_max_rooms: usize,
}

impl Default for WorldgenCfg {
    fn default() -> Self {
        // Mirrors `mush_worldgen::WorldgenConfig::default().batch_max_rooms`
        // (kept as a literal so `mush-config` needs no `mush-worldgen` dep).
        WorldgenCfg {
            batch_max_rooms: 40,
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

/// `[ipc]` section (M16b): where the forked gateway<->server boundary socket
/// lives. Read by `mush-gateway` (the supervisor connects here after spawning
/// the server child) and `mush-server` (it binds here). The all-in-one `mushd`
/// ignores it. A localhost TCP address works everywhere (the Windows dev path);
/// a filesystem path selects a Unix-domain socket where available (Linux prod).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct IpcCfg {
    /// The boundary socket address/path. Default `127.0.0.1:6791`.
    pub listen: String,
    /// Max frames queued gateway->server (`to_server`) before an INPUT frame
    /// (`Cmd`/`Query`) is shed with a logged warn + counter bump (M18 DoS
    /// hardening). Must-deliver frames are never shed. Default `1024`.
    pub to_server_cap: usize,
    /// Max frames queued server->gateway (`to_gw`) before an overflow tears
    /// the boundary down (every `to_gw` frame is must-deliver, so it is never
    /// per-message dropped; the supervisor re-attach heals the loss). Default
    /// `8192`.
    pub to_gw_cap: usize,
    /// Per-frame IPC write timeout, milliseconds. A connected-but-not-draining
    /// peer past this is treated as wedged and the boundary is torn down.
    /// Default `5000`.
    pub write_timeout_ms: u64,
    /// The high-water threshold (percent of a queue's cap) at which a crossing
    /// logs an edge-gated warn. Default `80`.
    pub high_water_warn_pct: u8,
}

impl Default for IpcCfg {
    fn default() -> Self {
        IpcCfg {
            listen: "127.0.0.1:6791".to_string(),
            to_server_cap: 1024,
            to_gw_cap: 8192,
            write_timeout_ms: 5000,
            high_water_warn_pct: 80,
        }
    }
}

/// The whole configuration (unknown sections/keys ignored).
#[derive(Clone, Debug, Deserialize)]
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
    /// `[web]` — the raw section, deserialized into `mush_net_web::WebConfig`
    /// by the gateway / all-in-one binary that starts the web ingress (kept
    /// untyped here so `mush-config` need not depend on `mush-net-web`);
    /// `features`/`panels` are filled from the module registry at boot.
    #[serde(default = "empty_toml_table")]
    pub web: toml::Value,
    /// `[game]`.
    pub game: GameCfg,
    /// `[rulesets]` — which ruleset module(s) load (M6).
    pub rulesets: RulesetsCfg,
    /// `[weather]` — the M10 weather LLM flavor top-up knobs (variants per
    /// bucket). Off-config defaults match the crate.
    pub weather: WeatherCfg,
    /// `[worldgen]` — the M10 batch `@gen/building` knobs (max rooms per
    /// batch). Off-config defaults match the crate.
    pub worldgen: WorldgenCfg,
    /// `[onboarding]` — the `chargen_required` gate + start rooms (M9). Off by
    /// default; consumed by the WoD ruleset via [`OnboardingCfg::to_ruleset`].
    pub onboarding: OnboardingCfg,
    /// `[llm]` — the M7 LLM layer (providers, feature routes, budgets, kill
    /// switch). The raw section, deserialized into `mush_llm::LlmConfig` by the
    /// server / all-in-one binary (kept untyped here so `mush-config` need not
    /// depend on `mush-llm`); the whole block defaults to `enabled = false` (a
    /// keyless clone still boots). API keys are named by env var
    /// (`api_key_env`), never inlined.
    #[serde(default = "empty_toml_table")]
    pub llm: toml::Value,
    /// `[ipc]` — the M16b forked gateway<->server boundary socket. Ignored by
    /// the all-in-one `mushd`; read by `mush-gateway`/`mush-server`.
    pub ipc: IpcCfg,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: ServerCfg::default(),
            log: LogCfg::default(),
            store: StoreCfg::default(),
            ssh: SshCfg::default(),
            web: empty_toml_table(),
            game: GameCfg::default(),
            rulesets: RulesetsCfg::default(),
            weather: WeatherCfg::default(),
            worldgen: WorldgenCfg::default(),
            onboarding: OnboardingCfg::default(),
            llm: empty_toml_table(),
            ipc: IpcCfg::default(),
        }
    }
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

    /// The parsed `[ipc].listen` boundary endpoint (M16b).
    ///
    /// # Errors
    ///
    /// [`ConfigError::BadIpcListen`] when the value is neither a socket address
    /// nor (where supported) a usable Unix path.
    pub fn ipc_endpoint(&self) -> Result<mush_ipc::IpcEndpoint, ConfigError> {
        mush_ipc::IpcEndpoint::parse(&self.ipc.listen).map_err(|msg| ConfigError::BadIpcListen {
            value: self.ipc.listen.clone(),
            detail: msg,
        })
    }

    /// The `[ipc].to_server_cap` bound (M18): gateway->server input-shed cap.
    #[must_use]
    pub fn ipc_to_server_cap(&self) -> usize {
        self.ipc.to_server_cap
    }

    /// The `[ipc].to_gw_cap` bound (M18): server->gateway teardown cap.
    #[must_use]
    pub fn ipc_to_gw_cap(&self) -> usize {
        self.ipc.to_gw_cap
    }

    /// The `[ipc].write_timeout_ms` per-frame IPC write timeout (M18).
    #[must_use]
    pub fn ipc_write_timeout_ms(&self) -> u64 {
        self.ipc.write_timeout_ms
    }

    /// The `[ipc].high_water_warn_pct` edge-gated high-water warn threshold
    /// (M18), clamped to `1..=100`.
    #[must_use]
    pub fn ipc_high_water_warn_pct(&self) -> u8 {
        self.ipc.high_water_warn_pct.clamp(1, 100)
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
    fn weather_and_worldgen_knobs_default_and_parse() {
        // Config-less: both fall back to the crate defaults (mirrored as
        // literals here; the typed conversion is covered in `mush-server`).
        let cfg = Config::default();
        assert_eq!(cfg.weather.flavor_variants, 10);
        assert_eq!(cfg.worldgen.batch_max_rooms, 40);

        // Explicit overrides parse.
        let cfg: Config =
            toml::from_str("[weather]\nflavor_variants = 6\n\n[worldgen]\nbatch_max_rooms = 12")
                .unwrap();
        assert_eq!(cfg.weather.flavor_variants, 6);
        assert_eq!(cfg.worldgen.batch_max_rooms, 12);
    }

    #[test]
    fn onboarding_defaults_off_and_resolves_rooms() {
        // Default: off, both rooms fold to Limbo (#0).
        let cfg = Config::default();
        assert!(!cfg.onboarding.chargen_required);
        assert_eq!(cfg.onboarding.start_room_id(), mush_core::LIMBO);
        assert_eq!(cfg.onboarding.start_room_unapproved_id(), mush_core::LIMBO);

        // Explicit: gate on, a distinct start room, unapproved falls back to it
        // when unset.
        let cfg: Config =
            toml::from_str("[onboarding]\nchargen_required = true\nstart_room = 5").unwrap();
        assert!(cfg.onboarding.chargen_required);
        assert_eq!(cfg.onboarding.start_room_id(), mush_core::ObjId(5));
        assert_eq!(
            cfg.onboarding.start_room_unapproved_id(),
            mush_core::ObjId(5)
        );

        // Both rooms explicit.
        let cfg: Config = toml::from_str(
            "[onboarding]\nchargen_required = true\nstart_room = 5\nstart_room_unapproved = 9",
        )
        .unwrap();
        assert_eq!(cfg.onboarding.start_room_id(), mush_core::ObjId(5));
        assert_eq!(
            cfg.onboarding.start_room_unapproved_id(),
            mush_core::ObjId(9)
        );
    }

    #[test]
    fn ipc_caps_default_and_back_compat() {
        // Config-less: the M18 bounds fall back to their defaults.
        let cfg = Config::default();
        assert_eq!(cfg.ipc_to_server_cap(), 1024);
        assert_eq!(cfg.ipc_to_gw_cap(), 8192);
        assert_eq!(cfg.ipc_write_timeout_ms(), 5000);
        assert_eq!(cfg.ipc_high_water_warn_pct(), 80);

        // Back-compat: an old [ipc] with only `listen` (no M18 keys) still
        // parses and yields the defaults.
        let cfg: Config = toml::from_str("[ipc]\nlisten = \"127.0.0.1:6799\"").unwrap();
        assert_eq!(cfg.ipc.listen, "127.0.0.1:6799");
        assert_eq!(cfg.ipc_to_server_cap(), 1024);
        assert_eq!(cfg.ipc_to_gw_cap(), 8192);
        assert_eq!(cfg.ipc_write_timeout_ms(), 5000);
        assert_eq!(cfg.ipc_high_water_warn_pct(), 80);

        // Explicit overrides parse.
        let cfg: Config = toml::from_str(
            "[ipc]\nlisten = \"127.0.0.1:6799\"\nto_server_cap = 16\nto_gw_cap = 32\nwrite_timeout_ms = 250\nhigh_water_warn_pct = 90",
        )
        .unwrap();
        assert_eq!(cfg.ipc_to_server_cap(), 16);
        assert_eq!(cfg.ipc_to_gw_cap(), 32);
        assert_eq!(cfg.ipc_write_timeout_ms(), 250);
        assert_eq!(cfg.ipc_high_water_warn_pct(), 90);

        // An out-of-range warn pct clamps into 1..=100.
        let cfg: Config = toml::from_str("[ipc]\nhigh_water_warn_pct = 0").unwrap();
        assert_eq!(cfg.ipc_high_water_warn_pct(), 1);
        let cfg: Config = toml::from_str("[ipc]\nhigh_water_warn_pct = 200").unwrap();
        assert_eq!(cfg.ipc_high_water_warn_pct(), 100);
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
        // The example's [weather]/[worldgen] sections parse (values are
        // commented, so they resolve to the crate defaults).
        assert_eq!(cfg.weather.flavor_variants, 10);
        assert_eq!(cfg.worldgen.batch_max_rooms, 40);
        // The example's [llm] block is captured raw here; its typed parse into
        // `mush_llm::LlmConfig` is covered by the server crate that owns it.
        assert!(cfg.llm.is_table());
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
