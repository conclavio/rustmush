//! **The frozen `[tacsim]` block** (10b §7, OPEN 4), parsed host-side and handed in
//! at wiring — never read from a document, never written by a command.
//!
//! The default is **`enabled = false`**: a server that has never heard of this
//! section is bit-identical to one built before cut 1 existed. The verbs still
//! register and answer "not enabled on this server", which is E5c deviation 1 and
//! E6a's `[shadowrun.economy]` precedent — a disabled layer refuses out loud rather
//! than hiding a verb, and it is also what keeps the rule-1 allowlist tripwire able
//! to see `@tacsim` in a default composition.
//!
//! # Live in cut 1, and inert in cut 1 (OPEN 4)
//!
//! | key | cut 1 | why |
//! |---|---|---|
//! | `enabled` | **live** | whether the layer does anything |
//! | `pack_paths` | **live** | where the host reads rules packs from |
//! | `seed_paths` | **live** | where the host reads campaign-seed packs from |
//! | `corridor` | **live** | which seed pack `@tacsim/seed` defaults to |
//! | `active_domain_limit` | parsed, **inert** | cut 6's `MAX_ACTIVE_DOMAINS` |
//! | `ground_pulse_ms`, `space_pulse_ms` | parsed, **inert** | cut 8's pulse periods |
//! | all of `[tacsim.time]` | parsed, **inert** | cuts 4/8/11's rate and catch-up policy |
//! | all of `[tacsim.recovery]` | **live** (E7r1 R4) | the recovery provider's pool, tiers, cap, thresholds, cooldown and insurance policy |
//!
//! `[tacsim.recovery]` was cut 11's and was **not added at all** — *a knob that parses
//! and configures nothing is a promise, and cut 11 has not made it yet*. **E7 residue
//! pass 1 (R4) adds it, in the same pass that builds the provider it configures**
//! (10b §15.1 items 5 and 9), so every field has a reader: [`TacsimRecoveryConfig`]
//! reaches `mush-campaign` through `mush_server::modules`' `with_recovery` seam, and
//! `@recovery` reads it. It is **live**, not inert, and a typo in it refuses the boot
//! for [`TacsimConfig`]'s own reason.
//!
//! # `[tacsim.time]` never seeds the world timeline
//!
//! 08i §3's configuration row, and a pinned test in `mush-server`. Generation 1 comes
//! from `[game].ic_ratio`/`ic_epoch` through `load_or_seed_world_timeline`'s
//! `if docs.is_empty()` branch, and persisted truth wins thereafter. [`TacsimTimeConfig`]
//! is **campaign rate policy** that cuts 4/8/11 read; it reaches that function never,
//! and nothing in this crate constructs a `GameClock` or a `WorldTimelineEpoch`.

use crate::pack::RulesPack;

/// The default active-domain limit (10b §7's `MAX_ACTIVE_DOMAINS`). Inert in cut 1.
pub const DEFAULT_ACTIVE_DOMAIN_LIMIT: u32 = 8;

/// The default ground pulse period (10b §7's `GROUND_PULSE_MS`). Inert in cut 1.
pub const DEFAULT_GROUND_PULSE_MS: u32 = 2_000;

/// The default space pulse period (10b §7's `SPACE_PULSE_MS`). Inert in cut 1.
pub const DEFAULT_SPACE_PULSE_MS: u32 = 6_000;

/// The most pack files one server may be configured to load.
///
/// Boot reads every one of them, so the list is bounded for the same reason every
/// other list in this programme is.
pub const MAX_CONFIGURED_PACKS: usize = 64;

/// The campaign's rate and catch-up **policy** (Q8D), parsed and inert in cut 1.
///
/// Doc 10 S1A's "≈ one OOC hour per IC day" is a value here, never a constant in
/// code. Nothing in cut 1 reads a field of this struct; it exists so an operator can
/// write the block once and cuts 4, 8 and 11 find it already parsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TacsimTimeConfig {
    /// IC seconds per real second the campaign layer intends. **Inert:** opening a
    /// generation is `Ctx::change_world_time_rate`'s, and cut 11 is the verb.
    pub rate: u32,
    /// The journey catch-up policy word (10b §3.11: *restamp preserving remaining IC
    /// duration*). **Inert until cut 6.**
    pub catch_up_journeys: String,
    /// The production catch-up policy word (*pause as `Interrupted { TimelineChanged }`*).
    /// **Inert until cut 4.**
    pub catch_up_production: String,
    /// The fronts catch-up policy word (*no restamp — there is nothing to restamp*).
    /// **Inert until cut 9.**
    pub catch_up_fronts: String,
    /// The tactical-domain catch-up policy word (*refuse to tick and page*).
    /// **Inert until cut 8.**
    pub catch_up_domains: String,
}

