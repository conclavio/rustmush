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
//! | `[tacsim.surface].black_box_max_lines` | **live** (doc 23 cut S.1) | the most lines one hull's black box holds ([`SurfaceConfig`]) |
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

use serde::{Deserialize, Serialize};

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
/// the rate its sky moves at — T17's constant sky as amended by doc 18 R9 (doc 18
/// §4.8, doc 10 §22's amendment). **`rate = 0` is the default**: it holds every body
/// at its authored phase (the constant sky), so a distance pinned at load is the
/// distance on any IC day. `epoch` is the instant for a body without its own
/// `epoch_ic`; a lane body (doc 18 cut IC2, D-18-48) carries the real date its
/// numbers were read for. The knob exists so a campaign that wants a moving sky can
/// have one coherently (every pair sharing a body changes together).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EphemerisConfig {
    /// The epoch, IC seconds on the game calendar (`mush_core::CivilDate`:
    /// 365-day years, year 0 at IC 0): the instant a body without its own
    /// `epoch_ic` has its phase stated at. Only a nonzero rate ever reads it.
    pub epoch: i64,
    /// Permille of the real sky's motion per IC second; 0 holds every body at
    /// its authored phase (the constant sky).
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// **Arena milliseconds between two snapshots of a running match** (16b cut
    /// AF.1; 16c N16; doc 16 §3.1 "Durability"), taken by the arena task after
    /// the pulse that crossed the boundary and written by the world on the arena
    /// writer lane (16b cut AF.2 reads it). **`0` turns snapshots off**, and a
    /// restart then voids every running match by name (16b cut AF.3).
    ///
    /// **Nothing reads it in cut AF.1.** It lands here, a sub-cut before its
    /// reader, because the snapshot carries this whole block
    /// (`crate::arena::snapshot`) and the host builds it by an exhaustive
    /// literal: a field added in AF.2 would change every encoded state one
    /// commit after format 1 was born. `0` or `1_000..=60_000` by
    /// [`ArenaConfig::validate`].
    pub snapshot_ms: u32,
    /// Encoded recovery-document ceiling including the escaped state string.
    /// 16–480 MiB; admission reserves future growth. Not a normal allocation
    /// or the harness report-only size threshold. Below Redis RESP 512 MiB.
    pub snapshot_max_bytes: u64,
    /// Per-match private input recording cap (4–128 MiB).
    pub tape_max_bytes: u64,
    /// Aggregate resident retained recordings (4–1024 MiB).
    pub tape_retained_max_bytes: u64,
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
            // 16b cut AF.1: 16c N16's default, every five seconds of arena time.
            snapshot_ms: 5_000,
            snapshot_max_bytes: 480 * 1024 * 1024,
            tape_max_bytes: 64 * 1024 * 1024,
            tape_retained_max_bytes: 128 * 1024 * 1024,
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
/// **`decide_ms` is a cadence of arena time, not of pulses.** The reflex
/// controller's tick was a due event applied at its own arena timestamp, so the
/// controller ran the same number of times per arena second at every tempo —
/// which is what let the cross-tempo property hold with AI units on the field
/// (doc 16 §3.1, D-16-2).
///
/// **17b cut M4.6 deleted the reflex controller; this block keeps its name**
/// (`[tacsim.arena.reflex]`, every key and validation sentence unchanged, the
/// conventions' D10) because what it holds outlived it: `decide_ms` is still
/// the motor's cadence ([`crate::arena::state::DueKind::MotorTick`], on the
/// same stagger grid), the path budgets are the motor's, and the doctrine
/// figures are the stock card's (`crate::arena::card::line`) — read off
/// `ArenaConfig::reflex`, never off a copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflexConfig {
    /// Arena milliseconds between one machine's motor ticks, staggered by unit id
    /// (between the reflex controller's decisions until 17b cut M4.6).
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
    /// The most path searches the motor may run in one `decide_ms` window of
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentsConfig {
    /// **The discrete orders — and, since 17b cut M1a.4, the `DRIVE` goals —
    /// one seat may give a unit per arena second; also the size, per unit, of
    /// the world side's inbox and pre-pulse buffer** (16b cut M.1, `T-82`).
    /// Counted in arena time, so the burst a pilot may send does not depend on
    /// the tempo; the reflex controller's own orders, and the motor's, are never
    /// counted. The other continuous controls (`HEADING`, `SPEED`, a squad's
    /// order) are latest-wins and do not count; every `DRIVE` does, `HOLD` and
    /// `OFF` included, on the same window as `FIRE`, `LOCK` and `STARTUP`,
    /// because one to a cell spends a path search from the budget every hull
    /// shares.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// flown by Standard Doctrine on a unit mind sealed at
    /// `[tacsim.minds].fill_class` (**16b cut N.1**; doc 16 §6's *the AI fill of a
    /// later lobby*; the reflex controller's until 17b cut M4.3).
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
    /// to its sealed mind (the reflex controller until 17b cut M4.3) with a
    /// `Control { Released }`; the reclaim is a
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
            // Fifteen minutes (16c Q47, the owner's ruling of 2026-10-02): the arena is an explicit OOC proving ground.
            max_duration_ms: 900_000,
            max_slots_per_side: 12,
            ai_fill: false,
            // Five minutes: long enough that a pilot who drops and reconnects
            // has not ended their own match, short enough that a centre is not
            // held for the quarter-hour `max_duration_ms` would take.
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

/// **`[tacsim.minds]`** (17b cut M1b.1; doc 17 §5 "The law", D-UM-17 as
/// amended 2026-10-04): the unit minds' block.
///
/// Today it holds one key, **the Compact floor** — the least chance to hit a
/// lawful condition slot may name, per mille, seed 100. It is **captured into
/// each arena at check-out** (`crate::arena_task::spawn_arena` writes it to
/// `Arena::odds_floor_permille`, which the snapshot carries), so a reload of this
/// block never reaches a running or a resumed match. From 17b cut M3 the
/// classes and the pool; 17b cut M4.1 `workers`; 17b cut M4.2 `cpu_lens_ms`
/// (the minds lane's reference figure, read by no match); **17b cut M4.3**
/// `fill_class`, the default ceiling for AI fill; 17d M6.2 adds `max_class`,
/// the independent operator ceiling for pilot and fill minds. The lab knob
/// `fill_controller` of 17b cut M1b.5 is retired:
/// the fill is a mind, and a config still naming it refuses the boot.
///
/// **This block is never carried** (the snapshot carries `match_cfg`, not
/// this): class caps are read at `crate::match_::start`, and move no
/// snapshot format (the seal carries the class it produced).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindsConfig {
    /// The `mind-bulk-maintenance` pulse, milliseconds (the bulk model's
    /// `BulkConfig::maintenance_ms`): the pace of every Registry lease, so of
    /// a genuine rating. **Not a
    /// file key** (`serde(skip)`): production runs at the second, and only
    /// code sets it, the mush-server acceptance stories' harness, so a
    /// certificate takes minutes there rather than three hours.
    #[serde(skip, default = "default_bulk_maintenance_ms")]
    pub bulk_maintenance_ms: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_proving_workers")]
    pub proving_workers: usize,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_proving_queued")]
    pub proving_queued: usize,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_proving_daily_unit_minutes")]
    pub proving_daily_unit_minutes: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_proving_completed_per_character")]
    pub proving_completed_per_character: usize,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_proving_bytes_per_character")]
    pub proving_bytes_per_character: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_bench_idle_ms")]
    pub bench_idle_ms: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_bench_sessions")]
    pub bench_sessions: usize,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_bench_state_bytes")]
    pub bench_state_bytes: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_bench_eval_ops")]
    pub bench_eval_ops: u64,
    /// Bounded private proving/bench resource limit (17d M6.6/M6.7).
    #[serde(default = "default_bench_eval_result_bytes")]
    pub bench_eval_result_bytes: usize,
    /// **The Compact floor**, per mille: a condition under it is refused
    /// `unlawful` at the landing of its arming, and a card under it is refused
    /// at the seal (17b cut M3.3). `10` to `950` in steps of `10`
    /// ([`MindsConfig::validate`]; widened from `100` by 17b cut M3.3).
    pub odds_floor_permille: u32,
    /// Operator ceiling on every selected mind. The effective class also
    /// respects its declared rating and fitted core. Stock III.
    #[serde(default = "default_max_class")]
    pub max_class: crate::arena::core::CoreClass,
    /// Default ceiling for AI fill when its side has chosen no class.
    /// A selected pilot mind is governed by `max_class`, independently of this.
    /// A side's fill cap and this default never raise the declared rating.
    #[serde(default = "default_fill_class")]
    pub fill_class: crate::arena::core::CoreClass,
    /// **The Compact class table** (17b cut M3.1; doc 17 §9, D-UM-35): the
    /// four classes' figures, keyed by class word —
    /// `[tacsim.minds.classes.II]` / `think_ms = 500` and so on. An absent key,
    /// an absent class and an absent figure each read
    /// [`ClassTable::STOCK`](crate::arena::core::ClassTable::STOCK)'s
    /// (the wire shape is [`crate::arena::core::ClassTable`]'s), and
    /// [`MindsConfig::validate`] holds every figure by name. Captured into
    /// each arena at check-out (`crate::arena_task::spawn_arena`).
    #[serde(default)]
    pub classes: crate::arena::core::ClassTable,
    /// **The pool's base, ops a sim second** (17b cut M3.5; doc 17 §9 "The
    /// pool", D-UM-36): a sim task's deterministic compute allotment before
    /// the per-unit charge — `crate::match_::pool::pool_for` is
    /// `max(floor, base − per_unit × units)`. Stock 500,000. At least 1.
    #[serde(default = "default_pool_base_ops_s")]
    pub pool_base_ops_s: u64,
    /// **The pool's charge per unit on the task, ops a sim second** (17b cut
    /// M3.5; D-UM-36): stock 1,000. At least 1.
    #[serde(default = "default_pool_per_unit_ops_s")]
    pub pool_per_unit_ops_s: u64,
    /// **The pool's floor, ops a sim second** (17b cut M3.5; D-UM-36): no
    /// task's allotment is less, however many units it carries. Stock
    /// 120,000. At least 1, and at most `pool_base_ops_s`.
    #[serde(default = "default_pool_floor_ops_s")]
    pub pool_floor_ops_s: u64,
    /// **The think pool's worker threads** (17b cut M4.1; doc 17 §6
    /// "Execution policy", D-UM-22): the process-wide
    /// [`crate::minds::MindPool`] every sim task sends its collected thinks
    /// to, sized ONCE per process from the first task's read
    /// (`MindPool::shared`). Stock 2; 1 to 32 ([`MindsConfig::validate`]).
    /// Never carried: a resumed match thinks on the process's pool.
    #[serde(default = "default_workers")]
    pub workers: u32,
    /// **The mind CPU lens, milliseconds per 500 ms window** (17b cut M4.2;
    /// doc 17 §18): the reference figure the minds lane
    /// (`tacsim_harness_arena_minds`) PRINTS beside the lens it asserts —
    /// which is computed, the pool's ops per window over the host's measured
    /// ops per microsecond times five quarters (the M4 row's finding (7)) —
    /// never a throttle the arena applies (D-16-2) and never read by a match.
    /// Stock 5; at least 1 ([`MindsConfig::validate`]).
    #[serde(default = "default_cpu_lens_ms")]
    pub cpu_lens_ms: u32,
    /// **The largest Registry fine, minor units** (doc 17h cut M8d D3; Q-M8-6, provisional):
    /// `@registry/fine` refuses an amount above it. Stock 100,000 (10× doc 17's 100cr filing
    /// fee); at least 1.
    #[serde(default = "default_fine_cap_minor")]
    pub fine_cap_minor: i64,
    /// **How long a revoked principal may hold no new licence, IC days** (doc 17h cut M8d
    /// D4; Q-M8-6, provisional): `@registry/revoke principal=`'s term. Stock 90 (one audit
    /// term); 1 to 3,650.
    #[serde(default = "default_revocation_ic_days")]
    pub revocation_ic_days: u32,
    /// **How old an inspection may be to ground a seizure, IC days** (doc 17h cut M8d2 S2;
    /// Q-M8-6, provisional): `@registry/seize` refuses on an older newest inspection. Stock 3;
    /// 1 to 3,650.
    #[serde(default = "default_seizure_inspection_max_age_ic_days")]
    pub seizure_inspection_max_age_ic_days: u32,
    /// **How long a seizure is held before a forfeit, IC days** (doc 17h cut M8d2 S5; Q-M8-6,
    /// provisional): `@registry/forfeit` refuses within it. Stock 3; 0 to 3,650.
    #[serde(default = "default_seizure_hold_ic_days")]
    pub seizure_hold_ic_days: u32,
}

