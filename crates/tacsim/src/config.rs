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
//! | all of `[tacsim.arena]`, `[tacsim.match]`, `[tacsim.sensors]`, `[tacsim.resolution]`, `[tacsim.harness]` | parsed, **inert** (16b cut G) | the combat arena's blocks; cuts A/T/S/W/H read and validate them |
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
/// **S7 (doc 10 T17)** — `[tacsim.time.ephemeris]`: the campaign's chosen epoch and
/// the rate its sky moves at. **`rate = 0` is the ruling and the default**: every
/// body's phase is the one the pack authored at the epoch, for ever, so a distance
/// pinned at load is the distance on any IC day. The knob exists so a campaign that
/// wants a moving sky can have one coherently (every pair sharing a body changes
/// together); this campaign does not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EphemerisConfig {
    /// The epoch, IC seconds since the Unix epoch: the instant the packs' phases
    /// are stated at. Only a nonzero rate ever reads it.
    pub epoch: i64,
    /// Permille of the real sky's motion per IC second; 0 is frozen.
    pub rate: u32,
}

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
    /// **S7** — the campaign ephemeris (T17). **Live:** read at every pack load.
    pub ephemeris: EphemerisConfig,
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

// ---- 16b cut G — the combat arena's typed blocks ----------------------------
//
// The twins of `mush_config::config`'s five `[tacsim.*]` sub-tables, mapped by
// `mush_server::config_ext::tacsim_config` and carried on [`TacsimConfig`]. They
// are **parsed and inert** in cut G: nothing in this crate reads one yet, and on
// [`TacsimTimeConfig`]'s rule the cut that reads a field is the cut that
// validates it (A for the arena, T for the match, S for sensors, W for
// resolution, H for the harness).
//
// **Integer only.** Doc 16 §3.1's determinism ruling (D-16-4) is that
// authoritative arena state is integer — centimetres, millidegrees,
// milliseconds, cm/s, per-mille probabilities — so no field below is a float.
// The one configured float, `[tacsim.sensors].max_signature_factor`, becomes
// [`SensorsConfig::max_signature_factor_permille`] once, at the host, where a
// nonsense value can still refuse a boot.

/// The arena's clock, its intent inbox and its ledger bounds (**16b cut G**;
/// doc 16 §3.1 and §4; the values are 16b §9's constants table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaConfig {
    /// The adaptive tempo's periods and holds.
    pub tempo: TempoConfig,
    /// Periods of wall time one pulse may absorb after a stall before the arena
    /// records `lagged` and skips the idle time rather than expanding `dt`
    /// (doc 16 §3.1 "Clock and lag").
    pub max_catch_up_steps: u32,
    /// The per-unit intent-queue bound.
    pub intents: IntentsConfig,
    /// **The least arena time between two deliberation requests for one squad**
    /// (16b cut N.2; doc 16 §6: *event-driven with a floor of
    /// `command.deliberate_min_ms` (30,000)*).
    ///
    /// The trigger is the commander's contact set changing class or count, or an
    /// order expiring; this is the floor under it, and it is the whole of what
    /// stops a firefight from asking for an order every sensor pass. It is
    /// **arena** milliseconds, enforced inside the pulse
    /// ([`crate::arena::command`]), so the same scripted match makes the same
    /// number of requests at every tempo (D-16-2).
    ///
    /// Zero is legal and means "no floor": every trigger asks. Nothing but a
    /// deliberate test should use it, and the cost run says why.
    pub deliberate_min_ms: u32,
    /// The match ledger's cadence and its two hard bounds.
    pub ledger: LedgerConfig,
    /// **The reflex controller's cadence, paths and doctrine** (16b cut N.1).
    pub reflex: ReflexConfig,
}

impl Default for ArenaConfig {
    fn default() -> Self {
        ArenaConfig {
            tempo: TempoConfig::default(),
            max_catch_up_steps: 4,
            intents: IntentsConfig::default(),
            deliberate_min_ms: 30_000,
            ledger: LedgerConfig::default(),
            reflex: ReflexConfig::default(),
        }
    }
}

