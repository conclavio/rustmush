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
//! | all of `[tacsim.command]` | **live** (16b cut Y.2) | doc 16 W8's span of control and order rate, held at the world where `ORDER` is admitted |
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

use std::collections::BTreeMap;

use crate::definitions::UnitClass;
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
    /// **Arena milliseconds a unit holds against another unit in its way, keeping
    /// its speed, before it stops and says so** (16b cut E.3; 16c `B-46`, §5's
    /// Q10).
    ///
    /// Doc 16 §3.4 step 4 stops a unit dead at every refused crossing. That is the
    /// rule for ground, and it is what breaks a column: a follower whose id sorts
    /// below its leader's integrates first, reaches the cell the leader has not
    /// vacated yet, and is stopped — at the ford, every time, needing a dozen
    /// seconds to rebuild way and teaching its machine to avoid the cell its own
    /// leader was about to leave. So an occupancy refusal is *held against* for
    /// this long ([`crate::arena::state::UnitState::occupied_hold`]) and only then
    /// becomes the literal stop, with its `blocked` line, its pilot notice and its
    /// lesson. Impassable ground, the window's edge and a slope are untouched.
    ///
    /// **Zero is the rule as it stood before the cut**: every occupancy refusal is
    /// a stop at once. Two seconds is long enough for a column of up to eight to
    /// be released one substep per unit and short enough that a real obstacle — a
    /// wreck in the ford — is learned while the match is still worth playing.
    pub occupied_stop_ms: u32,
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
    /// **How many machines one squad holds at most** (16b cut Z.1; 16c N6,
    /// *lances*; doc 16 §6 "Deliberation": *requested per squad or commander*).
    ///
    /// A side's reflex units are **dealt** into `ceil(n / squad_size)` squads
    /// whose sizes differ by at most one, consecutive in unit-id order, so five
    /// machines at four are a three and a two and never a four and a one; past
    /// `MAX_SQUADS_PER_SIDE` full squads the rest fly doctrine
    /// ([`crate::deliberation::squads_of`] is the whole rule). **Both** callers
    /// of `squads_of` read it from here — the match record's grouping at
    /// check-out and `Arena::new`'s own — so the two groupings of one match are
    /// one function of one data.
    ///
    /// Four, because twelve machines a side is an opponent only in lances of
    /// four; `1..=`[`crate::deliberation::SQUAD_SIZE_MAX`] by
    /// [`ArenaConfig::validate`].
    pub squad_size: u32,
    /// **The most machines of one class a side fields** (16b cut AE.3; doc 16
    /// §2 W2: *no drone swarms; `arena.max_units_by_class`; class counts on the
    /// map obey the caps*).
    ///
    /// A side fields at most this many units of a class. **Per side and not per
    /// map**, because both sides may field one (Q-AE4, an amendment to W2's "on
    /// the map"); a class absent from the map is **uncapped**; and a value of
    /// **zero** is legal and means the class cannot be fielded on this server.
    /// The drone's `1` is the compiled default and stands unless the operator
    /// names the drone: `mush_server::config_ext` lays the operator's entries
    /// over [`ArenaConfig::default`]'s map, so a table naming only `walker`
    /// does not lift it.
    ///
    /// A `BTreeMap` (never a `HashMap`): `Arena::new` walks it beside its own
    /// ordered count, and the lobby reads one entry. `0..=`
    /// [`crate::definitions::arena::MAX_ARENA_UNITS`] by [`ArenaConfig::validate`].
    pub max_units_by_class: BTreeMap<UnitClass, u32>,
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
            occupied_stop_ms: 2_000,
            intents: IntentsConfig::default(),
            deliberate_min_ms: 30_000,
            squad_size: 4,
            // 16b cut AE.3: N15's "drones as content: one, capped".
            max_units_by_class: BTreeMap::from([(UnitClass::Drone, 1)]),
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
    /// **How long a withdrawn unit waits, with nothing hostile on its board,
    /// before it comes back into the fight** (16b cut Q.7; doc 16 §6), arena
    /// milliseconds.
    ///
    /// `Withdraw` used to be terminal: a hurt tank parked in its entry zone and
    /// the match ran to `match.max_duration_ms`. It rallies once now — the health
    /// edge is not taken a second time — so this is the length of the lull a
    /// crew needs before it turns round, and not a respawn timer.
    ///
    /// It is validated at or above `decide_ms`: a rally shorter than one decision
    /// would fire on the tick after the one that withdrew.
    pub rally_ms: u32,
    /// **Armour plus internals, per mille, below which a machine still on its way
    /// home will not turn and fight even something close** (16b cut E.1; doc 16
    /// §6; 16c §5's Q8), arena doctrine like the rest of this block.
    ///
    /// The stand a `Withdraw` makes has two rules and this is the floor under the
    /// first: **en route** it turns for a hostile inside its `close` band and
    /// above this figure, and for nothing else; **once home** it shoots back at
    /// whatever comes inside its firing ceiling, at any health, because a machine
    /// that has arrived has nowhere further to run.
    ///
    /// Validated `0` to `1000`, **and not above [`ReflexConfig::withdraw_permille`]**:
    /// a machine only runs home below the second, so a floor above it would be a
    /// doctrine branch no unit could ever be in. Every shipped file carries 350
    /// there, so the default of 175 — half of it — is well clear.
    pub stand_min_permille: u32,
    /// **How long a machine keeps driving to where it last saw its target**, arena
    /// milliseconds (16b cut E.4; 16c `B-04`; doc 16 §6's `lost` edge).
    ///
    /// A contact expires after three sensor passes, and before this cut the `lost`
    /// edge dropped the machine straight back to `Patrol` — whose waypoint walk is
    /// a fixed cycle per unit id, and may lead directly away from the tank that
    /// has just driven behind a tree line. A crew that loses a contact drives to
    /// the last place it had one; so does the machine now.
    ///
    /// **`0` turns the search off**, which is the pre-E.4 rule exactly. Otherwise
    /// it is validated at or above `decide_ms` and at most ten minutes. The floor
    /// is exact and not a round number: the plot is written by the last tick that
    /// held the target and the `lost` edge is read one decide tick later, so the
    /// first `Patrol` tick always finds it `decide_ms` old. The age is compared
    /// inclusively, so at the floor the machine drives at it for that one tick;
    /// anything under the floor would drop it unread and be `0` under another
    /// name.
    pub search_ms: u32,
    /// **How long a patrolling machine that holds nothing hostile waits before it
    /// crosses to the other side's entry zone to look**, arena milliseconds (16b
    /// cut E.4; 16c `B-04`).
    ///
    /// The waypoint walk is deterministic per unit id, so two sides' loops need
    /// never come within a sight of each other — and a withdrawn machine parks in
    /// its own entry zone, which no patrol loop passes through except at the two
    /// road ends. That is the match that goes quiet for ever, and it was measured
    /// at HEAD before this key existed: a dry Red tank parked on its entry cell
    /// from 30 s of arena time, a Blue tank walking the east half of the map, and
    /// nothing at all for the next fifteen minutes.
    ///
    /// The other side's entry zone is the one place on the map a beaten enemy is
    /// certain to be, and it is **map knowledge** — the arena definition's own
    /// zones, never any unit's pose — so a machine that goes there has not been
    /// told anything about anybody (doc 16 §1).
    ///
    /// **`0` turns the hunt off.** Otherwise it is validated at or above
    /// `decide_ms`.
    pub hunt_after_ms: u32,
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
    /// derived**: the lesser of twice the longest weapon range in the match and
    /// the window's own diagonal, at match start (16b §9; the cap is 16b cut
    /// E.1's, 16c `T-32`).
    pub alert_range_cm: u32,
    /// The longest substep a pulse is cut into; the substep is
    /// `min(substep_max_ms, (cell_cm - 1) × 1000 / max_speed_cms)` floored at
    /// 50 ms, so one substep's travel — the motion carry (16b cut Q.6)
    /// included, because the carry is added *inside* that one division — is at
    /// most `cell_cm - 1`, and no unit advances more than one cell in a substep.
    /// At most `engaged_ms` ([`ArenaConfig::validate`], 16b cut I.3), so a staff
    /// tempo floor always moves arena time.
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