/// `seizure_inspection_max_age_ic_days`' default (Q-M8-6).
const fn default_seizure_inspection_max_age_ic_days() -> u32 {
    mush_roll::commerce::registry_inspection::seizure::DEFAULT_INSPECTION_MAX_AGE_IC_DAYS
}

/// `seizure_hold_ic_days`' default (Q-M8-6).
const fn default_seizure_hold_ic_days() -> u32 {
    mush_roll::commerce::registry_inspection::seizure::DEFAULT_SEIZURE_HOLD_IC_DAYS
}

/// `fine_cap_minor`'s default: 10× doc 17's 100cr filing fee (Q-M8-6).
const fn default_fine_cap_minor() -> i64 {
    100_000
}

/// `revocation_ic_days`' default: one audit term (Q-M8-6).
const fn default_revocation_ic_days() -> u32 {
    90
}

/// `workers`' default: doc 17 §6's two.
const fn default_workers() -> u32 {
    2
}

/// `cpu_lens_ms`' default: doc 17 §18's five.
const fn default_cpu_lens_ms() -> u32 {
    5
}

/// **The most worker threads `[tacsim.minds].workers` may name** (17b cut
/// M4.1): a figure past any host the engine targets, held so a typo cannot
/// spawn a thousand threads.
pub const MAX_MIND_WORKERS: u32 = 32;