/// The recovery provider's policy (**E7 residue pass 1, R4**; 10b §15.1 item 9,
/// doc 10 Q7A / T7 / S3A).
///
/// Every field is **live**: it is added in the same pass that builds the provider,
/// so there is no inert knob here. The host validates it before the module sees it
/// (`mush_server::config_ext::tacsim_config`), so a nonsense policy refuses the boot
/// rather than being discovered at the first grant.
///
/// **There is no faucet field, deliberately.** 10b §3.11's replenishment row is *the
/// provider's own production work orders and salvage recovery, funded by its budget —
/// never a mint into the pool*, and this struct is where that would have to be
/// written down to be true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TacsimRecoveryConfig {
    /// Whether the provider grants at all.
    pub enabled: bool,
    /// The provider's budget account id; zero is unwired.
    pub pool_account: u64,
    /// The balance the pool may not be granted below.
    pub pool_floor: i64,
    /// The loaner tier table, cheapest first.
    pub tiers: Vec<String>,
    /// Live applications one principal may hold.
    pub applications_per_principal: u32,
    /// The per-player cooldown after a lease ends, in seconds (10b §7's
    /// `RECOVERY_COOLDOWN` 14 d by default — cited, never redefined).
    pub cooldown_secs: i64,
    /// The eligibility threshold, over **accessible** operational assets.
    pub max_operational_assets: u32,
    /// The insurance policy word.
    pub insurance: String,
    /// The lease's upkeep per campaign cadence period.
    pub upkeep_per_period: i64,
}

impl Default for TacsimRecoveryConfig {
    fn default() -> Self {
        TacsimRecoveryConfig {
            enabled: false,
            pool_account: 0,
            pool_floor: 0,
            tiers: Vec::new(),
            applications_per_principal: 1,
            cooldown_secs: 14 * 24 * 60 * 60,
            max_operational_assets: 0,
            insurance: "approved_operations".to_string(),
            upkeep_per_period: 0,
        }
    }
}

/// The most tiers one deployment may declare. A tier table is a small ordered
/// vocabulary an operator writes by hand, and the grant walks it.
pub const MAX_RECOVERY_TIERS: usize = 8;

/// The insurance policy words `[tacsim.recovery].insurance` admits. A word outside
/// this set **refuses the boot**: an unrecognised policy is a policy nothing applies.
pub const RECOVERY_INSURANCE_WORDS: [&str; 2] = ["approved_operations", "none"];

/// One pack file the host has already read, as `(name, bytes)`.
///
/// **OPEN 1, taken as written: the host reads, the ruleset parses.** The name is
/// whatever the operator wrote in `pack_paths`/`seed_paths` and is what every refusal
/// is reported against, so a boot failure names the file rather than the pack id the
/// file failed to declare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackSource {
    /// The path the operator configured, verbatim.
    pub name: String,
    /// The bytes the host read.
    pub bytes: String,
}

/// The frozen `[tacsim]` block.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TacsimConfig {
    /// Whether the layer does anything. **Live.**
    pub enabled: bool,
    /// The rules-definition packs, already read by the host. **Live.**
    pub packs: Vec<PackSource>,
    /// The campaign-seed packs, already read by the host. **Live.**
    pub seeds: Vec<PackSource>,
    /// Which seed pack `@tacsim/seed` defaults to when given no argument. **Live.**
    pub corridor: String,
    /// Cut 6's `MAX_ACTIVE_DOMAINS`. **Parsed, inert.**
    pub active_domain_limit: u32,
    /// Cut 8's ground pulse period. **Parsed, inert.**
    pub ground_pulse_ms: u32,
    /// Cut 8's space pulse period. **Parsed, inert.**
    pub space_pulse_ms: u32,
    /// Cuts 4/8/11's rate and catch-up policy. **Parsed, inert.**
    pub time: TacsimTimeConfig,
    /// **E7 residue pass 1, R4** — the recovery provider's policy. **Live.**
    pub recovery: TacsimRecoveryConfig,
}

impl TacsimConfig {
    /// Every source the host read, rules first then seeds, in configured order.
    pub fn sources(&self) -> impl Iterator<Item = &PackSource> {
        self.packs.iter().chain(self.seeds.iter())
    }
}

/// A pack the module has parsed and verified, with the file it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPack {
    /// The configured path, for every message that names a file.
    pub source: String,
    /// The verified pack.
    pub pack: RulesPack,
}