/// The rate a seat's discrete orders are held to (**16b cut G**; doc 16 §3.1,
/// pulse phase 2; a rate since **16b cut M.1**, `T-82`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentsConfig {
    /// **The discrete orders one seat may give a unit per arena second; also the
    /// size, per unit, of the world side's inbox and pre-pulse buffer** (16b cut
    /// M.1, `T-82`). Counted in arena time, so the burst a pilot may send does
    /// not depend on the tempo; the reflex controller's own orders are never
    /// counted. Continuous controls are latest-wins and do not count.
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
    /// **How long a `Running` match every pilot has left runs on before it ends
    /// itself** (16b cut D.3, `B-33`), in **world** milliseconds; `0` turns it
    /// off.
    ///
    /// The lobby stamps the moment the last un-`left` slot goes `left` and its
    /// 1 s tick sends `StopReason::Unattended` once the interval has passed. A
    /// match with no slots at all — a staff `@arena/start` — is never unattended,
    /// because there was nobody to leave.
    pub unattended_ms: u32,
    /// **How long after a drop a pilot must wait before `BOARD` takes the tank
    /// back** (16b cut Y.3; 16c `N4`; doc 07 §1.3 "Reclaiming control"), in
    /// **world** milliseconds.
    ///
    /// A drop — the linkdead grace running out, or a `QUIT` — hands the tank
    /// to the reflex controller with a `Control { Released }`; the reclaim is a
    /// `Control { Live }` at a newer epoch. The floor does two things: it
    /// closes flapping (a line that drops and comes back every second does not
    /// hand the tank to the machine and back every second), and it gives the
    /// arena its pulse to process the `Released` before the `Live` arrives —
    /// which the epoch table would settle anyway (`apply_control`'s stale
    /// gate), but a pilot should not be the one who finds out.
    pub reclaim_min_ms: u32,
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
            // Five minutes: long enough that a pilot who drops and reconnects
            // has not ended their own match, short enough that a centre is not
            // held for the hour `max_duration_ms` would take.
            unattended_ms: 300_000,
            // Five seconds: one `Quiet` pulse, so the arena has processed the
            // `Released` before the `Live` is sent.
            reclaim_min_ms: 5_000,
        }
    }
}