/// `max_class`' default: all classes are available to authors.
const fn default_max_class() -> crate::arena::core::CoreClass {
    crate::arena::core::CoreClass::III
}

/// `fill_class`' default: Class I, Standard Doctrine's rating.
const fn default_fill_class() -> crate::arena::core::CoreClass {
    crate::arena::core::CoreClass::I
}

/// `pool_base_ops_s`' default: doc 17 §9's 500,000.
const fn default_pool_base_ops_s() -> u64 {
    500_000
}

/// `pool_per_unit_ops_s`' default: doc 17 §9's 1,000.
const fn default_pool_per_unit_ops_s() -> u64 {
    1_000
}

/// `pool_floor_ops_s`' default: doc 17 §9's 120,000.
const fn default_pool_floor_ops_s() -> u64 {
    120_000
}

fn default_proving_workers() -> usize {
    crate::minds::bulk::model::BulkConfig::default().proving_workers
}
fn default_proving_queued() -> usize {
    crate::minds::bulk::model::BulkConfig::default().queued
}
fn default_proving_daily_unit_minutes() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().daily_unit_minutes
}
fn default_proving_completed_per_character() -> usize {
    crate::minds::bulk::model::BulkConfig::default().completed_per_character
}
fn default_proving_bytes_per_character() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().bytes_per_character
}
fn default_bench_idle_ms() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().bench_idle_ms
}
fn default_bench_sessions() -> usize {
    crate::minds::bulk::model::BulkConfig::default().bench_sessions
}
fn default_bench_state_bytes() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().bench_state_bytes
}
fn default_bench_eval_ops() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().eval_ops
}
fn default_bench_eval_result_bytes() -> usize {
    crate::minds::bulk::model::BulkConfig::default().eval_result_bytes
}
fn default_bulk_maintenance_ms() -> u64 {
    crate::minds::bulk::model::BulkConfig::default().maintenance_ms
}

impl Default for MindsConfig {
    fn default() -> Self {
        MindsConfig {
            bulk_maintenance_ms: default_bulk_maintenance_ms(),
            proving_workers: default_proving_workers(),
            proving_queued: default_proving_queued(),
            proving_daily_unit_minutes: default_proving_daily_unit_minutes(),
            proving_completed_per_character: default_proving_completed_per_character(),
            proving_bytes_per_character: default_proving_bytes_per_character(),
            bench_idle_ms: default_bench_idle_ms(),
            bench_sessions: default_bench_sessions(),
            bench_state_bytes: default_bench_state_bytes(),
            bench_eval_ops: default_bench_eval_ops(),
            bench_eval_result_bytes: default_bench_eval_result_bytes(),
            // Doc 17 §5's seed value: one chance in ten, the Line preset's own
            // `min_odds`.
            odds_floor_permille: 100,
            // 17b cut M4.3: the fill at Class I.
            fill_class: default_fill_class(),
            max_class: default_max_class(),
            // 17b cut M3.1: doc 17 §9's table.
            classes: crate::arena::core::ClassTable::STOCK,
            // 17b cut M3.5: doc 17 §9's pool (D-UM-36).
            pool_base_ops_s: default_pool_base_ops_s(),
            pool_per_unit_ops_s: default_pool_per_unit_ops_s(),
            pool_floor_ops_s: default_pool_floor_ops_s(),
            // 17b cut M4.1: doc 17 §6's two workers.
            workers: default_workers(),
            // 17b cut M4.2: doc 17 §18's five milliseconds, printed beside.
            cpu_lens_ms: default_cpu_lens_ms(),
            // Doc 17h cut M8d (Q-M8-6): the provisional fine cap and revocation term.
            fine_cap_minor: default_fine_cap_minor(),
            revocation_ic_days: default_revocation_ic_days(),
            // Doc 17h cut M8d2 (Q-M8-6): the provisional seizure age and hold.
            seizure_inspection_max_age_ic_days: default_seizure_inspection_max_age_ic_days(),
            seizure_hold_ic_days: default_seizure_hold_ic_days(),
        }
    }
}

impl MindsConfig {
    pub(crate) fn bulk(&self) -> crate::minds::bulk::model::BulkConfig {
        crate::minds::bulk::model::BulkConfig {
            proving_workers: self.proving_workers,
            queued: self.proving_queued,
            daily_unit_minutes: self.proving_daily_unit_minutes,
            completed_per_character: self.proving_completed_per_character,
            bytes_per_character: self.proving_bytes_per_character,
            bench_idle_ms: self.bench_idle_ms,
            bench_sessions: self.bench_sessions,
            bench_state_bytes: self.bench_state_bytes,
            eval_ops: self.bench_eval_ops,
            eval_result_bytes: self.bench_eval_result_bytes,
            maintenance_ms: self.bulk_maintenance_ms,
        }
    }
    /// Operator-bounded class ceiling for a side's AI fill. An absent side
    /// choice uses the operator's default fill class.
    #[must_use]
    pub fn fill_cap(
        &self,
        chosen: Option<crate::arena::core::CoreClass>,
    ) -> crate::arena::core::CoreClass {
        self.max_class.min(chosen.unwrap_or(self.fill_class))
    }

