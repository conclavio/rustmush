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
//! | `[economy.cadences."<name>"].clock` | `"world_ic"` | SP1 declared cadence: `world_ic` \| `real_utc` (D-LBK-7) |
//! | `[economy.cadences."<name>"].period_secs` | (required) | the declared cadence's period, seconds on `clock`, `> 0` |
//! | `[economy.cadences."<name>"].catch_up` | `3` | periods one advance may cover, `1..=8` |
//! | `[wod.economy].personal_mode` | `"off"` | E5c WoD personal economy: `off` \| `abstract` \| `exact` |
//! | `[wod.economy].housing` | `true` | whether the WoD layer creates tenancies and registers the housing title issuer |
//! | `[wod.economy].careers` | `true` | whether careers, stances and opportunities are evaluated |
//! | `[wod.economy].default_engagement` | `"ambient"` | a new profile's engagement: `ambient` \| `engaged` |
//! | `[wod.economy].cadence` | `"chronicle.period"` | the campaign clock careers and abstract housing are assessed on |
//! | `[wod.economy].starting_funds_major` | `0` | exact mode: chargen starting funds, in major units |
//! | `[wod.economy].reserved_rooms` | `[]` | extra `#N` rooms that may never become property rooms (D-E5-12) |
//! | `[wod.economy.income]."<dots>"` | (none) | exact mode: income per period per Resources dot (`"0"`…`"5"`), major units |
//! | `[shadowrun.economy].enabled` | `false` | E6a Shadowrun run-and-pay layer: accounts, lifestyle, the run board |
//! | `[shadowrun.economy].instrument` | `"sr.nuyen"` | the currency code registered at scale 0 (D-E6-1) |
//! | `[shadowrun.economy].lifestyle_clock` | `"sr.month"` | the declared cadence lifestyle is billed on (contributed to `[economy.cadences]`) |
//! | `[shadowrun.economy].lifestyle` | `true` | whether lifestyle obligations are created |
//! | `[shadowrun.economy].player_run_offers` | `true` | whether `+run/post` accepts a personal payer |
//! | `[shadowrun.economy].organization_run_offers` | `true` | whether `+run/post … scope=<org>` is accepted |
//! | `[shadowrun.economy.lifestyle_costs]."<tier>"` | (SR5 defaults) | monthly cost per tier, major units |
//! | `[tacsim].enabled` | `false` | E7 TacSim layer: content packs, the campaign-seed latch, the corridor. Independent of `[rulesets]` |
//! | `[tacsim].pack_paths` | `[]` | rules-definition pack files or directories; read **host-side** at boot, and an unreadable/oversized/mis-digested one is a hard boot error naming the file |
//! | `[tacsim].seed_paths` | `[]` | campaign-seed pack files or directories, read the same way |
//! | `[tacsim].corridor` | `""` | which seed pack `@tacsim/seed` defaults to |
//! | `[tacsim].active_domain_limit` | `8` | parsed, **inert** until E7 cut 6 (`MAX_ACTIVE_DOMAINS`) |
//! | `[tacsim].ground_pulse_ms` | `2000` | parsed, **inert** until E7 cut 8 |
//! | `[tacsim].space_pulse_ms` | `6000` | parsed, **inert** until E7 cut 8 |
//! | `[tacsim.time].rate` | `0` | parsed, **inert** until E7 cuts 4/8/11 — campaign rate *policy*. It **never seeds the world timeline**: generation 1 comes from `[game]` |
//! | `[tacsim.time].catch_up_*` | `""` | parsed, **inert** — the per-subsystem catch-up policies of cuts 4/6/8/9 |
//! | `[tacsim.recovery].enabled` | `false` | E7 residue pass 1 (R4): the recovery provider. Disabled, `@recovery` still registers and answers "not enabled" |
//! | `[tacsim.recovery].pool_account` | `0` | the provider's budget account. **Never minted into** — a subsidy is an ordinary staff-authored system obligation, outside this block |
//! | `[tacsim.recovery].pool_floor` | `0` | the balance the pool may not be granted below: *funded, never infinite* |
//! | `[tacsim.recovery].tiers` | `[]` | the loaner tier table, cheapest first |
//! | `[tacsim.recovery].applications_per_principal` | `1` | live applications one principal may hold |
//! | `[tacsim.recovery].cooldown_days` | `14` | 10b §7's `RECOVERY_COOLDOWN`, cited not redefined |
//! | `[tacsim.recovery].max_operational_assets` | `0` | the eligibility threshold, over **accessible** assets |
//! | `[tacsim.recovery].insurance` | `"approved_operations"` | the insurance policy word; `none` is the other |
//! | `[tacsim.recovery].upkeep_per_period` | `0` | the lease's rent obligation, per campaign cadence period |
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
    /// `[store.archive]` — the ledger archive tier (J5, doc 12 §10).
    pub archive: ArchiveCfg,
}