/// **The span of control and the order rate** (**16b cut Y.2**; doc 16 §2 W8,
/// *the Compact caps span of control and needs a human trigger*).
///
/// Two numbers, and they bound different things. `max_span` is **concurrency**:
/// how many squads one issuer may hold a live order on at the same moment, read
/// off the world's own record of whose order each squad is under. A rate alone
/// could not say it — twelve orders a minute to twelve different squads is a
/// commander of twelve. `orders_per_minute` is the **floor under the human
/// trigger**: a span of four re-ordered every pulse is a bot typing, not a
/// commander. So the one is a set's size and the other a sliding window of
/// stamps.
///
/// **Both live at the world**, where `ORDER` is admitted (`cockpit::order`),
/// and never in the arena: a refused order reaches no pulse, so neither number
/// can change an outcome, and the tempo that decides nothing (doc 16 §3.1) still
/// decides nothing. The span and the window are both counted in world
/// milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandConfig {
    /// The most squads one issuer may hold a **live** order on at once. A
    /// replacement of a squad's own live order never counts against it.
    pub max_span: u32,
    /// The most `ORDER`s one issuer may give in sixty world seconds; one
    /// `ORDER ALL` is one.
    pub orders_per_minute: u32,
}

impl Default for CommandConfig {
    fn default() -> Self {
        CommandConfig {
            // Doc 16 W8's own figure: a lead and four machines' squads is a
            // command a human can hold in their head.
            max_span: 4,
            // One every five seconds, sustained: the deliberation cadence's own
            // order of magnitude, and far past what a pilot who is also flying
            // types.
            orders_per_minute: 12,
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
    /// `1.0` arrives here as `1000`.
    ///
    /// **16b cut M.4** (16c `B-55`, `T-26`): the default is `1_000` (it was
    /// `1_500`), and the host refuses a factor below 1.0. The perception pass
    /// scales its candidate cull by this, and the range test inside the cull is
    /// never longer than `max_range × 1.0` (no signature row exceeds 1.0), so
    /// `1_000` is the exact cull and anything less would drop contacts the sensor
    /// can see.
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
            // 1.0 × 1000 (16b cut M.4; 1.5 until then).
            max_signature_factor_permille: 1_000,
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
///
/// No weapon count (16b cut U.3, 16c `T-37`): the harness fields the catalog
/// armed as authored, so the cohort decides what is armed; the `weapons` field
/// it once carried was read by no lane and went with its knob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessConfig {
    /// Units the harness places and moves.
    pub units: u32,
    /// How many of those are fighting.
    pub fighting: u32,
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
    ///
    /// 768 since 16b cut J.3b, on the authored lane's measured per-wake peak of
    /// 452 at 300 units and the 30-cell mesh (16c Q26, the owner's answer); 256
    /// was S.1's figure from the CI cohort's reduced mesh. The figure is three
    /// halves of the peak, 678, rounded up to the next multiple of 256.
    pub max_share_ops_per_pulse: u32,
    /// The most bytes the streamed ledger may reach over the harness's hour of
    /// arena time (64 MiB) — the proof that memory stays flat because the ledger
    /// leaves.
    pub max_ledger_bytes: u64,
    /// **The most cells one pulse's line-of-sight traces may read** (16b cut U.2,
    /// 16c `T-36`; doc 16 §2 W4: *traces and cell probes per pulse*), detection's
    /// and the share mesh's together, asserted against the worst wake of the CI
    /// cohort.
    ///
    /// A threshold the harness asserts, never a throttle the arena applies: a pass
    /// that stopped tracing when a budget ran out would make what a pilot sees a
    /// function of the tempo, which D-16-2 forbids.
    pub max_cell_probes_per_pulse: u32,
    /// **The most index candidates one pulse's jammer scans may look at** (16b cut
    /// U.2, 16c `T-29`; doc 16 §9 item 4), asserted against the worst wake. The
    /// scan runs once per unit between two clears of the arena's jamming memo,
    /// and the memo is cleared at the substep's two index rebuilds and after
    /// **every** `StartupComplete` and **every** `Impact`, whoever it touched —
    /// so under fire the count grows with the impacts a substep resolves, and
    /// this bound is sized on a fight, not on the unit count alone. Like every
    /// key here it is a threshold and not a throttle.
    pub max_jam_checks_per_pulse: u32,
    /// **How far resident memory may grow over the harness's long run**,
    /// kibibytes (16b cut U.2, 16c `T-36`): the reading after the warm-up against
    /// the reading at the end of `TACSIM_LONG_RUN_S` arena seconds. The claim is
    /// that memory stays flat because the ledger leaves.
    pub max_rss_growth_kib: u32,
    /// **The largest encoded `arena.cockpit` keyframe the fan-out lane may see**,
    /// bytes (16b cut U.2, 16c `T-36`; 16b §11's fan-out risk): what one pilot's
    /// socket is handed in one pulse. The lane measures the opening — the two
    /// musters stand 2.4 km apart, past every stock sight — so the frames it saw
    /// carry teammates only; a combat frame (enemy rows, a lock, readings) is
    /// not yet measured against this bound.
    pub max_frame_bytes: u32,
    /// **The most output events one arena pulse may produce through the world's
    /// output path** (16b cut U.2, 16c `T-36`), counted by the fan-out lane on the
    /// one apply that carried that pulse's frames: every pilot's keyframe and
    /// every sentence its notices rendered. It is sized against the lane's first
    /// measured burst, the acquisition pass after power-up (24 `arena.cockpit`
    /// panels plus 11 contact-notice lines per pilot = 288 events), and set to 512
    /// so a 24-pilot burst has headroom; whoever moves it next must say why. That
    /// burst is teammates only — no enemy is in sight in the lane's thirty
    /// seconds — so an engaged pulse's fire and damage notices are unmeasured.
    pub max_output_events_per_pulse: u32,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        HarnessConfig {
            units: 300,
            fighting: 24,
            engaged_p99_ms: 25,
            max_traces_per_pulse: 4_000,
            max_path_queries_per_pulse: 64,
            // 16b cut J.3b (16c Q26): the authored lane's 452 peak, three halves
            // of it rounded up to the next multiple of 256.
            max_share_ops_per_pulse: 768,
            // 64 MiB.
            max_ledger_bytes: 67_108_864,
            // 16b cut U.2: the five bounds 16c `T-36` and `T-29` asked for.
            max_cell_probes_per_pulse: 32_000,
            max_jam_checks_per_pulse: 2_048,
            // 64 MiB.
            max_rss_growth_kib: 65_536,
            // 16 KiB.
            max_frame_bytes: 16_384,
            // The measured burst is 288 (24 panels + 11 lines per pilot), on
            // the opening with no enemy in sight; an engaged pulse is unmeasured.
            max_output_events_per_pulse: 512,
        }
    }
}

// ---- 16b cut Q.7 — the values, checked at the host ------------------------
//
// Doc 16 §7 and 16b §9 say what every key above *means*; until this cut nothing
// said what it may **be**. The blocks are all live now — cut A reads the arena's,
// T the match's, S and Q.1/Q.2 the sensors', H the harness's — so
// [`TacsimTimeConfig`]'s rule ("the cut that reads a field is the cut that
// validates it") has come due for all four at once, and a `[tacsim.arena.tempo]
// .engaged_ms = 0` that booted clean and froze arena time is what it cost not to
// have them.
//
// Four pure functions, no I/O, called once by
// `mush_server::config_ext::tacsim_config` on the way past, so a nonsense value
// **refuses the boot naming its key** rather than being discovered at the first
// match. They are deliberately NOT called from `Arena::new`: a test rig that
// builds a `TacsimConfig` by hand bypasses the mapper, and odd values are how
// half this crate's fixtures make an edge visible in two pulses.
//
// The message shape is one sentence, the TOML key first and the value last:
// `[tacsim.arena.tempo].engaged_ms must be at least 50 (it is 0)`.

/// One refusal, in the shape every message in this section takes. `key` is the
/// whole TOML key, written as an operator would find it in the file.
fn at_least(key: &str, floor: u64, value: u64) -> Result<(), String> {
    if value < floor {
        return Err(format!("{key} must be at least {floor} (it is {value})"));
    }
    Ok(())
}

/// One refusal for a value outside an inclusive range.
fn between(key: &str, low: u64, high: u64, value: u64) -> Result<(), String> {
    if value < low || value > high {
        return Err(format!("{key} must be {low} to {high} (it is {value})"));
    }
    Ok(())
}

impl ArenaConfig {
    /// **Check `[tacsim.arena]` and its four sub-tables** (16b cut Q.7).
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        let floor = u64::from(crate::arena::clock::MIN_SUBSTEP_MS);
        for (key, value) in [
            ("quiet_ms", self.tempo.quiet_ms),
            ("alert_ms", self.tempo.alert_ms),
            ("engaged_ms", self.tempo.engaged_ms),
        ] {
            at_least(
                &format!("[tacsim.arena.tempo].{key}"),
                floor,
                u64::from(value),
            )?;
        }
        // The three periods are a ladder: a tempo that got *slower* as the arena
        // got busier would be the table of doc 16 §3.1 read upside down.
        if self.tempo.quiet_ms < self.tempo.alert_ms || self.tempo.alert_ms < self.tempo.engaged_ms
        {
            return Err(format!(
                "[tacsim.arena.tempo] must have quiet_ms >= alert_ms >= engaged_ms (they are {}, \
                 {}, {})",
                self.tempo.quiet_ms, self.tempo.alert_ms, self.tempo.engaged_ms
            ));
        }
        at_least(
            "[tacsim.arena.tempo].substep_max_ms",
            floor,
            u64::from(self.tempo.substep_max_ms),
        )?;
        // **16b cut I.3** (16c N10): a substep no longer than the engaged
        // period. A staff floor is a multiple of `engaged_ms` and every floored
        // pulse is cut back to the substep lattice
        // (`crate::arena::clock`'s head, rule 2), so a substep longer than the
        // shortest wake would cut every floored wake back to nothing and a
        // floor would stop arena time — the pause it must never be. Under this
        // rule every on-time floored wake moves at least one substep.
        if self.tempo.substep_max_ms > self.tempo.engaged_ms {
            return Err(format!(
                "[tacsim.arena.tempo] must have substep_max_ms <= engaged_ms (they are {}, {})",
                self.tempo.substep_max_ms, self.tempo.engaged_ms
            ));
        }
        at_least(
            "[tacsim.arena.tempo].alert_hold_ms",
            1,
            u64::from(self.tempo.alert_hold_ms),
        )?;
        at_least(
            "[tacsim.arena.tempo].engaged_hold_ms",
            1,
            u64::from(self.tempo.engaged_hold_ms),
        )?;
        // A watchdog that voids a match sooner than one pulse of the fastest
        // tempo would void every busy match on its first pulse.
        at_least(
            "[tacsim.arena.tempo].stall_ms",
            u64::from(self.tempo.engaged_ms),
            u64::from(self.tempo.stall_ms),
        )?;
        at_least(
            "[tacsim.arena].max_catch_up_steps",
            1,
            u64::from(self.max_catch_up_steps),
        )?;
        // **16b cut E.3**: 0 is the pre-cut rule (every occupancy refusal is a
        // stop at once) and a minute of pushing at a wreck before anybody is told
        // is a match nobody can read.
        between(
            "[tacsim.arena].occupied_stop_ms",
            0,
            60_000,
            u64::from(self.occupied_stop_ms),
        )?;
        // **16b cut Z.1** (16c N6): a squad of none commands nothing, and a squad
        // past sixteen is a side and not a squad.
        between(
            "[tacsim.arena].squad_size",
            1,
            u64::from(crate::deliberation::SQUAD_SIZE_MAX),
            u64::from(self.squad_size),
        )?;
        // **16b cut AE.3** (doc 16 §2 W2): zero forbids the class on this
        // server; past the arena's own unit bound a cap caps nothing.
        for (class, cap) in &self.max_units_by_class {
            between(
                &format!("[tacsim.arena.max_units_by_class].{}", class.as_str()),
                0,
                crate::definitions::arena::MAX_ARENA_UNITS as u64,
                u64::from(*cap),
            )?;
        }
        at_least(
            "[tacsim.arena.intents].max_pending_per_unit",
            1,
            u64::from(self.intents.max_pending_per_unit),
        )?;
        at_least(
            "[tacsim.arena.ledger].summary_ms",
            1_000,
            u64::from(self.ledger.summary_ms),
        )?;
        at_least(
            "[tacsim.arena.ledger].max_bytes",
            1_048_576,
            self.ledger.max_bytes,
        )?;
        at_least(
            "[tacsim.arena.ledger].max_unflushed_chunks",
            1,
            u64::from(self.ledger.max_unflushed_chunks),
        )?;
        at_least(
            "[tacsim.arena.reflex].decide_ms",
            50,
            u64::from(self.reflex.decide_ms),
        )?;
        at_least(
            "[tacsim.arena.reflex].path_max_cells",
            1,
            u64::from(self.reflex.path_max_cells),
        )?;
        at_least(
            "[tacsim.arena.reflex].max_path_queries",
            1,
            u64::from(self.reflex.max_path_queries),
        )?;
        between(
            "[tacsim.arena.reflex].withdraw_permille",
            0,
            1_000,
            u64::from(self.reflex.withdraw_permille),
        )?;
        between(
            "[tacsim.arena.reflex].engage_band_permille",
            1,
            1_000,
            u64::from(self.reflex.engage_band_permille),
        )?;
        // 16b cut Q.7: a rally shorter than one decision would fire on the tick
        // after the one that withdrew.
        at_least(
            "[tacsim.arena.reflex].rally_ms",
            u64::from(self.reflex.decide_ms),
            u64::from(self.reflex.rally_ms),
        )?;
        between(
            "[tacsim.arena.reflex].stand_min_permille",
            0,
            1_000,
            u64::from(self.reflex.stand_min_permille),
        )?;
        // **16b cut E.1**: and the pair has to make sense together. A machine only
        // runs home below `withdraw_permille`, so a stand floor above it is a
        // doctrine branch nothing could ever be in — a free-standing key an
        // operator can strand by lowering the other number is a silently dead
        // rule, and a refusal that names both is the loud form.
        if self.reflex.stand_min_permille > self.reflex.withdraw_permille {
            return Err(format!(
                "[tacsim.arena.reflex].stand_min_permille is {}, above \
                 [tacsim.arena.reflex].withdraw_permille ({}): a machine only runs home below the \
                 second, so it could never turn and fight on the way",
                self.reflex.stand_min_permille, self.reflex.withdraw_permille
            ));
        }
        // **16b cut E.4**: off, or long enough to be read once — the plot is a
        // decide tick old when `Patrol` first reads it, and the age is compared
        // inclusively — and short enough to be a search and not a standing
        // order. The or-zero form is written the way
        // `[tacsim.match].unattended_ms`'s is.
        if self.reflex.search_ms != 0
            && (self.reflex.search_ms < self.reflex.decide_ms || self.reflex.search_ms > 600_000)
        {
            return Err(format!(
                "[tacsim.arena.reflex].search_ms is 0 (off) or {} to 600000; {} is not",
                self.reflex.decide_ms, self.reflex.search_ms
            ));
        }
        // **16b cut E.4**: off, or at least one decision. A hunt shorter than one
        // decide tick would be asked before the clock that gates it had ever been
        // read.
        if self.reflex.hunt_after_ms != 0 && self.reflex.hunt_after_ms < self.reflex.decide_ms {
            return Err(format!(
                "[tacsim.arena.reflex].hunt_after_ms is 0 (off) or at least {}; {} is not",
                self.reflex.decide_ms, self.reflex.hunt_after_ms
            ));
        }
        Ok(())
    }
}