    /// **Check `[tacsim.minds]`** (17b cut M1b.1, in Q.7's pattern).
    ///
    /// The floor is **10 to 950 in steps of 10**: the step because every
    /// sentence that names the floor prints it in whole per cents
    /// (`floor / 10`), and the ceiling the card's own `min_odds` ceiling of 95
    /// per cent. **17b cut M3.3 widened it from 100**: the seal now validates
    /// the card against the floor by name (`crate::match_::seal::seal_for`),
    /// so an operator floor above 100 — above the Line card's 10 per cent —
    /// refuses every Line-carded check-out
    /// (`crate::match_::StartError::CardUnlawful`): 17b cut M5.1 lets a
    /// principal choose a lawful preset with `SIM MIND` (one under the floor
    /// refused by name), which 17b cut M5.2 seals; an armed hull nobody chose
    /// for is still sealed on Line, and refused.
    ///
    /// # Errors
    ///
    /// One sentence naming the key and the value it was given.
    pub fn validate(&self) -> Result<(), String> {
        let floor = self.odds_floor_permille;
        if !(10..=950).contains(&floor) || !floor.is_multiple_of(10) {
            return Err(format!(
                "[tacsim.minds].odds_floor_permille must be 10 to 950 in steps of 10 (it is {floor})"
            ));
        }
        self.bulk()
            .validate()
            .map_err(|e| format!("[tacsim.minds] {e}"))?;
        self.validate_pool()?;
        // 17b cut M4.1: the think pool's workers, 1 to `MAX_MIND_WORKERS`.
        if !(1..=MAX_MIND_WORKERS).contains(&self.workers) {
            return Err(format!(
                "[tacsim.minds].workers must be 1 to {MAX_MIND_WORKERS} (it is {})",
                self.workers
            ));
        }
        // 17b cut M4.2: the CPU lens's reference figure, at least one.
        if self.cpu_lens_ms < 1 {
            return Err(format!(
                "[tacsim.minds].cpu_lens_ms must be at least 1 (it is {})",
                self.cpu_lens_ms
            ));
        }
        // Doc 17h cut M8d D3/D4: the fine cap at least 1, the revocation term 1 to 3,650.
        if self.fine_cap_minor < 1 {
            return Err(format!(
                "[tacsim.minds].fine_cap_minor must be at least 1 (it is {})",
                self.fine_cap_minor
            ));
        }
        if !(1..=3_650).contains(&self.revocation_ic_days) {
            return Err(format!(
                "[tacsim.minds].revocation_ic_days must be 1 to 3650 (it is {})",
                self.revocation_ic_days
            ));
        }
        // Doc 17h cut M8d2: the inspection age 1 to 3,650 IC days, the hold at most 3,650.
        if !(1..=3_650).contains(&self.seizure_inspection_max_age_ic_days) {
            return Err(format!(
                "[tacsim.minds].seizure_inspection_max_age_ic_days must be 1 to 3650 (it is {})",
                self.seizure_inspection_max_age_ic_days
            ));
        }
        if self.seizure_hold_ic_days > 3_650 {
            return Err(format!(
                "[tacsim.minds].seizure_hold_ic_days must be 0 to 3650 (it is {})",
                self.seizure_hold_ic_days
            ));
        }
        self.validate_classes()
    }

    /// **The pool's three keys, by name** (17b cut M3.5; doc 17 §9, D-UM-36):
    /// each at least 1, and the floor at most the base — a floor above the
    /// base would make the per-unit charge meaningless at every count.
    ///
    /// # Errors
    ///
    /// The first refusal, as one sentence naming the key and its value.
    fn validate_pool(&self) -> Result<(), String> {
        for (key, value) in [
            ("pool_base_ops_s", self.pool_base_ops_s),
            ("pool_per_unit_ops_s", self.pool_per_unit_ops_s),
            ("pool_floor_ops_s", self.pool_floor_ops_s),
        ] {
            if value == 0 {
                return Err(format!(
                    "[tacsim.minds].{key} must be at least 1 (it is {value})"
                ));
            }
        }
        if self.pool_floor_ops_s > self.pool_base_ops_s {
            return Err(format!(
                "[tacsim.minds].pool_floor_ops_s must be at most pool_base_ops_s (it is {} \
                 against {})",
                self.pool_floor_ops_s, self.pool_base_ops_s
            ));
        }
        Ok(())
    }

    /// **`[tacsim.minds].classes`, by name** (17b cut M3.1; doc 17 §9,
    /// D-UM-35), lowest class first and each class's figures in the row's
    /// order, each refused in one sentence naming the class, the figure and
    /// its value:
    ///
    /// * `think_ms` 250 to 5000 in steps of 250 (the substep's lattice), and
    ///   no longer than the class below's — a better core never thinks less
    ///   often;
    /// * `ops` 100 to `mush_mind::MAX_OPS_ANY_CLASS` (the engine's backstop:
    ///   a think past it is refused before it runs), and no fewer than the
    ///   class below's;
    /// * `rows` 2 to 32 and no fewer than the class below's (17b cut M5.1's
    ///   rider: the floor was 1 — a class of one row cannot carry the seat's
    ///   two, the hull's lock and its order's contact); `odds_rows` 0 to the
    ///   class's own `rows`;
    /// * `slots` 1 to [`crate::arena::core::MAX_CONDITION_SLOTS`] and no fewer
    ///   than the class below's;
    /// * `memory_keys` 1 to `mush_mind::MAX_MEMORY_ANY_CLASS.keys` (the
    ///   engine's data-size backstop is twice that) and no fewer than the
    ///   class below's;
    /// * `line_to` 0 to 8; `profiles` 1 to 4.
    ///
    /// # Errors
    ///
    /// The first refusal, as one sentence.
    fn validate_classes(&self) -> Result<(), String> {
        use crate::arena::core::{CoreClass, CoreProfile, MAX_CONDITION_SLOTS};
        let to_u64 = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let mut below: Option<CoreProfile> = None;
        for class in CoreClass::ALL {
            let row = self.classes.row(class);
            let key = |figure: &str| format!("[tacsim.minds].classes.{}.{figure}", class.as_str());
            let think = row.think_ms;
            if !(250..=5_000).contains(&think) || think % 250 != 0 {
                return Err(format!(
                    "{} must be 250 to 5000 in steps of 250 (it is {think})",
                    key("think_ms")
                ));
            }
            between(&key("ops"), 100, mush_mind::MAX_OPS_ANY_CLASS, row.ops)?;
            // 17b cut M5.1: the floor 1 -> 2 (doc 17 §24's carried "a configured
            // class of fewer rows than the seat's two").
            between(&key("rows"), 2, 32, to_u64(row.rows))?;
            between(
                &key("odds_rows"),
                0,
                to_u64(row.rows),
                to_u64(row.odds_rows),
            )?;
            between(
                &key("slots"),
                1,
                to_u64(MAX_CONDITION_SLOTS),
                to_u64(row.slots),
            )?;
            between(
                &key("memory_keys"),
                1,
                to_u64(mush_mind::MAX_MEMORY_ANY_CLASS.keys),
                to_u64(row.memory_keys),
            )?;
            between(&key("line_to"), 0, 8, u64::from(row.line_to))?;
            between(&key("profiles"), 1, 4, u64::from(row.profiles))?;
            if let Some(lower) = below {
                let name = lower.class.as_str();
                if think > lower.think_ms {
                    return Err(format!(
                        "{} must be at most class {name}'s {} (it is {think})",
                        key("think_ms"),
                        lower.think_ms
                    ));
                }
                for (figure, value, floor) in [
                    ("ops", row.ops, lower.ops),
                    ("rows", to_u64(row.rows), to_u64(lower.rows)),
                    ("slots", to_u64(row.slots), to_u64(lower.slots)),
                    (
                        "memory_keys",
                        to_u64(row.memory_keys),
                        to_u64(lower.memory_keys),
                    ),
                ] {
                    if value < floor {
                        return Err(format!(
                            "{} must be at least class {name}'s {floor} (it is {value})",
                            key(figure)
                        ));
                    }
                }
            }
            below = Some(row);
        }
        Ok(())
    }
}

