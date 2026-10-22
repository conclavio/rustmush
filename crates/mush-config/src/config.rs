//! `mushd.toml` configuration.
//!
//! Unknown sections/keys remain extensible, except that loading a file refuses
//! an unknown top-level section within two edits of a known section name.
//! This catches headers such as `[tac_sim]` before they disable a layer. Keys used now:
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
//! | `[weather].flavor_variants` | `10` | M10 variants per weather bucket (`0` disables, hard max `64`) |
//! | `[worldgen].batch_max_rooms` | `40` | M10 `@gen/building` room cap (valid range `2..=40`) |
//! | `[worldgen].plan_cost_cap_usd` | `5.0` | S3 building agent: a proposal's cost cap; a run stops before the call that would pass it |
//! | `[worldgen].plan_cost_target_usd` | `2.0` | S3: the target told to the model, never enforced |
//! | `[worldgen].plan_max_rounds` | `6` | S3: the most provider calls one planning run makes (`1..=24`) |
//! | `[worldgen].hydration_cap` | `2` | S3: a character's default cap on outstanding site requests (doc 14 §4.5) |
//! | `[worldgen].hydration_strikes_to_zero` | `3` | S3: strikes at which a character's cap becomes zero until staff reset it |
//! | `[economy].mint_obligation_cap_major` | `50000` | E4 per-period ceiling on a faucet-funded obligation, in major units |
//! | `[economy.instruments."<code>"].mint_obligation_cap_major` | (the above) | per-instrument override of that ceiling |
//! | `[economy.cadences."<name>"].clock` | `"world_ic"` | SP1 declared cadence: `world_ic` \| `real_utc` (D-LBK-7) |
//! | `[economy.cadences."<name>"].period_secs` | (required) | the declared cadence's period, seconds on `clock`, `> 0` |
//! | `[economy.cadences."<name>"].catch_up` | `3` | periods one advance may cover, `1..=8` |
//! | `[tacsim].enabled` | `false` | E7 TacSim layer: content packs, the campaign-seed latch, the corridor |
//! | `[tacsim].pack_paths` | `[]` | rules-definition pack files or directories; read **host-side** at boot, and an unreadable/oversized/mis-digested one is a hard boot error naming the file |
//! | `[tacsim].seed_paths` | `[]` | campaign-seed pack files or directories, read the same way |
//! | `[tacsim].corridor` | `""` | which seed pack `@tacsim/seed` defaults to |
//! | `[tacsim].active_domain_limit` | `8` | parsed, **inert** until E7 cut 6 (`MAX_ACTIVE_DOMAINS`) |
//! | `[tacsim].ground_pulse_ms` | `2000` | parsed, **inert** until E7 cut 8 |
//! | `[tacsim].ground_cell_cm` | `1000` | S7 (doc 14 §5.4): the TacSim ground driver's cell size; one of the atlas's default cell sizes (100, 200, 500, 1000), refused at boot otherwise |
//! | `[tacsim].space_pulse_ms` | `6000` | parsed, **inert** until E7 cut 8 |
//! | `[tacsim.time].rate` | `0` | parsed, **inert** until E7 cuts 4/8/11 — campaign rate *policy*. It **never seeds the world timeline**: generation 1 comes from `[game]` |
//! | `[tacsim.time].catch_up_*` | `""` | parsed, **inert** — the per-subsystem catch-up policies of cuts 4/6/8/9 |
//! | `[tacsim.time.ephemeris].epoch` | `0` | S7 (doc 10 T17): the campaign epoch a body without its own `epoch_ic` has its phase stated at, IC seconds on the game calendar (`mush_core::CivilDate`: 365-day years, year 0 at IC 0) |
//! | `[tacsim.time.ephemeris].rate` | `0` | S7: permille of the real sky's motion per IC second. **0 is the default**: it holds every body at its authored phase (the constant sky; doc 18 §4.8) and every pinned distance holds on any IC day |
//! | `[tacsim.recovery].enabled` | `false` | E7 residue pass 1 (R4): the recovery provider. Disabled, `@recovery` still registers and answers "not enabled" |
//! | `[tacsim.recovery].pool_account` | `0` | the provider's budget account. **Never minted into** — a subsidy is an ordinary staff-authored system obligation, outside this block |
//! | `[tacsim.recovery].pool_floor` | `0` | the balance the pool may not be granted below: *funded, never infinite* |
//! | `[tacsim.recovery].tiers` | `[]` | the loaner tier table, cheapest first |
//! | `[tacsim.recovery].applications_per_principal` | `1` | live applications one principal may hold |
//! | `[tacsim.recovery].cooldown_days` | `14` | 10b §7's `RECOVERY_COOLDOWN`, cited not redefined |
//! | `[tacsim.recovery].max_operational_assets` | `0` | the eligibility threshold, over **accessible** assets |
//! | `[tacsim.recovery].insurance` | `"approved_operations"` | the insurance policy word; `none` is the other |
//! | `[tacsim.recovery].upkeep_per_period` | `0` | the lease's rent obligation, per campaign cadence period |
//! | `[tacsim.economy].housing` | `true` | 16b cut R-b: whether the housing title issuer (`system:tacsim.housing`) is registered so `@property/create` can mint tenancies |
//! | `[tacsim.economy].market` | `false` | whether the market composes with its title issuer (`system:tacsim.market`) |
//! | `[tacsim.economy].instrument` | `"tacsim.credit"` | the settlement currency the sheet registers create-only at boot (doc 11 §2); must be a canonical instrument id |
//! | `[tacsim.economy].reserved_rooms` | `[]` | extra `#N` rooms that may never become property rooms, beside `[onboarding]`'s two |
//! | `[tacsim.arena.tempo].quiet_ms` | `5000` | **16b cut G** (doc 16 §3.1): the `Quiet` pulse period, milliseconds |
//! | `[tacsim.arena.tempo].alert_ms` | `1000` | the `Alert` pulse period, milliseconds |
//! | `[tacsim.arena.tempo].engaged_ms` | `500` | the `Engaged` pulse period, milliseconds — the tempo the harness's p99 is measured in |
//! | `[tacsim.arena.tempo].alert_hold_ms` | `30000` | milliseconds with no `Alert` trigger before the arena falls back to `Quiet` |
//! | `[tacsim.arena.tempo].engaged_hold_ms` | `10000` | milliseconds since the last shot, impact or in-range pair before `Engaged` is left |
//! | `[tacsim.arena.tempo].alert_range_cm` | `0` | the held-contact range that raises `Alert`, centimetres; **`0` is derived** at match start as the lesser of twice the longest weapon range in the match and the window's own diagonal (**16b cut E.1**) |
//! | `[tacsim.arena.tempo].substep_max_ms` | `250` | the longest substep a pulse is cut into (doc 16 §3.1 rate independence); at most `engaged_ms` (**16b cut I.3**: a staff tempo floor must always move arena time) |
//! | `[tacsim.arena.tempo].stall_ms` | `2000` | milliseconds one pulse may take before the watchdog voids the match |
//! | `[tacsim.arena].max_catch_up_steps` | `4` | periods of wall time one pulse may absorb before the arena records `lagged` and skips the idle time |
//! | `[tacsim.arena].occupied_stop_ms` | `2000` | **16b cut E.3** (doc 16 §3.4 step 4): arena milliseconds a unit holds against another unit in its way, keeping its speed, before it stops and says so; `0` stops it at once. Ground, the window's edge and a slope are untouched |
//! | `[tacsim.arena].deliberate_min_ms` | `30000` | **16b cut N.2** (doc 16 §6): the least ARENA time between two deliberation requests for one squad — the floor under the event-driven trigger |
//! | `[tacsim.arena].squad_size` | `4` | 16b cut Z.1 (16c N6): the most machines one squad holds; a side's machines are dealt into `ceil(n / size)` squads of sizes differing by at most one |
//! | `[tacsim.arena].max_units_by_class` | `{ drone = 1 }` | 16b cut AE.3 (doc 16 §2 W2): the most machines of one class a SIDE fields, by class word (`armour`, `walker`, `drone`), `0` to `24`; `0` means the class cannot be fielded; the operator's entries are laid over the compiled `drone = 1` (`walker = 2` alone leaves the drone at 1), and a class not named has no cap |
//! | `[tacsim.arena].snapshot_ms` | `5000` | **16b cut AF.1** (16c N16; doc 16 §3.1 "Durability"): arena milliseconds between two snapshots of a running match; `0` turns snapshots off (a restart then voids every running match by name), else `1000` to `60000`. Read by nothing until 16b cut AF.2 |
//! | `[tacsim.arena.intents].max_pending_per_unit` | `8` | **16b cut M.1** (`T-82`): discrete orders one seat may give a unit per arena second; also the size, per unit, of the world side's inbox and pre-pulse buffer |
//! | `[tacsim.arena.ledger].summary_ms` | `5000` | arena milliseconds between `position_summary` ledger events (doc 16 §4) |
//! | `[tacsim.arena.ledger].max_bytes` | `67108864` | 64 MiB: at 90% summaries stop, at 100% the match is stopped with `Timeout` |
//! | `[tacsim.arena.ledger].max_unflushed_chunks` | `256` | 16 KiB chunks the egress ring may hold before the match is stopped with `Void` |
//! | `[tacsim.arena.reflex].decide_ms` | `1000` | **16b cut N.1** (doc 16 §6): arena milliseconds between one driven hull's motor ticks, staggered by unit id — a cadence of arena time, not of pulses (the reflex controller's decisions until 17b cut M4.6 deleted it; the block keeps its name) |
//! | `[tacsim.arena.reflex].path_max_cells` | `65536` | `[atlas].path_max_cells` **raised for the arena** (16b §9): cells one bounded A\* may expand before the `unreachable` edge fires |
//! | `[tacsim.arena.reflex].max_path_queries` | `64` | path searches the motor may run in one `decide_ms` window of arena time; a unit refused keeps the path it has |
//! | `[tacsim.arena.reflex].repath_ms` | `4000` | the least arena time between two path searches by one unit |
//! | `[tacsim.arena.reflex].withdraw_permille` | `350` | armour plus internals, per mille of the authored totals, below which a unit breaks off toward its entry zone |
//! | `[tacsim.arena.reflex].weight_range` | `100` | the utility weight on range to a candidate hostile; lower scores win |
//! | `[tacsim.arena.reflex].weight_facet` | `40` | the utility weight on the facet a candidate presents |
//! | `[tacsim.arena.reflex].weight_health` | `30` | the utility weight on the chooser's own health — the same addend for every candidate |
//! | `[tacsim.arena.reflex].engage_band_permille` | `700` | the fraction of a weapon's reach `Engage` holds |
//! | `[tacsim.arena.reflex].rally_ms` | `20000` | **16b cut Q.7** (doc 16 §6): arena milliseconds a withdrawn unit waits, with nothing hostile on its board, before it comes back into the fight — once |
//! | `[tacsim.arena.reflex].stand_min_permille` | `175` | **16b cut E.1** (doc 16 §6): armour plus internals, per mille, below which a machine on its way home will not turn and fight even something close; once home it always shoots back. Not above `withdraw_permille` |
//! | `[tacsim.arena.reflex].search_ms` | `60000` | **16b cut E.4** (`B-04`, doc 16 §6's `lost` edge): arena milliseconds a machine that lost its contact keeps driving to where it last saw it; `0` is off, and anything else is `decide_ms` to `600000` |
//! | `[tacsim.arena.reflex].hunt_after_ms` | `90000` | **16b cut E.4** (`B-04`): arena milliseconds with nothing hostile held before a patrolling machine crosses to the other side's entry zone to look; `0` is off, and anything else is at least `decide_ms` |
//! | `[tacsim.match].startup_ms` | `10000` | **16b cut G** (doc 16 §3.3): milliseconds `STARTUP` takes to bring a unit's power online |
//! | `[tacsim.match].board_timeout_ms` | `120000` | milliseconds `Boarding` waits before unseated slots are dropped (doc 16 §3.8) |
//! | `[tacsim.match].max_duration_ms` | `900000` | the longest a match runs before it ends as a timeout; fifteen minutes since 16b cut AP.2 (16c Q47) |
//! | `[tacsim.match].max_slots_per_side` | `12` | pilot slots one side may claim (doc 16 §1's 12 v 12) |
//! | `[tacsim.match].ai_fill` | `false` | **16b cut N.1** (doc 16 §6): whether `SIM START` also fields the tanks of the pods nobody claimed, flown by Standard Doctrine on a unit mind (17b cut M4.3; the reflex controller's before it). Off by default because doc 16 §1's A1 is a two-tank story |
//! | `[tacsim.match].unattended_ms` | `300000` | **16b cut D.3** (`B-33`): world milliseconds a running match every pilot has left runs on before the lobby ends it `unattended`; `0` is off, and anything else is at least `10000` |
//! | `[tacsim.match].reclaim_min_ms` | `5000` | **16b cut Y.3** (16c `N4`; doc 07 §1.3): world milliseconds after a dropped line (the linkdead grace run out, or a `QUIT`) before `BOARD` takes the tank back from its unit mind (the reflex controller until 17b cut M4.3); `0` to `600000` |
//! | `[tacsim.command].max_span` | `4` | **16b cut Y.2** (doc 16 §2 W8): the most squads one commander may hold a live `ORDER` on at once; `1` to `32`, held at the world |
//! | `[tacsim.command].orders_per_minute` | `12` | **16b cut Y.2** (W8's human trigger): the most `ORDER`s one commander may give in sixty world seconds; `1` to `600` |
//! | `[tacsim.minds].fill_class` | `"I"` | Default AI fill class cap when its lead has selected none; pilot minds use `max_class` independently, and every seal also respects its rating and chassis core |
//! | `[tacsim.minds].max_class` | `"III"` | Operator maximum for pilot and AI fill minds; `0`, `I`, `II` or `III` |
//! | `[tacsim.minds].odds_floor_permille` | `100` | **17b cut M1b.1** (doc 17 §5, D-UM-17 as amended): the Compact floor, per mille — the least chance to hit a condition slot may name; captured into each arena at check-out, so a reload never reaches a running match; `10` to `950` in steps of `10` (17b cut M3.3 widened it from `100`: the seal refuses a card under the floor, so a floor above `100` refuses every Line-carded check-out; 17b cut M5.1 lets a principal choose a lawful preset with `SIM MIND`, sealed from 17b cut M5.2) |
//! | `[tacsim.sensors].pass_ms` | `1000` | **16b cut G** (doc 16 §3.6): arena milliseconds between one unit's sensor passes |
//! | `[tacsim.sensors].max_signature_factor` | `1.0` | the ceiling on a target's signature factor, which also scales the candidate cull; the **only float** here, converted once to per-mille at the host so the engine stays integer (doc 16 §3.1 determinism); **at least 1.0** (16b cut M.4: below it the cull is shorter than the range test), and 1.0 is exact because no shipped signature row exceeds 1.0 |
//! | `[tacsim.sensors].lost_after_passes` | `3` | **16b cut S.1** (doc 16 §3.6): passes a contact may go unrefreshed before it is lost; the TTL is `pass_ms × lost_after_passes` |
//! | `[tacsim.sensors].designate_on_detection` | `false` | **16b cut Q.1** (doc 16 §3.6, W5, §5): name a contact by team and centre slot (`BA`) the moment it is detected, instead of by a provisional `X<n>` until it is classified. On, the other side's tanks are legible `classify_after_ms` before the rule allows |
//! | `[tacsim.sensors].condition_green_permille` | `667` | **16b cut Q.2** (doc 16 §3.6): at or above this fraction of what it holds whole, a section reads `green` on `SCAN` |
//! | `[tacsim.sensors].condition_yellow_permille` | `334` | **16b cut Q.2**: at or above this fraction a section reads `yellow`, below it `red` |
//! | `[tacsim.sensors].condition_reach_permille` | `500` | **16b cut Q.2** (doc 16 §2 W5): how near a target must be, as a fraction of the sensor's effective reach, before a pass reads its bands at all — the close look |
//! | `[tacsim.resolution].paths` | `[]` | **16b cut G** (doc 16 §4): the resolution tables (hit curve, error terms, `p_min`, `p_max`); parsed and **inert** until cut W reads them |
//! | `[tacsim.harness].units` | `300` | **16b cut G** (doc 16 §1 A2): units the scale harness places and moves |
//! | `[tacsim.harness].fighting` | `24` | how many of those are fighting |
//! | `[tacsim.harness].engaged_p99_ms` | `40` | the per-pulse p99 ceiling in `Engaged` tempo (16c `T-109`: 25 before the lenses followed the production VPS) |
//! | `[tacsim.harness].max_traces_per_pulse` | `4000` | line-of-sight traces one pulse may make |
//! | `[tacsim.harness].max_path_queries_per_pulse` | `64` | path queries one pulse may make |
//! | `[tacsim.harness].max_share_ops_per_pulse` | `768` | **16b cut S.1** (doc 16 §2 W6): contact-share operations one pulse may make — one `(sharer, teammate)` pair each; 768 since 16b cut J.3b, on the authored lane's measured per-wake peak of 452 at 300 units and the 30-cell mesh (16c Q26, the owner's answer); 256 was S.1's figure from the CI cohort's reduced mesh |
//! | `[tacsim.harness].max_ledger_bytes` | `67108864` | 64 MiB the streamed ledger must stay under over an hour of arena time |
//! | `[tacsim.harness].max_cell_probes_per_pulse` | `32000` | **16b cut U.2** (doc 16 §2 W4): cells one pulse's line-of-sight traces may read — detection's and the mesh's together |
//! | `[tacsim.harness].max_jam_checks_per_pulse` | `2048` | **16b cut U.2** (doc 16 §9 item 4): index candidates one pulse's jammer scans may look at |
//! | `[tacsim.harness].max_rss_growth_kib` | `65536` | **16b cut U.2**: how far resident memory may grow over the harness's long run, KiB |
//! | `[tacsim.harness].max_frame_bytes` | `16384` | **16b cut U.2**: the largest encoded `arena.cockpit` keyframe the fan-out lane may see; the lane measures the opening with no enemy in sight, so a combat frame is unmeasured |
//! | `[tacsim.harness].max_snapshot_bytes` | `7340032` | **16b cut AF.2** (16c N16): the largest arena snapshot state the harness's 30-second lane may encode, bytes, asserted on both cohorts' `harness_arena_snapshot` line; measured before it was set — the authored lane's 4,198,651 bytes, three halves of it rounded up to the next MiB; the lane's own bound, kept out of the compared block |
//! | `[tacsim.harness].max_path_cells_per_pulse` | `48128` | **16b cut AI.1** (16c `B-84`): A star cells one wake's path searches expand, found or failed; sized on the four-seed measurement with the harness's 16 per cent spread |
//! | `[tacsim.harness].max_output_events_per_pulse` | `512` | **16b cut U.2**: the most output events one arena pulse may produce through the world's output path; sized against the fan-out lane's measured acquisition burst (24 panels + 11 contact-notice lines per pilot = 288), 512 so a 24-pilot burst has headroom — whoever moves it next must say why; that burst is teammates only, so an engaged pulse is unmeasured |
//! | `[tacsim.geology].key_dir` | absent | doc 20 SM.1: the geology key family's directory; absent, the mining layer reads disabled |
//! | `[tacsim.surface].registry_interlock` | `true` | doc 20 SM.10 (§9.4): the Registry interlock, phase 1's only claim enforcement; `false` is refused at boot (the lever is reserved for the fight, §17) |
//! | `[game].starter_area` | `true` | S1: apply the shipped starter area (the lot, the OOC Area, the Staff Lobby) at a world's first boot; static content, no LLM call; never re-applied |
//! | `[atlas].tile_max_cells` | `65536` | S1: the most cells one map tile holds; a larger footprint rasterizes coarser |
//! | `[atlas].nearest_room_cells` | `8` | S4: cells a pose may stand off every footprint and still resolve to the nearest room |
//! | `[atlas].window_margin_m` | `10` | S4: metres of ground a tactical window takes around its anchor rooms |
//! | `[atlas].window_max_cells` | `65536` | S4: the most cells one window holds before it coarsens to the next registered cell size |
//! | `[atlas].path_max_cells` | `4096` | S5: the most cells one path search visits before it gives up |
//! | `[atlas].pack_paths` | `[]` | S1: content pack files or directories, read **host-side** at boot like `[tacsim].pack_paths`; they go on the `@atlas/pack` shelf and nothing is applied by being listed |
//! | `[atlas].dwell_floor_s` | `30` | S2: real seconds a traveler stays at every checkpoint at least, so a stake-out can act |
//! | `[atlas].real_cap_s.<mode>` | `180` for `foot`, `car`, `transit`, `bus` | S2: real seconds a whole passage may take at most, per travel mode; a mode absent is uncapped |
//! | `[llm]` | `enabled = false` | M7 LLM layer (providers, routes, budgets); into `mush_llm::LlmConfig` |
//!
//! The `[web]` table deserializes straight into
//! `mush_net_web::WebConfig` (every knob it documents — TLS material,
//! rate limits, WS tuning — is available here). Its `features`/`panels`
//! fields are **overwritten at boot** from the module registry, so config
//! files should leave them unset.
//!
//! A missing file yields the defaults; a file that exists but does not parse
//! is a hard error (silently ignoring a typo'd config is worse than not
//! starting).

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
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
    /// An unknown top-level section looks like a mistyped known section.
    #[error("cannot load {path}: unknown section [{section}]; did you mean [{suggestion}]?")]
    MistypedSection {
        /// The config path.
        path: PathBuf,
        /// The unknown section.
        section: String,
        /// The nearest known section.
        suggestion: &'static str,
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
    /// `[ipc].listen` names the same host+port the store already occupies:
    /// the boundary listener would fail to bind, and the split would
    /// crash-loop under the gateway's relaunch guard.
    #[error(
        "[ipc].listen {ipc_listen:?} would bind port {port}, which \
         [store].redis_url {redis_url:?} already uses on the same host — the \
         gateway<->server boundary cannot share the store's port; give \
         [ipc].listen its own port (or, on Linux, a Unix socket path)"
    )]
    IpcStorePortCollision {
        /// The offending `[ipc].listen` value.
        ipc_listen: String,
        /// The `[store].redis_url` it collides with.
        redis_url: String,
        /// The shared TCP port.
        port: u16,
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
/// `pilot-chargen` job lifts the gate and moves it to `start_room`. Both rooms
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
    /// Per-instrument overrides, keyed by instrument code (`tacsim.credit`).
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

/// `[tacsim]` — the TacSim layer's own configuration (E7 cut 1, 10b §7).
///
/// Since 16b cut R-b the one game's block: the layer's packs and seeds, and, under
/// `[tacsim.economy]`, the game's economy wiring. `[rulesets]` left with the two
/// character rulesets.
///
/// The default is **`enabled = false`**: a server that has never heard of this section
/// is bit-identical to one built before E7 existed. The verbs still register and answer
/// "not enabled on this server" (E5c deviation 1).
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
    /// **S7 (doc 14 §5.4)** — the TacSim ground driver's cell size, centimetres:
    /// the windows its engagements are fought on. **Live:** one of the atlas's
    /// registered cell sizes, validated at the host.
    pub ground_cell_cm: u16,
    /// **Parsed, inert until E7 cut 8** — the space pulse period.
    pub space_pulse_ms: u32,
    /// `[tacsim.time]`.
    pub time: TacsimTimeCfg,
    /// `[tacsim.recovery]` — **E7 residue pass 1, R4**.
    pub recovery: TacsimRecoveryCfg,
    /// `[tacsim.economy]` — **16b cut R-b**: the single game's economy wiring.
    pub economy: TacsimEconomyCfg,
    /// `[tacsim.arena]` — **16b cut G**: the combat arena's clock, intent inbox
    /// and ledger bounds (doc 16 §3.1, §4).
    pub arena: TacsimArenaCfg,
    /// `[tacsim.match]` — **16b cut G**: the match lifecycle's slots and timeouts
    /// (doc 16 §3.3, §3.8). `match_` because `match` is a keyword; the TOML key is
    /// `match`.
    #[serde(rename = "match")]
    pub match_: TacsimMatchCfg,
    /// `[tacsim.command]` — **16b cut Y.2**: the span of control and the order
    /// rate a pilot's `ORDER` is held to (doc 16 §2 W8).
    pub command: TacsimCommandCfg,
    /// `[tacsim.minds]` — **17b cut M1b.1**: the unit minds' block, today the
    /// Compact floor (doc 17 §5, D-UM-17 as amended).
    pub minds: TacsimMindsCfg,
    /// `[tacsim.sensors]` — **16b cut G**: the sensor pass's cadence and its
    /// signature ceiling (doc 16 §3.6).
    pub sensors: TacsimSensorsCfg,
    /// `[tacsim.resolution]` — **16b cut G**: where the hit-resolution tables live
    /// (doc 16 §4).
    pub resolution: TacsimResolutionCfg,
    /// `[tacsim.harness]` — **16b cut G**: the scale harness's shape and its
    /// thresholds (doc 16 §1, acceptance A2).
    pub harness: TacsimHarnessCfg,
    /// `[tacsim.geology]` — **doc 20 cut SM.1**: where surface mining's geology
    /// key lives (§3.2, Q-20-12). Absent, the mining layer reads disabled.
    pub geology: TacsimGeologyCfg,
    /// `[tacsim.surface]` — **doc 20 cut SM.10**: the Registry interlock lever (§9.4; lead
    /// ruling 9). Always `true` in phase 1; `false` is refused at boot.
    pub surface: TacsimSurfaceCfg,
}