impl MatchConfig {
    /// **Check `[tacsim.match]`** (16b cut Q.7).
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        // Ten minutes of `STARTUP` is not a startup; zero is legal, and is a
        // tank that is online the moment it is told to be.
        between(
            "[tacsim.match].startup_ms",
            0,
            600_000,
            u64::from(self.startup_ms),
        )?;
        at_least(
            "[tacsim.match].board_timeout_ms",
            1_000,
            u64::from(self.board_timeout_ms),
        )?;
        at_least(
            "[tacsim.match].max_duration_ms",
            60_000,
            u64::from(self.max_duration_ms),
        )?;
        // W9, doc 16 §1: twelve a side is the ceiling the whole model is sized
        // for, and zero is a lobby nobody can claim a slot in.
        between(
            "[tacsim.match].max_slots_per_side",
            1,
            12,
            u64::from(self.max_slots_per_side),
        )?;
        // **16b cut D.3 (`B-33`)**: off, or long enough to be a lull and not a
        // hiccup. A few seconds would end a match over one pilot's reconnect,
        // which is the opposite of what the key is for, so the floor is ten
        // seconds and the sentence says both halves.
        if self.unattended_ms != 0 && self.unattended_ms < 10_000 {
            return Err(format!(
                "[tacsim.match].unattended_ms is 0 (off) or at least 10000; {} is not",
                self.unattended_ms
            ));
        }
        // **16b cut Y.3**: zero is legal (a reclaim at once, the epoch table
        // settling the race); ten minutes is past any reason to keep a pilot
        // from their own tank.
        between(
            "[tacsim.match].reclaim_min_ms",
            0,
            600_000,
            u64::from(self.reclaim_min_ms),
        )?;
        Ok(())
    }
}