/// The sensor pass's cadence and its signature ceiling (**16b cut G**; doc 16
/// §3.6 "Perception").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// The per-pulse p99 ceiling, milliseconds, in `Engaged` tempo (40 since
    /// 16c `T-109`: the owner's figures follow the production VPS; 25 before).
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
    /// **The largest arena snapshot the 30-second lane may encode**, bytes (16b
    /// cut AF.2; 16c N16): the state string of `crate::arena::Snapshot::bytes`,
    /// asserted on `harness_arena_snapshot`'s `bytes_max` on both cohorts. It
    /// is the harness's own lane's bound and never a throttle: nothing in the
    /// arena or the task reads it, and a larger snapshot is still taken and
    /// written.
    ///
    /// 7 MiB (7,340,032) since 16b cut AF.2, **measured before it was set**:
    /// the authored lane's 4,198,651 bytes, three halves of it rounded up to
    /// the next MiB (16b cut J.3b's precedent); the CI cohort measured
    /// 3,897,681 on the same run.
    pub max_snapshot_bytes: u64,
    /// **The most A\* cells one wake's path searches may expand, found or
    /// failed** (16b cut AI.1; 16c `B-84`), asserted against the worst wake's
    /// `crate::arena::motor::MotorCounters::path_cells` on both cohorts,
    /// beside [`HarnessConfig::max_path_queries_per_pulse`]. It bounds what the
    /// queries bound does not: a search's cost, which a `Budget` failure
    /// (`path_max_cells` expansions) and a `NoRoute` flood (the whole reachable
    /// region) are the two dearest of — both counted since the same cut.
    ///
    /// Like every other key in this block it is a **threshold the harness
    /// asserts**, never a throttle the arena applies: a controller that stopped
    /// searching when a per-wake budget ran out would make a route a function of
    /// the tempo, which D-16-2 forbids.
    ///
    /// 48,128 since 16b cut AI.1, **measured before it was set**: the largest
    /// `peak_path_cells` of both arena lanes at seeds 2350 to 2353 after the
    /// cut — 41,279, the authored lane at 2352 — times 1.16 (47,884), rounded
    /// up to the next multiple of 1,024. The eight peaks were 28,722, 19,636,
    /// 31,143 and 28,201 on the CI cohort and 31,200, 28,037, 41,279 and 24,102
    /// on the authored one. The cut's first measurement read 50,439 to 59,508,
    /// each wake dominated by one search into a pocket its closed corners had
    /// sealed that flooded the 50,400-odd cells it could reach; the goal-side
    /// probe (`crate::arena::path::POCKET_PROBE_CELLS`) answers those for the
    /// pocket, and the bound was sized after it.
    pub max_path_cells_per_pulse: u64,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        HarnessConfig {
            units: 300,
            fighting: 24,
            engaged_p99_ms: 40,
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
            // 16b cut AF.2: the authored lane's 4,198,651 bytes, three halves
            // of it rounded up to the next MiB (7 MiB).
            max_snapshot_bytes: 7_340_032,
            // 16b cut AI.1: the eight-run peak, 41,279, times 1.16, rounded up to
            // the next multiple of 1,024.
            max_path_cells_per_pulse: 48_128,
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
            // **Doc 18 cut IC2** (reviewer F10): a class no arena fields has no
            // cap to set — the typed path fixtures build by hand gets the same
            // door, and the same sentence, as the host's word parser. Doc 20
            // cut SM.3a's terrain vehicle is the second such class (the
            // hauler the first); the sentence names the fieldable three.
            if !class.fieldable() {
                return Err(format!(
                    "[tacsim.arena.max_units_by_class] names a class that is not armour, \
                     walker or drone: {}",
                    class.as_str()
                ));
            }
            between(
                &format!("[tacsim.arena.max_units_by_class].{}", class.as_str()),
                0,
                crate::definitions::arena::MAX_ARENA_UNITS as u64,
                u64::from(*cap),
            )?;
        }
        // **16b cut AF.1** (16c N16): off, or a cadence the harness can bound — a
        // snapshot every pulse of the fastest tempo is an encode every 500 ms,
        // and one a minute apart rewinds a resumed match by up to a minute.
        if !(16 * 1024 * 1024..=480 * 1024 * 1024).contains(&self.snapshot_max_bytes) {
            return Err("[tacsim.arena].snapshot_max_bytes must be 16 to 480 MiB".into());
        }
        if !(4 * 1024 * 1024..=128 * 1024 * 1024).contains(&self.tape_max_bytes) {
            return Err("[tacsim.arena].tape_max_bytes must be 4 to 128 MiB".into());
        }
        if !(4 * 1024 * 1024..=1024 * 1024 * 1024).contains(&self.tape_retained_max_bytes) {
            return Err("[tacsim.arena].tape_retained_max_bytes must be 4 to 1024 MiB".into());
        }
        if self.snapshot_ms != 0 && !(1_000..=60_000).contains(&self.snapshot_ms) {
            return Err(format!(
                "[tacsim.arena].snapshot_ms must be 0 or 1000 to 60000 (it is {})",
                self.snapshot_ms
            ));
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
        // pulse that does anything, so each of the five is at least one — and
        // **16b cut AF.2**'s snapshot bound with them.
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
            // 16b cut AF.2.
            ("max_snapshot_bytes", self.max_snapshot_bytes),
            // 16b cut AI.1.
            ("max_path_cells_per_pulse", self.max_path_cells_per_pulse),
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
    /// **17b cut M1b.1** — `[tacsim.minds]`: the Compact floor, captured into
    /// each arena at check-out. **Live** since M1b.1 (default when absent);
    /// from 17b cut M4.3 the fill's class, `fill_class`.
    pub minds: MindsConfig,
    /// **Doc 23 cut S.1** — the part of `[tacsim.surface]` the game reads: the black box's
    /// line cap. **Live.**
    pub surface: SurfaceConfig,
    /// **Doc 17h cut M8f** — `[tacsim.procurement]`: a procurement policy's defaults.
    /// **Live** (`crate::procurement`).
    pub procurement: ProcurementConfig,
}

/// **The largest monthly budget a procurement policy may name** (doc 17h cut M8f): a bound on
/// an operator's typo, not a tuning value (the launch's own price ceiling).
pub const MAX_PROCUREMENT_BUDGET_MINOR: i64 = 100_000_000;

/// **`[tacsim.procurement]`** (doc 17h cut M8f; Q-M8-6, D-UM-54): the defaults a House's
/// procurement policy takes when its deployment row names none. Provisional (the owner's
/// economy-defaults rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcurementConfig {
    /// Minor credits per IC calendar month (2,000cr).
    pub budget_minor_per_ic_month: i64,
    /// The diversity cap, per mille of the organisation's titled hulls (400).
    pub cap_permille: u16,
}

impl Default for ProcurementConfig {
    fn default() -> Self {
        Self {
            budget_minor_per_ic_month: 200_000,
            cap_permille: 400,
        }
    }
}