/// **The reflex controller** (16b cut N.1; doc 16 §6, 16b §8's cut N row).
///
/// Every value is an integer (D-16-4) and every one of them is *doctrine*: what
/// the controller is told about how often to think, how far to search, when to
/// run and what to shoot at. None of it decides an outcome on its own — the same
/// intents a pilot's verbs stage are what leaves the controller — so an operator
/// who retunes this block gets different AI and the same physics.
///
/// **`decide_ms` is a cadence of arena time, not of pulses.** The tick is a
/// [`crate::arena::state::DueKind::ReflexTick`] applied at its own arena
/// timestamp, so the controller runs the same number of times per arena second at
/// every tempo — which is what lets the cross-tempo property hold with AI units
/// on the field (doc 16 §3.1, D-16-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflexConfig {
    /// Arena milliseconds between one AI unit's decisions, staggered by unit id.
    pub decide_ms: u32,
    /// **The A\* node budget** — the most cells one path search may expand before
    /// it gives up and the `unreachable` edge fires (16b §9: *`[atlas]
    /// .path_max_cells` for the arena, 65,536*).
    ///
    /// It is the arena's own copy of `[atlas].path_max_cells` and deliberately
    /// **raised** past the atlas's 4,096: the atlas searches a room graph's
    /// neighbourhood and the arena searches a 256 by 256 window, which is 65,536
    /// cells, so a budget under the window's size would make "unreachable" a fact
    /// about the budget rather than about the ground. The arena cannot read the
    /// atlas's key at pulse time (D-16-1: no window, no records), so the value
    /// crosses at wiring like every other.
    pub path_max_cells: u32,
    /// The most path searches the controller may run in one `decide_ms` window of
    /// arena time. A unit refused by this budget keeps the path it has.
    ///
    /// A budget per *pulse* would be a throttle whose bite depends on the tempo,
    /// which D-16-2 forbids; a budget per window of arena time bites the same at
    /// every cadence. `[tacsim.harness].max_path_queries_per_pulse` stays what its
    /// own block says it is — a threshold the harness asserts.
    pub max_path_queries: u32,
    /// The least arena time between two path searches by **one** unit, so a unit
    /// that cannot reach its goal cannot spend the whole budget rediscovering
    /// that every tick.
    pub repath_ms: u32,
    /// Armour plus internals, per mille of the authored totals, below which a
    /// unit withdraws (doc 16 §6: *below a health threshold, toward the entry
    /// zone*).
    pub withdraw_permille: u32,
    /// The utility weight on range to a candidate hostile (doc 16 §6). Lower
    /// scores win, and every term is per mille before it is weighted.
    pub weight_range: u32,
    /// The utility weight on the facet the candidate presents: a target showing
    /// its rear scores better than one showing its front.
    pub weight_facet: u32,
    /// The utility weight on the chooser's **own** health. It is the same addend
    /// for every candidate, so it never changes which target is picked; it is in
    /// the score because doc 16 §6 puts it there, and because it is the number the
    /// `Withdraw` edge reads.
    pub weight_health: u32,
    /// The fraction of the weapon's reach `Engage` holds, per mille: the
    /// controller closes to this band and stops rather than driving onto the
    /// target's guns.
    pub engage_band_permille: u32,
}

impl Default for ReflexConfig {
    fn default() -> Self {
        ReflexConfig {
            decide_ms: 1_000,
            // 16b §9: the arena's raised `path_max_cells`, one node per cell of the
            // 256 by 256 window.
            path_max_cells: 65_536,
            max_path_queries: 64,
            repath_ms: 4_000,
            withdraw_permille: 350,
            weight_range: 100,
            weight_facet: 40,
            weight_health: 30,
            engage_band_permille: 700,
        }
    }
}