impl CommandConfig {
    /// **Check `[tacsim.command]`** (16b cut Y.2, in Q.7's pattern).
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        // Nought is a commander who can order nothing; more squads than a side
        // can ever be cut into is a span that never binds.
        between(
            "[tacsim.command].max_span",
            1,
            crate::deliberation::squad::MAX_SQUADS_PER_SIDE as u64,
            u64::from(self.max_span),
        )?;
        // Nought is no orders at all; ten a second is no floor.
        between(
            "[tacsim.command].orders_per_minute",
            1,
            600,
            u64::from(self.orders_per_minute),
        )?;
        Ok(())
    }
}

impl SensorsConfig {
    /// **Check `[tacsim.sensors]`** (16b cut Q.7).
    ///
    /// `max_signature_factor` is not here: it is the one configured float, and
    /// `mush_server::config_ext` refuses it on the way through the conversion to
    /// per mille, where the value the operator wrote is still in hand.
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        at_least("[tacsim.sensors].pass_ms", 50, u64::from(self.pass_ms))?;
        at_least(
            "[tacsim.sensors].lost_after_passes",
            1,
            u64::from(self.lost_after_passes),
        )?;
        between(
            "[tacsim.sensors].condition_green_permille",
            1,
            1_000,
            u64::from(self.condition_green_permille),
        )?;
        between(
            "[tacsim.sensors].condition_yellow_permille",
            1,
            1_000,
            u64::from(self.condition_yellow_permille),
        )?;
        // Green is the *higher* floor: a yellow floor at or above it would make
        // the yellow band empty and `SCAN` a two-colour read.
        if self.condition_yellow_permille >= self.condition_green_permille {
            return Err(format!(
                "[tacsim.sensors] must have condition_yellow_permille below \
                 condition_green_permille (they are {} and {})",
                self.condition_yellow_permille, self.condition_green_permille
            ));
        }
        between(
            "[tacsim.sensors].condition_reach_permille",
            1,
            1_000,
            u64::from(self.condition_reach_permille),
        )?;
        Ok(())
    }
}