impl ProcurementConfig {
    /// **Check `[tacsim.procurement]`**: the budget 1 to [`MAX_PROCUREMENT_BUDGET_MINOR`], the
    /// cap 1 to 1000.
    ///
    /// # Errors
    ///
    /// The sentence naming the key.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_PROCUREMENT_BUDGET_MINOR).contains(&self.budget_minor_per_ic_month) {
            return Err(format!(
                "[tacsim.procurement].budget_minor_per_ic_month must be 1 to \
                 {MAX_PROCUREMENT_BUDGET_MINOR} (it is {})",
                self.budget_minor_per_ic_month
            ));
        }
        if !(1..=1000).contains(&self.cap_permille) {
            return Err(format!(
                "[tacsim.procurement].cap_permille must be 1 to 1000 (it is {})",
                self.cap_permille
            ));
        }
        Ok(())
    }
}

/// **The most lines one hull's black box holds, by default** (doc 23 cut S.1, the lead's L2):
/// two per excursion, so 2,048 excursions a hull. Provisional (the owner's economy-defaults
/// rule of 2026-10-24: a reasonable default, tuned as we go).
pub const DEFAULT_BLACK_BOX_MAX_LINES: u32 = 4_096;

/// **The largest cap a server may set** — a million lines, far past any hull's life, so a
/// typo cannot ask the ledger for an unbounded log.
pub const MAX_BLACK_BOX_MAX_LINES: u32 = 1_048_576;

/// **`[tacsim.surface]`, as the game reads it** (doc 23 cut S.1). `registry_interlock` stays
/// the host's alone (`mush_server::config_ext` refuses `false`; nothing here reads it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceConfig {
    /// **The most lines one hull's black box holds** (doc 23 §7; `log:hull.<n>`): an append
    /// past it is refused and paged, never truncated — `mush_core::Ledger` has no retention
    /// primitive to drop a sealed chunk with.
    pub black_box_max_lines: u32,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            black_box_max_lines: DEFAULT_BLACK_BOX_MAX_LINES,
        }
    }
}

impl SurfaceConfig {
    /// **Check `[tacsim.surface]`**: the cap is 1 to [`MAX_BLACK_BOX_MAX_LINES`].
    ///
    /// # Errors
    ///
    /// The sentence naming the key.
    pub fn validate(&self) -> Result<(), String> {
        if self.black_box_max_lines == 0 || self.black_box_max_lines > MAX_BLACK_BOX_MAX_LINES {
            return Err(format!(
                "[tacsim.surface].black_box_max_lines must be 1 to {MAX_BLACK_BOX_MAX_LINES} (it is {})",
                self.black_box_max_lines
            ));
        }
        Ok(())
    }
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

    /// **Doc 18 cut IC2** (reviewer F10): a cap on the hauler — a class no arena
    /// fields — is refused by the typed path with the host parser's own
    /// sentence; the default still validates.
    #[test]
    fn ic2_arena_config_refuses_a_hauler_cap() {
        use crate::definitions::UnitClass;
        assert_eq!(ArenaConfig::default().validate(), Ok(()));
        let mut cfg = ArenaConfig::default();
        cfg.max_units_by_class.insert(UnitClass::Hauler, 1);
        assert_eq!(
            cfg.validate(),
            Err(
                "[tacsim.arena.max_units_by_class] names a class that is not armour, walker or \
                 drone: hauler"
                    .to_owned()
            )
        );
    }

    /// **Doc 20 cut SM.3a** (D-20-6): the hauler test's twin — a cap on the
    /// terrain vehicle, a class no arena fields in phase 1, is refused by the
    /// typed path with the same sentence.
    #[test]
    fn sm3a_arena_config_refuses_a_vehicle_cap() {
        use crate::definitions::UnitClass;
        let mut cfg = ArenaConfig::default();
        cfg.max_units_by_class.insert(UnitClass::Vehicle, 1);
        assert_eq!(
            cfg.validate(),
            Err(
                "[tacsim.arena.max_units_by_class] names a class that is not armour, walker or \
                 drone: vehicle"
                    .to_owned()
            )
        );
    }

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

    /// **16b cut AF.1** (16c N16): `[tacsim.arena].snapshot_ms` ships at five
    /// seconds, takes `0` (off) and both edges of `1000..=60000`, and refuses
    /// either side of the band in Q.7's sentence.
    #[test]
    fn af1_the_snapshot_cadence_validates_by_name() {
        assert_eq!(ArenaConfig::default().snapshot_ms, 5_000);
        for legal in [0, 1_000, 60_000] {
            assert_eq!(
                ArenaConfig {
                    snapshot_ms: legal,
                    ..ArenaConfig::default()
                }
                .validate(),
                Ok(()),
                "{legal} is legal"
            );
        }
        for value in [7, 999, 60_001] {
            assert_eq!(
                ArenaConfig {
                    snapshot_ms: value,
                    ..ArenaConfig::default()
                }
                .validate(),
                Err(format!(
                    "[tacsim.arena].snapshot_ms must be 0 or 1000 to 60000 (it is {value})"
                ))
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

    /// **17b cut M1b.1**: the Compact floor is 10 to 100 in steps of 10, and
    /// every refusal names the key and the value. **17b cut M3.3** widened the
    /// bound to 950 (the seal refuses a card under it by name).
    #[test]
    fn m1b1_the_floor_is_ten_to_a_hundred_in_steps_of_ten() {
        use super::MindsConfig;
        assert_eq!(MindsConfig::default().odds_floor_permille, 100);
        for floor in [10, 50, 100, 110, 200, 950] {
            assert_eq!(
                MindsConfig {
                    odds_floor_permille: floor,
                    ..MindsConfig::default()
                }
                .validate(),
                Ok(()),
                "{floor}"
            );
        }
        for floor in [0, 15, 205, 960] {
            assert_eq!(
                MindsConfig {
                    odds_floor_permille: floor,
                    ..MindsConfig::default()
                }
                .validate(),
                Err(format!(
                    "[tacsim.minds].odds_floor_permille must be 10 to 950 in steps of 10 (it is \
                     {floor})"
                ))
            );
        }
    }

    /// **17b cut M4.3** (doc 17 §9, D-UM-37): `[tacsim.minds].fill_class`
    /// reads Class I when absent and decodes by its word; an unknown word and
    /// the retired lab knob `fill_controller` are both refused by serde
    /// (`deny_unknown_fields`), so a config still naming it refuses the boot.
    #[test]
    fn m4_3_the_fill_class_defaults_to_i_and_decodes_by_word() {
        use super::MindsConfig;
        use crate::arena::core::CoreClass;
        assert_eq!(MindsConfig::default().fill_class, CoreClass::I);
        let absent: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100}"#).expect("the block decodes");
        assert_eq!(absent.fill_class, CoreClass::I);
        let three: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100, "fill_class": "III"}"#)
                .expect("the block decodes");
        assert_eq!(three.fill_class, CoreClass::III);
        assert_eq!(three.validate(), Ok(()));
        let zero: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100, "fill_class": "0"}"#)
                .expect("the block decodes");
        assert_eq!(zero.fill_class, CoreClass::Zero);
        let refused = serde_json::from_str::<MindsConfig>(
            r#"{"odds_floor_permille": 100, "fill_class": "IV"}"#,
        )
        .expect_err("IV is no class")
        .to_string();
        assert!(refused.contains("IV"), "{refused}");
        let retired = serde_json::from_str::<MindsConfig>(
            r#"{"odds_floor_permille": 100, "fill_controller": "mind"}"#,
        )
        .expect_err("the lab knob is retired")
        .to_string();
        assert!(
            retired.contains("fill_controller") && retired.contains("unknown field"),
            "{retired}"
        );
    }

