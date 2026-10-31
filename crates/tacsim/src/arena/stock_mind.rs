//! **The native stock mind** (17b cut M1a.6; doc 17 §17 "Migrating `reflex.rs`
//! into Standard Doctrine", D-UM-3, D-UM-48; the M1a plan's §6).
//!
//! T1 of doc 17 §17's three columns: Standard Doctrine — the reflex
//! controller's state machine, its utility score, its goal precedence, its
//! clocks and its orders' obedience — written in Rust over a [`MindView`] and
//! a [`Memory`] instead of over the arena, and flying the hull through the
//! mind schedule (`DueKind::MindThink`/`MindLand`) and the motor
//! ([`crate::arena::motor`]) instead of the reflex's own tick. It is the oracle the
//! Rhai port (T2, 17b cut M2) is held to byte for byte, and the measured
//! native fast path P3 allows. [`crate::arena::mind::default_mind_runner`]
//! answers it.
//!
//! # `decide`, re-expressed arm for arm
//!
//! [`NativeStockMind`]'s think is the reflex controller's `decide` (its module,
//! `crate::arena::reflex`, deleted in 17b cut M4.6), read with
//! its comments beside it and ported block by block **in the same order**:
//! the `Disabled` edge; the standing order and its three early ends (16b cuts
//! AA.1, O.1, Z.2); the power arms; the `lost` edge; the order's steering; the
//! two `Withdraw` edges; the hostile edges with the utility score
//! ([`best_hostile`]) and the supporter's leash; the `no line` edge; the
//! states' bodies — `Patrol`'s list, `Approach`, `Engage`, `Withdraw` with its
//! stand, dwell and rally — and the search's and the hunt's bookkeeping. Where
//! `decide` wrote a `ReflexPlan` it emits intents, and
//! where it wrote the hull's `MotorRow` it emits a goal:
//!
//! * **`Goal(Cell)` only when the goal CHANGED** — the memory's `goal` is the
//!   goal the mind believes the motor holds — so the motor keeps its route
//!   between thinks;
//! * **`Goal(Hold)`** where `decide` stopped the hull (`speed 0`): the motor
//!   lets its goal go and the hull stops (the memory's `goal` is null);
//! * **arrival** is the motor's persistent completion for the requested
//!   goal, even when formation assigned a different physical destination;
//! * a **NoRoute outcome** matching the memory's current goal performs the
//!   reflex's `unreachable` edge in its order — the `Advance` and `Support`
//!   ends, the search's and the hunt's give-ups, the Hold retry — and then the
//!   waypoint fallback ([`fallback_waypoint`]) as the next goal, one think
//!   later than the reflex's in-tick fallback (departure (i); the divergence
//!   `fallback`).
//!
//! # The emission order, and its two rules
//!
//! `Startup`, `Condition`, `Goal`, `Heading`, `Speed`, `Lock`, `Fire`. A think
//! that emits `Goal(Cell)` emits **no** `Heading` or `Speed`: the motor steers
//! from the landing on, and the landing's disengage rule (a mind's own heading
//! lets go of its goal, 17b cut M1a.5) would throw the goal away. A think that
//! **stands** emits `Goal(Hold)` first and then the facing `Heading` — *stop
//! here and face there*; the goal is already gone, so the disengage rule has
//! nothing to let go of. The stock mind never emits a `Speed` at all
//! (`Goal(Hold)` is its stop) and answers `Startup` with its disarm alone —
//! and, since 17b cut M1b.3, with `Startup` alone when no slot is armed.
//! **17b cut M1b.2**: a think of an armed hull answers its slots
//! ([`arming_for`]) — one `Condition(Arm)` per weapon of its first `slots`
//! (since 17b cut M3.1 the think's row's `slots`: Class I's two on every
//! shipped chassis), or one `Condition(Disarm)`; **17b cut M1b.3**
//! answers only where its view's `armed` rows differ from what it wants — so
//! a think emits at most `4 + slots` intents (`Condition` × 2 at Class I,
//! `Goal(Hold)`, `Heading`, `Lock`, `Fire`), well inside
//! [`crate::arena::mind::MIND_INTENTS_MAX`]; a `Fire` is planned only beside a
//! fresh `Lock` and cites its slot (`engage_orders`), and the armed slot owns
//! every other shot.
//!
//! # What it cannot see, and does instead (doc 17 §17 "The named divergences")
//!
//! An observer-pure port cannot copy four of the reflex's reads, and does not
//! put a leak back to try: the utility score breaks a tie by [`ContactId`]
//! and not by the object behind it (`tiebreak`); the line a stand or an
//! `Approach` asks for is traced to the **plot's cell** with [`line_to`] and
//! not to the target's live pose (`line`); a supporter prefers only the
//! order's own hint and not what its charge is fighting (`support`); and a
//! contact reads dead off the view's own `reads_dead` (`reads_dead`, value-
//! identical). Beside them, the scheduling and resolution changes the M1a
//! plan names: a think on the lattice landing one substep later (`landing`;
//! its shot is planned for the landing's millisecond, as the reflex plans its
//! own for the resolver's — see `engage_orders`);
//! the `no line` edge read off a `blocked`/`cover` miss notice (`no_line`);
//! the motor's fan at landing (`fan`); an order's hint re-keyed when it was
//! written (`order_hint`); an order the mind cannot end, only stop obeying
//! (`order_end`); the view's whole metres (`range_round`), its plot cells
//! (`plot_cell`) and whole degrees (`deg_round`); and the fallback a think
//! late (`fallback`). `crates/tacsim/tests/twin.rs` counts every one of them
//! against the reflex's own readers.
//!
//! The cost-row residual remains: every picker asks for [`CostRow::Tracked`].
//! M5r supplies public Hold geometry and the unit's current motor result to
//! production thinks. Match ownership, capture progress and the private fan
//! destination remain outside the view. The accepted mind-side `order_end`
//! behavior remains: an ended order is ignored by its expiry identity.
//!
//! # No draw, no clock
//!
//! It draws nothing: every choice is a total order over integers, and the walk
//! keys on the view's own `unit` (departure (e)). It reads no wall clock; the
//! twin lane times it from the test.

use mush_atlas::Cover;
use smol_str::SmolStr;

use crate::arena::card::StockCard;
use crate::arena::conditions::{Arming, FireCondition, SlotTarget};
use crate::arena::damage::{BLOCKED, COVER};
use crate::arena::doctrine::{
    EXPOSURE_FLANK, EXPOSURE_FRONT, EXPOSURE_REAR, MachineState, Ranges, SCREEN_HALF_ARC_MDEG,
    STAND_TICKS, SUPPORT_LEASH_CELLS, SUPPORT_STATION_CELLS, arc_between, stand_off_want,
};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{
    MemValue, Memory, MindRunner, MindView, ThinkJob, ThinkResult, ViewContact, decode_memory,
    encode_memory,
};
use crate::arena::motor::{
    Walk, entry_cell, fallback_waypoint, far_entry_cell, hold_arrived, patrol_step, stand_off_cell,
};
use crate::arena::perception::{Aspect, DriveOutcome, FrameNotice, FrameOrder, MindObjective};
use crate::arena::state::{
    ArenaDefinition, ContactId, CostRow, DEFAULT_SENSOR_HEIGHT_CM, HoldObjective, Power,
};
use crate::arena::weapons::{
    FrameArming, FrameWeapon, fight_inner_cm_of, fight_limit_cm_of, loaded_weapon_of,
};
use crate::config::ReflexConfig;
use crate::deliberation::order::RangeBand;

// ---------------------------------------------------------------------------
// The memory (D-UM-30's shape)
// ---------------------------------------------------------------------------

/// **The fifteen keys of the stock mind's memory** (17b cut M1a.6), in the
/// order [`StockMemory`]'s fields are declared — and nothing else: a test
/// pins the list.
pub const STOCK_MEMORY_KEY_NAMES: [&str; 15] = [
    "state",
    "target",
    "goal",
    "waypoint",
    "from_waypoint",
    "last_plot",
    "unheld_since_ms",
    "quiet_since_ms",
    "stand_until_ms",
    "hold_retry_ms",
    "rallied",
    "hunt",
    "no_line",
    "ignore_until_ms",
    "ordered",
];

/// **What the stock mind remembers between thinks** (17b cut M1a.6; doc 17
/// §8) — a typed view over its [`Memory`], read with `StockMemory::from` and
/// written back with `Memory::from`, so what the arena stores is the
/// canonical map of [`STOCK_MEMORY_KEY_NAMES`] (D-UM-30) and 17b cut M2's Rhai
/// port can write the same bytes.
///
/// The reflex's `ReflexUnit` (deleted with it in 17b cut M4.6) and the driving half of
/// its `MotorRow` that the decision reads, field for field: the motor's own
/// route, search stamp and stops are the motor's and are not remembered here.
/// Two fields are new: `goal` is the goal the mind believes the motor holds
/// (null once it let the hull hold), and `ignore_until_ms` is how a mind ends
/// an order it cannot journal the end of (the reflex's 16b cuts AA.1, O.1 and
/// Z.2 ends; the arena's own expiry still comes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StockMemory {
    /// `state`: where it is in the machine ([`MachineState::code`]).
    pub state: MachineState,
    /// `target`: the hostile it settled on, by its own row's [`ContactId`]
    /// (stored as the id's 64 bits read as an `i64`).
    pub target: Option<ContactId>,
    /// `goal`: the cell the motor was last handed, `[x, y]`.
    pub goal: Option<(u16, u16)>,
    /// `waypoint`: the node the walk is going to.
    pub waypoint: Option<u16>,
    /// `from_waypoint`: the node it came from.
    pub from_waypoint: Option<u16>,
    /// `last_plot`: where it last saw its target and when, `[x, y, t_ms]`.
    pub last_plot: Option<((u16, u16), i64)>,
    /// `unheld_since_ms`: since when it has held nothing hostile.
    pub unheld_since_ms: Option<i64>,
    /// `quiet_since_ms`: a home `Withdraw`'s quiet clock.
    pub quiet_since_ms: Option<i64>,
    /// `stand_until_ms`: a cornered `Withdraw`'s dwell.
    pub stand_until_ms: Option<i64>,
    /// `hold_retry_ms`: a Hold spot given up until then.
    pub hold_retry_ms: Option<i64>,
    /// `rallied`: it has come back into the fight once.
    pub rallied: bool,
    /// `hunt`: the far zone's cell it is crossing to, `[x, y]`.
    pub hunt: Option<(u16, u16)>,
    /// `no_line`: a round of its own was stopped by the ground.
    pub no_line: bool,
    /// `ignore_until_ms`: **the standing order this hull has ended**, by the
    /// `expires_ms` it carried (17b cut M1b.5, the lead's ruling on M1a.6's
    /// residual (10); M1a.6 kept a Bool, `ignore_order`). A think obeys the
    /// view's order only when its `expires_ms` is not this one — so a NEWER
    /// order (any other deadline) is obeyed at once, where the Bool ignored
    /// every order until the view carried none. Written by the three early
    /// ends (and the `unreachable` edge's two) as the ended order's
    /// `expires_ms`; cleared when the view carries no order or an order with
    /// another identity. DT orders use `ignore_identity`, encoded as STRING in
    /// this same key, because a standing deadline cannot identify an order.
    pub ignore_until_ms: Option<i64>,
    /// Explicit DT identity of an ended order. Encoded as a STRING in the same
    /// `ignore_until_ms` memory key; legacy INT memory stays byte-identical.
    pub ignore_identity: Option<smol_str::SmolStr>,
    /// `ordered`: **the `Withdraw` is an order's** (17b cut M4.3; the
    /// fifteenth key). The expiry's restoration (a healthy, not-dry `Withdraw`
    /// under no order goes back to `Patrol`) reads it, so only a `Withdraw` an
    /// order put the hull in is restored when the order is gone: a released
    /// seat's seeded `Withdraw` (`Arena::withdraw_seed`, 16b cut N.1's
    /// drives-home) and a dry or hurt one are left to Q.7's rally. Written
    /// `true` at the order's `Withdraw` edge, `false` at the dry/hurt edge, by
    /// the restoration and by the seed.
    pub ordered: bool,
}

/// An integer key, or `None` when absent, unit or not an INT.
fn int_at(memory: &Memory, key: &str) -> Option<i64> {
    match memory.get(key) {
        Some(MemValue::Int(value)) => Some(*value),
        _ => None,
    }
}

/// An array key of integers, or `None`.
fn ints_at<'m>(memory: &'m Memory, key: &str) -> Option<&'m [i64]> {
    match memory.get(key) {
        Some(MemValue::Ints(items)) => Some(items.as_slice()),
        _ => None,
    }
}

/// A cell key, `[x, y]`.
fn cell_at(memory: &Memory, key: &str) -> Option<(u16, u16)> {
    let items = ints_at(memory, key)?;
    let (Some(x), Some(y)) = (items.first(), items.get(1)) else {
        return None;
    };
    Some((u16::try_from(*x).ok()?, u16::try_from(*y).ok()?))
}

/// A cell, as its memory value.
fn cell_value(cell: Option<(u16, u16)>) -> MemValue {
    cell.map_or(MemValue::Unit, |(x, y)| {
        MemValue::Ints(vec![i64::from(x), i64::from(y)])
    })
}

/// An integer, as its memory value.
fn int_value(value: Option<i64>) -> MemValue {
    value.map_or(MemValue::Unit, MemValue::Int)
}

/// The state a code names, `Startup` for none — over the six the doctrine
/// writes ([`MachineState::DOCTRINE`], 17b cut M4.6), so a memory never
/// decodes to [`MachineState::Unknown`] (code 6 reads `Startup`, as every
/// code outside the six always did).
fn state_of(code: Option<i64>) -> MachineState {
    MachineState::DOCTRINE
        .iter()
        .copied()
        .find(|state| Some(i64::from(state.code())) == code)
        .unwrap_or_default()
}

impl From<&Memory> for StockMemory {
    /// Read the fifteen keys; an absent, unit or ill-shaped key is the
    /// field's default (an empty memory is a fresh sortie). 17b cut M2.2:
    /// over `mush_mind`'s [`MemValue`] — an `Int` for every integer (the
    /// state's code, the two waypoints, the clocks, `ignore_until_ms` and
    /// `target`'s bit-cast), or a STRING for an explicit ignored order identity;
    /// `Ints` for `goal`, `hunt` and `last_plot`, `Bool`
    /// for `rallied` and `no_line`.
    fn from(memory: &Memory) -> StockMemory {
        let flag = |key: &str| matches!(memory.get(key), Some(MemValue::Bool(true)));
        let u16_at = |key: &str| int_at(memory, key).and_then(|value| u16::try_from(value).ok());
        StockMemory {
            state: state_of(int_at(memory, "state")),
            target: int_at(memory, "target")
                .map(|value| ContactId(u64::from_ne_bytes(value.to_ne_bytes()))),
            goal: cell_at(memory, "goal"),
            waypoint: u16_at("waypoint"),
            from_waypoint: u16_at("from_waypoint"),
            last_plot: ints_at(memory, "last_plot").and_then(|items| {
                let (Some(x), Some(y), Some(at)) = (items.first(), items.get(1), items.get(2))
                else {
                    return None;
                };
                Some(((u16::try_from(*x).ok()?, u16::try_from(*y).ok()?), *at))
            }),
            unheld_since_ms: int_at(memory, "unheld_since_ms"),
            quiet_since_ms: int_at(memory, "quiet_since_ms"),
            stand_until_ms: int_at(memory, "stand_until_ms"),
            hold_retry_ms: int_at(memory, "hold_retry_ms"),
            rallied: flag("rallied"),
            hunt: cell_at(memory, "hunt"),
            no_line: flag("no_line"),
            ignore_until_ms: int_at(memory, "ignore_until_ms"),
            ignore_identity: match memory.get("ignore_until_ms") {
                Some(MemValue::Str(identity)) => Some(smol_str::SmolStr::new(identity)),
                _ => None,
            },
            ordered: flag("ordered"),
        }
    }
}

impl From<StockMemory> for Memory {
    /// Write all fifteen keys, a `Unit` for every field that holds nothing —
    /// so one memory has one encoding, whichever think wrote it. 17b cut
    /// M2.2: as `mush_mind`'s [`MemValue`]s — `Int` for every integer,
    /// `Ints(vec![x, y])` for `goal` and `hunt`, `Ints(vec![x, y, t])` for
    /// `last_plot`, `Bool` for `rallied` and `no_line`.
    fn from(stock: StockMemory) -> Memory {
        let mut memory = Memory::new();
        let mut put = |key: &str, value: MemValue| {
            memory.insert(key.to_owned(), value);
        };
        put("state", MemValue::Int(i64::from(stock.state.code())));
        put(
            "target",
            int_value(
                stock
                    .target
                    .map(|id| i64::from_ne_bytes(id.0.to_ne_bytes())),
            ),
        );
        put("goal", cell_value(stock.goal));
        put("waypoint", int_value(stock.waypoint.map(i64::from)));
        put(
            "from_waypoint",
            int_value(stock.from_waypoint.map(i64::from)),
        );
        put(
            "last_plot",
            stock.last_plot.map_or(MemValue::Unit, |((x, y), at)| {
                MemValue::Ints(vec![i64::from(x), i64::from(y), at])
            }),
        );
        put("unheld_since_ms", int_value(stock.unheld_since_ms));
        put("quiet_since_ms", int_value(stock.quiet_since_ms));
        put("stand_until_ms", int_value(stock.stand_until_ms));
        put("hold_retry_ms", int_value(stock.hold_retry_ms));
        put("rallied", MemValue::Bool(stock.rallied));
        put("hunt", cell_value(stock.hunt));
        put("no_line", MemValue::Bool(stock.no_line));
        put(
            "ignore_until_ms",
            stock.ignore_identity.map_or_else(
                || int_value(stock.ignore_until_ms),
                |identity| MemValue::Str(identity.to_string()),
            ),
        );
        put("ordered", MemValue::Bool(stock.ordered));
        memory
    }
}

// ---------------------------------------------------------------------------
// The kernels (doc 17 §3 "The map is public", §7's kernel list)
// ---------------------------------------------------------------------------

/// **A contact row's held range**, centimetres: the view's whole metres back
/// to the arena's unit (the divergence `range_round`).
#[must_use]
pub const fn row_range_cm(row: &ViewContact) -> u64 {
    row.range_m.saturating_mul(100)
}

/// **The view's own sight**, centimetres, off its whole metres.
#[must_use]
pub fn sight_cm(view: &MindView) -> u32 {
    u32::try_from(view.sight_m.saturating_mul(100)).unwrap_or(u32::MAX)
}