impl HarnessConfig {
    /// **Check `[tacsim.harness]`** (16b cut Q.7).
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        if self.fighting > self.units {
            return Err(format!(
                "[tacsim.harness].fighting must be at most units ({}) (it is {})",
                self.units, self.fighting
            ));
        }
        // **16b cut U.2**: a bound of nought is a harness that fails on the first
        // pulse that does anything, so each of the five is at least one.
        for (key, value) in [
            (
                "max_cell_probes_per_pulse",
                u64::from(self.max_cell_probes_per_pulse),
            ),
            (
                "max_jam_checks_per_pulse",
                u64::from(self.max_jam_checks_per_pulse),
            ),
            ("max_rss_growth_kib", u64::from(self.max_rss_growth_kib)),
            ("max_frame_bytes", u64::from(self.max_frame_bytes)),
            (
                "max_output_events_per_pulse",
                u64::from(self.max_output_events_per_pulse),
            ),
        ] {
            at_least(&format!("[tacsim.harness].{key}"), 1, value)?;
        }
        Ok(())
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
    /// **16b cut Y.2** — the span of control and the order rate a pilot's
    /// `ORDER` is held to at the world (doc 16 §2 W8). **Live** since Y.2.
    pub command: CommandConfig,
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

#[cfg(test)]
mod tests {
    use super::{ArenaConfig, CommandConfig};