    /// **17b cut M3.1** (doc 17 §9, D-UM-35): `[tacsim.minds].classes` reads
    /// the stock table where it is absent — the whole key, a class, a figure
    /// — the stock table validates, and every refusal names its class, its
    /// figure and its value in one sentence.
    #[test]
    fn m3_the_class_table_validates_by_name() {
        use super::MindsConfig;
        use crate::arena::core::{ClassTable, CoreClass};
        // Absent reads stock: the key, a class, a figure.
        let absent: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100}"#).expect("decodes");
        assert_eq!(absent.classes, ClassTable::STOCK);
        let one: MindsConfig = serde_json::from_str(
            r#"{"odds_floor_permille": 100, "classes": {"III": {"think_ms": 500}}}"#,
        )
        .expect("decodes");
        assert_eq!(one.classes.row(CoreClass::III).think_ms, 500);
        assert_eq!(one.classes.row(CoreClass::III).ops, 4_500);
        assert_eq!(one.validate(), Ok(()), "III at II's period is lawful");
        assert_eq!(MindsConfig::default().classes, ClassTable::STOCK);
        assert_eq!(MindsConfig::default().validate(), Ok(()));
        // Every refusal, by name: (the class, the JSON figures, the sentence).
        let refused = |class: &str, figures: &str| {
            let text =
                format!(r#"{{"odds_floor_permille": 100, "classes": {{"{class}": {figures}}}}}"#);
            let config: MindsConfig = serde_json::from_str(&text).expect("decodes");
            config.validate().expect_err("refused")
        };
        for (class, figures, sentence) in [
            (
                "I",
                r#"{"think_ms": 200}"#,
                "[tacsim.minds].classes.I.think_ms must be 250 to 5000 in steps of 250 (it is 200)",
            ),
            (
                "I",
                r#"{"think_ms": 300}"#,
                "[tacsim.minds].classes.I.think_ms must be 250 to 5000 in steps of 250 (it is 300)",
            ),
            (
                "0",
                r#"{"think_ms": 5250}"#,
                "[tacsim.minds].classes.0.think_ms must be 250 to 5000 in steps of 250 (it is 5250)",
            ),
            (
                "II",
                r#"{"think_ms": 1250}"#,
                "[tacsim.minds].classes.II.think_ms must be at most class I's 1000 (it is 1250)",
            ),
            (
                "I",
                r#"{"ops": 99}"#,
                "[tacsim.minds].classes.I.ops must be 100 to 4500 (it is 99)",
            ),
            (
                "III",
                r#"{"ops": 4501}"#,
                "[tacsim.minds].classes.III.ops must be 100 to 4500 (it is 4501)",
            ),
            (
                "II",
                r#"{"ops": 1000}"#,
                "[tacsim.minds].classes.II.ops must be at least class I's 1500 (it is 1000)",
            ),
            (
                "0",
                r#"{"rows": 0}"#,
                // 17b cut M5.1: `must be 1 to 32 (it is 0)` until the rider
                // raised the floor to the seat's two rows.
                "[tacsim.minds].classes.0.rows must be 2 to 32 (it is 0)",
            ),
            (
                "0",
                r#"{"rows": 1}"#,
                "[tacsim.minds].classes.0.rows must be 2 to 32 (it is 1)",
            ),
            (
                "III",
                r#"{"rows": 33}"#,
                // 17b cut M5.1: `must be 1 to 32 (it is 33)` until the rider.
                "[tacsim.minds].classes.III.rows must be 2 to 32 (it is 33)",
            ),
            (
                "II",
                r#"{"rows": 10}"#,
                "[tacsim.minds].classes.II.rows must be at least class I's 12 (it is 10)",
            ),
            (
                "I",
                r#"{"odds_rows": 13}"#,
                "[tacsim.minds].classes.I.odds_rows must be 0 to 12 (it is 13)",
            ),
            (
                "0",
                r#"{"slots": 0}"#,
                "[tacsim.minds].classes.0.slots must be 1 to 6 (it is 0)",
            ),
            (
                "III",
                r#"{"slots": 7}"#,
                "[tacsim.minds].classes.III.slots must be 1 to 6 (it is 7)",
            ),
            (
                "II",
                r#"{"slots": 1}"#,
                "[tacsim.minds].classes.II.slots must be at least class I's 2 (it is 1)",
            ),
            (
                "0",
                r#"{"memory_keys": 0}"#,
                "[tacsim.minds].classes.0.memory_keys must be 1 to 128 (it is 0)",
            ),
            (
                "III",
                r#"{"memory_keys": 129}"#,
                "[tacsim.minds].classes.III.memory_keys must be 1 to 128 (it is 129)",
            ),
            (
                "II",
                r#"{"memory_keys": 31}"#,
                "[tacsim.minds].classes.II.memory_keys must be at least class I's 32 (it is 31)",
            ),
            (
                "I",
                r#"{"line_to": 9}"#,
                "[tacsim.minds].classes.I.line_to must be 0 to 8 (it is 9)",
            ),
            (
                "I",
                r#"{"profiles": 0}"#,
                "[tacsim.minds].classes.I.profiles must be 1 to 4 (it is 0)",
            ),
            (
                "I",
                r#"{"profiles": 5}"#,
                "[tacsim.minds].classes.I.profiles must be 1 to 4 (it is 5)",
            ),
        ] {
            assert_eq!(refused(class, figures), sentence, "{class} {figures}");
        }
        // A word that is no class, and a figure that is none, refuse at decode.
        let wrong = serde_json::from_str::<MindsConfig>(
            r#"{"odds_floor_permille": 100, "classes": {"IV": {}}}"#,
        )
        .expect_err("no class IV")
        .to_string();
        assert!(wrong.contains("`IV` is not a core class"), "{wrong}");
        assert!(
            serde_json::from_str::<MindsConfig>(
                r#"{"odds_floor_permille": 100, "classes": {"I": {"speed": 1}}}"#
            )
            .is_err()
        );
    }