/// **The weapon this hull would actually fire** — the lifted
/// [`loaded_weapon_of`] over the view's weapon rows, so the mind and the hull
/// pick one gun.
#[must_use]
pub fn loaded_weapon(view: &MindView) -> Option<&FrameWeapon> {
    loaded_weapon_of(
        view.weapons
            .iter()
            .map(|weapon| (weapon.id.as_str(), weapon.rounds)),
    )
    .and_then(|at| view.weapons.get(at))
}

/// **The furthest this hull fights at**, centimetres — the lifted
/// [`fight_limit_cm_of`] over the loaded weapon's last band edge and the
/// view's sight.
#[must_use]
pub fn fight_limit_cm(view: &MindView) -> u64 {
    fight_limit_cm_of(
        loaded_weapon(view).and_then(|weapon| weapon.bands_cm.last().copied()),
        sight_cm(view),
    )
}

/// **Armour plus internals, per mille** (`Arena::health_permille`, the
/// reflex's `reflex_health_permille` until 17b cut M4.6a, over the view's own
/// sections): every facet's
/// armour and every section's internals, current against maximum; a hull
/// with none reads `1000`, a wreck `0`.
#[must_use]
pub fn health_permille(view: &MindView) -> u32 {
    if view.wreck {
        return 0;
    }
    let mut have: i64 = 0;
    let mut full: i64 = 0;
    for section in &view.sections {
        if let Some((current, max)) = section.internals {
            have = have.saturating_add(current.max(0));
            full = full.saturating_add(max.max(0));
        }
        for facet in &section.facets {
            have = have.saturating_add(facet.armor.max(0));
            full = full.saturating_add(facet.armor_max.max(0));
        }
    }
    if full <= 0 {
        return 1_000;
    }
    u32::try_from(have.saturating_mul(1_000) / full)
        .unwrap_or(1_000)
        .min(1_000)
}

/// **Whether a hull that had a gun has nothing left to fire** (the reflex's
/// `is_dry`, over the view): some weapon row holds a load, and every row is
/// empty. A hull with no weapon rows is never dry — it is unarmed.
#[must_use]
pub fn is_dry(view: &MindView) -> bool {
    view.weapons.iter().any(|weapon| weapon.rounds_max > 0)
        && view.weapons.iter().all(|weapon| weapon.rounds == 0)
}

/// **Whether firing `weapon` is held back by heat** (the reflex's
/// `would_overheat`, the engine's own line): a weapon that makes heat, on a
/// hull already at its ceiling.
#[must_use]
pub fn would_overheat(view: &MindView, weapon: &FrameWeapon) -> bool {
    weapon.heat_per_shot != 0 && view.heat.is_some_and(|heat| heat.overheated())
}

/// **Whether this hull holds any live hostile at all** (the reflex's
/// `holds_a_hostile`, over the view): a classified contact of another side
/// that does not read dead.
#[must_use]
pub fn holds_a_hostile(view: &MindView) -> bool {
    view.contacts.iter().any(|row| {
        row.relation == crate::arena::party::Relation::Hostile && row.reads_dead != Some(true)
    })
}

/// **The three distances a think fights at** (the reflex's `ranges`, over the
/// view: the loaded weapon's band edges, its rounds and the view's sight in
/// whole metres — the divergence `range_round`).
///
/// **17b cut M2.2** (17b §1 (iv)): over the band per mille
/// (`engage_band_permille`, the doctrine's or the card's) instead of the
/// whole `ReflexConfig`, so 17b cut M2.3's `ranges` kernel wraps this one
/// body with the card's figure. Byte-neutral: T1 passes
/// `cfg.engage_band_permille`, the one field the body read.
#[must_use]
pub fn view_ranges(
    view: &MindView,
    engage_band_permille: u32,
    order: Option<&FrameOrder>,
) -> Ranges {
    let loaded = loaded_weapon(view);
    let bands: &[u32] = loaded.map_or(&[], |weapon| weapon.bands_cm.as_slice());
    let sight = sight_cm(view);
    let order_cap = order
        .and_then(|order| order.max_range_band)
        .map_or(u64::MAX, |band| band.ceiling_cm(bands, sight));
    let cap_inner = fight_inner_cm_of(bands.first().copied(), sight).min(order_cap);
    Ranges {
        engage_cm: cap_inner.saturating_mul(u64::from(engage_band_permille)) / 1_000,
        release_cm: cap_inner,
        stand_cm: RangeBand::Close
            .ceiling_cm(bands, sight)
            .min(order_cap)
            .min(cap_inner),
        limit_cm: fight_limit_cm(view),
    }
}

/// **The utility score's three weights** (doc 16 §6; 17b cut M2.2, 17b §1
/// (iv)): the reflex doctrine's `weight_range`, `weight_facet` and
/// `weight_health` — what [`best_hostile_ranked`] reads of a
/// `ReflexConfig`, and all it reads, so 17b cut M2.3's `best_hostile`
/// kernel wraps the one body with a script's own weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weights {
    /// Per mille of the fight limit, per point.
    pub range: u32,
    /// Per exposure point.
    pub facet: u32,
    /// Per mille of the hull's own health.
    pub health: u32,
}

impl Weights {
    /// **The three weights of a doctrine** (T1's call sites pass this).
    #[must_use]
    pub const fn of(cfg: &ReflexConfig) -> Weights {
        Weights {
            range: cfg.weight_range,
            facet: cfg.weight_facet,
            health: cfg.weight_health,
        }
    }
}

/// **Doc 16 §6's utility score over the view, every candidate ranked** — what
/// [`best_hostile`] takes the first of, and what the twin lane reads a tie
/// off: `(score, ContactId)` ascending over the classified hostiles that do
/// not read dead and that `admit` accepts.
///
/// The reflex's `best_hostile_where`, term for term: the range as per mille of
/// [`fight_limit_cm`] off the row's **held** `range_m`; the exposure off the
/// row's `aspect` (rear, front, else a flank); the hull's own health, the same
/// addend for all; the order's hint scoring `0` outright. Empty on a hull with
/// nothing loaded (it has no target, which is how the fire half of an unarmed
/// hull is absent: D-UM-48). **17b cut M2.2**: over [`Weights`] instead of
/// the whole `ReflexConfig` (byte-neutral; [`Weights::of`]).
#[must_use]
pub fn best_hostile_ranked(
    view: &MindView,
    weights: &Weights,
    health_permille: u32,
    prefer: Option<ContactId>,
    admit: impl Fn(&ViewContact) -> bool,
) -> Vec<(u64, ContactId)> {
    let reach = fight_limit_cm(view);
    if reach == 0 {
        return Vec::new();
    }
    let health_term = weights.health.saturating_mul(health_permille.min(1_000));
    let mut ranked: Vec<(u64, ContactId)> = view
        .contacts
        .iter()
        .filter(|row| {
            row.relation == crate::arena::party::Relation::Hostile
                && row.reads_dead != Some(true)
                && admit(row)
        })
        .map(|row| {
            let range_permille = u32::try_from(row_range_cm(row).saturating_mul(1_000) / reach)
                .unwrap_or(u32::MAX)
                .min(1_000);
            let exposure = match row.aspect {
                Aspect::Rear => EXPOSURE_REAR,
                Aspect::Front => EXPOSURE_FRONT,
                Aspect::Left | Aspect::Right => EXPOSURE_FLANK,
            };
            let score = if prefer == Some(row.id) {
                0
            } else {
                u64::from(weights.range.saturating_mul(range_permille))
                    .saturating_add(u64::from(weights.facet.saturating_mul(exposure)))
                    .saturating_add(u64::from(health_term))
            };
            (score, row.id)
        })
        .collect();
    ranked.sort_unstable();
    ranked
}

/// **The hostile this hull should fight**, or `None` (doc 17 §7's kernel
/// `best_hostile(w_range, w_facet, w_health, prefer)`): the first of
/// [`best_hostile_ranked`] — ties broken by [`ContactId`] (the divergence
/// `tiebreak`: the reflex breaks them by the object behind the row).
#[must_use]
pub fn best_hostile(
    view: &MindView,
    weights: &Weights,
    health_permille: u32,
    prefer: Option<ContactId>,
    admit: impl Fn(&ViewContact) -> bool,
) -> Option<ContactId> {
    best_hostile_ranked(view, weights, health_permille, prefer, admit)
        .first()
        .map(|(_, id)| *id)
}

/// **Whether a shot from `own` would have a line to the plot's `cell`** (doc 17
/// §7's kernel `line_to(contact)`): `mush_atlas::trace_cells` over the ground
/// from the hull's eye to the cell's eye, both [`DEFAULT_SENSOR_HEIGHT_CM`]
/// above the ground there; a line that is clear or partial and leaves the
/// cell short of full cover. The reflex's `reflex_has_line` traces the same
/// walk to the target's **live** cell at the hull's own eye height — the
/// divergence `line`, with the eye height folded into it (every stock hull's
/// eye is the default).
#[must_use]
pub fn line_to(def: &ArenaDefinition, own: (u16, u16), cell: (u16, u16)) -> bool {
    let eye = i32::try_from(DEFAULT_SENSOR_HEIGHT_CM).unwrap_or(i32::MAX);
    let trace = mush_atlas::trace::trace_cells(
        &def.terrain,
        mush_atlas::trace::Eye {
            cell: own,
            height_cm: eye,
        },
        mush_atlas::trace::Eye {
            cell,
            height_cm: eye,
        },
    );
    trace.sight.is_clear_or_partial() && trace.target_cover != Cover::Full
}

/// [`line_to`] for two cells that may be off the window: off the window is a
/// line, the answer the reflex's `fire_line` gives a trace it cannot take.
///
/// `pub(crate)` since 17b cut M2.3: the unit-mind API's `line_to` kernel
/// (`crate::minds`) wraps this one body — never a second.
pub(crate) fn line_between(
    def: &ArenaDefinition,
    own: Option<(u16, u16)>,
    cell: Option<(u16, u16)>,
) -> bool {
    match (own, cell) {
        (Some(own), Some(cell)) => line_to(def, own, cell),
        _ => true,
    }
}

/// **The centre of a plot's cell** at the ground's height there, `(x_cm, y_cm,
/// z_cm)` — the point the view holds a contact at (the divergence
/// `plot_cell`).
#[must_use]
pub fn cell_point(def: &ArenaDefinition, cell: (u16, u16)) -> (i64, i64, i32) {
    let side = i64::from(def.terrain.cell_cm());
    let z = def
        .terrain
        .get(cell.0, cell.1)
        .map_or(0, |ground| i32::from(ground.elev_cm));
    (
        i64::from(cell.0)
            .saturating_mul(side)
            .saturating_add(side / 2),
        i64::from(cell.1)
            .saturating_mul(side)
            .saturating_add(side / 2),
        z,
    )
}

/// The view's own pose, `(x_cm, y_cm, z_cm)` (`pub(crate)` since 17b cut
/// M2.3, for the unit-mind API's `range_to`).
pub(crate) fn own_point(view: &MindView) -> (i64, i64, i32) {
    (
        view.x_cm,
        view.y_cm,
        i32::try_from(view.z_cm).unwrap_or(i32::MAX),
    )
}

/// The stand-off cell `want_cm` back from a row's plot cell, as a tracked hull
/// (`None` for a row whose plot is off the window). `pub(crate)` since 17b cut
/// M2.3: the unit-mind API's `stand_off_cell` kernel wraps this one body.
pub(crate) fn stand_off_of(
    def: &ArenaDefinition,
    view: &MindView,
    row: &ViewContact,
    want_cm: u64,
) -> Option<(u16, u16)> {
    let cell = row.cell?;
    stand_off_cell(
        def,
        CostRow::Tracked,
        own_point(view),
        cell_point(def, cell),
        want_cm,
    )
}

/// The row a contact id names, if the view holds it. `pub(crate)` since 17b
/// cut M2.3: the unit-mind API's `contact` kernel and every station and kernel
/// that takes a contact id look a row up through this one function.
pub(crate) fn contact(view: &MindView, id: ContactId) -> Option<&ViewContact> {
    view.contacts.iter().find(|row| row.id == id)
}

/// The contact an order points the hull at: an `Engage`'s re-keyed hint. A
/// `Support`'s charge designation is not on the view (the divergence
/// `support`), so a supporter prefers nothing.
fn hint(order: Option<&FrameOrder>) -> Option<ContactId> {
    match order.map(|order| order.objective) {
        Some(MindObjective::Engage { target }) => target,
        _ => None,
    }
}

/// **The view's standing order, unless this hull has ended it** (17b cut
/// M1b.5; the lead's ruling on M1a.6's residual (10)): the order whose
/// `expires_ms` is not the memory's `ignore_until_ms`.
fn obeyed<'v>(view: &'v MindView, mem: &StockMemory) -> Option<&'v FrameOrder> {
    view.order.as_ref().filter(|order| !mem.ignores(order))
}

impl StockMemory {
    fn ignores(&self, order: &FrameOrder) -> bool {
        match &order.identity {
            Some(identity) => self.ignore_identity.as_ref() == Some(identity),
            None => {
                self.ignore_identity.is_none() && Some(order.expires_ms) == self.ignore_until_ms
            }
        }
    }

    fn ignore(&mut self, order: Option<&FrameOrder>) {
        self.ignore_identity = order.and_then(|order| order.identity.clone());
        self.ignore_until_ms = order
            .filter(|order| order.identity.is_none())
            .map(|order| order.expires_ms);
    }
}

/// **Whether an `Advance { to }` has reached the Hold square** (the reflex's
/// `advance_reached_the_hold`): `to` inside the square, and the hull inside
/// it too or the motor has completed that requested goal. An outside hull
/// merely near the edge has not completed a possibly fanned destination.
pub(crate) fn advance_reached_the_hold(
    hold: &HoldObjective,
    to: (u16, u16),
    here: Option<(u16, u16)>,
    motor: Option<&crate::arena::step::FrameMotor>,
) -> bool {
    hold.square_contains(to)
        && here.is_some_and(|cell| {
            hold.square_contains(cell)
                || motor.is_some_and(|motor| {
                    motor.requested_goal == to && motor.outcome == Some(DriveOutcome::Arrived)
                })
        })
}

// ---------------------------------------------------------------------------
// The mind
// ---------------------------------------------------------------------------

/// **The native stock mind** (17b cut M1a.6; doc 17 §17, D-UM-3): Standard
/// Doctrine over the view and the memory, on the match's frozen doctrine.
#[derive(Clone, Debug)]
pub struct NativeStockMind {
    /// The doctrine it thinks on — `[tacsim.arena.reflex]`, the reflex's own.
    cfg: ReflexConfig,
}

impl NativeStockMind {
    /// A stock mind on this doctrine.
    #[must_use]
    pub fn with(cfg: &ReflexConfig) -> NativeStockMind {
        NativeStockMind { cfg: cfg.clone() }
    }

    /// The doctrine it thinks on.
    #[must_use]
    pub const fn config(&self) -> &ReflexConfig {
        &self.cfg
    }
}

impl Default for NativeStockMind {
    fn default() -> NativeStockMind {
        NativeStockMind::with(&ReflexConfig::default())
    }
}

impl MindRunner for NativeStockMind {
    /// One think: the memory decoded, `decide` ported, the memory encoded.
    /// The public Hold objective comes from the view. **17b cut M1b.2**:
    /// `job.card` is
    /// read for the fire discipline ([`arming_for`]); **17b cut M1b.5**: and
    /// for every mapped term, through the overlay (`decide`'s head).
    ///
    /// **17b cut M2.2**: the view and the map are read through the job's
    /// `Arc`s; `job.seed` and `job.dropped` are not read (T1 draws nothing,
    /// and it IS the stand-in a dropped hull falls back to); it answers no
    /// fault and no trace.
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let Some(card) = job.card.stock() else {
            return ThinkResult {
                fault: Some(super::mind::ThinkFault {
                    cause: "profile".into(),
                }),
                memory: job.memory.to_vec(),
                ..ThinkResult::default()
            };
        };
        let mut memory = StockMemory::from(&decode_memory(job.memory));
        let intents = self.decide(
            job.view,
            &mut memory,
            job.map,
            job.view.objective.as_ref(),
            job.t_ms,
            job.lands_ms,
            card,
            job.profile.slots,
        );
        ThinkResult {
            rule_sample: None,
            diagnostics: None,
            intents,
            memory: encode_memory(&Memory::from(memory)),
            fault: None,
            trace: Vec::new(),
        }
    }
}

/// What one think decided before any of it is emitted — the reflex's
/// `ReflexPlan` with the speed read as a stop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Plan {
    /// The hull stands this think: `Goal(Hold)`.
    stop: bool,
    /// The facing a stand turns onto, millidegrees.
    heading_mdeg: Option<u32>,
    /// The lock to take.
    lock: Option<ContactId>,
    /// The weapon to fire, and the slot its shot cites (17b cut M1b.2: only on
    /// a fresh lock; 17b cut M1b.3: the number its view row's `armed` shows,
    /// else `i + 1` for a weapon this think arms; see `engage_orders`).
    fire: Option<(SmolStr, u8)>,
    /// **17b cut M1b.2** (D-UM-18): `engage_orders` was reached with a row and
    /// its gates passed (power online, not under `hold_fire`, not dry) — the
    /// ceiling that admitted the row, centimetres (`release_cm`, or a
    /// withdraw-stand's `stand_cm` en route). `Some` arms the slots
    /// ([`arming_for`]); `None` disarms them.
    arm_ceiling_cm: Option<u64>,
    /// **17b cut M3.1** (D-UM-35): the condition slots the think is held to,
    /// its row's `slots` ([`ThinkJob::profile`]) — Class I's two on every
    /// shipped chassis. Read by `engage_orders`' citation and handed to
    /// [`arming_for`].
    slots: usize,
}

/// One think's reading of the motor's retained, goal-keyed completion.
#[derive(Clone, Copy, Debug)]
struct Drive {
    /// The goal the motor said it reached, or `None`.
    reached: Option<(u16, u16)>,
}

impl Drive {
    /// Arrival at the requested cell is the motor's retained completion,
    /// including when formation assigned a different physical destination.
    fn arrived(self, cell: (u16, u16)) -> bool {
        self.reached == Some(cell)
    }
}