    /// **16b cut Z.1** (16c N6): `[tacsim.arena].squad_size` ships at four, takes
    /// both edges of `1..=16`, and refuses either side of them in Q.7's sentence.
    #[test]
    fn z1_the_squad_size_validates_by_name() {
        assert_eq!(ArenaConfig::default().squad_size, 4);
        assert_eq!(ArenaConfig::default().validate(), Ok(()));
        for legal in [1, 16] {
            assert_eq!(
                ArenaConfig {
                    squad_size: legal,
                    ..ArenaConfig::default()
                }
                .validate(),
                Ok(()),
                "{legal} is legal"
            );
        }
        for (value, sentence) in [
            (0, "[tacsim.arena].squad_size must be 1 to 16 (it is 0)"),
            (17, "[tacsim.arena].squad_size must be 1 to 16 (it is 17)"),
        ] {
            assert_eq!(
                ArenaConfig {
                    squad_size: value,
                    ..ArenaConfig::default()
                }
                .validate(),
                Err(sentence.to_string())
            );
        }
    }

    /// **16b cut I.3** (16c N10): the substep may be as long as the engaged
    /// period and no longer, or a staff floor — a multiple of `engaged_ms`, every
    /// floored pulse cut back to the substep lattice — could stop arena time.
    #[test]
    fn i3_the_substep_is_no_longer_than_the_engaged_period() {
        let with = |substep_max_ms: u32, engaged_ms: u32| {
            let mut cfg = ArenaConfig::default();
            cfg.tempo.substep_max_ms = substep_max_ms;
            cfg.tempo.engaged_ms = engaged_ms;
            cfg.validate()
        };
        assert_eq!(with(250, 500), Ok(()), "the shipped pair");
        assert_eq!(with(500, 500), Ok(()), "equal is legal");
        assert_eq!(
            with(501, 500),
            Err(
                "[tacsim.arena.tempo] must have substep_max_ms <= engaged_ms (they are 501, 500)"
                    .to_string()
            )
        );
        assert_eq!(
            with(250, 100),
            Err(
                "[tacsim.arena.tempo] must have substep_max_ms <= engaged_ms (they are 250, 100)"
                    .to_string()
            )
        );
    }