/// `[store.archive]` — the ledger archive tier (J5, doc 12 §10): a nightly
/// pass inside a UTC maintenance window copies every sealed ledger chunk to
/// compressed disk under `[server].data_dir`, reads it back and verifies it,
/// and — above a high watermark of ledger bytes resident in the store, and only
/// for chunks archived at least `retain_hours` ago — purges the store of them
/// down to a low watermark. Reads fall through to the archive, so a boot still
/// replays the whole journal. Off by default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ArchiveCfg {
    /// Whether the tier is attached and the pass runs.
    pub enabled: bool,
    /// The archive directory, under `[server].data_dir`.
    pub dir: String,
    /// The daily window, `HH:MM-HH:MM` in UTC; the pass and the purge run only
    /// inside it, once a day.
    pub window: String,
    /// Never purge a chunk the archive copied less than this many hours ago.
    pub retain_hours: u64,
    /// Ledger bytes resident in the store (MB) that start a purge.
    pub high_watermark_mb: u64,
    /// The level a purge stops at (MB).
    pub low_watermark_mb: u64,
    /// Chunks per archive segment file.
    pub segment_chunks: usize,
    /// The most chunks one pass may purge.
    pub max_purge_per_pass: usize,
}

impl Default for ArchiveCfg {
    fn default() -> Self {
        ArchiveCfg {
            enabled: false,
            dir: "archive".to_string(),
            window: "03:00-04:00".to_string(),
            retain_hours: 48,
            high_watermark_mb: 512,
            low_watermark_mb: 256,
            segment_chunks: 256,
            max_purge_per_pass: 4_096,
        }
    }
}