impl NativeStockMind {
    /// **`decide`, re-expressed** (see the module head): walk the memory one
    /// step over `view` and answer the intents, in emission order. `hold` is
    /// the public Hold geometry from the view; `t_ms` is the think's lattice millisecond,
    /// the clock every memory stamp is written on, as the reflex writes its
    /// tick's; `lands_ms` is the think's landing millisecond
    /// ([`ThinkJob::lands_ms`]), the one its shot is resolved at; `card` is
    /// the hull's stock card ([`ThinkJob::card`]), read for the fire
    /// discipline its slots are armed on (17b cut M1b.2).
    ///
    /// **17b cut M1b.5 — the card through the overlay** (doc 17 §11, D-UM-43):
    /// the think runs on `card.as_reflex(&self.cfg)` — the match's doctrine
    /// with the card's mapped terms written over it (the engage band, the
    /// withdraw and stand edges, the rally, the search, the hunt and
    /// `prefer`'s weights) — so EVERY doctrine read of the think and of its
    /// helpers (`search_goal`, `hunt_goal`, `withdraw_stand`, `stand`, the
    /// utility score, `view_ranges`) reads the card. On Line that is the
    /// match's own doctrine byte for byte (`line(cfg).as_reflex(cfg) == cfg`:
    /// the anchor). `rally_ms` arrives as `u32::MAX` for a card's `rally_s 0`
    /// and the rally's `t − since >= rally_ms` (an `i64` compare) never holds.
    /// The eleven sliders marked `[M2]` are not read (`crate::arena::card`'s
    /// head).
    #[allow(clippy::too_many_arguments)]
    fn decide(
        &self,
        view: &MindView,
        mem: &mut StockMemory,
        def: &ArenaDefinition,
        hold: Option<&HoldObjective>,
        t_ms: i64,
        lands_ms: i64,
        card: &StockCard,
        slots: usize,
    ) -> Vec<IntentKind> {
        let doctrine = NativeStockMind {
            cfg: card.as_reflex(&self.cfg),
        };
        doctrine.decide_on_card(view, mem, def, hold, t_ms, lands_ms, card, slots)
    }

    /// `decide`'s body, on a doctrine the card is already written over
    /// (`self.cfg` IS the overlay here).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn decide_on_card(
        &self,
        view: &MindView,
        mem: &mut StockMemory,
        def: &ArenaDefinition,
        hold: Option<&HoldObjective>,
        t_ms: i64,
        lands_ms: i64,
        card: &StockCard,
        slots: usize,
    ) -> Vec<IntentKind> {
        let cfg = &self.cfg;
        let here = view.cell;
        // 17b cut M3.1: the slots this think is held to, its row's.
        let mut plan = Plan {
            slots,
            ..Plan::default()
        };

        // ---- what the motor and the rounds said since the last think ----
        //
        // The reflex learns its `no line` edge from the impact
        // (`Arena::note_no_line`) and its arrival and its `unreachable` edge
        // from the drive it runs itself; a mind reads all three off its notice
        // buffer (D-UM-8): a `blocked` or `cover` miss of its own round, and
        // the motor's `drive_arrived` / `drive_no_route` for the goal it
        // handed over.
        let drive = Drive {
            reached: view.motor.as_ref().and_then(|motor| {
                (motor.outcome == Some(DriveOutcome::Arrived)).then_some(motor.requested_goal)
            }),
        };
        let no_route = view.motor.as_ref().is_some_and(|motor| {
            motor.outcome == Some(DriveOutcome::NoRoute) && Some(motor.requested_goal) == mem.goal
        });
        for notice in &view.notices {
            if let FrameNotice::Miss {
                reason,
                launch_cell,
                ..
            } = notice
                && launch_cell.is_some()
                && *launch_cell == view.cell
                && (reason.as_str() == BLOCKED || reason.as_str() == COVER)
            {
                mem.no_line = true;
            }
        }
        // What the motor holds now, as far as the mind knows: the goal it was
        // handed — which it keeps past an arrival, as the reflex's row does —
        // and nothing once it said no route reached it.
        let mut held = if no_route { None } else { mem.goal };
        // `motor.goal`, as `decide` writes it through the think.
        let mut goal = held;

        // ---- the `Disabled` edge, before anything else ----
        if view.wreck || (view.immobile && view.weapons.is_empty()) {
            *mem = StockMemory {
                state: MachineState::Disabled,
                ..StockMemory::default()
            };
            return Vec::new();
        }

        // ---- the `unreachable` edge's decision half, a think late ----
        //
        // **Departure (i).** The reflex's motor fails a search and falls back
        // to a waypoint inside the tick; a mind's goal is dropped with a
        // `drive_no_route` notice (`Fallback::Notice`), and this is the same
        // edge read off it, in the reflex's order: the order's own ends, the
        // search's and the hunt's give-ups, the Hold retry, and the fallback
        // waypoint as the next goal.
        if no_route && let Some(failed) = mem.goal {
            // 17b cut M1b.5: the view's order unless this hull ended it.
            let live = obeyed(view, mem);
            // 16b cut E.2 (`B-41`): the order's own `Advance` cell.
            if live.is_some_and(|order| order.objective == MindObjective::Advance { to: failed }) {
                mem.ignore(live);
            }
            // 16b cut Z.2's review: a supporter's station, in `Patrol`.
            if live.is_some_and(|order| matches!(order.objective, MindObjective::Support { .. }))
                && mem.state == MachineState::Patrol
            {
                mem.ignore(live);
            }
            // 16b cut E.4: the search's plot and the hunt's cell.
            if mem.last_plot.is_some_and(|(cell, _)| cell == failed) {
                mem.last_plot = None;
            }
            if mem.hunt == Some(failed) {
                mem.hunt = None;
                mem.unheld_since_ms = Some(t_ms);
            }
            // 16b cut O.1: a Hold spot no route reaches, given up for a while.
            if hold.is_some_and(|hold| hold.spot == failed) {
                mem.hold_retry_ms =
                    Some(t_ms.saturating_add(i64::from(cfg.hunt_after_ms.max(cfg.repath_ms))));
            }
            goal = match here {
                Some(here) => {
                    let (waypoint, from_waypoint, fallback) = fallback_waypoint(
                        def,
                        here,
                        Some(failed),
                        mem.waypoint,
                        mem.from_waypoint,
                        view.unit,
                    );
                    mem.waypoint = waypoint;
                    mem.from_waypoint = from_waypoint;
                    fallback
                }
                None => None,
            };
        }

        // ---- the standing order (16b cut N.2) ----
        //
        // The expiry is the ARENA's (17b cut M1a.5 clears the order at the
        // landing that reaches its deadline and journals `order_expired`): the
        // mind sees `order: None` from the next think, with no squad filter —
        // the order was re-keyed to this hull when it was written (D-UM-15).
        // A mind cannot end an order itself, so the three early ends below
        // write the ended order's `expires_ms` to `ignore_until_ms` and it
        // stops obeying THAT order (the divergence `order_end`); the key is
        // spent when the view carries no order or an order with another
        // deadline — a newer order, obeyed at once (17b cut M1b.5).
        if view.order.as_ref().is_none_or(|order| !mem.ignores(order)) {
            mem.ignore_until_ms = None;
            mem.ignore_identity = None;
        }
        let health = health_permille(view);
        let dry = is_dry(view);
        // **The expiry's restoration** (`ReflexUnit::drop_to_doctrine`), read
        // off the view: a `Withdraw` that no live order holds, that the memory
        // says an order put it in (`ordered`, 17b cut M4.3), and that neither
        // of its own edges would have taken — healthy and not dry — is one
        // whose order is over. It goes back to `Patrol` with the target and
        // the goal dropped, as the reflex's expiry does; a hurt one keeps
        // going home, and so does a released seat's seeded `Withdraw`, which
        // no order put there (it rallies on Q.7's clock).
        if view.order.is_none()
            && mem.state == MachineState::Withdraw
            && mem.ordered
            && !dry
            && health >= cfg.withdraw_permille
        {
            mem.state = MachineState::Patrol;
            mem.target = None;
            mem.ordered = false;
            goal = None;
        }
        let mut order: Option<&FrameOrder> = obeyed(view, mem);
        let ends = match order.map(|order| order.objective) {
            // 16b cut AA.1 (`B-76`): an `Advance` a hull that cannot close can
            // never carry out. `immobile` is motion disabled, or `cannot_close`
            // once the power is online (17b cut M4.6b: a hull authored with no speed)
            // (the owner's STOP B ruling).
            Some(MindObjective::Advance { .. }) if view.immobile => true,
            // 16b cut O.1: an `Advance` into the Hold square, arrived.
            Some(MindObjective::Advance { to })
                if hold.is_some_and(|hold| {
                    advance_reached_the_hold(hold, to, here, view.motor.as_ref())
                }) =>
            {
                true
            }
            // 16b cut Z.2: a `Support` whose charge this hull's own row reads
            // dead.
            Some(MindObjective::Support { of: Some(of) }) => {
                contact(view, of).is_some_and(|row| row.reads_dead == Some(true))
            }
            _ => false,
        };
        if ends {
            mem.ignore(order);
            order = None;
        }
        let objective = order.map(|order| order.objective);

        // ---- the inert rule (16b cut O.3) ----
        //
        // A hull that cannot move and carries no weapon is inert. Over the view
        // that is `immobile` and no weapon rows — which is the `Disabled` edge's
        // own reading above, so it never reaches here. Since 17b cut M4.6b (the
        // owner's STOP B ruling, "power up, then park") a hull AUTHORED with no
        // speed (the range's target) reads `immobile` once its power is
        // `Online`: cold or starting it reads mobile and takes the power arm
        // below, and its first think online parks it on the `Disabled` edge —
        // the reflex's "start up, then park".

        // ---- power ----
        match view.power {
            Power::Off => {
                // 16b cut D.3 (`B-34`): a cold tank starts up, wherever it is
                // going; a `Withdraw` stays one.
                if mem.state != MachineState::Withdraw {
                    mem.state = MachineState::Startup;
                }
                mem.goal = held;
                // 17b cut M1b.2: a cold hull's slots are disarmed, after the
                // `Startup` — 17b cut M1b.3: where its view shows one armed.
                let mut intents = vec![IntentKind::Startup];
                intents.extend(arming_for(view, card, None, slots));
                return intents;
            }
            Power::Starting => {
                if mem.state != MachineState::Withdraw {
                    mem.state = MachineState::Startup;
                }
                mem.goal = held;
                return arming_for(view, card, None, slots);
            }
            Power::Online => {
                if matches!(mem.state, MachineState::Startup | MachineState::Disabled) {
                    mem.state = MachineState::Patrol;
                    goal = None;
                }
            }
        }

        // ---- the `lost` edge ----
        //
        // 16b cut E.2 (`B-45`): a target the view no longer holds, or one its
        // row reads dead.
        if let Some(target) = mem.target
            && contact(view, target).is_none_or(|row| row.reads_dead == Some(true))
        {
            mem.target = None;
            if matches!(mem.state, MachineState::Approach | MachineState::Engage) {
                mem.state = MachineState::Patrol;
                goal = None;
            }
        }

        // ---- what the order does to the machine (16b cut N.2, Z.2) ----
        match objective {
            Some(MindObjective::Withdraw) => {
                if mem.state != MachineState::Withdraw {
                    mem.state = MachineState::Withdraw;
                    mem.target = None;
                    goal = None;
                    mem.last_plot = None;
                    // 17b cut M4.3: an order's `Withdraw`, restored at its end.
                    mem.ordered = true;
                }
            }
            Some(
                MindObjective::Hold | MindObjective::Advance { .. } | MindObjective::Screen { .. },
            ) => {
                if matches!(mem.state, MachineState::Approach | MachineState::Engage) {
                    mem.state = MachineState::Patrol;
                    goal = None;
                }
                mem.target = None;
            }
            Some(MindObjective::Engage { .. } | MindObjective::Support { .. }) | None => {}
        }

        // ---- the `Withdraw` edges (16b cut Q.7) ----
        if mem.state != MachineState::Withdraw
            && (dry || (!mem.rallied && health < cfg.withdraw_permille))
        {
            mem.state = MachineState::Withdraw;
            mem.target = None;
            goal = None;
            mem.last_plot = None;
            // 17b cut M4.3: its own edge's `Withdraw`, never restored early.
            mem.ordered = false;
        }

        // ---- the hostile edges, and the leash's reading (16b cut Z.2) ----
        //
        // A supporter reads its own row of its charge: `Some(None)` is a
        // supporter that does not hold it. (A self-charge never reaches here:
        // since 17b cut L.2 `Arena::rekey_order` re-keys it to `Engage`.)
        let leash: Option<Option<&ViewContact>> = match objective {
            Some(MindObjective::Support { of })
                if matches!(
                    mem.state,
                    MachineState::Patrol | MachineState::Approach | MachineState::Engage
                ) =>
            {
                Some(of.and_then(|of| contact(view, of)))
            }
            _ => None,
        };
        let cell_cm = u64::from(def.terrain.cell_cm());
        let leash_cm = u64::from(SUPPORT_LEASH_CELLS).saturating_mul(cell_cm);
        let station_cm = u64::from(SUPPORT_STATION_CELLS).saturating_mul(cell_cm);
        let off_leash = match leash {
            Some(None) => true,
            Some(Some(plot)) => row_range_cm(plot) > leash_cm,
            None => false,
        };
        let may_pursue = !off_leash
            && !matches!(
                objective,
                Some(
                    MindObjective::Hold
                        | MindObjective::Advance { .. }
                        | MindObjective::Screen { .. }
                )
            );
        if may_pursue
            && matches!(
                mem.state,
                MachineState::Patrol | MachineState::Approach | MachineState::Engage
            )
        {
            match best_hostile(view, &Weights::of(cfg), health, hint(order), |_| true) {
                Some(found) => {
                    if mem.target != Some(found) {
                        mem.target = Some(found);
                        goal = None;
                    }
                    if mem.state == MachineState::Patrol {
                        mem.state = MachineState::Approach;
                    }
                }
                None => {
                    mem.target = None;
                    if matches!(mem.state, MachineState::Approach | MachineState::Engage) {
                        mem.state = MachineState::Patrol;
                        goal = None;
                    }
                }
            }
        }

        // ---- the `no line` edge (W's `blocked`/`cover` miss) ----
        if mem.no_line {
            mem.no_line = false;
            goal = None;
            if mem.state == MachineState::Engage {
                mem.state = MachineState::Approach;
            }
        }

        // ---- the state's own goal and orders ----
        let ranges = view_ranges(view, cfg.engage_band_permille, order);
        let mut stood_cell: Option<(u16, u16)> = None;

        // ---- the leash (16b cut Z.2) ----
        let mut leashed = false;
        let stands = match leash {
            Some(None) => true,
            Some(Some(_)) => off_leash && view.immobile,
            None => false,
        };
        match leash {
            // (a) blind to the charge, or past the leash and unable to close:
            // hold where it stands and fight from there.
            _ if stands => {
                leashed = true;
                if matches!(mem.state, MachineState::Approach | MachineState::Engage) {
                    mem.state = MachineState::Patrol;
                }
                mem.target = None;
                goal = here;
                plan.stop = true;
                mem.last_plot = None;
                mem.hunt = None;
                mem.unheld_since_ms = Some(t_ms);
                if let Some(row) = self.stand(view, def, health, &ranges, None, order) {
                    stood_cell = row.cell;
                    plan.heading_mdeg = Some(row.bearing_deg.saturating_mul(1_000));
                    engage_orders(
                        view,
                        row,
                        order,
                        &ranges,
                        ranges.release_cm,
                        lands_ms,
                        &mut plan,
                    );
                }
            }
            // (b) past the leash: back toward the station off its own plot of
            // the charge, firing nothing until it is back inside.
            Some(Some(plot)) if off_leash => {
                leashed = true;
                mem.state = MachineState::Patrol;
                mem.target = None;
                mem.last_plot = None;
                mem.hunt = None;
                mem.unheld_since_ms = Some(t_ms);
                let station = stand_off_of(def, view, plot, station_cm).or(here);
                if goal != station {
                    goal = station;
                }
            }
            // (c) inside the leash: the state's own arm.
            Some(Some(_) | None) | None => {}
        }
        let station_on = if leashed { None } else { leash.flatten() };
        if !leashed {
            match mem.state {
                // 17b cut M4.6: `Unknown` is never a doctrine memory's state
                // (`state_of` decodes over `MachineState::DOCTRINE`); the arm
                // keeps the match exhaustive and does nothing, as `Startup`'s.
                MachineState::Startup | MachineState::Disabled | MachineState::Unknown => {}
                MachineState::Patrol => {
                    let movement = matches!(
                        objective,
                        Some(
                            MindObjective::Advance { .. }
                                | MindObjective::Hold
                                | MindObjective::Screen { .. }
                        )
                    ) || station_on.is_some();
                    if movement {
                        mem.last_plot = None;
                        mem.hunt = None;
                        mem.unheld_since_ms = Some(t_ms);
                    }
                    match objective {
                        Some(MindObjective::Advance { to }) => {
                            if goal != Some(to) {
                                goal = Some(to);
                            }
                        }
                        Some(MindObjective::Hold) => {
                            goal = here;
                            plan.stop = true;
                        }
                        // The screen: parked, faced onto the order's bearing,
                        // and the stand over the arc — off the view's whole
                        // degrees (the divergence `deg_round`).
                        Some(MindObjective::Screen { facing_mdeg }) => {
                            goal = here;
                            plan.stop = true;
                            plan.heading_mdeg = Some(facing_mdeg % 360_000);
                            if let Some(row) = self.stand(
                                view,
                                def,
                                health,
                                &ranges,
                                Some((facing_mdeg, SCREEN_HALF_ARC_MDEG)),
                                order,
                            ) {
                                stood_cell = row.cell;
                                plan.heading_mdeg = Some(row.bearing_deg.saturating_mul(1_000));
                                engage_orders(
                                    view,
                                    row,
                                    order,
                                    &ranges,
                                    ranges.release_cm,
                                    lands_ms,
                                    &mut plan,
                                );
                            }
                        }
                        // A supporter idle inside its leash keeps station.
                        Some(MindObjective::Support { .. }) if station_on.is_some() => {
                            if let Some(plot) = station_on {
                                if row_range_cm(plot) <= station_cm {
                                    goal = here;
                                    plan.stop = true;
                                } else {
                                    let station = stand_off_of(def, view, plot, station_cm);
                                    let at_station =
                                        station.is_some_and(|cell| drive.arrived(cell));
                                    let next = if at_station {
                                        plot.cell.or(station)
                                    } else {
                                        station
                                    }
                                    .or(here);
                                    if goal != next {
                                        goal = next;
                                    }
                                }
                            }
                        }
                        // The search, then the Hold spot or the hunt, then the
                        // waypoint walk (16b cuts E.4, O.1).
                        _ => {
                            if !self.search_goal(t_ms, mem, &mut goal, drive) {
                                if let Some(hold) = hold {
                                    if !hold_goal(view, hold, t_ms, mem, &mut goal, here, &mut plan)
                                    {
                                        walk(view, def, mem, &mut goal, here, drive);
                                    }
                                } else if !self
                                    .hunt_goal(view, def, t_ms, mem, &mut goal, here, drive)
                                {
                                    walk(view, def, mem, &mut goal, here, drive);
                                }
                            }
                        }
                    }
                }
                // Every range, goal and heading off the hull's own row of its
                // target — the plot, at the view's resolution.
                MachineState::Approach => {
                    if let Some(target) = mem.target {
                        let row = contact(view, target);
                        let range = row.map_or(u64::MAX, row_range_cm);
                        let inside = hold.is_some_and(|hold| {
                            here.is_some_and(|cell| hold.square_contains(cell))
                        });
                        let close_edge = if view.immobile || inside {
                            ranges.release_cm
                        } else {
                            ranges.engage_cm
                        };
                        if ranges.limit_cm != 0 && range <= close_edge.max(1) {
                            if line_between(def, here, row.and_then(|row| row.cell)) {
                                mem.state = MachineState::Engage;
                                goal = None;
                                if let Some(row) = row {
                                    engage_orders(
                                        view,
                                        row,
                                        order,
                                        &ranges,
                                        ranges.release_cm,
                                        lands_ms,
                                        &mut plan,
                                    );
                                }
                            } else if inside {
                                goal = here;
                            } else {
                                goal = row.and_then(|row| row.cell).or(here);
                            }
                        } else if inside {
                            goal = here;
                        } else {
                            let want = stand_off_want(ranges.engage_cm, cell_cm.max(1), range);
                            goal = row
                                .and_then(|row| stand_off_of(def, view, row, want))
                                .or(here);
                        }
                    }
                }
                MachineState::Engage => {
                    if let Some(target) = mem.target {
                        let row = contact(view, target);
                        let range = row.map_or(u64::MAX, row_range_cm);
                        if ranges.limit_cm == 0 || range > ranges.release_cm {
                            mem.state = MachineState::Approach;
                            let want = stand_off_want(ranges.engage_cm, cell_cm.max(1), range);
                            goal = row
                                .and_then(|row| stand_off_of(def, view, row, want))
                                .or(here);
                        } else {
                            // Holding the band: stop, turn onto the plot, shoot.
                            goal = here;
                            plan.stop = true;
                            plan.heading_mdeg =
                                row.map(|row| row.bearing_deg.saturating_mul(1_000));
                            if let Some(row) = row {
                                engage_orders(
                                    view,
                                    row,
                                    order,
                                    &ranges,
                                    ranges.release_cm,
                                    lands_ms,
                                    &mut plan,
                                );
                            }
                        }
                    }
                }
                // 16b cut Q.7: home, the stand, the dwell, the rally.
                MachineState::Withdraw => {
                    if goal.is_none() {
                        goal = here
                            .and_then(|cell| entry_cell(def, view.party, CostRow::Tracked, cell))
                            .or(here);
                    }
                    let arrived = view.immobile
                        || match (here, goal) {
                            (Some(_), Some(cell)) => drive.arrived(cell),
                            (_, None) => true,
                            _ => false,
                        };
                    let cornered = self.withdraw_stand(view, def, health, arrived, &ranges, order);
                    if let Some(row) = cornered {
                        stood_cell = row.cell;
                        mem.stand_until_ms =
                            Some(t_ms.saturating_add(
                                STAND_TICKS.saturating_mul(i64::from(cfg.decide_ms)),
                            ));
                        mem.quiet_since_ms = None;
                        plan.stop = true;
                        plan.heading_mdeg = Some(row.bearing_deg.saturating_mul(1_000));
                        // 17b cut M1b.2: the ceiling `withdraw_stand` admitted
                        // the row by — `release_cm` home, `stand_cm` en route.
                        let ceiling = if arrived {
                            ranges.release_cm
                        } else {
                            ranges.stand_cm
                        };
                        engage_orders(view, row, order, &ranges, ceiling, lands_ms, &mut plan);
                    } else if mem.stand_until_ms.is_some_and(|until| t_ms < until) {
                        mem.quiet_since_ms = None;
                        plan.stop = true;
                    } else {
                        mem.stand_until_ms = None;
                        let ordered_home = objective == Some(MindObjective::Withdraw);
                        if holds_a_hostile(view) || !arrived || dry || ordered_home {
                            mem.quiet_since_ms = None;
                        } else {
                            let since = *mem.quiet_since_ms.get_or_insert(t_ms);
                            if t_ms.saturating_sub(since) >= i64::from(cfg.rally_ms) {
                                mem.state = MachineState::Patrol;
                                mem.rallied = true;
                                goal = None;
                                mem.quiet_since_ms = None;
                            }
                        }
                    }
                }
            }
        }

        // ---- what this think saw, and how long it has seen nothing (16b cut
        //      E.4) ----
        if let Some(cell) = stood_cell {
            mem.last_plot = Some((cell, t_ms));
        } else if matches!(mem.state, MachineState::Approach | MachineState::Engage)
            && let Some(cell) = mem
                .target
                .and_then(|target| contact(view, target))
                .and_then(|row| row.cell)
        {
            mem.last_plot = Some((cell, t_ms));
        }
        if mem.state != MachineState::Patrol {
            mem.hunt = None;
            mem.unheld_since_ms = Some(t_ms);
        }

        // ---- the emission (the module head's order and its two rules) ----
        // (`Startup` is the power arm's, which answers it with a disarm
        // where a slot is armed.)
        // 17b cut M1b.2: the slots first, after `Startup` and before `Goal`.
        let mut intents: Vec<IntentKind> = arming_for(view, card, plan.arm_ceiling_cm, plan.slots);
        // The helm half. A hull whose motion is disabled is not handed one:
        // every goal and heading would be refused `MotionDisabled`, as the
        // reflex's are, and the motor drops a goal it cannot drive anyway.
        // Nor, since 17b cut M4.6b (the owner's STOP B ruling), is a hull
        // authored with no speed once its power is `Online`: it could turn in
        // place, but it is handed no heading, so an armed one holds its
        // opening heading where the reflex turned it onto the plot (no
        // shipped armed hull has zero speed; doc 17 §24).
        if view.immobile {
            held = None;
        } else if plan.stop {
            intents.push(IntentKind::Goal(MotorGoal::Hold));
            held = None;
            if let Some(mdeg) = plan.heading_mdeg {
                intents.push(IntentKind::Heading { mdeg });
            }
        } else if let Some((x, y)) = goal {
            if held != goal {
                intents.push(IntentKind::Goal(MotorGoal::Cell { x, y }));
            }
            held = goal;
        } else if held.is_some() {
            intents.push(IntentKind::Goal(MotorGoal::Hold));
            held = None;
        }
        mem.goal = held;
        if let Some(contact) = plan.lock {
            intents.push(IntentKind::Lock { contact });
        }
        if let Some((weapon, slot)) = plan.fire {
            intents.push(IntentKind::Fire {
                weapon,
                slot: Some(slot),
            });
        }
        intents
    }

    /// **`Patrol`'s first goal: where it last saw something** (the reflex's
    /// `search_goal`): the last plot, while it is no older than `search_ms`
    /// (inclusively) and not arrived at.
    fn search_goal(
        &self,
        t_ms: i64,
        mem: &mut StockMemory,
        goal: &mut Option<(u16, u16)>,
        drive: Drive,
    ) -> bool {
        let Some((cell, at_ms)) = mem.last_plot else {
            return false;
        };
        if self.cfg.search_ms == 0 || t_ms.saturating_sub(at_ms) > i64::from(self.cfg.search_ms) {
            mem.last_plot = None;
            return false;
        }
        if drive.arrived(cell) {
            mem.last_plot = None;
            return false;
        }
        if *goal != Some(cell) {
            *goal = Some(cell);
        }
        true
    }

    /// **`Patrol`'s second goal: the hunt** (the reflex's `hunt_goal`): the far
    /// zone once `hunt_after_ms` has passed with nothing hostile held. The
    /// cell is [`far_entry_cell`] with no `taken` cells — the motor fans the
    /// landing instead (departure (d)).
    #[allow(clippy::too_many_arguments)]
    fn hunt_goal(
        &self,
        view: &MindView,
        def: &ArenaDefinition,
        t_ms: i64,
        mem: &mut StockMemory,
        goal: &mut Option<(u16, u16)>,
        here: Option<(u16, u16)>,
        drive: Drive,
    ) -> bool {
        if holds_a_hostile(view) {
            mem.unheld_since_ms = None;
            mem.hunt = None;
            return false;
        }
        if let Some(cell) = mem.hunt {
            if drive.arrived(cell) {
                // It went and looked: the walk resumes from the nearest node.
                mem.hunt = None;
                mem.unheld_since_ms = Some(t_ms);
                mem.waypoint = None;
                mem.from_waypoint = None;
                return false;
            }
            if *goal != Some(cell) {
                *goal = Some(cell);
            }
            return true;
        }
        let since = *mem.unheld_since_ms.get_or_insert(t_ms);
        if self.cfg.hunt_after_ms == 0
            || t_ms.saturating_sub(since) < i64::from(self.cfg.hunt_after_ms)
        {
            return false;
        }
        let Some(cell) =
            here.and_then(|cell| far_entry_cell(def, view.party, CostRow::Tracked, cell, &[]))
        else {
            mem.unheld_since_ms = Some(t_ms);
            return false;
        };
        mem.hunt = Some(cell);
        if *goal != Some(cell) {
            *goal = Some(cell);
        }
        true
    }

    /// **Whether a withdrawing hull is cornered, and by which row** (the
    /// reflex's `withdraw_stand`): never with nothing to fire or under
    /// `hold_fire`; the best hostile, inside `release_cm` once home and
    /// `stand_cm` (and `stand_min_permille` of health) en route, with a line.
    fn withdraw_stand<'v>(
        &self,
        view: &'v MindView,
        def: &ArenaDefinition,
        health: u32,
        arrived: bool,
        ranges: &Ranges,
        order: Option<&FrameOrder>,
    ) -> Option<&'v ViewContact> {
        if is_dry(view) || order.is_some_and(|order| order.hold_fire) {
            return None;
        }
        let row = contact(
            view,
            best_hostile(view, &Weights::of(&self.cfg), health, None, |_| true)?,
        )?;
        let ceiling = if arrived {
            ranges.release_cm
        } else {
            ranges.stand_cm
        };
        if row_range_cm(row) > ceiling.max(1) {
            return None;
        }
        if !arrived && health < self.cfg.stand_min_permille {
            return None;
        }
        if !line_between(def, view.cell, row.cell) {
            return None;
        }
        Some(row)
    }

    /// **A stand** (the reflex's `stand`): the best hostile inside
    /// `release_cm` — and, for a screen, inside `half` of `facing` off the
    /// row's whole-degree bearing — with a line; `None` with nothing to fire.
    fn stand<'v>(
        &self,
        view: &'v MindView,
        def: &ArenaDefinition,
        health: u32,
        ranges: &Ranges,
        sector: Option<(u32, u32)>,
        order: Option<&FrameOrder>,
    ) -> Option<&'v ViewContact> {
        if is_dry(view) {
            return None;
        }
        let ceiling = ranges.release_cm.max(1);
        let id = best_hostile(view, &Weights::of(&self.cfg), health, hint(order), |row| {
            row_range_cm(row) <= ceiling
                && sector.is_none_or(|(facing, half)| {
                    arc_between(row.bearing_deg.saturating_mul(1_000), facing) <= half
                })
        })?;
        let row = contact(view, id)?;
        if !line_between(def, view.cell, row.cell) {
            return None;
        }
        Some(row)
    }
}