/// `[tacsim.surface]` — doc 20 cut SM.10 (§9.4; lead ruling 9, D16, OC13): the Registry
/// interlock, **phase 1's only claim enforcement until the fight exists** (§17). It defaults to
/// `true`, every shipped config states it, and the host refuses `false` at boot
/// (`mush_server::config_ext`) with a line naming doc 20 §9.4. Nothing in the game reads the
/// key: the lever is reserved for the fight, which is what will enforce a claim with it off.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimSurfaceCfg {
    /// Whether the Registry's interlock enforces claims (phase 1: always).
    pub registry_interlock: bool,
}

impl Default for TacsimSurfaceCfg {
    fn default() -> Self {
        Self {
            registry_interlock: true,
        }
    }
}

/// `[tacsim.geology]` — doc 20 cut SM.1 (§3.2, Q-20-12): the geology key family's directory.
///
/// ABSENT means the mining layer is disabled (lead ruling 3), unlike the three protocol families,
/// which default under `[server].data_dir`: a world that never chose a geology key never
/// provisions one, so a deployment cannot quietly mint a second key beside a restored store. The
/// shipped template names one (`__DATA_DIR__/geology-keys`). A key that will not load, or whose
/// fingerprint differs from the one the world pinned at its first keyed boot, disables the layer
/// and never refuses the boot (§16: "a lost key is a disabled layer, not a corrupted one").
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimGeologyCfg {
    /// The geology key family's private directory (one key, never rotated).
    pub key_dir: Option<PathBuf>,
}

/// `[tacsim.economy]` — what the composition root turns on for the one game's
/// economy (**16b cut R-b**, replacing `[wod.economy]` and `[shadowrun.economy]`).
///
/// `housing` keeps the tenancy title issuer (`system:tacsim.housing`) registered so
/// `@property/create` can mint; `market` registers the market's title issuer and
/// composes the market (with no catalog sources until the game authors some);
/// `instrument` is the settlement currency the game registers at first boot (doc
/// 11 §2: the credit, `cr`), at scale 2; `reserved_rooms` are extra rooms (`#N`)
/// that may never become property rooms, beside `[onboarding]`'s two.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimEconomyCfg {
    pub housing: bool,
    pub market: bool,
    pub instrument: String,
    pub reserved_rooms: Vec<u32>,
}

