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
//! | `[store].receipt_policy` | `"applied"` | `"applied"` or strict `"host_power_loss"` receipts |
//! | `[ssh].listen` | `"0.0.0.0:2222"` | SSH listen address |
//! | `[web].listen` | `"127.0.0.1:8080"` | web (HTTP/WS) listen address |
//! | `[web].trusted_proxies` | `[]` | proxies whose `X-Forwarded-For` is honored |
//! | `[game].linkdead_grace_s` | `300` | linkdead grace before disconnect |
//! | `[game].max_characters` | `3` | characters per account cap |
//! | `[game].ic_ratio` | `1` | IC seconds per OOC second (game-clock speed); read at boot |
//! | `[game].ic_epoch` | `"2020-01-01 00:00"` | IC datetime at OOC unix 0 — calendar phase/start; read at boot |
//! | `[rulesets].enabled` | `["wod"]` | ruleset module(s) to load; unknown ids are a hard boot error |
//! | `[weather].flavor_variants` | `10` | M10 variants per weather bucket (`0` disables, hard max `64`) |
//! | `[worldgen].batch_max_rooms` | `40` | M10 `@gen/building` room cap (valid range `2..=40`) |
//! | `[economy].mint_obligation_cap_major` | `50000` | E4 per-period ceiling on a faucet-funded obligation, in major units |
//! | `[economy.instruments."<code>"].mint_obligation_cap_major` | (the above) | per-instrument override of that ceiling |
//! | `[wod.economy].personal_mode` | `"off"` | E5c WoD personal economy: `off` \| `abstract` \| `exact` |
//! | `[wod.economy].housing` | `true` | whether the WoD layer creates tenancies and registers the housing title issuer |
//! | `[wod.economy].careers` | `true` | whether careers, stances and opportunities are evaluated |
//! | `[wod.economy].default_engagement` | `"ambient"` | a new profile's engagement: `ambient` \| `engaged` |
//! | `[wod.economy].cadence` | `"chronicle.period"` | the campaign clock careers and abstract housing are assessed on |
//! | `[wod.economy].starting_funds_major` | `0` | exact mode: chargen starting funds, in major units |
//! | `[wod.economy].reserved_rooms` | `[]` | extra `#N` rooms that may never become property rooms (D-E5-12) |
//! | `[wod.economy.income]."<dots>"` | (none) | exact mode: income per period per Resources dot (`"0"`…`"5"`), major units |
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

use std::collections::BTreeMap;
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
    /// `[store].receipt_policy` is not recognized.
    #[error("unknown store receipt policy {value:?} (expected applied or host_power_loss)")]
    BadStoreReceiptPolicy { value: String },
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

/// What an acknowledged authoritative receipt promises survives.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StoreReceiptPolicy {
    /// Redis applied the write; host power loss may lose the configured AOF
    /// window.
    Applied,
    /// Redis must use AOF with fsync on every write acknowledgement.
    HostPowerLoss,
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

/// `[offers]` section — durable K5 legal-action offer signing.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OffersCfg {
    /// Optional key-ring directory. When absent, keys live beneath
    /// `[server].data_dir/offer-keys`.
    pub key_dir: Option<PathBuf>,
}

/// `[resolution]` section — durable K4 authoritative RNG root keys.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ResolutionCfg {
    /// Optional key-ring directory. When absent, keys live beneath
    /// `[server].data_dir/resolution-keys`.
    pub key_dir: Option<PathBuf>,
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
    /// `"applied"` (default) or `"host_power_loss"`. The stronger policy
    /// refuses boot unless Redis reports AOF + `appendfsync always`.
    pub receipt_policy: String,
}