/// **`Patrol`'s Hold goal: the key spot** (the reflex's `hold_goal`): declined
/// while the retry clock runs; parked once arrived ([`hold_arrived`], with
/// `immobile` as `cannot_close`), else the spot.
#[allow(clippy::too_many_arguments)]
fn hold_goal(
    view: &MindView,
    hold: &HoldObjective,
    t_ms: i64,
    mem: &mut StockMemory,
    goal: &mut Option<(u16, u16)>,
    here: Option<(u16, u16)>,
    plan: &mut Plan,
) -> bool {
    if mem.hold_retry_ms.is_some_and(|until| t_ms < until) {
        return false;
    }
    mem.hold_retry_ms = None;
    if hold_arrived(hold, view.immobile, here) {
        *goal = here;
        plan.stop = true;
    } else if *goal != Some(hold.spot) {
        *goal = Some(hold.spot);
    }
    mem.hunt = None;
    mem.unheld_since_ms = Some(t_ms);
    true
}

/// **The waypoint walk** — the lifted [`patrol_step`], keyed on the view's own
/// `unit` (departure (e)), with the motor's arrival word for a fanned node.
fn walk(
    view: &MindView,
    def: &ArenaDefinition,
    mem: &mut StockMemory,
    goal: &mut Option<(u16, u16)>,
    here: Option<(u16, u16)>,
    drive: Drive,
) {
    let step = patrol_step(
        def,
        view.unit,
        here,
        drive.reached.is_some() && drive.reached == *goal,
        Walk {
            waypoint: mem.waypoint,
            from_waypoint: mem.from_waypoint,
            goal: *goal,
        },
    );
    mem.waypoint = step.walk.waypoint;
    mem.from_waypoint = step.walk.from_waypoint;
    *goal = step.walk.goal;
}

/// **`Engage`'s two discrete actions** (the reflex's `engage_orders`): the
/// lock, then the first weapon in order that is ready **at the landing**,
/// loaded, reaches the row's held range and is not held back by heat — inside
/// `release_cm`, with power, and not under `hold_fire`.
///
/// **Ready at the landing, not at the view** (the reflex's 16b cut M.2 rule,
/// ported): `engage_orders` reads readiness at the millisecond
/// `Arena::resolve_fire` is then handed, and a mind's `Fire` is resolved at
/// `lands_ms` ([`ThinkJob::lands_ms`]), one substep after the think. The
/// view's weapon row is read at its own clock (`view.t_ms`, the end of the
/// substep the think fell in), so the test is its `ready_in_ms` against the
/// milliseconds still to run to the landing. Read off `ready` alone the mind
/// declined every shot whose cycle ended inside that substep and fired it a
/// whole think later — on the stock tank's one-second coaxial, every other
/// think: half the reflex's rate of fire, where doc 17 §17 budgets the
/// landing at most one substep of latency. The countdown is on the keyframe
/// a pilot reads, and the landing is the mind's own schedule (D-UM-21), so
/// nothing past the observer rule is read; the shot is decided at the think
/// and resolved at the landing, never held for a weapon (16c Q45).
///
/// **D-UM-48: on a hull with no weapon rows the fire half is absent** — not
/// idled. No caller reaches here for one (the utility score answers it no
/// target), and the guard says so where the reader looks.
///
/// **17b cut M1b.2 — the slots and the citation** (D-UM-16, D-UM-17,
/// D-UM-18). Past the power, `hold_fire` and dry gates the think records the
/// ceiling that admitted `row` (`ceiling_cm`: `release_cm`, or a withdraw
/// stand's `stand_cm` en route) on the plan, and the emission arms the hull's
/// first `slots` weapons on it ([`arming_for`]; Class I's two on every
/// shipped chassis): from then on the
/// **slot** owns the shot — the arena fires it at the weapon's own
/// `WeaponReady`, at the passes that refresh the lock and at the landing.
/// A `Fire` is planned here ONLY when the think also plans a `Lock` (a fresh
/// lock no slot has been evaluated against): the landing applies its kinds in
/// emission order — `Condition`, `Lock`, `Fire` — so that shot is resolved
/// right after the new lock lands and before `Arena::mind_land`'s one
/// `Landing` evaluation, which then finds the gun cycling.
///
/// **17b cut M1b.3 — the citation off the view** (D-UM-4: a mind reads its
/// own armed slots exactly as the pilot does). It cites the slot NUMBER the
/// view's row shows, `weapon.armed.slot` — the arena's own stable number,
/// which a `RemoveWeapons` consequence leaves on the surviving guns (the
/// arena frees only the lost gun's slot, `Arena::apply_consequence`), so a
/// hull that lost its S1 gun cites its S2 gun as S2 (M1b.2's residual (a),
/// removed; pinned by
/// `m1b2_residual_a_removed_weapon_parts_i_plus_one_from_the_arenas_number`).
/// A weapon with no `armed` row yet — armed by this very think, whose arming
/// lands ahead of the `Fire` — is cited `i + 1`, the number the arena's `arm`
/// gives it on a hull T1 has armed in component order from its first think.
/// The citation is picked by the rule above (ready at the
/// landing, loaded, in reach, `!would_overheat`: T0's own rule for the shot
/// it plans), among the armed weapons only (a weapon past the slots has no
/// slot to cite: the divergence `slot_starved`).
fn engage_orders(
    view: &MindView,
    row: &ViewContact,
    order: Option<&FrameOrder>,
    ranges: &Ranges,
    ceiling_cm: u64,
    lands_ms: i64,
    plan: &mut Plan,
) {
    if view.weapons.is_empty() {
        return;
    }
    let fresh_lock = view.lock != Some(row.id);
    if fresh_lock {
        plan.lock = Some(row.id);
    }
    if view.power != Power::Online {
        return;
    }
    if order.is_some_and(|order| order.hold_fire) {
        return;
    }
    if is_dry(view) {
        return;
    }
    // 17b cut M1b.2: reached with a row, the gates passed — the slots arm.
    plan.arm_ceiling_cm = Some(ceiling_cm);
    let range = row_range_cm(row);
    if range > ranges.release_cm {
        return;
    }
    if !fresh_lock {
        // The lock is held: the armed slot owns the shot.
        return;
    }
    // The milliseconds from the view's clock to the landing: a weapon whose
    // countdown ends inside them is ready when the shot is resolved.
    let to_landing = lands_ms.saturating_sub(view.t_ms).max(0);
    plan.fire = view
        .weapons
        .iter()
        // 17b cut M3.1: among the weapons the hull's slots arm.
        .take(plan.slots)
        .enumerate()
        .find(|(_, weapon)| {
            weapon.ready_in_ms <= to_landing
                && weapon.rounds > 0
                && weapon
                    .bands_cm
                    .last()
                    .is_some_and(|reach| u64::from(*reach) >= range)
                && !would_overheat(view, weapon)
        })
        .and_then(|(at, weapon)| {
            // 17b cut M1b.3: the number the view shows, else `i + 1` for a
            // weapon this think arms.
            let n = match &weapon.armed {
                Some(armed) => armed.slot,
                None => u8::try_from(at.saturating_add(1)).ok()?,
            };
            Some((weapon.id.clone(), n))
        });
}