/// The adaptive tempo (**16b cut G**; doc 16 §3.1's tempo table, its "Rate
/// independence" and "Failure" paragraphs; D-16-2).
///
/// The tempo decides only how often the arena steps and how its output is
/// batched, never an outcome. Every value is arena milliseconds except
/// `alert_range_cm`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempoConfig {
    /// The `Quiet` period: no hostile contact held, no pending impact, no fire
    /// intent, no controller approaching.
    pub quiet_ms: u32,
    /// The `Alert` period.
    pub alert_ms: u32,
    /// The `Engaged` period — the tempo the harness's p99 ceiling is measured in.
    pub engaged_ms: u32,
    /// Milliseconds with none of the `Alert` triggers before falling back to
    /// `Quiet`.
    pub alert_hold_ms: u32,
    /// Milliseconds since the last shot, impact or in-range pair before leaving
    /// `Engaged`.
    pub engaged_hold_ms: u32,
    /// The held-contact range that raises `Alert`, centimetres. **Zero means
    /// derived**: twice the longest weapon range in the match, at match start
    /// (16b §9).
    pub alert_range_cm: u32,
    /// The longest substep a pulse is cut into; the substep is
    /// `min(substep_max_ms, cell_cm × 1000 / max_speed_cms)` floored at 50 ms, so
    /// no unit advances more than one cell in a substep.
    pub substep_max_ms: u32,
    /// Milliseconds one pulse may take before the watchdog voids the match.
    pub stall_ms: u32,
}

impl Default for TempoConfig {
    fn default() -> Self {
        TempoConfig {
            quiet_ms: 5_000,
            alert_ms: 1_000,
            engaged_ms: 500,
            alert_hold_ms: 30_000,
            engaged_hold_ms: 10_000,
            // 0 = derived at match start (16b §9).
            alert_range_cm: 0,
            substep_max_ms: 250,
            stall_ms: 2_000,
        }
    }
}

/// The bound on the arena's per-unit intent queue (**16b cut G**; doc 16 §3.1,
/// pulse phase 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentsConfig {
    /// The most discrete actions one unit may have queued at once. Continuous
    /// controls are latest-wins and do not queue.
    pub max_pending_per_unit: u32,
}

impl Default for IntentsConfig {
    fn default() -> Self {
        IntentsConfig {
            max_pending_per_unit: 8,
        }
    }
}

/// The match ledger's cadence and its two hard bounds (**16b cut G**; doc 16 §4,
/// §3.1's egress paragraph).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerConfig {
    /// Arena milliseconds between `position_summary` events — every unit, in
    /// every tempo.
    pub summary_ms: u32,
    /// The most bytes one match's ledger may reach (64 MiB): at 90% summaries
    /// stop, at 100% the match is stopped with `Timeout`.
    pub max_bytes: u64,
    /// The most 16 KiB chunks the bounded egress ring may hold before the match
    /// is stopped with `Void`.
    pub max_unflushed_chunks: u32,
}

impl Default for LedgerConfig {
    fn default() -> Self {
        LedgerConfig {
            summary_ms: 5_000,
            // 64 MiB.
            max_bytes: 67_108_864,
            max_unflushed_chunks: 256,
        }
    }
}

/// The match lifecycle's slots and timeouts (**16b cut G**; doc 16 §3.3
/// "Startup", §3.8 "Lifecycle", §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    /// Milliseconds `STARTUP` takes to carry a unit's power from `starting` to
    /// `online`; until then motion and fire intents refuse.
    pub startup_ms: u32,
    /// Milliseconds `Boarding` waits for every claimed slot to be seated before
    /// unseated slots are dropped (an empty side aborts).
    pub board_timeout_ms: u32,
    /// The longest a match may run before it ends as a timeout.
    pub max_duration_ms: u32,
    /// Pilot slots one side may claim (doc 16 §1's 12 v 12).
    pub max_slots_per_side: u32,
    /// **Whether `SIM START` also fields the tanks of the pods nobody claimed**,
    /// under the reflex controller (**16b cut N.1**; doc 16 §6's *the AI fill of a
    /// later lobby*).
    ///
    /// Off by default, and `mush_config`'s own row says why at length: doc 16 §1's
    /// A1 is a two-tank story and a default-on fill would rewrite it from a cut
    /// that does not own it. The mechanism, the attribute and the check-out rule
    /// that reads it are all here and all live; this is the switch.
    pub ai_fill: bool,
}