impl Default for TacsimEconomyCfg {
    fn default() -> Self {
        TacsimEconomyCfg {
            housing: true,
            market: false,
            instrument: "tacsim.credit".to_string(),
            reserved_rooms: Vec::new(),
        }
    }
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

/// `[atlas]` — the spatial layer's knobs (S1, 14-spatial-implementation-plan §3.1,
/// §7). Every field defaults; like `[tacsim]` and `[npc]` the block is
/// `deny_unknown_fields`, so a mistyped key refuses the boot rather than taking a
/// default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AtlasCfg {
    /// The most cells one tile may hold (256×256 by default; a larger footprint
    /// rasterizes coarser rather than refusing).
    pub tile_max_cells: u32,
    /// How far, in cells, a point may be from a room and still count as in it
    /// (parsed now; S4's windows read it).
    pub nearest_room_cells: u32,
    /// Content pack files, or directories searched for `*.json`, read **host-side**
    /// at boot exactly as `[tacsim].pack_paths` are: an unreadable, oversized or
    /// mis-digested pack is a hard boot error naming the file. Loaded packs go on
    /// the shelf `@atlas/pack list|plan|apply` works from; nothing is applied by
    /// being listed here.
    pub pack_paths: Vec<String>,
    /// S2: real seconds a traveler stays at every checkpoint at least, so a stake-out
    /// can act (default 30).
    pub dwell_floor_s: u32,
    /// S2: real seconds a whole passage may take at most, per travel mode (default
    /// 180 for `foot`, `car`, `transit` and `bus`; a mode absent here is uncapped —
    /// `ship` and `torch` campaigns set their own).
    pub real_cap_s: BTreeMap<String, u32>,
    /// Metres of ground a tactical window takes around its anchor rooms (plan §7
    /// `window_margin_m`; S4).
    pub window_margin_m: u32,
    /// The most cells one window may hold before it coarsens to the next
    /// registered cell size (plan §7 `window_max_cells`; S4).
    pub window_max_cells: u32,
    /// The most cells one path search visits before it gives up (plan §7
    /// `path_max_cells`; S5).
    pub path_max_cells: u32,
}

impl Default for AtlasCfg {
    fn default() -> Self {
        AtlasCfg {
            tile_max_cells: 65_536,
            nearest_room_cells: 8,
            pack_paths: Vec::new(),
            dwell_floor_s: 30,
            real_cap_s: ["foot", "car", "transit", "bus"]
                .into_iter()
                .map(|m| (m.to_string(), 180))
                .collect(),
            window_margin_m: 10,
            window_max_cells: 65_536,
            path_max_cells: 4_096,
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
            ground_cell_cm: 1_000,
            space_pulse_ms: 6_000,
            time: TacsimTimeCfg::default(),
            recovery: TacsimRecoveryCfg::default(),
            economy: TacsimEconomyCfg::default(),
            arena: TacsimArenaCfg::default(),
            match_: TacsimMatchCfg::default(),
            command: TacsimCommandCfg::default(),
            minds: TacsimMindsCfg::default(),
            sensors: TacsimSensorsCfg::default(),
            resolution: TacsimResolutionCfg::default(),
            harness: TacsimHarnessCfg::default(),
            geology: TacsimGeologyCfg::default(),
            surface: TacsimSurfaceCfg::default(),
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
/// `tacsim` at all. It lives under `[tacsim.recovery]` because that is where
/// the campaign layer's operator-facing knobs live, not because it depends on the
/// layer. A deployment that wants no loaner provider sets **this** block's `enabled`
/// to `false`, which is already the default.
///
/// **The `insurance` word governs debt-versus-writeoff and nothing else**
/// (independent verify fold, C6/B2; owner decision F4(i)(b)). It used to gate the
/// whole cooldown, so on every default deployment — `"approved_operations"` is the
/// default here, in `tacsim` and in `mush-campaign` — `cooldown_days` was a
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
    /// **S7 (doc 10 T17)** — `[tacsim.time.ephemeris]`, the campaign's epoch and the
    /// rate its sky moves at. **Live:** the TacSim layer pins every corridor's
    /// distance from it at load.
    pub ephemeris: EphemerisCfg,
}

/// `[tacsim.time.ephemeris]` — the constant sky (doc 10 T17 as amended by doc 18
/// §4.8; 14-spatial-implementation-plan §3.8). `rate = 0` is the default (doc 18
/// D-18-42 rules the constant sky for the slice); `deny_unknown_fields` for the same
/// reason as [`TacsimTimeCfg`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EphemerisCfg {
    /// The epoch a body without its own `epoch_ic` has its phase stated at, IC
    /// seconds on the game calendar (`mush_core::CivilDate`: 365-day years, year 0
    /// at IC 0).
    pub epoch: i64,
    /// Permille of the real sky's motion per IC second; 0 holds every body at its
    /// authored phase (the constant sky).
    pub rate: u32,
}

// ---- 16b cut G — the combat arena's five blocks -----------------------------
//
// Doc 16 §7's "Configuration" paragraph, with the values of 16b §9's constants
// table. Every one of the five is `deny_unknown_fields` on [`TacsimCfg`]'s own
// precedent, and every one is **parsed and inert** in cut G: no field below has a
// reader yet, so on `[tacsim.time]`'s rule (`config_ext::tacsim_config`'s comment)
// nothing here validates a *value* — the cut that reads a field is the cut that
// validates it (A for the arena, T for the match, S for sensors, W for
// resolution, H for the harness). What cut G does enforce is *shape*: a mistyped
// key refuses the boot rather than leaving the operator a period, a bound or a
// threshold they believe is in force and is not.

/// `[tacsim.arena]` — the combat arena's clock, its intent inbox and its ledger
/// bounds (**16b cut G**; doc 16 §3.1 "Time, tempo and authority" and §4).
///
/// One arena runs one match on one map as the single writer for that match's
/// facts. Everything here bounds that task: how often it steps, how much wall
/// time one pulse may absorb, how much a pilot may queue, and how much black box
/// the match may leave behind.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimArenaCfg {
    /// `[tacsim.arena.tempo]` — the adaptive pulse periods and their holds.
    pub tempo: TacsimTempoCfg,
    /// Periods of wall time one pulse may absorb after a stall before the arena
    /// records `lagged` and skips the idle time rather than expanding `dt` to hide
    /// it (doc 16 §3.1 "Clock and lag"; doc 06 §6.4).
    pub max_catch_up_steps: u32,
    /// **Arena milliseconds a unit holds against another unit in its way, keeping
    /// its speed, before it stops and says so** (**16b cut E.3**; doc 16 §3.4 step
    /// 4).
    ///
    /// A refused crossing stops a unit dead, which is right for ground and wrong
    /// for traffic: a follower that reaches the cell its leader has not vacated
    /// yet was stopped and needed a dozen seconds to rebuild way, at the ford,
    /// every time. It now holds at the boundary keeping its speed, journalling
    /// nothing and telling nobody, and only a hold on one cell that outlasts this
    /// becomes the stop. `0` is the rule as it stood before the cut. Impassable
    /// ground, the window's edge and a slope are untouched.
    pub occupied_stop_ms: u32,
    /// The least **arena** time between two deliberation requests for one squad,
    /// milliseconds (**16b cut N.2**; doc 16 §6 "Deliberation": *event-driven with a
    /// floor of `command.deliberate_min_ms` (30,000)*).
    ///
    /// The trigger is the squad commander's contact set changing class or count, or
    /// an order expiring unrenewed; this is the floor under it, and it is the whole
    /// of what stops a firefight from asking a provider for a new order every sensor
    /// pass. It is enforced inside the pulse, in arena time, so the same scripted
    /// match makes the same number of requests at every tempo (D-16-2).
    pub deliberate_min_ms: u32,
    /// **The most machines one squad holds** (**16b cut Z.1**; 16c N6, *lances*;
    /// doc 16 §6 "Deliberation").
    ///
    /// A side's machines (unit minds since 17b cut M4.3; the reflex
    /// controller's units before it) are dealt, in unit-id order, into `ceil(n / size)`
    /// squads whose sizes differ by at most one, so five machines at four are a
    /// three and a two and never a four and a one. Both groupings of a match —
    /// the record's and the arena's — read it, so they cannot disagree. `1` to
    /// `16`, refused by name outside it.
    pub squad_size: u32,
    /// **The most machines of one class a side fields**, by class word
    /// (**16b cut AE.3**; doc 16 §2 W2: *no drone swarms;
    /// `arena.max_units_by_class`*).
    ///
    /// Written as an inline table under `[tacsim.arena]`
    /// (`max_units_by_class = { drone = 1 }`) or as its own
    /// `[tacsim.arena.max_units_by_class]` header; both parse to this map. Per
    /// side, not per map; `0` means the class cannot be fielded; a class not
    /// named has no cap. This crate cannot see tacsim's class words, so it
    /// checks nothing here: `mush_server::config_ext` refuses a word that is not
    /// `armour`, `walker` or `drone`, lays the operator's entries **over** the
    /// compiled default (so a table naming only `walker` leaves the drone at
    /// `1`, because a written table replaces this `serde(default)` map whole),
    /// and `tacsim::config::ArenaConfig::validate` bounds each value `0` to `24`.
    pub max_units_by_class: BTreeMap<String, u32>,
    /// **Arena milliseconds between two snapshots of a running match**
    /// (**16b cut AF.1**; 16c N16; doc 16 §3.1 "Durability"), taken by the arena
    /// task after the pulse that crossed the boundary and written by the world
    /// on the arena writer lane (16b cut AF.2 reads it). `0` turns snapshots
    /// off, and a restart then voids every running match by name (16b cut
    /// AF.3); otherwise `1000` to `60000`, refused by name outside it by
    /// `tacsim::config::ArenaConfig::validate`. Nothing reads it in AF.1: it
    /// lands with the snapshot's format because the snapshot carries the
    /// arena's frozen block whole.
    pub snapshot_ms: u32,
    /// Encoded recovery-document ceiling including the escaped state string.
    /// 16–480 MiB; admission reserves future growth. Not a normal allocation
    /// or the harness report-only size threshold. Below Redis RESP 512 MiB.
    pub snapshot_max_bytes: u64,
    /// Per-match private input recording cap (4–128 MiB).
    pub tape_max_bytes: u64,
    /// Aggregate resident retained recordings (4–1024 MiB).
    pub tape_retained_max_bytes: u64,
    /// `[tacsim.arena.intents]` — the per-unit intent inbox bound.
    pub intents: TacsimIntentsCfg,
    /// `[tacsim.arena.ledger]` — the match ledger's summary cadence and its two
    /// hard bounds.
    pub ledger: TacsimLedgerCfg,
    /// `[tacsim.arena.reflex]` — the machines' cadence, the motor's path
    /// budgets and the doctrine figures (the reflex controller's until 17b cut
    /// M4.6 deleted it; the block keeps its name).
    pub reflex: TacsimReflexCfg,
}

impl Default for TacsimArenaCfg {
    fn default() -> Self {
        TacsimArenaCfg {
            tempo: TacsimTempoCfg::default(),
            max_catch_up_steps: 4,
            occupied_stop_ms: 2_000,
            deliberate_min_ms: 30_000,
            squad_size: 4,
            max_units_by_class: BTreeMap::from([("drone".to_owned(), 1)]),
            // 16b cut AF.1: every five seconds of arena time (16c N16).
            snapshot_ms: 5_000,
            snapshot_max_bytes: 480 * 1024 * 1024,
            tape_max_bytes: 64 * 1024 * 1024,
            tape_retained_max_bytes: 128 * 1024 * 1024,
            intents: TacsimIntentsCfg::default(),
            ledger: TacsimLedgerCfg::default(),
            reflex: TacsimReflexCfg::default(),
        }
    }
}