/// **T1's slots for this think** (17b cut M1b.2; D-UM-16, D-UM-18, D-UM-43):
/// what the stock mind arms or disarms, emitted after `Startup` and before
/// `Goal`.
///
/// * `reach_ceiling_cm` `Some(ceiling)` — `engage_orders` was reached with a
///   row and its gates passed: one `Condition(Arm)` per `view.weapons[i]` for
///   `i < slots`, in component order, **whatever the heat**
///   (the resolver's `Overheated` holds a hot slot's shot silently and the slot
///   stays armed; a heat skip here would put T1's numbering out of step with
///   the arena's and build in a think of latency for nothing — 17b's
///   adversarial F6), each `FireCondition { weapon, target: BestHostile,
///   min_permille: card.min_odds_permille, aspects: card.aspect.mask(),
///   min_rounds_left: card.reserve_pct × rounds_max / 100, min_range_m: 0,
///   max_range_m: min(ceiling, the weapon's reach) / 100 }`;
/// * `None` — one `Condition(Disarm { weapon: None })` when some row is
///   armed, else nothing;
/// * an unarmed hull (no weapon rows) — nothing (D-UM-48).
///
/// **Emitted on change** (17b cut M1b.3; D-UM-4's parity rule — a mind reads
/// its own armed slots off its view exactly as a pilot reads them off
/// `WEAPONSTATUS`): an `Arm` only for a row among the first `slots` whose
/// `armed` is `None` or holds other terms than the ones wanted (the slot NUMBER is the arena's and is not compared), and
/// the `Disarm` only when some row is armed and the think wants none. A think
/// whose view already shows what it wants emits no `Condition` at all. (17b
/// cut M1b.2 emitted every think, idempotently, because the view could not
/// show the arming.) The card is read here for its fire discipline (on Line:
/// 100 per mille, every aspect, no reserve); its other mapped terms reach
/// `decide` through the overlay (17b cut M1b.5).
///
/// **17b cut M3.1** (D-UM-35): `slots` is the think's row's
/// ([`crate::arena::mind::ThinkJob::profile`]) — Class I's two on every
/// shipped chassis, and the Rhai port's `slots()` reads the same figure.
#[must_use]
pub fn arming_for(
    view: &MindView,
    card: &StockCard,
    reach_ceiling_cm: Option<u64>,
    slots: usize,
) -> Vec<IntentKind> {
    if view.weapons.is_empty() {
        return Vec::new();
    }
    let Some(ceiling_cm) = reach_ceiling_cm else {
        // 17b cut M1b.3: nothing to clear is nothing said.
        if view.weapons.iter().any(|weapon| weapon.armed.is_some()) {
            return vec![IntentKind::Condition(Arming::Disarm { weapon: None })];
        }
        return Vec::new();
    };
    view.weapons
        .iter()
        .take(slots)
        .filter_map(|weapon| {
            let reach_cm = weapon.bands_cm.last().map_or(0, |reach| u64::from(*reach));
            let keeps =
                u64::from(card.reserve_pct).saturating_mul(u64::from(weapon.rounds_max)) / 100;
            let wanted = FireCondition {
                weapon: weapon.id.clone(),
                target: SlotTarget::BestHostile,
                min_permille: card.min_odds_permille,
                aspects: card.aspect.mask(),
                min_rounds_left: u32::try_from(keeps).unwrap_or(u32::MAX),
                min_range_m: 0,
                max_range_m: u32::try_from(ceiling_cm.min(reach_cm) / 100).unwrap_or(u32::MAX),
            };
            // 17b cut M1b.3: the view's row already holds these terms.
            let held = weapon
                .armed
                .as_ref()
                .is_some_and(|armed| *armed == FrameArming::of(armed.slot, &wanted));
            (!held).then_some(IntentKind::Condition(Arming::Arm(wanted)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mush_core::ObjId;

    use super::*;
    use crate::arena::state::Classification;
    /// Class I's slots (17b cut M3.2: the row's figure, read off the class
    /// table — the constant is left to its definition and its row-I pin).
    const CLASS_I_SLOTS: usize = crate::arena::core::ClassTable::STOCK
        .row(crate::arena::core::CoreClass::I)
        .slots;
    use crate::arena::mind::{MIND_INTENTS_MAX, memory_check};
    use crate::arena::state::{
        Arena, ArenaTerrain, Controller, CustodyOut, CustodyUnit, MatchMode, PartyId, SeenPose,
        ShadowContact,
    };
    use crate::arena::test_support::{definition, ground, line_tank, pose, resolution, scout};
    use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    const BLUE2: ObjId = ObjId(11);

    /// The test definition's graph and zones on a 56-cell window of ground
    /// (`mind.rs`' own `wide`, copied): room for a 600 m sight.
    fn field(units: Vec<CustodyUnit>) -> Arena {
        Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::uniform(56, 56, 1_000, ground())
                        .unwrap_or_else(|error| panic!("the field: {error:?}")),
                    ..definition()
                },
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
                host: crate::arena::FieldHost::all_hostile(),
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the field arena: {error:?}"))
    }

    /// A line tank flown by a mind.
    fn minded(id: ObjId, team: PartyId, x: i64, y: i64) -> CustodyUnit {
        let mut unit = line_tank(id, team, pose(x, y));
        unit.controller = Controller::Mind;
        unit.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        unit
    }

    /// `viewer` holds `target` under `id`, classified, plotted where it stands.
    fn hold(arena: &mut Arena, viewer: ObjId, target: ObjId, id: ContactId) {
        let Some(state) = arena.unit(target) else {
            panic!("a fielded target");
        };
        let class = Classification::Classified { party: state.team };
        let plot = SeenPose::of(state);
        arena
            .contacts
            .insert(viewer, id, ShadowContact::sighted(target, class, 0, plot));
    }

    /// The hull's power, written.
    fn online(arena: &mut Arena, unit: ObjId) {
        if let Some(state) = arena.unit_mut(unit) {
            state.power = Power::Online;
        }
    }

    /// **17b cut M1b.2**: what a think that does not reach `engage_orders`
    /// with its gates passed answers for an armed hull's slots.
    fn disarm() -> IntentKind {
        IntentKind::Condition(Arming::Disarm { weapon: None })
    }

    /// The view `unit`'s next think would read, off a clone (the view
    /// settles and drains hashed state).
    fn view_of(arena: &Arena, unit: ObjId) -> MindView {
        arena
            .clone()
            .mind_view(unit)
            .unwrap_or_else(|| panic!("#{} has a view", unit.0))
    }

    /// One think of the stock mind on `view` from `memory`, landing one
    /// substep after `t_ms` as the arena lands it.
    fn think(
        arena: &Arena,
        view: &MindView,
        memory: &mut StockMemory,
        t_ms: i64,
    ) -> Vec<IntentKind> {
        NativeStockMind::with(&arena.arena_cfg.reflex).decide(
            view,
            memory,
            &arena.definition,
            None,
            t_ms,
            t_ms.saturating_add(arena.substep_len_ms()),
            &crate::arena::card::line(&arena.arena_cfg.reflex),
            CLASS_I_SLOTS,
        )
    }

    /// **The emission order and its two rules** (the module head): kinds in
    /// `Startup, Condition, Goal, Heading, Speed, Lock, Fire` order, each at
    /// most once EXCEPT `Condition` (one per armed weapon, 17b cut M1b.2); a
    /// `Goal(Cell)` with no `Heading` or `Speed` beside it; never more than
    /// `4 + CLASS_I_SLOTS` (the module head's maximum), which is under
    /// `MIND_INTENTS_MAX`.
    fn emission_holds(intents: &[IntentKind]) {
        let rank = |kind: &IntentKind| match kind {
            IntentKind::Startup => 0_u8,
            IntentKind::Condition(_) => 1,
            IntentKind::Goal(_) => 2,
            IntentKind::Heading { .. } => 3,
            IntentKind::Speed { .. } => 4,
            IntentKind::Lock { .. } => 5,
            IntentKind::Fire { .. } => 6,
            other => panic!("the stock mind emitted {other:?}"),
        };
        let ranks: Vec<u8> = intents.iter().map(rank).collect();
        assert!(
            ranks.windows(2).all(|pair| match pair {
                [1, 1] => true,
                [a, b] => a < b,
                _ => false,
            }),
            "out of order, or a kind twice: {intents:?}"
        );
        assert!(
            ranks.iter().filter(|rank| **rank == 1).count() <= CLASS_I_SLOTS,
            "{intents:?}"
        );
        if intents
            .iter()
            .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))
        {
            assert!(
                !intents.iter().any(|kind| matches!(
                    kind,
                    IntentKind::Heading { .. } | IntentKind::Speed { .. }
                )),
                "a heading or a speed beside a goal: {intents:?}"
            );
        }
        assert!(intents.len() <= 4 + CLASS_I_SLOTS, "{intents:?}");
        assert!(intents.len() <= MIND_INTENTS_MAX);
    }

    /// **`StockMemory` is fifteen keys and round-trips** (17b cut M1a.6;
    /// D-UM-30): every field set — a target id with its top bit set among
    /// them — writes exactly [`STOCK_MEMORY_KEY_NAMES`], passes
    /// `memory_check`, reads back to itself, and `encode(decode(encode(m)))
    /// == encode(m)`; the default writes the same fifteen keys, its nulls
    /// included, and an empty memory reads as the default.
    #[test]
    fn m1a6_the_stock_memory_is_fifteen_keys_and_round_trips() {
        let full = StockMemory {
            state: MachineState::Withdraw,
            target: Some(ContactId(0xF00D_0000_0000_0001)),
            goal: Some((12, 34)),
            waypoint: Some(3),
            from_waypoint: Some(2),
            last_plot: Some(((7, 8), 61_250)),
            unheld_since_ms: Some(1_000),
            quiet_since_ms: Some(2_000),
            stand_until_ms: Some(3_000),
            hold_retry_ms: Some(4_000),
            rallied: true,
            hunt: Some((250, 128)),
            no_line: true,
            ignore_until_ms: Some(60_000),
            ignore_identity: None,
            ordered: true,
        };
        let mut keys: Vec<&str> = STOCK_MEMORY_KEY_NAMES.to_vec();
        keys.sort_unstable();
        for stock in [full.clone(), StockMemory::default()] {
            let memory = Memory::from(stock.clone());
            assert_eq!(
                memory.keys().map(String::as_str).collect::<Vec<&str>>(),
                keys
            );
            assert_eq!(memory_check(&memory), Ok(()));
            assert_eq!(StockMemory::from(&memory), stock);
            let bytes = encode_memory(&memory);
            assert_eq!(encode_memory(&decode_memory(&bytes)), bytes);
        }
        assert_eq!(STOCK_MEMORY_KEY_NAMES.len(), 15);
        assert_eq!(StockMemory::from(&Memory::new()), StockMemory::default());
        assert_eq!(
            StockMemory::from(&decode_memory(&encode_memory(&Memory::from(full.clone())))),
            full
        );
    }

    /// Two hostile rows a view holds, tied: the same held range and the same
    /// facet, ids `0x30` and `0x10`.
    fn tied() -> (Arena, MindView) {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 30_500)),
            line_tank(BLUE2, PartyId::BLUE, pose(30_500, 20_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x30));
        hold(&mut arena, RED, BLUE2, ContactId(0x10));
        let mut view = view_of(&arena, RED);
        for row in &mut view.contacts {
            row.range_m = 100;
            row.aspect = Aspect::Front;
        }
        (arena, view)
    }

    /// **The utility score breaks a tie by `ContactId`** (the divergence
    /// `tiebreak`): two rows tied on score answer the lower id, the hint wins
    /// outright, a row that reads dead or is unclassified is no candidate, a
    /// friend is none, a rear beats a front at one range, and a hull with
    /// nothing loaded has no target at all.
    #[test]
    fn m1a6_best_hostile_breaks_a_tie_by_contact_id() {
        let (arena, view) = tied();
        let cfg = &arena.arena_cfg.reflex;
        let ranked = best_hostile_ranked(&view, &Weights::of(cfg), 1_000, None, |_| true);
        assert_eq!(ranked.len(), 2);
        assert_eq!(
            ranked.first().map(|(score, _)| *score),
            ranked.get(1).map(|(score, _)| *score)
        );
        let best = |view: &MindView, prefer: Option<ContactId>| {
            best_hostile(view, &Weights::of(cfg), 1_000, prefer, |_| true)
        };
        assert_eq!(best(&view, None), Some(ContactId(0x10)));
        assert_eq!(best(&view, Some(ContactId(0x30))), Some(ContactId(0x30)));
        assert_eq!(
            best_hostile(&view, &Weights::of(cfg), 1_000, None, |row| row.id
                != ContactId(0x10)),
            Some(ContactId(0x30))
        );
        let with = |change: &dyn Fn(&mut ViewContact)| {
            let mut changed = view.clone();
            for row in &mut changed.contacts {
                change(row);
            }
            changed
        };
        let dead = with(&|row| {
            if row.id == ContactId(0x10) {
                row.reads_dead = Some(true);
            }
        });
        assert_eq!(best(&dead, None), Some(ContactId(0x30)));
        let unknown = with(&|row| {
            row.class = Classification::Unknown;
            row.relation = crate::arena::party::Relation::Unknown;
        });
        assert_eq!(best(&unknown, None), None);
        let friends = with(&|row| {
            row.class = Classification::Classified {
                party: PartyId::RED,
            };
            row.relation = crate::arena::party::Relation::Own;
        });
        assert_eq!(best(&friends, None), None);
        let faces = with(&|row| {
            if row.id == ContactId(0x30) {
                row.aspect = Aspect::Rear;
            }
        });
        assert_eq!(best(&faces, None), Some(ContactId(0x30)));
        let mut unarmed = view.clone();
        unarmed.weapons.clear();
        assert_eq!(best(&unarmed, None), None);
    }

    /// **Hold the plot, move the truth** (doc 17 §3, D-UM-4): two views built
    /// from one arena before and after a held hostile's LIVE move — no pass
    /// between — are equal, so every kernel over them answers the same: the
    /// score, the ranges, the line to the plot cell, the stand-off cell, and a
    /// whole think's intents and memory. (The reflex's `reflex_has_line` reads
    /// the live pose; `tests/twin.rs` counts where the two part.)
    #[test]
    fn m1a6_every_kernel_holds_the_plot_while_the_truth_moves() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let before = view_of(&arena, RED);
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose.x_cm += 15_000;
            state.pose.y_cm -= 12_000;
            state.pose.heading_mdeg = 180_000;
        }
        let after = view_of(&arena, RED);
        assert_eq!(before, after, "a live move is not on the view");
        let cfg = &arena.arena_cfg.reflex;
        let def = &arena.definition;
        for view in [&before, &after] {
            assert_eq!(
                best_hostile(view, &Weights::of(cfg), health_permille(view), None, |_| {
                    true
                }),
                Some(ContactId(0x77))
            );
        }
        assert_eq!(
            view_ranges(&before, cfg.engage_band_permille, None),
            view_ranges(&after, cfg.engage_band_permille, None)
        );
        let (Some(a), Some(b)) = (before.contacts.first(), after.contacts.first()) else {
            panic!("the row");
        };
        let (Some(own), Some(cell)) = (before.cell, a.cell) else {
            panic!("on the window");
        };
        assert_eq!(Some(cell), b.cell);
        assert!(line_to(def, own, cell), "open ground");
        assert_eq!(
            stand_off_of(def, &before, a, 30_000),
            stand_off_of(def, &after, b, 30_000)
        );
        let mut one = StockMemory::default();
        let mut two = StockMemory::default();
        assert_eq!(
            think(&arena, &before, &mut one, 1_000),
            think(&arena, &after, &mut two, 1_000)
        );
        assert_eq!(one, two);
    }

    /// **The emission order, the no-`Heading`-with-a-`Goal(Cell)` rule and the
    /// cap**: a cold hull answers `Startup`; a patrol hands the motor a cell
    /// and nothing else, and the same think again hands it nothing (the goal
    /// did not change) — 17b cut M1b.3: a view with no slot armed answers no
    /// disarm, where 17b cut M1b.2 disarmed every think; a hostile at
    /// 200 m with a line is engaged — the arming, the lock and the cited shot
    /// on the think that finds it — and the next think stands: the arming,
    /// `Goal(Hold)`, the facing `Heading`, then the lock and the shot, in that
    /// order. (17b cut M1b.2 put the `Condition` rows in, after `Startup` and
    /// before `Goal`; the view here is never landed — it shows no slot armed,
    /// so every think that reaches `engage_orders` arms, and every lock is
    /// fresh and cites slot 1.)
    #[test]
    fn m1a6_the_emission_order_and_its_two_rules() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        let mut memory = StockMemory::default();
        let cold = think(&arena, &view_of(&arena, RED), &mut memory, 0);
        // 17b cut M1b.3: a cold hull with no slot armed has nothing to disarm.
        assert_eq!(cold, vec![IntentKind::Startup]);
        online(&mut arena, RED);
        let mut patrol = StockMemory::default();
        let walk = think(&arena, &view_of(&arena, RED), &mut patrol, 1_000);
        emission_holds(&walk);
        assert!(
            matches!(walk.as_slice(), [IntentKind::Goal(MotorGoal::Cell { .. })]),
            "{walk:?}"
        );
        let again = think(&arena, &view_of(&arena, RED), &mut patrol, 2_000);
        assert_eq!(again, Vec::<IntentKind>::new(), "the goal did not change");
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut fight = StockMemory::default();
        let found = think(&arena, &view_of(&arena, RED), &mut fight, 1_000);
        emission_holds(&found);
        assert_eq!(fight.state, MachineState::Engage);
        assert!(
            matches!(
                found.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Lock { contact },
                    IntentKind::Fire { slot: Some(1), .. }
                ] if *contact == ContactId(0x77)
            ),
            "{found:?}"
        );
        let stands = think(&arena, &view_of(&arena, RED), &mut fight, 2_000);
        emission_holds(&stands);
        assert!(
            matches!(
                stands.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { mdeg: 0 },
                    IntentKind::Lock { .. },
                    IntentKind::Fire { slot: Some(1), .. }
                ]
            ),
            "{stands:?}"
        );
        // One cannon, one slot: its arming, then the most a stand emits.
        assert_eq!(stands.len(), 1 + 4);
        assert_eq!(fight.goal, None, "a stand holds no goal");
    }

    /// **The NoRoute port** (departure (i)): a view carrying the
    /// matching motor result for the memory's goal performs the reflex's `unreachable` edge a
    /// think late — the search's plot and the hunt's cell on that goal given
    /// up (the hunt's clock restarted), an `Advance` to it ended by
    /// `ignore_until_ms` (its `expires_ms`) — and hands the motor the fallback waypoint
    /// ([`fallback_waypoint`]: from (3, 4) the nearest node, west at (1, 4))
    /// as the next `Goal(Cell)`.
    #[test]
    fn m5r_a_matching_no_route_result_performs_the_give_ups_and_the_fallback() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 3_500, 4_500)]);
        online(&mut arena, RED);
        let failed = (6, 4);
        let mut view = view_of(&arena, RED);
        view.motor = Some(crate::arena::step::FrameMotor {
            requested_goal: failed,
            outcome: Some(DriveOutcome::NoRoute),
        });
        let start = StockMemory {
            state: MachineState::Patrol,
            goal: Some(failed),
            waypoint: Some(1),
            last_plot: Some((failed, 9_000)),
            hunt: Some(failed),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        assert_eq!(
            fallback_waypoint(
                &arena.definition,
                (3, 4),
                Some(failed),
                Some(1),
                None,
                RED.0
            ),
            (Some(0), Some(1), Some((1, 4)))
        );
        let mut memory = start.clone();
        let intents = think(&arena, &view, &mut memory, 10_000);
        emission_holds(&intents);
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 1, y: 4 })]
        );
        assert_eq!(memory.last_plot, None);
        assert_eq!(memory.hunt, None);
        assert_eq!(memory.unheld_since_ms, Some(10_000));
        assert_eq!((memory.waypoint, memory.from_waypoint), (Some(0), Some(1)));
        assert_eq!(memory.goal, Some((1, 4)));
        assert_eq!(memory.ignore_until_ms, None);
        // The same result under an `Advance` to the failed cell ends the order.
        view.order = Some(FrameOrder {
            identity: None,
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Advance { to: failed },
            hold_fire: false,
            max_range_band: None,
            expires_ms: 60_000,
        });
        let mut ordered = start;
        let _ = think(&arena, &view, &mut ordered, 10_000);
        assert_eq!(
            ordered.ignore_until_ms,
            Some(60_000),
            "the order is over for this hull, by its deadline"
        );
        // No completion, no give-up: the hull drives on to the goal it holds.
        let mut quiet = StockMemory {
            state: MachineState::Patrol,
            goal: Some((1, 4)),
            waypoint: Some(0),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        assert_eq!(
            think(&arena, &view_of(&arena, RED), &mut quiet, 10_000),
            Vec::<IntentKind>::new()
        );
    }

    /// **The `drive_arrived` arrival** (departure (d)): a hunt whose cell the
    /// motor fanned the hull off — the hull nowhere near the named cell — is
    /// over when the motor says it arrived: the hunt is spent, its clock
    /// restarted, the walk forgotten and resumed from the nearest node, which
    /// is the next goal. Without completion the hull drives on.
    #[test]
    fn m5r_motor_completion_is_arrival_wherever_the_fan_landed() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 3_500, 4_500)]);
        online(&mut arena, RED);
        let named = (50, 40);
        let start = StockMemory {
            state: MachineState::Patrol,
            goal: Some(named),
            hunt: Some(named),
            waypoint: Some(1),
            from_waypoint: Some(0),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        let mut view = view_of(&arena, RED);
        let mut on = start.clone();
        assert_eq!(
            think(&arena, &view, &mut on, 5_000),
            Vec::<IntentKind>::new(),
            "driving on"
        );
        assert_eq!(on.hunt, Some(named));
        view.motor = Some(crate::arena::step::FrameMotor {
            requested_goal: named,
            outcome: Some(DriveOutcome::Arrived),
        });
        let mut arrived = start;
        let intents = think(&arena, &view, &mut arrived, 5_000);
        assert_eq!(arrived.hunt, None);
        assert_eq!(arrived.unheld_since_ms, Some(5_000));
        assert_eq!(arrived.waypoint, Some(0), "the nearest node, west");
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 1, y: 4 })]
        );
    }

    #[test]
    fn m5r_nearby_cells_and_stale_drive_notices_do_not_complete_a_new_goal() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 3_500, 4_500)]);
        online(&mut arena, RED);
        let goal = (3, 4);
        let start = StockMemory {
            state: MachineState::Patrol,
            goal: Some(goal),
            hunt: Some(goal),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        let mut view = view_of(&arena, RED);
        view.notices = vec![
            FrameNotice::Drive {
                outcome: DriveOutcome::Arrived,
            },
            FrameNotice::Drive {
                outcome: DriveOutcome::NoRoute,
            },
        ];
        for outcome in [DriveOutcome::Arrived, DriveOutcome::NoRoute] {
            view.motor = Some(crate::arena::step::FrameMotor {
                requested_goal: (50, 40),
                outcome: Some(outcome),
            });
            let mut memory = start.clone();
            let _ = think(&arena, &view, &mut memory, 5_000);
            assert_eq!(memory.hunt, Some(goal));
            assert_eq!(memory.goal, Some(goal));
            assert_eq!(memory.unheld_since_ms, Some(0));
        }
    }

    #[test]
    fn m5r_motor_completion_survives_withdraw_dwell_without_repeated_notices() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 20_500, 20_500)]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        let home = (1, 4);
        view.motor = Some(crate::arena::step::FrameMotor {
            requested_goal: home,
            outcome: Some(DriveOutcome::Arrived),
        });
        let mut memory = StockMemory {
            state: MachineState::Withdraw,
            goal: Some(home),
            ordered: false,
            ..StockMemory::default()
        };
        let _ = think(&arena, &view, &mut memory, 1_000);
        assert_eq!(memory.quiet_since_ms, Some(1_000));
        assert!(view.notices.is_empty());
        let rally = i64::from(arena.arena_cfg.reflex.rally_ms);
        let _ = think(&arena, &view, &mut memory, 1_000 + rally - 1);
        assert_eq!(memory.quiet_since_ms, Some(1_000));
        assert_eq!(memory.state, MachineState::Withdraw);
        let _ = think(&arena, &view, &mut memory, 1_000 + rally);
        assert_eq!(memory.state, MachineState::Patrol);
        assert!(memory.rallied);
    }

    /// A [`NativeStockMind`] that keeps every answer it gives.
    #[derive(Debug)]
    struct Kept {
        inner: NativeStockMind,
        answers: Mutex<Vec<(MindView, ThinkResult)>>,
    }

    impl MindRunner for Kept {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            let result = self.inner.think(job);
            if let Ok(mut answers) = self.answers.lock() {
                answers.push(((**job.view).clone(), result.clone()));
            }
            result
        }
    }

    /// **An unarmed hull's fire half is absent** (D-UM-48; doc 17 §22's italic
    /// line): a weaponless mind with two classified hostiles in its sight for
    /// a scripted minute starts up, drives, and never emits a `Lock` or a
    /// `Fire` — not one refused, none at all — and its view carries no weapon
    /// row, no odds and no `reads_dead`.
    #[test]
    fn m1a6_an_unarmed_hull_emits_no_lock_and_no_fire() {
        let mut red = scout(RED, PartyId::RED, pose(20_500, 20_500));
        red.weapons.clear();
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut arena = field(vec![
            red,
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 30_500)),
            line_tank(BLUE2, PartyId::BLUE, pose(30_500, 20_500)),
        ]);
        let kept = Arc::new(Kept {
            inner: NativeStockMind::with(&arena.arena_cfg.reflex),
            answers: Mutex::new(Vec::new()),
        });
        arena.mind_runner = kept.clone();
        let mut wall = 0_i64;
        while arena.now_ms < 60_000 {
            let _ = arena.step(wall, &[], &[]);
            wall += 250;
        }
        let answers = kept
            .answers
            .lock()
            .map(|answers| answers.clone())
            .unwrap_or_default();
        assert!(answers.len() >= 55, "{} thinks", answers.len());
        assert!(
            answers
                .iter()
                .any(|(view, _)| view.contacts.iter().any(|row| matches!(
                    row.class,
                    Classification::Classified {
                        party: PartyId::BLUE
                    }
                ))),
            "hostiles in view"
        );
        assert!(
            answers
                .iter()
                .any(|(_, result)| result.intents.contains(&IntentKind::Startup))
        );
        assert!(
            answers.iter().any(|(_, result)| result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))),
            "it drives"
        );
        for (view, result) in &answers {
            emission_holds(&result.intents);
            // 17b cut M1b.2: nor a `Condition` — an unarmed hull has no slot to
            // answer (D-UM-48).
            assert!(
                !result.intents.iter().any(|kind| matches!(
                    kind,
                    IntentKind::Lock { .. } | IntentKind::Fire { .. } | IntentKind::Condition(_)
                )),
                "{:?}",
                result.intents
            );
            assert!(view.weapons.is_empty());
            assert!(
                view.contacts
                    .iter()
                    .all(|row| row.odds.is_empty() && row.reads_dead.is_none())
            );
        }
    }

    // -----------------------------------------------------------------------
    // The ported arms the twin lane never reaches (no Hold match, no order)
    // -----------------------------------------------------------------------

    /// A friendly line tank, the supporter's charge.
    const MATE: ObjId = ObjId(12);

    /// A standing order of `objective` for `red-1`, a minute long.
    fn order_of(objective: MindObjective) -> FrameOrder {
        FrameOrder {
            identity: None,
            squad: SmolStr::new("red-1"),
            objective,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 60_000,
        }
    }

    /// The Hold objective on the spot `(30, 20)`, a three-cell square.
    fn spot() -> HoldObjective {
        HoldObjective {
            spot: (30, 20),
            letter: Some('A'),
            radius_cells: 3,
        }
    }

    /// One think of the stock mind handed the match's Hold objective.
    fn think_holding(
        arena: &Arena,
        view: &MindView,
        memory: &mut StockMemory,
        hold: &HoldObjective,
        t_ms: i64,
    ) -> Vec<IntentKind> {
        NativeStockMind::with(&arena.arena_cfg.reflex).decide(
            view,
            memory,
            &arena.definition,
            Some(hold),
            t_ms,
            t_ms.saturating_add(arena.substep_len_ms()),
            &crate::arena::card::line(&arena.arena_cfg.reflex),
            CLASS_I_SLOTS,
        )
    }

    /// A patrolling memory with nothing seen and the hunt's clock just set.
    fn patrolling(t_ms: i64) -> StockMemory {
        StockMemory {
            state: MachineState::Patrol,
            unheld_since_ms: Some(t_ms),
            ..StockMemory::default()
        }
    }

    /// **The Hold arms** (the reflex's `hold_goal`, 16b cut O.1; `Patrol`'s
    /// list): a patroller handed the Hold objective drives for the spot (the
    /// hunt's clock restarted), parks once it stands within the square's
    /// arrival ring (`Goal(Hold)`), and a `drive_no_route` for the spot sets
    /// the retry clock at `max(hunt_after_ms, repath_ms)` and falls back to a
    /// waypoint; inside the retry window the spot is declined for the walk,
    /// and past it the spot is the goal again and the clock is spent.
    #[test]
    fn m1a6_the_hold_arms_drive_for_the_spot_park_on_it_and_retry_it() {
        let hold = spot();
        let mut arena = field(vec![minded(RED, PartyId::RED, 20_500, 20_500)]);
        online(&mut arena, RED);
        let view = view_of(&arena, RED);
        let mut memory = patrolling(0);
        let intents = think_holding(&arena, &view, &mut memory, &hold, 1_000);
        emission_holds(&intents);
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 30, y: 20 })]
        );
        assert_eq!(memory.goal, Some(hold.spot));
        assert_eq!(memory.unheld_since_ms, Some(1_000));
        assert_eq!(memory.hold_retry_ms, None);
        // No route reaches it: the retry clock, and the fallback waypoint.
        let mut refused = view.clone();
        refused.motor = Some(crate::arena::step::FrameMotor {
            requested_goal: hold.spot,
            outcome: Some(DriveOutcome::NoRoute),
        });
        let retry = {
            let cfg = &arena.arena_cfg.reflex;
            2_000 + i64::from(cfg.hunt_after_ms.max(cfg.repath_ms))
        };
        let fell_back = think_holding(&arena, &refused, &mut memory, &hold, 2_000);
        emission_holds(&fell_back);
        assert_eq!(memory.hold_retry_ms, Some(retry));
        assert!(
            matches!(
                fell_back.as_slice(),
                [
                    IntentKind::Goal(MotorGoal::Cell { x, y })
                ] if (*x, *y) != hold.spot
            ),
            "{fell_back:?}"
        );
        assert_ne!(memory.goal, Some(hold.spot));
        // Inside the window the spot is declined; the walk holds its goal.
        let declined = think_holding(&arena, &view, &mut memory, &hold, retry - 1);
        assert!(
            !declined.contains(&IntentKind::Goal(MotorGoal::Cell { x: 30, y: 20 })),
            "{declined:?}"
        );
        assert_eq!(memory.hold_retry_ms, Some(retry));
        // Past it, the spot again.
        let again = think_holding(&arena, &view, &mut memory, &hold, retry);
        assert_eq!(
            again,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 30, y: 20 })]
        );
        assert_eq!(memory.hold_retry_ms, None);
        // On the spot: parked.
        let mut there = field(vec![minded(RED, PartyId::RED, 31_500, 20_500)]);
        online(&mut there, RED);
        let mut parked = StockMemory {
            goal: Some(hold.spot),
            ..patrolling(0)
        };
        let stop = think_holding(&there, &view_of(&there, RED), &mut parked, &hold, 3_000);
        assert_eq!(stop, vec![IntentKind::Goal(MotorGoal::Hold)]);
        assert_eq!(parked.goal, None);
        assert_eq!(parked.state, MachineState::Patrol);
    }

    /// **An `Advance` into the Hold square ends on arrival** (16b cut O.1's
    /// order end, as a mind performs it: `ignore_until_ms`), and the hull is
    /// parked by the Hold arm on the same think; with no Hold objective the
    /// same `Advance` stands and is driven. **And `Approach` inside the square
    /// does not leave it** (16b cut O.1): a target past the hull's reach is
    /// approached by holding the cell it stands on, not by the stand-off cell
    /// out of the square.
    #[test]
    fn m1a6_an_advance_into_the_square_ends_and_an_approach_inside_it_stays() {
        let hold = spot();
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 31_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(31_500, 55_500)),
        ]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Advance { to: (32, 20) }));
        let mut memory = patrolling(0);
        let ended = think_holding(&arena, &view, &mut memory, &hold, 1_000);
        assert_eq!(
            memory.ignore_until_ms,
            Some(60_000),
            "the Advance is over for this hull, by its deadline"
        );
        assert_eq!(ended, vec![IntentKind::Goal(MotorGoal::Hold)]);
        // With no Hold objective the Advance stands, and is driven.
        let mut skirmish = patrolling(0);
        let driven = think(&arena, &view, &mut skirmish, 1_000);
        assert_eq!(skirmish.ignore_until_ms, None);
        assert_eq!(
            driven,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 32, y: 20 })]
        );
        // `Approach`, inside the square, on a row past its reach.
        hold_contact(&mut arena, RED, BLUE, ContactId(0x55));
        let mut view = view_of(&arena, RED);
        let cfg = &arena.arena_cfg.reflex;
        let ranges = view_ranges(&view, cfg.engage_band_permille, None);
        // The row read past the hull's reach (the field is narrower than a
        // stock tank's 600 m sight).
        for row in &mut view.contacts {
            row.range_m = ranges.release_cm / 100 + 100;
        }
        let Some(row) = view.contacts.first() else {
            panic!("the blue row");
        };
        assert!(
            row_range_cm(row) > ranges.release_cm,
            "the precondition: {} past {}",
            row_range_cm(row),
            ranges.release_cm
        );
        let mut closing = StockMemory {
            state: MachineState::Approach,
            target: Some(ContactId(0x55)),
            ..patrolling(0)
        };
        let inside = think_holding(&arena, &view, &mut closing, &hold, 1_000);
        assert_eq!(closing.state, MachineState::Approach);
        assert_eq!(
            inside,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 31, y: 20 })],
            "it holds its own cell"
        );
        let mut outside = closing.clone();
        outside.goal = None;
        let out = think(&arena, &view, &mut outside, 1_000);
        assert!(
            matches!(out.as_slice(), [IntentKind::Goal(MotorGoal::Cell { .. })]) && out != inside,
            "on a skirmish it drives for the stand-off cell: {out:?}"
        );
    }

    #[test]
    fn m5r_an_advance_outside_the_hold_edge_waits_for_its_motor_completion() {
        use crate::arena::step::FrameMotor;

        let hold = spot();
        let edge = (27, 20);
        let mut arena = field(vec![minded(RED, PartyId::RED, 26_500, 20_500)]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Advance { to: edge }));
        assert!(!hold.square_contains(view.cell.expect("own cell")));
        for motor in [
            None,
            Some(FrameMotor {
                requested_goal: edge,
                outcome: None,
            }),
            Some(FrameMotor {
                requested_goal: (26, 20),
                outcome: Some(DriveOutcome::Arrived),
            }),
        ] {
            view.motor = motor;
            let mut memory = patrolling(0);
            let _ = think_holding(&arena, &view, &mut memory, &hold, 1_000);
            assert_eq!(
                memory.ignore_until_ms, None,
                "proximity or stale completion cannot end Advance"
            );
            assert_eq!(memory.goal, Some(edge));
        }
        view.motor = Some(FrameMotor {
            requested_goal: edge,
            outcome: Some(DriveOutcome::Arrived),
        });
        let mut memory = patrolling(0);
        let intents = think_holding(&arena, &view, &mut memory, &hold, 1_000);
        assert_eq!(memory.ignore_until_ms, Some(60_000));
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 30, y: 20 })]
        );
        let failed = FrameMotor {
            requested_goal: edge,
            outcome: Some(DriveOutcome::NoRoute),
        };
        assert!(!advance_reached_the_hold(
            &hold,
            edge,
            view.cell,
            Some(&failed)
        ));
        assert!(
            advance_reached_the_hold(&hold, edge, Some((28, 20)), None),
            "inside-square handover stays geometric"
        );
    }

    /// `viewer` holds `target` under `id` (the module's `hold`, named apart
    /// from the Hold objective).
    fn hold_contact(arena: &mut Arena, viewer: ObjId, target: ObjId, id: ContactId) {
        hold(arena, viewer, target, id);
    }

    /// **The screen** (16b cut Z.2): a screening patroller parks, faces the
    /// order's bearing, and stands on the best hostile **inside the arc** off
    /// the row's whole degrees — the lock and the shot, faced onto the row —
    /// and never on one outside it, which it neither locks nor chases.
    #[test]
    fn m1a6_a_screen_stands_on_a_row_inside_its_arc_and_not_outside() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 30_500)),
            line_tank(BLUE2, PartyId::BLUE, pose(30_500, 20_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE2, ContactId(0x22));
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Screen { facing_mdeg: 0 }));
        let mut memory = patrolling(0);
        let outside = think(&arena, &view, &mut memory, 1_000);
        emission_holds(&outside);
        assert_eq!(
            outside,
            vec![
                IntentKind::Goal(MotorGoal::Hold),
                IntentKind::Heading { mdeg: 0 }
            ],
            "the row at 90° is outside a 60° half-arc"
        );
        assert_eq!(memory.target, None);
        assert_eq!(memory.state, MachineState::Patrol);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Screen { facing_mdeg: 0 }));
        let mut memory = patrolling(0);
        let inside = think(&arena, &view, &mut memory, 1_000);
        emission_holds(&inside);
        assert!(
            matches!(
                inside.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { mdeg: 0 },
                    IntentKind::Lock { contact },
                    IntentKind::Fire { slot: Some(1), .. }
                ] if *contact == ContactId(0x11)
            ),
            "{inside:?}"
        );
        assert_eq!(
            memory.state,
            MachineState::Patrol,
            "a screen never gives chase"
        );
        assert_eq!(memory.last_plot.map(|(cell, _)| cell), Some((20, 30)));
    }

    /// **The supporter** (16b cut Z.2): blind to its charge (`of: None`; a
    /// self-charge read so until 17b cut L.2 re-keyed it to `Engage`) it holds
    /// where it stands and fights from there; past the leash it drives back
    /// toward its station off its own plot of the charge and fires nothing,
    /// a hostile at point blank or not; inside its station it parks.
    #[test]
    fn m1a6_a_supporter_stands_blind_rejoins_off_the_leash_and_keeps_station() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 30_500)),
            line_tank(MATE, PartyId::RED, pose(20_500, 50_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let view = view_of(&arena, RED);
        // (a) Blind.
        let mut blind_view = view.clone();
        blind_view.order = Some(order_of(MindObjective::Support { of: None }));
        let mut blind = patrolling(0);
        let stood = think(&arena, &blind_view, &mut blind, 1_000);
        emission_holds(&stood);
        assert!(
            matches!(
                stood.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. },
                    IntentKind::Lock { .. },
                    IntentKind::Fire { slot: Some(1), .. }
                ]
            ),
            "{stood:?}"
        );
        assert_eq!(blind.target, None);
        assert_eq!(blind.state, MachineState::Patrol);
        // (b) Off the leash: 30 cells from the charge's plot.
        hold(&mut arena, RED, MATE, ContactId(0x99));
        let mut leashed_view = view_of(&arena, RED);
        leashed_view.order = Some(order_of(MindObjective::Support {
            of: Some(ContactId(0x99)),
        }));
        let mut leashed = StockMemory {
            state: MachineState::Engage,
            target: Some(ContactId(0x11)),
            ..patrolling(0)
        };
        let back = think(&arena, &leashed_view, &mut leashed, 1_000);
        emission_holds(&back);
        assert!(
            matches!(
                back.as_slice(),
                [
                    IntentKind::Goal(MotorGoal::Cell { y, .. })
                ] if *y > 20
            ),
            "toward the station, south, firing nothing — no slot armed to clear: {back:?}"
        );
        assert_eq!(leashed.state, MachineState::Patrol);
        assert_eq!(leashed.target, None);
        // (c) Inside its station: parked.
        let mut close = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(MATE, PartyId::RED, pose(20_500, 23_500)),
        ]);
        online(&mut close, RED);
        hold(&mut close, RED, MATE, ContactId(0x99));
        let mut station_view = view_of(&close, RED);
        station_view.order = leashed_view.order.clone();
        let mut station = patrolling(0);
        let parked = think(&close, &station_view, &mut station, 1_000);
        assert_eq!(parked, vec![IntentKind::Goal(MotorGoal::Hold)]);
    }

    /// **An `Engage` hint outranks a better score** (16b cut N.2; the hint
    /// re-keyed at write time, D-UM-15): of two rows the score prefers the
    /// rear-facing one, and an `ENGAGE` naming the other settles on, locks and
    /// shoots the named one.
    #[test]
    fn m1a6_an_engage_hint_outranks_a_better_score() {
        let (arena, mut view) = tied();
        for row in &mut view.contacts {
            if row.id == ContactId(0x10) {
                row.aspect = Aspect::Rear;
            }
        }
        let mut free = patrolling(0);
        let _ = think(&arena, &view, &mut free, 1_000);
        assert_eq!(free.target, Some(ContactId(0x10)), "the score's pick");
        view.order = Some(order_of(MindObjective::Engage {
            target: Some(ContactId(0x30)),
        }));
        let mut told = patrolling(0);
        let intents = think(&arena, &view, &mut told, 1_000);
        assert_eq!(told.target, Some(ContactId(0x30)));
        assert!(
            intents.contains(&IntentKind::Lock {
                contact: ContactId(0x30)
            }),
            "{intents:?}"
        );
    }

    /// **The expiry's restoration, read off the view** (`drop_to_doctrine`):
    /// a healthy `Withdraw` whose order is gone from the view goes back to
    /// `Patrol` with its target dropped; under a live `WITHDRAW` it stays,
    /// and a hurt one keeps going home with no order.
    #[test]
    fn m1a6_a_healthy_withdraw_with_no_order_returns_to_patrol() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 20_500, 20_500)]);
        online(&mut arena, RED);
        let view = view_of(&arena, RED);
        let withdrawing = StockMemory {
            state: MachineState::Withdraw,
            target: Some(ContactId(0x11)),
            // 17b cut M4.3: the order's `Withdraw`, which the expiry restores.
            ordered: true,
            ..StockMemory::default()
        };
        let mut expired = withdrawing.clone();
        let _ = think(&arena, &view, &mut expired, 1_000);
        assert_eq!(expired.state, MachineState::Patrol);
        assert_eq!(expired.target, None);
        assert!(!expired.ordered, "the restoration spends the flag");
        // 17b cut M4.3: a `Withdraw` no order put it in — a released seat's
        // seed — is held under no order, healthy and not dry.
        let mut seeded = StockMemory {
            ordered: false,
            ..withdrawing.clone()
        };
        let _ = think(&arena, &view, &mut seeded, 1_000);
        assert_eq!(seeded.state, MachineState::Withdraw);
        let mut ordered_view = view.clone();
        ordered_view.order = Some(order_of(MindObjective::Withdraw));
        let mut ordered = withdrawing.clone();
        let _ = think(&arena, &ordered_view, &mut ordered, 1_000);
        assert_eq!(ordered.state, MachineState::Withdraw);
        let mut hurt_view = view;
        for section in &mut hurt_view.sections {
            if let Some((_, max)) = section.internals {
                section.internals = Some((0, max));
            }
            for facet in &mut section.facets {
                facet.armor = 0;
            }
        }
        assert!(health_permille(&hurt_view) < arena.arena_cfg.reflex.withdraw_permille);
        let mut hurt = withdrawing;
        let _ = think(&arena, &hurt_view, &mut hurt, 1_000);
        assert_eq!(hurt.state, MachineState::Withdraw);
    }

    /// **The `no line` edge off a miss notice** (W's `blocked`/`cover`; the
    /// divergence `no_line`): an `Engage` holding its band stands and shoots;
    /// the same think with a `blocked` miss of its own round in the view drops
    /// to `Approach`, throws the stand away and drives for the stand-off cell
    /// — no lock, no shot — and the flag is spent.
    #[test]
    fn m1a6_a_blocked_miss_notice_turns_engage_into_approach() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut view = view_of(&arena, RED);
        let ranges = view_ranges(&view, arena.arena_cfg.reflex.engage_band_permille, None);
        // Between the two edges: `Engage` keeps it, `Approach` would close.
        let between_m = (ranges.engage_cm + ranges.release_cm) / 200;
        for row in &mut view.contacts {
            row.range_m = between_m;
        }
        let engaged = StockMemory {
            state: MachineState::Engage,
            target: Some(ContactId(0x77)),
            ..patrolling(0)
        };
        let mut standing = engaged.clone();
        let stand = think(&arena, &view, &mut standing, 1_000);
        assert_eq!(standing.state, MachineState::Engage);
        assert!(
            stand.contains(&IntentKind::Goal(MotorGoal::Hold)),
            "{stand:?}"
        );
        view.notices = vec![FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new(BLOCKED),
            launch_cell: view.cell,
        }];
        let mut blocked = engaged;
        let closed = think(&arena, &view, &mut blocked, 1_000);
        emission_holds(&closed);
        assert_eq!(blocked.state, MachineState::Approach);
        assert!(!blocked.no_line, "spent");
        assert!(
            matches!(
                closed.as_slice(),
                [IntentKind::Goal(MotorGoal::Cell { .. })]
            ),
            "{closed:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 17b cut M1b.2: T1 arms its slots at think time and cites on a fresh lock
    // -----------------------------------------------------------------------

    /// A weapon row named `id` off `like`, its reach `reach_cm`, its heat
    /// `heat_per_shot`.
    fn row_like(like: &FrameWeapon, id: &str, reach_cm: u32, heat_per_shot: u32) -> FrameWeapon {
        FrameWeapon {
            id: SmolStr::new(id),
            bands_cm: vec![reach_cm / 2, reach_cm],
            heat_per_shot,
            ..like.clone()
        }
    }

    /// The `FireCondition` an `Arm` carries, or a panic.
    fn armed_terms(kind: &IntentKind) -> FireCondition {
        match kind {
            IntentKind::Condition(Arming::Arm(cond)) => cond.clone(),
            other => panic!("not an arming: {other:?}"),
        }
    }

    /// **`arming_for`'s terms, its no-skip rule and its disarm** (17b cut
    /// M1b.2; D-UM-16, D-UM-18, D-UM-43; 17b's adversarial F6). A hull of
    /// three weapons — a laser that makes heat, a cannon, a missile rack —
    /// on a platform AT its heat ceiling: `Some(ceiling)` arms exactly the
    /// first Class I's `slots` in component order, the hot laser
    /// included (no heat skip, so the numbering is the arena's), each on
    /// `BestHostile` with the card's odds, its aspect mask, its reserve as
    /// whole rounds of `rounds_max`, `min_range_m` 0 and `max_range_m` the
    /// lesser of the ceiling and the weapon's reach in metres; `None` is one
    /// `Disarm { None }` beside an armed view row (17b cut M1b.3: and nothing
    /// beside none); an unarmed hull answers nothing (D-UM-48).
    #[test]
    fn m1b2_arming_for_arms_the_first_two_weapons_whatever_the_heat() {
        let mut arena = field(vec![minded(RED, PartyId::RED, 20_500, 20_500)]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        let Some(like) = view.weapons.first().cloned() else {
            panic!("the line tank's cannon row");
        };
        view.weapons = vec![
            row_like(&like, "laser", 60_000, 20),
            row_like(&like, "cannon", 250_000, 0),
            row_like(&like, "missiles", 300_000, 0),
        ];
        view.heat = Some(crate::arena::step::FrameHeat { heat: 60, max: 60 });
        let laser = view
            .weapons
            .first()
            .cloned()
            .unwrap_or_else(|| like.clone());
        assert!(would_overheat(&view, &laser), "the precondition: hot");
        let mut card = crate::arena::card::line(&arena.arena_cfg.reflex);
        card.reserve_pct = 10;
        let arms = arming_for(&view, &card, Some(80_000), CLASS_I_SLOTS);
        assert_eq!(arms.len(), CLASS_I_SLOTS, "{arms:?}");
        let terms: Vec<FireCondition> = arms.iter().map(armed_terms).collect();
        assert_eq!(
            terms,
            vec![
                FireCondition {
                    weapon: SmolStr::new("laser"),
                    target: SlotTarget::BestHostile,
                    min_permille: card.min_odds_permille,
                    aspects: card.aspect.mask(),
                    min_rounds_left: like.rounds_max / 10,
                    min_range_m: 0,
                    // The laser's 600 m reach under the 800 m ceiling.
                    max_range_m: 600,
                },
                FireCondition {
                    weapon: SmolStr::new("cannon"),
                    target: SlotTarget::BestHostile,
                    min_permille: card.min_odds_permille,
                    aspects: card.aspect.mask(),
                    min_rounds_left: like.rounds_max / 10,
                    min_range_m: 0,
                    // The ceiling under the cannon's 2,500 m reach.
                    max_range_m: 800,
                },
            ]
        );
        // On Line the discipline is today's machine's: 100 per mille, every
        // aspect, no reserve.
        let line = crate::arena::card::line(&arena.arena_cfg.reflex);
        let on_line = arming_for(&view, &line, Some(80_000), CLASS_I_SLOTS);
        assert!(
            on_line
                .iter()
                .map(armed_terms)
                .all(|cond| cond.min_permille == 100
                    && cond.aspects == crate::arena::conditions::ASPECTS_ANY
                    && cond.min_rounds_left == 0)
        );
        // 17b cut M1b.3: nothing armed is nothing to disarm.
        assert_eq!(
            arming_for(&view, &line, None, CLASS_I_SLOTS),
            Vec::new(),
            "not reached, nothing armed: nothing said"
        );
        if let (Some(row), Some(cond)) = (view.weapons.get_mut(1), on_line.get(1).map(armed_terms))
        {
            row.armed = Some(FrameArming::of(2, &cond));
        }
        assert_eq!(
            arming_for(&view, &line, None, CLASS_I_SLOTS),
            vec![disarm()],
            "not reached: one disarm"
        );
        view.weapons.clear();
        assert_eq!(
            arming_for(&view, &line, Some(80_000), CLASS_I_SLOTS),
            Vec::new()
        );
        assert_eq!(
            arming_for(&view, &line, None, CLASS_I_SLOTS),
            Vec::new(),
            "D-UM-48"
        );
    }

    /// **When T1 arms, with which ceiling, and when it cites** (17b cut
    /// M1b.2). On the engage field:
    ///
    /// * the think that finds the hostile arms on `release_cm`, locks it and
    ///   cites S1 for the cannon — `Condition`, `Lock`, `Fire { slot: Some(1) }`
    ///   in that order;
    /// * with the lock already held (the view's `lock` on the row) it arms and
    ///   stands but plans NO `Lock` and NO `Fire`: the armed slot owns the shot;
    /// * a fresh lock on a cycling cannon arms and locks and cites nothing
    ///   (today's first-ready rule for the cited shot);
    /// * 17b cut M1b.3: a view whose `armed` row already holds the wanted
    ///   terms answers no `Condition`; one on other terms is re-armed;
    /// * under `hold_fire` the lock is still taken and the slots DISARMED
    ///   (where the view shows one armed — M1b.3);
    /// * powering up (`Starting`) disarms (likewise); cold, `Startup` then the
    ///   disarm;
    /// * a withdraw stand en route arms on `stand_cm`, not `release_cm`.
    #[test]
    fn m1b2_t1_arms_on_the_admitting_ceiling_and_cites_only_beside_a_fresh_lock() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let view = view_of(&arena, RED);
        let cfg = &arena.arena_cfg.reflex;
        let ranges = view_ranges(&view, cfg.engage_band_permille, None);
        let reach = view
            .weapons
            .first()
            .and_then(|weapon| weapon.bands_cm.last().copied())
            .map_or(0, u64::from);
        let release_m = u32::try_from(ranges.release_cm.min(reach) / 100).unwrap_or(u32::MAX);
        // Found: arm, lock, cite.
        let mut fight = StockMemory::default();
        let found = think(&arena, &view, &mut fight, 1_000);
        emission_holds(&found);
        assert_eq!(fight.state, MachineState::Engage);
        let [arm, lock, fire] = found.as_slice() else {
            panic!("{found:?}");
        };
        assert_eq!(armed_terms(arm).max_range_m, release_m);
        assert_eq!(armed_terms(arm).target, SlotTarget::BestHostile);
        assert_eq!(
            lock,
            &IntentKind::Lock {
                contact: ContactId(0x77)
            }
        );
        assert_eq!(
            fire,
            &IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: Some(1)
            }
        );
        // Held: arm and stand, no lock and no fire.
        let mut held_view = view.clone();
        held_view.lock = Some(ContactId(0x77));
        let mut holding = fight.clone();
        let stands = think(&arena, &held_view, &mut holding, 2_000);
        emission_holds(&stands);
        assert!(
            matches!(
                stands.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. }
                ]
            ),
            "the slot owns the shot: {stands:?}"
        );
        // A fresh lock on a cycling gun: armed, locked, nothing cited.
        let mut cycling = view.clone();
        for weapon in &mut cycling.weapons {
            weapon.ready = false;
            weapon.ready_in_ms = 4_000;
        }
        let mut busy = StockMemory::default();
        let locked = think(&arena, &cycling, &mut busy, 1_000);
        assert!(
            matches!(
                locked.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Lock { .. }
                ]
            ),
            "{locked:?}"
        );
        // 17b cut M1b.3: the view showing the slot as wanted answers no
        // `Condition` — on change only.
        let mut shown = held_view.clone();
        if let Some(row) = shown.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        let quietly = think(&arena, &shown, &mut fight.clone(), 2_000);
        emission_holds(&quietly);
        assert!(
            matches!(
                quietly.as_slice(),
                [
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. }
                ]
            ),
            "armed as wanted: nothing to say: {quietly:?}"
        );
        // Shown on other terms (a narrower ceiling): one `Arm`, in place.
        let mut narrower = shown.clone();
        if let Some(armed) = narrower
            .weapons
            .first_mut()
            .and_then(|row| row.armed.as_mut())
        {
            armed.max_range_m = armed.max_range_m.saturating_sub(100);
        }
        let rearmed = think(&arena, &narrower, &mut fight.clone(), 2_000);
        assert_eq!(
            rearmed.first().map(armed_terms),
            Some(armed_terms(arm)),
            "{rearmed:?}"
        );
        // Under `hold_fire`: the lock is taken, the slots disarmed — and,
        // 17b cut M1b.3, only where the view shows a slot armed.
        let mut held_fire = view.clone();
        held_fire.order = Some(FrameOrder {
            hold_fire: true,
            ..order_of(MindObjective::Engage { target: None })
        });
        let mut quiet = StockMemory::default();
        let holding_fire = think(&arena, &held_fire, &mut quiet.clone(), 1_000);
        assert_eq!(
            holding_fire,
            vec![IntentKind::Lock {
                contact: ContactId(0x77)
            }],
            "nothing armed: nothing to disarm"
        );
        if let Some(row) = held_fire.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        let holding_fire = think(&arena, &held_fire, &mut quiet, 1_000);
        assert_eq!(
            holding_fire,
            vec![
                disarm(),
                IntentKind::Lock {
                    contact: ContactId(0x77)
                }
            ]
        );
        // Powering up: disarmed where armed, silent where not.
        let mut starting = view.clone();
        starting.power = Power::Starting;
        assert_eq!(
            think(&arena, &starting, &mut StockMemory::default(), 1_000),
            Vec::new()
        );
        if let Some(row) = starting.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        assert_eq!(
            think(&arena, &starting, &mut StockMemory::default(), 1_000),
            vec![disarm()]
        );
        // A withdraw stand en route: the stand's own ceiling.
        let mut home = view.clone();
        home.order = Some(order_of(MindObjective::Withdraw));
        let stand_m = u32::try_from(ranges.stand_cm.min(reach) / 100).unwrap_or(u32::MAX);
        for row in &mut home.contacts {
            row.range_m = (ranges.stand_cm / 100).saturating_sub(10);
        }
        let mut withdrawing = StockMemory {
            state: MachineState::Withdraw,
            goal: Some((1, 1)),
            ..StockMemory::default()
        };
        let cornered = think(&arena, &home, &mut withdrawing, 1_000);
        emission_holds(&cornered);
        assert_eq!(withdrawing.state, MachineState::Withdraw);
        let Some(arm) = cornered.first() else {
            panic!("the cornered hull answers: {cornered:?}");
        };
        assert_eq!(armed_terms(arm).max_range_m, stand_m, "{cornered:?}");
        assert!(
            stand_m < release_m,
            "the precondition: the stand's ceiling is the nearer"
        );
    }

    /// **The citation's weapon is T0's first-ready pick among the armed
    /// weapons, numbered `i + 1`** (17b cut M1b.2): a hot laser first and a
    /// cold cannon second, a fresh lock — the laser is armed (S1) and
    /// skipped for the cited shot (`would_overheat`), the cannon cited as S2;
    /// a weapon past the slots is never cited (it has no slot).
    #[test]
    fn m1b2_the_cited_shot_is_the_first_ready_armed_weapon_numbered_as_the_arena_does() {
        let mut arena = field(vec![
            minded(RED, PartyId::RED, 20_500, 20_500),
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut view = view_of(&arena, RED);
        let Some(like) = view.weapons.first().cloned() else {
            panic!("the cannon row");
        };
        view.weapons = vec![
            row_like(&like, "laser", 250_000, 20),
            row_like(&like, "cannon", 250_000, 0),
        ];
        view.heat = Some(crate::arena::step::FrameHeat { heat: 60, max: 60 });
        let mut fight = StockMemory::default();
        let found = think(&arena, &view, &mut fight, 1_000);
        emission_holds(&found);
        let weapons: Vec<SmolStr> = found
            .iter()
            .filter_map(|kind| match kind {
                IntentKind::Condition(Arming::Arm(cond)) => Some(cond.weapon.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(weapons, vec![SmolStr::new("laser"), SmolStr::new("cannon")]);
        assert_eq!(
            found.last(),
            Some(&IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: Some(2)
            })
        );
        // A third weapon, the only one ready: no slot, so no citation.
        view.weapons = vec![
            row_like(&like, "laser", 250_000, 20),
            FrameWeapon {
                ready: false,
                ready_in_ms: 4_000,
                ..row_like(&like, "cannon", 250_000, 0)
            },
            row_like(&like, "missiles", 250_000, 0),
        ];
        let mut again = StockMemory::default();
        let found = think(&arena, &view, &mut again, 1_000);
        assert!(
            !found
                .iter()
                .any(|kind| matches!(kind, IntentKind::Fire { .. })),
            "the third weapon has no slot to cite: {found:?}"
        );
        assert!(found.contains(&IntentKind::Lock {
            contact: ContactId(0x77)
        }));
    }

    /// **The residual 17b cut M1b.3 removed, pinned** (the M1b.2 review's
    /// correctness F2; `engage_orders`' note): `n = i + 1` is the arena's
    /// slot number only while no armed weapon has been removed. Red carries
    /// the cannon on its turret and a machine gun on its hull, both armed (S1,
    /// S2) by a think's own `arming_for`; the turret's `RemoveWeapons` frees
    /// S1 and leaves the machine gun in S2 — the view's machine-gun row shows
    /// it there, as wanted, so nothing is re-armed. On a fresh lock the think
    /// cites the number its view shows — S2, where M1b.2 cited S1 (`view.weapons`
    /// has closed up) and the resolver refused it `unwarranted` — and the
    /// resolver now admits the citation.
    #[test]
    fn m1b2_residual_a_removed_weapon_parts_i_plus_one_from_the_arenas_number() {
        use crate::arena::resolution::Consequence;
        use crate::arena::weapons::FiredBy;
        let mut red = minded(RED, PartyId::RED, 20_500, 20_500);
        let mut mg = crate::arena::test_support::cannon();
        mg.id = SmolStr::new("machine_gun");
        mg.mount_section = SmolStr::new("hull");
        red.weapons.push(mg);
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("seal the complete initial weapon rack");
        let mut arena = field(vec![
            red,
            line_tank(BLUE, PartyId::BLUE, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let card = crate::arena::card::line(&arena.arena_cfg.reflex);
        let arm = |arena: &mut Arena| {
            let view = view_of(arena, RED);
            for kind in arming_for(&view, &card, Some(100_000), CLASS_I_SLOTS) {
                let IntentKind::Condition(arming) = kind else {
                    panic!("an arming");
                };
                assert!(arena.resolve_arming(RED, &arming, 0).is_ok(), "{arming:?}");
            }
        };
        arm(&mut arena);
        assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
        assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
        // The turret goes, and its gun with it: S1 freed, S2 kept.
        let mut journal = Vec::new();
        arena.apply_consequence(
            RED,
            &Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            0,
            None,
            &mut journal,
        );
        // 17b cut M1b.3: the view shows the machine gun in S2 on the terms
        // wanted, so the arming answers nothing — no re-arm, and S2 stands.
        assert_eq!(
            arming_for(&view_of(&arena, RED), &card, Some(100_000), CLASS_I_SLOTS),
            Vec::new(),
            "nothing to re-arm: the view shows S2 as wanted"
        );
        assert_eq!(arena.conditions.slot_of(RED, "cannon"), None);
        assert_eq!(
            arena.conditions.slot_of(RED, "machine_gun"),
            Some(2),
            "kept in place by the removal"
        );
        // A fresh lock's citation reads the number off the view's row.
        let view = view_of(&arena, RED);
        assert_eq!(
            view.weapons
                .iter()
                .map(|row| (row.id.as_str(), row.armed.as_ref().map(|armed| armed.slot)))
                .collect::<Vec<_>>(),
            vec![("machine_gun", Some(2))],
            "the view shows the arena's number"
        );
        let mut fight = StockMemory::default();
        let found = think(&arena, &view, &mut fight, 1_000);
        // The think's own ceiling (its release range, 600 m here) is not the
        // 1,000 m the arming above used, so its view shows S2 on OTHER terms
        // and it re-arms the machine gun — that gun alone, on change — and the
        // arena keeps it in place, S2.
        let rearmed: Vec<&FireCondition> = found
            .iter()
            .filter_map(|kind| match kind {
                IntentKind::Condition(Arming::Arm(cond)) => Some(cond),
                _ => None,
            })
            .collect();
        assert!(
            rearmed.iter().all(|cond| cond.weapon == "machine_gun")
                && !found
                    .iter()
                    .any(|kind| matches!(kind, IntentKind::Condition(Arming::Disarm { .. }))),
            "only the machine gun is re-armed, nothing disarmed: {found:?}"
        );
        for cond in &rearmed {
            assert_eq!(
                arena.resolve_arming(RED, &Arming::Arm((*cond).clone()), 0),
                Ok(Some(2)),
                "re-armed in place"
            );
        }
        assert_eq!(
            found.last(),
            Some(&IntentKind::Fire {
                weapon: SmolStr::new("machine_gun"),
                slot: Some(2)
            }),
            "17b cut M1b.3: the view's S2, where M1b.2 cited i + 1"
        );
        assert_eq!(arena.resolve_lock(RED, ContactId(0x77), 0), Ok(()));
        assert!(
            arena
                .resolve_fire_by(RED, "machine_gun", 0, FiredBy::Mind { slot: Some(2) })
                .is_ok(),
            "the citation holds its warrant"
        );
    }

    // -----------------------------------------------------------------------
    // 17b cut M4.6a (the critic's D11): the reflex's two orphan pins, tried
    // row for row on the view
    // -----------------------------------------------------------------------

    /// **The stock tank's three rows of `Ranges`' table, read off the view**
    /// (17b cut M4.6a, D11): the reflex's
    /// `e1_the_stock_tanks_three_rows_of_the_range_table` row for row, its
    /// fixture flown by a mind and every distance read by [`view_ranges`] off
    /// the mind's view instead of by the reflex's `ranges` off the live unit —
    /// the four order bands, `release_cm > engage_cm` under each, no order as
    /// the `far` row, the operator's one thousand, the short first band and
    /// the dry platform. Every row is the reflex test's own, so when that test
    /// dies with the reflex (17b cut M4.6c) this one holds the table.
    ///
    /// The shipped numbers, written here rather than read off the pack: a
    /// 60-cell sight of 1,000 cm cells, a cannon whose bands are 1.5 km and 3
    /// km, and doctrine's `engage_band_permille` of 700.
    #[test]
    fn e1_the_stock_tanks_three_rows_of_the_range_table_off_the_view() {
        use crate::arena::doctrine::Ranges;
        use crate::arena::perception::{SensorProfile, SignatureFactor};
        use crate::arena::state::WeaponSummary;
        use crate::arena::test_support::{arena_of, tank};
        use crate::arena::weapons::{DEFAULT_WEAPON, WeaponBand};

        let mut fixture = tank(ObjId(4), PartyId::RED, pose(1_500, 1_500));
        fixture.sensor = Some(SensorProfile {
            component: smol_str::SmolStr::new("visual_sensor"),
            max_range_cells: 60,
            max_attenuation: 40,
            classify_after_ms: 3_000,
            sees_wrecks: false,
            share_range_cells: 30,
            signature: vec![SignatureFactor {
                class: mush_atlas::TerrainClass::Ground,
                moving_permille: 1_000,
                stopped_permille: 1_000,
            }],
        });
        fixture.weapons = vec![WeaponSummary {
            id: SmolStr::new(DEFAULT_WEAPON),
            max_range_cm: 300_000,
            cycle_ms: 4_000,
            ammunition: SmolStr::new("tacsim.ammunition.he_105"),
            burst_rounds: 1,
            muzzle_speed_cms: 145_000,
            bands: vec![
                WeaponBand {
                    max_range_cm: 150_000,
                    dispersion_mrad: 3,
                },
                WeaponBand {
                    max_range_cm: 300_000,
                    dispersion_mrad: 6,
                },
            ],
            count_per_load: 40,
            damage: 120,
            mount_section: SmolStr::new("turret"),
        }];
        fixture.controller = Controller::Mind;
        fixture
            .reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let arena = arena_of(vec![fixture]);
        let view = view_of(&arena, ObjId(4));
        assert_eq!(sight_cm(&view), 60_000, "600 m of scope");
        assert_eq!(
            fight_inner_cm_of(
                loaded_weapon(&view).and_then(|weapon| weapon.bands_cm.first().copied()),
                sight_cm(&view)
            ),
            60_000
        );
        assert_eq!(fight_limit_cm(&view), 60_000);

        let cfg = ReflexConfig::default();
        let ordered = |band: Option<RangeBand>| FrameOrder {
            max_range_band: band,
            ..order_of(MindObjective::Engage { target: None })
        };
        for (band, engage_cm, release_cm, stand_cm) in [
            (None, 42_000_u64, 60_000_u64, 20_000_u64),
            (Some(RangeBand::Far), 42_000, 60_000, 20_000),
            (Some(RangeBand::Effective), 28_000, 40_000, 20_000),
            (Some(RangeBand::Close), 14_000, 20_000, 20_000),
        ] {
            let order = ordered(band);
            let have = view_ranges(&view, cfg.engage_band_permille, Some(&order));
            assert_eq!(
                have,
                Ranges {
                    engage_cm,
                    release_cm,
                    stand_cm,
                    limit_cm: 60_000,
                },
                "the stock tank under {band:?}"
            );
            assert!(
                have.release_cm > have.engage_cm,
                "the hysteresis collapsed under {band:?}: {have:?}"
            );
        }
        // No order at all is the `far` row, and the `far` row caps nothing.
        assert_eq!(
            view_ranges(&view, cfg.engage_band_permille, None),
            view_ranges(
                &view,
                cfg.engage_band_permille,
                Some(&ordered(Some(RangeBand::Far)))
            )
        );

        // An operator who writes one thousand has asked for one distance and
        // gets it — which is the key's own doc, pinned.
        let flat = ReflexConfig {
            engage_band_permille: 1_000,
            ..cfg.clone()
        };
        let have = view_ranges(&view, flat.engage_band_permille, None);
        assert_eq!((have.engage_cm, have.release_cm), (60_000, 60_000));

        // **A short first band**: the `close` band is a third of the *last* edge
        // and `release_cm` is the *first* one, so a gun whose inner band is under
        // a third of its outer reach would leave an en-route stand wider than the
        // firing ceiling. The hull's own band is cut and the view read again.
        let mut stubby = arena;
        if let Some(state) = stubby.unit_mut(ObjId(4))
            && let Some(weapon) = state.weapons.first_mut()
            && let Some(band) = weapon.bands.first_mut()
        {
            band.max_range_cm = 10_000;
        }
        let view = view_of(&stubby, ObjId(4));
        let have = view_ranges(&view, cfg.engage_band_permille, None);
        assert_eq!(
            have,
            Ranges {
                engage_cm: 7_000,
                release_cm: 10_000,
                stand_cm: 10_000,
                limit_cm: 60_000,
            },
            "a 100 m first band on a 3 km gun: the `close` band would have been 200 m"
        );
        assert!(
            have.stand_cm <= have.release_cm,
            "an en-route stand can never be wider than the firing ceiling: {have:?}"
        );

        // And a platform with nothing loaded has no distances at all.
        let mut dry = stubby;
        if let Some(state) = dry.unit_mut(ObjId(4)) {
            state.ammo.insert(SmolStr::new(DEFAULT_WEAPON), 0);
        }
        let view = view_of(&dry, ObjId(4));
        assert_eq!(
            view_ranges(&view, cfg.engage_band_permille, None),
            Ranges::default()
        );
    }

    /// **A weapon that makes no heat never holds the mind back, and a hot
    /// one only at the ceiling** (17b cut M4.6a, D11): the reflex's
    /// `d2_a_weapon_that_makes_no_heat_never_holds_the_controller_back` row for
    /// row, its fixture flown by a mind and every answer read by
    /// [`would_overheat`] off the mind's view instead of by the reflex's copy
    /// off the live unit — cold, one shot short of the ceiling (16b cut E.2's
    /// `T-55` row), at the ceiling, and no heat state at all. The hull carries
    /// the two weapons the reflex test names, so the view has a row for each
    /// (the reflex's copy took a name; the view's takes the row). Its last row
    /// holds on the view by another branch: the frame writes a weapon's
    /// per-shot heat as nought when the hull has no heat state (the cells
    /// still read cooked), so a change that took the per-shot heat off the
    /// weapon's definition would flip that row.
    #[test]
    fn d2_a_weapon_that_makes_no_heat_never_holds_the_mind_back() {
        use crate::arena::state::{HeatState, UnitState, WeaponSummary};
        use crate::arena::test_support::{arena_of, cannon, tank};

        let mut unit = tank(ObjId(4), PartyId::RED, pose(1_500, 1_500));
        unit.unit = d11_hot_tactical_unit(ObjId(4));
        unit.weapons = ["laser", "machine_gun"]
            .into_iter()
            .map(|id| WeaponSummary {
                id: SmolStr::new(id),
                ..cannon()
            })
            .collect();
        unit.controller = Controller::Mind;
        unit.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let mut arena = arena_of(vec![unit]);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.heat = Some(HeatState {
                section: SmolStr::new("hull"),
                max: 60,
                per_s: 3,
                per_shot: [(SmolStr::new("laser"), 20)].into_iter().collect(),
                carry_milli: 0,
            });
        }
        let asked = |arena: &Arena| -> (bool, bool) {
            let view = view_of(arena, ObjId(4));
            let row = |id: &str| {
                view.weapons
                    .iter()
                    .find(|weapon| weapon.id == id)
                    .cloned()
                    .unwrap_or_else(|| panic!("the view's {id} row"))
            };
            (
                would_overheat(&view, &row("laser")),
                would_overheat(&view, &row("machine_gun")),
            )
        };
        // Cold: neither is held back.
        assert_eq!(asked(&arena), (false, false));
        // One shot short of the ceiling: the engine accepts a shot that would
        // carry a unit past the maximum and clamps it there, and refuses only
        // AT the ceiling, so the mind takes this one too.
        d11_set_heat(&mut arena, ObjId(4), 50);
        assert_eq!(asked(&arena), (false, false));
        // **At** the ceiling: the engine refuses the hot gun and takes the cold
        // one, and so does the mind.
        d11_set_heat(&mut arena, ObjId(4), 60);
        assert_eq!(asked(&arena), (true, false));
        assert_eq!(
            arena.unit(ObjId(4)).map(UnitState::overheated),
            Some(true),
            "the engine's own predicate agrees the platform is cooked"
        );
        // And a platform with no heat state at all answers `false` for both.
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.heat = None;
        }
        assert_eq!(asked(&arena), (false, false));
    }

    /// The fixture's one-section unit **with a heat cell on its hull** (the
    /// reflex's d2 fixture, copied for its mirror above).
    fn d11_hot_tactical_unit(id: ObjId) -> mush_tactics::TacticalUnit {
        use crate::arena::test_support::{dref, rules};
        let cell = |name: &str, max: i64, init: i64| mush_tactics::CellSpec {
            id: mush_tactics::CellId::new(name).expect("a cell"),
            min: 0,
            max,
            init,
        };
        let spec = mush_tactics::UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank"),
            sections: vec![mush_tactics::SectionSpec {
                id: mush_tactics::SectionId::new("hull").expect("a section"),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                facets: vec![mush_tactics::FacetSpec {
                    id: mush_tactics::FacetId::new("front").expect("a facet"),
                    cells: vec![cell("armor", 10, 10)],
                }],
                slots: Vec::new(),
                cells: vec![
                    cell("internals", 8, 8),
                    cell(crate::definitions::unit::HEAT_CELL, 60, 0),
                ],
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: None,
            rules: rules(),
        };
        mush_tactics::TacticalUnit::from_spec(&spec, id).expect("a one-section unit with heat")
    }

    /// Put the heat cell at `value` through one plan (the reflex's d2 helper,
    /// copied for its mirror above).
    fn d11_set_heat(arena: &mut Arena, unit: ObjId, value: i64) {
        let addr = mush_tactics::CellAddr::Section {
            section: mush_tactics::SectionId::new("hull").expect("a section"),
            cell: mush_tactics::CellId::new(crate::definitions::unit::HEAT_CELL).expect("a cell"),
        };
        let hot = {
            let Some(state) = arena.unit(unit) else {
                panic!("the fixture tank");
            };
            let plan = mush_tactics::UnitMutationPlan::new(
                vec![mush_tactics::UnitMutationOp::SetCell { addr, value }],
                None,
                Some(SmolStr::new("test:heat")),
            )
            .expect("a one-op plan");
            mush_tactics::apply(&state.unit, &plan, 0)
                .expect("the plan applies")
                .0
        };
        if let Some(state) = arena.unit_mut(unit) {
            state.unit = hot;
        }
    }
}