impl Default for MatchConfig {
    fn default() -> Self {
        MatchConfig {
            startup_ms: 10_000,
            board_timeout_ms: 120_000,
            // One hour.
            max_duration_ms: 3_600_000,
            max_slots_per_side: 12,
            ai_fill: false,
        }
    }
}

/// The sensor pass's cadence and its signature ceiling (**16b cut G**; doc 16
/// §3.6 "Perception").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorsConfig {
    /// Arena milliseconds between one unit's sensor passes, staggered by unit id.
    pub pass_ms: u32,
    /// The ceiling on a target's signature factor, **per mille**: the configured
    /// `1.5` arrives here as `1500`.
    ///
    /// Doc 16 §3.1 (D-16-4): the engine stores nothing in floating point, so the
    /// `[tacsim.sensors].max_signature_factor` float is converted exactly once,
    /// in `mush_server::config_ext::tacsim_config`, where a negative, `NaN` or
    /// absurd factor is a hard boot error naming that key.
    pub max_signature_factor_permille: u32,
    /// **Passes a contact may go unrefreshed before it is lost** (16b cut S.1;
    /// doc 16 §3.6 step 4's TTL). The time-to-live is
    /// `pass_ms × lost_after_passes`, so the loss band is stated in *passes* and
    /// follows the cadence rather than drifting from it when `pass_ms` moves.
    /// Zero is read as one: a contact lost the instant it is not refreshed is a
    /// contact nothing can hold.
    pub lost_after_passes: u32,
    /// **Name a contact by team and slot the moment it is detected** (16b cut
    /// Q.1), instead of by a provisional `X<n>` until it is classified.
    ///
    /// Off, and off is doc 16's reading: W5 is that identification is *earned*,
    /// and §5 is that nothing in a frame identifies another unit before it has
    /// been told apart. On, a pilot reads `BA` at the first pass —
    /// `classify_after_ms` earlier than the rule allows — so the team is legible
    /// off a return the platform has not resolved, on four surfaces at once (the
    /// acquired notice, `CONTACTS`, the map label and a panel's list). The
    /// classification still happens in the arena; only the *naming* stops waiting
    /// for it.
    ///
    /// It is a configuration key and not a constant so that the owner can try the
    /// other reading on a box with a server-child restart and no build.
    pub designate_on_detection: bool,
    /// **At or above this fraction of what it holds whole, a section reads
    /// `green`** (16b cut Q.2; doc 16 §3.6), per mille.
    ///
    /// The three keys below are what `SCAN` turns the M32 cells into
    /// ([`crate::arena::condition::Condition::of`]). They are configuration and
    /// not a Rust table for the reason [`SensorsConfig::designate_on_detection`]
    /// is — the owner tunes them on a box with a server-child restart and no
    /// build — and not `resolution.json` because shipped content is compiled into
    /// the binary and an operator pack cannot override a shipped id, so only
    /// configuration is tunable at all.
    ///
    /// Every reader takes it `.min(1000)`, so a key above a whole section makes
    /// everything below `green` and can never make the arithmetic overflow.
    pub condition_green_permille: u32,
    /// **At or above this fraction, a section reads `yellow`** (16b cut Q.2), per
    /// mille; below it, `red`. Read `.min(1000)` for the reason above.
    pub condition_yellow_permille: u32,
    /// **How near a target has to be before a pass reads its bands at all** (16b
    /// cut Q.2; doc 16 §2 W5), as a fraction of the effective reach the range test
    /// just used — the ground's signature factor, the thermal term and the jamming
    /// scale all already in it.
    ///
    /// A *close look*: a per-section reading at the very edge of detection would
    /// be a bigger purchase than W5 prices identification at, so it costs one more
    /// comparison per detection that has already passed every other test, and a
    /// concealed or jammed target is that much harder to read for the same reason
    /// it is harder to see. Read `.min(1000)`, so the widest this key can be is
    /// *anything the platform can detect*.
    pub condition_reach_permille: u32,
}