    /// **16b cut Y.2**: `[tacsim.command]` passes at its shipped values and
    /// refuses each edge past its band in Q.7's sentence, naming the key and the
    /// value the operator wrote.
    #[test]
    fn y2_the_command_block_validates_by_name() {
        assert_eq!(CommandConfig::default().validate(), Ok(()));
        assert_eq!(
            CommandConfig {
                max_span: 32,
                orders_per_minute: 600,
            }
            .validate(),
            Ok(()),
            "both upper edges are legal"
        );
        for (config, sentence) in [
            (
                CommandConfig {
                    max_span: 0,
                    ..CommandConfig::default()
                },
                "[tacsim.command].max_span must be 1 to 32 (it is 0)",
            ),
            (
                CommandConfig {
                    max_span: 33,
                    ..CommandConfig::default()
                },
                "[tacsim.command].max_span must be 1 to 32 (it is 33)",
            ),
            (
                CommandConfig {
                    orders_per_minute: 0,
                    ..CommandConfig::default()
                },
                "[tacsim.command].orders_per_minute must be 1 to 600 (it is 0)",
            ),
            (
                CommandConfig {
                    orders_per_minute: 601,
                    ..CommandConfig::default()
                },
                "[tacsim.command].orders_per_minute must be 1 to 600 (it is 601)",
            ),
        ] {
            assert_eq!(config.validate(), Err(sentence.to_string()));
        }
    }
}