/// `[tacsim.arena.reflex]` — the reflex controller's block (**16b cut N.1**;
/// doc 16 §6), named for the deterministic state machine that flew a tank
/// nobody is sitting in: the AI fill of a lobby that fields fewer pilots than
/// tanks, the harness's field of 300, and the fallback a seat released past the
/// linkdead grace hands its tank to.
///
/// **17b cut M4.6 deleted the reflex controller; the block keeps its name**
/// (every key, default and validation sentence unchanged), because what it
/// holds outlived it: `decide_ms` is the motor's tick cadence, the path budgets
/// are the motor's, and the doctrine figures feed the stock card Standard
/// Doctrine thinks under on a unit mind (which has flown those tanks since 17b
/// cut M4.3) — `tacsim::config::ReflexConfig`'s own paragraph. A machine
/// submits the **same intents a pilot's verbs stage**, so nothing here can make
/// a unit do what a player could not.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimReflexCfg {
    /// Arena milliseconds between one driven hull's motor ticks (one AI unit's
    /// decisions until 17b cut M4.6 deleted the reflex controller), staggered by
    /// unit id. A cadence of **arena** time, so the motor runs the same number
    /// of times per arena second at every tempo (doc 16 §3.1, D-16-2).
    pub decide_ms: u32,
    /// The most cells one bounded A\* may expand before it gives up and the
    /// `unreachable` edge fires. This is `[atlas].path_max_cells` **raised for the
    /// arena** (16b §9): the atlas searches a room neighbourhood at 4,096 and the
    /// arena searches a 256 by 256 window, which is 65,536 cells.
    pub path_max_cells: u32,
    /// The most path searches the motor may run in one `decide_ms` window of
    /// arena time; a unit refused by the budget keeps the path it has. A budget
    /// per *pulse* would bite differently at each tempo, which D-16-2 forbids.
    pub max_path_queries: u32,
    /// The least arena time between two path searches by one unit.
    pub repath_ms: u32,
    /// Armour plus internals, per mille of the authored totals, below which a
    /// unit breaks off toward its entry zone.
    pub withdraw_permille: u32,
    /// The utility weight on range to a candidate hostile; lower scores win.
    pub weight_range: u32,
    /// The utility weight on the facet a candidate presents.
    pub weight_facet: u32,
    /// The utility weight on the chooser's own health — the same addend for every
    /// candidate, so it orders nothing and is what the `Withdraw` edge reads.
    pub weight_health: u32,
    /// The fraction of a weapon's reach `Engage` holds, per mille.
    pub engage_band_permille: u32,
    /// **How long a withdrawn unit waits with nothing hostile on its board before
    /// it comes back into the fight** (16b cut Q.7), arena milliseconds.
    ///
    /// `Withdraw` used to be terminal — a hurt AI tank parked in its entry zone
    /// and the match ran to `[tacsim.match].max_duration_ms`. It rallies once
    /// now, and this is the lull that buys it. At or above `decide_ms`, or the
    /// boot is refused.
    pub rally_ms: u32,
    /// **Armour plus internals, per mille, below which a machine on its way home
    /// will not turn and fight even something close** (16b cut E.1; doc 16 §6).
    ///
    /// A `Withdraw` has two stands. **En route** it turns only for a hostile
    /// inside its `close` band and only while it is above this figure; **once
    /// home** it shoots back at whatever comes inside its firing ceiling, at any
    /// health, because a machine that has arrived has nowhere further to run.
    ///
    /// `0` to `1000`, **and not above `withdraw_permille`**, or the boot is
    /// refused naming both keys: a machine only runs home below the second, so a
    /// floor above it is a doctrine branch nothing could ever be in.
    pub stand_min_permille: u32,
    /// **How long a machine keeps driving to where it last saw its target**
    /// (16b cut E.4; 16c `B-04`), arena milliseconds.
    ///
    /// A contact expires after three sensor passes, and the `lost` edge used to
    /// drop the machine straight back onto a waypoint walk that may lead away
    /// from the tank that has just driven behind a tree line. It goes and looks
    /// first now.
    ///
    /// `0` is off — the pre-E.4 rule exactly — and anything else is `decide_ms`
    /// to `600000`, or the boot is refused naming the key. The plot is one
    /// decide tick old when the machine first reads it, so at `decide_ms`
    /// exactly the search lasts that one tick.
    pub search_ms: u32,
    /// **How long a patrolling machine that holds nothing hostile waits before it
    /// crosses to the other side's entry zone to look** (16b cut E.4; 16c
    /// `B-04`), arena milliseconds.
    ///
    /// The waypoint walk is a fixed cycle per unit id, so two sides' loops need
    /// never meet, and a withdrawn machine parks in its own entry zone, which no
    /// loop passes through. That is the match that goes quiet for ever and never
    /// ends. The other side's entry zone is the one place a beaten enemy is
    /// certain to be, and it is map knowledge — the arena's own zones, no unit's
    /// pose.
    ///
    /// `0` is off, and anything else is at least `decide_ms`, or the boot is
    /// refused naming the key.
    pub hunt_after_ms: u32,
}

impl Default for TacsimReflexCfg {
    fn default() -> Self {
        TacsimReflexCfg {
            decide_ms: 1_000,
            path_max_cells: 65_536,
            max_path_queries: 64,
            repath_ms: 4_000,
            withdraw_permille: 350,
            weight_range: 100,
            weight_facet: 40,
            weight_health: 30,
            engage_band_permille: 700,
            // 16b cut Q.7: twenty seconds of quiet.
            rally_ms: 20_000,
            // 16b cut E.1: half of `withdraw_permille`.
            stand_min_permille: 175,
            // 16b cut E.4: a minute of driving to the last plot, and a minute and
            // a half of nothing at all before a machine goes and looks.
            search_ms: 60_000,
            hunt_after_ms: 90_000,
        }
    }
}

/// `[tacsim.arena.tempo]` — the adaptive tempo (**16b cut G**; doc 16 §3.1's
/// tempo table and its "Rate independence" and "Failure" paragraphs).
///
/// The tempo decides only how often the arena steps and how its output is
/// batched; it never decides an outcome (D-16-2). Every value is arena
/// milliseconds except `alert_range_cm`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimTempoCfg {
    /// The `Quiet` period: the arena's default step when no unit holds a hostile
    /// contact, no impact is pending, no fire intent is queued and no controller
    /// is approaching.
    pub quiet_ms: u32,
    /// The `Alert` period: a contact is held inside `alert_range_cm`, a fire
    /// intent is queued, or a controller has entered `Approach`.
    pub alert_ms: u32,
    /// The `Engaged` period: a shot is fired, an impact is scheduled, or hostile
    /// units are inside weapons range of each other. The harness's p99 ceiling is
    /// measured in this tempo.
    pub engaged_ms: u32,
    /// Milliseconds with none of the `Alert` triggers before the arena falls back
    /// to `Quiet`.
    pub alert_hold_ms: u32,
    /// Milliseconds since the last shot, impact or in-range pair before the arena
    /// leaves `Engaged`.
    pub engaged_hold_ms: u32,
    /// The range, centimetres, under which a held contact raises the tempo to
    /// `Alert`. **Zero means derived**: at match start the arena takes **the
    /// lesser of** twice the longest weapon range in the match **and the window's
    /// own diagonal** (16b §9's `tempo.alert_range_cm` row; the cap is 16b cut
    /// E.1's).
    pub alert_range_cm: u32,
    /// The longest substep a pulse is cut into. Doc 16 §3.1: a pulse is a
    /// sequence of substeps of `min(substep_max_ms, (cell_cm - 1) × 1000 /
    /// max_speed_cms)`, floored at 50 ms, so one substep's travel — the motion
    /// carry (16b cut Q.6) included, because the carry is added *inside* that
    /// one division — is at most `cell_cm - 1`, and no unit advances more than
    /// one cell in a substep and the outcome is the same at every period.
    pub substep_max_ms: u32,
    /// Milliseconds one pulse may take before the watchdog voids the match, the
    /// same way a panic does (doc 16 §3.1 "Failure").
    pub stall_ms: u32,
}

impl Default for TacsimTempoCfg {
    fn default() -> Self {
        TacsimTempoCfg {
            quiet_ms: 5_000,
            alert_ms: 1_000,
            engaged_ms: 500,
            alert_hold_ms: 30_000,
            engaged_hold_ms: 10_000,
            // 0 = derived at match start as the lesser of twice the longest
            // weapon range in the match and the window's own diagonal (16b §9;
            // the cap is 16b cut E.1's).
            alert_range_cm: 0,
            substep_max_ms: 250,
            stall_ms: 2_000,
        }
    }
}

/// `[tacsim.arena.intents]` — the rate a seat's discrete orders are held to
/// (**16b cut G**; doc 16 §3.1, pulse phase 2; a rate since **16b cut M.1**).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimIntentsCfg {
    /// **The discrete orders (`LOCK`, `FIRE`, `STARTUP`) one seat may give a unit
    /// per arena second; also the size, per unit, of the world side's inbox and
    /// pre-pulse buffer** (16b cut M.1, `T-82`). Continuous controls are
    /// latest-wins and do not count.
    pub max_pending_per_unit: u32,
}

impl Default for TacsimIntentsCfg {
    fn default() -> Self {
        TacsimIntentsCfg {
            max_pending_per_unit: 8,
        }
    }
}

/// `[tacsim.arena.ledger]` — the match ledger's cadence and its two hard bounds
/// (**16b cut G**; doc 16 §4 "Ledger events", §3.1's egress paragraph).
///
/// The ledger is the match's black box: JSON lines chunked at 16 KiB and streamed
/// to the world's own writer lane while the match runs, so arena memory stays
/// flat and the record survives a crash that voids the match.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimLedgerCfg {
    /// Arena milliseconds between `position_summary` events — every unit, in
    /// every tempo. There is no per-pulse `moved` event; this is the position
    /// record.
    pub summary_ms: u32,
    /// The most bytes one match's ledger may reach (64 MiB): at 90% summaries
    /// stop, and at 100% the match is stopped with `Timeout`.
    pub max_bytes: u64,
    /// The most 16 KiB chunks that may sit unflushed in the arena's bounded
    /// egress ring. Past it the world cannot keep up and the match is stopped
    /// with `Void`.
    pub max_unflushed_chunks: u32,
}

impl Default for TacsimLedgerCfg {
    fn default() -> Self {
        TacsimLedgerCfg {
            summary_ms: 5_000,
            // 64 MiB.
            max_bytes: 67_108_864,
            max_unflushed_chunks: 256,
        }
    }
}

/// `[tacsim.match]` — the match lifecycle's slots and timeouts (**16b cut G**;
/// doc 16 §3.3 "Startup", §3.8 "Lifecycle", §4).
///
/// The TOML key is `match`; the Rust field is `match_` because `match` is a
/// keyword ([`TacsimCfg::match_`] carries the `serde(rename)`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimMatchCfg {
    /// Milliseconds `STARTUP` takes to carry a unit's power from `starting` to
    /// `online`, scheduled as a due event; until then motion and fire intents
    /// refuse (doc 16 §3.3).
    pub startup_ms: u32,
    /// Milliseconds `Boarding` waits for every claimed slot to be seated before
    /// unseated slots are dropped and the match runs anyway — an empty side
    /// aborts (doc 16 §3.8).
    pub board_timeout_ms: u32,
    /// The longest a match may run before it ends as a timeout (doc 16 §3.8, §4).
    pub max_duration_ms: u32,
    /// Pilot slots one side may claim: doc 16 §1's 12 v 12, and the floor is two
    /// per side (doc 16 §3.8's `Selection`).
    pub max_slots_per_side: u32,
    /// **Whether a match fields the tanks of the pods nobody claimed**, under the
    /// reflex controller (**16b cut N.1**; doc 16 §6's *the AI fill of a later
    /// lobby*) — flown by Standard Doctrine on a unit mind since 17b cut M4.3.
    ///
    /// `false` is the default and it is not a placeholder. Doc 16 §6 and 16b §9's
    /// cut N row both put the lobby's AI fill in *a later lobby*, and doc 16 §1's
    /// A1 — the story test, on the production composition — is a **two-tank**
    /// story: a default-on fill would make every `SIM START` a four-a-side match
    /// and rewrite that acceptance from a cut that does not own it. So cut N.1
    /// ships the whole mechanism — the attribute, the check-out rule that reads it
    /// (`Reflex` for a unit with no seated pilot), the controller itself — and this
    /// switch, and the cut that changes what a `SIM START` fields is the cut that
    /// rewrites A1.
    ///
    /// With it on, every pod of a side that has at least one pilot goes to the
    /// arena: slot *n*'s tank is claimed or it is the machine's.
    pub ai_fill: bool,
    /// **How long a running match every pilot has left runs on before the lobby
    /// ends it** (**16b cut D.3**, `B-33`), in world milliseconds; `0` turns it
    /// off entirely.
    ///
    /// Doc 16 §11's N.1 amendment makes a released seat one-way: a pilot who
    /// `EXIT`s hands their tank to the machine (a unit mind since 17b cut M4.3;
    /// the reflex controller before it) and cannot take it back.
    /// So a match every human has walked out of has nobody left who can end it,
    /// and before this key it ran to `max_duration_ms` — a quarter-hour (an hour
    /// before AP.2) in which no second crew could claim a slot. The lobby stamps
    /// the moment the last slot goes `left` and its own 1 s tick sends the stop
    /// once this has passed.
    ///
    /// A staff `@arena/start` match has no slots and is therefore never
    /// unattended; a destroyed pilot who stays in the pod is attending.
    pub unattended_ms: u32,
    /// **How long after a dropped line a pilot waits before `BOARD` takes the
    /// tank back** (**16b cut Y.3**; 16c `N4`; doc 07 §1.3 "Reclaiming
    /// control"), in world milliseconds; `0` to `600000`.
    ///
    /// A drop — the linkdead grace running out, or a `QUIT` — hands the tank to
    /// its sealed unit mind (the reflex controller until 17b cut M4.3) and the
    /// board reads `flown by its mind` (`flown by doctrine` until 17b cut M5.3);
    /// `BOARD` in the running match takes it back, where it stands, at a new
    /// epoch.
    /// This floor closes flapping and gives the arena a pulse to process the
    /// release before the reclaim. `EXIT` is still one-way whatever this says.
    pub reclaim_min_ms: u32,
}

impl Default for TacsimMatchCfg {
    fn default() -> Self {
        TacsimMatchCfg {
            startup_ms: 10_000,
            board_timeout_ms: 120_000,
            // Fifteen minutes (16c Q47, the owner's ruling of 2026-10-02): the arena is an explicit OOC proving ground.
            max_duration_ms: 900_000,
            max_slots_per_side: 12,
            ai_fill: false,
            // 16b cut D.3: five minutes.
            unattended_ms: 300_000,
            // 16b cut Y.3: five seconds.
            reclaim_min_ms: 5_000,
        }
    }
}

/// `[tacsim.command]` — the span of control and the order rate (**16b cut
/// Y.2**; doc 16 §2 W8, *the Compact caps span of control and needs a human
/// trigger*).
///
/// Both are held at the world, where `ORDER` is admitted, and neither reaches
/// the arena: a refused order changes no outcome. The values are checked by
/// `tacsim::config::CommandConfig::validate`, reached through
/// `mush_server::config_ext::tacsim_config` whether or not the layer is on.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimCommandCfg {
    /// The most squads one commander may hold a **live** order on at once;
    /// replacing a squad's own live order never counts against it. `1` to `32`
    /// (a side's most squads).
    pub max_span: u32,
    /// The most `ORDER`s one commander may give in sixty world seconds; one
    /// `ORDER ALL` is one. `1` to `600`.
    pub orders_per_minute: u32,
}

impl Default for TacsimCommandCfg {
    fn default() -> Self {
        TacsimCommandCfg {
            max_span: 4,
            orders_per_minute: 12,
        }
    }
}

