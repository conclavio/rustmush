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
//!
//! `[tacsim.recovery]` is cut 11's and is **not added at all** — a knob that parses
//! and configures nothing is a promise, and cut 11 has not made it yet.
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