    /// **17b cut M3.5** (doc 17 §9, D-UM-36): the pool's three keys default to
    /// doc 17's figures, an absent key reads its default, each is held to at
    /// least 1 by name, and the floor to at most the base.
    #[test]
    fn m3_the_pool_keys_default_and_validate_by_name() {
        use super::MindsConfig;
        let stock = MindsConfig::default();
        assert_eq!(
            (
                stock.pool_base_ops_s,
                stock.pool_per_unit_ops_s,
                stock.pool_floor_ops_s
            ),
            (500_000, 1_000, 120_000)
        );
        let absent: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100}"#).expect("decodes");
        assert_eq!(absent, stock, "an absent key reads its default");
        let set: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100, "pool_base_ops_s": 300000}"#)
                .expect("decodes");
        assert_eq!(set.pool_base_ops_s, 300_000);
        assert_eq!(set.validate(), Ok(()));
        for (config, sentence) in [
            (
                MindsConfig {
                    pool_base_ops_s: 0,
                    pool_floor_ops_s: 0,
                    ..MindsConfig::default()
                },
                "[tacsim.minds].pool_base_ops_s must be at least 1 (it is 0)",
            ),
            (
                MindsConfig {
                    pool_per_unit_ops_s: 0,
                    ..MindsConfig::default()
                },
                "[tacsim.minds].pool_per_unit_ops_s must be at least 1 (it is 0)",
            ),
            (
                MindsConfig {
                    pool_floor_ops_s: 0,
                    ..MindsConfig::default()
                },
                "[tacsim.minds].pool_floor_ops_s must be at least 1 (it is 0)",
            ),
            (
                MindsConfig {
                    pool_floor_ops_s: 500_001,
                    ..MindsConfig::default()
                },
                "[tacsim.minds].pool_floor_ops_s must be at most pool_base_ops_s (it is 500001 \
                 against 500000)",
            ),
        ] {
            assert_eq!(config.validate(), Err(sentence.to_owned()));
        }
        let even = MindsConfig {
            pool_floor_ops_s: 500_000,
            ..MindsConfig::default()
        };
        assert_eq!(
            even.validate(),
            Ok(()),
            "a floor equal to the base is lawful"
        );
    }
}

#[cfg(test)]
mod m4_tests {
    use super::*;

    /// **17b cut M4.1** (doc 17 §6 "Execution policy", D-UM-22):
    /// `[tacsim.minds].workers` reads 2 when absent, 1 to
    /// [`MAX_MIND_WORKERS`] by name, and is never carried (the snapshot
    /// carries `match_cfg`, not this block).
    #[test]
    fn m4_the_think_pool_s_workers_read_two_and_are_held_one_to_thirty_two() {
        assert_eq!(MindsConfig::default().workers, 2);
        let absent: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100}"#).expect("the block decodes");
        assert_eq!(absent.workers, 2, "absent reads the stock two");
        let four: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100, "workers": 4}"#)
                .expect("the block decodes");
        assert_eq!(four.workers, 4);
        assert_eq!(four.validate(), Ok(()));
        for (workers, sentence) in [
            (0, "[tacsim.minds].workers must be 1 to 32 (it is 0)"),
            (33, "[tacsim.minds].workers must be 1 to 32 (it is 33)"),
        ] {
            assert_eq!(
                MindsConfig {
                    workers,
                    ..MindsConfig::default()
                }
                .validate(),
                Err(sentence.to_owned())
            );
        }
        assert_eq!(
            MindsConfig {
                workers: MAX_MIND_WORKERS,
                ..MindsConfig::default()
            }
            .validate(),
            Ok(())
        );
    }

    /// **17b cut M4.2** (doc 17 §18): `[tacsim.minds].cpu_lens_ms` reads 5
    /// when absent, is held at least 1 by name, and is the reference figure
    /// the minds lane prints beside its computed lens — never a throttle.
    #[test]
    fn m4_2_the_cpu_lens_reads_five_and_is_held_at_least_one() {
        assert_eq!(MindsConfig::default().cpu_lens_ms, 5);
        let absent: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100}"#).expect("the block decodes");
        assert_eq!(absent.cpu_lens_ms, 5, "absent reads the stock five");
        let fifty: MindsConfig =
            serde_json::from_str(r#"{"odds_floor_permille": 100, "cpu_lens_ms": 50}"#)
                .expect("the block decodes");
        assert_eq!(fifty.cpu_lens_ms, 50);
        assert_eq!(fifty.validate(), Ok(()));
        assert_eq!(
            MindsConfig {
                cpu_lens_ms: 1,
                ..MindsConfig::default()
            }
            .validate(),
            Ok(())
        );
        assert_eq!(
            MindsConfig {
                cpu_lens_ms: 0,
                ..MindsConfig::default()
            }
            .validate(),
            Err("[tacsim.minds].cpu_lens_ms must be at least 1 (it is 0)".to_owned())
        );
    }
}

#[cfg(test)]
mod m6_class_caps {
    use super::MindsConfig;
    use crate::arena::core::CoreClass;

    #[test]
    fn operator_and_fill_caps_are_independent_and_explicit() {
        let stock = MindsConfig::default();
        assert_eq!(stock.max_class, CoreClass::III);
        assert_eq!(stock.fill_cap(None), CoreClass::I);
        assert_eq!(stock.fill_cap(Some(CoreClass::II)), CoreClass::II);
        let capped = MindsConfig {
            max_class: CoreClass::I,
            ..stock
        };
        assert_eq!(capped.fill_cap(Some(CoreClass::III)), CoreClass::I);
        assert_eq!(capped.fill_cap(Some(CoreClass::Zero)), CoreClass::Zero);
        let decoded: MindsConfig = serde_json::from_str(
            r#"{"odds_floor_permille":100,"max_class":"II","fill_class":"0"}"#,
        )
        .unwrap();
        assert_eq!(decoded.max_class, CoreClass::II);
        assert_eq!(decoded.fill_cap(None), CoreClass::Zero);
        assert_eq!(decoded.fill_cap(Some(CoreClass::III)), CoreClass::II);
        assert!(
            serde_json::from_str::<MindsConfig>(r#"{"odds_floor_permille":100,"max_class":"IV"}"#)
                .is_err()
        );
    }
}

#[cfg(test)]
mod recovery_capacity_tests {
    #[test]
    fn m6_snapshot_ceiling_refuses_unstoreable_or_too_small_values() {
        for value in [0, 16 * 1024 * 1024 - 1, 480 * 1024 * 1024 + 1, u64::MAX] {
            let cfg = super::ArenaConfig {
                snapshot_max_bytes: value,
                ..Default::default()
            };
            assert!(cfg.validate().unwrap_err().contains("snapshot_max_bytes"));
        }
        for value in [16 * 1024 * 1024, 480 * 1024 * 1024] {
            super::ArenaConfig {
                snapshot_max_bytes: value,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
    }
}