/// `[tacsim.minds]` — the unit minds' block (**17b cut M1b.1**; doc 17 §5 "The
/// law", D-UM-17 as amended 2026-10-04).
///
/// The **Compact floor** (the least chance to hit, per mille, a lawful
/// condition slot may name; captured at check-out, so a reload never reaches a
/// running or a resumed match), the class table, the pool, the think pool's
/// workers, the minds lane's CPU lens figure and (17b cut M4.3) the class the
/// AI fill is sealed at. Checked by `tacsim::config::MindsConfig::validate`,
/// reached through `mush_server::config_ext::tacsim_config` whether or not the
/// layer is on.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimMindsCfg {
    /// Independent proving worker threads; 1–8, separate from live minds.
    pub proving_workers: usize,
    /// Maximum queued private jobs; 1–128.
    pub proving_queued: usize,
    /// Shared account allowance per UTC day, in simulated unit-minutes.
    pub proving_daily_unit_minutes: u64,
    /// Retained completed proving runs per character; 1–100.
    pub proving_completed_per_character: usize,
    /// Retained proving bytes per character; 16–256 MiB.
    pub proving_bytes_per_character: u64,
    /// Private bench idle expiry; 60 seconds through 30 minutes.
    pub bench_idle_ms: u64,
    /// Global bench session limit; each account may hold one.
    pub bench_sessions: usize,
    /// Global retained bench state cap; 16–512 MiB.
    pub bench_state_bytes: u64,
    /// Read-only expression ceiling including fees; 1–500.
    pub bench_eval_ops: u64,
    /// Maximum encoded expression result; 64–4096 bytes.
    pub bench_eval_result_bytes: usize,
    /// The Compact floor, per mille. `10` to `950` in steps of `10` (17b cut
    /// M3.3; above `100` every Line-carded check-out is refused, and 17b cut
    /// M5.1's `SIM MIND` refuses a preset under it at the choice).
    pub odds_floor_permille: u32,
    /// Default AI fill ceiling when the lead has selected none, by its word
    /// — `0`, `I`, `II` or `III`; pilot minds use `max_class` independently.
    /// Parsed and refused by name in
    /// `mush_server::config_ext::tacsim_config`. Stock `I`.
    pub fill_class: String,
    /// Operator ceiling for every mind, independently of the AI-fill default.
    /// Class word `0`, `I`, `II` or `III`; stock III.
    pub max_class: String,
    /// **17b cut M3.1** (doc 17 §9, D-UM-35): `[tacsim.minds.classes.<class>]`,
    /// the Compact class table's overrides — the class word (`0`, `I`, `II`,
    /// `III`) to any of its eight figures (`think_ms`, `ops`, `rows`,
    /// `odds_rows`, `slots`, `memory_keys`, `line_to`, `profiles`). Kept as
    /// the table the file wrote and decoded ONCE, by
    /// `tacsim::arena::core::ClassTable`'s own wire, in
    /// `mush_server::config_ext::tacsim_config`: an absent class or figure
    /// reads the stock row, an unknown word refuses the boot by name, and
    /// `MindsConfig::validate` holds every figure.
    pub classes: serde_json::Value,
    /// **17b cut M3.5** (doc 17 §9, D-UM-36): the compute pool's base, ops a
    /// sim second — a task's allotment is `max(floor, base − per_unit ×
    /// units)`. At least 1.
    pub pool_base_ops_s: u64,
    /// The pool's charge per unit on the task, ops a sim second. At least 1.
    pub pool_per_unit_ops_s: u64,
    /// The pool's floor, ops a sim second. At least 1, at most the base.
    pub pool_floor_ops_s: u64,
    /// **17b cut M4.1** (doc 17 §6 "Execution policy", D-UM-22): the think
    /// pool's worker threads — the process-wide pool every sim task sends a
    /// pulse's collected thinks to, sized once per process from the first
    /// task's read. `1` to `32`; stock `2`. Never carried by a snapshot.
    pub workers: u32,
    /// **17b cut M4.2** (doc 17 §18): the mind CPU lens, milliseconds per
    /// 500 ms window — the reference figure the minds lane prints beside the
    /// lens it asserts (computed from the pool and the host's measured ops
    /// per microsecond); never a throttle, never read by a match. At least
    /// `1`; stock `5`.
    pub cpu_lens_ms: u32,
}

impl Default for TacsimMindsCfg {
    fn default() -> Self {
        TacsimMindsCfg {
            proving_workers: 1,
            proving_queued: 32,
            proving_daily_unit_minutes: 2_000,
            proving_completed_per_character: 20,
            proving_bytes_per_character: 64 * 1024 * 1024,
            bench_idle_ms: 30 * 60 * 1_000,
            bench_sessions: 64,
            bench_state_bytes: 128 * 1024 * 1024,
            bench_eval_ops: 500,
            bench_eval_result_bytes: 4_096,
            odds_floor_permille: 100,
            fill_class: "I".to_owned(),
            max_class: "III".to_owned(),
            classes: serde_json::Value::Object(serde_json::Map::new()),
            pool_base_ops_s: 500_000,
            pool_per_unit_ops_s: 1_000,
            pool_floor_ops_s: 120_000,
            workers: 2,
            cpu_lens_ms: 5,
        }
    }
}

/// `[tacsim.sensors]` — the sensor pass's cadence and its signature ceiling
/// (**16b cut G**; doc 16 §3.6 "Perception").
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimSensorsCfg {
    /// Arena milliseconds between one unit's sensor passes, staggered by unit id
    /// and run as a due event at its own timestamp.
    pub pass_ms: u32,
    /// The ceiling on a target's signature factor — the multiplier a class and
    /// posture may apply to a sensor's range, which also bounds the candidate
    /// cull radius (doc 16 §3.6 steps 1 and 3).
    ///
    /// **The one float in this tree, and it never reaches the engine.** Doc 16
    /// §3.1's determinism rule is that authoritative arena state is integer, so
    /// `mush_server::config_ext::tacsim_config` converts this once into per-mille
    /// and hands the engine a `u32`; that conversion **is** a read, so a negative,
    /// `NaN` or absurd factor is a hard boot error naming this key.
    ///
    /// **At least 1.0, and 1.0 by default** (16b cut M.4; 16c `B-55`, `T-26`):
    /// the pass scales its candidate cull by this, so a factor below 1.0 draws a
    /// cull circle shorter than the range test and silently drops contacts the
    /// sensor can see — refused by name at boot. No shipped signature row exceeds
    /// 1.0, so 1.0 is the exact cull; the 1.5 it replaced was a circle 2.25 times
    /// the area it needed.
    pub max_signature_factor: f64,
    /// **Passes a contact may go unrefreshed before it is lost** (16b cut S.1;
    /// doc 16 §3.6 step 4). The time-to-live is `pass_ms × lost_after_passes`, so
    /// the band is stated in passes and follows the cadence rather than drifting
    /// from it when `pass_ms` moves. Zero is read as one.
    pub lost_after_passes: u32,
    /// **Name a contact by team and centre slot at detection** (16b cut Q.1),
    /// rather than by a provisional `X<n>` until it has been classified.
    ///
    /// Off, which is doc 16 W5 and §5: identification is earned, and a frame
    /// names no unit it has not told apart. On, a pilot reads `BA`
    /// `classify_after_ms` before the rule allows, so the other side's tanks are
    /// legible off an unresolved return.
    pub designate_on_detection: bool,
    /// **At or above this fraction of what it holds whole, a section reads
    /// `green` on `SCAN`** (16b cut Q.2; doc 16 §3.6), per mille.
    ///
    /// The three keys below are the numbers behind the gunner's
    /// green/yellow/red/destroyed reading. They are configuration rather than a
    /// Rust table so the owner can tune them with a server-child restart and no
    /// build, and rather than shipped content because shipped content is compiled
    /// into the binary and an operator pack cannot override a shipped id. The
    /// engine reads each `.min(1000)`.
    pub condition_green_permille: u32,
    /// **At or above this fraction, a section reads `yellow`** (16b cut Q.2), per
    /// mille; below it, `red`.
    pub condition_yellow_permille: u32,
    /// **How near a target has to be before a sensor pass reads its bands at
    /// all** (16b cut Q.2; doc 16 §2 W5), as a fraction of the effective reach the
    /// range test just used — signature, thermal and jamming terms included. A
    /// close look, so a per-section reading is not free at the edge of detection.
    pub condition_reach_permille: u32,
}

impl Default for TacsimSensorsCfg {
    fn default() -> Self {
        TacsimSensorsCfg {
            pass_ms: 1_000,
            max_signature_factor: 1.0,
            lost_after_passes: 3,
            designate_on_detection: false,
            condition_green_permille: 667,
            condition_yellow_permille: 334,
            condition_reach_permille: 500,
        }
    }
}

/// `[tacsim.resolution]` — where the hit-resolution tables live (**16b cut G**;
/// doc 16 §4's `resolution.json` row).
///
/// The tables are the hit curve, the error terms, `p_min` and `p_max`: the
/// per-mille no-dice resolution of D-16-9. **Parsed and inert here** — cut W is
/// the cut that reads a table and therefore the cut that validates one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimResolutionCfg {
    /// Resolution-table files, or directories searched for them. Empty — the
    /// default — is the game's own shipped content, which cut W wires.
    pub paths: Vec<String>,
}

/// `[tacsim.harness]` — the scale harness's shape and its thresholds (**16b cut
/// G**; doc 16 §1's acceptance A2).
///
/// A2 is *300 units placed and moving, 24 of them fighting, 150 weapons on their
/// cadences, one sensor pass per unit per second of arena time, at least 12
/// connected fake sessions, on one core*, with a per-pulse p99 under 25 ms in
/// `Engaged`. These are that acceptance's numbers, in config so the harness runs
/// at the configuration maxima rather than at constants compiled into it.
///
/// The weapon count is **not** a key (16b cut U.3, 16c `T-37`): the harness
/// fields the catalog armed as authored, so the cohort decides what is armed,
/// and the `weapons` key it once carried was read by no lane.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TacsimHarnessCfg {
    /// Units the harness places and moves on the arena window.
    pub units: u32,
    /// How many of those are fighting.
    pub fighting: u32,
    /// The per-pulse p99 ceiling, milliseconds, in `Engaged` tempo (40 since
    /// 16c `T-109`; 25 before).
    pub engaged_p99_ms: u32,
    /// The most line-of-sight traces one pulse may make. Perception has no
    /// cross-observer cache, so this is the bound that keeps it affordable.
    pub max_traces_per_pulse: u32,
    /// The most path queries one pulse may make.
    pub max_path_queries_per_pulse: u32,
    /// **The most contact-share operations one pulse may make** (16b cut S.1;
    /// doc 16 §2 W6, §3.6 step 5). One operation is one `(sharer, teammate)` pair
    /// considered — its range test and its line of sight — so the bound is on the
    /// fan-out of the comms mesh, not on how many contacts a pair carries. Like
    /// every key here it is a threshold the harness asserts and never a throttle
    /// the arena applies. 768 since 16b cut J.3b, on the authored lane's measured
    /// per-wake peak of 452 at 300 units and the 30-cell mesh (16c Q26, the
    /// owner's answer); 256 was S.1's figure from the CI cohort's reduced mesh.
    pub max_share_ops_per_pulse: u32,
    /// The most bytes the streamed ledger may reach over the harness's hour of
    /// arena time (64 MiB) — the proof that resident memory stays flat because
    /// the ledger leaves.
    pub max_ledger_bytes: u64,
    /// **The most cells one pulse's line-of-sight traces may read** (16b cut
    /// U.2, 16c `T-36`; doc 16 §2 W4). A threshold the harness asserts, never a
    /// throttle the arena applies.
    pub max_cell_probes_per_pulse: u32,
    /// **The most index candidates one pulse's jammer scans may look at** (16b
    /// cut U.2, 16c `T-29`; doc 16 §9 item 4). A threshold, never a throttle.
    pub max_jam_checks_per_pulse: u32,
    /// **How far resident memory may grow over the harness's long run**,
    /// kibibytes (16b cut U.2, 16c `T-36`).
    pub max_rss_growth_kib: u32,
    /// **The largest encoded `arena.cockpit` keyframe the fan-out lane may see**,
    /// bytes (16b cut U.2, 16c `T-36`). The lane measures the opening, with no
    /// enemy in sight, so a combat frame is not yet measured against it.
    pub max_frame_bytes: u32,
    /// **The most output events one arena pulse may produce through the world's
    /// output path**, as the fan-out lane counts them (16b cut U.2, 16c `T-36`).
    /// It is sized against the lane's first measured burst, the acquisition pass
    /// after power-up (24 panels plus 11 contact-notice lines per pilot = 288
    /// events), and set to 512 so a 24-pilot burst has headroom; whoever moves it
    /// next must say why. That burst is teammates only, so an engaged pulse's
    /// fire and damage notices are unmeasured.
    pub max_output_events_per_pulse: u32,
    /// **The largest arena snapshot the harness's 30-second lane may encode**,
    /// bytes (16b cut AF.2; 16c N16): the snapshot's state string, asserted on
    /// both cohorts. A threshold the harness asserts, never a limit on a running
    /// match. 7 MiB, measured before it was set: the authored lane's 4,198,651
    /// bytes, three halves of it rounded up to the next MiB.
    pub max_snapshot_bytes: u64,
    /// **The most A\* cells one wake's path searches may expand, found or
    /// failed** (16b cut AI.1; 16c `B-84`), asserted by the harness against the
    /// worst wake beside `max_path_queries_per_pulse`: a `Budget` failure and a
    /// `NoRoute` flood are the two dearest searches, and both count. A threshold
    /// the harness asserts, never a throttle the arena applies (D-16-2).
    /// 48,128, measured before it was set: the largest `peak_path_cells` of
    /// both arena lanes at seeds 2350 to 2353, 41,279, times 1.16, rounded up to
    /// the next multiple of 1,024.
    pub max_path_cells_per_pulse: u64,
}