impl Default for StoreCfg {
    fn default() -> Self {
        StoreCfg {
            store: "redis".to_string(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            receipt_policy: "applied".to_string(),
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
    /// `mush-weather` crate default (10); zero disables generation and values
    /// above the crate's hard maximum (64) are clamped. The typed conversion
    /// into `mush_weather::WeatherConfig` lives in `mush-server`.
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

/// `[economy.instruments."<code>"]` — per-instrument overrides of the `[economy]`
/// limits, for worlds running several currencies at different denominations.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EconomyInstrumentCfg {
    /// This instrument's per-period faucet-obligation ceiling, in **major** units.
    pub mint_obligation_cap_major: u64,
}

impl Default for EconomyInstrumentCfg {
    fn default() -> Self {
        // Mirrors `mush_economy::DEFAULT_MINT_OBLIGATION_CAP_MAJOR` (kept as a
        // literal so `mush-config` needs no `mush-economy` dep).
        EconomyInstrumentCfg {
            mint_obligation_cap_major: 50_000,
        }
    }
}

/// `[economy]` section — the E4b1 verify fold's mint bound (M1).
///
/// The economy's hard invariants are not configurable. What is configurable is how
/// large a hole a ruleset may open in the mint: E5's income table funds careers and
/// stipends from a registered faucet through `mush_economy::create_system_obligation`,
/// and such an obligation is a **standing mint** the scheduler executes forever. The
/// cap bounds one period of one such obligation.
///
/// Written in **major** units — the number a game-runner thinks in ("50,000 a
/// month"), not minor units — and scaled by each instrument's own `scale` at use.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EconomyCfg {
    /// The per-period ceiling on a faucet-funded obligation, in major units, for
    /// every instrument without an override. Defaults to the `mush-economy` crate
    /// default (50,000); the typed conversion into `mush_economy::EconomyConfig`
    /// lives in `mush-server`.
    pub mint_obligation_cap_major: u64,
    /// Per-instrument overrides, keyed by instrument code (`wod.usd`).
    pub instruments: BTreeMap<String, EconomyInstrumentCfg>,
}

impl Default for EconomyCfg {
    fn default() -> Self {
        EconomyCfg {
            mint_obligation_cap_major: 50_000,
            instruments: BTreeMap::new(),
        }
    }
}

/// `[wod]` — the World of Darkness ruleset's own configuration (E5c, 08d §5.1).
///
/// Ruleset-owned by decision **D-E5-9**: there is no platform economy switch, and
/// the knobs below turn the *WoD layer* on and off, never the E2–E4 economy the
/// server runs regardless.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WodCfg {
    /// `[wod.economy]`.
    pub economy: WodEconomyCfg,
}

/// `[wod.economy]` — 08d §5.1's block, verbatim, plus two host-side extras
/// (`starting_funds_major`, `reserved_rooms`).
///
/// The default is **`personal_mode = "off"`**: a server that has never heard of this
/// section keeps exactly its pre-E5c behaviour — housing works, staff create
/// tenancies, and the WoD layer creates no profile, evaluates nothing and shows
/// nothing. `housing` defaults **on** so the title issuer stays wired as E5b wired
/// it.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WodEconomyCfg {
    /// `off` | `abstract` | `exact`. An unknown word is a hard boot error (the
    /// `[rulesets].enabled` rule: a silently-ignored typo is worse than not
    /// starting), resolved in `mush_server::config_ext::wod_economy_config`.
    pub personal_mode: String,
    /// Whether the WoD layer creates tenancies at all, and whether the server
    /// registers `system:wod.housing` as an economy title issuer (08d §4.1's
    /// handoff). Existing tenancies are untouched when this goes false; only new
    /// ones stop.
    pub housing: bool,
    /// Whether careers, stances, opportunities and exposure are evaluated.
    pub careers: bool,
    /// `ambient` | `engaged` — a new profile's engagement.
    pub default_engagement: String,
    /// The **name** of the economy cadence clock (`@econ/cadence list`) careers and
    /// abstract housing are assessed on. Created at the ruleset's boot if it does not
    /// exist.
    pub cadence: String,
    /// Exact mode only: the funds a newly approved character starts with, in major
    /// units, minted from `system:wod.chargen`.
    pub starting_funds_major: u64,
    /// Extra rooms (`#N`) that may never become property rooms (D-E5-12), chained
    /// into `HousingModule::with_reserved_rooms` alongside `[onboarding]`'s two.
    pub reserved_rooms: Vec<u32>,
    /// `[wod.economy.income]` — exact mode only, and **authoritative only there**
    /// (owner answer 3): one per-period amount in **major** units per Resources dot,
    /// keyed `"0"`…`"5"`. Every line is bounded by `[economy]`'s mint cap (with the
    /// per-instrument override for `wod.usd`), re-checked at every settlement.
    pub income: BTreeMap<String, u64>,
}