impl Default for StoreCfg {
    fn default() -> Self {
        StoreCfg {
            store: "redis".to_string(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            receipt_policy: "applied".to_string(),
            archive: ArchiveCfg::default(),
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

/// `[economy.cadences."<name>"]` — one **declared** cadence clock (SP1, D-LBK-7 as
/// decided in 08h §2 Q6).
///
/// A declared cadence is registered at composition as a kernel clock
/// (`ClockRef::Registered { owner: economy, id: cadence.<name> }`) and adopted or
/// created at boot. The name is the table key and must be a cadence name that is
/// also a canonical clock id (lowercase `[a-z0-9_.]`, no empty segment, at most 32
/// bytes); at most 32 may be declared. All of that is validated in `mush-server`'s
/// typed conversion — a bad stanza refuses to start, never silently registers
/// nothing. `deny_unknown_fields` for the same reason: a `catchup = 5` typo must not
/// quietly become the default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EconomyCadenceCfg {
    /// What the clock advances from: `"world_ic"` (the default) or `"real_utc"`.
    /// A manual clock is a human's to step and cannot be declared.
    pub clock: String,
    /// The period in seconds on `clock`. No usable default: `0` is refused.
    pub period_secs: i64,
    /// How many periods one advance may cover (`1..=8`; `mush-economy`'s
    /// `DEFAULT_CATCH_UP` when omitted).
    pub catch_up: u32,
}

impl Default for EconomyCadenceCfg {
    fn default() -> Self {
        // Mirrors `mush_economy::DEFAULT_CATCH_UP` (kept as a literal so `mush-config`
        // needs no `mush-economy` dep).
        EconomyCadenceCfg {
            clock: "world_ic".to_string(),
            period_secs: 0,
            catch_up: 3,
        }
    }
}

/// `[economy]` section — the E4b1 verify fold's mint bound (M1), and since SP1 the
/// declared cadence clocks (D-LBK-7).
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
    /// `[economy.cadences]` — the declared cadence clocks, keyed by name (SP1,
    /// D-LBK-7). Empty off-config: a server that declares nothing registers no
    /// clock and every cadence stays a runtime one.
    pub cadences: BTreeMap<String, EconomyCadenceCfg>,
    /// How long a live contract may sit with no condition progress before the
    /// economy sweep pages staff, in seconds (E6a step 1, DL-08). Defaults to the
    /// `mush-economy` crate default (7 days). A contract whose own deadline has not
    /// lapsed is never paged for staleness.
    pub stale_contract_age_secs: i64,
    /// How many of the newest journal transactions the economy keeps in RAM as
    /// full records (doc 12 J2). Older ones stay reachable through the journal.
    /// Defaults to the `mush-economy` crate default (20 000); zero means that.
    pub tx_tail_max: usize,
    /// How many journal entries past the last checkpoint start the next one
    /// (doc 12 J3). Defaults to the `mush-economy` crate default (10 000);
    /// zero disables writing checkpoints.
    pub checkpoint_every_entries: u64,
    /// How long a posted idempotency key stays in RAM, in seconds (doc 13 W2).
    /// Every key is on disk forever; this is a cache. Defaults to the
    /// `mush-economy` crate default (30 days).
    pub idempotency_window_secs: i64,
    /// At most this many keys in RAM, whichever bound is smaller (doc 13 W2).
    /// Defaults to the `mush-economy` crate default (1 000 000); zero means that.
    pub idempotency_window_max: usize,
}

impl Default for EconomyCfg {
    fn default() -> Self {
        EconomyCfg {
            mint_obligation_cap_major: 50_000,
            instruments: BTreeMap::new(),
            cadences: BTreeMap::new(),
            // Mirrors `mush_economy::STALE_CONTRACT_AGE_SECS` (kept as a literal so
            // `mush-config` needs no `mush-economy` dep).
            stale_contract_age_secs: 7 * 86_400,
            // Mirrors `mush_economy::DEFAULT_TX_TAIL_MAX`.
            tx_tail_max: 20_000,
            // Mirrors `mush_economy::DEFAULT_CHECKPOINT_EVERY_ENTRIES`.
            checkpoint_every_entries: 10_000,
            // Mirrors `mush_economy::DEFAULT_IDEMPOTENCY_WINDOW_SECS` / `_MAX`.
            idempotency_window_secs: 30 * 86_400,
            idempotency_window_max: 1_000_000,
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

/// `[shadowrun]` — the Shadowrun ruleset's own configuration (E6a, 08f §9).
///
/// Ruleset-owned exactly as `[wod]` is (D-E5-9): there is no platform economy switch,
/// and the knobs below turn the *Shadowrun layer* on and off, never the E2–E4 economy
/// the server runs regardless.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ShadowrunCfg {
    /// `[shadowrun.economy]`.
    pub economy: ShadowrunEconomyCfg,
}

/// `[shadowrun.economy]` — 08f §9's block (answer 16), plus the board bounds.
///
/// The default is **`enabled = false`**: a server that has never heard of this
/// section keeps exactly its pre-E6a behaviour — SR5 rolls dice, `nuyen` is a sheet
/// number, and the economic layer creates nothing, evaluates nothing and shows
/// nothing.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ShadowrunEconomyCfg {
    /// Whether the layer exists at all. An `enabled = false` server still registers
    /// the verbs; they answer "not enabled on this server".
    pub enabled: bool,
    /// The instrument code. `sr.nuyen` at scale 0 (D-E6-1).
    pub instrument: String,
    /// The **name** of the cadence clock lifestyle is assessed on. When
    /// `[rulesets].enabled` names `shadowrun`, the host contributes the matching
    /// `[economy.cadences]` stanza unless the operator declared it (SP1, D-LBK-7).
    pub lifestyle_clock: String,
    /// Whether lifestyle obligations are created at all. `false` creates no new one
    /// and cancels nothing at boot — `@sr/migrate` ends the existing ones.
    pub lifestyle: bool,
    /// Whether `+run/post` accepts a personal payer.
    pub player_run_offers: bool,
    /// Whether `+run/post … scope=<org>` is accepted.
    pub organization_run_offers: bool,
    /// Parsed, inert until E6c (§11 q9).
    pub chargen_shopping: bool,
    /// Parsed, inert until E6c.
    pub catalog_paths: Vec<String>,
    /// `[shadowrun.economy.lifestyle_costs]` — one monthly cost in **major** units
    /// per tier, keyed `"street"`…`"luxury"`. A tier the table does not name, or one
    /// named with `0`, creates no obligation.
    pub lifestyle_costs: BTreeMap<String, u64>,
    /// Parsed, inert until E6c (`QUOTE_TTL_LIVE`, seconds).
    pub quote_ttl_live: i64,
    /// Parsed, inert until E6c (`QUOTE_TTL_REMOTE`, seconds).
    pub quote_ttl_remote: i64,
    /// Permille of the catalog baseline an immediate fence pays — inert until E6b.
    pub fence_rate_immediate: u32,
    /// Permille a consignment fence pays — inert until E6b.
    pub fence_rate_consign: u32,
    /// The consignment delay in seconds — inert until E6b.
    pub fence_delay_secs: i64,
    /// `[shadowrun.economy.fence_baseline]` — the **source-controlled reference
    /// price** a fence quotes against, in major units, keyed on the asset's
    /// `CatalogRef.code` (E6b, 08f §4.3 OPEN 1).
    ///
    /// There is no catalog until E6c, and `CatalogRef` is set nowhere today, so this
    /// table is the whole baseline. An object whose asset record carries no catalog
    /// reference, or one this table does not name, **cannot be quoted** — refused,
    /// never guessed, and never priced off its `AcquisitionRecord`, which is
    /// provenance the fence may not see (D-E6-11).
    pub fence_baseline: BTreeMap<String, u64>,
    /// `[shadowrun.economy].vendor_orgs` — the organization **ids** whose contract
    /// acceptance may be performed programmatically (E6b, DL-11 / M15).
    ///
    /// A staff-built NPC fence has no officer and an org party accepts only through
    /// `acts_for` holders or staff, which would put a staffer in every deal. This is
    /// the exception, and it is deliberately **composition data**: ids only, set in
    /// `mushd.toml`, frozen at wiring, and nothing a command or a grant can add to.
    /// It confers no money authority at all — the escrow still posts under the org
    /// account's own debit authority.
    pub vendor_orgs: Vec<u64>,
    /// The per-principal open-listing cap (§9's `MAX_OPEN_RUNS_PER_PRINCIPAL`).
    pub active_offer_cap: usize,
    /// How long a listing lives, in seconds — the contract's own deadline (M8).
    pub board_ttl_secs: i64,
    /// The whole board's open-listing cap.
    pub board_cap: usize,
    /// The 5,000¥ carry-over cap: **parsed and deliberately not applied** until
    /// chargen shopping ships (answer 17).
    pub chargen_carryover_cap: u64,
}

impl Default for ShadowrunEconomyCfg {
    fn default() -> Self {
        // Every literal mirrors `ruleset_shadowrun::SrEconomyConfig::default()` (kept
        // as literals so `mush-config` needs no ruleset dep); `config_ext` converts.
        ShadowrunEconomyCfg {
            enabled: false,
            instrument: "sr.nuyen".to_string(),
            lifestyle_clock: "sr.month".to_string(),
            lifestyle: true,
            player_run_offers: true,
            organization_run_offers: true,
            chargen_shopping: false,
            catalog_paths: Vec::new(),
            lifestyle_costs: BTreeMap::new(),
            fence_baseline: BTreeMap::new(),
            vendor_orgs: Vec::new(),
            quote_ttl_live: 3_600,
            quote_ttl_remote: 24 * 3_600,
            fence_rate_immediate: 300,
            fence_rate_consign: 600,
            fence_delay_secs: 48 * 3_600,
            active_offer_cap: 4,
            board_ttl_secs: 14 * 86_400,
            board_cap: 128,
            chargen_carryover_cap: 5_000,
        }
    }
}

/// `[tacsim]` — the TacSim layer's own configuration (E7 cut 1, 10b §7).
///
/// **Not a ruleset block.** `[rulesets].enabled` still selects exactly one character
/// ruleset (`wod` | `shadowrun`); TacSim is a *layer* that composes beside whichever
/// one that is (doc 10 §20), so its switch lives here rather than in `[rulesets]`.
///
/// The default is **`enabled = false`**: a server that has never heard of this section
/// is bit-identical to one built before E7 existed. The verbs still register and answer
/// "not enabled on this server" — E5c deviation 1, and `[shadowrun.economy]`'s
/// precedent.
/// **`deny_unknown_fields`, on `EconomyCadenceCfg`'s precedent** (independent verify
/// B6). Every other block on this tree tolerates a typo, and that is survivable where
/// a typo costs a default. `[tacsim]` is the only section whose keys carry a **boot
/// precondition**: `enable = true` or `pack_path = [...]` parsed clean and yielded
/// `enabled = false` with no paths, so the server booted with the layer off and 10b
/// §3.1 invariant (iv) — *an unreadable, oversized, unparseable or digest-mismatched
/// pack is a hard boot error naming the file* — was silently not in force, because
/// `tacsim_config`'s `if cfg.enabled` branch read nothing. `mushd.toml.example` promises
/// a hard boot error naming the file one paragraph away, for pack files; a typo in the
/// key that decides whether any file is read at all must not be quieter than that.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimCfg {
    /// Whether the layer does anything.
    pub enabled: bool,
    /// Rules-definition pack files, or directories searched for `*.json`.
    ///
    /// Read **host-side** (`mush_server::config_ext::tacsim_config`), so an
    /// unreadable, oversized or digest-mismatched pack is a hard boot error naming
    /// the file, before any module runs (10b §3.1 invariant (iv)).
    pub pack_paths: Vec<String>,
    /// Campaign-seed pack files or directories, read exactly the same way.
    pub seed_paths: Vec<String>,
    /// Which campaign-seed pack `@tacsim/seed` applies when given no argument.
    pub corridor: String,
    /// **Parsed, inert until E7 cut 6** — 10b §7's `MAX_ACTIVE_DOMAINS`.
    pub active_domain_limit: u32,
    /// **Parsed, inert until E7 cut 8** — the ground pulse period.
    pub ground_pulse_ms: u32,
    /// **Parsed, inert until E7 cut 8** — the space pulse period.
    pub space_pulse_ms: u32,
    /// `[tacsim.time]`.
    pub time: TacsimTimeCfg,
    /// `[tacsim.recovery]` — **E7 residue pass 1, R4**.
    pub recovery: TacsimRecoveryCfg,
}

/// `[npc]` — the NPC agent layer's world-task knobs (H2a/H2b). A new
/// section: nothing here existed before H2, and every field defaults, so an
/// unconfigured server keeps its pre-H2 behaviour.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NpcCfg {
    /// H2b: how long an NPC's scene turn may wait for its durable decision
    /// before the idle fallback fires, seconds (default 120).
    pub scene_turn_deadline_secs: u32,
}

impl Default for NpcCfg {
    fn default() -> Self {
        NpcCfg {
            scene_turn_deadline_secs: 120,
        }
    }
}

impl Default for TacsimCfg {
    fn default() -> Self {
        TacsimCfg {
            enabled: false,
            pack_paths: Vec::new(),
            seed_paths: Vec::new(),
            corridor: String::new(),
            active_domain_limit: 8,
            ground_pulse_ms: 2_000,
            space_pulse_ms: 6_000,
            time: TacsimTimeCfg::default(),
            recovery: TacsimRecoveryCfg::default(),
        }
    }
}

/// `[tacsim.recovery]` — the recovery provider's policy (**E7 residue pass 1, R4**;
/// 10b §15.1 item 9, doc 10 §12 Q7A / §13 T7).
///
/// Cut 11 did not add this block at all, and said so for a reason a knob deserves:
/// *a knob that parses and configures nothing is a promise*. It is added here in the
/// same pass that builds the provider it configures, so every field below has a
/// reader (10b §15.1 item 5).
///
/// **`deny_unknown_fields`, on [`TacsimCfg`]'s own precedent.** This block decides
/// whether a player who has lost everything can get a hull back and on what terms; a
/// mistyped key that silently took a default would be a policy the operator believes
/// is in force and is not. A typo **refuses the boot**.
///
/// **The pool is never minted into.** `pool_account` names an ordinary account the
/// provider organization already holds; nothing in `mush-campaign` mints, and there
/// is no faucet field here on purpose. A deployment that wants to subsidize the
/// provider does it the ordinary way — a staff-authored system obligation through
/// `@account/system` and the economy's own `[economy].mint_cap` — outside this block
/// and outside the campaign crate (10b §3.11's replenishment row: *the provider's own
/// production work orders and salvage recovery, funded by its budget — never a mint
/// into the pool*).
///
/// **It does not need `[tacsim].enabled`, and that is deliberate** (independent
/// verify fold, B12; owner decision F4(ii)(b)). `mushd` passes
/// `modules::recovery_policy(&tacsim.recovery)` into `server_modules_with`
/// unconditionally and `config_ext::recovery_config` validates on *this* block's own
/// `enabled`, so `[tacsim] enabled = false` with `[tacsim.recovery] enabled = true`
/// composes a live provider on a server with no packs, no corridor and no TacSim
/// verbs. That is allowed and supported: **the provider is campaign-shaped, not
/// tacsim-shaped** — it grants against `mush-sim` placement and `mush-campaign`
/// formations, needs no route, no pack and no domain, and reaches nothing in
/// `ruleset-tacsim` at all. It lives under `[tacsim.recovery]` because that is where
/// the campaign layer's operator-facing knobs live, not because it depends on the
/// layer. A deployment that wants no loaner provider sets **this** block's `enabled`
/// to `false`, which is already the default.
///
/// **The `insurance` word governs debt-versus-writeoff and nothing else**
/// (independent verify fold, C6/B2; owner decision F4(i)(b)). It used to gate the
/// whole cooldown, so on every default deployment — `"approved_operations"` is the
/// default here, in `ruleset-tacsim` and in `mush-campaign` — `cooldown_days` was a
/// value this host **validated at boot** and nothing ever read. Q7A's *"waived when
/// the loss was inside the restrictions"* is a per-loss fact, and nothing records
/// whether any loss was; the per-loss waiver is 10b §15.3's pass-2 item.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimRecoveryCfg {
    /// Whether the provider grants at all. `false` (the default) leaves `@recovery`
    /// registered and answering "not enabled on this server" — E5c deviation 1.
    pub enabled: bool,
    /// The provider's budget account id. Zero is unwired, and an unwired provider
    /// **queues and pages** rather than granting.
    pub pool_account: u64,
    /// The floor the pool may not be granted below, in the instrument's minor units.
    /// The provider stops granting at it: *funded, never infinite* (doc 10 §7 7A).
    pub pool_floor: i64,
    /// The loaner tier table, cheapest first. A grant names one of these.
    pub tiers: Vec<String>,
    /// How many live applications one principal may hold.
    pub applications_per_principal: u32,
    /// The per-player cooldown after a lease ends, in days. Defaults to 10b §7's
    /// `RECOVERY_COOLDOWN` of 14 d, **cited rather than redefined**, and **applies
    /// under every `insurance` word** since the fold (C6; F4(i)(b)).
    pub cooldown_days: u32,
    /// The eligibility threshold: an applicant with **more** than this many
    /// accessible operational assets is not eligible. Zero — the default — is doc 10
    /// S3A's reading: a player whose only hull is an unreachable wreck qualifies.
    pub max_operational_assets: u32,
    /// The insurance policy word: `approved_operations` or `none`. It decides whether
    /// a loss inside the lease's restrictions becomes a **debt** or is written off —
    /// and, since the fold, nothing else: it no longer switches the cooldown off (C6).
    pub insurance: String,
    /// The lease's upkeep, per campaign cadence period, in minor units.
    pub upkeep_per_period: i64,
}