impl Default for TacsimHarnessCfg {
    fn default() -> Self {
        TacsimHarnessCfg {
            units: 300,
            fighting: 24,
            engaged_p99_ms: 40,
            max_traces_per_pulse: 4_000,
            max_path_queries_per_pulse: 64,
            // 16b cut J.3b (16c Q26): the authored lane's 452 peak.
            max_share_ops_per_pulse: 768,
            // 64 MiB.
            max_ledger_bytes: 67_108_864,
            // 16b cut U.2: the five bounds.
            max_cell_probes_per_pulse: 32_000,
            max_jam_checks_per_pulse: 2_048,
            // 64 MiB.
            max_rss_growth_kib: 65_536,
            // 16 KiB.
            max_frame_bytes: 16_384,
            // The measured burst is 288 (24 panels + 11 lines per pilot), on
            // the opening with no enemy in sight; an engaged pulse is unmeasured.
            max_output_events_per_pulse: 512,
            // 16b cut AF.2: the authored lane's 4,198,651 bytes, three halves of
            // it rounded up to the next MiB.
            max_snapshot_bytes: 7_340_032,
            // 16b cut AI.1: the eight-run peak, 41,279, times 1.16, rounded up.
            max_path_cells_per_pulse: 48_128,
        }
    }
}

/// `[worldgen]` section — the M10 batch `@gen/building` knobs and, since S3, the
/// building agent's (14-spatial-implementation-plan §3.4).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorldgenCfg {
    /// The `@gen/building` interior-walk cap: the most rooms one batch will
    /// describe. Defaults to the `mush-worldgen` crate default (40); values
    /// outside `2..=40` fail module registration. The typed conversion into
    /// `mush_worldgen::WorldgenConfig` lives in `mush-server`.
    pub batch_max_rooms: usize,
    /// S3: a proposal's cost cap, USD; a planning run stops before the call that
    /// would pass it (default 5.0).
    pub plan_cost_cap_usd: f64,
    /// S3: the target told to the model and never enforced (default 2.0).
    pub plan_cost_target_usd: f64,
    /// S3: the most provider calls one planning run makes (default 6, `1..=24`).
    pub plan_max_rounds: usize,
    /// S3: a character's default cap on outstanding site requests (default 2).
    pub hydration_cap: u32,
    /// S3: strikes at which a character's cap becomes zero until staff reset it
    /// (default 3).
    pub hydration_strikes_to_zero: u32,
}

impl Default for WorldgenCfg {
    fn default() -> Self {
        // Mirrors `mush_worldgen::WorldgenConfig::default()` and the atlas's
        // hydration defaults (kept as literals so `mush-config` needs no
        // `mush-worldgen` or `mush-atlas` dep).
        WorldgenCfg {
            batch_max_rooms: 40,
            plan_cost_cap_usd: 5.0,
            plan_cost_target_usd: 2.0,
            plan_max_rounds: 6,
            hydration_cap: 2,
            hydration_strikes_to_zero: 3,
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
    /// S1 (doc 14 §2.6, plan §3.1): apply the shipped starter area — the lot, the
    /// OOC Area and the Staff Lobby — at a world's **first** boot. Static content,
    /// no LLM call; a later boot never re-applies it, and a retired pack stays
    /// retired. `true` by default; the testkit composes no content module, so a
    /// test world never grows unless the test asks for the lot.
    pub starter_area: bool,
}

impl Default for GameCfg {
    fn default() -> Self {
        GameCfg {
            linkdead_grace_s: 300,
            max_characters: 3,
            ic_ratio: 1,
            ic_epoch: "2020-01-01 00:00".to_string(),
            starter_area: true,
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
    /// Whether the gateway EXITS (non-zero) once its supervisor has given up on
    /// the server half (WS22-S17). Default `true`.
    ///
    /// An abandoned supervisor leaves the worst state the split can be in: the
    /// gateway process is healthy and idle, every held player is still
    /// connected, and every line they type is refused for ever. Exiting trades
    /// connections that are already useless for a `Restart=on-failure` that
    /// brings the whole game back. Set it `false` on a dev box to keep the
    /// corpse attached for inspection — the gauge and the log marker are
    /// published either way.
    pub exit_on_abandoned_supervision: bool,
}

impl Default for IpcCfg {
    fn default() -> Self {
        IpcCfg {
            listen: "127.0.0.1:6791".to_string(),
            to_server_cap: 1024,
            to_gw_cap: 8192,
            write_timeout_ms: 5000,
            high_water_warn_pct: 80,
            exit_on_abandoned_supervision: true,
        }
    }
}

/// `[pixel]` section (doc 21 cut G.1b): the terminal picture's clock and its
/// step-down on a slow link, read by the gateway half (the SSH TUI's `^G`
/// view). Every key defaults; the whole block may be left out.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PixelCfg {
    /// The picture's clock, milliseconds a frame. Default `50` (20 Hz, the
    /// owner's ruling for both encoders). Clamped to `10..=1000` where it is
    /// used.
    pub frame_interval_ms: u64,
    /// The clock after a step-down, milliseconds a frame. Default `100`
    /// (10 Hz); never faster than `frame_interval_ms`.
    pub low_interval_ms: u64,
    /// Picture writes in a row that each took longer than a frame before the
    /// view steps down to the narrow frame and the low clock. Default `3`.
    pub step_down_after: u32,
    /// Seconds of picture writes that kept up before a stepped-down view
    /// steps back up. Default `10`.
    pub step_up_after_s: u64,
    /// The staff diagnostics, box-wide (the owner's ruling of 2026-10-22):
    /// the `u` key that uncaps the clock and the engineering numbers on the
    /// picture's status line. Default `false`: players see neither;
    /// `@pixel/stats` reports the numbers either way.
    pub diagnostics: bool,
}

impl Default for PixelCfg {
    fn default() -> Self {
        PixelCfg {
            frame_interval_ms: 50,
            low_interval_ms: 100,
            step_down_after: 3,
            step_up_after_s: 10,
            diagnostics: false,
        }
    }
}

/// The host and TCP port a `[store].redis_url` points at, when it names one.
///
/// Deliberately small: enough of `redis://[user:pass@]host[:port][/db]` (and
/// `rediss://`) to answer "does this occupy the port `[ipc].listen` wants".
/// A `unix://` URL — or anything unparseable — is `None`, i.e. no TCP port to
/// collide with.
fn redis_url_authority(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default_port: u16 = match scheme.to_ascii_lowercase().as_str() {
        "redis" | "rediss" => 6379,
        _ => return None,
    };
    // Drop the path/query (`/0`, `?db=0`), then any `user:pass@` userinfo.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    if host_port.is_empty() {
        return None;
    }
    // A bracketed IPv6 literal: `[::1]` or `[::1]:6791`.
    if let Some(after_bracket) = host_port.strip_prefix('[') {
        let (host, tail) = after_bracket.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None => default_port,
        };
        return Some((host.to_string(), port));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((host_port.to_string(), default_port)),
    }
}

/// Whether binding `ipc` would land on the same box+interface the store host
/// names — the only case where equal ports actually collide. A remote store on
/// the same port number is not a collision, and must not be refused.
fn same_host(ipc: SocketAddr, redis_host: &str) -> bool {
    let ipc_ip = ipc.ip();
    // `0.0.0.0`/`::` binds loopback too, so it collides with a local store.
    let ipc_is_local = ipc_ip.is_loopback() || ipc_ip.is_unspecified();
    match redis_host.parse::<IpAddr>() {
        Ok(redis_ip) => ipc_ip == redis_ip || (ipc_is_local && redis_ip.is_loopback()),
        // A hostname we will not resolve here; only `localhost` is knowably
        // this box.
        Err(_) => ipc_is_local && redis_host.eq_ignore_ascii_case("localhost"),
    }
}

/// The whole configuration. [`Config::load`] also checks top-level header typos;
/// unrelated unknown sections and each block's existing key policy are preserved.
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
    /// default; mapped to the game sheet's onboarding policy by the host.
    pub onboarding: OnboardingCfg,
    /// `[tacsim]` — the TacSim layer's own knobs (E7 cut 1, 10b §7). `enabled` is
    /// `false` off-config, so an unconfigured server keeps its pre-E7 behaviour. Since 16b
    /// cut R-b it is also the one game's block (`[tacsim.economy]` included).
    pub tacsim: TacsimCfg,
    /// `[npc]` — the NPC agent layer's world-task knobs (H2). Defaults whole.
    #[serde(default)]
    pub npc: NpcCfg,
    /// `[atlas]` — the spatial layer's knobs (S1). Defaults whole.
    #[serde(default)]
    pub atlas: AtlasCfg,
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
    /// `[pixel]` — the terminal picture's clock, step-down and staff
    /// diagnostics (doc 21 cut G.1b). Defaults whole.
    pub pixel: PixelCfg,
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
            weather: WeatherCfg::default(),
            worldgen: WorldgenCfg::default(),
            economy: EconomyCfg::default(),
            onboarding: OnboardingCfg::default(),
            tacsim: TacsimCfg::default(),
            npc: NpcCfg::default(),
            atlas: AtlasCfg::default(),
            llm: empty_toml_table(),
            ipc: IpcCfg::default(),
            pixel: PixelCfg::default(),
        }
    }
}

// Keep alongside Config's top-level fields. We deliberately do not reject every
// unknown table: deployments may carry unrelated extension configuration.
const CONFIG_SECTIONS: &[&str] = &[
    "server",
    "log",
    "store",
    "offers",
    "resolution",
    "ssh",
    "web",
    "discord",
    "game",
    "weather",
    "worldgen",
    "economy",
    "onboarding",
    "tacsim",
    "npc",
    "atlas",
    "llm",
    "ipc",
    "pixel",
];

/// Levenshtein distance, only when it is at most two. The working row is bounded
/// by the known name's length, even if the input contains a very long header.
fn section_edit_distance(section: &str, known: &str) -> Option<usize> {
    let known: Vec<char> = known.chars().collect();
    if section.chars().count().abs_diff(known.len()) > 2 {
        return None;
    }
    let mut row: Vec<usize> = (0..=known.len()).collect();
    for (i, actual) in section.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, expected) in known.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(actual != *expected));
            diagonal = above;
        }
    }
    let edits = row[known.len()];
    (edits <= 2).then_some(edits)
}

impl Config {
    /// Load from `path`. A missing file returns the defaults; a present but
    /// malformed file or a likely mistyped top-level section is an error.
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
        let sections: toml::Table = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
        for (section, value) in &sections {
            let is_section = value.is_table()
                || value
                    .as_array()
                    .is_some_and(|items| items.iter().any(toml::Value::is_table));
            if !is_section || CONFIG_SECTIONS.contains(&section.as_str()) {
                continue;
            }
            if let Some(suggestion) = CONFIG_SECTIONS
                .iter()
                .copied()
                .filter_map(|known| {
                    section_edit_distance(section, known).map(|edits| (edits, known))
                })
                .min()
                .map(|(_, known)| known)
            {
                return Err(ConfigError::MistypedSection {
                    path: path.to_path_buf(),
                    section: section.clone(),
                    suggestion,
                });
            }
        }
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