impl Default for SensorsConfig {
    fn default() -> Self {
        SensorsConfig {
            pass_ms: 1_000,
            // 1.5 × 1000.
            max_signature_factor_permille: 1_500,
            lost_after_passes: 3,
            designate_on_detection: false,
            // Two thirds and a third, so the three live bands are three roughly
            // equal thirds of a section.
            condition_green_permille: 667,
            condition_yellow_permille: 334,
            // Half the reach the range test just used.
            condition_reach_permille: 500,
        }
    }
}

/// Where the hit-resolution tables live (**16b cut G**; doc 16 §4's
/// `resolution.json` row).
///
/// The tables are the hit curve, the error terms, `p_min` and `p_max` — the
/// per-mille no-dice resolution of D-16-9. **The table itself is live since 16b
/// cut W.2** (`crate::match_::resolution_for` is what a check-out reads it
/// through); this block's `paths` override is **still inert**, because cut C.2
/// shipped the table inside the crate's own content pack and the pack loader is
/// what finds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolutionConfig {
    /// Resolution-table files, or directories searched for them. **Nothing reads
    /// it yet**: the shipped table travels in
    /// `crate::definitions::content::SOURCES` with the rest of the game's content,
    /// and an override lane for it is a cut of its own.
    pub paths: Vec<String>,
}

/// The scale harness's shape and its thresholds (**16b cut G**; doc 16 §1's
/// acceptance A2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessConfig {
    /// Units the harness places and moves.
    pub units: u32,
    /// How many of those are fighting.
    pub fighting: u32,
    /// Weapons on their cadences.
    pub weapons: u32,
    /// The per-pulse p99 ceiling, milliseconds, in `Engaged` tempo.
    pub engaged_p99_ms: u32,
    /// The most line-of-sight traces one pulse may make.
    pub max_traces_per_pulse: u32,
    /// The most path queries one pulse may make.
    pub max_path_queries_per_pulse: u32,
    /// **The most contact-share operations one pulse may make** (16b cut S.1;
    /// doc 16 §2 W6, §3.6 step 5: *share operations bounded per pulse*). One
    /// operation is one `(sharer, teammate)` pair considered — its range test and
    /// its line of sight — so the bound is on the fan-out of the comms mesh and
    /// not on how many contacts a pair happens to carry.
    ///
    /// Like every other key in this block it is a **threshold the harness
    /// asserts**, never a throttle the arena applies: an engine that stopped
    /// sharing halfway through a pulse would make the outcome a function of the
    /// tempo, which is exactly what D-16-2 forbids.
    pub max_share_ops_per_pulse: u32,
    /// The most bytes the streamed ledger may reach over the harness's hour of
    /// arena time (64 MiB) — the proof that memory stays flat because the ledger
    /// leaves.
    pub max_ledger_bytes: u64,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        HarnessConfig {
            units: 300,
            fighting: 24,
            weapons: 150,
            engaged_p99_ms: 25,
            max_traces_per_pulse: 4_000,
            max_path_queries_per_pulse: 64,
            max_share_ops_per_pulse: 256,
            // 64 MiB.
            max_ledger_bytes: 67_108_864,
        }
    }
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
    /// **E7 residue pass 1, R4** — the recovery provider's policy. **Live.**
    pub recovery: TacsimRecoveryConfig,
    /// **16b cut G** — the arena's clock, intent inbox and ledger bounds.
    /// **Parsed, inert** until cut A.
    pub arena: ArenaConfig,
    /// **16b cut G** — the match lifecycle's slots and timeouts. **Parsed,
    /// inert** until cut T. `match_` because `match` is a keyword; the TOML key
    /// is `match`.
    pub match_: MatchConfig,
    /// **16b cut G** — the sensor pass's cadence and its per-mille signature
    /// ceiling. Live since cut S.1.
    pub sensors: SensorsConfig,
    /// **16b cut G** — where the resolution tables live. The table is live since
    /// cut W.2; this block's `paths` override is **parsed, inert** (see
    /// [`ResolutionConfig`]).
    pub resolution: ResolutionConfig,
    /// **16b cut G** — the scale harness's shape and thresholds. **Parsed,
    /// inert** until cut H.
    pub harness: HarnessConfig,
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