impl Default for TacsimRecoveryCfg {
    fn default() -> Self {
        TacsimRecoveryCfg {
            enabled: false,
            pool_account: 0,
            pool_floor: 0,
            tiers: Vec::new(),
            applications_per_principal: 1,
            // 10b §7's `RECOVERY_COOLDOWN` 14 d.
            cooldown_days: 14,
            max_operational_assets: 0,
            insurance: "approved_operations".to_string(),
            upkeep_per_period: 0,
        }
    }
}

/// `[tacsim.time]` — the campaign's rate and catch-up **policy** (Q8D).
///
/// Every field is **parsed and inert** in E7 cut 1; cuts 4, 8 and 11 read them.
///
/// **It never seeds the world timeline.** Generation one is seeded from
/// `[game].ic_ratio`/`ic_epoch` by `mush_server::timeline::load_or_seed_world_timeline`
/// — and only when the reserved namespace is empty — with persisted truth winning
/// thereafter. This block is campaign rate *policy*, and it reaches that function
/// never; a pinned `mush-server` test asserts exactly that (08i §3's configuration row).
/// `deny_unknown_fields` for [`TacsimCfg`]'s reason: a mistyped catch-up policy word
/// here is a policy cut 4/8/11 will silently not have (independent verify B6).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimTimeCfg {
    /// IC seconds per real second the campaign layer intends. Doc 10 S1A's "≈ one OOC
    /// hour per IC day" is a value here, never a constant in code. **Inert:** opening
    /// a generation is a cut-11 verb over the kernel's own timeline.
    pub rate: u32,
    /// The journey catch-up policy word. **Inert until cut 6.**
    pub catch_up_journeys: String,
    /// The production catch-up policy word. **Inert until cut 4.**
    pub catch_up_production: String,
    /// The fronts catch-up policy word. **Inert until cut 9.**
    pub catch_up_fronts: String,
    /// The tactical-domain catch-up policy word. **Inert until cut 8.**
    pub catch_up_domains: String,
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
    /// `[discord]` — the raw section, deserialized into
    /// `mush_net_discord::DiscordConfig` by the gateway / all-in-one binary that
    /// starts the Discord bot (kept untyped here so `mush-config` need not depend
    /// on `mush-net-discord`). Off by default (`enabled = false`); the bot token
    /// is named by env var (`token_env`), never inlined.
    #[serde(default = "empty_toml_table")]
    pub discord: toml::Value,
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
    /// `[shadowrun]` — the Shadowrun ruleset's own knobs (E6a, 08f §9). `enabled` is
    /// `false` off-config, so an unconfigured server keeps its pre-E6a behaviour.
    pub shadowrun: ShadowrunCfg,
    /// `[tacsim]` — the TacSim layer's own knobs (E7 cut 1, 10b §7). `enabled` is
    /// `false` off-config, so an unconfigured server keeps its pre-E7 behaviour. It is
    /// deliberately **not** a `[rulesets].enabled` id: TacSim is a layer beside the one
    /// character ruleset, not a third one (doc 10 §20).
    pub tacsim: TacsimCfg,
    /// `[npc]` — the NPC agent layer's world-task knobs (H2). Defaults whole.
    #[serde(default)]
    pub npc: NpcCfg,
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
            discord: empty_toml_table(),
            game: GameCfg::default(),
            rulesets: RulesetsCfg::default(),
            weather: WeatherCfg::default(),
            worldgen: WorldgenCfg::default(),
            economy: EconomyCfg::default(),
            onboarding: OnboardingCfg::default(),
            wod: WodCfg::default(),
            shadowrun: ShadowrunCfg::default(),
            tacsim: TacsimCfg::default(),
            npc: NpcCfg::default(),
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

    /// Directory containing the independent K7 delayed-decision HMAC keys.
    #[must_use]
    pub fn decision_key_dir(&self) -> PathBuf {
        self.server.data_dir.join("decision-keys")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H2a: `[npc]` is a whole-default section, and a misspelt key is a boot
    /// error like every other typed section.
    #[test]
    fn h2a_npc_section_defaults_whole_and_refuses_unknown_keys() {
        let absent: Config = toml::from_str("").expect("an empty config parses");
        assert_eq!(absent.npc.scene_turn_deadline_secs, 120);
        let set: Config = toml::from_str(
            "[npc]
scene_turn_deadline_secs = 30
",
        )
        .expect("parses");
        assert_eq!(set.npc.scene_turn_deadline_secs, 30);
        let refused = toml::from_str::<Config>(
            "[npc]
scene_turn_dedline_secs = 30
",
        );
        assert!(refused.is_err(), "an unknown [npc] key must refuse");
    }

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
        assert_eq!(
            cfg.decision_key_dir(),
            PathBuf::from("./data/decision-keys")
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

    /// SP1 (D-LBK-7): `[economy.cadences]` parses as a name-keyed table, `clock` and
    /// `catch_up` default to `create_cadence`'s (`world_ic`, `3`), `period_secs` has
    /// no usable default, and an unknown key in a stanza is refused rather than
    /// ignored. Absent, the table is empty.
    #[test]
    fn sp1_economy_cadences_parse_with_defaults() {
        assert!(Config::default().economy.cadences.is_empty());
        let cfg: Config = toml::from_str(
            "[economy.cadences.\"chronicle.period\"]\nperiod_secs = 2592000\n\n\
             [economy.cadences.\"sr.month\"]\nclock = \"real_utc\"\nperiod_secs = 604800\n\
             catch_up = 5\n",
        )
        .unwrap();
        let chronicle = &cfg.economy.cadences["chronicle.period"];
        assert_eq!(chronicle.clock, "world_ic");
        assert_eq!(chronicle.period_secs, 2_592_000);
        assert_eq!(chronicle.catch_up, 3);
        let month = &cfg.economy.cadences["sr.month"];
        assert_eq!(month.clock, "real_utc");
        assert_eq!(month.period_secs, 604_800);
        assert_eq!(month.catch_up, 5);
        // The inline-table form is the same shape.
        let cfg: Config = toml::from_str(
            "[economy.cadences]\nseason = { clock = \"real_utc\", period_secs = 100 }\n",
        )
        .unwrap();
        assert_eq!(cfg.economy.cadences["season"].period_secs, 100);
        // A typo'd key is a parse error, not a silently defaulted knob.
        let err =
            toml::from_str::<Config>("[economy.cadences.season]\nperiod_secs = 100\ncatchup = 5\n")
                .unwrap_err()
                .to_string();
        assert!(err.contains("catchup"), "{err}");
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

    // ---- E7 cut 1 independent verify fold ----

    /// **B6 — a typo in `[tacsim]` refuses to start rather than silently disarming the
    /// pack gate.**
    ///
    /// `TacsimCfg`/`TacsimTimeCfg` carried `#[serde(default)]` with no
    /// `deny_unknown_fields`, so `enable = true` or `pack_path = [...]` parsed clean and
    /// yielded `enabled = false` with no paths: the server booted with the layer off,
    /// and 10b §3.1 invariant (iv) — *an unreadable, oversized, unparseable or
    /// digest-mismatched pack is a hard boot error naming the file* — was silently not
    /// in force, because `tacsim_config`'s `if cfg.enabled` branch read nothing. This is
    /// deliberately stricter than the rest of the tree (only `EconomyCadenceCfg` had the
    /// attribute, and `WodCfg`/`ShadowrunCfg` still do not): `[tacsim]` is the only
    /// section whose keys carry a boot precondition. The other half of the pair is
    /// `example_config_parses` above, which keeps `mushd.toml.example` loadable.
    #[test]
    fn vf_b6_an_unknown_tacsim_key_refuses_to_start() {
        for (text, key) in [
            ("[tacsim]\nenable = true\n", "enable"),
            ("[tacsim]\npack_path = []\n", "pack_path"),
            (
                "[tacsim.time]\ncatch_up_journey = \"x\"\n",
                "catch_up_journey",
            ),
        ] {
            let refused = toml::from_str::<Config>(text).expect_err(text);
            let refused = refused.to_string();
            assert!(refused.contains(key), "{text}: {refused}");
            assert!(refused.contains("unknown field"), "{text}: {refused}");
        }
        // And the spelled-correctly block still parses to exactly what it says.
        let cfg: Config =
            toml::from_str("[tacsim]\nenabled = true\npack_paths = [\"a.json\"]\n").unwrap();
        assert!(cfg.tacsim.enabled);
        assert_eq!(cfg.tacsim.pack_paths, vec!["a.json".to_string()]);
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

    /// J5: `[store.archive]` parses with its defaults and its knobs.
    #[test]
    fn store_archive_parses_with_defaults() {
        let absent: Config = toml::from_str("").expect("an empty config parses");
        assert!(!absent.store.archive.enabled);
        assert_eq!(absent.store.archive.dir, "archive");
        assert_eq!(absent.store.archive.window, "03:00-04:00");
        assert_eq!(absent.store.archive.retain_hours, 48);
        assert_eq!(
            (
                absent.store.archive.high_watermark_mb,
                absent.store.archive.low_watermark_mb
            ),
            (512, 256)
        );
        let set: Config = toml::from_str(
            "[store.archive]\nenabled = true\nwindow = \"23:30-00:30\"\nretain_hours = 72\nhigh_watermark_mb = 64\nlow_watermark_mb = 32\nsegment_chunks = 16\n",
        )
        .expect("parses");
        assert!(set.store.archive.enabled);
        assert_eq!(set.store.archive.window, "23:30-00:30");
        assert_eq!(set.store.archive.retain_hours, 72);
        assert_eq!(set.store.archive.segment_chunks, 16);
        assert_eq!(set.store.archive.max_purge_per_pass, 4_096, "default kept");
    }
}