impl Default for WodEconomyCfg {
    fn default() -> Self {
        WodEconomyCfg {
            personal_mode: "off".to_string(),
            housing: true,
            careers: true,
            default_engagement: "ambient".to_string(),
            cadence: "chronicle.period".to_string(),
            starting_funds_major: 0,
            reserved_rooms: Vec::new(),
            income: BTreeMap::new(),
        }
    }
}

/// `[worldgen]` section — the M10 batch `@gen/building` knobs.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorldgenCfg {
    /// The `@gen/building` interior-walk cap: the most rooms one batch will
    /// describe. Defaults to the `mush-worldgen` crate default (40); values
    /// outside `2..=40` fail module registration. The typed conversion into
    /// `mush_worldgen::WorldgenConfig` lives in `mush-server`.
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
    /// `[offers]` — durable opaque legal-action offer signing keys.
    pub offers: OffersCfg,
    /// `[resolution]` — independent authoritative resolution RNG keys.
    pub resolution: ResolutionCfg,
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
    /// `[economy]` — the mint bound on a ruleset's faucet-funded obligations (E4b1
    /// verify fold, M1). Off-config defaults match the crate.
    pub economy: EconomyCfg,
    /// `[onboarding]` — the `chargen_required` gate + start rooms (M9). Off by
    /// default; consumed by the WoD ruleset via [`OnboardingCfg::to_ruleset`].
    pub onboarding: OnboardingCfg,
    /// `[wod]` — the WoD ruleset's own knobs (E5c, D-E5-9). `personal_mode` is
    /// `"off"` off-config, so an unconfigured server keeps its pre-E5c behaviour.
    pub wod: WodCfg,
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
            offers: OffersCfg::default(),
            resolution: ResolutionCfg::default(),
            ssh: SshCfg::default(),
            web: empty_toml_table(),
            game: GameCfg::default(),
            rulesets: RulesetsCfg::default(),
            weather: WeatherCfg::default(),
            worldgen: WorldgenCfg::default(),
            economy: EconomyCfg::default(),
            onboarding: OnboardingCfg::default(),
            wod: WodCfg::default(),
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

    /// Validated durability meaning of a successful store receipt.
    pub fn store_receipt_policy(&self) -> Result<StoreReceiptPolicy, ConfigError> {
        match self.store.receipt_policy.to_lowercase().as_str() {
            "applied" => Ok(StoreReceiptPolicy::Applied),
            "host_power_loss" => Ok(StoreReceiptPolicy::HostPowerLoss),
            other => Err(ConfigError::BadStoreReceiptPolicy {
                value: other.to_owned(),
            }),
        }
    }

    /// Directory containing the append-only K5 offer-key manifest and its
    /// bounded set of private key files.
    #[must_use]
    pub fn offer_key_dir(&self) -> PathBuf {
        self.offers
            .key_dir
            .clone()
            .unwrap_or_else(|| self.server.data_dir.join("offer-keys"))
    }

    /// Directory containing the append-only K4 resolution-key manifest and
    /// its bounded set of private key files.
    #[must_use]
    pub fn resolution_key_dir(&self) -> PathBuf {
        self.resolution
            .key_dir
            .clone()
            .unwrap_or_else(|| self.server.data_dir.join("resolution-keys"))
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
        assert_eq!(cfg.store.receipt_policy, "applied");
        assert_eq!(cfg.offer_key_dir(), PathBuf::from("./data/offer-keys"));
        assert_eq!(
            cfg.resolution_key_dir(),
            PathBuf::from("./data/resolution-keys")
        );
        assert_eq!(cfg.store_backend().unwrap(), StoreBackend::Redis);
        assert_eq!(
            cfg.store_receipt_policy().unwrap(),
            StoreReceiptPolicy::Applied
        );
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

    /// E4b1 verify fold, M1: the `[economy]` mint bound parses, the per-instrument
    /// override is keyed by instrument code, and an **absent** section is exactly the
    /// crate defaults — a server that has never heard of `[economy]` behaves as it
    /// did before the setting existed.
    #[test]
    fn vf_m1_economy_section_parses_and_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.economy.mint_obligation_cap_major, 50_000);
        assert!(cfg.economy.instruments.is_empty());

        let cfg: Config = toml::from_str(
            "[server]\nname = \"x\"\n\n\
             [economy]\nmint_obligation_cap_major = 2500\n\n\
             [economy.instruments.\"wod.usd\"]\nmint_obligation_cap_major = 9\n",
        )
        .unwrap();
        assert_eq!(cfg.economy.mint_obligation_cap_major, 2_500);
        assert_eq!(
            cfg.economy
                .instruments
                .get("wod.usd")
                .map(|c| c.mint_obligation_cap_major),
            Some(9)
        );
        // An instrument with no override is not in the map at all; the caller falls
        // back to the section-wide value.
        assert!(!cfg.economy.instruments.contains_key("wod.eur"));

        // A section naming only an override keeps the default ceiling for the rest.
        let cfg: Config =
            toml::from_str("[economy.instruments.\"wod.usd\"]\nmint_obligation_cap_major = 1\n")
                .unwrap();
        assert_eq!(cfg.economy.mint_obligation_cap_major, 50_000);
    }

    #[test]
    fn offer_key_directory_defaults_under_data_and_can_be_overridden() {
        let cfg: Config = toml::from_str(
            "[server]\ndata_dir = \"./world\"\n[offers]\nkey_dir = \"./secrets/offers\"",
        )
        .unwrap();
        assert_eq!(cfg.offer_key_dir(), PathBuf::from("./secrets/offers"));

        let defaulted: Config = toml::from_str("[server]\ndata_dir = \"./world\"").unwrap();
        assert_eq!(
            defaulted.offer_key_dir(),
            PathBuf::from("./world/offer-keys")
        );
    }

    #[test]
    fn resolution_key_directory_defaults_under_data_and_can_be_overridden() {
        let cfg: Config = toml::from_str(
            "[server]\ndata_dir = \"./world\"\n[resolution]\nkey_dir = \"./secrets/resolution\"",
        )
        .unwrap();
        assert_eq!(
            cfg.resolution_key_dir(),
            PathBuf::from("./secrets/resolution")
        );

        let defaulted: Config = toml::from_str("[server]\ndata_dir = \"./world\"").unwrap();
        assert_eq!(
            defaulted.resolution_key_dir(),
            PathBuf::from("./world/resolution-keys")
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
    fn store_receipt_policy_is_explicit_and_validated() {
        let cfg: Config = toml::from_str("[store]\nreceipt_policy = \"host_power_loss\"").unwrap();
        assert_eq!(
            cfg.store_receipt_policy().unwrap(),
            StoreReceiptPolicy::HostPowerLoss
        );
        let cfg: Config = toml::from_str("[store]\nreceipt_policy = \"hope\"").unwrap();
        assert!(matches!(
            cfg.store_receipt_policy(),
            Err(ConfigError::BadStoreReceiptPolicy { .. })
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