    /// The parsed and validated `[ipc].listen` boundary endpoint (M16b).
    ///
    /// Validation lives here, not at a call site, so it fires on every load
    /// path (gateway, server, and any future reader):
    ///
    /// - a TCP endpoint whose port is already the store's port on the same
    ///   host is **refused**. The default `[ipc].listen` is `127.0.0.1:6791`,
    ///   and an instance whose Redis happens to live on 6791 and which never
    ///   wrote an `[ipc]` section would try to bind the boundary on top of its
    ///   own store, fail, and crash-loop under the gateway's relaunch guard.
    /// - a TCP endpoint at all logs a loud warn. It is not refused — Windows
    ///   and portable development have no Unix sockets — but the boundary is
    ///   unauthenticated, so on Linux prod a `127.0.0.1` port lets *any* local
    ///   process impersonate the gateway and assert authenticated sessions
    ///   (`docs/ops.md`, "Prefer the Unix-socket form on Linux prod").
    ///
    /// # Errors
    ///
    /// [`ConfigError::BadIpcListen`] when the value is neither a socket address
    /// nor (where supported) a usable Unix path;
    /// [`ConfigError::IpcStorePortCollision`] when it would bind the store's
    /// own port.
    pub fn ipc_endpoint(&self) -> Result<mush_ipc::IpcEndpoint, ConfigError> {
        let endpoint = mush_ipc::IpcEndpoint::parse(&self.ipc.listen).map_err(|msg| {
            ConfigError::BadIpcListen {
                value: self.ipc.listen.clone(),
                detail: msg,
            }
        })?;
        // A Unix socket (where the platform has them) is the recommended prod
        // form: nothing further to check, and nothing to warn about.
        let addr = match &endpoint {
            mush_ipc::IpcEndpoint::Tcp(addr) => *addr,
            #[cfg(unix)]
            mush_ipc::IpcEndpoint::Unix(_) => return Ok(endpoint),
        };
        // Only a Redis-backed instance actually has a listener on that port;
        // `store = "memory"` never contacts `redis_url` at all.
        if self.store.store.eq_ignore_ascii_case("redis")
            && let Some((redis_host, redis_port)) = redis_url_authority(&self.store.redis_url)
            && addr.port() == redis_port
            && same_host(addr, &redis_host)
        {
            return Err(ConfigError::IpcStorePortCollision {
                ipc_listen: self.ipc.listen.clone(),
                redis_url: self.store.redis_url.clone(),
                port: redis_port,
            });
        }
        tracing::warn!(
            endpoint = %addr,
            "[ipc].listen is a TCP port, not a Unix socket: the \
             gateway<->server boundary is UNAUTHENTICATED, so any local \
             process can connect and impersonate the gateway. TCP is for \
             Windows/portable development; on Linux prod use a Unix socket \
             path in a mode-0750 RuntimeDirectory (docs/ops.md)"
        );
        Ok(endpoint)
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

    /// Whether an abandoned gateway supervisor exits the process (WS22-S17).
    #[must_use]
    pub fn ipc_exit_on_abandoned_supervision(&self) -> bool {
        self.ipc.exit_on_abandoned_supervision
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

    /// Directory of the geology key family (doc 20 cut SM.1; Q-20-12), or `None` when
    /// `[tacsim.geology].key_dir` is absent — which leaves the mining layer disabled. There is
    /// deliberately no default under `[server].data_dir`.
    #[must_use]
    pub fn geology_key_dir(&self) -> Option<PathBuf> {
        self.tacsim.geology.key_dir.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_refuses_near_miss_headers_but_preserves_extensions() {
        let dir = std::env::temp_dir().join(format!("mush-section-typos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        for header in ["tac_sim", "tascim", "tacsi", "tacximx", "tascim.arena"] {
            std::fs::write(&path, format!("[{header}]\nenabled = true\n")).unwrap();
            let err = Config::load(&path).expect_err(header);
            assert!(
                matches!(
                    &err,
                    ConfigError::MistypedSection {
                        suggestion: "tacsim",
                        ..
                    }
                ),
                "{header}: {err}"
            );
            assert!(err.to_string().contains(&path.display().to_string()));
            assert!(err.to_string().contains(header.split('.').next().unwrap()));
        }
        for known in CONFIG_SECTIONS {
            // An empty section either loads or is refused by its own required
            // fields; it must never be mistaken for an unknown header.
            std::fs::write(&path, format!("[{known}]\n")).unwrap();
            assert!(!matches!(
                Config::load(&path),
                Err(ConfigError::MistypedSection { .. })
            ));
        }
        std::fs::write(&path, "[[tascim]]\nenabled = true\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::MistypedSection { .. })
        ));
        std::fs::write(
            &path,
            "release_label = 'test'\n[external_extension]\nvalue = true\n\
             [web.custom_panel]\nvalue = 1\n[server]\nextra = true\n[tacsim]\nenabled = true\n",
        )
        .unwrap();
        assert!(Config::load(&path).unwrap().tacsim.enabled);
        std::fs::write(&path, "[tacsim]\nenabeld = true\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::Parse { .. })
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    fn surface_interlock_defaults_true_and_refuses_a_misspelt_key() {
        // Doc 20 cut SM.10 (§9.4, lead ruling 9): the lever defaults on, parses either way (the
        // host refuses `false` at boot, not the parser), and a near-miss key is refused.
        let defaulted: Config = toml::from_str("[server]\ndata_dir = \"./world\"").unwrap();
        assert!(defaulted.tacsim.surface.registry_interlock);
        let off: Config = toml::from_str("[tacsim.surface]\nregistry_interlock = false").unwrap();
        assert!(!off.tacsim.surface.registry_interlock);
        assert!(
            toml::from_str::<Config>("[tacsim.surface]\nregistry_interlok = true").is_err(),
            "deny_unknown_fields refuses a misspelt surface key"
        );
    }

    #[test]
    fn geology_key_directory_has_no_default_and_refuses_a_misspelt_key() {
        // Doc 20 cut SM.1: `[tacsim.geology].key_dir` names the directory; absent is `None`
        // (the mining layer reads disabled), and a near-miss key is refused, never defaulted.
        let cfg: Config = toml::from_str(
            "[server]\ndata_dir = \"./world\"\n[tacsim.geology]\nkey_dir = \"./secrets/geology\"",
        )
        .unwrap();
        assert_eq!(
            cfg.geology_key_dir(),
            Some(PathBuf::from("./secrets/geology"))
        );

        let defaulted: Config = toml::from_str("[server]\ndata_dir = \"./world\"").unwrap();
        assert_eq!(defaulted.geology_key_dir(), None);

        assert!(
            toml::from_str::<Config>("[tacsim.geology]\nkey_dirr = \"x\"").is_err(),
            "deny_unknown_fields refuses a misspelt geology key"
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
        // WS22-S17: an abandoned supervisor is fatal unless an operator says
        // otherwise. The default is the production posture, not the dev one.
        assert!(cfg.ipc_exit_on_abandoned_supervision());

        // Back-compat: an old [ipc] with only `listen` (no M18 keys) still
        // parses and yields the defaults.
        let cfg: Config = toml::from_str("[ipc]\nlisten = \"127.0.0.1:6799\"").unwrap();
        assert_eq!(cfg.ipc.listen, "127.0.0.1:6799");
        assert_eq!(cfg.ipc_to_server_cap(), 1024);
        assert_eq!(cfg.ipc_to_gw_cap(), 8192);
        assert_eq!(cfg.ipc_write_timeout_ms(), 5000);
        assert_eq!(cfg.ipc_high_water_warn_pct(), 80);
        assert!(cfg.ipc_exit_on_abandoned_supervision());

        // Explicit overrides parse.
        let cfg: Config = toml::from_str(
            "[ipc]\nlisten = \"127.0.0.1:6799\"\nto_server_cap = 16\nto_gw_cap = 32\nwrite_timeout_ms = 250\nhigh_water_warn_pct = 90",
        )
        .unwrap();
        assert_eq!(cfg.ipc_to_server_cap(), 16);
        assert_eq!(cfg.ipc_to_gw_cap(), 32);
        assert_eq!(cfg.ipc_write_timeout_ms(), 250);
        assert_eq!(cfg.ipc_high_water_warn_pct(), 90);

        // The dev-box opt-out parses, and is the only way to keep the corpse.
        let cfg: Config = toml::from_str("[ipc]\nexit_on_abandoned_supervision = false").unwrap();
        assert!(!cfg.ipc_exit_on_abandoned_supervision());

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
        // The example's [weather]/[worldgen] sections parse (values are
        // commented, so they resolve to the crate defaults).
        assert_eq!(cfg.weather.flavor_variants, 10);
        assert_eq!(cfg.worldgen.batch_max_rooms, 40);
        // 16b cut G: the example writes the combat arena's eight tables out with
        // real values rather than commenting them, so this is also the check that
        // those values ARE the defaults. A drift between the shipped file and the
        // code is a red test here rather than a box quietly running a number
        // nobody wrote.
        assert_eq!(cfg.tacsim.arena, TacsimArenaCfg::default());
        assert_eq!(cfg.tacsim.match_, TacsimMatchCfg::default());
        // 16b cut Y.2: and the command block, written out the same way.
        assert_eq!(cfg.tacsim.command, TacsimCommandCfg::default());
        // 17b cut M1b.1: the minds block is written out commented, so it
        // parses to the default.
        assert_eq!(cfg.tacsim.minds, TacsimMindsCfg::default());
        assert_eq!(cfg.tacsim.sensors, TacsimSensorsCfg::default());
        assert_eq!(cfg.tacsim.resolution, TacsimResolutionCfg::default());
        assert_eq!(cfg.tacsim.harness, TacsimHarnessCfg::default());
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
    /// attribute): `[tacsim]` is the only
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

    // ---- 16b cut G — the combat arena's five blocks ----

    /// Cut G, the defaults: an **empty** config carries 16b §9's constants table
    /// exactly. This is the assertion that makes every shipped file below
    /// redundant-on-purpose rather than load-bearing — a box that writes none of
    /// these sections runs the numbers the design note names.
    #[test]
    fn cut_g_the_five_arena_blocks_default_to_the_constants_table() {
        let cfg: Config = toml::from_str("").expect("an empty config parses");
        let arena = &cfg.tacsim.arena;
        assert_eq!(
            (
                arena.tempo.quiet_ms,
                arena.tempo.alert_ms,
                arena.tempo.engaged_ms
            ),
            (5_000, 1_000, 500),
            "[tacsim.arena.tempo] periods"
        );
        assert_eq!(
            (arena.tempo.alert_hold_ms, arena.tempo.engaged_hold_ms),
            (30_000, 10_000),
            "[tacsim.arena.tempo] holds"
        );
        assert_eq!(
            arena.tempo.alert_range_cm, 0,
            "0 is the DERIVED sentinel: twice the longest weapon range in the match, held to the \
             window's own diagonal"
        );
        assert_eq!(
            (arena.tempo.substep_max_ms, arena.tempo.stall_ms),
            (250, 2_000)
        );
        assert_eq!(arena.max_catch_up_steps, 4);
        // 16b cut E.3: two seconds of holding against traffic before the stop.
        assert_eq!(arena.occupied_stop_ms, 2_000);
        // 16b cut Z.1 (16c N6): lances of four.
        assert_eq!(arena.squad_size, 4);
        // 16b cut AE.3 (doc 16 §2 W2): one drone a side.
        assert_eq!(
            arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 1)])
        );
        // 16b cut AF.1 (16c N16): a snapshot every five seconds of arena time.
        assert_eq!(arena.snapshot_ms, 5_000);
        assert_eq!(arena.intents.max_pending_per_unit, 8);
        assert_eq!(
            (
                arena.ledger.summary_ms,
                arena.ledger.max_bytes,
                arena.ledger.max_unflushed_chunks
            ),
            (5_000, 67_108_864, 256),
            "[tacsim.arena.ledger]; max_bytes is 64 MiB"
        );

        let m = &cfg.tacsim.match_;
        assert_eq!(
            (
                m.startup_ms,
                m.board_timeout_ms,
                m.max_duration_ms,
                m.max_slots_per_side
            ),
            (10_000, 120_000, 900_000, 12),
            "[tacsim.match]"
        );
        // 16b cut Y.3 (16c `N4`): five seconds before a dropped pilot's `BOARD`
        // takes the tank back.
        assert_eq!(m.reclaim_min_ms, 5_000, "[tacsim.match].reclaim_min_ms");

        // 16b cut Y.2 (doc 16 W8): the span and the rate, at the figures the
        // help quotes.
        assert_eq!(cfg.tacsim.command.max_span, 4, "[tacsim.command].max_span");
        assert_eq!(
            cfg.tacsim.command.orders_per_minute, 12,
            "[tacsim.command].orders_per_minute"
        );

        assert_eq!(cfg.tacsim.sensors.pass_ms, 1_000);
        assert!(
            (cfg.tacsim.sensors.max_signature_factor - 1.0).abs() < f64::EPSILON,
            "[tacsim.sensors].max_signature_factor is 1.0 (16b cut M.4; 1.5 until then)"
        );
        assert_eq!(cfg.tacsim.sensors.lost_after_passes, 3);

        assert!(
            cfg.tacsim.resolution.paths.is_empty(),
            "[tacsim.resolution].paths defaults empty; cut W wires the shipped tables"
        );

        let h = &cfg.tacsim.harness;
        assert_eq!((h.units, h.fighting), (300, 24));
        assert_eq!(h.engaged_p99_ms, 40);
        assert_eq!(
            (
                h.max_traces_per_pulse,
                h.max_path_queries_per_pulse,
                h.max_share_ops_per_pulse
            ),
            (4_000, 64, 768),
            "max_share_ops_per_pulse is 768 since 16b cut J.3b (16c Q26)"
        );
        assert_eq!(h.max_ledger_bytes, 67_108_864, "64 MiB");
        // 16b cut U.2: the five bounds.
        assert_eq!(
            (
                h.max_cell_probes_per_pulse,
                h.max_jam_checks_per_pulse,
                h.max_rss_growth_kib,
                h.max_frame_bytes,
                h.max_output_events_per_pulse,
            ),
            (32_000, 2_048, 65_536, 16_384, 512)
        );
        // 16b cut AF.2: 7 MiB, measured.
        assert_eq!(h.max_snapshot_bytes, 7_340_032);
        // 16b cut AI.1 (16c `B-84`): the four-seed measurement, sized.
        assert_eq!(h.max_path_cells_per_pulse, 48_128);
    }

    /// Cut G: every key of all eight tables parses to what it says. Written as
    /// one explicit config so a field added to a struct without a line here is
    /// visible as an untested key rather than silently defaulting.
    #[test]
    fn cut_g_the_five_arena_blocks_parse_an_explicit_table() {
        let cfg: Config = toml::from_str(
            "[tacsim.arena]\n\
             max_catch_up_steps = 2\n\
             occupied_stop_ms = 1500\n\
             squad_size = 3\n\
             max_units_by_class = { drone = 2, walker = 4 }\n\
             snapshot_ms = 2500\n\
             [tacsim.arena.tempo]\n\
             quiet_ms = 4000\n\
             alert_ms = 900\n\
             engaged_ms = 400\n\
             alert_hold_ms = 20000\n\
             engaged_hold_ms = 9000\n\
             alert_range_cm = 120000\n\
             substep_max_ms = 200\n\
             stall_ms = 1500\n\
             [tacsim.arena.intents]\n\
             max_pending_per_unit = 3\n\
             [tacsim.arena.ledger]\n\
             summary_ms = 2500\n\
             max_bytes = 1048576\n\
             max_unflushed_chunks = 64\n\
             [tacsim.match]\n\
             startup_ms = 5000\n\
             board_timeout_ms = 60000\n\
             max_duration_ms = 600000\n\
             max_slots_per_side = 4\n\
             [tacsim.sensors]\n\
             pass_ms = 750\n\
             max_signature_factor = 2.25\n\
             lost_after_passes = 5\n\
             [tacsim.resolution]\n\
             paths = [\"content/resolution.json\"]\n\
             [tacsim.harness]\n\
             units = 10\n\
             fighting = 4\n\
             engaged_p99_ms = 50\n\
             max_traces_per_pulse = 100\n\
             max_path_queries_per_pulse = 8\n\
             max_share_ops_per_pulse = 12\n\
             max_ledger_bytes = 2048\n\
             max_cell_probes_per_pulse = 900\n\
             max_jam_checks_per_pulse = 70\n\
             max_rss_growth_kib = 4096\n\
             max_frame_bytes = 2000\n\
             max_output_events_per_pulse = 30\n\
             max_snapshot_bytes = 4096\n\
             max_path_cells_per_pulse = 5000\n",
        )
        .expect("the written-out blocks parse");

        let arena = &cfg.tacsim.arena;
        assert_eq!(arena.max_catch_up_steps, 2);
        // 16b cut E.3: the traffic hold's horizon, written out like the rest.
        assert_eq!(arena.occupied_stop_ms, 1_500);
        // 16b cut Z.1: the squad size, written out like the rest.
        assert_eq!(arena.squad_size, 3);
        // 16b cut AE.3: the class caps, as the inline table the example writes.
        assert_eq!(
            arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 2), ("walker".to_owned(), 4)])
        );
        // 16b cut AF.1: the snapshot cadence, written out like the rest.
        assert_eq!(arena.snapshot_ms, 2_500);
        assert_eq!(
            (
                arena.tempo.quiet_ms,
                arena.tempo.alert_ms,
                arena.tempo.engaged_ms,
                arena.tempo.alert_hold_ms,
                arena.tempo.engaged_hold_ms,
                arena.tempo.alert_range_cm,
                arena.tempo.substep_max_ms,
                arena.tempo.stall_ms,
            ),
            (4_000, 900, 400, 20_000, 9_000, 120_000, 200, 1_500)
        );
        assert_eq!(arena.intents.max_pending_per_unit, 3);
        assert_eq!(
            (
                arena.ledger.summary_ms,
                arena.ledger.max_bytes,
                arena.ledger.max_unflushed_chunks
            ),
            (2_500, 1_048_576, 64)
        );

        let m = &cfg.tacsim.match_;
        assert_eq!(
            (
                m.startup_ms,
                m.board_timeout_ms,
                m.max_duration_ms,
                m.max_slots_per_side
            ),
            (5_000, 60_000, 600_000, 4),
            "the TOML key is `match`, the Rust field `match_`"
        );

        assert_eq!(cfg.tacsim.sensors.pass_ms, 750);
        assert!((cfg.tacsim.sensors.max_signature_factor - 2.25).abs() < f64::EPSILON);
        assert_eq!(cfg.tacsim.sensors.lost_after_passes, 5);
        assert_eq!(
            cfg.tacsim.resolution.paths,
            vec!["content/resolution.json".to_string()]
        );

        let h = &cfg.tacsim.harness;
        assert_eq!(
            (
                h.units,
                h.fighting,
                h.engaged_p99_ms,
                h.max_traces_per_pulse,
                h.max_path_queries_per_pulse,
                h.max_share_ops_per_pulse,
            ),
            (10, 4, 50, 100, 8, 12)
        );
        assert_eq!(h.max_ledger_bytes, 2_048);
        // 16b cut U.2: the five bounds, written out like the rest.
        assert_eq!(
            (
                h.max_cell_probes_per_pulse,
                h.max_jam_checks_per_pulse,
                h.max_rss_growth_kib,
                h.max_frame_bytes,
                h.max_output_events_per_pulse,
            ),
            (900, 70, 4_096, 2_000, 30)
        );
        // 16b cut AF.2: the snapshot bound, written out like the rest.
        assert_eq!(h.max_snapshot_bytes, 4_096);
        // 16b cut AI.1: the cell bound, written out like the rest.
        assert_eq!(h.max_path_cells_per_pulse, 5_000);
    }

    /// **16b cut AF.2** (16c N16): `[tacsim.harness].max_snapshot_bytes` reads
    /// the measured 7 MiB when a config does not name it, reads what it names,
    /// and a misspelling refuses the config under `deny_unknown_fields` rather
    /// than leaving the default silently in force. The floor of one is
    /// `tacsim::config::HarnessConfig::validate`'s.
    #[test]
    fn af2_max_snapshot_bytes_absent_written_and_misspelt() {
        let absent: Config =
            toml::from_str("[tacsim.harness]\nunits = 300\n").expect("a section without it");
        assert_eq!(
            absent.tacsim.harness.max_snapshot_bytes, 7_340_032,
            "the compiled default"
        );
        for written in [0_u64, 1, 8_388_608, u64::from(u32::MAX) + 1] {
            let cfg: Config = toml::from_str(&format!(
                "[tacsim.harness]\nmax_snapshot_bytes = {written}\n"
            ))
            .expect("the key parses");
            assert_eq!(
                cfg.tacsim.harness.max_snapshot_bytes, written,
                "what is written is read; the floor is checked where the harness block is mapped"
            );
        }
        let typo = toml::from_str::<Config>("[tacsim.harness]\nmax_snapshots_bytes = 1\n")
            .expect_err("a misspelt key refuses the config");
        assert!(typo.to_string().contains("max_snapshots_bytes"), "{typo}");
    }

    /// **16b cut AF.1** (16c N16): `[tacsim.arena].snapshot_ms` reads the compiled
    /// five seconds when a config does not name it, reads what it names —
    /// `0`, the kill switch, included — and a misspelling of it refuses the
    /// config under `deny_unknown_fields` rather than silently leaving the
    /// default in force. The band is `tacsim::config::ArenaConfig::validate`'s.
    #[test]
    fn af1_snapshot_ms_absent_written_and_misspelt() {
        let absent: Config =
            toml::from_str("[tacsim.arena]\nsquad_size = 4\n").expect("a section without it");
        assert_eq!(
            absent.tacsim.arena.snapshot_ms, 5_000,
            "the compiled default"
        );
        for written in [0_u32, 1_000, 60_000, 7] {
            let cfg: Config = toml::from_str(&format!("[tacsim.arena]\nsnapshot_ms = {written}\n"))
                .expect("the key parses");
            assert_eq!(
                cfg.tacsim.arena.snapshot_ms, written,
                "what is written is read; the band is checked where the arena is mapped"
            );
        }
        let typo = toml::from_str::<Config>("[tacsim.arena]\nsnapshots_ms = 5000\n")
            .expect_err("a misspelt key refuses the config");
        assert!(typo.to_string().contains("snapshots_ms"), "{typo}");
    }

    /// **16b cut AE.3**: `[tacsim.arena].max_units_by_class` round-trips in both
    /// TOML spellings — the inline table the shipped example writes under
    /// `[tacsim.arena]`, and its own `[tacsim.arena.max_units_by_class]` header —
    /// to one map; a config that names nothing reads the compiled `drone = 1`;
    /// and a written table **replaces** that default here (the merge over it is
    /// `mush_server::config_ext`'s, where the class words are known), which is
    /// what this layer promises and no more. Words are not checked here.
    #[test]
    fn ae3_max_units_by_class_parses_inline_or_under_its_own_header() {
        let inline: Config = toml::from_str(
            "[tacsim.arena]\n\
             squad_size = 4\n\
             max_units_by_class = { drone = 1 }\n",
        )
        .expect("the inline table parses");
        assert_eq!(
            inline.tacsim.arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 1)])
        );

        let header: Config = toml::from_str(
            "[tacsim.arena]\n\
             squad_size = 4\n\
             [tacsim.arena.max_units_by_class]\n\
             drone = 3\n\
             walker = 0\n",
        )
        .expect("the header spelling parses");
        assert_eq!(
            header.tacsim.arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 3), ("walker".to_owned(), 0)])
        );

        let unnamed: Config = toml::from_str("[tacsim.arena]\nsquad_size = 4\n")
            .expect("a section without the key parses");
        assert_eq!(
            unnamed.tacsim.arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 1)]),
            "the compiled default when the key is absent"
        );

        let walker_only: Config = toml::from_str(
            "[tacsim.arena]\n\
             max_units_by_class = { walker = 2 }\n",
        )
        .expect("a table naming another class parses");
        assert_eq!(
            walker_only.tacsim.arena.max_units_by_class,
            BTreeMap::from([("walker".to_owned(), 2)]),
            "this layer replaces; config_ext lays it over the default"
        );

        let flier: Config = toml::from_str(
            "[tacsim.arena]\n\
             max_units_by_class = { flier = 2 }\n",
        )
        .expect("an unknown word parses here and is refused by config_ext");
        assert_eq!(flier.tacsim.arena.max_units_by_class.get("flier"), Some(&2));
    }

    /// Cut G's one enforced property: **a typo in any of the eight tables refuses
    /// the boot, naming the key.** No value here has a reader yet — the cut that
    /// reads a field validates it — so shape is all cut G can enforce, and it is
    /// 16b §3's "a typo in a new section refuses boot".
    ///
    /// `match_` is here twice on purpose: the Rust field name must NOT be
    /// accepted as a TOML key, or an operator who writes `[tacsim.match_]` gets a
    /// section the server silently ignores.
    #[test]
    fn cut_g_an_unknown_key_in_any_arena_block_refuses_to_start() {
        for (text, key) in [
            (
                "[tacsim.arena]\nmax_catch_up_step = 4\n",
                "max_catch_up_step",
            ),
            ("[tacsim.arena.tempo]\nquiet = 5000\n", "quiet"),
            ("[tacsim.arena.intents]\nmax_pending = 8\n", "max_pending"),
            ("[tacsim.arena.ledger]\nmax_byte = 1\n", "max_byte"),
            ("[tacsim.match]\nstartup = 10000\n", "startup"),
            // 16b cut Y.2: the command block refuses a typo like the rest.
            ("[tacsim.command]\nspan = 4\n", "span"),
            ("[tacsim.sensors]\npass = 1000\n", "pass"),
            ("[tacsim.resolution]\npath = []\n", "path"),
            ("[tacsim.harness]\nunit = 300\n", "unit"),
            // The keyword dodge: `match_` is the Rust spelling, never the key.
            ("[tacsim]\nmatch_ = {}\n", "match_"),
        ] {
            let refused = toml::from_str::<Config>(text).expect_err(text);
            let refused = refused.to_string();
            assert!(refused.contains(key), "{text}: {refused}");
            assert!(refused.contains("unknown field"), "{text}: {refused}");
        }
    }

    /// **16b cut Y.2**: `[tacsim.command]` is optional, carries both keys when
    /// written, and refuses an unknown key under it by name.
    #[test]
    fn y2_the_command_block_parses_absent_written_and_refuses_a_typo() {
        let absent: Config = toml::from_str("").expect("an empty config parses");
        assert_eq!(absent.tacsim.command, TacsimCommandCfg::default());
        assert_eq!(
            (
                absent.tacsim.command.max_span,
                absent.tacsim.command.orders_per_minute
            ),
            (4, 12)
        );

        let written: Config =
            toml::from_str("[tacsim.command]\nmax_span = 2\norders_per_minute = 30\n")
                .expect("the written block parses");
        assert_eq!(
            written.tacsim.command,
            TacsimCommandCfg {
                max_span: 2,
                orders_per_minute: 30,
            }
        );

        let text = "[tacsim.command]\nmax_span = 4\nmax_squads = 4\n";
        let refused = toml::from_str::<Config>(text).expect_err(text).to_string();
        assert!(refused.contains("max_squads"), "{refused}");
        assert!(refused.contains("unknown field"), "{refused}");
    }

    /// **17b cut M1b.1**: `[tacsim.minds]` is optional, carries the floor when
    /// written, and refuses an unknown key under it by name.
    #[test]
    fn m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo() {
        let absent: Config = toml::from_str("").expect("an empty config parses");
        assert_eq!(absent.tacsim.minds, TacsimMindsCfg::default());
        assert_eq!(absent.tacsim.minds.odds_floor_permille, 100);

        let written: Config = toml::from_str("[tacsim.minds]\nodds_floor_permille = 50\n")
            .expect("the written block parses");
        assert_eq!(
            written.tacsim.minds,
            TacsimMindsCfg {
                odds_floor_permille: 50,
                ..TacsimMindsCfg::default()
            }
        );
        // 17b cut M4.3: the fill's class, written and absent; the lab knob
        // it retired refuses the boot by name.
        let capped: Config =
            toml::from_str("[tacsim.minds]\nfill_class = \"II\"\n").expect("the fill block parses");
        assert_eq!(capped.tacsim.minds.fill_class, "II");
        assert_eq!(absent.tacsim.minds.fill_class, "I");
        let retired = "[tacsim.minds]\nfill_controller = \"mind\"\n";
        let refused = toml::from_str::<Config>(retired)
            .expect_err(retired)
            .to_string();
        assert!(refused.contains("fill_controller"), "{refused}");
        assert!(refused.contains("unknown field"), "{refused}");

        let text = "[tacsim.minds]\nodds_floor = 50\n";
        let refused = toml::from_str::<Config>(text).expect_err(text).to_string();
        assert!(refused.contains("odds_floor"), "{refused}");
        assert!(refused.contains("unknown field"), "{refused}");
    }

    /// **16b cut U.3** (16c `T-37`): `[tacsim.harness].weapons` is retired. The
    /// harness read it into a knob and reported it, and since 16b cut W no lane
    /// armed anything by it — the cohort is armed as the catalog authors it — so
    /// the key went with the knob, and a `mushd.toml` that still carries it
    /// refuses to start like any other unknown key rather than keeping a number
    /// that means nothing.
    #[test]
    fn u3_the_retired_harness_weapons_key_refuses_to_start() {
        let text = "[tacsim.harness]\nweapons = 150\n";
        let refused = toml::from_str::<Config>(text).expect_err(text).to_string();
        assert!(refused.contains("weapons"), "{refused}");
        assert!(refused.contains("unknown field"), "{refused}");
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

#[cfg(test)]
mod recovery_capacity_tests {
    #[test]
    fn m6_snapshot_ceiling_is_a_distinct_typed_operator_setting() {
        let cfg: super::Config =
            toml::from_str("[tacsim.arena]\nsnapshot_max_bytes = 33554432\n").unwrap();
        assert_eq!(cfg.tacsim.arena.snapshot_max_bytes, 32 * 1024 * 1024);
        assert_eq!(
            super::TacsimArenaCfg::default().snapshot_max_bytes,
            480 * 1024 * 1024
        );
    }
}
